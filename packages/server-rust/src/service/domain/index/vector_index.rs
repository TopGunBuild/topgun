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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::SystemTime;

use dashmap::{DashMap, DashSet};
use parking_lot::{Mutex, RwLock};
use topgun_core::vector::{distance_for_metric, Distance, DistanceMetric, SharedVector};
use tracing::{info, warn};

use crate::network::handlers::admin_types::{BackfillProgress, RebuildType, VectorIndexDescriptor};
use crate::service::domain::index::attribute::AttributeExtractor;
use crate::service::domain::index::hnsw::{ElementId, Hnsw, HnswParams};
use crate::service::domain::index::registry::VectorIndexStats;
use crate::service::domain::index::{Index, IndexType};
use crate::storage::factory::RecordStoreFactory;
use crate::storage::record::RecordValue;

/// A queued mutation waiting to be applied to the HNSW graph.
#[derive(Debug, Clone)]
pub enum VectorPendingUpdate {
    Upsert { key: String, vector: SharedVector },
    Remove { key: String },
}

impl VectorPendingUpdate {
    fn key(&self) -> &str {
        match self {
            Self::Upsert { key, .. } | Self::Remove { key } => key,
        }
    }
}

/// Tracks progress of an in-progress HNSW optimize (graph rebuild) operation.
///
/// Returned by `VectorIndex::optimize`. Both the background task and the HTTP
/// status handler share the same `Arc<OptimizeHandle>` — counters are updated
/// atomically so reads never block.
pub struct OptimizeHandle {
    /// Unique identifier for this optimize run (UUID v4).
    pub id: String,
    /// ISO-8601 UTC timestamp when the optimize was started.
    pub started_at: String,
    /// Number of vectors processed so far during rebuild.
    pub processed: AtomicU64,
    /// Total vectors to process (set once before the rebuild loop starts).
    pub total: AtomicU64,
    /// Set to `true` atomically when the rebuild completes and the pointer swap occurs.
    pub finished: AtomicBool,
    /// Set to `true` by the HTTP cancel handler to request cooperative cancellation.
    /// The rebuild loop checks this flag before each `fresh.insert` call and aborts
    /// early when `true`, without swapping the HNSW graph.
    pub cancelled: AtomicBool,
}

/// Thread-safe HNSW approximate nearest-neighbor index.
///
/// Implements the `Index` trait so it participates in the standard mutation
/// observer pipeline alongside `HashIndex`, `NavigableIndex`, and
/// `InvertedIndex`. Vector-specific operations (`search_nearest`,
/// `commit_pending`) are accessible via a concrete `Arc<VectorIndex>` obtained
/// through `IndexRegistry::get_vector_index`.
///
/// One entry per key (`TG-VEC-001`): a key "has an entry" when it is in
/// `pending_snapshot`, or in `key_to_id` and not in `pending_removed`. Keys
/// that share a vector are separate entries and separate graph nodes.
///
/// Lock order: the `pending` mutex and the `hnsw` lock are never held at the
/// same time. `insert`, `insert_if_absent` and `remove_key` hold `pending`
/// alone; `commit_pending` takes `pending` to drain, releases it, takes `hnsw`
/// (write) to apply, releases it, and takes `pending` again to settle. The
/// shard locks of `pending_snapshot`, `pending_removed`, `key_to_id` and
/// `id_to_key`, and the `next_id` mutex, are leaves: they may be taken while
/// `pending` or `hnsw` is held, and nothing else is taken while one is held.
/// `search_nearest` takes `hnsw` (read) and leaves only, never `pending`.
pub struct VectorIndex {
    attribute: String,
    /// User-visible name for this index (e.g. `embedding_index`).
    index_name: String,
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
    /// ISO-8601 UTC timestamp of the last completed optimize, or `None`.
    last_optimized: RwLock<Option<String>>,
    /// In-flight optimize handle, if an optimize is currently running.
    /// `None` when idle; `Some(Arc<OptimizeHandle>)` while a rebuild is active.
    optimize_handle: Mutex<Option<Arc<OptimizeHandle>>>,
}

impl VectorIndex {
    /// Creates a new, empty `VectorIndex` for the given attribute, dimension,
    /// and distance metric.
    ///
    /// `index_name` is the user-visible name for this index (used in admin API responses).
    /// `_dedup_enabled` is accepted and ignored: every key is its own entry,
    /// also when another key holds the same vector (`TG-VEC-001`).
    pub fn new(
        attribute: impl Into<String>,
        index_name: impl Into<String>,
        dimension: u16,
        distance_metric: DistanceMetric,
        _dedup_enabled: bool,
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
            index_name: index_name.into(),
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
            last_optimized: RwLock::new(None),
            optimize_handle: Mutex::new(None),
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
    /// the duration of the batch apply. Callers must not run two commits, or a
    /// commit and `clear`, at the same time.
    ///
    /// Returns the number of mutations that were drained.
    pub fn commit_pending(&self) -> u64 {
        let drained = self.take_pending();
        if drained.is_empty() {
            return 0;
        }
        self.apply_to_graph(&drained);
        self.settle_drained(&drained);
        drained.len() as u64
    }

    /// Drains the pending queue under the `pending` mutex.
    fn take_pending(&self) -> Vec<VectorPendingUpdate> {
        std::mem::take(&mut *self.pending.lock())
    }

    /// Applies drained mutations to the graph under the HNSW write lock. Every
    /// drained key keeps its snapshot entry or removal mark meanwhile, so a
    /// search never misses an upsert or returns a removed key in between.
    fn apply_to_graph(&self, drained: &[VectorPendingUpdate]) {
        let mut hnsw = self.hnsw.write();
        for mutation in drained {
            // HNSW has no native update: an upsert replaces via remove-then-insert.
            if let Some((_, old_id)) = self.key_to_id.remove(mutation.key()) {
                self.id_to_key.remove(&old_id);
                hnsw.remove(old_id);
            }
            if let VectorPendingUpdate::Upsert { key, vector } = mutation {
                let new_id = {
                    let mut id = self.next_id.lock();
                    let allocated = *id;
                    *id += 1;
                    allocated
                };
                self.key_to_id.insert(key.clone(), new_id);
                self.id_to_key.insert(new_id, key.clone());
                hnsw.insert(new_id, vector.clone());
            }
        }
    }

    /// Clears the snapshot entry and removal mark of each drained key that has
    /// no newer mutation queued, under the `pending` mutex. A key queued again
    /// after the drain keeps the entry or mark its newer mutation set, until
    /// that mutation is itself committed.
    fn settle_drained(&self, drained: &[VectorPendingUpdate]) {
        let queue = self.pending.lock();
        let queued: HashSet<&str> = queue.iter().map(VectorPendingUpdate::key).collect();
        for mutation in drained {
            let key = mutation.key();
            if !queued.contains(key) {
                self.pending_snapshot.remove(key);
                self.pending_removed.remove(key);
            }
        }
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
                .filter_map(|(id, dist)| self.id_to_key.get(&id).map(|k| (k.clone(), dist)))
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
        let mut merged: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
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

    /// Returns the user-visible index name.
    pub fn index_name(&self) -> &str {
        &self.index_name
    }

    /// Returns a snapshot of statistics for this vector index.
    ///
    /// All counts are atomic loads or cheap reads; does not block ongoing searches.
    /// `graph_layers` is read under the HNSW read lock (brief acquisition).
    pub fn stats(&self) -> VectorIndexStats {
        let vector_count = self.key_to_id.len() as u64;
        let pending_updates = self.pending.lock().len() as u64;
        let memory_bytes = self.estimate_memory_bytes();
        let graph_layers = self.hnsw.read().layer_count();
        let last_optimized = self.last_optimized.read().clone();
        VectorIndexStats {
            attribute: self.attribute.clone(),
            index_name: self.index_name.clone(),
            dimension: self.dimension,
            distance_metric: self.distance_metric,
            vector_count,
            memory_bytes,
            graph_layers,
            pending_updates,
            last_optimized,
        }
    }

    /// Triggers an HNSW graph rebuild in a background task.
    ///
    /// Returns `(handle, was_already_running)`:
    /// - `handle` — shared `Arc<OptimizeHandle>` for this optimize run.
    /// - `was_already_running` — `true` if an in-flight optimize was reused;
    ///   `false` when a new optimize was started.
    ///
    /// The rebuild uses a two-phase pattern:
    /// 1. Snapshot committed vectors from the HNSW graph while holding the read lock briefly.
    /// 2. Build a fresh HNSW graph in a background blocking task (no lock held).
    /// 3. Swap the new graph under the write lock (brief blocking window).
    ///
    /// Searches continue against the old graph during rebuild.
    /// Only one optimize may run at a time per index; subsequent calls return the
    /// existing handle with `was_already_running = true`.
    pub fn optimize(self: &Arc<Self>) -> (Arc<OptimizeHandle>, bool) {
        let mut guard = self.optimize_handle.lock();

        // If an optimize is already running (not yet finished), return its handle.
        if let Some(ref existing) = *guard {
            if !existing.finished.load(Ordering::Relaxed) {
                return (Arc::clone(existing), true);
            }
        }

        // Start a new optimize.
        let id = uuid::Uuid::new_v4().to_string();
        let started_at = {
            let secs = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            format_iso8601(secs)
        };
        let handle = Arc::new(OptimizeHandle {
            id: id.clone(),
            started_at,
            processed: AtomicU64::new(0),
            total: AtomicU64::new(0),
            finished: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
        });
        *guard = Some(Arc::clone(&handle));
        drop(guard);

        // Snapshot all committed (non-deleted) vectors from the HNSW graph.
        // We hold the read lock only for the duration of the snapshot — searches
        // can proceed concurrently against the current graph during the rebuild.
        let vectors_snapshot: Vec<(u64, SharedVector)> = self.hnsw.read().all_vectors();

        handle
            .total
            .store(vectors_snapshot.len() as u64, Ordering::Relaxed);

        let this = Arc::clone(self);
        let handle_clone = Arc::clone(&handle);
        tokio::spawn(async move {
            // Build the new HNSW graph in a blocking task to avoid blocking the tokio worker.
            // Returns Some(graph) on completion, None if cancelled cooperatively.
            let new_hnsw = tokio::task::spawn_blocking({
                let vectors = vectors_snapshot;
                let dimension = this.dimension;
                let distance_metric = this.distance_metric;
                let handle_inner = Arc::clone(&handle_clone);
                move || -> Option<Hnsw> {
                    let params = HnswParams {
                        dimension,
                        distance: distance_metric,
                        ..Default::default()
                    };
                    let mut fresh = Hnsw::new(params);
                    for (elem_id, vector) in vectors {
                        // Cooperative cancellation: check flag before each insert so
                        // the operator's DELETE request is observed within one insert's
                        // latency (~microseconds for 384-dim vectors).
                        if handle_inner.cancelled.load(Ordering::Relaxed) {
                            return None;
                        }
                        fresh.insert(elem_id, vector);
                        handle_inner.processed.fetch_add(1, Ordering::Relaxed);
                    }
                    Some(fresh)
                }
            })
            .await;

            match new_hnsw {
                Ok(Some(fresh_graph)) => {
                    // Successful rebuild: swap under write lock — brief blocking window.
                    *this.hnsw.write() = fresh_graph;
                    // Record optimize completion time.
                    let now_secs = SystemTime::now()
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    *this.last_optimized.write() = Some(format_iso8601(now_secs));
                    handle_clone.finished.store(true, Ordering::Relaxed);
                    info!(
                        attribute = %this.attribute,
                        index_name = %this.index_name,
                        processed = handle_clone.processed.load(Ordering::Relaxed),
                        "vector index optimize complete"
                    );
                }
                Ok(None) => {
                    // Cancelled: do NOT swap the HNSW graph, do NOT update last_optimized.
                    // The partial rebuild is dropped here; the live graph is unchanged.
                    handle_clone.finished.store(true, Ordering::Relaxed);
                    info!(
                        attribute = %this.attribute,
                        index_name = %this.index_name,
                        "vector index optimize cancelled"
                    );
                }
                Err(e) => {
                    warn!(
                        attribute = %this.attribute,
                        index_name = %this.index_name,
                        error = %e,
                        "vector index optimize task panicked"
                    );
                    handle_clone.finished.store(true, Ordering::Relaxed);
                }
            }
        });

        (handle, false)
    }

    /// Returns the current in-flight `OptimizeHandle`, or `None` if no optimize is running.
    ///
    /// Used by the admin status handler to report progress without triggering a new rebuild.
    pub fn current_optimize_handle(&self) -> Option<Arc<OptimizeHandle>> {
        self.optimize_handle.lock().clone()
    }

    // ------------------------------------------------------------------
    // Private helpers
    // ------------------------------------------------------------------

    /// Whether `key` has an entry. Called with the `pending` mutex held.
    fn has_entry(&self, key: &str) -> bool {
        self.pending_snapshot.contains_key(key)
            || (self.key_to_id.contains_key(key) && !self.pending_removed.contains(key))
    }

    /// Queues an upsert of `key`. Called with the `pending` mutex held.
    fn queue_upsert(&self, queue: &mut Vec<VectorPendingUpdate>, key: &str, vector: SharedVector) {
        self.pending_snapshot
            .insert(key.to_string(), vector.clone());
        self.pending_removed.remove(key);
        queue.push(VectorPendingUpdate::Upsert {
            key: key.to_string(),
            vector,
        });
    }

    /// Queues a removal of `key`. Called with the `pending` mutex held.
    fn queue_remove(&self, queue: &mut Vec<VectorPendingUpdate>, key: &str) {
        self.pending_snapshot.remove(key);
        self.pending_removed.insert(key.to_string());
        queue.push(VectorPendingUpdate::Remove {
            key: key.to_string(),
        });
    }

    /// Estimated memory usage of the HNSW graph in bytes.
    ///
    /// Formula: `vector_count * dimension * 4` (f32 components) +
    /// `vector_count * 16 * 8` (approximate 16 edges per node, 8 bytes each).
    ///
    /// This is a documented approximation, not exact allocator accounting.
    /// The formula is owned here so it stays accurate if edge-pointer width changes.
    fn estimate_memory_bytes(&self) -> u64 {
        let vc = self.key_to_id.len() as u64;
        let dim = u64::from(self.dimension);
        // 4 bytes per f32 component + average 16 neighbor pointers at 8 bytes each
        vc * dim * 4 + vc * 16 * 8
    }

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

        let decoded: topgun_core::vector::Vector = rmp_serde::from_slice(bytes)
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
// ISO-8601 timestamp helper (no external dep)
// ------------------------------------------------------------------

/// Formats Unix seconds as a minimal ISO-8601 UTC string: `YYYY-MM-DDTHH:MM:SSZ`.
#[allow(clippy::cast_possible_truncation)]
#[allow(clippy::cast_precision_loss, clippy::cast_sign_loss)]
pub(crate) fn format_iso8601(secs: u64) -> String {
    let sec = secs % 60;
    let min = (secs / 60) % 60;
    let hour = (secs / 3600) % 24;
    let days = secs / 86400;

    // Days since 1970-01-01 to Gregorian date (Meeus algorithm, simplified).
    let julian_z = days + 2_440_588; // Julian Day Number offset
    let julian_a_frac = (julian_z as f64 - 1_867_216.25) / 36_524.25;
    let julian_a = julian_z + 1 + julian_a_frac as u64 - (julian_a_frac as u64 / 4);
    let julian_b = julian_a + 1524;
    let julian_c = ((julian_b as f64 - 122.1) / 365.25) as u64;
    let days_in_year = (365.25 * julian_c as f64) as u64;
    let julian_e = ((julian_b - days_in_year) as f64 / 30.6001) as u64;

    let day = (julian_b - days_in_year - (30.6001 * julian_e as f64) as u64) as u32;
    let month = if julian_e < 14 {
        julian_e - 1
    } else {
        julian_e - 13
    } as u32;
    let year = if month > 2 {
        julian_c - 4716
    } else {
        julian_c - 4715
    } as u32;

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{min:02}:{sec:02}Z")
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
        // Decoded before the lock: only the entry check and the queue changes
        // need the `pending` hold.
        let vector = self.decode_vector_from_record(value);
        let mut queue = self.pending.lock();
        match vector {
            Some(vector) => self.queue_upsert(&mut queue, key, vector),
            // A value without a usable vector must not leave the key indexed
            // under its previous vector; a key without an entry stays without
            // one, so a load of such a record queues nothing.
            None if self.has_entry(key) => self.queue_remove(&mut queue, key),
            None => {}
        }
    }

    fn insert_if_absent(&self, key: &str, value: &rmpv::Value) {
        let vector = self.decode_vector_from_record(value);
        let mut queue = self.pending.lock();
        if self.has_entry(key) {
            return;
        }
        if let Some(vector) = vector {
            self.queue_upsert(&mut queue, key, vector);
        }
    }

    fn update(&self, key: &str, _old_value: &rmpv::Value, new_value: &rmpv::Value) {
        self.insert(key, new_value);
    }

    fn remove(&self, key: &str, _old_value: &rmpv::Value) {
        self.remove_key(key);
    }

    fn remove_key(&self, key: &str) {
        let mut queue = self.pending.lock();
        self.queue_remove(&mut queue, key);
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

// ------------------------------------------------------------------
// Startup rebuild support
// ------------------------------------------------------------------

/// Re-registers the vector indexes of `descriptors` and fills every vector
/// index registered in `index_factory` from the stored records, before
/// `set_ready()` (`TG-VEC-001`).
///
/// 1. Each descriptor's index is registered with
///    `add_vector_index_with_params`, which replaces an index already
///    registered for that attribute by a fresh one.
/// 2. Every map whose registry holds a vector index is rebuilt, so indexes
///    registered before the call (the `_embedding` indexes of
///    `TOPGUN_VECTOR_MAPS`) are filled too.
/// 3. A `BackfillProgress { rebuild_type: StartupRebuild }` entry is written
///    per `(map, attribute)` before the scan.
/// 4. One [`RecordStoreFactory::scan_merged`] per map reads the stored and
///    resident live rows (expired ones are not emitted, `TG-TTL-001`) without
///    loading them into memory. Each `Lww` row is given to every vector index
///    of the map with `insert_if_absent`, so a write the observer applied
///    during the scan is not replaced by its older stored row.
/// 5. After a successful scan each index is committed and marked `done`. A
///    failed scan logs one `ERROR`, leaves `done == false` and the next map is
///    rebuilt; a failure never aborts the boot.
///
/// The `backfill_progress` `Arc` is the one in `AppState`, so the entries are
/// visible through the admin status endpoint after `set_ready()`.
pub async fn rebuild_from_store(
    index_factory: &crate::service::domain::index::mutation_observer::IndexObserverFactory,
    store_factory: &RecordStoreFactory,
    descriptors: &[VectorIndexDescriptor],
    backfill_progress: &Arc<DashMap<(String, String), Arc<BackfillProgress>>>,
) {
    for d in descriptors {
        index_factory
            .register_map(&d.map_name)
            .add_vector_index_with_params(
                d.attribute.clone(),
                d.index_name.clone(),
                d.dimension,
                d.distance_metric,
                d.hnsw_m,
                d.hnsw_ef_construction,
                d.dedup_enabled,
            );
    }

    for (map_name, _) in index_factory.all_index_stats() {
        let Some(registry) = index_factory.get_registry(&map_name) else {
            continue;
        };
        let indexes = registry.all_vector_indexes();
        if indexes.is_empty() {
            continue;
        }
        let started = std::time::Instant::now();
        let progress: Vec<Arc<BackfillProgress>> = indexes
            .iter()
            .map(|vi| {
                let entry = Arc::new(BackfillProgress {
                    total: AtomicU64::new(0),
                    processed: AtomicU64::new(0),
                    done: AtomicBool::new(false),
                    rebuild_type: RebuildType::StartupRebuild,
                });
                backfill_progress
                    .insert((map_name.clone(), vi.attribute.clone()), Arc::clone(&entry));
                entry
            })
            .collect();

        let scanned = store_factory
            .scan_merged(&map_name, &mut |key, value| {
                for entry in &progress {
                    entry.total.fetch_add(1, Ordering::Relaxed);
                }
                if let RecordValue::Lww { value, .. } = value {
                    let value = crate::service::domain::predicate::value_to_rmpv(value);
                    for (vi, entry) in indexes.iter().zip(&progress) {
                        vi.insert_if_absent(key, &value);
                        entry.processed.fetch_add(1, Ordering::Relaxed);
                    }
                }
            })
            .await;
        if let Err(error) = scanned {
            tracing::error!(
                map = %map_name,
                error = %error,
                "vector index rebuild: the scan of the map failed; its vector indexes are not marked done"
            );
            continue;
        }

        for (vi, entry) in indexes.iter().zip(&progress) {
            let flushed = vi.commit_pending();
            entry.done.store(true, Ordering::Release);
            info!(
                map = %map_name,
                attribute = %vi.attribute,
                index_name = %vi.index_name,
                count = entry.processed.load(Ordering::Relaxed),
                flushed = flushed,
                elapsed_ms = started.elapsed().as_millis(),
                "vector index rebuilt from store"
            );
        }
    }
}

// ------------------------------------------------------------------
// Startup rebuild tests
// ------------------------------------------------------------------

#[cfg(test)]
mod rebuild_tests {
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use dashmap::DashMap;
    use topgun_core::types::Value;
    use topgun_core::vector::{DistanceMetric, Vector};

    use crate::network::handlers::admin_types::{
        BackfillProgress, RebuildType, VectorIndexDescriptor,
    };
    use crate::service::domain::index::mutation_observer::IndexObserverFactory;
    use crate::service::domain::index::Index;
    use crate::storage::datastores::NullDataStore;
    use crate::storage::expiry::ManualClock;
    use crate::storage::factory::test_store::TestStore;
    use crate::storage::factory::ttl_fixture::{expired_now, ts, wall_stamp, TTL_MS};
    use crate::storage::factory::RecordStoreFactory;
    use crate::storage::impls::StorageConfig;
    use crate::storage::record::RecordValue;

    fn make_store_factory() -> Arc<RecordStoreFactory> {
        Arc::new(RecordStoreFactory::new(
            StorageConfig::default(),
            Arc::new(NullDataStore),
            vec![],
        ))
    }

    fn make_backfill_progress() -> Arc<DashMap<(String, String), Arc<BackfillProgress>>> {
        Arc::new(DashMap::new())
    }

    fn descriptor(map: &str, attribute: &str) -> VectorIndexDescriptor {
        VectorIndexDescriptor {
            map_name: map.to_string(),
            attribute: attribute.to_string(),
            index_name: "emb_idx".to_string(),
            dimension: 4,
            distance_metric: DistanceMetric::Cosine,
            hnsw_m: 16,
            hnsw_ef_construction: 200,
            dedup_enabled: true,
            created_at: "2026-11-06T00:00:00Z".to_string(),
        }
    }

    /// A stored LWW row whose `attribute` holds `data` as an encoded vector.
    fn row(attribute: &str, data: &[f32], millis: u64, ttl_ms: Option<u64>) -> RecordValue {
        let bytes = rmp_serde::to_vec_named(&Vector::F32(data.to_vec())).unwrap();
        RecordValue::Lww {
            value: Value::Map(BTreeMap::from([(
                attribute.to_string(),
                Value::Bytes(bytes),
            )])),
            timestamp: ts(millis),
            ttl_ms,
        }
    }

    fn rmpv_record(attribute: &str, data: &[f32]) -> rmpv::Value {
        let bytes = rmp_serde::to_vec_named(&Vector::F32(data.to_vec())).unwrap();
        rmpv::Value::Map(vec![(
            rmpv::Value::from(attribute),
            rmpv::Value::Binary(bytes),
        )])
    }

    fn keys_at(results: &[(String, f64)], max_distance: f64) -> Vec<String> {
        let mut keys: Vec<String> = results
            .iter()
            .filter(|(_, d)| *d <= max_distance)
            .map(|(k, _)| k.clone())
            .collect();
        keys.sort();
        keys
    }

    fn store_factory_over(store: Arc<TestStore>) -> RecordStoreFactory {
        RecordStoreFactory::new(StorageConfig::default(), store, Vec::new())
    }

    #[tokio::test]
    async fn rebuild_from_store_no_op_when_empty_specs() {
        let factory = IndexObserverFactory::new();
        let store_factory = make_store_factory();
        let bp = make_backfill_progress();
        // Should complete without error and without registering any indexes.
        super::rebuild_from_store(&factory, &store_factory, &[], &bp).await;
        assert_eq!(factory.all_index_stats().len(), 0);
        assert!(bp.is_empty());
    }

    #[tokio::test]
    async fn rebuild_from_store_registers_index_when_no_records() {
        let factory = IndexObserverFactory::new();
        let store_factory = make_store_factory();
        let bp = make_backfill_progress();
        super::rebuild_from_store(
            &factory,
            &store_factory,
            &[descriptor("users", "_embedding")],
            &bp,
        )
        .await;
        // Index should be registered even with no records (empty store).
        let registry = factory
            .get_registry("users")
            .expect("registry should exist");
        assert!(registry.get_vector_index("_embedding").is_some());
        let stats = registry.vector_index_stats();
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].vector_count, 0);
        assert_eq!(stats[0].index_name, "emb_idx");
    }

    #[tokio::test]
    async fn rebuild_from_store_progress_done_and_startup_rebuild_type() {
        let factory = IndexObserverFactory::new();
        let store_factory = make_store_factory();
        let bp = make_backfill_progress();

        super::rebuild_from_store(
            &factory,
            &store_factory,
            &[descriptor("users", "_embedding")],
            &bp,
        )
        .await;

        let entry = bp
            .get(&("users".to_string(), "_embedding".to_string()))
            .expect("progress entry must be created by rebuild_from_store");
        assert!(
            entry.done.load(Ordering::Acquire),
            "done must be true after rebuild_from_store returns"
        );
        assert_eq!(
            entry.rebuild_type,
            RebuildType::StartupRebuild,
            "startup rebuild must use RebuildType::StartupRebuild"
        );
    }

    /// Stored rows that are not resident are indexed, and the rebuild loads
    /// nothing into the record stores.
    #[tokio::test]
    async fn rebuild_indexes_stored_non_resident_records() {
        let store = Arc::new(TestStore::new(2));
        let vectors = [
            ("a", [1.0, 0.0, 0.0, 0.0]),
            ("b", [0.0, 1.0, 0.0, 0.0]),
            ("c", [0.0, 0.0, 1.0, 0.0]),
            ("d", [0.0, 0.0, 0.0, 1.0]),
            ("e", [1.0, 1.0, 0.0, 0.0]),
        ];
        for (key, data) in &vectors {
            store.put_row("m", key, row("embedding", data, 1_000, None));
        }
        let store_factory = store_factory_over(Arc::clone(&store));
        let factory = IndexObserverFactory::new();
        let bp = make_backfill_progress();

        super::rebuild_from_store(
            &factory,
            &store_factory,
            &[descriptor("m", "embedding")],
            &bp,
        )
        .await;

        let vi = factory
            .get_registry("m")
            .and_then(|r| r.get_vector_index("embedding"))
            .expect("index registered");
        assert_eq!(vi.entry_count(), 5);
        for (key, data) in &vectors {
            let hits = vi.search_nearest(data, 1, 16);
            assert_eq!(keys_at(&hits, 1e-6), vec![(*key).to_string()]);
        }
        let resident: usize = store_factory
            .get_all_for_map("m")
            .iter()
            .map(|s| s.size())
            .sum();
        assert_eq!(resident, 0, "the rebuild must not load records into memory");
        let entry = bp.get(&("m".to_string(), "embedding".to_string())).unwrap();
        assert_eq!(entry.total.load(Ordering::Relaxed), 5);
        assert_eq!(entry.processed.load(Ordering::Relaxed), 5);
        assert!(entry.done.load(Ordering::Acquire));
    }

    /// Two stored records with the same vector are two entries after the
    /// rebuild's single commit.
    #[tokio::test]
    async fn rebuild_indexes_keys_that_share_a_vector() {
        let store = Arc::new(TestStore::new(100));
        store.put_row(
            "m",
            "s1",
            row("embedding", &[0.5, 0.5, 0.0, 0.0], 1_000, None),
        );
        store.put_row(
            "m",
            "s2",
            row("embedding", &[0.5, 0.5, 0.0, 0.0], 1_000, None),
        );
        let store_factory = store_factory_over(store);
        let factory = IndexObserverFactory::new();

        super::rebuild_from_store(
            &factory,
            &store_factory,
            &[descriptor("m", "embedding")],
            &make_backfill_progress(),
        )
        .await;

        let vi = factory
            .get_registry("m")
            .and_then(|r| r.get_vector_index("embedding"))
            .unwrap();
        assert_eq!(vi.entry_count(), 2);
        let hits = vi.search_nearest(&[0.5, 0.5, 0.0, 0.0], 2, 16);
        assert_eq!(keys_at(&hits, 1e-6), vec!["s1", "s2"]);
    }

    /// A row expired at the factory clock is not indexed (TG-TTL-001).
    #[tokio::test]
    async fn rebuild_skips_expired_rows() {
        let store = Arc::new(TestStore::new(100));
        let stamp = wall_stamp();
        store.put_row(
            "m",
            "gone",
            row("embedding", &[1.0, 0.0, 0.0, 0.0], stamp, Some(TTL_MS)),
        );
        store.put_row(
            "m",
            "kept",
            row("embedding", &[0.0, 1.0, 0.0, 0.0], stamp, None),
        );
        let store_factory =
            store_factory_over(store).with_clock(Arc::new(ManualClock::new(expired_now())));
        let factory = IndexObserverFactory::new();

        super::rebuild_from_store(
            &factory,
            &store_factory,
            &[descriptor("m", "embedding")],
            &make_backfill_progress(),
        )
        .await;

        let vi = factory
            .get_registry("m")
            .and_then(|r| r.get_vector_index("embedding"))
            .unwrap();
        assert_eq!(vi.entry_count(), 1);
        let hits = vi.search_nearest(&[1.0, 0.0, 0.0, 0.0], 5, 16);
        assert!(hits.iter().all(|(k, _)| k != "gone"), "{hits:?}");
        assert_eq!(keys_at(&hits, 1.5), vec!["kept"]);
    }

    /// An index registered without a descriptor (the `_embedding` index of
    /// `TOPGUN_VECTOR_MAPS`) is filled by a rebuild with no descriptors.
    #[tokio::test]
    async fn rebuild_backfills_indexes_registered_before_the_call() {
        let store = Arc::new(TestStore::new(100));
        store.put_row(
            "docs",
            "a",
            row("_embedding", &[1.0, 0.0, 0.0, 0.0], 1_000, None),
        );
        store.put_row(
            "docs",
            "b",
            row("_embedding", &[0.0, 1.0, 0.0, 0.0], 1_000, None),
        );
        let store_factory = store_factory_over(store);
        let factory = IndexObserverFactory::new();
        factory
            .register_map("docs")
            .add_vector_index("_embedding", 4, DistanceMetric::Cosine);
        let bp = make_backfill_progress();

        super::rebuild_from_store(&factory, &store_factory, &[], &bp).await;

        let vi = factory
            .get_registry("docs")
            .and_then(|r| r.get_vector_index("_embedding"))
            .unwrap();
        assert_eq!(vi.entry_count(), 2);
        let entry = bp
            .get(&("docs".to_string(), "_embedding".to_string()))
            .expect("progress entry");
        assert!(entry.done.load(Ordering::Acquire));
    }

    /// Counts the ERROR events logged while it is the default subscriber.
    #[derive(Clone, Default)]
    struct ErrorCount(Arc<AtomicUsize>);

    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for ErrorCount {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            if *event.metadata().level() == tracing::Level::ERROR {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
    }

    /// A map whose scan fails logs one ERROR and stays `done == false`; the
    /// other maps are rebuilt.
    #[tokio::test]
    async fn rebuild_failed_scan_leaves_done_false_and_continues() {
        use tracing_subscriber::layer::SubscriberExt;

        let store = Arc::new(TestStore::new(100));
        for map in ["bad", "good1", "good2"] {
            store.put_row(
                map,
                "a",
                row("embedding", &[1.0, 0.0, 0.0, 0.0], 1_000, None),
            );
        }
        store.fail_scans("bad", true);
        let store_factory = store_factory_over(store);
        let factory = IndexObserverFactory::new();
        let bp = make_backfill_progress();
        let descriptors = ["bad", "good1", "good2"].map(|m| descriptor(m, "embedding"));

        let errors = ErrorCount::default();
        let guard =
            tracing::subscriber::set_default(tracing_subscriber::registry().with(errors.clone()));
        super::rebuild_from_store(&factory, &store_factory, &descriptors, &bp).await;
        drop(guard);

        assert_eq!(errors.0.load(Ordering::SeqCst), 1, "one ERROR line");
        let done = |map: &str| {
            bp.get(&(map.to_string(), "embedding".to_string()))
                .expect("progress entry")
                .done
                .load(Ordering::Acquire)
        };
        assert!(!done("bad"));
        for map in ["good1", "good2"] {
            assert!(done(map), "{map} must be rebuilt after the failed map");
            let vi = factory
                .get_registry(map)
                .and_then(|r| r.get_vector_index("embedding"))
                .unwrap();
            assert_eq!(vi.entry_count(), 1);
        }
    }

    /// A key that an index registered without a descriptor already holds
    /// under a newer vector keeps it when the scan meets its older stored row.
    #[tokio::test]
    async fn rebuild_keeps_a_newer_entry() {
        let store = Arc::new(TestStore::new(100));
        store.put_row(
            "docs",
            "k",
            row("_embedding", &[1.0, 0.0, 0.0, 0.0], 1_000, None),
        );
        let store_factory = store_factory_over(store);
        let factory = IndexObserverFactory::new();
        let registry = factory.register_map("docs");
        registry.add_vector_index("_embedding", 4, DistanceMetric::Cosine);
        let vi = registry.get_vector_index("_embedding").unwrap();
        vi.insert("k", &rmpv_record("_embedding", &[0.0, 1.0, 0.0, 0.0]));

        super::rebuild_from_store(&factory, &store_factory, &[], &make_backfill_progress()).await;

        assert_eq!(vi.entry_count(), 1);
        let hits = vi.search_nearest(&[0.0, 1.0, 0.0, 0.0], 1, 16);
        assert_eq!(
            keys_at(&hits, 1e-6),
            vec!["k"],
            "the newer vector must survive the rebuild: {hits:?}"
        );
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use topgun_core::vector::{DistanceMetric, Vector};

    use super::*;

    fn make_index() -> VectorIndex {
        VectorIndex::new(
            "embedding",
            "embedding_index",
            4,
            DistanceMetric::Cosine,
            true,
        )
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

    // --- stats, optimize ---

    #[test]
    fn stats_returns_correct_vector_count() {
        let idx = make_index();
        let r1 = make_record(&[0.1, 0.2, 0.3, 0.4]);
        idx.insert("k1", &r1);
        idx.commit_pending();

        let stats = idx.stats();
        assert_eq!(stats.vector_count, 1);
        assert_eq!(stats.dimension, 4);
        assert_eq!(stats.index_name, "embedding_index");
        assert_eq!(stats.pending_updates, 0);
    }

    #[test]
    fn stats_pending_updates_reflects_uncommitted_mutations() {
        let idx = make_index();
        let r1 = make_record(&[0.1, 0.2, 0.3, 0.4]);
        idx.insert("k1", &r1);
        // Not committed yet.
        let stats = idx.stats();
        assert_eq!(stats.pending_updates, 1);
        assert_eq!(stats.vector_count, 0);
    }

    #[test]
    fn stats_memory_bytes_nonzero_after_insert_and_commit() {
        let idx = make_index();
        let r1 = make_record(&[0.1, 0.2, 0.3, 0.4]);
        idx.insert("k1", &r1);
        idx.commit_pending();

        let stats = idx.stats();
        // Approximate: 1 vector * 4 dims * 4 bytes + 1 * 16 * 8 = 144 bytes
        assert!(stats.memory_bytes > 0, "memory_bytes should be nonzero");
    }

    fn hits(idx: &VectorIndex, query: &[f32], k: usize) -> Vec<(String, f64)> {
        idx.search_nearest(query, k, 16)
    }

    fn returns(idx: &VectorIndex, query: &[f32], key: &str) -> bool {
        hits(idx, query, 16).iter().any(|(k, _)| k == key)
    }

    fn returns_at_zero(idx: &VectorIndex, query: &[f32], key: &str) -> bool {
        hits(idx, query, 16)
            .iter()
            .any(|(k, d)| k == key && d.abs() < 1e-6)
    }

    fn no_vector() -> rmpv::Value {
        rmpv::Value::Map(vec![(rmpv::Value::from("other"), rmpv::Value::from("x"))])
    }

    fn wrong_dimension() -> rmpv::Value {
        make_record(&[1.0, 0.0])
    }

    const V1: [f32; 4] = [1.0, 0.0, 0.0, 0.0];
    const V2: [f32; 4] = [0.0, 1.0, 0.0, 0.0];
    const V2B: [f32; 4] = [0.0, 0.0, 1.0, 0.0];
    const V3: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
    const V4: [f32; 4] = [1.0, 1.0, 0.0, 0.0];

    #[test]
    fn vector_remove_key_removes_without_old_value() {
        // Committed entry.
        let idx = make_index();
        idx.insert("k", &make_record(&V1));
        idx.commit_pending();
        idx.remove_key("k");
        assert!(!returns(&idx, &V1, "k"));
        assert_eq!(idx.pending_count(), 1);
        idx.commit_pending();
        assert_eq!(idx.entry_count(), 0);
        assert!(!returns(&idx, &V1, "k"));

        // Pending entry.
        let idx = make_index();
        idx.insert("k", &make_record(&V1));
        idx.remove_key("k");
        assert!(!returns(&idx, &V1, "k"));
        idx.commit_pending();
        assert_eq!(idx.entry_count(), 0);
        assert!(!returns(&idx, &V1, "k"));
    }

    #[test]
    fn vector_insert_if_absent_keeps_the_existing_entry() {
        // Pending entry.
        let idx = make_index();
        idx.insert("k", &make_record(&V1));
        idx.insert_if_absent("k", &make_record(&V2));
        assert_eq!(idx.pending_count(), 1);
        assert!(returns_at_zero(&idx, &V1, "k"));
        idx.commit_pending();
        assert!(returns_at_zero(&idx, &V1, "k"));
        assert!(!returns_at_zero(&idx, &V2, "k"));

        // Committed entry.
        idx.insert_if_absent("k", &make_record(&V2));
        assert_eq!(idx.pending_count(), 0);
        idx.commit_pending();
        assert!(returns_at_zero(&idx, &V1, "k"));
        assert_eq!(idx.entry_count(), 1);

        // A removed key has no entry: insert_if_absent inserts it.
        idx.remove_key("k");
        idx.insert_if_absent("k", &make_record(&V2));
        assert!(returns_at_zero(&idx, &V2, "k"));
        idx.commit_pending();
        assert!(returns_at_zero(&idx, &V2, "k"));
        assert_eq!(idx.entry_count(), 1);
    }

    #[test]
    fn vector_insert_without_a_decodable_vector_drops_the_entry() {
        for value in [no_vector(), wrong_dimension()] {
            let idx = make_index();
            idx.insert("k", &make_record(&V1));
            idx.insert("other", &make_record(&V2));
            idx.commit_pending();
            let before = idx.entry_count();

            idx.insert("k", &value);
            assert!(!returns(&idx, &V1, "k"), "{value:?}");
            assert_eq!(idx.pending_count(), 1, "{value:?}");
            idx.commit_pending();
            assert_eq!(idx.entry_count(), before - 1, "{value:?}");
            assert!(!returns(&idx, &V1, "k"), "{value:?}");
        }
    }

    #[test]
    fn vector_insert_without_a_decodable_vector_is_a_no_op_without_an_entry() {
        let idx = make_index();
        for value in [no_vector(), wrong_dimension()] {
            idx.insert("k", &value);
            idx.insert_if_absent("k", &value);
        }
        assert_eq!(idx.pending_count(), 0);
        assert!(!idx.pending_removed.contains("k"));
    }

    #[test]
    fn vector_same_vector_under_two_keys_gets_two_entries() {
        let idx = make_index();
        idx.insert("a", &make_record(&V1));
        idx.insert("b", &make_record(&V1));
        idx.commit_pending();
        assert_eq!(idx.entry_count(), 2);
        let found: Vec<String> = hits(&idx, &V1, 2)
            .into_iter()
            .filter(|(_, d)| d.abs() < 1e-6)
            .map(|(k, _)| k)
            .collect();
        assert_eq!(found.len(), 2, "{found:?}");
    }

    #[test]
    fn vector_remove_key_then_same_vector_under_a_new_key_is_committed() {
        let idx = make_index();
        idx.insert("a", &make_record(&V1));
        idx.commit_pending();
        idx.remove_key("a");
        idx.commit_pending();
        idx.insert("b", &make_record(&V1));
        idx.commit_pending();
        assert_eq!(idx.entry_count(), 1);
        assert!(idx.key_to_id.contains_key("b"));
    }

    /// Mutations queued between the drain and the settle of a commit keep
    /// their snapshot entries and removal marks until they are committed.
    #[test]
    fn vector_commit_keeps_the_marks_of_mutations_queued_after_the_drain() {
        let idx = make_index();
        idx.insert("k1", &make_record(&V1));
        idx.commit_pending();
        idx.insert("k2", &make_record(&V2));

        let drained = idx.take_pending();
        assert_eq!(drained.len(), 1);
        idx.insert("k3", &make_record(&V3));
        idx.remove_key("k1");
        idx.insert("k2", &make_record(&V2B));
        idx.apply_to_graph(&drained);
        idx.insert("k4", &make_record(&V4));
        idx.settle_drained(&drained);

        assert!(returns_at_zero(&idx, &V3, "k3"));
        assert!(returns_at_zero(&idx, &V4, "k4"));
        assert!(!returns(&idx, &V1, "k1"));
        assert!(returns_at_zero(&idx, &V2B, "k2"));
        assert_eq!(idx.pending_count(), 4);

        idx.commit_pending();
        assert_eq!(idx.entry_count(), 3);
        assert!(!idx.key_to_id.contains_key("k1"));
        assert!(idx.pending_snapshot.is_empty());
        assert!(idx.pending_removed.is_empty());
    }

    /// Stress check, no mutant: whatever the interleaving, a racing load never
    /// replaces the write.
    #[test]
    fn vector_concurrent_insert_if_absent_and_insert_leave_the_insert() {
        for _ in 0..200 {
            let idx = Arc::new(make_index());
            let loader = {
                let idx = Arc::clone(&idx);
                std::thread::spawn(move || {
                    for _ in 0..20 {
                        idx.insert_if_absent("k", &make_record(&V1));
                    }
                })
            };
            idx.insert("k", &make_record(&V2));
            loader.join().unwrap();
            assert!(returns_at_zero(&idx, &V2, "k"));
            idx.commit_pending();
            assert!(returns_at_zero(&idx, &V2, "k"));
            assert_eq!(idx.entry_count(), 1);
        }
    }

    /// The vector field the TS client writes: msgpackr
    /// `Uint8Array.from(pack({ type: 'f32', data: vectorToBytes([1, 0.5, -2, 0.25]) }))`
    /// (msgpackr 1.11.8), the encoding `vector-search-restart.test.ts` uses.
    #[test]
    fn decodes_a_vector_field_encoded_by_the_ts_client() {
        const FIXTURE: [u8; 35] = [
            0xde, 0x00, 0x02, 0xa4, 0x74, 0x79, 0x70, 0x65, 0xa3, 0x66, 0x33, 0x32, 0xa4, 0x64,
            0x61, 0x74, 0x61, 0xc4, 0x10, 0x00, 0x00, 0x80, 0x3f, 0x00, 0x00, 0x00, 0x3f, 0x00,
            0x00, 0x00, 0xc0, 0x00, 0x00, 0x80, 0x3e,
        ];
        let record = rmpv::Value::Map(vec![(
            rmpv::Value::from("embedding"),
            rmpv::Value::Binary(FIXTURE.to_vec()),
        )]);
        let decoded = make_index()
            .decode_vector_from_record(&record)
            .expect("the TS encoding decodes");
        assert_eq!(*decoded.vector(), Vector::F32(vec![1.0, 0.5, -2.0, 0.25]));
    }

    #[tokio::test]
    async fn optimize_completes_and_sets_last_optimized() {
        let idx = Arc::new(VectorIndex::new(
            "embedding",
            "emb_idx",
            4,
            DistanceMetric::Cosine,
            false,
        ));

        // Insert and commit some vectors.
        let r1 = make_record(&[1.0, 0.0, 0.0, 0.0]);
        let r2 = make_record(&[0.0, 1.0, 0.0, 0.0]);
        idx.insert("k1", &r1);
        idx.insert("k2", &r2);
        idx.commit_pending();

        assert!(
            idx.stats().last_optimized.is_none(),
            "should be None before optimize"
        );

        let (handle, was_already_running) = idx.optimize();
        assert!(
            !was_already_running,
            "first optimize should not report already_running"
        );

        // Wait for the background task to complete.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            if handle.finished.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }
            assert!(
                std::time::Instant::now() <= deadline,
                "optimize did not complete in time"
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        let stats = idx.stats();
        assert!(
            stats.last_optimized.is_some(),
            "last_optimized should be set after optimize"
        );
        // The ISO-8601 string should start with "20".
        assert!(
            stats.last_optimized.as_ref().unwrap().starts_with("20"),
            "last_optimized should be a valid ISO-8601 UTC timestamp"
        );

        // Vectors must still be searchable after optimize (Critical #1 guard).
        assert_eq!(
            idx.stats().vector_count,
            2,
            "optimize must not destroy committed vectors"
        );
        let results = idx.search_nearest(&[1.0, 0.0, 0.0, 0.0], 1, 8);
        assert_eq!(
            results.len(),
            1,
            "search after optimize should return results"
        );
        assert_eq!(results[0].0, "k1", "nearest to [1,0,0,0] should be k1");
    }

    #[tokio::test]
    async fn optimize_idempotent_returns_same_handle_when_running() {
        let idx = Arc::new(VectorIndex::new(
            "embedding",
            "emb_idx",
            4,
            DistanceMetric::Cosine,
            false,
        ));

        let (h1, already_running_first) = idx.optimize();
        assert!(
            !already_running_first,
            "first call should not report already_running"
        );

        let (h2, already_running_second) = idx.optimize();

        // While h1 is still running, h2 should return the same handle with already_running=true.
        if !h1.finished.load(std::sync::atomic::Ordering::Relaxed) {
            assert_eq!(h1.id, h2.id, "in-flight optimize should return same handle");
            assert!(
                already_running_second,
                "second call while running should report already_running=true"
            );
        }

        // Wait for completion.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            if h1.finished.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }
            assert!(
                std::time::Instant::now() <= deadline,
                "optimize did not complete in time"
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn cancel_mid_rebuild_does_not_swap_hnsw_graph() {
        // Build an index with committed vectors; count before and after cancel must match.
        let idx = Arc::new(VectorIndex::new(
            "embedding",
            "emb_idx",
            4,
            DistanceMetric::Cosine,
            false,
        ));

        let records = [
            make_record(&[1.0, 0.0, 0.0, 0.0]),
            make_record(&[0.0, 1.0, 0.0, 0.0]),
            make_record(&[0.0, 0.0, 1.0, 0.0]),
        ];
        for (i, rec) in records.iter().enumerate() {
            idx.insert(&format!("k{i}"), rec);
        }
        idx.commit_pending();
        let count_before = idx.entry_count();
        let last_optimized_before = idx.stats().last_optimized.clone();

        // Start optimize and immediately signal cancellation.
        let (handle, _) = idx.optimize();
        handle.cancelled.store(true, Ordering::Relaxed);

        // Wait for the background task to finish.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            if handle.finished.load(Ordering::Relaxed) {
                break;
            }
            assert!(
                std::time::Instant::now() <= deadline,
                "cancelled optimize did not finish in time"
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        // Invariants: cancelled flag set, finished set, live graph unchanged.
        assert!(
            handle.cancelled.load(Ordering::Relaxed),
            "cancelled flag must be true"
        );
        assert!(
            handle.finished.load(Ordering::Relaxed),
            "finished flag must be true after cancel"
        );
        assert_eq!(
            idx.entry_count(),
            count_before,
            "live graph vector count must be unchanged after cancel"
        );
        assert_eq!(
            idx.stats().last_optimized,
            last_optimized_before,
            "last_optimized must NOT be updated after cancel"
        );
    }

    #[tokio::test]
    async fn cancel_then_reoptimize_runs_to_completion() {
        let idx = Arc::new(VectorIndex::new(
            "embedding",
            "emb_idx",
            4,
            DistanceMetric::Cosine,
            false,
        ));

        let r1 = make_record(&[1.0, 0.0, 0.0, 0.0]);
        let r2 = make_record(&[0.0, 1.0, 0.0, 0.0]);
        idx.insert("k1", &r1);
        idx.insert("k2", &r2);
        idx.commit_pending();

        // First optimize: cancel immediately.
        let (handle1, _) = idx.optimize();
        let id1 = handle1.id.clone();
        handle1.cancelled.store(true, Ordering::Relaxed);

        // Wait for the cancelled optimize to finish.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            if handle1.finished.load(Ordering::Relaxed) {
                break;
            }
            assert!(
                std::time::Instant::now() <= deadline,
                "first optimize did not finish"
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        // Second optimize: must start a fresh run with a new optimization_id.
        let (handle2, was_already_running) = idx.optimize();
        assert!(
            !was_already_running,
            "after cancel+finish, new optimize should not report already_running"
        );
        assert_ne!(
            handle2.id, id1,
            "second optimize must have a distinct optimization_id"
        );

        // Wait for the fresh optimize to complete.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            if handle2.finished.load(Ordering::Relaxed) {
                break;
            }
            assert!(
                std::time::Instant::now() <= deadline,
                "second optimize did not finish"
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        // The second optimize completed successfully: last_optimized is now set,
        // and cancelled is false on handle2.
        assert!(
            !handle2.cancelled.load(Ordering::Relaxed),
            "second optimize must not be cancelled"
        );
        assert!(
            idx.stats().last_optimized.is_some(),
            "last_optimized must be set after successful re-optimize"
        );
        assert_eq!(
            idx.entry_count(),
            2,
            "vectors must be preserved through cancel+reoptimize"
        );
    }
}
