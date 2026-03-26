//! Mutation observer that keeps secondary indexes in sync with record mutations.
//!
//! [`IndexMutationObserver`] implements [`MutationObserver`] and routes each
//! mutation event to all indexes registered in the associated [`IndexRegistry`].
//!
//! [`IndexObserverFactory`] implements [`ObserverFactory`] and creates
//! [`IndexMutationObserver`] instances for maps that have registered indexes.
//! Maps without a registry entry return `None` from `create_observer`.

use std::sync::Arc;

use dashmap::DashMap;

use crate::service::domain::predicate::value_to_rmpv;
use crate::storage::factory::ObserverFactory;
use crate::storage::mutation_observer::MutationObserver;
use crate::storage::record::{Record, RecordValue};

use super::registry::IndexRegistry;

// ---------------------------------------------------------------------------
// IndexMutationObserver
// ---------------------------------------------------------------------------

/// `MutationObserver` that routes mutations to all indexes in an [`IndexRegistry`].
///
/// Each method extracts the attribute value from the record's `RecordValue` and
/// dispatches to the appropriate index operation. Only `RecordValue::Lww` records
/// are indexed; `OrMap` and `OrTombstones` are skipped because OR-Map records are
/// multi-entry and not suitable for single-attribute indexes.
pub struct IndexMutationObserver {
    registry: Arc<IndexRegistry>,
}

impl IndexMutationObserver {
    /// Creates a new observer backed by the given registry.
    #[must_use]
    pub fn new(registry: Arc<IndexRegistry>) -> Self {
        Self { registry }
    }

    /// Converts a `RecordValue` to `rmpv::Value` for index operations.
    ///
    /// Returns `None` for `OrMap` and `OrTombstones` variants — those are
    /// skipped because they are not suitable for single-attribute indexes.
    fn extract_rmpv(value: &RecordValue) -> Option<rmpv::Value> {
        match value {
            RecordValue::Lww { value, .. } => Some(value_to_rmpv(value)),
            RecordValue::OrMap { .. } | RecordValue::OrTombstones { .. } => None,
        }
    }
}

impl MutationObserver for IndexMutationObserver {
    fn on_put(&self, key: &str, record: &Record, _old_value: Option<&RecordValue>, _is_backup: bool) {
        let Some(rmpv_val) = Self::extract_rmpv(&record.value) else {
            return;
        };
        // Each index internally uses its own AttributeExtractor to pull its
        // attribute from the full record value, so we pass the whole map.
        for index in self.registry.indexes() {
            index.insert(key, &rmpv_val);
        }
    }

    fn on_update(
        &self,
        key: &str,
        _record: &Record,
        old_value: &RecordValue,
        new_value: &RecordValue,
        _is_backup: bool,
    ) {
        let Some(old_rmpv) = Self::extract_rmpv(old_value) else {
            return;
        };
        let Some(new_rmpv) = Self::extract_rmpv(new_value) else {
            return;
        };
        for index in self.registry.indexes() {
            index.update(key, &old_rmpv, &new_rmpv);
        }
    }

    fn on_remove(&self, key: &str, record: &Record, _is_backup: bool) {
        let Some(rmpv_val) = Self::extract_rmpv(&record.value) else {
            return;
        };
        for index in self.registry.indexes() {
            index.remove(key, &rmpv_val);
        }
    }

    fn on_evict(&self, key: &str, record: &Record, _is_backup: bool) {
        let Some(rmpv_val) = Self::extract_rmpv(&record.value) else {
            return;
        };
        for index in self.registry.indexes() {
            index.remove(key, &rmpv_val);
        }
    }

    fn on_load(&self, key: &str, record: &Record, _is_backup: bool) {
        let Some(rmpv_val) = Self::extract_rmpv(&record.value) else {
            return;
        };
        for index in self.registry.indexes() {
            index.insert(key, &rmpv_val);
        }
    }

    fn on_replication_put(&self, key: &str, record: &Record, populate_index: bool) {
        if !populate_index {
            return;
        }
        let Some(rmpv_val) = Self::extract_rmpv(&record.value) else {
            return;
        };
        for index in self.registry.indexes() {
            index.insert(key, &rmpv_val);
        }
    }

    fn on_clear(&self) {
        for index in self.registry.indexes() {
            index.clear();
        }
    }

    fn on_reset(&self) {
        for index in self.registry.indexes() {
            index.clear();
        }
    }

    fn on_destroy(&self, _is_shutdown: bool) {
        for index in self.registry.indexes() {
            index.clear();
        }
    }
}

// ---------------------------------------------------------------------------
// IndexObserverFactory
// ---------------------------------------------------------------------------

/// Factory that creates [`IndexMutationObserver`] instances for maps that
/// have registered indexes.
///
/// Maps that have not been pre-registered via `register_map` return `None`
/// from `create_observer`, meaning they do not participate in index maintenance.
pub struct IndexObserverFactory {
    /// Map from map name to its [`IndexRegistry`].
    registries: DashMap<String, Arc<IndexRegistry>>,
}

impl IndexObserverFactory {
    /// Creates a new factory with no registered maps.
    #[must_use]
    pub fn new() -> Self {
        Self {
            registries: DashMap::new(),
        }
    }

    /// Registers a map and returns its [`IndexRegistry`].
    ///
    /// If the map is already registered, returns the existing registry without
    /// replacing it. This is idempotent: repeated calls with the same name are safe.
    pub fn register_map(&self, map_name: impl Into<String>) -> Arc<IndexRegistry> {
        let name = map_name.into();
        self.registries
            .entry(name)
            .or_insert_with(|| Arc::new(IndexRegistry::new()))
            .clone()
    }
}

impl Default for IndexObserverFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl ObserverFactory for IndexObserverFactory {
    fn create_observer(
        &self,
        map_name: &str,
        _partition_id: u32,
    ) -> Option<Arc<dyn MutationObserver>> {
        self.registries
            .get(map_name)
            .map(|registry| -> Arc<dyn MutationObserver> {
                Arc::new(IndexMutationObserver::new(Arc::clone(registry.value())))
            })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use topgun_core::hlc::Timestamp;
    use topgun_core::types::Value;

    use super::*;
    use crate::storage::record::RecordMetadata;

    // Helpers

    fn ts() -> Timestamp {
        Timestamp {
            millis: 1_000_000,
            counter: 0,
            node_id: "node-1".to_string(),
        }
    }

    fn make_lww_record(map_pairs: Vec<(&str, rmpv::Value)>) -> Record {
        // Build a Value::Map from rmpv pairs so that AttributeExtractor can traverse it.
        // topgun_core Value::Map is BTreeMap<String, Value>.
        let mut inner_map = std::collections::BTreeMap::new();
        for (k, v) in map_pairs {
            inner_map.insert(k.to_string(), rmpv_to_core_value(v));
        }
        Record {
            value: RecordValue::Lww {
                value: Value::Map(inner_map),
                timestamp: ts(),
            },
            metadata: RecordMetadata::new(1_000_000, 64),
        }
    }

    fn make_ormap_record() -> Record {
        Record {
            value: RecordValue::OrMap { records: vec![] },
            metadata: RecordMetadata::new(1_000_000, 64),
        }
    }

    fn make_ortombstones_record() -> Record {
        Record {
            value: RecordValue::OrTombstones { tags: vec![] },
            metadata: RecordMetadata::new(1_000_000, 64),
        }
    }

    fn rmpv_to_core_value(v: rmpv::Value) -> Value {
        match v {
            rmpv::Value::Nil => Value::Null,
            rmpv::Value::Boolean(b) => Value::Bool(b),
            rmpv::Value::Integer(i) => Value::Int(i.as_i64().unwrap_or(0)),
            rmpv::Value::F64(f) => Value::Float(f),
            rmpv::Value::F32(f) => Value::Float(f64::from(f)),
            rmpv::Value::String(s) => Value::String(s.into_str().unwrap_or_default()),
            _ => Value::Null,
        }
    }

    // --- IndexMutationObserver tests ---

    #[test]
    fn on_put_indexes_lww_record() {
        let registry = Arc::new(IndexRegistry::new());
        registry.add_hash_index("name");
        let observer = IndexMutationObserver::new(Arc::clone(&registry));

        let record = make_lww_record(vec![
            ("name", rmpv::Value::String("alice".into())),
        ]);
        observer.on_put("k1", &record, None, false);

        let idx = registry.get_index("name").unwrap();
        let result = idx.lookup_eq(&rmpv::Value::String("alice".into()));
        assert!(result.contains("k1"), "k1 should appear in hash index");
    }

    #[test]
    fn on_put_skips_ormap_record() {
        let registry = Arc::new(IndexRegistry::new());
        registry.add_hash_index("name");
        let observer = IndexMutationObserver::new(Arc::clone(&registry));

        let record = make_ormap_record();
        observer.on_put("k1", &record, None, false);

        let idx = registry.get_index("name").unwrap();
        assert_eq!(idx.entry_count(), 0, "OrMap records should not be indexed");
    }

    #[test]
    fn on_put_skips_ortombstones_record() {
        let registry = Arc::new(IndexRegistry::new());
        registry.add_hash_index("name");
        let observer = IndexMutationObserver::new(Arc::clone(&registry));

        let record = make_ortombstones_record();
        observer.on_put("k1", &record, None, false);

        let idx = registry.get_index("name").unwrap();
        assert_eq!(idx.entry_count(), 0, "OrTombstones records should not be indexed");
    }

    #[test]
    fn on_remove_removes_from_index() {
        let registry = Arc::new(IndexRegistry::new());
        registry.add_hash_index("name");
        let observer = IndexMutationObserver::new(Arc::clone(&registry));

        let record = make_lww_record(vec![
            ("name", rmpv::Value::String("alice".into())),
        ]);
        observer.on_put("k1", &record, None, false);
        assert_eq!(registry.get_index("name").unwrap().entry_count(), 1);

        observer.on_remove("k1", &record, false);
        let idx = registry.get_index("name").unwrap();
        let result = idx.lookup_eq(&rmpv::Value::String("alice".into()));
        assert!(!result.contains("k1"), "k1 should be removed from index");
    }

    #[test]
    fn on_evict_removes_from_index() {
        let registry = Arc::new(IndexRegistry::new());
        registry.add_hash_index("status");
        let observer = IndexMutationObserver::new(Arc::clone(&registry));

        let record = make_lww_record(vec![
            ("status", rmpv::Value::String("active".into())),
        ]);
        observer.on_put("k1", &record, None, false);
        observer.on_evict("k1", &record, false);

        let idx = registry.get_index("status").unwrap();
        let result = idx.lookup_eq(&rmpv::Value::String("active".into()));
        assert!(!result.contains("k1"), "evicted record should be removed from index");
    }

    #[test]
    fn on_load_adds_to_index() {
        let registry = Arc::new(IndexRegistry::new());
        registry.add_hash_index("name");
        let observer = IndexMutationObserver::new(Arc::clone(&registry));

        let record = make_lww_record(vec![
            ("name", rmpv::Value::String("bob".into())),
        ]);
        observer.on_load("k1", &record, false);

        let idx = registry.get_index("name").unwrap();
        let result = idx.lookup_eq(&rmpv::Value::String("bob".into()));
        assert!(result.contains("k1"), "loaded record should be indexed");
    }

    #[test]
    fn on_replication_put_indexes_when_populate_index_true() {
        let registry = Arc::new(IndexRegistry::new());
        registry.add_hash_index("name");
        let observer = IndexMutationObserver::new(Arc::clone(&registry));

        let record = make_lww_record(vec![
            ("name", rmpv::Value::String("carol".into())),
        ]);
        observer.on_replication_put("k1", &record, true);

        let idx = registry.get_index("name").unwrap();
        let result = idx.lookup_eq(&rmpv::Value::String("carol".into()));
        assert!(result.contains("k1"), "replication record should be indexed");
    }

    #[test]
    fn on_replication_put_skips_when_populate_index_false() {
        let registry = Arc::new(IndexRegistry::new());
        registry.add_hash_index("name");
        let observer = IndexMutationObserver::new(Arc::clone(&registry));

        let record = make_lww_record(vec![
            ("name", rmpv::Value::String("dave".into())),
        ]);
        observer.on_replication_put("k1", &record, false);

        let idx = registry.get_index("name").unwrap();
        assert_eq!(idx.entry_count(), 0, "should not index when populate_index=false");
    }

    #[test]
    fn on_update_reindexes_changed_value() {
        let registry = Arc::new(IndexRegistry::new());
        registry.add_hash_index("name");
        let observer = IndexMutationObserver::new(Arc::clone(&registry));

        let old_record = make_lww_record(vec![
            ("name", rmpv::Value::String("alice".into())),
        ]);
        observer.on_put("k1", &old_record, None, false);

        let new_record = make_lww_record(vec![
            ("name", rmpv::Value::String("bob".into())),
        ]);
        observer.on_update("k1", &new_record, &old_record.value, &new_record.value, false);

        let idx = registry.get_index("name").unwrap();
        let old_result = idx.lookup_eq(&rmpv::Value::String("alice".into()));
        assert!(!old_result.contains("k1"), "old value should be removed from index");
        let new_result = idx.lookup_eq(&rmpv::Value::String("bob".into()));
        assert!(new_result.contains("k1"), "new value should be in index");
    }

    #[test]
    fn on_reset_empties_all_indexes() {
        let registry = Arc::new(IndexRegistry::new());
        registry.add_hash_index("name");
        let observer = IndexMutationObserver::new(Arc::clone(&registry));

        let record = make_lww_record(vec![
            ("name", rmpv::Value::String("alice".into())),
        ]);
        observer.on_put("k1", &record, None, false);
        assert_eq!(registry.get_index("name").unwrap().entry_count(), 1);

        observer.on_reset();
        assert_eq!(
            registry.get_index("name").unwrap().entry_count(),
            0,
            "on_reset should empty all indexes"
        );
    }

    #[test]
    fn on_clear_empties_all_indexes() {
        let registry = Arc::new(IndexRegistry::new());
        registry.add_hash_index("name");
        registry.add_navigable_index("age");
        let observer = IndexMutationObserver::new(Arc::clone(&registry));

        let r1 = make_lww_record(vec![("name", rmpv::Value::String("alice".into()))]);
        let r2 = make_lww_record(vec![("name", rmpv::Value::String("bob".into()))]);
        observer.on_put("k1", &r1, None, false);
        observer.on_put("k2", &r2, None, false);
        assert_eq!(registry.get_index("name").unwrap().entry_count(), 2);

        observer.on_clear();
        assert_eq!(
            registry.get_index("name").unwrap().entry_count(),
            0,
            "on_clear should empty all indexes"
        );
        let result = registry
            .get_index("name")
            .unwrap()
            .lookup_eq(&rmpv::Value::String("alice".into()));
        assert!(result.is_empty(), "cleared index returns empty lookup");
    }

    #[test]
    fn on_destroy_clears_all_indexes() {
        let registry = Arc::new(IndexRegistry::new());
        registry.add_hash_index("field");
        let observer = IndexMutationObserver::new(Arc::clone(&registry));

        let record = make_lww_record(vec![("field", rmpv::Value::String("val".into()))]);
        observer.on_put("k1", &record, None, false);
        observer.on_destroy(false);

        assert_eq!(
            registry.get_index("field").unwrap().entry_count(),
            0,
            "on_destroy should clear all indexes"
        );
    }

    #[test]
    fn insert_remove_100_records_no_leaks() {
        let registry = Arc::new(IndexRegistry::new());
        registry.add_hash_index("id");
        let observer = IndexMutationObserver::new(Arc::clone(&registry));

        let records: Vec<(String, Record)> = (0..100)
            .map(|i| {
                let key = format!("key-{i}");
                let record = make_lww_record(vec![
                    ("id", rmpv::Value::Integer(i.into())),
                ]);
                (key, record)
            })
            .collect();

        for (key, record) in &records {
            observer.on_put(key, record, None, false);
        }
        assert_eq!(
            registry.get_index("id").unwrap().entry_count(),
            100,
            "should have 100 entries after 100 inserts"
        );

        for (key, record) in &records {
            observer.on_remove(key, record, false);
        }
        assert_eq!(
            registry.get_index("id").unwrap().entry_count(),
            0,
            "should have 0 entries after removing all 100 records"
        );
    }

    // --- IndexObserverFactory tests ---

    #[test]
    fn factory_creates_observer_for_registered_map() {
        let factory = IndexObserverFactory::new();
        factory.register_map("users");

        let observer = factory.create_observer("users", 0);
        assert!(
            observer.is_some(),
            "should return observer for registered map"
        );
    }

    #[test]
    fn factory_returns_none_for_unregistered_map() {
        let factory = IndexObserverFactory::new();
        let observer = factory.create_observer("unknown", 0);
        assert!(
            observer.is_none(),
            "should return None for unregistered map"
        );
    }

    #[test]
    fn factory_register_map_is_idempotent() {
        let factory = IndexObserverFactory::new();
        let registry1 = factory.register_map("users");
        let registry1_ptr = Arc::as_ptr(&registry1);

        let registry2 = factory.register_map("users");
        let registry2_ptr = Arc::as_ptr(&registry2);

        assert_eq!(
            registry1_ptr, registry2_ptr,
            "repeated register_map returns the same registry"
        );
    }

    #[test]
    fn factory_register_map_returns_usable_registry() {
        let factory = Arc::new(IndexObserverFactory::new());
        let registry = factory.register_map("products");
        registry.add_hash_index("sku");

        let observer = factory
            .create_observer("products", 0)
            .expect("should create observer");

        let record = make_lww_record(vec![("sku", rmpv::Value::String("ABC123".into()))]);
        observer.on_put("p1", &record, None, false);

        let idx = registry.get_index("sku").unwrap();
        let result = idx.lookup_eq(&rmpv::Value::String("ABC123".into()));
        assert!(result.contains("p1"), "observer should update the registry's indexes");
    }
}
