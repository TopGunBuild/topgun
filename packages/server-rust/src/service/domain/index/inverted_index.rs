//! Inverted index for keyword token search.
//!
//! Maps tokens → record keys. Tokens are produced by a default tokenizer
//! that lowercases the string and splits on whitespace and punctuation.
//! This is a lightweight complement to the tantivy SearchService; it is NOT
//! a replacement.

use std::collections::HashSet;

use dashmap::DashMap;
use dashmap::DashSet;

use super::{AttributeExtractor, Index, IndexType};

/// An in-memory inverted index supporting O(K) token lookups where K is the
/// number of records containing the queried token.
///
/// Backed by `DashMap<String, DashSet<String>>` for concurrent access.
pub struct InvertedIndex {
    attribute_name: String,
    extractor: AttributeExtractor,
    /// Map from token → set of record keys.
    map: DashMap<String, DashSet<String>>,
}

impl InvertedIndex {
    pub fn new(attribute_name: String) -> Self {
        InvertedIndex {
            extractor: AttributeExtractor::new(attribute_name.clone()),
            attribute_name,
            map: DashMap::new(),
        }
    }

    /// Returns the set of record keys whose indexed text contains ALL of the
    /// given tokens (intersection).
    ///
    /// This is an inherent method, not part of the `Index` trait.
    pub fn lookup_contains_all(&self, tokens: &[&str]) -> HashSet<String> {
        if tokens.is_empty() {
            return HashSet::new();
        }
        let mut result: Option<HashSet<String>> = None;
        for token in tokens {
            let norm = token.to_lowercase();
            let keys: HashSet<String> = self
                .map
                .get(&norm)
                .map(|s| s.iter().map(|k| k.clone()).collect())
                .unwrap_or_default();
            result = Some(match result {
                None => keys,
                Some(acc) => acc.intersection(&keys).cloned().collect(),
            });
        }
        result.unwrap_or_default()
    }

    /// Returns the set of record keys whose indexed text contains ANY of the
    /// given tokens (union).
    ///
    /// This is an inherent method, not part of the `Index` trait.
    pub fn lookup_contains_any(&self, tokens: &[&str]) -> HashSet<String> {
        let mut result = HashSet::new();
        for token in tokens {
            let norm = token.to_lowercase();
            if let Some(set) = self.map.get(&norm) {
                for key in set.iter() {
                    result.insert(key.clone());
                }
            }
        }
        result
    }

    /// Tokenizes a string value: lowercase, split on whitespace and ASCII
    /// punctuation.
    fn tokenize(text: &str) -> Vec<String> {
        text.to_lowercase()
            .split(|c: char| c.is_whitespace() || c.is_ascii_punctuation())
            .filter(|t| !t.is_empty())
            .map(|t| t.to_owned())
            .collect()
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
        // Only string values are tokenized; other types are ignored.
        if let Some(text) = val.as_str() {
            for token in Self::tokenize(text) {
                self.map
                    .entry(token)
                    .or_insert_with(DashSet::new)
                    .insert(key.to_owned());
            }
        }
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
        if let Some(text) = val.as_str() {
            for token in Self::tokenize(text) {
                if let Some(set) = self.map.get(&token) {
                    set.remove(key);
                }
                self.map.remove_if(&token, |_, set| set.is_empty());
            }
        }
    }
}

impl Index for InvertedIndex {
    fn index_type(&self) -> IndexType {
        IndexType::Inverted
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

    /// Returns keys whose indexed text contains the given token (case-insensitive).
    fn lookup_contains(&self, token: &str) -> HashSet<String> {
        let norm = token.to_lowercase();
        self.map
            .get(&norm)
            .map(|set| set.iter().map(|k| k.clone()).collect())
            .unwrap_or_default()
    }

    /// InvertedIndex does not support equality lookups on raw values; returns
    /// an empty set. Use `lookup_contains` instead.
    fn lookup_eq(&self, _value: &rmpv::Value) -> HashSet<String> {
        HashSet::new()
    }

    /// InvertedIndex does not support range queries; returns an empty set.
    fn lookup_range(
        &self,
        _lower: Option<&rmpv::Value>,
        _lower_inclusive: bool,
        _upper: Option<&rmpv::Value>,
        _upper_inclusive: bool,
    ) -> HashSet<String> {
        HashSet::new()
    }

    fn entry_count(&self) -> u64 {
        // Count total (token, key) pairs.
        self.map.iter().map(|entry| entry.value().len() as u64).sum()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn text_record(text: &str) -> rmpv::Value {
        rmpv::Value::Map(vec![(
            rmpv::Value::String(rmpv::Utf8String::from("description")),
            rmpv::Value::String(rmpv::Utf8String::from(text)),
        )])
    }

    #[test]
    fn insert_and_lookup_contains() {
        let idx = InvertedIndex::new("description".to_string());
        idx.insert("k1", &text_record("laptop computer"));
        idx.insert("k2", &text_record("desktop computer"));
        idx.insert("k3", &text_record("laptop stand"));

        let result = idx.lookup_contains("laptop");
        assert_eq!(result.len(), 2);
        assert!(result.contains("k1"));
        assert!(result.contains("k3"));
    }

    #[test]
    fn lookup_is_case_insensitive() {
        let idx = InvertedIndex::new("description".to_string());
        idx.insert("k1", &text_record("Apple Laptop"));

        // Query with different cases.
        assert!(!idx.lookup_contains("laptop").is_empty());
        assert!(!idx.lookup_contains("LAPTOP").is_empty());
        assert!(!idx.lookup_contains("Laptop").is_empty());
    }

    #[test]
    fn lookup_contains_all_intersection() {
        let idx = InvertedIndex::new("description".to_string());
        idx.insert("k1", &text_record("laptop computer bag"));
        idx.insert("k2", &text_record("laptop stand"));
        idx.insert("k3", &text_record("computer desk"));

        let result = idx.lookup_contains_all(&["laptop", "computer"]);
        assert_eq!(result.len(), 1);
        assert!(result.contains("k1"));
    }

    #[test]
    fn lookup_contains_any_union() {
        let idx = InvertedIndex::new("description".to_string());
        idx.insert("k1", &text_record("laptop stand"));
        idx.insert("k2", &text_record("desk lamp"));
        idx.insert("k3", &text_record("mouse pad"));

        let result = idx.lookup_contains_any(&["laptop", "desk"]);
        assert_eq!(result.len(), 2);
        assert!(result.contains("k1"));
        assert!(result.contains("k2"));
    }

    #[test]
    fn remove_cleans_up_tokens() {
        let idx = InvertedIndex::new("description".to_string());
        idx.insert("k1", &text_record("laptop computer"));
        idx.remove("k1", &text_record("laptop computer"));

        assert!(idx.lookup_contains("laptop").is_empty());
        assert!(idx.lookup_contains("computer").is_empty());
    }

    #[test]
    fn clear_empties_index() {
        let idx = InvertedIndex::new("description".to_string());
        idx.insert("k1", &text_record("laptop computer"));
        idx.insert("k2", &text_record("desktop"));
        idx.clear();

        assert_eq!(idx.entry_count(), 0);
        assert!(idx.lookup_contains("laptop").is_empty());
    }

    #[test]
    fn punctuation_is_used_as_delimiter() {
        let idx = InvertedIndex::new("description".to_string());
        // Comma and period should be treated as delimiters.
        idx.insert("k1", &text_record("laptop, computer. stand"));

        assert!(idx.lookup_contains("laptop").contains("k1"));
        assert!(idx.lookup_contains("computer").contains("k1"));
        assert!(idx.lookup_contains("stand").contains("k1"));
    }

    #[test]
    fn update_reindexes() {
        let idx = InvertedIndex::new("description".to_string());
        idx.insert("k1", &text_record("laptop computer"));
        idx.update(
            "k1",
            &text_record("laptop computer"),
            &text_record("desktop monitor"),
        );

        assert!(idx.lookup_contains("laptop").is_empty());
        assert!(idx.lookup_contains("computer").is_empty());
        assert!(idx.lookup_contains("desktop").contains("k1"));
        assert!(idx.lookup_contains("monitor").contains("k1"));
    }
}
