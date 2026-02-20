//! Graceful shutdown controller with in-flight request tracking.
//!
//! Uses `ArcSwap` for lock-free health state transitions and an atomic
//! counter with RAII guards for accurate in-flight request tracking.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use tokio::sync::watch;

/// Server health state, transitioned by the shutdown controller.
///
/// State machine: Starting -> Ready -> Draining -> Stopped
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthState {
    /// Server is initializing (not yet accepting requests).
    Starting,
    /// Server is fully operational and accepting requests.
    Ready,
    /// Server is draining in-flight requests (no new requests accepted).
    Draining,
    /// Server has fully stopped (all in-flight requests completed).
    Stopped,
}

/// Controls graceful shutdown with health state management and in-flight tracking.
///
/// The controller coordinates shutdown across the server:
/// 1. Health probes check `health_state()` to report readiness
/// 2. Middleware checks state before accepting new requests
/// 3. `trigger_shutdown()` moves to Draining and signals all listeners
/// 4. `wait_for_drain()` blocks until in-flight requests complete
#[derive(Debug)]
pub struct ShutdownController {
    shutdown_signal: watch::Sender<bool>,
    in_flight: Arc<AtomicU64>,
    health_state: Arc<ArcSwap<HealthState>>,
}

impl ShutdownController {
    /// Creates a new shutdown controller in the `Starting` state.
    #[must_use]
    pub fn new() -> Self {
        let (tx, _rx) = watch::channel(false);
        Self {
            shutdown_signal: tx,
            in_flight: Arc::new(AtomicU64::new(0)),
            health_state: Arc::new(ArcSwap::from_pointee(HealthState::Starting)),
        }
    }

    /// Transitions to the `Ready` state, indicating the server can accept requests.
    pub fn set_ready(&self) {
        self.health_state.store(Arc::new(HealthState::Ready));
    }

    /// Returns a receiver that will be notified when shutdown is triggered.
    ///
    /// Listeners should select on this receiver alongside their main loop
    /// to initiate graceful teardown.
    #[must_use]
    pub fn shutdown_receiver(&self) -> watch::Receiver<bool> {
        self.shutdown_signal.subscribe()
    }

    /// Initiates graceful shutdown.
    ///
    /// Transitions to `Draining` state and signals all shutdown receivers.
    /// After this, new requests should be rejected by middleware.
    pub fn trigger_shutdown(&self) {
        self.health_state.store(Arc::new(HealthState::Draining));
        // Ignore send errors -- receivers may have been dropped
        let _ = self.shutdown_signal.send(true);
    }

    /// Returns the current health state.
    #[must_use]
    pub fn health_state(&self) -> HealthState {
        **self.health_state.load()
    }

    /// Returns a shared handle to the health state for use by middleware/handlers.
    #[must_use]
    pub fn health_state_handle(&self) -> Arc<ArcSwap<HealthState>> {
        Arc::clone(&self.health_state)
    }

    /// Creates an RAII guard that tracks an in-flight request.
    ///
    /// The in-flight counter is incremented on creation and decremented
    /// when the guard is dropped, even if the handler panics.
    #[must_use]
    pub fn in_flight_guard(&self) -> InFlightGuard {
        self.in_flight.fetch_add(1, Ordering::Relaxed);
        InFlightGuard {
            in_flight: Arc::clone(&self.in_flight),
        }
    }

    /// Returns the current number of in-flight requests.
    #[must_use]
    pub fn in_flight_count(&self) -> u64 {
        self.in_flight.load(Ordering::Relaxed)
    }

    /// Waits for all in-flight requests to complete, up to the given timeout.
    ///
    /// Returns `true` if all requests drained successfully (transitions to
    /// `Stopped` state). Returns `false` if the timeout expired (state
    /// remains `Draining`).
    pub async fn wait_for_drain(&self, timeout: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            if self.in_flight.load(Ordering::Relaxed) == 0 {
                self.health_state.store(Arc::new(HealthState::Stopped));
                return true;
            }

            if tokio::time::Instant::now() >= deadline {
                return false;
            }

            // Poll at 10ms intervals to avoid busy-waiting
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

impl Default for ShutdownController {
    fn default() -> Self {
        Self::new()
    }
}

/// RAII guard that decrements the in-flight counter when dropped.
///
/// Ensures accurate in-flight tracking even if request handlers panic,
/// since Drop is called during stack unwinding.
#[derive(Debug)]
pub struct InFlightGuard {
    in_flight: Arc<AtomicU64>,
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.in_flight.fetch_sub(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_state_is_starting() {
        let controller = ShutdownController::new();
        assert_eq!(controller.health_state(), HealthState::Starting);
        assert_eq!(controller.in_flight_count(), 0);
    }

    #[test]
    fn set_ready_transitions_state() {
        let controller = ShutdownController::new();
        controller.set_ready();
        assert_eq!(controller.health_state(), HealthState::Ready);
    }

    #[test]
    fn trigger_shutdown_transitions_to_draining() {
        let controller = ShutdownController::new();
        controller.set_ready();
        controller.trigger_shutdown();
        assert_eq!(controller.health_state(), HealthState::Draining);
    }

    #[test]
    fn health_state_transitions_starting_ready_draining() {
        // AC5: correct state machine transitions
        let controller = ShutdownController::new();

        assert_eq!(controller.health_state(), HealthState::Starting);

        controller.set_ready();
        assert_eq!(controller.health_state(), HealthState::Ready);

        controller.trigger_shutdown();
        assert_eq!(controller.health_state(), HealthState::Draining);
    }

    #[test]
    fn in_flight_guard_increments_and_decrements() {
        let controller = ShutdownController::new();
        assert_eq!(controller.in_flight_count(), 0);

        let guard1 = controller.in_flight_guard();
        assert_eq!(controller.in_flight_count(), 1);

        let guard2 = controller.in_flight_guard();
        assert_eq!(controller.in_flight_count(), 2);

        drop(guard1);
        assert_eq!(controller.in_flight_count(), 1);

        drop(guard2);
        assert_eq!(controller.in_flight_count(), 0);
    }

    #[tokio::test]
    async fn shutdown_receiver_notified() {
        let controller = ShutdownController::new();
        let mut rx = controller.shutdown_receiver();

        // Not yet triggered
        assert!(!*rx.borrow());

        controller.trigger_shutdown();

        // Wait for the notification
        rx.changed().await.unwrap();
        assert!(*rx.borrow());
    }

    #[tokio::test]
    async fn wait_for_drain_immediate_success() {
        // AC5: Draining -> Stopped via wait_for_drain on successful drain
        let controller = ShutdownController::new();
        controller.set_ready();
        controller.trigger_shutdown();

        // No in-flight requests, should drain immediately
        let drained = controller.wait_for_drain(Duration::from_secs(1)).await;
        assert!(drained);
        assert_eq!(controller.health_state(), HealthState::Stopped);
    }

    #[tokio::test]
    async fn wait_for_drain_with_active_requests() {
        let controller = ShutdownController::new();
        controller.set_ready();

        let guard = controller.in_flight_guard();
        controller.trigger_shutdown();

        // Spawn a task that drops the guard after a short delay
        let guard_handle = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            drop(guard);
        });

        let drained = controller.wait_for_drain(Duration::from_secs(2)).await;
        assert!(drained);
        assert_eq!(controller.health_state(), HealthState::Stopped);

        guard_handle.await.unwrap();
    }

    #[tokio::test]
    async fn wait_for_drain_timeout() {
        let controller = ShutdownController::new();
        controller.set_ready();

        let _guard = controller.in_flight_guard();
        controller.trigger_shutdown();

        // Very short timeout -- should fail because guard is still held
        let drained = controller.wait_for_drain(Duration::from_millis(50)).await;
        assert!(!drained);
        // State should remain Draining on timeout
        assert_eq!(controller.health_state(), HealthState::Draining);
    }

    #[test]
    fn health_state_handle_shares_state() {
        let controller = ShutdownController::new();
        let handle = controller.health_state_handle();

        assert_eq!(**handle.load(), HealthState::Starting);

        controller.set_ready();
        assert_eq!(**handle.load(), HealthState::Ready);
    }
}
