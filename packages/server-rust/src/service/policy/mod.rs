//! Permission policy engine — types, trait, in-memory store, and evaluator.
//!
//! Provides declarative, pattern-based permission policies that control
//! read/write/remove access per map, with optional predicate conditions
//! evaluated against auth context and record data.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use topgun_core::messages::base::PredicateNode;
use utoipa::ToSchema;

// rmpv is used for auth context construction in PolicyEvaluator::evaluate.

pub mod durable;
pub mod expr_parser;

pub use durable::DurablePolicyStore;

/// Role name with a hardcoded privileged meaning in the policy engine.
///
/// A principal is only granted the RBAC admin bypass when its subject is on the
/// server-configured admin allow-list (see [`PolicyEvaluator`]) — never by
/// carrying this role in a JWT claim. The string is still reserved so that
/// untrusted mint paths (token exchange) strip it before signing, preventing a
/// self-granted `admin` role from satisfying a policy condition that references
/// `$auth.roles`.
pub const RESERVED_ADMIN_ROLE: &str = "admin";

/// Roles that an external/untrusted token must never be able to inject. Stripped
/// at every mint boundary that copies attacker-influenced claims (token exchange).
pub const RESERVED_PRIVILEGED_ROLES: &[&str] = &[RESERVED_ADMIN_ROLE];

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

/// The fail-closed gate decision returned by `PolicyEvaluator::should_evaluate`.
///
/// This is the single predicate both enforcement points (authorization
/// middleware and the HTTP sync path) consult before deciding whether to run
/// the full policy evaluation. It encodes the configured-marker decision table:
///
/// - `AllowAll`  — store was never configured (no policy ever upserted). Skip
///   evaluation entirely; backward-compatible permissive default.
/// - `Evaluate`  — store is configured (a policy has been upserted at least
///   once). Run `evaluate`, whose default-deny applies even if all policies
///   were since deleted.
/// - `Deny`      — the store could not be read. Fail closed: deny without
///   evaluating, so a backend outage cannot silently open access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateDecision {
    AllowAll,
    Evaluate,
    Deny,
}

/// A single permission rule binding a map pattern + action to an allow/deny effect.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
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
    #[schema(value_type = Object, nullable = true)]
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

    /// Returns `true` once any policy has ever been upserted (the "configured"
    /// marker has been persisted), even if every policy was later deleted.
    ///
    /// This is the durable signal that distinguishes a deployment that has
    /// adopted RBAC (configured, possibly emptied -> must default-deny) from a
    /// deployment that has never configured RBAC at all (allow-all backward
    /// compat). It exists so the evaluator can fail closed after a restart:
    /// the empty-policy short-circuit must NOT be derived from live policy
    /// count, because a configured-but-emptied store still has to enforce
    /// default-deny semantics.
    async fn is_configured(&self) -> Result<bool, PolicyError>;
}

// ---------------------------------------------------------------------------
// Glob matching
// ---------------------------------------------------------------------------

/// Matches a glob pattern against an input string using dot-separated segments.
///
/// Segment semantics:
/// - A lone `"*"` matches any input (historical "match everything" behavior).
/// - `"*"` as a segment matches exactly one segment (`users.*` ⇒ `users.profiles`
///   but not `users` or `users.a.b`).
/// - `"**"` matches zero or more segments (recursive namespace lock). A trailing
///   `"**"` therefore locks an entire namespace: `users.**` matches `users`,
///   `users.profiles`, and `users.a.b.c`. This is the namespace-isolation form an
///   operator reaches for; single-segment `*` does NOT lock nested keys, so a deny
///   on `users.*` alone could be dodged by choosing a shallower/deeper map name.
/// - Any other segment matches literally.
fn glob_matches(pattern: &str, input: &str) -> bool {
    // A lone "*" matches everything (equivalent to "**"), kept as a fast path and
    // for backward compatibility with existing allow/deny-all policies.
    if pattern == "*" {
        return true;
    }

    let pat_segs: Vec<&str> = pattern.split('.').collect();
    let inp_segs: Vec<&str> = input.split('.').collect();

    glob_match_segments(&pat_segs, &inp_segs)
}

/// Recursive segment matcher backing [`glob_matches`].
///
/// `**` matches zero or more input segments; `*` matches exactly one; every other
/// pattern segment must equal the corresponding input segment.
fn glob_match_segments(pat: &[&str], inp: &[&str]) -> bool {
    match pat.first() {
        // Pattern exhausted: match iff input is also exhausted.
        None => inp.is_empty(),
        Some(&"**") => {
            // Try consuming 0, 1, 2, ... input segments against the remaining
            // pattern. Zero-consumption first so `users.**` matches `users`.
            for skip in 0..=inp.len() {
                if glob_match_segments(&pat[1..], &inp[skip..]) {
                    return true;
                }
            }
            false
        }
        // `*` consumes exactly one input segment.
        Some(&"*") => !inp.is_empty() && glob_match_segments(&pat[1..], &inp[1..]),
        // Literal segment must match exactly.
        Some(seg) => !inp.is_empty() && *seg == inp[0] && glob_match_segments(&pat[1..], &inp[1..]),
    }
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
    /// Process-lifetime sticky marker: set to `true` on the first upsert and
    /// never cleared on delete. Deliberately NOT derived from `policies.len()`
    /// so that a configured-then-emptied store reports `is_configured() == true`
    /// and keeps enforcing default-deny. Resets to `false` only on process
    /// restart — the documented, legitimate fail-open window for this
    /// ephemeral (non-durable) backend.
    configured: AtomicBool,
}

impl InMemoryPolicyStore {
    /// Creates a new empty store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            policies: DashMap::new(),
            configured: AtomicBool::new(false),
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
        // Sticky: once configured, stays configured for this process lifetime.
        self.configured.store(true, Ordering::SeqCst);
        Ok(())
    }

    async fn delete_policy(&self, policy_id: &str) -> Result<(), PolicyError> {
        self.policies.remove(policy_id);
        Ok(())
    }

    async fn is_configured(&self) -> Result<bool, PolicyError> {
        Ok(self.configured.load(Ordering::SeqCst))
    }
}

// ---------------------------------------------------------------------------
// PolicyEvaluator
// ---------------------------------------------------------------------------

/// Evaluates permission policies for a given principal, action, and map.
///
/// Uses deny-wins semantics: if any matching policy denies, the result is
/// `Deny` regardless of allow policies.
///
/// The admin RBAC bypass is **server-anchored**: a principal bypasses policy
/// checks only when its subject (`Principal.id`) is on the `admin_subjects`
/// allow-list, which is sourced exclusively from server-trusted configuration
/// (e.g. `TOPGUN_ADMIN_SUBJECTS`). A `roles: ["admin"]` claim in a JWT — whether
/// minted by an app, copied from an external `IdP` via token exchange, or issued by
/// the operator console — does **not** grant the bypass. This closes the
/// transitive-trust / self-grant footgun where a user-controllable `IdP` field
/// mapped to a roles claim could escalate to full RBAC bypass.
pub struct PolicyEvaluator {
    store: Arc<dyn PolicyStore>,
    /// Subjects (`Principal.id`) granted the unconditional RBAC bypass. Sourced
    /// only from server-trusted config; empty by default (no data-plane admin).
    admin_subjects: Arc<HashSet<String>>,
}

impl PolicyEvaluator {
    /// Creates a new evaluator backed by the given policy store with no admin
    /// subjects configured (no principal receives the RBAC bypass).
    #[must_use]
    pub fn new(store: Arc<dyn PolicyStore>) -> Self {
        Self {
            store,
            admin_subjects: Arc::new(HashSet::new()),
        }
    }

    /// Creates an evaluator with a server-trusted admin-subject allow-list.
    ///
    /// Only subjects in `admin_subjects` receive the RBAC admin bypass; the set
    /// must come from server configuration, never from token claims.
    #[must_use]
    pub fn with_admin_subjects(
        store: Arc<dyn PolicyStore>,
        admin_subjects: Arc<HashSet<String>>,
    ) -> Self {
        Self {
            store,
            admin_subjects,
        }
    }

    /// Returns true if the store contains at least one policy.
    ///
    /// Used by the authorization middleware to implement a permissive default:
    /// when no policies are configured, all operations are allowed so existing
    /// deployments continue to work without any RBAC configuration.
    pub async fn has_policies(&self) -> bool {
        !self
            .store
            .list_policies()
            .await
            .unwrap_or_default()
            .is_empty()
    }

    /// Fail-closed gate consulted by enforcement points before evaluating.
    ///
    /// This replaces the bare `!has_policies()` permissive short-circuit at the
    /// call sites. The contract:
    ///
    /// | `is_configured()` | result          |
    /// |-----------------|-------------------|
    /// | Ok(false)       | `AllowAll` (skip — never configured, backward compat) |
    /// | Ok(true)        | `Evaluate` (configured — run `evaluate`, default-deny if empty) |
    /// | Err(_)          | `Deny` (fail closed — never silently allow on a read error) |
    ///
    /// The "configured" signal is durable (it survives policy deletion), so an
    /// operator who once enabled RBAC and then deleted every rule still gets
    /// default-deny rather than reverting to allow-all. `has_policies()` remains
    /// available for callers that need the live count, but it must NOT be used
    /// as the gate, because an emptied-but-configured store would wrongly skip.
    pub async fn should_evaluate(&self) -> GateDecision {
        match self.store.is_configured().await {
            Ok(true) => GateDecision::Evaluate,
            Ok(false) => GateDecision::AllowAll,
            Err(_) => GateDecision::Deny,
        }
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

        // Admin bypass is server-anchored: only subjects on the configured
        // allow-list bypass policy checks. A `roles: ["admin"]` JWT claim is
        // deliberately NOT sufficient — privilege must originate from server
        // configuration, not from a (possibly self-granted) token claim.
        if let Some(p) = principal {
            if self.admin_subjects.contains(&p.id) {
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

    // F6: recursive `**` locks an entire namespace — covers the base segment and
    // any depth beneath it. This is the namespace-isolation form single-segment
    // `*` could not express.
    #[test]
    fn glob_double_star_locks_namespace() {
        assert!(glob_matches("users.**", "users")); // base segment
        assert!(glob_matches("users.**", "users.profiles")); // one deeper
        assert!(glob_matches("users.**", "users.a.b.c")); // arbitrarily deep
                                                          // Does not leak into a sibling namespace.
        assert!(!glob_matches("users.**", "posts"));
        assert!(!glob_matches("users.**", "usersX.a"));
    }

    // F6 negative control (crafted-key bypass): with the OLD single-segment glob,
    // a deny on `users.*` could be dodged by choosing a map name at a different
    // depth (`users`, `users.a.b`). A `users.**` deny must catch every such
    // crafted name so the namespace lock cannot be escaped.
    #[test]
    fn glob_double_star_blocks_crafted_key_bypass() {
        for crafted in [
            "users",
            "users.public",
            "users.private.secret",
            "users.a.b.c.d",
        ] {
            assert!(
                glob_matches("users.**", crafted),
                "namespace lock users.** must match crafted name {crafted}"
            );
        }
    }

    // F6: `**` alone matches everything (recursive equivalent of lone `*`).
    #[test]
    fn glob_double_star_alone_matches_all() {
        assert!(glob_matches("**", "users"));
        assert!(glob_matches("**", "a.b.c.d"));
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

    fn admin_subjects(subs: &[&str]) -> Arc<HashSet<String>> {
        Arc::new(subs.iter().map(|s| (*s).to_string()).collect())
    }

    // A subject on the server-trusted allow-list bypasses all policy checks,
    // even with no matching policies (which would otherwise default-deny).
    #[tokio::test]
    async fn allow_listed_subject_bypasses_policies() {
        let store = make_store_with(vec![]);
        let eval = PolicyEvaluator::with_admin_subjects(store, admin_subjects(&["admin1"]));
        let data = rmpv::Value::Nil;
        let principal = admin_principal(); // id == "admin1"
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

    // F5 negative control / regression: a principal carrying `roles: ["admin"]`
    // whose subject is NOT on the server allow-list must NOT receive the bypass.
    // This is the self-grant vector (token-exchange / app-minted claims) — it
    // must default-deny, not escalate.
    #[tokio::test]
    async fn admin_role_claim_alone_does_not_bypass() {
        // Empty allow-list: even via `new` (no admin subjects), an admin-role
        // claim cannot escalate.
        let store = make_store_with(vec![]);
        let eval = PolicyEvaluator::new(store);
        let data = rmpv::Value::Nil;
        let principal = admin_principal(); // roles: ["admin"], id "admin1"
        let decision = eval
            .evaluate(
                Some(&principal),
                PermissionAction::Write,
                "users.profiles",
                &data,
            )
            .await;
        assert_eq!(
            decision,
            PolicyDecision::Deny,
            "a roles:[\"admin\"] claim must not grant the RBAC bypass; only server-configured \
             admin subjects do"
        );
    }

    // F5: the allow-list is keyed on subject, not role. A different subject that
    // also happens to claim the admin role is still denied.
    #[tokio::test]
    async fn admin_subject_match_is_by_id_not_role() {
        let store = make_store_with(vec![]);
        let eval = PolicyEvaluator::with_admin_subjects(store, admin_subjects(&["ops-root"]));
        let data = rmpv::Value::Nil;
        // user1 with roles:["user"] is not "ops-root" -> denied.
        let principal = user_principal();
        let decision = eval
            .evaluate(Some(&principal), PermissionAction::Read, "users.x", &data)
            .await;
        assert_eq!(decision, PolicyDecision::Deny);
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
            .evaluate(Some(&principal), PermissionAction::Read, "anything", &data)
            .await;
        // No effective policies -> default deny
        assert_eq!(decision, PolicyDecision::Deny);
    }
}
