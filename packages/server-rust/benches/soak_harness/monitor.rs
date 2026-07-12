//! Server memory monitoring for leak / unbounded-growth detection.
//!
//! The soak runs a fixed keyspace overwritten in place, so the server's
//! resident set should plateau. A sustained upward trend implicates a leak —
//! most plausibly unbounded OR-Map tombstone accumulation, which the soak
//! deliberately drives via add/remove churn on an OR-Map with a distinct tag
//! per iteration.
//!
//! RSS is sampled by shelling out to `ps -o rss= -p <pid>` (KiB on both macOS
//! and Linux), avoiding a platform-specific dependency. The assessment fits a
//! least-squares line to `(elapsed_hours, rss_mb)` and fails if either the
//! slope exceeds a per-hour threshold (with a minimum absolute growth guard to
//! ignore short-run noise) or the peak exceeds a hard ceiling.
//!
//! ## Slope threshold is calibrated to catch the tombstone leak, not mask it
//!
//! The OR-Map tombstone set on the server write path is intentionally unbounded
//! (there is no causal-stability tracking that would let the server prune a
//! tombstone without risking resurrection on a lagging client — pruning is a
//! correctness hazard, not a memory optimization). The soak's OR churn stream
//! therefore grows tombstones linearly, estimated at ~3-5 MB/h RSS over a 72h
//! run. The earlier default slope threshold (50 MB/h in-process, 25 MB/h on the
//! Hetzner 72h runner) sat 5-10x ABOVE that rate, so a real linear leak fitted a
//! ~3-5 MB/h line that passed both clauses — a false GREEN masking an eventual
//! out-of-memory.
//!
//! [`DEFAULT_MEM_THRESHOLD_MB_PER_HOUR`] and [`DEFAULT_MEM_MIN_GROWTH_MB`] are
//! set so a sustained ~3-5 MB/h linear series FAILS while a genuine in-place
//! plateau (slope near zero, total growth below the min-growth guard) PASSES.
//! The min-growth guard is what keeps short bounded soaks (10-60 min) green: a
//! plateau never accumulates enough absolute growth to trip the slope clause, so
//! only a run that grows past [`DEFAULT_MEM_MIN_GROWTH_MB`] at more than
//! [`DEFAULT_MEM_THRESHOLD_MB_PER_HOUR`] — i.e. the 72h tombstone leak — fails.
//! See `tests::calibration_*` for the executable proof of both directions.
//!
//! ## Detection floor — a short green soak is not a bounded-memory guarantee
//!
//! This is a *sustained-leak* detector, not an instantaneous one. Because the
//! min-growth guard suppresses the slope clause until total growth clears
//! [`DEFAULT_MEM_MIN_GROWTH_MB`], a leak is only caught once the run is long
//! enough to accumulate that much: ~16h at 5 MB/h, ~27h at 3 MB/h. That is
//! deliberate, not a gap — over a few hours a 3-5 MB/h leak's absolute growth is
//! within RSS noise (allocator retention, cache warmup, GC), so failing on slope
//! alone would false-FAIL healthy short soaks. The 72h run is therefore the gate
//! that actually asserts bounded tombstone memory; the minutes-scale smoke and
//! bounded soaks exercise crash/convergence, and their green memory verdict means
//! "no leak large enough to clear the noise floor in this window", NOT "bounded
//! memory proven". Do not read a short green soak as the latter.
//!
//! ## Tombstone-byte gate: same slope idea, no detection floor
//!
//! `topgun_ormap_tombstone_bytes_total` (sampled over HTTP from `GET /metrics`)
//! is a direct, residency-independent count of tombstone bytes on the write
//! path — unlike RSS it carries no allocator retention, no read/GC jitter, and
//! no cache-warmup wobble. Every unit of observed growth is either a newly
//! inserted tombstone or nothing; there is no noise floor to wait out. That is
//! why [`assess_tombstone_bytes`] uses a much tighter per-hour threshold than
//! the RSS gate AND does not replicate RSS's large min-growth guard (see
//! "Detection floor" above) — the RSS guard exists solely to suppress noise
//! until a leak's absolute growth clears it, and this gauge has no analogous
//! noise to suppress. That is precisely what lets *short* soak runs gain real
//! leak signal from the byte gate long before the RSS gate's multi-hour
//! detection floor would let it see anything.
//!
//! Note on gating responsibility: [`assess_tombstone_bytes`] *computes* the byte
//! verdict (including its `passed` flag), but whether that verdict gates the run
//! is decided in `main.rs`, not here. The byte **slope** is surfaced
//! REPORT-ONLY, NOT hard-gated. Although the gauge is restart-survivable
//! (`reconcile_tombstone_bytes` in `bin/topgun_server.rs` re-seeds it via
//! `set_tombstone_bytes` at boot), it is ADDITIVE-ONLY within a process life:
//! every tombstone-add increments it and no prune-side decrement is wired yet
//! (`sub_tombstone_bytes` has no live call site), and the OR-Map epoch prune is
//! dark until a follow-up supplies the frontier's durable-epoch watermark. So
//! under sustained OR churn the gauge climbs at the tombstone-*creation* rate
//! and cannot plateau — hard-gating the slope would false-RED every healthy
//! long run. `main.rs` therefore routes a slope breach to its `pending_gates`
//! (honest, non-failing) channel, exactly like the disk gate; only the
//! blind-monitor zero-sample case hard-gates. The RSS gate above remains a
//! coarse, non-tombstone backstop. Re-promote the slope to a hard gate once the
//! prune-activation follow-up makes the gauge track residency and a live
//! bounded soak is observed to plateau. (This module's `passed: bool` on
//! [`TombstoneAssessment`] still drives the report-only signal and the
//! calibration tests below.)
//!
//! ## Boot-recompute-gap exclusion
//!
//! Between a process start and `reconcile_tombstone_bytes` completion there is
//! a transient window during which the gauge is not yet trustworthy for the
//! CURRENT life. [`exclude_boot_gap_samples`] is a pure, `retain`-style helper
//! that drops every sample falling inside a recorded [`BootGap`] before the
//! series reaches [`assess_tombstone_bytes`], so a spurious pre-reconcile read
//! (or the very act of a `kill -9` and restart) can never manufacture a false
//! leak/plateau signal. `main.rs` calls it once per scraped tombstone sample
//! from the sampling loop; being pure and series-based, it is also driven
//! directly by `tests::calibration_boot_gap_exclusion_does_not_trip_gate` with
//! a fully synthetic sequence — no real process required.

use std::path::Path;
use std::process::Command;

/// Calibrated slope ceiling (MB/hour) for the 72h soak's memory assertion.
///
/// Below the ~3-5 MB/h the OR tombstone leak produces, above the ~0 MB/h a real
/// in-place-overwrite plateau produces — so it distinguishes leak from plateau
/// at the rate the soak actually drives. Callers (soak `main.rs` defaults, the
/// Hetzner runner env default) should feed this value so the 72h run cannot
/// false-GREEN the accepted-but-real tombstone growth.
pub const DEFAULT_MEM_THRESHOLD_MB_PER_HOUR: f64 = 2.0;

/// Calibrated absolute-growth guard (MB): below this total growth the slope
/// clause is ignored as short-run noise. Sized so a genuine plateau (and any
/// bounded short soak) stays green, while the 72h tombstone leak — which
/// accumulates hundreds of MB — clears the guard and is judged on slope.
pub const DEFAULT_MEM_MIN_GROWTH_MB: f64 = 80.0;

/// One resident-set sample.
#[derive(Debug, Clone, Copy)]
pub struct MemSample {
    pub elapsed_secs: f64,
    pub rss_mb: f64,
}

/// Verdict of a memory-growth assessment.
#[derive(Debug, Clone)]
pub struct MemoryAssessment {
    pub samples: usize,
    pub first_mb: f64,
    pub peak_mb: f64,
    pub last_mb: f64,
    pub slope_mb_per_hour: f64,
    pub passed: bool,
    pub reason: Option<String>,
}

/// Sample the resident set of `pid` in megabytes via `ps`. Returns `None` if
/// the process is gone or `ps` output cannot be parsed (e.g. mid-restart).
pub fn sample_rss_mb(pid: u32) -> Option<f64> {
    let out = Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let kib: f64 = text.trim().parse().ok()?;
    Some(kib / 1024.0)
}

/// Assess a series of samples for bounded memory.
///
/// * `threshold_mb_per_hour` — maximum tolerated growth slope.
/// * `min_growth_mb` — absolute growth (peak − first) below which slope is
///   treated as noise regardless of fit (guards tiny/short runs).
/// * `ceiling_mb` — hard cap on peak RSS.
#[allow(clippy::cast_precision_loss)]
pub fn assess(
    samples: &[MemSample],
    threshold_mb_per_hour: f64,
    min_growth_mb: f64,
    ceiling_mb: f64,
) -> MemoryAssessment {
    if samples.is_empty() {
        // Zero samples over a real run means the monitor was BLIND (ps failing or
        // the server pid never resolved) — a leak would be invisible. Fail rather
        // than silently pass: an assertion that cannot observe must not report ok.
        return MemoryAssessment {
            samples: 0,
            first_mb: 0.0,
            peak_mb: 0.0,
            last_mb: 0.0,
            slope_mb_per_hour: 0.0,
            passed: false,
            reason: Some(
                "no RSS samples collected — memory monitoring was blind (ps failed or server \
                 pid never available); cannot assert bounded memory"
                    .to_string(),
            ),
        };
    }

    let first_mb = samples[0].rss_mb;
    let last_mb = samples[samples.len() - 1].rss_mb;
    let peak_mb = samples.iter().fold(0.0_f64, |m, s| m.max(s.rss_mb));

    let points: Vec<(f64, f64)> = samples.iter().map(|s| (s.elapsed_secs, s.rss_mb)).collect();
    let slope_mb_per_hour = least_squares_slope_per_hour(&points);

    let growth = peak_mb - first_mb;
    let mut reasons = Vec::new();

    if peak_mb > ceiling_mb {
        reasons.push(format!(
            "peak RSS {peak_mb:.1}MB exceeds ceiling {ceiling_mb:.1}MB"
        ));
    }
    if growth >= min_growth_mb && slope_mb_per_hour > threshold_mb_per_hour {
        reasons.push(format!(
            "growth slope {slope_mb_per_hour:.1}MB/h exceeds {threshold_mb_per_hour:.1}MB/h \
             (total growth {growth:.1}MB over {} samples)",
            samples.len()
        ));
    }

    let passed = reasons.is_empty();
    MemoryAssessment {
        samples: samples.len(),
        first_mb,
        peak_mb,
        last_mb,
        slope_mb_per_hour,
        passed,
        reason: if passed {
            None
        } else {
            Some(reasons.join("; "))
        },
    }
}

/// Least-squares slope of `value` vs. `elapsed_secs`, expressed as a per-hour
/// rate. Returns 0 when there is insufficient variance in the time axis (e.g.
/// one sample).
///
/// Shared by all three samplers (RSS MB, tombstone-bytes, disk MB) — the fit
/// math is identical regardless of which quantity is being tracked, so it
/// exists exactly once here rather than as a per-sampler-type copy.
#[allow(clippy::cast_precision_loss)]
fn least_squares_slope_per_hour(points: &[(f64, f64)]) -> f64 {
    let n = points.len() as f64;
    if n < 2.0 {
        return 0.0;
    }
    // x in hours so the slope is directly <unit>/hour.
    let xs: Vec<f64> = points.iter().map(|(secs, _)| secs / 3600.0).collect();
    let ys: Vec<f64> = points.iter().map(|(_, value)| *value).collect();
    let mean_x = xs.iter().sum::<f64>() / n;
    let mean_y = ys.iter().sum::<f64>() / n;
    let mut num = 0.0;
    let mut den = 0.0;
    for (x, y) in xs.iter().zip(ys.iter()) {
        num += (x - mean_x) * (y - mean_y);
        den += (x - mean_x) * (x - mean_x);
    }
    if den.abs() < f64::EPSILON {
        0.0
    } else {
        num / den
    }
}

/// Least-squares slope over just the LAST HALF of `points` (by time order),
/// expressed as a per-hour rate.
///
/// This is the plateau/leak statistic for the tombstone-byte gate
/// ([`assess_tombstone_bytes`]). A first-half/second-half growth RATIO is
/// unstable as the denominator (first-half growth) approaches zero — exactly
/// the shape a genuinely-bounded run produces once the M4 tombstone bound
/// engages. A last-half-window OLS slope has no such singularity: it stays
/// well-defined and near zero whether the window is perfectly flat or has
/// tiny jitter, and it correctly reports near-zero on a "grow, then flatten"
/// series even though a full-window fit would still be dragged upward by the
/// earlier growth. `points.len() / 2` (floor) biases the split toward
/// INCLUDING more of the recent half on an odd count.
///
/// The slope statistic and the minimum-window-span guard in
/// [`assess_tombstone_bytes`] MUST agree on which samples make up the "recent
/// half" — otherwise the guard could clear a window the slope was actually fit
/// over (or vice versa) — so both derive it from [`last_half_window`].
fn last_half_window(points: &[(f64, f64)]) -> &[(f64, f64)] {
    let half = points.len() / 2;
    &points[half..]
}

fn last_half_window_slope_per_hour(points: &[(f64, f64)]) -> f64 {
    least_squares_slope_per_hour(last_half_window(points))
}

/// Wall-clock span (seconds) covered by the last-half window — `0.0` when that
/// window has fewer than two points (no meaningful span to extrapolate over).
///
/// Gates the per-hour slope clause in [`assess_tombstone_bytes`]: the per-hour
/// rate is an extrapolation (bytes/sec × 3600), so over a sub-minute window the
/// `3600 / span` amplification turns a few KB of ordinary ramp-up into a
/// six-figure B/h "leak". This span lets the gate suppress that clause until the
/// fitted window covers enough real time for the per-hour number to mean
/// anything.
///
/// A degenerate last-half window of ≤2 points therefore yields `0.0` and is
/// intentionally suppressed: a run that produced only one or two samples (a very
/// short soak) cannot trip even the report-only slope clause regardless of how
/// linear its growth looks — there is no window to extrapolate over, and the
/// 120s [`DEFAULT_TOMBSTONE_BYTES_MIN_WINDOW_SECS`] floor would suppress it in any
/// case. Real bounded/72h soaks accumulate hundreds of samples, so this bounds
/// nothing they rely on.
fn last_half_window_span_secs(points: &[(f64, f64)]) -> f64 {
    let window = last_half_window(points);
    match (window.first(), window.last()) {
        (Some(first), Some(last)) if window.len() >= 2 => last.0 - first.0,
        _ => 0.0,
    }
}

/// Calibrated slope ceiling (bytes/hour) for the tombstone-byte gate.
///
/// ~0.5 KB/h. Tight by design — see the module-level "no detection floor" doc:
/// with no RSS-style noise to absorb, any sustained per-hour growth this small
/// is already real signal, not measurement wobble.
///
/// Consumed by `main.rs` (the soak loop scrapes `GET /metrics` and calls
/// [`assess_tombstone_bytes`] with this threshold alongside the RSS `assess`) and
/// by the `soak_monitor_calibration` integration target's tests — the harness
/// wiring has landed, so no `allow(dead_code)` is needed here. The slope this
/// threshold measures is surfaced REPORT-ONLY by `main.rs` (the additive-only
/// gauge cannot plateau under sustained churn until an OR-Map prune-activation
/// follow-up wires residency tracking); the blind-monitor zero-sample clause is
/// the only tombstone hard gate today. RSS above is the coarse backstop.
pub const DEFAULT_TOMBSTONE_BYTES_THRESHOLD_PER_HOUR: f64 = 512.0;

/// Absolute-growth guard (bytes) for the tombstone-byte slope clause.
///
/// Deliberately minimal — NOT the RSS gate's large [`DEFAULT_MEM_MIN_GROWTH_MB`]
/// analogue. The RSS guard exists to suppress a real leak's *slope* signal
/// until enough hours have passed for its absolute growth to clear RSS noise;
/// this gauge has no such noise, so a large guard would only reintroduce the
/// RSS gate's multi-hour detection floor for no benefit. Kept just above zero
/// so a one/two-sample run (degenerate least-squares fit) cannot trip the
/// clause on rounding.
pub const DEFAULT_TOMBSTONE_BYTES_MIN_GROWTH: f64 = 1.0;

/// Minimum wall-clock span (seconds) of the last-half fit window before the
/// per-hour slope clause is surfaced as a (report-only) breach.
///
/// The slope is a per-hour EXTRAPOLATION (bytes/sec × 3600). Over a sub-minute
/// window the `3600 / span_secs` amplification is enormous: a healthy short run
/// that has simply not yet had time to plateau (e.g. the 25s blocking CI "Short
/// no-crash soak", ~6 samples over ~25s with a few KB of ordinary ramp-up)
/// extrapolates to a six-figure B/h rate and would surface a spurious breach.
/// Below this floor the slope carries no plateau signal — a leak and a
/// not-yet-plateaued healthy run are indistinguishable — so the clause is
/// suppressed (no breach is emitted for it) and the assessment passes on the
/// blind-monitor + absolute-growth clauses alone. (The breach is report-only
/// today regardless — see [`DEFAULT_TOMBSTONE_BYTES_THRESHOLD_PER_HOUR`]; this
/// floor keeps even the report-only signal from crying wolf on a short run.)
///
/// 120s sits well above the smoke run's ~10-15s last-half span (suppressed) and
/// well below a real bounded soak's window (a 10-60 min live run's last-half
/// span is minutes, so a genuine leak still trips the clause; the 72h soak's
/// span is orders of magnitude above it). The gauge is restart-survivable, so a
/// crash-enabled long run keeps a continuous series whose window clears the
/// floor.
pub const DEFAULT_TOMBSTONE_BYTES_MIN_WINDOW_SECS: f64 = 120.0;

/// One tombstone-byte-gauge sample (`topgun_ormap_tombstone_bytes_total`,
/// scraped over HTTP). `bytes` is `u64` — a counted byte total is a
/// non-negative integer, never a float.
#[derive(Debug, Clone, Copy)]
pub struct TombstoneSample {
    pub elapsed_secs: f64,
    pub bytes: u64,
}

/// Verdict of a tombstone-byte-growth assessment. Mirrors [`MemoryAssessment`]'s
/// shape so callers (soak `main.rs`) can report both gates uniformly.
#[derive(Debug, Clone)]
pub struct TombstoneAssessment {
    pub samples: usize,
    pub first_bytes: u64,
    pub peak_bytes: u64,
    pub last_bytes: u64,
    pub slope_bytes_per_hour: f64,
    pub passed: bool,
    pub reason: Option<String>,
}

/// Assess a series of tombstone-byte samples for bounded growth.
///
/// * `threshold_bytes_per_hour` — maximum tolerated growth slope.
/// * `min_growth_bytes` — absolute growth (peak − first) below which slope is
///   treated as noise. Kept minimal (see [`DEFAULT_TOMBSTONE_BYTES_MIN_GROWTH`]
///   doc) rather than mirroring the RSS gate's large guard.
/// * `min_window_secs` — minimum wall-clock span of the last-half fit window
///   before the per-hour slope clause is surfaced as a (report-only) breach
///   (see [`DEFAULT_TOMBSTONE_BYTES_MIN_WINDOW_SECS`]). Guards against a
///   too-short-to-plateau run false-REDing on the per-hour extrapolation of a
///   sub-minute window. Pass `0.0` to disable the guard.
#[allow(clippy::cast_precision_loss)]
pub fn assess_tombstone_bytes(
    samples: &[TombstoneSample],
    threshold_bytes_per_hour: f64,
    min_growth_bytes: f64,
    min_window_secs: f64,
) -> TombstoneAssessment {
    if samples.is_empty() {
        // Same rationale as the RSS gate's empty-samples branch: zero samples
        // means the monitor was BLIND (metrics scrape failed or the server was
        // never reachable) — a leak would be invisible. Fail rather than
        // silently pass.
        return TombstoneAssessment {
            samples: 0,
            first_bytes: 0,
            peak_bytes: 0,
            last_bytes: 0,
            slope_bytes_per_hour: 0.0,
            passed: false,
            reason: Some(
                "no tombstone-byte samples collected — monitoring was blind (metrics scrape \
                 failed or server never reachable); cannot assert bounded tombstone growth"
                    .to_string(),
            ),
        };
    }

    let first_bytes = samples[0].bytes;
    let last_bytes = samples[samples.len() - 1].bytes;
    let peak_bytes = samples.iter().map(|s| s.bytes).max().unwrap_or(first_bytes);

    #[allow(clippy::cast_precision_loss)]
    let points: Vec<(f64, f64)> = samples
        .iter()
        .map(|s| (s.elapsed_secs, s.bytes as f64))
        .collect();
    // Last-half-window OLS, not the full-window fit: robust near zero and
    // correctly reports a genuine plateau even after an earlier ramp (see
    // `last_half_window_slope_per_hour` doc).
    let slope_bytes_per_hour = last_half_window_slope_per_hour(&points);
    // Span of the window the slope was actually fit over — the per-hour rate is
    // an extrapolation over exactly this span, so it is what the min-window
    // guard must clear.
    let last_half_span_secs = last_half_window_span_secs(&points);

    #[allow(clippy::cast_precision_loss)]
    let growth = peak_bytes.saturating_sub(first_bytes) as f64;
    let mut reasons = Vec::new();

    // The per-hour slope clause only carries a plateau/leak signal once the fit
    // window spans enough wall-clock time (min_window_secs). Below that, the
    // per-hour extrapolation of a sub-minute window is dominated by the
    // `3600 / span` amplification — a healthy run that simply has not plateaued
    // yet is indistinguishable from a leak — so the clause is suppressed and the
    // run passes on the blind-monitor + absolute-growth clauses alone. A real
    // unbounded leak still trips it once the run is long enough (the 72h soak's
    // window is orders of magnitude above the floor).
    if last_half_span_secs >= min_window_secs
        && growth >= min_growth_bytes
        && slope_bytes_per_hour > threshold_bytes_per_hour
    {
        reasons.push(format!(
            "tombstone-byte growth slope {slope_bytes_per_hour:.1} bytes/h exceeds \
             {threshold_bytes_per_hour:.1} bytes/h (total growth {growth:.0} bytes over {} \
             samples, last-half window {last_half_span_secs:.0}s)",
            samples.len()
        ));
    }

    let passed = reasons.is_empty();
    TombstoneAssessment {
        samples: samples.len(),
        first_bytes,
        peak_bytes,
        last_bytes,
        slope_bytes_per_hour,
        passed,
        reason: if passed {
            None
        } else {
            Some(reasons.join("; "))
        },
    }
}

/// One boot-recompute-gap window: the span (on the `elapsed_secs` clock shared
/// with [`TombstoneSample`]) between a `kill -9` and the restarted process's
/// health-ready signal, during which the tombstone-bytes gauge for the new
/// process life has not yet been re-seeded by `reconcile_tombstone_bytes` and
/// a scrape could observe a spurious low/zero total.
///
/// Constructed by the soak orchestrator (`main.rs`) around each
/// `ServerSupervisor::restart` call: `start_secs` is recorded immediately
/// before the kill, `end_secs` immediately after the restart's health-ready
/// signal fires. `end_secs` is left at `f64::INFINITY` while a restart is
/// still in flight, so a real-time consumer (the sampling loop) treats the gap
/// as still-open rather than briefly reappearing as closed.
#[derive(Debug, Clone, Copy)]
pub struct BootGap {
    pub start_secs: f64,
    pub end_secs: f64,
}

/// True if `elapsed_secs` falls inside any recorded [`BootGap`].
///
/// The window is half-open `[start_secs, end_secs)`, and `start_secs` is
/// recorded just BEFORE the `kill -9`, so a sample from the still-alive
/// pre-kill process can be excluded. That conservatism is intentional: dropping
/// a couple of trustworthy pre-kill samples is strictly safer than ever
/// admitting a post-kill, pre-reconcile spurious low/zero read into the slope
/// fit — the gap is deliberately a touch wider than the strict kill→ready span.
fn in_boot_gap(elapsed_secs: f64, boot_gaps: &[BootGap]) -> bool {
    boot_gaps
        .iter()
        .any(|g| elapsed_secs >= g.start_secs && elapsed_secs < g.end_secs)
}

/// Drop every sample that falls inside a recorded boot-recompute gap.
///
/// PURE and `retain`-style: samples and gap windows in, a filtered `Vec` out —
/// no process/HTTP/clock dependency. `main.rs`'s tombstone sampler calls this
/// once per scraped sample (so a spurious pre-reconcile read never enters the
/// series in the first place), and a calibration test drives it directly with
/// a fully synthetic post-kill 0 -> reconciled-total sequence (AC11) without
/// spawning any real process.
pub fn exclude_boot_gap_samples(
    samples: &[TombstoneSample],
    boot_gaps: &[BootGap],
) -> Vec<TombstoneSample> {
    samples
        .iter()
        .copied()
        .filter(|s| !in_boot_gap(s.elapsed_secs, boot_gaps))
        .collect()
}

/// Calibrated slope ceiling (MB/hour) for the soak's on-disk data-dir growth
/// assertion.
///
/// PRE-566 PLACEHOLDER: loosely calibrated so the deliberately-unbounded
/// pre-TODO-566 OR-Map tombstone growth (which grows the durable dir linearly
/// by design, see the module-level "Critical fence" rationale in the parent
/// spec) does not spuriously trip this clause during a bounded/short soak.
/// This is NOT the tight bounded-expectation value TODO-566/Gap 2 will
/// calibrate once the leak is bounded — do not read this as a proven bound.
pub const DEFAULT_DISK_THRESHOLD_MB_PER_HOUR: f64 = 50.0;

/// Absolute-growth guard (MB) for the disk slope clause.
///
/// PRE-566 PLACEHOLDER (see [`DEFAULT_DISK_THRESHOLD_MB_PER_HOUR`] doc) — not
/// the tight bounded-expectation guard TODO-566/Gap 2 will calibrate.
pub const DEFAULT_DISK_MIN_GROWTH_MB: f64 = 100.0;

/// Hard ceiling (MB) on peak on-disk data-dir size.
///
/// PRE-566 PLACEHOLDER (see [`DEFAULT_DISK_THRESHOLD_MB_PER_HOUR`] doc) — not
/// the tight bounded-expectation ceiling TODO-566/Gap 2 will calibrate.
pub const DEFAULT_DISK_CEILING_MB: f64 = 4096.0;

/// One on-disk data-dir size sample (`du -sk <dir>`, KiB -> MB).
#[derive(Debug, Clone, Copy)]
pub struct DiskSample {
    pub elapsed_secs: f64,
    pub disk_mb: f64,
}

/// Verdict of a disk-growth assessment. Mirrors [`MemoryAssessment`]'s shape so
/// callers (soak `main.rs`) can report RSS, tombstone bytes, and disk uniformly.
#[derive(Debug, Clone)]
pub struct DiskAssessment {
    pub samples: usize,
    pub first_mb: f64,
    pub peak_mb: f64,
    pub last_mb: f64,
    pub slope_mb_per_hour: f64,
    pub passed: bool,
    pub reason: Option<String>,
}

/// Sample the on-disk size of `dir` in megabytes via `du -sk` (KiB on both
/// macOS and Linux with `-k`). Returns `None` if `du` fails or its output
/// cannot be parsed (mirrors `sample_rss_mb`'s `None`-on-failure contract).
pub fn sample_disk_mb(dir: &Path) -> Option<f64> {
    let out = Command::new("du")
        .args(["-sk", dir.to_str()?])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    // `du -sk` prints "<KiB>\t<path>"; the KiB total is the first
    // whitespace-separated field.
    let kib: f64 = text.split_whitespace().next()?.parse().ok()?;
    Some(kib / 1024.0)
}

/// Assess a series of on-disk data-dir samples for bounded growth. Mirrors
/// [`assess`]'s shape and clauses verbatim (empty-samples blind-monitor
/// branch, least-squares slope via the shared OLS helper, growth/threshold and
/// ceiling clauses) so RSS, tombstone-byte, and disk gates report uniformly.
///
/// * `threshold_mb_per_hour` — maximum tolerated growth slope.
/// * `min_growth_mb` — absolute growth (peak − first) below which slope is
///   treated as noise (guards tiny/short runs).
/// * `ceiling_mb` — hard cap on peak on-disk size.
#[allow(clippy::cast_precision_loss)]
pub fn assess_disk(
    samples: &[DiskSample],
    threshold_mb_per_hour: f64,
    min_growth_mb: f64,
    ceiling_mb: f64,
) -> DiskAssessment {
    if samples.is_empty() {
        // Zero samples over a real run means the monitor was BLIND (`du` failing
        // or the data dir never resolving) — a leak would be invisible on disk
        // exactly as an unreachable `ps`/`/metrics` would be for RSS/tombstone
        // bytes. Fail rather than silently pass.
        return DiskAssessment {
            samples: 0,
            first_mb: 0.0,
            peak_mb: 0.0,
            last_mb: 0.0,
            slope_mb_per_hour: 0.0,
            passed: false,
            reason: Some(
                "no disk-usage samples collected — disk monitoring was blind (du failed or \
                 the data dir never resolved); cannot assert bounded disk growth"
                    .to_string(),
            ),
        };
    }

    let first_mb = samples[0].disk_mb;
    let last_mb = samples[samples.len() - 1].disk_mb;
    let peak_mb = samples.iter().fold(0.0_f64, |m, s| m.max(s.disk_mb));

    let points: Vec<(f64, f64)> = samples
        .iter()
        .map(|s| (s.elapsed_secs, s.disk_mb))
        .collect();
    let slope_mb_per_hour = least_squares_slope_per_hour(&points);

    let growth = peak_mb - first_mb;
    let mut reasons = Vec::new();

    if peak_mb > ceiling_mb {
        reasons.push(format!(
            "peak disk usage {peak_mb:.1}MB exceeds ceiling {ceiling_mb:.1}MB"
        ));
    }
    if growth >= min_growth_mb && slope_mb_per_hour > threshold_mb_per_hour {
        reasons.push(format!(
            "disk growth slope {slope_mb_per_hour:.1}MB/h exceeds {threshold_mb_per_hour:.1}MB/h \
             (total growth {growth:.1}MB over {} samples)",
            samples.len()
        ));
    }

    let passed = reasons.is_empty();
    DiskAssessment {
        samples: samples.len(),
        first_mb,
        peak_mb,
        last_mb,
        slope_mb_per_hour,
        passed,
        reason: if passed {
            None
        } else {
            Some(reasons.join("; "))
        },
    }
}

// This bench target is `harness = false` (it owns `main`), so these `#[test]`
// functions do NOT run under `cargo test --bench soak_harness` — libtest never
// drives them. They are executed as a real CI gate by the integration target
// `tests/soak_monitor_calibration.rs`, which re-includes this module under the
// standard harness. `allow(dead_code)` keeps the bench's own test-mode compile
// warning-free (the helper is unreferenced there because libtest is absent).
#[cfg(test)]
#[allow(dead_code)]
mod tests {
    use super::*;

    /// Build a 72h series sampled hourly with a constant per-hour slope,
    /// starting at `first_mb`.
    fn linear_72h(first_mb: f64, slope_mb_per_hour: f64) -> Vec<MemSample> {
        (0..=72)
            .map(|h| MemSample {
                elapsed_secs: f64::from(h) * 3600.0,
                rss_mb: first_mb + slope_mb_per_hour * f64::from(h),
            })
            .collect()
    }

    /// A ~4 MB/h linear tombstone leak over 72h (≈288 MB total growth) MUST FAIL
    /// under the calibrated defaults — this is the exact shape the soak's OR
    /// churn drives, and the whole point of the calibration is that it can no
    /// longer false-GREEN.
    #[test]
    fn calibration_fails_linear_tombstone_leak() {
        let samples = linear_72h(220.0, 4.0);
        let a = assess(
            &samples,
            DEFAULT_MEM_THRESHOLD_MB_PER_HOUR,
            DEFAULT_MEM_MIN_GROWTH_MB,
            2048.0,
        );
        assert!(
            !a.passed,
            "4 MB/h linear leak must fail the calibrated gate; slope={:.2} reason={:?}",
            a.slope_mb_per_hour, a.reason
        );
        assert!(a.slope_mb_per_hour > DEFAULT_MEM_THRESHOLD_MB_PER_HOUR);
    }

    /// The lower edge of the estimated leak band (~3 MB/h) must also FAIL — the
    /// gate must not have a blind spot just above plateau.
    #[test]
    fn calibration_fails_low_end_leak() {
        let samples = linear_72h(220.0, 3.0);
        let a = assess(
            &samples,
            DEFAULT_MEM_THRESHOLD_MB_PER_HOUR,
            DEFAULT_MEM_MIN_GROWTH_MB,
            2048.0,
        );
        assert!(!a.passed, "3 MB/h leak must fail; reason={:?}", a.reason);
    }

    /// Pin the lower edge of the leak band: a 2.5 MB/h series (180 MB over 72h)
    /// must FAIL at the 2.0 MB/h threshold. Without this, a future loosening of
    /// the threshold toward the estimated leak band (e.g. to 2.5) would still
    /// pass the 3.0/4.0 tests yet silently false-GREEN a 2.3 MB/h leak — this
    /// test fails the moment the threshold creeps up to meet the band.
    #[test]
    fn calibration_pins_low_band_edge() {
        let samples = linear_72h(220.0, 2.5);
        let a = assess(
            &samples,
            DEFAULT_MEM_THRESHOLD_MB_PER_HOUR,
            DEFAULT_MEM_MIN_GROWTH_MB,
            2048.0,
        );
        assert!(!a.passed, "2.5 MB/h leak must fail; reason={:?}", a.reason);
    }

    /// A genuine in-place-overwrite plateau (slope ~0, tiny bounded jitter) MUST
    /// PASS — otherwise the gate false-FAILs every healthy long run.
    #[test]
    fn calibration_passes_plateau() {
        // Flat at 300 MB with ±1 MB sawtooth jitter, hourly over 72h.
        let samples: Vec<MemSample> = (0..=72)
            .map(|h| MemSample {
                elapsed_secs: f64::from(h) * 3600.0,
                rss_mb: 300.0 + if h % 2 == 0 { 1.0 } else { -1.0 },
            })
            .collect();
        let a = assess(
            &samples,
            DEFAULT_MEM_THRESHOLD_MB_PER_HOUR,
            DEFAULT_MEM_MIN_GROWTH_MB,
            2048.0,
        );
        assert!(
            a.passed,
            "plateau must pass; slope={:.3} peak={:.1} reason={:?}",
            a.slope_mb_per_hour, a.peak_mb, a.reason
        );
    }

    /// Proof the tightening is what makes it a gate: the SAME 4 MB/h leak that
    /// the calibrated defaults reject would have PASSED under the old loose
    /// thresholds (25 MB/h slope, 150 MB min-growth). Guards against a future
    /// loosening silently restoring the false-GREEN.
    #[test]
    fn old_loose_thresholds_would_false_green_the_leak() {
        let samples = linear_72h(220.0, 4.0);
        let old = assess(&samples, 25.0, 150.0, 2048.0);
        assert!(
            old.passed,
            "the old 25 MB/h threshold is exactly the false-GREEN this spec closes"
        );
    }

    /// A plateau that parks at a high-but-flat RSS (allocator retention) must
    /// still PASS on slope — retention is not growth. Only the hard ceiling may
    /// fail it, which it does not here.
    #[test]
    fn calibration_passes_high_flat_retention() {
        let samples = linear_72h(900.0, 0.0);
        let a = assess(
            &samples,
            DEFAULT_MEM_THRESHOLD_MB_PER_HOUR,
            DEFAULT_MEM_MIN_GROWTH_MB,
            2048.0,
        );
        assert!(
            a.passed,
            "flat-but-high retention must pass; reason={:?}",
            a.reason
        );
    }

    /// A *noisy* linear leak — a 3.5 MB/h trend with a large ±40 MB sawtooth on
    /// top of it (allocator GC / cache warmup jitter) — must still FAIL. This
    /// pins the robustness of the growth clause: because `assess` computes total
    /// growth as `peak_mb - first_mb` (fold-max, not `last - first`), a low final
    /// sample cannot drag computed growth below the min-growth guard, so endpoint
    /// noise cannot suppress the slope clause on a real trend.
    #[test]
    fn calibration_fails_noisy_linear_leak() {
        // 3.5 MB/h trend + deterministic ±40 MB oscillation, hourly over 72h.
        let samples: Vec<MemSample> = (0..=72)
            .map(|h| MemSample {
                elapsed_secs: f64::from(h) * 3600.0,
                rss_mb: 220.0 + 3.5 * f64::from(h) + 40.0 * (f64::from(h % 3) - 1.0),
            })
            .collect();
        let a = assess(
            &samples,
            DEFAULT_MEM_THRESHOLD_MB_PER_HOUR,
            DEFAULT_MEM_MIN_GROWTH_MB,
            2048.0,
        );
        assert!(
            !a.passed,
            "noisy 3.5 MB/h leak must fail; slope={:.2} peak={:.1} reason={:?}",
            a.slope_mb_per_hour, a.peak_mb, a.reason
        );
        assert!(a.slope_mb_per_hour > DEFAULT_MEM_THRESHOLD_MB_PER_HOUR);
    }

    /// The Hetzner 72h runner and this module are two independent sources of
    /// truth for the same slope threshold. The runner default MUST track
    /// [`DEFAULT_MEM_THRESHOLD_MB_PER_HOUR`] so the 72h gate cannot be silently
    /// loosened by editing only one of them (e.g. bumping the shell default back
    /// to 25 without failing any test — the exact false-GREEN this spec closes).
    /// `include_str!` is resolved relative to this file, i.e. the `soak_harness` dir.
    #[test]
    fn hetzner_runner_default_matches_calibrated_constant() {
        let script = include_str!("hetzner-soak-runner.sh");
        // `{}` on an integral f64 prints without a trailing `.0` (2.0 -> "2"),
        // matching the integer MB/h the shell default carries.
        let expected =
            format!("MEM_THRESHOLD=\"${{MEM_THRESHOLD:-{DEFAULT_MEM_THRESHOLD_MB_PER_HOUR}}}\"");
        assert!(
            script.contains(&expected),
            "hetzner-soak-runner.sh MEM_THRESHOLD default must match \
             monitor::DEFAULT_MEM_THRESHOLD_MB_PER_HOUR (expected {expected:?})"
        );
    }

    /// Build an hourly-sampled tombstone-byte series with a constant per-hour
    /// growth rate, starting at `first_bytes`.
    fn linear_bytes_series(
        first_bytes: u64,
        hours: u32,
        bytes_per_hour: u64,
    ) -> Vec<TombstoneSample> {
        (0..=hours)
            .map(|h| TombstoneSample {
                elapsed_secs: f64::from(h) * 3600.0,
                bytes: first_bytes + u64::from(h) * bytes_per_hour,
            })
            .collect()
    }

    /// A clearly-linear tombstone-byte-growth series — well above the tight
    /// per-hour threshold — MUST FAIL. Mirrors the RSS gate's
    /// `calibration_fails_linear_tombstone_leak`, but at the byte gauge's much
    /// tighter scale: with no RSS noise floor, a sustained per-hour growth this
    /// small is already real signal (see module docs).
    #[test]
    fn calibration_fails_linear_tombstone_bytes() {
        let samples = linear_bytes_series(1_000, 24, 5_000);
        let a = assess_tombstone_bytes(
            &samples,
            DEFAULT_TOMBSTONE_BYTES_THRESHOLD_PER_HOUR,
            DEFAULT_TOMBSTONE_BYTES_MIN_GROWTH,
            DEFAULT_TOMBSTONE_BYTES_MIN_WINDOW_SECS,
        );
        assert!(
            !a.passed,
            "5000 bytes/h linear tombstone-byte growth must fail the tight gate; \
             slope={:.1} reason={:?}",
            a.slope_bytes_per_hour, a.reason
        );
        assert!(a.slope_bytes_per_hour > DEFAULT_TOMBSTONE_BYTES_THRESHOLD_PER_HOUR);
    }

    /// A flat tombstone-byte series (churn stopped, or a future prune landed)
    /// MUST PASS — otherwise the gate false-FAILs every healthy run.
    #[test]
    fn calibration_passes_flat_tombstone_bytes() {
        let samples = linear_bytes_series(50_000, 24, 0);
        let a = assess_tombstone_bytes(
            &samples,
            DEFAULT_TOMBSTONE_BYTES_THRESHOLD_PER_HOUR,
            DEFAULT_TOMBSTONE_BYTES_MIN_GROWTH,
            DEFAULT_TOMBSTONE_BYTES_MIN_WINDOW_SECS,
        );
        assert!(
            a.passed,
            "flat tombstone-byte series must pass; slope={:.3} reason={:?}",
            a.slope_bytes_per_hour, a.reason
        );
    }

    /// Zero samples means the metrics scrape was blind — must FAIL, same
    /// rationale as the RSS gate's empty-samples branch (AC6: a zero-sample
    /// run must not silently report bounded growth).
    #[test]
    fn calibration_fails_zero_tombstone_samples() {
        let a = assess_tombstone_bytes(
            &[],
            DEFAULT_TOMBSTONE_BYTES_THRESHOLD_PER_HOUR,
            DEFAULT_TOMBSTONE_BYTES_MIN_GROWTH,
            DEFAULT_TOMBSTONE_BYTES_MIN_WINDOW_SECS,
        );
        assert!(!a.passed, "zero samples must fail as a blind monitor");
    }

    /// AC10: the prune-disabled negative control, as a synthetic calibration
    /// unit test (no live prune-disable toggle exists in the harness/server —
    /// the live run is a documented manual step). A sustained byte-growth rate
    /// realistic for a bounded 10-60 min run (single-digit KB total) MUST FAIL
    /// — and specifically on the BYTE gate, not RSS: RSS's
    /// `DEFAULT_MEM_MIN_GROWTH_MB` (80 MB) guard would false-GREEN this exact
    /// magnitude of growth, so the byte gate is the only instrument that can
    /// actually deliver the required FAIL.
    #[test]
    fn calibration_sustained_growth_fails_byte_gate_not_rss() {
        // 4 hourly samples (enough points for a non-degenerate last-half OLS
        // window) at 5000 bytes/h -> ~20 KB total growth over 4h, single-digit
        // scale next to RSS's 80 MB guard.
        let byte_samples = linear_bytes_series(1_000, 4, 5_000);
        let bytes_assessment = assess_tombstone_bytes(
            &byte_samples,
            DEFAULT_TOMBSTONE_BYTES_THRESHOLD_PER_HOUR,
            DEFAULT_TOMBSTONE_BYTES_MIN_GROWTH,
            DEFAULT_TOMBSTONE_BYTES_MIN_WINDOW_SECS,
        );
        assert!(
            !bytes_assessment.passed,
            "sustained byte growth must fail the byte gate; slope={:.1} reason={:?}",
            bytes_assessment.slope_bytes_per_hour, bytes_assessment.reason
        );

        // Same tiny (single-digit-KB-scale) magnitude of growth, expressed as
        // an RSS series, must NOT fail the RSS gate — proving the byte gate,
        // not RSS, is what makes the negative control fail.
        let rss_samples = vec![
            MemSample {
                elapsed_secs: 0.0,
                rss_mb: 220.0,
            },
            MemSample {
                elapsed_secs: 4.0 * 3600.0,
                rss_mb: 220.02,
            },
        ];
        let rss_assessment = assess(
            &rss_samples,
            DEFAULT_MEM_THRESHOLD_MB_PER_HOUR,
            DEFAULT_MEM_MIN_GROWTH_MB,
            2048.0,
        );
        assert!(
            rss_assessment.passed,
            "a single-digit-KB-scale leak must NOT fail the RSS gate (its 80 MB \
             min-growth guard structurally cannot see it); reason={:?}",
            rss_assessment.reason
        );
    }

    /// The grow-then-flatten shape a residency-tracking gauge WOULD produce once
    /// the OR-Map bound engages: tombstone bytes grow while the churn stream
    /// fills the keyspace, then flatten. Only the LAST-HALF window should drive
    /// the fitted slope (R9(d)), so this PASSes even though the run grew earlier.
    ///
    /// NOTE — this validates the OLS/last-half-window MATH only; it is NOT a
    /// reachable production PASS today. The `bytes` series here is hand-authored
    /// to flatten, but the live gauge is additive-only and cannot flatten under
    /// sustained churn (see `calibration_additive_only_gauge_never_plateaus`
    /// below) — which is exactly why `main.rs` surfaces the slope report-only
    /// rather than hard-gating it.
    #[test]
    fn calibration_delayed_plateau_grow_then_flatten_passes() {
        let mut samples = Vec::new();
        for h in 0..=11u32 {
            samples.push(TombstoneSample {
                elapsed_secs: f64::from(h) * 3600.0,
                bytes: 1_000 + u64::from(h) * 1_000,
            });
        }
        for h in 12..=23u32 {
            samples.push(TombstoneSample {
                elapsed_secs: f64::from(h) * 3600.0,
                bytes: 12_000,
            });
        }
        let a = assess_tombstone_bytes(
            &samples,
            DEFAULT_TOMBSTONE_BYTES_THRESHOLD_PER_HOUR,
            DEFAULT_TOMBSTONE_BYTES_MIN_GROWTH,
            DEFAULT_TOMBSTONE_BYTES_MIN_WINDOW_SECS,
        );
        assert!(
            a.passed,
            "delayed-plateau (grow-then-flatten) must pass on the last-half-window \
             slope; slope={:.2} reason={:?}",
            a.slope_bytes_per_hour, a.reason
        );
    }

    /// Pins the REAL production shape and the reason the slope is report-only.
    ///
    /// The exported gauge (`add_tombstone_bytes`) is additive-only: it counts
    /// every tombstone-add and is never decremented on prune, and the OR-Map
    /// epoch prune is dark until a follow-up supplies the frontier watermark.
    /// So under sustained OR churn the gauge climbs monotonically at the
    /// tombstone-*creation* rate and never flattens within a process life — the
    /// grow-then-flatten shape the plateau statistic looks for cannot occur.
    /// This test feeds exactly that monotone shape (well past the min-window
    /// floor) and asserts the assessment reports NOT-passed — which is why
    /// `main.rs` routes this verdict to `pending_gates` (report-only) instead
    /// of hard-gating the run. When a prune-activation follow-up makes the gauge
    /// track residency, replace this with a genuine live-plateau PASS test and
    /// re-promote the slope to a hard gate.
    #[test]
    fn calibration_additive_only_gauge_never_plateaus() {
        // ~6h of steady creation at 5000 B/h — a multi-hour last-half window
        // (well past the 120s min-window floor), monotone, no flatten. This is
        // the shape a real sustained-churn soak actually produces today.
        let samples = linear_bytes_series(1_000, 6, 5_000);
        let a = assess_tombstone_bytes(
            &samples,
            DEFAULT_TOMBSTONE_BYTES_THRESHOLD_PER_HOUR,
            DEFAULT_TOMBSTONE_BYTES_MIN_GROWTH,
            DEFAULT_TOMBSTONE_BYTES_MIN_WINDOW_SECS,
        );
        assert!(
            !a.passed,
            "the additive-only gauge's monotone-growth shape must NOT be reported \
             as a plateau — this is why the slope is surfaced report-only, not \
             hard-gated; slope={:.1} reason={:?}",
            a.slope_bytes_per_hour, a.reason
        );
    }

    /// R9(d) proof: on the same grow-then-flatten shape, the FULL-window slope
    /// is dragged well above the gate threshold by the earlier growth even
    /// though the run has genuinely plateaued, while the LAST-HALF-window
    /// slope correctly reports near-zero. This is why the plateau statistic
    /// must be last-half-window, not full-window.
    #[test]
    fn last_half_window_slope_differs_from_full_window_on_delayed_plateau() {
        let mut points = Vec::new();
        for h in 0..=11u32 {
            points.push((f64::from(h) * 3600.0, 1_000.0 + f64::from(h) * 5_000.0));
        }
        for h in 12..=23u32 {
            points.push((f64::from(h) * 3600.0, 56_000.0));
        }
        let full = least_squares_slope_per_hour(&points);
        let last_half = last_half_window_slope_per_hour(&points);
        assert!(
            full > DEFAULT_TOMBSTONE_BYTES_THRESHOLD_PER_HOUR,
            "sanity: the full-window slope on this shape must itself be large \
             enough to matter (full={full:.1} B/h)"
        );
        assert!(
            last_half.abs() < DEFAULT_TOMBSTONE_BYTES_THRESHOLD_PER_HOUR,
            "the last-half-window slope on a genuinely-flattened tail must be \
             near zero even though the full-window slope is not \
             (full={full:.1} B/h, last_half={last_half:.1} B/h)"
        );
    }

    /// AC11: a synthetic post-kill sample sequence (a spurious near-zero read
    /// during the boot-recompute gap, then the reconciled-total jump) must NOT
    /// pollute the fitted slope. [`exclude_boot_gap_samples`] drops every
    /// sample inside the recorded [`BootGap`] before the series reaches
    /// [`assess_tombstone_bytes`], so the gate sees only reconciled, continuous
    /// data.
    #[test]
    fn calibration_boot_gap_exclusion_does_not_trip_gate() {
        // Life 0 plateaus at 50_000 bytes from t=0 to t=3600 (1h). At t=3605
        // the process is killed; the restarted process's gauge is not yet
        // reconciled until t=3610 (a 5s boot-recompute gap) — a scrape taken
        // at t=3607 during that window reads a spurious near-zero total. From
        // t=3610 onward, life 1 resumes at a slightly higher plateau (a small
        // amount of legitimate growth, restart-survivable — not a reset to 0).
        let samples = vec![
            TombstoneSample {
                elapsed_secs: 0.0,
                bytes: 50_000,
            },
            TombstoneSample {
                elapsed_secs: 1_800.0,
                bytes: 50_000,
            },
            TombstoneSample {
                elapsed_secs: 3_600.0,
                bytes: 50_000,
            },
            // Spurious boot-gap read: the new process's counter has not yet
            // been reconciled by `reconcile_tombstone_bytes`.
            TombstoneSample {
                elapsed_secs: 3_607.0,
                bytes: 5,
            },
            TombstoneSample {
                elapsed_secs: 3_610.0,
                bytes: 50_010,
            },
            TombstoneSample {
                elapsed_secs: 5_400.0,
                bytes: 50_010,
            },
            TombstoneSample {
                elapsed_secs: 7_200.0,
                bytes: 50_010,
            },
        ];
        let boot_gaps = vec![BootGap {
            start_secs: 3_605.0,
            end_secs: 3_610.0,
        }];

        let filtered = exclude_boot_gap_samples(&samples, &boot_gaps);
        assert_eq!(
            filtered.len(),
            samples.len() - 1,
            "exactly the one spurious in-gap sample must be dropped"
        );
        assert!(
            !filtered.iter().any(|s| s.bytes == 5),
            "the spurious near-zero in-gap sample must be excluded"
        );

        let a = assess_tombstone_bytes(
            &filtered,
            DEFAULT_TOMBSTONE_BYTES_THRESHOLD_PER_HOUR,
            DEFAULT_TOMBSTONE_BYTES_MIN_GROWTH,
            DEFAULT_TOMBSTONE_BYTES_MIN_WINDOW_SECS,
        );
        assert!(
            a.passed,
            "boot-gap-excluded plateau must pass; slope={:.2} reason={:?}",
            a.slope_bytes_per_hour, a.reason
        );

        // Sanity: the unfiltered series (spurious dip included) is actually
        // capable of tripping the gate, or this test would not be proving the
        // exclusion is load-bearing.
        let unfiltered = assess_tombstone_bytes(
            &samples,
            DEFAULT_TOMBSTONE_BYTES_THRESHOLD_PER_HOUR,
            DEFAULT_TOMBSTONE_BYTES_MIN_GROWTH,
            DEFAULT_TOMBSTONE_BYTES_MIN_WINDOW_SECS,
        );
        assert!(
            !unfiltered.passed,
            "sanity: the unfiltered series with the boot-gap dip must actually \
             trip the gate, or this test would not be proving anything"
        );
    }

    /// Regression lock for the blocking CI "Short no-crash soak must be GREEN"
    /// gate: a too-short-to-plateau run grows the tombstone gauge by a few KB
    /// while the keyspace fills but has not had wall-clock time to plateau. Its
    /// last-half fit window spans only seconds, so the per-hour extrapolation is
    /// meaningless (a few KB over ~25s reads as a six-figure B/h "leak"). The
    /// minimum-window-span guard MUST suppress the slope clause so the run
    /// PASSES. Proven load-bearing: the SAME series with the guard disabled
    /// (`min_window_secs = 0.0`) FAILS — this is exactly the reproduced smoke
    /// regression the guard closes.
    #[test]
    fn calibration_short_run_below_min_window_passes() {
        // 6 samples at 5s intervals = 25s total, linear 0 -> 6000 bytes — the
        // exact shape reproduced FAILing the blocking CI smoke gate.
        let samples: Vec<TombstoneSample> = (0..6u32)
            .map(|i| TombstoneSample {
                elapsed_secs: f64::from(i) * 5.0,
                bytes: u64::from(i) * 1_200,
            })
            .collect();

        // Guard disabled: the sub-minute per-hour slope is enormous and trips
        // the gate — without this the test would prove nothing.
        let no_guard = assess_tombstone_bytes(
            &samples,
            DEFAULT_TOMBSTONE_BYTES_THRESHOLD_PER_HOUR,
            DEFAULT_TOMBSTONE_BYTES_MIN_GROWTH,
            0.0,
        );
        assert!(
            !no_guard.passed,
            "sanity: with the window guard disabled the sub-minute slope must \
             trip the gate (slope={:.0} B/h) — otherwise this test is vacuous",
            no_guard.slope_bytes_per_hour
        );
        assert!(no_guard.slope_bytes_per_hour > DEFAULT_TOMBSTONE_BYTES_THRESHOLD_PER_HOUR);

        // Real guard: the too-short window is suppressed and the run passes.
        let guarded = assess_tombstone_bytes(
            &samples,
            DEFAULT_TOMBSTONE_BYTES_THRESHOLD_PER_HOUR,
            DEFAULT_TOMBSTONE_BYTES_MIN_GROWTH,
            DEFAULT_TOMBSTONE_BYTES_MIN_WINDOW_SECS,
        );
        assert!(
            guarded.passed,
            "a too-short-to-plateau run (last-half window well under the \
             {:.0}s floor) must pass; slope={:.0} B/h reason={:?}",
            DEFAULT_TOMBSTONE_BYTES_MIN_WINDOW_SECS, guarded.slope_bytes_per_hour, guarded.reason
        );
    }

    /// Build an hourly-sampled disk-usage series with a constant per-hour
    /// growth rate, starting at `first_mb`.
    fn linear_disk_series(first_mb: f64, hours: u32, mb_per_hour: f64) -> Vec<DiskSample> {
        (0..=hours)
            .map(|h| DiskSample {
                elapsed_secs: f64::from(h) * 3600.0,
                disk_mb: first_mb + mb_per_hour * f64::from(h),
            })
            .collect()
    }

    /// Zero disk samples means `du` never succeeded / the data dir never
    /// resolved — the monitor was BLIND. AC2: must FAIL, same rationale as the
    /// RSS/tombstone-byte empty-samples branches, since this is the ONE clause
    /// that hard-gates `passed` in `main.rs`.
    #[test]
    fn calibration_fails_zero_disk_samples() {
        let a = assess_disk(
            &[],
            DEFAULT_DISK_THRESHOLD_MB_PER_HOUR,
            DEFAULT_DISK_MIN_GROWTH_MB,
            DEFAULT_DISK_CEILING_MB,
        );
        assert!(!a.passed, "zero disk samples must fail as a blind monitor");
    }

    /// A flat disk-usage series (durable dir stopped growing) MUST PASS —
    /// otherwise the gate false-FAILs every healthy in-place-overwrite run.
    #[test]
    fn calibration_passes_flat_disk_series() {
        let samples = linear_disk_series(500.0, 72, 0.0);
        let a = assess_disk(
            &samples,
            DEFAULT_DISK_THRESHOLD_MB_PER_HOUR,
            DEFAULT_DISK_MIN_GROWTH_MB,
            DEFAULT_DISK_CEILING_MB,
        );
        assert!(
            a.passed,
            "flat disk series must pass; slope={:.3} reason={:?}",
            a.slope_mb_per_hour, a.reason
        );
    }

    /// A steep, clearly-linear disk-growth series (well above the loosely
    /// calibrated pre-566 threshold) MUST be flagged by `assess_disk`'s
    /// `passed` field — this only proves the assessment's own slope clause is
    /// correctly wired, NOT that it gates the run (it is report-only in
    /// `main.rs`; see AC3/AC2).
    #[test]
    fn calibration_flags_steep_linear_disk_growth() {
        let samples = linear_disk_series(200.0, 24, 200.0);
        let a = assess_disk(
            &samples,
            DEFAULT_DISK_THRESHOLD_MB_PER_HOUR,
            DEFAULT_DISK_MIN_GROWTH_MB,
            DEFAULT_DISK_CEILING_MB,
        );
        assert!(
            !a.passed,
            "steep 200 MB/h disk growth must be flagged; slope={:.1} reason={:?}",
            a.slope_mb_per_hour, a.reason
        );
        assert!(a.slope_mb_per_hour > DEFAULT_DISK_THRESHOLD_MB_PER_HOUR);
    }
}
