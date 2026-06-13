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

use crate::storage::datastores::{WalBootstrap, WriteBehindConfig, WriteBehindDataStore};
use crate::storage::map_data_store::MapDataStore;
use crate::storage::record::RecordValue;
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
    use crate::storage::wal::{WalEntry, WalOp};

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
            value: Value::Int(1),
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
