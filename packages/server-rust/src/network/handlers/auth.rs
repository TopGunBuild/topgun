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

/// Normalize a PEM string received from environment variables.
///
/// Docker and Kubernetes environment variables often encode newlines as the
/// two-character sequence `\n` (backslash + n) rather than real newline
/// characters. This function replaces those escaped sequences with actual
/// newlines so that PEM parsing succeeds.
pub(crate) fn normalize_pem(input: &str) -> String {
    input.replace("\\n", "\n")
}

/// Detect the JWT algorithm and construct the appropriate `DecodingKey`.
///
/// If the (normalized) secret starts with `"-----BEGIN"`, it is treated as an
/// RSA public key in PEM format and RS256 is selected. Otherwise the secret is
/// treated as an HMAC shared secret and HS256 is selected.
///
/// Returns `Err` only when the input looks like PEM but cannot be parsed.
pub(crate) fn decode_jwt_key(secret: &str) -> Result<(Algorithm, DecodingKey), String> {
    let normalized = normalize_pem(secret);
    if normalized.starts_with("-----BEGIN") {
        let key = DecodingKey::from_rsa_pem(normalized.as_bytes())
            .map_err(|e| format!("failed to parse RSA PEM key: {e}"))?;
        Ok((Algorithm::RS256, key))
    } else {
        Ok((Algorithm::HS256, DecodingKey::from_secret(secret.as_bytes())))
    }
}

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
    /// When `insecure_forward_auth_errors` is `false` (the default), the
    /// `AUTH_FAIL` message sent to the client contains only a generic
    /// "Authentication failed" string. Detailed reasons are always logged at
    /// `warn` level regardless of this flag.
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
        insecure_forward_auth_errors: bool,
    ) -> Result<Principal, AuthError> {
        let (algorithm, key) = decode_jwt_key(&self.jwt_secret)
            .map_err(|reason| AuthError::InvalidToken { reason })?;
        let mut validation = Validation::new(algorithm);
        // Disable audience/issuer checks since TopGun tokens do not use them.
        // Do NOT clear required_spec_claims: the jsonwebtoken crate defaults to
        // requiring `exp`, which enforces token expiry validation.
        validation.validate_aud = false;
        validation.leeway = leeway;

        match jsonwebtoken::decode::<JwtClaims>(&auth_msg.token, &key, &validation) {
            Ok(token_data) => {
                // Reject tokens without a subject claim — anonymous identity is
                // not permitted; callers must always provide a `sub` field.
                let Some(user_id) = token_data.claims.sub else {
                    let detail = "missing sub claim in JWT";
                    warn!("JWT accepted by signature but missing required `sub` claim");
                    let client_error = if insecure_forward_auth_errors {
                        detail.to_string()
                    } else {
                        "Authentication failed".to_string()
                    };
                    let fail_msg = Message::AuthFail(AuthFailData {
                        error: Some(client_error),
                        ..Default::default()
                    });
                    let bytes = rmp_serde::to_vec_named(&fail_msg)?;
                    tx.send(OutboundMessage::Binary(bytes)).await?;
                    return Err(AuthError::InvalidToken {
                        reason: detail.to_string(),
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

                let client_error = if insecure_forward_auth_errors {
                    reason.clone()
                } else {
                    "Authentication failed".to_string()
                };

                // Send AUTH_FAIL via outbound channel. Detailed reason is logged
                // above; only the client-facing message is controlled by the flag.
                let fail_msg = Message::AuthFail(AuthFailData {
                    error: Some(client_error),
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
        let result = handler.handle_auth(&auth_msg, &tx, 60, false).await;
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
        let result = handler.handle_auth(&auth_msg, &tx, 60, false).await;
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
        let result = handler.handle_auth(&auth_msg, &tx, 60, false).await;
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
        let result = handler.handle_auth(&auth_msg, &tx, 60, false).await;
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
        let result = handler.handle_auth(&auth_msg, &tx, 60, false).await;
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

    // RSA 2048-bit test key pair (PKCS#8 PEM, generated for testing only).
    // Private key used to sign tokens; public key embedded in AuthHandler.
    const TEST_RSA_PRIVATE_PEM: &str = "-----BEGIN PRIVATE KEY-----
MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQCsOcDOoBAsEl9w
osjKuwm0TVX9wF0kWhOwSeHmGPn01o5ngBHfWeJ7a8CXE7U5rPsvIxIvjQvLiKsg
jyattjkaHsIGn+X1/vqsv7ETIKTSN4VBiPiVLmtl08vFGy2mp9FTNlbnkSh8JcK9
u2VqqOpKgZ2gt4MvZaLYnlD93POA3K0Aho4WtzQvg+hIseK2VKXlXVrUhhoac6yC
vsi/QjKjA+VUpu/IEZxXxDXpkqflX1uYzm1EFSPBlS+sDjprnOwc2ZUy1Dqd6q2y
zoIqgYh9uoRIbsEsLnBP1wr8Q5Zb/k0nKk8JfdO7jLndNY+NQmt8j8N89Z46/3eY
0q4RpskJAgMBAAECggEAEioFK8W17vABIOAKTVdsrpd5eknPiQX3DaC9Modv1WLL
oh7fw663NE0pAsYRVwPnehE42csGc3D2m3h9m9ScMSUNUaWLm2ZJCe8tvdazi3hP
lZncnd9HdHXiB+fV6L3KVfxlLgchPfa9k0UwbQ9jpngFJ+4y58zQYAhSgnPLOsve
40r6JFEkpGmU/zjcmztAL08DqVNoG/Ayyf1fGHTsTZ41h/bjHpGqR/umUDKD3mUu
nHKnH7KSdZGhd664gkxU15djQ3fvo8MKfycjHJJIB08loRjssF1cM2YucgUUBKGb
20pZNhI08/u+P1qY/RA2mgMXufuOQl3fR6v7sj4AAQKBgQDqfhNJ3aprU5aZt8Cj
TD64rrlJwaGd+KH3So8R7RqHaLDZ0tnuTo1Qd5Ug/Nj3+/YoV3SsBXHTz2vEySqE
6TtZC3BIX8fgOBS/NKkEutLxIWJA2Rvb9Z7TuvoqNrRz4C//Ov+da8ejAm145BEt
nICmJZAgTBWAIg2NIylJqgRF6QKBgQC8BaP0P747BhEbRvKWX7h3fTBmsNpbkFOI
Zc9grdukBclyY7YTpJlNS9DsMIvoLKGbfb+MarkoiIdhD1OzsZv6CKl9m4W4L896
Q2tVAKrdhicxDjEzF1+9JV3+sLAYpHyCL34VJ/SYykgPYISqi6zk9gwEfANeLKTR
XYpz4MHWIQKBgQCetrLLfjNI7YyzgoHqhUK2sdxLpbmEOLM3s8lecsNP/3YkGOjU
uWpAmo/fggRA5NNZvsgDXrQKjwv8Z8RVrZ8zx+A5vEqG4q54NGZqAyGff98G0Wxf
1sGnwZhtVhWRkJ4r/Hziyf6XwJ7kAkn2O0WAL1B768NptKLDcpcRevflcQKBgAwA
8C6vwx1RjdYH+YTQJ565R1XHBKnD1RFoLo0ljFg0Zl//LaijYYYlyPjLQKNZ9hdP
N+NnDNshnEL+D4HxXNvhobB7NVZE9yH/G+MZX880uVvQZCO24k3ZDN8tuJBaL/i/
v3TqUBtRDrismMuqjycu7iV7JVvlzb/wEN7FApsBAoGANHsJS78Hxl1yY7ABixpu
nRD/SytM0demZdDwwB4SEeBpWQxhrYDVkikJa0ZOLmmOpuZwk9Tfc8z7Kd6qSJX2
CHBa2gUH4KdtTxyw4E0nlwt4EO50DLl7fFYm9h5V/tZnO5IlSzZ3mMj0DlFuw9bG
Wto2YWk79zlL0d+wF6HV47I=
-----END PRIVATE KEY-----";

    const TEST_RSA_PUBLIC_PEM: &str = "-----BEGIN PUBLIC KEY-----
MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEArDnAzqAQLBJfcKLIyrsJ
tE1V/cBdJFoTsEnh5hj59NaOZ4AR31nie2vAlxO1Oaz7LyMSL40Ly4irII8mrbY5
Gh7CBp/l9f76rL+xEyCk0jeFQYj4lS5rZdPLxRstpqfRUzZW55EofCXCvbtlaqjq
SoGdoLeDL2Wi2J5Q/dzzgNytAIaOFrc0L4PoSLHitlSl5V1a1IYaGnOsgr7Iv0Iy
owPlVKbvyBGcV8Q16ZKn5V9bmM5tRBUjwZUvrA46a5zsHNmVMtQ6neqtss6CKoGI
fbqESG7BLC5wT9cK/EOWW/5NJypPCX3Tu4y53TWPjUJrfI/DfPWeOv93mNKuEabJ
CQIDAQAB
-----END PUBLIC KEY-----";

    // The second key pair's public key is used to verify that a token signed
    // with a different private key is rejected (public key mismatch).
    const TEST_RSA2_PUBLIC_PEM: &str = "-----BEGIN PUBLIC KEY-----
MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAmXiX6Of50UhWPdO79155
Ei+LinQ4GguJR/y7FFqNis68HcP0ef3d+RX0ieRap5HG/FRLG+3RKReQMUBVDP9L
FhLEBs/3oN+iZUnEAbsheDGGmuGqCO/W3+xQXK9jZC/q/bHwHptTwRi9/MJbyzV7
47wkpuI11oLq/Di+9Dt/W7Xlfe7aJYfRnVmXghpPvlRkrgY5RoQUxJMj5x6NwTYq
keIc62PCIfvk01JBzFY6X5yl2FgQ4URMoBG66mLlqgXnaIsECz7ARxZbvrxOIZQ5
S+E8Clz7nFHjuIFziI1HGkT8XNRyGEudRCjOCYCH/6iuFW1ViWGlxJkw7TjHp1lB
RQIDAQAB
-----END PUBLIC KEY-----";

    /// Encode a token with RS256 using the given RSA private key PEM.
    fn make_rs256_token(sub: &str, exp_offset_secs: i64, private_pem: &str) -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after epoch")
            .as_secs();
        let exp = if exp_offset_secs >= 0 {
            now + exp_offset_secs as u64
        } else {
            now.saturating_sub((-exp_offset_secs) as u64)
        };
        let claims = TestClaims { sub: Some(sub.to_owned()), exp };
        let encoding_key = EncodingKey::from_rsa_pem(private_pem.as_bytes())
            .expect("test RSA private key should be valid PEM");
        jsonwebtoken::encode(
            &Header::new(Algorithm::RS256),
            &claims,
            &encoding_key,
        )
        .expect("RS256 token encoding should not fail")
    }

    // RS256 token signed with TEST_RSA_PRIVATE_PEM is accepted when the handler
    // is configured with the corresponding TEST_RSA_PUBLIC_PEM.
    #[tokio::test]
    async fn rs256_valid_token_accepted() {
        let token = make_rs256_token("rs-user-1", 3600, TEST_RSA_PRIVATE_PEM);
        let handler = AuthHandler::new(TEST_RSA_PUBLIC_PEM.to_owned());
        let (tx, _rx) = mpsc::channel(8);
        let auth_msg = AuthMessage { token, protocol_version: None };
        let result = handler.handle_auth(&auth_msg, &tx, 60, false).await;
        assert!(result.is_ok(), "RS256 token with matching public key should be accepted, got {result:?}");
        assert_eq!(result.unwrap().id, "rs-user-1");
    }

    // RS256 token signed with TEST_RSA_PRIVATE_PEM is rejected when the handler
    // is configured with a different public key (TEST_RSA2_PUBLIC_PEM).
    #[tokio::test]
    async fn rs256_invalid_signature_rejected() {
        let token = make_rs256_token("rs-user-2", 3600, TEST_RSA_PRIVATE_PEM);
        // Use the second key pair's public key — signature will not match.
        let handler = AuthHandler::new(TEST_RSA2_PUBLIC_PEM.to_owned());
        let (tx, mut rx) = mpsc::channel(8);
        let auth_msg = AuthMessage { token, protocol_version: None };
        let result = handler.handle_auth(&auth_msg, &tx, 60, false).await;
        assert!(result.is_err(), "RS256 token with wrong public key should be rejected");
        assert!(rx.try_recv().is_ok(), "AUTH_FAIL should be sent on rejection");
    }

    // decode_jwt_key selects RS256 for PEM input.
    #[test]
    fn pem_key_detected_as_rsa() {
        let result = decode_jwt_key(TEST_RSA_PUBLIC_PEM);
        assert!(result.is_ok(), "valid PEM should parse without error");
        let (algo, _key) = result.unwrap();
        assert_eq!(algo, Algorithm::RS256, "PEM key should be detected as RS256");
    }

    // decode_jwt_key selects HS256 for plain string input.
    #[test]
    fn non_pem_key_detected_as_hmac() {
        let result = decode_jwt_key("my-secret");
        assert!(result.is_ok(), "plain secret should always succeed");
        let (algo, _key) = result.unwrap();
        assert_eq!(algo, Algorithm::HS256, "plain string should be detected as HS256");
    }

    // Default mode: AUTH_FAIL error field contains generic message for expired token.
    #[tokio::test]
    async fn opaque_mode_generic_error_on_expired_token() {
        let (handler, tx, mut rx) = setup();
        let token = make_token(Some("user-1"), -3600);
        let auth_msg = AuthMessage { token, protocol_version: None };
        let _ = handler.handle_auth(&auth_msg, &tx, 60, false).await;
        let msg = rx.try_recv().expect("AUTH_FAIL should be sent");
        let OutboundMessage::Binary(bytes) = msg else {
            panic!("expected Binary message");
        };
        let tg_msg: topgun_core::messages::Message =
            rmp_serde::from_slice(&bytes).expect("should deserialize");
        if let topgun_core::messages::Message::AuthFail(data) = tg_msg {
            let error_text = data.error.expect("error field should be Some");
            assert_eq!(error_text, "Authentication failed", "opaque mode should return generic message");
        } else {
            panic!("expected AuthFail message");
        }
    }

    // Insecure mode: AUTH_FAIL error field contains detailed message for expired token.
    #[tokio::test]
    async fn insecure_mode_detailed_error_on_expired_token() {
        let (handler, tx, mut rx) = setup();
        let token = make_token(Some("user-1"), -3600);
        let auth_msg = AuthMessage { token, protocol_version: None };
        let _ = handler.handle_auth(&auth_msg, &tx, 60, true).await;
        let msg = rx.try_recv().expect("AUTH_FAIL should be sent");
        let OutboundMessage::Binary(bytes) = msg else {
            panic!("expected Binary message");
        };
        let tg_msg: topgun_core::messages::Message =
            rmp_serde::from_slice(&bytes).expect("should deserialize");
        if let topgun_core::messages::Message::AuthFail(data) = tg_msg {
            let error_text = data.error.expect("error field should be Some");
            assert_ne!(error_text, "Authentication failed", "insecure mode should return detailed message");
        } else {
            panic!("expected AuthFail message");
        }
    }

    // Default mode: AUTH_FAIL contains generic message when sub claim is missing.
    #[tokio::test]
    async fn opaque_mode_generic_error_on_missing_sub() {
        let (handler, tx, mut rx) = setup();
        let token = make_token(None, 3600);
        let auth_msg = AuthMessage { token, protocol_version: None };
        let _ = handler.handle_auth(&auth_msg, &tx, 60, false).await;
        let msg = rx.try_recv().expect("AUTH_FAIL should be sent");
        let OutboundMessage::Binary(bytes) = msg else {
            panic!("expected Binary message");
        };
        let tg_msg: topgun_core::messages::Message =
            rmp_serde::from_slice(&bytes).expect("should deserialize");
        if let topgun_core::messages::Message::AuthFail(data) = tg_msg {
            let error_text = data.error.expect("error field should be Some");
            assert_eq!(error_text, "Authentication failed", "opaque mode should return generic message for missing sub");
        } else {
            panic!("expected AuthFail message");
        }
    }

    // normalize_pem replaces escaped backslash-n with real newlines.
    #[test]
    fn normalize_pem_replaces_escaped_newlines() {
        let escaped = "-----BEGIN PUBLIC KEY-----\\nABC\\n-----END PUBLIC KEY-----";
        let normalized = normalize_pem(escaped);
        assert!(
            normalized.contains('\n'),
            "normalized string should contain real newlines"
        );
        assert!(
            !normalized.contains("\\n"),
            "normalized string should not contain escaped newlines"
        );
    }
}
