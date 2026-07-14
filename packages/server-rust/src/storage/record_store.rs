//! Per-map-per-partition record store trait.
//!
//! Defines [`RecordStore`], the primary interface that operation handlers
//! interact with. Orchestrates Layer 1 ([`StorageEngine`](super::StorageEngine))
//! and Layer 3 ([`MapDataStore`](super::MapDataStore)), adding metadata tracking,
//! expiry, eviction, and mutation observation.
//!
//! Also defines supporting types: [`CallerProvenance`], [`ExpiryPolicy`],
//! and [`ExpiryReason`].

use async_trait::async_trait;

use super::engine::{FetchResult, IterationCursor, StorageEngine};
use super::map_data_store::MapDataStore;
use super::record::{Record, RecordValue};

/// Origin of a write operation.
///
/// Determines how the `RecordStore` processes the write (e.g., whether to
/// trigger write-through, update access statistics, or notify observers).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallerProvenance {
    /// Write originated from a client request.
    Client,
    /// Write is a backup replication from the primary partition owner.
    Backup,
    /// Write is a replication event from another cluster node.
    Replication,
    /// Write is a load from the backing `MapDataStore`.
    Load,
    /// Write is the result of a CRDT merge operation.
    CrdtMerge,
}

/// Expiry configuration for a record.
///
/// Controls time-to-live and maximum idle time for automatic expiration.
/// Use [`ExpiryPolicy::NONE`] for records that should never expire.
#[derive(Debug, Clone)]
pub struct ExpiryPolicy {
    /// Time-to-live in milliseconds from creation. 0 = no TTL.
    pub ttl_millis: u64,
    /// Maximum idle time in milliseconds since last access. 0 = no max idle.
    pub max_idle_millis: u64,
}

impl ExpiryPolicy {
    /// No expiration policy. Both TTL and max-idle are disabled.
    pub const NONE: Self = Self {
        ttl_millis: 0,
        max_idle_millis: 0,
    };
}

/// Reason a record expired.
///
/// Returned by [`RecordStore::has_expired`] to indicate whether and why
/// a record has expired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpiryReason {
    /// The record has not expired.
    NotExpired,
    /// The record expired due to time-to-live.
    Ttl,
    /// The record expired due to exceeding the maximum idle time.
    MaxIdle,
}

/// Per-map-per-partition record store.
///
/// Primary interface that operation handlers interact with.
/// Orchestrates Layer 1 ([`StorageEngine`]) and Layer 3 ([`MapDataStore`]),
/// adding metadata tracking, expiry, eviction, and mutation observation.
///
/// **never-evict-dirty invariant:** Any record whose
/// [`RecordMetadata::is_dirty`](super::record::RecordMetadata::is_dirty) returns
/// `true` MUST NOT be evicted by [`RecordStore::evict_lru`]. Evicting a dirty
/// record discards an acknowledged write that has not yet been flushed to the
/// backing [`MapDataStore`], violating the durability contract. All
/// implementations that handle LRU eviction are required to enforce this
/// invariant; the orchestrator surfaces violations via [`RecordStore::dirty_count`]
/// backpressure logging.
///
/// Used as `Arc<dyn RecordStore>`.
#[async_trait]
pub trait RecordStore: Send + Sync {
    /// Name of the map this record store manages.
    fn name(&self) -> &str;

    /// Partition ID this record store belongs to.
    fn partition_id(&self) -> u32;

    // --- Core CRUD ---

    /// Get a record, loading from `MapDataStore` if not in memory.
    ///
    /// Updates access statistics if `touch` is true.
    async fn get(&self, key: &str, touch: bool) -> anyhow::Result<Option<Record>>;

    /// Check if a key exists in memory (does NOT load from `MapDataStore`).
    fn exists_in_memory(&self, key: &str) -> bool;

    /// Put a value, returning the old value if it existed.
    ///
    /// Handles write-through to `MapDataStore` based on provenance.
    async fn put(
        &self,
        key: &str,
        value: RecordValue,
        expiry: ExpiryPolicy,
        provenance: CallerProvenance,
    ) -> anyhow::Result<Option<RecordValue>>;

    /// Mutate a resident record's value in place, firing the same observer
    /// notifications and durable write-through as [`put`](RecordStore::put)
    /// WITHOUT a full get→build→put round trip.
    ///
    /// `mutate` runs synchronously under the engine's per-key write lock and
    /// returns `true` if it made a change that must be persisted, or `false`
    /// for a no-op (e.g. a prune whose target tag was already gone). It MUST
    /// return `true` whenever it altered the value.
    ///
    /// If the key is absent: when `init` is `Some`, a fresh record is created
    /// from it, `mutate` is applied, and (on a `true` return) `on_put` fires;
    /// when `init` is `None`, the call is a no-op. Returns `true` when a record
    /// was created or updated (a write-through was owed), `false` otherwise.
    ///
    /// This exists for the OR-Map write path, whose per-op read-modify-write
    /// otherwise cloned the whole ~130 KB resident snapshot on every op. The OR
    /// observers ignore the pre-image, so no old value is materialized.
    ///
    /// The default implementation falls back to a get→mutate→put round trip so
    /// non-optimized stores remain correct; [`DefaultRecordStore`] overrides it
    /// with the true in-place seam.
    async fn update_in_place(
        &self,
        key: &str,
        init: Option<RecordValue>,
        expiry: ExpiryPolicy,
        provenance: CallerProvenance,
        mutate: &mut (dyn for<'a> FnMut(&'a mut RecordValue) -> bool + Send),
    ) -> anyhow::Result<bool> {
        let existing = self.get(key, false).await?;
        let mut value = match existing {
            Some(record) => record.value,
            None => match init {
                Some(v) => v,
                None => return Ok(false),
            },
        };
        if !mutate(&mut value) {
            return Ok(false);
        }
        self.put(key, value, expiry, provenance).await?;
        Ok(true)
    }

    /// Remove a record, returning the old value.
    async fn remove(
        &self,
        key: &str,
        provenance: CallerProvenance,
    ) -> anyhow::Result<Option<RecordValue>>;

    /// Put a record received from backup replication.
    async fn put_backup(
        &self,
        key: &str,
        record: Record,
        provenance: CallerProvenance,
    ) -> anyhow::Result<()>;

    /// Remove a record on backup.
    async fn remove_backup(&self, key: &str, provenance: CallerProvenance) -> anyhow::Result<()>;

    // --- Batch operations ---

    /// Get multiple records.
    async fn get_all(&self, keys: &[String]) -> anyhow::Result<Vec<(String, Record)>>;

    // --- Iteration ---

    /// Fetch keys with cursor-based pagination.
    fn fetch_keys(&self, cursor: &IterationCursor, size: usize) -> FetchResult<String>;

    /// Fetch entries with cursor-based pagination.
    fn fetch_entries(&self, cursor: &IterationCursor, size: usize)
        -> FetchResult<(String, Record)>;

    /// Iterate all records with an object-safe consumer.
    ///
    /// Calls `consumer` for each non-expired entry. Uses `&mut dyn FnMut`
    /// instead of generic `F: FnMut` for `Box<dyn RecordStore>` compatibility.
    fn for_each_boxed(&self, consumer: &mut dyn FnMut(&str, &Record), is_backup: bool);

    /// Materialize a durable-but-non-resident record into the in-memory engine,
    /// but ONLY if the key is currently absent.
    ///
    /// This generalizes the single-key lazy-load in [`get()`](RecordStore::get)
    /// to the full-scan path: a record streamed from the datastore is made
    /// resident so the synchronous `for_each_boxed` scan (and the DAG scan) sees
    /// it, surfacing persisted-but-non-resident keys (after restart or eviction)
    /// to a full-scan QUERY. It MUST NOT overwrite a resident value — an
    /// in-memory record reflects the live (possibly fresher) write-path state,
    /// and clobbering it with a stale durable read would lose an unflushed or
    /// concurrently-merged update. Returns `true` if the record was inserted,
    /// `false` if a resident copy already existed.
    fn hydrate_loaded(&self, key: &str, value: RecordValue) -> bool;

    // --- Size and cost ---

    /// Number of entries in the record store.
    fn size(&self) -> usize;

    /// Whether the record store is empty.
    fn is_empty(&self) -> bool;

    /// Total estimated heap cost of all entries.
    fn owned_entry_cost(&self) -> u64;

    // --- Expiry ---

    /// Check if a record has expired.
    fn has_expired(&self, key: &str, now: i64, is_backup: bool) -> ExpiryReason;

    /// Evict expired entries up to a percentage of total expirable entries.
    fn evict_expired(&self, percentage: u32, now: i64, is_backup: bool);

    /// Whether this record store has any entries that can expire.
    fn is_expirable(&self) -> bool;

    // --- Eviction ---

    /// Evict a single entry (e.g., due to memory pressure).
    fn evict(&self, key: &str, is_backup: bool) -> Option<RecordValue>;

    /// Evict all non-locked entries.
    fn evict_all(&self, is_backup: bool) -> u32;

    /// Whether eviction should be triggered based on current memory usage.
    fn should_evict(&self) -> bool;

    /// Evict up to `target_count` least-recently-used non-dirty records.
    ///
    /// MUST skip records where `metadata.is_dirty()` is true — evicting a
    /// dirty record discards an acked write that has not yet flushed to the
    /// backend, violating durability. This is the never-evict-dirty invariant.
    ///
    /// Returns the number of records actually evicted (may be less than
    /// `target_count` if fewer non-dirty candidates exist).
    ///
    /// Implementations MUST use saturating cast (`u32::try_from(...).unwrap_or(u32::MAX)`)
    /// when converting the internal `usize` eviction count to the `u32` return type.
    /// Exceeding `u32::MAX` evictions in a single call is unreachable in practice
    /// but MUST NOT panic.
    fn evict_lru(&self, target_count: u32, is_backup: bool) -> u32;

    /// Number of records currently dirty (in-memory mutation not yet
    /// flushed to the backing `MapDataStore`). Surfaced for orchestrator
    /// backpressure logging.
    fn dirty_count(&self) -> u64;

    // --- Lifecycle ---

    /// Initialize the record store (create backing storage, register observers).
    fn init(&mut self);

    /// Clear all data (used by `IMap.clear()`).
    fn clear(&self, is_backup: bool) -> u32;

    /// Reset to initial state (used during migration).
    fn reset(&self);

    /// Destroy the record store and release all resources.
    fn destroy(&self);

    // --- MapDataStore integration ---

    /// Flush pending writes to the backing `MapDataStore`.
    ///
    /// Returns the sequence number of the last QUEUED (assigned) operation, or 0
    /// if empty — NOT the last flushed one. The implementation delegates to
    /// [`MapDataStore::soft_flush`], which notifies the background flush loop and
    /// returns the current assigned-sequence counter; the actual flush completes
    /// asynchronously. A caller that needs a real byte-durability signal must use
    /// [`MapDataStore::flushed_watermark`], not this return value.
    async fn soft_flush(&self) -> anyhow::Result<u64>;

    /// Access the underlying `StorageEngine` (Layer 1).
    fn storage(&self) -> &dyn StorageEngine;

    /// Access the underlying `MapDataStore` (Layer 3).
    fn map_data_store(&self) -> &dyn MapDataStore;
}
