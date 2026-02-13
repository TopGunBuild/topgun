//! MerkleTree and `ORMapMerkleTree` for efficient delta synchronization.
//!
//! Both trees use a prefix trie structure keyed by hex digits of the FNV-1a hash
//! of entry keys. The trie depth (default 3) determines bucket granularity.
//! Nodes compare root hashes to identify differing subtrees, then walk down
//! to discover the specific keys that need synchronization.
