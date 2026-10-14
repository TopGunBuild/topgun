//! Write-behind buffering layer for [`MapDataStore`].
//!
//! Decouples mutation latency from persistence latency by buffering writes
//! in per-partition coalesced queues and flushing them on a configurable schedule.
//! An optional WAL ensures every acked write is durable before the ack is returned.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ops::Bound;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use dashmap::DashMap;
use tokio::sync::{watch, Notify};
use topgun_core::{fnv1a_hash, PARTITION_COUNT};
use tracing::{error, warn};

use crate::storage::engine::SlotCell;
use crate::storage::map_data_store::{
    merkle_leaf_hash, LeafSink, Loaded, MapDataStore, MerkleLeaf, ScanBatch, ScanCursor,
    StoreBrokenCause, StoreErrorClass, WriteSource,
};
use crate::storage::record::RecordValue;
use crate::storage::wal::{
    describe_wal_watermark_metrics, OrDelta, Wal, WalEntry, WalFsyncPolicy, WalOp, WalStorePayload,
    WAL_WATERMARK_LAG_GAUGE,
};

/// Boot pending-map seeding seam (C12 re-introduction). Reused from the
/// crash/recovery harness's vocabulary rather than duplicated here — see
/// `wal_harness::BootSeedMode`'s doc-comment for the production/defect
/// semantics of each variant.
#[cfg(test)]
use self::wal_harness::BootSeedMode;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for the write-behind buffering layer.
///
/// Controls flush timing, batch sizes, retry behavior, capacity limits, and
/// the maximum time to wait for a graceful-shutdown drain before giving up.
/// WAL fields control crash-safe durability for acked writes.
#[derive(Debug, Clone)]
pub struct WriteBehindConfig {
    /// Delay in milliseconds before a queued entry becomes eligible for flush.
    pub write_delay_ms: u64,
    /// How often the background flush task runs, in milliseconds.
    pub flush_interval_ms: u64,
    /// Accepted and parsed, and currently without effect.
    ///
    /// Nothing reads it: a flush pass takes every entry that is due when it
    /// begins and stores them one at a time, so the value neither groups nor
    /// limits the pass. It is kept so that a configuration which sets it
    /// still loads.
    pub batch_size: u32,
    /// The wait after the first refusal of an entry is twice this, and the
    /// first length of a store-wide hold is this, in milliseconds; each
    /// doubles with every further refusal, up to `backoff_cap_ms`.
    pub backoff_base_ms: u64,
    /// The longest wait between two attempts of one entry, and the longest
    /// store-wide hold, in milliseconds.
    pub backoff_cap_ms: u64,
    /// Soft bound on the number of buffered keys. `0` disables the check.
    ///
    /// What is counted: buffered keys — queued entries plus the entries a
    /// flush pass holds, which stay counted until each is persisted or
    /// retired into a newer write of its key. An entry the store refuses is
    /// neither: it goes back to its queue and stays counted until the store
    /// takes it.
    ///
    /// The rule: a write (`add`, `remove`, each key of `remove_all`) of a key
    /// that is not buffered is refused while the count is at or above
    /// `capacity`, with the error `Write-behind capacity exceeded`. A write of
    /// a buffered key is always accepted.
    ///
    /// It is not a hard limit. The check and the queue insert are separate
    /// steps with nothing reserved between them. With `C = capacity > 0`:
    ///
    /// ```text
    /// pending_count  <=  C - 1 + W + H
    /// H              <=  C - 1 + W
    /// H              <=  1                      while a store-wide hold is in force
    /// ```
    ///
    /// - `W` is the number of writes of distinct keys that are not buffered
    ///   and that are, at one instant, between the check and their queue
    ///   insert (the WAL append lies in between). `W >= 1` while any write is
    ///   in progress: the writer that is inserting is itself past its check,
    ///   so with one writer and no flush pass the count reaches `C`.
    /// - `H` is the number of entries the running flush pass holds whose key
    ///   was re-written while the pass holds them; each such key is counted a
    ///   second time, once. A pass holds every entry that was due when it
    ///   began — `batch_size` does not bound it — hence the second line.
    ///
    /// In the worst case the count stays below `2 * (C + W)`.
    ///
    /// How long the bound is met: while the store refuses, nothing leaves the
    /// buffer, so the count stays at the bound until the store accepts again,
    /// and for that long every write of a key that is not buffered is refused.
    /// An entry waiting for its next attempt is in its queue, not in a pass,
    /// and adds nothing to `H`; a hold puts back every entry the pass did not
    /// attempt, and a pass during a hold holds at most the one entry it probes
    /// the store with.
    ///
    /// Precondition, which the callers uphold and this store does not
    /// enforce: one writer per key at a time (`TG-KEY-001`).
    ///
    /// What the formula rests on inside the store (`TG-WB-007`): a key's
    /// staging slot exists only while an entry of the key is queued or held
    /// by a flush pass, or while the key's writer is between its `stage` call
    /// and its queue insert — a stretch with no await and no lock held, so it
    /// ends by itself. Not covered: a panic inside that stretch leaves a slot
    /// with nothing buffered.
    pub capacity: u64,
    /// Maximum milliseconds to wait for the write-behind buffer to drain on
    /// graceful shutdown. On timeout, still-pending ops are logged and the
    /// process exits rather than blocking termination indefinitely.
    pub shutdown_timeout_ms: u64,
    /// Directory for WAL files. Must be on the same volume as the durable store
    /// to avoid split-brain on partial-disk failure.
    pub wal_dir: PathBuf,
    /// Controls how aggressively the WAL writer calls fsync after writing frames.
    ///
    /// The default is `Batched`: a write is acked after its frame is appended, but
    /// the fsync is deferred to a ~10ms group-commit timer (or a 100-frame flush).
    /// This is the throughput-favouring default — it does NOT make an acked write
    /// durable under an unclean `kill -9`: writes inside the group-commit window are
    /// lost. That is an accepted tradeoff for the single-node demo tier, where the
    /// originating client still holds the write locally and re-converges on the next
    /// delta-sync. Set `TOPGUN_WAL_FSYNC_POLICY=per_op` to make every acked write
    /// fsynced-before-ack (acked == durable) at a large per-write latency cost.
    pub wal_fsync_policy: WalFsyncPolicy,
    /// How long a partition's smallest un-resolved WAL sequence may stay put,
    /// under ongoing writes, before the stalled-watermark alarm fires.
    ///
    /// It must sit an order of magnitude above the longest LEGITIMATE
    /// non-advance, or the alarm fires on correct behaviour. The default is
    /// derived from the OTHER defaults in this struct (write delay + flush
    /// interval + the longest wait between two attempts of a refused entry,
    /// and clear of the shutdown timeout), so raising `flush_interval_ms` or
    /// `shutdown_timeout_ms` requires raising this alongside them. A store that
    /// refuses for longer than the bound is not a legitimate non-advance: the
    /// refused entry is retried until it is stored, its sequence stays
    /// un-resolved for as long, and the alarm fires for the whole of it.
    pub wal_watermark_stall_bound_ms: u64,
    /// Whether an OR mutation may be framed as a per-op delta in the WAL
    /// instead of a full record snapshot.
    ///
    /// This is the SINGLE arming flag of the delta-framing seam, and it arms
    /// BOTH halves of it: it is what
    /// [`wants_or_witness()`](MapDataStore::wants_or_witness) returns, and it is
    /// the left conjunct of the framing branch in
    /// [`add_with_witness()`](MapDataStore::add_with_witness). One flag, so the
    /// demand gate and the emission arm cannot diverge — a store that demands a
    /// witness always frames it, and a store that frames deltas is always fed
    /// one. A second, independent switch for either half would permit exactly
    /// the divergence this field exists to make impossible.
    ///
    /// `false` is a true zero-cost rollback rather than a discarded-work one:
    /// the demand gate answers `false` first, so the mutation path never builds
    /// a witness at all, and every write frames the full snapshot exactly as it
    /// did before the seam existed.
    pub or_delta_wal: bool,
}

/// Absolute lower clamp on [`WriteBehindConfig::reconfirm_delay_ms`].
///
/// The re-confirmation delay exists to outlast the classifier's own
/// drain-between-probes window, which is bounded by a task schedule. A `0` delay
/// would take both samples back-to-back and collapse the two-sample rule into
/// the single sample it replaced, so the floor makes non-zero a property of the
/// type rather than of operator discipline.
const RECONFIRM_FLOOR_MS: u64 = 100;

/// Absolute lower clamp on [`WriteBehindConfig::watchdog_tick_ms`].
///
/// Same reasoning as [`RECONFIRM_FLOOR_MS`]: a tick derived by division must
/// never reach zero, or the watchdog task spins.
const WATCHDOG_TICK_FLOOR_MS: u64 = 100;

/// The smallest stall bound an operator may set.
///
/// Chosen so the two guards MEET rather than overlap ambiguously: at exactly
/// this bound, `6_000 / 60 == 100 == RECONFIRM_FLOOR_MS`. Below it the derived
/// re-confirmation delay would be governed by the floor alone while the boot
/// line still echoes the operator's value, which is why a lower value is fatal
/// rather than clamped.
const MIN_WAL_WATERMARK_STALL_BOUND_MS: u64 = 6_000;

impl Default for WriteBehindConfig {
    fn default() -> Self {
        Self {
            write_delay_ms: 1000,
            flush_interval_ms: 1000,
            batch_size: 100,
            backoff_base_ms: 500,
            backoff_cap_ms: 5000,
            capacity: 10_000,
            shutdown_timeout_ms: 30_000,
            wal_dir: PathBuf::from("./topgun-wal"),
            wal_fsync_policy: WalFsyncPolicy::Batched,
            wal_watermark_stall_bound_ms: 60_000,
            or_delta_wal: true,
        }
    }
}

impl WriteBehindConfig {
    /// Construct [`WriteBehindConfig`] from environment variables.
    ///
    /// Reads env vars; most missing or unparseable vars fall back to the
    /// corresponding [`Self::default`] field and emit a `tracing::warn!`. Two
    /// misconfigurations are fatal instead — see [`Self::from_source`]'s
    /// `# Panics` section for both and for why each is not silently downgraded.
    ///
    /// | Env var | Field | Default |
    /// |---|---|---|
    /// | `TOPGUN_WRITEBEHIND_FLUSH_INTERVAL_MS` | `flush_interval_ms` | 1000 ms |
    /// | `TOPGUN_WRITEBEHIND_BATCH_SIZE` | `batch_size` | 100 — accepted and currently without effect |
    /// | `TOPGUN_WRITEBEHIND_CAPACITY` | `capacity` | 10 000 |
    /// | `TOPGUN_WRITEBEHIND_SHUTDOWN_TIMEOUT_MS` | `shutdown_timeout_ms` | 30 000 ms |
    /// | `TOPGUN_WAL_DIR` | `wal_dir` | `./topgun-wal` |
    /// | `TOPGUN_WAL_FSYNC_POLICY` | `wal_fsync_policy` | `batched` |
    /// | `TOPGUN_WAL_WATERMARK_STALL_BOUND_MS` | `wal_watermark_stall_bound_ms` | 60 000 ms — raise it alongside `TOPGUN_WRITEBEHIND_FLUSH_INTERVAL_MS` / `TOPGUN_WRITEBEHIND_SHUTDOWN_TIMEOUT_MS`, whose defaults this default is derived from |
    /// | `TOPGUN_OR_DELTA_WAL` | `or_delta_wal` | `true` — the delta-framing kill-switch; `false` restores full-snapshot framing for every OR write |
    ///
    /// Fields not covered by env vars (`write_delay_ms`, `backoff_base_ms`,
    /// `backoff_cap_ms`) retain their [`Self::default`] values.
    #[must_use]
    pub fn from_env() -> Self {
        Self::from_source(|key| std::env::var(key).ok())
    }

    /// Construct [`WriteBehindConfig`] from an injected variable source.
    ///
    /// `get` maps a variable name to its value (or `None` if unset). This is the
    /// testable seam behind [`Self::from_env`], which simply passes a closure
    /// over `std::env::var`. Tests pass a map-backed closure so they exercise the
    /// full parse logic without mutating process-global environment state —
    /// eliminating the cross-test env race that made parallel runs flaky.
    ///
    /// # Panics
    ///
    /// Panics (refuses to start) if `TOPGUN_WAL_FSYNC_POLICY` is set to a value
    /// that does not parse to a known [`WalFsyncPolicy`]. Silently downgrading an
    /// unknown durability policy to the weaker default is what masked a durability
    /// regression through a full RED soak, so the misconfiguration is made fatal at
    /// startup rather than surfacing as data loss on the next unclean shutdown.
    ///
    /// Panics (refuses to start) if `TOPGUN_WAL_WATERMARK_STALL_BOUND_MS` PARSES
    /// but is below [`MIN_WAL_WATERMARK_STALL_BOUND_MS`]. This is a different case
    /// from an unparseable value, which warns and falls back: an unparseable value
    /// leaves the operator's intent unknown and costs nothing to default, whereas
    /// an out-of-range value is honoured, echoed into the boot line as the
    /// effective bound, and silently disarms the two-sample confirmation that is
    /// the fatal stalled-watermark alarm's only false-positive guard. That is a
    /// fatal-path guard being disabled, not an alarm threshold being mis-set.
    #[must_use]
    pub fn from_source<F: Fn(&str) -> Option<String>>(get: F) -> Self {
        let defaults = Self::default();
        let mut cfg = defaults.clone();

        // Parse TOPGUN_WRITEBEHIND_FLUSH_INTERVAL_MS → flush_interval_ms (u64)
        if let Some(raw) = get("TOPGUN_WRITEBEHIND_FLUSH_INTERVAL_MS") {
            match raw.trim().parse::<u64>() {
                Ok(ms) => {
                    cfg.flush_interval_ms = ms;
                }
                Err(err) => {
                    tracing::warn!(
                        target: "topgun_server::storage::write_behind",
                        var = "TOPGUN_WRITEBEHIND_FLUSH_INTERVAL_MS",
                        value = %raw,
                        error = %err,
                        default = defaults.flush_interval_ms,
                        "Failed to parse env var; using default"
                    );
                    cfg.flush_interval_ms = defaults.flush_interval_ms;
                }
            }
        }

        // Parse TOPGUN_WRITEBEHIND_BATCH_SIZE → batch_size (u32). The value is
        // validated so a deployment that sets it keeps starting; no flush
        // pass reads it.
        if let Some(raw) = get("TOPGUN_WRITEBEHIND_BATCH_SIZE") {
            match raw.trim().parse::<u32>() {
                Ok(size) => {
                    cfg.batch_size = size;
                }
                Err(err) => {
                    tracing::warn!(
                        target: "topgun_server::storage::write_behind",
                        var = "TOPGUN_WRITEBEHIND_BATCH_SIZE",
                        value = %raw,
                        error = %err,
                        default = defaults.batch_size,
                        "Failed to parse env var; using default"
                    );
                    cfg.batch_size = defaults.batch_size;
                }
            }
        }

        // Parse TOPGUN_WRITEBEHIND_CAPACITY → capacity (u64)
        if let Some(raw) = get("TOPGUN_WRITEBEHIND_CAPACITY") {
            match raw.trim().parse::<u64>() {
                Ok(cap) => {
                    cfg.capacity = cap;
                }
                Err(err) => {
                    tracing::warn!(
                        target: "topgun_server::storage::write_behind",
                        var = "TOPGUN_WRITEBEHIND_CAPACITY",
                        value = %raw,
                        error = %err,
                        default = defaults.capacity,
                        "Failed to parse env var; using default"
                    );
                    cfg.capacity = defaults.capacity;
                }
            }
        }

        // Parse TOPGUN_WRITEBEHIND_SHUTDOWN_TIMEOUT_MS → shutdown_timeout_ms (u64)
        if let Some(raw) = get("TOPGUN_WRITEBEHIND_SHUTDOWN_TIMEOUT_MS") {
            match raw.trim().parse::<u64>() {
                Ok(ms) => {
                    cfg.shutdown_timeout_ms = ms;
                }
                Err(err) => {
                    tracing::warn!(
                        target: "topgun_server::storage::write_behind",
                        var = "TOPGUN_WRITEBEHIND_SHUTDOWN_TIMEOUT_MS",
                        value = %raw,
                        error = %err,
                        default = defaults.shutdown_timeout_ms,
                        "Failed to parse env var; using default"
                    );
                    cfg.shutdown_timeout_ms = defaults.shutdown_timeout_ms;
                }
            }
        }

        // Parse TOPGUN_WAL_DIR → wal_dir (PathBuf)
        if let Some(raw) = get("TOPGUN_WAL_DIR") {
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                cfg.wal_dir = PathBuf::from(trimmed);
            }
        }

        // Parse TOPGUN_WAL_FSYNC_POLICY → wal_fsync_policy (WalFsyncPolicy)
        if let Some(raw) = get("TOPGUN_WAL_FSYNC_POLICY") {
            cfg.wal_fsync_policy = Self::parse_fsync_policy(&raw);
        }

        // Parse TOPGUN_WAL_WATERMARK_STALL_BOUND_MS → wal_watermark_stall_bound_ms
        //
        // Unparseable warns and falls back: this is an alarm threshold, not a
        // durability policy, so a bad value cannot lose data and refusing to boot
        // over one would trade a durability guard for an availability outage.
        //
        // A value that PARSES but is below the minimum is FATAL, for the same
        // reason the fsync policy above is: it is honoured, echoed into the boot
        // line as the effective bound, and disarms the two-sample confirmation
        // that keeps the fatal alarm from firing on correct code.
        if let Some(raw) = get("TOPGUN_WAL_WATERMARK_STALL_BOUND_MS") {
            cfg.wal_watermark_stall_bound_ms =
                Self::parse_stall_bound_ms(&raw, defaults.wal_watermark_stall_bound_ms);
        }

        // Parse TOPGUN_OR_DELTA_WAL → or_delta_wal
        //
        // Read ONCE here, never per write: the flag arms both the demand gate
        // and the framing branch, and a value that could change under a running
        // store would let those two halves observe different answers for the
        // same op.
        if let Some(raw) = get("TOPGUN_OR_DELTA_WAL") {
            cfg.or_delta_wal = Self::parse_or_delta_wal(&raw);
        }

        cfg
    }

    /// Parses one `TOPGUN_OR_DELTA_WAL` value.
    ///
    /// Only an explicit falsey word disables the seam. An unrecognised value
    /// leaves it ARMED rather than rolling an operator back to snapshot framing
    /// they did not ask for: this is a kill-switch, and a typo in one must not
    /// quietly change the durable encoding a running node writes.
    ///
    /// Staying armed is deliberate, but staying SILENT about it is not. A
    /// misspelled disarm (`flase`, or a value some env systems deliver with its
    /// quotes still attached) otherwise leaves an operator believing the WAL is
    /// safe to roll back across while the node keeps writing delta frames, and
    /// the effective-value boot line reports the armed truth nobody asked about.
    /// The warning names the unrecognised value so the typo is visible at boot
    /// rather than at the downgrade that fails because of it.
    fn parse_or_delta_wal(raw: &str) -> bool {
        let normalized = raw.trim().to_lowercase();
        if matches!(normalized.as_str(), "false" | "0" | "no" | "off") {
            return false;
        }
        if !matches!(normalized.as_str(), "true" | "1" | "yes" | "on") {
            tracing::warn!(
                target: "topgun_server::storage::write_behind",
                var = "TOPGUN_OR_DELTA_WAL",
                value = %raw,
                "Unrecognised value; leaving OR delta framing ARMED. Only \
                 false/0/no/off (case-insensitive) disarm it — check for a typo \
                 if you meant to roll this node back to full-snapshot framing"
            );
        }
        true
    }

    /// Parses one `TOPGUN_WAL_FSYNC_POLICY` value.
    ///
    /// An unknown value is FATAL: silently falling back to the `Batched` default
    /// here is exactly what masked a durability regression through a full RED
    /// soak (a misspelled durable policy quietly ran with the weaker default).
    /// Refusing to start forces the misconfiguration to surface at boot rather
    /// than as silent data loss on the next unclean shutdown.
    ///
    /// # Panics
    ///
    /// Panics if `raw` does not parse to a known [`WalFsyncPolicy`].
    fn parse_fsync_policy(raw: &str) -> WalFsyncPolicy {
        match raw.trim().parse::<WalFsyncPolicy>() {
            Ok(policy) => policy,
            Err(err) => {
                panic!(
                    "TOPGUN_WAL_FSYNC_POLICY={raw:?} is not a valid WAL fsync policy: {err}. \
                     Refusing to start: an unknown durability policy must not be silently \
                     downgraded to a weaker default. Set a valid value or unset the variable."
                );
            }
        }
    }

    /// Parses one `TOPGUN_WAL_WATERMARK_STALL_BOUND_MS` value.
    ///
    /// See [`Self::from_source`]'s `# Panics` section for why the two failure
    /// modes are disposed of differently.
    fn parse_stall_bound_ms(raw: &str, default: u64) -> u64 {
        match raw.trim().parse::<u64>() {
            Ok(ms) if ms < MIN_WAL_WATERMARK_STALL_BOUND_MS => {
                panic!(
                    "TOPGUN_WAL_WATERMARK_STALL_BOUND_MS={ms} is below the minimum of \
                     {MIN_WAL_WATERMARK_STALL_BOUND_MS} ms. Refusing to start: a shorter bound \
                     shrinks the derived re-confirmation delay below the window it exists to \
                     absorb, which disarms the stalled-watermark alarm's only false-positive \
                     guard while the boot line still reports the value as effective."
                );
            }
            Ok(ms) => ms,
            Err(err) => {
                tracing::warn!(
                    target: "topgun_server::storage::write_behind",
                    var = "TOPGUN_WAL_WATERMARK_STALL_BOUND_MS",
                    value = %raw,
                    error = %err,
                    default,
                    "Failed to parse env var; using default"
                );
                default
            }
        }
    }

    /// How long the stalled-watermark classifier waits between the two samples a
    /// `TrackerLeak` verdict requires.
    ///
    /// Derived rather than independently settable: the quantity has no standalone
    /// operational meaning — it is "long enough to outlast the benign
    /// drain-between-probes window, short enough to be noise against the stall
    /// bound" — and both halves are expressed relative to the bound. A separate
    /// knob would let an operator raise one while the other went stale, with the
    /// stale one sitting on the fatal path.
    ///
    /// The floor is what makes the lower half hold at every admissible bound; the
    /// divisor is a literal, so the division cannot trap and no subtraction on
    /// this path can underflow toward a zero delay.
    #[must_use]
    pub fn reconfirm_delay_ms(&self) -> u64 {
        (self.wal_watermark_stall_bound_ms / 60).max(RECONFIRM_FLOOR_MS)
    }

    /// How often the stalled-watermark watchdog task takes a classifier sample.
    ///
    /// The bound is how long a stall must persist before it is reportable, so the
    /// probe must be frequent enough to detect one within a small fraction of it
    /// and infrequent enough that the per-tick queue scan stays off every hot
    /// path — it is paid once per tick, never per write. Floored for the same
    /// reason as [`Self::reconfirm_delay_ms`]: a zero tick would spin.
    #[must_use]
    pub fn watchdog_tick_ms(&self) -> u64 {
        (self.wal_watermark_stall_bound_ms / 10).max(WATCHDOG_TICK_FLOOR_MS)
    }
}

// ---------------------------------------------------------------------------
// Delayed operation types
// ---------------------------------------------------------------------------

/// A buffered operation waiting to be flushed to the inner store.
#[derive(Debug, Clone)]
pub(crate) enum DelayedOp {
    /// Buffered add/update with the value and expiration time.
    Store {
        value: RecordValue,
        expiration_time: i64,
    },
    /// Buffered in-place write: the flush persists the cell's value as it is
    /// at flush time.
    StoreCell {
        cell: Arc<SlotCell>,
        expiration_time: i64,
    },
    /// Buffered remove (tombstone).
    Remove,
}

// ---------------------------------------------------------------------------
// WAL-sequence pending tracker — tags and alarm classes
// ---------------------------------------------------------------------------

/// Why a pending `wal_seq` is un-resolved — the input the stalled-pending alarm
/// classifier dispatches on.
///
/// The tag is what distinguishes a code bug from a correctly-surfaced stall: a
/// sequence absent from both the partition queue and the in-flight registry is
/// NORMAL for three of these four tags, so a bare "absent from both means leak"
/// rule would fire on healthy writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PendingOrigin {
    /// Seeded at boot from a frame a PRIOR incarnation left un-applied. No live
    /// entry owns it and none ever will; it resolves only by replaying on a
    /// future restart. A stall here is correct surfacing, not a leak.
    BootUnreplayed,
    /// Assigned by this process; its frame is STILL BEING APPENDED and no entry
    /// exists yet. The owner is the calling task, parked in the WAL append —
    /// which under a per-op fsync policy is a device-barrier fsync. Absent from
    /// the queue AND from the registry is this state's normal, so the registry is
    /// not consulted for it. A stall here means a hung or full disk.
    Appending,
    /// Assigned by this process and ENQUEUED — the append returned `Ok`, so a
    /// frame exists and an entry owns the sequence, either queued or in-flight.
    /// The in-flight registry is what tells those two apart, and it is consulted
    /// only for this tag. Because this tag is entered only at the queue insert,
    /// absent-from-both means precisely "was queued and dequeued, but is missing
    /// from the registry".
    Live,
    /// This process assigned it, and its entry hit an abandoned terminal. The
    /// store is rejecting an acked write; the frame stays replayable.
    Abandoned,
}

/// Why a partition's applied watermark has stopped advancing.
///
/// The class is an enum rather than a log-message distinction so the alarm can
/// be asserted by type, and so the operator action it implies cannot drift away
/// from the condition that produced it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WalWatermarkAlarm {
    /// A `Live` sequence that is in NEITHER the partition queue NOR the in-flight
    /// registry: a missed resolve, a sequence no one owns. A code bug. Nothing is
    /// at risk except unbounded WAL growth. A boot-seeded, appending or abandoned
    /// sequence is also absent from both and is NOT a leak, which is why the
    /// classifier reads the origin tag first.
    TrackerLeak,
    /// The store is rejecting or hanging on an acked write, or the WAL itself is
    /// hanging on the append, or a prior incarnation left a frame un-replayed.
    /// The data is safe in the WAL — or, for `Appending`, was never acked — and
    /// the operator must act.
    ///
    /// `origin` carries the cause at the type level and is load-bearing: it is
    /// what sends the operator to the disk rather than to the backend.
    AbandonedWrite { origin: PendingOrigin },
}

impl WalWatermarkAlarm {
    /// The operator action this alarm prescribes, derived from the class and —
    /// for an abandoned write — from the origin that caused it.
    ///
    /// Derived rather than written at each emission site so the message cannot
    /// drift away from the condition that produced it: the cause lives in the
    /// type, and the prose is a rendering of it.
    fn operator_action(self) -> &'static str {
        match self {
            Self::TrackerLeak => {
                "WAL watermark stalled on a sequence no one owns: it was queued, dequeued, and \
                 never resolved or registered. This is a code bug — investigate the write-behind \
                 terminal paths. No data is at risk; the WAL grows until it is fixed."
            }
            Self::AbandonedWrite {
                origin: PendingOrigin::Appending,
            } => {
                "WAL watermark stalled inside the WAL append itself: the disk is hung or full. \
                 Investigate the DISK, not the backend store. Nothing is lost — the write was \
                 never acked."
            }
            Self::AbandonedWrite {
                origin: PendingOrigin::Live,
            } => {
                "WAL watermark stalled on a write the store has not accepted: the backend is down \
                 or its disk is full. The write stays buffered and is retried until the store \
                 accepts it; its frame is safe in the WAL."
            }
            Self::AbandonedWrite {
                origin: PendingOrigin::BootUnreplayed,
            } => {
                "WAL watermark stalled on a frame a PRIOR incarnation left un-applied. It resolves \
                 by replaying on a future restart; WAL GC stays held back until it does."
            }
            Self::AbandonedWrite {
                origin: PendingOrigin::Abandoned,
            } => {
                "WAL watermark stalled on an acked write a shutdown gave up on: the store had not \
                 accepted it when the node stopped. Its frame replays at the next start."
            }
        }
    }
}

/// Outcome of ONE stalled-watermark classifier sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SampleOutcome {
    /// Nothing pending for the partition, or no WAL to track.
    Healthy,
    /// A verdict, on a single sample for every class but `TrackerLeak`.
    Alarm {
        /// The stalled sequence — the partition's smallest un-resolved one.
        sequence: u64,
        /// Which of the two categorically different alarms this is.
        alarm: WalWatermarkAlarm,
    },
    /// `Live` and absent from BOTH the queue and the registry on THIS sample.
    ///
    /// Deliberately not a verdict: a sequence merely drained between the
    /// registry probe and the queue scan reads exactly like this, which is a
    /// false positive on correct code. Only a second, independent sample that
    /// reads the same way promotes it to `TrackerLeak` — and a genuine leak,
    /// being permanent, always survives that.
    LeakCandidate {
        /// The sequence that read ownerless.
        sequence: u64,
    },
}

/// Per-partition bookkeeping the stall watchdog carries between ticks.
#[derive(Debug, Default)]
struct StallWatch {
    /// Per partition: the smallest pending sequence last observed, the highest
    /// sequence assigned at that moment, and when the pair was first seen.
    ///
    /// A verdict is reportable only once the SAME smallest sequence has held its
    /// position for the whole stall bound WHILE further sequences were assigned.
    /// Both halves are load-bearing: without the duration an ordinary retry
    /// ladder reports, and without the ongoing-writes half an idle partition
    /// reports a stall that is simply quiet.
    observed: HashMap<u32, StallObservation>,
    /// A sequence that read absent-from-both on the previous sample, awaiting
    /// the confirming one.
    leak_candidate: HashMap<u32, u64>,
}

/// One partition's stall observation, carried between watchdog ticks.
#[derive(Debug, Clone, Copy)]
struct StallObservation {
    /// The smallest un-resolved sequence when the observation opened.
    sequence: u64,
    /// The highest assigned sequence when the observation opened; growth past it
    /// is what makes writes "ongoing".
    max_assigned: u64,
    /// When the observation opened.
    since: std::time::Instant,
}

/// How the per-partition applied watermark is computed.
///
/// Test-only, and it exists so the prefix-complete rule and the naive scalar-max
/// rule it replaces can both run in one committed suite: the differential test
/// must FAIL under `ScalarMax` and PASS under `PrefixComplete`, which keeps the
/// negative control live instead of relying on a one-off manual revert.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum WatermarkMode {
    /// The largest resolved sequence, ignoring holes below it — the rule that
    /// lets a coalesced high sequence mark lower, still-buffered frames applied.
    ScalarMax,
    /// The largest sequence whose entire prefix is resolved.
    #[default]
    PrefixComplete,
    /// C13 re-introduction: returns `min(pending)` instead of `min(pending) - 1`
    /// — the inclusive off-by-one that marks the smallest still-unresolved
    /// sequence applied instead of stopping one below it. Carries its own
    /// `#[cfg(test)]` (on top of the enum-level one) so neither the variant nor
    /// any branch handling it can reach a release build.
    #[cfg(test)]
    InclusiveOffByOne,
}

/// Test-only instrumentation of the stalled-pending classifier.
///
/// The two-sample confirmation the classifier applies is only provable if a test
/// can (a) drive a sample on demand rather than waiting on the watchdog interval,
/// (b) mutate state at the exact point between the registry probe and the queue
/// scan where a transient ownerless read is produced, and (c) verify a second
/// sample was actually taken, so a classifier that never fired cannot satisfy the
/// assertion by doing nothing.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct ClassifierSeam {
    /// Samples taken per partition.
    samples: Mutex<HashMap<u32, u64>>,
    /// Invoked between the registry probe and the queue scan of each sample.
    #[allow(clippy::type_complexity)]
    mid_sample: Mutex<Option<Arc<dyn Fn(u32) + Send + Sync>>>,
}

#[cfg(test)]
impl ClassifierSeam {
    /// Records that a sample was taken for `partition`.
    fn record_sample(&self, partition: u32) {
        let mut samples = self
            .samples
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *samples.entry(partition).or_insert(0) += 1;
    }

    /// Number of samples taken for `partition` so far.
    fn sample_count(&self, partition: u32) -> u64 {
        self.samples
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&partition)
            .copied()
            .unwrap_or(0)
    }

    /// Runs the installed mid-sample hook, if any.
    ///
    /// The hook is cloned out before it runs so it can touch store state without
    /// deadlocking against this lock.
    fn run_mid_sample_hook(&self, partition: u32) {
        let hook = self
            .mid_sample
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some(hook) = hook {
            hook(partition);
        }
    }
}

#[cfg(test)]
impl std::fmt::Debug for ClassifierSeam {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClassifierSeam").finish_non_exhaustive()
    }
}

/// The parked side of a one-shot test park: announces the park, then waits
/// for the release (or for the test's handle to be dropped).
#[cfg(test)]
struct ParkGate {
    parked: tokio::sync::oneshot::Sender<()>,
    release: tokio::sync::oneshot::Receiver<()>,
}

#[cfg(test)]
impl ParkGate {
    async fn pass(self) {
        let _ = self.parked.send(());
        let _ = self.release.await;
    }
}

/// The test's side of a one-shot park on one of this store's own paths.
/// Dropping it releases the parked caller, so a failing test never leaves a
/// task parked.
#[cfg(test)]
pub(crate) struct TestParkHandle {
    parked: Option<tokio::sync::oneshot::Receiver<()>>,
    release: Option<tokio::sync::oneshot::Sender<()>>,
}

#[cfg(test)]
impl TestParkHandle {
    fn arm() -> (ParkGate, Self) {
        let (parked_tx, parked_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        (
            ParkGate {
                parked: parked_tx,
                release: release_rx,
            },
            Self {
                parked: Some(parked_rx),
                release: Some(release_tx),
            },
        )
    }

    /// Waits until the caller parks; panics after two seconds, which means the
    /// setup never reached the park point.
    pub(crate) async fn wait_parked(&mut self) {
        let parked = self.parked.take().expect("wait_parked called once");
        tokio::time::timeout(std::time::Duration::from_secs(2), parked)
            .await
            .expect("the store never parked: the setup did not reach the park point")
            .expect("park dropped before parking");
    }

    pub(crate) fn release(&mut self) {
        if let Some(release) = self.release.take() {
            let _ = release.send(());
        }
    }
}

/// One-shot parks on this store's own paths, for tests that must interleave a
/// second caller at a point no wrapper can reach: a wrapper around this store
/// would answer `load_slot` and `add_with_witness` through the trait defaults,
/// i.e. take the value path instead of the cell path under test.
#[cfg(test)]
#[derive(Default)]
struct TestParks {
    /// Parks the next `load_slot` of the key on return, only when it answers
    /// a cell.
    load_slot: Mutex<Option<(String, String, ParkGate)>>,
    /// How many `load_slot` calls parked, so a test that expected the cell
    /// branch fails instead of passing vacuously when it was never taken.
    load_slot_hits: AtomicU64,
    /// Parks the next `remove` of the key once its delete is staged.
    after_remove: Mutex<Option<(String, String, ParkGate)>>,
    /// Parks the next write of the key once it holds its entry sequence,
    /// before it stages the key and before it takes the queue.
    before_queue_insert: Mutex<Option<(String, String, ParkGate)>>,
    /// Parks the next write of the key between the staging of its key and its
    /// queue insert.
    between_stage_and_queue_insert: Mutex<Option<(String, String, ParkGate)>>,
    /// Parks `hard_flush` once the shutdown flag is set, before the flush
    /// loop is joined.
    after_shutdown_flag: Mutex<Option<ParkGate>>,
    /// Parks the flush task between the choice of a probe and its removal
    /// from its queue.
    between_probe_choice_and_removal: Mutex<Option<ParkGate>>,
}

#[cfg(test)]
impl TestParks {
    fn take_keyed(
        slot: &Mutex<Option<(String, String, ParkGate)>>,
        map: &str,
        key: &str,
    ) -> Option<ParkGate> {
        let mut slot = slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if slot
            .as_ref()
            .is_some_and(|(m, k, _)| m.as_str() == map && k.as_str() == key)
        {
            slot.take().map(|(_, _, gate)| gate)
        } else {
            None
        }
    }

    /// Parks the caller on `slot` when it is armed for `(map, key)`. One call
    /// instead of an `if let` at the consult site keeps that site to a single
    /// statement.
    async fn pass_keyed(slot: &Mutex<Option<(String, String, ParkGate)>>, map: &str, key: &str) {
        if let Some(gate) = Self::take_keyed(slot, map, key) {
            gate.pass().await;
        }
    }
}

/// An entry in the write-behind queue representing a pending operation.
#[derive(Debug, Clone)]
pub(crate) struct DelayedEntry {
    /// Map name this entry belongs to.
    pub map: String,
    /// Record key within the map.
    pub key: String,
    /// The buffered operation (store or remove).
    pub operation: DelayedOp,
    /// Wall-clock millis when first enqueued. Preserved on coalesce so the
    /// key flushes on its original schedule.
    pub store_time: i64,
    /// Global ordering counter for fairness during flush.
    pub sequence: u64,
    /// Number of failed flush attempts so far.
    pub retry_count: u32,
    /// When the store may next be asked to take this entry; `None` until the
    /// store has refused it once. Monotonic, so a test can drive it with a
    /// paused clock and a wall-clock step cannot move it.
    pub next_attempt: Option<tokio::time::Instant>,
    /// When the store first refused this key's buffered entry; `None` until
    /// then. Written by the refusal only and never moved by a later one, so
    /// it says for how long the store has been refusing the key.
    pub first_refused: Option<tokio::time::Instant>,
    /// WAL sequences whose frames this entry carries; resolved together at the
    /// entry's terminal outcome.
    ///
    /// A sequence is assigned unconditionally — including when no WAL is
    /// configured — and identifies this entry's frame. With no WAL no frame is
    /// written and nothing tracks the sequence, so it is an inert identifier
    /// rather than a sentinel: there is no "zero means no WAL entry" state,
    /// because the counter starts at 1 and always hands out a real number.
    ///
    /// It is a SET, not a scalar, because a coalesce survivor whose frame does
    /// not subsume its predecessor's must carry that predecessor's sequence
    /// forward and resolve both together.
    pub wal_sequences: BTreeSet<u64>,
}

impl DelayedEntry {
    /// The entry of a first buffered write of a key: never refused, carrying
    /// the one WAL sequence of its own frame.
    fn fresh(
        map: &str,
        key: &str,
        operation: DelayedOp,
        store_time: i64,
        sequence: u64,
        wal_seq: u64,
    ) -> Self {
        Self {
            map: map.to_string(),
            key: key.to_string(),
            operation,
            store_time,
            sequence,
            retry_count: 0,
            next_attempt: None,
            first_refused: None,
            wal_sequences: BTreeSet::from([wal_seq]),
        }
    }

    /// What a write that replaces a queued entry takes from it: its place in
    /// the flush schedule and its retry state, which belong to the key and not
    /// to one write of it. Without the retry state a key that is rewritten
    /// while the store refuses it would be attempted in every pass.
    fn inherit_from(&mut self, replaced: &DelayedEntry) {
        self.store_time = replaced.store_time;
        self.retry_count = replaced.retry_count;
        self.next_attempt = replaced.next_attempt;
        self.first_refused = replaced.first_refused;
    }

    /// Where a pass attempts this entry: the never-refused ones first, in the
    /// order they were written, then the refused ones, the one whose time came
    /// longest ago first. A refused entry therefore never stands between a key
    /// the store has not spoken about and the store.
    fn pass_order(&self) -> (bool, i64, Option<tokio::time::Instant>, u64) {
        if self.retry_count == 0 {
            (false, self.store_time, None, self.sequence)
        } else {
            (true, 0, self.next_attempt, self.sequence)
        }
    }

    /// Whether a flush pass that began at `now` may take this entry.
    fn is_due(&self, deadline: i64, now: tokio::time::Instant) -> bool {
        self.store_time <= deadline && self.next_attempt.is_none_or(|at| at <= now)
    }
}

/// The earlier of two first-refusal instants; an absent one is "no value", not
/// the earliest.
fn earlier_refusal(
    a: Option<tokio::time::Instant>,
    b: Option<tokio::time::Instant>,
) -> Option<tokio::time::Instant> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// What a probe compares of a due entry, and the entry's sequence, which only
/// breaks ties.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ProbeRank {
    never_refused: bool,
    store_time: i64,
    next_attempt: Option<tokio::time::Instant>,
    sequence: u64,
}

impl ProbeRank {
    fn of(entry: &DelayedEntry) -> Self {
        Self {
            never_refused: entry.retry_count == 0,
            store_time: entry.store_time,
            next_attempt: entry.next_attempt,
            sequence: entry.sequence,
        }
    }

    /// Whether `self` is a better probe than `other`. A never-refused entry
    /// goes before a refused one, and of the never-refused the newest: a key
    /// written during a hold is then the next probe, however many refused keys
    /// are buffered. Of the refused the one whose time came longest ago, so
    /// successive probes walk them in turn. `store_time` decides before
    /// `sequence` because a rewritten queued entry keeps its `store_time` and
    /// takes a new, larger sequence.
    fn before(&self, other: &Self) -> bool {
        match (self.never_refused, other.never_refused) {
            (true, false) => true,
            (false, true) => false,
            (true, true) => (self.store_time, self.sequence) > (other.store_time, other.sequence),
            (false, false) => {
                (self.next_attempt, self.sequence) < (other.next_attempt, other.sequence)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Partition queue
// ---------------------------------------------------------------------------

/// What [`PartitionQueue::reinsert_front`] did with a retry entry.
///
/// The flush loop holds a drained entry outside every queue while it persists
/// it, so a newer write of the same key can be queued in the meantime. The
/// caller has to know which case it met: an entry that is not put back has left
/// the queue for good, and its count and sequences still need a disposition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Reinsert {
    /// No entry for the key was queued; the retry entry is back on the queue.
    Requeued,
    /// A newer write for the key was queued while the retry entry was out of the queue.
    Superseded {
        /// The entry sequence (`DelayedEntry::sequence`) of the retry entry.
        retired_sequence: u64,
    },
}

/// Per-partition queue that coalesces writes by (map, key).
///
/// Only the latest value per key is stored; frequently-updated keys
/// do not starve the queue.
#[derive(Debug, Default)]
pub(crate) struct PartitionQueue {
    entries: HashMap<(String, String), DelayedEntry>,
}

impl PartitionQueue {
    /// Inserts or replaces an entry for the given (map, key).
    ///
    /// Returns the previous entry if one was replaced (coalesce),
    /// or `None` if this was a new key.
    pub fn insert(&mut self, entry: DelayedEntry) -> Option<DelayedEntry> {
        let key = (entry.map.clone(), entry.key.clone());
        self.entries.insert(key, entry)
    }

    /// Removes and returns the entry for the given key, or `None` if not present.
    pub fn remove(&mut self, map: &str, key: &str) -> Option<DelayedEntry> {
        self.entries.remove(&(map.to_string(), key.to_string()))
    }

    /// Collects all entries whose `store_time` is at or before the deadline,
    /// whatever their next attempt time: the shutdown drain gives every
    /// buffered entry one last attempt.
    ///
    /// Eligible entries are removed from the queue and returned in no
    /// particular order.
    pub fn drain_ready(&mut self, deadline: i64) -> Vec<DelayedEntry> {
        let ready_keys: Vec<(String, String)> = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.store_time <= deadline)
            .map(|(k, _)| k.clone())
            .collect();

        let mut ready = Vec::with_capacity(ready_keys.len());
        for k in ready_keys {
            if let Some(entry) = self.entries.remove(&k) {
                ready.push(entry);
            }
        }
        ready
    }

    /// Like [`drain_ready`](Self::drain_ready), for a flush pass: leaves behind
    /// the entries whose next attempt is still ahead of `now`.
    pub fn drain_due(&mut self, deadline: i64, now: tokio::time::Instant) -> Vec<DelayedEntry> {
        let due_keys: Vec<(String, String)> = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.is_due(deadline, now))
            .map(|(k, _)| k.clone())
            .collect();

        let mut due = Vec::with_capacity(due_keys.len());
        for k in due_keys {
            if let Some(entry) = self.entries.remove(&k) {
                due.push(entry);
            }
        }
        due
    }

    /// The rank of the entry a probe would take from this queue, with that
    /// entry's `(map, key)`; `None` when nothing in the queue is due.
    pub fn best_due(
        &self,
        deadline: i64,
        now: tokio::time::Instant,
    ) -> Option<(ProbeRank, (String, String))> {
        self.entries
            .iter()
            .filter(|(_, entry)| entry.is_due(deadline, now))
            .map(|(key, entry)| (ProbeRank::of(entry), key))
            .reduce(|best, candidate| {
                if candidate.0.before(&best.0) {
                    candidate
                } else {
                    best
                }
            })
            .map(|(rank, key)| (rank, key.clone()))
    }

    /// Removes and returns the queued entry of the key, whatever entry
    /// sequence it carries now; `None` when the key has no queued entry.
    ///
    /// By key and not by sequence: a writer that replaced the entry after it
    /// was chosen as the probe left the key's buffered entry in its place,
    /// with the same schedule and the newer value, and that is what the probe
    /// has to carry.
    pub fn take(&mut self, map: &str, key: &str) -> Option<DelayedEntry> {
        self.remove(map, key)
    }

    /// How many queued entries the store has refused at least once, and the
    /// earliest first refusal among them.
    pub fn retrying(&self) -> (u64, Option<tokio::time::Instant>) {
        let mut count = 0;
        let mut earliest = None;
        for entry in self.entries.values().filter(|entry| entry.retry_count > 0) {
            count += 1;
            earliest = earlier_refusal(earliest, entry.first_refused);
        }
        (count, earliest)
    }

    /// Re-inserts an entry that failed flush and needs retry, and reports
    /// whether it went back on the queue.
    ///
    /// Preserves original `store_time` so the entry retains its flush priority.
    ///
    /// When a newer write of the key is already queued the retry entry is not
    /// put back: the newer entry carries the value that has to become durable,
    /// so the retry entry is retired into it (see
    /// [`retire_into_queued`](Self::retire_into_queued)) and the caller is told,
    /// because the retired entry's count and entry sequence are the caller's to
    /// settle once the shard guard is gone.
    pub fn reinsert_front(&mut self, entry: DelayedEntry) -> Reinsert {
        if self.retire_into_queued(&entry) {
            return Reinsert::Superseded {
                retired_sequence: entry.sequence,
            };
        }
        self.entries
            .insert((entry.map.clone(), entry.key.clone()), entry);
        Reinsert::Requeued
    }

    /// Hands the WAL sequences of `retired` — an entry that left the queue and
    /// will not come back — to the entry queued for the same key, and reports
    /// whether there was one.
    ///
    /// The sequences are carried, not resolved: the retired frame's effect is
    /// not durable yet, and it becomes durable only through the queued entry.
    /// Carried, they resolve at that entry's own terminal, so the frame stays
    /// replayable for exactly as long as it is needed. Resolving them here
    /// would let another key's flush move the partition watermark past a frame
    /// whose mutation a non-subsuming survivor does not re-carry, and a crash
    /// would then lose it.
    ///
    /// # Assumption: the queued entry covers the retired one
    ///
    /// "Durable only through the queued entry" is true only when storing the
    /// queued entry also stores the retired mutation — that is, when the
    /// queued entry is the NEWER write of the key. One writer per key at a
    /// time guarantees it: the entry queued while this one was out of the
    /// queue was then written after it. Without that precondition the queued
    /// entry could hold the OLDER value. Its flush would then resolve the
    /// carried sequences although the newer value was never stored, and a
    /// restart would recover the older one. Nothing here can tell the two
    /// cases apart: the entries do not carry the value's own timestamp, and a
    /// later arrival with an older value has the higher entry sequence. For
    /// record keys the precondition is upheld by `TG-KEY-001`: every write of
    /// a record holds that record's per-key writer (see `add_with_witness`).
    ///
    /// The queued entry also takes the earlier `store_time`, as a coalesce
    /// does, so a key rewritten during a store outage keeps its original flush
    /// schedule — and, for the same reason, the larger refusal count, the later
    /// next attempt and the earlier first refusal of the two.
    ///
    /// Runs under the caller's shard guard, which is what stops the queued
    /// entry being drained between the lookup and the merge.
    pub fn retire_into_queued(&mut self, retired: &DelayedEntry) -> bool {
        let Some(survivor) = self
            .entries
            .get_mut(&(retired.map.clone(), retired.key.clone()))
        else {
            return false;
        };
        survivor
            .wal_sequences
            .extend(retired.wal_sequences.iter().copied());
        survivor.store_time = survivor.store_time.min(retired.store_time);
        // The retry schedule belongs to the key, not to one entry of it: a key
        // rewritten while the store refuses it must not be attempted more
        // often than a key that is left alone, and must not look as if the
        // store had begun to refuse it only now.
        survivor.retry_count = survivor.retry_count.max(retired.retry_count);
        survivor.next_attempt = survivor.next_attempt.max(retired.next_attempt);
        survivor.first_refused = earlier_refusal(survivor.first_refused, retired.first_refused);
        true
    }

    /// Returns the number of entries in this partition queue.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether any queued entry still carries `seq`.
    ///
    /// Entries are keyed by `(map, key)`, not by WAL sequence, so this is a scan
    /// rather than a lookup. It is bounded by the configured buffer capacity and
    /// paid once per watchdog tick — never per write and never on the flush path.
    pub fn contains_wal_sequence(&self, seq: u64) -> bool {
        self.entries
            .values()
            .any(|entry| entry.wal_sequences.contains(&seq))
    }
}

// ---------------------------------------------------------------------------
// Partition assignment
// ---------------------------------------------------------------------------

/// Current wall-clock time as millis since epoch.
fn now_millis() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(i64::MAX)
}

/// Deterministic partition assignment from (map, key) via `fnv1a_hash`.
fn partition_for(map: &str, key: &str) -> u32 {
    let combined = format!("{map}:{key}");
    fnv1a_hash(&combined) % PARTITION_COUNT
}

// ---------------------------------------------------------------------------
// WriteBehindDataStore
// ---------------------------------------------------------------------------

/// One entry in the staging overlay: the latest buffered value for a key plus
/// the sequence of the write that produced it.
///
/// The `seq` is the identity check the background flush uses to remove a staging
/// entry only when it still corresponds to the write it just persisted. Without
/// it, a flush that drains an older value (`E1`) and then unconditionally clears
/// the staging slot would wipe a NEWER value (`E2`) that coalesced in after the
/// drain dequeued the batch — dropping read-your-writes back to the now-stale
/// durable value until `E2` itself flushes. That window is what breaks
/// read-your-writes under active eviction, where the resident copy is gone and
/// the staging overlay is the only correct source.
#[derive(Clone)]
struct StagingSlot {
    /// Sequence of the write that set this staged value (`DelayedEntry.sequence`).
    seq: u64,
    /// `Some(value)` = pending write, `None` = pending delete.
    value: Option<StagedValue>,
}

/// A pending write's value in the staging overlay: an owned value (a plain
/// `add`), or the cell of an in-place write.
#[derive(Clone)]
enum StagedValue {
    Value(RecordValue),
    Cell(Arc<SlotCell>),
}

impl StagedValue {
    /// An owned copy of the staged value, read under the cell's lock for a cell.
    fn to_value(&self) -> RecordValue {
        match self {
            StagedValue::Value(value) => value.clone(),
            StagedValue::Cell(cell) => cell.lock().value.clone(),
        }
    }
}

/// The queue operation and the staging value for one write. A value is queued
/// and staged as two copies; an in-place write's cell is shared by the queued
/// entry, its staging slot and the engine, so the flush persists whatever the
/// cell holds at flush time and a re-adoption finds the same cell (TG-WB-003).
fn queued_and_staged(src: WriteSource<'_>, expiration_time: i64) -> (DelayedOp, StagedValue) {
    match src {
        WriteSource::Value(value) => (
            DelayedOp::Store {
                value: value.clone(),
                expiration_time,
            },
            StagedValue::Value(value.clone()),
        ),
        WriteSource::Cell(cell) => (
            DelayedOp::StoreCell {
                cell: Arc::clone(cell),
                expiration_time,
            },
            StagedValue::Cell(Arc::clone(cell)),
        ),
    }
}

/// Per-partition `wal_seq` state: which sequences are still unresolved, why,
/// and which are currently owned by a flush worker.
///
/// The in-flight registry shares ONE mutex with the pending map deliberately: a
/// single lock cannot invert against itself, so the mandated
/// registry-before-tracker acquisition order becomes impossible to violate
/// rather than a rule every call site has to remember.
#[derive(Debug, Default)]
pub(crate) struct PartitionTracking {
    /// Assigned-but-unresolved `wal_seq`s, mapped to why each is unresolved.
    /// A map rather than a set because the stalled-watermark classifier must be
    /// able to tell a boot-seeded sequence from a leaked one.
    pending: BTreeMap<u64, PendingOrigin>,
    /// Highest `wal_seq` ever assigned to this partition. Monotonic: a rolled-back
    /// sequence is NOT restored, which is safe because a rolled-back sequence
    /// names no frame, so a later watermark at that value asserts nothing about it.
    max_assigned: u64,
    /// `wal_seq`s currently owned by a flush worker — removed from the queue and
    /// not yet terminal. Without it a hung inner store reads as a code bug: the
    /// entry exists only as a local binding for the whole of the inner call.
    in_flight: BTreeSet<u64>,
    /// Whether this partition's pending map has been seeded from the on-disk WAL.
    ///
    /// Recovery replays frames DIRECTLY into the inner store, bypassing
    /// write-behind, so a fresh map knows nothing about the frames a prior
    /// incarnation left un-applied. Until the seed lands, "nothing pending" would
    /// mean "nothing this process assigned", not "every frame applied" — and
    /// answering the watermark from it would mark a prior incarnation's
    /// un-replayed frames applied.
    seeded: bool,
}

/// Why a partition's applied watermark could not be computed.
///
/// Typed rather than a silent fallback: the one state in which the watermark is
/// unknowable — an empty pending map that has not been seeded from the WAL — is
/// exactly the state in which a guessed answer marks a prior incarnation's
/// frames applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum WalWatermarkError {
    /// The pending map is empty only because boot seeding has not run yet, so
    /// emptiness carries no information about what a prior incarnation left
    /// behind. Reachable only with a WAL present: with no WAL there is no
    /// tracker, no frames, and nothing to seed.
    #[error("WAL watermark queried before boot seeding completed")]
    Unseeded,
}

/// The WAL handle bundled with all of its `wal_seq` tracking state.
///
/// Bundling is what makes a tracker without a WAL UNREPRESENTABLE. With no WAL
/// no frame is ever written, so the applied-watermark invariant has no subject —
/// nothing to filter on replay, nothing to GC, and no watermark to compute. Held
/// as two separate fields, that would be a branch every call site had to
/// remember; held as one struct, it falls out of the type.
pub(crate) struct WalTracking {
    /// The WAL every write is appended to before ack.
    wal: Arc<dyn Wal>,
    /// Per-partition tracking state, lazily created on a partition's first assign.
    partitions: DashMap<u32, Arc<Mutex<PartitionTracking>>>,
}

impl WalTracking {
    fn new(wal: Arc<dyn Wal>) -> Self {
        Self {
            wal,
            partitions: DashMap::new(),
        }
    }

    /// The tracking cell for `partition`, cloned out so the `DashMap` shard guard
    /// is released before the per-partition mutex is taken.
    fn cell(&self, partition: u32) -> Arc<Mutex<PartitionTracking>> {
        if let Some(existing) = self.partitions.get(&partition) {
            return Arc::clone(existing.value());
        }
        Arc::clone(self.partitions.entry(partition).or_default().value())
    }

    /// Runs `f` under `partition`'s tracking lock.
    ///
    /// The guard cannot escape the closure and the closure is synchronous, so the
    /// lock is structurally incapable of being held across an `.await` — which is
    /// the one discipline the compiler does not otherwise enforce here, since
    /// nesting this mutex under a `DashMap` guard compiles cleanly.
    fn with_partition<R>(&self, partition: u32, f: impl FnOnce(&mut PartitionTracking) -> R) -> R {
        let cell = self.cell(partition);
        let mut guard = cell
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        f(&mut guard)
    }
}

/// A background task the write-behind store runs for its whole life.
///
/// These are the tasks whose death the store cannot work around: without the
/// flush task nothing queued ever reaches the inner store, and without the
/// stall watchdog nothing reports a WAL watermark that has stopped moving.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackgroundTask {
    /// The task that moves queued entries to the inner store.
    Flush,
    /// The task that samples each partition's applied watermark for a stall.
    StallWatchdog,
}

impl BackgroundTask {
    /// The task's name as it appears in log fields and metric labels.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Flush => "flush",
            Self::StallWatchdog => "stall_watchdog",
        }
    }
}

impl std::fmt::Display for BackgroundTask {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why the store failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureCause {
    /// A background task ended by a panic (`TG-WB-006`).
    Panicked,
    /// The inner store reported itself broken (`TG-WB-010`).
    StoreBroken(StoreBrokenCause),
}

/// The record of a failed store: a background task that ended by a panic
/// (`TG-WB-006`), or an inner store that reported itself broken (`TG-WB-010`).
///
/// It is an error type so that a shutdown on a failed store can carry it as
/// the source of what it returns: a caller then recognises the failed store by
/// downcasting, not by matching on the text of a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackgroundFailure {
    /// The task that panicked. For a broken store it is `Flush`: the flush
    /// path, which the flush task and the shutdown drain share, is where the
    /// inner store's error was met.
    pub task: BackgroundTask,
    /// The panic payload when it was a string, a fixed placeholder otherwise;
    /// for a broken store, the inner store's error.
    pub message: String,
    /// Why the store failed.
    pub cause: FailureCause,
}

impl BackgroundFailure {
    /// Whether the store failed because its inner store is broken. A method
    /// rather than a match on `cause`, because the server binary cannot name
    /// this module's types and acts on the failure through its methods.
    #[must_use]
    pub fn store_broken(&self) -> bool {
        matches!(self.cause, FailureCause::StoreBroken(_))
    }

    /// The cause as a log field: `panic`, or the broken store's cause.
    #[must_use]
    pub fn cause_label(&self) -> &'static str {
        match self.cause {
            FailureCause::Panicked => "panic",
            FailureCause::StoreBroken(cause) => cause.as_str(),
        }
    }
}

impl std::fmt::Display for BackgroundFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.cause {
            FailureCause::Panicked => write!(
                f,
                "write-behind {} background task panicked: {}",
                self.task, self.message
            ),
            FailureCause::StoreBroken(cause) => write!(
                f,
                "write-behind durable store is broken ({}): {}",
                cause.as_str(),
                self.message
            ),
        }
    }
}

impl std::error::Error for BackgroundFailure {}

/// Where the store is in its life.
///
/// `Failed` is absorbing: a store whose background task panicked stays failed
/// through its shutdown, so nothing that reads the state afterwards can take
/// it for a store that merely stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum Lifecycle {
    /// Accepting writes; both background tasks are expected to be alive.
    Running = 0,
    /// A background task panicked. Writes are refused.
    Failed = 1,
    /// `hard_flush` was entered on a healthy store. Writes are refused.
    ShuttingDown = 2,
}

/// The store's [`Lifecycle`] in one atomic, so that a write entry learns
/// "running, failed or shutting down" from the single load it always made.
struct LifecycleState(AtomicU8);

impl LifecycleState {
    fn new() -> Self {
        Self(AtomicU8::new(Lifecycle::Running as u8))
    }

    fn load(&self, order: Ordering) -> Lifecycle {
        match self.0.load(order) {
            state if state == Lifecycle::Running as u8 => Lifecycle::Running,
            state if state == Lifecycle::ShuttingDown as u8 => Lifecycle::ShuttingDown,
            // Only the three values above are ever stored; reading anything
            // else as failed keeps an impossible value on the refusing side.
            _ => Lifecycle::Failed,
        }
    }

    /// A background task panicked or the inner store reported itself broken.
    /// Unconditional, because the store is failed from then on whatever it was
    /// doing, a shutdown included.
    fn fail(&self) {
        self.0.store(Lifecycle::Failed as u8, Ordering::Release);
    }

    /// `hard_flush` was entered. A transition from `Running` only: a failed
    /// store stays failed, and a second call changes nothing.
    fn begin_shutdown(&self) {
        let _ = self.0.compare_exchange(
            Lifecycle::Running as u8,
            Lifecycle::ShuttingDown as u8,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
}

/// Counts background tasks that ended by a panic, by `task` label.
const BACKGROUND_TASK_PANICS_COUNTER: &str = "topgun_write_behind_background_task_panics_total";

/// Registers the panic counter with both of its series at zero.
///
/// A series that first appears when it is incremented cannot be told from a
/// series that was never exported, so an alert on it would have nothing to
/// compare against until the failure it is there to report.
fn register_background_task_panic_metrics() {
    metrics::describe_counter!(
        BACKGROUND_TASK_PANICS_COUNTER,
        "Write-behind background tasks that ended by a panic, labeled by the task"
    );
    for task in [BackgroundTask::Flush, BackgroundTask::StallWatchdog] {
        metrics::counter!(BACKGROUND_TASK_PANICS_COUNTER, "task" => task.as_str()).increment(0);
    }
}

/// Counts stores whose inner store reported itself broken, by `cause` label.
const STORE_BROKEN_COUNTER: &str = "topgun_write_behind_store_broken_total";

/// Registers the broken-store counter with all four of its series at zero,
/// for the reason the panic counter is registered at zero.
fn register_store_broken_metrics() {
    metrics::describe_counter!(
        STORE_BROKEN_COUNTER,
        "Write-behind stores whose inner store reported itself broken, labeled by the cause"
    );
    // The `io` label does not carry the kind, so any kind registers it.
    for cause in [
        StoreBrokenCause::Io(std::io::ErrorKind::Other),
        StoreBrokenCause::PreviousIo,
        StoreBrokenCause::Corrupted,
        StoreBrokenCause::LockPoisoned,
    ] {
        metrics::counter!(STORE_BROKEN_COUNTER, "cause" => cause.as_str()).increment(0);
    }
}

/// Counts store calls of the flush task that returned an error.
const FLUSH_REFUSALS_COUNTER: &str = "topgun_write_behind_flush_refusals_total";
/// Buffered entries the store has refused at least once, as of the last pass.
const RETRYING_ENTRIES_GAUGE: &str = "topgun_write_behind_retrying_entries";
/// The buffered-key count the capacity check uses.
const BUFFERED_ENTRIES_GAUGE: &str = "topgun_write_behind_buffered_entries";
/// Seconds since the store first refused the entry it has refused longest.
const OLDEST_REFUSAL_AGE_GAUGE: &str = "topgun_write_behind_oldest_refusal_age_seconds";
/// 1 while a store-wide hold is in force.
const STORE_HOLD_GAUGE: &str = "topgun_write_behind_store_hold";

/// Registers the retry series at zero.
///
/// They render from the first scrape for the same reason as the panic
/// counter: a refusal counter or a hold gauge that first appears during an
/// outage gives an alert nothing to compare against before it.
fn register_retry_metrics() {
    metrics::describe_counter!(
        FLUSH_REFUSALS_COUNTER,
        "Store calls made by the write-behind flush task that returned an error"
    );
    metrics::describe_gauge!(
        RETRYING_ENTRIES_GAUGE,
        "Buffered write-behind entries the store has refused at least once, as of the last \
         pass. It is not the size of the buffer: while the store refuses everything a hold \
         allows one call per period, so this number is about the number of store calls made \
         so far"
    );
    metrics::describe_gauge!(
        BUFFERED_ENTRIES_GAUGE,
        "Buffered write-behind keys, the count the capacity check uses, as of the last pass; \
         writes of new keys are refused once it reaches the capacity"
    );
    metrics::describe_gauge!(
        OLDEST_REFUSAL_AGE_GAUGE,
        "Seconds the store has been refusing the buffered entry it has refused longest, 0 when \
         none is refused. The node-wide flushed watermark, and with it tombstone pruning on \
         every map of the node, has stood for at least this long"
    );
    metrics::describe_gauge!(
        STORE_HOLD_GAUGE,
        "1 while a store-wide hold is in force, from the pass that started it until a probe is \
         accepted, else 0"
    );
    metrics::counter!(FLUSH_REFUSALS_COUNTER).increment(0);
    metrics::gauge!(RETRYING_ENTRIES_GAUGE).set(0.0);
    metrics::gauge!(BUFFERED_ENTRIES_GAUGE).set(0.0);
    metrics::gauge!(OLDEST_REFUSAL_AGE_GAUGE).set(0.0);
    metrics::gauge!(STORE_HOLD_GAUGE).set(0.0);
}

/// A count as a gauge value. A count above `u32::MAX` reads as that: every
/// `u32` is exact as an `f64`, and a gauge that large is not read digit by
/// digit.
fn count_as_gauge(count: u64) -> f64 {
    f64::from(u32::try_from(count).unwrap_or(u32::MAX))
}

/// The text of a panic payload: the message when the panic carried a string,
/// which is what `panic!` produces, and a fixed placeholder otherwise.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_string()
    }
}

/// The body of a background task, run so that a panic in it is seen by the
/// store (`TG-WB-006`).
///
/// The unwind is caught inside `poll`, on the dying task and before that poll
/// returns to the runtime, and the failure is recorded right there. Awaiting
/// the task's `JoinHandle` from another task would learn of the panic only
/// when the runtime next ran that other task, and every write accepted in
/// between would be queued behind a flush task that no longer exists.
///
/// A task that is aborted is dropped, not unwound, so an abort records
/// nothing; neither does a body that returns.
struct ObservedTask<F> {
    store: Arc<WriteBehindDataStore>,
    task: BackgroundTask,
    /// Boxed so the wrapper is `Unpin` and needs no pin projection.
    body: std::pin::Pin<Box<F>>,
}

impl<F> std::future::Future for ObservedTask<F>
where
    F: std::future::Future<Output = ()>,
{
    type Output = ();

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<()> {
        let this = &mut *self;
        let body = this.body.as_mut();
        // The body is never polled again after it unwound: the task ends here,
        // so no state the panic left half-updated inside it is observed.
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| body.poll(cx))) {
            Ok(poll) => poll,
            Err(payload) => {
                this.store
                    .record_background_failure(this.task, panic_message(payload.as_ref()));
                std::task::Poll::Ready(())
            }
        }
    }
}

/// Write-behind buffering layer that wraps any [`MapDataStore`].
///
/// Buffers `add`/`remove` calls in per-partition coalesced queues and flushes
/// them to the inner store on a configurable schedule. Reads check the staging
/// area first to provide read-your-writes consistency.
///
/// When a WAL is provided, every write is appended to the WAL and satisfies
/// the configured fsync policy **before** the method returns `Ok(())` so that
/// an acked write is durable even if the process crashes before the background
/// flush loop persists it to the inner store.
pub struct WriteBehindDataStore {
    /// The wrapped persistence store.
    inner: Arc<dyn MapDataStore>,
    /// Configuration controlling flush timing, retries, and capacity.
    config: WriteBehindConfig,
    /// Per-partition write queues, keyed by partition id.
    queues: DashMap<u32, PartitionQueue>,
    /// Staging area for read-your-writes consistency.
    /// `value`: `Some(value)` = pending write, `None` = pending delete.
    staging: DashMap<(String, String), StagingSlot>,
    /// Monotonically increasing operation counter for ordering.
    sequence: AtomicU64,
    /// Node-wide count of pending entries across all partition queues.
    pending_count: AtomicU64,
    /// Signal to wake the flush loop for immediate processing.
    flush_notify: Arc<Notify>,
    /// Shutdown signal sender — wakes the background flush task and tells it to exit.
    shutdown: watch::Sender<bool>,
    /// Running, failed or shutting down. A write entry loads it once and
    /// refuses in the last two, so a straggler write is rejected loudly rather
    /// than silently lost: after a shutdown began nothing would drain it, and
    /// after a background task panicked nothing would flush it.
    lifecycle: LifecycleState,
    /// The first failure — a background-task panic or a broken inner store —
    /// `None` until there is one. A `watch` channel so that whoever owns the
    /// process can wait for the failure instead of polling the store for it.
    background_failure: watch::Sender<Option<BackgroundFailure>>,
    /// Set by the first broken-store record, whatever the failure record
    /// holds, so that the drain meeting the same broken store after the flush
    /// task adds no second line and no second count.
    store_broken_seen: AtomicBool,
    /// Optional WAL for append-before-ack durability, bundled with its
    /// per-partition sequence tracking. When `Some`, every write is persisted to
    /// the WAL per the configured fsync policy before the store method returns,
    /// closing the unclean-crash loss window. When `None` no frame is ever
    /// written, so there is no tracking to do and none exists.
    wal: Option<WalTracking>,
    /// WAL sequence counter, monotonically increasing per-entry for ordering.
    wal_sequence: AtomicU64,
    /// Entry sequences ASSIGNED but not yet RESOLVED — a write is resolved when
    /// its bytes reach the inner store (flushed) OR a later write to the same key
    /// coalesces it away (retired, its data carried forward by the survivor).
    ///
    /// The smallest still-pending sequence is the PREFIX-COMPLETE flushed
    /// watermark (see [`flushed_watermark`](MapDataStore::flushed_watermark)):
    /// every sequence below it is resolved, with no mid-range hole. This is the
    /// real byte-durability signal the tombstone frontier fences the prune on —
    /// NOT the last-assigned counter, which advances at enqueue and would let a
    /// prune drop a tombstone still buffered in RAM.
    pending_seqs: Mutex<BTreeSet<u64>>,
    /// Cross-tick state of the stalled-watermark watchdog: how long each
    /// partition's smallest pending sequence has held still, and which sequence
    /// is one confirming sample away from a `TrackerLeak` verdict.
    stall_watch: Mutex<StallWatch>,
    /// The background flush loop's task, taken by the first `hard_flush`, which
    /// waits for the loop to finish its in-flight batch before draining, so no
    /// key's writes reach the inner store from two tasks at once.
    flush_loop: Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Which rule the per-partition applied watermark is computed by. Production
    /// always uses the prefix-complete rule; the scalar-max rule exists only so
    /// the differential test keeps a live negative control.
    #[cfg(test)]
    watermark_mode: Mutex<WatermarkMode>,
    /// Test-only instrumentation of the stalled-pending classifier.
    #[cfg(test)]
    classifier_seam: ClassifierSeam,
    /// Forces every coalesce survivor to answer "does not subsume".
    ///
    /// A production framing now answers `false` — the delta frame this store
    /// emits under an armed `or_delta_wal` carries only its own mutation — so
    /// the carry-forward route is reachable without this seam. It stays because
    /// reaching it that way depends on which ops a run happens to coalesce,
    /// while this forces the shape deterministically: the abandoned-survivor
    /// stall that keeps a retired frame replayable is exercised on every run
    /// rather than on the runs that happen to produce the right interleaving.
    #[cfg(test)]
    force_non_subsuming_survivor: AtomicBool,
    /// Selects whether boot seeding populates a partition's pending map from
    /// `wal.unapplied(p)` (`Seeded`, production) or skips it (`Empty`, the C12
    /// re-introduction the crash/recovery harness drives). Production has no
    /// field and no branch for this: with no seam to read, boot seeding always
    /// takes the `Seeded` path.
    #[cfg(test)]
    boot_seed_mode: Mutex<BootSeedMode>,
    /// Reports the WAL sequence a mutation was just assigned, at append time only.
    ///
    /// This transfers an IDENTIFIER (the sequence number the implementation
    /// assigned), never a lifecycle: the crash/recovery harness's reference model
    /// learns what a write is *called* so it can name it in its own event log,
    /// while every `Appended -> Acked -> DurablyApplied` transition is decided by
    /// the model alone. Fired synchronously inside `assign_wal_sequence`, the one
    /// chokepoint every mutating path funnels a sequence assignment through, so a
    /// single observation covers `add`, `remove`, and each key of `remove_all`.
    #[cfg(test)]
    #[allow(clippy::type_complexity)]
    append_observer: Mutex<Option<Arc<dyn Fn(u32, u64) + Send + Sync>>>,
    /// One-shot parks on `load_slot`, `remove` and `hard_flush`.
    #[cfg(test)]
    test_parks: TestParks,
    /// Bumped when the flush loop opens a pass and again when it closes one, so
    /// it is odd exactly while the loop may hold a drained batch. "Every queue
    /// empty" alone cannot tell a test that nothing is outstanding: a drained
    /// batch is in no queue.
    #[cfg(test)]
    flush_pass_epoch: AtomicU64,
    /// Whether a store-wide hold is in force. The flush task owns the hold;
    /// this mirrors it for readers outside that task.
    store_hold: AtomicBool,
    /// Probes refused in a row in the current hold, mirrored like the hold.
    failed_probes: AtomicU32,
    /// Set by a refusal and cleared by the walk that finds no refused entry,
    /// so a pass on a store that refuses nothing pays one load and no walk.
    retrying: AtomicBool,
}

/// WAL plus the live sequence counter's starting value, threaded into
/// `WriteBehindDataStore::new_with_wal`.
///
/// `sequence_start` must be `max_observed_sequence() + 1` computed *after*
/// recovery so the live counter never re-hands a number at or below any
/// persisted log sequence or `.applied` watermark — reusing such a number lets
/// the recovery filter (`e.sequence > applied_seq`) silently drop a post-restart
/// acked write. A fresh node passes `1` (sequence `0` is the reserved
/// "nothing applied" sentinel and is never handed out).
pub struct WalBootstrap {
    /// The WAL implementation every write is appended to before ack.
    pub wal: Arc<dyn Wal>,
    /// First sequence number the live counter hands out (1-based).
    pub sequence_start: u64,
}

impl WriteBehindDataStore {
    /// Creates a new write-behind data store wrapping `inner`.
    ///
    /// Spawns a background flush task that runs until the returned `Arc` (and
    /// all clones) are dropped or shutdown is signalled.
    pub fn new(inner: Arc<dyn MapDataStore>, config: WriteBehindConfig) -> Arc<Self> {
        Self::new_with_wal(inner, config, None)
    }

    /// Creates a new write-behind data store with an optional WAL bootstrap.
    ///
    /// When `bootstrap` is `Some`, every write is appended to the WAL and
    /// satisfies the configured fsync policy before returning `Ok(())`, and the
    /// live sequence counter starts at `bootstrap.sequence_start`. When `None`,
    /// the store behaves identically to `new` (no crash-safe durability
    /// guarantee) and the counter starts at `1` so sequence `0` is never handed
    /// out.
    pub fn new_with_wal(
        inner: Arc<dyn MapDataStore>,
        config: WriteBehindConfig,
        bootstrap: Option<WalBootstrap>,
    ) -> Arc<Self> {
        // A build that can refuse store writes on demand must say so at boot,
        // whether or not the seam is configured: such a binary must never be
        // mistaken for a production one.
        #[cfg(feature = "fault-injection")]
        if let Some(path) = fault_store_refuse_path() {
            warn!(
                refuse_file = %path.display(),
                "fault-injection build: store writes are refused while the sentinel file exists"
            );
        } else {
            warn!(
                "fault-injection build: no sentinel file is configured, store writes are never refused"
            );
        }
        // The second seam is announced on a line of its own, after the first,
        // for the same reason and likewise whether or not it is configured.
        #[cfg(feature = "fault-injection")]
        if let Some(path) = fault_store_panic_path() {
            warn!(
                env = FAULT_STORE_PANIC_FILE_ENV,
                panic_file = %path.display(),
                "fault-injection build: persisting an entry panics while the panic sentinel file exists"
            );
        } else {
            warn!(
                env = FAULT_STORE_PANIC_FILE_ENV,
                "fault-injection build: no panic sentinel file is configured, persisting an entry never panics"
            );
        }

        register_background_task_panic_metrics();
        register_store_broken_metrics();
        register_retry_metrics();

        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let flush_notify = Arc::new(Notify::new());

        let wal = bootstrap
            .as_ref()
            .map(|b| WalTracking::new(Arc::clone(&b.wal)));
        // Seed the live counter from the WAL watermark so post-restart writes
        // never reuse a sequence at or below a persisted watermark (which the
        // recovery filter `e.sequence > applied_seq` would silently drop). A
        // missing bootstrap seeds to 1: sequence 0 is the reserved sentinel.
        let wal_sequence_start = bootstrap.map_or(1, |b| b.sequence_start);

        let store = Arc::new(Self {
            inner,
            config,
            queues: DashMap::new(),
            staging: DashMap::new(),
            sequence: AtomicU64::new(0),
            pending_count: AtomicU64::new(0),
            flush_notify: flush_notify.clone(),
            shutdown: shutdown_tx,
            lifecycle: LifecycleState::new(),
            background_failure: watch::channel(None).0,
            store_broken_seen: AtomicBool::new(false),
            wal,
            wal_sequence: AtomicU64::new(wal_sequence_start),
            pending_seqs: Mutex::new(BTreeSet::new()),
            stall_watch: Mutex::new(StallWatch::default()),
            flush_loop: Mutex::new(None),
            #[cfg(test)]
            watermark_mode: Mutex::new(WatermarkMode::default()),
            #[cfg(test)]
            classifier_seam: ClassifierSeam::default(),
            #[cfg(test)]
            force_non_subsuming_survivor: AtomicBool::new(false),
            #[cfg(test)]
            boot_seed_mode: Mutex::new(BootSeedMode::default()),
            #[cfg(test)]
            append_observer: Mutex::new(None),
            #[cfg(test)]
            test_parks: TestParks::default(),
            #[cfg(test)]
            flush_pass_epoch: AtomicU64::new(0),
            store_hold: AtomicBool::new(false),
            failed_probes: AtomicU32::new(0),
            retrying: AtomicBool::new(false),
        });

        // Spawn background flush loop with a clone of the Arc
        let store_clone = Arc::clone(&store);
        let flush_task = Self::spawn_background_task(
            &store,
            BackgroundTask::Flush,
            flush_loop(store_clone, shutdown_rx),
        );
        *store
            .flush_loop
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(flush_task);

        // The stall watchdog runs on its own schedule, not the flush loop's: the
        // flush loop is exactly what stops making progress in the states the
        // watchdog exists to report, so a probe folded into it would go quiet
        // precisely when it is needed.
        let watchdog_shutdown = store.shutdown.subscribe();
        let watchdog_store = Arc::clone(&store);
        // Under the same policy as the flush task: the watchdog reads the state
        // the flush path changes, so its panic is a sign of a defect in that
        // state, and a store that ran on without it would have lost its only
        // stall detector without saying so.
        Self::spawn_background_task(
            &store,
            BackgroundTask::StallWatchdog,
            watchdog_loop(watchdog_store, watchdog_shutdown),
        );

        store
    }

    /// Starts a background task. Every one of them starts here, so none can
    /// run without its panic being recorded.
    fn spawn_background_task<F>(
        store: &Arc<Self>,
        task: BackgroundTask,
        body: F,
    ) -> tokio::task::JoinHandle<()>
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        tokio::spawn(ObservedTask {
            store: Arc::clone(store),
            task,
            body: Box::pin(body),
        })
    }

    /// Fails the store because `task` panicked. Runs on the dying task.
    ///
    /// Synchronous on purpose, and the state is stored first: once the poll of
    /// the panicked task has returned, the next write already finds a failed
    /// store. Were the state stored after a suspension point, or after the
    /// signal that makes the owner of the process react, a write could be
    /// accepted by a store already known to be dead.
    ///
    /// The record keeps the first failure. A second task that panics is still
    /// logged and counted, because it is a second defect.
    fn record_background_failure(&self, task: BackgroundTask, message: String) {
        self.lifecycle.fail();
        error!(
            task = task.as_str(),
            panic = %message,
            "write-behind background task panicked; the store refuses writes from now on"
        );
        metrics::counter!(BACKGROUND_TASK_PANICS_COUNTER, "task" => task.as_str()).increment(1);
        // `send_if_modified` and not `send`: `send` stores nothing when no
        // receiver exists at that moment, and the record has to be there for a
        // receiver taken later.
        self.background_failure.send_if_modified(|recorded| {
            if recorded.is_some() {
                return false;
            }
            *recorded = Some(BackgroundFailure {
                task,
                message,
                cause: FailureCause::Panicked,
            });
            true
        });
    }

    /// The failure of a background task, `None` while both are alive.
    ///
    /// The value is set once, by the task that panicked, and never replaced or
    /// cleared: a second failure leaves the first in place, and a shutdown
    /// does not erase it. A receiver taken after the failure sees it as its
    /// current value, so a caller that waits must look at the value before it
    /// waits for a change (`wait_for` does).
    #[must_use]
    pub fn background_failure(&self) -> watch::Receiver<Option<BackgroundFailure>> {
        self.background_failure.subscribe()
    }

    /// Fails the store because its inner store reported itself broken
    /// (`TG-WB-010`). Once per process: a later call does nothing.
    ///
    /// Synchronous, like the panic record. The record is set before the state,
    /// so that a write refused for the failed state finds the record when it
    /// chooses its sentence; both precede the line. The record is set only if
    /// empty: a panic recorded earlier stays the first failure.
    fn record_store_broken(
        &self,
        cause: StoreBrokenCause,
        err: &anyhow::Error,
        map: &str,
        key: &str,
        site: &'static str,
    ) {
        if self.store_broken_seen.swap(true, Ordering::AcqRel) {
            return;
        }
        let message = err.to_string();
        self.background_failure.send_if_modified(|recorded| {
            if recorded.is_some() {
                return false;
            }
            *recorded = Some(BackgroundFailure {
                task: BackgroundTask::Flush,
                message: message.clone(),
                cause: FailureCause::StoreBroken(cause),
            });
            true
        });
        self.lifecycle.fail();
        let io_kind = match cause {
            StoreBrokenCause::Io(kind) => Some(format!("{kind:?}")),
            _ => None,
        };
        error!(
            cause = cause.as_str(),
            io_kind = io_kind.as_deref(),
            error = %message,
            map,
            key,
            site,
            "write-behind store: the durable store is broken and stores nothing more in this \
             process; writes are refused from now on and the node stops"
        );
        metrics::counter!(STORE_BROKEN_COUNTER, "cause" => cause.as_str()).increment(1);
    }

    /// Why a failed store refuses a write, as the refusal sentence names it.
    /// Read only when a write is refused for the failed state, so an accepted
    /// write pays nothing for it.
    fn failed_reason(&self) -> &'static str {
        match self.background_failure.borrow().as_ref() {
            Some(failure) if failure.store_broken() => "the durable store is broken",
            _ => "background task panicked",
        }
    }

    /// The shutdown drain: persists every queued entry to the inner store, or
    /// gives up on what is left when `deadline` passes.
    ///
    /// Returns how many entries it took from the queues, persisted or not. It
    /// ends when the queues are empty and never looks at `pending_count`: on a
    /// store whose flush task died holding a batch that count stays above zero
    /// for good, and a drain that waited for it would never return.
    ///
    /// Only what is queued is touched. The entries a dead flush task held are
    /// in no queue; their WAL sequences stay un-resolved, which holds the
    /// applied watermark below their frames so the next start replays them
    /// (`TG-WB-006`).
    async fn drain_for_shutdown(&self, deadline: tokio::time::Instant) -> usize {
        // Flush all partition queues directly to the inner store. Using i64::MAX
        // as the drain deadline ensures every buffered entry is eligible regardless
        // of its write_delay_ms schedule — on shutdown we want everything, not just
        // entries past their delay window.
        let mut drained_entries = 0usize;
        loop {
            // Collect all entries still in the queues.
            let mut all_entries: Vec<DelayedEntry> = Vec::new();
            for mut queue_ref in self.queues.iter_mut() {
                let drained = queue_ref.value_mut().drain_ready(i64::MAX);
                all_entries.extend(drained);
            }
            drained_entries += all_entries.len();

            if all_entries.is_empty() {
                return drained_entries;
            }

            // Same window as the flush loop's drain: from here the entries exist
            // only as local bindings, so record ownership before the first
            // inner-store call and after every queue guard has dropped.
            for entry in &all_entries {
                self.register_in_flight(
                    partition_for(&entry.map, &entry.key),
                    &entry.wal_sequences,
                );
            }

            // Persist each entry, wrapping each inner-store call with the
            // remaining deadline budget. This ensures that even a single slow
            // inner-store write cannot hold up the process past the configured
            // timeout.
            let mut timed_out_entries: Vec<DelayedEntry> = Vec::new();
            let mut broken: Option<StoreBrokenCause> = None;

            let mut entries = all_entries.into_iter();
            while let Some(entry) = entries.next() {
                let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                if remaining.is_zero() {
                    // Budget exhausted — collect this entry as timed-out.
                    timed_out_entries.push(entry);
                    continue;
                }

                let flush_future = persist_entry(self.inner.as_ref(), &entry);

                let partition_id_for_wal = partition_for(&entry.map, &entry.key);
                match tokio::time::timeout(remaining, flush_future).await {
                    Ok(Ok(())) => {
                        // A writer that passed the lifecycle check before the
                        // shutdown may have staged a newer write of this key —
                        // its entry already queued (it completed while this
                        // drain was inside the store call above) or not queued
                        // yet. Removing the slot without comparing sequences
                        // would leave that write buffered with no slot, and
                        // reads would fall back to the older durable value
                        // (`TG-WB-007`).
                        self.clear_staging_if_current(&entry.map, &entry.key, entry.sequence);
                        self.pending_count.fetch_sub(1, Ordering::Relaxed);
                        self.resolve_pending(entry.sequence);
                        // Resolve and advance so a restart after clean shutdown is
                        // a no-op rather than re-replaying already-durable writes.
                        self.resolve_and_advance(partition_id_for_wal, &entry.wal_sequences)
                            .await;
                    }
                    Ok(Err(err)) => {
                        if let StoreErrorClass::StoreBroken(cause) = self.inner.classify_error(&err)
                        {
                            // A broken store stores nothing more: no further
                            // call. This entry and every one not attempted are
                            // abandoned, never resolved, so their frames replay
                            // at the next start.
                            self.record_store_broken(
                                cause,
                                &err,
                                &entry.map,
                                &entry.key,
                                "shutdown_drain",
                            );
                            broken = Some(cause);
                            timed_out_entries.push(entry);
                            timed_out_entries.extend(entries.by_ref());
                            break;
                        }
                        // Inner store returned an error; log and treat as timed-out
                        // rather than looping forever on a failing backend.
                        warn!(
                            map = %entry.map,
                            key = %entry.key,
                            error = %err,
                            "Inner store error during shutdown drain; entry not persisted"
                        );
                        timed_out_entries.push(entry);
                    }
                    Err(_elapsed) => {
                        // Deadline expired mid-write.
                        timed_out_entries.push(entry);
                    }
                }
            }

            if !timed_out_entries.is_empty() {
                // Also sweep any entries reinserted by concurrent retry paths.
                // These are drained and abandoned in one go — deliberately never
                // registered, since they never enter a flush and an ownership
                // that is taken and released with no call in between is a state
                // no observer can see.
                for mut queue_ref in self.queues.iter_mut() {
                    let drained = queue_ref.value_mut().drain_ready(i64::MAX);
                    drained_entries += drained.len();
                    timed_out_entries.extend(drained);
                }

                // The process is exiting with these writes in neither the inner
                // store nor applied in the WAL. Abandoning rather than resolving
                // holds the watermark below their frames, so recovery replays
                // them on the next boot.
                for entry in &timed_out_entries {
                    self.abandon_wal_sequences(
                        partition_for(&entry.map, &entry.key),
                        &entry.wal_sequences,
                    );
                }

                if let Some(cause) = broken {
                    error!(
                        abandoned_entries = timed_out_entries.len(),
                        cause = cause.as_str(),
                        "write-behind shutdown drain stopped at a broken store: the entries it \
                         did not store stay un-resolved and replay from the WAL at the next start"
                    );
                }

                // Log every op that could not be persisted within the timeout.
                for entry in &timed_out_entries {
                    warn!(
                        map = %entry.map,
                        key = %entry.key,
                        timeout_ms = self.config.shutdown_timeout_ms,
                        "Shutdown drain timed out; write-behind entry could not be persisted"
                    );
                }
                return drained_entries;
            }

            // Yield briefly between iterations to avoid starving the tokio runtime.
            tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;
        }
    }

    /// The one way out of `hard_flush`: makes the WAL durable, then says
    /// whether the store that was shut down had failed.
    ///
    /// The sync comes first and on every shutdown, not only a failed one. Under
    /// a deferred fsync policy the frames of the last moments are in the file
    /// but not fsynced, and the task that would fsync them ends with the
    /// process. Its error does not replace the result: the caller has to learn
    /// about the store either way.
    ///
    /// The failure is read here, after everything else, because a task can
    /// panic at any moment of the shutdown — while the flush task was being
    /// waited for, or during the drain. Read at the entry, such a failure
    /// would end in `Ok(())` and a node reporting a clean stop.
    async fn finish_shutdown(&self, drained_entries: usize) -> anyhow::Result<()> {
        if let Some(tracking) = &self.wal {
            if let Err(err) = tracking.wal.sync().await {
                error!(
                    error = %err,
                    "WAL sync at write-behind shutdown failed; frames appended since the last \
                     fsync may not be durable"
                );
            }
        }

        let failure = self.background_failure.borrow().clone();
        let Some(failure) = failure else {
            return Ok(());
        };
        // `pending_count` here is the dead flush task's batch plus whatever
        // this drain timed out or failed on; it is not the size of the batch.
        error!(
            task = failure.task.as_str(),
            unpersisted_entries = self.pending_count.load(Ordering::Relaxed),
            drained_entries,
            "write-behind shutdown on a failed store: entries not persisted here stay \
             un-resolved and replay from the WAL at the next start when the store has one"
        );
        let context = if failure.store_broken() {
            "write-behind shutdown ran on a failed store: the durable store is broken"
        } else {
            "write-behind shutdown ran on a failed store: a background task panicked"
        };
        Err(anyhow::Error::new(failure).context(context))
    }

    /// Assign the next ordering sequence AND record it pending byte-durability in
    /// one atomic step, under the `pending_seqs` lock. Folding the `fetch_add` and
    /// the set insert under the same lock `flushed_watermark()` reads is what makes
    /// the watermark prefix-complete under concurrency: a separate bump-then-track
    /// leaves a window where the counter is already incremented but the set is
    /// still empty, during which `flushed_watermark()`'s empty-set branch would
    /// return `sequence.load()` — a value ABOVE the just-assigned, still-buffered
    /// sequence (a mid-range hole that could license pruning a non-durable
    /// tombstone). Returns the assigned sequence.
    fn assign_tracked_sequence(&self) -> u64 {
        let mut pending = self.pending_seqs();
        let seq = self.sequence.fetch_add(1, Ordering::Relaxed);
        pending.insert(seq);
        seq
    }

    /// Lock the pending-sequence set, recovering from a poisoned mutex (a prior
    /// panic while holding it leaves a consistent set — a stale entry only holds
    /// the watermark back, the safe direction).
    /// Waits, until `deadline`, for the flush loop to finish the batch it may be
    /// applying. It checks the shutdown signal only between ticks, and draining
    /// while it is mid-batch would apply one key's in-flight write and a later
    /// write of the same key from two tasks, in either order. On timeout the loop
    /// is aborted; the entries of its aborted batch are neither resolved nor
    /// abandoned, so with a WAL they replay on the next boot. A later call finds
    /// the handle already taken and returns at once.
    async fn join_flush_loop(&self, deadline: tokio::time::Instant) {
        let flush_task = self
            .flush_loop
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let Some(mut flush_task) = flush_task else {
            return;
        };
        if tokio::time::timeout_at(deadline, &mut flush_task)
            .await
            .is_err()
        {
            flush_task.abort();
            warn!(
                shutdown_timeout_ms = self.config.shutdown_timeout_ms,
                "write-behind flush loop did not finish its in-flight batch within the \
                 shutdown timeout; aborted it (its entries replay from the WAL on the \
                 next boot when the store has one)"
            );
        }
    }

    fn pending_seqs(&self) -> std::sync::MutexGuard<'_, BTreeSet<u64>> {
        self.pending_seqs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Record `seq` as enqueued-but-not-yet-durable. Test-only injector for
    /// exercising the watermark deterministically; production assigns and tracks
    /// atomically via [`assign_tracked_sequence`](Self::assign_tracked_sequence).
    #[cfg(test)]
    fn track_pending(&self, seq: u64) {
        self.pending_seqs().insert(seq);
    }

    /// The cell of the queued in-place write for `(map, key)`, if the key's
    /// queued entry is one. Entries already dequeued by a flush are not seen.
    #[cfg(test)]
    pub(crate) fn test_pending_cell(&self, map: &str, key: &str) -> Option<Arc<SlotCell>> {
        let queue = self.queues.get(&partition_for(map, key))?;
        match &queue
            .entries
            .get(&(map.to_string(), key.to_string()))?
            .operation
        {
            DelayedOp::StoreCell { cell, .. } => Some(Arc::clone(cell)),
            DelayedOp::Store { .. } | DelayedOp::Remove => None,
        }
    }

    /// The cell staged for `(map, key)`, if its staging slot holds one.
    #[cfg(test)]
    pub(crate) fn test_staged_cell(&self, map: &str, key: &str) -> Option<Arc<SlotCell>> {
        let slot = self.staging.get(&(map.to_string(), key.to_string()))?;
        match &slot.value {
            Some(StagedValue::Cell(cell)) => Some(Arc::clone(cell)),
            Some(StagedValue::Value(_)) | None => None,
        }
    }

    /// Parks the next `load_slot` of `(map, key)` on return, but only when it
    /// answers the staged cell: the caller then holds the cell while the test
    /// interleaves a second caller. One-shot; each park counts one hit.
    #[cfg(test)]
    pub(crate) fn test_park_load_slot(&self, map: &str, key: &str) -> TestParkHandle {
        let (gate, handle) = TestParkHandle::arm();
        *self
            .test_parks
            .load_slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some((map.to_string(), key.to_string(), gate));
        handle
    }

    /// How many `load_slot` calls have parked on an armed
    /// [`test_park_load_slot`](Self::test_park_load_slot).
    #[cfg(test)]
    pub(crate) fn test_load_slot_park_hits(&self) -> u64 {
        self.test_parks.load_slot_hits.load(Ordering::Relaxed)
    }

    /// Parks the next `remove` of `(map, key)` once its delete is staged and
    /// before it returns, i.e. between the two steps of a record-store remove.
    #[cfg(test)]
    pub(crate) fn test_park_after_remove(&self, map: &str, key: &str) -> TestParkHandle {
        let (gate, handle) = TestParkHandle::arm();
        *self
            .test_parks
            .after_remove
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some((map.to_string(), key.to_string(), gate));
        handle
    }

    /// Parks the next write of `(map, key)` after its sequence assignment,
    /// before it stages the key and before it takes the queue, so a second
    /// writer of the same key can be staged and queued first with a higher
    /// sequence. One-shot and held by this store instance:
    /// the first writer of the key to reach the point parks, later ones pass.
    #[cfg(test)]
    pub(crate) fn test_park_before_queue_insert(&self, map: &str, key: &str) -> TestParkHandle {
        let (gate, handle) = TestParkHandle::arm();
        *self
            .test_parks
            .before_queue_insert
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some((map.to_string(), key.to_string(), gate));
        handle
    }

    /// Parks the next write of `(map, key)` between the staging of its key and
    /// its queue insert, so a flush pass or a shutdown drain can run while the
    /// write has done one of the two and not the other. One-shot and held by
    /// this store instance: the first write of the key to reach the point
    /// parks, later ones pass.
    #[cfg(test)]
    pub(crate) fn test_park_between_stage_and_queue_insert(
        &self,
        map: &str,
        key: &str,
    ) -> TestParkHandle {
        let (gate, handle) = TestParkHandle::arm();
        *self
            .test_parks
            .between_stage_and_queue_insert
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some((map.to_string(), key.to_string(), gate));
        handle
    }

    /// Parks the next `hard_flush` once the shutdown flag is set and before the
    /// flush loop is joined, so a write can hit the shutdown gate while the
    /// drain has not started.
    #[cfg(test)]
    pub(crate) fn test_park_after_shutdown_flag(&self) -> TestParkHandle {
        let (gate, handle) = TestParkHandle::arm();
        *self
            .test_parks
            .after_shutdown_flag
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(gate);
        handle
    }

    /// Parks the flush task the next time it has chosen a probe, before it
    /// removes the entry from its queue, so a writer can replace the entry in
    /// between. One-shot and held by this store instance.
    #[cfg(test)]
    pub(crate) fn test_park_between_probe_choice_and_removal(&self) -> TestParkHandle {
        let (gate, handle) = TestParkHandle::arm();
        *self
            .test_parks
            .between_probe_choice_and_removal
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(gate);
        handle
    }

    /// Whether a store-wide hold is in force: from the trip until a probe is
    /// accepted, including the stretch after the hold time has passed and
    /// before a probe has been made.
    #[cfg(test)]
    pub(crate) fn test_store_hold_active(&self) -> bool {
        self.store_hold.load(Ordering::SeqCst)
    }

    /// Probes refused in a row in the current hold.
    #[cfg(test)]
    pub(crate) fn test_failed_probes(&self) -> u32 {
        self.failed_probes.load(Ordering::SeqCst)
    }

    /// `(retry_count, next_attempt)` of the entry queued for a key; `None`
    /// when the key has no queued entry.
    #[cfg(test)]
    pub(crate) fn test_retry_state(
        &self,
        map: &str,
        key: &str,
    ) -> Option<(u32, Option<tokio::time::Instant>)> {
        let queue = self.queues.get(&partition_for(map, key))?;
        let entry = queue.entries.get(&(map.to_string(), key.to_string()))?;
        Some((entry.retry_count, entry.next_attempt))
    }

    /// When the store first refused the entry queued for a key; `None` when
    /// the key has no queued entry or the entry was never refused.
    #[cfg(test)]
    pub(crate) fn test_first_refused(&self, map: &str, key: &str) -> Option<tokio::time::Instant> {
        let queue = self.queues.get(&partition_for(map, key))?;
        let entry = queue.entries.get(&(map.to_string(), key.to_string()))?;
        entry.first_refused
    }

    /// How many flush passes have ended. The epoch is bumped once when a pass
    /// opens and once when it closes, so half of it counts the closed ones.
    #[cfg(test)]
    pub(crate) fn test_passes_closed(&self) -> u64 {
        self.flush_pass_epoch.load(Ordering::SeqCst) / 2
    }

    /// Whether a flush pass is running: the epoch is odd while one is open.
    #[cfg(test)]
    pub(crate) fn test_pass_open(&self) -> bool {
        self.flush_pass_epoch.load(Ordering::SeqCst) % 2 == 1
    }

    /// Selects the watermark rule this store computes with.
    ///
    /// Reachable from sibling test modules, which Rust privacy otherwise keeps
    /// out of this store's internals entirely.
    #[cfg(test)]
    pub(crate) fn test_set_watermark_mode(&self, mode: WatermarkMode) {
        *self
            .watermark_mode
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = mode;
    }

    /// The watermark rule currently in force.
    #[cfg(test)]
    pub(crate) fn watermark_mode(&self) -> WatermarkMode {
        *self
            .watermark_mode
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Selects whether the NEXT unseeded partition's boot seeding populates its
    /// pending map from `wal.unapplied(p)` (`Seeded`) or skips it (`Empty`, the
    /// C12 re-introduction). Reachable from sibling test modules the same way
    /// [`Self::test_set_watermark_mode`] is.
    #[cfg(test)]
    pub(crate) fn test_set_boot_seed_mode(&self, mode: BootSeedMode) {
        *self
            .boot_seed_mode
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = mode;
    }

    /// The boot-seed mode currently in force.
    #[cfg(test)]
    fn boot_seed_mode(&self) -> BootSeedMode {
        *self
            .boot_seed_mode
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Installs a hook that runs inside every classifier sample, between the
    /// registry probe and the queue scan.
    ///
    /// That position is the only one where draining a queued sequence produces
    /// the transient ownerless read the two-sample confirmation exists to absorb,
    /// so injecting there makes the property deterministic instead of a race.
    #[cfg(test)]
    pub(crate) fn test_set_classifier_hook(&self, hook: Arc<dyn Fn(u32) + Send + Sync>) {
        *self
            .classifier_seam
            .mid_sample
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(hook);
    }

    /// Number of classifier samples taken for `partition` so far.
    #[cfg(test)]
    pub(crate) fn test_classifier_sample_count(&self, partition: u32) -> u64 {
        self.classifier_seam.sample_count(partition)
    }

    /// Drives ONE classifier sample for `partition` on demand and returns the
    /// alarm it classifies, if any.
    ///
    /// Tests drive the sample rather than waiting on the watchdog's interval, so
    /// no scenario is written as a timing race or has to wait out the stall bound.
    #[cfg(test)]
    pub(crate) fn test_run_classifier_sample(&self, partition: u32) -> Option<WalWatermarkAlarm> {
        match self.classifier_sample(partition) {
            SampleOutcome::Alarm { alarm, .. } => Some(alarm),
            SampleOutcome::Healthy | SampleOutcome::LeakCandidate { .. } => None,
        }
    }

    /// Lock the watchdog's cross-tick state, recovering from a poisoned mutex —
    /// the state is observational, so a stale entry costs at most one delayed or
    /// one repeated verdict.
    fn stall_watch(&self) -> std::sync::MutexGuard<'_, StallWatch> {
        self.stall_watch
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Takes ONE classifier sample for `partition` and classifies its smallest
    /// un-resolved WAL sequence.
    ///
    /// The verdict DISPATCHES ON THE ORIGIN TAG FIRST, and consults the in-flight
    /// registry and the partition queue only inside the `Live` arm. That order is
    /// the whole point: a boot-seeded, mid-append or abandoned sequence is absent
    /// from both BY CONSTRUCTION, so a bare "absent from both means leak" rule
    /// would report a hung disk, and a prior incarnation's un-replayed frame, as
    /// code bugs.
    ///
    /// The three probes are sequential and non-overlapping — the registry and the
    /// tag share one guard, the queue scan runs after it drops — so a sequence can
    /// move between them. That is why an ownerless read is a CANDIDATE, not a
    /// verdict: only a second sample confirms it.
    fn classifier_sample(&self, partition: u32) -> SampleOutcome {
        #[cfg(test)]
        self.classifier_seam.record_sample(partition);

        // Registry probe and tag read under ONE guard. They share a lock by
        // construction, so the mandated registry-before-tracker order cannot
        // invert; the queue scan below is what must stay outside it. With no WAL
        // there is no tracker, so the probe is vacuous — but the sample still runs
        // to completion, because the sample's SHAPE is what tests observe.
        let (stalled, lag) = self.wal.as_ref().map_or((None, None), |tracking| {
            tracking.with_partition(partition, |state| {
                let stalled = state
                    .pending
                    .iter()
                    .next()
                    .map(|(seq, origin)| (*seq, *origin, state.in_flight.contains(seq)));
                let lag = Self::prefix_complete_watermark(state)
                    .ok()
                    .map(|watermark| state.max_assigned.saturating_sub(watermark));
                (stalled, lag)
            })
        });

        Self::emit_watermark_lag(partition, lag);

        // Between the registry probe and the queue scan — the only point at which
        // draining a queued sequence produces the transient ownerless read the
        // two-sample rule exists to absorb.
        #[cfg(test)]
        self.classifier_seam.run_mid_sample_hook(partition);

        let Some((sequence, origin, in_flight)) = stalled else {
            self.stall_watch().leak_candidate.remove(&partition);
            return SampleOutcome::Healthy;
        };

        let owned = in_flight
            || self
                .queues
                .get(&partition)
                .is_some_and(|queue| queue.value().contains_wal_sequence(sequence));

        match origin {
            // "Absent from both" is this state's NORMAL: recovery replays
            // straight into the inner store and never creates an entry.
            PendingOrigin::BootUnreplayed
            // The frame is still being appended, so no entry exists yet and the
            // owner is the calling task, not a flush worker.
            | PendingOrigin::Appending
            // The tag was written AT the abandoned terminal, so it already
            // carries the class; by then the entry is gone from both.
            | PendingOrigin::Abandoned => {
                self.stall_watch().leak_candidate.remove(&partition);
                SampleOutcome::Alarm {
                    sequence,
                    alarm: WalWatermarkAlarm::AbandonedWrite { origin },
                }
            }
            PendingOrigin::Live if owned => {
                // Either a flush worker's inner call has not returned, or the
                // worker is blocked on an earlier entry and this one is still
                // queued. Both are environmental, and neither is a leak: the
                // sequence has an owner.
                self.stall_watch().leak_candidate.remove(&partition);
                SampleOutcome::Alarm {
                    sequence,
                    alarm: WalWatermarkAlarm::AbandonedWrite { origin },
                }
            }
            PendingOrigin::Live => {
                // `Live` is entered only at the queue insert, so this means
                // precisely: the sequence WAS queued and dequeued, and is missing
                // from the registry. Confirm it before accusing the code.
                let confirmed = {
                    let mut watch = self.stall_watch();
                    if watch.leak_candidate.get(&partition) == Some(&sequence) {
                        true
                    } else {
                        watch.leak_candidate.insert(partition, sequence);
                        false
                    }
                };
                if confirmed {
                    SampleOutcome::Alarm {
                        sequence,
                        alarm: WalWatermarkAlarm::TrackerLeak,
                    }
                } else {
                    SampleOutcome::LeakCandidate { sequence }
                }
            }
        }
    }

    /// Publishes the partition's applied-watermark lag on the Prometheus facade.
    ///
    /// An unseeded partition reports nothing rather than zero: its watermark is
    /// unknowable, and a zero there would read as "fully caught up" — the one
    /// claim that state cannot support.
    fn emit_watermark_lag(partition: u32, lag: Option<u64>) {
        let Some(lag) = lag else {
            return;
        };
        describe_wal_watermark_metrics();
        // Precision loss only past 2^53 un-applied frames in one partition, which
        // the buffer capacity makes unreachable for a monitoring signal.
        #[allow(clippy::cast_precision_loss)]
        metrics::gauge!(WAL_WATERMARK_LAG_GAUGE, "partition" => partition.to_string())
            .set(lag as f64);
    }

    /// Whether `partition`'s smallest pending sequence has held still for the
    /// whole stall bound while writes kept arriving.
    ///
    /// This is the ONE Tier-2 trigger, and it is a DURATION. The lag is a
    /// sequence count bounded by the buffer capacity, so a count ceiling would
    /// either fire on a legitimately deep queue or never fire at all; it stays a
    /// continuous gauge and never promotes itself to a fatal alarm.
    ///
    /// An idle partition is NOT stalled: with no further sequences assigned there
    /// is nothing the watermark is failing to keep up with, and reporting one
    /// would send an operator after a quiet system.
    fn stall_reportable(&self, partition: u32, bound: std::time::Duration) -> bool {
        let Some(tracking) = &self.wal else {
            return false;
        };
        let probe = tracking.with_partition(partition, |state| {
            (state.pending.keys().next().copied(), state.max_assigned)
        });
        let (min, max_assigned) = probe;
        let mut watch = self.stall_watch();
        let Some(sequence) = min else {
            watch.observed.remove(&partition);
            return false;
        };
        match watch.observed.get(&partition) {
            Some(observed) if observed.sequence == sequence => {
                observed.since.elapsed() >= bound && max_assigned > observed.max_assigned
            }
            _ => {
                watch.observed.insert(
                    partition,
                    StallObservation {
                        sequence,
                        max_assigned,
                        since: std::time::Instant::now(),
                    },
                );
                false
            }
        }
    }

    /// Emits one stalled-watermark alarm on its own operator-visible target.
    fn report_alarm(&self, partition: u32, sequence: u64, alarm: WalWatermarkAlarm) {
        error!(
            target: "topgun_server::storage::wal_watermark",
            partition,
            sequence,
            class = ?alarm,
            stall_bound_ms = self.config.wal_watermark_stall_bound_ms,
            "{}",
            alarm.operator_action()
        );
    }

    /// Runs one watchdog pass over every partition that has tracking state.
    ///
    /// A partition is sampled every pass — the lag gauge is continuous — but a
    /// verdict is only REPORTED once the stall has outlived the bound. Sampling
    /// and reporting are split so the fatal tier stays quiet through ordinary
    /// backoff while the early warning keeps moving.
    async fn run_watchdog_pass(&self, bound: std::time::Duration) {
        let Some(tracking) = &self.wal else {
            return;
        };
        let partitions: Vec<u32> = tracking
            .partitions
            .iter()
            .map(|entry| *entry.key())
            .collect();

        let mut reconfirm = Vec::new();
        for partition in partitions {
            let reportable = self.stall_reportable(partition, bound);
            match self.classifier_sample(partition) {
                SampleOutcome::Alarm { sequence, alarm } if reportable => {
                    self.report_alarm(partition, sequence, alarm);
                }
                SampleOutcome::LeakCandidate { .. } if reportable => reconfirm.push(partition),
                SampleOutcome::Alarm { .. }
                | SampleOutcome::LeakCandidate { .. }
                | SampleOutcome::Healthy => {}
            }
        }

        if reconfirm.is_empty() {
            return;
        }
        // The confirming sample is separated from the first by a delay derived
        // from the bound and floored above the task-schedule window the false
        // positive lives in, so a sequence drained mid-classification has landed
        // by the time the second sample reads it.
        tokio::time::sleep(std::time::Duration::from_millis(
            self.config.reconfirm_delay_ms(),
        ))
        .await;
        for partition in reconfirm {
            if let SampleOutcome::Alarm { sequence, alarm } = self.classifier_sample(partition) {
                self.report_alarm(partition, sequence, alarm);
            }
        }
    }

    /// Mark `seq` resolved — its bytes are durable in the inner store (flushed)
    /// or it was superseded by a coalescing write (retired). This is what lets
    /// the prefix-complete flushed watermark advance past `seq`.
    fn resolve_pending(&self, seq: u64) {
        self.pending_seqs().remove(&seq);
    }

    /// Snapshot the pending staging entries for one map as a key-sorted map of
    /// `key -> Option<RecordValue>` (`Some` = buffered write, `None` = pending
    /// delete).
    ///
    /// The staging map is keyed globally by `(map, key)`, so this filters by map
    /// name. The buffer is bounded by `TOPGUN_WRITEBEHIND_CAPACITY`, so the
    /// snapshot is O(buffer), not O(map). Taking it upfront keeps the scan and
    /// leaf overlays single-pass and free of the staging lock during the inner
    /// enumeration.
    ///
    /// A value scan calls this once per batch, not once per scan: every call
    /// walks every slot of every map and clones each buffered value of `map`.
    fn collect_staging_for_map(&self, map: &str) -> BTreeMap<String, Option<RecordValue>> {
        let mut pending = BTreeMap::new();
        for entry in &self.staging {
            if entry.key().0 == map {
                pending.insert(
                    entry.key().1.clone(),
                    entry.value().value.as_ref().map(StagedValue::to_value),
                );
            }
        }
        pending
    }

    /// Remove the staging slot for `(map, key)` ONLY if it still corresponds to
    /// the write identified by `flushed_seq`. Returns `true` if it was removed.
    ///
    /// The background flush drains a batch and then persists each entry; the
    /// queue lock is released during the persist, so a newer write can coalesce
    /// into staging (with a higher `seq`) in between. Clearing the slot
    /// unconditionally after a flush would then wipe that newer value, dropping
    /// read-your-writes back to the now-stale durable copy — fatal under active
    /// eviction, where the resident copy is gone and staging is the only correct
    /// source. The `seq` guard removes the slot only when no newer write has
    /// landed, leaving the newer value to be cleared by its own flush.
    fn clear_staging_if_current(&self, map: &str, key: &str, flushed_seq: u64) -> bool {
        self.staging
            .remove_if(&(map.to_string(), key.to_string()), |_, slot| {
                slot.seq == flushed_seq
            })
            .is_some()
    }

    /// Stage the latest buffered `value` for `(map, key)` under sequence `seq`,
    /// keeping monotonicity: a write only replaces the slot if its `seq` is at
    /// least the slot's current `seq`. `seq` is from `assign_tracked_sequence()`,
    /// strictly increasing per call, so a higher `seq` is always the later write.
    /// Two concurrent writers on the same key can otherwise interleave between
    /// the sequence assignment and this insert and let the older write clobber the
    /// newer staged value (a plain `insert` is last-writer-wins by wall-clock,
    /// not by `seq`). The monotonic guard makes the slot's `seq` truthful, which
    /// is the identity [`clear_staging_if_current`] relies on.
    ///
    /// Caller contract (`TG-WB-007`): a write calls this directly before its
    /// queue insert, with no await and no lock held from this call to the end
    /// of the insert. Staged first, the slot is there for whichever terminal
    /// persists the entry; with nothing able to suspend the writer in
    /// between, a slot is never left behind without an entry.
    fn stage(&self, map: &str, key: &str, seq: u64, value: Option<StagedValue>) {
        use dashmap::mapref::entry::Entry;
        match self.staging.entry((map.to_string(), key.to_string())) {
            Entry::Occupied(mut e) => {
                if seq >= e.get().seq {
                    e.insert(StagingSlot { seq, value });
                }
            }
            Entry::Vacant(e) => {
                e.insert(StagingSlot { seq, value });
            }
        }
    }

    /// Returns the next WAL sequence number for ordering.
    fn next_wal_sequence(&self) -> u64 {
        self.wal_sequence.fetch_add(1, Ordering::Relaxed)
    }

    /// Assigns the next WAL sequence for `partition` and, when a WAL exists,
    /// records it pending in the SAME critical section the watermark reads.
    ///
    /// Folding the counter bump and the pending insert under one lock is what
    /// keeps the watermark honest under concurrency: a separate bump-then-track
    /// leaves a window where the counter has moved but the pending map is still
    /// empty, during which the empty-map rule returns a value ABOVE the
    /// just-assigned, still-buffered sequence — a mid-range hole that marks an
    /// un-flushed frame applied.
    ///
    /// The tag is `Appending`, never `Live`: `Live` asserts that an entry owns the
    /// sequence, and at the assign the frame is not even appended yet. Tagging
    /// `Live` here is what routes a hung append to "this is a code bug" instead of
    /// to the disk.
    ///
    /// With no WAL the counter still bumps and nothing tracks the sequence. That
    /// is correct: no frame will ever bear it, so it is an inert identifier. The
    /// counter is global and its monotonicity is load-bearing across restarts, so
    /// the bump must not become conditional.
    fn assign_wal_sequence(&self, partition: u32) -> u64 {
        let seq = match &self.wal {
            None => self.next_wal_sequence(),
            Some(tracking) => tracking.with_partition(partition, |state| {
                let seq = self.next_wal_sequence();
                state.pending.insert(seq, PendingOrigin::Appending);
                state.max_assigned = state.max_assigned.max(seq);
                seq
            }),
        };
        // Report the assigned sequence as a NAME to any installed harness observer.
        // This is the ONLY point a sequence is minted for a mutation, so it covers
        // every mutating path exactly once; it hands out an identifier, not a
        // durability verdict.
        #[cfg(test)]
        self.notify_append_observer(partition, seq);
        seq
    }

    /// Installs the harness's append observer (see [`Self::append_observer`]).
    ///
    /// Reachable from sibling test modules the same way the other `test_set_*`
    /// seams are; a `None` observer (the default) makes this a no-op branch.
    #[cfg(test)]
    pub(crate) fn test_set_append_observer(&self, observer: Arc<dyn Fn(u32, u64) + Send + Sync>) {
        *self
            .append_observer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(observer);
    }

    /// Fires the installed append observer, if any, with the partition and the
    /// sequence just assigned. Cloned out before invoking so the observer cannot
    /// deadlock against this lock.
    #[cfg(test)]
    fn notify_append_observer(&self, partition: u32, seq: u64) {
        let observer = self
            .append_observer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some(observer) = observer {
            observer(partition, seq);
        }
    }

    /// Whether the SURVIVOR of a coalesce makes its predecessor's frame
    /// redundant.
    ///
    /// Read off the survivor's own framing, never the retired entry's: a
    /// survivor carrying complete state subsumes any predecessor, while one
    /// carrying only its own delta subsumes nothing regardless of what it
    /// displaces.
    #[cfg_attr(not(test), allow(clippy::unused_self))]
    fn survivor_subsumes(&self, op: &WalOp) -> bool {
        #[cfg(test)]
        if self.force_non_subsuming_survivor.load(Ordering::Relaxed) {
            return false;
        }
        op.subsumes_on_coalesce()
    }

    /// Makes every subsequent coalesce route as if the survivor's framing
    /// carried only partial state.
    #[cfg(test)]
    pub(crate) fn test_force_non_subsuming_survivor(&self, force: bool) {
        self.force_non_subsuming_survivor
            .store(force, Ordering::Relaxed);
    }

    /// Promotes `seq` from `Appending` to `Live` once its frame is on disk and its
    /// entry is in the partition queue.
    ///
    /// CONDITIONAL on the sequence still being present: the flush loop is a
    /// separately spawned task and a coalesced entry inherits its predecessor's
    /// store time, so it is immediately drain-eligible — the whole
    /// drain/flush/resolve sequence can complete between the queue insert and this
    /// call. A blind insert here would re-insert a sequence that already reached a
    /// durable terminal, pinning the partition's watermark forever and reporting a
    /// leak against code that did exactly the right thing.
    fn promote_wal_sequence(&self, partition: u32, seq: u64) {
        let Some(tracking) = &self.wal else {
            return;
        };
        tracking.with_partition(partition, |state| {
            if let Some(origin) = state.pending.get_mut(&seq) {
                *origin = PendingOrigin::Live;
            }
        });
    }

    /// Drops `seq` from tracking after its WAL append failed before any byte of
    /// the frame could reach the segment.
    ///
    /// Scoped to the FAILED sequence only. In a multi-key loop the earlier keys'
    /// sequences are frame-backed and MUST stay pending — rolling those back would
    /// let the watermark advance past frames that were never applied, which is the
    /// exact loss this tracker exists to prevent.
    ///
    /// Every error that reaches this point is pre-frame by construction: a failure
    /// at or after the frame write fail-stops the process inside the WAL rather
    /// than returning, so a returned error can only come from upstream of the
    /// write path.
    fn rollback_wal_sequence(&self, partition: u32, seq: u64) {
        let Some(tracking) = &self.wal else {
            return;
        };
        tracking.with_partition(partition, |state| {
            state.pending.remove(&seq);
        });
    }

    /// The applied watermark for a partition, in the INCLUSIVE space
    /// `mark_applied` speaks: the largest sequence such that every frame at or
    /// below it is resolved.
    ///
    /// The smallest pending sequence is itself UNRESOLVED, so the watermark stops
    /// one below it — marking it would filter its frame out of replay and make its
    /// segment collectable while the write is still buffered.
    ///
    /// With nothing pending, every sequence ever assigned to the partition is
    /// resolved, so the watermark is the highest one assigned. It is NEVER the
    /// counter's current value: the counter hands out sequences with `fetch_add`,
    /// which returns the OLD value, so its loaded value is the NEXT sequence to
    /// assign — using it would pre-mark the next write's frame applied before that
    /// frame is even written.
    ///
    /// NOTHING is knowable before the partition is seeded from the on-disk WAL,
    /// and that holds whether or not this incarnation has pending sequences of
    /// its own. A prior incarnation's un-applied frames sit BELOW every sequence
    /// this incarnation hands out (the live counter starts above the highest
    /// observed), so `min(pending) - 1` computed on an unseeded partition sits
    /// ABOVE them — it would mark frames applied that were never replayed and
    /// hand their segments to GC. The seeding failure that makes this reachable
    /// is rare, but its cost is permanent loss, so the guard covers both arms.
    fn prefix_complete_watermark(state: &PartitionTracking) -> Result<u64, WalWatermarkError> {
        if !state.seeded {
            return Err(WalWatermarkError::Unseeded);
        }
        match state.pending.keys().next() {
            Some(min) => Ok(min.saturating_sub(1)),
            None => Ok(state.max_assigned),
        }
    }

    /// C13 re-introduction (`WatermarkMode::InclusiveOffByOne`): identical to
    /// [`Self::prefix_complete_watermark`] except it returns `min(pending)`
    /// rather than `min(pending) - 1`, marking the smallest still-unresolved
    /// sequence applied instead of stopping one below it. Test-only: production
    /// always uses the prefix-complete rule.
    #[cfg(test)]
    fn inclusive_off_by_one_watermark(state: &PartitionTracking) -> Result<u64, WalWatermarkError> {
        if !state.seeded {
            return Err(WalWatermarkError::Unseeded);
        }
        match state.pending.keys().next() {
            Some(min) => Ok(*min),
            None => Ok(state.max_assigned),
        }
    }

    /// Seeds `partition`'s pending map from the frames the on-disk WAL still
    /// reports as un-applied, so this incarnation's watermark respects what the
    /// previous one left behind.
    ///
    /// Recovery replays directly into the inner store and never touches
    /// write-behind, so without this a restarted process boots with an empty
    /// pending map and the first flush marks every un-replayed frame applied —
    /// filtering them out of the next replay and making their segments
    /// collectable. The seeded sequences never resolve in this incarnation: they
    /// resolve by replaying on a future restart, and the stall they cause is the
    /// intended surfacing of a write the store rejected.
    ///
    /// `unapplied` is a full read-and-decode of every retained segment for the
    /// partition, so it runs once per partition, off the steady-state path, and
    /// outside the tracking lock.
    ///
    /// The install is double-checked, and that is load-bearing rather than
    /// defensive. Only the FIRST installer's scan provably predates any live
    /// append for the partition — a losing racer's scan may have observed a
    /// winner's freshly-appended live frame, so its install must not reach the
    /// map. Sequences are inserted without displacing existing entries for the
    /// same reason: a seed must never overwrite what a live write recorded.
    async fn ensure_wal_seeded(&self, partition: u32) {
        let Some(tracking) = &self.wal else {
            return;
        };
        if tracking.with_partition(partition, |state| state.seeded) {
            return;
        }

        #[cfg(test)]
        if self.boot_seed_mode() == BootSeedMode::Empty {
            // C12 re-introduction: mark the partition seeded WITHOUT populating
            // its pending map from `wal.unapplied(p)` — the pending map boots
            // empty/blind, exactly as if every frame a prior incarnation left
            // un-applied had already been applied.
            tracking.with_partition(partition, |state| {
                state.seeded = true;
            });
            return;
        }

        let frames = match tracking.wal.unapplied(partition).await {
            Ok(frames) => frames,
            Err(err) => {
                // Leaving the partition unseeded holds its watermark at the typed
                // error, which under-advances rather than over-advances: GC is
                // delayed, no frame is marked applied.
                error!(
                    partition,
                    error = %err,
                    "Failed to read un-applied WAL frames; partition watermark stays unseeded"
                );
                return;
            }
        };

        tracking.with_partition(partition, |state| {
            if state.seeded {
                return;
            }
            for frame in &frames {
                state
                    .pending
                    .entry(frame.sequence)
                    .or_insert(PendingOrigin::BootUnreplayed);
            }
            state.seeded = true;
        });
    }

    /// Records that a flush worker now owns `seqs`, having taken their entry out
    /// of the partition queue.
    ///
    /// Between the queue removal and the terminal the entry exists only as a
    /// local binding, so without this a sequence is absent from both the queue
    /// and the registry — which is the signature of a leaked sequence. A hung
    /// inner store would then be reported as a code bug rather than as a disk to
    /// go and look at.
    fn register_in_flight(&self, partition: u32, seqs: &BTreeSet<u64>) {
        let Some(tracking) = &self.wal else {
            return;
        };
        tracking.with_partition(partition, |state| {
            state.in_flight.extend(seqs.iter().copied());
        });
    }

    /// Releases a flush worker's ownership of `seqs` without resolving them.
    ///
    /// Used where the entry leaves in-flight but has NOT reached a terminal — a
    /// retry puts it back on the queue. Skipping it would leave the sequence
    /// both queued and in-flight, and would leak one registry entry per retry.
    fn deregister_in_flight(&self, partition: u32, seqs: &BTreeSet<u64>) {
        let Some(tracking) = &self.wal else {
            return;
        };
        tracking.with_partition(partition, |state| {
            for seq in seqs {
                state.in_flight.remove(seq);
            }
        });
    }

    /// Marks `seqs` abandoned: their entry reached a terminal that did NOT make
    /// the write durable in the inner store.
    ///
    /// Deliberately does not resolve. Resolving would let the watermark advance
    /// past an acked write that is in neither the inner store nor — once GC runs
    /// behind the advanced watermark — the WAL, which is permanent silent loss.
    /// Leaving the sequence pending keeps the frame replayable, stalls the
    /// watermark, and raises the abandoned-write alarm; a transient store failure
    /// then self-heals on the next restart's replay.
    ///
    /// The re-tag is conditional. A sequence may already have been resolved by a
    /// racing durable terminal for the same entry, and a blind insert would
    /// re-introduce a sequence that is already applied, pinning the watermark
    /// forever.
    fn abandon_wal_sequences(&self, partition: u32, seqs: &BTreeSet<u64>) {
        let Some(tracking) = &self.wal else {
            return;
        };
        tracking.with_partition(partition, |state| {
            for seq in seqs {
                state.in_flight.remove(seq);
                if let Some(origin) = state.pending.get_mut(seq) {
                    *origin = PendingOrigin::Abandoned;
                }
            }
        });
    }

    /// Resolves `seqs` WITHOUT advancing the durable watermark.
    ///
    /// This is the coalesce-retire disposition. The retired frames are subsumed
    /// by the survivor's, so they must stop holding the pending prefix back — but
    /// no write has reached the inner store here, and the survivor is still only
    /// buffered. Advancing would put `mark_applied`'s read-write-fsync, and the
    /// segment seal/rotate it triggers, on the `add()` path — changing the sidecar
    /// cadence and paying disk I/O per coalesce. The survivor's own durable
    /// terminal is what advances past them, one flush later.
    fn resolve_wal_sequences(&self, partition: u32, seqs: &BTreeSet<u64>) {
        let Some(tracking) = &self.wal else {
            return;
        };
        tracking.with_partition(partition, |state| {
            for seq in seqs {
                state.in_flight.remove(seq);
                state.pending.remove(seq);
            }
        });
    }

    /// Resolves `seqs` as durably applied and advances the partition's WAL
    /// watermark to the prefix-complete frontier.
    ///
    /// Resolve and advance are ONE unit: a durable terminal that resolves without
    /// advancing leaves the sidecar frozen and the WAL growing, and neither alarm
    /// tier can see it because both key on the pending set, which advanced.
    ///
    /// The tracking lock is dropped before `mark_applied`, which does disk I/O.
    /// A racing computation can therefore hand it a stale-but-lower watermark;
    /// that is harmless because `mark_applied` clamps monotonically, and lower is
    /// the safe direction.
    async fn resolve_and_advance(&self, partition: u32, seqs: &BTreeSet<u64>) {
        let Some(tracking) = &self.wal else {
            return;
        };
        // Seeded by construction at every advance, not only at the mutating
        // entry points — this is the seed RETRY site.
        //
        // `ensure_wal_seeded` swallows a WAL-read error and leaves the partition
        // unseeded (under-advancing is the safe direction), so a transient read
        // failure at the entry point leaves a partition with no knowable
        // watermark. The mutating entry points retry only when another write for
        // the same partition arrives; a partition that goes quiet after that
        // failure reaches its terminals through the flush loop and the shutdown
        // drain alone, and would otherwise stay unseeded forever and grow its
        // WAL where neither alarm can see it. Retrying here makes every advance
        // site seeded by construction.
        //
        // Cheap after the first success — a bool read under the partition lock.
        self.ensure_wal_seeded(partition).await;
        #[cfg(test)]
        let watermark_mode = self.watermark_mode();

        let watermark = tracking.with_partition(partition, |state| {
            for seq in seqs {
                state.in_flight.remove(seq);
                state.pending.remove(seq);
            }
            #[cfg(test)]
            match watermark_mode {
                WatermarkMode::ScalarMax => {
                    // Reproduces the pre-fix scalar advance so the differential
                    // test keeps a live negative control rather than a one-off
                    // revert.
                    return Ok(seqs.iter().copied().max().unwrap_or(0));
                }
                #[cfg(test)]
                WatermarkMode::InclusiveOffByOne => {
                    return Self::inclusive_off_by_one_watermark(state);
                }
                WatermarkMode::PrefixComplete => {}
            }
            Self::prefix_complete_watermark(state)
        });

        let watermark = match watermark {
            Ok(watermark) => watermark,
            Err(err) => {
                // Nothing is marked applied. The frames stay replayable and the
                // WAL grows, which is the safe direction: the alternative is
                // guessing a watermark for frames this process never saw.
                error!(
                    partition,
                    error = %err,
                    "Refusing to advance the WAL watermark for an unseeded partition"
                );
                return;
            }
        };

        if let Err(err) = tracking.wal.mark_applied(partition, watermark).await {
            warn!(
                partition,
                watermark,
                error = %err,
                "Failed to mark WAL watermark applied; next restart will re-replay (safe but redundant)"
            );
        }
    }

    /// Disposes of a coalesce-retired entry's WAL sequences against the survivor.
    ///
    /// Returns the sequences the caller must EARLY-RESOLVE (once the partition
    /// queue guard has dropped) when the survivor's frame subsumes the retired
    /// one, and `None` when it does not — in which case they are carried forward
    /// onto the survivor, which then resolves them together with its own at its
    /// terminal outcome.
    ///
    /// Carry-forward is not the unconditional choice even though it is
    /// framing-agnostic: it pays unbounded set growth on a hot, repeatedly
    /// coalesced key, and an abandoned survivor stalls the watermark at the
    /// OLDEST carried sequence rather than at its own. Early-resolve is sound
    /// exactly when the survivor's frame re-carries the retired frame's effect,
    /// which is what the predicate decides.
    ///
    /// Dropping the retired sequences instead — carrying neither disposition —
    /// leaves them pending with no owner, so a repeatedly coalesced key pins its
    /// partition's watermark forever.
    fn coalesce_wal_sequences(
        survivor_subsumes: bool,
        survivor: &mut DelayedEntry,
        retired: BTreeSet<u64>,
    ) -> Option<BTreeSet<u64>> {
        if survivor_subsumes {
            Some(retired)
        } else {
            survivor.wal_sequences.extend(retired);
            None
        }
    }

    /// Appends an entry to the WAL and satisfies the fsync policy before
    /// returning. Returns `Ok(())` immediately when no WAL is configured.
    ///
    /// Must be called before returning `Ok(())` from any mutating method so
    /// that a crash after ack but before the background flush still allows
    /// recovery to replay the acked write.
    async fn wal_append(&self, partition: u32, entry: &WalEntry) -> anyhow::Result<()> {
        if let Some(tracking) = &self.wal {
            tracking.wal.append(partition, entry).await?;
        }
        Ok(())
    }

    /// The pending `wal_seq`s of `partition` with their origin tags.
    ///
    /// Reachable from sibling test modules, which Rust privacy otherwise keeps out
    /// of this store's internals entirely.
    #[cfg(test)]
    pub(crate) fn test_pending_wal_sequences(&self, partition: u32) -> Vec<(u64, PendingOrigin)> {
        self.wal.as_ref().map_or_else(Vec::new, |tracking| {
            tracking.with_partition(partition, |state| {
                state.pending.iter().map(|(s, o)| (*s, *o)).collect()
            })
        })
    }

    /// The highest `wal_seq` ever assigned to `partition`.
    #[cfg(test)]
    pub(crate) fn test_max_assigned_wal_sequence(&self, partition: u32) -> u64 {
        self.wal.as_ref().map_or(0, |tracking| {
            tracking.with_partition(partition, |state| state.max_assigned)
        })
    }

    /// The `wal_seq`s currently owned by a flush worker in `partition`.
    #[cfg(test)]
    pub(crate) fn test_in_flight_wal_sequences(&self, partition: u32) -> Vec<u64> {
        self.wal.as_ref().map_or_else(Vec::new, |tracking| {
            tracking.with_partition(partition, |state| state.in_flight.iter().copied().collect())
        })
    }

    /// Whether nothing is outstanding: no entry queued, no flush pass open, no
    /// sequence in flight.
    ///
    /// The epoch is read on both sides of the scan and must be even and
    /// unchanged, so the flush loop was outside a pass for the whole scan and
    /// held no drained batch. That makes the answer exact rather than a matter
    /// of timing, provided the caller is the store's only writer. A pass that
    /// never closes leaves the epoch odd, so this fails closed.
    #[cfg(test)]
    pub(crate) fn test_is_idle(&self) -> bool {
        let before = self.flush_pass_epoch.load(Ordering::SeqCst);
        if before % 2 == 1 {
            return false;
        }
        if self.queues.iter().any(|queue| !queue.entries.is_empty()) {
            return false;
        }
        if let Some(tracking) = &self.wal {
            let partitions: Vec<u32> = tracking
                .partitions
                .iter()
                .map(|entry| *entry.key())
                .collect();
            for partition in partitions {
                if !tracking.with_partition(partition, |state| state.in_flight.is_empty()) {
                    return false;
                }
            }
        }
        self.flush_pass_epoch.load(Ordering::SeqCst) == before
    }

    /// The prefix-complete watermark `partition` would advance to right now.
    ///
    /// `None` means no WAL exists, so the watermark has no subject at all; an
    /// inner `Err` means the partition has not been boot-seeded yet. The two are
    /// kept distinct because collapsing them is exactly the conflation the
    /// typed error exists to prevent.
    #[cfg(test)]
    pub(crate) fn test_wal_watermark(
        &self,
        partition: u32,
    ) -> Option<Result<u64, WalWatermarkError>> {
        self.wal.as_ref().map(|tracking| {
            tracking.with_partition(partition, |state| Self::prefix_complete_watermark(state))
        })
    }

    /// Whether `partition`'s pending map has been seeded from the on-disk WAL.
    #[cfg(test)]
    pub(crate) fn test_wal_partition_seeded(&self, partition: u32) -> bool {
        self.wal
            .as_ref()
            .is_some_and(|tracking| tracking.with_partition(partition, |state| state.seeded))
    }
}

// ---------------------------------------------------------------------------
// Background flush loop (R3)
// ---------------------------------------------------------------------------

/// Environment variable naming the sentinel file of the store-refusal seam.
#[cfg(feature = "fault-injection")]
const FAULT_STORE_REFUSE_FILE_ENV: &str = "TOPGUN_FAULT_STORE_REFUSE_FILE";

/// The sentinel path of the store-refusal seam, read from the environment once
/// per process: the arming gate and the boot line must not observe different
/// answers, and a path that could change mid-run would make a refusal
/// unattributable.
#[cfg(feature = "fault-injection")]
fn fault_store_refuse_path() -> Option<&'static std::path::Path> {
    static PATH: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    PATH.get_or_init(|| std::env::var_os(FAULT_STORE_REFUSE_FILE_ENV).map(PathBuf::from))
        .as_deref()
}

/// Whether the store-refusal seam is armed right now: a sentinel path is
/// configured AND a file exists there. The file is checked on every call so a
/// test can let writes through, then start refusing them, inside one process.
#[cfg(feature = "fault-injection")]
fn fault_store_refusal_armed() -> bool {
    fault_store_refuse_path().is_some_and(std::path::Path::exists)
}

/// Environment variable naming the sentinel file of the store-panic seam.
#[cfg(feature = "fault-injection")]
const FAULT_STORE_PANIC_FILE_ENV: &str = "TOPGUN_FAULT_STORE_PANIC_FILE";

/// The sentinel path of the store-panic seam, read from the environment once
/// per process, for the reasons the refusal seam's path is.
#[cfg(feature = "fault-injection")]
fn fault_store_panic_path() -> Option<&'static std::path::Path> {
    static PATH: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    PATH.get_or_init(|| std::env::var_os(FAULT_STORE_PANIC_FILE_ENV).map(PathBuf::from))
        .as_deref()
}

/// Whether the store-panic seam is armed right now: a sentinel path is
/// configured AND a file exists there. Checked on every call, so a test can
/// let entries through and then make the next one panic inside one process.
#[cfg(feature = "fault-injection")]
fn fault_store_panic_armed() -> bool {
    fault_store_panic_path().is_some_and(std::path::Path::exists)
}

/// Persists one dequeued entry to the inner store — the single body the
/// background flush and the shutdown drain share.
///
/// A `StoreCell` entry persists the cell's value as it is now, read under the
/// cell lock. When the inner store accepts encoded records, the value is
/// encoded under the lock and handed over as bytes, so no copy of the record
/// outlives the lock; otherwise it is cloned out under the lock. Either way the
/// guard lives only inside its block: it is `!Send`, and must not be held
/// across the inner store's await (or into any call that takes the same lock).
async fn persist_entry(inner: &dyn MapDataStore, entry: &DelayedEntry) -> anyhow::Result<()> {
    // Panic here, before the inner store is reached, so the background flush
    // and the shutdown drain both unwind from the point where a panic of the
    // inner store itself would reach them.
    #[cfg(feature = "fault-injection")]
    assert!(
        !fault_store_panic_armed(),
        "fault injection: the inner store panicked"
    );

    // Refuse here, before the inner store is reached, so the background flush
    // and the shutdown drain both see the refusal exactly as they would see an
    // inner-store error.
    #[cfg(feature = "fault-injection")]
    if fault_store_refusal_armed() {
        return Err(anyhow::anyhow!(
            "fault injection: the inner store refused the write"
        ));
    }

    match &entry.operation {
        DelayedOp::Store {
            value,
            expiration_time,
        } => {
            inner
                .add(
                    &entry.map,
                    &entry.key,
                    value,
                    *expiration_time,
                    entry.store_time,
                )
                .await
        }
        DelayedOp::StoreCell {
            cell,
            expiration_time,
        } => {
            if inner.accepts_encoded() {
                let bytes = {
                    let guard = cell.lock();
                    rmp_serde::to_vec_named(&guard.value)?
                };
                inner
                    .add_encoded(
                        &entry.map,
                        &entry.key,
                        &bytes,
                        *expiration_time,
                        entry.store_time,
                    )
                    .await
            } else {
                let value = {
                    let guard = cell.lock();
                    guard.value.clone()
                };
                inner
                    .add(
                        &entry.map,
                        &entry.key,
                        &value,
                        *expiration_time,
                        entry.store_time,
                    )
                    .await
            }
        }
        DelayedOp::Remove => inner.remove(&entry.map, &entry.key, entry.store_time).await,
    }
}

/// Refused entries in a row, inside one flush pass, that start a store-wide
/// hold. One refused entry can never start one.
const HOLD_TRIP_REFUSALS: u32 = 3;
/// Refusals of one entry from which the retry warning is emitted.
const RETRY_WARN_ATTEMPTS: u32 = 3;
/// The retry warning is emitted at most once per this interval.
const RETRY_WARN_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

/// How long the store is left alone after the `attempt`-th refusal:
/// `min(base << attempt, cap)`, for every `attempt`.
///
/// Refusals are not limited, so `attempt` grows for as long as the store
/// refuses; a shift that would lose bits is past any cap.
fn retry_backoff(base_ms: u64, cap_ms: u64, attempt: u32) -> std::time::Duration {
    let ms = base_ms
        .checked_shl(attempt)
        .filter(|shifted| shifted >> attempt == base_ms)
        .map_or(cap_ms, |shifted| shifted.min(cap_ms));
    std::time::Duration::from_millis(ms)
}

/// What the flush task remembers between passes about a store that refuses.
///
/// Owned by the flush task alone, so it needs no lock; the store carries
/// atomics that mirror the hold for readers outside the task.
#[derive(Default)]
struct RetrySchedule {
    /// Set from the pass that trips the hold until a probe is accepted. Until
    /// this instant no pass takes anything from a queue; from it on, a pass
    /// takes one entry only, the probe.
    hold_until: Option<tokio::time::Instant>,
    /// Probes refused in a row since the hold began.
    failed_probes: u32,
    /// An entry reached the warning threshold or a hold began, and the
    /// backlog has not cleared since.
    warning_armed: bool,
    /// When the warning was last emitted; `None` when it has not been since
    /// the backlog was last clear.
    last_warning: Option<tokio::time::Instant>,
    /// The most recently refused entry, as the warning's example.
    example: Option<RefusedExample>,
    /// The inner store reported itself broken: no pass calls it again
    /// (`TG-WB-010`).
    store_broken: bool,
}

/// One refused entry, named in the retry warning so an operator has a
/// concrete write to look at.
struct RefusedExample {
    map: String,
    key: String,
    attempts: u32,
    error: String,
}

impl WriteBehindDataStore {
    /// The one terminal of an entry the store accepted, whether a pass
    /// attempted it in its turn or as a probe.
    async fn settle_stored(&self, entry: &DelayedEntry) {
        // Remove this write's staging slot ONLY if it is still the one that
        // was flushed: a newer coalesced write must survive.
        self.clear_staging_if_current(&entry.map, &entry.key, entry.sequence);
        self.pending_count.fetch_sub(1, Ordering::Relaxed);
        // The bytes are in the inner store, so the entry sequence may let the
        // prefix-complete flushed watermark past it. This is the ONLY signal
        // that advances the tombstone durability fence.
        self.resolve_pending(entry.sequence);
        // Resolve the entry's WAL sequences and advance the partition
        // watermark to the prefix-complete frontier, so a clean restart does
        // not re-replay writes that are already in the inner store — and so
        // no lower, still un-flushed frame of the partition is marked applied.
        self.resolve_and_advance(partition_for(&entry.map, &entry.key), &entry.wal_sequences)
            .await;
    }

    /// Puts an entry a pass holds back on its queue, as it is.
    ///
    /// Nothing here suspends. A task can be dropped only where it suspends, so
    /// an entry that is out of every queue has its count and its entry
    /// sequence settled before the flush task can go away.
    fn put_back(&self, entry: DelayedEntry) {
        let partition = partition_for(&entry.map, &entry.key);
        // Back on the queue, so this worker no longer owns the sequences —
        // but they are NOT terminal and stay pending. Skipping this would
        // leave them both queued and in flight, and would leak a registry
        // entry per retry.
        self.deregister_in_flight(partition, &entry.wal_sequences);
        // One statement, so the shard guard is a temporary: held any longer
        // it would block every synchronous access to the shard — a client
        // write, the stall watchdog (TG-WB-004).
        let outcome = self
            .queues
            .entry(partition)
            .or_default()
            .reinsert_front(entry);
        // A newer write of the key was queued while this entry was out of the
        // queue, so the entry is gone for good and two increments stand for
        // one queued entry (TG-WB-005). Settled after the shard guard, so the
        // pending-set lock is never nested under it.
        if let Reinsert::Superseded { retired_sequence } = outcome {
            self.pending_count.fetch_sub(1, Ordering::Relaxed);
            self.resolve_pending(retired_sequence);
        }
    }

    /// The disposition of an entry the store refused: it stays buffered, with
    /// its staging slot, its count, its entry sequence pending and its WAL
    /// sequences live, and is not attempted again before its backoff has
    /// passed.
    ///
    /// `returned` is when the refused call came back. The backoff is counted
    /// from there and not from the start of the pass, so a store that is slow
    /// to refuse is still left alone for the whole of it.
    ///
    /// Returns what the inner store says about the error. On `StoreBroken`
    /// the store is failed here and the schedule stops every later pass; the
    /// caller makes no further call in this pass.
    fn requeue_refused(
        &self,
        schedule: &mut RetrySchedule,
        mut entry: DelayedEntry,
        err: &anyhow::Error,
        returned: tokio::time::Instant,
    ) -> StoreErrorClass {
        metrics::counter!(FLUSH_REFUSALS_COUNTER).increment(1);
        self.retrying.store(true, Ordering::Relaxed);
        entry.retry_count = entry.retry_count.saturating_add(1);
        entry.next_attempt = Some(
            returned
                + retry_backoff(
                    self.config.backoff_base_ms,
                    self.config.backoff_cap_ms,
                    entry.retry_count,
                ),
        );
        // Written once: it says since when the store has been refusing the
        // key, which a later refusal must not move.
        if entry.first_refused.is_none() {
            entry.first_refused = Some(returned);
        }
        if entry.retry_count >= RETRY_WARN_ATTEMPTS {
            schedule.warning_armed = true;
        }
        schedule.example = Some(RefusedExample {
            map: entry.map.clone(),
            key: entry.key.clone(),
            attempts: entry.retry_count,
            error: err.to_string(),
        });
        let class = self.inner.classify_error(err);
        if let StoreErrorClass::StoreBroken(cause) = class {
            // Put back first, exactly as any refused entry: the record only
            // stops further calls, it never decides the entry's fate.
            let (map, key) = (entry.map.clone(), entry.key.clone());
            self.put_back(entry);
            self.record_store_broken(cause, err, &map, &key, "flush");
            schedule.store_broken = true;
        } else {
            self.put_back(entry);
        }
        class
    }

    /// Chooses, over all queues, the entry a probe attempts, and says which
    /// queue holds it and which key it belongs to. `None` when nothing is due.
    fn choose_probe(
        &self,
        deadline: i64,
        now: tokio::time::Instant,
    ) -> Option<(u32, (String, String))> {
        let mut best: Option<(ProbeRank, u32, (String, String))> = None;
        for queue in &self.queues {
            if let Some((rank, key)) = queue.value().best_due(deadline, now) {
                if best.as_ref().is_none_or(|(held, _, _)| rank.before(held)) {
                    best = Some((rank, *queue.key(), key));
                }
            }
        }
        best.map(|(_, partition, key)| (partition, key))
    }

    /// The test park between the choice of a probe and its removal.
    #[cfg(test)]
    async fn pass_probe_park(&self) {
        let gate = self
            .test_parks
            .between_probe_choice_and_removal
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(gate) = gate {
            gate.pass().await;
        }
    }

    /// Publishes the flush task's hold to the readers outside the task and
    /// to its gauge. A hold is in force for as long as `hold_until` is set —
    /// also after that instant has passed and before a probe has been
    /// accepted. Called where the hold changes, so a pass that changes
    /// nothing sets no gauge for it.
    fn publish_hold(&self, schedule: &RetrySchedule) {
        self.store_hold
            .store(schedule.hold_until.is_some(), Ordering::SeqCst);
        self.failed_probes
            .store(schedule.failed_probes, Ordering::SeqCst);
        metrics::gauge!(STORE_HOLD_GAUGE).set(if schedule.hold_until.is_some() {
            1.0
        } else {
            0.0
        });
    }

    /// One pass of the flush task.
    ///
    /// The pass never waits on behalf of an entry: when an entry may be
    /// attempted again is written on the entry, and a later pass honours it.
    async fn run_flush_pass(
        &self,
        schedule: &mut RetrySchedule,
        deadline: i64,
        now: tokio::time::Instant,
    ) {
        let base = self.config.backoff_base_ms;
        let cap = self.config.backoff_cap_ms;

        // A broken store stores nothing more in this process (`TG-WB-010`).
        if schedule.store_broken {
            return;
        }

        if let Some(until) = schedule.hold_until {
            // The store is being left alone: nothing is taken and no call is
            // made, whoever woke the pass.
            if now < until {
                return;
            }
            let Some((partition, (map, key))) = self.choose_probe(deadline, now) else {
                return;
            };
            #[cfg(test)]
            self.pass_probe_park().await;
            // No shard guard is held between the choice and the removal
            // (TG-WB-004), so a writer can replace the chosen entry in
            // between. The replacement inherited the schedule of the entry it
            // replaced, so it is due all the same and is the probe. The key's
            // entry can be gone only through the drain of a shutdown; then
            // there is nothing to ask the store with and nothing changes.
            let taken = self
                .queues
                .get_mut(&partition)
                .and_then(|mut queue| queue.take(&map, &key));
            let Some(probe) = taken else {
                return;
            };
            self.register_in_flight(partition, &probe.wal_sequences);
            match persist_entry(self.inner.as_ref(), &probe).await {
                Ok(()) => {
                    self.settle_stored(&probe).await;
                    // The store answers again: the hold ends and this same
                    // pass goes on over everything that is due.
                    schedule.hold_until = None;
                    schedule.failed_probes = 0;
                    self.publish_hold(schedule);
                }
                Err(err) => {
                    let returned = tokio::time::Instant::now();
                    if let StoreErrorClass::StoreBroken(_) =
                        self.requeue_refused(schedule, probe, &err, returned)
                    {
                        return;
                    }
                    schedule.failed_probes = schedule.failed_probes.saturating_add(1);
                    schedule.hold_until =
                        Some(returned + retry_backoff(base, cap, schedule.failed_probes));
                    self.publish_hold(schedule);
                    return;
                }
            }
        }

        let mut ready = Vec::new();
        for mut queue_ref in self.queues.iter_mut() {
            ready.extend(queue_ref.value_mut().drain_due(deadline, now));
        }
        if ready.is_empty() {
            return;
        }
        // The drained entries now exist only as local bindings, so record this
        // worker's ownership of their WAL sequences before the first
        // inner-store call. Done after every partition-queue guard has
        // dropped, so the tracking lock is never nested under one.
        for entry in &ready {
            self.register_in_flight(partition_for(&entry.map, &entry.key), &entry.wal_sequences);
        }
        ready.sort_by_key(DelayedEntry::pass_order);

        let mut refused_in_a_row = 0u32;
        let mut entries = ready.into_iter();
        while let Some(entry) = entries.next() {
            match persist_entry(self.inner.as_ref(), &entry).await {
                Ok(()) => {
                    self.settle_stored(&entry).await;
                    refused_in_a_row = 0;
                }
                Err(err) => {
                    let returned = tokio::time::Instant::now();
                    if let StoreErrorClass::StoreBroken(_) =
                        self.requeue_refused(schedule, entry, &err, returned)
                    {
                        // What was not attempted goes back as it was.
                        for unattempted in entries.by_ref() {
                            self.put_back(unattempted);
                        }
                        return;
                    }
                    refused_in_a_row += 1;
                    if refused_in_a_row >= HOLD_TRIP_REFUSALS {
                        // The store refuses whatever it is given: stop asking.
                        // What was not attempted goes back as it was, so the
                        // store's refusal of other entries is not held
                        // against it.
                        for unattempted in entries.by_ref() {
                            self.put_back(unattempted);
                        }
                        schedule.failed_probes = 0;
                        schedule.hold_until = Some(returned + retry_backoff(base, cap, 0));
                        schedule.warning_armed = true;
                        self.publish_hold(schedule);
                    }
                }
            }
        }
    }

    /// Reports the buffer and its refused entries at the end of a pass: the
    /// gauges, one rate-limited warning while any entry is refused, and one
    /// line when none is left after a warning.
    ///
    /// The queues are walked only while a refusal has happened and the last
    /// walk still found a refused entry; a pass on a store that refuses
    /// nothing pays one atomic load for that, and one more load and one gauge
    /// set for the buffered count.
    fn report_retries(&self, schedule: &mut RetrySchedule) {
        metrics::gauge!(BUFFERED_ENTRIES_GAUGE)
            .set(count_as_gauge(self.pending_count.load(Ordering::Relaxed)));
        if !self.retrying.load(Ordering::Relaxed) {
            return;
        }
        let now = tokio::time::Instant::now();
        let mut retrying = 0u64;
        let mut earliest = None;
        for queue in &self.queues {
            let (count, queue_earliest) = queue.value().retrying();
            retrying += count;
            earliest = earlier_refusal(earliest, queue_earliest);
        }
        // Monotonic time since the first refusal, never the entry's
        // `store_time`: that is supplied by the caller and is not a time at
        // all for some callers.
        let oldest_refusal_age = earliest.map_or(std::time::Duration::ZERO, |first| {
            now.saturating_duration_since(first)
        });
        // Also on the walk that finds nothing refused any more: it is the
        // last walk until the next refusal, and it is what brings both gauges
        // back to zero.
        metrics::gauge!(RETRYING_ENTRIES_GAUGE).set(count_as_gauge(retrying));
        metrics::gauge!(OLDEST_REFUSAL_AGE_GAUGE).set(oldest_refusal_age.as_secs_f64());

        if retrying == 0 {
            self.retrying.store(false, Ordering::Relaxed);
            // Only after a warning: a refusal nobody was told about needs no
            // all-clear either.
            if schedule.last_warning.is_some() {
                tracing::info!(
                    "Write-behind retry backlog cleared: the store accepted every refused entry"
                );
            }
            schedule.warning_armed = false;
            schedule.last_warning = None;
            schedule.example = None;
            return;
        }

        let due = schedule
            .last_warning
            .is_none_or(|last| now.saturating_duration_since(last) >= RETRY_WARN_INTERVAL);
        if !(schedule.warning_armed && due) {
            return;
        }
        let Some(example) = &schedule.example else {
            return;
        };
        // Whole seconds of the age gauge, so the line and the series agree.
        let oldest_refusal_age_s = oldest_refusal_age.as_secs();
        warn!(
            retrying,
            oldest_refusal_age_s,
            hold = schedule.hold_until.is_some(),
            map = %example.map,
            key = %example.key,
            attempts = example.attempts,
            error = %example.error,
            "Write-behind entries are being retried: the store refuses them"
        );
        schedule.last_warning = Some(now);
    }
}

/// Background task that periodically flushes eligible entries to the inner store.
///
/// Runs until the shutdown signal is received. Wakes on either the configured
/// interval or an explicit notify (from `soft_flush`).
async fn flush_loop(store: Arc<WriteBehindDataStore>, mut shutdown_rx: watch::Receiver<bool>) {
    let interval = tokio::time::Duration::from_millis(store.config.flush_interval_ms);
    let mut schedule = RetrySchedule::default();

    loop {
        // Wait for flush interval or early wake, checking shutdown
        tokio::select! {
            () = tokio::time::sleep(interval) => {}
            () = store.flush_notify.notified() => {}
            result = shutdown_rx.changed() => {
                if result.is_err() || *shutdown_rx.borrow() {
                    // Sender dropped or shutdown signalled -- exit loop
                    return;
                }
            }
        }

        // Check shutdown before doing work
        if *shutdown_rx.borrow() {
            return;
        }

        #[allow(clippy::cast_possible_wrap)]
        let deadline = now_millis().saturating_sub(store.config.write_delay_ms as i64);
        // Read once: every entry of the pass is judged against one instant.
        let now = tokio::time::Instant::now();

        #[cfg(test)]
        store.flush_pass_epoch.fetch_add(1, Ordering::SeqCst);
        store.run_flush_pass(&mut schedule, deadline, now).await;
        store.report_retries(&mut schedule);
        #[cfg(test)]
        store.flush_pass_epoch.fetch_add(1, Ordering::SeqCst);
    }
}

/// Background task that samples every tracked partition for a stalled applied
/// watermark and raises the two-class alarm.
///
/// Exits immediately when no WAL is configured: with no WAL no frame is ever
/// written, so there is no tracker, no watermark, and nothing to watch.
async fn watchdog_loop(store: Arc<WriteBehindDataStore>, mut shutdown_rx: watch::Receiver<bool>) {
    if store.wal.is_none() {
        return;
    }
    describe_wal_watermark_metrics();

    let tick = std::time::Duration::from_millis(store.config.watchdog_tick_ms());
    let bound = std::time::Duration::from_millis(store.config.wal_watermark_stall_bound_ms);

    loop {
        tokio::select! {
            () = tokio::time::sleep(tick) => {}
            result = shutdown_rx.changed() => {
                if result.is_err() || *shutdown_rx.borrow() {
                    return;
                }
            }
        }

        if *shutdown_rx.borrow() {
            return;
        }

        store.run_watchdog_pass(bound).await;
    }
}

// ---------------------------------------------------------------------------
// MapDataStore trait implementation -- Segment S1 (delegating methods)
// ---------------------------------------------------------------------------

#[async_trait]
impl MapDataStore for WriteBehindDataStore {
    async fn add(
        &self,
        map: &str,
        key: &str,
        value: &RecordValue,
        expiration_time: i64,
        now: i64,
    ) -> anyhow::Result<()> {
        // One write path, not two. Every caller without a per-op delta — every
        // LWW write, bulk and SYNC ingestion, cross-node merge, rehydration —
        // arrives here, and it is the same code the witness-bearing callers run
        // with the witness set. A second body would let the two drift.
        self.add_with_witness(
            map,
            key,
            WriteSource::Value(value),
            expiration_time,
            now,
            None,
        )
        .await
    }

    /// Whether the mutation point should build an OR witness for this store.
    ///
    /// Returns the store's SINGLE arming flag — the same field the framing
    /// branch in [`Self::add_with_witness`] tests. One flag, so the demand gate
    /// and the emission arm move together: armed means a witness is demanded,
    /// built, delivered and framed; unarmed means nothing upstream builds one at
    /// all, so the rollback costs the write path nothing rather than paying for
    /// a witness it then discards.
    fn wants_or_witness(&self) -> bool {
        self.config.or_delta_wal
    }

    /// # Precondition: one writer per key at a time
    ///
    /// The entry sequence is assigned and the entry is queued in two separate
    /// steps, with no lock held across both. Two concurrent writers of one
    /// `(map, key)` could therefore interleave so that the write with the
    /// higher sequence is queued first and the one with the lower sequence
    /// then replaces it: the queue would be left holding the lower-sequence
    /// write while staging keeps the higher one.
    ///
    /// This store's accounting (`TG-WB-005`) is proven for one writer per key
    /// at a time, and it is the callers that provide it. For record keys they
    /// do: every write that reaches `RecordStore::put`, `update_in_place` or
    /// `remove` holds the key's writer from
    /// `service::domain::key_writer::KeyWriterRegistry` (`TG-KEY-001`). A
    /// caller that writes this store directly, not through the record store,
    /// is outside that invariant and has to exclude a second writer of its
    /// key itself. `remove` and `remove_all` rest on the same precondition.
    ///
    /// One known exception: the fallback delete in
    /// `TombstoneFrontier::forget_client` runs on its caller's task while the
    /// cursor worker may be writing the same client's row. It has no
    /// production caller today (TODO-777).
    async fn add_with_witness(
        &self,
        map: &str,
        key: &str,
        src: WriteSource<'_>,
        expiration_time: i64,
        now: i64,
        witness: Option<&OrDelta>,
    ) -> anyhow::Result<()> {
        // Reject new writes once graceful drain is in progress so any out-of-order
        // arrival is visible as an error rather than silently lost. In the normal
        // shutdown sequence the HTTP server is already draining before the shutdown
        // state is set, making that arm effectively unreachable under orderly
        // shutdown. A failed store refuses for the opposite reason: the write
        // would be acked and then queued behind a flush task that is gone.
        match self.lifecycle.load(Ordering::Acquire) {
            Lifecycle::Running => {}
            Lifecycle::Failed => anyhow::bail!(
                "write-behind store has failed ({}); write rejected for map={map} key={key}",
                self.failed_reason()
            ),
            Lifecycle::ShuttingDown => anyhow::bail!(
                "write-behind store is shutting down; write rejected for map={map} key={key}"
            ),
        }

        // Check capacity before insertion to avoid partial state on rejection
        if self.config.capacity != 0 {
            let current = self.pending_count.load(Ordering::Relaxed);
            // A soft bound, not a reservation. The check reads staging, and a
            // key whose entry is held by a flush pass counts as known: its
            // staging slot is cleared only once the entry is persisted. The
            // check and the insert below are separate steps with nothing held
            // across them, and what `PartitionQueue::insert` returns is not
            // consulted.
            //
            // The bound that holds: `pending_count` can exceed `capacity` by
            // at most the number of entries a flush pass holds plus the number
            // of writers that are between this check and their insert. Every
            // write of a key that is in neither staging nor a flush pass is
            // rejected while `pending_count >= capacity`. A pass holds every
            // entry that was due when it began (`batch_size` has no effect), so the
            // first of those two terms is itself bounded only by what was
            // queued at that moment. The formula and its precondition are on
            // `WriteBehindConfig::capacity`.
            let is_new_key = !self
                .staging
                .contains_key(&(map.to_string(), key.to_string()));
            if is_new_key && current >= self.config.capacity {
                anyhow::bail!("Write-behind capacity exceeded");
            }
        }

        let partition_id = partition_for(map, key);
        // Seed before the first sequence of this partition is handed out, so the
        // watermark this write eventually advances already accounts for whatever
        // a prior incarnation left un-applied.
        self.ensure_wal_seeded(partition_id).await;
        let wal_seq = self.assign_wal_sequence(partition_id);

        // Append to WAL and satisfy the fsync policy before touching in-memory
        // state. This must happen before returning Ok(()) so a crash after ack
        // still has the write in the WAL for recovery to replay.
        // The WAL entry timestamp is the LWW idempotency-dedup key; OR values
        // carry their own per-entry timestamps inside the record, so it stays
        // None for them — including for the delta framing below, whose payload
        // is an OR mutation by construction.
        let wal_timestamp = src.with_value(|value| {
            if let RecordValue::Lww { timestamp, .. } = value {
                Some(timestamp.clone())
            } else {
                None
            }
        });
        // FAIL-CLOSED CONJUNCTION. The config's arming flag is the AUTHORITY —
        // one flag, the same field `wants_or_witness` returns — and the
        // witness is EVIDENCE, never authority. A `Some` witness with the
        // flag off is a caller that bypassed the demand gate, and it gets
        // today's full snapshot; keying the branch on the `Option` alone would
        // read no flag at all and make the store disagree with its own rollback
        // switch. Every combination other than (armed, `Some`) frames the full
        // record, so the failure mode of any divergence is LESS delta framing,
        // never an unexpected delta frame on disk.
        //
        // The snapshot arm persists the full `RecordValue` so OR-Map adds and
        // tombstones survive a kill -9 in the write-behind window — not just LWW
        // scalars. The delta arm does not shrink what is QUEUED or STORED: the
        // full value still rides the write-behind entry below, and only the WAL
        // frame differs. Nothing here diffs anything — the mutation point already
        // built this delta, and re-deriving it would be a second copy of the OR
        // algebra (`TG-OR-003`).
        //
        // Written as a guarded match rather than a boolean conjunction so the
        // witness's presence is established by the BINDING instead of re-derived
        // afterwards: an `if` on `witness.is_some()` leaves an unwrap on the far
        // side of the branch, and a later edit that weakens the guard turns that
        // unwrap into a live production panic on the write path.
        let wal_op = match witness {
            // Cloned into the frame rather than borrowed: this store defers its
            // inner-store write, so the caller's reference must not be held past
            // this call.
            Some(delta) if self.config.or_delta_wal => WalOp::OrDelta {
                delta: delta.clone(),
            },
            _ => WalOp::Store {
                value: WalStorePayload::Record(src.to_value()),
                expiration_time: if expiration_time == 0 {
                    None
                } else {
                    Some(expiration_time)
                },
            },
        };
        let wal_entry = WalEntry {
            map: map.to_string(),
            key: key.to_string(),
            op: wal_op,
            timestamp: wal_timestamp,
            sequence: wal_seq,
        };
        // Whether THIS write, as the survivor of a coalesce, makes a predecessor's
        // frame redundant. Read off the SURVIVOR, never the retired entry: a
        // survivor carrying complete state subsumes any predecessor, while a
        // survivor carrying only its own delta subsumes nothing regardless of what
        // it displaces.
        let survivor_subsumes = self.survivor_subsumes(&wal_entry.op);
        // A failed append leaves a tracked sequence that no frame bears and that
        // nothing can ever flush to resolve, so drop it before propagating.
        if let Err(err) = self.wal_append(partition_id, &wal_entry).await {
            self.rollback_wal_sequence(partition_id, wal_seq);
            return Err(err);
        }

        // Assign and track the durability sequence atomically (see
        // `assign_tracked_sequence`) BEFORE the entry enters the queue.
        let entry_seq = self.assign_tracked_sequence();
        #[cfg(test)]
        if let Some(gate) = TestParks::take_keyed(&self.test_parks.before_queue_insert, map, key) {
            gate.pass().await;
        }
        let (operation, staged) = queued_and_staged(src, expiration_time);
        let entry = DelayedEntry::fresh(map, key, operation, now, entry_seq, wal_seq);

        let staging_key = (map.to_string(), key.to_string());
        // Staged before the entry is queued: a flush pass can take the entry the
        // moment it is in the queue, and the terminal that persists it clears
        // the slot only if it finds it. From here to the queue insert there is
        // no await and no lock held, and it must stay that way (`TG-WB-007`).
        let (smap, skey) = staging_key;
        self.stage(&smap, &skey, entry_seq, Some(staged));
        #[cfg(test)]
        TestParks::pass_keyed(&self.test_parks.between_stage_and_queue_insert, map, key).await;
        // Insert into partition queue, preserving original store_time on coalesce
        let (retired_seq, subsumed_wal_seqs) = {
            let mut queue = self.queues.entry(partition_id).or_default();
            // If coalescing, preserve the original store_time
            if let Some(existing) = queue.value_mut().remove(map, key) {
                let retired = existing.sequence;
                let mut coalesced = entry;
                coalesced.inherit_from(&existing);
                let subsumed = Self::coalesce_wal_sequences(
                    survivor_subsumes,
                    &mut coalesced,
                    existing.wal_sequences,
                );
                let _ = queue.value_mut().insert(coalesced);
                // No pending_count change on coalesce
                (Some(retired), subsumed)
            } else {
                let _ = queue.value_mut().insert(entry);
                // New key -- increment pending count
                self.pending_count.fetch_add(1, Ordering::Relaxed);
                (None, None)
            }
        };

        // The frame is on disk and the entry is queued, so an entry now owns this
        // WAL sequence. Done after releasing the partition-queue lock so neither
        // tracking lock is ever nested under it.
        self.promote_wal_sequence(partition_id, wal_seq);

        // Retire the coalesced-away predecessor (its data is carried forward by
        // this write). Done after releasing the partition-queue lock so the
        // pending-set lock is never nested under it.
        if let Some(retired) = retired_seq {
            self.resolve_pending(retired);
        }

        // The retired frame is subsumed by the survivor's, so nothing is lost by
        // letting the pending prefix past it. Sited after the queue guard drops,
        // like every other tracking call.
        if let Some(subsumed) = subsumed_wal_seqs {
            self.resolve_wal_sequences(partition_id, &subsumed);
        }

        Ok(())
    }

    async fn add_backup(
        &self,
        map: &str,
        key: &str,
        value: &RecordValue,
        expiration_time: i64,
        now: i64,
    ) -> anyhow::Result<()> {
        // Backups pass through directly -- backup replication has its own
        // consistency guarantees
        self.inner
            .add_backup(map, key, value, expiration_time, now)
            .await
    }

    async fn remove(&self, map: &str, key: &str, now: i64) -> anyhow::Result<()> {
        // Reject once graceful drain begins or the store has failed — same
        // reasoning as `add`.
        match self.lifecycle.load(Ordering::Acquire) {
            Lifecycle::Running => {}
            Lifecycle::Failed => anyhow::bail!(
                "write-behind store has failed ({}); write rejected for map={map} key={key}",
                self.failed_reason()
            ),
            Lifecycle::ShuttingDown => anyhow::bail!(
                "write-behind store is shutting down; remove rejected for map={map} key={key}"
            ),
        }

        // The same soft bound as in `add_with_witness`, where it is described.
        if self.config.capacity != 0 {
            let current = self.pending_count.load(Ordering::Relaxed);
            let is_new_key = !self
                .staging
                .contains_key(&(map.to_string(), key.to_string()));
            if is_new_key && current >= self.config.capacity {
                anyhow::bail!("Write-behind capacity exceeded");
            }
        }

        let partition_id = partition_for(map, key);
        // Seed before the first sequence of this partition is handed out, so the
        // watermark this write eventually advances already accounts for whatever
        // a prior incarnation left un-applied.
        self.ensure_wal_seeded(partition_id).await;

        let wal_seq = self.assign_wal_sequence(partition_id);

        // Append to WAL before updating in-memory state so crash recovery can
        // replay the tombstone if the process dies after ack but before flush. A
        // Remove frame carries no timestamp: replay deletes unconditionally, exactly
        // as this live path does (see `WalRecovery::replay_entry`, TG-WAL-009).
        let wal_entry = WalEntry {
            map: map.to_string(),
            key: key.to_string(),
            op: WalOp::Remove,
            timestamp: None,
            sequence: wal_seq,
        };
        // Evaluated on the SURVIVOR — this write — never on what it displaces.
        let survivor_subsumes = self.survivor_subsumes(&wal_entry.op);
        // A failed append leaves a tracked sequence no frame bears — drop it.
        if let Err(err) = self.wal_append(partition_id, &wal_entry).await {
            self.rollback_wal_sequence(partition_id, wal_seq);
            return Err(err);
        }

        // Assign and track the durability sequence atomically before queuing.
        let entry_seq = self.assign_tracked_sequence();
        let entry = DelayedEntry::fresh(map, key, DelayedOp::Remove, now, entry_seq, wal_seq);

        let staging_key = (map.to_string(), key.to_string());
        // The pending delete is staged before the entry is queued: a flush pass
        // can take the entry the moment it is in the queue, and the terminal
        // that persists it clears the slot only if it finds it. From here to the
        // queue insert there is no await and no lock held, and it must stay that
        // way (`TG-WB-007`).
        let (smap, skey) = staging_key;
        self.stage(&smap, &skey, entry_seq, None);
        #[cfg(test)]
        TestParks::pass_keyed(&self.test_parks.between_stage_and_queue_insert, map, key).await;
        let (retired_seq, subsumed_wal_seqs) = {
            let mut queue = self.queues.entry(partition_id).or_default();
            if let Some(existing) = queue.value_mut().remove(map, key) {
                let retired = existing.sequence;
                let mut coalesced = entry;
                coalesced.inherit_from(&existing);
                let subsumed = Self::coalesce_wal_sequences(
                    survivor_subsumes,
                    &mut coalesced,
                    existing.wal_sequences,
                );
                let _ = queue.value_mut().insert(coalesced);
                // No pending_count change on coalesce
                (Some(retired), subsumed)
            } else {
                let _ = queue.value_mut().insert(entry);
                self.pending_count.fetch_add(1, Ordering::Relaxed);
                (None, None)
            }
        };

        // An entry now owns this WAL sequence; sited after the queue guard drops.
        self.promote_wal_sequence(partition_id, wal_seq);

        if let Some(retired) = retired_seq {
            self.resolve_pending(retired);
        }

        // The survivor's frame subsumes the retired one, so its sequences resolve
        // now rather than being dropped and stranding the watermark.
        if let Some(subsumed) = subsumed_wal_seqs {
            self.resolve_wal_sequences(partition_id, &subsumed);
        }

        #[cfg(test)]
        if let Some(gate) = TestParks::take_keyed(&self.test_parks.after_remove, map, key) {
            gate.pass().await;
        }

        Ok(())
    }

    async fn remove_backup(&self, map: &str, key: &str, now: i64) -> anyhow::Result<()> {
        // Backups pass through directly
        self.inner.remove_backup(map, key, now).await
    }

    async fn load(&self, map: &str, key: &str) -> anyhow::Result<Option<RecordValue>> {
        let staging_key = (map.to_string(), key.to_string());

        // Check staging first for read-your-writes consistency
        if let Some(entry) = self.staging.get(&staging_key) {
            return match &entry.value().value {
                Some(value) => Ok(Some(value.to_value())),
                // Pending delete -- do not consult inner store
                None => Ok(None),
            };
        }

        // Not in staging -- delegate to inner store
        self.inner.load(map, key).await
    }

    /// Like [`Self::load`], but a staged in-place write answers its cell rather
    /// than a copy, so the engine re-adopts the one cell the queued entry and
    /// the staging slot already hold (TG-OR-007: the caller inserts it only
    /// through the generation-checked insert).
    async fn load_slot(&self, map: &str, key: &str) -> anyhow::Result<Option<Loaded>> {
        let staging_key = (map.to_string(), key.to_string());

        let staged = self
            .staging
            .get(&staging_key)
            .map(|entry| match &entry.value().value {
                Some(StagedValue::Cell(cell)) => Some(Loaded::Cell(Arc::clone(cell))),
                Some(StagedValue::Value(value)) => Some(Loaded::Value(value.clone())),
                // Pending delete -- do not consult inner store
                None => None,
            });
        if let Some(answer) = staged {
            #[cfg(test)]
            if matches!(answer, Some(Loaded::Cell(_))) {
                if let Some(gate) = TestParks::take_keyed(&self.test_parks.load_slot, map, key) {
                    self.test_parks
                        .load_slot_hits
                        .fetch_add(1, Ordering::Relaxed);
                    gate.pass().await;
                }
            }
            return Ok(answer);
        }

        self.inner.load_slot(map, key).await
    }

    async fn load_all(
        &self,
        map: &str,
        keys: &[String],
    ) -> anyhow::Result<Vec<(String, RecordValue)>> {
        let mut results = Vec::new();
        let mut inner_keys = Vec::new();

        for key in keys {
            let staging_key = (map.to_string(), key.clone());
            if let Some(entry) = self.staging.get(&staging_key) {
                if let Some(value) = &entry.value().value {
                    results.push((key.clone(), value.to_value()));
                }
                // Pending delete (None) -- skip entirely, do not fetch from inner
            } else {
                inner_keys.push(key.clone());
            }
        }

        // Batch-load remaining keys from the inner store
        if !inner_keys.is_empty() {
            let inner_results = self.inner.load_all(map, &inner_keys).await?;
            results.extend(inner_results);
        }

        Ok(results)
    }

    async fn remove_all(&self, map: &str, keys: &[String]) -> anyhow::Result<()> {
        // Reject new writes once graceful drain is in progress so any out-of-order
        // arrival is visible as an error rather than silently lost, matching the
        // add/remove guards. Effectively unreachable under orderly shutdown (the
        // HTTP server drains before the shutdown state is set). A failed store
        // refuses for the reason `add` gives.
        match self.lifecycle.load(Ordering::Acquire) {
            Lifecycle::Running => {}
            Lifecycle::Failed => anyhow::bail!(
                "write-behind store has failed ({}); write rejected for map={map}",
                self.failed_reason()
            ),
            Lifecycle::ShuttingDown => anyhow::bail!(
                "write-behind store is shutting down; remove_all rejected for map={map}"
            ),
        }

        let now = now_millis();

        for key in keys {
            // The same soft bound as in `add_with_witness`, checked per key.
            if self.config.capacity != 0 {
                let current = self.pending_count.load(Ordering::Relaxed);
                let is_new_key = !self.staging.contains_key(&(map.to_string(), key.clone()));
                if is_new_key && current >= self.config.capacity {
                    anyhow::bail!("Write-behind capacity exceeded");
                }
            }

            let partition_id = partition_for(map, key);
            // Seed before the first sequence of this partition is handed out, so
            // the watermark this write eventually advances already accounts for
            // whatever a prior incarnation left un-applied.
            self.ensure_wal_seeded(partition_id).await;

            let wal_seq = self.assign_wal_sequence(partition_id);

            // Append each tombstone to the WAL before queuing so crash recovery
            // can replay every individual key deletion, not just the batch call. A
            // Remove frame carries no timestamp: replay deletes unconditionally, as
            // this live path does (see `WalRecovery::replay_entry`, TG-WAL-009).
            let wal_entry = WalEntry {
                map: map.to_string(),
                key: key.clone(),
                op: WalOp::Remove,
                timestamp: None,
                sequence: wal_seq,
            };
            // Evaluated on the SURVIVOR — this key's write — never on what it
            // displaces in the queue.
            let survivor_subsumes = self.survivor_subsumes(&wal_entry.op);
            // Roll back ONLY the key whose own append failed. The earlier keys'
            // sequences are frame-backed and must stay pending — rolling those
            // back would let the watermark advance past frames never applied.
            if let Err(err) = self.wal_append(partition_id, &wal_entry).await {
                self.rollback_wal_sequence(partition_id, wal_seq);
                return Err(err);
            }

            // Assign and track the durability sequence atomically before queuing.
            let entry_seq = self.assign_tracked_sequence();
            let entry = DelayedEntry::fresh(map, key, DelayedOp::Remove, now, entry_seq, wal_seq);

            let staging_key = (map.to_string(), key.clone());
            // The pending delete is staged before the entry is queued: a flush pass
            // can take the entry the moment it is in the queue, and the terminal
            // that persists it clears the slot only if it finds it. From here to the
            // queue insert there is no await and no lock held, and it must stay that
            // way (`TG-WB-007`).
            let (smap, skey) = staging_key;
            self.stage(&smap, &skey, entry_seq, None);
            #[cfg(test)]
            TestParks::pass_keyed(&self.test_parks.between_stage_and_queue_insert, map, key).await;
            let (retired_seq, subsumed_wal_seqs) = {
                let mut queue = self.queues.entry(partition_id).or_default();
                if let Some(existing) = queue.value_mut().remove(map, key) {
                    let retired = existing.sequence;
                    let mut coalesced = entry;
                    coalesced.inherit_from(&existing);
                    let subsumed = Self::coalesce_wal_sequences(
                        survivor_subsumes,
                        &mut coalesced,
                        existing.wal_sequences,
                    );
                    let _ = queue.value_mut().insert(coalesced);
                    (Some(retired), subsumed)
                } else {
                    let _ = queue.value_mut().insert(entry);
                    self.pending_count.fetch_add(1, Ordering::Relaxed);
                    (None, None)
                }
            };

            // An entry now owns this WAL sequence; sited after the queue guard drops.
            self.promote_wal_sequence(partition_id, wal_seq);

            // Retire the coalesced-away predecessor so its durability sequence does
            // not stall the flushed watermark forever (a leaked pending sequence
            // would permanently pin the prune). Done after the queue lock is
            // released so the pending-set lock is never nested under it.
            if let Some(retired) = retired_seq {
                self.resolve_pending(retired);
            }

            // The survivor's frame subsumes the retired one, so its sequences
            // resolve now rather than being dropped and stranding the watermark.
            if let Some(subsumed) = subsumed_wal_seqs {
                self.resolve_wal_sequences(partition_id, &subsumed);
            }
        }

        Ok(())
    }

    async fn list_maps(&self) -> anyhow::Result<Vec<String>> {
        // Start from the durable catalog so a map persisted before a restart is
        // discovered even when not resident.
        let mut names = self.inner.list_maps().await?;

        // Union in maps whose only write is still buffered in staging. A buffered
        // record used to be necessarily resident too, so the durable catalog
        // covered it; clean-marking made a buffered record evictable, so a map
        // whose sole write is buffered-and-evicted is now reachable only via
        // staging — it must be surfaced here or the residency-independent seed
        // would miss it. A map present in staging only via pending deletes
        // (None) contributes nothing.
        let mut staging_only: Vec<String> = self
            .staging
            .iter()
            .filter(|entry| entry.value().value.is_some())
            .map(|entry| entry.key().0.clone())
            .collect();
        staging_only.sort();
        staging_only.dedup();
        for name in staging_only {
            if !names.contains(&name) {
                names.push(name);
            }
        }
        names.sort();
        names.dedup();
        Ok(names)
    }

    async fn enumerate_leaves(
        &self,
        map: &str,
        is_backup: bool,
        sink: &mut dyn LeafSink,
    ) -> anyhow::Result<()> {
        // Backup scans carry no staging overlay: add_backup/remove_backup go
        // straight to inner, so staging holds only non-backup writes. Overlaying
        // would double-count or falsely surface a primary write onto a backup
        // tree.
        if is_backup {
            return self.inner.enumerate_leaves(map, is_backup, sink).await;
        }

        // Collect the map's pending staging set upfront so the durable
        // enumeration can be overlaid in a single pass. The buffer is bounded by
        // TOPGUN_WRITEBEHIND_CAPACITY, so this stays O(buffer), not O(map).
        let pending = self.collect_staging_for_map(map);

        // Overlay the durable leaves with the buffered state: a buffered Some
        // replaces the durable leaf hash (recomputed from the buffered value),
        // and a buffered None (pending delete) suppresses the durable leaf so an
        // evicted-but-deleted key cannot resurrect. The sink is invoked through
        // an overlay adapter so peak memory stays bounded exactly as the durable
        // path bounds it.
        let mut overlay = StagingLeafSink::new(&pending, sink);
        self.inner
            .enumerate_leaves(map, is_backup, &mut overlay)
            .await?;

        // Emit staging-only leaves (buffered Some keys the durable store never
        // had) after the durable enumeration completes. merkle_leaf_hash returns
        // None for OrMap entries that must contribute no leaf; propagate that
        // None so a rebuilt root stays byte-identical to the live root.
        let mut extra: Vec<MerkleLeaf> = Vec::new();
        for (key, value) in &pending {
            if let Some(value) = value {
                if !overlay.was_seen(key) {
                    if let Some((kind, leaf_hash)) = merkle_leaf_hash(key, value) {
                        extra.push(MerkleLeaf {
                            key: key.clone(),
                            kind,
                            leaf_hash,
                        });
                    }
                }
            }
        }
        if !extra.is_empty() {
            sink.consume(extra).await?;
        }
        Ok(())
    }

    async fn scan_values(
        &self,
        map: &str,
        is_backup: bool,
        max_batch_cost: u64,
    ) -> anyhow::Result<ScanBatch> {
        // Backup scans never carry staging overlay (see enumerate_leaves).
        if is_backup {
            return self.inner.scan_values(map, is_backup, max_batch_cost).await;
        }

        // The snapshot is taken BEFORE the inner read. A flush makes the row
        // durable and only then clears the slot, so in this order a key is in
        // the snapshot or in the batch; the other order loses a key flushed
        // between the two reads (TG-WB-008).
        let pending = self.collect_staging_for_map(map);
        let ranged = !pending.is_empty() && self.inner.scans_in_key_order();
        let batch = self
            .inner
            .scan_values(map, is_backup, max_batch_cost)
            .await?;
        if !ranged {
            // Over a store that does not declare key order the first batch
            // takes every buffered key that has no row in it. A buffered key
            // whose row is in a later batch is then emitted twice (TODO-819).
            return Ok(overlay_scan_batch(batch, &pending, &StagingOnly::Every));
        }
        // Key-ordered store: the first batch covers the keys up to its cursor,
        // or every key when it is the only batch (TG-WB-008).
        let upper = batch.next_cursor.as_ref().map(cursor_key).transpose()?;
        let range = StagingOnly::InRange {
            lower: None,
            upper: upper.as_deref(),
        };
        Ok(overlay_scan_batch(batch, &pending, &range))
    }

    async fn scan_values_batched(
        &self,
        map: &str,
        is_backup: bool,
        cursor: ScanCursor,
        max_batch_cost: u64,
    ) -> anyhow::Result<ScanBatch> {
        // Backup scans never carry staging overlay (see enumerate_leaves).
        if is_backup {
            return self
                .inner
                .scan_values_batched(map, is_backup, cursor, max_batch_cost)
                .await;
        }

        // Snapshot before the inner read, for the reason given in
        // `scan_values`: the other order loses a key flushed between the two.
        let pending = self.collect_staging_for_map(map);
        let ranged = !pending.is_empty() && self.inner.scans_in_key_order();
        // The lower bound is read from the very bytes the previous batch
        // returned as its upper bound, so consecutive ranges share an edge.
        let lower = if ranged {
            Some(cursor_key(&cursor)?)
        } else {
            None
        };
        let batch = self
            .inner
            .scan_values_batched(map, is_backup, cursor, max_batch_cost)
            .await?;
        let Some(lower) = lower else {
            // Over a store that does not declare key order a resumed batch only
            // replaces and drops rows: the first batch took the buffered keys
            // without a row there, including those whose row is in this batch,
            // which are therefore emitted twice (TODO-819).
            return Ok(overlay_scan_batch(batch, &pending, &StagingOnly::Nothing));
        };
        // Key-ordered store: this batch covers the keys above the cursor it was
        // resumed from, up to the cursor it returns (TG-WB-008).
        let upper = batch.next_cursor.as_ref().map(cursor_key).transpose()?;
        // A key-ordered store never returns a cursor below the one it was
        // resumed from. One that does has broken its declaration, and the range
        // would have no meaning: fail the call, as for a cursor that is no key.
        if upper.as_deref().is_some_and(|upper| upper < lower.as_str()) {
            anyhow::bail!("scan cursor is below the cursor it was resumed from");
        }
        let range = StagingOnly::InRange {
            lower: Some(&lower),
            upper: upper.as_deref(),
        };
        Ok(overlay_scan_batch(batch, &pending, &range))
    }

    fn is_loadable(&self, _key: &str) -> bool {
        // Staging area handles consistency, so always loadable
        true
    }

    fn pending_operation_count(&self) -> u64 {
        self.pending_count.load(Ordering::Relaxed)
    }

    async fn soft_flush(&self) -> anyhow::Result<u64> {
        // Notify the background task to flush immediately
        self.flush_notify.notify_one();
        Ok(self.sequence.load(Ordering::Relaxed))
    }

    fn assigned_write_sequence(&self) -> u64 {
        // One past the highest sequence handed out — an upper bound on any
        // in-flight write's sequence. The tombstone frontier snapshots this at
        // stamp time; because the tombstone's own byte-write was enqueued (and
        // thus assigned a sequence) strictly before the stamp, this is `>=` it.
        self.sequence.load(Ordering::Relaxed)
    }

    fn flushed_watermark(&self) -> u64 {
        // Prefix-complete: the smallest still-pending sequence is the first
        // un-resolved write; every sequence below it is durable (flushed) or
        // retired (coalesced away). With nothing pending, all assigned writes
        // are resolved, so the watermark is the full assigned counter. This can
        // NEVER expose a value above an un-flushed sequence — the property that
        // makes `stamped_seq <= flushed_watermark()` a sound durability fence.
        let pending = self.pending_seqs();
        match pending.iter().next().copied() {
            Some(min_pending) => min_pending,
            None => self.sequence.load(Ordering::Relaxed),
        }
    }

    async fn hard_flush(&self) -> anyhow::Result<()> {
        // Mark the store as shutting down so any straggler writes are rejected
        // rather than queued behind the drain. From `Running` only: a store that
        // has already failed stays failed, so its writers keep being told why.
        self.lifecycle.begin_shutdown();

        #[cfg(test)]
        {
            let gate = self
                .test_parks
                .after_shutdown_flag
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            if let Some(gate) = gate {
                gate.pass().await;
            }
        }

        // Stop the background flush loop so it doesn't race with our direct drain.
        let _ = self.shutdown.send(true);

        let timeout_duration = tokio::time::Duration::from_millis(self.config.shutdown_timeout_ms);
        let deadline = tokio::time::Instant::now() + timeout_duration;

        self.join_flush_loop(deadline).await;

        let drained_entries = self.drain_for_shutdown(deadline).await;
        self.finish_shutdown(drained_entries).await
    }

    fn is_null(&self) -> bool {
        false
    }
}

// ---------------------------------------------------------------------------
// Staging overlay helpers for the buffer-aware read surface
// ---------------------------------------------------------------------------

/// Which buffered keys that have no durable row in a batch are appended to it.
enum StagingOnly<'a> {
    /// None: a resumed batch over a store that does not declare key order.
    Nothing,
    /// All of them: the first batch over such a store.
    Every,
    /// Those in the batch's key range `lower < key <= upper`; an absent bound is
    /// open. Used over a store that declares key order.
    InRange {
        lower: Option<&'a str>,
        upper: Option<&'a str>,
    },
}

/// The key a key-ordered store put into a scan cursor.
fn cursor_key(cursor: &ScanCursor) -> anyhow::Result<String> {
    String::from_utf8(cursor.0.clone())
        .map_err(|e| anyhow::anyhow!("scan cursor is not valid UTF-8: {e}"))
}

/// Overlay a map's pending staging set onto one durable [`ScanBatch`].
///
/// For each durable row: a buffered `Some(v)` replaces the row with the newer
/// buffered value; a buffered `None` (pending delete) suppresses the row so an
/// evicted-but-deleted key cannot resurrect; an absent staging entry keeps the
/// durable row unchanged. Buffered values with no row in the batch are then
/// appended, in ascending key order, as `staging_only` says; a pending delete
/// is appended nowhere.
///
/// Over a store that declares key order (`TG-WB-008`) the ranges of the batches
/// of one scan tile the key space — each starts at the cursor the previous one
/// ended at — so a buffered key is appended by exactly one batch, and never by
/// a batch that also replaced its row.
///
/// Over any other store the first batch appends every buffered key without a
/// row in it and later batches append none. That rule is known to emit twice a
/// buffered key whose durable row is in a later batch (TODO-819).
fn overlay_scan_batch(
    batch: ScanBatch,
    pending: &BTreeMap<String, Option<RecordValue>>,
    staging_only: &StagingOnly<'_>,
) -> ScanBatch {
    if pending.is_empty() {
        return batch;
    }

    let ScanBatch {
        records,
        next_cursor,
    } = batch;

    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out: Vec<(String, RecordValue)> = Vec::with_capacity(records.len());
    for (key, value) in records {
        match pending.get(&key) {
            Some(Some(buffered)) => {
                // Buffered write is newer than the durable row; emit it instead.
                seen.insert(key.clone());
                out.push((key, buffered.clone()));
            }
            Some(None) => {
                // Pending delete hides the durable value.
            }
            None => {
                out.push((key, value));
            }
        }
    }

    let mut append = |key: &String, value: &Option<RecordValue>| {
        if let Some(value) = value {
            if !seen.contains(key) {
                out.push((key.clone(), value.clone()));
            }
        }
    };
    match *staging_only {
        StagingOnly::Nothing => {}
        StagingOnly::Every => {
            for (key, value) in pending {
                append(key, value);
            }
        }
        StagingOnly::InRange { lower, upper } => {
            let bounds = (
                lower.map_or(Bound::Unbounded, Bound::Excluded),
                upper.map_or(Bound::Unbounded, Bound::Included),
            );
            for (key, value) in pending.range::<str, _>(bounds) {
                append(key, value);
            }
        }
    }

    ScanBatch {
        records: out,
        next_cursor,
    }
}

/// A [`LeafSink`] adapter that overlays a map's pending staging set onto the
/// durable leaf stream before forwarding to the real sink.
///
/// A buffered `Some` recomputes the leaf hash from the buffered value (so a
/// buffered-then-flushed record yields an identical leaf, leaving the Merkle
/// root unchanged); a buffered `None` suppresses the durable leaf. Keys it has
/// emitted are tracked so the outer enumeration can append the staging-only
/// leaves exactly once.
struct StagingLeafSink<'a> {
    pending: &'a BTreeMap<String, Option<RecordValue>>,
    inner: &'a mut dyn LeafSink,
    seen: std::collections::HashSet<String>,
}

impl<'a> StagingLeafSink<'a> {
    fn new(
        pending: &'a BTreeMap<String, Option<RecordValue>>,
        inner: &'a mut dyn LeafSink,
    ) -> Self {
        Self {
            pending,
            inner,
            seen: std::collections::HashSet::new(),
        }
    }

    fn was_seen(&self, key: &str) -> bool {
        self.seen.contains(key)
    }
}

#[async_trait]
impl LeafSink for StagingLeafSink<'_> {
    async fn consume(&mut self, batch: Vec<MerkleLeaf>) -> anyhow::Result<()> {
        let mut out: Vec<MerkleLeaf> = Vec::with_capacity(batch.len());
        for leaf in batch {
            match self.pending.get(&leaf.key) {
                Some(Some(buffered)) => {
                    self.seen.insert(leaf.key.clone());
                    // merkle_leaf_hash returns None for OrMap entries that must
                    // contribute no leaf; propagate that None (emit nothing)
                    // rather than injecting a zero/placeholder leaf.
                    if let Some((kind, leaf_hash)) = merkle_leaf_hash(&leaf.key, buffered) {
                        out.push(MerkleLeaf {
                            key: leaf.key,
                            kind,
                            leaf_hash,
                        });
                    }
                }
                Some(None) => {
                    // Pending delete suppresses the durable leaf.
                }
                None => {
                    out.push(leaf);
                }
            }
        }
        if out.is_empty() {
            return Ok(());
        }
        self.inner.consume(out).await
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use topgun_core::hlc::Timestamp;
    use topgun_core::types::Value;

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn dummy_value() -> RecordValue {
        RecordValue::Lww {
            value: Value::Null,
            timestamp: Timestamp {
                millis: 0,
                counter: 0,
                node_id: String::new(),
            },
        }
    }

    fn dummy_value_with(millis: u64) -> RecordValue {
        RecordValue::Lww {
            value: Value::Null,
            timestamp: Timestamp {
                millis,
                counter: 0,
                node_id: String::new(),
            },
        }
    }

    fn short_delay_config() -> WriteBehindConfig {
        WriteBehindConfig {
            write_delay_ms: 10,
            flush_interval_ms: 10,
            batch_size: 100,
            backoff_base_ms: 10,
            backoff_cap_ms: 100,
            capacity: 0,
            shutdown_timeout_ms: 5_000,
            ..WriteBehindConfig::default()
        }
    }

    // -----------------------------------------------------------------------
    // SpyDataStore
    // -----------------------------------------------------------------------

    /// Records which calls were made to the inner store, for test assertions.
    #[derive(Debug, Clone)]
    #[allow(dead_code)]
    enum SpyCall {
        Add { map: String, key: String },
        Remove { map: String, key: String },
    }

    /// One inner-store call of a flush as the store saw it: which key, when the
    /// call began, when it answered and whether it was accepted.
    #[derive(Debug, Clone)]
    struct SpyTimedCall {
        key: String,
        began: tokio::time::Instant,
        returned: Option<tokio::time::Instant>,
        accepted: bool,
    }

    /// Test-only data store that records calls and optionally returns pre-seeded values.
    struct SpyDataStore {
        calls: Arc<Mutex<Vec<SpyCall>>>,
        /// Pre-seeded values returned by `load()`.
        seeded: DashMap<(String, String), RecordValue>,
        /// When set, `add` returns `Err` — simulates an inner-store durability
        /// failure so the flush error path can be exercised.
        fail_add: std::sync::atomic::AtomicBool,
        /// Optional handshake: on the first `add`, signal `entered` then park on
        /// `proceed` before returning. Lets a test read the outer store's
        /// `flushed_watermark()` while a flush's inner add is in flight.
        add_gate: Mutex<Option<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>>,
        /// What `add()` was actually asked to persist, kept apart from `seeded`
        /// so no existing expectation about `load()` changes.
        persisted: DashMap<(String, String), RecordValue>,
        /// When set, `remove` returns `Err`.
        fail_remove: std::sync::atomic::AtomicBool,
        /// Keys whose `add` and `remove` return `Err` while every other key is
        /// served: a record the store rejects next to a store that works.
        refused_keys: Mutex<std::collections::HashSet<String>>,
        /// How long `add` and `remove` take to answer. Slept on the runtime's
        /// clock, so a paused clock drives it.
        answer_delay_ms: AtomicU64,
        /// Every `add` and `remove`, in the order they began.
        timeline: Mutex<Vec<SpyTimedCall>>,
        in_flight: std::sync::atomic::AtomicUsize,
        most_in_flight: std::sync::atomic::AtomicUsize,
        /// When set, an accepted `add` is kept and an accepted `remove` deletes,
        /// so `load()` answers with what the store was actually given. Off by
        /// default, so no existing expectation about `load()` changes.
        retain_accepted: bool,
    }

    impl SpyDataStore {
        fn new() -> Self {
            Self {
                calls: Arc::new(Mutex::new(Vec::new())),
                seeded: DashMap::new(),
                fail_add: std::sync::atomic::AtomicBool::new(false),
                add_gate: Mutex::new(None),
                persisted: DashMap::new(),
                fail_remove: std::sync::atomic::AtomicBool::new(false),
                refused_keys: Mutex::new(std::collections::HashSet::new()),
                answer_delay_ms: AtomicU64::new(0),
                timeline: Mutex::new(Vec::new()),
                in_flight: std::sync::atomic::AtomicUsize::new(0),
                most_in_flight: std::sync::atomic::AtomicUsize::new(0),
                retain_accepted: false,
            }
        }

        /// A store that keeps what it accepts: `load()` and `held()` answer
        /// with the last accepted value of a key, as a real store would.
        fn retaining() -> Self {
            Self {
                retain_accepted: true,
                ..Self::new()
            }
        }

        /// Refuse, or stop refusing, every `add` and every `remove`.
        fn refuse_everything(&self, refuse: bool) {
            self.set_fail_add(refuse);
            self.fail_remove
                .store(refuse, std::sync::atomic::Ordering::Relaxed);
        }

        /// Refuse every call for `key` from now on, whatever the map.
        fn refuse_key(&self, key: &str) {
            self.refused_keys.lock().unwrap().insert(key.to_string());
        }

        /// Lift every per-key refusal.
        fn accept_every_key(&self) {
            self.refused_keys.lock().unwrap().clear();
        }

        fn set_answer_delay_ms(&self, delay_ms: u64) {
            self.answer_delay_ms
                .store(delay_ms, std::sync::atomic::Ordering::Relaxed);
        }

        /// Every `add` and `remove` so far, in the order they began.
        fn timeline(&self) -> Vec<SpyTimedCall> {
            self.timeline.lock().unwrap().clone()
        }

        fn call_count(&self) -> usize {
            self.timeline.lock().unwrap().len()
        }

        fn calls_for(&self, key: &str) -> usize {
            self.timeline
                .lock()
                .unwrap()
                .iter()
                .filter(|call| call.key == key)
                .count()
        }

        fn accepted_for(&self, key: &str) -> usize {
            self.timeline
                .lock()
                .unwrap()
                .iter()
                .filter(|call| call.key == key && call.accepted)
                .count()
        }

        /// What a retaining store holds for a key: the last accepted value.
        fn held(&self, map: &str, key: &str) -> Option<RecordValue> {
            self.seeded
                .get(&(map.to_string(), key.to_string()))
                .map(|v| v.value().clone())
        }

        /// The largest number of calls that were unanswered at one instant.
        fn most_in_flight(&self) -> usize {
            self.most_in_flight
                .load(std::sync::atomic::Ordering::SeqCst)
        }

        /// Records that a call began and returns its place in the timeline.
        fn call_began(&self, key: &str) -> usize {
            let now_in_flight = self
                .in_flight
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                + 1;
            self.most_in_flight
                .fetch_max(now_in_flight, std::sync::atomic::Ordering::SeqCst);
            let mut timeline = self.timeline.lock().unwrap();
            timeline.push(SpyTimedCall {
                key: key.to_string(),
                began: tokio::time::Instant::now(),
                returned: None,
                accepted: false,
            });
            timeline.len() - 1
        }

        fn call_returned(&self, index: usize, accepted: bool) {
            self.in_flight
                .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            let mut timeline = self.timeline.lock().unwrap();
            timeline[index].returned = Some(tokio::time::Instant::now());
            timeline[index].accepted = accepted;
        }

        async fn answer_delay(&self) {
            let delay_ms = self
                .answer_delay_ms
                .load(std::sync::atomic::Ordering::Relaxed);
            if delay_ms > 0 {
                tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
            }
        }

        fn refuses_key(&self, key: &str) -> bool {
            self.refused_keys.lock().unwrap().contains(key)
        }

        /// The value the inner store was handed for a key, if any.
        fn persisted(&self, map: &str, key: &str) -> Option<RecordValue> {
            self.persisted
                .get(&(map.to_string(), key.to_string()))
                .map(|v| v.value().clone())
        }

        fn calls(&self) -> Arc<Mutex<Vec<SpyCall>>> {
            Arc::clone(&self.calls)
        }

        /// Pre-seed a value so `load()` returns it (simulates persisted data).
        fn seed(&self, map: &str, key: &str, value: RecordValue) {
            self.seeded
                .insert((map.to_string(), key.to_string()), value);
        }

        /// Make every subsequent `add` fail (inner-store durability failure).
        fn set_fail_add(&self, fail: bool) {
            self.fail_add
                .store(fail, std::sync::atomic::Ordering::Relaxed);
        }

        /// Install a one-shot gate on `add`. Returns `(entered, proceed)`: the
        /// store fires `entered` when it reaches the gated `add` and then awaits
        /// `proceed` before returning.
        fn install_add_gate(&self) -> (Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>) {
            let entered = Arc::new(tokio::sync::Notify::new());
            let proceed = Arc::new(tokio::sync::Notify::new());
            *self.add_gate.lock().unwrap() = Some((Arc::clone(&entered), Arc::clone(&proceed)));
            (entered, proceed)
        }
    }

    #[async_trait]
    impl MapDataStore for SpyDataStore {
        async fn add(
            &self,
            map: &str,
            key: &str,
            value: &RecordValue,
            _expiration_time: i64,
            _now: i64,
        ) -> anyhow::Result<()> {
            self.calls.lock().unwrap().push(SpyCall::Add {
                map: map.to_string(),
                key: key.to_string(),
            });
            self.persisted
                .insert((map.to_string(), key.to_string()), value.clone());
            let index = self.call_began(key);
            // If a gate is installed, consume it: signal that the add is now in
            // flight, then park until the test lets it proceed.
            let gate = self.add_gate.lock().unwrap().take();
            if let Some((entered, proceed)) = gate {
                entered.notify_one();
                proceed.notified().await;
            }
            self.answer_delay().await;
            // Read when the call answers, not when it began: a refusal lifted
            // while a slow call is in flight is an accepted call.
            let refused =
                self.fail_add.load(std::sync::atomic::Ordering::Relaxed) || self.refuses_key(key);
            self.call_returned(index, !refused);
            if refused {
                anyhow::bail!("SpyDataStore: simulated inner-store add failure");
            }
            if self.retain_accepted {
                self.seeded
                    .insert((map.to_string(), key.to_string()), value.clone());
            }
            Ok(())
        }

        async fn add_backup(
            &self,
            _map: &str,
            _key: &str,
            _value: &RecordValue,
            _expiration_time: i64,
            _now: i64,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn remove(&self, map: &str, key: &str, _now: i64) -> anyhow::Result<()> {
            self.calls.lock().unwrap().push(SpyCall::Remove {
                map: map.to_string(),
                key: key.to_string(),
            });
            let index = self.call_began(key);
            self.answer_delay().await;
            let refused = self.fail_remove.load(std::sync::atomic::Ordering::Relaxed)
                || self.refuses_key(key);
            self.call_returned(index, !refused);
            if refused {
                anyhow::bail!("SpyDataStore: simulated inner-store remove failure");
            }
            if self.retain_accepted {
                self.seeded.remove(&(map.to_string(), key.to_string()));
            }
            Ok(())
        }

        async fn remove_backup(&self, _map: &str, _key: &str, _now: i64) -> anyhow::Result<()> {
            Ok(())
        }

        async fn load(&self, map: &str, key: &str) -> anyhow::Result<Option<RecordValue>> {
            Ok(self
                .seeded
                .get(&(map.to_string(), key.to_string()))
                .map(|v| v.value().clone()))
        }

        async fn load_all(
            &self,
            map: &str,
            keys: &[String],
        ) -> anyhow::Result<Vec<(String, RecordValue)>> {
            let mut results = Vec::new();
            for key in keys {
                if let Some(v) = self.seeded.get(&(map.to_string(), key.clone())) {
                    results.push((key.clone(), v.value().clone()));
                }
            }
            Ok(results)
        }

        async fn remove_all(&self, _map: &str, _keys: &[String]) -> anyhow::Result<()> {
            Ok(())
        }

        async fn enumerate_leaves(
            &self,
            _map: &str,
            _is_backup: bool,
            _sink: &mut dyn LeafSink,
        ) -> anyhow::Result<()> {
            // Test spy holds no durable leaves.
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

        fn is_null(&self) -> bool {
            false
        }
    }

    // -----------------------------------------------------------------------
    // Test 1: enqueue_and_load_returns_staged_value
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn enqueue_and_load_returns_staged_value() {
        let inner: Arc<dyn MapDataStore> = Arc::new(SpyDataStore::new());
        let config = WriteBehindConfig {
            // Long delay so the flush loop never fires during the test
            write_delay_ms: 60_000,
            flush_interval_ms: 60_000,
            ..WriteBehindConfig::default()
        };
        let store = WriteBehindDataStore::new(inner, config);

        let val = dummy_value();
        store.add("map1", "key1", &val, 0, 1000).await.unwrap();

        // load() should return the staged value immediately
        let loaded = store.load("map1", "key1").await.unwrap();
        assert!(
            loaded.is_some(),
            "Staged value should be returned by load()"
        );
    }

    // -----------------------------------------------------------------------
    // Test 2: coalescing_preserves_store_time
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn coalescing_preserves_store_time() {
        let inner: Arc<dyn MapDataStore> = Arc::new(SpyDataStore::new());
        let config = WriteBehindConfig {
            write_delay_ms: 60_000,
            flush_interval_ms: 60_000,
            ..WriteBehindConfig::default()
        };
        let store = WriteBehindDataStore::new(inner, config);

        let val1 = dummy_value_with(1);
        let val2 = dummy_value_with(2);

        // First write at store_time=1000
        store.add("map1", "key1", &val1, 0, 1000).await.unwrap();
        // Second write at store_time=2000 (coalesces)
        store.add("map1", "key1", &val2, 0, 2000).await.unwrap();

        // Only one pending entry (coalesced)
        assert_eq!(
            store.pending_operation_count(),
            1,
            "Coalesced writes should count as one pending entry"
        );

        // Verify the queue has the original store_time but latest value
        let partition_id = partition_for("map1", "key1");
        let queue = store.queues.get(&partition_id).unwrap();
        let entry = queue
            .entries
            .get(&("map1".to_string(), "key1".to_string()))
            .unwrap();
        assert_eq!(
            entry.store_time, 1000,
            "Coalesced entry should preserve original store_time"
        );
    }

    // -----------------------------------------------------------------------
    // Test 3: flush_persists_to_inner
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn flush_persists_to_inner() {
        let spy = Arc::new(SpyDataStore::new());
        let calls = spy.calls();
        let inner: Arc<dyn MapDataStore> = spy;
        let config = short_delay_config();
        let store = WriteBehindDataStore::new(inner, config);

        let val = dummy_value();
        store.add("map1", "key1", &val, 0, 1000).await.unwrap();

        // Wait for the flush loop to process the entry
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        let recorded = calls.lock().unwrap();
        assert!(
            recorded
                .iter()
                .any(|c| matches!(c, SpyCall::Add { map, key } if map == "map1" && key == "key1")),
            "Inner store should have received the add call after flush"
        );
    }

    // -----------------------------------------------------------------------
    // Test 4: pending_count_tracks_operations
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn pending_count_tracks_operations() {
        let inner: Arc<dyn MapDataStore> = Arc::new(SpyDataStore::new());
        let config = short_delay_config();
        let store = WriteBehindDataStore::new(inner, config);

        let val = dummy_value();
        store.add("map1", "a", &val, 0, 1000).await.unwrap();
        store.add("map1", "b", &val, 0, 1000).await.unwrap();
        store.add("map1", "c", &val, 0, 1000).await.unwrap();

        assert_eq!(store.pending_operation_count(), 3);

        // Flush all entries
        store.hard_flush().await.unwrap();

        assert_eq!(store.pending_operation_count(), 0);
    }

    // -----------------------------------------------------------------------
    // Test 5: capacity_limit_rejects_excess
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn capacity_limit_rejects_excess() {
        let inner: Arc<dyn MapDataStore> = Arc::new(SpyDataStore::new());
        let config = WriteBehindConfig {
            capacity: 2,
            write_delay_ms: 60_000,
            flush_interval_ms: 60_000,
            ..WriteBehindConfig::default()
        };
        let store = WriteBehindDataStore::new(inner, config);

        let val = dummy_value();
        store.add("map1", "a", &val, 0, 1000).await.unwrap();
        store.add("map1", "b", &val, 0, 1000).await.unwrap();

        // Third key should be rejected
        let result = store.add("map1", "c", &val, 0, 1000).await;
        assert!(result.is_err(), "Should reject writes exceeding capacity");

        // Queue still contains exactly 2 entries
        assert_eq!(store.pending_operation_count(), 2);
    }

    // -----------------------------------------------------------------------
    // Test 6: hard_flush_drains_all
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn hard_flush_drains_all() {
        let inner: Arc<dyn MapDataStore> = Arc::new(SpyDataStore::new());
        let config = short_delay_config();
        let store = WriteBehindDataStore::new(inner, config);

        let val = dummy_value();
        for i in 0..5 {
            store
                .add("map1", &format!("key{i}"), &val, 0, 1000)
                .await
                .unwrap();
        }

        store.hard_flush().await.unwrap();
        assert_eq!(store.pending_operation_count(), 0);
    }

    // -----------------------------------------------------------------------
    // Test 7: remove_enqueues_delete_op
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn remove_enqueues_delete_op() {
        let spy = Arc::new(SpyDataStore::new());
        // Pre-seed the inner store so it would return a value if consulted
        spy.seed("map1", "key1", dummy_value());
        let inner: Arc<dyn MapDataStore> = spy;

        let config = WriteBehindConfig {
            write_delay_ms: 60_000,
            flush_interval_ms: 60_000,
            ..WriteBehindConfig::default()
        };
        let store = WriteBehindDataStore::new(inner, config);

        let val = dummy_value();
        store.add("map1", "key1", &val, 0, 1000).await.unwrap();
        store.remove("map1", "key1", 2000).await.unwrap();

        // load() should return None because staging has a pending delete marker,
        // NOT falling through to the inner store (which has a pre-seeded value)
        let loaded = store.load("map1", "key1").await.unwrap();
        assert!(
            loaded.is_none(),
            "Pending delete should cause load() to return None without consulting inner store"
        );
    }

    // -----------------------------------------------------------------------
    // AC3c: the flushed watermark is PREFIX-COMPLETE — it never exposes a value
    // above a still-pending (un-flushed) sequence, even when a HIGHER sequence
    // is resolved out of order below it. A max-with-holes watermark would admit
    // a mid-range-hole kill -9 tombstone resurrection; this proves no hole is
    // ever exposed.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn ac3c_flushed_watermark_prefix_complete_never_exposes_hole() {
        let inner: Arc<dyn MapDataStore> = Arc::new(SpyDataStore::new());
        // Long delays so the background flush loop never touches our injected
        // pending set — this test drives resolution deterministically.
        let config = WriteBehindConfig {
            write_delay_ms: 60_000,
            flush_interval_ms: 60_000,
            ..WriteBehindConfig::default()
        };
        let store = WriteBehindDataStore::new(inner, config);

        // Three writes enqueued: sequences 1, 2, 3 pending, nothing flushed yet.
        store.track_pending(1);
        store.track_pending(2);
        store.track_pending(3);
        assert_eq!(
            store.flushed_watermark(),
            1,
            "smallest pending sequence is the watermark — everything below it is resolved"
        );

        // Resolve the MIDDLE sequence (2) out of order: 1 is still pending, so
        // the watermark must NOT jump past the hole at 1 to 2 or 3.
        store.resolve_pending(2);
        assert_eq!(
            store.flushed_watermark(),
            1,
            "out-of-order resolve of 2 must not expose a value above the still-pending 1"
        );

        // Resolve the lowest (1): now 1 and 2 are resolved, only 3 pends, so the
        // watermark advances to exactly 3 — never skipping over an un-flushed seq.
        store.resolve_pending(1);
        assert_eq!(
            store.flushed_watermark(),
            3,
            "watermark advances to the next pending sequence, 3 — prefix stays complete"
        );

        // Resolve the last: nothing pending, watermark is the full assigned counter.
        store.resolve_pending(3);
        assert_eq!(
            store.flushed_watermark(),
            store.assigned_write_sequence(),
            "with nothing pending the watermark is the assigned-sequence counter"
        );
    }

    // -----------------------------------------------------------------------
    // AC3c (real path): a genuine add → in-order background/hard flush advances
    // the watermark monotonically, and a still-buffered write holds it back.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn ac3c_real_flush_advances_watermark_only_on_byte_durability() {
        let inner: Arc<dyn MapDataStore> = Arc::new(SpyDataStore::new());
        let config = WriteBehindConfig {
            write_delay_ms: 60_000,
            flush_interval_ms: 60_000,
            shutdown_timeout_ms: 5_000,
            ..WriteBehindConfig::default()
        };
        let store = WriteBehindDataStore::new(inner, config);

        let val = dummy_value();
        store.add("m", "k1", &val, 0, 1).await.unwrap();
        store.add("m", "k2", &val, 0, 1).await.unwrap();

        // Both writes are buffered (write_delay is 60s), so the watermark sits at
        // the lowest pending sequence — no byte durability yet.
        let assigned = store.assigned_write_sequence();
        assert!(
            store.flushed_watermark() < assigned,
            "buffered writes are not byte-durable: watermark {} must be below assigned {}",
            store.flushed_watermark(),
            assigned
        );

        // Drain everything to the inner store (deterministic byte durability).
        store.hard_flush().await.unwrap();
        assert_eq!(
            store.flushed_watermark(),
            assigned,
            "after a full drain every assigned sequence is durable, so the watermark reaches it"
        );
    }

    // -----------------------------------------------------------------------
    // A flush pass must resolve an entry's sequence AFTER `inner.add` returns,
    // not before. Gate the inner add so the watermark can be read while the pass
    // is parked mid-add — it must still sit at the buffered sequence. Resolving
    // before the add advances the watermark past a write whose bytes are not yet
    // durable, licensing a prune of a tombstone the inner store has not
    // persisted.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn a_flush_pass_resolves_the_watermark_after_inner_add_not_before() {
        use std::time::{Duration, Instant};

        let spy = Arc::new(SpyDataStore::new());
        let (entered, proceed) = spy.install_add_gate();
        let inner: Arc<dyn MapDataStore> = spy;
        // Due at once, but the timer never fires within the test: the requested
        // pass is the only pass.
        let config = WriteBehindConfig {
            write_delay_ms: 0,
            flush_interval_ms: 60_000,
            ..WriteBehindConfig::default()
        };
        let store = WriteBehindDataStore::new(inner, config);

        store.add("m", "k", &dummy_value(), 0, 1).await.unwrap();
        let buffered_seq = store.flushed_watermark();
        let assigned = store.assigned_write_sequence();
        assert!(
            buffered_seq < assigned,
            "precondition: the buffered write is not yet byte-durable"
        );

        // The pass takes the entry and parks inside the gated inner add.
        store.soft_flush().await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), entered.notified())
            .await
            .expect("the requested pass never reached the inner add");
        assert_eq!(
            store.flushed_watermark(),
            buffered_seq,
            "watermark must stay at the buffered sequence while inner.add is in flight — \
             resolving before the add would falsely report the write as durable"
        );

        // Let the add complete; the sequence is now durable and resolvable.
        proceed.notify_one();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !store.test_is_idle() {
            assert!(
                Instant::now() < deadline,
                "the pass never closed after the inner add returned"
            );
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        assert_eq!(
            store.flushed_watermark(),
            assigned,
            "after inner.add returns the resolved sequence lets the watermark advance"
        );
    }

    /// An entry the store refuses is never given up: it stays buffered and
    /// readable, and its ENTRY sequence stays unresolved, so the flushed
    /// watermark — the tombstone durability fence — waits below it for as
    /// long as the inner store has not taken the write. Only the accepted
    /// store call resolves it.
    #[tokio::test(start_paused = true)]
    async fn a_refused_entry_stays_buffered_and_holds_the_flushed_watermark() {
        let spy = Arc::new(SpyDataStore::retaining());
        spy.refuse_everything(true);
        let store = schedule_store(&spy, schedule_config());
        store
            .add(SCHEDULE_MAP, "k", &lww("v", 1), 0, 1)
            .await
            .unwrap();
        advance(60_000).await;
        let while_refused = (
            store.pending_operation_count(),
            store.load(SCHEDULE_MAP, "k").await.unwrap(),
            store.flushed_watermark() < store.assigned_write_sequence(),
        );
        spy.refuse_everything(false);
        let drained = poll_until(10_000, || store.test_is_idle()).await.is_some();
        let after = (
            store.pending_operation_count(),
            store.flushed_watermark() == store.assigned_write_sequence(),
            spy.held(SCHEDULE_MAP, "k"),
        );
        assert_eq!(
            (while_refused, drained, after),
            (
                (1, Some(lww("v", 1)), true),
                true,
                (0, true, Some(lww("v", 1)))
            )
        );
    }

    // -----------------------------------------------------------------------
    // A batch `remove_all` that coalesces a still-buffered write MUST resolve the
    // retired sequence — otherwise it leaks in the pending set forever and pins
    // the flushed watermark below it, permanently stalling the tombstone prune.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn remove_all_coalesce_resolves_retired_sequence_no_watermark_stall() {
        let inner: Arc<dyn MapDataStore> = Arc::new(SpyDataStore::new());
        let config = WriteBehindConfig {
            write_delay_ms: 60_000,
            flush_interval_ms: 60_000,
            shutdown_timeout_ms: 5_000,
            ..WriteBehindConfig::default()
        };
        let store = WriteBehindDataStore::new(inner, config);

        // A buffered add for k1: its sequence is the lowest pending, so the
        // watermark sits at it (nothing below it is durable yet).
        store.add("m", "k1", &dummy_value(), 0, 1).await.unwrap();
        let add_seq = store.flushed_watermark();

        // A batch remove of the SAME key coalesces the buffered add. The retired
        // add sequence must be resolved so the watermark can advance past it —
        // before the fix it leaked and the watermark stayed pinned at `add_seq`.
        store.remove_all("m", &["k1".to_string()]).await.unwrap();
        assert!(
            store.flushed_watermark() > add_seq,
            "coalesced-away add sequence {add_seq} must be resolved, not leaked: watermark is {}",
            store.flushed_watermark()
        );

        // And after a full drain the watermark reaches the assigned counter —
        // proving no pending sequence was orphaned by the coalesce.
        store.hard_flush().await.unwrap();
        assert_eq!(
            store.flushed_watermark(),
            store.assigned_write_sequence(),
            "no orphaned pending sequence after coalesce + drain"
        );
    }

    // -----------------------------------------------------------------------
    // Test 9: soft_flush_returns_sequence
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn soft_flush_returns_sequence() {
        let inner: Arc<dyn MapDataStore> = Arc::new(SpyDataStore::new());
        let config = WriteBehindConfig {
            write_delay_ms: 60_000,
            flush_interval_ms: 60_000,
            ..WriteBehindConfig::default()
        };
        let store = WriteBehindDataStore::new(inner, config);

        let val = dummy_value();
        store.add("map1", "a", &val, 0, 1000).await.unwrap();
        store.add("map1", "b", &val, 0, 1000).await.unwrap();

        let seq = store.soft_flush().await.unwrap();
        assert!(seq > 0, "Sequence should be > 0 after enqueuing entries");
    }

    // -----------------------------------------------------------------------
    // Test 10: hard_flush_drains_all_to_inner_store (AC1)
    //
    // Verifies that every write acked to a client is readable from the inner
    // store after hard_flush() completes. This covers the graceful-shutdown
    // guarantee: no acked write is lost when the process exits cleanly.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn hard_flush_drains_all_to_inner_store() {
        // Use a SpyDataStore that also persists values so load() works post-drain.
        let spy = Arc::new(SpyDataStore::new());
        let inner: Arc<dyn MapDataStore> = Arc::clone(&spy) as Arc<dyn MapDataStore>;

        let config = WriteBehindConfig {
            // Long write delay ensures the background loop would NOT flush on its own
            // during this test — only hard_flush() should drive persistence.
            write_delay_ms: 60_000,
            flush_interval_ms: 60_000,
            shutdown_timeout_ms: 5_000,
            ..WriteBehindConfig::default()
        };
        let store = WriteBehindDataStore::new(inner, config);

        let val = dummy_value();
        let keys = ["alpha", "beta", "gamma", "delta"];
        for key in &keys {
            store.add("drain_map", key, &val, 0, 1000).await.unwrap();
        }

        // All writes are acked (add returned Ok) but not yet in the inner store.
        assert_eq!(store.pending_operation_count(), keys.len() as u64);

        // hard_flush() should drain every pending entry.
        store.hard_flush().await.unwrap();

        assert_eq!(
            store.pending_operation_count(),
            0,
            "All pending entries should be drained after hard_flush"
        );

        // Verify the inner store received an add call for every key.
        // Drop the lock guard before any await point to satisfy clippy's
        // `await_holding_lock` lint (holding a MutexGuard across an await
        // can deadlock if another task also tries to acquire the same mutex).
        {
            let recorded = spy.calls.lock().unwrap();
            for key in &keys {
                assert!(
                    recorded.iter().any(
                        |c| matches!(c, SpyCall::Add { map, key: k } if map == "drain_map" && k == *key)
                    ),
                    "Inner store should have received add for key={key}"
                );
            }
        } // recorded guard dropped here

        // Writes after shutdown must be rejected.
        let result = store.add("drain_map", "late_write", &val, 0, 1000).await;
        assert!(
            result.is_err(),
            "Writes after hard_flush (shutdown) must be rejected"
        );
    }

    // -----------------------------------------------------------------------
    // Test 11: hard_flush_timeout_logs_warn_and_returns (AC2)
    //
    // Injects a slow inner store to verify that hard_flush() respects
    // shutdown_timeout_ms: on timeout it logs each pending op via warn! and
    // returns Ok(()) rather than hanging the process.
    // -----------------------------------------------------------------------

    /// Inner store that sleeps for a configurable duration on every `add` call,
    /// allowing tests to simulate a slow or unresponsive persistence backend.
    struct SlowDataStore {
        delay_ms: u64,
        calls: Arc<Mutex<Vec<SpyCall>>>,
    }

    impl SlowDataStore {
        fn new(delay_ms: u64) -> Self {
            Self {
                delay_ms,
                calls: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    #[async_trait]
    impl MapDataStore for SlowDataStore {
        async fn add(
            &self,
            map: &str,
            key: &str,
            _value: &RecordValue,
            _expiration_time: i64,
            _now: i64,
        ) -> anyhow::Result<()> {
            // Simulate a slow inner store that takes longer than the drain timeout.
            tokio::time::sleep(tokio::time::Duration::from_millis(self.delay_ms)).await;
            self.calls.lock().unwrap().push(SpyCall::Add {
                map: map.to_string(),
                key: key.to_string(),
            });
            Ok(())
        }

        async fn add_backup(
            &self,
            _map: &str,
            _key: &str,
            _value: &RecordValue,
            _expiration_time: i64,
            _now: i64,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn remove(&self, _map: &str, _key: &str, _now: i64) -> anyhow::Result<()> {
            tokio::time::sleep(tokio::time::Duration::from_millis(self.delay_ms)).await;
            Ok(())
        }

        async fn remove_backup(&self, _map: &str, _key: &str, _now: i64) -> anyhow::Result<()> {
            Ok(())
        }

        async fn load(&self, _map: &str, _key: &str) -> anyhow::Result<Option<RecordValue>> {
            Ok(None)
        }

        async fn load_all(
            &self,
            _map: &str,
            _keys: &[String],
        ) -> anyhow::Result<Vec<(String, RecordValue)>> {
            Ok(Vec::new())
        }

        async fn remove_all(&self, _map: &str, _keys: &[String]) -> anyhow::Result<()> {
            Ok(())
        }

        async fn enumerate_leaves(
            &self,
            _map: &str,
            _is_backup: bool,
            _sink: &mut dyn LeafSink,
        ) -> anyhow::Result<()> {
            // Slow test store has no durable leaves to enumerate.
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

        fn is_null(&self) -> bool {
            false
        }
    }

    #[tokio::test]
    async fn hard_flush_timeout_logs_warn_and_returns() {
        // The inner store sleeps 500ms per write — far longer than our 50ms
        // drain timeout — so hard_flush() will always exceed the deadline.
        let slow_inner: Arc<dyn MapDataStore> = Arc::new(SlowDataStore::new(500));
        let config = WriteBehindConfig {
            write_delay_ms: 60_000,
            flush_interval_ms: 60_000,
            // Very short timeout so the test completes quickly.
            shutdown_timeout_ms: 50,
            ..WriteBehindConfig::default()
        };
        let store = WriteBehindDataStore::new(slow_inner, config);

        let val = dummy_value();
        store.add("slow_map", "key1", &val, 0, 1000).await.unwrap();
        store.add("slow_map", "key2", &val, 0, 1000).await.unwrap();

        assert_eq!(store.pending_operation_count(), 2);

        // hard_flush() must return within a window proportional to
        // shutdown_timeout_ms, not proportional to the slow inner-store delay.
        // The outer guard (500ms) is 10× the configured timeout (50ms) to
        // allow for scheduling jitter while still catching a pathological hang.
        let result =
            tokio::time::timeout(tokio::time::Duration::from_millis(500), store.hard_flush()).await;

        assert!(
            result.is_ok(),
            "hard_flush() must return within the outer timeout (should not hang)"
        );
        // hard_flush() itself should succeed (timeout is a warn, not an Err).
        assert!(
            result.unwrap().is_ok(),
            "hard_flush() should return Ok(()) on timeout, logging warnings"
        );
    }

    // -----------------------------------------------------------------------
    // WAL behavioral tests (AC1, AC2, AC7)
    // -----------------------------------------------------------------------

    use super::WalBootstrap;
    use crate::storage::wal::{Wal, WalEntry, WalFsyncPolicy};
    use std::sync::atomic::{AtomicU64 as TestAtomicU64, Ordering as TestOrdering};

    /// Minimal in-memory WAL for testing write-path WAL integration.
    struct InMemoryTestWal {
        appended: Arc<tokio::sync::Mutex<Vec<(u32, WalEntry)>>>,
        applied: Arc<tokio::sync::Mutex<Vec<(u32, u64)>>>,
        append_count: Arc<TestAtomicU64>,
    }

    impl InMemoryTestWal {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                appended: Arc::new(tokio::sync::Mutex::new(Vec::new())),
                applied: Arc::new(tokio::sync::Mutex::new(Vec::new())),
                append_count: Arc::new(TestAtomicU64::new(0)),
            })
        }
    }

    #[async_trait::async_trait]
    impl Wal for InMemoryTestWal {
        async fn append(&self, partition: u32, entry: &WalEntry) -> anyhow::Result<()> {
            self.appended.lock().await.push((partition, entry.clone()));
            self.append_count.fetch_add(1, TestOrdering::Relaxed);
            Ok(())
        }

        async fn mark_applied(&self, partition: u32, sequence: u64) -> anyhow::Result<()> {
            self.applied.lock().await.push((partition, sequence));
            Ok(())
        }

        async fn unapplied(&self, partition: u32) -> anyhow::Result<Vec<WalEntry>> {
            let guard = self.appended.lock().await;
            let applied_guard = self.applied.lock().await;
            let applied_seqs: std::collections::HashSet<u64> = applied_guard
                .iter()
                .filter(|(p, _)| *p == partition)
                .map(|(_, seq)| *seq)
                .collect();
            Ok(guard
                .iter()
                .filter(|(p, e)| *p == partition && !applied_seqs.contains(&e.sequence))
                .map(|(_, e)| e.clone())
                .collect())
        }
    }

    // AC1: WAL entry is appended before add() returns Ok(())
    #[tokio::test]
    async fn wal_append_happens_before_add_returns() {
        let wal = InMemoryTestWal::new();
        let wal_arc: Arc<dyn Wal> = Arc::clone(&wal) as Arc<dyn Wal>;

        let inner: Arc<dyn MapDataStore> = Arc::new(SpyDataStore::new());
        let config = WriteBehindConfig {
            write_delay_ms: 60_000,
            flush_interval_ms: 60_000,
            ..WriteBehindConfig::default()
        };
        let store = WriteBehindDataStore::new_with_wal(
            inner,
            config,
            Some(WalBootstrap {
                wal: wal_arc,
                sequence_start: 1,
            }),
        );

        let val = dummy_value();
        store.add("m", "k1", &val, 0, 1000).await.unwrap();

        // WAL must contain the entry immediately after add() returns
        let count = wal.append_count.load(TestOrdering::Relaxed);
        assert_eq!(count, 1, "WAL must have exactly 1 entry after add()");

        let appended = wal.appended.lock().await;
        assert_eq!(appended.len(), 1);
        assert_eq!(appended[0].1.map, "m");
        assert_eq!(appended[0].1.key, "k1");
    }

    /// TG-WAL-010: WAL-sequence assignment is strictly monotone per partition and
    /// never reuses a sequence globally. This is premise (c) of the TG-WAL-009
    /// gate-free warrant (a strictly-newer re-creation of a key gets a strictly
    /// HIGHER WAL sequence than an older op on the same key, so `M > N` holds).
    ///
    /// The assigns MUST be spread across distinct partitions. `assign_wal_sequence`
    /// mints inside `with_partition`, i.e. under that partition's OWN mutex, so
    /// pinning every task to one partition lets the lock serialize the counter bump
    /// and hides whether the counter itself is atomic: with all tasks on a single
    /// partition, replacing the `fetch_add` with a racy load/store keeps this test
    /// (and the whole suite) green. Spread over 8 partitions the mutexes no longer
    /// overlap and the same mutation goes RED on the arrival-order assertion (the
    /// counter is observed moving backwards within one partition). One task per
    /// partition is what makes that arrival order observable at all.
    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn wal_sequence_assignment_is_strictly_monotone_per_partition() {
        // Eight DISTINCT partitions, named explicitly so the test needs no
        // usize/u32 casts.
        const PARTITIONS: [u32; 8] = [0, 1, 2, 3, 4, 5, 6, 7];
        const PER_TASK: usize = 250;

        let wal = InMemoryTestWal::new();
        let wal_arc: Arc<dyn Wal> = Arc::clone(&wal) as Arc<dyn Wal>;
        let inner: Arc<dyn MapDataStore> = Arc::new(SpyDataStore::new());
        let store = WriteBehindDataStore::new_with_wal(
            inner,
            WriteBehindConfig::default(),
            Some(WalBootstrap {
                wal: wal_arc,
                sequence_start: 1,
            }),
        );

        // One task per partition: distinct partitions => distinct tracking mutexes,
        // so the only thing ordering these assigns is the counter itself.
        let mut handles = Vec::new();
        for partition in PARTITIONS {
            let store = Arc::clone(&store);
            handles.push(tokio::spawn(async move {
                (
                    partition,
                    (0..PER_TASK)
                        .map(|_| store.assign_wal_sequence(partition))
                        .collect::<Vec<u64>>(),
                )
            }));
        }

        let mut all = Vec::new();
        for h in handles {
            let (partition, assigned) = h.await.unwrap();

            // Per-partition monotonicity, in ARRIVAL order (not sorted): a later
            // arrival on a partition gets a strictly higher sequence. This is the
            // `M > N` step of the TG-WAL-009 warrant.
            assert!(
                assigned.windows(2).all(|w| w[0] < w[1]),
                "partition {partition} assignments must be strictly increasing in \
                 arrival order, got {assigned:?}"
            );
            all.extend(assigned);
        }

        // Global no-reuse: one process-global counter feeds every partition, so no
        // sequence may be handed out twice across partitions either.
        let total = PARTITIONS.len() * PER_TASK;
        let unique: std::collections::HashSet<u64> = all.iter().copied().collect();
        assert_eq!(
            unique.len(),
            total,
            "a strictly-monotone counter never reuses a sequence under concurrency \
             ({total} concurrent assigns produced {} distinct values)",
            unique.len()
        );
    }

    // AC1: WAL entry is appended before remove() returns Ok(())
    #[tokio::test]
    async fn wal_append_happens_before_remove_returns() {
        let wal = InMemoryTestWal::new();
        let wal_arc: Arc<dyn Wal> = Arc::clone(&wal) as Arc<dyn Wal>;

        let inner: Arc<dyn MapDataStore> = Arc::new(SpyDataStore::new());
        let config = WriteBehindConfig {
            write_delay_ms: 60_000,
            flush_interval_ms: 60_000,
            ..WriteBehindConfig::default()
        };
        let store = WriteBehindDataStore::new_with_wal(
            inner,
            config,
            Some(WalBootstrap {
                wal: wal_arc,
                sequence_start: 1,
            }),
        );

        store.remove("m", "k1", 1000).await.unwrap();

        let count = wal.append_count.load(TestOrdering::Relaxed);
        assert_eq!(count, 1, "WAL must have exactly 1 entry after remove()");
    }

    // AC1: remove_all appends one WAL frame per key before returning Ok(())
    #[tokio::test]
    async fn wal_append_happens_before_remove_all_returns() {
        let wal = InMemoryTestWal::new();
        let wal_arc: Arc<dyn Wal> = Arc::clone(&wal) as Arc<dyn Wal>;

        let inner: Arc<dyn MapDataStore> = Arc::new(SpyDataStore::new());
        let config = WriteBehindConfig {
            write_delay_ms: 60_000,
            flush_interval_ms: 60_000,
            ..WriteBehindConfig::default()
        };
        let store = WriteBehindDataStore::new_with_wal(
            inner,
            config,
            Some(WalBootstrap {
                wal: wal_arc,
                sequence_start: 1,
            }),
        );

        let keys = vec!["k1".to_string(), "k2".to_string(), "k3".to_string()];
        store.remove_all("m", &keys).await.unwrap();

        // Every key must be durable in the WAL the moment remove_all() returns,
        // so an unclean crash replays each individual tombstone, not just the batch.
        let count = wal.append_count.load(TestOrdering::Relaxed);
        assert_eq!(
            count, 3,
            "WAL must have one entry per removed key after remove_all()"
        );

        let appended = wal.appended.lock().await;
        assert_eq!(appended.len(), 3);
        for entry in appended.iter() {
            assert_eq!(entry.1.map, "m");
            assert!(matches!(entry.1.op, WalOp::Remove));
        }
    }

    // AC2: after flush, WAL entry is marked applied
    #[tokio::test]
    async fn wal_entry_marked_applied_after_flush() {
        let wal = InMemoryTestWal::new();
        let applied_store = Arc::clone(&wal.applied);
        let wal_arc: Arc<dyn Wal> = Arc::clone(&wal) as Arc<dyn Wal>;

        let inner: Arc<dyn MapDataStore> = Arc::new(SpyDataStore::new());
        let config = short_delay_config();
        let store = WriteBehindDataStore::new_with_wal(
            inner,
            config,
            Some(WalBootstrap {
                wal: wal_arc,
                sequence_start: 1,
            }),
        );

        let val = dummy_value();
        store.add("m", "k1", &val, 0, 1000).await.unwrap();

        // Wait for flush loop to process the entry
        tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;

        let applied = applied_store.lock().await;
        assert!(
            !applied.is_empty(),
            "WAL entry must be marked applied after the flush loop persists it to the inner store"
        );
    }

    /// Build a `from_source` lookup closure from a fixed map of overrides — the
    /// parallel-safe seam: full parse logic is exercised without touching
    /// process-global env, so this test cannot race any other test reading the
    /// same vars (the flake behind G3's withdrawal, TODO-468).
    fn config_source(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: std::collections::HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        move |key: &str| map.get(key).cloned()
    }

    // AC7: from_source parses TOPGUN_WAL_DIR and TOPGUN_WAL_FSYNC_POLICY
    #[test]
    fn from_source_parses_wal_dir_and_fsync_policy() {
        // TOPGUN_WAL_DIR
        let cfg =
            WriteBehindConfig::from_source(config_source(&[("TOPGUN_WAL_DIR", "/tmp/test-wal")]));
        assert_eq!(cfg.wal_dir, std::path::PathBuf::from("/tmp/test-wal"));

        // TOPGUN_WAL_FSYNC_POLICY with a valid value
        let cfg =
            WriteBehindConfig::from_source(config_source(&[("TOPGUN_WAL_FSYNC_POLICY", "per_op")]));
        assert_eq!(cfg.wal_fsync_policy, WalFsyncPolicy::PerOp);

        // The harness/doc `perop` spelling must resolve to PerOp (no silent
        // downgrade) now that the parser normalizes the separator/case.
        let cfg =
            WriteBehindConfig::from_source(config_source(&[("TOPGUN_WAL_FSYNC_POLICY", "perop")]));
        assert_eq!(cfg.wal_fsync_policy, WalFsyncPolicy::PerOp);

        // No override → Batched default
        let cfg = WriteBehindConfig::from_source(config_source(&[]));
        assert_eq!(cfg.wal_fsync_policy, WalFsyncPolicy::Batched);
    }

    #[test]
    #[should_panic(expected = "is not a valid WAL fsync policy")]
    fn from_source_unknown_fsync_policy_is_fatal() {
        // An unknown durability policy must refuse to start rather than silently
        // degrade to the weaker default — the silent-downgrade path is what hid a
        // durability regression through a full RED soak.
        let _ = WriteBehindConfig::from_source(config_source(&[(
            "TOPGUN_WAL_FSYNC_POLICY",
            "invalid_policy",
        )]));
    }

    #[test]
    fn from_source_parses_the_wal_watermark_stall_bound() {
        // Unset ⇒ the recorded default. Asserting the server's OWN parse rather
        // than re-deriving the value from the environment is the point: the
        // defect class this guards is a parse that silently disagrees with what
        // the operator-facing boot line reports.
        let cfg = WriteBehindConfig::from_source(config_source(&[]));
        assert_eq!(cfg.wal_watermark_stall_bound_ms, 60_000);

        let cfg = WriteBehindConfig::from_source(config_source(&[(
            "TOPGUN_WAL_WATERMARK_STALL_BOUND_MS",
            "90000",
        )]));
        assert_eq!(cfg.wal_watermark_stall_bound_ms, 90_000);

        // Unparseable warns and falls back — deliberately NOT symmetric with the
        // out-of-range case below: a typo must not take the process down, while
        // a value that parses and disarms the alarm's false-positive guard must.
        let cfg = WriteBehindConfig::from_source(config_source(&[(
            "TOPGUN_WAL_WATERMARK_STALL_BOUND_MS",
            "not-a-number",
        )]));
        assert_eq!(cfg.wal_watermark_stall_bound_ms, 60_000);

        // The ACCEPTED boundary, with the derived values read off the config's own
        // arithmetic so a regression to a bare division cannot pass: at the floor
        // both derivations are pinned by their floors, not by the quotient.
        let cfg = WriteBehindConfig::from_source(config_source(&[(
            "TOPGUN_WAL_WATERMARK_STALL_BOUND_MS",
            "6000",
        )]));
        assert_eq!(cfg.wal_watermark_stall_bound_ms, 6_000);
        assert_eq!(cfg.reconfirm_delay_ms(), 100);
        assert_eq!(cfg.watchdog_tick_ms(), 600);
    }

    #[test]
    #[should_panic(expected = "is below the minimum of")]
    fn from_source_stall_bound_below_the_minimum_is_fatal() {
        // One millisecond under the floor: the check must be a boundary, not a
        // rough band, or the alarm's re-confirmation window shrinks below the
        // race it exists to absorb while the boot line still reports it as
        // effective.
        let _ = WriteBehindConfig::from_source(config_source(&[(
            "TOPGUN_WAL_WATERMARK_STALL_BOUND_MS",
            "5999",
        )]));
    }

    /// `TOPGUN_OR_DELTA_WAL` reaches the field, and only a falsey WORD disarms.
    ///
    /// The kill-switch's whole operator contract is written in terms of the
    /// VARIABLE, so asserting the config field alone leaves the half an operator
    /// actually touches — the name of the key, and the parse — unasserted. A typo
    /// in the lookup key would be invisible to every other test in this file.
    #[test]
    fn from_source_parses_the_or_delta_wal_kill_switch() {
        // Armed by default: the seam is on unless an operator turns it off.
        assert!(WriteBehindConfig::from_source(config_source(&[])).or_delta_wal);

        for falsey in ["false", "0", "no", "off", "FALSE", " Off "] {
            let cfg =
                WriteBehindConfig::from_source(config_source(&[("TOPGUN_OR_DELTA_WAL", falsey)]));
            assert!(
                !cfg.or_delta_wal,
                "{falsey:?} is an explicit disarm and must reach the field"
            );
        }

        for truthy in ["true", "1", "yes", "on", "TRUE"] {
            let cfg =
                WriteBehindConfig::from_source(config_source(&[("TOPGUN_OR_DELTA_WAL", truthy)]));
            assert!(cfg.or_delta_wal, "{truthy:?} must leave the seam armed");
        }
    }

    /// An UNRECOGNISED value leaves the seam ARMED — and says so.
    ///
    /// Both halves are load-bearing and neither is checkable from the other.
    /// Staying armed is the deliberate direction: a typo must not quietly change
    /// the durable encoding a running node writes, which is the opposite choice
    /// from the fsync policy next door, and that asymmetry is only defensible
    /// while the typo is VISIBLE. A silent arm leaves an operator believing the
    /// WAL is safe to roll back across while the node keeps writing delta frames.
    #[test]
    fn an_unrecognised_or_delta_wal_value_stays_armed_and_names_itself() {
        #[derive(Clone)]
        struct CapturedLog(Arc<std::sync::Mutex<Vec<u8>>>);
        impl std::io::Write for CapturedLog {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let captured: Arc<std::sync::Mutex<Vec<u8>>> = Arc::new(std::sync::Mutex::new(Vec::new()));

        // Scoped, never a global install: this test binary is shared and runs in
        // parallel, so a global subscriber would leak into every other test.
        let writer = CapturedLog(Arc::clone(&captured));
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_ansi(false)
            .finish();

        // A misspelled disarm, and a value some env systems deliver with its
        // quotes still attached -- the two shapes that actually reach operators.
        let cfgs = tracing::subscriber::with_default(subscriber, || {
            ["flase", "\"false\""].map(|raw| {
                WriteBehindConfig::from_source(config_source(&[("TOPGUN_OR_DELTA_WAL", raw)]))
            })
        });

        for (raw, cfg) in ["flase", "\"false\""].iter().zip(cfgs.iter()) {
            assert!(
                cfg.or_delta_wal,
                "{raw:?} is not an explicit disarm, so the seam must stay ARMED rather than \
                 roll a node back to an encoding nobody asked for"
            );
        }

        let logged = String::from_utf8(
            captured
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone(),
        )
        .expect("captured log is utf-8");

        for raw in ["flase", "\"false\""] {
            assert!(
                logged.contains(raw),
                "the warning must NAME the unrecognised value {raw:?} so the typo is visible at \
                 boot rather than at the downgrade that fails because of it; captured: {logged}"
            );
        }
        assert!(
            logged.contains("TOPGUN_OR_DELTA_WAL"),
            "the warning must name the variable it is about; captured: {logged}"
        );
    }

    /// Wiring-only smoke test for the real `from_env` → `std::env` seam. The sole
    /// env-mutating config test, `#[serial]` so it cannot race the env-free tests
    /// above. Parse logic is covered by `from_source_*`; here we only prove
    /// `from_env` actually reads the process environment.
    #[serial_test::serial]
    #[test]
    fn from_env_reads_process_environment() {
        std::env::set_var("TOPGUN_WAL_FSYNC_POLICY", "per_op");
        let cfg = WriteBehindConfig::from_env();
        std::env::remove_var("TOPGUN_WAL_FSYNC_POLICY");
        assert_eq!(cfg.wal_fsync_policy, WalFsyncPolicy::PerOp);
    }

    // -----------------------------------------------------------------------
    // Buffer-aware read surface (scan/enumerate/list_maps overlay staging)
    // -----------------------------------------------------------------------

    use crate::storage::map_data_store::{MerkleLeaf, MerkleLeafKind};
    use crate::storage::record::OrMapEntry;
    use crate::storage::RedbDataStore;

    /// `WriteBehind` over a real redb inner with a flush delay long enough that
    /// `add`/`remove` stay buffered for the duration of a test (no flush pass
    /// runs unless a test requests one with `soft_flush`).
    fn redb_backed_store() -> (Arc<WriteBehindDataStore>, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("wb.redb");
        let inner: Arc<dyn MapDataStore> = Arc::new(RedbDataStore::new(&path).expect("redb open"));
        // Entries are due at once, but the timer never fires within a test:
        // only a pass a test requests with `soft_flush` takes them.
        let config = WriteBehindConfig {
            write_delay_ms: 0,
            flush_interval_ms: 600_000,
            capacity: 0,
            ..WriteBehindConfig::default()
        };
        (WriteBehindDataStore::new(inner, config), dir)
    }

    fn lww(value: &str, millis: u64) -> RecordValue {
        RecordValue::Lww {
            value: Value::String(value.to_string()),
            timestamp: Timestamp {
                millis,
                counter: 0,
                node_id: "n".to_string(),
            },
        }
    }

    /// Assert an `Lww` record carries the expected string value and HLC millis.
    /// `RecordValue` is intentionally not `PartialEq` (avoids a production derive
    /// just for tests), so destructure instead.
    fn assert_lww(record: &RecordValue, value: &str, millis: u64) {
        match record {
            RecordValue::Lww {
                value: Value::String(v),
                timestamp,
            } => {
                assert_eq!(v, value, "lww value mismatch");
                assert_eq!(timestamp.millis, millis, "lww hlc millis mismatch");
            }
            other => panic!("expected Lww string record, got {other:?}"),
        }
    }

    async fn drain_scan(
        store: &Arc<WriteBehindDataStore>,
        map: &str,
    ) -> Vec<(String, RecordValue)> {
        let mut out = Vec::new();
        let mut batch = store.scan_values(map, false, 0).await.unwrap();
        loop {
            out.append(&mut batch.records);
            match batch.next_cursor.take() {
                Some(cursor) => {
                    batch = store
                        .scan_values_batched(map, false, cursor, 0)
                        .await
                        .unwrap();
                }
                None => break,
            }
        }
        out
    }

    /// `LeafSink` that collects every emitted leaf for assertions.
    struct CollectLeaves {
        leaves: Vec<MerkleLeaf>,
    }

    #[async_trait]
    impl LeafSink for CollectLeaves {
        async fn consume(&mut self, batch: Vec<MerkleLeaf>) -> anyhow::Result<()> {
            self.leaves.extend(batch);
            Ok(())
        }
    }

    async fn collect_leaves(store: &Arc<WriteBehindDataStore>, map: &str) -> Vec<MerkleLeaf> {
        let mut sink = CollectLeaves { leaves: Vec::new() };
        store.enumerate_leaves(map, false, &mut sink).await.unwrap();
        sink.leaves
    }

    // AC2 — scan_values surfaces a buffered-only value; no double-count after
    // flush; a buffered write overrides an older flushed redb value.
    #[tokio::test]
    async fn ac2_scan_values_buffer_aware() {
        let (store, _dir) = redb_backed_store();

        // Buffered-only key: only in staging, never flushed.
        store.add("m", "k", &lww("v1", 1), 0, 1000).await.unwrap();
        let rows = drain_scan(&store, "m").await;
        assert_eq!(rows.len(), 1, "buffered-only key must be surfaced by scan");
        assert_eq!(rows[0].0, "k");
        assert_lww(&rows[0].1, "v1", 1);

        // Flush to redb; scan must still return it exactly once (no double-count).
        store.soft_flush().await.unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !store.test_is_idle() {
            assert!(
                std::time::Instant::now() < deadline,
                "the requested pass never flushed the key"
            );
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
        let rows = drain_scan(&store, "m").await;
        assert_eq!(rows.len(), 1, "flushed key must appear exactly once");
        assert_lww(&rows[0].1, "v1", 1);

        // A newer buffered write over the flushed redb value wins, exactly once.
        store.add("m", "k", &lww("v2", 5), 0, 2000).await.unwrap();
        let rows = drain_scan(&store, "m").await;
        assert_eq!(rows.len(), 1, "override must not double-count");
        assert_lww(&rows[0].1, "v2", 5);
    }

    // AC3 — a pending delete buffered in staging hides the flushed redb value
    // from both scan_values and enumerate_leaves (no resurrection).
    #[tokio::test]
    async fn ac3_pending_delete_suppression() {
        let (store, _dir) = redb_backed_store();

        store.add("m", "k", &lww("v1", 1), 0, 1000).await.unwrap();
        store.soft_flush().await.unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !store.test_is_idle() {
            assert!(
                std::time::Instant::now() < deadline,
                "the requested pass never flushed the key"
            );
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
        // Now buffer a delete (staging None) over the flushed value.
        store.remove("m", "k", 2000).await.unwrap();

        let rows = drain_scan(&store, "m").await;
        assert!(
            rows.is_empty(),
            "pending delete must hide the flushed value"
        );

        let leaves = collect_leaves(&store, "m").await;
        assert!(
            leaves.is_empty(),
            "pending delete must suppress the durable leaf"
        );
    }

    // AC4 — leaf-hash parity across flush + OrMap None-leaf propagation.
    #[tokio::test]
    async fn ac4_enumerate_leaves_hash_parity() {
        let (store, _dir) = redb_backed_store();

        // Buffered-only Lww leaf.
        store.add("m", "k", &lww("v1", 7), 0, 1000).await.unwrap();
        let buffered_leaves = collect_leaves(&store, "m").await;
        assert_eq!(buffered_leaves.len(), 1, "buffered-only leaf must surface");

        // Flush and re-enumerate: same leaf hash (Merkle root unchanged).
        store.soft_flush().await.unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !store.test_is_idle() {
            assert!(
                std::time::Instant::now() < deadline,
                "the requested pass never flushed the key"
            );
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
        let flushed_leaves = collect_leaves(&store, "m").await;
        assert_eq!(
            buffered_leaves, flushed_leaves,
            "leaf hash must be identical buffered vs flushed"
        );
        assert_eq!(buffered_leaves[0].kind, MerkleLeafKind::Lww);

        // OrMap sub-assertion: an OrMap entry yields a leaf; an OrTombstones
        // entry (merkle_leaf_hash == None) contributes NO leaf — no zero/
        // placeholder leaf injected.
        let or_value = RecordValue::OrMap {
            records: vec![OrMapEntry {
                value: Value::String("x".to_string()),
                tag: "t1".to_string(),
                timestamp: Timestamp {
                    millis: 1,
                    counter: 0,
                    node_id: "n".to_string(),
                },
            }],
            tombstones: Vec::new(),
        };
        store.add("o", "ok", &or_value, 0, 1000).await.unwrap();
        let none_value = RecordValue::OrTombstones {
            tags: vec!["gone".to_string()],
        };
        store
            .add("o", "tombstoned", &none_value, 0, 1000)
            .await
            .unwrap();

        let or_leaves = collect_leaves(&store, "o").await;
        assert_eq!(
            or_leaves.len(),
            1,
            "OrMap entry yields a leaf; OrTombstones (None) contributes none"
        );
        assert_eq!(or_leaves[0].key, "ok");
        assert_eq!(or_leaves[0].kind, MerkleLeafKind::OrMap);
    }

    // AC5 — a backup scan/enumerate is byte-for-byte the inner result: no
    // overlay of non-backup staging writes.
    #[tokio::test]
    async fn ac5_is_backup_untouched() {
        let (store, _dir) = redb_backed_store();

        // Non-backup buffered write on map "m".
        store
            .add("m", "k", &lww("primary", 1), 0, 1000)
            .await
            .unwrap();

        // Backup scan must not see the non-backup staging write.
        let backup_batch = store.scan_values("m", true, 0).await.unwrap();
        assert!(
            backup_batch.records.is_empty(),
            "backup scan must not overlay non-backup staging writes"
        );

        let mut sink = CollectLeaves { leaves: Vec::new() };
        store.enumerate_leaves("m", true, &mut sink).await.unwrap();
        assert!(
            sink.leaves.is_empty(),
            "backup enumerate must not overlay non-backup staging writes"
        );
    }

    // AC6 — list_maps unions staging-only maps; pending-delete-only maps are
    // excluded.
    #[tokio::test]
    async fn ac6_list_maps_union() {
        let (store, _dir) = redb_backed_store();

        // Map whose only write is buffered, never flushed.
        store
            .add("buffered_map", "k", &lww("v", 1), 0, 1000)
            .await
            .unwrap();
        // Map present in staging only via a pending delete.
        store.remove("delete_only_map", "k", 1000).await.unwrap();

        let maps = store.list_maps().await.unwrap();
        assert!(
            maps.contains(&"buffered_map".to_string()),
            "buffered-only map must appear in list_maps"
        );
        assert!(
            !maps.contains(&"delete_only_map".to_string()),
            "pending-delete-only map must not appear in list_maps"
        );
    }

    // AC7 — multi-batch scan over flushed + staging-only keys returns the full
    // logical key set exactly once, no boundary miss, no staging-only duplicate.
    #[tokio::test]
    async fn ac7_cursor_safe_merge() {
        let (store, _dir) = redb_backed_store();

        // Flush several keys to redb so the scan spans multiple small batches.
        for i in 0..6u32 {
            let k = format!("flushed_{i:02}");
            let v = lww(&format!("v{i}"), 1);
            store.add("m", &k, &v, 0, 1000).await.unwrap();
        }
        store.soft_flush().await.unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !store.test_is_idle() {
            assert!(
                std::time::Instant::now() < deadline,
                "the requested pass never flushed the keys"
            );
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
        // Add staging-only keys (buffered, never flushed) interleaved by name.
        for i in 0..4u32 {
            let k = format!("buffered_{i:02}");
            store.add("m", &k, &lww("b", 2), 0, 2000).await.unwrap();
        }

        // Force a tiny batch budget so the redb scan pages across many batches.
        let mut out: Vec<(String, RecordValue)> = Vec::new();
        let mut batch = store.scan_values("m", false, 1).await.unwrap();
        loop {
            out.append(&mut batch.records);
            match batch.next_cursor.take() {
                Some(cursor) => {
                    batch = store
                        .scan_values_batched("m", false, cursor, 1)
                        .await
                        .unwrap();
                }
                None => break,
            }
        }

        let mut keys: Vec<String> = out.iter().map(|(k, _)| k.clone()).collect();
        keys.sort();
        let mut expected: Vec<String> = (0..6)
            .map(|i| format!("flushed_{i:02}"))
            .chain((0..4).map(|i| format!("buffered_{i:02}")))
            .collect();
        expected.sort();
        assert_eq!(
            keys, expected,
            "merged scan must yield every key exactly once across batches"
        );
    }

    // AC8 — the flush-vs-coalesce race must not drop read-your-writes. When a
    // flush dequeues an older write (E1) and a newer write (E2) coalesces into
    // staging before E1's terminal staging removal runs, that removal must NOT
    // wipe E2's slot. White-box: drive the exact race window deterministically
    // (the 600s flush interval keeps the background loop out of the way), then
    // assert the read surface still serves the newer value. Reproduces the
    // active-eviction soak residual (`expected=2 actual=1`) at unit scope.
    #[tokio::test]
    async fn ac8_flush_does_not_clear_newer_coalesced_staging() {
        let (store, _dir) = redb_backed_store();

        // E1: write v1. Staging now holds (seq1, v1); queue holds E1.
        store.add("m", "k", &lww("v1", 1), 0, 1000).await.unwrap();
        let pid = partition_for("m", "k");
        // Drain E1 out of the queue (the flush loop's first step). The queue lock
        // is now released, opening the race window.
        let drained = store.queues.get_mut(&pid).unwrap().drain_ready(i64::MAX);
        assert_eq!(drained.len(), 1, "E1 must be the only drained entry");
        let e1_seq = drained[0].sequence;

        // E2: a newer write coalesces in. Queue is empty (E1 already drained), so
        // E2 enters as a new entry; staging is overwritten to (seq2, v2).
        store.add("m", "k", &lww("v2", 5), 0, 2000).await.unwrap();

        // E1's flush completes and tries to clear its staging slot. With the seq
        // guard it must NOT remove the newer E2 slot.
        let removed = store.clear_staging_if_current("m", "k", e1_seq);
        assert!(
            !removed,
            "E1's terminal flush must not clear E2's newer staging slot"
        );

        // Read-your-writes holds: load and scan both surface v2, not the stale
        // durable/None value that the old unconditional remove exposed.
        assert_lww(&store.load("m", "k").await.unwrap().unwrap(), "v2", 5);
        let rows = drain_scan(&store, "m").await;
        assert_eq!(rows.len(), 1, "exactly one row for the key");
        assert_lww(&rows[0].1, "v2", 5);

        // When E2 itself flushes, its own seq clears the slot (no leak).
        let e2 = store.queues.get_mut(&pid).unwrap().drain_ready(i64::MAX);
        assert_eq!(e2.len(), 1, "E2 must be drained");
        let e2_seq = e2[0].sequence;
        assert!(
            store.clear_staging_if_current("m", "k", e2_seq),
            "E2's own flush clears its staging slot"
        );
        assert!(
            store.staging.is_empty(),
            "no staging slot may leak after both writes flush"
        );
    }

    // An out-of-order (lower-seq) stage must not clobber a newer staged value.
    // Two concurrent same-key writers can interleave between `next_sequence()`
    // and the staging insert; `stage` keeps the slot monotonic by `seq` so the
    // later write's value is the one that survives.
    #[tokio::test]
    async fn stage_is_monotonic_by_seq() {
        let (store, _dir) = redb_backed_store();

        store.stage("m", "k", 10, Some(StagedValue::Value(lww("v10", 10))));
        // A higher seq replaces.
        store.stage("m", "k", 11, Some(StagedValue::Value(lww("v11", 11))));
        assert_lww(&store.load("m", "k").await.unwrap().unwrap(), "v11", 11);
        // A late, lower seq is rejected (does not clobber the newer value).
        store.stage("m", "k", 10, Some(StagedValue::Value(lww("v10", 10))));
        assert_lww(&store.load("m", "k").await.unwrap().unwrap(), "v11", 11);
        // Equal seq (same write, idempotent) is allowed.
        store.stage("m", "k", 11, None);
        assert!(
            store.load("m", "k").await.unwrap().is_none(),
            "equal-seq restage applies (idempotent same-write update)"
        );
    }

    // -----------------------------------------------------------------------
    // Watermark-mode and classifier seams
    // -----------------------------------------------------------------------

    fn seam_store() -> Arc<WriteBehindDataStore> {
        let inner: Arc<dyn MapDataStore> = Arc::new(SpyDataStore::new());
        let config = WriteBehindConfig {
            // Long delay so the flush loop never fires during these tests.
            write_delay_ms: 60_000,
            flush_interval_ms: 60_000,
            ..WriteBehindConfig::default()
        };
        WriteBehindDataStore::new(inner, config)
    }

    #[tokio::test]
    async fn watermark_mode_defaults_to_prefix_complete_and_is_selectable() {
        let store = seam_store();
        assert_eq!(
            store.watermark_mode(),
            WatermarkMode::PrefixComplete,
            "production semantics must be the default; scalar-max is only ever opted into"
        );

        store.test_set_watermark_mode(WatermarkMode::ScalarMax);
        assert_eq!(store.watermark_mode(), WatermarkMode::ScalarMax);

        store.test_set_watermark_mode(WatermarkMode::PrefixComplete);
        assert_eq!(store.watermark_mode(), WatermarkMode::PrefixComplete);
    }

    #[tokio::test]
    async fn classifier_sample_counts_and_runs_the_mid_sample_hook() {
        let store = seam_store();
        let partition = 7_u32;
        assert_eq!(store.test_classifier_sample_count(partition), 0);

        let seen = Arc::new(Mutex::new(Vec::<u32>::new()));
        let seen_hook = Arc::clone(&seen);
        store.test_set_classifier_hook(Arc::new(move |p: u32| {
            seen_hook
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(p);
        }));

        assert!(store.test_run_classifier_sample(partition).is_none());
        assert!(store.test_run_classifier_sample(partition).is_none());

        assert_eq!(
            store.test_classifier_sample_count(partition),
            2,
            "the counter is what proves a second sample was actually taken, so a \
             classifier that never fires cannot satisfy an assertion by doing nothing"
        );
        assert_eq!(
            *seen
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            vec![partition, partition],
            "the hook must run once inside every sample"
        );
        assert_eq!(
            store.test_classifier_sample_count(partition + 1),
            0,
            "samples are counted per partition"
        );
    }

    // -----------------------------------------------------------------------
    // Per-partition WAL-sequence tracker
    // -----------------------------------------------------------------------

    fn wal_seam_store(wal: &Arc<InMemoryTestWal>) -> Arc<WriteBehindDataStore> {
        let inner: Arc<dyn MapDataStore> = Arc::new(SpyDataStore::new());
        let config = WriteBehindConfig {
            // Long delays so nothing drains behind the assertions.
            write_delay_ms: 60_000,
            flush_interval_ms: 60_000,
            ..WriteBehindConfig::default()
        };
        WriteBehindDataStore::new_with_wal(
            inner,
            config,
            Some(WalBootstrap {
                wal: Arc::clone(wal) as Arc<dyn Wal>,
                sequence_start: 1,
            }),
        )
    }

    #[tokio::test]
    async fn watermark_stops_one_below_the_lowest_unresolved_sequence() {
        let wal = InMemoryTestWal::new();
        let store = wal_seam_store(&wal);
        let partition = partition_for("m", "k1");

        store.add("m", "k1", &dummy_value(), 0, 1000).await.unwrap();

        assert_eq!(
            store.test_pending_wal_sequences(partition),
            vec![(1, PendingOrigin::Live)],
            "the append returned Ok and the entry is queued, so an entry owns the sequence"
        );
        assert_eq!(store.test_max_assigned_wal_sequence(partition), 1);
        assert!(store.test_in_flight_wal_sequences(partition).is_empty());
        assert_eq!(
            store.test_wal_watermark(partition),
            Some(Ok(0)),
            "the lowest pending sequence is itself UNRESOLVED, so the inclusive \
             watermark stops one below it — marking it would filter a buffered \
             frame out of replay"
        );

        store
            .resolve_and_advance(partition, &BTreeSet::from([1]))
            .await;

        assert_eq!(
            store.test_wal_watermark(partition),
            Some(Ok(1)),
            "nothing pending ⇒ every assigned sequence resolved ⇒ the highest \
             ASSIGNED, never the counter's next value"
        );
        assert_eq!(
            store.wal_sequence.load(Ordering::Relaxed),
            2,
            "fetch_add returns the OLD value, so the counter's load is the NEXT \
             sequence to assign; using it would pre-mark an unwritten frame applied"
        );
    }

    #[tokio::test]
    async fn scalar_max_mode_advances_past_an_unresolved_lower_sequence() {
        let wal = InMemoryTestWal::new();
        let applied = Arc::clone(&wal.applied);
        let store = wal_seam_store(&wal);
        let partition = 11_u32;

        let low = store.assign_wal_sequence(partition);
        let high = store.assign_wal_sequence(partition);
        store.promote_wal_sequence(partition, low);
        store.promote_wal_sequence(partition, high);

        store.test_set_watermark_mode(WatermarkMode::ScalarMax);
        store
            .resolve_and_advance(partition, &BTreeSet::from([high]))
            .await;
        assert_eq!(
            *applied.lock().await,
            vec![(partition, high)],
            "the negative control must actually discriminate: scalar-max marks the \
             still-buffered lower frame applied"
        );

        store.test_set_watermark_mode(WatermarkMode::PrefixComplete);
        store.resolve_and_advance(partition, &BTreeSet::new()).await;
        assert_eq!(
            applied.lock().await.last().copied(),
            Some((partition, low - 1)),
            "prefix-complete holds below the unresolved sequence"
        );
    }

    #[tokio::test]
    async fn rollback_drops_only_the_failed_sequence() {
        let wal = InMemoryTestWal::new();
        let store = wal_seam_store(&wal);
        let partition = 5_u32;

        let framed = store.assign_wal_sequence(partition);
        let failed = store.assign_wal_sequence(partition);
        store.promote_wal_sequence(partition, framed);

        store.rollback_wal_sequence(partition, failed);

        assert_eq!(
            store.test_pending_wal_sequences(partition),
            vec![(framed, PendingOrigin::Live)],
            "a frame-backed sequence must survive a later key's append failure — \
             rolling it back would let the watermark pass a frame never applied"
        );
        assert_eq!(
            store.test_max_assigned_wal_sequence(partition),
            failed,
            "max_assigned stays monotonic across a rollback: a rolled-back \
             sequence names no frame, so a watermark at that value asserts nothing"
        );
    }

    #[tokio::test]
    async fn promotion_is_a_no_op_once_the_sequence_reached_a_terminal() {
        let wal = InMemoryTestWal::new();
        let store = wal_seam_store(&wal);
        let partition = 9_u32;

        let seq = store.assign_wal_sequence(partition);
        // The flush task is separate and a coalesced entry is immediately
        // drain-eligible, so a terminal can land inside the assign→promote window.
        store
            .resolve_and_advance(partition, &BTreeSet::from([seq]))
            .await;
        store.promote_wal_sequence(partition, seq);

        assert!(
            store.test_pending_wal_sequences(partition).is_empty(),
            "re-inserting a sequence that already reached a durable terminal would \
             pin the watermark forever and report a leak against correct code"
        );
    }

    #[tokio::test]
    async fn no_wal_means_no_tracker_but_the_counter_still_advances() {
        let store = seam_store();
        let partition = 4_u32;

        assert!(
            store.test_wal_watermark(partition).is_none(),
            "with no WAL no frame is ever written, so the watermark has no subject"
        );
        assert_eq!(store.assign_wal_sequence(partition), 1);
        assert_eq!(store.assign_wal_sequence(partition), 2);
        assert!(
            store.test_pending_wal_sequences(partition).is_empty(),
            "the sequences are inert identifiers; nothing tracks them"
        );
    }

    #[tokio::test]
    async fn boot_seeding_holds_the_watermark_below_a_prior_incarnation_frame() {
        let wal = InMemoryTestWal::new();
        let partition = partition_for("m", "k1");
        // A previous process appended this frame and died before it was applied.
        // Recovery replays such frames straight into the inner store, so nothing
        // else would ever tell this store the frame exists.
        wal.appended.lock().await.push((
            partition,
            WalEntry {
                map: "m".to_string(),
                key: "stranded".to_string(),
                op: WalOp::Remove,
                timestamp: None,
                sequence: 7,
            },
        ));

        let inner: Arc<dyn MapDataStore> = Arc::new(SpyDataStore::new());
        let store = WriteBehindDataStore::new_with_wal(
            inner,
            WriteBehindConfig {
                write_delay_ms: 60_000,
                flush_interval_ms: 60_000,
                ..WriteBehindConfig::default()
            },
            Some(WalBootstrap {
                wal: Arc::clone(&wal) as Arc<dyn Wal>,
                sequence_start: 100,
            }),
        );

        assert!(!store.test_wal_partition_seeded(partition));
        assert_eq!(
            store.test_wal_watermark(partition),
            Some(Err(WalWatermarkError::Unseeded)),
            "an empty map that has not been seeded says nothing about what a \
             prior incarnation left behind, so it must not answer the watermark"
        );

        store.add("m", "k1", &dummy_value(), 0, 1000).await.unwrap();

        assert!(store.test_wal_partition_seeded(partition));
        assert_eq!(
            store.test_pending_wal_sequences(partition),
            vec![
                (7, PendingOrigin::BootUnreplayed),
                (100, PendingOrigin::Live)
            ],
            "the stranded frame is pending in this incarnation even though no \
             entry owns it; it resolves only by replaying on a future restart"
        );
        assert_eq!(
            store.test_wal_watermark(partition),
            Some(Ok(6)),
            "the watermark stops below the stranded frame — advancing past it \
             would filter it out of the next replay and free its segment"
        );
    }

    #[test]
    fn alarm_classes_carry_their_cause() {
        // The origin is load-bearing, not decoration: a hung store and a hung WAL
        // append both classify as an abandoned write, and only the tag says which
        // of the two an operator must go and look at.
        assert_ne!(
            WalWatermarkAlarm::AbandonedWrite {
                origin: PendingOrigin::Appending,
            },
            WalWatermarkAlarm::AbandonedWrite {
                origin: PendingOrigin::Live,
            }
        );
        assert_ne!(
            WalWatermarkAlarm::TrackerLeak,
            WalWatermarkAlarm::AbandonedWrite {
                origin: PendingOrigin::BootUnreplayed,
            }
        );
        assert_eq!(
            WalWatermarkAlarm::AbandonedWrite {
                origin: PendingOrigin::Abandoned,
            },
            WalWatermarkAlarm::AbandonedWrite {
                origin: PendingOrigin::Abandoned,
            }
        );
    }

    // -----------------------------------------------------------------------
    // The store boundary's emission behaviour, over a REAL write-ahead log
    // -----------------------------------------------------------------------
    //
    // Driven against a real `WalWriter` rather than a recording double. The
    // append-time refusal that used to reject a delta frame lived in the
    // concrete writer, not in the log trait, so a spy log would have recorded
    // whatever it was handed and could never have fail-stopped — a proof built
    // on one would be green on an instrument incapable of going red.

    /// The 64-byte payload both framing arms carry, so a size comparison
    /// measures the FRAMING and not two different values.
    const PINNED_64_BYTE_VALUE: &str =
        "the very same sixty-four byte utf8 payload on both sides of it!!";
    /// A 16-character tag, the width the OR entry pins.
    const PINNED_16_CHAR_TAG: &str = "tag-0123456789ab";

    fn pinned_or_entry() -> OrMapEntry {
        OrMapEntry {
            value: Value::String(PINNED_64_BYTE_VALUE.to_string()),
            tag: PINNED_16_CHAR_TAG.to_string(),
            timestamp: Timestamp {
                millis: 1_700_000_000_000,
                counter: 7,
                node_id: "node-a".to_string(),
            },
        }
    }

    fn pinned_or_snapshot() -> RecordValue {
        RecordValue::OrMap {
            records: vec![pinned_or_entry()],
            tombstones: Vec::new(),
        }
    }

    /// A store bound to a real log at `dir`, framing under `armed`.
    ///
    /// The delays are long so the frame stays un-applied for the read-back:
    /// the watermark advancing would let segment reclamation remove exactly the
    /// frames under assertion. Durability does not depend on them — the per-op
    /// policy fsyncs each frame before its write acks, so a read-back cannot
    /// pass by observing an empty segment.
    fn store_over_a_real_wal(
        dir: &std::path::Path,
        armed: bool,
    ) -> (Arc<dyn Wal>, Arc<WriteBehindDataStore>) {
        use crate::storage::wal::WalWriter;

        let wal = WalWriter::new(dir.to_path_buf(), WalFsyncPolicy::PerOp).unwrap();
        let store = WriteBehindDataStore::new_with_wal(
            Arc::new(crate::storage::datastores::null::NullDataStore) as Arc<dyn MapDataStore>,
            WriteBehindConfig {
                write_delay_ms: 60_000,
                flush_interval_ms: 60_000,
                shutdown_timeout_ms: 5_000,
                or_delta_wal: armed,
                ..WriteBehindConfig::default()
            },
            Some(WalBootstrap {
                wal: Arc::clone(&wal) as Arc<dyn Wal>,
                sequence_start: 1,
            }),
        );
        (wal as Arc<dyn Wal>, store)
    }

    /// Every frame the log directory holds, read back off disk.
    ///
    /// Segments are selected by the log's OWN name parser rather than by
    /// guessing an extension: the directory also holds per-partition
    /// applied-watermark files, which carry no frames.
    fn frames_on_disk(dir: &std::path::Path) -> Vec<WalEntry> {
        use crate::storage::wal::format::{decode_all, FrameDecodeResult};
        use crate::storage::wal::segment::parse_segment_filename;

        let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.is_file()
                    && p.file_name()
                        .and_then(|n| n.to_str())
                        .and_then(parse_segment_filename)
                        .is_some()
            })
            .collect();
        paths.sort();

        let mut out = Vec::new();
        for path in paths {
            let bytes = std::fs::read(&path).unwrap();
            let entries = match decode_all(&bytes) {
                FrameDecodeResult::Complete(entries) => entries,
                FrameDecodeResult::TruncatedTail { complete } => complete,
                FrameDecodeResult::CleanEof => Vec::new(),
                other => panic!("unexpected decode result for {}: {other:?}", path.display()),
            };
            out.extend(entries);
        }
        out
    }

    /// The per-op OR frame is no larger than three times the mean full-record
    /// LWW frame, on a payload shape pinned so the ratio measures framing.
    ///
    /// Both frames carry the SAME 64-byte string; the OR side additionally
    /// carries its 16-character tag and its own timestamp, which is the honest
    /// per-op floor rather than a favourable one. Sizes are the whole encoded
    /// entry, CRC and header included, because that is what reaches the disk.
    #[test]
    fn a_per_op_or_frame_costs_no_more_than_three_full_record_lww_frames() {
        use crate::storage::wal::format::encode;

        let lww = WalEntry {
            map: "rooms".to_string(),
            key: "lobby".to_string(),
            op: WalOp::Store {
                value: WalStorePayload::Record(RecordValue::Lww {
                    value: Value::String(PINNED_64_BYTE_VALUE.to_string()),
                    timestamp: Timestamp {
                        millis: 1_700_000_000_000,
                        counter: 7,
                        node_id: "node-a".to_string(),
                    },
                }),
                expiration_time: None,
            },
            timestamp: None,
            sequence: 11,
        };
        let or = WalEntry {
            map: "rooms".to_string(),
            key: "lobby".to_string(),
            op: WalOp::OrDelta {
                delta: OrDelta::Add {
                    entry: pinned_or_entry(),
                },
            },
            timestamp: None,
            sequence: 11,
        };

        assert_eq!(
            PINNED_64_BYTE_VALUE.len(),
            64,
            "the comparison is only meaningful on the payload width it pins"
        );
        assert_eq!(PINNED_16_CHAR_TAG.chars().count(), 16);

        let lww_bytes = encode(&lww).unwrap().len();
        let or_bytes = encode(&or).unwrap().len();
        // Frame sizes are hundreds of bytes, far inside f64's exact-integer
        // range, so the ratio is reported rather than asserted on.
        #[allow(clippy::cast_precision_loss)]
        let ratio = or_bytes as f64 / lww_bytes as f64;
        eprintln!("frame sizes: lww={lww_bytes}B or_delta={or_bytes}B ratio={ratio:.3}");
        assert!(
            or_bytes <= lww_bytes * 3,
            "a per-op OR frame must not reamplify: lww={lww_bytes}B or_delta={or_bytes}B \
             ratio={ratio:.3}"
        );
    }

    /// An armed store hands the witness straight to the log, and the process
    /// survives doing it.
    ///
    /// Surviving is the assertion, not a side effect: the retired append guard
    /// aborted the process on the first delta frame it saw, so a build that
    /// still carried it would take this test's whole harness down rather than
    /// fail it.
    #[tokio::test]
    async fn an_armed_store_frames_the_witness_it_is_handed() {
        let dir = tempfile::tempdir().unwrap();
        let (_wal, store) = store_over_a_real_wal(dir.path(), true);
        // The demand gate and the emission arm cannot diverge: an armed store
        // that framed a delta while telling the mutation point not to build one
        // would be an emitter fed nothing, which is the shape that looks
        // complete and moves no bytes.
        assert!(
            store.wants_or_witness(),
            "an armed store must ASK for the witness it is about to frame"
        );
        let delta = OrDelta::Add {
            entry: pinned_or_entry(),
        };

        store
            .add_with_witness(
                "rooms",
                "lobby",
                WriteSource::Value(&pinned_or_snapshot()),
                0,
                1_700_000_000_000,
                Some(&delta),
            )
            .await
            .unwrap();

        let frames = frames_on_disk(dir.path());
        // Non-vacuity: a claim quantified over zero frames is free.
        assert!(
            !frames.is_empty(),
            "at least one frame must have been decoded, or the claim below is vacuous"
        );
        let deltas: Vec<&WalEntry> = frames
            .iter()
            .filter(|e| matches!(e.op, WalOp::OrDelta { .. }))
            .collect();
        assert_eq!(
            deltas.len(),
            1,
            "an armed witness-carrying write frames exactly one delta, got {frames:?}"
        );
        match &deltas[0].op {
            WalOp::OrDelta {
                delta: on_disk_payload,
            } => assert_eq!(
                on_disk_payload, &delta,
                "the framed delta must be the witness the caller handed over, not a \
                 re-derivation of it"
            ),
            other => panic!("filtered for a delta frame, got {other:?}"),
        }
    }

    /// The delta arm shrinks the FRAME and nothing else: the queued write-behind
    /// entry and the durable store write still carry the full record.
    ///
    /// Without this the delta framing would be a data-loss change dressed as an
    /// encoding change — the inner store would receive a mutation instead of the
    /// state every reader of it expects.
    #[tokio::test]
    async fn a_delta_frame_never_shrinks_what_is_queued_or_stored() {
        let dir = tempfile::tempdir().unwrap();
        let inner = Arc::new(SpyDataStore::new());
        let wal =
            crate::storage::wal::WalWriter::new(dir.path().to_path_buf(), WalFsyncPolicy::PerOp)
                .unwrap();
        let store = WriteBehindDataStore::new_with_wal(
            Arc::clone(&inner) as Arc<dyn MapDataStore>,
            WriteBehindConfig {
                write_delay_ms: 0,
                flush_interval_ms: 5,
                shutdown_timeout_ms: 5_000,
                or_delta_wal: true,
                ..WriteBehindConfig::default()
            },
            Some(WalBootstrap {
                wal: Arc::clone(&wal) as Arc<dyn Wal>,
                sequence_start: 1,
            }),
        );

        let snapshot = pinned_or_snapshot();
        store
            .add_with_witness(
                "rooms",
                "lobby",
                WriteSource::Value(&snapshot),
                0,
                1_700_000_000_000,
                Some(&OrDelta::Add {
                    entry: pinned_or_entry(),
                }),
            )
            .await
            .unwrap();

        // The value the store serves before any flush is the full record, which
        // is the queued entry read back through the only accessor that exists.
        assert_eq!(
            store.load("rooms", "lobby").await.unwrap(),
            Some(snapshot.clone()),
            "the queued entry must carry the full record, not the delta"
        );

        store.hard_flush().await.unwrap();
        assert_eq!(
            inner.persisted("rooms", "lobby"),
            Some(snapshot),
            "the durable store must receive the full record, not the delta"
        );
    }

    /// A store handed no witness frames the full snapshot, and so does a store
    /// whose kill-switch is off even when it IS handed one.
    ///
    /// The two halves are asserted against each other because the rollback claim
    /// is unconditional: the flag is the authority and the witness is evidence,
    /// so a caller that bypasses the demand gate still gets today's framing. A
    /// branch keyed on the witness alone would read no flag at all and make the
    /// store disagree with its own rollback switch — and the disagreement would
    /// be invisible from either half on its own.
    #[tokio::test]
    async fn the_rollback_switch_frames_a_snapshot_whatever_the_caller_hands_it() {
        let delta = OrDelta::Add {
            entry: pinned_or_entry(),
        };
        let mut baselines = Vec::new();
        for (armed, witness) in [(false, None), (false, Some(&delta)), (true, None)] {
            let dir = tempfile::tempdir().unwrap();
            let (_wal, store) = store_over_a_real_wal(dir.path(), armed);
            assert_eq!(
                store.wants_or_witness(),
                armed,
                "the demand gate must answer the SAME flag the framing branch reads, or \
                 the write path pays for a witness the store then throws away"
            );
            store
                .add_with_witness(
                    "rooms",
                    "lobby",
                    WriteSource::Value(&pinned_or_snapshot()),
                    0,
                    1_700_000_000_000,
                    witness,
                )
                .await
                .unwrap();

            let frames = frames_on_disk(dir.path());
            assert_eq!(
                frames.len(),
                1,
                "armed={armed} witness={}: exactly one frame is expected",
                witness.is_some()
            );
            assert!(
                matches!(frames[0].op, WalOp::Store { .. }),
                "armed={armed} witness={}: must frame the full snapshot, got {:?}",
                witness.is_some(),
                frames[0].op
            );
            // Nothing here diffs anything: the caller's witness is the only delta
            // that exists, and this arm ignores it entirely.
            baselines.push(crate::storage::wal::format::encode(&frames[0]).unwrap());
        }

        assert_eq!(
            baselines[0], baselines[1],
            "the kill-switch is unconditional: a bypassing caller's `Some` witness must \
             produce the same bytes as the witness-free write, or the rollback is only \
             approximately byte-identical"
        );
        assert_eq!(
            baselines[0], baselines[2],
            "an armed store handed no witness must frame the same snapshot bytes as the \
             rolled-back store, or bulk, SYNC, read-repair and rehydrate have changed \
             representation without asking"
        );
    }

    /// The arming flag is read in exactly two expression positions, and the
    /// demand predicate names no other.
    ///
    /// A second switch — another config field, an env read, an atomic — could
    /// otherwise let the demand gate and the emission arm diverge, which is the
    /// shape that produces a predicate answering `true` while every witness
    /// still arrives `None`.
    #[test]
    fn one_flag_arms_both_the_demand_gate_and_the_framing_branch() {
        const SOURCE: &str = include_str!("write_behind.rs");
        let marker = "#[cfg(test)]\nmod tests {";
        let production = &SOURCE[..SOURCE.find(marker).expect("inline tests module")];
        let flag = concat!("self.config.", "or_delta_wal");

        assert_eq!(
            production.matches(flag).count(),
            2,
            "the arming flag must be read in exactly two expression positions -- the \
             demand predicate's body and the guard on the framing branch's delta arm"
        );
        assert!(
            production.contains(concat!("Some(delta) if self.config.", "or_delta_wal =>")),
            "the delta arm must be GUARDED BY THE FLAG, not by the witness alone: a branch \
             that reads no flag cannot be rolled back. The guard also carries the witness's \
             presence as a binding, so no arm of this match can unwrap it"
        );

        let predicate_start = production
            .find(concat!("fn wants_or_", "witness(&self) -> bool {"))
            .expect("the demand predicate is overridden here");
        let body = &production[predicate_start..];
        let body = &body[..body.find("\n    }").expect("the predicate's body ends")];
        assert_eq!(
            body.matches(flag).count(),
            1,
            "the demand predicate must return the arming flag and nothing else"
        );
        for other in ["env::var", "AtomicBool", "OnceLock"] {
            assert!(
                !body.contains(other),
                "the demand predicate must name no second switch, found {other}"
            );
        }
    }

    /// Writes a real WAL, at this tree's framing, for the cross-binary
    /// compatibility script to hand to an older binary.
    ///
    /// A FIXTURE GENERATOR, not a proof — hence `#[ignore]`: the property it
    /// serves is asserted by `scripts/wal-compat-worktree.sh`, which runs a
    /// binary built from another commit and so cannot live inside this process.
    /// The in-process crash model runs ONE binary by construction.
    ///
    /// `TG_WAL_COMPAT_POSITION` selects where the delta frame sits, because a
    /// decoder that does not know the variant treats a trailing frame as a torn
    /// tail and a mid-segment one as corruption.
    #[tokio::test]
    #[ignore = "fixture generator for scripts/wal-compat-worktree.sh"]
    async fn emit_a_wal_this_tree_wrote_for_the_cross_binary_compat_script() {
        let out = std::env::var("TG_WAL_COMPAT_OUT")
            .expect("TG_WAL_COMPAT_OUT must name the directory to write the WAL into");
        let trailing = std::env::var("TG_WAL_COMPAT_POSITION")
            .as_deref()
            .unwrap_or("trailing")
            == "trailing";
        let dir = std::path::PathBuf::from(&out);
        std::fs::create_dir_all(&dir).unwrap();

        let (_wal, store) = store_over_a_real_wal(&dir, true);
        store
            .add_with_witness(
                "rooms",
                "lobby",
                WriteSource::Value(&pinned_or_snapshot()),
                0,
                1_700_000_000_000,
                Some(&OrDelta::Add {
                    entry: pinned_or_entry(),
                }),
            )
            .await
            .unwrap();
        if !trailing {
            // A frame the old decoder understands, appended AFTER the delta, so
            // the delta is no longer the last frame in the byte stream.
            store
                .add(
                    "rooms",
                    "lobby",
                    &RecordValue::Lww {
                        value: Value::String("after-the-delta".to_string()),
                        timestamp: Timestamp {
                            millis: 1_700_000_000_001,
                            counter: 0,
                            node_id: "node-a".to_string(),
                        },
                    },
                    0,
                    1_700_000_000_001,
                )
                .await
                .unwrap();
        }

        let frames = frames_on_disk(&dir);
        assert!(
            frames.iter().any(|e| matches!(e.op, WalOp::OrDelta { .. })),
            "the generated WAL must actually carry a delta frame, or the script proves \
             nothing about one"
        );
        eprintln!("TG_WAL_COMPAT_EMITTED {} frames into {out}", frames.len());
    }

    /// The emitted frame is the checked-in on-disk shape, byte-for-byte, once
    /// the one identity the log assigns is normalized away.
    ///
    /// The fixture is a fully concrete frame behind a CRC over all of it, and
    /// the writer assigns its own sequence, so whole-frame equality would be RED
    /// on identity rather than on shape. Normalizing the sequence — and only the
    /// sequence — keeps the comparison genuinely byte-level on everything the
    /// fixture pins. An added field, a renamed key, a changed serde tag or a
    /// bumped version reddens here; a different sequence must not.
    #[tokio::test]
    async fn the_emitted_frame_is_the_golden_on_disk_shape() {
        use crate::storage::wal::format::{decode_all, encode, FrameDecodeResult};

        const GOLDEN: &[u8] = include_bytes!("../wal/fixtures/or_delta_frame_v1.msgpack");

        let golden = match decode_all(GOLDEN) {
            FrameDecodeResult::Complete(entries) => {
                assert_eq!(entries.len(), 1, "the fixture holds exactly one frame");
                entries.into_iter().next().expect("one frame")
            }
            other => panic!("the fixture must decode Complete, got {other:?}"),
        };
        let golden_delta = match &golden.op {
            WalOp::OrDelta { delta } => delta.clone(),
            other => panic!("the fixture must carry a delta frame, got {other:?}"),
        };

        let dir = tempfile::tempdir().unwrap();
        let (_wal, store) = store_over_a_real_wal(dir.path(), true);
        store
            .add_with_witness(
                &golden.map,
                &golden.key,
                WriteSource::Value(&pinned_or_snapshot()),
                0,
                1_700_000_000_000,
                Some(&golden_delta),
            )
            .await
            .unwrap();

        let frames = frames_on_disk(dir.path());
        assert_eq!(frames.len(), 1, "exactly one frame is expected");
        let emitted = &frames[0];

        assert_eq!(
            emitted.op, golden.op,
            "the emitter's frame must carry the fixture's payload shape"
        );
        assert_eq!(
            emitted.timestamp, golden.timestamp,
            "an OR frame carries no log-level timestamp: its entries carry their own"
        );

        let mut normalized_golden = golden.clone();
        normalized_golden.sequence = emitted.sequence;
        assert_eq!(
            encode(&normalized_golden).unwrap(),
            encode(emitted).unwrap(),
            "the emitter encodes different bytes than the checked-in on-disk shape. A \
             shape change is a format decision that re-proves the reader first -- it is \
             never a reason to regenerate the fixture"
        );
    }

    // -----------------------------------------------------------------------
    // In-place writes share one cell with the queue and staging (TG-WB-003)
    // -----------------------------------------------------------------------

    fn lww_cell(value: &str, millis: u64) -> Arc<SlotCell> {
        crate::storage::engine::new_slot_cell(crate::storage::record::Record {
            value: lww(value, millis),
            metadata: crate::storage::record::RecordMetadata::new(0, 0),
        })
    }

    async fn add_cell(store: &WriteBehindDataStore, key: &str, cell: &Arc<SlotCell>) {
        store
            .add_with_witness("m", key, WriteSource::Cell(cell), 0, 1, None)
            .await
            .expect("add_with_witness");
    }

    /// The queued entry, the staging slot and `load_slot` all hold the writer's
    /// cell, reads see a mutation made after the enqueue, and the flush
    /// persists the cell's state at flush time and then drops every reference
    /// but the writer's.
    #[tokio::test]
    async fn an_in_place_write_shares_its_cell_with_the_queue_and_staging() {
        let (store, _dir) = redb_backed_store();
        let cell = lww_cell("a", 1);
        add_cell(&store, "k", &cell).await;

        let pending = store.test_pending_cell("m", "k").expect("queued cell");
        let staged = store.test_staged_cell("m", "k").expect("staged cell");
        assert!(
            Arc::ptr_eq(&pending, &cell),
            "the queue must hold the writer's cell"
        );
        assert!(
            Arc::ptr_eq(&staged, &cell),
            "staging must hold the writer's cell"
        );
        drop((pending, staged));
        match store.load_slot("m", "k").await.expect("load_slot") {
            Some(Loaded::Cell(loaded)) => {
                assert!(
                    Arc::ptr_eq(&loaded, &cell),
                    "load_slot must answer the staged cell"
                );
            }
            _ => panic!("a staged in-place write must load as its cell"),
        }

        cell.lock().value = lww("b", 2);
        assert_lww(
            &store.load("m", "k").await.expect("load").expect("staged"),
            "b",
            2,
        );

        store.hard_flush().await.expect("hard_flush");
        assert_lww(
            &store
                .inner
                .load("m", "k")
                .await
                .expect("inner load")
                .expect("persisted"),
            "b",
            2,
        );
        assert!(store.test_pending_cell("m", "k").is_none());
        assert!(store.test_staged_cell("m", "k").is_none());
        assert_eq!(
            Arc::strong_count(&cell),
            1,
            "the flush must release the cell"
        );
    }

    /// A value write over a queued cell replaces both the queue entry and the
    /// staging slot, so neither keeps the cell and later reads load the value.
    #[tokio::test]
    async fn a_value_write_over_a_queued_cell_displaces_it() {
        let (store, _dir) = redb_backed_store();
        let cell = lww_cell("a", 1);
        add_cell(&store, "k", &cell).await;
        store.add("m", "k", &lww("c", 3), 0, 2).await.expect("add");

        assert!(store.test_pending_cell("m", "k").is_none());
        assert!(store.test_staged_cell("m", "k").is_none());
        assert_eq!(
            Arc::strong_count(&cell),
            1,
            "the value write must drop the cell"
        );
        match store.load_slot("m", "k").await.expect("load_slot") {
            Some(Loaded::Value(value)) => assert_lww(&value, "c", 3),
            _ => panic!("a staged value write must load as a value"),
        }

        store.hard_flush().await.expect("hard_flush");
        assert_lww(
            &store
                .inner
                .load("m", "k")
                .await
                .expect("inner load")
                .expect("persisted"),
            "c",
            3,
        );
    }

    /// A remove over a queued cell drops the queue's and staging's references,
    /// and `load_slot` answers the pending delete, so the cell is never
    /// re-adopted.
    #[tokio::test]
    async fn a_remove_over_a_queued_cell_drops_it() {
        let (store, _dir) = redb_backed_store();
        let cell = lww_cell("a", 1);
        add_cell(&store, "k", &cell).await;
        store.remove("m", "k", 2).await.expect("remove");

        assert_eq!(Arc::strong_count(&cell), 1, "the remove must drop the cell");
        assert!(store
            .load_slot("m", "k")
            .await
            .expect("load_slot")
            .is_none());
    }

    // -----------------------------------------------------------------------
    // A cell flushed to an inner store that does not accept encoded records
    // -----------------------------------------------------------------------

    fn or_cell(tags: &[&str]) -> Arc<SlotCell> {
        crate::storage::engine::new_slot_cell(crate::storage::record::Record {
            value: or_value(tags),
            metadata: crate::storage::record::RecordMetadata::new(0, 0),
        })
    }

    fn or_value(tags: &[&str]) -> RecordValue {
        RecordValue::OrMap {
            records: tags.iter().map(|tag| or_entry(tag)).collect(),
            tombstones: Vec::new(),
        }
    }

    fn or_entry(tag: &str) -> OrMapEntry {
        OrMapEntry {
            value: Value::Null,
            tag: tag.to_string(),
            timestamp: Timestamp {
                millis: 1,
                counter: 0,
                node_id: "n".to_string(),
            },
        }
    }

    /// A non-encoding inner store takes the clone branch of the flush: each
    /// flushed cell entry reaches `inner.add` exactly once, with the value the
    /// cell holds at flush time — a mutation made after the enqueue included —
    /// and never reaches `add_encoded`, whose default body refuses (a refused
    /// flush would leave the entry queued and nothing persisted).
    #[tokio::test]
    async fn a_non_encoding_inner_receives_each_flushed_cell_once_with_its_current_value() {
        let spy = Arc::new(SpyDataStore::new());
        assert!(
            !spy.accepts_encoded(),
            "precondition: the inner store does not accept encoded records"
        );
        let store = WriteBehindDataStore::new(
            Arc::clone(&spy) as Arc<dyn MapDataStore>,
            WriteBehindConfig {
                write_delay_ms: 600_000,
                flush_interval_ms: 600_000,
                capacity: 0,
                ..WriteBehindConfig::default()
            },
        );
        let first = or_cell(&["a"]);
        let second = or_cell(&["x"]);
        add_cell(&store, "k1", &first).await;
        add_cell(&store, "k2", &second).await;
        // A second in-place write on k1 coalesces into k1's one queued entry.
        first.lock().value = or_value(&["a", "b"]);
        add_cell(&store, "k1", &first).await;
        // Mutated after its last enqueue: the flush must still see it.
        first.lock().value = or_value(&["a", "b", "c"]);

        store.hard_flush().await.expect("hard_flush");

        let adds: Vec<String> = spy
            .calls()
            .lock()
            .unwrap()
            .iter()
            .filter_map(|call| match call {
                SpyCall::Add { key, .. } => Some(key.clone()),
                SpyCall::Remove { .. } => None,
            })
            .collect();
        let mut sorted = adds.clone();
        sorted.sort();
        assert_eq!(
            sorted,
            vec!["k1".to_string(), "k2".to_string()],
            "one inner add per flushed entry, got {adds:?}"
        );
        assert_eq!(
            spy.persisted("m", "k1"),
            Some(or_value(&["a", "b", "c"])),
            "the inner store must receive the cell's value at flush time"
        );
        assert_eq!(spy.persisted("m", "k2"), Some(or_value(&["x"])));
        assert_eq!(
            Arc::strong_count(&first),
            1,
            "the flush must release k1's cell"
        );
        assert_eq!(
            Arc::strong_count(&second),
            1,
            "the flush must release k2's cell"
        );
    }

    /// An inner store that persists nothing and allocates nothing in `add`: it
    /// counts the call and checks the value against the one it expects, so the
    /// bytes a flush allocates are the flush's own.
    #[cfg(feature = "count-alloc")]
    struct ReceivingStore {
        expected: RecordValue,
        adds: std::sync::atomic::AtomicU32,
        matched: std::sync::atomic::AtomicBool,
    }

    #[cfg(feature = "count-alloc")]
    #[async_trait]
    impl MapDataStore for ReceivingStore {
        async fn add(
            &self,
            _map: &str,
            _key: &str,
            value: &RecordValue,
            _expiration_time: i64,
            _now: i64,
        ) -> anyhow::Result<()> {
            self.adds.fetch_add(1, Ordering::Relaxed);
            self.matched
                .store(*value == self.expected, Ordering::Relaxed);
            Ok(())
        }

        async fn add_backup(
            &self,
            _map: &str,
            _key: &str,
            _value: &RecordValue,
            _expiration_time: i64,
            _now: i64,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn remove(&self, _map: &str, _key: &str, _now: i64) -> anyhow::Result<()> {
            Ok(())
        }

        async fn remove_backup(&self, _map: &str, _key: &str, _now: i64) -> anyhow::Result<()> {
            Ok(())
        }

        async fn load(&self, _map: &str, _key: &str) -> anyhow::Result<Option<RecordValue>> {
            Ok(None)
        }

        async fn load_all(
            &self,
            _map: &str,
            _keys: &[String],
        ) -> anyhow::Result<Vec<(String, RecordValue)>> {
            Ok(Vec::new())
        }

        async fn remove_all(&self, _map: &str, _keys: &[String]) -> anyhow::Result<()> {
            Ok(())
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

        fn is_null(&self) -> bool {
            false
        }
    }

    /// `(bytes, allocations)` one `value.clone()` of an OR slot of `n` entries
    /// allocates, and the same for one `persist_entry` of a cell entry holding
    /// that slot, flushed to a [`ReceivingStore`].
    #[cfg(feature = "count-alloc")]
    async fn cell_flush_allocations(n: usize) -> ((u64, u64), (u64, u64)) {
        let tags: Vec<String> = (0..n).map(|i| format!("{i:020}:0:node-a")).collect();
        let tags: Vec<&str> = tags.iter().map(String::as_str).collect();
        let value = or_value(&tags);
        let inner = ReceivingStore {
            expected: value.clone(),
            adds: std::sync::atomic::AtomicU32::new(0),
            matched: std::sync::atomic::AtomicBool::new(false),
        };
        let cell = crate::storage::engine::new_slot_cell(crate::storage::record::Record {
            value,
            metadata: crate::storage::record::RecordMetadata::new(0, 0),
        });
        let entry = DelayedEntry {
            map: "m".to_string(),
            key: "k".to_string(),
            operation: DelayedOp::StoreCell {
                cell: Arc::clone(&cell),
                expiration_time: 0,
            },
            store_time: 0,
            sequence: 1,
            retry_count: 0,
            next_attempt: None,
            first_refused: None,
            wal_sequences: BTreeSet::new(),
        };

        let read = || {
            let stats = stats_alloc::INSTRUMENTED_SYSTEM.stats();
            (stats.bytes_allocated as u64, stats.allocations as u64)
        };
        let delta =
            |before: (u64, u64), after: (u64, u64)| (after.0 - before.0, after.1 - before.1);

        let before = read();
        let copy = cell.lock().value.clone();
        let clone = delta(before, read());
        std::hint::black_box(copy);

        let before = read();
        persist_entry(&inner, &entry).await.expect("persist_entry");
        let flush = delta(before, read());

        assert_eq!(
            inner.adds.load(Ordering::Relaxed),
            1,
            "N={n}: one inner add per flushed entry"
        );
        assert!(
            inner.matched.load(Ordering::Relaxed),
            "N={n}: the inner add must receive the cell's value"
        );
        (clone, flush)
    }

    /// Flushing a cell entry to a store that does not accept encoded records
    /// copies the record exactly once: the flush's per-entry byte and
    /// allocation slopes equal those of one `RecordValue::clone()` of the same
    /// slot (a second copy would double them; the constant part — the boxed
    /// `add` future — does not grow with the slot).
    ///
    /// A local allocation proof, not a CI guard: CI never enables `count-alloc`,
    /// and the counters are process-global, so it is meaningful only when run
    /// alone and single-threaded:
    /// `cargo test --release -p topgun-server --lib --features count-alloc -- --ignored --test-threads=1 count_alloc_`
    #[cfg(feature = "count-alloc")]
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "local allocation proof: run single-threaded under count-alloc"]
    async fn count_alloc_cell_flush_to_a_non_encoding_inner_copies_once() {
        let (small_clone, small_flush) = cell_flush_allocations(1_000).await;
        let (large_clone, large_flush) = cell_flush_allocations(10_000).await;
        #[allow(clippy::cast_precision_loss)]
        let slope = |small: u64, large: u64| (large as f64 - small as f64) / 9_000.0;
        let clone_bytes = slope(small_clone.0, large_clone.0);
        let flush_bytes = slope(small_flush.0, large_flush.0);
        let clone_allocs = slope(small_clone.1, large_clone.1);
        let flush_allocs = slope(small_flush.1, large_flush.1);
        println!(
            "count_alloc_cell_flush clone N=1000 bytes={} allocs={} N=10000 bytes={} allocs={}",
            small_clone.0, small_clone.1, large_clone.0, large_clone.1
        );
        println!(
            "count_alloc_cell_flush flush N=1000 bytes={} allocs={} N=10000 bytes={} allocs={}",
            small_flush.0, small_flush.1, large_flush.0, large_flush.1
        );
        println!(
            "count_alloc_cell_flush slope clone={clone_bytes:.3} B/entry {clone_allocs:.3} allocs/entry \
             flush={flush_bytes:.3} B/entry {flush_allocs:.3} allocs/entry"
        );
        assert!(clone_bytes > 0.0, "the clone slope must be measurable");
        assert!(
            (flush_bytes - clone_bytes).abs() <= 0.1 * clone_bytes,
            "one clone per flushed entry: flush slope {flush_bytes:.3} B/entry must equal the \
             clone slope {clone_bytes:.3} B/entry within 10 %"
        );
        assert!(
            (flush_allocs - clone_allocs).abs() <= 0.1 * clone_allocs,
            "one clone per flushed entry: flush slope {flush_allocs:.3} allocs/entry must equal \
             the clone slope {clone_allocs:.3} allocs/entry within 10 %"
        );
    }

    /// TG-WB-004: while a refused entry waits out its retry backoff, the queue
    /// shard it was put back on must stay usable. A write takes that shard lock
    /// synchronously, so a guard kept alive across the backoff parks the
    /// writer's worker thread for the whole backoff — the first step of a node
    /// that stops serving.
    ///
    /// The probe write runs as a task of its own and the JOIN is bounded: a
    /// timeout polled around the write itself would block inside the write and
    /// never look at its timer, so the test would hang instead of failing.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn the_queue_shard_is_free_during_a_retry_backoff() {
        use std::time::{Duration, Instant};

        let spy = Arc::new(SpyDataStore::new());
        spy.set_fail_add(true);
        let calls = spy.calls();
        // First backoff = min(2000 * 2^1, 5000) = 4000 ms: long enough that the
        // probe and every later assertion run inside it.
        let store = WriteBehindDataStore::new(
            Arc::clone(&spy) as Arc<dyn MapDataStore>,
            WriteBehindConfig {
                write_delay_ms: 0,
                flush_interval_ms: 50,
                backoff_base_ms: 2000,
                ..WriteBehindConfig::default()
            },
        );

        // A second key of the same partition: the same DashMap key, hence the
        // same shard. Not the refused key itself, which would coalesce with the
        // entry under retry and reset it.
        let partition = partition_for("m", "k");
        let probe_key = (0..100_000_u32)
            .map(|i| format!("probe-{i}"))
            .find(|candidate| partition_for("m", candidate) == partition)
            .expect("no probe key shares the partition of the refused entry");
        let refused = ("m".to_string(), "k".to_string());
        let attempts = || {
            calls
                .lock()
                .unwrap()
                .iter()
                .filter(
                    |call| matches!(call, SpyCall::Add { map, key } if map == "m" && key == "k"),
                )
                .count()
        };

        store
            .add("m", "k", &dummy_value(), 0, now_millis())
            .await
            .expect("the refused entry is accepted into the buffer");

        // Wait until the first attempt has failed and the entry is back on its
        // queue. `try_get` never blocks: with the guard held across the backoff
        // it reads `Locked` for the whole backoff, otherwise it reads the
        // reinserted entry.
        let poll_deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let back_on_queue = match store.queues.try_get(&partition) {
                dashmap::try_result::TryResult::Locked => true,
                dashmap::try_result::TryResult::Present(queue) => queue
                    .entries
                    .get(&refused)
                    .is_some_and(|entry| entry.retry_count == 1),
                dashmap::try_result::TryResult::Absent => false,
            };
            if attempts() >= 1 && back_on_queue {
                break;
            }
            assert!(
                Instant::now() < poll_deadline,
                "the retry entry was never observed back on its queue"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let reached_backoff = Instant::now();

        let probe = {
            let store = Arc::clone(&store);
            tokio::spawn(async move {
                store
                    .add("m", &probe_key, &dummy_value(), 0, now_millis())
                    .await
            })
        };
        let joined = tokio::time::timeout(Duration::from_millis(1000), probe).await;
        assert!(
            joined.is_ok(),
            "a write to the partition under retry backoff did not complete within 1000 ms: \
             the queue shard guard is held across the backoff await"
        );

        joined
            .expect("bounded join")
            .expect("the probe task panicked")
            .expect("the probe write failed");

        assert_eq!(
            attempts(),
            1,
            "the probe must run inside the first backoff, before a second attempt"
        );
        assert!(
            reached_backoff.elapsed() < Duration::from_millis(3000),
            "the probe must finish inside the first backoff (4000 ms)"
        );

        let retry_count = store
            .queues
            .get(&partition)
            .and_then(|queue| queue.entries.get(&refused).map(|entry| entry.retry_count));
        assert_eq!(
            retry_count,
            Some(1),
            "the refused entry must still be queued for its second attempt"
        );
    }

    /// TG-WB-005: a refused entry that finds a newer write of its key on the
    /// queue is retired into it.
    ///
    /// While an entry is out of the queue — drained into the flush batch and
    /// waiting behind another key's store call — a write to the same key has
    /// nothing to coalesce with, so it is queued and counted as a new entry.
    /// When the older entry's attempt then fails, its count, its entry sequence
    /// and its WAL sequences must pass to that queued write. Dropping the entry
    /// instead strands all three until restart: the capacity check reads an
    /// inflated count, the flushed watermark stops, and the partition's WAL is
    /// never collected.
    ///
    /// Idle is read through `test_is_idle`, never as "every queue empty": a
    /// drained batch is in no queue.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[allow(clippy::too_many_lines)]
    async fn a_refused_entry_superseded_by_a_newer_write_is_retired_into_it() {
        use std::time::{Duration, Instant};

        let wal = InMemoryTestWal::new();
        let spy = Arc::new(SpyDataStore::new());
        spy.set_fail_add(true);
        let calls = spy.calls();
        // The stall bound puts the watchdog's first tick 60 s out, past every
        // wait below. The watchdog shares its leak-candidate state with the two
        // classifier samples at the end, so a background sample taken first
        // would change what those two report.
        let store = WriteBehindDataStore::new_with_wal(
            Arc::clone(&spy) as Arc<dyn MapDataStore>,
            WriteBehindConfig {
                write_delay_ms: 0,
                flush_interval_ms: 200,
                backoff_base_ms: 500,
                backoff_cap_ms: 5000,
                capacity: 0,
                wal_watermark_stall_bound_ms: 600_000,
                ..WriteBehindConfig::default()
            },
            Some(WalBootstrap {
                wal: Arc::clone(&wal) as Arc<dyn Wal>,
                sequence_start: 1,
            }),
        );
        let attempts = |wanted: &str| {
            calls
                .lock()
                .unwrap()
                .iter()
                .filter(
                    |call| matches!(call, SpyCall::Add { map, key } if map == "m" && key == wanted),
                )
                .count()
        };
        let pb = partition_for("m", "b");
        let b = ("m".to_string(), "b".to_string());

        // `a` is the older of the two, so it is first in the batch and `b`
        // waits behind it. The store call of `a` is held open: that is the
        // window in which the older `b` entry sits in the batch, and it lasts
        // until the test closes it, whatever the schedule of a retry is.
        let (entered, proceed) = spy.install_add_gate();
        let t = now_millis();
        store
            .add("m", "a", &dummy_value_with(1), 0, t - 10)
            .await
            .expect("a is accepted into the buffer");
        store
            .add("m", "b", &dummy_value_with(2), 0, t - 5)
            .await
            .expect("b is accepted into the buffer");

        tokio::time::timeout(Duration::from_secs(5), entered.notified())
            .await
            .expect("a was never attempted");
        let b_queued = store
            .queues
            .get(&pb)
            .is_some_and(|queue| queue.entries.contains_key(&b));
        assert!(
            !b_queued && attempts("b") == 0,
            "setup: b must be in the flush batch, out of its queue and not yet attempted"
        );

        // The newer write, acked while the older `b` entry is in the batch.
        let newer = dummy_value_with(3);
        store
            .add("m", "b", &newer, 0, now_millis())
            .await
            .expect("the newer b is accepted into the buffer");
        proceed.notify_one();

        let deadline = Instant::now() + Duration::from_secs(5);
        while attempts("b") < 1 {
            assert!(Instant::now() < deadline, "the older b was never attempted");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }

        // What the queued newer write owns once the older entry's attempt has
        // failed. Recorded, not asserted yet: the accounting below must be the
        // first thing a failing run reports.
        let mut carried = BTreeSet::new();
        let deadline = Instant::now() + Duration::from_millis(500);
        loop {
            if let Some(owned) = store.queues.get(&pb).and_then(|queue| {
                queue
                    .entries
                    .get(&b)
                    .map(|entry| entry.wal_sequences.clone())
            }) {
                carried = owned;
            }
            if carried == BTreeSet::from([2, 3]) || Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }

        spy.set_fail_add(false);
        let deadline = Instant::now() + Duration::from_secs(20);
        while !store.test_is_idle() {
            assert!(
                Instant::now() < deadline,
                "the store never became idle after the inner store healed"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let first_sample = store.test_run_classifier_sample(pb);
        let second_sample = store.test_run_classifier_sample(pb);

        assert_eq!(
            (
                store.pending_operation_count(),
                (store.flushed_watermark(), store.assigned_write_sequence()),
                store.test_pending_wal_sequences(pb),
                (first_sample, second_sample),
            ),
            (0, (3, 3), Vec::new(), (None, None)),
            "on a healed, idle store nothing may stay counted, fenced or pending: \
             (pending count, (flushed watermark, assigned sequence), pending WAL \
             sequences of b's partition, two classifier samples)"
        );
        assert_eq!(
            carried,
            BTreeSet::from([2, 3]),
            "the queued newer write must own the superseded entry's WAL sequence \
             while both are unresolved"
        );
        assert_eq!(
            spy.persisted("m", "b"),
            Some(newer),
            "the inner store must hold the newer write's value"
        );
    }

    /// Two writers of one key, interleaved so that the write holding the
    /// HIGHER entry sequence is queued first and the one holding the lower
    /// sequence then replaces it. Nothing serialises that pair for every
    /// caller today, so the store's accounting has to come out even under it:
    /// no count left, no fence held, no WAL sequence left pending.
    ///
    /// The order is exact, not raced: the first writer is parked between its
    /// sequence assignment and its queue insert, and the write delay keeps the
    /// flush loop from draining anything in between.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn two_writers_of_one_key_interleaved_between_sequence_and_insert_leave_nothing_unaccounted(
    ) {
        let wal = InMemoryTestWal::new();
        let spy = Arc::new(SpyDataStore::new());
        let store = WriteBehindDataStore::new_with_wal(
            Arc::clone(&spy) as Arc<dyn MapDataStore>,
            WriteBehindConfig {
                write_delay_ms: 600_000,
                capacity: 0,
                wal_watermark_stall_bound_ms: 600_000,
                ..WriteBehindConfig::default()
            },
            Some(WalBootstrap {
                wal: Arc::clone(&wal) as Arc<dyn Wal>,
                sequence_start: 1,
            }),
        );
        let partition = partition_for("m", "k");
        let k = ("m".to_string(), "k".to_string());
        let queued_sequence = || {
            store
                .queues
                .get(&partition)
                .and_then(|queue| queue.entries.get(&k).map(|entry| entry.sequence))
        };

        // X takes the lower sequence and parks before it reaches the queue.
        let mut park = store.test_park_before_queue_insert("m", "k");
        let x = {
            let store = Arc::clone(&store);
            tokio::spawn(async move {
                store
                    .add("m", "k", &dummy_value_with(1), 0, now_millis())
                    .await
            })
        };
        park.wait_parked().await;

        // Y runs to completion while X is parked: higher sequence, queued first.
        store
            .add("m", "k", &dummy_value_with(2), 0, now_millis())
            .await
            .expect("y is accepted into the buffer");
        assert_eq!(
            (queued_sequence(), store.assigned_write_sequence()),
            (Some(1), 2),
            "setup: y must be queued with the higher of the two sequences handed out"
        );

        park.release();
        x.await
            .expect("x did not panic")
            .expect("x is accepted into the buffer");
        let staged_sequence = store.staging.get(&k).map(|slot| slot.seq);
        assert_eq!(
            (queued_sequence(), staged_sequence),
            (Some(0), Some(1)),
            "setup: x must have replaced y in the queue while staging keeps y's higher sequence"
        );

        store
            .hard_flush()
            .await
            .expect("the drain stores what is queued");

        // Accounting only. Which value the inner store and staging end up with
        // is deliberately not asserted: in this interleaving the two are known
        // to differ, that is a defect of the callers that write one key
        // concurrently, and a green run here must not be read as covering it.
        assert_eq!(
            (
                store.pending_operation_count(),
                (store.flushed_watermark(), store.assigned_write_sequence()),
                store.test_pending_wal_sequences(partition),
            ),
            (0, (2, 2), vec![]),
            "an entry replaced by a lower-sequence write of its key must leave nothing unaccounted"
        );
    }

    // -----------------------------------------------------------------------
    // Placement of buffered keys in a resumable value scan (TG-WB-008)
    // -----------------------------------------------------------------------

    fn scan_rows(batch: &ScanBatch) -> Vec<String> {
        batch
            .records
            .iter()
            .map(|(key, value)| match value {
                RecordValue::Lww {
                    value: Value::String(v),
                    timestamp,
                } => format!("{key}={v}@{}", timestamp.millis),
                other => format!("{key}=?{other:?}"),
            })
            .collect()
    }

    /// Reads a scan of map `m` to its end, starting from its first batch.
    async fn scan_rest(
        store: &dyn MapDataStore,
        first: ScanBatch,
        budget: u64,
    ) -> Vec<Vec<String>> {
        let mut out = Vec::new();
        let mut batch = first;
        loop {
            out.push(scan_rows(&batch));
            match batch.next_cursor.take() {
                Some(cursor) => {
                    batch = store
                        .scan_values_batched("m", false, cursor, budget)
                        .await
                        .unwrap();
                }
                None => break,
            }
        }
        out
    }

    async fn scan_transcript(store: &dyn MapDataStore, budget: u64) -> Vec<Vec<String>> {
        let first = store.scan_values("m", false, budget).await.unwrap();
        scan_rest(store, first, budget).await
    }

    async fn flush_to_idle(store: &WriteBehindDataStore) {
        store.soft_flush().await.unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !store.test_is_idle() {
            assert!(
                std::time::Instant::now() < deadline,
                "the requested pass never reached idle"
            );
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
    }

    /// The stored size of two rows whose value is a two-character string.
    fn two_row_budget() -> u64 {
        let row = rmp_serde::to_vec_named(&lww("v1", 1)).expect("encode");
        2 * u64::try_from(row.len()).expect("row length")
    }

    /// Six flushed rows `d10..d60` with values `v1..v6`.
    async fn write_six(store: &WriteBehindDataStore) {
        for (index, key) in ["d10", "d20", "d30", "d40", "d50", "d60"]
            .iter()
            .enumerate()
        {
            let value = lww(&format!("v{}", index + 1), 1);
            store.add("m", key, &value, 0, 1000).await.unwrap();
        }
        flush_to_idle(store).await;
    }

    fn scan_config() -> WriteBehindConfig {
        WriteBehindConfig {
            write_delay_ms: 0,
            flush_interval_ms: 600_000,
            capacity: 0,
            ..WriteBehindConfig::default()
        }
    }

    /// Answers the n-th scan call with a prepared batch, records the cursors it
    /// is given, and accepts writes. Declares key order.
    struct ScriptedScanStore {
        script: Mutex<std::collections::VecDeque<ScanBatch>>,
        cursors: Mutex<Vec<String>>,
    }

    impl ScriptedScanStore {
        fn new(script: Vec<ScanBatch>) -> Self {
            Self {
                script: Mutex::new(script.into()),
                cursors: Mutex::new(Vec::new()),
            }
        }

        fn next_batch(&self) -> ScanBatch {
            self.script
                .lock()
                .unwrap()
                .pop_front()
                .expect("a scripted batch for every scan call")
        }
    }

    #[async_trait]
    impl MapDataStore for ScriptedScanStore {
        async fn add(
            &self,
            _map: &str,
            _key: &str,
            _value: &RecordValue,
            _expiration_time: i64,
            _now: i64,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn add_backup(
            &self,
            _map: &str,
            _key: &str,
            _value: &RecordValue,
            _expiration_time: i64,
            _now: i64,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn remove(&self, _map: &str, _key: &str, _now: i64) -> anyhow::Result<()> {
            Ok(())
        }

        async fn remove_backup(&self, _map: &str, _key: &str, _now: i64) -> anyhow::Result<()> {
            Ok(())
        }

        async fn load(&self, _map: &str, _key: &str) -> anyhow::Result<Option<RecordValue>> {
            Ok(None)
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

        fn scans_in_key_order(&self) -> bool {
            true
        }

        async fn scan_values(
            &self,
            _map: &str,
            _is_backup: bool,
            _max_batch_cost: u64,
        ) -> anyhow::Result<ScanBatch> {
            Ok(self.next_batch())
        }

        async fn scan_values_batched(
            &self,
            _map: &str,
            _is_backup: bool,
            cursor: ScanCursor,
            _max_batch_cost: u64,
        ) -> anyhow::Result<ScanBatch> {
            self.cursors
                .lock()
                .unwrap()
                .push(String::from_utf8_lossy(&cursor.0).into_owned());
            Ok(self.next_batch())
        }

        async fn remove_all(&self, _map: &str, _keys: &[String]) -> anyhow::Result<()> {
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
    }

    type ScanGate = (
        tokio::sync::oneshot::Sender<()>,
        tokio::sync::oneshot::Receiver<()>,
    );

    /// Forwards everything to redb and declares key order. Can be armed to park
    /// the next `add` on entry, or the next `scan_values` / `scan_values_batched`
    /// after it has read its batch and before it returns.
    struct ParkingScanStore {
        inner: RedbDataStore,
        park_add: Mutex<Option<ScanGate>>,
        park_scan: Mutex<Option<ScanGate>>,
        park_resumed_scan: Mutex<Option<ScanGate>>,
    }

    impl ParkingScanStore {
        fn arm(
            slot: &Mutex<Option<ScanGate>>,
        ) -> (
            tokio::sync::oneshot::Receiver<()>,
            tokio::sync::oneshot::Sender<()>,
        ) {
            let (parked_tx, parked_rx) = tokio::sync::oneshot::channel();
            let (release_tx, release_rx) = tokio::sync::oneshot::channel();
            *slot.lock().unwrap() = Some((parked_tx, release_rx));
            (parked_rx, release_tx)
        }

        async fn park(slot: &Mutex<Option<ScanGate>>) {
            let gate = slot.lock().unwrap().take();
            if let Some((parked, release)) = gate {
                let _ = parked.send(());
                let _ = release.await;
            }
        }
    }

    #[async_trait]
    impl MapDataStore for ParkingScanStore {
        async fn add(
            &self,
            map: &str,
            key: &str,
            value: &RecordValue,
            expiration_time: i64,
            now: i64,
        ) -> anyhow::Result<()> {
            Self::park(&self.park_add).await;
            self.inner.add(map, key, value, expiration_time, now).await
        }

        async fn add_backup(
            &self,
            map: &str,
            key: &str,
            value: &RecordValue,
            expiration_time: i64,
            now: i64,
        ) -> anyhow::Result<()> {
            self.inner
                .add_backup(map, key, value, expiration_time, now)
                .await
        }

        async fn remove(&self, map: &str, key: &str, now: i64) -> anyhow::Result<()> {
            self.inner.remove(map, key, now).await
        }

        async fn remove_backup(&self, map: &str, key: &str, now: i64) -> anyhow::Result<()> {
            self.inner.remove_backup(map, key, now).await
        }

        async fn load(&self, map: &str, key: &str) -> anyhow::Result<Option<RecordValue>> {
            self.inner.load(map, key).await
        }

        async fn load_all(
            &self,
            map: &str,
            keys: &[String],
        ) -> anyhow::Result<Vec<(String, RecordValue)>> {
            self.inner.load_all(map, keys).await
        }

        async fn enumerate_leaves(
            &self,
            map: &str,
            is_backup: bool,
            sink: &mut dyn LeafSink,
        ) -> anyhow::Result<()> {
            self.inner.enumerate_leaves(map, is_backup, sink).await
        }

        fn scans_in_key_order(&self) -> bool {
            true
        }

        async fn scan_values(
            &self,
            map: &str,
            is_backup: bool,
            max_batch_cost: u64,
        ) -> anyhow::Result<ScanBatch> {
            let batch = self
                .inner
                .scan_values(map, is_backup, max_batch_cost)
                .await?;
            Self::park(&self.park_scan).await;
            Ok(batch)
        }

        async fn scan_values_batched(
            &self,
            map: &str,
            is_backup: bool,
            cursor: ScanCursor,
            max_batch_cost: u64,
        ) -> anyhow::Result<ScanBatch> {
            let batch = self
                .inner
                .scan_values_batched(map, is_backup, cursor, max_batch_cost)
                .await?;
            Self::park(&self.park_resumed_scan).await;
            Ok(batch)
        }

        async fn remove_all(&self, map: &str, keys: &[String]) -> anyhow::Result<()> {
            self.inner.remove_all(map, keys).await
        }

        fn is_loadable(&self, key: &str) -> bool {
            self.inner.is_loadable(key)
        }

        fn pending_operation_count(&self) -> u64 {
            self.inner.pending_operation_count()
        }

        async fn soft_flush(&self) -> anyhow::Result<u64> {
            self.inner.soft_flush().await
        }

        async fn hard_flush(&self) -> anyhow::Result<()> {
            self.inner.hard_flush().await
        }
    }

    /// Forwards everything to redb and does NOT forward the key-order
    /// declaration: it answers the trait's default, so a write-behind store over
    /// it keeps the order-free rule whatever it wraps.
    struct UndeclaredScanStore(RedbDataStore);

    #[async_trait]
    impl MapDataStore for UndeclaredScanStore {
        async fn add(
            &self,
            map: &str,
            key: &str,
            value: &RecordValue,
            expiration_time: i64,
            now: i64,
        ) -> anyhow::Result<()> {
            self.0.add(map, key, value, expiration_time, now).await
        }

        async fn add_backup(
            &self,
            map: &str,
            key: &str,
            value: &RecordValue,
            expiration_time: i64,
            now: i64,
        ) -> anyhow::Result<()> {
            self.0
                .add_backup(map, key, value, expiration_time, now)
                .await
        }

        async fn remove(&self, map: &str, key: &str, now: i64) -> anyhow::Result<()> {
            self.0.remove(map, key, now).await
        }

        async fn remove_backup(&self, map: &str, key: &str, now: i64) -> anyhow::Result<()> {
            self.0.remove_backup(map, key, now).await
        }

        async fn load(&self, map: &str, key: &str) -> anyhow::Result<Option<RecordValue>> {
            self.0.load(map, key).await
        }

        async fn load_all(
            &self,
            map: &str,
            keys: &[String],
        ) -> anyhow::Result<Vec<(String, RecordValue)>> {
            self.0.load_all(map, keys).await
        }

        async fn enumerate_leaves(
            &self,
            map: &str,
            is_backup: bool,
            sink: &mut dyn LeafSink,
        ) -> anyhow::Result<()> {
            self.0.enumerate_leaves(map, is_backup, sink).await
        }

        async fn scan_values(
            &self,
            map: &str,
            is_backup: bool,
            max_batch_cost: u64,
        ) -> anyhow::Result<ScanBatch> {
            self.0.scan_values(map, is_backup, max_batch_cost).await
        }

        async fn scan_values_batched(
            &self,
            map: &str,
            is_backup: bool,
            cursor: ScanCursor,
            max_batch_cost: u64,
        ) -> anyhow::Result<ScanBatch> {
            self.0
                .scan_values_batched(map, is_backup, cursor, max_batch_cost)
                .await
        }

        async fn remove_all(&self, map: &str, keys: &[String]) -> anyhow::Result<()> {
            self.0.remove_all(map, keys).await
        }

        fn is_loadable(&self, key: &str) -> bool {
            self.0.is_loadable(key)
        }

        fn pending_operation_count(&self) -> u64 {
            self.0.pending_operation_count()
        }

        async fn soft_flush(&self) -> anyhow::Result<u64> {
            self.0.soft_flush().await
        }

        async fn hard_flush(&self) -> anyhow::Result<()> {
            self.0.hard_flush().await
        }
    }

    fn parking_backed_store() -> (
        Arc<WriteBehindDataStore>,
        Arc<ParkingScanStore>,
        tempfile::TempDir,
    ) {
        let dir = tempfile::tempdir().expect("tempdir");
        let parking = Arc::new(ParkingScanStore {
            inner: RedbDataStore::new(dir.path().join("wb.redb")).expect("redb open"),
            park_add: Mutex::new(None),
            park_scan: Mutex::new(None),
            park_resumed_scan: Mutex::new(None),
        });
        let store = WriteBehindDataStore::new(parking.clone(), scan_config());
        (store, parking, dir)
    }

    fn batches(expected: &[&[&str]]) -> Vec<Vec<String>> {
        expected
            .iter()
            .map(|batch| batch.iter().map(ToString::to_string).collect())
            .collect()
    }

    fn scripted_row(key: &str, value: &str) -> (String, RecordValue) {
        (key.to_string(), lww(value, 1))
    }

    #[tokio::test]
    async fn a_staged_key_whose_durable_row_is_in_a_later_batch_is_emitted_once() {
        let (store, _dir) = redb_backed_store();
        for i in 0..6u32 {
            let key = format!("flushed_{i:02}");
            store
                .add("m", &key, &lww(&format!("v{i}"), 1), 0, 1000)
                .await
                .unwrap();
        }
        flush_to_idle(&store).await;
        store
            .add("m", "flushed_04", &lww("NEW", 9), 0, 2000)
            .await
            .unwrap();

        let observed = scan_transcript(store.as_ref(), 1).await;

        assert_eq!(
            observed,
            vec![
                vec!["flushed_00=v0@1"],
                vec!["flushed_01=v1@1"],
                vec!["flushed_02=v2@1"],
                vec!["flushed_03=v3@1"],
                vec!["flushed_04=NEW@9"],
                vec!["flushed_05=v5@1"],
            ],
            "a staged key is emitted in the batch of its durable row and in no other"
        );
    }

    #[tokio::test]
    async fn a_staging_only_key_is_emitted_in_the_batch_whose_key_range_holds_it() {
        let (store, _dir) = redb_backed_store();
        write_six(&store).await;
        for key in ["a00", "d15", "d25", "d45", "z99"] {
            store.add("m", key, &lww("st", 2), 0, 2000).await.unwrap();
        }
        for key in ["d40", "d50"] {
            store.add("m", key, &lww("NW", 9), 0, 2000).await.unwrap();
        }

        let observed = scan_transcript(store.as_ref(), two_row_budget()).await;

        assert_eq!(
            observed,
            vec![
                vec!["d10=v1@1", "d20=v2@1", "a00=st@2", "d15=st@2"],
                vec!["d30=v3@1", "d40=NW@9", "d25=st@2"],
                vec!["d50=NW@9", "d60=v6@1", "d45=st@2", "z99=st@2"],
            ],
            "each staged key belongs to the one batch whose key range holds it"
        );
    }

    #[tokio::test]
    async fn a_key_flushed_between_two_batches_is_emitted_once() {
        let (store, _dir) = redb_backed_store();
        write_six(&store).await;
        store.add("m", "d45", &lww("st", 2), 0, 2000).await.unwrap();
        let budget = two_row_budget();

        let first = store.scan_values("m", false, budget).await.unwrap();
        flush_to_idle(&store).await;
        let observed = scan_rest(store.as_ref(), first, budget).await;

        assert_eq!(
            observed,
            vec![
                vec!["d10=v1@1", "d20=v2@1"],
                vec!["d30=v3@1", "d40=v4@1"],
                vec!["d45=st@2", "d50=v5@1"],
                vec!["d60=v6@1"],
            ],
            "a key flushed between two batches is emitted by the batch of its row only"
        );
    }

    #[tokio::test]
    async fn a_key_written_behind_the_scan_position_is_not_emitted_and_one_written_ahead_is_emitted_once(
    ) {
        let (store, _dir) = redb_backed_store();
        write_six(&store).await;
        let budget = two_row_budget();

        let first = store.scan_values("m", false, budget).await.unwrap();
        for key in ["d15", "d45"] {
            store.add("m", key, &lww("st", 2), 0, 2000).await.unwrap();
        }
        let first_scan = scan_rest(store.as_ref(), first, budget).await;
        let second_scan = scan_transcript(store.as_ref(), budget).await;

        assert_eq!(
            (first_scan, second_scan),
            (
                batches(&[
                    &["d10=v1@1", "d20=v2@1"],
                    &["d30=v3@1", "d40=v4@1"],
                    &["d50=v5@1", "d60=v6@1", "d45=st@2"],
                ]),
                batches(&[
                    &["d10=v1@1", "d20=v2@1", "d15=st@2"],
                    &["d30=v3@1", "d40=v4@1"],
                    &["d50=v5@1", "d60=v6@1", "d45=st@2"],
                ]),
            ),
            "a key written behind the position waits for the next scan; one ahead is emitted once"
        );
    }

    #[tokio::test]
    async fn a_rowless_batch_with_a_cursor_still_covers_its_key_range() {
        let scripted = Arc::new(ScriptedScanStore::new(vec![
            ScanBatch {
                records: vec![scripted_row("d10", "v1")],
                next_cursor: Some(ScanCursor(b"d10".to_vec())),
            },
            ScanBatch {
                records: Vec::new(),
                next_cursor: Some(ScanCursor(b"d30".to_vec())),
            },
            ScanBatch {
                records: vec![scripted_row("d50", "v5")],
                next_cursor: None,
            },
        ]));
        let store = WriteBehindDataStore::new(scripted.clone(), scan_config());
        for key in ["d05", "d20", "d30", "d40", "d60"] {
            store.add("m", key, &lww("st", 2), 0, 2000).await.unwrap();
        }

        let transcript = scan_transcript(store.as_ref(), 0).await;
        let cursors = scripted.cursors.lock().unwrap().clone();

        assert_eq!(
            (transcript, cursors),
            (
                batches(&[
                    &["d10=v1@1", "d05=st@2"],
                    &["d20=st@2", "d30=st@2"],
                    &["d50=v5@1", "d40=st@2", "d60=st@2"],
                ]),
                vec!["d10".to_string(), "d30".to_string()],
            ),
            "a batch with a cursor and no row still emits the staged keys of its range"
        );
    }

    #[tokio::test]
    async fn a_cursor_that_is_not_a_key_fails_the_scan_of_a_key_ordered_store() {
        let scripted = Arc::new(ScriptedScanStore::new(vec![ScanBatch {
            records: vec![scripted_row("d10", "v1")],
            next_cursor: Some(ScanCursor(vec![0xFF])),
        }]));
        let store = WriteBehindDataStore::new(scripted, scan_config());
        store.add("m", "d05", &lww("st", 2), 0, 2000).await.unwrap();

        let outcome = store.scan_values("m", false, 0).await;
        let message = outcome
            .as_ref()
            .err()
            .map(ToString::to_string)
            .unwrap_or_default();

        assert_eq!(
            (
                outcome.is_err(),
                message.contains("scan cursor is not valid UTF-8")
            ),
            (true, true),
            "a cursor that is not a key cannot bound a key range"
        );
    }

    #[tokio::test]
    async fn a_staged_key_whose_durable_row_is_in_the_first_batch_is_emitted_once() {
        let (store, _dir) = redb_backed_store();
        write_six(&store).await;
        store.add("m", "d10", &lww("NW", 9), 0, 2000).await.unwrap();

        let observed = scan_transcript(store.as_ref(), two_row_budget()).await;

        assert_eq!(
            observed,
            vec![
                vec!["d10=NW@9", "d20=v2@1"],
                vec!["d30=v3@1", "d40=v4@1"],
                vec!["d50=v5@1", "d60=v6@1"],
            ],
            "a staged key replaces its row and is not appended again"
        );
    }

    #[tokio::test]
    async fn staging_only_keys_over_an_empty_durable_map_are_emitted_once() {
        let (store, _dir) = redb_backed_store();
        for key in ["k1", "k2"] {
            store.add("m", key, &lww("st", 2), 0, 2000).await.unwrap();
        }
        store.remove("m", "k3", 2000).await.unwrap();

        let observed = scan_transcript(store.as_ref(), 1).await;

        assert_eq!(
            observed,
            vec![vec!["k1=st@2", "k2=st@2"]],
            "the only batch of an empty durable map holds every staged value once"
        );
    }

    #[tokio::test]
    async fn a_staged_delete_is_emitted_in_no_batch() {
        let (store, _dir) = redb_backed_store();
        write_six(&store).await;
        for key in ["d30", "d40", "q00"] {
            store.remove("m", key, 2000).await.unwrap();
        }

        let observed = scan_transcript(store.as_ref(), two_row_budget()).await;
        let batch_count = observed.len();

        assert_eq!(
            (observed, batch_count),
            (
                batches(&[
                    &["d10=v1@1", "d20=v2@1"][..],
                    &[],
                    &["d50=v5@1", "d60=v6@1"],
                ]),
                3,
            ),
            "a staged delete drops its row and is appended nowhere"
        );
    }

    #[tokio::test]
    async fn a_key_flushed_while_its_batch_is_being_read_is_still_emitted() {
        let (store, parking, _dir) = parking_backed_store();
        write_six(&store).await;
        store.add("m", "d45", &lww("st", 2), 0, 2000).await.unwrap();

        let (parked, release) = ParkingScanStore::arm(&parking.park_scan);
        let scan = {
            let store = Arc::clone(&store);
            tokio::spawn(async move { store.scan_values("m", false, 0).await })
        };
        tokio::time::timeout(std::time::Duration::from_secs(5), parked)
            .await
            .expect("the scan never reached the inner store")
            .expect("park dropped");
        flush_to_idle(&store).await;
        let _ = release.send(());
        let first = scan.await.expect("scan task").expect("scan");
        let observed = scan_rest(store.as_ref(), first, 0).await;

        assert_eq!(
            observed,
            vec![vec![
                "d10=v1@1", "d20=v2@1", "d30=v3@1", "d40=v4@1", "d50=v5@1", "d60=v6@1", "d45=st@2",
            ]],
            "a key flushed after the batch was read is still emitted from the buffer"
        );
    }

    #[tokio::test]
    async fn a_key_being_flushed_is_emitted_once_before_and_after_its_row_is_durable() {
        let (store, parking, _dir) = parking_backed_store();
        write_six(&store).await;
        store.add("m", "d45", &lww("st", 2), 0, 2000).await.unwrap();

        let (parked, release) = ParkingScanStore::arm(&parking.park_add);
        store.soft_flush().await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), parked)
            .await
            .expect("the flush never reached the inner store")
            .expect("park dropped");
        let while_parked = scan_transcript(store.as_ref(), 0).await;
        let _ = release.send(());
        flush_to_idle(&store).await;
        let after = scan_transcript(store.as_ref(), 0).await;

        assert_eq!(
            (while_parked, after),
            (
                batches(&[&[
                    "d10=v1@1", "d20=v2@1", "d30=v3@1", "d40=v4@1", "d50=v5@1", "d60=v6@1",
                    "d45=st@2",
                ]]),
                batches(&[&[
                    "d10=v1@1", "d20=v2@1", "d30=v3@1", "d40=v4@1", "d45=st@2", "d50=v5@1",
                    "d60=v6@1",
                ]]),
            ),
            "the key is in the buffer until its row is durable, and in one of them at a time"
        );
    }

    #[tokio::test]
    async fn a_key_flushed_while_a_resumed_batch_is_being_read_is_still_emitted() {
        let (store, parking, _dir) = parking_backed_store();
        write_six(&store).await;
        store.add("m", "d35", &lww("st", 2), 0, 2000).await.unwrap();
        let budget = two_row_budget();

        let mut first = store.scan_values("m", false, budget).await.unwrap();
        let mut observed = vec![scan_rows(&first)];
        let cursor = first.next_cursor.take().expect("a second batch");
        let (parked, release) = ParkingScanStore::arm(&parking.park_resumed_scan);
        let scan = {
            let store = Arc::clone(&store);
            tokio::spawn(async move { store.scan_values_batched("m", false, cursor, budget).await })
        };
        tokio::time::timeout(std::time::Duration::from_secs(5), parked)
            .await
            .expect("the resumed scan never reached the inner store")
            .expect("park dropped");
        flush_to_idle(&store).await;
        let _ = release.send(());
        let second = scan.await.expect("scan task").expect("scan");
        observed.extend(scan_rest(store.as_ref(), second, budget).await);

        assert_eq!(
            observed,
            vec![
                vec!["d10=v1@1", "d20=v2@1"],
                vec!["d30=v3@1", "d40=v4@1", "d35=st@2"],
                vec!["d50=v5@1", "d60=v6@1"],
            ],
            "a key flushed after a resumed batch was read is still emitted from the buffer"
        );
    }

    #[tokio::test]
    async fn a_resumed_scan_given_a_cursor_that_is_not_a_key_fails() {
        let scripted = Arc::new(ScriptedScanStore::new(vec![ScanBatch {
            records: vec![scripted_row("d50", "v5")],
            next_cursor: None,
        }]));
        let store = WriteBehindDataStore::new(scripted, scan_config());
        store.add("m", "d05", &lww("st", 2), 0, 2000).await.unwrap();

        let outcome = store
            .scan_values_batched("m", false, ScanCursor(vec![0xFF]), 0)
            .await;
        let message = outcome
            .as_ref()
            .err()
            .map(ToString::to_string)
            .unwrap_or_default();

        assert_eq!(
            (
                outcome.is_err(),
                message.contains("scan cursor is not valid UTF-8")
            ),
            (true, true),
            "a cursor handed in that is not a key cannot be a lower bound"
        );
    }

    #[tokio::test]
    async fn a_cursor_returned_by_a_resumed_batch_that_is_not_a_key_fails_the_scan() {
        let scripted = Arc::new(ScriptedScanStore::new(vec![
            ScanBatch {
                records: vec![scripted_row("d10", "v1")],
                next_cursor: Some(ScanCursor(b"d10".to_vec())),
            },
            ScanBatch {
                records: vec![scripted_row("d30", "v3")],
                next_cursor: Some(ScanCursor(vec![0xFF])),
            },
        ]));
        let store = WriteBehindDataStore::new(scripted, scan_config());
        store.add("m", "d05", &lww("st", 2), 0, 2000).await.unwrap();

        let first = store.scan_values("m", false, 0).await;
        let second = store
            .scan_values_batched("m", false, ScanCursor(b"d10".to_vec()), 0)
            .await;
        let message = second
            .as_ref()
            .err()
            .map(ToString::to_string)
            .unwrap_or_default();

        assert_eq!(
            (
                first.is_ok(),
                second.is_err(),
                message.contains("scan cursor is not valid UTF-8")
            ),
            (true, true, true),
            "a cursor a resumed batch returns that is not a key cannot be an upper bound"
        );
    }

    #[tokio::test]
    async fn a_cursor_below_the_one_a_batch_was_resumed_from_fails_the_scan() {
        let scripted = Arc::new(ScriptedScanStore::new(vec![ScanBatch {
            records: Vec::new(),
            next_cursor: Some(ScanCursor(b"d10".to_vec())),
        }]));
        let store = WriteBehindDataStore::new(scripted, scan_config());
        store.add("m", "d20", &lww("st", 2), 0, 2000).await.unwrap();

        // In a task of its own, so that a panic is an observation and not the
        // end of the test.
        let task = {
            let store = Arc::clone(&store);
            tokio::spawn(async move {
                store
                    .scan_values_batched("m", false, ScanCursor(b"d30".to_vec()), 0)
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            })
        };
        let observed = match task.await {
            Ok(Ok(())) => (false, false, false),
            Ok(Err(message)) => (
                false,
                true,
                message.contains("scan cursor is below the cursor it was resumed from"),
            ),
            Err(join) => (join.is_panic(), false, false),
        };

        assert_eq!(
            observed,
            (false, true, true),
            "a cursor that goes backwards is an error of the call, not a panic"
        );
    }

    /// Pins the path this store does NOT fix. Over a store that does not declare
    /// key order the first batch still takes every buffered key without a row in
    /// it, so `d40` and `d50` — buffered, with their rows in later batches — are
    /// emitted twice. That is the known defect `TG-WB-008` does not cover
    /// (TODO-819), not wanted behaviour: when such a store is fixed, this
    /// transcript changes with it.
    #[tokio::test]
    async fn over_a_store_that_does_not_declare_key_order_a_staged_key_with_a_later_row_is_still_emitted_twice(
    ) {
        let dir = tempfile::tempdir().expect("tempdir");
        let undeclared = Arc::new(UndeclaredScanStore(
            RedbDataStore::new(dir.path().join("wb.redb")).expect("redb open"),
        ));
        let store = WriteBehindDataStore::new(undeclared, scan_config());
        write_six(&store).await;
        for key in ["a00", "d15", "d25", "d45", "z99"] {
            store.add("m", key, &lww("st", 2), 0, 2000).await.unwrap();
        }
        for key in ["d40", "d50"] {
            store.add("m", key, &lww("NW", 9), 0, 2000).await.unwrap();
        }

        let observed = scan_transcript(store.as_ref(), two_row_budget()).await;

        assert_eq!(
            observed,
            vec![
                vec![
                    "d10=v1@1", "d20=v2@1", "a00=st@2", "d15=st@2", "d25=st@2", "d40=NW@9",
                    "d45=st@2", "d50=NW@9", "z99=st@2",
                ],
                vec!["d30=v3@1", "d40=NW@9"],
                vec!["d50=NW@9", "d60=v6@1"],
            ],
            "the order-free rule is unchanged, double emission included"
        );
    }

    #[tokio::test]
    async fn a_cursor_that_is_not_a_key_passes_through_while_nothing_is_staged_for_the_map() {
        let scripted = Arc::new(ScriptedScanStore::new(vec![
            ScanBatch {
                records: vec![scripted_row("d10", "v1")],
                next_cursor: Some(ScanCursor(vec![0xFF])),
            },
            ScanBatch {
                records: vec![scripted_row("d50", "v5")],
                next_cursor: None,
            },
        ]));
        let store = WriteBehindDataStore::new(scripted, scan_config());
        // Buffered, but for another map: nothing is staged for `m`.
        store
            .add("other", "d05", &lww("st", 2), 0, 2000)
            .await
            .unwrap();

        let describe = |outcome: anyhow::Result<ScanBatch>| {
            outcome
                .map(|batch| (scan_rows(&batch), batch.next_cursor.map(|cursor| cursor.0)))
                .map_err(|e| e.to_string())
        };
        let first = describe(store.scan_values("m", false, 0).await);
        let second = describe(
            store
                .scan_values_batched("m", false, ScanCursor(vec![0xFF]), 0)
                .await,
        );

        assert_eq!(
            vec![first, second],
            vec![
                Ok((vec!["d10=v1@1".to_string()], Some(vec![0xFF]))),
                Ok((vec!["d50=v5@1".to_string()], None)),
            ],
            "with nothing staged for the map a cursor is passed on and never decoded"
        );
    }

    // -----------------------------------------------------------------------
    // The retry schedule of an entry the store refuses
    // -----------------------------------------------------------------------

    /// The map the schedule tests write to.
    const SCHEDULE_MAP: &str = "m";

    /// Every write is due at once and a pass begins every 100 ms.
    fn schedule_config() -> WriteBehindConfig {
        WriteBehindConfig {
            write_delay_ms: 0,
            flush_interval_ms: 100,
            capacity: 0,
            ..WriteBehindConfig::default()
        }
    }

    fn schedule_store(
        spy: &Arc<SpyDataStore>,
        config: WriteBehindConfig,
    ) -> Arc<WriteBehindDataStore> {
        WriteBehindDataStore::new(Arc::clone(spy) as Arc<dyn MapDataStore>, config)
    }

    fn schedule_store_with_wal(
        spy: &Arc<SpyDataStore>,
        config: WriteBehindConfig,
    ) -> Arc<WriteBehindDataStore> {
        WriteBehindDataStore::new_with_wal(
            Arc::clone(spy) as Arc<dyn MapDataStore>,
            config,
            Some(WalBootstrap {
                wal: InMemoryTestWal::new() as Arc<dyn Wal>,
                sequence_start: 1,
            }),
        )
    }

    /// Lets `ms` milliseconds of the runtime's clock pass.
    async fn advance(ms: u64) {
        tokio::time::sleep(tokio::time::Duration::from_millis(ms)).await;
    }

    /// Polls `cond` every 10 ms of the runtime's clock for at most `limit_ms`
    /// and says after how many milliseconds it first held.
    async fn poll_until(limit_ms: u64, mut cond: impl FnMut() -> bool) -> Option<u64> {
        let start = tokio::time::Instant::now();
        loop {
            let waited = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
            if cond() {
                return Some(waited);
            }
            if waited >= limit_ms {
                return None;
            }
            advance(10).await;
        }
    }

    /// The first `count` keys of the schedule map that fall into `partition`.
    fn keys_of_partition(partition: u32, count: usize) -> Vec<String> {
        (0u32..)
            .map(|i| format!("p{i}"))
            .filter(|key| partition_for(SCHEDULE_MAP, key) == partition)
            .take(count)
            .collect()
    }

    #[tokio::test(start_paused = true)]
    async fn an_entry_refused_for_longer_than_the_old_ladder_is_stored_when_the_store_returns() {
        let spy = Arc::new(SpyDataStore::retaining());
        spy.refuse_everything(true);
        let store = schedule_store_with_wal(&spy, schedule_config());
        store
            .add(SCHEDULE_MAP, "k", &lww("v", 1), 0, 1)
            .await
            .unwrap();
        let partition = partition_for(SCHEDULE_MAP, "k");
        advance(60_000).await;
        spy.refuse_everything(false);
        let healed = poll_until(5_110, || {
            spy.held(SCHEDULE_MAP, "k").is_some() && store.test_is_idle()
        })
        .await
        .is_some();
        assert_eq!(
            (
                healed,
                spy.held(SCHEDULE_MAP, "k"),
                store.test_wal_watermark(partition)
                    == Some(Ok(store.test_max_assigned_wal_sequence(partition))),
                store.test_pending_wal_sequences(partition),
                store.staging.len(),
            ),
            (true, Some(lww("v", 1)), true, Vec::new(), 0)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_value_added_straight_to_the_store_is_loadable_while_refused() {
        let spy = Arc::new(SpyDataStore::retaining());
        spy.seed(SCHEDULE_MAP, "seeded", lww("old", 1));
        spy.refuse_everything(true);
        let store = schedule_store(&spy, schedule_config());
        store
            .add(SCHEDULE_MAP, "k", &lww("v", 2), 0, 1)
            .await
            .unwrap();
        store.remove(SCHEDULE_MAP, "seeded", 1).await.unwrap();
        advance(60_000).await;
        assert_eq!(
            (
                store.load(SCHEDULE_MAP, "k").await.unwrap(),
                store.load(SCHEDULE_MAP, "seeded").await.unwrap(),
            ),
            (Some(lww("v", 2)), None)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_device_identity_is_presented_back_and_stays_revoked_while_the_store_refuses() {
        use crate::network::device_identity::DeviceIdentityStore;

        let spy = Arc::new(SpyDataStore::retaining());
        let store = schedule_store(&spy, schedule_config());
        let identities = DeviceIdentityStore::new(Arc::clone(&store) as Arc<dyn MapDataStore>);

        // X reaches the inner store while it still accepts, so its later
        // revocation is the removal of a record the inner store holds.
        let x = identities.present_or_mint(None, None).await.unwrap();
        let x_token = x
            .minted_token
            .clone()
            .expect("a fresh identity has a token");
        poll_until(10_000, || store.test_is_idle())
            .await
            .expect("the first identity is stored");

        spy.refuse_everything(true);
        let y = identities.present_or_mint(None, None).await.unwrap();
        let y_token = y
            .minted_token
            .clone()
            .expect("a fresh identity has a token");
        advance(60_000).await;
        let again = identities
            .present_or_mint(None, Some(&y_token))
            .await
            .unwrap();
        let presented_back = (again.device_id == y.device_id, again.minted_token.is_none());

        identities.revoke(None, &x.device_id).await.unwrap();
        advance(60_000).await;
        let after_revoke = identities
            .present_or_mint(None, Some(&x_token))
            .await
            .unwrap();
        let revoked = (
            after_revoke.device_id != x.device_id,
            after_revoke.minted_token.is_some(),
        );
        assert_eq!((presented_back, revoked), ((true, true), (true, true)));
    }

    #[tokio::test(start_paused = true)]
    async fn a_pass_over_refused_entries_ends_without_a_timer_wait() {
        let spy = Arc::new(SpyDataStore::retaining());
        let store = schedule_store(&spy, schedule_config());
        for i in 0..50 {
            let key = format!("r{i}");
            spy.refuse_key(&key);
            store
                .add(SCHEDULE_MAP, &key, &lww("v", 1), 0, 1)
                .await
                .unwrap();
        }
        store
            .add(SCHEDULE_MAP, "healthy", &lww("h", 1), 0, 1)
            .await
            .unwrap();
        let closed_before = store.test_passes_closed();
        store.soft_flush().await.unwrap();
        let started = poll_until(10, || spy.call_count() > 0).await;
        let closed = poll_until(10, || {
            !store.test_pass_open() && store.test_passes_closed() > closed_before
        })
        .await;
        let healthy_after =
            poll_until(10_000, || spy.held(SCHEDULE_MAP, "healthy").is_some()).await;
        assert_eq!(
            (
                started.is_some(),
                closed.is_some(),
                healthy_after.map(|ms| ms <= 5_110)
            ),
            (true, true, Some(true)),
            "healthy key stored after {healthy_after:?} ms"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn one_refused_entry_costs_one_store_call_per_backoff_and_delays_no_other_key() {
        let spy = Arc::new(SpyDataStore::retaining());
        spy.refuse_key("A");
        let store = schedule_store(&spy, schedule_config());
        store
            .add(SCHEDULE_MAP, "A", &lww("a", 1), 0, 1)
            .await
            .unwrap();
        let mut slowest = 0u64;
        let mut all_stored = true;
        let mut hold_seen = false;
        for i in 0..60 {
            let round = tokio::time::Instant::now();
            let key = format!("h{i}");
            store
                .add(SCHEDULE_MAP, &key, &lww("h", 1), 0, 1)
                .await
                .unwrap();
            match poll_until(1_000, || spy.held(SCHEDULE_MAP, &key).is_some()).await {
                Some(waited) => slowest = slowest.max(waited),
                None => all_stored = false,
            }
            hold_seen |= store.test_store_hold_active();
            tokio::time::sleep_until(round + tokio::time::Duration::from_millis(1_000)).await;
        }
        let calls = spy.calls_for("A");
        assert_eq!(
            (
                (5..=15).contains(&calls),
                hold_seen,
                all_stored,
                slowest <= 210
            ),
            (true, false, true, true),
            "calls(A) = {calls}, slowest healthy key = {slowest} ms"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_rewritten_refused_key_keeps_its_backoff() {
        let spy = Arc::new(SpyDataStore::retaining());
        spy.refuse_key("A");
        let store = schedule_store(&spy, schedule_config());
        let mut last = lww("a0", 1);
        store.add(SCHEDULE_MAP, "A", &last, 0, 1).await.unwrap();
        for i in 1..=100u64 {
            advance(100).await;
            last = lww(&format!("a{i}"), i + 1);
            store.add(SCHEDULE_MAP, "A", &last, 0, 1).await.unwrap();
        }
        let calls = spy.calls_for("A");
        assert_eq!(
            (calls <= 6, store.load(SCHEDULE_MAP, "A").await.unwrap()),
            (true, Some(last)),
            "calls(A) = {calls}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_superseded_entry_hands_its_backoff_to_the_queued_write() {
        fn answered(spy: &SpyDataStore) -> usize {
            spy.timeline()
                .iter()
                .filter(|call| call.returned.is_some())
                .count()
        }
        let spy = Arc::new(SpyDataStore::retaining());
        spy.refuse_key("A");
        let store = schedule_store(&spy, schedule_config());
        store
            .add(SCHEDULE_MAP, "A", &lww("a1", 1), 0, 1)
            .await
            .unwrap();
        let refused_once = poll_until(1_000, || answered(&spy) == 1).await.is_some();

        // The next attempt on A stops inside the store, so the pass holds the
        // entry out of its queue while A is written again.
        let (entered, proceed) = spy.install_add_gate();
        let held_by_a_pass =
            tokio::time::timeout(tokio::time::Duration::from_secs(30), entered.notified())
                .await
                .is_ok();
        store
            .add(SCHEDULE_MAP, "A", &lww("a2", 2), 0, 1)
            .await
            .unwrap();
        let queued_before = store.test_retry_state(SCHEDULE_MAP, "A");
        proceed.notify_one();
        let refused_twice = poll_until(1_000, || answered(&spy) == 2).await.is_some();
        let queued_after = store.test_retry_state(SCHEDULE_MAP, "A");
        assert_eq!(
            (
                (refused_once, held_by_a_pass, refused_twice),
                queued_before,
                queued_after.map(|(count, next)| (count, next.is_some())),
                store.pending_operation_count(),
            ),
            ((true, true, true), Some((0, None)), Some((2, true)), 1)
        );
    }

    /// How many store calls a store that refuses everything gets in 60 s
    /// with `keys` entries buffered.
    async fn calls_under_a_full_refusal(keys: usize) -> usize {
        let spy = Arc::new(SpyDataStore::retaining());
        spy.refuse_everything(true);
        let store = schedule_store(&spy, schedule_config());
        for i in 0..keys {
            store
                .add(SCHEDULE_MAP, &format!("k{i}"), &lww("v", 1), 0, 1)
                .await
                .unwrap();
        }
        advance(60_000).await;
        spy.call_count()
    }

    #[tokio::test(start_paused = true)]
    async fn a_store_that_refuses_everything_gets_a_bounded_number_of_calls() {
        let ten = calls_under_a_full_refusal(10).await;
        let thousand = calls_under_a_full_refusal(1_000).await;
        assert_eq!(
            ((8..=20).contains(&ten), (8..=20).contains(&thousand)),
            (true, true),
            "calls for 10 keys = {ten}, for 1000 keys = {thousand}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_hold_makes_no_store_call_even_when_a_flush_is_requested() {
        let spy = Arc::new(SpyDataStore::retaining());
        spy.refuse_everything(true);
        let store = schedule_store(&spy, schedule_config());
        for i in 0..10 {
            store
                .add(SCHEDULE_MAP, &format!("k{i}"), &lww("v", 1), 0, 1)
                .await
                .unwrap();
        }
        let tripped = poll_until(1_000, || store.test_store_hold_active())
            .await
            .is_some();
        let before = spy.call_count();
        // Yields only: the clock does not move, so every request lands inside
        // the hold that has just begun.
        for _ in 0..100 {
            store.soft_flush().await.unwrap();
            tokio::task::yield_now().await;
            tokio::task::yield_now().await;
        }
        assert_eq!((tripped, before, spy.call_count()), (true, 3, 3));
    }

    #[tokio::test(start_paused = true)]
    async fn after_a_refusal_longer_than_the_old_ladder_the_buffer_drains_within_one_backoff_cap_and_one_interval(
    ) {
        let spy = Arc::new(SpyDataStore::retaining());
        spy.refuse_everything(true);
        let store = schedule_store(
            &spy,
            WriteBehindConfig {
                write_delay_ms: 0,
                capacity: 0,
                ..WriteBehindConfig::default()
            },
        );
        for i in 0..100 {
            store
                .add(SCHEDULE_MAP, &format!("k{i}"), &lww("v", 1), 0, 1)
                .await
                .unwrap();
        }
        advance(600_000).await;
        let buffered_at_t = store.pending_operation_count();
        spy.refuse_everything(false);
        let drained = poll_until(6_010, || store.test_is_idle()).await;
        let stored = (0..100)
            .filter(|i| spy.held(SCHEDULE_MAP, &format!("k{i}")).is_some())
            .count();
        assert_eq!(
            (buffered_at_t, drained.is_some(), stored),
            (100, true, 100),
            "drained after {drained:?} ms"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn keys_of_one_partition_are_stored_out_of_write_order_and_the_watermarks_follow() {
        let spy = Arc::new(SpyDataStore::retaining());
        let store = schedule_store_with_wal(&spy, schedule_config());
        let partition = 7;
        let keys = keys_of_partition(partition, 2);
        spy.refuse_key(&keys[0]);
        store
            .add(SCHEDULE_MAP, &keys[0], &lww("first", 1), 0, 1)
            .await
            .unwrap();
        let first_wal_seq = store.test_pending_wal_sequences(partition)[0].0;
        // The first entry is the only pending one, so the flushed watermark is
        // its entry sequence.
        let first_entry_seq = store.flushed_watermark();
        store
            .add(SCHEDULE_MAP, &keys[1], &lww("second", 1), 0, 1)
            .await
            .unwrap();
        advance(60_000).await;
        let while_refused = (
            spy.held(SCHEDULE_MAP, &keys[1]).is_some(),
            store.test_wal_watermark(partition) == Some(Ok(first_wal_seq - 1)),
            store.flushed_watermark() == first_entry_seq,
        );
        spy.accept_every_key();
        let drained = poll_until(10_000, || store.test_is_idle()).await.is_some();
        let after = (
            store.test_wal_watermark(partition)
                == Some(Ok(store.test_max_assigned_wal_sequence(partition))),
            store.flushed_watermark() == store.assigned_write_sequence(),
        );
        assert_eq!(
            (while_refused, drained, after),
            ((true, true, true), true, (true, true))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_refused_cell_entry_persists_what_the_cell_holds_when_it_is_accepted() {
        let spy = Arc::new(SpyDataStore::retaining());
        spy.refuse_everything(true);
        let store = schedule_store_with_wal(&spy, schedule_config());
        let partition = partition_for(SCHEDULE_MAP, "set");
        let cell = crate::storage::engine::new_slot_cell(crate::storage::record::Record {
            value: or_value(&["t-1", "t-2"]),
            metadata: crate::storage::record::RecordMetadata::new(0, 0),
        });
        let add_t2 = OrDelta::Add {
            entry: or_entry("t-2"),
        };
        store
            .add_with_witness(
                SCHEDULE_MAP,
                "set",
                WriteSource::Cell(&cell),
                0,
                1,
                Some(&add_t2),
            )
            .await
            .unwrap();
        advance(60_000).await;
        cell.lock().value = or_value(&["t-1", "t-2", "t-3"]);
        let add_t3 = OrDelta::Add {
            entry: or_entry("t-3"),
        };
        store
            .add_with_witness(
                SCHEDULE_MAP,
                "set",
                WriteSource::Cell(&cell),
                0,
                1,
                Some(&add_t3),
            )
            .await
            .unwrap();
        let pending_while_refused: Vec<PendingOrigin> = store
            .test_pending_wal_sequences(partition)
            .into_iter()
            .map(|(_, origin)| origin)
            .collect();
        spy.refuse_everything(false);
        let drained = poll_until(10_000, || store.test_is_idle()).await.is_some();
        assert_eq!(
            (
                pending_while_refused,
                drained,
                spy.held(SCHEDULE_MAP, "set"),
                spy.accepted_for("set"),
                store.test_pending_wal_sequences(partition),
            ),
            (
                vec![PendingOrigin::Live, PendingOrigin::Live],
                true,
                Some(or_value(&["t-1", "t-2", "t-3"])),
                1,
                Vec::new()
            )
        );
    }

    /// `refused` keys are written first and are never accepted, then one key
    /// the store accepts every two seconds for two minutes. Says: how many of
    /// those were stored, the longest wait of one of them in milliseconds, the
    /// buffered-key count at the end, and whether a hold was ever in force.
    async fn healthy_keys_after_refused_ones(refused: usize) -> (usize, u64, u64, bool) {
        let spy = Arc::new(SpyDataStore::retaining());
        let store = schedule_store(&spy, schedule_config());
        for i in 0..refused {
            let key = format!("r{i}");
            spy.refuse_key(&key);
            store
                .add(SCHEDULE_MAP, &key, &lww("v", 1), 0, 1)
                .await
                .unwrap();
        }
        let mut stored = 0usize;
        let mut slowest = 0u64;
        let mut hold_seen = false;
        for i in 0..60 {
            let round = tokio::time::Instant::now();
            let key = format!("h{i}");
            store
                .add(SCHEDULE_MAP, &key, &lww("h", 1), 0, 1)
                .await
                .unwrap();
            let waited = poll_until(60_000, || {
                hold_seen |= store.test_store_hold_active();
                spy.held(SCHEDULE_MAP, &key).is_some()
            })
            .await;
            if let Some(waited) = waited {
                stored += 1;
                slowest = slowest.max(waited);
            }
            let next = round + tokio::time::Duration::from_millis(2_000);
            if tokio::time::Instant::now() < next {
                tokio::time::sleep_until(next).await;
            }
        }
        (stored, slowest, store.pending_operation_count(), hold_seen)
    }

    #[tokio::test(start_paused = true)]
    async fn healthy_keys_written_after_refused_ones_are_never_starved() {
        let (stored, slowest, pending, hold_seen) = healthy_keys_after_refused_ones(10).await;
        assert_eq!(
            (stored, slowest <= 5_110, pending, hold_seen),
            (60, true, 10, true),
            "slowest healthy key = {slowest} ms"
        );
    }

    /// A key the store accepts is refused during an outage of the whole
    /// store; the store then returns for everything but `rejected` records
    /// written at that instant, which it never accepts. Says: refusals of the
    /// key during the outage, after how many milliseconds it was stored, and —
    /// between the first call on the last rejected record to be tried and the
    /// key's accepted call — the refused store calls and the milliseconds.
    async fn a_refused_healthy_key_among_rejected_records(
        rejected: usize,
        limit_ms: u64,
    ) -> (usize, Option<u64>, Option<usize>, Option<u64>) {
        let spy = Arc::new(SpyDataStore::retaining());
        let store = schedule_store(&spy, schedule_config());
        spy.refuse_everything(true);
        store
            .add(SCHEDULE_MAP, "H", &lww("h", 1), 0, 1)
            .await
            .unwrap();
        poll_until(60_000, || spy.calls_for("H") == 5).await;
        let refusals_in_outage = spy.calls_for("H");
        spy.refuse_everything(false);
        for i in 0..rejected {
            let key = format!("rejected{i}");
            spy.refuse_key(&key);
            store
                .add(SCHEDULE_MAP, &key, &lww("v", 1), 0, 2)
                .await
                .unwrap();
        }
        let waited = poll_until(limit_ms, || spy.held(SCHEDULE_MAP, "H").is_some()).await;
        let calls = spy.timeline();
        let last_first_try = (0..rejected)
            .map(|i| {
                let key = format!("rejected{i}");
                calls.iter().position(|call| call.key == key)
            })
            .collect::<Option<Vec<usize>>>()
            .and_then(|firsts| firsts.into_iter().max());
        let accepted_at = calls
            .iter()
            .position(|call| call.key == "H" && call.accepted);
        let stretch = match (last_first_try, accepted_at) {
            (Some(from), Some(to)) if from < to => Some((
                calls[from + 1..to]
                    .iter()
                    .filter(|call| !call.accepted)
                    .count(),
                u64::try_from(
                    calls[to]
                        .began
                        .duration_since(calls[from].began)
                        .as_millis(),
                )
                .unwrap_or(u64::MAX),
            )),
            _ => None,
        };
        (
            refusals_in_outage,
            waited,
            stretch.map(|(refused, _)| refused),
            stretch.map(|(_, ms)| ms),
        )
    }

    #[tokio::test(start_paused = true)]
    async fn a_refused_healthy_key_is_reached_after_at_most_one_probe_per_rejected_key() {
        let (refusals, waited, refused_between, stretch_ms) =
            a_refused_healthy_key_among_rejected_records(30, 600_000).await;
        assert_eq!(
            (
                refusals,
                waited.is_some(),
                refused_between.is_some_and(|refused| refused <= 30),
                stretch_ms.is_some_and(|ms| ms <= 158_110),
            ),
            (5, true, true, true),
            "waited {waited:?} ms, {refused_between:?} refused calls in {stretch_ms:?} ms"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_store_that_refuses_slowly_is_called_once_at_a_time_and_left_alone_for_a_hold() {
        let spy = Arc::new(SpyDataStore::retaining());
        spy.refuse_everything(true);
        // Longer than the backoff cap: a hold counted from the start of a call
        // would be over before the call answered.
        spy.set_answer_delay_ms(8_000);
        let store = schedule_store(&spy, schedule_config());
        for i in 0..100 {
            store
                .add(SCHEDULE_MAP, &format!("k{i}"), &lww("v", 1), 0, 1)
                .await
                .unwrap();
        }
        let mut hold_seen = false;
        for _ in 0..1_200 {
            advance(100).await;
            hold_seen |= store.test_store_hold_active();
        }
        let calls = spy.timeline();
        let begun = calls.len();
        let begins_ms: Vec<u128> = calls
            .iter()
            .map(|call| call.began.duration_since(calls[0].began).as_millis())
            .collect();
        let left_alone = calls.windows(2).skip(2).all(|pair| {
            pair[0].returned.is_some_and(|returned| {
                pair[1].began >= returned + tokio::time::Duration::from_millis(500)
            })
        });
        assert_eq!(
            (
                spy.most_in_flight(),
                begun <= 12,
                begun >= 9,
                left_alone,
                hold_seen
            ),
            (1, true, true, true, true),
            "calls begun = {begun}, at ms {begins_ms:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn never_refused_entries_are_stored_before_refused_ones_in_a_pass() {
        let spy = Arc::new(SpyDataStore::retaining());
        let store = schedule_store(&spy, schedule_config());
        for i in 0..5 {
            let key = format!("r{i}");
            spy.refuse_key(&key);
            store
                .add(SCHEDULE_MAP, &key, &lww("v", 1), 0, 1)
                .await
                .unwrap();
        }
        // Refused once and due again, written onto the queued entries: passes
        // cannot bring five keys there without a hold beginning first.
        let now = tokio::time::Instant::now();
        for mut queue in store.queues.iter_mut() {
            for entry in queue.value_mut().entries.values_mut() {
                entry.retry_count = 1;
                entry.next_attempt = Some(now);
            }
        }
        for i in 0..5 {
            store
                .add(SCHEDULE_MAP, &format!("h{i}"), &lww("h", 1), 0, 2)
                .await
                .unwrap();
        }
        store.soft_flush().await.unwrap();
        advance(10).await;
        let healthy_stored = (0..5)
            .filter(|i| spy.held(SCHEDULE_MAP, &format!("h{i}")).is_some())
            .count();
        assert_eq!((healthy_stored, store.test_store_hold_active()), (5, true));
    }

    #[tokio::test(start_paused = true)]
    async fn refused_entries_are_attempted_longest_waiting_first() {
        let spy = Arc::new(SpyDataStore::retaining());
        let store = schedule_store(&spy, schedule_config());
        // So that an instant two seconds in the past exists on this clock.
        advance(3_000).await;
        store
            .add(SCHEDULE_MAP, "X", &lww("x", 1), 0, 1)
            .await
            .unwrap();
        store
            .add(SCHEDULE_MAP, "Y", &lww("y", 1), 0, 1)
            .await
            .unwrap();
        store
            .add(SCHEDULE_MAP, "Z", &lww("z", 1), 0, 2)
            .await
            .unwrap();
        let now = tokio::time::Instant::now();
        for mut queue in store.queues.iter_mut() {
            for entry in queue.value_mut().entries.values_mut() {
                match entry.key.as_str() {
                    "X" => {
                        entry.retry_count = 5;
                        entry.next_attempt = Some(now - tokio::time::Duration::from_secs(2));
                    }
                    "Y" => {
                        entry.retry_count = 1;
                        entry.next_attempt = Some(now - tokio::time::Duration::from_secs(1));
                    }
                    _ => {}
                }
            }
        }
        store.soft_flush().await.unwrap();
        advance(10).await;
        let order: Vec<String> = spy.timeline().into_iter().map(|call| call.key).collect();
        assert_eq!(order, ["Z", "X", "Y"]);
    }

    // -----------------------------------------------------------------------
    // The store-wide hold, its probe, and what an operator is told
    // -----------------------------------------------------------------------

    /// How many inner-store calls have answered so far.
    fn calls_answered(spy: &SpyDataStore) -> usize {
        spy.timeline()
            .iter()
            .filter(|call| call.returned.is_some())
            .count()
    }

    /// The entry sequence of the entry queued for a key of the schedule map.
    fn queued_sequence(store: &WriteBehindDataStore, key: &str) -> Option<u64> {
        let queue = store.queues.get(&partition_for(SCHEDULE_MAP, key))?;
        let entry = queue
            .entries
            .get(&(SCHEDULE_MAP.to_string(), key.to_string()))?;
        Some(entry.sequence)
    }

    #[tokio::test(start_paused = true)]
    async fn entries_put_back_by_a_hold_are_accounted_like_a_retry() {
        const KEYS: [&str; 5] = ["A", "B", "C", "D", "E"];

        let spy = Arc::new(SpyDataStore::retaining());
        spy.refuse_everything(true);
        let store = schedule_store_with_wal(&spy, schedule_config());
        // The first store call of the pass, on A, stops inside the store: the
        // pass then holds all five entries out of their queues.
        let (entered, proceed) = spy.install_add_gate();
        for (store_time, key) in (1i64..).zip(KEYS) {
            store
                .add(SCHEDULE_MAP, key, &lww("v", 1), 0, store_time)
                .await
                .unwrap();
        }
        let held_by_a_pass =
            tokio::time::timeout(tokio::time::Duration::from_secs(30), entered.notified())
                .await
                .is_ok();
        store
            .add(SCHEDULE_MAP, "E", &lww("v2", 2), 0, 9)
            .await
            .unwrap();
        let count_while_held = store.pending_operation_count();
        proceed.notify_one();
        let closed = poll_until(10_000, || !store.test_pass_open() && spy.call_count() >= 3)
            .await
            .is_some();

        let e_wal_sequences = store
            .queues
            .get(&partition_for(SCHEDULE_MAP, "E"))
            .and_then(|queue| {
                queue
                    .entries
                    .get(&(SCHEDULE_MAP.to_string(), "E".to_string()))
                    .map(|entry| entry.wal_sequences.len())
            });
        let in_flight: usize = KEYS
            .iter()
            .map(|key| {
                store
                    .test_in_flight_wal_sequences(partition_for(SCHEDULE_MAP, key))
                    .len()
            })
            .sum();
        let pending_sequences = store.pending_seqs().len();
        let loaded = store.load(SCHEDULE_MAP, "E").await.unwrap();
        assert_eq!(
            (
                (held_by_a_pass, closed),
                (spy.call_count(), spy.calls_for("D") + spy.calls_for("E")),
                (
                    store.test_retry_state(SCHEDULE_MAP, "D"),
                    store.test_retry_state(SCHEDULE_MAP, "E"),
                ),
                (
                    count_while_held,
                    store.pending_operation_count(),
                    pending_sequences
                ),
                (e_wal_sequences, in_flight),
                loaded,
            ),
            (
                (true, true),
                (3, 0),
                (Some((0, None)), Some((0, None))),
                (6, 5, 5),
                (Some(2), 0),
                Some(lww("v2", 2)),
            ),
            "((held by a pass, pass closed), (store calls, calls on D and E), (retry state \
             of D, of E), (count while held, count, pending entry sequences), (WAL \
             sequences E's queued entry owns, WAL sequences in flight), load of E)"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn an_accepted_probe_ends_the_hold_and_the_next_trip_starts_at_the_first_length() {
        // From the return of the call that tripped the pass to the first probe.
        fn first_probe_gap(calls: &[SpyTimedCall]) -> Option<u128> {
            let tripped = calls.get(2).and_then(|call| call.returned)?;
            let probe = calls.get(3)?;
            Some(probe.began.duration_since(tripped).as_millis())
        }
        fn is_the_first_length(gap: Option<u128>) -> bool {
            gap.is_some_and(|ms| (500..=610).contains(&ms))
        }

        let spy = Arc::new(SpyDataStore::retaining());
        spy.refuse_everything(true);
        let store = schedule_store(&spy, schedule_config());
        for i in 0..4 {
            store
                .add(SCHEDULE_MAP, &format!("k{i}"), &lww("v", 1), 0, 1)
                .await
                .unwrap();
        }
        let mut hold_seen = false;
        let mut most_failed_probes = 0;
        for _ in 0..200 {
            advance(100).await;
            hold_seen |= store.test_store_hold_active();
            most_failed_probes = most_failed_probes.max(store.test_failed_probes());
        }
        let first_trip = spy.timeline();
        spy.refuse_everything(false);
        let _ = poll_until(30_000, || store.test_is_idle()).await;
        let after_the_accepted_probe = (store.test_store_hold_active(), store.test_failed_probes());

        let before_second = spy.call_count();
        spy.refuse_everything(true);
        for i in 0..4 {
            store
                .add(SCHEDULE_MAP, &format!("n{i}"), &lww("v", 1), 0, 1)
                .await
                .unwrap();
        }
        advance(5_000).await;
        let second_trip = spy.timeline().split_off(before_second);
        let first_gap = first_probe_gap(&first_trip);
        let second_gap = first_probe_gap(&second_trip);
        assert_eq!(
            (
                hold_seen,
                most_failed_probes >= 2,
                after_the_accepted_probe.0,
                after_the_accepted_probe.1,
                is_the_first_length(first_gap),
                is_the_first_length(second_gap),
            ),
            (true, true, false, 0, true, true),
            "most failed probes = {most_failed_probes}, gaps: {first_gap:?} ms, \
             {second_gap:?} ms"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn an_accepted_entry_between_refusals_resets_the_count_in_a_row() {
        let spy = Arc::new(SpyDataStore::retaining());
        for key in ["A", "B", "D", "E"] {
            spy.refuse_key(key);
        }
        let store = schedule_store(&spy, schedule_config());
        for key in ["A", "B", "C", "D", "E"] {
            store
                .add(SCHEDULE_MAP, key, &lww("v", 1), 0, 1)
                .await
                .unwrap();
        }
        store.soft_flush().await.unwrap();
        let _ = poll_until(1_000, || spy.call_count() > 0).await;
        advance(10).await;
        assert_eq!(
            (
                spy.call_count(),
                store.test_store_hold_active(),
                spy.held(SCHEDULE_MAP, "C").is_some()
            ),
            (5, false, true)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_probe_does_not_take_an_entry_whose_time_has_not_come() {
        let spy = Arc::new(SpyDataStore::retaining());
        spy.refuse_everything(true);
        let store = schedule_store(&spy, schedule_config());
        for i in 0..3 {
            store
                .add(SCHEDULE_MAP, &format!("k{i}"), &lww("v", 1), 0, 1)
                .await
                .unwrap();
        }
        poll_until(30_000, || calls_answered(&spy) >= 3)
            .await
            .expect("three refusals");
        let third_returned = spy.timeline()[2].returned.expect("the third call returned");
        // The hold is over by now and every entry is still inside its own
        // backoff: nothing is due for a probe.
        tokio::time::sleep_until(third_returned + tokio::time::Duration::from_millis(900)).await;
        let later = (
            spy.call_count(),
            store.test_store_hold_active(),
            store.test_failed_probes(),
        );
        let _ = poll_until(30_000, || spy.call_count() >= 4).await;
        let fourth_gap = spy
            .timeline()
            .get(3)
            .map(|call| call.began.duration_since(third_returned).as_millis());
        assert_eq!(
            (
                later.0,
                later.1,
                later.2,
                fourth_gap.is_some_and(|ms| ms >= 1_000)
            ),
            (3, true, 0, true),
            "the fourth call began {fourth_gap:?} ms after the third returned"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_probe_whose_key_was_rewritten_before_it_was_taken_stores_the_new_value_in_that_pass()
    {
        let spy = Arc::new(SpyDataStore::retaining());
        for key in ["a", "b", "c"] {
            spy.refuse_key(key);
        }
        let store = schedule_store(&spy, schedule_config());
        for key in ["a", "b", "c"] {
            store
                .add(SCHEDULE_MAP, key, &lww("v", 1), 0, 1)
                .await
                .unwrap();
        }
        let _ = poll_until(1_000, || store.test_store_hold_active()).await;
        let mut park = store.test_park_between_probe_choice_and_removal();
        // Never refused and the newest: the next probe.
        store
            .add(SCHEDULE_MAP, "D", &lww("d1", 1), 0, 1)
            .await
            .unwrap();
        let parked = park.parked.take().expect("armed");
        let park_reached = tokio::time::timeout(tokio::time::Duration::from_secs(2), parked)
            .await
            .is_ok_and(|parked| parked.is_ok());
        // The chosen key gets a new entry, with a new entry sequence, while
        // the flush task stands between its choice and the removal.
        store
            .add(SCHEDULE_MAP, "D", &lww("d2", 2), 0, 1)
            .await
            .unwrap();
        let closed_before = store.test_passes_closed();
        park.release();
        let _ = poll_until(1_000, || {
            !store.test_pass_open() && store.test_passes_closed() > closed_before
        })
        .await;
        assert_eq!(
            (
                park_reached,
                spy.call_count(),
                spy.held(SCHEDULE_MAP, "D") == Some(lww("d2", 2)),
                store.test_retry_state(SCHEDULE_MAP, "D").is_some(),
                store.test_store_hold_active(),
                store.test_failed_probes(),
            ),
            (true, 4, true, false, false, 0)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn an_accepted_probe_is_followed_by_a_normal_pass_over_what_is_due() {
        let spy = Arc::new(SpyDataStore::retaining());
        for key in ["a", "b", "c"] {
            spy.refuse_key(key);
        }
        let store = schedule_store(&spy, schedule_config());
        for key in ["a", "b", "c"] {
            store
                .add(SCHEDULE_MAP, key, &lww("v", 1), 0, 1)
                .await
                .unwrap();
        }
        let hold_at_the_writes = poll_until(1_000, || store.test_store_hold_active())
            .await
            .is_some();
        // Both never refused and due, with one `store_time`: E, the newer, is
        // the probe and D is what the rest of that pass has to store.
        for key in ["D", "E"] {
            store
                .add(SCHEDULE_MAP, key, &lww("v", 1), 0, 1)
                .await
                .unwrap();
        }
        let _ = poll_until(1_000, || calls_answered(&spy) >= 4).await;
        let fourth = spy.timeline().get(3).cloned();
        if let Some(returned) = fourth.as_ref().and_then(|call| call.returned) {
            // One poll step after the probe answered and no more: a D left to
            // the next pass arrives a whole interval later.
            tokio::time::sleep_until(returned + tokio::time::Duration::from_millis(10)).await;
        }
        assert_eq!(
            (
                hold_at_the_writes,
                fourth.is_some_and(|call| call.key == "E"),
                spy.held(SCHEDULE_MAP, "D").is_some(),
                store.test_store_hold_active(),
                store.test_failed_probes(),
            ),
            (true, true, true, false, 0)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn the_probe_is_the_never_refused_entry_with_the_latest_store_time_whatever_its_sequence()
    {
        let spy = Arc::new(SpyDataStore::retaining());
        for key in ["r0", "r1", "r2"] {
            spy.refuse_key(key);
        }
        let store = schedule_store(&spy, schedule_config());
        for key in ["r0", "r1", "r2"] {
            store
                .add(SCHEDULE_MAP, key, &lww("v", 1), 0, 1)
                .await
                .unwrap();
        }
        let hold_at_the_writes = poll_until(1_000, || store.test_store_hold_active())
            .await
            .is_some();
        store
            .add(SCHEDULE_MAP, "A", &lww("a1", 1), 0, 1)
            .await
            .unwrap();
        store
            .add(SCHEDULE_MAP, "B", &lww("b", 1), 0, 2)
            .await
            .unwrap();
        // A rewrite of a queued entry takes a new entry sequence and keeps the
        // `store_time` of the entry it replaces: A is now the newer by
        // sequence and still the older by `store_time`.
        store
            .add(SCHEDULE_MAP, "A", &lww("a2", 2), 0, 3)
            .await
            .unwrap();
        let a_is_newer_by_sequence = queued_sequence(&store, "A")
            .zip(queued_sequence(&store, "B"))
            .is_some_and(|(a, b)| a > b);
        let _ = poll_until(1_000, || spy.call_count() >= 4).await;
        let fourth_key = spy.timeline().get(3).map(|call| call.key.clone());
        assert_eq!(
            (hold_at_the_writes, a_is_newer_by_sequence, fourth_key),
            (true, true, Some("B".to_string()))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_long_refusal_fills_the_buffer_and_new_keys_are_refused_while_buffered_keys_are_accepted(
    ) {
        let spy = Arc::new(SpyDataStore::retaining());
        spy.refuse_everything(true);
        let store = schedule_store(
            &spy,
            WriteBehindConfig {
                capacity: 5,
                ..schedule_config()
            },
        );
        for i in 0..5 {
            store
                .add(SCHEDULE_MAP, &format!("k{i}"), &lww("v", 1), 0, 1)
                .await
                .unwrap();
        }
        advance(60_000).await;
        let count_refused = store.pending_operation_count();
        let sixth = store
            .add(SCHEDULE_MAP, "k5", &lww("v", 1), 0, 1)
            .await
            .err()
            .map(|err| err.to_string().contains("Write-behind capacity exceeded"));
        let rewrite = store
            .add(SCHEDULE_MAP, "k0", &lww("rewritten", 2), 0, 1)
            .await
            .is_ok();
        let after_rewrite = (
            store.pending_operation_count(),
            store.load(SCHEDULE_MAP, "k0").await.unwrap(),
        );
        spy.refuse_everything(false);
        let drained = poll_until(10_000, || store.test_is_idle()).await.is_some();
        let stored = (0..5)
            .filter(|i| spy.held(SCHEDULE_MAP, &format!("k{i}")).is_some())
            .count();
        let sixth_after = store
            .add(SCHEDULE_MAP, "k5", &lww("v", 1), 0, 1)
            .await
            .is_ok();
        assert_eq!(
            (
                count_refused,
                sixth,
                rewrite,
                after_rewrite,
                drained,
                (stored, spy.held(SCHEDULE_MAP, "k0")),
                sixth_after
            ),
            (
                5,
                Some(true),
                true,
                (5, Some(lww("rewritten", 2))),
                true,
                (5, Some(lww("rewritten", 2))),
                true
            ),
            "(count while refused, the sixth key failed on capacity, a buffered key was \
             rewritten, (count, load) after the rewrite, drained, (buffered keys stored, \
             the store's value of the rewritten key), the sixth key accepted afterwards)"
        );
    }

    // Real time and a real WAL: the shutdown drain is bounded by a wall-clock
    // deadline, and what a restart finds is what was written to disk.
    #[tokio::test]
    async fn a_shutdown_while_the_store_refuses_ends_in_time_and_keeps_slots_and_frames() {
        use crate::storage::wal::{WalRecovery, WalWriter};

        let dir = tempfile::tempdir().unwrap();
        let wal = WalWriter::new(dir.path().to_path_buf(), WalFsyncPolicy::PerOp).unwrap();
        let spy = Arc::new(SpyDataStore::retaining());
        spy.refuse_everything(true);
        let store = WriteBehindDataStore::new_with_wal(
            Arc::clone(&spy) as Arc<dyn MapDataStore>,
            WriteBehindConfig {
                write_delay_ms: 0,
                flush_interval_ms: 5,
                backoff_base_ms: 1,
                backoff_cap_ms: 2,
                shutdown_timeout_ms: 500,
                capacity: 0,
                ..WriteBehindConfig::default()
            },
            Some(WalBootstrap {
                wal: Arc::clone(&wal) as Arc<dyn Wal>,
                sequence_start: 1,
            }),
        );
        let partition = 11;
        let keys = keys_of_partition(partition, 3);
        for key in &keys {
            store
                .add(SCHEDULE_MAP, key, &lww(key, 1), 0, 1)
                .await
                .unwrap();
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        let started = std::time::Instant::now();
        let outcome = store.hard_flush().await.is_ok();
        let took = started.elapsed();
        let mut loaded = Vec::new();
        for key in &keys {
            loaded.push(store.load(SCHEDULE_MAP, key).await.unwrap() == Some(lww(key, 1)));
        }
        let tags: Vec<PendingOrigin> = store
            .test_pending_wal_sequences(partition)
            .into_iter()
            .map(|(_, origin)| origin)
            .collect();
        let frames = wal.unapplied(partition).await.unwrap().len();
        drop(store);
        let recovered = Arc::new(SpyDataStore::retaining());
        WalRecovery::new(Arc::clone(&wal), Vec::new())
            .run(Arc::clone(&recovered) as Arc<dyn MapDataStore>)
            .await
            .unwrap();
        let replayed: Vec<bool> = keys
            .iter()
            .map(|key| recovered.held(SCHEDULE_MAP, key) == Some(lww(key, 1)))
            .collect();
        assert_eq!(
            (
                outcome,
                took < std::time::Duration::from_millis(750),
                loaded,
                tags,
                frames,
                replayed
            ),
            (
                true,
                true,
                vec![true; 3],
                vec![PendingOrigin::Abandoned; 3],
                3,
                vec![true; 3]
            ),
            "hard_flush took {took:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn an_entry_refused_a_hundred_times_backs_off_at_the_cap_and_the_flush_task_lives() {
        let spy = Arc::new(SpyDataStore::retaining());
        spy.refuse_everything(true);
        let store = schedule_store(&spy, schedule_config());
        store
            .add(SCHEDULE_MAP, "k", &lww("v", 1), 0, 1)
            .await
            .unwrap();
        // A hundred refusals take hours of backoff to reach through passes, so
        // the count is written onto the queued entry.
        for mut queue in store.queues.iter_mut() {
            for entry in queue.value_mut().entries.values_mut() {
                entry.retry_count = 100;
            }
        }
        let now = tokio::time::Instant::now();
        store.soft_flush().await.unwrap();
        // Yields only: the clock does not move, so the refusal is answered at
        // `now` and the next attempt is counted from it.
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        let flush_task_lives = store.background_failure().borrow().is_none();
        assert_eq!(
            (
                spy.call_count(),
                flush_task_lives,
                store.test_retry_state(SCHEDULE_MAP, "k"),
            ),
            (
                1,
                true,
                Some((101, Some(now + tokio::time::Duration::from_millis(5_000))))
            )
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_refusal_count_at_its_maximum_stays_there() {
        let spy = Arc::new(SpyDataStore::retaining());
        spy.refuse_everything(true);
        let store = schedule_store(&spy, schedule_config());
        store
            .add(SCHEDULE_MAP, "k", &lww("v", 1), 0, 1)
            .await
            .unwrap();
        for mut queue in store.queues.iter_mut() {
            for entry in queue.value_mut().entries.values_mut() {
                entry.retry_count = u32::MAX;
            }
        }
        let now = tokio::time::Instant::now();
        store.soft_flush().await.unwrap();
        // Yields only, so the clock stands at `now` when the refusal answers.
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        let flush_task_lives = store.background_failure().borrow().is_none();
        assert_eq!(
            (flush_task_lives, store.test_retry_state(SCHEDULE_MAP, "k")),
            (
                true,
                Some((
                    u32::MAX,
                    Some(now + tokio::time::Duration::from_millis(5_000))
                ))
            )
        );
    }

    #[tokio::test(start_paused = true)]
    async fn the_first_refusal_time_survives_a_rewrite_and_a_retirement() {
        let spy = Arc::new(SpyDataStore::retaining());
        spy.refuse_key("A");
        let store = schedule_store(&spy, schedule_config());
        store
            .add(SCHEDULE_MAP, "A", &lww("a1", 1), 0, 1)
            .await
            .unwrap();
        poll_until(30_000, || calls_answered(&spy) >= 1)
            .await
            .expect("the first refusal");
        let first_refusal = spy.timeline()[0].returned;
        let kept = |store: &WriteBehindDataStore| {
            first_refusal.is_some() && store.test_first_refused(SCHEDULE_MAP, "A") == first_refusal
        };
        let after_the_refusal = kept(&store);

        // Rewritten while its entry is queued.
        store
            .add(SCHEDULE_MAP, "A", &lww("a2", 2), 0, 1)
            .await
            .unwrap();
        let after_the_rewrite = kept(&store);

        // Rewritten while its entry is held by a pass, which is then refused:
        // the held entry is retired into the rewrite.
        let (entered, proceed) = spy.install_add_gate();
        tokio::time::timeout(tokio::time::Duration::from_secs(30), entered.notified())
            .await
            .expect("the second attempt never began");
        store
            .add(SCHEDULE_MAP, "A", &lww("a3", 3), 0, 1)
            .await
            .unwrap();
        proceed.notify_one();
        let _ = poll_until(30_000, || {
            calls_answered(&spy) == 2 && !store.test_pass_open()
        })
        .await;
        let after_the_retirement = kept(&store);
        assert_eq!(
            (after_the_refusal, after_the_rewrite, after_the_retirement),
            (true, true, true)
        );
    }

    /// A current-thread runtime on a paused clock, for a test that has to
    /// bind something to its thread around the runtime: a metrics recorder or
    /// a `tracing` subscriber belongs to the calling thread, and on such a
    /// runtime that is also the thread the flush task runs on.
    fn paused_runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .start_paused(true)
            .build()
            .expect("current-thread runtime")
    }

    #[test]
    fn refusals_are_counted_and_the_retry_and_buffer_gauges_follow() {
        const REFUSALS: &str = "topgun_write_behind_flush_refusals_total";
        const RETRYING: &str = "topgun_write_behind_retrying_entries";
        const BUFFERED: &str = "topgun_write_behind_buffered_entries";
        const AGE: &str = "topgun_write_behind_oldest_refusal_age_seconds";
        const HOLD: &str = "topgun_write_behind_store_hold";

        fn value_of(rendered: &str, name: &str) -> Option<f64> {
            rendered
                .lines()
                .find(|line| line.starts_with(&format!("{name} ")))
                .and_then(|line| line[name.len() + 1..].parse().ok())
        }

        let recorder = metrics_exporter_prometheus::PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        let (at_8s, at_9s, healed) = metrics::with_local_recorder(&recorder, || {
            paused_runtime().block_on(async {
                let spy = Arc::new(SpyDataStore::retaining());
                spy.refuse_everything(true);
                let store = schedule_store(&spy, schedule_config());
                // A `store_time` of 0 is what a caller with no clock of its own
                // passes: the age of a refusal must not be read from it.
                store
                    .add(SCHEDULE_MAP, "A", &lww("a", 1), 0, 0)
                    .await
                    .unwrap();
                advance(8_000).await;
                let at_8s = handle.render();
                store
                    .add(SCHEDULE_MAP, "A", &lww("a2", 2), 0, 0)
                    .await
                    .unwrap();
                for key in ["B", "C", "D"] {
                    store
                        .add(SCHEDULE_MAP, key, &lww("v", 1), 0, 1)
                        .await
                        .unwrap();
                }
                advance(1_000).await;
                let at_9s = handle.render();
                spy.refuse_everything(false);
                let _ = poll_until(60_000, || store.test_is_idle()).await;
                (at_8s, at_9s, handle.render())
            })
        });
        let first = (
            value_of(&at_8s, REFUSALS),
            value_of(&at_8s, RETRYING),
            value_of(&at_8s, BUFFERED),
            value_of(&at_8s, AGE).map(|age| (7.0..=8.0).contains(&age)),
            value_of(&at_8s, HOLD),
        );
        let second = (
            value_of(&at_9s, RETRYING).map(|retrying| retrying >= 1.0),
            value_of(&at_9s, BUFFERED),
            value_of(&at_9s, AGE).map(|age| age >= 8.0),
            value_of(&at_9s, HOLD),
        );
        let third = (
            value_of(&healed, RETRYING),
            value_of(&healed, BUFFERED),
            value_of(&healed, AGE),
            value_of(&healed, HOLD),
        );
        assert_eq!(
            (first, second, third),
            (
                (Some(4.0), Some(1.0), Some(1.0), Some(true), Some(0.0)),
                (Some(true), Some(4.0), Some(true), Some(1.0)),
                (Some(0.0), Some(0.0), Some(0.0), Some(0.0)),
            )
        );
    }

    /// One `tracing` event as a schedule test reads it: its level, the module
    /// that emitted it, and its fields by name, the text under `message`.
    #[derive(Debug, Clone)]
    struct ScheduleEvent {
        level: tracing::Level,
        target: String,
        fields: BTreeMap<String, String>,
    }

    impl ScheduleEvent {
        fn says(&self, text: &str) -> bool {
            self.fields.get("message").map(String::as_str) == Some(text)
        }
    }

    /// Collects every event of the thread it is the default subscriber on.
    struct ScheduleEventCapture(Arc<Mutex<Vec<ScheduleEvent>>>);

    struct ScheduleEventFields<'a>(&'a mut BTreeMap<String, String>);

    impl tracing::field::Visit for ScheduleEventFields<'_> {
        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            self.0.insert(field.name().to_string(), value.to_string());
        }

        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            self.0
                .insert(field.name().to_string(), format!("{value:?}"));
        }
    }

    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for ScheduleEventCapture {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            let mut fields = BTreeMap::new();
            event.record(&mut ScheduleEventFields(&mut fields));
            self.0.lock().unwrap().push(ScheduleEvent {
                level: *event.metadata().level(),
                target: event.metadata().target().to_string(),
                fields,
            });
        }
    }

    const RETRY_WARNING: &str = "Write-behind entries are being retried: the store refuses them";
    const BACKLOG_CLEARED: &str =
        "Write-behind retry backlog cleared: the store accepted every refused entry";

    /// Runs `scenario` on a paused clock and returns every event logged on
    /// its thread meanwhile. The subscriber is scoped to the thread, never
    /// installed globally: the test binary is shared and runs in parallel.
    fn events_on_a_paused_clock(
        scenario: impl std::future::Future<Output = ()>,
    ) -> Vec<ScheduleEvent> {
        use tracing_subscriber::layer::SubscriberExt;

        let events = Arc::new(Mutex::new(Vec::new()));
        let subscriber =
            tracing_subscriber::registry().with(ScheduleEventCapture(Arc::clone(&events)));
        tracing::subscriber::with_default(subscriber, || {
            paused_runtime().block_on(scenario);
        });
        let events = events.lock().unwrap().clone();
        events
    }

    #[test]
    fn the_only_warnings_of_a_refused_key_are_the_rate_limited_retry_warning() {
        let events = events_on_a_paused_clock(async {
            let spy = Arc::new(SpyDataStore::retaining());
            spy.refuse_everything(true);
            let store = schedule_store(&spy, schedule_config());
            store
                .add(SCHEDULE_MAP, "k", &lww("v", 1), 0, 1)
                .await
                .unwrap();
            advance(95_000).await;
            spy.refuse_everything(false);
            let _ = poll_until(60_000, || store.test_is_idle()).await;
            advance(1_000).await;
        });
        let loud: Vec<&ScheduleEvent> = events
            .iter()
            .filter(|event| {
                event.target.contains("write_behind")
                    && (event.level == tracing::Level::WARN || event.level == tracing::Level::ERROR)
            })
            .collect();
        let complete = loud
            .iter()
            .filter(|event| {
                event.level == tracing::Level::WARN
                    && event.says(RETRY_WARNING)
                    && [
                        "retrying",
                        "oldest_refusal_age_s",
                        "hold",
                        "map",
                        "key",
                        "attempts",
                        "error",
                    ]
                    .iter()
                    .all(|field| event.fields.contains_key(*field))
            })
            .count();
        let cleared = events
            .iter()
            .filter(|event| event.level == tracing::Level::INFO && event.says(BACKLOG_CLEARED))
            .count();
        assert_eq!(
            (loud.len(), complete, cleared),
            (4, 4, 1),
            "events at WARN or ERROR: {:?}",
            loud.iter()
                .map(|event| (event.level, &event.fields))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_backlog_that_never_warned_clears_without_a_recovery_line() {
        let events = events_on_a_paused_clock(async {
            let spy = Arc::new(SpyDataStore::retaining());
            spy.refuse_key("k");
            let store = schedule_store(&spy, schedule_config());
            store
                .add(SCHEDULE_MAP, "k", &lww("v", 1), 0, 1)
                .await
                .unwrap();
            let _ = poll_until(30_000, || calls_answered(&spy) >= 1).await;
            spy.accept_every_key();
            let _ = poll_until(60_000, || store.test_is_idle()).await;
            advance(1_000).await;
        });
        let warnings = events
            .iter()
            .filter(|event| event.says(RETRY_WARNING))
            .count();
        let cleared = events
            .iter()
            .filter(|event| event.says(BACKLOG_CLEARED))
            .count();
        assert_eq!((warnings, cleared), (0, 0));
    }
}

// ---------------------------------------------------------------------------
// Durability / crash-recovery tests (R12(b) WAL-retention + AC3f double-crash)
// ---------------------------------------------------------------------------

#[cfg(all(test, feature = "redb"))]
mod durability_tests {
    use super::*;
    use crate::storage::datastores::RedbDataStore;
    use crate::storage::wal::{WalRecovery, WalWriter};

    /// An OR-Map record carrying a single tombstone and no active records — the
    /// "element removed" state whose bytes the durability fence must never let a
    /// prune drop before they are durable.
    fn ormap_tombstone(tag: &str) -> RecordValue {
        RecordValue::OrMap {
            records: Vec::new(),
            tombstones: vec![tag.to_string()],
        }
    }

    fn buffered_config() -> WriteBehindConfig {
        // 60s delays so a write stays buffered (WAL-fsynced, not yet flushed to
        // the inner store) until we explicitly drain — the exact crash window.
        WriteBehindConfig {
            write_delay_ms: 60_000,
            flush_interval_ms: 60_000,
            shutdown_timeout_ms: 5_000,
            ..WriteBehindConfig::default()
        }
    }

    /// `R12(b)`: an un-flushed frame is NEVER discarded — a tombstone's bytes are
    /// always in the inner store OR still in the WAL, never both-lost. A WAL-GC
    /// change that deleted an unapplied frame would break this guard.
    #[tokio::test]
    async fn r12b_wal_retains_unflushed_tombstone_frame_until_applied() {
        let wal_dir = tempfile::tempdir().unwrap();
        let redb_dir = tempfile::tempdir().unwrap();
        let inner: Arc<dyn MapDataStore> =
            Arc::new(RedbDataStore::new(redb_dir.path().join("d.redb")).unwrap());
        let wal = WalWriter::new(wal_dir.path().to_path_buf(), WalFsyncPolicy::PerOp).unwrap();
        let store = WriteBehindDataStore::new_with_wal(
            Arc::clone(&inner),
            buffered_config(),
            Some(WalBootstrap {
                wal: Arc::clone(&wal) as Arc<dyn Wal>,
                sequence_start: 1,
            }),
        );

        store
            .add("m", "k1", &ormap_tombstone("T1"), 0, 1)
            .await
            .unwrap();
        let partition = partition_for("m", "k1");

        // Buffered: the WAL frame is fsynced but the inner store has not seen it.
        // R12(b) requires the WAL to still hold the frame (never both-lost).
        assert!(
            !wal.unapplied(partition).await.unwrap().is_empty(),
            "un-flushed tombstone frame must be retained in the WAL"
        );
        assert!(
            inner.load("m", "k1").await.unwrap().is_none(),
            "the buffered tombstone is not yet in the inner store"
        );

        // Drain to the inner store; only NOW is the frame eligible for release.
        store.hard_flush().await.unwrap();
        assert!(
            wal.unapplied(partition).await.unwrap().is_empty(),
            "once the bytes are durable in the inner store the frame is applied/releasable"
        );
        assert!(
            matches!(
                inner.load("m", "k1").await.unwrap(),
                Some(RecordValue::OrMap { tombstones, .. }) if tombstones == vec!["T1".to_string()]
            ),
            "tombstone bytes are durable in the inner store after the drain"
        );
    }

    /// `AC3f`: crash after WAL-fsync but before inner-store flush, recover, crash
    /// AGAIN during recovery (before the frame is marked applied) — the tombstone
    /// bytes are recoverable at every step (WAL retention held) and no removed
    /// element is resurrected.
    #[tokio::test]
    async fn ac3f_double_crash_wal_retention_recovers_tombstone_no_resurrection() {
        let wal_dir = tempfile::tempdir().unwrap();
        let redb_dir = tempfile::tempdir().unwrap();
        let redb_path = redb_dir.path().join("d.redb");
        let partition = partition_for("m", "k1");

        // --- Crash 1: after WAL fsync, before inner-store flush ---
        {
            // Emulate the write path's WAL-append-before-ack for a tombstone,
            // then a kill -9 STRICTLY before the background flush reaches the inner
            // store: fsync the frame, drop the writer, touch no inner store. (Going
            // through WriteBehindDataStore here would leave its background flush task
            // holding the inner-store handle — a real crash releases it; the direct
            // append is the faithful "frame fsynced, inner never written" window.)
            let wal = WalWriter::new(wal_dir.path().to_path_buf(), WalFsyncPolicy::PerOp).unwrap();
            let entry = WalEntry {
                map: "m".to_string(),
                key: "k1".to_string(),
                op: WalOp::Store {
                    value: WalStorePayload::Record(ormap_tombstone("T1")),
                    expiration_time: None,
                },
                timestamp: None,
                sequence: 1,
            };
            wal.append(partition, &entry).await.unwrap();
        }

        // --- Crash 2: DURING recovery — replay the still-unapplied frame into the
        // inner store, then "die" before marking it applied. ---
        {
            let inner: Arc<dyn MapDataStore> = Arc::new(RedbDataStore::new(&redb_path).unwrap());
            let wal = WalWriter::new(wal_dir.path().to_path_buf(), WalFsyncPolicy::PerOp).unwrap();
            let unapplied = wal.unapplied(partition).await.unwrap();
            assert!(
                !unapplied.is_empty(),
                "the un-flushed tombstone frame survived crash 1 in the WAL (R12(b))"
            );
            for entry in &unapplied {
                if let WalOp::Store {
                    value: WalStorePayload::Record(rv),
                    expiration_time,
                } = &entry.op
                {
                    let exp = expiration_time.unwrap_or(0);
                    inner.add(&entry.map, &entry.key, rv, exp, 1).await.unwrap();
                }
            }
            // Process "crashes" HERE — no mark_applied.
            assert!(
                matches!(
                    inner.load("m", "k1").await.unwrap(),
                    Some(RecordValue::OrMap { tombstones, .. }) if tombstones == vec!["T1".to_string()]
                ),
                "tombstone recovered into the inner store mid-recovery"
            );
        }

        // --- Recovery completes: crash 2 never marked the frame applied, so R12(b)
        // still holds it; a full WalRecovery replays it again (idempotent) and
        // finally marks it applied. The tombstone survives BOTH crashes. ---
        {
            let inner: Arc<dyn MapDataStore> = Arc::new(RedbDataStore::new(&redb_path).unwrap());
            let wal = WalWriter::new(wal_dir.path().to_path_buf(), WalFsyncPolicy::PerOp).unwrap();
            assert!(
                !wal.unapplied(partition).await.unwrap().is_empty(),
                "crash 2 did not mark the frame applied, so WAL retention still holds it"
            );
            let recovery = WalRecovery::new(Arc::clone(&wal), Vec::new());
            recovery.run(Arc::clone(&inner)).await.unwrap();

            match inner.load("m", "k1").await.unwrap() {
                Some(RecordValue::OrMap {
                    records,
                    tombstones,
                }) => {
                    assert_eq!(
                        tombstones,
                        vec!["T1".to_string()],
                        "tombstone present after both crashes — the removal is preserved"
                    );
                    assert!(
                        records.is_empty(),
                        "no active record resurrected for the removed element"
                    );
                }
                other => {
                    panic!("expected the OR-Map tombstone to survive both crashes, got {other:?}")
                }
            }
            assert!(
                wal.unapplied(partition).await.unwrap().is_empty(),
                "a completed recovery marks the frame applied"
            );
        }
    }
}

/// Proofs for the per-partition prefix-complete WAL watermark.
///
/// A CHILD module, not a sibling: `datastores/mod.rs` declares `mod write_behind`
/// privately, so nothing outside `datastores` can name this store's `pub(crate)`
/// internals. A child reaches the pending map, `max_assigned`, the in-flight
/// registry and the watermark-mode seam directly, with no production visibility
/// widened for a test.
#[cfg(test)]
#[path = "prefix_watermark_proptest.rs"]
mod prefix_watermark_proptest;

/// The deterministic write-behind + WAL crash/recovery harness's vocabulary
/// (op alphabet, incarnation shape, defect modes, reference-model trait,
/// typed violations).
///
/// A CHILD module, not a sibling, for the same reason
/// `prefix_watermark_proptest` is one: `datastores/mod.rs` declares `mod
/// write_behind` privately, so nothing outside `datastores` can name this
/// store's `pub(crate)` internals. A child reaches the pending map,
/// `max_assigned`, the in-flight registry, the `BootSeedMode` seam and the
/// `WatermarkMode` seam directly, with no production visibility widened for a
/// test.
#[cfg(test)]
#[path = "wal_harness/mod.rs"]
mod wal_harness;

/// Graceful shutdown must apply a key's writes in the order they were staged,
/// even when the flush loop is mid-batch when `hard_flush` starts (TG-OR-007).
#[cfg(all(test, feature = "redb"))]
mod hard_flush_order_tests {
    use super::*;
    use std::sync::Mutex;

    use crate::storage::datastores::RedbDataStore;
    use crate::storage::map_data_store::{LeafSink, ScanBatch, ScanCursor};

    const MAP: &str = "hard_flush_map";
    const KEY: &str = "k";
    const PARK_BOUND: std::time::Duration = std::time::Duration::from_secs(2);

    type Gate = (
        tokio::sync::oneshot::Sender<()>,
        tokio::sync::oneshot::Receiver<()>,
    );

    /// Forwards to `inner`; when armed, parks the next `add` for [`KEY`] on
    /// ENTRY, and records the order in which `add`/`remove` for [`KEY`] were
    /// applied to `inner`.
    struct ParkingAddStore {
        inner: Arc<dyn MapDataStore>,
        park: Mutex<Option<Gate>>,
        applied: Mutex<Vec<&'static str>>,
    }

    impl ParkingAddStore {
        fn new(inner: Arc<dyn MapDataStore>) -> Self {
            Self {
                inner,
                park: Mutex::new(None),
                applied: Mutex::new(Vec::new()),
            }
        }

        /// Returns the receiver that fires when the `add` parks, and the
        /// sender that releases it.
        fn arm(
            &self,
        ) -> (
            tokio::sync::oneshot::Receiver<()>,
            tokio::sync::oneshot::Sender<()>,
        ) {
            let (parked_tx, parked_rx) = tokio::sync::oneshot::channel();
            let (release_tx, release_rx) = tokio::sync::oneshot::channel();
            *self.park.lock().unwrap() = Some((parked_tx, release_rx));
            (parked_rx, release_tx)
        }

        fn applied(&self) -> Vec<&'static str> {
            self.applied.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl MapDataStore for ParkingAddStore {
        async fn add(
            &self,
            map: &str,
            key: &str,
            value: &RecordValue,
            expiration_time: i64,
            now: i64,
        ) -> anyhow::Result<()> {
            if key == KEY {
                let gate = self.park.lock().unwrap().take();
                if let Some((parked, release)) = gate {
                    let _ = parked.send(());
                    let _ = release.await;
                }
            }
            let result = self.inner.add(map, key, value, expiration_time, now).await;
            if key == KEY && result.is_ok() {
                self.applied.lock().unwrap().push("add");
            }
            result
        }

        async fn add_backup(
            &self,
            map: &str,
            key: &str,
            value: &RecordValue,
            expiration_time: i64,
            now: i64,
        ) -> anyhow::Result<()> {
            self.inner
                .add_backup(map, key, value, expiration_time, now)
                .await
        }

        async fn remove(&self, map: &str, key: &str, now: i64) -> anyhow::Result<()> {
            let result = self.inner.remove(map, key, now).await;
            if key == KEY && result.is_ok() {
                self.applied.lock().unwrap().push("remove");
            }
            result
        }

        async fn remove_backup(&self, map: &str, key: &str, now: i64) -> anyhow::Result<()> {
            self.inner.remove_backup(map, key, now).await
        }

        async fn load(&self, map: &str, key: &str) -> anyhow::Result<Option<RecordValue>> {
            self.inner.load(map, key).await
        }

        async fn load_all(
            &self,
            map: &str,
            keys: &[String],
        ) -> anyhow::Result<Vec<(String, RecordValue)>> {
            self.inner.load_all(map, keys).await
        }

        async fn enumerate_leaves(
            &self,
            map: &str,
            is_backup: bool,
            sink: &mut dyn LeafSink,
        ) -> anyhow::Result<()> {
            self.inner.enumerate_leaves(map, is_backup, sink).await
        }

        async fn scan_values(
            &self,
            map: &str,
            is_backup: bool,
            max_batch_cost: u64,
        ) -> anyhow::Result<ScanBatch> {
            self.inner.scan_values(map, is_backup, max_batch_cost).await
        }

        async fn scan_values_batched(
            &self,
            map: &str,
            is_backup: bool,
            cursor: ScanCursor,
            max_batch_cost: u64,
        ) -> anyhow::Result<ScanBatch> {
            self.inner
                .scan_values_batched(map, is_backup, cursor, max_batch_cost)
                .await
        }

        async fn remove_all(&self, map: &str, keys: &[String]) -> anyhow::Result<()> {
            self.inner.remove_all(map, keys).await
        }

        fn is_loadable(&self, key: &str) -> bool {
            self.inner.is_loadable(key)
        }

        fn pending_operation_count(&self) -> u64 {
            self.inner.pending_operation_count()
        }

        async fn soft_flush(&self) -> anyhow::Result<u64> {
            self.inner.soft_flush().await
        }

        async fn hard_flush(&self) -> anyhow::Result<()> {
            self.inner.hard_flush().await
        }
    }

    // AC-15: a Remove staged after the flush loop dequeued the key's Store must
    // land after that Store, also when `hard_flush` starts while the Store is
    // in flight.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn hard_flush_applies_a_later_remove_after_the_loops_in_flight_store() {
        let dir = tempfile::tempdir().expect("tempdir");
        let redb: Arc<dyn MapDataStore> =
            Arc::new(RedbDataStore::new(dir.path().join("hard_flush.redb")).expect("redb open"));
        let parking = Arc::new(ParkingAddStore::new(Arc::clone(&redb)));
        let (parked, release) = parking.arm();
        // The shutdown budget is well above the park bound, so the join in
        // `hard_flush` never times out while the store is parked.
        let wb = WriteBehindDataStore::new(
            parking.clone(),
            WriteBehindConfig {
                write_delay_ms: 10,
                flush_interval_ms: 10,
                batch_size: 100,
                backoff_base_ms: 10,
                backoff_cap_ms: 100,
                capacity: 0,
                shutdown_timeout_ms: 5_000,
                ..WriteBehindConfig::default()
            },
        );
        let value = RecordValue::OrMap {
            records: Vec::new(),
            tombstones: vec!["t".to_string()],
        };

        wb.add(MAP, KEY, &value, 0, now_millis())
            .await
            .expect("add");
        tokio::time::timeout(PARK_BOUND, parked)
            .await
            .expect("the flush loop never reached the inner add")
            .expect("park dropped");

        wb.remove(MAP, KEY, now_millis()).await.expect("remove");
        let flusher = {
            let wb = Arc::clone(&wb);
            tokio::spawn(async move { wb.hard_flush().await })
        };
        let deadline = tokio::time::Instant::now() + PARK_BOUND;
        while tokio::time::Instant::now() < deadline && !parking.applied().contains(&"remove") {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let _ = release.send(());
        flusher.await.expect("hard_flush task").expect("hard_flush");

        assert_eq!(
            parking.applied(),
            vec!["add", "remove"],
            "the later Remove must be applied after the in-flight Store"
        );
        assert!(
            redb.load(MAP, KEY).await.expect("load").is_none(),
            "the removed key must stay removed"
        );
    }
}

/// A write-behind background task that ends by a panic must not leave a store
/// that keeps acknowledging writes nothing will ever flush.
///
/// Waiting here is by a message sent from the injection point immediately
/// before it panics, on a current-thread runtime: with one thread the test
/// task cannot run again until the poll of the panicking task has returned, so
/// the first statement after the receipt already sees whatever that task left
/// behind. A timeout around a receipt only bounds a hung test.
#[cfg(test)]
mod background_task_panic_tests {
    use super::*;

    use crate::storage::datastores::NullDataStore;

    const MAP: &str = "background_task_panic_map";
    /// Upper bound on any single wait; reaching it is a failed test, not a wait.
    const HANG_LIMIT: std::time::Duration = std::time::Duration::from_secs(5);

    fn lww_value() -> RecordValue {
        RecordValue::Lww {
            value: topgun_core::types::Value::Null,
            timestamp: topgun_core::hlc::Timestamp {
                millis: 0,
                counter: 0,
                node_id: String::new(),
            },
        }
    }

    /// Forwards to `inner`, except that the `panic_on`-th `add` announces itself
    /// on a channel and then panics instead of returning. Counting calls rather
    /// than matching a key lets a test place the panic in the flush task's pass
    /// and leave the shutdown drain's later calls alone. `panic_on` 0 never
    /// panics.
    ///
    /// An armed park holds the next `add` on entry, before the count is taken:
    /// the flush task then sits inside a pass, holding its batch, for as long
    /// as the test needs to act around it.
    struct PanickingAddStore {
        inner: Arc<dyn MapDataStore>,
        panic_on: u64,
        adds: AtomicU64,
        about_to_panic: tokio::sync::mpsc::UnboundedSender<()>,
        park: Mutex<Option<ParkGate>>,
    }

    impl PanickingAddStore {
        /// Over a store that keeps nothing. Returns the store and the receiver
        /// that yields one message immediately before the panic.
        fn new(panic_on: u64) -> (Arc<Self>, tokio::sync::mpsc::UnboundedReceiver<()>) {
            Self::over(Arc::new(NullDataStore), panic_on)
        }

        /// Over `inner`, which receives every `add` that does not panic.
        fn over(
            inner: Arc<dyn MapDataStore>,
            panic_on: u64,
        ) -> (Arc<Self>, tokio::sync::mpsc::UnboundedReceiver<()>) {
            let (about_to_panic, announced) = tokio::sync::mpsc::unbounded_channel();
            let store = Arc::new(Self {
                inner,
                panic_on,
                adds: AtomicU64::new(0),
                about_to_panic,
                park: Mutex::new(None),
            });
            (store, announced)
        }

        /// Parks the next `add` on entry. Dropping the handle releases it.
        fn arm_park(&self) -> TestParkHandle {
            let (gate, handle) = TestParkHandle::arm();
            *self
                .park
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(gate);
            handle
        }
    }

    #[async_trait]
    impl MapDataStore for PanickingAddStore {
        async fn add(
            &self,
            map: &str,
            key: &str,
            value: &RecordValue,
            expiration_time: i64,
            now: i64,
        ) -> anyhow::Result<()> {
            let gate = self
                .park
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            if let Some(gate) = gate {
                gate.pass().await;
            }
            if self.adds.fetch_add(1, Ordering::SeqCst) + 1 == self.panic_on {
                let _ = self.about_to_panic.send(());
                panic!("injected inner-store panic");
            }
            self.inner.add(map, key, value, expiration_time, now).await
        }

        async fn add_backup(
            &self,
            map: &str,
            key: &str,
            value: &RecordValue,
            expiration_time: i64,
            now: i64,
        ) -> anyhow::Result<()> {
            self.inner
                .add_backup(map, key, value, expiration_time, now)
                .await
        }

        async fn remove(&self, map: &str, key: &str, now: i64) -> anyhow::Result<()> {
            self.inner.remove(map, key, now).await
        }

        async fn remove_backup(&self, map: &str, key: &str, now: i64) -> anyhow::Result<()> {
            self.inner.remove_backup(map, key, now).await
        }

        async fn load(&self, map: &str, key: &str) -> anyhow::Result<Option<RecordValue>> {
            self.inner.load(map, key).await
        }

        async fn load_all(
            &self,
            map: &str,
            keys: &[String],
        ) -> anyhow::Result<Vec<(String, RecordValue)>> {
            self.inner.load_all(map, keys).await
        }

        async fn enumerate_leaves(
            &self,
            map: &str,
            is_backup: bool,
            sink: &mut dyn LeafSink,
        ) -> anyhow::Result<()> {
            self.inner.enumerate_leaves(map, is_backup, sink).await
        }

        async fn scan_values(
            &self,
            map: &str,
            is_backup: bool,
            max_batch_cost: u64,
        ) -> anyhow::Result<ScanBatch> {
            self.inner.scan_values(map, is_backup, max_batch_cost).await
        }

        async fn scan_values_batched(
            &self,
            map: &str,
            is_backup: bool,
            cursor: ScanCursor,
            max_batch_cost: u64,
        ) -> anyhow::Result<ScanBatch> {
            self.inner
                .scan_values_batched(map, is_backup, cursor, max_batch_cost)
                .await
        }

        async fn remove_all(&self, map: &str, keys: &[String]) -> anyhow::Result<()> {
            self.inner.remove_all(map, keys).await
        }

        fn is_loadable(&self, key: &str) -> bool {
            self.inner.is_loadable(key)
        }

        fn pending_operation_count(&self) -> u64 {
            self.inner.pending_operation_count()
        }

        async fn soft_flush(&self) -> anyhow::Result<u64> {
            self.inner.soft_flush().await
        }

        async fn hard_flush(&self) -> anyhow::Result<()> {
            self.inner.hard_flush().await
        }
    }

    /// A WAL that accepts every frame and holds none. It is here because the
    /// stall watchdog returns at once on a store that has no WAL.
    struct AcceptingWal;

    #[async_trait]
    impl Wal for AcceptingWal {
        async fn append(&self, _partition: u32, _entry: &WalEntry) -> anyhow::Result<()> {
            Ok(())
        }

        async fn mark_applied(&self, _partition: u32, _sequence: u64) -> anyhow::Result<()> {
            Ok(())
        }

        async fn unapplied(&self, _partition: u32) -> anyhow::Result<Vec<WalEntry>> {
            Ok(Vec::new())
        }
    }

    /// What a [`RecordingWal`] was asked to do, in call order.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum WalCall {
        Append,
        MarkApplied,
        Sync,
    }

    /// A WAL that holds no frame and records which of its mutating calls were
    /// made, in order. `unapplied` is a read and is not recorded.
    #[derive(Default)]
    struct RecordingWal {
        calls: Mutex<Vec<WalCall>>,
    }

    impl RecordingWal {
        fn record(&self, call: WalCall) {
            self.calls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(call);
        }

        fn calls(&self) -> Vec<WalCall> {
            self.calls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
        }

        fn appends(&self) -> usize {
            self.calls()
                .iter()
                .filter(|call| **call == WalCall::Append)
                .count()
        }
    }

    #[async_trait]
    impl Wal for RecordingWal {
        async fn append(&self, _partition: u32, _entry: &WalEntry) -> anyhow::Result<()> {
            self.record(WalCall::Append);
            Ok(())
        }

        async fn mark_applied(&self, _partition: u32, _sequence: u64) -> anyhow::Result<()> {
            self.record(WalCall::MarkApplied);
            Ok(())
        }

        async fn unapplied(&self, _partition: u32) -> anyhow::Result<Vec<WalEntry>> {
            Ok(Vec::new())
        }

        async fn sync(&self) -> anyhow::Result<()> {
            self.record(WalCall::Sync);
            Ok(())
        }
    }

    /// An inner store that keeps what it is given, so a test can read back
    /// what the flush task, the shutdown drain or a WAL replay put there. With
    /// `refuse_adds` set every `add` fails, which is how a shutdown drain is
    /// made to give up on an entry without any clock being involved.
    #[derive(Default)]
    struct MemoryStore {
        values: Mutex<HashMap<(String, String), RecordValue>>,
        refuse_adds: AtomicBool,
    }

    impl MemoryStore {
        fn values(&self) -> std::sync::MutexGuard<'_, HashMap<(String, String), RecordValue>> {
            self.values
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        }

        fn get(&self, map: &str, key: &str) -> Option<RecordValue> {
            self.values()
                .get(&(map.to_string(), key.to_string()))
                .cloned()
        }
    }

    #[async_trait]
    impl MapDataStore for MemoryStore {
        async fn add(
            &self,
            map: &str,
            key: &str,
            value: &RecordValue,
            _expiration_time: i64,
            _now: i64,
        ) -> anyhow::Result<()> {
            if self.refuse_adds.load(Ordering::SeqCst) {
                anyhow::bail!("injected inner-store refusal");
            }
            self.values()
                .insert((map.to_string(), key.to_string()), value.clone());
            Ok(())
        }

        async fn add_backup(
            &self,
            map: &str,
            key: &str,
            value: &RecordValue,
            expiration_time: i64,
            now: i64,
        ) -> anyhow::Result<()> {
            NullDataStore
                .add_backup(map, key, value, expiration_time, now)
                .await
        }

        async fn remove(&self, map: &str, key: &str, _now: i64) -> anyhow::Result<()> {
            self.values().remove(&(map.to_string(), key.to_string()));
            Ok(())
        }

        async fn remove_backup(&self, map: &str, key: &str, now: i64) -> anyhow::Result<()> {
            NullDataStore.remove_backup(map, key, now).await
        }

        async fn load(&self, map: &str, key: &str) -> anyhow::Result<Option<RecordValue>> {
            Ok(self.get(map, key))
        }

        async fn load_all(
            &self,
            map: &str,
            keys: &[String],
        ) -> anyhow::Result<Vec<(String, RecordValue)>> {
            Ok(keys
                .iter()
                .filter_map(|key| self.get(map, key).map(|value| (key.clone(), value)))
                .collect())
        }

        async fn enumerate_leaves(
            &self,
            map: &str,
            is_backup: bool,
            sink: &mut dyn LeafSink,
        ) -> anyhow::Result<()> {
            NullDataStore.enumerate_leaves(map, is_backup, sink).await
        }

        async fn scan_values(
            &self,
            map: &str,
            is_backup: bool,
            max_batch_cost: u64,
        ) -> anyhow::Result<ScanBatch> {
            NullDataStore
                .scan_values(map, is_backup, max_batch_cost)
                .await
        }

        async fn scan_values_batched(
            &self,
            map: &str,
            is_backup: bool,
            cursor: ScanCursor,
            max_batch_cost: u64,
        ) -> anyhow::Result<ScanBatch> {
            NullDataStore
                .scan_values_batched(map, is_backup, cursor, max_batch_cost)
                .await
        }

        async fn remove_all(&self, map: &str, keys: &[String]) -> anyhow::Result<()> {
            let mut values = self.values();
            for key in keys {
                values.remove(&(map.to_string(), key.clone()));
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
    }

    /// One `tracing` event as a test reads it: its level and its fields by
    /// name, the text under `message`.
    #[derive(Debug, Clone)]
    struct CapturedEvent {
        level: tracing::Level,
        fields: BTreeMap<String, String>,
    }

    impl CapturedEvent {
        fn field(&self, name: &str) -> Option<&str> {
            self.fields.get(name).map(String::as_str)
        }
    }

    /// Collects every event of the thread it is the default subscriber on.
    struct EventCapture(Arc<Mutex<Vec<CapturedEvent>>>);

    struct FieldCollector<'a>(&'a mut BTreeMap<String, String>);

    impl tracing::field::Visit for FieldCollector<'_> {
        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            self.0.insert(field.name().to_string(), value.to_string());
        }

        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            self.0
                .insert(field.name().to_string(), format!("{value:?}"));
        }
    }

    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for EventCapture {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            let mut fields = BTreeMap::new();
            event.record(&mut FieldCollector(&mut fields));
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(CapturedEvent {
                    level: *event.metadata().level(),
                    fields,
                });
        }
    }

    /// Runs `body` on a current-thread runtime and returns what it returned
    /// together with every event logged on that thread meanwhile.
    ///
    /// The subscriber is scoped to the thread, never installed globally: the
    /// test binary is shared and runs in parallel. One thread is also what
    /// makes the capture complete — the store's background tasks run on it.
    fn run_captured<T>(body: impl std::future::Future<Output = T>) -> (T, Vec<CapturedEvent>) {
        use tracing_subscriber::layer::SubscriberExt;

        let events = Arc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::registry().with(EventCapture(Arc::clone(&events)));
        let output = tracing::subscriber::with_default(subscriber, || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("current-thread runtime")
                .block_on(body)
        });
        let events = events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        (output, events)
    }

    /// The events whose text is exactly `message`.
    fn events_saying<'a>(events: &'a [CapturedEvent], message: &str) -> Vec<&'a CapturedEvent> {
        events
            .iter()
            .filter(|event| event.field("message") == Some(message))
            .collect()
    }

    const PANIC_LINE: &str =
        "write-behind background task panicked; the store refuses writes from now on";
    const FAILED_SHUTDOWN_LINE: &str = "write-behind shutdown on a failed store: entries not \
         persisted here stay un-resolved and replay from the WAL at the next start when the \
         store has one";
    const FLUSH_LOOP_ABORTED_LINE_START: &str = "write-behind flush loop did not finish";

    /// An LWW value that can be told from another by its text, with `millis`
    /// as its timestamp.
    fn lww_text(text: &str, millis: u64) -> RecordValue {
        RecordValue::Lww {
            value: topgun_core::types::Value::String(text.to_string()),
            timestamp: topgun_core::hlc::Timestamp {
                millis,
                counter: 0,
                node_id: "node-a".to_string(),
            },
        }
    }

    /// A flush task that runs a pass only when a test asks for one
    /// (`soft_flush`), and then takes everything queued. With a timer-driven
    /// loop the test could not say which entries a pass holds.
    fn manual_flush_config() -> WriteBehindConfig {
        WriteBehindConfig {
            write_delay_ms: 0,
            flush_interval_ms: 60_000,
            capacity: 0,
            shutdown_timeout_ms: 5_000,
            ..WriteBehindConfig::default()
        }
    }

    fn bootstrap(wal: Arc<dyn Wal>) -> WalBootstrap {
        WalBootstrap {
            wal,
            sequence_start: 1,
        }
    }

    const DEAD_BATCH_KEY: &str = "in-the-dead-batch";

    /// A store whose flush task has just panicked holding a batch of one
    /// entry, [`DEAD_BATCH_KEY`]. Returns when the poll of that task has
    /// returned; nothing was awaited after the receipt of its announcement.
    async fn store_with_a_dead_flush_task(
        wal: Option<Arc<dyn Wal>>,
    ) -> (Arc<WriteBehindDataStore>, Arc<MemoryStore>) {
        let memory = Arc::new(MemoryStore::default());
        let (inner, mut about_to_panic) =
            PanickingAddStore::over(Arc::clone(&memory) as Arc<dyn MapDataStore>, 1);
        let store =
            WriteBehindDataStore::new_with_wal(inner, manual_flush_config(), wal.map(bootstrap));
        store
            .add(MAP, DEAD_BATCH_KEY, &lww_value(), 0, now_millis())
            .await
            .expect("a healthy store accepts the write");
        store.soft_flush().await.expect("soft_flush");
        tokio::time::timeout(HANG_LIMIT, about_to_panic.recv())
            .await
            .expect("the flush task never reached the inner store")
            .expect("the inner store was dropped before it panicked");
        (store, memory)
    }

    /// The lifecycle value, read the way a write entry reads it.
    fn lifecycle(store: &WriteBehindDataStore) -> Lifecycle {
        store.lifecycle.load(Ordering::Acquire)
    }

    #[tokio::test]
    async fn a_flush_task_panic_fails_the_store() {
        let (inner, mut about_to_panic) = PanickingAddStore::new(1);
        let store = WriteBehindDataStore::new(
            inner,
            // No write delay, so the first flush tick already takes the entry.
            WriteBehindConfig {
                write_delay_ms: 0,
                flush_interval_ms: 10,
                capacity: 0,
                shutdown_timeout_ms: 5_000,
                ..WriteBehindConfig::default()
            },
        );
        let value = lww_value();

        store
            .add(MAP, "in-the-dead-batch", &value, 0, now_millis())
            .await
            .expect("a healthy store accepts the write");
        tokio::time::timeout(HANG_LIMIT, about_to_panic.recv())
            .await
            .expect("the flush task never reached the inner store")
            .expect("the inner store was dropped before it panicked");

        // Nothing is awaited between the receipt and this call: it is the first
        // write after the poll of the panicked flush task returned.
        let after_panic = store
            .add(MAP, "after-the-panic", &value, 0, now_millis())
            .await;
        let shutdown = store.hard_flush().await;

        assert!(
            after_panic.is_err(),
            "`add` returned Ok after the flush task panicked: the write was acknowledged by \
             a store that no task flushes any more (`hard_flush` then returned {shutdown:?})"
        );
        assert!(
            shutdown.is_err(),
            "`hard_flush` returned Ok(()) on a store whose flush task panicked holding a batch"
        );
    }

    #[tokio::test]
    async fn a_stall_watchdog_panic_fails_the_store() {
        let wal: Arc<dyn Wal> = Arc::new(AcceptingWal);
        let store = WriteBehindDataStore::new_with_wal(
            Arc::new(NullDataStore),
            // The flush task stays out of the way for the whole test, and the
            // watchdog ticks at its floor instead of once every six seconds.
            WriteBehindConfig {
                write_delay_ms: 60_000,
                flush_interval_ms: 60_000,
                capacity: 0,
                shutdown_timeout_ms: 5_000,
                wal_watermark_stall_bound_ms: 1_000,
                ..WriteBehindConfig::default()
            },
            Some(WalBootstrap {
                wal,
                sequence_start: 1,
            }),
        );
        let (about_to_panic, mut announced) = tokio::sync::mpsc::unbounded_channel();
        store.test_set_classifier_hook(Arc::new(move |_partition| {
            let _ = about_to_panic.send(());
            panic!("injected stall watchdog panic");
        }));
        let value = lww_value();

        // The watchdog samples only partitions that have tracking state, and a
        // partition gets its state with its first write.
        store
            .add(MAP, "tracked", &value, 0, now_millis())
            .await
            .expect("a healthy store accepts the write");
        tokio::time::timeout(HANG_LIMIT, announced.recv())
            .await
            .expect("the stall watchdog never sampled the written partition")
            .expect("the classifier hook was dropped before it ran");

        let after_panic = store
            .add(MAP, "after-the-panic", &value, 0, now_millis())
            .await;

        assert!(
            after_panic.is_err(),
            "`add` returned Ok after the stall watchdog panicked: the store runs on without \
             its only stall detector"
        );
    }

    #[tokio::test]
    async fn a_flush_task_panic_is_observed_without_any_call() {
        let (store, _memory) = store_with_a_dead_flush_task(None).await;

        // Read at once: no `hard_flush`, no write and no suspension point lies
        // between the panic and this read, so nothing but the dying task can
        // have put the record there.
        let failure = store.background_failure().borrow().clone();

        assert_eq!(
            failure,
            Some(BackgroundFailure {
                task: BackgroundTask::Flush,
                message: "injected inner-store panic".to_string(),
                cause: FailureCause::Panicked,
            }),
            "the store must learn of its flush task's panic from the task itself"
        );
    }

    #[test]
    fn a_flush_task_panic_is_logged_once_at_error() {
        let ((), events) = run_captured(async {
            let _store = store_with_a_dead_flush_task(None).await;
        });

        let lines = events_saying(&events, PANIC_LINE);
        assert_eq!(
            lines.len(),
            1,
            "one panic must give exactly one line; captured events: {events:?}"
        );
        assert_eq!(lines[0].level, tracing::Level::ERROR);
        assert_eq!(lines[0].field("task"), Some("flush"));
        assert_eq!(lines[0].field("panic"), Some("injected inner-store panic"));
    }

    #[test]
    fn a_background_task_panic_is_counted_by_task() {
        fn series(task: &str, value: u64) -> String {
            format!("{BACKGROUND_TASK_PANICS_COUNTER}{{task=\"{task}\"}} {value}")
        }
        fn renders(rendered: &str, line: &str) -> bool {
            rendered.lines().any(|rendered_line| rendered_line == line)
        }

        let recorder = metrics_exporter_prometheus::PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        // The recorder is bound around the runtime, not inside a task of it: it
        // belongs to the calling thread, and on a current-thread runtime that
        // is also the thread the dying flush task increments the counter on.
        let (at_construction, after_the_panic) = metrics::with_local_recorder(&recorder, || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("current-thread runtime")
                .block_on(async {
                    let (inner, mut about_to_panic) = PanickingAddStore::new(1);
                    let store = WriteBehindDataStore::new(inner, manual_flush_config());
                    let at_construction = handle.render();

                    store
                        .add(MAP, DEAD_BATCH_KEY, &lww_value(), 0, now_millis())
                        .await
                        .expect("a healthy store accepts the write");
                    store.soft_flush().await.expect("soft_flush");
                    tokio::time::timeout(HANG_LIMIT, about_to_panic.recv())
                        .await
                        .expect("the flush task never reached the inner store")
                        .expect("the inner store was dropped before it panicked");
                    (at_construction, handle.render())
                })
        });

        // Line by line: the order of the two series in a render is not stable.
        for task in ["flush", "stall_watchdog"] {
            assert!(
                renders(&at_construction, &series(task, 0)),
                "the `{task}` series must render at 0 from construction; render was:\n\
                 {at_construction}"
            );
        }
        assert!(
            renders(&after_the_panic, &series("flush", 1)),
            "the flush task's panic must be counted once; render was:\n{after_the_panic}"
        );
        assert!(
            renders(&after_the_panic, &series("stall_watchdog", 0)),
            "the watchdog did not panic and must still read 0; render was:\n{after_the_panic}"
        );
    }

    /// A failed store over a WAL that records its calls, with how many frames
    /// were appended and how many entries were pending when the task died.
    async fn failed_store_over_a_recording_wal(
    ) -> (Arc<WriteBehindDataStore>, Arc<RecordingWal>, usize, u64) {
        let wal = Arc::new(RecordingWal::default());
        let (store, _memory) =
            store_with_a_dead_flush_task(Some(Arc::clone(&wal) as Arc<dyn Wal>)).await;
        let appends = wal.appends();
        let pending = store.pending_operation_count();
        assert_eq!(appends, 1, "the dead batch's one frame was appended");
        assert_eq!(pending, 1, "the dead batch's one entry is still counted");
        (store, wal, appends, pending)
    }

    #[tokio::test]
    async fn a_failed_store_refuses_add() {
        let (store, wal, appends, pending) = failed_store_over_a_recording_wal().await;

        let refused = store
            .add(MAP, "after-the-panic", &lww_value(), 0, now_millis())
            .await
            .expect_err("a failed store must refuse `add`");

        assert_eq!(
            refused.to_string(),
            format!(
                "write-behind store has failed (background task panicked); write rejected for \
                 map={MAP} key=after-the-panic"
            )
        );
        assert_eq!(
            wal.appends(),
            appends,
            "a refused write must not reach the WAL: its frame would replay a write the client \
             was told had failed"
        );
        assert_eq!(store.pending_operation_count(), pending);
    }

    #[tokio::test]
    async fn a_failed_store_refuses_remove() {
        let (store, wal, appends, pending) = failed_store_over_a_recording_wal().await;

        let refused = store
            .remove(MAP, "after-the-panic", now_millis())
            .await
            .expect_err("a failed store must refuse `remove`");

        assert_eq!(
            refused.to_string(),
            format!(
                "write-behind store has failed (background task panicked); write rejected for \
                 map={MAP} key=after-the-panic"
            )
        );
        assert_eq!(
            wal.appends(),
            appends,
            "a refused remove must not reach the WAL"
        );
        assert_eq!(store.pending_operation_count(), pending);
    }

    #[tokio::test]
    async fn a_failed_store_refuses_remove_all() {
        let (store, wal, appends, pending) = failed_store_over_a_recording_wal().await;

        let refused = store
            .remove_all(MAP, &["first".to_string(), "second".to_string()])
            .await
            .expect_err("a failed store must refuse `remove_all`");

        assert_eq!(
            refused.to_string(),
            format!(
                "write-behind store has failed (background task panicked); write rejected for \
                 map={MAP}"
            )
        );
        assert_eq!(
            wal.appends(),
            appends,
            "a refused remove_all must not reach the WAL for any of its keys"
        );
        assert_eq!(store.pending_operation_count(), pending);
    }

    /// The shutdown of a store whose flush task panicked renders its context
    /// byte for byte: a broken store has a context of its own, and the panic
    /// one must not drift with it.
    #[tokio::test]
    async fn a_panicked_store_shutdown_keeps_its_context_text() {
        let (store, _memory) = store_with_a_dead_flush_task(None).await;

        let shutdown = tokio::time::timeout(HANG_LIMIT, store.hard_flush())
            .await
            .expect("hard_flush on a store whose flush task panicked did not return");
        let error = shutdown.expect_err("a shutdown on a store whose flush task panicked fails");

        assert_eq!(
            error.to_string(),
            "write-behind shutdown ran on a failed store: a background task panicked"
        );
    }

    #[tokio::test]
    async fn a_write_past_the_check_when_the_task_dies_is_drained_at_shutdown() {
        const PARKED_KEY: &str = "past-the-check";

        let memory = Arc::new(MemoryStore::default());
        let (inner, mut about_to_panic) =
            PanickingAddStore::over(Arc::clone(&memory) as Arc<dyn MapDataStore>, 1);
        let store = WriteBehindDataStore::new(inner, manual_flush_config());
        let parked_value = lww_text("past-the-check", 1);

        // This write loads the state while the store is healthy and stops
        // before it takes the queue.
        let mut park = store.test_park_before_queue_insert(MAP, PARKED_KEY);
        let parked_write = {
            let store = Arc::clone(&store);
            let value = parked_value.clone();
            tokio::spawn(async move { store.add(MAP, PARKED_KEY, &value, 0, now_millis()).await })
        };
        park.wait_parked().await;

        store
            .add(MAP, DEAD_BATCH_KEY, &lww_value(), 0, now_millis())
            .await
            .expect("a healthy store accepts the write");
        store.soft_flush().await.expect("soft_flush");
        tokio::time::timeout(HANG_LIMIT, about_to_panic.recv())
            .await
            .expect("the flush task never reached the inner store")
            .expect("the inner store was dropped before it panicked");
        assert!(
            store.background_failure().borrow().is_some(),
            "the failure must be on record before the parked write is let go"
        );

        park.release();
        let parked_result = tokio::time::timeout(HANG_LIMIT, parked_write)
            .await
            .expect("the parked write never returned")
            .expect("the parked write's task");
        assert!(
            parked_result.is_ok(),
            "a write that passed the state check before the failure was recorded must be \
             accepted, not refused half-way with its entry sequence already assigned: \
             {parked_result:?}"
        );

        // What the shutdown returns is not this test's subject.
        let _ = store.hard_flush().await;

        assert_eq!(
            memory.get(MAP, PARKED_KEY),
            Some(parked_value),
            "the shutdown drain of a failed store must still persist what is queued"
        );
    }

    /// `count` keys of [`MAP`] that share one partition.
    fn keys_in_one_partition(count: usize) -> Vec<String> {
        let mut by_partition: HashMap<u32, Vec<String>> = HashMap::new();
        for index in 0u32.. {
            let key = format!("key-{index}");
            let bucket = by_partition.entry(partition_for(MAP, &key)).or_default();
            bucket.push(key);
            if bucket.len() == count {
                return bucket.clone();
            }
        }
        unreachable!("the key space is unbounded")
    }

    /// The keys of the stranded-batch scenario. They share one partition
    /// because the applied watermark is per partition: a defect that moves it
    /// past the dead batch's frames can only be seen from a key whose frames
    /// lie in the partition the shutdown drain advances.
    struct StrandedKeys {
        /// Written once; its entry is in the dead batch.
        batch_only: String,
        /// Written twice: the older value is in the dead batch, the newer one
        /// is persisted by the shutdown drain.
        in_both: String,
        /// Written once; persisted by the shutdown drain.
        drained_only: String,
        /// An OR-Map key written with delta frames: one tag in the dead batch,
        /// a second one persisted by the shutdown drain.
        or_map: String,
    }

    impl StrandedKeys {
        fn chosen() -> Self {
            let mut keys = keys_in_one_partition(4).into_iter();
            let mut next = || keys.next().expect("four keys");
            Self {
                batch_only: next(),
                in_both: next(),
                drained_only: next(),
                or_map: next(),
            }
        }

        /// Asserts the keys share one partition and returns it.
        fn assert_one_partition(&self) -> u32 {
            let partition = partition_for(MAP, &self.batch_only);
            for key in [&self.in_both, &self.drained_only, &self.or_map] {
                assert_eq!(
                    partition_for(MAP, key),
                    partition,
                    "`{key}` must share the partition of `{}`",
                    self.batch_only
                );
            }
            partition
        }
    }

    fn batch_only_value() -> RecordValue {
        lww_text("batch-only", 1)
    }

    fn older_value() -> RecordValue {
        lww_text("older", 2)
    }

    fn newer_value() -> RecordValue {
        lww_text("newer", 3)
    }

    fn drained_only_value() -> RecordValue {
        lww_text("drained-only", 4)
    }

    fn or_entry(tag: &str) -> crate::storage::record::OrMapEntry {
        crate::storage::record::OrMapEntry {
            value: topgun_core::types::Value::String(format!("value-of-{tag}")),
            tag: tag.to_string(),
            timestamp: topgun_core::hlc::Timestamp {
                millis: 1,
                counter: 0,
                node_id: "node-a".to_string(),
            },
        }
    }

    fn or_snapshot(tags: &[&str]) -> RecordValue {
        RecordValue::OrMap {
            records: tags.iter().map(|tag| or_entry(tag)).collect(),
            tombstones: Vec::new(),
        }
    }

    /// The `(key, value)` of every full-record store frame, in frame order.
    fn stored_frames(frames: &[WalEntry]) -> Vec<(String, RecordValue)> {
        frames
            .iter()
            .filter_map(|frame| match &frame.op {
                WalOp::Store {
                    value: WalStorePayload::Record(value),
                    ..
                } => Some((frame.key.clone(), value.clone())),
                _ => None,
            })
            .collect()
    }

    /// What the stranded-batch scenario leaves behind.
    struct Stranded {
        wal_dir: tempfile::TempDir,
        wal: Arc<crate::storage::wal::WalWriter>,
        memory: Arc<MemoryStore>,
        store: Arc<WriteBehindDataStore>,
    }

    /// A flush task dies holding a batch while newer writes are queued, and
    /// the store is then shut down, over a real WAL in a temporary directory.
    ///
    /// The pass is parked inside the inner store, holding `batch_only` and the
    /// older value of `in_both`. While it is parked the store is healthy, so
    /// the newer value of `in_both` and `drained_only` are accepted and queued
    /// — nothing takes them, the only flush task is inside its pass. Then the
    /// pass is released and panics. With `with_or_map`, the OR-Map key gets
    /// one tag on each side of the park, each as a delta frame.
    async fn stranded_shutdown(keys: &StrandedKeys, with_or_map: bool) -> Stranded {
        let wal_dir = tempfile::tempdir().expect("tempdir");
        let wal = crate::storage::wal::WalWriter::new(
            wal_dir.path().to_path_buf(),
            WalFsyncPolicy::PerOp,
        )
        .expect("WAL writer");
        let memory = Arc::new(MemoryStore::default());
        let (inner, mut about_to_panic) =
            PanickingAddStore::over(Arc::clone(&memory) as Arc<dyn MapDataStore>, 1);
        let mut pass_parked = inner.arm_park();
        let store = WriteBehindDataStore::new_with_wal(
            inner,
            WriteBehindConfig {
                or_delta_wal: true,
                ..manual_flush_config()
            },
            Some(bootstrap(Arc::clone(&wal) as Arc<dyn Wal>)),
        );
        let now = now_millis();

        store
            .add(MAP, &keys.batch_only, &batch_only_value(), 0, now)
            .await
            .expect("batch-only write");
        store
            .add(MAP, &keys.in_both, &older_value(), 0, now)
            .await
            .expect("older write of the key in both");
        if with_or_map {
            store
                .add_with_witness(
                    MAP,
                    &keys.or_map,
                    WriteSource::Value(&or_snapshot(&["tag-a"])),
                    0,
                    now,
                    Some(&OrDelta::Add {
                        entry: or_entry("tag-a"),
                    }),
                )
                .await
                .expect("first OR write");
        }
        store.soft_flush().await.expect("soft_flush");
        pass_parked.wait_parked().await;

        store
            .add(MAP, &keys.in_both, &newer_value(), 0, now)
            .await
            .expect("newer write of the key in both");
        store
            .add(MAP, &keys.drained_only, &drained_only_value(), 0, now)
            .await
            .expect("drained-only write");
        if with_or_map {
            store
                .add_with_witness(
                    MAP,
                    &keys.or_map,
                    WriteSource::Value(&or_snapshot(&["tag-a", "tag-b"])),
                    0,
                    now,
                    Some(&OrDelta::Add {
                        entry: or_entry("tag-b"),
                    }),
                )
                .await
                .expect("second OR write");
        }

        pass_parked.release();
        tokio::time::timeout(HANG_LIMIT, about_to_panic.recv())
            .await
            .expect("the released flush pass never panicked")
            .expect("the inner store was dropped before it panicked");

        // A limiter, not a wait: the dead batch keeps the pending count above
        // zero for good, so a shutdown that waited for zero would never return.
        // What it returns is asserted where the failed store is the subject.
        let _ = tokio::time::timeout(std::time::Duration::from_secs(10), store.hard_flush())
            .await
            .expect(
                "`hard_flush` did not complete on a store whose pending count stays above zero",
            );

        Stranded {
            wal_dir,
            wal,
            memory,
            store,
        }
    }

    #[test]
    fn the_dead_tasks_batch_stays_unresolved_through_shutdown() {
        let keys = StrandedKeys::chosen();
        let partition = keys.assert_one_partition();

        let ((pending, unapplied, memory), events) = run_captured(async {
            let stranded = stranded_shutdown(&keys, false).await;
            let unapplied = stranded
                .wal
                .unapplied(partition)
                .await
                .expect("unapplied frames");
            (
                stranded.store.pending_operation_count(),
                unapplied,
                stranded.memory,
            )
        });

        let lines = events_saying(&events, FAILED_SHUTDOWN_LINE);
        assert_eq!(lines.len(), 1, "captured events: {events:?}");
        assert_eq!(lines[0].level, tracing::Level::ERROR);
        assert_eq!(lines[0].field("task"), Some("flush"));
        assert_eq!(
            lines[0].field("unpersisted_entries"),
            Some("2"),
            "the drain failed on nothing here, so what is left un-persisted is the dead batch"
        );
        assert_eq!(lines[0].field("drained_entries"), Some("2"));
        assert!(
            pending > 0,
            "the shutdown completed although the dead batch's entries stay counted for good"
        );

        assert_eq!(
            memory.get(MAP, &keys.in_both),
            Some(newer_value()),
            "the drain persisted the newer value"
        );
        assert_eq!(
            memory.get(MAP, &keys.drained_only),
            Some(drained_only_value())
        );
        assert_eq!(
            memory.get(MAP, &keys.batch_only),
            None,
            "the dead batch never reached the inner store"
        );

        let frames = stored_frames(&unapplied);
        for (key, value, what) in [
            (&keys.batch_only, batch_only_value(), "the batch-only frame"),
            (
                &keys.in_both,
                older_value(),
                "the dead batch's frame of the key in both",
            ),
            (
                &keys.in_both,
                newer_value(),
                "the later frame of the key in both, whose value the drain persisted",
            ),
            (
                &keys.drained_only,
                drained_only_value(),
                "the drained-only frame, later than the dead batch's in the same partition",
            ),
        ] {
            assert!(
                frames.contains(&(key.clone(), value)),
                "{what} must still be un-applied in partition {partition}: the dead batch \
                 holds the watermark below every later frame; un-applied frames: {frames:?}"
            );
        }
    }

    #[tokio::test]
    async fn after_a_flush_task_panic_a_restart_holds_every_acked_write() {
        use crate::storage::wal::{WalRecovery, WalWriter};

        let keys = StrandedKeys::chosen();
        let partition = keys.assert_one_partition();

        let Stranded {
            wal_dir,
            wal,
            memory,
            store,
        } = stranded_shutdown(&keys, true).await;

        // The precondition the outcome rests on, asserted and not assumed: the
        // drained value's frame is still in the replay window, after the dead
        // batch's older frame of the same key.
        let unapplied = wal.unapplied(partition).await.expect("unapplied frames");
        let frames = stored_frames(&unapplied);
        let older_at = frames
            .iter()
            .position(|frame| *frame == (keys.in_both.clone(), older_value()));
        let newer_at = frames
            .iter()
            .position(|frame| *frame == (keys.in_both.clone(), newer_value()));
        assert!(
            matches!((older_at, newer_at), (Some(older), Some(newer)) if older < newer),
            "both frames of the key in both must be un-applied, the older first: {frames:?}"
        );
        assert_eq!(
            memory.get(MAP, &keys.batch_only),
            None,
            "before the start the batch-only write exists in the WAL and nowhere else"
        );

        // The simulated start: the old process is gone, a new writer opens
        // the same directory and recovery replays into the same inner store.
        drop(store);
        drop(wal);
        let reopened = WalWriter::new(wal_dir.path().to_path_buf(), WalFsyncPolicy::PerOp)
            .expect("reopened WAL writer");
        WalRecovery::new(Arc::clone(&reopened), Vec::new())
            .run(Arc::clone(&memory) as Arc<dyn MapDataStore>)
            .await
            .expect("recovery");

        assert_eq!(memory.get(MAP, &keys.batch_only), Some(batch_only_value()));
        assert_eq!(
            memory.get(MAP, &keys.drained_only),
            Some(drained_only_value())
        );
        assert_eq!(
            memory.get(MAP, &keys.in_both),
            Some(newer_value()),
            "replaying the dead batch's older frame must not bring the older value back"
        );
        match memory.get(MAP, &keys.or_map) {
            Some(RecordValue::OrMap {
                records,
                tombstones,
            }) => {
                let mut tags: Vec<&str> = records.iter().map(|entry| entry.tag.as_str()).collect();
                tags.sort_unstable();
                assert_eq!(
                    tags,
                    vec!["tag-a", "tag-b"],
                    "each acked tag must be held exactly once after the replay"
                );
                assert!(tombstones.is_empty(), "nothing was removed: {tombstones:?}");
            }
            other => panic!("expected the OR-Map record after the replay, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn hard_flush_syncs_the_wal_before_every_return() {
        fn assert_sync_is_the_last_call(calls: &[WalCall], case: &str) {
            assert_eq!(
                calls.last(),
                Some(&WalCall::Sync),
                "{case}: `sync` must be the last WAL call; calls were {calls:?}"
            );
            assert_eq!(
                calls.iter().filter(|call| **call == WalCall::Sync).count(),
                1,
                "{case}: one shutdown, one `sync`; calls were {calls:?}"
            );
        }
        // Nothing is flushed before the shutdown asks for it.
        fn buffered_config() -> WriteBehindConfig {
            WriteBehindConfig {
                write_delay_ms: 60_000,
                ..manual_flush_config()
            }
        }

        // The return taken when the queues are empty, reached with nothing
        // ever queued.
        {
            let wal = Arc::new(RecordingWal::default());
            let store = WriteBehindDataStore::new_with_wal(
                Arc::new(MemoryStore::default()),
                buffered_config(),
                Some(bootstrap(Arc::clone(&wal) as Arc<dyn Wal>)),
            );
            store.hard_flush().await.expect("a healthy shutdown");
            assert_eq!(wal.calls(), vec![WalCall::Sync], "nothing queued");
        }

        // The same return, reached after the drain persisted an entry.
        {
            let wal = Arc::new(RecordingWal::default());
            let store = WriteBehindDataStore::new_with_wal(
                Arc::new(MemoryStore::default()),
                buffered_config(),
                Some(bootstrap(Arc::clone(&wal) as Arc<dyn Wal>)),
            );
            store
                .add(MAP, "drained", &lww_value(), 0, now_millis())
                .await
                .expect("write");
            store.hard_flush().await.expect("a healthy shutdown");
            let calls = wal.calls();
            assert!(
                calls.contains(&WalCall::Append) && calls.contains(&WalCall::MarkApplied),
                "the case must append a frame and mark it applied; calls were {calls:?}"
            );
            assert_sync_is_the_last_call(&calls, "queues drained");
        }

        // The return taken when the drain gave up on an entry. The inner store
        // refuses it, which takes the same exit as an entry that ran out of
        // time, with no clock involved.
        {
            let wal = Arc::new(RecordingWal::default());
            let memory = Arc::new(MemoryStore::default());
            let store = WriteBehindDataStore::new_with_wal(
                Arc::clone(&memory) as Arc<dyn MapDataStore>,
                buffered_config(),
                Some(bootstrap(Arc::clone(&wal) as Arc<dyn Wal>)),
            );
            store
                .add(MAP, "given-up-on", &lww_value(), 0, now_millis())
                .await
                .expect("write");
            memory.refuse_adds.store(true, Ordering::SeqCst);
            // The limiter is for a shutdown that would wait for the entry it
            // gave up on: that entry stays counted as pending for good.
            tokio::time::timeout(HANG_LIMIT, store.hard_flush())
                .await
                .expect("`hard_flush` did not return after its drain gave up on an entry")
                .expect("a healthy store's shutdown returns Ok when its drain gives up");
            let calls = wal.calls();
            assert!(
                calls.contains(&WalCall::Append) && !calls.contains(&WalCall::MarkApplied),
                "the entry must be appended and never applied; calls were {calls:?}"
            );
            assert_sync_is_the_last_call(&calls, "drain gave up");
        }

        // The return of a failed store.
        {
            let wal = Arc::new(RecordingWal::default());
            let (store, _memory) =
                store_with_a_dead_flush_task(Some(Arc::clone(&wal) as Arc<dyn Wal>)).await;
            // What it returns is asserted where the failed store is the
            // subject; here only the order of the WAL calls is.
            let _ = store.hard_flush().await;
            let calls = wal.calls();
            assert!(calls.contains(&WalCall::Append), "calls were {calls:?}");
            assert_sync_is_the_last_call(&calls, "failed store");
        }
    }

    #[test]
    fn a_stall_watchdog_panic_is_recorded_and_shutdown_still_drains() {
        const QUEUED: [&str; 3] = ["tracked-1", "tracked-2", "tracked-3"];

        let ((failure, flush_task_alive, shutdown, persisted), events) = run_captured(async {
            let memory = Arc::new(MemoryStore::default());
            let wal: Arc<dyn Wal> = Arc::new(AcceptingWal);
            let store = WriteBehindDataStore::new_with_wal(
                Arc::clone(&memory) as Arc<dyn MapDataStore>,
                // The flush task stays out of the way for the whole test, and
                // the watchdog ticks at its floor.
                WriteBehindConfig {
                    write_delay_ms: 60_000,
                    flush_interval_ms: 60_000,
                    capacity: 0,
                    shutdown_timeout_ms: 5_000,
                    wal_watermark_stall_bound_ms: 1_000,
                    ..WriteBehindConfig::default()
                },
                Some(bootstrap(wal)),
            );
            let (about_to_panic, mut announced) = tokio::sync::mpsc::unbounded_channel();
            store.test_set_classifier_hook(Arc::new(move |_partition| {
                let _ = about_to_panic.send(());
                panic!("injected stall watchdog panic");
            }));

            for key in QUEUED {
                store
                    .add(MAP, key, &lww_text(key, 1), 0, now_millis())
                    .await
                    .expect("a healthy store accepts the write");
            }
            tokio::time::timeout(HANG_LIMIT, announced.recv())
                .await
                .expect("the stall watchdog never sampled a written partition")
                .expect("the classifier hook was dropped before it ran");

            let failure = store.background_failure().borrow().clone();
            let flush_task_alive = store
                .flush_loop
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .as_ref()
                .map(|task| !task.is_finished());
            // Twice the store's own shutdown timeout, so a flush task that
            // ignored the signal shows as the abort line asserted on below and
            // not as this limiter.
            let shutdown = tokio::time::timeout(HANG_LIMIT * 2, store.hard_flush())
                .await
                .expect("`hard_flush` did not complete after a watchdog panic");
            let persisted = QUEUED.map(|key| memory.get(MAP, key));
            (failure, flush_task_alive, shutdown, persisted)
        });

        assert_eq!(
            failure,
            Some(BackgroundFailure {
                task: BackgroundTask::StallWatchdog,
                message: "injected stall watchdog panic".to_string(),
                cause: FailureCause::Panicked,
            })
        );
        assert_eq!(
            flush_task_alive,
            Some(true),
            "the watchdog's panic must not take the flush task with it"
        );
        assert!(
            !events.iter().any(|event| event
                .field("message")
                .is_some_and(|message| message.starts_with(FLUSH_LOOP_ABORTED_LINE_START))),
            "the flush task must leave on the shutdown signal, not be aborted at the shutdown \
             timeout; captured events: {events:?}"
        );

        let error = shutdown.expect_err("a store whose watchdog panicked has failed");
        assert_eq!(
            error
                .downcast_ref::<BackgroundFailure>()
                .map(|failure| failure.task),
            Some(BackgroundTask::StallWatchdog)
        );
        assert_eq!(
            persisted,
            QUEUED.map(|key| Some(lww_text(key, 1))),
            "the shutdown of a failed store must drain everything that is queued"
        );

        let lines = events_saying(&events, FAILED_SHUTDOWN_LINE);
        assert_eq!(lines.len(), 1, "captured events: {events:?}");
        assert_eq!(lines[0].field("task"), Some("stall_watchdog"));
        assert_eq!(
            lines[0].field("unpersisted_entries"),
            Some("0"),
            "the watchdog held no batch and the drain failed on nothing"
        );
        assert_eq!(lines[0].field("drained_entries"), Some("3"));
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn a_flush_task_panic_after_shutdown_began_is_still_a_failure() {
        const PARKED_KEY: &str = "in-the-parked-batch";

        struct Observed {
            state_while_waiting: Lifecycle,
            flush_task_taken: bool,
            shutdown_pending: bool,
            failure_while_waiting: Option<BackgroundFailure>,
            shutdown: anyhow::Result<()>,
            failure: Option<BackgroundFailure>,
            unapplied: Vec<WalEntry>,
            write_after: anyhow::Result<()>,
        }

        let parked_value = lww_text("in-the-parked-batch", 1);
        let (observed, events) = run_captured(async {
            let wal_dir = tempfile::tempdir().expect("tempdir");
            let wal = crate::storage::wal::WalWriter::new(
                wal_dir.path().to_path_buf(),
                WalFsyncPolicy::PerOp,
            )
            .expect("WAL writer");
            let (inner, mut about_to_panic) = PanickingAddStore::new(1);
            let mut pass_parked = inner.arm_park();
            let store = WriteBehindDataStore::new_with_wal(
                inner,
                manual_flush_config(),
                Some(bootstrap(Arc::clone(&wal) as Arc<dyn Wal>)),
            );

            store
                .add(MAP, PARKED_KEY, &parked_value, 0, now_millis())
                .await
                .expect("a healthy store accepts the write");
            store.soft_flush().await.expect("soft_flush");
            pass_parked.wait_parked().await;

            // The store's own shutdown signal is what tells this test that
            // `hard_flush` is waiting for the flush task. `hard_flush` sends it
            // after it stored the shutting-down state and suspends next at the
            // join of the flush task, with no suspension point in between; on
            // one thread this task is not run again until that poll returned.
            let mut shutdown_signal = store.shutdown.subscribe();
            let shutdown_task = {
                let store = Arc::clone(&store);
                tokio::spawn(async move { store.hard_flush().await })
            };
            tokio::time::timeout(HANG_LIMIT, shutdown_signal.wait_for(|signalled| *signalled))
                .await
                .expect("`hard_flush` never signalled the shutdown")
                .expect("the shutdown signal's sender was dropped");

            let state_while_waiting = lifecycle(&store);
            // The join takes the handle out of its slot before it awaits it.
            let flush_task_taken = store
                .flush_loop
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_none();
            let shutdown_pending = !shutdown_task.is_finished();
            let failure_while_waiting = store.background_failure().borrow().clone();

            pass_parked.release();
            tokio::time::timeout(HANG_LIMIT, about_to_panic.recv())
                .await
                .expect("the released flush pass never panicked")
                .expect("the inner store was dropped before it panicked");

            let shutdown = tokio::time::timeout(HANG_LIMIT, shutdown_task)
                .await
                .expect("`hard_flush` never returned after the flush task died")
                .expect("the `hard_flush` task");
            let failure = store.background_failure().borrow().clone();
            let unapplied = wal
                .unapplied(partition_for(MAP, PARKED_KEY))
                .await
                .expect("unapplied frames");
            let write_after = store
                .add(MAP, "after-the-shutdown", &lww_value(), 0, now_millis())
                .await;

            Observed {
                state_while_waiting,
                flush_task_taken,
                shutdown_pending,
                failure_while_waiting,
                shutdown,
                failure,
                unapplied,
                write_after,
            }
        });

        // The setup: the panic landed while `hard_flush` waited for the task.
        assert_eq!(observed.state_while_waiting, Lifecycle::ShuttingDown);
        assert!(
            observed.flush_task_taken,
            "`hard_flush` must have begun to wait for the flush task"
        );
        assert!(observed.shutdown_pending);
        assert_eq!(observed.failure_while_waiting, None);

        let flush_failure = BackgroundFailure {
            task: BackgroundTask::Flush,
            message: "injected inner-store panic".to_string(),
            cause: FailureCause::Panicked,
        };
        let error = observed.shutdown.expect_err(
            "a flush task that panics during the shutdown leaves a failed store, and its \
             shutdown must say so",
        );
        assert_eq!(
            error.downcast_ref::<BackgroundFailure>(),
            Some(&flush_failure),
            "the failed store must be recognisable by type, not by the text of the error"
        );
        assert!(
            format!("{error:#}").contains("background task panicked"),
            "error was: {error:#}"
        );
        assert_eq!(observed.failure, Some(flush_failure));

        let lines = events_saying(&events, FAILED_SHUTDOWN_LINE);
        assert_eq!(lines.len(), 1, "captured events: {events:?}");
        assert_eq!(lines[0].field("task"), Some("flush"));
        assert_eq!(lines[0].field("unpersisted_entries"), Some("1"));
        assert_eq!(lines[0].field("drained_entries"), Some("0"));

        assert_eq!(
            stored_frames(&observed.unapplied),
            vec![(PARKED_KEY.to_string(), parked_value)],
            "the parked batch's frame must stay un-applied so the next start replays it"
        );
        assert_eq!(
            observed
                .write_after
                .expect_err("a failed store refuses writes")
                .to_string(),
            format!(
                "write-behind store has failed (background task panicked); write rejected for \
                 map={MAP} key=after-the-shutdown"
            ),
            "the store is failed, not merely shutting down, and must say which"
        );
    }

    #[tokio::test]
    async fn a_clean_stop_is_not_a_failure() {
        // `hard_flush` on a healthy WAL-backed store with a write queued.
        {
            let memory = Arc::new(MemoryStore::default());
            let store = WriteBehindDataStore::new_with_wal(
                Arc::clone(&memory) as Arc<dyn MapDataStore>,
                manual_flush_config(),
                Some(bootstrap(Arc::new(AcceptingWal))),
            );
            store
                .add(MAP, "queued", &lww_value(), 0, now_millis())
                .await
                .expect("write");
            store.hard_flush().await.expect("a healthy shutdown");
            assert_eq!(*store.background_failure().borrow(), None);
            assert_eq!(lifecycle(&store), Lifecycle::ShuttingDown);
            assert_eq!(memory.get(MAP, "queued"), Some(lww_value()));
        }

        // A store with no WAL: its watchdog returned at once, which is a
        // return and not a panic.
        {
            let store =
                WriteBehindDataStore::new(Arc::new(MemoryStore::default()), manual_flush_config());
            store
                .add(MAP, "queued", &lww_value(), 0, now_millis())
                .await
                .expect("write");
            store.hard_flush().await.expect("a healthy shutdown");
            assert_eq!(*store.background_failure().borrow(), None);
            assert_eq!(lifecycle(&store), Lifecycle::ShuttingDown);
        }

        // The flush task aborted at the shutdown timeout, parked inside a
        // pass. An abort drops the task; nothing unwinds.
        {
            let (inner, _about_to_panic) = PanickingAddStore::new(0);
            let mut pass_parked = inner.arm_park();
            let store = WriteBehindDataStore::new(
                inner,
                WriteBehindConfig {
                    shutdown_timeout_ms: 1_000,
                    ..manual_flush_config()
                },
            );
            store
                .add(MAP, "in-the-aborted-batch", &lww_value(), 0, now_millis())
                .await
                .expect("write");
            store.soft_flush().await.expect("soft_flush");
            pass_parked.wait_parked().await;

            // The clock is paused so the shutdown timeout elapses by the
            // runtime advancing it when every task is waiting, not by waiting
            // it out. The limiter is later than that timeout on the same
            // clock; it is for a shutdown that would wait for the aborted
            // batch, whose entry stays counted as pending for good.
            tokio::time::pause();
            tokio::time::timeout(HANG_LIMIT, store.hard_flush())
                .await
                .expect("`hard_flush` did not return after it aborted the flush task")
                .expect("a shutdown that aborted the flush task returns Ok, as it always did");
            tokio::time::resume();

            // The abort drops the task when the runtime next gets to it, which
            // is after `hard_flush` returned: the shutdown does not wait for a
            // task it aborted. The parked pass going away is what shows the
            // abort took place, and it closes the park's release channel.
            let release = pass_parked.release.as_mut().expect("not released");
            tokio::time::timeout(HANG_LIMIT, release.closed())
                .await
                .expect("the parked pass was never dropped: the flush task was not aborted");
            assert_eq!(
                *store.background_failure().borrow(),
                None,
                "an aborted flush task is a cancellation, not a failure"
            );
            assert_eq!(lifecycle(&store), Lifecycle::ShuttingDown);
        }
    }

    /// Poisons a lock of `store` the way a panic under it would: a thread takes
    /// the lock through `lock` and panics while it holds it.
    fn poison(
        store: &Arc<WriteBehindDataStore>,
        lock: impl FnOnce(&WriteBehindDataStore) + Send + 'static,
    ) {
        let store = Arc::clone(store);
        let poisoner = std::thread::spawn(move || lock(&store));
        assert!(
            poisoner.join().is_err(),
            "the poisoning thread must have panicked"
        );
    }

    #[tokio::test]
    async fn hard_flush_completes_with_a_poisoned_pending_seqs_lock() {
        let memory = Arc::new(MemoryStore::default());
        let store = WriteBehindDataStore::new(
            Arc::clone(&memory) as Arc<dyn MapDataStore>,
            WriteBehindConfig {
                write_delay_ms: 60_000,
                ..manual_flush_config()
            },
        );
        store
            .add(MAP, "queued", &lww_value(), 0, now_millis())
            .await
            .expect("write");

        poison(&store, |store| {
            let _guard = store.pending_seqs.lock().expect("not yet poisoned");
            panic!("injected panic under the pending_seqs lock");
        });
        assert!(store.pending_seqs.is_poisoned());

        // The drain resolves the entry's sequence under this lock.
        store
            .hard_flush()
            .await
            .expect("`hard_flush` over a poisoned pending_seqs lock");
        assert_eq!(memory.get(MAP, "queued"), Some(lww_value()));
    }

    #[tokio::test]
    async fn hard_flush_completes_with_a_poisoned_flush_loop_handle_lock() {
        let memory = Arc::new(MemoryStore::default());
        let store = WriteBehindDataStore::new(
            Arc::clone(&memory) as Arc<dyn MapDataStore>,
            WriteBehindConfig {
                write_delay_ms: 60_000,
                ..manual_flush_config()
            },
        );
        store
            .add(MAP, "queued", &lww_value(), 0, now_millis())
            .await
            .expect("write");

        poison(&store, |store| {
            let _guard = store.flush_loop.lock().expect("not yet poisoned");
            panic!("injected panic under the flush_loop handle lock");
        });
        assert!(store.flush_loop.is_poisoned());

        // The shutdown takes the flush task's handle out from under this lock.
        store
            .hard_flush()
            .await
            .expect("`hard_flush` over a poisoned flush_loop handle lock");
        assert!(
            store
                .flush_loop
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_none(),
            "the shutdown must still have taken and joined the flush task"
        );
        assert_eq!(memory.get(MAP, "queued"), Some(lww_value()));
    }

    #[tokio::test]
    async fn hard_flush_completes_with_a_poisoned_wal_tracking_cell_lock() {
        let memory = Arc::new(MemoryStore::default());
        let wal = Arc::new(RecordingWal::default());
        let store = WriteBehindDataStore::new_with_wal(
            Arc::clone(&memory) as Arc<dyn MapDataStore>,
            WriteBehindConfig {
                write_delay_ms: 60_000,
                ..manual_flush_config()
            },
            Some(bootstrap(Arc::clone(&wal) as Arc<dyn Wal>)),
        );
        store
            .add(MAP, "queued", &lww_value(), 0, now_millis())
            .await
            .expect("write");

        let cell = store
            .wal
            .as_ref()
            .expect("a WAL-backed store")
            .cell(partition_for(MAP, "queued"));
        poison(&store, {
            let cell = Arc::clone(&cell);
            move |_store| {
                let _guard = cell.lock().expect("not yet poisoned");
                panic!("injected panic under a partition's WAL tracking lock");
            }
        });
        assert!(cell.is_poisoned());

        // The drain registers, resolves and advances the entry's WAL sequence
        // under this lock.
        store
            .hard_flush()
            .await
            .expect("`hard_flush` over a poisoned WAL tracking lock");
        assert_eq!(memory.get(MAP, "queued"), Some(lww_value()));
        assert!(
            wal.calls().contains(&WalCall::MarkApplied),
            "the drained entry's frame must still have been marked applied"
        );
    }

    #[tokio::test]
    async fn the_capacity_pre_check_bound_is_the_stated_one() {
        const CAPACITY: u64 = 3;
        /// The keys whose entries the parked flush pass holds.
        const HELD: [&str; 2] = ["held-1", "held-2"];
        /// Queued but too young for the pass, so it stays in its queue.
        const QUEUED: &str = "queued";

        async fn assert_a_new_key_is_rejected(store: &WriteBehindDataStore, when: &str) {
            let pending = store.pending_operation_count();
            let rejected = store
                .add(MAP, "in-neither", &lww_value(), 0, now_millis())
                .await
                .expect_err("a key in neither staging nor a flush pass must be rejected");
            assert_eq!(
                rejected.to_string(),
                "Write-behind capacity exceeded",
                "{when}"
            );
            assert_eq!(
                store.pending_operation_count(),
                pending,
                "{when}: a rejected write must not be counted"
            );
        }

        let (inner, _about_to_panic) = PanickingAddStore::new(0);
        let mut pass_parked = inner.arm_park();
        let store = WriteBehindDataStore::new(
            inner,
            WriteBehindConfig {
                capacity: CAPACITY,
                ..manual_flush_config()
            },
        );
        let now = now_millis();
        let held = u64::try_from(HELD.len()).expect("a small count");

        for key in HELD {
            store
                .add(MAP, key, &lww_value(), 0, now)
                .await
                .expect("below capacity");
        }
        // Stored an hour from now, so no pass finds it old enough to take.
        store
            .add(MAP, QUEUED, &lww_value(), 0, now + 3_600_000)
            .await
            .expect("below capacity");
        assert_eq!(store.pending_operation_count(), CAPACITY);
        assert_a_new_key_is_rejected(&store, "at capacity, before any pass").await;

        store.soft_flush().await.expect("soft_flush");
        pass_parked.wait_parked().await;
        assert_eq!(
            store.pending_operation_count(),
            CAPACITY,
            "the entries a pass holds stay counted until they are persisted"
        );
        assert_a_new_key_is_rejected(&store, "a pass holds entries").await;

        // The held keys are still staged, so the pre-check takes each re-write
        // for a coalesce; their queue entries are gone, so each is counted anew.
        let mut rewritten = 0;
        for key in HELD {
            store
                .add(MAP, key, &lww_value(), 0, now)
                .await
                .expect("a re-write of a key the flush pass holds is accepted over capacity");
            rewritten += 1;
            assert_eq!(store.pending_operation_count(), CAPACITY + rewritten);
            assert_a_new_key_is_rejected(&store, "over capacity, while the pass is parked").await;
        }
        assert_eq!(store.pending_operation_count(), CAPACITY + held);

        // A second round finds every key queued and coalesces: the excess does
        // not grow past the number of entries the pass holds.
        for key in HELD.into_iter().chain([QUEUED]) {
            store
                .add(MAP, key, &lww_value(), 0, now)
                .await
                .expect("a re-write of a queued key coalesces");
        }
        assert_eq!(
            store.pending_operation_count(),
            CAPACITY + held,
            "the count must not exceed capacity by more than the pass holds"
        );
        assert_a_new_key_is_rejected(&store, "after the second round").await;

        pass_parked.release();
        tokio::time::timeout(HANG_LIMIT, store.hard_flush())
            .await
            .expect("`hard_flush` did not return")
            .expect("a healthy shutdown");
    }

    /// A flush pass takes every entry that is due when it begins, in every
    /// partition; `batch_size` only groups the iteration and limits nothing.
    #[tokio::test]
    async fn a_flush_pass_holds_every_due_entry_whatever_the_batch_size() {
        // Two keys in one partition and a third in another: a pass cut short
        // per queue would leave one of the pair behind, and a pass cut short
        // as a whole would leave two of the three.
        let first = "due-0".to_string();
        let shared = partition_for(MAP, &first);
        let mut same_partition = None;
        let mut other_partition = None;
        for i in 1..100_000 {
            let key = format!("due-{i}");
            if partition_for(MAP, &key) == shared {
                same_partition.get_or_insert(key);
            } else {
                other_partition.get_or_insert(key);
            }
            if same_partition.is_some() && other_partition.is_some() {
                break;
            }
        }
        let keys = [
            first,
            same_partition.expect("a second key in the first key's partition"),
            other_partition.expect("a key in another partition"),
        ];
        let partitions: Vec<u32> = keys.iter().map(|key| partition_for(MAP, key)).collect();
        assert_eq!(
            partitions[0], partitions[1],
            "two due keys share a partition"
        );
        assert_ne!(
            partitions[0], partitions[2],
            "the due keys span two partitions"
        );
        let due = u64::try_from(keys.len()).expect("a small count");

        let (inner, _about_to_panic) = PanickingAddStore::new(0);
        let mut pass_parked = inner.arm_park();
        let store = WriteBehindDataStore::new(
            inner,
            WriteBehindConfig {
                batch_size: 1,
                ..manual_flush_config()
            },
        );
        let now = now_millis();
        for key in &keys {
            store
                .add(MAP, key, &lww_value(), 0, now)
                .await
                .expect("write");
        }
        assert_eq!(store.pending_operation_count(), due);

        store.soft_flush().await.expect("soft_flush");
        pass_parked.wait_parked().await;
        assert!(
            store.queues.iter().all(|queue| queue.entries.is_empty()),
            "the pass must have taken every due entry out of every queue, not one batch"
        );
        assert_eq!(
            store.pending_operation_count(),
            due,
            "the pass holds all of them, and held entries stay counted"
        );

        pass_parked.release();
        let deadline = std::time::Instant::now() + HANG_LIMIT;
        while !store.test_is_idle() {
            assert!(
                std::time::Instant::now() < deadline,
                "the pass never closed after its park was released"
            );
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
        assert_eq!(store.pending_operation_count(), 0);
    }

    /// Never due and never requested: nothing leaves the buffer during a test.
    fn never_due_config(capacity: u64) -> WriteBehindConfig {
        WriteBehindConfig {
            write_delay_ms: 600_000,
            flush_interval_ms: 600_000,
            capacity,
            shutdown_timeout_ms: 5_000,
            ..WriteBehindConfig::default()
        }
    }

    #[tokio::test]
    async fn a_remove_of_a_key_that_is_not_buffered_is_refused_at_capacity() {
        const CAPACITY: u64 = 3;
        let store = WriteBehindDataStore::new(Arc::new(NullDataStore), never_due_config(CAPACITY));
        let now = now_millis();

        for i in 0..CAPACITY - 1 {
            store
                .add(MAP, &format!("filled-{i}"), &lww_value(), 0, now)
                .await
                .expect("below capacity");
        }
        assert_eq!(store.pending_operation_count(), CAPACITY - 1);

        store
            .remove(MAP, "not-buffered-1", now)
            .await
            .expect("a remove checked one below capacity is accepted");
        assert_eq!(store.pending_operation_count(), CAPACITY);

        let refused = store
            .remove(MAP, "not-buffered-2", now)
            .await
            .expect_err("a remove of a key that is not buffered is refused at capacity");
        assert_eq!(refused.to_string(), "Write-behind capacity exceeded");
        assert_eq!(
            store.pending_operation_count(),
            CAPACITY,
            "a refused remove must not be counted"
        );

        store
            .remove(MAP, "filled-0", now)
            .await
            .expect("a remove of a buffered key is accepted at capacity");
        assert_eq!(
            store.pending_operation_count(),
            CAPACITY,
            "a remove of a buffered key coalesces and is not counted again"
        );
    }

    #[tokio::test]
    async fn a_remove_all_of_a_key_that_is_not_buffered_is_refused_at_capacity() {
        const CAPACITY: u64 = 3;
        let store = WriteBehindDataStore::new(Arc::new(NullDataStore), never_due_config(CAPACITY));
        let now = now_millis();

        for i in 0..CAPACITY - 1 {
            store
                .add(MAP, &format!("filled-{i}"), &lww_value(), 0, now)
                .await
                .expect("below capacity");
        }
        assert_eq!(store.pending_operation_count(), CAPACITY - 1);

        store
            .remove_all(MAP, &["not-buffered-1".to_string()])
            .await
            .expect("a remove_all checked one below capacity is accepted");
        assert_eq!(store.pending_operation_count(), CAPACITY);

        let refused = store
            .remove_all(MAP, &["not-buffered-2".to_string()])
            .await
            .expect_err("a remove_all of a key that is not buffered is refused at capacity");
        assert_eq!(refused.to_string(), "Write-behind capacity exceeded");
        assert_eq!(
            store.pending_operation_count(),
            CAPACITY,
            "a refused remove_all must not be counted"
        );

        store
            .remove_all(MAP, &["filled-0".to_string()])
            .await
            .expect("a remove_all of a buffered key is accepted at capacity");
        assert_eq!(
            store.pending_operation_count(),
            CAPACITY,
            "a remove_all of a buffered key coalesces and is not counted again"
        );
    }

    /// The key whose write is raced against a flush pass or a shutdown drain.
    const RACED_KEY: &str = "raced";
    /// Far enough ahead that no requested pass finds the entry old enough.
    const NOT_DUE_FOR_MS: i64 = 3_600_000;

    /// Asks for one flush pass and returns once it has closed. The epoch moves
    /// by two through a pass whether or not the pass found anything, so the
    /// wait does not depend on what was queued.
    async fn run_one_whole_pass(store: &WriteBehindDataStore) {
        let opened_at = store.flush_pass_epoch.load(Ordering::SeqCst);
        assert_eq!(opened_at % 2, 0, "a pass was already open");
        store.soft_flush().await.expect("soft_flush");
        tokio::time::timeout(HANG_LIMIT, async {
            while store.flush_pass_epoch.load(Ordering::SeqCst) < opened_at + 2 {
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("the requested flush pass never closed");
    }

    /// Returns once nothing is queued and no pass is open.
    async fn wait_until_idle(store: &WriteBehindDataStore) {
        tokio::time::timeout(HANG_LIMIT, async {
            while !store.test_is_idle() {
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("the store never came to rest");
    }

    /// The sequence of the entry queued for `key`, if one is.
    fn queued_sequence(store: &WriteBehindDataStore, key: &str) -> Option<u64> {
        let slot_key = (MAP.to_string(), key.to_string());
        store
            .queues
            .get(&partition_for(MAP, key))
            .and_then(|queue| queue.entries.get(&slot_key).map(|entry| entry.sequence))
    }

    /// The sequence of the staging slot of `key`, if it has one.
    fn staged_sequence(store: &WriteBehindDataStore, key: &str) -> Option<u64> {
        store
            .staging
            .get(&(MAP.to_string(), key.to_string()))
            .map(|slot| slot.seq)
    }

    /// What a write was answered: its error text, or `accepted`.
    fn answer_of(result: anyhow::Result<()>) -> String {
        result.map_or_else(|refused| refused.to_string(), |()| "accepted".to_string())
    }

    /// Waits for a spawned write and requires that it was accepted.
    async fn accepted(write: tokio::task::JoinHandle<anyhow::Result<()>>) {
        tokio::time::timeout(HANG_LIMIT, write)
            .await
            .expect("the released write never returned")
            .expect("the write's task")
            .expect("the write is accepted");
    }

    /// Shuts `store` down from a running state and requires a clean return.
    async fn shut_down(store: &WriteBehindDataStore) {
        tokio::time::timeout(HANG_LIMIT, store.hard_flush())
            .await
            .expect("hard_flush never returned")
            .expect("hard_flush");
    }

    fn seed(memory: &MemoryStore, key: &str) {
        memory
            .values()
            .insert((MAP.to_string(), key.to_string()), lww_text("seeded", 1));
    }

    #[tokio::test]
    async fn a_pass_between_the_two_steps_of_a_new_key_write_leaves_no_slot_without_an_entry() {
        let memory = Arc::new(MemoryStore::default());
        let store = WriteBehindDataStore::new(
            Arc::clone(&memory) as Arc<dyn MapDataStore>,
            WriteBehindConfig {
                capacity: 1,
                ..manual_flush_config()
            },
        );
        let now = now_millis();
        let written = lww_text("written", 1);

        let mut park = store.test_park_between_stage_and_queue_insert(MAP, RACED_KEY);
        let write = {
            let store = Arc::clone(&store);
            let value = written.clone();
            tokio::spawn(async move { store.add(MAP, RACED_KEY, &value, 0, now).await })
        };
        park.wait_parked().await;

        run_one_whole_pass(&store).await;
        park.release();
        accepted(write).await;
        run_one_whole_pass(&store).await;
        wait_until_idle(&store).await;

        // Read before the filler below, which stages a key of its own.
        let queued = queued_sequence(&store, RACED_KEY);
        let staged = staged_sequence(&store, RACED_KEY);
        let slots_at_rest = store.staging.len();
        let persisted = memory.get(MAP, RACED_KEY);

        store
            .add(MAP, "filler", &lww_value(), 0, now + NOT_DUE_FOR_MS)
            .await
            .expect("the filler is accepted below capacity");
        let answer = answer_of(
            store
                .add(MAP, RACED_KEY, &written, 0, now + NOT_DUE_FOR_MS)
                .await,
        );
        let pending = store.pending_operation_count();

        assert_eq!(
            (queued, staged, slots_at_rest, persisted, answer, pending),
            (
                None,
                None,
                0,
                Some(written),
                "Write-behind capacity exceeded".to_string(),
                1
            ),
            "(queued, staged, slots at rest, persisted, answer at capacity, pending)"
        );
    }

    #[tokio::test]
    async fn a_pass_between_the_two_steps_of_a_coalescing_write_leaves_no_slot_without_an_entry() {
        let memory = Arc::new(MemoryStore::default());
        let store = WriteBehindDataStore::new(
            Arc::clone(&memory) as Arc<dyn MapDataStore>,
            manual_flush_config(),
        );
        let now = now_millis();
        let first = lww_text("first", 1);
        let second = lww_text("second", 2);

        store
            .add(MAP, RACED_KEY, &first, 0, now)
            .await
            .expect("the first write is accepted");

        // Its own time is an hour ahead: if its entry is due at once, that is
        // only because it took over the first entry's time.
        let mut park = store.test_park_between_stage_and_queue_insert(MAP, RACED_KEY);
        let write = {
            let store = Arc::clone(&store);
            let value = second.clone();
            tokio::spawn(async move {
                store
                    .add(MAP, RACED_KEY, &value, 0, now + NOT_DUE_FOR_MS)
                    .await
            })
        };
        park.wait_parked().await;

        run_one_whole_pass(&store).await;
        park.release();
        accepted(write).await;

        let after_the_write = (
            queued_sequence(&store, RACED_KEY),
            staged_sequence(&store, RACED_KEY),
            store.pending_operation_count(),
            store.load(MAP, RACED_KEY).await.expect("load"),
        );

        shut_down(&store).await;
        let after_shutdown = (
            queued_sequence(&store, RACED_KEY),
            staged_sequence(&store, RACED_KEY),
            store.staging.len(),
            store.pending_operation_count(),
            memory.get(MAP, RACED_KEY),
        );

        assert_eq!(
            (after_the_write, after_shutdown),
            (
                (Some(1), Some(1), 1, Some(second.clone())),
                (None, None, 0, 0, Some(second))
            ),
            "((queued, staged, pending, load), (queued, staged, slots at rest, pending, persisted))"
        );
    }

    #[tokio::test]
    async fn a_pass_between_the_two_steps_of_a_remove_leaves_no_slot_without_an_entry() {
        let memory = Arc::new(MemoryStore::default());
        seed(&memory, RACED_KEY);
        let store = WriteBehindDataStore::new(
            Arc::clone(&memory) as Arc<dyn MapDataStore>,
            WriteBehindConfig {
                capacity: 1,
                ..manual_flush_config()
            },
        );
        let now = now_millis();

        let mut park = store.test_park_between_stage_and_queue_insert(MAP, RACED_KEY);
        let write = {
            let store = Arc::clone(&store);
            tokio::spawn(async move { store.remove(MAP, RACED_KEY, now).await })
        };
        park.wait_parked().await;

        run_one_whole_pass(&store).await;
        park.release();
        accepted(write).await;
        run_one_whole_pass(&store).await;
        wait_until_idle(&store).await;

        let queued = queued_sequence(&store, RACED_KEY);
        let staged = staged_sequence(&store, RACED_KEY);
        let slots_at_rest = store.staging.len();
        let persisted = memory.get(MAP, RACED_KEY);

        store
            .add(MAP, "filler", &lww_value(), 0, now + NOT_DUE_FOR_MS)
            .await
            .expect("the filler is accepted below capacity");
        let answer = answer_of(store.remove(MAP, RACED_KEY, now + NOT_DUE_FOR_MS).await);
        let pending = store.pending_operation_count();

        assert_eq!(
            (queued, staged, slots_at_rest, persisted, answer, pending),
            (
                None,
                None,
                0,
                None,
                "Write-behind capacity exceeded".to_string(),
                1
            ),
            "(queued, staged, slots at rest, persisted, answer at capacity, pending)"
        );
    }

    #[tokio::test]
    async fn a_pass_between_the_two_steps_of_a_remove_all_leaves_no_slot_without_an_entry() {
        /// Goes through the loop ahead of the raced key, without parking.
        const UNPARKED_KEY: &str = "ahead-of-the-raced-key";

        let memory = Arc::new(MemoryStore::default());
        seed(&memory, UNPARKED_KEY);
        seed(&memory, RACED_KEY);
        let store = WriteBehindDataStore::new(
            Arc::clone(&memory) as Arc<dyn MapDataStore>,
            WriteBehindConfig {
                capacity: 2,
                ..manual_flush_config()
            },
        );
        let now = now_millis();

        let mut park = store.test_park_between_stage_and_queue_insert(MAP, RACED_KEY);
        let write = {
            let store = Arc::clone(&store);
            let keys = [UNPARKED_KEY.to_string(), RACED_KEY.to_string()];
            tokio::spawn(async move { store.remove_all(MAP, &keys).await })
        };
        park.wait_parked().await;

        run_one_whole_pass(&store).await;
        park.release();
        accepted(write).await;
        run_one_whole_pass(&store).await;
        wait_until_idle(&store).await;

        let queued = queued_sequence(&store, RACED_KEY);
        let staged = staged_sequence(&store, RACED_KEY);
        let staged_unparked = staged_sequence(&store, UNPARKED_KEY);
        let slots_at_rest = store.staging.len();
        let persisted = (memory.get(MAP, UNPARKED_KEY), memory.get(MAP, RACED_KEY));

        for filler in ["filler-1", "filler-2"] {
            store
                .add(MAP, filler, &lww_value(), 0, now + NOT_DUE_FOR_MS)
                .await
                .expect("the filler is accepted below capacity");
        }
        let answer = answer_of(store.remove_all(MAP, &[RACED_KEY.to_string()]).await);
        let pending = store.pending_operation_count();

        assert_eq!(
            (
                queued,
                staged,
                staged_unparked,
                slots_at_rest,
                persisted,
                answer,
                pending
            ),
            (
                None,
                None,
                None,
                0,
                (None, None),
                "Write-behind capacity exceeded".to_string(),
                2
            ),
            "(queued, staged, staged of the unparked key, slots at rest, persisted, answer at \
             capacity, pending)"
        );
    }

    #[tokio::test]
    async fn a_shutdown_drain_between_the_two_steps_of_a_write_keeps_slot_and_entry_together() {
        let memory = Arc::new(MemoryStore::default());
        let store = WriteBehindDataStore::new(
            Arc::clone(&memory) as Arc<dyn MapDataStore>,
            manual_flush_config(),
        );
        let later = now_millis() + NOT_DUE_FOR_MS;
        let first = lww_text("first", 1);
        let second = lww_text("second", 2);

        store
            .add(MAP, RACED_KEY, &first, 0, later)
            .await
            .expect("the first write is accepted");

        // Parked past the lifecycle check, so the shutdown below does not
        // refuse it.
        let mut park = store.test_park_between_stage_and_queue_insert(MAP, RACED_KEY);
        let write = {
            let store = Arc::clone(&store);
            let value = second.clone();
            tokio::spawn(async move { store.add(MAP, RACED_KEY, &value, 0, later).await })
        };
        park.wait_parked().await;

        shut_down(&store).await;
        park.release();
        accepted(write).await;

        assert_eq!(
            (
                queued_sequence(&store, RACED_KEY),
                staged_sequence(&store, RACED_KEY),
                store.load(MAP, RACED_KEY).await.expect("load"),
            ),
            (Some(1), Some(1), Some(second)),
            "(queued, staged, load)"
        );
    }

    #[tokio::test]
    async fn a_write_completed_while_the_drain_persists_an_older_entry_of_its_key_keeps_its_slot() {
        let memory = Arc::new(MemoryStore::default());
        let (inner, _about_to_panic) =
            PanickingAddStore::over(Arc::clone(&memory) as Arc<dyn MapDataStore>, 0);
        let store = WriteBehindDataStore::new(
            Arc::clone(&inner) as Arc<dyn MapDataStore>,
            manual_flush_config(),
        );
        let later = now_millis() + NOT_DUE_FOR_MS;
        let first = lww_text("first", 1);
        let second = lww_text("second", 2);

        store
            .add(MAP, RACED_KEY, &first, 0, later)
            .await
            .expect("the first write is accepted");

        // Past the lifecycle check, with neither of its two steps done.
        let mut write_park = store.test_park_before_queue_insert(MAP, RACED_KEY);
        let write = {
            let store = Arc::clone(&store);
            let value = second.clone();
            tokio::spawn(async move { store.add(MAP, RACED_KEY, &value, 0, later).await })
        };
        write_park.wait_parked().await;

        // The drain takes the first entry and stops inside its store call.
        let mut store_call_for_first = inner.arm_park();
        let shutdown = {
            let store = Arc::clone(&store);
            tokio::spawn(async move { store.hard_flush().await })
        };
        store_call_for_first.wait_parked().await;

        let mut store_call_for_second = inner.arm_park();
        write_park.release();
        accepted(write).await;
        let drain_inside_first_call = (
            queued_sequence(&store, RACED_KEY),
            staged_sequence(&store, RACED_KEY),
            store.load(MAP, RACED_KEY).await.expect("load"),
        );

        // The drain persists the first entry, clears for it, takes the second
        // entry in its next round and stops inside the store call for it.
        store_call_for_first.release();
        store_call_for_second.wait_parked().await;
        let drain_inside_second_call = (
            queued_sequence(&store, RACED_KEY),
            staged_sequence(&store, RACED_KEY),
            store.pending_operation_count(),
            store.load(MAP, RACED_KEY).await.expect("load"),
        );

        store_call_for_second.release();
        tokio::time::timeout(HANG_LIMIT, shutdown)
            .await
            .expect("hard_flush never returned")
            .expect("the shutdown's task")
            .expect("hard_flush");
        let after_shutdown = (
            queued_sequence(&store, RACED_KEY),
            staged_sequence(&store, RACED_KEY),
            store.staging.len(),
            memory.get(MAP, RACED_KEY),
        );

        assert_eq!(
            (
                drain_inside_first_call,
                drain_inside_second_call,
                after_shutdown
            ),
            (
                (Some(1), Some(1), Some(second.clone())),
                (None, Some(1), 1, Some(second.clone())),
                (None, None, 0, Some(second))
            ),
            "((queued, staged, load), (queued, staged, pending, load), (queued, staged, slots at \
             rest, persisted))"
        );
    }
}

#[cfg(test)]
mod store_broken_tests {
    use super::*;

    use crate::storage::datastores::NullDataStore;

    const MAP: &str = "store_broken_map";
    /// Upper bound on any single wait; reaching it is a failed test, not a wait.
    const HANG_LIMIT: std::time::Duration = std::time::Duration::from_secs(5);
    const KEYS: [&str; 3] = ["k1", "k2", "k3"];
    const BROKEN_LINE: &str = "write-behind store: the durable store is broken and stores nothing \
         more in this process; writes are refused from now on and the node stops";
    const DRAIN_LINE: &str = "write-behind shutdown drain stopped at a broken store: the entries \
         it did not store stay un-resolved and replay from the WAL at the next start";

    fn value(text: &str) -> RecordValue {
        RecordValue::Lww {
            value: topgun_core::types::Value::String(text.to_string()),
            timestamp: topgun_core::hlc::Timestamp {
                millis: 0,
                counter: 0,
                node_id: String::new(),
            },
        }
    }

    fn broken() -> StoreErrorClass {
        StoreErrorClass::StoreBroken(StoreBrokenCause::PreviousIo)
    }

    /// An inner store that refuses every write and classifies its error as
    /// `class`. It counts every call it gets, reads included, and remembers
    /// the key of every refused write in call order.
    struct RefusingStore {
        class: StoreErrorClass,
        calls: AtomicU64,
        refused_keys: Mutex<Vec<String>>,
        empty: NullDataStore,
    }

    impl RefusingStore {
        fn new(class: StoreErrorClass) -> Arc<Self> {
            Arc::new(Self {
                class,
                calls: AtomicU64::new(0),
                refused_keys: Mutex::new(Vec::new()),
                empty: NullDataStore,
            })
        }

        fn calls(&self) -> u64 {
            self.calls.load(Ordering::SeqCst)
        }

        fn refused_keys(&self) -> Vec<String> {
            self.refused_keys
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
        }

        fn refuse(&self, key: &str) -> anyhow::Result<()> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.refused_keys
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(key.to_string());
            Err(anyhow::anyhow!("the test store refuses the write"))
        }
    }

    #[async_trait]
    impl MapDataStore for RefusingStore {
        async fn add(
            &self,
            _map: &str,
            key: &str,
            _value: &RecordValue,
            _expiration_time: i64,
            _now: i64,
        ) -> anyhow::Result<()> {
            self.refuse(key)
        }

        async fn add_backup(
            &self,
            _map: &str,
            key: &str,
            _value: &RecordValue,
            _expiration_time: i64,
            _now: i64,
        ) -> anyhow::Result<()> {
            self.refuse(key)
        }

        async fn remove(&self, _map: &str, key: &str, _now: i64) -> anyhow::Result<()> {
            self.refuse(key)
        }

        async fn remove_backup(&self, _map: &str, key: &str, _now: i64) -> anyhow::Result<()> {
            self.refuse(key)
        }

        async fn load(&self, map: &str, key: &str) -> anyhow::Result<Option<RecordValue>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.empty.load(map, key).await
        }

        async fn load_all(
            &self,
            map: &str,
            keys: &[String],
        ) -> anyhow::Result<Vec<(String, RecordValue)>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.empty.load_all(map, keys).await
        }

        async fn enumerate_leaves(
            &self,
            map: &str,
            is_backup: bool,
            sink: &mut dyn LeafSink,
        ) -> anyhow::Result<()> {
            self.empty.enumerate_leaves(map, is_backup, sink).await
        }

        async fn scan_values(
            &self,
            map: &str,
            is_backup: bool,
            max_records: u64,
        ) -> anyhow::Result<ScanBatch> {
            self.empty.scan_values(map, is_backup, max_records).await
        }

        async fn scan_values_batched(
            &self,
            map: &str,
            is_backup: bool,
            cursor: ScanCursor,
            max_records: u64,
        ) -> anyhow::Result<ScanBatch> {
            self.empty
                .scan_values_batched(map, is_backup, cursor, max_records)
                .await
        }

        async fn remove_all(&self, _map: &str, keys: &[String]) -> anyhow::Result<()> {
            self.refuse(&keys.join(","))
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

        fn classify_error(&self, _err: &anyhow::Error) -> StoreErrorClass {
            self.class
        }
    }

    /// A WAL that keeps every frame and the applied watermark of each
    /// partition, so a test can tell an abandoned frame from a resolved one.
    #[derive(Default)]
    struct FrameWal {
        frames: Mutex<Vec<(u32, WalEntry)>>,
        applied: Mutex<Vec<(u32, u64)>>,
    }

    impl FrameWal {
        fn appends(&self) -> usize {
            self.frames
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .len()
        }

        fn mark_applied_calls(&self) -> usize {
            self.applied
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .len()
        }

        async fn unapplied_frames(&self) -> usize {
            let partitions: BTreeSet<u32> = self
                .frames
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .iter()
                .map(|(partition, _)| *partition)
                .collect();
            let mut total = 0;
            for partition in partitions {
                total += self
                    .unapplied(partition)
                    .await
                    .expect("the test WAL never fails")
                    .len();
            }
            total
        }
    }

    #[async_trait]
    impl Wal for FrameWal {
        async fn append(&self, partition: u32, entry: &WalEntry) -> anyhow::Result<()> {
            self.frames
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push((partition, entry.clone()));
            Ok(())
        }

        async fn mark_applied(&self, partition: u32, sequence: u64) -> anyhow::Result<()> {
            self.applied
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push((partition, sequence));
            Ok(())
        }

        async fn unapplied(&self, partition: u32) -> anyhow::Result<Vec<WalEntry>> {
            let watermark = self
                .applied
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .iter()
                .filter(|(applied_partition, _)| *applied_partition == partition)
                .map(|(_, sequence)| *sequence)
                .max()
                .unwrap_or(0);
            Ok(self
                .frames
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .iter()
                .filter(|(frame_partition, entry)| {
                    *frame_partition == partition && entry.sequence > watermark
                })
                .map(|(_, entry)| entry.clone())
                .collect())
        }
    }

    /// A flush task that runs a pass only when a test asks for one, and a
    /// backoff long enough that no refused entry and no hold comes due again
    /// inside a test.
    fn manual_flush_config() -> WriteBehindConfig {
        WriteBehindConfig {
            write_delay_ms: 0,
            flush_interval_ms: 3_600_000,
            capacity: 0,
            shutdown_timeout_ms: 5_000,
            backoff_base_ms: 60_000,
            backoff_cap_ms: 60_000,
            ..WriteBehindConfig::default()
        }
    }

    fn store_over(inner: &Arc<RefusingStore>) -> Arc<WriteBehindDataStore> {
        WriteBehindDataStore::new(
            Arc::clone(inner) as Arc<dyn MapDataStore>,
            manual_flush_config(),
        )
    }

    fn store_with_wal(
        inner: &Arc<RefusingStore>,
        wal: &Arc<FrameWal>,
    ) -> Arc<WriteBehindDataStore> {
        WriteBehindDataStore::new_with_wal(
            Arc::clone(inner) as Arc<dyn MapDataStore>,
            manual_flush_config(),
            Some(WalBootstrap {
                wal: Arc::clone(wal) as Arc<dyn Wal>,
                sequence_start: 1,
            }),
        )
    }

    async fn add_keys(store: &WriteBehindDataStore) {
        for key in KEYS {
            store
                .add(MAP, key, &value(key), 0, now_millis())
                .await
                .expect("a healthy store accepts the write");
        }
    }

    /// Asks the flush task for one pass and returns once that pass has closed.
    async fn one_pass(store: &WriteBehindDataStore) {
        let opened = store.flush_pass_epoch.load(Ordering::SeqCst);
        store.soft_flush().await.expect("soft_flush");
        tokio::time::timeout(HANG_LIMIT, async {
            while store.flush_pass_epoch.load(Ordering::SeqCst) < opened + 2 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("no flush pass closed after soft_flush");
    }

    async fn bounded_hard_flush(store: &WriteBehindDataStore) -> anyhow::Result<()> {
        tokio::time::timeout(HANG_LIMIT, store.hard_flush())
            .await
            .expect("hard_flush did not return")
    }

    fn failed_on_a_broken_store(store: &WriteBehindDataStore) -> bool {
        store
            .background_failure()
            .borrow()
            .as_ref()
            .is_some_and(BackgroundFailure::store_broken)
    }

    /// Every queued entry, read in place: the queues are left as they are.
    fn queued_entries(store: &WriteBehindDataStore) -> Vec<DelayedEntry> {
        store
            .queues
            .iter()
            .flat_map(|queue| queue.value().entries.values().cloned().collect::<Vec<_>>())
            .collect()
    }

    // ---- what a test reads of the logs and the metrics ----------------------

    #[derive(Debug, Clone)]
    struct CapturedEvent {
        fields: BTreeMap<String, String>,
    }

    impl CapturedEvent {
        fn field(&self, name: &str) -> Option<&str> {
            self.fields.get(name).map(String::as_str)
        }
    }

    struct EventCapture(Arc<Mutex<Vec<CapturedEvent>>>);

    struct FieldCollector<'a>(&'a mut BTreeMap<String, String>);

    impl tracing::field::Visit for FieldCollector<'_> {
        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            self.0.insert(field.name().to_string(), value.to_string());
        }

        fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
            self.0.insert(field.name().to_string(), value.to_string());
        }

        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            self.0
                .insert(field.name().to_string(), format!("{value:?}"));
        }
    }

    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for EventCapture {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            let mut fields = BTreeMap::new();
            event.record(&mut FieldCollector(&mut fields));
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(CapturedEvent { fields });
        }
    }

    /// Runs `body` on a current-thread runtime under `recorder` and a
    /// thread-scoped subscriber, and returns what it returned with every event
    /// logged meanwhile. One thread is what makes both complete: the store's
    /// background tasks run on it.
    fn run_observed<T>(
        recorder: &metrics_exporter_prometheus::PrometheusRecorder,
        body: impl std::future::Future<Output = T>,
    ) -> (T, Vec<CapturedEvent>) {
        use tracing_subscriber::layer::SubscriberExt;

        let events = Arc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::registry().with(EventCapture(Arc::clone(&events)));
        let output = metrics::with_local_recorder(recorder, || {
            tracing::subscriber::with_default(subscriber, || {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("current-thread runtime")
                    .block_on(body)
            })
        });
        let events = events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        (output, events)
    }

    fn events_saying<'a>(events: &'a [CapturedEvent], message: &str) -> Vec<&'a CapturedEvent> {
        events
            .iter()
            .filter(|event| event.field("message") == Some(message))
            .collect()
    }

    /// Whether `rendered` holds `line` as one whole line: the order of series
    /// in a render is not stable.
    fn renders(rendered: &str, line: &str) -> bool {
        rendered.lines().any(|rendered_line| rendered_line == line)
    }

    fn broken_series(cause: &str, count: u64) -> String {
        format!("{STORE_BROKEN_COUNTER}{{cause=\"{cause}\"}} {count}")
    }

    fn panic_series(task: &str, count: u64) -> String {
        format!("{BACKGROUND_TASK_PANICS_COUNTER}{{task=\"{task}\"}} {count}")
    }

    // ---- tests ----------------------------------------------------------------

    #[test]
    fn t_no_more_calls_after_a_broken_store() {
        let recorder = metrics_exporter_prometheus::PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        run_observed(&recorder, async {
            let inner = RefusingStore::new(broken());
            let store = store_over(&inner);
            add_keys(&store).await;

            one_pass(&store).await;

            assert_eq!(inner.calls(), 1, "the failing pass makes exactly one call");
            assert!(
                failed_on_a_broken_store(&store),
                "the refusal of a broken store must fail the store"
            );
            let rendered = handle.render();
            assert!(
                renders(&rendered, &format!("{FLUSH_REFUSALS_COUNTER} 1")),
                "the failing call is counted as one refusal, like any refusal; render \
                 was:\n{rendered}"
            );
            let refused_key = inner.refused_keys()[0].clone();
            let entries = queued_entries(&store);
            assert_eq!(
                entries.len(),
                KEYS.len(),
                "every entry of the pass, the refused one included, is back on its queue"
            );
            let refused: Vec<&DelayedEntry> = entries
                .iter()
                .filter(|entry| entry.key == refused_key)
                .collect();
            assert_eq!(
                refused.len(),
                1,
                "the refused entry {refused_key} is queued"
            );
            assert_eq!(refused[0].retry_count, 1, "the refused entry was counted");
            assert!(
                refused[0].next_attempt.is_some() && refused[0].first_refused.is_some(),
                "the refused entry carries its backoff and its first refusal time"
            );
            for entry in entries.iter().filter(|entry| entry.key != refused_key) {
                assert_eq!(
                    (entry.retry_count, entry.next_attempt, entry.first_refused),
                    (0, None, None),
                    "the entry {} was not attempted and goes back untouched",
                    entry.key
                );
            }

            one_pass(&store).await;
            one_pass(&store).await;
            assert_eq!(
                inner.calls(),
                1,
                "no pass calls a store that reported itself broken"
            );
        });
    }

    #[test]
    fn t_drain_one_call_on_a_broken_store() {
        let recorder = metrics_exporter_prometheus::PrometheusBuilder::new().build_recorder();
        let ((), events) = run_observed(&recorder, async {
            let inner = RefusingStore::new(broken());
            let wal = Arc::new(FrameWal::default());
            let store = store_with_wal(&inner, &wal);
            add_keys(&store).await;

            let error = bounded_hard_flush(&store)
                .await
                .expect_err("a shutdown drain on a broken store returns an error");

            assert!(
                error
                    .downcast_ref::<BackgroundFailure>()
                    .is_some_and(BackgroundFailure::store_broken),
                "the error carries the broken-store failure: {error:?}"
            );
            assert_eq!(inner.calls(), 1, "the drain calls a broken store once");
            assert_eq!(
                wal.unapplied_frames().await,
                KEYS.len(),
                "every frame of an entry the drain did not store stays un-applied"
            );
            assert_eq!(
                wal.mark_applied_calls(),
                0,
                "no frame of an entry the drain did not store is resolved"
            );
        });
        let lines = events_saying(&events, DRAIN_LINE);
        assert_eq!(
            lines.len(),
            1,
            "one drain line; captured events: {events:?}"
        );
        assert_eq!(lines[0].field("abandoned_entries"), Some("3"));
        assert_eq!(lines[0].field("cause"), Some("previous_io"));
        let broken_lines = events_saying(&events, BROKEN_LINE);
        assert_eq!(broken_lines.len(), 1, "captured events: {events:?}");
        assert_eq!(broken_lines[0].field("site"), Some("shutdown_drain"));
    }

    #[tokio::test]
    async fn t_unknown_unchanged() {
        let inner = RefusingStore::new(StoreErrorClass::Unknown);
        let store = store_over(&inner);
        add_keys(&store).await;

        one_pass(&store).await;

        assert_eq!(
            inner.calls(),
            3,
            "every due entry is attempted until the hold trips"
        );
        assert!(
            store.background_failure().borrow().is_none(),
            "an unknown refusal records no failure"
        );
        let backoff = retry_backoff(store.config.backoff_base_ms, store.config.backoff_cap_ms, 1);
        let entries = queued_entries(&store);
        assert_eq!(entries.len(), KEYS.len(), "every refused entry is back");
        for entry in &entries {
            assert_eq!(entry.retry_count, 1, "{} was counted once", entry.key);
            let first_refused = entry
                .first_refused
                .expect("a refused entry carries its first refusal time");
            assert_eq!(
                entry.next_attempt,
                Some(first_refused + backoff),
                "{} waits one backoff from its refusal",
                entry.key
            );
        }
        assert!(
            store.store_hold.load(Ordering::SeqCst),
            "three refusals in a row start the store-wide hold"
        );
        assert_eq!(store.failed_probes.load(Ordering::SeqCst), 0);

        one_pass(&store).await;
        assert_eq!(inner.calls(), 3, "no call while the hold is in force");

        store
            .add(MAP, "k4", &value("k4"), 0, now_millis())
            .await
            .expect("an unknown refusal leaves the store accepting writes");
        bounded_hard_flush(&store)
            .await
            .expect("a drain over unknown refusals ends as on the base");
        assert_eq!(
            inner.calls(),
            7,
            "the drain gives each of the four entries its one last attempt"
        );
    }

    #[test]
    fn t_once_per_process() {
        let recorder = metrics_exporter_prometheus::PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        let ((), events) = run_observed(&recorder, async {
            let inner = RefusingStore::new(broken());
            let store = store_over(&inner);
            add_keys(&store).await;

            one_pass(&store).await;
            assert!(failed_on_a_broken_store(&store));
            assert!(
                bounded_hard_flush(&store).await.is_err(),
                "the shutdown of a broken store fails"
            );
            // At least: how many calls the drain makes is the drain's own
            // test; this one needs only that the drain met the broken store.
            assert!(
                inner.calls() >= 2,
                "the drain met the broken store a second time"
            );
        });
        let lines = events_saying(&events, BROKEN_LINE);
        assert_eq!(
            lines.len(),
            1,
            "one broken store is one line; captured events: {events:?}"
        );
        assert_eq!(lines[0].field("site"), Some("flush"));
        let rendered = handle.render();
        assert!(
            renders(&rendered, &broken_series("previous_io", 1)),
            "one broken store is counted once; render was:\n{rendered}"
        );
    }

    #[test]
    fn t_metric_broken() {
        let recorder = metrics_exporter_prometheus::PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        let (at_construction, _events) = run_observed(&recorder, async {
            let inner = RefusingStore::new(broken());
            let store = store_over(&inner);
            let at_construction = handle.render();
            add_keys(&store).await;

            one_pass(&store).await;
            assert!(failed_on_a_broken_store(&store));
            at_construction
        });
        let after = handle.render();
        for cause in ["io", "previous_io", "corrupted", "lock_poisoned"] {
            assert!(
                renders(&at_construction, &broken_series(cause, 0)),
                "the `{cause}` series renders at 0 from construction; render \
                 was:\n{at_construction}"
            );
            let expected = u64::from(cause == "previous_io");
            assert!(
                renders(&after, &broken_series(cause, expected)),
                "the `{cause}` series reads {expected}; render was:\n{after}"
            );
        }
        for task in ["flush", "stall_watchdog"] {
            assert!(
                renders(&after, &panic_series(task, 0)),
                "a broken store is not a panic: the `{task}` panic series stays 0; render \
                 was:\n{after}"
            );
        }
    }

    #[test]
    fn t_refuse_broken() {
        let recorder = metrics_exporter_prometheus::PrometheusBuilder::new().build_recorder();
        run_observed(&recorder, async {
            let inner = RefusingStore::new(broken());
            let wal = Arc::new(FrameWal::default());
            let store = store_with_wal(&inner, &wal);
            add_keys(&store).await;
            one_pass(&store).await;
            assert!(failed_on_a_broken_store(&store));
            let appends = wal.appends();
            let pending = store.pending_operation_count();

            let add = store
                .add(MAP, "after", &value("after"), 0, now_millis())
                .await
                .expect_err("a store failed on a broken store refuses `add`");
            let remove = store
                .remove(MAP, "after", now_millis())
                .await
                .expect_err("a store failed on a broken store refuses `remove`");
            let remove_all = store
                .remove_all(MAP, &["after".to_string()])
                .await
                .expect_err("a store failed on a broken store refuses `remove_all`");

            let with_key = format!(
                "write-behind store has failed (the durable store is broken); write rejected for \
                 map={MAP} key=after"
            );
            assert_eq!(add.to_string(), with_key);
            assert_eq!(remove.to_string(), with_key);
            assert_eq!(
                remove_all.to_string(),
                format!(
                    "write-behind store has failed (the durable store is broken); write rejected \
                     for map={MAP}"
                )
            );
            assert_eq!(wal.appends(), appends, "a refused write reaches no WAL");
            assert_eq!(store.pending_operation_count(), pending);
        });
    }

    #[test]
    fn t_read_after_break() {
        let recorder = metrics_exporter_prometheus::PrometheusBuilder::new().build_recorder();
        run_observed(&recorder, async {
            let inner = RefusingStore::new(broken());
            let store = store_over(&inner);
            add_keys(&store).await;
            one_pass(&store).await;
            assert!(failed_on_a_broken_store(&store));
            let calls = inner.calls();

            let read = store
                .load(MAP, "k1")
                .await
                .expect("a read of a buffered key answers");

            assert_eq!(
                read,
                Some(value("k1")),
                "the acked value is answered from the buffer"
            );
            assert_eq!(inner.calls(), calls, "the read makes no inner call");
        });
    }
}
