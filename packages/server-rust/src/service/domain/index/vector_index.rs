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

use crate::network::handlers::admin_types::{BackfillProgress, RebuildType};
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
    /// When `true`, BLAKE3 hashes of inserted vectors are recorded in `blake3_seen`
    /// and duplicates skip HNSW insertion. Defaults to `true` (`SurrealDB` pattern).
    dedup_enabled: bool,
    /// BLAKE3-256 hashes of all committed vectors (when dedup is enabled).
    /// `DashSet::insert` returns `false` for duplicates — the atomic check-and-insert
    /// prevents double-counting even under concurrent writes.
    blake3_seen: DashSet<[u8; 32]>,
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
    /// `dedup_enabled` controls BLAKE3-based duplicate vector suppression (default: `true`).
    pub fn new(
        attribute: impl Into<String>,
        index_name: impl Into<String>,
        dimension: u16,
        distance_metric: DistanceMetric,
        dedup_enabled: bool,
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
            dedup_enabled,
            blake3_seen: DashSet::new(),
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

    /// Computes the BLAKE3-256 hash of the raw f32 byte representation of a vector.
    ///
    /// The vector is serialized as little-endian f32 values (matching `Vector::to_f32_bytes_le`).
    /// The 32-byte hash is stored in `blake3_seen` for duplicate detection.
    fn blake3_hash(vector: &SharedVector) -> [u8; 32] {
        let bytes = vector.vector().to_f32_bytes_le();
        *blake3::hash(&bytes).as_bytes()
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
        let Some(vector) = self.decode_vector_from_record(value) else {
            return;
        };

        // BLAKE3 deduplication: skip HNSW insertion if this exact vector bytes
        // have already been committed. `DashSet::insert` returns `false` if the
        // hash was already present — the atomic check-and-insert prevents
        // duplicate HNSW nodes even under concurrent writes.
        if self.dedup_enabled {
            let hash = Self::blake3_hash(&vector);
            if !self.blake3_seen.insert(hash) {
                // Duplicate — update the pending_snapshot so reads return the key,
                // but skip HNSW mutation (the vector data is already in the graph).
                self.pending_snapshot
                    .insert(key.to_string(), vector.clone());
                self.pending_removed.remove(key);
                return;
            }
        }

        self.pending_snapshot
            .insert(key.to_string(), vector.clone());
        self.pending_removed.remove(key);
        self.pending.lock().push(VectorPendingUpdate::Upsert {
            key: key.to_string(),
            vector,
        });
    }

    fn update(&self, key: &str, _old_value: &rmpv::Value, new_value: &rmpv::Value) {
        // HNSW has no native update; delegate to insert (commit path does
        // remove-then-insert for keys already in the graph).
        self.insert(key, new_value);
    }

    fn remove(&self, key: &str, old_value: &rmpv::Value) {
        // Evict the BLAKE3 hash when the vector is removed, so the same bytes
        // can be inserted again after deletion.
        if self.dedup_enabled {
            if let Some(vector) = self.decode_vector_from_record(old_value) {
                let hash = Self::blake3_hash(&vector);
                self.blake3_seen.remove(&hash);
            }
            // If old_value doesn't decode (e.g. Nil), we cannot evict the hash,
            // but the HNSW node is still removed via the pending queue.
        }
        self.pending_snapshot.remove(key);
        self.pending_removed.insert(key.to_string());
        self.pending.lock().push(VectorPendingUpdate::Remove {
            key: key.to_string(),
        });
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
        self.blake3_seen.clear();
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

/// Minimal descriptor for re-registering and rebuilding a vector index at startup.
///
/// Populated from the on-disk `VectorIndexDescriptor` JSON (loaded in `module.rs`).
/// Keeping a separate struct avoids a circular import between the service domain
/// layer and the network handler layer.
pub struct VectorRebuildSpec {
    /// Map name this index belongs to.
    pub map_name: String,
    /// Attribute name this index covers.
    pub attribute: String,
    /// User-visible index name.
    pub index_name: String,
    /// Vector dimensionality.
    pub dimension: u16,
    /// Distance metric.
    pub distance_metric: DistanceMetric,
    /// HNSW m parameter.
    pub hnsw_m: u16,
    /// HNSW `ef_construction` parameter.
    pub hnsw_ef_construction: u32,
    /// BLAKE3 dedup setting.
    pub dedup_enabled: bool,
}

/// Re-registers and rebuilds vector indexes from persisted record stores after restart.
///
/// Called from `NetworkModule::serve` after `start()` binds the TCP listener and
/// before `set_ready()` is called, ensuring vector indexes are fully populated
/// before `VECTOR_SEARCH` requests are served.
///
/// For each descriptor:
/// 1. Re-registers the vector index via `registry.add_vector_index_with_params`.
/// 2. Creates a `BackfillProgress` entry with `rebuild_type = StartupRebuild` before iterating.
/// 3. Iterates all partition stores for the map and inserts records containing
///    the attribute of matching dimension into the rebuilt index.
/// 4. Calls `commit_pending()` to flush all buffered mutations into the HNSW graph.
/// 5. Sets `done = true` on the progress entry.
/// 6. Logs progress at INFO level.
///
/// Missing `store_factory` records are a no-op (clean first start).
///
/// The `backfill_progress` `Arc` is the same instance stored in `AppState` —
/// progress entries written here are visible through the admin status endpoint
/// immediately after `set_ready()`.
pub async fn rebuild_from_store(
    index_factory: &crate::service::domain::index::mutation_observer::IndexObserverFactory,
    store_factory: &RecordStoreFactory,
    specs: &[VectorRebuildSpec],
    backfill_progress: &Arc<DashMap<(String, String), Arc<BackfillProgress>>>,
) {
    // Emit the Arc strong_count at entry — confirms shared-identity invariant
    // (count must be >= 2: one in the caller, one passed into this function).
    tracing::debug!(
        "rebuild_from_store: Arc strong_count={}",
        Arc::strong_count(backfill_progress)
    );

    if specs.is_empty() {
        return;
    }

    for spec in specs {
        let started = std::time::Instant::now();
        let registry = index_factory.register_map(&spec.map_name);

        let vi = registry.add_vector_index_with_params(
            spec.attribute.clone(),
            spec.index_name.clone(),
            spec.dimension,
            spec.distance_metric,
            spec.hnsw_m,
            spec.hnsw_ef_construction,
            spec.dedup_enabled,
        );

        // Create progress entry before iteration so operators can observe liveness
        // through the backfill status endpoint during the rebuild window.
        // `total = 0` here because a cheap pre-count API is not available on RecordStore;
        // `processed` provides a liveness signal, and `done = true` marks completion.
        let progress = Arc::new(BackfillProgress {
            total: AtomicU64::new(0),
            processed: AtomicU64::new(0),
            done: AtomicBool::new(false),
            rebuild_type: RebuildType::StartupRebuild,
        });
        backfill_progress.insert(
            (spec.map_name.clone(), spec.attribute.clone()),
            Arc::clone(&progress),
        );

        let stores = store_factory.get_all_for_map(&spec.map_name);
        let mut count: u64 = 0;

        for store in &stores {
            store.for_each_boxed(
                &mut |key, record| {
                    if let RecordValue::Lww { ref value, .. } = record.value {
                        let rmpv_val = crate::service::domain::predicate::value_to_rmpv(value);
                        vi.insert(key, &rmpv_val);
                        count += 1;
                        progress.processed.fetch_add(1, Ordering::Relaxed);
                    }
                },
                false,
            );
        }

        // Flush all buffered inserts into the HNSW graph.
        let flushed = vi.commit_pending();
        // Mark done after commit_pending() — operators see done=true only when
        // the HNSW graph is fully populated and ready to serve queries.
        progress.done.store(true, Ordering::Release);
        let elapsed = started.elapsed();

        info!(
            map = %spec.map_name,
            attribute = %spec.attribute,
            index_name = %spec.index_name,
            count = count,
            flushed = flushed,
            elapsed_ms = elapsed.as_millis(),
            "vector index rebuilt from store"
        );
    }
}

// ------------------------------------------------------------------
// Startup rebuild tests
// ------------------------------------------------------------------

#[cfg(test)]
mod rebuild_tests {
    use std::sync::Arc;

    use dashmap::DashMap;
    use topgun_core::vector::DistanceMetric;

    use super::VectorRebuildSpec;
    use crate::network::handlers::admin_types::{BackfillProgress, RebuildType};
    use crate::service::domain::index::mutation_observer::IndexObserverFactory;
    use crate::storage::datastores::NullDataStore;
    use crate::storage::factory::RecordStoreFactory;
    use crate::storage::impls::StorageConfig;

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

    #[tokio::test]
    async fn rebuild_from_store_no_op_when_empty_specs() {
        let factory = IndexObserverFactory::new();
        let store_factory = make_store_factory();
        let bp = make_backfill_progress();
        // Should complete without error and without registering any indexes.
        super::rebuild_from_store(&factory, &store_factory, &[], &bp).await;
        assert_eq!(factory.all_index_stats().len(), 0);
    }

    #[tokio::test]
    async fn rebuild_from_store_registers_index_when_no_records() {
        let factory = IndexObserverFactory::new();
        let store_factory = make_store_factory();
        let bp = make_backfill_progress();
        let specs = vec![VectorRebuildSpec {
            map_name: "users".to_string(),
            attribute: "_embedding".to_string(),
            index_name: "emb_idx".to_string(),
            dimension: 4,
            distance_metric: DistanceMetric::Cosine,
            hnsw_m: 16,
            hnsw_ef_construction: 200,
            dedup_enabled: true,
        }];
        super::rebuild_from_store(&factory, &store_factory, &specs, &bp).await;
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
        use std::sync::atomic::Ordering;

        let factory = IndexObserverFactory::new();
        let store_factory = make_store_factory();
        let bp = make_backfill_progress();

        let specs = vec![VectorRebuildSpec {
            map_name: "users".to_string(),
            attribute: "_embedding".to_string(),
            index_name: "emb_idx".to_string(),
            dimension: 4,
            distance_metric: DistanceMetric::Cosine,
            hnsw_m: 16,
            hnsw_ef_construction: 200,
            dedup_enabled: true,
        }];

        super::rebuild_from_store(&factory, &store_factory, &specs, &bp).await;

        // Progress entry must exist after rebuild.
        let entry = bp
            .get(&("users".to_string(), "_embedding".to_string()))
            .expect("progress entry must be created by rebuild_from_store");

        // done must be set after commit_pending() (AC #11).
        assert!(
            entry.done.load(Ordering::Acquire),
            "done must be true after rebuild_from_store returns"
        );

        // rebuild_type must be StartupRebuild (AC #12).
        assert_eq!(
            entry.rebuild_type,
            RebuildType::StartupRebuild,
            "startup rebuild must use RebuildType::StartupRebuild"
        );
    }

    #[test]
    fn rebuild_spec_fields_preserved() {
        let spec = VectorRebuildSpec {
            map_name: "m".to_string(),
            attribute: "a".to_string(),
            index_name: "n".to_string(),
            dimension: 8,
            distance_metric: DistanceMetric::Euclidean,
            hnsw_m: 12,
            hnsw_ef_construction: 100,
            dedup_enabled: false,
        };
        assert_eq!(spec.dimension, 8);
        assert_eq!(spec.hnsw_m, 12);
        assert!(!spec.dedup_enabled);
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

    // --- G3: stats, dedup, optimize ---

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

    #[test]
    fn dedup_prevents_duplicate_hnsw_insert() {
        // Two inserts with identical vectors — only one should go to HNSW.
        let idx = VectorIndex::new("embedding", "emb_idx", 4, DistanceMetric::Cosine, true);
        let record = make_record(&[1.0, 0.0, 0.0, 0.0]);
        idx.insert("k1", &record);
        idx.commit_pending();
        assert_eq!(idx.entry_count(), 1);

        // Second insert of same bytes to a different key — dedup should skip HNSW.
        idx.insert("k2", &record);
        idx.commit_pending();
        // Dedup fires: k2 gets the same BLAKE3 hash as k1, HNSW insert is skipped.
        // key_to_id has k1 only; entry_count stays 1.
        assert_eq!(
            idx.entry_count(),
            1,
            "dedup should prevent second HNSW insert"
        );
    }

    #[test]
    fn dedup_disabled_allows_duplicate_hnsw_insert() {
        let idx = VectorIndex::new("embedding", "emb_idx", 4, DistanceMetric::Cosine, false);
        let record = make_record(&[1.0, 0.0, 0.0, 0.0]);
        idx.insert("k1", &record);
        idx.commit_pending();
        assert_eq!(idx.entry_count(), 1);

        // With dedup disabled, second insert proceeds to HNSW.
        idx.insert("k2", &record);
        // pending queue should have 1 upsert for k2.
        assert_eq!(idx.pending_count(), 1);
    }

    #[tokio::test]
    async fn concurrent_dedup_same_embedding_yields_single_entry() {
        use std::sync::Arc;

        let idx = Arc::new(VectorIndex::new(
            "embedding",
            "emb_idx",
            4,
            DistanceMetric::Cosine,
            true,
        ));
        let record = make_record(&[1.0, 0.0, 0.0, 0.0]);

        // Two concurrent writers with the same vector bytes.
        let idx1 = Arc::clone(&idx);
        let r1 = record.clone();
        let t1 = tokio::task::spawn_blocking(move || idx1.insert("k1", &r1));
        let idx2 = Arc::clone(&idx);
        let r2 = record.clone();
        let t2 = tokio::task::spawn_blocking(move || idx2.insert("k2", &r2));

        let _ = tokio::join!(t1, t2);
        idx.commit_pending();

        // DashSet::insert is atomic — only one writer succeeds in reserving the hash.
        // entry_count should be 1.
        assert_eq!(
            idx.entry_count(),
            1,
            "concurrent dedup: only one HNSW node expected"
        );
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
