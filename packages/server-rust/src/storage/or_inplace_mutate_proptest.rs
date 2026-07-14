//! Differential resident-state equivalence proof for the OR-Map in-place mutate
//! write path.
//!
//! The OR write path used to read-modify-write by cloning the whole per-key
//! `RecordValue::OrMap` snapshot, rebuilding it, and re-putting it on every op.
//! It now mutates the resident slot IN PLACE via
//! [`RecordStore::update_in_place`](crate::storage::RecordStore::update_in_place).
//! These tests prove the switch changed only *how* the slot is written, not
//! *what* ends up resident: a random OR op sequence applied via BOTH paths
//! yields a byte-identical resident `OrMap` (same records order, same tombstone
//! order — and, canonically, the same [`or_map_semantic_view`] live/tombstone
//! sets), fires the SAME observer notifications, and performs the SAME durable
//! write-through.
//!
//! This is a LIVE-PATH equivalence proof: no `kill -9`, no WAL recovery fold
//! (the full-snapshot WAL write-through is unchanged, so crash recovery is
//! byte-identical to before and is covered by the crash-safety proptests).
//!
//! It also proves the SPEC-345 tombstone-bytes gauge stays consistent under the
//! in-place path (the gauge deltas route through `add_tombstone_bytes` /
//! `sub_tombstone_bytes`, never bypassed), and that the cheap OrMap-arm
//! `estimated_cost` estimate stays close to the true serialized size so the
//! eviction water-mark is not skewed.

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use proptest::prelude::*;
    use tokio::runtime::Handle;
    use tokio::sync::Mutex as AsyncMutex;
    use tokio::task::block_in_place;
    use topgun_core::hlc::Timestamp;
    use topgun_core::types::Value;

    use crate::service::domain::crdt::or_map_semantic_view;
    use crate::storage::engines::HashMapStorage;
    use crate::storage::impls::{DefaultRecordStore, StorageConfig};
    use crate::storage::map_data_store::{LeafSink, MapDataStore, ScanBatch, ScanCursor};
    use crate::storage::mutation_observer::{CompositeMutationObserver, MutationObserver};
    use crate::storage::record::{
        add_tombstone_bytes, estimated_cost, sub_tombstone_bytes, tombstone_bytes, OrMapEntry,
        Record, RecordValue,
    };
    use crate::storage::record_store::{CallerProvenance, ExpiryPolicy, RecordStore};

    const MAP: &str = "omap";
    const KEY: &str = "k1";

    // -----------------------------------------------------------------------
    // Async-bridge runtime (CLAUDE.md §Proptest Async Bridge)
    // -----------------------------------------------------------------------

    static PROPTEST_RUNTIME: std::sync::LazyLock<tokio::runtime::Runtime> =
        std::sync::LazyLock::new(|| {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("build multi-thread runtime for proptest async bridge")
        });

    fn block_on_async<F: std::future::Future>(fut: F) -> F::Output {
        let handle = PROPTEST_RUNTIME.handle().clone();
        let _guard = handle.enter();
        block_in_place(|| Handle::current().block_on(fut))
    }

    // The tombstone-bytes gauge is a single process-global static shared with
    // every other test that touches OR removes/prunes. Serialize this module's
    // gauge-measuring region so its own passes do not interleave; delta-based
    // assertions keep it robust against unrelated modules' mutations.
    static GAUGE_LOCK: Mutex<()> = Mutex::new(());

    // -----------------------------------------------------------------------
    // Counting mutation observer — proves the two paths fire identical fan-out
    // -----------------------------------------------------------------------

    #[derive(Default)]
    struct CountingObserver {
        put: AtomicUsize,
        update: AtomicUsize,
        remove: AtomicUsize,
        load: AtomicUsize,
    }

    impl CountingObserver {
        fn snapshot(&self) -> (usize, usize, usize, usize) {
            (
                self.put.load(Ordering::Relaxed),
                self.update.load(Ordering::Relaxed),
                self.remove.load(Ordering::Relaxed),
                self.load.load(Ordering::Relaxed),
            )
        }
    }

    impl MutationObserver for CountingObserver {
        fn on_put(&self, _: &str, _: &Record, _: Option<&RecordValue>, _: bool) {
            self.put.fetch_add(1, Ordering::Relaxed);
        }
        fn on_update(&self, _: &str, _: &Record, _: &RecordValue, _: &RecordValue, _: bool) {
            self.update.fetch_add(1, Ordering::Relaxed);
        }
        fn on_remove(&self, _: &str, _: &Record, _: bool) {
            self.remove.fetch_add(1, Ordering::Relaxed);
        }
        fn on_evict(&self, _: &str, _: &Record, _: bool) {}
        fn on_load(&self, _: &str, _: &Record, _: bool) {
            self.load.fetch_add(1, Ordering::Relaxed);
        }
        fn on_replication_put(&self, _: &str, _: &Record, _: bool) {}
        fn on_clear(&self) {}
        fn on_reset(&self) {}
        fn on_destroy(&self, _: bool) {}
    }

    // -----------------------------------------------------------------------
    // Retaining + add-counting data store — proves the two paths write through
    // the SAME durable content the same number of times. `is_null()` is false
    // so the full write-through + mark_stored discipline runs.
    // -----------------------------------------------------------------------

    #[derive(Default)]
    struct RetainingCountingStore {
        data: AsyncMutex<HashMap<(String, String), RecordValue>>,
        adds: AtomicUsize,
    }

    #[async_trait]
    impl MapDataStore for RetainingCountingStore {
        async fn add(
            &self,
            map: &str,
            key: &str,
            value: &RecordValue,
            _exp: i64,
            _now: i64,
        ) -> anyhow::Result<()> {
            self.adds.fetch_add(1, Ordering::Relaxed);
            self.data
                .lock()
                .await
                .insert((map.to_string(), key.to_string()), value.clone());
            Ok(())
        }

        async fn add_backup(
            &self,
            _: &str,
            _: &str,
            _: &RecordValue,
            _: i64,
            _: i64,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn remove(&self, map: &str, key: &str, _now: i64) -> anyhow::Result<()> {
            self.data
                .lock()
                .await
                .remove(&(map.to_string(), key.to_string()));
            Ok(())
        }

        async fn remove_backup(&self, _: &str, _: &str, _: i64) -> anyhow::Result<()> {
            Ok(())
        }

        async fn load(&self, map: &str, key: &str) -> anyhow::Result<Option<RecordValue>> {
            Ok(self
                .data
                .lock()
                .await
                .get(&(map.to_string(), key.to_string()))
                .cloned())
        }

        async fn load_all(
            &self,
            map: &str,
            keys: &[String],
        ) -> anyhow::Result<Vec<(String, RecordValue)>> {
            let guard = self.data.lock().await;
            let mut out = Vec::new();
            for key in keys {
                if let Some(value) = guard.get(&(map.to_string(), key.clone())) {
                    out.push((key.clone(), value.clone()));
                }
            }
            Ok(out)
        }

        async fn remove_all(&self, map: &str, keys: &[String]) -> anyhow::Result<()> {
            let mut guard = self.data.lock().await;
            for key in keys {
                guard.remove(&(map.to_string(), key.clone()));
            }
            Ok(())
        }

        async fn enumerate_leaves(
            &self,
            _: &str,
            _: bool,
            _: &mut dyn LeafSink,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn scan_values(&self, _: &str, _: bool, _: u64) -> anyhow::Result<ScanBatch> {
            Ok(ScanBatch::default())
        }

        async fn scan_values_batched(
            &self,
            _: &str,
            _: bool,
            _: ScanCursor,
            _: u64,
        ) -> anyhow::Result<ScanBatch> {
            Ok(ScanBatch::default())
        }

        fn is_loadable(&self, _: &str) -> bool {
            true
        }

        fn pending_operation_count(&self) -> u64 {
            0
        }

        async fn soft_flush(&self) -> anyhow::Result<u64> {
            Ok(0)
        }

        async fn hard_flush(&self) -> anyhow::Result<()> {
            Ok(())
        }

        async fn flush_key(&self, _: &str, _: &str, _: &RecordValue, _: bool) -> anyhow::Result<()> {
            Ok(())
        }

        fn reset(&self) {}

        // A real (non-null) backend so the full write-through + mark_stored path
        // exercises exactly what production does.
        fn is_null(&self) -> bool {
            false
        }
    }

    // -----------------------------------------------------------------------
    // Op model + shared helpers
    // -----------------------------------------------------------------------

    #[derive(Debug, Clone)]
    enum OrOp {
        Add { tag: String, val: i64 },
        Remove { tag: String },
        Prune { tag: String },
    }

    fn ts(millis: u64) -> Timestamp {
        Timestamp {
            millis,
            counter: 0,
            node_id: "node-1".to_string(),
        }
    }

    fn empty_ormap() -> RecordValue {
        RecordValue::OrMap {
            records: Vec::new(),
            tombstones: Vec::new(),
        }
    }

    /// Mirror of the private `read_or_map_state` in `crdt.rs`, so the legacy
    /// comparison path reproduces the exact old get→build→put behavior.
    fn read_state(value: Option<RecordValue>) -> (Vec<OrMapEntry>, Vec<String>) {
        match value {
            Some(RecordValue::OrMap {
                records,
                tombstones,
            }) => (records, tombstones),
            Some(RecordValue::OrTombstones { tags }) => (Vec::new(), tags),
            _ => (Vec::new(), Vec::new()),
        }
    }

    struct Harness {
        store: DefaultRecordStore,
        observer: Arc<CountingObserver>,
        datastore: Arc<RetainingCountingStore>,
    }

    fn make_harness() -> Harness {
        let observer = Arc::new(CountingObserver::default());
        let datastore = Arc::new(RetainingCountingStore::default());
        let composite = Arc::new(CompositeMutationObserver::new(vec![
            Arc::clone(&observer) as Arc<dyn MutationObserver>
        ]));
        let store = DefaultRecordStore::new(
            MAP.to_string(),
            0,
            Box::new(HashMapStorage::new()),
            Arc::clone(&datastore) as Arc<dyn MapDataStore>,
            composite,
            StorageConfig::default(),
        );
        Harness {
            store,
            observer,
            datastore,
        }
    }

    /// Apply an op through the NEW in-place mutate path — mirrors the production
    /// `crdt.rs` OR write path exactly (same mutate closures, same gauge calls).
    async fn apply_inplace(store: &DefaultRecordStore, op: &OrOp) {
        match op {
            OrOp::Add { tag, val } => {
                let entry = OrMapEntry {
                    value: Value::Int(*val),
                    tag: tag.clone(),
                    timestamp: ts(u64::try_from(val.rem_euclid(1_000_000)).unwrap_or(0)),
                };
                let mut entry_opt = Some(entry);
                let mut merge = move |value: &mut RecordValue| {
                    if let RecordValue::OrMap {
                        records,
                        tombstones,
                    } = value
                    {
                        let e = entry_opt.take().expect("runs once");
                        if !tombstones.contains(&e.tag) {
                            records.retain(|r| r.tag != e.tag);
                            records.push(e);
                        }
                    }
                    true
                };
                store
                    .update_in_place(
                        KEY,
                        Some(empty_ormap()),
                        ExpiryPolicy::NONE,
                        CallerProvenance::CrdtMerge,
                        &mut merge,
                    )
                    .await
                    .unwrap();
            }
            OrOp::Remove { tag } => {
                let mut is_new = false;
                {
                    let mut apply = |value: &mut RecordValue| {
                        if let RecordValue::OrMap {
                            records,
                            tombstones,
                        } = value
                        {
                            records.retain(|e| e.tag != *tag);
                            if !tombstones.contains(tag) {
                                tombstones.push(tag.clone());
                                is_new = true;
                            }
                        }
                        true
                    };
                    store
                        .update_in_place(
                            KEY,
                            Some(empty_ormap()),
                            ExpiryPolicy::NONE,
                            CallerProvenance::CrdtMerge,
                            &mut apply,
                        )
                        .await
                        .unwrap();
                }
                if is_new {
                    add_tombstone_bytes(tag.len() as u64);
                }
            }
            OrOp::Prune { tag } => {
                let mut dropped = false;
                {
                    let mut drop_tag = |value: &mut RecordValue| {
                        if let RecordValue::OrMap { tombstones, .. } = value {
                            let before = tombstones.len();
                            tombstones.retain(|t| t != tag);
                            dropped = tombstones.len() != before;
                        }
                        dropped
                    };
                    store
                        .update_in_place(
                            KEY,
                            None,
                            ExpiryPolicy::NONE,
                            CallerProvenance::CrdtMerge,
                            &mut drop_tag,
                        )
                        .await
                        .unwrap();
                }
                if dropped {
                    sub_tombstone_bytes(tag.len() as u64);
                }
            }
        }
    }

    /// Apply an op through the OLD get→build→put path — the historical
    /// `crdt.rs` OR write path this fix replaced.
    async fn apply_legacy(store: &DefaultRecordStore, op: &OrOp) {
        match op {
            OrOp::Add { tag, val } => {
                let entry = OrMapEntry {
                    value: Value::Int(*val),
                    tag: tag.clone(),
                    timestamp: ts(u64::try_from(val.rem_euclid(1_000_000)).unwrap_or(0)),
                };
                let existing = store.get(KEY, false).await.unwrap();
                let (mut records, tombstones) = read_state(existing.map(|r| r.value));
                let rv = if tombstones.contains(&entry.tag) {
                    RecordValue::OrMap {
                        records,
                        tombstones,
                    }
                } else {
                    records.retain(|e| e.tag != entry.tag);
                    records.push(entry);
                    RecordValue::OrMap {
                        records,
                        tombstones,
                    }
                };
                store
                    .put(KEY, rv, ExpiryPolicy::NONE, CallerProvenance::CrdtMerge)
                    .await
                    .unwrap();
            }
            OrOp::Remove { tag } => {
                let existing = store.get(KEY, false).await.unwrap();
                let (mut records, mut tombstones) = read_state(existing.map(|r| r.value));
                records.retain(|e| e.tag != *tag);
                let is_new = !tombstones.contains(tag);
                if is_new {
                    tombstones.push(tag.clone());
                    add_tombstone_bytes(tag.len() as u64);
                }
                store
                    .put(
                        KEY,
                        RecordValue::OrMap {
                            records,
                            tombstones,
                        },
                        ExpiryPolicy::NONE,
                        CallerProvenance::CrdtMerge,
                    )
                    .await
                    .unwrap();
            }
            OrOp::Prune { tag } => {
                let existing = store.get(KEY, false).await.unwrap();
                let (records, mut tombstones) = read_state(existing.map(|r| r.value));
                let before = tombstones.len();
                tombstones.retain(|t| t != tag);
                if tombstones.len() != before {
                    store
                        .put(
                            KEY,
                            RecordValue::OrMap {
                                records,
                                tombstones,
                            },
                            ExpiryPolicy::NONE,
                            CallerProvenance::CrdtMerge,
                        )
                        .await
                        .unwrap();
                    sub_tombstone_bytes(tag.len() as u64);
                }
            }
        }
    }

    /// Sum of every resident tombstone tag's UTF-8 byte length — the same
    /// `tag.len()` accounting `reconcile_tombstone_bytes` recomputes at boot.
    fn resident_tombstone_bytes(value: &Option<RecordValue>) -> u64 {
        match value {
            Some(RecordValue::OrMap { tombstones, .. }) => {
                tombstones.iter().map(|t| t.len() as u64).sum()
            }
            _ => 0,
        }
    }

    // -----------------------------------------------------------------------
    // Strategies
    // -----------------------------------------------------------------------

    /// Small tag space so adds/removes/prunes overlap on the same tags
    /// (interleaved add/remove on the same tag, removes of non-existent tags,
    /// prunes racing removes).
    fn or_op() -> impl Strategy<Value = OrOp> {
        prop_oneof![
            (0u8..6, any::<i32>()).prop_map(|(t, v)| OrOp::Add {
                tag: format!("t{t}"),
                val: i64::from(v),
            }),
            (0u8..6).prop_map(|t| OrOp::Remove {
                tag: format!("t{t}"),
            }),
            (0u8..6).prop_map(|t| OrOp::Prune {
                tag: format!("t{t}"),
            }),
        ]
    }

    // -----------------------------------------------------------------------
    // AC6: resident-state + observer + write-through differential equivalence
    // -----------------------------------------------------------------------

    proptest! {
        #![proptest_config(ProptestConfig { cases: 128, ..ProptestConfig::default() })]

        /// The in-place path and the get→build→put path leave a byte-identical
        /// resident `OrMap` (records order + tombstones order), the same canonical
        /// [`or_map_semantic_view`], the same observer fan-out counts, and the same
        /// durable write-through content and add count.
        #[test]
        fn inplace_matches_get_build_put(ops in prop::collection::vec(or_op(), 1..40)) {
            block_on_async(async {
                let inplace = make_harness();
                let legacy = make_harness();

                for op in &ops {
                    apply_inplace(&inplace.store, op).await;
                    apply_legacy(&legacy.store, op).await;
                }

                let a = inplace.store.get(KEY, false).await.unwrap().map(|r| r.value);
                let b = legacy.store.get(KEY, false).await.unwrap().map(|r| r.value);

                // Byte-for-byte resident equality (records order + tombstones order),
                // the HARD resident-state-equivalence invariant.
                prop_assert_eq!(&a, &b, "resident OrMap must be byte-identical across the two write paths");

                // Canonical set-based oracle (order-independent) as a second check.
                prop_assert_eq!(
                    or_map_semantic_view(a.clone()),
                    or_map_semantic_view(b.clone()),
                    "semantic view (live + tombstone sets) must match"
                );

                // Identical observer fan-out (on_put for a fresh key, on_update for
                // an existing one — the ENTER/UPDATE distinction search relies on).
                prop_assert_eq!(
                    inplace.observer.snapshot(),
                    legacy.observer.snapshot(),
                    "observer notification counts must match across the two write paths"
                );

                // Identical durable write-through: same persisted content, same
                // number of add() calls.
                let da = inplace.datastore.data.lock().await.get(&(MAP.to_string(), KEY.to_string())).cloned();
                let db = legacy.datastore.data.lock().await.get(&(MAP.to_string(), KEY.to_string())).cloned();
                prop_assert_eq!(da, db, "durable write-through content must match");
                prop_assert_eq!(
                    inplace.datastore.adds.load(Ordering::Relaxed),
                    legacy.datastore.adds.load(Ordering::Relaxed),
                    "write-through must fire the same number of times"
                );

                Ok(())
            })?;
        }
    }

    // -----------------------------------------------------------------------
    // AC6: SPEC-345 gauge reconciliation under the in-place path
    // -----------------------------------------------------------------------

    /// After a mixed add/remove/dup-remove/prune sequence via the in-place path,
    /// the net tombstone-bytes gauge change equals the tombstone-byte sum
    /// recomputed from the resident slot — i.e. the in-place path routes gauge
    /// deltas through `add_tombstone_bytes` / `sub_tombstone_bytes`, never
    /// bypassing them, so a boot `reconcile_tombstone_bytes` would agree.
    #[test]
    fn gauge_reconciles_with_resident_tombstones_under_inplace() {
        let _serialize = GAUGE_LOCK.lock().unwrap();
        block_on_async(async {
            let h = make_harness();

            let ops = vec![
                OrOp::Add {
                    tag: "t0".into(),
                    val: 1,
                },
                OrOp::Add {
                    tag: "t1".into(),
                    val: 2,
                },
                OrOp::Remove { tag: "t0".into() }, // new tombstone
                OrOp::Remove { tag: "t0".into() }, // duplicate — must NOT re-count
                OrOp::Remove { tag: "t2".into() }, // tombstone for a never-added tag
                OrOp::Add {
                    tag: "t0".into(),
                    val: 3,
                }, // remove-wins: suppressed, tombstone stays
                OrOp::Prune { tag: "t2".into() }, // drop one tombstone
                OrOp::Prune { tag: "t9".into() }, // no-op prune (absent tag)
            ];

            let baseline = tombstone_bytes();
            for op in &ops {
                apply_inplace(&h.store, op).await;
            }
            let net_delta = tombstone_bytes().saturating_sub(baseline);

            let resident = h.store.get(KEY, false).await.unwrap().map(|r| r.value);
            let recomputed = resident_tombstone_bytes(&resident);

            assert_eq!(
                net_delta, recomputed,
                "in-place gauge delta ({net_delta}) must equal the resident tombstone-byte \
                 sum ({recomputed}) — the gauge must not be bypassed"
            );
        });
    }

    // -----------------------------------------------------------------------
    // AC6 (Rec 3): cheap OrMap estimated_cost stays close to the true size
    // -----------------------------------------------------------------------

    /// The structural OrMap-arm cost estimate feeds the `TOPGUN_MAX_RAM_MB`
    /// eviction water-mark, so it must not materially diverge from the true
    /// `rmp_serde` serialized size. Assert it stays within 1.5× in both
    /// directions for representative OrMap slots.
    #[test]
    fn estimated_cost_ormap_fidelity_within_1_5x() {
        let slots = [
            build_slot(1, 0),
            build_slot(10, 3),
            build_slot(48, 12),
            build_slot(100, 40),
        ];
        for value in &slots {
            let cheap = estimated_cost(value);
            let true_bytes = rmp_serde::to_vec_named(value).unwrap().len() as u64;
            let cheap_f = cheap as f64;
            let true_f = true_bytes as f64;
            assert!(
                cheap_f <= true_f * 1.5 && true_f <= cheap_f * 1.5,
                "estimated_cost {cheap} must be within 1.5x of true serialized size \
                 {true_bytes} (ratio {:.3})",
                cheap_f / true_f
            );
        }
    }

    fn build_slot(records: usize, tombstones: usize) -> RecordValue {
        RecordValue::OrMap {
            records: (0..records)
                .map(|i| OrMapEntry {
                    value: Value::Int(i as i64 * 7 + 1),
                    tag: format!("{}:{}:node-1", 1_700_000_000_000_u64 + i as u64, i),
                    timestamp: ts(1_700_000_000_000 + i as u64),
                })
                .collect(),
            tombstones: (0..tombstones)
                .map(|i| format!("{}:{}:node-1", 1_600_000_000_000_u64 + i as u64, i))
                .collect(),
        }
    }
}
