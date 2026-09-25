//! In-memory [`StorageEngine`] implementation backed by [`DashMap`].
//!
//! Provides concurrent read/write access without external locking.
//! Suitable for development, testing, and production workloads where
//! all data fits in memory.

use std::hash::BuildHasher;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use dashmap::mapref::entry::Entry;
use dashmap::DashMap;
use rand::Rng;

use crate::storage::engine::{
    new_slot_cell, observer_reentry, FetchResult, IterationCursor, PutIfAbsentOutcome, SlotCell,
    SlotInit, StorageEngine, UpdateInPlaceOutcome,
};
use crate::storage::record::{Record, RecordMetadata, RecordValue};

/// Number of vacancy-generation counters per engine. Keys share counters by
/// hash, so a removal of one key can spuriously move another key's
/// generation; 256 keeps that rare enough that a bounded retry absorbs it.
const VACANCY_STRIPES: usize = 256;

/// In-memory storage backed by [`DashMap`] for concurrent read access.
///
/// All operations are lock-free for readers and use fine-grained sharding
/// internally (via `DashMap`) for writers. This makes it well-suited for
/// read-heavy workloads typical of CRDT data grids.
///
/// Each slot is a shared [`SlotCell`], so an in-place write mutates the record
/// the engine, a pending write-behind entry and its flush all hold, instead of
/// copying it out. Locks are always taken entry (`DashMap` shard) first, cell
/// second.
pub struct HashMapStorage {
    entries: DashMap<String, Arc<SlotCell>>,
    /// Vacancy generations, indexed by key hash (see
    /// [`StorageEngine::vacancy_generation`]).
    vacancy: Box<[AtomicU64]>,
}

impl HashMapStorage {
    /// Creates a new, empty `HashMapStorage`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: DashMap::new(),
            vacancy: (0..VACANCY_STRIPES).map(|_| AtomicU64::new(0)).collect(),
        }
    }

    fn stripe(&self, key: &str) -> &AtomicU64 {
        // The modulo bounds the value below VACANCY_STRIPES, so the cast is exact.
        #[allow(clippy::cast_possible_truncation)]
        let index = (self.entries.hasher().hash_one(key) % VACANCY_STRIPES as u64) as usize;
        &self.vacancy[index]
    }

    /// Advances the key's vacancy generation. Called with the key's entry lock
    /// held, so an insert that checks the generation under the same lock sees
    /// either the key still present or the advanced generation (TG-OR-007).
    fn bump(&self, key: &str) {
        self.stripe(key).fetch_add(1, Ordering::AcqRel);
    }
}

impl Default for HashMapStorage {
    fn default() -> Self {
        Self::new()
    }
}

/// Decodes a cursor's opaque state into a `u64` offset.
///
/// Empty state (from `IterationCursor::start()`) is treated as offset 0.
fn decode_cursor_offset(cursor: &IterationCursor) -> u64 {
    if cursor.state.is_empty() {
        0
    } else {
        let mut buf = [0u8; 8];
        let len = cursor.state.len().min(8);
        buf[..len].copy_from_slice(&cursor.state[..len]);
        u64::from_le_bytes(buf)
    }
}

/// The record of a cell the engine just dropped: moved out when the engine held
/// the only reference, else copied under the lock, leaving the cell to the
/// holders that still owe it a write.
fn take_record(cell: Arc<SlotCell>) -> Record {
    match Arc::try_unwrap(cell) {
        Ok(cell) => cell.into_inner(),
        Err(shared) => shared.lock().clone(),
    }
}

/// Encodes an offset into cursor state bytes (little-endian `u64`).
fn encode_cursor_offset(offset: u64) -> Vec<u8> {
    offset.to_le_bytes().to_vec()
}

impl StorageEngine for HashMapStorage {
    fn put(&self, key: &str, record: Record) -> Option<Record> {
        observer_reentry::check("put");
        match self.entries.entry(key.to_string()) {
            Entry::Occupied(mut occ) => {
                // The strong count is read under the entry lock: every new
                // reference to a slot's cell is made by the engine under this
                // lock or cloned from a holder, so "unshared" cannot change
                // before the replace below.
                if Arc::strong_count(occ.get()) == 1 {
                    let mut current = occ.get().lock();
                    Some(std::mem::replace(&mut *current, record))
                } else {
                    // A pending write still holds this cell and must persist what
                    // it holds: the new record gets a cell of its own.
                    let old = occ.get().lock().clone();
                    occ.insert(new_slot_cell(record));
                    Some(old)
                }
            }
            Entry::Vacant(vac) => {
                vac.insert(new_slot_cell(record));
                None
            }
        }
    }

    fn get(&self, key: &str) -> Option<Record> {
        observer_reentry::check("get");
        self.entries.get(key).map(|cell| cell.lock().clone())
    }

    fn mark_stored(&self, key: &str, now: i64, token: u64) -> bool {
        observer_reentry::check("mark_stored");
        // The entry guard keeps the slot from being removed or replaced while
        // the cell lock makes the token check and the mutation atomic against
        // any in-place write. The token uniquely identifies the exact write the
        // caller persisted: a concurrent same-key write (any timestamp, equal or
        // newer) carries a different token and is left dirty until its own
        // persist completes.
        if let Some(cell) = self.entries.get(key) {
            let mut record = cell.lock();
            if record.metadata.write_token == token {
                record.metadata.on_store(now);
                return true;
            }
        }
        false
    }

    fn update_in_place(
        &self,
        key: &str,
        now: i64,
        init: Option<SlotInit>,
        init_generation: Option<u64>,
        mutate: &mut dyn FnMut(&mut RecordValue) -> bool,
        cost_of: &dyn Fn(&RecordValue) -> u64,
    ) -> UpdateInPlaceOutcome {
        observer_reentry::check("update_in_place");
        // `entry` holds the shard write lock across the match, so the
        // check-mutate-or-insert is atomic against any other engine op on this
        // key — there is no get()+put() window a concurrent writer could tear.
        // The one owned-key allocation on the occupied path (bytes) is trivial
        // next to the per-op resident-snapshot churn this seam exists to remove.
        match self.entries.entry(key.to_string()) {
            Entry::Occupied(occ) => {
                let cell = occ.get();
                {
                    let mut record = cell.lock();
                    if !mutate(&mut record.value) {
                        return UpdateInPlaceOutcome::Unchanged;
                    }
                    // Mint a fresh write token and bump version/last_update in
                    // place, then re-cost from the mutated value.
                    record.metadata.on_update(now);
                    record.metadata.cost = cost_of(&record.value);
                }
                UpdateInPlaceOutcome::Written {
                    cell: Arc::clone(cell),
                    inserted: false,
                }
            }
            Entry::Vacant(vac) => {
                let Some(init) = init else {
                    return UpdateInPlaceOutcome::Absent;
                };
                // A removal of the key since the caller read `init` means `init`
                // may be a value that removal superseded: refuse it before the
                // closure runs, so the caller can re-read (TG-OR-001, TG-OR-007).
                if init_generation.is_some_and(|g| g != self.vacancy_generation(key)) {
                    return UpdateInPlaceOutcome::Stale;
                }
                let cell = match init {
                    SlotInit::Value(mut value) => {
                        if !mutate(&mut value) {
                            return UpdateInPlaceOutcome::Unchanged;
                        }
                        let cost = cost_of(&value);
                        new_slot_cell(Record {
                            value,
                            metadata: RecordMetadata::new(now, cost),
                        })
                    }
                    SlotInit::Cell(cell) => {
                        {
                            // A pending write owns this cell: a `false` from the
                            // closure leaves it untouched (the caller obligation
                            // above), and nothing is inserted.
                            let mut record = cell.lock();
                            if !mutate(&mut record.value) {
                                return UpdateInPlaceOutcome::Unchanged;
                            }
                            let cost = cost_of(&record.value);
                            record.metadata = RecordMetadata::new(now, cost);
                        }
                        cell
                    }
                };
                vac.insert(Arc::clone(&cell));
                UpdateInPlaceOutcome::Written {
                    cell,
                    inserted: true,
                }
            }
        }
    }

    fn vacancy_generation(&self, key: &str) -> u64 {
        observer_reentry::check("vacancy_generation");
        self.stripe(key).load(Ordering::Acquire)
    }

    fn put_if_absent_at(
        &self,
        key: &str,
        init: SlotInit,
        metadata: RecordMetadata,
        generation: u64,
    ) -> PutIfAbsentOutcome {
        observer_reentry::check("put_if_absent_at");
        match self.entries.entry(key.to_string()) {
            Entry::Occupied(occ) => PutIfAbsentOutcome::Resident(occ.get().lock().clone()),
            Entry::Vacant(vac) => {
                if generation != self.vacancy_generation(key) {
                    return PutIfAbsentOutcome::Stale;
                }
                let cell = match init {
                    SlotInit::Value(value) => new_slot_cell(Record { value, metadata }),
                    SlotInit::Cell(cell) => {
                        cell.lock().metadata = metadata;
                        cell
                    }
                };
                vac.insert(cell);
                PutIfAbsentOutcome::Inserted
            }
        }
    }

    fn remove(&self, key: &str) -> Option<Record> {
        observer_reentry::check("remove");
        // Through `entry` so the bump happens under the key's lock whether or
        // not the key is resident: removing a non-resident key must still
        // invalidate a reader that loaded it from the data store.
        match self.entries.entry(key.to_string()) {
            Entry::Occupied(occ) => {
                self.bump(key);
                Some(take_record(occ.remove()))
            }
            Entry::Vacant(_vac) => {
                self.bump(key);
                None
            }
        }
    }

    fn remove_if(&self, key: &str, predicate: &dyn Fn(&Record) -> bool) -> Option<Record> {
        observer_reentry::check("remove_if");
        self.entries
            .remove_if(key, |_, cell| {
                let remove = predicate(&cell.lock());
                if remove {
                    self.bump(key);
                }
                remove
            })
            .map(|(_, cell)| take_record(cell))
    }

    fn touch(&self, key: &str, now: i64) -> Option<Record> {
        observer_reentry::check("touch");
        let cell = self.entries.get(key)?;
        let mut record = cell.lock();
        record.metadata.on_access(now);
        Some(record.clone())
    }

    fn contains_key(&self, key: &str) -> bool {
        observer_reentry::check("contains_key");
        self.entries.contains_key(key)
    }

    fn len(&self) -> usize {
        observer_reentry::check("len");
        self.entries.len()
    }

    fn is_empty(&self) -> bool {
        observer_reentry::check("is_empty");
        self.entries.is_empty()
    }

    fn clear(&self) {
        observer_reentry::check("clear");
        // Each resident key is bumped under its own lock as it is removed; the
        // stripes are then all advanced, because `retain` never visits a key
        // that is absent but being loaded. A cell a pending write still holds
        // stays alive with that holder.
        self.entries.retain(|key, _| {
            self.bump(key);
            false
        });
        for stripe in &*self.vacancy {
            stripe.fetch_add(1, Ordering::AcqRel);
        }
    }

    fn destroy(&self) {
        self.clear();
    }

    fn estimated_cost(&self) -> u64 {
        observer_reentry::check("estimated_cost");
        self.entries
            .iter()
            .map(|entry| entry.value().lock().metadata.cost)
            .sum()
    }

    fn snapshot_iter(&self) -> Vec<(String, Record)> {
        observer_reentry::check("snapshot_iter");
        self.entries
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().lock().clone()))
            .collect()
    }

    fn random_samples(&self, sample_count: usize) -> Vec<(String, Record)> {
        observer_reentry::check("random_samples");
        if sample_count == 0 {
            return Vec::new();
        }

        let mut rng = rand::rng();
        let mut reservoir: Vec<(String, Record)> = Vec::with_capacity(sample_count);

        for (i, entry) in self.entries.iter().enumerate() {
            let pair = (entry.key().clone(), entry.value().lock().clone());
            if i < sample_count {
                reservoir.push(pair);
            } else {
                // Replace an existing sample with probability sample_count / (i + 1)
                let j = rng.random_range(0..=i);
                if j < sample_count {
                    reservoir[j] = pair;
                }
            }
        }

        reservoir
    }

    #[cfg(test)]
    fn test_slot(&self, key: &str) -> Option<Arc<SlotCell>> {
        self.entries.get(key).map(|cell| Arc::clone(&cell))
    }

    fn fetch_keys(&self, cursor: &IterationCursor, size: usize) -> FetchResult<String> {
        let snapshot = self.snapshot_iter();
        let total = snapshot.len();
        // Cursor offsets are always small (bounded by storage size), so truncation is safe.
        #[allow(clippy::cast_possible_truncation)]
        let offset = decode_cursor_offset(cursor) as usize;

        let items: Vec<String> = snapshot
            .into_iter()
            .skip(offset)
            .take(size)
            .map(|(k, _)| k)
            .collect();

        let new_offset = offset + items.len();
        let finished = new_offset >= total;

        FetchResult {
            items,
            next_cursor: IterationCursor {
                state: encode_cursor_offset(new_offset as u64),
                finished,
            },
        }
    }

    fn fetch_entries(
        &self,
        cursor: &IterationCursor,
        size: usize,
    ) -> FetchResult<(String, Record)> {
        let snapshot = self.snapshot_iter();
        let total = snapshot.len();
        // Cursor offsets are always small (bounded by storage size), so truncation is safe.
        #[allow(clippy::cast_possible_truncation)]
        let offset = decode_cursor_offset(cursor) as usize;

        let items: Vec<(String, Record)> = snapshot.into_iter().skip(offset).take(size).collect();

        let new_offset = offset + items.len();
        let finished = new_offset >= total;

        FetchResult {
            items,
            next_cursor: IterationCursor {
                state: encode_cursor_offset(new_offset as u64),
                finished,
            },
        }
    }
}

#[cfg(test)]
#[allow(clippy::cast_sign_loss)]
mod tests {
    use super::*;
    use crate::storage::record::{RecordMetadata, RecordValue};
    use topgun_core::hlc::Timestamp;
    use topgun_core::types::Value;

    fn make_record(cost: u64) -> Record {
        Record {
            value: RecordValue::Lww {
                value: Value::Null,
                timestamp: Timestamp {
                    millis: 0,
                    counter: 0,
                    node_id: String::new(),
                },
            },
            metadata: RecordMetadata::new(0, cost),
        }
    }

    #[test]
    fn put_get_remove_round_trip() {
        let storage = HashMapStorage::new();
        let record = make_record(100);

        assert!(storage.put("key1", record).is_none());

        let fetched = storage.get("key1");
        assert!(fetched.is_some());
        assert_eq!(fetched.unwrap().metadata.cost, 100);

        let removed = storage.remove("key1");
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().metadata.cost, 100);

        assert!(storage.get("key1").is_none());
    }

    #[test]
    fn contains_key_reflects_state() {
        let storage = HashMapStorage::new();

        assert!(!storage.contains_key("key1"));

        storage.put("key1", make_record(10));
        assert!(storage.contains_key("key1"));

        storage.remove("key1");
        assert!(!storage.contains_key("key1"));
    }

    #[test]
    fn len_and_is_empty() {
        let storage = HashMapStorage::new();

        assert!(storage.is_empty());
        assert_eq!(storage.len(), 0);

        storage.put("a", make_record(1));
        assert!(!storage.is_empty());
        assert_eq!(storage.len(), 1);

        storage.put("b", make_record(2));
        assert_eq!(storage.len(), 2);

        storage.remove("a");
        assert_eq!(storage.len(), 1);
    }

    #[test]
    fn clear_empties_storage() {
        let storage = HashMapStorage::new();

        storage.put("a", make_record(1));
        storage.put("b", make_record(2));
        storage.put("c", make_record(3));
        assert_eq!(storage.len(), 3);

        storage.clear();
        assert!(storage.is_empty());
        assert_eq!(storage.len(), 0);
    }

    #[test]
    fn fetch_keys_with_cursor_pagination() {
        let storage = HashMapStorage::new();
        for i in 0..5 {
            storage.put(&format!("key{i}"), make_record(i as u64));
        }

        // First page: 3 keys
        let cursor = IterationCursor::start();
        let result = storage.fetch_keys(&cursor, 3);
        assert_eq!(result.items.len(), 3);
        assert!(!result.next_cursor.finished);

        // Second page: remaining 2 keys
        let result2 = storage.fetch_keys(&result.next_cursor, 3);
        assert_eq!(result2.items.len(), 2);
        assert!(result2.next_cursor.finished);
    }

    #[test]
    fn fetch_entries_with_cursor_pagination() {
        let storage = HashMapStorage::new();
        for i in 0..5 {
            storage.put(&format!("key{i}"), make_record(i as u64));
        }

        // First page: 2 entries
        let cursor = IterationCursor::start();
        let result = storage.fetch_entries(&cursor, 2);
        assert_eq!(result.items.len(), 2);
        assert!(!result.next_cursor.finished);

        // Second page: 2 entries
        let result2 = storage.fetch_entries(&result.next_cursor, 2);
        assert_eq!(result2.items.len(), 2);
        assert!(!result2.next_cursor.finished);

        // Third page: 1 entry, finished
        let result3 = storage.fetch_entries(&result2.next_cursor, 2);
        assert_eq!(result3.items.len(), 1);
        assert!(result3.next_cursor.finished);
    }

    #[test]
    fn snapshot_iter_returns_all_entries() {
        let storage = HashMapStorage::new();
        storage.put("a", make_record(10));
        storage.put("b", make_record(20));
        storage.put("c", make_record(30));

        let snapshot = storage.snapshot_iter();
        assert_eq!(snapshot.len(), 3);

        let mut keys: Vec<String> = snapshot.into_iter().map(|(k, _)| k).collect();
        keys.sort();
        assert_eq!(keys, vec!["a", "b", "c"]);
    }

    #[test]
    fn random_samples_respects_count() {
        let storage = HashMapStorage::new();
        for i in 0..100 {
            storage.put(&format!("key{i}"), make_record(i as u64));
        }

        let samples = storage.random_samples(5);
        assert_eq!(samples.len(), 5);

        // Requesting more than available returns all
        let all = storage.random_samples(200);
        assert_eq!(all.len(), 100);
    }

    #[test]
    fn random_samples_empty_storage() {
        let storage = HashMapStorage::new();

        let samples = storage.random_samples(10);
        assert_eq!(samples.len(), 0);

        let samples = storage.random_samples(0);
        assert_eq!(samples.len(), 0);
    }

    #[test]
    fn fetch_keys_cursor_past_end() {
        let storage = HashMapStorage::new();
        storage.put("a", make_record(1));
        storage.put("b", make_record(2));

        // Create a cursor with offset well beyond storage size
        let past_end = IterationCursor {
            state: encode_cursor_offset(100),
            finished: false,
        };

        let result = storage.fetch_keys(&past_end, 10);
        assert!(result.items.is_empty());
        assert!(result.next_cursor.finished);
    }

    #[test]
    fn fetch_entries_cursor_past_end() {
        let storage = HashMapStorage::new();
        storage.put("a", make_record(1));
        storage.put("b", make_record(2));

        // Create a cursor with offset well beyond storage size
        let past_end = IterationCursor {
            state: encode_cursor_offset(100),
            finished: false,
        };

        let result = storage.fetch_entries(&past_end, 10);
        assert!(result.items.is_empty());
        assert!(result.next_cursor.finished);
    }

    #[test]
    fn mark_stored_marks_resident_record_clean() {
        let storage = HashMapStorage::new();
        // make_record stamps last_update_time = 0 (RecordMetadata::new), so it
        // starts dirty (last_update 0 > last_stored 0 is false → actually clean
        // at 0/0; bump update to force dirty, which also mints a fresh token).
        let mut record = make_record(10);
        record.metadata.on_update(5);
        let token = record.metadata.write_token;
        storage.put("a", record);
        assert!(storage.get("a").unwrap().metadata.is_dirty());

        // Matching token → mark applies.
        assert!(storage.mark_stored("a", 5, token));
        assert!(!storage.get("a").unwrap().metadata.is_dirty());
        assert_eq!(storage.get("a").unwrap().metadata.last_stored_time, 5);
    }

    #[test]
    fn mark_stored_skips_newer_write() {
        let storage = HashMapStorage::new();
        // Two records for the same key: first write (A) is placed, then a newer
        // write (B) replaces it. Passing A's token must not mark B clean —
        // a different token means a different logical write owns the slot.
        let record_a = make_record(10);
        let token_a = record_a.metadata.write_token;
        storage.put("a", record_a);

        // Second write — on_update mints a new token, making B's token != A's.
        let mut record_b = make_record(10);
        record_b.metadata.on_update(10);
        let token_b = record_b.metadata.write_token;
        storage.put("a", record_b);

        // Tokens must differ: each logical write boundary gets a unique token.
        assert_ne!(token_a, token_b, "each write must carry a unique token");

        // A stale persist (using A's token) must NOT mark B's slot clean.
        assert!(!storage.mark_stored("a", 5, token_a));
        assert!(storage.get("a").unwrap().metadata.is_dirty());
        assert_eq!(storage.get("a").unwrap().metadata.last_stored_time, 0);

        // Matching token (B's) → mark applies.
        assert!(storage.mark_stored("a", 10, token_b));
        assert!(!storage.get("a").unwrap().metadata.is_dirty());
    }

    #[test]
    fn mark_stored_absent_key_returns_false() {
        let storage = HashMapStorage::new();
        assert!(!storage.mark_stored("missing", 100, 42));
    }

    /// AC1 behavioral test: same-millisecond two-concurrent-same-key writes.
    ///
    /// Two writes to the same key share an identical timestamp (same-ms tie).
    /// Under the old timestamp guard (<= now), write A's `mark_stored` call would
    /// pass for write B's resident record, prematurely marking B clean before B
    /// persists. Under the token guard (== token), the tokens differ and the
    /// loser stays dirty until its own persist.
    ///
    /// This test is RED if the token guard is reverted to the timestamp-only
    /// `<= now` guard — the dirty-state assertion would pass for the loser
    /// under the old guard, so the test would fail on the assert that the
    /// loser remains dirty.
    #[test]
    fn ac1_same_millisecond_write_stays_dirty_until_own_persist() {
        let storage = HashMapStorage::new();

        // Both writes share the same "now" — simulating a same-millisecond tie
        // without relying on wall-clock timing.
        let same_now: i64 = 1_000_000;

        // Write A: first to arrive, gets pushed out by Write B.
        let record_a = Record {
            value: RecordValue::Lww {
                value: topgun_core::types::Value::String("value-a".to_string()),
                timestamp: Timestamp {
                    millis: same_now as u64,
                    counter: 0,
                    node_id: "node-a".to_string(),
                },
            },
            metadata: RecordMetadata::new(same_now, 64),
        };
        let token_a = record_a.metadata.write_token;
        storage.put("key", record_a);

        // Write B: arrives after A, replaces the slot. Same timestamp as A.
        let record_b = Record {
            value: RecordValue::Lww {
                value: topgun_core::types::Value::String("value-b".to_string()),
                timestamp: Timestamp {
                    millis: same_now as u64,
                    counter: 0,
                    node_id: "node-b".to_string(),
                },
            },
            metadata: RecordMetadata::new(same_now, 64),
        };
        let token_b = record_b.metadata.write_token;
        storage.put("key", record_b);

        // AC1 KL2 proof: tokens must differ even for same-ms writes.
        assert_ne!(
            token_a, token_b,
            "same-millisecond writes must carry distinct tokens (KL2)"
        );

        // Resident is now Write B. Write A's persist completes and calls
        // mark_stored with A's token — must NOT mark B clean.
        let marked = storage.mark_stored("key", same_now, token_a);
        assert!(
            !marked,
            "mark_stored with A's token must not match B's resident slot"
        );

        // B is still dirty — its own persist has not completed.
        let resident = storage.get("key").unwrap();
        assert!(
            resident.metadata.is_dirty(),
            "loser write B must remain dirty until its own persist completes; \
             this assertion is RED under the old timestamp-only guard"
        );

        // Token inequality is the concrete proof that KL2 holds.
        assert_ne!(
            token_a, resident.metadata.write_token,
            "loser token must differ from resident token (KL2 token-inequality proof)"
        );

        // AC2: B's own persist then completes with the matching token → B is marked clean.
        let marked_b = storage.mark_stored("key", same_now, token_b);
        assert!(marked_b, "B's own persist must mark it clean");
        assert!(
            !storage.get("key").unwrap().metadata.is_dirty(),
            "B must be clean after its own persist"
        );
    }

    #[test]
    fn estimated_cost_sums_all_records() {
        let storage = HashMapStorage::new();

        assert_eq!(storage.estimated_cost(), 0);

        storage.put("a", make_record(100));
        storage.put("b", make_record(200));
        storage.put("c", make_record(300));

        assert_eq!(storage.estimated_cost(), 600);

        storage.remove("b");
        assert_eq!(storage.estimated_cost(), 400);
    }
}
