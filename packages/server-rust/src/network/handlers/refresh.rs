//! POST /api/auth/refresh handler -- single-use refresh token rotation.
//!
//! Issues a new access JWT and refresh token when presented with a valid
//! single-use refresh grant. The old grant is atomically consumed (deleted)
//! and a new grant is created so that the refresh token itself rotates on
//! every use, preventing replay attacks.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use jsonwebtoken::{EncodingKey, Header};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::admin_types::ErrorResponse;
use super::{AppState, RefreshGrant, RefreshGrantStore};

/// Request body for POST /api/auth/refresh.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshRequest {
    /// The opaque refresh token issued by a previous token exchange or refresh.
    pub refresh_token: String,
}

/// Response body for a successful POST /api/auth/refresh.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshResponse {
    /// Newly issued access JWT.
    pub token: String,
    /// Access token expiry as seconds since Unix epoch.
    pub expires_at: u64,
    /// Newly issued refresh token (rotated -- old token is now invalid).
    pub refresh_token: String,
    /// Refresh token expiry as seconds since Unix epoch.
    pub refresh_expires_at: u64,
}

/// JWT claims for the access token issued by the refresh endpoint.
#[derive(serde::Serialize)]
struct AccessJwtClaims {
    sub: String,
    roles: Vec<String>,
    exp: u64,
    iat: u64,
}

/// Generate a new refresh grant (rotation) and sign a new access JWT.
///
/// Returns `(access_token, access_expires_at, raw_refresh_token, refresh_expires_at)`
/// on success, or an error message on failure.
async fn rotate_grant_and_sign(
    store: &dyn RefreshGrantStore,
    grant: &RefreshGrant,
    jwt_secret: &str,
) -> Result<(String, u64, String, u64), String> {
    use std::time::SystemTime;

    // Generate a new refresh token: 32 random bytes, hex-encoded (64 chars).
    let mut new_token_bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut new_token_bytes);
    let new_refresh_token = hex::encode(new_token_bytes);

    // Hash the new refresh token before storage.
    let mut hasher = Sha256::new();
    hasher.update(new_refresh_token.as_bytes());
    let new_token_hash = hex::encode(hasher.finalize());

    // Compute timestamps.
    let now_secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let refresh_expires_at = now_secs + store.grant_duration_secs();

    // Create the rotated grant in the store.
    let new_grant = RefreshGrant {
        id: uuid::Uuid::new_v4().to_string(),
        sub: grant.sub.clone(),
        roles: grant.roles.clone(),
        token_hash: new_token_hash,
        created_at: now_secs,
        expires_at: refresh_expires_at,
    };
    store
        .insert_grant(&new_grant)
        .await
        .map_err(|e| format!("Failed to insert rotated grant: {e}"))?;

    // Sign a new access JWT (1-hour expiry, HS256).
    let access_expires_at = now_secs + 3600;
    let claims = AccessJwtClaims {
        sub: grant.sub.clone(),
        roles: grant.roles.clone(),
        exp: access_expires_at,
        iat: now_secs,
    };
    let encoding_key = EncodingKey::from_secret(jwt_secret.as_bytes());
    let access_token = jsonwebtoken::encode(&Header::default(), &claims, &encoding_key)
        .map_err(|e| format!("JWT signing failed: {e}"))?;

    Ok((access_token, access_expires_at, new_refresh_token, refresh_expires_at))
}

/// POST /api/auth/refresh handler.
///
/// Validates the presented refresh token against the grant store, rotates the
/// grant (atomic DELETE RETURNING), and issues a new access+refresh token pair.
pub async fn refresh_handler(
    State(state): State<AppState>,
    Json(body): Json<RefreshRequest>,
) -> impl IntoResponse {
    // Return 404 when refresh grants are not configured (refresh disabled).
    let Some(store) = state.refresh_grant_store.as_ref() else {
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse { code: 404, message: "Not found".to_string(), field: None }),
        )
            .into_response();
    };

    // JWT secret must be present to sign a new access token.
    let Some(jwt_secret) = state.jwt_secret.as_deref() else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse { code: 500, message: "Server misconfiguration".to_string(), field: None }),
        )
            .into_response();
    };

    // Hash the incoming refresh token with SHA-256.
    let mut hasher = Sha256::new();
    hasher.update(body.refresh_token.as_bytes());
    let token_hash = hex::encode(hasher.finalize());

    // Atomically consume the grant. DELETE ... RETURNING eliminates TOCTOU races:
    // concurrent requests with the same token hash race at the DB level and only
    // one DELETE returns a row.
    let grant = match store.consume_grant(&token_hash).await {
        Ok(Some(g)) => g,
        Ok(None) => {
            let msg = if state.config.insecure_forward_auth_errors {
                "Refresh grant not found or expired".to_string()
            } else {
                "Authentication failed".to_string()
            };
            return (StatusCode::UNAUTHORIZED, Json(ErrorResponse { code: 401, message: msg, field: None }))
                .into_response();
        }
        Err(e) => {
            let msg = if state.config.insecure_forward_auth_errors {
                format!("Grant store error: {e}")
            } else {
                "Authentication failed".to_string()
            };
            return (StatusCode::UNAUTHORIZED, Json(ErrorResponse { code: 401, message: msg, field: None }))
                .into_response();
        }
    };

    // Rotate the grant and sign a new access JWT.
    match rotate_grant_and_sign(store.as_ref(), &grant, jwt_secret).await {
        Ok((access_token, access_expires_at, new_refresh_token, refresh_expires_at)) => {
            Json(RefreshResponse {
                token: access_token,
                expires_at: access_expires_at,
                refresh_token: new_refresh_token,
                refresh_expires_at,
            })
            .into_response()
        }
        Err(detail) => {
            let msg = if state.config.insecure_forward_auth_errors { detail } else { "Authentication failed".to_string() };
            (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse { code: 500, message: msg, field: None }))
                .into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::Instant;

    use axum::http::{self, Request, StatusCode};
    use axum::routing::post;
    use axum::Router;
    use serde_json::{json, Value};
    use tower::ServiceExt;

    use super::*;
    use crate::network::handlers::AppState;
    use crate::network::{ConnectionRegistry, NetworkConfig, ShutdownController};

    // ── In-memory test store ─────────────────────────────────────────────────

    #[derive(Default)]
    struct InMemoryGrantStore {
        grants: Mutex<HashMap<String, RefreshGrant>>,
        duration: u64,
    }

    impl InMemoryGrantStore {
        fn new(duration: u64) -> Self {
            Self { grants: Mutex::new(HashMap::new()), duration }
        }
    }

    #[async_trait::async_trait]
    impl RefreshGrantStore for InMemoryGrantStore {
        fn grant_duration_secs(&self) -> u64 {
            self.duration
        }

        async fn insert_grant(&self, grant: &RefreshGrant) -> anyhow::Result<()> {
            self.grants.lock().unwrap().insert(grant.token_hash.clone(), grant.clone());
            Ok(())
        }

        async fn consume_grant(&self, token_hash: &str) -> anyhow::Result<Option<RefreshGrant>> {
            use std::time::SystemTime;
            let now = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let mut map = self.grants.lock().unwrap();
            if let Some(g) = map.get(token_hash) {
                if g.expires_at > now {
                    let grant = g.clone();
                    map.remove(token_hash);
                    return Ok(Some(grant));
                }
            }
            Ok(None)
        }

        async fn delete_expired_grants(&self) -> anyhow::Result<u64> {
            use std::time::SystemTime;
            let now = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let mut map = self.grants.lock().unwrap();
            let before = map.len();
            map.retain(|_, g| g.expires_at > now);
            Ok((before - map.len()) as u64)
        }
    }

    // ── Helpers ──────────────────────────────────────────────────────────────

    use std::sync::Arc;

    fn make_state_with_store(
        store: Arc<dyn RefreshGrantStore>,
        jwt_secret: Option<&str>,
    ) -> AppState {
        AppState {
            registry: Arc::new(ConnectionRegistry::new()),
            shutdown: Arc::new(ShutdownController::new()),
            config: Arc::new(NetworkConfig::default()),
            start_time: Instant::now(),
            observability: None,
            operation_service: None,
            dispatcher: None,
            jwt_secret: jwt_secret.map(|s| s.to_string()),
            cluster_state: None,
            store_factory: None,
            server_config: None,
            policy_store: None,
            auth_providers: Arc::new(vec![]),
            refresh_grant_store: Some(store),
        }
    }

    fn make_state_no_store(jwt_secret: Option<&str>) -> AppState {
        AppState {
            registry: Arc::new(ConnectionRegistry::new()),
            shutdown: Arc::new(ShutdownController::new()),
            config: Arc::new(NetworkConfig::default()),
            start_time: Instant::now(),
            observability: None,
            operation_service: None,
            dispatcher: None,
            jwt_secret: jwt_secret.map(|s| s.to_string()),
            cluster_state: None,
            store_factory: None,
            server_config: None,
            policy_store: None,
            auth_providers: Arc::new(vec![]),
            refresh_grant_store: None,
        }
    }

    fn make_app(state: AppState) -> Router {
        Router::new()
            .route("/api/auth/refresh", post(refresh_handler))
            .with_state(state)
    }

    async fn post_refresh(app: Router, body: Value) -> (StatusCode, Value) {
        let resp = app
            .oneshot(
                Request::builder()
                    .method(http::Method::POST)
                    .uri("/api/auth/refresh")
                    .header(http::header::CONTENT_TYPE, "application/json")
                    .body(axum::body::Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, json)
    }

    // Insert a pre-hashed grant directly into the store for testing.
    async fn seed_grant(
        store: &InMemoryGrantStore,
        raw_token: &str,
        sub: &str,
        roles: Vec<String>,
        expires_at: u64,
    ) {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(raw_token.as_bytes());
        let hash = hex::encode(hasher.finalize());
        let grant = RefreshGrant {
            id: uuid::Uuid::new_v4().to_string(),
            sub: sub.to_string(),
            roles,
            token_hash: hash,
            created_at: 0,
            expires_at,
        };
        store.insert_grant(&grant).await.unwrap();
    }

    // ── Tests ─────────────────────────────────────────────────────────────────

    /// AC6: refresh endpoint returns 404 when no grant store is configured.
    #[tokio::test]
    async fn refresh_returns_404_when_disabled() {
        let state = make_state_no_store(Some("secret"));
        let app = make_app(state);
        let (status, _) = post_refresh(app, json!({"refreshToken": "anything"})).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    /// AC3: valid refresh token returns 200 with new access and refresh tokens.
    #[tokio::test]
    async fn refresh_with_valid_token_returns_200() {
        use std::time::SystemTime;
        let store = Arc::new(InMemoryGrantStore::new(2_592_000));
        let raw_token = "validtoken123";
        let future_expiry = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 86400;
        seed_grant(&store, raw_token, "user-1", vec!["viewer".to_string()], future_expiry).await;

        let state = make_state_with_store(Arc::clone(&store) as Arc<dyn RefreshGrantStore>, Some("test-secret"));
        let app = make_app(state);
        let (status, body) = post_refresh(app, json!({"refreshToken": raw_token})).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body["token"].is_string(), "response must include access token");
        assert!(body["expiresAt"].is_number(), "response must include expiresAt");
        assert!(body["refreshToken"].is_string(), "response must include new refresh token");
        assert!(body["refreshExpiresAt"].is_number(), "response must include refreshExpiresAt");
        // New refresh token must differ from the original.
        assert_ne!(body["refreshToken"].as_str().unwrap(), raw_token);
    }

    /// AC4: using the same refresh token a second time returns 401 (single-use).
    #[tokio::test]
    async fn refresh_token_is_single_use() {
        use std::time::SystemTime;
        let store = Arc::new(InMemoryGrantStore::new(2_592_000));
        let raw_token = "single-use-token";
        let future_expiry = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 86400;
        seed_grant(&store, raw_token, "user-2", vec![], future_expiry).await;

        let state1 = make_state_with_store(Arc::clone(&store) as Arc<dyn RefreshGrantStore>, Some("test-secret"));
        let (status1, _) = post_refresh(make_app(state1), json!({"refreshToken": raw_token})).await;
        assert_eq!(status1, StatusCode::OK, "first use must succeed");

        let state2 = make_state_with_store(Arc::clone(&store) as Arc<dyn RefreshGrantStore>, Some("test-secret"));
        let (status2, _) = post_refresh(make_app(state2), json!({"refreshToken": raw_token})).await;
        assert_eq!(status2, StatusCode::UNAUTHORIZED, "second use must be rejected");
    }

    /// AC5: expired refresh token returns 401.
    #[tokio::test]
    async fn expired_refresh_token_returns_401() {
        let store = Arc::new(InMemoryGrantStore::new(2_592_000));
        let raw_token = "expired-token";
        // expires_at = 1 (far in the past)
        seed_grant(&store, raw_token, "user-3", vec![], 1).await;

        let state = make_state_with_store(Arc::clone(&store) as Arc<dyn RefreshGrantStore>, Some("test-secret"));
        let app = make_app(state);
        let (status, _) = post_refresh(app, json!({"refreshToken": raw_token})).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
}
