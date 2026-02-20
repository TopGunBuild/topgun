//! Connection management types and registry for the `TopGun` server.
//!
//! Provides per-connection backpressure via bounded mpsc channels,
//! lock-free concurrent connection tracking via `DashMap`, and
//! metadata storage for authentication and subscription state.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use tokio::sync::{mpsc, RwLock};
use topgun_core::{Principal, Timestamp};

use super::config::ConnectionConfig;

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

/// Error returned when sending a message to a connection fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendError {
    /// The send operation timed out (channel is full and remained full).
    Timeout,
    /// The connection has been closed; the receiver was dropped.
    Disconnected,
    /// The channel is full (non-blocking `try_send` only).
    Full,
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

    /// Sends a message with a timeout.
    ///
    /// # Errors
    ///
    /// Returns `SendError::Timeout` if the channel remains full for the
    /// entire timeout duration. Returns `SendError::Disconnected` if the
    /// receiver has been dropped (connection closed).
    pub async fn send_timeout(
        &self,
        msg: OutboundMessage,
        timeout: Duration,
    ) -> Result<(), SendError> {
        match tokio::time::timeout(timeout, self.tx.send(msg)).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_)) => Err(SendError::Disconnected),
            Err(_) => Err(SendError::Timeout),
        }
    }

    /// Checks whether the connection is still open.
    ///
    /// Returns `false` if the receiver end of the channel has been dropped,
    /// meaning the WebSocket write loop has exited.
    #[must_use]
    pub fn is_connected(&self) -> bool {
        !self.tx.is_closed()
    }
}

/// Mutable metadata associated with a connection.
///
/// Protected by an `RwLock` to allow concurrent reads (e.g., broadcast
/// filtering) while serializing writes (e.g., authentication updates).
#[derive(Debug)]
pub struct ConnectionMetadata {
    /// Whether this connection has completed authentication.
    pub authenticated: bool,
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
}

impl Default for ConnectionMetadata {
    fn default() -> Self {
        Self {
            authenticated: false,
            principal: None,
            subscriptions: HashSet::new(),
            topics: HashSet::new(),
            last_heartbeat: Instant::now(),
            last_hlc: None,
            peer_node_id: None,
        }
    }
}

/// Thread-safe registry of all active connections.
///
/// Uses `DashMap` for lock-free concurrent access, supporting 10K+
/// simultaneous connections without contention.
#[derive(Debug)]
pub struct ConnectionRegistry {
    connections: DashMap<ConnectionId, Arc<ConnectionHandle>>,
    next_id: AtomicU64,
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
        });

        self.connections.insert(id, Arc::clone(&handle));
        (handle, rx)
    }

    /// Removes a connection from the registry, returning its handle if found.
    pub fn remove(&self, id: ConnectionId) -> Option<Arc<ConnectionHandle>> {
        self.connections.remove(&id).map(|(_, handle)| handle)
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

    /// Sends a binary message to all connections of a given kind.
    ///
    /// Uses non-blocking `try_send` so a single slow connection cannot
    /// block the broadcast. Full channels are silently skipped.
    pub fn broadcast(&self, msg_bytes: &[u8], kind: ConnectionKind) {
        for entry in &self.connections {
            let handle = entry.value();
            if handle.kind == kind {
                // Intentionally ignore the result: broadcast skips full channels
                let _ = handle.try_send(OutboundMessage::Binary(msg_bytes.to_vec()));
            }
        }
    }

    /// Removes and returns all connections. Used during graceful shutdown.
    pub fn drain_all(&self) -> Vec<Arc<ConnectionHandle>> {
        let keys: Vec<ConnectionId> = self
            .connections
            .iter()
            .map(|entry| *entry.key())
            .collect();

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

    #[tokio::test]
    async fn connection_handle_send_timeout_success() {
        let config = test_config();
        let registry = ConnectionRegistry::new();
        let (handle, _rx) = registry.register(ConnectionKind::Client, &config);

        let result = handle
            .send_timeout(
                OutboundMessage::Binary(vec![1, 2, 3]),
                Duration::from_secs(1),
            )
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn connection_handle_send_timeout_disconnected() {
        let config = test_config();
        let registry = ConnectionRegistry::new();
        let (handle, rx) = registry.register(ConnectionKind::Client, &config);
        drop(rx);

        let result = handle
            .send_timeout(
                OutboundMessage::Binary(vec![1, 2, 3]),
                Duration::from_secs(1),
            )
            .await;
        assert_eq!(result, Err(SendError::Disconnected));
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
}
