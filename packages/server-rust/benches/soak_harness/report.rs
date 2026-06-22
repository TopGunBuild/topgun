//! JSON report + live progress snapshots.
//!
//! A 72-hour run must be observable while it is in flight, so the harness
//! appends a one-line JSON snapshot to a progress file on every checkpoint
//! (tail-able by the operator) and writes a final structured report at the end
//! for CI/ops consumption.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use serde::Serialize;

/// Memory section of the final report.
#[derive(Debug, Clone, Serialize)]
pub struct MemoryReport {
    pub samples: usize,
    pub first_mb: f64,
    pub peak_mb: f64,
    pub last_mb: f64,
    pub slope_mb_per_hour: f64,
    pub passed: bool,
    pub reason: Option<String>,
}

/// Final structured outcome of a soak run.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SoakReport {
    pub mode: String,
    pub duration_secs_target: u64,
    pub duration_secs_actual: u64,
    pub churn_clients: usize,
    pub keyspace: usize,
    pub total_writes: u64,
    pub write_errors: u64,
    pub reconnects: u64,
    pub resends: u64,
    pub steady_checkpoints: u64,
    pub recovery_checkpoints: u64,
    pub crashes: u64,
    pub convergence_failures: Vec<String>,
    pub recovery_failures: Vec<String>,
    pub memory: MemoryReport,
    pub panic_report: Option<String>,
    pub passed: bool,
    pub finished_reason: String,
    pub timestamp: String,
}

/// One progress snapshot line (JSONL).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressSnapshot {
    pub timestamp: String,
    pub elapsed_secs: u64,
    pub phase: String,
    pub total_writes: u64,
    pub write_errors: u64,
    pub reconnects: u64,
    pub crashes: u64,
    pub steady_checkpoints: u64,
    pub recovery_checkpoints: u64,
    pub last_convergence_ok: bool,
    pub peak_rss_mb: f64,
    pub last_rss_mb: f64,
    pub panics_seen: bool,
}

/// Append a progress snapshot as a single JSON line. Best-effort: a write error
/// is logged but never aborts the soak.
pub fn append_progress(path: &Path, snap: &ProgressSnapshot) {
    let line = match serde_json::to_string(snap) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("progress serialize failed: {e}");
            return;
        }
    };
    match OpenOptions::new().create(true).append(true).open(path) {
        Ok(mut f) => {
            if let Err(e) = writeln!(f, "{line}") {
                eprintln!("progress write failed: {e}");
            }
        }
        Err(e) => eprintln!("progress open failed for {}: {e}", path.display()),
    }
}

/// Write the final report as pretty JSON.
pub fn write_report(path: &Path, report: &SoakReport) {
    match std::fs::File::create(path) {
        Ok(f) => {
            if let Err(e) = serde_json::to_writer_pretty(f, report) {
                eprintln!("report write failed: {e}");
            }
        }
        Err(e) => eprintln!("report create failed for {}: {e}", path.display()),
    }
}

/// `YYYY-MM-DDTHH:MM:SSZ` from `SystemTime`, no external date crate.
#[allow(clippy::cast_possible_truncation)]
pub fn utc_timestamp_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let hh = (secs % 86400) / 3600;
    let mm = (secs % 3600) / 60;
    let ss = secs % 60;
    let mut days = secs / 86400;
    let mut year: u64 = 1970;
    loop {
        let diy = if is_leap(year) { 366 } else { 365 };
        if days < diy {
            break;
        }
        days -= diy;
        year += 1;
    }
    let leap = is_leap(year);
    let md: [u64; 12] = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 1;
    for &m in &md {
        if days < m {
            break;
        }
        days -= m;
        month += 1;
    }
    let day = days + 1;
    format!("{year:04}-{month:02}-{day:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

fn is_leap(y: u64) -> bool {
    (y.is_multiple_of(4) && !y.is_multiple_of(100)) || y.is_multiple_of(400)
}
