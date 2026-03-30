//! Cluster formation service: seed discovery, join handshake, inbound listener.
//!
//! Orchestrates the cluster formation protocol: accepts inbound TCP connections
//! from other nodes, discovers seed nodes with exponential backoff, and handles
//! the join handshake (both as joiner and as master).

use std::sync::Arc;

use bytes::BytesMut;
use futures_util::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Mutex};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use tracing::{info, warn};

use super::assignment::compute_assignment;
use super::messages::{
    ClusterMessage, JoinRequestPayload, JoinResponsePayload, MembersUpdatePayload,
};
use super::peer_connection::PeerConnectionMap;
use super::state::{ClusterState, InboundClusterMessage};
use super::types::{ClusterConfig, MemberInfo, MembersView, NodeState};

// ---------------------------------------------------------------------------
// ClusterFormationService
// ---------------------------------------------------------------------------

/// Cluster formation service managing seed discovery, join handshake,
/// and inbound peer connection acceptance.
pub struct ClusterFormationService {
    pub cluster_state: Arc<ClusterState>,
    pub peers: Arc<PeerConnectionMap>,
    pub config: Arc<ClusterConfig>,
    pub local_member: MemberInfo,
    /// Channel for forwarding unhandled messages to SPEC-165b and existing dispatch.
    /// Intentionally unbounded to avoid backpressure stalling the per-peer read loop
    /// -- cluster protocol messages are small and bounded by cluster size.
    pub inbound_tx: mpsc::UnboundedSender<InboundClusterMessage>,
    /// Serializes join request processing to prevent stale-view races from concurrent joiners.
    join_mutex: Mutex<()>,
}

impl ClusterFormationService {
    /// Creates a new formation service.
    #[must_use]
    pub fn new(
        cluster_state: Arc<ClusterState>,
        peers: Arc<PeerConnectionMap>,
        config: Arc<ClusterConfig>,
        local_member: MemberInfo,
        inbound_tx: mpsc::UnboundedSender<InboundClusterMessage>,
    ) -> Self {
        Self {
            cluster_state,
            peers,
            config,
            local_member,
            inbound_tx,
            join_mutex: Mutex::new(()),
        }
    }

    /// Starts the formation service: spawns inbound listener and seed discovery tasks.
    ///
    /// The `cluster_listener` should already be bound to the cluster port.
    pub fn start(self: Arc<Self>, cluster_listener: TcpListener) {
        // Spawn inbound connection listener
        let this = Arc::clone(&self);
        tokio::spawn(async move {
            this.run_inbound_listener(cluster_listener).await;
        });

        // Spawn seed discovery task
        let this = Arc::clone(&self);
        tokio::spawn(async move {
            this.run_seed_discovery().await;
        });
    }

    // -----------------------------------------------------------------------
    // Inbound listener
    // -----------------------------------------------------------------------

    /// Accepts inbound TCP connections and spawns per-peer read/write loops.
    async fn run_inbound_listener(self: Arc<Self>, listener: TcpListener) {
        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    info!(%addr, "Accepted inbound cluster connection");
                    let this = Arc::clone(&self);
                    tokio::spawn(async move {
                        this.handle_peer_connection(stream, None).await;
                    });
                }
                Err(e) => {
                    warn!("Failed to accept cluster connection: {e}");
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Per-peer connection handling
    // -----------------------------------------------------------------------

    /// Handles a single peer TCP connection with length-prefixed framing.
    ///
    /// Frame format: `[4 bytes: payload length][N bytes: rmp_serde::to_vec_named(ClusterMessage)]`
    ///
    /// If `known_node_id` is `Some`, the peer is registered immediately (outbound connection).
    /// For inbound connections (`None`), the peer is registered when a JoinRequest is received.
    async fn handle_peer_connection(
        self: Arc<Self>,
        stream: TcpStream,
        known_node_id: Option<String>,
    ) {
        let framed = Framed::new(stream, LengthDelimitedCodec::new());
        let (mut sink, mut stream_reader) = framed.split();

        // Write channel: peer sends frames via this channel, write loop forwards to TCP
        let (write_tx, mut write_rx) = mpsc::unbounded_channel::<Vec<u8>>();

        // If we already know the peer's node_id (outbound connection), register immediately
        if let Some(ref node_id) = known_node_id {
            self.peers.insert(node_id.clone(), write_tx.clone());
        }

        let peer_node_id: Arc<Mutex<Option<String>>> =
            Arc::new(Mutex::new(known_node_id.clone()));

        // Spawn write loop: forwards bytes from write_tx to the TCP sink
        let write_handle = tokio::spawn(async move {
            while let Some(bytes) = write_rx.recv().await {
                if sink.send(bytes.into()).await.is_err() {
                    break;
                }
            }
        });

        // Read loop: reads framed messages from TCP and routes them
        while let Some(frame_result) = stream_reader.next().await {
            match frame_result {
                Ok(frame) => {
                    self.handle_frame(
                        &frame,
                        &write_tx,
                        &peer_node_id,
                    )
                    .await;
                }
                Err(e) => {
                    let id = peer_node_id.lock().await;
                    warn!(
                        peer = ?id,
                        "Peer connection read error: {e}"
                    );
                    break;
                }
            }
        }

        // Cleanup: remove peer from connection map
        let node_id = peer_node_id.lock().await;
        if let Some(ref id) = *node_id {
            self.peers.remove(id);
            info!(node_id = %id, "Peer disconnected");
        }
        write_handle.abort();
    }

    /// Deserializes a frame and routes it to the appropriate handler.
    async fn handle_frame(
        self: &Arc<Self>,
        frame: &BytesMut,
        write_tx: &mpsc::UnboundedSender<Vec<u8>>,
        peer_node_id: &Arc<Mutex<Option<String>>>,
    ) {
        let msg: ClusterMessage = match rmp_serde::from_slice(frame) {
            Ok(m) => m,
            Err(e) => {
                warn!("Deserialization failure on cluster frame: {e}");
                return;
            }
        };

        match msg {
            ClusterMessage::JoinRequest(payload) => {
                self.handle_join_request(payload, write_tx, peer_node_id)
                    .await;
            }
            ClusterMessage::JoinResponse(payload) => {
                self.handle_join_response(payload, write_tx, peer_node_id)
                    .await;
            }
            ClusterMessage::MembersUpdate(payload) => {
                self.handle_members_update(payload);
            }
            // All other variants forwarded to inbound_tx for dispatch
            other => {
                let sender_id = peer_node_id
                    .lock()
                    .await
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string());
                let _ = self.inbound_tx.send(InboundClusterMessage {
                    sender_node_id: sender_id,
                    message: other,
                });
            }
        }
    }

    // -----------------------------------------------------------------------
    // Seed discovery (placeholder -- implemented in G3-S2)
    // -----------------------------------------------------------------------

    async fn run_seed_discovery(self: Arc<Self>) {
        // Implemented in G3-S2
        self.discover_seeds_and_join().await;
    }

    // -----------------------------------------------------------------------
    // Message handlers (placeholders -- implemented in G3-S2)
    // -----------------------------------------------------------------------

    async fn handle_join_request(
        &self,
        _payload: JoinRequestPayload,
        _write_tx: &mpsc::UnboundedSender<Vec<u8>>,
        _peer_node_id: &Arc<Mutex<Option<String>>>,
    ) {
        // Implemented in G3-S2
    }

    async fn handle_join_response(
        &self,
        _payload: JoinResponsePayload,
        _write_tx: &mpsc::UnboundedSender<Vec<u8>>,
        _peer_node_id: &Arc<Mutex<Option<String>>>,
    ) {
        // Implemented in G3-S2
    }

    fn handle_members_update(&self, _payload: MembersUpdatePayload) {
        // Implemented in G3-S2
    }

    async fn discover_seeds_and_join(self: &Arc<Self>) {
        // Implemented in G3-S2
    }
}
