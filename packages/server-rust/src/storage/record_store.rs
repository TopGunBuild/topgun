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
/// Used as `Box<dyn RecordStore>`.
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
    async fn remove_backup(
        &self,
        key: &str,
        provenance: CallerProvenance,
    ) -> anyhow::Result<()>;

    // --- Batch operations ---

    /// Get multiple records.
    async fn get_all(&self, keys: &[String]) -> anyhow::Result<Vec<(String, Record)>>;

    // --- Iteration ---

    /// Fetch keys with cursor-based pagination.
    fn fetch_keys(&self, cursor: &IterationCursor, size: usize) -> FetchResult<String>;

    /// Fetch entries with cursor-based pagination.
    fn fetch_entries(
        &self,
        cursor: &IterationCursor,
        size: usize,
    ) -> FetchResult<(String, Record)>;

    /// Iterate all records with an object-safe consumer.
    ///
    /// Calls `consumer` for each non-expired entry. Uses `&mut dyn FnMut`
    /// instead of generic `F: FnMut` for `Box<dyn RecordStore>` compatibility.
    fn for_each_boxed(&self, consumer: &mut dyn FnMut(&str, &Record), is_backup: bool);

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
    /// Returns the sequence number of the last flushed operation.
    async fn soft_flush(&self) -> anyhow::Result<u64>;

    /// Access the underlying `StorageEngine` (Layer 1).
    fn storage(&self) -> &dyn StorageEngine;

    /// Access the underlying `MapDataStore` (Layer 3).
    fn map_data_store(&self) -> &dyn MapDataStore;
}
