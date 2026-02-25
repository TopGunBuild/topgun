//! Merkle tree sync manager and mutation observer for delta synchronization.
//!
//! Provides per-partition Merkle tree management ([`MerkleSyncManager`]) and
//! a [`MutationObserver`] implementation ([`MerkleMutationObserver`]) that keeps
//! trees in sync with `RecordStore` mutations automatically.

use std::sync::Arc;

use dashmap::DashMap;
use parking_lot::Mutex;
use topgun_core::merkle::{MerkleTree, ORMapMerkleTree};

use super::mutation_observer::MutationObserver;

// ---------------------------------------------------------------------------
// MerkleSyncManager
// ---------------------------------------------------------------------------

/// Per-partition Merkle tree manager for delta sync.
///
/// Maintains separate `MerkleTree` (LWW) and `ORMapMerkleTree` (OR-Map) instances
/// per `(map_name, partition_id)` pair. Trees are lazily created on first access.
///
/// All access goes through the `with_lww_tree` / `with_ormap_tree` closure API,
/// which locks only the specific tree's `Mutex` without holding the DashMap shard
/// lock across tree operations. This allows concurrent access to different
/// partitions while serializing mutations on each individual tree.
pub struct MerkleSyncManager {
    /// Key: (map_name, partition_id) -> LWW MerkleTree
    lww_trees: DashMap<(String, u32), Mutex<MerkleTree>>,
    /// Key: (map_name, partition_id) -> OR-Map MerkleTree
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
        // Entry API: insert lazily if absent, then lock and invoke closure.
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
}

impl MutationObserver for MerkleMutationObserver {
    fn on_put(
        &self,
        _key: &str,
        _record: &super::record::Record,
        _old_value: Option<&super::record::RecordValue>,
        _is_backup: bool,
    ) {
        // Implemented in G2
    }

    fn on_update(
        &self,
        _key: &str,
        _record: &super::record::Record,
        _old_value: &super::record::RecordValue,
        _new_value: &super::record::RecordValue,
        _is_backup: bool,
    ) {
        // Implemented in G2
    }

    fn on_remove(&self, _key: &str, _record: &super::record::Record, _is_backup: bool) {
        // Implemented in G2
    }

    fn on_evict(&self, _key: &str, _record: &super::record::Record, _is_backup: bool) {
        // Implemented in G2
    }

    fn on_load(&self, _key: &str, _record: &super::record::Record, _is_backup: bool) {
        // Implemented in G2
    }

    fn on_replication_put(
        &self,
        _key: &str,
        _record: &super::record::Record,
        _populate_index: bool,
    ) {
        // Implemented in G2
    }

    fn on_clear(&self) {
        // Implemented in G2
    }

    fn on_reset(&self) {
        // Implemented in G2
    }

    fn on_destroy(&self, _is_shutdown: bool) {
        // Implemented in G2
    }
}
