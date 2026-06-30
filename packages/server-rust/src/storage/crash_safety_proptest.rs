//! Storage-layer crash-safety property tests for the no-acked-loss invariant.
//!
//! These tests prove the no-acked-loss crash-safety guarantee behaviorally,
//! driving the **real** production write path end-to-end:
//!
//! - A real `WriteBehindDataStore` built over a real file-backed `WalWriter` in
//!   a temp directory (append-before-ack is production behavior: the WAL frame is
//!   durable before the store method returns `Ok(())`).
//! - The crash is modeled by **dropping** the `WriteBehindDataStore` — its
//!   in-memory staging buffer vanishes — while the temp WAL files on disk survive
//!   intact. This faithfully models SIGKILL: the process dies but the OS preserves
//!   bytes already written to the file (the test process stays alive, so the file
//!   is fully readable, including written-but-not-yet-fsynced bytes).
//! - Recovery runs the real `WalRecovery` into a **fresh** inner `MapDataStore`
//!   (a retaining store — NOT `NullDataStore`, which would discard writes and make
//!   the post-recovery assertion vacuous).
//!
//! The oracle is "acked-before-crash ⊆ present-after-recovery": every op that
//! received an ack before the drop must be present after recovery; ops that never
//! acked may legitimately be absent.
//!
//! Every WAL / recovery / classification code path exercised is production code
//! (`WalRecovery::run`, `WalWriter::unapplied` → `format::decode_all`); the only
//! thing faked is the crash itself.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use proptest::prelude::*;
use tokio::runtime::Handle;
use tokio::sync::Mutex as AsyncMutex;
use tokio::task::block_in_place;
use topgun_core::hlc::Timestamp;
use topgun_core::types::Value;

use crate::storage::datastores::{
    RedbDataStore, WalBootstrap, WriteBehindConfig, WriteBehindDataStore,
};
use crate::storage::map_data_store::{LeafSink, MapDataStore, ScanBatch, ScanCursor};
use crate::storage::record::{OrMapEntry, RecordValue};
use crate::storage::wal::{Wal, WalFsyncPolicy, WalRecovery, WalWriter};

// ---------------------------------------------------------------------------
// Retaining inner store
// ---------------------------------------------------------------------------

/// Inner `MapDataStore` that actually retains writes in a map.
///
/// Recovery must replay into a store that *keeps* what it is told, otherwise the
/// post-recovery read assertion is vacuous. `NullDataStore` discards every write,
/// so it cannot be used here — this is the model `ReplayStore` in `wal/mod.rs`
/// follows for the same reason.
#[derive(Default)]
struct RetainingStore {
    /// (map, key) -> value, or `None` for a tombstone.
    data: AsyncMutex<HashMap<(String, String), Option<RecordValue>>>,
}

impl RetainingStore {
    /// Returns `true` if the key is present and not tombstoned.
    async fn contains(&self, map: &str, key: &str) -> bool {
        matches!(
            self.data
                .lock()
                .await
                .get(&(map.to_string(), key.to_string())),
            Some(Some(_))
        )
    }

    /// Returns `true` if the key is present as a tombstone (removed).
    async fn is_tombstone(&self, map: &str, key: &str) -> bool {
        matches!(
            self.data
                .lock()
                .await
                .get(&(map.to_string(), key.to_string())),
            Some(None)
        )
    }
}

#[async_trait]
impl MapDataStore for RetainingStore {
    async fn add(
        &self,
        map: &str,
        key: &str,
        value: &RecordValue,
        _exp: i64,
        _now: i64,
    ) -> anyhow::Result<()> {
        self.data
            .lock()
            .await
            .insert((map.to_string(), key.to_string()), Some(value.clone()));
        Ok(())
    }

    async fn add_backup(
        &self,
        _map: &str,
        _key: &str,
        _value: &RecordValue,
        _exp: i64,
        _now: i64,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn remove(&self, map: &str, key: &str, _now: i64) -> anyhow::Result<()> {
        self.data
            .lock()
            .await
            .insert((map.to_string(), key.to_string()), None);
        Ok(())
    }

    async fn remove_backup(&self, _map: &str, _key: &str, _now: i64) -> anyhow::Result<()> {
        Ok(())
    }

    async fn load(&self, map: &str, key: &str) -> anyhow::Result<Option<RecordValue>> {
        Ok(self
            .data
            .lock()
            .await
            .get(&(map.to_string(), key.to_string()))
            .and_then(Clone::clone))
    }

    async fn load_all(
        &self,
        map: &str,
        keys: &[String],
    ) -> anyhow::Result<Vec<(String, RecordValue)>> {
        let guard = self.data.lock().await;
        let mut out = Vec::new();
        for key in keys {
            if let Some(Some(value)) = guard.get(&(map.to_string(), key.clone())) {
                out.push((key.clone(), value.clone()));
            }
        }
        Ok(out)
    }

    async fn remove_all(&self, map: &str, keys: &[String]) -> anyhow::Result<()> {
        let mut guard = self.data.lock().await;
        for key in keys {
            guard.insert((map.to_string(), key.clone()), None);
        }
        Ok(())
    }

    async fn enumerate_leaves(
        &self,
        _map: &str,
        _is_backup: bool,
        _sink: &mut dyn LeafSink,
    ) -> anyhow::Result<()> {
        // This store is only used as a WAL-recovery sink; the crash-safety
        // proptests never enumerate leaves from it, so an explicit empty
        // enumeration is the correct conscious body.
        Ok(())
    }

    async fn scan_values(
        &self,
        _map: &str,
        _is_backup: bool,
        _max_batch_cost: u64,
    ) -> anyhow::Result<ScanBatch> {
        Ok(ScanBatch::default())
    }

    async fn scan_values_batched(
        &self,
        _map: &str,
        _is_backup: bool,
        _cursor: ScanCursor,
        _max_batch_cost: u64,
    ) -> anyhow::Result<ScanBatch> {
        Ok(ScanBatch::default())
    }

    fn is_loadable(&self, _key: &str) -> bool {
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

    async fn flush_key(
        &self,
        _map: &str,
        _key: &str,
        _value: &RecordValue,
        _backup: bool,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn reset(&self) {}
}

// ---------------------------------------------------------------------------
// Op model
// ---------------------------------------------------------------------------

/// A single generated operation in the test sequence.
#[derive(Debug, Clone)]
enum Op {
    /// Store the value for `key` (overlapping keys exercise coalescing).
    Store { key: String, value_tag: u64 },
    /// Remove `key`.
    Remove { key: String },
}

const TEST_MAP: &str = "crash_map";

/// Build an LWW `RecordValue` with a monotonically increasing HLC so a later
/// store always wins the inner store's LWW merge during replay.
fn lww_value(value_tag: u64, hlc_millis: u64) -> RecordValue {
    RecordValue::Lww {
        value: Value::Int(i64::try_from(value_tag).unwrap_or(i64::MAX)),
        timestamp: Timestamp {
            millis: hlc_millis,
            counter: 0,
            node_id: "crash-test".to_string(),
        },
    }
}

/// Test write-behind config with a long delay/interval so the background flush
/// never runs during the test window — every acked op must survive on the WAL
/// alone, not because it leaked through to the inner store before the crash.
fn never_flush_config() -> WriteBehindConfig {
    WriteBehindConfig {
        write_delay_ms: 600_000,
        flush_interval_ms: 600_000,
        ..WriteBehindConfig::default()
    }
}

/// Final intended state of each key (last op wins) given a fully-acked sequence.
/// `Some(value_tag)` means the key should be present, `None` means removed.
fn expected_final_state(ops: &[Op]) -> HashMap<String, Option<u64>> {
    let mut state: HashMap<String, Option<u64>> = HashMap::new();
    for op in ops {
        match op {
            Op::Store { key, value_tag } => {
                state.insert(key.clone(), Some(*value_tag));
            }
            Op::Remove { key } => {
                state.insert(key.clone(), None);
            }
        }
    }
    state
}

/// The core scenario, parameterized by fsync policy and the number of ops that
/// are applied (and thus acked) before the crash.
///
/// Returns the set of keys that were acked-as-stored and acked-as-removed before
/// the drop so the caller can assert the no-acked-loss oracle.
async fn run_crash_scenario(
    ops: &[Op],
    crash_after: usize,
    policy: WalFsyncPolicy,
) -> (Arc<RetainingStore>, HashMap<String, Option<u64>>) {
    let dir = tempfile::tempdir().expect("tempdir");
    let wal_dir = dir.path().to_path_buf();

    // Pre-crash: build a real write-behind store over a real file WAL.
    let pre_crash_inner: Arc<dyn MapDataStore> = Arc::new(RetainingStore::default());
    let wal = WalWriter::new(wal_dir.clone(), policy).expect("WalWriter::new");
    let wal_dyn: Arc<dyn Wal> = Arc::clone(&wal) as Arc<dyn Wal>;
    let store = WriteBehindDataStore::new_with_wal(
        pre_crash_inner,
        never_flush_config(),
        Some(WalBootstrap {
            wal: wal_dyn,
            sequence_start: 1,
        }),
    );

    // Apply ops up to the crash point, recording the acked set per key. Each ack
    // (the store method returning Ok) means the WAL frame is already durable.
    let mut acked: HashMap<String, Option<u64>> = HashMap::new();
    let mut hlc_millis: u64 = 1;
    for op in ops.iter().take(crash_after) {
        match op {
            Op::Store { key, value_tag } => {
                let value = lww_value(*value_tag, hlc_millis);
                hlc_millis += 1;
                store
                    .add(TEST_MAP, key, &value, 0, 1000)
                    .await
                    .expect("add must ack");
                acked.insert(key.clone(), Some(*value_tag));
            }
            Op::Remove { key } => {
                store
                    .remove(TEST_MAP, key, 1000)
                    .await
                    .expect("remove must ack");
                acked.insert(key.clone(), None);
            }
        }
    }

    // Model the crash: drop the write-behind store. Its in-memory staging buffer
    // is lost; the temp WAL files on disk survive. We must NOT reuse the pre-crash
    // inner store — recovery replays into a fresh one so the test proves WAL
    // replay, not surviving in-memory state.
    drop(store);

    // Recover from the surviving on-disk WAL into a fresh inner store via the
    // real recovery path (empty partition list ⇒ auto-discover from the dir).
    let recovered_inner = Arc::new(RetainingStore::default());
    let recovery = WalRecovery::new(Arc::clone(&wal), Vec::new());
    recovery
        .run(Arc::clone(&recovered_inner) as Arc<dyn MapDataStore>)
        .await
        .expect("recovery must succeed on an intact WAL");

    // Keep the tempdir alive until after recovery has read the files.
    drop(dir);

    (recovered_inner, acked)
}

/// A process-wide multi-threaded Tokio runtime for the proptest bridge.
///
/// `proptest!` expands to a *synchronous* `#[test]`, so there is no ambient
/// Tokio runtime inside the property body — `Handle::current()` would panic with
/// "there is no reactor running". We therefore own a multi-threaded runtime here
/// and drive each async scenario on it. The `multi_thread` flavor is required so
/// the `block_in_place` + `Handle::block_on` bridge (CLAUDE.md §Proptest Async
/// Bridge) does not panic, which it does on a single-threaded runtime.
static PROPTEST_RUNTIME: std::sync::LazyLock<tokio::runtime::Runtime> =
    std::sync::LazyLock::new(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("build multi-thread runtime for proptest async bridge")
    });

/// Run an async closure to completion from inside a synchronous proptest body.
///
/// Proptest closures are synchronous but the scenario is async. The bridge uses
/// `block_in_place` + `Handle::block_on` per CLAUDE.md §Proptest Async Bridge,
/// driven on the process-wide multi-threaded runtime above (`block_in_place`
/// panics on a single-threaded runtime).
fn block_on_async<F: std::future::Future>(fut: F) -> F::Output {
    let handle = PROPTEST_RUNTIME.handle().clone();
    let _guard = handle.enter();
    block_in_place(|| Handle::current().block_on(fut))
}

// ---------------------------------------------------------------------------
// Strategies
// ---------------------------------------------------------------------------

/// Generates a sequence of store/remove ops over a small key space so keys
/// overlap (exercising coalescing) and a crash point within the sequence.
fn ops_and_crash_point() -> impl Strategy<Value = (Vec<Op>, usize)> {
    // Small key space (k0..k4) guarantees overlap across a sequence of ops.
    let op = prop_oneof![
        (0u8..5, 0u64..1000).prop_map(|(k, tag)| Op::Store {
            key: format!("k{k}"),
            value_tag: tag,
        }),
        (0u8..5).prop_map(|k| Op::Remove {
            key: format!("k{k}"),
        }),
    ];
    prop::collection::vec(op, 1..30).prop_flat_map(|ops| {
        let len = ops.len();
        // Crash anywhere from 0 acked ops up to all of them.
        (Just(ops), 0..=len)
    })
}

// ---------------------------------------------------------------------------
// AC1 + AC3: strict no-acked-loss invariant under PerOp across many schedules
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

    /// Across many randomized op sequences (overlapping keys, mixed store/remove)
    /// and crash points, dropping a real write-behind store while the temp WAL
    /// survives and recovering into a fresh inner store loses no acked op.
    #[test]
    fn no_acked_op_lost_after_crash_perop((ops, crash_after) in ops_and_crash_point()) {
        let (recovered, acked) =
            block_on_async(run_crash_scenario(&ops, crash_after, WalFsyncPolicy::PerOp));

        block_on_async(async {
            for (key, expected) in &acked {
                match expected {
                    // An acked store must be present after recovery.
                    Some(_) => prop_assert!(
                        recovered.contains(TEST_MAP, key).await,
                        "acked store of key={key} lost after recovery"
                    ),
                    // An acked remove must be reflected as a tombstone after
                    // recovery (the key must not resurface as present).
                    None => prop_assert!(
                        recovered.is_tombstone(TEST_MAP, key).await
                            && !recovered.contains(TEST_MAP, key).await,
                        "acked remove of key={key} not honored after recovery"
                    ),
                }
            }
            Ok(())
        })?;
    }
}

// ---------------------------------------------------------------------------
// AC1 (RED) + AC3: acked == durable when the policy is derived FROM THE ENV
// STRING the harness actually passes (`"perop"`), not the `WalFsyncPolicy::PerOp`
// literal. This closes the green-bypasses-env gap: the proptest above passes the
// enum directly, so it stays green even if `from_str` is broken. This test goes
// RED on revert of the `from_str` normalization — a reverted parser makes
// `"perop"` either fail to parse (panic at `.expect`) or, if a fallback is
// reintroduced, downgrade to Batched, whose ~10ms fsync window drops the
// last-appended-but-not-fsynced frame as a one-behind loss after the crash.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn acked_equals_durable_from_env_string_perop() {
    // Derive the policy the way the soak harness does: from the env *string*, so
    // the test exercises the parser, not a hand-picked enum variant. A reverted
    // `from_str` cannot satisfy this expect for the `perop` spelling.
    let policy: WalFsyncPolicy = "perop"
        .parse()
        .expect("the env-string spelling used by the soak harness must parse to a durable policy");
    assert_eq!(
        policy,
        WalFsyncPolicy::PerOp,
        "the harness env string must resolve to the durable PerOp policy, \
         not a downgraded default"
    );

    // A load-shaped sequence: many overlapping keys each incremented several
    // times, so a one-behind loss (actual = expected - 1) would surface as the
    // last store of a key going missing after the crash — exactly the soak's
    // observed signature.
    let mut ops = Vec::new();
    for round in 0..8u64 {
        for k in 0..32u8 {
            ops.push(Op::Store {
                key: format!("k{k}"),
                value_tag: round * 100 + u64::from(k),
            });
        }
    }
    let crash_after = ops.len();

    // Crash after EVERY op is acked, with NO pre-flush drain: the scenario drops
    // the store immediately after the last ack, so every acked write must survive
    // on the WAL alone (never_flush_config keeps the background flush from leaking
    // writes to the inner store before the crash).
    let (recovered, acked) = run_crash_scenario(&ops, crash_after, policy).await;

    // Zero one-behind: every acked store must be present after recovery, with its
    // final value_tag intact.
    let expected = expected_final_state(&ops);
    let mut missing = Vec::new();
    for (key, state) in &expected {
        match state {
            Some(_) => {
                if !recovered.contains(TEST_MAP, key).await {
                    missing.push(key.clone());
                }
            }
            None => {
                if recovered.contains(TEST_MAP, key).await {
                    missing.push(format!("{key} (should be tombstoned)"));
                }
            }
        }
    }
    assert!(
        missing.is_empty(),
        "acked == durable violated under env-string PerOp: {} acked writes lost \
         after crash recovery (one-behind regression): {:?}",
        missing.len(),
        &missing[..missing.len().min(10)]
    );
    assert_eq!(
        acked.len(),
        expected.len(),
        "every distinct key must have been acked before the crash"
    );
}

// ---------------------------------------------------------------------------
// AC2(a): clean/strict recovery replays all acked ops (full sequence acked).
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn ac2a_clean_recovery_replays_all_acked_ops() {
    let ops = vec![
        Op::Store {
            key: "a".to_string(),
            value_tag: 1,
        },
        Op::Store {
            key: "b".to_string(),
            value_tag: 2,
        },
        Op::Remove {
            key: "a".to_string(),
        },
        Op::Store {
            key: "c".to_string(),
            value_tag: 3,
        },
    ];
    // crash_after == ops.len() ⇒ the whole sequence is acked; the intact log
    // decodes as Complete/CleanEof and every acked op must replay.
    let (recovered, _acked) = run_crash_scenario(&ops, ops.len(), WalFsyncPolicy::PerOp).await;

    let expected = expected_final_state(&ops);
    for (key, state) in expected {
        match state {
            Some(_) => assert!(
                recovered.contains(TEST_MAP, &key).await,
                "key={key} expected present after clean recovery"
            ),
            None => assert!(
                !recovered.contains(TEST_MAP, &key).await,
                "key={key} expected absent (removed) after clean recovery"
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// AC2(b): a physically-truncated WAL tail is tolerated — recovery replays the
// intact prefix, WARN-logs, and returns Ok (the Batched acked-but-not-yet-
// fsynced tail scenario).
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn ac2b_truncated_tail_tolerated() {
    let dir = tempfile::tempdir().unwrap();
    let wal_dir = dir.path().to_path_buf();

    // Write two complete frames into the SAME partition file, then physically
    // truncate the file's tail to simulate an acked-but-not-yet-fsynced write
    // lost on crash. Two writes to the same key land in one partition file
    // (partition is derived from map:key), so the file holds frame1 (prefix) then
    // frame2 (tail) — truncating its tail leaves an intact, recoverable prefix.
    let key = "same_partition_key";
    {
        let inner: Arc<dyn MapDataStore> = Arc::new(RetainingStore::default());
        let wal = WalWriter::new(wal_dir.clone(), WalFsyncPolicy::Batched).unwrap();
        let wal_dyn: Arc<dyn Wal> = Arc::clone(&wal) as Arc<dyn Wal>;
        let store = WriteBehindDataStore::new_with_wal(
            inner,
            never_flush_config(),
            Some(WalBootstrap {
                wal: wal_dyn,
                sequence_start: 1,
            }),
        );
        store
            .add(TEST_MAP, key, &lww_value(1, 1), 0, 1000)
            .await
            .unwrap();
        store
            .add(TEST_MAP, key, &lww_value(2, 2), 0, 1000)
            .await
            .unwrap();
        drop(store);
    }

    // Locate the single partition file and lop off its last few bytes.
    let log_path = std::fs::read_dir(&wal_dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                n.strip_prefix("partition-")
                    .and_then(|r| r.strip_suffix(".log"))
                    .is_some()
            })
        })
        .expect("a partition log file must exist");

    let bytes = std::fs::read(&log_path).unwrap();
    assert!(bytes.len() > 4, "log must hold at least one full frame");
    // Truncate the last 3 bytes so the final frame is incomplete on disk.
    std::fs::write(&log_path, &bytes[..bytes.len() - 3]).unwrap();

    // Recovery must tolerate the truncated tail (Ok) and replay the prefix.
    let recovered = Arc::new(RetainingStore::default());
    let wal = WalWriter::new(wal_dir.clone(), WalFsyncPolicy::Batched).unwrap();
    let recovery = WalRecovery::new(Arc::clone(&wal), Vec::new());
    let result = recovery
        .run(Arc::clone(&recovered) as Arc<dyn MapDataStore>)
        .await;
    assert!(
        result.is_ok(),
        "truncated tail must be tolerated, not fatal: {result:?}"
    );
    assert!(
        recovered.contains(TEST_MAP, key).await,
        "the intact prefix frame must be replayed despite a truncated tail"
    );
}

// ---------------------------------------------------------------------------
// AC2(c): mid-file corruption is FATAL — corrupting a complete frame's bytes
// makes WalRecovery::run return Err (the caller exits non-zero).
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn ac2c_mid_file_corruption_is_fatal() {
    use crate::storage::wal::format;

    let dir = tempfile::tempdir().unwrap();
    let wal_dir = dir.path().to_path_buf();

    // Write two complete frames into the SAME partition file, then flip a payload
    // byte in the FIRST frame so it is mid-file corruption (a complete-but-wrong
    // frame followed by a valid frame, not a truncated tail). Two writes to the
    // same key land in one partition file, so the first frame is genuinely
    // mid-file. decode_all classifies this as Corruption ⇒ recovery must bail.
    let key = "same_partition_key";
    {
        let inner: Arc<dyn MapDataStore> = Arc::new(RetainingStore::default());
        let wal = WalWriter::new(wal_dir.clone(), WalFsyncPolicy::PerOp).unwrap();
        let wal_dyn: Arc<dyn Wal> = Arc::clone(&wal) as Arc<dyn Wal>;
        let store = WriteBehindDataStore::new_with_wal(
            inner,
            never_flush_config(),
            Some(WalBootstrap {
                wal: wal_dyn,
                sequence_start: 1,
            }),
        );
        store
            .add(TEST_MAP, key, &lww_value(1, 1), 0, 1000)
            .await
            .unwrap();
        store
            .add(TEST_MAP, key, &lww_value(2, 2), 0, 1000)
            .await
            .unwrap();
        drop(store);
    }

    let log_path = std::fs::read_dir(&wal_dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                n.strip_prefix("partition-")
                    .and_then(|r| r.strip_suffix(".log"))
                    .is_some()
            })
        })
        .expect("a partition log file must exist");

    let mut bytes = std::fs::read(&log_path).unwrap();
    // Flip a byte in the first frame's payload (just past the frame header) so
    // its stored CRC no longer matches — a complete-frame corruption mid-file.
    let corrupt_at = format::FRAME_HEADER_LEN;
    assert!(bytes.len() > corrupt_at, "log must hold a full first frame");
    bytes[corrupt_at] ^= 0xFF;
    std::fs::write(&log_path, &bytes).unwrap();

    let recovered = Arc::new(RetainingStore::default());
    let wal = WalWriter::new(wal_dir.clone(), WalFsyncPolicy::PerOp).unwrap();
    let recovery = WalRecovery::new(Arc::clone(&wal), Vec::new());
    let result = recovery
        .run(Arc::clone(&recovered) as Arc<dyn MapDataStore>)
        .await;
    assert!(
        result.is_err(),
        "mid-file corruption must be fatal (recovery returns Err so the caller exits non-zero)"
    );
}

// ---------------------------------------------------------------------------
// AC4: multi-restart no-acked-loss. Write k0 → drop → recover (k0 present) →
// rebuild a NEW store over a NEW WalWriter on the SAME wal_dir seeded via
// max_observed_sequence()+1 → write k1 → drop → recover → BOTH k0 and k1
// present. Without the seed fix the rebuilt counter resets to a low value and
// k1 reuses a sequence at or below the watermark recovery wrote for the shared
// partition, so the second recovery drops it — this is the only schedule that
// exercises Symptom 2 (counter-resume), not just the fresh-node off-by-one.
// ---------------------------------------------------------------------------

/// Finds a key distinct from `first_key` that maps to the **same** partition as
/// `first_key` under `partition_for(map, key)`.
///
/// k0 and k1 must share a partition so that the watermark recovery advances for
/// k0's partition is the exact value the rebuilt counter would (without the
/// seed) re-hand to k1 — making the second recovery drop k1.
fn colliding_key(map: &str, first_key: &str) -> String {
    let target = partition_of(map, first_key);
    for i in 0..1_000_000u64 {
        let candidate = format!("collide-{i}");
        if candidate != first_key && partition_of(map, &candidate) == target {
            return candidate;
        }
    }
    panic!("no colliding key found for partition {target}");
}

/// Mirrors `write_behind::partition_for` (private): deterministic partition id.
fn partition_of(map: &str, key: &str) -> u32 {
    let combined = format!("{map}:{key}");
    topgun_core::fnv1a_hash(&combined) % topgun_core::PARTITION_COUNT
}

/// Recovers from the on-disk WAL at `wal_dir` into `inner`.
///
/// `inner` models the durable backend (redb): it persists across restart cycles,
/// so an applied-then-compacted entry stays present even though it is no longer
/// in the WAL tail. Recovery replays only the unapplied WAL entries on top.
async fn recover_into(wal_dir: &std::path::Path, inner: Arc<RetainingStore>) {
    let wal = WalWriter::new(wal_dir.to_path_buf(), WalFsyncPolicy::PerOp).unwrap();
    let recovery = WalRecovery::new(Arc::clone(&wal), Vec::new());
    recovery
        .run(Arc::clone(&inner) as Arc<dyn MapDataStore>)
        .await
        .expect("recovery must succeed on an intact WAL");
}

#[tokio::test(flavor = "multi_thread")]
async fn ac4_acked_writes_survive_multiple_restart_cycles() {
    let dir = tempfile::tempdir().unwrap();
    let wal_dir = dir.path().to_path_buf();

    let k0 = "k0".to_string();
    // k1 must share a partition with k0 so the watermark collision bites.
    let k1 = colliding_key(TEST_MAP, &k0);
    assert_eq!(
        partition_of(TEST_MAP, &k0),
        partition_of(TEST_MAP, &k1),
        "k0 and k1 must collide on one partition to exercise the watermark resume"
    );

    // The durable backend (modeled by redb) persists across restart cycles, so
    // an applied-then-compacted entry stays present even once it leaves the WAL.
    let durable: Arc<RetainingStore> = Arc::new(RetainingStore::default());

    // --- Cycle 1: write k0, drop before flush ---
    {
        let inner: Arc<dyn MapDataStore> = Arc::new(RetainingStore::default());
        let wal = WalWriter::new(wal_dir.clone(), WalFsyncPolicy::PerOp).unwrap();
        let wal_dyn: Arc<dyn Wal> = Arc::clone(&wal) as Arc<dyn Wal>;
        // Fresh node: seed 1 (no WAL observed yet).
        let store = WriteBehindDataStore::new_with_wal(
            inner,
            never_flush_config(),
            Some(WalBootstrap {
                wal: wal_dyn,
                sequence_start: 1,
            }),
        );
        store
            .add(TEST_MAP, &k0, &lww_value(1, 1), 0, 1000)
            .await
            .expect("k0 add must ack");
        drop(store);
    }

    // First recovery into the durable store: k0 must be present. This also
    // advances the .applied watermark for k0's partition and compacts that log
    // to empty.
    recover_into(&wal_dir, Arc::clone(&durable)).await;
    assert!(
        durable.contains(TEST_MAP, &k0).await,
        "k0 must be present after the first recovery (Symptom 1 path)"
    );

    // --- Cycle 2: rebuild a NEW store over a NEW WalWriter on the SAME wal_dir,
    // seeded from the WAL so the live counter resumes past the watermark. ---
    {
        let inner: Arc<dyn MapDataStore> = Arc::new(RetainingStore::default());
        let wal = WalWriter::new(wal_dir.clone(), WalFsyncPolicy::PerOp).unwrap();
        // Seed AFTER recovery so it observes the advanced watermark (the
        // compaction trap: k0's log is now empty but its sidecar persists).
        let sequence_start = wal
            .max_observed_sequence()
            .await
            .expect("max_observed_sequence")
            + 1;
        let wal_dyn: Arc<dyn Wal> = Arc::clone(&wal) as Arc<dyn Wal>;
        let store = WriteBehindDataStore::new_with_wal(
            inner,
            never_flush_config(),
            Some(WalBootstrap {
                wal: wal_dyn,
                sequence_start,
            }),
        );
        store
            .add(TEST_MAP, &k1, &lww_value(2, 2), 0, 1000)
            .await
            .expect("k1 add must ack");
        drop(store);
    }

    // Second recovery into the SAME durable store: k0 stayed there from cycle 1,
    // and k1 must now replay from the WAL. Without the seed fix k1 reuses a
    // sequence <= the watermark recovery wrote for the shared partition, so the
    // recovery filter drops it here and the assertion fails.
    recover_into(&wal_dir, Arc::clone(&durable)).await;
    assert!(
        durable.contains(TEST_MAP, &k0).await,
        "k0 must still be present after the second recovery"
    );
    assert!(
        durable.contains(TEST_MAP, &k1).await,
        "k1 (written post-restart) must survive the second recovery — \
         the counter must resume past the persisted watermark"
    );

    drop(dir);
}

// ---------------------------------------------------------------------------
// Compaction trap: max_observed_sequence must read the .applied sidecar of a
// partition whose log was compacted to empty (so its log max is 0) but whose
// watermark W persists. Returned max must be >= W or the seed lands below W and
// a post-restart write to that partition is dropped.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn max_observed_sequence_reads_watermark_of_compacted_empty_partition() {
    use crate::storage::wal::{WalEntry, WalOp, WalStorePayload};

    let dir = tempfile::tempdir().unwrap();
    let wal_dir = dir.path().to_path_buf();
    let watermark: u64 = 7;

    let wal = WalWriter::new(wal_dir.clone(), WalFsyncPolicy::PerOp).unwrap();
    // Append an entry, then mark it applied: mark_applied writes the .applied
    // sidecar (watermark) AND compacts the log to retain only seq > watermark,
    // leaving partition 3's log empty while its sidecar holds W.
    let entry = WalEntry {
        map: "m".to_string(),
        key: "k".to_string(),
        op: WalOp::Store {
            value: WalStorePayload::Record(RecordValue::Lww {
                value: Value::Int(1),
                timestamp: Timestamp {
                    millis: 1,
                    counter: 0,
                    node_id: "n".to_string(),
                },
            }),
            expiration_time: None,
        },
        timestamp: Some(Timestamp {
            millis: 1,
            counter: 0,
            node_id: "n".to_string(),
        }),
        sequence: watermark,
    };
    wal.append(3, &entry).await.unwrap();
    wal.mark_applied(3, watermark).await.unwrap();

    // Sanity: the log is now compacted to empty (no entries above the watermark).
    let unapplied = wal.unapplied(3).await.unwrap();
    assert!(
        unapplied.is_empty(),
        "log must be compacted-empty after mark_applied"
    );

    // A fresh WalWriter (cold start) must still observe the sidecar watermark.
    let cold = WalWriter::new(wal_dir.clone(), WalFsyncPolicy::PerOp).unwrap();
    let max = cold.max_observed_sequence().await.unwrap();
    assert!(
        max >= watermark,
        "max_observed_sequence must be >= the .applied watermark {watermark} of a \
         compacted-empty partition, got {max}"
    );
}

// ---------------------------------------------------------------------------
// Concurrent same-partition durability under crash (F1 / visibility class).
//
// The other crash scenarios drive the write path single-threaded-sequentially:
// every op is appended, then the store is dropped. That schedule can never see
// the seal/rotate race (mark_applied sealing the active segment while an append
// interleaves) — it slipped past the suite and `kill -9` cannot see it either
// (the page cache survives a process kill, and steady-state entries usually also
// reached the inner store before the kill).
//
// This test runs append and compaction CONCURRENTLY on ONE partition through the
// production WriteBehindDataStore write path, then models the crash (drop), and
// asserts against a FRESH durable inner store recovered from the on-disk WAL —
// NOT against the OS-cached WAL file. With never-flush config every acked write
// lives only in the WAL until the crash, so a seal/rotate that drops a
// concurrently-appended frame is a lost acked write that recovery cannot find.
//
// `mark_applied(partition, 0)` is the fault injection: it retains every entry
// (watermark 0) yet still seals the active segment and rotates a fresh one in,
// exactly as the real flush loop's mark_applied does while clients keep writing
// the same partition. Driving it directly (rather than via the flush loop) is
// what makes the race deterministic instead of timing-dependent.
//
// Negative control: reverting mark_applied to the pre-PR#50 unlocked
// read/rewrite/rename shape (or sealing without freezing the active frames first)
// makes this test drop a large fraction of acked writes and fail.
// ---------------------------------------------------------------------------

/// Generates `count` distinct keys that all map to `partition` under
/// `partition_of`, so appends and compaction contend on a single WAL file.
fn keys_for_partition(map: &str, partition: u32, count: usize) -> Vec<String> {
    let mut out = Vec::with_capacity(count);
    let mut i = 0u64;
    while out.len() < count {
        let candidate = format!("cp-{i}");
        if partition_of(map, &candidate) == partition {
            out.push(candidate);
        }
        i += 1;
    }
    out
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_compaction_loses_no_acked_write_across_crash() {
    let dir = tempfile::tempdir().unwrap();
    let wal_dir = dir.path().to_path_buf();

    // One partition; every key collides onto it so append races compaction on
    // the same file. 2000 ops reproduces the pre-fix loss reliably (the audit's
    // repro dropped 1864/2000).
    let total = 2000usize;
    let partition = partition_of(TEST_MAP, "anchor");
    let keys = keys_for_partition(TEST_MAP, partition, total);

    // Production write path; never-flush so acked writes live ONLY in the WAL
    // until the crash (no leak to the inner store before the drop). None policy
    // also maximally stresses the append()-flush visibility path (PR #49).
    let pre_crash_inner: Arc<dyn MapDataStore> = Arc::new(RetainingStore::default());
    let wal = WalWriter::new(wal_dir.clone(), WalFsyncPolicy::None).expect("WalWriter::new");
    let store = WriteBehindDataStore::new_with_wal(
        pre_crash_inner,
        never_flush_config(),
        Some(WalBootstrap {
            wal: Arc::clone(&wal) as Arc<dyn Wal>,
            sequence_start: 1,
        }),
    );

    // Appender: each add returns Ok (an ack) ⇒ its WAL frame is durable per the
    // append-before-ack contract.
    let appender = {
        let store = Arc::clone(&store);
        let keys = keys.clone();
        tokio::spawn(async move {
            for (i, key) in keys.iter().enumerate() {
                let tag = u64::try_from(i).unwrap() + 1;
                store
                    .add(TEST_MAP, key, &lww_value(tag, tag), 0, 1000)
                    .await
                    .expect("add must ack");
            }
        })
    };

    // Compactor: hammer mark_applied on the SAME partition, racing the appender's
    // writes to the active log file.
    let compactor = {
        let wal = Arc::clone(&wal);
        tokio::spawn(async move {
            for _ in 0..total {
                wal.mark_applied(partition, 0).await.expect("mark_applied");
                tokio::task::yield_now().await;
            }
        })
    };

    appender.await.unwrap();
    compactor.await.unwrap();

    // Crash: the in-memory staging buffer is gone; the on-disk WAL survives.
    drop(store);

    // Recover into a FRESH durable inner store via the real recovery path and
    // assert against THAT — not the OS-cached WAL file.
    let recovered: Arc<RetainingStore> = Arc::new(RetainingStore::default());
    let recovery = WalRecovery::new(Arc::clone(&wal), Vec::new());
    recovery
        .run(Arc::clone(&recovered) as Arc<dyn MapDataStore>)
        .await
        .expect("recovery must succeed on an intact WAL");
    drop(dir);

    // Every acked write must survive. Pre-fix the compaction race silently drops
    // most concurrently-appended frames and this fails (negative control).
    let mut missing = Vec::new();
    for key in &keys {
        if !recovered.contains(TEST_MAP, key).await {
            missing.push(key.clone());
        }
    }
    assert!(
        missing.is_empty(),
        "concurrent compaction lost {} of {total} acked writes across the crash \
         (first few: {:?})",
        missing.len(),
        &missing[..missing.len().min(10)]
    );
}

// ---------------------------------------------------------------------------
// Crash on a PARTIAL FINAL (active) segment under concurrent same-partition
// load (G3 / AC6).
//
// A SIGKILL can land mid-append: the active segment ends with a half-written
// frame whose bytes reached the page cache but whose final frame was never
// completed. Under segment rotation this is the ONLY place a torn tail may
// legitimately appear — sealed segments are fsynced whole before they join the
// sealed set, so a torn tail on a non-last segment is fatal corruption.
//
// This test drives concurrent appends + seal/rotate on ONE partition (so the
// partition ends with several sealed segments plus a populated active segment),
// models the crash by dropping the store, then PHYSICALLY TRUNCATES the active
// (highest-first_seq) segment's tail to model the half-written final frame.
// Recovery into a FRESH durable inner store must:
//   - succeed (the truncated tail on the active segment is tolerated, WARN + Ok);
//   - replay every acked write whose frame is in the intact prefix (the sealed
//     segments + the active segment up to the torn byte) — no acked op below the
//     torn frame is lost.
//
// The oracle is conservative on the boundary: only the single key whose frame was
// torn off may be absent. We assert the prefix is fully recovered by checking
// that recovery is Ok and that the overwhelming majority (all but at most the
// torn tail's key) survive — and crucially that a key written EARLY (deep in a
// sealed segment) is always present.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[allow(clippy::too_many_lines)]
async fn crash_on_partial_active_segment_recovers_intact_prefix() {
    use crate::storage::wal::segment::parse_segment_filename;

    let dir = tempfile::tempdir().unwrap();
    let wal_dir = dir.path().to_path_buf();

    let total = 1000usize;
    let partition = partition_of(TEST_MAP, "anchor");
    let keys = keys_for_partition(TEST_MAP, partition, total);

    // Production write path; never-flush so acked writes live ONLY in the WAL
    // until the crash. PerOp fsync so each frame is fully on disk at ack time —
    // we then synthesize the torn FINAL frame by truncating, not by relying on a
    // partial OS flush.
    let pre_crash_inner: Arc<dyn MapDataStore> = Arc::new(RetainingStore::default());
    let wal = WalWriter::new(wal_dir.clone(), WalFsyncPolicy::PerOp).expect("WalWriter::new");
    let store = WriteBehindDataStore::new_with_wal(
        pre_crash_inner,
        never_flush_config(),
        Some(WalBootstrap {
            wal: Arc::clone(&wal) as Arc<dyn Wal>,
            sequence_start: 1,
        }),
    );

    // Appender: each add returns Ok ⇒ its WAL frame is durable.
    let appender = {
        let store = Arc::clone(&store);
        let keys = keys.clone();
        tokio::spawn(async move {
            for (i, key) in keys.iter().enumerate() {
                let tag = u64::try_from(i).unwrap() + 1;
                store
                    .add(TEST_MAP, key, &lww_value(tag, tag), 0, 1000)
                    .await
                    .expect("add must ack");
            }
        })
    };

    // Compactor: seal/rotate the same partition repeatedly so the partition ends
    // with several SEALED segments plus a populated ACTIVE segment — the layout
    // where "torn final segment" is meaningfully distinct from "torn whole log".
    let compactor = {
        let wal = Arc::clone(&wal);
        tokio::spawn(async move {
            for _ in 0..total {
                wal.mark_applied(partition, 0).await.expect("mark_applied");
                tokio::task::yield_now().await;
            }
        })
    };

    appender.await.unwrap();
    // Stop the compactor BEFORE the final tail batch so no rotation can run
    // between those appends and the truncation — the seal/rotate race already ran
    // for the bulk of the load above; the tail batch deterministically populates
    // the active segment so a torn final frame is guaranteed to exist there
    // (a last-moment seal would otherwise leave the active segment empty, which is
    // a legitimate state but not the "torn active segment" this test targets).
    compactor.await.unwrap();

    // Final deterministic tail batch through the real write path, landing in the
    // current active segment with no concurrent rotation. The last of these is the
    // frame the truncation tears off.
    let tail_keys = keys_for_partition(TEST_MAP, partition, total + 8)
        .split_off(total)
        .into_iter()
        .collect::<Vec<_>>();
    for (i, key) in tail_keys.iter().enumerate() {
        let tag = u64::try_from(total + i).unwrap() + 1;
        store
            .add(TEST_MAP, key, &lww_value(tag, tag), 0, 1000)
            .await
            .expect("tail add must ack");
    }

    // Crash: in-memory staging gone; on-disk segments survive.
    drop(store);
    // Drop the WAL handle so its open active-segment file descriptor is closed
    // before we truncate on disk (avoids racing an in-flight fsync timer).
    drop(wal);

    // Identify the ACTIVE (highest-first_seq) segment of the partition and lop off
    // its last few bytes to model a half-written final frame at SIGKILL time.
    let mut segments: Vec<(u64, std::path::PathBuf)> = std::fs::read_dir(&wal_dir)
        .unwrap()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            parse_segment_filename(&name)
                .and_then(|(p, first_seq)| (p == partition).then(|| (first_seq, e.path())))
        })
        .collect();
    segments.sort_by_key(|(first_seq, _)| *first_seq);
    assert!(
        segments.len() >= 2,
        "test must produce a multi-segment partition (sealed + active); got {}",
        segments.len()
    );
    let (_active_first_seq, active_path) = segments.last().cloned().unwrap();

    let active_bytes = std::fs::read(&active_path).unwrap();
    assert!(
        active_bytes.len() > 4,
        "active segment must hold the tail batch's frames to truncate a torn tail"
    );
    // Truncate the last 3 bytes so the active segment's final frame is incomplete
    // on disk — the classic torn-tail SIGKILL signature.
    std::fs::write(&active_path, &active_bytes[..active_bytes.len() - 3]).unwrap();

    // Recover into a FRESH durable inner store via the real recovery path. A torn
    // tail on the ACTIVE (last) segment must be tolerated (Ok), not fatal.
    let recovered: Arc<RetainingStore> = Arc::new(RetainingStore::default());
    let wal2 = WalWriter::new(wal_dir.clone(), WalFsyncPolicy::PerOp).unwrap();
    let recovery = WalRecovery::new(Arc::clone(&wal2), Vec::new());
    let result = recovery
        .run(Arc::clone(&recovered) as Arc<dyn MapDataStore>)
        .await;
    assert!(
        result.is_ok(),
        "a torn tail on the active segment must be tolerated, not fatal: {result:?}"
    );

    // Every bulk-phase acked write lives in a sealed segment or in the active
    // segment's intact prefix below the torn frame — so ALL must survive. Only the
    // single torn final frame (the last tail-batch key) may be absent.
    let mut missing_bulk = Vec::new();
    for key in &keys {
        if !recovered.contains(TEST_MAP, key).await {
            missing_bulk.push(key.clone());
        }
    }
    assert!(
        missing_bulk.is_empty(),
        "torn active-segment tail lost {} bulk-phase acked writes that should be in \
         sealed segments or the intact active prefix (first few: {:?})",
        missing_bulk.len(),
        &missing_bulk[..missing_bulk.len().min(10)]
    );

    // The tail batch landed in the active segment; only its torn final frame may
    // be missing. Every tail key except the last must be recovered from the intact
    // active prefix.
    let mut missing_tail = Vec::new();
    for key in &tail_keys {
        if !recovered.contains(TEST_MAP, key).await {
            missing_tail.push(key.clone());
        }
    }
    assert!(
        missing_tail.len() <= 1,
        "torn active-segment tail lost {} acked writes beyond the single torn final \
         frame (first few: {:?})",
        missing_tail.len(),
        &missing_tail[..missing_tail.len().min(10)]
    );

    // An EARLY key (frame deep in a sealed segment, far from the torn tail) must
    // always survive — proves recovery replayed across all sealed segments, not
    // just the active one.
    assert!(
        recovered.contains(TEST_MAP, &keys[0]).await,
        "the earliest acked write (deep in a sealed segment) must survive a torn \
         active-segment tail"
    );

    drop(dir);
}

// ---------------------------------------------------------------------------
// OR-Map durability across recovery (SPEC-333)
// ---------------------------------------------------------------------------

/// Build an HLC timestamp for the OR-Map durability tests.
fn or_ts(millis: u64) -> Timestamp {
    Timestamp {
        millis,
        counter: 0,
        node_id: "sf333".to_string(),
    }
}

/// A populated OR-Map record with two live adds and a tombstone, exercising the
/// `records` + `tombstones` shape the WAL must carry losslessly.
fn or_record() -> RecordValue {
    RecordValue::OrMap {
        records: vec![
            OrMapEntry {
                value: Value::String("alpha".to_string()),
                tag: "tag-a".to_string(),
                timestamp: or_ts(1),
            },
            OrMapEntry {
                value: Value::Int(42),
                tag: "tag-b".to_string(),
                timestamp: or_ts(2),
            },
        ],
        tombstones: vec!["gone-tag".to_string()],
    }
}

/// AC2 — an acked OR-Map add that survives only on the WAL (never flushed to the
/// inner store before the crash) must replay as its exact OR content, not as a
/// `Value::Null`/LWW placeholder.
///
/// Red on revert of the full-`RecordValue` WAL encode/replay: the pre-fix code
/// encoded every non-LWW value as `Value::Null` and replayed every `Store` as
/// `RecordValue::Lww`, so the recovered value would be `Lww(Null)`, not the OR
/// record — this assertion would fail.
#[tokio::test]
async fn ac2_wal_only_recovery_preserves_ormap_adds() {
    let dir = tempfile::tempdir().expect("tempdir");
    let wal_dir = dir.path().to_path_buf();

    // never_flush_config ⇒ the OR add never reaches the inner store before the
    // crash; its only durable record is the WAL frame.
    let pre_crash_inner: Arc<dyn MapDataStore> = Arc::new(RetainingStore::default());
    let wal = WalWriter::new(wal_dir.clone(), WalFsyncPolicy::PerOp).expect("WalWriter::new");
    let wal_dyn: Arc<dyn Wal> = Arc::clone(&wal) as Arc<dyn Wal>;
    let store = WriteBehindDataStore::new_with_wal(
        pre_crash_inner,
        never_flush_config(),
        Some(WalBootstrap {
            wal: wal_dyn,
            sequence_start: 1,
        }),
    );

    let or_value = or_record();
    store
        .add(TEST_MAP, "ork-1", &or_value, 0, 1000)
        .await
        .expect("OR add must ack");

    // Crash: drop the write-behind store; only the on-disk WAL survives.
    drop(store);

    let recovered = Arc::new(RetainingStore::default());
    let recovery = WalRecovery::new(Arc::clone(&wal), Vec::new());
    recovery
        .run(Arc::clone(&recovered) as Arc<dyn MapDataStore>)
        .await
        .expect("recovery must succeed on an intact WAL");

    let got = recovered
        .load(TEST_MAP, "ork-1")
        .await
        .unwrap()
        .expect("acked OR add must survive WAL recovery (lost as Value::Null pre-fix)");
    assert_eq!(
        got, or_value,
        "recovered OR content must equal the acked OR record exactly"
    );

    drop(dir);
}

/// AC4 — a legacy bare-`Value` WAL frame (the shape older servers wrote) must
/// still decode and replay as LWW; the server must never refuse to start on an
/// old WAL.
///
/// `WalStorePayload::Legacy(v)` serializes (untagged) to the exact bare-`Value`
/// wire shape an older server emitted for `WalOp::Store { value: Value }`, so
/// appending it reproduces a pre-existing on-disk frame. Red on revert of the
/// untagged backward-compat decode: a tag-renamed variant would fail to decode
/// the `"store"`-tagged legacy frame and recovery would treat it as corruption.
#[tokio::test]
async fn ac4_legacy_bare_value_wal_frame_replays_as_lww() {
    use crate::storage::wal::{WalEntry, WalOp, WalStorePayload};

    let dir = tempfile::tempdir().expect("tempdir");
    let wal_dir = dir.path().to_path_buf();
    let wal = WalWriter::new(wal_dir.clone(), WalFsyncPolicy::PerOp).expect("WalWriter::new");

    let legacy_entry = WalEntry {
        map: TEST_MAP.to_string(),
        key: "legacy-k".to_string(),
        op: WalOp::Store {
            value: WalStorePayload::Legacy(Value::String("legacy-val".to_string())),
            expiration_time: None,
        },
        timestamp: Some(or_ts(5)),
        sequence: 1,
    };
    wal.append(0, &legacy_entry)
        .await
        .expect("append legacy frame");

    let recovered = Arc::new(RetainingStore::default());
    let recovery = WalRecovery::new(Arc::clone(&wal), Vec::new());
    recovery
        .run(Arc::clone(&recovered) as Arc<dyn MapDataStore>)
        .await
        .expect("server must not refuse to start on a legacy WAL");

    let got = recovered
        .load(TEST_MAP, "legacy-k")
        .await
        .unwrap()
        .expect("legacy frame must replay");
    match got {
        RecordValue::Lww { value, .. } => {
            assert_eq!(value, Value::String("legacy-val".to_string()));
        }
        other => panic!("legacy frame must replay as LWW, got {other:?}"),
    }

    drop(dir);
}

/// AC3 — acked OR adds flushed through the real write-behind→redb drain survive a
/// process restart: re-opening redb with no in-memory residency still yields the
/// full OR content. Confirms the durable OR path (Cause-2 collapse guard) and
/// catches any regression that drops OR structure on the redb persist/reload path.
#[tokio::test(flavor = "multi_thread")]
async fn ac3_drained_redb_path_preserves_ormap_adds() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("redb_or.db");
    let or_value = or_record();

    {
        let redb: Arc<dyn MapDataStore> =
            Arc::new(RedbDataStore::new(&db_path).expect("redb new"));
        let store = WriteBehindDataStore::new(redb, never_flush_config());
        store
            .add(TEST_MAP, "ork-persist-1", &or_value, 0, 1000)
            .await
            .expect("OR add must ack");
        // Drain the buffer into redb, then drop all in-memory write-behind state.
        store.hard_flush().await.expect("hard_flush drains to redb");
        drop(store);
    }

    // Fresh redb open — no write-behind overlay, no in-memory residency.
    let reopened = RedbDataStore::new(&db_path).expect("redb reopen");
    let got = reopened
        .load(TEST_MAP, "ork-persist-1")
        .await
        .unwrap()
        .expect("drained OR add must persist durably in redb");
    assert_eq!(
        got, or_value,
        "redb-reloaded OR content must equal the acked OR record exactly"
    );

    drop(dir);
}
