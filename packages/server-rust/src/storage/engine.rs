//! Low-level storage engine trait and cursor-based iteration types.
//!
//! Defines [`StorageEngine`], the innermost storage layer (analogous to
//! Hazelcast's `Storage<K,R>`). Implementations provide in-memory key-value
//! storage with cursor-based iteration support.

use super::record::Record;

/// Opaque cursor for resumable iteration over storage entries.
///
/// Implementations encode their internal position in the `state` field.
/// Consumers should treat `state` as opaque and only check `finished`.
#[derive(Debug, Clone)]
pub struct IterationCursor {
    /// Opaque state for the storage implementation to resume iteration.
    pub state: Vec<u8>,
    /// Whether iteration has completed (no more entries).
    pub finished: bool,
}

impl IterationCursor {
    /// Creates a cursor positioned at the beginning of the storage.
    #[must_use]
    pub fn start() -> Self {
        Self {
            state: Vec::new(),
            finished: false,
        }
    }
}

/// Result of a cursor-based fetch operation.
///
/// Contains the fetched items and an updated cursor for the next call.
#[derive(Debug)]
pub struct FetchResult<T> {
    /// The fetched items.
    pub items: Vec<T>,
    /// Updated cursor for the next fetch call.
    pub next_cursor: IterationCursor,
}

/// Low-level typed key-value storage with cursor-based iteration.
///
/// Innermost storage layer (analogous to Hazelcast's `Storage<K,R>`).
/// Implementations are in-memory (`HashMap`, `BTreeMap`, etc.).
/// All operations are synchronous.
///
/// Wrapped in `Arc<dyn StorageEngine>` for sharing across async boundaries.
pub trait StorageEngine: Send + Sync + 'static {
    /// Insert or replace a record by key. Returns the previous record if any.
    fn put(&self, key: &str, record: Record) -> Option<Record>;

    /// Retrieve a record by key, or `None` if not present.
    fn get(&self, key: &str) -> Option<Record>;

    /// Remove a record by key, returning the removed record.
    fn remove(&self, key: &str) -> Option<Record>;

    /// Check if a key exists without returning the record.
    fn contains_key(&self, key: &str) -> bool;

    /// Return the number of entries.
    fn len(&self) -> usize;

    /// Check if the storage is empty.
    fn is_empty(&self) -> bool;

    /// Clear all entries. Takes `&self` for `Arc<dyn StorageEngine>` compatibility.
    fn clear(&self);

    /// Destroy the storage, releasing all resources. Takes `&self`.
    fn destroy(&self);

    /// Estimated heap cost of all stored entries in bytes.
    fn estimated_cost(&self) -> u64;

    /// Fetch at least `size` keys starting from `cursor`.
    fn fetch_keys(&self, cursor: &IterationCursor, size: usize) -> FetchResult<String>;

    /// Fetch at least `size` entries (key + record) starting from `cursor`.
    fn fetch_entries(
        &self,
        cursor: &IterationCursor,
        size: usize,
    ) -> FetchResult<(String, Record)>;

    /// Return a point-in-time snapshot of all entries.
    ///
    /// The snapshot is mutation-tolerant (concurrent modifications do not fail).
    fn snapshot_iter(&self) -> Vec<(String, Record)>;

    /// Return `sample_count` random entries for eviction sampling.
    fn random_samples(&self, sample_count: usize) -> Vec<(String, Record)>;
}
