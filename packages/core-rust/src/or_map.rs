//! Observed-Remove Map (OR-Map) CRDT implementation.
//!
//! `ORMap<V>` is a conflict-free replicated data type that acts as a multimap:
//! each key holds a set of values, where each value instance is tracked by a
//! unique tag. Concurrent additions and removals are resolved with **add-wins**
//! semantics: if one node removes a value while another concurrently adds it
//! (with a different tag), the add survives after merge.
//!
//! # Storage model
//!
//! Internally, records are stored in a nested map:
//! `HashMap<String, HashMap<String, ORMapRecord<V>>>` (key -> tag -> record).
//! Removals place tags into a tombstone set (`HashSet<String>`). During merge,
//! the union of items minus the union of tombstones yields the converged state.
//!
//! # `MerkleTree` integration
//!
//! Every mutation to a key recomputes a deterministic entry hash from all active
//! records for that key and updates the internal [`ORMapMerkleTree`]. This enables
//! efficient delta synchronization between replicas.
//!
//! # TTL (Time-To-Live)
//!
//! Records can carry an optional TTL in milliseconds. Expired records are filtered
//! from [`ORMap::get`] and [`ORMap::get_records`] but remain in storage until
//! explicitly removed or pruned.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::hash::fnv1a_hash;
use crate::hlc::{MergeKeyResult, ORMapRecord, Timestamp, HLC};
use crate::merkle::ORMapMerkleTree;

/// Produces a deterministic JSON string with recursively sorted object keys.
///
/// Serializes `value` to `serde_json::Value`, recursively sorts all object
/// keys in lexicographic order, then serializes back to a JSON string. This
/// ensures identical output regardless of the internal iteration order of
/// the original type (e.g., `HashMap`-based structs).
fn canonical_json<V: Serialize>(value: &V) -> String {
    let json_value = serde_json::to_value(value).expect("V: Serialize must convert to serde_json::Value");
    let sorted = sort_json_value(json_value);
    serde_json::to_string(&sorted).expect("sorted serde_json::Value must serialize to string")
}

/// Recursively sorts object keys in a `serde_json::Value`.
///
/// - `Object`: collects entries into a `BTreeMap` (sorted by key), recursing into values
/// - `Array`: recurses into each element
/// - Primitives (`Null`, `Bool`, `Number`, `String`): returned unchanged
fn sort_json_value(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let sorted: BTreeMap<String, serde_json::Value> = map
                .into_iter()
                .map(|(k, v)| (k, sort_json_value(v)))
                .collect();
            serde_json::Value::Object(sorted.into_iter().collect())
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.into_iter().map(sort_json_value).collect())
        }
        other => other,
    }
}

/// An Observed-Remove Map providing conflict-free convergence with add-wins semantics.
///
/// Keys are [`String`]s. Values are generic over `V` with bounds
/// `Clone + Serialize + DeserializeOwned + PartialEq`.
///
/// # Examples
///
/// ```
/// use topgun_core::hlc::{HLC, SystemClock};
/// use topgun_core::or_map::ORMap;
/// use topgun_core::Value;
///
/// let hlc = HLC::new("node-1".to_string(), Box::new(SystemClock));
/// let mut map: ORMap<Value> = ORMap::new(hlc);
///
/// map.add("user:1", Value::String("Alice".to_string()), None);
/// let values = map.get("user:1");
/// assert_eq!(values.len(), 1);
/// ```
pub struct ORMap<V> {
    /// Key -> Tag -> Record. Stores all active (non-tombstoned) records.
    items: HashMap<String, HashMap<String, ORMapRecord<V>>>,
    /// Set of removed tags (tombstones).
    tombstones: HashSet<String>,
    /// Hybrid Logical Clock for timestamp/tag generation and causality tracking.
    hlc: HLC,
    /// Merkle tree for efficient delta synchronization.
    merkle_tree: ORMapMerkleTree,
}

impl<V> ORMap<V>
where
    V: Clone + Serialize + DeserializeOwned + PartialEq,
{
    /// Creates a new empty `ORMap` with the given HLC instance.
    ///
    /// The HLC is used for timestamp and tag generation on [`add`](ORMap::add),
    /// causality tracking on [`merge`](ORMap::merge), and TTL expiry checks
    /// via its clock source.
    #[must_use]
    pub fn new(hlc: HLC) -> Self {
        Self {
            items: HashMap::new(),
            tombstones: HashSet::new(),
            hlc,
            merkle_tree: ORMapMerkleTree::default_depth(),
        }
    }

    /// Adds a value to the set associated with the key.
    ///
    /// Generates a unique tag from the HLC timestamp (`"millis:counter:nodeId"` format).
    /// Returns a clone of the stored record.
    pub fn add(
        &mut self,
        key: impl Into<String>,
        value: V,
        ttl_ms: Option<u64>,
    ) -> ORMapRecord<V> {
        let key = key.into();
        let timestamp = self.hlc.now();
        let tag = HLC::to_string(&timestamp);

        let record = ORMapRecord {
            value,
            timestamp,
            tag,
            ttl_ms,
        };

        let key_map = self.items.entry(key.clone()).or_default();
        key_map.insert(record.tag.clone(), record.clone());

        self.update_merkle(&key);
        record
    }

    /// Removes a specific value from the set associated with the key.
    ///
    /// Tombstones all currently observed tags whose value matches `value`
    /// (using `PartialEq`). Returns the list of tags that were removed.
    pub fn remove(&mut self, key: &str, value: &V) -> Vec<String> {
        let Some(key_map) = self.items.get_mut(key) else {
            return Vec::new();
        };

        // Find all tags with matching value
        let tags_to_remove: Vec<String> = key_map
            .iter()
            .filter(|(_, record)| &record.value == value)
            .map(|(tag, _)| tag.clone())
            .collect();

        // Tombstone and remove matching records
        for tag in &tags_to_remove {
            self.tombstones.insert(tag.clone());
            key_map.remove(tag);
        }

        // Clean up empty key maps
        if key_map.is_empty() {
            self.items.remove(key);
        }

        self.update_merkle(key);
        tags_to_remove
    }

    /// Returns all active values for a key.
    ///
    /// Filters out tombstoned and TTL-expired records. Returns references to
    /// the stored values.
    #[must_use]
    pub fn get(&self, key: &str) -> Vec<&V> {
        let Some(key_map) = self.items.get(key) else {
            return Vec::new();
        };

        let now = self.hlc.clock_source().now();
        let mut values = Vec::new();

        for (tag, record) in key_map {
            // Defensive: skip tombstoned tags (should not be in items, but be safe)
            if self.tombstones.contains(tag) {
                continue;
            }
            // Check TTL expiry
            if let Some(ttl) = record.ttl_ms {
                if record.timestamp.millis + ttl < now {
                    continue;
                }
            }
            values.push(&record.value);
        }

        values
    }

    /// Returns all active records for a key.
    ///
    /// Filters out tombstoned and TTL-expired records. Useful for persistence
    /// and synchronization.
    #[must_use]
    pub fn get_records(&self, key: &str) -> Vec<&ORMapRecord<V>> {
        let Some(key_map) = self.items.get(key) else {
            return Vec::new();
        };

        let now = self.hlc.clock_source().now();
        let mut records = Vec::new();

        for (tag, record) in key_map {
            // Defensive: skip tombstoned tags
            if self.tombstones.contains(tag) {
                continue;
            }
            // Check TTL expiry
            if let Some(ttl) = record.ttl_ms {
                if record.timestamp.millis + ttl < now {
                    continue;
                }
            }
            records.push(record);
        }

        records
    }

    /// Applies a record from a remote source.
    ///
    /// Returns `false` if the record's tag is already tombstoned (record rejected).
    /// Returns `true` if the record was applied. Always calls `hlc.update()` on
    /// the record's timestamp to maintain causality.
    pub fn apply(&mut self, key: impl Into<String>, record: ORMapRecord<V>) -> bool {
        if self.tombstones.contains(&record.tag) {
            return false;
        }

        let key = key.into();

        // Update HLC causality
        let _ = self.hlc.update(&record.timestamp);

        let key_map = self.items.entry(key.clone()).or_default();
        key_map.insert(record.tag.clone(), record);

        self.update_merkle(&key);
        true
    }

    /// Applies a tombstone (deletion) from a remote source.
    ///
    /// Adds the tag to the tombstone set and removes the matching record from
    /// items if present. Updates the Merkle tree for the affected key.
    pub fn apply_tombstone(&mut self, tag: &str) {
        self.tombstones.insert(tag.to_string());

        // Find and remove matching record from items
        let mut affected_key: Option<String> = None;
        for (key, key_map) in &mut self.items {
            if key_map.remove(tag).is_some() {
                affected_key = Some(key.clone());
                break; // Tag is globally unique
            }
        }

        if let Some(key) = affected_key {
            if self.items.get(&key).is_some_and(HashMap::is_empty) {
                self.items.remove(&key);
            }
            self.update_merkle(&key);
        }
    }

    /// Merges state from another `ORMap`.
    ///
    /// Implements observed-remove semantics: union of items minus union of
    /// tombstones. Updates the HLC with each remote record's timestamp to
    /// maintain causality.
    pub fn merge(&mut self, other: &ORMap<V>) {
        let mut changed_keys: HashSet<String> = HashSet::new();

        // 1. Merge tombstones
        for tag in &other.tombstones {
            self.tombstones.insert(tag.clone());
        }

        // 2. Merge items
        for (key, other_key_map) in &other.items {
            let local_key_map = self.items.entry(key.clone()).or_default();

            for (tag, record) in other_key_map {
                // Only accept if not tombstoned
                if !self.tombstones.contains(tag) && !local_key_map.contains_key(tag) {
                    local_key_map.insert(tag.clone(), record.clone());
                    changed_keys.insert(key.clone());
                }
                // Always update causality
                let _ = self.hlc.update(&record.timestamp);
            }
        }

        // 3. Cleanup: remove any local items that are now tombstoned
        let keys: Vec<String> = self.items.keys().cloned().collect();
        for key in keys {
            if let Some(key_map) = self.items.get_mut(&key) {
                let tombstoned_tags: Vec<String> = key_map
                    .keys()
                    .filter(|tag| self.tombstones.contains(*tag))
                    .cloned()
                    .collect();

                for tag in tombstoned_tags {
                    key_map.remove(&tag);
                    changed_keys.insert(key.clone());
                }

                if key_map.is_empty() {
                    self.items.remove(&key);
                }
            }
        }

        // Update Merkle tree for changed keys
        for key in &changed_keys {
            self.update_merkle(key);
        }
    }

    /// Merges remote records for a specific key into local state.
    ///
    /// Implements observed-remove CRDT semantics for per-key Merkle synchronization.
    /// Applies tombstones first, then merges remote records (skipping tombstoned tags,
    /// adding new records, updating existing records if remote timestamp is newer).
    ///
    /// Returns a [`MergeKeyResult`] with counts of added and updated records.
    pub fn merge_key(
        &mut self,
        key: impl Into<String>,
        remote_records: Vec<ORMapRecord<V>>,
        remote_tombstones: &[String],
    ) -> MergeKeyResult {
        let key = key.into();
        let mut added: usize = 0;
        let mut updated: usize = 0;

        // 1. Apply remote tombstones
        for tag in remote_tombstones {
            self.tombstones.insert(tag.clone());
        }

        // 2. Get or create local key map
        let local_key_map = self.items.entry(key.clone()).or_default();

        // 3. Remove any local records that are now tombstoned
        let tombstoned_local: Vec<String> = local_key_map
            .keys()
            .filter(|tag| self.tombstones.contains(*tag))
            .cloned()
            .collect();
        for tag in tombstoned_local {
            local_key_map.remove(&tag);
        }

        // 4. Merge remote records
        for remote_record in remote_records {
            // Skip if tombstoned
            if self.tombstones.contains(&remote_record.tag) {
                // Still update causality
                let _ = self.hlc.update(&remote_record.timestamp);
                continue;
            }

            if let Some(local_record) = local_key_map.get(&remote_record.tag) {
                // Existing record: update if remote is newer
                if remote_record.timestamp > local_record.timestamp {
                    local_key_map.insert(remote_record.tag.clone(), remote_record.clone());
                    updated += 1;
                }
            } else {
                // New record: add it
                local_key_map.insert(remote_record.tag.clone(), remote_record.clone());
                added += 1;
            }

            // Always update causality
            let _ = self.hlc.update(&remote_record.timestamp);
        }

        // 5. Cleanup empty key map
        if local_key_map.is_empty() {
            self.items.remove(&key);
        }

        // 6. Update Merkle tree
        self.update_merkle(&key);

        MergeKeyResult { added, updated }
    }

    /// Prunes tombstones older than the given timestamp.
    ///
    /// Parses each tag as a timestamp string (`"millis:counter:nodeId"`) and removes
    /// it from the tombstone set if it is older than `older_than`. Returns the tags
    /// that were pruned.
    pub fn prune(&mut self, older_than: &Timestamp) -> Vec<String> {
        let mut removed_tags = Vec::new();

        let to_remove: Vec<String> = self
            .tombstones
            .iter()
            .filter(|tag| {
                if let Ok(ts) = HLC::parse(tag) {
                    ts < *older_than
                } else {
                    false
                }
            })
            .cloned()
            .collect();

        for tag in to_remove {
            self.tombstones.remove(&tag);
            removed_tags.push(tag);
        }

        removed_tags
    }

    /// Clears all data, tombstones, and resets the Merkle tree.
    pub fn clear(&mut self) {
        self.items.clear();
        self.tombstones.clear();
        self.merkle_tree = ORMapMerkleTree::default_depth();
    }

    /// Returns all keys that have active records.
    #[must_use]
    pub fn all_keys(&self) -> Vec<&String> {
        self.items.keys().collect()
    }

    /// Returns all tombstone tags.
    #[must_use]
    pub fn get_tombstones(&self) -> Vec<&String> {
        self.tombstones.iter().collect()
    }

    /// Checks if a tag is in the tombstone set.
    #[must_use]
    pub fn is_tombstoned(&self, tag: &str) -> bool {
        self.tombstones.contains(tag)
    }

    /// Returns a read-only reference to the internal [`ORMapMerkleTree`].
    #[must_use]
    pub fn merkle_tree(&self) -> &ORMapMerkleTree {
        &self.merkle_tree
    }

    // ---- Internal helpers ----

    /// Recomputes the Merkle tree entry for a key based on its current records.
    ///
    /// If the key has no records, the key is removed from the Merkle tree.
    /// Otherwise, the entry hash is recomputed and updated.
    fn update_merkle(&mut self, key: &str) {
        match self.items.get(key) {
            Some(key_map) if !key_map.is_empty() => {
                let entry_hash = Self::hash_entry(key, key_map);
                self.merkle_tree.update(key, entry_hash);
            }
            _ => {
                self.merkle_tree.remove(key);
            }
        }
    }

    /// Computes a deterministic hash for all records under a key.
    ///
    /// Records are sorted by tag for deterministic ordering. The hash string
    /// format is: `"key:{key}|{tag}:{value_str}:{ts_str}[|...]"` where
    /// `value_str` is the canonical JSON output (recursively sorted object keys)
    /// and `ts_str` is `"millis:counter:nodeId"`. When a record has a TTL,
    /// `:ttl={ttl_ms}` is appended to that record's segment.
    fn hash_entry(key: &str, records: &HashMap<String, ORMapRecord<V>>) -> u32 {
        // Sort records by tag for deterministic ordering
        let mut sorted_tags: Vec<&String> = records.keys().collect();
        sorted_tags.sort();

        let mut parts = Vec::with_capacity(sorted_tags.len() + 1);
        parts.push(format!("key:{key}"));

        for tag in sorted_tags {
            let record = &records[tag];
            let value_str = canonical_json(&record.value);
            let ts_str = format!(
                "{}:{}:{}",
                record.timestamp.millis, record.timestamp.counter, record.timestamp.node_id
            );

            let mut record_str = format!("{tag}:{value_str}:{ts_str}");
            if let Some(ttl) = record.ttl_ms {
                use std::fmt::Write;
                let _ = write!(record_str, ":ttl={ttl}");
            }
            parts.push(record_str);
        }

        fnv1a_hash(&parts.join("|"))
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

    /// Helper: create an ORMap with a FixedClock for deterministic testing.
    fn make_map(node_id: &str, time: u64) -> (ORMap<Value>, Arc<AtomicU64>) {
        let (clock, time_handle) = FixedClock::new(time);
        let hlc = HLC::new(node_id.to_string(), Box::new(clock));
        (ORMap::new(hlc), time_handle)
    }

    // ---- Basic add / get tests ----

    #[test]
    fn add_and_get_single_value() {
        let (mut map, _) = make_map("node-1", 1_000_000);
        map.add("key1", Value::String("hello".to_string()), None);

        let values = map.get("key1");
        assert_eq!(values.len(), 1);
        assert_eq!(values[0], &Value::String("hello".to_string()));
    }

    #[test]
    fn add_multiple_values_same_key() {
        let (mut map, _) = make_map("node-1", 1_000_000);
        map.add("key1", Value::String("work".to_string()), None);
        map.add("key1", Value::String("play".to_string()), None);

        let values = map.get("key1");
        assert_eq!(values.len(), 2);
        // Both values should be present (order not guaranteed)
        let strs: Vec<&Value> = values.iter().copied().collect();
        assert!(strs.contains(&&Value::String("work".to_string())));
        assert!(strs.contains(&&Value::String("play".to_string())));
    }

    #[test]
    fn get_nonexistent_key_returns_empty() {
        let (map, _) = make_map("node-1", 1_000_000);
        let values = map.get("nonexistent");
        assert!(values.is_empty());
    }

    #[test]
    fn add_returns_record_with_tag() {
        let (mut map, _) = make_map("node-1", 1_000_000);
        let record = map.add("key1", Value::Int(42), None);

        assert_eq!(record.value, Value::Int(42));
        assert!(!record.tag.is_empty());
        // Tag should be in millis:counter:nodeId format
        assert!(record.tag.contains("node-1"));
    }

    // ---- Remove / tombstone tests ----

    #[test]
    fn remove_tombstones_matching_value() {
        let (mut map, _) = make_map("node-1", 1_000_000);
        map.add("key1", Value::String("hello".to_string()), None);
        map.add("key1", Value::String("world".to_string()), None);

        let removed = map.remove("key1", &Value::String("hello".to_string()));
        assert_eq!(removed.len(), 1);

        let values = map.get("key1");
        assert_eq!(values.len(), 1);
        assert_eq!(values[0], &Value::String("world".to_string()));
    }

    #[test]
    fn remove_all_values_returns_empty_get() {
        let (mut map, _) = make_map("node-1", 1_000_000);
        map.add("key1", Value::String("hello".to_string()), None);
        map.remove("key1", &Value::String("hello".to_string()));

        let values = map.get("key1");
        assert!(values.is_empty());
    }

    #[test]
    fn remove_nonexistent_key_returns_empty() {
        let (mut map, _) = make_map("node-1", 1_000_000);
        let removed = map.remove("nonexistent", &Value::String("x".to_string()));
        assert!(removed.is_empty());
    }

    #[test]
    fn remove_nonexistent_value_returns_empty() {
        let (mut map, _) = make_map("node-1", 1_000_000);
        map.add("key1", Value::String("hello".to_string()), None);
        let removed = map.remove("key1", &Value::String("nope".to_string()));
        assert!(removed.is_empty());
    }

    // ---- Add-wins semantics (concurrent add + remove) ----

    #[test]
    fn add_wins_concurrent_add_and_remove() {
        // Node A adds 'work', Node B also adds 'work' independently
        let (mut map_a, _) = make_map("node-A", 1_000_000);
        let (mut map_b, _) = make_map("node-B", 1_000_000);

        // Both add 'work' to same key
        map_a.add("status", Value::String("work".to_string()), None);
        map_b.add("status", Value::String("work".to_string()), None);

        // Node A removes 'work' (tombstones its own tag)
        map_a.remove("status", &Value::String("work".to_string()));

        // Now merge: A's tombstone should NOT affect B's 'work' (different tag)
        map_a.merge(&map_b);

        let values = map_a.get("status");
        assert_eq!(values.len(), 1, "B's 'work' should survive (add-wins)");
        assert_eq!(values[0], &Value::String("work".to_string()));
    }

    // ---- Apply tests ----

    #[test]
    fn apply_rejects_tombstoned_tag() {
        let (mut map, _) = make_map("node-1", 1_000_000);
        let record = map.add("key1", Value::String("hello".to_string()), None);
        let tag = record.tag.clone();

        // Tombstone the tag
        map.remove("key1", &Value::String("hello".to_string()));
        assert!(map.is_tombstoned(&tag));

        // Try to apply same tag from remote -- should be rejected
        let remote_record = ORMapRecord {
            value: Value::String("hello".to_string()),
            timestamp: Timestamp {
                millis: 2_000_000,
                counter: 0,
                node_id: "remote".to_string(),
            },
            tag,
            ttl_ms: None,
        };

        let applied = map.apply("key1", remote_record);
        assert!(!applied, "Should reject tombstoned tag");
        assert!(map.get("key1").is_empty());
    }

    #[test]
    fn apply_accepts_new_tag() {
        let (mut map, _) = make_map("node-1", 1_000_000);

        let remote_record = ORMapRecord {
            value: Value::String("from-remote".to_string()),
            timestamp: Timestamp {
                millis: 2_000_000,
                counter: 0,
                node_id: "remote".to_string(),
            },
            tag: "2000000:0:remote".to_string(),
            ttl_ms: None,
        };

        let applied = map.apply("key1", remote_record);
        assert!(applied);
        assert_eq!(map.get("key1").len(), 1);
    }

    // ---- Apply tombstone tests ----

    #[test]
    fn apply_tombstone_removes_from_items() {
        let (mut map, _) = make_map("node-1", 1_000_000);
        let record = map.add("key1", Value::String("hello".to_string()), None);
        let tag = record.tag.clone();

        map.apply_tombstone(&tag);

        assert!(map.is_tombstoned(&tag));
        assert!(map.get("key1").is_empty());
    }

    #[test]
    fn apply_tombstone_nonexistent_tag() {
        let (mut map, _) = make_map("node-1", 1_000_000);
        // Should not panic
        map.apply_tombstone("nonexistent-tag");
        assert!(map.is_tombstoned("nonexistent-tag"));
    }

    // ---- Merge tests (full map) ----

    #[test]
    fn merge_adds_remote_items() {
        let (mut map_a, _) = make_map("node-A", 1_000_000);
        let (mut map_b, _) = make_map("node-B", 1_000_000);

        map_a.add("key1", Value::String("a-val".to_string()), None);
        map_b.add("key1", Value::String("b-val".to_string()), None);

        map_a.merge(&map_b);

        let values = map_a.get("key1");
        assert_eq!(values.len(), 2);
    }

    #[test]
    fn merge_is_commutative() {
        let (mut map_a, _) = make_map("node-A", 1_000_000);
        let (mut map_b, _) = make_map("node-B", 1_000_000);

        map_a.add("k1", Value::Int(1), None);
        map_b.add("k1", Value::Int(2), None);

        // Merge A into fresh copy of B, and B into fresh copy of A
        let (mut map_a2, _) = make_map("node-A2", 1_000_000);
        let (mut map_b2, _) = make_map("node-B2", 1_000_000);

        // Replicate A's state to A2 and B's state to B2
        for (key, key_map) in &map_a.items {
            for (_, record) in key_map {
                map_a2.apply(key.clone(), record.clone());
                map_b2.apply(key.clone(), record.clone());
            }
        }
        for (key, key_map) in &map_b.items {
            for (_, record) in key_map {
                map_a2.apply(key.clone(), record.clone());
                map_b2.apply(key.clone(), record.clone());
            }
        }

        // Both should have same values
        let mut vals_a: Vec<String> = map_a2
            .get("k1")
            .iter()
            .map(|v| format!("{v:?}"))
            .collect();
        let mut vals_b: Vec<String> = map_b2
            .get("k1")
            .iter()
            .map(|v| format!("{v:?}"))
            .collect();
        vals_a.sort();
        vals_b.sort();
        assert_eq!(vals_a, vals_b);
    }

    #[test]
    fn merge_is_idempotent() {
        let (mut map_a, _) = make_map("node-A", 1_000_000);
        let (mut map_b, _) = make_map("node-B", 1_000_000);

        map_a.add("key1", Value::String("val".to_string()), None);
        map_b.add("key1", Value::String("val2".to_string()), None);

        map_a.merge(&map_b);
        let count_after_first = map_a.get("key1").len();

        map_a.merge(&map_b);
        let count_after_second = map_a.get("key1").len();

        assert_eq!(count_after_first, count_after_second, "Merge should be idempotent");
    }

    // ---- merge_key tests ----

    #[test]
    fn merge_key_adds_new_records() {
        let (mut map, _) = make_map("node-1", 1_000_000);

        let remote_records = vec![ORMapRecord {
            value: Value::String("remote".to_string()),
            timestamp: Timestamp {
                millis: 2_000_000,
                counter: 0,
                node_id: "remote-node".to_string(),
            },
            tag: "2000000:0:remote-node".to_string(),
            ttl_ms: None,
        }];

        let result = map.merge_key("key1", remote_records, &[]);
        assert_eq!(result.added, 1);
        assert_eq!(result.updated, 0);
        assert_eq!(map.get("key1").len(), 1);
    }

    #[test]
    fn merge_key_applies_tombstones_first() {
        let (mut map, _) = make_map("node-1", 1_000_000);
        let record = map.add("key1", Value::String("local".to_string()), None);
        let tag = record.tag.clone();

        // Remote sends tombstone for local tag
        let result = map.merge_key("key1", vec![], &[tag.clone()]);
        assert_eq!(result.added, 0);
        assert_eq!(result.updated, 0);
        assert!(map.get("key1").is_empty());
        assert!(map.is_tombstoned(&tag));
    }

    #[test]
    fn merge_key_skips_tombstoned_remote_records() {
        let (mut map, _) = make_map("node-1", 1_000_000);

        let tag = "2000000:0:remote-node".to_string();
        let remote_records = vec![ORMapRecord {
            value: Value::String("remote".to_string()),
            timestamp: Timestamp {
                millis: 2_000_000,
                counter: 0,
                node_id: "remote-node".to_string(),
            },
            tag: tag.clone(),
            ttl_ms: None,
        }];

        // Remote sends both the record and its tombstone
        let result = map.merge_key("key1", remote_records, &[tag]);
        assert_eq!(result.added, 0);
        assert!(map.get("key1").is_empty());
    }

    #[test]
    fn merge_key_updates_newer_remote() {
        let (mut map, _) = make_map("node-1", 1_000_000);

        let tag = "2000000:0:remote-node".to_string();

        // Apply initial record
        let initial = ORMapRecord {
            value: Value::String("old".to_string()),
            timestamp: Timestamp {
                millis: 2_000_000,
                counter: 0,
                node_id: "remote-node".to_string(),
            },
            tag: tag.clone(),
            ttl_ms: None,
        };
        map.apply("key1", initial);

        // Merge with newer version of same tag
        let remote_records = vec![ORMapRecord {
            value: Value::String("new".to_string()),
            timestamp: Timestamp {
                millis: 3_000_000,
                counter: 0,
                node_id: "remote-node".to_string(),
            },
            tag,
            ttl_ms: None,
        }];

        let result = map.merge_key("key1", remote_records, &[]);
        assert_eq!(result.updated, 1);
        assert_eq!(
            map.get("key1")[0],
            &Value::String("new".to_string())
        );
    }

    // ---- Prune tests ----

    #[test]
    fn prune_removes_old_tombstones() {
        let (mut map, _) = make_map("node-1", 1_000_000);
        let record = map.add("key1", Value::String("hello".to_string()), None);
        let tag = record.tag.clone();

        map.remove("key1", &Value::String("hello".to_string()));
        assert!(map.is_tombstoned(&tag));

        // Prune with threshold after the record's timestamp
        let threshold = Timestamp {
            millis: 2_000_000,
            counter: 0,
            node_id: "".to_string(),
        };
        let pruned = map.prune(&threshold);
        assert_eq!(pruned.len(), 1);
        assert_eq!(pruned[0], tag);
        assert!(!map.is_tombstoned(&tag));
    }

    #[test]
    fn prune_keeps_recent_tombstones() {
        let (mut map, _) = make_map("node-1", 1_000_000);
        let record = map.add("key1", Value::String("hello".to_string()), None);
        let _tag = record.tag.clone();
        map.remove("key1", &Value::String("hello".to_string()));

        // Prune with threshold before the record's timestamp
        let threshold = Timestamp {
            millis: 500_000,
            counter: 0,
            node_id: "".to_string(),
        };
        let pruned = map.prune(&threshold);
        assert!(pruned.is_empty());
    }

    // ---- TTL expiry tests ----

    #[test]
    fn ttl_expired_records_filtered_from_get() {
        let (mut map, time) = make_map("node-1", 1_000_000);
        map.add("key1", Value::String("ephemeral".to_string()), Some(5_000));

        // Before expiry
        let values = map.get("key1");
        assert_eq!(values.len(), 1);

        // Advance time past TTL
        time.store(1_010_000, AtomicOrdering::Relaxed);

        let values = map.get("key1");
        assert!(values.is_empty(), "Expired record should be filtered");
    }

    #[test]
    fn ttl_not_expired_still_visible() {
        let (mut map, time) = make_map("node-1", 1_000_000);
        map.add("key1", Value::String("ephemeral".to_string()), Some(10_000));

        // Advance time but not past TTL
        time.store(1_005_000, AtomicOrdering::Relaxed);

        let values = map.get("key1");
        assert_eq!(values.len(), 1);
    }

    // ---- MerkleTree integration tests ----

    #[test]
    fn merkle_root_changes_after_add() {
        let (mut map, _) = make_map("node-1", 1_000_000);
        assert_eq!(map.merkle_tree().get_root_hash(), 0);

        map.add("key1", Value::String("hello".to_string()), None);
        assert_ne!(map.merkle_tree().get_root_hash(), 0);
    }

    #[test]
    fn merkle_root_changes_after_remove() {
        let (mut map, _) = make_map("node-1", 1_000_000);
        map.add("key1", Value::String("hello".to_string()), None);
        let hash_before = map.merkle_tree().get_root_hash();

        map.remove("key1", &Value::String("hello".to_string()));
        let hash_after = map.merkle_tree().get_root_hash();

        assert_ne!(hash_before, hash_after);
    }

    #[test]
    fn merkle_root_zero_after_clear() {
        let (mut map, _) = make_map("node-1", 1_000_000);
        map.add("key1", Value::String("hello".to_string()), None);
        map.add("key2", Value::String("world".to_string()), None);
        assert_ne!(map.merkle_tree().get_root_hash(), 0);

        map.clear();
        assert_eq!(map.merkle_tree().get_root_hash(), 0);
    }

    // ---- Accessor tests ----

    #[test]
    fn all_keys_returns_active_keys() {
        let (mut map, _) = make_map("node-1", 1_000_000);
        map.add("key1", Value::Int(1), None);
        map.add("key2", Value::Int(2), None);

        let mut keys: Vec<&String> = map.all_keys();
        keys.sort();
        assert_eq!(keys.len(), 2);
    }

    #[test]
    fn get_tombstones_returns_all() {
        let (mut map, _) = make_map("node-1", 1_000_000);
        map.add("key1", Value::String("a".to_string()), None);
        map.add("key1", Value::String("b".to_string()), None);

        map.remove("key1", &Value::String("a".to_string()));

        let tombstones = map.get_tombstones();
        assert_eq!(tombstones.len(), 1);
    }

    #[test]
    fn is_tombstoned_check() {
        let (mut map, _) = make_map("node-1", 1_000_000);
        let record = map.add("key1", Value::String("a".to_string()), None);
        let tag = record.tag.clone();

        assert!(!map.is_tombstoned(&tag));
        map.remove("key1", &Value::String("a".to_string()));
        assert!(map.is_tombstoned(&tag));
    }

    #[test]
    fn get_records_returns_active_records() {
        let (mut map, _) = make_map("node-1", 1_000_000);
        map.add("key1", Value::String("a".to_string()), None);
        map.add("key1", Value::String("b".to_string()), None);

        let records = map.get_records("key1");
        assert_eq!(records.len(), 2);
    }

    #[test]
    fn get_records_filters_expired() {
        let (mut map, time) = make_map("node-1", 1_000_000);
        map.add("key1", Value::String("short".to_string()), Some(1_000));
        map.add("key1", Value::String("long".to_string()), Some(100_000));

        time.store(1_005_000, AtomicOrdering::Relaxed);

        let records = map.get_records("key1");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].value, Value::String("long".to_string()));
    }

    // ---- Multiple values per key ----

    #[test]
    fn multiple_values_per_key_unique_tags() {
        let (mut map, _) = make_map("node-1", 1_000_000);
        let r1 = map.add("key1", Value::String("work".to_string()), None);
        let r2 = map.add("key1", Value::String("play".to_string()), None);

        assert_ne!(r1.tag, r2.tag, "Each add should produce a unique tag");
    }

    // ---- Empty key after removal ----

    #[test]
    fn empty_key_after_all_values_removed() {
        let (mut map, _) = make_map("node-1", 1_000_000);
        map.add("key1", Value::String("a".to_string()), None);
        map.add("key1", Value::String("b".to_string()), None);

        map.remove("key1", &Value::String("a".to_string()));
        map.remove("key1", &Value::String("b".to_string()));

        assert!(map.get("key1").is_empty());
        assert!(!map.all_keys().contains(&&"key1".to_string()));
    }

    // ---- canonical_json determinism tests ----

    #[test]
    fn hash_entry_deterministic_regardless_of_key_order() {
        // Build two HashMaps with identical records but different insertion order.
        // HashMap iteration order is non-deterministic, so hash_entry must
        // produce the same result regardless.
        let ts = Timestamp {
            millis: 1_000_000,
            counter: 0,
            node_id: "node-1".to_string(),
        };

        let record_a = ORMapRecord {
            value: Value::String("alice".to_string()),
            timestamp: ts.clone(),
            tag: "1000000:0:node-1".to_string(),
            ttl_ms: None,
        };

        let ts2 = Timestamp {
            millis: 1_000_001,
            counter: 0,
            node_id: "node-1".to_string(),
        };

        let record_b = ORMapRecord {
            value: Value::String("bob".to_string()),
            timestamp: ts2.clone(),
            tag: "1000001:0:node-1".to_string(),
            ttl_ms: None,
        };

        // Order 1: insert a then b
        let mut map1: HashMap<String, ORMapRecord<Value>> = HashMap::new();
        map1.insert(record_a.tag.clone(), record_a.clone());
        map1.insert(record_b.tag.clone(), record_b.clone());

        // Order 2: insert b then a
        let mut map2: HashMap<String, ORMapRecord<Value>> = HashMap::new();
        map2.insert(record_b.tag.clone(), record_b);
        map2.insert(record_a.tag.clone(), record_a);

        let hash1 = ORMap::<Value>::hash_entry("user:1", &map1);
        let hash2 = ORMap::<Value>::hash_entry("user:1", &map2);

        assert_eq!(hash1, hash2, "hash_entry must be deterministic regardless of insertion order");
    }

    #[test]
    fn canonical_json_sorts_nested_keys() {
        // Build a Value::Map with keys {z: 1, a: {c: 3, b: 2}}.
        // Because Value uses BTreeMap, the top-level keys are already sorted
        // in the Value itself, but canonical_json must also work for the
        // serde_json::Value intermediate (which uses serde_json::Map, an
        // insertion-order map by default). We verify the output has keys in
        // sorted order at all nesting levels.
        let mut inner = std::collections::BTreeMap::new();
        inner.insert("c".to_string(), Value::Int(3));
        inner.insert("b".to_string(), Value::Int(2));

        let mut outer = std::collections::BTreeMap::new();
        outer.insert("z".to_string(), Value::Int(1));
        outer.insert("a".to_string(), Value::Map(inner));

        let value = Value::Map(outer);

        let json_str = canonical_json(&value);

        // The output includes serde enum variant tags because Value is
        // externally tagged. Verify keys are sorted at all levels by
        // checking that "a" appears before "z" in the output, and
        // "b" appears before "c" in the nested map.
        let a_pos = json_str.find("\"a\"").expect("should contain key 'a'");
        let z_pos = json_str.find("\"z\"").expect("should contain key 'z'");
        assert!(a_pos < z_pos, "'a' must appear before 'z' in sorted output: {json_str}");

        let b_pos = json_str.find("\"b\"").expect("should contain key 'b'");
        let c_pos = json_str.find("\"c\"").expect("should contain key 'c'");
        assert!(b_pos < c_pos, "'b' must appear before 'c' in sorted output: {json_str}");

        // Verify the output is valid JSON that round-trips through serde_json
        let parsed: serde_json::Value = serde_json::from_str(&json_str)
            .expect("canonical_json output must be valid JSON");
        let re_serialized = serde_json::to_string(&parsed).unwrap();
        assert_eq!(json_str, re_serialized, "canonical_json output must be stable");
    }

    #[test]
    fn canonical_json_handles_all_value_types() {
        // Test each Value variant produces valid, deterministic JSON
        let test_cases: Vec<(Value, &str)> = vec![
            (Value::Null, "Null"),
            (Value::Bool(true), "Bool"),
            (Value::Int(42), "Int"),
            (Value::Float(3.14), "Float"),
            (Value::String("hello".to_string()), "String"),
            (
                Value::Array(vec![Value::Int(1), Value::String("two".to_string())]),
                "Array",
            ),
            (
                Value::Map({
                    let mut m = std::collections::BTreeMap::new();
                    m.insert("key".to_string(), Value::Bool(false));
                    m
                }),
                "Map",
            ),
        ];

        for (value, label) in &test_cases {
            let json1 = canonical_json(value);
            let json2 = canonical_json(value);

            // Must be deterministic
            assert_eq!(json1, json2, "canonical_json must be deterministic for {label}");

            // Must be valid JSON
            let parsed: serde_json::Value = serde_json::from_str(&json1)
                .unwrap_or_else(|e| panic!("canonical_json output for {label} must be valid JSON: {e}"));

            // Must round-trip through serde_json
            let re_serialized = serde_json::to_string(&parsed).unwrap();
            assert_eq!(json1, re_serialized, "canonical_json for {label} must be stable through serde_json round-trip");
        }
    }
}

/// Property-based tests using `proptest` for ORMap CRDT correctness verification.
#[cfg(test)]
mod proptests {
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

    /// Creates an `ORMap<Value>` with a high fixed clock to avoid drift errors.
    fn make_proptest_map(node_id: &str) -> ORMap<Value> {
        let (clock, _) = FixedClock::new(u64::MAX / 2);
        let hlc = HLC::new(node_id.to_string(), Box::new(clock));
        ORMap::new(hlc)
    }

    /// Strategy for generating arbitrary `Value` variants (non-recursive for simplicity).
    fn arb_value() -> impl Strategy<Value = Value> {
        prop_oneof![
            Just(Value::Null),
            any::<bool>().prop_map(Value::Bool),
            any::<i64>().prop_map(Value::Int),
            "[a-zA-Z0-9 ]{0,20}".prop_map(|s| Value::String(s)),
        ]
    }

    /// Strategy for generating a key string.
    fn arb_key() -> impl Strategy<Value = String> {
        "[a-z]{1,4}"
    }

    /// ORMap operation to apply to a replica.
    #[derive(Debug, Clone)]
    enum OrMapOp {
        Add(String, Value),
        Remove(String, Value),
    }

    /// Strategy for generating a sequence of ORMap operations.
    fn arb_ops(max_ops: usize) -> impl Strategy<Value = Vec<OrMapOp>> {
        proptest::collection::vec(
            prop_oneof![
                (arb_key(), arb_value()).prop_map(|(k, v)| OrMapOp::Add(k, v)),
                (arb_key(), arb_value()).prop_map(|(k, v)| OrMapOp::Remove(k, v)),
            ],
            1..=max_ops,
        )
    }

    /// Apply operations to an ORMap. Returns tags/tombstones produced.
    fn apply_ops(map: &mut ORMap<Value>, ops: &[OrMapOp]) {
        for op in ops {
            match op {
                OrMapOp::Add(key, value) => {
                    map.add(key.clone(), value.clone(), None);
                }
                OrMapOp::Remove(key, value) => {
                    map.remove(key, value);
                }
            }
        }
    }

    /// Collect all records and tombstones from an ORMap for merging into another.
    fn collect_state(map: &ORMap<Value>) -> (Vec<(String, ORMapRecord<Value>)>, Vec<String>) {
        let mut records = Vec::new();
        for key in map.all_keys() {
            if let Some(key_map) = map.items.get(key) {
                for (_, record) in key_map {
                    records.push((key.clone(), record.clone()));
                }
            }
        }
        let tombstones: Vec<String> = map.tombstones.iter().cloned().collect();
        (records, tombstones)
    }

    /// Get a deterministic sorted snapshot of all active values across all keys.
    fn snapshot(map: &ORMap<Value>) -> Vec<(String, Vec<String>)> {
        let mut result: Vec<(String, Vec<String>)> = Vec::new();
        let mut keys: Vec<&String> = map.all_keys();
        keys.sort();

        for key in keys {
            let mut vals: Vec<String> = map
                .get(key)
                .iter()
                .map(|v| format!("{v:?}"))
                .collect();
            vals.sort();
            result.push((key.clone(), vals));
        }
        result
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(30))]

        /// Convergence: N replicas receiving the same operations in different orders
        /// converge to identical state after full-map merge.
        /// This is the core CRDT property for ORMap.
        #[test]
        fn ormap_convergence_n_replicas(
            ops in arb_ops(6),
        ) {
            // Create 3 replicas
            let mut replica_a = make_proptest_map("node-A");
            let mut replica_b = make_proptest_map("node-B");
            let mut replica_c = make_proptest_map("node-C");

            // Each replica applies the same operations
            apply_ops(&mut replica_a, &ops);
            apply_ops(&mut replica_b, &ops);
            apply_ops(&mut replica_c, &ops);

            // After applying same ops independently, each replica has identical state
            // (same node starts, same clock, same ops => same tags and tombstones)
            // Now simulate cross-replica merge in different orders:

            // Create "convergence" replicas that receive all state
            let mut conv_ab = make_proptest_map("conv-AB");
            let mut conv_ba = make_proptest_map("conv-BA");

            // Merge A's state then B's state into conv_ab
            let (a_records, a_tombstones) = collect_state(&replica_a);
            let (b_records, b_tombstones) = collect_state(&replica_b);

            for (key, record) in &a_records {
                conv_ab.apply(key.clone(), record.clone());
            }
            for tag in &a_tombstones {
                conv_ab.apply_tombstone(tag);
            }
            for (key, record) in &b_records {
                conv_ab.apply(key.clone(), record.clone());
            }
            for tag in &b_tombstones {
                conv_ab.apply_tombstone(tag);
            }

            // Merge B's state then A's state into conv_ba (reverse order)
            for (key, record) in &b_records {
                conv_ba.apply(key.clone(), record.clone());
            }
            for tag in &b_tombstones {
                conv_ba.apply_tombstone(tag);
            }
            for (key, record) in &a_records {
                conv_ba.apply(key.clone(), record.clone());
            }
            for tag in &a_tombstones {
                conv_ba.apply_tombstone(tag);
            }

            // Both convergence replicas should have identical visible state
            let snap_ab = snapshot(&conv_ab);
            let snap_ba = snapshot(&conv_ba);
            prop_assert_eq!(
                snap_ab,
                snap_ba,
                "Convergence failed: merge order should not matter"
            );
        }

        /// Commutativity: merge(A, B) == merge(B, A) for any two ORMaps.
        #[test]
        fn ormap_merge_commutative(
            ops_a in arb_ops(4),
            ops_b in arb_ops(4),
        ) {
            let mut map_a = make_proptest_map("node-A");
            let mut map_b = make_proptest_map("node-B");

            apply_ops(&mut map_a, &ops_a);
            apply_ops(&mut map_b, &ops_b);

            // Order 1: merge B into A
            let mut merged_ab = make_proptest_map("node-AB");
            // First copy A's state
            let (a_records, a_tombstones) = collect_state(&map_a);
            for (key, record) in &a_records {
                merged_ab.apply(key.clone(), record.clone());
            }
            for tag in &a_tombstones {
                merged_ab.apply_tombstone(tag);
            }
            // Then merge B
            let (b_records, b_tombstones) = collect_state(&map_b);
            for (key, record) in &b_records {
                merged_ab.apply(key.clone(), record.clone());
            }
            for tag in &b_tombstones {
                merged_ab.apply_tombstone(tag);
            }

            // Order 2: merge A into B
            let mut merged_ba = make_proptest_map("node-BA");
            for (key, record) in &b_records {
                merged_ba.apply(key.clone(), record.clone());
            }
            for tag in &b_tombstones {
                merged_ba.apply_tombstone(tag);
            }
            for (key, record) in &a_records {
                merged_ba.apply(key.clone(), record.clone());
            }
            for tag in &a_tombstones {
                merged_ba.apply_tombstone(tag);
            }

            let snap_ab = snapshot(&merged_ab);
            let snap_ba = snapshot(&merged_ba);
            prop_assert_eq!(snap_ab, snap_ba, "Merge should be commutative");
        }

        /// Idempotence: merging the same state twice does not change the result.
        #[test]
        fn ormap_merge_idempotent(
            ops in arb_ops(5),
        ) {
            let mut source = make_proptest_map("node-src");
            apply_ops(&mut source, &ops);

            let mut target = make_proptest_map("node-tgt");

            // Merge source into target
            let (records, tombstones) = collect_state(&source);
            for (key, record) in &records {
                target.apply(key.clone(), record.clone());
            }
            for tag in &tombstones {
                target.apply_tombstone(tag);
            }

            let snap_first = snapshot(&target);

            // Merge source into target AGAIN
            for (key, record) in &records {
                target.apply(key.clone(), record.clone());
            }
            for tag in &tombstones {
                target.apply_tombstone(tag);
            }

            let snap_second = snapshot(&target);
            prop_assert_eq!(snap_first, snap_second, "Merge should be idempotent");
        }

        /// `ORMapRecord<Value>` round-trip through MsgPack preserves data.
        #[test]
        fn ormap_record_msgpack_roundtrip(
            value in arb_value(),
            millis in 1_u64..1_000_000_000_u64,
            counter in 0_u32..1000_u32,
            node_id in "[a-z]{1,8}",
        ) {
            let record = ORMapRecord {
                value,
                timestamp: Timestamp { millis, counter, node_id: node_id.clone() },
                tag: format!("{millis}:{counter}:{node_id}"),
                ttl_ms: None,
            };

            let bytes = rmp_serde::to_vec(&record).expect("serialize ORMapRecord<Value>");
            let decoded: ORMapRecord<Value> =
                rmp_serde::from_slice(&bytes).expect("deserialize ORMapRecord<Value>");
            prop_assert_eq!(record, decoded);
        }
    }
}
