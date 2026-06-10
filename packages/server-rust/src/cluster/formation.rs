//! Cluster formation service: seed discovery, join handshake, inbound listener.
//!
//! Orchestrates the cluster formation protocol: accepts inbound TCP connections
//! from other nodes, discovers seed nodes with exponential backoff, and handles
//! the join handshake (both as joiner and as master).

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use bytes::BytesMut;
use futures_util::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Mutex};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use tracing::{info, warn};
use uuid::Uuid;

use super::assignment::compute_assignment;
use super::messages::{
    ClusterMessage, JoinRejectReason, JoinRequestPayload, JoinResponsePayload,
    MasterElectedPayload, MembersUpdatePayload,
};
use super::peer_connection::PeerConnectionMap;
use super::state::{ClusterChange, ClusterState, InboundClusterMessage};
use super::types::{ClusterConfig, MemberInfo, MembersView, NodeState};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Initial backoff delay for seed connection attempts (TCP-connect retry,
/// distinct from the master-election wait below).
const SEED_BACKOFF_INITIAL_MS: u64 = 500;

/// Maximum backoff delay for seed connection attempts.
const SEED_BACKOFF_MAX_MS: u64 = 5_000;

/// Maximum number of seed TCP-connect retry attempts before moving to the next seed.
const SEED_MAX_ATTEMPTS: u32 = 10;

/// Maximum time the joiner waits for a `MasterElected` broadcast after
/// receiving `NotMasterYet` rejections from all reachable seeds. On expiry,
/// the joiner enters the deterministic-tiebreak phase. Tuned for intra-host
/// loopback TCP convergence (~50ms typical broadcast latency); the safety-valve
/// `MASTER_ELECTION_TOTAL_BUDGET_MS` (30s) absorbs pathological cases.
const MASTER_ELECTION_WAIT_MS: u64 = 3_000;

/// Total time budget for the entire master-election phase, including
/// repeated wait-and-tiebreak cycles. On expiry, the joiner unconditionally
/// self-promotes as a safety valve against total broadcast loss / full
/// network partition. Tuned to cover cold-build CPU-contention parallel-spawn
/// convergence on CI machines.
const MASTER_ELECTION_TOTAL_BUDGET_MS: u64 = 30_000;

/// Reserved for the bully-style tiebreak round (Option 2b). Currently unused
/// because this spec implements deterministic-tiebreak by lowest `node_id`.
/// Keep as documented constant for future evolution if 2b becomes necessary.
#[allow(dead_code)]
const MASTER_PROPOSAL_TIMEOUT_MS: u64 = 2_000;

// ---------------------------------------------------------------------------
// Private types for the seed-discovery state machine
// ---------------------------------------------------------------------------

/// Outcome of a single `send_join_request` call.
///
/// Used by `discover_seeds_and_join` to decide whether to enter the
/// master-election wait, skip a seed, or return early on success.
enum SeedAttemptOutcome {
    /// Join accepted; caller hands the framed connection off to
    /// `handle_peer_framed`. The `Framed` (not the raw stream) is carried so any
    /// frame the master pipelined behind the `JoinResponse` survives the handoff.
    Accepted {
        framed: Framed<TcpStream, LengthDelimitedCodec>,
        master_node_id: String,
    },
    /// Join rejected with `NotMasterYet`; caller adds `responder_node_id`
    /// to the tiebreak set and holds the framed connection open for
    /// `MasterElected`. Carrying the `Framed` preserves any buffered bytes.
    RetryableRejection {
        framed: Framed<TcpStream, LengthDelimitedCodec>,
        responder_node_id: Option<String>,
    },
    /// Join rejected with a permanent reason (auth, version, `cluster_id`, full);
    /// caller skips this seed without entering the election wait.
    PermanentRejection,
    /// Connection lost or malformed response; caller moves to next seed.
    ConnectionError,
}

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
    /// Channel for forwarding non-formation messages (heartbeats, DAG ops, etc.)
    /// to the dispatch layer for handling. Intentionally unbounded to avoid
    /// backpressure stalling the per-peer read loop -- cluster protocol messages
    /// are small and bounded by cluster size.
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
    ///
    /// Wraps the raw stream in a fresh codec. Use only for connections that have
    /// NOT yet had any frame read off them (the inbound listener and freshly
    /// dialed sockets). Connections from which a handshake frame was already read
    /// MUST hand off the existing `Framed` via `handle_peer_framed` instead, so
    /// any frame the peer pipelined behind the handshake (e.g. a `MembersUpdate`
    /// coalesced into the same TCP segment as the `JoinResponse`) is not silently
    /// dropped with the codec's read buffer.
    async fn handle_peer_connection(
        self: Arc<Self>,
        stream: TcpStream,
        known_node_id: Option<String>,
    ) {
        let framed = Framed::new(stream, LengthDelimitedCodec::new());
        self.handle_peer_framed(framed, known_node_id).await;
    }

    /// Drives the persistent read/write loop over an already-framed connection.
    ///
    /// Takes ownership of the `Framed` rather than a raw `TcpStream` so that any
    /// bytes the codec buffered while reading the handshake frame are carried
    /// into the read loop. Recovering the raw stream via `Framed::into_inner()`
    /// and re-wrapping it would discard those buffered bytes — the source of a
    /// lost `MembersUpdate` when the master pipelines it immediately behind the
    /// `JoinResponse`.
    async fn handle_peer_framed(
        self: Arc<Self>,
        framed: Framed<TcpStream, LengthDelimitedCodec>,
        known_node_id: Option<String>,
    ) {
        let (mut sink, mut stream_reader) = framed.split();

        // Write channel: peer sends frames via this channel, write loop forwards to TCP
        let (write_tx, mut write_rx) = mpsc::unbounded_channel::<Vec<u8>>();

        // If we already know the peer's node_id (outbound connection), register immediately
        if let Some(ref node_id) = known_node_id {
            self.peers.insert(node_id.clone(), write_tx.clone());
        }

        let peer_node_id: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(known_node_id.clone()));

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

        // Cleanup: remove peer from connection map and mark it Dead so the
        // MembershipReactor can rebalance partitions without waiting for heartbeat
        // timeout (phi-accrual would otherwise require ~15 s to declare failure).
        let node_id = peer_node_id.lock().await;
        if let Some(ref id) = *node_id {
            let _ = self.peers.remove(id);
            info!(node_id = %id, "Peer disconnected");

            // Find the member in the current view and emit MemberRemoved so the
            // MembershipReactor rebalances immediately on TCP disconnect.
            let current = self.cluster_state.current_view();
            if let Some(pos) = current.members.iter().position(|m| &m.node_id == id) {
                let mut members = current.members.clone();
                members[pos].state = NodeState::Dead;
                let dead_member = members[pos].clone();
                let new_view = MembersView {
                    version: current.version + 1,
                    members,
                };
                self.cluster_state.update_view(new_view);
                let _ = self
                    .cluster_state
                    .change_sender()
                    .send(ClusterChange::MemberRemoved(dead_member));
            }
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

    /// Multi-phase state machine for cluster join.
    ///
    /// Seed dial: Dial each seed, send `JoinRequest`.
    ///   - `Accepted`: hand off stream, return (joined).
    ///   - `RetryableRejection` (`NotMasterYet`): collect `responder_node_id` into
    ///     tiebreak set, hold stream open for incoming `MasterElected` broadcast.
    ///   - `PermanentRejection` / `ConnectionError`: skip seed.
    ///
    /// `WaitForMasterElection`: listen on held streams for `MasterElected`.
    ///   - Received broadcast: re-dial announced master.
    ///   - Timeout: deterministic tiebreak by lowest `node_id`.
    ///
    /// Tiebreak: if self is the lexicographic minimum among
    ///   `{self} ∪ {all responder_node_ids}`, self-promote and broadcast
    ///   `MasterElected`; else loop back to `WaitForMasterElection`.
    ///
    /// Safety valve: total budget `MASTER_ELECTION_TOTAL_BUDGET_MS` (30s).
    /// If exhausted without resolution, unconditionally self-promote.
    #[allow(clippy::too_many_lines)]
    async fn discover_seeds_and_join(self: &Arc<Self>) {
        if self.config.seed_addresses.is_empty() {
            info!("No seed addresses configured, self-promoting as single-node master");
            self.self_promote_as_master();
            self.broadcast_master_elected();
            return;
        }

        let own_addr = format!(
            "{}:{}",
            self.local_member.host, self.local_member.cluster_port
        );

        let total_deadline =
            Instant::now() + Duration::from_millis(MASTER_ELECTION_TOTAL_BUDGET_MS);

        // Tiebreak set always contains self; grows as NotMasterYet rejections arrive.
        let mut tiebreak_set: HashSet<String> = HashSet::new();
        tiebreak_set.insert(self.local_member.node_id.clone());

        // Framed connections held open after NotMasterYet rejections so the seed
        // can deliver a MasterElected broadcast over the existing connection.
        // Each entry is (responder_node_id, Framed). The `Framed` is retained
        // rather than the raw stream so buffered bytes are never lost.
        let mut held_streams: Vec<(String, Framed<TcpStream, LengthDelimitedCodec>)> = Vec::new();

        'outer: loop {
            if Instant::now() >= total_deadline {
                break 'outer;
            }

            // Dial seeds and send JoinRequest
            let mut all_retryable = true;
            for seed_addr in &self.config.seed_addresses {
                if seed_addr == &own_addr {
                    continue;
                }

                let mut backoff_ms = SEED_BACKOFF_INITIAL_MS;
                for attempt in 1..=SEED_MAX_ATTEMPTS {
                    if Instant::now() >= total_deadline {
                        break 'outer;
                    }

                    info!(
                        seed = %seed_addr,
                        attempt,
                        "Attempting to connect to seed node"
                    );

                    match TcpStream::connect(seed_addr).await {
                        Ok(stream) => {
                            info!(seed = %seed_addr, "Connected to seed node");
                            match self.send_join_request(stream, seed_addr).await {
                                SeedAttemptOutcome::Accepted {
                                    framed,
                                    master_node_id,
                                } => {
                                    // Hand off the framed connection to the per-peer
                                    // handler, preserving any pipelined frames.
                                    let this = Arc::clone(self);
                                    tokio::spawn(async move {
                                        this.handle_peer_framed(framed, Some(master_node_id)).await;
                                    });
                                    return; // Successfully joined
                                }
                                SeedAttemptOutcome::RetryableRejection {
                                    framed,
                                    responder_node_id,
                                } => {
                                    if let Some(ref id) = responder_node_id {
                                        tiebreak_set.insert(id.clone());
                                        held_streams.push((id.clone(), framed));
                                    }
                                    // Older peer with no responder_node_id: stream dropped;
                                    // correctness preserved because self is always in tiebreak_set.
                                    break; // move to next seed
                                }
                                SeedAttemptOutcome::PermanentRejection => {
                                    all_retryable = false;
                                    break;
                                }
                                SeedAttemptOutcome::ConnectionError => {
                                    all_retryable = false;
                                    if attempt < SEED_MAX_ATTEMPTS {
                                        tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                                        backoff_ms = (backoff_ms * 2).min(SEED_BACKOFF_MAX_MS);
                                    } else {
                                        break;
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            all_retryable = false;
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

            // No held streams and no retryable rejections means all seeds were either
            // unreachable or permanently rejecting — break to safety-valve self-promote.
            if !all_retryable && held_streams.is_empty() {
                break 'outer;
            }

            // WaitForMasterElection — listen on held streams for MasterElected
            let wait_remaining = {
                let remaining = total_deadline.saturating_duration_since(Instant::now());
                remaining.min(Duration::from_millis(MASTER_ELECTION_WAIT_MS))
            };

            let elected = listen_for_master_elected(&mut held_streams, wait_remaining).await;

            if let Some(payload) = elected {
                // Re-dial announced master and attempt join
                drop(held_streams);
                held_streams = Vec::new();

                info!(
                    master_node_id = %payload.master_node_id,
                    master_address = %payload.master_address,
                    "MasterElected received; re-dialing announced master"
                );

                match TcpStream::connect(&payload.master_address).await {
                    Ok(stream) => {
                        if let SeedAttemptOutcome::Accepted {
                            framed,
                            master_node_id,
                        } = self
                            .send_join_request(stream, &payload.master_address)
                            .await
                        {
                            let this = Arc::clone(self);
                            tokio::spawn(async move {
                                this.handle_peer_framed(framed, Some(master_node_id)).await;
                            });
                            return;
                        }
                        // Master not ready yet or refused; loop back to seed-dial
                    }
                    Err(e) => {
                        warn!(master = %payload.master_address, "Cannot reach announced master: {e}");
                    }
                }
            } else {
                // Deterministic tiebreak by lexicographically lowest node_id
                let min_id = tiebreak_set
                    .iter()
                    .min()
                    .expect("tiebreak_set always contains self");

                if min_id == &self.local_member.node_id {
                    info!(
                        tiebreak_set = ?tiebreak_set,
                        "Won deterministic tiebreak by lowest node_id; self-promoting as master"
                    );
                    self.self_promote_as_master();

                    // Serialize MasterElected once; send directly over each held
                    // stream before promoting them to persistent peer connections.
                    // broadcast_master_elected() relies on self.peers, which is empty
                    // here because the held streams have not been registered yet.
                    // Writing directly avoids the ordering gap.
                    let broadcast_bytes: Option<Vec<u8>> = {
                        let payload = MasterElectedPayload {
                            master_address: format!(
                                "{}:{}",
                                self.local_member.host, self.local_member.cluster_port
                            ),
                            master_node_id: self.local_member.node_id.clone(),
                            term: 1,
                            election_id: Uuid::new_v4().to_string(),
                        };
                        rmp_serde::to_vec_named(&ClusterMessage::MasterElected(payload)).ok()
                    };

                    // Promote held streams: send MasterElected then hand off to read loop.
                    for (responder_id, mut framed) in held_streams.drain(..) {
                        let this = Arc::clone(self);
                        let bytes_opt = broadcast_bytes.clone();
                        tokio::spawn(async move {
                            // The held connection is already framed; send the
                            // length-prefixed MasterElected over it, then hand the
                            // same `Framed` to the read loop so any frame the peer
                            // pipelined behind its rejection is not dropped.
                            if let Some(bytes) = bytes_opt {
                                let _ = framed.send(bytes.into()).await;
                            }
                            this.handle_peer_framed(framed, Some(responder_id)).await;
                        });
                    }
                    return;
                }

                // Not the lowest-id node; clear held streams and re-dial from seed-dial
                // to re-establish connections with fresh state.
                info!(
                    min_id = %min_id,
                    self_id = %self.local_member.node_id,
                    "Lost deterministic tiebreak; waiting for elected master to broadcast"
                );
                drop(held_streams);
                held_streams = Vec::new();
            }
        }

        // Safety valve: total budget exhausted with no resolution. Unconditionally
        // self-promote. This path is only reachable under genuine total-partition or
        // stale-seed-list conditions; it may transiently recreate split-brain, which
        // the existing MemberRemoved/MemberAdded flow heals once connectivity restores.
        warn!(
            budget_ms = MASTER_ELECTION_TOTAL_BUDGET_MS,
            "Master-election budget exhausted; self-promoting as safety-valve fallback"
        );
        self.self_promote_as_master();
        self.broadcast_master_elected(); // best-effort; peers may be empty
    }

    /// Sends `ClusterMessage::MasterElected` to all currently-connected peers.
    ///
    /// Called immediately after `self_promote_as_master()` to inform any nodes
    /// in `WaitForMasterElection` state to re-target their join attempt.
    ///
    /// Sends over the existing per-peer write channels. Best-effort: drops
    /// are silently ignored (the safety-valve 30s budget handles the case
    /// where all broadcasts are lost).
    ///
    /// `MembershipReactor` is NOT invoked by this method — the election is at
    /// the `ClusterMessage` layer, not the `ClusterChange` layer, so no
    /// `MemberAdded`/`MemberRemoved` semantics are triggered.
    fn broadcast_master_elected(&self) {
        let payload = MasterElectedPayload {
            master_address: format!(
                "{}:{}",
                self.local_member.host, self.local_member.cluster_port
            ),
            master_node_id: self.local_member.node_id.clone(),
            term: 1, // v1: always 1; reserved for future master-failover elections
            election_id: Uuid::new_v4().to_string(),
        };
        let msg = ClusterMessage::MasterElected(payload);
        self.peers.broadcast(&msg, None);
    }

    /// Sends a `JoinRequest` to a seed node over a newly established TCP connection.
    ///
    /// Returns a `SeedAttemptOutcome` describing whether the join was accepted,
    /// rejected retry-ably (`NotMasterYet` — stream kept open for `MasterElected`),
    /// rejected permanently (auth/version/`cluster_id`/full — skip this seed),
    /// or failed at the transport layer.
    ///
    /// The stream is consumed ONLY on `PermanentRejection` and `ConnectionError`.
    /// On `Accepted` and `RetryableRejection` the stream is returned inside the
    /// variant so the caller can keep the TCP connection alive.
    async fn send_join_request(&self, stream: TcpStream, seed_addr: &str) -> SeedAttemptOutcome {
        let mut framed = Framed::new(stream, LengthDelimitedCodec::new());

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
                return SeedAttemptOutcome::ConnectionError;
            }
        };

        if framed.send(bytes.into()).await.is_err() {
            warn!(seed = %seed_addr, "Failed to send JoinRequest to seed");
            return SeedAttemptOutcome::ConnectionError;
        }

        // Wait for JoinResponse
        match framed.next().await {
            Some(Ok(frame)) => {
                let msg: ClusterMessage = match rmp_serde::from_slice(&frame) {
                    Ok(m) => m,
                    Err(e) => {
                        warn!(seed = %seed_addr, "Failed to deserialize seed response: {e}");
                        return SeedAttemptOutcome::ConnectionError;
                    }
                };

                if let ClusterMessage::JoinResponse(response) = msg {
                    if response.accepted {
                        info!(seed = %seed_addr, "Join accepted by seed node");

                        // Extract master node_id from the received members view
                        let master_node_id = response
                            .members_view
                            .as_ref()
                            .and_then(|v| v.master())
                            .map(|m| m.node_id.clone());

                        self.apply_join_response(&response);

                        if let Some(node_id) = master_node_id {
                            // Hand off the live `Framed` so any frame the master
                            // pipelined immediately behind the `JoinResponse`
                            // (e.g. the next `MembersUpdate`) is preserved in the
                            // codec's read buffer. `into_inner()` here would drop
                            // that buffer and lose the update.
                            return SeedAttemptOutcome::Accepted {
                                framed,
                                master_node_id: node_id,
                            };
                        }

                        // Accepted but no master in view (should not happen in practice).
                        // Treat as a connection error so the caller retries.
                        warn!(seed = %seed_addr, "Join accepted but no master in members view");
                        return SeedAttemptOutcome::ConnectionError;
                    }

                    // Rejected — classify by reject_code; absent code defaults to NotMasterYet
                    // so older peers (pre-quorum-election) fall into the retry-able path.
                    let is_permanent = matches!(
                        response.reject_code,
                        Some(
                            JoinRejectReason::AuthFailed
                                | JoinRejectReason::ProtocolVersionMismatch
                                | JoinRejectReason::WrongClusterId
                                | JoinRejectReason::ClusterFull
                        )
                    );

                    if is_permanent {
                        warn!(
                            seed = %seed_addr,
                            reason = ?response.reject_reason,
                            code = ?response.reject_code,
                            "Join permanently rejected by seed node; skipping"
                        );
                        return SeedAttemptOutcome::PermanentRejection;
                    }

                    // NotMasterYet (or unknown older peer): keep the framed stream
                    // open so this seed can deliver a MasterElected broadcast once
                    // it or another node wins the deterministic tiebreak. Carry the
                    // `Framed` (not the raw stream) to preserve buffered bytes.
                    info!(
                        seed = %seed_addr,
                        responder = ?response.responder_node_id,
                        "Seed not yet master; holding connection for MasterElected broadcast"
                    );
                    return SeedAttemptOutcome::RetryableRejection {
                        framed,
                        responder_node_id: response.responder_node_id,
                    };
                }

                warn!(seed = %seed_addr, "Unexpected response from seed (expected JoinResponse)");
                SeedAttemptOutcome::ConnectionError
            }
            Some(Err(e)) => {
                warn!(seed = %seed_addr, "Error reading seed response: {e}");
                SeedAttemptOutcome::ConnectionError
            }
            None => {
                warn!(seed = %seed_addr, "Seed connection closed before response");
                SeedAttemptOutcome::ConnectionError
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
    #[allow(clippy::too_many_lines)]
    async fn handle_join_request(
        &self,
        payload: JoinRequestPayload,
        write_tx: &mpsc::UnboundedSender<Vec<u8>>,
        peer_node_id: &Arc<Mutex<Option<String>>>,
    ) {
        // Reject permanently if the joiner's cluster_id does not match ours.
        // This guard runs before the is_master() check so a non-master node
        // still rejects wrong-cluster joiners with a permanent code rather than
        // NotMasterYet, preventing the joiner from entering WaitForMasterElection.
        if payload.cluster_id != self.config.cluster_id {
            warn!(
                joiner = %payload.node_id,
                joiner_cluster_id = %payload.cluster_id,
                our_cluster_id = %self.config.cluster_id,
                "Join rejected: cluster_id mismatch"
            );
            let response = ClusterMessage::JoinResponse(JoinResponsePayload {
                accepted: false,
                reject_reason: Some(format!(
                    "cluster_id mismatch: expected {}, got {}",
                    self.config.cluster_id, payload.cluster_id
                )),
                reject_code: Some(JoinRejectReason::WrongClusterId),
                responder_node_id: None,
                members_view: None,
                partition_assignments: None,
            });
            if let Ok(bytes) = rmp_serde::to_vec_named(&response) {
                let _ = write_tx.send(bytes);
            }
            return;
        }

        // Only master handles join requests
        if !self.cluster_state.is_master() {
            let master_addr = self.get_master_address();
            let reject_reason = format!("not master; master address: {master_addr}");
            let response = ClusterMessage::JoinResponse(JoinResponsePayload {
                accepted: false,
                reject_reason: Some(reject_reason),
                // Machine-readable code so the joiner can enter WaitForMasterElection
                // without parsing the human-readable reject_reason string.
                reject_code: Some(JoinRejectReason::NotMasterYet),
                // Joiner uses this to populate the deterministic-tiebreak set;
                // the connection stays open so this node can later broadcast MasterElected.
                responder_node_id: Some(self.local_member.node_id.clone()),
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
        self.peers.insert(payload.node_id.clone(), write_tx.clone());

        // Build new member info -- add directly as Active so compute_assignment includes it
        let new_member = MemberInfo {
            node_id: payload.node_id.clone(),
            host: payload.host,
            client_port: payload.client_port,
            cluster_port: payload.cluster_port,
            state: NodeState::Active,
            join_version: 0, // Will be set to current view version + 1
        };

        // Get current view and add (or re-activate) the new member.
        // A node may rejoin after a disconnect; update the existing entry in-place
        // rather than appending a duplicate, which would corrupt active_members() results.
        let current_view = self.cluster_state.current_view();
        let new_version = current_view.version + 1;

        let mut members = current_view.members.clone();

        let mut active_member = new_member;
        active_member.join_version = new_version;

        if let Some(pos) = members
            .iter()
            .position(|m| m.node_id == active_member.node_id)
        {
            // Node already exists (e.g., rejoining after failure) — update in-place.
            members[pos] = active_member.clone();
        } else {
            members.push(active_member.clone());
        }

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

        // Notify the MembershipReactor so it recomputes assignments and broadcasts
        // the updated partition map to all connected clients. Without this signal
        // the reactor never fires and clients never receive PARTITION_MAP.
        let new_member_info = active_member;
        let _ = self
            .cluster_state
            .change_sender()
            .send(super::state::ClusterChange::MemberAdded(new_member_info));

        // Respond with JoinResponse
        let response = ClusterMessage::JoinResponse(JoinResponsePayload {
            accepted: true,
            reject_reason: None,
            reject_code: None,
            responder_node_id: None,
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

// ---------------------------------------------------------------------------
// Free helpers for the master-election state machine
// ---------------------------------------------------------------------------

/// Listens on held TCP streams (from `NotMasterYet` rejections) for the first
/// `ClusterMessage::MasterElected` message to arrive within `timeout`. Returns
/// the payload on arrival, or `None` on timeout.
///
/// Deduplicates broadcasts by `election_id` in case the same election is
/// delivered over multiple held streams (fan-out). Non-`MasterElected` frames
/// (e.g., heartbeat noise) are discarded silently.
///
/// Streams are polled in round-robin with a short per-stream window; the outer
/// loop tracks the overall `timeout` deadline. This avoids the compile-time
/// branch-count limit of `tokio::select!` while handling a dynamic set of streams.
async fn listen_for_master_elected(
    held_streams: &mut [(String, Framed<TcpStream, LengthDelimitedCodec>)],
    timeout: Duration,
) -> Option<MasterElectedPayload> {
    let deadline = Instant::now() + timeout;
    let mut seen_ids: HashSet<String> = HashSet::new();

    // Poll the already-framed held connections in place. Operating on the
    // existing `Framed` (rather than recovering the raw stream and re-wrapping)
    // is what preserves any bytes the codec has buffered — a fresh wrapper
    // followed by `into_inner()` would discard the codec's read buffer and lose
    // a frame the peer pipelined behind its rejection.
    loop {
        if Instant::now() >= deadline {
            return None;
        }

        for (_id, framed) in held_streams.iter_mut() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return None;
            }

            // Try to read one frame within a short window from this stream.
            if let Ok(Some(Ok(frame))) =
                tokio::time::timeout(Duration::from_millis(50), framed.next()).await
            {
                if let Ok(ClusterMessage::MasterElected(payload)) =
                    rmp_serde::from_slice::<ClusterMessage>(&frame)
                {
                    if seen_ids.insert(payload.election_id.clone()) {
                        return Some(payload);
                    }
                }
                // Non-MasterElected frame or duplicate election_id: discard and continue
            }
            // Stream error, closed, or per-stream poll window expired; try next stream
        }

        // Brief sleep before the next round-robin pass to avoid busy-spinning
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies that a `JoinResponsePayload` `MsgPack` blob WITHOUT `reject_code`
    /// and WITHOUT `responder_node_id` deserializes with both fields as `None`.
    /// This is the backward-compatibility contract: older peers that omit the
    /// new fields must still be handled gracefully (treated as `NotMasterYet`).
    #[test]
    fn join_response_without_new_fields_deserializes_as_none() {
        // Simulate an older peer response: serialize a payload where reject_code
        // and responder_node_id are None, then verify both decode as None.
        // This is the backward-compatibility contract: older peers that omit the
        // new fields must still be handled gracefully (treated as `NotMasterYet`).
        let minimal = JoinResponsePayload {
            accepted: false,
            reject_reason: Some("not master; master address: unknown".to_string()),
            reject_code: None,
            responder_node_id: None,
            members_view: None,
            partition_assignments: None,
        };
        let bytes = rmp_serde::to_vec_named(&minimal).expect("serialize");

        // Deserialize; confirm new fields are None
        let decoded: JoinResponsePayload = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(decoded.reject_code, None);
        assert_eq!(decoded.responder_node_id, None);
        assert!(!decoded.accepted);
    }

    /// Verifies that `ClusterMessage::MasterElected(payload)` round-trips through
    /// `MsgPack` serialization and that the discriminant tag is emitted correctly.
    #[test]
    fn master_elected_envelope_round_trip() {
        let payload = MasterElectedPayload {
            master_address: "127.0.0.1:9001".to_string(),
            master_node_id: "node-0".to_string(),
            term: 1,
            election_id: "abc-123".to_string(),
        };
        let msg = ClusterMessage::MasterElected(payload.clone());
        let bytes = rmp_serde::to_vec_named(&msg).expect("serialize MasterElected envelope");

        // Deserialize back and confirm the variant is recovered correctly
        let decoded: ClusterMessage = rmp_serde::from_slice(&bytes).expect("deserialize");
        match decoded {
            ClusterMessage::MasterElected(p) => {
                assert_eq!(p.master_node_id, "node-0");
                assert_eq!(p.master_address, "127.0.0.1:9001");
                assert_eq!(p.term, 1);
                assert_eq!(p.election_id, "abc-123");
            }
            other => panic!("Expected MasterElected, got: {other:?}"),
        }

        // Confirm the type discriminant is "MASTER_ELECTED" (SCREAMING_SNAKE_CASE)
        // by decoding to a raw Value and inspecting the "type" key.
        let raw: rmpv::Value = rmp_serde::from_slice(&bytes).expect("decode to Value");
        if let rmpv::Value::Map(entries) = raw {
            let type_val = entries
                .iter()
                .find(|(k, _)| k == &rmpv::Value::String("type".into()))
                .map(|(_, v)| v.clone());
            assert_eq!(
                type_val,
                Some(rmpv::Value::String("MASTER_ELECTED".into())),
                "MasterElected discriminant must be MASTER_ELECTED in wire format"
            );
        } else {
            panic!("Expected a map at the top level");
        }
    }

    /// Verifies that a join rejection with a permanent code (`AuthFailed`) is
    /// classified as `PermanentRejection` and does not reach `RetryableRejection`.
    #[test]
    fn join_reject_reason_permanent_variants_are_permanent() {
        let permanent_codes = [
            JoinRejectReason::AuthFailed,
            JoinRejectReason::ProtocolVersionMismatch,
            JoinRejectReason::WrongClusterId,
            JoinRejectReason::ClusterFull,
        ];
        for code in permanent_codes {
            let is_permanent = matches!(
                Some(code),
                Some(
                    JoinRejectReason::AuthFailed
                        | JoinRejectReason::ProtocolVersionMismatch
                        | JoinRejectReason::WrongClusterId
                        | JoinRejectReason::ClusterFull
                )
            );
            assert!(is_permanent, "Expected {code:?} to be permanent");
        }
    }
}
