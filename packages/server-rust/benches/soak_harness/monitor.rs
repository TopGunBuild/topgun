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
//! is decided in `main.rs`, not here. Today the byte **slope** is report-only —
//! the process-local counter resets to 0 on every `kill -9`, so across a
//! crash-enabled run its series is a per-life sawtooth whose OLS slope is
//! untrustworthy, and the OR-Map tombstone leak it measures is deliberately
//! unbounded until TODO-566. `main.rs` therefore routes a slope breach to its
//! `pending_gates` channel (surfaced, does NOT fail the run) and hard-gates only
//! on the blind-monitor (zero-sample) case. So do not read this module's
//! `passed: bool` as load-bearing everywhere — see `main.rs`'s run-end
//! verdict-assembly for the authoritative gating decision.

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

    let slope_mb_per_hour = least_squares_slope_per_hour(samples);

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

/// Least-squares slope of `rss_mb` vs. time, expressed in MB per hour. Returns
/// 0 when there is insufficient variance in the time axis (e.g. one sample).
#[allow(clippy::cast_precision_loss)]
fn least_squares_slope_per_hour(samples: &[MemSample]) -> f64 {
    let n = samples.len() as f64;
    if n < 2.0 {
        return 0.0;
    }
    // x in hours so the slope is directly MB/hour.
    let xs: Vec<f64> = samples.iter().map(|s| s.elapsed_secs / 3600.0).collect();
    let ys: Vec<f64> = samples.iter().map(|s| s.rss_mb).collect();
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

/// Calibrated slope ceiling (bytes/hour) for the tombstone-byte gate.
///
/// ~0.5 KB/h. Tight by design — see the module-level "no detection floor" doc:
/// with no RSS-style noise to absorb, any sustained per-hour growth this small
/// is already real signal, not measurement wobble.
///
/// Consumed by `main.rs` (the soak loop scrapes `GET /metrics` and calls
/// [`assess_tombstone_bytes`] with this threshold alongside the RSS `assess`) and
/// by the `soak_monitor_calibration` integration target's tests — the harness
/// wiring has landed, so no `allow(dead_code)` is needed here.
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
#[allow(clippy::cast_precision_loss)]
pub fn assess_tombstone_bytes(
    samples: &[TombstoneSample],
    threshold_bytes_per_hour: f64,
    min_growth_bytes: f64,
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

    let slope_bytes_per_hour = least_squares_slope_per_hour_bytes(samples);

    #[allow(clippy::cast_precision_loss)]
    let growth = peak_bytes.saturating_sub(first_bytes) as f64;
    let mut reasons = Vec::new();

    if growth >= min_growth_bytes && slope_bytes_per_hour > threshold_bytes_per_hour {
        reasons.push(format!(
            "tombstone-byte growth slope {slope_bytes_per_hour:.1} bytes/h exceeds \
             {threshold_bytes_per_hour:.1} bytes/h (total growth {growth:.0} bytes over {} \
             samples)",
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

/// Least-squares slope of `bytes` vs. time, expressed in bytes per hour.
/// Same fit as [`least_squares_slope_per_hour`]; kept as a separate function
/// because the sample type differs (`u64` bytes vs. `f64` MB).
#[allow(clippy::cast_precision_loss)]
fn least_squares_slope_per_hour_bytes(samples: &[TombstoneSample]) -> f64 {
    let n = samples.len() as f64;
    if n < 2.0 {
        return 0.0;
    }
    let xs: Vec<f64> = samples.iter().map(|s| s.elapsed_secs / 3600.0).collect();
    let ys: Vec<f64> = samples.iter().map(|s| s.bytes as f64).collect();
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
        );
        assert!(!a.passed, "zero samples must fail as a blind monitor");
    }
}
