//! Cluster formation service: seed discovery, join handshake, inbound listener.
//!
//! Orchestrates the cluster formation protocol: accepts inbound TCP connections
//! from other nodes, discovers seed nodes with exponential backoff, and handles
//! the join handshake (both as joiner and as master).

use std::sync::Arc;
use std::time::{Duration, SystemTime};

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
// Constants
// ---------------------------------------------------------------------------

/// Initial backoff delay for seed connection attempts.
const SEED_BACKOFF_INITIAL_MS: u64 = 500;

/// Maximum backoff delay for seed connection attempts.
const SEED_BACKOFF_MAX_MS: u64 = 5_000;

/// Maximum number of seed connection retry attempts.
const SEED_MAX_ATTEMPTS: u32 = 10;

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
    /// For inbound connections (`None`), the peer is registered when a `JoinRequest` is received.
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
                    self.handle_frame(&frame, &write_tx, &peer_node_id).await;
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
            let _ = self.peers.remove(id);
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
            ClusterMessage::JoinResponse(ref payload) => {
                self.handle_join_response(payload);
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
    // Seed discovery
    // -----------------------------------------------------------------------

    /// Runs seed discovery: attempts to connect to seed nodes and join the cluster.
    async fn run_seed_discovery(self: Arc<Self>) {
        self.discover_seeds_and_join().await;
    }

    /// Attempts to connect to each seed address with exponential backoff.
    /// If all seeds are unreachable after all retry attempts, self-promotes
    /// as a single-node master.
    async fn discover_seeds_and_join(self: &Arc<Self>) {
        if self.config.seed_addresses.is_empty() {
            info!("No seed addresses configured, self-promoting as single-node master");
            self.self_promote_as_master();
            return;
        }

        // Filter out our own address from seeds
        let own_addr = format!(
            "{}:{}",
            self.local_member.host, self.local_member.cluster_port
        );

        for seed_addr in &self.config.seed_addresses {
            if seed_addr == &own_addr {
                continue;
            }

            let mut backoff_ms = SEED_BACKOFF_INITIAL_MS;

            for attempt in 1..=SEED_MAX_ATTEMPTS {
                info!(
                    seed = %seed_addr,
                    attempt,
                    "Attempting to connect to seed node"
                );

                match TcpStream::connect(seed_addr).await {
                    Ok(stream) => {
                        info!(seed = %seed_addr, "Connected to seed node");
                        if self.send_join_request(stream, seed_addr).await {
                            return; // Successfully joined
                        }
                        // Join request failed (rejected or connection lost), try next seed
                        break;
                    }
                    Err(e) => {
                        warn!(
                            seed = %seed_addr,
                            attempt,
                            max_attempts = SEED_MAX_ATTEMPTS,
                            "Failed to connect to seed: {e}"
                        );

                        if attempt < SEED_MAX_ATTEMPTS {
                            tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                            backoff_ms = (backoff_ms * 2).min(SEED_BACKOFF_MAX_MS);
                        }
                    }
                }
            }
        }

        // All seeds unreachable after all retries
        info!("All seed nodes unreachable, self-promoting as single-node master");
        self.self_promote_as_master();
    }

    /// Sends a `JoinRequest` to a seed node over a newly established TCP connection.
    /// Returns `true` if join was successful.
    async fn send_join_request(&self, stream: TcpStream, seed_addr: &str) -> bool {
        let framed = Framed::new(stream, LengthDelimitedCodec::new());
        let (mut sink, mut stream_reader) = framed.split();

        let join_req = ClusterMessage::JoinRequest(JoinRequestPayload {
            node_id: self.local_member.node_id.clone(),
            host: self.local_member.host.clone(),
            client_port: self.local_member.client_port,
            cluster_port: self.local_member.cluster_port,
            cluster_id: self.config.cluster_id.clone(),
            protocol_version: 1,
            auth_token: None,
        });

        let bytes = match rmp_serde::to_vec_named(&join_req) {
            Ok(b) => b,
            Err(e) => {
                warn!("Failed to serialize JoinRequest: {e}");
                return false;
            }
        };

        if sink.send(bytes.into()).await.is_err() {
            warn!(seed = %seed_addr, "Failed to send JoinRequest to seed");
            return false;
        }

        // Wait for JoinResponse
        match stream_reader.next().await {
            Some(Ok(frame)) => {
                let msg: ClusterMessage = match rmp_serde::from_slice(&frame) {
                    Ok(m) => m,
                    Err(e) => {
                        warn!(seed = %seed_addr, "Failed to deserialize seed response: {e}");
                        return false;
                    }
                };

                if let ClusterMessage::JoinResponse(response) = msg {
                    if response.accepted {
                        info!(seed = %seed_addr, "Join accepted by seed node");
                        self.apply_join_response(&response);
                        return true;
                    }
                    warn!(
                        seed = %seed_addr,
                        reason = ?response.reject_reason,
                        "Join rejected by seed node"
                    );
                    return false;
                }

                warn!(seed = %seed_addr, "Unexpected response from seed (expected JoinResponse)");
                false
            }
            Some(Err(e)) => {
                warn!(seed = %seed_addr, "Error reading seed response: {e}");
                false
            }
            None => {
                warn!(seed = %seed_addr, "Seed connection closed before response");
                false
            }
        }
    }

    /// Applies a successful `JoinResponse`: updates `ClusterState` with the received
    /// `members_view` and `partition_assignments`.
    fn apply_join_response(&self, response: &JoinResponsePayload) {
        if let Some(ref view) = response.members_view {
            info!(
                version = view.version,
                members = view.members.len(),
                "Applying members view from join response"
            );
            self.cluster_state.update_view(view.clone());
        }

        if let Some(ref assignments) = response.partition_assignments {
            info!(
                count = assignments.len(),
                "Applying partition assignments from join response"
            );
            self.cluster_state
                .partition_table
                .apply_assignments(assignments);
        }
    }

    /// Self-promotes this node as a single-node master when no seeds are reachable.
    fn self_promote_as_master(&self) {
        let mut member = self.local_member.clone();
        member.state = NodeState::Active;

        let view = MembersView {
            version: 1,
            members: vec![member.clone()],
        };

        self.cluster_state.update_view(view);

        // Compute initial partition assignments for single node
        let assignments = compute_assignment(
            &[member],
            self.cluster_state.partition_table.partition_count(),
            self.config.backup_count,
        );
        self.cluster_state
            .partition_table
            .apply_assignments(&assignments);

        info!(
            node_id = %self.local_member.node_id,
            "Self-promoted as single-node master"
        );
    }

    // -----------------------------------------------------------------------
    // Join request handling (inbound -- this node is master)
    // -----------------------------------------------------------------------

    /// Handles an inbound `JoinRequest` from another node.
    ///
    /// Processing is serialized via `join_mutex` to prevent stale-view races
    /// from concurrent joiners.
    async fn handle_join_request(
        &self,
        payload: JoinRequestPayload,
        write_tx: &mpsc::UnboundedSender<Vec<u8>>,
        peer_node_id: &Arc<Mutex<Option<String>>>,
    ) {
        // Only master handles join requests
        if !self.cluster_state.is_master() {
            let master_addr = self.get_master_address();
            let reject_reason = format!("not master; master address: {master_addr}");
            let response = ClusterMessage::JoinResponse(JoinResponsePayload {
                accepted: false,
                reject_reason: Some(reject_reason),
                members_view: None,
                partition_assignments: None,
            });

            if let Ok(bytes) = rmp_serde::to_vec_named(&response) {
                let _ = write_tx.send(bytes);
            }
            return;
        }

        // Serialize join processing to prevent concurrent join races
        let _guard = self.join_mutex.lock().await;

        info!(
            node_id = %payload.node_id,
            host = %payload.host,
            "Processing join request"
        );

        // Register the peer's node_id for this connection
        {
            let mut id = peer_node_id.lock().await;
            *id = Some(payload.node_id.clone());
        }
        self.peers
            .insert(payload.node_id.clone(), write_tx.clone());

        // Build new member info -- add directly as Active so compute_assignment includes it
        let new_member = MemberInfo {
            node_id: payload.node_id.clone(),
            host: payload.host,
            client_port: payload.client_port,
            cluster_port: payload.cluster_port,
            state: NodeState::Active,
            join_version: 0, // Will be set to current view version + 1
        };

        // Get current view and add new member
        let current_view = self.cluster_state.current_view();
        let new_version = current_view.version + 1;

        let mut members = current_view.members.clone();

        let mut active_member = new_member;
        active_member.join_version = new_version;
        members.push(active_member);

        let updated_view = MembersView {
            version: new_version,
            members,
        };

        // Update cluster state with new view (member is Active)
        self.cluster_state.update_view(updated_view.clone());

        // Compute new partition assignments (Active-only filter now includes the new node)
        let assignments = compute_assignment(
            &updated_view.members,
            self.cluster_state.partition_table.partition_count(),
            self.config.backup_count,
        );
        self.cluster_state
            .partition_table
            .apply_assignments(&assignments);

        // Respond with JoinResponse
        let response = ClusterMessage::JoinResponse(JoinResponsePayload {
            accepted: true,
            reject_reason: None,
            members_view: Some(updated_view.clone()),
            partition_assignments: Some(assignments),
        });

        if let Ok(bytes) = rmp_serde::to_vec_named(&response) {
            let _ = write_tx.send(bytes);
        }

        // Broadcast MembersUpdate to all existing peers (excluding the new joiner,
        // who already received the full view in the JoinResponse)
        #[allow(clippy::cast_possible_truncation)]
        let cluster_time_ms = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let members_update = ClusterMessage::MembersUpdate(MembersUpdatePayload {
            view: updated_view,
            cluster_time_ms,
        });

        self.peers
            .broadcast(&members_update, Some(&payload.node_id));

        info!(
            node_id = %payload.node_id,
            "Join request accepted, member added as Active"
        );
    }

    // -----------------------------------------------------------------------
    // Join response handling (outbound -- this node is joining)
    // -----------------------------------------------------------------------

    /// Handles a `JoinResponse` received after sending a `JoinRequest`.
    /// This is used for the per-peer read loop path (not the seed discovery path).
    fn handle_join_response(&self, payload: &JoinResponsePayload) {
        if payload.accepted {
            info!("Received accepted JoinResponse via peer connection");
            self.apply_join_response(payload);
        } else {
            warn!(
                reason = ?payload.reject_reason,
                "Received rejected JoinResponse via peer connection"
            );
        }
    }

    // -----------------------------------------------------------------------
    // Members update handling
    // -----------------------------------------------------------------------

    /// Applies a `MembersUpdate` to the cluster state.
    fn handle_members_update(&self, payload: MembersUpdatePayload) {
        info!(
            version = payload.view.version,
            members = payload.view.members.len(),
            cluster_time_ms = payload.cluster_time_ms,
            "Received MembersUpdate"
        );
        self.cluster_state.update_view(payload.view);
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    /// Returns the master's cluster address, or `"unknown"` if no master is found.
    fn get_master_address(&self) -> String {
        let view = self.cluster_state.current_view();
        view.master().map_or_else(
            || "unknown".to_string(),
            |m| format!("{}:{}", m.host, m.cluster_port),
        )
    }
}
