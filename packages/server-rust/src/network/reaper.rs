//! Connection reaper: a background task that evicts stale and abandoned
//! WebSocket connections so connection slots and per-connection resources do
//! not leak.
//!
//! Without a reaper the server has two lifecycle leaks:
//! - **Slowloris / never-authed:** a client that connects but never completes
//!   authentication (or dribbles malformed frames) parks the Phase-1 auth loop
//!   forever, holding a connection slot and a spawned outbound task.
//! - **Half-open:** a peer whose TCP died without a FIN is never detected; the
//!   read loop stays parked on `receiver.next()` until the OS TCP stack times
//!   out (which can be hours, or never behind a middlebox).
//!
//! The reaper closes the gap by periodically scanning the registry and
//! cancelling connections that have exceeded their timeout. Cancellation only
//! *signals* teardown — the read loop performs the actual resource cleanup via
//! its normal disconnect path, so a reaped connection releases exactly the same
//! lock/topic/counter/query/journal/search subscriptions as a cleanly
//! disconnected one.

use std::sync::Arc;
use std::time::Instant;

use tokio::task::JoinHandle;
use tracing::{debug, info};

use super::config::ConnectionConfig;
use super::connection::{ConnectionKind, ConnectionRegistry, OutboundMessage};

/// Tunables for the connection reaper, derived from [`ConnectionConfig`].
#[derive(Debug, Clone, Copy)]
pub struct ReaperConfig {
    /// Steady-state connections idle (no heartbeat) longer than this are reaped.
    pub idle_timeout: std::time::Duration,
    /// Connections still in the auth handshake older than this are reaped
    /// (slowloris guard).
    pub auth_timeout: std::time::Duration,
    /// Interval between registry scans.
    pub interval: std::time::Duration,
}

impl From<&ConnectionConfig> for ReaperConfig {
    fn from(c: &ConnectionConfig) -> Self {
        Self {
            idle_timeout: c.idle_timeout,
            auth_timeout: c.auth_timeout,
            interval: c.reaper_interval,
        }
    }
}

/// Spawns the connection reaper as a background task.
///
/// The returned [`JoinHandle`] should be aborted on server shutdown; the task
/// otherwise loops until aborted. Scanning an empty (drained) registry is
/// harmless, so abort ordering relative to connection drain does not matter.
#[must_use]
pub fn spawn_connection_reaper(
    registry: Arc<ConnectionRegistry>,
    config: ReaperConfig,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        info!(
            idle_timeout_ms = config.idle_timeout.as_millis(),
            auth_timeout_ms = config.auth_timeout.as_millis(),
            interval_ms = config.interval.as_millis(),
            "connection reaper started"
        );
        let mut ticker = tokio::time::interval(config.interval);
        // Skip the immediate first tick so the first scan happens one interval
        // in, not at startup when nothing is stale yet.
        ticker.tick().await;
        loop {
            ticker.tick().await;
            let reaped = reap_stale_once(&registry, config.idle_timeout, config.auth_timeout).await;
            if reaped > 0 {
                debug!(reaped, "connection reaper evicted stale connections");
            }
        }
    })
}

/// Scans the registry once and cancels every stale client connection.
///
/// A client connection is stale when either:
/// - it has **completed the handshake** (steady state) and its last heartbeat
///   is older than `idle_timeout` (half-open / dead peer), or
/// - it is **still in the handshake** and has existed longer than `auth_timeout`
///   (slowloris / abandoned handshake).
///
/// `handshake_complete` rather than `authenticated` is the discriminator so a
/// no-auth server (where connections are never `authenticated`) still applies
/// the idle bound to active connections instead of reaping them all at the
/// auth deadline.
///
/// Cluster-peer connections are skipped: their liveness is governed by the
/// cluster failure detector, not the client idle timeout.
///
/// Cancellation is idempotent, so a connection that was already cancelled (and
/// is still finishing its cleanup) is skipped rather than re-signalled. Returns
/// the number of connections newly cancelled by this scan.
pub async fn reap_stale_once(
    registry: &ConnectionRegistry,
    idle_timeout: std::time::Duration,
    auth_timeout: std::time::Duration,
) -> usize {
    let now = Instant::now();
    let mut reaped = 0;

    for handle in registry.connections() {
        if handle.kind != ConnectionKind::Client || handle.is_cancelled() {
            continue;
        }

        let (handshake_complete, last_heartbeat) = {
            let meta = handle.metadata.read().await;
            (meta.handshake_complete, meta.last_heartbeat)
        };

        let stale = if handshake_complete {
            now.duration_since(last_heartbeat) > idle_timeout
        } else {
            now.duration_since(handle.connected_at) > auth_timeout
        };

        if stale {
            // Best-effort Close so a still-reachable-but-idle client gets a
            // clean reason; the read-loop teardown (triggered by cancel) does
            // the authoritative socket close and resource release.
            let reason = if handshake_complete {
                "idle timeout"
            } else {
                "authentication timeout"
            };
            let _ = handle.try_send(OutboundMessage::Close(Some(reason.to_string())));
            handle.cancel();
            debug!(
                conn_id = ?handle.id,
                handshake_complete,
                "reaping stale connection: {reason}"
            );
            reaped += 1;
        }
    }

    reaped
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::network::config::ConnectionConfig;

    fn client_config() -> ConnectionConfig {
        ConnectionConfig::default()
    }

    /// An authenticated connection whose heartbeat is older than `idle_timeout`
    /// is cancelled.
    #[tokio::test]
    async fn reaps_idle_authenticated_connection() {
        let registry = ConnectionRegistry::new();
        let (handle, _rx) = registry.register(ConnectionKind::Client, &client_config());
        {
            let mut meta = handle.metadata.write().await;
            meta.handshake_complete = true;
            meta.last_heartbeat = Instant::now().checked_sub(Duration::from_secs(2)).unwrap();
        }

        let reaped = reap_stale_once(
            &registry,
            Duration::from_millis(500),
            Duration::from_secs(60),
        )
        .await;

        assert_eq!(reaped, 1);
        assert!(
            handle.is_cancelled(),
            "idle authed connection should be cancelled"
        );
    }

    /// Negative control: an authenticated connection with a fresh heartbeat is
    /// NOT reaped.
    #[tokio::test]
    async fn does_not_reap_active_authenticated_connection() {
        let registry = ConnectionRegistry::new();
        let (handle, _rx) = registry.register(ConnectionKind::Client, &client_config());
        {
            let mut meta = handle.metadata.write().await;
            meta.handshake_complete = true;
            meta.last_heartbeat = Instant::now();
        }

        let reaped = reap_stale_once(
            &registry,
            Duration::from_millis(500),
            Duration::from_secs(60),
        )
        .await;

        assert_eq!(reaped, 0);
        assert!(
            !handle.is_cancelled(),
            "fresh authed connection must survive"
        );
    }

    /// A never-authenticated connection older than `auth_timeout` is reaped
    /// (slowloris guard). `auth_timeout = 0` makes any non-zero age stale.
    #[tokio::test]
    async fn reaps_never_authenticated_past_auth_deadline() {
        let registry = ConnectionRegistry::new();
        let (handle, _rx) = registry.register(ConnectionKind::Client, &client_config());
        // Leave authenticated = false (default). Tiny sleep so connected_at is
        // strictly in the past relative to the scan.
        tokio::time::sleep(Duration::from_millis(5)).await;

        let reaped =
            reap_stale_once(&registry, Duration::from_secs(60), Duration::from_millis(1)).await;

        assert_eq!(reaped, 1);
        assert!(
            handle.is_cancelled(),
            "stale unauthed connection should be reaped"
        );
    }

    /// Negative control: a never-authenticated connection within `auth_timeout`
    /// is NOT reaped.
    #[tokio::test]
    async fn does_not_reap_unauthenticated_within_auth_deadline() {
        let registry = ConnectionRegistry::new();
        let (handle, _rx) = registry.register(ConnectionKind::Client, &client_config());

        let reaped =
            reap_stale_once(&registry, Duration::from_secs(60), Duration::from_secs(60)).await;

        assert_eq!(reaped, 0);
        assert!(
            !handle.is_cancelled(),
            "young unauthed connection must survive"
        );
    }

    /// Cluster-peer connections are never reaped by the client idle reaper.
    #[tokio::test]
    async fn does_not_reap_cluster_peer() {
        let registry = ConnectionRegistry::new();
        let (handle, _rx) = registry.register(ConnectionKind::ClusterPeer, &client_config());
        // Make it look maximally stale: unauthenticated and old.
        tokio::time::sleep(Duration::from_millis(5)).await;

        let reaped = reap_stale_once(
            &registry,
            Duration::from_millis(1),
            Duration::from_millis(1),
        )
        .await;

        assert_eq!(reaped, 0);
        assert!(!handle.is_cancelled(), "cluster peer must not be reaped");
    }

    /// A connection already cancelled (mid-cleanup) is not counted again.
    #[tokio::test]
    async fn skips_already_cancelled_connection() {
        let registry = ConnectionRegistry::new();
        let (handle, _rx) = registry.register(ConnectionKind::Client, &client_config());
        {
            let mut meta = handle.metadata.write().await;
            meta.handshake_complete = true;
            meta.last_heartbeat = Instant::now().checked_sub(Duration::from_secs(2)).unwrap();
        }
        handle.cancel();

        let reaped = reap_stale_once(
            &registry,
            Duration::from_millis(500),
            Duration::from_secs(60),
        )
        .await;

        assert_eq!(
            reaped, 0,
            "already-cancelled connection must not be re-counted"
        );
    }
}
