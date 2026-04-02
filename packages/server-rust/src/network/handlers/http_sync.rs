//! HTTP sync endpoint handler for `MsgPack`-encoded request/response.
//!
//! Provides a POST /sync endpoint that accepts `MsgPack` bodies and returns
//! `MsgPack` responses. This is the HTTP fallback transport for clients that
//! cannot maintain a WebSocket connection (e.g., behind restrictive proxies).

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::State;
use axum::extract::FromRequestParts;
use axum::extract::OptionalFromRequestParts;
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use bytes::Bytes;
use jsonwebtoken::Validation;
use topgun_core::hash_to_partition;
use topgun_core::messages::{HttpSyncAck, HttpSyncError, HttpSyncRequest, HttpSyncResponse};
use topgun_core::Timestamp;

use super::AppState;
use super::auth::JwtClaims;
use crate::service::dispatch::PartitionDispatcher;
use crate::service::operation::CallerOrigin;

// ---------------------------------------------------------------------------
// ClientAuthError
// ---------------------------------------------------------------------------

/// Errors that can occur when extracting `ClientClaims` from a request.
#[derive(Debug)]
pub enum ClientAuthError {
    /// No Authorization header present.
    MissingToken,
    /// JWT is present but invalid, expired, or malformed.
    InvalidToken(String),
    /// JWT secret not configured on the server.
    NotConfigured,
}

impl IntoResponse for ClientAuthError {
    fn into_response(self) -> axum::response::Response {
        // The handler uses Option<ClientClaims>, so axum converts Err to None
        // and this impl is only invoked if ClientClaims is used directly (non-optional).
        // For completeness, return 401 for all error variants.
        let json = match &self {
            Self::MissingToken => r#"{"code":401,"message":"authentication required"}"#.to_string(),
            Self::InvalidToken(_) => r#"{"code":401,"message":"invalid or expired token"}"#.to_string(),
            Self::NotConfigured => r#"{"code":401,"message":"authentication not configured"}"#.to_string(),
        };
        (
            StatusCode::UNAUTHORIZED,
            [("content-type", "application/json")],
            json.into_bytes(),
        )
            .into_response()
    }
}

// ---------------------------------------------------------------------------
// TokenPresence
// ---------------------------------------------------------------------------

/// Records whether an `Authorization` header was present in the request,
/// regardless of whether the token was valid.
///
/// Injected into request extensions by the `ClientClaims` extractor so the
/// handler can distinguish "no token" from "bad token" when using
/// `Option<ClientClaims>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenPresence {
    Present,
    Absent,
}

// ---------------------------------------------------------------------------
// ClientClaims
// ---------------------------------------------------------------------------

/// Axum extractor for client Bearer JWT authentication.
///
/// Unlike `AdminClaims`, this extractor:
/// - Does NOT require an `"admin"` role — any valid JWT with a `sub` claim is accepted.
/// - Is intended for use as `Option<ClientClaims>` in the handler signature so
///   missing tokens return `None` rather than rejecting the request. The handler
///   decides rejection based on `require_auth`.
///
/// When the `Authorization` header is present (valid or invalid), the extractor
/// sets `TokenPresence::Present` in request extensions. When absent, it sets
/// `TokenPresence::Absent`. The handler reads `Extension<TokenPresence>` to
/// distinguish "no token" from "bad token".
#[derive(Debug, Clone)]
pub struct ClientClaims {
    /// The authenticated user's identifier (from JWT `sub` claim).
    pub user_id: String,
    /// Roles from the JWT.
    pub roles: Vec<String>,
}

impl OptionalFromRequestParts<AppState> for ClientClaims {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Option<Self>, Self::Rejection> {
        // Extract Bearer token from Authorization header.
        let auth_header = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok());

        let Some(token_str) = auth_header else {
            // No Authorization header at all.
            parts.extensions.insert(TokenPresence::Absent);
            return Ok(None);
        };

        // Header present — record presence regardless of token validity.
        parts.extensions.insert(TokenPresence::Present);

        let token = token_str
            .strip_prefix("Bearer ")
            .unwrap_or("")
            .trim();

        if token.is_empty() {
            return Ok(None);
        }

        // JWT secret must be configured. If not configured, treat as no token.
        let Some(jwt_secret) = state.jwt_secret.as_deref() else {
            return Ok(None);
        };

        // Validate JWT — supports both HS256 and RS256 via decode_jwt_key.
        let Ok((algorithm, key)) = super::auth::decode_jwt_key(jwt_secret) else {
            return Ok(None);
        };
        let mut validation = Validation::new(algorithm);
        validation.validate_aud = false;
        validation.leeway = state.config.jwt_clock_skew_secs;

        let Ok(token_data) = jsonwebtoken::decode::<JwtClaims>(token, &key, &validation) else {
            return Ok(None);
        };

        // Require sub claim — anonymous identity is not permitted.
        let Some(user_id) = token_data.claims.sub else {
            return Ok(None);
        };

        let roles = token_data.claims.roles.unwrap_or_default();

        Ok(Some(ClientClaims { user_id, roles }))
    }
}

// Keep FromRequestParts for direct (non-optional) use of ClientClaims.
impl FromRequestParts<AppState> for ClientClaims {
    type Rejection = ClientAuthError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        <ClientClaims as OptionalFromRequestParts<AppState>>::from_request_parts(parts, state)
            .await
            .unwrap() // Infallible
            .ok_or(ClientAuthError::MissingToken)
    }
}

/// Content-Type header value for `MsgPack` responses.
const MSGPACK_CONTENT_TYPE: &str = "application/msgpack";

/// Returns the current wall-clock time as a `Timestamp` when the HLC is unavailable.
fn wall_clock_timestamp() -> Timestamp {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(0))
        .unwrap_or(0);
    Timestamp {
        millis,
        counter: 0,
        node_id: String::new(),
    }
}

/// Checks auth requirements and returns an error response if authentication fails.
/// Returns `None` when the request is authorized to proceed.
fn enforce_auth(
    require_auth: bool,
    token_presence: TokenPresence,
    claims: Option<&ClientClaims>,
) -> Option<axum::response::Response> {
    if !require_auth {
        return None;
    }
    match token_presence {
        TokenPresence::Absent => {
            let json = r#"{"code":401,"message":"authentication required"}"#.to_string();
            Some(
                (StatusCode::UNAUTHORIZED, [("content-type", "application/json")], json.into_bytes())
                    .into_response(),
            )
        }
        TokenPresence::Present if claims.is_none() => {
            let json = r#"{"code":401,"message":"invalid or expired token"}"#.to_string();
            Some(
                (StatusCode::UNAUTHORIZED, [("content-type", "application/json")], json.into_bytes())
                    .into_response(),
            )
        }
        TokenPresence::Present => None,
    }
}

/// Dispatches operations through the partition pipeline and populates the response.
async fn dispatch_operations(
    ops: Vec<topgun_core::messages::ClientOp>,
    classify_svc: &crate::service::classify::OperationService,
    dispatcher: &Arc<PartitionDispatcher>,
    claims: Option<&ClientClaims>,
    caller_origin: CallerOrigin,
    principal: Option<&topgun_core::Principal>,
    response: &mut HttpSyncResponse,
) {
    if ops.is_empty() {
        return;
    }

    let last_id = ops
        .last()
        .and_then(|op| op.id.clone())
        .unwrap_or_else(|| "unknown".to_string());

    // Group ops by partition so each group targets one partition worker.
    let mut partition_groups: HashMap<u32, Vec<topgun_core::messages::ClientOp>> = HashMap::new();
    for op in ops {
        let partition_id = hash_to_partition(&op.key);
        partition_groups.entry(partition_id).or_default().push(op);
    }

    // Build sub-batch operations up front, then dispatch concurrently.
    let mut sub_ops: Vec<crate::service::operation::Operation> =
        Vec::with_capacity(partition_groups.len());
    for (partition_id, group_ops) in partition_groups {
        let mut op = classify_svc.classify_op_batch_for_partition(
            group_ops,
            partition_id,
            claims.map(|c| c.user_id.clone()),
            caller_origin,
            None,
            None,
        );
        // Set principal on context for RBAC authorization middleware (HTTP path).
        if let Some(p) = principal {
            op.set_principal(p.clone());
        }
        sub_ops.push(op);
    }

    // Dispatch all sub-batches concurrently.
    let mut join_set = tokio::task::JoinSet::new();
    for sub_op in sub_ops {
        let d = Arc::clone(dispatcher);
        join_set.spawn(async move { d.dispatch(sub_op).await });
    }

    // Collect results; record any dispatch errors.
    let mut dispatch_error: Option<String> = None;
    while let Some(result) = join_set.join_next().await {
        match result {
            Ok(Ok(_resp)) => {}
            Ok(Err(e)) => {
                dispatch_error = Some(format!("{e}"));
            }
            Err(join_err) => {
                dispatch_error = Some(format!("join error: {join_err}"));
            }
        }
    }

    if let Some(msg) = dispatch_error {
        let errors = response.errors.get_or_insert_with(Vec::new);
        errors.push(HttpSyncError {
            code: 500,
            message: msg,
            context: None,
        });
    } else {
        response.ack = Some(HttpSyncAck {
            last_id,
            results: None,
        });
    }
}

/// Handles POST /sync requests with `MsgPack`-encoded bodies.
///
/// Decodes the request body as `HttpSyncRequest`, dispatches operations through
/// the `OperationService`/`PartitionDispatcher` pipeline, and returns an
/// `HttpSyncResponse` with the server's current HLC timestamp and an ack for
/// any submitted operations.
///
/// Returns HTTP 401 with a JSON error body when authentication is required but
/// no valid Bearer JWT is provided.
///
/// Returns HTTP 400 with a JSON error body when the request body is not valid
/// `MsgPack` or cannot be decoded as `HttpSyncRequest`.
///
/// When `operation_service` or `dispatcher` is absent (test environments without
/// service wiring), returns a minimal response with only `server_hlc` populated.
pub async fn http_sync_handler(
    State(state): State<AppState>,
    claims: Option<ClientClaims>,
    axum::Extension(token_presence): axum::Extension<TokenPresence>,
    body: Bytes,
) -> impl IntoResponse {
    // Read require_auth from server_config. Defaults to false when not configured.
    let require_auth = state
        .server_config
        .as_ref()
        .is_some_and(|sc| sc.load().security.require_auth);

    // Authentication enforcement: reject unauthenticated requests when required.
    if let Some(err_response) = enforce_auth(require_auth, token_presence, claims.as_ref()) {
        return err_response;
    }

    // Determine caller_origin and principal based on claims.
    let (caller_origin, principal) = if let Some(ref c) = claims {
        (
            CallerOrigin::HttpClient,
            Some(topgun_core::Principal {
                id: c.user_id.clone(),
                roles: c.roles.clone(),
            }),
        )
    } else {
        (CallerOrigin::System, None)
    };

    // Obtain the server's current HLC timestamp.
    let server_hlc: Timestamp = state
        .operation_service
        .as_ref()
        .map_or_else(wall_clock_timestamp, |s| s.now());

    // Decode the request body.
    let request: HttpSyncRequest = if body.is_empty() {
        HttpSyncRequest::default()
    } else {
        match rmp_serde::from_slice::<HttpSyncRequest>(&body) {
            Ok(req) => req,
            Err(e) => {
                let json = format!(r#"{{"code":400,"message":"invalid MsgPack body: {e}"}}"#);
                return (
                    StatusCode::BAD_REQUEST,
                    [("content-type", "application/json")],
                    json.into_bytes(),
                )
                    .into_response();
            }
        }
    };

    // When neither service is wired, return a minimal response with only server_hlc.
    let (Some(classify_svc), Some(dispatcher)) =
        (state.operation_service.as_ref(), state.dispatcher.as_ref())
    else {
        return msgpack_response(&HttpSyncResponse { server_hlc, ..Default::default() });
    };

    let mut http_response = HttpSyncResponse {
        server_hlc,
        ..Default::default()
    };

    if let Some(ops) = request.operations {
        dispatch_operations(
            ops, classify_svc, dispatcher, claims.as_ref(), caller_origin, principal.as_ref(), &mut http_response,
        )
        .await;
    }

    msgpack_response(&http_response)
}

/// Serializes `response` as `MsgPack` and wraps it in an HTTP 200 response.
fn msgpack_response(response: &HttpSyncResponse) -> axum::response::Response {
    match rmp_serde::to_vec_named(response) {
        Ok(bytes) => (
            StatusCode::OK,
            [("content-type", MSGPACK_CONTENT_TYPE)],
            bytes,
        )
            .into_response(),
        Err(e) => {
            let json = format!(r#"{{"code":500,"message":"serialization error: {e}"}}"#);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                [("content-type", "application/json")],
                json.into_bytes(),
            )
                .into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::handlers::auth::{decode_jwt_key, JwtClaims};
    use crate::network::{ConnectionRegistry, NetworkConfig, ShutdownController};
    use arc_swap::ArcSwap;
    use jsonwebtoken::{EncodingKey, Header};
    use serde::Serialize;
    use std::sync::Arc;
    use std::time::{Instant, SystemTime, UNIX_EPOCH};
    use topgun_core::messages::HttpSyncRequest;
    use topgun_core::Timestamp;

    const TEST_SECRET: &str = "test-http-sync-secret";

    /// Minimal claims struct for building test tokens.
    #[derive(Serialize)]
    struct TestClaims {
        sub: Option<String>,
        exp: u64,
    }

    /// Encode a JWT token with HS256 for testing.
    fn make_token(sub: Option<&str>, exp_offset_secs: i64) -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after epoch")
            .as_secs();
        let exp = if exp_offset_secs >= 0 {
            now + exp_offset_secs as u64
        } else {
            now.saturating_sub((-exp_offset_secs) as u64)
        };
        let claims = TestClaims { sub: sub.map(str::to_owned), exp };
        jsonwebtoken::encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(TEST_SECRET.as_bytes()),
        )
        .expect("test token encoding should not fail")
    }

    fn test_state() -> AppState {
        AppState {
            registry: Arc::new(ConnectionRegistry::new()),
            shutdown: Arc::new(ShutdownController::new()),
            config: Arc::new(NetworkConfig::default()),
            start_time: Instant::now(),
            observability: None,
            operation_service: None,
            dispatcher: None,
            jwt_secret: None,
            cluster_state: None,
            store_factory: None,
            server_config: None,
            policy_store: None,
        }
    }

    fn test_state_with_auth(require_auth: bool) -> AppState {
        use crate::service::config::ServerConfig;
        use crate::service::security::SecurityConfig;
        let mut config = NetworkConfig::default();
        config.jwt_clock_skew_secs = 60;
        let server_cfg = ServerConfig {
            security: SecurityConfig { require_auth, ..SecurityConfig::default() },
            ..ServerConfig::default()
        };
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
            server_config: Some(Arc::new(ArcSwap::from_pointee(server_cfg))),
            policy_store: None,
        }
    }

    #[tokio::test]
    async fn http_sync_handler_returns_msgpack_content_type() {
        let state = test_state();
        let body = Bytes::from_static(b"");

        let response =
            http_sync_handler(State(state), None, axum::Extension(TokenPresence::Absent), body)
                .await
                .into_response();
        let content_type = response
            .headers()
            .get("content-type")
            .expect("content-type header must be present");
        assert_eq!(content_type, "application/msgpack");
    }

    #[tokio::test]
    async fn http_sync_handler_returns_200_for_empty_body() {
        let state = test_state();
        let body = Bytes::from_static(b"");

        let response =
            http_sync_handler(State(state), None, axum::Extension(TokenPresence::Absent), body)
                .await
                .into_response();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }

    #[tokio::test]
    async fn http_sync_handler_returns_valid_server_hlc_for_empty_request() {
        let state = test_state();

        // Encode a minimal HttpSyncRequest (no operations) as MsgPack.
        let req = HttpSyncRequest {
            client_id: "test-client".to_string(),
            client_hlc: Timestamp {
                millis: 1_000,
                counter: 0,
                node_id: "node-1".to_string(),
            },
            ..Default::default()
        };
        let body = Bytes::from(rmp_serde::to_vec_named(&req).unwrap());

        let response =
            http_sync_handler(State(state), None, axum::Extension(TokenPresence::Absent), body)
                .await
                .into_response();
        assert_eq!(response.status(), axum::http::StatusCode::OK);

        let body_bytes = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .expect("body read must succeed");
        let decoded: HttpSyncResponse =
            rmp_serde::from_slice(&body_bytes).expect("response must be valid MsgPack");

        // server_hlc.millis must be a positive wall-clock timestamp.
        assert!(decoded.server_hlc.millis > 0, "server_hlc.millis must be > 0");
    }

    #[tokio::test]
    async fn http_sync_handler_returns_400_for_malformed_body() {
        let state = test_state();
        // Two bytes that are not a valid MsgPack-encoded HttpSyncRequest.
        let body = Bytes::from_static(&[0xFF, 0xFF]);

        let response =
            http_sync_handler(State(state), None, axum::Extension(TokenPresence::Absent), body)
                .await
                .into_response();
        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    // -----------------------------------------------------------------------
    // Auth tests (AC1-AC5)
    // -----------------------------------------------------------------------

    /// AC1: Valid Bearer JWT + require_auth=true → HTTP 200.
    #[tokio::test]
    async fn auth_valid_token_sets_http_client_origin() {
        let state = test_state_with_auth(true);
        let token = make_token(Some("user-1"), 3600);
        // Build real ClientClaims by parsing the token with the test secret.
        let claims = {
            let (algo, key) = decode_jwt_key(TEST_SECRET).unwrap();
            let mut v = jsonwebtoken::Validation::new(algo);
            v.validate_aud = false;
            v.leeway = 60;
            let data = jsonwebtoken::decode::<JwtClaims>(&token, &key, &v).unwrap();
            Some(ClientClaims {
                user_id: data.claims.sub.unwrap(),
                roles: data.claims.roles.unwrap_or_default(),
            })
        };
        let body = Bytes::from_static(b"");
        let response =
            http_sync_handler(State(state), claims, axum::Extension(TokenPresence::Present), body)
                .await
                .into_response();
        // With valid token + require_auth=true, handler should proceed (200 since no dispatch).
        assert_eq!(
            response.status(),
            axum::http::StatusCode::OK,
            "valid token + require_auth=true must return HTTP 200"
        );
    }

    /// AC2: Present-but-invalid token + require_auth=true → HTTP 401 "invalid or expired token".
    #[tokio::test]
    async fn auth_invalid_token_rejected_when_required() {
        let state = test_state_with_auth(true);
        // Token present in header but claims=None (extractor failed validation).
        let body = Bytes::from_static(b"");
        let response = http_sync_handler(
            State(state),
            None, // token was present but invalid, so claims=None
            axum::Extension(TokenPresence::Present),
            body,
        )
        .await
        .into_response();
        assert_eq!(
            response.status(),
            axum::http::StatusCode::UNAUTHORIZED,
            "present-but-invalid token must return HTTP 401"
        );
        let body_bytes = axum::body::to_bytes(response.into_body(), 1024).await.unwrap();
        let body_str = std::str::from_utf8(&body_bytes).unwrap();
        assert!(
            body_str.contains("invalid or expired token"),
            "response body must contain 'invalid or expired token', got: {body_str}"
        );
    }

    /// AC3: No Authorization header + require_auth=true → HTTP 401 "authentication required".
    #[tokio::test]
    async fn auth_missing_token_rejected_when_required() {
        let state = test_state_with_auth(true);
        let body = Bytes::from_static(b"");
        let response = http_sync_handler(
            State(state),
            None,
            axum::Extension(TokenPresence::Absent),
            body,
        )
        .await
        .into_response();
        assert_eq!(
            response.status(),
            axum::http::StatusCode::UNAUTHORIZED,
            "missing token + require_auth=true must return HTTP 401"
        );
        let body_bytes = axum::body::to_bytes(response.into_body(), 1024).await.unwrap();
        let body_str = std::str::from_utf8(&body_bytes).unwrap();
        assert!(
            body_str.contains("authentication required"),
            "response body must contain 'authentication required', got: {body_str}"
        );
    }

    /// AC4: No Authorization header + require_auth=false → HTTP 200 (backward compatible).
    #[tokio::test]
    async fn auth_missing_token_allowed_when_not_required() {
        let state = test_state_with_auth(false);
        let body = Bytes::from_static(b"");
        let response = http_sync_handler(
            State(state),
            None,
            axum::Extension(TokenPresence::Absent),
            body,
        )
        .await
        .into_response();
        assert_eq!(
            response.status(),
            axum::http::StatusCode::OK,
            "missing token + require_auth=false must return HTTP 200"
        );
    }

    /// AC5: Valid Bearer JWT + require_auth=false → HTTP 200.
    #[tokio::test]
    async fn auth_valid_token_optional_auth_uses_http_client() {
        let state = test_state_with_auth(false);
        let token = make_token(Some("user-2"), 3600);
        let claims = {
            let (algo, key) = decode_jwt_key(TEST_SECRET).unwrap();
            let mut v = jsonwebtoken::Validation::new(algo);
            v.validate_aud = false;
            v.leeway = 60;
            let data = jsonwebtoken::decode::<JwtClaims>(&token, &key, &v).unwrap();
            Some(ClientClaims {
                user_id: data.claims.sub.unwrap(),
                roles: data.claims.roles.unwrap_or_default(),
            })
        };
        let body = Bytes::from_static(b"");
        let response =
            http_sync_handler(State(state), claims, axum::Extension(TokenPresence::Present), body)
                .await
                .into_response();
        assert_eq!(
            response.status(),
            axum::http::StatusCode::OK,
            "valid token + require_auth=false must return HTTP 200"
        );
    }
}
