//! Network processors for inter-node data flow during distributed DAG execution.
//!
//! Two processor types bridge the local DAG execution engine to the cluster
//! transport layer:
//!
//! - `NetworkSenderProcessor`: buffers items and flushes them as `DagData`
//!   cluster messages to a remote node in batches of 256.
//! - `NetworkReceiverProcessor`: drains items from an mpsc channel fed by the
//!   cluster message handler and emits them to downstream processors.
//!
//! Both processors use synchronous channel APIs (`try_send`/`try_recv`) because
//! `Processor::process()` and `complete()` are synchronous methods.

use std::sync::Mutex;

use anyhow::Result;
use tokio::sync::mpsc;

use crate::cluster::messages::{ClusterMessage, DagDataPayload};
use crate::dag::types::{Inbox, Outbox, Processor, ProcessorContext, ProcessorSupplier};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Number of items accumulated before a `DagData` batch is flushed to the
/// remote node. Chosen to balance per-message overhead with latency.
const SENDER_BATCH_SIZE: usize = 256;

/// Capacity of the mpsc channel feeding `NetworkReceiverProcessor`.
/// 4× the sender batch size to accommodate burst traffic and provide
/// backpressure to the cluster message handler.
const RECEIVER_CHANNEL_CAPACITY: usize = 1024;

// ---------------------------------------------------------------------------
// NetworkSenderProcessor
// ---------------------------------------------------------------------------

/// Non-cooperative processor that buffers items from its inbox and flushes
/// them in batches of 256 to a remote node via `ClusterMessage::DagData`.
///
/// The processor is non-cooperative because sending to the cluster transport
/// may involve syscalls or lock contention that should not share a tokio task.
pub struct NetworkSenderProcessor {
    // Retained for observability and future routing decisions (e.g., topology-aware batching).
    #[allow(dead_code)]
    target_node_id: String,
    execution_id: String,
    source_vertex: String,
    dest_vertex: String,
    /// Outbound cluster message channel to the transport layer.
    transport: mpsc::Sender<ClusterMessage>,
    /// Accumulated items not yet flushed.
    buffer: Vec<rmpv::Value>,
}

impl NetworkSenderProcessor {
    fn new(
        target_node_id: String,
        execution_id: String,
        source_vertex: String,
        dest_vertex: String,
        transport: mpsc::Sender<ClusterMessage>,
    ) -> Self {
        Self {
            target_node_id,
            execution_id,
            source_vertex,
            dest_vertex,
            transport,
            buffer: Vec::with_capacity(SENDER_BATCH_SIZE),
        }
    }

    /// Serialize `items` and send a single `DagData` message over the transport.
    ///
    /// Uses `try_send` because this method is called from synchronous context.
    /// Silently drops the batch if the transport channel is full — the caller
    /// is responsible for not overflowing the channel.
    fn flush_batch(&self, items: &[rmpv::Value]) {
        if items.is_empty() {
            return;
        }
        let Ok(serialized) = rmp_serde::to_vec_named(items) else {
            return;
        };
        let msg = ClusterMessage::DagData(DagDataPayload {
            execution_id: self.execution_id.clone(),
            source_vertex: self.source_vertex.clone(),
            dest_vertex: self.dest_vertex.clone(),
            items: serialized,
        });
        // try_send is the synchronous path; if the channel is full we drop
        // the batch rather than blocking the executor thread.
        let _ = self.transport.try_send(msg);
    }
}

impl Processor for NetworkSenderProcessor {
    fn init(&mut self, _context: &ProcessorContext) -> Result<()> {
        Ok(())
    }

    fn process(
        &mut self,
        _ordinal: u32,
        inbox: &mut dyn Inbox,
        _outbox: &mut dyn Outbox,
    ) -> Result<bool> {
        // Drain inbox into the buffer, flushing each time we accumulate 256 items.
        inbox.drain(&mut |item| {
            self.buffer.push(item);
            if self.buffer.len() >= SENDER_BATCH_SIZE {
                let batch =
                    std::mem::replace(&mut self.buffer, Vec::with_capacity(SENDER_BATCH_SIZE));
                self.flush_batch(&batch);
            }
        });
        // The sender never self-completes from process(); complete() drives final flush.
        Ok(false)
    }

    fn complete(&mut self, _outbox: &mut dyn Outbox) -> Result<bool> {
        // Flush any remaining buffered items as a final partial batch.
        let remainder = std::mem::take(&mut self.buffer);
        // Flush any tail items; if the transport channel is full, the final
        // batch is silently dropped (try_send semantics — no async blocking).
        self.flush_batch(&remainder);
        Ok(true)
    }

    fn is_cooperative(&self) -> bool {
        false
    }

    fn close(&mut self) {
        self.buffer.clear();
    }
}

// ---------------------------------------------------------------------------
// NetworkSenderProcessorSupplier
// ---------------------------------------------------------------------------

/// Supplier for `NetworkSenderProcessor`.
///
/// Holds the routing metadata and transport channel sender. Each call to
/// `get()` creates fresh processor instances sharing the same transport.
pub struct NetworkSenderProcessorSupplier {
    target_node_id: String,
    execution_id: String,
    source_vertex: String,
    dest_vertex: String,
    transport: mpsc::Sender<ClusterMessage>,
}

impl NetworkSenderProcessorSupplier {
    #[must_use]
    pub fn new(
        target_node_id: String,
        execution_id: String,
        source_vertex: String,
        dest_vertex: String,
        transport: mpsc::Sender<ClusterMessage>,
    ) -> Self {
        Self {
            target_node_id,
            execution_id,
            source_vertex,
            dest_vertex,
            transport,
        }
    }
}

impl ProcessorSupplier for NetworkSenderProcessorSupplier {
    fn get(&self, count: u32) -> Vec<Box<dyn Processor>> {
        (0..count)
            .map(|_| {
                Box::new(NetworkSenderProcessor::new(
                    self.target_node_id.clone(),
                    self.execution_id.clone(),
                    self.source_vertex.clone(),
                    self.dest_vertex.clone(),
                    self.transport.clone(),
                )) as Box<dyn Processor>
            })
            .collect()
    }

    fn clone_supplier(&self) -> Box<dyn ProcessorSupplier> {
        Box::new(NetworkSenderProcessorSupplier {
            target_node_id: self.target_node_id.clone(),
            execution_id: self.execution_id.clone(),
            source_vertex: self.source_vertex.clone(),
            dest_vertex: self.dest_vertex.clone(),
            transport: self.transport.clone(),
        })
    }
}

// ---------------------------------------------------------------------------
// NetworkReceiverProcessor
// ---------------------------------------------------------------------------

/// Non-cooperative processor that drains items from an mpsc channel fed by
/// the cluster message handler and emits them to the downstream outbox.
///
/// The channel carries `Vec<rmpv::Value>` batches (one batch = one `DagData`
/// message). Each item in the batch is forwarded individually to outbox ordinal 0.
pub struct NetworkReceiverProcessor {
    receiver: mpsc::Receiver<Vec<rmpv::Value>>,
}

impl NetworkReceiverProcessor {
    fn new(receiver: mpsc::Receiver<Vec<rmpv::Value>>) -> Self {
        Self { receiver }
    }
}

impl Processor for NetworkReceiverProcessor {
    fn init(&mut self, _context: &ProcessorContext) -> Result<()> {
        Ok(())
    }

    fn process(
        &mut self,
        _ordinal: u32,
        _inbox: &mut dyn Inbox,
        outbox: &mut dyn Outbox,
    ) -> Result<bool> {
        // Drain all available batches from the channel without blocking.
        loop {
            match self.receiver.try_recv() {
                Ok(batch) => {
                    for item in batch {
                        outbox.offer(0, item);
                    }
                }
                Err(mpsc::error::TryRecvError::Empty) => {
                    // No more data available right now; yield to the scheduler.
                    return Ok(false);
                }
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    // All senders have dropped: the upstream is done.
                    return Ok(true);
                }
            }
        }
    }

    fn complete(&mut self, _outbox: &mut dyn Outbox) -> Result<bool> {
        Ok(true)
    }

    fn is_cooperative(&self) -> bool {
        false
    }

    fn close(&mut self) {}
}

// ---------------------------------------------------------------------------
// NetworkReceiverProcessorSupplier
// ---------------------------------------------------------------------------

/// Supplier for `NetworkReceiverProcessor`.
///
/// Creates the mpsc channel pair at construction time (capacity 1024) and
/// exposes the sending half so the cluster message handler can feed data into
/// the processor.
///
/// **Single-use constraint:** The `Receiver` is non-Clone and must be moved
/// into the processor. Interior mutability (`Mutex<Option<Receiver>>`) is used
/// to satisfy the shared-reference signature of `ProcessorSupplier::get(&self)`.
/// `get()` may only be called once with `count=1`. A second call or a
/// `clone_supplier()` after the receiver has been consumed yields a supplier
/// without a valid receiver, which is a programming error.
pub struct NetworkReceiverProcessorSupplier {
    sender: mpsc::Sender<Vec<rmpv::Value>>,
    /// Interior mutability so `get(&self)` can move the Receiver into the processor.
    receiver: Mutex<Option<mpsc::Receiver<Vec<rmpv::Value>>>>,
}

impl NetworkReceiverProcessorSupplier {
    /// Create a new supplier. The internal channel has capacity 1024.
    #[must_use]
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel(RECEIVER_CHANNEL_CAPACITY);
        Self {
            sender: tx,
            receiver: Mutex::new(Some(rx)),
        }
    }

    /// Returns the channel's sending half for use by the cluster message handler.
    pub fn sender(&self) -> mpsc::Sender<Vec<rmpv::Value>> {
        self.sender.clone()
    }
}

impl Default for NetworkReceiverProcessorSupplier {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessorSupplier for NetworkReceiverProcessorSupplier {
    fn get(&self, count: u32) -> Vec<Box<dyn Processor>> {
        assert_eq!(
            count, 1,
            "NetworkReceiverProcessorSupplier is single-use: count must be 1"
        );
        let rx = self
            .receiver
            .lock()
            .expect("receiver mutex poisoned")
            .take()
            .expect("NetworkReceiverProcessorSupplier::get() called more than once");
        vec![Box::new(NetworkReceiverProcessor::new(rx)) as Box<dyn Processor>]
    }

    fn clone_supplier(&self) -> Box<dyn ProcessorSupplier> {
        // Clone produces a supplier with the sender cloned but no receiver.
        // This is intentionally limited: clone_supplier is used for plan
        // distribution, but NetworkReceiverProcessorSupplier is constructed
        // fresh on each receiving node with its own channel pair.
        Box::new(NetworkReceiverProcessorSupplier {
            sender: self.sender.clone(),
            receiver: Mutex::new(None),
        })
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster::messages::{DagCompletePayload, DagExecutePayload};
    use crate::dag::executor::{VecDequeInbox, VecDequeOutbox};
    use crate::dag::types::ProcessorContext;

    fn make_context() -> ProcessorContext {
        ProcessorContext {
            node_id: "test-node".to_string(),
            global_processor_index: 0,
            local_processor_index: 0,
            total_parallelism: 1,
            vertex_name: "net-sender".to_string(),
            partition_ids: vec![0],
        }
    }

    fn make_transport() -> (mpsc::Sender<ClusterMessage>, mpsc::Receiver<ClusterMessage>) {
        mpsc::channel(2048)
    }

    // --- AC1: NetworkSenderProcessor batches at 256 ---

    #[test]
    fn sender_flushes_on_full_batch() {
        let (tx, mut rx) = make_transport();
        let mut proc = NetworkSenderProcessor::new(
            "node-2".to_string(),
            "exec-1".to_string(),
            "scan".to_string(),
            "combine".to_string(),
            tx,
        );
        let ctx = make_context();
        proc.init(&ctx).unwrap();

        // Push exactly 256 items — should trigger exactly one flush.
        let mut inbox = VecDequeInbox::new(512);
        for i in 0..256i64 {
            inbox.push(rmpv::Value::Integer(i.into()));
        }
        let mut outbox = VecDequeOutbox::new(1, 512);
        let done = proc.process(0, &mut inbox, &mut outbox).unwrap();
        assert!(!done, "sender never self-completes from process()");

        // Exactly one DagData message should have been sent.
        let msg = rx.try_recv().expect("one DagData message expected");
        assert!(matches!(msg, ClusterMessage::DagData(_)));
        if let ClusterMessage::DagData(payload) = msg {
            assert_eq!(payload.execution_id, "exec-1");
            assert_eq!(payload.source_vertex, "scan");
            assert_eq!(payload.dest_vertex, "combine");
            // Deserialize the items and verify count.
            let items: Vec<rmpv::Value> =
                rmp_serde::from_slice(&payload.items).expect("deserialize items");
            assert_eq!(items.len(), 256);
        }

        // No second message.
        assert!(
            rx.try_recv().is_err(),
            "no second batch should be sent for exactly 256 items"
        );
    }

    #[test]
    fn sender_batches_across_multiple_flushes() {
        let (tx, mut rx) = make_transport();
        let mut proc = NetworkSenderProcessor::new(
            "node-2".to_string(),
            "exec-1".to_string(),
            "scan".to_string(),
            "combine".to_string(),
            tx,
        );
        let ctx = make_context();
        proc.init(&ctx).unwrap();

        // Push 512 items — should trigger exactly two flushes of 256.
        let mut inbox = VecDequeInbox::new(1024);
        for i in 0..512i64 {
            inbox.push(rmpv::Value::Integer(i.into()));
        }
        let mut outbox = VecDequeOutbox::new(1, 1024);
        proc.process(0, &mut inbox, &mut outbox).unwrap();

        let mut batch_count = 0;
        while let Ok(msg) = rx.try_recv() {
            if let ClusterMessage::DagData(p) = msg {
                let items: Vec<rmpv::Value> = rmp_serde::from_slice(&p.items).expect("deserialize");
                assert_eq!(items.len(), 256, "each batch should have 256 items");
                batch_count += 1;
            }
        }
        assert_eq!(batch_count, 2, "512 items => 2 batches of 256");
    }

    // --- AC2: NetworkSenderProcessor::complete() flushes remainder ---

    #[test]
    fn sender_flushes_remainder_on_complete() {
        let (tx, mut rx) = make_transport();
        let mut proc = NetworkSenderProcessor::new(
            "node-2".to_string(),
            "exec-1".to_string(),
            "scan".to_string(),
            "combine".to_string(),
            tx,
        );
        let ctx = make_context();
        proc.init(&ctx).unwrap();

        // Push 100 items (< 256) — no flush during process().
        let mut inbox = VecDequeInbox::new(256);
        for i in 0..100i64 {
            inbox.push(rmpv::Value::Integer(i.into()));
        }
        let mut outbox = VecDequeOutbox::new(1, 256);
        let done = proc.process(0, &mut inbox, &mut outbox).unwrap();
        assert!(!done);

        // No messages yet.
        assert!(rx.try_recv().is_err(), "no flush before complete()");

        // complete() should flush the 100 buffered items.
        let done = proc.complete(&mut outbox).unwrap();
        assert!(done);

        let msg = rx
            .try_recv()
            .expect("final batch expected after complete()");
        if let ClusterMessage::DagData(payload) = msg {
            let items: Vec<rmpv::Value> =
                rmp_serde::from_slice(&payload.items).expect("deserialize");
            assert_eq!(
                items.len(),
                100,
                "all 100 remainder items should be flushed"
            );
        } else {
            panic!("expected DagData message");
        }
    }

    #[test]
    fn sender_complete_with_empty_buffer_does_not_send() {
        let (tx, mut rx) = make_transport();
        let mut proc = NetworkSenderProcessor::new(
            "node-2".to_string(),
            "exec-1".to_string(),
            "scan".to_string(),
            "combine".to_string(),
            tx,
        );
        let ctx = make_context();
        proc.init(&ctx).unwrap();

        let mut outbox = VecDequeOutbox::new(1, 4);
        let done = proc.complete(&mut outbox).unwrap();
        assert!(done);

        // No messages should be sent for an empty buffer.
        assert!(rx.try_recv().is_err(), "no message for empty buffer");
    }

    // --- AC3: NetworkReceiverProcessor emits to outbox ---

    #[test]
    fn receiver_emits_batches_to_outbox() {
        // Create the channel directly so we fully control both ends.
        let (tx, rx) = mpsc::channel::<Vec<rmpv::Value>>(1024);
        let mut proc = NetworkReceiverProcessor::new(rx);
        let ctx = make_context();
        proc.init(&ctx).unwrap();

        // Send two batches of 3 items each.
        let batch1 = vec![
            rmpv::Value::Integer(1.into()),
            rmpv::Value::Integer(2.into()),
            rmpv::Value::Integer(3.into()),
        ];
        let batch2 = vec![
            rmpv::Value::Integer(4.into()),
            rmpv::Value::Integer(5.into()),
            rmpv::Value::Integer(6.into()),
        ];
        tx.try_send(batch1).expect("send batch1");
        tx.try_send(batch2).expect("send batch2");
        // Drop the only sender so the channel transitions to Disconnected after drain.
        drop(tx);

        let mut inbox = VecDequeInbox::new(1);
        let mut outbox = VecDequeOutbox::new(1, 32);

        // process() drains both batches, then sees Disconnected => returns true.
        let done = proc.process(0, &mut inbox, &mut outbox).unwrap();
        assert!(
            done,
            "receiver should report done when channel is disconnected"
        );

        let items: Vec<_> = outbox.drain_bucket(0).collect();
        assert_eq!(items.len(), 6);
        for (i, item) in items.iter().enumerate() {
            #[allow(clippy::cast_possible_wrap)]
            let expected = rmpv::Value::Integer(((i + 1) as i64).into());
            assert_eq!(*item, expected);
        }
    }

    #[test]
    fn receiver_returns_false_when_channel_empty() {
        let supplier = NetworkReceiverProcessorSupplier::new();
        let _sender = supplier.sender(); // keep sender alive (channel not disconnected)

        let mut processors = supplier.get(1);
        let mut proc = processors.remove(0);
        let ctx = make_context();
        proc.init(&ctx).unwrap();

        let mut inbox = VecDequeInbox::new(1);
        let mut outbox = VecDequeOutbox::new(1, 4);

        // No items sent; channel is empty but connected.
        let done = proc.process(0, &mut inbox, &mut outbox).unwrap();
        assert!(
            !done,
            "should return false when channel is empty but not disconnected"
        );
    }

    // --- AC4: Payload serialization round-trips ---

    #[test]
    fn dag_execute_payload_roundtrip() {
        let payload = DagExecutePayload {
            execution_id: "exec-42".to_string(),
            plan: vec![0xDE, 0xAD, 0xBE, 0xEF],
        };
        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: DagExecutePayload = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(decoded, payload);
    }

    #[test]
    fn dag_data_payload_roundtrip() {
        let inner_items: Vec<rmpv::Value> = vec![
            rmpv::Value::Integer(1.into()),
            rmpv::Value::String("hello".into()),
        ];
        let items_bytes = rmp_serde::to_vec_named(&inner_items).expect("serialize items");
        let payload = DagDataPayload {
            execution_id: "exec-42".to_string(),
            source_vertex: "scan".to_string(),
            dest_vertex: "combine".to_string(),
            items: items_bytes,
        };
        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: DagDataPayload = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(decoded, payload);
        // Also verify the inner items decode correctly.
        let decoded_items: Vec<rmpv::Value> =
            rmp_serde::from_slice(&decoded.items).expect("deserialize items");
        assert_eq!(decoded_items.len(), 2);
    }

    #[test]
    fn dag_complete_payload_roundtrip() {
        let payload = DagCompletePayload {
            execution_id: "exec-42".to_string(),
            node_id: "node-1".to_string(),
            success: true,
            error: None,
            results: Some(vec![1, 2, 3]),
        };
        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: DagCompletePayload = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(decoded, payload);
    }

    #[test]
    fn dag_complete_payload_roundtrip_with_error() {
        let payload = DagCompletePayload {
            execution_id: "exec-7".to_string(),
            node_id: "node-2".to_string(),
            success: false,
            error: Some("timeout".to_string()),
            results: None,
        };
        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: DagCompletePayload = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(decoded, payload);
        assert_eq!(decoded.error, Some("timeout".to_string()));
        assert!(decoded.results.is_none());
    }

    // --- AC5: DagCompletePayload derives Default ---

    #[test]
    fn dag_complete_payload_default() {
        let d = DagCompletePayload::default();
        assert!(d.execution_id.is_empty());
        assert!(d.node_id.is_empty());
        assert!(!d.success);
        assert!(d.error.is_none());
        assert!(d.results.is_none());
    }

    // --- Supplier: single-use constraint ---

    #[test]
    fn receiver_supplier_get_called_once() {
        let supplier = NetworkReceiverProcessorSupplier::new();
        let processors = supplier.get(1);
        assert_eq!(processors.len(), 1);
    }

    #[test]
    #[should_panic(expected = "called more than once")]
    fn receiver_supplier_get_panics_on_second_call() {
        let supplier = NetworkReceiverProcessorSupplier::new();
        let _ = supplier.get(1);
        let _ = supplier.get(1); // should panic
    }

    // --- NetworkSenderProcessorSupplier ---

    #[test]
    fn sender_supplier_creates_multiple_processors() {
        let (tx, _rx) = make_transport();
        let supplier = NetworkSenderProcessorSupplier::new(
            "node-2".to_string(),
            "exec-1".to_string(),
            "scan".to_string(),
            "combine".to_string(),
            tx,
        );
        let processors = supplier.get(4);
        assert_eq!(processors.len(), 4);
    }
}
