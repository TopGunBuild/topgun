//! Phi-accrual and deadline failure detectors.
//!
//! Provides two implementations of the `FailureDetector` trait:
//! - `PhiAccrualFailureDetector`: statistical failure detection based on heartbeat
//!   interval distribution, using the CDF-based phi formula from the phi-accrual
//!   failure detector paper (Hayashibara et al., 2004).
//! - `DeadlineFailureDetector`: simple deadline-based detection for testing.

use std::collections::HashMap;

use parking_lot::RwLock;

use super::traits::FailureDetector;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for the phi-accrual failure detector.
#[derive(Debug, Clone)]
pub struct PhiAccrualConfig {
    /// Phi value at which a node is considered dead.
    pub phi_threshold: f64,
    /// Maximum number of heartbeat intervals to retain in the sample window.
    pub max_sample_size: usize,
    /// Floor for standard deviation to prevent false positives from very
    /// consistent heartbeats (ms).
    pub min_std_dev_ms: u64,
    /// Maximum time without a heartbeat before considering a node dead (ms).
    pub max_no_heartbeat_ms: u64,
    /// Expected interval between heartbeats (ms).
    pub heartbeat_interval_ms: u64,
}

impl Default for PhiAccrualConfig {
    fn default() -> Self {
        Self {
            phi_threshold: 8.0,
            max_sample_size: 200,
            min_std_dev_ms: 100,
            max_no_heartbeat_ms: 5000,
            heartbeat_interval_ms: 1000,
        }
    }
}

// ---------------------------------------------------------------------------
// Internal heartbeat tracking
// ---------------------------------------------------------------------------

/// Per-node heartbeat tracking state.
struct NodeHeartbeatState {
    /// Timestamp of the most recent heartbeat (ms since epoch).
    last_heartbeat_ms: u64,
    /// Circular buffer of observed inter-heartbeat intervals.
    intervals: Vec<u64>,
}

// ---------------------------------------------------------------------------
// Phi-accrual failure detector
// ---------------------------------------------------------------------------

/// Statistical failure detector using the phi-accrual algorithm.
///
/// Tracks heartbeat intervals per node and computes a suspicion level (phi)
/// based on the probability that the observed silence duration would occur
/// given the historical interval distribution. Higher phi means greater
/// suspicion of failure.
pub struct PhiAccrualFailureDetector {
    config: PhiAccrualConfig,
    states: RwLock<HashMap<String, NodeHeartbeatState>>,
}

impl PhiAccrualFailureDetector {
    /// Creates a new phi-accrual failure detector with the given configuration.
    #[must_use]
    pub fn new(config: PhiAccrualConfig) -> Self {
        Self {
            config,
            states: RwLock::new(HashMap::new()),
        }
    }
}

impl FailureDetector for PhiAccrualFailureDetector {
    fn heartbeat(&self, node_id: &str, timestamp_ms: u64) {
        let mut states = self.states.write();
        let max_samples = self.config.max_sample_size;

        match states.get_mut(node_id) {
            Some(state) => {
                // Compute interval since last heartbeat and record it.
                let interval = timestamp_ms.saturating_sub(state.last_heartbeat_ms);
                state.last_heartbeat_ms = timestamp_ms;

                // Maintain circular buffer: evict oldest when at capacity.
                if state.intervals.len() >= max_samples {
                    state.intervals.remove(0);
                }
                state.intervals.push(interval);
            }
            None => {
                // First heartbeat for this node -- just record the timestamp.
                states.insert(
                    node_id.to_string(),
                    NodeHeartbeatState {
                        last_heartbeat_ms: timestamp_ms,
                        intervals: Vec::new(),
                    },
                );
            }
        }
    }

    fn is_alive(&self, node_id: &str, timestamp_ms: u64) -> bool {
        self.suspicion_level(node_id, timestamp_ms) < self.config.phi_threshold
    }

    fn last_heartbeat(&self, node_id: &str) -> Option<u64> {
        let states = self.states.read();
        states.get(node_id).map(|s| s.last_heartbeat_ms)
    }

    #[allow(clippy::cast_precision_loss)]
    fn suspicion_level(&self, node_id: &str, timestamp_ms: u64) -> f64 {
        let states = self.states.read();
        let Some(state) = states.get(node_id) else {
            return 0.0;
        };

        let elapsed = timestamp_ms.saturating_sub(state.last_heartbeat_ms) as f64;

        if state.intervals.len() < 3 {
            // Not enough samples for statistical analysis -- fall back to
            // a simple deadline-style linear scaling.
            return elapsed / self.config.max_no_heartbeat_ms as f64
                * self.config.phi_threshold;
        }

        // Compute mean and standard deviation of recorded intervals.
        let n = state.intervals.len() as f64;
        let sum: u64 = state.intervals.iter().sum();
        let mean = sum as f64 / n;

        let variance = state
            .intervals
            .iter()
            .map(|&iv| {
                let diff = iv as f64 - mean;
                diff * diff
            })
            .sum::<f64>()
            / n;
        let std_dev = variance.sqrt().max(self.config.min_std_dev_ms as f64);

        // CDF of normal distribution: P(X <= elapsed)
        // CDF(x) = 0.5 * erfc(-(x - mean) / (std_dev * sqrt(2)))
        let y = -(elapsed - mean) / (std_dev * std::f64::consts::SQRT_2);
        let cdf = 0.5 * erfc(y);

        // phi = -log10(1 - CDF(elapsed))
        // Clamp (1 - CDF) to a small epsilon to prevent infinity when CDF
        // approaches 1.0. This bounds phi to a finite maximum (~308).
        let one_minus_cdf = (1.0 - cdf).max(f64::MIN_POSITIVE);
        let phi = -(one_minus_cdf.log10());

        // Clamp to non-negative (phi should never be negative).
        phi.max(0.0)
    }

    fn remove(&self, node_id: &str) {
        self.states.write().remove(node_id);
    }

    fn reset(&self) {
        self.states.write().clear();
    }
}

// ---------------------------------------------------------------------------
// Deadline failure detector (for testing)
// ---------------------------------------------------------------------------

/// Simple deadline-based failure detector.
///
/// A node is considered dead if no heartbeat arrives within
/// `max_no_heartbeat_ms`. Intended for testing scenarios where statistical
/// detection is unnecessary.
pub struct DeadlineFailureDetector {
    max_no_heartbeat_ms: u64,
    states: RwLock<HashMap<String, u64>>,
}

impl DeadlineFailureDetector {
    /// Creates a new deadline failure detector with the given timeout.
    #[must_use]
    pub fn new(max_no_heartbeat_ms: u64) -> Self {
        Self {
            max_no_heartbeat_ms,
            states: RwLock::new(HashMap::new()),
        }
    }
}

impl FailureDetector for DeadlineFailureDetector {
    fn heartbeat(&self, node_id: &str, timestamp_ms: u64) {
        self.states
            .write()
            .insert(node_id.to_string(), timestamp_ms);
    }

    fn is_alive(&self, node_id: &str, timestamp_ms: u64) -> bool {
        let states = self.states.read();
        match states.get(node_id) {
            Some(&last) => timestamp_ms.saturating_sub(last) <= self.max_no_heartbeat_ms,
            // No heartbeat recorded -- assume alive (no evidence of failure).
            None => true,
        }
    }

    fn last_heartbeat(&self, node_id: &str) -> Option<u64> {
        self.states.read().get(node_id).copied()
    }

    #[allow(clippy::cast_precision_loss)]
    fn suspicion_level(&self, node_id: &str, timestamp_ms: u64) -> f64 {
        let states = self.states.read();
        match states.get(node_id) {
            Some(&last) => {
                let elapsed = timestamp_ms.saturating_sub(last) as f64;
                // Linear scaling: reaches 8.0 at the deadline boundary.
                elapsed / self.max_no_heartbeat_ms as f64 * 8.0
            }
            None => 0.0,
        }
    }

    fn remove(&self, node_id: &str) {
        self.states.write().remove(node_id);
    }

    fn reset(&self) {
        self.states.write().clear();
    }
}

// ---------------------------------------------------------------------------
// erfc approximation (Abramowitz and Stegun, formula 7.1.26)
// ---------------------------------------------------------------------------

/// Complementary error function approximation.
///
/// Uses the rational approximation from Abramowitz and Stegun (Handbook of
/// Mathematical Functions, formula 7.1.26) with maximum error |epsilon| < 1.5e-7.
fn erfc(x: f64) -> f64 {
    // For negative arguments: erfc(-x) = 2 - erfc(x)
    let (z, negate) = if x < 0.0 { (-x, true) } else { (x, false) };

    let t = 1.0 / (1.0 + 0.327_591_1 * z);

    // Horner's form of the polynomial coefficients
    let poly = t
        * (0.254_829_592
            + t * (-0.284_496_736
                + t * (1.421_413_741 + t * (-1.453_152_027 + t * 1.061_405_429))));

    let result = poly * (-z * z).exp();

    if negate {
        2.0 - result
    } else {
        result
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- erfc approximation accuracy --

    #[test]
    fn erfc_at_zero() {
        // erfc(0) = 1.0
        let val = erfc(0.0);
        assert!((val - 1.0).abs() < 1e-6, "erfc(0) = {val}, expected 1.0");
    }

    #[test]
    fn erfc_at_positive_values() {
        // Known values: erfc(1) ≈ 0.1573, erfc(2) ≈ 0.00468
        let e1 = erfc(1.0);
        assert!(
            (e1 - 0.1572992).abs() < 1e-5,
            "erfc(1) = {e1}, expected ~0.1573"
        );

        let e2 = erfc(2.0);
        assert!(
            (e2 - 0.0046778).abs() < 1e-5,
            "erfc(2) = {e2}, expected ~0.00468"
        );
    }

    #[test]
    fn erfc_at_negative_value() {
        // erfc(-1) = 2 - erfc(1) ≈ 1.8427
        let val = erfc(-1.0);
        assert!(
            (val - 1.8427008).abs() < 1e-5,
            "erfc(-1) = {val}, expected ~1.8427"
        );
    }

    #[test]
    fn erfc_symmetry() {
        // erfc(x) + erfc(-x) = 2
        for &x in &[0.5, 1.0, 1.5, 2.0, 3.0] {
            let sum = erfc(x) + erfc(-x);
            assert!(
                (sum - 2.0).abs() < 1e-6,
                "erfc({x}) + erfc(-{x}) = {sum}, expected 2.0"
            );
        }
    }

    // -- PhiAccrualFailureDetector --

    #[test]
    fn phi_returns_zero_when_no_history() {
        let fd = PhiAccrualFailureDetector::new(PhiAccrualConfig::default());
        assert_eq!(fd.suspicion_level("node-1", 10_000), 0.0);
    }

    #[test]
    fn phi_increases_monotonically_with_elapsed_time() {
        let fd = PhiAccrualFailureDetector::new(PhiAccrualConfig::default());

        // Record enough heartbeats for statistical mode (>= 3 intervals).
        // Use varied intervals to produce a meaningful standard deviation,
        // preventing premature saturation at the float precision ceiling.
        fd.heartbeat("node-1", 1000);
        fd.heartbeat("node-1", 2200);
        fd.heartbeat("node-1", 3100);
        fd.heartbeat("node-1", 4500);
        fd.heartbeat("node-1", 5300);

        // Test at points close enough to the last heartbeat to stay within
        // the meaningful range of the CDF (avoid deep saturation).
        let phi_at_5500 = fd.suspicion_level("node-1", 5500);
        let phi_at_6000 = fd.suspicion_level("node-1", 6000);
        let phi_at_6500 = fd.suspicion_level("node-1", 6500);
        let phi_at_7000 = fd.suspicion_level("node-1", 7000);

        assert!(
            phi_at_6000 > phi_at_5500,
            "phi should increase: phi(6.0s)={phi_at_6000} > phi(5.5s)={phi_at_5500}"
        );
        assert!(
            phi_at_6500 > phi_at_6000,
            "phi should increase: phi(6.5s)={phi_at_6500} > phi(6.0s)={phi_at_6000}"
        );
        assert!(
            phi_at_7000 > phi_at_6500,
            "phi should increase: phi(7.0s)={phi_at_7000} > phi(6.5s)={phi_at_6500}"
        );
    }

    #[test]
    fn phi_is_alive_returns_false_after_timeout() {
        let fd = PhiAccrualFailureDetector::new(PhiAccrualConfig::default());

        // Record regular heartbeats.
        for i in 0..5 {
            fd.heartbeat("node-1", 1000 + i * 1000);
        }

        // Should be alive shortly after last heartbeat.
        assert!(fd.is_alive("node-1", 5500));

        // Should be dead well after max_no_heartbeat_ms.
        assert!(!fd.is_alive("node-1", 20_000));
    }

    #[test]
    fn phi_last_heartbeat_returns_timestamp() {
        let fd = PhiAccrualFailureDetector::new(PhiAccrualConfig::default());

        assert_eq!(fd.last_heartbeat("node-1"), None);

        fd.heartbeat("node-1", 5000);
        assert_eq!(fd.last_heartbeat("node-1"), Some(5000));

        fd.heartbeat("node-1", 6000);
        assert_eq!(fd.last_heartbeat("node-1"), Some(6000));
    }

    #[test]
    fn phi_remove_clears_node_state() {
        let fd = PhiAccrualFailureDetector::new(PhiAccrualConfig::default());

        fd.heartbeat("node-1", 1000);
        assert_eq!(fd.last_heartbeat("node-1"), Some(1000));

        fd.remove("node-1");
        assert_eq!(fd.last_heartbeat("node-1"), None);
        assert_eq!(fd.suspicion_level("node-1", 5000), 0.0);
    }

    #[test]
    fn phi_reset_clears_all_state() {
        let fd = PhiAccrualFailureDetector::new(PhiAccrualConfig::default());

        fd.heartbeat("node-1", 1000);
        fd.heartbeat("node-2", 2000);

        fd.reset();

        assert_eq!(fd.last_heartbeat("node-1"), None);
        assert_eq!(fd.last_heartbeat("node-2"), None);
    }

    #[test]
    fn phi_circular_buffer_caps_at_max_sample_size() {
        let config = PhiAccrualConfig {
            max_sample_size: 5,
            ..PhiAccrualConfig::default()
        };
        let fd = PhiAccrualFailureDetector::new(config);

        // Record 10 heartbeats (producing 9 intervals) with max_sample_size = 5.
        for i in 0..10 {
            fd.heartbeat("node-1", 1000 + i * 1000);
        }

        let states = fd.states.read();
        let state = states.get("node-1").unwrap();
        assert_eq!(
            state.intervals.len(),
            5,
            "circular buffer should cap at max_sample_size"
        );
    }

    #[test]
    fn phi_fallback_with_few_samples() {
        // With < 3 samples, the detector falls back to deadline-style linear scaling.
        let fd = PhiAccrualFailureDetector::new(PhiAccrualConfig::default());

        // One heartbeat at t=1000, no intervals yet.
        fd.heartbeat("node-1", 1000);

        // At t=1000 (elapsed=0), suspicion should be 0.
        let phi0 = fd.suspicion_level("node-1", 1000);
        assert!(
            phi0.abs() < 0.01,
            "phi should be ~0 at heartbeat time, got {phi0}"
        );

        // At t=3500 (elapsed=2500), suspicion = 2500/5000 * 8.0 = 4.0
        let phi_mid = fd.suspicion_level("node-1", 3500);
        assert!(
            (phi_mid - 4.0).abs() < 0.01,
            "phi should be ~4.0, got {phi_mid}"
        );
    }

    // -- DeadlineFailureDetector --

    #[test]
    fn deadline_is_alive_within_timeout() {
        let fd = DeadlineFailureDetector::new(5000);

        fd.heartbeat("node-1", 1000);

        assert!(fd.is_alive("node-1", 3000));
        assert!(fd.is_alive("node-1", 6000)); // exactly at deadline
    }

    #[test]
    fn deadline_is_dead_after_timeout() {
        let fd = DeadlineFailureDetector::new(5000);

        fd.heartbeat("node-1", 1000);

        assert!(!fd.is_alive("node-1", 6001));
        assert!(!fd.is_alive("node-1", 10_000));
    }

    #[test]
    fn deadline_suspicion_level_linear_scaling() {
        let fd = DeadlineFailureDetector::new(5000);

        // No history -> 0.
        assert_eq!(fd.suspicion_level("node-1", 1000), 0.0);

        fd.heartbeat("node-1", 1000);

        // At deadline boundary: 5000/5000 * 8.0 = 8.0
        let phi = fd.suspicion_level("node-1", 6000);
        assert!(
            (phi - 8.0).abs() < 0.01,
            "suspicion at deadline should be ~8.0, got {phi}"
        );

        // Half way: 2500/5000 * 8.0 = 4.0
        let phi_mid = fd.suspicion_level("node-1", 3500);
        assert!(
            (phi_mid - 4.0).abs() < 0.01,
            "suspicion at midpoint should be ~4.0, got {phi_mid}"
        );
    }

    #[test]
    fn deadline_no_heartbeat_is_alive() {
        // No heartbeat recorded -- node is considered alive (no evidence of failure).
        let fd = DeadlineFailureDetector::new(5000);
        assert!(fd.is_alive("unknown-node", 99_999));
    }

    #[test]
    fn deadline_remove_and_reset() {
        let fd = DeadlineFailureDetector::new(5000);

        fd.heartbeat("node-1", 1000);
        fd.heartbeat("node-2", 2000);

        fd.remove("node-1");
        assert_eq!(fd.last_heartbeat("node-1"), None);
        assert_eq!(fd.last_heartbeat("node-2"), Some(2000));

        fd.reset();
        assert_eq!(fd.last_heartbeat("node-2"), None);
    }
}
