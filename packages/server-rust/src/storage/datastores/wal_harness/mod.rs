//! Types and traits for a deterministic, incarnation-based crash/recovery harness for the
//! write-behind + WAL subsystem (`write_behind.rs` + `wal/mod.rs`).
//!
//! This module declares the harness's vocabulary only: the generated op alphabet, the
//! incarnation-shaped case structure, the defect-injection knobs, the reference-model shape, and
//! the typed violations the oracles report. The executor (`driver.rs`) and the proptest strategies
//! + property tests (`cases.rs`) are separate modules layered on top of this one.
//!
//! # Crash-model limit (normative)
//!
//! The harness's crash is a **modelled incarnation loss**: it destroys the in-memory
//! `WriteBehindDataStore` and inner-store handle between incarnations while preserving the durable
//! WAL directory, watermark sidecars, and inner store's on-disk file. It does **not** model OS
//! page-cache loss under `kill -9` — an un-fsynced-but-written byte that the OS still holds in page
//! cache survives an in-process drop-and-reopen, so this harness cannot exercise or bound that loss
//! window. Claiming otherwise would be an over-claimed crash model, which is worse than no harness
//! at all. That page-cache-loss / durable-frontier proof is a categorically different crash model
//! and is out of scope here.
//!
//! # Extension seams (R9)
//!
//! The design admits both of the following as **cases on this harness**, not forks:
//!
//! - **Value-equality oracle.** `OracleConfig::value_equality` defaults to `false` (opt-in per run);
//!   `TG-WAL-006` is enforced (LWW-scoped), so a run with the flag on is green on the fixed path and
//!   `DefectMode::ReplayClobberOlderFrame` is caught by it. No driver change is needed to exercise it.
//! - **OR-Map delta-fold recovery across a restart.** This lands as an `OrDelta`-shaped variant
//!   added to [`WorkOp`] plus an OR-shaped variant added to [`ModelValue`] — the OR-Map delta-fold
//!   recovery proof lands as a case on this harness, rather than a fork of it. The driver's
//!   incarnation/crash/recover machinery, the oracles' structure, and the shrinking shape require no
//!   modification for this addition. The invariant this seam eventually proves is catalogued as
//!   `TG-OR-003`; the spec that lands it is found by following that ID into `INVARIANTS.md`, not by
//!   a reference here.

/// The executor: applies a generated [`Case`] to a real `WriteBehindDataStore`
/// + `WalWriter`, crosses incarnations with real crashes/recoveries, and
/// evaluates both oracles. Layered on the vocabulary this module declares.
pub(crate) mod driver;

/// Proptest strategies, incarnation-preserving shrinking, the property tests
/// (AC1/AC2/AC3), the four regression meta-tests (AC4–AC7), and the oracle-
/// coverage guard (AC14). Layered on the driver and the vocabulary this module
/// declares.
#[cfg(test)]
mod cases;

/// Index into the small per-case key space (`0..=K`, `K` ≈ 4).
///
/// Keys drawn from this space are forced into a single partition (the existing
/// `keys_in_partition` technique) so every partition holds one pending sequence and the
/// stranded-lower-sequence state stays constructible — scattering across the 271 partitions would
/// make it unconstructible.
pub(crate) type Key = u8;

/// Inclusive upper bound of the key-index space (`K` in `0..=K`).
pub(crate) const MAX_KEY_INDEX: Key = 4;

/// Partition identifier, matching `write_behind.rs`'s and `wal/mod.rs`'s `u32` partition space.
pub(crate) type PartitionId = u32;

/// WAL sequence number, matching `write_behind.rs`'s and `wal/mod.rs`'s `u64` sequence space.
pub(crate) type Sequence = u64;

/// The op alphabet a generated case draws from (R2). Each variant drives exactly one real
/// store/WAL entry point — the harness MUST NOT invent a separate code path for any of these.
///
/// # Extension seam (R9, `TG-OR-003`)
/// Adding an `OrDelta`-shaped variant here (for the OR-Map delta-fold recovery proof — see
/// [`ModelValue`]'s doc-comment) requires no change to how the driver dispatches ops, crosses
/// incarnations, or shrinks a [`Case`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WorkOp {
    /// Drives the real `WriteBehindDataStore::add` (LWW record). `millis` is the epoch-millisecond
    /// write timestamp — matches `Timestamp.millis` and is never `f64`.
    Append { key: Key, millis: i64 },
    /// Drives the real `remove`.
    Remove { key: Key },
    /// Drives the real `remove_all` — the mid-loop partial-failure path.
    RemoveAll { keys: Vec<Key> },
    /// Drives the real due-time drain + coalescing (`drain_ready`), never a synthetic flush.
    /// `advance_ms` is a millisecond duration and is never `f64`.
    FlushTick { advance_ms: u64 },
    /// Drives the real `mark_applied` at the current `W(p)` — including seal + segment unlink. The
    /// harness MUST NOT invent a separate GC path.
    GcTick,
    /// Toggles the inner-store reject-all switch on/off — manufactures abandoned terminals and R5
    /// `Indeterminate` states.
    SetStoreHealth { healthy: bool },
    /// Reads through the store (staging buffer + inner).
    Read { key: Key },
}

/// How an incarnation ends (R1). `Recover` is deliberately not a generated op — it is what the
/// driver does when it starts the next incarnation after a `Crash`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IncarnationEnd {
    /// Destroys every in-memory structure while preserving durable artefacts (R3); the next
    /// incarnation reopens and runs real recovery.
    Crash,
    /// Runs the real graceful drain instead of a crash.
    CleanShutdown,
}

/// One incarnation: a sequence of ops followed by how it ends (R1).
#[derive(Debug, Clone)]
pub(crate) struct Incarnation {
    pub ops: Vec<WorkOp>,
    pub end: IncarnationEnd,
}

/// A generated case: `1..=CaseShape::max_incarnations` incarnations (R1). Deliberately not a flat
/// `Vec<WorkOp>` with inline crash markers — shrinking a `Vec<Incarnation>` can only drop whole
/// incarnations or shrink ops within one, so it can never produce a dangling crash or an orphaned
/// recovery.
pub(crate) type Case = Vec<Incarnation>;

/// Default cap on the number of incarnations a generated [`Case`] may contain (R8).
pub(crate) const MAX_INCARNATIONS: usize = 4;

/// Default cap on the number of ops within a single generated [`Incarnation`] (R8).
pub(crate) const MAX_OPS_PER_INCARNATION: usize = 8;

/// Shape constraints a case generator draws within.
///
/// The two `force_*` fields exist for the below-floor control runs the acceptance criteria require
/// on purpose: AC5(c)'s single-incarnation negative control, and AC14(e)'s deliberately-vacuous
/// discrimination run for the oracle-coverage guard. Neither is a "normal" run configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CaseShape {
    pub max_incarnations: usize,
    pub max_ops_per_incarnation: usize,
    /// AC5(c) negative control: pin every generated case to exactly one incarnation, so no crash
    /// (and therefore no recovery) can occur — the mechanical proof that restart cycles are
    /// load-bearing for detecting C12.
    pub force_single_incarnation: bool,
    /// AC14(e) discrimination run: force every generated op to `SetStoreHealth { healthy: false }`,
    /// driving O1 healthy-recovery evaluations to (near) zero on purpose, so the coverage guard can
    /// be proven to report the resulting floor failure.
    pub force_unhealthy: bool,
}

impl Default for CaseShape {
    fn default() -> Self {
        Self {
            max_incarnations: MAX_INCARNATIONS,
            max_ops_per_incarnation: MAX_OPS_PER_INCARNATION,
            force_single_incarnation: false,
            force_unhealthy: false,
        }
    }
}

/// Selects at most one re-introduced defect per run (R6). `None` is the baseline the harness
/// expects zero O1/O2 violations under.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum DefectMode {
    #[default]
    None,
    /// Re-introduces C3 via the existing `WatermarkMode::ScalarMax` seam.
    ScalarMaxWatermark,
    /// Re-introduces C12 via `write_behind.rs`'s new `BootSeedMode::Empty` seam.
    EmptyBootSeed,
    /// Re-introduces C13 via `write_behind.rs`'s new `WatermarkMode::InclusiveOffByOne` seam.
    InclusiveOffByOne,
    /// Re-introduces the `TG-WAL-003` hazard via `wal/mod.rs`'s new
    /// `GcOrderMode::UnlinkThenFsync` seam.
    UnlinkThenFsync,
    /// Re-introduces the pre-`TG-WAL-006` recovery clobber via `wal/mod.rs`'s
    /// `WalRecovery` test seams: the `RecordValue::Lww` merge gate is disabled
    /// (blind replay) AND each partition's oldest un-applied frame is re-replayed
    /// once more after the in-order pass — reproducing "an older frame is
    /// re-applied over a newer durable value". Caught by the `value_equality`
    /// oracle as an `AckedValueMismatch`.
    ReplayClobberOlderFrame,
}

/// `wal/mod.rs::mark_applied` crash-injection point (R7), fired between the sidecar write+fsync
/// and the unlink loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum GcCrashPoint {
    #[default]
    None,
    /// Crash between the sidecar write+fsync and the unlink loop: the sidecar is durable, the
    /// segments are not yet unlinked.
    PreUnlink,
}

/// Boot pending-map seeding seam (R6) in `write_behind.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum BootSeedMode {
    /// Production behaviour: the pending map is seeded from `wal.unapplied(p)` on boot.
    #[default]
    Seeded,
    /// C12 re-introduction: skip populating the pending map from `wal.unapplied(p)` on boot.
    Empty,
}

/// Which oracles are active for a run (R5).
///
/// `value_equality` gates an equality check between the model's latest acked value (its HLC
/// millis) and the recovered value's `RecordValue::Lww` timestamp, layered on top of O1/O2. When on,
/// a recovered value whose timestamp does not match the model's latest acked write is reported as an
/// `AckedValueMismatch` — the shape a stale re-replay clobber produces.
///
/// It defaults to `false` deliberately: the equality oracle is opt-in per run, and
/// `ac11_value_equality_defaults_off` guards that `OracleConfig::default()` never silently enables
/// it. `TG-WAL-006` is now enforced (LWW-scoped): `WalRecovery::replay_entry` discards a modern Lww
/// frame older than the current durable value, so the baseline suite is GREEN with this flag turned
/// on — that green run is the closing evidence the invariant refers to. `DefectMode::ReplayClobberOlderFrame`
/// re-introduces the pre-fix blind clobber and IS caught by this oracle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct OracleConfig {
    pub value_equality: bool,
}

/// Per-floor pass/fail evaluation of an [`OracleCoverage`] sample against AC14's floors.
///
/// This is intentionally value-returning rather than a set of bare assertions: AC5(c)'s
/// single-incarnation control and AC14(e)'s deliberately-below-floor run both need to observe a
/// failing floor without aborting the suite, while the baseline run asserts every floor passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct OracleCoverageFloors {
    pub o1_floor_met: bool,
    pub o2_floor_met: bool,
    pub indeterminate_ratio_floor_met: bool,
}

/// Oracle-run-vacuity guard (AC14) — the counterpart to AC2's crash-vacuity guard. Counts how many
/// times each oracle evaluated against a **non-empty** model (an evaluation against an empty model
/// proves nothing and MUST NOT be counted), plus the derived per-floor pass/fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct OracleCoverage {
    /// O1 evaluations against a non-empty model, performed after a healthy recovery.
    pub o1_healthy_recovery_evaluations: u64,
    /// O2 evaluations against a non-empty model (O2 fires after every op).
    pub o2_evaluations: u64,
    /// Of `o2_evaluations`, how many were skipped as `Indeterminate` (R5.0.3) rather than compared.
    pub o2_indeterminate_skips: u64,
    /// This sample's floors, evaluated against AC14's per-2000-cases-derived rate.
    pub floors: OracleCoverageFloors,
}

/// The reference model's value shape for a single key: LWW-only for this spec.
///
/// # Extension seam (R9, `TG-OR-003`)
/// The OR-Map delta-fold recovery proof (catalogued as `TG-OR-003` in `INVARIANTS.md`; the spec
/// that lands it is found by following that ID) adds an OR-shaped variant here — carrying the
/// observed-remove tag set an OR-Map needs — instead of forking the model. Adding it requires no
/// change to [`WorkOp`] dispatch or to the driver/oracle/shrinking machinery: the OR-Map delta-fold
/// recovery proof lands as a case on this harness, not a fork.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ModelValue {
    /// An acked live value, carrying the epoch-millisecond timestamp it was written with.
    Live { millis: i64 },
    /// An acked tombstone (the key was removed after being live).
    Tombstone,
}

/// The model side of every oracle comparison (R5.0): a record of what the driver did and observed,
/// built exclusively from the driver's own event log and fault schedule — independent of the
/// implementation's own bookkeeping.
///
/// # Binding rule (normative, R5.0)
/// Implementors MUST derive all state exclusively from (a) the ops the driver issued and their ack
/// outcomes, and (b) the fault schedule the driver injected (crash points, store-health flips,
/// clock advances, defect mode). Reading the implementation's own pending/in-flight/seeded/staging
/// state to compute an expected value is forbidden — the one sanctioned exception (AC2's
/// boot-empty assertions) is a check ON the implementation, not a model input, and is therefore not
/// part of this trait.
pub(crate) trait ReferenceModel {
    /// Records the WAL sequence assigned to the op the driver issued at incarnation `incarnation`,
    /// step `step` — transfers an identifier only (R5.0.1). Does not itself change the sequence's
    /// lifecycle state.
    fn bind_sequence(
        &mut self,
        partition: PartitionId,
        incarnation: usize,
        step: usize,
        sequence: Sequence,
    );

    /// Records that the driver's store call for (`incarnation`, `step`) returned `Ok(())`: updates
    /// the acked-value-per-key map and transitions the bound sequence (if any) to `Acked`. An `Err`,
    /// a panic, or a call still parked when the incarnation ends is not an ack and MUST NOT be
    /// reported through this method.
    fn record_ack(&mut self, incarnation: usize, step: usize, key: Key, value: ModelValue);

    /// Records that a healthy `FlushTick` drained `sequence` under the replicated flush policy
    /// (due-time and coalescing derived from the driver's own `WriteBehindConfig`, never a copied
    /// constant), transitioning it to `DurablyApplied`.
    fn record_durably_applied(&mut self, partition: PartitionId, sequence: Sequence);

    /// Marks `sequence` `Indeterminate` — attribution genuinely ambiguous (R5.0.3).
    fn record_indeterminate(&mut self, partition: PartitionId, sequence: Sequence);

    /// The `IncarnationEnd::Crash` transition: sequences `Acked` but not yet `DurablyApplied` stay
    /// unresolved across the boundary (R5.0.2's Crash clause) — that is exactly the state the
    /// harness exists to police, so this method MUST NOT resolve them.
    fn on_crash(&mut self);

    /// The set of sequences for `partition` that are `Acked` and neither `DurablyApplied` nor
    /// `Indeterminate` — the model side of every O2 "unresolved" term.
    fn unresolved(&self, partition: PartitionId) -> Vec<Sequence>;

    /// The latest acked value for `key`, or `None` if the model holds no acked write for it — the
    /// model side of O1.
    fn acked_value(&self, key: Key) -> Option<&ModelValue>;

    /// `true` once the model holds an acked value for at least one key — used by the AC14 coverage
    /// guard to avoid counting an evaluation against an empty model.
    fn is_non_empty(&self) -> bool;
}

/// A typed oracle violation. Every variant's doc-comment names the catalogued `TG-<DOMAIN>-<NNN>`
/// invariant ID it violates, so a reader follows the ID into `INVARIANTS.md` to find the contract
/// and the spec that established it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum InvariantViolation {
    /// `TG-WAL-005` / `TG-WB-001`: an acked write is unrecoverable — the model holds an acked value
    /// for `key` that recovery does not reproduce (O1), or the watermark advanced past it before it
    /// was durably applied (O2). Carries the lost key and the incarnation index at which the loss
    /// became observable.
    AckedWriteLost { key: Key, incarnation: usize },

    /// `TG-WAL-005` (the O2 `wal.unapplied(p)` clause): the implementation's `wal.unapplied(p)`
    /// does not contain a sequence the model still holds as acked-but-not-durably-applied — a frame
    /// that should still be replayed on next boot was filtered out.
    UnappliedFrameFiltered {
        partition: PartitionId,
        sequence: Sequence,
    },

    /// `TG-WAL-005` (the O2 `W(p)` clause): the implementation's watermark advanced to or past a
    /// sequence the model still holds as unresolved for `partition` — the exact off-by-one shape
    /// `InclusiveOffByOne` (C13) re-introduces.
    WatermarkAboveUnresolved {
        partition: PartitionId,
        sequence: Sequence,
    },

    /// `TG-WAL-003`: a sealed segment holding a model-unresolved sequence was unlinked before the
    /// watermark sidecar's write+fsync completed — the GC-ordering hazard the R7 crash-point seam
    /// and `GcOrderMode::UnlinkThenFsync` re-introduce.
    SegmentUnlinkedBeforeWatermarkFsync {
        partition: PartitionId,
        sequence: Sequence,
    },

    /// `TG-WAL-006`: the recovered `RecordValue::Lww` value for `key` carries a timestamp STRICTLY
    /// OLDER than the model's latest acked HLC millis — a stale re-replayed frame clobbered a newer
    /// durable value. Only reported when the `value_equality` oracle is enabled. A recovered value
    /// newer than the model's last-arrival is the LWW winner and is NOT flagged. Carries the
    /// incarnation the mismatch surfaced at, the model's expected millis, and the recovered millis.
    AckedValueMismatch {
        key: Key,
        incarnation: usize,
        expected_millis: i64,
        recovered_millis: i64,
    },
}
