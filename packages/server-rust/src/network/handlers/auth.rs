//! Authentication handshake handler for WebSocket connections.
//!
//! Implements the `AUTH_REQUIRED` -> `AUTH` -> `AUTH_ACK`/`AUTH_FAIL` flow.
//! Uses a two-phase approach:
//!   - `send_auth_required`: sends on the raw `axum::extract::ws::WebSocket` before split
//!   - `handle_auth`: sends via `mpsc::Sender<OutboundMessage>` after split

use axum::extract::ws::WebSocket;
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use tokio::sync::mpsc;
use topgun_core::messages::{AuthFailData, AuthMessage, AuthRequiredMessage, Message};
use topgun_core::Principal;
use tracing::{debug, warn};

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

/// JWT claims extracted from the authentication token.
///
/// Uses the standard `sub` (subject) claim for user identification per RFC 7519.
#[derive(Debug, Deserialize)]
pub struct JwtClaims {
    /// User identifier -- standard JWT `sub` (subject) claim.
    pub sub: Option<String>,
    /// Roles assigned to this principal (e.g., `["admin"]`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub roles: Option<Vec<String>>,
    /// Token expiry timestamp (seconds since Unix epoch). The `jsonwebtoken`
    /// crate validates `exp` from raw JSON even without a struct field, but
    /// making it explicit enables future use and ensures consistent serde
    /// behavior across all `Option<T>` fields.
    #[serde(default)]
    pub exp: Option<u64>,
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

    /// Send `AUTH_REQUIRED` message to the client.
    ///
    /// Called immediately on WebSocket connect, BEFORE the socket is split,
    /// so it takes the raw axum `WebSocket` directly. Serializes the message
    /// via `rmp_serde::to_vec_named()` and sends as a binary frame.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization or WebSocket send fails.
    pub async fn send_auth_required(
        &self,
        socket: &mut WebSocket,
    ) -> Result<(), anyhow::Error> {
        let msg = Message::AuthRequired(AuthRequiredMessage {});
        let bytes = rmp_serde::to_vec_named(&msg)?;
        socket
            .send(axum::extract::ws::Message::Binary(bytes.into()))
            .await
            .map_err(|e| anyhow::anyhow!("failed to send AUTH_REQUIRED: {e}"))?;
        debug!("sent AUTH_REQUIRED to client");
        Ok(())
    }

    /// Process an incoming AUTH message.
    ///
    /// Returns `Ok(Principal)` on success so the caller can update
    /// `ConnectionMetadata` and send `AUTH_ACK` with the user's identity.
    ///
    /// On failure, sends `AUTH_FAIL` via the outbound channel and returns
    /// `Err(AuthError)`. The caller should close the connection.
    ///
    /// # Errors
    ///
    /// Returns `AuthError::InvalidToken` if the JWT is invalid or expired.
    /// Returns `AuthError::SendFailed` if the outbound channel is closed.
    /// Returns `AuthError::Serialization` if message encoding fails.
    pub async fn handle_auth(
        &self,
        auth_msg: &AuthMessage,
        tx: &mpsc::Sender<OutboundMessage>,
    ) -> Result<Principal, AuthError> {
        let mut validation = Validation::new(Algorithm::HS256);
        // Disable audience/issuer checks since TopGun tokens do not use them
        validation.validate_aud = false;
        validation.required_spec_claims.clear();

        let key = DecodingKey::from_secret(self.jwt_secret.as_bytes());

        match jsonwebtoken::decode::<JwtClaims>(&auth_msg.token, &key, &validation) {
            Ok(token_data) => {
                let user_id = token_data
                    .claims
                    .sub
                    .unwrap_or_else(|| "anonymous".to_string());

                let roles = token_data.claims.roles.unwrap_or_default();
                debug!(user_id = %user_id, ?roles, "JWT verified successfully");

                Ok(Principal {
                    id: user_id,
                    roles,
                })
            }
            Err(e) => {
                let reason = format!("{e}");
                warn!(error = %reason, "JWT verification failed");

                // Send AUTH_FAIL with error description via outbound channel
                let fail_msg = Message::AuthFail(AuthFailData {
                    error: Some(reason.clone()),
                    ..Default::default()
                });
                let bytes = rmp_serde::to_vec_named(&fail_msg)?;
                tx.send(OutboundMessage::Binary(bytes)).await?;

                // No Close frame sent here: the caller (ws_handler) breaks out of the
                // message loop on Err(AuthError::InvalidToken), dropping the sender half
                // of the mpsc channel. This causes the writer task to shut down and close
                // the socket, giving the client time to process AUTH_FAIL first.
                Err(AuthError::InvalidToken { reason })
            }
        }
    }
}
