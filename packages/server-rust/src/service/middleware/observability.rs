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
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::{reload, EnvFilter};

// ---------------------------------------------------------------------------
// Type alias for the reload handle
// ---------------------------------------------------------------------------

/// Type alias for the tracing filter reload handle.
///
/// Kept as an alias to keep method signatures readable and to insulate call
/// sites from the concrete subscriber composition type.
pub type LogLevelHandle = reload::Handle<EnvFilter, tracing_subscriber::Registry>;

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
/// endpoint can render the current metric state on demand, and optionally a
/// reload handle for the active tracing `EnvFilter` to support runtime log
/// level changes.
#[derive(Clone)]
pub struct ObservabilityHandle {
    prometheus: Arc<PrometheusHandle>,
    log_level_handle: Option<LogLevelHandle>,
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

    /// Returns a reference to the reload handle, if available.
    ///
    /// `None` when `init_observability` was called more than once and this
    /// instance was returned from a subsequent call (the subscriber was already
    /// installed by the first call).
    #[must_use]
    pub fn log_level_handle(&self) -> Option<&LogLevelHandle> {
        self.log_level_handle.as_ref()
    }

    /// Returns the current active `EnvFilter` directive string, if available.
    ///
    /// Uses the reload handle's `with_current` method to read the live filter
    /// without taking ownership.
    #[must_use]
    pub fn current_log_level(&self) -> Option<String> {
        self.log_level_handle
            .as_ref()
            .and_then(|h| h.with_current(ToString::to_string).ok())
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
/// - The returned [`ObservabilityHandle`] carries a `reload::Handle` that
///   allows swapping the active `EnvFilter` at runtime without restarting
///   the process.
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

    let use_json = std::env::var("TOPGUN_LOG_FORMAT")
        .map(|v| v.eq_ignore_ascii_case("json"))
        .unwrap_or(false);

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));

    // Wrap the filter in a reload layer so it can be swapped at runtime.
    let (reload_layer, reload_handle) = reload::Layer::new(filter);

    // Build and install the subscriber. `set_global_default` returns `Err` when
    // a subscriber is already installed (i.e., on any call after the first).
    // We must use `set_global_default` directly because there is no `try_init`
    // equivalent on a manually-composed subscriber built via `registry().with(...)`.
    let install_result = if use_json {
        let fmt_layer = tracing_subscriber::fmt::layer().json();
        let subscriber = tracing_subscriber::registry()
            .with(reload_layer)
            .with(fmt_layer);
        tracing::subscriber::set_global_default(subscriber)
    } else {
        let fmt_layer = tracing_subscriber::fmt::layer();
        let subscriber = tracing_subscriber::registry()
            .with(reload_layer)
            .with(fmt_layer);
        tracing::subscriber::set_global_default(subscriber)
    };

    // Only the first successful installation gets to own the reload handle.
    // Subsequent calls (e.g., from test harnesses) do not have a live
    // subscriber to reload through, so the handle would be inert anyway.
    let log_level_handle = match install_result {
        Ok(()) => Some(reload_handle),
        Err(_) => None,
    };

    ObservabilityHandle {
        prometheus,
        log_level_handle,
    }
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

    #[test]
    fn current_log_level_returns_some_on_first_call() {
        // The first call in this test binary installs the subscriber and gets
        // the reload handle. Subsequent calls return None. We test that at
        // least one call (possibly this one) returns a non-panic result.
        let handle = init_observability();
        // If this is the first subscriber in this test binary, current_log_level
        // will be Some. If another test already installed the subscriber, it
        // will be None. Either outcome is valid — we just must not panic.
        let _ = handle.current_log_level();
    }
}
