//! Factory for creating fully-wired [`RecordStore`] instances.
//!
//! [`RecordStoreFactory`] is the dependency injection point that creates
//! [`DefaultRecordStore`] instances with all three layers connected:
//! [`HashMapStorage`] (Layer 1), a shared [`MapDataStore`] (Layer 3),
//! and a [`CompositeMutationObserver`] assembled from registered observers.
//!
//! Stores are cached by `(map_name, partition_id)` in a [`DashMap`] so that
//! repeated lookups for the same pair return the same [`Arc<dyn RecordStore>`].

use std::sync::Arc;

use dashmap::DashMap;

use crate::storage::engines::HashMapStorage;
use crate::storage::impls::{DefaultRecordStore, StorageConfig};
use crate::storage::map_data_store::MapDataStore;
use crate::storage::mutation_observer::{CompositeMutationObserver, MutationObserver};
use crate::storage::record_store::RecordStore;

/// Factory that creates per-map mutation observers at store-creation time.
///
/// Enables domain services (e.g., search indexing) to inject observers
/// that are wired with map-specific context when a [`RecordStore`] is created.
pub trait ObserverFactory: Send + Sync {
    /// Creates an observer for the given `(map_name, partition_id)` pair.
    ///
    /// Returns `None` if this factory does not need to observe the given map.
    fn create_observer(
        &self,
        map_name: &str,
        partition_id: u32,
    ) -> Option<Arc<dyn MutationObserver>>;
}

/// Factory for creating fully-wired [`RecordStore`] instances.
///
/// Holds shared configuration, the persistence backend, and a list of
/// mutation observers. Stores are cached by `(map_name, partition_id)` so
/// that [`get_or_create()`](RecordStoreFactory::get_or_create) returns the
/// same [`Arc<dyn RecordStore>`] for repeated lookups of the same pair.
pub struct RecordStoreFactory {
    config: StorageConfig,
    data_store: Arc<dyn MapDataStore>,
    observers: Vec<Arc<dyn MutationObserver>>,
    observer_factories: Vec<Arc<dyn ObserverFactory>>,
    store_cache: DashMap<(String, u32), Arc<dyn RecordStore>>,
}

impl RecordStoreFactory {
    /// Creates a new factory with the given configuration.
    ///
    /// Signature preserved for backward compatibility. For observer factory
    /// support, use [`with_observer_factories()`](RecordStoreFactory::with_observer_factories).
    #[must_use]
    pub fn new(
        config: StorageConfig,
        data_store: Arc<dyn MapDataStore>,
        observers: Vec<Arc<dyn MutationObserver>>,
    ) -> Self {
        Self {
            config,
            data_store,
            observers,
            observer_factories: Vec::new(),
            store_cache: DashMap::new(),
        }
    }

    /// Builder method that adds per-map observer factories.
    ///
    /// Each factory's [`create_observer()`](ObserverFactory::create_observer) is
    /// called on every first [`get_or_create()`](RecordStoreFactory::get_or_create)
    /// invocation for a new `(map_name, partition_id)` pair, and the returned
    /// observers are added to the per-store `CompositeMutationObserver`
    /// alongside the shared static observers from `new()`.
    #[must_use]
    pub fn with_observer_factories(
        mut self,
        factories: Vec<Arc<dyn ObserverFactory>>,
    ) -> Self {
        self.observer_factories = factories;
        self
    }

    /// Returns the cached [`RecordStore`] for the given map and partition,
    /// creating one if it does not exist yet.
    ///
    /// On first call for a `(map_name, partition_id)` pair, assembles a fresh
    /// [`HashMapStorage`] engine, clones the shared [`MapDataStore`] reference,
    /// and builds a [`CompositeMutationObserver`] from both the static observer
    /// list and any per-map observers returned by registered [`ObserverFactory`]
    /// instances. The resulting store is cached and returned as an `Arc`.
    ///
    /// On subsequent calls for the same pair, returns `Arc::clone()` of the
    /// cached store without invoking observer factories again.
    #[must_use]
    pub fn get_or_create(&self, map_name: &str, partition_id: u32) -> Arc<dyn RecordStore> {
        let key = (map_name.to_string(), partition_id);

        // Fast path: return cached store.
        if let Some(entry) = self.store_cache.get(&key) {
            return Arc::clone(entry.value());
        }

        // Slow path: create a new store and cache it.
        // Using entry API to avoid TOCTOU race between the get above and insert.
        self.store_cache
            .entry(key)
            .or_insert_with(|| {
                let engine = Box::new(HashMapStorage::new());

                // Start with the shared static observers.
                let mut all_observers = self.observers.clone();

                // Ask each factory for a per-map observer.
                for factory in &self.observer_factories {
                    if let Some(obs) = factory.create_observer(map_name, partition_id) {
                        all_observers.push(obs);
                    }
                }

                let observer = Arc::new(CompositeMutationObserver::new(all_observers));
                let record_store = DefaultRecordStore::new(
                    map_name.to_string(),
                    partition_id,
                    engine,
                    self.data_store.clone(),
                    observer,
                    self.config.clone(),
                );
                Arc::new(record_store)
            })
            .value()
            .clone()
    }

    /// Returns a sorted, deduplicated list of all map names in the store cache.
    ///
    /// Iterates the cache keys, extracts the map name (first element of each
    /// `(String, u32)` tuple), deduplicates, and returns sorted.
    #[must_use]
    pub fn map_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .store_cache
            .iter()
            .map(|entry| entry.key().0.clone())
            .collect();
        names.sort();
        names.dedup();
        names
    }

    /// Returns all cached stores for the given map name across all partitions.
    ///
    /// Iterates the store cache and collects entries where the map name matches.
    /// Returns an empty `Vec` if no stores exist for the map.
    ///
    /// Note: This is O(N) over the entire cache. Acceptable at current scale;
    /// consider a secondary index (`DashMap<String, Vec<u32>>`) if map count
    /// grows large.
    #[must_use]
    pub fn get_all_for_map(&self, map_name: &str) -> Vec<Arc<dyn RecordStore>> {
        self.store_cache
            .iter()
            .filter(|entry| entry.key().0 == map_name)
            .map(|entry| Arc::clone(entry.value()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use topgun_core::hlc::Timestamp;
    use topgun_core::types::Value;

    use super::*;
    use crate::storage::datastores::NullDataStore;
    use crate::storage::record::RecordValue;
    use crate::storage::record_store::{CallerProvenance, ExpiryPolicy};

    fn make_value(s: &str) -> RecordValue {
        RecordValue::Lww {
            value: Value::String(s.to_string()),
            timestamp: Timestamp {
                millis: 1_000_000,
                counter: 0,
                node_id: "node-1".to_string(),
            },
        }
    }

    /// get_or_create() returns a working Arc<dyn RecordStore>
    #[tokio::test]
    async fn factory_get_or_create_returns_working_record_store() {
        let factory = RecordStoreFactory::new(
            StorageConfig::default(),
            Arc::new(NullDataStore),
            Vec::new(),
        );

        let store = factory.get_or_create("users", 7);
        assert_eq!(store.name(), "users");
        assert_eq!(store.partition_id(), 7);

        // Put and get round-trip through Arc<dyn RecordStore>
        store
            .put("alice", make_value("data"), ExpiryPolicy::NONE, CallerProvenance::Client)
            .await
            .unwrap();

        let record = store.get("alice", false).await.unwrap();
        assert!(record.is_some());
        assert_eq!(store.size(), 1);
    }

    /// StorageConfig::default() has all fields set to 0
    #[test]
    fn storage_config_default_all_zeros() {
        let config = StorageConfig::default();
        assert_eq!(config.default_ttl_millis, 0);
        assert_eq!(config.default_max_idle_millis, 0);
        assert_eq!(config.max_entry_count, 0);
    }

    /// DefaultRecordStore compiles as Arc<dyn RecordStore> via factory
    #[test]
    fn factory_output_is_object_safe() {
        let factory = RecordStoreFactory::new(
            StorageConfig::default(),
            Arc::new(NullDataStore),
            Vec::new(),
        );
        let _store: Arc<dyn RecordStore> = factory.get_or_create("map", 0);
    }

    /// Factory creates independent stores for different map/partition pairs
    #[tokio::test]
    async fn factory_creates_independent_stores() {
        let factory = RecordStoreFactory::new(
            StorageConfig::default(),
            Arc::new(NullDataStore),
            Vec::new(),
        );

        let store_a = factory.get_or_create("map-a", 0);
        let store_b = factory.get_or_create("map-b", 1);

        store_a
            .put("key1", make_value("a"), ExpiryPolicy::NONE, CallerProvenance::Client)
            .await
            .unwrap();

        assert_eq!(store_a.size(), 1);
        assert_eq!(store_b.size(), 0, "stores should be independent");
    }

    /// Factory with custom config propagates settings to created stores
    #[tokio::test]
    async fn factory_propagates_config() {
        let config = StorageConfig {
            default_ttl_millis: 5000,
            default_max_idle_millis: 0,
            max_entry_count: 100,
        };
        let factory = RecordStoreFactory::new(config, Arc::new(NullDataStore), Vec::new());

        let store = factory.get_or_create("events", 3);
        assert!(store.is_expirable());
    }

    /// AC1: get_or_create() called twice for the same (map, partition) returns
    /// the same Arc instance (pointer equality via Arc::ptr_eq)
    #[test]
    fn cache_hit_returns_same_arc_instance() {
        let factory = RecordStoreFactory::new(
            StorageConfig::default(),
            Arc::new(NullDataStore),
            Vec::new(),
        );

        let store1 = factory.get_or_create("users", 5);
        let store2 = factory.get_or_create("users", 5);

        assert!(
            Arc::ptr_eq(&store1, &store2),
            "same (map_name, partition_id) must return the same Arc"
        );
    }

    /// AC1 negative: different partition IDs return different stores
    #[test]
    fn different_partitions_return_different_stores() {
        let factory = RecordStoreFactory::new(
            StorageConfig::default(),
            Arc::new(NullDataStore),
            Vec::new(),
        );

        let store1 = factory.get_or_create("users", 0);
        let store2 = factory.get_or_create("users", 1);

        assert!(
            !Arc::ptr_eq(&store1, &store2),
            "different partition IDs must return different stores"
        );
    }

    /// AC2: get_all_for_map() returns all stores created for the given map
    #[tokio::test]
    async fn get_all_for_map_returns_all_partitions() {
        let factory = RecordStoreFactory::new(
            StorageConfig::default(),
            Arc::new(NullDataStore),
            Vec::new(),
        );

        // Create stores across multiple partitions for "users"
        let _s0 = factory.get_or_create("users", 0);
        let _s1 = factory.get_or_create("users", 5);
        let _s2 = factory.get_or_create("users", 270);

        // Create a store for a different map
        let _other = factory.get_or_create("orders", 0);

        let user_stores = factory.get_all_for_map("users");
        assert_eq!(user_stores.len(), 3, "should return 3 user stores");

        let order_stores = factory.get_all_for_map("orders");
        assert_eq!(order_stores.len(), 1, "should return 1 order store");

        let empty_stores = factory.get_all_for_map("nonexistent");
        assert!(empty_stores.is_empty(), "nonexistent map returns empty vec");
    }

    /// Observer factories fire only on first get_or_create for a pair
    #[test]
    fn observer_factory_fires_only_on_cache_miss() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct CountingFactory {
            count: AtomicUsize,
        }

        impl ObserverFactory for CountingFactory {
            fn create_observer(
                &self,
                _map_name: &str,
                _partition_id: u32,
            ) -> Option<Arc<dyn MutationObserver>> {
                self.count.fetch_add(1, Ordering::SeqCst);
                None
            }
        }

        let counting = Arc::new(CountingFactory {
            count: AtomicUsize::new(0),
        });

        let factory = RecordStoreFactory::new(
            StorageConfig::default(),
            Arc::new(NullDataStore),
            Vec::new(),
        )
        .with_observer_factories(vec![counting.clone()]);

        // First call: factory fires
        let _s1 = factory.get_or_create("map", 0);
        assert_eq!(counting.count.load(Ordering::SeqCst), 1);

        // Second call same pair: factory does NOT fire (cache hit)
        let _s2 = factory.get_or_create("map", 0);
        assert_eq!(counting.count.load(Ordering::SeqCst), 1);

        // Different pair: factory fires again
        let _s3 = factory.get_or_create("map", 1);
        assert_eq!(counting.count.load(Ordering::SeqCst), 2);
    }
}
