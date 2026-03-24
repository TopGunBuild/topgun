//! Per-query Merkle tree manager for query-aware delta sync.
//!
//! `QueryMerkleSyncManager` manages per-`(query_id, map_name, partition_id)` Merkle
//! trees, following the same patterns as `ShapeMerkleSyncManager` for shape sync.
//!
//! Trees are populated at subscribe time via `init_tree` and cleaned up on
//! unsubscribe/disconnect via `cleanup_query`. The traversal protocol reuses
//! the existing `SyncRespRoot`/`SyncRespBuckets`/`SyncRespLeaf` wire messages,
//! with query-prefixed paths (e.g. `"query:<query_id>/<partition_id>/<depth>/<bucket>"`)
//! to distinguish them from regular partition paths and shape-prefixed paths.

use dashmap::DashMap;
use parking_lot::Mutex;
use topgun_core::merkle::MerkleTree;

// ---------------------------------------------------------------------------
// QueryMerkleSyncManager
// ---------------------------------------------------------------------------

/// Per-query Merkle tree manager for query-aware delta sync.
///
/// Maintains a separate `MerkleTree` per `(query_id, map_name, partition_id)` triple.
/// Trees are wrapped in `Mutex` to avoid holding the `DashMap` shard lock during
/// tree operations -- the same pattern as `ShapeMerkleSyncManager`.
///
/// Trees are populated at subscribe time via `init_tree` and removed on
/// unsubscribe or disconnect via `cleanup_query`. Mutation updates keep trees
/// current as records enter/update/leave query results.
pub struct QueryMerkleSyncManager {
    /// Key: (`query_id`, `map_name`, `partition_id`) -> `MerkleTree`
    trees: DashMap<(String, String, u32), Mutex<MerkleTree>>,
    /// Tree depth -- matches the depth used by `ShapeMerkleSyncManager`.
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
        todo!()
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
        todo!()
    }

    /// Removes an entry from the query's Merkle tree.
    ///
    /// Used when a record no longer matches the query filter (LEAVE event).
    /// No-ops if the tree does not exist.
    pub fn remove_entry(
        &self,
        query_id: &str,
        map_name: &str,
        partition_id: u32,
        key: &str,
    ) {
        todo!()
    }

    /// Returns the root hash for `(query_id, map_name, partition_id)`.
    ///
    /// Returns `0` if no tree exists for the given triple.
    #[must_use]
    pub fn get_root_hash(&self, query_id: &str, map_name: &str, partition_id: u32) -> u32 {
        todo!()
    }

    /// Computes the aggregate root hash across all partitions for `(query_id, map_name)`.
    ///
    /// Accumulates per-partition root hashes using `wrapping_add`. This is
    /// commutative and associative, so the result is independent of `DashMap`'s
    /// non-deterministic iteration order. Returns `0` if no partitions exist.
    #[must_use]
    pub fn aggregate_query_root_hash(&self, query_id: &str, map_name: &str) -> u32 {
        todo!()
    }

    /// Removes all Merkle trees for a given `query_id`.
    ///
    /// Called on unsubscribe or disconnect. Uses `DashMap::retain` which holds
    /// the shard lock internally -- safe for removing during iteration, without
    /// the deadlock risk of external iteration-while-removing.
    pub fn cleanup_query(&self, query_id: &str) {
        todo!()
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
        todo!()
    }
}

impl Default for QueryMerkleSyncManager {
    fn default() -> Self {
        Self::new()
    }
}
