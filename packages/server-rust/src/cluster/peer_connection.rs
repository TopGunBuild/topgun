//! Peer connection management for inter-node cluster communication.
//!
//! Maintains length-prefixed TCP connections to other cluster nodes,
//! sends and receives `ClusterMessage` frames serialized with MsgPack.

use dashmap::DashMap;
use tokio::sync::mpsc;

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
