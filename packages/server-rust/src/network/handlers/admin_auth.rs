//! Admin authentication middleware for axum handlers.
//!
//! Provides `AdminClaims`, an axum extractor that validates a JWT Bearer token
//! and verifies the caller has the `"admin"` role. Handlers that require admin
//! access simply add `AdminClaims` to their parameter list.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use jsonwebtoken::Validation;

use super::admin_types::ErrorResponse;
use super::auth_validator::AuthValidationContext;
use super::AppState;

/// Error type returned when admin authentication fails.
#[derive(Debug)]
pub enum AdminAuthError {
    /// No Authorization header or invalid Bearer format.
    MissingToken,
    /// JWT is invalid, expired, or malformed.
    InvalidToken(String),
    /// JWT is valid but lacks the `"admin"` role.
    Forbidden,
    /// JWT secret not configured on the server.
    NotConfigured,
}

impl IntoResponse for AdminAuthError {
    fn into_response(self) -> Response {
        let (status, error_msg) = match self {
            Self::MissingToken => (StatusCode::UNAUTHORIZED, "missing or invalid Bearer token"),
            Self::InvalidToken(ref _e) => (StatusCode::UNAUTHORIZED, "invalid or expired token"),
            Self::Forbidden => (StatusCode::FORBIDDEN, "admin role required"),
            Self::NotConfigured => (StatusCode::UNAUTHORIZED, "authentication not configured"),
        };

        let body = Json(ErrorResponse {
            code: status.as_u16().into(),
            message: error_msg.to_string(),
            field: None,
        });

        (status, body).into_response()
    }
}

/// Axum extractor that validates a JWT Bearer token and requires the `"admin"` role.
///
/// Usage in a handler:
/// ```ignore
/// async fn admin_endpoint(claims: AdminClaims) -> impl IntoResponse {
///     // claims.user_id and claims.roles are available
/// }
/// ```
#[derive(Debug, Clone)]
pub struct AdminClaims {
    /// The authenticated user's identifier (from JWT `sub` claim).
    pub user_id: String,
    /// Roles from the JWT.
    pub roles: Vec<String>,
}

impl FromRequestParts<AppState> for AdminClaims {
    type Rejection = AdminAuthError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        // Extract JWT secret from app state
        let jwt_secret = state.jwt_secret.as_deref().ok_or(AdminAuthError::NotConfigured)?;

        // Extract Bearer token from Authorization header
        let auth_header = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .ok_or(AdminAuthError::MissingToken)?;

        let token = auth_header
            .strip_prefix("Bearer ")
            .ok_or(AdminAuthError::MissingToken)?;

        if token.is_empty() {
            return Err(AdminAuthError::MissingToken);
        }

        // Validate JWT — do NOT clear required_spec_claims so that `exp` is
        // enforced. Use the configured clock skew tolerance for leeway.
        let (algorithm, key) = super::auth::decode_jwt_key(jwt_secret)
            .map_err(AdminAuthError::InvalidToken)?;
        let mut validation = Validation::new(algorithm);
        validation.validate_aud = false;
        validation.leeway = state.config.jwt_clock_skew_secs;

        // Decode into serde_json::Value to obtain raw_claims for AuthValidationContext.
        let token_data = jsonwebtoken::decode::<serde_json::Value>(token, &key, &validation)
            .map_err(|e| AdminAuthError::InvalidToken(e.to_string()))?;

        let raw_claims = token_data.claims.clone();

        // Reject tokens without a subject claim — anonymous identity is not
        // permitted for admin endpoints.
        let user_id = raw_claims["sub"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| AdminAuthError::InvalidToken("missing sub claim in JWT".to_string()))?;

        let roles: Vec<String> = raw_claims["roles"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();

        // Verify admin role before calling custom validator.
        if !roles.iter().any(|r| r == "admin") {
            return Err(AdminAuthError::Forbidden);
        }

        // Call custom validator after signature verification if configured.
        // Admin endpoints are internal-facing — always forward the full reason string.
        if let Some(ref validator) = state.auth_validator {
            let ctx = AuthValidationContext {
                user_id: user_id.clone(),
                roles: roles.clone(),
                raw_claims,
            };
            if let Err(reason) = validator.validate(&ctx).await {
                tracing::warn!(user_id = %user_id, reason = %reason, "custom auth validator rejected admin token");
                return Err(AdminAuthError::InvalidToken(reason));
            }
        }

        Ok(AdminClaims { user_id, roles })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::FromRequestParts;
    use axum::http::{header, Request};
    use jsonwebtoken::{EncodingKey, Header};
    use serde::Serialize;
    use std::sync::Arc;
    use std::time::{Instant, SystemTime, UNIX_EPOCH};

    use crate::network::connection::ConnectionRegistry;
    use crate::network::config::NetworkConfig;
    use crate::network::shutdown::ShutdownController;
    use crate::network::handlers::AppState;

    const TEST_SECRET: &str = "test-admin-secret";

    /// Minimal claims struct for building test admin tokens.
    #[derive(Serialize)]
    struct TestAdminClaims {
        sub: Option<String>,
        roles: Vec<String>,
        exp: u64,
    }

    /// Encode a token with given claims, using `exp_offset_secs` relative to now.
    fn make_token(sub: Option<&str>, exp_offset_secs: i64) -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after epoch")
            .as_secs();
        #[allow(clippy::cast_sign_loss)]
        let exp = if exp_offset_secs >= 0 {
            now + exp_offset_secs as u64
        } else {
            now.saturating_sub((-exp_offset_secs) as u64)
        };
        let claims = TestAdminClaims {
            sub: sub.map(str::to_owned),
            roles: vec!["admin".to_string()],
            exp,
        };
        jsonwebtoken::encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(TEST_SECRET.as_bytes()),
        )
        .expect("test token encoding should not fail")
    }

    /// Construct a minimal `AppState` for testing.
    fn test_state(leeway: u64) -> AppState {
        let config = NetworkConfig { jwt_clock_skew_secs: leeway, ..NetworkConfig::default() };
        AppState {
            registry: Arc::new(ConnectionRegistry::new()),
            shutdown: Arc::new(ShutdownController::new()),
            config: Arc::new(config),
            start_time: Instant::now(),
            observability: None,
            operation_service: None,
            dispatcher: None,
            jwt_secret: Some(TEST_SECRET.to_owned()),
            cluster_state: None,
            store_factory: None,
            server_config: None,
            policy_store: None,
            auth_providers: Arc::new(vec![]),
            refresh_grant_store: None,
            auth_validator: None,
            index_observer_factory: None,
            backfill_progress: Arc::new(dashmap::DashMap::new()),
        }
    }

    /// Build request parts with a Bearer token in the Authorization header.
    fn parts_with_bearer(token: &str) -> Parts {
        let req = Request::builder()
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(())
            .expect("request construction should not fail");
        let (parts, ()) = req.into_parts();
        parts
    }

    // AC8 + AC9: valid signature but no sub claim is rejected with HTTP 401
    #[tokio::test]
    async fn missing_sub_rejected() {
        let state = test_state(60);
        let token = make_token(None, 3600);
        let mut parts = parts_with_bearer(&token);
        let result = AdminClaims::from_request_parts(&mut parts, &state).await;
        assert!(result.is_err(), "expected Err when sub is missing");
        match result.unwrap_err() {
            AdminAuthError::InvalidToken(msg) => {
                assert!(
                    msg.contains("sub"),
                    "error should mention 'sub', got: {msg}"
                );
            }
            e => panic!("expected InvalidToken, got {e:?}"),
        }
    }

    // AC1 equivalent: expired token (1 hour ago) is rejected
    #[tokio::test]
    async fn expired_token_rejected() {
        let state = test_state(60);
        let token = make_token(Some("admin-user"), -3600);
        let mut parts = parts_with_bearer(&token);
        let result = AdminClaims::from_request_parts(&mut parts, &state).await;
        assert!(result.is_err(), "expected Err for expired token");
        assert!(
            matches!(result.unwrap_err(), AdminAuthError::InvalidToken(_)),
            "should be InvalidToken"
        );
    }

    // AC3 equivalent: token expired 30s ago is accepted when leeway is 60s
    #[tokio::test]
    async fn token_within_leeway_accepted() {
        let state = test_state(60);
        let token = make_token(Some("admin-user"), -30);
        let mut parts = parts_with_bearer(&token);
        let result = AdminClaims::from_request_parts(&mut parts, &state).await;
        assert!(
            result.is_ok(),
            "token 30s expired should be accepted within 60s leeway, got {result:?}"
        );
    }

    // AC4 equivalent: token expired 90s ago is rejected when leeway is 60s
    #[tokio::test]
    async fn token_beyond_leeway_rejected() {
        let state = test_state(60);
        let token = make_token(Some("admin-user"), -90);
        let mut parts = parts_with_bearer(&token);
        let result = AdminClaims::from_request_parts(&mut parts, &state).await;
        assert!(result.is_err(), "token 90s expired should be rejected with 60s leeway");
    }

    // Valid admin token is accepted
    #[tokio::test]
    async fn valid_admin_token_accepted() {
        let state = test_state(60);
        let token = make_token(Some("admin-user"), 3600);
        let mut parts = parts_with_bearer(&token);
        let result = AdminClaims::from_request_parts(&mut parts, &state).await;
        assert!(result.is_ok(), "valid admin token should be accepted, got {result:?}");
        let claims = result.unwrap();
        assert_eq!(claims.user_id, "admin-user");
        assert!(claims.roles.contains(&"admin".to_string()));
    }

    // NOT a real key — RSA 2048-bit test pair generated for unit tests only
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

    /// Encode an RS256 admin token with the given sub and roles.
    fn make_rs256_admin_token(sub: &str) -> String {
        use jsonwebtoken::Algorithm;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after epoch")
            .as_secs();
        let exp = now + 3600;
        let claims = TestAdminClaims {
            sub: Some(sub.to_owned()),
            roles: vec!["admin".to_string()],
            exp,
        };
        let encoding_key = EncodingKey::from_rsa_pem(TEST_RSA_PRIVATE_PEM.as_bytes())
            .expect("test RSA private key should be valid PEM");
        jsonwebtoken::encode(
            &Header::new(Algorithm::RS256),
            &claims,
            &encoding_key,
        )
        .expect("RS256 admin token encoding should not fail")
    }

    /// Build an `AppState` with the RSA public key as `jwt_secret`.
    fn test_state_rsa(leeway: u64) -> AppState {
        let config = NetworkConfig { jwt_clock_skew_secs: leeway, ..NetworkConfig::default() };
        AppState {
            registry: Arc::new(ConnectionRegistry::new()),
            shutdown: Arc::new(ShutdownController::new()),
            config: Arc::new(config),
            start_time: Instant::now(),
            observability: None,
            operation_service: None,
            dispatcher: None,
            jwt_secret: Some(TEST_RSA_PUBLIC_PEM.to_owned()),
            cluster_state: None,
            store_factory: None,
            server_config: None,
            policy_store: None,
            auth_providers: Arc::new(vec![]),
            refresh_grant_store: None,
            auth_validator: None,
            index_observer_factory: None,
            backfill_progress: Arc::new(dashmap::DashMap::new()),
        }
    }

    // RS256 admin token accepted when jwt_secret is the RSA public key PEM.
    #[tokio::test]
    async fn rs256_admin_token_accepted() {
        let state = test_state_rsa(60);
        let token = make_rs256_admin_token("admin-rsa-user");
        let mut parts = parts_with_bearer(&token);
        let result = AdminClaims::from_request_parts(&mut parts, &state).await;
        assert!(
            result.is_ok(),
            "RS256 admin token with matching RSA public key should be accepted, got {result:?}"
        );
        let claims = result.unwrap();
        assert_eq!(claims.user_id, "admin-rsa-user");
        assert!(claims.roles.contains(&"admin".to_string()));
    }

    // -----------------------------------------------------------------------
    // AuthValidator integration tests (SPEC-189 AC3, AC4)
    // -----------------------------------------------------------------------

    /// Build a minimal `AppState` for testing with an optional validator.
    fn test_state_with_validator(validator: Option<Arc<dyn crate::network::handlers::auth_validator::AuthValidator>>) -> AppState {
        let config = NetworkConfig { jwt_clock_skew_secs: 60, ..NetworkConfig::default() };
        AppState {
            registry: Arc::new(ConnectionRegistry::new()),
            shutdown: Arc::new(ShutdownController::new()),
            config: Arc::new(config),
            start_time: Instant::now(),
            observability: None,
            operation_service: None,
            dispatcher: None,
            jwt_secret: Some(TEST_SECRET.to_owned()),
            cluster_state: None,
            store_factory: None,
            server_config: None,
            policy_store: None,
            auth_providers: Arc::new(vec![]),
            refresh_grant_store: None,
            auth_validator: validator,
            index_observer_factory: None,
            backfill_progress: Arc::new(dashmap::DashMap::new()),
        }
    }

    /// AC3 (SPEC-189): A rejecting `AuthValidator` causes `AdminClaims` extractor to return `Err(InvalidToken)`.
    #[tokio::test]
    async fn rejecting_validator_returns_invalid_token() {
        let validator = Arc::new(|_ctx: &crate::network::handlers::auth_validator::AuthValidationContext| {
            Err("ip not allowlisted".to_string())
        });
        let state = test_state_with_validator(Some(validator));
        let token = make_token(Some("admin-user"), 3600);
        let mut parts = parts_with_bearer(&token);
        let result = AdminClaims::from_request_parts(&mut parts, &state).await;
        assert!(result.is_err(), "expected Err when validator rejects");
        match result.unwrap_err() {
            AdminAuthError::InvalidToken(reason) => {
                assert_eq!(reason, "ip not allowlisted", "admin path should forward full reason");
            }
            e => panic!("expected InvalidToken, got {e:?}"),
        }
    }

    /// AC4 (SPEC-189): When `auth_validator` is `None`, valid admin token is accepted (no regression).
    #[tokio::test]
    async fn no_validator_accepts_valid_admin_token() {
        let state = test_state_with_validator(None);
        let token = make_token(Some("admin-user"), 3600);
        let mut parts = parts_with_bearer(&token);
        let result = AdminClaims::from_request_parts(&mut parts, &state).await;
        assert!(result.is_ok(), "no validator should accept valid admin token, got {result:?}");
        assert_eq!(result.unwrap().user_id, "admin-user");
    }
}
