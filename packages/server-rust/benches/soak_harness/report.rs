//! JSON report + live progress snapshots, plus the post-run WAL frame-kind
//! census the reports are built from.
//!
//! A 72-hour run must be observable while it is in flight, so the harness
//! appends a one-line JSON snapshot to a progress file on every checkpoint
//! (tail-able by the operator) and writes a final structured report at the end
//! for CI/ops consumption.
//!
//! The WAL census lives here rather than in the bench crate root because the
//! bench is declared `harness = false`: tests written beside the crate root
//! compile but never run, so the census is sited in a module an integration
//! target can `#[path]`-include and drive under the standard libtest harness.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use serde::Serialize;
use topgun_server::storage::record::RecordValue;
use topgun_server::storage::wal::format::{self, FrameDecodeResult};
use topgun_server::storage::wal::{WalOp, WalStorePayload};

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
    /// The WAL fsync policy the child server was launched with, verbatim.
    ///
    /// Recorded because the policy is a ~40x write-path difference: without it
    /// in the artifact, a run that meant to execute at `batched` but forgot the
    /// flag executed at `per_op` and no consumer of this report could tell.
    /// A `String` and not an enum on purpose — it is the exact byte sequence
    /// forwarded to the child, not this harness's interpretation of it.
    pub wal_fsync: String,
    /// The tombstone epoch width the server applies, derived by
    /// [`effective_epoch_width`].
    ///
    /// Derived in THIS process from THIS process's environment, not read back
    /// from the child. That is a valid proxy only because the harness never
    /// sets `TOPGUN_EPOCH_WIDTH` on the child and never clears its environment,
    /// so the child inherits this value verbatim.
    pub epoch_width: u64,
    pub total_writes: u64,
    pub write_errors: u64,
    pub reconnects: u64,
    pub resends: u64,
    pub steady_checkpoints: u64,
    pub recovery_checkpoints: u64,
    pub crashes: u64,
    pub convergence_failures: Vec<String>,
    pub recovery_failures: Vec<String>,
    /// Expected-fail gates that did NOT fail the run (currently the SPEC-322b
    /// post-restart QUERY-path read-back). Tracked so the gap stays visible and
    /// can be promoted to a required gate when its dependency lands.
    pub pending_gates: Vec<String>,
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

/// The tombstone epoch width the server binary will apply, derived by the same
/// rule the binary itself applies: `TOPGUN_EPOCH_WIDTH` parsed as a positive
/// integer, otherwise the shipped default.
///
/// The default is NAMED rather than written as a literal so the harness's answer
/// and the server's answer cannot drift apart at a future default change.
///
/// This is a named function rather than an inline expression at the report's
/// construction site so it can be driven directly by a test — a derivation only
/// reachable through the bench crate root is a derivation nothing can check.
#[must_use]
pub fn effective_epoch_width() -> u64 {
    match std::env::var("TOPGUN_EPOCH_WIDTH") {
        Ok(raw) => match raw.parse::<u64>() {
            Ok(w) if w > 0 => w,
            _ => topgun_server::tombstone_frontier_impl::DEFAULT_EPOCH_WIDTH,
        },
        Err(_) => topgun_server::tombstone_frontier_impl::DEFAULT_EPOCH_WIDTH,
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

// ---------------------------------------------------------------------------
// WAL frame-kind census
// ---------------------------------------------------------------------------

/// Per-op-kind WAL frame-size aggregates from a post-run scan of the retained
/// segment files. The retained WAL is the acked-beyond-applied tail (GC has
/// reclaimed everything at/below the applied watermark), so its highest
/// sequences are the most-recently-appended frames — which lets a single
/// end-of-run scan answer Q1 (does an OR op's frame size RISE over the run?) by
/// bucketing OR frames by sequence into an early and a late half.
///
/// The OR side is tallied THREE ways, and the redundancy is deliberate: the
/// `or_*` fields cover both OR frame kinds together (a mixture), while the
/// `or_delta_*` and `or_snapshot_*` fields keep each kind's counts AND bytes
/// separate. A claim about delta framing read off the mixture is refuted by a
/// working emitter as soon as a residual share of snapshot frames survives, so
/// the per-kind buckets are what make the claim checkable at all.
pub struct WalFrameStats {
    pub segment_files: usize,
    /// Both OR frame kinds together — the mixture.
    pub or_frames: usize,
    pub or_bytes_total: u64,
    pub or_bytes_max: u64,
    /// `WalOp::OrDelta` frames only.
    pub or_delta_frames: usize,
    pub or_delta_bytes_total: u64,
    pub or_delta_bytes_max: u64,
    pub or_delta_bytes_mean: f64,
    /// Full OR-side snapshot frames only (`OrMap` / legacy `OrTombstones`).
    pub or_snapshot_frames: usize,
    pub or_snapshot_bytes_total: u64,
    pub or_snapshot_bytes_max: u64,
    pub or_snapshot_bytes_mean: f64,
    pub lww_frames: usize,
    pub lww_bytes_total: u64,
    pub lww_bytes_max: u64,
    pub remove_frames: usize,
    /// Mean OR frame bytes in the earliest / latest sequence half of the retained
    /// corpus. `late > early` is the O(N)-per-op growth signature.
    pub or_bytes_mean_early: f64,
    pub or_bytes_mean_late: f64,
    pub or_bytes_max_early: u64,
    pub or_bytes_max_late: u64,
    /// Frames whose sequence is in the top decile of the retained corpus (the
    /// drain-window proxy) plus the largest OR frame among them: many small
    /// frames points at a rotation/flush storm, a few large OR frames at a final
    /// full-slot re-write.
    pub top_decile_frames: usize,
    pub top_decile_or_bytes_max: u64,
}

/// Which accounting bucket a WAL frame belongs to.
///
/// `OrDelta` and `OrSnapshot` are distinct variants rather than one "OR" answer
/// because telling a per-op mutation frame from a full-slot snapshot frame is
/// the entire point of the census: folded together, an armed emitter and a
/// disarmed one produce the same number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameKind {
    /// A per-op OR mutation frame.
    OrDelta,
    /// A full OR-side slot snapshot (`OrMap`, or the legacy `OrTombstones` blob).
    OrSnapshot,
    /// An LWW frame, including legacy bare-`Value` frames, which replay as LWW.
    Lww,
    /// A whole-key tombstone frame.
    Remove,
}

#[must_use]
pub fn classify_frame(op: &WalOp) -> FrameKind {
    match op {
        WalOp::Remove => FrameKind::Remove,
        WalOp::OrDelta { .. } => FrameKind::OrDelta,
        WalOp::Store { value, .. } => match value {
            WalStorePayload::Record(
                RecordValue::OrMap { .. } | RecordValue::OrTombstones { .. },
            ) => FrameKind::OrSnapshot,
            WalStorePayload::Record(RecordValue::Lww { .. }) | WalStorePayload::Legacy(_) => {
                FrameKind::Lww
            }
        },
    }
}

/// Scan every `*.log` WAL segment under `wal_dir`, decode intact frames, and
/// aggregate per-op-kind frame sizes. Read-only and best-effort: a torn active
/// tail decodes as its intact prefix (`TruncatedTail`), a corrupt/foreign file
/// is skipped, and an exact on-disk frame size is recovered by re-encoding each
/// decoded entry through the deterministic codec (the same trick WAL recovery
/// uses to measure a segment's intact-prefix length). Returns `None` only if the
/// directory cannot be read at all.
#[must_use]
pub fn scan_wal_frame_sizes(wal_dir: &Path) -> Option<WalFrameStats> {
    let read_dir = std::fs::read_dir(wal_dir).ok()?;
    // (sequence, frame_bytes) for every OR frame of EITHER kind, plus the same
    // pairs split per kind, and running LWW/Remove tallies.
    let mut or_frames: Vec<(u64, u64)> = Vec::new();
    let mut or_delta_bytes: Vec<u64> = Vec::new();
    let mut or_snapshot_bytes: Vec<u64> = Vec::new();
    let mut lww_bytes_total: u64 = 0;
    let mut lww_bytes_max: u64 = 0;
    let mut lww_frames: usize = 0;
    let mut remove_frames: usize = 0;
    let mut segment_files: usize = 0;
    let mut max_seq: u64 = 0;

    for entry in read_dir.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("log") {
            continue;
        }
        let Ok(data) = std::fs::read(&path) else {
            continue;
        };
        segment_files += 1;
        // A corrupt/foreign/empty file contributes no frames — skip it rather than
        // abort the whole scan (this is a report-only instrument).
        let (FrameDecodeResult::Complete(entries)
        | FrameDecodeResult::TruncatedTail { complete: entries }) = format::decode_all(&data)
        else {
            continue;
        };
        for wal_entry in &entries {
            let Ok(frame) = format::encode(wal_entry) else {
                continue;
            };
            let bytes = frame.len() as u64;
            max_seq = max_seq.max(wal_entry.sequence);
            match classify_frame(&wal_entry.op) {
                FrameKind::OrDelta => {
                    or_frames.push((wal_entry.sequence, bytes));
                    or_delta_bytes.push(bytes);
                }
                FrameKind::OrSnapshot => {
                    or_frames.push((wal_entry.sequence, bytes));
                    or_snapshot_bytes.push(bytes);
                }
                FrameKind::Lww => {
                    lww_frames += 1;
                    lww_bytes_total += bytes;
                    lww_bytes_max = lww_bytes_max.max(bytes);
                }
                FrameKind::Remove => remove_frames += 1,
            }
        }
    }

    or_frames.sort_by_key(|(seq, _)| *seq);
    let or_count = or_frames.len();
    let or_bytes_total: u64 = or_frames.iter().map(|(_, b)| *b).sum();
    let or_bytes_max = or_frames.iter().map(|(_, b)| *b).max().unwrap_or(0);

    let half = or_count / 2;
    let (early, late) = or_frames.split_at(half);
    let mean = |s: &[(u64, u64)]| -> f64 {
        if s.is_empty() {
            0.0
        } else {
            s.iter().map(|(_, b)| *b as f64).sum::<f64>() / s.len() as f64
        }
    };
    let max_of = |s: &[(u64, u64)]| -> u64 { s.iter().map(|(_, b)| *b).max().unwrap_or(0) };
    let kind_mean = |s: &[u64]| -> f64 {
        if s.is_empty() {
            0.0
        } else {
            s.iter().map(|b| *b as f64).sum::<f64>() / s.len() as f64
        }
    };

    // Top-decile-by-sequence frames across BOTH kinds: the drain-window proxy.
    let decile_floor = max_seq.saturating_sub(max_seq / 10);
    let top_decile_frames = or_frames.iter().filter(|(s, _)| *s >= decile_floor).count();
    let top_decile_or_bytes_max = or_frames
        .iter()
        .filter(|(s, _)| *s >= decile_floor)
        .map(|(_, b)| *b)
        .max()
        .unwrap_or(0);

    Some(WalFrameStats {
        segment_files,
        or_frames: or_count,
        or_bytes_total,
        or_bytes_max,
        or_delta_frames: or_delta_bytes.len(),
        or_delta_bytes_total: or_delta_bytes.iter().sum(),
        or_delta_bytes_max: or_delta_bytes.iter().copied().max().unwrap_or(0),
        or_delta_bytes_mean: kind_mean(&or_delta_bytes),
        or_snapshot_frames: or_snapshot_bytes.len(),
        or_snapshot_bytes_total: or_snapshot_bytes.iter().sum(),
        or_snapshot_bytes_max: or_snapshot_bytes.iter().copied().max().unwrap_or(0),
        or_snapshot_bytes_mean: kind_mean(&or_snapshot_bytes),
        lww_frames,
        lww_bytes_total,
        lww_bytes_max,
        remove_frames,
        or_bytes_mean_early: mean(early),
        or_bytes_mean_late: mean(late),
        or_bytes_max_early: max_of(early),
        or_bytes_max_late: max_of(late),
        top_decile_frames,
        top_decile_or_bytes_max,
    })
}
