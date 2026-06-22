//! Minimal WebSocket client for the soak harness.
//!
//! Connects to the real out-of-process `topgun-server`, completes the JWT auth
//! handshake, and exposes the four operations the soak loop needs:
//!
//! - `write_lww` — PUT an LWW record and wait for `OP_ACK` (durable write).
//! - `read_all` — `QUERY_SUB` with an empty query → full map snapshot, used by
//!   the convergence verifier to read every key back.
//! - `merkle_root` — `SYNC_INIT` → `SYNC_RESP_ROOT`, the single-`u32` Merkle
//!   fingerprint compared across clients and across a crash/recovery boundary.
//!
//! Unlike the load harness `ConnectionPool`, a `SoakClient` owns a single
//! connection that is opened and dropped repeatedly by the churn driver, so the
//! type is deliberately small and cheap to construct.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use futures_util::{SinkExt, StreamExt};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use topgun_core::hlc::{LWWRecord, ORMapRecord, Timestamp};
use topgun_core::messages::{
    AuthMessage, ClientOp, Message, OpBatchMessage, OpBatchPayload, Query, QuerySubMessage,
    QuerySubPayload, QueryUnsubMessage, QueryUnsubPayload, SyncInitMessage, WriteConcern,
};

type Ws = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

/// How long a single request/response exchange may take before giving up.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// JWT claims for a soak connection. Mirrors the load harness shape so the same
/// `JWT_SECRET=test-e2e-secret` validates.
#[derive(Debug, Serialize, Deserialize)]
struct SoakClaims {
    sub: String,
    iat: u64,
    exp: u64,
}

/// A single authenticated WebSocket connection to the soak server.
pub struct SoakClient {
    ws: Ws,
    /// Stable subject id, reused for log correlation.
    user: String,
}

impl SoakClient {
    /// Open a connection to `addr` and complete the auth handshake as
    /// `soak-user-{user_idx}`.
    pub async fn connect(addr: SocketAddr, user_idx: usize, jwt_secret: &str) -> Result<Self> {
        let url = format!("ws://{addr}/ws");
        let (mut ws, _resp) = tokio_tungstenite::connect_async(&url)
            .await
            .map_err(|e| anyhow!("user {user_idx}: ws connect failed: {e}"))?;

        // AUTH_REQUIRED → AUTH(jwt) → AUTH_ACK
        let _auth_required = recv_decoded(&mut ws, "AUTH_REQUIRED").await?;

        let token = generate_jwt(user_idx, jwt_secret)?;
        let auth = Message::Auth(AuthMessage {
            token,
            protocol_version: None,
        });
        send_encoded(&mut ws, &auth).await?;

        match recv_decoded(&mut ws, "AUTH_ACK").await? {
            Message::AuthAck(_) => {}
            Message::AuthFail(fail) => {
                let reason = fail.error.unwrap_or_else(|| "unknown".to_string());
                bail!("user {user_idx}: auth failed: {reason}");
            }
            other => bail!("user {user_idx}: expected AUTH_ACK, got {other:?}"),
        }

        Ok(Self {
            ws,
            user: format!("soak-{user_idx}"),
        })
    }

    /// PUT `value` at `map`/`key` with `APPLIED` write concern and wait for the
    /// `OP_ACK`. The `(millis, counter)` pair must be monotonically increasing
    /// per key so Last-Write-Wins keeps the latest value; the caller owns that.
    pub async fn write_lww(
        &mut self,
        map: &str,
        key: &str,
        value: i64,
        millis: u64,
        counter: u32,
    ) -> Result<()> {
        let record = LWWRecord {
            value: Some(rmpv::Value::Map(vec![(
                rmpv::Value::from("v"),
                rmpv::Value::from(value),
            )])),
            timestamp: Timestamp {
                millis,
                counter,
                node_id: self.user.clone(),
            },
            ttl_ms: None,
        };
        let op = ClientOp {
            map_name: map.to_string(),
            key: key.to_string(),
            op_type: Some("PUT".to_string()),
            record: Some(Some(record)),
            ..Default::default()
        };
        self.send_op_await_ack(op).await
    }

    /// Add an OR-Map tag at `map`/`key`. Paired with `or_remove`, this drives
    /// per-key tombstone accumulation — the unbounded-growth memory candidate
    /// the soak watches (TODO-479/480). Waits for `OP_ACK`.
    pub async fn or_add(
        &mut self,
        map: &str,
        key: &str,
        tag: &str,
        millis: u64,
        counter: u32,
    ) -> Result<()> {
        let or_record = ORMapRecord {
            value: rmpv::Value::from(counter),
            timestamp: Timestamp {
                millis,
                counter,
                node_id: self.user.clone(),
            },
            tag: tag.to_string(),
            ttl_ms: None,
        };
        let op = ClientOp {
            map_name: map.to_string(),
            key: key.to_string(),
            or_record: Some(Some(or_record)),
            ..Default::default()
        };
        self.send_op_await_ack(op).await
    }

    /// Remove an OR-Map tag previously added at `map`/`key`, appending a
    /// tombstone server-side. Waits for `OP_ACK`.
    pub async fn or_remove(&mut self, map: &str, key: &str, tag: &str) -> Result<()> {
        let op = ClientOp {
            map_name: map.to_string(),
            key: key.to_string(),
            or_tag: Some(Some(tag.to_string())),
            ..Default::default()
        };
        self.send_op_await_ack(op).await
    }

    /// Send a single op in an `APPLIED` batch and wait for its `OP_ACK`.
    async fn send_op_await_ack(&mut self, op: ClientOp) -> Result<()> {
        let msg = Message::OpBatch(OpBatchMessage {
            payload: OpBatchPayload {
                ops: vec![op],
                write_concern: Some(WriteConcern::APPLIED),
                timeout: Some(5000),
            },
        });
        send_encoded(&mut self.ws, &msg).await?;
        match recv_decoded(&mut self.ws, "OP_ACK").await? {
            Message::OpAck(_) => Ok(()),
            Message::OpRejected(r) => bail!("op rejected: {}", r.payload.reason),
            other => bail!("expected OP_ACK, got {other:?}"),
        }
    }

    /// Read every record of `map` back via an empty `QUERY_SUB`, returning a
    /// `key -> v` map. Values are decoded from the `{ "v": <int> }` shape the
    /// soak writes. Unsubscribes afterwards so the live subscription does not
    /// leak across repeated checkpoint reads.
    pub async fn read_all(&mut self, map: &str) -> Result<HashMap<String, i64>> {
        let query_id = format!("soak-read-{}-{map}", self.user);
        let sub = Message::QuerySub(QuerySubMessage {
            payload: QuerySubPayload {
                query_id: query_id.clone(),
                map_name: map.to_string(),
                query: Query::default(),
                fields: None,
            },
        });
        send_encoded(&mut self.ws, &sub).await?;

        let resp = recv_decoded(&mut self.ws, "QUERY_RESP").await?;
        let Message::QueryResp(qr) = resp else {
            bail!("expected QUERY_RESP, got {resp:?}");
        };

        if qr.payload.has_more == Some(true) {
            bail!(
                "QUERY_RESP for '{map}' was paginated (has_more) — soak keyspace exceeds the \
                 single-page max_query_records cap; lower --keyspace"
            );
        }

        let mut out = HashMap::with_capacity(qr.payload.results.len());
        for entry in qr.payload.results {
            if let Some(v) = extract_v(&entry.value) {
                out.insert(entry.key, v);
            }
        }

        // Best-effort unsubscribe; ignore the (rare) send error on a dying conn.
        let unsub = Message::QueryUnsub(QueryUnsubMessage {
            payload: QueryUnsubPayload { query_id },
        });
        let _ = send_encoded(&mut self.ws, &unsub).await;

        Ok(out)
    }

    /// Fetch the server's Merkle root hash for `map` via `SYNC_INIT`. Two
    /// clients reading the same single-node server must agree; the value must
    /// also be identical before a quiesced `kill -9` and after WAL recovery.
    pub async fn merkle_root(&mut self, map: &str) -> Result<u32> {
        let init = Message::SyncInit(SyncInitMessage {
            map_name: map.to_string(),
            last_sync_timestamp: None,
        });
        send_encoded(&mut self.ws, &init).await?;

        match recv_decoded(&mut self.ws, "SYNC_RESP_ROOT").await? {
            Message::SyncRespRoot(r) => Ok(r.payload.root_hash),
            other => bail!("expected SYNC_RESP_ROOT, got {other:?}"),
        }
    }
}

/// Decode the soak value shape `{ "v": <int> }` back to an `i64`.
fn extract_v(value: &rmpv::Value) -> Option<i64> {
    value
        .as_map()?
        .iter()
        .find(|(k, _)| k.as_str() == Some("v"))
        .and_then(|(_, v)| v.as_i64())
}

/// Send a `Message` as a binary MsgPack frame.
async fn send_encoded(ws: &mut Ws, msg: &Message) -> Result<()> {
    let bytes = rmp_serde::to_vec_named(msg).map_err(|e| anyhow!("encode failed: {e}"))?;
    ws.send(WsMessage::Binary(bytes.into()))
        .await
        .map_err(|e| anyhow!("ws send failed: {e}"))
}

/// Receive the next decodable `Message`, skipping control frames and unrelated
/// server pushes (e.g. `QUERY_UPDATE`, `SERVER_EVENT`) until a frame decodes to
/// a `Message`. `what` names the awaited message for error context. Bounded by
/// `REQUEST_TIMEOUT` so a wedged server surfaces as an error, not a hang.
async fn recv_decoded(ws: &mut Ws, what: &str) -> Result<Message> {
    let deadline = tokio::time::Instant::now() + REQUEST_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            bail!("timed out waiting for {what}");
        }
        let frame = match tokio::time::timeout(remaining, ws.next()).await {
            Ok(Some(Ok(frame))) => frame,
            Ok(Some(Err(e))) => bail!("ws error while waiting for {what}: {e}"),
            Ok(None) => bail!("connection closed while waiting for {what}"),
            Err(_) => bail!("timed out waiting for {what}"),
        };
        match frame {
            WsMessage::Binary(data) => {
                if let Ok(msg) = rmp_serde::from_slice::<Message>(&data) {
                    return Ok(msg);
                }
                // Undecodable frame — skip and keep waiting.
            }
            WsMessage::Text(text) => {
                if let Ok(msg) = rmp_serde::from_slice::<Message>(text.as_bytes()) {
                    return Ok(msg);
                }
            }
            WsMessage::Close(_) => bail!("server closed connection while waiting for {what}"),
            WsMessage::Ping(_) | WsMessage::Pong(_) | WsMessage::Frame(_) => {}
        }
    }
}

/// Mint an HS256 JWT for `soak-user-{idx}` valid for one hour.
fn generate_jwt(idx: usize, jwt_secret: &str) -> Result<String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| anyhow!("system time error: {e}"))?
        .as_secs();
    let claims = SoakClaims {
        sub: format!("soak-user-{idx}"),
        iat: now,
        exp: now + 3600,
    };
    jsonwebtoken::encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(jwt_secret.as_bytes()),
    )
    .map_err(|e| anyhow!("JWT encode failed for user {idx}: {e}"))
}
