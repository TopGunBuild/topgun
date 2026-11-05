//! Factory for creating fully-wired [`RecordStore`] instances.
//!
//! [`RecordStoreFactory`] is the dependency injection point that creates
//! [`DefaultRecordStore`] instances with all three layers connected:
//! [`HashMapStorage`] (Layer 1), a shared [`MapDataStore`] (Layer 3),
//! and a [`CompositeMutationObserver`] assembled from registered observers.
//!
//! Stores are cached by `(map_name, partition_id)` in a [`DashMap`] so that
//! repeated lookups for the same pair return the same [`Arc<dyn RecordStore>`].

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use dashmap::DashMap;
use topgun_core::hlc::{ClockSource, SystemClock, Timestamp};

use crate::storage::engines::HashMapStorage;
use crate::storage::expiry::retain_live;
use crate::storage::impls::{DefaultRecordStore, StorageConfig};
use crate::storage::map_data_store::MapDataStore;
use crate::storage::mutation_observer::{CompositeMutationObserver, MutationObserver};
use crate::storage::record::RecordValue;
use crate::storage::record_store::RecordStore;

/// Factory that creates per-map mutation observers at store-creation time.
///
/// Enables domain services (e.g., search indexing) to inject observers
/// that are wired with map-specific context when a [`RecordStore`] is created.
pub trait ObserverFactory: Send + Sync {
    /// Creates an observer for the given `(map_name, partition_id)` pair.
    ///
    /// Called once, when the store of that pair is created; the store keeps
    /// what this returns for its lifetime. Returns `None` if this factory does
    /// not need to observe the given map. A factory whose need for a map can
    /// begin after the map's stores exist (an index created at runtime) must
    /// not return `None` for it, but an observer that decides at each event.
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
    /// The clock every expiry decision on a record of this factory reads
    /// (TG-TTL-001), so a test can decide expiry at an exact millisecond.
    clock: Arc<dyn ClockSource>,
    /// Per map, how many values with a TTL were stored since start, so an
    /// expiry sweep can skip a map that never had one. In memory only: a
    /// restart resets it.
    ttl_writes: DashMap<String, u64>,
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
            clock: Arc::new(SystemClock),
            ttl_writes: DashMap::new(),
        }
    }

    /// Builder method that replaces the wall clock expiry decisions read.
    #[must_use]
    pub fn with_clock(mut self, clock: Arc<dyn ClockSource>) -> Self {
        self.clock = clock;
        self
    }

    /// The current time, in milliseconds since the Unix epoch, against which a
    /// record of this factory is judged expired (TG-TTL-001).
    #[must_use]
    pub fn now_ms(&self) -> u64 {
        self.clock.now()
    }

    /// Records that a value with a TTL was stored in `map`.
    pub fn note_ttl_write(&self, map: &str) {
        if let Some(mut count) = self.ttl_writes.get_mut(map) {
            *count += 1;
            return;
        }
        *self.ttl_writes.entry(map.to_string()).or_insert(0) += 1;
    }

    /// How many values with a TTL were stored in `map` since start; 0 when none.
    #[must_use]
    pub fn ttl_write_generation(&self, map: &str) -> u64 {
        self.ttl_writes.get(map).map_or(0, |count| *count)
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
    /// cached store without invoking observer factories again: each factory is
    /// consulted once per store, so a factory whose need for a map can begin
    /// later must return an observer that decides at each event (see
    /// [`ObserverFactory::create_observer`]).
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

    /// Visits every live record of `map_name` once, from the resident record or
    /// the data store, without loading anything into the in-memory engine
    /// (`TG-READ-001`).
    ///
    /// The same merge as [`scan_merged_batches`](Self::scan_merged_batches),
    /// with `consumer` called once per emitted row.
    ///
    /// # Errors
    ///
    /// A data-store error on the first or a resumed batch. Rows already given
    /// to `consumer` stay given; the caller must treat the whole read as
    /// failed: there is no "resident rows only" fallback.
    pub async fn scan_merged(
        &self,
        map_name: &str,
        consumer: &mut (dyn FnMut(&str, &RecordValue) + Send),
    ) -> anyhow::Result<()> {
        self.scan_merged_batches(map_name, |batch| {
            for (key, value) in &batch {
                consumer(key, value);
            }
            std::future::ready(Ok(()))
        })
        .await
    }

    /// The batch-level form of [`scan_merged`](Self::scan_merged)
    /// (`TG-READ-001`): the rows emitted for each data-store batch, and last
    /// the surviving residents, are handed to `on_batch` as one `Vec`, and the
    /// step is awaited before the next data-store batch is requested, so a
    /// step that hands rows to another task applies back-pressure.
    ///
    /// The contract:
    /// - **Resident snapshot first.** Every store of the map is read with
    ///   `for_each_boxed(.., false)` into a keyed snapshot before the data
    ///   store is called.
    /// - **Merge.** For a scanned row whose key is resident, the resident copy
    ///   wins when both are `Lww` and the scanned timestamp is `<=` the
    ///   resident one, or when either side is not `Lww` (a resident OR slot is
    ///   authoritative). Otherwise the scanned row is emitted and the key leaves
    ///   the snapshot. The keys left in the snapshot are emitted after the scan.
    ///   The `<=` rule assumes a key's timestamps rise in write order, which
    ///   holds for writes the server stamps.
    /// - **Each key at most once**, and exactly once if it is resident or
    ///   emitted by the data-store scan, unless it has expired. A key whose last
    ///   write is a delete is neither resident nor emitted by the write-behind
    ///   scan (`TG-WB-008`).
    /// - **Live rows only** (`TG-TTL-001`): the merge decides on the unfiltered
    ///   values, then each row about to be emitted goes through
    ///   [`retain_live`] against `now`, read once per scan from
    ///   [`now_ms`](Self::now_ms). An expired LWW row is not emitted, and an
    ///   expired resident still shadows an older stored row of its key, so the
    ///   stored copy is never served in its place. An OR slot is emitted without
    ///   its expired entries, as `OrMap { records: [], tombstones }` when none
    ///   is left.
    /// - **No engine writes**: no store is created, hydrated or read through
    ///   `get`; every store's `size()` is unchanged.
    /// - **Null store**: with `data_store().is_null()` only the resident
    ///   snapshot is emitted and the data store is not called.
    /// - **Memory**: the resident snapshot, one data-store batch and the batch
    ///   being handed to the step, plus what the step keeps. Each call holds its
    ///   own snapshot.
    /// - No lock guard is held across an `.await`.
    ///
    /// # Errors
    ///
    /// A data-store error on the first or a resumed batch, or the first `Err`
    /// of `on_batch`, which ends the scan.
    pub async fn scan_merged_batches<F, Fut>(
        &self,
        map_name: &str,
        mut on_batch: F,
    ) -> anyhow::Result<()>
    where
        F: FnMut(Vec<(String, RecordValue)>) -> Fut + Send,
        Fut: Future<Output = anyhow::Result<()>> + Send,
    {
        let now = self.now_ms();
        // The resident snapshot is taken before the data store is called, so a
        // write that lands between the two is either in the snapshot or newer
        // than the stored row it races, and the merge below keeps the newer one.
        let mut resident: HashMap<String, (RecordValue, Option<Timestamp>)> = HashMap::new();
        for store in self.get_all_for_map(map_name) {
            store.for_each_boxed(
                &mut |key, record| {
                    let timestamp = match &record.value {
                        RecordValue::Lww { timestamp, .. } => Some(timestamp.clone()),
                        _ => None,
                    };
                    resident.insert(key.to_string(), (record.value.clone(), timestamp));
                },
                false,
            );
        }

        if !self.data_store.is_null() {
            let data_store = self.data_store();
            // 0 = the backend's default batch budget.
            let mut batch = data_store.scan_values(map_name, false, 0).await?;
            loop {
                let mut emitted = Vec::with_capacity(batch.records.len());
                for (key, scanned) in batch.records {
                    let resident_wins = match resident.get(&key) {
                        None => false,
                        Some((_, Some(resident_ts))) => match &scanned {
                            RecordValue::Lww { timestamp, .. } => timestamp <= resident_ts,
                            // A resident LWW copy against a stored non-LWW slot:
                            // the resident copy is the live one.
                            _ => true,
                        },
                        // A resident OR slot is authoritative.
                        Some((_, None)) => true,
                    };
                    if resident_wins {
                        continue;
                    }
                    resident.remove(&key);
                    if let Some(live) = retain_live(scanned, now) {
                        emitted.push((key, live));
                    }
                }
                if !emitted.is_empty() {
                    on_batch(emitted).await?;
                }
                let Some(cursor) = batch.next_cursor else {
                    break;
                };
                batch = data_store
                    .scan_values_batched(map_name, false, cursor, 0)
                    .await?;
            }
        }

        let survivors: Vec<(String, RecordValue)> = resident
            .into_iter()
            .filter_map(|(key, (value, _))| retain_live(value, now).map(|live| (key, live)))
            .collect();
        if !survivors.is_empty() {
            on_batch(survivors).await?;
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

/// A configurable in-memory `MapDataStore` for the whole-map read tests of this
/// crate: real batched scans, plus failures, panics, a gate that parks
/// `scan_values` and call counters.
#[cfg(test)]
pub(crate) mod test_store {
    use std::collections::{BTreeMap, HashMap, HashSet};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    use async_trait::async_trait;
    use parking_lot::Mutex;

    use crate::storage::map_data_store::{LeafSink, MapDataStore, ScanBatch, ScanCursor};
    use crate::storage::record::RecordValue;

    pub(crate) struct TestStore {
        rows: Mutex<BTreeMap<String, BTreeMap<String, RecordValue>>>,
        batch_rows: usize,
        null: bool,
        calls: AtomicU64,
        scans: Mutex<HashMap<String, u64>>,
        fail_first: Mutex<HashSet<String>>,
        fail_resumed: Mutex<HashSet<String>>,
        fail_loads: Mutex<HashSet<(String, String)>>,
        panic_scan: AtomicBool,
        /// `true` while closed: a `scan_values` call fixes its rows, counts
        /// itself as parked and waits until the gate opens.
        gate: tokio::sync::watch::Sender<bool>,
        parked: AtomicU64,
    }

    impl TestStore {
        /// A store whose scans return `batch_rows` rows per batch.
        pub(crate) fn new(batch_rows: usize) -> Self {
            Self {
                rows: Mutex::new(BTreeMap::new()),
                batch_rows: batch_rows.max(1),
                null: false,
                calls: AtomicU64::new(0),
                scans: Mutex::new(HashMap::new()),
                fail_first: Mutex::new(HashSet::new()),
                fail_resumed: Mutex::new(HashSet::new()),
                fail_loads: Mutex::new(HashSet::new()),
                panic_scan: AtomicBool::new(false),
                gate: tokio::sync::watch::channel(false).0,
                parked: AtomicU64::new(0),
            }
        }

        /// An empty store that reports itself as a null store.
        pub(crate) fn null() -> Self {
            Self {
                null: true,
                ..Self::new(1000)
            }
        }

        /// Writes a row directly, out of band of any record store.
        pub(crate) fn put_row(&self, map: &str, key: &str, value: RecordValue) {
            self.rows
                .lock()
                .entry(map.to_string())
                .or_default()
                .insert(key.to_string(), value);
        }

        pub(crate) fn delete_row(&self, map: &str, key: &str) {
            if let Some(rows) = self.rows.lock().get_mut(map) {
                rows.remove(key);
            }
        }

        pub(crate) fn row(&self, map: &str, key: &str) -> Option<RecordValue> {
            self.rows.lock().get(map).and_then(|r| r.get(key)).cloned()
        }

        /// Every `MapDataStore` method call so far.
        pub(crate) fn calls(&self) -> u64 {
            self.calls.load(Ordering::SeqCst)
        }

        /// `scan_values` calls for `map` so far.
        pub(crate) fn scans(&self, map: &str) -> u64 {
            self.scans.lock().get(map).copied().unwrap_or(0)
        }

        pub(crate) fn fail_scans(&self, map: &str, fail: bool) {
            let mut set = self.fail_first.lock();
            if fail {
                set.insert(map.to_string());
            } else {
                set.remove(map);
            }
        }

        pub(crate) fn fail_resumed_scans(&self, map: &str) {
            self.fail_resumed.lock().insert(map.to_string());
        }

        /// Every later `load` of `key` in `map` fails.
        pub(crate) fn fail_loads(&self, map: &str, key: &str) {
            self.fail_loads
                .lock()
                .insert((map.to_string(), key.to_string()));
        }

        pub(crate) fn panic_scans(&self, panic: bool) {
            self.panic_scan.store(panic, Ordering::SeqCst);
        }

        pub(crate) fn close_gate(&self) {
            self.gate.send_replace(true);
        }

        pub(crate) fn open_gate(&self) {
            self.gate.send_replace(false);
        }

        /// `scan_values` calls that have parked on the gate so far.
        pub(crate) fn parked(&self) -> u64 {
            self.parked.load(Ordering::SeqCst)
        }

        fn count(&self) {
            self.calls.fetch_add(1, Ordering::SeqCst);
        }

        fn batch_after(&self, map: &str, after: Option<&str>) -> ScanBatch {
            let rows = self.rows.lock();
            let Some(map_rows) = rows.get(map) else {
                return ScanBatch::default();
            };
            let mut records = Vec::new();
            let mut next_cursor = None;
            for (key, value) in map_rows {
                if after.is_some_and(|a| key.as_str() <= a) {
                    continue;
                }
                if records.len() == self.batch_rows {
                    let last: String = records
                        .last()
                        .map(|(k, _): &(String, RecordValue)| k.clone())
                        .unwrap_or_default();
                    next_cursor = Some(ScanCursor(last.into_bytes()));
                    break;
                }
                records.push((key.clone(), value.clone()));
            }
            ScanBatch {
                records,
                next_cursor,
            }
        }
    }

    #[async_trait]
    impl MapDataStore for TestStore {
        async fn add(
            &self,
            map: &str,
            key: &str,
            value: &RecordValue,
            _now: i64,
        ) -> anyhow::Result<()> {
            self.count();
            if !self.null {
                self.put_row(map, key, value.clone());
            }
            Ok(())
        }

        async fn add_backup(
            &self,
            _map: &str,
            _key: &str,
            _value: &RecordValue,
            _now: i64,
        ) -> anyhow::Result<()> {
            self.count();
            Ok(())
        }

        async fn remove(&self, map: &str, key: &str, _now: i64) -> anyhow::Result<()> {
            self.count();
            self.delete_row(map, key);
            Ok(())
        }

        async fn remove_backup(&self, _map: &str, _key: &str, _now: i64) -> anyhow::Result<()> {
            self.count();
            Ok(())
        }

        async fn load(&self, map: &str, key: &str) -> anyhow::Result<Option<RecordValue>> {
            self.count();
            if self
                .fail_loads
                .lock()
                .contains(&(map.to_string(), key.to_string()))
            {
                anyhow::bail!("injected load failure");
            }
            Ok(self.row(map, key))
        }

        async fn load_all(
            &self,
            map: &str,
            keys: &[String],
        ) -> anyhow::Result<Vec<(String, RecordValue)>> {
            self.count();
            Ok(keys
                .iter()
                .filter_map(|k| self.row(map, k).map(|v| (k.clone(), v)))
                .collect())
        }

        async fn remove_all(&self, map: &str, keys: &[String]) -> anyhow::Result<()> {
            self.count();
            for key in keys {
                self.delete_row(map, key);
            }
            Ok(())
        }

        async fn enumerate_leaves(
            &self,
            _map: &str,
            _is_backup: bool,
            _sink: &mut dyn LeafSink,
        ) -> anyhow::Result<()> {
            self.count();
            Ok(())
        }

        async fn scan_values(
            &self,
            map: &str,
            _is_backup: bool,
            _max_batch_cost: u64,
        ) -> anyhow::Result<ScanBatch> {
            self.count();
            *self.scans.lock().entry(map.to_string()).or_default() += 1;
            assert!(
                !self.panic_scan.load(Ordering::SeqCst),
                "injected scan_values panic"
            );
            if self.fail_first.lock().contains(map) {
                anyhow::bail!("injected scan_values failure");
            }
            // The rows of a parked call are fixed before it parks: a key written
            // while it is parked is not in this scan.
            let batch = self.batch_after(map, None);
            let mut gate = self.gate.subscribe();
            if *gate.borrow_and_update() {
                self.parked.fetch_add(1, Ordering::SeqCst);
                let _ = gate.wait_for(|closed| !*closed).await;
            }
            Ok(batch)
        }

        async fn scan_values_batched(
            &self,
            map: &str,
            _is_backup: bool,
            cursor: ScanCursor,
            _max_batch_cost: u64,
        ) -> anyhow::Result<ScanBatch> {
            self.count();
            if self.fail_resumed.lock().contains(map) || self.fail_first.lock().contains(map) {
                anyhow::bail!("injected scan_values_batched failure");
            }
            let after = String::from_utf8(cursor.0).expect("test cursor is a key");
            Ok(self.batch_after(map, Some(&after)))
        }

        fn is_loadable(&self, _key: &str) -> bool {
            true
        }

        fn pending_operation_count(&self) -> u64 {
            0
        }

        async fn soft_flush(&self) -> anyhow::Result<u64> {
            self.count();
            Ok(0)
        }

        async fn hard_flush(&self) -> anyhow::Result<()> {
            self.count();
            Ok(())
        }

        fn is_null(&self) -> bool {
            self.null
        }
    }
}

/// Times for the TTL read tests (TG-TTL-001). Every case puts the factory
/// clock far from wall time, so a read path that consulted the wall clock
/// instead of [`RecordStoreFactory::now_ms`] would flip the outcome:
/// - an *expired* record is stamped at wall time with [`TTL_MS`] and read with
///   the clock [`expired_now`], 30 days later (by wall time it is live);
/// - a *live* record is stamped at [`live_stamp`], 10 days ago, and read with
///   the clock [`live_now`], one day before its stamp (by wall time it has
///   expired).
#[cfg(test)]
pub(crate) mod ttl_fixture {
    use topgun_core::hlc::{ClockSource, SystemClock, Timestamp};

    pub(crate) const DAY_MS: u64 = 86_400_000;
    /// The TTL of every expiring record in these tests: one hour.
    pub(crate) const TTL_MS: u64 = 3_600_000;

    /// Wall time now, the stamp of an *expired* record.
    pub(crate) fn wall_stamp() -> u64 {
        SystemClock.now()
    }

    /// The factory clock of an *expired* case: 30 days after wall time.
    pub(crate) fn expired_now() -> u64 {
        wall_stamp() + 30 * DAY_MS
    }

    /// The stamp of a *live* record: 10 days before wall time.
    pub(crate) fn live_stamp() -> u64 {
        wall_stamp() - 10 * DAY_MS
    }

    /// The factory clock of a *live* case: one day before `stamp`.
    pub(crate) fn live_now(stamp: u64) -> u64 {
        stamp - DAY_MS
    }

    pub(crate) fn ts(millis: u64) -> Timestamp {
        Timestamp {
            millis,
            counter: 0,
            node_id: "n".to_string(),
        }
    }
}

#[cfg(test)]
#[allow(clippy::doc_markdown)]
mod tests {
    use crate::storage::key_writer::KeyWriteToken;
    use topgun_core::hlc::Timestamp;
    use topgun_core::types::Value;

    use super::*;
    use crate::storage::datastores::NullDataStore;
    use crate::storage::record::RecordValue;
    use crate::storage::record_store::CallerProvenance;

    fn make_value(s: &str) -> RecordValue {
        RecordValue::Lww {
            value: Value::String(s.to_string()),
            timestamp: Timestamp {
                millis: 1_000_000,
                counter: 0,
                node_id: "node-1".to_string(),
            },
            ttl_ms: None,
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
                &KeyWriteToken::for_test(store.name(), "alice"),
                "alice",
                make_value("data"),
                CallerProvenance::Client,
            )
            .await
            .unwrap();

        let record = store.get("alice", false).await.unwrap();
        assert!(record.is_some());
        assert_eq!(store.size(), 1);
    }

    #[test]
    fn factory_reads_time_from_the_injected_clock() {
        let clock = Arc::new(crate::storage::expiry::ManualClock::new(42));
        let factory = RecordStoreFactory::new(
            StorageConfig::default(),
            Arc::new(NullDataStore),
            Vec::new(),
        )
        .with_clock(clock.clone());
        assert_eq!(factory.now_ms(), 42);
        clock.advance(8);
        assert_eq!(factory.now_ms(), 50);
    }

    #[test]
    fn ttl_write_generation_counts_per_map() {
        let factory = RecordStoreFactory::new(
            StorageConfig::default(),
            Arc::new(NullDataStore),
            Vec::new(),
        );
        assert_eq!(factory.ttl_write_generation("a"), 0);
        factory.note_ttl_write("a");
        assert_eq!(factory.ttl_write_generation("a"), 1, "created at 1");
        factory.note_ttl_write("a");
        factory.note_ttl_write("b");
        assert_eq!(factory.ttl_write_generation("a"), 2);
        assert_eq!(factory.ttl_write_generation("b"), 1);
        assert_eq!(factory.ttl_write_generation("c"), 0);
    }

    /// StorageConfig::default() has all fields set to 0
    #[test]
    fn storage_config_default_all_zeros() {
        let config = StorageConfig::default();
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
                &KeyWriteToken::for_test(store_a.name(), "key1"),
                "key1",
                make_value("a"),
                CallerProvenance::Client,
            )
            .await
            .unwrap();

        assert_eq!(store_a.size(), 1);
        assert_eq!(store_b.size(), 0, "stores should be independent");
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

    /// AC7b(i) negative control: WITHOUT hydration (the pre-fix in-memory-only
    /// scan) a stably-present durable-but-non-resident key is dropped. Proves the
    /// datastore-backed scan is load-bearing.
    #[tokio::test]
    async fn pre_fix_in_memory_only_scan_drops_non_resident_key() {
        let (factory, data_store, _dir) = redb_factory();
        data_store
            .add("m", "stable", &make_value("v"), 1000)
            .await
            .unwrap();

        // No hydration call — this is the pre-fix behavior.
        let keys = scan_resident_keys(&factory, "m");
        assert!(
            keys.is_empty(),
            "pre-fix scan misses the durable non-resident key"
        );
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
                .add("m", &format!("k{i:03}"), &make_value("v"), 1000)
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
                        .add("m", "k000z", &make_value("late"), 2000)
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
        // All originally-seeded keys must still appear exactly once, in the
        // order the store declares: the list is compared as seen, so a
        // repeated key fails it.
        let originals: Vec<String> = seen
            .iter()
            .filter(|k| k.as_str() != "k000z")
            .cloned()
            .collect();
        let mut expected: Vec<String> = (0..20).map(|i| format!("k{i:03}")).collect();
        expected.sort_by_key(|key| {
            (crate::storage::datastores::engine_store::PARTITION_THEN_KEY.key_position)(key)
        });
        assert_eq!(originals, expected, "no original key missed or duplicated");
    }

    // ── TG-READ-001: scan_merged ─────────────────────────────────────────

    mod scan_merged_tests {
        use std::collections::HashMap;

        use topgun_core::hash_to_partition;
        use topgun_core::hlc::Timestamp;
        use topgun_core::types::Value;

        use super::super::test_store::TestStore;
        use super::super::*;
        use crate::storage::datastores::{WriteBehindConfig, WriteBehindDataStore};
        use crate::storage::key_writer::KeyWriteToken;
        use crate::storage::record::{OrMapEntry, RecordValue};
        use crate::storage::record_store::CallerProvenance;

        const MAP: &str = "m";

        fn ts(millis: i64) -> Timestamp {
            Timestamp {
                millis: u64::try_from(millis).unwrap(),
                counter: 0,
                node_id: "n".to_string(),
            }
        }

        fn lww(value: &str, millis: i64) -> RecordValue {
            RecordValue::Lww {
                value: Value::String(value.to_string()),
                timestamp: ts(millis),
                ttl_ms: None,
            }
        }

        fn or_slot(value: &str) -> RecordValue {
            RecordValue::OrMap {
                records: vec![OrMapEntry {
                    value: Value::String(value.to_string()),
                    tag: format!("tag-{value}"),
                    timestamp: ts(1_000),
                    ttl_ms: None,
                }],
                tombstones: Vec::new(),
            }
        }

        fn factory_over(store: Arc<dyn MapDataStore>) -> RecordStoreFactory {
            RecordStoreFactory::new(StorageConfig::default(), store, Vec::new())
        }

        /// Puts `value` into the engine only: replication provenance writes
        /// nothing through to the data store.
        async fn put_resident(factory: &RecordStoreFactory, key: &str, value: RecordValue) {
            let store = factory.get_or_create(MAP, hash_to_partition(key));
            store
                .put(
                    &KeyWriteToken::for_test(MAP, key),
                    key,
                    value,
                    CallerProvenance::Replication,
                )
                .await
                .unwrap();
        }

        async fn collect(factory: &RecordStoreFactory) -> Vec<(String, RecordValue)> {
            let mut out = Vec::new();
            factory
                .scan_merged(MAP, &mut |key, value| {
                    out.push((key.to_string(), value.clone()));
                })
                .await
                .unwrap();
            out
        }

        fn only(rows: &[(String, RecordValue)], key: &str) -> RecordValue {
            let hits: Vec<&RecordValue> = rows
                .iter()
                .filter(|(k, _)| k == key)
                .map(|(_, v)| v)
                .collect();
            assert_eq!(hits.len(), 1, "key {key} emitted {} times", hits.len());
            hits[0].clone()
        }

        /// AC8(a): a resident LWW value newer than the stored row is emitted.
        #[tokio::test(flavor = "multi_thread")]
        async fn scan_merged_a_newer_resident_lww_value_wins() {
            let store = Arc::new(TestStore::new(100));
            store.put_row(MAP, "k", lww("stored", 1_000));
            let factory = factory_over(store.clone());
            put_resident(&factory, "k", lww("resident", 2_000)).await;

            let rows = collect(&factory).await;
            assert_eq!(only(&rows, "k"), lww("resident", 2_000));
            assert_eq!(rows.len(), 1);
        }

        /// AC8(b): a stored row strictly newer than the resident copy is emitted.
        #[tokio::test(flavor = "multi_thread")]
        async fn scan_merged_a_strictly_newer_stored_row_wins() {
            let store = Arc::new(TestStore::new(100));
            store.put_row(MAP, "k", lww("stored", 2_000));
            let factory = factory_over(store.clone());
            put_resident(&factory, "k", lww("resident", 1_000)).await;

            let rows = collect(&factory).await;
            assert_eq!(only(&rows, "k"), lww("stored", 2_000));
            assert_eq!(rows.len(), 1);
        }

        /// AC8(c): equal timestamps keep the resident copy.
        #[tokio::test(flavor = "multi_thread")]
        async fn scan_merged_equal_timestamps_keep_the_resident_copy() {
            let store = Arc::new(TestStore::new(100));
            store.put_row(MAP, "k", lww("stored", 1_500));
            let factory = factory_over(store.clone());
            put_resident(&factory, "k", lww("resident", 1_500)).await;

            let rows = collect(&factory).await;
            assert_eq!(only(&rows, "k"), lww("resident", 1_500));
        }

        /// AC8(d): a resident-only and a stored-only key are each emitted once,
        /// also when the stored rows span several batches.
        #[tokio::test(flavor = "multi_thread")]
        async fn scan_merged_emits_resident_only_and_stored_only_keys_once() {
            let store = Arc::new(TestStore::new(2));
            for i in 0..5 {
                store.put_row(MAP, &format!("s{i}"), lww("stored", 1_000));
            }
            let factory = factory_over(store.clone());
            put_resident(&factory, "r", lww("resident", 1_000)).await;

            let rows = collect(&factory).await;
            assert_eq!(only(&rows, "r"), lww("resident", 1_000));
            for i in 0..5 {
                assert_eq!(only(&rows, &format!("s{i}")), lww("stored", 1_000));
            }
            assert_eq!(rows.len(), 6);
        }

        /// AC8(e): a delete staged in the write-behind store over a stored row
        /// is not emitted.
        #[tokio::test(flavor = "multi_thread")]
        async fn scan_merged_skips_a_staged_delete_over_a_stored_row() {
            let inner = Arc::new(TestStore::new(100));
            inner.put_row(MAP, "gone", lww("stored", 1_000));
            inner.put_row(MAP, "kept", lww("stored", 1_000));
            let write_behind = WriteBehindDataStore::new(
                inner.clone() as Arc<dyn MapDataStore>,
                WriteBehindConfig {
                    // Long enough that the delete stays staged for the test.
                    write_delay_ms: 600_000,
                    flush_interval_ms: 10,
                    ..WriteBehindConfig::default()
                },
            );
            write_behind.remove(MAP, "gone", 2_000).await.unwrap();
            assert!(
                inner.row(MAP, "gone").is_some(),
                "the delete is staged, not yet in the inner store"
            );
            let factory = factory_over(write_behind);

            let rows = collect(&factory).await;
            assert!(
                rows.iter().all(|(k, _)| k != "gone"),
                "staged delete emitted: {rows:?}"
            );
            assert_eq!(only(&rows, "kept"), lww("stored", 1_000));
        }

        /// AC8(f): a scan over non-resident rows loads nothing into the engine.
        #[tokio::test(flavor = "multi_thread")]
        async fn scan_merged_leaves_engine_residency_unchanged() {
            let store = Arc::new(TestStore::new(3));
            for i in 0..10 {
                store.put_row(MAP, &format!("s{i}"), lww("stored", 1_000));
            }
            let factory = factory_over(store.clone());
            put_resident(&factory, "r", lww("resident", 1_000)).await;

            let sizes = |f: &RecordStoreFactory| -> HashMap<u32, usize> {
                f.all_stores()
                    .iter()
                    .map(|s| (s.partition_id(), s.size()))
                    .collect()
            };
            let before = sizes(&factory);
            let rows = collect(&factory).await;
            assert_eq!(rows.len(), 11);
            assert_eq!(
                sizes(&factory),
                before,
                "store set or sizes changed by the scan"
            );
        }

        /// AC8(g): over a null store only the residents are emitted and the data
        /// store is not called.
        #[tokio::test]
        async fn scan_merged_over_a_null_store_emits_residents_and_calls_nothing() {
            let store = Arc::new(TestStore::null());
            let factory = factory_over(store.clone());
            put_resident(&factory, "r1", lww("a", 1_000)).await;
            put_resident(&factory, "r2", lww("b", 1_000)).await;
            let calls = store.calls();

            let rows = collect(&factory).await;
            assert_eq!(only(&rows, "r1"), lww("a", 1_000));
            assert_eq!(only(&rows, "r2"), lww("b", 1_000));
            assert_eq!(rows.len(), 2);
            assert_eq!(store.calls(), calls, "the null store was called");
        }

        /// AC8(h): a failing resumed batch fails the read.
        #[tokio::test(flavor = "multi_thread")]
        async fn scan_merged_fails_on_a_failing_resumed_batch() {
            let store = Arc::new(TestStore::new(1));
            store.put_row(MAP, "a", lww("x", 1_000));
            store.put_row(MAP, "b", lww("y", 1_000));
            store.fail_resumed_scans(MAP);
            let factory = factory_over(store.clone());

            let result = factory.scan_merged(MAP, &mut |_, _| {}).await;
            assert!(result.is_err(), "a failed resumed batch must fail the scan");
        }

        /// AC8(i): a resident OR slot wins over the stored one.
        #[tokio::test(flavor = "multi_thread")]
        async fn scan_merged_a_resident_or_slot_wins_over_the_stored_one() {
            let store = Arc::new(TestStore::new(100));
            store.put_row(MAP, "o", or_slot("stored"));
            let factory = factory_over(store.clone());
            put_resident(&factory, "o", or_slot("resident")).await;

            let rows = collect(&factory).await;
            assert_eq!(only(&rows, "o"), or_slot("resident"));
        }

        // --- Live rows only (TG-TTL-001) ---

        use super::super::ttl_fixture::{expired_now, live_now, live_stamp, wall_stamp, TTL_MS};
        use crate::storage::expiry::ManualClock;

        fn lww_ttl(value: &str, millis: u64, ttl_ms: Option<u64>) -> RecordValue {
            RecordValue::Lww {
                value: Value::String(value.to_string()),
                timestamp: super::super::ttl_fixture::ts(millis),
                ttl_ms,
            }
        }

        fn entry(tag: &str, millis: u64, ttl_ms: Option<u64>) -> OrMapEntry {
            OrMapEntry {
                value: Value::String(tag.to_string()),
                tag: tag.to_string(),
                timestamp: super::super::ttl_fixture::ts(millis),
                ttl_ms,
            }
        }

        fn clocked(
            store: Arc<dyn MapDataStore>,
            now: u64,
        ) -> (RecordStoreFactory, Arc<ManualClock>) {
            let clock = Arc::new(ManualClock::new(now));
            let factory = factory_over(store).with_clock(Arc::clone(&clock) as _);
            (factory, clock)
        }

        fn keys(rows: &[(String, RecordValue)]) -> Vec<String> {
            let mut keys: Vec<String> = rows.iter().map(|(k, _)| k.clone()).collect();
            keys.sort();
            keys
        }

        /// A stored-only row that has expired is not emitted; a stored row
        /// without a TTL beside it is.
        #[tokio::test(flavor = "multi_thread")]
        async fn scan_merged_hides_expired_non_resident() {
            let store = Arc::new(TestStore::new(1));
            store.put_row(MAP, "gone", lww_ttl("x", wall_stamp(), Some(TTL_MS)));
            store.put_row(MAP, "kept", lww_ttl("y", wall_stamp(), None));
            let (factory, _clock) = clocked(store, expired_now());

            assert_eq!(keys(&collect(&factory).await), vec!["kept"]);
        }

        /// A resident-only record that has expired is not emitted.
        #[tokio::test(flavor = "multi_thread")]
        async fn scan_merged_hides_expired_resident() {
            let store = Arc::new(TestStore::new(100));
            let (factory, _clock) = clocked(store, expired_now());
            put_resident(&factory, "gone", lww_ttl("x", wall_stamp(), Some(TTL_MS))).await;
            put_resident(&factory, "kept", lww_ttl("y", wall_stamp(), None)).await;

            assert_eq!(keys(&collect(&factory).await), vec!["kept"]);
        }

        /// An expired resident still wins the merge over an older live stored
        /// row of its key, so neither is emitted: the stored copy is never
        /// served in its place.
        #[tokio::test(flavor = "multi_thread")]
        async fn scan_merged_expired_resident_still_shadows_stored() {
            let store = Arc::new(TestStore::new(100));
            store.put_row(MAP, "k", lww_ttl("stored", wall_stamp() - 1_000, None));
            let (factory, _clock) = clocked(store, expired_now());
            put_resident(
                &factory,
                "k",
                lww_ttl("resident", wall_stamp(), Some(TTL_MS)),
            )
            .await;

            let rows = collect(&factory).await;
            assert!(rows.is_empty(), "the older stored row was served: {rows:?}");
        }

        /// A resident LWW tombstone with an expired TTL (a delete written with
        /// `ttlMs`) shadows an older live stored row like any expired
        /// resident: the deleted key is not resurrected.
        #[tokio::test(flavor = "multi_thread")]
        async fn scan_merged_expired_tombstone_does_not_resurrect_stored() {
            let store = Arc::new(TestStore::new(100));
            store.put_row(MAP, "k", lww_ttl("stored", wall_stamp() - 1_000, None));
            let (factory, _clock) = clocked(store, expired_now());
            put_resident(
                &factory,
                "k",
                RecordValue::Lww {
                    value: Value::Null,
                    timestamp: super::super::ttl_fixture::ts(wall_stamp()),
                    ttl_ms: Some(TTL_MS),
                },
            )
            .await;

            let rows = collect(&factory).await;
            assert!(rows.is_empty(), "the deleted key was resurrected: {rows:?}");
        }

        /// A live record is emitted at the exact millisecond it expires and not
        /// one millisecond later, resident and stored alike.
        #[tokio::test(flavor = "multi_thread")]
        async fn scan_merged_live_at_expiry_instant() {
            let stamp = live_stamp();
            let store = Arc::new(TestStore::new(100));
            store.put_row(MAP, "stored", lww_ttl("x", stamp, Some(TTL_MS)));
            let (factory, clock) = clocked(store, stamp + TTL_MS);
            put_resident(&factory, "resident", lww_ttl("y", stamp, Some(TTL_MS))).await;

            assert_eq!(
                keys(&collect(&factory).await),
                vec!["resident", "stored"],
                "live at now == expires_at"
            );
            clock.set(stamp + TTL_MS + 1);
            assert!(
                collect(&factory).await.is_empty(),
                "expired one millisecond later"
            );
        }

        /// The same instant, judged with the clock far before wall time: a
        /// record stamped 10 days ago with a one-hour TTL is live while the
        /// factory clock says so, whatever the wall clock says.
        #[tokio::test(flavor = "multi_thread")]
        async fn lww_is_live_at_expiry_instant() {
            let stamp = live_stamp();
            let store = Arc::new(TestStore::new(100));
            store.put_row(MAP, "stored", lww_ttl("x", stamp, Some(TTL_MS)));
            let (factory, clock) = clocked(store, live_now(stamp));
            put_resident(&factory, "resident", lww_ttl("y", stamp, Some(TTL_MS))).await;

            assert_eq!(keys(&collect(&factory).await), vec!["resident", "stored"]);
            clock.set(stamp + TTL_MS);
            assert_eq!(keys(&collect(&factory).await), vec!["resident", "stored"]);
        }

        /// `ttl_ms: Some(0)` never expires, matching the client.
        #[tokio::test(flavor = "multi_thread")]
        async fn ttl_zero_never_expires() {
            let store = Arc::new(TestStore::new(100));
            store.put_row(MAP, "stored", lww_ttl("x", live_stamp(), Some(0)));
            let (factory, _clock) = clocked(store, expired_now());
            put_resident(&factory, "resident", lww_ttl("y", live_stamp(), Some(0))).await;

            assert_eq!(keys(&collect(&factory).await), vec!["resident", "stored"]);
        }

        /// A TTL near `u64::MAX` saturates to "never" (in `expires_at`), so the
        /// record stays live 30 days on.
        #[tokio::test(flavor = "multi_thread")]
        async fn scan_merged_huge_ttl_is_live() {
            let store = Arc::new(TestStore::new(100));
            store.put_row(MAP, "stored", lww_ttl("x", wall_stamp(), Some(u64::MAX)));
            let (factory, _clock) = clocked(store, expired_now());
            put_resident(
                &factory,
                "resident",
                lww_ttl("y", wall_stamp(), Some(u64::MAX)),
            )
            .await;

            assert_eq!(keys(&collect(&factory).await), vec!["resident", "stored"]);
        }

        /// An OR slot keeps its live entries and its tombstones and loses only
        /// its expired entries; a slot whose entries all expired is still
        /// emitted, with its key, as `OrMap { records: [], tombstones }`, the
        /// shape of a slot every entry of which was removed.
        #[tokio::test(flavor = "multi_thread")]
        async fn scan_merged_or_slot_keeps_live_tag() {
            let w = wall_stamp();
            let store = Arc::new(TestStore::new(100));
            store.put_row(
                MAP,
                "stored",
                RecordValue::OrMap {
                    records: vec![entry("old", w, Some(TTL_MS)), entry("live", w, None)],
                    tombstones: vec!["t1".to_string()],
                },
            );
            store.put_row(
                MAP,
                "empty",
                RecordValue::OrMap {
                    records: vec![entry("old", w, Some(TTL_MS))],
                    tombstones: vec!["t2".to_string()],
                },
            );
            let (factory, _clock) = clocked(store, expired_now());
            put_resident(
                &factory,
                "resident",
                RecordValue::OrMap {
                    records: vec![
                        entry("live", w, Some(u64::MAX)),
                        entry("old", w, Some(TTL_MS)),
                    ],
                    tombstones: Vec::new(),
                },
            )
            .await;

            let rows = collect(&factory).await;
            assert_eq!(
                only(&rows, "stored"),
                RecordValue::OrMap {
                    records: vec![entry("live", w, None)],
                    tombstones: vec!["t1".to_string()],
                }
            );
            assert_eq!(
                only(&rows, "resident"),
                RecordValue::OrMap {
                    records: vec![entry("live", w, Some(u64::MAX))],
                    tombstones: Vec::new(),
                }
            );
            assert_eq!(
                only(&rows, "empty"),
                RecordValue::OrMap {
                    records: Vec::new(),
                    tombstones: vec!["t2".to_string()],
                }
            );
        }
    }
}
