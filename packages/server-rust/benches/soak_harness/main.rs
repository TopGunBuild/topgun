//! Soak harness (G4b) — long-duration endurance test for the TopGun server.
//!
//! Unlike `load_harness` (which boots the server in-process on a `NullDataStore`
//! to measure latency/throughput), the soak harness drives the **real
//! out-of-process `topgun-server` binary** against an on-disk redb + WAL so it
//! can `kill -9` the process and watch it recover. It exercises four endurance
//! properties continuously and fails — with context — the moment any breaks:
//!
//! 1. **Convergence:** under client churn, a quiesced read-back of every key
//!    must equal the harness's authoritative model (no lost/garbled writes).
//! 2. **Crash recovery:** a quiesced-then-`kill -9`-then-restart cycle must
//!    restore the *exact* pre-crash state, repeated many times across a run.
//!    This is checked along two read paths: the Merkle root plus every value
//!    pulled back via the **delta-sync leaf-fetch** path (single-key lazy-load
//!    from the datastore — the path the persistent Merkle index makes correct)
//!    is a HARD gate; the **full-scan QUERY** read-back is a tracked
//!    *expected-fail* gate pending the datastore-backed full-scan, so the two
//!    halves are scoped to the capability each actually delivers.
//! 3. **Bounded memory:** the decrementable OR-Map tombstone-bytes gauge
//!    (`topgun_ormap_tombstone_bytes`, scraped from the server's own
//!    `GET /metrics`) is sampled every interval and its bounded-plateau slope
//!    (last-half-window OLS, boot-recompute-gap samples excluded — see
//!    `monitor.rs`) is computed against a tight per-hour threshold — a direct,
//!    residency-independent leak signal with no allocator/cache noise floor.
//!    The slope is a HARD gate in every run class. The durable-corpus level
//!    clause below is report-only and takes no run class over from it.
//!    A tracked-and-ACKing client
//!    (`SoakClient::connect_tracked` + `confirm_apply`) is driven alongside the
//!    churn clients for the run's duration so the server's per-device causal
//!    frontier — and therefore its low-water-mark — actually advances, which is
//!    what lets the epoch-scoped prune fire and the gauge genuinely plateau
//!    under sustained churn instead of only ever growing. The min-window-span
//!    guard (`DEFAULT_TOMBSTONE_BYTES_MIN_WINDOW_SECS`) keeps a
//!    too-short-to-plateau run (e.g. the 25s blocking smoke) from false-REDing
//!    on a not-yet-flattened ramp, and the blind-monitor (zero-sample) clause
//!    hard-gates independently of the slope. Beside the gauge, the DURABLE
//!    corpus itself is sampled from a byte copy of the datastore at every
//!    recovery checkpoint: a blind sampler, a recent half peaking more than
//!    the configured headroom above the earlier half, and a peak above an
//!    armed ceiling are each recorded and rendered. All three are REPORT-ONLY,
//!    because a control cell with no fault injected at all breached the
//!    headroom clause on its own — an instrument that fires on an unperturbed
//!    run cannot yet tell a leak from ordinary growth, so it decides no
//!    verdict and takes nothing over from the gauge slope above.
//!    Server RSS is sampled in parallel
//!    and asserted against a looser slope as a coarse, non-tombstone backstop
//!    gate. The on-disk data-dir slope (below) remains REPORT-ONLY — bounding
//!    it is a separate, not-yet-landed follow-up.
//! 4. **Zero panics:** any panic marker in server output, or any un-requested
//!    exit, fails the run with the captured context.
//!
//! Two **negative controls** prove the harness can actually fail:
//! `--inject-divergence` makes the convergence check go red, and
//! `--inject-panic` makes the panic capture go red. A soak that cannot fail
//! proves nothing. Two more MODES exist specifically to prove the
//! durable-corpus instrument and the tombstone-byte hard gate above are
//! honest: `--no-ack` disables the tracked
//! client's confirm-apply loop (the low-water-mark then never advances, prune
//! never fires, and the gate must FAIL under sustained churn), and
//! `--inject-slow-leak` adds a second, deliberately slow-acking tracked client
//! whose stale cursor repeatedly caps the fleet-wide low-water-mark — a bounded
//! ramp-then-catch-up pattern used to calibrate the OLS slope's detection floor
//! against a small, non-instantaneous leak rather than only total blockage.
//! Independently of the gauge, `scan_redb_tombstone_corpus` sums the real
//! on-disk tombstone corpus. It runs once per recovery checkpoint — between
//! the `kill -9` and the restart, over a BYTE COPY on a scratch path outside
//! the data dir, because opening a redb file that closed uncleanly repairs and
//! commits it and the instrument must not run the server's recovery ahead of
//! the server — and once terminally after the process exits, where it reads
//! the data dir directly because nothing boots after it. The resulting series
//! feeds the durable-corpus level/ceiling assessment, and its terminal value
//! still cross-checks the gauge for a gauge/corpus divergence (e.g. a legacy
//! `OrTombstones` blob the hot-path gauge never counted on add).
//!
//! See `benches/soak_harness/README.md` for usage and the Hetzner 72h runner.

#![allow(
    clippy::too_many_lines,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::struct_excessive_bools,
    // Subjective style lints not worth contorting a bench harness for: many
    // local snapshot pairs read naturally as pre_/post_ etc., and prose like
    // "TopGun"/"kill -9" should not be backtick-quoted.
    clippy::similar_names,
    clippy::doc_markdown
)]

mod client;
mod model;
mod monitor;
mod or_noloss;
// The recovery checkpoint expanded `ServerSupervisor::restart` in place — it
// has to take a corpus sample between the kill and the start — so no caller
// inside this binary is left. `restart` itself is deliberately kept: its
// contract is unchanged and `tests/soak_tombstone_restart.rs`, which includes
// this same module, still calls it.
#[allow(dead_code)]
mod process;
mod report;

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use parking_lot::Mutex;
use redb::ReadableTable;
use tokio::io::{AsyncBufReadExt, BufReader};

use client::SoakClient;
use model::{compare, next_stamp, Model};
use monitor::{
    aggregate_origin_lines, assess, assess_disk, assess_tombstone_bytes,
    assess_tombstone_corpus_level, classify_durable_reading, classify_origin_reading,
    classify_series_shape, exclude_boot_gap_samples, fold_lww_key, fold_or_key,
    fold_undecodable_key, parse_labelled_gauge, sample_disk_mb, sample_redb_bytes, sample_rss_mb,
    sample_wal_retention, slope_clause_stays_hard, BootGap, CensusRecord, CensusSource,
    CorpusLevelDisposition, CorpusSample, DiskAssessment, DiskSample, DurableCensus,
    DurableReading, EpochsExitedAbsence, GaugeFold, GaugeObservation, GaugeReading, MemSample,
    OrVariant, OriginLine, SeriesPoint, SeriesShapeReading, TombstoneAssessment,
    TombstoneCorpusAssessment, TombstoneSample, DECIDING_SERIES, DEFAULT_DISK_CEILING_MB,
    DEFAULT_DISK_MIN_GROWTH_MB, DEFAULT_DISK_THRESHOLD_MB_PER_HOUR,
    DEFAULT_TOMBSTONE_BYTES_MIN_GROWTH, DEFAULT_TOMBSTONE_BYTES_MIN_WINDOW_SECS,
    DEFAULT_TOMBSTONE_BYTES_THRESHOLD_PER_HOUR, DEFAULT_TOMBSTONE_CORPUS_CEILING_BYTES,
    DEFAULT_TOMBSTONE_CORPUS_HEADROOM_BYTES, DEFAULT_TOMBSTONE_CORPUS_MIN_SAMPLES,
    DEFAULT_TOMBSTONE_CORPUS_MIN_SPAN_SECS,
};
use or_noloss::{missing_acked_adds, OrLedger};
use process::{resolve_server_binary, OriginCaptureSnapshot, ServerConfig, ServerSupervisor};
use report::{
    append_progress, effective_epoch_width, scan_wal_frame_sizes, utc_timestamp_now, write_report,
    ConfirmApplyReport, DiskReport, MemoryReport, ProgressSnapshot, SoakReport,
    TombstoneCorpusReport, TombstoneReport, WalFrameStats,
};
use topgun_server::storage::record::RecordValue;

/// Subject index used by the orchestrator's verifier connections. Far above the
/// churn-client range so it never owns keys or collides with churn auth.
const VERIFIER_IDX: usize = 1_000_000;

/// Subject index for the tracked-and-ACKing client that drives the server's
/// low-water-mark forward. Distinct from `VERIFIER_IDX` (and its `+1` sibling)
/// and the churn-client range so its device identity never collides.
const TRACKER_IDX: usize = 2_000_000;

/// Subject index for the `--inject-slow-leak` variant's second tracked client.
const SLOW_LEAK_TRACKER_IDX: usize = 2_000_001;

/// Confirm-apply cadence for the `--inject-slow-leak` tracked client —
/// deliberately much slower than any reasonable `--confirm-interval`, so its
/// stale cursor is the binding (minimum) term in the fleet-wide low-water-mark
/// for most of the run, producing a repeated ramp-then-catch-up pattern rather
/// than a continuous plateau.
const SLOW_LEAK_ACK_INTERVAL: Duration = Duration::from_secs(90);

const LWW_MAP: &str = "soak_lww";
const OR_MAP: &str = "soak_or";

/// The rendered prefix of the durable-corpus gate's verdict line. Single
/// source of truth: both `println!` sites and every assertion take it from
/// here, so a rename REDs loudly instead of missing silently.
const TOMBSTONE_CORPUS_LINE_PREFIX: &str = "tombstone_corpus_redb_scan:";

/// The `finished_reason` a run carries when nothing wrote a breach reason.
/// Single source of truth: the initialiser and the ranked verdict writer's
/// rank-0 guard both take it from here, so rewording the default cannot
/// silently disable rank 0 and let the verdict block overwrite an
/// earlier-phase reason.
const FINISHED_REASON_DURATION_REACHED: &str = "duration reached";

/// Parsed CLI configuration.
struct Config {
    duration: Duration,
    churn_clients: usize,
    keyspace: usize,
    write_interval: Duration,
    writes_per_life: usize,
    offline_keys: usize,
    crash_interval: Option<Duration>,
    steady_interval: Duration,
    quiesce: Duration,
    ready_timeout: Duration,
    mem_sample_interval: Duration,
    mem_threshold_mb_per_hour: f64,
    mem_min_growth_mb: f64,
    mem_ceiling_mb: f64,
    server_port: u16,
    data_dir: Option<PathBuf>,
    wal_fsync: String,
    or_churn: bool,
    or_keyspace: usize,
    or_every: u64,
    json_output: Option<PathBuf>,
    progress_output: Option<PathBuf>,
    inject_divergence: bool,
    inject_panic: bool,
    /// How often the tracked-and-ACKing client (see `TRACKER_IDX`) runs one
    /// `confirm_apply` round. This is the cadence at which the server's
    /// per-device causal frontier — and therefore its low-water-mark — can
    /// advance, which in turn licenses the epoch-scoped tombstone prune.
    confirm_interval: Duration,
    /// Negative control: disables the tracked client's confirm-apply loop
    /// entirely. With no client ever confirming, the low-water-mark stays 0,
    /// prune never fires, and sustained OR churn must trip the tombstone-byte
    /// hard gate — with the report-only durable-corpus instrument recording
    /// the same growth alongside it.
    no_ack: bool,
    /// Adds a second tracked client (`SLOW_LEAK_TRACKER_IDX`) that ACKs on a
    /// much slower cadence than the primary tracker. Because the low-water-mark
    /// is the MINIMUM cursor across all tracked clients, this caps pruning to
    /// the slow client's stale confirmations, producing a bounded
    /// ramp-then-catch-up pattern that exercises the OLS slope gate's
    /// detection floor against a small, slow leak rather than only the
    /// `--no-ack` total-blockage case.
    inject_slow_leak: bool,
    /// Skip the pre-`kill -9` quiesce drain in the recovery checkpoint. When set,
    /// the checkpoint kills the server WITHOUT first letting the write-behind
    /// buffer flush to redb, so post-restart recovery must rely on the WAL alone.
    /// This is the assertion mode that proves acked == durable on `kill -9` under
    /// load: it does NOT depend on a pre-kill flush masking a durability gap.
    no_pre_kill_drain: bool,
    /// Slack, in bytes, the durable-corpus LEVEL clause allows the recent half
    /// of the corpus series to sit above the earlier half by before it breaches.
    /// Exposed so a measurement round can retune the clause without a rebuild.
    tombstone_corpus_headroom_bytes: u64,
    /// Absolute durable-corpus ceiling, in bytes. `None` leaves the ceiling
    /// clause DISARMED, which is the default: arming it honestly needs a
    /// validated number, and this flag is how a run supplies one.
    tombstone_corpus_ceiling_bytes: Option<u64>,
    /// Opt-in measurement mode: after the run, scan the retained WAL segment
    /// files and emit a structured mechanism report (Q1-Q4) attributing the
    /// OR-churn WAL+RSS growth. REPORT-ONLY — never affects `passed`, so it
    /// cannot false-RED the CI smoke; it exists so a LIVE 60-min OR-churn run
    /// produces the numbers that pin the fix mechanism. Also snapshots the WAL
    /// dir's on-disk size immediately before stop and immediately after the
    /// shutdown drain so the Q2 drain-window burst can be attributed.
    mechanism_report: bool,
    /// Opt-in durable-layer reading: arms the filesystem samplers, the widened
    /// observation columns and the sibling durable artifact. REPORT-ONLY —
    /// nothing it produces is ANDed into `passed`. An unflagged run spawns no
    /// extra sampler task at all, so its sampling behaviour is exactly what it
    /// was before this mode existed.
    durable_reading: bool,
    /// How often, in seconds, to take a store-level census from a byte COPY of
    /// the LIVE store file. `0` DISARMS it, and that is the default: a copy of
    /// a live store file is a smeared image, so the census it yields is
    /// best-effort by construction and OBSERVATION ONLY — it may not enter any
    /// predicate, and an armed run also pays a full file copy per sample.
    live_census_interval_secs: u64,
    /// Seed for the filesystem samplers' bounded cadence jitter. SUPPLIED BY
    /// THE RUNNER and never derived inside this binary: the runner echoes the
    /// same shell variable into its matrix record, so the seed on the record
    /// and the seed the sampler actually used have ONE source and cannot
    /// drift — and the seed is provably fixed before the run produces any data,
    /// which a run-time-derived seed reported afterwards could not be.
    sampler_jitter_seed: u64,
    /// True once any soak-controlling flag is parsed. A bare invocation (or one
    /// carrying only foreign libtest args, as `cargo test --all-targets` passes)
    /// leaves this false so the harness prints usage and exits 0 instead of
    /// launching a multi-hour default soak inside the test runner.
    mode_requested: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            duration: Duration::from_secs(3600),
            churn_clients: 16,
            keyspace: 200,
            write_interval: Duration::from_millis(20),
            writes_per_life: 200,
            offline_keys: 3,
            crash_interval: Some(Duration::from_secs(120)),
            steady_interval: Duration::from_secs(30),
            quiesce: Duration::from_secs(3),
            ready_timeout: Duration::from_secs(40),
            mem_sample_interval: Duration::from_secs(5),
            // Calibrated to catch the OR-Map tombstone leak (~3-5 MB/h) rather
            // than mask it: the old 50 MB/h slope sat far above the leak rate and
            // false-GREENed it. See monitor.rs for the calibration rationale and
            // the executable proof (tests::calibration_*).
            mem_threshold_mb_per_hour: monitor::DEFAULT_MEM_THRESHOLD_MB_PER_HOUR,
            mem_min_growth_mb: monitor::DEFAULT_MEM_MIN_GROWTH_MB,
            mem_ceiling_mb: 1800.0,
            server_port: 0,
            data_dir: None,
            // Durability under the soak comes from PerOp: every WAL frame is
            // fdatasync'd before the ingress write acks, so acked == durable on a
            // `kill -9`. The parser normalizes case/separator, so per_op/perop are
            // equivalent; this canonical spelling matches the production default.
            wal_fsync: "per_op".to_string(),
            or_churn: true,
            or_keyspace: 32,
            or_every: 5,
            json_output: None,
            progress_output: None,
            inject_divergence: false,
            inject_panic: false,
            confirm_interval: Duration::from_secs(2),
            no_ack: false,
            inject_slow_leak: false,
            no_pre_kill_drain: false,
            tombstone_corpus_headroom_bytes: DEFAULT_TOMBSTONE_CORPUS_HEADROOM_BYTES,
            tombstone_corpus_ceiling_bytes: DEFAULT_TOMBSTONE_CORPUS_CEILING_BYTES,
            mechanism_report: false,
            durable_reading: false,
            live_census_interval_secs: 0,
            sampler_jitter_seed: 0,
            mode_requested: false,
        }
    }
}

/// The sampler-start clock plus the mutable boot-recompute-gap window list,
/// bundled so `recovery_checkpoint` (which records a gap around every
/// `kill -9` + restart) takes one parameter instead of two — keeping the
/// function under clippy's argument-count lint without an `#[allow]`.
struct BootGapClock {
    sampler_start: Instant,
    boot_gaps: Arc<Mutex<Vec<BootGap>>>,
}

/// Everything the durable-layer reading observes while the run is live.
///
/// One shared sink, so every sampler task and the metrics scrape write to the
/// same place and the reading is assembled from one value rather than from
/// several loosely-related locals. EVERY field here is an OBSERVATION: none of
/// them is ANDed into the run verdict, and the durable-corpus estimator reads
/// none of them — which is what keeps the estimator's input identical whether
/// this mode is armed or not.
#[derive(Default)]
struct DurableObservations {
    /// Resident-set series in KiB, derived from the memory sampler's own
    /// stream so the durable reading and the memory gate provably read the
    /// SAME series on the SAME cadence.
    rss_kib: Mutex<Vec<SeriesPoint>>,
    /// Apparent size of the store file, in bytes.
    redb_bytes: Mutex<Vec<SeriesPoint>>,
    /// Sum of the retained WAL segments' apparent sizes, in bytes.
    wal_bytes: Mutex<Vec<SeriesPoint>>,
    /// Number of retained WAL segment files.
    wal_segment_files: Mutex<Vec<SeriesPoint>>,
    /// The write-behind watermark lag column, folded across every scrape of the
    /// run. Its `value` is the largest lag seen; `None` means no scrape ever
    /// read the metric — a visible gap, deliberately NOT recorded as a zero,
    /// because a silent absence must never be able to masquerade as a flat
    /// series, and the counters beside it say whether the metric was missing
    /// from the bodies or present in them and unreadable.
    writebehind_lag: Mutex<GaugeObservation>,
    /// The exited-epoch column, folded across every scrape. Its `value` is a
    /// maximum rather than a last value, because the counter restarts with the
    /// process.
    ///
    /// Held as an observation rather than as an atomic counter: an atomic
    /// seeded at zero cannot distinguish "never observed" from "observed as
    /// zero", and that distinction is the entire reason this column exists —
    /// the zero is the most reassuring reading the origin classifier can emit,
    /// and it must never be reachable from no evidence at all.
    epochs_exited: Mutex<GaugeObservation>,
    /// Server restarts during the run. The recovery checkpoint is the ONLY
    /// producer of this count; nothing infers it from a log line.
    restarts: AtomicU64,
}

/// One bounded, deterministic sleep for the filesystem samplers: the nominal
/// interval plus a seeded offset uniform over +/-1 s.
///
/// The jitter exists against ALIASING. The write-behind flush period divides
/// the nominal sampling interval exactly, and file sizes step at flush
/// boundaries, so a sampler in fixed phase with the flush cycle can capture the
/// same phase of every cycle for the whole run. Only the filesystem series
/// carry it; the RSS cadence is deliberately left alone.
///
/// SplitMix64, written out here rather than pulled in, because the sampler
/// needs exactly one property: the same seed reproduces the same cadence on a
/// re-run. The result saturates at zero, so a nominal interval shorter than the
/// jitter bound can never ask for a negative sleep.
fn jittered_interval(state: &mut u64, nominal: Duration) -> Duration {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    let offset_ms = i64::try_from(z % 2001).unwrap_or(1000) - 1000;
    let nominal_ms = i64::try_from(nominal.as_millis()).unwrap_or(i64::MAX);
    Duration::from_millis(u64::try_from(nominal_ms.saturating_add(offset_ms)).unwrap_or(0))
}

/// Shared atomic counters mutated by churn clients.
#[derive(Default)]
struct SoakMetrics {
    total_writes: AtomicU64,
    write_errors: AtomicU64,
    reconnects: AtomicU64,
    resends: AtomicU64,
    /// Count of completed `confirm_apply` rounds that actually ACKed an epoch
    /// (i.e. `Ok(Some(_))`), across every tracked client. Visibility signal for
    /// a long soak: this staying at 0 for the whole run means the low-water-mark
    /// never advanced (expected under `--no-ack`; a bug otherwise).
    confirms: AtomicU64,
    /// Highest epoch any tracked client has ACKed via `ClientApplyAck`. The
    /// server's fleet-wide low-water-mark tracks the MIN across tracked clients,
    /// so this is an upper bound on the LWM — a diagnostic that the confirm-apply
    /// path is actually advancing the causal frontier the prune keys off.
    last_confirmed_epoch: AtomicU64,
    /// Count of `confirm_apply` rounds that errored (forcing a reconnect). A
    /// non-`--no-ack` run with this climbing while `confirms` stays flat means a
    /// harness plumbing failure (the tracked client can't ACK) — NOT a server
    /// tombstone leak, even though both surface as an unbounded gauge slope.
    confirm_errors: AtomicU64,
}

/// Context shared with every churn client task.
struct ChurnCtx {
    supervisor: Arc<ServerSupervisor>,
    model: Arc<Model>,
    /// Acked persistent-OR-add ledger: the set of adds that must survive a
    /// `kill -9`. Updated only on an `or_add` ACK for the add-only persistent
    /// keyspace; read by `recovery_checkpoint` for the directional no-loss check.
    or_ledger: Arc<OrLedger>,
    metrics: Arc<SoakMetrics>,
    paused: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    jwt_secret: String,
    or_churn: bool,
    or_keyspace: usize,
    or_every: u64,
    write_interval: Duration,
    writes_per_life: usize,
    offline_keys: usize,
}

#[tokio::main]
async fn main() {
    let config = parse_args();

    // Guard: a bare invocation (e.g. `cargo test --all-targets` running this
    // harness=false bench, which passes only foreign libtest args) must not
    // launch a multi-hour soak. Require an explicit mode flag.
    if !config.mode_requested {
        print_usage();
        std::process::exit(0);
    }

    let code = if config.inject_panic {
        run_inject_panic().await
    } else if config.inject_divergence {
        run_inject_divergence(&config).await
    } else {
        run_soak(&config).await
    };

    std::process::exit(code);
}

// ---------------------------------------------------------------------------
// Main soak run
// ---------------------------------------------------------------------------

#[allow(clippy::cognitive_complexity)]
async fn run_soak(config: &Config) -> i32 {
    println!("=== TopGun soak harness (G4b / TODO-484) ===");
    println!(
        "duration={}s churn_clients={} keyspace={} crash_interval={:?} steady_interval={}s \
         wal_fsync={} or_churn={}",
        config.duration.as_secs(),
        config.churn_clients,
        config.keyspace,
        config.crash_interval.map(|d| d.as_secs()),
        config.steady_interval.as_secs(),
        config.wal_fsync,
        config.or_churn,
    );

    // Single-writer-per-persist-key invariant. The persistent OR keyspace maps
    // slot `i` to `ork-persist-{i % or_keyspace}` and slot `i` is owned solely by
    // churn client `i % churn_clients`. Two distinct clients share a persist
    // bucket only if some owned slots collide under `% or_keyspace` without
    // colliding under `% churn_clients` — impossible exactly when
    // `churn_clients | or_keyspace`. A config that breaks this (e.g.
    // `--churn-clients 16 --or-keyspace 24`) would make the persist keyspace
    // multi-writer and invalidate the no-loss check's single-writer premise, so
    // fail loudly rather than silently degrade the gate.
    if config.or_churn {
        // `or_keyspace == 0` would make `is_multiple_of` vacuously true while every
        // `slot % or_keyspace.max(1)` collapses to a single multi-writer persist key,
        // defeating the single-writer premise the assert exists to protect. Require a
        // positive keyspace explicitly.
        assert!(
            config.or_keyspace > 0,
            "or_keyspace must be > 0 when or_churn is enabled"
        );
        assert!(
            config.or_keyspace.is_multiple_of(config.churn_clients),
            "or_keyspace ({}) must be a multiple of churn_clients ({}) to keep the \
             persistent OR keyspace single-writer-per-key",
            config.or_keyspace,
            config.churn_clients,
        );
    }

    let binary = resolve_server_binary();

    // Persistent on-disk data dir. A caller-supplied dir survives the run for
    // forensics; otherwise a tempdir is created and kept for the process lifetime.
    let (data_dir, _tempdir_guard) = match &config.data_dir {
        Some(d) => {
            if let Err(e) = std::fs::create_dir_all(d) {
                eprintln!("FATAL: cannot create data dir {}: {e}", d.display());
                return 2;
            }
            (d.clone(), None)
        }
        None => match tempfile::tempdir() {
            Ok(td) => (td.path().to_path_buf(), Some(td)),
            Err(e) => {
                eprintln!("FATAL: cannot create tempdir: {e}");
                return 2;
            }
        },
    };

    // Scratch root for the durable-corpus sampler's byte copies. Created ONCE
    // and OUTSIDE `data_dir`, because the disk gate's input is `du -sk` over
    // `data_dir` and a copy written inside it would step that gate's own
    // measurement. The guard removes the whole tree when the run returns, on
    // the pass path and the fail path alike, so a failing run leaves nothing
    // behind; each sample already deletes its own copy long before that.
    let (corpus_scratch, _corpus_scratch_guard) = match tempfile::tempdir() {
        Ok(td) => (td.path().to_path_buf(), td),
        Err(e) => {
            eprintln!("FATAL: cannot create corpus scratch dir: {e}");
            return 2;
        }
    };

    let port = if config.server_port == 0 {
        match ServerSupervisor::pick_free_port() {
            Ok(p) => p,
            Err(e) => {
                eprintln!("FATAL: cannot pick free port: {e}");
                return 2;
            }
        }
    } else {
        config.server_port
    };

    let jwt_secret = "test-e2e-secret".to_string();
    let supervisor = ServerSupervisor::new(ServerConfig {
        binary,
        data_dir: data_dir.clone(),
        port,
        jwt_secret: jwt_secret.clone(),
        wal_fsync_policy: config.wal_fsync.clone(),
    });

    println!(
        "starting server (port {port}, data {}) ...",
        data_dir.display()
    );
    if let Err(e) = supervisor.start(config.ready_timeout).await {
        eprintln!("FATAL: server failed to start: {e}");
        return 2;
    }
    let panic_watch = supervisor.panic_watch();

    let model = Arc::new(Model::new(config.keyspace, config.churn_clients));
    let or_ledger = Arc::new(OrLedger::new());
    let metrics = Arc::new(SoakMetrics::default());
    let paused = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));

    // --- Spawn churn clients ---
    let mut churn_handles = Vec::with_capacity(config.churn_clients);
    for idx in 0..config.churn_clients {
        let ctx = ChurnCtx {
            supervisor: Arc::clone(&supervisor),
            model: Arc::clone(&model),
            or_ledger: Arc::clone(&or_ledger),
            metrics: Arc::clone(&metrics),
            paused: Arc::clone(&paused),
            stop: Arc::clone(&stop),
            jwt_secret: jwt_secret.clone(),
            or_churn: config.or_churn,
            or_keyspace: config.or_keyspace,
            or_every: config.or_every,
            write_interval: config.write_interval,
            writes_per_life: config.writes_per_life,
            offline_keys: config.offline_keys,
        };
        churn_handles.push(tokio::spawn(run_churn_client(idx, ctx)));
    }

    // --- Spawn the tracked-and-ACKing client(s) that drive the server's
    // low-water-mark ---
    //
    // Without at least one client running `connect_tracked` + `confirm_apply`,
    // the server's per-device causal frontier tracks NO clients at all, so
    // `low_water_mark()` is vacuously 0 forever and the epoch-scoped prune
    // never fires regardless of how much churn runs — this is what made the
    // tombstone-byte slope report-only before this driver existed. The primary
    // tracker below always spawns (its ack loop is skipped, not the connection,
    // when `--no-ack` is set — the negative control needs the request/response
    // shape to still run so a real regression in the harness plumbing itself
    // would still be caught by other assertions). `--inject-slow-leak` adds a
    // second tracked client with a much slower ack cadence purely for slope-gate
    // calibration; see `SLOW_LEAK_ACK_INTERVAL`.
    churn_handles.push(tokio::spawn(run_tracked_confirm_client(TrackerConfig {
        supervisor: Arc::clone(&supervisor),
        jwt_secret: jwt_secret.clone(),
        stop: Arc::clone(&stop),
        paused: Arc::clone(&paused),
        metrics: Arc::clone(&metrics),
        idx: TRACKER_IDX,
        confirm_interval: config.confirm_interval,
        no_ack: config.no_ack,
    })));
    if config.inject_slow_leak {
        churn_handles.push(tokio::spawn(run_tracked_confirm_client(TrackerConfig {
            supervisor: Arc::clone(&supervisor),
            jwt_secret: jwt_secret.clone(),
            stop: Arc::clone(&stop),
            paused: Arc::clone(&paused),
            metrics: Arc::clone(&metrics),
            idx: SLOW_LEAK_TRACKER_IDX,
            confirm_interval: SLOW_LEAK_ACK_INTERVAL,
            no_ack: false,
        })));
    }

    // --- Spawn memory + tombstone-bytes + disk samplers ---
    // All three samplers share one clock so their `elapsed_secs` axes line up
    // exactly (required for a fair side-by-side slope comparison in the
    // summary/report).
    let sampler_start = Instant::now();
    // The durable-layer observation sink. Created unconditionally because the
    // restart counter and the widened `/metrics` columns below feed it on every
    // run; only the extra SAMPLER TASKS are gated on `--durable-reading`.
    let durable = Arc::new(DurableObservations::default());
    let samples: Arc<Mutex<Vec<MemSample>>> = Arc::new(Mutex::new(Vec::new()));
    let peak_rss = Arc::new(Mutex::new(0.0_f64));
    {
        let supervisor = Arc::clone(&supervisor);
        let samples = Arc::clone(&samples);
        let peak_rss = Arc::clone(&peak_rss);
        let stop = Arc::clone(&stop);
        let interval = config.mem_sample_interval;
        let start = sampler_start;
        tokio::spawn(async move {
            loop {
                if stop.load(Ordering::SeqCst) {
                    return;
                }
                if let Some(pid) = supervisor.current_pid() {
                    if let Some(mb) = sample_rss_mb(pid) {
                        let elapsed = start.elapsed().as_secs_f64();
                        samples.lock().push(MemSample {
                            elapsed_secs: elapsed,
                            rss_mb: mb,
                        });
                        let mut p = peak_rss.lock();
                        if mb > *p {
                            *p = mb;
                        }
                    }
                }
                tokio::time::sleep(interval).await;
            }
        });
    }

    // Direct, residency-independent tombstone-byte gauge sampler: scrapes the
    // real `topgun_ormap_tombstone_bytes_total` Prometheus counter off the same
    // running server over HTTP, the production surface (KL2) rather than a
    // test-only hook. A transient scrape failure (connection refused mid-restart,
    // non-200, or the metric line absent) is skipped rather than recorded as a
    // bogus point, mirroring `sample_rss_mb`'s `None`-on-gone-process contract.
    let tombstone_samples: Arc<Mutex<Vec<TombstoneSample>>> = Arc::new(Mutex::new(Vec::new()));
    // Boot-recompute-gap windows recorded around each `kill -9` + restart
    // cycle (`recovery_checkpoint` records both `start_secs` and, once the
    // restarted process signals health-ready, `end_secs`). The tombstone
    // sampler below excludes any sample landing inside one of these windows
    // so a spurious pre-reconcile read never pollutes the OLS slope.
    let boot_gaps: Arc<Mutex<Vec<BootGap>>> = Arc::new(Mutex::new(Vec::new()));
    {
        let samples = Arc::clone(&tombstone_samples);
        let boot_gaps = Arc::clone(&boot_gaps);
        let stop = Arc::clone(&stop);
        let durable = Arc::clone(&durable);
        let interval = config.mem_sample_interval;
        let start = sampler_start;
        let http = reqwest::Client::new();
        tokio::spawn(async move {
            loop {
                if stop.load(Ordering::SeqCst) {
                    return;
                }
                if let Some(scraped) = scrape_tombstone_bytes(&http, port).await {
                    // OBSERVATION-ONLY columns, taken off the SAME response body
                    // the gauge came from so they cost no extra request and an
                    // unflagged run's traffic is unchanged. Neither may enter any
                    // predicate: the lag is the only candidate that would be a
                    // self-report by the very process under measurement, and its
                    // durable consequence is already carried by the WAL retention
                    // series; the exited-epoch total is only ever the origin
                    // reading's qualifier.
                    {
                        // `max` across label sets: the lag is per-partition, and
                        // the observation this column carries is how far the
                        // WORST partition fell behind.
                        let mut lag = durable.writebehind_lag.lock();
                        fold_scrape_into(&mut lag, &scraped.writebehind_lag, GaugeFold::Max);
                    }
                    {
                        // `sum` across label sets: this is a total, and an
                        // unlabelled series sums to its own single value.
                        let mut exited = durable.epochs_exited.lock();
                        fold_scrape_into(&mut exited, &scraped.epochs_exited, GaugeFold::Sum);
                    }
                    if let Some(bytes) = scraped.tombstone_bytes {
                        let elapsed = start.elapsed().as_secs_f64();
                        let candidate = TombstoneSample {
                            elapsed_secs: elapsed,
                            bytes,
                        };
                        // The exclusion predicate is a pure, retain-style helper in
                        // `monitor.rs` (unit-tested directly by the calibration
                        // target's synthetic boot-gap sequence, AC11) — this loop
                        // only calls it, so the filtering logic is never
                        // duplicated or buried here, and the tested path is the
                        // production path.
                        let gaps_snapshot = boot_gaps.lock().clone();
                        if !exclude_boot_gap_samples(
                            std::slice::from_ref(&candidate),
                            &gaps_snapshot,
                        )
                        .is_empty()
                        {
                            samples.lock().push(candidate);
                        }
                    }
                }
                tokio::time::sleep(interval).await;
            }
        });
    }

    // Disk-usage sampler: shells `du -sk` over the resolved local `data_dir`
    // binding above (the same value handed to `ServerConfig.data_dir`) — NOT
    // the raw `config.data_dir` `Option`, which is `None` on the default/CI
    // path (the blocking Soak Smoke G4b invocation passes no `--data-dir`) and
    // would yield zero samples, spuriously tripping the disk blind-monitor
    // clause below. Runs on the same `sampler_start` clock as RSS/tombstone
    // bytes so all three `elapsed_secs` axes line up for comparison.
    let disk_samples: Arc<Mutex<Vec<DiskSample>>> = Arc::new(Mutex::new(Vec::new()));
    {
        let samples = Arc::clone(&disk_samples);
        let stop = Arc::clone(&stop);
        let interval = config.mem_sample_interval;
        let start = sampler_start;
        let dir = data_dir.clone();
        tokio::spawn(async move {
            loop {
                if stop.load(Ordering::SeqCst) {
                    return;
                }
                if let Some(mb) = sample_disk_mb(&dir) {
                    let elapsed = start.elapsed().as_secs_f64();
                    samples.lock().push(DiskSample {
                        elapsed_secs: elapsed,
                        disk_mb: mb,
                    });
                }
                tokio::time::sleep(interval).await;
            }
        });
    }

    // Durable-layer filesystem samplers, ARMED ONLY under `--durable-reading`:
    // an unflagged run spawns no task here at all, so its sampling behaviour is
    // byte-identical to what it was before this mode existed. Filesystem
    // METADATA only — no store or segment file is ever opened — so sampling
    // these cannot perturb the process being measured.
    //
    // The cadence carries the seeded jitter, and these three series are the ONLY
    // ones that carry it (see `jittered_interval` for the aliasing hazard it
    // exists against). The RSS series above is deliberately left un-jittered:
    // the resident set moves continuously rather than stepping at flush
    // boundaries, so there is no phase for a fixed-phase sampler to lock onto,
    // and it is the same series the memory gate of EVERY run reads — changing
    // its cadence would change that gate's input on flagged and unflagged runs
    // alike.
    if config.durable_reading {
        let durable = Arc::clone(&durable);
        let stop = Arc::clone(&stop);
        let interval = config.mem_sample_interval;
        let start = sampler_start;
        let dir = data_dir.clone();
        let mut jitter_state = config.sampler_jitter_seed;
        tokio::spawn(async move {
            loop {
                if stop.load(Ordering::SeqCst) {
                    return;
                }
                let elapsed = start.elapsed().as_secs_f64();
                if let Some(bytes) = sample_redb_bytes(&dir) {
                    durable.redb_bytes.lock().push(SeriesPoint {
                        elapsed_secs: elapsed,
                        value: bytes,
                    });
                }
                // One retention read feeds both WAL series, so the two can never
                // disagree about which set of segments they describe.
                if let Some(retention) = sample_wal_retention(&dir) {
                    durable.wal_bytes.lock().push(SeriesPoint {
                        elapsed_secs: elapsed,
                        value: retention.bytes,
                    });
                    durable.wal_segment_files.lock().push(SeriesPoint {
                        elapsed_secs: elapsed,
                        value: retention.segment_files,
                    });
                }
                tokio::time::sleep(jittered_interval(&mut jitter_state, interval)).await;
            }
        });
    }

    let boot_gap_clock = BootGapClock {
        sampler_start,
        boot_gaps: Arc::clone(&boot_gaps),
    };

    // Durable-corpus series: one sample per recovery checkpoint (taken from a
    // byte copy while the child is reaped) plus the terminal scan, all on the
    // same `sampler_start` clock as the RSS/tombstone-byte/disk series.
    let corpus_sampler = Arc::new(CorpusSampler::new(data_dir.clone(), corpus_scratch));

    // Live-copy census sampler: DISARMED by default (interval 0), and armed
    // only by an explicit positive interval. Its records are OBSERVATION ONLY
    // and land in their own tally — see `sample_live_census_via_copy` for why a
    // census taken off a live store file may not decide anything.
    if config.live_census_interval_secs > 0 {
        let sampler = Arc::clone(&corpus_sampler);
        let stop = Arc::clone(&stop);
        let interval = Duration::from_secs(config.live_census_interval_secs);
        let start = sampler_start;
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;
                if stop.load(Ordering::SeqCst) {
                    return;
                }
                let elapsed = start.elapsed().as_secs_f64();
                let sampler = Arc::clone(&sampler);
                // A whole-file copy is genuinely blocking work; keeping it off
                // the async workers is what stops an observation-only sampler
                // from stalling the churn clients whose write rate defines the
                // workload under measurement.
                if tokio::task::spawn_blocking(move || {
                    sample_live_census_via_copy(&sampler, elapsed);
                })
                .await
                .is_err()
                {
                    return;
                }
            }
        });
    }

    // --- Orchestration loop ---
    let start = Instant::now();
    let deadline = start + config.duration;
    let mut next_steady = start + config.steady_interval;
    let mut next_crash = config
        .crash_interval
        .map_or(deadline + Duration::from_secs(86400), |d| start + d);

    let mut steady_checkpoints = 0u64;
    let mut recovery_checkpoints = 0u64;
    let mut crashes = 0u64;
    let mut convergence_failures: Vec<String> = Vec::new();
    let mut recovery_failures: Vec<String> = Vec::new();
    // SPEC-322b expected-fail gate: post-restart QUERY-path read-back. Tracked,
    // reported, and never fails the run on this (322a) branch.
    let mut pending_gates: Vec<String> = Vec::new();
    let mut last_convergence_ok = true;
    let mut finished_reason = FINISHED_REASON_DURATION_REACHED.to_string();

    loop {
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        let wake = next_steady.min(next_crash).min(deadline);
        tokio::time::sleep_until(tokio::time::Instant::from_std(wake)).await;

        if panic_watch.tripped() {
            finished_reason = "server panic detected".to_string();
            break;
        }

        let now = Instant::now();
        if now >= deadline {
            break;
        }

        let phase;
        if now >= next_crash {
            phase = "recovery";
            match recovery_checkpoint(
                &supervisor,
                &model,
                &or_ledger,
                &jwt_secret,
                config,
                &paused,
                &boot_gap_clock,
                &corpus_sampler,
                &durable.restarts,
            )
            .await
            {
                Ok(outcome) => {
                    recovery_checkpoints += 1;
                    crashes += 1;
                    pending_gates.extend(outcome.pending_gates);
                    if outcome.hard.is_empty() {
                        last_convergence_ok = true;
                    } else {
                        last_convergence_ok = false;
                        recovery_failures.extend(outcome.hard);
                    }
                }
                Err(e) => {
                    recovery_failures.push(format!("recovery checkpoint error: {e}"));
                }
            }
            next_crash = now + config.crash_interval.unwrap_or(config.steady_interval);
        } else {
            phase = "steady";
            match steady_checkpoint(&supervisor, &model, &jwt_secret, config, &paused).await {
                Ok((hard, pending)) => {
                    steady_checkpoints += 1;
                    pending_gates.extend(pending);
                    if hard.is_empty() {
                        last_convergence_ok = true;
                    } else {
                        last_convergence_ok = false;
                        convergence_failures.extend(hard);
                    }
                }
                Err(e) => {
                    convergence_failures.push(format!("steady checkpoint error: {e}"));
                }
            }
            next_steady = now + config.steady_interval;
        }

        // Progress snapshot for live monitoring of long runs.
        let peak = *peak_rss.lock();
        let last = samples.lock().last().map_or(0.0, |s| s.rss_mb);
        if let Some(path) = &config.progress_output {
            append_progress(
                path,
                &ProgressSnapshot {
                    timestamp: utc_timestamp_now(),
                    elapsed_secs: start.elapsed().as_secs(),
                    phase: phase.to_string(),
                    total_writes: metrics.total_writes.load(Ordering::Relaxed),
                    write_errors: metrics.write_errors.load(Ordering::Relaxed),
                    reconnects: metrics.reconnects.load(Ordering::Relaxed),
                    crashes,
                    steady_checkpoints,
                    recovery_checkpoints,
                    last_convergence_ok,
                    peak_rss_mb: peak,
                    last_rss_mb: last,
                    panics_seen: panic_watch.tripped(),
                    confirms: metrics.confirms.load(Ordering::Relaxed),
                    last_confirmed_epoch: metrics.last_confirmed_epoch.load(Ordering::Relaxed),
                    confirm_errors: metrics.confirm_errors.load(Ordering::Relaxed),
                },
            );
        }
        println!(
            "[{:>6}s] {phase:<8} writes={} errs={} reconnects={} crashes={} steady={} recovery={} \
             converged={} confirms={} lastEpoch={} confirmErrs={} rss={:.0}MB(peak {:.0})",
            start.elapsed().as_secs(),
            metrics.total_writes.load(Ordering::Relaxed),
            metrics.write_errors.load(Ordering::Relaxed),
            metrics.reconnects.load(Ordering::Relaxed),
            crashes,
            steady_checkpoints,
            recovery_checkpoints,
            last_convergence_ok,
            metrics.confirms.load(Ordering::Relaxed),
            metrics.last_confirmed_epoch.load(Ordering::Relaxed),
            metrics.confirm_errors.load(Ordering::Relaxed),
            last,
            peak,
        );

        // Fail fast: a real divergence/recovery miss IS the finding.
        if !convergence_failures.is_empty() {
            finished_reason = "convergence divergence detected".to_string();
            break;
        }
        if !recovery_failures.is_empty() {
            finished_reason = "crash recovery mismatch detected".to_string();
            break;
        }
        if panic_watch.tripped() {
            finished_reason = "server panic detected".to_string();
            break;
        }
    }

    // --- Tear down ---
    // Q2: bracket the shutdown-drain window by measuring the WAL dir on disk just
    // before we stop and again after the drain completes, so the report can
    // attribute the end-of-run WAL burst (Run B saw 84 -> 222 MB in the final
    // ~60s). Gated on the opt-in measurement mode so a normal run pays nothing.
    let wal_dir = data_dir.join("wal");
    let wal_mb_before_drain = if config.mechanism_report {
        sample_disk_mb(&wal_dir)
    } else {
        None
    };

    stop.store(true, Ordering::SeqCst);
    paused.store(false, Ordering::SeqCst);
    for h in churn_handles {
        let _ = tokio::time::timeout(Duration::from_secs(5), h).await;
    }
    supervisor.shutdown().await;

    let wal_mb_after_drain = if config.mechanism_report {
        sample_disk_mb(&wal_dir)
    } else {
        None
    };

    // The TERMINAL corpus scan: the server process has just been reaped, so its
    // redb handle is released and this scan can safely open the same file
    // directly — no copy, because nothing boots after it. It contributes the
    // final sample of the durable-corpus series, which feeds the instrument,
    // level and ceiling clauses rendered below. Its cross-check against the
    // gauge's
    // `last_bytes` survives on the same rendered line — see
    // `scan_redb_tombstone_corpus`'s doc for why a divergence there is exactly
    // the failure mode this exists to catch.
    let redb_tombstone_census = scan_redb_tombstone_corpus(&data_dir, OR_MAP);
    // The summary line, the estimator and the mechanism report all consume the
    // same byte total they consumed before the census existed: the census is
    // strictly additive to what this scan returns, and this adapter is the whole
    // of the difference.
    let redb_tombstone_scan = redb_tombstone_census.map(|c| c.tombstone_bytes);
    corpus_sampler.record_terminal_scan(
        boot_gap_clock.sampler_start.elapsed().as_secs_f64(),
        redb_tombstone_census,
    );

    // --- Assess memory (secondary/backstop gate) ---
    let mem_samples = samples.lock().clone();
    // The durable reading's RSS series, derived from the memory gate's OWN
    // stream. The round trip is EXACT: `ps` reports integer KiB and the sampler
    // divided it by 1024, so multiplying back moves the exponent and leaves the
    // mantissa alone. Deriving it here — rather than teaching the sampler to
    // keep KiB — is what makes "the durable reading's RSS series IS the memory
    // gate's series" a structural fact instead of an intention.
    if config.durable_reading {
        durable
            .rss_kib
            .lock()
            .extend(mem_samples.iter().map(|s| SeriesPoint {
                elapsed_secs: s.elapsed_secs,
                value: (s.rss_mb * 1024.0) as u64,
            }));
    }
    let mem = assess(
        &mem_samples,
        config.mem_threshold_mb_per_hour,
        config.mem_min_growth_mb,
        config.mem_ceiling_mb,
    );

    // --- Assess tombstone-byte growth (direct residency-independent leak
    // instrument, the bounded-plateau signal — a HARD gate in every run
    // class, see the note below).
    // Coexists with the RSS gate above as a coarse non-tombstone
    // backstop — neither replaces the other. The sampling loop already
    // excluded boot-recompute-gap samples (R9(c)), so this series is safe to
    // fit directly.
    let tombstone_samples_snapshot = tombstone_samples.lock().clone();
    let tombstones = assess_tombstone_bytes(
        &tombstone_samples_snapshot,
        DEFAULT_TOMBSTONE_BYTES_THRESHOLD_PER_HOUR,
        DEFAULT_TOMBSTONE_BYTES_MIN_GROWTH,
        DEFAULT_TOMBSTONE_BYTES_MIN_WINDOW_SECS,
    );

    // The tombstone-byte SLOPE clause is an UNCONDITIONAL HARD gate: it
    // decides the verdict in every run class, and the durable-corpus level
    // clause below takes none of them over, because that clause is
    // report-only. The tracked-and-ACKing client spawned above
    // (`run_tracked_confirm_client` / `TRACKER_IDX`) drives the server's
    // per-device causal frontier forward every `confirm_interval`, so the
    // fleet-wide low-water-mark actually advances and the epoch-scoped prune
    // fires — the exported gauge (`topgun_ormap_tombstone_bytes`) is
    // decrementable and is expected to genuinely plateau under sustained churn,
    // not merely climb at the tombstone-creation rate. Two guards keep this
    // from false-REDing:
    //   - the min-window-span guard (`DEFAULT_TOMBSTONE_BYTES_MIN_WINDOW_SECS`)
    //     suppresses the slope clause until the last-half fit window covers
    //     enough wall-clock time for a per-hour extrapolation to mean anything
    //     — this is what keeps the 25s blocking Soak Smoke G4b run green even
    //     though its keyspace has not yet had time to plateau;
    //   - boot-recompute-gap exclusion (unchanged) keeps a spurious
    //     post-restart pre-reconcile read from manufacturing a false leak.
    // RSS above remains a coarse, non-tombstone backstop (its large min-growth
    // guard cannot catch a single-digit-MB/KB tombstone leak on a bounded run).
    // `--no-ack` and `--inject-slow-leak` (see their doc comments on `Config`)
    // are the negative/slow-leak control modes that prove this gate can
    // actually fail and actually catches a small sustained leak, not only a
    // total blockage.
    //
    // The blind-monitor guard hard-gates independently of the slope clause:
    // zero samples means the `/metrics` scrape was dead — a real harness
    // defect independent of the leak magnitude, so a run that monitored
    // nothing must not pass.
    let blind_monitor = tombstones.samples == 0;

    // --- Assess the DURABLE tombstone corpus: the level/ceiling instrument
    // that reads the on-disk corpus itself rather than the exported gauge.
    // Its clauses are REPORT-ONLY and none of them is ANDed into the run
    // verdict below. The INSTRUMENT clause flags a run whose sampler could not
    // obtain the state at some checkpoint — a blind instrument must never be
    // read as bounded growth. The LEVEL clause compares the peak of the
    // series' recent half against the peak of its earlier half, and decides
    // only on a series long enough for that comparison to mean anything (the
    // sample and span guards). A live control cell with no fault injected
    // breached the level clause on its own, so the clause is not yet able to
    // separate a leak from ordinary growth and must not fail honest runs; the
    // byte slope above therefore keeps gating every run class, and no run
    // configuration is left ungated.
    let corpus_snapshot = corpus_sampler.snapshot();
    let corpus = assess_tombstone_corpus_level(
        &corpus_snapshot.samples,
        corpus_snapshot.scans_attempted,
        corpus_snapshot.scans_failed,
        config.tombstone_corpus_headroom_bytes,
        DEFAULT_TOMBSTONE_CORPUS_MIN_SPAN_SECS,
        DEFAULT_TOMBSTONE_CORPUS_MIN_SAMPLES,
        config.tombstone_corpus_ceiling_bytes,
    );

    // --- Assess disk growth (durable-dir footprint; catches leaks RSS cannot
    // see, e.g. lazy-loaded records that never touch the in-memory cache).
    // Mirrors the tombstone-byte gate exactly: the SLOPE is report-only (the
    // pre-TODO-566 OR-churn leak grows the durable dir linearly by design, so
    // hard-gating the slope would RED the blocking no-crash Soak Smoke G4b
    // run — the same regression class SPEC-340 hit); only the blind-monitor
    // (zero-sample) clause hard-gates.
    let disk_samples_snapshot = disk_samples.lock().clone();
    let disk = assess_disk(
        &disk_samples_snapshot,
        DEFAULT_DISK_THRESHOLD_MB_PER_HOUR,
        DEFAULT_DISK_MIN_GROWTH_MB,
        DEFAULT_DISK_CEILING_MB,
    );
    let disk_blind_monitor = disk.samples == 0;

    let panic_report = panic_watch.report();
    // The tombstone-byte slope is the HARD gate, in every run class and with
    // no guard in front of it. The durable-corpus verdict is deliberately
    // absent from this conjunction: the control cell that injected no fault at
    // all breached the level clause on its own, which destroys attribution —
    // an instrument that reds an unperturbed run would fail honest runs rather
    // than leaking ones. It is still computed, still rendered and still
    // serialized on every run, and a breach is recorded on `pending_gates`
    // below, so demoting it hides nothing and gates nothing.
    let passed = convergence_failures.is_empty()
        && recovery_failures.is_empty()
        && mem.passed
        && !blind_monitor
        && tombstones.passed
        && !disk_blind_monitor
        && panic_report.is_none();

    // ONE ranked writer for `finished_reason`, in place of four independent
    // assignments whose last one happened to win. The precedence is decided
    // here rather than by source order: a cause unrelated to the tombstone
    // gates outranks a gate verdict, so a broken run can never read as a clean
    // gate demonstration. On a run with a single failure the
    // string is what it was before; only multi-failure runs re-order, and they
    // re-order toward the unrelated cause. Rank 3 — the durable-corpus arm —
    // writes no reason at all now that those clauses are report-only: a run
    // they cannot fail must not be handed a reason saying they failed it.
    //
    // Rank 0: an earlier phase — convergence divergence, crash-recovery
    // mismatch, server panic — already wrote a reason, so this writer does not
    // fire at all and that reason survives. The sentinel it switches on comes
    // from the source constant, never a retyped literal.
    if finished_reason == FINISHED_REASON_DURATION_REACHED {
        let ranked = if !mem.passed {
            // Rank 1.
            Some(format!(
                "memory growth assertion failed: {}",
                mem.reason.clone().unwrap_or_default()
            ))
        } else if blind_monitor {
            // Rank 2: the scrape was dead, which is a harness defect
            // independent of any leak magnitude.
            Some(format!(
                "tombstone-byte monitoring blind: {}",
                tombstones.reason.clone().unwrap_or_default()
            ))
        } else if !tombstones.passed {
            // An UNCONDITIONAL HARD gate: with the tracked-and-ACKing
            // client driving the low-water-mark, a sustained slope breach past the
            // min-window-guarded threshold means the epoch-scoped prune is not
            // keeping up with (or has stopped) bounding tombstone growth — a real
            // regression, not an expected/known gap. AND it into `passed`.
            //
            // Rank 4: the slope clause, which hard-gates every run class. The
            // guard is exactly the verdict's own term above, so the verdict and
            // the reason cannot disagree, and a passing run cannot carry a
            // failure reason.
            Some(format!(
                "tombstone-byte growth slope {:.1} bytes/h exceeds {:.1} bytes/h: {}",
                tombstones.slope_bytes_per_hour,
                DEFAULT_TOMBSTONE_BYTES_THRESHOLD_PER_HOUR,
                tombstones.reason.clone().unwrap_or_default()
            ))
        } else if disk_blind_monitor {
            // Rank 5.
            Some(format!(
                "disk monitoring blind: {}",
                disk.reason.clone().unwrap_or_default()
            ))
        } else {
            // Ranks 6-8 write no reason: the disk slope is report-only, the
            // panic report travels on its own two transports, and a run with
            // nothing to report keeps the default.
            None
        };
        if let Some(reason) = ranked {
            finished_reason = reason;
        }
    }
    if !corpus.passed {
        // The durable-corpus clauses decide nothing, so a breach is recorded
        // here instead of failing the run — the same channel the disk slope
        // below uses for an observation that is real but not yet gate-worthy.
        // Which clause to name comes from the exhaustive disposition predicate,
        // so a future fourth disposition cannot land silently on either side of
        // the split, and this block re-types no comparison of its own.
        let clause = if slope_clause_stays_hard(corpus.disposition) {
            if corpus.disposition == CorpusLevelDisposition::InstrumentFailed {
                "instrument failed"
            } else {
                "ceiling assertion failed"
            }
        } else {
            "level assertion failed"
        };
        pending_gates.push(format!(
            "durable-corpus {clause}: {} (report-only, did NOT fail the run)",
            corpus.reason.clone().unwrap_or_default()
        ));
    }
    if !disk_blind_monitor && !disk.passed {
        // Report-only, same rationale as the tombstone-byte slope above: the
        // pre-566 OR churn grows the durable dir linearly by design, so this is
        // the EXPECTED honest signal, not a regression. Do NOT AND this into
        // `passed`. Rank 6 of the precedence above, and deliberately NOT a
        // reason writer: it appends to `pending_gates` and nothing else, as at
        // HEAD.
        pending_gates.push(format!(
            "disk growth slope {:.1} MB/h exceeds {:.1} MB/h — EXPECTED until \
             TODO-566 bounds OR-Map tombstones (report-only, did NOT fail the run)",
            disk.slope_mb_per_hour, DEFAULT_DISK_THRESHOLD_MB_PER_HOUR
        ));
    }
    if let Some(pr) = &panic_report {
        if passed {
            // unreachable, but keep finished_reason informative
        }
        eprintln!("PANIC CONTEXT:\n{pr}");
    }

    // --- Durable-layer reading (report-only) ---
    // Assembled BEFORE the report literal because `pending_gates` is the ONLY
    // channel this reading has: nothing it produces is ANDed into `passed`, and
    // `report.rs` is deliberately given no field for it, so the durable reading
    // ships as a sibling artifact instead. Armed runs only — an unflagged run
    // builds nothing here and writes no sibling file.
    let durable_report = if config.durable_reading {
        // ONE read of the effective child log filter, reused for both the
        // reported value and the arming derivation, so the filter an artifact
        // reports and the filter its `armed` flag was derived from cannot
        // disagree.
        let log_filter = process::effective_server_log_filter();
        // Checkpoint and terminal censuses, plus the observation-only live-copy
        // records, in clock order. The live records travel in their own tally
        // right up to this point, so the durable-corpus estimator's input above
        // is identical whether the live sampler was armed or not.
        let mut census_records = corpus_snapshot.censuses.clone();
        census_records.extend(corpus_sampler.live_tally.lock().censuses.iter().copied());
        census_records.sort_by(|a, b| a.elapsed_secs.total_cmp(&b.elapsed_secs));
        let origin_snapshot = supervisor.origin_capture().snapshot();
        let (reading, built) =
            build_durable_reading_report(&durable, &census_records, &origin_snapshot, &log_filter);
        if reading == DurableReading::IndeterminateInstrument {
            pending_gates.push(format!(
                "durable reading {}: {} (report-only, did NOT fail the run)",
                reading.as_str(),
                built.reason.clone().unwrap_or_default()
            ));
        }
        Some(built)
    } else {
        None
    };

    let report = SoakReport {
        mode: "soak".to_string(),
        duration_secs_target: config.duration.as_secs(),
        duration_secs_actual: start.elapsed().as_secs(),
        churn_clients: config.churn_clients,
        keyspace: config.keyspace,
        wal_fsync: config.wal_fsync.clone(),
        epoch_width: effective_epoch_width(),
        total_writes: metrics.total_writes.load(Ordering::Relaxed),
        write_errors: metrics.write_errors.load(Ordering::Relaxed),
        reconnects: metrics.reconnects.load(Ordering::Relaxed),
        resends: metrics.resends.load(Ordering::Relaxed),
        steady_checkpoints,
        recovery_checkpoints,
        crashes,
        convergence_failures: convergence_failures.clone(),
        recovery_failures: recovery_failures.clone(),
        pending_gates: pending_gates.clone(),
        memory: MemoryReport {
            samples: mem.samples,
            first_mb: mem.first_mb,
            peak_mb: mem.peak_mb,
            last_mb: mem.last_mb,
            slope_mb_per_hour: mem.slope_mb_per_hour,
            passed: mem.passed,
            reason: mem.reason.clone(),
        },
        tombstones: TombstoneReport {
            samples: tombstones.samples,
            first_bytes: tombstones.first_bytes,
            peak_bytes: tombstones.peak_bytes,
            last_bytes: tombstones.last_bytes,
            slope_bytes_per_hour: tombstones.slope_bytes_per_hour,
            passed: tombstones.passed,
            reason: tombstones.reason.clone(),
        },
        tombstone_corpus: TombstoneCorpusReport {
            scans_attempted: corpus.scans_attempted,
            scans_failed: corpus.scans_failed,
            samples: corpus.samples,
            first_bytes: corpus.first_bytes,
            min_bytes: corpus.min_bytes,
            peak_bytes: corpus.peak_bytes,
            last_bytes: corpus.last_bytes,
            first_half_peak_bytes: corpus.first_half_peak_bytes,
            last_half_peak_bytes: corpus.last_half_peak_bytes,
            rise_bytes: corpus.rise_bytes,
            span_secs: corpus.span_secs,
            disposition: corpus.disposition,
            ceiling_bytes: corpus.ceiling_bytes,
            passed: corpus.passed,
            reason: corpus.reason.clone(),
        },
        disk: DiskReport {
            samples: disk.samples,
            first_mb: disk.first_mb,
            peak_mb: disk.peak_mb,
            last_mb: disk.last_mb,
            slope_mb_per_hour: disk.slope_mb_per_hour,
            passed: disk.passed,
            reason: disk.reason.clone(),
        },
        confirm_apply: ConfirmApplyReport {
            confirms: metrics.confirms.load(Ordering::Relaxed),
            last_confirmed_epoch: metrics.last_confirmed_epoch.load(Ordering::Relaxed),
            confirm_errors: metrics.confirm_errors.load(Ordering::Relaxed),
        },
        panic_report,
        passed,
        finished_reason: finished_reason.clone(),
        timestamp: utc_timestamp_now(),
    };

    print_summary(
        &report,
        &tombstones,
        &disk,
        &corpus,
        config.tombstone_corpus_headroom_bytes,
        redb_tombstone_scan,
    );
    if let Some(path) = &config.json_output {
        write_report(path, &report);
        println!("wrote JSON report to {}", path.display());
    }

    // --- Mechanism report (R1: Q1-Q4, report-only) ---
    // Built from the retained WAL frame scan plus the RSS/disk slopes and the
    // independent tombstone corpus scan already computed above. REPORT-ONLY:
    // it is printed (and serialized next to the JSON report when set) but never
    // asserted into `passed`, so it cannot false-RED the bounded CI smoke.
    if config.mechanism_report {
        if let Some(wal) = scan_wal_frame_sizes(&wal_dir) {
            let mechanism = build_mechanism_report(&MechanismInputs {
                wal: &wal,
                rss_samples: &mem_samples,
                disk_samples: &disk_samples_snapshot,
                rss_slope_mb_per_hour: mem.slope_mb_per_hour,
                disk_slope_mb_per_hour: disk.slope_mb_per_hour,
                tombstone_corpus_bytes: redb_tombstone_scan,
                tombstone_gauge_bytes: tombstones.last_bytes,
                wal_mb_before_drain,
                wal_mb_after_drain,
            });
            print_mechanism_report(&mechanism);
            if let Some(path) = &config.json_output {
                let mech_path = path.with_extension("mechanism.json");
                match std::fs::File::create(&mech_path) {
                    Ok(f) => {
                        if let Err(e) = serde_json::to_writer_pretty(f, &mechanism) {
                            eprintln!("mechanism report write failed: {e}");
                        } else {
                            println!("wrote mechanism report to {}", mech_path.display());
                        }
                    }
                    Err(e) => {
                        eprintln!(
                            "mechanism report create failed for {}: {e}",
                            mech_path.display()
                        );
                    }
                }
            }
        } else {
            eprintln!(
                "mechanism report requested but WAL dir {} could not be scanned",
                wal_dir.display()
            );
        }
    }

    // --- Durable-layer reading artifact (report-only) ---
    // A SIBLING of the primary JSON report, written the way the mechanism
    // report already is, because adding a field to `SoakReport` would cascade
    // into the struct literal the WAL-census integration target builds.
    if let Some(durable_report) = &durable_report {
        assert_scrape_counter_identity(durable_report);
        print_durable_reading_report(durable_report);
        if let Some(path) = &config.json_output {
            let durable_path = path.with_extension("durable.json");
            match std::fs::File::create(&durable_path) {
                Ok(f) => {
                    if let Err(e) = serde_json::to_writer_pretty(f, durable_report) {
                        eprintln!("durable reading write failed: {e}");
                    } else {
                        println!("wrote durable reading to {}", durable_path.display());
                    }
                }
                Err(e) => {
                    eprintln!(
                        "durable reading create failed for {}: {e}",
                        durable_path.display()
                    );
                }
            }
        }
    }
    i32::from(!passed)
}

fn print_summary(
    r: &SoakReport,
    tombstones: &TombstoneAssessment,
    disk: &DiskAssessment,
    corpus: &TombstoneCorpusAssessment,
    corpus_headroom_bytes: u64,
    redb_tombstone_scan: Option<u64>,
) {
    println!("\n=== SOAK SUMMARY ===");
    println!(
        "result:            {}",
        if r.passed { "PASS" } else { "FAIL" }
    );
    println!("finished_reason:   {}", r.finished_reason);
    println!("actual_duration:   {}s", r.duration_secs_actual);
    println!("total_writes:      {}", r.total_writes);
    println!("write_errors:      {}", r.write_errors);
    println!("reconnects:        {}", r.reconnects);
    println!("resends:           {}", r.resends);
    println!(
        "confirm_apply:     confirms={} lastEpoch={} errors={} \
         (tracked client advances the low-water-mark that licenses pruning; \
         confirms=0 with a climbing gauge = LWM never advanced)",
        r.confirm_apply.confirms,
        r.confirm_apply.last_confirmed_epoch,
        r.confirm_apply.confirm_errors
    );
    println!("steady_checkpts:   {}", r.steady_checkpoints);
    println!(
        "recovery_checkpts: {} (crashes {})",
        r.recovery_checkpoints, r.crashes
    );
    println!(
        "memory:            first={:.0}MB peak={:.0}MB last={:.0}MB slope={:.1}MB/h -> {} (backstop)",
        r.memory.first_mb,
        r.memory.peak_mb,
        r.memory.last_mb,
        r.memory.slope_mb_per_hour,
        if r.memory.passed { "ok" } else { "FAIL" }
    );
    // The byte SLOPE is a HARD gate in every run class; the durable-corpus
    // level clause beside it is report-only and takes none of them over. The
    // tracked-and-ACKing client drives
    // the low-water-mark forward, so the epoch-scoped prune actually fires and
    // the gauge is expected to plateau under sustained churn (subject to the
    // min-window-span guard and boot-gap exclusion). See the run-end verdict
    // rationale.
    let tombstone_role =
        "slope + blind-monitor both hard-gate; durable-corpus clauses are report-only";
    println!(
        "tombstone_bytes:   first={} peak={} last={} slope={:.1}B/h samples={} -> {} ({}){}",
        tombstones.first_bytes,
        tombstones.peak_bytes,
        tombstones.last_bytes,
        tombstones.slope_bytes_per_hour,
        tombstones.samples,
        if tombstones.passed { "ok" } else { "FAIL" },
        tombstone_role,
        tombstones
            .reason
            .as_ref()
            .map_or_else(String::new, |r| format!(" reason={r}")),
    );
    // The durable-corpus gate's own verdict line, rendered under the single
    // prefix constant so no site retypes it. Its content and meaning changed
    // with the gate: it was a report-only positive-control cross-check against
    // the gauge, and it now carries the gate's verdict, the clause that decided
    // it, the pre-registered series aggregates and both knob values. The
    // gauge cross-check rides along on the same line — a divergence from
    // `tombstones.last_bytes` still means the exported gauge is not tracking
    // the true corpus (e.g. an un-migrated legacy `OrTombstones` blob the
    // hot-path gauge never added on read).
    let corpus_disposition = match corpus.disposition {
        // A suppressed clause is rendered WITH the two numbers that suppressed
        // it, never as a pass: a clause that did not decide must not be
        // indistinguishable from one that decided and was satisfied.
        CorpusLevelDisposition::LevelSuppressed => format!(
            "{}(n={}, span={:.0}s)",
            corpus.disposition.as_str(),
            corpus.samples,
            corpus.span_secs
        ),
        // Every other disposition renders its bare token, taken from the same
        // `as_str` the JSON report serializes through so the console and the
        // artifact cannot disagree.
        _ => corpus.disposition.as_str().to_string(),
    };
    // A disarmed ceiling renders as the word, never as an absent token:
    // "disarmed" and "armed at n" have to be distinguishable on the line.
    let corpus_ceiling = corpus
        .ceiling_bytes
        .map_or_else(|| "disarmed".to_string(), |c| c.to_string());
    let corpus_reason = corpus
        .reason
        .as_ref()
        .map_or_else(String::new, |r| format!(" reason={r}"));
    let corpus_tokens = format!(
        "first={} min={} peak={} last={} first_half_peak={} last_half_peak={} rise={} \
         span={:.0}s scans_ok={} scans_failed={} headroom={} ceiling={} disposition={} -> {}",
        corpus.first_bytes,
        corpus.min_bytes,
        corpus.peak_bytes,
        corpus.last_bytes,
        corpus.first_half_peak_bytes,
        corpus.last_half_peak_bytes,
        corpus.rise_bytes,
        corpus.span_secs,
        corpus.scans_attempted.saturating_sub(corpus.scans_failed),
        corpus.scans_failed,
        corpus_headroom_bytes,
        corpus_ceiling,
        corpus_disposition,
        if corpus.passed { "ok" } else { "FAIL" },
    );
    match redb_tombstone_scan {
        Some(scanned_bytes) => {
            let gauge_bytes = tombstones.last_bytes;
            let diverged = scanned_bytes != gauge_bytes;
            println!(
                "{TOMBSTONE_CORPUS_LINE_PREFIX} {corpus_tokens} (terminal scan {scanned_bytes} \
                 bytes; last gauge value {gauge_bytes} bytes -> {}){corpus_reason}",
                if diverged { "DIVERGED" } else { "match" }
            );
        }
        None => {
            println!(
                "{TOMBSTONE_CORPUS_LINE_PREFIX} {corpus_tokens} (terminal scan could not read the \
                 durable corpus — counted as a failed scan){corpus_reason}"
            );
        }
    }
    // Unlike the tombstone-byte slope above (a hard gate in every run
    // class), the disk slope
    // stays REPORT-ONLY: it is EXPECTED to breach pre-TODO-566 under default
    // OR-churn (linear durable-dir growth by design, independent of the
    // tombstone prune this spec drives); only the blind-monitor (zero-sample)
    // clause hard-gates.
    let disk_role = "slope report-only until TODO-566; blind-monitor hard-gates";
    println!(
        "disk_mb:           first={:.1} peak={:.1} last={:.1} slope={:.1}MB/h samples={} -> {} ({}){}",
        disk.first_mb,
        disk.peak_mb,
        disk.last_mb,
        disk.slope_mb_per_hour,
        disk.samples,
        if disk.passed { "ok" } else { "FAIL" },
        disk_role,
        disk.reason
            .as_ref()
            .map_or_else(String::new, |r| format!(" reason={r}")),
    );
    if !r.convergence_failures.is_empty() {
        println!("convergence_failures:");
        for f in r.convergence_failures.iter().take(10) {
            println!("  - {f}");
        }
    }
    if !r.recovery_failures.is_empty() {
        println!("recovery_failures:");
        for f in r.recovery_failures.iter().take(10) {
            println!("  - {f}");
        }
    }
    if !r.pending_gates.is_empty() {
        println!("pending_gates (expected-fail, did NOT fail the run):");
        for f in r.pending_gates.iter().take(10) {
            println!("  - {f}");
        }
    }
    if let Some(pr) = &r.panic_report {
        println!("panic_report:      {pr}");
    }
}

// ---------------------------------------------------------------------------
// Tombstone-bytes gauge sampling
// ---------------------------------------------------------------------------

/// Scrape `topgun_ormap_tombstone_bytes` — the DECREMENTABLE gauge, not the
/// `_total` monotonic creation-rate counter — from the real running server's
/// `GET /metrics` (Prometheus text exposition format). The plateau/slope signal
/// this harness gates on MUST come from the decrementable gauge: the `_total`
/// counter only ever grows (every add, never subtracted on prune), so fitting
/// a slope against it would always look like an unbounded leak regardless of
/// whether pruning is actually keeping tombstone residency bounded. Returns
/// `None` on any transient failure — connection refused (server mid-restart),
/// non-2xx status, an unreadable body, or the metric line simply not being
/// present — so the caller skips the sample instead of recording a bogus
/// point. This mirrors `sample_rss_mb`'s `None`-on-gone-process contract in
/// `monitor.rs`.
async fn scrape_tombstone_bytes(http: &reqwest::Client, port: u16) -> Option<MetricsScrape> {
    const WRITEBEHIND_LAG_METRIC: &str = "topgun_wal_applied_watermark_lag";
    const EPOCHS_EXITED_METRIC: &str = "topgun_or_prune_epochs_exited_total";

    let url = format!("http://127.0.0.1:{port}/metrics");
    let resp = http.get(&url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body = resp.text().await.ok()?;
    Some(MetricsScrape {
        tombstone_bytes: parse_tombstone_bytes_gauge(&body),
        // Handed on WHOLE, unnarrowed: which statistic each column folds is
        // stated at the fold site, and the two no-value readings are distinct
        // instrument faults that are counted apart there. Collapsing either
        // decision into this site would throw away the fact the fold needs.
        writebehind_lag: parse_labelled_gauge(&body, WRITEBEHIND_LAG_METRIC),
        epochs_exited: parse_labelled_gauge(&body, EPOCHS_EXITED_METRIC),
    })
}

/// Fold ONE scrape's reading into a run-scoped observation column.
///
/// Counting is delegated to [`GaugeObservation::record`] — the single place the
/// six counters move, so no call site can drift the scrape identity apart. What
/// this function decides is the other half `record` deliberately leaves to its
/// caller: how a run's MANY scrapes combine into ONE observed value. The rule
/// is a RUNNING MAXIMUM over the per-scrape folded values, for both columns —
/// the lag column reports the worst moment of the run rather than whatever the
/// last scrape happened to catch, and the exited-epoch column must be a maximum
/// rather than a last value because the counter restarts with the process.
///
/// A scrape that read nothing leaves the value exactly as it stood, so a column
/// no scrape ever read stays `None`: the combine can never launder an absence
/// into a zero, which is the one way this hop could re-create the masquerade
/// the three-valued reading exists to refuse.
fn fold_scrape_into(column: &mut GaugeObservation, reading: &GaugeReading, fold: GaugeFold) {
    if let Some(scraped) = column.record(reading, fold) {
        column.value = Some(match column.value {
            Some(highest) => highest.max(scraped),
            None => scraped,
        });
    }
}

/// The three quantities one `/metrics` response body yields.
///
/// Read from ONE body rather than from three requests, so the widening costs no
/// extra traffic and the three values describe the same instant. No field can
/// report a zero it did not read: `tombstone_bytes` is `None` where its gauge
/// was absent, and the two observed gauges carry the whole three-valued
/// [`GaugeReading`], which keeps "never appeared" and "appeared and nothing
/// read" apart all the way to the fold. Only `tombstone_bytes` feeds a gate;
/// the other two are OBSERVATION ONLY.
///
/// The two gauge fields are named for their METRIC, not for a statistic, because
/// the statistic is no longer chosen here — the fold site names it, and a field
/// called `_max` holding an unfolded reading would invite exactly the silent
/// transposition the named fold exists to prevent.
struct MetricsScrape {
    tombstone_bytes: Option<u64>,
    writebehind_lag: GaugeReading,
    epochs_exited: GaugeReading,
}

/// Parse the `topgun_ormap_tombstone_bytes` (decrementable gauge) sample value
/// out of a Prometheus text exposition body, skipping `# HELP`/`# TYPE` comment
/// lines and any blank lines. A metric line is `name value` or
/// `name{labels} value` (space-separated); this gauge carries no labels today,
/// but the label-form prefix match keeps the parser correct if one is ever
/// added. Exact-name equality (not a bare prefix match) is what keeps this from
/// also matching the co-resident `topgun_ormap_tombstone_bytes_total` counter
/// line the same `/metrics` response carries.
fn parse_tombstone_bytes_gauge(body: &str) -> Option<u64> {
    const METRIC: &str = "topgun_ormap_tombstone_bytes";
    // Compute the labelled-series prefix once rather than allocating a fresh
    // `String` per scanned line in the hot scrape loop.
    let labelled_prefix = format!("{METRIC}{{");
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let name = parts.next()?;
        if name == METRIC || name.starts_with(&labelled_prefix) {
            let value = parts.next()?;
            // The gauge is logically u64 bytes. Parse as u64 first (exact for a
            // whole-number counter); fall back to f64-then-truncate only for the
            // trailing ".0" float rendering Prometheus may emit — avoids the f64
            // precision loss a `u64` value above 2^53 would otherwise suffer.
            return value
                .parse::<u64>()
                .ok()
                .or_else(|| value.parse::<f64>().ok().map(|f| f as u64));
        }
    }
    None
}

/// Durable-corpus series accumulated across a run, plus the two scan counters
/// the instrument clause is decided on.
///
/// `scans_attempted` is incremented ONCE per checkpoint. A checkpoint that
/// could not obtain the state — whether the byte copy failed or the scan
/// itself returned `None` — increments `scans_failed` and pushes no sample:
/// an instrument that was blind is blind whichever step failed, and there is
/// no third outcome and no silent skip.
#[derive(Debug, Default, Clone)]
struct CorpusScanTally {
    samples: Vec<CorpusSample>,
    scans_attempted: usize,
    scans_failed: usize,
    /// The census taken at each of those same scans. Strictly ADDITIVE: it is
    /// derived from the scan the three fields above were already counting, and
    /// it never changes which scans are attempted or which of them failed, so
    /// the estimator's input is unchanged in value as well as in type.
    censuses: Vec<CensusRecord>,
}

/// The live-copy census sampler's own tally, deliberately SEPARATE from
/// [`CorpusScanTally`].
///
/// A byte copy of a LIVE store file is a smeared image, so the census it yields
/// is best-effort by construction and OBSERVATION ONLY. Keeping those records
/// here — never on the corpus-scan tally's `samples` / `scans_attempted` /
/// `scans_failed` — is what makes the durable-corpus estimator's input
/// identical whether this sampler is armed or not, rather than merely intended
/// to be. The fence is a typed property of the record, not a convention: every
/// record carries its own source.
#[derive(Debug, Default, Clone)]
struct LiveCensusTally {
    censuses: Vec<CensusRecord>,
    scans_attempted: usize,
    scans_failed: usize,
}

/// The durable-corpus sampler's per-run state, bundled the way the boot-gap
/// clock above is: the recovery checkpoint takes one parameter for it instead
/// of three. It holds the server's data dir (the copy's SOURCE), the scratch
/// root the copies are written to — deliberately OUTSIDE the data dir — and
/// the shared sink the samples and counters land in.
struct CorpusSampler {
    data_dir: PathBuf,
    scratch_root: PathBuf,
    tally: Mutex<CorpusScanTally>,
    /// The observation-only sink the live-copy sampler folds into. A second
    /// tally rather than a flag on the first one, so no code path can add a
    /// smeared-image record to the series a gate's estimator reads.
    live_tally: Mutex<LiveCensusTally>,
}

impl CorpusSampler {
    fn new(data_dir: PathBuf, scratch_root: PathBuf) -> Self {
        Self {
            data_dir,
            scratch_root,
            tally: Mutex::new(CorpusScanTally::default()),
            live_tally: Mutex::new(LiveCensusTally::default()),
        }
    }

    /// The accumulated series and counters, as the estimator consumes them.
    fn snapshot(&self) -> CorpusScanTally {
        self.tally.lock().clone()
    }

    /// Fold the TERMINAL scan onto the same two counters as the checkpoint
    /// samples. That scan is the one siting that reads the data dir directly
    /// and takes no copy: it runs post-teardown, nothing boots after it, so
    /// there is nothing left for the instrument to stay neutral toward.
    fn record_terminal_scan(&self, elapsed_secs: f64, scanned: Option<DurableCensus>) {
        let mut t = self.tally.lock();
        t.scans_attempted += 1;
        if let Some(census) = scanned {
            t.censuses.push(CensusRecord {
                elapsed_secs,
                // PLACEHOLDER(g1): which instant fills this is decided where the
                // copy window is actually measured. The terminal scan takes no
                // copy at all, so an honest value here may well stay `None`.
                copy_completed_secs: None,
                source: CensusSource::Terminal,
                census,
            });
        }
        // The estimator keeps consuming the same summation over the same two
        // variants it consumed before the census existed; this adapter is the
        // whole of the difference.
        match scanned.map(|c| c.tombstone_bytes) {
            Some(bytes) => t.samples.push(CorpusSample {
                elapsed_secs,
                bytes,
            }),
            None => t.scans_failed += 1,
        }
    }
}

/// Take one durable-corpus sample while the server process is dead, WITHOUT
/// letting the instrument touch the file the server is about to recover from.
///
/// The server's redb file is byte-copied to a per-checkpoint scratch directory
/// outside the data dir, the COPY is scanned, and the copy is deleted before
/// the caller restarts the server. `redb::Database::open` repairs and commits
/// an uncleanly-closed file, so scanning the original here would run the
/// server's own recovery ahead of the server and perturb the input of the
/// crash-recovery gate; scanning a copy leaves that gate measuring exactly
/// what it measured before this sampler existed. The scratch directory also
/// lives outside the data dir because the disk gate's input is `du -sk` over
/// the data dir, and a copy written inside it would step that gate too.
///
/// The scratch path carries the checkpoint ordinal, so two samples can never
/// alias and a delete that failed cannot leave a previous sample's bytes to be
/// re-scanned as this one's. A failed DELETE is not a scan failure — the
/// sample was obtained — so it is logged and the run continues; the teardown
/// sweep removes the residue.
fn sample_durable_corpus_via_copy(sampler: &CorpusSampler, elapsed_secs: f64) {
    // One attempt per checkpoint, counted before anything can fail, so a
    // failure can never go unattributed.
    let ordinal = {
        let mut t = sampler.tally.lock();
        t.scans_attempted += 1;
        t.scans_attempted
    };
    let scanned = copy_and_scan_census(sampler, "durable-corpus", ordinal);

    let mut t = sampler.tally.lock();
    if let Some(census) = scanned {
        t.censuses.push(CensusRecord {
            elapsed_secs,
            // PLACEHOLDER(g1): the copy window is measured where the copy runs.
            copy_completed_secs: None,
            source: CensusSource::Checkpoint,
            census,
        });
    }
    // The estimator keeps consuming the same summation over the same two
    // variants it consumed before the census existed; this adapter is the whole
    // of the difference.
    match scanned.map(|c| c.tombstone_bytes) {
        Some(bytes) => t.samples.push(CorpusSample {
            elapsed_secs,
            bytes,
        }),
        None => t.scans_failed += 1,
    }
}

/// Take one OBSERVATION-ONLY census from a byte copy of the LIVE store file.
///
/// The copy-then-scan discipline is the checkpoint sampler's, for the same two
/// reasons: the copy lives outside the data dir so the disk gate's `du -sk`
/// input is not stepped, and the ORIGINAL is never opened, so this harness
/// never runs the server's own recovery ahead of the server.
///
/// What differs is what the result may DECIDE. The server is running here, so
/// the copy is a smeared image of a file being written underneath it and the
/// census it yields is best-effort by construction. It therefore lands in the
/// live tally and NEVER on the corpus-scan tally the durable-corpus estimator
/// reads — including its `scans_attempted` / `scans_failed` counters, so an
/// armed live sampler cannot even move that estimator's instrument clause.
fn sample_live_census_via_copy(sampler: &CorpusSampler, elapsed_secs: f64) {
    let ordinal = {
        let mut t = sampler.live_tally.lock();
        t.scans_attempted += 1;
        t.scans_attempted
    };
    let scanned = copy_and_scan_census(sampler, "live-census", ordinal);

    let mut t = sampler.live_tally.lock();
    match scanned {
        Some(census) => t.censuses.push(CensusRecord {
            elapsed_secs,
            // PLACEHOLDER(g1): the copy window is measured where the copy runs.
            copy_completed_secs: None,
            source: CensusSource::LiveCopy,
            census,
        }),
        None => t.scans_failed += 1,
    }
}

/// Byte-copy the server's store file into an ordinal-scoped scratch directory,
/// scan the COPY, delete it, and return the census. `None` if the copy or the
/// scan failed — the caller owns what a failure means for its own tally.
///
/// A COPY, never the original: `redb::Database::open` repairs and commits an
/// uncleanly-closed file, so opening the server's own file here would make this
/// harness perform the server's crash recovery ahead of the server — and the
/// server's crash recovery is precisely what the run's recovery gate measures.
/// The scratch root also lives outside the data dir, because the disk gate's
/// input is `du -sk` over that dir and a copy written inside it would step that
/// gate too.
///
/// `label` names the sampler in both the scratch path and the diagnostics, so
/// two samplers running concurrently can never alias each other's copies, and a
/// delete that failed cannot leave one sampler's bytes to be re-scanned as the
/// other's. A failed DELETE is NOT a scan failure — the census was obtained —
/// so it is logged and the run continues; the teardown sweep removes the
/// residue.
fn copy_and_scan_census(
    sampler: &CorpusSampler,
    label: &str,
    ordinal: usize,
) -> Option<DurableCensus> {
    let copy_dir = sampler.scratch_root.join(format!("{label}-{ordinal}"));
    // The scan resolves `topgun.redb` under the directory it is given, so the
    // ordinal is carried by the directory and the copy keeps the name the scan
    // looks for.
    let copy_path = copy_dir.join("topgun.redb");
    if let Err(e) = std::fs::create_dir_all(&copy_dir) {
        eprintln!(
            "{label} sample {ordinal}: cannot create scratch dir {}: {e}",
            copy_dir.display()
        );
        return None;
    }
    // `std::fs::copy` truncates an existing destination, so even an aliasing
    // bug could not produce a partial-overwrite hybrid.
    if let Err(e) = std::fs::copy(sampler.data_dir.join("topgun.redb"), &copy_path) {
        eprintln!("{label} sample {ordinal}: cannot copy redb file: {e}");
        return None;
    }

    // The scan owns and drops its redb handle inside its own body, so the
    // handle is released before the delete below and before the caller
    // restarts the server. No handle may be hoisted out of it.
    let scanned = scan_redb_tombstone_corpus(&copy_dir, OR_MAP);

    if let Err(e) = std::fs::remove_dir_all(&copy_dir) {
        eprintln!(
            "{label} sample {ordinal}: scratch copy left behind at {} ({e}) — sample \
             still counted; teardown sweeps the residue",
            copy_dir.display()
        );
    }

    scanned
}

/// Independent, gauge-free census of the OR-Map's on-disk state: the positive
/// control's cross-check that `topgun_ormap_tombstone_bytes` is actually
/// tracking the real durable byte total, not merely plateauing because it
/// drifted out of sync with it.
///
/// Returns the whole store-level census; its `tombstone_bytes` field is the
/// byte total this scan returned before the census existed, computed by the
/// same summation over the same two variants, so every caller that wants only
/// that total takes it and is unchanged in value as well as in type.
///
/// EVERY row folds through exactly ONE entry point — including a row that fails
/// to decode, which is COUNTED rather than silently skipped — which is what
/// makes `keys_scanned` equal the number of rows iterated. All the arithmetic
/// lives in the fold functions, so this site only decodes and dispatches.
///
/// Opens the server's own redb file directly (`{data_dir}/topgun.redb`, the
/// same layout `RedbDataStore` writes: table `map__{map}`, msgpack-encoded
/// `RecordValue` values) and sums the UTF-8 byte length of every tombstoned
/// tag across BOTH tombstone-carrying shapes: the live `RecordValue::OrMap`'s
/// `tombstones` field, and the legacy `RecordValue::OrTombstones` blob a
/// pre-migration server may have left on disk. The legacy shape matters here:
/// the in-process gauge's `sub_tombstone_bytes` call site lives on the CRDT
/// write path's `OR_REMOVE`/prune handling for the CURRENT `OrMap` shape only
/// — a decoded legacy `OrTombstones` blob is folded into the read-side merge
/// view but never re-adds itself to the gauge, so the gauge can silently drift
/// BELOW the true corpus (toward its saturating-0 floor) while these bytes
/// remain resident. Comparing this scan's total against the gauge's last
/// sampled value is exactly what catches that divergence.
///
/// PRECONDITION: THE CHILD PROCESS HAS BEEN REAPED. redb is single-writer, so
/// the server's own handle on the file must already be released before this
/// process can open it. `kill9` satisfies that exactly as `shutdown` does —
/// both await `child.wait()` — so this IS a live sampler: it is called once
/// per recovery checkpoint, in the window between the `kill -9` and the
/// restart, and once more terminally after teardown. What it reads is the
/// PRE-RECOVERY on-disk state, because at both sites no recovery has run yet.
///
/// CALLER OBLIGATION AT A LIVE CHECKPOINT: PASS A DIRECTORY HOLDING A BYTE
/// COPY, NEVER THE SERVER'S OWN DATA DIR. This function OPENS a redb database,
/// and `redb::Database::open` on an uncleanly-closed file REPAIRS AND COMMITS
/// it — a write. Opening the server's own file between the kill and the
/// restart would make this harness perform the server's own crash recovery,
/// and the server's crash recovery is precisely what the run's
/// `recovery_failures` gate measures; an instrument may not mutate what
/// another gate measures. So a checkpoint caller copies
/// `{data_dir}/topgun.redb` to a scratch path outside the data dir and hands
/// this function the directory holding that copy — any repair redb performs
/// happens on the copy, which is deleted immediately afterwards, and the
/// server stays the FIRST opener of the original. The terminal call is the one
/// exception, and it is stated as such: it runs post-teardown, nothing boots
/// after it, so it reads the data dir directly.
///
/// Returns `None` on any failure (file missing, corrupt header, table absent)
/// — best-effort, mirroring `sample_rss_mb`/`sample_disk_mb`'s None-on-failure
/// contract. That `None` is the durable-corpus INSTRUMENT clause's breach
/// input, and at a live checkpoint a failed COPY reaches the same clause by the
/// same path. The clause is report-only, so a blind instrument is recorded on
/// `pending_gates` rather than failing the run. An honest `Some(0)` from a
/// never-written table stays distinguishable from it.
fn scan_redb_tombstone_corpus(data_dir: &Path, map: &str) -> Option<DurableCensus> {
    let db_path = data_dir.join("topgun.redb");
    if !db_path.exists() {
        return None;
    }
    let db = redb::Database::open(&db_path).ok()?;
    let read_txn = db.begin_read().ok()?;
    let table_name = format!("map__{map}");
    let table_def: redb::TableDefinition<&str, &[u8]> = redb::TableDefinition::new(&table_name);
    let table = match read_txn.open_table(table_def) {
        Ok(t) => t,
        // Never-written table (e.g. no OR-Map write ever landed) contributes an
        // EMPTY census — a real, honest answer, not a scan failure.
        Err(redb::TableError::TableDoesNotExist(_)) => return Some(DurableCensus::default()),
        Err(_) => return None,
    };

    let mut census = DurableCensus::default();
    let iter = table.iter().ok()?;
    for entry in iter {
        let (_key_guard, val_guard) = entry.ok()?;
        let Ok(value) = rmp_serde::from_slice::<RecordValue>(val_guard.value()) else {
            // A corrupt/foreign row is COUNTED rather than silently skipped: a
            // row the instrument could not read must be visible in the census,
            // not invisible in it.
            fold_undecodable_key(&mut census);
            continue;
        };
        match value {
            RecordValue::OrMap {
                records,
                tombstones,
            } => {
                let live_tags: Vec<&str> = records.iter().map(|e| e.tag.as_str()).collect();
                let tombstone_tags: Vec<&str> = tombstones.iter().map(String::as_str).collect();
                fold_or_key(&mut census, OrVariant::OrMap, &live_tags, &tombstone_tags);
            }
            // Legacy pre-migration shape — see the function doc's divergence
            // rationale for why this is exactly what the gauge can miss. The
            // variant carries no live side at all, which is why it folds with an
            // empty one.
            RecordValue::OrTombstones { tags } => {
                let tombstone_tags: Vec<&str> = tags.iter().map(String::as_str).collect();
                fold_or_key(&mut census, OrVariant::OrTombstones, &[], &tombstone_tags);
            }
            RecordValue::Lww { .. } => fold_lww_key(&mut census),
        }
    }
    Some(census)
}

// ---------------------------------------------------------------------------
// Mechanism report (R1: Q1-Q4 attribution of OR-churn WAL + RSS growth)
// ---------------------------------------------------------------------------

/// Pearson correlation between the RSS and disk time series, matching each disk
/// sample to the RSS sample nearest in elapsed time (both share `sampler_start`,
/// but a skipped scrape can drop a point on either side). `None` if either
/// series has fewer than two matched points or is degenerate (zero variance).
fn rss_disk_correlation(rss: &[MemSample], disk: &[DiskSample]) -> Option<f64> {
    if rss.len() < 2 || disk.len() < 2 {
        return None;
    }
    let mut xs: Vec<f64> = Vec::new();
    let mut ys: Vec<f64> = Vec::new();
    for d in disk {
        let nearest = rss.iter().min_by(|a, b| {
            (a.elapsed_secs - d.elapsed_secs)
                .abs()
                .partial_cmp(&(b.elapsed_secs - d.elapsed_secs).abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })?;
        xs.push(nearest.rss_mb);
        ys.push(d.disk_mb);
    }
    let n = xs.len() as f64;
    if n < 2.0 {
        return None;
    }
    let mean_x = xs.iter().sum::<f64>() / n;
    let mean_y = ys.iter().sum::<f64>() / n;
    let mut cov = 0.0;
    let mut var_x = 0.0;
    let mut var_y = 0.0;
    for (x, y) in xs.iter().zip(&ys) {
        cov += (x - mean_x) * (y - mean_y);
        var_x += (x - mean_x).powi(2);
        var_y += (y - mean_y).powi(2);
    }
    if var_x <= f64::EPSILON || var_y <= f64::EPSILON {
        return None;
    }
    Some(cov / (var_x.sqrt() * var_y.sqrt()))
}

/// Structured mechanism report answering Q1-Q4. REPORT-ONLY: serialized to the
/// JSON output (when set) and printed to the console; never asserted into
/// `passed`.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct MechanismReport {
    // Q1 — bytes per frame, OR vs LWW, and growth over the run.
    q1_or_frame_bytes_mean_early: f64,
    q1_or_frame_bytes_mean_late: f64,
    q1_or_frame_bytes_max_early: u64,
    q1_or_frame_bytes_max_late: u64,
    q1_or_frame_bytes_max: u64,
    q1_lww_frame_bytes_mean: f64,
    q1_lww_frame_bytes_max: u64,
    q1_or_frame_size_rises: bool,
    /// The OR-vs-LWW mean ratio over BOTH OR frame kinds together. Derived and
    /// non-normative: it reads a mixture, so a surviving share of snapshot
    /// frames moves it even when every delta frame is small. Read the per-kind
    /// census below for a claim about delta framing itself.
    q1_or_vs_lww_mean_ratio: f64,
    // Per-frame-kind census — the arming evidence. `orDeltaFrames > 0` is what
    // proves the emitter was armed for a run; `orSnapshotFrames > 0` is the
    // positive control that distinguishes a disarmed emitter from a retained
    // corpus that simply held no OR frames at all.
    or_delta_frames: usize,
    or_delta_bytes_total: u64,
    or_delta_bytes_max: u64,
    or_delta_bytes_mean: f64,
    or_snapshot_frames: usize,
    or_snapshot_bytes_total: u64,
    or_snapshot_bytes_max: u64,
    or_snapshot_bytes_mean: f64,
    // Q2 — the shutdown drain-window WAL burst.
    q2_wal_mb_before_drain: Option<f64>,
    q2_wal_mb_after_drain: Option<f64>,
    q2_drain_delta_mb: Option<f64>,
    q2_top_decile_frames: usize,
    q2_top_decile_or_bytes_max: u64,
    q2_attribution: String,
    // Q3 — what holds RSS.
    q3_rss_slope_mb_per_hour: f64,
    q3_disk_slope_mb_per_hour: f64,
    q3_rss_disk_correlation: Option<f64>,
    q3_attribution: String,
    // Q4 — NOT the tombstone index.
    q4_tombstone_corpus_bytes: Option<u64>,
    q4_tombstone_gauge_bytes: u64,
    q4_or_wal_bytes_total: u64,
    q4_growth_is_or_snapshot_not_tombstone: bool,
    // Scan meta + verdict.
    wal_segment_files: usize,
    or_frames: usize,
    lww_frames: usize,
    remove_frames: usize,
    conclusion: String,
    sequencing_note: String,
}

/// Inputs the mechanism report is built from, bundled so the builder stays under
/// the argument-count lint without an `#[allow]`.
struct MechanismInputs<'a> {
    wal: &'a WalFrameStats,
    rss_samples: &'a [MemSample],
    disk_samples: &'a [DiskSample],
    rss_slope_mb_per_hour: f64,
    disk_slope_mb_per_hour: f64,
    tombstone_corpus_bytes: Option<u64>,
    tombstone_gauge_bytes: u64,
    wal_mb_before_drain: Option<f64>,
    wal_mb_after_drain: Option<f64>,
}

fn build_mechanism_report(inp: &MechanismInputs) -> MechanismReport {
    let wal = inp.wal;
    let lww_mean = if wal.lww_frames == 0 {
        0.0
    } else {
        wal.lww_bytes_total as f64 / wal.lww_frames as f64
    };
    // A 10% late-vs-early mean growth (with a non-trivial OR frame count) is the
    // threshold for calling the O(N)-per-op frame growth confirmed.
    let rises = wal.or_frames >= 20 && wal.or_bytes_mean_late > wal.or_bytes_mean_early * 1.10;
    let or_vs_lww = if lww_mean <= f64::EPSILON {
        0.0
    } else {
        (wal.or_bytes_mean_late.max(wal.or_bytes_mean_early)) / lww_mean
    };

    let drain_delta = match (inp.wal_mb_before_drain, inp.wal_mb_after_drain) {
        (Some(before), Some(after)) => Some(after - before),
        _ => None,
    };
    // Attribute the drain burst: a large delta carried by a few large OR frames in
    // the top sequence decile points at a final full-slot re-write on drain; a
    // large delta spread across many frames points at a rotation/flush storm.
    let q2_attribution = match drain_delta {
        Some(d) if d > 5.0 => {
            if wal.top_decile_or_bytes_max > 4096 && wal.top_decile_frames < 512 {
                format!(
                    "drain grew WAL by {d:.1}MB carried by a few large OR frames \
                     (top-decile max OR frame {}B across {} frames) — consistent with a \
                     final full-slot re-write on drain, which delta-framing removes",
                    wal.top_decile_or_bytes_max, wal.top_decile_frames
                )
            } else {
                format!(
                    "drain grew WAL by {d:.1}MB spread across {} top-decile frames — \
                     consistent with a rotation/flush storm on shutdown drain",
                    wal.top_decile_frames
                )
            }
        }
        Some(d) => format!("no significant drain burst (WAL delta {d:.1}MB)"),
        None => {
            "drain-window WAL size not captured (need --data-dir + --mechanism-report)".to_string()
        }
    };

    let corr = rss_disk_correlation(inp.rss_samples, inp.disk_samples);
    // Q3 branch (spec R2 rec 6): if RSS tracks disk in lockstep AND OR frames grow,
    // the growing OR blobs sit in the write-behind queue / record cache and
    // delta-framing shrinks BOTH. If RSS climbs while disk is flat, RSS is
    // record-cache-driven and delta-framing would NOT reduce it — the fix would
    // instead be compacting the resident representation.
    let q3_attribution = match corr {
        Some(c) if c > 0.8 && inp.disk_slope_mb_per_hour > 1.0 => format!(
            "RSS tracks disk in lockstep (r={c:.2}); the growing OR blobs sit in the \
             write-behind queue / record cache — delta-framing should reduce RSS too"
        ),
        Some(c) if inp.rss_slope_mb_per_hour > 1.0 && inp.disk_slope_mb_per_hour <= 1.0 => format!(
            "RSS grows ({:.1}MB/h) while disk is ~flat (r={c:.2}); RSS is record-cache-driven \
             — delta-framing alone would NOT bound RSS, compact the resident slot instead",
            inp.rss_slope_mb_per_hour
        ),
        Some(c) => format!(
            "RSS slope {:.1}MB/h, disk slope {:.1}MB/h, r={c:.2}",
            inp.rss_slope_mb_per_hour, inp.disk_slope_mb_per_hour
        ),
        None => format!(
            "RSS slope {:.1}MB/h, disk slope {:.1}MB/h (correlation unavailable)",
            inp.rss_slope_mb_per_hour, inp.disk_slope_mb_per_hour
        ),
    };

    // Q4: the growth is the OR snapshot path iff the retained OR WAL bytes dwarf
    // the bounded (~11KB, pruned) tombstone corpus.
    let corpus = inp.tombstone_corpus_bytes.unwrap_or(0);
    let q4_is_or_not_tombstone = wal.or_bytes_total > corpus.saturating_mul(10).max(1_000_000);

    let conclusion = if rises && q4_is_or_not_tombstone {
        "CONFIRMED: the OR write path appends the full per-key OR-Map snapshot per op, \
         so an OR frame grows O(N) over the run while LWW frames stay flat, and the growth \
         is the OR record/snapshot serialization path — NOT the bounded tombstone corpus. \
         Fix surface: delta-frame the OR write path (append the mutation, fold on recovery)."
            .to_string()
    } else if !rises {
        "REFUTED / INCONCLUSIVE: OR frame size did NOT rise late-vs-early over the retained \
         corpus — the O(N)-per-op-frame hypothesis is not confirmed by this run. Inspect the \
         acked-beyond-applied tail bound and write-behind retention before choosing the fix."
            .to_string()
    } else {
        "PARTIAL: OR frames grow, but the retained OR WAL bytes do not clearly dwarf the \
         tombstone corpus — re-check Q4 against the redb corpus scan before pinning the fix."
            .to_string()
    };

    MechanismReport {
        q1_or_frame_bytes_mean_early: wal.or_bytes_mean_early,
        q1_or_frame_bytes_mean_late: wal.or_bytes_mean_late,
        q1_or_frame_bytes_max_early: wal.or_bytes_max_early,
        q1_or_frame_bytes_max_late: wal.or_bytes_max_late,
        q1_or_frame_bytes_max: wal.or_bytes_max,
        q1_lww_frame_bytes_mean: lww_mean,
        q1_lww_frame_bytes_max: wal.lww_bytes_max,
        q1_or_frame_size_rises: rises,
        q1_or_vs_lww_mean_ratio: or_vs_lww,
        or_delta_frames: wal.or_delta_frames,
        or_delta_bytes_total: wal.or_delta_bytes_total,
        or_delta_bytes_max: wal.or_delta_bytes_max,
        or_delta_bytes_mean: wal.or_delta_bytes_mean,
        or_snapshot_frames: wal.or_snapshot_frames,
        or_snapshot_bytes_total: wal.or_snapshot_bytes_total,
        or_snapshot_bytes_max: wal.or_snapshot_bytes_max,
        or_snapshot_bytes_mean: wal.or_snapshot_bytes_mean,
        q2_wal_mb_before_drain: inp.wal_mb_before_drain,
        q2_wal_mb_after_drain: inp.wal_mb_after_drain,
        q2_drain_delta_mb: drain_delta,
        q2_top_decile_frames: wal.top_decile_frames,
        q2_top_decile_or_bytes_max: wal.top_decile_or_bytes_max,
        q2_attribution,
        q3_rss_slope_mb_per_hour: inp.rss_slope_mb_per_hour,
        q3_disk_slope_mb_per_hour: inp.disk_slope_mb_per_hour,
        q3_rss_disk_correlation: corr,
        q3_attribution,
        q4_tombstone_corpus_bytes: inp.tombstone_corpus_bytes,
        q4_tombstone_gauge_bytes: inp.tombstone_gauge_bytes,
        q4_or_wal_bytes_total: wal.or_bytes_total,
        q4_growth_is_or_snapshot_not_tombstone: q4_is_or_not_tombstone,
        wal_segment_files: wal.segment_files,
        or_frames: wal.or_frames,
        lww_frames: wal.lww_frames,
        remove_frames: wal.remove_frames,
        conclusion,
        sequencing_note: "Sequencing option for the R1 /xask gate: tail-bounding the \
             acked-beyond-applied WAL is the lowest-risk FIRST containment (OOM/disk-full \
             -> backpressure, zero recovery-path risk); delta-framing is the eventual \
             O(1)-frame fix. If Q2 shows the drain burst is buffered O(N) frames, \
             tail-bounding addresses it directly."
            .to_string(),
    }
}

fn print_mechanism_report(r: &MechanismReport) {
    println!("\n=== OR-CHURN MECHANISM REPORT (report-only, Q1-Q4) ===");
    println!(
        "Q1 bytes/frame:    OR early(mean={:.0}B max={}B) late(mean={:.0}B max={}B) max={}B | \
         LWW mean={:.0}B max={}B | OR/LWW={:.1}x | rises={}",
        r.q1_or_frame_bytes_mean_early,
        r.q1_or_frame_bytes_max_early,
        r.q1_or_frame_bytes_mean_late,
        r.q1_or_frame_bytes_max_late,
        r.q1_or_frame_bytes_max,
        r.q1_lww_frame_bytes_mean,
        r.q1_lww_frame_bytes_max,
        r.q1_or_vs_lww_mean_ratio,
        r.q1_or_frame_size_rises,
    );
    println!(
        "OR frame census:   delta(n={} mean={:.0}B max={}B total={}B) | \
         snapshot(n={} mean={:.0}B max={}B total={}B)",
        r.or_delta_frames,
        r.or_delta_bytes_mean,
        r.or_delta_bytes_max,
        r.or_delta_bytes_total,
        r.or_snapshot_frames,
        r.or_snapshot_bytes_mean,
        r.or_snapshot_bytes_max,
        r.or_snapshot_bytes_total,
    );
    println!(
        "Q2 drain window:   before={} after={} delta={} | {}",
        r.q2_wal_mb_before_drain
            .map_or_else(|| "n/a".to_string(), |v| format!("{v:.1}MB")),
        r.q2_wal_mb_after_drain
            .map_or_else(|| "n/a".to_string(), |v| format!("{v:.1}MB")),
        r.q2_drain_delta_mb
            .map_or_else(|| "n/a".to_string(), |v| format!("{v:.1}MB")),
        r.q2_attribution,
    );
    println!(
        "Q3 RSS holder:     rss_slope={:.1}MB/h disk_slope={:.1}MB/h corr={} | {}",
        r.q3_rss_slope_mb_per_hour,
        r.q3_disk_slope_mb_per_hour,
        r.q3_rss_disk_correlation
            .map_or_else(|| "n/a".to_string(), |v| format!("{v:.2}")),
        r.q3_attribution,
    );
    println!(
        "Q4 not tombstone:  or_wal_bytes={} tombstone_corpus={} gauge={} | growth_is_or={}",
        r.q4_or_wal_bytes_total,
        r.q4_tombstone_corpus_bytes
            .map_or_else(|| "n/a".to_string(), |v| v.to_string()),
        r.q4_tombstone_gauge_bytes,
        r.q4_growth_is_or_snapshot_not_tombstone,
    );
    println!(
        "scan:              segments={} or_frames={} lww_frames={} remove_frames={}",
        r.wal_segment_files, r.or_frames, r.lww_frames, r.remove_frames
    );
    println!("conclusion:        {}", r.conclusion);
    println!("sequencing:        {}", r.sequencing_note);
}

// ---------------------------------------------------------------------------
// Durable-layer reading (report-only)
// ---------------------------------------------------------------------------

/// One deciding series' classified shape, mirrored for serialization.
///
/// A mirror rather than a `Serialize` on the classifier's own type: `monitor.rs`
/// is `#[path]`-included by two integration targets with no sibling module, so
/// it stays `std`-only and every derive lives here — the same split
/// `MechanismReport` already uses.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SeriesShapeRow {
    /// One of the frozen deciding-series names, carried so a row names its own
    /// series instead of being identified by position.
    name: String,
    /// Rendered through the classifier's own `as_str`, so the console and the
    /// artifact can never disagree about a shape.
    shape: String,
    /// Which envelope fired — `PEAKS`, `FLOOR` or `BOTH`. Meaningful only under
    /// a rising shape, which `shape` already states, so its absence carries no
    /// disposition and it keeps the default skip.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    firing_envelope: Option<String>,
    /// Retained (post-warmup-exclusion) sample count.
    samples: u64,
    /// Retained span in seconds.
    span_secs: f64,
    first_half_peak: u64,
    last_half_peak: u64,
    third_quarter_peak: u64,
    last_quarter_peak: u64,
    first_half_trough: u64,
    last_half_trough: u64,
    third_quarter_trough: u64,
    last_quarter_trough: u64,
    /// Integer floor of the retained last half's mean, taken off the SAME
    /// series and the SAME split index the peaks above come from — so a peak
    /// and a mean are never read across two cadences.
    last_half_mean: u64,
}

/// One census, mirrored for serialization: where it was taken, when, and all
/// thirteen counted fields.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CensusRow {
    /// `CHECKPOINT`, `TERMINAL` or `LIVE_COPY`, rendered through the source's
    /// own `as_str`. A `LIVE_COPY` row is taken from a smeared image of a live
    /// store file and is OBSERVATION ONLY — the token is what fences it.
    source: String,
    /// When the census was INITIATED. Meaning unchanged, so the sort and the
    /// nearest-record lookup keyed on it are value-identical to before.
    elapsed_secs: f64,
    /// When the census COMPLETED — the far edge of the window a live copy is
    /// smeared across. Disposition-bearing: it always serializes, as an
    /// explicit null where the producer took no copy, so "this producer takes
    /// no copy" is never confused with "this artifact predates the field".
    copy_completed_secs: Option<f64>,
    keys_scanned: u64,
    keys_undecodable: u64,
    or_map_keys: u64,
    or_tombstones_keys: u64,
    lww_keys: u64,
    live_entries: u64,
    live_tag_bytes: u64,
    tombstone_entries: u64,
    tombstone_bytes: u64,
    tombstone_dup_entries: u64,
    keys_with_tombstones: u64,
    keys_all_dead: u64,
    max_tombstones_per_key: u64,
}

/// The origin reading and every typed counter it was classified from.
///
/// Flattened onto [`DurableReadingReport`], so all of these are ROOT-level keys
/// in the artifact rather than nested under an object — which is what the
/// checks over this instrument read.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct OriginReport {
    /// Lines the capture matched on the origin target and retained. Taken from
    /// the aggregate the classifier itself read, never from a second counter:
    /// a report whose numbers came from two sources could disagree with the
    /// reading it prints beside them.
    origin_matched: u64,
    /// Matched lines that did not yield all eight fields.
    origin_unparsed: u64,
    /// Matched lines refused because the capture was full.
    origin_dropped: u64,
    /// Rendered through the reading's own `as_str`.
    origin_reading: String,
    /// The origin reading's NAMED reason — distinct from the durable reading's
    /// own `reason`, and given its own key so no reader has to work out which
    /// one an artifact is showing.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    origin_reason: Option<String>,
    /// Whether the effective child log filter actually selected the origin
    /// target for this run.
    armed: bool,
    /// The exited-epoch qualifier. Observation only: it qualifies the origin
    /// reading and never enters a plateau predicate.
    ///
    /// Disposition-bearing: it always serializes, as an explicit null where no
    /// scrape ever read the metric, because a qualifier reported as `0` when it
    /// was in fact never observed produces the most reassuring reading this
    /// instrument can emit off no evidence at all.
    epochs_exited: Option<u64>,
    /// WHY the qualifier has no value, as a TYPED token rendered through
    /// `EpochsExitedAbsence::as_str` — `ABSENT` or `UNREADABLE`, never a
    /// sentence — so a consumer reads the disposition by equality rather than
    /// by substring match. Disposition-bearing, and explicitly null whenever
    /// `epochs_exited` carries a number: exactly one of the two is populated.
    epochs_exited_absence: Option<String>,
    /// Scrapes that reached the exited-epoch parse.
    epochs_exited_scrapes_total: u64,
    /// Scrapes in which at least one sample of it read. Independently
    /// incremented on the `Read` arm — never computed as
    /// `total - absent - unreadable` — so the identity
    /// `read + absent + unreadable == total` can actually fail.
    epochs_exited_scrapes_read: u64,
    /// Scrapes in which the metric never appeared in the body.
    epochs_exited_scrapes_absent: u64,
    /// Scrapes in which it appeared and nothing read.
    epochs_exited_scrapes_unreadable: u64,
    /// Samples counted malformed across the run. Sample-scoped: NOT a term in
    /// the scrape identity above.
    epochs_exited_malformed_samples: u64,
    /// Samples whose fold into the summed column overflowed. Sample-scoped. A
    /// non-zero value makes that column a saturation marker, not a total.
    epochs_exited_overflowed_samples: u64,
    /// Server restarts during the run, from the recovery checkpoint and from
    /// nothing else.
    restarts: u64,
    /// The first parsed line that returned zero references while holding some
    /// at entry, rendered back to its eight fields.
    ///
    /// Disposition-bearing: it always serializes, as an explicit null where
    /// there was no such line, so "no line of that shape" is never confused
    /// with "this artifact predates the field".
    first_zero_return_line: Option<String>,
}

/// The durable-layer reading over the four deciding series, plus the census and
/// origin columns it is read beside.
///
/// REPORT-ONLY, without exception: nothing here is ANDed into the run verdict,
/// and an instrument-blind reading is surfaced on `pending_gates` and nowhere
/// else. A non-rising reading means *this horizon did not show it*, never
/// *there is nothing to show*.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DurableReadingReport {
    /// Rendered through the reading's own `as_str`.
    reading: String,
    /// The reading's NAMED reason.
    ///
    /// Disposition-bearing: it always serializes, as an explicit null where it
    /// has no value.
    reason: Option<String>,
    /// The four frozen deciding series, one row each.
    deciding_series: Vec<SeriesShapeRow>,
    /// Every census taken during the run, checkpoint, terminal and live-copy
    /// alike, in clock order.
    censuses: Vec<CensusRow>,
    /// The last census taken with the server process DEAD after the run.
    ///
    /// Disposition-bearing: it always serializes, as an explicit null where the
    /// terminal scan produced nothing, because a missing terminal census is a
    /// visible gap in the instrument rather than an absent field.
    census_terminal: Option<CensusRow>,
    /// Largest write-behind watermark lag seen across the run.
    ///
    /// An OBSERVATION column, never a deciding series. Disposition-bearing: it
    /// always serializes, as an explicit null where the metric was absent from
    /// every scraped body, so a silent absence can never masquerade as a zero.
    writebehind_lag_max: Option<u64>,
    /// Scrapes that reached the write-behind-lag parse.
    writebehind_lag_scrapes_total: u64,
    /// Scrapes in which at least one sample of it read. Independently
    /// incremented on the `Read` arm, for the same falsifiability reason as its
    /// exited-epoch sibling.
    writebehind_lag_scrapes_read: u64,
    /// Scrapes in which the metric never appeared in the body.
    writebehind_lag_scrapes_absent: u64,
    /// Scrapes in which it appeared and nothing read.
    writebehind_lag_scrapes_unreadable: u64,
    /// Samples counted malformed across the run. Sample-scoped: NOT a term in
    /// the scrape identity above.
    writebehind_lag_malformed_samples: u64,
    /// Samples whose fold overflowed. Sample-scoped. This column is a `max`, so
    /// a non-zero value here reports the parse's own saturation rather than a
    /// clamped column.
    writebehind_lag_overflowed_samples: u64,
    /// The effective child log filter for this run — the one string that both
    /// launched the child and derived `armed`.
    log_filter: String,
    #[serde(flatten)]
    origin: OriginReport,
}

/// Strip ANSI SGR escape sequences from one captured child line.
///
/// The child's log formatter emits colour UNCONDITIONALLY — it never tests
/// whether its output is a terminal — and the harness always reads that output
/// through a pipe. Colour turns `ts=1756…` into an escape-interleaved token
/// whose `key=value` split finds neither the key nor the value, so every origin
/// line would read as unparsed and the reading would fail closed on a run where
/// the emitter was in fact working perfectly. Normalizing HERE, at the harness
/// boundary that owns the child's output, is what keeps the parser
/// formatter-agnostic and `std`-only and keeps this instrument out of the
/// server it measures.
fn strip_ansi(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(esc) = rest.find('\u{1b}') {
        out.push_str(&rest[..esc]);
        let tail = &rest[esc..];
        let Some(after) = tail.strip_prefix("\u{1b}[") else {
            // A lone escape that is not a CSI introducer: drop the escape alone,
            // so no payload character is ever swallowed by mistake.
            rest = &tail['\u{1b}'.len_utf8()..];
            continue;
        };
        match after
            .char_indices()
            .find(|(_, c)| ('\u{40}'..='\u{7e}').contains(c))
        {
            // A complete CSI sequence: drop it up to and including its final byte.
            Some((idx, c)) => rest = &after[idx + c.len_utf8()..],
            // Unterminated: there is no final byte, so nothing after it is payload.
            None => rest = "",
        }
    }
    out.push_str(rest);
    out
}

/// Render one parsed origin line back to its eight fields.
///
/// Rendered rather than mirrored as a struct so the artifact carries the
/// evidence in the same `key=value` shape the parser read it in, and so every
/// one of the eight parsed fields has a reader — a parsed field nobody reads is
/// a field that can rot without anything noticing.
fn render_origin_line(line: &OriginLine) -> String {
    format!(
        "ts={} op_seq={} epoch={} refs_returned={} refs_at_entry={} \
         bytes_returned={} watermark={} ceiling={}",
        line.ts,
        line.op_seq,
        line.epoch,
        line.refs_returned,
        line.refs_at_entry,
        line.bytes_returned,
        line.watermark,
        line.ceiling
    )
}

fn census_row(record: &CensusRecord) -> CensusRow {
    let c = record.census;
    CensusRow {
        source: record.source.as_str().to_string(),
        elapsed_secs: record.elapsed_secs,
        copy_completed_secs: record.copy_completed_secs,
        keys_scanned: c.keys_scanned,
        keys_undecodable: c.keys_undecodable,
        or_map_keys: c.or_map_keys,
        or_tombstones_keys: c.or_tombstones_keys,
        lww_keys: c.lww_keys,
        live_entries: c.live_entries,
        live_tag_bytes: c.live_tag_bytes,
        tombstone_entries: c.tombstone_entries,
        tombstone_bytes: c.tombstone_bytes,
        tombstone_dup_entries: c.tombstone_dup_entries,
        keys_with_tombstones: c.keys_with_tombstones,
        keys_all_dead: c.keys_all_dead,
        max_tombstones_per_key: c.max_tombstones_per_key,
    }
}

fn shape_row(reading: &SeriesShapeReading) -> SeriesShapeRow {
    SeriesShapeRow {
        name: reading.name.to_string(),
        shape: reading.shape.as_str().to_string(),
        firing_envelope: reading.firing_envelope.map(ToString::to_string),
        samples: reading.samples,
        span_secs: reading.span_secs,
        first_half_peak: reading.first_half_peak,
        last_half_peak: reading.last_half_peak,
        third_quarter_peak: reading.third_quarter_peak,
        last_quarter_peak: reading.last_quarter_peak,
        first_half_trough: reading.first_half_trough,
        last_half_trough: reading.last_half_trough,
        third_quarter_trough: reading.third_quarter_trough,
        last_quarter_trough: reading.last_quarter_trough,
        last_half_mean: reading.last_half_mean,
    }
}

/// Assemble the durable-layer reading from the run's observations.
///
/// Returns the typed reading alongside its serializable mirror so the caller
/// decides on the ENUM rather than on a rendered token — a disposition read
/// back out of a string is a disposition that a renaming can silently break.
///
/// Every series is snapshotted into a local under its own lock before any row
/// is built: the guards are not reentrant, so two reads of one series meeting
/// inside a single expression would deadlock the run at its very last step.
fn build_durable_reading_report(
    durable: &DurableObservations,
    censuses: &[CensusRecord],
    origin: &OriginCaptureSnapshot,
    log_filter: &str,
) -> (DurableReading, DurableReadingReport) {
    let rss_kib = durable.rss_kib.lock().clone();
    let redb_bytes = durable.redb_bytes.lock().clone();
    let wal_bytes = durable.wal_bytes.lock().clone();
    let wal_segment_files = durable.wal_segment_files.lock().clone();
    let writebehind_lag = *durable.writebehind_lag.lock();
    let restarts = durable.restarts.load(Ordering::Relaxed);
    let epochs_exited = *durable.epochs_exited.lock();

    // The four series are paired with the frozen names by construction, so the
    // classifier's name-set check cannot be satisfied by a set this site
    // assembled loosely.
    let series: [(&'static str, &[SeriesPoint]); 4] = [
        (DECIDING_SERIES[0], &rss_kib),
        (DECIDING_SERIES[1], &redb_bytes),
        (DECIDING_SERIES[2], &wal_bytes),
        (DECIDING_SERIES[3], &wal_segment_files),
    ];
    let readings: Vec<SeriesShapeReading> = series
        .iter()
        .map(|(name, points)| {
            classify_series_shape(
                name,
                points,
                DEFAULT_TOMBSTONE_CORPUS_MIN_SAMPLES,
                DEFAULT_TOMBSTONE_CORPUS_MIN_SPAN_SECS,
            )
        })
        .collect();
    let (reading, reason) = classify_durable_reading(&readings);

    // The filter that launched the child is the same string the arming flag is
    // derived from, so a run cannot be reported as armed while the child was in
    // fact quiet.
    let armed = log_filter.contains(process::ORIGIN_TARGET);
    let captured: Vec<String> = origin.lines.iter().map(|l| strip_ansi(l)).collect();
    // The qualifier crosses this hop as the `Option` the fold produced. Nothing
    // here may supply a stand-in value: an absence that arrives as a number is
    // an absence the classifier can no longer refuse to classify from.
    let aggregate = aggregate_origin_lines(
        &captured,
        origin.dropped,
        restarts,
        armed,
        epochs_exited.value,
    );
    let (origin_reading, origin_reason) = classify_origin_reading(&aggregate);
    let first_zero_return_line = aggregate
        .lines
        .iter()
        .find(|l| l.refs_returned == 0 && l.refs_at_entry > 0)
        .map(render_origin_line);

    let census_terminal = censuses
        .iter()
        .rev()
        .find(|r| r.source == CensusSource::Terminal)
        .map(census_row);

    // WHY the qualifier has no value, derived from the counters rather than
    // from a second parse, and rendered through the token type's own `as_str`
    // so the artifact can only ever carry one of the two pinned strings. A
    // column that read nothing was either never seen in any body or seen in one
    // and never legible, and those are different instrument faults: the second
    // says the exporter is emitting the metric and the harness cannot read it.
    // A run in which no scrape reached the parse at all reads ABSENT, and the
    // `scrapes_total == 0` beside the token is what makes that case legible
    // rather than indistinguishable from a body that omitted the metric.
    // Exactly one of this and `epochs_exited` is populated.
    let epochs_exited_absence = match epochs_exited.value {
        Some(_) => None,
        None if epochs_exited.scrapes_unreadable > 0 => {
            Some(EpochsExitedAbsence::Unreadable.as_str().to_string())
        }
        None => Some(EpochsExitedAbsence::Absent.as_str().to_string()),
    };

    let report = DurableReadingReport {
        reading: reading.as_str().to_string(),
        reason,
        deciding_series: readings.iter().map(shape_row).collect(),
        censuses: censuses.iter().map(census_row).collect(),
        census_terminal,
        writebehind_lag_max: writebehind_lag.value,
        writebehind_lag_scrapes_total: writebehind_lag.scrapes_total,
        writebehind_lag_scrapes_read: writebehind_lag.scrapes_read,
        writebehind_lag_scrapes_absent: writebehind_lag.scrapes_absent,
        writebehind_lag_scrapes_unreadable: writebehind_lag.scrapes_unreadable,
        writebehind_lag_malformed_samples: writebehind_lag.malformed_samples,
        writebehind_lag_overflowed_samples: writebehind_lag.overflowed_samples,
        log_filter: log_filter.to_string(),
        origin: OriginReport {
            origin_matched: aggregate.matched,
            origin_unparsed: aggregate.unparsed,
            origin_dropped: aggregate.dropped,
            origin_reading: origin_reading.as_str().to_string(),
            origin_reason,
            armed: aggregate.armed,
            epochs_exited: aggregate.epochs_exited,
            epochs_exited_absence,
            epochs_exited_scrapes_total: epochs_exited.scrapes_total,
            epochs_exited_scrapes_read: epochs_exited.scrapes_read,
            epochs_exited_scrapes_absent: epochs_exited.scrapes_absent,
            epochs_exited_scrapes_unreadable: epochs_exited.scrapes_unreadable,
            epochs_exited_malformed_samples: epochs_exited.malformed_samples,
            epochs_exited_overflowed_samples: epochs_exited.overflowed_samples,
            restarts: aggregate.restarts,
            first_zero_return_line,
        },
    };
    (reading, report)
}

/// Check, at the moment the reading becomes an artifact, that each observed
/// column's scrape counters still add up: `read + absent + unreadable == total`.
///
/// A hard `assert!`, deliberately NOT a `debug_assert!`: every arm this harness
/// runs is a release build, so a debug assertion here would be a check that
/// never once executed in the use it was written for. It is sited at emission
/// rather than at the fold because this is the last point before the numbers
/// leave the process — a console line and a JSON file — and an artifact whose
/// own counters disagree is not evidence of anything, so producing one silently
/// is worse than failing loudly.
///
/// The identity is a real constraint rather than a restatement of subtraction
/// only because `read` is recorded on the `Read` arm instead of being computed
/// as `total - absent - unreadable`; a dropped increment anywhere in the fold
/// therefore surfaces here. The two sample-scoped counters are not terms in it:
/// they count samples, not scrapes.
///
/// This decides NOTHING about the run. It cannot turn a failing run into a
/// passing one or the reverse — no counter reaches the verdict or the exit
/// code — it fires only when the harness's own bookkeeping is inconsistent.
fn assert_scrape_counter_identity(r: &DurableReadingReport) {
    let epochs_parts = r
        .origin
        .epochs_exited_scrapes_read
        .saturating_add(r.origin.epochs_exited_scrapes_absent)
        .saturating_add(r.origin.epochs_exited_scrapes_unreadable);
    assert!(
        epochs_parts == r.origin.epochs_exited_scrapes_total,
        "exited-epoch scrape counters disagree: read {} + absent {} + unreadable {} = {} != total {}",
        r.origin.epochs_exited_scrapes_read,
        r.origin.epochs_exited_scrapes_absent,
        r.origin.epochs_exited_scrapes_unreadable,
        epochs_parts,
        r.origin.epochs_exited_scrapes_total
    );

    let lag_parts = r
        .writebehind_lag_scrapes_read
        .saturating_add(r.writebehind_lag_scrapes_absent)
        .saturating_add(r.writebehind_lag_scrapes_unreadable);
    assert!(
        lag_parts == r.writebehind_lag_scrapes_total,
        "write-behind-lag scrape counters disagree: read {} + absent {} + unreadable {} = {} != total {}",
        r.writebehind_lag_scrapes_read,
        r.writebehind_lag_scrapes_absent,
        r.writebehind_lag_scrapes_unreadable,
        lag_parts,
        r.writebehind_lag_scrapes_total
    );
}

/// Render the durable-layer reading to the console.
///
/// Every number the artifact carries is printed here too, so a console log is a
/// complete record of the reading and a reader never has to open the JSON to
/// find out which envelope fired or what the terminal census counted.
fn print_durable_reading_report(r: &DurableReadingReport) {
    println!("\n=== DURABLE-LAYER READING (report-only, decides nothing) ===");
    println!("reading:           {}", r.reading);
    println!(
        "reason:            {}",
        r.reason.as_deref().unwrap_or("(none)")
    );
    println!("log filter:        {}", r.log_filter);
    for row in &r.deciding_series {
        println!(
            "  {:<18} shape={} envelope={} samples={} span={:.1}s",
            row.name,
            row.shape,
            row.firing_envelope.as_deref().unwrap_or("-"),
            row.samples,
            row.span_secs
        );
        println!(
            "  {:<18} peaks  h1={} h2={} q3={} q4={} last_half_mean={}",
            "",
            row.first_half_peak,
            row.last_half_peak,
            row.third_quarter_peak,
            row.last_quarter_peak,
            row.last_half_mean
        );
        println!(
            "  {:<18} floor  h1={} h2={} q3={} q4={}",
            "",
            row.first_half_trough,
            row.last_half_trough,
            row.third_quarter_trough,
            row.last_quarter_trough
        );
    }
    println!(
        "writebehind lag:   {} (observation only)",
        r.writebehind_lag_max
            .map_or_else(|| "absent".to_string(), |v| v.to_string())
    );
    println!("censuses:          {}", r.censuses.len());
    for row in &r.censuses {
        println!(
            "  {:<10} t={:.1}s keys={} undecodable={} or_map={} or_tomb={} lww={} \
             live={} live_tag_bytes={} tombstones={} tombstone_bytes={} dups={} \
             keys_with_tombstones={} keys_all_dead={} max_per_key={}",
            row.source,
            row.elapsed_secs,
            row.keys_scanned,
            row.keys_undecodable,
            row.or_map_keys,
            row.or_tombstones_keys,
            row.lww_keys,
            row.live_entries,
            row.live_tag_bytes,
            row.tombstone_entries,
            row.tombstone_bytes,
            row.tombstone_dup_entries,
            row.keys_with_tombstones,
            row.keys_all_dead,
            row.max_tombstones_per_key
        );
    }
    println!(
        "terminal census:   {}",
        r.census_terminal.as_ref().map_or_else(
            || "absent".to_string(),
            |t| format!(
                "keys={} tombstone_bytes={}",
                t.keys_scanned, t.tombstone_bytes
            )
        )
    );
    println!(
        "origin:            reading={} matched={} unparsed={} dropped={} armed={} \
         epochs_exited={} restarts={}",
        r.origin.origin_reading,
        r.origin.origin_matched,
        r.origin.origin_unparsed,
        r.origin.origin_dropped,
        r.origin.armed,
        // PLACEHOLDER(g1): how an ABSENT qualifier renders on the console — the
        // token beside it, or in place of it — is decided with the fold that
        // can actually produce one.
        match r.origin.epochs_exited {
            Some(observed) => observed.to_string(),
            None => "(absent)".to_string(),
        },
        r.origin.restarts
    );
    println!(
        "origin reason:     {}",
        r.origin.origin_reason.as_deref().unwrap_or("(none)")
    );
    println!(
        "first zero-return: {}",
        r.origin
            .first_zero_return_line
            .as_deref()
            .unwrap_or("(none)")
    );
}

// ---------------------------------------------------------------------------
// Checkpoints
// ---------------------------------------------------------------------------

/// Quiesce churn, then verify the server read-back equals the model exactly and
/// that two client connections agree on the Merkle root. No re-assertion of
/// state, so this genuinely tests that the server stored every acked write.
/// Returns `(hard, pending)` failures. `hard` reddens the run; `pending` is
/// reserved for future expected-fail gates (currently empty).
async fn steady_checkpoint(
    supervisor: &Arc<ServerSupervisor>,
    model: &Arc<Model>,
    jwt: &str,
    config: &Config,
    paused: &Arc<AtomicBool>,
) -> Result<(Vec<String>, Vec<String>)> {
    paused.store(true, Ordering::SeqCst);
    tokio::time::sleep(config.quiesce).await;
    let result = steady_checkpoint_inner(supervisor, model, jwt).await;
    paused.store(false, Ordering::SeqCst);
    result
}

async fn steady_checkpoint_inner(
    supervisor: &Arc<ServerSupervisor>,
    model: &Arc<Model>,
    jwt: &str,
) -> Result<(Vec<String>, Vec<String>)> {
    let mut hard = Vec::new();
    let pending = Vec::new();
    let expected = model.snapshot();

    // HARD: full-scan read-your-writes convergence — the read surface this spec
    // makes buffer-aware (correct under active eviction).
    let mut v1 = SoakClient::connect(supervisor.addr(), VERIFIER_IDX, jwt).await?;
    let actual = v1.read_all(LWW_MAP).await?;
    let diffs = compare(&expected, &actual);
    if !diffs.is_empty() {
        hard.push(format!(
            "steady convergence: {} key(s) diverged (e.g. {})",
            diffs.len(),
            diffs
                .iter()
                .take(5)
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    // HARD: two clients must agree on the Merkle root. DurableMerkleIndex
    // (SPEC-325b / SYNC-treewalk) now builds from the datastore rather than the
    // resident set, so eviction no longer mutates the root between reads.
    let root1 = v1.merkle_root(LWW_MAP).await?;
    let mut v2 = SoakClient::connect(supervisor.addr(), VERIFIER_IDX + 1, jwt).await?;
    let root2 = v2.merkle_root(LWW_MAP).await?;
    if root1 != root2 {
        hard.push(format!(
            "merkle disagreement between two clients under eviction: {root1} != {root2}"
        ));
    }

    Ok((hard, pending))
}

/// Outcome of a recovery checkpoint, split into required vs known-pending gates.
///
/// `hard` failures fail the run. `pending_gates` records capabilities that are
/// owned by a separate, tracked track and whose failure is *expected* — they are
/// logged (never silently dropped) but must not redden the soak. Each carries
/// strict-xfail → xpass semantics: if a pending gate turns green it is promoted
/// to a `hard` failure so a maintainer flips it to required and it can never
/// silently regress. All prior TODO-530 gates (Merkle root + delta-sync +
/// QUERY-path full-scan) have been promoted to `hard` by SPEC-325b; this struct
/// is preserved so new expected-fail gates can be added without changing the
/// checkpoint interface.
#[derive(Default)]
struct RecoveryOutcome {
    hard: Vec<String>,
    pending_gates: Vec<String>,
}

/// Quiesce + capture pre-crash state, `kill -9` + restart (WAL recovery), then
/// verify the recovered state is byte-for-byte the pre-crash state. This tests
/// crash recovery in isolation: no client writes occur across the boundary, so
/// any post != pre is a recovery defect, not a lost-in-flight write.
// One parameter over the threshold, and deliberately so: the alternative is a
// wrapper struct whose only job is to carry the boot-gap clock and the corpus
// sampler together, which would hide two independent instruments behind one
// name for no gain. The corpus sampler already bundles its own three fields.
#[allow(clippy::too_many_arguments)]
async fn recovery_checkpoint(
    supervisor: &Arc<ServerSupervisor>,
    model: &Arc<Model>,
    or_ledger: &Arc<OrLedger>,
    jwt: &str,
    config: &Config,
    paused: &Arc<AtomicBool>,
    boot_gap_clock: &BootGapClock,
    corpus_sampler: &CorpusSampler,
    restarts: &AtomicU64,
) -> Result<RecoveryOutcome> {
    paused.store(true, Ordering::SeqCst);
    // Pause new client writes, then choose the pre-kill behavior:
    //
    // - default (drain): sleep `quiesce` so in-flight acks settle and the
    //   write-behind buffer flushes to redb+WAL before the kill. This scopes the
    //   assertion to durable-state recovery (the Merkle/SYNC read path).
    // - `--no-pre-kill-drain`: skip the flush entirely and kill immediately, so
    //   the only thing standing between an acked write and a `kill -9` is the WAL.
    //   This is the acked == durable assertion: it must NOT depend on a pre-kill
    //   flush. Under correctly-applied PerOp the WAL frame is fsynced before the
    //   ack returns, so recovery replays every acked write with zero one-behind
    //   loss even though the buffer never drained.
    if config.no_pre_kill_drain {
        // Settle only the in-flight ACK pipeline (a few network RTTs), NOT the
        // write-behind buffer. This stops new acks so the pre-crash snapshot is
        // a stable acked set, while staying well under the production write-behind
        // flush interval (1000ms) so acked writes remain unflushed in the buffer —
        // recovery is then forced to rebuild them from the WAL alone. This is what
        // makes the acked == durable assertion NOT depend on a pre-kill flush.
        //
        // NOTE: this assertion is only honest when the server runs the production
        // flush cadence (TOPGUN_WRITEBEHIND_FLUSH_INTERVAL_MS=1000); the harness's
        // default fast flush would drain the buffer inside this settle and mask the
        // WAL durability path. The runner sets the production cadence for the
        // no-drain validator.
        const ACK_SETTLE: Duration = Duration::from_millis(250);
        tokio::time::sleep(ACK_SETTLE).await;
    } else {
        tokio::time::sleep(config.quiesce).await;
    }

    let mut out = RecoveryOutcome::default();

    // Pre-crash snapshot (also a steady convergence check). The OR-Map no-loss
    // check is directional against the acked-add ledger, not a pre/post root
    // comparison, so no pre-crash OR root is captured here.
    let (pre_lww, pre_root) = {
        let mut v = SoakClient::connect(supervisor.addr(), VERIFIER_IDX, jwt).await?;
        let lww = v.read_all(LWW_MAP).await?;
        let root = v.merkle_root(LWW_MAP).await?;
        (lww, root)
    };
    let expected = model.snapshot();
    let pre_diffs = compare(&expected, &pre_lww);
    if !pre_diffs.is_empty() {
        out.hard.push(format!(
            "pre-crash convergence: {} key(s) diverged (e.g. {})",
            pre_diffs.len(),
            pre_diffs
                .iter()
                .take(5)
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    // kill -9 + restart against the same redb + WAL. Record the boot-recompute
    // gap around the restart: `end_secs` starts at `f64::INFINITY` (still
    // open) so a real-time consumer (the tombstone sampling loop) treats any
    // sample taken before the restart is confirmed ready as excluded, rather
    // than briefly seeing no gap at all while the restart is in flight.
    let gap_start_secs = boot_gap_clock.sampler_start.elapsed().as_secs_f64();
    boot_gap_clock.boot_gaps.lock().push(BootGap {
        start_secs: gap_start_secs,
        end_secs: f64::INFINITY,
    });
    // Expanded in place from the single `supervisor.restart(...)` this used to
    // call: the durable-corpus sample has to be taken while the child is
    // reaped and BEFORE the server re-opens its file, because opening it is
    // what runs recovery. `restart`'s 250 ms socket-release gap and its
    // failed-start handling are kept verbatim below, and `restart` itself
    // stays in use by its other call sites.
    supervisor.kill9().await;
    // The ONLY producer of the restart count, and it counts the KILL rather
    // than a successful start: what the count exists to witness is that the
    // process boundary was crossed, and the per-process counters a reading may
    // qualify on reset there whether or not the new life comes up healthy.
    restarts.fetch_add(1, Ordering::Relaxed);
    sample_durable_corpus_via_copy(
        corpus_sampler,
        boot_gap_clock.sampler_start.elapsed().as_secs_f64(),
    );
    // Brief gap so the OS releases the listening socket before rebind.
    tokio::time::sleep(Duration::from_millis(250)).await;
    if let Err(e) = supervisor.start(config.ready_timeout).await {
        out.hard
            .push(format!("server failed to restart after kill -9: {e}"));
        paused.store(false, Ordering::SeqCst);
        // Leave the just-pushed gap open (`end_secs = INFINITY`): the run is
        // already failing via `out.hard`, and the still-limping server may
        // serve stale pre-reconcile reads, so excluding subsequent samples is
        // safer than trusting them. This does NOT blind the sampler forever —
        // the next `recovery_checkpoint` whose `restart()` reaches health-ready
        // closes every lingering open gap (see the close-all loop below).
        return Ok(out);
    }
    // Close the boot gap as the FIRST action after `restart()` returns ready
    // (health-ready ⇒ `reconcile_tombstone_bytes` has re-seeded the gauge for
    // the new life). Capture the timestamp before taking the lock so lock
    // contention cannot widen the window in which a genuinely-post-reconcile
    // sample would still see the gap open and be wrongly excluded.
    //
    // Close EVERY still-open gap, not just the just-pushed one. A prior
    // `recovery_checkpoint` whose `restart()` FAILED deliberately left its gap
    // open (`end_secs = INFINITY`) so the still-limping server's stale
    // pre-reconcile reads stay excluded — but a lone `last_mut()` would close
    // only this checkpoint's gap and leave that earlier one open forever,
    // blinding the tombstone sampler for the rest of the run (every later
    // sample past its `start_secs` would be dropped). Reaching health-ready
    // here means the gauge is trustworthy again for ALL prior lives, so any
    // lingering open gap is closed at the same reconcile boundary.
    {
        let gap_end_secs = boot_gap_clock.sampler_start.elapsed().as_secs_f64();
        for gap in boot_gap_clock.boot_gaps.lock().iter_mut() {
            if gap.end_secs.is_infinite() {
                gap.end_secs = gap_end_secs;
            }
        }
    }

    // Post-recovery snapshot — no writes happened in between. `post_query` reads
    // via the full-scan QUERY path (SPEC-322b); `post_delta` reads every value
    // back via the delta-sync leaf-fetch path (the path SPEC-322a makes correct).
    //
    // ORDER IS LOAD-BEARING: the query read MUST run first, on the cold
    // post-restart store. A delta-sync leaf fetch lazy-loads each record into the
    // server's in-memory store via `RecordStore::get`, so if the delta walk ran
    // first it would warm the store and the subsequent full-scan query would
    // observe the now-resident records — a false "322b recovered" signal. Reading
    // the query path before anything touches the store measures the genuine gap.
    let (post_query, post_delta, post_root) = {
        let mut v = SoakClient::connect(supervisor.addr(), VERIFIER_IDX, jwt).await?;
        let query = v.read_all(LWW_MAP).await?;
        let delta = v.delta_sync_all(LWW_MAP).await?;
        let root = v.merkle_root(LWW_MAP).await?;
        (query, delta, root)
    };

    // HARD: DurableMerkleIndex (SPEC-325b) builds from the datastore, not the
    // resident set, so the Merkle root must survive a kill -9 + restart unchanged.
    if post_root != pre_root {
        out.hard.push(format!(
            "LWW merkle root changed across recovery: pre={pre_root} post={post_root}"
        ));
    }
    // HARD: the delta-sync leaf-fetch path drills the DurableMerkleIndex, which
    // now reads from the datastore. Post-restart the index is rebuilt from durable
    // storage, so every leaf must be reachable regardless of residency.
    let delta_diffs = compare(&pre_lww, &post_delta);
    if !delta_diffs.is_empty() {
        out.hard.push(format!(
            "LWW delta-sync read-back changed across recovery: {} key(s) (e.g. {})",
            delta_diffs.len(),
            delta_diffs
                .iter()
                .take(5)
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    // HARD: SPEC-322c wired FullScanPager with a datastore-backed streaming scan,
    // so the full-scan QUERY path must return the complete persisted dataset after
    // restart with no residency dependency.
    let query_diffs = compare(&pre_lww, &post_query);
    if !query_diffs.is_empty() {
        out.hard.push(format!(
            "QUERY-path full-scan read-back not recovered post-restart: {} key(s) (e.g. {})",
            query_diffs.len(),
            query_diffs
                .iter()
                .take(5)
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    // HARD: OR-Map directional no-loss. Every acked add in the add-only persistent
    // keyspace MUST still be observed after recovery; a missing one is a lost acked
    // write. This replaces the old OR-root equality, which false-redded on the
    // benign "recovered-more" asymmetry (WAL replay reconstructs intermediate churn
    // add/remove tags the live tree had compacted). Recovered-more is a SUPERSET of
    // the ledger, so it never reddens here — only a true loss does. The OR read runs
    // last and on its own connection: it is a different map from the LWW reads, so
    // it cannot warm the LWW store the cold full-scan query above depends on.
    if config.or_churn && !or_ledger.is_empty() {
        // Snapshot the ledger BEFORE reading the server. Any acked add recorded
        // after this point (e.g. a late-delivered ack on a fresh post-restart
        // reconnect, or a kernel-buffered pre-kill ack processed late) is simply
        // absent from `acked` and never checked, so it cannot manufacture a false
        // loss. Every tag in `acked` was recorded on an ack that returned before
        // the read below, so the server had applied it before the read — keeping
        // this a true directional superset check, never a read/snapshot race.
        let acked = or_ledger.snapshot();
        let mut v = SoakClient::connect(supervisor.addr(), VERIFIER_IDX, jwt).await?;
        match v.ormap_read_all(OR_MAP).await {
            Ok(post_observed) => {
                let missing = missing_acked_adds(&acked, &post_observed);
                if missing.is_empty() {
                    // Positive completion marker. A CI gate that only greps for the
                    // LOST signature is false-green if the check never reaches this
                    // point (crash, early exit, or the read-failed HARD below): the
                    // absence of a failure string is indistinguishable from "did not
                    // run". Emit a PASS line the gate can positively assert ran.
                    let acked_count: usize =
                        acked.values().map(std::collections::HashSet::len).sum();
                    println!(
                        "OR-Map no-loss check: PASS — {acked_count} acked add(s) verified across recovery"
                    );
                } else {
                    out.hard.push(format!(
                        "OR-Map acked add(s) LOST across recovery: {} (key,tag) pair(s) (e.g. {})",
                        missing.len(),
                        missing
                            .iter()
                            .take(5)
                            .map(|(k, t)| format!("{k}/{t}"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
            }
            // A read failure must NOT silently skip the no-loss check — that would
            // let a real loss hide behind a generic harness error (a loss could
            // even be the cause of the read failure). Surface it as a HARD failure.
            Err(e) => out.hard.push(format!(
                "OR-Map no-loss check could not complete (post-recovery read failed): {e}"
            )),
        }
    }

    paused.store(false, Ordering::SeqCst);
    Ok(out)
}

// ---------------------------------------------------------------------------
// Churn client
// ---------------------------------------------------------------------------

/// A single churn client: connects, replays its owned keys (durability +
/// offline-buffer flush), writes a burst, disconnects, optionally buffers
/// offline writes, and repeats. Each key is owned by exactly one client, so its
/// expected value is unambiguous.
async fn run_churn_client(idx: usize, ctx: ChurnCtx) {
    let owned = ctx.model.keys_owned_by(idx);
    if owned.is_empty() {
        return;
    }
    // Client-local latest intended value per owned slot. Ahead of the model only
    // while an offline-buffered write is pending; the model is updated solely on ack.
    let mut local: std::collections::HashMap<usize, i64> = std::collections::HashMap::new();
    let mut write_count: u64 = 0;
    let mut rr: usize = 0;

    loop {
        if ctx.stop.load(Ordering::SeqCst) {
            return;
        }
        // Do not (re)connect during a checkpoint quiesce.
        while ctx.paused.load(Ordering::SeqCst) && !ctx.stop.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        if ctx.stop.load(Ordering::SeqCst) {
            return;
        }

        let Some(mut client) = connect_with_retry(&ctx, idx).await else {
            continue;
        };

        // Replay: resend every owned key's latest local value. Restores any
        // kill-window loss and flushes offline-buffered writes; idempotent under
        // LWW (a fresh, higher HLC stamp always wins). Model updated on ack.
        let mut session_alive = true;
        for &slot in &owned {
            if ctx.stop.load(Ordering::SeqCst) {
                return;
            }
            if let Some(&v) = local.get(&slot) {
                let key = Model::key_for(slot);
                let (ms, ctr) = next_stamp();
                if client.write_lww(LWW_MAP, &key, v, ms, ctr).await.is_ok() {
                    ctx.model.record(&key, v);
                    ctx.metrics.resends.fetch_add(1, Ordering::Relaxed);
                } else {
                    session_alive = false;
                    break;
                }
            }
        }

        // Active write burst.
        let life = ctx.writes_per_life;
        let mut n = 0;
        while session_alive && n < life {
            if ctx.stop.load(Ordering::SeqCst) {
                return;
            }
            // Hold (without disconnecting) during a checkpoint quiesce.
            while ctx.paused.load(Ordering::SeqCst) && !ctx.stop.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            if ctx.stop.load(Ordering::SeqCst) {
                return;
            }

            let slot = owned[rr % owned.len()];
            rr += 1;
            let key = Model::key_for(slot);
            let v = local.get(&slot).copied().unwrap_or(0) + 1;
            let (ms, ctr) = next_stamp();
            if client.write_lww(LWW_MAP, &key, v, ms, ctr).await.is_ok() {
                local.insert(slot, v);
                ctx.model.record(&key, v);
                ctx.metrics.total_writes.fetch_add(1, Ordering::Relaxed);
            } else {
                ctx.metrics.write_errors.fetch_add(1, Ordering::Relaxed);
                session_alive = false;
                break;
            }

            // OR-Map churn. Two independent streams on the same map:
            //
            //  - `ork-*` add-then-immediately-remove: drives tombstone growth, the
            //    unbounded-memory candidate the soak watches (TODO-479/480). This
            //    keyspace is EXCLUDED from the no-loss check (its net observed set
            //    is empty by construction; under the WAL-only window its replayed
            //    intermediate tags are the benign "recovered-more").
            //  - `ork-persist-*` add-only: a stable `(key, tag)` per owned slot,
            //    never removed, recorded into the acked-add ledger ON ACK. Because
            //    it is never tombstoned, the post-recovery observed set must always
            //    contain it — that is the directional no-loss invariant. Re-adding
            //    the same tag is idempotent and keeps the persistent keyspace
            //    bounded, so it does not itself look like a memory leak.
            write_count += 1;
            if ctx.or_churn && write_count.is_multiple_of(ctx.or_every) {
                let or_key = format!("ork-{}", slot % ctx.or_keyspace.max(1));
                let tag = format!("{ms}:{ctr}:{idx}");
                // Churn value is irrelevant — this stream is add-then-remove and is
                // excluded from the no-loss ledger; only its tombstone growth matters.
                let churn_value = i64::from(ctr);
                if client
                    .or_add(OR_MAP, &or_key, &tag, churn_value, ms, ctr)
                    .await
                    .is_ok()
                {
                    if client.or_remove(OR_MAP, &or_key, &tag).await.is_err() {
                        session_alive = false;
                        break;
                    }
                } else {
                    session_alive = false;
                    break;
                }

                // `slot` cycles over this client's FIXED owned-slot set
                // (`owned[rr % owned.len()]`), so `pt-{idx}-{slot}` ranges over a
                // bounded set of distinct tags — one stable tag per owned slot.
                // Re-visiting a slot re-adds the same tag (idempotent), so neither
                // the ledger nor the server's persistent OR keyspace grows without
                // bound over a long soak.
                let persist_key = format!("ork-persist-{}", slot % ctx.or_keyspace.max(1));
                let persist_tag = format!("pt-{idx}-{slot}");
                // The no-loss ledger keys on the record VALUE, not the tag: the
                // server re-stamps every OR add's HLC and regenerates the tag from
                // it, so the client tag is never the persisted identity. The value
                // is stored verbatim, so a stable unique value per (idx, slot)
                // gives an identity that survives sanitization and crash recovery.
                // `idx`<churn_clients and `slot`<keyspace, so this is collision-free
                // across clients and slots and stable across re-adds of the slot.
                let persist_value = (idx as i64) * 1_000_000 + slot as i64;
                let (pms, pctr) = next_stamp();
                if client
                    .or_add(OR_MAP, &persist_key, &persist_tag, persist_value, pms, pctr)
                    .await
                    .is_ok()
                {
                    // Ledger updated ONLY on ack: an add whose ack never returned
                    // is never recorded, so a kill-window drop of an unacked add is
                    // not miscounted as loss.
                    ctx.or_ledger
                        .record_add(&persist_key, &persist_value.to_string());
                } else {
                    session_alive = false;
                    break;
                }
            }

            tokio::time::sleep(ctx.write_interval).await;
            n += 1;
        }

        // Churn: disconnect.
        drop(client);

        // Offline-write-then-reconnect: buffer a few increments locally. They are
        // NOT recorded into the model until the next reconnect resends and acks
        // them, so a crash while offline cannot manufacture a false divergence.
        if ctx.offline_keys > 0 && session_alive {
            for &slot in owned.iter().take(ctx.offline_keys) {
                let v = local.get(&slot).copied().unwrap_or(0) + 1;
                local.insert(slot, v);
            }
        }

        // Brief disconnected gap (deterministic per-client jitter).
        let jitter = 100 + (idx as u64 % 7) * 30;
        tokio::time::sleep(Duration::from_millis(jitter)).await;
    }
}

/// Connect with bounded retry, honoring the pause flag and stop signal.
async fn connect_with_retry(ctx: &ChurnCtx, idx: usize) -> Option<SoakClient> {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if ctx.stop.load(Ordering::SeqCst) {
            return None;
        }
        while ctx.paused.load(Ordering::SeqCst) && !ctx.stop.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        if ctx.stop.load(Ordering::SeqCst) {
            return None;
        }
        if let Ok(c) = SoakClient::connect(ctx.supervisor.addr(), idx, &ctx.jwt_secret).await {
            ctx.metrics.reconnects.fetch_add(1, Ordering::Relaxed);
            return Some(c);
        }
        if Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

// ---------------------------------------------------------------------------
// Tracked confirm-apply client (drives the low-water-mark)
// ---------------------------------------------------------------------------

/// Configuration for a single tracked-and-ACKing replica. Bundled into one
/// struct (mirroring `ChurnCtx`/`BootGapClock`) so `run_tracked_confirm_client`
/// takes one parameter instead of clippy's too-many-arguments threshold.
struct TrackerConfig {
    supervisor: Arc<ServerSupervisor>,
    jwt_secret: String,
    stop: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    metrics: Arc<SoakMetrics>,
    idx: usize,
    confirm_interval: Duration,
    no_ack: bool,
}

/// A single tracked-and-ACKing replica: reconnects with `connect_tracked` (so
/// its `(principal, deviceId)` identity survives across reconnects — the
/// identity the server's per-device causal frontier keys its cursor on), and
/// runs `confirm_apply` on `OR_MAP` every `confirm_interval` while connected.
///
/// Every acked `confirm_apply` round advances this replica's high-water-mark;
/// since the server's low-water-mark is the MINIMUM across all tracked
/// clients, this is what lets the epoch-scoped tombstone prune fire at all.
/// When `no_ack` is set the confirm-apply call is skipped entirely (not just
/// the ack) — the connection still exists so the harness's other assertions
/// keep exercising it, but the server never sees an `ORMAP_SYNC_INIT` from this
/// replica, so it is never tracked and the low-water-mark stays at its vacuous
/// 0 for the whole run: the negative control for the tombstone-byte hard gate,
/// with the report-only durable-corpus instrument recording it alongside.
async fn run_tracked_confirm_client(cfg: TrackerConfig) {
    let mut device_token: Option<String> = None;
    // Persisted across reconnects alongside `device_token`: the replica's
    // last-ACKed covering epoch. Re-seeding it on the fresh connection keeps the
    // first post-reconnect `claimed_epoch` truthful instead of a spurious `None`,
    // which would fail-closed gate the replica under active split-brain protection.
    let mut resume_cursor: Option<u64> = None;

    while !cfg.stop.load(Ordering::SeqCst) {
        while cfg.paused.load(Ordering::SeqCst) && !cfg.stop.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        if cfg.stop.load(Ordering::SeqCst) {
            return;
        }

        let connect = SoakClient::connect_tracked(
            cfg.supervisor.addr(),
            cfg.idx,
            &cfg.jwt_secret,
            device_token.clone(),
        )
        .await;
        let Ok(mut client) = connect else {
            tokio::time::sleep(Duration::from_millis(200)).await;
            continue;
        };
        client.resume_from_cursor(resume_cursor);

        let mut session_alive = true;
        while session_alive && !cfg.stop.load(Ordering::SeqCst) {
            // Hold across a checkpoint quiesce WITHOUT disconnecting, mirroring
            // the churn client's active-burst pause behavior — a checkpoint's
            // quiesced read-back must not race a mid-quiesce reconnect.
            while cfg.paused.load(Ordering::SeqCst) && !cfg.stop.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            if cfg.stop.load(Ordering::SeqCst) {
                break;
            }

            if !cfg.no_ack {
                match client.confirm_apply(OR_MAP).await {
                    Ok(Some(epoch)) => {
                        cfg.metrics.confirms.fetch_add(1, Ordering::Relaxed);
                        cfg.metrics
                            .last_confirmed_epoch
                            .fetch_max(epoch, Ordering::Relaxed);
                    }
                    // `Ok(None)`: nothing to confirm yet (no tombstone stamped
                    // yet) — skip, not an error.
                    Ok(None) => {}
                    Err(e) => {
                        // Surface the failure rather than reconnect-looping
                        // silently: a swallowed confirm error looks identical to
                        // a server tombstone leak (LWM never advances → gauge
                        // climbs → the tombstone-byte hard gate REDs), so an
                        // invisible plumbing bug
                        // would masquerade as the very defect this gate hunts.
                        cfg.metrics.confirm_errors.fetch_add(1, Ordering::Relaxed);
                        eprintln!("[tracker {}] confirm_apply error: {e:#}", cfg.idx);
                        session_alive = false;
                    }
                }
            }

            tokio::time::sleep(cfg.confirm_interval).await;
        }

        // Capture the device token AND last-applied cursor BEFORE dropping so the
        // next reconnect presents the SAME (principal, deviceId) identity and
        // resumes its causal frontier — otherwise every reconnect would mint a
        // fresh, forgotten replica whose cursor never accumulates, or reset the
        // claimed epoch to None and risk a fail-closed gate under protection.
        let token = client.device_token().map(str::to_string);
        let cursor = client.last_applied_cursor();
        drop(client);
        if token.is_some() {
            device_token = token;
        }
        if cursor.is_some() {
            resume_cursor = cursor;
        }
        if cfg.stop.load(Ordering::SeqCst) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}

// ---------------------------------------------------------------------------
// Negative control: divergence
// ---------------------------------------------------------------------------

/// Prove the convergence check can go red. Writes a handful of keys (server and
/// model agree), then injects an op into the model that is deliberately NOT
/// applied to the server ("skip applying one op on a replica"), and asserts the
/// real convergence comparison detects the resulting divergence.
async fn run_inject_divergence(config: &Config) -> i32 {
    println!("=== NEGATIVE CONTROL: inject-divergence ===");
    let binary = resolve_server_binary();
    let tempdir = match tempfile::tempdir() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("FATAL: {e}");
            return 2;
        }
    };
    let port = match ServerSupervisor::pick_free_port() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("FATAL: {e}");
            return 2;
        }
    };
    let jwt = "test-e2e-secret".to_string();
    let supervisor = ServerSupervisor::new(ServerConfig {
        binary,
        data_dir: tempdir.path().to_path_buf(),
        port,
        jwt_secret: jwt.clone(),
        wal_fsync_policy: config.wal_fsync.clone(),
    });
    if let Err(e) = supervisor.start(config.ready_timeout).await {
        eprintln!("FATAL: server start: {e}");
        return 2;
    }

    let model = Model::new(16, 1);
    let mut client = match SoakClient::connect(supervisor.addr(), 0, &jwt).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("FATAL: connect: {e}");
            supervisor.shutdown().await;
            return 2;
        }
    };

    // Honest baseline: write k-0..k-9, recording each into the model.
    for i in 0..10 {
        let key = Model::key_for(i);
        let (ms, ctr) = next_stamp();
        if let Err(e) = client.write_lww(LWW_MAP, &key, i as i64, ms, ctr).await {
            eprintln!("FATAL: baseline write: {e}");
            supervisor.shutdown().await;
            return 2;
        }
        model.record(&key, i as i64);
    }

    // INJECTION: record an op in the model that the server never sees.
    let injected_key = Model::key_for(0);
    model.record(&injected_key, 999_999);
    println!("injected: model[{injected_key}]=999999 was NOT applied to the server");

    tokio::time::sleep(config.quiesce).await;

    let actual = match client.read_all(LWW_MAP).await {
        Ok(a) => a,
        Err(e) => {
            eprintln!("FATAL: read_all: {e}");
            supervisor.shutdown().await;
            return 2;
        }
    };
    supervisor.shutdown().await;

    let diffs = compare(&model.snapshot(), &actual);
    if diffs.is_empty() {
        eprintln!(
            "NEGATIVE CONTROL FAILED: harness did NOT detect the injected divergence — \
             the convergence check is blind and proves nothing"
        );
        return 3;
    }
    println!(
        "NEGATIVE CONTROL PASSED: divergence correctly detected (assertion RED as expected): {}",
        diffs
            .iter()
            .take(3)
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    );
    // Exit non-zero: a detected divergence means the soak assertion is RED.
    1
}

// ---------------------------------------------------------------------------
// Negative control: panic
// ---------------------------------------------------------------------------

/// Prove the panic capture can go red. Runs a synthetic child that prints a Rust
/// panic line and exits 101, feeds its output through the SAME `PanicWatch`
/// detection code the supervisor uses, and asserts the watch tripped. This keeps
/// the production server free of any test-only panic hook while still exercising
/// the real detection path end-to-end.
async fn run_inject_panic() -> i32 {
    println!("=== NEGATIVE CONTROL: inject-panic ===");

    let watch = process::PanicWatch::new_standalone();

    let mut child = match tokio::process::Command::new("sh")
        .arg("-c")
        .arg("echo \"thread 'main' panicked at src/synthetic.rs:1:1: injected soak panic\" 1>&2; exit 101")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("FATAL: cannot spawn synthetic panic process: {e}");
            return 2;
        }
    };

    if let Some(stderr) = child.stderr.take() {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            watch.record_line(&line);
        }
    }
    if let Ok(status) = child.wait().await {
        if !status.success() {
            let detail = format!("synthetic exit status {status:?}");
            watch.record_unexpected_exit(&detail);
        }
    }

    if watch.tripped() {
        println!(
            "NEGATIVE CONTROL PASSED: panic correctly captured (assertion RED as expected):\n{}",
            watch.report().unwrap_or_default()
        );
        1
    } else {
        eprintln!(
            "NEGATIVE CONTROL FAILED: harness did NOT capture the synthetic panic — \
             the panic watch is blind and proves nothing"
        );
        3
    }
}

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

/// Every flag the soak recognizes. Presence of any one marks an explicit run
/// mode (so foreign libtest args from `cargo test` do not trigger a default soak).
const KNOWN_FLAGS: &[&str] = &[
    "--duration",
    "--churn-clients",
    "--keyspace",
    "--write-interval-ms",
    "--writes-per-life",
    "--offline-keys",
    "--crash-interval",
    "--steady-interval",
    "--quiesce",
    "--ready-timeout",
    "--mem-sample-interval",
    "--mem-threshold-mb-per-hour",
    "--mem-min-growth-mb",
    "--mem-ceiling-mb",
    "--server-port",
    "--data-dir",
    "--wal-fsync",
    "--or-churn",
    "--or-keyspace",
    "--or-every",
    "--json-output",
    "--progress-output",
    "--inject-divergence",
    "--inject-panic",
    "--confirm-interval",
    "--no-ack",
    "--inject-slow-leak",
    "--no-pre-kill-drain",
    "--tombstone-corpus-headroom-bytes",
    "--tombstone-corpus-ceiling-bytes",
    "--mechanism-report",
    "--durable-reading",
    "--live-census-interval",
    "--sampler-jitter-seed",
    "--smoke",
];

fn print_usage() {
    // The two durable-corpus knobs are spelled out with their effect, because
    // an operator arming a gate needs to know what each one moves:
    //   --tombstone-corpus-headroom-bytes <n>  slack the LEVEL clause allows
    //   --tombstone-corpus-ceiling-bytes <n>  arm L2's absolute ceiling — a
    //       reported bound, not a HARD gate ceiling
    println!(
        "TopGun soak harness (G4b / TODO-484)\n\n\
         Drives the real out-of-process topgun-server against an on-disk redb + WAL,\n\
         exercising churn, kill -9 crash loops, convergence/recovery assertions,\n\
         memory-growth monitoring, and panic capture.\n\n\
         No mode flag was given, so nothing ran. Examples:\n\
         \x20 # 1h smoke soak with crash loop + JSON report\n\
         \x20 soak_harness --duration 3600 --crash-interval 120 --json-output soak.json \\\n\
         \x20              --progress-output soak-progress.jsonl\n\
         \x20 # convenience preset (short, full-feature)\n\
         \x20 soak_harness --smoke\n\
         \x20 # negative controls (must exit non-zero == assertion RED)\n\
         \x20 soak_harness --inject-divergence\n\
         \x20 soak_harness --inject-panic\n\
         \x20 soak_harness --no-ack --duration 3600  # slope hard-gate must FAIL\n\
         \x20 soak_harness --inject-slow-leak --duration 3600  # slope detection-floor calibration\n\
         \x20 # durable-corpus knobs (default: headroom 65536 bytes, ceiling disarmed)\n\
         \x20 soak_harness --tombstone-corpus-headroom-bytes 262144 \\\n\
         \x20              --tombstone-corpus-ceiling-bytes 8388608\n\
         \x20 # OR-churn WAL/RSS growth mechanism report (Q1-Q4), report-only\n\
         \x20 soak_harness --mechanism-report --or-churn true --or-keyspace 48 \\\n\
         \x20              --crash-interval 0 --duration 3600 --data-dir ./soak-data\n\
         \x20 # durable-layer reading (report-only): arms the filesystem samplers and\n\
         \x20 # writes <json-output>.durable.json beside the primary report\n\
         \x20 soak_harness --durable-reading --json-output soak.json --data-dir ./soak-data\n\
         \x20 # live-copy census sampler (seconds; 0 = DISARMED, the default).\n\
         \x20 # Observation only: a census off a live store file is a smeared image\n\
         \x20 soak_harness --durable-reading --live-census-interval 60\n\
         \x20 # seed for the filesystem samplers\' cadence jitter. Runner-supplied,\n\
         \x20 # never derived in the binary, so the recorded seed IS the seed used\n\
         \x20 soak_harness --durable-reading --sampler-jitter-seed 20260831\n\n\
         See packages/server-rust/benches/soak_harness/README.md for the full flag list\n\
         and the Hetzner 72h runner."
    );
}

fn parse_args() -> Config {
    let mut c = Config::default();
    let args: Vec<String> = std::env::args().collect();
    c.mode_requested = args
        .iter()
        .skip(1)
        .any(|a| KNOWN_FLAGS.contains(&a.as_str()));
    let mut i = 1;
    let need = |i: usize, args: &[String], name: &str| -> String {
        if i + 1 >= args.len() {
            eprintln!("{name} requires a value");
            std::process::exit(2);
        }
        args[i + 1].clone()
    };
    while i < args.len() {
        match args[i].as_str() {
            "--duration" => {
                c.duration = Duration::from_secs(parse_u64(&need(i, &args, "--duration")));
                i += 2;
            }
            "--churn-clients" => {
                c.churn_clients = parse_usize(&need(i, &args, "--churn-clients")).max(1);
                i += 2;
            }
            "--keyspace" => {
                c.keyspace = parse_usize(&need(i, &args, "--keyspace")).max(1);
                i += 2;
            }
            "--write-interval-ms" => {
                c.write_interval =
                    Duration::from_millis(parse_u64(&need(i, &args, "--write-interval-ms")));
                i += 2;
            }
            "--writes-per-life" => {
                c.writes_per_life = parse_usize(&need(i, &args, "--writes-per-life")).max(1);
                i += 2;
            }
            "--offline-keys" => {
                c.offline_keys = parse_usize(&need(i, &args, "--offline-keys"));
                i += 2;
            }
            "--crash-interval" => {
                let v = parse_u64(&need(i, &args, "--crash-interval"));
                c.crash_interval = if v == 0 {
                    None
                } else {
                    Some(Duration::from_secs(v))
                };
                i += 2;
            }
            "--steady-interval" => {
                c.steady_interval =
                    Duration::from_secs(parse_u64(&need(i, &args, "--steady-interval")).max(1));
                i += 2;
            }
            "--quiesce" => {
                c.quiesce = Duration::from_secs(parse_u64(&need(i, &args, "--quiesce")).max(1));
                i += 2;
            }
            "--ready-timeout" => {
                c.ready_timeout =
                    Duration::from_secs(parse_u64(&need(i, &args, "--ready-timeout")).max(1));
                i += 2;
            }
            "--mem-sample-interval" => {
                c.mem_sample_interval =
                    Duration::from_secs(parse_u64(&need(i, &args, "--mem-sample-interval")).max(1));
                i += 2;
            }
            "--mem-threshold-mb-per-hour" => {
                c.mem_threshold_mb_per_hour =
                    parse_f64(&need(i, &args, "--mem-threshold-mb-per-hour"));
                i += 2;
            }
            "--mem-min-growth-mb" => {
                c.mem_min_growth_mb = parse_f64(&need(i, &args, "--mem-min-growth-mb"));
                i += 2;
            }
            "--mem-ceiling-mb" => {
                c.mem_ceiling_mb = parse_f64(&need(i, &args, "--mem-ceiling-mb"));
                i += 2;
            }
            "--server-port" => {
                c.server_port = parse_u64(&need(i, &args, "--server-port")) as u16;
                i += 2;
            }
            "--data-dir" => {
                c.data_dir = Some(PathBuf::from(need(i, &args, "--data-dir")));
                i += 2;
            }
            "--wal-fsync" => {
                c.wal_fsync = need(i, &args, "--wal-fsync");
                i += 2;
            }
            "--or-churn" => {
                c.or_churn = matches!(need(i, &args, "--or-churn").as_str(), "true" | "1" | "on");
                i += 2;
            }
            "--or-keyspace" => {
                c.or_keyspace = parse_usize(&need(i, &args, "--or-keyspace")).max(1);
                i += 2;
            }
            "--or-every" => {
                c.or_every = parse_u64(&need(i, &args, "--or-every")).max(1);
                i += 2;
            }
            "--json-output" => {
                c.json_output = Some(PathBuf::from(need(i, &args, "--json-output")));
                i += 2;
            }
            "--progress-output" => {
                c.progress_output = Some(PathBuf::from(need(i, &args, "--progress-output")));
                i += 2;
            }
            "--inject-divergence" => {
                c.inject_divergence = true;
                i += 1;
            }
            "--inject-panic" => {
                c.inject_panic = true;
                i += 1;
            }
            "--confirm-interval" => {
                c.confirm_interval =
                    Duration::from_secs(parse_u64(&need(i, &args, "--confirm-interval")).max(1));
                i += 2;
            }
            "--no-ack" => {
                c.no_ack = true;
                i += 1;
            }
            "--inject-slow-leak" => {
                c.inject_slow_leak = true;
                i += 1;
            }
            "--no-pre-kill-drain" => {
                c.no_pre_kill_drain = true;
                i += 1;
            }
            "--tombstone-corpus-headroom-bytes" => {
                c.tombstone_corpus_headroom_bytes =
                    parse_u64(&need(i, &args, "--tombstone-corpus-headroom-bytes"));
                i += 2;
            }
            "--tombstone-corpus-ceiling-bytes" => {
                c.tombstone_corpus_ceiling_bytes = Some(parse_u64(&need(
                    i,
                    &args,
                    "--tombstone-corpus-ceiling-bytes",
                )));
                i += 2;
            }
            "--mechanism-report" => {
                c.mechanism_report = true;
                i += 1;
            }
            "--durable-reading" => {
                c.durable_reading = true;
                i += 1;
            }
            "--live-census-interval" => {
                c.live_census_interval_secs = parse_u64(&need(i, &args, "--live-census-interval"));
                i += 2;
            }
            "--sampler-jitter-seed" => {
                c.sampler_jitter_seed = parse_u64(&need(i, &args, "--sampler-jitter-seed"));
                i += 2;
            }
            "--smoke" => {
                // Convenience preset: short but full-feature (used by CI + local).
                c.duration = Duration::from_secs(25);
                c.crash_interval = Some(Duration::from_secs(8));
                c.steady_interval = Duration::from_secs(5);
                c.churn_clients = 8;
                c.keyspace = 64;
                c.quiesce = Duration::from_secs(3);
                c.or_keyspace = 16;
                i += 1;
            }
            // Ignore cargo-injected bench args (e.g. the bench-name filter).
            _ => {
                i += 1;
            }
        }
    }
    // OR-Map churn now runs under --no-pre-kill-drain too. The old root-equality
    // check false-redded here on the benign "recovered-more" asymmetry (the live
    // tree compacts an add+remove pair while WAL replay reconstructs both tags),
    // so OR churn used to be suppressed in this mode. The check is now a
    // DIRECTIONAL no-loss assertion instead: a separate add-only persistent OR
    // keyspace (`ork-persist-*`) seeds an acked-add ledger, and recovery asserts
    // the post-restart observed set is a SUPERSET of those acked adds. Recovered-
    // more is a superset and never reddens; only a missing acked add fails. There
    // is therefore no longer any reason to shed OR writes in no-drain mode — doing
    // so is exactly what this spec re-enables to make WAL-only OR recovery honest.
    c
}

fn parse_u64(s: &str) -> u64 {
    s.parse().unwrap_or_else(|_| {
        eprintln!("expected an integer, got '{s}'");
        std::process::exit(2);
    })
}

fn parse_usize(s: &str) -> usize {
    s.parse().unwrap_or_else(|_| {
        eprintln!("expected an integer, got '{s}'");
        std::process::exit(2);
    })
}

fn parse_f64(s: &str) -> f64 {
    s.parse().unwrap_or_else(|_| {
        eprintln!("expected a number, got '{s}'");
        std::process::exit(2);
    })
}

#[cfg(test)]
mod tests {
    /// The observation fence, asserted rather than assumed: folding live-copy
    /// censuses must leave the durable-corpus estimator's ENTIRE input alone —
    /// its series and both of its scan counters — so an armed live sampler
    /// cannot move a gate's instrument clause, let alone its level clause.
    ///
    /// Everything this proof needs is nested inside it, so the whole fixture
    /// travels with the assertion it exists for and nothing else in this module
    /// can name it.
    #[test]
    fn live_copy_census_never_contaminates_the_corpus_tally() {
        assert_live_copy_census_never_contaminates_the_corpus_tally();
    }

    /// The assertion body, and it is uncalled ON PURPOSE.
    ///
    /// This bench target is declared `harness = false`, so rustc strips every
    /// `#[test]` item from it: the wrapper above is removed before type-checking
    /// and takes its only call to this function with it. Siting the body here
    /// instead is what keeps it COMPILED — `cargo clippy --all-targets` builds
    /// this module — so it cannot rot silently while the code it asserts over
    /// moves underneath it. EXECUTING it needs a target with a test harness,
    /// which this harness's file ledger does not have.
    #[allow(dead_code)]
    fn assert_live_copy_census_never_contaminates_the_corpus_tally() {
        use super::*;
        use topgun_core::hlc::Timestamp;
        use topgun_core::types::Value;
        use topgun_server::storage::record::OrMapEntry;

        /// Write one OR-Map row — live entries plus tombstones — into a real
        /// store file under `dir`, in the same table and encoding the scan
        /// reads. A real file rather than a stub, so the path under test is the
        /// whole path: copy, open, iterate, fold.
        fn write_or_fixture(dir: &Path) {
            let db = redb::Database::create(dir.join("topgun.redb")).expect("create store file");
            let table_name = format!("map__{OR_MAP}");
            let table_def: redb::TableDefinition<&str, &[u8]> =
                redb::TableDefinition::new(&table_name);
            let value = RecordValue::OrMap {
                records: vec![OrMapEntry {
                    value: Value::String("alive".to_string()),
                    tag: "live-tag".to_string(),
                    timestamp: Timestamp {
                        millis: 1,
                        counter: 0,
                        node_id: "live-census-fixture".to_string(),
                    },
                }],
                tombstones: vec!["dead-tag-a".to_string(), "dead-tag-b".to_string()],
            };
            let encoded = rmp_serde::to_vec_named(&value).expect("encode record");
            let write_txn = db.begin_write().expect("begin write");
            {
                let mut table = write_txn.open_table(table_def).expect("open table");
                table
                    .insert("ork-1", encoded.as_slice())
                    .expect("insert row");
            }
            write_txn.commit().expect("commit");
        }

        let data = tempfile::tempdir().expect("data dir");
        let scratch = tempfile::tempdir().expect("scratch dir");
        write_or_fixture(data.path());
        let sampler = CorpusSampler::new(data.path().to_path_buf(), scratch.path().to_path_buf());

        for i in 0..3 {
            sample_live_census_via_copy(&sampler, f64::from(i));
        }

        let corpus = sampler.tally.lock().clone();
        assert!(
            corpus.samples.is_empty(),
            "live-copy censuses must never reach the estimator's series"
        );
        assert_eq!(
            corpus.scans_attempted, 0,
            "live-copy censuses must never be counted as corpus scan attempts"
        );
        assert_eq!(
            corpus.scans_failed, 0,
            "live-copy censuses must never be counted as corpus scan failures"
        );
        assert!(
            corpus.censuses.is_empty(),
            "live-copy censuses must never reach the corpus tally's census list"
        );

        let live = sampler.live_tally.lock().clone();
        assert_eq!(live.scans_attempted, 3);
        assert_eq!(live.scans_failed, 0);
        assert_eq!(live.censuses.len(), 3);
        assert!(
            live.censuses
                .iter()
                .all(|r| r.source == CensusSource::LiveCopy),
            "every record from this sampler carries the live-copy source"
        );
        // The census is really taken, not merely recorded as an empty shell:
        // one row, one live entry, two tombstones.
        let first = live.censuses[0].census;
        assert_eq!(first.keys_scanned, 1);
        assert_eq!(first.or_map_keys, 1);
        assert_eq!(first.live_entries, 1);
        assert_eq!(first.tombstone_entries, 2);
    }
}
