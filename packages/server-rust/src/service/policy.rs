//! Permission policy engine — types, trait, in-memory store, and evaluator.
//!
//! Provides declarative, pattern-based permission policies that control
//! read/write/remove access per map, with optional predicate conditions
//! evaluated against auth context and record data.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use topgun_core::messages::base::PredicateNode;

// ---------------------------------------------------------------------------
// Permission types
// ---------------------------------------------------------------------------

/// The specific data operation a policy governs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionAction {
    Read,
    Write,
    Remove,
    All,
}

/// Whether a matching policy grants or denies access.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PolicyEffect {
    Allow,
    Deny,
}

/// The outcome of a policy evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyDecision {
    Allow,
    Deny,
}

/// A single permission rule binding a map pattern + action to an allow/deny effect.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionPolicy {
    pub id: String,
    /// Glob pattern: "*" matches all, "users.*" matches "users.profiles", etc.
    pub map_pattern: String,
    pub action: PermissionAction,
    pub effect: PolicyEffect,
    /// Optional predicate condition. When present, policy only applies if
    /// condition evaluates to true against record data and auth context.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub condition: Option<PredicateNode>,
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can occur during policy storage operations.
#[derive(Debug, thiserror::Error)]
pub enum PolicyError {
    #[error("policy not found: {0}")]
    NotFound(String),
    #[error("internal error: {0}")]
    Internal(String),
}

// ---------------------------------------------------------------------------
// PolicyStore trait
// ---------------------------------------------------------------------------

/// Abstraction over policy persistence backends.
///
/// Implementations include `InMemoryPolicyStore` for v1 and future
/// PostgreSQL-backed stores. Using a trait here allows the storage
/// backend to be swapped without touching the evaluator or admin handlers.
#[async_trait]
pub trait PolicyStore: Send + Sync {
    /// Returns all policies whose map_pattern could match the given map_name.
    async fn get_policies(&self, map_name: &str) -> Result<Vec<PermissionPolicy>, PolicyError>;
    /// Returns all policies.
    async fn list_policies(&self) -> Result<Vec<PermissionPolicy>, PolicyError>;
    /// Creates or updates a policy by id.
    async fn upsert_policy(&self, policy: PermissionPolicy) -> Result<(), PolicyError>;
    /// Deletes a policy by id. Returns Ok(()) even if not found.
    async fn delete_policy(&self, policy_id: &str) -> Result<(), PolicyError>;
}
