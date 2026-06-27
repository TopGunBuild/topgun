//! External persistence backend trait for the storage layer.
//!
//! Defines [`MapDataStore`], the Layer 3 abstraction over write-through and
//! write-behind persistence strategies. The [`RecordStore`](super::RecordStore)
//! calls `add()` / `remove()` on every mutation; the implementation decides
//! when and how to actually persist the data.
//!
//! Also defines [`DurableMerkleIndex`] and [`MerkleSession`]: a residency-
//! independent Merkle computation surface that materialises the full coordinate-
//! trie once from durable storage and serves repeated drill-down calls from that
//! snapshot, so no re-enumeration is needed per query.

use std::collections::HashMap;

use async_trait::async_trait;

use super::record::RecordValue;
use topgun_core::hash::fnv1a_hash;

/// Which CRDT-kind tree a durable leaf belongs to.
///
/// The write path keeps a SEPARATE Merkle tree per CRDT kind — LWW leaves in
/// the LWW tree, OR-Map leaves in the OR-Map tree. The rebuild consumer must
/// reproduce that split exactly, but a bare `u32` leaf hash carries no kind
/// information. This discriminator lets the rebuild sink route each enumerated
/// leaf to the same tree the write-path observer would have written, so the
/// rebuilt roots match the pre-crash roots for maps that mix both kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MerkleLeafKind {
    /// Last-Write-Wins record — routes to the LWW tree.
    Lww,
    /// Observed-Remove Map record — routes to the OR-Map tree.
    OrMap,
}

/// A single durable record's Merkle leaf coordinate: its key, CRDT kind, and
/// the `u32` leaf hash computed over the persisted value.
///
/// `leaf_hash` is the same `u32` space as the in-memory Merkle leaf hash
/// (`fnv1a`-derived); enumerating it from the durable store lets the Merkle
/// root be rebuilt from persistence WITHOUT loading full record values into
/// memory. `kind` tells the rebuild consumer which per-CRDT tree to fold the
/// leaf into, mirroring the write-path observer's per-kind tree separation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MerkleLeaf {
    /// Record key within the map.
    pub key: String,
    /// CRDT kind, selecting the LWW vs OR-Map tree on rebuild.
    pub kind: MerkleLeafKind,
    /// `u32` leaf hash over the persisted value (matches in-memory leaf hash).
    pub leaf_hash: u32,
}

/// Compute the Merkle leaf coordinate (CRDT kind + `u32` leaf hash) for a
/// record value — the single source of truth shared by the write-path observer
/// and every durable enumeration backend.
///
/// LWW leaves hash `"{key}:{millis}:{counter}:{node_id}"`; OR-Map leaves hash
/// the sorted active + tombstone tag sets (`"key:{key}|{tags}#{tombs}"`), so a
/// removal still changes the leaf and peers can observe a tombstone-only delta.
/// Returns `None` for `OrTombstones`: the write path removes such keys from the
/// OR-Map tree rather than contributing a leaf, so enumeration must likewise
/// emit no leaf to keep a rebuilt root identical to the live one.
///
/// Keeping this in one place is load-bearing: a Merkle root rebuilt from
/// persistence must be byte-identical to the live root, so this formula must
/// never drift between the observer and the storage backends. Callers that
/// fold leaves into per-CRDT trees route on the returned [`MerkleLeafKind`].
#[must_use]
pub fn merkle_leaf_hash(key: &str, value: &RecordValue) -> Option<(MerkleLeafKind, u32)> {
    match value {
        RecordValue::Lww { timestamp, .. } => Some((
            MerkleLeafKind::Lww,
            fnv1a_hash(&format!(
                "{key}:{}:{}:{}",
                timestamp.millis, timestamp.counter, timestamp.node_id
            )),
        )),
        RecordValue::OrMap {
            records,
            tombstones,
        } => {
            let mut tags: Vec<&str> = records.iter().map(|r| r.tag.as_str()).collect();
            tags.sort_unstable();
            let joined = tags.join("|");
            let mut tomb_tags: Vec<&str> = tombstones.iter().map(String::as_str).collect();
            tomb_tags.sort_unstable();
            let joined_tombs = tomb_tags.join("|");
            Some((
                MerkleLeafKind::OrMap,
                fnv1a_hash(&format!("key:{key}|{joined}#{joined_tombs}")),
            ))
        }
        RecordValue::OrTombstones { .. } => None,
    }
}

/// A bounded batch of fully-loaded durable records produced by a value-streamed
/// scan.
///
/// Batches are sized so that the resident cost of `records` stays under the
/// `TOPGUN_MAX_RAM_MB` ceiling; the scan never materializes the whole map at
/// once. `next_cursor` is `None` once enumeration is exhausted.
#[derive(Debug, Default)]
pub struct ScanBatch {
    /// The records in this batch, as `(key, value)` pairs.
    pub records: Vec<(String, RecordValue)>,
    /// Opaque resume token for the next batch, or `None` when exhausted.
    pub next_cursor: Option<ScanCursor>,
}

/// Opaque, backend-defined resume token for a value-streamed scan.
///
/// The byte payload is interpreted only by the producing backend (e.g. a redb
/// last-key marker or a Postgres keyset offset). Callers treat it as opaque and
/// pass it back unchanged to fetch the next [`ScanBatch`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanCursor(pub Vec<u8>);

/// Async sink invoked once per bounded batch of Merkle leaves during
/// enumeration.
///
/// The enumeration drives paging internally and calls [`consume`](LeafSink::consume)
/// for each batch, so the async caller can `.await` per batch (e.g. fold leaves
/// into a Merkle tree) WITHOUT the producer ever holding the whole key set in
/// memory. Implemented as a trait object rather than an async closure because
/// async closures do not pass cleanly through `#[async_trait]`.
#[async_trait]
pub trait LeafSink: Send {
    /// Consume one bounded batch of leaves. Returning `Err` aborts enumeration.
    async fn consume(&mut self, batch: Vec<MerkleLeaf>) -> anyhow::Result<()>;
}

/// External persistence backend for a `RecordStore`.
///
/// Provides the abstraction over write-through and write-behind strategies.
/// The [`RecordStore`](super::RecordStore) calls [`add()`](MapDataStore::add)
/// / [`remove()`](MapDataStore::remove) on every mutation. The implementation
/// decides when and how to actually persist the data.
///
/// Used as `Arc<dyn MapDataStore>`.
#[async_trait]
pub trait MapDataStore: Send + Sync {
    /// Persist a record (or queue it for async persistence).
    ///
    /// `expiration_time` is absolute millis since epoch (0 = no expiry).
    async fn add(
        &self,
        map: &str,
        key: &str,
        value: &RecordValue,
        expiration_time: i64,
        now: i64,
    ) -> anyhow::Result<()>;

    /// Persist a backup record.
    async fn add_backup(
        &self,
        map: &str,
        key: &str,
        value: &RecordValue,
        expiration_time: i64,
        now: i64,
    ) -> anyhow::Result<()>;

    /// Remove a record from the backing store (or queue the removal).
    async fn remove(&self, map: &str, key: &str, now: i64) -> anyhow::Result<()>;

    /// Remove a backup record.
    async fn remove_backup(&self, map: &str, key: &str, now: i64) -> anyhow::Result<()>;

    /// Load a single record from the backing store.
    ///
    /// Returns `None` if the key does not exist.
    async fn load(&self, map: &str, key: &str) -> anyhow::Result<Option<RecordValue>>;

    /// Load multiple records from the backing store.
    async fn load_all(
        &self,
        map: &str,
        keys: &[String],
    ) -> anyhow::Result<Vec<(String, RecordValue)>>;

    /// Stream the `(key, leaf_hash)` of every durable record of `map`, in
    /// bounded batches, WITHOUT loading full record values.
    ///
    /// This is the Merkle leaf source: it lets the sync layer rebuild a map's
    /// Merkle root from persistence alone, so a record that is persisted but
    /// not resident in memory still contributes its leaf to the root. The
    /// producer pages the durable store internally and invokes `sink` once per
    /// batch, bounding peak memory regardless of map size; only keys and `u32`
    /// hashes cross the boundary, never values.
    ///
    /// Deliberately has NO default body: every backend MUST provide a real
    /// enumeration. A default empty body would silently yield an empty Merkle
    /// root for an un-overridden backend, coupling correctness to residency.
    async fn enumerate_leaves(
        &self,
        map: &str,
        is_backup: bool,
        sink: &mut dyn LeafSink,
    ) -> anyhow::Result<()>;

    /// Begin a value-streamed scan of `map`, returning the first bounded
    /// [`ScanBatch`].
    ///
    /// This is the datastore-aware scan entrypoint consumed by the async query
    /// path: it surfaces persisted-but-non-resident records to full scans
    /// without requiring the whole map to be in memory. `max_batch_cost` caps
    /// the resident byte cost of a single batch so the scan honors the
    /// `TOPGUN_MAX_RAM_MB` ceiling; pass `0` for the backend default.
    ///
    /// Deliberately has NO default body so an un-overridden backend cannot
    /// silently scan only the resident subset.
    async fn scan_values(
        &self,
        map: &str,
        is_backup: bool,
        max_batch_cost: u64,
    ) -> anyhow::Result<ScanBatch>;

    /// Fetch the next bounded [`ScanBatch`] for an in-progress value-streamed
    /// scan, resuming from `cursor`.
    ///
    /// Each call loads at most `max_batch_cost` bytes of records (pass `0` for
    /// the backend default), keeping the scan within the `TOPGUN_MAX_RAM_MB`
    /// ceiling. Enumeration is exhausted when the returned batch carries
    /// `next_cursor == None`.
    ///
    /// Deliberately has NO default body for the same residency-correctness
    /// reason as [`scan_values`](MapDataStore::scan_values).
    async fn scan_values_batched(
        &self,
        map: &str,
        is_backup: bool,
        cursor: ScanCursor,
        max_batch_cost: u64,
    ) -> anyhow::Result<ScanBatch>;

    /// Remove all specified keys from the backing store.
    async fn remove_all(&self, map: &str, keys: &[String]) -> anyhow::Result<()>;

    /// List the names of every map that has durable (primary) records in this
    /// backend, regardless of whether any of those records are currently
    /// resident in memory.
    ///
    /// This is the residency-independent map source the startup Merkle-index
    /// seed iterates: it lets the server rebuild each persisted map's Merkle
    /// root from durable keys+hashes alone, so a map that survived a restart but
    /// has not yet been touched in memory still answers `SYNC_INIT` with the
    /// correct (non-zero, pre-crash-equal) root. Only primary partitions are
    /// listed — backup partitions are not part of the `SYNC_INIT` root path.
    ///
    /// Unlike [`enumerate_leaves`](MapDataStore::enumerate_leaves), this has a
    /// default body returning an empty list: a backend that holds no durable
    /// maps (the null / in-memory test stores) correctly contributes nothing to
    /// seed, and the durable backends (redb / Postgres / write-behind) override
    /// it. An empty default here cannot couple correctness to residency the way
    /// an empty `enumerate_leaves` would — at worst the seed is a no-op and the
    /// map's trees stay empty (the pre-existing behavior), never wrong leaves.
    async fn list_maps(&self) -> anyhow::Result<Vec<String>> {
        Ok(Vec::new())
    }

    /// Check if a key is safe to load (not queued for write-behind).
    ///
    /// For write-through implementations, always returns `true`.
    fn is_loadable(&self, key: &str) -> bool;

    /// Number of pending (not yet flushed) operations.
    ///
    /// For write-through, always returns 0.
    fn pending_operation_count(&self) -> u64;

    /// Mark the store as flushable. Actual flushing happens on a background task.
    ///
    /// Returns the sequence number of the last queued operation, or 0 if empty.
    async fn soft_flush(&self) -> anyhow::Result<u64>;

    /// Flush all pending writes immediately in the calling task.
    ///
    /// Called during node shutdown for data safety.
    async fn hard_flush(&self) -> anyhow::Result<()>;

    /// Flush a single key immediately (used during eviction).
    async fn flush_key(
        &self,
        map: &str,
        key: &str,
        value: &RecordValue,
        is_backup: bool,
    ) -> anyhow::Result<()>;

    /// Reset the data store to initial state (clear queues, etc.).
    fn reset(&self);

    /// Whether this is a null (no-op) implementation.
    ///
    /// Returns `false` by default. Null implementations override to return `true`.
    fn is_null(&self) -> bool {
        false
    }
}

/// A point-in-time snapshot of a map's coordinate-trie, held entirely in
/// memory after a single enumeration pass.
///
/// Methods on `MerkleSession` answer root, internal-node bucket, and leaf-key
/// queries from the already-materialised trie — no additional storage round
/// trips are needed per call. This avoids re-enumeration when a sync peer
/// drills down through multiple trie levels in one session.
///
/// Holds two independent trie views (LWW and OR-Map) mirroring the dual-tree
/// structure the write-path observer maintains in `MerkleSyncManager`. The
/// `root()` method returns the cross-kind combined root so a single hash is
/// sufficient for the `SYNC_INIT` comparison; `buckets()` and `leaf_keys()`
/// merge both trees at the requested path.
///
/// Created by [`DurableMerkleIndex::build_session`]; the caller is responsible
/// for deciding when to discard the snapshot (e.g. after the sync round-trip
/// completes or a write invalidates the root).
pub struct MerkleSession {
    /// Pre-computed per-path aggregate bucket hashes for the LWW tree.
    /// `""` maps to the root-level children; each child path maps to its own
    /// children, following the same BFS expansion the write-path observer uses.
    pub(crate) lww_nodes: HashMap<String, HashMap<char, u32>>,
    /// Pre-computed per-path aggregate bucket hashes for the OR-Map tree.
    pub(crate) ormap_nodes: HashMap<String, HashMap<char, u32>>,
    /// Aggregate LWW root hash (mirrors `MerkleSyncManager::aggregate_lww_root_hash`).
    pub(crate) lww_root: u32,
    /// Aggregate OR-Map root hash (mirrors `MerkleSyncManager::aggregate_ormap_root_hash`).
    pub(crate) ormap_root: u32,
    /// Leaf key membership by hex-path prefix (depth-length) for `leaf_keys` queries.
    /// Key: hex-path of length `tree_depth`; value: record keys hashing to that path.
    pub(crate) leaf_keys_by_path: HashMap<String, Vec<String>>,
}

impl MerkleSession {
    /// Return the aggregate root hash for the map.
    ///
    /// Combines the LWW and OR-Map aggregate roots with the same
    /// `combine_hashes` function used across all partition aggregation sites,
    /// producing a single hash byte-compatible with the live in-memory root.
    #[must_use]
    pub fn root(&self) -> u32 {
        topgun_core::hash::combine_hashes(&[self.lww_root, self.ormap_root])
    }

    /// Return the per-hex-bucket child hashes for the internal trie node at
    /// `path`.
    ///
    /// `path` encodes the route from the root to this node as a sequence of
    /// hex nibble characters, following the same convention as
    /// `aggregate_lww_buckets` / `get_buckets` in `merkle_sync.rs` (e.g.
    /// `""` = root level, `"a"` = bucket `'a'` under root, `"a3"` = sub-bucket
    /// `'3'` under `'a'`). Returns a merged view of LWW + OR-Map children.
    /// Returns an empty map if the path does not exist in the snapshot.
    #[must_use]
    pub fn buckets(&self, path: &str) -> HashMap<char, u32> {
        let lww = self.lww_nodes.get(path).cloned().unwrap_or_default();
        let ormap = self.ormap_nodes.get(path).cloned().unwrap_or_default();
        // Merge: for chars that appear in both trees, combine their hashes.
        let mut merged: HashMap<char, Vec<u32>> = HashMap::new();
        for (c, h) in lww {
            merged.entry(c).or_default().push(h);
        }
        for (c, h) in ormap {
            merged.entry(c).or_default().push(h);
        }
        merged
            .into_iter()
            .map(|(c, hs)| (c, topgun_core::hash::combine_hashes(&hs)))
            .collect()
    }

    /// Return the LWW aggregate root hash only.
    ///
    /// Useful for tests that need to compare against
    /// `MerkleSyncManager::aggregate_lww_root_hash` independently.
    #[must_use]
    pub fn lww_root(&self) -> u32 {
        self.lww_root
    }

    /// Return the OR-Map aggregate root hash only.
    ///
    /// Useful for tests that need to compare against
    /// `MerkleSyncManager::aggregate_ormap_root_hash` independently.
    #[must_use]
    pub fn ormap_root(&self) -> u32 {
        self.ormap_root
    }

    /// Return the record keys that are leaves under `path` in the trie.
    ///
    /// Used by the sync peer to confirm leaf-level membership without loading
    /// full record values. Returns an empty vec if the path has no leaves in
    /// the snapshot.
    #[must_use]
    pub fn leaf_keys(&self, path: &str) -> Vec<String> {
        self.leaf_keys_by_path
            .get(path)
            .cloned()
            .unwrap_or_default()
    }
}

/// Residency-independent Merkle index surface.
///
/// Implementations build a [`MerkleSession`] by enumerating durable leaves
/// from `store` for the given `map` (via [`MapDataStore::enumerate_leaves`]),
/// folding them into a coordinate-trie, and returning the opaque handle.
/// The caller drives the drill-down through `MerkleSession` methods without
/// touching storage again for that session.
///
/// Decoupling the snapshot build from the drill-down queries means the sync
/// handler can serve multiple `SYNC_STEP` messages from one enumeration pass,
/// and records that are persisted but not in-memory still contribute their
/// leaf hashes to the root — fixing the residency-coupling defect (TODO-530).
pub trait DurableMerkleIndex {
    /// Enumerate all durable leaves for `map` from `store` and materialise a
    /// point-in-time coordinate-trie snapshot as a [`MerkleSession`] handle.
    ///
    /// Callers should hold the returned session for the duration of one sync
    /// round-trip, then drop it. A new session should be built after any write
    /// to `map` to keep the snapshot consistent with the durable state.
    ///
    /// # Errors
    ///
    /// Returns any error surfaced by [`MapDataStore::enumerate_leaves`]. A
    /// failed or partial enumeration MUST NOT silently yield a session built
    /// over an incomplete leaf set — that would produce a wrong (yet
    /// plausible) root and let the sync handler answer with leaves that diverge
    /// from durable truth. The contract mirrors
    /// [`MerkleSyncManager::rebuild_from_datastore`](crate::storage::merkle_sync::MerkleSyncManager::rebuild_from_datastore):
    /// propagate, never degrade to wrong leaves.
    fn build_session(&self, map: &str, store: &dyn MapDataStore) -> anyhow::Result<MerkleSession>;
}
