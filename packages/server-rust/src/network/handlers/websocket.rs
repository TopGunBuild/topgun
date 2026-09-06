//! WebSocket upgrade handler with inbound/outbound message loops.
//!
//! Uses the socket-split pattern: the WebSocket is split into a sender
//! (owned by the outbound task) and a receiver (owned by the inbound loop).
//! This avoids holding a single mutable reference across concurrent reads
//! and writes.
//!
//! Authentication is two-phase: Phase 1 reads messages sequentially until
//! the connection is authenticated (or the connection closes). Phase 2
//! spawns each dispatch task concurrently, bounded by a semaphore, so the
//! reader can continue consuming frames while previous dispatches are
//! still in flight. If no JWT secret is configured, Phase 1 is skipped.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use axum::extract::ws::{CloseFrame, Message, WebSocket};
use axum::extract::State;
use axum::response::Response;
use futures_util::sink::SinkExt;
use futures_util::stream::{SplitSink, StreamExt};
use tokio::sync::mpsc;
use topgun_core::hash_to_partition;
use topgun_core::messages::{
    AuthAckData, ClientOp, DeviceAckData, ErrorPayload, Message as TopGunMessage, OpAckMessage,
    OpAckPayload, WriteConcern,
};
use tracing::{debug, warn};

use super::auth::AuthHandler;
use super::decode;
use super::AppState;
use crate::network::connection::{ConnectionHandle, ConnectionId};
use crate::network::device_identity::{frontier_client_id, DeviceIdentityStore};
use crate::network::{ConnectionKind, OutboundMessage};
use crate::service::classify::OperationService;
use crate::service::dispatch::PartitionDispatcher;
use crate::service::operation::{
    CallerOrigin, ClassifyError, ErrorDisposition, OpOutcome, OpVerdict, Operation, OperationError,
    OperationResponse, ALL_ERROR_KINDS,
};
use topgun_core::Principal;

/// Maximum number of in-flight dispatch tasks per connection.
///
/// Each spawned task holds a semaphore permit until its dispatch completes.
/// This bounds memory and task overhead: at 6-11µs per op, 32 slots is
/// ~200µs of parallelism — enough to saturate the pipeline without
/// accumulating an unbounded backlog.
const MAX_IN_FLIGHT: usize = 32;

/// Upgrades an HTTP connection to a WebSocket connection.
///
/// Configures write buffer sizes from the connection config, then hands
/// off to `handle_socket` for the message processing loops.
pub async fn ws_upgrade_handler(
    State(state): State<AppState>,
    ws: axum::extract::ws::WebSocketUpgrade,
) -> Response {
    ws.write_buffer_size(state.config.connection.ws_write_buffer_size)
        .max_write_buffer_size(state.config.connection.ws_max_write_buffer_size)
        // Cap inbound message/frame size so an unauthenticated client cannot force
        // a large allocation (tungstenite defaults are 64 MiB / 16 MiB) and so the
        // depth-checked decoder only ever sees bounded frames.
        .max_message_size(state.config.connection.ws_max_message_size)
        .max_frame_size(state.config.connection.ws_max_frame_size)
        .on_upgrade(|socket| handle_socket(socket, state))
}

/// Processes a connected WebSocket: registers it, runs message loops, and
/// cleans up on disconnect.
///
/// Two-phase message processing:
/// - Phase 1 (auth): reads messages sequentially until authenticated. If no
///   JWT secret is configured the connection skips directly to Phase 2.
/// - Phase 2 (pipeline): each binary frame spawns a concurrent dispatch task
///   bounded by `MAX_IN_FLIGHT` semaphore permits.
///
/// On exit, the semaphore is closed and drained to ensure all in-flight
/// tasks complete before the connection handle (and its outbound sender)
/// is dropped, allowing the outbound task to flush cleanly.
#[allow(clippy::too_many_lines)]
async fn handle_socket(mut socket: WebSocket, state: AppState) {
    let (handle, rx) = state
        .registry
        .register(ConnectionKind::Client, &state.config.connection);
    let conn_id = handle.id;

    debug!("WebSocket connected: {:?}", conn_id);

    // Register the refusal series at zero before this connection can write
    // anything, so a scrape taken between connect and first write reads an
    // explicit zero rather than an absent series. Cost is one `Once` check per
    // connection and nothing per operation.
    register_client_op_refusal_series();

    // Send AUTH_REQUIRED before splitting the socket, so the client
    // knows to authenticate before sending any other messages.
    if let Some(ref secret) = state.jwt_secret {
        let auth_handler = AuthHandler::new(secret.clone(), state.auth_validator.clone());
        if let Err(e) = auth_handler.send_auth_required(&mut socket).await {
            warn!("failed to send AUTH_REQUIRED to {:?}: {}", conn_id, e);
            release_session_state(&state, conn_id);
            state.registry.remove(conn_id);
            return;
        }
    }

    let (sender, mut receiver) = socket.split();

    // Outbound task owns the write half of the socket and drains
    // the mpsc channel, coalescing messages when multiple are ready.
    let outbound_handle = tokio::spawn(outbound_task(sender, rx));

    // Tracks auth state for phase switching. AtomicBool avoids RwLock
    // contention in Phase 2 — set once in Phase 1, read once to decide
    // which phase to enter. handle.metadata is still written during Phase 1
    // so domain services can read the principal.
    let authenticated = AtomicBool::new(false);

    // Semaphore limits in-flight dispatch tasks to MAX_IN_FLIGHT.
    // Closed on shutdown to unblock any pending acquire.
    let semaphore = Arc::new(tokio::sync::Semaphore::new(MAX_IN_FLIGHT));

    // Cancellation signal for forced teardown by the reaper. Cloned (cheap —
    // Arc inside) so both read phases can select on it without borrowing the
    // handle across the moves they perform on the disconnect path.
    let cancel = handle.cancel.clone();

    // Phase 1: sequential auth — only proceed when JWT secret is configured.
    // If no secret is set, every connection is pre-authenticated.
    if state.jwt_secret.is_some() {
        // Bound the auth handshake: a client that connects and never finishes
        // authenticating (slowloris) must not hold a connection slot forever.
        // The deadline is enforced per read; malformed frames `continue` the
        // loop but do not reset it.
        let auth_deadline = tokio::time::Instant::now() + state.config.connection.auth_timeout;
        'auth: loop {
            let next = tokio::select! {
                biased;
                () = cancel.cancelled() => {
                    debug!("connection {:?} reaped during auth phase", conn_id);
                    semaphore.close();
                    drop(handle);
                    join_outbound_with_timeout(outbound_handle).await;
                    release_session_state(&state, conn_id);
                    state.registry.remove(conn_id);
                    debug!("WebSocket disconnected: {:?}", conn_id);
                    return;
                }
                () = tokio::time::sleep_until(auth_deadline) => {
                    debug!(
                        "connection {:?} exceeded auth deadline; closing (slowloris guard)",
                        conn_id
                    );
                    semaphore.close();
                    drop(handle);
                    join_outbound_with_timeout(outbound_handle).await;
                    release_session_state(&state, conn_id);
                    state.registry.remove(conn_id);
                    debug!("WebSocket disconnected: {:?}", conn_id);
                    return;
                }
                msg = receiver.next() => msg,
            };
            match next {
                Some(Ok(Message::Binary(data))) => {
                    // Depth-checked decode BEFORE auth: our version-independent
                    // guard against an unbounded recursive decode (a deeply-nested
                    // frame), which on a codec without an internal cap would
                    // stack-overflow and abort the whole node from an
                    // unauthenticated client. Over-deep/malformed frames are
                    // dropped, not fatal.
                    let tg_msg = match decode::decode_depth_checked::<TopGunMessage>(&data) {
                        Ok(msg) => msg,
                        Err(e) => {
                            debug!("failed to deserialize message from {:?}: {}", conn_id, e);
                            continue;
                        }
                    };

                    if let TopGunMessage::Auth(ref auth_msg) = tg_msg {
                        if let Some(ref secret) = state.jwt_secret {
                            let auth_handler =
                                AuthHandler::new(secret.clone(), state.auth_validator.clone())
                                    .with_issuer_audience(
                                        state.config.jwt_issuer.clone(),
                                        state.config.jwt_audience.clone(),
                                    );
                            match auth_handler
                                .handle_auth(
                                    auth_msg,
                                    &handle.tx,
                                    state.config.jwt_clock_skew_secs,
                                    state.config.insecure_forward_auth_errors,
                                )
                                .await
                            {
                                Ok(principal) => {
                                    // Store principal in metadata so domain services can read it.
                                    // AtomicBool is set after metadata write so no reader sees
                                    // authenticated=true without principal being set.
                                    {
                                        let mut meta = handle.metadata.write().await;
                                        meta.authenticated = true;
                                        meta.principal = Some(principal.clone());
                                    }
                                    authenticated.store(true, Ordering::Release);

                                    // Device identity present-or-mint (fail-open — never blocks
                                    // auth). One-shot here is structural: this arm runs once then
                                    // `break 'auth`s, so there is no second JWT-mode bind site.
                                    let (device_id, device_token) = bind_device_identity(
                                        &state,
                                        &handle,
                                        conn_id,
                                        Some(&principal.id),
                                        auth_msg.device_token.as_deref(),
                                    )
                                    .await;

                                    // Send AUTH_ACK with userId + any device identity via the
                                    // outbound channel.
                                    let ack_msg = TopGunMessage::AuthAck(AuthAckData {
                                        user_id: Some(principal.id.clone()),
                                        device_id,
                                        device_token,
                                        ..Default::default()
                                    });
                                    if let Ok(bytes) = rmp_serde::to_vec_named(&ack_msg) {
                                        let _ =
                                            handle.tx.send(OutboundMessage::Binary(bytes)).await;
                                    }

                                    debug!(
                                        user_id = %principal.id,
                                        "connection {:?} authenticated",
                                        conn_id
                                    );

                                    break 'auth;
                                }
                                Err(e) => {
                                    // AUTH_FAIL already sent by handle_auth; close connection
                                    debug!("auth failed for {:?}: {}", conn_id, e);
                                    // Drain semaphore and drop before returning
                                    semaphore.close();
                                    drop(handle);
                                    join_outbound_with_timeout(outbound_handle).await;
                                    release_session_state(&state, conn_id);
                                    state.registry.remove(conn_id);
                                    debug!("WebSocket disconnected: {:?}", conn_id);
                                    return;
                                }
                            }
                        }
                    } else {
                        // Drop non-AUTH messages from unauthenticated connections
                        debug!(
                            "dropping message from unauthenticated connection {:?}",
                            conn_id
                        );
                    }
                }
                Some(Ok(Message::Close(_))) | None => {
                    debug!("connection {:?} closed during auth phase", conn_id);
                    semaphore.close();
                    drop(handle);
                    join_outbound_with_timeout(outbound_handle).await;
                    release_session_state(&state, conn_id);
                    state.registry.remove(conn_id);
                    debug!("WebSocket disconnected: {:?}", conn_id);
                    return;
                }
                Some(Ok(Message::Text(_))) => {
                    warn!(
                        "ignoring text message from connection {:?} -- binary only",
                        conn_id
                    );
                }
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {
                    // Handled automatically by axum/tungstenite.
                }
                Some(Err(e)) => {
                    debug!(
                        "WebSocket error on connection {:?} during auth: {}",
                        conn_id, e
                    );
                    semaphore.close();
                    drop(handle);
                    join_outbound_with_timeout(outbound_handle).await;
                    release_session_state(&state, conn_id);
                    state.registry.remove(conn_id);
                    debug!("WebSocket disconnected: {:?}", conn_id);
                    return;
                }
            }
        }
    }

    // Resolve principal once for this connection so the authorization middleware
    // can read ctx.principal without performing a registry lookup per operation.
    // This is done after Phase 1 completes so the metadata is guaranteed to be set.
    // Also mark the handshake complete so the reaper switches this connection
    // from the auth-deadline bound to the idle (heartbeat) bound — true for
    // no-auth connections too, which skip Phase 1 entirely.
    let principal: Option<Principal> = {
        let mut meta = handle.metadata.write().await;
        meta.handshake_complete = true;
        meta.principal.clone()
    };

    // Per-connection inbound op-rate limiter (data-plane fairness). Aggregate
    // load shedding (MAX_IN_FLIGHT + worker-inbox Overloaded) bounds total work
    // but not a single abusive peer; this token bucket caps one connection's op
    // rate so a flood is throttled (429 back-off) without starving others or
    // tearing the connection down. Owned solely by this read loop — no locking.
    let mut rate_limiter = crate::network::rate_limit::TokenBucket::new(
        state.config.connection.data_plane_max_ops_per_sec,
        state.config.connection.data_plane_ops_burst,
        std::time::Instant::now(),
    );

    // Phase 2: pipeline mode — each binary frame is dispatched concurrently.
    // The reader continues immediately after spawning, so multiple frames
    // can be in-flight simultaneously up to MAX_IN_FLIGHT.
    loop {
        // Select on the cancel token too, so the reaper can unblock a reader
        // parked on a half-open socket (no FIN, no client traffic). On cancel
        // we fall through to the shared cleanup below.
        let next = tokio::select! {
            biased;
            () = cancel.cancelled() => {
                debug!("connection {:?} reaped (idle/half-open)", conn_id);
                break;
            }
            msg = receiver.next() => msg,
        };
        match next {
            Some(Ok(Message::Binary(data))) => {
                let tg_msg = match decode::decode_depth_checked::<TopGunMessage>(&data) {
                    Ok(msg) => msg,
                    Err(e) => {
                        debug!("failed to deserialize message from {:?}: {}", conn_id, e);
                        continue;
                    }
                };

                // Device-credential presentation, handled at the websocket layer (both
                // modes). A token-less client (NO_AUTH, or a JWT client before it has a
                // token) sends DEVICE_HELLO as its first frame instead of an empty-token
                // AUTH — the latter collides with JWT validation and a JWT server would
                // AUTH_FAIL + tear the connection down. DEVICE_HELLO is a distinct,
                // non-AUTH frame: a JWT server drops it in Phase 1 (never reaching here),
                // and here we run present-or-mint and reply DEVICE_ACK. It NEVER enters
                // data-plane dispatch. The identity namespace follows the connection's
                // authenticated principal (Some in the unlikely JWT-in-Phase-2 case, None
                // → sentinel in NO_AUTH).
                if let TopGunMessage::DeviceHello(ref hello) = tg_msg {
                    // One-shot binding (explicit Phase-2 guard): once an identity is bound,
                    // drop any further DEVICE_HELLO so in-flight identity-scoped state is
                    // never silently re-attributed. This also makes the handler a safe
                    // no-op for an already-authenticated JWT connection (bound in Phase 1).
                    // The read loop is sequential, so the first bind wins and later frames
                    // observe device_id = Some.
                    let already_bound = handle.metadata.read().await.device_id.is_some();
                    if already_bound {
                        debug!(
                            "dropping repeat DEVICE_HELLO on bound connection {:?}",
                            conn_id
                        );
                        continue;
                    }
                    let (device_id, device_token) = bind_device_identity(
                        &state,
                        &handle,
                        conn_id,
                        principal.as_ref().map(|p| p.id.as_str()),
                        hello.device_token.as_deref(),
                    )
                    .await;
                    // Reply DEVICE_ACK only when an identity was actually bound (a store
                    // must be wired); network-only test servers stay identity-less.
                    if device_id.is_some() {
                        let ack = TopGunMessage::DeviceAck(DeviceAckData {
                            device_id,
                            device_token,
                        });
                        if let Ok(bytes) = rmp_serde::to_vec_named(&ack) {
                            let _ = handle.tx.send(OutboundMessage::Binary(bytes)).await;
                        }
                    }
                    continue;
                }

                // Per-connection inbound op-rate limit. Cost = number of ops the
                // frame carries (a batch counts as its op count) so one peer's
                // flood is throttled fairly. On exceed we send a 429 back-off and
                // drop the frame — the connection stays up and recovers as tokens
                // refill (отбой, не падение).
                let op_cost = inbound_op_cost(&tg_msg);
                if !rate_limiter.try_consume(op_cost) {
                    debug!(
                        "rate limit exceeded for {:?} (op_cost={}); backing off",
                        conn_id, op_cost
                    );
                    let err_msg = TopGunMessage::Error {
                        payload: ErrorPayload {
                            code: 429,
                            message: "rate limit exceeded, slow down".to_string(),
                            details: None,
                        },
                    };
                    if let Ok(bytes) = rmp_serde::to_vec_named(&err_msg) {
                        // Best-effort, non-blocking: a flooding client is by
                        // definition behind on its outbound channel, so awaiting
                        // here would stall this read loop on the very connection
                        // we are throttling (and would gate its own recovery
                        // tokens). If the channel is full, dropping the 429 is
                        // fine — the client is already getting backpressure.
                        let _ = handle.try_send(OutboundMessage::Binary(bytes));
                    }
                    continue;
                }

                // Confirmed-apply ACK: advance the per-device causal frontier inline.
                // Identity-scoped and connection-ownership-fenced, so it is handled here
                // (it needs the connection's device identity + the ownership registry,
                // both in `state`/`handle`) rather than in the spawned data-plane
                // dispatch task. The read loop is sequential, so cursor monotonicity is
                // naturally preserved across a connection's ACK stream.
                if let TopGunMessage::ClientApplyAck(ref ack) = tg_msg {
                    handle_client_apply_ack(
                        &state,
                        &handle,
                        conn_id,
                        principal.as_ref(),
                        ack.cursor,
                    )
                    .await;
                    continue;
                }

                // Acquire a permit before spawning; if the semaphore is closed
                // (shutdown signal), exit the reader loop.
                let Ok(permit) = semaphore.clone().acquire_owned().await else {
                    break; // Semaphore closed — exit reader loop
                };

                let tx = handle.tx.clone();
                let op_service = state.operation_service.clone();
                let dispatcher = state.dispatcher.clone();
                let principal_clone = principal.clone();

                tokio::spawn(async move {
                    dispatch_message(tg_msg, conn_id, principal_clone, op_service, dispatcher, tx)
                        .await;
                    drop(permit); // Release after dispatch completes
                });
            }
            Some(Ok(Message::Close(_))) | None => {
                debug!("close frame received from connection {:?}", conn_id);
                break;
            }
            Some(Ok(Message::Text(_))) => {
                // TopGun uses binary MsgPack only; text messages are unexpected.
                warn!(
                    "ignoring text message from connection {:?} -- binary only",
                    conn_id
                );
            }
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => {
                // Ping/Pong are handled automatically by axum/tungstenite.
            }
            Some(Err(e)) => {
                debug!("WebSocket error on connection {:?}: {}", conn_id, e);
                break;
            }
        }
    }

    // Graceful shutdown: acquire all permits to wait for in-flight dispatch
    // tasks to complete. Each task holds one permit and drops it when done.
    // Once all permits are re-acquired, close the semaphore so any racing
    // acquire in the reader loop returns Err (defensive — loop has exited).
    for _ in 0..MAX_IN_FLIGHT {
        let _ = semaphore.acquire().await;
    }
    semaphore.close();

    // All in-flight tasks have completed; drop the handle to close handle.tx.
    // The outbound task will drain remaining buffered messages before exiting.
    drop(handle);

    // Wait for the outbound task to finish flushing, then abort it if it is
    // wedged so a stuck writer cannot linger holding the socket.
    join_outbound_with_timeout(outbound_handle).await;

    release_session_state(&state, conn_id);
    state.registry.remove(conn_id);
    debug!("WebSocket disconnected: {:?}", conn_id);
}

/// Runs device-credential present-or-mint and binds the resulting identity to the
/// connection (one-shot `device_id`) plus connection ownership (TAKEOVER).
///
/// Returns `(device_id, minted_token)` for the `AUTH_ACK`. **Fail-open:** a missing
/// store or any storage error leaves the connection identity-less (`device_id` stays
/// `None`) and NEVER blocks authentication — an attacker can always claim "no token"
/// anyway, so failing an honest user out buys no security.
///
/// `principal_id` is `Some` in JWT mode and `None` in `NO_AUTH` mode (keyed under the
/// frontier sentinel namespace). The caller guarantees the one-shot precondition
/// (JWT: structural single Phase-1 bind; `NO_AUTH`: explicit pre-call guard), so this
/// only sets `device_id` when it is still `None`.
async fn bind_device_identity(
    state: &AppState,
    handle: &Arc<ConnectionHandle>,
    conn_id: ConnectionId,
    principal_id: Option<&str>,
    presented: Option<&str>,
) -> (Option<String>, Option<String>) {
    let Some(factory) = state.store_factory.as_ref() else {
        return (None, None);
    };
    let dev_store = DeviceIdentityStore::new(factory.data_store());
    // present_or_mint tags the no-auth namespace structurally from the `None` principal;
    // no content sentinel is substituted here (a JWT `sub` can never forge the tag).
    match dev_store.present_or_mint(principal_id, presented).await {
        Ok(binding) => {
            {
                let mut meta = handle.metadata.write().await;
                if meta.device_id.is_none() {
                    meta.device_id = Some(binding.device_id.clone());
                }
            }
            // TAKEOVER: a valid credential for an already-owned identity displaces the
            // prior connection, which is closed so its in-flight identity-scoped
            // actions can be fenced out by `is_current_owner`.
            let identity_key = frontier_client_id(principal_id, &binding.device_id);
            if let Some(displaced) = state
                .registry
                .claim_device_ownership(identity_key.clone(), conn_id)
            {
                if let Some(old) = state.registry.get(displaced) {
                    old.cancel();
                }
                warn!(
                    "device-identity takeover on {:?}: displaced {:?}",
                    conn_id, displaced
                );
            }
            // Rehydrate any persisted confirmed-apply cursor for this KNOWN identity
            // into the in-memory frontier BEFORE any ACK, so a reconnecting device pins
            // the prune low-water-mark at its true cursor instead of falling through the
            // "unknown == forgotten" path (which pins nothing and would let the LWM jump
            // forward). A freshly-minted identity has no persisted cursor and correctly
            // stays untracked (unknown → gated). Best-effort: no store → no-op.
            if let Some(frontier) = state.frontier.as_ref() {
                frontier.rehydrate(&identity_key).await;
            }
            (Some(binding.device_id), binding.minted_token)
        }
        Err(e) => {
            debug!(
                "device present-or-mint failed for {:?}: {} (fail-open, identity-less)",
                conn_id, e
            );
            (None, None)
        }
    }
}

/// Handles a client→server confirmed-apply ACK: advances the per-device causal
/// frontier under the bounded, monotone, connection-ownership-fenced rule.
///
/// The replica identity is derived ENTIRELY from server-authenticated connection
/// state — the authenticated `principal` plus the server-issued `device_id` — never
/// from the wire (the ACK carries only `cursor`). The ACK is accepted ONLY from the
/// connection that currently OWNS the identity (342i `is_current_owner` fencing), so
/// a displaced (taken-over) connection's in-flight stale ACK cannot advance the
/// shared cursor past the current owner's durable state. Identity-less connections
/// and rejected stale ACKs are dropped at `debug` (not errors) — they pin nothing.
async fn handle_client_apply_ack(
    state: &AppState,
    handle: &Arc<ConnectionHandle>,
    conn_id: ConnectionId,
    principal: Option<&Principal>,
    claimed: u64,
) {
    let Some(frontier) = state.frontier.as_ref() else {
        return; // frontier not wired (network-only tests) — ACK is a no-op
    };
    let device_id = { handle.metadata.read().await.device_id.clone() };
    let Some(device_id) = device_id else {
        debug!(
            "dropping confirmed-apply ACK from identity-less {:?}",
            conn_id
        );
        return;
    };
    let client = frontier_client_id(principal.map(|p| p.id.as_str()), &device_id);
    if !state.registry.is_current_owner(&client, conn_id) {
        debug!(
            "rejecting stale confirmed-apply ACK from non-owner {:?}",
            conn_id
        );
        return;
    }
    if frontier.confirm_apply_ack(&client, claimed, conn_id).await {
        debug!(conn = ?conn_id, cursor = claimed, "confirmed-apply cursor advanced");
    }
}

/// Releases all session-scoped registry state for a disconnecting connection.
///
/// Invoked at every exit point in `handle_socket` BEFORE `registry.remove(conn_id)`
/// to ensure lock/topic/counter resources are freed even if the connection
/// closes without explicit release messages from the client.
///
/// Each registry field is `Option<Arc<_>>` — `None` is the in-test default
/// so this function is a no-op in test contexts that do not wire the registries.
/// Order: Lock -> Topic -> Counter -> Query -> Journal -> Search -> Hybrid
/// (mirrors the struct field declaration order).
fn release_session_state(state: &AppState, conn_id: ConnectionId) {
    if let Some(ref reg) = state.lock_registry {
        reg.release_on_disconnect(conn_id);
    }
    if let Some(ref reg) = state.topic_registry {
        reg.release_on_disconnect(conn_id);
    }
    if let Some(ref reg) = state.counter_registry {
        reg.release_on_disconnect(conn_id);
    }
    if let Some(ref reg) = state.query_registry {
        reg.unregister_by_connection(conn_id);
    }
    if let Some(ref reg) = state.journal_store {
        reg.unsubscribe_by_connection(conn_id);
    }
    // Removed-id vectors are intentionally dropped: disconnect cleanup needs no
    // downstream fan-out, only the registry-side removal.
    if let Some(ref reg) = state.search_registry {
        let _ = reg.unregister_by_connection(conn_id);
    }
    if let Some(ref reg) = state.hybrid_search_registry {
        let _ = reg.unregister_by_connection(conn_id);
    }
    // Drop this connection's per-connection `delivered` clamp state; the per-identity
    // cursors are untouched (they survive reconnect via rehydration).
    if let Some(ref frontier) = state.frontier {
        frontier.remove_connection(conn_id);
    }
}

/// Dispatches a single deserialized message through the operation pipeline.
///
/// Takes owned Arc and Sender so this function can be moved into a
/// `tokio::spawn` closure (satisfying the `'static` bound). Helpers called
/// from within this function borrow from its owned locals.
///
/// Handles BATCH messages by unpacking and routing each inner message
/// individually. Non-BATCH messages are classified, have `connection_id`
/// and `principal` set, and are routed through the pipeline. Each
/// `OperationResponse` variant is mapped to the appropriate outbound message(s).
/// Hard upper bound on the number of inner items a single transport `Batch`
/// frame may carry through `unpack_and_dispatch_batch`.
///
/// Bounds worst-case per-frame dispatch work: the 2 MB inbound frame cap lets a
/// `Batch.data` blob pack roughly 524K minimal 4-byte length-prefixed entries,
/// each of which would otherwise drive a decode + classify + dispatch. Capping
/// at `8_192` keeps per-frame dispatch cost bounded while sitting far above any
/// legitimate batch size (tens to low thousands), so honest clients never hit it.
const MAX_BATCH_INNER_ITEMS: usize = 8_192;

/// Counts the well-formed length-prefixed inner items packed into a transport
/// `Batch.data` blob, without decoding any inner payload.
///
/// This is the single source of truth for BOTH the rate-limiter charge and the
/// hard inner-item cap, so the charged count and the dispatched count agree by
/// construction. It mirrors `unpack_and_dispatch_batch`'s framing walk exactly:
/// read a 4-byte big-endian length prefix, skip that many bytes, repeat. A
/// truncated trailing prefix or payload ends the walk (the unpacker `break`s on
/// the same condition), so the count is exactly the number of complete framed
/// items the unpacker iterates over — i.e. the charge is never *smaller* than
/// what the unpacker would actually attempt to decode and dispatch, which is the
/// property the rate limiter relies on.
///
/// Comparisons are written against the remaining byte count (`data.len() -
/// offset`) rather than `offset + len` so an adversarial length prefix near
/// `u32::MAX` cannot overflow `usize` on a 32-bit target (where it would
/// otherwise wrap the truncation guard and spin the walk). `offset <= data.len()`
/// holds on every iteration, so the subtraction never underflows.
///
/// Pure pointer arithmetic over the already-in-memory slice: no heap allocation,
/// no inner decode.
fn count_batch_items(data: &[u8]) -> usize {
    let mut offset = 0;
    let mut count = 0;
    while data.len() - offset >= 4 {
        let len = u32::from_be_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]) as usize;
        offset += 4;
        if len > data.len() - offset {
            // Truncated payload: the unpacker stops here, so do we.
            break;
        }
        offset += len;
        count += 1;
    }
    count
}

/// Cost, in op-rate-limiter tokens, of an inbound frame.
///
/// A frame's cost is the number of individual ops it carries so a single peer
/// cannot evade the per-connection rate limit by packing many ops into one
/// `OpBatch`/`Batch` frame. For a transport `Batch` the declared `count` field
/// is attacker-controlled and is NOT trusted: the cost is the actual number of
/// length-prefixed inner items the unpacker would dispatch. Non-batch messages
/// cost one token. Always at least 1 so an empty batch still consumes a token
/// (and cannot be used to spin).
fn inbound_op_cost(msg: &TopGunMessage) -> u32 {
    let count = match msg {
        TopGunMessage::OpBatch(b) => b.payload.ops.len(),
        TopGunMessage::Batch(b) => count_batch_items(&b.data),
        _ => 1,
    };
    u32::try_from(count.max(1)).unwrap_or(u32::MAX)
}

async fn dispatch_message(
    tg_msg: TopGunMessage,
    conn_id: ConnectionId,
    principal: Option<Principal>,
    operation_service: Option<Arc<OperationService>>,
    dispatcher: Option<Arc<PartitionDispatcher>>,
    tx: mpsc::Sender<OutboundMessage>,
) {
    let (Some(classify_svc), Some(dispatcher)) = (operation_service, dispatcher) else {
        debug!(
            "dispatcher not configured, dropping message from {:?}",
            conn_id
        );
        return;
    };

    // Handle BATCH messages: unpack each inner message and route individually
    if let TopGunMessage::Batch(ref batch_msg) = tg_msg {
        unpack_and_dispatch_batch(
            batch_msg,
            conn_id,
            principal,
            &classify_svc,
            &dispatcher,
            &tx,
        )
        .await;
        return;
    }

    // Intercept OpBatch messages before generic classify/dispatch.
    // Split by partition so each sub-batch runs on a dedicated partition worker
    // rather than serializing all ops on the single global worker.
    if let TopGunMessage::OpBatch(ref batch_msg) = tg_msg {
        dispatch_op_batch(
            batch_msg,
            conn_id,
            principal,
            &classify_svc,
            &dispatcher,
            &tx,
        )
        .await;
        return;
    }

    // Classify the message into a typed Operation
    match classify_svc.classify(tg_msg, None, CallerOrigin::Client) {
        Ok(mut op) => {
            // Set connection_id for domain services (subscription tracking, heartbeat).
            // Set principal so the authorization middleware can evaluate RBAC without
            // a registry lookup.
            op.set_connection_id(conn_id);
            if let Some(p) = principal.clone() {
                op.set_principal(p);
            }

            // Route through the partition dispatcher (MPSC channel per worker)
            match dispatcher.dispatch(op).await {
                Ok(resp) => {
                    send_operation_response(resp, &tx).await;
                }
                Err(OperationError::Overloaded) => {
                    // Worker inbox is full; tell the client to back off and retry.
                    let err_msg = TopGunMessage::Error {
                        payload: ErrorPayload {
                            code: 429,
                            message: "server overloaded, try again later".to_string(),
                            details: None,
                        },
                    };
                    if let Ok(bytes) = rmp_serde::to_vec_named(&err_msg) {
                        let _ = tx.send(OutboundMessage::Binary(bytes)).await;
                    }
                }
                Err(e) => {
                    debug!("dispatch error for {:?}: {}", conn_id, e);
                }
            }
        }
        Err(ClassifyError::TransportEnvelope { variant }) => {
            // BATCH messages should be caught above; log if another envelope type appears
            debug!(
                "unexpected transport envelope '{}' from {:?}",
                variant, conn_id
            );
        }
        Err(ClassifyError::AuthMessage { variant }) => {
            // AUTH messages from authenticated connections are unexpected
            debug!(
                "ignoring auth message '{}' from already-authenticated {:?}",
                variant, conn_id
            );
        }
        Err(ClassifyError::ServerToClient { variant }) => {
            // Client should not send server-to-client messages
            debug!(
                "ignoring server-to-client message '{}' from {:?}",
                variant, conn_id
            );
        }
    }
}

/// Which transport a verdict was produced on.
///
/// An enum rather than a string because it is a closed value set that ends up as
/// a metric label: two transports spelling the same label differently would
/// silently split one series into two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransportKind {
    /// Persistent client `WebSocket` connection.
    WebSocket,
    /// One-shot HTTP `/sync` request.
    Http,
}

impl TransportKind {
    /// The metric label for this transport.
    pub(crate) fn as_label(self) -> &'static str {
        match self {
            Self::WebSocket => "ws",
            Self::Http => "http",
        }
    }
}

/// Everything the verdict fold needs to dispatch a batch and then re-dispatch a
/// single operation under the *same* contract as the batch pass.
///
/// A struct rather than a parameter list because both transports must supply the
/// identical set, and because `write_concern` / `timeout` travelling together
/// with the dispatcher is what stops a singleton being judged under a different
/// contract than the batch attempt that failed.
pub(crate) struct OpBatchDispatchContext<'a> {
    /// Builds the per-partition `Operation::OpBatch` values.
    pub classify_svc: &'a OperationService,
    /// Routes each operation to its partition worker.
    pub dispatcher: &'a Arc<PartitionDispatcher>,
    /// Transport this batch arrived on; becomes the refusal metric's label.
    pub transport: TransportKind,
    /// Caller origin recorded on every dispatched operation.
    pub caller_origin: CallerOrigin,
    /// Client identifier, where the transport knows one.
    pub client_id: Option<String>,
    /// Authenticated principal, used by the authorization middleware.
    pub principal: Option<Principal>,
    /// Connection the batch arrived on, where the transport has one.
    pub connection_id: Option<ConnectionId>,
    /// Write concern of the originating batch. The singleton pass MUST reuse it.
    pub write_concern: Option<WriteConcern>,
    /// Timeout of the originating batch. The singleton pass MUST reuse it.
    pub timeout: Option<u64>,
}

// ---------------------------------------------------------------------------
// Per-operation refusal metric
// ---------------------------------------------------------------------------

/// Counter: permanent per-operation refusals, labelled by `transport` and `reason`.
///
/// Deliberately a different series from `topgun_operation_errors_total`, which
/// counts failed *pipeline calls*: one refused operation raises that counter
/// twice (the batch call plus the singleton re-dispatch that names the operation)
/// and raises this one exactly once. Reading a refusal count off the
/// pipeline-error counter is the confusion this series exists to prevent.
pub(crate) const METRIC_CLIENT_OP_REFUSALS_TOTAL: &str = "topgun_client_op_refusals_total";

/// Guards the one-time registration of the refusal series' whole label space.
static REFUSAL_SERIES_TOUCHED: std::sync::Once = std::sync::Once::new();

/// Registers every `{transport, reason}` refusal series at zero, once per process.
///
/// A counter that has never been incremented does not render on `/metrics` at
/// all, so without this an operator cannot tell "nothing was refused" from "the
/// refusal instrument is missing" — and reading an absent series as a zero is
/// exactly how a broken instrument passes for a healthy server.
///
/// Called from each transport's connection-setup path rather than at boot, so it
/// runs before that transport's first write while still costing one `Once` check
/// per connection and nothing at all per operation.
pub(crate) fn register_client_op_refusal_series() {
    REFUSAL_SERIES_TOUCHED.call_once(touch_client_op_refusal_series);
}

/// Touches every `transport × reason` series once.
///
/// Split from the `Once` guard so a test can drive it under its own recorder: a
/// process-lifetime `Once` fires for whichever recorder happens to be bound
/// first, which would make an assertion about the registration unrepeatable.
///
/// The label space is the full cross-product, not one series per transport: a
/// query for `{transport="ws", reason="forbidden"}` reads a different series from
/// one carrying `transport` alone, and would find it absent.
fn touch_client_op_refusal_series() {
    for transport in [TransportKind::WebSocket, TransportKind::Http] {
        for reason in ALL_ERROR_KINDS {
            // An `increment(0)` is what puts the series in the exporter's
            // registry; merely resolving the handle does not.
            metrics::counter!(
                METRIC_CLIENT_OP_REFUSALS_TOTAL,
                "transport" => transport.as_label(),
                "reason" => reason,
            )
            .increment(0);
        }
    }
}

// ---------------------------------------------------------------------------
// Verdict fold
// ---------------------------------------------------------------------------

impl OpBatchDispatchContext<'_> {
    /// Builds one dispatchable sub-batch operation under this batch's contract.
    ///
    /// Every field the batch pass used is reapplied here, write concern and
    /// timeout included, so a singleton re-dispatch is judged under the same
    /// contract as the batch attempt that failed. A singleton judged under a
    /// different contract could refuse an operation the batch would have accepted
    /// — and the client would retire a write that was never really refused.
    fn sub_batch_operation(
        &self,
        ops: Vec<topgun_core::messages::ClientOp>,
        partition_id: u32,
    ) -> Operation {
        let mut op = self.classify_svc.classify_op_batch_for_partition(
            ops,
            partition_id,
            self.client_id.clone(),
            self.caller_origin,
            self.write_concern.clone(),
            self.timeout,
        );
        if let Some(connection_id) = self.connection_id {
            op.set_connection_id(connection_id);
        }
        if let Some(principal) = self.principal.clone() {
            op.set_principal(principal);
        }
        op
    }
}

/// Precedence of a transient error for the single `ERROR` frame, lowest first.
///
/// `Unauthorized > Overloaded > Timeout > Internal` — auth first because it names
/// the most actionable cause. A total order at all is the point: the alternative
/// is "whichever sub-batch finished last", which makes the error the client sees
/// depend on a task-completion race.
///
/// Exhaustive with no `_` arm, so a new variant has to be placed in the order
/// deliberately.
fn transient_precedence(error: &OperationError) -> u8 {
    match error {
        OperationError::Unauthorized => 0,
        OperationError::Overloaded => 1,
        OperationError::Timeout { .. } => 2,
        OperationError::Internal(_) => 3,
        // Permanent variants never reach the transient sink. Ranking them last
        // keeps the match exhaustive without a catch-all arm.
        OperationError::Forbidden { .. }
        | OperationError::SchemaInvalid { .. }
        | OperationError::ValueTooLarge { .. }
        | OperationError::WrongService
        | OperationError::UnknownService { .. } => u8::MAX,
    }
}

/// Precedence of a non-attributed permanent error for the single `ERROR` frame.
///
/// `Forbidden > SchemaInvalid > ValueTooLarge > WrongService > UnknownService`.
/// The last two are unreachable for an operation batch — the classifier hardcodes
/// the CRDT service name and the server registers it unconditionally — and are
/// kept in the order so the match needs no `_` arm, which is what makes a future
/// variant a compile error here.
fn batch_error_precedence(error: &OperationError) -> u8 {
    match error {
        OperationError::Forbidden { .. } => 0,
        OperationError::SchemaInvalid { .. } => 1,
        OperationError::ValueTooLarge { .. } => 2,
        OperationError::WrongService => 3,
        OperationError::UnknownService { .. } => 4,
        // Transient variants have their own sink and their own order.
        OperationError::Unauthorized
        | OperationError::Overloaded
        | OperationError::Timeout { .. }
        | OperationError::Internal(_) => u8::MAX,
    }
}

/// Keeps whichever of the held and the candidate error ranks higher.
///
/// Ties keep the error already held, so the outcome does not depend on the order
/// sub-batches happened to complete in.
fn keep_by_precedence(
    slot: &mut Option<OperationError>,
    candidate: OperationError,
    precedence: fn(&OperationError) -> u8,
) {
    let take_candidate = match slot.as_ref() {
        None => true,
        Some(held) => precedence(&candidate) < precedence(held),
    };
    if take_candidate {
        *slot = Some(candidate);
    }
}

/// Whether a permanently failed sub-batch may be re-dispatched one op at a time.
///
/// Only the three admission refusals qualify. Each is decided before the batch
/// applies anything, so re-dispatching an operation of that sub-batch cannot
/// apply a write twice (TG-SYNC-003). `UnknownService` / `WrongService` describe
/// server misrouting rather than a defect in any one operation, so splitting them
/// would attribute a server fault to a client's write and make the client retire
/// it forever.
///
/// `Unauthorized` is transient and therefore never reaches this gate: a singleton
/// must not retire a valid operation that will succeed once the connection
/// re-authenticates.
///
/// Exhaustive with no `_` arm, so a future error variant must be classified
/// explicitly rather than silently inheriting either answer.
fn is_redispatchable(error: &OperationError) -> bool {
    match error {
        OperationError::Forbidden { .. }
        | OperationError::SchemaInvalid { .. }
        | OperationError::ValueTooLarge { .. } => true,
        // One arm, two reasons: the misrouting pair above, and the transient
        // variants, which have their own sink and never reach this gate at all.
        OperationError::UnknownService { .. }
        | OperationError::WrongService
        | OperationError::Unauthorized
        | OperationError::Overloaded
        | OperationError::Timeout { .. }
        | OperationError::Internal(_) => false,
    }
}

/// The acknowledgement entry for one accepted operation.
fn accepted_result(op_id: String) -> topgun_core::messages::OpResult {
    topgun_core::messages::OpResult {
        op_id,
        success: true,
        achieved_level: WriteConcern::APPLIED,
        error: None,
    }
}

/// Dispatches one client batch and returns a per-operation verdict for every
/// operation it could attribute one to.
///
/// This is the single implementation of the verdict fold: both transports call
/// it and then only shape frames from its result, so there is no second place
/// where "which write did the server refuse?" is decided.
///
/// **Why a fold at all.** A sub-batch that fails permanently fails as a whole,
/// so the failure names a partition, not a write. Reporting that to the client
/// as a batch error leaves every operation in the batch un-verdicted, and the
/// client keeps re-sending writes the server will never accept.
///
/// **Contract.**
///
/// - `partition_groups` is taken **by value**, so the association between a
///   sub-batch's operations and its result cannot be lost: an operation that was
///   part of an accepted sub-batch is accepted, and one that was part of a
///   refused sub-batch is a candidate for individual attribution.
/// - Optimistic first: dispatch the sub-batches as-is. A sub-batch that succeeds
///   contributes its operations to `OpOutcome::accepted` — but only if some
///   operation of the batch was refused, because that is the only case in which
///   the acknowledgement names operations individually. See the field's own
///   contract before reading an empty `accepted` as "nothing was accepted".
/// - A sub-batch failing with a **transient** error contributes to
///   `OpOutcome::transient`; the batch stays retryable and nothing is retired.
/// - A sub-batch failing **permanently** is re-dispatched one operation at a
///   time, each as its own singleton batch through the same dispatcher, **in the
///   sub-batch's original order**, and each singleton's own outcome becomes that
///   operation's verdict. Only admission refusals are re-dispatched — the ones
///   proven to be decided before anything is applied — so re-dispatching cannot
///   apply a write twice (TG-SYNC-003).
/// - A permanent error that is **not** re-dispatchable, and a sub-batch holding
///   an operation with no id, both land in `OpOutcome::batch_error` and stay a
///   batch-level error. Attribution that cannot be proven is not guessed.
/// - Every refusal in `OpOutcome::refused` names an operation exactly once, and
///   no operation appears in both `accepted` and `refused` (TG-SYNC-001).
/// - The refusal metric is emitted here, once per refusal, so neither transport
///   can forget to count one.
pub(crate) async fn attribute_permanent_failure_per_op(
    partition_groups: Vec<(u32, Vec<topgun_core::messages::ClientOp>)>,
    cx: &OpBatchDispatchContext<'_>,
) -> crate::service::operation::OpOutcome {
    let mut outcome = OpOutcome::default();

    // Optimistic pass: dispatch every sub-batch concurrently, exactly as an
    // unattributed batch does today.
    let mut join_set = tokio::task::JoinSet::new();
    for (partition_id, group_ops) in partition_groups {
        // The classifier consumes the operations it wraps, so the task keeps its
        // own copy of them. Without one the completion carries only a result, and
        // the sub-batch <-> result association — which decides both "these ops
        // were accepted" and "re-dispatch exactly these ops" — is unrecoverable.
        let sub_op = cx.sub_batch_operation(group_ops.clone(), partition_id);
        let dispatcher = Arc::clone(cx.dispatcher);
        join_set.spawn(async move {
            let result = dispatcher.dispatch(sub_op).await;
            (partition_id, group_ops, result)
        });
    }

    let mut accepted_groups: Vec<(u32, Vec<ClientOp>)> = Vec::new();
    let mut permanent_failures: Vec<(u32, Vec<ClientOp>, OperationError)> = Vec::new();

    while let Some(joined) = join_set.join_next().await {
        match joined {
            Ok((partition_id, ops, Ok(_resp))) => accepted_groups.push((partition_id, ops)),
            Ok((partition_id, ops, Err(e))) => match e.disposition() {
                ErrorDisposition::Transient => {
                    keep_by_precedence(&mut outcome.transient, e, transient_precedence);
                }
                ErrorDisposition::Permanent => permanent_failures.push((partition_id, ops, e)),
            },
            Err(join_err) => {
                // A panicked or cancelled worker task names no operation, and
                // `Internal` is transient, so the batch stays retryable.
                keep_by_precedence(
                    &mut outcome.transient,
                    OperationError::Internal(anyhow::anyhow!("join error: {join_err}")),
                    transient_precedence,
                );
            }
        }
    }

    // Sub-batches complete in whatever order the runtime finishes them, so the
    // fallback is walked in partition order instead: which frames a client
    // receives for one batch must not depend on a task-completion race.
    permanent_failures.sort_by_key(|(partition_id, _, _)| *partition_id);

    let mut singleton_accepted: Vec<String> = Vec::new();
    for (partition_id, ops, error) in permanent_failures {
        if !is_redispatchable(&error) {
            keep_by_precedence(&mut outcome.batch_error, error, batch_error_precedence);
            continue;
        }
        // An operation with no id cannot be named in a rejection, and guessing
        // which write was refused is worse than reporting that the batch failed.
        // Collecting into `Option<Vec<_>>` yields `None` if ANY op lacks an id.
        let Some(op_ids) = ops
            .iter()
            .map(|op| op.id.clone())
            .collect::<Option<Vec<String>>>()
        else {
            keep_by_precedence(&mut outcome.batch_error, error, batch_error_precedence);
            continue;
        };

        // Sequentially, in the sub-batch's original order: apply order is
        // observable through the Event Journal, so it is part of what the caller
        // sees and not an implementation detail.
        for (op, op_id) in ops.into_iter().zip(op_ids) {
            let singleton = cx.sub_batch_operation(vec![op], partition_id);
            match cx.dispatcher.dispatch(singleton).await {
                Ok(_) => singleton_accepted.push(op_id),
                Err(e) => match e.disposition() {
                    ErrorDisposition::Permanent => {
                        metrics::counter!(
                            METRIC_CLIENT_OP_REFUSALS_TOTAL,
                            "transport" => cx.transport.as_label(),
                            "reason" => e.error_kind(),
                        )
                        .increment(1);
                        outcome.refused.push(OpVerdict::Refused {
                            op_id,
                            code: e.wire_code(),
                            reason: format!("{e}"),
                            kind: e.error_kind(),
                        });
                    }
                    ErrorDisposition::Transient => {
                        keep_by_precedence(&mut outcome.transient, e, transient_precedence);
                    }
                },
            }
        }
    }

    // The acknowledgement names individual operations only when some operation
    // was refused: a batch nobody refused anything in is acknowledged by its last
    // id with no `results` at all, so materializing the per-operation vector
    // there would be hot-path cost for something no caller reads.
    if !outcome.refused.is_empty() {
        accepted_groups.sort_by_key(|(partition_id, _)| *partition_id);
        outcome.accepted = accepted_groups
            .into_iter()
            .flat_map(|(_, ops)| ops)
            .filter_map(|op| op.id)
            .chain(singleton_accepted)
            .map(accepted_result)
            .collect();
    }

    outcome
}

/// The `lastId` an acknowledgement that names operations individually may carry.
///
/// The numeric maximum of the accepted ids, returned in its original spelling.
/// Numeric and not lexicographic: operation ids are stringified autoincrement
/// integers, so a lexicographic maximum picks `"9"` over `"10"` and would leave
/// the client believing everything up to `"9"` was acknowledged.
///
/// `None` when no accepted id is a number at all. A non-numeric `lastId` makes
/// the client mark **every** pending operation as synced, which on an exchange
/// that refused something would durably discard a write the server never
/// accepted — so the caller sends no acknowledgement instead, and the accepted
/// operations are re-sent on the next flush.
pub(crate) fn numeric_max_op_id(accepted: &[topgun_core::messages::OpResult]) -> Option<String> {
    accepted
        .iter()
        .filter_map(|result| result.op_id.parse::<u64>().ok().map(|n| (n, &result.op_id)))
        .max_by_key(|(n, _)| *n)
        .map(|(_, op_id)| op_id.clone())
}

/// Serializes one frame and queues it on the connection's outbound channel.
///
/// `to_vec_named` and not `to_vec`: the client decodes `MsgPack` maps by field
/// name, so an array-encoded frame is unreadable to it.
async fn send_frame(msg: &TopGunMessage, tx: &mpsc::Sender<OutboundMessage>) {
    if let Ok(bytes) = rmp_serde::to_vec_named(msg) {
        let _ = tx.send(OutboundMessage::Binary(bytes)).await;
    }
}

/// Queues one `OP_REJECTED` frame per attributed refusal.
///
/// Called before anything else the exchange sends, and unconditionally: an
/// exchange that also hit a transient error still reports the operations it
/// already reached a verdict on. The outbound channel is FIFO, so queueing these
/// first IS the mechanism behind "a refusal always reaches the client ahead of
/// any acknowledgement" (TG-SYNC-002) — there is no second ordering guard on the
/// server side.
///
/// `permanent` is `true` on every frame this sends: the refusal sink holds only
/// verdicts a retry cannot change, and a client that read a terminal refusal as
/// retryable would re-send the write forever.
async fn send_op_rejected_frames(refused: &[OpVerdict], tx: &mpsc::Sender<OutboundMessage>) {
    for verdict in refused {
        let OpVerdict::Refused {
            op_id,
            code,
            reason,
            ..
        } = verdict
        else {
            continue;
        };
        let rejected = TopGunMessage::OpRejected(topgun_core::messages::OpRejectedMessage {
            payload: topgun_core::messages::OpRejectedPayload {
                op_id: op_id.clone(),
                reason: reason.clone(),
                code: Some(*code),
                permanent: true,
            },
        });
        send_frame(&rejected, tx).await;
    }
}

/// Splits an `OpBatch` by partition, dispatches all sub-batches concurrently and
/// answers with a per-operation verdict for everything the batch touched.
///
/// Groups the batch's ops by `hash_to_partition(key)`, hands the groups to the
/// shared verdict fold — which dispatches one `Operation::OpBatch` per group and,
/// when a group fails permanently, re-dispatches its operations one at a time to
/// find out which of them the server actually refused — and then only shapes
/// frames from the fold's outcome.
///
/// **Frames, in this order.** Every refusal becomes one `OP_REJECTED`, and all of
/// them precede any acknowledgement (TG-SYNC-002); the outbound channel is FIFO,
/// so queueing them first is the ordering mechanism. Then exactly one of:
///
/// - a transient (or non-attributed permanent) error → one `ERROR` frame and **no
///   acknowledgement**, even when some operations were accepted;
/// - nothing refused → today's `OP_ACK { lastId, achievedLevel }` with no
///   `results`, byte-identical to the acknowledgement a server without per-operation
///   verdicts sends;
/// - everything refused → **nothing further**: the last `OP_REJECTED` terminates
///   the exchange, because a batch in which every operation was refused has
///   nothing to acknowledge;
/// - otherwise → one `OP_ACK` naming the accepted operations in `results`.
///
/// **Why no partial acknowledgement beside a transient error.** Take sub-batches
/// `{1,3,5}` (all refused) and `{2,4,6}` (transient). An acknowledgement whose
/// `lastId` is the largest accepted id lets the client durably delete every
/// operation at or below it — including 2 and 4, which nobody ever accepted. The
/// operations a client re-sends must be exactly the operations nobody confirmed.
async fn dispatch_op_batch(
    batch_msg: &topgun_core::messages::OpBatchMessage,
    conn_id: ConnectionId,
    principal: Option<Principal>,
    classify_svc: &OperationService,
    dispatcher: &Arc<PartitionDispatcher>,
    tx: &mpsc::Sender<OutboundMessage>,
) {
    let ops = &batch_msg.payload.ops;

    if ops.is_empty() {
        // The only surviving `"unknown"` acknowledgement, and by construction it
        // carries no refused operation: there is no operation to name.
        let ack = TopGunMessage::OpAck(OpAckMessage {
            payload: OpAckPayload {
                last_id: "unknown".to_string(),
                ..Default::default()
            },
        });
        send_frame(&ack, tx).await;
        return;
    }

    // Compute lastId from the last op in the original batch order.
    let last_id = ops
        .last()
        .and_then(|op| op.id.clone())
        .unwrap_or_else(|| "unknown".to_string());

    // Group ops by their partition so each group targets one partition worker.
    let mut partition_groups: HashMap<u32, Vec<topgun_core::messages::ClientOp>> = HashMap::new();
    for op in ops {
        let partition_id = hash_to_partition(&op.key);
        partition_groups
            .entry(partition_id)
            .or_default()
            .push(op.clone());
    }

    let cx = OpBatchDispatchContext {
        classify_svc,
        dispatcher,
        transport: TransportKind::WebSocket,
        caller_origin: CallerOrigin::Client,
        client_id: None,
        principal,
        connection_id: Some(conn_id),
        write_concern: batch_msg.payload.write_concern.clone(),
        timeout: batch_msg.payload.timeout,
    };

    let mut outcome =
        attribute_permanent_failure_per_op(partition_groups.into_iter().collect(), &cx).await;

    send_op_rejected_frames(&outcome.refused, tx).await;

    // (i) A transient error outranks a non-attributed permanent one: retrying is
    //     the only response that can still change the outcome.
    if let Some(err) = outcome
        .transient
        .take()
        .or_else(|| outcome.batch_error.take())
    {
        debug!("dispatch_op_batch error for {:?}: {}", conn_id, err);
        let (code, message) = match err {
            OperationError::Overloaded => (429, "server overloaded, try again later".to_string()),
            ref e => (e.wire_code(), format!("{e}")),
        };
        let err_response = TopGunMessage::Error {
            payload: ErrorPayload {
                code,
                message,
                details: None,
            },
        };
        send_frame(&err_response, tx).await;
        return;
    }

    // (ii) Nothing refused — today's aggregated acknowledgement, unchanged.
    //      `achievedLevel` is set explicitly because each sub-batch's CRDT merge
    //      succeeded in memory, and the per-sub-batch acks were folded away.
    //
    //      This branch MUST stay ahead of the one below: the fold populates
    //      `accepted` only once something was refused, so an empty `accepted`
    //      here means "not computed", not "nothing was accepted".
    if outcome.refused.is_empty() {
        let ack = TopGunMessage::OpAck(OpAckMessage {
            payload: OpAckPayload {
                last_id,
                achieved_level: Some(WriteConcern::APPLIED),
                ..Default::default()
            },
        });
        send_frame(&ack, tx).await;
        return;
    }

    // (iii) Every id-bearing operation was refused: the `OP_REJECTED` frames are
    //       the whole answer. No `OP_ACK` (there is nothing to acknowledge) and
    //       no `ERROR` (the exchange is already explained operation by operation).
    if outcome.accepted.is_empty() {
        return;
    }

    // Same silence for an acceptance set no numeric `lastId` can address: see
    // `numeric_max_op_id`. The accepted operations stay pending and are re-sent,
    // which is safe; falsely acknowledging the refused one would not be.
    let Some(partial_last_id) = numeric_max_op_id(&outcome.accepted) else {
        warn!(
            "op batch for {:?}: accepted operations carry no numeric id; \
             sending no partial ack rather than one the client would over-apply",
            conn_id
        );
        return;
    };

    // (iv) Partial: `results` is the authoritative acceptance set, and it is
    //      never empty on this path.
    let ack = TopGunMessage::OpAck(OpAckMessage {
        payload: OpAckPayload {
            last_id: partial_last_id,
            achieved_level: Some(WriteConcern::APPLIED),
            results: Some(std::mem::take(&mut outcome.accepted)),
        },
    });
    send_frame(&ack, tx).await;
}

/// Unpacks a BATCH message and dispatches each inner message individually.
///
/// The BATCH `data` field contains length-prefixed binary messages: each
/// inner message is preceded by a 4-byte big-endian u32 length header.
async fn unpack_and_dispatch_batch(
    batch_msg: &topgun_core::messages::BatchMessage,
    conn_id: ConnectionId,
    principal: Option<Principal>,
    classify_svc: &OperationService,
    dispatcher: &Arc<PartitionDispatcher>,
    tx: &mpsc::Sender<OutboundMessage>,
) {
    let data = &batch_msg.data;

    // Authoritative dispatch-side bound: drop the WHOLE frame if it packs more
    // inner items than the cap. This holds even if the cost function is later
    // changed or this path is reached another way, and matches the existing
    // whole-frame token-exhaustion behavior — the batch path has no per-item ack,
    // so partial dispatch would be silent unacked loss. The client is told via an
    // explicit error rather than a silent drop.
    let item_count = count_batch_items(data);
    if item_count > MAX_BATCH_INNER_ITEMS {
        debug!(
            "batch from {:?} exceeds max inner items ({} > {}); dropping whole frame",
            conn_id, item_count, MAX_BATCH_INNER_ITEMS
        );
        let err = TopGunMessage::Error {
            payload: ErrorPayload {
                code: 413,
                message: "batch exceeds maximum inner item count".to_string(),
                details: None,
            },
        };
        if let Ok(bytes) = rmp_serde::to_vec_named(&err) {
            let _ = tx.send(OutboundMessage::Binary(bytes)).await;
        }
        return;
    }

    let mut offset = 0;

    while offset < data.len() {
        // Read 4-byte big-endian length prefix. Comparisons use the remaining
        // byte count (`data.len() - offset`) rather than `offset + len` so an
        // adversarial near-`u32::MAX` length cannot overflow `usize` on a 32-bit
        // target; `offset < data.len()` here and `offset <= data.len()` after the
        // prefix read keep the subtraction from underflowing. This mirrors
        // `count_batch_items` exactly so the charged and dispatched counts agree.
        if data.len() - offset < 4 {
            debug!("truncated batch length prefix from {:?}", conn_id);
            break;
        }
        let len = u32::from_be_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]) as usize;
        offset += 4;

        if len > data.len() - offset {
            debug!(
                "truncated batch message (need {} bytes, {} available) from {:?}",
                len,
                data.len() - offset,
                conn_id
            );
            break;
        }

        let msg_bytes = &data[offset..offset + len];
        offset += len;

        // Deserialize the inner message. Inner messages live inside the batch's
        // opaque `bin` body, which the outer frame's depth pre-scan skips without
        // descending — so each inner message gets its own depth-checked decode,
        // consistent with the top-level Phase 1/2 sites. This is the
        // version-independent guard against an unbounded recursive decode of a
        // deeply-nested inner frame; the pinned rmp_serde also caps recursion at
        // 1024, but that is an unstable codec internal we do not rely on.
        let inner_msg = match decode::decode_depth_checked::<TopGunMessage>(msg_bytes) {
            Ok(msg) => msg,
            Err(e) => {
                debug!(
                    "failed to deserialize batch inner message from {:?}: {}",
                    conn_id, e
                );
                continue;
            }
        };

        // Classify and route each inner message individually.
        // Inner messages target different services and partitions, so each
        // must be dispatched separately for correct partition routing.
        match classify_svc.classify(inner_msg, None, CallerOrigin::Client) {
            Ok(mut op) => {
                op.set_connection_id(conn_id);
                if let Some(p) = principal.clone() {
                    op.set_principal(p);
                }

                match dispatcher.dispatch(op).await {
                    Ok(resp) => {
                        send_operation_response(resp, tx).await;
                    }
                    Err(e) => {
                        debug!("dispatch error for batch item from {:?}: {}", conn_id, e);
                    }
                }
            }
            Err(e) => {
                debug!(
                    "failed to classify batch inner message from {:?}: {}",
                    conn_id, e
                );
            }
        }
    }
}

/// Sends an `OperationResponse` as outbound WebSocket message(s).
///
/// Maps each variant to the appropriate wire format:
/// - `Message` -> serialize and send as binary frame
/// - `Messages` -> serialize each individually and send as separate frames
/// - `Empty` -> no response
/// - `Ack` -> construct `OpAck` with `call_id.to_string()` as `last_id`
/// - `NotImplemented` -> construct `Error` with code 501
async fn send_operation_response(resp: OperationResponse, tx: &mpsc::Sender<OutboundMessage>) {
    match resp {
        OperationResponse::Message(msg) => {
            if let Ok(bytes) = rmp_serde::to_vec_named(&*msg) {
                let _ = tx.send(OutboundMessage::Binary(bytes)).await;
            }
        }
        OperationResponse::Messages(msgs) => {
            for msg in msgs {
                if let Ok(bytes) = rmp_serde::to_vec_named(&msg) {
                    let _ = tx.send(OutboundMessage::Binary(bytes)).await;
                }
            }
        }
        OperationResponse::Empty => {
            // No response needed
        }
        OperationResponse::Ack { call_id } => {
            let ack = TopGunMessage::OpAck(OpAckMessage {
                payload: OpAckPayload {
                    last_id: call_id.to_string(),
                    ..Default::default()
                },
            });
            if let Ok(bytes) = rmp_serde::to_vec_named(&ack) {
                let _ = tx.send(OutboundMessage::Binary(bytes)).await;
            }
        }
        OperationResponse::NotImplemented {
            service_name,
            call_id: _,
        } => {
            let err_msg = TopGunMessage::Error {
                payload: ErrorPayload {
                    code: 501,
                    message: format!("not implemented: {service_name}"),
                    details: None,
                },
            };
            if let Ok(bytes) = rmp_serde::to_vec_named(&err_msg) {
                let _ = tx.send(OutboundMessage::Binary(bytes)).await;
            }
        }
    }
}

/// Drains the outbound mpsc channel and writes messages to the WebSocket.
///
/// Implements message coalescing: after receiving the first message, it
/// checks `try_recv()` for additional ready messages and sends them all
/// before waiting again. This reduces the number of individual write
/// syscalls under load.
/// Awaits the outbound task's clean exit, then aborts it if it is wedged.
///
/// The outbound task can stall on a writer whose TCP send-buffer is full (a
/// slow or dead client). We give it a bounded window to drain and close
/// gracefully; if it does not, `abort()` reclaims the task so it cannot linger
/// holding the socket sender and receiver until the OS TCP layer errors.
async fn join_outbound_with_timeout(handle: tokio::task::JoinHandle<()>) {
    let mut handle = handle;
    if tokio::time::timeout(std::time::Duration::from_secs(2), &mut handle)
        .await
        .is_err()
    {
        handle.abort();
    }
}

async fn outbound_task(
    mut sender: SplitSink<WebSocket, Message>,
    mut rx: mpsc::Receiver<OutboundMessage>,
) {
    while let Some(msg) = rx.recv().await {
        if send_outbound_message(&mut sender, msg).await.is_err() {
            break;
        }

        // Coalesce: drain any additional messages that are already buffered
        // in the channel without blocking. This batches multiple messages
        // into a burst of writes before flushing.
        while let Ok(msg) = rx.try_recv() {
            if send_outbound_message(&mut sender, msg).await.is_err() {
                return;
            }
        }

        // Flush after draining all ready messages to push the batch to
        // the network in a single write.
        if sender.flush().await.is_err() {
            break;
        }
    }

    // Gracefully close the WebSocket write half.
    let _ = sender.close().await;
}

/// Sends a single outbound message to the WebSocket sender.
///
/// Returns `Ok(())` on success or `Err(())` when the connection should
/// be torn down (send error or Close message).
async fn send_outbound_message(
    sender: &mut SplitSink<WebSocket, Message>,
    msg: OutboundMessage,
) -> Result<(), ()> {
    match msg {
        OutboundMessage::Binary(data) => {
            if sender.send(Message::Binary(data.into())).await.is_err() {
                return Err(());
            }
        }
        OutboundMessage::Close(reason) => {
            let close_frame = reason.map(|r| CloseFrame {
                code: axum::extract::ws::close_code::NORMAL,
                reason: r.into(),
            });
            let _ = sender.send(Message::Close(close_frame)).await;
            return Err(());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::delta_buffer::DeltaBuffer;
    use crate::query::window::LiveWindow;
    use crate::service::domain::journal::{JournalStore, JournalSubscription};
    use crate::service::domain::query::{QueryRegistry, QuerySubscription};
    use crate::service::domain::search::{
        HybridSearchSubscription, SearchSubscription, SubscriptionRegistry,
    };
    use dashmap::DashSet;
    use std::collections::BTreeSet;
    use topgun_core::messages::base::Query;
    use topgun_core::messages::search::SearchOptions;
    use topgun_core::messages::{BatchMessage, ClientOp, OpBatchMessage, OpBatchPayload};

    /// Frames `body` as one length-prefixed batch inner item (4-byte big-endian
    /// length header + body), matching the wire framing the unpacker expects.
    fn frame_inner_item(body: &[u8]) -> Vec<u8> {
        let len = u32::try_from(body.len()).expect("inner item fits in u32");
        let mut out = len.to_be_bytes().to_vec();
        out.extend_from_slice(body);
        out
    }

    /// Concatenates `n` minimal (empty-body) framed inner items.
    fn pack_n_minimal_items(n: usize) -> Vec<u8> {
        let mut data = Vec::new();
        for _ in 0..n {
            data.extend_from_slice(&frame_inner_item(&[]));
        }
        data
    }

    /// The op-rate limiter charges a frame by the number of ops it carries, so a
    /// peer cannot evade the per-connection cap by packing ops into one batch.
    /// For a transport `Batch` the declared `count` is attacker-controlled and
    /// must NOT be the cost basis — the cost is the actual packed inner-item count.
    #[test]
    fn inbound_op_cost_counts_batch_ops() {
        // A non-batch message costs one token.
        let ping = TopGunMessage::Ping(topgun_core::messages::PingData { timestamp: 0 });
        assert_eq!(inbound_op_cost(&ping), 1);

        // An OpBatch costs its op count.
        let three_ops = TopGunMessage::OpBatch(OpBatchMessage {
            payload: OpBatchPayload {
                ops: vec![
                    ClientOp::default(),
                    ClientOp::default(),
                    ClientOp::default(),
                ],
                write_concern: None,
                timeout: None,
            },
        });
        assert_eq!(inbound_op_cost(&three_ops), 3);

        // An empty OpBatch still costs one token (cannot be used to spin for free).
        let empty = TopGunMessage::OpBatch(OpBatchMessage {
            payload: OpBatchPayload::default(),
        });
        assert_eq!(inbound_op_cost(&empty), 1);

        // A transport Batch declaring count=1 but packing 50 framed inner items
        // is charged ~50, NOT 1 — the declared count is not trusted.
        let amplified = TopGunMessage::Batch(BatchMessage {
            count: 1,
            data: pack_n_minimal_items(50),
        });
        assert_eq!(inbound_op_cost(&amplified), 50);

        // An empty transport Batch (no framed items) still costs one token.
        let empty_batch = TopGunMessage::Batch(BatchMessage {
            count: 0,
            data: vec![],
        });
        assert_eq!(inbound_op_cost(&empty_batch), 1);

        // A transport Batch declaring count=99 but packing only 3 framed items
        // is charged 3 — the cost follows the actual packed count, not the lie.
        let over_declared = TopGunMessage::Batch(BatchMessage {
            count: 99,
            data: pack_n_minimal_items(3),
        });
        assert_eq!(inbound_op_cost(&over_declared), 3);
    }

    /// Disconnect cleanup must drain a connection's standing subscriptions from
    /// every registry wired into `AppState`, not just lock/topic/counter. This
    /// drives `release_session_state` directly and asserts each of the four
    /// later-added registries reports zero subscriptions for the disconnected
    /// connection — the regression that left query/journal/search/hybrid
    /// subscriptions leaked on disconnect.
    #[test]
    fn release_session_state_clears_query_journal_search_hybrid_subscriptions() {
        let conn = ConnectionId(7);

        let query_registry = Arc::new(QueryRegistry::new());
        let journal_store = Arc::new(JournalStore::new(100));
        let search_registry = Arc::new(SubscriptionRegistry::<SearchSubscription>::new());
        let hybrid_registry = Arc::new(SubscriptionRegistry::<HybridSearchSubscription>::new());

        // One subscription per registry, all owned by the same connection.
        let query = Query::default();
        let live_window = Arc::new(LiveWindow::new(
            query.sort.clone().unwrap_or_default(),
            query.limit,
        ));
        query_registry.register(QuerySubscription {
            query_id: "q-1".to_string(),
            connection_id: conn,
            map_name: "users".to_string(),
            query,
            previous_result_keys: DashSet::new(),
            live_window,
            fields: None,
            delta_buffer: Arc::new(DeltaBuffer::new(64)),
        });

        journal_store.subscribe(
            "j-1".to_string(),
            JournalSubscription {
                connection_id: conn,
                map_name: Some("users".to_string()),
                types: None,
            },
        );

        search_registry.register(SearchSubscription::new(
            "s-1".to_string(),
            conn,
            "users".to_string(),
            "hello".to_string(),
            SearchOptions::default(),
        ));

        hybrid_registry.register(HybridSearchSubscription::new(
            "h-1".to_string(),
            conn,
            "users".to_string(),
            "hello".to_string(),
            Vec::new(),
            10,
            None,
            None,
            false,
            None,
        ));

        // Sanity: each registry holds the connection's subscription before disconnect.
        assert_eq!(query_registry.subscription_count(), 1);
        assert_eq!(journal_store.subscription_count_for_connection(conn), 1);
        assert_eq!(search_registry.get_subscriptions_for_map("users").len(), 1);
        assert_eq!(hybrid_registry.get_subscriptions_for_map("users").len(), 1);

        let state = AppState {
            query_registry: Some(Arc::clone(&query_registry)),
            journal_store: Some(Arc::clone(&journal_store)),
            search_registry: Some(Arc::clone(&search_registry)),
            hybrid_search_registry: Some(Arc::clone(&hybrid_registry)),
            ..AppState::for_test()
        };

        release_session_state(&state, conn);

        assert_eq!(
            query_registry.subscription_count(),
            0,
            "query registry retained subscription after disconnect"
        );
        assert_eq!(
            journal_store.subscription_count_for_connection(conn),
            0,
            "journal store retained subscription after disconnect"
        );
        assert_eq!(
            search_registry.get_subscriptions_for_map("users").len(),
            0,
            "search registry retained subscription after disconnect"
        );
        assert_eq!(
            hybrid_registry.get_subscriptions_for_map("users").len(),
            0,
            "hybrid-search registry retained subscription after disconnect"
        );
    }

    // -----------------------------------------------------------------------
    // Verdict fold: per-operation attribution of a permanent sub-batch failure
    // -----------------------------------------------------------------------

    use crate::network::config::ConnectionConfig;
    use crate::network::connection::ConnectionRegistry;
    use crate::service::config::ServerConfig;
    use crate::service::dispatch::DispatchConfig;
    use crate::service::domain::crdt::CrdtService;
    use crate::service::domain::schema::SchemaService;
    use crate::service::middleware::pipeline::build_operation_pipeline;
    use crate::service::operation::service_names;
    use crate::service::policy::{
        InMemoryPolicyStore, PermissionAction, PermissionPolicy, PolicyEffect, PolicyEvaluator,
        PolicyStore,
    };
    use crate::service::router::OperationRouter;
    use crate::service::security::{SecurityConfig, WriteAdmission};
    use crate::storage::datastores::NullDataStore;
    use crate::storage::factory::RecordStoreFactory;
    use crate::storage::impls::StorageConfig;
    use crate::traits::SchemaProvider;
    use metrics_exporter_prometheus::PrometheusBuilder;
    use topgun_core::messages::base::{PredicateNode, PredicateOp};
    use topgun_core::{
        FieldDef, FieldType, LWWRecord, MapSchema, Principal, SystemClock, Timestamp, HLC,
    };

    /// The map every fold test writes to. One map for every op of a sub-batch is
    /// deliberate: it is what makes the fixture model the real refusal. Policy is
    /// evaluated per `(map_name, op_data)`, so two ops on the same map can get
    /// different decisions and the map name does NOT identify the denied write —
    /// a fixture that put the bad op on its own map would let a wrong
    /// implementation (split the sub-batch by map name) pass.
    const FOLD_MAP: &str = "notes";

    /// A map with a registered schema, used by the `SchemaInvalid` case.
    const TYPED_MAP: &str = "typed-notes";

    /// A whole pipeline — router, middleware, dispatcher, CRDT service — wired the
    /// way the server wires it, so the fold is exercised against the real
    /// Authorization middleware rather than a stub that returns `Forbidden`.
    struct FoldFixture {
        classify_svc: OperationService,
        dispatcher: Arc<PartitionDispatcher>,
        factory: Arc<RecordStoreFactory>,
        journal: Arc<JournalStore>,
        conn_id: ConnectionId,
        /// Held so the connection stays registered: the CRDT service snapshots
        /// its metadata on every batch that carries a `connection_id`.
        _connection: Arc<ConnectionHandle>,
        _outbound: mpsc::Receiver<OutboundMessage>,
        /// How many operations reached the domain service. One per sub-batch on
        /// the optimistic pass, plus one per singleton the fallback re-dispatched
        /// — which is what makes "the fallback was never entered" checkable.
        service_calls: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl FoldFixture {
        /// The dispatch context the transports will build, with a write concern
        /// and a timeout set so the singleton pass is exercised carrying them.
        fn cx(&self) -> OpBatchDispatchContext<'_> {
            OpBatchDispatchContext {
                classify_svc: &self.classify_svc,
                dispatcher: &self.dispatcher,
                transport: TransportKind::WebSocket,
                caller_origin: CallerOrigin::Client,
                client_id: None,
                principal: Some(fold_principal()),
                connection_id: Some(self.conn_id),
                write_concern: Some(WriteConcern::APPLIED),
                timeout: Some(5_000),
            }
        }

        /// Whether the durable store holds a live value for `key`.
        async fn holds(&self, map_name: &str, key: &str) -> bool {
            self.factory
                .get_or_create(map_name, hash_to_partition(key))
                .get(key, false)
                .await
                .expect("store read")
                .is_some()
        }

        /// Keys of every journalled event, in apply order.
        fn journalled_keys(&self) -> Vec<String> {
            self.journal
                .read(0, 1000, None)
                .0
                .into_iter()
                .map(|event| event.key)
                .collect()
        }
    }

    /// The authenticated writer every fold test dispatches as.
    fn fold_principal() -> Principal {
        Principal {
            id: "writer-1".to_string(),
            roles: vec!["user".to_string()],
        }
    }

    /// A CRDT service that answers `Overloaded` for any batch touching
    /// `shed_key`, and delegates everything else to the real one.
    ///
    /// The production `LoadShedLayer` sheds by semaphore occupancy, which cannot
    /// be aimed at one operation of a sequentially re-dispatched fallback. What
    /// the transport branches on is the error's disposition, not where it was
    /// raised, so the transient is staged at the service instead — the same
    /// `OperationError::Overloaded` a singleton re-entering load shedding
    /// produces.
    #[derive(Clone)]
    struct SheddingCrdt {
        inner: Arc<CrdtService>,
        shed_key: Option<String>,
        calls: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl tower::Service<Operation> for SheddingCrdt {
        type Response = OperationResponse;
        type Error = OperationError;
        type Future = std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<OperationResponse, OperationError>> + Send>,
        >;

        fn poll_ready(
            &mut self,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Ready(Ok(()))
        }

        fn call(&mut self, op: Operation) -> Self::Future {
            self.calls.fetch_add(1, Ordering::Relaxed);
            let sheds = self.shed_key.as_ref().is_some_and(|shed_key| match &op {
                Operation::OpBatch { payload, .. } => payload
                    .payload
                    .ops
                    .iter()
                    .any(|client_op| &client_op.key == shed_key),
                _ => false,
            });
            if sheds {
                return Box::pin(async { Err(OperationError::Overloaded) });
            }
            let mut inner = Arc::clone(&self.inner);
            Box::pin(async move { tower::Service::call(&mut inner, op).await })
        }
    }

    /// Builds the fixture with the given policies and optional map schema.
    ///
    /// An empty policy list leaves the store unconfigured, which is the
    /// evaluator's documented allow-all gate — that is how the schema case gets a
    /// permanent failure raised inside the CRDT service rather than by RBAC.
    async fn build_fold_fixture(
        policies: Vec<PermissionPolicy>,
        schema: Option<(&str, MapSchema)>,
    ) -> FoldFixture {
        build_fold_fixture_shedding(policies, schema, None).await
    }

    /// The same fixture, with the domain service made to shed every batch that
    /// touches `shed_key` so a transient failure can be aimed at one operation.
    async fn build_fold_fixture_shedding(
        policies: Vec<PermissionPolicy>,
        schema: Option<(&str, MapSchema)>,
        shed_key: Option<String>,
    ) -> FoldFixture {
        let server_config = Arc::new(ServerConfig::default());
        let factory = Arc::new(RecordStoreFactory::new(
            StorageConfig::default(),
            Arc::new(NullDataStore),
            Vec::new(),
        ));
        let connection_registry = Arc::new(ConnectionRegistry::new());
        let journal = Arc::new(JournalStore::new(1_000));
        let hlc = Arc::new(parking_lot::Mutex::new(HLC::new(
            "fold-test-node".to_string(),
            Box::new(SystemClock),
        )));
        let schema_svc = Arc::new(SchemaService::new());
        if let Some((map_name, map_schema)) = schema {
            schema_svc
                .register_schema(map_name, map_schema)
                .await
                .expect("schema registers");
        }
        let crdt = Arc::new(
            CrdtService::new(
                Arc::clone(&factory),
                Arc::clone(&connection_registry),
                Arc::new(WriteAdmission::new(
                    Arc::new(SecurityConfig::default()),
                    Arc::clone(&hlc),
                )),
                Arc::new(QueryRegistry::new()),
                schema_svc,
            )
            .with_journal(Arc::clone(&journal)),
        );

        let policy_store = Arc::new(InMemoryPolicyStore::new());
        for policy in policies {
            policy_store
                .upsert_policy(policy)
                .await
                .expect("policy upsert");
        }
        let evaluator = Arc::new(PolicyEvaluator::new(policy_store));

        // One worker: every sub-batch and every singleton is served by the same
        // pipeline instance, so nothing in these tests depends on which worker a
        // partition happened to land on.
        let dispatch_config = DispatchConfig {
            worker_count: 1,
            channel_buffer_size: 64,
        };
        let service_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let dispatcher = Arc::new(PartitionDispatcher::new(&dispatch_config, || {
            let mut router = OperationRouter::new();
            router.register(
                service_names::CRDT,
                SheddingCrdt {
                    inner: Arc::clone(&crdt),
                    shed_key: shed_key.clone(),
                    calls: Arc::clone(&service_calls),
                },
            );
            build_operation_pipeline(router, &server_config, Some(Arc::clone(&evaluator)))
        }));

        let (connection, outbound) =
            connection_registry.register(ConnectionKind::Client, &ConnectionConfig::default());
        let conn_id = connection.id;

        FoldFixture {
            classify_svc: OperationService::new(hlc, server_config),
            dispatcher,
            factory,
            journal,
            conn_id,
            _connection: connection,
            _outbound: outbound,
            service_calls,
        }
    }

    /// Allow every write to `FOLD_MAP`, except one whose record carries
    /// `blocked: true`.
    ///
    /// Deny-wins, and the deny is selected by the record's own content, so the
    /// denied op is identified by its data and not by its map.
    fn deny_blocked_records() -> Vec<PermissionPolicy> {
        vec![
            PermissionPolicy {
                id: "allow-notes".to_string(),
                map_pattern: FOLD_MAP.to_string(),
                action: PermissionAction::Write,
                effect: PolicyEffect::Allow,
                condition: None,
            },
            PermissionPolicy {
                id: "deny-blocked".to_string(),
                map_pattern: FOLD_MAP.to_string(),
                action: PermissionAction::Write,
                effect: PolicyEffect::Deny,
                condition: Some(PredicateNode {
                    op: PredicateOp::Eq,
                    attribute: Some("blocked".to_string()),
                    value: Some(rmpv::Value::Boolean(true)),
                    children: None,
                    value_ref: None,
                }),
            },
        ]
    }

    /// A schema requiring a `name` string, so a record without one is rejected by
    /// the CRDT service itself rather than by the middleware.
    fn required_name_schema() -> MapSchema {
        MapSchema {
            version: 1,
            fields: vec![FieldDef {
                name: "name".to_string(),
                required: true,
                field_type: FieldType::String,
                constraints: None,
            }],
            strict: false,
        }
    }

    /// A client PUT carrying `fields` as its record value.
    fn put_op(
        op_id: &str,
        map_name: &str,
        key: &str,
        fields: Vec<(&str, rmpv::Value)>,
    ) -> ClientOp {
        ClientOp {
            id: Some(op_id.to_string()),
            map_name: map_name.to_string(),
            key: key.to_string(),
            op_type: None,
            record: Some(Some(LWWRecord {
                value: Some(rmpv::Value::Map(
                    fields
                        .into_iter()
                        .map(|(name, value)| (rmpv::Value::String(name.into()), value))
                        .collect(),
                )),
                timestamp: Timestamp {
                    millis: 1_700_000_000_000,
                    counter: 1,
                    node_id: "fold-test-node".to_string(),
                },
                ttl_ms: None,
            })),
            or_record: None,
            or_tag: None,
            write_concern: None,
            timeout: None,
        }
    }

    /// A record the deny policy matches.
    fn blocked_fields() -> Vec<(&'static str, rmpv::Value)> {
        vec![("blocked", rmpv::Value::Boolean(true))]
    }

    /// A record the deny policy does not match.
    fn allowed_fields(name: &str) -> Vec<(&'static str, rmpv::Value)> {
        vec![("name", rmpv::Value::String(name.into()))]
    }

    /// Op ids named by `OpOutcome::accepted`, in the order the fold produced them.
    fn accepted_ids(outcome: &OpOutcome) -> Vec<String> {
        outcome
            .accepted
            .iter()
            .map(|result| result.op_id.clone())
            .collect()
    }

    /// Destructures the single expected refusal.
    fn sole_refusal(outcome: &OpOutcome) -> (String, u32, &'static str) {
        assert_eq!(
            outcome.refused.len(),
            1,
            "expected exactly one refusal, got {:?}",
            outcome.refused
        );
        match &outcome.refused[0] {
            OpVerdict::Refused {
                op_id, code, kind, ..
            } => (op_id.clone(), *code, *kind),
            other @ OpVerdict::Accepted { .. } => {
                panic!("expected a refusal verdict, got {other:?}")
            }
        }
    }

    /// A permanently refused sub-batch applies nothing before the fallback, and
    /// the fallback names the refused op without re-applying the accepted ones
    /// (TG-SYNC-003).
    ///
    /// The journal count is the mechanical guard, not a code-reading: every
    /// applied mutation appends exactly one event, so a singleton re-applying
    /// what the batch pass had already applied would show up as two.
    #[tokio::test]
    async fn permanent_subbatch_failure_applied_nothing_before_fallback() {
        let fx = build_fold_fixture(deny_blocked_records(), None).await;
        let ops = vec![
            put_op("101", FOLD_MAP, "k-first", allowed_fields("first")),
            put_op("102", FOLD_MAP, "k-denied", blocked_fields()),
            put_op("103", FOLD_MAP, "k-third", allowed_fields("third")),
        ];
        let partition_id = 7;

        // 1. Zero-applied canary. This is the fold's own batch pass, issued here
        //    so the state BETWEEN the two passes is observable at all — the fold
        //    performs it internally and never exposes the intermediate store.
        let batch_pass = fx.cx().sub_batch_operation(ops.clone(), partition_id);
        let refusal = fx
            .dispatcher
            .dispatch(batch_pass)
            .await
            .expect_err("the whole sub-batch is refused");
        assert!(
            matches!(refusal, OperationError::Forbidden { .. }),
            "expected a middleware refusal, got {refusal:?}"
        );
        for key in ["k-first", "k-denied", "k-third"] {
            assert!(
                !fx.holds(FOLD_MAP, key).await,
                "{key} was applied by a sub-batch that failed permanently"
            );
        }
        assert!(
            fx.journalled_keys().is_empty(),
            "a permanently refused sub-batch journalled something"
        );

        // 2. The fold: two accepted, one refused, nothing else.
        let outcome = attribute_permanent_failure_per_op(vec![(partition_id, ops)], &fx.cx()).await;
        assert_eq!(
            sole_refusal(&outcome),
            ("102".to_string(), 403, "forbidden")
        );
        assert_eq!(accepted_ids(&outcome), vec!["101", "103"]);
        assert!(
            outcome.transient.is_none(),
            "an attributed refusal must not also produce a transient error"
        );
        assert!(
            outcome.batch_error.is_none(),
            "an attributed refusal must not also produce a batch error"
        );

        // 3. Both good ops read back; the refused one did not land.
        assert!(fx.holds(FOLD_MAP, "k-first").await);
        assert!(fx.holds(FOLD_MAP, "k-third").await);
        assert!(!fx.holds(FOLD_MAP, "k-denied").await);

        // 4. Exactly one journal event per accepted op — the double-apply guard.
        assert_eq!(fx.journalled_keys(), vec!["k-first", "k-third"]);
    }

    /// The same fallback attributes a permanent failure raised INSIDE the CRDT
    /// service, not only one raised by the middleware, and carries its own code.
    #[tokio::test]
    async fn permanent_subbatch_failure_applied_nothing_before_fallback_schema_case() {
        let fx = build_fold_fixture(Vec::new(), Some((TYPED_MAP, required_name_schema()))).await;
        let ops = vec![
            put_op("201", TYPED_MAP, "s-first", allowed_fields("first")),
            put_op(
                "202",
                TYPED_MAP,
                "s-invalid",
                vec![("unrelated", rmpv::Value::Integer(1.into()))],
            ),
            put_op("203", TYPED_MAP, "s-third", allowed_fields("third")),
        ];
        let partition_id = 3;

        let batch_pass = fx.cx().sub_batch_operation(ops.clone(), partition_id);
        let refusal = fx
            .dispatcher
            .dispatch(batch_pass)
            .await
            .expect_err("the whole sub-batch is refused");
        assert!(
            matches!(refusal, OperationError::SchemaInvalid { .. }),
            "expected an in-service schema refusal, got {refusal:?}"
        );
        assert!(
            fx.journalled_keys().is_empty(),
            "the validate-all loop must run before the apply loop"
        );

        let outcome = attribute_permanent_failure_per_op(vec![(partition_id, ops)], &fx.cx()).await;
        assert_eq!(
            sole_refusal(&outcome),
            ("202".to_string(), 422, "schema_invalid")
        );
        assert_eq!(accepted_ids(&outcome), vec!["201", "203"]);
        assert_eq!(fx.journalled_keys(), vec!["s-first", "s-third"]);
    }

    /// The singleton pass re-dispatches in the sub-batch's original order.
    ///
    /// Order is observable: each applied op appends one journal event, so the
    /// journal sequence is the dispatch sequence. A fold that re-dispatched in
    /// completion order, or in reverse, would show the two accepted ops swapped.
    #[tokio::test]
    async fn fallback_redispatches_in_original_order() {
        let fx = build_fold_fixture(deny_blocked_records(), None).await;
        // Deliberately ordered so a lexicographic or reversed walk is visible:
        // `z-alpha` is dispatched FIRST and `a-omega` last.
        let ops = vec![
            put_op("301", FOLD_MAP, "z-alpha", allowed_fields("alpha")),
            put_op("302", FOLD_MAP, "m-denied", blocked_fields()),
            put_op("303", FOLD_MAP, "a-omega", allowed_fields("omega")),
        ];

        let outcome = attribute_permanent_failure_per_op(vec![(11, ops)], &fx.cx()).await;

        assert_eq!(sole_refusal(&outcome).0, "302");
        assert_eq!(
            fx.journalled_keys(),
            vec!["z-alpha", "a-omega"],
            "singletons must be re-dispatched in the sub-batch's original order"
        );
        assert_eq!(accepted_ids(&outcome), vec!["301", "303"]);
    }

    /// Reads one labelled counter out of a Prometheus render.
    ///
    /// Matching on the metric name plus every required `label="value"` pair
    /// rather than on a whole formatted line keeps the assertion independent of
    /// the exporter's label ordering.
    fn rendered_labelled_counter(rendered: &str, name: &str, labels: &[(&str, &str)]) -> u64 {
        let matches: Vec<&str> = rendered
            .lines()
            .filter(|line| line.starts_with(&format!("{name}{{")))
            .filter(|line| {
                labels
                    .iter()
                    .all(|(key, value)| line.contains(&format!("{key}=\"{value}\"")))
            })
            .collect();
        assert_eq!(
            matches.len(),
            1,
            "expected exactly one {name} series for {labels:?}, render was:\n{rendered}"
        );
        matches[0]
            .rsplit(' ')
            .next()
            .expect("a value follows the series")
            .parse()
            .expect("counter renders an integer")
    }

    /// Every `transport × reason` refusal series renders at zero before anything
    /// is refused.
    ///
    /// Without the eager touch the series would simply be absent from a scrape,
    /// and an operator cannot tell an absent series from a healthy zero — which
    /// is how a refusal instrument that never fires passes for a server that
    /// never refuses.
    #[test]
    fn refusal_series_registered_at_zero_for_every_transport_and_reason() {
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        // The process-wide `Once` fires for whichever recorder is bound first, so
        // the touch itself is driven here rather than through the guard.
        metrics::with_local_recorder(&recorder, touch_client_op_refusal_series);
        let rendered = handle.render();

        for transport in ["ws", "http"] {
            for reason in ALL_ERROR_KINDS {
                assert_eq!(
                    rendered_labelled_counter(
                        &rendered,
                        METRIC_CLIENT_OP_REFUSALS_TOTAL,
                        &[("transport", transport), ("reason", reason)],
                    ),
                    0,
                    "series for {transport}/{reason} must render at zero"
                );
            }
        }
    }

    /// The refusals series counts refusals; the pipeline counter counts pipeline
    /// calls, and its `1 + k` arithmetic is pinned so nobody "fixes" it.
    ///
    /// For one refused op in a three-op sub-batch (k = 1) the refusals series
    /// rises by exactly 1, while `topgun_operation_errors_total` rises by 2: the
    /// failed batch call plus the failed singleton. That is correct for a counter
    /// of pipeline-call errors — `MetricsLayer` sits above `AuthorizationLayer`,
    /// so both calls are observed — and "correcting" it to 1 fails this test.
    #[test]
    fn refused_op_metric_arithmetic() {
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();

        // A current-thread runtime driven from inside the binding: the recorder is
        // a THREAD-local, and the dispatcher's worker — where `MetricsLayer` emits
        // — must be polled on the same thread for the emission to be seen.
        let rendered = metrics::with_local_recorder(&recorder, || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("current-thread runtime");
            rt.block_on(async {
                let fx = build_fold_fixture(deny_blocked_records(), None).await;

                // Pre-write snapshot: the eager registration is what makes this a
                // readable zero rather than an absent series.
                touch_client_op_refusal_series();
                assert_eq!(
                    rendered_labelled_counter(
                        &handle.render(),
                        METRIC_CLIENT_OP_REFUSALS_TOTAL,
                        &[("transport", "ws"), ("reason", "forbidden")],
                    ),
                    0,
                    "the pre-write snapshot must read zero"
                );

                let ops = vec![
                    put_op("401", FOLD_MAP, "m-first", allowed_fields("first")),
                    put_op("402", FOLD_MAP, "m-denied", blocked_fields()),
                    put_op("403", FOLD_MAP, "m-third", allowed_fields("third")),
                ];
                let outcome = attribute_permanent_failure_per_op(vec![(5, ops)], &fx.cx()).await;
                assert_eq!(
                    sole_refusal(&outcome),
                    ("402".to_string(), 403, "forbidden")
                );
            });
            handle.render()
        });

        assert_eq!(
            rendered_labelled_counter(
                &rendered,
                METRIC_CLIENT_OP_REFUSALS_TOTAL,
                &[("transport", "ws"), ("reason", "forbidden")],
            ),
            1,
            "one refused op must count exactly once on the refusals series"
        );
        assert_eq!(
            rendered_labelled_counter(
                &rendered,
                "topgun_operation_errors_total",
                &[("service", service_names::CRDT), ("error", "forbidden")],
            ),
            2,
            "1 + k pipeline-call errors: the batch call plus the refused singleton"
        );
        assert_eq!(
            rendered_labelled_counter(
                &rendered,
                "topgun_operations_total",
                &[("service", service_names::CRDT), ("outcome", "error")],
            ),
            2,
            "the batch call still returns Err, so the outcome label stays 'error'"
        );
    }

    // -----------------------------------------------------------------------
    // Frames: what one exchange puts on the wire
    // -----------------------------------------------------------------------

    /// `n` distinct keys that all hash to the SAME partition.
    ///
    /// `dispatch_op_batch` groups by `hash_to_partition(key)`, so arbitrary keys
    /// would each form their own single-operation sub-batch and the multi-operation
    /// sub-batch these tests exist to exercise would never be built.
    fn keys_in_one_partition(n: usize) -> Vec<String> {
        let mut buckets: HashMap<u32, Vec<String>> = HashMap::new();
        for i in 0..100_000_u32 {
            let key = format!("fold-key-{i}");
            let bucket = buckets.entry(hash_to_partition(&key)).or_default();
            bucket.push(key);
            if bucket.len() == n {
                return bucket.clone();
            }
        }
        panic!("no partition collected {n} keys");
    }

    /// Runs one `OP_BATCH` through the real transport entry point and returns the
    /// frames it queued, in wire order, as raw bytes.
    ///
    /// Bytes rather than decoded messages because one caller asserts the happy
    /// path is byte-identical to what a server without per-operation verdicts
    /// sent, and a decoded comparison cannot see an added-then-defaulted field.
    async fn dispatch_batch_frames(fx: &FoldFixture, ops: Vec<ClientOp>) -> Vec<Vec<u8>> {
        let (tx, mut rx) = mpsc::channel(64);
        let batch = OpBatchMessage {
            payload: OpBatchPayload {
                ops,
                write_concern: Some(WriteConcern::APPLIED),
                timeout: Some(5_000),
            },
        };
        dispatch_op_batch(
            &batch,
            fx.conn_id,
            Some(fold_principal()),
            &fx.classify_svc,
            &fx.dispatcher,
            &tx,
        )
        .await;
        // The sender the handler holds is a borrow of this one; dropping ours
        // closes the channel so the drain terminates.
        drop(tx);

        let mut frames = Vec::new();
        while let Some(outbound) = rx.recv().await {
            match outbound {
                OutboundMessage::Binary(bytes) => frames.push(bytes),
                OutboundMessage::Close(reason) => {
                    panic!("an op batch must not close the connection: {reason:?}")
                }
            }
        }
        frames
    }

    /// Decodes queued frames back into messages, preserving wire order.
    fn decode_frames(frames: &[Vec<u8>]) -> Vec<TopGunMessage> {
        frames
            .iter()
            .map(|bytes| rmp_serde::from_slice(bytes).expect("a queued frame decodes"))
            .collect()
    }

    /// `(op_id, code, permanent)` of every `OP_REJECTED`, in frame order.
    fn rejections(msgs: &[TopGunMessage]) -> Vec<(String, Option<u32>, bool)> {
        msgs.iter()
            .filter_map(|msg| match msg {
                TopGunMessage::OpRejected(rejected) => Some((
                    rejected.payload.op_id.clone(),
                    rejected.payload.code,
                    rejected.payload.permanent,
                )),
                _ => None,
            })
            .collect()
    }

    /// Every `OP_ACK` payload, in frame order.
    fn ack_payloads(msgs: &[TopGunMessage]) -> Vec<&OpAckPayload> {
        msgs.iter()
            .filter_map(|msg| match msg {
                TopGunMessage::OpAck(ack) => Some(&ack.payload),
                _ => None,
            })
            .collect()
    }

    /// Every `ERROR` payload, in frame order.
    fn error_payloads(msgs: &[TopGunMessage]) -> Vec<&ErrorPayload> {
        msgs.iter()
            .filter_map(|msg| match msg {
                TopGunMessage::Error { payload } => Some(payload),
                _ => None,
            })
            .collect()
    }

    /// The operation ids an acknowledgement covers, computed the way the client
    /// computes them: `results` when present, otherwise every batch id at or
    /// below the numeric `lastId`.
    fn ack_coverage(ack: &OpAckPayload, batch_ids: &[&str]) -> BTreeSet<String> {
        if let Some(results) = ack.results.as_ref() {
            return results
                .iter()
                .map(|result| result.op_id.clone())
                .collect::<BTreeSet<String>>();
        }
        let last: u64 = ack
            .last_id
            .parse()
            .expect("a prefix acknowledgement must carry a numeric lastId");
        batch_ids
            .iter()
            .filter(|id| id.parse::<u64>().is_ok_and(|n| n <= last))
            .map(|id| (*id).to_string())
            .collect()
    }

    /// Collects `&str` ids into an owned set, for comparing verdict coverage.
    fn id_set(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|id| (*id).to_string()).collect()
    }

    /// Every operation of a mixed batch reaches exactly one terminal verdict, and
    /// the two channels partition the batch (TG-SYNC-001 clauses (a) and (b)).
    ///
    /// Coverage is computed the client's way rather than read off the frames'
    /// shape, so an acknowledgement that covered the refused operation by prefix
    /// would fail here even though its `results` did not name it.
    #[tokio::test]
    async fn op_batch_every_op_gets_exactly_one_terminal_verdict() {
        let fx = build_fold_fixture(deny_blocked_records(), None).await;
        let keys = keys_in_one_partition(3);
        let ops = vec![
            put_op("501", FOLD_MAP, &keys[0], allowed_fields("first")),
            put_op("502", FOLD_MAP, &keys[1], blocked_fields()),
            put_op("503", FOLD_MAP, &keys[2], allowed_fields("third")),
        ];
        let batch_ids = ["501", "502", "503"];

        let msgs = decode_frames(&dispatch_batch_frames(&fx, ops).await);

        let rejected = rejections(&msgs);
        assert_eq!(rejected, vec![("502".to_string(), Some(403), true)]);
        let refused: BTreeSet<String> = rejected.into_iter().map(|(op_id, _, _)| op_id).collect();

        let acks = ack_payloads(&msgs);
        assert_eq!(acks.len(), 1, "one exchange acknowledges at most once");
        let covered = ack_coverage(acks[0], &batch_ids);

        assert!(
            covered.is_disjoint(&refused),
            "an operation must not be both acknowledged and refused: {covered:?} vs {refused:?}"
        );
        assert_eq!(
            covered.union(&refused).cloned().collect::<BTreeSet<_>>(),
            id_set(&batch_ids),
            "every id-bearing operation of the batch must carry a verdict"
        );
        assert!(
            error_payloads(&msgs).is_empty(),
            "an attributed refusal is not also a batch error"
        );
        assert!(
            acks[0].last_id.parse::<u64>().is_ok(),
            "a partial ack's lastId must be numeric, got {:?}",
            acks[0].last_id
        );
        assert_eq!(
            acks[0].last_id, "503",
            "lastId is the numeric max accepted id"
        );
    }

    /// A batch in which every operation is refused is answered by the
    /// `OP_REJECTED` frames and nothing else (TG-SYNC-001 clause (b')).
    ///
    /// The last rejection terminates the exchange: there is nothing to
    /// acknowledge, and an `ERROR` frame would report a batch-level failure for
    /// an exchange already explained operation by operation.
    #[tokio::test]
    async fn op_batch_all_refused_emits_rejections_and_no_ack() {
        let fx = build_fold_fixture(deny_blocked_records(), None).await;
        let keys = keys_in_one_partition(2);
        let ops = vec![
            put_op("601", FOLD_MAP, &keys[0], blocked_fields()),
            put_op("602", FOLD_MAP, &keys[1], blocked_fields()),
        ];

        let msgs = decode_frames(&dispatch_batch_frames(&fx, ops).await);

        assert!(
            ack_payloads(&msgs).is_empty(),
            "an all-refused batch has nothing to acknowledge"
        );
        assert!(
            error_payloads(&msgs).is_empty(),
            "the refusals already explain the exchange"
        );
        let rejected = rejections(&msgs);
        assert_eq!(rejected.len(), 2, "one rejection per operation");
        let named: BTreeSet<String> = rejected.iter().map(|(op_id, _, _)| op_id.clone()).collect();
        assert_eq!(
            named,
            id_set(&["601", "602"]),
            "each operation is named in exactly one rejection"
        );
        for (op_id, code, permanent) in rejected {
            assert_eq!(code, Some(403), "{op_id} was refused by policy");
            assert!(permanent, "{op_id} must not be presented as retryable");
        }
    }

    /// A transient failure during the fallback leaves the operations after it
    /// with no verdict at all, and acknowledges nothing (TG-SYNC-001 clause (c)).
    ///
    /// Two predicates carry the clause. The rejections already determined are
    /// delivered, and NO acknowledgement is sent — and since `handleOpAck` is the
    /// only client-side site that marks an operation synced, zero `OP_ACK` on the
    /// wire is precisely "no operation of this exchange was marked synced". The
    /// accepted-but-unacknowledged operations stay pending and are re-sent, which
    /// is safe: a permanently failed sub-batch applied nothing (TG-SYNC-003) and
    /// re-applying an accepted LWW write is idempotent.
    #[tokio::test]
    async fn op_batch_transient_mid_fallback_emits_no_ack() {
        let keys = keys_in_one_partition(3);
        let fx =
            build_fold_fixture_shedding(deny_blocked_records(), None, Some(keys[2].clone())).await;
        let ops = vec![
            put_op("701", FOLD_MAP, &keys[0], blocked_fields()),
            put_op("702", FOLD_MAP, &keys[1], blocked_fields()),
            put_op("703", FOLD_MAP, &keys[2], allowed_fields("shed")),
        ];

        let msgs = decode_frames(&dispatch_batch_frames(&fx, ops).await);

        let named: BTreeSet<String> = rejections(&msgs)
            .into_iter()
            .map(|(op_id, _, _)| op_id)
            .collect();
        assert_eq!(
            named,
            id_set(&["701", "702"]),
            "the rejections settled before the transient failure are still delivered"
        );
        assert!(
            ack_payloads(&msgs).is_empty(),
            "no operation of this exchange may be acknowledged"
        );
    }

    /// Every rejection reaches the client ahead of the acknowledgement, and the
    /// acknowledgement never covers a refused operation (TG-SYNC-002).
    ///
    /// The ordering rests on FIFO delivery of the connection's outbound channel,
    /// so it is asserted on frame positions in that channel.
    #[tokio::test]
    async fn op_rejected_precedes_ack_and_ack_excludes_refused_op() {
        let fx = build_fold_fixture(deny_blocked_records(), None).await;
        let keys = keys_in_one_partition(3);
        let ops = vec![
            put_op("801", FOLD_MAP, &keys[0], allowed_fields("first")),
            put_op("802", FOLD_MAP, &keys[1], blocked_fields()),
            put_op("803", FOLD_MAP, &keys[2], allowed_fields("third")),
        ];

        let msgs = decode_frames(&dispatch_batch_frames(&fx, ops).await);

        let rejected_at: Vec<usize> = msgs
            .iter()
            .enumerate()
            .filter(|(_, msg)| matches!(msg, TopGunMessage::OpRejected(_)))
            .map(|(index, _)| index)
            .collect();
        let acked_at: Vec<usize> = msgs
            .iter()
            .enumerate()
            .filter(|(_, msg)| matches!(msg, TopGunMessage::OpAck(_)))
            .map(|(index, _)| index)
            .collect();
        assert_eq!(rejected_at.len(), 1);
        assert_eq!(acked_at.len(), 1);
        assert!(
            rejected_at[0] < acked_at[0],
            "OP_REJECTED must be queued before OP_ACK, got {rejected_at:?} then {acked_at:?}"
        );

        let ack = ack_payloads(&msgs)[0];
        assert!(
            !ack_coverage(ack, &["801", "802", "803"]).contains("802"),
            "the acknowledgement must not cover the refused operation"
        );
    }

    /// A batch nobody refused anything in is acknowledged with the exact bytes a
    /// server without per-operation verdicts sent (AC6, constraint C7).
    ///
    /// Byte equality against a hand-built acknowledgement, not a field-by-field
    /// comparison: an added-but-defaulted field is invisible to the latter and is
    /// exactly what would change the hot path's wire output.
    #[tokio::test]
    async fn uniformly_accepted_batch_ack_is_byte_identical() {
        let fx = build_fold_fixture(deny_blocked_records(), None).await;
        let keys = keys_in_one_partition(3);
        let ops = vec![
            put_op("901", FOLD_MAP, &keys[0], allowed_fields("first")),
            put_op("902", FOLD_MAP, &keys[1], allowed_fields("second")),
            put_op("903", FOLD_MAP, &keys[2], allowed_fields("third")),
        ];

        let frames = dispatch_batch_frames(&fx, ops).await;

        assert_eq!(frames.len(), 1, "the happy path emits exactly one frame");
        let expected = TopGunMessage::OpAck(OpAckMessage {
            payload: OpAckPayload {
                last_id: "903".to_string(),
                achieved_level: Some(WriteConcern::APPLIED),
                ..Default::default()
            },
        });
        assert_eq!(
            frames[0],
            rmp_serde::to_vec_named(&expected).expect("expected ack serializes"),
            "the uniformly accepted path must not change a single byte"
        );

        let msgs = decode_frames(&frames);
        assert!(rejections(&msgs).is_empty());
        assert!(ack_payloads(&msgs)[0].results.is_none(), "no results field");
    }

    /// A transient sub-batch failure is reported as the retryable `ERROR` frame,
    /// is never presented as a refusal, and does not enter the fallback at all.
    ///
    /// "No fallback" is asserted mechanically: the service is called exactly once
    /// — the batch pass — where a fallback would have called it once per
    /// operation. `Overloaded` is transient, so retrying is the correct client
    /// response and retiring any of these operations would be wrong.
    #[tokio::test]
    async fn transient_subbatch_yields_429_error_and_no_fallback() {
        let keys = keys_in_one_partition(3);
        let fx = build_fold_fixture_shedding(Vec::new(), None, Some(keys[1].clone())).await;
        let ops = vec![
            put_op("1001", FOLD_MAP, &keys[0], allowed_fields("first")),
            put_op("1002", FOLD_MAP, &keys[1], allowed_fields("shed")),
            put_op("1003", FOLD_MAP, &keys[2], allowed_fields("third")),
        ];

        let msgs = decode_frames(&dispatch_batch_frames(&fx, ops).await);

        let errors = error_payloads(&msgs);
        assert_eq!(
            errors.len(),
            1,
            "one batch-level error, not one per operation"
        );
        assert_eq!(errors[0].code, 429);
        assert!(
            rejections(&msgs).is_empty(),
            "a transient failure is never a refusal"
        );
        assert!(
            ack_payloads(&msgs).is_empty(),
            "nothing may be acknowledged for a sub-batch that failed"
        );
        assert_eq!(
            fx.service_calls.load(Ordering::Relaxed),
            1,
            "a transient sub-batch failure must not be re-dispatched per operation"
        );
    }

    /// A transient failure raised by the THIRD singleton of a fallback yields the
    /// 429 `ERROR` frame with no acknowledgement, behind the rejections the
    /// earlier singletons already settled.
    ///
    /// This is the server-side half of the clause (c) test above and must not
    /// drift from it: both establish "no operation was marked synced" the same
    /// way, by asserting no acknowledgement is sent at all.
    #[tokio::test]
    async fn transient_mid_fallback_yields_429_after_earlier_rejections() {
        let keys = keys_in_one_partition(3);
        let fx =
            build_fold_fixture_shedding(deny_blocked_records(), None, Some(keys[2].clone())).await;
        let ops = vec![
            put_op("1101", FOLD_MAP, &keys[0], blocked_fields()),
            put_op("1102", FOLD_MAP, &keys[1], blocked_fields()),
            put_op("1103", FOLD_MAP, &keys[2], allowed_fields("shed")),
        ];

        let msgs = decode_frames(&dispatch_batch_frames(&fx, ops).await);

        let error_at = msgs
            .iter()
            .position(|msg| matches!(msg, TopGunMessage::Error { .. }))
            .expect("the transient failure is reported as an ERROR frame");
        let TopGunMessage::Error { payload } = &msgs[error_at] else {
            unreachable!("position matched an ERROR frame")
        };
        assert_eq!(payload.code, 429);

        let last_rejection = msgs
            .iter()
            .rposition(|msg| matches!(msg, TopGunMessage::OpRejected(_)))
            .expect("the earlier singletons were refused");
        assert!(
            last_rejection < error_at,
            "rejections already settled must precede the ERROR frame"
        );
        assert_eq!(rejections(&msgs).len(), 2);
        assert!(
            ack_payloads(&msgs).is_empty(),
            "no ack at all, even though nothing before the transient was accepted"
        );
    }
}
