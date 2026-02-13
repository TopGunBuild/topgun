//! MerkleTree and `ORMapMerkleTree` for efficient delta synchronization.
//!
//! Both trees use a prefix trie structure keyed by hex digits of the FNV-1a hash
//! of entry keys. The trie depth (default 3) determines bucket granularity.
//! Nodes compare root hashes to identify differing subtrees, then walk down
//! to discover the specific keys that need synchronization.

use std::collections::HashMap;

use crate::hash::fnv1a_hash;

/// A node in the MerkleTree prefix trie.
///
/// Internal nodes have `children` (keyed by hex digit). Leaf nodes have `entries`
/// (keyed by the original string key, mapping to the content hash). The `hash`
/// field is the wrapping sum of all child/entry hashes.
#[derive(Debug, Clone)]
pub struct MerkleNode {
    /// Aggregated hash of all descendants (wrapping sum).
    pub hash: u32,
    /// Child nodes keyed by hex digit (0-9, a-f). Present on internal nodes.
    pub children: HashMap<char, MerkleNode>,
    /// Leaf entries keyed by original key, mapping to content hash. Present on leaf nodes.
    pub entries: HashMap<String, u32>,
}

impl MerkleNode {
    /// Creates a new empty node.
    fn new() -> Self {
        Self {
            hash: 0,
            children: HashMap::new(),
            entries: HashMap::new(),
        }
    }
}

/// Computes the hex path for a key: FNV-1a hash formatted as 8-char hex string.
fn key_to_path(key: &str) -> String {
    format!("{:08x}", fnv1a_hash(key))
}

/// A MerkleTree for efficient delta synchronization of LWW-Maps.
///
/// Uses a prefix trie structure where keys are routed to buckets based on the
/// hex digits of their FNV-1a hash. The tree has a configurable depth (default 3),
/// meaning the first 3 hex digits of the hash determine the bucket.
///
/// # Synchronization protocol
///
/// 1. Compare root hashes -- if equal, maps are in sync.
/// 2. If different, compare level-1 buckets to identify divergent subtrees.
/// 3. Drill down to leaf level to identify specific keys that differ.
/// 4. Exchange only the differing records.
///
/// # Examples
///
/// ```
/// use topgun_core::merkle::MerkleTree;
/// use topgun_core::hash::fnv1a_hash;
///
/// let mut tree = MerkleTree::new(3);
/// let item_hash = fnv1a_hash("key1:100:0:test");
/// tree.update("key1", item_hash);
/// assert_ne!(tree.get_root_hash(), 0);
/// ```
pub struct MerkleTree {
    root: MerkleNode,
    depth: usize,
}

impl MerkleTree {
    /// Creates a new empty `MerkleTree` with the given depth.
    ///
    /// The depth determines how many hex digits of the key hash are used for
    /// bucket routing. Default is 3, giving 4096 possible leaf buckets.
    pub fn new(depth: usize) -> Self {
        Self {
            root: MerkleNode::new(),
            depth,
        }
    }

    /// Creates a new `MerkleTree` with the default depth of 3.
    pub fn default_depth() -> Self {
        Self::new(3)
    }

    /// Updates the tree with a key and its content hash.
    ///
    /// The key is routed to its bucket based on the hex digits of `fnv1a_hash(key)`.
    /// The `item_hash` is stored as the entry's content hash. Typically computed as
    /// `fnv1a_hash(format!("{}:{}:{}:{}", key, timestamp.millis, timestamp.counter, timestamp.node_id))`.
    pub fn update(&mut self, key: &str, item_hash: u32) {
        let path = key_to_path(key);
        Self::update_node(&mut self.root, key, item_hash, &path, 0, self.depth);
    }

    /// Removes a key from the tree, recalculating hashes up the trie.
    pub fn remove(&mut self, key: &str) {
        let path = key_to_path(key);
        Self::remove_node(&mut self.root, key, &path, 0, self.depth);
    }

    /// Returns the root hash for quick comparison with a remote tree.
    ///
    /// If the root hashes match, the trees contain identical data (with high probability).
    pub fn get_root_hash(&self) -> u32 {
        self.root.hash
    }

    /// Returns child hashes at the given path for bucket comparison.
    ///
    /// Pass an empty string to get root-level children. Each entry maps a hex digit
    /// to the aggregated hash of that subtree.
    pub fn get_buckets(&self, path: &str) -> HashMap<char, u32> {
        match self.get_node(path) {
            Some(node) => node.children.iter().map(|(&k, v)| (k, v.hash)).collect(),
            None => HashMap::new(),
        }
    }

    /// Returns the keys stored in a leaf bucket at the given path.
    ///
    /// Used to identify specific keys that need synchronization after bucket
    /// comparison identifies a differing subtree.
    pub fn get_keys_in_bucket(&self, path: &str) -> Vec<String> {
        match self.get_node(path) {
            Some(node) => node.entries.keys().cloned().collect(),
            None => Vec::new(),
        }
    }

    /// Returns the node at the given path, or `None` if the path does not exist.
    ///
    /// The path is a string of hex digits navigating down the trie.
    pub fn get_node(&self, path: &str) -> Option<&MerkleNode> {
        let mut current = &self.root;
        for ch in path.chars() {
            match current.children.get(&ch) {
                Some(child) => current = child,
                None => return None,
            }
        }
        Some(current)
    }

    /// Recursively updates a node in the trie.
    fn update_node(
        node: &mut MerkleNode,
        key: &str,
        item_hash: u32,
        path: &str,
        level: usize,
        depth: usize,
    ) {
        if level >= depth {
            // Leaf node: store entry and recalculate hash
            node.entries.insert(key.to_string(), item_hash);
            node.hash = Self::recalc_leaf_hash(&node.entries);
            return;
        }

        // Internal node: route to child
        let bucket_char = path.as_bytes()[level] as char;
        let child = node
            .children
            .entry(bucket_char)
            .or_insert_with(MerkleNode::new);

        Self::update_node(child, key, item_hash, path, level + 1, depth);

        // Recalculate this node's hash from children
        node.hash = Self::recalc_internal_hash(&node.children);
    }

    /// Recursively removes a key from the trie.
    fn remove_node(
        node: &mut MerkleNode,
        key: &str,
        path: &str,
        level: usize,
        depth: usize,
    ) {
        if level >= depth {
            // Leaf node: remove entry and recalculate hash
            node.entries.remove(key);
            node.hash = Self::recalc_leaf_hash(&node.entries);
            return;
        }

        // Internal node: route to child
        let bucket_char = path.as_bytes()[level] as char;
        if let Some(child) = node.children.get_mut(&bucket_char) {
            Self::remove_node(child, key, path, level + 1, depth);
        }

        // Recalculate this node's hash from children
        node.hash = Self::recalc_internal_hash(&node.children);
    }

    /// Recalculates a leaf node's hash from its entries.
    fn recalc_leaf_hash(entries: &HashMap<String, u32>) -> u32 {
        let mut h: u32 = 0;
        for &val in entries.values() {
            h = h.wrapping_add(val);
        }
        h
    }

    /// Recalculates an internal node's hash from its children.
    fn recalc_internal_hash(children: &HashMap<char, MerkleNode>) -> u32 {
        let mut h: u32 = 0;
        for child in children.values() {
            h = h.wrapping_add(child.hash);
        }
        h
    }
}

/// A MerkleTree for efficient delta synchronization of OR-Maps.
///
/// Similar to [`MerkleTree`] but designed for ORMap where each key can have
/// multiple records (tags). The entry hash represents the combined hash of all
/// records for a key. Methods like [`ORMapMerkleTree::find_diff_keys`] enable
/// efficient identification of keys that differ between local and remote trees.
pub struct ORMapMerkleTree {
    root: MerkleNode,
    depth: usize,
}

impl ORMapMerkleTree {
    /// Creates a new empty `ORMapMerkleTree` with the given depth.
    pub fn new(depth: usize) -> Self {
        Self {
            root: MerkleNode::new(),
            depth,
        }
    }

    /// Creates a new `ORMapMerkleTree` with the default depth of 3.
    pub fn default_depth() -> Self {
        Self::new(3)
    }

    /// Updates a key's entry hash in the tree.
    ///
    /// The `entry_hash` should be computed from all records for the key
    /// (e.g., using `hashORMapEntry` logic: sorted tags, deterministic string representation).
    pub fn update(&mut self, key: &str, entry_hash: u32) {
        let path = key_to_path(key);
        Self::update_node(&mut self.root, key, entry_hash, &path, 0, self.depth);
    }

    /// Removes a key from the tree.
    pub fn remove(&mut self, key: &str) {
        let path = key_to_path(key);
        Self::remove_node(&mut self.root, key, &path, 0, self.depth);
    }

    /// Returns the root hash for quick comparison with a remote tree.
    pub fn get_root_hash(&self) -> u32 {
        self.root.hash
    }

    /// Returns child hashes at the given path for bucket comparison.
    pub fn get_buckets(&self, path: &str) -> HashMap<char, u32> {
        match self.get_node(path) {
            Some(node) => node.children.iter().map(|(&k, v)| (k, v.hash)).collect(),
            None => HashMap::new(),
        }
    }

    /// Returns the keys stored in a leaf bucket at the given path.
    pub fn get_keys_in_bucket(&self, path: &str) -> Vec<String> {
        match self.get_node(path) {
            Some(node) => node.entries.keys().cloned().collect(),
            None => Vec::new(),
        }
    }

    /// Returns the node at the given path, or `None` if the path does not exist.
    pub fn get_node(&self, path: &str) -> Option<&MerkleNode> {
        let mut current = &self.root;
        for ch in path.chars() {
            match current.children.get(&ch) {
                Some(child) => current = child,
                None => return None,
            }
        }
        Some(current)
    }

    /// Finds keys that differ between this tree and remote entry hashes at a path.
    ///
    /// Returns keys that:
    /// - Exist locally but have a different hash on remote
    /// - Exist on remote but not locally
    /// - Exist locally but not on remote
    pub fn find_diff_keys(&self, path: &str, remote_entries: &HashMap<String, u32>) -> Vec<String> {
        let local_entries = match self.get_node(path) {
            Some(node) => &node.entries,
            None => {
                // No local node: all remote keys are diffs
                return remote_entries.keys().cloned().collect();
            }
        };

        let mut diff_keys = Vec::new();

        // Keys in local but not remote, or different hash
        for (key, &local_hash) in local_entries {
            match remote_entries.get(key) {
                Some(&remote_hash) if remote_hash == local_hash => {}
                _ => diff_keys.push(key.clone()),
            }
        }

        // Keys in remote but not local
        for key in remote_entries.keys() {
            if !local_entries.contains_key(key) {
                diff_keys.push(key.clone());
            }
        }

        diff_keys
    }

    /// Returns all entry hashes at a leaf path.
    ///
    /// Used when sending bucket details to a remote node for comparison.
    pub fn get_entry_hashes(&self, path: &str) -> HashMap<String, u32> {
        match self.get_node(path) {
            Some(node) => node.entries.clone(),
            None => HashMap::new(),
        }
    }

    /// Checks if a path leads to a leaf node (a node with entries).
    pub fn is_leaf(&self, path: &str) -> bool {
        match self.get_node(path) {
            Some(node) => !node.entries.is_empty(),
            None => false,
        }
    }

    /// Recursively updates a node in the trie.
    fn update_node(
        node: &mut MerkleNode,
        key: &str,
        entry_hash: u32,
        path: &str,
        level: usize,
        depth: usize,
    ) {
        if level >= depth {
            node.entries.insert(key.to_string(), entry_hash);
            node.hash = Self::recalc_leaf_hash(&node.entries);
            return;
        }

        let bucket_char = path.as_bytes()[level] as char;
        let child = node
            .children
            .entry(bucket_char)
            .or_insert_with(MerkleNode::new);

        Self::update_node(child, key, entry_hash, path, level + 1, depth);
        node.hash = Self::recalc_internal_hash(&node.children);
    }

    /// Recursively removes a key from the trie.
    fn remove_node(
        node: &mut MerkleNode,
        key: &str,
        path: &str,
        level: usize,
        depth: usize,
    ) {
        if level >= depth {
            node.entries.remove(key);
            node.hash = Self::recalc_leaf_hash(&node.entries);
            return;
        }

        let bucket_char = path.as_bytes()[level] as char;
        if let Some(child) = node.children.get_mut(&bucket_char) {
            Self::remove_node(child, key, path, level + 1, depth);
        }

        node.hash = Self::recalc_internal_hash(&node.children);
    }

    /// Recalculates a leaf node's hash from its entries.
    fn recalc_leaf_hash(entries: &HashMap<String, u32>) -> u32 {
        let mut h: u32 = 0;
        for &val in entries.values() {
            h = h.wrapping_add(val);
        }
        h
    }

    /// Recalculates an internal node's hash from its children.
    fn recalc_internal_hash(children: &HashMap<char, MerkleNode>) -> u32 {
        let mut h: u32 = 0;
        for child in children.values() {
            h = h.wrapping_add(child.hash);
        }
        h
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::fnv1a_hash;

    // ---- Helper: create an item hash like the TS MerkleTree.update does ----
    fn make_item_hash(key: &str, millis: u64, counter: u32, node_id: &str) -> u32 {
        fnv1a_hash(&format!("{key}:{millis}:{counter}:{node_id}"))
    }

    // ---- MerkleTree tests ----

    #[test]
    fn empty_tree_has_zero_root_hash() {
        let tree = MerkleTree::default_depth();
        assert_eq!(tree.get_root_hash(), 0);
    }

    #[test]
    fn same_data_same_root_hash_regardless_of_order() {
        let h_a = make_item_hash("a", 100, 0, "test");
        let h_b = make_item_hash("b", 200, 0, "test");

        // Order 1: a then b
        let mut tree1 = MerkleTree::default_depth();
        tree1.update("a", h_a);
        tree1.update("b", h_b);

        // Order 2: b then a
        let mut tree2 = MerkleTree::default_depth();
        tree2.update("b", h_b);
        tree2.update("a", h_a);

        assert_eq!(tree1.get_root_hash(), tree2.get_root_hash());
        assert_ne!(tree1.get_root_hash(), 0);
    }

    #[test]
    fn different_data_different_root_hash() {
        let mut tree1 = MerkleTree::default_depth();
        tree1.update("a", make_item_hash("a", 100, 0, "test"));

        let mut tree2 = MerkleTree::default_depth();
        tree2.update("a", make_item_hash("a", 101, 0, "test"));

        assert_ne!(tree1.get_root_hash(), tree2.get_root_hash());
    }

    #[test]
    fn update_changes_root_hash() {
        let mut tree = MerkleTree::default_depth();
        assert_eq!(tree.get_root_hash(), 0);

        tree.update("key1", make_item_hash("key1", 100, 0, "test"));
        let h1 = tree.get_root_hash();
        assert_ne!(h1, 0);

        tree.update("key2", make_item_hash("key2", 200, 0, "test"));
        let h2 = tree.get_root_hash();
        assert_ne!(h2, h1);
    }

    #[test]
    fn remove_restores_hash() {
        let mut tree = MerkleTree::default_depth();
        let h_a = make_item_hash("a", 100, 0, "test");

        tree.update("a", h_a);
        tree.update("b", make_item_hash("b", 200, 0, "test"));
        let hash_with_ab = tree.get_root_hash();

        tree.remove("b");
        let hash_after_remove = tree.get_root_hash();

        // Should not be the same as with both keys
        assert_ne!(hash_after_remove, hash_with_ab);

        // Should be the same as a tree with only "a"
        let mut tree_a_only = MerkleTree::default_depth();
        tree_a_only.update("a", h_a);
        assert_eq!(hash_after_remove, tree_a_only.get_root_hash());
    }

    #[test]
    fn remove_all_keys_returns_to_zero() {
        let mut tree = MerkleTree::default_depth();
        tree.update("a", make_item_hash("a", 100, 0, "test"));
        tree.update("b", make_item_hash("b", 200, 0, "test"));

        tree.remove("a");
        tree.remove("b");

        assert_eq!(tree.get_root_hash(), 0);
    }

    #[test]
    fn get_buckets_at_root() {
        let mut tree = MerkleTree::default_depth();
        tree.update("key1", make_item_hash("key1", 100, 0, "test"));
        tree.update("key2", make_item_hash("key2", 100, 0, "test"));

        let buckets = tree.get_buckets("");
        assert!(!buckets.is_empty());
    }

    #[test]
    fn get_keys_in_bucket() {
        let mut tree = MerkleTree::new(1); // Depth 1: only 1 level before leaf
        tree.update("a", make_item_hash("a", 100, 0, "test"));

        // Get the first hex char of the hash of "a"
        let path = key_to_path("a");
        let bucket = &path[..1];

        let keys = tree.get_keys_in_bucket(bucket);
        assert!(keys.contains(&"a".to_string()));
    }

    #[test]
    fn get_node_nonexistent_path() {
        let tree = MerkleTree::default_depth();
        assert!(tree.get_node("zzz").is_none());
    }

    #[test]
    fn get_node_root() {
        let mut tree = MerkleTree::default_depth();
        tree.update("a", 42);
        let node = tree.get_node("");
        assert!(node.is_some());
        assert_eq!(node.unwrap().hash, tree.get_root_hash());
    }

    // ---- ORMapMerkleTree tests ----

    #[test]
    fn ormap_empty_tree_has_zero_root() {
        let tree = ORMapMerkleTree::default_depth();
        assert_eq!(tree.get_root_hash(), 0);
    }

    #[test]
    fn ormap_same_data_same_hash_regardless_of_order() {
        let h_a = fnv1a_hash("key:a|tag1:v1:100:0:test");
        let h_b = fnv1a_hash("key:b|tag2:v2:200:0:test");

        let mut tree1 = ORMapMerkleTree::default_depth();
        tree1.update("a", h_a);
        tree1.update("b", h_b);

        let mut tree2 = ORMapMerkleTree::default_depth();
        tree2.update("b", h_b);
        tree2.update("a", h_a);

        assert_eq!(tree1.get_root_hash(), tree2.get_root_hash());
        assert_ne!(tree1.get_root_hash(), 0);
    }

    #[test]
    fn ormap_update_and_remove() {
        let mut tree = ORMapMerkleTree::default_depth();
        let h = fnv1a_hash("key:a|tag1:v:100:0:test");

        tree.update("a", h);
        assert_ne!(tree.get_root_hash(), 0);

        tree.remove("a");
        assert_eq!(tree.get_root_hash(), 0);
    }

    #[test]
    fn ormap_find_diff_keys_all_match() {
        let mut tree = ORMapMerkleTree::new(1);
        tree.update("a", 100);

        let path = key_to_path("a");
        let bucket = &path[..1];

        let mut remote = HashMap::new();
        remote.insert("a".to_string(), 100_u32);

        let diffs = tree.find_diff_keys(bucket, &remote);
        assert!(diffs.is_empty());
    }

    #[test]
    fn ormap_find_diff_keys_hash_mismatch() {
        let mut tree = ORMapMerkleTree::new(1);
        tree.update("a", 100);

        let path = key_to_path("a");
        let bucket = &path[..1];

        let mut remote = HashMap::new();
        remote.insert("a".to_string(), 200_u32); // Different hash

        let diffs = tree.find_diff_keys(bucket, &remote);
        assert!(diffs.contains(&"a".to_string()));
    }

    #[test]
    fn ormap_find_diff_keys_local_only() {
        let mut tree = ORMapMerkleTree::new(1);
        tree.update("a", 100);

        let path = key_to_path("a");
        let bucket = &path[..1];

        let remote = HashMap::new(); // Remote has nothing

        let diffs = tree.find_diff_keys(bucket, &remote);
        assert!(diffs.contains(&"a".to_string()));
    }

    #[test]
    fn ormap_find_diff_keys_remote_only() {
        let tree = ORMapMerkleTree::new(1);

        // Use the bucket path for key "a"
        let path = key_to_path("a");
        let bucket = &path[..1];

        let mut remote = HashMap::new();
        remote.insert("a".to_string(), 100_u32);

        let diffs = tree.find_diff_keys(bucket, &remote);
        assert!(diffs.contains(&"a".to_string()));
    }

    #[test]
    fn ormap_get_entry_hashes() {
        let mut tree = ORMapMerkleTree::new(1);
        tree.update("a", 100);

        let path = key_to_path("a");
        let bucket = &path[..1];

        let hashes = tree.get_entry_hashes(bucket);
        assert_eq!(hashes.get("a"), Some(&100));
    }

    #[test]
    fn ormap_is_leaf() {
        let mut tree = ORMapMerkleTree::new(1);
        tree.update("a", 100);

        let path = key_to_path("a");
        let bucket = &path[..1];

        assert!(tree.is_leaf(bucket));
        assert!(!tree.is_leaf("z")); // Non-existent path
    }

    #[test]
    fn ormap_get_buckets() {
        let mut tree = ORMapMerkleTree::default_depth();
        tree.update("key1", 100);
        tree.update("key2", 200);

        let buckets = tree.get_buckets("");
        assert!(!buckets.is_empty());
    }

    #[test]
    fn ormap_get_keys_in_bucket() {
        let mut tree = ORMapMerkleTree::new(1);
        tree.update("a", 100);

        let path = key_to_path("a");
        let bucket = &path[..1];

        let keys = tree.get_keys_in_bucket(bucket);
        assert!(keys.contains(&"a".to_string()));
    }
}
