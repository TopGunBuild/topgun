//! Per-shape Merkle tree manager for shape-aware delta sync.
//!
//! `ShapeMerkleSyncManager` manages per-`(shape_id, map_name, partition_id)` Merkle
//! trees, following the same patterns as `MerkleSyncManager` for full-map sync.
//!
//! Trees are populated at subscribe time via `init_tree` and cleaned up on
//! unsubscribe/disconnect via `cleanup_shape`. The traversal protocol reuses
//! the existing `SyncRespRoot`/`SyncRespBuckets`/`SyncRespLeaf` wire messages,
//! with shape-prefixed paths (e.g. `"<shape_id>/<partition_id>/<depth>/<bucket>"`)
//! to distinguish them from regular partition paths.

use dashmap::DashMap;
use parking_lot::Mutex;
use topgun_core::merkle::MerkleTree;

// ---------------------------------------------------------------------------
// ShapeMerkleSyncManager
// ---------------------------------------------------------------------------

/// Per-shape Merkle tree manager for shape-aware delta sync.
///
/// Maintains a separate `MerkleTree` per `(shape_id, map_name, partition_id)` triple.
/// Trees are wrapped in `Mutex` to avoid holding the `DashMap` shard lock during
/// tree operations — the same pattern as `MerkleSyncManager`.
///
/// Trees are populated at subscribe time via `init_tree` and removed on
/// unsubscribe or disconnect via `cleanup_shape`. Mutation updates are deferred
/// to a future spec; until then, trees reflect the state at subscribe time.
pub struct ShapeMerkleSyncManager {
    /// Key: (`shape_id`, `map_name`, `partition_id`) -> `MerkleTree`
    trees: DashMap<(String, String, u32), Mutex<MerkleTree>>,
    /// Tree depth — matches the depth used by `MerkleSyncManager`.
    depth: usize,
}

impl ShapeMerkleSyncManager {
    /// Default tree depth: 3 levels = 4096 leaf buckets.
    pub const DEFAULT_DEPTH: usize = 3;

    /// Creates a new manager with the default tree depth (3).
    #[must_use]
    pub fn new() -> Self {
        Self {
            trees: DashMap::new(),
            depth: Self::DEFAULT_DEPTH,
        }
    }

    /// Builds (or replaces) the Merkle tree for `(shape_id, map_name, partition_id)`
    /// from a slice of `(key, hash)` pairs.
    ///
    /// Called by `ShapeService` at subscribe time to populate the initial tree
    /// from records that matched the shape filter.
    pub fn init_tree(
        &self,
        shape_id: &str,
        map_name: &str,
        partition_id: u32,
        matching_keys: &[(String, u32)],
    ) {
        let key = (shape_id.to_string(), map_name.to_string(), partition_id);
        let mut tree = MerkleTree::new(self.depth);
        for (k, h) in matching_keys {
            tree.update(k, *h);
        }
        self.trees.insert(key, Mutex::new(tree));
    }

    /// Updates a single entry in the shape's Merkle tree.
    ///
    /// Used when a mutation occurs on a key that matches the shape filter
    /// (ENTER or UPDATE event). No-ops if the tree does not exist.
    pub fn update_entry(
        &self,
        shape_id: &str,
        map_name: &str,
        partition_id: u32,
        key: &str,
        hash: u32,
    ) {
        let k = (shape_id.to_string(), map_name.to_string(), partition_id);
        if let Some(entry) = self.trees.get(&k) {
            entry.lock().update(key, hash);
        }
    }

    /// Removes an entry from the shape's Merkle tree.
    ///
    /// Used when a record no longer matches the shape filter (LEAVE event).
    /// No-ops if the tree does not exist.
    pub fn remove_entry(
        &self,
        shape_id: &str,
        map_name: &str,
        partition_id: u32,
        key: &str,
    ) {
        let k = (shape_id.to_string(), map_name.to_string(), partition_id);
        if let Some(entry) = self.trees.get(&k) {
            entry.lock().remove(key);
        }
    }

    /// Returns the root hash for `(shape_id, map_name, partition_id)`.
    ///
    /// Returns `0` if no tree exists for the given triple.
    #[must_use]
    pub fn get_root_hash(&self, shape_id: &str, map_name: &str, partition_id: u32) -> u32 {
        let k = (shape_id.to_string(), map_name.to_string(), partition_id);
        self.trees
            .get(&k)
            .map_or(0, |entry| entry.lock().get_root_hash())
    }

    /// Computes the aggregate root hash across all partitions for `(shape_id, map_name)`.
    ///
    /// Accumulates per-partition root hashes using `wrapping_add`. This is
    /// commutative and associative, so the result is independent of `DashMap`'s
    /// non-deterministic iteration order. Returns `0` if no partitions exist.
    #[must_use]
    pub fn aggregate_shape_root_hash(&self, shape_id: &str, map_name: &str) -> u32 {
        self.trees
            .iter()
            .filter(|entry| {
                let (sid, mn, _) = entry.key();
                sid == shape_id && mn == map_name
            })
            .fold(0u32, |acc, entry| {
                let hash = entry.value().lock().get_root_hash();
                acc.wrapping_add(hash)
            })
    }

    /// Removes all Merkle trees for a given `shape_id`.
    ///
    /// Called on unsubscribe or disconnect. Uses `DashMap::retain` which holds
    /// the shard lock internally — safe for removing during iteration, without
    /// the deadlock risk of external iteration-while-removing.
    pub fn cleanup_shape(&self, shape_id: &str) {
        self.trees.retain(|(sid, _, _), _| sid != shape_id);
    }

    /// Provides closure-based mutable access to the tree for `(shape_id, map_name, partition_id)`.
    ///
    /// The closure `f` receives `&mut MerkleTree` and its return value is
    /// propagated to the caller. Returns `None` if the tree does not exist.
    ///
    /// The `Mutex` is held only for the duration of `f`. Callers MUST NOT hold
    /// the closure open across `.await` points.
    pub fn with_tree<R>(
        &self,
        shape_id: &str,
        map_name: &str,
        partition_id: u32,
        f: impl FnOnce(&mut MerkleTree) -> R,
    ) -> Option<R> {
        let k = (shape_id.to_string(), map_name.to_string(), partition_id);
        self.trees.get(&k).map(|entry| {
            let mut guard = entry.lock();
            f(&mut guard)
        })
    }

    /// Returns the partition IDs for which a shape tree exists.
    ///
    /// Used by `SyncService` during shape-prefixed bucket traversal to iterate
    /// over known partitions for a `(shape_id, map_name)` pair.
    #[must_use]
    pub fn partition_ids(&self, shape_id: &str, map_name: &str) -> Vec<u32> {
        self.trees
            .iter()
            .filter_map(|entry| {
                let (sid, mn, pid) = entry.key();
                if sid == shape_id && mn == map_name {
                    Some(*pid)
                } else {
                    None
                }
            })
            .collect()
    }
}

impl Default for ShapeMerkleSyncManager {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_manager() -> ShapeMerkleSyncManager {
        ShapeMerkleSyncManager::new()
    }

    /// Init tree then verify root hash is non-zero.
    #[test]
    fn init_tree_non_zero_root_hash() {
        let mgr = make_manager();
        mgr.init_tree("shape-1", "users", 0, &[("key-1".to_string(), 12345_u32)]);
        assert_ne!(mgr.get_root_hash("shape-1", "users", 0), 0);
    }

    /// Update an entry and verify the root hash changes.
    #[test]
    fn update_entry_changes_root_hash() {
        let mgr = make_manager();
        mgr.init_tree("shape-1", "users", 0, &[("key-1".to_string(), 100_u32)]);
        let before = mgr.get_root_hash("shape-1", "users", 0);
        mgr.update_entry("shape-1", "users", 0, "key-1", 200);
        let after = mgr.get_root_hash("shape-1", "users", 0);
        assert_ne!(before, after);
    }

    /// Remove an entry and verify the root hash changes.
    #[test]
    fn remove_entry_changes_root_hash() {
        let mgr = make_manager();
        mgr.init_tree("shape-1", "users", 0, &[("key-1".to_string(), 100_u32)]);
        let before = mgr.get_root_hash("shape-1", "users", 0);
        mgr.remove_entry("shape-1", "users", 0, "key-1");
        let after = mgr.get_root_hash("shape-1", "users", 0);
        assert_ne!(before, after);
    }

    /// Cleanup shape removes all trees for that shape_id.
    #[test]
    fn cleanup_shape_removes_all_trees() {
        let mgr = make_manager();
        mgr.init_tree("shape-1", "users", 0, &[("key-1".to_string(), 100_u32)]);
        mgr.init_tree("shape-1", "users", 1, &[("key-2".to_string(), 200_u32)]);
        mgr.init_tree("shape-2", "users", 0, &[("key-3".to_string(), 300_u32)]);

        mgr.cleanup_shape("shape-1");

        // shape-1 trees are gone.
        assert_eq!(mgr.get_root_hash("shape-1", "users", 0), 0);
        assert_eq!(mgr.get_root_hash("shape-1", "users", 1), 0);
        // shape-2 tree is unaffected.
        assert_ne!(mgr.get_root_hash("shape-2", "users", 0), 0);
    }

    /// Get root hash of a non-existent tree returns 0.
    #[test]
    fn get_root_hash_nonexistent_returns_zero() {
        let mgr = make_manager();
        assert_eq!(mgr.get_root_hash("nonexistent", "users", 0), 0);
    }

    /// Aggregate root hash sums across multiple partitions.
    #[test]
    fn aggregate_root_hash_sums_partitions() {
        let mgr = make_manager();
        mgr.init_tree("shape-1", "users", 0, &[("key-1".to_string(), 100_u32)]);
        mgr.init_tree("shape-1", "users", 1, &[("key-2".to_string(), 200_u32)]);

        let hash_0 = mgr.get_root_hash("shape-1", "users", 0);
        let hash_1 = mgr.get_root_hash("shape-1", "users", 1);
        let aggregate = mgr.aggregate_shape_root_hash("shape-1", "users");

        assert_ne!(aggregate, 0);
        assert_eq!(aggregate, hash_0.wrapping_add(hash_1));
    }

    /// Aggregate root hash returns 0 when no partitions exist.
    #[test]
    fn aggregate_root_hash_empty_returns_zero() {
        let mgr = make_manager();
        assert_eq!(mgr.aggregate_shape_root_hash("nonexistent", "users"), 0);
    }
}
