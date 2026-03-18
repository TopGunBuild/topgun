use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use futures_util::future::join_all;
use futures_util::{SinkExt, StreamExt};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use topgun_core::messages::{AuthMessage, Message};

type WsSink = futures_util::stream::SplitSink<
    WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
    WsMessage,
>;
type WsStream =
    futures_util::stream::SplitStream<WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>>;

type Connection = (Arc<Mutex<WsSink>>, Arc<Mutex<WsStream>>);

/// JWT claims used for load test connections.
#[derive(Debug, Serialize, Deserialize)]
struct LoadTestClaims {
    sub: String,
    iat: u64,
    exp: u64,
}

/// Pool of authenticated WebSocket connections to the server.
///
/// Connections are opened in batches to avoid overwhelming the server with
/// simultaneous SYN packets, and each connection completes the full auth
/// handshake before being considered ready.
pub struct ConnectionPool {
    connections: Vec<Connection>,
}

impl ConnectionPool {
    /// Open `pool_size` WebSocket connections to `addr`, completing auth on each.
    ///
    /// Connections are opened in batches of `batch_size` (default 500) with a
    /// 10ms delay between batches to prevent SYN flooding.
    pub async fn new(addr: SocketAddr, pool_size: usize, jwt_secret: &str) -> Result<Self> {
        const BATCH_SIZE: usize = 500;
        let batch_size = BATCH_SIZE;
        let mut connections: Vec<Connection> = Vec::with_capacity(pool_size);

        for batch_start in (0..pool_size).step_by(batch_size) {
            let batch_end = (batch_start + batch_size).min(pool_size);

            let mut handles = Vec::with_capacity(batch_end - batch_start);
            for idx in batch_start..batch_end {
                let secret = jwt_secret.to_string();
                let handle =
                    tokio::spawn(async move { connect_and_auth(addr, idx, &secret).await });
                handles.push(handle);
            }

            for (offset, handle) in handles.into_iter().enumerate() {
                let idx = batch_start + offset;
                let conn = handle
                    .await
                    .map_err(|e| anyhow!("task panicked for connection {idx}: {e}"))??;
                connections.push(conn);
            }

            let opened = batch_end;
            tracing::info!("Opened {opened}/{pool_size} connections");

            // Throttle batch opening to avoid SYN flooding
            if batch_end < pool_size {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }

        Ok(Self { connections })
    }

    /// Send a binary message to a specific connection by index.
    pub async fn send_to(&self, conn_idx: usize, msg: &[u8]) -> Result<()> {
        let (sink, _) = self
            .connections
            .get(conn_idx)
            .ok_or_else(|| anyhow!("connection index {conn_idx} out of range"))?;
        let mut sink = sink.lock().await;
        sink.send(WsMessage::Binary(msg.to_vec().into()))
            .await
            .map_err(|e| anyhow!("send_to({conn_idx}) failed: {e}"))
    }

    /// Send a binary message to every connection in parallel.
    #[allow(dead_code)]
    pub async fn broadcast(&self, msg: &[u8]) -> Result<()> {
        let sends: Vec<_> = self
            .connections
            .iter()
            .enumerate()
            .map(|(idx, (sink, _))| {
                let sink = Arc::clone(sink);
                let bytes = msg.to_vec();
                async move {
                    let mut guard = sink.lock().await;
                    guard
                        .send(WsMessage::Binary(bytes.into()))
                        .await
                        .map_err(|e| anyhow!("broadcast to connection {idx} failed: {e}"))
                }
            })
            .collect();

        let results = join_all(sends).await;
        for result in results {
            result?;
        }
        Ok(())
    }

    /// Receive the next message from a specific connection by index.
    pub async fn recv_from(&self, conn_idx: usize) -> Result<Vec<u8>> {
        let (_, stream) = self
            .connections
            .get(conn_idx)
            .ok_or_else(|| anyhow!("connection index {conn_idx} out of range"))?;
        let mut stream = stream.lock().await;
        loop {
            match stream.next().await {
                Some(Ok(WsMessage::Binary(data))) => return Ok(data.to_vec()),
                Some(Ok(WsMessage::Text(text))) => return Ok(text.as_bytes().to_vec()),
                Some(Ok(
                    WsMessage::Ping(_) | WsMessage::Pong(_) | WsMessage::Frame(_),
                )) => {}
                Some(Ok(WsMessage::Close(_))) => {
                    return Err(anyhow!("connection {conn_idx} closed by server"))
                }
                Some(Err(e)) => {
                    return Err(anyhow!("recv_from({conn_idx}) error: {e}"))
                }
                None => return Err(anyhow!("connection {conn_idx} stream ended")),
            }
        }
    }

    /// Gracefully close all connections in the pool.
    #[allow(dead_code)]
    pub async fn close_all(&self) {
        let closes: Vec<_> = self
            .connections
            .iter()
            .enumerate()
            .map(|(idx, (sink, _))| {
                let sink = Arc::clone(sink);
                async move {
                    let mut guard = sink.lock().await;
                    if let Err(e) = guard.close().await {
                        tracing::warn!("Failed to close connection {idx}: {e}");
                    }
                }
            })
            .collect();

        join_all(closes).await;
    }

    /// Return the number of connections in the pool.
    #[allow(dead_code)]
    pub fn size(&self) -> usize {
        self.connections.len()
    }
}

/// Generate a JWT token for a load test user.
fn generate_jwt(idx: usize, jwt_secret: &str) -> Result<String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| anyhow!("system time error: {e}"))?
        .as_secs();

    let claims = LoadTestClaims {
        sub: format!("load-user-{idx}"),
        iat: now,
        exp: now + 3600,
    };

    jsonwebtoken::encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(jwt_secret.as_bytes()),
    )
    .map_err(|e| anyhow!("JWT encoding failed for connection {idx}: {e}"))
}

/// Establish a single WebSocket connection with full auth handshake and retry on 429.
async fn connect_and_auth(addr: SocketAddr, idx: usize, jwt_secret: &str) -> Result<Connection> {
    let url = format!("ws://{addr}/ws");
    let max_retries = 3;

    let ws_stream = connect_with_backpressure_retry(&url, idx, max_retries).await?;
    let (mut sink, mut stream) = ws_stream.split();

    // Receive AUTH_REQUIRED
    let auth_required_bytes = recv_binary_message(&mut stream, idx).await?;
    let _auth_required: Message = rmp_serde::from_slice(&auth_required_bytes)
        .map_err(|e| anyhow!("connection {idx}: failed to decode AUTH_REQUIRED: {e}"))?;

    // Send AUTH with JWT
    let token = generate_jwt(idx, jwt_secret)?;
    let auth_msg = Message::Auth(AuthMessage {
        token,
        protocol_version: None,
    });
    let auth_bytes = rmp_serde::to_vec_named(&auth_msg)
        .map_err(|e| anyhow!("connection {idx}: failed to encode AUTH: {e}"))?;
    sink.send(WsMessage::Binary(auth_bytes.into()))
        .await
        .map_err(|e| anyhow!("connection {idx}: failed to send AUTH: {e}"))?;

    // Receive AUTH_ACK
    let ack_bytes = recv_binary_message(&mut stream, idx).await?;
    let ack_msg: Message = rmp_serde::from_slice(&ack_bytes)
        .map_err(|e| anyhow!("connection {idx}: failed to decode AUTH_ACK: {e}"))?;

    match ack_msg {
        Message::AuthAck(_data) => {}
        Message::AuthFail(fail) => {
            let reason = fail.error.unwrap_or_else(|| "unknown".to_string());
            return Err(anyhow!("connection {idx}: authentication failed: {reason}"));
        }
        _other => {
            return Err(anyhow!(
                "connection {idx}: expected AUTH_ACK, got unexpected message variant"
            ));
        }
    }

    let (sink, stream) = (Arc::new(Mutex::new(sink)), Arc::new(Mutex::new(stream)));
    Ok((sink, stream))
}

/// Connect to the WebSocket server, retrying up to `max_retries` times on HTTP 429.
async fn connect_with_backpressure_retry(
    url: &str,
    idx: usize,
    max_retries: usize,
) -> Result<WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>> {
    let mut attempt = 0;
    loop {
        match tokio_tungstenite::connect_async(url).await {
            Ok((ws, _response)) => return Ok(ws),
            Err(e) => {
                if is_429_error(&e) && attempt < max_retries {
                    attempt += 1;
                    tracing::warn!(
                        "Connection {idx} received 429, retrying ({attempt}/{max_retries})"
                    );
                    tokio::time::sleep(Duration::from_millis(100)).await;
                } else {
                    return Err(anyhow!("connection {idx}: WebSocket connect failed: {e}"));
                }
            }
        }
    }
}

/// Check whether a tungstenite error represents an HTTP 429 upgrade rejection.
fn is_429_error(e: &tokio_tungstenite::tungstenite::Error) -> bool {
    use tokio_tungstenite::tungstenite::Error;
    if let Error::Http(response) = e {
        return response.status() == 429;
    }
    false
}

/// Receive the next binary frame from a stream, skipping control frames.
async fn recv_binary_message(stream: &mut WsStream, idx: usize) -> Result<Vec<u8>> {
    loop {
        match stream.next().await {
            Some(Ok(WsMessage::Binary(data))) => return Ok(data.to_vec()),
            Some(Ok(
                WsMessage::Ping(_) | WsMessage::Pong(_) | WsMessage::Frame(_),
            )) => {}
            Some(Ok(WsMessage::Text(text))) => return Ok(text.as_bytes().to_vec()),
            Some(Ok(WsMessage::Close(_))) => {
                return Err(anyhow!("connection {idx}: server closed connection during auth"))
            }
            Some(Err(e)) => {
                return Err(anyhow!("connection {idx}: WebSocket error during auth: {e}"))
            }
            None => {
                return Err(anyhow!(
                    "connection {idx}: stream ended unexpectedly during auth"
                ))
            }
        }
    }
}
