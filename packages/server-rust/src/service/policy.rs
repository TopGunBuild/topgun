//! Permission policy engine — types, trait, in-memory store, and evaluator.
//!
//! Provides declarative, pattern-based permission policies that control
//! read/write/remove access per map, with optional predicate conditions
//! evaluated against auth context and record data.

use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
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

// ---------------------------------------------------------------------------
// Glob matching
// ---------------------------------------------------------------------------

/// Matches a glob pattern against an input string using dot-separated segments.
///
/// A lone `"*"` pattern matches any input. Otherwise each dot-separated
/// segment must match exactly or be `"*"` (matching exactly one segment).
/// Segment count must be equal for non-lone-star patterns.
fn glob_matches(pattern: &str, input: &str) -> bool {
    // A lone "*" matches everything.
    if pattern == "*" {
        return true;
    }

    let pat_segs: Vec<&str> = pattern.split('.').collect();
    let inp_segs: Vec<&str> = input.split('.').collect();

    if pat_segs.len() != inp_segs.len() {
        return false;
    }

    pat_segs
        .iter()
        .zip(inp_segs.iter())
        .all(|(p, i)| *p == "*" || p == i)
}

// ---------------------------------------------------------------------------
// InMemoryPolicyStore
// ---------------------------------------------------------------------------

/// Concurrent in-memory policy store backed by `DashMap`.
///
/// Suitable for single-node deployments and testing. Policies are lost on
/// restart; a PostgreSQL-backed store can be swapped in via the `PolicyStore`
/// trait without changing callers.
#[derive(Default)]
pub struct InMemoryPolicyStore {
    policies: DashMap<String, PermissionPolicy>,
}

impl InMemoryPolicyStore {
    /// Creates a new empty store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            policies: DashMap::new(),
        }
    }
}

#[async_trait]
impl PolicyStore for InMemoryPolicyStore {
    async fn get_policies(&self, map_name: &str) -> Result<Vec<PermissionPolicy>, PolicyError> {
        let matching = self
            .policies
            .iter()
            .filter(|entry| glob_matches(&entry.map_pattern, map_name))
            .map(|entry| entry.value().clone())
            .collect();
        Ok(matching)
    }

    async fn list_policies(&self) -> Result<Vec<PermissionPolicy>, PolicyError> {
        let all = self
            .policies
            .iter()
            .map(|entry| entry.value().clone())
            .collect();
        Ok(all)
    }

    async fn upsert_policy(&self, policy: PermissionPolicy) -> Result<(), PolicyError> {
        self.policies.insert(policy.id.clone(), policy);
        Ok(())
    }

    async fn delete_policy(&self, policy_id: &str) -> Result<(), PolicyError> {
        self.policies.remove(policy_id);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// PolicyEvaluator — placeholder (implementation added in G2b)
// ---------------------------------------------------------------------------

/// Evaluates permission policies for a given principal, action, and map.
pub struct PolicyEvaluator {
    pub(crate) store: Arc<dyn PolicyStore>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- glob_matches --

    #[test]
    fn glob_star_matches_anything() {
        assert!(glob_matches("*", "anything.here"));
        assert!(glob_matches("*", "users"));
        assert!(glob_matches("*", "a.b.c.d"));
    }

    #[test]
    fn glob_users_dot_star_matches_one_segment() {
        assert!(glob_matches("users.*", "users.profiles"));
        assert!(glob_matches("users.*", "users.settings"));
        assert!(!glob_matches("users.*", "users"));
        assert!(!glob_matches("users.*", "users.a.b"));
    }

    #[test]
    fn glob_exact_match() {
        assert!(glob_matches("users", "users"));
        assert!(!glob_matches("users", "posts"));
    }

    #[test]
    fn glob_multi_segment_wildcard() {
        assert!(glob_matches("users.*.settings", "users.abc.settings"));
        assert!(!glob_matches("users.*.settings", "users.settings"));
    }

    // -- InMemoryPolicyStore --

    #[tokio::test]
    async fn upsert_then_list_returns_policy() {
        let store = InMemoryPolicyStore::new();
        let policy = PermissionPolicy {
            id: "p1".to_string(),
            map_pattern: "users.*".to_string(),
            action: PermissionAction::Read,
            effect: PolicyEffect::Allow,
            condition: None,
        };
        store.upsert_policy(policy.clone()).await.unwrap();
        let all = store.list_policies().await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, "p1");
    }

    #[tokio::test]
    async fn delete_removes_policy() {
        let store = InMemoryPolicyStore::new();
        let policy = PermissionPolicy {
            id: "p2".to_string(),
            map_pattern: "*".to_string(),
            action: PermissionAction::Write,
            effect: PolicyEffect::Deny,
            condition: None,
        };
        store.upsert_policy(policy).await.unwrap();
        store.delete_policy("p2").await.unwrap();
        let matching = store.get_policies("anything").await.unwrap();
        assert!(matching.is_empty());
    }

    #[tokio::test]
    async fn delete_nonexistent_returns_ok() {
        let store = InMemoryPolicyStore::new();
        let result = store.delete_policy("nonexistent").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn get_policies_filters_by_pattern() {
        let store = InMemoryPolicyStore::new();
        store
            .upsert_policy(PermissionPolicy {
                id: "p1".to_string(),
                map_pattern: "users.*".to_string(),
                action: PermissionAction::Read,
                effect: PolicyEffect::Allow,
                condition: None,
            })
            .await
            .unwrap();
        store
            .upsert_policy(PermissionPolicy {
                id: "p2".to_string(),
                map_pattern: "posts.*".to_string(),
                action: PermissionAction::Read,
                effect: PolicyEffect::Allow,
                condition: None,
            })
            .await
            .unwrap();

        let matching = store.get_policies("users.profiles").await.unwrap();
        assert_eq!(matching.len(), 1);
        assert_eq!(matching[0].id, "p1");
    }
}
