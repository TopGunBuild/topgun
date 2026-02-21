//! Factory for creating fully-wired [`RecordStore`] instances.
//!
//! [`RecordStoreFactory`] is the dependency injection point that creates
//! [`DefaultRecordStore`] instances with all three layers connected:
//! [`HashMapStorage`] (Layer 1), a shared [`MapDataStore`] (Layer 3),
//! and a [`CompositeMutationObserver`] assembled from registered observers.

use std::sync::Arc;

use crate::storage::engines::HashMapStorage;
use crate::storage::impls::{DefaultRecordStore, StorageConfig};
use crate::storage::map_data_store::MapDataStore;
use crate::storage::mutation_observer::{CompositeMutationObserver, MutationObserver};
use crate::storage::record_store::RecordStore;

/// Factory for creating fully-wired [`RecordStore`] instances.
///
/// Holds shared configuration, the persistence backend, and a list of
/// mutation observers. Each call to [`create()`](RecordStoreFactory::create)
/// produces a new [`DefaultRecordStore`] for a specific `(map_name, partition_id)` pair.
pub struct RecordStoreFactory {
    config: StorageConfig,
    data_store: Arc<dyn MapDataStore>,
    observers: Vec<Arc<dyn MutationObserver>>,
}

impl RecordStoreFactory {
    /// Creates a new factory with the given configuration.
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
        }
    }

    /// Creates a [`RecordStore`] for the given map and partition.
    ///
    /// Assembles a fresh [`HashMapStorage`] engine, clones the shared
    /// [`MapDataStore`] reference, and builds a [`CompositeMutationObserver`]
    /// from the registered observer list.
    #[must_use]
    pub fn create(&self, map_name: &str, partition_id: u32) -> Box<dyn RecordStore> {
        let engine = Box::new(HashMapStorage::new());
        let observer = Arc::new(CompositeMutationObserver::new(
            self.observers.clone(),
        ));
        let record_store = DefaultRecordStore::new(
            map_name.to_string(),
            partition_id,
            engine,
            self.data_store.clone(),
            observer,
            self.config.clone(),
        );
        Box::new(record_store)
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

    /// AC4: RecordStoreFactory::create() returns a working Box<dyn RecordStore>
    #[tokio::test]
    async fn factory_create_returns_working_record_store() {
        let factory = RecordStoreFactory::new(
            StorageConfig::default(),
            Arc::new(NullDataStore),
            Vec::new(),
        );

        let store = factory.create("users", 7);
        assert_eq!(store.name(), "users");
        assert_eq!(store.partition_id(), 7);

        // Put and get round-trip through Box<dyn RecordStore>
        store
            .put("alice", make_value("data"), ExpiryPolicy::NONE, CallerProvenance::Client)
            .await
            .unwrap();

        let record = store.get("alice", false).await.unwrap();
        assert!(record.is_some());
        assert_eq!(store.size(), 1);
    }

    /// AC5: StorageConfig::default() has all fields set to 0
    #[test]
    fn storage_config_default_all_zeros() {
        let config = StorageConfig::default();
        assert_eq!(config.default_ttl_millis, 0);
        assert_eq!(config.default_max_idle_millis, 0);
        assert_eq!(config.max_entry_count, 0);
    }

    /// AC6: DefaultRecordStore compiles as Box<dyn RecordStore> via factory
    #[test]
    fn factory_output_is_object_safe() {
        let factory = RecordStoreFactory::new(
            StorageConfig::default(),
            Arc::new(NullDataStore),
            Vec::new(),
        );
        let _store: Box<dyn RecordStore> = factory.create("map", 0);
    }

    /// Factory creates independent stores for different map/partition pairs
    #[tokio::test]
    async fn factory_creates_independent_stores() {
        let factory = RecordStoreFactory::new(
            StorageConfig::default(),
            Arc::new(NullDataStore),
            Vec::new(),
        );

        let store_a = factory.create("map-a", 0);
        let store_b = factory.create("map-b", 1);

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

        let store = factory.create("events", 3);
        assert!(store.is_expirable());
    }
}
