//! Merkle tree sync manager and mutation observer for delta synchronization.
//!
//! Provides per-partition Merkle tree management ([`MerkleSyncManager`]) and
//! a [`MutationObserver`] implementation ([`MerkleMutationObserver`]) that keeps
//! trees in sync with `RecordStore` mutations automatically.

use std::sync::Arc;

use dashmap::DashMap;
use parking_lot::Mutex;
use topgun_core::hash::fnv1a_hash;
use topgun_core::merkle::{MerkleTree, ORMapMerkleTree};

use super::factory::ObserverFactory;
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
/// Sorts tags for determinism, then hashes `"key:tag1|tag2|..."`.
fn compute_ormap_hash(key: &str, records: &[super::record::OrMapEntry]) -> u32 {
    let mut tags: Vec<&str> = records.iter().map(|r| r.tag.as_str()).collect();
    tags.sort_unstable();
    let joined = tags.join("|");
    fnv1a_hash(&format!("key:{key}|{joined}"))
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

    /// Client-sync partition ID. All Merkle trees at partition 0 serve as
    /// cross-partition aggregates for client-facing delta sync.
    const CLIENT_SYNC_PARTITION: u32 = 0;

    /// Updates the appropriate Merkle tree based on the `RecordValue` variant.
    ///
    /// Dual-writes to both `self.partition_id` (the record's actual partition)
    /// and partition 0 (the client-sync aggregate). When `self.partition_id`
    /// is already 0, only one write occurs.
    fn update_tree(&self, key: &str, value: &RecordValue) {
        match value {
            RecordValue::Lww { timestamp, .. } => {
                let hash =
                    compute_lww_hash(key, timestamp.millis, timestamp.counter, &timestamp.node_id);
                self.manager
                    .update_lww(&self.map_name, self.partition_id, key, hash);
                if self.partition_id != Self::CLIENT_SYNC_PARTITION {
                    self.manager
                        .update_lww(&self.map_name, Self::CLIENT_SYNC_PARTITION, key, hash);
                }
            }
            RecordValue::OrMap { records } => {
                let hash = compute_ormap_hash(key, records);
                self.manager
                    .update_ormap(&self.map_name, self.partition_id, key, hash);
                if self.partition_id != Self::CLIENT_SYNC_PARTITION {
                    self.manager
                        .update_ormap(&self.map_name, Self::CLIENT_SYNC_PARTITION, key, hash);
                }
            }
            RecordValue::OrTombstones { .. } => {
                // Tombstones represent deletions — remove from OR-Map tree.
                self.manager
                    .remove_ormap(&self.map_name, self.partition_id, key);
                if self.partition_id != Self::CLIENT_SYNC_PARTITION {
                    self.manager
                        .remove_ormap(&self.map_name, Self::CLIENT_SYNC_PARTITION, key);
                }
            }
        }
    }
}

impl MutationObserver for MerkleMutationObserver {
    fn on_put(&self, key: &str, record: &Record, _old_value: Option<&RecordValue>, is_backup: bool) {
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
        // Dual-write: also remove from client-sync partition.
        if self.partition_id != Self::CLIENT_SYNC_PARTITION {
            self.manager
                .remove_lww(&self.map_name, Self::CLIENT_SYNC_PARTITION, key);
            self.manager
                .remove_ormap(&self.map_name, Self::CLIENT_SYNC_PARTITION, key);
        }
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
        // Dual-write: also remove from client-sync partition.
        if self.partition_id != Self::CLIENT_SYNC_PARTITION {
            self.manager
                .remove_lww(&self.map_name, Self::CLIENT_SYNC_PARTITION, key);
            self.manager
                .remove_ormap(&self.map_name, Self::CLIENT_SYNC_PARTITION, key);
        }
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
        assert!(observer2.is_some(), "factory should return Some for any partition");
    }

    // ---------------------------------------------------------------------------
    // AC8: MerkleMutationObserver updates LWW tree on put/remove
    // ---------------------------------------------------------------------------

    #[test]
    fn ac8_lww_tree_updates_on_put() {
        let manager = Arc::new(MerkleSyncManager::default());
        let observer = MerkleMutationObserver::new(
            Arc::clone(&manager),
            "users".to_string(),
            0,
        );

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
        assert_eq!(hash_after_remove, 0, "root hash should return to 0 after remove");
    }

    #[test]
    fn ac8_backup_mutations_do_not_update_tree() {
        let manager = Arc::new(MerkleSyncManager::default());
        let observer = MerkleMutationObserver::new(
            Arc::clone(&manager),
            "users".to_string(),
            0,
        );

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
        let observer = MerkleMutationObserver::new(
            Arc::clone(&manager),
            "tags".to_string(),
            0,
        );

        // Initially the OR-Map root hash is zero.
        let initial_hash = manager.with_ormap_tree("tags", 0, |tree| tree.get_root_hash());
        assert_eq!(initial_hash, 0, "empty OR-Map tree should have root_hash = 0");

        // After on_put with an OR-Map record, root hash should be non-zero.
        let record = make_ormap_record("tag-1", "1700000000000:0:node-1", 1_700_000_000_000);
        observer.on_put("tag-1", &record, None, false);
        let hash_after_put = manager.with_ormap_tree("tags", 0, |tree| tree.get_root_hash());
        assert_ne!(hash_after_put, 0, "OR-Map root hash should be non-zero after put");

        // After on_remove, OR-Map root hash should return to zero.
        observer.on_remove("tag-1", &record, false);
        let hash_after_remove = manager.with_ormap_tree("tags", 0, |tree| tree.get_root_hash());
        assert_eq!(hash_after_remove, 0, "OR-Map root hash should return to 0 after remove");
    }

    // ---------------------------------------------------------------------------
    // AC10: MerkleSyncManager clear_partition resets trees
    // ---------------------------------------------------------------------------

    #[test]
    fn ac10_clear_partition_resets_both_trees() {
        let manager = Arc::new(MerkleSyncManager::default());
        let observer = MerkleMutationObserver::new(
            Arc::clone(&manager),
            "users".to_string(),
            0,
        );

        // Populate both trees.
        let lww_record = make_lww_record("user-1", 1_700_000_000_000, 0, "node-1");
        observer.on_put("user-1", &lww_record, None, false);

        let ormap_record = make_ormap_record("tag-1", "1700000000000:0:node-1", 1_700_000_000_000);
        let ormap_observer = MerkleMutationObserver::new(
            Arc::clone(&manager),
            "users".to_string(),
            0,
        );
        ormap_observer.on_put("tag-1", &ormap_record, None, false);

        // Verify both trees have non-zero hashes before clearing.
        let lww_hash = manager.with_lww_tree("users", 0, |tree| tree.get_root_hash());
        assert_ne!(lww_hash, 0, "LWW tree should have non-zero hash before clear");
        let ormap_hash = manager.with_ormap_tree("users", 0, |tree| tree.get_root_hash());
        assert_ne!(ormap_hash, 0, "OR-Map tree should have non-zero hash before clear");

        // Clear the partition.
        manager.clear_partition("users", 0);

        // After clearing, both trees should return hash = 0 (new empty trees on next access).
        let lww_hash_after = manager.with_lww_tree("users", 0, |tree| tree.get_root_hash());
        let ormap_hash_after = manager.with_ormap_tree("users", 0, |tree| tree.get_root_hash());
        assert_eq!(lww_hash_after, 0, "LWW tree should have root_hash = 0 after clear_partition");
        assert_eq!(ormap_hash_after, 0, "OR-Map tree should have root_hash = 0 after clear_partition");
    }

    // ---------------------------------------------------------------------------
    // Additional: on_update uses new_value
    // ---------------------------------------------------------------------------

    #[test]
    fn on_update_uses_new_value_not_record_value() {
        let manager = Arc::new(MerkleSyncManager::default());
        let observer = MerkleMutationObserver::new(
            Arc::clone(&manager),
            "users".to_string(),
            0,
        );

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
        assert_ne!(hash_v1, hash_v2, "hash should change after update with new timestamp");
    }

    // ---------------------------------------------------------------------------
    // Additional: on_clear and on_reset
    // ---------------------------------------------------------------------------

    #[test]
    fn on_clear_clears_partition() {
        let manager = Arc::new(MerkleSyncManager::default());
        let observer = MerkleMutationObserver::new(
            Arc::clone(&manager),
            "users".to_string(),
            0,
        );

        let record = make_lww_record("user-1", 1_000, 0, "node-1");
        observer.on_put("user-1", &record, None, false);

        observer.on_clear();

        let hash = manager.with_lww_tree("users", 0, |tree| tree.get_root_hash());
        assert_eq!(hash, 0, "on_clear should reset the partition");
    }

    #[test]
    fn on_reset_clears_partition() {
        let manager = Arc::new(MerkleSyncManager::default());
        let observer = MerkleMutationObserver::new(
            Arc::clone(&manager),
            "users".to_string(),
            0,
        );

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
    // Dual-write: partition N observers also update partition 0
    // ---------------------------------------------------------------------------

    #[test]
    fn dual_write_lww_put_updates_partition_0() {
        let manager = Arc::new(MerkleSyncManager::default());
        // Observer for partition 42 (non-zero).
        let observer = MerkleMutationObserver::new(
            Arc::clone(&manager),
            "users".to_string(),
            42,
        );

        let record = make_lww_record("alice", 1_700_000_000_000, 0, "node-1");
        observer.on_put("alice", &record, None, false);

        // Partition 42 should have the record.
        let hash_42 = manager.with_lww_tree("users", 42, |tree| tree.get_root_hash());
        assert_ne!(hash_42, 0, "partition 42 should have non-zero hash");

        // Partition 0 (client-sync) should also have the record.
        let hash_0 = manager.with_lww_tree("users", 0, |tree| tree.get_root_hash());
        assert_ne!(hash_0, 0, "partition 0 should have non-zero hash via dual-write");

        // Both should have the same root hash (same single key, same value).
        assert_eq!(hash_42, hash_0, "partition 42 and partition 0 should match");
    }

    #[test]
    fn dual_write_ormap_put_updates_partition_0() {
        let manager = Arc::new(MerkleSyncManager::default());
        let observer = MerkleMutationObserver::new(
            Arc::clone(&manager),
            "tags".to_string(),
            42,
        );

        let record = make_ormap_record("tag-1", "1700000000000:0:node-1", 1_700_000_000_000);
        observer.on_put("tag-1", &record, None, false);

        let hash_42 = manager.with_ormap_tree("tags", 42, |tree| tree.get_root_hash());
        assert_ne!(hash_42, 0, "partition 42 OR-Map should have non-zero hash");

        let hash_0 = manager.with_ormap_tree("tags", 0, |tree| tree.get_root_hash());
        assert_ne!(hash_0, 0, "partition 0 OR-Map should have non-zero hash via dual-write");
    }

    #[test]
    fn dual_write_remove_clears_from_partition_0() {
        let manager = Arc::new(MerkleSyncManager::default());
        let observer = MerkleMutationObserver::new(
            Arc::clone(&manager),
            "users".to_string(),
            42,
        );

        let record = make_lww_record("alice", 1_700_000_000_000, 0, "node-1");
        observer.on_put("alice", &record, None, false);

        // Verify non-zero before remove.
        let hash_0_before = manager.with_lww_tree("users", 0, |tree| tree.get_root_hash());
        assert_ne!(hash_0_before, 0);

        observer.on_remove("alice", &record, false);

        // Both partitions should be empty.
        let hash_42 = manager.with_lww_tree("users", 42, |tree| tree.get_root_hash());
        let hash_0 = manager.with_lww_tree("users", 0, |tree| tree.get_root_hash());
        assert_eq!(hash_42, 0, "partition 42 should be empty after remove");
        assert_eq!(hash_0, 0, "partition 0 should be empty after dual-write remove");
    }

    #[test]
    fn dual_write_evict_clears_from_partition_0() {
        let manager = Arc::new(MerkleSyncManager::default());
        let observer = MerkleMutationObserver::new(
            Arc::clone(&manager),
            "users".to_string(),
            42,
        );

        let record = make_lww_record("alice", 1_700_000_000_000, 0, "node-1");
        observer.on_put("alice", &record, None, false);

        observer.on_evict("alice", &record, false);

        let hash_0 = manager.with_lww_tree("users", 0, |tree| tree.get_root_hash());
        assert_eq!(hash_0, 0, "partition 0 should be empty after dual-write evict");
    }

    #[test]
    fn on_clear_does_not_clear_partition_0_for_non_zero_partition() {
        let manager = Arc::new(MerkleSyncManager::default());

        // Observer at partition 42 writes a record (dual-writes to partition 0).
        let observer_42 = MerkleMutationObserver::new(
            Arc::clone(&manager),
            "users".to_string(),
            42,
        );
        let record = make_lww_record("alice", 1_700_000_000_000, 0, "node-1");
        observer_42.on_put("alice", &record, None, false);

        // Partition 0 should have data.
        let hash_0_before = manager.with_lww_tree("users", 0, |tree| tree.get_root_hash());
        assert_ne!(hash_0_before, 0);

        // Clearing partition 42 should NOT clear partition 0.
        observer_42.on_clear();

        let hash_42 = manager.with_lww_tree("users", 42, |tree| tree.get_root_hash());
        assert_eq!(hash_42, 0, "partition 42 should be cleared");

        let hash_0_after = manager.with_lww_tree("users", 0, |tree| tree.get_root_hash());
        assert_ne!(hash_0_after, 0, "partition 0 should NOT be cleared when a non-zero partition clears");
    }

    #[test]
    fn partition_0_observer_does_not_double_write() {
        let manager = Arc::new(MerkleSyncManager::default());
        // Observer at partition 0 should write only once, not twice.
        let observer = MerkleMutationObserver::new(
            Arc::clone(&manager),
            "users".to_string(),
            0,
        );

        let record = make_lww_record("alice", 1_700_000_000_000, 0, "node-1");
        observer.on_put("alice", &record, None, false);

        let hash_0 = manager.with_lww_tree("users", 0, |tree| tree.get_root_hash());
        assert_ne!(hash_0, 0, "partition 0 should have the record");
    }

    #[test]
    fn dual_write_replication_put_updates_partition_0() {
        let manager = Arc::new(MerkleSyncManager::default());
        let observer = MerkleMutationObserver::new(
            Arc::clone(&manager),
            "users".to_string(),
            42,
        );

        let record = make_lww_record("alice", 1_700_000_000_000, 0, "node-1");
        // on_replication_put does NOT check is_backup (by design).
        observer.on_replication_put("alice", &record, false);

        let hash_0 = manager.with_lww_tree("users", 0, |tree| tree.get_root_hash());
        assert_ne!(hash_0, 0, "partition 0 should have data after replication put dual-write");
    }

    // ---------------------------------------------------------------------------
    // Additional: clear_all
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
