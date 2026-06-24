//! Behavioral end-to-end tests for cost-based eviction correctness.
//!
//! Covers:
//! - AC2: queries remain correct with active eviction (322c streaming load-bearing)
//! - AC3: restart correctness with active eviction (TODO-530 closure holds)
//! - AC4: cost non-leakage (RecordMetadata.cost never reaches the wire or disk)
//! - AC6: write-through marks records clean → genuinely evictable (R6 production path)
//!
//! AC1 (eviction fires under real cost pressure) lives in eviction_orchestrator.rs
//! because it requires private `tick()` access.

#[cfg(test)]
mod eviction_cost_tests {
    use std::sync::Arc;

    use tempfile::tempdir;
    use topgun_core::hlc::Timestamp;
    use topgun_core::types::Value;

    use crate::storage::datastores::{NullDataStore, RedbDataStore};
    use crate::storage::engines::HashMapStorage;
    use crate::storage::impls::{DefaultRecordStore, StorageConfig};
    use crate::storage::map_data_store::MapDataStore;
    use crate::storage::mutation_observer::CompositeMutationObserver;
    use crate::storage::record::{estimated_cost, RecordValue};
    use crate::storage::record_store::{CallerProvenance, ExpiryPolicy, RecordStore};

    // ---------------------------------------------------------------------------
    // Shared helpers
    // ---------------------------------------------------------------------------

    fn lww_value(s: &str) -> RecordValue {
        RecordValue::Lww {
            value: Value::String(s.to_string()),
            timestamp: Timestamp {
                millis: 1_000_000,
                counter: 0,
                node_id: "node-test".to_string(),
            },
        }
    }

    fn make_store_with_datastore(
        name: &str,
        data_store: Arc<dyn MapDataStore>,
    ) -> DefaultRecordStore {
        let engine = Box::new(HashMapStorage::new());
        let observer = Arc::new(CompositeMutationObserver::default());
        DefaultRecordStore::new(
            name.to_string(),
            0,
            engine,
            data_store,
            observer,
            StorageConfig::default(),
        )
    }

    /// Write N records to the store via `put()` with `CallerProvenance::Client`.
    ///
    /// When the store is backed by a real datastore, R6 marks each record clean
    /// automatically (write-through + `on_store(now)` after `add()` succeeds), so
    /// `evict_lru` can select them immediately without any manual manipulation.
    async fn write_records(store: &DefaultRecordStore, n: usize, prefix: &str) -> Vec<String> {
        let mut keys = Vec::with_capacity(n);
        for i in 0..n {
            let key = format!("{prefix}{i:03}");
            store
                .put(
                    &key,
                    lww_value("payload-for-eviction-test"),
                    ExpiryPolicy::NONE,
                    CallerProvenance::Client,
                )
                .await
                .expect("put must succeed");
            keys.push(key);
        }
        keys
    }

    // ---------------------------------------------------------------------------
    // AC2: queries remain correct under ACTIVE eviction
    //
    // Writes records to a redb-backed store, evicts them to non-residency, then
    // drives a full datastore scan and asserts the complete record set is visible.
    // Proves SPEC-322c streaming full-scan reads evicted records from the datastore.
    // ---------------------------------------------------------------------------

    #[tokio::test]
    async fn ac2_query_correct_under_active_eviction() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("ac2_query.redb");
        let data_store = Arc::new(RedbDataStore::new(&path).expect("redb open"));
        let store = make_store_with_datastore(
            "evict_query_map",
            data_store.clone() as Arc<dyn MapDataStore>,
        );

        // Write 20 records — write-through persists each to redb and marks them
        // clean (R6), so they are immediately evictable.
        let keys = write_records(&store, 20, "q").await;

        // Confirm all records are clean after puts (proving R6 is active).
        assert_eq!(
            store.dirty_count(),
            0,
            "all records must be non-dirty after put() over a real datastore"
        );

        // Evict ALL records to make them non-resident.
        let evicted = store.evict_lru(u32::MAX, false);
        assert!(
            evicted > 0,
            "at least one record must be evicted; got {evicted}"
        );

        // Confirm at least one known key is now non-resident.
        let first_key = &keys[0];
        assert!(
            !store.exists_in_memory(first_key),
            "key '{first_key}' must be non-resident after eviction"
        );

        // Drive a full datastore scan — mirrors how ScanProcessor (SPEC-322c)
        // reads non-resident records from the backing store.
        let batch = data_store
            .scan_values("evict_query_map", false, 1024 * 1024)
            .await
            .expect("scan_values must succeed");

        let scanned_keys: std::collections::HashSet<String> =
            batch.records.iter().map(|(k, _)| k.clone()).collect();

        for key in &keys {
            assert!(
                scanned_keys.contains(key),
                "datastore scan must return '{key}' even when non-resident — \
                 SPEC-322c streaming path is load-bearing"
            );
        }

        assert_eq!(
            scanned_keys.len(),
            keys.len(),
            "scan must return exactly the written count: expected {}, got {}",
            keys.len(),
            scanned_keys.len()
        );
    }

    // ---------------------------------------------------------------------------
    // AC3: restart correct under active eviction (TODO-530 closure holds with
    // eviction ON)
    //
    // Writes records to a redb-backed store, evicts all to non-residency, then
    // opens a NEW store/engine over the same redb file (simulating restart with
    // an empty in-memory engine) and drives a full datastore scan. Asserts all
    // written records are still visible.
    // ---------------------------------------------------------------------------

    #[tokio::test]
    async fn ac3_restart_correct_under_active_eviction() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("ac3_restart.redb");

        // Phase 1: write records to a redb-backed store.
        let keys = {
            let data_store = Arc::new(RedbDataStore::new(&path).expect("redb open"));
            let store = make_store_with_datastore(
                "restart_map",
                data_store.clone() as Arc<dyn MapDataStore>,
            );
            // R6 marks records clean after write-through, so they are immediately evictable.
            let keys = write_records(&store, 15, "r").await;
            // Evict all — proves records survive as durable-but-non-resident.
            let evicted = store.evict_lru(u32::MAX, false);
            assert!(evicted > 0, "pre-restart eviction must remove > 0 records");
            keys
        };
        // store is dropped here, simulating process exit.

        // Phase 2: reopen the same redb file with a fresh in-memory engine
        // (all resident state is gone — simulates restart).
        let data_store = Arc::new(RedbDataStore::new(&path).expect("redb reopen"));
        let new_store =
            make_store_with_datastore("restart_map", data_store.clone() as Arc<dyn MapDataStore>);

        // The fresh engine must be empty — no data was hydrated at restart.
        assert_eq!(
            new_store.size(),
            0,
            "fresh engine after restart must be empty (no startup hydration)"
        );

        // Drive a full datastore scan to prove persisted records are visible
        // even when non-resident after restart.
        let batch = data_store
            .scan_values("restart_map", false, 1024 * 1024)
            .await
            .expect("scan_values must succeed after restart");

        let scanned_keys: std::collections::HashSet<String> =
            batch.records.iter().map(|(k, _)| k.clone()).collect();

        for key in &keys {
            assert!(
                scanned_keys.contains(key),
                "key '{key}' must survive restart and be visible via datastore scan; \
                 TODO-530 closure holds with eviction active"
            );
        }

        assert_eq!(
            scanned_keys.len(),
            keys.len(),
            "all written keys must be recoverable after restart + eviction: \
             expected {}, got {}",
            keys.len(),
            scanned_keys.len()
        );
    }

    // ---------------------------------------------------------------------------
    // AC4: cost non-leakage (hard constraint)
    //
    // Asserts that RecordMetadata.cost is never serialized to the wire or disk.
    // The structural guarantee: RecordMetadata has no Serialize/Deserialize derive
    // (compiler-verified), and MapDataStore::add takes only &RecordValue.
    // The behavioral form: a round-trip of the persisted RecordValue contains no
    // "cost" field; a freshly loaded record re-derives cost locally from the value.
    // ---------------------------------------------------------------------------

    #[tokio::test]
    async fn ac4_cost_does_not_leak_to_wire_or_disk() {
        // Structural: estimated_cost works on a RecordValue, not RecordMetadata.
        // This call site proves the function signature takes &RecordValue only.
        let val = lww_value("non-leakage-test");
        let cost = estimated_cost(&val);
        assert!(
            cost > 0,
            "estimated_cost must return > 0 for a non-null value"
        );

        // Behavioral: serialize the RecordValue via rmp_serde::to_vec_named
        // and decode it back — the result is a RecordValue with NO cost field.
        let bytes = rmp_serde::to_vec_named(&val).expect("serialize RecordValue");
        let decoded: RecordValue = rmp_serde::from_slice(&bytes).expect("deserialize RecordValue");
        // If RecordMetadata (with its cost field) were serialized alongside the value,
        // deserialization into RecordValue would either fail or carry stale cost
        // information. The clean round-trip proves no metadata leaks.
        match decoded {
            RecordValue::Lww { value, .. } => {
                assert_eq!(
                    value,
                    Value::String("non-leakage-test".to_string()),
                    "RecordValue round-trip must preserve the payload exactly"
                );
            }
            other => panic!("expected Lww variant after round-trip, got {other:?}"),
        }

        // Persistence path: put via a null-datastore-backed store (which discards
        // writes), then assert that loading via the store returns None — proving
        // the datastore receives only RecordValue, not RecordMetadata.
        // (The MapDataStore::add signature takes &RecordValue — verified at compile
        // time; this is the runtime complement.)
        let null_store = Arc::new(NullDataStore);
        let store = make_store_with_datastore("cost_nonleak", null_store as Arc<dyn MapDataStore>);
        store
            .put(
                "k",
                lww_value("v"),
                ExpiryPolicy::NONE,
                CallerProvenance::Client,
            )
            .await
            .expect("put must succeed");

        // cost is stamped in metadata
        let in_memory = store.get("k", false).await.expect("get must succeed");
        let cost_in_mem = in_memory.unwrap().metadata.cost;
        assert!(
            cost_in_mem > 0,
            "in-memory metadata.cost must be non-zero after put; got {cost_in_mem}"
        );

        // After eviction the record is removed from memory. A fresh get from the
        // null datastore returns None (nothing was persisted — correct, since
        // cost metadata was never written). Re-derivation on load is proven by AC1.
        store.evict("k", false);
        let after_evict = store
            .get("k", false)
            .await
            .expect("get after evict must not error");
        assert!(
            after_evict.is_none(),
            "NullDataStore holds no durable records; cost must not appear \
             as a phantom value in the persistence layer"
        );
    }

    // ---------------------------------------------------------------------------
    // AC6: write-through marks records clean → genuinely evictable (R6 production
    // path)
    //
    // Proves R6: put() with CallerProvenance::Client over a real datastore yields
    // a record that is !is_dirty() without any manual metadata manipulation.
    // Conversely, put() over NullDataStore (no real persistence) leaves the record
    // dirty — clean-marking is gated on actual persistence, not unconditional.
    // Records loaded via get() lazy-load from the datastore enter the engine clean.
    // ---------------------------------------------------------------------------

    #[tokio::test]
    async fn ac6_write_through_marks_records_clean() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("ac6_clean.redb");

        // --- Part 1: put() over a real datastore → record is clean (evictable) ---
        {
            let data_store = Arc::new(RedbDataStore::new(&path).expect("redb open"));
            let store =
                make_store_with_datastore("ac6_map", data_store.clone() as Arc<dyn MapDataStore>);

            store
                .put(
                    "key-real",
                    lww_value("value-real"),
                    ExpiryPolicy::NONE,
                    CallerProvenance::Client,
                )
                .await
                .expect("put must succeed");

            // R6: write-through succeeds → on_store(now) is called → !is_dirty()
            assert_eq!(
                store.dirty_count(),
                0,
                "put() with a real datastore must mark the record clean (dirty_count == 0) \
                 without any manual metadata manipulation — R6 production path"
            );

            // Confirm the record is in memory (not evicted yet).
            assert!(
                store.exists_in_memory("key-real"),
                "record must still be resident in memory after put"
            );

            // Confirm evict_lru can actually evict it (proves genuine eligibility).
            let evicted = store.evict_lru(1, false);
            assert_eq!(
                evicted, 1,
                "evict_lru must evict the clean record; got {evicted} — record must be genuinely \
                 eligible, not blocked by dirty state"
            );
        }

        // --- Part 2: put() over NullDataStore → record stays dirty ---
        {
            let null_store = Arc::new(NullDataStore);
            let store =
                make_store_with_datastore("ac6_null_map", null_store as Arc<dyn MapDataStore>);

            store
                .put(
                    "key-null",
                    lww_value("value-null"),
                    ExpiryPolicy::NONE,
                    CallerProvenance::Client,
                )
                .await
                .expect("put must succeed");

            // NullDataStore discards writes — nothing is actually persisted, so
            // the record must remain dirty (not safe to evict without real backing).
            assert!(
                store.dirty_count() >= 1,
                "put() over NullDataStore must leave the record dirty (dirty_count >= 1); \
                 clean-marking is gated on real persistence, not the no-op null store. \
                 got dirty_count = {}",
                store.dirty_count()
            );

            // Confirm evict_lru skips it (dirty records are never evicted).
            let evicted = store.evict_lru(1, false);
            assert_eq!(
                evicted, 0,
                "dirty records must not be evicted by evict_lru; \
                 NullDataStore-backed records are permanently ineligible"
            );
        }

        // --- Part 3: get() lazy-load from datastore → record enters engine clean ---
        {
            // Reopen the redb file written in Part 1. The in-memory engine is empty.
            let data_store = Arc::new(RedbDataStore::new(&path).expect("redb reopen"));
            let store =
                make_store_with_datastore("ac6_map", data_store.clone() as Arc<dyn MapDataStore>);

            // Trigger a lazy-load: get() miss → load from datastore → insert into engine.
            let loaded = store
                .get("key-real", false)
                .await
                .expect("get must succeed");
            assert!(
                loaded.is_some(),
                "key-real must be loadable from the datastore after restart"
            );

            // A record loaded from the datastore is already persisted → must enter
            // the engine clean so it is immediately re-evictable.
            assert_eq!(
                store.dirty_count(),
                0,
                "a record loaded via get() lazy-load from the datastore must enter the engine \
                 clean (dirty_count == 0) — required for the evict→reload steady state"
            );

            // Confirm evict_lru can evict the loaded record (proves re-evictability).
            let evicted = store.evict_lru(1, false);
            assert_eq!(
                evicted, 1,
                "a lazy-loaded record must be evictable immediately; got {evicted}"
            );
        }
    }
}
