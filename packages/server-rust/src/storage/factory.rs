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

use topgun_core::hash_to_partition;

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

impl std::fmt::Debug for RecordStoreFactory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecordStoreFactory")
            .field("cached_stores", &self.store_cache.len())
            .field("observer_count", &self.observers.len())
            .field("factory_count", &self.observer_factories.len())
            .finish_non_exhaustive()
    }
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
    pub fn with_observer_factories(mut self, factories: Vec<Arc<dyn ObserverFactory>>) -> Self {
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

    /// Stream every durable record of `map_name` from the datastore and make
    /// any persisted-but-non-resident record resident, in bounded batches.
    ///
    /// This is the datastore-aware async scan entrypoint for the full-scan QUERY
    /// path. After a restart (no rehydration) or after eviction (records dropped
    /// from the in-memory engine under `TOPGUN_MAX_RAM_MB` pressure), durable
    /// records exist on disk but not in memory, so the synchronous
    /// `for_each_boxed` scan (and the DAG scan) would silently omit them. This
    /// method generalizes the single-key lazy-load in `RecordStore::get` to the
    /// whole map: it pages the datastore via `scan_values` / `scan_values_batched`
    /// (byte-bounded by `max_batch_cost`, so a single batch never exceeds the RAM
    /// ceiling) and hydrates each record into its partition store via
    /// `hydrate_loaded`, which never clobbers a resident (possibly fresher) value.
    ///
    /// Key-ordered datastore paging (redb ordered `table.range`, postgres keyset
    /// `key > last_key ORDER BY key`) guarantees a stably-present key is never
    /// missed or duplicated across batch boundaries regardless of snapshot
    /// strategy — the load-bearing invariant against silent divergence from
    /// durable state.
    ///
    /// A no-op for null / in-memory datastores (nothing durable to surface).
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying datastore scan fails (e.g. a backend
    /// I/O error or a malformed resume cursor).
    pub async fn hydrate_non_resident_for_scan(&self, map_name: &str) -> anyhow::Result<()> {
        if self.data_store.is_null() {
            return Ok(());
        }

        // `0` requests the backend's default per-batch byte budget, a
        // conservative fraction of TOPGUN_MAX_RAM_MB. Each scan_* call observes
        // one per-batch snapshot; we do not hold a cross-batch session.
        let batch_cost: u64 = 0;
        let mut batch = self
            .data_store
            .scan_values(map_name, false, batch_cost)
            .await?;
        loop {
            for (key, value) in batch.records {
                let partition_id = hash_to_partition(&key);
                let store = self.get_or_create(map_name, partition_id);
                store.hydrate_loaded(&key, value);
            }
            let Some(cursor) = batch.next_cursor else {
                break;
            };
            batch = self
                .data_store
                .scan_values_batched(map_name, false, cursor, batch_cost)
                .await?;
        }
        Ok(())
    }

    /// Returns a reference to the shared persistence backend.
    ///
    /// Exposes the underlying [`MapDataStore`] so callers (e.g. the query scan
    /// path) can drive datastore-backed streaming scans without going through the
    /// per-partition hydration loop.
    #[must_use]
    pub fn data_store(&self) -> Arc<dyn MapDataStore> {
        self.data_store.clone()
    }

    /// Returns a snapshot of all live stores across all maps and partitions.
    ///
    /// Allocates a Vec + clones every Arc per call — O(N stores). Negligible at
    /// 1s eviction interval; becomes the hot path if `interval_ms` < ~50. A
    /// `for_each_store` callback variant would eliminate the alloc if profiling
    /// targets this.
    #[must_use]
    pub fn all_stores(&self) -> Vec<Arc<dyn RecordStore>> {
        self.store_cache
            .iter()
            .map(|entry| Arc::clone(entry.value()))
            .collect()
    }
}

#[cfg(test)]
#[allow(clippy::doc_markdown)]
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
            .put(
                "alice",
                make_value("data"),
                ExpiryPolicy::NONE,
                CallerProvenance::Client,
            )
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
            .put(
                "key1",
                make_value("a"),
                ExpiryPolicy::NONE,
                CallerProvenance::Client,
            )
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

    // --- Datastore-backed full-scan (non-resident read-availability) ---

    use crate::storage::datastores::RedbDataStore;

    /// Collect every resident key visible to the full-scan path, exactly as
    /// `handle_query_subscribe` does: iterate every partition store for the map
    /// and call `for_each_boxed` (the synchronous in-memory scan).
    fn scan_resident_keys(factory: &RecordStoreFactory, map: &str) -> Vec<String> {
        let mut keys = Vec::new();
        for store in factory.get_all_for_map(map) {
            store.for_each_boxed(&mut |k, _| keys.push(k.to_string()), false);
        }
        keys.sort();
        keys.dedup();
        keys
    }

    /// Build a factory over a fresh redb file, returning the shared data store so
    /// the test can write durable records "out of band" (simulating data that is
    /// durable on disk but not resident in any in-memory engine).
    fn redb_factory() -> (RecordStoreFactory, Arc<RedbDataStore>, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("scan.redb");
        let store = Arc::new(RedbDataStore::new(&path).expect("redb open"));
        let factory = RecordStoreFactory::new(StorageConfig::default(), store.clone(), Vec::new());
        (factory, store, dir)
    }

    /// AC-RESTART-QUERY + AC7b(i): after a restart (fresh factory, empty store
    /// cache) a full-scan over a durable-but-non-resident map surfaces ALL durable
    /// keys — not an empty map. The records exist only on disk; hydration must
    /// make them resident.
    #[tokio::test]
    async fn hydrate_surfaces_durable_non_resident_keys() {
        let (factory, data_store, _dir) = redb_factory();
        for i in 0..30 {
            data_store
                .add("m", &format!("k{i:03}"), &make_value("v"), 0, 1000)
                .await
                .unwrap();
        }

        // Pre-hydration: nothing is resident, the in-memory-only scan is empty.
        assert!(
            scan_resident_keys(&factory, "m").is_empty(),
            "fresh factory has no resident records (the pre-fix defect)"
        );

        factory.hydrate_non_resident_for_scan("m").await.unwrap();

        let keys = scan_resident_keys(&factory, "m");
        assert_eq!(
            keys.len(),
            30,
            "every durable key is surfaced post-hydration"
        );
        assert_eq!(keys.first().map(String::as_str), Some("k000"));
        assert_eq!(keys.last().map(String::as_str), Some("k029"));
    }

    /// AC7b(i) negative control: WITHOUT hydration (the pre-fix in-memory-only
    /// scan) a stably-present durable-but-non-resident key is dropped. Proves the
    /// datastore-backed scan is load-bearing.
    #[tokio::test]
    async fn pre_fix_in_memory_only_scan_drops_non_resident_key() {
        let (factory, data_store, _dir) = redb_factory();
        data_store
            .add("m", "stable", &make_value("v"), 0, 1000)
            .await
            .unwrap();

        // No hydration call — this is the pre-fix behavior.
        let keys = scan_resident_keys(&factory, "m");
        assert!(
            keys.is_empty(),
            "pre-fix scan misses the durable non-resident key"
        );
    }

    /// AC3 (EVICTION, no restart): after a record is dropped from the in-memory
    /// engine (the eviction lever), a subsequent full-scan re-hydrates it from the
    /// datastore and remains COMPLETE.
    #[tokio::test]
    async fn evicted_record_reappears_after_rehydration() {
        let (factory, data_store, _dir) = redb_factory();
        for k in ["a", "b", "c"] {
            data_store
                .add("m", k, &make_value("v"), 0, 1000)
                .await
                .unwrap();
        }
        factory.hydrate_non_resident_for_scan("m").await.unwrap();
        assert_eq!(scan_resident_keys(&factory, "m").len(), 3);

        // Eviction lever: drop one key from the in-memory engine of its partition.
        let victim = "b";
        let pid = hash_to_partition(victim);
        let store = factory.get_or_create("m", pid);
        let evicted = store.evict(victim, false);
        assert!(evicted.is_some(), "victim was resident before eviction");
        assert!(
            !scan_resident_keys(&factory, "m").contains(&victim.to_string()),
            "evicted key is gone from the in-memory scan (the latent defect)"
        );

        // Re-scan: hydration restores the evicted record from durable storage.
        factory.hydrate_non_resident_for_scan("m").await.unwrap();
        let keys = scan_resident_keys(&factory, "m");
        assert_eq!(keys.len(), 3, "evicted record is re-surfaced");
        assert!(keys.contains(&victim.to_string()));
    }

    /// hydrate_loaded must NOT clobber a resident (possibly fresher) value: a
    /// concurrent/newer in-memory write survives a subsequent hydration of the
    /// same key from a staler durable copy.
    #[tokio::test]
    async fn hydration_never_clobbers_resident_value() {
        let (factory, data_store, _dir) = redb_factory();
        // Durable (stale) copy on disk.
        data_store
            .add("m", "k", &make_value("stale"), 0, 1000)
            .await
            .unwrap();
        // Fresher resident write lands in the in-memory engine first.
        let pid = hash_to_partition("k");
        let store = factory.get_or_create("m", pid);
        store
            .put(
                "k",
                make_value("fresh"),
                ExpiryPolicy::NONE,
                CallerProvenance::Client,
            )
            .await
            .unwrap();

        factory.hydrate_non_resident_for_scan("m").await.unwrap();

        let rec = store.get("k", false).await.unwrap().expect("present");
        match rec.value {
            RecordValue::Lww {
                value: Value::String(s),
                ..
            } => assert_eq!(s, "fresh", "resident value must survive hydration"),
            other => panic!("unexpected value: {other:?}"),
        }
    }

    /// AC7b(i) belt-and-suspenders: a stably-present key is never skipped when a
    /// concurrent write inserts a DIFFERENT key mid-scan at an ordering position
    /// that straddles the active cursor. Exercises L4(a)'s "regardless of snapshot
    /// strategy" claim non-vacuously across a batch boundary (key-ordered paging).
    #[tokio::test]
    async fn concurrent_write_mid_scan_does_not_skip_stable_key() {
        let (_factory, data_store, _dir) = redb_factory();
        // Seed keys k000..k019. "k010" is the stably-present key under observation.
        for i in 0..20 {
            data_store
                .add("m", &format!("k{i:03}"), &make_value("v"), 0, 1000)
                .await
                .unwrap();
        }

        // Drive a multi-batch scan with a tiny budget so batch boundaries fall
        // between keys; insert a NEW key behind the cursor mid-scan.
        let mut seen: Vec<String> = Vec::new();
        let mut inserted_mid_scan = false;
        let mut batch = data_store.scan_values("m", false, 1).await.unwrap();
        loop {
            for (k, _) in &batch.records {
                seen.push(k.clone());
                // After we pass k005 (cursor has advanced past it), insert a key
                // BELOW the cursor ("k000z" sorts after k000, before k001) — a key
                // the cursor already moved past. Key-ordered paging must not let
                // this disturb the still-pending stable key k010.
                if k == "k005" && !inserted_mid_scan {
                    data_store
                        .add("m", "k000z", &make_value("late"), 0, 2000)
                        .await
                        .unwrap();
                    inserted_mid_scan = true;
                }
            }
            match batch.next_cursor.take() {
                Some(cursor) => {
                    batch = data_store
                        .scan_values_batched("m", false, cursor, 1)
                        .await
                        .unwrap();
                }
                None => break,
            }
        }

        assert!(
            seen.contains(&"k010".to_string()),
            "the stably-present key past the cursor is never skipped by a concurrent insert behind the cursor"
        );
        // All originally-seeded keys must still appear exactly once.
        let mut originals: Vec<String> = seen
            .iter()
            .filter(|k| k.as_str() != "k000z")
            .cloned()
            .collect();
        originals.sort();
        originals.dedup();
        assert_eq!(originals.len(), 20, "no original key missed or duplicated");
    }
}
