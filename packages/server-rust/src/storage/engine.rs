//! Low-level storage engine trait and cursor-based iteration types.
//!
//! Defines [`StorageEngine`], the innermost storage layer (analogous to
//! Hazelcast's `Storage<K,R>`). Implementations provide in-memory key-value
//! storage with cursor-based iteration support.

use std::sync::Arc;

use super::record::{Record, RecordMetadata, RecordValue};

/// One key's resident record behind its own lock, shareable by `Arc` between
/// the engine slot and whatever still owes the record a durable write (a queued
/// write-behind entry, its staging slot, an in-flight flush).
///
/// `parking_lot` rather than `std`: the std mutex allocates its OS lock lazily
/// on first `lock()` on some platforms, which would make every new key cost a
/// second allocation beside the `Arc`. Its guard is `!Send` while the crate's
/// `send_guard` feature is off, so a guard held across `.await` in a `Send`
/// future is a compile error. There is no poisoning: a panic under the lock
/// leaves the record as the panicking code left it, the same as the engine's
/// shard locks.
///
/// [`SlotCell::lock`] is the only way to lock a cell. In debug builds it counts
/// the cell locks the current thread holds, and every engine entry point calls
/// [`held_cell_locks::check`], so an engine call made while its own thread
/// holds a cell lock — which would wait on that lock, or take an entry lock
/// after a cell lock against the entry → cell order — panics by name instead
/// of hanging.
pub struct SlotCell {
    record: parking_lot::Mutex<Record>,
}

// The wrapper adds no state: a cell costs exactly the mutex it wraps, which is
// what the allocation proofs price a new key at.
const _: () =
    assert!(std::mem::size_of::<SlotCell>() == std::mem::size_of::<parking_lot::Mutex<Record>>());

impl SlotCell {
    /// Locks the cell for the lifetime of the returned guard.
    pub fn lock(&self) -> SlotGuard<'_> {
        let guard = self.record.lock();
        held_cell_locks::acquired();
        SlotGuard { guard }
    }

    /// Consumes a cell nobody else holds and returns its record.
    #[must_use]
    pub fn into_inner(self) -> Record {
        self.record.into_inner()
    }
}

impl std::fmt::Debug for SlotCell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `try_lock` inside the mutex's own formatter: formatting never waits
        // and holds nothing past the call, so it needs no depth accounting.
        f.debug_struct("SlotCell")
            .field("record", &self.record)
            .finish()
    }
}

/// A held [`SlotCell`] lock; derefs to the record.
#[must_use = "the cell stays locked only while the guard is alive"]
pub struct SlotGuard<'a> {
    guard: parking_lot::MutexGuard<'a, Record>,
}

impl std::ops::Deref for SlotGuard<'_> {
    type Target = Record;

    fn deref(&self) -> &Record {
        &self.guard
    }
}

impl std::ops::DerefMut for SlotGuard<'_> {
    fn deref_mut(&mut self) -> &mut Record {
        &mut self.guard
    }
}

impl Drop for SlotGuard<'_> {
    fn drop(&mut self) {
        // The guard is `!Send`, so it drops on the thread that counted it.
        held_cell_locks::released();
    }
}

/// Wraps a record in a new, unshared [`SlotCell`].
#[must_use]
pub fn new_slot_cell(record: Record) -> Arc<SlotCell> {
    Arc::new(SlotCell {
        record: parking_lot::Mutex::new(record),
    })
}

/// Debug-build detector for a storage engine entered while the current thread
/// holds a slot-cell lock.
///
/// Such a call either waits on a cell lock its own thread holds (a silent
/// hang: `parking_lot` is not reentrant) or takes an entry lock after a cell
/// lock, against the entry → cell order. Observers running under the in-place
/// write's cell lock, `mutate` and cost closures, `remove_if` predicates and a
/// caller that passes `cell.lock()…` as an argument to an engine call are all
/// covered, because the count follows the guard, not the call site. In release
/// builds the count and the check compile to nothing.
pub mod held_cell_locks {
    #[cfg(debug_assertions)]
    thread_local! {
        static DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    }

    #[inline]
    pub(super) fn acquired() {
        #[cfg(debug_assertions)]
        DEPTH.with(|depth| depth.set(depth.get() + 1));
    }

    #[inline]
    pub(super) fn released() {
        #[cfg(debug_assertions)]
        DEPTH.with(|depth| depth.set(depth.get() - 1));
    }

    /// Guards an engine entry point; `entry` names the operation for the
    /// message.
    ///
    /// # Panics
    ///
    /// In debug builds, when the current thread holds a slot-cell lock.
    #[inline]
    pub fn check(entry: &'static str) {
        #[cfg(debug_assertions)]
        DEPTH.with(|depth| {
            assert!(
                depth.get() == 0,
                "storage engine entered while this thread holds a slot-cell lock: \
                 `{entry}` would wait on that lock or take an entry lock after it"
            );
        });
        #[cfg(not(debug_assertions))]
        let _ = entry;
    }
}

/// What a caller hands the engine to create a key it found absent: an owned
/// value, or a cell another holder (a pending write-behind entry) already owns.
#[derive(Debug)]
pub enum SlotInit {
    /// An owned value; the engine builds the slot's record around it.
    Value(RecordValue),
    /// A cell loaded from the data store's pending state.
    Cell(Arc<SlotCell>),
}

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
    /// The key was absent and an `init` value was supplied together with an
    /// `init_generation` that no longer equals the key's vacancy generation: a
    /// removal of this key landed after the caller read `init`, so `init` may
    /// be a value that removal already superseded. Returned BEFORE `mutate`
    /// runs and nothing is inserted; the caller re-reads and retries (TG-OR-007).
    Stale,
    /// The mutation closure ran but reported no durable change was needed
    /// (returned `false`), so the resident metadata was left untouched. No
    /// observer notification and no write-through are owed. Used by the prune
    /// sweep when the target tag was already gone from the tombstone set.
    Unchanged,
    /// The record was created or updated in place. `cell` holds the mutated
    /// record (for the caller's observer fan-out under the cell lock and the
    /// async write-through); `inserted` is `true` when the key was absent and a
    /// fresh record was created (fire `on_put`), `false` when an existing
    /// resident record was mutated (fire `on_update`).
    Written {
        /// The cell holding the mutated record — the handle the caller passes
        /// to the observer fan-out and the async write-through.
        cell: Arc<SlotCell>,
        /// `true` if a fresh record was inserted (key was absent), `false` if
        /// an existing resident record was mutated in place.
        inserted: bool,
    },
}

/// Outcome of [`StorageEngine::put_if_absent_at`].
#[derive(Debug)]
pub enum PutIfAbsentOutcome {
    /// The key was absent and the generation matched: the record was inserted.
    Inserted,
    /// The key was already resident; nothing was inserted. Carries a clone of
    /// the resident record.
    Resident(Record),
    /// The key was absent but its vacancy generation moved: a removal
    /// intervened, so nothing was inserted.
    Stale,
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
    /// `cost_of` over the mutated value, all under the same lock, then returns
    /// the cell holding the mutated record in [`UpdateInPlaceOutcome::Written`].
    ///
    /// Caller obligation: a `mutate` closure that returns `false` MUST NOT have
    /// modified the value. An occupied slot keeps whatever the closure left in
    /// it, and when `init` is a [`SlotInit::Cell`] the value belongs to a
    /// pending write-behind entry whose flush would persist such a change with
    /// no frame and no observer.
    ///
    /// If the key is absent: when `init` is `None`, the call is a no-op returning
    /// [`UpdateInPlaceOutcome::Absent`] without invoking `mutate`. When `init` is
    /// `Some` and `init_generation` is `Some(g)` with `g` different from the
    /// key's current [`vacancy_generation`](StorageEngine::vacancy_generation),
    /// the call returns [`UpdateInPlaceOutcome::Stale`] without invoking
    /// `mutate`. Otherwise `mutate` is applied to `init`'s value — for a
    /// [`SlotInit::Cell`], to the cell's own value under its lock — and on
    /// `true` the slot is inserted with metadata minted via
    /// `RecordMetadata::new` (a cell is inserted itself, not a copy of it); on
    /// `false` nothing is inserted and the call returns
    /// [`UpdateInPlaceOutcome::Unchanged`]. The generation check and the insert
    /// happen under the same per-key lock, so no removal can fall between them.
    fn update_in_place(
        &self,
        key: &str,
        now: i64,
        init: Option<SlotInit>,
        init_generation: Option<u64>,
        mutate: &mut dyn FnMut(&mut RecordValue) -> bool,
        cost_of: &dyn Fn(&RecordValue) -> u64,
    ) -> UpdateInPlaceOutcome;

    /// The key's vacancy generation: a counter that every removal of the key
    /// advances (whether or not the key was present), read BEFORE a caller
    /// loads the key's value from the data store.
    ///
    /// A caller that later inserts that loaded value passes the generation back
    /// ([`put_if_absent_at`](StorageEngine::put_if_absent_at),
    /// [`update_in_place`](StorageEngine::update_in_place)); an unchanged
    /// generation proves no removal of the key intervened since the read began,
    /// so the loaded value cannot resurrect a removed or superseded one
    /// (TG-OR-007). Counters may be shared between keys, so a change can be
    /// spurious; that costs the caller a retry, never a wrong insert.
    fn vacancy_generation(&self, key: &str) -> u64;

    /// Insert `init` with `metadata` only if the key is absent AND its vacancy
    /// generation still equals `generation`, both checked under the key's lock.
    /// A [`SlotInit::Cell`] is inserted itself, its metadata replaced by
    /// `metadata` under its lock.
    fn put_if_absent_at(
        &self,
        key: &str,
        init: SlotInit,
        metadata: RecordMetadata,
        generation: u64,
    ) -> PutIfAbsentOutcome;

    /// Remove a record by key, returning the removed record.
    ///
    /// Advances the key's vacancy generation under the key's lock, also when the
    /// key is absent (a removal of a non-resident key must still invalidate a
    /// concurrent reader's load).
    fn remove(&self, key: &str) -> Option<Record>;

    /// Remove the key only if `predicate` holds for the resident record,
    /// evaluated under the key's lock; returns the removed record. Advances the
    /// vacancy generation exactly when a record is removed.
    fn remove_if(&self, key: &str, predicate: &dyn Fn(&Record) -> bool) -> Option<Record>;

    /// Record a read access on the resident record in place (`on_access(now)`
    /// under the key's lock) and return a clone of it, or `None` if absent.
    /// Never writes a value back, so a concurrent write cannot be overwritten.
    fn touch(&self, key: &str, now: i64) -> Option<Record>;

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

    /// The cell the engine's slot for `key` holds, for identity assertions in
    /// tests. `None` when the key is absent or the engine keeps no cells.
    #[cfg(test)]
    fn test_slot(&self, key: &str) -> Option<Arc<SlotCell>> {
        let _ = key;
        None
    }
}
