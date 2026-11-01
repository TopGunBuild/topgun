//! Navigable (range-capable) secondary index backed by a sorted `BTreeMap`.
//!
//! Provides O(log N) range queries and O(log N) equality lookups. All
//! concurrent access is mediated by a `parking_lot::RwLock`.

use std::collections::{BTreeMap, HashMap, HashSet};
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
    inner: RwLock<Inner>,
}

/// The buckets and the reverse map live under one lock, so every mutation
/// reads the reverse entry, changes the buckets and writes the reverse entry
/// in one write-guard critical section (`TG-IDX-001`).
#[derive(Default)]
struct Inner {
    /// Map from indexed value → set of record keys.
    buckets: BTreeMap<ComparableValue, HashSet<String>>,
    /// Reverse map: record key → the attribute value it is indexed under.
    entries: HashMap<String, rmpv::Value>,
}

impl Inner {
    fn index_value(&mut self, key: &str, attr_val: &rmpv::Value) {
        match attr_val {
            rmpv::Value::Array(elements) => {
                for elem in elements {
                    self.index_single(key, elem);
                }
            }
            other => self.index_single(key, other),
        }
    }

    fn index_single(&mut self, key: &str, val: &rmpv::Value) {
        let cv = ComparableValue::from_value(val);
        self.buckets.entry(cv).or_default().insert(key.to_owned());
    }

    fn deindex_value(&mut self, key: &str, attr_val: &rmpv::Value) {
        match attr_val {
            rmpv::Value::Array(elements) => {
                for elem in elements {
                    self.deindex_single(key, elem);
                }
            }
            other => self.deindex_single(key, other),
        }
    }

    fn deindex_single(&mut self, key: &str, val: &rmpv::Value) {
        let cv = ComparableValue::from_value(val);
        if let Some(set) = self.buckets.get_mut(&cv) {
            set.remove(key);
            if set.is_empty() {
                self.buckets.remove(&cv);
            }
        }
    }

    fn remove_key(&mut self, key: &str) {
        if let Some(old_attr) = self.entries.remove(key) {
            self.deindex_value(key, &old_attr);
        }
    }
}

impl NavigableIndex {
    #[must_use]
    pub fn new(attribute_name: String) -> Self {
        NavigableIndex {
            extractor: AttributeExtractor::new(attribute_name.clone()),
            attribute_name,
            inner: RwLock::new(Inner::default()),
        }
    }

    /// Indexes `key` under the attribute value of `value`, replacing its entry,
    /// or does nothing when `only_if_absent` and the key has an entry.
    fn put_entry(&self, key: &str, value: &rmpv::Value, only_if_absent: bool) {
        let attr_val = self.extractor.extract(value);
        let mut inner = self.inner.write();
        if only_if_absent && inner.entries.contains_key(key) {
            return;
        }
        if let Some(old_attr) = inner.entries.insert(key.to_owned(), attr_val.clone()) {
            inner.deindex_value(key, &old_attr);
        }
        inner.index_value(key, &attr_val);
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
        self.put_entry(key, value, false);
    }

    fn insert_if_absent(&self, key: &str, value: &rmpv::Value) {
        self.put_entry(key, value, true);
    }

    fn update(&self, key: &str, _old_value: &rmpv::Value, new_value: &rmpv::Value) {
        self.put_entry(key, new_value, false);
    }

    fn remove(&self, key: &str, _old_value: &rmpv::Value) {
        self.remove_key(key);
    }

    fn remove_key(&self, key: &str) {
        self.inner.write().remove_key(key);
    }

    fn clear(&self) {
        let mut inner = self.inner.write();
        inner.buckets.clear();
        inner.entries.clear();
    }

    fn lookup_eq(&self, value: &rmpv::Value) -> HashSet<String> {
        let cv = ComparableValue::from_value(value);
        self.inner
            .read()
            .buckets
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

        let inner = self.inner.read();
        inner
            .buckets
            .range((lower_bound, upper_bound))
            .flat_map(|(_, set)| set.iter().cloned())
            .collect()
    }

    /// `NavigableIndex` does not support token search; returns an empty set.
    fn lookup_contains(&self, _token: &str) -> HashSet<String> {
        HashSet::new()
    }

    fn entry_count(&self) -> u64 {
        self.inner
            .read()
            .buckets
            .values()
            .map(|s| s.len() as u64)
            .sum()
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

    // --- TG-IDX-001: one entry per key, through the reverse entry ---

    fn holds(idx: &NavigableIndex, v: i64, key: &str) -> bool {
        idx.lookup_eq(&rmpv::Value::Integer(v.into())).contains(key)
    }

    #[test]
    fn tg_idx_001_insert_replaces_the_key_entry() {
        let idx = NavigableIndex::new("price".to_string());
        idx.insert("k", &price_record(1));
        idx.insert("k", &price_record(2));
        assert!(!holds(&idx, 1, "k"), "the replaced value must not hold k");
        assert!(holds(&idx, 2, "k"));
        assert_eq!(idx.lookup_range(None, true, None, true).len(), 1);
        assert_eq!(idx.entry_count(), 1);
    }

    #[test]
    fn tg_idx_001_remove_key_removes_without_old_value() {
        let idx = NavigableIndex::new("price".to_string());
        idx.insert("k", &price_record(1));
        idx.insert("other", &price_record(1));
        idx.remove_key("k");
        assert!(!holds(&idx, 1, "k"));
        assert!(holds(&idx, 1, "other"));
        assert_eq!(idx.entry_count(), 1);
        idx.remove_key("k");
        assert_eq!(
            idx.entry_count(),
            1,
            "removing an absent key changes nothing"
        );
    }

    #[test]
    fn tg_idx_001_insert_if_absent_keeps_the_existing_entry() {
        let idx = NavigableIndex::new("price".to_string());
        idx.insert("k", &price_record(2));
        idx.insert_if_absent("k", &price_record(1));
        assert!(holds(&idx, 2, "k"));
        assert!(!holds(&idx, 1, "k"));
        assert_eq!(idx.entry_count(), 1);
        idx.insert_if_absent("fresh", &price_record(1));
        assert!(holds(&idx, 1, "fresh"), "an absent key is inserted");
    }

    #[test]
    fn tg_idx_001_update_and_remove_ignore_a_stale_old_value() {
        let idx = NavigableIndex::new("price".to_string());
        idx.insert("k", &price_record(1));
        idx.update("k", &price_record(0), &price_record(2));
        assert!(
            !holds(&idx, 1, "k"),
            "the current value is replaced, not the passed one"
        );
        assert!(holds(&idx, 2, "k"));
        assert_eq!(idx.entry_count(), 1);
        idx.remove("k", &price_record(0));
        assert!(!holds(&idx, 2, "k"));
        assert_eq!(idx.entry_count(), 0);
    }

    #[test]
    fn tg_idx_001_clear_clears_the_reverse_entries() {
        let idx = NavigableIndex::new("price".to_string());
        idx.insert("k", &price_record(1));
        idx.clear();
        idx.insert_if_absent("k", &price_record(2));
        assert!(
            holds(&idx, 2, "k"),
            "after clear the key has no entry to keep"
        );
        assert_eq!(idx.entry_count(), 1);
        idx.remove_key("k");
        assert_eq!(idx.entry_count(), 0);
    }

    #[test]
    fn tg_idx_001_concurrent_replacements_leave_one_entry() {
        use std::sync::Arc;
        use std::thread;

        const THREADS: i64 = 8;
        const ROUNDS: i64 = 10_000;
        let idx = Arc::new(NavigableIndex::new("price".to_string()));
        let handles: Vec<_> = (0..THREADS)
            .map(|t| {
                let idx = Arc::clone(&idx);
                thread::spawn(move || {
                    for r in 0..ROUNDS {
                        idx.insert("k", &price_record(t * ROUNDS + r));
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().expect("thread panicked");
        }
        assert_eq!(idx.entry_count(), 1);
        let named = idx.inner.read().entries["k"]
            .as_i64()
            .expect("integer attribute");
        assert!(
            holds(&idx, named, "k"),
            "k sits in the bucket its reverse entry names"
        );
    }
}
