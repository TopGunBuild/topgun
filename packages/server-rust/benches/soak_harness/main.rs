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
//! 3. **Bounded memory:** with a fixed keyspace overwritten in place, the
//!    server RSS must plateau; a sustained upward slope flags a leak (e.g.
//!    unbounded OR-Map tombstone growth, TODO-479/480).
//! 4. **Zero panics:** any panic marker in server output, or any un-requested
//!    exit, fails the run with the captured context.
//!
//! Two **negative controls** prove the harness can actually fail:
//! `--inject-divergence` makes the convergence check go red, and
//! `--inject-panic` makes the panic capture go red. A soak that cannot fail
//! proves nothing.
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
mod process;
mod report;

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use parking_lot::Mutex;
use tokio::io::{AsyncBufReadExt, BufReader};

use client::SoakClient;
use model::{compare, next_stamp, Model};
use monitor::{assess, sample_rss_mb, MemSample};
use process::{resolve_server_binary, ServerConfig, ServerSupervisor};
use report::{
    append_progress, utc_timestamp_now, write_report, MemoryReport, ProgressSnapshot, SoakReport,
};

/// Subject index used by the orchestrator's verifier connections. Far above the
/// churn-client range so it never owns keys or collides with churn auth.
const VERIFIER_IDX: usize = 1_000_000;

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
            mem_threshold_mb_per_hour: 50.0,
            mem_min_growth_mb: 150.0,
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
            no_pre_kill_drain: false,
            mode_requested: false,
        }
    }
}

/// Shared atomic counters mutated by churn clients.
#[derive(Default)]
struct SoakMetrics {
    total_writes: AtomicU64,
    write_errors: AtomicU64,
    reconnects: AtomicU64,
    resends: AtomicU64,
}

/// Context shared with every churn client task.
struct ChurnCtx {
    supervisor: Arc<ServerSupervisor>,
    model: Arc<Model>,
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
    let metrics = Arc::new(SoakMetrics::default());
    let paused = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));

    // --- Spawn churn clients ---
    let mut churn_handles = Vec::with_capacity(config.churn_clients);
    for idx in 0..config.churn_clients {
        let ctx = ChurnCtx {
            supervisor: Arc::clone(&supervisor),
            model: Arc::clone(&model),
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

    // --- Spawn memory sampler ---
    let samples: Arc<Mutex<Vec<MemSample>>> = Arc::new(Mutex::new(Vec::new()));
    let peak_rss = Arc::new(Mutex::new(0.0_f64));
    {
        let supervisor = Arc::clone(&supervisor);
        let samples = Arc::clone(&samples);
        let peak_rss = Arc::clone(&peak_rss);
        let stop = Arc::clone(&stop);
        let interval = config.mem_sample_interval;
        let start = Instant::now();
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
            match recovery_checkpoint(&supervisor, &model, &jwt_secret, config, &paused).await {
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
             converged={} rss={:.0}MB(peak {:.0})",
            start.elapsed().as_secs(),
            metrics.total_writes.load(Ordering::Relaxed),
            metrics.write_errors.load(Ordering::Relaxed),
            metrics.reconnects.load(Ordering::Relaxed),
            crashes,
            steady_checkpoints,
            recovery_checkpoints,
            last_convergence_ok,
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

    // --- Assess memory ---
    let mem_samples = samples.lock().clone();
    let mem = assess(
        &mem_samples,
        config.mem_threshold_mb_per_hour,
        config.mem_min_growth_mb,
        config.mem_ceiling_mb,
    );

    let panic_report = panic_watch.report();
    let passed = convergence_failures.is_empty()
        && recovery_failures.is_empty()
        && mem.passed
        && panic_report.is_none();

    if !mem.passed {
        finished_reason = format!(
            "memory growth assertion failed: {}",
            mem.reason.clone().unwrap_or_default()
        );
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

    print_summary(&report);
    if let Some(path) = &config.json_output {
        write_report(path, &report);
        println!("wrote JSON report to {}", path.display());
    }

    i32::from(!passed)
}

fn print_summary(r: &SoakReport) {
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
    println!("steady_checkpts:   {}", r.steady_checkpoints);
    println!(
        "recovery_checkpts: {} (crashes {})",
        r.recovery_checkpoints, r.crashes
    );
    println!(
        "memory:            first={:.0}MB peak={:.0}MB last={:.0}MB slope={:.1}MB/h -> {}",
        r.memory.first_mb,
        r.memory.peak_mb,
        r.memory.last_mb,
        r.memory.slope_mb_per_hour,
        if r.memory.passed { "ok" } else { "FAIL" }
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
    jwt: &str,
    config: &Config,
    paused: &Arc<AtomicBool>,
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

    // Pre-crash snapshot (also a steady convergence check).
    let (pre_lww, pre_root, pre_or_root) = {
        let mut v = SoakClient::connect(supervisor.addr(), VERIFIER_IDX, jwt).await?;
        let lww = v.read_all(LWW_MAP).await?;
        let root = v.merkle_root(LWW_MAP).await?;
        let or_root = if config.or_churn {
            Some(v.merkle_root(OR_MAP).await?)
        } else {
            None
        };
        (lww, root, or_root)
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

    // kill -9 + restart against the same redb + WAL.
    if let Err(e) = supervisor.restart(config.ready_timeout).await {
        out.hard
            .push(format!("server failed to restart after kill -9: {e}"));
        paused.store(false, Ordering::SeqCst);
        return Ok(out);
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
    let (post_query, post_delta, post_root, post_or_root) = {
        let mut v = SoakClient::connect(supervisor.addr(), VERIFIER_IDX, jwt).await?;
        let query = v.read_all(LWW_MAP).await?;
        let delta = v.delta_sync_all(LWW_MAP).await?;
        let root = v.merkle_root(LWW_MAP).await?;
        let or_root = if config.or_churn {
            Some(v.merkle_root(OR_MAP).await?)
        } else {
            None
        };
        (query, delta, root, or_root)
    };

    // HARD: DurableMerkleIndex (SPEC-325b) builds from the datastore, not the
    // resident set, so the Merkle root must survive a kill -9 + restart unchanged.
    if post_root != pre_root {
        out.hard.push(format!(
            "LWW merkle root changed across recovery: pre={pre_root} post={post_root}"
        ));
    }
    // HARD (unconditional): the OR-Map merkle root must survive recovery unchanged.
    // This is the OR-Map's only durability assertion, so it must never be skipped —
    // a blanket skip would let an acked OR-Map write lost on kill -9 pass silently.
    // Under --no-pre-kill-drain OR churn is disabled (see parse_args), so both roots
    // are absent and this holds honestly; OR-Map crash-recovery under load is still
    // covered by the drained mode (its WAL-only behavior is a tracked follow-up).
    if pre_or_root != post_or_root {
        out.hard.push(format!(
            "OR-Map merkle root changed across recovery: pre={pre_or_root:?} post={post_or_root:?}"
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

            // OR-Map add/remove to drive tombstone growth (memory watch).
            write_count += 1;
            if ctx.or_churn && write_count.is_multiple_of(ctx.or_every) {
                let or_key = format!("ork-{}", slot % ctx.or_keyspace.max(1));
                let tag = format!("{ms}:{ctr}:{idx}");
                if client.or_add(OR_MAP, &or_key, &tag, ms, ctr).await.is_ok() {
                    if client.or_remove(OR_MAP, &or_key, &tag).await.is_err() {
                        session_alive = false;
                        break;
                    }
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
         \x20 soak_harness --inject-panic\n\n\
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
    // The no-drain validator scopes strictly to the LWW acked==durable
    // target. OR-Map WAL-only crash-recovery semantics — the live net-compacted
    // observed set vs the WAL-replayed intermediate tags — is a distinct question
    // that needs its own audit. Rather than run OR churn and relax its assertion
    // (which would let a genuine OR-Map acked-write loss pass silently), we simply
    // do not generate OR writes in this mode, so the OR-root equality check stays
    // unconditionally HARD and holds honestly (both roots absent).
    if c.no_pre_kill_drain {
        c.or_churn = false;
    }
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
