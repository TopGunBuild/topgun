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
    #[serde(default)]
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
    /// The `leeway` parameter specifies the clock skew tolerance in seconds.
    /// Tokens expired within this window are still accepted to handle clock
    /// drift between clients and the server.
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
        leeway: u64,
    ) -> Result<Principal, AuthError> {
        let mut validation = Validation::new(Algorithm::HS256);
        // Disable audience/issuer checks since TopGun tokens do not use them.
        // Do NOT clear required_spec_claims: the jsonwebtoken crate defaults to
        // requiring `exp`, which enforces token expiry validation.
        validation.validate_aud = false;
        validation.leeway = leeway;

        let key = DecodingKey::from_secret(self.jwt_secret.as_bytes());

        match jsonwebtoken::decode::<JwtClaims>(&auth_msg.token, &key, &validation) {
            Ok(token_data) => {
                // Reject tokens without a subject claim — anonymous identity is
                // not permitted; callers must always provide a `sub` field.
                let Some(user_id) = token_data.claims.sub else {
                    warn!("JWT accepted by signature but missing required `sub` claim");
                    let fail_msg = Message::AuthFail(AuthFailData {
                        error: Some("missing sub claim in JWT".to_string()),
                        ..Default::default()
                    });
                    let bytes = rmp_serde::to_vec_named(&fail_msg)?;
                    tx.send(OutboundMessage::Binary(bytes)).await?;
                    return Err(AuthError::InvalidToken {
                        reason: "missing sub claim in JWT".to_string(),
                    });
                };

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

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{EncodingKey, Header};
    use serde::Serialize;
    use std::time::{SystemTime, UNIX_EPOCH};
    use tokio::sync::mpsc;
    use topgun_core::messages::AuthMessage;

    const TEST_SECRET: &str = "test-unit-secret";

    /// Minimal claims struct for building test tokens.
    #[derive(Serialize)]
    struct TestClaims {
        sub: Option<String>,
        exp: u64,
    }

    /// Encode a token with the given claims using HS256 and the test secret.
    fn make_token(sub: Option<&str>, exp_offset_secs: i64) -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after epoch")
            .as_secs();
        // Cast carefully: if offset is negative the result saturates to 0
        let exp = if exp_offset_secs >= 0 {
            now + exp_offset_secs as u64
        } else {
            now.saturating_sub((-exp_offset_secs) as u64)
        };
        let claims = TestClaims {
            sub: sub.map(str::to_owned),
            exp,
        };
        jsonwebtoken::encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(TEST_SECRET.as_bytes()),
        )
        .expect("test token encoding should not fail")
    }

    /// Create an `AuthHandler` and a channel pair for testing.
    fn setup() -> (AuthHandler, mpsc::Sender<OutboundMessage>, mpsc::Receiver<OutboundMessage>) {
        let handler = AuthHandler::new(TEST_SECRET.to_owned());
        let (tx, rx) = mpsc::channel(8);
        (handler, tx, rx)
    }

    // AC2: valid token (exp 1 hour from now) is accepted
    #[tokio::test]
    async fn valid_token_accepted() {
        let (handler, tx, _rx) = setup();
        let token = make_token(Some("user-1"), 3600);
        let auth_msg = AuthMessage { token, protocol_version: None };
        let result = handler.handle_auth(&auth_msg, &tx, 60).await;
        assert!(result.is_ok(), "expected Ok, got {result:?}");
        let principal = result.unwrap();
        assert_eq!(principal.id, "user-1");
    }

    // AC1: token expired 1 hour ago is rejected
    #[tokio::test]
    async fn expired_token_rejected() {
        let (handler, tx, mut rx) = setup();
        let token = make_token(Some("user-1"), -3600);
        let auth_msg = AuthMessage { token, protocol_version: None };
        let result = handler.handle_auth(&auth_msg, &tx, 60).await;
        assert!(result.is_err(), "expected Err for expired token");
        // AUTH_FAIL must have been sent on the channel (AC12 for expired path)
        assert!(
            rx.try_recv().is_ok(),
            "AUTH_FAIL should be sent when token is rejected"
        );
    }

    // AC3: token expired 30 seconds ago is accepted when leeway is 60s
    #[tokio::test]
    async fn token_within_leeway_accepted() {
        let (handler, tx, _rx) = setup();
        let token = make_token(Some("user-1"), -30);
        let auth_msg = AuthMessage { token, protocol_version: None };
        let result = handler.handle_auth(&auth_msg, &tx, 60).await;
        assert!(
            result.is_ok(),
            "token 30s expired should be accepted within 60s leeway, got {result:?}"
        );
    }

    // AC4: token expired 90 seconds ago is rejected when leeway is 60s
    #[tokio::test]
    async fn token_beyond_leeway_rejected() {
        let (handler, tx, mut rx) = setup();
        let token = make_token(Some("user-1"), -90);
        let auth_msg = AuthMessage { token, protocol_version: None };
        let result = handler.handle_auth(&auth_msg, &tx, 60).await;
        assert!(
            result.is_err(),
            "token 90s expired should be rejected with 60s leeway"
        );
        assert!(
            rx.try_recv().is_ok(),
            "AUTH_FAIL should be sent when token is beyond leeway"
        );
    }

    // AC8 + AC12: token with valid signature but no sub is rejected and AUTH_FAIL is sent
    #[tokio::test]
    async fn missing_sub_rejected_with_auth_fail() {
        let (handler, tx, mut rx) = setup();
        let token = make_token(None, 3600);
        let auth_msg = AuthMessage { token, protocol_version: None };
        let result = handler.handle_auth(&auth_msg, &tx, 60).await;
        assert!(result.is_err(), "expected Err when sub is missing");
        match result.unwrap_err() {
            AuthError::InvalidToken { reason } => {
                assert!(
                    reason.contains("sub"),
                    "error reason should mention 'sub', got: {reason}"
                );
            }
            e => panic!("expected InvalidToken, got {e:?}"),
        }
        // AC12: AUTH_FAIL must be sent before returning the error
        assert!(
            rx.try_recv().is_ok(),
            "AUTH_FAIL should be sent when sub claim is missing"
        );
    }
}
