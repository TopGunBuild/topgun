//! Token exchange handler for POST /api/auth/token.
//!
//! Accepts an external provider's JWT, verifies it via the configured
//! [`AuthProvider`] implementations, and returns a signed `TopGun` JWT.
//! This eliminates the need for a custom bridge server when integrating
//! external auth providers (Clerk, Auth0, Firebase, etc.).

use std::time::SystemTime;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use jsonwebtoken::{EncodingKey, Header};
use serde::{Deserialize, Serialize};

use super::admin_types::ErrorResponse;
use super::AppState;

// ── Request / Response types ──────────────────────────────────────────────────

/// Request body for POST /api/auth/token.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenExchangeRequest {
    /// The external provider's JWT to verify.
    pub token: String,
    /// Optional provider name. If omitted, all configured providers are tried in order.
    #[serde(default)]
    pub provider: Option<String>,
}

/// Response body for a successful POST /api/auth/token exchange.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenExchangeResponse {
    /// The issued `TopGun` JWT.
    pub token: String,
    /// Token expiry as seconds since Unix epoch.
    pub expires_at: u64,
}

// ── Internal JWT claims struct ────────────────────────────────────────────────

/// JWT claims for token exchange signing.
///
/// Mirrors `AdminJwtClaims` but includes `iat` and is local to this handler.
#[derive(Serialize)]
struct ExchangeJwtClaims {
    sub: String,
    roles: Vec<String>,
    exp: u64,
    iat: u64,
}

// ── Handler ───────────────────────────────────────────────────────────────────

/// Exchange an external provider token for a `TopGun` JWT.
///
/// If `provider` is specified, only that provider is tried. If omitted, every
/// configured provider is tried in order and the first successful verification
/// wins. Returns 404 when no providers are configured.
///
/// # Errors
///
/// Returns an error tuple of `(StatusCode, Json<ErrorResponse>)` when:
/// - No auth providers are configured (404)
/// - JWT secret is missing (500)
/// - Named provider not found (400)
/// - All providers fail verification (401)
/// - Token signing fails (500)
#[allow(clippy::too_many_lines)]
pub async fn token_exchange_handler(
    State(state): State<AppState>,
    Json(req): Json<TokenExchangeRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<ErrorResponse>)> {
    // Return 404 when no providers are configured — token exchange is disabled.
    if state.auth_providers.is_empty() {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                code: 404,
                message: "token exchange not configured".to_string(),
                field: None,
            }),
        ));
    }

    let jwt_secret = state.jwt_secret.as_deref().ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                code: 500,
                message: "token signing not configured".to_string(),
                field: None,
            }),
        )
    })?;

    // Resolve which providers to attempt.
    let providers: Vec<_> = if let Some(ref name) = req.provider {
        let found: Vec<_> = state
            .auth_providers
            .iter()
            .filter(|p| p.name() == name.as_str())
            .cloned()
            .collect();

        if found.is_empty() {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    code: 400,
                    message: format!("unknown provider '{name}'"),
                    field: Some("provider".to_string()),
                }),
            ));
        }
        found
    } else {
        state.auth_providers.iter().cloned().collect()
    };

    // Try each provider in order — use the first that succeeds.
    let mut last_error = String::new();
    for provider in &providers {
        match provider.verify(&req.token).await {
            Ok(external_claims) => {
                // Sign a TopGun JWT valid for 1 hour.
                let now = SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let exp = now + 3600;

                let claims = ExchangeJwtClaims {
                    sub: external_claims.sub,
                    roles: external_claims.roles,
                    exp,
                    iat: now,
                };

                let token = jsonwebtoken::encode(
                    &Header::default(), // HS256
                    &claims,
                    &EncodingKey::from_secret(jwt_secret.as_bytes()),
                )
                .map_err(|e| {
                    let message = if state.config.insecure_forward_auth_errors {
                        format!("token signing failed: {e}")
                    } else {
                        "Internal server error".to_string()
                    };
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse {
                            code: 500,
                            message,
                            field: None,
                        }),
                    )
                })?;

                return Ok(Json(TokenExchangeResponse {
                    token,
                    expires_at: exp,
                }));
            }
            Err(e) => {
                last_error = e;
            }
        }
    }

    // No provider succeeded. Detailed reason logged above by the provider; only
    // forward to client when insecure mode is explicitly enabled.
    let auth_fail_message = if state.config.insecure_forward_auth_errors {
        format!("token verification failed: {last_error}")
    } else {
        "Authentication failed".to_string()
    };
    Err((
        StatusCode::UNAUTHORIZED,
        Json(ErrorResponse {
            code: 401,
            message: auth_fail_message,
            field: None,
        }),
    ))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Instant;

    use async_trait::async_trait;
    use axum::body::Body;
    use axum::http::{self, Request, StatusCode};
    use axum::routing::post;
    use axum::Router;
    use serde_json::{json, Value};
    use tower::ServiceExt;

    use super::*;
    use crate::network::handlers::auth_provider::{AuthProvider, ExternalClaims};
    use crate::network::handlers::{AppState};
    use crate::network::{ConnectionRegistry, NetworkConfig, ShutdownController};

    // ── Stub providers ────────────────────────────────────────────────────────

    struct AlwaysSucceed {
        name: String,
        sub: String,
        roles: Vec<String>,
    }

    #[async_trait]
    impl AuthProvider for AlwaysSucceed {
        fn name(&self) -> &str {
            &self.name
        }
        async fn verify(&self, _token: &str) -> Result<ExternalClaims, String> {
            Ok(ExternalClaims {
                sub: self.sub.clone(),
                roles: self.roles.clone(),
            })
        }
    }

    struct AlwaysFail {
        name: String,
    }

    #[async_trait]
    impl AuthProvider for AlwaysFail {
        fn name(&self) -> &str {
            &self.name
        }
        async fn verify(&self, _token: &str) -> Result<ExternalClaims, String> {
            Err("always fails".to_string())
        }
    }

    // ── Test helpers ──────────────────────────────────────────────────────────

    fn make_state(providers: Vec<Arc<dyn AuthProvider>>, jwt_secret: Option<&str>) -> AppState {
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
            auth_providers: Arc::new(providers),
        }
    }

    fn make_app(state: AppState) -> Router {
        Router::new()
            .route("/api/auth/token", post(token_exchange_handler))
            .with_state(state)
    }

    async fn post_exchange(app: Router, body: Value) -> (StatusCode, Value) {
        let resp = app
            .oneshot(
                Request::builder()
                    .method(http::Method::POST)
                    .uri("/api/auth/token")
                    .header(http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: Value = serde_json::from_slice(&bytes).unwrap_or(json!({}));
        (status, json)
    }

    // ── Tests ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn no_providers_returns_404() {
        let app = make_app(make_state(vec![], Some("secret")));
        let (status, body) = post_exchange(app, json!({ "token": "any" })).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["code"], 404);
    }

    #[tokio::test]
    async fn no_jwt_secret_returns_500() {
        let provider: Arc<dyn AuthProvider> = Arc::new(AlwaysSucceed {
            name: "test".to_string(),
            sub: "u".to_string(),
            roles: vec![],
        });
        let app = make_app(make_state(vec![provider], None));
        let (status, body) = post_exchange(app, json!({ "token": "any" })).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["code"], 500);
    }

    #[tokio::test]
    async fn valid_token_returns_200_with_jwt() {
        let provider: Arc<dyn AuthProvider> = Arc::new(AlwaysSucceed {
            name: "hmac".to_string(),
            sub: "user-1".to_string(),
            roles: vec!["admin".to_string()],
        });
        let app = make_app(make_state(vec![provider], Some("signing-secret")));
        let (status, body) = post_exchange(app, json!({ "token": "any" })).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body["token"].as_str().is_some());
        assert!(body["expiresAt"].as_u64().is_some());
    }

    #[tokio::test]
    async fn all_providers_fail_returns_401() {
        let p1: Arc<dyn AuthProvider> = Arc::new(AlwaysFail { name: "p1".to_string() });
        let p2: Arc<dyn AuthProvider> = Arc::new(AlwaysFail { name: "p2".to_string() });
        let app = make_app(make_state(vec![p1, p2], Some("secret")));
        let (status, body) = post_exchange(app, json!({ "token": "bad" })).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["code"], 401);
    }

    #[tokio::test]
    async fn unknown_provider_name_returns_400() {
        let provider: Arc<dyn AuthProvider> = Arc::new(AlwaysSucceed {
            name: "clerk".to_string(),
            sub: "u".to_string(),
            roles: vec![],
        });
        let app = make_app(make_state(vec![provider], Some("secret")));
        let (status, body) =
            post_exchange(app, json!({ "token": "any", "provider": "nonexistent" })).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], 400);
    }

    #[tokio::test]
    async fn named_provider_is_used() {
        let succeed: Arc<dyn AuthProvider> = Arc::new(AlwaysSucceed {
            name: "good".to_string(),
            sub: "alice".to_string(),
            roles: vec![],
        });
        let fail: Arc<dyn AuthProvider> = Arc::new(AlwaysFail { name: "bad".to_string() });
        let app = make_app(make_state(vec![succeed, fail], Some("secret")));
        let (status, _) =
            post_exchange(app, json!({ "token": "any", "provider": "good" })).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn first_succeeding_provider_is_used_when_no_provider_specified() {
        let fail: Arc<dyn AuthProvider> = Arc::new(AlwaysFail { name: "first".to_string() });
        let succeed: Arc<dyn AuthProvider> = Arc::new(AlwaysSucceed {
            name: "second".to_string(),
            sub: "bob".to_string(),
            roles: vec!["viewer".to_string()],
        });
        let app = make_app(make_state(vec![fail, succeed], Some("secret")));
        let (status, body) = post_exchange(app, json!({ "token": "any" })).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body["token"].as_str().is_some());
    }
}
