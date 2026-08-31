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
//! `topgun_ormap_tombstone_bytes` (the DECREMENTABLE gauge, sampled over HTTP
//! from `GET /metrics` — distinct from the monotonic `_total` creation-rate
//! counter also exported on the same endpoint) is a direct,
//! residency-independent count of tombstone bytes on the write path — unlike
//! RSS it carries no allocator retention, no read/GC jitter, and no
//! cache-warmup wobble. Every unit of observed growth is either a newly
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
//! is decided in `main.rs`, not here. The byte **slope** is a HARD gate there
//! in every run class. The durable-corpus level clause beside it is
//! report-only: a control cell that injected no fault at all breached it, so it
//! is recorded and rendered but decides no verdict, and the slope gates exactly
//! as it always did — so no run configuration is left with neither.
//! The gauge is restart-survivable (`reconcile_tombstone_bytes` in
//! `storage/record.rs` re-seeds it via `set_tombstone_bytes` at boot) AND
//! decrementable within a process life: every tombstone-add increments it and
//! a successful prune-drop decrements it (`sub_tombstone_bytes`, wired on the
//! CRDT write path's remove/prune handling). Decrementing alone is not
//! sufficient for a plateau, though — the prune only fires once the server's
//! per-device causal frontier low-water-mark has advanced past a tombstone's
//! epoch, and the low-water-mark is vacuously 0 (prune NOTHING) until at least
//! one client actually runs the confirm-apply protocol. `main.rs` therefore
//! also drives a tracked-and-ACKing client (`SoakClient::connect_tracked` +
//! `confirm_apply`) alongside the churn clients for the run's duration, which
//! is what makes the gauge's plateau — and thus the hard gate — reachable in
//! practice rather than merely possible in principle. `main.rs`'s `--no-ack`
//! and `--inject-slow-leak` modes are the negative/slow-leak controls that
//! exercise this: disabling the tracked client's ack loop pins the
//! low-water-mark at 0 and must trip the gate, and a deliberately
//! slow-acking second tracked client calibrates the OLS slope's detection
//! floor against a small, non-instantaneous leak. The blind-monitor
//! zero-sample case remains a second, independent hard gate (an unreachable
//! `/metrics` scrape is a harness defect regardless of leak magnitude). The
//! RSS gate above remains a coarse, non-tombstone backstop. (This module's
//! `passed: bool` on [`TombstoneAssessment`] drives both the hard-gate
//! decision in `main.rs` — which applies in every run class — and the
//! calibration tests below; the assessment itself does not know or care
//! whether its caller treats a breach as report-only or hard-gating.)
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
/// The index at which a sample series splits into its first and last halves.
///
/// The last-half window is `&points[last_half_split_index(points.len())..]`, so
/// an ODD count puts the middle sample in the RECENT half. Every consumer that
/// partitions a series — the slope statistic, its span guard and the
/// durable-corpus level clause — derives the split from here, so the partitions
/// are provably identical and a reader comparing two of them is comparing like
/// with like. It takes a length rather than a slice so a series of counted
/// BYTE totals can share it without any of them becoming an `f64`.
const fn last_half_split_index(len: usize) -> usize {
    len / 2
}

fn last_half_window(points: &[(f64, f64)]) -> &[(f64, f64)] {
    &points[last_half_split_index(points.len())..]
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
/// threshold measures is a HARD gate in `main.rs` in every run class; the
/// durable-corpus level clause beside it is report-only and takes none of
/// them over. The decrementable gauge is
/// expected to plateau under sustained churn now that a tracked-and-ACKing
/// client drives the server's low-water-mark forward (see the module-level
/// "Tombstone-byte gate" doc above), subject to the min-window-span guard and
/// boot-gap exclusion. The blind-monitor zero-sample clause is a second,
/// independent hard gate, and it too is unconditional: it asserts harness
/// health, not the tombstone property. RSS above is the coarse backstop.
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
/// per-hour slope clause is allowed to hard-gate the run.
///
/// This floor governs the slope clause alone. The durable-corpus level clause
/// selects no run class away from it: that clause is report-only, so the slope
/// decides every run, and no run configuration is left ungated.
///
/// The slope is a per-hour EXTRAPOLATION (bytes/sec × 3600). Over a sub-minute
/// window the `3600 / span_secs` amplification is enormous: a healthy short run
/// that has simply not yet had time to plateau (e.g. the 25s blocking CI "Short
/// no-crash soak", ~6 samples over ~25s with a few KB of ordinary ramp-up)
/// extrapolates to a six-figure B/h rate and would surface a spurious breach.
/// Below this floor the slope carries no plateau signal — a leak and a
/// not-yet-plateaued healthy run are indistinguishable — so the clause is
/// suppressed (no breach is emitted for it) and the assessment passes on the
/// blind-monitor + absolute-growth clauses alone. (The slope is a HARD gate
/// once the window clears this floor — see
/// [`DEFAULT_TOMBSTONE_BYTES_THRESHOLD_PER_HOUR`];
/// this floor is what keeps that hard gate from crying wolf on a short run.)
///
/// 120s sits well above the smoke run's ~10-15s last-half span (suppressed) and
/// well below a real bounded soak's window (a 10-60 min live run's last-half
/// span is minutes, so a genuine leak still trips the clause; the 72h soak's
/// span is orders of magnitude above it). The gauge is restart-survivable, so a
/// crash-enabled long run keeps a continuous series whose window clears the
/// floor.
pub const DEFAULT_TOMBSTONE_BYTES_MIN_WINDOW_SECS: f64 = 120.0;

/// One tombstone-byte-gauge sample (`topgun_ormap_tombstone_bytes`, the
/// decrementable gauge scraped over HTTP — not the monotonic `_total` counter).
/// `bytes` is `u64` — a counted byte total is a non-negative integer, never a
/// float.
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

// ---------------------------------------------------------------------------
// Durable-layer OR tombstone corpus: level/ceiling estimator.
//
// The items below are the estimator's frozen surface. They are `allow`ed for
// dead code because the soak binary does not reference them until the sampler
// and the verdict re-point are wired; the calibration integration target
// already includes this module under its own `allow(dead_code)`.
// ---------------------------------------------------------------------------

/// Level headroom (bytes) between the first-half and last-half corpus peaks.
///
/// Derived, not chosen, and every number below is either quoted from a
/// committed source with its `file:line` or computed from two such numbers with
/// the arithmetic shown. All three steps are normalised to the SAME window: the
/// control cells run 900 s at `--crash-interval 120`, so the corpus series
/// spans ≈ 780 s (terminal scan minus FIRST checkpoint) and its last half
/// covers ≈ 390 s = 0.108333 h.
///
/// 1. NOISE, EXPRESSED AS A LEVEL RATHER THAN A RATE. The recorded width-100
///    spread of 8,756 B/h was fitted over a 900 s = 0.25 h last-half window
///    (`spec356-manifest.md:1634`, `:1961-1962`), so the primitive noise
///    magnitude is a LEVEL of `8,756 × 0.25 = 2,189 B`; renormalised to the
///    390 s window it is `8,756 × 0.108333 = 948.6 B`. 65,536 sits 29.9× above
///    the as-measured figure and 69.1× above the normalised one.
/// 2. ABOVE THE HEALTHY SIGNAL. The width-100 keeping-up cell recorded a
///    backlog delta of +8,602 B over its own coordinate last-half window of 90
///    rows / 900 s (`spec356-manifest.md:1961-1964`). Pro-rated to 390 s that
///    is `8,602 × 390 / 900 = 3,727.5 B`, so the headroom sits 17.6× above it.
/// 3. BELOW THE UNHEALTHY SIGNAL. The `long` cell's coordinate last-half window
///    carried +5,303,731 B of backlog growth over a span of 7,190 s
///    (`spec356-manifest.md:1660-1661`), i.e. 2,655,565 B/h; over the 390 s
///    window that is 287,686 B, which is 4.4× this headroom.
///
/// Healthy ≈ 3,727.5 B, headroom 65,536 B, unhealthy ≈ 287,686 B — roughly one
/// order of magnitude of margin on each side, with a 77× total separation
/// (`287,686 / 3,727.5`). That separation is window-INVARIANT: it is a ratio of
/// two quantities pro-rated by the same factor. No claim of two orders of
/// magnitude is made, because the arithmetic does not support one.
///
/// THE PROXY IS DISCLOSED, NOT GLOSSED. Every magnitude above is a gauge or
/// counter figure; no durable-corpus noise floor has ever been measured. What
/// the committed evidence DOES bound is the substitution error of deriving a
/// byte headroom from gauge magnitudes and applying it to corpus magnitudes:
/// the 29 terminal `tombstone_corpus_redb_scan:` lines under
/// `benches/soak_harness/evidence/` (29 files, 21 distinct runs) each render the
/// scanned corpus beside the last gauge value, and across all of them the
/// ABSOLUTE gap never exceeds 27,925 B, so `65,536 / 27,925 = 2.3×`. The bound
/// is stated in bytes and not as a percentage because the relative gap is not
/// uniform (0.5–3 % on the ≥ 400 KB cells, tens of percent on the ~20–50 KB
/// ones) and an absolute bound is what this clause actually consumes. Those
/// lines are single terminal scalars, one per run: they are NOT a noise floor
/// and NOT a retune input. Revising this constant on measured durable-corpus
/// data is a recorded revision, never a keyboard retune.
pub const DEFAULT_TOMBSTONE_CORPUS_HEADROOM_BYTES: u64 = 65_536;

/// Minimum wall-clock span (seconds) of the corpus series before L1 may decide.
///
/// Five times [`DEFAULT_TOMBSTONE_BYTES_MIN_WINDOW_SECS`]: far above the 25 s
/// blocking smoke, far below any real soak.
///
/// It is also the BINDING guard on a control cell's duration, which is why the
/// admissible range is derived here rather than chosen at the keyboard. At
/// `--crash-interval 120` a cell of duration `D` puts its first checkpoint at
/// ≈ 120 s and its terminal scan at ≈ `D`, so
/// `span_secs ≈ D − 120` (terminal minus FIRST checkpoint, not minus `t = 0`)
/// and `samples = floor(D / 120) + 1`. The sample guard `samples >= 4` binds at
/// `D >= 360`; this span guard binds at `D − 120 >= 600`, i.e. `D >= 720`, and
/// is therefore the binding one. The admissible range is
/// `730 s <= D <= 900 s` — the 730 s lower bound carries a 10 s cushion over
/// the derived 720 s for the strict comparison and for a first checkpoint that
/// fires marginally late, and 900 s is the top of the range. A cell shorter
/// than 730 s is inadmissible by construction.
pub const DEFAULT_TOMBSTONE_CORPUS_MIN_SPAN_SECS: f64 = 600.0;

/// Minimum corpus samples before L1 may decide (at least 2 per half).
///
/// Two per half, so neither half-peak is a single point — the degenerate case
/// [`last_half_window_span_secs`]'s own guard excludes for the same reason. At
/// `--crash-interval 120` a 900 s cell yields 8 samples (7 checkpoints plus the
/// terminal scan), clearing this guard with margin.
pub const DEFAULT_TOMBSTONE_CORPUS_MIN_SAMPLES: usize = 4;

/// L2's absolute ceiling in bytes. `None` = DISARMED, and that is the default.
///
/// Arming it honestly requires a validated ceiling, and validating one requires
/// a plateau demonstration that is out of scope here — so the clause ships
/// implemented, unit-tested, rendered and armable from the CLI, and a later
/// measurement round can arm it without a code change. `None` renders as
/// `ceiling=disarmed` on the report line and as an EXPLICIT `null` in the JSON
/// report, never as an omitted key: omitting it would make "L2 disarmed"
/// indistinguishable from "this report predates the field", which is the
/// invisible-suppression defect this estimator exists to refuse.
pub const DEFAULT_TOMBSTONE_CORPUS_CEILING_BYTES: Option<u64> = None;

/// One durable-layer OR tombstone corpus scan, taken while the server process is
/// DEAD (redb is single-writer, so its file lock must be free) and therefore
/// reading the PRE-RECOVERY on-disk state.
#[derive(Debug, Clone, Copy)]
pub struct CorpusSample {
    pub elapsed_secs: f64,
    pub bytes: u64,
}

/// Which clause actually decided a corpus assessment. Rendered verbatim, so a
/// clause that did not fire is visible rather than indistinguishable from a pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CorpusLevelDisposition {
    /// L1 was evaluated and decided.
    LevelEvaluated,
    /// L1 was NOT evaluated. The slope clause hard-gates regardless. Never "ok".
    LevelSuppressed,
    /// L0 failed. L1 and L2 are NOT EVALUATED (fail-closed order).
    InstrumentFailed,
}

impl CorpusLevelDisposition {
    /// The single rendered token for this disposition. Consumed BOTH by the
    /// console line in `main.rs` and by the JSON serializer in `report.rs`, so
    /// the two transports can never disagree and no site retypes a literal.
    /// Deliberately hand-written rather than serde-derived: this file is
    /// `#[path]`-included by an integration target and must stay `std`-only.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LevelEvaluated => "LEVEL_EVALUATED",
            Self::LevelSuppressed => "LEVEL_SUPPRESSED",
            Self::InstrumentFailed => "INSTRUMENT_FAILED",
        }
    }
}

/// Verdict of a durable-layer corpus assessment.
#[derive(Debug, Clone)]
pub struct TombstoneCorpusAssessment {
    pub scans_attempted: usize,
    pub scans_failed: usize,
    pub samples: usize,
    /// PRE-REGISTERED AGGREGATES of the corpus series. These four — count
    /// (`samples`), min, max (`peak_bytes`) and last — are the ONLY shape in
    /// which the series reaches a consumer: publishing the per-sample series is
    /// forbidden, so every predicate any downstream grading rests on must be
    /// statable over exactly these.
    pub first_bytes: u64,
    pub min_bytes: u64,
    pub peak_bytes: u64,
    pub last_bytes: u64,
    pub first_half_peak_bytes: u64,
    pub last_half_peak_bytes: u64,
    pub rise_bytes: u64,
    pub span_secs: f64,
    pub disposition: CorpusLevelDisposition,
    /// `None` = L2 DISARMED. Serialized EXPLICITLY as `null` by the report
    /// layer — absence must never be indistinguishable from suppression.
    pub ceiling_bytes: Option<u64>,
    pub passed: bool,
    pub reason: Option<String>,
}

/// L0 and L1 are REPORT-ONLY, not HARD gate clauses: a control cell with no
/// fault injected breached L1 on its own, so `main.rs` renders and persists
/// this verdict without folding it into the run's.
///
/// The three clauses are evaluated in this order, and the order is fail-closed:
///
/// * `L0` — INSTRUMENT. Breaches when `scans_failed > 0` OR `samples == 0`,
///   yielding `passed = false`, [`CorpusLevelDisposition::InstrumentFailed`],
///   a named reason, and NO evaluation of L1 or L2. A scan that could not
///   obtain the state — missing file, corrupt header, a table-open error, or a
///   failed byte copy — is a BLIND instrument, and a gate whose instrument was
///   blind must not report bounded growth. An honest `Some(0)` from a
///   never-written table is not a breach and must stay distinguishable from a
///   blind `None`.
/// * `L1` — LEVEL / FLATNESS. Evaluated only when `samples >= min_samples` AND
///   `span_secs >= min_span_secs`; breaches when
///   `last_half_peak_bytes.saturating_sub(first_half_peak_bytes) >
///   headroom_bytes`, yielding [`CorpusLevelDisposition::LevelEvaluated`]. The
///   `saturating_sub` is deliberate: a FALLING corpus yields `rise = 0` and
///   passes, which is the correct answer for a ceiling test. When either guard
///   is unmet the disposition is
///   [`CorpusLevelDisposition::LevelSuppressed`], this clause contributes
///   nothing to `passed`, and the suppression is rendered with BOTH numbers so
///   it can never be read as a pass.
/// * `L2` — ABSOLUTE CEILING. Evaluated only when `ceiling_bytes` is `Some(c)`;
///   breaches when `peak_bytes > c`. It is an ENVELOPE test — the peak, not the
///   last sample — because prune legitimately produces large downward
///   excursions and a level-vs-final test would let a rising envelope hide
///   behind one of them. Disarmed by default; see
///   [`DEFAULT_TOMBSTONE_CORPUS_CEILING_BYTES`].
///
/// The peak is also why neither the mean nor the median is used anywhere here:
/// a ceiling is an UPPER ENVELOPE, and those same downward excursions would
/// drag a mean or a median and hide a rising envelope. Nothing in this clause
/// is fitted; it compares bytes to bytes, so no `1/span` amplification can
/// re-import a rate spread as if it were an independent noise source.
///
/// The window partition is the same split index [`last_half_window`] uses, so
/// this clause and the slope clause split any series identically and a reader
/// comparing the two is comparing like with like.
///
/// Total over its inputs: no panic path, no interior mutability, no I/O, and no
/// dependency beyond `std` — this file is `#[path]`-included by an integration
/// target that includes no sibling module.
#[must_use]
pub fn assess_tombstone_corpus_level(
    samples: &[CorpusSample],
    scans_attempted: usize,
    scans_failed: usize,
    headroom_bytes: u64,
    min_span_secs: f64,
    min_samples: usize,
    ceiling_bytes: Option<u64>,
) -> TombstoneCorpusAssessment {
    // The series aggregates are DESCRIPTIVE: deriving them is not evaluating a
    // clause, so they are filled in on every path — including the blind one —
    // and the rendered line stays informative even when no clause decided.
    // They are also the ONLY shape in which the series reaches a consumer, so
    // every downstream predicate has to be statable over exactly these.
    let span_secs = match (samples.first(), samples.last()) {
        (Some(first), Some(last)) => last.elapsed_secs - first.elapsed_secs,
        _ => 0.0,
    };
    let first_bytes = samples.first().map_or(0, |s| s.bytes);
    let last_bytes = samples.last().map_or(0, |s| s.bytes);
    let min_bytes = samples.iter().map(|s| s.bytes).min().unwrap_or(0);
    let peak_bytes = samples.iter().map(|s| s.bytes).max().unwrap_or(0);
    // The same split index the slope clause partitions on, applied to counted
    // byte totals directly: no total is ever routed through an `f64`.
    let (first_half, last_half) = samples.split_at(last_half_split_index(samples.len()));
    let first_half_peak_bytes = first_half.iter().map(|s| s.bytes).max().unwrap_or(0);
    let last_half_peak_bytes = last_half.iter().map(|s| s.bytes).max().unwrap_or(0);
    // Saturating: a FALLING corpus yields a rise of zero and passes, which is
    // the correct answer for a ceiling test.
    let rise_bytes = last_half_peak_bytes.saturating_sub(first_half_peak_bytes);

    // L0 — INSTRUMENT, evaluated FIRST and fail-closed. A blind instrument
    // returns here, so neither L1 nor L2 is evaluated at all: a gate whose
    // instrument could not obtain the state must not report bounded growth.
    if scans_failed > 0 || samples.is_empty() {
        return TombstoneCorpusAssessment {
            scans_attempted,
            scans_failed,
            samples: samples.len(),
            first_bytes,
            min_bytes,
            peak_bytes,
            last_bytes,
            first_half_peak_bytes,
            last_half_peak_bytes,
            rise_bytes,
            span_secs,
            disposition: CorpusLevelDisposition::InstrumentFailed,
            ceiling_bytes,
            passed: false,
            reason: Some(format!(
                "durable-corpus instrument blind: {} of {} scans failed, {} usable samples",
                scans_failed,
                scans_attempted,
                samples.len()
            )),
        };
    }

    let mut breaches: Vec<String> = Vec::new();

    // L1 — LEVEL / FLATNESS. Decided only when BOTH guards are met; otherwise
    // it contributes nothing to the verdict and the suppression is rendered
    // with its two numbers rather than as a pass.
    let level_evaluated = samples.len() >= min_samples && span_secs >= min_span_secs;
    if level_evaluated && rise_bytes > headroom_bytes {
        breaches.push(format!(
            "durable-corpus level rose {rise_bytes} B between half-peaks \
             ({first_half_peak_bytes} B -> {last_half_peak_bytes} B) over \
             {span_secs:.0}s, above the {headroom_bytes} B headroom"
        ));
    }

    // L2 — ABSOLUTE CEILING, an ENVELOPE test on the PEAK. Its only guard is
    // being armed, so a series too short for L1 is still held to a ceiling.
    if let Some(ceiling) = ceiling_bytes {
        if peak_bytes > ceiling {
            breaches.push(format!(
                "durable-corpus peak {peak_bytes} B exceeds the armed {ceiling} B ceiling"
            ));
        }
    }

    TombstoneCorpusAssessment {
        scans_attempted,
        scans_failed,
        samples: samples.len(),
        first_bytes,
        min_bytes,
        peak_bytes,
        last_bytes,
        first_half_peak_bytes,
        last_half_peak_bytes,
        rise_bytes,
        span_secs,
        disposition: if level_evaluated {
            CorpusLevelDisposition::LevelEvaluated
        } else {
            CorpusLevelDisposition::LevelSuppressed
        },
        ceiling_bytes,
        passed: breaches.is_empty(),
        reason: if breaches.is_empty() {
            None
        } else {
            Some(breaches.join("; "))
        },
    }
}

/// Whether the tombstone-byte SLOPE clause would be the only hard-gate left on
/// a run that produced this disposition — equivalently, whether the
/// durable-corpus level clause decided. EXHAUSTIVE BY CONSTRUCTION: a fourth
/// variant does not compile here, which is what makes the no-ungated-window
/// coverage argument structural rather than a source-read. The slope now
/// decides every run class, so the verdict expression does not consult this;
/// `main.rs` uses it to name which corpus clause a report-only breach came
/// from, and re-types no comparison of its own.
#[must_use]
// One arm per variant, deliberately not merged into a single `|` pattern: the
// point of the enumeration is that every variant is classified in its own
// right, so a fourth variant has to be given an explicit answer here rather
// than being absorbed into an existing pattern.
#[allow(clippy::match_same_arms)]
pub const fn slope_clause_stays_hard(disposition: CorpusLevelDisposition) -> bool {
    match disposition {
        CorpusLevelDisposition::LevelEvaluated => false,
        CorpusLevelDisposition::LevelSuppressed => true,
        CorpusLevelDisposition::InstrumentFailed => true,
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

// ---------------------------------------------------------------------------
// Durable-layer instrument: census, series shapes and the origin reading.
//
// Declarations only at this wave. Every item below carries its own item-scoped
// dead-code allow because `main.rs` declares `mod monitor;` with no
// module-level one, so an unwired declaration would fail `-D warnings` at the
// wave boundary. Each allow expires when the item is wired in G4, and a blanket
// module-level allow is deliberately NOT used: it would also silence the lint
// for this module's existing items, permanently.
// ---------------------------------------------------------------------------

/// A store-level live/dead census of the durable OR corpus.
///
/// Every field is answerable **by the durable store alone**: the scan decodes
/// each row and folds counters, and joins nothing against server-side state.
///
/// # Caller contract — what this census may and may not be read as
///
/// The durable tombstone blob is a bare tag list with **no epoch attribution**
/// (the epoch association is server-side metadata that is not persisted), so
/// *"dead = below the reclamation ceiling"* is **NOT computable** from durable
/// state and this census does not claim it. Migrating durable blobs to an
/// epoch-indexed form is TODO-566's obligation and has not landed; until it
/// does, a caller that wants an epoch-attributed reading must get it from the
/// in-memory index, and such a join is an OBSERVATION that may never enter a
/// predicate — importing it would import the very instrument under suspicion.
///
/// All thirteen fields are counts or byte totals and therefore `u64`;
/// `Default` zero-initialises every one so a fold starts from an empty census.
#[allow(dead_code)] // wired in G4
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DurableCensus {
    /// Rows iterated.
    pub keys_scanned: u64,
    /// Rows whose record value failed to decode. Counted rather than skipped
    /// silently, so an undecodable row is visible instead of invisible.
    pub keys_undecodable: u64,
    /// Keys decoded as the unified OR-Map variant.
    pub or_map_keys: u64,
    /// Keys decoded as the legacy, read-only tombstones-only variant.
    pub or_tombstones_keys: u64,
    /// Keys decoded as the LWW variant.
    pub lww_keys: u64,
    /// Σ live record count across scanned keys.
    pub live_entries: u64,
    /// Σ live tag length — the SAME unit as [`DurableCensus::tombstone_bytes`],
    /// so live and dead are comparable without conversion.
    pub live_tag_bytes: u64,
    /// Σ tombstone tags across BOTH OR variants.
    pub tombstone_entries: u64,
    /// Σ tombstone tag length across BOTH OR variants.
    pub tombstone_bytes: u64,
    /// Σ per-key (`len` − distinct). Duplicates are counted **within a key**,
    /// never across keys, so no cross-key set is ever built.
    pub tombstone_dup_entries: u64,
    /// Keys with a non-empty tombstone vector.
    pub keys_with_tombstones: u64,
    /// Keys whose live records are empty **and** whose tombstones are not.
    pub keys_all_dead: u64,
    /// The per-key maximum tombstone count.
    pub max_tombstones_per_key: u64,
}

/// Which OR record variant a scanned key decoded as.
///
/// The two variants share every tombstone fold — a tombstone is a tombstone in
/// either — and differ only in which variant-share counter they bump, which is
/// why the fold takes this rather than duplicating the arithmetic per variant.
#[allow(dead_code)] // wired in G4
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrVariant {
    /// The unified OR-Map variant: live records plus tombstones.
    OrMap,
    /// The legacy, read-only tombstones-only variant.
    OrTombstones,
}

/// Where a census record was taken, and therefore what it may decide.
///
/// The observation fence is a TYPED property of the record, not a convention:
/// [`CensusSource::LiveCopy`] records are taken from a byte copy of a **live**
/// store file, which is a smeared image, so they are best-effort by
/// construction and may not enter any predicate. They land in their own tally
/// and are never added to the corpus-scan tally, which is what keeps the
/// durable-corpus estimator's input identical whether the live sampler is armed
/// or not.
#[allow(dead_code)] // wired in G4
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CensusSource {
    /// Taken at a recovery checkpoint, with the server process DEAD.
    Checkpoint,
    /// Taken after the run, with the server process DEAD.
    Terminal,
    /// Taken from a byte copy of the LIVE store file. OBSERVATION ONLY.
    LiveCopy,
}

impl CensusSource {
    /// The single rendered token for this source. Consumed BOTH by the console
    /// renderer and by the JSON serializer, so the two transports can never
    /// disagree and no site retypes a literal. Deliberately hand-written rather
    /// than serde-derived: this file is `#[path]`-included by two integration
    /// targets and must stay `std`-only.
    #[allow(dead_code)] // wired in G4
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Checkpoint => "CHECKPOINT",
            Self::Terminal => "TERMINAL",
            Self::LiveCopy => "LIVE_COPY",
        }
    }
}

/// One census, tagged with when it was taken and from what.
#[allow(dead_code)] // wired in G4
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CensusRecord {
    pub elapsed_secs: f64,
    pub source: CensusSource,
    pub census: DurableCensus,
}

/// One sample of a deciding series.
///
/// The value is `u64` for every deciding series — all four are counts or byte
/// totals — so the shape rule's strict comparisons are exact and no series
/// becomes an `f64` on the way through.
#[allow(dead_code)] // wired in G4
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SeriesPoint {
    pub elapsed_secs: f64,
    pub value: u64,
}

/// The shape of one deciding series under the frozen, threshold-free rule.
///
/// Evaluated in declaration order, fail-closed first. The rule reads
/// **envelopes, not rates**: peaks and troughs over halves and quarters, with
/// strict `>` comparisons and no headroom constant anywhere.
#[allow(dead_code)] // wired in G4
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeriesShape {
    /// Too few retained samples, too short a retained span, or no samples at
    /// all. Carries a named reason. Never "ok".
    Indeterminate,
    /// An envelope — the peak envelope or the trough floor — rose across the
    /// run **and was still rising at the end**.
    MonotoneRising,
    /// Something rose across the run but was no longer rising at the end.
    RisingDecelerating,
    /// Neither the peak envelope nor the trough floor rose across the run.
    Levelled,
}

impl SeriesShape {
    /// The single rendered token for this shape. Consumed BOTH by the console
    /// renderer and by the JSON serializer, so the two transports can never
    /// disagree and no site retypes a literal.
    #[allow(dead_code)] // wired in G4
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Indeterminate => "INDETERMINATE",
            Self::MonotoneRising => "MONOTONE_RISING",
            Self::RisingDecelerating => "RISING_DECELERATING",
            Self::Levelled => "LEVELLED",
        }
    }
}

impl Default for SeriesShape {
    /// Fail-closed: an unset shape is [`SeriesShape::Indeterminate`], never a
    /// levelled one. A `Default` that read as "ok" would let a partially built
    /// reading pass for a measured one.
    fn default() -> Self {
        Self::Indeterminate
    }
}

/// The classification of one deciding series, with every number the verdict
/// was computed from.
///
/// The per-shape numbers are carried on the reading rather than recomputed
/// downstream so that the rendered row and the verdict provably come from the
/// same arithmetic over the same retained series.
#[allow(dead_code)] // wired in G4
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SeriesShapeReading {
    /// Which deciding series this is — one of [`DECIDING_SERIES`]. Carried as a
    /// typed field so the reading over all four can NAME the offending series
    /// instead of inferring it from position.
    pub name: &'static str,
    pub shape: SeriesShape,
    /// WHICH envelope fired, so the culprit is named and never inferred.
    /// Exactly one of `"PEAKS"`, `"FLOOR"` or `"BOTH"`, and `None` when no
    /// envelope fired. Meaningful only under [`SeriesShape::MonotoneRising`],
    /// which `shape` already states, so its absence carries no disposition.
    pub firing_envelope: Option<&'static str>,
    /// Retained (post-warmup-exclusion) sample count.
    pub samples: u64,
    /// Retained span in seconds.
    pub span_secs: f64,
    pub first_half_peak: u64,
    pub last_half_peak: u64,
    pub third_quarter_peak: u64,
    pub last_quarter_peak: u64,
    pub first_half_trough: u64,
    pub last_half_trough: u64,
    pub third_quarter_trough: u64,
    pub last_quarter_trough: u64,
    /// Arithmetic mean of the retained series' LAST HALF, over the same split
    /// index the peaks and troughs use and from the SAME series they come from,
    /// so a peak and a mean are never read off two different cadences.
    /// Integer FLOOR of the mean: every deciding series is integer-valued and
    /// every integer-semantic field here is `u64`.
    pub last_half_mean: u64,
    /// Why the shape is what it is when the guards decided it. Disposition
    /// bearing: it always serializes, as an explicit null where it has no
    /// value, so "no reason" is never confused with "this artifact predates the
    /// field".
    pub reason: Option<String>,
}

/// The four deciding series, frozen.
///
/// All four are durable-layer and **externally observable** — each is read by
/// the harness from `ps` or filesystem metadata, none is the measured process's
/// own self-report. Write-behind occupancy is deliberately NOT among them: it
/// is the only candidate read from the server's own metrics endpoint, its
/// durable consequence is already carried by the two WAL series, and under a
/// peak rule a single late stall would decide the whole run. It is recorded,
/// rendered and serialized as an observation column instead.
#[allow(dead_code)] // wired in G4
pub const DECIDING_SERIES: [&str; 4] = ["rss_kib", "redb_bytes", "wal_bytes", "wal_segment_files"];

/// The reading over all four deciding series, evaluated in declaration order.
#[allow(dead_code)] // wired in G4
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurableReading {
    /// ANY deciding series is [`SeriesShape::Indeterminate`]. Fail-closed, with
    /// the offending series named in the reason.
    IndeterminateInstrument,
    /// ANY deciding series is [`SeriesShape::MonotoneRising`].
    PlateauNotMet,
    /// Every deciding series is levelled or rising-decelerating.
    ///
    /// Deliberately weak, and the strongest reading this instrument may emit: a
    /// non-rising verdict over a bounded horizon means *this horizon did not
    /// show it*, never *there is nothing to show*.
    NoRisingEnvelopeObserved,
}

impl DurableReading {
    /// The single rendered token for this reading. Consumed BOTH by the console
    /// renderer and by the JSON serializer, so the two transports can never
    /// disagree and no site retypes a literal.
    #[allow(dead_code)] // wired in G4
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::IndeterminateInstrument => "INDETERMINATE_INSTRUMENT",
            Self::PlateauNotMet => "PLATEAU_NOT_MET",
            Self::NoRisingEnvelopeObserved => "NO_RISING_ENVELOPE_OBSERVED",
        }
    }
}

/// One parsed removal-site observation line.
///
/// The eight fields are extracted from the RENDERED line by whitespace-token
/// scan rather than by format position, so the parser is not coupled to the log
/// formatter's field or message order. A line yields a value only when all
/// eight parse.
#[allow(dead_code)] // wired in G4
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OriginLine {
    /// Wall-clock milliseconds since the epoch, as the emitter wrote it.
    pub ts: i64,
    pub op_seq: u64,
    pub epoch: u64,
    /// References the index removal itself returned.
    pub refs_returned: u64,
    /// References the slot held when the epoch entered the index.
    pub refs_at_entry: u64,
    pub bytes_returned: u64,
    pub watermark: u64,
    pub ceiling: u64,
}

/// Everything the origin classifier reads, as TYPED fields.
///
/// No classifier input is inferred: the restart count, the armed flag and the
/// exited-epoch count are threaded in by their producers and read from here, so
/// a reading can never rest on a quantity nobody produced.
#[allow(dead_code)] // wired in G4
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OriginAggregate {
    /// Lines the capture matched on the origin target.
    pub matched: u64,
    /// Matched lines that did NOT yield all eight fields. Never silent: any
    /// unparsed line forces the fail-closed reading.
    pub unparsed: u64,
    /// Matched lines dropped because the capture was full. Never silent: any
    /// drop forces the fail-closed reading.
    pub dropped: u64,
    /// The successfully parsed lines.
    pub lines: Vec<OriginLine>,
    /// Server restarts during the run, threaded in from the recovery-checkpoint
    /// counter and from NOTHING else — the `epochs_exited` qualifier resets on
    /// restart and would be misread.
    pub restarts: u64,
    /// Whether the log filter actually selected the origin target for this run.
    pub armed: bool,
    /// The exited-epoch qualifier, named BY PARAMETER here and never by metric
    /// literal: the literal belongs where the scrape that reads it lives. Used
    /// ONLY as the origin reading's qualifier and never as a plateau predicate
    /// input.
    pub epochs_exited: u64,
    /// The named reason for the classified reading, carried here so the
    /// serializer emits it beside the counters it was derived from. Distinct
    /// from the durable reading's own reason. Disposition-bearing: it always
    /// serializes, as an explicit null where it has no value.
    pub reason: Option<String>,
}

/// The origin reading, evaluated in declaration order — fail-closed first.
///
/// Every variant ROUTES; none is diagnosed here. The diagnosis line is
/// hard-stopped, and this instrument ships regardless of which way it reads.
#[allow(dead_code)] // wired in G4
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OriginReading {
    /// The filter was not armed, OR a line was dropped, OR a line was
    /// unparsed, OR the server restarted during the run.
    IndeterminateInstrument,
    /// At least one line had `refs_returned == 0 && refs_at_entry > 0`: the
    /// empty-return state is reached in production, and the origin question
    /// becomes a code-level fix spec.
    ReachedInProduction,
    /// At least one line had `refs_returned != refs_at_entry`, none of them of
    /// the reached-in-production shape. A fourth physical state the two
    /// pre-registered readings do not cover; naming it is what stops it being
    /// folded silently into the equal-refs reading.
    PartialDivergence,
    /// At least one line, and EVERY line had `refs_returned == refs_at_entry`:
    /// not reached under this load.
    NotReachedEqualRefs,
    /// Zero lines, filter armed, and epochs did exit: the drain arm is not
    /// reached — a dark path or a gate.
    NoLinesWhileEpochsExited,
    /// Zero lines, filter armed, and no epoch exited: nothing exited, so the
    /// arm could not have been reached.
    NotObservedAtHead,
}

impl OriginReading {
    /// The single rendered token for this reading. Consumed BOTH by the console
    /// renderer and by the JSON serializer, so the two transports can never
    /// disagree and no site retypes a literal.
    #[allow(dead_code)] // wired in G4
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::IndeterminateInstrument => "INDETERMINATE_INSTRUMENT",
            Self::ReachedInProduction => "REACHED_IN_PRODUCTION",
            Self::PartialDivergence => "PARTIAL_DIVERGENCE",
            Self::NotReachedEqualRefs => "NOT_REACHED_EQUAL_REFS",
            Self::NoLinesWhileEpochsExited => "NO_LINES_WHILE_EPOCHS_EXITED",
            Self::NotObservedAtHead => "NOT_OBSERVED_AT_HEAD",
        }
    }
}

/// One sample of WAL retention: the two deciding series that share a sampler.
///
/// Both are read from filesystem metadata only — no segment file is ever
/// opened — so sampling them cannot perturb the process being measured.
#[allow(dead_code)] // wired in G4
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WalRetention {
    /// Σ length of the retained segment files, in bytes.
    pub bytes: u64,
    /// Number of retained segment files.
    pub segment_files: u64,
}

/// Fold ONE scanned OR key into `census`.
///
/// All census arithmetic lives here rather than at the scan site so that it is
/// unit-testable in isolation and the scan only decodes and dispatches. The two
/// OR variants share every tombstone clause — a tombstone is a tombstone in
/// either — and differ only in which variant-share counter they bump, which is
/// what `variant` selects.
///
/// `live_tags` carries the live records' tags (empty for the legacy
/// tombstones-only variant) and `tombstone_tags` the key's tombstone vector, in
/// the order the store returned them. Duplicates are counted **within this key
/// only**: the distinct count is taken over this key's own vector and no
/// cross-key set is ever built, so the fold's memory is bounded by the widest
/// single key rather than by the corpus.
#[allow(dead_code)] // wired in G4
pub fn fold_or_key(
    census: &mut DurableCensus,
    variant: OrVariant,
    live_tags: &[&str],
    tombstone_tags: &[&str],
) {
    census.keys_scanned = census.keys_scanned.saturating_add(1);
    match variant {
        OrVariant::OrMap => census.or_map_keys = census.or_map_keys.saturating_add(1),
        OrVariant::OrTombstones => {
            census.or_tombstones_keys = census.or_tombstones_keys.saturating_add(1);
        }
    }

    census.live_entries = census
        .live_entries
        .saturating_add(len_as_u64(live_tags.len()));
    for tag in live_tags {
        census.live_tag_bytes = census.live_tag_bytes.saturating_add(len_as_u64(tag.len()));
    }

    let tombstones = len_as_u64(tombstone_tags.len());
    census.tombstone_entries = census.tombstone_entries.saturating_add(tombstones);
    for tag in tombstone_tags {
        census.tombstone_bytes = census.tombstone_bytes.saturating_add(len_as_u64(tag.len()));
    }

    if !tombstone_tags.is_empty() {
        census.keys_with_tombstones = census.keys_with_tombstones.saturating_add(1);
        if live_tags.is_empty() {
            census.keys_all_dead = census.keys_all_dead.saturating_add(1);
        }
    }

    let distinct = distinct_count_within_key(tombstone_tags);
    census.tombstone_dup_entries = census
        .tombstone_dup_entries
        .saturating_add(tombstones.saturating_sub(distinct));

    census.max_tombstones_per_key = census.max_tombstones_per_key.max(tombstones);
}

/// Fold ONE scanned LWW key into `census`.
///
/// An LWW row carries neither an OR entry list nor a tombstone vector, so it
/// contributes to the variant share and to the scanned-row count and to nothing
/// else. Folding it through its own entry point — rather than letting the scan
/// site skip it — is what keeps `keys_scanned` equal to the rows actually
/// iterated.
#[allow(dead_code)] // wired in G4
pub fn fold_lww_key(census: &mut DurableCensus) {
    census.keys_scanned = census.keys_scanned.saturating_add(1);
    census.lww_keys = census.lww_keys.saturating_add(1);
}

/// Fold ONE row whose record value failed to decode.
///
/// The row is COUNTED rather than silently skipped: an undecodable row is a
/// visible gap in the census, and a census that hid it would report a smaller
/// corpus than the store actually holds without saying so.
#[allow(dead_code)] // wired in G4
pub fn fold_undecodable_key(census: &mut DurableCensus) {
    census.keys_scanned = census.keys_scanned.saturating_add(1);
    census.keys_undecodable = census.keys_undecodable.saturating_add(1);
}

/// Distinct tag count **within one key's own tombstone vector**.
///
/// Quadratic in the key's tombstone count on purpose: it needs no allocation
/// and no hashing, and the alternative — a set — is the cross-key structure the
/// census contract forbids. The per-key vectors this runs over are small
/// relative to the corpus, so the corpus-scale cost stays linear in rows.
fn distinct_count_within_key(tags: &[&str]) -> u64 {
    let mut distinct: u64 = 0;
    for (index, tag) in tags.iter().enumerate() {
        if !tags[..index].contains(tag) {
            distinct = distinct.saturating_add(1);
        }
    }
    distinct
}

/// Widen a length to the census's `u64` field type without a lossy cast.
const fn len_as_u64(len: usize) -> u64 {
    len as u64
}

/// The index at which a series splits into its third and fourth quarters.
///
/// Derived by applying [`last_half_split_index`] TWICE — once to the whole
/// series and once to what remains after the first half — so halves and
/// quarters partition identically to every other consumer of the split and a
/// reader comparing two of them is comparing like with like.
const fn quarter_split_index(len: usize) -> usize {
    let half = last_half_split_index(len);
    half + last_half_split_index(len - half)
}

/// The index of the first sample RETAINED after the frozen warmup exclusion:
/// the first 1/16 of the series' RAW span is dropped.
///
/// A process's warm-up peak lands in the first half and can make a genuinely
/// late-rising series read as levelled by inflating the first-half peak. The
/// fraction is frozen pre-data at a coarse binary fraction, chosen for being
/// one rather than for anything it does to a number.
///
/// This is a SEPARATE mechanism from [`exclude_boot_gap_samples`], which is
/// restart-oriented; neither may stand in for the other.
fn warmup_exclusion_index(points: &[SeriesPoint]) -> usize {
    let (Some(first), Some(last)) = (points.first(), points.last()) else {
        return 0;
    };
    let span = last.elapsed_secs - first.elapsed_secs;
    if !span.is_finite() || span <= 0.0 {
        return 0;
    }
    let cutoff = first.elapsed_secs + span / 16.0;
    points
        .iter()
        .position(|p| p.elapsed_secs >= cutoff)
        .unwrap_or(points.len())
}

/// Classify one deciding series under the frozen, threshold-free shape rule.
///
/// The rule reads ENVELOPES, not rates: peaks and troughs over halves and
/// quarters, strict `>` throughout, and no headroom constant anywhere. The
/// trough clauses are the exact mirror of the peak clauses and exist because a
/// peaks-only rule is blind to a saw whose FLOOR climbs while its peaks stand
/// still — which is a leak's signature.
///
/// `min_samples` and `min_span_secs` are GUARDS, not thresholds: they decide
/// whether the clauses are evaluated, never which side they fall on, and they
/// check the retained sample count and the retained span ONLY — never the
/// number of distinct values. A perfectly constant series is therefore a
/// healthy levelled one, not an instrument failure.
#[allow(dead_code)] // wired in G4
#[must_use]
pub fn classify_series_shape(
    name: &'static str,
    points: &[SeriesPoint],
    min_samples: usize,
    min_span_secs: f64,
) -> SeriesShapeReading {
    if points.is_empty() {
        return SeriesShapeReading {
            name,
            shape: SeriesShape::Indeterminate,
            reason: Some("the sampler recorded zero samples".to_string()),
            ..SeriesShapeReading::default()
        };
    }

    let retained = &points[warmup_exclusion_index(points)..];
    let samples = len_as_u64(retained.len());
    let span_secs = match (retained.first(), retained.last()) {
        (Some(first), Some(last)) => last.elapsed_secs - first.elapsed_secs,
        _ => 0.0,
    };

    if retained.len() < min_samples {
        return SeriesShapeReading {
            name,
            shape: SeriesShape::Indeterminate,
            samples,
            span_secs,
            reason: Some(format!(
                "retained samples {} below the minimum {min_samples}",
                retained.len()
            )),
            ..SeriesShapeReading::default()
        };
    }
    if span_secs < min_span_secs {
        return SeriesShapeReading {
            name,
            shape: SeriesShape::Indeterminate,
            samples,
            span_secs,
            reason: Some(format!(
                "retained span {span_secs:.1}s below the minimum {min_span_secs:.1}s"
            )),
            ..SeriesShapeReading::default()
        };
    }

    let half = last_half_split_index(retained.len());
    let quarter = quarter_split_index(retained.len());

    let first_half_peak = window_peak(&retained[..half]);
    let last_half_peak = window_peak(&retained[half..]);
    let third_quarter_peak = window_peak(&retained[half..quarter]);
    let last_quarter_peak = window_peak(&retained[quarter..]);
    let first_half_trough = window_trough(&retained[..half]);
    let last_half_trough = window_trough(&retained[half..]);
    let third_quarter_trough = window_trough(&retained[half..quarter]);
    let last_quarter_trough = window_trough(&retained[quarter..]);

    let rising_peaks_full = last_half_peak > first_half_peak;
    let rising_peaks_tail = last_quarter_peak > third_quarter_peak;
    let rising_floor_full = last_half_trough > first_half_trough;
    let rising_floor_tail = last_quarter_trough > third_quarter_trough;

    let peaks_fired = rising_peaks_full && rising_peaks_tail;
    let floor_fired = rising_floor_full && rising_floor_tail;

    let (shape, firing_envelope) = match (peaks_fired, floor_fired) {
        (true, true) => (SeriesShape::MonotoneRising, Some(ENVELOPE_BOTH)),
        (true, false) => (SeriesShape::MonotoneRising, Some(ENVELOPE_PEAKS)),
        (false, true) => (SeriesShape::MonotoneRising, Some(ENVELOPE_FLOOR)),
        (false, false) if rising_peaks_full || rising_floor_full => {
            (SeriesShape::RisingDecelerating, None)
        }
        (false, false) => (SeriesShape::Levelled, None),
    };

    SeriesShapeReading {
        name,
        shape,
        firing_envelope,
        samples,
        span_secs,
        first_half_peak,
        last_half_peak,
        third_quarter_peak,
        last_quarter_peak,
        first_half_trough,
        last_half_trough,
        third_quarter_trough,
        last_quarter_trough,
        last_half_mean: window_mean_floor(&retained[half..]),
        reason: None,
    }
}

/// The three tokens a fired envelope may be named by. They are the ONLY values
/// [`SeriesShapeReading::firing_envelope`] ever carries, and they are consumed
/// verbatim by both transports so neither retypes a literal.
const ENVELOPE_PEAKS: &str = "PEAKS";
const ENVELOPE_FLOOR: &str = "FLOOR";
const ENVELOPE_BOTH: &str = "BOTH";

fn window_peak(window: &[SeriesPoint]) -> u64 {
    window.iter().map(|p| p.value).max().unwrap_or(0)
}

fn window_trough(window: &[SeriesPoint]) -> u64 {
    window.iter().map(|p| p.value).min().unwrap_or(0)
}

/// Integer FLOOR of the arithmetic mean over `window`.
///
/// Every deciding series is integer-valued and every integer-semantic field on
/// the reading is `u64`, so the mean is floored rather than routed through an
/// `f64`. It is computed over the SAME retained series, on the SAME split, that
/// the peaks and troughs come from, so a peak and a mean are never read off two
/// different cadences.
fn window_mean_floor(window: &[SeriesPoint]) -> u64 {
    let count = len_as_u64(window.len());
    if count == 0 {
        return 0;
    }
    let sum = window
        .iter()
        .fold(0u64, |acc, p| acc.saturating_add(p.value));
    sum / count
}

/// The reading over the four deciding series, evaluated fail-closed first.
///
/// Returns the reading together with its NAMED reason, so the offending series
/// — and, when an envelope fired, which envelope it was — is always named and
/// never inferred from position.
///
/// The name set is checked FIRST: a caller that hands over a set which is not
/// exactly the frozen deciding series gets the fail-closed reading, so a fifth
/// series cannot be smuggled into a verdict through the classifier.
#[allow(dead_code)] // wired in G4
#[must_use]
pub fn classify_durable_reading(
    readings: &[SeriesShapeReading],
) -> (DurableReading, Option<String>) {
    if !names_are_exactly_deciding(readings) {
        let observed: Vec<&str> = readings.iter().map(|r| r.name).collect();
        return (
            DurableReading::IndeterminateInstrument,
            Some(format!(
                "reading set {observed:?} is not exactly the frozen deciding series {DECIDING_SERIES:?}"
            )),
        );
    }

    if let Some(offender) = readings
        .iter()
        .find(|r| r.shape == SeriesShape::Indeterminate)
    {
        let why = offender.reason.as_deref().unwrap_or("no reason recorded");
        return (
            DurableReading::IndeterminateInstrument,
            Some(format!("series {} is indeterminate: {why}", offender.name)),
        );
    }

    if let Some(offender) = readings
        .iter()
        .find(|r| r.shape == SeriesShape::MonotoneRising)
    {
        let envelope = offender.firing_envelope.unwrap_or("UNNAMED");
        return (
            DurableReading::PlateauNotMet,
            Some(format!(
                "series {} rose and was still rising at the end; firing envelope {envelope}",
                offender.name
            )),
        );
    }

    (
        DurableReading::NoRisingEnvelopeObserved,
        Some(
            "no deciding series showed a rising envelope over this horizon; \
             this horizon did not show it, which is not the same as there being nothing to show"
                .to_string(),
        ),
    )
}

/// Whether `readings` names exactly the frozen deciding series — same count,
/// every name present, none repeated. Order is not constrained, because the
/// reading ORs over the set and no clause reads a position.
fn names_are_exactly_deciding(readings: &[SeriesShapeReading]) -> bool {
    readings.len() == DECIDING_SERIES.len()
        && DECIDING_SERIES
            .iter()
            .all(|expected| readings.iter().filter(|r| r.name == *expected).count() == 1)
}

/// Parse one RENDERED removal-site observation line into its eight fields.
///
/// The scan is over whitespace-separated `key=value` tokens rather than over
/// format positions, so the parser is not coupled to the log formatter's field
/// or message order. Returns `None` unless ALL eight fields are present and
/// parse: a half-read line is never turned into a partially-populated one.
/// A malformed value for a recognised key also yields `None` — fail-closed,
/// because a line that was emitted but could not be read is exactly what the
/// aggregate's `unparsed` counter exists to make visible.
#[allow(dead_code)] // wired in G4
#[must_use]
pub fn parse_origin_line(line: &str) -> Option<OriginLine> {
    let mut ts: Option<i64> = None;
    let mut op_seq: Option<u64> = None;
    let mut epoch: Option<u64> = None;
    let mut refs_returned: Option<u64> = None;
    let mut refs_at_entry: Option<u64> = None;
    let mut bytes_returned: Option<u64> = None;
    let mut watermark: Option<u64> = None;
    let mut ceiling: Option<u64> = None;

    for token in line.split_whitespace() {
        let Some((key, raw)) = token.split_once('=') else {
            continue;
        };
        match key {
            "ts" => set_field(&mut ts, raw.parse::<i64>().ok())?,
            "op_seq" => set_field(&mut op_seq, raw.parse::<u64>().ok())?,
            "epoch" => set_field(&mut epoch, raw.parse::<u64>().ok())?,
            "refs_returned" => set_field(&mut refs_returned, raw.parse::<u64>().ok())?,
            "refs_at_entry" => set_field(&mut refs_at_entry, raw.parse::<u64>().ok())?,
            "bytes_returned" => set_field(&mut bytes_returned, raw.parse::<u64>().ok())?,
            "watermark" => set_field(&mut watermark, raw.parse::<u64>().ok())?,
            "ceiling" => set_field(&mut ceiling, raw.parse::<u64>().ok())?,
            _ => {}
        }
    }

    Some(OriginLine {
        ts: ts?,
        op_seq: op_seq?,
        epoch: epoch?,
        refs_returned: refs_returned?,
        refs_at_entry: refs_at_entry?,
        bytes_returned: bytes_returned?,
        watermark: watermark?,
        ceiling: ceiling?,
    })
}

/// Record the FIRST occurrence of a recognised field, and fail closed on a
/// value that did not parse. Returns `None` — which the caller propagates — so
/// a malformed recognised key can never be mistaken for an absent one.
fn set_field<T>(slot: &mut Option<T>, parsed: Option<T>) -> Option<()> {
    let value = parsed?;
    if slot.is_none() {
        *slot = Some(value);
    }
    Some(())
}

/// Fold captured origin lines into the aggregate the classifier reads.
///
/// Every classifier input is a TYPED field here: `dropped` comes from the
/// capture's own overflow counter, `restarts` from the recovery-checkpoint
/// counter and from nothing else, `armed` from the effective child log filter,
/// and `epochs_exited` from the scrape. None of the four is inferred from a log
/// line, so a reading can never rest on a quantity nobody produced.
#[allow(dead_code)] // wired in G4
#[must_use]
pub fn aggregate_origin_lines(
    captured: &[String],
    dropped: u64,
    restarts: u64,
    armed: bool,
    epochs_exited: u64,
) -> OriginAggregate {
    let mut lines = Vec::with_capacity(captured.len());
    let mut unparsed: u64 = 0;
    for raw in captured {
        match parse_origin_line(raw) {
            Some(parsed) => lines.push(parsed),
            None => unparsed = unparsed.saturating_add(1),
        }
    }
    OriginAggregate {
        matched: len_as_u64(captured.len()),
        unparsed,
        dropped,
        lines,
        restarts,
        armed,
        epochs_exited,
        reason: None,
    }
}

/// Classify the origin reading, evaluated fail-closed first.
///
/// Returns the reading together with its NAMED reason — the reason the
/// aggregate carries and the serializer emits beside the counters it was
/// derived from. Every variant ROUTES; none is diagnosed here.
#[allow(dead_code)] // wired in G4
#[must_use]
pub fn classify_origin_reading(aggregate: &OriginAggregate) -> (OriginReading, Option<String>) {
    if !aggregate.armed {
        return (
            OriginReading::IndeterminateInstrument,
            Some("the origin log filter was not armed for this run".to_string()),
        );
    }
    if aggregate.dropped > 0 {
        return (
            OriginReading::IndeterminateInstrument,
            Some(format!(
                "{} captured line(s) were dropped, so the window is incomplete",
                aggregate.dropped
            )),
        );
    }
    if aggregate.unparsed > 0 {
        return (
            OriginReading::IndeterminateInstrument,
            Some(format!(
                "{} matched line(s) did not yield all eight fields",
                aggregate.unparsed
            )),
        );
    }
    if aggregate.restarts > 0 {
        return (
            OriginReading::IndeterminateInstrument,
            Some(format!(
                "the server restarted {} time(s) during the run, so the epochs_exited \
                 qualifier reset and would be misread",
                aggregate.restarts
            )),
        );
    }

    if let Some(line) = aggregate
        .lines
        .iter()
        .find(|l| l.refs_returned == 0 && l.refs_at_entry > 0)
    {
        return (
            OriginReading::ReachedInProduction,
            Some(format!(
                "epoch {} returned 0 refs while holding {} at entry",
                line.epoch, line.refs_at_entry
            )),
        );
    }

    if let Some(line) = aggregate
        .lines
        .iter()
        .find(|l| l.refs_returned != l.refs_at_entry)
    {
        return (
            OriginReading::PartialDivergence,
            Some(format!(
                "epoch {} returned {} refs against {} at entry",
                line.epoch, line.refs_returned, line.refs_at_entry
            )),
        );
    }

    if !aggregate.lines.is_empty() {
        return (
            OriginReading::NotReachedEqualRefs,
            Some(format!(
                "all {} parsed line(s) returned exactly the refs held at entry",
                aggregate.lines.len()
            )),
        );
    }

    if aggregate.epochs_exited > 0 {
        return (
            OriginReading::NoLinesWhileEpochsExited,
            Some(format!(
                "no line was captured while epochs_exited reached {}",
                aggregate.epochs_exited
            )),
        );
    }

    (
        OriginReading::NotObservedAtHead,
        Some("no line was captured and epochs_exited stayed at 0".to_string()),
    )
}

/// The `max` and the `sum` of one labelled gauge, taken over its label sets.
///
/// Returned as a named pair rather than a positional one so a caller cannot
/// silently transpose the two. `None` from [`parse_labelled_gauge`] — never a
/// zero here — is what carries "the metric was absent from the body".
#[allow(dead_code)] // wired in G4
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LabelledGauge {
    /// The largest value across the metric's label sets.
    pub max: u64,
    /// The sum across the metric's label sets.
    pub sum: u64,
}

/// Parse a labelled gauge out of a Prometheus text exposition body, folding its
/// label sets into a max and a sum.
///
/// Comment lines (`# HELP` / `# TYPE`) and blanks are skipped, and the name is
/// matched EXACTLY — either bare or immediately followed by `{` — so a metric
/// is never confused with a co-resident one that merely shares its prefix.
/// Returns `None` when the metric does not appear in the body at all: an
/// absence is a visible gap, and reporting it as a zero would let a silent
/// absence masquerade as a flat series.
#[allow(dead_code)] // wired in G4
#[must_use]
pub fn parse_labelled_gauge(body: &str, metric: &str) -> Option<LabelledGauge> {
    let mut seen = false;
    let mut gauge = LabelledGauge::default();
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((head, raw_value)) = line.rsplit_once(char::is_whitespace) else {
            continue;
        };
        let head = head.trim();
        let matches =
            head == metric || (head.starts_with(metric) && head[metric.len()..].starts_with('{'));
        if !matches {
            continue;
        }
        // Gauge samples are rendered as decimals by some exporters; the durable
        // instrument's own quantities are integer-semantic, so a fractional
        // sample is floored rather than routed through an `f64` field.
        let value: u64 = match raw_value.trim().parse::<u64>() {
            Ok(v) => v,
            Err(_) => match raw_value.trim().split_once('.') {
                Some((whole, _)) => whole.parse::<u64>().ok()?,
                None => continue,
            },
        };
        seen = true;
        gauge.max = gauge.max.max(value);
        gauge.sum = gauge.sum.saturating_add(value);
    }
    seen.then_some(gauge)
}

/// The embedded store file's apparent size in bytes, from filesystem METADATA
/// only — the file is never opened, so sampling cannot perturb the process
/// being measured. `None` on any failure, mirroring [`sample_rss_mb`]'s
/// `None`-on-failure contract.
#[allow(dead_code)] // wired in G4
#[must_use]
pub fn sample_redb_bytes(data_dir: &Path) -> Option<u64> {
    let meta = std::fs::metadata(data_dir.join("topgun.redb")).ok()?;
    meta.is_file().then_some(meta.len())
}

/// WAL retention: the summed apparent size and the count of the retained `.log`
/// segments, both from directory metadata only — no segment file is opened.
///
/// `None` when the WAL directory cannot be read at all. A directory that reads
/// but holds no segment yields `Some` zeros: an honest empty retention, which
/// must stay distinguishable from a blind sampler.
#[allow(dead_code)] // wired in G4
#[must_use]
pub fn sample_wal_retention(data_dir: &Path) -> Option<WalRetention> {
    let entries = std::fs::read_dir(data_dir.join("wal")).ok()?;
    let mut retention = WalRetention::default();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("log") {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        retention.bytes = retention.bytes.saturating_add(meta.len());
        retention.segment_files = retention.segment_files.saturating_add(1);
    }
    Some(retention)
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
    /// NOTE — this validates the OLS/last-half-window MATH directly on a
    /// hand-authored series; it does not itself drive a live server. A real
    /// run reaches this same grow-then-flatten shape once the tracked-and-ACKing
    /// client (`main.rs`'s `SoakClient::connect_tracked` + `confirm_apply`)
    /// advances the low-water-mark far enough for the epoch-scoped prune to
    /// engage — see `calibration_additive_only_gauge_never_plateaus` below for
    /// the contrasting shape produced when nothing drives the low-water-mark
    /// (e.g. `main.rs`'s `--no-ack` negative control).
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

    /// Pins the shape produced when NOTHING drives the low-water-mark forward —
    /// exactly `main.rs`'s `--no-ack` negative control (or, pre-this-spec, every
    /// production run, since no client ever ran the confirm-apply protocol at
    /// all): with the low-water-mark vacuously 0, the epoch-scoped prune never
    /// fires, so the gauge climbs monotonically at the tombstone-*creation* rate
    /// and never flattens — the grow-then-flatten shape the plateau statistic
    /// looks for cannot occur. This test feeds exactly that monotone shape (well
    /// past the min-window floor) and asserts the assessment reports NOT-passed
    /// — which is precisely the hard-gate failure `main.rs` asserts on for this
    /// scenario in every run class; the durable-corpus level clause beside it is
    /// report-only (see the module doc's "Tombstone-byte gate" section).
    #[test]
    fn calibration_additive_only_gauge_never_plateaus() {
        // ~6h of steady creation at 5000 B/h — a multi-hour last-half window
        // (well past the 120s min-window floor), monotone, no flatten. This is
        // the shape an unbounded (no-driver / `--no-ack`) soak produces.
        let samples = linear_bytes_series(1_000, 6, 5_000);
        let a = assess_tombstone_bytes(
            &samples,
            DEFAULT_TOMBSTONE_BYTES_THRESHOLD_PER_HOUR,
            DEFAULT_TOMBSTONE_BYTES_MIN_GROWTH,
            DEFAULT_TOMBSTONE_BYTES_MIN_WINDOW_SECS,
        );
        assert!(
            !a.passed,
            "an unbounded monotone-growth shape (no low-water-mark driver) must \
             NOT be reported as a plateau — this is the hard-gate FAIL `main.rs` \
             asserts on in every run class; \
             slope={:.1} reason={:?}",
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

    /// Build a durable-corpus series from explicit byte totals, sampled at a
    /// fixed interval on the harness's elapsed-seconds clock. The totals stay
    /// `u64` end to end — no counted byte total is routed through an `f64`.
    fn corpus_series(interval_secs: f64, totals: &[u64]) -> Vec<CorpusSample> {
        totals
            .iter()
            .enumerate()
            .map(|(index, &bytes)| CorpusSample {
                elapsed_secs: f64::from(u32::try_from(index).unwrap_or(u32::MAX)) * interval_secs,
                bytes,
            })
            .collect()
    }

    /// Assess a corpus series under the calibrated defaults, leaving only the
    /// scan counters and the ceiling to the caller.
    fn assess_corpus_defaults(
        samples: &[CorpusSample],
        scans_attempted: usize,
        scans_failed: usize,
        ceiling_bytes: Option<u64>,
    ) -> TombstoneCorpusAssessment {
        assess_tombstone_corpus_level(
            samples,
            scans_attempted,
            scans_failed,
            DEFAULT_TOMBSTONE_CORPUS_HEADROOM_BYTES,
            DEFAULT_TOMBSTONE_CORPUS_MIN_SPAN_SECS,
            DEFAULT_TOMBSTONE_CORPUS_MIN_SAMPLES,
            ceiling_bytes,
        )
    }

    /// A LINEAR durable-corpus leak MUST fail the level clause: the last-half
    /// peak stands far above the first-half peak, which is precisely the
    /// unbounded-growth shape the clause exists to discriminate. A comparison
    /// that could not see the difference between the two half-peaks would let
    /// this series through.
    #[test]
    fn calibration_fails_linear_corpus_level() {
        let samples = corpus_series(
            90.0,
            &[
                0, 100_000, 200_000, 300_000, 400_000, 500_000, 600_000, 700_000,
            ],
        );
        let a = assess_corpus_defaults(&samples, samples.len(), 0, None);
        assert_eq!(a.disposition, CorpusLevelDisposition::LevelEvaluated);
        assert_eq!(a.first_half_peak_bytes, 300_000);
        assert_eq!(a.last_half_peak_bytes, 700_000);
        assert_eq!(a.rise_bytes, 400_000);
        assert!(
            !a.passed,
            "a linear corpus leak must breach the level clause; rise={} B reason={:?}",
            a.rise_bytes, a.reason
        );
    }

    /// FROZEN FIXTURE: 8 samples 90s apart; first sample 10,000 B; the series
    /// ramps to 300,000 B by sample 4 and then plateaus FLAT at 300,000 B for
    /// samples 5-8.
    ///
    /// Frozen relationship: the ramp's height above the FIRST sample exceeds
    /// the headroom (290,000 B > 65,536 B) while the half-to-half rise is
    /// exactly zero. A genuine ramp-then-plateau is the healthy shape and must
    /// NOT false-RED; a level test taken against the series' ORIGIN instead of
    /// its first-half peak would condemn it.
    #[test]
    fn calibration_passes_plateau_after_ramp() {
        let samples = corpus_series(
            90.0,
            &[
                10_000, 110_000, 210_000, 300_000, 300_000, 300_000, 300_000, 300_000,
            ],
        );
        let a = assess_corpus_defaults(&samples, samples.len(), 0, None);
        assert_eq!(a.disposition, CorpusLevelDisposition::LevelEvaluated);
        assert!(a.span_secs >= DEFAULT_TOMBSTONE_CORPUS_MIN_SPAN_SECS);
        assert_eq!(a.first_half_peak_bytes, 300_000);
        assert_eq!(a.last_half_peak_bytes, 300_000);
        assert_eq!(a.rise_bytes, 0);
        assert!(
            a.peak_bytes - a.first_bytes > DEFAULT_TOMBSTONE_CORPUS_HEADROOM_BYTES,
            "the fixture's ramp must clear the headroom measured from the origin, \
             or a level-vs-origin comparison would agree with the correct one"
        );
        assert!(
            a.passed,
            "a ramp that plateaus must pass; rise={} B reason={:?}",
            a.rise_bytes, a.reason
        );
    }

    /// FROZEN FIXTURE: 5 scans attempted, 1 FAILED, 4 usable samples 210s apart
    /// on a flat series (span 630s).
    ///
    /// Frozen relationship: the surviving samples clear BOTH level-clause
    /// guards and the series is flat, so an instrument clause that ignored the
    /// failed scan would evaluate the level clause and PASS. A fixture in which
    /// every scan failed would be decided by the zero-sample disjunct instead
    /// and would prove nothing about the failed-scan one.
    #[test]
    fn calibration_fails_on_failed_scan() {
        let samples = corpus_series(210.0, &[120_000; 4]);
        let a = assess_corpus_defaults(&samples, 5, 1, None);
        assert_eq!(a.samples, 4);
        assert_eq!(a.scans_failed, 1);
        assert!(a.samples >= DEFAULT_TOMBSTONE_CORPUS_MIN_SAMPLES);
        assert!(a.span_secs >= DEFAULT_TOMBSTONE_CORPUS_MIN_SPAN_SECS);
        assert_eq!(a.rise_bytes, 0);
        assert_eq!(a.disposition, CorpusLevelDisposition::InstrumentFailed);
        assert!(
            !a.passed,
            "a blind scan must be assessed NOT-passed and typed InstrumentFailed; \
             the clause itself is report-only. reason={:?}",
            a.reason
        );
    }

    /// A scan that read a never-written table and reported zero bytes is an
    /// honest measurement, not a blindness: a whole series of them MUST pass,
    /// and must stay distinguishable from a scan that could not read the state
    /// at all. Treating a zero total as a failed scan would fail an empty but
    /// perfectly healthy corpus.
    #[test]
    fn calibration_passes_honest_empty_corpus() {
        let samples = corpus_series(90.0, &[0; 8]);
        let a = assess_corpus_defaults(&samples, samples.len(), 0, None);
        assert_eq!(a.scans_failed, 0);
        assert_eq!(a.min_bytes, 0);
        assert_eq!(a.peak_bytes, 0);
        assert_eq!(a.rise_bytes, 0);
        assert_eq!(a.disposition, CorpusLevelDisposition::LevelEvaluated);
        assert!(
            a.passed,
            "an all-zero corpus is an honest empty one and must pass; reason={:?}",
            a.reason
        );
    }

    /// A SLOW unbounded leak must still be caught once enough duration has been
    /// modelled: 16 samples 120s apart growing 9,000 B per sample. No single
    /// step comes anywhere near the headroom, so the breach exists only in the
    /// accumulated half-to-half level — which is the property that lets the
    /// level clause discriminate a leak the per-hour slope statistic would
    /// report as a small, unremarkable rate.
    #[test]
    fn calibration_fails_slow_unbounded_ramp() {
        let totals: Vec<u64> = (0..16).map(|i| 50_000 + i * 9_000).collect();
        let samples = corpus_series(120.0, &totals);
        let a = assess_corpus_defaults(&samples, samples.len(), 0, None);
        assert_eq!(a.disposition, CorpusLevelDisposition::LevelEvaluated);
        let widest_step = samples
            .windows(2)
            .map(|pair| pair[1].bytes.saturating_sub(pair[0].bytes))
            .max()
            .unwrap_or(0);
        assert!(
            widest_step < DEFAULT_TOMBSTONE_CORPUS_HEADROOM_BYTES,
            "no single step may reach the headroom, or the ramp is not slow; \
             widest step={widest_step} B"
        );
        assert_eq!(a.rise_bytes, 72_000);
        assert!(a.rise_bytes > DEFAULT_TOMBSTONE_CORPUS_HEADROOM_BYTES);
        assert!(
            !a.passed,
            "a slow but unbounded ramp must breach once the modelled duration is \
             long enough; rise={} B reason={:?}",
            a.rise_bytes, a.reason
        );
    }

    /// FROZEN FIXTURE: 8 samples 90s apart with the ceiling ARMED at 100,000 B;
    /// the series peaks at 200,000 B inside the FIRST half and decreases
    /// monotonically to 50,000 B at the last sample.
    ///
    /// Frozen relationship: `peak > ceiling >= last`, and the half-to-half rise
    /// is zero. So the level clause PASSES and the failure is attributable to
    /// the absolute ceiling alone — and a ceiling compared against the LAST
    /// sample rather than the peak would not fire at all. The ceiling is an
    /// ENVELOPE test, because prune legitimately produces large downward
    /// excursions a rising envelope could otherwise hide behind.
    #[test]
    fn calibration_fails_armed_ceiling() {
        let samples = corpus_series(
            90.0,
            &[
                200_000, 175_000, 150_000, 125_000, 100_000, 85_000, 70_000, 50_000,
            ],
        );
        let a = assess_corpus_defaults(&samples, samples.len(), 0, Some(100_000));
        assert_eq!(a.disposition, CorpusLevelDisposition::LevelEvaluated);
        assert_eq!(
            a.rise_bytes, 0,
            "the level clause must pass, so the verdict is attributable to the ceiling"
        );
        assert_eq!(a.peak_bytes, 200_000);
        assert_eq!(a.last_bytes, 50_000);
        assert_eq!(a.ceiling_bytes, Some(100_000));
        assert!(a.peak_bytes > 100_000 && a.last_bytes <= 100_000);
        assert!(
            !a.passed,
            "an armed ceiling must breach on the PEAK even though the last sample \
             sits below it; reason={:?}",
            a.reason
        );
    }

    /// Whenever the level clause did NOT decide, the slope clause must still
    /// decide — otherwise a run could pass through a window no clause held.
    /// The enumeration lives on the type: a fourth disposition fails to compile
    /// inside the predicate, so what this asserts is the CLASSIFICATION of each
    /// variant, not the exhaustiveness (the compiler owns that).
    ///
    /// The blind-instrument variant is checked a second way: such an assessment
    /// already carries `passed == false`, so it fails through the first
    /// conjunct and never depends on the fallback to rescue it.
    #[test]
    fn calibration_slope_clause_hard_for_every_non_evaluated_disposition() {
        for (disposition, stays_hard) in [
            (CorpusLevelDisposition::LevelEvaluated, false),
            (CorpusLevelDisposition::LevelSuppressed, true),
            (CorpusLevelDisposition::InstrumentFailed, true),
        ] {
            assert_eq!(
                slope_clause_stays_hard(disposition),
                stays_hard,
                "{} is classified wrongly",
                disposition.as_str()
            );
        }

        let blind = assess_corpus_defaults(&[], 1, 1, None);
        assert_eq!(blind.disposition, CorpusLevelDisposition::InstrumentFailed);
        assert!(
            !blind.passed,
            "a blind instrument must already fail through the first conjunct"
        );
    }
}
