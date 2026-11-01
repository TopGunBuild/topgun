//! Mutation observer that keeps secondary indexes in sync with record mutations.
//!
//! [`IndexMutationObserver`] implements [`MutationObserver`] and routes each
//! mutation event to all indexes registered in its map's [`IndexRegistry`].
//!
//! [`IndexObserverFactory`] implements [`ObserverFactory`] and gives every
//! store an [`IndexMutationObserver`]. The observer looks its map's registry up
//! at each event and does nothing while the map has none, so an index created
//! on a map whose stores already exist receives every later mutation
//! (`TG-IDX-001`).
//!
//! Vector indexes receive mutations through the same path as scalar indexes.
//! The observer passes the full record `rmpv::Value` to every registered
//! index; each index internally extracts its own attribute via `AttributeExtractor`.
//! For `VectorIndex`, the per-index extraction navigates to the configured
//! attribute, expects a `Binary` field containing a MsgPack-encoded vector,
//! and queues an upsert. Records without the vector attribute or with a
//! non-Binary attribute value are silently skipped — no observer-level
//! branching is required.

use std::sync::Arc;

use dashmap::DashMap;

use crate::service::domain::predicate::value_to_rmpv;
use crate::storage::factory::ObserverFactory;
use crate::storage::mutation_observer::MutationObserver;
use crate::storage::record::{Record, RecordValue};

use super::registry::{IndexRegistry, IndexStats};

// ---------------------------------------------------------------------------
// IndexMutationObserver
// ---------------------------------------------------------------------------

/// `MutationObserver` that routes mutations to all indexes in its map's
/// [`IndexRegistry`].
///
/// The registry is looked up at each event, not captured when the store is
/// created: the factory is consulted once per store, and the map's first index
/// can be created after its stores exist (`TG-IDX-001`). An event of a map with
/// no registry changes nothing.
///
/// Each method extracts the attribute value from the record's `RecordValue` and
/// dispatches to the appropriate index operation. Only `RecordValue::Lww` records
/// are indexed; `OrMap` and `OrTombstones` are skipped because OR-Map records are
/// multi-entry and not suitable for single-attribute indexes.
pub struct IndexMutationObserver {
    registries: Arc<DashMap<String, Arc<IndexRegistry>>>,
    map_name: String,
}

impl IndexMutationObserver {
    /// An observer of a map named `test` whose registry is `registry`.
    #[cfg(test)]
    fn new(registry: Arc<IndexRegistry>) -> Self {
        let registries = DashMap::new();
        registries.insert("test".to_string(), registry);
        Self {
            registries: Arc::new(registries),
            map_name: "test".to_string(),
        }
    }

    /// The map's registry at this moment. The shard guard is dropped before
    /// this returns, so no `DashMap` guard is held while an index is mutated.
    fn registry(&self) -> Option<Arc<IndexRegistry>> {
        self.registries
            .get(&self.map_name)
            .map(|registry| Arc::clone(registry.value()))
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
    fn on_put(
        &self,
        key: &str,
        record: &Record,
        _old_value: Option<&RecordValue>,
        _is_backup: bool,
    ) {
        let Some(rmpv_val) = Self::extract_rmpv(&record.value) else {
            return;
        };
        let Some(registry) = self.registry() else {
            return;
        };
        // Each index internally uses its own AttributeExtractor to pull its
        // attribute from the full record value, so we pass the whole map.
        for index in registry.indexes() {
            index.insert(key, &rmpv_val);
        }
    }

    fn on_update(
        &self,
        key: &str,
        _record: &Record,
        _old_value: &RecordValue,
        new_value: &RecordValue,
        _is_backup: bool,
    ) {
        let Some(new_rmpv) = Self::extract_rmpv(new_value) else {
            return;
        };
        let Some(registry) = self.registry() else {
            return;
        };
        for index in registry.indexes() {
            index.insert(key, &new_rmpv);
        }
    }

    fn on_remove(&self, key: &str, record: &Record, _is_backup: bool) {
        let Some(rmpv_val) = Self::extract_rmpv(&record.value) else {
            return;
        };
        let Some(registry) = self.registry() else {
            return;
        };
        // A scalar index removes by its reverse entry and ignores the value;
        // the vector index still needs it to forget the vector's hash.
        for index in registry.indexes() {
            index.remove(key, &rmpv_val);
        }
    }

    fn on_remove_key(&self, key: &str, _is_backup: bool) {
        let Some(registry) = self.registry() else {
            return;
        };
        for index in registry.indexes() {
            index.remove_key(key);
        }
    }

    // An index covers the map's data, not its residency: an evicted record is
    // still stored, so its entry stays (`TG-IDX-001`).
    fn on_evict(&self, _key: &str, _record: &Record, _is_backup: bool) {}

    fn on_load(&self, key: &str, record: &Record, _is_backup: bool) {
        let Some(rmpv_val) = Self::extract_rmpv(&record.value) else {
            return;
        };
        let Some(registry) = self.registry() else {
            return;
        };
        // A loaded value is never newer than an entry a write put there.
        for index in registry.indexes() {
            index.insert_if_absent(key, &rmpv_val);
        }
    }

    fn on_replication_put(&self, key: &str, record: &Record, populate_index: bool) {
        if !populate_index {
            return;
        }
        let Some(rmpv_val) = Self::extract_rmpv(&record.value) else {
            return;
        };
        let Some(registry) = self.registry() else {
            return;
        };
        for index in registry.indexes() {
            index.insert(key, &rmpv_val);
        }
    }

    fn on_clear(&self) {
        let Some(registry) = self.registry() else {
            return;
        };
        for index in registry.indexes() {
            index.clear();
        }
    }

    fn on_reset(&self) {
        let Some(registry) = self.registry() else {
            return;
        };
        for index in registry.indexes() {
            index.clear();
        }
    }

    fn on_destroy(&self, _is_shutdown: bool) {
        let Some(registry) = self.registry() else {
            return;
        };
        for index in registry.indexes() {
            index.clear();
        }
    }
}

// ---------------------------------------------------------------------------
// IndexObserverFactory
// ---------------------------------------------------------------------------

/// Factory that gives every store an [`IndexMutationObserver`] over the
/// registries it holds.
///
/// Only `register_map` creates a registry; `create_observer` never does, so a
/// map without an index has no registry and its events reach no index.
pub struct IndexObserverFactory {
    /// Map from map name to its [`IndexRegistry`], shared with every observer
    /// this factory created.
    registries: Arc<DashMap<String, Arc<IndexRegistry>>>,
}

impl IndexObserverFactory {
    /// Creates a new factory with no registered maps.
    #[must_use]
    pub fn new() -> Self {
        Self {
            registries: Arc::new(DashMap::new()),
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

    /// Returns the [`IndexRegistry`] for `map_name`, or `None` if the map has not
    /// been registered.
    ///
    /// Unlike `register_map`, this method does not create a registry when one is
    /// absent — it is a pure read-only lookup suitable for query-time index selection.
    #[must_use]
    pub fn get_registry(&self, map_name: &str) -> Option<Arc<IndexRegistry>> {
        self.registries.get(map_name).map(|r| Arc::clone(r.value()))
    }

    /// Returns a snapshot of all indexes across all maps.
    ///
    /// Each entry is `(map_name, Vec<IndexStats>)`. Maps with no registered
    /// indexes are excluded from the result.
    #[must_use]
    pub fn all_index_stats(&self) -> Vec<(String, Vec<IndexStats>)> {
        self.registries
            .iter()
            .map(|r| (r.key().clone(), r.value().stats()))
            .filter(|(_, stats)| !stats.is_empty())
            .collect()
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
        Some(Arc::new(IndexMutationObserver {
            registries: Arc::clone(&self.registries),
            map_name: map_name.to_string(),
        }))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use topgun_core::hlc::Timestamp;
    use topgun_core::types::Value;

    use super::*;
    use crate::service::domain::index::Index;
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
            value: RecordValue::OrMap {
                records: vec![],
                tombstones: vec![],
            },
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
            rmpv::Value::Boolean(b) => Value::Bool(b),
            rmpv::Value::Integer(i) => Value::Int(i.as_i64().unwrap_or(0)),
            rmpv::Value::F64(f) => Value::Float(f),
            rmpv::Value::F32(f) => Value::Float(f64::from(f)),
            rmpv::Value::String(s) => Value::String(s.into_str().unwrap_or_default()),
            // Nil, Binary, Array, Map, Ext — not needed for scalar index tests.
            _ => Value::Null,
        }
    }

    // --- IndexMutationObserver tests ---

    #[test]
    fn on_put_indexes_lww_record() {
        let registry = Arc::new(IndexRegistry::new());
        registry.add_hash_index("name");
        let observer = IndexMutationObserver::new(Arc::clone(&registry));

        let record = make_lww_record(vec![("name", rmpv::Value::String("alice".into()))]);
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
        assert_eq!(
            idx.entry_count(),
            0,
            "OrTombstones records should not be indexed"
        );
    }

    #[test]
    fn on_remove_removes_from_index() {
        let registry = Arc::new(IndexRegistry::new());
        registry.add_hash_index("name");
        let observer = IndexMutationObserver::new(Arc::clone(&registry));

        let record = make_lww_record(vec![("name", rmpv::Value::String("alice".into()))]);
        observer.on_put("k1", &record, None, false);
        assert_eq!(registry.get_index("name").unwrap().entry_count(), 1);

        observer.on_remove("k1", &record, false);
        let idx = registry.get_index("name").unwrap();
        let result = idx.lookup_eq(&rmpv::Value::String("alice".into()));
        assert!(!result.contains("k1"), "k1 should be removed from index");
    }

    /// One index of each scalar type, on attributes `h`, `n` and `i`, so one
    /// record value `{h, n, i}` reaches all three.
    fn scalar_registry() -> Arc<IndexRegistry> {
        let registry = Arc::new(IndexRegistry::new());
        registry.add_hash_index("h");
        registry.add_navigable_index("n");
        registry.add_inverted_index("i");
        registry
    }

    fn scalar_record(v: &str) -> Record {
        make_lww_record(vec![
            ("h", rmpv::Value::String(v.into())),
            ("n", rmpv::Value::String(v.into())),
            ("i", rmpv::Value::String(v.into())),
        ])
    }

    /// Keys the index of `attribute` holds under `v`, through the lookup its
    /// type supports.
    fn scalar_lookup(registry: &IndexRegistry, attribute: &str, v: &str) -> HashSet<String> {
        let idx = registry.get_index(attribute).unwrap();
        if attribute == "i" {
            idx.lookup_contains(v)
        } else {
            idx.lookup_eq(&rmpv::Value::String(v.into()))
        }
    }

    const SCALAR_ATTRIBUTES: [&str; 3] = ["h", "n", "i"];

    #[test]
    fn on_evict_keeps_index_entries() {
        let registry = scalar_registry();
        let observer = IndexMutationObserver::new(Arc::clone(&registry));

        let record = scalar_record("active");
        observer.on_put("k1", &record, None, false);
        observer.on_evict("k1", &record, false);

        for attribute in SCALAR_ATTRIBUTES {
            assert!(
                scalar_lookup(&registry, attribute, "active").contains("k1"),
                "an evicted record is still stored, so index {attribute} keeps it"
            );
            assert_eq!(registry.get_index(attribute).unwrap().entry_count(), 1);
        }
    }

    #[test]
    fn tg_idx_001_rewrite_after_evict_replaces_the_entry() {
        const N: usize = 50;
        let registry = scalar_registry();
        let observer = IndexMutationObserver::new(Arc::clone(&registry));
        let (old, new) = (scalar_record("old"), scalar_record("new"));

        for k in 0..N {
            let key = format!("k{k}");
            observer.on_put(&key, &old, None, false);
            observer.on_evict(&key, &old, false);
            // A rewrite of a key that is not resident has no old value.
            observer.on_put(&key, &new, None, false);
        }

        for attribute in SCALAR_ATTRIBUTES {
            assert!(
                scalar_lookup(&registry, attribute, "old").is_empty(),
                "index {attribute} keeps no entry under the old value"
            );
            let new_keys = scalar_lookup(&registry, attribute, "new");
            assert_eq!(
                new_keys.len(),
                N,
                "index {attribute} holds every key under new"
            );
            assert_eq!(
                registry.get_index(attribute).unwrap().entry_count(),
                N as u64,
                "index {attribute} holds one entry per key"
            );
        }
    }

    #[test]
    fn tg_idx_001_on_load_keeps_a_newer_put() {
        let registry = scalar_registry();
        let observer = IndexMutationObserver::new(Arc::clone(&registry));

        observer.on_put("k1", &scalar_record("newer"), None, false);
        observer.on_load("k1", &scalar_record("older"), false);

        for attribute in SCALAR_ATTRIBUTES {
            assert!(scalar_lookup(&registry, attribute, "newer").contains("k1"));
            assert!(
                !scalar_lookup(&registry, attribute, "older").contains("k1"),
                "a load must not replace the entry of index {attribute}"
            );
            assert_eq!(registry.get_index(attribute).unwrap().entry_count(), 1);
        }
    }

    #[test]
    fn tg_idx_001_on_remove_key_removes_from_every_index() {
        let registry = scalar_registry();
        let observer = IndexMutationObserver::new(Arc::clone(&registry));

        observer.on_put("k1", &scalar_record("gone"), None, false);
        observer.on_put("k2", &scalar_record("kept"), None, false);
        observer.on_remove_key("k1", false);

        for attribute in SCALAR_ATTRIBUTES {
            assert!(
                scalar_lookup(&registry, attribute, "gone").is_empty(),
                "index {attribute} must drop the removed key"
            );
            assert!(scalar_lookup(&registry, attribute, "kept").contains("k2"));
            assert_eq!(registry.get_index(attribute).unwrap().entry_count(), 1);
        }
    }

    #[test]
    fn on_load_adds_to_index() {
        let registry = Arc::new(IndexRegistry::new());
        registry.add_hash_index("name");
        let observer = IndexMutationObserver::new(Arc::clone(&registry));

        let record = make_lww_record(vec![("name", rmpv::Value::String("bob".into()))]);
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

        let record = make_lww_record(vec![("name", rmpv::Value::String("carol".into()))]);
        observer.on_replication_put("k1", &record, true);

        let idx = registry.get_index("name").unwrap();
        let result = idx.lookup_eq(&rmpv::Value::String("carol".into()));
        assert!(
            result.contains("k1"),
            "replication record should be indexed"
        );
    }

    #[test]
    fn on_replication_put_skips_when_populate_index_false() {
        let registry = Arc::new(IndexRegistry::new());
        registry.add_hash_index("name");
        let observer = IndexMutationObserver::new(Arc::clone(&registry));

        let record = make_lww_record(vec![("name", rmpv::Value::String("dave".into()))]);
        observer.on_replication_put("k1", &record, false);

        let idx = registry.get_index("name").unwrap();
        assert_eq!(
            idx.entry_count(),
            0,
            "should not index when populate_index=false"
        );
    }

    #[test]
    fn on_update_reindexes_changed_value() {
        let registry = Arc::new(IndexRegistry::new());
        registry.add_hash_index("name");
        let observer = IndexMutationObserver::new(Arc::clone(&registry));

        let old_record = make_lww_record(vec![("name", rmpv::Value::String("alice".into()))]);
        observer.on_put("k1", &old_record, None, false);

        let new_record = make_lww_record(vec![("name", rmpv::Value::String("bob".into()))]);
        observer.on_update(
            "k1",
            &new_record,
            &old_record.value,
            &new_record.value,
            false,
        );

        let idx = registry.get_index("name").unwrap();
        let old_result = idx.lookup_eq(&rmpv::Value::String("alice".into()));
        assert!(
            !old_result.contains("k1"),
            "old value should be removed from index"
        );
        let new_result = idx.lookup_eq(&rmpv::Value::String("bob".into()));
        assert!(new_result.contains("k1"), "new value should be in index");
    }

    #[test]
    fn on_reset_empties_all_indexes() {
        let registry = Arc::new(IndexRegistry::new());
        registry.add_hash_index("name");
        let observer = IndexMutationObserver::new(Arc::clone(&registry));

        let record = make_lww_record(vec![("name", rmpv::Value::String("alice".into()))]);
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
                let record = make_lww_record(vec![("id", rmpv::Value::Integer(i.into()))]);
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
    fn factory_creates_observer_for_unregistered_map_without_registering_it() {
        let factory = IndexObserverFactory::new();
        let observer = factory.create_observer("unknown", 0);
        assert!(
            observer.is_some(),
            "the map's first index can be created after its stores exist"
        );
        assert!(
            factory.get_registry("unknown").is_none(),
            "creating an observer registers no map"
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
        assert!(
            result.contains("p1"),
            "observer should update the registry's indexes"
        );
    }

    #[test]
    fn all_index_stats_returns_only_maps_with_indexes() {
        let factory = IndexObserverFactory::new();

        // Register a map with indexes.
        let registry = factory.register_map("users");
        registry.add_hash_index("email");
        registry.add_navigable_index("age");

        // Register a map with no indexes.
        factory.register_map("empty_map");

        let stats = factory.all_index_stats();

        // Only "users" should appear (empty_map has no indexes).
        assert_eq!(stats.len(), 1, "only maps with indexes should be returned");
        let (map_name, index_stats) = &stats[0];
        assert_eq!(map_name, "users");
        assert_eq!(index_stats.len(), 2, "users should have 2 indexes");
    }

    #[test]
    fn all_index_stats_returns_empty_when_no_maps_registered() {
        let factory = IndexObserverFactory::new();
        let stats = factory.all_index_stats();
        assert!(
            stats.is_empty(),
            "should return empty when no maps are registered"
        );
    }

    #[test]
    fn all_index_stats_includes_entry_counts() {
        let factory = IndexObserverFactory::new();
        let registry = factory.register_map("products");
        registry.add_hash_index("sku");

        // Insert a record to increment entry count.
        let idx = registry.get_index("sku").unwrap();
        idx.insert("p1", &rmpv::Value::String("ABC".into()));

        let stats = factory.all_index_stats();
        assert_eq!(stats.len(), 1);
        let (_, index_stats) = &stats[0];
        assert_eq!(index_stats.len(), 1);
        assert_eq!(
            index_stats[0].entry_count, 1,
            "entry count should reflect inserted records"
        );
    }

    // --- VectorIndex observer integration tests ---

    /// Build a Record whose LWW value is a Map with the given `field` set to
    /// `Value::Bytes(encoded_vector_bytes)`. This is the canonical on-wire form:
    /// `Vector::F32(...)` → `rmp_serde::to_vec_named` → `Value::Bytes` →
    /// `value_to_rmpv` → `rmpv::Value::Binary`.
    fn make_vector_record(field: &str, data: &[f32]) -> Record {
        use topgun_core::types::Value;
        use topgun_core::vector::Vector;

        let v = Vector::F32(data.to_vec());
        let encoded = rmp_serde::to_vec_named(&v).expect("vector serialization failed");
        let mut inner_map = std::collections::BTreeMap::new();
        inner_map.insert(field.to_string(), Value::Bytes(encoded));
        Record {
            value: RecordValue::Lww {
                value: Value::Map(inner_map),
                timestamp: ts(),
            },
            metadata: RecordMetadata::new(1_000_000, 64),
        }
    }

    #[test]
    fn vector_index_receives_upserts_via_observer() {
        use topgun_core::vector::DistanceMetric;

        let registry = Arc::new(IndexRegistry::new());
        registry.add_vector_index("embedding", 4, DistanceMetric::Cosine);

        let observer = IndexMutationObserver::new(Arc::clone(&registry));
        let record = make_vector_record("embedding", &[0.1, 0.2, 0.3, 0.4]);
        observer.on_put("k1", &record, None, false);

        let vi = registry
            .get_vector_index("embedding")
            .expect("vector index must exist");
        assert_eq!(
            vi.pending_count(),
            1,
            "observer should have queued one upsert"
        );

        vi.commit_pending();
        assert_eq!(
            vi.entry_count(),
            1,
            "after commit one entry should be committed"
        );
    }

    #[test]
    fn vector_index_ignores_non_vector_fields() {
        use topgun_core::vector::DistanceMetric;

        let registry = Arc::new(IndexRegistry::new());
        registry.add_vector_index("embedding", 4, DistanceMetric::Cosine);

        let observer = IndexMutationObserver::new(Arc::clone(&registry));
        // Record contains "title" but not "embedding" — insert must be a no-op.
        let record = make_lww_record(vec![("title", rmpv::Value::String("hello".into()))]);
        observer.on_put("k1", &record, None, false);

        let vi = registry
            .get_vector_index("embedding")
            .expect("vector index must exist");
        assert_eq!(
            vi.pending_count(),
            0,
            "insert with no vector field should not queue a mutation"
        );
    }

    // --- TG-IDX-001: stores created before the map's registry ---

    mod existing_stores {
        use std::collections::{BTreeMap, HashSet};
        use std::sync::atomic::{AtomicBool, AtomicU64};
        use std::sync::Arc;

        use topgun_core::hlc::Timestamp;
        use topgun_core::types::Value;

        use super::super::*;
        use crate::network::handlers::admin_types::{BackfillProgress, RebuildType};
        use crate::service::domain::index::scalar_rebuild::backfill_scalar_index;
        use crate::storage::datastores::NullDataStore;
        use crate::storage::factory::RecordStoreFactory;
        use crate::storage::impls::StorageConfig;
        use crate::storage::key_writer::KeyWriteToken;
        use crate::storage::record_store::{CallerProvenance, ExpiryPolicy, RecordStore};

        /// A store factory with `index_factory` wired in, as the server wires it.
        fn wired() -> (Arc<IndexObserverFactory>, Arc<RecordStoreFactory>) {
            let index_factory = Arc::new(IndexObserverFactory::new());
            let store_factory = Arc::new(
                RecordStoreFactory::new(
                    StorageConfig::default(),
                    Arc::new(NullDataStore),
                    Vec::new(),
                )
                .with_observer_factories(vec![
                    Arc::clone(&index_factory) as Arc<dyn ObserverFactory>
                ]),
            );
            (index_factory, store_factory)
        }

        fn status_record(status: &str) -> RecordValue {
            RecordValue::Lww {
                value: Value::Map(BTreeMap::from([(
                    "status".to_string(),
                    Value::String(status.to_string()),
                )])),
                timestamp: Timestamp {
                    millis: 1_000,
                    counter: 0,
                    node_id: "n".to_string(),
                },
            }
        }

        async fn put(store: &Arc<dyn RecordStore>, key: &str, status: &str) {
            store
                .put(
                    &KeyWriteToken::for_test(store.name(), key),
                    key,
                    status_record(status),
                    ExpiryPolicy::NONE,
                    CallerProvenance::Client,
                )
                .await
                .expect("put");
        }

        async fn remove(store: &Arc<dyn RecordStore>, key: &str) {
            store
                .remove(
                    &KeyWriteToken::for_test(store.name(), key),
                    key,
                    CallerProvenance::Client,
                )
                .await
                .expect("remove");
        }

        /// Registers `map` with a hash index on `status` and backfills it from
        /// the stored records, as `create_index` does.
        async fn create_status_index(
            index_factory: &IndexObserverFactory,
            store_factory: &RecordStoreFactory,
            map: &str,
        ) -> Arc<dyn crate::service::domain::index::Index> {
            let registry = index_factory.register_map(map);
            registry.add_hash_index("status");
            let progress = BackfillProgress {
                total: AtomicU64::new(0),
                processed: AtomicU64::new(0),
                done: AtomicBool::new(false),
                rebuild_type: RebuildType::Backfill,
            };
            backfill_scalar_index(store_factory, map, "status", &registry, &progress)
                .await
                .expect("backfill");
            registry.get_index("status").expect("index registered")
        }

        fn lookup(
            index: &Arc<dyn crate::service::domain::index::Index>,
            status: &str,
        ) -> HashSet<String> {
            index.lookup_eq(&rmpv::Value::from(status))
        }

        fn keys(names: &[&str]) -> HashSet<String> {
            names.iter().map(ToString::to_string).collect()
        }

        /// R4.1: a put and a delete on a store created before the map's first
        /// index reach that index.
        #[tokio::test]
        async fn tg_idx_001_create_index_on_existing_store_indexes_later_writes() {
            let (index_factory, store_factory) = wired();
            let store = store_factory.get_or_create("m", 7);
            put(&store, "k1", "one").await;

            let index = create_status_index(&index_factory, &store_factory, "m").await;
            assert_eq!(
                lookup(&index, "one"),
                keys(&["k1"]),
                "the backfill holds k1"
            );

            put(&store, "k2", "two").await;
            assert_eq!(
                lookup(&index, "two"),
                keys(&["k2"]),
                "a put on a store created before the index reaches it"
            );

            remove(&store, "k1").await;
            assert!(
                lookup(&index, "one").is_empty(),
                "a delete on a store created before the index reaches it"
            );
            assert_eq!(lookup(&index, "two"), keys(&["k2"]));
            assert_eq!(index.entry_count(), 1);
        }

        /// R4.2: an overwrite (`on_update`) on a store created before the map's
        /// first index replaces the entry the backfill made.
        #[tokio::test]
        async fn tg_idx_001_overwrite_after_backfill_on_existing_store_replaces_entry() {
            let (index_factory, store_factory) = wired();
            let store = store_factory.get_or_create("m", 7);
            put(&store, "k1", "old").await;

            let index = create_status_index(&index_factory, &store_factory, "m").await;
            assert_eq!(
                lookup(&index, "old"),
                keys(&["k1"]),
                "the backfill holds k1"
            );
            assert_eq!(index.entry_count(), 1);

            put(&store, "k1", "new").await;
            assert_eq!(
                lookup(&index, "new"),
                keys(&["k1"]),
                "the overwrite moves k1 under its new value"
            );
            assert!(
                lookup(&index, "old").is_empty(),
                "no entry stays under the old value"
            );
            assert_eq!(index.entry_count(), 1);
        }

        /// R4.4: writes to a map without an index register no map and reach no
        /// other map's index.
        #[tokio::test]
        async fn tg_idx_001_index_observer_registers_no_map_without_an_index() {
            let (index_factory, store_factory) = wired();
            let index_a = create_status_index(&index_factory, &store_factory, "A").await;
            let store_a = store_factory.get_or_create("A", 1);
            put(&store_a, "a1", "a-active").await;
            put(&store_a, "a2", "a-idle").await;
            let count_a = index_a.entry_count();
            let stats = index_factory.all_index_stats();

            let b_values = ["b-open", "b-closed", "b-held"];
            for (partition, status) in (0u32..).zip(b_values) {
                let store_b = store_factory.get_or_create("B", partition);
                put(&store_b, &format!("b{partition}"), status).await;
            }
            remove(&store_factory.get_or_create("B", 0), "b0").await;

            // This assertion, not `all_index_stats()`, catches an observer
            // that registers the map: `all_index_stats()` drops registries
            // without indexes, so an empty registry for `B` would not show
            // there.
            assert!(
                index_factory.get_registry("B").is_none(),
                "writes to a map without an index create no registry"
            );
            assert_eq!(index_factory.all_index_stats(), stats);
            assert_eq!(index_a.entry_count(), count_a);
            for status in b_values {
                assert!(
                    lookup(&index_a, status).is_empty(),
                    "B's value {status} must not reach A's index"
                );
            }
            assert_eq!(lookup(&index_a, "a-active"), keys(&["a1"]));
        }

        /// R4.5: `clear`, `reset` and `destroy` on a store created before the
        /// map's first index empty that index.
        #[tokio::test]
        async fn tg_idx_001_clear_reset_destroy_on_store_created_before_registry_empty_the_index() {
            let (index_factory, store_factory) = wired();
            for map in ["cleared", "reset", "destroyed"] {
                let store = store_factory.get_or_create(map, 3);
                put(&store, "k1", "one").await;
                put(&store, "k2", "two").await;

                let index = create_status_index(&index_factory, &store_factory, map).await;
                assert_eq!(
                    index.entry_count(),
                    2,
                    "{map}: the backfill fills the index"
                );

                match map {
                    "cleared" => {
                        store.clear(false);
                    }
                    "reset" => store.reset(),
                    _ => store.destroy(),
                }
                assert_eq!(index.entry_count(), 0, "{map}: the index is emptied");
                assert!(lookup(&index, "one").is_empty(), "{map}");
            }
        }
    }
}
