//! The crash/recovery harness executor.
//!
//! Applies a generated [`Case`] to a REAL [`WriteBehindDataStore`] + real
//! [`WalWriter`], crosses incarnation boundaries with real crashes and real
//! [`WalRecovery::run`], and evaluates the two model-based oracles after the
//! steps the spec mandates. The reference model ([`LogModel`]) is built
//! EXCLUSIVELY from the ops the driver issued (and their ack outcomes) plus the
//! fault schedule the driver injected; it never reads the implementation's own
//! pending / in-flight / seeded / staging state to compute an expected value.
//!
//! # Model↔implementation independence (the binding rule)
//!
//! The only fact the model learns from the implementation is a sequence NUMBER,
//! at append time, via the `test_set_append_observer` seam in `write_behind.rs`.
//! That transfers an identifier ("this write is called `s`"), never a lifecycle:
//! every `Appended -> Acked -> DurablyApplied` / `Indeterminate` transition is
//! decided by the model alone from the driver's own event log. The single
//! sanctioned read of implementation state — [`WriteBehindDataStore::test_wal_partition_seeded`]
//! and `test_pending_wal_sequences` at boot — is a CHECK that the rebuilt store
//! starts empty (the crash-vacuity guard), and nothing derived from it flows
//! into the model.
//!
//! # Crash model limit
//!
//! An `IncarnationEnd::Crash` DESTROYS the in-memory `WriteBehindDataStore` (its
//! background tasks are signalled to stop and the `Arc` is dropped, so the
//! staging buffer, pending tracker, in-flight registry, queues and seeded-set
//! all vanish) while PRESERVING the durable artefacts (the on-disk WAL directory
//! and the retained inner store's contents). It does NOT model OS page-cache
//! loss — an un-fsynced byte the OS still holds survives an in-process drop — so
//! this harness cannot bound that loss window; that is a categorically different
//! crash model and is out of scope (see `wal_harness/mod.rs`'s module doc).
//!
//! # Inner-store double
//!
//! The default inner store is [`HarnessInnerStore`], a RETAINING map double with
//! a reject-all health toggle — never a discarding `NullDataStore`, which would
//! make a lost frame and a working replay look identical. It is retained across
//! incarnations so it models the durable on-disk file a crash preserves. Its
//! reject state persists across the crash too, modelling a backend that is still
//! down after a restart — which is the state that makes C12 (empty boot seed)
//! reachable, since only a recovery that cannot replay a frame leaves work for
//! the boot-seed to protect.
//!
//! Reopening a real `RedbDataStore` in-process while its file lock races the
//! background flush task is flaky, so the redb drop-and-reopen case is recorded
//! as a documented limitation here rather than driven — the retaining in-memory
//! double exercises the identical drop/preserve/reopen control flow.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex as AsyncMutex;
use topgun_core::hlc::Timestamp;
use topgun_core::types::Value;

use super::super::{
    partition_for, DelayedOp, WalBootstrap, WatermarkMode, WriteBehindConfig, WriteBehindDataStore,
};
use super::{
    BootSeedMode, Case, CaseShape, DefectMode, GcCrashPoint, IncarnationEnd, InvariantViolation,
    Key, ModelValue, OracleConfig, OracleCoverage, OracleCoverageFloors, PartitionId,
    ReferenceModel, Sequence, WorkOp, MAX_KEY_INDEX,
};
use crate::storage::map_data_store::{LeafSink, MapDataStore, ScanBatch, ScanCursor};
use crate::storage::record::RecordValue;
use crate::storage::wal::{
    GcCrashPoint as WalGcCrashPoint, GcOrderMode as WalGcOrderMode, Wal, WalFsyncPolicy,
    WalRecovery, WalWriter,
};

/// The single map every harness key lives in.
const TEST_MAP: &str = "wm";

/// The driver's virtual clock base, deliberately far above any real epoch-
/// millisecond value.
///
/// Every buffered entry's `store_time` is the driver's virtual clock, which sits
/// around `2^50` ms. The store's own background flush loop computes its drain
/// deadline from the REAL wall clock (`now_millis() - write_delay_ms`), a value
/// around `1.7e12` — always far below these virtual store times — so the
/// background loop never finds a harness entry eligible and can never race the
/// driver's explicit, deterministic drain. All flushing is driven by `FlushTick`
/// against the real `drain_ready` at a virtual deadline instead.
const CLOCK_BASE: i64 = 1 << 50;

// ---------------------------------------------------------------------------
// Retaining inner-store double (never NullDataStore)
// ---------------------------------------------------------------------------

/// Inner store that RETAINS what it is told and can be switched to reject every
/// write. Retention is load-bearing: a discarding store would make every
/// post-recovery read vacuously absent, so a lost frame and a working replay
/// would be indistinguishable. The reject switch manufactures the failed-flush /
/// abandoned terminals and the failed-replay state C12 needs.
struct HarnessInnerStore {
    data: AsyncMutex<HashMap<(String, String), Option<RecordValue>>>,
    /// When `false`, every `add`/`remove` is rejected. Reads always succeed.
    healthy: AtomicBool,
}

impl HarnessInnerStore {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            data: AsyncMutex::new(HashMap::new()),
            healthy: AtomicBool::new(true),
        })
    }

    fn set_healthy(&self, healthy: bool) {
        self.healthy.store(healthy, Ordering::Relaxed);
    }

    fn is_healthy(&self) -> bool {
        self.healthy.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl MapDataStore for HarnessInnerStore {
    async fn add(
        &self,
        map: &str,
        key: &str,
        value: &RecordValue,
        _exp: i64,
        _now: i64,
    ) -> anyhow::Result<()> {
        if !self.is_healthy() {
            anyhow::bail!("injected inner-store rejection (unhealthy) for key={key}");
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
        if !self.is_healthy() {
            anyhow::bail!("injected inner-store rejection (unhealthy) for key={key}");
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
// Run configuration and outcome
// ---------------------------------------------------------------------------

/// One run's fault schedule and oracle selection.
#[derive(Debug, Clone)]
pub(crate) struct RunConfig {
    /// At most one re-introduced defect per run (R6).
    pub defect: DefectMode,
    /// Which oracles are active. Value equality defaults OFF (TG-WAL-006).
    pub oracle: OracleConfig,
    /// The GC crash point to install (R7). Used by AC7(a) with the production
    /// order and by AC7(b) with the inverted order; `DefectMode::UnlinkThenFsync`
    /// forces `PreUnlink` regardless, so the loss window is exercised.
    pub gc_crash_point: GcCrashPoint,
    /// Case-shape constraints, including the below-floor control flags.
    pub shape: CaseShape,
}

impl RunConfig {
    /// The baseline: no defect, both structural oracles on, value equality off,
    /// no GC crash point, default shape.
    pub fn baseline() -> Self {
        Self {
            defect: DefectMode::None,
            oracle: OracleConfig::default(),
            gc_crash_point: GcCrashPoint::None,
            shape: CaseShape::default(),
        }
    }

    /// The effective GC crash point: `UnlinkThenFsync` forces `PreUnlink` (R6).
    fn effective_gc_crash_point(&self) -> GcCrashPoint {
        if self.defect == DefectMode::UnlinkThenFsync {
            GcCrashPoint::PreUnlink
        } else {
            self.gc_crash_point
        }
    }
}

/// The result of driving one case: every violation the oracles reported plus the
/// coverage counters that prove the oracles actually ran (AC14).
#[derive(Debug, Clone, Default)]
pub(crate) struct RunOutcome {
    pub violations: Vec<InvariantViolation>,
    pub coverage: OracleCoverage,
}

impl RunOutcome {
    /// The AC14 oracle-coverage guard, value-returning (never a bare assert) so
    /// the baseline run can assert all floors pass while AC5(c)'s single-
    /// incarnation control and AC14(e)'s deliberately-below-floor run inspect a
    /// failing floor WITHOUT aborting the suite.
    ///
    /// The floor NUMBERS here are the structural minima; the tuned per-2000-cases
    /// rates are pinned by the strategy layer, which owns the case budget.
    pub fn check_oracle_coverage(&self, shape: &CaseShape) -> OracleCoverage {
        let cov = self.coverage;
        let o1_floor_met =
            shape.force_single_incarnation || cov.o1_healthy_recovery_evaluations >= 1;
        let o2_floor_met = cov.o2_evaluations >= 1;
        // Skipping is honest but bounded: indeterminate skips must not dominate,
        // or O2 would be "passing" by evaluating nothing. The structural floor is
        // held at the SAME 25% bound the strategy layer's tight AC14(c) check
        // asserts, so the reusable guard can never report pass on a skip ratio the
        // AC-level assertion would reject (a 26-49% run must fail BOTH, not just one).
        let indeterminate_ratio_floor_met = cov.o2_evaluations == 0
            || cov.o2_indeterminate_skips.saturating_mul(4) <= cov.o2_evaluations;
        OracleCoverage {
            floors: OracleCoverageFloors {
                o1_floor_met,
                o2_floor_met,
                indeterminate_ratio_floor_met,
            },
            ..cov
        }
    }
}

// ---------------------------------------------------------------------------
// Reference model — the driver's own event log
// ---------------------------------------------------------------------------

/// One key's currently-buffered write, mirroring the store's coalesced queue
/// entry. `store_time` is preserved across coalesce exactly as `DelayedEntry`
/// preserves it, so the model's due-time policy matches the implementation's.
#[derive(Debug, Clone)]
struct BufferEntry {
    partition: PartitionId,
    store_time: i64,
    wal_seqs: BTreeSet<Sequence>,
}

/// The reference model: acked writes and per-partition sequence lifecycle,
/// derived purely from the driver's log and fault schedule.
#[derive(Debug, Default)]
struct LogModel {
    /// Latest acked value per key (O1 holds only the latest per key).
    acked: HashMap<Key, ModelValue>,
    /// `Acked ∧ ¬DurablyApplied ∧ ¬Indeterminate`, per partition (the O2
    /// "unresolved" set).
    unresolved: HashMap<PartitionId, BTreeSet<Sequence>>,
    /// Sequences the model refuses to attribute (unhealthy-flush abandonment,
    /// `remove_all` partial failure). O2 skips these and counts the skip.
    indeterminate: HashMap<PartitionId, BTreeSet<Sequence>>,
    /// Currently-buffered write per key.
    buffer: HashMap<Key, BufferEntry>,
    /// `(incarnation, step) -> (partition, sequence)` bindings the trait facade
    /// uses to attribute a `record_ack` to the sequence bound at that step.
    bindings: HashMap<(usize, usize), (PartitionId, Sequence)>,
    /// The store time the trait facade stamps a `record_ack` with (the driver
    /// sets it to the virtual clock before issuing the op).
    cur_store_time: i64,
}

impl LogModel {
    fn unresolved_set(&mut self, p: PartitionId) -> &mut BTreeSet<Sequence> {
        self.unresolved.entry(p).or_default()
    }

    fn indeterminate_set(&mut self, p: PartitionId) -> &mut BTreeSet<Sequence> {
        self.indeterminate.entry(p).or_default()
    }

    /// Records an acked mutation: updates the latest value for the key and the
    /// per-partition unresolved set, replicating the store's subsuming coalesce.
    ///
    /// Every harness op frames complete state (a full LWW record or a whole-key
    /// tombstone), so a re-write of the same key SUBSUMES: the store early-
    /// resolves the retired frame's sequence out of its pending map. The model
    /// does the same — it drops the prior buffered sequences from `unresolved` —
    /// so the two pending views stay in lockstep, while preserving the original
    /// `store_time` on the surviving entry.
    fn ack(&mut self, p: PartitionId, seq: Sequence, key: Key, value: ModelValue, store_time: i64) {
        self.acked.insert(key, value);

        let effective_store_time = if let Some(prev) = self.buffer.remove(&key) {
            // Subsuming coalesce: the predecessor's sequences resolve early.
            let unresolved = self.unresolved_set(p);
            for retired in prev.wal_seqs {
                unresolved.remove(&retired);
            }
            prev.store_time
        } else {
            store_time
        };

        self.unresolved_set(p).insert(seq);
        let mut wal_seqs = BTreeSet::new();
        wal_seqs.insert(seq);
        self.buffer.insert(
            key,
            BufferEntry {
                partition: p,
                store_time: effective_store_time,
                wal_seqs,
            },
        );
    }

    /// The keys whose buffered entry is due at `deadline` (`store_time` ≤ deadline),
    /// matching `PartitionQueue::drain_ready`'s eligibility test.
    fn due_keys(&self, deadline: i64) -> Vec<Key> {
        let mut keys: Vec<Key> = self
            .buffer
            .iter()
            .filter(|(_, entry)| entry.store_time <= deadline)
            .map(|(k, _)| *k)
            .collect();
        keys.sort_unstable();
        keys
    }

    /// A due key's buffered sequences transition to `DurablyApplied` (a healthy
    /// flush drained them and advanced the watermark) — removed from
    /// `unresolved`, buffer entry cleared.
    fn flush_key_durable(&mut self, key: Key) {
        if let Some(entry) = self.buffer.remove(&key) {
            let unresolved = self.unresolved_set(entry.partition);
            for s in entry.wal_seqs {
                unresolved.remove(&s);
            }
        }
    }

    /// A due key flushed against an UNHEALTHY store: the store abandons the frame
    /// (it stays in the implementation's pending map, but the entry has left the
    /// queue and will never re-flush this incarnation), so attribution is
    /// ambiguous — the model marks the sequences `Indeterminate`.
    fn flush_key_indeterminate(&mut self, key: Key) {
        if let Some(entry) = self.buffer.remove(&key) {
            let p = entry.partition;
            for s in entry.wal_seqs {
                self.unresolved_set(p).remove(&s);
                self.indeterminate_set(p).insert(s);
            }
        }
    }

    /// A HEALTHY recovery replays and marks-applied every previously-unresolved
    /// frame (up to the contiguous-success frontier, which under a healthy store
    /// is every frame), so those sequences become `DurablyApplied`. A subsequent
    /// data loss is caught by O1 against `acked`, independently of this
    /// transition; this only keeps the per-partition unresolved view consistent
    /// with the implementation's empty post-recovery pending map.
    fn recover_healthy(&mut self) {
        for set in self.unresolved.values_mut() {
            set.clear();
        }
        self.buffer.clear();
    }

    fn unresolved_seqs(&self, p: PartitionId) -> Vec<Sequence> {
        self.unresolved
            .get(&p)
            .map(|s| s.iter().copied().collect())
            .unwrap_or_default()
    }

    fn has_indeterminate(&self, p: PartitionId) -> bool {
        self.indeterminate.get(&p).is_some_and(|s| !s.is_empty())
    }
}

impl ReferenceModel for LogModel {
    fn bind_sequence(
        &mut self,
        partition: PartitionId,
        incarnation: usize,
        step: usize,
        sequence: Sequence,
    ) {
        self.bindings
            .insert((incarnation, step), (partition, sequence));
    }

    fn record_ack(&mut self, incarnation: usize, step: usize, key: Key, value: ModelValue) {
        let Some((p, seq)) = self.bindings.get(&(incarnation, step)).copied() else {
            return;
        };
        let store_time = self.cur_store_time;
        self.ack(p, seq, key, value, store_time);
    }

    fn record_durably_applied(&mut self, partition: PartitionId, sequence: Sequence) {
        self.unresolved_set(partition).remove(&sequence);
        // Drop the sequence from any buffered entry that still carries it, and
        // remove the entry once it holds no further sequences.
        let drained: Vec<Key> = self
            .buffer
            .iter_mut()
            .filter_map(|(k, entry)| {
                entry.wal_seqs.remove(&sequence);
                entry.wal_seqs.is_empty().then_some(*k)
            })
            .collect();
        for k in drained {
            self.buffer.remove(&k);
        }
    }

    fn record_indeterminate(&mut self, partition: PartitionId, sequence: Sequence) {
        self.unresolved_set(partition).remove(&sequence);
        self.indeterminate_set(partition).insert(sequence);
    }

    fn on_crash(&mut self) {
        // A crash transitions nothing: sequences that are Acked but not
        // DurablyApplied stay unresolved across the boundary — exactly the state
        // the harness exists to police.
    }

    fn unresolved(&self, partition: PartitionId) -> Vec<Sequence> {
        self.unresolved_seqs(partition)
    }

    fn acked_value(&self, key: Key) -> Option<&ModelValue> {
        self.acked.get(&key)
    }

    fn is_non_empty(&self) -> bool {
        !self.acked.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Key-space helper (single partition, R2)
// ---------------------------------------------------------------------------

/// `count` distinct keys that all hash to `partition`, so the whole per-partition
/// invariant is constructible (scattering keys across the 271 partitions leaves
/// one pending sequence each and the stranded-lower-sequence state unreachable).
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

/// A partition with room for the whole `0..=MAX_KEY_INDEX` key space.
fn harness_partition() -> u32 {
    partition_for(TEST_MAP, "key-0")
}

/// The write-behind config every incarnation's store is built with — the ONE
/// instance the model reads `flush_interval_ms` from (R5.0 single-source rule).
fn harness_config() -> WriteBehindConfig {
    WriteBehindConfig {
        // Large real-clock write delay; combined with the far-future virtual
        // store times it guarantees the background flush loop never drains, so the
        // only flushing is the driver's deterministic `FlushTick` drain.
        write_delay_ms: 3_600_000,
        // The single source for the model's due delay AND the driver's drain
        // deadline. Kept small so a FlushTick can make a write due.
        flush_interval_ms: 1_000,
        batch_size: 100,
        // Fast, bounded graceful drain for CleanShutdown.
        max_retries: 1,
        backoff_base_ms: 1,
        backoff_cap_ms: 1,
        capacity: 0,
        shutdown_timeout_ms: 1_000,
        ..WriteBehindConfig::default()
    }
}

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

/// Drives one generated [`Case`] end-to-end and returns the oracle outcome.
///
/// Constructs a real `WalWriter` (durable across incarnations) and a retained
/// inner store, then for each incarnation builds a fresh `WriteBehindDataStore`
/// on real boot-seeding, applies the ops, and ends the incarnation with a real
/// crash or graceful drain. O1 runs after each healthy recovery; O2 runs after
/// every op.
pub(crate) async fn run_case(case: &Case, config: &RunConfig) -> RunOutcome {
    let wal_dir = tempfile::tempdir().expect("wal tempdir");
    // No fsync: the in-process crash model preserves un-fsynced bytes (the OS
    // page cache holds them across a drop-and-reopen), so PerOp would only slow
    // the run without changing what a crash can lose here.
    let wal =
        WalWriter::new(wal_dir.path().to_path_buf(), WalFsyncPolicy::None).expect("wal writer");
    let inner = HarnessInnerStore::new();

    let partition = harness_partition();
    let key_strings = keys_in_partition(partition, usize::from(MAX_KEY_INDEX) + 1);

    // The single-incarnation control (AC5c) must be honoured by the generator; the
    // driver asserts the shape it was handed matches, so a mis-generated control
    // cannot silently pass as a multi-incarnation run.
    debug_assert!(
        !config.shape.force_single_incarnation || case.len() <= 1,
        "force_single_incarnation run must not contain a crash/recover cycle"
    );

    let gc_crash_point = config.effective_gc_crash_point();

    let mut driver = Driver {
        wal: Arc::clone(&wal),
        inner: Arc::clone(&inner),
        partition,
        key_strings,
        config: config.clone(),
        wb_config: harness_config(),
        clock: CLOCK_BASE,
        store_healthy: true,
        model: LogModel::default(),
        outcome: RunOutcome::default(),
        gc_crash_point,
    };

    let incarnation_count = case.len();
    for (i, incarnation) in case.iter().enumerate() {
        let is_first = i == 0;
        let store = driver.boot_store(is_first, i).await;

        let mut crashed_by_gc = false;
        for (step, op) in incarnation.ops.iter().enumerate() {
            let ended = driver.apply_op(&store, op, i, step).await;
            if ended {
                crashed_by_gc = true;
                break;
            }
        }

        // A GcTick under an active PreUnlink crash point ends the incarnation as a
        // crash regardless of the generated end. Otherwise honour the end.
        if crashed_by_gc {
            driver.crash(store);
        } else {
            match incarnation.end {
                IncarnationEnd::Crash => driver.crash(store),
                IncarnationEnd::CleanShutdown => driver.clean_shutdown(store).await,
            }
        }

        // Recovery is what the NEXT incarnation runs against; O1 evaluates there.
        let has_next = i + 1 < incarnation_count;
        if has_next {
            driver.recover(i + 1).await;
        }
    }

    driver.outcome
}

struct Driver {
    wal: Arc<WalWriter>,
    inner: Arc<HarnessInnerStore>,
    partition: PartitionId,
    key_strings: Vec<String>,
    config: RunConfig,
    /// The ONE config instance used to build every incarnation's store; the model
    /// reads `flush_interval_ms` from here, never from a copied literal.
    wb_config: WriteBehindConfig,
    /// Virtual clock; `store_time` of every write and the base for flush
    /// deadlines. Decoupled from wall clock so cases are deterministic.
    clock: i64,
    /// The driver's own view of inner-store health (its injected fault schedule),
    /// used to decide flush attribution and whether O1 may run. Never read from
    /// the implementation.
    store_healthy: bool,
    model: LogModel,
    outcome: RunOutcome,
    gc_crash_point: GcCrashPoint,
}

impl Driver {
    fn key_str(&self, key: Key) -> &str {
        &self.key_strings[usize::from(key) % self.key_strings.len()]
    }

    /// Builds a fresh `WriteBehindDataStore` for one incarnation on real boot-
    /// seeding, applies the run's defect seams, installs the append observer, and
    /// (for a non-first incarnation) asserts the crash-vacuity guard (AC2).
    async fn boot_store(
        &mut self,
        is_first: bool,
        incarnation: usize,
    ) -> Arc<WriteBehindDataStore> {
        let sequence_start = if is_first {
            1
        } else {
            self.wal
                .max_observed_sequence()
                .await
                .expect("read max observed WAL sequence")
                .saturating_add(1)
        };

        let store = WriteBehindDataStore::new_with_wal(
            Arc::clone(&self.inner) as Arc<dyn MapDataStore>,
            self.wb_config.clone(),
            Some(WalBootstrap {
                wal: Arc::clone(&self.wal) as Arc<dyn Wal>,
                sequence_start,
            }),
        );

        // AC2 crash-vacuity guard: before ANY op of a non-first incarnation the
        // rebuilt store must report the partition unseeded and its pending map
        // empty. This is a CHECK on the implementation, not a model input.
        if !is_first {
            assert!(
                !store.test_wal_partition_seeded(self.partition),
                "AC2: rebuilt store must boot with the partition unseeded \
                 (incarnation {incarnation})"
            );
            assert!(
                store.test_pending_wal_sequences(self.partition).is_empty(),
                "AC2: rebuilt store must boot with an empty pending map \
                 (incarnation {incarnation})"
            );
        }

        // Apply the run's defect seams (R6).
        match self.config.defect {
            // `ReplayClobberOlderFrame` is a recovery-side seam (see `recover`),
            // so it installs nothing on the store.
            DefectMode::None
            | DefectMode::UnlinkThenFsync
            | DefectMode::ReplayClobberOlderFrame => {}
            DefectMode::ScalarMaxWatermark => {
                store.test_set_watermark_mode(WatermarkMode::ScalarMax);
            }
            DefectMode::EmptyBootSeed => store.test_set_boot_seed_mode(BootSeedMode::Empty),
            DefectMode::InclusiveOffByOne => {
                store.test_set_watermark_mode(WatermarkMode::InclusiveOffByOne);
            }
        }

        // Install the append observer: a synchronous callback that records the
        // (partition, sequence) the store just assigned. The driver reads it back
        // immediately after each mutating call to bind the sequence NAME.
        let sink = APPEND_SINK.with(Arc::clone);
        sink.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        let sink_for_hook = Arc::clone(&sink);
        store.test_set_append_observer(Arc::new(move |p, s| {
            sink_for_hook
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push((p, s));
        }));

        store
    }

    /// The flush delay policy constant, read from the SAME config instance the
    /// store was built with — never a copied literal (R5.0 single-source rule).
    fn flush_interval_ms(&self) -> i64 {
        i64::try_from(self.wb_config.flush_interval_ms).unwrap_or(i64::MAX)
    }

    /// Drains and binds the sequences the store assigned during the just-completed
    /// mutating call. Reads the process-wide append sink, so it takes no `self`.
    fn drain_observed() -> Vec<(PartitionId, Sequence)> {
        let sink = APPEND_SINK.with(Arc::clone);
        let mut guard = sink
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        std::mem::take(&mut *guard)
    }

    /// Applies one op. Returns `true` if the op ended the incarnation (a `GcTick`
    /// under an active `PreUnlink` crash point).
    async fn apply_op(
        &mut self,
        store: &Arc<WriteBehindDataStore>,
        op: &WorkOp,
        incarnation: usize,
        step: usize,
    ) -> bool {
        let mut ended = false;
        match op {
            WorkOp::Append { key, millis } => {
                let value = lww(*millis);
                let now = self.clock;
                let res = store
                    .add(TEST_MAP, self.key_str(*key), &value, 0, now)
                    .await;
                let observed = Self::drain_observed();
                if res.is_ok() {
                    // A successful `add` mints exactly one WAL frame, so exactly one
                    // sequence must have been observed. A mismatch means the append
                    // path started coalescing/deduplicating before assigning a
                    // sequence, which would silently misalign the model↔sequence
                    // binding and blind the oracle — fail loud rather than drift.
                    assert_eq!(
                        observed.len(),
                        1,
                        "add must append exactly one WAL frame per call"
                    );
                    // Bind the sequence NAME, then let the MODEL decide the ack
                    // transition (R5.0): identifier from the impl, lifecycle from
                    // the model's own log.
                    self.model.cur_store_time = now;
                    for (p, s) in observed {
                        self.model.bind_sequence(p, incarnation, step, s);
                        self.model.record_ack(
                            incarnation,
                            step,
                            *key,
                            ModelValue::Live { millis: *millis },
                        );
                    }
                }
            }
            WorkOp::Remove { key } => {
                let now = self.clock;
                let res = store.remove(TEST_MAP, self.key_str(*key), now).await;
                let observed = Self::drain_observed();
                if res.is_ok() {
                    assert_eq!(
                        observed.len(),
                        1,
                        "remove must append exactly one WAL frame per call"
                    );
                    self.model.cur_store_time = now;
                    for (p, s) in observed {
                        self.model.bind_sequence(p, incarnation, step, s);
                        self.model
                            .record_ack(incarnation, step, *key, ModelValue::Tombstone);
                    }
                }
            }
            WorkOp::RemoveAll { keys } => {
                // Unlike `add`/`remove`, the store's `remove_all` takes no injected
                // clock and stamps each buffered entry's `store_time` with the real
                // `now_millis()` wall clock — a value that always sits FAR BELOW the
                // driver's virtual clock (`CLOCK_BASE`), so a store-side remove_all
                // entry is due against every virtual flush deadline. The model must
                // stamp the same wall-clock semantics, or it would believe the entry
                // is not-yet-due while the store has already drained it — a false O2
                // RED. `0` is below every virtual deadline, so it reproduces the
                // store's always-due behaviour; coalescing then preserves the
                // survivor's `store_time` on both sides exactly as `DelayedEntry` does.
                let store_time = 0;
                let key_strs: Vec<String> =
                    keys.iter().map(|k| self.key_str(*k).to_string()).collect();
                let res = store.remove_all(TEST_MAP, &key_strs).await;
                let observed = Self::drain_observed();
                if res.is_ok() {
                    // `remove_all` appends exactly one frame per input key, in order
                    // and WITHOUT deduplicating (verified in write_behind.rs), so the
                    // Nth observed sequence belongs to the Nth key and the zip is
                    // fully aligned. Assert the 1:1 length so a future append-path
                    // change that dedups/skips keys cannot silently misattribute a
                    // sequence and blind the oracle — the zip would otherwise stop at
                    // the shorter iterator with no error.
                    assert_eq!(
                        observed.len(),
                        keys.len(),
                        "remove_all must append exactly one WAL frame per input key"
                    );
                    for ((p, s), key) in observed.iter().zip(keys.iter()) {
                        self.model
                            .ack(*p, *s, *key, ModelValue::Tombstone, store_time);
                    }
                } else {
                    // Partial failure: attribution is ambiguous — mark every
                    // observed sequence Indeterminate rather than guess. This is
                    // deliberately conservative: keys appended BEFORE the failing one
                    // are frame-backed and stay pending in the impl, so marking them
                    // Indeterminate over-skips them in O2 (a narrowed coverage gap,
                    // never a false GREEN). Not exercised by the current generator —
                    // `remove_all` fails only on capacity pressure (unbounded here) or
                    // a WAL-append error (WAL is a TempDir here). Precise per-key
                    // attribution when the path becomes reachable is TODO-606.
                    for (p, s) in observed {
                        self.model.record_indeterminate(p, s);
                    }
                }
            }
            WorkOp::FlushTick { advance_ms } => {
                self.clock = self
                    .clock
                    .saturating_add(i64::try_from(*advance_ms).unwrap_or(i64::MAX));
                self.flush_due(store).await;
            }
            WorkOp::GcTick => {
                ended = self.gc_tick(store).await;
            }
            WorkOp::SetStoreHealth { healthy } => {
                self.store_healthy = *healthy;
                self.inner.set_healthy(*healthy);
            }
            WorkOp::Read { key } => {
                let _ = store.load(TEST_MAP, self.key_str(*key)).await;
            }
        }

        if !ended {
            self.evaluate_o2(store).await;
        }
        ended
    }

    /// Drives the REAL due-time drain + coalescing at the virtual deadline, then
    /// applies each drained entry through the store's own resolution path — the
    /// path that carries the C3 / C13 watermark seam. Not a synthetic per-key
    /// flush.
    async fn flush_due(&mut self, store: &Arc<WriteBehindDataStore>) {
        let deadline = self.clock.saturating_sub(self.flush_interval_ms());

        // Model side: the same due-key set, computed from the driver's own buffer.
        let due_keys = self.model.due_keys(deadline);

        // Implementation side: the real coalescing drain across partition queues.
        let mut drained = Vec::new();
        for mut queue in store.queues.iter_mut() {
            drained.extend(queue.value_mut().drain_ready(deadline));
        }

        for entry in drained {
            let p = partition_for(&entry.map, &entry.key);
            let result = match &entry.operation {
                DelayedOp::Store {
                    value,
                    expiration_time,
                } => {
                    store
                        .inner
                        .add(
                            &entry.map,
                            &entry.key,
                            value,
                            *expiration_time,
                            entry.store_time,
                        )
                        .await
                }
                DelayedOp::Remove => {
                    store
                        .inner
                        .remove(&entry.map, &entry.key, entry.store_time)
                        .await
                }
            };
            store.resolve_pending(entry.sequence);
            if result.is_ok() {
                // Real resolve + advance: this is where the WatermarkMode seam
                // (C3 / C13) computes the watermark and marks applied.
                store.resolve_and_advance(p, &entry.wal_sequences).await;
            } else {
                store.abandon_wal_sequences(p, &entry.wal_sequences);
            }
        }

        // Model side: transition each due key by the health the flush ran under.
        for key in due_keys {
            if self.store_healthy {
                self.model.flush_key_durable(key);
            } else {
                self.model.flush_key_indeterminate(key);
            }
        }
    }

    /// Real `mark_applied` at the current prefix-complete watermark, honouring the
    /// installed GC order and crash point (R7). Returns `true` when a `PreUnlink`
    /// crash point is active, ending the incarnation at the seam (sidecar durable
    /// under the production order; segments already unlinked under the inverted
    /// one).
    async fn gc_tick(&mut self, store: &Arc<WriteBehindDataStore>) -> bool {
        self.wal.test_set_gc_order_mode(match self.config.defect {
            DefectMode::UnlinkThenFsync => WalGcOrderMode::UnlinkThenFsync,
            _ => WalGcOrderMode::FsyncThenUnlink,
        });
        self.wal.test_set_gc_crash_point(match self.gc_crash_point {
            GcCrashPoint::PreUnlink => WalGcCrashPoint::PreUnlink,
            GcCrashPoint::None => WalGcCrashPoint::None,
        });

        // Read the watermark to DRIVE the implementation's own GC (not a model
        // input). Unseeded / no-WAL → nothing to collect.
        let Some(Ok(watermark)) = store.test_wal_watermark(self.partition) else {
            return false;
        };
        let _ = self.wal.mark_applied(self.partition, watermark).await;

        matches!(self.gc_crash_point, GcCrashPoint::PreUnlink)
    }

    /// O2 (frame oracle) — model-derived LEFT, implementation-observed RIGHT.
    async fn evaluate_o2(&mut self, store: &Arc<WriteBehindDataStore>) {
        if !self.model.is_non_empty() {
            return;
        }
        let p = self.partition;
        let unresolved = ReferenceModel::unresolved(&self.model, p);
        self.outcome.coverage.o2_evaluations += 1;
        if self.model.has_indeterminate(p) {
            self.outcome.coverage.o2_indeterminate_skips += 1;
        }
        if unresolved.is_empty() {
            return;
        }
        let min_unresolved = unresolved[0];

        // The impl-observed watermark, read once. `Some(Ok(w))` = seeded value
        // under test; `Some(Err(Unseeded))` / `None` = nothing claimed applied.
        let watermark = match store.test_wal_watermark(p) {
            Some(Ok(w)) => Some(w),
            _ => None,
        };

        // Clause 1: W(p) must sit strictly below the smallest model-unresolved
        // sequence.
        if let Some(w) = watermark {
            if w >= min_unresolved {
                self.outcome
                    .violations
                    .push(InvariantViolation::WatermarkAboveUnresolved {
                        partition: p,
                        sequence: min_unresolved,
                    });
            }
        }

        // Clause 3 (+ clause 2): wal.unapplied(p) must still contain every
        // model-unresolved sequence — a frame that should replay must not have
        // been filtered out. When a frame IS missing, the PERSISTED applied
        // watermark (the `.applied` sidecar, the value that actually governs what
        // `unapplied` returns) discriminates the cause: a persisted watermark that
        // advanced to or past it FILTERED it (the C13 inclusive-off-by-one shape);
        // a frame gone while the persisted watermark stayed below it means its
        // sealed segment was unlinked before the sidecar was durable (the
        // TG-WAL-003 hazard). The recomputed prefix-complete `test_wal_watermark`
        // used by clause 1 always sits one below the smallest unresolved sequence,
        // so it cannot make this distinction — only the persisted sidecar can.
        let persisted_watermark = self.wal.test_applied_watermark(p);
        let unapplied: HashSet<Sequence> = match self.wal.unapplied(p).await {
            Ok(frames) => frames.into_iter().map(|f| f.sequence).collect(),
            Err(_) => HashSet::new(),
        };
        for s in &unresolved {
            if unapplied.contains(s) {
                continue;
            }
            let violation = if persisted_watermark >= *s {
                InvariantViolation::UnappliedFrameFiltered {
                    partition: p,
                    sequence: *s,
                }
            } else {
                InvariantViolation::SegmentUnlinkedBeforeWatermarkFsync {
                    partition: p,
                    sequence: *s,
                }
            };
            self.outcome.violations.push(violation);
        }
    }

    /// O1 (data oracle) — evaluated only after a healthy recovery. Every acked
    /// live value must be observable; every acked tombstone must not be live.
    async fn evaluate_o1(&mut self, incarnation: usize) {
        if !self.model.is_non_empty() {
            return;
        }
        self.outcome.coverage.o1_healthy_recovery_evaluations += 1;
        // Snapshot the acked keys so the read borrow does not conflict with the
        // mutable violation push; the value comes back through the trait accessor.
        let keys: Vec<Key> = self.model.acked.keys().copied().collect();
        for key in keys {
            let Some(value) = self.model.acked_value(key).cloned() else {
                continue;
            };
            let key_str = self.key_strings[usize::from(key) % self.key_strings.len()].clone();
            let loaded = self.inner.load(TEST_MAP, &key_str).await.ok().flatten();
            let present = loaded.is_some();
            let violated = match value {
                ModelValue::Live { .. } => !present,
                ModelValue::Tombstone => present,
            };
            if violated {
                self.outcome
                    .violations
                    .push(InvariantViolation::AckedWriteLost { key, incarnation });
            }

            // value_equality oracle (opt-in, R5): a present acked-live value must
            // NOT carry a timestamp OLDER than the model's latest acked HLC millis.
            // A stale re-replay clobber resurrects an older durable value — the exact
            // regression TG-WAL-006's fix prevents. Only a strictly-older recovered
            // timestamp is flagged: a recovered value NEWER than the model's
            // last-arrival is the LWW-by-timestamp winner (never data loss), so it
            // must not false-positive when the generator emits out-of-timestamp-order
            // writes for a key. Off by default; enabled explicitly for the
            // closing-evidence and regression runs.
            //
            // Soundness domain (two constraints, both currently satisfied):
            //  1. Value coupling (do not break in `lww()`): comparing millis alone is
            //     sound only because `lww(millis)` makes value a pure function of millis
            //     and fixes `counter:0`/`node_id:""`, so full-`Timestamp` order collapses
            //     to millis order and equal millis implies an identical value. A generator
            //     that emitted distinct values at equal millis, or varied counter/node_id,
            //     would let an equal-millis different-value clobber slip this check.
            //  2. Reference point: `expected` is the model's LATEST-ARRIVAL millis, which
            //     equals the true LWW winner (max timestamp) only when a key's writes are
            //     MONOTONE in arrival order — as production HLC always is, and as the sole
            //     `value_equality: true` case (`ac4_5_...`) is by construction. If a defect
            //     run ever used a non-monotone key (arrival 20, 30, 10) a clobber landing in
            //     `[latest_arrival, max_acked)` (e.g. recovered 20 vs latest-arrival 10)
            //     would slip the strictly-older check. Making the oracle sound for
            //     non-monotone defect runs needs a per-key max-live-millis reference.
            if self.config.oracle.value_equality {
                if let (
                    ModelValue::Live { millis: expected },
                    Some(RecordValue::Lww { timestamp, .. }),
                ) = (&value, &loaded)
                {
                    let recovered = i64::try_from(timestamp.millis).unwrap_or(i64::MAX);
                    if recovered < *expected {
                        self.outcome
                            .violations
                            .push(InvariantViolation::AckedValueMismatch {
                                key,
                                incarnation,
                                expected_millis: *expected,
                                recovered_millis: recovered,
                            });
                    }
                }
            }
        }
    }

    /// Ends an incarnation with a real crash: signal the store's background tasks
    /// to stop WITHOUT a graceful drain, then drop the `Arc` so every in-memory
    /// structure is destroyed. The WAL directory and the retained inner store —
    /// the durable artefacts — survive.
    fn crash(&mut self, store: Arc<WriteBehindDataStore>) {
        // Stop the background flush + watchdog loops so their Arc clones drop and
        // the store is actually destroyed; this is a process-stop signal, not a
        // drain (the flush loop returns immediately on the shutdown flag).
        let _ = store.shutdown.send(true);
        drop(store);
        self.model.on_crash();
    }

    /// Ends an incarnation with a real graceful drain.
    async fn clean_shutdown(&mut self, store: Arc<WriteBehindDataStore>) {
        let _ = store.hard_flush().await;
        // Mirror the drain in the model: every remaining buffered key resolves —
        // durably if the store is healthy, indeterminately if the drain abandoned
        // it under an unhealthy inner store.
        let remaining: Vec<Key> = self.model.buffer.keys().copied().collect();
        for key in remaining {
            if self.store_healthy {
                self.model.flush_key_durable(key);
            } else {
                self.model.flush_key_indeterminate(key);
            }
        }
        drop(store);
    }

    /// Runs the REAL recovery against the reopened (retained) inner store, then —
    /// if the store is healthy — evaluates O1 and advances the model's per-
    /// partition unresolved view to match the empty post-recovery pending map.
    /// `next_incarnation` is the index the loss surfaces at (for the O1 record).
    async fn recover(&mut self, next_incarnation: usize) {
        let mut recovery = WalRecovery::new(Arc::clone(&self.wal), vec![self.partition]);
        // `ReplayClobberOlderFrame` reproduces the pre-`TG-WAL-006` blind clobber:
        // disable the merge gate AND re-replay each partition's oldest un-applied
        // frame over the newer durable value the in-order pass left behind.
        if self.config.defect == DefectMode::ReplayClobberOlderFrame {
            recovery.test_set_replay_merge_gate(false);
            recovery.test_set_re_replay_oldest_frame(true);
        }
        let _ = recovery
            .run(Arc::clone(&self.inner) as Arc<dyn MapDataStore>)
            .await;

        if self.store_healthy {
            // O1 is evaluated ONLY after a recovery performed with a healthy inner
            // store.
            self.evaluate_o1(next_incarnation).await;
            self.model.recover_healthy();
        }
        // The `on_crash` transition already ran; nothing else to do for an
        // unhealthy recovery (the unresolved frames stay unresolved, exactly the
        // cross-boundary state the harness polices).
    }
}

thread_local! {
    /// Per-thread sink the append observer pushes into. Thread-local because the
    /// synchronous proptest bridge drives a whole case on one thread, so a single
    /// case's observations never interleave with another's.
    static APPEND_SINK: Arc<std::sync::Mutex<Vec<(PartitionId, Sequence)>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
}

/// Builds an LWW record carrying `millis` as both the value and the timestamp.
///
/// Load-bearing for the `value_equality` oracle's soundness: value MUST stay a pure
/// function of `millis`, and `counter`/`node_id` MUST stay constant. The oracle
/// compares `millis` alone; if value stopped tracking `millis`, or `counter`/`node_id`
/// varied, an equal-millis different-value clobber would slip its strictly-older
/// check (see the soundness-coupling note at the oracle in `evaluate_o1`).
fn lww(millis: i64) -> RecordValue {
    RecordValue::Lww {
        value: Value::Int(millis),
        timestamp: Timestamp {
            millis: u64::try_from(millis).unwrap_or(0),
            counter: 0,
            node_id: String::new(),
        },
    }
}

// ---------------------------------------------------------------------------
// Smoke tests (de-risk the strategy layer; NOT the full proptest suite)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod smoke {
    use super::*;

    fn incarnation(ops: Vec<WorkOp>, end: IncarnationEnd) -> super::super::Incarnation {
        super::super::Incarnation { ops, end }
    }

    /// A tiny hand-built case drives end-to-end: append + flush, crash, then
    /// recover + read. Proves the driver compiles and runs, that AC2's post-crash
    /// empty-pending assertion holds (the boot guard would panic otherwise), and
    /// that O1/O2 evaluate without panicking on a healthy baseline.
    #[tokio::test]
    async fn baseline_append_flush_crash_recover_is_green() {
        let case: Case = vec![
            incarnation(
                vec![
                    WorkOp::Append { key: 0, millis: 10 },
                    WorkOp::FlushTick { advance_ms: 5_000 },
                ],
                IncarnationEnd::Crash,
            ),
            incarnation(vec![WorkOp::Read { key: 0 }], IncarnationEnd::CleanShutdown),
        ];
        let outcome = run_case(&case, &RunConfig::baseline()).await;
        assert!(
            outcome.violations.is_empty(),
            "healthy baseline must report zero violations, got {:?}",
            outcome.violations
        );
        // The oracles must have actually run.
        assert!(
            outcome.coverage.o2_evaluations >= 1,
            "O2 must have evaluated at least once"
        );
    }

    /// An acked-but-unflushed write survives a crash: recovery replays it and O1
    /// finds it observable. Exercises the model↔impl binding across a boundary.
    #[tokio::test]
    async fn unflushed_write_survives_crash_and_recovery() {
        let case: Case = vec![
            incarnation(
                vec![WorkOp::Append { key: 1, millis: 42 }],
                IncarnationEnd::Crash,
            ),
            incarnation(vec![WorkOp::Read { key: 1 }], IncarnationEnd::Crash),
            incarnation(vec![], IncarnationEnd::CleanShutdown),
        ];
        let outcome = run_case(&case, &RunConfig::baseline()).await;
        assert!(
            outcome.violations.is_empty(),
            "an acked write must replay losslessly on the production path, got {:?}",
            outcome.violations
        );
        assert!(
            outcome.coverage.o1_healthy_recovery_evaluations >= 1,
            "O1 must have evaluated after a healthy recovery"
        );
    }

    /// The coverage guard is value-returning: a run that never crosses a crash
    /// reports a failing O1 floor without aborting.
    #[tokio::test]
    async fn coverage_guard_reports_floor_without_aborting() {
        let case: Case = vec![incarnation(
            vec![WorkOp::Append { key: 0, millis: 1 }],
            IncarnationEnd::CleanShutdown,
        )];
        let shape = CaseShape {
            force_single_incarnation: true,
            ..CaseShape::default()
        };
        let cfg = RunConfig {
            shape: shape.clone(),
            ..RunConfig::baseline()
        };
        let outcome = run_case(&case, &cfg).await;
        let coverage = outcome.check_oracle_coverage(&shape);
        // Single-incarnation control: no healthy recovery evaluations, and the
        // guard reports that as an exempt (met-by-design) O1 floor.
        assert_eq!(coverage.o1_healthy_recovery_evaluations, 0);
        assert!(coverage.floors.o1_floor_met);
    }

    /// The default oracle config keeps value equality OFF (AC11 / TG-WAL-006).
    #[test]
    fn value_equality_defaults_off() {
        assert!(!OracleConfig::default().value_equality);
        assert!(!RunConfig::baseline().oracle.value_equality);
    }

    /// The reference-model trait facade is internally consistent: a bound + acked
    /// sequence reads unresolved, a durable-apply resolves it, and an
    /// indeterminate mark removes it from the unresolved set. This exercises the
    /// full `ReferenceModel` surface a future OR-Map delta-fold case (R9,
    /// `TG-OR-003`) extends.
    #[test]
    fn reference_model_trait_is_consistent() {
        let mut model = LogModel::default();
        let p: PartitionId = 7;
        model.cur_store_time = 100;

        model.bind_sequence(p, 0, 0, 1);
        model.record_ack(0, 0, 0, ModelValue::Live { millis: 1 });
        assert_eq!(ReferenceModel::unresolved(&model, p), vec![1]);
        assert!(model.is_non_empty());
        assert!(matches!(
            model.acked_value(0),
            Some(ModelValue::Live { millis: 1 })
        ));

        model.record_durably_applied(p, 1);
        assert!(ReferenceModel::unresolved(&model, p).is_empty());

        model.bind_sequence(p, 0, 1, 2);
        model.record_ack(0, 1, 1, ModelValue::Tombstone);
        model.record_indeterminate(p, 2);
        assert!(ReferenceModel::unresolved(&model, p).is_empty());

        // A crash transitions nothing.
        model.bind_sequence(p, 0, 2, 3);
        model.record_ack(0, 2, 2, ModelValue::Live { millis: 3 });
        model.on_crash();
        assert_eq!(ReferenceModel::unresolved(&model, p), vec![3]);
    }
}
