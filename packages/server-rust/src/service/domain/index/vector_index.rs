//! Thread-safe HNSW vector index with two-phase write semantics.
//!
//! Mutations are buffered in a pending queue without touching the HNSW graph.
//! A separate `commit_pending` call drains the queue under an exclusive write
//! lock, so concurrent readers are never blocked by write operations.
//!
//! This two-phase design keeps writes on a lock-free path: mutations always hit a
//! `DashMap` + `Mutex<Vec>` buffer, while the HNSW read lock is taken only for
//! queries — never for incoming writes.

use std::collections::HashSet;

use dashmap::{DashMap, DashSet};
use parking_lot::{Mutex, RwLock};
use topgun_core::vector::{Distance, DistanceMetric, SharedVector, distance_for_metric};
use tracing::warn;

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
    /// Pre-allocated distance implementation, built once in `new` to avoid
    /// reboxing on every `search_nearest` call.
    distance: Box<dyn Distance>,
    /// Attribute extractor built once in `new`, reused in the decode helper
    /// for consistency with HashIndex/NavigableIndex/InvertedIndex.
    extractor: AttributeExtractor,
    /// HNSW graph protected by `RwLock` — concurrent reads, exclusive writes
    /// only during `commit_pending`.
    hnsw: RwLock<Hnsw>,
    /// Pending mutations buffered under a cheap mutex, drained into the graph
    /// during `commit_pending`. Writers never take the HNSW write lock.
    pending: Mutex<Vec<VectorPendingUpdate>>,
    /// Monotonic allocator for HNSW `ElementId` values.
    next_id: Mutex<u64>,
    /// Bidirectional key <-> `ElementId` mapping for concurrent reads.
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
    /// otherwise the HNSW write lock will block the current tokio worker for
    /// the duration of the batch apply.
    ///
    /// Returns the number of mutations that were drained.
    pub fn commit_pending(&self) -> u64 {
        // Drain the pending queue under the mutex, then release it before
        // taking the HNSW write lock — avoids holding two locks simultaneously.
        let mutations = {
            let mut guard = self.pending.lock();
            std::mem::take(&mut *guard)
        };

        let count = mutations.len() as u64;
        if count == 0 {
            return 0;
        }

        let mut hnsw = self.hnsw.write();
        for mutation in mutations {
            match mutation {
                VectorPendingUpdate::Upsert { key, vector } => {
                    // HNSW has no native update; replace via remove-then-insert.
                    if let Some((_, old_id)) = self.key_to_id.remove(&key) {
                        self.id_to_key.remove(&old_id);
                        hnsw.remove(old_id);
                    }
                    let new_id = {
                        let mut id = self.next_id.lock();
                        let allocated = *id;
                        *id += 1;
                        allocated
                    };
                    self.key_to_id.insert(key.clone(), new_id);
                    self.id_to_key.insert(new_id, key);
                    hnsw.insert(new_id, vector);
                }
                VectorPendingUpdate::Remove { key } => {
                    if let Some((_, id)) = self.key_to_id.remove(&key) {
                        self.id_to_key.remove(&id);
                        hnsw.remove(id);
                    }
                }
            }
        }
        drop(hnsw);

        // Clear the read-merge snapshot — committed state is now in the graph.
        self.pending_snapshot.clear();
        self.pending_removed.clear();

        count
    }

    /// Returns the number of queued mutations not yet applied to the graph.
    pub fn pending_count(&self) -> u64 {
        self.pending.lock().len() as u64
    }

    /// Runs ANN search, merging committed HNSW results with pending upserts
    /// and applying `pending_removed` suppression.
    ///
    /// Returns the top-`k` nearest neighbors as `(record_key, distance)`
    /// pairs sorted ascending by distance.
    pub fn search_nearest(&self, query: &[f32], k: usize, ef: usize) -> Vec<(String, f64)> {
        // Acquire HNSW read lock, run search, release before expensive merge.
        let committed: Vec<(String, f64)> = {
            let hnsw = self.hnsw.read();
            hnsw.search(query, k, ef)
                .into_iter()
                .filter_map(|(id, dist)| {
                    self.id_to_key.get(&id).map(|k| (k.clone(), dist))
                })
                .collect()
        };

        // Compute distances for pending upserts using the pre-built distance fn.
        let pending_results: Vec<(String, f64)> = self
            .pending_snapshot
            .iter()
            .map(|entry| {
                let vf = entry.value().vector().to_f32_vec();
                let dist = self.distance.compute(&vf, query);
                (entry.key().clone(), dist)
            })
            .collect();

        // Merge: pending wins over committed for the same key.
        let mut merged: std::collections::HashMap<String, f64> =
            std::collections::HashMap::new();
        for (key, dist) in committed {
            merged.insert(key, dist);
        }
        for (key, dist) in pending_results {
            // Pending is newer — always overrides committed value.
            merged.insert(key, dist);
        }

        // Suppress removed keys.
        let mut results: Vec<(String, f64)> = merged
            .into_iter()
            .filter(|(key, _)| !self.pending_removed.contains(key.as_str()))
            .collect();

        results.sort_by(|a, b| a.1.total_cmp(&b.1));
        results.truncate(k);
        results
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
        let field = self.extractor.extract(record);
        let bytes = match &field {
            rmpv::Value::Binary(b) => b.as_slice(),
            _ => return None,
        };

        let decoded: topgun_core::vector::Vector =
            rmp_serde::from_slice(bytes)
                .map_err(|e| {
                    warn!(
                        attribute = %self.attribute,
                        error = %e,
                        "failed to decode vector from record binary field"
                    );
                    e
                })
                .ok()?;

        let expected = self.dimension as usize;
        let got = decoded.dimension();
        if got != expected {
            warn!(
                attribute = %self.attribute,
                expected,
                got,
                "vector dimension mismatch: expected {expected}, got {got}"
            );
            return None;
        }

        Some(SharedVector::new(decoded))
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
        let Some(vector) = self.decode_vector_from_record(value) else {
            return;
        };
        self.pending_snapshot.insert(key.to_string(), vector.clone());
        self.pending_removed.remove(key);
        self.pending
            .lock()
            .push(VectorPendingUpdate::Upsert { key: key.to_string(), vector });
    }

    fn update(&self, key: &str, _old_value: &rmpv::Value, new_value: &rmpv::Value) {
        // HNSW has no native update; delegate to insert (commit path does
        // remove-then-insert for keys already in the graph).
        self.insert(key, new_value);
    }

    fn remove(&self, key: &str, _old_value: &rmpv::Value) {
        self.pending_snapshot.remove(key);
        self.pending_removed.insert(key.to_string());
        self.pending
            .lock()
            .push(VectorPendingUpdate::Remove { key: key.to_string() });
    }

    fn clear(&self) {
        // Rebuild a fresh HNSW graph under the write lock.
        let params = HnswParams {
            dimension: self.dimension,
            distance: self.distance_metric,
            ..Default::default()
        };
        *self.hnsw.write() = Hnsw::new(params);

        // Reset all auxiliary state.
        self.pending.lock().clear();
        *self.next_id.lock() = 0;
        self.key_to_id.clear();
        self.id_to_key.clear();
        self.pending_snapshot.clear();
        self.pending_removed.clear();
    }

    fn lookup_eq(&self, _value: &rmpv::Value) -> HashSet<String> {
        // Vector search is not predicate-driven; callers use search_nearest.
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
        // Committed entries only; pending count is accessible via pending_count().
        self.key_to_id.len() as u64
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use topgun_core::vector::{DistanceMetric, Vector};

    use super::*;

    fn make_index() -> VectorIndex {
        VectorIndex::new("embedding", 4, DistanceMetric::Cosine)
    }

    fn make_record(data: &[f32]) -> rmpv::Value {
        let v = Vector::F32(data.to_vec());
        let encoded = rmp_serde::to_vec_named(&v).unwrap();
        let field_value = rmpv::Value::Binary(encoded);
        rmpv::Value::Map(vec![(
            rmpv::Value::String(rmpv::Utf8String::from("embedding")),
            field_value,
        )])
    }

    #[test]
    fn new_vector_index_is_empty() {
        let idx = make_index();
        assert_eq!(idx.entry_count(), 0);
        assert_eq!(idx.pending_count(), 0);
    }

    #[test]
    fn insert_queues_pending_mutation() {
        let idx = make_index();
        let record = make_record(&[0.1, 0.2, 0.3, 0.4]);
        idx.insert("k1", &record);
        assert_eq!(idx.pending_count(), 1);
        assert_eq!(idx.entry_count(), 0); // not committed yet
    }

    #[test]
    fn insert_then_commit_flushes_to_graph() {
        let idx = make_index();
        let record = make_record(&[0.1, 0.2, 0.3, 0.4]);
        idx.insert("k1", &record);
        let flushed = idx.commit_pending();
        assert_eq!(flushed, 1);
        assert_eq!(idx.entry_count(), 1);
        assert_eq!(idx.pending_count(), 0);
    }

    #[test]
    fn search_merges_pending_and_committed_results() {
        let idx = make_index();
        // Insert and commit first entry
        let r1 = make_record(&[1.0, 0.0, 0.0, 0.0]);
        idx.insert("k1", &r1);
        idx.commit_pending();

        // Insert a second entry (pending only)
        let r2 = make_record(&[0.9, 0.1, 0.0, 0.0]);
        idx.insert("k2", &r2);

        // Search should return both committed and pending
        let results = idx.search_nearest(&[1.0, 0.0, 0.0, 0.0], 10, 10);
        let keys: Vec<&str> = results.iter().map(|(k, _)| k.as_str()).collect();
        assert!(keys.contains(&"k1"), "committed result missing");
        assert!(keys.contains(&"k2"), "pending result missing");
    }

    #[test]
    fn remove_suppresses_pending_snapshot_entry() {
        let idx = make_index();
        let record = make_record(&[0.1, 0.2, 0.3, 0.4]);
        idx.insert("k1", &record);
        assert_eq!(idx.pending_count(), 1);
        idx.remove("k1", &rmpv::Value::Nil);
        // The upsert is still in pending queue, but pending_removed suppresses it
        // in search results
        assert!(idx.pending_removed.contains("k1"));
        assert!(!idx.pending_snapshot.contains_key("k1"));
    }

    #[test]
    fn clear_resets_graph_and_mapping() {
        let idx = make_index();
        let record = make_record(&[0.1, 0.2, 0.3, 0.4]);
        idx.insert("k1", &record);
        idx.commit_pending();
        assert_eq!(idx.entry_count(), 1);
        idx.clear();
        assert_eq!(idx.entry_count(), 0);
        assert_eq!(idx.pending_count(), 0);
        assert!(idx.pending_snapshot.is_empty());
        assert!(idx.key_to_id.is_empty());
    }

    #[test]
    fn search_returns_empty_when_graph_empty() {
        let idx = make_index();
        let results = idx.search_nearest(&[1.0, 0.0, 0.0, 0.0], 5, 5);
        assert!(results.is_empty());
    }

    #[test]
    fn concurrent_reads_allowed_during_pending_writes() {
        let idx = Arc::new(make_index());
        // Pre-populate some committed data
        let r1 = make_record(&[1.0, 0.0, 0.0, 0.0]);
        idx.insert("k1", &r1);
        idx.commit_pending();

        let idx_clone = Arc::clone(&idx);
        let reader = std::thread::spawn(move || {
            // Run many searches while writers are active
            for _ in 0..20 {
                let results = idx_clone.search_nearest(&[1.0, 0.0, 0.0, 0.0], 5, 5);
                assert!(!results.is_empty(), "reader got empty results");
            }
        });

        // Writer thread: insert more pending items
        for i in 0..10u32 {
            // Small integers cast exactly to f32.
            #[allow(clippy::cast_precision_loss)]
            let data = vec![i as f32 / 10.0, 0.1, 0.0, 0.0];
            let record = make_record(&data);
            idx.insert(&format!("w{i}"), &record);
        }

        reader.join().expect("reader thread panicked");
    }

    #[test]
    fn update_behaves_as_upsert() {
        let idx = make_index();
        let r1 = make_record(&[1.0, 0.0, 0.0, 0.0]);
        idx.insert("k1", &r1);
        idx.commit_pending();

        // Update with a different vector
        let r2 = make_record(&[0.0, 1.0, 0.0, 0.0]);
        idx.update("k1", &r1, &r2);
        idx.commit_pending();

        // After commit, entry_count should still be 1 (replace semantics)
        assert_eq!(idx.entry_count(), 1);
    }

    #[test]
    fn index_trait_lookup_methods_return_empty() {
        let idx = make_index();
        let record = make_record(&[0.1, 0.2, 0.3, 0.4]);
        idx.insert("k1", &record);
        idx.commit_pending();

        assert!(idx.lookup_eq(&rmpv::Value::Nil).is_empty());
        assert!(idx.lookup_range(None, true, None, true).is_empty());
        assert!(idx.lookup_contains("token").is_empty());
    }
}
