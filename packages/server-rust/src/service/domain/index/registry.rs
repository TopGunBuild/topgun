//! Per-map index registry holding all secondary indexes for a given map.
//!
//! [`IndexRegistry`] stores one `Arc<dyn Index>` per attribute name. Callers
//! add indexes explicitly via `add_hash_index`, `add_navigable_index`, or
//! `add_inverted_index`. Adding a second index for the same attribute replaces
//! the existing one — only one index strategy per attribute is supported.
//!
//! [`IndexRegistry::get_best_index`] implements the operation-to-index-type
//! mapping so that query evaluation can pick the right index for a predicate
//! leaf without knowing which index types are registered.

use std::sync::Arc;

use dashmap::DashMap;
use topgun_core::messages::base::{PredicateNode, PredicateOp};

use super::{HashIndex, Index, IndexType, InvertedIndex, NavigableIndex};

// ---------------------------------------------------------------------------
// IndexStats
// ---------------------------------------------------------------------------

/// Snapshot statistics for a single registered index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexStats {
    /// The attribute name this index covers.
    pub attribute: String,
    /// The strategy used by this index.
    pub index_type: IndexType,
    /// Number of distinct attribute values currently tracked.
    pub entry_count: u64,
}

// ---------------------------------------------------------------------------
// IndexRegistry
// ---------------------------------------------------------------------------

/// Thread-safe per-map registry of secondary indexes.
///
/// All operations are lock-free at the registry level (backed by `DashMap`).
/// Individual index implementations are also internally concurrent.
pub struct IndexRegistry {
    /// Map from attribute name → registered index.
    indexes: DashMap<String, Arc<dyn Index>>,
}

impl IndexRegistry {
    /// Creates a new, empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            indexes: DashMap::new(),
        }
    }

    /// Registers a [`HashIndex`] for `attribute`, replacing any existing index.
    pub fn add_hash_index(&self, attribute: impl Into<String>) {
        let attr = attribute.into();
        self.indexes
            .insert(attr.clone(), Arc::new(HashIndex::new(attr)));
    }

    /// Registers a [`NavigableIndex`] for `attribute`, replacing any existing index.
    pub fn add_navigable_index(&self, attribute: impl Into<String>) {
        let attr = attribute.into();
        self.indexes
            .insert(attr.clone(), Arc::new(NavigableIndex::new(attr)));
    }

    /// Registers an [`InvertedIndex`] for `attribute`, replacing any existing index.
    pub fn add_inverted_index(&self, attribute: impl Into<String>) {
        let attr = attribute.into();
        self.indexes
            .insert(attr.clone(), Arc::new(InvertedIndex::new(attr)));
    }

    /// Returns the index registered for `attribute`, or `None` if none exists.
    #[must_use]
    pub fn get_index(&self, attribute: &str) -> Option<Arc<dyn Index>> {
        self.indexes.get(attribute).map(|r| Arc::clone(r.value()))
    }

    /// Returns the best index for accelerating a leaf predicate, or `None`.
    ///
    /// The mapping from predicate operation to required index type is:
    /// - `Eq` / `Neq` → requires a `Hash` index (O(1) equality lookup)
    /// - `Gt` / `Gte` / `Lt` / `Lte` → requires a `Navigable` index (range scan)
    /// - `Like` → requires an `Inverted` index (token-based partial match)
    /// - `Regex` → always `None` (regex cannot be accelerated)
    /// - `And` / `Or` / `Not` → always `None` (compound predicates handled by query optimizer)
    ///
    /// Returns `None` if the required index type is not registered for the
    /// attribute, even if an index of a different type is present — returning a
    /// mismatched type would yield incorrect results.
    #[must_use]
    pub fn get_best_index(&self, predicate: &PredicateNode) -> Option<Arc<dyn Index>> {
        let required_type = match predicate.op {
            // Compound and unindexable operators: fall back to full scan
            PredicateOp::And
            | PredicateOp::Or
            | PredicateOp::Not
            | PredicateOp::Regex
            | PredicateOp::In
            | PredicateOp::Between
            | PredicateOp::IsNull
            | PredicateOp::IsNotNull => {
                return None;
            }
            PredicateOp::Eq | PredicateOp::Neq => IndexType::Hash,
            PredicateOp::Gt | PredicateOp::Gte | PredicateOp::Lt | PredicateOp::Lte => {
                IndexType::Navigable
            }
            PredicateOp::Like => IndexType::Inverted,
        };

        let attribute = predicate.attribute.as_deref()?;
        let index = self.get_index(attribute)?;
        if index.index_type() == required_type {
            Some(index)
        } else {
            None
        }
    }

    /// Returns all registered indexes (for iteration by the mutation observer).
    #[must_use]
    pub fn indexes(&self) -> Vec<Arc<dyn Index>> {
        self.indexes
            .iter()
            .map(|r| Arc::clone(r.value()))
            .collect()
    }

    /// Returns statistics for all registered indexes.
    #[must_use]
    pub fn stats(&self) -> Vec<IndexStats> {
        self.indexes
            .iter()
            .map(|r| IndexStats {
                attribute: r.key().clone(),
                index_type: r.value().index_type(),
                entry_count: r.value().entry_count(),
            })
            .collect()
    }
}

impl Default for IndexRegistry {
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
    use topgun_core::messages::base::PredicateOp;

    fn make_leaf(op: PredicateOp, attribute: &str) -> PredicateNode {
        PredicateNode {
            op,
            attribute: Some(attribute.to_string()),
            value: None,
            children: None,
        }
    }

    fn make_compound(op: PredicateOp) -> PredicateNode {
        PredicateNode {
            op,
            attribute: None,
            value: None,
            children: Some(vec![]),
        }
    }

    #[test]
    fn add_and_get_hash_index() {
        let registry = IndexRegistry::new();
        registry.add_hash_index("name");
        let idx = registry.get_index("name").expect("index should be present");
        assert_eq!(idx.index_type(), IndexType::Hash);
        assert_eq!(idx.attribute_name(), "name");
    }

    #[test]
    fn add_and_get_navigable_index() {
        let registry = IndexRegistry::new();
        registry.add_navigable_index("age");
        let idx = registry.get_index("age").expect("index should be present");
        assert_eq!(idx.index_type(), IndexType::Navigable);
    }

    #[test]
    fn add_and_get_inverted_index() {
        let registry = IndexRegistry::new();
        registry.add_inverted_index("description");
        let idx = registry
            .get_index("description")
            .expect("index should be present");
        assert_eq!(idx.index_type(), IndexType::Inverted);
    }

    #[test]
    fn get_index_missing_returns_none() {
        let registry = IndexRegistry::new();
        assert!(registry.get_index("nonexistent").is_none());
    }

    #[test]
    fn add_index_replaces_existing() {
        let registry = IndexRegistry::new();
        registry.add_hash_index("field");
        registry.add_navigable_index("field");
        let idx = registry.get_index("field").expect("index should be present");
        assert_eq!(idx.index_type(), IndexType::Navigable, "should be replaced");
    }

    #[test]
    fn stats_accuracy() {
        let registry = IndexRegistry::new();
        registry.add_hash_index("name");
        registry.add_navigable_index("age");

        let mut stats = registry.stats();
        stats.sort_by(|a, b| a.attribute.cmp(&b.attribute));

        assert_eq!(stats.len(), 2);
        assert_eq!(stats[0].attribute, "age");
        assert_eq!(stats[0].index_type, IndexType::Navigable);
        assert_eq!(stats[0].entry_count, 0);
        assert_eq!(stats[1].attribute, "name");
        assert_eq!(stats[1].index_type, IndexType::Hash);
        assert_eq!(stats[1].entry_count, 0);
    }

    #[test]
    fn stats_entry_count_updates_after_insert() {
        let registry = IndexRegistry::new();
        registry.add_hash_index("name");
        let idx = registry.get_index("name").unwrap();
        idx.insert("k1", &rmpv::Value::String("alice".into()));
        idx.insert("k2", &rmpv::Value::String("bob".into()));

        let stats = registry.stats();
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].entry_count, 2);
    }

    #[test]
    fn get_best_index_eq_returns_hash() {
        let registry = IndexRegistry::new();
        registry.add_hash_index("status");
        let pred = make_leaf(PredicateOp::Eq, "status");
        let idx = registry.get_best_index(&pred).expect("should find hash index");
        assert_eq!(idx.index_type(), IndexType::Hash);
    }

    #[test]
    fn get_best_index_neq_returns_hash() {
        let registry = IndexRegistry::new();
        registry.add_hash_index("status");
        let pred = make_leaf(PredicateOp::Neq, "status");
        let idx = registry.get_best_index(&pred).expect("should find hash index");
        assert_eq!(idx.index_type(), IndexType::Hash);
    }

    #[test]
    fn get_best_index_gt_returns_navigable() {
        let registry = IndexRegistry::new();
        registry.add_navigable_index("age");
        let pred = make_leaf(PredicateOp::Gt, "age");
        let idx = registry
            .get_best_index(&pred)
            .expect("should find navigable index");
        assert_eq!(idx.index_type(), IndexType::Navigable);
    }

    #[test]
    fn get_best_index_like_returns_inverted() {
        let registry = IndexRegistry::new();
        registry.add_inverted_index("bio");
        let pred = make_leaf(PredicateOp::Like, "bio");
        let idx = registry
            .get_best_index(&pred)
            .expect("should find inverted index");
        assert_eq!(idx.index_type(), IndexType::Inverted);
    }

    #[test]
    fn get_best_index_regex_always_none() {
        let registry = IndexRegistry::new();
        registry.add_hash_index("field");
        registry.add_navigable_index("field");
        let pred = make_leaf(PredicateOp::Regex, "field");
        assert!(
            registry.get_best_index(&pred).is_none(),
            "Regex cannot be accelerated"
        );
    }

    #[test]
    fn get_best_index_compound_always_none() {
        let registry = IndexRegistry::new();
        registry.add_hash_index("field");
        for op in [PredicateOp::And, PredicateOp::Or, PredicateOp::Not] {
            let pred = make_compound(op);
            assert!(
                registry.get_best_index(&pred).is_none(),
                "compound predicates should return None"
            );
        }
    }

    #[test]
    fn get_best_index_type_mismatch_returns_none() {
        let registry = IndexRegistry::new();
        // Register only a hash index on "age", but predicate is Gt (needs Navigable)
        registry.add_hash_index("age");
        let pred = make_leaf(PredicateOp::Gt, "age");
        assert!(
            registry.get_best_index(&pred).is_none(),
            "hash index should not be returned for range predicate"
        );
    }

    #[test]
    fn get_best_index_no_index_for_attribute_returns_none() {
        let registry = IndexRegistry::new();
        let pred = make_leaf(PredicateOp::Eq, "missing");
        assert!(registry.get_best_index(&pred).is_none());
    }

    #[test]
    fn indexes_returns_all_registered() {
        let registry = IndexRegistry::new();
        registry.add_hash_index("a");
        registry.add_navigable_index("b");
        registry.add_inverted_index("c");
        assert_eq!(registry.indexes().len(), 3);
    }
}
