//! Hash-based secondary index for O(1) equality lookups.
//!
//! Backed by `DashMap<IndexableValue, DashSet<String>>` to provide
//! concurrent reads and writes without a global lock.

use std::collections::HashSet;

use dashmap::DashMap;
use dashmap::DashSet;

use super::{AttributeExtractor, Index, IndexType, IndexableValue};

/// An in-memory secondary index supporting O(1) equality lookups.
///
/// Each entry maps an `IndexableValue` (the indexed attribute value) to the
/// set of record keys that carry that value. Multi-value attributes (i.e.
/// attributes stored as `rmpv::Value::Array`) are expanded: each array
/// element gets its own entry.
pub struct HashIndex {
    attribute_name: String,
    extractor: AttributeExtractor,
    /// Map from indexed value → set of record keys.
    map: DashMap<IndexableValue, DashSet<String>>,
}

impl HashIndex {
    #[must_use]
    pub fn new(attribute_name: String) -> Self {
        HashIndex {
            extractor: AttributeExtractor::new(attribute_name.clone()),
            attribute_name,
            map: DashMap::new(),
        }
    }

    /// Inserts a record key under the given `rmpv::Value` attribute value.
    ///
    /// If `attr_val` is an Array, each element is indexed separately.
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
        let iv = IndexableValue::from_value(val);
        self.map
            .entry(iv)
            .or_default()
            .insert(key.to_owned());
    }

    /// Removes a record key from the index entry for the given attribute value.
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
        let iv = IndexableValue::from_value(val);
        if let Some(set) = self.map.get(&iv) {
            set.remove(key);
        }
        // Remove the bucket entirely if it is now empty to avoid unbounded
        // memory growth when many distinct values are inserted and removed.
        self.map.remove_if(&iv, |_, set| set.is_empty());
    }
}

impl Index for HashIndex {
    fn index_type(&self) -> IndexType {
        IndexType::Hash
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
        self.map.clear();
    }

    fn lookup_eq(&self, value: &rmpv::Value) -> HashSet<String> {
        let iv = IndexableValue::from_value(value);
        self.map
            .get(&iv)
            .map(|set| set.iter().map(|s| String::clone(&s)).collect())
            .unwrap_or_default()
    }

    /// `HashIndex` does not support range queries; returns an empty set.
    fn lookup_range(
        &self,
        _lower: Option<&rmpv::Value>,
        _lower_inclusive: bool,
        _upper: Option<&rmpv::Value>,
        _upper_inclusive: bool,
    ) -> HashSet<String> {
        HashSet::new()
    }

    /// `HashIndex` does not support token search; returns an empty set.
    fn lookup_contains(&self, _token: &str) -> HashSet<String> {
        HashSet::new()
    }

    fn entry_count(&self) -> u64 {
        // Count unique (value, key) pairs across all buckets.
        self.map.iter().map(|entry| entry.value().len() as u64).sum()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn record(category: &str) -> rmpv::Value {
        rmpv::Value::Map(vec![(
            rmpv::Value::String(rmpv::Utf8String::from("category")),
            rmpv::Value::String(rmpv::Utf8String::from(category)),
        )])
    }

    fn record_with_tags(tags: &[&str]) -> rmpv::Value {
        rmpv::Value::Map(vec![(
            rmpv::Value::String(rmpv::Utf8String::from("tags")),
            rmpv::Value::Array(
                tags.iter()
                    .map(|t| rmpv::Value::String(rmpv::Utf8String::from(*t)))
                    .collect(),
            ),
        )])
    }

    #[test]
    fn insert_and_lookup_eq() {
        let idx = HashIndex::new("category".to_string());
        idx.insert("k1", &record("electronics"));
        idx.insert("k2", &record("books"));
        idx.insert("k3", &record("electronics"));

        let result = idx.lookup_eq(&rmpv::Value::String(rmpv::Utf8String::from("electronics")));
        assert_eq!(result.len(), 2);
        assert!(result.contains("k1"));
        assert!(result.contains("k3"));

        let books = idx.lookup_eq(&rmpv::Value::String(rmpv::Utf8String::from("books")));
        assert_eq!(books.len(), 1);
        assert!(books.contains("k2"));
    }

    #[test]
    fn update_removes_old_and_adds_new() {
        let idx = HashIndex::new("category".to_string());
        idx.insert("k1", &record("electronics"));
        idx.update("k1", &record("electronics"), &record("books"));

        let elec = idx.lookup_eq(&rmpv::Value::String(rmpv::Utf8String::from("electronics")));
        assert!(elec.is_empty());

        let books = idx.lookup_eq(&rmpv::Value::String(rmpv::Utf8String::from("books")));
        assert!(books.contains("k1"));
    }

    #[test]
    fn remove_cleans_up_entry() {
        let idx = HashIndex::new("category".to_string());
        idx.insert("k1", &record("electronics"));
        idx.remove("k1", &record("electronics"));

        let result = idx.lookup_eq(&rmpv::Value::String(rmpv::Utf8String::from("electronics")));
        assert!(result.is_empty());
    }

    #[test]
    fn multi_value_array_attribute() {
        let idx = HashIndex::new("tags".to_string());
        idx.insert("k1", &record_with_tags(&["rust", "indexing"]));

        let rust = idx.lookup_eq(&rmpv::Value::String(rmpv::Utf8String::from("rust")));
        assert!(rust.contains("k1"));

        let indexing = idx.lookup_eq(&rmpv::Value::String(rmpv::Utf8String::from("indexing")));
        assert!(indexing.contains("k1"));
    }

    #[test]
    fn clear_empties_index() {
        let idx = HashIndex::new("category".to_string());
        idx.insert("k1", &record("electronics"));
        idx.insert("k2", &record("books"));
        idx.clear();

        assert_eq!(idx.entry_count(), 0);
        let result = idx.lookup_eq(&rmpv::Value::String(rmpv::Utf8String::from("electronics")));
        assert!(result.is_empty());
    }

    #[test]
    fn entry_count_tracks_inserts_and_removes() {
        let idx = HashIndex::new("category".to_string());
        assert_eq!(idx.entry_count(), 0);
        idx.insert("k1", &record("electronics"));
        idx.insert("k2", &record("books"));
        idx.insert("k3", &record("electronics"));
        assert_eq!(idx.entry_count(), 3);
        idx.remove("k3", &record("electronics"));
        assert_eq!(idx.entry_count(), 2);
    }

    #[test]
    fn concurrent_access_does_not_panic() {
        use std::sync::Arc;
        use std::thread;

        let idx = Arc::new(HashIndex::new("category".to_string()));
        let mut handles = vec![];
        for i in 0..4 {
            let idx_clone = Arc::clone(&idx);
            handles.push(thread::spawn(move || {
                for j in 0..100 {
                    let key = format!("k{}-{}", i, j);
                    let cat = if j % 2 == 0 { "electronics" } else { "books" };
                    idx_clone.insert(&key, &record(cat));
                    let _ = idx_clone.lookup_eq(&rmpv::Value::String(
                        rmpv::Utf8String::from("electronics"),
                    ));
                }
            }));
        }
        for h in handles {
            h.join().expect("thread panicked");
        }
    }
}
