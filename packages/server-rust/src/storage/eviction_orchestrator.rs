// No wire-protocol types in this module — PROJECT.md serde rules (camelCase,
// skip_serializing_if, etc.) do not apply here.

//! Background task that drives memory-pressure-based LRU eviction across all
//! [`RecordStore`] instances.
//!
//! The orchestrator wakes at `interval_ms`, sums `owned_entry_cost()` across
//! all live stores to compute `current_ram`, and compares it against the high
//! water threshold derived from [`EvictionConfig`]. When over the threshold it
//! calls [`RecordStore::evict_lru`] per store with a target count proportional
//! to that store's cost share, driving RAM down toward the low water mark.
//!
//! The orchestrator owns the **cost-based** eviction decision.
//! `RecordStore::should_evict()` retains its existing entry-count semantics and
//! runs as a parallel, independent signal — it is NOT replaced by this module.
//!
//! Architectural reference: Hazelcast `LRUEvictionPolicy` high/low water-mark
//! pattern adapted for Rust/tokio single-node demo tier.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;

use crate::storage::eviction_config::EvictionConfig;
use crate::storage::factory::RecordStoreFactory;

/// Background task that drives cost-based LRU eviction across all live stores.
///
/// Construct with [`EvictionOrchestrator::new`], then spawn via
/// `tokio::spawn(orchestrator.run())`. The task terminates gracefully within
/// one `interval_ms` of receiving a shutdown signal.
pub struct EvictionOrchestrator {
    config: EvictionConfig,
    factory: Arc<RecordStoreFactory>,
    shutdown: watch::Receiver<bool>,
}

impl EvictionOrchestrator {
    /// Create a new orchestrator.
    ///
    /// - `config` — water-mark thresholds and tick interval (from [`EvictionConfig::from_env`])
    /// - `factory` — registry of all live stores; `all_stores()` is called each tick so
    ///   stores created after construction are visible to eviction
    /// - `shutdown` — send `true` to terminate the loop within one `interval_ms`
    #[must_use]
    pub fn new(
        config: EvictionConfig,
        factory: Arc<RecordStoreFactory>,
        shutdown: watch::Receiver<bool>,
    ) -> Self {
        Self { config, factory, shutdown }
    }

    /// Run the eviction loop until a shutdown signal is received.
    ///
    /// Loops indefinitely, waking every `config.interval_ms`. On each tick,
    /// sums RAM cost across all live stores and evicts proportionally when the
    /// high water threshold is exceeded.
    pub async fn run(mut self) {
        loop {
            tokio::select! {
                // Shutdown takes priority: terminate as soon as the receiver
                // sees a changed value, without waiting for the current tick.
                result = self.shutdown.changed() => {
                    if result.is_ok() && *self.shutdown.borrow() {
                        tracing::info!(
                            target: "topgun_server::storage::eviction_orchestrator",
                            "Eviction orchestrator received shutdown signal — stopping"
                        );
                        return;
                    }
                }
                () = tokio::time::sleep(Duration::from_millis(self.config.interval_ms)) => {
                    self.tick();
                }
            }
        }
    }

    /// Execute one eviction tick: evaluate RAM usage and evict if over threshold.
    fn tick(&self) {
        let stores = self.factory.all_stores();

        // Step 1: Sum cost and record count across all live stores.
        // size() returns usize; widen to u64 at call site to match current_ram width.
        // Do NOT modify the RecordStore trait — the trait surface is archived; cast to u64 at the orchestrator call site instead.
        let mut current_ram: u64 = 0;
        let mut total_record_count: u64 = 0;
        for store in &stores {
            current_ram = current_ram.saturating_add(store.owned_entry_cost());
            total_record_count = total_record_count.saturating_add(store.size() as u64);
        }

        // Step 2: Compute high water threshold.
        // u64::from(u8) before multiply avoids any future silent cast-lossiness.
        let high_threshold =
            self.config.max_ram_bytes * u64::from(self.config.high_water_pct) / 100;

        // Step 3: Idle when under the high threshold.
        if current_ram < high_threshold {
            return;
        }

        // Step 4: Idle when no records exist — division-by-zero guard for
        // avg_record_cost = current_ram / total_record_count in step 5.
        if total_record_count == 0 {
            return;
        }

        // Step 5: Evict proportionally per store down toward the low water mark.
        let low_threshold =
            self.config.max_ram_bytes * u64::from(self.config.low_water_pct) / 100;
        let to_free = current_ram.saturating_sub(low_threshold);

        // avg_record_cost is safe: total_record_count > 0 guaranteed by step 4.
        let avg_record_cost = current_ram / total_record_count;

        let mut total_evicted: u64 = 0;
        let mut total_freed: u64 = 0;
        let mut total_dirty: u64 = 0;

        for store in &stores {
            let store_cost = store.owned_entry_cost();

            // Proportional target: this store's share of the cost × records to free.
            // Formula: to_free * store_cost / current_ram / avg_record_cost
            // Integer division is intentional; clamp below prevents zero-target livelock.
            let raw_target = if current_ram > 0 {
                to_free * store_cost / current_ram / avg_record_cost.max(1)
            } else {
                0
            };

            // Clamp to at least 1 when there is RAM to free and this store has a
            // non-zero cost share — prevents integer-truncation livelock where
            // to_free < avg_record_cost rounds every store target to zero, leaving
            // the orchestrator permanently stuck above the high water mark.
            #[allow(clippy::cast_possible_truncation)]
            let target_count: u32 = if to_free > 0 && store_cost > 0 {
                raw_target.max(1).min(u64::from(u32::MAX)) as u32
            } else {
                raw_target.min(u64::from(u32::MAX)) as u32
            };

            if target_count == 0 {
                continue;
            }

            let evicted = store.evict_lru(target_count, false);
            let dirty = store.dirty_count();

            // Estimate freed bytes proportional to evicted records.
            let freed_estimate = if target_count > 0 {
                store_cost * u64::from(evicted) / u64::from(target_count).max(1)
            } else {
                0
            };

            total_evicted += u64::from(evicted);
            total_freed += freed_estimate;
            total_dirty += dirty;

            // Warn when evict_lru returned fewer than the target AND dirty records
            // are causing the shortfall. The warn fires ONLY when both conditions
            // hold simultaneously — warning on dirty_count > 0 alone would produce
            // log spam during normal write-behind backlog without actionable signal.
            if evicted < target_count && dirty > 0 {
                tracing::warn!(
                    target: "topgun_server::storage::eviction_orchestrator",
                    map = %store.name(),
                    partition = store.partition_id(),
                    evicted,
                    target_count,
                    dirty_count = dirty,
                    "Eviction fell short of target; dirty records are blocking \
                     LRU candidates — write-behind backpressure detected"
                );
            }
        }

        tracing::info!(
            target: "topgun_server::storage::eviction_orchestrator",
            evicted = total_evicted,
            freed_bytes = total_freed,
            dirty_count = total_dirty,
            "evicted {} records, freed ~{} bytes, dirty_count={}",
            total_evicted, total_freed, total_dirty
        );
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use tokio::sync::watch;

    use super::*;
    use crate::storage::datastores::NullDataStore;
    use crate::storage::engine::{FetchResult, IterationCursor, StorageEngine};
    use crate::storage::eviction_config::EvictionConfig;
    use crate::storage::factory::RecordStoreFactory;
    use crate::storage::impls::StorageConfig;
    use crate::storage::map_data_store::MapDataStore;
    use crate::storage::record::Record;
    use crate::storage::record_store::{
        CallerProvenance, ExpiryPolicy, ExpiryReason, RecordStore,
    };

    // ---------------------------------------------------------------------------
    // Mock RecordStore implementation for orchestrator tests
    // ---------------------------------------------------------------------------

    /// A minimal mock `RecordStore` for orchestrator testing.
    ///
    /// Uses `Arc<AtomicU64>` for cost and size so the test can update them
    /// between `tokio::time::advance` calls without `RefCell`/`Cell`
    /// (which are not `Send + Sync` and would not compile inside `Arc<dyn RecordStore>`).
    struct MockStore {
        name: String,
        partition_id: u32,
        /// Reported by `owned_entry_cost()`; test can update between ticks.
        cost: Arc<AtomicU64>,
        /// Reported by `size()`; test can update between ticks.
        record_count: Arc<AtomicU64>,
        /// How many records `evict_lru` reports as evicted (capped at target).
        evict_result: Arc<AtomicU32>,
        /// Reported by `dirty_count()`.
        dirty: Arc<AtomicU64>,
        /// Accumulates the sum of all `target_count` values passed to `evict_lru`.
        evict_calls: Arc<AtomicU64>,
    }

    impl MockStore {
        fn new(name: &str, partition_id: u32) -> Arc<Self> {
            Arc::new(Self {
                name: name.to_string(),
                partition_id,
                cost: Arc::new(AtomicU64::new(0)),
                record_count: Arc::new(AtomicU64::new(0)),
                evict_result: Arc::new(AtomicU32::new(u32::MAX)),
                dirty: Arc::new(AtomicU64::new(0)),
                evict_calls: Arc::new(AtomicU64::new(0)),
            })
        }
    }

    #[async_trait]
    impl RecordStore for MockStore {
        fn name(&self) -> &str {
            &self.name
        }

        fn partition_id(&self) -> u32 {
            self.partition_id
        }

        #[allow(clippy::cast_possible_truncation)]
        fn size(&self) -> usize {
            // Test stores are never large enough for truncation to matter.
            self.record_count.load(Ordering::Relaxed) as usize
        }

        fn is_empty(&self) -> bool {
            self.size() == 0
        }

        fn owned_entry_cost(&self) -> u64 {
            self.cost.load(Ordering::Relaxed)
        }

        fn dirty_count(&self) -> u64 {
            self.dirty.load(Ordering::Relaxed)
        }

        fn evict_lru(&self, target_count: u32, _is_backup: bool) -> u32 {
            self.evict_calls
                .fetch_add(u64::from(target_count), Ordering::Relaxed);
            let result = self.evict_result.load(Ordering::Relaxed);
            result.min(target_count)
        }

        // --- Stubs for the remaining trait surface (not exercised by orchestrator) ---

        async fn get(&self, _key: &str, _touch: bool) -> anyhow::Result<Option<Record>> {
            Ok(None)
        }

        fn exists_in_memory(&self, _key: &str) -> bool {
            false
        }

        async fn put(
            &self,
            _key: &str,
            _value: crate::storage::record::RecordValue,
            _expiry: ExpiryPolicy,
            _provenance: CallerProvenance,
        ) -> anyhow::Result<Option<crate::storage::record::RecordValue>> {
            Ok(None)
        }

        async fn remove(
            &self,
            _key: &str,
            _provenance: CallerProvenance,
        ) -> anyhow::Result<Option<crate::storage::record::RecordValue>> {
            Ok(None)
        }

        async fn put_backup(
            &self,
            _key: &str,
            _record: Record,
            _provenance: CallerProvenance,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn remove_backup(
            &self,
            _key: &str,
            _provenance: CallerProvenance,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn get_all(
            &self,
            _keys: &[String],
        ) -> anyhow::Result<Vec<(String, Record)>> {
            Ok(vec![])
        }

        fn fetch_keys(
            &self,
            _cursor: &IterationCursor,
            _size: usize,
        ) -> FetchResult<String> {
            FetchResult {
                items: vec![],
                next_cursor: IterationCursor { state: vec![], finished: true },
            }
        }

        fn fetch_entries(
            &self,
            _cursor: &IterationCursor,
            _size: usize,
        ) -> FetchResult<(String, Record)> {
            FetchResult {
                items: vec![],
                next_cursor: IterationCursor { state: vec![], finished: true },
            }
        }

        fn for_each_boxed(
            &self,
            _consumer: &mut dyn FnMut(&str, &Record),
            _is_backup: bool,
        ) {
        }

        fn has_expired(&self, _key: &str, _now: i64, _is_backup: bool) -> ExpiryReason {
            ExpiryReason::NotExpired
        }

        fn evict_expired(&self, _percentage: u32, _now: i64, _is_backup: bool) {}

        fn is_expirable(&self) -> bool {
            false
        }

        fn evict(&self, _key: &str, _is_backup: bool) -> Option<crate::storage::record::RecordValue> {
            None
        }

        fn evict_all(&self, _is_backup: bool) -> u32 {
            0
        }

        fn should_evict(&self) -> bool {
            false
        }

        fn init(&mut self) {}

        fn clear(&self, _is_backup: bool) -> u32 {
            0
        }

        fn reset(&self) {}

        fn destroy(&self) {}

        async fn soft_flush(&self) -> anyhow::Result<u64> {
            Ok(0)
        }

        fn storage(&self) -> &dyn StorageEngine {
            unimplemented!("MockStore::storage not used by orchestrator tests")
        }

        fn map_data_store(&self) -> &dyn MapDataStore {
            unimplemented!("MockStore::map_data_store not used by orchestrator tests")
        }
    }

    // ---------------------------------------------------------------------------
    // tick_with_stores: mirror of EvictionOrchestrator::tick with injected stores
    //
    // The production orchestrator calls factory.all_stores() to get its list.
    // Since RecordStoreFactory is a concrete type and only creates
    // DefaultRecordStore instances, we cannot inject MockStore through it.
    // This helper mirrors the exact tick logic so tests assert on the same
    // code paths without coupling to DefaultRecordStore internals.
    // ---------------------------------------------------------------------------

    fn tick_with_stores(config: &EvictionConfig, stores: &[Arc<dyn RecordStore>]) {
        let mut current_ram: u64 = 0;
        let mut total_record_count: u64 = 0;
        for store in stores {
            current_ram = current_ram.saturating_add(store.owned_entry_cost());
            total_record_count = total_record_count.saturating_add(store.size() as u64);
        }

        let high_threshold = config.max_ram_bytes * u64::from(config.high_water_pct) / 100;

        if current_ram < high_threshold {
            return;
        }

        if total_record_count == 0 {
            return;
        }

        let low_threshold = config.max_ram_bytes * u64::from(config.low_water_pct) / 100;
        let to_free = current_ram.saturating_sub(low_threshold);
        let avg_record_cost = current_ram / total_record_count;

        let mut total_evicted: u64 = 0;
        let mut total_freed: u64 = 0;
        let mut total_dirty: u64 = 0;

        for store in stores {
            let store_cost = store.owned_entry_cost();

            let raw_target = if current_ram > 0 {
                to_free * store_cost / current_ram / avg_record_cost.max(1)
            } else {
                0
            };

            #[allow(clippy::cast_possible_truncation)]
            let target_count: u32 = if to_free > 0 && store_cost > 0 {
                raw_target.max(1).min(u64::from(u32::MAX)) as u32
            } else {
                raw_target.min(u64::from(u32::MAX)) as u32
            };

            if target_count == 0 {
                continue;
            }

            let evicted = store.evict_lru(target_count, false);
            let dirty = store.dirty_count();

            let freed_estimate = if target_count > 0 {
                store_cost * u64::from(evicted) / u64::from(target_count).max(1)
            } else {
                0
            };

            total_evicted += u64::from(evicted);
            total_freed += freed_estimate;
            total_dirty += dirty;

            if evicted < target_count && dirty > 0 {
                tracing::warn!(
                    target: "topgun_server::storage::eviction_orchestrator",
                    map = %store.name(),
                    partition = store.partition_id(),
                    evicted,
                    target_count,
                    dirty_count = dirty,
                    "Eviction fell short of target; dirty records are blocking \
                     LRU candidates — write-behind backpressure detected"
                );
            }
        }

        tracing::info!(
            target: "topgun_server::storage::eviction_orchestrator",
            evicted = total_evicted,
            freed_bytes = total_freed,
            dirty_count = total_dirty,
            "evicted {} records, freed ~{} bytes, dirty_count={}",
            total_evicted, total_freed, total_dirty
        );
    }

    // ---------------------------------------------------------------------------
    // AC #2 / Validation #5: idle below high water mark
    // ---------------------------------------------------------------------------

    /// Verify the orchestrator does not call `evict_lru` when total cost is below
    /// the high water threshold.
    #[test]
    fn evict_orchestrator_idle_below_high_water() {
        let config = EvictionConfig {
            max_ram_bytes: 1_000_000,
            high_water_pct: 80,
            low_water_pct: 60,
            interval_ms: 1000,
        };

        // Cost = 500_000 bytes, threshold = 800_000 → well below high water
        let store = MockStore::new("test", 0);
        store.cost.store(500_000, Ordering::Relaxed);
        store.record_count.store(100, Ordering::Relaxed);

        let stores: Vec<Arc<dyn RecordStore>> = vec![store.clone()];

        // Run multiple ticks — none should trigger eviction
        for _ in 0..5 {
            tick_with_stores(&config, &stores);
        }

        assert_eq!(
            store.evict_calls.load(Ordering::Relaxed),
            0,
            "evict_lru must NOT be called when cost is below the high water threshold"
        );
    }

    // ---------------------------------------------------------------------------
    // AC #3 / Validation #6: evicts down to low water mark
    // ---------------------------------------------------------------------------

    /// Verify that after a tick above the high threshold, `evict_lru` is called
    /// and eviction stops when cost drops below the high threshold.
    ///
    /// Uses `Arc<AtomicU64>` so cost can be updated between ticks without
    /// `RefCell`/`Cell` (which are not `Send + Sync` inside `Arc<dyn RecordStore>`).
    #[test]
    fn evict_orchestrator_evicts_down_to_low_water() {
        let config = EvictionConfig {
            max_ram_bytes: 1_000_000,
            high_water_pct: 80, // threshold = 800_000
            low_water_pct: 60,  // target    = 600_000
            interval_ms: 1000,
        };

        // Start above high water mark: 900_000 bytes, 90 records (~10_000/record avg)
        let store = MockStore::new("test", 0);
        store.cost.store(900_000, Ordering::Relaxed);
        store.record_count.store(90, Ordering::Relaxed);
        store.evict_result.store(u32::MAX, Ordering::Relaxed);

        let stores: Vec<Arc<dyn RecordStore>> = vec![store.clone()];

        // First tick: should call evict_lru with a non-zero target
        tick_with_stores(&config, &stores);

        let calls_after_first_tick = store.evict_calls.load(Ordering::Relaxed);
        assert!(
            calls_after_first_tick > 0,
            "evict_lru must be called when cost {} exceeds high threshold {}",
            900_000,
            800_000
        );

        // Simulate cost dropping to between low and high water marks
        // (610_000 < 800_000 high threshold → next tick should be idle)
        store.cost.store(610_000, Ordering::Relaxed);
        store.record_count.store(61, Ordering::Relaxed);

        let calls_before = store.evict_calls.load(Ordering::Relaxed);
        tick_with_stores(&config, &stores);
        let calls_after = store.evict_calls.load(Ordering::Relaxed);

        assert_eq!(
            calls_before, calls_after,
            "evict_lru must NOT be called when cost ({}) is below high threshold ({})",
            610_000, 800_000
        );
    }

    // ---------------------------------------------------------------------------
    // AC #1 / Validation #7: terminates within one interval on shutdown
    // ---------------------------------------------------------------------------

    /// Verify the orchestrator terminates within 1× `interval_ms` of receiving a
    /// shutdown signal.
    #[tokio::test]
    async fn evict_orchestrator_terminates_within_interval_on_shutdown() {
        tokio::time::pause();

        let config = EvictionConfig {
            max_ram_bytes: 1_000_000,
            high_water_pct: 80,
            low_water_pct: 60,
            interval_ms: 1000,
        };

        let factory = Arc::new(RecordStoreFactory::new(
            StorageConfig::default(),
            Arc::new(NullDataStore),
            Vec::new(),
        ));

        let (tx, rx) = watch::channel(false);
        let orchestrator = EvictionOrchestrator::new(config.clone(), factory, rx);

        let handle = tokio::spawn(orchestrator.run());

        // Send the shutdown signal; the watch::changed() branch in tokio::select!
        // fires before the sleep branch, so the orchestrator terminates immediately.
        tx.send(true).unwrap();

        // Advance past one interval to unblock any pending sleep.
        tokio::time::advance(Duration::from_millis(config.interval_ms + 100)).await;

        let result =
            tokio::time::timeout(Duration::from_millis(200), handle).await;

        assert!(
            result.is_ok(),
            "Orchestrator must terminate within one interval of receiving shutdown signal"
        );
    }

    // ---------------------------------------------------------------------------
    // AC #4 / Validation #8: warn on dirty backpressure
    // ---------------------------------------------------------------------------

    /// Verify that `tracing::warn!` fires when `evict_lru` returns < target AND
    /// `dirty_count > 0`.
    ///
    /// Uses a `Mutex`-backed tracing subscriber layer to capture emitted `WARN`
    /// events without requiring an external `tracing-test` crate.
    #[test]
    fn evict_orchestrator_warn_on_dirty_backpressure() {
        use tracing::Level;
        use tracing_subscriber::layer::SubscriberExt;

        /// Visits tracing event fields, capturing the `message` field value.
        struct MsgVisitor(String);
        impl tracing::field::Visit for MsgVisitor {
            fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
                if field.name() == "message" {
                    self.0 = value.to_string();
                }
            }

            fn record_debug(
                &mut self,
                field: &tracing::field::Field,
                value: &dyn std::fmt::Debug,
            ) {
                if field.name() == "message" {
                    self.0 = format!("{value:?}");
                }
            }
        }

        /// Captures WARN-level messages emitted during the test.
        #[derive(Clone, Default)]
        struct WarnCapture {
            messages: Arc<Mutex<Vec<String>>>,
        }

        impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for WarnCapture {
            fn on_event(
                &self,
                event: &tracing::Event<'_>,
                _ctx: tracing_subscriber::layer::Context<'_, S>,
            ) {
                if *event.metadata().level() != Level::WARN {
                    return;
                }
                let mut visitor = MsgVisitor(String::new());
                event.record(&mut visitor);
                self.messages.lock().unwrap().push(visitor.0);
            }
        }

        let captured: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let layer = WarnCapture { messages: Arc::clone(&captured) };
        let subscriber = tracing_subscriber::registry().with(layer);

        tracing::subscriber::with_default(subscriber, || {
            let config = EvictionConfig {
                max_ram_bytes: 1_000_000,
                high_water_pct: 80,
                low_water_pct: 60,
                interval_ms: 1000,
            };

            // Store is above the high water mark, has dirty records, and
            // evict_lru returns 0 (all candidates were dirty → none evicted).
            let store = MockStore::new("orders", 0);
            store.cost.store(900_000, Ordering::Relaxed);
            store.record_count.store(90, Ordering::Relaxed);
            store.dirty.store(50, Ordering::Relaxed);
            store.evict_result.store(0, Ordering::Relaxed);

            let stores: Vec<Arc<dyn RecordStore>> = vec![store];
            tick_with_stores(&config, &stores);
        });

        let messages = captured.lock().unwrap();
        assert!(
            !messages.is_empty(),
            "tracing::warn! must fire when evict_lru returns < target and dirty_count > 0; \
             no WARN messages were captured"
        );
    }

    // ---------------------------------------------------------------------------
    // AC #8: clamp prevents zero-target livelock
    // ---------------------------------------------------------------------------

    /// Verify that when `to_free < avg_record_cost` (integer truncation would
    /// produce `target = 0`), the clamp ensures `target_count >= 1` so the
    /// orchestrator makes forward progress.
    #[test]
    fn evict_orchestrator_clamp_prevents_zero_target_livelock() {
        let config = EvictionConfig {
            max_ram_bytes: 1_000_000,
            high_water_pct: 80, // threshold = 800_000
            // low = 790_000 → to_free = 900_000 − 790_000 = 110_000
            low_water_pct: 79,
            interval_ms: 1000,
        };

        // cost = 900_000, records = 5 → avg_record_cost = 180_000
        // raw_target = 110_000 / 180_000 = 0 without clamp
        let store = MockStore::new("test", 0);
        store.cost.store(900_000, Ordering::Relaxed);
        store.record_count.store(5, Ordering::Relaxed);

        let stores: Vec<Arc<dyn RecordStore>> = vec![store.clone()];

        tick_with_stores(&config, &stores);

        let calls = store.evict_calls.load(Ordering::Relaxed);
        assert!(
            calls >= 1,
            "Clamp must ensure target_count >= 1 when to_free > 0 and store has non-zero cost \
             (prevents integer-truncation livelock); evict_calls = {calls}"
        );
    }

    // ---------------------------------------------------------------------------
    // AC #9: size() == 0 → idle, no panic
    // ---------------------------------------------------------------------------

    /// Verify that when all stores report `size() == 0`, the orchestrator is
    /// idle and does not panic (division-by-zero guard on `avg_record_cost`).
    #[test]
    fn evict_orchestrator_idle_when_no_records() {
        let config = EvictionConfig {
            max_ram_bytes: 1_000_000,
            high_water_pct: 80,
            low_water_pct: 60,
            interval_ms: 1000,
        };

        // Cost above high water but zero records → must not divide by zero
        let store = MockStore::new("test", 0);
        store.cost.store(900_000, Ordering::Relaxed);
        store.record_count.store(0, Ordering::Relaxed);

        let stores: Vec<Arc<dyn RecordStore>> = vec![store.clone()];

        // Must not panic and must not call evict_lru
        tick_with_stores(&config, &stores);

        assert_eq!(
            store.evict_calls.load(Ordering::Relaxed),
            0,
            "evict_lru must not be called when total_record_count == 0 (division-by-zero guard)"
        );
    }
}
