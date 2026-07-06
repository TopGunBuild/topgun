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
    AuthAckData, ErrorPayload, Message as TopGunMessage, OpAckMessage, OpAckPayload, WriteConcern,
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
use crate::service::operation::{CallerOrigin, ClassifyError, OperationError, OperationResponse};
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

                // Opportunistic device-identity AUTH, handled at the websocket layer.
                // In NO_AUTH mode a new client sends AUTH{token:"", deviceToken?} as its
                // first frame; run present-or-mint under the sentinel namespace and reply
                // AUTH_ACK. The Auth message NEVER enters data-plane dispatch. In JWT mode
                // Phase-2 re-auth is unsupported, so a stray Auth is simply dropped here.
                if let TopGunMessage::Auth(ref auth_msg) = tg_msg {
                    if state.jwt_secret.is_none() {
                        // One-shot binding (explicit Phase-2 guard): once an identity is
                        // bound, drop any further AUTH so in-flight identity-scoped state
                        // is never silently re-attributed. The read loop is sequential, so
                        // the first AUTH binds and all later ones see device_id = Some.
                        let already_bound = handle.metadata.read().await.device_id.is_some();
                        if already_bound {
                            debug!("dropping repeat AUTH on bound connection {:?}", conn_id);
                            continue;
                        }
                        let (device_id, device_token) = bind_device_identity(
                            &state,
                            &handle,
                            conn_id,
                            None,
                            auth_msg.device_token.as_deref(),
                        )
                        .await;
                        // Reply AUTH_ACK only when an identity was actually bound (a store
                        // must be wired); network-only test servers stay identity-less.
                        if device_id.is_some() {
                            let ack = TopGunMessage::AuthAck(AuthAckData {
                                device_id,
                                device_token,
                                ..Default::default()
                            });
                            if let Ok(bytes) = rmp_serde::to_vec_named(&ack) {
                                let _ = handle.tx.send(OutboundMessage::Binary(bytes)).await;
                            }
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
            if let Some(displaced) = state.registry.claim_device_ownership(identity_key, conn_id) {
                if let Some(old) = state.registry.get(displaced) {
                    old.cancel();
                }
                warn!(
                    "device-identity takeover on {:?}: displaced {:?}",
                    conn_id, displaced
                );
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

/// Splits an `OpBatch` by partition and dispatches all sub-batches concurrently.
///
/// Groups the batch's ops by `hash_to_partition(key)`, creates one
/// `Operation::OpBatch` per partition group (each carrying `partition_id=Some(id)`
/// so the dispatcher routes it to the correct partition worker instead of the
/// single global worker), dispatches all groups concurrently, and sends a single
/// `OP_ACK` with `lastId` from the last op in the original batch.
///
/// Per-sub-batch `OpAck` responses from `CrdtService::handle_op_batch()` are
/// discarded; the aggregated ack is constructed from the original batch's
/// last-op ID so the client always receives exactly one `OP_ACK`.
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
        let ack = TopGunMessage::OpAck(OpAckMessage {
            payload: OpAckPayload {
                last_id: "unknown".to_string(),
                ..Default::default()
            },
        });
        if let Ok(bytes) = rmp_serde::to_vec_named(&ack) {
            let _ = tx.send(OutboundMessage::Binary(bytes)).await;
        }
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

    let write_concern = batch_msg.payload.write_concern.clone();
    let timeout = batch_msg.payload.timeout;

    // Build all sub-batch operations up front, then dispatch concurrently.
    let mut sub_ops: Vec<crate::service::operation::Operation> =
        Vec::with_capacity(partition_groups.len());
    for (partition_id, group_ops) in partition_groups {
        let mut op = classify_svc.classify_op_batch_for_partition(
            group_ops,
            partition_id,
            None,
            CallerOrigin::Client,
            write_concern.clone(),
            timeout,
        );
        op.set_connection_id(conn_id);
        if let Some(p) = principal.clone() {
            op.set_principal(p);
        }
        sub_ops.push(op);
    }

    // Dispatch all sub-batches concurrently; collect results.
    let mut join_set = tokio::task::JoinSet::new();
    for sub_op in sub_ops {
        let dispatcher = Arc::clone(dispatcher);
        join_set.spawn(async move { dispatcher.dispatch(sub_op).await });
    }

    // Collect results and check for errors. Per-sub-batch OpAck responses are
    // discarded; the aggregated OP_ACK is built from the original batch's lastId.
    // Preserve the OperationError type so we can distinguish 429 from 500
    // without inspecting string content.
    let mut dispatch_error: Option<OperationError> = None;
    while let Some(result) = join_set.join_next().await {
        match result {
            Ok(Ok(_resp)) => {
                // Discard the per-sub-batch OpAck; we send one aggregated ack below.
            }
            Ok(Err(e)) => {
                dispatch_error = Some(e);
            }
            Err(join_err) => {
                dispatch_error = Some(OperationError::Internal(anyhow::anyhow!(
                    "join error: {join_err}"
                )));
            }
        }
    }

    if let Some(err) = dispatch_error {
        debug!("dispatch_op_batch error for {:?}: {}", conn_id, err);
        let (code, message) = match err {
            OperationError::Overloaded => (429, "server overloaded, try again later".to_string()),
            ref e => (500, format!("{e}")),
        };
        let err_response = TopGunMessage::Error {
            payload: ErrorPayload {
                code,
                message,
                details: None,
            },
        };
        if let Ok(bytes) = rmp_serde::to_vec_named(&err_response) {
            let _ = tx.send(OutboundMessage::Binary(bytes)).await;
        }
        return;
    }

    // All sub-batches succeeded — send one OP_ACK with the original batch's lastId.
    // Sub-batch responses are discarded; set APPLIED explicitly on the aggregated ack
    // because each sub-batch's CRDT merge succeeded in memory.
    let ack = TopGunMessage::OpAck(OpAckMessage {
        payload: OpAckPayload {
            last_id,
            achieved_level: Some(WriteConcern::APPLIED),
            ..Default::default()
        },
    });
    if let Ok(bytes) = rmp_serde::to_vec_named(&ack) {
        let _ = tx.send(OutboundMessage::Binary(bytes)).await;
    }
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
}
