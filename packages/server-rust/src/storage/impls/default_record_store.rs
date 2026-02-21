//! Per-map-per-partition record store that orchestrates all three storage layers.
//!
//! [`DefaultRecordStore`] is the Layer 2 component in the storage hierarchy.
//! It coordinates the in-memory [`StorageEngine`](crate::storage::StorageEngine)
//! (Layer 1) with the [`MapDataStore`](crate::storage::MapDataStore) (Layer 3),
//! managing metadata, expiry, eviction, and mutation observation.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;

use crate::storage::engine::{FetchResult, IterationCursor, StorageEngine};
use crate::storage::map_data_store::MapDataStore;
use crate::storage::mutation_observer::{CompositeMutationObserver, MutationObserver};
use crate::storage::record::{Record, RecordMetadata, RecordValue};
use crate::storage::record_store::{CallerProvenance, ExpiryPolicy, ExpiryReason, RecordStore};

/// Returns the current wall-clock time as milliseconds since the Unix epoch.
///
/// Millisecond timestamps fit comfortably in i64 until the year 292 million.
#[allow(clippy::cast_possible_truncation)]
fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

/// Configuration for storage behavior, applied per-RecordStore.
///
/// Controls default TTL, max-idle, and eviction thresholds. Imported from
/// [`factory`](crate::storage::factory) when wiring via `RecordStoreFactory`,
/// or constructed directly for tests.
#[derive(Debug, Clone, Default)]
pub struct StorageConfig {
    /// Default TTL in milliseconds for new records. 0 = no TTL.
    pub default_ttl_millis: u64,
    /// Default max idle time in milliseconds. 0 = no max idle.
    pub default_max_idle_millis: u64,
    /// Maximum number of entries before eviction triggers. 0 = unlimited.
    pub max_entry_count: u64,
}

/// Per-map-per-partition record store that orchestrates all three storage layers.
///
/// Coordinates:
/// - Layer 1 ([`StorageEngine`]): in-memory key-value storage
/// - Layer 3 ([`MapDataStore`]): external persistence backend
/// - [`CompositeMutationObserver`]: mutation notification fan-out
///
/// Provides metadata tracking, TTL/max-idle expiry checks, eviction support,
/// and write-through persistence based on caller provenance.
pub struct DefaultRecordStore {
    name: String,
    partition_id: u32,
    engine: Box<dyn StorageEngine>,
    data_store: Arc<dyn MapDataStore>,
    observer: Arc<CompositeMutationObserver>,
    config: StorageConfig,
}

impl DefaultRecordStore {
    /// Creates a new `DefaultRecordStore` with the given dependencies.
    #[must_use]
    pub fn new(
        name: String,
        partition_id: u32,
        engine: Box<dyn StorageEngine>,
        data_store: Arc<dyn MapDataStore>,
        observer: Arc<CompositeMutationObserver>,
        config: StorageConfig,
    ) -> Self {
        Self {
            name,
            partition_id,
            engine,
            data_store,
            observer,
            config,
        }
    }

    /// Computes `expiration_time` from the expiry policy and config defaults.
    ///
    /// Returns 0 if no TTL applies (the record does not expire based on absolute time).
    fn compute_expiration_time(&self, expiry: &ExpiryPolicy, creation_time: i64) -> i64 {
        let ttl = if expiry.ttl_millis > 0 {
            expiry.ttl_millis
        } else {
            self.config.default_ttl_millis
        };

        if ttl > 0 {
            // TTL values are always reasonable millisecond durations, not near u64::MAX
            #[allow(clippy::cast_possible_wrap)]
            let ttl_signed = ttl as i64;
            creation_time + ttl_signed
        } else {
            0
        }
    }
}

#[async_trait]
impl RecordStore for DefaultRecordStore {
    fn name(&self) -> &str {
        &self.name
    }

    fn partition_id(&self) -> u32 {
        self.partition_id
    }

    // --- Core CRUD ---

    async fn get(&self, key: &str, touch: bool) -> anyhow::Result<Option<Record>> {
        // Step 1: Check engine
        if let Some(mut record) = self.engine.get(key) {
            if touch {
                let now = now_millis();
                record.metadata.on_access(now);
                self.engine.put(key, record.clone());
            }
            return Ok(Some(record));
        }

        // Step 2: Try loading from data store if non-null
        if !self.data_store.is_null() {
            if let Some(value) = self.data_store.load(&self.name, key).await? {
                let now = now_millis();
                let metadata = RecordMetadata::new(now, 0);
                let record = Record { value, metadata };
                self.engine.put(key, record.clone());
                self.observer.on_load(key, &record, false);
                return Ok(Some(record));
            }
        }

        // Step 3: Not found anywhere
        Ok(None)
    }

    fn exists_in_memory(&self, key: &str) -> bool {
        self.engine.contains_key(key)
    }

    async fn put(
        &self,
        key: &str,
        value: RecordValue,
        expiry: ExpiryPolicy,
        provenance: CallerProvenance,
    ) -> anyhow::Result<Option<RecordValue>> {
        let now = now_millis();

        // Step 1: Check if key already exists
        let old_record = self.engine.get(key);

        // Step 2: Create metadata
        let metadata = RecordMetadata::new(now, 0);

        // Step 3: Create record
        let record = Record {
            value,
            metadata,
        };

        // Step 4: Put into engine
        self.engine.put(key, record.clone());

        // Step 5: Fire observer notifications
        if let Some(ref old) = old_record {
            self.observer
                .on_update(key, &record, &old.value, &record.value, false);
        } else {
            self.observer.on_put(key, &record, None, false);
        }

        // Step 6: Write-through for Client or CrdtMerge provenance
        if matches!(provenance, CallerProvenance::Client | CallerProvenance::CrdtMerge) {
            let expiration_time = self.compute_expiration_time(&expiry, now);
            self.data_store
                .add(&self.name, key, &record.value, expiration_time, now)
                .await?;
        }

        // Step 7: Return old value
        Ok(old_record.map(|r| r.value))
    }

    async fn remove(
        &self,
        key: &str,
        provenance: CallerProvenance,
    ) -> anyhow::Result<Option<RecordValue>> {
        // Step 1: Remove from engine
        let old_record = self.engine.remove(key);

        // Step 2: Fire observer if removed
        if let Some(ref record) = old_record {
            self.observer.on_remove(key, record, false);
        }

        // Step 3: Remove from data store
        let now = now_millis();
        let _ = provenance; // provenance available for future use
        self.data_store.remove(&self.name, key, now).await?;

        // Step 4: Return old value
        Ok(old_record.map(|r| r.value))
    }

    async fn put_backup(
        &self,
        key: &str,
        record: Record,
        provenance: CallerProvenance,
    ) -> anyhow::Result<()> {
        // Step 1: Put into engine
        let old = self.engine.put(key, record.clone());

        // Step 2: Fire observer
        self.observer
            .on_put(key, &record, old.as_ref().map(|r| &r.value), true);

        // Step 3: Write-through for Client or CrdtMerge provenance
        if matches!(provenance, CallerProvenance::Client | CallerProvenance::CrdtMerge) {
            let now = now_millis();
            self.data_store
                .add_backup(&self.name, key, &record.value, 0, now)
                .await?;
        }

        Ok(())
    }

    async fn remove_backup(
        &self,
        key: &str,
        provenance: CallerProvenance,
    ) -> anyhow::Result<()> {
        // Step 1: Remove from engine
        let old = self.engine.remove(key);

        // Step 2: Fire observer if removed
        if let Some(ref record) = old {
            self.observer.on_remove(key, record, true);
        }

        // Step 3: Remove from data store backup
        let now = now_millis();
        let _ = provenance;
        self.data_store.remove_backup(&self.name, key, now).await?;

        Ok(())
    }

    // --- Batch ---

    async fn get_all(&self, keys: &[String]) -> anyhow::Result<Vec<(String, Record)>> {
        let mut results = Vec::with_capacity(keys.len());
        for key in keys {
            if let Some(record) = self.get(key, false).await? {
                results.push((key.clone(), record));
            }
        }
        Ok(results)
    }

    // --- Iteration ---

    fn fetch_keys(&self, cursor: &IterationCursor, size: usize) -> FetchResult<String> {
        self.engine.fetch_keys(cursor, size)
    }

    fn fetch_entries(
        &self,
        cursor: &IterationCursor,
        size: usize,
    ) -> FetchResult<(String, Record)> {
        self.engine.fetch_entries(cursor, size)
    }

    fn for_each_boxed(&self, consumer: &mut dyn FnMut(&str, &Record), is_backup: bool) {
        let now = now_millis();
        let _ = is_backup;
        for (key, record) in self.engine.snapshot_iter() {
            // Skip expired entries
            if self.check_expired(&record, now) == ExpiryReason::NotExpired {
                consumer(&key, &record);
            }
        }
    }

    // --- Size and cost ---

    fn size(&self) -> usize {
        self.engine.len()
    }

    fn is_empty(&self) -> bool {
        self.engine.is_empty()
    }

    fn owned_entry_cost(&self) -> u64 {
        self.engine.estimated_cost()
    }

    // --- Expiry ---

    fn has_expired(&self, key: &str, now: i64, _is_backup: bool) -> ExpiryReason {
        let Some(record) = self.engine.get(key) else {
            return ExpiryReason::NotExpired;
        };

        self.check_expired(&record, now)
    }

    fn evict_expired(&self, percentage: u32, now: i64, _is_backup: bool) {
        let snapshot = self.engine.snapshot_iter();
        let total = snapshot.len();
        if total == 0 {
            return;
        }

        // Calculate how many expired entries to remove (percentage of total)
        let max_removals = ((total as u64 * u64::from(percentage)) / 100) as usize;
        if max_removals == 0 {
            return;
        }

        let mut removed = 0_usize;
        for (key, record) in &snapshot {
            if removed >= max_removals {
                break;
            }
            if self.check_expired(record, now) != ExpiryReason::NotExpired {
                if let Some(removed_record) = self.engine.remove(key) {
                    self.observer.on_evict(key, &removed_record, false);
                    removed += 1;
                }
            }
        }
    }

    fn is_expirable(&self) -> bool {
        self.config.default_ttl_millis > 0 || self.config.default_max_idle_millis > 0
    }

    // --- Eviction ---

    fn evict(&self, key: &str, is_backup: bool) -> Option<RecordValue> {
        // Step 1: Remove from engine
        let old_record = self.engine.remove(key);

        // Step 2: Fire observer if removed
        if let Some(ref record) = old_record {
            self.observer.on_evict(key, record, is_backup);

            // Step 3: Log warning if dirty and data store is non-null
            if record.metadata.is_dirty() && !self.data_store.is_null() {
                tracing::warn!(
                    map = %self.name,
                    key = %key,
                    "Dirty record evicted without being flushed; \
                     will be handled by next flush cycle or shutdown"
                );
            }
        }

        // Step 4: Return old value
        old_record.map(|r| r.value)
    }

    fn evict_all(&self, is_backup: bool) -> u32 {
        let snapshot = self.engine.snapshot_iter();
        let mut count = 0_u32;
        for (key, _) in &snapshot {
            if self.evict(key, is_backup).is_some() {
                count = count.saturating_add(1);
            }
        }
        count
    }

    fn should_evict(&self) -> bool {
        self.config.max_entry_count > 0 && self.engine.len() as u64 >= self.config.max_entry_count
    }

    // --- Lifecycle ---

    fn init(&mut self) {
        // No-op for Phase 3
    }

    fn clear(&self, _is_backup: bool) -> u32 {
        self.observer.on_clear();
        let previous_size = self.engine.len();
        self.engine.clear();
        // Storage sizes are always small enough for u32 in practice
        #[allow(clippy::cast_possible_truncation)]
        let size = previous_size as u32;
        size
    }

    fn reset(&self) {
        self.observer.on_reset();
        self.engine.clear();
    }

    fn destroy(&self) {
        self.observer.on_destroy(false);
        self.engine.destroy();
    }

    // --- MapDataStore integration ---

    async fn soft_flush(&self) -> anyhow::Result<u64> {
        self.data_store.soft_flush().await
    }

    fn storage(&self) -> &dyn StorageEngine {
        &*self.engine
    }

    fn map_data_store(&self) -> &dyn MapDataStore {
        &*self.data_store
    }
}

impl DefaultRecordStore {
    /// Checks whether a record has expired based on store-wide config defaults.
    #[allow(clippy::cast_possible_wrap)]
    fn check_expired(&self, record: &Record, now: i64) -> ExpiryReason {
        // Check TTL: config values are reasonable millisecond durations
        if self.config.default_ttl_millis > 0
            && now - record.metadata.creation_time > self.config.default_ttl_millis as i64
        {
            return ExpiryReason::Ttl;
        }

        // Check max-idle
        if self.config.default_max_idle_millis > 0
            && now - record.metadata.last_access_time
                > self.config.default_max_idle_millis as i64
        {
            return ExpiryReason::MaxIdle;
        }

        ExpiryReason::NotExpired
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use topgun_core::hlc::Timestamp;
    use topgun_core::types::Value;

    use super::*;
    use crate::storage::datastores::NullDataStore;
    use crate::storage::engines::HashMapStorage;
    use crate::storage::mutation_observer::MutationObserver;

    /// Test observer that counts how many times each method is called.
    #[allow(clippy::struct_field_names)]
    struct CountingObserver {
        put_count: AtomicUsize,
        update_count: AtomicUsize,
        remove_count: AtomicUsize,
        evict_count: AtomicUsize,
        load_count: AtomicUsize,
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
        fn on_replication_put(&self, _: &str, _: &Record, _: bool) {}
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

    fn make_store_with_observer(
        observer: Arc<CountingObserver>,
        config: StorageConfig,
    ) -> DefaultRecordStore {
        let engine = Box::new(HashMapStorage::new());
        let data_store: Arc<dyn MapDataStore> = Arc::new(NullDataStore);
        let composite = Arc::new(CompositeMutationObserver::new(vec![
            observer as Arc<dyn MutationObserver>,
        ]));
        DefaultRecordStore::new(
            "test-map".to_string(),
            0,
            engine,
            data_store,
            composite,
            config,
        )
    }

    fn make_store() -> DefaultRecordStore {
        let engine = Box::new(HashMapStorage::new());
        let data_store: Arc<dyn MapDataStore> = Arc::new(NullDataStore);
        let observer = Arc::new(CompositeMutationObserver::default());
        DefaultRecordStore::new(
            "test-map".to_string(),
            42,
            engine,
            data_store,
            observer,
            StorageConfig::default(),
        )
    }

    // --- AC3: Put-then-get round-trip ---

    #[tokio::test]
    async fn put_then_get_round_trip() {
        let store = make_store();
        let value = make_value("hello");

        let old = store
            .put("key1", value.clone(), ExpiryPolicy::NONE, CallerProvenance::Client)
            .await
            .unwrap();
        assert!(old.is_none(), "first put should return None");

        let fetched = store.get("key1", false).await.unwrap();
        assert!(fetched.is_some());
        let record = fetched.unwrap();
        match &record.value {
            RecordValue::Lww { value, .. } => {
                assert_eq!(*value, Value::String("hello".to_string()));
            }
            _ => panic!("expected Lww variant"),
        }
    }

    // --- AC3: Put fires on_put observer ---

    #[tokio::test]
    async fn put_fires_on_put_observer() {
        let observer = Arc::new(CountingObserver::new());
        let store = make_store_with_observer(observer.clone(), StorageConfig::default());

        store
            .put("key1", make_value("v1"), ExpiryPolicy::NONE, CallerProvenance::Client)
            .await
            .unwrap();

        assert_eq!(observer.put_count.load(Ordering::Relaxed), 1);
        assert_eq!(observer.update_count.load(Ordering::Relaxed), 0);
    }

    // --- AC3: Update fires on_update observer ---

    #[tokio::test]
    async fn update_fires_on_update_observer() {
        let observer = Arc::new(CountingObserver::new());
        let store = make_store_with_observer(observer.clone(), StorageConfig::default());

        // First put
        store
            .put("key1", make_value("v1"), ExpiryPolicy::NONE, CallerProvenance::Client)
            .await
            .unwrap();

        // Second put on same key = update
        let old = store
            .put("key1", make_value("v2"), ExpiryPolicy::NONE, CallerProvenance::Client)
            .await
            .unwrap();

        assert!(old.is_some(), "second put should return old value");
        assert_eq!(observer.put_count.load(Ordering::Relaxed), 1);
        assert_eq!(observer.update_count.load(Ordering::Relaxed), 1);
    }

    // --- AC3: Remove fires on_remove observer ---

    #[tokio::test]
    async fn remove_fires_on_remove_observer() {
        let observer = Arc::new(CountingObserver::new());
        let store = make_store_with_observer(observer.clone(), StorageConfig::default());

        store
            .put("key1", make_value("v1"), ExpiryPolicy::NONE, CallerProvenance::Client)
            .await
            .unwrap();

        let old = store.remove("key1", CallerProvenance::Client).await.unwrap();
        assert!(old.is_some());
        assert_eq!(observer.remove_count.load(Ordering::Relaxed), 1);
    }

    // --- AC3: Expiry returns Ttl when TTL exceeded ---

    #[tokio::test]
    async fn has_expired_returns_ttl() {
        let config = StorageConfig {
            default_ttl_millis: 1000,
            default_max_idle_millis: 0,
            max_entry_count: 0,
        };
        let store = make_store_with_observer(Arc::new(CountingObserver::new()), config);

        store
            .put("key1", make_value("v1"), ExpiryPolicy::NONE, CallerProvenance::Client)
            .await
            .unwrap();

        // Use a now far in the future (creation_time + 2000ms > 1000ms TTL)
        let far_future = now_millis() + 2000;
        let reason = store.has_expired("key1", far_future, false);
        assert_eq!(reason, ExpiryReason::Ttl);
    }

    // --- AC3: Expiry returns MaxIdle when max-idle exceeded ---

    #[tokio::test]
    async fn has_expired_returns_max_idle() {
        let config = StorageConfig {
            default_ttl_millis: 0,
            default_max_idle_millis: 500,
            max_entry_count: 0,
        };
        let store = make_store_with_observer(Arc::new(CountingObserver::new()), config);

        store
            .put("key1", make_value("v1"), ExpiryPolicy::NONE, CallerProvenance::Client)
            .await
            .unwrap();

        // Use a now far in the future (last_access_time + 1000ms > 500ms max-idle)
        let far_future = now_millis() + 1000;
        let reason = store.has_expired("key1", far_future, false);
        assert_eq!(reason, ExpiryReason::MaxIdle);
    }

    // --- AC3: Expiry returns NotExpired for fresh records ---

    #[tokio::test]
    async fn has_expired_returns_not_expired_for_fresh() {
        let config = StorageConfig {
            default_ttl_millis: 60_000,
            default_max_idle_millis: 30_000,
            max_entry_count: 0,
        };
        let store = make_store_with_observer(Arc::new(CountingObserver::new()), config);

        store
            .put("key1", make_value("v1"), ExpiryPolicy::NONE, CallerProvenance::Client)
            .await
            .unwrap();

        // Check immediately -- should not be expired
        let now = now_millis();
        let reason = store.has_expired("key1", now, false);
        assert_eq!(reason, ExpiryReason::NotExpired);
    }

    // --- AC3: size() and owned_entry_cost() ---

    #[tokio::test]
    async fn size_and_owned_entry_cost_reflect_records() {
        let store = make_store();

        assert_eq!(store.size(), 0);
        assert_eq!(store.owned_entry_cost(), 0);

        store
            .put("key1", make_value("v1"), ExpiryPolicy::NONE, CallerProvenance::Client)
            .await
            .unwrap();
        store
            .put("key2", make_value("v2"), ExpiryPolicy::NONE, CallerProvenance::Client)
            .await
            .unwrap();

        assert_eq!(store.size(), 2);
        // Cost is 0 because RecordMetadata::new uses the cost param (which is 0 in put())
        assert_eq!(store.owned_entry_cost(), 0);
    }

    // --- AC3: clear() fires on_clear observer and empties store ---

    #[tokio::test]
    async fn clear_fires_on_clear_and_empties_store() {
        let observer = Arc::new(CountingObserver::new());
        let store = make_store_with_observer(observer.clone(), StorageConfig::default());

        store
            .put("key1", make_value("v1"), ExpiryPolicy::NONE, CallerProvenance::Client)
            .await
            .unwrap();
        store
            .put("key2", make_value("v2"), ExpiryPolicy::NONE, CallerProvenance::Client)
            .await
            .unwrap();

        let count = store.clear(false);
        assert_eq!(count, 2);
        assert!(store.is_empty());
        assert_eq!(observer.clear_count.load(Ordering::Relaxed), 1);
    }

    // --- AC3: get() with touch=true updates access statistics ---

    #[tokio::test]
    async fn get_with_touch_updates_access_stats() {
        let store = make_store();

        store
            .put("key1", make_value("v1"), ExpiryPolicy::NONE, CallerProvenance::Client)
            .await
            .unwrap();

        // Get the initial record to capture creation time
        let initial = store.get("key1", false).await.unwrap().unwrap();
        let initial_access_time = initial.metadata.last_access_time;
        let initial_hits = initial.metadata.hits;

        // Small delay so the access time measurably changes
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;

        // Touch the record
        let touched = store.get("key1", true).await.unwrap().unwrap();
        assert!(
            touched.metadata.last_access_time >= initial_access_time,
            "last_access_time should be updated"
        );
        assert_eq!(
            touched.metadata.hits,
            initial_hits + 1,
            "hits should increment"
        );
    }

    // --- Additional coverage ---

    #[tokio::test]
    async fn exists_in_memory_reflects_state() {
        let store = make_store();

        assert!(!store.exists_in_memory("key1"));
        store
            .put("key1", make_value("v1"), ExpiryPolicy::NONE, CallerProvenance::Client)
            .await
            .unwrap();
        assert!(store.exists_in_memory("key1"));

        store.remove("key1", CallerProvenance::Client).await.unwrap();
        assert!(!store.exists_in_memory("key1"));
    }

    #[test]
    fn name_and_partition_id() {
        let store = make_store();
        assert_eq!(store.name(), "test-map");
        assert_eq!(store.partition_id(), 42);
    }

    #[test]
    fn is_expirable_reflects_config() {
        let store_no_expiry = make_store();
        assert!(!store_no_expiry.is_expirable());

        let config = StorageConfig {
            default_ttl_millis: 1000,
            default_max_idle_millis: 0,
            max_entry_count: 0,
        };
        let store_with_ttl =
            make_store_with_observer(Arc::new(CountingObserver::new()), config);
        assert!(store_with_ttl.is_expirable());
    }

    #[test]
    fn should_evict_reflects_config() {
        let store = make_store();
        assert!(!store.should_evict(), "unlimited config should not trigger eviction");

        let config = StorageConfig {
            default_ttl_millis: 0,
            default_max_idle_millis: 0,
            max_entry_count: 2,
        };
        let store_limited =
            make_store_with_observer(Arc::new(CountingObserver::new()), config);
        assert!(!store_limited.should_evict(), "empty store should not trigger eviction");
    }

    #[tokio::test]
    async fn evict_removes_and_fires_observer() {
        let observer = Arc::new(CountingObserver::new());
        let store = make_store_with_observer(observer.clone(), StorageConfig::default());

        store
            .put("key1", make_value("v1"), ExpiryPolicy::NONE, CallerProvenance::Client)
            .await
            .unwrap();

        let evicted = store.evict("key1", false);
        assert!(evicted.is_some());
        assert_eq!(observer.evict_count.load(Ordering::Relaxed), 1);
        assert!(!store.exists_in_memory("key1"));
    }

    #[tokio::test]
    async fn evict_all_removes_all_entries() {
        let observer = Arc::new(CountingObserver::new());
        let store = make_store_with_observer(observer.clone(), StorageConfig::default());

        store
            .put("a", make_value("v1"), ExpiryPolicy::NONE, CallerProvenance::Client)
            .await
            .unwrap();
        store
            .put("b", make_value("v2"), ExpiryPolicy::NONE, CallerProvenance::Client)
            .await
            .unwrap();
        store
            .put("c", make_value("v3"), ExpiryPolicy::NONE, CallerProvenance::Client)
            .await
            .unwrap();

        let count = store.evict_all(false);
        assert_eq!(count, 3);
        assert!(store.is_empty());
        assert_eq!(observer.evict_count.load(Ordering::Relaxed), 3);
    }

    #[tokio::test]
    async fn reset_fires_on_reset_and_clears() {
        let observer = Arc::new(CountingObserver::new());
        let store = make_store_with_observer(observer.clone(), StorageConfig::default());

        store
            .put("key1", make_value("v1"), ExpiryPolicy::NONE, CallerProvenance::Client)
            .await
            .unwrap();

        store.reset();
        assert!(store.is_empty());
        assert_eq!(observer.reset_count.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn destroy_fires_on_destroy() {
        let observer = Arc::new(CountingObserver::new());
        let store = make_store_with_observer(observer.clone(), StorageConfig::default());

        store
            .put("key1", make_value("v1"), ExpiryPolicy::NONE, CallerProvenance::Client)
            .await
            .unwrap();

        store.destroy();
        assert_eq!(observer.destroy_count.load(Ordering::Relaxed), 1);
    }

    /// Verifies `DefaultRecordStore` compiles as `Box<dyn RecordStore>`.
    #[test]
    fn default_record_store_is_object_safe() {
        let store = make_store();
        let _boxed: Box<dyn RecordStore> = Box::new(store);
    }

    #[tokio::test]
    async fn get_all_returns_matching_entries() {
        let store = make_store();

        store
            .put("a", make_value("va"), ExpiryPolicy::NONE, CallerProvenance::Client)
            .await
            .unwrap();
        store
            .put("b", make_value("vb"), ExpiryPolicy::NONE, CallerProvenance::Client)
            .await
            .unwrap();

        let results = store
            .get_all(&["a".to_string(), "b".to_string(), "missing".to_string()])
            .await
            .unwrap();

        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn has_expired_returns_not_expired_for_missing_key() {
        let store = make_store();
        let reason = store.has_expired("nonexistent", now_millis(), false);
        assert_eq!(reason, ExpiryReason::NotExpired);
    }

    #[tokio::test]
    async fn for_each_boxed_skips_expired() {
        let config = StorageConfig {
            default_ttl_millis: 100,
            default_max_idle_millis: 0,
            max_entry_count: 0,
        };
        let store = make_store_with_observer(Arc::new(CountingObserver::new()), config);

        store
            .put("key1", make_value("v1"), ExpiryPolicy::NONE, CallerProvenance::Client)
            .await
            .unwrap();

        // Wait for TTL to expire
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;

        let mut count = 0_usize;
        store.for_each_boxed(&mut |_key, _record| count += 1, false);
        assert_eq!(count, 0, "expired entries should be skipped");
    }
}
