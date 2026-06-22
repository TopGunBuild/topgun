//! External persistence backend trait for the storage layer.
//!
//! Defines [`MapDataStore`], the Layer 3 abstraction over write-through and
//! write-behind persistence strategies. The [`RecordStore`](super::RecordStore)
//! calls `add()` / `remove()` on every mutation; the implementation decides
//! when and how to actually persist the data.

use async_trait::async_trait;

use super::record::RecordValue;

/// A single durable record's Merkle leaf coordinate: its key and the `u32`
/// leaf hash computed over the persisted value.
///
/// `leaf_hash` is the same `u32` space as the in-memory Merkle leaf hash
/// (`fnv1a`-derived); enumerating it from the durable store lets the Merkle
/// root be rebuilt from persistence WITHOUT loading full record values into
/// memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MerkleLeaf {
    /// Record key within the map.
    pub key: String,
    /// `u32` leaf hash over the persisted value (matches in-memory leaf hash).
    pub leaf_hash: u32,
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
