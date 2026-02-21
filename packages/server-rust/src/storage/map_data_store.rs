//! External persistence backend trait for the storage layer.
//!
//! Defines [`MapDataStore`], the Layer 3 abstraction over write-through and
//! write-behind persistence strategies. The [`RecordStore`](super::RecordStore)
//! calls `add()` / `remove()` on every mutation; the implementation decides
//! when and how to actually persist the data.

use async_trait::async_trait;

use super::record::RecordValue;

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
