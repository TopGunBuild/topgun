//! Authentication handshake handler for WebSocket connections.
//!
//! Implements the AUTH_REQUIRED -> AUTH -> AUTH_ACK/AUTH_FAIL flow.
//! Uses a two-phase approach:
//!   - `send_auth_required`: sends on the raw `axum::extract::ws::WebSocket` before split
//!   - `handle_auth`: sends via `mpsc::Sender<OutboundMessage>` after split

use tokio::sync::mpsc;
use topgun_core::messages::AuthMessage;
use topgun_core::Principal;

use crate::network::OutboundMessage;

/// Errors that can occur during authentication.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    /// JWT token is invalid, expired, or has missing claims.
    #[error("authentication failed: {reason}")]
    InvalidToken { reason: String },
    /// Failed to send a message on the outbound channel.
    #[error("failed to send auth response: {0}")]
    SendFailed(#[from] mpsc::error::SendError<OutboundMessage>),
    /// Serialization error when encoding auth messages.
    #[error("serialization error: {0}")]
    Serialization(#[from] rmp_serde::encode::Error),
}

/// Handles the authentication handshake for WebSocket connections.
///
/// JWT verification uses HS256 algorithm with the configured secret.
pub struct AuthHandler {
    jwt_secret: String,
}

impl AuthHandler {
    /// Create a new `AuthHandler` with the given JWT secret.
    #[must_use]
    pub fn new(jwt_secret: String) -> Self {
        Self { jwt_secret }
    }

    /// Send AUTH_REQUIRED message to the client.
    ///
    /// Called immediately on WebSocket connect, BEFORE the socket is split,
    /// so it takes the raw axum WebSocket directly.
    pub async fn send_auth_required(
        &self,
        _socket: &mut axum::extract::ws::WebSocket,
    ) -> Result<(), anyhow::Error> {
        todo!()
    }

    /// Process an incoming AUTH message.
    ///
    /// Returns `Ok(Principal)` on success so the caller can update
    /// `ConnectionMetadata` and send AUTH_ACK with the user's identity.
    ///
    /// On failure, sends AUTH_FAIL via the outbound channel and returns
    /// `Err(AuthError)`. The caller should close the connection.
    pub async fn handle_auth(
        &self,
        _auth_msg: &AuthMessage,
        _tx: &mpsc::Sender<OutboundMessage>,
    ) -> Result<Principal, AuthError> {
        todo!()
    }
}
