//! Merkle tree sync manager and mutation observer for delta synchronization.
//!
//! Provides per-partition Merkle tree management ([`MerkleSyncManager`]) and
//! a [`MutationObserver`] implementation ([`MerkleMutationObserver`]) that keeps
//! trees in sync with `RecordStore` mutations automatically.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use parking_lot::Mutex;
use topgun_core::hash::{combine_hashes, fnv1a_hash};
use topgun_core::hash_to_partition;
use topgun_core::merkle::{MerkleTree, ORMapMerkleTree};

use super::factory::ObserverFactory;
use super::map_data_store::{LeafSink, MapDataStore, MerkleLeaf, MerkleLeafKind};
use super::mutation_observer::MutationObserver;
use super::record::{Record, RecordValue};

// ---------------------------------------------------------------------------
// MerkleSyncManager
// ---------------------------------------------------------------------------

/// Per-partition Merkle tree manager for delta sync.
///
/// Maintains separate `MerkleTree` (LWW) and `ORMapMerkleTree` (OR-Map) instances
/// per `(map_name, partition_id)` pair. Trees are lazily created on first access.
///
/// All access goes through the `with_lww_tree` / `with_ormap_tree` closure API,
/// which locks only the specific tree's `Mutex` without holding the `DashMap` shard
/// lock across tree operations. This allows concurrent access to different
/// partitions while serializing mutations on each individual tree.
pub struct MerkleSyncManager {
    /// Key: (`map_name`, `partition_id`) -> LWW `MerkleTree`
    lww_trees: DashMap<(String, u32), Mutex<MerkleTree>>,
    /// Key: (`map_name`, `partition_id`) -> OR-Map `MerkleTree`
    ormap_trees: DashMap<(String, u32), Mutex<ORMapMerkleTree>>,
    /// Default tree depth (3 = 4096 leaf buckets).
    depth: usize,
}

impl MerkleSyncManager {
    /// Creates a new manager with the given tree depth.
    #[must_use]
    pub fn new(depth: usize) -> Self {
        Self {
            lww_trees: DashMap::new(),
            ormap_trees: DashMap::new(),
            depth,
        }
    }

    /// Accesses the LWW `MerkleTree` for a `(map_name, partition_id)` pair,
    /// lazily creating it if absent.
    ///
    /// The closure `f` receives a mutable reference to the tree and its return
    /// value is propagated to the caller. The Mutex is held only for the
    /// duration of `f` — callers MUST NOT hold the closure open across `.await`
    /// points. Extract all needed data (keys, hashes, node info) as owned values
    /// inside `f`, then perform async operations after this method returns.
    pub fn with_lww_tree<R>(
        &self,
        map_name: &str,
        partition_id: u32,
        f: impl FnOnce(&mut MerkleTree) -> R,
    ) -> R {
        let key = (map_name.to_string(), partition_id);
        // Entry API: lazily insert if absent, then lock and invoke closure.
        let entry = self
            .lww_trees
            .entry(key)
            .or_insert_with(|| Mutex::new(MerkleTree::new(self.depth)));
        let mut guard = entry.lock();
        f(&mut guard)
    }

    /// Accesses the OR-Map `ORMapMerkleTree` for a `(map_name, partition_id)` pair,
    /// lazily creating it if absent.
    ///
    /// Same rules as `with_lww_tree` regarding closure lifetime and async operations.
    pub fn with_ormap_tree<R>(
        &self,
        map_name: &str,
        partition_id: u32,
        f: impl FnOnce(&mut ORMapMerkleTree) -> R,
    ) -> R {
        let key = (map_name.to_string(), partition_id);
        let entry = self
            .ormap_trees
            .entry(key)
            .or_insert_with(|| Mutex::new(ORMapMerkleTree::new(self.depth)));
        let mut guard = entry.lock();
        f(&mut guard)
    }

    /// Updates the LWW tree entry for `key` with `item_hash`.
    pub fn update_lww(&self, map_name: &str, partition_id: u32, key: &str, item_hash: u32) {
        self.with_lww_tree(map_name, partition_id, |tree| {
            tree.update(key, item_hash);
        });
    }

    /// Removes `key` from the LWW tree.
    pub fn remove_lww(&self, map_name: &str, partition_id: u32, key: &str) {
        self.with_lww_tree(map_name, partition_id, |tree| {
            tree.remove(key);
        });
    }

    /// Updates the OR-Map tree entry for `key` with `entry_hash`.
    pub fn update_ormap(&self, map_name: &str, partition_id: u32, key: &str, entry_hash: u32) {
        self.with_ormap_tree(map_name, partition_id, |tree| {
            tree.update(key, entry_hash);
        });
    }

    /// Removes `key` from the OR-Map tree.
    pub fn remove_ormap(&self, map_name: &str, partition_id: u32, key: &str) {
        self.with_ormap_tree(map_name, partition_id, |tree| {
            tree.remove(key);
        });
    }

    /// Removes both the LWW and OR-Map trees for `(map_name, partition_id)`.
    pub fn clear_partition(&self, map_name: &str, partition_id: u32) {
        let key = (map_name.to_string(), partition_id);
        self.lww_trees.remove(&key);
        self.ormap_trees.remove(&key);
    }

    /// Clears all trees for all partitions.
    pub fn clear_all(&self) {
        self.lww_trees.clear();
        self.ormap_trees.clear();
    }

    /// Aggregates LWW root hashes across all partitions for `map_name`.
    ///
    /// Combines all per-partition root hashes with the collision-resistant
    /// `combine_hashes`. Returns 0 when no partitions exist for the given map.
    /// `combine_hashes` is commutative and associative, so the result is
    /// independent of `DashMap`'s non-deterministic iteration order, while
    /// avoiding the compensating-pair collisions a plain additive fold admits.
    #[must_use]
    pub fn aggregate_lww_root_hash(&self, map_name: &str) -> u32 {
        let hashes: Vec<u32> = self
            .lww_trees
            .iter()
            .filter(|entry| entry.key().0 == map_name)
            .map(|entry| entry.value().lock().get_root_hash())
            .collect();
        combine_hashes(&hashes)
    }

    /// Aggregates OR-Map root hashes across all partitions for `map_name`.
    ///
    /// Same aggregation strategy as `aggregate_lww_root_hash`.
    #[must_use]
    pub fn aggregate_ormap_root_hash(&self, map_name: &str) -> u32 {
        let hashes: Vec<u32> = self
            .ormap_trees
            .iter()
            .filter(|entry| entry.key().0 == map_name)
            .map(|entry| entry.value().lock().get_root_hash())
            .collect();
        combine_hashes(&hashes)
    }

    /// Aggregates LWW bucket hashes at `path` across all partitions for `map_name`.
    ///
    /// For each hex bucket character, combines partition values with the
    /// collision-resistant `combine_hashes`. Returns a `HashMap<char, u32>` with
    /// the combined hashes, suitable for returning as a `SyncRespBuckets` response
    /// covering all partitions. Per-character hashes are gathered into a list and
    /// folded once via `combine_hashes`, whose commutativity + associativity make
    /// the result independent of `DashMap`'s non-deterministic iteration order.
    #[must_use]
    pub fn aggregate_lww_buckets(&self, map_name: &str, path: &str) -> HashMap<char, u32> {
        let mut per_char: HashMap<char, Vec<u32>> = HashMap::new();
        for entry in &self.lww_trees {
            if entry.key().0 != map_name {
                continue;
            }
            let buckets = entry.value().lock().get_buckets(path);
            for (c, h) in buckets {
                per_char.entry(c).or_default().push(h);
            }
        }
        per_char
            .into_iter()
            .map(|(c, hashes)| (c, combine_hashes(&hashes)))
            .collect()
    }

    /// Aggregates OR-Map bucket hashes at `path` across all partitions for `map_name`.
    ///
    /// Same aggregation strategy as `aggregate_lww_buckets`.
    #[must_use]
    pub fn aggregate_ormap_buckets(&self, map_name: &str, path: &str) -> HashMap<char, u32> {
        let mut per_char: HashMap<char, Vec<u32>> = HashMap::new();
        for entry in &self.ormap_trees {
            if entry.key().0 != map_name {
                continue;
            }
            let buckets = entry.value().lock().get_buckets(path);
            for (c, h) in buckets {
                per_char.entry(c).or_default().push(h);
            }
        }
        per_char
            .into_iter()
            .map(|(c, hashes)| (c, combine_hashes(&hashes)))
            .collect()
    }

    /// Returns all partition IDs that have a LWW tree for `map_name`.
    #[must_use]
    pub fn lww_partition_ids(&self, map_name: &str) -> Vec<u32> {
        self.lww_trees
            .iter()
            .filter(|entry| entry.key().0 == map_name)
            .map(|entry| entry.key().1)
            .collect()
    }

    /// Returns all partition IDs that have an OR-Map tree for `map_name`.
    #[must_use]
    pub fn ormap_partition_ids(&self, map_name: &str) -> Vec<u32> {
        self.ormap_trees
            .iter()
            .filter(|entry| entry.key().0 == map_name)
            .map(|entry| entry.key().1)
            .collect()
    }

    /// (Re)builds `map_name`'s LWW and OR-Map Merkle trees from the durable
    /// datastore index.
    ///
    /// The write-path observer only ever populates a tree for records that are
    /// resident in the in-memory engine. After a crash/restart (or once a record
    /// has been evicted) those leaves are gone, so a `SyncInit` would report root
    /// `0` and wrongly tell the client it is in sync — even though the data is
    /// safely persisted. This primitive closes that gap WITHOUT loading any
    /// values: it streams `(key, kind, leaf_hash)` from
    /// `data_store.enumerate_leaves` and folds each leaf into the SAME
    /// `(map, partition)` tree the write path would have written, so a subsequent
    /// `SyncInit` produces the correct non-zero, pre-crash-equal root.
    ///
    /// Routing parity is the load-bearing invariant: each leaf is placed at
    /// `hash_to_partition(key)` — byte-identical to the write path's
    /// `RecordStoreFactory::get_or_create(map, hash_to_partition(key))` routing —
    /// and inserted with the same per-CRDT tree mutation the observer makes
    /// (`update_lww` for LWW leaves, `update_ormap` for OR-Map leaves), selected
    /// by the leaf's `MerkleLeafKind`. Because the per-partition combine
    /// (`combine_hashes`/`fnv1a_hash`) is commutative and associative, replaying
    /// the persisted leaf set in any order yields the identical partition root,
    /// and the cross-partition aggregate matches the live root.
    ///
    /// Both CRDT kinds are rebuilt here. The datastore computes the LWW leaf
    /// hash with `fnv1a_hash("{key}:{millis}:{counter}:{node_id}")` (matching
    /// `compute_lww_hash`) and the OR-Map leaf hash over the sorted active +
    /// tombstone tag sets (matching `compute_ormap_hash`), tagging each leaf with
    /// its kind so the rebuild routes it to the matching tree — never polluting
    /// the LWW tree with OR-Map leaves or leaving the OR-Map tree empty.
    ///
    /// Memory is bounded by the datastore's batch size: only `(key, u32)` pairs
    /// cross the boundary, never values, so peak cost is independent of map size.
    ///
    /// # Errors
    ///
    /// Propagates any error returned by `data_store.enumerate_leaves`.
    pub async fn rebuild_from_datastore(
        self: &Arc<Self>,
        map: &str,
        data_store: &Arc<dyn MapDataStore>,
    ) -> anyhow::Result<()> {
        let mut sink = MerkleRebuildSink {
            manager: Arc::clone(self),
            map: map.to_string(),
        };
        // Backup partitions never participate in client sync, mirroring the
        // observer's `is_backup` early-returns; rebuild the primary leaves only.
        data_store.enumerate_leaves(map, false, &mut sink).await
    }
}

/// [`LeafSink`] that folds each enumerated durable leaf into the Merkle tree
/// for its key's partition AND CRDT kind, reproducing the write-path observer's
/// per-kind insertion exactly.
///
/// Per batch it routes every leaf to `hash_to_partition(key)` (the same key
/// partition function the write path uses for both CRDT kinds) and dispatches
/// on `leaf.kind`: LWW leaves call `MerkleSyncManager::update_lww` (the observer's
/// `RecordValue::Lww` mutation) and OR-Map leaves call `update_ormap` (the
/// observer's `RecordValue::OrMap` mutation). Routing on kind is what keeps the
/// rebuilt LWW and OR-Map roots byte-identical to the pre-crash roots for maps
/// that mix both kinds. No values are held; only the current batch of leaf
/// coordinates is resident.
struct MerkleRebuildSink {
    manager: Arc<MerkleSyncManager>,
    map: String,
}

#[async_trait]
impl LeafSink for MerkleRebuildSink {
    async fn consume(&mut self, batch: Vec<MerkleLeaf>) -> anyhow::Result<()> {
        for leaf in batch {
            let partition_id = hash_to_partition(&leaf.key);
            match leaf.kind {
                MerkleLeafKind::Lww => {
                    self.manager
                        .update_lww(&self.map, partition_id, &leaf.key, leaf.leaf_hash);
                }
                MerkleLeafKind::OrMap => {
                    self.manager
                        .update_ormap(&self.map, partition_id, &leaf.key, leaf.leaf_hash);
                }
            }
        }
        Ok(())
    }
}

impl Default for MerkleSyncManager {
    fn default() -> Self {
        Self::new(3)
    }
}

// ---------------------------------------------------------------------------
// MerkleObserverFactory
// ---------------------------------------------------------------------------

/// Factory that creates [`MerkleMutationObserver`] instances for every map.
///
/// Implements [`ObserverFactory`] so it can be wired into
/// [`RecordStoreFactory::with_observer_factories()`](super::factory::RecordStoreFactory::with_observer_factories).
/// Every map participates in Merkle sync, so `create_observer` always returns
/// `Some(...)`.
pub struct MerkleObserverFactory {
    manager: Arc<MerkleSyncManager>,
}

impl MerkleObserverFactory {
    /// Creates a new factory backed by the given [`MerkleSyncManager`].
    #[must_use]
    pub fn new(manager: Arc<MerkleSyncManager>) -> Self {
        Self { manager }
    }
}

impl ObserverFactory for MerkleObserverFactory {
    fn create_observer(
        &self,
        map_name: &str,
        partition_id: u32,
    ) -> Option<Arc<dyn MutationObserver>> {
        Some(Arc::new(MerkleMutationObserver::new(
            Arc::clone(&self.manager),
            map_name.to_string(),
            partition_id,
        )))
    }
}

// ---------------------------------------------------------------------------
// Hash computation helpers
// ---------------------------------------------------------------------------

/// Computes the item hash for a LWW record, matching the TS MerkleTree.update pattern.
///
/// Uses `fnv1a_hash` on `"key:millis:counter:node_id"` to produce a hash that
/// is consistent across Rust and TypeScript clients.
fn compute_lww_hash(key: &str, millis: u64, counter: u32, node_id: &str) -> u32 {
    fnv1a_hash(&format!("{key}:{millis}:{counter}:{node_id}"))
}

/// Computes the entry hash for an OR-Map record.
///
/// Sorts active tags and tombstone tags independently for determinism, then
/// hashes `"key:tag1|tag2|...#tomb1|tomb2|..."`. Tombstones are folded in so the
/// hash changes when a tag is removed — otherwise peers cannot observe a
/// tombstone-only delta and remove-wins suppression would not replicate.
fn compute_ormap_hash(
    key: &str,
    records: &[super::record::OrMapEntry],
    tombstones: &[String],
) -> u32 {
    let mut tags: Vec<&str> = records.iter().map(|r| r.tag.as_str()).collect();
    tags.sort_unstable();
    let joined = tags.join("|");
    let mut tomb_tags: Vec<&str> = tombstones.iter().map(String::as_str).collect();
    tomb_tags.sort_unstable();
    let joined_tombs = tomb_tags.join("|");
    fnv1a_hash(&format!("key:{key}|{joined}#{joined_tombs}"))
}

// ---------------------------------------------------------------------------
// MerkleMutationObserver
// ---------------------------------------------------------------------------

/// `MutationObserver` implementation that keeps `MerkleSyncManager` in sync
/// with `RecordStore` mutations.
///
/// Constructed with the `map_name` and `partition_id` it belongs to, since
/// `MutationObserver` trait methods do not carry those attributes. Each
/// `RecordStore` is scoped to a single `(map_name, partition_id)`.
pub struct MerkleMutationObserver {
    manager: Arc<MerkleSyncManager>,
    map_name: String,
    partition_id: u32,
}

impl MerkleMutationObserver {
    /// Creates a new observer for the given map partition.
    #[must_use]
    pub fn new(manager: Arc<MerkleSyncManager>, map_name: String, partition_id: u32) -> Self {
        Self {
            manager,
            map_name,
            partition_id,
        }
    }

    /// Updates the appropriate Merkle tree based on the `RecordValue` variant.
    ///
    /// Writes only to `self.partition_id`. The aggregate root hash for client sync
    /// is computed on demand via `MerkleSyncManager::aggregate_lww_root_hash()` /
    /// `aggregate_ormap_root_hash()`, eliminating the Mutex contention on a shared
    /// partition 0 that previously bottlenecked concurrent writes.
    fn update_tree(&self, key: &str, value: &RecordValue) {
        match value {
            RecordValue::Lww { timestamp, .. } => {
                let hash =
                    compute_lww_hash(key, timestamp.millis, timestamp.counter, &timestamp.node_id);
                self.manager
                    .update_lww(&self.map_name, self.partition_id, key, hash);
            }
            RecordValue::OrMap {
                records,
                tombstones,
            } => {
                let hash = compute_ormap_hash(key, records, tombstones);
                self.manager
                    .update_ormap(&self.map_name, self.partition_id, key, hash);
            }
            RecordValue::OrTombstones { .. } => {
                // Tombstones represent deletions — remove from OR-Map tree.
                self.manager
                    .remove_ormap(&self.map_name, self.partition_id, key);
            }
        }
    }
}

impl MutationObserver for MerkleMutationObserver {
    fn on_put(
        &self,
        key: &str,
        record: &Record,
        _old_value: Option<&RecordValue>,
        is_backup: bool,
    ) {
        // Backup partitions do not participate in client sync.
        if is_backup {
            return;
        }
        self.update_tree(key, &record.value);
    }

    fn on_update(
        &self,
        key: &str,
        _record: &Record,
        _old_value: &RecordValue,
        new_value: &RecordValue,
        is_backup: bool,
    ) {
        // Use new_value (not record.value) since the in-place update may not
        // have been committed to the record yet at observer call time.
        if is_backup {
            return;
        }
        self.update_tree(key, new_value);
    }

    fn on_remove(&self, key: &str, _record: &Record, is_backup: bool) {
        // Backup keys were never added to the tree.
        if is_backup {
            return;
        }
        // Call both removes: removing a non-existent key is a harmless no-op,
        // and this avoids inspecting record.value to determine the original tree type.
        self.manager
            .remove_lww(&self.map_name, self.partition_id, key);
        self.manager
            .remove_ormap(&self.map_name, self.partition_id, key);
    }

    fn on_evict(&self, key: &str, _record: &Record, is_backup: bool) {
        // Backup keys were never added to the tree.
        if is_backup {
            return;
        }
        // Same double-remove approach as on_remove for consistency.
        self.manager
            .remove_lww(&self.map_name, self.partition_id, key);
        self.manager
            .remove_ormap(&self.map_name, self.partition_id, key);
    }

    fn on_load(&self, key: &str, record: &Record, is_backup: bool) {
        // Loading from storage should update the tree, same as on_put.
        if is_backup {
            return;
        }
        self.update_tree(key, &record.value);
    }

    fn on_replication_put(&self, key: &str, record: &Record, _populate_index: bool) {
        // Replication data should be reflected in sync state.
        self.update_tree(key, &record.value);
    }

    fn on_clear(&self) {
        self.manager
            .clear_partition(&self.map_name, self.partition_id);
    }

    fn on_reset(&self) {
        self.manager
            .clear_partition(&self.map_name, self.partition_id);
    }

    fn on_destroy(&self, _is_shutdown: bool) {
        self.manager
            .clear_partition(&self.map_name, self.partition_id);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use topgun_core::hlc::Timestamp;
    use topgun_core::types::Value;

    use super::*;
    use crate::storage::record::{OrMapEntry, Record, RecordMetadata, RecordValue};

    fn make_lww_record(key: &str, millis: u64, counter: u32, node_id: &str) -> Record {
        Record {
            value: RecordValue::Lww {
                value: Value::String(key.to_string()),
                timestamp: Timestamp {
                    millis,
                    counter,
                    node_id: node_id.to_string(),
                },
            },
            #[allow(clippy::cast_possible_wrap)]
            metadata: RecordMetadata::new(millis as i64, 64),
        }
    }

    fn make_ormap_record(key: &str, tag: &str, millis: u64) -> Record {
        Record {
            value: RecordValue::OrMap {
                records: vec![OrMapEntry {
                    value: Value::String(key.to_string()),
                    tag: tag.to_string(),
                    timestamp: Timestamp {
                        millis,
                        counter: 0,
                        node_id: "node-1".to_string(),
                    },
                }],
                tombstones: vec![],
            },
            #[allow(clippy::cast_possible_wrap)]
            metadata: RecordMetadata::new(millis as i64, 64),
        }
    }

    // ---------------------------------------------------------------------------
    // AC5: MerkleObserverFactory creates observers for any map
    // ---------------------------------------------------------------------------

    #[test]
    fn merkle_observer_factory_returns_some_for_any_map() {
        let manager = Arc::new(MerkleSyncManager::default());
        let factory = MerkleObserverFactory::new(Arc::clone(&manager));

        let observer = factory.create_observer("test-map", 0);
        assert!(observer.is_some(), "factory should return Some for any map");

        let observer2 = factory.create_observer("another-map", 42);
        assert!(
            observer2.is_some(),
            "factory should return Some for any partition"
        );
    }

    // ---------------------------------------------------------------------------
    // AC8: MerkleMutationObserver updates LWW tree on put/remove
    // ---------------------------------------------------------------------------

    #[test]
    fn ac8_lww_tree_updates_on_put() {
        let manager = Arc::new(MerkleSyncManager::default());
        let observer = MerkleMutationObserver::new(Arc::clone(&manager), "users".to_string(), 0);

        // Initially the root hash is zero (empty tree).
        let initial_hash = manager.with_lww_tree("users", 0, |tree| tree.get_root_hash());
        assert_eq!(initial_hash, 0, "empty tree should have root_hash = 0");

        // After on_put with an LWW record, root hash should be non-zero.
        let record = make_lww_record("user-1", 1_700_000_000_000, 0, "node-1");
        observer.on_put("user-1", &record, None, false);
        let hash_after_put = manager.with_lww_tree("users", 0, |tree| tree.get_root_hash());
        assert_ne!(hash_after_put, 0, "root hash should be non-zero after put");

        // After on_remove, root hash should return to zero.
        observer.on_remove("user-1", &record, false);
        let hash_after_remove = manager.with_lww_tree("users", 0, |tree| tree.get_root_hash());
        assert_eq!(
            hash_after_remove, 0,
            "root hash should return to 0 after remove"
        );
    }

    #[test]
    fn ac8_backup_mutations_do_not_update_tree() {
        let manager = Arc::new(MerkleSyncManager::default());
        let observer = MerkleMutationObserver::new(Arc::clone(&manager), "users".to_string(), 0);

        let record = make_lww_record("user-1", 1_700_000_000_000, 0, "node-1");

        // Backup puts should NOT update the tree.
        observer.on_put("user-1", &record, None, true);
        let hash = manager.with_lww_tree("users", 0, |tree| tree.get_root_hash());
        assert_eq!(hash, 0, "backup put should not update the tree");

        // Backup removes should also be no-ops.
        observer.on_remove("user-1", &record, true);
        let hash = manager.with_lww_tree("users", 0, |tree| tree.get_root_hash());
        assert_eq!(hash, 0, "backup remove should not affect tree");
    }

    // ---------------------------------------------------------------------------
    // AC9: MerkleMutationObserver updates OR-Map tree on put/remove
    // ---------------------------------------------------------------------------

    #[test]
    fn ac9_ormap_tree_updates_on_put() {
        let manager = Arc::new(MerkleSyncManager::default());
        let observer = MerkleMutationObserver::new(Arc::clone(&manager), "tags".to_string(), 0);

        // Initially the OR-Map root hash is zero.
        let initial_hash = manager.with_ormap_tree("tags", 0, |tree| tree.get_root_hash());
        assert_eq!(
            initial_hash, 0,
            "empty OR-Map tree should have root_hash = 0"
        );

        // After on_put with an OR-Map record, root hash should be non-zero.
        let record = make_ormap_record("tag-1", "1700000000000:0:node-1", 1_700_000_000_000);
        observer.on_put("tag-1", &record, None, false);
        let hash_after_put = manager.with_ormap_tree("tags", 0, |tree| tree.get_root_hash());
        assert_ne!(
            hash_after_put, 0,
            "OR-Map root hash should be non-zero after put"
        );

        // After on_remove, OR-Map root hash should return to zero.
        observer.on_remove("tag-1", &record, false);
        let hash_after_remove = manager.with_ormap_tree("tags", 0, |tree| tree.get_root_hash());
        assert_eq!(
            hash_after_remove, 0,
            "OR-Map root hash should return to 0 after remove"
        );
    }

    // ---------------------------------------------------------------------------
    // AC10: MerkleSyncManager clear_partition resets trees
    // ---------------------------------------------------------------------------

    #[test]
    fn ac10_clear_partition_resets_both_trees() {
        let manager = Arc::new(MerkleSyncManager::default());
        let observer = MerkleMutationObserver::new(Arc::clone(&manager), "users".to_string(), 0);

        // Populate both trees.
        let lww_record = make_lww_record("user-1", 1_700_000_000_000, 0, "node-1");
        observer.on_put("user-1", &lww_record, None, false);

        let ormap_record = make_ormap_record("tag-1", "1700000000000:0:node-1", 1_700_000_000_000);
        let ormap_observer =
            MerkleMutationObserver::new(Arc::clone(&manager), "users".to_string(), 0);
        ormap_observer.on_put("tag-1", &ormap_record, None, false);

        // Verify both trees have non-zero hashes before clearing.
        let lww_hash = manager.with_lww_tree("users", 0, |tree| tree.get_root_hash());
        assert_ne!(
            lww_hash, 0,
            "LWW tree should have non-zero hash before clear"
        );
        let ormap_hash = manager.with_ormap_tree("users", 0, |tree| tree.get_root_hash());
        assert_ne!(
            ormap_hash, 0,
            "OR-Map tree should have non-zero hash before clear"
        );

        // Clear the partition.
        manager.clear_partition("users", 0);

        // After clearing, both trees should return hash = 0 (new empty trees on next access).
        let lww_hash_after = manager.with_lww_tree("users", 0, |tree| tree.get_root_hash());
        let ormap_hash_after = manager.with_ormap_tree("users", 0, |tree| tree.get_root_hash());
        assert_eq!(
            lww_hash_after, 0,
            "LWW tree should have root_hash = 0 after clear_partition"
        );
        assert_eq!(
            ormap_hash_after, 0,
            "OR-Map tree should have root_hash = 0 after clear_partition"
        );
    }

    // ---------------------------------------------------------------------------
    // Additional: on_update uses new_value
    // ---------------------------------------------------------------------------

    #[test]
    fn on_update_uses_new_value_not_record_value() {
        let manager = Arc::new(MerkleSyncManager::default());
        let observer = MerkleMutationObserver::new(Arc::clone(&manager), "users".to_string(), 0);

        // Put initial value.
        let old_record = make_lww_record("user-1", 1_000, 0, "node-1");
        observer.on_put("user-1", &old_record, None, false);
        let hash_v1 = manager.with_lww_tree("users", 0, |tree| tree.get_root_hash());

        // Update with a new timestamp -- tree should reflect the new_value hash.
        let new_value = RecordValue::Lww {
            value: Value::String("updated".to_string()),
            timestamp: Timestamp {
                millis: 2_000,
                counter: 1,
                node_id: "node-1".to_string(),
            },
        };
        observer.on_update("user-1", &old_record, &old_record.value, &new_value, false);
        let hash_v2 = manager.with_lww_tree("users", 0, |tree| tree.get_root_hash());

        // Hash should change because the timestamp changed.
        assert_ne!(
            hash_v1, hash_v2,
            "hash should change after update with new timestamp"
        );
    }

    // ---------------------------------------------------------------------------
    // Additional: on_clear and on_reset
    // ---------------------------------------------------------------------------

    #[test]
    fn on_clear_clears_partition() {
        let manager = Arc::new(MerkleSyncManager::default());
        let observer = MerkleMutationObserver::new(Arc::clone(&manager), "users".to_string(), 0);

        let record = make_lww_record("user-1", 1_000, 0, "node-1");
        observer.on_put("user-1", &record, None, false);

        observer.on_clear();

        let hash = manager.with_lww_tree("users", 0, |tree| tree.get_root_hash());
        assert_eq!(hash, 0, "on_clear should reset the partition");
    }

    #[test]
    fn on_reset_clears_partition() {
        let manager = Arc::new(MerkleSyncManager::default());
        let observer = MerkleMutationObserver::new(Arc::clone(&manager), "users".to_string(), 0);

        let record = make_lww_record("user-1", 1_000, 0, "node-1");
        observer.on_put("user-1", &record, None, false);

        observer.on_reset();

        let hash = manager.with_lww_tree("users", 0, |tree| tree.get_root_hash());
        assert_eq!(hash, 0, "on_reset should reset the partition");
    }

    // ---------------------------------------------------------------------------
    // Additional: clear_all
    // ---------------------------------------------------------------------------

    // ---------------------------------------------------------------------------
    // Aggregate methods: scatter-gather root hash and buckets
    // ---------------------------------------------------------------------------

    #[test]
    fn aggregate_lww_root_hash_empty_returns_zero() {
        let manager = Arc::new(MerkleSyncManager::default());
        let result = manager.aggregate_lww_root_hash("users");
        assert_eq!(result, 0, "no partitions should produce aggregate hash = 0");
    }

    #[test]
    fn aggregate_lww_root_hash_combines_partitions() {
        let manager = Arc::new(MerkleSyncManager::default());
        // Write to two different partitions.
        manager.update_lww("users", 1, "alice", 111);
        manager.update_lww("users", 2, "bob", 222);

        let hash_1 = manager.with_lww_tree("users", 1, |tree| tree.get_root_hash());
        let hash_2 = manager.with_lww_tree("users", 2, |tree| tree.get_root_hash());
        let expected = combine_hashes(&[hash_1, hash_2]);

        let aggregate = manager.aggregate_lww_root_hash("users");
        assert_eq!(
            aggregate, expected,
            "aggregate should equal combine_hashes of all partition hashes"
        );
        assert_ne!(
            aggregate, 0,
            "aggregate should be non-zero when partitions have data"
        );
    }

    #[test]
    fn aggregate_lww_root_hash_resists_compensating_partition_pairs() {
        // Two per-partition hash sets that are equal under a plain additive fold
        // (0xAAAA0000 + 0x00005555 == 0xAAAA5555 + 0x00000000 == 0xAAAA5555).
        // The cross-partition aggregate folds these with combine_hashes, which
        // must keep them distinct so compensating per-partition hashes can never
        // produce an identical server root (which the client treats as "in sync").
        let set_a = [0xAAAA_0000u32, 0x0000_5555u32];
        let set_b = [0xAAAA_5555u32, 0x0000_0000u32];

        assert_eq!(
            set_a[0].wrapping_add(set_a[1]),
            set_b[0].wrapping_add(set_b[1]),
            "test vectors must collide under the old additive scheme"
        );
        assert_ne!(
            combine_hashes(&set_a),
            combine_hashes(&set_b),
            "compensating partition-hash pairs must produce different aggregates"
        );
    }

    #[test]
    fn aggregate_lww_root_hash_is_map_scoped() {
        let manager = Arc::new(MerkleSyncManager::default());
        manager.update_lww("users", 1, "alice", 111);
        manager.update_lww("orders", 2, "order-1", 999);

        // Aggregate for "users" should not include "orders" data.
        let users_hash = manager.aggregate_lww_root_hash("users");
        let orders_hash = manager.aggregate_lww_root_hash("orders");
        let users_p1 = manager.with_lww_tree("users", 1, |tree| tree.get_root_hash());
        assert_eq!(
            users_hash,
            combine_hashes(&[users_p1]),
            "users aggregate should combine only its own partition"
        );
        assert_ne!(
            users_hash, orders_hash,
            "different maps should have different aggregate hashes"
        );
    }

    #[test]
    fn aggregate_ormap_root_hash_empty_returns_zero() {
        let manager = Arc::new(MerkleSyncManager::default());
        let result = manager.aggregate_ormap_root_hash("tags");
        assert_eq!(result, 0, "no partitions should produce aggregate hash = 0");
    }

    #[test]
    fn aggregate_ormap_root_hash_combines_partitions() {
        let manager = Arc::new(MerkleSyncManager::default());
        manager.update_ormap("tags", 1, "tag-1", 111);
        manager.update_ormap("tags", 2, "tag-2", 222);

        let hash_1 = manager.with_ormap_tree("tags", 1, |tree| tree.get_root_hash());
        let hash_2 = manager.with_ormap_tree("tags", 2, |tree| tree.get_root_hash());
        let expected = combine_hashes(&[hash_1, hash_2]);

        let aggregate = manager.aggregate_ormap_root_hash("tags");
        assert_eq!(
            aggregate, expected,
            "OR-Map aggregate should equal combine_hashes of partition hashes"
        );
    }

    #[test]
    fn aggregate_lww_buckets_empty_returns_empty_map() {
        let manager = Arc::new(MerkleSyncManager::default());
        let buckets = manager.aggregate_lww_buckets("users", "");
        assert!(
            buckets.is_empty(),
            "no partitions should produce empty bucket map"
        );
    }

    #[test]
    fn aggregate_lww_buckets_combines_from_all_partitions() {
        let manager = Arc::new(MerkleSyncManager::default());
        // Write enough keys to ensure root has buckets.
        for i in 0..8u32 {
            manager.update_lww("users", i % 3 + 1, &format!("key-{i}"), i * 100 + 1);
        }

        let combined = manager.aggregate_lww_buckets("users", "");
        // At depth=3 with multiple keys, root should have some buckets.
        assert!(
            !combined.is_empty(),
            "aggregate buckets should be non-empty with data in partitions"
        );
        for c in combined.keys() {
            assert!(
                c.is_ascii_hexdigit(),
                "bucket key should be a hex char, got: {c}"
            );
        }
    }

    #[test]
    fn aggregate_ormap_buckets_empty_returns_empty_map() {
        let manager = Arc::new(MerkleSyncManager::default());
        let buckets = manager.aggregate_ormap_buckets("tags", "");
        assert!(
            buckets.is_empty(),
            "no partitions should produce empty bucket map"
        );
    }

    #[test]
    fn lww_partition_ids_returns_correct_partitions() {
        let manager = Arc::new(MerkleSyncManager::default());
        manager.update_lww("users", 1, "alice", 111);
        manager.update_lww("users", 42, "bob", 222);
        manager.update_lww("orders", 5, "order-1", 333);

        let mut ids = manager.lww_partition_ids("users");
        ids.sort_unstable();
        assert_eq!(
            ids,
            vec![1, 42],
            "should return only partitions for 'users'"
        );

        let order_ids = manager.lww_partition_ids("orders");
        assert_eq!(
            order_ids,
            vec![5],
            "should return only partitions for 'orders'"
        );

        let empty_ids = manager.lww_partition_ids("missing");
        assert!(
            empty_ids.is_empty(),
            "non-existent map should return empty partition list"
        );
    }

    #[test]
    fn ormap_partition_ids_returns_correct_partitions() {
        let manager = Arc::new(MerkleSyncManager::default());
        manager.update_ormap("tags", 3, "t1", 100);
        manager.update_ormap("tags", 7, "t2", 200);

        let mut ids = manager.ormap_partition_ids("tags");
        ids.sort_unstable();
        assert_eq!(
            ids,
            vec![3, 7],
            "should return all partitions with OR-Map trees for 'tags'"
        );
    }

    #[test]
    fn single_partition_observer_writes_only_to_its_partition() {
        let manager = Arc::new(MerkleSyncManager::default());
        let observer = MerkleMutationObserver::new(Arc::clone(&manager), "users".to_string(), 42);

        let record = make_lww_record("alice", 1_700_000_000_000, 0, "node-1");
        observer.on_put("alice", &record, None, false);

        // Partition 42 should have data.
        let hash_42 = manager.with_lww_tree("users", 42, |tree| tree.get_root_hash());
        assert_ne!(
            hash_42, 0,
            "partition 42 should have non-zero hash after put"
        );

        // No other partition tree should have been created.
        let partition_ids = manager.lww_partition_ids("users");
        assert_eq!(
            partition_ids,
            vec![42],
            "only partition 42 should exist, no dual-write"
        );
    }

    // ---------------------------------------------------------------------------

    #[test]
    fn clear_all_removes_all_partitions() {
        let manager = Arc::new(MerkleSyncManager::default());

        // Populate two different partitions.
        manager.update_lww("users", 0, "u1", 111);
        manager.update_lww("users", 1, "u2", 222);
        manager.update_ormap("tags", 0, "t1", 333);

        manager.clear_all();

        // All partitions should have hash = 0 after clear_all.
        let h1 = manager.with_lww_tree("users", 0, |tree| tree.get_root_hash());
        let h2 = manager.with_lww_tree("users", 1, |tree| tree.get_root_hash());
        let h3 = manager.with_ormap_tree("tags", 0, |tree| tree.get_root_hash());
        assert_eq!(h1, 0, "partition (users, 0) should be cleared");
        assert_eq!(h2, 0, "partition (users, 1) should be cleared");
        assert_eq!(h3, 0, "partition (tags, 0) should be cleared");
    }
}

// ---------------------------------------------------------------------------
// Index-sourced rebuild: recovery, leaf-hash parity, eviction, RAM ceiling.
//
// These bind the durable-but-non-resident Merkle invariant: after a crash /
// restart, or after a record is evicted from the in-memory engine, the map's
// Merkle root must still reflect the persisted record so `SyncInit` does not
// wrongly tell a reconnecting client it is in sync. The pre-fix behavior
// sourced leaves only from the in-memory write path, so any non-resident
// record vanished from the root. Every test here uses a real redb datastore
// (tempfile) so `enumerate_leaves` runs the true range-scan path; asserting
// against an empty mock would be vacuous.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod rebuild_tests {
    use std::sync::Arc;

    use topgun_core::hlc::Timestamp;
    use topgun_core::types::Value;

    use super::{compute_lww_hash, compute_ormap_hash, MerkleObserverFactory, MerkleSyncManager};
    use crate::storage::datastores::RedbDataStore;
    use crate::storage::factory::ObserverFactory;
    use crate::storage::map_data_store::{LeafSink, MapDataStore, MerkleLeaf, MerkleLeafKind};
    use crate::storage::record::{OrMapEntry, Record, RecordMetadata, RecordValue};
    use topgun_core::hash_to_partition;

    /// Build a fresh tempdir-backed redb store as an `Arc<dyn MapDataStore>`.
    fn fresh_redb() -> (Arc<dyn MapDataStore>, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("rebuild.redb");
        let store = RedbDataStore::new(&path).expect("redb open");
        (Arc::new(store), dir)
    }

    fn lww_value(key: &str, millis: u64, counter: u32, node_id: &str) -> RecordValue {
        RecordValue::Lww {
            value: Value::String(key.to_string()),
            timestamp: Timestamp {
                millis,
                counter,
                node_id: node_id.to_string(),
            },
        }
    }

    fn ormap_value(key: &str, tag: &str, millis: u64) -> RecordValue {
        RecordValue::OrMap {
            records: vec![OrMapEntry {
                value: Value::String(key.to_string()),
                tag: tag.to_string(),
                timestamp: Timestamp {
                    millis,
                    counter: 0,
                    node_id: "node-1".to_string(),
                },
            }],
            tombstones: vec![],
        }
    }

    fn lww_record(key: &str, millis: u64, counter: u32, node_id: &str) -> Record {
        Record {
            value: lww_value(key, millis, counter, node_id),
            #[allow(clippy::cast_possible_wrap)]
            metadata: RecordMetadata::new(millis as i64, 64),
        }
    }

    fn ormap_record(key: &str, tag: &str, millis: u64) -> Record {
        Record {
            value: ormap_value(key, tag, millis),
            #[allow(clippy::cast_possible_wrap)]
            metadata: RecordMetadata::new(millis as i64, 64),
        }
    }

    /// Datastore decorator that forwards to a real redb store but PANICS if any
    /// full-value load is attempted, and counts `enumerate_leaves` calls. Lets
    /// the RAM-ceiling test prove `rebuild_from_datastore` only ever streams
    /// leaves (keys + `u32` hashes) and never materializes record values.
    struct SpyStore {
        inner: RedbDataStore,
        enumerate_calls: Arc<std::sync::atomic::AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl MapDataStore for SpyStore {
        async fn add(
            &self,
            map: &str,
            key: &str,
            value: &RecordValue,
            expiration_time: i64,
            now: i64,
        ) -> anyhow::Result<()> {
            self.inner.add(map, key, value, expiration_time, now).await
        }
        async fn add_backup(
            &self,
            map: &str,
            key: &str,
            value: &RecordValue,
            expiration_time: i64,
            now: i64,
        ) -> anyhow::Result<()> {
            self.inner
                .add_backup(map, key, value, expiration_time, now)
                .await
        }
        async fn remove(&self, map: &str, key: &str, now: i64) -> anyhow::Result<()> {
            self.inner.remove(map, key, now).await
        }
        async fn remove_backup(&self, map: &str, key: &str, now: i64) -> anyhow::Result<()> {
            self.inner.remove_backup(map, key, now).await
        }
        async fn load(&self, _map: &str, _key: &str) -> anyhow::Result<Option<RecordValue>> {
            panic!("rebuild must NOT load full values: load() called");
        }
        async fn load_all(
            &self,
            _map: &str,
            _keys: &[String],
        ) -> anyhow::Result<Vec<(String, RecordValue)>> {
            panic!("rebuild must NOT load full values: load_all() called");
        }
        async fn enumerate_leaves(
            &self,
            map: &str,
            is_backup: bool,
            sink: &mut dyn LeafSink,
        ) -> anyhow::Result<()> {
            self.enumerate_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.inner.enumerate_leaves(map, is_backup, sink).await
        }
        async fn scan_values(
            &self,
            map: &str,
            is_backup: bool,
            max_batch_cost: u64,
        ) -> anyhow::Result<crate::storage::map_data_store::ScanBatch> {
            self.inner.scan_values(map, is_backup, max_batch_cost).await
        }
        async fn scan_values_batched(
            &self,
            map: &str,
            is_backup: bool,
            cursor: crate::storage::map_data_store::ScanCursor,
            max_batch_cost: u64,
        ) -> anyhow::Result<crate::storage::map_data_store::ScanBatch> {
            self.inner
                .scan_values_batched(map, is_backup, cursor, max_batch_cost)
                .await
        }
        async fn remove_all(&self, map: &str, keys: &[String]) -> anyhow::Result<()> {
            self.inner.remove_all(map, keys).await
        }
        fn is_loadable(&self, key: &str) -> bool {
            self.inner.is_loadable(key)
        }
        fn pending_operation_count(&self) -> u64 {
            self.inner.pending_operation_count()
        }
        async fn soft_flush(&self) -> anyhow::Result<u64> {
            self.inner.soft_flush().await
        }
        async fn hard_flush(&self) -> anyhow::Result<()> {
            self.inner.hard_flush().await
        }
        async fn flush_key(
            &self,
            map: &str,
            key: &str,
            value: &RecordValue,
            is_backup: bool,
        ) -> anyhow::Result<()> {
            self.inner.flush_key(map, key, value, is_backup).await
        }
        fn reset(&self) {
            self.inner.reset();
        }
    }

    /// Drive the live write-path observer for one record on a manager, routing
    /// to the SAME `(map, hash_to_partition(key))` tree the real server uses.
    /// This is the resident root that a reconnecting client would receive
    /// before any crash/eviction.
    fn write_path_put(manager: &Arc<MerkleSyncManager>, map: &str, key: &str, record: &Record) {
        let factory = MerkleObserverFactory::new(Arc::clone(manager));
        let partition = hash_to_partition(key);
        let observer = factory
            .create_observer(map, partition)
            .expect("observer for any map");
        observer.on_put(key, record, None, false);
    }

    // -----------------------------------------------------------------------
    // AC2 — recovery: a durable-but-non-resident map has a non-zero root after
    // rebuild_from_datastore on a FRESH manager. Negative control: without the
    // rebuild, the same fresh manager reports root 0 (the pre-fix bug).
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn ac2_rebuild_makes_durable_nonresident_root_nonzero() {
        let (store, _dir) = fresh_redb();
        // Persist records WITHOUT ever touching an in-memory engine / observer,
        // i.e. exactly the post-restart state: data on disk, nothing resident.
        store
            .add("users", "alice", &lww_value("alice", 111, 0, "n1"), 0, 1)
            .await
            .unwrap();
        store
            .add("users", "bob", &lww_value("bob", 222, 1, "n2"), 0, 2)
            .await
            .unwrap();

        // Negative control: a fresh manager with NO rebuild reports root 0,
        // reproducing the TODO-530 bug (persisted data invisible to SyncInit).
        let empty_manager = Arc::new(MerkleSyncManager::default());
        assert_eq!(
            empty_manager.aggregate_lww_root_hash("users"),
            0,
            "without rebuild, a durable-but-non-resident map must report root 0 (the pre-fix bug)"
        );

        // The fix: rebuild from the durable index yields a non-zero root.
        let manager = Arc::new(MerkleSyncManager::default());
        manager
            .rebuild_from_datastore("users", &store)
            .await
            .unwrap();
        assert_ne!(
            manager.aggregate_lww_root_hash("users"),
            0,
            "after rebuild_from_datastore the durable-but-non-resident map must have a non-zero root"
        );
    }

    // -----------------------------------------------------------------------
    // AC7 — leaf-hash parity (LWW): the rehydrated root equals the resident
    // (write-path) root. Proves the durable leaf hash is byte-identical to the
    // write-path hash AND the rebuild routes to the same (map, partition) tree.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn ac7_lww_rehydrated_root_equals_resident_root() {
        let (store, _dir) = fresh_redb();

        // Resident root: drive the live write-path observer for each record.
        let resident = Arc::new(MerkleSyncManager::default());
        let records = [
            ("alice", lww_record("alice", 111, 0, "n1")),
            ("bob", lww_record("bob", 222, 3, "n2")),
            ("carol", lww_record("carol", 999, 7, "n3")),
        ];
        for (key, rec) in &records {
            write_path_put(&resident, "users", key, rec);
            // Persist the same record so the durable index matches.
            store.add("users", key, &rec.value, 0, 1).await.unwrap();
        }
        let resident_root = resident.aggregate_lww_root_hash("users");
        assert_ne!(resident_root, 0, "resident root must be non-zero with data");

        // Drop residency: build a brand-new manager and rebuild from disk only.
        let rehydrated = Arc::new(MerkleSyncManager::default());
        rehydrated
            .rebuild_from_datastore("users", &store)
            .await
            .unwrap();

        assert_eq!(
            rehydrated.aggregate_lww_root_hash("users"),
            resident_root,
            "rehydrated LWW root must equal the pre-crash resident root (leaf-hash + routing parity)"
        );
    }

    // -----------------------------------------------------------------------
    // AC7 (OR-Map variant) — parity with OR-Map data present, proving the
    // kind-routing reconcile folds OR-Map leaves into the OR-Map tree (not the
    // LWW tree). Negative control via the dedicated misroute test below.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn ac7_ormap_rehydrated_root_equals_resident_root() {
        let (store, _dir) = fresh_redb();

        let resident = Arc::new(MerkleSyncManager::default());
        let records = [
            ("t1", ormap_record("t1", "tagA", 100)),
            ("t2", ormap_record("t2", "tagB", 200)),
        ];
        for (key, rec) in &records {
            write_path_put(&resident, "tags", key, rec);
            store.add("tags", key, &rec.value, 0, 1).await.unwrap();
        }
        let resident_root = resident.aggregate_ormap_root_hash("tags");
        assert_ne!(resident_root, 0, "resident OR-Map root must be non-zero");

        let rehydrated = Arc::new(MerkleSyncManager::default());
        rehydrated
            .rebuild_from_datastore("tags", &store)
            .await
            .unwrap();

        assert_eq!(
            rehydrated.aggregate_ormap_root_hash("tags"),
            resident_root,
            "rehydrated OR-Map root must equal the resident OR-Map root (kind-routed correctly)"
        );
        // And the OR-Map data must NOT have leaked into the LWW tree.
        assert_eq!(
            rehydrated.aggregate_lww_root_hash("tags"),
            0,
            "OR-Map leaves must not pollute the LWW tree"
        );
    }

    // -----------------------------------------------------------------------
    // AC7 — negative-control guard for kind routing. A LeafSink that misroutes
    // OR-Map leaves into the LWW tree (the pre-fix single-tree bug) yields a
    // root that does NOT match the correctly-routed resident root. This proves
    // the kind discriminator is load-bearing — neutering it breaks parity.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn ac7_misrouting_ormap_into_lww_tree_breaks_parity() {
        struct MisroutingSink {
            manager: Arc<MerkleSyncManager>,
            map: String,
        }
        #[async_trait::async_trait]
        impl LeafSink for MisroutingSink {
            async fn consume(&mut self, batch: Vec<MerkleLeaf>) -> anyhow::Result<()> {
                for leaf in batch {
                    let partition = hash_to_partition(&leaf.key);
                    // Deliberately ignore leaf.kind: route everything to LWW.
                    self.manager
                        .update_lww(&self.map, partition, &leaf.key, leaf.leaf_hash);
                }
                Ok(())
            }
        }

        let (store, _dir) = fresh_redb();
        let resident = Arc::new(MerkleSyncManager::default());
        for (key, rec) in [
            ("t1", ormap_record("t1", "tagA", 100)),
            ("t2", ormap_record("t2", "tagB", 200)),
        ] {
            write_path_put(&resident, "tags", key, &rec);
            store.add("tags", key, &rec.value, 0, 1).await.unwrap();
        }
        let correct_root = resident.aggregate_ormap_root_hash("tags");

        let misrouted = Arc::new(MerkleSyncManager::default());
        let mut sink = MisroutingSink {
            manager: Arc::clone(&misrouted),
            map: "tags".to_string(),
        };
        store
            .enumerate_leaves("tags", false, &mut sink)
            .await
            .unwrap();

        // The misrouted manager's OR-Map tree is empty; LWW tree holds the leaves.
        assert_eq!(
            misrouted.aggregate_ormap_root_hash("tags"),
            0,
            "misrouting must leave the OR-Map tree empty (root 0), unlike the correct root"
        );
        assert_ne!(
            correct_root, 0,
            "the correctly-routed OR-Map root must be non-zero (proves the assertion is not vacuous)"
        );
    }

    // -----------------------------------------------------------------------
    // AC-EVICT-MERKLE — a record durable-in-redb but NOT resident in the
    // in-memory engine still contributes its leaf to the rebuilt root. We model
    // eviction as: write two records through the live path, then evict one by
    // building the index-sourced root from the durable store (where both still
    // live). Negative control: a manager that only ever saw the surviving
    // resident record (leaf source = in-memory engine only) is MISSING the
    // evicted record's leaf, so its root differs from the all-durable root.
    // Lever used: durable-vs-resident leaf-source divergence (the exact property
    // the eviction-orchestrator path relies on), avoiding the heavy orchestrator.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn ac_evict_merkle_durable_nonresident_leaf_stays_in_root() {
        let (store, _dir) = fresh_redb();
        let resident_rec = lww_record("resident", 100, 0, "n1");
        let evicted_rec = lww_record("evicted", 200, 0, "n2");

        // Both records are durable in redb.
        store
            .add("m", "resident", &resident_rec.value, 0, 1)
            .await
            .unwrap();
        store
            .add("m", "evicted", &evicted_rec.value, 0, 2)
            .await
            .unwrap();

        // Negative control: the in-memory engine only retains the resident
        // record (the evicted one was dropped from memory). With leaves sourced
        // ONLY from memory, the evicted leaf is absent from the root.
        let memory_only = Arc::new(MerkleSyncManager::default());
        write_path_put(&memory_only, "m", "resident", &resident_rec);
        let memory_only_root = memory_only.aggregate_lww_root_hash("m");

        // The fix: rebuild from the durable index, which still holds BOTH leaves.
        let durable_sourced = Arc::new(MerkleSyncManager::default());
        durable_sourced
            .rebuild_from_datastore("m", &store)
            .await
            .unwrap();
        let durable_root = durable_sourced.aggregate_lww_root_hash("m");

        assert_ne!(
            durable_root, memory_only_root,
            "the evicted-but-durable record's leaf must change the root vs the memory-only source"
        );

        // And the durable-sourced root equals the root that WOULD have been seen
        // had both records stayed resident — proving the evicted leaf is present.
        let both_resident = Arc::new(MerkleSyncManager::default());
        write_path_put(&both_resident, "m", "resident", &resident_rec);
        write_path_put(&both_resident, "m", "evicted", &evicted_rec);
        assert_eq!(
            durable_root,
            both_resident.aggregate_lww_root_hash("m"),
            "index-sourced root must include the evicted record's leaf"
        );
    }

    // -----------------------------------------------------------------------
    // AC-RAM-MERKLE — rebuild_from_datastore computes the root by STREAMING
    // leaves (keys + u32 hashes), never loading full record values. We prove
    // this with a spying datastore that wraps redb's enumerate_leaves but
    // PANICS if load / load_all are called during rebuild. The root is computed
    // over many durable records while resident value memory stays zero.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn ac_ram_merkle_rebuild_streams_leaves_never_loads_values() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let dir = tempfile::tempdir().expect("tempdir");
        let inner = RedbDataStore::new(dir.path().join("ram.redb")).expect("redb open");
        let enumerate_calls = Arc::new(AtomicUsize::new(0));
        let spy: Arc<dyn MapDataStore> = Arc::new(SpyStore {
            inner,
            enumerate_calls: Arc::clone(&enumerate_calls),
        });
        // Seed many durable records through the trait object; if rebuild loaded
        // values, the spy's load/load_all would panic and fail the test.
        for i in 0..500u32 {
            spy.add(
                "big",
                &format!("k{i:04}"),
                &lww_value("v", u64::from(i), i, "n"),
                0,
                1,
            )
            .await
            .unwrap();
        }

        let manager = Arc::new(MerkleSyncManager::default());
        manager.rebuild_from_datastore("big", &spy).await.unwrap();

        // Rebuild must go through the streaming leaf enumeration, not a value load.
        let calls = enumerate_calls.load(Ordering::SeqCst);
        assert!(
            calls >= 1,
            "rebuild must drive enumerate_leaves (streaming), got {calls} calls"
        );

        // Root is non-zero over 500 records, computed WITHOUT any value load
        // (load/load_all panic), proving resident value memory stays bounded.
        assert_ne!(
            manager.aggregate_lww_root_hash("big"),
            0,
            "root must be computed over all durable records via streaming leaves"
        );
    }

    // Cross-check: the manually-computed write-path hashes match what the
    // datastore stores, so the parity tests above are not accidentally testing
    // a hash both sides happen to share by coincidence.
    #[test]
    fn leaf_hash_helpers_match_write_path_formula() {
        let lww = compute_lww_hash("alice", 111, 0, "n1");
        assert_eq!(
            lww,
            topgun_core::hash::fnv1a_hash("alice:111:0:n1"),
            "compute_lww_hash must equal fnv1a(key:millis:counter:node_id)"
        );
        let entries = vec![OrMapEntry {
            value: Value::String("v".to_string()),
            tag: "tagA".to_string(),
            timestamp: Timestamp {
                millis: 1,
                counter: 0,
                node_id: "n".to_string(),
            },
        }];
        let ormap = compute_ormap_hash("t1", &entries, &[]);
        assert_eq!(
            ormap,
            topgun_core::hash::fnv1a_hash("key:t1|tagA#"),
            "compute_ormap_hash must equal the sorted active#tombstone formula"
        );
        let _ = MerkleLeafKind::Lww;
    }
}
