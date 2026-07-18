//! Proofs for the per-partition, prefix-complete, cross-incarnation WAL applied
//! watermark `W(p)`.
//!
//! Sited as a CHILD of `write_behind` rather than a sibling of it: the pending
//! map, `max_assigned(p)`, the in-flight registry and the watermark-mode seam are
//! `pub(crate)` inside a PRIVATE module (`mod write_behind;`), so nothing outside
//! `datastores` can name them. A child reaches them directly, with no visibility
//! widening on the production surface.
//!
//! The incarnation-crossing model follows `storage/crash_safety_proptest.rs`:
//! drop the `WriteBehindDataStore` (staging buffer, pending tracker, in-flight
//! registry, queues and inner store all vanish), keep the on-disk WAL, and replay
//! into a FRESH inner store. The WAL handle itself may cross — it is the durable
//! artefact's accessor and every value it returns is read from disk. This proves
//! WAL-REPLAY correctness, NOT WAL-REOPEN correctness (the torn-tail truncate and
//! segment re-discovery paths are not re-run), and it does not model OS page
//! cache loss; both are the out-of-process soak harness's job.

use std::collections::HashSet;
use std::sync::atomic::AtomicU64 as FaultAtomicU64;

use proptest::prelude::*;
use tokio::runtime::Handle;
use tokio::sync::Mutex as AsyncMutex;
use tokio::task::block_in_place;
use topgun_core::hlc::Timestamp;
use topgun_core::types::Value;

use super::*;
use crate::storage::wal::{
    wal_fail_stop, WalFailStopTier, WalRecovery, WalWriter, FAIL_STOP_TEST_LOCK,
};

const TEST_MAP: &str = "wm";

// ---------------------------------------------------------------------------
// Async bridge for the synchronous proptest body
// ---------------------------------------------------------------------------

/// A process-wide multi-threaded runtime for the proptest bridge.
///
/// `proptest!` expands to a synchronous `#[test]`, so there is no ambient
/// runtime in the property body. `block_in_place` panics on a single-threaded
/// runtime, hence `multi_thread`.
static PROPTEST_RUNTIME: std::sync::LazyLock<tokio::runtime::Runtime> =
    std::sync::LazyLock::new(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("build multi-thread runtime for the proptest async bridge")
    });

fn block_on_async<F: std::future::Future>(fut: F) -> F::Output {
    let handle = PROPTEST_RUNTIME.handle().clone();
    let _guard = handle.enter();
    block_in_place(|| Handle::current().block_on(fut))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn lww(millis: u64) -> RecordValue {
    RecordValue::Lww {
        value: Value::Int(i64::try_from(millis).unwrap_or(0)),
        timestamp: Timestamp {
            millis,
            counter: 0,
            node_id: String::new(),
        },
    }
}

/// Delays long enough that nothing drains behind an assertion: every flush in
/// these tests is explicit, so a background pass can never resolve a sequence the
/// test is about to read.
fn never_flush_config() -> WriteBehindConfig {
    WriteBehindConfig {
        write_delay_ms: 600_000,
        flush_interval_ms: 600_000,
        shutdown_timeout_ms: 1_000,
        ..WriteBehindConfig::default()
    }
}

/// `count` distinct keys that all hash to `partition`.
///
/// The whole invariant is PER PARTITION, so a test whose keys scatter across the
/// 271 partitions would have one pending sequence each and could never construct
/// the stranded-lower-sequence state at all.
fn keys_in_partition(partition: u32, count: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0u64;
    while out.len() < count {
        let key = format!("key-{i}");
        if partition_for(TEST_MAP, &key) == partition {
            out.push(key);
        }
        i += 1;
        assert!(
            i < 1_000_000,
            "no key space found for partition {partition}"
        );
    }
    out
}

fn test_partition() -> u32 {
    partition_for(TEST_MAP, "key-0")
}

// ---------------------------------------------------------------------------
// Inner store double
// ---------------------------------------------------------------------------

/// Inner store that RETAINS what it is told and can be made to reject named keys.
///
/// Retention is load-bearing: a discarding store (`NullDataStore`) would make
/// every post-recovery read vacuously absent, so a lost frame and a working
/// replay would look identical. The reject set is what manufactures the abandoned
/// and failed-replay terminals.
#[derive(Default)]
struct FaultStore {
    /// (map, key) -> value, or `None` for a tombstone.
    data: AsyncMutex<HashMap<(String, String), Option<RecordValue>>>,
    reject: std::sync::Mutex<HashSet<String>>,
}

impl FaultStore {
    fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn reject_key(&self, key: &str) {
        self.reject
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(key.to_string());
    }

    fn accept_all(&self) {
        self.reject
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }

    fn rejects(&self, key: &str) -> bool {
        self.reject
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(key)
    }

    async fn contains(&self, key: &str) -> bool {
        matches!(
            self.data
                .lock()
                .await
                .get(&(TEST_MAP.to_string(), key.to_string())),
            Some(Some(_))
        )
    }

    async fn is_tombstone(&self, key: &str) -> bool {
        matches!(
            self.data
                .lock()
                .await
                .get(&(TEST_MAP.to_string(), key.to_string())),
            Some(None)
        )
    }
}

#[async_trait]
impl MapDataStore for FaultStore {
    async fn add(
        &self,
        map: &str,
        key: &str,
        value: &RecordValue,
        _exp: i64,
        _now: i64,
    ) -> anyhow::Result<()> {
        if self.rejects(key) {
            anyhow::bail!("injected inner-store rejection for key={key}");
        }
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
        if self.rejects(key) {
            anyhow::bail!("injected inner-store rejection for key={key}");
        }
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
            .cloned()
            .flatten())
    }

    async fn load_all(
        &self,
        _map: &str,
        _keys: &[String],
    ) -> anyhow::Result<Vec<(String, RecordValue)>> {
        Ok(Vec::new())
    }

    async fn enumerate_leaves(
        &self,
        _map: &str,
        _is_backup: bool,
        _sink: &mut dyn LeafSink,
    ) -> anyhow::Result<()> {
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

    async fn remove_all(&self, map: &str, keys: &[String]) -> anyhow::Result<()> {
        for key in keys {
            self.remove(map, key, 0).await?;
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
        map: &str,
        key: &str,
        value: &RecordValue,
        _is_backup: bool,
    ) -> anyhow::Result<()> {
        self.add(map, key, value, 0, 0).await
    }

    fn reset(&self) {}
}

// ---------------------------------------------------------------------------
// WAL double: a real WAL that can fail in EITHER class, selectably
// ---------------------------------------------------------------------------

/// Where an injected `append` failure sits RELATIVE TO THE WRITE PATH.
///
/// The two classes are distinguished structurally, never by inspecting a
/// returned `Err`: after the sealed-segment pre-check, a residual error is class
/// (B) by construction. Injecting only the easy pre-frame stub is exactly how the
/// (B) disposition ships untested.
#[derive(Clone, Copy, PartialEq, Eq)]
enum FaultClass {
    /// Upstream of the write path (encode / handle): no byte of the frame can
    /// have reached the segment, so the assigned sequence names nothing.
    PreFrame,
    /// At or after the frame write: the frame may be in the segment and its
    /// durability is unknown, so the process fail-stops instead of returning.
    PostFrame,
}

/// A real `WalWriter` behind an injectable failure, so frames are genuinely on
/// disk and a later incarnation replays them through the production recovery
/// path. An in-memory WAL double could not prove either.
struct FaultWal {
    inner: Arc<WalWriter>,
    appends: FaultAtomicU64,
    fail_on_nth: std::sync::Mutex<Option<(u64, FaultClass)>>,
}

impl FaultWal {
    fn new(inner: Arc<WalWriter>) -> Arc<Self> {
        Arc::new(Self {
            inner,
            appends: FaultAtomicU64::new(0),
            fail_on_nth: std::sync::Mutex::new(None),
        })
    }

    /// Fails the `nth` append this WAL sees (1-based) in `class`.
    fn fail_on(&self, nth: u64, class: FaultClass) {
        *self
            .fail_on_nth
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some((nth, class));
    }

    fn planned(&self, nth: u64) -> Option<FaultClass> {
        self.fail_on_nth
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .and_then(|(target, class)| (target == nth).then_some(class))
    }
}

#[async_trait]
impl Wal for FaultWal {
    async fn append(&self, partition: u32, entry: &WalEntry) -> anyhow::Result<()> {
        let nth = self.appends.fetch_add(1, Ordering::Relaxed) + 1;
        match self.planned(nth) {
            Some(FaultClass::PreFrame) => {
                anyhow::bail!("injected pre-frame WAL failure at append #{nth}")
            }
            Some(FaultClass::PostFrame) => {
                // The frame IS written first: class (B) is defined by the failure
                // sitting at or after the write, and a stub that fails before it
                // would silently be class (A) wearing a (B) label.
                self.inner.append(partition, entry).await?;
                wal_fail_stop(
                    WalFailStopTier::B,
                    &format!(
                        "injected post-frame WAL failure: partition={partition}, sequence={}",
                        entry.sequence
                    ),
                );
            }
            None => self.inner.append(partition, entry).await,
        }
    }

    async fn mark_applied(&self, partition: u32, sequence: u64) -> anyhow::Result<()> {
        self.inner.mark_applied(partition, sequence).await
    }

    async fn unapplied(&self, partition: u32) -> anyhow::Result<Vec<WalEntry>> {
        self.inner.unapplied(partition).await
    }
}

// ---------------------------------------------------------------------------
// Store construction
// ---------------------------------------------------------------------------

fn build_store(
    inner: &Arc<FaultStore>,
    wal: Arc<dyn Wal>,
    sequence_start: u64,
) -> Arc<WriteBehindDataStore> {
    WriteBehindDataStore::new_with_wal(
        Arc::clone(inner) as Arc<dyn MapDataStore>,
        never_flush_config(),
        Some(WalBootstrap {
            wal,
            sequence_start,
        }),
    )
}

// ===========================================================================
// AC1(a) — the stranded-entry differential, run in BOTH directions
// ===========================================================================

/// Drives the stranded-entry interleaving and returns the acked keys that are
/// ABSENT after a crash + recovery.
///
/// A coalesced entry holds the partition's LOWEST wal sequence (its first write)
/// and its HIGHEST (the write that coalesced onto it), while the lower sequences
/// of OTHER keys stay buffered. Flushing that one entry is therefore the exact
/// state in which a scalar `max` watermark marks still-buffered frames applied.
///
/// The coalesced entry is flushed via `flush_key` rather than by waiting for its
/// inherited `store_time` to become eligible: both reach the same resolve, but a
/// timing race would make the proof flaky, and a flaky negative control is not a
/// control.
async fn stranded_entry_scenario(mode: WatermarkMode, low_key_count: usize) -> Vec<String> {
    let dir = tempfile::tempdir().expect("tempdir");
    let wal = WalWriter::new(dir.path().to_path_buf(), WalFsyncPolicy::PerOp).expect("wal");
    let pre_crash_inner = FaultStore::new();
    let store = build_store(&pre_crash_inner, Arc::clone(&wal) as Arc<dyn Wal>, 1);
    store.test_set_watermark_mode(mode);

    let partition = test_partition();
    let keys = keys_in_partition(partition, low_key_count + 1);
    let (high, lows) = keys.split_first().expect("at least one key");

    // Lowest sequence in the partition, owned by the entry that will flush first.
    store.add(TEST_MAP, high, &lww(1), 0, 1000).await.unwrap();
    for (i, key) in lows.iter().enumerate() {
        let millis = 2 + i as u64;
        store
            .add(TEST_MAP, key, &lww(millis), 0, 1000)
            .await
            .unwrap();
    }
    // Coalesces onto the entry above, inheriting its store_time while taking a
    // sequence ABOVE every buffered low key.
    let high_value = lww(1000);
    store
        .add(TEST_MAP, high, &high_value, 0, 1000)
        .await
        .unwrap();

    store
        .flush_key(TEST_MAP, high, &high_value, false)
        .await
        .unwrap();

    // Crash: every write-behind-side structure dies with the store; the WAL files
    // on disk survive and the handle that reads them may cross.
    drop(store);

    let recovered = FaultStore::new();
    WalRecovery::new(Arc::clone(&wal), Vec::new())
        .run(Arc::clone(&recovered) as Arc<dyn MapDataStore>)
        .await
        .expect("recovery must succeed on an intact WAL");

    let mut missing = Vec::new();
    for key in &keys {
        if !recovered.contains(key).await {
            missing.push(key.clone());
        }
    }
    drop(dir);
    missing
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 12, ..ProptestConfig::default() })]

    /// The differential, in ONE committed suite: `PrefixComplete` loses nothing,
    /// `ScalarMax` loses the stranded writes.
    ///
    /// The second half is the negative control and it stays live. A test that
    /// passed in both directions would prove only that recovery runs, not that the
    /// watermark arithmetic is what saves the data.
    #[test]
    fn stranded_lower_sequences_survive_only_under_prefix_complete(low_keys in 1usize..4) {
        let lost_prefix_complete =
            block_on_async(stranded_entry_scenario(WatermarkMode::PrefixComplete, low_keys));
        prop_assert!(
            lost_prefix_complete.is_empty(),
            "prefix-complete must lose no acked write; lost {lost_prefix_complete:?}"
        );

        let lost_scalar_max =
            block_on_async(stranded_entry_scenario(WatermarkMode::ScalarMax, low_keys));
        prop_assert!(
            !lost_scalar_max.is_empty(),
            "the negative control did not discriminate: scalar-max must mark the \
             still-buffered lower frames applied and lose them on replay"
        );
    }
}

// ===========================================================================
// AC1(b) — the exact boundary, not merely the ordering
// ===========================================================================

#[tokio::test]
async fn watermark_is_exactly_one_below_the_lowest_unresolved_sequence() {
    let dir = tempfile::tempdir().unwrap();
    let wal = WalWriter::new(dir.path().to_path_buf(), WalFsyncPolicy::None).unwrap();
    let inner = FaultStore::new();
    let store = build_store(&inner, Arc::clone(&wal) as Arc<dyn Wal>, 1);
    let partition = test_partition();

    store.ensure_wal_seeded(partition).await;
    let low = store.assign_wal_sequence(partition);
    let mid = store.assign_wal_sequence(partition);
    let high = store.assign_wal_sequence(partition);
    store.promote_wal_sequence(partition, low);
    store.promote_wal_sequence(partition, mid);
    store.promote_wal_sequence(partition, high);

    assert_eq!(
        store.test_wal_watermark(partition),
        Some(Ok(low - 1)),
        "the watermark is INCLUSIVE, so it stops one BELOW the lowest unresolved \
         sequence; returning `low` itself would mark an un-resolved frame applied"
    );
    assert_ne!(
        store.test_wal_watermark(partition),
        Some(Ok(low)),
        "an off-by-one here still loses data: `low`'s frame would be filtered out \
         of replay and its segment made collectable while the write is buffered"
    );

    store
        .resolve_and_advance(partition, &BTreeSet::from([low]))
        .await;
    assert_eq!(
        store.test_wal_watermark(partition),
        Some(Ok(mid - 1)),
        "resolving the lowest moves the boundary up to the NEXT unresolved one"
    );
}

#[tokio::test]
async fn empty_and_seeded_watermark_is_max_assigned_never_the_counter() {
    let dir = tempfile::tempdir().unwrap();
    let wal = WalWriter::new(dir.path().to_path_buf(), WalFsyncPolicy::None).unwrap();
    let inner = FaultStore::new();
    let store = build_store(&inner, Arc::clone(&wal) as Arc<dyn Wal>, 1);
    let partition = test_partition();

    store.ensure_wal_seeded(partition).await;
    let seq = store.assign_wal_sequence(partition);
    store.promote_wal_sequence(partition, seq);
    store
        .resolve_and_advance(partition, &BTreeSet::from([seq]))
        .await;

    assert_eq!(
        store.test_wal_watermark(partition),
        Some(Ok(seq)),
        "with nothing pending every sequence ever assigned is resolved, so the \
         watermark is the highest ASSIGNED one"
    );
    assert_eq!(
        store.wal_sequence.load(Ordering::Relaxed),
        seq + 1,
        "fetch_add returns the OLD value, so the counter's load is the NEXT \
         sequence to assign — one ABOVE anything that exists"
    );
    assert_ne!(
        store.test_wal_watermark(partition),
        Some(Ok(store.wal_sequence.load(Ordering::Relaxed))),
        "using the counter's load would pre-mark the next write's frame applied \
         before that frame is even written"
    );
}

#[tokio::test]
async fn a_frame_written_after_the_partition_drains_is_still_replayed() {
    let dir = tempfile::tempdir().unwrap();
    let wal = WalWriter::new(dir.path().to_path_buf(), WalFsyncPolicy::PerOp).unwrap();
    let inner = FaultStore::new();
    let store = build_store(&inner, Arc::clone(&wal) as Arc<dyn Wal>, 1);
    let partition = test_partition();
    let keys = keys_in_partition(partition, 2);

    // Drain the partition completely, so the empty-and-seeded branch is the one
    // that computes the watermark.
    let first_value = lww(1);
    store
        .add(TEST_MAP, &keys[0], &first_value, 0, 1000)
        .await
        .unwrap();
    store
        .flush_key(TEST_MAP, &keys[0], &first_value, false)
        .await
        .unwrap();
    assert!(
        store.test_pending_wal_sequences(partition).is_empty(),
        "the partition must actually be drained for this to test the empty branch"
    );

    // The ordinary path: one more frame, then a crash before it flushes. Had the
    // empty branch used the counter's load, this frame was marked applied before
    // it existed and replay would filter it out.
    store
        .add(TEST_MAP, &keys[1], &lww(2), 0, 1000)
        .await
        .unwrap();
    drop(store);

    let recovered = FaultStore::new();
    WalRecovery::new(Arc::clone(&wal), Vec::new())
        .run(Arc::clone(&recovered) as Arc<dyn MapDataStore>)
        .await
        .unwrap();

    assert!(
        recovered.contains(&keys[1]).await,
        "a frame appended AFTER the partition drained must still be replayed"
    );
}

#[tokio::test]
async fn an_unseeded_partition_yields_a_typed_error_not_a_silent_advance() {
    let dir = tempfile::tempdir().unwrap();
    let wal = WalWriter::new(dir.path().to_path_buf(), WalFsyncPolicy::None).unwrap();
    let inner = FaultStore::new();
    let store = build_store(&inner, Arc::clone(&wal) as Arc<dyn Wal>, 1);
    let partition = test_partition();

    assert!(
        !store.test_wal_partition_seeded(partition),
        "a fresh store has seeded nothing"
    );
    assert_eq!(
        store.test_wal_watermark(partition),
        Some(Err(WalWatermarkError::Unseeded)),
        "an empty pending map says NOTHING about what a prior incarnation left \
         un-applied until the partition is seeded from the on-disk WAL"
    );

    store.ensure_wal_seeded(partition).await;
    assert_eq!(
        store.test_wal_watermark(partition),
        Some(Ok(0)),
        "once seeded from an empty WAL the branch is a value, not an error"
    );
}

// ===========================================================================
// AC2(a) — durable/coalesce terminals RESOLVE, across all three write paths
// ===========================================================================

#[tokio::test]
async fn heavy_coalescing_across_add_remove_and_remove_all_advances_the_full_prefix() {
    let dir = tempfile::tempdir().unwrap();
    let wal = WalWriter::new(dir.path().to_path_buf(), WalFsyncPolicy::None).unwrap();
    let inner = FaultStore::new();
    let store = build_store(&inner, Arc::clone(&wal) as Arc<dyn Wal>, 1);
    let partition = test_partition();
    let keys = keys_in_partition(partition, 4);

    // Repeated writes to the same keys retire an entry per coalesce, so every
    // retired sequence must reach a disposition — dropping one stalls the
    // watermark below it forever.
    for round in 0..4u64 {
        for key in &keys {
            store
                .add(TEST_MAP, key, &lww(round + 1), 0, 1000)
                .await
                .unwrap();
        }
    }
    store.remove(TEST_MAP, &keys[0], 1000).await.unwrap();
    store.remove(TEST_MAP, &keys[0], 1000).await.unwrap();
    store
        .remove_all(TEST_MAP, &[keys[1].clone(), keys[2].clone()])
        .await
        .unwrap();
    store
        .remove_all(TEST_MAP, &[keys[1].clone(), keys[2].clone()])
        .await
        .unwrap();

    let max_assigned = store.test_max_assigned_wal_sequence(partition);
    store.hard_flush().await.unwrap();

    assert!(
        store.test_pending_wal_sequences(partition).is_empty(),
        "after a full drain every sequence has a disposition; a leftover names a \
         coalesce-retire whose resolve was dropped"
    );
    assert_eq!(
        store.test_wal_watermark(partition),
        Some(Ok(max_assigned)),
        "the watermark must advance to the FULL prefix — a dropped coalesce-retire \
         resolve shows up here as a stall, not as a loss"
    );
    assert_eq!(
        wal.test_read_applied_sequence(partition),
        max_assigned,
        "the durable sidecar carries the advance, not just the in-memory tracker"
    );
}

// ===========================================================================
// AC2(b) — abandoned terminals do NOT resolve
// ===========================================================================

#[tokio::test]
async fn an_abandoned_terminal_holds_the_watermark_and_its_frame_is_replayed() {
    let dir = tempfile::tempdir().unwrap();
    let wal = WalWriter::new(dir.path().to_path_buf(), WalFsyncPolicy::PerOp).unwrap();
    let inner = FaultStore::new();
    let store = build_store(&inner, Arc::clone(&wal) as Arc<dyn Wal>, 1);
    let partition = test_partition();
    let keys = keys_in_partition(partition, 2);

    let doomed_value = lww(1);
    store
        .add(TEST_MAP, &keys[0], &doomed_value, 0, 1000)
        .await
        .unwrap();
    let doomed_seq = store
        .test_pending_wal_sequences(partition)
        .first()
        .map(|(seq, _)| *seq)
        .expect("the acked write is pending");

    // The inner store now permanently rejects the key, so the flush terminal is
    // ABANDONED: the write is durable nowhere but the WAL.
    inner.reject_key(&keys[0]);
    assert!(store
        .flush_key(TEST_MAP, &keys[0], &doomed_value, false)
        .await
        .is_err());

    assert_eq!(
        store.test_pending_wal_sequences(partition),
        vec![(doomed_seq, PendingOrigin::Abandoned)],
        "an abandoned terminal must NOT resolve — resolving here is the data-loss \
         bug, because the write exists only in the frame the resolve would release"
    );
    assert_eq!(
        store.test_wal_watermark(partition),
        Some(Ok(doomed_seq - 1)),
        "the watermark must not advance past an abandoned sequence"
    );

    // A later, healthy write must not drag the watermark over it either.
    store
        .add(TEST_MAP, &keys[1], &lww(2), 0, 1000)
        .await
        .unwrap();
    store.hard_flush().await.unwrap();
    assert_eq!(
        store.test_wal_watermark(partition),
        Some(Ok(doomed_seq - 1)),
        "a later success is not contiguous with the prefix and cannot license the \
         abandoned frame's release"
    );

    drop(store);
    let recovered = FaultStore::new();
    WalRecovery::new(Arc::clone(&wal), Vec::new())
        .run(Arc::clone(&recovered) as Arc<dyn MapDataStore>)
        .await
        .unwrap();
    assert!(
        recovered.contains(&keys[0]).await,
        "the abandoned write's frame must still be replayable on the next boot"
    );
}

// ===========================================================================
// AC2(c1-A) / (c2) — rollback is scoped to the FRAMELESS sequence only
// ===========================================================================

/// Builds a store over a real WAL wrapped in the injectable fault layer.
fn fault_wal_store(
    dir: &tempfile::TempDir,
) -> (
    Arc<WalWriter>,
    Arc<FaultWal>,
    Arc<FaultStore>,
    Arc<WriteBehindDataStore>,
) {
    let wal = WalWriter::new(dir.path().to_path_buf(), WalFsyncPolicy::PerOp).unwrap();
    let fault = FaultWal::new(Arc::clone(&wal));
    let inner = FaultStore::new();
    let store = build_store(&inner, Arc::clone(&fault) as Arc<dyn Wal>, 1);
    (wal, fault, inner, store)
}

#[tokio::test]
async fn a_pre_frame_append_failure_removes_only_the_frameless_sequence_from_add() {
    let dir = tempfile::tempdir().unwrap();
    let (_wal, fault, _inner, store) = fault_wal_store(&dir);
    let partition = test_partition();
    let keys = keys_in_partition(partition, 2);

    store
        .add(TEST_MAP, &keys[0], &lww(1), 0, 1000)
        .await
        .unwrap();
    let framed = store.test_max_assigned_wal_sequence(partition);

    fault.fail_on(2, FaultClass::PreFrame);
    assert!(store
        .add(TEST_MAP, &keys[1], &lww(2), 0, 1000)
        .await
        .is_err());

    assert_eq!(
        store.test_pending_wal_sequences(partition),
        vec![(framed, PendingOrigin::Live)],
        "the frameless sequence must be gone (it would pin the watermark forever \
         and fire a phantom abandoned-write alarm) while the frame-backed one stays"
    );
    assert!(
        store.test_max_assigned_wal_sequence(partition) > framed,
        "max_assigned stays monotonic across a rollback: a rolled-back sequence \
         names no frame, so a watermark at that value asserts nothing"
    );
}

#[tokio::test]
async fn a_pre_frame_append_failure_removes_only_the_frameless_sequence_from_remove() {
    let dir = tempfile::tempdir().unwrap();
    let (_wal, fault, _inner, store) = fault_wal_store(&dir);
    let partition = test_partition();
    let keys = keys_in_partition(partition, 2);

    store
        .add(TEST_MAP, &keys[0], &lww(1), 0, 1000)
        .await
        .unwrap();
    let framed = store.test_max_assigned_wal_sequence(partition);

    fault.fail_on(2, FaultClass::PreFrame);
    assert!(store.remove(TEST_MAP, &keys[1], 1000).await.is_err());

    assert_eq!(
        store.test_pending_wal_sequences(partition),
        vec![(framed, PendingOrigin::Live)],
        "remove() rolls back its own frameless sequence and nothing else"
    );
}

#[tokio::test]
async fn a_mid_loop_remove_all_failure_rolls_back_only_the_failed_key() {
    let dir = tempfile::tempdir().unwrap();
    let (wal, fault, _inner, store) = fault_wal_store(&dir);
    let partition = test_partition();
    let keys = keys_in_partition(partition, 4);

    // Keys 1..3 append Ok and are enqueued and staged; key 4's append fails
    // upstream of the write path.
    fault.fail_on(4, FaultClass::PreFrame);
    assert!(store.remove_all(TEST_MAP, &keys).await.is_err());

    let pending = store.test_pending_wal_sequences(partition);
    assert_eq!(
        pending.len(),
        3,
        "the three frame-backed sequences MUST remain pending — rolling them back \
         is the loss that resurrects deleted keys across a crash; got {pending:?}"
    );
    let lowest = pending.first().map(|(seq, _)| *seq).unwrap();
    assert_eq!(
        store.test_wal_watermark(partition),
        Some(Ok(lowest - 1)),
        "the surviving sequences must still BOUND the watermark at the instant the \
         Err returns"
    );
    assert_eq!(
        wal.test_read_applied_sequence(partition),
        0,
        "nothing durable may have been marked applied past those frames"
    );
}

/// AC2(d) — the end-to-end half of the mid-loop guard. The tracker holding the
/// right sequences (above) proves only that they are TRACKED; this proves the
/// data actually survives, which fails if any other path advances `W(p)`.
#[tokio::test]
async fn a_mid_loop_remove_all_failure_still_replays_the_earlier_tombstones() {
    let dir = tempfile::tempdir().unwrap();
    let (wal, fault, _inner, store) = fault_wal_store(&dir);
    let partition = test_partition();
    let keys = keys_in_partition(partition, 4);

    fault.fail_on(4, FaultClass::PreFrame);
    assert!(store.remove_all(TEST_MAP, &keys).await.is_err());

    // Cross the incarnation boundary BEFORE keys 1..3 flush.
    drop(store);

    // Pre-populate the fresh store so an un-replayed tombstone is visible as a
    // RESURRECTED key rather than as an indistinguishable absence.
    let recovered = FaultStore::new();
    for key in &keys {
        recovered
            .add(TEST_MAP, key, &lww(1), 0, 1000)
            .await
            .unwrap();
    }
    WalRecovery::new(Arc::clone(&wal), Vec::new())
        .run(Arc::clone(&recovered) as Arc<dyn MapDataStore>)
        .await
        .unwrap();

    for key in &keys[..3] {
        assert!(
            recovered.is_tombstone(key).await,
            "the tombstone for {key} was acked into the WAL before the mid-loop \
             failure and MUST be replayed; a rollback of its sequence resurrects it"
        );
    }
    assert!(
        recovered.contains(&keys[3]).await,
        "the failed key's removal was never acked, so it correctly did not apply"
    );
}

// ===========================================================================
// AC2(c1-B) — the class (B) disposition: fail-stop at tier B, NO rollback
// ===========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn a_post_frame_append_failure_fail_stops_at_tier_b_without_rolling_back() {
    // Serialised against every other fail-stop assertion: the observation log is
    // process-global, so a concurrent tier would make the index read below race.
    let _guard = FAIL_STOP_TEST_LOCK.lock().await;

    let dir = tempfile::tempdir().unwrap();
    let (wal, fault, _inner, store) = fault_wal_store(&dir);
    let partition = test_partition();
    let keys = keys_in_partition(partition, 2);

    store
        .add(TEST_MAP, &keys[0], &lww(1), 0, 1000)
        .await
        .unwrap();
    let framed = store.test_max_assigned_wal_sequence(partition);

    let before = WalWriter::test_fail_stop_observations().len();

    fault.fail_on(2, FaultClass::PostFrame);
    // Run the failing write on its own task: the fail-stop's test-mode arm panics
    // instead of aborting the process precisely so the tier stays readable, and
    // the JoinHandle is what catches it.
    let store_clone = Arc::clone(&store);
    let key = keys[1].clone();
    let outcome =
        tokio::spawn(async move { store_clone.add(TEST_MAP, &key, &lww(2), 0, 1000).await })
            .await
            .err();
    assert!(
        outcome.is_some_and(|e| e.is_panic()),
        "a failure at or after the frame write must fail-stop, not return an Err \
         the caller could mistake for a rollback-able condition"
    );

    let observed = WalWriter::test_fail_stop_observations();
    assert_eq!(
        observed.get(before),
        Some(&WalFailStopTier::B),
        "the TIER is the assertion: without it class (B) is indistinguishable from \
         the sealed-segment stop, which is exactly how the gap survived"
    );
    assert_eq!(
        observed.len(),
        before + 1,
        "exactly one stop fired; a second would mean the residual path also ran a \
         disposition it must never reach"
    );

    let pending = store.test_pending_wal_sequences(partition);
    let stopped_seq = store.test_max_assigned_wal_sequence(partition);
    assert!(
        pending.iter().any(|(seq, _)| *seq == stopped_seq),
        "the class (B) sequence must NOT be rolled back: its frame may be in the \
         segment, so dropping it would let the watermark pass a frame never applied"
    );
    assert_eq!(
        store.test_wal_watermark(partition),
        Some(Ok(framed - 1)),
        "the watermark must not advance past the stopped sequence"
    );
    drop(wal);
}

// ===========================================================================
// AC13 — boot seeding across THREE incarnations with an injected replay failure
// ===========================================================================

/// The full restart-crossing scenario: a frame a prior incarnation left
/// un-applied must keep pinning `W(p)` in the NEXT incarnation, whose pending map
/// provably started empty.
async fn boot_seeding_scenario(mode: WatermarkMode) -> BootSeedingOutcome {
    let dir = tempfile::tempdir().unwrap();
    let partition = test_partition();
    let keys = keys_in_partition(partition, 4);
    let wal = WalWriter::new(dir.path().to_path_buf(), WalFsyncPolicy::PerOp).unwrap();

    // --- Incarnation 1: one applied frame at 100, then 101/102 left buffered ---
    {
        let inner = FaultStore::new();
        let store = build_store(&inner, Arc::clone(&wal) as Arc<dyn Wal>, 100);
        let first_value = lww(1);
        store
            .add(TEST_MAP, &keys[0], &first_value, 0, 1000)
            .await
            .unwrap();
        // Drained key-by-key rather than with `hard_flush`, which marks the store
        // shut down and would reject the two writes this incarnation must still ack.
        store
            .flush_key(TEST_MAP, &keys[0], &first_value, false)
            .await
            .unwrap();
        assert_eq!(
            wal.test_read_applied_sequence(partition),
            100,
            "the first frame is durably applied, so the sidecar sits at 100"
        );
        store
            .add(TEST_MAP, &keys[1], &lww(2), 0, 1000)
            .await
            .unwrap();
        store
            .add(TEST_MAP, &keys[2], &lww(3), 0, 1000)
            .await
            .unwrap();
        // kill -9: frames 101 and 102 are on disk and un-applied.
        drop(store);
    }

    // --- Incarnation 2: recovery fails the replay of 101 ---
    let inner = FaultStore::new();
    inner.reject_key(&keys[1]);
    WalRecovery::new(Arc::clone(&wal), Vec::new())
        .run(Arc::clone(&inner) as Arc<dyn MapDataStore>)
        .await
        .unwrap();
    assert_eq!(
        wal.test_read_applied_sequence(partition),
        100,
        "R5: the sidecar stops at the contiguous-SUCCESS frontier, so 102's success \
         cannot license the release of the 101 that failed below it"
    );

    let store = build_store(&inner, Arc::clone(&wal) as Arc<dyn Wal>, 103);
    store.test_set_watermark_mode(mode);

    // (a)'s precondition, asserted rather than assumed: this tracker provably
    // started empty, so the seeding path is genuinely exercised.
    assert!(
        !store.test_wal_partition_seeded(partition),
        "the rebuilt store must start unseeded or (a) passes trivially"
    );
    assert!(store.test_pending_wal_sequences(partition).is_empty());
    assert_eq!(
        store.test_wal_watermark(partition),
        Some(Err(WalWatermarkError::Unseeded)),
        "the seeding-order guard: a watermark computed before the partition is \
         seeded is a typed error, never a silent advance"
    );

    // A live write to p seeds the partition and then flushes Ok.
    let live_value = lww(4);
    store
        .add(TEST_MAP, &keys[3], &live_value, 0, 1000)
        .await
        .unwrap();
    let seeded = store.test_pending_wal_sequences(partition);
    store
        .flush_key(TEST_MAP, &keys[3], &live_value, false)
        .await
        .unwrap();

    let sidecar_after_live_flush = wal.test_read_applied_sequence(partition);
    let alarm = store.test_run_classifier_sample(partition);
    let retained: Vec<u64> = wal
        .unapplied(partition)
        .await
        .unwrap()
        .iter()
        .map(|e| e.sequence)
        .collect();

    // --- Incarnation 3: the store is healthy again ---
    drop(store);
    let healthy = FaultStore::new();
    healthy.accept_all();
    WalRecovery::new(Arc::clone(&wal), Vec::new())
        .run(Arc::clone(&healthy) as Arc<dyn MapDataStore>)
        .await
        .unwrap();
    let healed_keys_present = healthy.contains(&keys[1]).await && healthy.contains(&keys[2]).await;
    let sidecar_after_heal = wal.test_read_applied_sequence(partition);

    drop(dir);
    BootSeedingOutcome {
        seeded,
        sidecar_after_live_flush,
        alarm,
        retained,
        healed_keys_present,
        sidecar_after_heal,
    }
}

/// What the three incarnations observed. Returned rather than asserted inside the
/// scenario because the two modes must reach OPPOSITE outcomes — an assertion
/// shared by both directions could only be one that does not discriminate.
struct BootSeedingOutcome {
    seeded: Vec<(u64, PendingOrigin)>,
    sidecar_after_live_flush: u64,
    alarm: Option<WalWatermarkAlarm>,
    retained: Vec<u64>,
    healed_keys_present: bool,
    sidecar_after_heal: u64,
}

#[tokio::test]
async fn boot_seeding_holds_the_watermark_at_the_prior_incarnations_frontier() {
    let outcome = boot_seeding_scenario(WatermarkMode::PrefixComplete).await;

    let seeded_boot: Vec<u64> = outcome
        .seeded
        .iter()
        .filter(|(_, origin)| *origin == PendingOrigin::BootUnreplayed)
        .map(|(seq, _)| *seq)
        .collect();
    assert_eq!(
        seeded_boot,
        vec![101, 102],
        "the pending map is seeded from wal.unapplied(p) with EVERY frame the \
         prior incarnation left un-applied"
    );
    assert_eq!(
        outcome.sidecar_after_live_flush, 100,
        "W(p) stays at the prior incarnation's frontier — it must NOT jump to this \
         incarnation's live counter, which is what erases R5's work on the first flush"
    );
    assert!(
        matches!(
            outcome.alarm,
            Some(WalWatermarkAlarm::AbandonedWrite {
                origin: PendingOrigin::BootUnreplayed
            })
        ),
        "a frame a prior incarnation left un-applied is an abandoned write, NOT a \
         tracker leak: it is absent from the queue and the registry by construction; \
         got {:?}",
        outcome.alarm
    );
    assert!(
        outcome.retained.contains(&101) && outcome.retained.contains(&102),
        "the frames must still be on disk and still returned by unapplied(); got {:?}",
        outcome.retained
    );
    assert!(
        outcome.healed_keys_present,
        "the stall self-heals on a healthy third incarnation: acked-implies-durable \
         working as contracted"
    );
    assert!(
        outcome.sidecar_after_heal >= 102,
        "and only THEN does W(p) advance past them"
    );
}

#[tokio::test]
async fn the_naive_empty_set_watermark_erases_the_prior_incarnations_frontier() {
    // The live negative control for AC13. The seeding is identical in both modes —
    // what differs is the arithmetic applied to it, so this isolates exactly the
    // failure R1.2 exists to prevent: the first flush of a NEW incarnation marking
    // the previous one's un-replayed frames applied.
    let outcome = boot_seeding_scenario(WatermarkMode::ScalarMax).await;
    assert!(
        outcome.sidecar_after_live_flush > 102,
        "the control did not discriminate: a scalar advance must push the sidecar \
         past the retained frames 101/102; got {}",
        outcome.sidecar_after_live_flush
    );
    assert!(
        outcome.retained.is_empty(),
        "and that advance FILTERS them out of the replay window — the acked-write \
         loss this spec exists to close; retained {:?}",
        outcome.retained
    );
    assert!(
        !outcome.healed_keys_present,
        "so a healthy third incarnation has nothing left to replay and the writes \
         are gone for good"
    );
}

/// The other half of the seeding-order guard: a partition that never takes a
/// live write is never seeded, never advances, and keeps its frames.
#[tokio::test]
async fn a_partition_without_a_live_write_never_marks_anything_applied() {
    let dir = tempfile::tempdir().unwrap();
    let wal = WalWriter::new(dir.path().to_path_buf(), WalFsyncPolicy::PerOp).unwrap();
    let partition = test_partition();
    let keys = keys_in_partition(partition, 1);

    {
        let inner = FaultStore::new();
        let store = build_store(&inner, Arc::clone(&wal) as Arc<dyn Wal>, 1);
        store
            .add(TEST_MAP, &keys[0], &lww(1), 0, 1000)
            .await
            .unwrap();
        drop(store);
    }

    let inner = FaultStore::new();
    let store = build_store(&inner, Arc::clone(&wal) as Arc<dyn Wal>, 2);
    // Writes land in a DIFFERENT partition, so `partition` is never touched.
    let other = (0..PARTITION_COUNT).find(|p| *p != partition).unwrap();
    let other_key = keys_in_partition(other, 1);
    store
        .add(TEST_MAP, &other_key[0], &lww(2), 0, 1000)
        .await
        .unwrap();
    store.hard_flush().await.unwrap();

    assert!(
        !store.test_wal_partition_seeded(partition),
        "an untouched partition is never seeded"
    );
    assert_eq!(
        wal.test_read_applied_sequence(partition),
        0,
        "and never has anything marked applied, so its frames stay retained"
    );
    assert!(!wal.unapplied(partition).await.unwrap().is_empty());
}
