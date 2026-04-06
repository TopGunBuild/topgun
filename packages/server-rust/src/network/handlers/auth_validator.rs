//! Custom post-JWT-verification auth validation hook.
//!
//! Operators implement `AuthValidator` to inject custom token validation logic
//! that runs after JWT signature verification. This enables token rejection
//! based on arbitrary business rules (revocation lists, tenant membership,
//! IP allowlists, custom claims) without forking the server.

use async_trait::async_trait;

/// Context passed to `AuthValidator::validate` after successful JWT decode.
///
/// Contains the principal's identity and the full decoded claims for
/// custom inspection logic.
#[derive(Debug, Clone)]
pub struct AuthValidationContext {
    /// The `sub` claim from the JWT — the principal's user ID.
    pub user_id: String,
    /// Roles extracted from the JWT.
    pub roles: Vec<String>,
    /// Full decoded JWT claims as JSON for custom claim inspection.
    pub raw_claims: serde_json::Value,
}

/// Trait for custom post-JWT-verification validation logic.
///
/// Implement this trait to add custom auth policies without forking the server.
/// The validator is called on every successful JWT decode across all three auth
/// paths (WebSocket handshake, HTTP sync, admin API).
///
/// Return `Ok(())` to accept, `Err(reason)` to reject.
/// The reason string is logged server-side and forwarded to the client
/// only when `insecure_forward_auth_errors` is enabled.
#[async_trait]
pub trait AuthValidator: Send + Sync + 'static {
    /// Validate a principal after JWT signature verification.
    ///
    /// Return `Ok(())` to accept, `Err(reason)` to reject.
    /// The reason string is logged server-side and forwarded to the client
    /// only when `insecure_forward_auth_errors` is enabled.
    async fn validate(&self, ctx: &AuthValidationContext) -> Result<(), String>;
}

/// Blanket implementation allowing synchronous closures to be used as validators.
///
/// This allows registering simple closures directly:
/// ```rust,ignore
/// app_state.auth_validator = Some(Arc::new(|ctx: &AuthValidationContext| {
///     if ctx.roles.contains(&"banned".to_string()) {
///         Err("banned user".to_string())
///     } else {
///         Ok(())
///     }
/// }));
/// ```
///
/// Async closures that require `.await` must implement the trait directly
/// (the blanket impl covers synchronous-only closures).
#[async_trait]
impl<F> AuthValidator for F
where
    F: Fn(&AuthValidationContext) -> Result<(), String> + Send + Sync + 'static,
{
    async fn validate(&self, ctx: &AuthValidationContext) -> Result<(), String> {
        (self)(ctx)
    }
}
