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
}

impl ServerSupervisor {
    /// Build a supervisor; does not start the child yet.
    pub fn new(config: ServerConfig) -> Arc<Self> {
        Arc::new(Self {
            config,
            child: Mutex::new(None),
            intentional_kill: Arc::new(AtomicBool::new(false)),
            panic_watch: PanicWatch::new(),
        })
    }

    /// Address clients connect to (stable across restarts).
    pub fn addr(&self) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], self.config.port))
    }

    pub fn panic_watch(&self) -> Arc<PanicWatch> {
        Arc::clone(&self.panic_watch)
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
            .env(
                "RUST_LOG",
                std::env::var("SOAK_SERVER_LOG").unwrap_or_else(|_| "warn".to_string()),
            )
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

        spawn_line_reader(stdout, Arc::clone(&self.panic_watch), Some(ready_tx));
        spawn_line_reader(stderr, Arc::clone(&self.panic_watch), None);

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
    pub async fn shutdown(&self) {
        self.kill9().await;
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
/// panic watch and (for stdout) signalling readiness on the first `PORT=` line.
fn spawn_line_reader<R>(
    reader: R,
    panic_watch: Arc<PanicWatch>,
    ready_tx: Option<Arc<Mutex<Option<oneshot::Sender<()>>>>>,
) where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let passthrough = std::env::var("SOAK_SERVER_LOG_PASSTHROUGH").is_ok();
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            panic_watch.record_line(&line);
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
