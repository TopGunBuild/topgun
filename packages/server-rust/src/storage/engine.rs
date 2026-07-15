//! Low-level storage engine trait and cursor-based iteration types.
//!
//! Defines [`StorageEngine`], the innermost storage layer (analogous to
//! Hazelcast's `Storage<K,R>`). Implementations provide in-memory key-value
//! storage with cursor-based iteration support.

use super::record::{Record, RecordValue};

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

/// Outcome of an in-place record mutation via [`StorageEngine::update_in_place`].
///
/// Distinguishes the three terminal states so the caller (a
/// [`RecordStore`](super::RecordStore)) can fire the correct observer
/// notification and decide whether a durable write-through is owed, WITHOUT a
/// full get→clone→put round trip.
pub enum UpdateInPlaceOutcome {
    /// The key was absent and no `init` value was supplied, so nothing was
    /// mutated. No observer notification and no write-through are owed.
    Absent,
    /// The mutation closure ran but reported no durable change was needed
    /// (returned `false`), so the resident metadata was left untouched. No
    /// observer notification and no write-through are owed. Used by the prune
    /// sweep when the target tag was already gone from the tombstone set.
    Unchanged,
    /// The record was created or updated in place. `record` is a clone of the
    /// mutated resident record (for the caller's post-lock observer fan-out and
    /// async write-through); `inserted` is `true` when the key was absent and a
    /// fresh record was created (fire `on_put`), `false` when an existing
    /// resident record was mutated (fire `on_update`).
    Written {
        /// Clone of the mutated resident record — the single owned copy the
        /// caller hands to the async write-through and observer fan-out.
        record: Record,
        /// `true` if a fresh record was inserted (key was absent), `false` if
        /// an existing resident record was mutated in place.
        inserted: bool,
    },
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

    /// Mark a resident record as persisted (clean) in place.
    ///
    /// Sets `last_stored_time = now` under the engine's per-key write lock, so
    /// the check-and-mutate is atomic with respect to any other engine op on
    /// this key: there is no read-modify-write window of a separate `get()` +
    /// `put()`, and the value is never re-put, so a concurrent same-key write
    /// can be neither clobbered nor lost.
    ///
    /// The mark is applied only when the resident record's `write_token` equals
    /// the caller's `token` — a per-write identity check. This guarantees the
    /// mark applies only when the resident record is the exact write the caller
    /// just persisted. A concurrent same-key write (any timestamp, equal or
    /// newer) carries a different token and is left dirty until its own persist
    /// completes. Two concurrent puts to the same key in the same millisecond
    /// therefore never prematurely mark each other clean.
    ///
    /// `now` is retained: on a successful match, `on_store(now)` stamps
    /// `last_stored_time = now` for `is_dirty()` bookkeeping. The token
    /// identifies the write just persisted, whether a direct `put()` or a
    /// deferred flush of the current resident.
    ///
    /// Returns `true` if a record was found and the mark applied; `false` if
    /// the key is absent or the resident record is a different write (token
    /// mismatch — a newer write owns the slot).
    fn mark_stored(&self, key: &str, now: i64, token: u64) -> bool;

    /// Mutate a resident record's value in place under the engine's per-key
    /// write lock, avoiding the full get→clone→put round trip.
    ///
    /// `mutate` runs synchronously while the shard write lock is held, receiving
    /// `&mut RecordValue` for the resident slot, and returns `true` if it made a
    /// change that must be persisted (a durable write is owed) or `false` if the
    /// call is a no-op. The closure MUST return `true` whenever it altered the
    /// value — returning `false` after a change would leave a resident mutation
    /// that never reaches the durable backend (data loss on eviction).
    ///
    /// Implementations MUST invoke `mutate` **at most once** per call. Callers may
    /// rely on single invocation (e.g. the `OR_ADD` merge closure moves its entry in
    /// via `Option::take` and would panic on a second call); an engine that retries
    /// the closure would break that contract.
    ///
    /// On a `true` return the engine stamps `on_update(now)` (bumping version and
    /// minting a fresh per-write token) and recomputes `metadata.cost` via
    /// `cost_of` over the mutated value, all under the same lock, then returns a
    /// clone of the mutated record in [`UpdateInPlaceOutcome::Written`].
    ///
    /// If the key is absent: when `init` is `Some`, the value is created from it,
    /// `mutate` is applied, and (on a `true` return) the fresh record is inserted
    /// with metadata minted via `RecordMetadata::new`; when `init` is `None`, the
    /// call is a no-op returning [`UpdateInPlaceOutcome::Absent`].
    fn update_in_place(
        &self,
        key: &str,
        now: i64,
        init: Option<RecordValue>,
        mutate: &mut dyn FnMut(&mut RecordValue) -> bool,
        cost_of: &dyn Fn(&RecordValue) -> u64,
    ) -> UpdateInPlaceOutcome;

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
    fn fetch_entries(&self, cursor: &IterationCursor, size: usize)
        -> FetchResult<(String, Record)>;

    /// Return a point-in-time snapshot of all entries.
    ///
    /// The snapshot is mutation-tolerant (concurrent modifications do not fail).
    fn snapshot_iter(&self) -> Vec<(String, Record)>;

    /// Return `sample_count` random entries for eviction sampling.
    fn random_samples(&self, sample_count: usize) -> Vec<(String, Record)>;
}
