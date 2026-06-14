//! Configuration for the LRU eviction background task.
//!
//! [`EvictionConfig`] is constructed via [`EvictionConfig::from_env`] at startup,
//! falling back to conservative defaults on any parse failure so the server can
//! boot without operator intervention.

/// Configuration for the LRU eviction orchestrator.
///
/// All integer fields use non-`f64` types per the project's Rust Type Mapping
/// Rules: byte counts and intervals are `u64`; percent thresholds are `u8`.
///
/// # Defaults
///
/// | Field | Default |
/// |---|---|
/// | `max_ram_bytes` | 1 073 741 824 (1 GiB) |
/// | `high_water_pct` | 85 |
/// | `low_water_pct` | 70 |
/// | `interval_ms` | 1 000 ms |
///
/// Defaults follow the Hazelcast `LRUEvictionPolicy` reference pattern and are
/// appropriate for a single-node HN-demo deployment.
///
/// Use [`EvictionConfig::from_env`] to override defaults via environment variables
/// at startup. [`Default::default`] supplies the canonical conservative defaults
/// and is used by `from_env` as the fallback source of truth.
#[derive(Debug, Clone)]
pub struct EvictionConfig {
    /// Maximum in-memory footprint in bytes before eviction engages.
    ///
    /// Derived from `TOPGUN_MAX_RAM_MB` (megabytes); stored internally as bytes
    /// to avoid repeated multiplication in the hot eviction loop.
    pub max_ram_bytes: u64,
    /// Fraction of `max_ram_bytes` (0–100) at which eviction starts.
    ///
    /// Parsed from `TOPGUN_EVICTION_HIGH_PCT`. Must be strictly greater than
    /// [`low_water_pct`](Self::low_water_pct) and ≤ 100.
    pub high_water_pct: u8,
    /// Fraction of `max_ram_bytes` (0–100) at which eviction stops.
    ///
    /// Parsed from `TOPGUN_EVICTION_LOW_PCT`. Must be strictly less than
    /// [`high_water_pct`](Self::high_water_pct).
    pub low_water_pct: u8,
    /// How often the eviction loop wakes and checks memory usage, in milliseconds.
    ///
    /// Parsed from `TOPGUN_EVICTION_INTERVAL_MS`.
    pub interval_ms: u64,
}

impl Default for EvictionConfig {
    fn default() -> Self {
        Self {
            max_ram_bytes: 1024 * 1024 * 1024, // 1 GiB
            high_water_pct: 85,
            low_water_pct: 70,
            interval_ms: 1000,
        }
    }
}

impl EvictionConfig {
    /// Construct [`EvictionConfig`] from environment variables.
    ///
    /// Reads four env vars; any missing or unparseable var falls back to the
    /// corresponding [`Self::default`] field and emits a `tracing::warn!`. The
    /// server never panics due to a misconfigured eviction env var.
    ///
    /// | Env var | Field | Default |
    /// |---|---|---|
    /// | `TOPGUN_MAX_RAM_MB` | `max_ram_bytes` | 1024 MiB |
    /// | `TOPGUN_EVICTION_HIGH_PCT` | `high_water_pct` | 85 |
    /// | `TOPGUN_EVICTION_LOW_PCT` | `low_water_pct` | 70 |
    /// | `TOPGUN_EVICTION_INTERVAL_MS` | `interval_ms` | 1000 ms |
    ///
    /// After parsing, if `low_water_pct >= high_water_pct`, both water-mark
    /// fields are atomically reverted to their defaults together (85 / 70) to
    /// prevent a half-default state where one was sourced from the env and the
    /// other from the default.
    #[must_use]
    pub fn from_env() -> Self {
        Self::from_source(|key| std::env::var(key).ok())
    }

    /// Construct [`EvictionConfig`] from an injected variable source.
    ///
    /// `get` maps a variable name to its value (or `None` if unset). This is the
    /// testable seam behind [`Self::from_env`], which simply passes a closure
    /// over `std::env::var`. Tests pass a map-backed closure so they exercise the
    /// full parse/validation logic without mutating process-global environment
    /// state — eliminating the cross-test env race that made parallel runs flaky.
    #[must_use]
    pub fn from_source<F: Fn(&str) -> Option<String>>(get: F) -> Self {
        let defaults = Self::default();
        let mut cfg = defaults.clone();

        // Parse TOPGUN_MAX_RAM_MB → max_ram_bytes (u64, then multiply by 1 MiB)
        if let Some(raw) = get("TOPGUN_MAX_RAM_MB") {
            match raw.trim().parse::<u64>() {
                Ok(mb) => {
                    cfg.max_ram_bytes = mb.saturating_mul(1024 * 1024);
                }
                Err(err) => {
                    tracing::warn!(
                        target: "topgun_server::storage::eviction",
                        var = "TOPGUN_MAX_RAM_MB",
                        value = %raw,
                        error = %err,
                        default_mb = defaults.max_ram_bytes / (1024 * 1024),
                        "Failed to parse env var; using default"
                    );
                    cfg.max_ram_bytes = defaults.max_ram_bytes;
                }
            }
        }

        // Parse TOPGUN_EVICTION_HIGH_PCT → high_water_pct (u8)
        if let Some(raw) = get("TOPGUN_EVICTION_HIGH_PCT") {
            match raw.trim().parse::<u8>() {
                Ok(pct) => {
                    cfg.high_water_pct = pct;
                }
                Err(err) => {
                    tracing::warn!(
                        target: "topgun_server::storage::eviction",
                        var = "TOPGUN_EVICTION_HIGH_PCT",
                        value = %raw,
                        error = %err,
                        default = defaults.high_water_pct,
                        "Failed to parse env var; using default"
                    );
                    cfg.high_water_pct = defaults.high_water_pct;
                }
            }
        }

        // Parse TOPGUN_EVICTION_LOW_PCT → low_water_pct (u8)
        if let Some(raw) = get("TOPGUN_EVICTION_LOW_PCT") {
            match raw.trim().parse::<u8>() {
                Ok(pct) => {
                    cfg.low_water_pct = pct;
                }
                Err(err) => {
                    tracing::warn!(
                        target: "topgun_server::storage::eviction",
                        var = "TOPGUN_EVICTION_LOW_PCT",
                        value = %raw,
                        error = %err,
                        default = defaults.low_water_pct,
                        "Failed to parse env var; using default"
                    );
                    cfg.low_water_pct = defaults.low_water_pct;
                }
            }
        }

        // Parse TOPGUN_EVICTION_INTERVAL_MS → interval_ms (u64)
        if let Some(raw) = get("TOPGUN_EVICTION_INTERVAL_MS") {
            match raw.trim().parse::<u64>() {
                Ok(ms) => {
                    cfg.interval_ms = ms;
                }
                Err(err) => {
                    tracing::warn!(
                        target: "topgun_server::storage::eviction",
                        var = "TOPGUN_EVICTION_INTERVAL_MS",
                        value = %raw,
                        error = %err,
                        default = defaults.interval_ms,
                        "Failed to parse env var; using default"
                    );
                    cfg.interval_ms = defaults.interval_ms;
                }
            }
        }

        // Validate water-mark ordering: low_water_pct must be strictly less than
        // high_water_pct, and high_water_pct must not exceed 100. On any violation
        // atomically revert BOTH water-mark fields to defaults to prevent a
        // half-default state (e.g., env-sourced high with defaulted low).
        let high_valid = cfg.high_water_pct <= 100;
        let ordering_valid = cfg.low_water_pct < cfg.high_water_pct;
        if !high_valid || !ordering_valid {
            tracing::warn!(
                target: "topgun_server::storage::eviction",
                high_water_pct = cfg.high_water_pct,
                low_water_pct = cfg.low_water_pct,
                default_high = defaults.high_water_pct,
                default_low = defaults.low_water_pct,
                "Invalid water-mark ordering (low_water_pct must be < high_water_pct <= 100); \
                 reverting both to defaults"
            );
            cfg.high_water_pct = defaults.high_water_pct;
            cfg.low_water_pct = defaults.low_water_pct;
        }

        cfg
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::collections::HashMap;

    /// Build a `from_source` lookup closure from a fixed map of overrides.
    ///
    /// This is the parallel-safe seam: parsing/validation is exercised in full
    /// without touching process-global env, so these tests never race each other
    /// or any other test reading the same vars.
    fn source(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        move |key: &str| map.get(key).cloned()
    }

    #[test]
    fn default_values_are_sane() {
        let cfg = EvictionConfig::default();
        assert_eq!(cfg.max_ram_bytes, 1024 * 1024 * 1024);
        assert_eq!(cfg.high_water_pct, 85);
        assert_eq!(cfg.low_water_pct, 70);
        assert_eq!(cfg.interval_ms, 1000);
        // Ordering invariant holds on defaults
        assert!(cfg.low_water_pct < cfg.high_water_pct);
        assert!(cfg.high_water_pct <= 100);
    }

    #[test]
    fn from_source_without_vars_returns_defaults() {
        let cfg = EvictionConfig::from_source(source(&[]));
        let defaults = EvictionConfig::default();
        assert_eq!(cfg.max_ram_bytes, defaults.max_ram_bytes);
        assert_eq!(cfg.high_water_pct, defaults.high_water_pct);
        assert_eq!(cfg.low_water_pct, defaults.low_water_pct);
        assert_eq!(cfg.interval_ms, defaults.interval_ms);
    }

    #[test]
    fn from_source_parses_valid_values() {
        let cfg = EvictionConfig::from_source(source(&[
            ("TOPGUN_MAX_RAM_MB", "2048"),
            ("TOPGUN_EVICTION_HIGH_PCT", "90"),
            ("TOPGUN_EVICTION_LOW_PCT", "60"),
            ("TOPGUN_EVICTION_INTERVAL_MS", "500"),
        ]));

        assert_eq!(cfg.max_ram_bytes, 2048 * 1024 * 1024);
        assert_eq!(cfg.high_water_pct, 90);
        assert_eq!(cfg.low_water_pct, 60);
        assert_eq!(cfg.interval_ms, 500);
    }

    #[test]
    fn unparseable_value_falls_back_to_default() {
        let cfg = EvictionConfig::from_source(source(&[("TOPGUN_MAX_RAM_MB", "not-a-number")]));
        assert_eq!(cfg.max_ram_bytes, EvictionConfig::default().max_ram_bytes);
    }

    #[test]
    fn watermark_inversion_reverts_both_to_defaults() {
        // low > high should trigger atomic revert of BOTH fields
        let cfg = EvictionConfig::from_source(source(&[
            ("TOPGUN_EVICTION_HIGH_PCT", "50"),
            ("TOPGUN_EVICTION_LOW_PCT", "75"),
        ]));

        let defaults = EvictionConfig::default();
        assert_eq!(
            cfg.high_water_pct, defaults.high_water_pct,
            "high_water_pct must revert to default on water-mark inversion"
        );
        assert_eq!(
            cfg.low_water_pct, defaults.low_water_pct,
            "low_water_pct must revert to default on water-mark inversion"
        );
    }

    #[test]
    fn watermark_equal_reverts_both_to_defaults() {
        // low == high is also invalid (requires strict ordering)
        let cfg = EvictionConfig::from_source(source(&[
            ("TOPGUN_EVICTION_HIGH_PCT", "70"),
            ("TOPGUN_EVICTION_LOW_PCT", "70"),
        ]));

        let defaults = EvictionConfig::default();
        assert_eq!(cfg.high_water_pct, defaults.high_water_pct);
        assert_eq!(cfg.low_water_pct, defaults.low_water_pct);
    }

    /// Wiring-only smoke test for the real `from_env` → `std::env` seam. This is
    /// the sole env-mutating test and is `#[serial]` so it cannot race the other
    /// (env-free) config tests. All parse/validation logic is covered above via
    /// `from_source`; here we only prove `from_env` actually reads the process
    /// environment.
    #[serial]
    #[test]
    fn from_env_reads_process_environment() {
        std::env::set_var("TOPGUN_MAX_RAM_MB", "2048");
        let cfg = EvictionConfig::from_env();
        std::env::remove_var("TOPGUN_MAX_RAM_MB");
        assert_eq!(cfg.max_ram_bytes, 2048 * 1024 * 1024);
    }
}
