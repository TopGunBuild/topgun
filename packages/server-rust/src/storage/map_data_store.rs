//! External persistence backend trait for the storage layer.
//!
//! Defines [`MapDataStore`], the Layer 3 abstraction over write-through and
//! write-behind persistence strategies. The [`RecordStore`](super::RecordStore)
//! calls `add()` (directly, or as the default body of `add_with_witness()`) /
//! `remove()` on every mutation; the implementation decides when and how to
//! actually persist the data.
//!
//! Also defines [`DurableMerkleIndex`] and [`MerkleSession`]: a residency-
//! independent Merkle computation surface that materialises the full coordinate-
//! trie once from durable storage and serves repeated drill-down calls from that
//! snapshot, so no re-enumeration is needed per query.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;

use super::engine::SlotCell;
use super::record::RecordValue;
use super::wal::OrDelta;
use topgun_core::hash::{fnv1a_hash, fnv1a_update, FNV1A_OFFSET_BASIS};

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
///
/// Presence rule (TG-MRK-001): an OR slot contributes a leaf only while it
/// holds at least one live tag or one tombstone. A slot with neither returns
/// `None`, because a peer that never saw the key — or pruned it once it emptied
/// — carries no leaf for it, and a leaf on this side alone would keep the two
/// roots apart forever. Every non-empty slot hashes exactly as before.
///
/// Returns `None` for `OrTombstones`: the write path removes such keys from the
/// OR-Map tree rather than contributing a leaf, so enumeration must likewise
/// emit no leaf to keep a rebuilt root identical to the live one.
///
/// Known gap (tracked by TODO-559): because a legacy `OrTombstones` slot has no
/// leaf, the equal-roots guarantee of TG-MRK-002 does not extend to its tags —
/// two sides can report equal roots while only one of them holds those tags.
/// This is harmless today because such tags are never epoch-stamped, so no
/// covering epoch is ever confirmed on their behalf.
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
            // No live tag and no tombstone means the key is absent from every
            // peer's tree, so it must be absent from this one too (TG-MRK-001
            // presence rule).
            if records.is_empty() && tombstones.is_empty() {
                return None;
            }
            // Streams `"key:{key}|{tags joined by |}#{tombs joined by |}"` into the
            // hash part by part instead of building it: the OR arm runs on every OR
            // write, and the joined and formatted strings were whole-slot copies.
            // Bit-identical by construction — FNV-1a is a left fold and each part
            // is whole code points (TG-MRK-001: a rebuilt root must equal the live
            // one). The sort stays: the leaf must not depend on slot order.
            let mut tags: Vec<&str> = records.iter().map(|r| r.tag.as_str()).collect();
            tags.sort_unstable();
            let mut tomb_tags: Vec<&str> = tombstones.iter().map(String::as_str).collect();
            tomb_tags.sort_unstable();
            let mut hash = fnv1a_update(FNV1A_OFFSET_BASIS, "key:");
            hash = fnv1a_update(hash, key);
            hash = fnv1a_update(hash, "|");
            hash = fnv1a_update_joined(hash, &tags);
            hash = fnv1a_update(hash, "#");
            hash = fnv1a_update_joined(hash, &tomb_tags);
            Some((MerkleLeafKind::OrMap, hash))
        }
        RecordValue::OrTombstones { .. } => None,
    }
}

/// Feeds `parts` separated by `"|"` into a running FNV-1a state — the hash of
/// `parts.join("|")` without allocating the joined string.
fn fnv1a_update_joined(mut hash: u32, parts: &[&str]) -> u32 {
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            hash = fnv1a_update(hash, "|");
        }
        hash = fnv1a_update(hash, part);
    }
    hash
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
/// / [`remove()`](MapDataStore::remove) on every mutation — with one indirection
/// worth naming, because it is the doc an implementor reads before concluding
/// it need not override anything: the in-place write path calls
/// [`add_with_witness()`](MapDataStore::add_with_witness), whose default body
/// delegates straight to [`add()`](MapDataStore::add) and drops the witness. A
/// store that overrides neither that method nor
/// [`wants_or_witness()`](MapDataStore::wants_or_witness) therefore still sees
/// every mutation arrive through [`add()`](MapDataStore::add), unchanged. The
/// implementation decides when and how to actually persist the data.
///
/// The record a write-through persists: a borrowed value, or the cell the
/// engine mutated in place.
///
/// A store that buffers writes can hold the cell instead of a copy of its
/// value; any other store reads the value under the cell's lock.
#[derive(Clone, Copy)]
pub enum WriteSource<'a> {
    /// A value owned by the caller.
    Value(&'a RecordValue),
    /// The engine's cell for the key, holding the record just written.
    Cell(&'a Arc<SlotCell>),
}

impl WriteSource<'_> {
    /// Runs `f` over the value, under the cell's lock for a cell. The guard
    /// never escapes `f`, so it cannot be held across an `.await`.
    pub fn with_value<R>(&self, f: impl FnOnce(&RecordValue) -> R) -> R {
        match self {
            WriteSource::Value(value) => f(value),
            WriteSource::Cell(cell) => f(&cell.lock().value),
        }
    }

    /// An owned copy of the value.
    #[must_use]
    pub fn to_value(&self) -> RecordValue {
        self.with_value(RecordValue::clone)
    }
}

/// A key's state as the data store answers it for materialization: a decoded
/// value, or the cell a pending write of the key still holds.
#[derive(Debug)]
pub enum Loaded {
    /// A value read (or copied) from the store.
    Value(RecordValue),
    /// The cell of the key's pending write.
    Cell(Arc<SlotCell>),
}

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

    /// Persist a record together with the delta that produced it.
    ///
    /// `expiration_time` is absolute millis since epoch (0 = no expiry).
    /// `witness` is the OR delta that was just applied to `value`, so the two
    /// always describe the same op: a store that records deltas durably gets
    /// the one the mutation point already built and never has to re-derive or
    /// diff it. Callers with no per-op delta — every LWW write, bulk and SYNC
    /// ingestion, cross-node merge, rehydration — pass `None`, and so does
    /// every caller whenever [`wants_or_witness()`](MapDataStore::wants_or_witness)
    /// answered `false`.
    ///
    /// `src` is the record: a borrowed value, or the engine's cell for the key
    /// (see [`WriteSource`]).
    ///
    /// The default body delegates to [`add()`](MapDataStore::add) and IGNORES
    /// the witness, so a backend that consumes no deltas persists exactly what
    /// it persists without this method and needs no override. A cell is read
    /// by copying its value out under the lock, so the lock is never held
    /// across the `add()` await.
    async fn add_with_witness(
        &self,
        map: &str,
        key: &str,
        src: WriteSource<'_>,
        expiration_time: i64,
        now: i64,
        witness: Option<&OrDelta>,
    ) -> anyhow::Result<()> {
        // Dropped rather than forwarded: a store that has not asked for a
        // witness has nowhere to put one, and the full value below is a
        // complete record of the mutation on its own.
        let _ = witness;
        match src {
            WriteSource::Value(value) => self.add(map, key, value, expiration_time, now).await,
            WriteSource::Cell(cell) => {
                let value = cell.lock().value.clone();
                self.add(map, key, &value, expiration_time, now).await
            }
        }
    }

    /// Whether [`add_encoded()`](MapDataStore::add_encoded) persists a record
    /// from its msgpack encoding. A store answering `true` lets a buffering
    /// caller encode a record under its cell lock instead of copying it out.
    /// Defaulted to `false`.
    fn accepts_encoded(&self) -> bool {
        false
    }

    /// Persist a record from its msgpack encoding (`rmp_serde::to_vec_named` of
    /// the `RecordValue`), storing exactly what [`add()`](MapDataStore::add)
    /// stores for that value. Called only when
    /// [`accepts_encoded()`](MapDataStore::accepts_encoded) is `true`; the
    /// default body refuses.
    async fn add_encoded(
        &self,
        map: &str,
        key: &str,
        bytes: &[u8],
        expiration_time: i64,
        now: i64,
    ) -> anyhow::Result<()> {
        let _ = (bytes, expiration_time, now);
        anyhow::bail!("this store does not accept encoded records (map={map} key={key})")
    }

    /// Whether this store consumes the `witness` handed to
    /// [`add_with_witness()`](MapDataStore::add_with_witness).
    ///
    /// Read by the write path before it builds a witness, so a store that
    /// ignores deltas costs the mutation path nothing at all rather than a
    /// discarded copy. Defaulted to `false`: only a backend that actually
    /// records deltas overrides it.
    fn wants_or_witness(&self) -> bool {
        false
    }

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

    /// Load a single key for materialization into the engine. A store that
    /// buffers writes may answer with the cell its pending write holds, so the
    /// engine can adopt it; the default body wraps [`load()`](MapDataStore::load).
    async fn load_slot(&self, map: &str, key: &str) -> anyhow::Result<Option<Loaded>> {
        Ok(self.load(map, key).await?.map(Loaded::Value))
    }

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

    /// The highest write-ordering sequence this store has ASSIGNED so far
    /// (one past the last handed-out value), regardless of whether it has been
    /// flushed to durable storage. This is a cheap synchronous snapshot — the
    /// same value [`soft_flush`](MapDataStore::soft_flush) returns, but without
    /// triggering a flush — used by the tombstone frontier to stamp an
    /// upper-bound sequence onto a genuinely-new tombstone at write time (the
    /// tombstone's own byte-write was enqueued strictly before this is read, so
    /// the returned value is `>=` the tombstone's sequence).
    ///
    /// The default returns 0, correct for write-through backends that never
    /// buffer (their writes are durable on return, so there is nothing to fence).
    fn assigned_write_sequence(&self) -> u64 {
        0
    }

    /// The PREFIX-COMPLETE byte-durability watermark: the largest `W` such that
    /// EVERY write with an assigned sequence `< W` is durable in the inner store
    /// (flushed) or has been superseded by a later coalesced write (retired).
    ///
    /// Prefix-completeness is the load-bearing property: the returned value NEVER
    /// sits above an un-flushed sequence (no mid-range hole), so a consumer that
    /// gates on `stamped_seq <= flushed_watermark()` can be certain the stamped
    /// write's bytes are durable — never a max-with-holes value that would admit
    /// a `kill -9` tombstone resurrection. It advances ONLY on real inner-store
    /// byte durability, never on WAL-fsync alone and never from the last-assigned
    /// sequence.
    ///
    /// The default returns 0, which keeps every tombstone byte-undurable from the
    /// fence's perspective (the safe direction — no prune is ever licensed) for
    /// write-through backends and test doubles that do not buffer writes.
    fn flushed_watermark(&self) -> u64 {
        0
    }

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
/// Holds two independent trie views, one per CRDT kind (LWW and OR-Map). Each
/// is ONE flat trie over every key of that kind in the map — depth 3, addressed
/// by the lowercase hex of `fnv1a(key)`, whichever partition the key routes to
/// — which is the shape a client keeps, so the roots and buckets reported here
/// are directly comparable with the client's (TG-MRK-002). The per-partition
/// trees `MerkleSyncManager` maintains on the write path are not visible
/// through a session. The `root()` method returns the cross-kind combined root;
/// `buckets()` and `leaf_keys()` merge both tries at the requested path.
///
/// Created by [`DurableMerkleIndex::build_session`]; the caller is responsible
/// for deciding when to discard the snapshot (e.g. after the sync round-trip
/// completes or a write invalidates the root).
///
/// # Consistency contract: pins structure, not values
///
/// A session pins ONLY the trie STRUCTURE captured at build time:
/// - the per-path bucket hashes of each flat trie (`lww_nodes` / `ormap_nodes`),
/// - the leaf-KEY membership (`leaf_keys_by_path`), and
/// - the flat-trie roots (`lww_root` / `ormap_root`).
///
/// It deliberately does NOT pin per-leaf record VALUES. During a sync drill-down
/// the leaf-serving handlers fetch each record's bytes LIVE from the store
/// (a lazy `store.get` keyed by the pinned leaf key), so the value a peer
/// receives is always the newest durable value, not the one that contributed to
/// the pinned bucket hash. Holding only key membership keeps the snapshot's
/// memory cost proportional to the key set rather than the full value set.
///
/// This means a write landing BETWEEN the session build and a leaf fetch yields
/// a wire-level torn read: the bucket hash the peer verifies was folded from the
/// OLD leaf, while the served leaf carries the NEWER live value. The guarantee
/// THIS side provides is narrow and local: the leaf-serving handlers always hand
/// back the live durable value for a pinned key, never the stale value that fed
/// the pinned bucket hash.
///
/// That torn read is self-healing under the reconnect protocol the peer runs: a
/// peer uses the bucket hash only as a drill-down TRIGGER — it descends into the
/// subtree on a hash mismatch but never commits the hash as authoritative state —
/// and folds the served leaf in monotonically (LWW by timestamp; OR-Map by
/// tag/tombstone CRDT merge). Because the served bytes are the live store value,
/// the peer never commits a value the store does not hold, and any residual root
/// divergence is resolved on the next root compare. This convergence is a
/// property of that protocol, NOT an invariant enforced by this struct — a peer
/// that treated the bucket hash as authoritative state would not get it.
///
/// Flat-shape corollary: because the trie has the client's shape, a pinned
/// bucket can MATCH the peer's, and a match lets the peer skip that bucket's
/// whole subtree. A key written after the build under a matching bucket is
/// therefore not served this round either. Whoever conveys a covering epoch
/// alongside a round served from this session must bound that epoch to what
/// the snapshot can vouch for (TG-MRK-002): an epoch newer than the snapshot
/// would claim delivery of a change the peer was just allowed to skip.
///
/// One bounded-staleness corollary: a key created AFTER the build is absent from
/// the pinned `leaf_keys_by_path` and is not served this round; the peer learns
/// it on the next `SYNC_INIT`, whose fresh snapshot includes it. That is delayed
/// visibility, not divergence. Pinning per-leaf values here would buy nothing for
/// correctness while inflating the snapshot to the full value set.
pub struct MerkleSession {
    /// Pre-computed per-path bucket hashes of the flat LWW trie.
    /// `""` maps to the root-level children; each child path maps to its own
    /// children, for every internal node of the trie. A child absent from a
    /// map is absent from the trie: no entry carries a zero hash.
    pub(crate) lww_nodes: HashMap<String, HashMap<char, u32>>,
    /// Pre-computed per-path bucket hashes of the flat OR-Map trie, with the
    /// same layout as `lww_nodes`.
    pub(crate) ormap_nodes: HashMap<String, HashMap<char, u32>>,
    /// Root of the flat LWW trie over every LWW key of the map.
    pub(crate) lww_root: u32,
    /// Root of the flat OR-Map trie over every OR-Map key of the map.
    pub(crate) ormap_root: u32,
    /// Leaf key membership by hex-path prefix (depth-length) for `leaf_keys` queries.
    /// Key: hex-path of length `tree_depth`; value: record keys hashing to that path.
    pub(crate) leaf_keys_by_path: HashMap<String, Vec<String>>,
}

impl MerkleSession {
    /// Return the cross-kind root hash for the map.
    ///
    /// Folds the flat LWW root and the flat OR-Map root with `combine_hashes`
    /// into a single hash. Clients compare per kind, against `lww_root()` or
    /// `ormap_root()`; this combined value is a server-side summary of both.
    #[must_use]
    pub fn root(&self) -> u32 {
        topgun_core::hash::combine_hashes(&[self.lww_root, self.ormap_root])
    }

    /// Return the per-hex-bucket child hashes for the internal trie node at
    /// `path`.
    ///
    /// `path` encodes the route from the root to this node as a sequence of
    /// hex nibble characters, following the same convention as the core
    /// `MerkleTree::get_buckets` (e.g. `""` = root level, `"a"` = bucket `'a'`
    /// under root, `"a3"` = sub-bucket `'3'` under `'a'`). Returns a merged
    /// view of the two flat tries' children: a hex digit present in both kinds
    /// carries the `combine_hashes` fold of the two child hashes.
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

    /// Return the root of the flat LWW trie only.
    ///
    /// This is the value a client's LWW trie root is compared against
    /// (TG-MRK-002).
    #[must_use]
    pub fn lww_root(&self) -> u32 {
        self.lww_root
    }

    /// Return the root of the flat OR-Map trie only.
    ///
    /// This is the value a client's OR-Map trie root is compared against
    /// (TG-MRK-002).
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

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet, HashMap};
    use std::path::PathBuf;

    use proptest::prelude::*;
    use serde::{Deserialize, Serialize};
    use topgun_core::hash::fnv1a_hash;
    use topgun_core::hlc::Timestamp;
    use topgun_core::merkle::{MerkleTree, ORMapMerkleTree};
    use topgun_core::partition::hash_to_partition;
    use topgun_core::types::Value;

    use super::{merkle_leaf_hash, MerkleLeafKind};
    use crate::storage::record::{OrMapEntry, RecordValue};

    /// The OR-Map leaf formula exactly as it was first written — joined tag sets
    /// fed to one `format!` — kept verbatim as the oracle any cheaper rewrite of
    /// `merkle_leaf_hash`'s OR arm must reproduce bit for bit (TG-MRK-001: a
    /// rebuilt root must equal the live one, so the formula can never drift).
    fn oracle_or_leaf_hash(
        key: &str,
        records: &[OrMapEntry],
        tombstones: &[String],
    ) -> Option<u32> {
        // An OR slot with no live tags and no tombstones has no leaf (TG-MRK-001 presence rule).
        if records.is_empty() && tombstones.is_empty() {
            return None;
        }
        let mut tags: Vec<&str> = records.iter().map(|r| r.tag.as_str()).collect();
        tags.sort_unstable();
        let joined = tags.join("|");
        let mut tomb_tags: Vec<&str> = tombstones.iter().map(String::as_str).collect();
        tomb_tags.sort_unstable();
        let joined_tombs = tomb_tags.join("|");
        Some(fnv1a_hash(&format!("key:{key}|{joined}#{joined_tombs}")))
    }

    fn entry(tag: String) -> OrMapEntry {
        OrMapEntry {
            value: Value::Null,
            tag,
            timestamp: Timestamp {
                millis: 1_700_000_000_000,
                counter: 0,
                node_id: "node-a".to_string(),
            },
        }
    }

    fn or_hash(key: &str, records: Vec<OrMapEntry>, tombstones: Vec<String>) -> Option<u32> {
        match merkle_leaf_hash(
            key,
            &RecordValue::OrMap {
                records,
                tombstones,
            },
        ) {
            Some((MerkleLeafKind::OrMap, h)) => Some(h),
            None => None,
            Some(other) => {
                panic!("an OrMap value must never yield a non-OrMap leaf, got {other:?}")
            }
        }
    }

    /// Tags drawn from two pools: a wide one (the separators `|` and `#`, a
    /// non-ASCII BMP code point, an astral one) and a tiny one, so duplicate tags
    /// and separator-only tags occur often rather than by luck.
    fn tag_strategy() -> impl Strategy<Value = String> {
        prop_oneof![
            "[a-z0-9:|#\u{e9}\u{3a9}\u{1F600}\u{10FFFF}]{0,8}",
            "[a|#\u{1F600}]{0,2}",
        ]
    }

    fn set_and_shuffle() -> impl Strategy<Value = (Vec<String>, Vec<String>)> {
        prop::collection::vec(tag_strategy(), 0..12)
            .prop_flat_map(|v| (Just(v.clone()), Just(v).prop_shuffle()))
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]

        /// The OR leaf hash equals the oracle formula for every input, including
        /// empty tag and/or tombstone sets (no leaf when both are empty),
        /// separator characters inside tags,
        /// duplicate tags and multi-byte code points; and it is independent of
        /// the input order of `records` and of `tombstones` (TG-MRK-001).
        #[test]
        fn or_leaf_hash_matches_oracle_and_ignores_input_order(
            key in "[a-z|#\u{e9}\u{1F600}]{0,6}",
            (tags, tags_shuffled) in set_and_shuffle(),
            (tombs, tombs_shuffled) in set_and_shuffle(),
        ) {
            let records: Vec<OrMapEntry> = tags.iter().cloned().map(entry).collect();
            let expected = oracle_or_leaf_hash(&key, &records, &tombs);
            let hash = or_hash(&key, records, tombs);
            prop_assert_eq!(hash, expected, "leaf hash drifted from the formula");

            let shuffled: Vec<OrMapEntry> = tags_shuffled.into_iter().map(entry).collect();
            prop_assert_eq!(
                or_hash(&key, shuffled, tombs_shuffled),
                hash,
                "leaf hash depends on input order"
            );
        }
    }

    /// An OR slot with no live tags and no tombstones has no leaf (TG-MRK-001
    /// presence rule): a client drops such a key from its tree, so a server
    /// leaf for it would keep the two roots apart for every map that ever
    /// emptied and pruned a key.
    #[test]
    fn empty_or_slot_yields_no_leaf() {
        let empty = RecordValue::OrMap {
            records: Vec::new(),
            tombstones: Vec::new(),
        };
        assert_eq!(
            merkle_leaf_hash("k", &empty),
            None,
            "an OR slot with no live tags and no tombstones must not contribute a leaf"
        );
    }

    fn tags(items: &[&str]) -> Vec<String> {
        items.iter().map(|tag| (*tag).to_string()).collect()
    }

    /// The leaf of key `"k"` for the given live and tombstone tags.
    fn or_leaf_of(live: &[&str], tombstones: &[&str]) -> Option<u32> {
        or_hash(
            "k",
            tags(live).into_iter().map(entry).collect(),
            tags(tombstones),
        )
    }

    /// KNOWN LIMITATION (TG-MRK-001): tags are hashed verbatim and joined by
    /// `|`, so one live tag containing `|` is indistinguishable from the two
    /// tags it splits into. Equal leaves for different tag sets are possible
    /// only when a tag carries a separator character; nothing rejects such a
    /// tag today, so this pins the collision as it stands instead of leaving it
    /// to be rediscovered. The literal is the same one the TypeScript client
    /// pins for the same inputs.
    #[test]
    fn or_leaf_tag_containing_pipe_collides_with_the_split_tags() {
        let joined = or_leaf_of(&["a|b"], &[]);
        let split = or_leaf_of(&["a", "b"], &[]);

        assert_eq!(
            joined, split,
            "a tag containing `|` hashes like the tags it splits into"
        );
        assert_eq!(joined, Some(473_285_503));
    }

    /// KNOWN LIMITATION (TG-MRK-001): `#` separates the live tags from the
    /// tombstones, so a `#` inside a tag can move where that boundary appears
    /// to be: live `a#b` with tombstone `c` hashes like live `a` with tombstone
    /// `b#c`, although one side holds `a#b` live and the other never saw it.
    ///
    /// A set whose tags are all free of `#` hashes a string with exactly one
    /// `#`, and a tag containing `#` adds another, so this collision needs a
    /// `#` in a tag on BOTH sides — it cannot be produced against a peer whose
    /// tags are clean. The second half pins that: live `a#b` with no tombstone
    /// does not collide with live `a` plus tombstone `b`.
    #[test]
    fn or_leaf_tag_containing_hash_collides_with_hash_in_a_tombstone() {
        let hash_in_live = or_leaf_of(&["a#b"], &["c"]);
        let hash_in_tombstone = or_leaf_of(&["a"], &["b#c"]);

        assert_eq!(
            hash_in_live, hash_in_tombstone,
            "a `#` inside a tag moves the live/tombstone boundary"
        );
        assert_eq!(hash_in_live, Some(2_457_121_691));

        let hash_tag_alone = or_leaf_of(&["a#b"], &[]);
        let clean_tag_and_tombstone = or_leaf_of(&["a"], &["b"]);

        assert_ne!(
            hash_tag_alone, clean_tag_and_tombstone,
            "a tag containing `#` must not collide with separator-free tags"
        );
        assert_eq!(hash_tag_alone, Some(902_949_562));
        assert_eq!(clean_tag_and_tombstone, Some(3_821_700_797));
    }

    /// Two orderings the client must reproduce exactly (TG-MRK-001), pinned as
    /// literals shared with the TypeScript suite: a tag that is a prefix of
    /// another sorts first, and after a shared prefix an astral character sorts
    /// AFTER a high BMP one. The second is where code-point order (UTF-8 bytes,
    /// used here) and UTF-16 code-unit order (a naive client sort) disagree, so
    /// each case also shows that the opposite order hashes differently — else
    /// the literal would not distinguish the two sorts.
    #[test]
    fn or_leaf_sort_cases_prefix_pair_and_shared_prefix_then_astral() {
        let prefix_pair = or_leaf_of(&["ab", "abc"], &[]);
        assert_eq!(prefix_pair, or_leaf_of(&["abc", "ab"], &[]));
        assert_eq!(prefix_pair, Some(1_482_261_199));
        assert_eq!(prefix_pair, Some(fnv1a_hash("key:k|ab|abc#")));
        assert_ne!(prefix_pair, Some(fnv1a_hash("key:k|abc|ab#")));

        let astral = "p\u{10000}";
        let high_bmp = "p\u{FF61}";
        let astral_pair = or_leaf_of(&[astral, high_bmp], &[]);
        assert_eq!(astral_pair, or_leaf_of(&[high_bmp, astral], &[]));
        assert_eq!(astral_pair, Some(2_938_983_567));
        assert_eq!(
            astral_pair,
            Some(fnv1a_hash("key:k|p\u{FF61}|p\u{10000}#")),
            "the high BMP tag sorts before the astral one"
        );
        assert_ne!(
            astral_pair,
            Some(fnv1a_hash("key:k|p\u{10000}|p\u{FF61}#")),
            "UTF-16 code-unit order would put the astral tag first"
        );
    }

    /// The cross-language golden vector file (TG-MRK-001). Rust is the canonical
    /// producer of every derived field; the TypeScript suite asserts against the
    /// same file, so the two implementations cannot drift apart silently.
    ///
    /// Field order is the on-disk order, and unknown fields are rejected so a
    /// regeneration can never silently drop an input it does not model.
    #[derive(Debug, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct VectorFile {
        version: u32,
        or_leaf: Vec<OrLeafCase>,
        lww_leaf: Vec<LwwLeafCase>,
        flat_trie: Vec<FlatTrieCase>,
    }

    #[derive(Debug, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct OrLeafCase {
        name: String,
        key: String,
        records: Vec<OrRecordCase>,
        tombstones: Vec<String>,
        /// `None` (JSON `null`) means the slot has no leaf.
        expected: Option<u32>,
    }

    #[derive(Debug, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct OrRecordCase {
        tag: String,
        value: serde_json::Value,
        /// Carried only so the TypeScript side can prove TTL never reaches the
        /// leaf; the server stores no TTL on an OR entry.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ttl_ms: Option<u64>,
    }

    #[derive(Debug, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct LwwLeafCase {
        name: String,
        key: String,
        millis: u64,
        counter: u32,
        node_id: String,
        expected: u32,
    }

    #[derive(Debug, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct FlatTrieCase {
        name: String,
        kind: String,
        leaves: Vec<FlatTrieLeaf>,
        expected_root: u32,
        /// Path (`""` and every depth-1 path present) to child hashes. Sorted
        /// maps keep the regenerated file stable and its diffs readable.
        expected_buckets: BTreeMap<String, BTreeMap<String, u32>>,
    }

    #[derive(Debug, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct FlatTrieLeaf {
        key: String,
        leaf_hash: u32,
        /// Recorded so the partition spread of a case can be checked by reading
        /// the file alone.
        partition: u32,
    }

    const REGEN_ENV: &str = "TOPGUN_REGEN_MERKLE_VECTORS";

    fn vectors_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../core-rust/tests/fixtures/merkle_vectors.json")
    }

    fn load_vectors() -> VectorFile {
        let path = vectors_path();
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        let file: VectorFile = serde_json::from_str(&raw)
            .unwrap_or_else(|e| panic!("cannot parse {}: {e}", path.display()));
        assert_eq!(file.version, 1, "unknown merkle vector schema version");
        file
    }

    fn regen_requested() -> bool {
        std::env::var(REGEN_ENV).as_deref() == Ok("1")
    }

    /// Rewrites the derived fields of one section. The file is re-read right
    /// here, not reused from the start of the test, because the three pin tests
    /// share it: a stale copy would undo another section's regeneration.
    fn rewrite_vectors(apply: impl FnOnce(&mut VectorFile)) {
        let mut file = load_vectors();
        apply(&mut file);
        let mut out = serde_json::to_string_pretty(&file).expect("vector file serializes");
        out.push('\n');
        let path = vectors_path();
        std::fs::write(&path, out)
            .unwrap_or_else(|e| panic!("cannot write {}: {e}", path.display()));
    }

    /// The value never reaches the leaf, but the slot is built with the case's
    /// real value so the pin exercises the same shape of record the server holds.
    fn value_from_json(json: &serde_json::Value) -> Value {
        match json {
            serde_json::Value::Null => Value::Null,
            serde_json::Value::Bool(b) => Value::Bool(*b),
            serde_json::Value::Number(n) => match n.as_i64() {
                Some(i) => Value::Int(i),
                None => Value::Float(n.as_f64().unwrap_or(f64::NAN)),
            },
            serde_json::Value::String(s) => Value::String(s.clone()),
            serde_json::Value::Array(items) => {
                Value::Array(items.iter().map(value_from_json).collect())
            }
            serde_json::Value::Object(fields) => Value::Map(
                fields
                    .iter()
                    .map(|(k, v)| (k.clone(), value_from_json(v)))
                    .collect(),
            ),
        }
    }

    fn or_case_leaf(case: &OrLeafCase) -> Option<u32> {
        let records = case
            .records
            .iter()
            .map(|r| OrMapEntry {
                value: value_from_json(&r.value),
                ..entry(r.tag.clone())
            })
            .collect();
        let slot = RecordValue::OrMap {
            records,
            tombstones: case.tombstones.clone(),
        };
        match merkle_leaf_hash(&case.key, &slot) {
            Some((MerkleLeafKind::OrMap, hash)) => Some(hash),
            None => None,
            Some(other) => {
                panic!("an OrMap value must never yield a non-OrMap leaf, got {other:?}")
            }
        }
    }

    /// Every OR leaf vector equals the canonical `merkle_leaf_hash` OR arm,
    /// with `null` standing for "no leaf" (TG-MRK-001).
    #[test]
    fn merkle_vectors_or_leaf_cases_match_canonical_leaf() {
        let file = load_vectors();
        let computed: Vec<Option<u32>> = file.or_leaf.iter().map(or_case_leaf).collect();

        if regen_requested() {
            rewrite_vectors(|fresh| {
                assert_eq!(fresh.or_leaf.len(), computed.len(), "orLeaf cases changed");
                for (case, leaf) in fresh.or_leaf.iter_mut().zip(&computed) {
                    case.expected = *leaf;
                }
            });
            return;
        }

        // Collected rather than asserted one by one, so a single run names every
        // case that disagrees instead of stopping at the first.
        let mismatches: Vec<String> = file
            .or_leaf
            .iter()
            .zip(&computed)
            .enumerate()
            .filter(|(_, (case, leaf))| case.expected != **leaf)
            .map(|(index, (case, leaf))| {
                format!(
                    "orLeaf[{index}] `{}` (key {:?}): vector expects {:?}, merkle_leaf_hash returned {:?}",
                    case.name, case.key, case.expected, leaf
                )
            })
            .collect();
        assert!(
            mismatches.is_empty(),
            "{} orLeaf vector(s) disagree with the canonical leaf:\n{}",
            mismatches.len(),
            mismatches.join("\n")
        );
    }

    fn lww_case_leaf(case: &LwwLeafCase) -> u32 {
        let record = RecordValue::Lww {
            value: Value::Null,
            timestamp: Timestamp {
                millis: case.millis,
                counter: case.counter,
                node_id: case.node_id.clone(),
            },
        };
        match merkle_leaf_hash(&case.key, &record) {
            Some((MerkleLeafKind::Lww, hash)) => hash,
            other => panic!("an Lww value must yield an Lww leaf, got {other:?}"),
        }
    }

    /// Every LWW leaf vector equals the canonical `merkle_leaf_hash` LWW arm
    /// (TG-MRK-001).
    #[test]
    fn merkle_vectors_lww_leaf_cases_match_canonical_leaf() {
        let file = load_vectors();
        let computed: Vec<u32> = file.lww_leaf.iter().map(lww_case_leaf).collect();

        if regen_requested() {
            rewrite_vectors(|fresh| {
                assert_eq!(
                    fresh.lww_leaf.len(),
                    computed.len(),
                    "lwwLeaf cases changed"
                );
                for (case, leaf) in fresh.lww_leaf.iter_mut().zip(&computed) {
                    case.expected = *leaf;
                }
            });
            return;
        }

        let mismatches: Vec<String> = file
            .lww_leaf
            .iter()
            .zip(&computed)
            .enumerate()
            .filter(|(_, (case, leaf))| case.expected != **leaf)
            .map(|(index, (case, leaf))| {
                format!(
                    "lwwLeaf[{index}] `{}` (key {:?}): vector expects {}, merkle_leaf_hash returned {leaf}",
                    case.name, case.key, case.expected
                )
            })
            .collect();
        assert!(
            mismatches.is_empty(),
            "{} lwwLeaf vector(s) disagree with the canonical leaf:\n{}",
            mismatches.len(),
            mismatches.join("\n")
        );
    }

    /// Bucket maps are compared with zero-hash children dropped on both sides:
    /// an emptied child keeps a zero hash as residue, and the protocol reads a
    /// missing child as zero, so the two are the same bucket.
    fn normalised_buckets(buckets: &HashMap<char, u32>) -> BTreeMap<String, u32> {
        buckets
            .iter()
            .filter(|(_, hash)| **hash != 0)
            .map(|(child, hash)| (child.to_string(), *hash))
            .collect()
    }

    /// What ONE flat trie over a case's leaves reports: its root, and the
    /// buckets at `""` and at every depth-1 path present.
    struct FlatTrieView {
        root: u32,
        buckets: BTreeMap<String, BTreeMap<String, u32>>,
    }

    fn flat_trie_view(case: &FlatTrieCase) -> FlatTrieView {
        match case.kind.as_str() {
            "lww" => {
                let mut tree = MerkleTree::default_depth();
                for leaf in &case.leaves {
                    tree.update(&leaf.key, leaf.leaf_hash);
                }
                view_of(tree.get_root_hash(), |path| tree.get_buckets(path))
            }
            "or" => {
                let mut tree = ORMapMerkleTree::default_depth();
                for leaf in &case.leaves {
                    tree.update(&leaf.key, leaf.leaf_hash);
                }
                view_of(tree.get_root_hash(), |path| tree.get_buckets(path))
            }
            other => panic!("flatTrie `{}`: unknown kind {other:?}", case.name),
        }
    }

    fn view_of(root: u32, buckets_at: impl Fn(&str) -> HashMap<char, u32>) -> FlatTrieView {
        let top = normalised_buckets(&buckets_at(""));
        let mut buckets = BTreeMap::new();
        for child in top.keys() {
            buckets.insert(child.clone(), normalised_buckets(&buckets_at(child)));
        }
        buckets.insert(String::new(), top);
        FlatTrieView { root, buckets }
    }

    fn normalised_expected(case: &FlatTrieCase) -> BTreeMap<String, BTreeMap<String, u32>> {
        case.expected_buckets
            .iter()
            .map(|(path, children)| {
                let kept = children
                    .iter()
                    .filter(|(_, hash)| **hash != 0)
                    .map(|(child, hash)| (child.clone(), *hash))
                    .collect();
                (path.clone(), kept)
            })
            .collect()
    }

    /// Every flat-trie vector equals the root and buckets of ONE core-rust trie
    /// folding the case's leaves, whatever partitions the keys hash to
    /// (TG-MRK-001). A case whose keys share fewer than three partitions could
    /// not tell a flat trie from a per-partition aggregate, so it is rejected.
    #[test]
    fn merkle_vectors_flat_trie_cases_match_single_trie_reference() {
        let file = load_vectors();
        assert!(
            file.flat_trie.iter().any(|c| c.kind == "lww")
                && file.flat_trie.iter().any(|c| c.kind == "or"),
            "flatTrie needs at least one lww and one or case"
        );

        let partitions: Vec<Vec<u32>> = file
            .flat_trie
            .iter()
            .map(|case| {
                case.leaves
                    .iter()
                    .map(|l| hash_to_partition(&l.key))
                    .collect()
            })
            .collect();
        for (case, case_partitions) in file.flat_trie.iter().zip(&partitions) {
            let distinct: BTreeSet<u32> = case_partitions.iter().copied().collect();
            assert!(
                case.leaves.len() >= 6 && distinct.len() >= 3,
                "flatTrie `{}` must hold at least 6 keys over at least 3 distinct partitions, \
                 got {} keys over partitions {distinct:?}",
                case.name,
                case.leaves.len()
            );
        }
        let views: Vec<FlatTrieView> = file.flat_trie.iter().map(flat_trie_view).collect();

        if regen_requested() {
            rewrite_vectors(|fresh| {
                assert_eq!(fresh.flat_trie.len(), views.len(), "flatTrie cases changed");
                for ((case, view), case_partitions) in
                    fresh.flat_trie.iter_mut().zip(views).zip(&partitions)
                {
                    assert_eq!(
                        case.leaves.len(),
                        case_partitions.len(),
                        "flatTrie leaves changed"
                    );
                    for (leaf, partition) in case.leaves.iter_mut().zip(case_partitions) {
                        leaf.partition = *partition;
                    }
                    case.expected_root = view.root;
                    case.expected_buckets = view.buckets;
                }
            });
            return;
        }

        let mut mismatches: Vec<String> = Vec::new();
        for ((case, view), case_partitions) in file.flat_trie.iter().zip(&views).zip(&partitions) {
            for (leaf, partition) in case.leaves.iter().zip(case_partitions) {
                if leaf.partition != *partition {
                    mismatches.push(format!(
                        "flatTrie `{}` key {:?}: vector records partition {}, hash_to_partition returned {partition}",
                        case.name, leaf.key, leaf.partition
                    ));
                }
            }
            if case.expected_root != view.root {
                mismatches.push(format!(
                    "flatTrie `{}`: vector expects root {}, single trie returned {}",
                    case.name, case.expected_root, view.root
                ));
            }
            let expected = normalised_expected(case);
            if expected != view.buckets {
                mismatches.push(format!(
                    "flatTrie `{}`: vector expects buckets {expected:?}, single trie returned {:?}",
                    case.name, view.buckets
                ));
            }
        }
        assert!(
            mismatches.is_empty(),
            "{} flatTrie vector field(s) disagree with the single-trie reference:\n{}",
            mismatches.len(),
            mismatches.join("\n")
        );
    }

    /// Bytes one `merkle_leaf_hash` call allocates on an OR slot of `n` records
    /// with 29-char distinct tags and no tombstones.
    #[cfg(feature = "count-alloc")]
    fn leaf_hash_bytes(n: usize) -> u64 {
        let records: Vec<OrMapEntry> = (0..n).map(|i| entry(format!("{i:020}:0:node-a"))).collect();
        let value = RecordValue::OrMap {
            records,
            tombstones: Vec::new(),
        };
        let before = stats_alloc::INSTRUMENTED_SYSTEM.stats().bytes_allocated;
        let leaf = merkle_leaf_hash("k", &value);
        let after = stats_alloc::INSTRUMENTED_SYSTEM.stats().bytes_allocated;
        std::hint::black_box(leaf);
        (after - before) as u64
    }

    /// The OR leaf hash allocates only the tag sort buffer: with no tombstones the
    /// tombstone buffer is empty and allocates nothing, so the whole call stays
    /// within `n * size_of::<&str>() + 64` bytes — no joined tag string, no
    /// formatted leaf string.
    ///
    /// A local allocation proof, not a CI guard: CI never enables `count-alloc`,
    /// and the counters are process-global, so it is meaningful only when run
    /// alone and single-threaded:
    /// `cargo test --release -p topgun-server --lib --features count-alloc -- --ignored --test-threads=1 count_alloc_`
    #[cfg(feature = "count-alloc")]
    #[test]
    #[ignore = "local allocation proof: run single-threaded under count-alloc"]
    fn count_alloc_leaf_hash() {
        let readings: Vec<(usize, u64, u64)> = [1_000_usize, 10_000]
            .into_iter()
            .map(|n| {
                let bound = (n * std::mem::size_of::<&str>() + 64) as u64;
                (n, leaf_hash_bytes(n), bound)
            })
            .collect();
        for (n, bytes, bound) in &readings {
            println!("count_alloc_leaf_hash N={n} bytes_allocated={bytes} bound={bound}");
        }
        for (n, bytes, bound) in readings {
            assert!(
                bytes <= bound,
                "N={n}: merkle_leaf_hash allocated {bytes} bytes, bound {bound}"
            );
        }
    }
}
