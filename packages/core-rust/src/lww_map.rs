//! Last-Write-Wins Map (LWW-Map) CRDT implementation.
//!
//! `LWWMap<V>` is a conflict-free replicated data type that resolves concurrent
//! writes by always keeping the entry with the highest [`Timestamp`]. It integrates
//! with [`MerkleTree`] for efficient delta synchronization.
//!
//! # Conflict resolution
//!
//! When two records compete for the same key, the one with the greater timestamp
//! wins. Timestamp ordering is: `millis` first, then `counter`, then `node_id`
//! (lexicographic). This provides total ordering across all nodes.
//!
//! # Tombstones
//!
//! Deletions are represented as tombstones (`value: None`) with a timestamp.
//! Tombstones participate in merge conflict resolution like any other record.
//! Use [`LWWMap::prune`] to garbage-collect old tombstones.
//!
//! # TTL (Time-To-Live)
//!
//! Records can carry an optional TTL in milliseconds. Expired records are
//! filtered from [`LWWMap::get`] and [`LWWMap::entries`] but remain in storage
//! until explicitly pruned or overwritten.

use std::collections::HashMap;

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::hash::fnv1a_hash;
use crate::hlc::{HLC, LWWRecord, Timestamp};
use crate::merkle::MerkleTree;

/// A Last-Write-Wins Map providing conflict-free convergence.
///
/// Keys are [`String`]s. Values are generic over `V` with bounds
/// `Clone + Serialize + DeserializeOwned + PartialEq`.
///
/// # Examples
///
/// ```
/// use topgun_core::hlc::{HLC, SystemClock};
/// use topgun_core::lww_map::LWWMap;
/// use topgun_core::Value;
///
/// let hlc = HLC::new("node-1".to_string(), Box::new(SystemClock));
/// let mut map: LWWMap<Value> = LWWMap::new(hlc);
///
/// map.set("user:1", Value::String("Alice".to_string()), None);
/// assert_eq!(map.get("user:1"), Some(&Value::String("Alice".to_string())));
/// ```
pub struct LWWMap<V> {
    data: HashMap<String, LWWRecord<V>>,
    hlc: HLC,
    merkle_tree: MerkleTree,
}

impl<V> LWWMap<V>
where
    V: Clone + Serialize + DeserializeOwned + PartialEq,
{
    /// Creates a new empty `LWWMap` with the given HLC instance.
    ///
    /// The HLC is used for timestamp generation on `set()` and `remove()`,
    /// causality tracking on `merge()`, and TTL expiry checks via its clock source.
    #[must_use]
    pub fn new(hlc: HLC) -> Self {
        Self {
            data: HashMap::new(),
            hlc,
            merkle_tree: MerkleTree::default_depth(),
        }
    }

    /// Stores a value with a new HLC timestamp.
    ///
    /// Generates a fresh timestamp from the internal HLC, stores the record,
    /// and updates the `MerkleTree`. Returns a clone of the stored record.
    pub fn set(
        &mut self,
        key: impl Into<String>,
        value: V,
        ttl_ms: Option<u64>,
    ) -> LWWRecord<V> {
        let key = key.into();
        let timestamp = self.hlc.now();
        let record = LWWRecord {
            value: Some(value),
            timestamp,
            ttl_ms,
        };
        self.data.insert(key.clone(), record.clone());
        self.update_merkle(&key, &record.timestamp);
        record
    }

    /// Returns the value for a key, filtering tombstones and expired records.
    ///
    /// Returns `None` if:
    /// - The key does not exist
    /// - The record is a tombstone (value is `None`)
    /// - The record's TTL has expired
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&V> {
        let record = self.data.get(key)?;
        // Filter tombstones
        let value = record.value.as_ref()?;
        // Filter expired TTL
        if self.is_expired(record) {
            return None;
        }
        Some(value)
    }

    /// Returns the full record for a key, including timestamp and tombstone state.
    ///
    /// Unlike [`get`](LWWMap::get), this does not filter tombstones or expired records.
    /// Useful for synchronization protocols that need the raw record.
    #[must_use]
    pub fn get_record(&self, key: &str) -> Option<&LWWRecord<V>> {
        self.data.get(key)
    }

    /// Creates a tombstone for a key with a new HLC timestamp.
    ///
    /// Always creates and stores a tombstone even if the key does not exist,
    /// matching TypeScript behavior where tombstones are created regardless of
    /// prior existence. Returns a clone of the tombstone record.
    pub fn remove(&mut self, key: &str) -> LWWRecord<V> {
        let timestamp = self.hlc.now();
        let tombstone = LWWRecord {
            value: None,
            timestamp,
            ttl_ms: None,
        };
        self.data.insert(key.to_string(), tombstone.clone());
        self.update_merkle(key, &tombstone.timestamp);
        tombstone
    }

    /// Merges a remote record, returning `true` if local state changed.
    ///
    /// Always updates the HLC with the remote timestamp to maintain causality
    /// (silently ignoring drift errors, matching TypeScript behavior).
    ///
    /// Merge logic: accept remote if no local record exists OR if the remote
    /// timestamp is strictly greater than the local timestamp.
    pub fn merge(&mut self, key: impl Into<String>, remote_record: LWWRecord<V>) -> bool {
        // Always update HLC for causality, ignoring errors
        let _ = self.hlc.update(&remote_record.timestamp);

        let key = key.into();
        let should_accept = match self.data.get(&key) {
            None => true,
            Some(local) => remote_record.timestamp > local.timestamp,
        };

        if should_accept {
            let ts = remote_record.timestamp.clone();
            self.data.insert(key.clone(), remote_record);
            self.update_merkle(&key, &ts);
            true
        } else {
            false
        }
    }

    /// Removes tombstones older than the given threshold.
    ///
    /// Only tombstones (records with `value: None`) whose timestamp is strictly
    /// less than `older_than` are removed. Non-tombstone records are never pruned.
    /// Returns the list of pruned keys.
    pub fn prune(&mut self, older_than: &Timestamp) -> Vec<String> {
        let pruned_keys: Vec<String> = self
            .data
            .iter()
            .filter(|(_, record)| record.value.is_none() && record.timestamp < *older_than)
            .map(|(key, _)| key.clone())
            .collect();

        for key in &pruned_keys {
            self.data.remove(key);
            self.merkle_tree.remove(key);
        }

        pruned_keys
    }

    /// Removes all data and resets the `MerkleTree`.
    pub fn clear(&mut self) {
        self.data.clear();
        self.merkle_tree = MerkleTree::default_depth();
    }

    /// Iterates over non-tombstone, non-expired entries.
    ///
    /// Yields `(&String, &V)` pairs for all live entries. Tombstones and
    /// TTL-expired records are skipped.
    pub fn entries(&self) -> impl Iterator<Item = (&String, &V)> {
        self.data.iter().filter_map(move |(key, record)| {
            let value = record.value.as_ref()?;
            if self.is_expired(record) {
                return None;
            }
            Some((key, value))
        })
    }

    /// Iterates over all keys, including tombstones.
    pub fn all_keys(&self) -> impl Iterator<Item = &String> {
        self.data.keys()
    }

    /// Returns the number of entries, including tombstones.
    #[must_use]
    pub fn size(&self) -> usize {
        self.data.len()
    }

    /// Returns read-only access to the internal `MerkleTree`.
    #[must_use]
    pub fn merkle_tree(&self) -> &MerkleTree {
        &self.merkle_tree
    }

    /// Checks whether a record's TTL has expired.
    fn is_expired(&self, record: &LWWRecord<V>) -> bool {
        if let Some(ttl_ms) = record.ttl_ms {
            let now = self.hlc.clock_source().now();
            record.timestamp.millis + ttl_ms < now
        } else {
            false
        }
    }

    /// Updates the `MerkleTree` with a key's current timestamp.
    ///
    /// Computes the item hash as `fnv1a_hash("{key}:{millis}:{counter}:{node_id}")`.
    fn update_merkle(&mut self, key: &str, ts: &Timestamp) {
        let hash_input = format!("{}:{}:{}:{}", key, ts.millis, ts.counter, ts.node_id);
        let item_hash = fnv1a_hash(&hash_input);
        self.merkle_tree.update(key, item_hash);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
    use std::sync::Arc;

    use super::*;
    use crate::hlc::ClockSource;
    use crate::Value;

    /// A deterministic clock source for testing. Time is controlled explicitly.
    struct FixedClock {
        time: Arc<AtomicU64>,
    }

    impl FixedClock {
        fn new(initial: u64) -> (Self, Arc<AtomicU64>) {
            let time = Arc::new(AtomicU64::new(initial));
            (Self { time: time.clone() }, time)
        }
    }

    impl ClockSource for FixedClock {
        fn now(&self) -> u64 {
            self.time.load(AtomicOrdering::Relaxed)
        }
    }

    /// Helper to create an LWWMap with a fixed clock and return the time control handle.
    fn make_map(initial_time: u64) -> (LWWMap<Value>, Arc<AtomicU64>) {
        let (clock, time) = FixedClock::new(initial_time);
        let hlc = HLC::new("test-node".to_string(), Box::new(clock));
        (LWWMap::new(hlc), time)
    }

    // ---- Basic CRUD ----

    #[test]
    fn set_and_get_basic() {
        let (mut map, _) = make_map(1_000_000);
        map.set("key1", Value::String("value1".to_string()), None);
        assert_eq!(
            map.get("key1"),
            Some(&Value::String("value1".to_string()))
        );
    }

    #[test]
    fn get_nonexistent_key_returns_none() {
        let (map, _) = make_map(1_000_000);
        assert_eq!(map.get("missing"), None);
    }

    #[test]
    fn set_overwrites_existing_value() {
        let (mut map, _) = make_map(1_000_000);
        map.set("key1", Value::String("old".to_string()), None);
        map.set("key1", Value::String("new".to_string()), None);
        assert_eq!(map.get("key1"), Some(&Value::String("new".to_string())));
    }

    #[test]
    fn get_record_returns_full_record() {
        let (mut map, _) = make_map(1_000_000);
        map.set("key1", Value::Int(42), None);
        let record = map.get_record("key1").expect("record exists");
        assert_eq!(record.value, Some(Value::Int(42)));
        assert_eq!(record.timestamp.millis, 1_000_000);
        assert_eq!(record.timestamp.node_id, "test-node");
    }

    #[test]
    fn get_record_nonexistent_returns_none() {
        let (map, _) = make_map(1_000_000);
        assert!(map.get_record("missing").is_none());
    }

    #[test]
    fn size_counts_all_entries() {
        let (mut map, _) = make_map(1_000_000);
        assert_eq!(map.size(), 0);
        map.set("a", Value::Int(1), None);
        assert_eq!(map.size(), 1);
        map.set("b", Value::Int(2), None);
        assert_eq!(map.size(), 2);
    }

    // ---- Tombstone behavior ----

    #[test]
    fn remove_creates_tombstone() {
        let (mut map, _) = make_map(1_000_000);
        map.set("key1", Value::String("value1".to_string()), None);
        map.remove("key1");
        // get returns None for tombstones
        assert_eq!(map.get("key1"), None);
        // but the record still exists
        let record = map.get_record("key1").expect("tombstone exists");
        assert_eq!(record.value, None);
    }

    #[test]
    fn remove_nonexistent_key_creates_tombstone() {
        let (mut map, _) = make_map(1_000_000);
        // Remove a key that was never set
        let tombstone = map.remove("phantom");
        assert_eq!(tombstone.value, None);
        // It should be stored
        let record = map.get_record("phantom").expect("tombstone exists");
        assert_eq!(record.value, None);
        assert_eq!(map.size(), 1);
    }

    #[test]
    fn size_includes_tombstones() {
        let (mut map, _) = make_map(1_000_000);
        map.set("key1", Value::Int(1), None);
        map.set("key2", Value::Int(2), None);
        map.remove("key1");
        // Tombstone still counts
        assert_eq!(map.size(), 2);
    }

    // ---- TTL expiry ----

    #[test]
    fn ttl_not_expired_returns_value() {
        let (mut map, _time) = make_map(1_000_000);
        map.set("temp", Value::String("data".to_string()), Some(500));
        // Clock is at 1_000_000, timestamp.millis is 1_000_000, TTL is 500ms
        // Expires when millis + 500 < now, i.e., 1_000_500 < now
        // now is 1_000_000, not expired
        assert_eq!(
            map.get("temp"),
            Some(&Value::String("data".to_string()))
        );
    }

    #[test]
    fn ttl_expired_returns_none() {
        let (mut map, time) = make_map(1_000_000);
        map.set("temp", Value::String("data".to_string()), Some(500));
        // Advance clock past expiry: millis(1_000_000) + 500 < 1_000_600
        time.store(1_000_600, AtomicOrdering::Relaxed);
        assert_eq!(map.get("temp"), None);
    }

    #[test]
    fn ttl_boundary_not_expired() {
        let (mut map, time) = make_map(1_000_000);
        map.set("temp", Value::String("data".to_string()), Some(500));
        // At exactly the boundary: millis(1_000_000) + 500 = 1_000_500
        // Condition is `<`, so 1_000_500 < 1_000_500 is false => not expired
        time.store(1_000_500, AtomicOrdering::Relaxed);
        assert_eq!(
            map.get("temp"),
            Some(&Value::String("data".to_string()))
        );
    }

    #[test]
    fn ttl_none_never_expires() {
        let (mut map, time) = make_map(1_000_000);
        map.set("perm", Value::String("forever".to_string()), None);
        time.store(u64::MAX, AtomicOrdering::Relaxed);
        assert_eq!(
            map.get("perm"),
            Some(&Value::String("forever".to_string()))
        );
    }

    // ---- Conflict resolution ----

    #[test]
    fn conflict_higher_millis_wins() {
        let (mut map, _) = make_map(1_000_000);

        let record_old = LWWRecord {
            value: Some(Value::String("old".to_string())),
            timestamp: Timestamp {
                millis: 100,
                counter: 0,
                node_id: "A".to_string(),
            },
            ttl_ms: None,
        };
        let record_new = LWWRecord {
            value: Some(Value::String("new".to_string())),
            timestamp: Timestamp {
                millis: 200,
                counter: 0,
                node_id: "B".to_string(),
            },
            ttl_ms: None,
        };

        // Merge old first, then new
        map.merge("key", record_old.clone());
        assert_eq!(map.get("key"), Some(&Value::String("old".to_string())));

        map.merge("key", record_new.clone());
        assert_eq!(map.get("key"), Some(&Value::String("new".to_string())));

        // Merge old again -- should NOT revert
        let changed = map.merge("key", record_old);
        assert!(!changed);
        assert_eq!(map.get("key"), Some(&Value::String("new".to_string())));
    }

    #[test]
    fn conflict_higher_counter_wins() {
        let (mut map, _) = make_map(1_000_000);

        let record_low = LWWRecord {
            value: Some(Value::String("low".to_string())),
            timestamp: Timestamp {
                millis: 100,
                counter: 1,
                node_id: "A".to_string(),
            },
            ttl_ms: None,
        };
        let record_high = LWWRecord {
            value: Some(Value::String("high".to_string())),
            timestamp: Timestamp {
                millis: 100,
                counter: 5,
                node_id: "A".to_string(),
            },
            ttl_ms: None,
        };

        map.merge("key", record_low);
        map.merge("key", record_high);
        assert_eq!(map.get("key"), Some(&Value::String("high".to_string())));
    }

    #[test]
    fn conflict_higher_node_id_wins() {
        let (mut map, _) = make_map(1_000_000);

        let record_a = LWWRecord {
            value: Some(Value::String("valA".to_string())),
            timestamp: Timestamp {
                millis: 100,
                counter: 0,
                node_id: "A".to_string(),
            },
            ttl_ms: None,
        };
        let record_b = LWWRecord {
            value: Some(Value::String("valB".to_string())),
            timestamp: Timestamp {
                millis: 100,
                counter: 0,
                node_id: "B".to_string(),
            },
            ttl_ms: None,
        };

        // Apply A then B
        map.merge("key", record_a.clone());
        map.merge("key", record_b.clone());
        assert_eq!(map.get("key"), Some(&Value::String("valB".to_string())));

        // Try reverse order in a fresh map
        let (mut map2, _) = make_map(1_000_000);
        map2.merge("key", record_b);
        map2.merge("key", record_a);
        // B still wins regardless of merge order
        assert_eq!(map2.get("key"), Some(&Value::String("valB".to_string())));
    }

    #[test]
    fn merge_returns_true_when_state_changes() {
        let (mut map, _) = make_map(1_000_000);
        let record = LWWRecord {
            value: Some(Value::Int(42)),
            timestamp: Timestamp {
                millis: 100,
                counter: 0,
                node_id: "A".to_string(),
            },
            ttl_ms: None,
        };
        assert!(map.merge("key", record));
    }

    #[test]
    fn merge_returns_false_when_no_change() {
        let (mut map, _) = make_map(1_000_000);
        let newer = LWWRecord {
            value: Some(Value::Int(1)),
            timestamp: Timestamp {
                millis: 200,
                counter: 0,
                node_id: "A".to_string(),
            },
            ttl_ms: None,
        };
        let older = LWWRecord {
            value: Some(Value::Int(2)),
            timestamp: Timestamp {
                millis: 100,
                counter: 0,
                node_id: "A".to_string(),
            },
            ttl_ms: None,
        };
        map.merge("key", newer);
        let changed = map.merge("key", older);
        assert!(!changed);
    }

    // ---- Prune ----

    #[test]
    fn prune_removes_old_tombstones() {
        let (mut map, _) = make_map(1_000_000);
        map.set("key1", Value::String("val1".to_string()), None);
        let tombstone = map.remove("key1");

        // Threshold newer than tombstone -> should prune
        let threshold = Timestamp {
            millis: tombstone.timestamp.millis + 1000,
            counter: 0,
            node_id: "test-node".to_string(),
        };
        let pruned = map.prune(&threshold);
        assert_eq!(pruned, vec!["key1".to_string()]);
        assert!(map.get_record("key1").is_none());
    }

    #[test]
    fn prune_does_not_remove_recent_tombstones() {
        let (mut map, _) = make_map(1_000_000);
        map.set("key1", Value::String("val1".to_string()), None);
        let tombstone = map.remove("key1");

        // Threshold older than tombstone -> should NOT prune
        let threshold = Timestamp {
            millis: tombstone.timestamp.millis - 1000,
            counter: 0,
            node_id: "test-node".to_string(),
        };
        let pruned = map.prune(&threshold);
        assert!(pruned.is_empty());
        assert!(map.get_record("key1").is_some());
    }

    #[test]
    fn prune_does_not_remove_non_tombstones() {
        let (mut map, _) = make_map(1_000_000);
        map.set("key1", Value::String("val1".to_string()), None);

        // Even with a very future threshold, non-tombstones are not pruned
        let threshold = Timestamp {
            millis: u64::MAX,
            counter: u32::MAX,
            node_id: "z".to_string(),
        };
        let pruned = map.prune(&threshold);
        assert!(pruned.is_empty());
        assert!(map.get_record("key1").is_some());
    }

    #[test]
    fn prune_returns_empty_on_empty_map() {
        let (mut map, _) = make_map(1_000_000);
        let threshold = Timestamp {
            millis: u64::MAX,
            counter: 0,
            node_id: "x".to_string(),
        };
        let pruned = map.prune(&threshold);
        assert!(pruned.is_empty());
    }

    // ---- entries() and all_keys() ----

    #[test]
    fn entries_skips_tombstones() {
        let (mut map, _) = make_map(1_000_000);
        map.set("a", Value::Int(1), None);
        map.set("b", Value::Int(2), None);
        map.remove("b");

        let entries: Vec<_> = map.entries().collect();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, "a");
        assert_eq!(entries[0].1, &Value::Int(1));
    }

    #[test]
    fn entries_skips_expired_ttl() {
        let (mut map, time) = make_map(1_000_000);
        map.set("live", Value::Int(1), None);
        map.set("expired", Value::Int(2), Some(100));

        // Advance past TTL expiry
        time.store(1_000_200, AtomicOrdering::Relaxed);

        let entries: Vec<_> = map.entries().collect();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, "live");
    }

    #[test]
    fn all_keys_includes_tombstones() {
        let (mut map, _) = make_map(1_000_000);
        map.set("a", Value::Int(1), None);
        map.set("b", Value::Int(2), None);
        map.remove("b");

        let mut keys: Vec<&String> = map.all_keys().collect();
        keys.sort();
        assert_eq!(keys.len(), 2);
        assert_eq!(*keys[0], "a");
        assert_eq!(*keys[1], "b");
    }

    // ---- clear() ----

    #[test]
    fn clear_removes_all_data() {
        let (mut map, _) = make_map(1_000_000);
        map.set("a", Value::Int(1), None);
        map.set("b", Value::Int(2), None);
        map.clear();
        assert_eq!(map.size(), 0);
        assert_eq!(map.get("a"), None);
        assert_eq!(map.get("b"), None);
    }

    #[test]
    fn clear_resets_merkle_tree() {
        let (mut map, _) = make_map(1_000_000);
        map.set("key1", Value::Int(1), None);
        assert_ne!(map.merkle_tree().get_root_hash(), 0);

        map.clear();
        assert_eq!(map.merkle_tree().get_root_hash(), 0);
    }

    // ---- MerkleTree integration ----

    #[test]
    fn merkle_tree_updates_on_set() {
        let (mut map, _) = make_map(1_000_000);
        let hash_before = map.merkle_tree().get_root_hash();
        map.set("key1", Value::Int(1), None);
        let hash_after = map.merkle_tree().get_root_hash();
        assert_ne!(hash_before, hash_after);
    }

    #[test]
    fn merkle_tree_updates_on_remove() {
        let (mut map, _) = make_map(1_000_000);
        map.set("key1", Value::Int(1), None);
        let hash_after_set = map.merkle_tree().get_root_hash();
        map.remove("key1");
        let hash_after_remove = map.merkle_tree().get_root_hash();
        // Remove changes the record's timestamp, so hash should change
        assert_ne!(hash_after_set, hash_after_remove);
    }

    #[test]
    fn merkle_tree_updates_on_merge() {
        let (mut map, _) = make_map(1_000_000);
        let hash_before = map.merkle_tree().get_root_hash();

        let record = LWWRecord {
            value: Some(Value::Int(42)),
            timestamp: Timestamp {
                millis: 500,
                counter: 0,
                node_id: "remote".to_string(),
            },
            ttl_ms: None,
        };
        map.merge("key1", record);
        let hash_after = map.merkle_tree().get_root_hash();
        assert_ne!(hash_before, hash_after);
    }

    #[test]
    fn merkle_tree_prune_removes_entry() {
        let (mut map, _) = make_map(1_000_000);
        map.set("key1", Value::Int(1), None);
        let tombstone = map.remove("key1");

        let hash_before_prune = map.merkle_tree().get_root_hash();
        assert_ne!(hash_before_prune, 0);

        let threshold = Timestamp {
            millis: tombstone.timestamp.millis + 1000,
            counter: 0,
            node_id: "z".to_string(),
        };
        map.prune(&threshold);

        // After pruning the only entry, root hash goes to 0
        assert_eq!(map.merkle_tree().get_root_hash(), 0);
    }

    #[test]
    fn merkle_tree_hash_deterministic() {
        // Two maps with same operations should produce same merkle root
        let (mut map1, _) = make_map(1_000_000);
        let (mut map2, _) = make_map(1_000_000);

        // Use merge with explicit timestamps (set() would use HLC which may differ)
        let record = LWWRecord {
            value: Some(Value::String("hello".to_string())),
            timestamp: Timestamp {
                millis: 500,
                counter: 0,
                node_id: "node".to_string(),
            },
            ttl_ms: None,
        };
        map1.merge("key1", record.clone());
        map2.merge("key1", record);

        assert_eq!(
            map1.merkle_tree().get_root_hash(),
            map2.merkle_tree().get_root_hash()
        );
    }

    // ---- set() returns record ----

    #[test]
    fn set_returns_record_with_correct_fields() {
        let (mut map, _) = make_map(1_000_000);
        let record = map.set("key1", Value::Int(99), Some(5000));
        assert_eq!(record.value, Some(Value::Int(99)));
        assert_eq!(record.timestamp.millis, 1_000_000);
        assert_eq!(record.timestamp.node_id, "test-node");
        assert_eq!(record.ttl_ms, Some(5000));
    }

    // ---- remove() returns tombstone record ----

    #[test]
    fn remove_returns_tombstone_record() {
        let (mut map, _) = make_map(1_000_000);
        map.set("key1", Value::Int(1), None);
        let tombstone = map.remove("key1");
        assert_eq!(tombstone.value, None);
        assert_eq!(tombstone.ttl_ms, None);
        assert_eq!(tombstone.timestamp.node_id, "test-node");
    }

    // ---- Merge commutativity (manual spot check) ----

    #[test]
    fn merge_commutativity_spot_check() {
        let record_a = LWWRecord {
            value: Some(Value::String("A".to_string())),
            timestamp: Timestamp {
                millis: 100,
                counter: 0,
                node_id: "nodeA".to_string(),
            },
            ttl_ms: None,
        };
        let record_b = LWWRecord {
            value: Some(Value::String("B".to_string())),
            timestamp: Timestamp {
                millis: 200,
                counter: 0,
                node_id: "nodeB".to_string(),
            },
            ttl_ms: None,
        };

        // Order 1: A then B
        let (mut map1, _) = make_map(1_000_000);
        map1.merge("key", record_a.clone());
        map1.merge("key", record_b.clone());

        // Order 2: B then A
        let (mut map2, _) = make_map(1_000_000);
        map2.merge("key", record_b);
        map2.merge("key", record_a);

        // Both should have the same value (B wins)
        assert_eq!(map1.get("key"), map2.get("key"));
        assert_eq!(map1.get("key"), Some(&Value::String("B".to_string())));
    }

    // ---- Merge idempotence (manual spot check) ----

    #[test]
    fn merge_idempotence_spot_check() {
        let (mut map, _) = make_map(1_000_000);
        let record = LWWRecord {
            value: Some(Value::Int(42)),
            timestamp: Timestamp {
                millis: 500,
                counter: 0,
                node_id: "node".to_string(),
            },
            ttl_ms: None,
        };

        let changed1 = map.merge("key", record.clone());
        assert!(changed1);

        let hash_after_first = map.merkle_tree().get_root_hash();

        let changed2 = map.merge("key", record);
        assert!(!changed2);

        // State unchanged
        assert_eq!(map.merkle_tree().get_root_hash(), hash_after_first);
        assert_eq!(map.get("key"), Some(&Value::Int(42)));
    }

    // ---- MsgPack round-trip for LWWRecord<Value> ----

    #[test]
    fn lww_record_value_msgpack_roundtrip() {
        let record = LWWRecord {
            value: Some(Value::String("hello".to_string())),
            timestamp: Timestamp {
                millis: 1_700_000_000_000,
                counter: 42,
                node_id: "node-abc-123".to_string(),
            },
            ttl_ms: Some(5000),
        };
        let bytes = rmp_serde::to_vec(&record).expect("serialize LWWRecord<Value>");
        let decoded: LWWRecord<Value> =
            rmp_serde::from_slice(&bytes).expect("deserialize LWWRecord<Value>");
        assert_eq!(record, decoded);
    }

    #[test]
    fn lww_record_tombstone_msgpack_roundtrip() {
        let record: LWWRecord<Value> = LWWRecord {
            value: None,
            timestamp: Timestamp {
                millis: 999,
                counter: 0,
                node_id: "tomb".to_string(),
            },
            ttl_ms: None,
        };
        let bytes = rmp_serde::to_vec(&record).expect("serialize");
        let decoded: LWWRecord<Value> = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(record, decoded);
    }

    #[test]
    fn lww_record_all_value_variants_roundtrip() {
        let variants: Vec<LWWRecord<Value>> = vec![
            LWWRecord {
                value: Some(Value::Null),
                timestamp: Timestamp { millis: 1, counter: 0, node_id: "n".to_string() },
                ttl_ms: None,
            },
            LWWRecord {
                value: Some(Value::Bool(true)),
                timestamp: Timestamp { millis: 2, counter: 0, node_id: "n".to_string() },
                ttl_ms: None,
            },
            LWWRecord {
                value: Some(Value::Int(-42)),
                timestamp: Timestamp { millis: 3, counter: 0, node_id: "n".to_string() },
                ttl_ms: Some(1000),
            },
            LWWRecord {
                value: Some(Value::Float(3.14)),
                timestamp: Timestamp { millis: 4, counter: 0, node_id: "n".to_string() },
                ttl_ms: None,
            },
            LWWRecord {
                value: Some(Value::Bytes(vec![0xDE, 0xAD])),
                timestamp: Timestamp { millis: 5, counter: 0, node_id: "n".to_string() },
                ttl_ms: None,
            },
        ];

        for record in variants {
            let bytes = rmp_serde::to_vec(&record).expect("serialize");
            let decoded: LWWRecord<Value> = rmp_serde::from_slice(&bytes).expect("deserialize");
            assert_eq!(record, decoded);
        }
    }
}

/// Property-based tests using `proptest` for CRDT correctness verification.
#[cfg(test)]
mod proptests {
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
    use std::sync::Arc;

    use proptest::prelude::*;

    use super::*;
    use crate::hlc::ClockSource;
    use crate::Value;

    /// Deterministic clock source for proptest.
    struct FixedClock {
        time: Arc<AtomicU64>,
    }

    impl FixedClock {
        fn new(initial: u64) -> (Self, Arc<AtomicU64>) {
            let time = Arc::new(AtomicU64::new(initial));
            (Self { time: time.clone() }, time)
        }
    }

    impl ClockSource for FixedClock {
        fn now(&self) -> u64 {
            self.time.load(AtomicOrdering::Relaxed)
        }
    }

    /// Creates an `LWWMap<Value>` with a high fixed clock to avoid drift errors
    /// when merging records with arbitrary timestamps.
    fn make_proptest_map() -> LWWMap<Value> {
        let (clock, _) = FixedClock::new(u64::MAX / 2);
        let hlc = HLC::new("proptest-node".to_string(), Box::new(clock));
        LWWMap::new(hlc)
    }

    /// Strategy for generating arbitrary `Timestamp` values.
    fn arb_timestamp() -> impl Strategy<Value = Timestamp> {
        (1_u64..1_000_000_000_u64, 0_u32..1000_u32, "[a-z]{1,8}")
            .prop_map(|(millis, counter, node_id)| Timestamp {
                millis,
                counter,
                node_id,
            })
    }

    /// Strategy for generating arbitrary `Value` variants (non-recursive for simplicity).
    fn arb_value() -> impl Strategy<Value = Value> {
        prop_oneof![
            Just(Value::Null),
            any::<bool>().prop_map(Value::Bool),
            any::<i64>().prop_map(Value::Int),
            any::<f64>().prop_map(Value::Float),
            "[a-zA-Z0-9 ]{0,20}".prop_map(|s| Value::String(s)),
            proptest::collection::vec(any::<u8>(), 0..16).prop_map(Value::Bytes),
        ]
    }

    /// Strategy for generating `LWWRecord<Value>` (non-tombstone).
    fn arb_lww_record() -> impl Strategy<Value = LWWRecord<Value>> {
        (arb_value(), arb_timestamp()).prop_map(|(value, timestamp)| LWWRecord {
            value: Some(value),
            timestamp,
            ttl_ms: None,
        })
    }

    /// Strategy for generating `LWWRecord<Value>` that may be a tombstone.
    fn arb_lww_record_maybe_tombstone() -> impl Strategy<Value = LWWRecord<Value>> {
        (prop::option::of(arb_value()), arb_timestamp()).prop_map(|(value, timestamp)| {
            LWWRecord {
                value,
                timestamp,
                ttl_ms: None,
            }
        })
    }

    proptest! {
        /// Commutativity: merging R1 then R2 produces the same state as
        /// merging R2 then R1 for any two records on the same key.
        #[test]
        fn merge_is_commutative(
            r1 in arb_lww_record_maybe_tombstone(),
            r2 in arb_lww_record_maybe_tombstone(),
        ) {
            // Order 1: R1 then R2
            let mut map1 = make_proptest_map();
            map1.merge("k", r1.clone());
            map1.merge("k", r2.clone());

            // Order 2: R2 then R1
            let mut map2 = make_proptest_map();
            map2.merge("k", r2);
            map2.merge("k", r1);

            // Both maps should have the same record for key "k"
            let rec1 = map1.get_record("k");
            let rec2 = map2.get_record("k");
            prop_assert_eq!(rec1, rec2);
        }

        /// Idempotence: merging the same record twice does not change state.
        #[test]
        fn merge_is_idempotent(
            r in arb_lww_record_maybe_tombstone(),
        ) {
            let mut map = make_proptest_map();

            // First merge
            map.merge("k", r.clone());
            let record_after_first = map.get_record("k").cloned();
            let hash_after_first = map.merkle_tree().get_root_hash();

            // Second merge (same record)
            let changed = map.merge("k", r);
            let record_after_second = map.get_record("k").cloned();
            let hash_after_second = map.merkle_tree().get_root_hash();

            prop_assert!(!changed, "Second merge should not change state");
            prop_assert_eq!(record_after_first, record_after_second);
            prop_assert_eq!(hash_after_first, hash_after_second);
        }

        /// Convergence: merging multiple records in any order on the same key
        /// produces the same final state (tests 3 records in all 6 permutations).
        #[test]
        fn merge_convergence_three_records(
            r1 in arb_lww_record(),
            r2 in arb_lww_record(),
            r3 in arb_lww_record(),
        ) {
            let orders: Vec<Vec<LWWRecord<Value>>> = vec![
                vec![r1.clone(), r2.clone(), r3.clone()],
                vec![r1.clone(), r3.clone(), r2.clone()],
                vec![r2.clone(), r1.clone(), r3.clone()],
                vec![r2.clone(), r3.clone(), r1.clone()],
                vec![r3.clone(), r1.clone(), r2.clone()],
                vec![r3.clone(), r2.clone(), r1.clone()],
            ];

            let mut results = Vec::new();
            for order in &orders {
                let mut map = make_proptest_map();
                for record in order {
                    map.merge("k", record.clone());
                }
                results.push(map.get_record("k").cloned());
            }

            // All permutations should produce the same final record
            for (i, result) in results.iter().enumerate().skip(1) {
                prop_assert_eq!(
                    &results[0], result,
                    "Permutation {} diverged from permutation 0",
                    i
                );
            }
        }

        /// `LWWRecord<Value>` round-trip through MsgPack preserves data.
        #[test]
        fn lww_record_msgpack_roundtrip_proptest(
            r in arb_lww_record_maybe_tombstone(),
        ) {
            let bytes = rmp_serde::to_vec(&r).expect("serialize");
            let decoded: LWWRecord<Value> = rmp_serde::from_slice(&bytes).expect("deserialize");
            prop_assert_eq!(r, decoded);
        }

        /// The winner of merge is always the record with the highest timestamp.
        #[test]
        fn merge_winner_has_highest_timestamp(
            r1 in arb_lww_record(),
            r2 in arb_lww_record(),
        ) {
            let mut map = make_proptest_map();
            map.merge("k", r1.clone());
            map.merge("k", r2.clone());

            let winner = map.get_record("k").expect("must have record");
            let expected_winner = if r1.timestamp >= r2.timestamp { &r1 } else { &r2 };
            prop_assert_eq!(winner, expected_winner);
        }

        /// Convergence for multiple keys: merging records for different keys
        /// in different orders produces identical maps.
        #[test]
        fn multi_key_convergence(
            records in proptest::collection::vec(
                ("[a-z]{1,4}", arb_lww_record()),
                1..5
            ),
        ) {
            // Group records by key for comparison
            let mut expected: BTreeMap<String, &LWWRecord<Value>> = BTreeMap::new();
            for (key, record) in &records {
                match expected.get(key.as_str()) {
                    None => { expected.insert(key.clone(), record); },
                    Some(existing) => {
                        if record.timestamp > existing.timestamp {
                            expected.insert(key.clone(), record);
                        }
                    }
                }
            }

            // Forward order
            let mut map1 = make_proptest_map();
            for (key, record) in &records {
                map1.merge(key.clone(), record.clone());
            }

            // Reverse order
            let mut map2 = make_proptest_map();
            for (key, record) in records.iter().rev() {
                map2.merge(key.clone(), record.clone());
            }

            // Both should have the same records for all keys
            for (key, expected_rec) in &expected {
                let rec1 = map1.get_record(key);
                let rec2 = map2.get_record(key);
                prop_assert_eq!(rec1, Some(*expected_rec), "Key {} mismatch in map1", key);
                prop_assert_eq!(rec2, Some(*expected_rec), "Key {} mismatch in map2", key);
            }
        }
    }
}
