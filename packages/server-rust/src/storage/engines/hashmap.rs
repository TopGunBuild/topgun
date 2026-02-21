//! In-memory [`StorageEngine`] implementation backed by [`DashMap`].
//!
//! Provides concurrent read/write access without external locking.
//! Suitable for development, testing, and production workloads where
//! all data fits in memory.

use dashmap::DashMap;
use rand::Rng;

use crate::storage::engine::{FetchResult, IterationCursor, StorageEngine};
use crate::storage::record::Record;

/// In-memory storage backed by [`DashMap`] for concurrent read access.
///
/// All operations are lock-free for readers and use fine-grained sharding
/// internally (via `DashMap`) for writers. This makes it well-suited for
/// read-heavy workloads typical of CRDT data grids.
pub struct HashMapStorage {
    entries: DashMap<String, Record>,
}

impl HashMapStorage {
    /// Creates a new, empty `HashMapStorage`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: DashMap::new(),
        }
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

/// Encodes an offset into cursor state bytes (little-endian `u64`).
fn encode_cursor_offset(offset: u64) -> Vec<u8> {
    offset.to_le_bytes().to_vec()
}

impl StorageEngine for HashMapStorage {
    fn put(&self, key: &str, record: Record) -> Option<Record> {
        self.entries.insert(key.to_string(), record)
    }

    fn get(&self, key: &str) -> Option<Record> {
        self.entries.get(key).map(|r| r.clone())
    }

    fn remove(&self, key: &str) -> Option<Record> {
        self.entries.remove(key).map(|(_, r)| r)
    }

    fn contains_key(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    fn len(&self) -> usize {
        self.entries.len()
    }

    fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn clear(&self) {
        self.entries.clear();
    }

    fn destroy(&self) {
        self.clear();
    }

    fn estimated_cost(&self) -> u64 {
        self.entries.iter().map(|r| r.value().metadata.cost).sum()
    }

    fn snapshot_iter(&self) -> Vec<(String, Record)> {
        self.entries
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect()
    }

    fn random_samples(&self, sample_count: usize) -> Vec<(String, Record)> {
        if sample_count == 0 {
            return Vec::new();
        }

        let mut rng = rand::rng();
        let mut reservoir: Vec<(String, Record)> = Vec::with_capacity(sample_count);

        for (i, entry) in self.entries.iter().enumerate() {
            let pair = (entry.key().clone(), entry.value().clone());
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

        let items: Vec<(String, Record)> =
            snapshot.into_iter().skip(offset).take(size).collect();

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
