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
use utoipa::ToSchema;

// rmpv is used for auth context construction in PolicyEvaluator::evaluate.

// ---------------------------------------------------------------------------
// Permission types
// ---------------------------------------------------------------------------

/// The specific data operation a policy governs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum PermissionAction {
    Read,
    Write,
    Remove,
    All,
}

/// Whether a matching policy grants or denies access.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
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
    /// Returns all policies whose `map_pattern` could match the given `map_name`.
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
// PolicyEvaluator
// ---------------------------------------------------------------------------

/// Evaluates permission policies for a given principal, action, and map.
///
/// Uses deny-wins semantics: if any matching policy denies, the result is
/// `Deny` regardless of allow policies. Principals with role "admin" bypass
/// all policy checks.
pub struct PolicyEvaluator {
    store: Arc<dyn PolicyStore>,
}

impl PolicyEvaluator {
    /// Creates a new evaluator backed by the given policy store.
    #[must_use]
    pub fn new(store: Arc<dyn PolicyStore>) -> Self {
        Self { store }
    }

    /// Returns true if the store contains at least one policy.
    ///
    /// Used by the authorization middleware to implement a permissive default:
    /// when no policies are configured, all operations are allowed so existing
    /// deployments continue to work without any RBAC configuration.
    pub async fn has_policies(&self) -> bool {
        !self.store.list_policies().await.unwrap_or_default().is_empty()
    }

    /// Evaluates whether the given principal may perform `action` on `map_name`.
    ///
    /// Constructs an auth context from `principal` so that policy conditions
    /// can reference `$auth.id` and `$auth.roles` via variable references.
    pub async fn evaluate(
        &self,
        principal: Option<&topgun_core::Principal>,
        action: PermissionAction,
        map_name: &str,
        data: &rmpv::Value,
    ) -> PolicyDecision {
        use crate::service::domain::predicate::{evaluate_predicate, EvalContext};

        // Admin principals bypass all policy checks.
        if let Some(p) = principal {
            if p.roles.iter().any(|r| r == "admin") {
                return PolicyDecision::Allow;
            }
        }

        let Ok(policies) = self.store.get_policies(map_name).await else {
            return PolicyDecision::Deny;
        };

        // Filter to policies that apply to this action.
        let action_matches: Vec<&PermissionPolicy> = policies
            .iter()
            .filter(|p| p.action == action || p.action == PermissionAction::All)
            .collect();

        // Build the auth rmpv::Value once, outside the per-policy loop.
        let auth_value = principal.map(|p| {
            rmpv::Value::Map(vec![
                (
                    rmpv::Value::String("id".into()),
                    rmpv::Value::String(p.id.clone().into()),
                ),
                (
                    rmpv::Value::String("roles".into()),
                    rmpv::Value::Array(
                        p.roles
                            .iter()
                            .map(|r| rmpv::Value::String(r.clone().into()))
                            .collect(),
                    ),
                ),
            ])
        });

        // Evaluate conditions and collect policies that pass.
        let mut effective: Vec<&PermissionPolicy> = Vec::new();
        for policy in action_matches {
            if let Some(ref condition) = policy.condition {
                let ctx = EvalContext {
                    auth: auth_value.as_ref(),
                    data,
                };
                if !evaluate_predicate(condition, &ctx) {
                    continue;
                }
            }
            effective.push(policy);
        }

        // Deny-wins: any deny in effective policies returns Deny.
        if effective.iter().any(|p| p.effect == PolicyEffect::Deny) {
            return PolicyDecision::Deny;
        }

        // Any allow returns Allow; otherwise default-deny.
        if effective.iter().any(|p| p.effect == PolicyEffect::Allow) {
            PolicyDecision::Allow
        } else {
            PolicyDecision::Deny
        }
    }
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

    // -- PolicyEvaluator --

    fn make_store_with(policies: Vec<PermissionPolicy>) -> Arc<InMemoryPolicyStore> {
        let store = Arc::new(InMemoryPolicyStore::new());
        let store_clone = store.clone();
        // Use block_on only in tests; runtime is provided by #[tokio::test].
        for p in policies {
            store_clone.policies.insert(p.id.clone(), p);
        }
        store
    }

    fn admin_principal() -> topgun_core::Principal {
        topgun_core::Principal {
            id: "admin1".to_string(),
            roles: vec!["admin".to_string()],
        }
    }

    fn user_principal() -> topgun_core::Principal {
        topgun_core::Principal {
            id: "user1".to_string(),
            roles: vec!["user".to_string()],
        }
    }

    #[tokio::test]
    async fn admin_always_allowed() {
        let store = make_store_with(vec![]);
        let eval = PolicyEvaluator::new(store);
        let data = rmpv::Value::Nil;
        let principal = admin_principal();
        let decision = eval
            .evaluate(
                Some(&principal),
                PermissionAction::Write,
                "users.profiles",
                &data,
            )
            .await;
        assert_eq!(decision, PolicyDecision::Allow);
    }

    #[tokio::test]
    async fn default_deny_when_no_policies() {
        let store = make_store_with(vec![]);
        let eval = PolicyEvaluator::new(store);
        let data = rmpv::Value::Nil;
        let principal = user_principal();
        let decision = eval
            .evaluate(
                Some(&principal),
                PermissionAction::Read,
                "users.profiles",
                &data,
            )
            .await;
        assert_eq!(decision, PolicyDecision::Deny);
    }

    #[tokio::test]
    async fn allow_when_matching_allow_policy() {
        let store = make_store_with(vec![PermissionPolicy {
            id: "p1".to_string(),
            map_pattern: "users.*".to_string(),
            action: PermissionAction::Read,
            effect: PolicyEffect::Allow,
            condition: None,
        }]);
        let eval = PolicyEvaluator::new(store);
        let data = rmpv::Value::Nil;
        let principal = user_principal();
        let decision = eval
            .evaluate(
                Some(&principal),
                PermissionAction::Read,
                "users.profiles",
                &data,
            )
            .await;
        assert_eq!(decision, PolicyDecision::Allow);
    }

    #[tokio::test]
    async fn deny_wins_over_allow() {
        let store = make_store_with(vec![
            PermissionPolicy {
                id: "allow".to_string(),
                map_pattern: "users.*".to_string(),
                action: PermissionAction::Read,
                effect: PolicyEffect::Allow,
                condition: None,
            },
            PermissionPolicy {
                id: "deny".to_string(),
                map_pattern: "*".to_string(),
                action: PermissionAction::All,
                effect: PolicyEffect::Deny,
                condition: None,
            },
        ]);
        let eval = PolicyEvaluator::new(store);
        let data = rmpv::Value::Nil;
        let principal = user_principal();
        let decision = eval
            .evaluate(
                Some(&principal),
                PermissionAction::Read,
                "users.profiles",
                &data,
            )
            .await;
        assert_eq!(decision, PolicyDecision::Deny);
    }

    #[tokio::test]
    async fn condition_false_skips_policy() {
        use topgun_core::messages::base::{PredicateNode, PredicateOp};

        // Condition that always evaluates to false: attribute "never" == "true",
        // but data has no "never" field so the predicate fails.
        let false_condition = PredicateNode {
            op: PredicateOp::Eq,
            attribute: Some("never".to_string()),
            value: Some(rmpv::Value::String("true".into())),
            value_ref: None,
            children: None,
        };

        let store = make_store_with(vec![PermissionPolicy {
            id: "conditional".to_string(),
            map_pattern: "*".to_string(),
            action: PermissionAction::Read,
            effect: PolicyEffect::Allow,
            condition: Some(false_condition),
        }]);
        let eval = PolicyEvaluator::new(store);
        // data has no "never" field, so condition evaluates to false — policy is skipped
        let data = rmpv::Value::Map(vec![]);
        let principal = user_principal();
        let decision = eval
            .evaluate(
                Some(&principal),
                PermissionAction::Read,
                "anything",
                &data,
            )
            .await;
        // No effective policies -> default deny
        assert_eq!(decision, PolicyDecision::Deny);
    }
}
