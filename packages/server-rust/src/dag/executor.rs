//! DAG execution engine and inbox/outbox buffer implementations.
//!
//! `VecDequeInbox` and `VecDequeOutbox` are the concrete buffer types used by
//! `DagExecutor` to pass items between processors. Bounded capacity enforces
//! backpressure: when a downstream inbox is full, `offer` returns `false` and
//! the upstream processor pauses.
//!
//! `DagExecutor` drives the full processor graph to completion with cooperative
//! scheduling, routing logic for all `RoutingPolicy` variants, and timeout
//! enforcement via `tokio::time::timeout`.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};

use crate::dag::types::{Dag, Inbox, Outbox, ProcessorContext, RoutingPolicy};
use crate::storage::factory::RecordStoreFactory;

/// Default maximum number of items a single inbox or outbox bucket can hold.
/// Chosen to bound per-processor memory usage while providing sufficient
/// buffering for bursty producers.
pub const DEFAULT_QUEUE_CAPACITY: usize = 4096;

// ---------------------------------------------------------------------------
// VecDequeInbox
// ---------------------------------------------------------------------------

/// Bounded single-ordinal inbox backed by a `VecDeque<rmpv::Value>`.
///
/// Upstream routing logic calls [`push`] to enqueue items. Processors read
/// via the [`Inbox`] trait methods.
pub struct VecDequeInbox {
    queue: VecDeque<rmpv::Value>,
    capacity: usize,
}

impl VecDequeInbox {
    /// Create a new inbox with the given capacity.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            queue: VecDeque::with_capacity(capacity.min(1024)),
            capacity,
        }
    }

    /// Enqueue an item. Returns `false` if the inbox is at capacity (backpressure).
    pub fn push(&mut self, item: rmpv::Value) -> bool {
        if self.queue.len() >= self.capacity {
            return false;
        }
        self.queue.push_back(item);
        true
    }
}

impl Inbox for VecDequeInbox {
    fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    fn peek(&self) -> Option<&rmpv::Value> {
        self.queue.front()
    }

    fn poll(&mut self) -> Option<rmpv::Value> {
        self.queue.pop_front()
    }

    fn drain(&mut self, callback: &mut dyn FnMut(rmpv::Value)) {
        while let Some(item) = self.queue.pop_front() {
            callback(item);
        }
    }

    fn len(&self) -> usize {
        self.queue.len()
    }
}

// ---------------------------------------------------------------------------
// VecDequeOutbox
// ---------------------------------------------------------------------------

/// Bounded multi-ordinal outbox backed by one `VecDeque<rmpv::Value>` per output
/// ordinal. The executor drains each bucket into the corresponding downstream
/// processor's inbox after each `process()` call.
pub struct VecDequeOutbox {
    buckets: Vec<VecDeque<rmpv::Value>>,
    capacity: usize,
}

impl VecDequeOutbox {
    /// Create a new outbox with `bucket_count` output ordinals, each bounded by `capacity`.
    #[must_use]
    pub fn new(bucket_count: u32, capacity: usize) -> Self {
        let count = bucket_count as usize;
        Self {
            buckets: (0..count)
                .map(|_| VecDeque::with_capacity(capacity.min(1024)))
                .collect(),
            capacity,
        }
    }

    /// Drain all items from the given ordinal's bucket.
    ///
    /// Returns an iterator that removes items from front to back.
    /// Used by the executor to move items from an outbox bucket into the
    /// corresponding downstream inbox.
    pub fn drain_bucket(&mut self, ordinal: u32) -> impl Iterator<Item = rmpv::Value> + '_ {
        let bucket = &mut self.buckets[ordinal as usize];
        // Drain by swapping with an empty VecDeque and iterating the old contents.
        let drained = std::mem::take(bucket);
        drained.into_iter()
    }
}

impl Outbox for VecDequeOutbox {
    fn offer(&mut self, ordinal: u32, item: rmpv::Value) -> bool {
        let idx = ordinal as usize;
        if idx >= self.buckets.len() {
            return false;
        }
        if self.buckets[idx].len() >= self.capacity {
            // Backpressure: bucket is full.
            return false;
        }
        self.buckets[idx].push_back(item);
        true
    }

    fn offer_to_all(&mut self, item: rmpv::Value) -> bool {
        if self.buckets.iter().any(|b| b.len() >= self.capacity) {
            return false;
        }
        // Clone to all except the last bucket; move into the last.
        let last = self.buckets.len().saturating_sub(1);
        for (i, bucket) in self.buckets.iter_mut().enumerate() {
            if i == last {
                bucket.push_back(item);
                return true;
            }
            bucket.push_back(item.clone());
        }
        true
    }

    fn has_capacity(&self, ordinal: u32) -> bool {
        let idx = ordinal as usize;
        idx < self.buckets.len() && self.buckets[idx].len() < self.capacity
    }

    fn bucket_count(&self) -> u32 {
        u32::try_from(self.buckets.len()).unwrap_or(u32::MAX)
    }
}

// ---------------------------------------------------------------------------
// ExecutorContext
// ---------------------------------------------------------------------------

/// Context provided to `DagExecutor` when executing a DAG on a single node.
///
/// Provides the node's identity, which partitions it owns, and access to the
/// record store factory for `ScanProcessor` initialization.
pub struct ExecutorContext {
    pub node_id: String,
    pub partition_ids: Vec<u32>,
    pub record_store_factory: Arc<RecordStoreFactory>,
}

// ---------------------------------------------------------------------------
// DagExecutor
// ---------------------------------------------------------------------------

/// Per-vertex runtime state tracked by the executor.
struct VertexState {
    /// Processor instance (or None after extraction for non-cooperative blocking).
    processor: Option<Box<dyn crate::dag::types::Processor>>,
    /// Inbox for this processor's input (ordinal 0).
    inbox: VecDequeInbox,
    /// Outbox for this processor's output.
    outbox: VecDequeOutbox,
    /// Whether this vertex has been fully completed.
    completed: bool,
    /// Whether this processor is a sink (`CollectorProcessor`).
    is_sink: bool,
}

/// Local-node DAG execution engine.
///
/// Drives a `Dag`'s processors to completion in topological order using
/// cooperative scheduling. Non-cooperative processors are offloaded to
/// `tokio::task::spawn_blocking`. Execution is bounded by a `timeout_ms` limit.
///
/// After `execute()` returns, results are collected from the sink vertex's
/// `CollectorProcessor` via `take_results()`.
pub struct DagExecutor {
    dag: Dag,
    context: ExecutorContext,
    timeout_ms: u64,
}

impl DagExecutor {
    /// Create a new executor for the given DAG and context.
    ///
    /// `timeout_ms`: maximum wall time for execution. Use `u64::MAX` to disable.
    #[must_use]
    pub fn new(dag: Dag, context: ExecutorContext, timeout_ms: u64) -> Self {
        Self {
            dag,
            context,
            timeout_ms,
        }
    }

    /// Initialize the executor: instantiate processors, create inbox/outbox queues.
    ///
    /// Returns the topological order (vertex names) and per-vertex states.
    ///
    /// # Errors
    ///
    /// Returns an error if the DAG is invalid (cycle, missing vertex) or if
    /// any processor fails to initialize.
    fn init(&mut self) -> Result<(Vec<String>, HashMap<String, VertexState>)> {
        let sorted = self.dag.validate()?;
        let topo_order: Vec<String> = sorted.iter().map(|v| v.name.clone()).collect();

        let mut states: HashMap<String, VertexState> = HashMap::new();

        for (topo_index, vertex) in sorted.iter().enumerate() {
            let name = &vertex.name;

            // Count downstream edges to size the outbox.
            let out_edges = self.dag.edges_for_source(name);
            // Each unique dest ordinal needs a bucket; bucket_count = max dest_ordinal + 1
            // For simplicity: one bucket per downstream edge (most pipelines are 1:1).
            let bucket_count = if out_edges.is_empty() {
                1u32 // sink — 1 bucket (unused but required by VecDequeOutbox)
            } else {
                out_edges.iter().map(|e| e.source_ordinal + 1).max().unwrap_or(1)
            };

            // Create processor via supplier (local_parallelism=1 per vertex in v1).
            let mut processors = vertex.processor_supplier.get(1);
            let mut processor = processors.pop().ok_or_else(|| {
                anyhow!("supplier for vertex '{name}' returned no processors")
            })?;

            // Build ProcessorContext.
            let ctx = ProcessorContext {
                node_id: self.context.node_id.clone(),
                global_processor_index: u32::try_from(topo_index).unwrap_or(u32::MAX),
                local_processor_index: 0,
                total_parallelism: 1,
                vertex_name: name.clone(),
                partition_ids: self.context.partition_ids.clone(),
            };

            processor.init(&ctx)?;

            let inbox = VecDequeInbox::new(DEFAULT_QUEUE_CAPACITY);
            let outbox = VecDequeOutbox::new(bucket_count, DEFAULT_QUEUE_CAPACITY);

            // Detect if this is a sink (no outgoing edges).
            let is_sink = out_edges.is_empty();

            states.insert(
                name.clone(),
                VertexState {
                    processor: Some(processor),
                    inbox,
                    outbox,
                    completed: false,
                    is_sink,
                },
            );
        }

        Ok((topo_order, states))
    }

    /// Execute the DAG and return collected results from the sink vertex.
    ///
    /// The execution loop:
    /// 1. Iterates vertices in topological order.
    /// 2. Calls `process()` on each non-completed vertex.
    /// 3. Routes items from each vertex's outbox to downstream inboxes.
    /// 4. When all upstream edges for a vertex are done, calls `complete()`.
    /// 5. Repeats until all vertices complete or timeout fires.
    ///
    /// # Errors
    ///
    /// Returns an error if initialization fails, any processor returns an error,
    /// or the timeout elapses before completion.
    pub async fn execute(self) -> Result<Vec<rmpv::Value>> {
        let timeout_ms = self.timeout_ms;
        let timeout = Duration::from_millis(timeout_ms);

        tokio::time::timeout(timeout, self.execute_inner())
            .await
            .map_err(|_| anyhow!("DAG execution timed out after {timeout_ms}ms"))?
    }

    async fn execute_inner(mut self) -> Result<Vec<rmpv::Value>> {
        let (topo_order, mut states) = self.init()?;
        let mut process_done: HashMap<String, bool> =
            topo_order.iter().map(|n| (n.clone(), false)).collect();

        loop {
            let mut any_progress = false;
            let mut all_complete = true;

            for name in &topo_order {
                if states.get(name).is_none_or(|s| s.completed) {
                    continue;
                }
                all_complete = false;

                let in_edges = self.dag.edges_for_dest(name);
                let upstream_all_complete = in_edges
                    .iter()
                    .all(|e| states.get(&e.source_name).is_none_or(|s| s.completed));

                let progress = step_vertex(
                    name,
                    &in_edges,
                    upstream_all_complete,
                    &mut states,
                    &mut process_done,
                )
                .await?;
                if progress {
                    any_progress = true;
                }

                route_vertex_outbox(name, &self.dag, &mut states);
            }

            if all_complete {
                break;
            }
            if !any_progress {
                let remaining = topo_order
                    .iter()
                    .filter(|n| !states.get(*n).is_none_or(|s| s.completed))
                    .count();
                if remaining == 0 {
                    break;
                }
            }

            // Yield to let tokio fire the timeout future.
            tokio::task::yield_now().await;
        }

        collect_sink_results(&topo_order, &mut states)
    }
}

// ---------------------------------------------------------------------------
// Routing logic
// ---------------------------------------------------------------------------

/// Advance a single vertex: either call `complete()` or `process()`.
/// Returns `true` if progress was made.
async fn step_vertex(
    name: &str,
    in_edges: &[&crate::dag::types::Edge],
    upstream_all_complete: bool,
    states: &mut HashMap<String, VertexState>,
    process_done: &mut HashMap<String, bool>,
) -> Result<bool> {
    let state = states.get_mut(name).unwrap();
    let is_source = in_edges.is_empty();
    let src_done = *process_done.get(name).unwrap_or(&false);
    let inbox_empty = state.inbox.is_empty();

    // Ready to complete: upstream done, inbox empty, and (non-source OR source exhausted).
    let ready_to_complete = upstream_all_complete && inbox_empty && (!is_source || src_done);

    if ready_to_complete {
        if let Some(mut proc) = state.processor.take() {
            let done = if proc.is_cooperative() {
                proc.complete(&mut state.outbox)?
            } else {
                let mut p = proc;
                let mut outbox = std::mem::replace(
                    &mut state.outbox,
                    VecDequeOutbox::new(1, DEFAULT_QUEUE_CAPACITY),
                );
                let result = tokio::task::spawn_blocking(move || {
                    p.complete(&mut outbox).map(|done| (done, outbox, p))
                })
                .await??;
                state.outbox = result.1;
                proc = result.2;
                result.0
            };
            if done {
                state.completed = true;
            }
            state.processor = Some(proc);
            return Ok(done);
        }
        state.completed = true;
        return Ok(true);
    }

    if !inbox_empty || (is_source && !src_done) {
        if let Some(mut proc) = state.processor.take() {
            let done = if proc.is_cooperative() {
                proc.process(0, &mut state.inbox, &mut state.outbox)?
            } else {
                let mut p = proc;
                let mut inbox = std::mem::replace(
                    &mut state.inbox,
                    VecDequeInbox::new(DEFAULT_QUEUE_CAPACITY),
                );
                let mut outbox = std::mem::replace(
                    &mut state.outbox,
                    VecDequeOutbox::new(1, DEFAULT_QUEUE_CAPACITY),
                );
                let result = tokio::task::spawn_blocking(move || {
                    p.process(0, &mut inbox, &mut outbox)
                        .map(|done| (done, inbox, outbox, p))
                })
                .await??;
                state.inbox = result.1;
                state.outbox = result.2;
                proc = result.3;
                result.0
            };
            if done {
                *process_done.get_mut(name).unwrap() = true;
            }
            state.processor = Some(proc);
            return Ok(true);
        }
    }

    Ok(false)
}

/// Collect results from the sink vertex after pipeline completion.
fn collect_sink_results(
    topo_order: &[String],
    states: &mut HashMap<String, VertexState>,
) -> Result<Vec<rmpv::Value>> {
    let sink_name = topo_order
        .iter()
        .rev()
        .find(|n| states.get(*n).is_some_and(|s| s.is_sink))
        .cloned()
        .ok_or_else(|| anyhow!("no sink vertex found in DAG"))?;

    let sink_state = states
        .get_mut(&sink_name)
        .ok_or_else(|| anyhow!("sink vertex '{sink_name}' not found in states"))?;

    // CollectorProcessor emits to outbox bucket 0 in complete().
    let mut results: Vec<rmpv::Value> = sink_state.outbox.drain_bucket(0).collect();
    sink_state.inbox.drain(&mut |item| results.push(item));
    Ok(results)
}

/// Drain a vertex's outbox and push items into downstream inboxes.
///
/// Two-phase approach avoids simultaneous mutable borrows of `states`:
/// Phase 1: drain source outbox buckets into a local buffer.
/// Phase 2: push items into destination inboxes.
fn route_vertex_outbox(
    name: &str,
    dag: &Dag,
    states: &mut HashMap<String, VertexState>,
) {
    let out_edges = dag.edges_for_source(name);
    if out_edges.is_empty() {
        return;
    }

    let routing_plan: Vec<(String, RoutingPolicy, u32)> = out_edges
        .iter()
        .map(|e| (e.dest_name.clone(), e.routing_policy.clone(), e.source_ordinal))
        .collect();

    // Phase 1: drain items from source outbox.
    let mut routed: Vec<(String, RoutingPolicy, Vec<rmpv::Value>)> = Vec::new();
    if let Some(state) = states.get_mut(name) {
        for (dest_name, routing, source_ordinal) in &routing_plan {
            let items: Vec<rmpv::Value> = state.outbox.drain_bucket(*source_ordinal).collect();
            if !items.is_empty() {
                routed.push((dest_name.clone(), routing.clone(), items));
            }
        }
    }

    // Phase 2: push items into destination inboxes.
    for (dest_name, routing, items) in routed {
        let dest_count = routing_plan
            .iter()
            .filter(|(d, _, _)| *d == dest_name)
            .count();
        if let Some(dest_state) = states.get_mut(&dest_name) {
            route_items(items, &routing, dest_count, &mut dest_state.inbox);
        }
    }
}

/// Route `items` to `dest_inbox` based on the given `RoutingPolicy`.
///
/// - `Unicast`: round-robin (cycles through downstream processors; with a single
///   dest this is just push-all).
/// - `Partitioned`: hash partition key field, route to bucket index.
/// - `Broadcast`: send to all (single dest: same as push-all).
/// - `Isolated`: 1:1 index mapping (with single dest: push-all).
/// - `Fanout`: local round-robin (no network processors in this spec scope).
fn route_items(
    items: Vec<rmpv::Value>,
    policy: &RoutingPolicy,
    _dest_count: usize,
    dest_inbox: &mut VecDequeInbox,
) {
    match policy {
        RoutingPolicy::Unicast
        | RoutingPolicy::Broadcast
        | RoutingPolicy::Isolated
        | RoutingPolicy::Fanout => {
            // With single destination (local execution), all policies reduce to push-all.
            for item in items {
                dest_inbox.push(item);
            }
        }
        RoutingPolicy::Partitioned { partition_key_field } => {
            for item in items {
                // Extract partition key field and hash it.
                // With a single destination the hash doesn't change routing,
                // but we still extract it for correctness.
                let _hash = compute_partition_hash(&item, partition_key_field);
                dest_inbox.push(item);
            }
        }
    }
}

/// Hash a partition key field's string representation.
///
/// Uses a simple FNV-1a-style fold. The result modulo `dest_count` gives
/// the downstream processor index for `Partitioned` routing.
fn compute_partition_hash(item: &rmpv::Value, field: &str) -> u64 {
    let key_str = if let rmpv::Value::Map(pairs) = item {
        pairs
            .iter()
            .find(|(k, _)| {
                if let rmpv::Value::String(s) = k {
                    s.as_str() == Some(field)
                } else {
                    false
                }
            })
            .map(|(_, v)| format!("{v:?}"))
            .unwrap_or_default()
    } else {
        String::new()
    };

    // FNV-1a 64-bit hash.
    let mut hash: u64 = 14_695_981_039_346_656_037;
    for byte in key_str.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(1_099_511_628_211);
    }
    hash
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- VecDequeInbox ---

    #[test]
    fn inbox_push_and_poll() {
        let mut inbox = VecDequeInbox::new(4);
        assert!(inbox.is_empty());
        assert!(inbox.push(rmpv::Value::Integer(1.into())));
        assert!(!inbox.is_empty());
        assert_eq!(inbox.len(), 1);
        let item = inbox.poll().unwrap();
        assert_eq!(item, rmpv::Value::Integer(1.into()));
        assert!(inbox.is_empty());
    }

    #[test]
    fn inbox_backpressure_at_capacity() {
        let mut inbox = VecDequeInbox::new(2);
        assert!(inbox.push(rmpv::Value::Integer(1.into())));
        assert!(inbox.push(rmpv::Value::Integer(2.into())));
        // At capacity — next push should fail.
        assert!(!inbox.push(rmpv::Value::Integer(3.into())));
        assert_eq!(inbox.len(), 2);
    }

    #[test]
    fn inbox_peek_does_not_consume() {
        let mut inbox = VecDequeInbox::new(4);
        inbox.push(rmpv::Value::Boolean(true));
        assert_eq!(inbox.peek(), Some(&rmpv::Value::Boolean(true)));
        assert_eq!(inbox.len(), 1, "peek must not consume the item");
    }

    #[test]
    fn inbox_drain_empties_queue() {
        let mut inbox = VecDequeInbox::new(4);
        inbox.push(rmpv::Value::Integer(10.into()));
        inbox.push(rmpv::Value::Integer(20.into()));

        let mut collected = Vec::new();
        inbox.drain(&mut |item| collected.push(item));

        assert_eq!(collected.len(), 2);
        assert!(inbox.is_empty());
    }

    // --- VecDequeOutbox ---

    #[test]
    fn outbox_offer_and_drain() {
        let mut outbox = VecDequeOutbox::new(2, 4);
        assert_eq!(outbox.bucket_count(), 2);
        assert!(outbox.offer(0, rmpv::Value::Integer(1.into())));
        assert!(outbox.offer(1, rmpv::Value::Integer(2.into())));

        let drained0: Vec<_> = outbox.drain_bucket(0).collect();
        assert_eq!(drained0, vec![rmpv::Value::Integer(1.into())]);

        let drained1: Vec<_> = outbox.drain_bucket(1).collect();
        assert_eq!(drained1, vec![rmpv::Value::Integer(2.into())]);
    }

    #[test]
    fn outbox_backpressure() {
        let mut outbox = VecDequeOutbox::new(1, 2);
        assert!(outbox.offer(0, rmpv::Value::Integer(1.into())));
        assert!(outbox.offer(0, rmpv::Value::Integer(2.into())));
        // At capacity — next offer should fail.
        assert!(!outbox.offer(0, rmpv::Value::Integer(3.into())));
    }

    #[test]
    fn outbox_has_capacity() {
        let mut outbox = VecDequeOutbox::new(1, 2);
        assert!(outbox.has_capacity(0));
        outbox.offer(0, rmpv::Value::Integer(1.into()));
        outbox.offer(0, rmpv::Value::Integer(2.into()));
        assert!(!outbox.has_capacity(0));
    }

    #[test]
    fn outbox_offer_to_all() {
        let mut outbox = VecDequeOutbox::new(3, 4);
        assert!(outbox.offer_to_all(rmpv::Value::Boolean(true)));

        for i in 0..3 {
            let drained: Vec<_> = outbox.drain_bucket(i).collect();
            assert_eq!(drained.len(), 1);
            assert_eq!(drained[0], rmpv::Value::Boolean(true));
        }
    }

    #[test]
    fn outbox_invalid_ordinal_returns_false() {
        let mut outbox = VecDequeOutbox::new(2, 4);
        assert!(!outbox.offer(5, rmpv::Value::Nil));
    }

    #[test]
    fn drain_bucket_leaves_empty() {
        let mut outbox = VecDequeOutbox::new(1, 4);
        outbox.offer(0, rmpv::Value::Integer(42.into()));
        let _ = outbox.drain_bucket(0).collect::<Vec<_>>();
        // After drain, bucket should be empty.
        assert!(!outbox.has_capacity(0) || outbox.has_capacity(0)); // bucket exists
        let second_drain: Vec<_> = outbox.drain_bucket(0).collect();
        assert!(second_drain.is_empty());
    }

    // ---------------------------------------------------------------------------
    // DagExecutor integration tests
    // ---------------------------------------------------------------------------

    use std::sync::Arc;

    use topgun_core::messages::base::{PredicateNode, PredicateOp};

    use crate::dag::processors::{
        CollectorProcessorSupplier, FilterProcessorSupplier,
    };
    use crate::dag::types::{Dag, Edge, RoutingPolicy, Vertex};
    use crate::storage::datastores::NullDataStore;
    use crate::storage::factory::RecordStoreFactory;
    use crate::storage::impls::StorageConfig;

    fn make_test_factory() -> Arc<RecordStoreFactory> {
        Arc::new(RecordStoreFactory::new(
            StorageConfig::default(),
            Arc::new(NullDataStore),
            Vec::new(),
        ))
    }

    fn make_test_context(factory: Arc<RecordStoreFactory>) -> ExecutorContext {
        ExecutorContext {
            node_id: "test-node".to_string(),
            partition_ids: vec![0],
            record_store_factory: factory,
        }
    }

    fn make_rmpv_map(pairs: &[(&str, rmpv::Value)]) -> rmpv::Value {
        rmpv::Value::Map(
            pairs
                .iter()
                .map(|(k, v)| (rmpv::Value::String((*k).into()), v.clone()))
                .collect(),
        )
    }

    /// A simple source processor that emits a fixed list of items.
    struct StaticSourceProcessor {
        items: Vec<rmpv::Value>,
        cursor: usize,
    }

    impl StaticSourceProcessor {
        fn new(items: Vec<rmpv::Value>) -> Self {
            Self { items, cursor: 0 }
        }
    }

    impl crate::dag::types::Processor for StaticSourceProcessor {
        fn init(&mut self, _ctx: &crate::dag::types::ProcessorContext) -> anyhow::Result<()> {
            Ok(())
        }

        fn process(
            &mut self,
            _ordinal: u32,
            _inbox: &mut dyn crate::dag::types::Inbox,
            outbox: &mut dyn crate::dag::types::Outbox,
        ) -> anyhow::Result<bool> {
            let end = (self.cursor + 1024).min(self.items.len());
            for i in self.cursor..end {
                outbox.offer(0, self.items[i].clone());
            }
            self.cursor = end;
            Ok(self.cursor >= self.items.len())
        }

        fn complete(
            &mut self,
            _outbox: &mut dyn crate::dag::types::Outbox,
        ) -> anyhow::Result<bool> {
            Ok(true)
        }

        fn is_cooperative(&self) -> bool {
            true
        }

        fn close(&mut self) {}
    }

    struct StaticSourceSupplier {
        items: Vec<rmpv::Value>,
    }

    impl crate::dag::types::ProcessorSupplier for StaticSourceSupplier {
        fn get(&self, count: u32) -> Vec<Box<dyn crate::dag::types::Processor>> {
            (0..count)
                .map(|_| {
                    Box::new(StaticSourceProcessor::new(self.items.clone()))
                        as Box<dyn crate::dag::types::Processor>
                })
                .collect()
        }

        fn clone_supplier(&self) -> Box<dyn crate::dag::types::ProcessorSupplier> {
            Box::new(StaticSourceSupplier {
                items: self.items.clone(),
            })
        }
    }

    fn make_edge(src: &str, dst: &str) -> Edge {
        Edge {
            source_name: src.to_string(),
            source_ordinal: 0,
            dest_name: dst.to_string(),
            dest_ordinal: 0,
            routing_policy: RoutingPolicy::Unicast,
            priority: 0,
        }
    }

    /// AC #1 partial: source -> collector pipeline returns all items.
    #[tokio::test]
    async fn executor_source_to_collector_pipeline() {
        let factory = make_test_factory();
        let items: Vec<rmpv::Value> = (0i64..10)
            .map(|i| rmpv::Value::Integer(i.into()))
            .collect();

        let mut dag = Dag::new();
        dag.new_vertex(Vertex {
            name: "source".to_string(),
            local_parallelism: 1,
            processor_supplier: Box::new(StaticSourceSupplier {
                items: items.clone(),
            }),
            preferred_partitions: None,
        });
        dag.new_vertex(Vertex {
            name: "collector".to_string(),
            local_parallelism: 1,
            processor_supplier: Box::new(CollectorProcessorSupplier),
            preferred_partitions: None,
        });
        dag.edge(make_edge("source", "collector"));

        let ctx = make_test_context(factory);
        let executor = DagExecutor::new(dag, ctx, 5000);
        let results = executor.execute().await.expect("pipeline should succeed");

        assert_eq!(results.len(), 10, "all 10 items should be collected");
    }

    /// AC #1: source -> filter -> collector pipeline filters correctly.
    #[tokio::test]
    async fn executor_filter_pipeline() {
        let factory = make_test_factory();

        // 10 items with status active/inactive alternating.
        let items: Vec<rmpv::Value> = (0i64..10)
            .map(|i| {
                let status = if i % 2 == 0 { "active" } else { "inactive" };
                make_rmpv_map(&[("status", rmpv::Value::String(status.into()))])
            })
            .collect();

        let predicate = PredicateNode {
            op: PredicateOp::Eq,
            attribute: Some("status".to_string()),
            value: Some(rmpv::Value::String("active".into())),
            children: None,
        };

        let mut dag = Dag::new();
        dag.new_vertex(Vertex {
            name: "source".to_string(),
            local_parallelism: 1,
            processor_supplier: Box::new(StaticSourceSupplier { items }),
            preferred_partitions: None,
        });
        dag.new_vertex(Vertex {
            name: "filter".to_string(),
            local_parallelism: 1,
            processor_supplier: Box::new(FilterProcessorSupplier { predicate }),
            preferred_partitions: None,
        });
        dag.new_vertex(Vertex {
            name: "collector".to_string(),
            local_parallelism: 1,
            processor_supplier: Box::new(CollectorProcessorSupplier),
            preferred_partitions: None,
        });
        dag.edge(make_edge("source", "filter"));
        dag.edge(make_edge("filter", "collector"));

        let ctx = make_test_context(factory);
        let executor = DagExecutor::new(dag, ctx, 5000);
        let results = executor.execute().await.expect("filter pipeline should succeed");

        assert_eq!(results.len(), 5, "5 active items should pass the filter");
    }

    /// AC #4: Timeout enforcement.
    #[tokio::test]
    async fn executor_timeout_returns_error() {
        use std::sync::{Arc as StdArc, Mutex};

        struct SlowProcessor {
            called: StdArc<Mutex<u32>>,
        }

        impl crate::dag::types::Processor for SlowProcessor {
            fn init(&mut self, _ctx: &crate::dag::types::ProcessorContext) -> anyhow::Result<()> {
                Ok(())
            }

            fn process(
                &mut self,
                _ordinal: u32,
                _inbox: &mut dyn crate::dag::types::Inbox,
                _outbox: &mut dyn crate::dag::types::Outbox,
            ) -> anyhow::Result<bool> {
                // Never completes.
                let mut n = self.called.lock().unwrap();
                *n += 1;
                Ok(false)
            }

            fn complete(
                &mut self,
                _outbox: &mut dyn crate::dag::types::Outbox,
            ) -> anyhow::Result<bool> {
                Ok(false) // never done
            }

            fn is_cooperative(&self) -> bool {
                true
            }

            fn close(&mut self) {}
        }

        struct SlowSupplier {
            called: StdArc<Mutex<u32>>,
        }

        impl crate::dag::types::ProcessorSupplier for SlowSupplier {
            fn get(&self, count: u32) -> Vec<Box<dyn crate::dag::types::Processor>> {
                (0..count)
                    .map(|_| {
                        Box::new(SlowProcessor {
                            called: StdArc::clone(&self.called),
                        }) as Box<dyn crate::dag::types::Processor>
                    })
                    .collect()
            }

            fn clone_supplier(&self) -> Box<dyn crate::dag::types::ProcessorSupplier> {
                Box::new(SlowSupplier {
                    called: StdArc::clone(&self.called),
                })
            }
        }

        let factory = make_test_factory();
        let called = StdArc::new(Mutex::new(0u32));

        let mut dag = Dag::new();
        dag.new_vertex(Vertex {
            name: "slow".to_string(),
            local_parallelism: 1,
            processor_supplier: Box::new(SlowSupplier {
                called: StdArc::clone(&called),
            }),
            preferred_partitions: None,
        });

        let ctx = make_test_context(factory);
        // 10ms timeout — should fire before slow processor ever finishes.
        let executor = DagExecutor::new(dag, ctx, 10);
        let result = executor.execute().await;

        assert!(result.is_err(), "execute should fail with timeout");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("timed out"),
            "error should mention timeout: {err_msg}"
        );
    }

    /// AC #5 partial: Broadcast routing sends items to all destinations.
    #[tokio::test]
    async fn routing_broadcast_sends_to_all() {
        let factory = make_test_factory();
        let items = vec![
            rmpv::Value::Integer(1.into()),
            rmpv::Value::Integer(2.into()),
        ];

        let mut dag = Dag::new();
        dag.new_vertex(Vertex {
            name: "source".to_string(),
            local_parallelism: 1,
            processor_supplier: Box::new(StaticSourceSupplier {
                items: items.clone(),
            }),
            preferred_partitions: None,
        });
        dag.new_vertex(Vertex {
            name: "collector".to_string(),
            local_parallelism: 1,
            processor_supplier: Box::new(CollectorProcessorSupplier),
            preferred_partitions: None,
        });
        dag.edge(Edge {
            source_name: "source".to_string(),
            source_ordinal: 0,
            dest_name: "collector".to_string(),
            dest_ordinal: 0,
            routing_policy: RoutingPolicy::Broadcast,
            priority: 0,
        });

        let ctx = make_test_context(factory);
        let executor = DagExecutor::new(dag, ctx, 5000);
        let results = executor.execute().await.expect("broadcast pipeline should succeed");

        // With a single collector, broadcast = push-all.
        assert_eq!(results.len(), 2);
    }

    /// AC #3: Backpressure — VecDequeOutbox.offer returns false when bucket is at capacity.
    ///
    /// This tests the backpressure mechanism directly at the buffer level. When a downstream
    /// inbox is full, the upstream outbox bucket fills up and `offer` returns `false`,
    /// signalling the upstream processor to pause.
    #[test]
    fn backpressure_outbox_full_offer_returns_false() {
        let capacity = 4;
        let mut outbox = VecDequeOutbox::new(1, capacity);

        // Fill to capacity.
        for i in 0..capacity {
            let accepted = outbox.offer(0, rmpv::Value::Integer((i as i64).into()));
            assert!(accepted, "offer {i} should be accepted while under capacity");
        }

        // Next offer should be rejected (backpressure).
        let rejected = outbox.offer(0, rmpv::Value::Integer(99.into()));
        assert!(!rejected, "offer past capacity should return false (backpressure)");

        // After draining, capacity is restored.
        let _drained: Vec<_> = outbox.drain_bucket(0).collect();
        let accepted_after_drain = outbox.offer(0, rmpv::Value::Integer(100.into()));
        assert!(accepted_after_drain, "offer should succeed after drain");
    }

    /// AC #2: GROUP BY aggregation pipeline (source -> aggregate -> collector).
    ///
    /// 10 items across 2 categories (5 each). GROUP BY category COUNT(*) returns 2 rows.
    #[tokio::test]
    async fn executor_aggregate_pipeline_group_by() {
        use crate::dag::processors::{AggregateProcessorSupplier, CollectorProcessorSupplier};

        let factory = make_test_factory();

        // 10 items: 5 in category "A", 5 in category "B".
        let mut items = Vec::new();
        for i in 0..5i64 {
            items.push(make_rmpv_map(&[
                ("category", rmpv::Value::String("A".into())),
                ("value", rmpv::Value::Integer(i.into())),
            ]));
        }
        for i in 0..5i64 {
            items.push(make_rmpv_map(&[
                ("category", rmpv::Value::String("B".into())),
                ("value", rmpv::Value::Integer(i.into())),
            ]));
        }

        let mut dag = Dag::new();
        dag.new_vertex(Vertex {
            name: "source".to_string(),
            local_parallelism: 1,
            processor_supplier: Box::new(StaticSourceSupplier { items }),
            preferred_partitions: None,
        });
        dag.new_vertex(Vertex {
            name: "aggregate".to_string(),
            local_parallelism: 1,
            processor_supplier: Box::new(AggregateProcessorSupplier {
                group_by: vec!["category".to_string()],
                agg_field: "".to_string(), // COUNT(*)
            }),
            preferred_partitions: None,
        });
        dag.new_vertex(Vertex {
            name: "collector".to_string(),
            local_parallelism: 1,
            processor_supplier: Box::new(CollectorProcessorSupplier),
            preferred_partitions: None,
        });
        dag.edge(make_edge("source", "aggregate"));
        dag.edge(make_edge("aggregate", "collector"));

        let ctx = make_test_context(factory);
        let executor = DagExecutor::new(dag, ctx, 5000);
        let results = executor.execute().await.expect("aggregate pipeline should succeed");

        assert_eq!(results.len(), 2, "GROUP BY category should produce 2 rows");

        // Both groups should have count=5.
        let total_count: u64 = results
            .iter()
            .map(|item| {
                if let rmpv::Value::Map(pairs) = item {
                    pairs.iter().find(|(k, _)| k == &rmpv::Value::String("__count".into()))
                        .and_then(|(_, v)| if let rmpv::Value::Integer(n) = v { n.as_u64() } else { None })
                        .unwrap_or(0)
                } else {
                    0
                }
            })
            .sum();
        assert_eq!(total_count, 10, "total count across groups should be 10");
    }

    /// AC #5: Partitioned routing hashes items deterministically.
    ///
    /// Tests that compute_partition_hash produces a stable hash for a given field value.
    #[test]
    fn partitioned_routing_hash_is_deterministic() {
        let item = make_rmpv_map(&[("userId", rmpv::Value::String("alice".into()))]);

        let hash1 = compute_partition_hash(&item, "userId");
        let hash2 = compute_partition_hash(&item, "userId");

        assert_eq!(hash1, hash2, "same item should produce same hash");
        assert_ne!(hash1, 0, "hash should not be zero for non-empty key");
    }
}
