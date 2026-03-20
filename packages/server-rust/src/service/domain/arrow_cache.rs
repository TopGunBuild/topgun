//! Lazy MsgPack-to-Arrow cache with per-partition mutation invalidation.
//!
//! `ArrowCacheManager` caches Arrow `RecordBatch` instances keyed by
//! `(map_name, partition_id)`. Cache entries are lazily built on first query
//! and invalidated when mutations occur via the `ArrowCacheObserver`.
//!
//! All types in this module are feature-gated behind `#[cfg(feature = "datafusion")]`.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use arrow::array::RecordBatch;
use dashmap::DashMap;

use crate::storage::factory::ObserverFactory;
use crate::storage::mutation_observer::MutationObserver;
use crate::storage::record::{Record, RecordValue};

// ---------------------------------------------------------------------------
// CachedBatch
// ---------------------------------------------------------------------------

/// A cached Arrow `RecordBatch` with its version at the time of caching.
///
/// The `version` is compared against the `ArrowCacheManager`'s version counter
/// to determine if the cached batch is still valid.
pub struct CachedBatch {
    pub batch: RecordBatch,
    pub version: u64,
}

// ---------------------------------------------------------------------------
// ArrowCacheManager
// ---------------------------------------------------------------------------

/// Lazy per-partition Arrow cache with version-based invalidation.
///
/// Each `(map_name, partition_id)` has an independent version counter.
/// When a mutation occurs, the counter is incremented and the cached batch is
/// removed. The next `get_or_build` call rebuilds the batch from the
/// `RecordStore` and stores it with the new version.
pub struct ArrowCacheManager {
    cache: DashMap<(String, u32), CachedBatch>,
    /// Version counters per (map_name, partition_id). Incremented on invalidation.
    versions: DashMap<(String, u32), Arc<AtomicU64>>,
}

impl fmt::Debug for ArrowCacheManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ArrowCacheManager")
            .field("cache_entries", &self.cache.len())
            .field("version_entries", &self.versions.len())
            .finish()
    }
}

impl ArrowCacheManager {
    /// Creates an empty cache with no entries or version counters.
    #[must_use]
    pub fn new() -> Self {
        Self {
            cache: DashMap::new(),
            versions: DashMap::new(),
        }
    }

    /// Returns a cached `RecordBatch` if valid, or builds one via `build_fn`.
    ///
    /// Compares the cached entry's version against the current version counter.
    /// If they match, returns the cached batch. Otherwise, calls `build_fn()`
    /// to produce a fresh `RecordBatch`, stores it, and returns it.
    pub fn get_or_build<F>(
        &self,
        map_name: &str,
        partition_id: u32,
        build_fn: F,
    ) -> Result<RecordBatch, anyhow::Error>
    where
        F: FnOnce() -> Result<RecordBatch, anyhow::Error>,
    {
        let key = (map_name.to_string(), partition_id);
        let current_ver = self.current_version(map_name, partition_id);

        // Fast path: check if cached batch is still valid.
        if let Some(entry) = self.cache.get(&key) {
            if entry.version == current_ver {
                return Ok(entry.batch.clone());
            }
        }

        // Slow path: build a new batch and cache it.
        let batch = build_fn()?;
        self.cache.insert(
            key,
            CachedBatch {
                batch: batch.clone(),
                version: current_ver,
            },
        );
        Ok(batch)
    }

    /// Invalidates the cache for a specific `(map_name, partition_id)`.
    ///
    /// Increments the version counter and removes the cached batch entry,
    /// forcing the next `get_or_build` call to rebuild.
    pub fn invalidate(&self, map_name: &str, partition_id: u32) {
        let key = (map_name.to_string(), partition_id);
        let counter = self
            .versions
            .entry(key.clone())
            .or_insert_with(|| Arc::new(AtomicU64::new(0)));
        counter.fetch_add(1, Ordering::Release);
        self.cache.remove(&key);
    }

    /// Returns the current version counter for a `(map_name, partition_id)`.
    ///
    /// Returns 0 if no version counter exists for the key.
    #[must_use]
    pub fn current_version(&self, map_name: &str, partition_id: u32) -> u64 {
        let key = (map_name.to_string(), partition_id);
        self.versions
            .get(&key)
            .map(|v| v.load(Ordering::Acquire))
            .unwrap_or(0)
    }
}

impl Default for ArrowCacheManager {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// ArrowCacheObserver
// ---------------------------------------------------------------------------

/// Mutation observer that invalidates the Arrow cache on data changes.
///
/// Created by `ArrowCacheObserverFactory` for each `(map_name, partition_id)`.
/// Calls `ArrowCacheManager::invalidate()` on mutation events that change data.
pub struct ArrowCacheObserver {
    cache_manager: Arc<ArrowCacheManager>,
    map_name: String,
    partition_id: u32,
}

impl ArrowCacheObserver {
    /// Creates a new observer for the given map and partition.
    #[must_use]
    pub fn new(
        cache_manager: Arc<ArrowCacheManager>,
        map_name: String,
        partition_id: u32,
    ) -> Self {
        Self {
            cache_manager,
            map_name,
            partition_id,
        }
    }

    /// Invalidates the cache entry for this observer's map and partition.
    fn invalidate(&self) {
        self.cache_manager
            .invalidate(&self.map_name, self.partition_id);
    }
}

impl MutationObserver for ArrowCacheObserver {
    fn on_put(&self, _key: &str, _record: &Record, _old_value: Option<&RecordValue>, _is_backup: bool) {
        self.invalidate();
    }

    fn on_update(
        &self,
        _key: &str,
        _record: &Record,
        _old_value: &RecordValue,
        _new_value: &RecordValue,
        _is_backup: bool,
    ) {
        self.invalidate();
    }

    fn on_remove(&self, _key: &str, _record: &Record, _is_backup: bool) {
        self.invalidate();
    }

    fn on_evict(&self, _key: &str, _record: &Record, _is_backup: bool) {
        // Eviction does not change the logical data set visible to queries.
    }

    fn on_load(&self, _key: &str, _record: &Record, _is_backup: bool) {
        // Load from backing store does not change the logical data set.
    }

    fn on_replication_put(&self, _key: &str, _record: &Record, _populate_index: bool) {
        self.invalidate();
    }

    fn on_clear(&self) {
        self.invalidate();
    }

    fn on_reset(&self) {
        self.invalidate();
    }

    fn on_destroy(&self, _is_shutdown: bool) {
        // No-op: cache will be dropped with the manager.
    }
}

// ---------------------------------------------------------------------------
// ArrowCacheObserverFactory
// ---------------------------------------------------------------------------

/// Factory that creates `ArrowCacheObserver` instances for each record store.
///
/// Registered with `RecordStoreFactory` so that every new `(map_name, partition_id)`
/// store gets an observer that invalidates the Arrow cache on mutations.
pub struct ArrowCacheObserverFactory {
    cache_manager: Arc<ArrowCacheManager>,
}

impl ArrowCacheObserverFactory {
    /// Creates a new factory backed by the given cache manager.
    #[must_use]
    pub fn new(cache_manager: Arc<ArrowCacheManager>) -> Self {
        Self { cache_manager }
    }
}

impl ObserverFactory for ArrowCacheObserverFactory {
    fn create_observer(
        &self,
        map_name: &str,
        partition_id: u32,
    ) -> Option<Arc<dyn MutationObserver>> {
        Some(Arc::new(ArrowCacheObserver::new(
            Arc::clone(&self.cache_manager),
            map_name.to_string(),
            partition_id,
        )))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_cache_manager_is_empty() {
        let mgr = ArrowCacheManager::new();
        assert_eq!(mgr.current_version("test", 0), 0);
    }

    #[test]
    fn get_or_build_caches_batch() {
        let mgr = ArrowCacheManager::new();
        let mut call_count = 0u32;

        // First call should invoke build_fn.
        let _batch = mgr
            .get_or_build("users", 0, || {
                call_count += 1;
                let schema = arrow::datatypes::Schema::new(vec![arrow::datatypes::Field::new(
                    "_key",
                    arrow::datatypes::DataType::Utf8,
                    false,
                )]);
                let col: Arc<dyn arrow::array::Array> =
                    Arc::new(arrow::array::StringArray::from(vec!["k1"]));
                Ok(RecordBatch::try_new(Arc::new(schema), vec![col])?)
            })
            .expect("build should succeed");
        assert_eq!(call_count, 1);

        // Second call should return cached batch without calling build_fn.
        let _batch2 = mgr
            .get_or_build("users", 0, || {
                panic!("build_fn should not be called for cached entry");
            })
            .expect("cached lookup should succeed");
    }

    #[test]
    fn invalidate_increments_version_and_forces_rebuild() {
        let mgr = ArrowCacheManager::new();
        let schema = Arc::new(arrow::datatypes::Schema::new(vec![
            arrow::datatypes::Field::new("_key", arrow::datatypes::DataType::Utf8, false),
        ]));

        let s = schema.clone();
        let _batch = mgr
            .get_or_build("users", 0, move || {
                let col: Arc<dyn arrow::array::Array> =
                    Arc::new(arrow::array::StringArray::from(vec!["k1"]));
                Ok(RecordBatch::try_new(s, vec![col])?)
            })
            .unwrap();

        assert_eq!(mgr.current_version("users", 0), 0);

        mgr.invalidate("users", 0);
        assert_eq!(mgr.current_version("users", 0), 1);

        // Next get_or_build must call build_fn since cache was invalidated.
        let rebuilt = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let rebuilt_clone = Arc::clone(&rebuilt);
        let s2 = schema.clone();
        let _batch2 = mgr
            .get_or_build("users", 0, move || {
                rebuilt_clone.store(true, Ordering::Release);
                let col: Arc<dyn arrow::array::Array> =
                    Arc::new(arrow::array::StringArray::from(vec!["k2"]));
                Ok(RecordBatch::try_new(s2, vec![col])?)
            })
            .unwrap();
        assert!(rebuilt.load(Ordering::Acquire), "build_fn should have been called after invalidation");
    }

    #[test]
    fn invalidate_only_affects_target_partition() {
        let mgr = ArrowCacheManager::new();
        let schema = Arc::new(arrow::datatypes::Schema::new(vec![
            arrow::datatypes::Field::new("_key", arrow::datatypes::DataType::Utf8, false),
        ]));

        // Build for partition 0 and partition 1.
        for pid in 0..2 {
            let s = schema.clone();
            mgr.get_or_build("users", pid, move || {
                let col: Arc<dyn arrow::array::Array> =
                    Arc::new(arrow::array::StringArray::from(vec!["k"]));
                Ok(RecordBatch::try_new(s, vec![col])?)
            })
            .unwrap();
        }

        // Invalidate only partition 0.
        mgr.invalidate("users", 0);
        assert_eq!(mgr.current_version("users", 0), 1);
        assert_eq!(mgr.current_version("users", 1), 0);

        // Partition 1 should still be cached.
        mgr.get_or_build("users", 1, || {
            panic!("partition 1 should still be cached");
        })
        .unwrap();
    }

    #[test]
    fn observer_factory_creates_observers() {
        let mgr = Arc::new(ArrowCacheManager::new());
        let factory = ArrowCacheObserverFactory::new(Arc::clone(&mgr));

        let observer = factory.create_observer("orders", 5);
        assert!(observer.is_some(), "factory should always create an observer");
    }

    #[test]
    fn observer_invalidates_on_mutation_events() {
        let mgr = Arc::new(ArrowCacheManager::new());
        let observer = ArrowCacheObserver::new(Arc::clone(&mgr), "users".to_string(), 0);

        assert_eq!(mgr.current_version("users", 0), 0);

        // Each of these should increment the version.
        let record = Record {
            value: RecordValue::Lww {
                value: topgun_core::types::Value::Null,
                timestamp: topgun_core::hlc::Timestamp {
                    millis: 1000,
                    counter: 0,
                    node_id: "n1".to_string(),
                },
            },
            metadata: crate::storage::record::RecordMetadata::default(),
        };
        let rv = RecordValue::Lww {
            value: topgun_core::types::Value::Int(1),
            timestamp: topgun_core::hlc::Timestamp {
                millis: 1001,
                counter: 0,
                node_id: "n1".to_string(),
            },
        };

        observer.on_put("k1", &record, None, false);
        assert_eq!(mgr.current_version("users", 0), 1);

        observer.on_update("k1", &record, &rv, &rv, false);
        assert_eq!(mgr.current_version("users", 0), 2);

        observer.on_remove("k1", &record, false);
        assert_eq!(mgr.current_version("users", 0), 3);

        observer.on_clear();
        assert_eq!(mgr.current_version("users", 0), 4);

        observer.on_reset();
        assert_eq!(mgr.current_version("users", 0), 5);

        observer.on_replication_put("k1", &record, true);
        assert_eq!(mgr.current_version("users", 0), 6);

        // These should NOT increment the version.
        observer.on_evict("k1", &record, false);
        assert_eq!(mgr.current_version("users", 0), 6);

        observer.on_load("k1", &record, false);
        assert_eq!(mgr.current_version("users", 0), 6);

        observer.on_destroy(false);
        assert_eq!(mgr.current_version("users", 0), 6);
    }
}
