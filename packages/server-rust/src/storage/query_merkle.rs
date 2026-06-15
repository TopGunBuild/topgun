//! Per-query Merkle tree manager for query-aware delta sync.
//!
//! `QueryMerkleSyncManager` manages per-`(query_id, map_name, partition_id)` Merkle
//! trees for query-aware delta sync.
//!
//! Trees are populated at subscribe time via `init_tree` and cleaned up on
//! unsubscribe/disconnect via `cleanup_query`. The traversal protocol reuses
//! the existing `SyncRespRoot`/`SyncRespBuckets`/`SyncRespLeaf` wire messages,
//! with query-prefixed paths (e.g. `"query:<query_id>/<partition_id>/<depth>/<bucket>"`)
//! to distinguish them from regular partition paths.

use dashmap::DashMap;
use parking_lot::Mutex;
use topgun_core::hash::combine_hashes;
use topgun_core::merkle::MerkleTree;

// ---------------------------------------------------------------------------
// QueryMerkleSyncManager
// ---------------------------------------------------------------------------

/// Per-query Merkle tree manager for query-aware delta sync.
///
/// Maintains a separate `MerkleTree` per `(query_id, map_name, partition_id)` triple.
/// Trees are wrapped in `Mutex` to avoid holding the `DashMap` shard lock during
/// tree operations.
///
/// Trees are populated at subscribe time via `init_tree` and removed on
/// unsubscribe or disconnect via `cleanup_query`. Mutation updates keep trees
/// current as records enter/update/leave query results.
pub struct QueryMerkleSyncManager {
    /// Key: (`query_id`, `map_name`, `partition_id`) -> `MerkleTree`
    trees: DashMap<(String, String, u32), Mutex<MerkleTree>>,
    /// Tree depth for Merkle trees.
    depth: usize,
}

impl QueryMerkleSyncManager {
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

    /// Builds (or replaces) the Merkle tree for `(query_id, map_name, partition_id)`
    /// from a slice of `(key, hash)` pairs.
    ///
    /// Called by `QueryService` at subscribe time to populate the initial tree
    /// from records that matched the query filter.
    pub fn init_tree(
        &self,
        query_id: &str,
        map_name: &str,
        partition_id: u32,
        matching_keys: &[(String, u32)],
    ) {
        let key = (query_id.to_string(), map_name.to_string(), partition_id);
        let mut tree = MerkleTree::new(self.depth);
        for (k, h) in matching_keys {
            tree.update(k, *h);
        }
        self.trees.insert(key, Mutex::new(tree));
    }

    /// Updates a single entry in the query's Merkle tree.
    ///
    /// Used when a mutation occurs on a key that matches the query filter
    /// (ENTER or UPDATE event). No-ops if the tree does not exist.
    pub fn update_entry(
        &self,
        query_id: &str,
        map_name: &str,
        partition_id: u32,
        key: &str,
        hash: u32,
    ) {
        let k = (query_id.to_string(), map_name.to_string(), partition_id);
        if let Some(entry) = self.trees.get(&k) {
            entry.lock().update(key, hash);
        }
    }

    /// Removes an entry from the query's Merkle tree.
    ///
    /// Used when a record no longer matches the query filter (LEAVE event).
    /// No-ops if the tree does not exist.
    pub fn remove_entry(&self, query_id: &str, map_name: &str, partition_id: u32, key: &str) {
        let k = (query_id.to_string(), map_name.to_string(), partition_id);
        if let Some(entry) = self.trees.get(&k) {
            entry.lock().remove(key);
        }
    }

    /// Returns the root hash for `(query_id, map_name, partition_id)`.
    ///
    /// Returns `0` if no tree exists for the given triple.
    #[must_use]
    pub fn get_root_hash(&self, query_id: &str, map_name: &str, partition_id: u32) -> u32 {
        let k = (query_id.to_string(), map_name.to_string(), partition_id);
        self.trees
            .get(&k)
            .map_or(0, |entry| entry.lock().get_root_hash())
    }

    /// Computes the aggregate root hash across all partitions for `(query_id, map_name)`.
    ///
    /// Combines per-partition root hashes with the collision-resistant
    /// `combine_hashes`. It is commutative and associative, so the result is
    /// independent of `DashMap`'s non-deterministic iteration order, while
    /// preventing compensating per-partition hashes from producing an identical
    /// query root (which the client treats as the query-scoped in-sync signal).
    /// Returns `0` if no partitions exist.
    #[must_use]
    pub fn aggregate_query_root_hash(&self, query_id: &str, map_name: &str) -> u32 {
        let hashes: Vec<u32> = self
            .trees
            .iter()
            .filter(|entry| {
                let (qid, mn, _) = entry.key();
                qid == query_id && mn == map_name
            })
            .map(|entry| entry.value().lock().get_root_hash())
            .collect();
        combine_hashes(&hashes)
    }

    /// Removes all Merkle trees for a given `query_id`.
    ///
    /// Called on unsubscribe or disconnect. Uses `DashMap::retain` which holds
    /// the shard lock internally -- safe for removing during iteration, without
    /// the deadlock risk of external iteration-while-removing.
    pub fn cleanup_query(&self, query_id: &str) {
        self.trees.retain(|(qid, _, _), _| qid != query_id);
    }

    /// Provides closure-based mutable access to the tree for `(query_id, map_name, partition_id)`.
    ///
    /// The closure `f` receives `&mut MerkleTree` and its return value is
    /// propagated to the caller. Returns `None` if the tree does not exist.
    ///
    /// The `Mutex` is held only for the duration of `f`. Callers MUST NOT hold
    /// the closure open across `.await` points.
    pub fn with_tree<R>(
        &self,
        query_id: &str,
        map_name: &str,
        partition_id: u32,
        f: impl FnOnce(&mut MerkleTree) -> R,
    ) -> Option<R> {
        let k = (query_id.to_string(), map_name.to_string(), partition_id);
        self.trees.get(&k).map(|entry| {
            let mut guard = entry.lock();
            f(&mut guard)
        })
    }
}

impl Default for QueryMerkleSyncManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregate_query_root_hash_empty_returns_zero() {
        let manager = QueryMerkleSyncManager::new();
        assert_eq!(
            manager.aggregate_query_root_hash("q1", "users"),
            0,
            "no partitions should produce aggregate hash = 0"
        );
    }

    #[test]
    fn aggregate_query_root_hash_combines_partitions() {
        let manager = QueryMerkleSyncManager::new();
        manager.init_tree("q1", "users", 1, &[("alice".to_string(), 111)]);
        manager.init_tree("q1", "users", 2, &[("bob".to_string(), 222)]);

        let hash_1 = manager.get_root_hash("q1", "users", 1);
        let hash_2 = manager.get_root_hash("q1", "users", 2);
        let expected = combine_hashes(&[hash_1, hash_2]);

        assert_eq!(
            manager.aggregate_query_root_hash("q1", "users"),
            expected,
            "aggregate should equal combine_hashes of all partition hashes"
        );
    }

    #[test]
    fn aggregate_query_root_hash_resists_compensating_partition_pairs() {
        // Two per-partition hash sets that are equal under a plain additive fold
        // (0xAAAA0000 + 0x00005555 == 0xAAAA5555 + 0x00000000). The query-scoped
        // aggregate must keep them distinct via combine_hashes so compensating
        // per-partition hashes can never produce an identical query root, which
        // the client treats as the query-scoped "in sync" signal.
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
}
