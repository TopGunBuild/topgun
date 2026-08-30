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
//! is decided in `main.rs`, not here. The byte **slope** is now a HARD gate
//! there. The gauge is restart-survivable (`reconcile_tombstone_bytes` in
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
//! decision in `main.rs` and the calibration tests below — the assessment
//! itself does not know or care whether its caller treats a breach as
//! report-only or hard-gating.)
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
/// threshold measures is a HARD gate in `main.rs`: the decrementable gauge is
/// expected to plateau under sustained churn now that a tracked-and-ACKing
/// client drives the server's low-water-mark forward (see the module-level
/// "Tombstone-byte gate" doc above), subject to the min-window-span guard and
/// boot-gap exclusion. The blind-monitor zero-sample clause is a second,
/// independent hard gate. RSS above is the coarse backstop.
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
/// The slope is a per-hour EXTRAPOLATION (bytes/sec × 3600). Over a sub-minute
/// window the `3600 / span_secs` amplification is enormous: a healthy short run
/// that has simply not yet had time to plateau (e.g. the 25s blocking CI "Short
/// no-crash soak", ~6 samples over ~25s with a few KB of ordinary ramp-up)
/// extrapolates to a six-figure B/h rate and would surface a spurious breach.
/// Below this floor the slope carries no plateau signal — a leak and a
/// not-yet-plateaued healthy run are indistinguishable — so the clause is
/// suppressed (no breach is emitted for it) and the assessment passes on the
/// blind-monitor + absolute-growth clauses alone. (The slope is now a HARD gate
/// once the window clears this floor — see [`DEFAULT_TOMBSTONE_BYTES_THRESHOLD_PER_HOUR`];
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
#[allow(dead_code)]
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
#[allow(dead_code)]
pub const DEFAULT_TOMBSTONE_CORPUS_MIN_SPAN_SECS: f64 = 600.0;

/// Minimum corpus samples before L1 may decide (at least 2 per half).
///
/// Two per half, so neither half-peak is a single point — the degenerate case
/// [`last_half_window_span_secs`]'s own guard excludes for the same reason. At
/// `--crash-interval 120` a 900 s cell yields 8 samples (7 checkpoints plus the
/// terminal scan), clearing this guard with margin.
#[allow(dead_code)]
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
#[allow(dead_code)]
pub const DEFAULT_TOMBSTONE_CORPUS_CEILING_BYTES: Option<u64> = None;

/// One durable-layer OR tombstone corpus scan, taken while the server process is
/// DEAD (redb is single-writer, so its file lock must be free) and therefore
/// reading the PRE-RECOVERY on-disk state.
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct CorpusSample {
    pub elapsed_secs: f64,
    pub bytes: u64,
}

/// Which clause actually decided a corpus assessment. Rendered verbatim, so a
/// clause that did not fire is visible rather than indistinguishable from a pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum CorpusLevelDisposition {
    /// L1 was evaluated and decided.
    LevelEvaluated,
    /// L1 was NOT evaluated, so the slope clause hard-gates instead. Never "ok".
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
    #[allow(dead_code)]
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
#[allow(dead_code)]
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

/// L0 and L1 are HARD gate clauses; L1 only when its two guards are met.
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
#[allow(dead_code)]
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

/// Whether the tombstone-byte SLOPE clause still hard-gates a run that
/// produced this disposition. EXHAUSTIVE BY CONSTRUCTION: a fourth variant
/// does not compile here, which is what makes the no-ungated-window coverage
/// argument structural rather than a source-read. `main.rs`'s verdict
/// expression consumes this and re-types no comparison of its own.
#[must_use]
// One arm per variant, deliberately not merged into a single `|` pattern: the
// point of the enumeration is that every variant is classified in its own
// right, so a fourth variant has to be given an explicit answer here rather
// than being absorbed into an existing pattern.
#[allow(dead_code, clippy::match_same_arms)]
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
    /// — which is precisely the hard-gate failure `main.rs` now asserts on for
    /// this scenario (see the module doc's "Tombstone-byte gate" section).
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
             now asserts on; slope={:.1} reason={:?}",
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
            "a blind scan must fail the run closed; reason={:?}",
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
