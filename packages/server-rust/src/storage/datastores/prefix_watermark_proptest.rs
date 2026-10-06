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
use crate::storage::record::reconcile_tombstone_bytes;
use crate::storage::tombstone_gauge::with_isolated_gauge;
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
    /// Off unless a test arms it, so every scenario that does not step the
    /// store call by call sees the double exactly as before.
    gate: StepGate,
}

/// What a test decides for one parked inner-store call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    /// Record the value and return `Ok`.
    Accept,
    /// Return `Err` and record nothing.
    Refuse,
}

/// Where the store stands once nothing further happens without the test.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Settled {
    /// One inner-store call for this key is waiting for its verdict.
    Parked(String),
    /// Nothing is queued and the flush loop holds no batch.
    Idle,
}

/// Lets a test hand out the result of every inner-store call, one at a time.
///
/// A refusing store alone cannot place a write inside the flush window: the
/// window is open only while the flush loop holds a drained entry, and without
/// a park the test would have to race the loop for it. Parking each call makes
/// the window as long as the test needs it to be.
#[derive(Default)]
struct StepGate {
    state: std::sync::Mutex<GateState>,
    wake: tokio::sync::Notify,
}

#[derive(Default)]
struct GateState {
    armed: bool,
    /// The key of the call that is waiting, if one is.
    parked: Option<String>,
    /// The verdict granted to the waiting call, until that call takes it.
    verdict: Option<Verdict>,
    /// How many calls have parked since the gate was armed.
    calls: u64,
}

impl FaultStore {
    fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn gate_state(&self) -> std::sync::MutexGuard<'_, GateState> {
        self.gate
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// From now on every `add` and `remove` parks until it is granted a verdict.
    fn arm_gate(&self) {
        self.gate_state().armed = true;
    }

    fn parked(&self) -> Option<String> {
        self.gate_state().parked.clone()
    }

    fn gate_calls(&self) -> u64 {
        self.gate_state().calls
    }

    /// Releases the parked call with `verdict`.
    ///
    /// The parked mark is cleared here rather than by the released call, so a
    /// `settle` issued right after cannot mistake that call for a new one.
    fn grant(&self, verdict: Verdict) {
        {
            let mut state = self.gate_state();
            assert!(
                state.parked.take().is_some(),
                "a verdict was granted with no call parked"
            );
            state.verdict = Some(verdict);
        }
        self.gate.wake.notify_waiters();
    }

    /// Disarms the gate: the parked call, if any, and every later call is
    /// accepted.
    fn open(&self) {
        {
            let mut state = self.gate_state();
            state.armed = false;
            if state.parked.take().is_some() {
                state.verdict = Some(Verdict::Accept);
            }
        }
        self.gate.wake.notify_waiters();
    }

    /// Parks the calling inner-store call until the test grants its verdict.
    ///
    /// The armed check and the parked mark are one critical section, so `open`
    /// can never slip between them and leave a call parked behind a disarmed
    /// gate.
    async fn pass_gate(&self, key: &str) -> Verdict {
        {
            let mut state = self.gate_state();
            if !state.armed {
                return Verdict::Accept;
            }
            assert!(
                state.parked.is_none(),
                "two inner-store calls reached the gate at once"
            );
            state.parked = Some(key.to_string());
            state.calls += 1;
        }
        loop {
            // Created before the check: a grant that lands between the check
            // and the wait still wakes it.
            let granted = self.gate.wake.notified();
            if let Some(verdict) = self.gate_state().verdict.take() {
                return verdict;
            }
            granted.await;
        }
    }

    /// Waits until either one call is parked or the store is idle, and says
    /// which.
    ///
    /// Idle is the store's own witness, which cannot hold while the flush loop
    /// has a batch in hand; "every queue is empty" is also true between the
    /// drain and the first store call, with an entry in the loop's hands.
    async fn settle(&self, store: &WriteBehindDataStore, after: &str) -> Settled {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if let Some(key) = self.parked() {
                return Settled::Parked(key);
            }
            if store.test_is_idle() {
                return Settled::Idle;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "neither a parked call nor an idle store within 5 s after {after}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
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
        if self.pass_gate(key).await == Verdict::Refuse {
            anyhow::bail!("gated inner-store refusal for key={key}");
        }
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
        if self.pass_gate(key).await == Verdict::Refuse {
            anyhow::bail!("gated inner-store refusal for key={key}");
        }
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
    /// When set, a parked append that is released goes on to write its frame
    /// instead of failing: a slow append rather than a failed one.
    pass_on_release: std::sync::atomic::AtomicBool,
    released: std::sync::atomic::AtomicBool,
    wake: tokio::sync::Notify,
    /// Every frame the inner WAL accepted, in append order.
    appended: std::sync::Mutex<Vec<WalEntry>>,
}

impl FaultWal {
    fn new(inner: Arc<WalWriter>) -> Arc<Self> {
        Arc::new(Self {
            inner,
            appends: FaultAtomicU64::new(0),
            fail_on_nth: std::sync::Mutex::new(None),
            hang: std::sync::atomic::AtomicBool::new(false),
            pass_on_release: std::sync::atomic::AtomicBool::new(false),
            released: std::sync::atomic::AtomicBool::new(false),
            wake: tokio::sync::Notify::new(),
            appended: std::sync::Mutex::new(Vec::new()),
        })
    }

    /// Parks every subsequent append until [`release`](Self::release).
    fn hang_appends(&self) {
        self.hang.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// Parks every subsequent append until [`release`](Self::release), which
    /// then lets each parked append write its frame.
    fn slow_appends(&self) {
        self.pass_on_release
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self.hang_appends();
    }

    /// The frames the inner WAL accepted, in append order.
    fn appended(&self) -> Vec<WalEntry> {
        self.appended
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    async fn append_through(&self, partition: u32, entry: &WalEntry) -> anyhow::Result<()> {
        self.inner.append(partition, entry).await?;
        self.appended
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(entry.clone());
        Ok(())
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
            // Registered before the flag is read, so a release that lands in
            // between still wakes this append.
            loop {
                let notified = self.wake.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                if self.released.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }
                notified.await;
            }
            if self
                .pass_on_release
                .load(std::sync::atomic::Ordering::Relaxed)
            {
                return self.append_through(partition, entry).await;
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
            None => self.append_through(partition, entry).await,
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

    // The durable tombstone-byte ground truth is read through the PRODUCTION boot
    // reconciliation, so this test and the server share one accounting and cannot
    // drift apart. Three differences from the test-local sum this replaced, all
    // deliberate: it walks EVERY map via `list_maps` rather than `TEST_MAP` alone
    // (equivalent here, because `TEST_MAP` is the only map this module ever
    // writes), it returns `u64` rather than `usize`, and it re-baselines the
    // tombstone-bytes gauge as a side effect. That side effect is why the call is
    // scoped to a fresh isolated sink: the absolute-set path trips a
    // `debug_assert!` on the process-global gauge once ANY test in this binary has
    // fired an `add`, and that gauge is shared across the whole lib test binary.
    let (before, _scoped_gauge) = with_isolated_gauge(reconcile_tombstone_bytes(
        Arc::clone(&redb) as Arc<dyn MapDataStore>
    ))
    .await;

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
    let (after, _scoped_gauge) = with_isolated_gauge(reconcile_tombstone_bytes(
        Arc::clone(&redb) as Arc<dyn MapDataStore>
    ))
    .await;
    assert_eq!(
        after, before,
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

/// A WAL whose `unapplied` fails only while the flag is set, modelling a
/// transient read failure that later clears.
struct FlakyUnseedableWal {
    inner: Arc<WalWriter>,
    failing: Arc<std::sync::atomic::AtomicBool>,
}

#[async_trait]
impl Wal for FlakyUnseedableWal {
    async fn append(&self, partition: u32, entry: &WalEntry) -> anyhow::Result<()> {
        self.inner.append(partition, entry).await
    }

    async fn mark_applied(&self, partition: u32, sequence: u64) -> anyhow::Result<()> {
        self.inner.mark_applied(partition, sequence).await
    }

    async fn unapplied(&self, partition: u32) -> anyhow::Result<Vec<WalEntry>> {
        if self.failing.load(Ordering::Relaxed) {
            anyhow::bail!("injected transient failure reading un-applied frames");
        }
        self.inner.unapplied(partition).await
    }
}

/// The two guards compose: while a partition is unseeded the advance is REFUSED
/// however many sequences are pending, and the advance site is where seeding is
/// retried once the WAL read recovers.
///
/// Both halves are load-bearing and this asserts each against its own failure
/// mode. Removing the refuse-guard turns phase A into an over-advance above
/// never-replayed frames; removing the seed retry from `resolve_and_advance`
/// leaves phase B unseeded forever, so the sidecar never moves and the WAL grows
/// where neither alarm can see it.
#[tokio::test]
async fn an_advance_refuses_while_unseeded_and_retries_the_seed_once_the_wal_read_recovers() {
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

    let failing = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let inner = FaultStore::new();
    let store = build_store(
        &inner,
        Arc::new(FlakyUnseedableWal {
            inner: Arc::clone(&wal),
            failing: Arc::clone(&failing),
        }) as Arc<dyn Wal>,
        3,
    );

    // ---- Phase A: seeding fails at the entry point, so the advance is refused.
    // Two writes, so the pending map is NON-EMPTY at the advance — the case a
    // seeded-only-on-empty guard lets straight through.
    for key in &keys {
        store.add(TEST_MAP, key, &lww(2), 0, 0).await.unwrap();
    }
    assert!(
        !store.test_wal_partition_seeded(partition),
        "the injected read failure must leave the partition unseeded"
    );
    assert_eq!(
        store.test_wal_watermark(partition),
        Some(Err(WalWatermarkError::Unseeded)),
        "an unseeded partition has no knowable watermark, pending or not"
    );
    assert_eq!(
        wal.test_read_applied_sequence(partition),
        0,
        "nothing may be marked applied while unseeded: min(pending) - 1 would be 2, \
         which sits ABOVE frames 1 and 2 that were never replayed"
    );

    // ---- Phase B: the WAL read recovers. No further add/remove arrives for this
    // partition, so `resolve_and_advance` — reached through `flush_key` — is the
    // ONLY remaining site that can retry the seed.
    failing.store(false, Ordering::Relaxed);
    store
        .flush_key(TEST_MAP, &keys[0], &lww(2), false)
        .await
        .unwrap();

    assert!(
        store.test_wal_partition_seeded(partition),
        "the advance site must retry the seed once the WAL read recovers — \
         otherwise this partition stays unseeded forever"
    );
    assert_eq!(
        store.test_wal_watermark(partition),
        Some(Ok(0)),
        "seeding replaces the typed error with a KNOWABLE watermark that accounts \
         for the prior incarnation's un-applied frames 1 and 2"
    );
    assert_eq!(
        wal.test_read_applied_sequence(partition),
        0,
        "still 0 — frames 1 and 2 remain un-replayed, so the prefix stops below them"
    );

    // ---- Phase C: once the prior incarnation's frames resolve too, the sidecar
    // actually moves. This is what the seed bought: without it the watermark
    // would still be Err(Unseeded) here and the WAL would grow forever.
    //
    // It moves to 3, not 4: `keys[1]`'s sequence 4 was never flushed and is still
    // pending, so the prefix stops immediately below it. Advancing to 4 here
    // would be the over-advance this whole invariant exists to forbid.
    store
        .resolve_and_advance(partition, &BTreeSet::from([1, 2]))
        .await;
    assert_eq!(
        store.test_wal_watermark(partition),
        Some(Ok(3)),
        "the prefix stops below the still-pending sequence 4"
    );
    assert_eq!(
        wal.test_read_applied_sequence(partition),
        3,
        "the sidecar moves 0 -> 3 — a seeded partition can reclaim its segments"
    );
}

// ===========================================================================
// In-place writes through a shared cell: un-framed mutations (TG-WB-003 (b)),
// lead-not-lag against the resolved frames, and the shutdown drain
// ===========================================================================

use crate::service::domain::crdt::apply_or_delta;
use crate::storage::engines::HashMapStorage;
use crate::storage::impls::{DefaultRecordStore, StorageConfig};
use crate::storage::mutation_observer::CompositeMutationObserver;
use crate::storage::record::{OrMapEntry, Record, RecordMetadata};
use crate::storage::record_store::{CallerProvenance, ExpiryPolicy, MutateOutcome, RecordStore};

fn or_entry(tag: &str) -> OrMapEntry {
    OrMapEntry {
        value: Value::String(format!("v-{tag}")),
        tag: tag.to_string(),
        timestamp: Timestamp {
            millis: 1_000_000,
            counter: 0,
            node_id: "node-1".to_string(),
        },
    }
}

fn or_value(live: &[&str], tombstones: &[&str]) -> RecordValue {
    RecordValue::OrMap {
        records: live.iter().map(|t| or_entry(t)).collect(),
        tombstones: tombstones.iter().map(|t| (*t).to_string()).collect(),
    }
}

/// The live tags and the tombstones of an OR value, each sorted.
fn or_view(value: &RecordValue) -> (Vec<String>, Vec<String>) {
    match value {
        RecordValue::OrMap {
            records,
            tombstones,
        } => {
            let mut live: Vec<String> = records.iter().map(|e| e.tag.clone()).collect();
            live.sort();
            let mut tombs = tombstones.clone();
            tombs.sort();
            (live, tombs)
        }
        other => panic!("not an OrMap: {other:?}"),
    }
}

fn view(live: &[&str], tombstones: &[&str]) -> (Vec<String>, Vec<String>) {
    or_view(&or_value(live, tombstones))
}

fn is_subset(sub: &[String], sup: &[String]) -> bool {
    sub.iter().all(|t| sup.contains(t))
}

async fn inner_view(inner: &FaultStore, key: &str) -> (Vec<String>, Vec<String>) {
    or_view(
        &inner
            .load(TEST_MAP, key)
            .await
            .expect("inner load")
            .expect("the key has a durable row"),
    )
}

fn record_store_over(data_store: Arc<dyn MapDataStore>) -> Arc<DefaultRecordStore> {
    Arc::new(DefaultRecordStore::new(
        TEST_MAP.to_string(),
        0,
        Box::new(HashMapStorage::new()),
        data_store,
        Arc::new(CompositeMutationObserver::new(Vec::new())),
        StorageConfig::default(),
    ))
}

/// One OR op, applied in place the way the CRDT service's key writer applies
/// it: the closure mutates the slot and hands the write-through its witness.
#[derive(Clone, Copy)]
enum OrOp {
    Add(&'static str),
    Remove(&'static str),
}

async fn apply_or(store: &DefaultRecordStore, key: &str, op: OrOp) -> anyhow::Result<bool> {
    let mut mutate = |value: &mut RecordValue| {
        let RecordValue::OrMap {
            records,
            tombstones,
        } = value
        else {
            return MutateOutcome {
                changed: false,
                witness: None,
            };
        };
        match op {
            OrOp::Add(tag) => {
                if records.iter().any(|e| e.tag == tag) || tombstones.iter().any(|t| t == tag) {
                    return MutateOutcome {
                        changed: false,
                        witness: None,
                    };
                }
                records.push(or_entry(tag));
                MutateOutcome {
                    changed: true,
                    witness: Some(OrDelta::Add {
                        entry: or_entry(tag),
                    }),
                }
            }
            OrOp::Remove(tag) => {
                let Some(at) = records.iter().position(|e| e.tag == tag) else {
                    return MutateOutcome {
                        changed: false,
                        witness: None,
                    };
                };
                records.remove(at);
                if !tombstones.iter().any(|t| t == tag) {
                    tombstones.push(tag.to_string());
                }
                MutateOutcome {
                    changed: true,
                    witness: Some(OrDelta::Remove {
                        tag: tag.to_string(),
                    }),
                }
            }
        }
    };
    store
        .update_in_place(
            key,
            Some(or_value(&[], &[])),
            ExpiryPolicy::NONE,
            CallerProvenance::CrdtMerge,
            &mut mutate,
        )
        .await
}

/// A record store over a write-behind store over a retaining inner store, on a
/// real WAL behind the fault layer. `key` is durable as {old-1, old-2} and
/// resident (hydrated, not staged).
struct CellFixture {
    _dir: tempfile::TempDir,
    wal: Arc<WalWriter>,
    fault: Arc<FaultWal>,
    inner: Arc<FaultStore>,
    write_behind: Arc<WriteBehindDataStore>,
    store: Arc<DefaultRecordStore>,
    partition: u32,
    keys: Vec<String>,
}

impl CellFixture {
    async fn new(config: WriteBehindConfig) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let wal = WalWriter::new(dir.path().to_path_buf(), WalFsyncPolicy::PerOp).expect("wal");
        let fault = FaultWal::new(Arc::clone(&wal));
        let inner = FaultStore::new();
        let write_behind = WriteBehindDataStore::new_with_wal(
            Arc::clone(&inner) as Arc<dyn MapDataStore>,
            config,
            Some(WalBootstrap {
                wal: Arc::clone(&fault) as Arc<dyn Wal>,
                sequence_start: 1,
            }),
        );
        let partition = test_partition();
        let keys = keys_in_partition(partition, 2);
        inner
            .add(
                TEST_MAP,
                &keys[0],
                &or_value(&["old-1", "old-2"], &[]),
                0,
                0,
            )
            .await
            .expect("seed");
        let store = record_store_over(Arc::clone(&write_behind) as Arc<dyn MapDataStore>);
        store
            .get(&keys[0], false)
            .await
            .expect("hydrate")
            .expect("seeded");
        assert!(
            store.exists_in_memory(&keys[0]),
            "precondition: the key is resident"
        );
        Self {
            _dir: dir,
            wal,
            fault,
            inner,
            write_behind,
            store,
            partition,
            keys,
        }
    }

    fn key(&self) -> &str {
        &self.keys[0]
    }

    fn max_assigned(&self) -> u64 {
        self.write_behind
            .test_max_assigned_wal_sequence(self.partition)
    }

    /// No pending WAL sequence above `before` names the failed op.
    fn assert_no_pending_sequence_above(&self, before: u64) {
        let pending = self.write_behind.test_pending_wal_sequences(self.partition);
        assert!(
            pending.iter().all(|(seq, _)| *seq <= before),
            "no pending WAL sequence may name the un-acked op; pending {pending:?}, \
             last acked sequence {before}"
        );
    }

    /// op1 acked as an in-place write whose entry is queued as a cell.
    async fn ack_op1(&self) {
        assert!(apply_or(&self.store, self.key(), OrOp::Add("op1"))
            .await
            .expect("op1 must be acked"));
        assert!(
            self.write_behind
                .test_pending_cell(TEST_MAP, self.key())
                .is_some(),
            "precondition: op1 is queued as a cell entry"
        );
    }

    /// After the older entry's flush: the inner store holds `flushed` (the
    /// un-framed op included). Then crash, recover on the same durable store,
    /// and check that op1 survived, nothing outside `flushed` appeared, and
    /// that retrying `op2` on the recovered store changes nothing.
    async fn crash_recover_and_retry(self, flushed: (Vec<String>, Vec<String>), op2: OrOp) {
        let key = self.key().to_string();
        assert_eq!(
            inner_view(&self.inner, &key).await,
            flushed,
            "the older entry's flush persists the cell, un-framed op included"
        );

        let Self {
            _dir,
            wal,
            inner,
            write_behind,
            store,
            ..
        } = self;
        drop(store);
        drop(write_behind);
        WalRecovery::new(Arc::clone(&wal), Vec::new())
            .run(Arc::clone(&inner) as Arc<dyn MapDataStore>)
            .await
            .expect("recovery");
        let recovered = inner_view(&inner, &key).await;
        assert!(
            recovered.0.contains(&"op1".to_string()),
            "the acked op1 must survive the crash; recovered {recovered:?}"
        );
        assert!(
            is_subset(&recovered.0, &flushed.0) && is_subset(&recovered.1, &flushed.1),
            "recovered {recovered:?} must lie within old ∪ op1 ∪ op2 = {flushed:?}"
        );

        // The retry runs on a fresh stack over the recovered store.
        let retry_write_behind = WriteBehindDataStore::new(
            Arc::clone(&inner) as Arc<dyn MapDataStore>,
            never_flush_config(),
        );
        let retry_store =
            record_store_over(Arc::clone(&retry_write_behind) as Arc<dyn MapDataStore>);
        apply_or(&retry_store, &key, op2).await.expect("the retry");
        retry_write_behind.hard_flush().await.expect("hard_flush");
        assert_eq!(
            inner_view(&inner, &key).await,
            recovered,
            "re-applying the un-acked op on the recovered store must change nothing"
        );
    }
}

#[tokio::test]
async fn an_unframed_mutation_after_a_pre_frame_append_error_is_unacked_and_idempotent() {
    let fx = CellFixture::new(never_flush_config()).await;
    fx.ack_op1().await;

    let before = fx.max_assigned();
    fx.fault.fail_on(2, FaultClass::PreFrame);
    assert!(
        apply_or(&fx.store, fx.key(), OrOp::Add("op2"))
            .await
            .is_err(),
        "op2's append fails before its frame, so op2 is never acked"
    );
    fx.assert_no_pending_sequence_above(before);

    fx.write_behind.hard_flush().await.expect("hard_flush");
    fx.crash_recover_and_retry(
        view(&["old-1", "old-2", "op1", "op2"], &[]),
        OrOp::Add("op2"),
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unframed_mutation_rejected_by_the_shutdown_gate_is_unacked_and_idempotent() {
    let fx = CellFixture::new(never_flush_config()).await;
    fx.ack_op1().await;

    let mut park = fx.write_behind.test_park_after_shutdown_flag();
    let drain = {
        let write_behind = Arc::clone(&fx.write_behind);
        tokio::spawn(async move { write_behind.hard_flush().await })
    };
    park.wait_parked().await;

    let before = fx.max_assigned();
    assert!(
        apply_or(&fx.store, fx.key(), OrOp::Add("op2"))
            .await
            .is_err(),
        "the shutdown gate rejects op2 after its in-place mutation"
    );
    fx.assert_no_pending_sequence_above(before);

    park.release();
    drain.await.expect("drain task").expect("hard_flush");
    fx.crash_recover_and_retry(
        view(&["old-1", "old-2", "op1", "op2"], &[]),
        OrOp::Add("op2"),
    )
    .await;
}

#[tokio::test]
async fn an_unframed_or_remove_is_unacked_and_idempotent() {
    let fx = CellFixture::new(never_flush_config()).await;
    fx.ack_op1().await;

    let before = fx.max_assigned();
    fx.fault.fail_on(2, FaultClass::PreFrame);
    assert!(
        apply_or(&fx.store, fx.key(), OrOp::Remove("old-1"))
            .await
            .is_err(),
        "the OR_REMOVE's append fails before its frame, so it is never acked"
    );
    fx.assert_no_pending_sequence_above(before);

    fx.write_behind.hard_flush().await.expect("hard_flush");
    fx.crash_recover_and_retry(view(&["old-2", "op1"], &["old-1"]), OrOp::Remove("old-1"))
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cancelled_append_never_lets_the_watermark_pass_its_sequence() {
    let fx = CellFixture::new(never_flush_config()).await;
    fx.ack_op1().await;

    let before = fx.max_assigned();
    fx.fault.hang_appends();
    let op2 = {
        let store = Arc::clone(&fx.store);
        let key = fx.key().to_string();
        tokio::spawn(async move { apply_or(&store, &key, OrOp::Add("op2")).await })
    };
    let partition = fx.partition;
    wait_until("op2 to park inside its append", || {
        fx.write_behind
            .test_pending_wal_sequences(partition)
            .iter()
            .any(|(seq, origin)| *seq > before && *origin == PendingOrigin::Appending)
    })
    .await;
    let op2_seq = fx.max_assigned();
    let watermark_below_op2 = |fx: &CellFixture| {
        matches!(
            fx.write_behind.test_wal_watermark(partition),
            Some(Ok(watermark)) if watermark < op2_seq
        )
    };

    // The older entry flushes while op2 is parked inside its append.
    fx.write_behind.hard_flush().await.expect("hard_flush");
    assert!(
        watermark_below_op2(&fx),
        "the flushed watermark must not pass op2's sequence while it is appending"
    );

    op2.abort();
    assert!(
        op2.await.expect_err("aborted").is_cancelled(),
        "op2's future is dropped mid-append"
    );
    fx.fault.release();
    assert!(
        fx.write_behind
            .test_pending_wal_sequences(partition)
            .contains(&(op2_seq, PendingOrigin::Appending)),
        "a cancelled append's sequence stays appending for the life of the process"
    );
    assert!(
        watermark_below_op2(&fx),
        "the flushed watermark must still not pass the cancelled sequence"
    );

    fx.crash_recover_and_retry(
        view(&["old-1", "old-2", "op1", "op2"], &[]),
        OrOp::Add("op2"),
    )
    .await;
}

#[tokio::test]
async fn a_capacity_rejected_mutation_is_persisted_by_the_next_write_through() {
    // Entries are due at once, but only a requested flush drains them.
    let fx = CellFixture::new(WriteBehindConfig {
        capacity: 1,
        write_delay_ms: 0,
        ..never_flush_config()
    })
    .await;
    let other = fx.keys[1].clone();
    assert!(apply_or(&fx.store, &other, OrOp::Add("other-op"))
        .await
        .expect("the other key's op fills the capacity"));

    let before = fx.max_assigned();
    assert!(
        apply_or(&fx.store, fx.key(), OrOp::Add("op2"))
            .await
            .is_err(),
        "a non-staged key is rejected at capacity, after its in-place mutation"
    );
    fx.assert_no_pending_sequence_above(before);

    fx.write_behind.soft_flush().await.expect("soft_flush");
    wait_until("the other key to flush", || {
        fx.write_behind.pending_operation_count() == 0
    })
    .await;
    assert!(apply_or(&fx.store, fx.key(), OrOp::Add("op3"))
        .await
        .expect("op3 must be acked once capacity is free"));
    fx.write_behind.hard_flush().await.expect("hard_flush");

    let key = fx.key().to_string();
    let CellFixture {
        _dir,
        wal,
        inner,
        write_behind,
        store,
        ..
    } = fx;
    drop(store);
    drop(write_behind);
    WalRecovery::new(Arc::clone(&wal), Vec::new())
        .run(Arc::clone(&inner) as Arc<dyn MapDataStore>)
        .await
        .expect("recovery");
    assert_eq!(
        inner_view(&inner, &other).await,
        view(&["other-op"], &[]),
        "the other key's acked op must survive"
    );
    assert_eq!(
        inner_view(&inner, &key).await,
        view(&["old-1", "old-2", "op2", "op3"], &[]),
        "op3's write-through persists the resident value, the rejected op2 included"
    );
}

/// Folds the op of every frame of `key` at or below the partition watermark
/// onto `base` — the state the inner store must at least contain. Each frame is
/// matched to the op it was appended for by its sequence (`ops`), so a frame
/// this test did not write fails the fold.
fn fold_resolved(
    frames: &[WalEntry],
    key: &str,
    watermark: u64,
    ops: &[(u64, OrOp)],
    base: RecordValue,
) -> (Vec<String>, Vec<String>) {
    let mut value = base;
    for frame in frames
        .iter()
        .filter(|f| f.key == key && f.sequence <= watermark)
    {
        let (_, op) = ops
            .iter()
            .find(|(seq, _)| *seq == frame.sequence)
            .unwrap_or_else(|| panic!("an unexpected frame at sequence {}", frame.sequence));
        let delta = match *op {
            OrOp::Add(tag) => OrDelta::Add {
                entry: or_entry(tag),
            },
            OrOp::Remove(tag) => OrDelta::Remove {
                tag: tag.to_string(),
            },
        };
        apply_or_delta(delta, &mut value);
    }
    or_view(&value)
}

// A flush that races an op parked in its append persists a state containing
// every resolved frame (it may lead, never lag), and so does the next flush once
// the op's frame lands.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_flush_racing_an_in_flight_op_persists_every_resolved_frame() {
    // Entries are due at once, but only a requested flush drains them.
    let fx = CellFixture::new(WriteBehindConfig {
        write_delay_ms: 0,
        ..never_flush_config()
    })
    .await;
    let seed = or_value(&["old-1", "old-2"], &[]);
    let partition = fx.partition;
    let contains_resolved = |fx: &CellFixture,
                             ops: &[(u64, OrOp)],
                             stored: &(Vec<String>, Vec<String>)| {
        let Some(Ok(watermark)) = fx.write_behind.test_wal_watermark(partition) else {
            panic!("the partition is seeded");
        };
        let resolved = fold_resolved(&fx.fault.appended(), fx.key(), watermark, ops, seed.clone());
        assert!(
            is_subset(&resolved.0, &stored.0) && is_subset(&resolved.1, &stored.1),
            "the inner store {stored:?} must contain every resolved frame {resolved:?} \
             (watermark {watermark})"
        );
        watermark
    };

    fx.ack_op1().await;
    let before = fx.max_assigned();
    fx.fault.slow_appends();
    let op2 = {
        let store = Arc::clone(&fx.store);
        let key = fx.key().to_string();
        tokio::spawn(async move { apply_or(&store, &key, OrOp::Add("op2")).await })
    };
    wait_until("op2 to park inside its append", || {
        fx.write_behind
            .test_pending_wal_sequences(partition)
            .iter()
            .any(|(seq, origin)| *seq > before && *origin == PendingOrigin::Appending)
    })
    .await;
    let ops = [
        (before, OrOp::Add("op1")),
        (fx.max_assigned(), OrOp::Add("op2")),
    ];

    fx.write_behind.soft_flush().await.expect("soft_flush");
    wait_until("op1's entry to flush", || {
        fx.write_behind.pending_operation_count() == 0
    })
    .await;
    let first = inner_view(&fx.inner, fx.key()).await;
    let watermark = contains_resolved(&fx, &ops, &first);
    assert_eq!(watermark, before, "op1's frame is resolved, op2's is not");
    assert!(
        first.0.contains(&"op2".to_string()),
        "the flush may lead the resolved frames: it persisted op2 before op2's frame"
    );

    fx.fault.release();
    assert!(op2
        .await
        .expect("op2 task")
        .expect("op2 is acked once its frame lands"));
    fx.write_behind.soft_flush().await.expect("soft_flush");
    wait_until("op2's entry to flush", || {
        fx.write_behind.pending_operation_count() == 0
    })
    .await;
    let second = inner_view(&fx.inner, fx.key()).await;
    assert!(
        contains_resolved(&fx, &ops, &second) > before,
        "op2's frame is resolved by the second flush"
    );
    assert_eq!(second, view(&["old-1", "old-2", "op1", "op2"], &[]));
}

/// How one pending entry for the shutdown-drain parity checks is written.
#[derive(Clone, Copy, Debug)]
enum EntryKind {
    Value,
    Cell,
}

async fn write_entry(store: &WriteBehindDataStore, key: &str, kind: EntryKind, tag: &str) {
    let value = or_value(&[tag], &[]);
    match kind {
        EntryKind::Value => store.add(TEST_MAP, key, &value, 0, 1).await,
        EntryKind::Cell => {
            let cell = crate::storage::engine::new_slot_cell(Record {
                value,
                metadata: RecordMetadata::new(0, 0),
            });
            store
                .add_with_witness(TEST_MAP, key, WriteSource::Cell(&cell), 0, 1, None)
                .await
        }
    }
    .expect("write");
    assert_eq!(
        store.test_pending_cell(TEST_MAP, key).is_some(),
        matches!(kind, EntryKind::Cell),
        "the entry must be queued as {kind:?}"
    );
}

// `hard_flush` drains cell entries alongside value entries within the timeout.
#[tokio::test]
async fn hard_flush_drains_cell_entries_like_value_entries() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (_wal, _fault, inner, store) = fault_wal_store(&dir);
    let partition = test_partition();
    let keys = keys_in_partition(partition, 4);
    let kinds = [
        EntryKind::Cell,
        EntryKind::Value,
        EntryKind::Cell,
        EntryKind::Cell,
    ];
    for (key, kind) in keys.iter().zip(kinds) {
        write_entry(&store, key, kind, key).await;
    }

    store.hard_flush().await.expect("hard_flush");

    for key in &keys {
        assert_eq!(
            inner_view(&inner, key).await,
            view(&[key.as_str()], &[]),
            "every entry is drained"
        );
        assert!(store.test_pending_cell(TEST_MAP, key).is_none());
        assert!(store.test_staged_cell(TEST_MAP, key).is_none());
    }
    assert!(
        store.test_pending_wal_sequences(partition).is_empty(),
        "every drained entry resolves its WAL sequence"
    );
    assert_eq!(store.pending_operation_count(), 0);
}

/// Drives one entry of `kind` into a shutdown drain whose inner-store write
/// hangs past the timeout, either in the drain itself or in the flush loop's
/// in-flight batch (`in_loop`). Returns the entry's WAL disposition after the
/// drain and whether its frame replays on the next boot.
async fn timed_out_drain(kind: EntryKind, in_loop: bool) -> (Vec<PendingOrigin>, bool) {
    let dir = tempfile::tempdir().expect("tempdir");
    let wal = WalWriter::new(dir.path().to_path_buf(), WalFsyncPolicy::PerOp).expect("wal");
    let inner = FaultStore::new();
    let config = WriteBehindConfig {
        write_delay_ms: if in_loop { 0 } else { 600_000 },
        ..never_flush_config()
    };
    let store = WriteBehindDataStore::new_with_wal(
        Arc::clone(&inner) as Arc<dyn MapDataStore>,
        config,
        Some(WalBootstrap {
            wal: Arc::clone(&wal) as Arc<dyn Wal>,
            sequence_start: 1,
        }),
    );
    let partition = test_partition();
    let key = keys_in_partition(partition, 1).remove(0);
    inner.hang_key(&key);
    write_entry(&store, &key, kind, "tag").await;
    if in_loop {
        store.soft_flush().await.expect("soft_flush");
        wait_until("the flush loop to take the entry in flight", || {
            !store.test_in_flight_wal_sequences(partition).is_empty()
        })
        .await;
    }

    store
        .hard_flush()
        .await
        .expect("hard_flush returns at the timeout");
    let disposition = store
        .test_pending_wal_sequences(partition)
        .into_iter()
        .map(|(_, origin)| origin)
        .collect();

    inner.release();
    drop(store);
    let recovered = FaultStore::new();
    WalRecovery::new(Arc::clone(&wal), Vec::new())
        .run(Arc::clone(&recovered) as Arc<dyn MapDataStore>)
        .await
        .expect("recovery");
    (disposition, recovered.contains(&key).await)
}

// A cell entry the drain times out on is abandoned and replayed exactly as a
// value entry is.
#[tokio::test]
async fn a_timed_out_cell_entry_is_abandoned_and_replayed_like_a_value_entry() {
    let value = timed_out_drain(EntryKind::Value, false).await;
    let cell = timed_out_drain(EntryKind::Cell, false).await;
    assert_eq!(
        value,
        (vec![PendingOrigin::Abandoned], true),
        "control: a timed-out value entry is abandoned and its frame replays"
    );
    assert_eq!(
        cell, value,
        "a cell entry must behave exactly as a value entry"
    );
}

// A cell entry in the flush loop's batch when the drain aborts the loop is left
// neither resolved nor abandoned and replays, exactly as a value entry does.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_aborted_in_flight_cell_entry_replays_like_a_value_entry() {
    let value = timed_out_drain(EntryKind::Value, true).await;
    let cell = timed_out_drain(EntryKind::Cell, true).await;
    assert!(
        value.1,
        "control: an aborted in-flight value entry replays on the next boot"
    );
    assert_eq!(
        cell, value,
        "a cell entry must behave exactly as a value entry"
    );
}

// ===========================================================================
// Refusals interleaved with writes to one key: whatever leaves a queue or a
// flush batch is accounted
// ===========================================================================

/// Drains at once and retries almost at once, so a stepped run spends its time
/// in the steps rather than in waits.
fn stepped_config(max_retries: u32) -> WriteBehindConfig {
    WriteBehindConfig {
        write_delay_ms: 0,
        flush_interval_ms: 5,
        backoff_base_ms: 1,
        backoff_cap_ms: 2,
        capacity: 0,
        max_retries,
        ..WriteBehindConfig::default()
    }
}

/// A write-behind store on a real WAL over a gated inner store, with two keys
/// of one partition.
struct Stepped {
    _dir: tempfile::TempDir,
    wal: Arc<WalWriter>,
    inner: Arc<FaultStore>,
    store: Arc<WriteBehindDataStore>,
    partition: u32,
    keys: Vec<String>,
}

impl Stepped {
    fn new(partition: u32, config: WriteBehindConfig) -> Self {
        let (dir, wal) = real_wal();
        let inner = FaultStore::new();
        inner.arm_gate();
        let store = build_store_with(&inner, Arc::clone(&wal) as Arc<dyn Wal>, 1, config);
        Self {
            _dir: dir,
            wal,
            inner,
            store,
            partition,
            keys: keys_in_partition(partition, 2),
        }
    }

    fn key(&self) -> &str {
        &self.keys[0]
    }

    async fn settle(&self, after: &str) -> Settled {
        self.inner.settle(&self.store, after).await
    }

    fn pending(&self) -> Vec<(u64, PendingOrigin)> {
        self.store.test_pending_wal_sequences(self.partition)
    }

    fn abandoned(&self) -> Vec<u64> {
        self.pending()
            .into_iter()
            .filter(|(_, origin)| *origin == PendingOrigin::Abandoned)
            .map(|(sequence, _)| sequence)
            .collect()
    }

    /// The WAL sequences owned by the queued entry of the first key, if one is
    /// queued.
    fn queued_wal_sequences(&self) -> Option<Vec<u64>> {
        let slot = (TEST_MAP.to_string(), self.key().to_string());
        self.store.queues.get(&self.partition).and_then(|queue| {
            queue
                .entries
                .get(&slot)
                .map(|entry| entry.wal_sequences.iter().copied().collect())
        })
    }

    /// Accepts everything from here on and waits for the store to go idle.
    async fn open_and_drain(&self) {
        self.inner.open();
        assert_eq!(
            self.settle("the gate was opened").await,
            Settled::Idle,
            "an open gate parks nothing, so the store can only settle idle"
        );
    }

    fn classifier_samples(&self) -> (Option<WalWatermarkAlarm>, Option<WalWatermarkAlarm>) {
        // Two, because a leak is reported only for a sequence seen ownerless on
        // two consecutive samples.
        (
            self.store.test_run_classifier_sample(self.partition),
            self.store.test_run_classifier_sample(self.partition),
        )
    }
}

fn parked_on(key: &str) -> Settled {
    Settled::Parked(key.to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Write,
    Remove,
    Refuse,
    Accept,
}

/// The last thing the client asked for, which is what the inner store must
/// hold once nothing was abandoned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LastOp {
    Value(u64),
    Tombstone,
}

/// Runs steps against the first key, settling after each one.
///
/// At every settle point the store is either parked on one call or idle, and
/// which of the two follows from the steps alone: the flush loop is the only
/// other task, and it is either blocked in the gate or between passes.
struct Script {
    at: Settled,
    /// Whether the key was written or removed since the parked call parked —
    /// that is, while its entry was in the flush batch.
    written_in_window: bool,
    next_value: u64,
    last: Option<LastOp>,
}

impl Script {
    fn new() -> Self {
        Self {
            at: Settled::Idle,
            written_in_window: false,
            next_value: 0,
            last: None,
        }
    }

    async fn run(&mut self, stepped: &Stepped, step: Step) -> Result<(), String> {
        let parked = matches!(self.at, Settled::Parked(_));
        match step {
            Step::Write => {
                self.next_value += 1;
                stepped
                    .store
                    .add(TEST_MAP, stepped.key(), &lww(self.next_value), 0, 0)
                    .await
                    .map_err(|err| format!("a write was not accepted: {err}"))?;
                self.last = Some(LastOp::Value(self.next_value));
                self.written_in_window |= parked;
                self.at = stepped.settle("a write").await;
            }
            Step::Remove => {
                stepped
                    .store
                    .remove(TEST_MAP, stepped.key(), 0)
                    .await
                    .map_err(|err| format!("a remove was not accepted: {err}"))?;
                self.last = Some(LastOp::Tombstone);
                self.written_in_window |= parked;
                self.at = stepped.settle("a remove").await;
            }
            Step::Refuse | Step::Accept if !parked => {
                // Nothing is waiting for a verdict, so there is nothing to grant.
                self.at = stepped.settle("a verdict with no call parked").await;
            }
            Step::Refuse | Step::Accept => {
                let refusal_in_window = step == Step::Refuse && self.written_in_window;
                let abandoned_before = stepped.abandoned();
                stepped.inner.grant(if step == Step::Refuse {
                    Verdict::Refuse
                } else {
                    Verdict::Accept
                });
                self.written_in_window = false;
                self.at = stepped.settle("a verdict").await;
                // A newer write of the key is queued — it could not be drained
                // while the loop was blocked in the gate — so the refused entry
                // has a survivor to be retired into, whether or not it had
                // retries left. Abandoning it would pin the partition's WAL
                // for a key that does become durable.
                let abandoned_after = stepped.abandoned();
                if refusal_in_window && abandoned_after != abandoned_before {
                    return Err(format!(
                        "a refused entry with a queued newer write was abandoned: \
                         abandoned sequences went from {abandoned_before:?} to {abandoned_after:?}"
                    ));
                }
            }
        }
        Ok(())
    }
}

/// The quiescence predicate, on an idle store that accepts writes again.
///
/// One conjunct at a time and in a fixed order, so a failure names the first
/// one that is false. The reads are not atomic and do not need to be: an idle
/// store stays idle until the test writes again.
async fn quiescent(stepped: &Stepped, last: Option<LastOp>) -> Result<(), String> {
    let partition = stepped.partition;

    let count = stepped.store.pending_operation_count();
    if count != 0 {
        return Err(format!("Q1 pending_operation_count() == 0: it is {count}"));
    }

    let flushed = stepped.store.flushed_watermark();
    let assigned = stepped.store.assigned_write_sequence();
    if flushed != assigned {
        return Err(format!(
            "Q2 flushed_watermark() == assigned_write_sequence(): {flushed} != {assigned}"
        ));
    }

    let pending = stepped.pending();
    if pending
        .iter()
        .any(|(_, origin)| matches!(origin, PendingOrigin::Live | PendingOrigin::Appending))
    {
        return Err(format!(
            "Q3 no Live and no Appending WAL sequence: {pending:?}"
        ));
    }

    let in_flight = stepped.store.test_in_flight_wal_sequences(partition);
    if !in_flight.is_empty() {
        return Err(format!("Q4 no WAL sequence in flight: {in_flight:?}"));
    }

    let samples = stepped.classifier_samples();
    if samples.0 == Some(WalWatermarkAlarm::TrackerLeak)
        || samples.1 == Some(WalWatermarkAlarm::TrackerLeak)
    {
        return Err(format!("Q5 no tracker leak: {samples:?}"));
    }

    // With no client op there is no last value, and the partition was never
    // tracked, so the remaining conjunct has no subject.
    let Some(last) = last else {
        return Ok(());
    };
    // A discard with no newer write queued abandons by design; the frame then
    // stays pending and the store need not hold the last op.
    if !stepped.abandoned().is_empty() {
        return Ok(());
    }
    if !pending.is_empty() {
        return Err(format!("Q6 nothing pending: {pending:?}"));
    }
    let watermark = stepped.store.test_wal_watermark(partition);
    let max_assigned = stepped.store.test_max_assigned_wal_sequence(partition);
    if watermark != Some(Ok(max_assigned)) {
        return Err(format!(
            "Q6 the watermark is at the highest assigned sequence: {watermark:?}, \
             highest assigned {max_assigned}"
        ));
    }
    let held = stepped
        .inner
        .load(TEST_MAP, stepped.key())
        .await
        .map_err(|err| format!("the inner store could not be read: {err}"))?;
    let holds_last = match last {
        LastOp::Value(millis) => held == Some(lww(millis)),
        LastOp::Tombstone => stepped.inner.is_tombstone(stepped.key()).await,
    };
    if !holds_last {
        return Err(format!(
            "Q6 the inner store holds the last client op {last:?}: it holds {held:?}"
        ));
    }
    Ok(())
}

async fn interleaving_case(
    steps: Vec<Step>,
    max_retries: u32,
    non_subsuming: bool,
) -> Result<(), String> {
    let stepped = Stepped::new(265, stepped_config(max_retries));
    stepped
        .store
        .test_force_non_subsuming_survivor(non_subsuming);

    let mut script = Script::new();
    for step in steps {
        script.run(&stepped, step).await?;
    }
    stepped.open_and_drain().await;
    quiescent(&stepped, script.last).await?;

    // The property runs on a process-wide runtime, so without this the flush
    // loop and the watchdog of every case would keep running beside the later
    // cases and compete with their settle bound.
    stepped
        .store
        .hard_flush()
        .await
        .map_err(|err| format!("shutting down an idle store failed: {err}"))
}

fn any_step() -> impl Strategy<Value = Step> {
    prop_oneof![
        Just(Step::Write),
        Just(Step::Remove),
        Just(Step::Refuse),
        Just(Step::Accept),
    ]
}

proptest! {
    // No regression file: the predicate is only ever read at a settle point,
    // and a settle bound that expires on a slow machine must not be persisted
    // as a counterexample. The fixed counterexamples are the pinned tests below.
    #![proptest_config(ProptestConfig {
        cases: 64,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// However refusals, acceptances, writes and removes of one key interleave,
    /// a store that accepts writes again and has gone idle counts nothing that
    /// is not queued, fences nothing, and owns every WAL sequence it tracks.
    #[test]
    fn interleaved_refusals_and_writes_to_one_key_leave_nothing_unaccounted(
        steps in proptest::collection::vec(any_step(), 1..24),
        max_retries in 1u32..=3,
        non_subsuming in any::<bool>(),
    ) {
        let outcome = block_on_async(interleaving_case(steps, max_retries, non_subsuming));
        prop_assert_eq!(outcome, Ok(()));
    }
}

#[tokio::test]
async fn a_write_in_the_flush_window_then_a_refusal_leaves_nothing_unaccounted() {
    let stepped = Stepped::new(266, stepped_config(3));
    let mut script = Script::new();
    for step in [Step::Write, Step::Write, Step::Refuse] {
        script.run(&stepped, step).await.expect("step");
    }
    // Recorded now, while the store still refuses, and asserted only after the
    // accounting below: the leak is the finding, this is the proof that the
    // first write did meet the second one in the window.
    let survivor_wal = stepped
        .store
        .test_in_flight_wal_sequences(stepped.partition);

    script.run(&stepped, Step::Accept).await.expect("step");
    stepped.open_and_drain().await;
    if let Err(violated) = quiescent(&stepped, script.last).await {
        panic!("{violated}");
    }

    assert_eq!(
        survivor_wal,
        vec![1, 2],
        "the newer write owns the refused entry's WAL sequence until it is durable"
    );
}

#[tokio::test]
async fn a_superseded_retry_entry_stays_replayable_until_its_survivor_is_durable() {
    let config = WriteBehindConfig {
        or_delta_wal: true,
        ..stepped_config(2)
    };
    let stepped = Stepped::new(267, config);
    let partition = stepped.partition;
    let (key, other_key) = (stepped.keys[0].as_str(), stepped.keys[1].as_str());
    let add_tag = |tag: &str| OrDelta::Add {
        entry: or_entry(tag),
    };

    // Real delta frames: each carries one mutation, so neither makes the other
    // redundant and a lost frame is a lost mutation.
    stepped
        .store
        .add_with_witness(
            TEST_MAP,
            key,
            WriteSource::Value(&or_value(&["a"], &[])),
            0,
            0,
            Some(&add_tag("a")),
        )
        .await
        .unwrap();
    assert_eq!(stepped.settle("the first add").await, parked_on(key));
    stepped
        .store
        .add_with_witness(
            TEST_MAP,
            key,
            WriteSource::Value(&or_value(&["a", "b"], &[])),
            0,
            0,
            Some(&add_tag("b")),
        )
        .await
        .unwrap();

    // The first entry meets the second in the queue and is retired into it.
    stepped.inner.grant(Verdict::Refuse);
    assert_eq!(stepped.settle("the first refusal").await, parked_on(key));
    let pending_after_supersede = stepped.pending();

    // A second key of the same partition: its flush is what can move the
    // partition's watermark past a sequence that was resolved too early.
    stepped
        .store
        .add(TEST_MAP, other_key, &lww(1), 0, 0)
        .await
        .unwrap();
    stepped.inner.grant(Verdict::Refuse);
    assert_eq!(
        stepped.settle("the survivor's first refusal").await,
        parked_on(key),
        "the requeued survivor is first in the batch it shares with the other key"
    );
    // Its last attempt, with no newer write queued: abandoned.
    stepped.inner.grant(Verdict::Refuse);
    assert_eq!(
        stepped.settle("the survivor's last refusal").await,
        parked_on(other_key)
    );
    stepped.inner.grant(Verdict::Accept);
    assert_eq!(stepped.settle("the other key's flush").await, Settled::Idle);

    let applied = stepped.wal.test_read_applied_sequence(partition);

    // Crash: only the WAL survives.
    let wal = Arc::clone(&stepped.wal);
    let key = key.to_string();
    let Stepped {
        _dir: wal_dir,
        store,
        ..
    } = stepped;
    drop(store);
    let recovered_store = FaultStore::new();
    WalRecovery::new(wal, Vec::new())
        .run(Arc::clone(&recovered_store) as Arc<dyn MapDataStore>)
        .await
        .expect("recovery");
    let recovered = inner_view(&recovered_store, &key).await;
    drop(wal_dir);

    assert_eq!(
        (pending_after_supersede, applied, recovered),
        (
            vec![(1, PendingOrigin::Live), (2, PendingOrigin::Live)],
            0,
            view(&["a", "b"], &[]),
        ),
        "the retired frame must stay pending, un-applied and replayable until the \
         entry that took it over is durable; resolving it at the supersede lets the \
         other key's flush move the watermark past it, and the acked `a` is then \
         filtered out of replay"
    );
}

#[tokio::test]
async fn a_subsuming_write_over_a_survivor_resolves_the_carried_sequences() {
    // The first backoff is min(base * 2, cap). It has to be long: the third
    // write below must land inside it, and with a short one that would be a
    // race against the flush loop instead of a two-second window.
    let config = WriteBehindConfig {
        backoff_base_ms: 1000,
        backoff_cap_ms: 5000,
        ..stepped_config(3)
    };
    let stepped = Stepped::new(268, config);
    let key = stepped.key().to_string();

    stepped
        .store
        .add(TEST_MAP, &key, &lww(1), 0, 0)
        .await
        .unwrap();
    assert_eq!(stepped.settle("the first write").await, parked_on(&key));
    stepped
        .store
        .add(TEST_MAP, &key, &lww(2), 0, 0)
        .await
        .unwrap();

    let calls_at_refusal = stepped.inner.gate_calls();
    stepped.inner.grant(Verdict::Refuse);
    // Not settled on purpose: the loop now sleeps its backoff with the pass
    // still open, and the point is what a write does during that sleep.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    while stepped.queued_wal_sequences() != Some(vec![1, 2]) {
        assert!(
            std::time::Instant::now() < deadline,
            "the queued newer write did not take over the refused entry's WAL \
             sequence within 1 s: it owns {:?}",
            stepped.queued_wal_sequences()
        );
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    }
    // The hand-over is visible, so the loop has suspended since, and the first
    // place it can suspend is the backoff sleep. The retired entry's count and
    // entry sequence must be settled by then: a task can be dropped only where
    // it suspends, so settling them after the sleep would leave both wrong for
    // the whole backoff, and for good if the task were aborted in it.
    assert_eq!(
        (
            stepped.store.pending_operation_count(),
            stepped.store.flushed_watermark(),
        ),
        (1, 1),
        "inside the backoff one entry is queued and counted, and the flushed \
         watermark stands at the queued entry's sequence (entry sequences start \
         at 0), no longer held at the retired one"
    );

    stepped
        .store
        .add(TEST_MAP, &key, &lww(3), 0, 0)
        .await
        .unwrap();
    assert_eq!(
        stepped.pending(),
        vec![(3, PendingOrigin::Live)],
        "a full-value write over the survivor makes every frame it carried \
         redundant, so the carried set cannot grow without bound"
    );
    assert_eq!(
        stepped.inner.gate_calls(),
        calls_at_refusal,
        "the third write must have coalesced inside the backoff, before the \
         survivor was drained"
    );

    stepped.open_and_drain().await;
    if let Err(violated) = quiescent(&stepped, Some(LastOp::Value(3))).await {
        panic!("{violated}");
    }
}

#[tokio::test]
async fn a_discarded_entry_with_a_queued_newer_write_is_retired_into_it() {
    // One attempt only, so the first refusal is the discard.
    let stepped = Stepped::new(269, stepped_config(1));
    let partition = stepped.partition;
    let key = stepped.key().to_string();

    stepped
        .store
        .add(TEST_MAP, &key, &lww(1), 0, 0)
        .await
        .unwrap();
    assert_eq!(stepped.settle("the first write").await, parked_on(&key));
    stepped
        .store
        .add(TEST_MAP, &key, &lww(2), 0, 0)
        .await
        .unwrap();

    stepped.inner.grant(Verdict::Refuse);
    assert_eq!(
        stepped.settle("the discard").await,
        parked_on(&key),
        "the newer write is drained next"
    );
    let after_discard = stepped.pending();

    stepped.inner.grant(Verdict::Accept);
    assert_eq!(
        stepped.settle("the newer write's flush").await,
        Settled::Idle
    );

    assert_eq!(
        (
            after_discard,
            stepped.pending(),
            stepped.store.test_wal_watermark(partition),
            stepped.store.pending_operation_count(),
            (
                stepped.store.flushed_watermark(),
                stepped.store.assigned_write_sequence(),
            ),
            stepped.classifier_samples(),
        ),
        (
            vec![(1, PendingOrigin::Live), (2, PendingOrigin::Live)],
            Vec::new(),
            Some(Ok(2)),
            0,
            (2, 2),
            (None, None),
        ),
        "the key became durable through the newer write, so the discarded \
         entry's frame must not stay pinned as abandoned"
    );
    assert_eq!(
        stepped.inner.load(TEST_MAP, &key).await.unwrap(),
        Some(lww(2))
    );
}

#[tokio::test]
async fn a_discarded_delta_entry_stays_replayable_until_its_survivor_is_durable() {
    // One attempt only, so every refusal is a discard.
    let config = WriteBehindConfig {
        or_delta_wal: true,
        ..stepped_config(1)
    };
    let stepped = Stepped::new(270, config);
    let partition = stepped.partition;
    let (key, other_key) = (stepped.keys[0].as_str(), stepped.keys[1].as_str());
    let add_tag = |tag: &str| OrDelta::Add {
        entry: or_entry(tag),
    };

    // Real delta frames, as in the retry-path test above: the second frame
    // does not repeat the first one's mutation, so a lost frame is a lost tag.
    stepped
        .store
        .add_with_witness(
            TEST_MAP,
            key,
            WriteSource::Value(&or_value(&["a"], &[])),
            0,
            0,
            Some(&add_tag("a")),
        )
        .await
        .unwrap();
    assert_eq!(stepped.settle("the first add").await, parked_on(key));
    stepped
        .store
        .add_with_witness(
            TEST_MAP,
            key,
            WriteSource::Value(&or_value(&["a", "b"], &[])),
            0,
            0,
            Some(&add_tag("b")),
        )
        .await
        .unwrap();

    // The first entry is discarded with the second one queued, and is retired
    // into it.
    stepped.inner.grant(Verdict::Refuse);
    assert_eq!(stepped.settle("the discard").await, parked_on(key));
    let pending_after_discard = stepped.pending();

    // A second key of the same partition: its flush is what can move the
    // partition's watermark past a sequence that was resolved too early.
    stepped
        .store
        .add(TEST_MAP, other_key, &lww(1), 0, 0)
        .await
        .unwrap();
    // The survivor is discarded too, with nothing queued for its key.
    stepped.inner.grant(Verdict::Refuse);
    assert_eq!(
        stepped.settle("the survivor's discard").await,
        parked_on(other_key)
    );
    stepped.inner.grant(Verdict::Accept);
    assert_eq!(stepped.settle("the other key's flush").await, Settled::Idle);

    let pending_at_idle = stepped.pending();
    let applied = stepped.wal.test_read_applied_sequence(partition);

    // Crash: only the WAL survives.
    let wal = Arc::clone(&stepped.wal);
    let key = key.to_string();
    let Stepped {
        _dir: wal_dir,
        store,
        ..
    } = stepped;
    drop(store);
    let recovered_store = FaultStore::new();
    WalRecovery::new(wal, Vec::new())
        .run(Arc::clone(&recovered_store) as Arc<dyn MapDataStore>)
        .await
        .expect("recovery");
    let recovered = inner_view(&recovered_store, &key).await;
    drop(wal_dir);

    assert_eq!(
        (pending_after_discard, pending_at_idle, applied, recovered),
        (
            vec![(1, PendingOrigin::Live), (2, PendingOrigin::Live)],
            vec![(1, PendingOrigin::Abandoned), (2, PendingOrigin::Abandoned)],
            0,
            view(&["a", "b"], &[]),
        ),
        "the discarded frame must stay pending, un-applied and replayable for as \
         long as the entry that took it over is not durable; resolving it at the \
         discard lets the other key's flush move the watermark past it, and the \
         acked `a` is then filtered out of replay"
    );
}

/// A queued entry of key `k` with the given due time and WAL sequence.
fn queued_entry(store_time: i64, sequence: u64) -> DelayedEntry {
    DelayedEntry {
        map: TEST_MAP.to_string(),
        key: "k".to_string(),
        operation: DelayedOp::Store {
            value: lww(sequence),
            expiration_time: 0,
        },
        store_time,
        sequence,
        retry_count: 0,
        wal_sequences: BTreeSet::from([sequence]),
    }
}

#[test]
fn a_retired_entry_hands_its_earlier_due_time_to_the_queued_write() {
    // The outage case: the first write was due at 1000 and was refused; the
    // key was written again at 9000 while the first entry was out of the queue.
    let mut queue = PartitionQueue::default();
    queue.insert(queued_entry(9_000, 2));
    assert!(queue.retire_into_queued(&queued_entry(1_000, 1)));
    let due = queue.drain_ready(1_000);
    assert_eq!(
        due.iter()
            .map(|entry| (entry.sequence, entry.store_time))
            .collect::<Vec<_>>(),
        vec![(2, 1_000)],
        "the queued write is due when the retired one was: left at its own later \
         time, a key rewritten during an outage would lose its original flush \
         schedule"
    );

    // The earlier time wins, whichever entry holds it: a retired entry that
    // was enqueued later does not push the queued one back.
    let mut queue = PartitionQueue::default();
    queue.insert(queued_entry(1_000, 2));
    assert!(queue.retire_into_queued(&queued_entry(9_000, 1)));
    let due = queue.drain_ready(1_000);
    assert_eq!(
        due.iter()
            .map(|entry| (entry.sequence, entry.store_time))
            .collect::<Vec<_>>(),
        vec![(2, 1_000)],
        "the queued write keeps its own earlier due time"
    );
}
