//! Standard query processors for the DAG execution engine.
//!
//! Implements the standard processors used in query pipelines:
//! - `ScanProcessor`: reads records from `RecordStoreFactory`, emits as `rmpv::Value`
//! - `FilterProcessor`: evaluates a `PredicateNode` against each item
//! - `ProjectProcessor`: retains only the specified fields from each item
//! - `AggregateProcessor`: two-phase GROUP BY aggregation (local pre-aggregate)
//! - `CombineProcessor`: merges partial aggregates from multiple nodes
//! - `SortProcessor`: buffers all items, sorts by multi-field sort keys on complete
//! - `LimitProcessor`: passes through at most N items, then signals completion
//! - `CollectorProcessor`: sink that accumulates all results
//!
//! Each processor has a corresponding supplier (`*ProcessorSupplier`) that
//! implements `ProcessorSupplier` to create instances for the executor.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use topgun_core::messages::base::{PredicateNode, SortDirection};

use crate::dag::types::{Inbox, Outbox, Processor, ProcessorContext, ProcessorSupplier};
use crate::service::domain::predicate::{evaluate_predicate, EvalContext};
use crate::storage::factory::RecordStoreFactory;
use crate::storage::record::RecordValue;

// ---------------------------------------------------------------------------
// Processor batch size
// ---------------------------------------------------------------------------

/// Number of items emitted per `process()` call to ensure cooperative yielding.
const BATCH_SIZE: usize = 1024;

// ---------------------------------------------------------------------------
// AggregatorState
// ---------------------------------------------------------------------------

/// Partial aggregate state for a single GROUP BY key.
///
/// Accumulates partial results locally during phase 1 (`AggregateProcessor::process()`).
/// Combined across nodes during phase 2 (`CombineProcessor::process()`).
pub struct AggregatorState {
    pub count: u64,
    pub sum: f64,
    pub min: Option<rmpv::Value>,
    pub max: Option<rmpv::Value>,
}

impl AggregatorState {
    fn new() -> Self {
        Self {
            count: 0,
            sum: 0.0,
            min: None,
            max: None,
        }
    }

    fn update(&mut self, value: f64, raw: rmpv::Value) {
        self.count += 1;
        self.sum += value;

        // Update min
        self.min = Some(match &self.min {
            None => raw.clone(),
            Some(current) => {
                let cur_f = rmpv_to_f64(current).unwrap_or(f64::MAX);
                if value < cur_f { raw.clone() } else { current.clone() }
            }
        });

        // Update max
        self.max = Some(match &self.max {
            None => raw.clone(),
            Some(current) => {
                let cur_f = rmpv_to_f64(current).unwrap_or(f64::MIN);
                if value > cur_f { raw } else { current.clone() }
            }
        });
    }
}

// ---------------------------------------------------------------------------
// Helper: extract a named field value from an rmpv::Value Map
// ---------------------------------------------------------------------------

fn get_field<'a>(item: &'a rmpv::Value, field: &str) -> Option<&'a rmpv::Value> {
    if let rmpv::Value::Map(pairs) = item {
        for (k, v) in pairs {
            if let rmpv::Value::String(s) = k {
                if s.as_str() == Some(field) {
                    return Some(v);
                }
            }
        }
    }
    None
}

fn rmpv_to_f64(v: &rmpv::Value) -> Option<f64> {
    match v {
        rmpv::Value::Integer(i) => Some(i.as_f64().unwrap_or(0.0)),
        rmpv::Value::F32(f) => Some(f64::from(*f)),
        rmpv::Value::F64(f) => Some(*f),
        _ => None,
    }
}

fn rmpv_to_key_part(v: &rmpv::Value) -> String {
    match v {
        rmpv::Value::String(s) => s.as_str().unwrap_or("").to_string(),
        rmpv::Value::Integer(i) => i.to_string(),
        rmpv::Value::Boolean(b) => b.to_string(),
        rmpv::Value::F64(f) => format!("{f}"),
        rmpv::Value::F32(f) => format!("{f}"),
        rmpv::Value::Nil => String::new(),
        _ => format!("{v:?}"),
    }
}

fn group_key_string(item: &rmpv::Value, group_by: &[String]) -> String {
    let mut parts = Vec::with_capacity(group_by.len());
    for field in group_by {
        let val = get_field(item, field)
            .map(rmpv_to_key_part)
            .unwrap_or_default();
        parts.push(val);
    }
    parts.join("|")
}

// ---------------------------------------------------------------------------
// ScanProcessor
// ---------------------------------------------------------------------------

/// Source processor that reads all records from a named map and assigned
/// partitions, emitting them as `rmpv::Value` items in batches of 1024.
///
/// `RecordStore::for_each_boxed` yields `RecordValue` items containing
/// `topgun_core::types::Value`. The bridge to `rmpv::Value` is done via
/// `rmp_serde::to_value(&record)` (serialize through `MsgPack`) because adding
/// a reverse `From` impl in `core-rust` would require modifying a file outside
/// this spec's scope.
pub struct ScanProcessor {
    map_name: String,
    factory: Arc<RecordStoreFactory>,
    partition_ids: Vec<u32>,
    buffer: Vec<rmpv::Value>,
    cursor: usize,
    initialized: bool,
    done: bool,
}

impl ScanProcessor {
    fn new(map_name: String, factory: Arc<RecordStoreFactory>) -> Self {
        Self {
            map_name,
            factory,
            partition_ids: Vec::new(),
            buffer: Vec::new(),
            cursor: 0,
            initialized: false,
            done: false,
        }
    }
}

impl Processor for ScanProcessor {
    fn init(&mut self, context: &ProcessorContext) -> Result<()> {
        self.partition_ids.clone_from(&context.partition_ids);
        // Pre-load all records from all assigned partitions into the buffer.
        for &pid in &self.partition_ids {
            let store = self.factory.get_or_create(&self.map_name, pid);
            store.for_each_boxed(
                &mut |_key, record| {
                    // Bridge topgun_core::types::Value -> rmpv::Value via MsgPack round-trip.
                    if let RecordValue::Lww { value, .. } = &record.value {
                        if let Ok(bytes) = rmp_serde::to_vec_named(value) {
                            if let Ok(rmpv_val) = rmp_serde::from_slice::<rmpv::Value>(&bytes) {
                                self.buffer.push(rmpv_val);
                            }
                        }
                    }
                },
                false,
            );
        }
        self.initialized = true;
        Ok(())
    }

    fn process(
        &mut self,
        _ordinal: u32,
        _inbox: &mut dyn Inbox,
        outbox: &mut dyn Outbox,
    ) -> Result<bool> {
        if self.done {
            return Ok(true);
        }
        let end = (self.cursor + BATCH_SIZE).min(self.buffer.len());
        for i in self.cursor..end {
            let item = self.buffer[i].clone();
            outbox.offer(0, item);
        }
        self.cursor = end;
        if self.cursor >= self.buffer.len() {
            self.done = true;
        }
        Ok(self.done)
    }

    fn complete(&mut self, _outbox: &mut dyn Outbox) -> Result<bool> {
        Ok(true)
    }

    fn is_cooperative(&self) -> bool {
        true
    }

    fn close(&mut self) {
        self.buffer.clear();
    }
}

/// Supplier for `ScanProcessor`.
pub struct ScanProcessorSupplier {
    pub map_name: String,
    pub factory: Arc<RecordStoreFactory>,
}

impl ProcessorSupplier for ScanProcessorSupplier {
    fn get(&self, count: u32) -> Vec<Box<dyn Processor>> {
        (0..count)
            .map(|_| {
                Box::new(ScanProcessor::new(self.map_name.clone(), Arc::clone(&self.factory)))
                    as Box<dyn Processor>
            })
            .collect()
    }

    fn clone_supplier(&self) -> Box<dyn ProcessorSupplier> {
        Box::new(ScanProcessorSupplier {
            map_name: self.map_name.clone(),
            factory: Arc::clone(&self.factory),
        })
    }
}

// ---------------------------------------------------------------------------
// FilterProcessor
// ---------------------------------------------------------------------------

/// Drains inbox items and emits those matching the predicate.
///
/// Uses `evaluate_predicate` from `crate::service::domain::predicate`.
pub struct FilterProcessor {
    predicate: PredicateNode,
}

impl FilterProcessor {
    fn new(predicate: PredicateNode) -> Self {
        Self { predicate }
    }
}

impl Processor for FilterProcessor {
    fn init(&mut self, _context: &ProcessorContext) -> Result<()> {
        Ok(())
    }

    fn process(
        &mut self,
        _ordinal: u32,
        inbox: &mut dyn Inbox,
        outbox: &mut dyn Outbox,
    ) -> Result<bool> {
        // Poll items one at a time up to BATCH_SIZE to avoid consuming items
        // beyond the batch limit (drain() would remove ALL items permanently).
        for _ in 0..BATCH_SIZE {
            let Some(item) = inbox.poll() else { break };
            if evaluate_predicate(&self.predicate, &EvalContext::data_only(&item)) {
                outbox.offer(0, item);
            }
        }
        Ok(false) // filter never completes on its own; executor marks done when upstream completes
    }

    fn complete(&mut self, _outbox: &mut dyn Outbox) -> Result<bool> {
        Ok(true)
    }

    fn is_cooperative(&self) -> bool {
        true
    }

    fn close(&mut self) {}
}

/// Supplier for `FilterProcessor`.
pub struct FilterProcessorSupplier {
    pub predicate: PredicateNode,
}

impl ProcessorSupplier for FilterProcessorSupplier {
    fn get(&self, count: u32) -> Vec<Box<dyn Processor>> {
        (0..count)
            .map(|_| {
                Box::new(FilterProcessor::new(self.predicate.clone())) as Box<dyn Processor>
            })
            .collect()
    }

    fn clone_supplier(&self) -> Box<dyn ProcessorSupplier> {
        Box::new(FilterProcessorSupplier {
            predicate: self.predicate.clone(),
        })
    }
}

// ---------------------------------------------------------------------------
// ProjectProcessor
// ---------------------------------------------------------------------------

/// Drains inbox items and emits only the specified fields from each `rmpv::Value` Map.
///
/// Used only in SQL-derived DAGs (`DataFusion` SQL path). The base `Query` struct
/// has no projection field, so this processor is not used in predicate-only paths.
pub struct ProjectProcessor {
    fields: Vec<String>,
}

impl ProjectProcessor {
    fn new(fields: Vec<String>) -> Self {
        Self { fields }
    }
}

impl Processor for ProjectProcessor {
    fn init(&mut self, _context: &ProcessorContext) -> Result<()> {
        Ok(())
    }

    fn process(
        &mut self,
        _ordinal: u32,
        inbox: &mut dyn Inbox,
        outbox: &mut dyn Outbox,
    ) -> Result<bool> {
        let fields = &self.fields;
        inbox.drain(&mut |item| {
            if let rmpv::Value::Map(pairs) = item {
                let projected: Vec<(rmpv::Value, rmpv::Value)> = pairs
                    .into_iter()
                    .filter(|(k, _)| {
                        if let rmpv::Value::String(s) = k {
                            fields.iter().any(|f| s.as_str() == Some(f.as_str()))
                        } else {
                            false
                        }
                    })
                    .collect();
                outbox.offer(0, rmpv::Value::Map(projected));
            } else {
                outbox.offer(0, item);
            }
        });
        Ok(false)
    }

    fn complete(&mut self, _outbox: &mut dyn Outbox) -> Result<bool> {
        Ok(true)
    }

    fn is_cooperative(&self) -> bool {
        true
    }

    fn close(&mut self) {}
}

/// Supplier for `ProjectProcessor`.
pub struct ProjectProcessorSupplier {
    pub fields: Vec<String>,
}

impl ProcessorSupplier for ProjectProcessorSupplier {
    fn get(&self, count: u32) -> Vec<Box<dyn Processor>> {
        (0..count)
            .map(|_| {
                Box::new(ProjectProcessor::new(self.fields.clone())) as Box<dyn Processor>
            })
            .collect()
    }

    fn clone_supplier(&self) -> Box<dyn ProcessorSupplier> {
        Box::new(ProjectProcessorSupplier {
            fields: self.fields.clone(),
        })
    }
}

// ---------------------------------------------------------------------------
// AggregateProcessor
// ---------------------------------------------------------------------------

/// Two-phase local pre-aggregation.
///
/// Phase 1 (`process()`): accumulates `AggregatorState` per GROUP BY key.
/// Phase 2 (`complete()`): emits partial aggregates as `rmpv::Value` Maps to
/// the outbox for downstream `CombineProcessor`.
///
/// Emitted aggregate map fields:
/// - `__key`: GROUP BY key string
/// - `__count`: u64 count
/// - `__sum`: f64 sum
/// - `__min`: min value (or Nil)
/// - `__max`: max value (or Nil)
/// - one field per `group_by` column with its sampled value
pub struct AggregateProcessor {
    group_by: Vec<String>,
    agg_field: String,
    /// GROUP BY key string -> partial aggregate
    partial: HashMap<String, AggregatorState>,
    done: bool,
}

impl AggregateProcessor {
    fn new(group_by: Vec<String>, agg_field: String) -> Self {
        Self {
            group_by,
            agg_field,
            partial: HashMap::new(),
            done: false,
        }
    }
}

impl Processor for AggregateProcessor {
    fn init(&mut self, _context: &ProcessorContext) -> Result<()> {
        Ok(())
    }

    fn process(
        &mut self,
        _ordinal: u32,
        inbox: &mut dyn Inbox,
        _outbox: &mut dyn Outbox,
    ) -> Result<bool> {
        let group_by = &self.group_by;
        let agg_field = &self.agg_field;
        inbox.drain(&mut |item| {
            let key = group_key_string(&item, group_by);
            let state = self.partial.entry(key).or_insert_with(AggregatorState::new);
            // Aggregate the numeric value of agg_field; count all if no specific agg_field
            let numeric = if agg_field.is_empty() {
                1.0 // COUNT(*) — always increment by 1
            } else {
                get_field(&item, agg_field)
                    .and_then(rmpv_to_f64)
                    .unwrap_or(0.0)
            };
            let raw = get_field(&item, agg_field)
                .cloned()
                .unwrap_or(rmpv::Value::Nil);
            state.update(numeric, raw);
        });
        Ok(false)
    }

    fn complete(&mut self, outbox: &mut dyn Outbox) -> Result<bool> {
        if self.done {
            return Ok(true);
        }
        // Emit partial aggregates
        for (key, state) in &self.partial {
            let mut pairs = vec![
                (
                    rmpv::Value::String("__key".into()),
                    rmpv::Value::String(key.clone().into()),
                ),
                (
                    rmpv::Value::String("__count".into()),
                    rmpv::Value::Integer(state.count.into()),
                ),
                (
                    rmpv::Value::String("__sum".into()),
                    rmpv::Value::F64(state.sum),
                ),
                (
                    rmpv::Value::String("__min".into()),
                    state.min.clone().unwrap_or(rmpv::Value::Nil),
                ),
                (
                    rmpv::Value::String("__max".into()),
                    state.max.clone().unwrap_or(rmpv::Value::Nil),
                ),
            ];
            // Also emit the GROUP BY field values for join-back
            for field in &self.group_by {
                pairs.push((
                    rmpv::Value::String(field.clone().into()),
                    rmpv::Value::String(key.clone().into()),
                ));
            }
            outbox.offer(0, rmpv::Value::Map(pairs));
        }
        self.done = true;
        Ok(true)
    }

    fn is_cooperative(&self) -> bool {
        true
    }

    fn close(&mut self) {
        self.partial.clear();
    }
}

/// Supplier for `AggregateProcessor`.
pub struct AggregateProcessorSupplier {
    pub group_by: Vec<String>,
    pub agg_field: String,
}

impl ProcessorSupplier for AggregateProcessorSupplier {
    fn get(&self, count: u32) -> Vec<Box<dyn Processor>> {
        (0..count)
            .map(|_| {
                Box::new(AggregateProcessor::new(
                    self.group_by.clone(),
                    self.agg_field.clone(),
                )) as Box<dyn Processor>
            })
            .collect()
    }

    fn clone_supplier(&self) -> Box<dyn ProcessorSupplier> {
        Box::new(AggregateProcessorSupplier {
            group_by: self.group_by.clone(),
            agg_field: self.agg_field.clone(),
        })
    }
}

// ---------------------------------------------------------------------------
// CombineProcessor
// ---------------------------------------------------------------------------

/// Merges partial aggregate maps emitted by `AggregateProcessor` instances
/// from multiple nodes or partitions.
///
/// Expects items with the `__key`, `__count`, `__sum`, `__min`, `__max` fields
/// produced by `AggregateProcessor::complete()`. Combines them by GROUP BY key,
/// producing one final aggregate row per unique key.
pub struct CombineProcessor {
    combined: HashMap<String, AggregatorState>,
    done: bool,
}

impl CombineProcessor {
    fn new() -> Self {
        Self {
            combined: HashMap::new(),
            done: false,
        }
    }
}

impl Processor for CombineProcessor {
    fn init(&mut self, _context: &ProcessorContext) -> Result<()> {
        Ok(())
    }

    fn process(
        &mut self,
        _ordinal: u32,
        inbox: &mut dyn Inbox,
        _outbox: &mut dyn Outbox,
    ) -> Result<bool> {
        inbox.drain(&mut |item| {
            let key = match get_field(&item, "__key") {
                Some(rmpv::Value::String(s)) => s.as_str().unwrap_or("").to_string(),
                _ => return,
            };
            let count = get_field(&item, "__count")
                .and_then(|v| if let rmpv::Value::Integer(i) = v { Some(i.as_u64().unwrap_or(0)) } else { None })
                .unwrap_or(0);
            let sum = get_field(&item, "__sum")
                .and_then(rmpv_to_f64)
                .unwrap_or(0.0);
            let min = get_field(&item, "__min").cloned();
            let max = get_field(&item, "__max").cloned();

            let state = self.combined.entry(key).or_insert_with(AggregatorState::new);
            state.count += count;
            state.sum += sum;

            // Merge min
            if let Some(new_min) = min {
                if new_min != rmpv::Value::Nil {
                    let new_f = rmpv_to_f64(&new_min).unwrap_or(f64::MAX);
                    state.min = Some(match &state.min {
                        None => new_min,
                        Some(cur) => {
                            let cur_f = rmpv_to_f64(cur).unwrap_or(f64::MAX);
                            if new_f < cur_f { new_min } else { cur.clone() }
                        }
                    });
                }
            }

            // Merge max
            if let Some(new_max) = max {
                if new_max != rmpv::Value::Nil {
                    let new_f = rmpv_to_f64(&new_max).unwrap_or(f64::MIN);
                    state.max = Some(match &state.max {
                        None => new_max,
                        Some(cur) => {
                            let cur_f = rmpv_to_f64(cur).unwrap_or(f64::MIN);
                            if new_f > cur_f { new_max } else { cur.clone() }
                        }
                    });
                }
            }
        });
        Ok(false)
    }

    fn complete(&mut self, outbox: &mut dyn Outbox) -> Result<bool> {
        if self.done {
            return Ok(true);
        }
        for (key, state) in &self.combined {
            let pairs = vec![
                (
                    rmpv::Value::String("__key".into()),
                    rmpv::Value::String(key.clone().into()),
                ),
                (
                    rmpv::Value::String("__count".into()),
                    rmpv::Value::Integer(state.count.into()),
                ),
                (
                    rmpv::Value::String("__sum".into()),
                    rmpv::Value::F64(state.sum),
                ),
                (
                    rmpv::Value::String("__min".into()),
                    state.min.clone().unwrap_or(rmpv::Value::Nil),
                ),
                (
                    rmpv::Value::String("__max".into()),
                    state.max.clone().unwrap_or(rmpv::Value::Nil),
                ),
            ];
            outbox.offer(0, rmpv::Value::Map(pairs));
        }
        self.done = true;
        Ok(true)
    }

    fn is_cooperative(&self) -> bool {
        true
    }

    fn close(&mut self) {
        self.combined.clear();
    }
}

/// Supplier for `CombineProcessor`.
pub struct CombineProcessorSupplier;

impl ProcessorSupplier for CombineProcessorSupplier {
    fn get(&self, count: u32) -> Vec<Box<dyn Processor>> {
        (0..count)
            .map(|_| Box::new(CombineProcessor::new()) as Box<dyn Processor>)
            .collect()
    }

    fn clone_supplier(&self) -> Box<dyn ProcessorSupplier> {
        Box::new(CombineProcessorSupplier)
    }
}

// ---------------------------------------------------------------------------
// CollectorProcessor
// ---------------------------------------------------------------------------

/// Sink processor. Accumulates all inbox items into an internal `Vec<rmpv::Value>`.
///
/// The executor retrieves the collected results via `take_results()` after the
/// pipeline completes.
pub struct CollectorProcessor {
    results: Vec<rmpv::Value>,
}

impl CollectorProcessor {
    #[must_use]
    pub fn new() -> Self {
        Self {
            results: Vec::new(),
        }
    }

    /// Drain and return all collected results.
    pub fn take_results(&mut self) -> Vec<rmpv::Value> {
        std::mem::take(&mut self.results)
    }
}

impl Default for CollectorProcessor {
    fn default() -> Self {
        Self::new()
    }
}

impl Processor for CollectorProcessor {
    fn init(&mut self, _context: &ProcessorContext) -> Result<()> {
        Ok(())
    }

    fn process(
        &mut self,
        _ordinal: u32,
        inbox: &mut dyn Inbox,
        _outbox: &mut dyn Outbox,
    ) -> Result<bool> {
        inbox.drain(&mut |item| {
            self.results.push(item);
        });
        Ok(false)
    }

    fn complete(&mut self, outbox: &mut dyn Outbox) -> Result<bool> {
        // Emit all accumulated results to outbox bucket 0 so the executor can
        // collect them via VecDequeOutbox::drain_bucket(0) after pipeline completion.
        for item in self.results.drain(..) {
            outbox.offer(0, item);
        }
        Ok(true)
    }

    fn is_cooperative(&self) -> bool {
        true
    }

    fn close(&mut self) {
        self.results.clear();
    }
}

/// Supplier for `CollectorProcessor`.
pub struct CollectorProcessorSupplier;

impl ProcessorSupplier for CollectorProcessorSupplier {
    fn get(&self, count: u32) -> Vec<Box<dyn Processor>> {
        (0..count)
            .map(|_| Box::new(CollectorProcessor::new()) as Box<dyn Processor>)
            .collect()
    }

    fn clone_supplier(&self) -> Box<dyn ProcessorSupplier> {
        Box::new(CollectorProcessorSupplier)
    }
}

// ---------------------------------------------------------------------------
// SortProcessor
// ---------------------------------------------------------------------------

/// Buffers all incoming items and sorts them on `complete()` by the given fields.
///
/// Sort comparison is multi-field: for each `(field, direction)` pair in order,
/// values are compared numerically when possible, falling back to lexicographic
/// string comparison. Nil/missing field values sort last regardless of direction.
/// Uses a stable sort to maintain deterministic ordering of equal elements.
pub struct SortProcessor {
    sort_fields: Vec<(String, SortDirection)>,
    buffer: Vec<rmpv::Value>,
    done: bool,
}

impl SortProcessor {
    fn new(sort_fields: Vec<(String, SortDirection)>) -> Self {
        Self {
            sort_fields,
            buffer: Vec::new(),
            done: false,
        }
    }
}

/// Compare two rmpv values for sorting, with nil/missing sorting last.
fn compare_sort_values(
    a: Option<&rmpv::Value>,
    b: Option<&rmpv::Value>,
    direction: &SortDirection,
) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    let a_nil = a.is_none() || matches!(a, Some(rmpv::Value::Nil));
    let b_nil = b.is_none() || matches!(b, Some(rmpv::Value::Nil));

    // Nil/missing values sort last regardless of direction
    match (a_nil, b_nil) {
        (true, true) => return Ordering::Equal,
        (true, false) => return Ordering::Greater,
        (false, true) => return Ordering::Less,
        (false, false) => {}
    }

    let a_val = a.unwrap();
    let b_val = b.unwrap();

    // Try numeric comparison first
    let a_num = rmpv_to_f64(a_val);
    let b_num = rmpv_to_f64(b_val);

    let cmp = match (a_num, b_num) {
        (Some(af), Some(bf)) => af.partial_cmp(&bf).unwrap_or(Ordering::Equal),
        (Some(_), None) => {
            // Numeric values sort before string values in ascending order
            Ordering::Less
        }
        (None, Some(_)) => {
            Ordering::Greater
        }
        (None, None) => {
            // Fall back to string comparison
            let a_str = rmpv_to_key_part(a_val);
            let b_str = rmpv_to_key_part(b_val);
            a_str.cmp(&b_str)
        }
    };

    match direction {
        SortDirection::Asc => cmp,
        SortDirection::Desc => cmp.reverse(),
    }
}

impl Processor for SortProcessor {
    fn init(&mut self, _context: &ProcessorContext) -> Result<()> {
        Ok(())
    }

    fn process(
        &mut self,
        _ordinal: u32,
        inbox: &mut dyn Inbox,
        _outbox: &mut dyn Outbox,
    ) -> Result<bool> {
        inbox.drain(&mut |item| {
            self.buffer.push(item);
        });
        // Never self-completes; waits for upstream to finish
        Ok(false)
    }

    fn complete(&mut self, outbox: &mut dyn Outbox) -> Result<bool> {
        if self.done {
            return Ok(true);
        }

        let sort_fields = &self.sort_fields;
        self.buffer.sort_by(|a, b| {
            for (field, direction) in sort_fields {
                let a_val = get_field(a, field);
                let b_val = get_field(b, field);
                let cmp = compare_sort_values(a_val, b_val, direction);
                if cmp != std::cmp::Ordering::Equal {
                    return cmp;
                }
            }
            std::cmp::Ordering::Equal
        });

        for item in self.buffer.drain(..) {
            outbox.offer(0, item);
        }

        self.done = true;
        Ok(true)
    }

    fn is_cooperative(&self) -> bool {
        true
    }

    fn close(&mut self) {
        self.buffer.clear();
    }
}

/// Supplier for `SortProcessor`.
pub struct SortProcessorSupplier {
    pub sort_fields: Vec<(String, SortDirection)>,
}

impl ProcessorSupplier for SortProcessorSupplier {
    fn get(&self, count: u32) -> Vec<Box<dyn Processor>> {
        (0..count)
            .map(|_| {
                Box::new(SortProcessor::new(self.sort_fields.clone())) as Box<dyn Processor>
            })
            .collect()
    }

    fn clone_supplier(&self) -> Box<dyn ProcessorSupplier> {
        Box::new(SortProcessorSupplier {
            sort_fields: self.sort_fields.clone(),
        })
    }
}

// ---------------------------------------------------------------------------
// LimitProcessor
// ---------------------------------------------------------------------------

/// Passes through at most `limit` items, then signals completion.
///
/// Edge case: `limit: 0` immediately returns `true` without polling any items.
pub struct LimitProcessor {
    limit: u32,
    emitted: u32,
}

impl LimitProcessor {
    fn new(limit: u32) -> Self {
        Self { limit, emitted: 0 }
    }
}

impl Processor for LimitProcessor {
    fn init(&mut self, _context: &ProcessorContext) -> Result<()> {
        Ok(())
    }

    fn process(
        &mut self,
        _ordinal: u32,
        inbox: &mut dyn Inbox,
        outbox: &mut dyn Outbox,
    ) -> Result<bool> {
        if self.emitted >= self.limit {
            return Ok(true);
        }

        while self.emitted < self.limit {
            let Some(item) = inbox.poll() else { break };
            outbox.offer(0, item);
            self.emitted += 1;
        }

        Ok(self.emitted >= self.limit)
    }

    fn complete(&mut self, _outbox: &mut dyn Outbox) -> Result<bool> {
        Ok(true)
    }

    fn is_cooperative(&self) -> bool {
        true
    }

    fn close(&mut self) {}
}

/// Supplier for `LimitProcessor`.
pub struct LimitProcessorSupplier {
    pub limit: u32,
}

impl ProcessorSupplier for LimitProcessorSupplier {
    fn get(&self, count: u32) -> Vec<Box<dyn Processor>> {
        (0..count)
            .map(|_| Box::new(LimitProcessor::new(self.limit)) as Box<dyn Processor>)
            .collect()
    }

    fn clone_supplier(&self) -> Box<dyn ProcessorSupplier> {
        Box::new(LimitProcessorSupplier { limit: self.limit })
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::map_unwrap_or,
)]
mod tests {
    use topgun_core::messages::base::{PredicateNode, PredicateOp};

    use super::*;
    use crate::dag::executor::{VecDequeInbox, VecDequeOutbox};
    use crate::dag::types::ProcessorContext;

    fn make_context() -> ProcessorContext {
        ProcessorContext {
            node_id: "test-node".to_string(),
            global_processor_index: 0,
            local_processor_index: 0,
            total_parallelism: 1,
            vertex_name: "test".to_string(),
            partition_ids: vec![0],
        }
    }

    fn make_map_item(pairs: &[(&str, rmpv::Value)]) -> rmpv::Value {
        rmpv::Value::Map(
            pairs
                .iter()
                .map(|(k, v)| (rmpv::Value::String((*k).into()), v.clone()))
                .collect(),
        )
    }

    fn get_f64_field(item: &rmpv::Value, field: &str) -> Option<f64> {
        get_field(item, field).and_then(rmpv_to_f64)
    }

    // --- FilterProcessor ---

    #[test]
    fn filter_passes_matching_items() {
        let predicate = PredicateNode {
            op: PredicateOp::Eq,
            attribute: Some("status".to_string()),
            value: Some(rmpv::Value::String("active".into())),
            ..Default::default()
        };

        let mut proc = FilterProcessor::new(predicate);
        let ctx = make_context();
        proc.init(&ctx).unwrap();

        let mut inbox = VecDequeInbox::new(16);
        inbox.push(make_map_item(&[("status", rmpv::Value::String("active".into()))]));
        inbox.push(make_map_item(&[("status", rmpv::Value::String("inactive".into()))]));
        inbox.push(make_map_item(&[("status", rmpv::Value::String("active".into()))]));

        let mut outbox = VecDequeOutbox::new(1, 16);
        let done = proc.process(0, &mut inbox, &mut outbox).unwrap();
        assert!(!done); // filter never self-completes

        let drained: Vec<_> = outbox.drain_bucket(0).collect();
        assert_eq!(drained.len(), 2, "only 2 active items should pass");
    }

    // --- ProjectProcessor ---

    #[test]
    fn project_retains_only_specified_fields() {
        let mut proc = ProjectProcessor::new(vec!["name".to_string()]);
        let ctx = make_context();
        proc.init(&ctx).unwrap();

        let mut inbox = VecDequeInbox::new(4);
        inbox.push(make_map_item(&[
            ("name", rmpv::Value::String("Alice".into())),
            ("age", rmpv::Value::Integer(30.into())),
        ]));

        let mut outbox = VecDequeOutbox::new(1, 4);
        proc.process(0, &mut inbox, &mut outbox).unwrap();

        let items: Vec<_> = outbox.drain_bucket(0).collect();
        assert_eq!(items.len(), 1);
        if let rmpv::Value::Map(pairs) = &items[0] {
            assert_eq!(pairs.len(), 1, "only 'name' should remain");
            assert_eq!(pairs[0].0, rmpv::Value::String("name".into()));
        } else {
            panic!("expected Map");
        }
    }

    // --- AggregateProcessor ---

    #[test]
    fn aggregate_groups_by_field() {
        let mut proc = AggregateProcessor::new(vec!["category".to_string()], String::new());
        let ctx = make_context();
        proc.init(&ctx).unwrap();

        let mut inbox = VecDequeInbox::new(16);
        for _ in 0..3 {
            inbox.push(make_map_item(&[("category", rmpv::Value::String("A".into()))]));
        }
        for _ in 0..2 {
            inbox.push(make_map_item(&[("category", rmpv::Value::String("B".into()))]));
        }

        let mut outbox = VecDequeOutbox::new(1, 16);
        proc.process(0, &mut inbox, &mut outbox).unwrap();

        let mut emit_outbox = VecDequeOutbox::new(1, 16);
        let done = proc.complete(&mut emit_outbox).unwrap();
        assert!(done);

        let items: Vec<_> = emit_outbox.drain_bucket(0).collect();
        assert_eq!(items.len(), 2, "two distinct groups (A and B)");

        // Find group A
        let group_a = items.iter().find(|item| {
            get_field(item, "__key")
                .map(|v| v == &rmpv::Value::String("A".into()))
                .unwrap_or(false)
        });
        assert!(group_a.is_some(), "group A should be in results");
        assert_eq!(
            get_f64_field(group_a.unwrap(), "__count").unwrap() as u64,
            3
        );

        let group_b = items.iter().find(|item| {
            get_field(item, "__key")
                .map(|v| v == &rmpv::Value::String("B".into()))
                .unwrap_or(false)
        });
        assert!(group_b.is_some(), "group B should be in results");
        assert_eq!(
            get_f64_field(group_b.unwrap(), "__count").unwrap() as u64,
            2
        );
    }

    // --- CombineProcessor ---

    #[test]
    fn combine_merges_partial_aggregates() {
        let mut proc = CombineProcessor::new();
        let ctx = make_context();
        proc.init(&ctx).unwrap();

        // Two partial aggregates for group "A" with count=3 and count=2.
        let mut inbox = VecDequeInbox::new(8);
        inbox.push(rmpv::Value::Map(vec![
            (rmpv::Value::String("__key".into()), rmpv::Value::String("A".into())),
            (rmpv::Value::String("__count".into()), rmpv::Value::Integer(3u64.into())),
            (rmpv::Value::String("__sum".into()), rmpv::Value::F64(30.0)),
            (rmpv::Value::String("__min".into()), rmpv::Value::Integer(5.into())),
            (rmpv::Value::String("__max".into()), rmpv::Value::Integer(15.into())),
        ]));
        inbox.push(rmpv::Value::Map(vec![
            (rmpv::Value::String("__key".into()), rmpv::Value::String("A".into())),
            (rmpv::Value::String("__count".into()), rmpv::Value::Integer(2u64.into())),
            (rmpv::Value::String("__sum".into()), rmpv::Value::F64(20.0)),
            (rmpv::Value::String("__min".into()), rmpv::Value::Integer(3.into())),
            (rmpv::Value::String("__max".into()), rmpv::Value::Integer(12.into())),
        ]));

        let mut outbox = VecDequeOutbox::new(1, 8);
        proc.process(0, &mut inbox, &mut outbox).unwrap();

        let mut emit_outbox = VecDequeOutbox::new(1, 8);
        proc.complete(&mut emit_outbox).unwrap();

        let items: Vec<_> = emit_outbox.drain_bucket(0).collect();
        assert_eq!(items.len(), 1);

        // Combined: count=5, sum=50.0
        assert_eq!(
            get_f64_field(&items[0], "__count").unwrap() as u64,
            5
        );
        assert!((get_f64_field(&items[0], "__sum").unwrap() - 50.0).abs() < 1e-9);
        // min=3, max=15 after merge
        assert_eq!(get_f64_field(&items[0], "__min").unwrap() as i64, 3);
        assert_eq!(get_f64_field(&items[0], "__max").unwrap() as i64, 15);
    }

    // --- CollectorProcessor ---

    #[test]
    fn collector_accumulates_all_items() {
        let mut proc = CollectorProcessor::new();
        let ctx = make_context();
        proc.init(&ctx).unwrap();

        let mut inbox = VecDequeInbox::new(8);
        for i in 0..5i64 {
            inbox.push(rmpv::Value::Integer(i.into()));
        }

        let mut outbox = VecDequeOutbox::new(1, 8);
        proc.process(0, &mut inbox, &mut outbox).unwrap();
        // complete() emits accumulated results to outbox bucket 0.
        proc.complete(&mut outbox).unwrap();

        let results: Vec<_> = outbox.drain_bucket(0).collect();
        assert_eq!(results.len(), 5);
        for (i, item) in results.iter().enumerate() {
            assert_eq!(*item, rmpv::Value::Integer((i as i64).into()));
        }
    }

    #[test]
    fn collector_take_results_clears_internal_vec() {
        let mut proc = CollectorProcessor::new();
        let ctx = make_context();
        proc.init(&ctx).unwrap();

        let mut inbox = VecDequeInbox::new(4);
        inbox.push(rmpv::Value::Boolean(true));
        let mut outbox = VecDequeOutbox::new(1, 4);
        proc.process(0, &mut inbox, &mut outbox).unwrap();

        let first = proc.take_results();
        assert_eq!(first.len(), 1);

        let second = proc.take_results();
        assert!(second.is_empty(), "take_results should clear the vec");
    }

    // --- SortProcessor ---

    #[test]
    fn sort_ascending_by_age() {
        let mut proc = SortProcessor::new(vec![
            ("age".to_string(), SortDirection::Asc),
        ]);
        let ctx = make_context();
        proc.init(&ctx).unwrap();

        let mut inbox = VecDequeInbox::new(16);
        inbox.push(make_map_item(&[("age", rmpv::Value::Integer(30.into()))]));
        inbox.push(make_map_item(&[("age", rmpv::Value::Integer(10.into()))]));
        inbox.push(make_map_item(&[("age", rmpv::Value::Integer(20.into()))]));

        let mut outbox = VecDequeOutbox::new(1, 16);
        let done = proc.process(0, &mut inbox, &mut outbox).unwrap();
        assert!(!done, "sort should not self-complete during process");

        let mut emit_outbox = VecDequeOutbox::new(1, 16);
        let done = proc.complete(&mut emit_outbox).unwrap();
        assert!(done);

        let items: Vec<_> = emit_outbox.drain_bucket(0).collect();
        assert_eq!(items.len(), 3);
        assert_eq!(get_f64_field(&items[0], "age").unwrap() as i64, 10);
        assert_eq!(get_f64_field(&items[1], "age").unwrap() as i64, 20);
        assert_eq!(get_f64_field(&items[2], "age").unwrap() as i64, 30);
    }

    #[test]
    fn sort_descending_by_age() {
        let mut proc = SortProcessor::new(vec![
            ("age".to_string(), SortDirection::Desc),
        ]);
        let ctx = make_context();
        proc.init(&ctx).unwrap();

        let mut inbox = VecDequeInbox::new(16);
        inbox.push(make_map_item(&[("age", rmpv::Value::Integer(30.into()))]));
        inbox.push(make_map_item(&[("age", rmpv::Value::Integer(10.into()))]));
        inbox.push(make_map_item(&[("age", rmpv::Value::Integer(20.into()))]));

        let mut outbox = VecDequeOutbox::new(1, 16);
        proc.process(0, &mut inbox, &mut outbox).unwrap();

        let mut emit_outbox = VecDequeOutbox::new(1, 16);
        proc.complete(&mut emit_outbox).unwrap();

        let items: Vec<_> = emit_outbox.drain_bucket(0).collect();
        assert_eq!(items.len(), 3);
        assert_eq!(get_f64_field(&items[0], "age").unwrap() as i64, 30);
        assert_eq!(get_f64_field(&items[1], "age").unwrap() as i64, 20);
        assert_eq!(get_f64_field(&items[2], "age").unwrap() as i64, 10);
    }

    #[test]
    fn sort_multi_field_status_asc_then_age_desc() {
        let mut proc = SortProcessor::new(vec![
            ("status".to_string(), SortDirection::Asc),
            ("age".to_string(), SortDirection::Desc),
        ]);
        let ctx = make_context();
        proc.init(&ctx).unwrap();

        let mut inbox = VecDequeInbox::new(16);
        inbox.push(make_map_item(&[
            ("status", rmpv::Value::String("b".into())),
            ("age", rmpv::Value::Integer(25.into())),
        ]));
        inbox.push(make_map_item(&[
            ("status", rmpv::Value::String("a".into())),
            ("age", rmpv::Value::Integer(20.into())),
        ]));
        inbox.push(make_map_item(&[
            ("status", rmpv::Value::String("a".into())),
            ("age", rmpv::Value::Integer(30.into())),
        ]));
        inbox.push(make_map_item(&[
            ("status", rmpv::Value::String("b".into())),
            ("age", rmpv::Value::Integer(10.into())),
        ]));

        let mut outbox = VecDequeOutbox::new(1, 16);
        proc.process(0, &mut inbox, &mut outbox).unwrap();

        let mut emit_outbox = VecDequeOutbox::new(1, 16);
        proc.complete(&mut emit_outbox).unwrap();

        let items: Vec<_> = emit_outbox.drain_bucket(0).collect();
        assert_eq!(items.len(), 4);

        // status="a" first (asc), then within "a" age desc: 30, 20
        assert_eq!(get_field(&items[0], "status").unwrap().as_str(), Some("a"));
        assert_eq!(get_f64_field(&items[0], "age").unwrap() as i64, 30);
        assert_eq!(get_field(&items[1], "status").unwrap().as_str(), Some("a"));
        assert_eq!(get_f64_field(&items[1], "age").unwrap() as i64, 20);

        // status="b" second, age desc: 25, 10
        assert_eq!(get_field(&items[2], "status").unwrap().as_str(), Some("b"));
        assert_eq!(get_f64_field(&items[2], "age").unwrap() as i64, 25);
        assert_eq!(get_field(&items[3], "status").unwrap().as_str(), Some("b"));
        assert_eq!(get_f64_field(&items[3], "age").unwrap() as i64, 10);
    }

    #[test]
    fn sort_nil_missing_fields_sort_last() {
        let mut proc = SortProcessor::new(vec![
            ("score".to_string(), SortDirection::Asc),
        ]);
        let ctx = make_context();
        proc.init(&ctx).unwrap();

        let mut inbox = VecDequeInbox::new(16);
        inbox.push(make_map_item(&[("score", rmpv::Value::Nil)]));
        inbox.push(make_map_item(&[("score", rmpv::Value::Integer(5.into()))]));
        inbox.push(make_map_item(&[("name", rmpv::Value::String("no_score".into()))])); // missing field
        inbox.push(make_map_item(&[("score", rmpv::Value::Integer(1.into()))]));

        let mut outbox = VecDequeOutbox::new(1, 16);
        proc.process(0, &mut inbox, &mut outbox).unwrap();

        let mut emit_outbox = VecDequeOutbox::new(1, 16);
        proc.complete(&mut emit_outbox).unwrap();

        let items: Vec<_> = emit_outbox.drain_bucket(0).collect();
        assert_eq!(items.len(), 4);

        // Non-nil items first in ascending order
        assert_eq!(get_f64_field(&items[0], "score").unwrap() as i64, 1);
        assert_eq!(get_f64_field(&items[1], "score").unwrap() as i64, 5);

        // Nil/missing items last
        let score_2 = get_field(&items[2], "score");
        let score_3 = get_field(&items[3], "score");
        let is_nil_or_missing = |v: Option<&rmpv::Value>| {
            v.is_none() || matches!(v, Some(rmpv::Value::Nil))
        };
        assert!(is_nil_or_missing(score_2), "third item should have nil/missing score");
        assert!(is_nil_or_missing(score_3), "fourth item should have nil/missing score");
    }

    #[test]
    fn sort_nil_fields_sort_last_even_with_desc() {
        let mut proc = SortProcessor::new(vec![
            ("score".to_string(), SortDirection::Desc),
        ]);
        let ctx = make_context();
        proc.init(&ctx).unwrap();

        let mut inbox = VecDequeInbox::new(16);
        inbox.push(make_map_item(&[("score", rmpv::Value::Nil)]));
        inbox.push(make_map_item(&[("score", rmpv::Value::Integer(5.into()))]));
        inbox.push(make_map_item(&[("score", rmpv::Value::Integer(1.into()))]));

        let mut outbox = VecDequeOutbox::new(1, 16);
        proc.process(0, &mut inbox, &mut outbox).unwrap();

        let mut emit_outbox = VecDequeOutbox::new(1, 16);
        proc.complete(&mut emit_outbox).unwrap();

        let items: Vec<_> = emit_outbox.drain_bucket(0).collect();
        assert_eq!(items.len(), 3);

        // Desc: 5, 1, then nil last
        assert_eq!(get_f64_field(&items[0], "score").unwrap() as i64, 5);
        assert_eq!(get_f64_field(&items[1], "score").unwrap() as i64, 1);
        assert_eq!(get_field(&items[2], "score"), Some(&rmpv::Value::Nil));
    }

    // --- LimitProcessor ---

    #[test]
    fn limit_returns_at_most_n_items() {
        let mut proc = LimitProcessor::new(3);
        let ctx = make_context();
        proc.init(&ctx).unwrap();

        let mut inbox = VecDequeInbox::new(16);
        for i in 0..10i64 {
            inbox.push(rmpv::Value::Integer(i.into()));
        }

        let mut outbox = VecDequeOutbox::new(1, 16);
        let done = proc.process(0, &mut inbox, &mut outbox).unwrap();
        assert!(done, "limit should signal completion after emitting limit items");

        let items: Vec<_> = outbox.drain_bucket(0).collect();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0], rmpv::Value::Integer(0.into()));
        assert_eq!(items[1], rmpv::Value::Integer(1.into()));
        assert_eq!(items[2], rmpv::Value::Integer(2.into()));
    }

    #[test]
    fn limit_zero_returns_no_items() {
        let mut proc = LimitProcessor::new(0);
        let ctx = make_context();
        proc.init(&ctx).unwrap();

        let mut inbox = VecDequeInbox::new(8);
        inbox.push(rmpv::Value::Integer(1.into()));
        inbox.push(rmpv::Value::Integer(2.into()));

        let mut outbox = VecDequeOutbox::new(1, 8);
        let done = proc.process(0, &mut inbox, &mut outbox).unwrap();
        assert!(done, "limit 0 should immediately signal completion");

        let items: Vec<_> = outbox.drain_bucket(0).collect();
        assert!(items.is_empty(), "limit 0 should emit no items");
    }

    #[test]
    fn limit_with_fewer_items_than_limit() {
        let mut proc = LimitProcessor::new(10);
        let ctx = make_context();
        proc.init(&ctx).unwrap();

        let mut inbox = VecDequeInbox::new(8);
        inbox.push(rmpv::Value::Integer(1.into()));
        inbox.push(rmpv::Value::Integer(2.into()));

        let mut outbox = VecDequeOutbox::new(1, 8);
        let done = proc.process(0, &mut inbox, &mut outbox).unwrap();
        assert!(!done, "should not complete when fewer items than limit");

        let items: Vec<_> = outbox.drain_bucket(0).collect();
        assert_eq!(items.len(), 2, "all available items should pass through");
    }

    // --- Sort + Limit integration ---

    #[test]
    fn sort_then_limit_returns_top_n() {
        // Simulate sort -> limit pipeline: sort desc by age, limit 2
        let mut sort = SortProcessor::new(vec![
            ("age".to_string(), SortDirection::Desc),
        ]);
        let ctx = make_context();
        sort.init(&ctx).unwrap();

        let mut inbox = VecDequeInbox::new(16);
        inbox.push(make_map_item(&[("age", rmpv::Value::Integer(10.into()))]));
        inbox.push(make_map_item(&[("age", rmpv::Value::Integer(30.into()))]));
        inbox.push(make_map_item(&[("age", rmpv::Value::Integer(20.into()))]));
        inbox.push(make_map_item(&[("age", rmpv::Value::Integer(40.into()))]));
        inbox.push(make_map_item(&[("age", rmpv::Value::Integer(50.into()))]));

        let mut sort_outbox = VecDequeOutbox::new(1, 16);
        sort.process(0, &mut inbox, &mut sort_outbox).unwrap();

        let mut sort_emit_outbox = VecDequeOutbox::new(1, 16);
        sort.complete(&mut sort_emit_outbox).unwrap();

        // Feed sorted output into LimitProcessor
        let mut limit = LimitProcessor::new(2);
        limit.init(&ctx).unwrap();

        let mut limit_inbox = VecDequeInbox::new(16);
        for item in sort_emit_outbox.drain_bucket(0) {
            limit_inbox.push(item);
        }

        let mut limit_outbox = VecDequeOutbox::new(1, 16);
        limit.process(0, &mut limit_inbox, &mut limit_outbox).unwrap();

        let items: Vec<_> = limit_outbox.drain_bucket(0).collect();
        assert_eq!(items.len(), 2, "limit 2 should return exactly 2 items");
        assert_eq!(get_f64_field(&items[0], "age").unwrap() as i64, 50);
        assert_eq!(get_f64_field(&items[1], "age").unwrap() as i64, 40);
    }

    // --- SortProcessorSupplier ---

    #[test]
    fn sort_supplier_creates_correct_count() {
        let supplier = SortProcessorSupplier {
            sort_fields: vec![("age".to_string(), SortDirection::Asc)],
        };
        let processors = supplier.get(3);
        assert_eq!(processors.len(), 3);

        let cloned = supplier.clone_supplier();
        let cloned_procs = cloned.get(1);
        assert_eq!(cloned_procs.len(), 1);
    }

    // --- LimitProcessorSupplier ---

    #[test]
    fn limit_supplier_creates_correct_count() {
        let supplier = LimitProcessorSupplier { limit: 5 };
        let processors = supplier.get(2);
        assert_eq!(processors.len(), 2);

        let cloned = supplier.clone_supplier();
        let cloned_procs = cloned.get(1);
        assert_eq!(cloned_procs.len(), 1);
    }
}
