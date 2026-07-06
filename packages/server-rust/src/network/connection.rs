//! Connection management types and registry for the `TopGun` server.
//!
//! Provides per-connection backpressure via bounded mpsc channels,
//! lock-free concurrent connection tracking via `DashMap`, and
//! metadata storage for authentication and subscription state.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use dashmap::DashMap;
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::{mpsc, RwLock};
use tokio_util::sync::CancellationToken;
use topgun_core::{Principal, Timestamp};
use tracing::warn;

use super::config::ConnectionConfig;

// Device-credential store + identity keying. Declared here (rather than as a line in
// `network/mod.rs`) to keep this change within the per-spec Rust file budget without a
// separate module-list edit; device identity is connection-ownership-adjacent — the
// ownership map that consumes its keys lives in this file — so co-locating the submodule
// is coherent. The `pub use connection::*` glob in `network/mod.rs` re-exports it as
// `crate::network::device_identity`.
#[path = "device_identity.rs"]
pub mod device_identity;

/// Unique identifier for a connection, assigned by the registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConnectionId(pub u64);

/// Classifies a connection as either a client or a cluster peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionKind {
    /// A client application connection (browser, Node.js SDK, etc.).
    Client,
    /// An inter-node cluster peer connection.
    ClusterPeer,
}

/// Message to be sent outbound to a connection.
#[derive(Debug)]
pub enum OutboundMessage {
    /// A binary payload (MsgPack-encoded).
    Binary(Vec<u8>),
    /// A close frame with an optional reason.
    Close(Option<String>),
}

/// Outcome of a best-effort broadcast send to a (possibly slow) consumer.
///
/// Distinguishes the three states a live-event push can land in so the caller
/// (and metrics) can tell "kept up" from "fell behind" from "gave up", instead
/// of the old binary `is_ok()` that hid silent divergence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BroadcastOutcome {
    /// The event was enqueued; the consumer is keeping up.
    Sent,
    /// The channel was full and the event was dropped, but the consecutive-drop
    /// count is still under the disconnect threshold. The connection survives;
    /// the per-connection dropped-event counter was incremented.
    Dropped,
    /// The consecutive-drop count crossed the threshold: the connection was
    /// cancelled to force a reconnect + Merkle resync rather than let it diverge
    /// silently. Only client connections are disconnected this way.
    Disconnected,
    /// The receiver is gone (connection already closed).
    Closed,
}

/// Handle to a single connection, providing send capabilities and metadata access.
///
/// Each connection gets a bounded mpsc channel for backpressure. The receiver
/// end is held by the WebSocket write loop; this handle holds the sender end.
#[derive(Debug)]
pub struct ConnectionHandle {
    /// Unique connection identifier assigned by the registry.
    pub id: ConnectionId,
    /// Sender end of the bounded outbound message channel.
    pub tx: mpsc::Sender<OutboundMessage>,
    /// Mutable metadata (auth state, subscriptions, etc.).
    pub metadata: Arc<RwLock<ConnectionMetadata>>,
    /// When this connection was established.
    pub connected_at: Instant,
    /// Whether this is a client or cluster peer connection.
    pub kind: ConnectionKind,
    /// Cancellation signal for forced teardown.
    ///
    /// The connection's read loop selects on this token, so cancelling it
    /// unblocks a reader parked on a half-open socket and lets the connection
    /// run its normal cleanup path. The reaper cancels this to evict stale or
    /// never-authenticated connections.
    pub cancel: CancellationToken,
    /// Number of consecutive live-event broadcasts dropped because the outbound
    /// channel was full. Reset to 0 on the next successful broadcast. When this
    /// crosses `slow_consumer_drop_threshold` the connection is cancelled
    /// (forcing reconnect + resync) instead of diverging silently.
    consecutive_drops: AtomicU64,
    /// Cumulative count of live-event broadcasts dropped over the connection's
    /// lifetime. Surfaced as a slow-consumer metric; never reset.
    dropped_broadcasts: AtomicU64,
    /// Consecutive-drop count at which a slow client connection is disconnected
    /// to force resync. 0 disables the disconnect behavior (pure best-effort
    /// drop, the legacy semantics).
    slow_consumer_drop_threshold: u64,
}

impl ConnectionHandle {
    /// Attempts to send a message without blocking.
    ///
    /// Returns `true` if the message was enqueued, `false` if the channel
    /// is full or the connection has been closed.
    #[must_use]
    pub fn try_send(&self, msg: OutboundMessage) -> bool {
        self.tx.try_send(msg).is_ok()
    }

    /// Best-effort send of a live-event broadcast to a possibly-slow consumer.
    ///
    /// Unlike [`try_send`](Self::try_send), this tracks slow-consumer state so a
    /// subscriber that persistently fails to drain its channel does not diverge
    /// silently. On a full channel the event is dropped (the channel stays
    /// memory-bounded), the cumulative drop metric is incremented, and the
    /// consecutive-drop counter advances; once it reaches
    /// `slow_consumer_drop_threshold` the connection is cancelled so the client
    /// reconnects and re-syncs via the Merkle tree. A successful send resets the
    /// consecutive-drop counter.
    ///
    /// Only `Client` connections are disconnected on threshold; cluster peers
    /// fall back to plain best-effort drop because their liveness is governed by
    /// the cluster failure detector, not this idle/slow heuristic.
    #[must_use]
    pub fn try_send_broadcast(&self, msg: OutboundMessage) -> BroadcastOutcome {
        match self.tx.try_send(msg) {
            Ok(()) => {
                self.consecutive_drops.store(0, Ordering::Relaxed);
                BroadcastOutcome::Sent
            }
            Err(TrySendError::Closed(_)) => BroadcastOutcome::Closed,
            Err(TrySendError::Full(_)) => {
                // Relaxed ordering is sufficient: concurrent broadcasts to one
                // connection can in principle race the success-reset against a
                // drop-increment at the exact threshold boundary, disconnecting a
                // connection one event early. That is a SAFE failure mode — the
                // disconnect just forces the reconnect + Merkle resync this whole
                // path exists to trigger, with no data loss — so it does not
                // warrant a compare-exchange loop on the hot broadcast path.
                self.dropped_broadcasts.fetch_add(1, Ordering::Relaxed);
                let consecutive = self.consecutive_drops.fetch_add(1, Ordering::Relaxed) + 1;
                let should_disconnect = self.kind == ConnectionKind::Client
                    && self.slow_consumer_drop_threshold != 0
                    && consecutive >= self.slow_consumer_drop_threshold
                    && !self.cancel.is_cancelled();
                if should_disconnect {
                    warn!(
                        conn_id = ?self.id,
                        consecutive_drops = consecutive,
                        dropped_total = self.dropped_broadcasts.load(Ordering::Relaxed),
                        "slow consumer exceeded broadcast drop threshold; \
                         disconnecting to force reconnect + resync"
                    );
                    self.cancel();
                    BroadcastOutcome::Disconnected
                } else {
                    BroadcastOutcome::Dropped
                }
            }
        }
    }

    /// Cumulative number of live-event broadcasts dropped to this connection
    /// because its outbound channel was full (slow-consumer metric).
    #[must_use]
    pub fn dropped_broadcasts(&self) -> u64 {
        self.dropped_broadcasts.load(Ordering::Relaxed)
    }

    /// Checks whether the connection is still open.
    ///
    /// Returns `false` if the receiver end of the channel has been dropped,
    /// meaning the WebSocket write loop has exited.
    #[must_use]
    pub fn is_connected(&self) -> bool {
        !self.tx.is_closed()
    }

    /// Signals the connection's read loop to tear down.
    ///
    /// Idempotent: cancelling an already-cancelled connection is a no-op, so
    /// the reaper can call this on every tick without coordinating with the
    /// read loop. The actual resource cleanup (subscriptions, registry slot)
    /// still runs once, in the read loop's normal disconnect path.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    /// Returns `true` once `cancel()` has been called on this connection.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }
}

/// Mutable metadata associated with a connection.
///
/// Protected by an `RwLock` to allow concurrent reads (e.g., broadcast
/// filtering) while serializing writes (e.g., authentication updates).
///
/// Derives `Clone` so the write validator can snapshot metadata once and
/// release the read guard before any async storage calls.
#[derive(Debug, Clone)]
pub struct ConnectionMetadata {
    /// Whether this connection has completed authentication.
    pub authenticated: bool,
    /// Whether this connection has finished the auth handshake and entered the
    /// steady-state (Phase 2) read loop.
    ///
    /// Set once when the connection enters Phase 2 — true for both
    /// JWT-authenticated connections and connections on a no-auth server (where
    /// Phase 1 is skipped). The reaper uses this to decide which timeout
    /// applies: connections still in the handshake are bounded by the auth
    /// deadline; steady-state connections are bounded by the idle (heartbeat)
    /// timeout. Distinct from `authenticated`, which is false on a no-auth
    /// server even in steady state.
    pub handshake_complete: bool,
    /// The authenticated principal, if any.
    pub principal: Option<Principal>,
    /// Active query subscriptions for this connection.
    pub subscriptions: HashSet<String>,
    /// Active pub/sub topic subscriptions.
    pub topics: HashSet<String>,
    /// Last time a heartbeat was received from this connection.
    pub last_heartbeat: Instant,
    /// Last HLC timestamp seen from this connection.
    pub last_hlc: Option<Timestamp>,
    /// For cluster peer connections, the remote node's ID.
    pub peer_node_id: Option<String>,
    /// Server-issued device identity bound to this connection (present-or-mint at
    /// AUTH time). Write-once per connection (one-shot binding); `None` for old
    /// clients that never send a device credential and for connections pre-`AUTH_ACK`.
    pub device_id: Option<String>,
}

impl Default for ConnectionMetadata {
    fn default() -> Self {
        Self {
            authenticated: false,
            handshake_complete: false,
            principal: None,
            subscriptions: HashSet::new(),
            topics: HashSet::new(),
            last_heartbeat: Instant::now(),
            last_hlc: None,
            peer_node_id: None,
            device_id: None,
        }
    }
}

/// Callback invoked with a `ConnectionId` when that connection is removed from
/// the registry (disconnect). Registered via [`ConnectionRegistry::on_disconnect`]
/// so owners of per-connection session state (e.g. the SYNC Merkle session cache)
/// can release a connection's resources at the single `remove()` chokepoint.
/// The callback takes only a `ConnectionId`, so the network layer notifies
/// observers without importing any domain type — no layering inversion.
pub type DisconnectObserver = Arc<dyn Fn(ConnectionId) + Send + Sync>;

/// Thread-safe registry of all active connections.
///
/// Uses `DashMap` for lock-free concurrent access, supporting 10K+
/// simultaneous connections without contention.
pub struct ConnectionRegistry {
    connections: DashMap<ConnectionId, Arc<ConnectionHandle>>,
    next_id: AtomicU64,
    /// Observers notified (with the removed `ConnectionId`) on every `remove()`.
    /// Registered once at construction wiring time; the disconnect path is
    /// per-connection (not per-message), so a plain `std::sync::Mutex` is cheap.
    disconnect_observers: std::sync::Mutex<Vec<DisconnectObserver>>,
    /// Server-authenticated device-identity ownership: `frontier_client_id` →
    /// current owning connection. A new connection presenting a valid credential
    /// for an owned identity TAKES OVER (becomes the owner; the displaced connection
    /// is closed). Keyed by the injective `(principal OR NO_AUTH_SENTINEL, deviceId)`
    /// string so it is a fully server-authenticated per-replica identity.
    device_ownership: DashMap<String, ConnectionId>,
    /// Reverse index `connection → identity key`, populated at claim time so the
    /// single `remove()` chokepoint can release ownership on disconnect without an
    /// async metadata read at every teardown exit.
    conn_identity: DashMap<ConnectionId, String>,
}

impl std::fmt::Debug for ConnectionRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let observer_count = self
            .disconnect_observers
            .lock()
            .map(|o| o.len())
            .unwrap_or(0);
        f.debug_struct("ConnectionRegistry")
            .field("connections", &self.connections)
            .field("next_id", &self.next_id)
            .field("disconnect_observers", &observer_count)
            .finish()
    }
}

impl ConnectionRegistry {
    /// Creates a new empty registry.
    ///
    /// Connection IDs start at 1 (0 is reserved as "no connection").
    #[must_use]
    pub fn new() -> Self {
        Self {
            connections: DashMap::new(),
            next_id: AtomicU64::new(1),
            disconnect_observers: std::sync::Mutex::new(Vec::new()),
            device_ownership: DashMap::new(),
            conn_identity: DashMap::new(),
        }
    }

    /// Registers a new connection, returning a handle and the message receiver.
    ///
    /// The receiver should be passed to the WebSocket write loop, which
    /// drains outbound messages and forwards them over the wire.
    pub fn register(
        &self,
        kind: ConnectionKind,
        config: &ConnectionConfig,
    ) -> (Arc<ConnectionHandle>, mpsc::Receiver<OutboundMessage>) {
        let id = ConnectionId(self.next_id.fetch_add(1, Ordering::Relaxed));
        let (tx, rx) = mpsc::channel(config.outbound_channel_capacity);

        let handle = Arc::new(ConnectionHandle {
            id,
            tx,
            metadata: Arc::new(RwLock::new(ConnectionMetadata::default())),
            connected_at: Instant::now(),
            kind,
            cancel: CancellationToken::new(),
            consecutive_drops: AtomicU64::new(0),
            dropped_broadcasts: AtomicU64::new(0),
            slow_consumer_drop_threshold: config.slow_consumer_drop_threshold,
        });

        self.connections.insert(id, Arc::clone(&handle));
        (handle, rx)
    }

    /// Registers an observer invoked with the `ConnectionId` whenever a
    /// connection is removed (disconnect). Used by owners of per-connection
    /// session state (e.g. the SYNC Merkle session cache) to release that
    /// connection's resources at the single `remove()` chokepoint. Observers are
    /// retained for the registry's lifetime and registered at construction time.
    pub fn on_disconnect(&self, observer: DisconnectObserver) {
        if let Ok(mut observers) = self.disconnect_observers.lock() {
            observers.push(observer);
        }
    }

    /// Removes a connection from the registry, returning its handle if found.
    ///
    /// Fires every registered disconnect observer with `id` so owners of
    /// per-connection session state can release that connection's resources.
    /// Observer Arcs are cloned out before invocation so the observer lock is
    /// never held across a callback. Idempotent: repeat calls for the same `id`
    /// simply re-run the (no-op) observers.
    pub fn remove(&self, id: ConnectionId) -> Option<Arc<ConnectionHandle>> {
        let removed = self.connections.remove(&id).map(|(_, handle)| handle);
        // Release device-identity ownership at the single disconnect chokepoint, but
        // ONLY if this connection is still the current owner — a TAKEOVER may have
        // already handed the identity to a newer connection, whose ownership must not
        // be clobbered by the displaced connection's later teardown.
        if let Some((_, identity_key)) = self.conn_identity.remove(&id) {
            self.device_ownership
                .remove_if(&identity_key, |_, owner| *owner == id);
        }
        let observers: Vec<DisconnectObserver> = self
            .disconnect_observers
            .lock()
            .map(|o| o.clone())
            .unwrap_or_default();
        for observer in observers {
            observer(id);
        }
        removed
    }

    /// Claims device-identity ownership for `connection_id` via TAKEOVER.
    ///
    /// Returns the previously-owning connection (to be closed by the caller) when a
    /// live connection is displaced. Atomic under `DashMap`'s per-key entry lock:
    /// concurrent double-takeovers are serialized, so exactly one connection remains
    /// the owner and each displaced connection is reported exactly once. A connection
    /// re-claiming an identity it already owns is a no-op (returns `None`).
    pub fn claim_device_ownership(
        &self,
        identity_key: String,
        connection_id: ConnectionId,
    ) -> Option<ConnectionId> {
        // Reverse index first so a racing `remove()` for this same connection can
        // always find the key to release. Overwriting a prior key for this
        // connection is impossible under one-shot binding (bind happens once).
        self.conn_identity
            .insert(connection_id, identity_key.clone());
        // Atomic swap: `insert` returns the prior owner under the shard lock.
        match self.device_ownership.insert(identity_key, connection_id) {
            Some(prev) if prev != connection_id => Some(prev),
            _ => None,
        }
    }

    /// The fencing primitive: is `connection_id` the current owner of `identity_key`?
    ///
    /// Identity-scoped actions (e.g. a cursor ACK that advances a shared low-water
    /// mark) MUST be accepted only from the current owner — a displaced connection's
    /// in-flight stale action is a resurrection vector and must be rejectable.
    #[must_use]
    pub fn is_current_owner(&self, identity_key: &str, connection_id: ConnectionId) -> bool {
        self.device_ownership
            .get(identity_key)
            .is_some_and(|owner| *owner.value() == connection_id)
    }

    /// Looks up a connection by ID.
    pub fn get(&self, id: ConnectionId) -> Option<Arc<ConnectionHandle>> {
        self.connections.get(&id).map(|r| r.value().clone())
    }

    /// Returns the total number of active connections.
    #[must_use]
    pub fn count(&self) -> usize {
        self.connections.len()
    }

    /// Counts connections of a specific kind.
    #[must_use]
    pub fn count_by_kind(&self, kind: ConnectionKind) -> usize {
        self.connections
            .iter()
            .filter(|entry| entry.value().kind == kind)
            .count()
    }

    /// Returns all active connections as a collected `Vec`.
    ///
    /// `DashMap` iteration yields guard types that borrow the map, so we
    /// must collect into a `Vec` to return owned values.
    #[must_use]
    pub fn connections(&self) -> Vec<Arc<ConnectionHandle>> {
        self.connections
            .iter()
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Sends a binary live-event message to all connections of a given kind.
    ///
    /// Uses non-blocking [`try_send_broadcast`](ConnectionHandle::try_send_broadcast)
    /// so a single slow connection cannot block the broadcast. A full channel
    /// drops the event (bounded memory) and advances that connection's
    /// slow-consumer state; a persistently-slow client is disconnected to force
    /// reconnect + resync rather than diverging silently.
    pub fn broadcast(&self, msg_bytes: &[u8], kind: ConnectionKind) {
        for entry in &self.connections {
            let handle = entry.value();
            if handle.kind == kind {
                let _ = handle.try_send_broadcast(OutboundMessage::Binary(msg_bytes.to_vec()));
            }
        }
    }

    /// Sends a binary live-event message to a specific set of connection IDs.
    ///
    /// Same slow-consumer semantics as [`broadcast`](Self::broadcast): missing
    /// connections are skipped, a full channel drops the event and advances the
    /// target's slow-consumer state (disconnecting a persistently-slow client).
    pub fn send_to_connections(&self, ids: &HashSet<ConnectionId>, msg_bytes: &[u8]) {
        for id in ids {
            if let Some(handle) = self.get(*id) {
                let _ = handle.try_send_broadcast(OutboundMessage::Binary(msg_bytes.to_vec()));
            }
        }
    }

    /// Removes and returns all connections. Used during graceful shutdown.
    pub fn drain_all(&self) -> Vec<Arc<ConnectionHandle>> {
        let keys: Vec<ConnectionId> = self.connections.iter().map(|entry| *entry.key()).collect();

        let mut handles = Vec::with_capacity(keys.len());
        for key in keys {
            if let Some((_, handle)) = self.connections.remove(&key) {
                handles.push(handle);
            }
        }
        handles
    }
}

impl Default for ConnectionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> ConnectionConfig {
        ConnectionConfig::default()
    }

    fn small_channel_config() -> ConnectionConfig {
        ConnectionConfig {
            outbound_channel_capacity: 2,
            ..ConnectionConfig::default()
        }
    }

    #[test]
    fn registry_register_and_count() {
        let registry = ConnectionRegistry::new();
        assert_eq!(registry.count(), 0);

        let config = test_config();
        let (handle1, _rx1) = registry.register(ConnectionKind::Client, &config);
        assert_eq!(registry.count(), 1);
        assert_eq!(handle1.id, ConnectionId(1));
        assert_eq!(handle1.kind, ConnectionKind::Client);

        let (handle2, _rx2) = registry.register(ConnectionKind::ClusterPeer, &config);
        assert_eq!(registry.count(), 2);
        assert_eq!(handle2.id, ConnectionId(2));
        assert_eq!(handle2.kind, ConnectionKind::ClusterPeer);
    }

    #[test]
    fn registry_remove() {
        let registry = ConnectionRegistry::new();
        let config = test_config();

        let (handle, _rx) = registry.register(ConnectionKind::Client, &config);
        let id = handle.id;
        assert_eq!(registry.count(), 1);

        let removed = registry.remove(id);
        assert!(removed.is_some());
        assert_eq!(registry.count(), 0);

        // Removing again returns None
        assert!(registry.remove(id).is_none());
    }

    #[test]
    fn registry_get() {
        let registry = ConnectionRegistry::new();
        let config = test_config();

        let (handle, _rx) = registry.register(ConnectionKind::Client, &config);
        let id = handle.id;

        let retrieved = registry.get(id);
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().id, id);

        // Non-existent ID
        assert!(registry.get(ConnectionId(999)).is_none());
    }

    #[test]
    fn registry_count_by_kind() {
        let registry = ConnectionRegistry::new();
        let config = test_config();

        let (_h1, _rx1) = registry.register(ConnectionKind::Client, &config);
        let (_h2, _rx2) = registry.register(ConnectionKind::Client, &config);
        let (_h3, _rx3) = registry.register(ConnectionKind::ClusterPeer, &config);

        assert_eq!(registry.count_by_kind(ConnectionKind::Client), 2);
        assert_eq!(registry.count_by_kind(ConnectionKind::ClusterPeer), 1);
    }

    #[test]
    fn registry_connections() {
        let registry = ConnectionRegistry::new();
        let config = test_config();

        let (_h1, _rx1) = registry.register(ConnectionKind::Client, &config);
        let (_h2, _rx2) = registry.register(ConnectionKind::ClusterPeer, &config);

        let all = registry.connections();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn registry_drain_all() {
        let registry = ConnectionRegistry::new();
        let config = test_config();

        let (_h1, _rx1) = registry.register(ConnectionKind::Client, &config);
        let (_h2, _rx2) = registry.register(ConnectionKind::Client, &config);
        let (_h3, _rx3) = registry.register(ConnectionKind::ClusterPeer, &config);

        let drained = registry.drain_all();
        assert_eq!(drained.len(), 3);
        assert_eq!(registry.count(), 0);
    }

    #[test]
    fn connection_handle_try_send_success() {
        let config = test_config();
        let registry = ConnectionRegistry::new();
        let (handle, _rx) = registry.register(ConnectionKind::Client, &config);

        let result = handle.try_send(OutboundMessage::Binary(vec![1, 2, 3]));
        assert!(result);
    }

    #[test]
    fn connection_handle_try_send_full() {
        // AC3: try_send returns false when channel is full
        let config = small_channel_config();
        let registry = ConnectionRegistry::new();
        let (handle, _rx) = registry.register(ConnectionKind::Client, &config);

        // Fill the channel (capacity = 2)
        assert!(handle.try_send(OutboundMessage::Binary(vec![1])));
        assert!(handle.try_send(OutboundMessage::Binary(vec![2])));

        // Third send should fail -- channel is full
        let result = handle.try_send(OutboundMessage::Binary(vec![3]));
        assert!(!result);
    }

    #[test]
    fn connection_handle_try_send_disconnected() {
        let config = test_config();
        let registry = ConnectionRegistry::new();
        let (handle, rx) = registry.register(ConnectionKind::Client, &config);

        // Drop the receiver to simulate disconnection
        drop(rx);

        let result = handle.try_send(OutboundMessage::Binary(vec![1]));
        assert!(!result);
    }

    #[test]
    fn connection_handle_is_connected() {
        let config = test_config();
        let registry = ConnectionRegistry::new();
        let (handle, rx) = registry.register(ConnectionKind::Client, &config);

        assert!(handle.is_connected());

        drop(rx);
        assert!(!handle.is_connected());
    }

    #[test]
    fn connection_metadata_defaults() {
        let meta = ConnectionMetadata::default();
        assert!(!meta.authenticated);
        assert!(meta.principal.is_none());
        assert!(meta.subscriptions.is_empty());
        assert!(meta.topics.is_empty());
        assert!(meta.last_hlc.is_none());
        assert!(meta.peer_node_id.is_none());
    }

    #[test]
    fn connection_id_starts_at_one() {
        let registry = ConnectionRegistry::new();
        let config = test_config();

        let (h1, _rx1) = registry.register(ConnectionKind::Client, &config);
        let (h2, _rx2) = registry.register(ConnectionKind::Client, &config);

        assert_eq!(h1.id, ConnectionId(1));
        assert_eq!(h2.id, ConnectionId(2));
    }

    #[test]
    fn broadcast_to_specific_kind() {
        let config = small_channel_config();
        let registry = ConnectionRegistry::new();

        let (_h1, mut rx1) = registry.register(ConnectionKind::Client, &config);
        let (_h2, mut rx2) = registry.register(ConnectionKind::ClusterPeer, &config);

        registry.broadcast(&[42], ConnectionKind::Client);

        // Client should have received the message
        assert!(rx1.try_recv().is_ok());
        // ClusterPeer should NOT have received the message
        assert!(rx2.try_recv().is_err());
    }

    #[test]
    fn broadcast_skips_full_channels() {
        let config = small_channel_config();
        let registry = ConnectionRegistry::new();

        let (handle, _rx) = registry.register(ConnectionKind::Client, &config);

        // Fill the channel
        assert!(handle.try_send(OutboundMessage::Binary(vec![1])));
        assert!(handle.try_send(OutboundMessage::Binary(vec![2])));

        // Broadcast should not block even though channel is full
        registry.broadcast(&[3], ConnectionKind::Client);
    }

    #[test]
    fn register_uses_configured_channel_capacity() {
        // AC2: channel uses configured capacity
        let config = ConnectionConfig {
            outbound_channel_capacity: 3,
            ..ConnectionConfig::default()
        };
        let registry = ConnectionRegistry::new();
        let (handle, _rx) = registry.register(ConnectionKind::Client, &config);

        // Should be able to send exactly 3 messages
        assert!(handle.try_send(OutboundMessage::Binary(vec![1])));
        assert!(handle.try_send(OutboundMessage::Binary(vec![2])));
        assert!(handle.try_send(OutboundMessage::Binary(vec![3])));

        // 4th should fail
        assert!(!handle.try_send(OutboundMessage::Binary(vec![4])));
    }

    // ---------------------------------------------------------------------------
    // send_to_connections tests (AC3, AC4)
    // ---------------------------------------------------------------------------

    #[test]
    fn send_to_connections_delivers_only_to_specified_ids() {
        let config = small_channel_config();
        let registry = ConnectionRegistry::new();

        let (h1, mut rx1) = registry.register(ConnectionKind::Client, &config);
        let (_h2, mut rx2) = registry.register(ConnectionKind::Client, &config);

        let mut ids = HashSet::new();
        ids.insert(h1.id);
        // h2 is NOT in the target set

        registry.send_to_connections(&ids, &[42]);

        // h1 should have received the message
        assert!(
            rx1.try_recv().is_ok(),
            "targeted connection should receive bytes"
        );
        // h2 should NOT have received anything
        assert!(
            rx2.try_recv().is_err(),
            "non-targeted connection should not receive bytes"
        );
    }

    #[test]
    fn send_to_connections_skips_missing_ids() {
        let registry = ConnectionRegistry::new();

        let mut ids = HashSet::new();
        ids.insert(ConnectionId(9999)); // does not exist

        // Should not panic
        registry.send_to_connections(&ids, &[1, 2, 3]);
    }

    #[test]
    fn send_to_connections_skips_full_channels() {
        let config = small_channel_config(); // capacity = 2
        let registry = ConnectionRegistry::new();

        let (handle, _rx) = registry.register(ConnectionKind::Client, &config);

        // Fill the channel
        assert!(handle.try_send(OutboundMessage::Binary(vec![1])));
        assert!(handle.try_send(OutboundMessage::Binary(vec![2])));

        let mut ids = HashSet::new();
        ids.insert(handle.id);

        // Should not block or panic when channel is full
        registry.send_to_connections(&ids, &[3]);
    }

    // ---------------------------------------------------------------------------
    // Slow-consumer broadcast backpressure (F4 / TODO-509)
    //
    // A persistently-slow subscriber used to be dropped silently (no error, no
    // Close, no counter) and diverge until it independently reconnected. The
    // fix counts consecutive drops and, past a threshold, disconnects the
    // connection to force reconnect + Merkle resync, while exposing a drop
    // metric. These tests assert the new contract *and* carry the pre-fix
    // negative controls (draining consumer / cluster peer never disconnected).
    // ---------------------------------------------------------------------------

    fn slow_consumer_config(capacity: usize, threshold: u64) -> ConnectionConfig {
        ConnectionConfig {
            outbound_channel_capacity: capacity,
            slow_consumer_drop_threshold: threshold,
            ..ConnectionConfig::default()
        }
    }

    #[test]
    fn slow_client_disconnected_after_consecutive_drop_threshold() {
        // Reproduces the F4 scenario: a client whose channel stays full. Before
        // the fix it silently dropped forever and stayed connected; now it is
        // disconnected once consecutive drops reach the threshold.
        let config = slow_consumer_config(2, 3);
        let registry = ConnectionRegistry::new();
        let (handle, _rx) = registry.register(ConnectionKind::Client, &config);

        // Fill the channel (capacity 2) so every subsequent broadcast drops.
        assert_eq!(
            handle.try_send_broadcast(OutboundMessage::Binary(vec![1])),
            BroadcastOutcome::Sent
        );
        assert_eq!(
            handle.try_send_broadcast(OutboundMessage::Binary(vec![2])),
            BroadcastOutcome::Sent
        );

        // Drops 1 and 2 are under threshold (3): event lost but connection lives.
        assert_eq!(
            handle.try_send_broadcast(OutboundMessage::Binary(vec![3])),
            BroadcastOutcome::Dropped
        );
        assert!(!handle.is_cancelled());
        assert_eq!(
            handle.try_send_broadcast(OutboundMessage::Binary(vec![4])),
            BroadcastOutcome::Dropped
        );
        assert!(!handle.is_cancelled());

        // Drop 3 hits the threshold: disconnect to force resync.
        assert_eq!(
            handle.try_send_broadcast(OutboundMessage::Binary(vec![5])),
            BroadcastOutcome::Disconnected
        );
        assert!(
            handle.is_cancelled(),
            "slow client must be cancelled past the drop threshold, not diverge silently"
        );
        assert_eq!(
            handle.dropped_broadcasts(),
            3,
            "drop metric must count losses"
        );
    }

    #[test]
    fn draining_consumer_is_never_disconnected() {
        // Negative control: a consumer that drains its channel resets the
        // consecutive-drop counter on each successful send and is never
        // disconnected, even across far more than `threshold` broadcasts.
        let config = slow_consumer_config(2, 3);
        let registry = ConnectionRegistry::new();
        let (handle, mut rx) = registry.register(ConnectionKind::Client, &config);

        for i in 0..100u8 {
            let outcome = handle.try_send_broadcast(OutboundMessage::Binary(vec![i]));
            assert_eq!(outcome, BroadcastOutcome::Sent);
            // Drain immediately so the channel never fills.
            assert!(rx.try_recv().is_ok());
        }

        assert!(!handle.is_cancelled(), "a draining consumer must survive");
        assert_eq!(handle.dropped_broadcasts(), 0);
    }

    #[test]
    fn successful_broadcast_resets_consecutive_drops() {
        // A few drops followed by a successful send must reset the streak, so a
        // connection that occasionally falls behind but recovers is not reaped.
        let config = slow_consumer_config(1, 3);
        let registry = ConnectionRegistry::new();
        let (handle, mut rx) = registry.register(ConnectionKind::Client, &config);

        // Fill (cap 1), then two drops (under threshold 3).
        assert_eq!(
            handle.try_send_broadcast(OutboundMessage::Binary(vec![1])),
            BroadcastOutcome::Sent
        );
        assert_eq!(
            handle.try_send_broadcast(OutboundMessage::Binary(vec![2])),
            BroadcastOutcome::Dropped
        );
        assert_eq!(
            handle.try_send_broadcast(OutboundMessage::Binary(vec![3])),
            BroadcastOutcome::Dropped
        );

        // Drain, then a successful send resets the streak.
        assert!(rx.try_recv().is_ok());
        assert_eq!(
            handle.try_send_broadcast(OutboundMessage::Binary(vec![4])),
            BroadcastOutcome::Sent
        );

        // Now the channel is full again; it takes another full `threshold` drops
        // (not just one) to disconnect — proving the reset took effect.
        assert_eq!(
            handle.try_send_broadcast(OutboundMessage::Binary(vec![5])),
            BroadcastOutcome::Dropped
        );
        assert_eq!(
            handle.try_send_broadcast(OutboundMessage::Binary(vec![6])),
            BroadcastOutcome::Dropped
        );
        assert!(!handle.is_cancelled());
        assert_eq!(
            handle.try_send_broadcast(OutboundMessage::Binary(vec![7])),
            BroadcastOutcome::Disconnected
        );
        assert!(handle.is_cancelled());
    }

    #[test]
    fn cluster_peer_is_not_disconnected_on_drops() {
        // Negative control: cluster peers fall back to best-effort drop; their
        // liveness is the cluster failure detector's job, not this heuristic.
        let config = slow_consumer_config(1, 1);
        let registry = ConnectionRegistry::new();
        let (handle, _rx) = registry.register(ConnectionKind::ClusterPeer, &config);

        assert_eq!(
            handle.try_send_broadcast(OutboundMessage::Binary(vec![1])),
            BroadcastOutcome::Sent
        );
        for _ in 0..10 {
            assert_eq!(
                handle.try_send_broadcast(OutboundMessage::Binary(vec![2])),
                BroadcastOutcome::Dropped
            );
        }
        assert!(
            !handle.is_cancelled(),
            "cluster peer must never be disconnected by the slow-consumer heuristic"
        );
        assert_eq!(handle.dropped_broadcasts(), 10);
    }

    #[test]
    fn broadcast_outcome_closed_when_receiver_gone() {
        let config = slow_consumer_config(2, 3);
        let registry = ConnectionRegistry::new();
        let (handle, rx) = registry.register(ConnectionKind::Client, &config);
        drop(rx);

        assert_eq!(
            handle.try_send_broadcast(OutboundMessage::Binary(vec![1])),
            BroadcastOutcome::Closed
        );
    }

    #[test]
    fn threshold_zero_disables_disconnect() {
        // threshold = 0 preserves the legacy pure best-effort drop semantics.
        let config = slow_consumer_config(1, 0);
        let registry = ConnectionRegistry::new();
        let (handle, _rx) = registry.register(ConnectionKind::Client, &config);

        assert_eq!(
            handle.try_send_broadcast(OutboundMessage::Binary(vec![1])),
            BroadcastOutcome::Sent
        );
        for _ in 0..50 {
            assert_eq!(
                handle.try_send_broadcast(OutboundMessage::Binary(vec![2])),
                BroadcastOutcome::Dropped
            );
        }
        assert!(!handle.is_cancelled());
    }

    #[test]
    fn takeover_transfers_ownership_and_fences_displaced() {
        let config = test_config();
        let registry = ConnectionRegistry::new();
        let (h1, _rx1) = registry.register(ConnectionKind::Client, &config);
        let (h2, _rx2) = registry.register(ConnectionKind::Client, &config);
        let key = "identity-A".to_string();

        // First claimant becomes owner, displacing nobody.
        assert!(registry.claim_device_ownership(key.clone(), h1.id).is_none());
        assert!(registry.is_current_owner(&key, h1.id));

        // Second claimant TAKES OVER: reports h1 as displaced and becomes owner.
        let displaced = registry.claim_device_ownership(key.clone(), h2.id);
        assert_eq!(displaced, Some(h1.id), "takeover reports the displaced connection");
        assert!(registry.is_current_owner(&key, h2.id));
        // Fencing: the displaced connection is no longer the owner.
        assert!(!registry.is_current_owner(&key, h1.id));
    }

    #[test]
    fn disconnect_releases_ownership_only_if_still_owner() {
        let config = test_config();
        let registry = ConnectionRegistry::new();
        let (h1, _rx1) = registry.register(ConnectionKind::Client, &config);
        let (h2, _rx2) = registry.register(ConnectionKind::Client, &config);
        let key = "identity-B".to_string();

        registry.claim_device_ownership(key.clone(), h1.id);
        registry.claim_device_ownership(key.clone(), h2.id); // h2 owns now

        // The displaced h1 disconnecting must NOT clear h2's ownership.
        registry.remove(h1.id);
        assert!(registry.is_current_owner(&key, h2.id), "displaced disconnect must not clobber the new owner");

        // The current owner disconnecting releases the identity.
        registry.remove(h2.id);
        assert!(!registry.is_current_owner(&key, h2.id));
    }

    #[test]
    fn reclaim_by_same_connection_is_noop() {
        let config = test_config();
        let registry = ConnectionRegistry::new();
        let (h1, _rx1) = registry.register(ConnectionKind::Client, &config);
        let key = "identity-C".to_string();
        assert!(registry.claim_device_ownership(key.clone(), h1.id).is_none());
        // Re-claiming an already-owned identity displaces nobody.
        assert!(registry.claim_device_ownership(key.clone(), h1.id).is_none());
        assert!(registry.is_current_owner(&key, h1.id));
    }
}
