//! Peer connection management for inter-node cluster communication.
//!
//! Maintains length-prefixed TCP connections to other cluster nodes,
//! sends and receives `ClusterMessage` frames serialized with MsgPack.

use std::time::SystemTime;

use dashmap::DashMap;
use tokio::sync::mpsc;
use tracing::warn;

use super::messages::ClusterMessage;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can occur when sending a message to a peer.
#[derive(Debug)]
pub enum PeerSendError {
    /// The target node is not in the connection map.
    NotConnected,
    /// The peer's write channel has been closed.
    ChannelClosed,
    /// MsgPack serialization failed.
    Serialize(rmp_serde::encode::Error),
}

impl std::fmt::Display for PeerSendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotConnected => write!(f, "peer not connected"),
            Self::ChannelClosed => write!(f, "peer channel closed"),
            Self::Serialize(e) => write!(f, "serialization error: {e}"),
        }
    }
}

impl std::error::Error for PeerSendError {}

// ---------------------------------------------------------------------------
// Peer connection types
// ---------------------------------------------------------------------------

/// A single active connection to a cluster peer.
pub struct PeerConnection {
    pub node_id: String,
    pub tx: mpsc::UnboundedSender<Vec<u8>>,
    pub connected_at_ms: u64,
}

/// Concurrent map of active peer connections, keyed by node ID.
///
/// Wraps `DashMap` to provide typed methods for peer lifecycle
/// management and message broadcasting.
pub struct PeerConnectionMap(DashMap<String, PeerConnection>);

impl PeerConnectionMap {
    /// Creates a new empty peer connection map.
    #[must_use]
    pub fn new() -> Self {
        Self(DashMap::new())
    }

    /// Inserts a peer connection, capturing the current system time as `connected_at_ms`.
    pub fn insert(&self, node_id: String, tx: mpsc::UnboundedSender<Vec<u8>>) {
        let connected_at_ms = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        self.0.insert(
            node_id.clone(),
            PeerConnection {
                node_id,
                tx,
                connected_at_ms,
            },
        );
    }

    /// Removes and returns the peer connection for the given node ID.
    pub fn remove(&self, node_id: &str) -> Option<PeerConnection> {
        self.0.remove(node_id).map(|(_, conn)| conn)
    }

    /// Serializes a `ClusterMessage` and sends it to a specific peer.
    pub fn send_to(&self, node_id: &str, msg: &ClusterMessage) -> Result<(), PeerSendError> {
        let entry = self.0.get(node_id).ok_or(PeerSendError::NotConnected)?;
        let bytes = rmp_serde::to_vec_named(msg).map_err(PeerSendError::Serialize)?;
        entry
            .tx
            .send(bytes)
            .map_err(|_| PeerSendError::ChannelClosed)
    }

    /// Broadcasts a `ClusterMessage` to all connected peers, optionally excluding one node.
    ///
    /// Serializes the message once and clones the bytes for each recipient.
    /// Peers whose channels are closed are logged at warn level and skipped.
    pub fn broadcast(&self, msg: &ClusterMessage, exclude: Option<&str>) {
        let bytes = match rmp_serde::to_vec_named(msg) {
            Ok(b) => b,
            Err(e) => {
                warn!("Failed to serialize broadcast message: {e}");
                return;
            }
        };

        for entry in self.0.iter() {
            if exclude == Some(entry.key().as_str()) {
                continue;
            }
            if entry.tx.send(bytes.clone()).is_err() {
                warn!(
                    node_id = %entry.key(),
                    "Failed to send broadcast to peer: channel closed"
                );
            }
        }
    }

    /// Returns all connected peer node IDs.
    #[must_use]
    pub fn connected_peers(&self) -> Vec<String> {
        self.0.iter().map(|entry| entry.key().clone()).collect()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster::messages::{HeartbeatPayload, JoinRequestPayload};

    #[test]
    fn new_map_has_no_peers() {
        let map = PeerConnectionMap::new();
        assert!(map.connected_peers().is_empty());
    }

    #[test]
    fn insert_and_connected_peers() {
        let map = PeerConnectionMap::new();
        let (tx1, _rx1) = mpsc::unbounded_channel();
        let (tx2, _rx2) = mpsc::unbounded_channel();

        map.insert("node-1".to_string(), tx1);
        map.insert("node-2".to_string(), tx2);

        let mut peers = map.connected_peers();
        peers.sort();
        assert_eq!(peers, vec!["node-1", "node-2"]);
    }

    #[test]
    fn insert_sets_connected_at_ms() {
        let map = PeerConnectionMap::new();
        let (tx, _rx) = mpsc::unbounded_channel();

        map.insert("node-1".to_string(), tx);

        let entry = map.0.get("node-1").unwrap();
        // connected_at_ms should be a reasonable epoch-ms value (after 2020)
        assert!(entry.connected_at_ms > 1_577_836_800_000);
    }

    #[test]
    fn remove_returns_connection() {
        let map = PeerConnectionMap::new();
        let (tx, _rx) = mpsc::unbounded_channel();

        map.insert("node-1".to_string(), tx);

        let removed = map.remove("node-1");
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().node_id, "node-1");
        assert!(map.connected_peers().is_empty());
    }

    #[test]
    fn remove_nonexistent_returns_none() {
        let map = PeerConnectionMap::new();
        assert!(map.remove("nonexistent").is_none());
    }

    #[test]
    fn send_to_delivers_serialized_message() {
        let map = PeerConnectionMap::new();
        let (tx, mut rx) = mpsc::unbounded_channel();

        map.insert("node-1".to_string(), tx);

        let msg = ClusterMessage::Heartbeat(HeartbeatPayload {
            sender_id: "node-0".to_string(),
            timestamp_ms: 1_000,
            members_view_version: 1,
            suspected_nodes: vec![],
        });

        map.send_to("node-1", &msg).unwrap();

        let bytes = rx.try_recv().unwrap();
        let decoded: ClusterMessage = rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!(decoded, msg);
    }

    #[test]
    fn send_to_not_connected() {
        let map = PeerConnectionMap::new();
        let msg = ClusterMessage::FetchPartitionTable;

        let err = map.send_to("nonexistent", &msg).unwrap_err();
        assert!(matches!(err, PeerSendError::NotConnected));
    }

    #[test]
    fn send_to_channel_closed() {
        let map = PeerConnectionMap::new();
        let (tx, rx) = mpsc::unbounded_channel();
        drop(rx); // close the receiver

        map.insert("node-1".to_string(), tx);

        let msg = ClusterMessage::FetchPartitionTable;
        let err = map.send_to("node-1", &msg).unwrap_err();
        assert!(matches!(err, PeerSendError::ChannelClosed));
    }

    #[test]
    fn broadcast_sends_to_all_except_excluded() {
        let map = PeerConnectionMap::new();
        let (tx1, mut rx1) = mpsc::unbounded_channel();
        let (tx2, mut rx2) = mpsc::unbounded_channel();
        let (tx3, mut rx3) = mpsc::unbounded_channel();

        map.insert("node-1".to_string(), tx1);
        map.insert("node-2".to_string(), tx2);
        map.insert("node-3".to_string(), tx3);

        let msg = ClusterMessage::JoinRequest(JoinRequestPayload {
            node_id: "node-0".to_string(),
            host: "127.0.0.1".to_string(),
            client_port: 8080,
            cluster_port: 9090,
            cluster_id: "test".to_string(),
            protocol_version: 1,
            auth_token: None,
        });

        map.broadcast(&msg, Some("node-2"));

        // node-1 and node-3 should receive the message
        assert!(rx1.try_recv().is_ok());
        assert!(rx3.try_recv().is_ok());
        // node-2 should not receive (excluded)
        assert!(rx2.try_recv().is_err());
    }

    #[test]
    fn broadcast_without_exclude_sends_to_all() {
        let map = PeerConnectionMap::new();
        let (tx1, mut rx1) = mpsc::unbounded_channel();
        let (tx2, mut rx2) = mpsc::unbounded_channel();

        map.insert("node-1".to_string(), tx1);
        map.insert("node-2".to_string(), tx2);

        let msg = ClusterMessage::FetchPartitionTable;
        map.broadcast(&msg, None);

        assert!(rx1.try_recv().is_ok());
        assert!(rx2.try_recv().is_ok());
    }

    #[test]
    fn broadcast_skips_closed_channels() {
        let map = PeerConnectionMap::new();
        let (tx1, mut rx1) = mpsc::unbounded_channel();
        let (tx2, rx2) = mpsc::unbounded_channel();
        drop(rx2); // close node-2's receiver

        map.insert("node-1".to_string(), tx1);
        map.insert("node-2".to_string(), tx2);

        let msg = ClusterMessage::FetchPartitionTable;
        // Should not panic -- closed channels are logged and skipped
        map.broadcast(&msg, None);

        // node-1 should still receive the message
        assert!(rx1.try_recv().is_ok());
    }
}
