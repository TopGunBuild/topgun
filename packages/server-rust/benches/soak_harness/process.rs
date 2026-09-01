//! Out-of-process `topgun-server` supervisor.
//!
//! The soak harness must `kill -9` the server and watch it recover real data
//! from a real redb + WAL on disk — neither is possible with the in-process
//! load-harness server (which runs on a `NullDataStore` inside the test
//! process). This supervisor therefore launches the actual `topgun-server`
//! binary (located via `CARGO_BIN_EXE_topgun-server`) as a child process,
//! pointed at a fixed port and a persistent data directory, and can SIGKILL +
//! relaunch it against the same files to exercise WAL recovery repeatedly.
//!
//! Every line the child writes to stdout/stderr is mirrored into a bounded ring
//! buffer and scanned for panic markers as it arrives, so a panic anywhere in a
//! 72-hour run is captured with surrounding context — not silently swallowed.

use std::collections::VecDeque;
use std::net::{SocketAddr, TcpListener};
use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use parking_lot::Mutex;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::oneshot;

/// Substrings that mark a Rust panic / abnormal runtime abort in server output.
const PANIC_MARKERS: &[&str] = &[
    "panicked at",
    "fatal runtime error",
    "stack backtrace:",
    "Aborted (core dumped)",
    "SIGABRT",
];

/// Number of recent log lines retained for crash-context reporting.
const LOG_RING_CAPACITY: usize = 400;

/// The log target of the removal-site observation line the origin instrument
/// reads.
///
/// The target is what makes the line selectable without a discriminant field,
/// so the capture matches on it rather than on message text. Declared here
/// beside the capture that consumes it.
pub const ORIGIN_TARGET: &str = "topgun_server::tombstone_frontier::removal";

/// Maximum origin lines retained by [`OriginCapture`].
///
/// Roughly two orders of magnitude above the per-run line cadence the existing
/// data implies, so overflow is not expected — and if it happens anyway it is
/// never silent: lines beyond this bound increment the capture's drop counter,
/// and a non-zero drop count forces the fail-closed origin reading. Retention
/// is in the HARNESS process, whose resident set nothing in this run measures.
pub const ORIGIN_CAPTURE_CAPACITY: usize = 50_000;

/// Bounded sink for the removal-site observation lines, fed from the same
/// per-line reader that drives the panic watch.
///
/// The counters are carried beside the retained lines so a reader never has to
/// infer how many lines were seen from how many were kept.
pub struct OriginCapture {
    /// Retained lines, capped at [`ORIGIN_CAPTURE_CAPACITY`].
    lines: Mutex<Vec<String>>,
    /// Lines that matched [`ORIGIN_TARGET`], retained or not.
    matched: std::sync::atomic::AtomicU64,
    /// Matched lines refused because the retention cap was reached.
    dropped: std::sync::atomic::AtomicU64,
}

impl OriginCapture {
    /// Construct an empty capture. Private because the supervisor is the only
    /// producer of one; the inline tests reach it from inside this module.
    fn new() -> Arc<Self> {
        Arc::new(Self {
            lines: Mutex::new(Vec::new()),
            matched: std::sync::atomic::AtomicU64::new(0),
            dropped: std::sync::atomic::AtomicU64::new(0),
        })
    }

    /// Record one child output line, retaining it when it carries the origin
    /// target and the retention cap has not been reached.
    ///
    /// The line is ANSI-stripped FIRST and both the match and the retained
    /// bytes are the stripped form. The child colours its output
    /// unconditionally, so an escape sequence landing inside the target
    /// substring makes a raw match miss a line the emitter did produce — an
    /// instrument fault that reads, downstream, as a silent emitter. Stripping
    /// at this boundary is also the only place the normalization can happen
    /// exactly ONCE per line: the match and the evidence then see the same
    /// bytes and no consumer has to re-derive whether what it holds was
    /// normalized. The PANIC WATCH is deliberately fed the RAW line by the
    /// caller and is unaffected by this — it matches payload text, not
    /// structured fields, and its report is meant to quote what the child
    /// actually printed.
    ///
    /// Matching is on the TARGET substring rather than on message text, which
    /// is what lets the line be selected without a discriminant field. A match
    /// arriving once the cap is full is COUNTED as a drop and never silently
    /// discarded: the drop counter is the evidence that the captured set is
    /// incomplete, and it is what forces the fail-closed reading downstream. A
    /// capture that discarded quietly would let a truncated run be read as a
    /// complete one.
    pub fn record_line(&self, line: &str) {
        let normalized = strip_ansi(line);
        if !normalized.contains(ORIGIN_TARGET) {
            return;
        }
        // Both counters move under the same lock the retained lines do, so a
        // snapshot can never observe a matched line that is neither retained
        // nor counted as dropped.
        let mut lines = self.lines.lock();
        self.matched.fetch_add(1, Ordering::SeqCst);
        if lines.len() >= ORIGIN_CAPTURE_CAPACITY {
            self.dropped.fetch_add(1, Ordering::SeqCst);
        } else {
            lines.push(normalized);
        }
    }

    /// One consistent view of the retained lines and both counters.
    ///
    /// The counters travel WITH the lines so a reader never has to infer how
    /// many lines were seen from how many were kept.
    pub fn snapshot(&self) -> OriginCaptureSnapshot {
        let lines = self.lines.lock();
        OriginCaptureSnapshot {
            lines: lines.clone(),
            matched: self.matched.load(Ordering::SeqCst),
            dropped: self.dropped.load(Ordering::SeqCst),
        }
    }
}

/// A consistent view of an [`OriginCapture`]: the retained lines plus the two
/// counters, all taken under one lock.
///
/// A named struct rather than a tuple because the two counters are not
/// interchangeable — `dropped` is fail-closed evidence, and a transposition at
/// the wiring site would turn a discarded line into a matched one.
pub struct OriginCaptureSnapshot {
    /// The retained lines, in arrival order, capped at
    /// [`ORIGIN_CAPTURE_CAPACITY`].
    pub lines: Vec<String>,
    /// Lines that matched [`ORIGIN_TARGET`], retained or not.
    pub matched: u64,
    /// Matched lines refused because the retention cap was reached.
    pub dropped: u64,
}

/// Configuration for launching the server child.
#[derive(Clone)]
pub struct ServerConfig {
    /// Absolute path to the `topgun-server` binary.
    pub binary: PathBuf,
    /// Persistent data directory (holds the redb file and WAL subdir).
    pub data_dir: PathBuf,
    /// Loopback port the server binds; stable across restarts so clients
    /// reconnect to the same address.
    pub port: u16,
    /// JWT signing secret shared with `SoakClient`.
    pub jwt_secret: String,
    /// WAL fsync policy forwarded to `TOPGUN_WAL_FSYNC_POLICY`. Accepted spellings:
    /// `per_op` (also `perop`, `per-op`, case-insensitive) | `batched` | `none`.
    /// The soak relies on `per_op` for acked == durable on `kill -9`.
    pub wal_fsync_policy: String,
}

/// Captured server output plus a tripwire set the instant a panic marker is seen.
pub struct PanicWatch {
    tripped: AtomicBool,
    /// The first panic line observed, with a few preceding lines for context.
    report: Mutex<Option<String>>,
    ring: Mutex<VecDeque<String>>,
}

impl PanicWatch {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            tripped: AtomicBool::new(false),
            report: Mutex::new(None),
            ring: Mutex::new(VecDeque::with_capacity(LOG_RING_CAPACITY)),
        })
    }

    /// Construct a standalone watch not attached to a server, used by the
    /// `--inject-panic` negative control to drive the real detection code
    /// against synthetic panic output.
    pub fn new_standalone() -> Arc<Self> {
        Self::new()
    }

    /// Record one output line: append to the ring and, if it matches a panic
    /// marker, trip the watch and snapshot recent context. This is the function
    /// the `--inject-panic` negative control exercises directly.
    pub fn record_line(&self, line: &str) {
        {
            let mut ring = self.ring.lock();
            if ring.len() == LOG_RING_CAPACITY {
                ring.pop_front();
            }
            ring.push_back(line.to_string());
        }
        if PANIC_MARKERS.iter().any(|m| line.contains(m))
            && !self.tripped.swap(true, Ordering::SeqCst)
        {
            let ring = self.ring.lock();
            let context: Vec<String> = ring.iter().rev().take(20).rev().cloned().collect();
            *self.report.lock() = Some(format!(
                "panic marker in server output:\n  >>> {line}\n  context (most recent lines):\n{}",
                context
                    .iter()
                    .map(|l| format!("    {l}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }
    }

    /// True once any panic marker has been seen.
    pub fn tripped(&self) -> bool {
        self.tripped.load(Ordering::SeqCst)
    }

    /// The captured panic report, if any.
    pub fn report(&self) -> Option<String> {
        self.report.lock().clone()
    }

    /// Force a report from an unexpected (un-requested) child exit, e.g. exit
    /// code 101 (panic=unwind) or a non-SIGKILL termination signal.
    pub fn record_unexpected_exit(&self, detail: &str) {
        if !self.tripped.swap(true, Ordering::SeqCst) {
            let ring = self.ring.lock();
            let context: Vec<String> = ring.iter().rev().take(20).rev().cloned().collect();
            *self.report.lock() = Some(format!(
                "unexpected server exit: {detail}\n  context (most recent lines):\n{}",
                context
                    .iter()
                    .map(|l| format!("    {l}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }
    }
}

/// Supervises one server child at a time, restarting it on demand.
pub struct ServerSupervisor {
    config: ServerConfig,
    child: Mutex<Option<Child>>,
    /// Set true immediately before a deliberate SIGKILL so the exit watcher does
    /// not misreport our own `kill -9` as a crash.
    intentional_kill: Arc<AtomicBool>,
    panic_watch: Arc<PanicWatch>,
    /// Bounded sink for the removal-site observation lines, shared by the
    /// readers of every child generation so a restart does not reset it. Built
    /// here rather than passed in, which is what keeps `ServerConfig`'s fields
    /// and this supervisor's constructor signature unchanged.
    origin_capture: Arc<OriginCapture>,
}

impl ServerSupervisor {
    /// Build a supervisor; does not start the child yet.
    pub fn new(config: ServerConfig) -> Arc<Self> {
        Arc::new(Self {
            config,
            child: Mutex::new(None),
            intentional_kill: Arc::new(AtomicBool::new(false)),
            panic_watch: PanicWatch::new(),
            origin_capture: OriginCapture::new(),
        })
    }

    /// Address clients connect to (stable across restarts).
    pub fn addr(&self) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], self.config.port))
    }

    pub fn panic_watch(&self) -> Arc<PanicWatch> {
        Arc::clone(&self.panic_watch)
    }

    /// The origin-line sink fed by every child generation's output readers.
    pub fn origin_capture(&self) -> Arc<OriginCapture> {
        Arc::clone(&self.origin_capture)
    }

    /// PID of the currently running child, if any. Changes across restarts, so
    /// the memory sampler must re-read it every tick.
    pub fn current_pid(&self) -> Option<u32> {
        self.child
            .lock()
            .as_ref()
            .and_then(tokio::process::Child::id)
    }

    /// Pick a free loopback port by binding `:0` and immediately releasing it.
    /// Used when the caller passes `--server-port 0` so restarts reuse one port.
    pub fn pick_free_port() -> Result<u16> {
        let listener = TcpListener::bind("127.0.0.1:0").context("probe free port")?;
        let port = listener.local_addr()?.port();
        drop(listener);
        Ok(port)
    }

    /// Launch the child and block until it prints its `PORT=` readiness line
    /// (which the server emits only after WAL recovery completes and the
    /// listener is bound). Returns an error if the child dies or the line does
    /// not appear within `ready_timeout`.
    pub async fn start(self: &Arc<Self>, ready_timeout: Duration) -> Result<()> {
        let mut cmd = Command::new(&self.config.binary);
        cmd.arg("--port")
            .arg(self.config.port.to_string())
            .env("STORAGE_BACKEND", "redb")
            .env("TOPGUN_REDB_PATH", self.config.data_dir.join("topgun.redb"))
            .env("TOPGUN_WAL_DIR", self.config.data_dir.join("wal"))
            .env("TOPGUN_WAL_FSYNC_POLICY", &self.config.wal_fsync_policy)
            .env("TOPGUN_BIND_ADDR", "127.0.0.1")
            .env("JWT_SECRET", &self.config.jwt_secret)
            // Crash recovery (AC1/G4b) asserts that DURABLE state survives a
            // `kill -9` and is re-served residency-independently. It does NOT
            // assert the in-flight write-behind window is durable — acked writes
            // buffer ~1s before persisting, and losing that window on an unclean
            // kill is an accepted demo-tier tradeoff tracked separately (TODO-339).
            // So the buffer must be flushed to redb+WAL before the kill, otherwise
            // the recovery checkpoint races the flush and reports phantom one-behind
            // losses. The checkpoint pauses churn and quiesces; a fast flush
            // interval guarantees the (now-static) backlog drains inside that
            // window regardless of load. Operator can override to exercise the
            // production-default flush cadence explicitly.
            .env(
                "TOPGUN_WRITEBEHIND_FLUSH_INTERVAL_MS",
                std::env::var("TOPGUN_WRITEBEHIND_FLUSH_INTERVAL_MS")
                    .unwrap_or_else(|_| "100".to_string()),
            )
            .env(
                "TOPGUN_WRITEBEHIND_BATCH_SIZE",
                std::env::var("TOPGUN_WRITEBEHIND_BATCH_SIZE")
                    .unwrap_or_else(|_| "5000".to_string()),
            )
            // Keep journal on (production default) so the soak measures the real
            // write path; capacity small since the soak never reads the journal.
            .env("TOPGUN_JOURNAL_ENABLED", "true")
            .env("RUST_BACKTRACE", "1")
            // Quiet the server's own logs unless the operator opts in.
            .env("RUST_LOG", effective_server_log_filter())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = cmd.spawn().context("spawn topgun-server")?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("child stdout not captured"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow!("child stderr not captured"))?;

        // `PORT=` arrives on stdout; deliver it once via a oneshot.
        let (ready_tx, ready_rx) = oneshot::channel::<()>();
        let ready_tx = Arc::new(Mutex::new(Some(ready_tx)));

        spawn_line_reader(
            stdout,
            Arc::clone(&self.panic_watch),
            Arc::clone(&self.origin_capture),
            Some(ready_tx),
        );
        spawn_line_reader(
            stderr,
            Arc::clone(&self.panic_watch),
            Arc::clone(&self.origin_capture),
            None,
        );

        // Reset the intentional-kill flag for the new child generation.
        self.intentional_kill.store(false, Ordering::SeqCst);
        *self.child.lock() = Some(child);

        // Watch for an unexpected exit of THIS generation.
        self.spawn_exit_watcher();

        match tokio::time::timeout(ready_timeout, ready_rx).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_)) => bail!("server exited before signalling readiness"),
            Err(_) => bail!("server did not become ready within {ready_timeout:?}"),
        }
    }

    /// SIGKILL the current child (unclean shutdown — no graceful drain) and wait
    /// for it to reap. Marks the kill intentional so the exit watcher stays quiet.
    pub async fn kill9(&self) {
        self.intentional_kill.store(true, Ordering::SeqCst);
        // Take the child out from under the lock, then await its reap without
        // holding the (non-async) lock across the await point.
        let child = self.child.lock().take();
        if let Some(mut child) = child {
            // start_kill sends SIGKILL on Unix — the un-catchable kill -9.
            let _ = child.start_kill();
            let _ = child.wait().await;
        }
    }

    /// `kill -9` then relaunch against the same data dir, exercising one full
    /// WAL-recovery cycle.
    pub async fn restart(self: &Arc<Self>, ready_timeout: Duration) -> Result<()> {
        self.kill9().await;
        // Brief gap so the OS releases the listening socket before rebind.
        tokio::time::sleep(Duration::from_millis(250)).await;
        self.start(ready_timeout).await
    }

    /// Final teardown: SIGKILL without restart.
    ///
    /// When `TOPGUN_SOAK_GRACEFUL_SHUTDOWN=1`, send SIGTERM instead and give the
    /// child up to 90s to exit cleanly (SIGKILL fallback). Graceful teardown lets
    /// the server's `main` return normally so an end-of-run heap profiler (dhat)
    /// can flush; the default SIGKILL path is unaffected for crash-recovery runs.
    pub async fn shutdown(&self) {
        if std::env::var("TOPGUN_SOAK_GRACEFUL_SHUTDOWN").as_deref() == Ok("1") {
            self.graceful_shutdown().await;
        } else {
            self.kill9().await;
        }
    }

    /// SIGTERM the child and wait (bounded) for a clean exit, falling back to
    /// SIGKILL if it overruns. Marks the kill intentional so the exit watcher
    /// stays quiet, mirroring `kill9`.
    async fn graceful_shutdown(&self) {
        self.intentional_kill.store(true, Ordering::SeqCst);
        let pid = self.current_pid();
        let child = self.child.lock().take();
        if let Some(mut child) = child {
            if let Some(pid) = pid {
                // tokio's Child only exposes SIGKILL via start_kill; shell out to
                // deliver the catchable SIGTERM the server drains on.
                let _ = Command::new("kill")
                    .arg("-TERM")
                    .arg(pid.to_string())
                    .status()
                    .await;
            }
            if tokio::time::timeout(Duration::from_secs(90), child.wait())
                .await
                .is_err()
            {
                let _ = child.start_kill();
                let _ = child.wait().await;
            }
        }
    }

    /// Spawn a task that reaps the current child and, if it died without an
    /// intentional kill, records an unexpected-exit panic report.
    ///
    /// tokio's `Child` is not `Clone` and cannot be awaited from two places, so
    /// this polls `try_wait` periodically. All lock access is confined to the
    /// synchronous `poll_child_exit` helper so no guard is held across an await.
    fn spawn_exit_watcher(self: &Arc<Self>) {
        let this = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(200)).await;
                match this.poll_child_exit() {
                    ChildOutcome::Exited(detail) => {
                        if !this.intentional_kill.load(Ordering::SeqCst) {
                            this.panic_watch.record_unexpected_exit(&detail);
                        }
                        return;
                    }
                    ChildOutcome::Gone => return,
                    ChildOutcome::Running => {}
                }
            }
        });
    }

    /// Synchronously poll the current child's status, clearing the slot on exit.
    /// Holds the lock only for the duration of this call (no awaits inside).
    fn poll_child_exit(&self) -> ChildOutcome {
        let mut guard = self.child.lock();
        let Some(child) = guard.as_mut() else {
            return ChildOutcome::Gone;
        };
        match child.try_wait() {
            Ok(Some(status)) => {
                let detail = match (status.code(), status.signal()) {
                    (Some(c), _) => format!("exited with code {c}"),
                    (None, Some(s)) => format!("terminated by signal {s}"),
                    _ => "exited (unknown status)".to_string(),
                };
                *guard = None;
                ChildOutcome::Exited(detail)
            }
            Ok(None) => ChildOutcome::Running,
            Err(_) => ChildOutcome::Gone,
        }
    }
}

/// Result of polling the supervised child's status.
enum ChildOutcome {
    Running,
    Exited(String),
    Gone,
}

/// Spawn a task that reads `reader` line-by-line, mirroring each line into the
/// panic watch and the origin capture and (for stdout) signalling readiness on
/// the first `PORT=` line.
///
/// The origin capture is taken as a parameter rather than reached through the
/// supervisor because this reader is spawned before the child is stored, and
/// because the function is PRIVATE to this module — widening it moves no call
/// site outside the file.
fn spawn_line_reader<R>(
    reader: R,
    panic_watch: Arc<PanicWatch>,
    origin_capture: Arc<OriginCapture>,
    ready_tx: Option<Arc<Mutex<Option<oneshot::Sender<()>>>>>,
) where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let passthrough = std::env::var("SOAK_SERVER_LOG_PASSTHROUGH").is_ok();
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            panic_watch.record_line(&line);
            origin_capture.record_line(&line);
            // Opt-in diagnostic: mirror server log lines to the harness's stderr so
            // an operator can confirm server-side behavior (e.g. eviction firing)
            // from the run output. Off by default — keeps CI / 72h runs quiet.
            if passthrough {
                eprintln!("[server] {line}");
            }
            if let Some(tx) = &ready_tx {
                if line.starts_with("PORT=") {
                    if let Some(sender) = tx.lock().take() {
                        let _ = sender.send(());
                    }
                }
            }
        }
    });
}

/// Resolve the `topgun-server` binary path. Prefers the Cargo-provided
/// `CARGO_BIN_EXE_topgun-server` (set for benches), falling back to the
/// `SOAK_SERVER_BINARY` env override for ad-hoc runs against a prebuilt binary.
pub fn resolve_server_binary() -> PathBuf {
    if let Ok(p) = std::env::var("SOAK_SERVER_BINARY") {
        return PathBuf::from(p);
    }
    PathBuf::from(env!("CARGO_BIN_EXE_topgun-server"))
}

/// The log filter handed to the child as `RUST_LOG`, read from
/// `SOAK_SERVER_LOG` and defaulting to `warn`.
///
/// Single source on purpose: the harness both SETS this on the child and
/// REPORTS which filter a run used, and those two must be the same string. A
/// second environment read at the reporting site could disagree with the one
/// the child was launched with, and a run reported as armed while the child was
/// in fact quiet is exactly the false witness the origin reading must never
/// produce.
pub fn effective_server_log_filter() -> String {
    std::env::var("SOAK_SERVER_LOG").unwrap_or_else(|_| "warn".to_string())
}

/// Strip ANSI SGR escape sequences from one captured child line.
///
/// Sited HERE, in the module that owns the child's output, because the capture
/// boundary is the only place a line can be normalized exactly ONCE: the origin
/// match and the line the capture retains then see the same bytes, and no
/// downstream consumer has to re-derive whether what it holds was stripped. The
/// panic watch deliberately keeps reading the RAW line — it matches on payload
/// text, not on structured fields, and a strip there could only lose signal.
///
/// The child's log formatter emits colour UNCONDITIONALLY — it never tests
/// whether its output is a terminal — and the harness always reads that output
/// through a pipe. Colour turns `ts=1756…` into an escape-interleaved token
/// whose `key=value` split finds neither the key nor the value, and it can
/// equally interleave the target itself, so a coloured run would both fail the
/// capture's target match and read as unparsed downstream — on a run where the
/// emitter was in fact working perfectly. Normalizing HERE, at the harness
/// boundary that owns the child's output, keeps the parser formatter-agnostic
/// and `std`-only and keeps this instrument out of the server it measures.
#[must_use]
pub fn strip_ansi(line: &str) -> String {
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

// These tests are the executable half of the capture contract, and the
// integration target `tests/soak_tombstone_restart.rs` — which re-includes this
// module under the standard harness — is what actually runs them. The bench
// target compiles this module in test mode WITHOUT libtest, so the `#[test]`
// items are stripped there and their fixture helper reads as unreferenced; the
// module-scoped dead-code exemption below keeps that compile warning-free. It
// is a property of the two compile modes, not a placeholder, so it stays — the
// alternative is inlining the fixture into every test that needs it, which
// buys a tidier lint surface with two copies of one literal that can drift.
#[cfg(test)]
#[allow(dead_code)]
mod tests {
    use super::*;

    /// A line shaped like the rendered removal-site observation. The capture
    /// selects on the target alone, so the field payload is deliberately not
    /// what these tests turn on.
    fn origin_line(op_seq: usize) -> String {
        format!(
            "2026-08-31T12:00:00.000000Z  INFO {ORIGIN_TARGET}: ts=1 op_seq={op_seq} epoch=3 \
             refs_returned=0 refs_at_entry=2 bytes_returned=0 watermark=9 ceiling=9"
        )
    }

    #[test]
    fn retains_lines_carrying_the_origin_target() {
        let capture = OriginCapture::new();
        capture.record_line(&origin_line(1));

        let snap = capture.snapshot();
        assert_eq!(snap.matched, 1);
        assert_eq!(snap.dropped, 0);
        assert_eq!(snap.lines.len(), 1);
        assert!(snap.lines[0].contains(ORIGIN_TARGET));
    }

    #[test]
    fn ignores_lines_without_the_origin_target() {
        let capture = OriginCapture::new();
        capture.record_line(
            "2026-08-31T12:00:00.000000Z  WARN topgun_server::storage::wal: segment rotated",
        );
        capture.record_line("PORT=7300");

        let snap = capture.snapshot();
        assert_eq!(snap.matched, 0);
        assert_eq!(snap.dropped, 0);
        assert!(snap.lines.is_empty());
    }

    /// Overflow must be OBSERVABLE. A dropped line means the captured set is
    /// incomplete, which forces the fail-closed reading downstream; a capture
    /// that discarded quietly would let a truncated run be read as a complete
    /// one.
    #[test]
    fn overflow_beyond_capacity_counts_drops_and_is_never_silent() {
        let capture = OriginCapture::new();
        let overflow = 3_usize;
        for seq in 0..(ORIGIN_CAPTURE_CAPACITY + overflow) {
            capture.record_line(&origin_line(seq));
        }

        let snap = capture.snapshot();
        assert_eq!(snap.lines.len(), ORIGIN_CAPTURE_CAPACITY);
        assert!(
            snap.dropped > 0,
            "a capture beyond the cap must report drops, never discard silently"
        );
        assert_eq!(snap.dropped, u64::try_from(overflow).unwrap());
        assert_eq!(
            snap.matched,
            u64::try_from(ORIGIN_CAPTURE_CAPACITY + overflow).unwrap()
        );
    }

    /// The strip is the ONLY normalization left in the chain, so its edge cases
    /// have to hold on their own: a complete CSI sequence disappears, a lone
    /// escape takes no payload character with it, and an unterminated CSI has
    /// nothing after it that could be payload.
    #[test]
    fn strip_ansi_removes_escapes_without_swallowing_payload() {
        assert_eq!(strip_ansi("plain line"), "plain line");
        assert_eq!(strip_ansi("\u{1b}[32mgreen\u{1b}[0m"), "green");
        assert_eq!(strip_ansi("a\u{1b}[1;2;3mb"), "ab");
        assert_eq!(strip_ansi("a\u{1b}b"), "ab");
        assert_eq!(strip_ansi("a\u{1b}[1;2"), "a");
    }

    /// Colour is emitted unconditionally by the child, so the capture must
    /// select on the NORMALIZED line and retain the normalized form. The second
    /// fixture puts an escape INSIDE the target — the case a raw match misses
    /// outright, turning a working emitter into a silent one — and the
    /// assertion that it does not match raw is what keeps this test falsifiable
    /// against a capture that matched the raw bytes.
    #[test]
    fn matches_a_target_wrapped_in_ansi_and_retains_the_stripped_line() {
        let capture = OriginCapture::new();

        let wrapped = format!(
            "2026-08-31T12:00:00.000000Z  \u{1b}[32mINFO\u{1b}[0m \u{1b}[2m{ORIGIN_TARGET}\u{1b}[0m: \
             ts=\u{1b}[1m1\u{1b}[0m op_seq=4 epoch=3 refs_returned=0 refs_at_entry=2 \
             bytes_returned=0 watermark=9 ceiling=9"
        );
        // The target is ASCII, so splitting it at a byte index is a char
        // boundary; an escape landing there is what a span-per-segment
        // formatter produces.
        let (head, tail) = ORIGIN_TARGET.split_at(ORIGIN_TARGET.len() / 2);
        let interleaved = format!(
            "2026-08-31T12:00:00.000000Z  INFO {head}\u{1b}[2m{tail}\u{1b}[0m: ts=1 op_seq=5 \
             epoch=3 refs_returned=0 refs_at_entry=2 bytes_returned=0 watermark=9 ceiling=9"
        );
        assert!(
            !interleaved.contains(ORIGIN_TARGET),
            "the interleaved fixture must NOT match raw, or it witnesses nothing"
        );

        capture.record_line(&wrapped);
        capture.record_line(&interleaved);

        let snap = capture.snapshot();
        assert_eq!(snap.matched, 2);
        assert_eq!(snap.dropped, 0);
        assert_eq!(snap.lines.len(), 2);
        for line in &snap.lines {
            assert!(
                !line.contains('\u{1b}'),
                "the retained line must be ANSI-free: {line:?}"
            );
            assert!(line.contains(ORIGIN_TARGET));
        }
        assert!(snap.lines[0].contains("ts=1 op_seq=4"));
        assert!(snap.lines[1].contains("ts=1 op_seq=5"));
    }

    /// The capture's normalization must not reach the panic watch. The watch
    /// matches payload text rather than structured fields, and its report is
    /// meant to quote what the child actually printed, so it is fed the RAW
    /// line and quotes it byte for byte.
    #[test]
    fn panic_watch_still_sees_the_raw_line_the_capture_normalizes() {
        let watch = PanicWatch::new_standalone();
        let capture = OriginCapture::new();
        let raw = "\u{1b}[31mthread 'main' panicked at src/lib.rs:1:1\u{1b}[0m: boom";
        assert!(raw.contains('\u{1b}'), "the fixture must carry escapes");

        // The same two calls, in the same order, the line reader makes.
        watch.record_line(raw);
        capture.record_line(raw);

        assert!(watch.tripped());
        let Some(report) = watch.report() else {
            panic!("a tripped watch must carry a report")
        };
        assert!(
            report.contains(raw),
            "the panic report must quote the RAW line byte for byte: {report:?}"
        );
    }
}
