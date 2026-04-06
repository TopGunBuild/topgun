//! Refresh grant types and storage trait for POST /api/auth/refresh.
//!
//! Defines the internal grant model and the `RefreshGrantStore` trait.
//! Raw refresh tokens are never stored; only their SHA-256 hashes persist.

use async_trait::async_trait;

/// A stored refresh grant.
///
/// The token itself is never stored; only its SHA-256 hash.
/// Internal type -- not serialized over the wire.
#[derive(Debug, Clone)]
pub struct RefreshGrant {
    /// Unique grant identifier (UUIDv4).
    pub id: String,
    /// Subject (user ID) this grant belongs to.
    pub sub: String,
    /// Roles carried forward from the original token exchange.
    pub roles: Vec<String>,
    /// SHA-256 hash of the refresh token (hex-encoded).
    pub token_hash: String,
    /// Grant creation time (seconds since epoch).
    pub created_at: u64,
    /// Grant expiry (seconds since epoch).
    pub expires_at: u64,
}

/// Storage trait for refresh grants.
///
/// Implementations wrap a backing store (e.g., PostgreSQL) and are injected
/// into `AppState` as `Option<Arc<dyn RefreshGrantStore>>`. When `None`,
/// the refresh endpoint returns 404 (refresh disabled).
#[async_trait]
pub trait RefreshGrantStore: Send + Sync + 'static {
    /// Refresh token grant duration in seconds (e.g., 30 days = 2_592_000).
    ///
    /// Implementations return the value they were constructed with.
    fn grant_duration_secs(&self) -> u64;

    /// Store a new refresh grant.
    ///
    /// Returns an error if insertion fails.
    async fn insert_grant(&self, grant: &RefreshGrant) -> anyhow::Result<()>;

    /// Atomically consume a grant by token hash.
    ///
    /// Uses `DELETE WHERE token_hash = $1 AND expires_at > $2 RETURNING *` to
    /// eliminate the TOCTOU race that would exist with separate find + delete steps.
    /// Two concurrent requests with the same token hash race at the database level;
    /// only one `DELETE` returns a row.
    ///
    /// Returns `None` if no unexpired grant matches (not found or already consumed).
    async fn consume_grant(&self, token_hash: &str) -> anyhow::Result<Option<RefreshGrant>>;

    /// Delete all expired grants (housekeeping).
    ///
    /// Returns the number of rows deleted.
    async fn delete_expired_grants(&self) -> anyhow::Result<u64>;
}
