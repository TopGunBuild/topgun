//! Thread-safe HNSW vector index with two-phase write semantics.
//!
//! Mutations are buffered in a pending queue without touching the HNSW graph.
//! A separate `commit_pending` call drains the queue under an exclusive write
//! lock, so concurrent readers are never blocked by write operations.

use std::collections::HashSet;

use dashmap::{DashMap, DashSet};
use parking_lot::{Mutex, RwLock};
use topgun_core::vector::{Distance, DistanceMetric, SharedVector, distance_for_metric};

use crate::service::domain::index::attribute::AttributeExtractor;
use crate::service::domain::index::hnsw::{ElementId, Hnsw, HnswParams};
use crate::service::domain::index::{Index, IndexType};

/// A queued mutation waiting to be applied to the HNSW graph.
#[derive(Debug, Clone)]
pub enum VectorPendingUpdate {
    Upsert { key: String, vector: SharedVector },
    Remove { key: String },
}

/// Thread-safe HNSW approximate nearest-neighbor index.
///
/// Implements the `Index` trait so it participates in the standard mutation
/// observer pipeline alongside `HashIndex`, `NavigableIndex`, and
/// `InvertedIndex`. Vector-specific operations (`search_nearest`,
/// `commit_pending`) are accessible via a concrete `Arc<VectorIndex>` obtained
/// through `IndexRegistry::get_vector_index`.
pub struct VectorIndex {
    attribute: String,
    dimension: u16,
    distance_metric: DistanceMetric,
    /// Pre-allocated distance implementation, built once in `new`.
    distance: Box<dyn Distance>,
    /// Attribute extractor built once in `new`, reused in the decode helper
    /// for consistency with HashIndex/NavigableIndex/InvertedIndex.
    extractor: AttributeExtractor,
    /// HNSW graph protected by RwLock — concurrent reads, exclusive writes
    /// only during commit_pending.
    hnsw: RwLock<Hnsw>,
    /// Pending mutations buffered under a cheap mutex, drained into the graph
    /// during commit_pending. Writers never take the HNSW write lock.
    pending: Mutex<Vec<VectorPendingUpdate>>,
    /// Monotonic allocator for HNSW ElementId values.
    next_id: Mutex<u64>,
    /// Bidirectional key <-> ElementId mapping for concurrent reads.
    key_to_id: DashMap<String, ElementId>,
    id_to_key: DashMap<ElementId, String>,
    /// Snapshot of pending upserts for merging into read results before commit.
    pending_snapshot: DashMap<String, SharedVector>,
    /// Keys queued for removal — suppressed in read results until commit.
    pending_removed: DashSet<String>,
}

impl VectorIndex {
    /// Creates a new, empty `VectorIndex` for the given attribute, dimension,
    /// and distance metric.
    pub fn new(
        attribute: impl Into<String>,
        dimension: u16,
        distance_metric: DistanceMetric,
    ) -> Self {
        let attr = attribute.into();
        let params = HnswParams {
            dimension,
            distance: distance_metric,
            ..Default::default()
        };
        let distance = distance_for_metric(distance_metric);
        let extractor = AttributeExtractor::new(attr.clone());
        let hnsw = Hnsw::new(params);
        Self {
            attribute: attr,
            dimension,
            distance_metric,
            distance,
            extractor,
            hnsw: RwLock::new(hnsw),
            pending: Mutex::new(Vec::new()),
            next_id: Mutex::new(0),
            key_to_id: DashMap::new(),
            id_to_key: DashMap::new(),
            pending_snapshot: DashMap::new(),
            pending_removed: DashSet::new(),
        }
    }

    // ------------------------------------------------------------------
    // Vector-specific methods (not on the Index trait)
    // ------------------------------------------------------------------

    /// Commits all queued mutations into the HNSW graph.
    ///
    /// Takes the HNSW write lock for the duration of the flush. Callers
    /// running on a tokio runtime MUST wrap this call in
    /// `tokio::task::spawn_blocking` when the pending queue may be large;
    /// otherwise the HNSW write lock will block the current tokio worker.
    ///
    /// Returns the number of mutations that were drained.
    pub fn commit_pending(&self) -> u64 {
        unimplemented!("G3 will implement commit_pending")
    }

    /// Returns the number of queued mutations not yet applied to the graph.
    pub fn pending_count(&self) -> u64 {
        unimplemented!("G3 will implement pending_count")
    }

    /// Runs ANN search, merging committed HNSW results with pending upserts
    /// and applying pending_removed suppression.
    ///
    /// Returns the top-`k` nearest neighbors as `(record_key, distance)`
    /// pairs sorted ascending by distance.
    pub fn search_nearest(&self, query: &[f32], k: usize, ef: usize) -> Vec<(String, f64)> {
        unimplemented!("G3 will implement search_nearest")
    }

    /// Returns the configured vector dimension.
    pub fn dimension(&self) -> u16 {
        self.dimension
    }

    /// Returns the configured distance metric.
    pub fn distance_metric(&self) -> DistanceMetric {
        self.distance_metric
    }

    // ------------------------------------------------------------------
    // Private helpers
    // ------------------------------------------------------------------

    /// Extracts and decodes a `SharedVector` from the full record `rmpv::Value`.
    ///
    /// Uses `AttributeExtractor` to navigate to `self.attribute` inside the
    /// record map, matches `rmpv::Value::Binary(bytes)`, and decodes via
    /// `rmp_serde::from_slice::<topgun_core::vector::Vector>`. Returns `None`
    /// on any extraction or decoding failure, including dimension mismatches.
    fn decode_vector_from_record(&self, record: &rmpv::Value) -> Option<SharedVector> {
        unimplemented!("G3 will implement decode_vector_from_record")
    }
}

// ------------------------------------------------------------------
// Index trait implementation
// ------------------------------------------------------------------

impl Index for VectorIndex {
    fn index_type(&self) -> IndexType {
        IndexType::Vector
    }

    fn attribute_name(&self) -> &str {
        &self.attribute
    }

    fn insert(&self, key: &str, value: &rmpv::Value) {
        unimplemented!("G3 will implement insert")
    }

    fn update(&self, key: &str, _old_value: &rmpv::Value, new_value: &rmpv::Value) {
        unimplemented!("G3 will implement update")
    }

    fn remove(&self, key: &str, _old_value: &rmpv::Value) {
        unimplemented!("G3 will implement remove")
    }

    fn clear(&self) {
        unimplemented!("G3 will implement clear")
    }

    fn lookup_eq(&self, _value: &rmpv::Value) -> HashSet<String> {
        HashSet::new()
    }

    fn lookup_range(
        &self,
        _lower: Option<&rmpv::Value>,
        _lower_inclusive: bool,
        _upper: Option<&rmpv::Value>,
        _upper_inclusive: bool,
    ) -> HashSet<String> {
        HashSet::new()
    }

    fn lookup_contains(&self, _token: &str) -> HashSet<String> {
        HashSet::new()
    }

    fn entry_count(&self) -> u64 {
        self.key_to_id.len() as u64
    }
}

