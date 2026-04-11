//! Per-map index registry holding all secondary indexes for a given map.
//!
//! [`IndexRegistry`] stores one `Arc<dyn Index>` per attribute name. Callers
//! add indexes explicitly via `add_hash_index`, `add_navigable_index`,
//! `add_inverted_index`, or `add_vector_index`. Adding a second index for the
//! same attribute replaces the existing one — only one index strategy per
//! attribute is supported.
//!
//! [`IndexRegistry::get_best_index`] implements the operation-to-index-type
//! mapping so that query evaluation can pick the right index for a predicate
//! leaf without knowing which index types are registered.
//!
//! Vector indexes bypass the standard predicate path: `get_best_index` always
//! returns `None` for vector-typed attributes because vector search is not
//! driven by `PredicateOp` comparisons. Callers obtain a concrete
//! `Arc<VectorIndex>` via `get_vector_index` to run ANN queries.

use std::sync::Arc;

use dashmap::DashMap;
use topgun_core::messages::base::{PredicateNode, PredicateOp};
use topgun_core::vector::DistanceMetric;

use super::{HashIndex, Index, IndexType, InvertedIndex, NavigableIndex, VectorIndex};

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
///
/// A side-channel `vector_indexes` map preserves the concrete `Arc<VectorIndex>`
/// type alongside the `Arc<dyn Index>` trait-object view, avoiding the need to
/// downcast from a trait object to access vector-specific methods.
pub struct IndexRegistry {
    /// Map from attribute name → registered index (trait-object view).
    indexes: DashMap<String, Arc<dyn Index>>,
    /// Side-channel map preserving the concrete type for vector indexes.
    /// Both maps share the same underlying allocation via `Arc::clone`.
    vector_indexes: DashMap<String, Arc<VectorIndex>>,
}

impl IndexRegistry {
    /// Creates a new, empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            indexes: DashMap::new(),
            vector_indexes: DashMap::new(),
        }
    }

    /// Registers a [`HashIndex`] for `attribute`, replacing any existing index.
    pub fn add_hash_index(&self, attribute: impl Into<String>) {
        let attr = attribute.into();
        self.vector_indexes.remove(&attr);
        self.indexes
            .insert(attr.clone(), Arc::new(HashIndex::new(attr)));
    }

    /// Registers a [`NavigableIndex`] for `attribute`, replacing any existing index.
    pub fn add_navigable_index(&self, attribute: impl Into<String>) {
        let attr = attribute.into();
        self.vector_indexes.remove(&attr);
        self.indexes
            .insert(attr.clone(), Arc::new(NavigableIndex::new(attr)));
    }

    /// Registers an [`InvertedIndex`] for `attribute`, replacing any existing index.
    pub fn add_inverted_index(&self, attribute: impl Into<String>) {
        let attr = attribute.into();
        self.vector_indexes.remove(&attr);
        self.indexes
            .insert(attr.clone(), Arc::new(InvertedIndex::new(attr)));
    }

    /// Registers a [`VectorIndex`] for `attribute`, replacing any existing index.
    ///
    /// Inserts into both the `indexes` trait-object map and the `vector_indexes`
    /// side-channel so that `get_vector_index` can return the concrete type
    /// without downcasting.
    pub fn add_vector_index(
        &self,
        attribute: impl Into<String>,
        dimension: u16,
        distance_metric: DistanceMetric,
    ) {
        let attr = attribute.into();
        let vi = Arc::new(VectorIndex::new(attr.clone(), dimension, distance_metric));
        // Store the concrete type in the side-channel before moving into the
        // trait-object map so both views share the same allocation.
        self.vector_indexes.insert(attr.clone(), Arc::clone(&vi));
        self.indexes.insert(attr, vi as Arc<dyn Index>);
    }

    /// Returns the index registered for `attribute`, or `None` if none exists.
    #[must_use]
    pub fn get_index(&self, attribute: &str) -> Option<Arc<dyn Index>> {
        self.indexes.get(attribute).map(|r| Arc::clone(r.value()))
    }

    /// Returns the concrete `VectorIndex` for `attribute`, or `None` if the
    /// attribute has no vector index (or has a different index type).
    ///
    /// Query service callers use this to obtain an `Arc<VectorIndex>` with
    /// access to `search_nearest` and `commit_pending` without downcasting.
    #[must_use]
    pub fn get_vector_index(&self, attribute: &str) -> Option<Arc<VectorIndex>> {
        self.vector_indexes
            .get(attribute)
            .map(|r| Arc::clone(r.value()))
    }

    /// Returns the best index for accelerating a leaf predicate, or `None`.
    ///
    /// The mapping from predicate operation to required index type is:
    /// - `Eq` / `Neq` → requires a `Hash` index (O(1) equality lookup)
    /// - `Gt` / `Gte` / `Lt` / `Lte` → requires a `Navigable` index (range scan)
    /// - `Like` → requires an `Inverted` index (token-based partial match)
    /// - `Regex` → always `None` (regex cannot be accelerated)
    /// - `And` / `Or` / `Not` → always `None` (compound predicates handled by query optimizer)
    /// - Vector indexes → always `None` (vector search is not predicate-driven)
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
            | PredicateOp::IsNotNull
            | PredicateOp::ContainsAll
            | PredicateOp::ContainsAny
            | PredicateOp::StartsWith
            | PredicateOp::EndsWith => {
                return None;
            }
            PredicateOp::Eq | PredicateOp::Neq => IndexType::Hash,
            PredicateOp::Gt | PredicateOp::Gte | PredicateOp::Lt | PredicateOp::Lte => {
                IndexType::Navigable
            }
            PredicateOp::Like => IndexType::Inverted,
        };

        // `required_type` is one of Hash, Navigable, Inverted — never Vector —
        // so this check naturally returns None for vector-indexed attributes
        // without requiring an explicit short-circuit branch.
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

    /// Removes the index for `attribute`, clearing all its data.
    ///
    /// For vector indexes, removes from both the trait-object map and the
    /// concrete `vector_indexes` side-channel.
    ///
    /// Returns `true` if an index was removed, `false` if none existed.
    #[must_use]
    pub fn remove_index(&self, attribute: &str) -> bool {
        // Always attempt removal from both maps; removal from an empty map
        // is a no-op, so this is safe for non-vector indexes.
        self.vector_indexes.remove(attribute);
        if let Some((_, index)) = self.indexes.remove(attribute) {
            index.clear();
            true
        } else {
            false
        }
    }

    /// Returns `true` if an index is registered for `attribute`.
    #[must_use]
    pub fn has_index(&self, attribute: &str) -> bool {
        self.indexes.contains_key(attribute)
    }

    /// Returns the number of registered vector indexes.
    #[must_use]
    pub fn vector_index_count(&self) -> usize {
        self.vector_indexes.len()
    }

    /// Returns the attribute name of the first registered vector index,
    /// or `None` if no vector indexes are registered.
    ///
    /// Used by `handle_vector_search` when `index_name` is not specified and the
    /// map has exactly one vector index. If the map has zero or multiple vector
    /// indexes, the caller handles those cases before invoking this method.
    #[must_use]
    pub fn first_vector_index_attribute(&self) -> Option<String> {
        self.vector_indexes
            .iter()
            .next()
            .map(|r| r.key().clone())
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
    use topgun_core::messages::base::PredicateOp;
    use topgun_core::vector::DistanceMetric;

    use super::*;

    fn make_leaf(op: PredicateOp, attribute: &str) -> PredicateNode {
        PredicateNode {
            op,
            attribute: Some(attribute.to_string()),
            ..Default::default()
        }
    }

    fn make_compound(op: PredicateOp) -> PredicateNode {
        PredicateNode {
            op,
            children: Some(vec![]),
            ..Default::default()
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
    fn add_and_get_vector_index() {
        let registry = IndexRegistry::new();
        registry.add_vector_index("embedding", 768, DistanceMetric::Cosine);
        let idx = registry
            .get_index("embedding")
            .expect("index should be present");
        assert_eq!(idx.index_type(), IndexType::Vector);
        let vi = registry
            .get_vector_index("embedding")
            .expect("vector index should be accessible");
        assert_eq!(vi.dimension(), 768);
        assert_eq!(vi.distance_metric(), DistanceMetric::Cosine);
    }

    #[test]
    fn get_vector_index_returns_none_for_hash_attribute() {
        let registry = IndexRegistry::new();
        registry.add_hash_index("name");
        assert!(
            registry.get_vector_index("name").is_none(),
            "hash index should not be returned by get_vector_index"
        );
    }

    #[test]
    fn get_best_index_vector_returns_none_for_all_predicate_ops() {
        let registry = IndexRegistry::new();
        registry.add_vector_index("embedding", 4, DistanceMetric::Cosine);

        // Iterate over every PredicateOp variant to ensure future additions are
        // also covered. The strum crate is not available; enumerate manually.
        let all_ops = [
            PredicateOp::Eq,
            PredicateOp::Neq,
            PredicateOp::Gt,
            PredicateOp::Gte,
            PredicateOp::Lt,
            PredicateOp::Lte,
            PredicateOp::Like,
            PredicateOp::Regex,
            PredicateOp::In,
            PredicateOp::Between,
            PredicateOp::IsNull,
            PredicateOp::IsNotNull,
            PredicateOp::ContainsAll,
            PredicateOp::ContainsAny,
            PredicateOp::StartsWith,
            PredicateOp::EndsWith,
            PredicateOp::And,
            PredicateOp::Or,
            PredicateOp::Not,
        ];
        for op in all_ops {
            let pred = make_leaf(op.clone(), "embedding");
            assert!(
                registry.get_best_index(&pred).is_none(),
                "get_best_index should return None for vector index with op {op:?}"
            );
        }
    }

    #[test]
    fn remove_vector_index_clears_both_views() {
        let registry = IndexRegistry::new();
        registry.add_vector_index("embedding", 4, DistanceMetric::Cosine);
        assert!(registry.get_vector_index("embedding").is_some());
        assert!(registry.get_index("embedding").is_some());

        let removed = registry.remove_index("embedding");
        assert!(removed, "should return true");
        assert!(
            registry.get_vector_index("embedding").is_none(),
            "concrete view should be cleared"
        );
        assert!(
            registry.get_index("embedding").is_none(),
            "trait-object view should be cleared"
        );
    }

    #[test]
    fn stats_includes_vector_index() {
        let registry = IndexRegistry::new();
        registry.add_hash_index("name");
        registry.add_vector_index("embedding", 4, DistanceMetric::Cosine);

        let mut stats = registry.stats();
        stats.sort_by(|a, b| a.attribute.cmp(&b.attribute));

        let vector_stat = stats.iter().find(|s| s.attribute == "embedding");
        assert!(vector_stat.is_some());
        assert_eq!(vector_stat.unwrap().index_type, IndexType::Vector);
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

    #[test]
    fn remove_index_returns_true_when_present() {
        let registry = IndexRegistry::new();
        registry.add_hash_index("email");
        assert!(registry.remove_index("email"), "should return true when index existed");
    }

    #[test]
    fn remove_index_returns_false_when_absent() {
        let registry = IndexRegistry::new();
        assert!(
            !registry.remove_index("nonexistent"),
            "should return false when no index existed"
        );
    }

    #[test]
    fn remove_index_clears_data() {
        let registry = IndexRegistry::new();
        registry.add_hash_index("name");

        // Insert some data into the index.
        let idx = registry.get_index("name").unwrap();
        idx.insert("k1", &rmpv::Value::String("alice".into()));
        assert_eq!(idx.entry_count(), 1);

        // Remove the index — data must be cleared.
        let _ = registry.remove_index("name");

        // The index should no longer exist in the registry.
        assert!(registry.get_index("name").is_none(), "index should be gone after remove");
        // The evicted Arc still holds cleared data.
        assert_eq!(idx.entry_count(), 0, "index data should be cleared");
    }

    #[test]
    fn has_index_reflects_state() {
        let registry = IndexRegistry::new();
        assert!(!registry.has_index("status"), "should be false before adding");

        registry.add_hash_index("status");
        assert!(registry.has_index("status"), "should be true after adding");

        let _ = registry.remove_index("status");
        assert!(!registry.has_index("status"), "should be false after removing");
    }
}
