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
use crate::service::middleware::init_observability;
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
    /// Keys whose `add` parks instead of returning.
    ///
    /// A hang is categorically different from a rejection: it returns neither
    /// `Ok` nor `Err`, so no terminal runs and no retry ladder starts. That is
    /// the state the classifier must read as an environment problem rather than
    /// as a lost sequence.
    hang: std::sync::Mutex<HashSet<String>>,
    /// Set once the test releases every parked call, so the hang cannot outlive
    /// the test and wedge the runtime's worker threads.
    released: std::sync::atomic::AtomicBool,
    wake: tokio::sync::Notify,
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

    /// Parks every subsequent `add` of `key` until [`release`](Self::release).
    fn hang_key(&self, key: &str) {
        self.hang
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(key.to_string());
    }

    fn release(&self) {
        self.released
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self.wake.notify_waiters();
    }

    fn hangs(&self, key: &str) -> bool {
        self.hang
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(key)
    }

    /// Parks until released. A flag plus a notify rather than a bare `Notify`:
    /// a release that lands before the park would otherwise be missed and the
    /// call would hang for real.
    async fn park(&self) {
        while !self.released.load(std::sync::atomic::Ordering::Relaxed) {
            self.wake.notified().await;
        }
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
        if self.hangs(key) {
            self.park().await;
            anyhow::bail!("injected inner-store hang released for key={key}");
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
        if self.hangs(key) {
            self.park().await;
            anyhow::bail!("injected inner-store hang released for key={key}");
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
    /// Parks every append instead of returning, modelling a hung or full disk.
    ///
    /// Distinct from every failure class above: the caller is parked INSIDE the
    /// append, so the rollback that a returned `Err` triggers never runs and the
    /// sequence stays mid-append with no entry and no frame.
    hang: std::sync::atomic::AtomicBool,
    released: std::sync::atomic::AtomicBool,
    wake: tokio::sync::Notify,
}

impl FaultWal {
    fn new(inner: Arc<WalWriter>) -> Arc<Self> {
        Arc::new(Self {
            inner,
            appends: FaultAtomicU64::new(0),
            fail_on_nth: std::sync::Mutex::new(None),
            hang: std::sync::atomic::AtomicBool::new(false),
            released: std::sync::atomic::AtomicBool::new(false),
            wake: tokio::sync::Notify::new(),
        })
    }

    /// Parks every subsequent append until [`release`](Self::release).
    fn hang_appends(&self) {
        self.hang.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    fn release(&self) {
        self.released
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self.wake.notify_waiters();
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
        if self.hang.load(std::sync::atomic::Ordering::Relaxed) {
            while !self.released.load(std::sync::atomic::Ordering::Relaxed) {
                self.wake.notified().await;
            }
            anyhow::bail!("injected WAL append hang released");
        }
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

// ===========================================================================
// AC3(a) — the classifier matrix: six scenarios, one per row
// ===========================================================================

/// A config whose flush loop actually drains, for the scenarios that need an
/// entry to reach the inner store.
fn draining_config() -> WriteBehindConfig {
    WriteBehindConfig {
        write_delay_ms: 0,
        flush_interval_ms: 5,
        shutdown_timeout_ms: 200,
        max_retries: 1,
        backoff_base_ms: 1,
        backoff_cap_ms: 2,
        ..WriteBehindConfig::default()
    }
}

fn build_store_with(
    inner: &Arc<FaultStore>,
    wal: Arc<dyn Wal>,
    sequence_start: u64,
    config: WriteBehindConfig,
) -> Arc<WriteBehindDataStore> {
    WriteBehindDataStore::new_with_wal(
        Arc::clone(inner) as Arc<dyn MapDataStore>,
        config,
        Some(WalBootstrap {
            wal,
            sequence_start,
        }),
    )
}

/// A real `WalWriter` in a fresh directory, kept alive by the returned guard.
fn real_wal() -> (tempfile::TempDir, Arc<WalWriter>) {
    let dir = tempfile::tempdir().expect("tempdir");
    let wal = WalWriter::new(dir.path().to_path_buf(), WalFsyncPolicy::PerOp).expect("wal");
    (dir, wal)
}

/// Waits for a condition instead of sleeping a guessed duration: the scenarios
/// depend on a background task having reached a specific state, and a fixed
/// sleep would make that a race rather than a proof.
async fn wait_until(label: &str, mut cond: impl FnMut() -> bool) {
    for _ in 0..2_000 {
        if cond() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    }
    panic!("timed out waiting for {label}");
}

/// The `topgun_wal_applied_watermark_lag` sample for `partition`, read off the
/// exact text `GET /metrics` serves.
fn scraped_lag(rendered: &str, partition: u32) -> Option<f64> {
    let needle = format!("partition=\"{partition}\"");
    rendered
        .lines()
        .find(|line| line.starts_with(WAL_WATERMARK_LAG_GAUGE) && line.contains(&needle))
        .and_then(|line| line.rsplit(' ').next())
        .and_then(|v| v.parse::<f64>().ok())
}

/// Asserts the partition's stall is visible to an operator scraping `/metrics`,
/// not merely to an internal read.
///
/// The partition label is per-test-unique by construction (each scenario owns
/// its own partition), so a hit here can never be another test's emission.
fn assert_lag_visible(partition: u32, at_least: f64) {
    let rendered = init_observability().render_metrics();
    assert!(
        rendered.contains(&format!("# HELP {WAL_WATERMARK_LAG_GAUGE}")),
        "the lag gauge must be DESCRIBED on the scrape"
    );
    let lag = scraped_lag(&rendered, partition);
    assert!(
        lag.is_some_and(|v| v >= at_least),
        "partition {partition} must report a lag of at least {at_least} on the \
         scrape; got {lag:?}"
    );
}

#[tokio::test]
async fn a_hung_inner_store_is_an_abandoned_write_not_a_leak() {
    // Install the recorder BEFORE the first emission: a metric emitted against
    // the no-op recorder is invisible to every later scrape.
    init_observability();
    let partition = 250;
    let key = keys_in_partition(partition, 1).remove(0);
    let (_dir, wal) = real_wal();
    let inner = FaultStore::new();
    let store = build_store_with(
        &inner,
        Arc::clone(&wal) as Arc<dyn Wal>,
        1,
        draining_config(),
    );

    inner.hang_key(&key);
    store.add(TEST_MAP, &key, &lww(1), 0, 0).await.unwrap();

    // The flush worker has taken the entry out of the queue and registered it,
    // and its `inner.add` will never return: in-flight, no `Err`, no terminal.
    wait_until("the hung entry to be registered in flight", || {
        !store.test_in_flight_wal_sequences(partition).is_empty()
    })
    .await;

    let alarm = store.test_run_classifier_sample(partition);
    assert_eq!(
        alarm,
        Some(WalWatermarkAlarm::AbandonedWrite {
            origin: PendingOrigin::Live
        }),
        "a hung STORE sends the operator to the backend"
    );
    // Explicitly NOT the other class, and explicitly not the other origin: under
    // a rule that keys on "absent from queue and registry" this reads as a code
    // bug, and under a collapsed tag it reads as a hung disk.
    assert_ne!(alarm, Some(WalWatermarkAlarm::TrackerLeak));
    assert_ne!(
        alarm,
        Some(WalWatermarkAlarm::AbandonedWrite {
            origin: PendingOrigin::Appending
        })
    );
    assert_lag_visible(partition, 1.0);

    inner.release();
}

#[tokio::test]
async fn a_boot_unreplayed_sequence_is_an_abandoned_write_not_a_leak() {
    // Install the recorder BEFORE the first emission: a metric emitted against
    // the no-op recorder is invisible to every later scrape.
    init_observability();
    let partition = 251;
    let key = keys_in_partition(partition, 1).remove(0);
    let (_dir, wal) = real_wal();

    // A prior incarnation acked a write whose frame never reached the store.
    {
        let inner = FaultStore::new();
        let store = build_store(&inner, Arc::clone(&wal) as Arc<dyn Wal>, 1);
        store.add(TEST_MAP, &key, &lww(1), 0, 0).await.unwrap();
        drop(store);
    }

    // A fresh incarnation: no queue, no registry, no entry — by construction.
    let inner = FaultStore::new();
    let store = build_store(&inner, Arc::clone(&wal) as Arc<dyn Wal>, 2);
    store.ensure_wal_seeded(partition).await;
    assert_eq!(
        store.test_pending_wal_sequences(partition),
        vec![(1, PendingOrigin::BootUnreplayed)],
    );

    let alarm = store.test_run_classifier_sample(partition);
    assert_eq!(
        alarm,
        Some(WalWatermarkAlarm::AbandonedWrite {
            origin: PendingOrigin::BootUnreplayed
        }),
        "a prior incarnation's un-replayed frame is a stall to surface, not a bug \
         to fix"
    );
    assert_ne!(alarm, Some(WalWatermarkAlarm::TrackerLeak));
    // Zero, not one: `max_assigned` seeds at 0 and this incarnation has assigned
    // nothing, so the LAG is genuinely zero while the stall is real. What the
    // scrape must show here is that the partition is reported at all.
    assert_lag_visible(partition, 0.0);
}

#[tokio::test]
async fn a_queued_then_dequeued_sequence_missing_from_the_registry_is_a_leak() {
    // Install the recorder BEFORE the first emission: a metric emitted against
    // the no-op recorder is invisible to every later scrape.
    init_observability();
    let partition = 252;
    let key = keys_in_partition(partition, 1).remove(0);
    let (_dir, wal) = real_wal();
    let inner = FaultStore::new();
    let store = build_store(&inner, Arc::clone(&wal) as Arc<dyn Wal>, 1);

    store.add(TEST_MAP, &key, &lww(1), 0, 0).await.unwrap();
    assert_eq!(
        store.test_pending_wal_sequences(partition),
        vec![(1, PendingOrigin::Live)],
        "a successful add promotes to Live at the queue insert — an un-promoted \
         sequence would report a healthy queued write as a hung disk"
    );

    // Dequeue WITHOUT registering: the missed disposition this class exists to
    // name. `Live` is entered only at the queue insert, so this state cannot
    // arise from any correct path.
    let drained = store
        .queues
        .get_mut(&partition)
        .unwrap()
        .drain_ready(i64::MAX);
    assert_eq!(drained.len(), 1);
    drop(drained);
    assert!(store.test_in_flight_wal_sequences(partition).is_empty());

    assert_eq!(
        store.test_run_classifier_sample(partition),
        None,
        "one ownerless read is a CANDIDATE: a sequence drained between the \
         registry probe and the queue scan reads identically"
    );
    let alarm = store.test_run_classifier_sample(partition);
    assert_eq!(
        alarm,
        Some(WalWatermarkAlarm::TrackerLeak),
        "a genuine leak is permanent, so it survives re-confirmation"
    );
    assert_ne!(
        alarm,
        Some(WalWatermarkAlarm::AbandonedWrite {
            origin: PendingOrigin::Live
        })
    );
    assert_lag_visible(partition, 1.0);
}

#[tokio::test]
async fn a_max_retries_discard_is_an_abandoned_write_not_a_leak() {
    // Install the recorder BEFORE the first emission: a metric emitted against
    // the no-op recorder is invisible to every later scrape.
    init_observability();
    let partition = 253;
    let key = keys_in_partition(partition, 1).remove(0);
    let (_dir, wal) = real_wal();
    let inner = FaultStore::new();
    let store = build_store_with(
        &inner,
        Arc::clone(&wal) as Arc<dyn Wal>,
        1,
        draining_config(),
    );

    inner.reject_key(&key);
    store.add(TEST_MAP, &key, &lww(1), 0, 0).await.unwrap();

    wait_until("the entry to exhaust its retries", || {
        store.test_pending_wal_sequences(partition) == vec![(1, PendingOrigin::Abandoned)]
    })
    .await;
    assert!(
        store.test_in_flight_wal_sequences(partition).is_empty(),
        "the discard deregisters, so the tag is the ONLY thing carrying the class"
    );

    let alarm = store.test_run_classifier_sample(partition);
    assert_eq!(
        alarm,
        Some(WalWatermarkAlarm::AbandonedWrite {
            origin: PendingOrigin::Abandoned
        }),
    );
    assert_ne!(alarm, Some(WalWatermarkAlarm::TrackerLeak));
    assert_lag_visible(partition, 1.0);
}

#[tokio::test]
async fn a_sequence_queued_behind_a_blocked_worker_is_an_abandoned_write_not_a_leak() {
    // Install the recorder BEFORE the first emission: a metric emitted against
    // the no-op recorder is invisible to every later scrape.
    init_observability();
    let blocked_partition = 254;
    let queued_partition = 255;
    let blocked_key = keys_in_partition(blocked_partition, 1).remove(0);
    let queued_key = keys_in_partition(queued_partition, 1).remove(0);
    let (_dir, wal) = real_wal();
    let inner = FaultStore::new();
    let store = build_store_with(
        &inner,
        Arc::clone(&wal) as Arc<dyn Wal>,
        1,
        draining_config(),
    );

    // The flush loop is sequential across partitions, so one hung `inner` call
    // holds up every entry drained after it.
    inner.hang_key(&blocked_key);
    store
        .add(TEST_MAP, &blocked_key, &lww(1), 0, 0)
        .await
        .unwrap();
    wait_until("the worker to block on the hung entry", || {
        !store
            .test_in_flight_wal_sequences(blocked_partition)
            .is_empty()
    })
    .await;

    store
        .add(TEST_MAP, &queued_key, &lww(2), 0, 0)
        .await
        .unwrap();
    assert!(
        store
            .test_in_flight_wal_sequences(queued_partition)
            .is_empty(),
        "the second write is still QUEUED — the worker never got to it"
    );

    let alarm = store.test_run_classifier_sample(queued_partition);
    assert_eq!(
        alarm,
        Some(WalWatermarkAlarm::AbandonedWrite {
            origin: PendingOrigin::Live
        }),
        "a queued sequence has an owner, so it is a stall, not a leak"
    );
    assert_ne!(alarm, Some(WalWatermarkAlarm::TrackerLeak));
    assert_lag_visible(queued_partition, 1.0);

    inner.release();
}

#[tokio::test]
async fn a_hung_wal_append_is_an_appending_abandoned_write_not_a_leak() {
    // Install the recorder BEFORE the first emission: a metric emitted against
    // the no-op recorder is invisible to every later scrape.
    init_observability();
    let partition = 256;
    let key = keys_in_partition(partition, 1).remove(0);
    let (_dir, inner_wal) = real_wal();
    let wal = FaultWal::new(inner_wal);
    let inner = FaultStore::new();
    let store = build_store(&inner, Arc::clone(&wal) as Arc<dyn Wal>, 1);

    wal.hang_appends();
    // `add` never returns, so the write must run on its own task — this is the
    // state a per-op device-barrier fsync parks a caller in.
    let writer = {
        let store = Arc::clone(&store);
        let key = key.clone();
        tokio::spawn(async move { store.add(TEST_MAP, &key, &lww(1), 0, 0).await })
    };

    wait_until("the sequence to be assigned mid-append", || {
        store.test_pending_wal_sequences(partition) == vec![(1, PendingOrigin::Appending)]
    })
    .await;
    assert!(
        store.test_in_flight_wal_sequences(partition).is_empty(),
        "absent from the queue AND the registry is this state's NORMAL"
    );

    let alarm = store.test_run_classifier_sample(partition);
    assert_eq!(
        alarm,
        Some(WalWatermarkAlarm::AbandonedWrite {
            origin: PendingOrigin::Appending
        }),
        "a hung APPEND sends the operator to the disk, not to the backend"
    );
    // Without the fourth state this sequence is `Live` and absent from both,
    // which classifies as a code bug against a perfectly healthy write path.
    assert_ne!(alarm, Some(WalWatermarkAlarm::TrackerLeak));
    assert_ne!(
        alarm,
        Some(WalWatermarkAlarm::AbandonedWrite {
            origin: PendingOrigin::Live
        })
    );
    assert_lag_visible(partition, 1.0);

    wal.release();
    let _ = writer.await;
}

#[tokio::test]
async fn a_transient_ownerless_window_never_fires_an_alarm() {
    let partition = 257;
    let key = keys_in_partition(partition, 1).remove(0);
    let (_dir, wal) = real_wal();
    let inner = FaultStore::new();
    let store = build_store(&inner, Arc::clone(&wal) as Arc<dyn Wal>, 1);

    store.add(TEST_MAP, &key, &lww(1), 0, 0).await.unwrap();

    // Drain INSIDE the classifier's own sample, between the registry probe and
    // the queue scan: the first sample then reads absent-from-both on a write
    // that is doing exactly the right thing. Deterministic by construction —
    // the alternative is the sleep-based race this must not be.
    let fired = Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let store_hook = Arc::clone(&store);
        let fired = Arc::clone(&fired);
        store.test_set_classifier_hook(Arc::new(move |p: u32| {
            if fired.swap(true, std::sync::atomic::Ordering::Relaxed) {
                return;
            }
            let drained = store_hook.queues.get_mut(&p).unwrap().drain_ready(i64::MAX);
            for entry in &drained {
                store_hook.register_in_flight(p, &entry.wal_sequences);
            }
        }));
    }

    assert_eq!(
        store.test_run_classifier_sample(partition),
        None,
        "the transient window must NOT be a verdict"
    );

    // The write then completes normally, before the confirming sample.
    store
        .resolve_and_advance(partition, &BTreeSet::from([1]))
        .await;

    assert_eq!(
        store.test_run_classifier_sample(partition),
        None,
        "and no alarm of ANY class may fire for a sequence that resolved"
    );
    assert_eq!(
        store.test_classifier_sample_count(partition),
        2,
        "the assertion above must not be satisfiable by a classifier that simply \
         never fired"
    );
    assert!(store.test_pending_wal_sequences(partition).is_empty());
    assert_eq!(
        store.test_wal_watermark(partition),
        Some(Ok(1)),
        "the watermark advances normally past a sequence that was never leaked"
    );
}

// ===========================================================================
// AC4 — the widened `[W, max]` re-replay window is idempotent
// ===========================================================================

/// A real `RedbDataStore`, so the merge that makes re-replay a no-op is the
/// production one. A hand-rolled double could be made to "merge" by fiat and the
/// assertion would prove nothing about the store the server actually runs.
fn redb_store(path: &std::path::Path) -> Arc<crate::storage::datastores::RedbDataStore> {
    Arc::new(crate::storage::datastores::RedbDataStore::new(path).expect("redb store"))
}

async fn stored_millis(
    store: &Arc<crate::storage::datastores::RedbDataStore>,
    key: &str,
) -> Option<u64> {
    match store.load(TEST_MAP, key).await.expect("load") {
        Some(RecordValue::Lww { timestamp, .. }) => Some(timestamp.millis),
        _ => None,
    }
}

/// The durable tombstone-byte ground truth, computed the way the boot-time
/// reconciliation computes it.
///
/// The production `reconcile_tombstone_bytes` lives in the server BINARY and
/// cannot be called from the lib test binary, so this asserts the quantity it
/// derives — the durable sum — rather than the function call.
async fn durable_tombstone_bytes(store: &Arc<crate::storage::datastores::RedbDataStore>) -> usize {
    let mut total = 0usize;
    let batch = store.scan_values(TEST_MAP, false, 0).await.expect("scan");
    for (_key, value) in &batch.records {
        if let RecordValue::OrMap { tombstones, .. } = value {
            total += tombstones.iter().map(String::len).sum::<usize>();
        }
    }
    total
}

#[tokio::test]
async fn a_re_replayed_window_lands_on_the_newest_frame_and_moves_no_tombstone_bytes() {
    let partition = 258;
    let key = keys_in_partition(partition, 1).remove(0);
    let (_wal_dir, wal) = real_wal();
    let store_dir = tempfile::tempdir().expect("tempdir");

    // ONE durable handle for the whole test: the background flush task holds an
    // `Arc` of the write-behind store, so a reopen here would race redb's file
    // lock. Re-opening is AC9's subject, not this one's.
    let redb = redb_store(&store_dir.path().join("redb"));
    {
        let store = WriteBehindDataStore::new_with_wal(
            Arc::clone(&redb) as Arc<dyn MapDataStore>,
            never_flush_config(),
            Some(WalBootstrap {
                wal: Arc::clone(&wal) as Arc<dyn Wal>,
                sequence_start: 1,
            }),
        );
        // Two framed writes for one key, both inside the widened window.
        store.add(TEST_MAP, &key, &lww(10), 0, 0).await.unwrap();
        store.add(TEST_MAP, &key, &lww(20), 0, 0).await.unwrap();
        drop(store);
    }

    let before = durable_tombstone_bytes(&redb).await;

    // Replay the window twice. Re-replay is what widening `[W, max]` makes
    // routine, so landing on the newest frame must be a property of the window,
    // not of running it exactly once.
    for _ in 0..2 {
        WalRecovery::new(Arc::clone(&wal), Vec::new())
            .run(Arc::clone(&redb) as Arc<dyn MapDataStore>)
            .await
            .expect("re-replay of an intact window must succeed");
        assert_eq!(
            stored_millis(&redb, &key).await,
            Some(20),
            "the window replays in sequence order, so it always settles on the \
             newest frame — an older frame in the same window must not be the \
             last word"
        );
    }

    // The gauge assertion kept in its only non-vacuous form: replay bypasses the
    // gauge helpers entirely, so "gauge unchanged" proves nothing — the durable
    // ground truth a boot reconciliation would recompute is what must hold.
    assert_eq!(
        durable_tombstone_bytes(&redb).await,
        before,
        "re-replaying the widened window must not move the durable tombstone \
         ground truth"
    );
}

// ===========================================================================
// AC5 — recovery stops at the contiguous-success frontier, and legacy boots
// ===========================================================================

#[tokio::test]
async fn recovery_stops_at_the_contiguous_success_frontier() {
    let partition = 259;
    let keys = keys_in_partition(partition, 3);
    let (_wal_dir, wal) = real_wal();

    {
        let inner = FaultStore::new();
        let store = build_store(&inner, Arc::clone(&wal) as Arc<dyn Wal>, 1);
        for (i, key) in keys.iter().enumerate() {
            store
                .add(TEST_MAP, key, &lww(i as u64 + 1), 0, 0)
                .await
                .unwrap();
        }
        drop(store);
    }

    // [1 Ok, 2 Err, 3 Ok]: the frontier is 1, NOT 3 — marking 3 applied would
    // license GC of the frame that failed to replay.
    let recovered = FaultStore::new();
    recovered.reject_key(&keys[1]);
    let outcome = WalRecovery::new(Arc::clone(&wal), Vec::new())
        .run(Arc::clone(&recovered) as Arc<dyn MapDataStore>)
        .await;
    assert!(outcome.is_ok(), "a failed replay must not refuse the boot");

    assert_eq!(
        wal.test_read_applied_sequence(partition),
        1,
        "the sidecar stops at the last CONTIGUOUS success"
    );
    let unapplied: Vec<u64> = wal
        .unapplied(partition)
        .await
        .unwrap()
        .iter()
        .map(|e| e.sequence)
        .collect();
    assert_eq!(
        unapplied,
        vec![2, 3],
        "both the failed frame and everything after it stay replayable"
    );

    // The stall self-heals on the next boot once the store accepts the write.
    recovered.accept_all();
    WalRecovery::new(Arc::clone(&wal), Vec::new())
        .run(Arc::clone(&recovered) as Arc<dyn MapDataStore>)
        .await
        .expect("second recovery");
    assert_eq!(wal.test_read_applied_sequence(partition), 3);
    assert!(wal.unapplied(partition).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_legacy_shaped_wal_still_boots_and_replays() {
    let partition = 260;
    let key = keys_in_partition(partition, 1).remove(0);
    let (_wal_dir, wal) = real_wal();

    // The bare-`Value` framing an older server wrote. Refusing to start on it
    // would strand every existing deployment's WAL.
    wal.append(
        partition,
        &WalEntry {
            map: TEST_MAP.to_string(),
            key: key.clone(),
            op: WalOp::Store {
                value: WalStorePayload::Legacy(Value::Int(7)),
                expiration_time: None,
            },
            timestamp: None,
            sequence: 1,
        },
    )
    .await
    .unwrap();

    let recovered = FaultStore::new();
    WalRecovery::new(Arc::clone(&wal), Vec::new())
        .run(Arc::clone(&recovered) as Arc<dyn MapDataStore>)
        .await
        .expect("a legacy WAL must boot, not refuse to start");
    assert!(recovered.contains(&key).await, "and its frame must replay");
}

// ===========================================================================
// AC9 — the durable-store assumption everything else rests on
// ===========================================================================

#[tokio::test]
async fn a_committed_redb_write_survives_a_drop_and_reopen() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("redb");

    // No write-behind and no WAL: this asserts the STORE's own durability
    // contract. Under a downgraded commit durability an un-checkpointed commit
    // does not survive the reopen — which is what makes this discriminate, and
    // which is why every resolve-on-flush-success in this spec depends on it.
    {
        let store = redb_store(&path);
        store.add(TEST_MAP, "durable", &lww(1), 0, 0).await.unwrap();
    }

    let reopened = redb_store(&path);
    assert_eq!(
        stored_millis(&reopened, "durable").await,
        Some(1),
        "a committed write must survive a drop and reopen, or advancing the \
         watermark on flush success advances past a non-durable write"
    );
}

// ===========================================================================
// AC10 — a coalesce never shifts a key's due time
// ===========================================================================

#[tokio::test]
async fn a_coalesce_leaves_the_keys_due_time_stable() {
    let partition = 261;
    let key = keys_in_partition(partition, 1).remove(0);
    let (_wal_dir, wal) = real_wal();
    let inner = FaultStore::new();
    let store = build_store(&inner, Arc::clone(&wal) as Arc<dyn Wal>, 1);

    store.add(TEST_MAP, &key, &lww(1), 0, 1_000).await.unwrap();
    store.add(TEST_MAP, &key, &lww(2), 0, 9_000).await.unwrap();

    let drained = store
        .queues
        .get_mut(&partition)
        .unwrap()
        .drain_ready(i64::MAX);
    assert_eq!(
        drained.len(),
        1,
        "the second write coalesced onto the first"
    );
    assert_eq!(
        drained[0].store_time, 1_000,
        "a coalesce carries the survivor's VALUE but never shifts WHEN the key \
         flushes: a later store_time would let a hot key defer its own flush \
         indefinitely"
    );
}

// ===========================================================================
// AC11 — the coalesce-retire routes, both directions
// ===========================================================================

#[tokio::test]
async fn a_subsuming_survivor_early_resolves_the_retired_sequence() {
    let partition = 262;
    let keys = keys_in_partition(partition, 2);
    let (_wal_dir, wal) = real_wal();
    let inner = FaultStore::new();
    let store = build_store(&inner, Arc::clone(&wal) as Arc<dyn Wal>, 1);

    // Store-onto-Store: a full snapshot subsumes its predecessor.
    store.add(TEST_MAP, &keys[0], &lww(1), 0, 0).await.unwrap();
    store.add(TEST_MAP, &keys[0], &lww(2), 0, 0).await.unwrap();
    let pending: Vec<u64> = store
        .test_pending_wal_sequences(partition)
        .iter()
        .map(|(s, _)| *s)
        .collect();
    assert_eq!(
        pending,
        vec![2],
        "the retired sequence resolves at the coalesce; leaving it pending pins \
         the partition's watermark on a repeatedly coalesced key"
    );

    // Remove-onto-Remove: the unit variant carries no payload, so the predicate
    // must live on `WalOp` itself for this row to be answerable at all.
    store.remove(TEST_MAP, &keys[1], 0).await.unwrap();
    store.remove(TEST_MAP, &keys[1], 0).await.unwrap();
    let pending: Vec<u64> = store
        .test_pending_wal_sequences(partition)
        .iter()
        .map(|(s, _)| *s)
        .collect();
    assert_eq!(pending, vec![2, 4]);
}

#[tokio::test]
async fn a_non_subsuming_survivor_carries_the_retired_sequence_forward() {
    let partition = 263;
    let key = keys_in_partition(partition, 1).remove(0);
    let (_wal_dir, wal) = real_wal();

    {
        let inner = FaultStore::new();
        let store = build_store_with(
            &inner,
            Arc::clone(&wal) as Arc<dyn Wal>,
            1,
            draining_config(),
        );
        // The survivor's framing carries only partial state, so the retired
        // frame's effect is NOT re-carried and must stay replayable.
        store.test_force_non_subsuming_survivor(true);
        inner.reject_key(&key);

        store.add(TEST_MAP, &key, &lww(1), 0, 0).await.unwrap();
        store.add(TEST_MAP, &key, &lww(2), 0, 0).await.unwrap();

        assert_eq!(
            store
                .test_pending_wal_sequences(partition)
                .iter()
                .map(|(s, _)| *s)
                .collect::<Vec<_>>(),
            vec![1, 2],
            "the survivor OWNS the retired sequence: it does not early-resolve"
        );

        // The survivor then hits an abandoned terminal, so neither sequence may
        // resolve and the watermark must stay below BOTH.
        wait_until("the survivor to exhaust its retries", || {
            store
                .test_pending_wal_sequences(partition)
                .iter()
                .all(|(_, origin)| *origin == PendingOrigin::Abandoned)
        })
        .await;
        assert_eq!(
            store.test_wal_watermark(partition),
            Some(Ok(0)),
            "the watermark stalls at the OLDEST carried sequence, not at the \
             survivor's own"
        );
        drop(store);
    }

    assert_eq!(
        wal.test_read_applied_sequence(partition),
        0,
        "nothing was marked applied, so the retired frame stays GC-ineligible"
    );
    let recovered = FaultStore::new();
    WalRecovery::new(Arc::clone(&wal), Vec::new())
        .run(Arc::clone(&recovered) as Arc<dyn MapDataStore>)
        .await
        .expect("recovery");
    assert!(
        recovered.contains(&key).await,
        "the retired frame IS replayed when the survivor is abandoned — which is \
         exactly what early-resolving it would have made impossible"
    );
}

/// A WAL whose `unapplied` fails, so boot seeding cannot complete.
struct UnseedableWal(Arc<WalWriter>);

#[async_trait]
impl Wal for UnseedableWal {
    async fn append(&self, partition: u32, entry: &WalEntry) -> anyhow::Result<()> {
        self.0.append(partition, entry).await
    }

    async fn mark_applied(&self, partition: u32, sequence: u64) -> anyhow::Result<()> {
        self.0.mark_applied(partition, sequence).await
    }

    async fn unapplied(&self, _partition: u32) -> anyhow::Result<Vec<WalEntry>> {
        anyhow::bail!("injected failure reading un-applied frames")
    }
}

#[tokio::test]
async fn an_unseeded_partition_never_advances_even_with_pending_sequences() {
    let partition = 264;
    let keys = keys_in_partition(partition, 2);
    let (_wal_dir, wal) = real_wal();

    // A prior incarnation left frames 1 and 2 un-replayed.
    {
        let inner = FaultStore::new();
        let store = build_store(&inner, Arc::clone(&wal) as Arc<dyn Wal>, 1);
        for key in &keys {
            store.add(TEST_MAP, key, &lww(1), 0, 0).await.unwrap();
        }
        drop(store);
    }

    // The new incarnation cannot seed: the WAL read fails. Its own sequences
    // start ABOVE the prior incarnation's, so a watermark computed from them
    // alone would sit above frames that were never replayed.
    let inner = FaultStore::new();
    let store = build_store(
        &inner,
        Arc::new(UnseedableWal(Arc::clone(&wal))) as Arc<dyn Wal>,
        3,
    );
    for key in &keys {
        store.add(TEST_MAP, key, &lww(2), 0, 0).await.unwrap();
    }
    assert!(!store.test_wal_partition_seeded(partition));

    // Resolve only the FIRST of the two, so the pending map is non-empty at the
    // advance — the case a seeded-only-on-empty guard lets straight through.
    store
        .resolve_and_advance(partition, &BTreeSet::from([3]))
        .await;

    assert_eq!(
        store.test_wal_watermark(partition),
        Some(Err(WalWatermarkError::Unseeded)),
        "an unseeded partition has no knowable watermark, pending or not"
    );
    assert_eq!(
        wal.test_read_applied_sequence(partition),
        0,
        "and nothing may be marked applied: frames 1 and 2 were never replayed, \
         so advancing to 2 would hand their segments to GC"
    );
    assert_eq!(
        wal.unapplied(partition).await.unwrap().len(),
        4,
        "every frame stays replayable — under-advancing is the safe direction"
    );
}
