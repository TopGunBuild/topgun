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
//!    The slope is a HARD gate: a tracked-and-ACKing client
//!    (`SoakClient::connect_tracked` + `confirm_apply`) is driven alongside the
//!    churn clients for the run's duration so the server's per-device causal
//!    frontier — and therefore its low-water-mark — actually advances, which is
//!    what lets the epoch-scoped prune fire and the gauge genuinely plateau
//!    under sustained churn instead of only ever growing. The min-window-span
//!    guard (`DEFAULT_TOMBSTONE_BYTES_MIN_WINDOW_SECS`) keeps a
//!    too-short-to-plateau run (e.g. the 25s blocking smoke) from false-REDing
//!    on a not-yet-flattened ramp, and the blind-monitor (zero-sample) clause
//!    hard-gates independently of the slope. Server RSS is sampled in parallel
//!    and asserted against a looser slope as a coarse, non-tombstone backstop
//!    gate. The on-disk data-dir slope (below) remains REPORT-ONLY — bounding
//!    it is a separate, not-yet-landed follow-up.
//! 4. **Zero panics:** any panic marker in server output, or any un-requested
//!    exit, fails the run with the captured context.
//!
//! Two **negative controls** prove the harness can actually fail:
//! `--inject-divergence` makes the convergence check go red, and
//! `--inject-panic` makes the panic capture go red. A soak that cannot fail
//! proves nothing. Two more MODES exist specifically to prove the promoted
//! tombstone-byte hard gate above is honest: `--no-ack` disables the tracked
//! client's confirm-apply loop (the low-water-mark then never advances, prune
//! never fires, and the gate must FAIL under sustained churn), and
//! `--inject-slow-leak` adds a second, deliberately slow-acking tracked client
//! whose stale cursor repeatedly caps the fleet-wide low-water-mark — a bounded
//! ramp-then-catch-up pattern used to calibrate the OLS slope's detection floor
//! against a small, non-instantaneous leak rather than only total blockage.
//! Independently of the gauge, `scan_redb_tombstone_corpus` opens the server's
//! own redb file after it exits and sums the real on-disk tombstone corpus —
//! the positive control's cross-check against a gauge/corpus divergence (e.g. a
//! legacy `OrTombstones` blob the hot-path gauge never counted on add).
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
    assess, assess_disk, assess_tombstone_bytes, exclude_boot_gap_samples, sample_disk_mb,
    sample_rss_mb, BootGap, DiskAssessment, DiskSample, MemSample, TombstoneAssessment,
    TombstoneSample, DEFAULT_DISK_CEILING_MB, DEFAULT_DISK_MIN_GROWTH_MB,
    DEFAULT_DISK_THRESHOLD_MB_PER_HOUR, DEFAULT_TOMBSTONE_BYTES_MIN_GROWTH,
    DEFAULT_TOMBSTONE_BYTES_MIN_WINDOW_SECS, DEFAULT_TOMBSTONE_BYTES_THRESHOLD_PER_HOUR,
};
use or_noloss::{missing_acked_adds, OrLedger};
use process::{resolve_server_binary, ServerConfig, ServerSupervisor};
use report::{
    append_progress, utc_timestamp_now, write_report, MemoryReport, ProgressSnapshot, SoakReport,
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
    /// prune never fires, and sustained OR churn must trip the promoted
    /// tombstone-byte hard gate.
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
        let interval = config.mem_sample_interval;
        let start = sampler_start;
        let http = reqwest::Client::new();
        tokio::spawn(async move {
            loop {
                if stop.load(Ordering::SeqCst) {
                    return;
                }
                if let Some(bytes) = scrape_tombstone_bytes(&http, port).await {
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
                    if !exclude_boot_gap_samples(std::slice::from_ref(&candidate), &gaps_snapshot)
                        .is_empty()
                    {
                        samples.lock().push(candidate);
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

    let boot_gap_clock = BootGapClock {
        sampler_start,
        boot_gaps: Arc::clone(&boot_gaps),
    };

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
    let mut finished_reason = "duration reached".to_string();

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
    stop.store(true, Ordering::SeqCst);
    paused.store(false, Ordering::SeqCst);
    for h in churn_handles {
        let _ = tokio::time::timeout(Duration::from_secs(5), h).await;
    }
    supervisor.shutdown().await;

    // Positive-control independent corpus assertion (R5): the server process
    // has just been reaped, so its redb handle is released and this scan can
    // safely open the same file. REPORT-ONLY (printed in the console summary,
    // never asserted into `passed`) — see `scan_redb_tombstone_corpus`'s doc
    // for why a divergence from the gauge's `last_bytes` below is exactly the
    // failure mode this exists to catch.
    let redb_tombstone_scan = scan_redb_tombstone_corpus(&data_dir, OR_MAP);

    // --- Assess memory (secondary/backstop gate) ---
    let mem_samples = samples.lock().clone();
    let mem = assess(
        &mem_samples,
        config.mem_threshold_mb_per_hour,
        config.mem_min_growth_mb,
        config.mem_ceiling_mb,
    );

    // --- Assess tombstone-byte growth (direct residency-independent leak
    // instrument, the bounded-plateau signal — a HARD gate, see the promotion
    // note below). Coexists with the RSS gate above as a coarse non-tombstone
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

    // The tombstone-byte SLOPE clause is now a HARD gate (promoted from
    // report-only): the tracked-and-ACKing client spawned above
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
    let passed = convergence_failures.is_empty()
        && recovery_failures.is_empty()
        && mem.passed
        && !blind_monitor
        && tombstones.passed
        && !disk_blind_monitor
        && panic_report.is_none();

    if !mem.passed {
        finished_reason = format!(
            "memory growth assertion failed: {}",
            mem.reason.clone().unwrap_or_default()
        );
    }
    if blind_monitor {
        finished_reason = format!(
            "tombstone-byte monitoring blind: {}",
            tombstones.reason.clone().unwrap_or_default()
        );
    } else if !tombstones.passed {
        // HARD gate (promoted from report-only): with the tracked-and-ACKing
        // client driving the low-water-mark, a sustained slope breach past the
        // min-window-guarded threshold means the epoch-scoped prune is not
        // keeping up with (or has stopped) bounding tombstone growth — a real
        // regression, not an expected/known gap. AND it into `passed`.
        finished_reason = format!(
            "tombstone-byte growth slope {:.1} bytes/h exceeds {:.1} bytes/h: {}",
            tombstones.slope_bytes_per_hour,
            DEFAULT_TOMBSTONE_BYTES_THRESHOLD_PER_HOUR,
            tombstones.reason.clone().unwrap_or_default()
        );
    }
    if disk_blind_monitor {
        finished_reason = format!(
            "disk monitoring blind: {}",
            disk.reason.clone().unwrap_or_default()
        );
    } else if !disk.passed {
        // Report-only, same rationale as the tombstone-byte slope above: the
        // pre-566 OR churn grows the durable dir linearly by design, so this is
        // the EXPECTED honest signal, not a regression. Do NOT AND this into
        // `passed`.
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

    let report = SoakReport {
        mode: "soak".to_string(),
        duration_secs_target: config.duration.as_secs(),
        duration_secs_actual: start.elapsed().as_secs(),
        churn_clients: config.churn_clients,
        keyspace: config.keyspace,
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
        panic_report,
        passed,
        finished_reason: finished_reason.clone(),
        timestamp: utc_timestamp_now(),
    };

    print_summary(
        &report,
        &tombstones,
        &disk,
        redb_tombstone_scan,
        &ConfirmDiagnostics {
            confirms: metrics.confirms.load(Ordering::Relaxed),
            last_confirmed_epoch: metrics.last_confirmed_epoch.load(Ordering::Relaxed),
            confirm_errors: metrics.confirm_errors.load(Ordering::Relaxed),
        },
    );
    if let Some(path) = &config.json_output {
        write_report(path, &report);
        println!("wrote JSON report to {}", path.display());
    }
    // NOTE: neither the tombstone-byte nor the disk verdict is a field on
    // `SoakReport` (report.rs) — both are asserted into `passed`/`finished_reason`
    // above and printed in the console summary below, but adding structured
    // fields there would be a 6th touched file against this spec's 5-file cap.
    // Tracked as a follow-up so JSON consumers (CI dashboards) gain the same
    // visibility the console already has.

    i32::from(!passed)
}

/// Tracked-client confirm-apply diagnostics, surfaced in the summary without
/// widening the persisted `SoakReport` (report.rs is outside this change's file
/// budget). Purely a console signal for distinguishing a healthy LWM-advancing
/// run from a stalled-confirm harness failure.
struct ConfirmDiagnostics {
    confirms: u64,
    last_confirmed_epoch: u64,
    confirm_errors: u64,
}

fn print_summary(
    r: &SoakReport,
    tombstones: &TombstoneAssessment,
    disk: &DiskAssessment,
    redb_tombstone_scan: Option<u64>,
    confirm: &ConfirmDiagnostics,
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
        confirm.confirms, confirm.last_confirmed_epoch, confirm.confirm_errors
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
    // The byte SLOPE is now a HARD gate: the tracked-and-ACKing client drives
    // the low-water-mark forward, so the epoch-scoped prune actually fires and
    // the gauge is expected to plateau under sustained churn (subject to the
    // min-window-span guard and boot-gap exclusion). See the run-end verdict
    // rationale.
    let tombstone_role = "slope + blind-monitor both hard-gate";
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
    // Positive-control cross-check (R5): an independent, gauge-free scan of the
    // real on-disk tombstone corpus, taken after the server process exited. A
    // large divergence from `tombstones.last_bytes` above means the exported
    // gauge is not tracking the true corpus (e.g. an un-migrated legacy
    // `OrTombstones` blob the hot-path gauge never added on read) — REPORT-ONLY,
    // never asserted into `passed`.
    match redb_tombstone_scan {
        Some(scanned_bytes) => {
            let gauge_bytes = tombstones.last_bytes;
            let diverged = scanned_bytes != gauge_bytes;
            println!(
                "tombstone_corpus_redb_scan: {scanned_bytes} bytes (independent of gauge; last \
                 gauge value {gauge_bytes} bytes) -> {} (positive-control cross-check, \
                 report-only)",
                if diverged { "DIVERGED" } else { "match" }
            );
        }
        None => {
            println!(
                "tombstone_corpus_redb_scan: could not scan (missing/corrupt redb file or \
                 table) — positive-control cross-check skipped, report-only"
            );
        }
    }
    // Unlike the tombstone-byte slope above (now a hard gate), the disk slope
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
async fn scrape_tombstone_bytes(http: &reqwest::Client, port: u16) -> Option<u64> {
    let url = format!("http://127.0.0.1:{port}/metrics");
    let resp = http.get(&url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body = resp.text().await.ok()?;
    parse_tombstone_bytes_gauge(&body)
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

/// Independent, gauge-free scan of the OR-Map's on-disk tombstone corpus: the
/// positive control's cross-check that `topgun_ormap_tombstone_bytes` is
/// actually tracking the real durable byte total, not merely plateauing
/// because it drifted out of sync with it.
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
/// MUST run only after `ServerSupervisor::shutdown` has reaped the child
/// process: redb is single-writer, so the server's own handle on the file must
/// already be released before this process can open it. This is therefore a
/// post-teardown assertion, not a live sampler alongside RSS/tombstone-bytes/
/// disk above. Returns `None` on any failure (file missing, corrupt header,
/// table absent) — best-effort, mirroring `sample_rss_mb`/`sample_disk_mb`'s
/// None-on-failure contract — so a scan failure is reported as "could not
/// scan", never mistaken for a genuinely empty (zero-byte) corpus.
fn scan_redb_tombstone_corpus(data_dir: &Path, map: &str) -> Option<u64> {
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
        // Never-written table (e.g. no OR-Map write ever landed) contributes 0
        // tombstone bytes — a real, honest answer, not a scan failure.
        Err(redb::TableError::TableDoesNotExist(_)) => return Some(0),
        Err(_) => return None,
    };

    let mut total_bytes: u64 = 0;
    let iter = table.iter().ok()?;
    for entry in iter {
        let (_key_guard, val_guard) = entry.ok()?;
        let Ok(value) = rmp_serde::from_slice::<RecordValue>(val_guard.value()) else {
            // An undecodable value is a corrupt/foreign row, not part of this
            // scan's remit — skip it rather than aborting the whole scan.
            continue;
        };
        match value {
            RecordValue::OrMap { tombstones, .. } => {
                total_bytes += tombstones.iter().map(|t| t.len() as u64).sum::<u64>();
            }
            // Legacy pre-migration shape — see the function doc's divergence
            // rationale for why this is exactly what the gauge can miss.
            RecordValue::OrTombstones { tags } => {
                total_bytes += tags.iter().map(|t| t.len() as u64).sum::<u64>();
            }
            RecordValue::Lww { .. } => {}
        }
    }
    Some(total_bytes)
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
async fn recovery_checkpoint(
    supervisor: &Arc<ServerSupervisor>,
    model: &Arc<Model>,
    or_ledger: &Arc<OrLedger>,
    jwt: &str,
    config: &Config,
    paused: &Arc<AtomicBool>,
    boot_gap_clock: &BootGapClock,
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
    if let Err(e) = supervisor.restart(config.ready_timeout).await {
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
/// 0 for the whole run: the negative control for the promoted hard gate.
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
                        // climbs → hard gate REDs), so an invisible plumbing bug
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
    "--smoke",
];

fn print_usage() {
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
         \x20 soak_harness --no-ack --duration 3600  # tombstone hard-gate must FAIL\n\
         \x20 soak_harness --inject-slow-leak --duration 3600  # slope detection-floor calibration\n\n\
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
