//! Observability initialization: tracing-subscriber + Prometheus metrics recorder.
//!
//! Call [`init_observability`] once at application startup (before any tracing
//! macros fire) to install both a `tracing-subscriber` that respects `RUST_LOG`
//! and a global Prometheus metrics recorder.  The returned [`ObservabilityHandle`]
//! must be kept alive for the lifetime of the process and provides
//! [`ObservabilityHandle::render_metrics`] for the `/metrics` HTTP endpoint.
//!
//! Calling `init_observability` multiple times is safe: subsequent calls return a
//! handle that delegates to the already-installed recorder without panicking.

use std::sync::{Arc, OnceLock};

use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use tracing_subscriber::EnvFilter;

// ---------------------------------------------------------------------------
// Static initialisation guard
// ---------------------------------------------------------------------------

/// Guards the global Prometheus handle so that `init_observability` can be called
/// from multiple tests without panicking on the second recorder installation.
static PROMETHEUS_HANDLE: OnceLock<Arc<PrometheusHandle>> = OnceLock::new();

// ---------------------------------------------------------------------------
// ObservabilityHandle
// ---------------------------------------------------------------------------

/// Handle returned by [`init_observability`].
///
/// Holds a reference to the installed Prometheus recorder so that the `/metrics`
/// endpoint can render the current metric state on demand.
#[derive(Clone)]
pub struct ObservabilityHandle {
    prometheus: Arc<PrometheusHandle>,
}

impl ObservabilityHandle {
    /// Renders the current Prometheus metric state as a text exposition string.
    ///
    /// The returned string is suitable for serving directly as the body of a
    /// `GET /metrics` response with
    /// `Content-Type: text/plain; version=0.0.4; charset=utf-8`.
    #[must_use]
    pub fn render_metrics(&self) -> String {
        self.prometheus.render()
    }
}

// ---------------------------------------------------------------------------
// init_observability
// ---------------------------------------------------------------------------

/// Initialises the global tracing subscriber and Prometheus metrics recorder.
///
/// **Thread-safe and idempotent:** safe to call from multiple threads or from
/// multiple tests.  Only the first call installs the subscriber and recorder;
/// subsequent calls return a handle that delegates to the already-installed
/// recorder.
///
/// ## Tracing
///
/// - Filtering: respects the `RUST_LOG` environment variable via
///   `EnvFilter::from_default_env()`.  Defaults to `info` when `RUST_LOG` is
///   not set.
/// - Format: JSON when `TOPGUN_LOG_FORMAT=json`, human-readable otherwise.
///
/// ## Metrics
///
/// Installs `metrics-exporter-prometheus` as the global [`metrics`] recorder.
/// All `metrics::counter!`, `metrics::histogram!`, and `metrics::gauge!` calls
/// throughout the codebase will report to this recorder.
///
/// # Panics
///
/// Panics if the Prometheus recorder cannot be installed on the first call.
/// This can only happen if a recorder from a different library was already
/// installed globally before `init_observability` was first called.
/// Subsequent calls never panic.
pub fn init_observability() -> ObservabilityHandle {
    // Initialise the Prometheus recorder exactly once.  If a second call races
    // with the first the `OnceLock::get_or_init` closure is only executed once.
    let prometheus = PROMETHEUS_HANDLE
        .get_or_init(|| {
            let handle = PrometheusBuilder::new()
                .install_recorder()
                .expect("Prometheus recorder installation failed");
            Arc::new(handle)
        })
        .clone();

    // Install the tracing subscriber.  `try_init()` returns an error when a
    // subscriber has already been set (e.g., in a second test run) — we
    // silently ignore that error because the subscriber from the first call is
    // still active.
    let use_json = std::env::var("TOPGUN_LOG_FORMAT")
        .map(|v| v.eq_ignore_ascii_case("json"))
        .unwrap_or(false);

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));

    if use_json {
        let _ = tracing_subscriber::fmt()
            .json()
            .with_env_filter(filter)
            .try_init();
    } else {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .try_init();
    }

    ObservabilityHandle { prometheus }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_observability_returns_handle() {
        let handle = init_observability();
        // render_metrics returns a non-empty string (contains at least headers).
        // We only check it doesn't panic and returns a String.
        let _ = handle.render_metrics();
    }

    #[test]
    fn init_observability_is_idempotent() {
        // Calling twice must not panic.
        let h1 = init_observability();
        let h2 = init_observability();
        // Both handles render from the same underlying recorder.
        let _ = h1.render_metrics();
        let _ = h2.render_metrics();
    }

    #[test]
    fn render_metrics_returns_string() {
        let handle = init_observability();
        let output = handle.render_metrics();
        // Prometheus text format is always valid ASCII UTF-8.
        assert!(output.contains("# TYPE") || output.is_empty());
    }
}
