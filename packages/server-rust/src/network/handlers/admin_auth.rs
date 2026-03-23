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
use jsonwebtoken::{Algorithm, DecodingKey, Validation};

use super::admin_types::ErrorResponse;
use super::auth::JwtClaims;
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
            error: error_msg.to_string(),
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
        let mut validation = Validation::new(Algorithm::HS256);
        validation.validate_aud = false;
        validation.leeway = state.config.jwt_clock_skew_secs;

        let key = DecodingKey::from_secret(jwt_secret.as_bytes());

        let token_data = jsonwebtoken::decode::<JwtClaims>(token, &key, &validation)
            .map_err(|e| AdminAuthError::InvalidToken(e.to_string()))?;

        // Reject tokens without a subject claim — anonymous identity is not
        // permitted for admin endpoints.
        let user_id = token_data
            .claims
            .sub
            .ok_or_else(|| AdminAuthError::InvalidToken("missing sub claim in JWT".to_string()))?;

        let roles = token_data.claims.roles.unwrap_or_default();

        // Verify admin role
        if !roles.iter().any(|r| r == "admin") {
            return Err(AdminAuthError::Forbidden);
        }

        Ok(AdminClaims { user_id, roles })
    }
}
