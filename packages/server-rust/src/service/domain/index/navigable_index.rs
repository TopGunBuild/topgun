//! Navigable (range-capable) secondary index backed by a sorted BTreeMap.
//!
//! Provides O(log N) range queries and O(log N) equality lookups. All
//! concurrent access is mediated by a `parking_lot::RwLock`.

use std::collections::{BTreeMap, HashSet};
use std::ops::Bound;

use parking_lot::RwLock;

use super::{AttributeExtractor, ComparableValue, Index, IndexType};

/// An in-memory secondary index supporting O(log N) range queries.
///
/// Backed by a `BTreeMap` wrapped in a `parking_lot::RwLock`. Writes
/// acquire an exclusive lock; reads acquire a shared lock.
pub struct NavigableIndex {
    attribute_name: String,
    extractor: AttributeExtractor,
    map: RwLock<BTreeMap<ComparableValue, HashSet<String>>>,
}

impl NavigableIndex {
    pub fn new(attribute_name: String) -> Self {
        NavigableIndex {
            extractor: AttributeExtractor::new(attribute_name.clone()),
            attribute_name,
            map: RwLock::new(BTreeMap::new()),
        }
    }

    fn index_value(&self, key: &str, attr_val: &rmpv::Value) {
        match attr_val {
            rmpv::Value::Array(elements) => {
                for elem in elements {
                    self.index_single(key, elem);
                }
            }
            other => self.index_single(key, other),
        }
    }

    fn index_single(&self, key: &str, val: &rmpv::Value) {
        let cv = ComparableValue::from_value(val);
        let mut guard = self.map.write();
        guard.entry(cv).or_insert_with(HashSet::new).insert(key.to_owned());
    }

    fn deindex_value(&self, key: &str, attr_val: &rmpv::Value) {
        match attr_val {
            rmpv::Value::Array(elements) => {
                for elem in elements {
                    self.deindex_single(key, elem);
                }
            }
            other => self.deindex_single(key, other),
        }
    }

    fn deindex_single(&self, key: &str, val: &rmpv::Value) {
        let cv = ComparableValue::from_value(val);
        let mut guard = self.map.write();
        if let Some(set) = guard.get_mut(&cv) {
            set.remove(key);
        }
        if guard.get(&cv).map(|s| s.is_empty()).unwrap_or(false) {
            guard.remove(&cv);
        }
    }
}

impl Index for NavigableIndex {
    fn index_type(&self) -> IndexType {
        IndexType::Navigable
    }

    fn attribute_name(&self) -> &str {
        &self.attribute_name
    }

    fn insert(&self, key: &str, value: &rmpv::Value) {
        let attr_val = self.extractor.extract(value);
        self.index_value(key, &attr_val);
    }

    fn update(&self, key: &str, old_value: &rmpv::Value, new_value: &rmpv::Value) {
        let old_attr = self.extractor.extract(old_value);
        let new_attr = self.extractor.extract(new_value);
        self.deindex_value(key, &old_attr);
        self.index_value(key, &new_attr);
    }

    fn remove(&self, key: &str, old_value: &rmpv::Value) {
        let attr_val = self.extractor.extract(old_value);
        self.deindex_value(key, &attr_val);
    }

    fn clear(&self) {
        self.map.write().clear();
    }

    fn lookup_eq(&self, value: &rmpv::Value) -> HashSet<String> {
        let cv = ComparableValue::from_value(value);
        self.map
            .read()
            .get(&cv)
            .cloned()
            .unwrap_or_default()
    }

    fn lookup_range(
        &self,
        lower: Option<&rmpv::Value>,
        lower_inclusive: bool,
        upper: Option<&rmpv::Value>,
        upper_inclusive: bool,
    ) -> HashSet<String> {
        let lower_bound = match lower {
            None => Bound::Unbounded,
            Some(v) => {
                let cv = ComparableValue::from_value(v);
                if lower_inclusive {
                    Bound::Included(cv)
                } else {
                    Bound::Excluded(cv)
                }
            }
        };
        let upper_bound = match upper {
            None => Bound::Unbounded,
            Some(v) => {
                let cv = ComparableValue::from_value(v);
                if upper_inclusive {
                    Bound::Included(cv)
                } else {
                    Bound::Excluded(cv)
                }
            }
        };

        let guard = self.map.read();
        guard
            .range((lower_bound, upper_bound))
            .flat_map(|(_, set)| set.iter().cloned())
            .collect()
    }

    /// NavigableIndex does not support token search; returns an empty set.
    fn lookup_contains(&self, _token: &str) -> HashSet<String> {
        HashSet::new()
    }

    fn entry_count(&self) -> u64 {
        self.map.read().values().map(|s| s.len() as u64).sum()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn price_record(price: i64) -> rmpv::Value {
        rmpv::Value::Map(vec![(
            rmpv::Value::String(rmpv::Utf8String::from("price")),
            rmpv::Value::Integer(price.into()),
        )])
    }

    #[test]
    fn insert_and_range_lookup() {
        let idx = NavigableIndex::new("price".to_string());
        idx.insert("k1", &price_record(10));
        idx.insert("k2", &price_record(50));
        idx.insert("k3", &price_record(100));
        idx.insert("k4", &price_record(200));

        let lower = rmpv::Value::Integer(20i64.into());
        let upper = rmpv::Value::Integer(100i64.into());
        let result = idx.lookup_range(Some(&lower), true, Some(&upper), true);

        assert_eq!(result.len(), 2);
        assert!(result.contains("k2"));
        assert!(result.contains("k3"));
    }

    #[test]
    fn exclusive_boundary_conditions() {
        let idx = NavigableIndex::new("price".to_string());
        idx.insert("k1", &price_record(20));
        idx.insert("k2", &price_record(50));
        idx.insert("k3", &price_record(100));

        // Exclusive lower, inclusive upper: (20, 100]
        let lower = rmpv::Value::Integer(20i64.into());
        let upper = rmpv::Value::Integer(100i64.into());
        let result = idx.lookup_range(Some(&lower), false, Some(&upper), true);

        assert!(!result.contains("k1"), "20 should be excluded");
        assert!(result.contains("k2"));
        assert!(result.contains("k3"));
    }

    #[test]
    fn eq_lookup_via_range() {
        let idx = NavigableIndex::new("price".to_string());
        idx.insert("k1", &price_record(50));
        idx.insert("k2", &price_record(50));
        idx.insert("k3", &price_record(75));

        let v = rmpv::Value::Integer(50i64.into());
        let result = idx.lookup_eq(&v);
        assert_eq!(result.len(), 2);
        assert!(result.contains("k1"));
        assert!(result.contains("k2"));
    }

    #[test]
    fn update_moves_key_to_new_value() {
        let idx = NavigableIndex::new("price".to_string());
        idx.insert("k1", &price_record(10));
        idx.update("k1", &price_record(10), &price_record(200));

        let low = rmpv::Value::Integer(10i64.into());
        assert!(idx.lookup_eq(&low).is_empty());

        let high = rmpv::Value::Integer(200i64.into());
        assert!(idx.lookup_eq(&high).contains("k1"));
    }

    #[test]
    fn remove_cleans_up() {
        let idx = NavigableIndex::new("price".to_string());
        idx.insert("k1", &price_record(50));
        idx.remove("k1", &price_record(50));

        let v = rmpv::Value::Integer(50i64.into());
        assert!(idx.lookup_eq(&v).is_empty());
    }

    #[test]
    fn clear_empties_index() {
        let idx = NavigableIndex::new("price".to_string());
        idx.insert("k1", &price_record(10));
        idx.insert("k2", &price_record(20));
        idx.clear();

        assert_eq!(idx.entry_count(), 0);
        let lower = rmpv::Value::Integer(0i64.into());
        let upper = rmpv::Value::Integer(1000i64.into());
        let result = idx.lookup_range(Some(&lower), true, Some(&upper), true);
        assert!(result.is_empty());
    }

    #[test]
    fn unbounded_range_returns_all() {
        let idx = NavigableIndex::new("price".to_string());
        idx.insert("k1", &price_record(10));
        idx.insert("k2", &price_record(999));

        let result = idx.lookup_range(None, true, None, true);
        assert_eq!(result.len(), 2);
    }
}
