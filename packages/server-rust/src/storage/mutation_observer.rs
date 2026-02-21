//! Mutation observer trait and composite implementation.
//!
//! Defines [`MutationObserver`] for reacting to record mutations within a
//! [`RecordStore`](super::RecordStore), and [`CompositeMutationObserver`]
//! which fans out notifications to multiple observers.

use std::sync::Arc;

use super::record::{Record, RecordValue};

/// Observer for record mutations within a `RecordStore`.
///
/// Implementations can track statistics, maintain indexes, broadcast
/// change events, or perform other side effects in response to data changes.
///
/// Used as `Arc<dyn MutationObserver>`.
pub trait MutationObserver: Send + Sync {
    /// Called after a new record is inserted.
    fn on_put(
        &self,
        key: &str,
        record: &Record,
        old_value: Option<&RecordValue>,
        is_backup: bool,
    );

    /// Called after an existing record is updated in place.
    fn on_update(
        &self,
        key: &str,
        record: &Record,
        old_value: &RecordValue,
        new_value: &RecordValue,
        is_backup: bool,
    );

    /// Called after a record is removed.
    fn on_remove(&self, key: &str, record: &Record, is_backup: bool);

    /// Called after a record is evicted (e.g., due to memory pressure).
    fn on_evict(&self, key: &str, record: &Record, is_backup: bool);

    /// Called after a record is loaded from the backing `MapDataStore`.
    fn on_load(&self, key: &str, record: &Record, is_backup: bool);

    /// Called when a record is replicated from another node.
    fn on_replication_put(&self, key: &str, record: &Record, populate_index: bool);

    /// Called when all entries are cleared.
    fn on_clear(&self);

    /// Called when the record store is reset to initial state.
    fn on_reset(&self);

    /// Called when the record store is destroyed.
    fn on_destroy(&self, is_shutdown: bool);
}

/// Composite observer that fans out to multiple observers.
///
/// Iterates all registered observers for each notification method,
/// enabling multiple independent reactions to a single mutation event.
#[derive(Default)]
pub struct CompositeMutationObserver {
    observers: Vec<Arc<dyn MutationObserver>>,
}

impl CompositeMutationObserver {
    /// Creates a composite observer with the given list of observers.
    #[must_use]
    pub fn new(observers: Vec<Arc<dyn MutationObserver>>) -> Self {
        Self { observers }
    }

    /// Adds an observer after construction.
    pub fn add(&mut self, observer: Arc<dyn MutationObserver>) {
        self.observers.push(observer);
    }
}

impl MutationObserver for CompositeMutationObserver {
    fn on_put(
        &self,
        key: &str,
        record: &Record,
        old_value: Option<&RecordValue>,
        is_backup: bool,
    ) {
        for observer in &self.observers {
            observer.on_put(key, record, old_value, is_backup);
        }
    }

    fn on_update(
        &self,
        key: &str,
        record: &Record,
        old_value: &RecordValue,
        new_value: &RecordValue,
        is_backup: bool,
    ) {
        for observer in &self.observers {
            observer.on_update(key, record, old_value, new_value, is_backup);
        }
    }

    fn on_remove(&self, key: &str, record: &Record, is_backup: bool) {
        for observer in &self.observers {
            observer.on_remove(key, record, is_backup);
        }
    }

    fn on_evict(&self, key: &str, record: &Record, is_backup: bool) {
        for observer in &self.observers {
            observer.on_evict(key, record, is_backup);
        }
    }

    fn on_load(&self, key: &str, record: &Record, is_backup: bool) {
        for observer in &self.observers {
            observer.on_load(key, record, is_backup);
        }
    }

    fn on_replication_put(&self, key: &str, record: &Record, populate_index: bool) {
        for observer in &self.observers {
            observer.on_replication_put(key, record, populate_index);
        }
    }

    fn on_clear(&self) {
        for observer in &self.observers {
            observer.on_clear();
        }
    }

    fn on_reset(&self) {
        for observer in &self.observers {
            observer.on_reset();
        }
    }

    fn on_destroy(&self, is_shutdown: bool) {
        for observer in &self.observers {
            observer.on_destroy(is_shutdown);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use topgun_core::hlc::Timestamp;
    use topgun_core::types::Value;

    use super::*;
    use crate::storage::record::RecordMetadata;

    /// Test observer that counts how many times each method is called.
    #[allow(clippy::struct_field_names)]
    struct CountingObserver {
        put_count: AtomicUsize,
        update_count: AtomicUsize,
        remove_count: AtomicUsize,
        evict_count: AtomicUsize,
        load_count: AtomicUsize,
        replication_put_count: AtomicUsize,
        clear_count: AtomicUsize,
        reset_count: AtomicUsize,
        destroy_count: AtomicUsize,
    }

    impl CountingObserver {
        fn new() -> Self {
            Self {
                put_count: AtomicUsize::new(0),
                update_count: AtomicUsize::new(0),
                remove_count: AtomicUsize::new(0),
                evict_count: AtomicUsize::new(0),
                load_count: AtomicUsize::new(0),
                replication_put_count: AtomicUsize::new(0),
                clear_count: AtomicUsize::new(0),
                reset_count: AtomicUsize::new(0),
                destroy_count: AtomicUsize::new(0),
            }
        }
    }

    impl MutationObserver for CountingObserver {
        fn on_put(&self, _: &str, _: &Record, _: Option<&RecordValue>, _: bool) {
            self.put_count.fetch_add(1, Ordering::Relaxed);
        }
        fn on_update(&self, _: &str, _: &Record, _: &RecordValue, _: &RecordValue, _: bool) {
            self.update_count.fetch_add(1, Ordering::Relaxed);
        }
        fn on_remove(&self, _: &str, _: &Record, _: bool) {
            self.remove_count.fetch_add(1, Ordering::Relaxed);
        }
        fn on_evict(&self, _: &str, _: &Record, _: bool) {
            self.evict_count.fetch_add(1, Ordering::Relaxed);
        }
        fn on_load(&self, _: &str, _: &Record, _: bool) {
            self.load_count.fetch_add(1, Ordering::Relaxed);
        }
        fn on_replication_put(&self, _: &str, _: &Record, _: bool) {
            self.replication_put_count.fetch_add(1, Ordering::Relaxed);
        }
        fn on_clear(&self) {
            self.clear_count.fetch_add(1, Ordering::Relaxed);
        }
        fn on_reset(&self) {
            self.reset_count.fetch_add(1, Ordering::Relaxed);
        }
        fn on_destroy(&self, _: bool) {
            self.destroy_count.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn make_test_record() -> Record {
        Record {
            value: RecordValue::Lww {
                value: Value::String("test".to_string()),
                timestamp: Timestamp {
                    millis: 1_000_000,
                    counter: 0,
                    node_id: "node-1".to_string(),
                },
            },
            metadata: RecordMetadata::new(1_000_000, 64),
        }
    }

    fn make_test_value() -> RecordValue {
        RecordValue::Lww {
            value: Value::Int(42),
            timestamp: Timestamp {
                millis: 1_000_001,
                counter: 0,
                node_id: "node-1".to_string(),
            },
        }
    }

    #[test]
    fn empty_composite_does_not_panic() {
        let composite = CompositeMutationObserver::default();
        let record = make_test_record();
        let value = make_test_value();

        // All of these should succeed without panic on an empty observer list.
        composite.on_put("key", &record, None, false);
        composite.on_put("key", &record, Some(&value), true);
        composite.on_update("key", &record, &value, &value, false);
        composite.on_remove("key", &record, false);
        composite.on_evict("key", &record, false);
        composite.on_load("key", &record, false);
        composite.on_replication_put("key", &record, true);
        composite.on_clear();
        composite.on_reset();
        composite.on_destroy(false);
        composite.on_destroy(true);
    }

    #[test]
    fn single_observer_receives_all_notifications() {
        let observer = Arc::new(CountingObserver::new());
        let dyn_observer: Arc<dyn MutationObserver> = Arc::clone(&observer) as _;
        let composite = CompositeMutationObserver::new(vec![dyn_observer]);
        let record = make_test_record();
        let value = make_test_value();

        composite.on_put("k", &record, None, false);
        composite.on_update("k", &record, &value, &value, false);
        composite.on_remove("k", &record, false);
        composite.on_evict("k", &record, false);
        composite.on_load("k", &record, false);
        composite.on_replication_put("k", &record, true);
        composite.on_clear();
        composite.on_reset();
        composite.on_destroy(false);

        assert_eq!(observer.put_count.load(Ordering::Relaxed), 1);
        assert_eq!(observer.update_count.load(Ordering::Relaxed), 1);
        assert_eq!(observer.remove_count.load(Ordering::Relaxed), 1);
        assert_eq!(observer.evict_count.load(Ordering::Relaxed), 1);
        assert_eq!(observer.load_count.load(Ordering::Relaxed), 1);
        assert_eq!(observer.replication_put_count.load(Ordering::Relaxed), 1);
        assert_eq!(observer.clear_count.load(Ordering::Relaxed), 1);
        assert_eq!(observer.reset_count.load(Ordering::Relaxed), 1);
        assert_eq!(observer.destroy_count.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn multiple_observers_all_receive_notifications() {
        let obs1 = Arc::new(CountingObserver::new());
        let obs2 = Arc::new(CountingObserver::new());
        let obs3 = Arc::new(CountingObserver::new());

        let composite = CompositeMutationObserver::new(vec![
            Arc::clone(&obs1) as Arc<dyn MutationObserver>,
            Arc::clone(&obs2) as Arc<dyn MutationObserver>,
            Arc::clone(&obs3) as Arc<dyn MutationObserver>,
        ]);

        let record = make_test_record();

        composite.on_put("k", &record, None, false);
        composite.on_put("k", &record, None, false);
        composite.on_clear();

        assert_eq!(obs1.put_count.load(Ordering::Relaxed), 2);
        assert_eq!(obs2.put_count.load(Ordering::Relaxed), 2);
        assert_eq!(obs3.put_count.load(Ordering::Relaxed), 2);
        assert_eq!(obs1.clear_count.load(Ordering::Relaxed), 1);
        assert_eq!(obs2.clear_count.load(Ordering::Relaxed), 1);
        assert_eq!(obs3.clear_count.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn add_observer_after_construction() {
        let mut composite = CompositeMutationObserver::default();
        let observer = Arc::new(CountingObserver::new());
        let record = make_test_record();

        // Call before adding -- no observers to notify.
        composite.on_put("k", &record, None, false);
        assert_eq!(observer.put_count.load(Ordering::Relaxed), 0);

        // Add observer and call again.
        composite.add(Arc::clone(&observer) as Arc<dyn MutationObserver>);
        composite.on_put("k", &record, None, false);
        assert_eq!(observer.put_count.load(Ordering::Relaxed), 1);
    }

    // --- RecordMetadata unit tests ---

    #[test]
    fn record_metadata_new_sets_fields_correctly() {
        let now = 1_700_000_000_000_i64;
        let cost = 256_u64;
        let meta = RecordMetadata::new(now, cost);

        assert_eq!(meta.version, 1);
        assert_eq!(meta.creation_time, now);
        assert_eq!(meta.last_access_time, now);
        assert_eq!(meta.last_update_time, now);
        assert_eq!(meta.last_stored_time, 0);
        assert_eq!(meta.hits, 0);
        assert_eq!(meta.cost, cost);
    }

    #[test]
    fn record_metadata_on_access_increments_hits_and_updates_time() {
        let mut meta = RecordMetadata::new(1000, 64);
        assert_eq!(meta.hits, 0);
        assert_eq!(meta.last_access_time, 1000);

        meta.on_access(2000);
        assert_eq!(meta.hits, 1);
        assert_eq!(meta.last_access_time, 2000);

        meta.on_access(3000);
        assert_eq!(meta.hits, 2);
        assert_eq!(meta.last_access_time, 3000);
    }

    #[test]
    fn record_metadata_on_update_increments_version_and_updates_time() {
        let mut meta = RecordMetadata::new(1000, 64);
        assert_eq!(meta.version, 1);
        assert_eq!(meta.last_update_time, 1000);

        meta.on_update(2000);
        assert_eq!(meta.version, 2);
        assert_eq!(meta.last_update_time, 2000);

        meta.on_update(3000);
        assert_eq!(meta.version, 3);
        assert_eq!(meta.last_update_time, 3000);
    }

    #[test]
    fn record_metadata_is_dirty_tracks_store_vs_update() {
        let mut meta = RecordMetadata::new(1000, 64);

        // Newly created: last_update_time=1000, last_stored_time=0 -> dirty
        assert!(meta.is_dirty());

        // After storing: last_stored_time catches up
        meta.on_store(1000);
        assert!(!meta.is_dirty());

        // After updating: dirty again
        meta.on_update(2000);
        assert!(meta.is_dirty());

        // After storing again: clean
        meta.on_store(2000);
        assert!(!meta.is_dirty());
    }

    #[test]
    fn record_metadata_default_is_all_zeros() {
        let meta = RecordMetadata::default();
        assert_eq!(meta.version, 0);
        assert_eq!(meta.creation_time, 0);
        assert_eq!(meta.last_access_time, 0);
        assert_eq!(meta.last_update_time, 0);
        assert_eq!(meta.last_stored_time, 0);
        assert_eq!(meta.hits, 0);
        assert_eq!(meta.cost, 0);
    }

    #[test]
    fn record_metadata_on_access_saturates_at_u32_max() {
        let mut meta = RecordMetadata::new(1000, 64);
        meta.hits = u32::MAX;
        meta.on_access(2000);
        assert_eq!(meta.hits, u32::MAX);
    }

    #[test]
    fn record_metadata_on_update_saturates_at_u32_max() {
        let mut meta = RecordMetadata::new(1000, 64);
        meta.version = u32::MAX;
        meta.on_update(2000);
        assert_eq!(meta.version, u32::MAX);
    }

    // --- Object-safety compile tests ---

    /// Verifies `Arc<dyn StorageEngine>` compiles (object safety).
    #[test]
    fn storage_engine_is_object_safe() {
        fn _assert_object_safe(_: &Arc<dyn crate::storage::StorageEngine>) {}
    }

    /// Verifies `Arc<dyn MapDataStore>` compiles (object safety).
    #[test]
    fn map_data_store_is_object_safe() {
        fn _assert_object_safe(_: &Arc<dyn crate::storage::MapDataStore>) {}
    }

    /// Verifies `Arc<dyn MutationObserver>` compiles (object safety).
    #[test]
    fn mutation_observer_is_object_safe() {
        fn _assert_object_safe(_: &Arc<dyn MutationObserver>) {}
    }

    /// Verifies `Box<dyn RecordStore>` compiles (object safety).
    #[test]
    #[allow(clippy::borrowed_box)]
    fn record_store_is_object_safe() {
        fn _assert_object_safe(_: &Box<dyn crate::storage::RecordStore>) {}
    }
}
