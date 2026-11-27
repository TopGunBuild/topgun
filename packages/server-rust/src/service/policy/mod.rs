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
/// `auth.roles`.
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
///
/// Persisted through [`StoredPolicy`]: a policy that uses no placeholder and no
/// `key_pattern` keeps the encoding of earlier versions byte for byte; one that
/// uses either is written in a scoped encoding earlier versions refuse to
/// decode, so they stop at start instead of widening it (`TG-AUTHZ-003`).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(
    rename_all = "camelCase",
    try_from = "StoredPolicy",
    into = "StoredPolicy"
)]
pub struct PermissionPolicy {
    pub id: String,
    /// `.`-segment pattern: `*` is one segment, `**` zero or more, a lone `*`
    /// everything; a whole segment `{auth.id}` or `{auth.tenant}` is the
    /// principal's own value.
    pub map_pattern: String,
    /// `:`-segment pattern over the keys a write names, same grammar; write
    /// rules only (`TG-AUTHZ-004`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub key_pattern: Option<String>,
    pub action: PermissionAction,
    pub effect: PolicyEffect,
    /// Optional predicate condition. When present, policy only applies if
    /// condition evaluates to true against record data and auth context.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[schema(value_type = Object, nullable = true)]
    pub condition: Option<PredicateNode>,
}

/// The persisted row of a [`PermissionPolicy`].
///
/// Field order and names of a plain policy are those of earlier versions, so
/// its bytes do not change. A scoped policy names its map pattern
/// `scopedMapPattern` and has no `mapPattern`: a binary built before
/// placeholders fails to decode it (missing field) and refuses to start,
/// instead of reading a placeholder as a literal or dropping `keyPattern`.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredPolicy {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    map_pattern: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    scoped_map_pattern: Option<String>,
    action: PermissionAction,
    effect: PolicyEffect,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    condition: Option<PredicateNode>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    key_pattern: Option<String>,
}

impl From<PermissionPolicy> for StoredPolicy {
    fn from(policy: PermissionPolicy) -> Self {
        let scoped = policy.key_pattern.is_some() || has_placeholder(&policy.map_pattern, '.');
        let (map_pattern, scoped_map_pattern) = if scoped {
            (None, Some(policy.map_pattern))
        } else {
            (Some(policy.map_pattern), None)
        };
        Self {
            id: policy.id,
            map_pattern,
            scoped_map_pattern,
            action: policy.action,
            effect: policy.effect,
            condition: policy.condition,
            key_pattern: policy.key_pattern,
        }
    }
}

impl TryFrom<StoredPolicy> for PermissionPolicy {
    type Error = String;

    /// Applies the pattern grammar and the `keyPattern`/action rule to a
    /// stored row (not the tenancy or condition-namespace rules, so a policy
    /// stored under other settings still loads), and logs a condition whose
    /// meaning this version changes.
    fn try_from(row: StoredPolicy) -> Result<Self, String> {
        let map_pattern = match (row.map_pattern, row.scoped_map_pattern) {
            (Some(plain), None) => {
                if row.key_pattern.is_some() {
                    return Err("a plain row carries keyPattern".to_string());
                }
                if plain.contains(['{', '}']) {
                    return Err(format!(
                        "mapPattern {plain:?} contains a brace; a policy stored before \
                         placeholders existed is not read as a template"
                    ));
                }
                plain
            }
            (None, Some(scoped)) => scoped,
            _ => return Err("exactly one of mapPattern and scopedMapPattern".to_string()),
        };
        let policy = Self {
            id: row.id,
            map_pattern,
            key_pattern: row.key_pattern,
            action: row.action,
            effect: row.effect,
            condition: row.condition,
        };
        validate_patterns(&policy, None).map_err(|refusal| refusal.message)?;
        if let Some(warning) = condition_meaning_change(&policy) {
            tracing::warn!("{warning}");
        }
        Ok(policy)
    }
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
/// durable stores. Using a trait here allows the storage
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
// Pattern grammar and matching
// ---------------------------------------------------------------------------

/// A principal value a pattern segment can name (`TG-AUTHZ-003`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Placeholder {
    Id,
    Tenant,
}

impl Placeholder {
    fn token(self) -> &'static str {
        match self {
            Self::Id => "{auth.id}",
            Self::Tenant => "{auth.tenant}",
        }
    }

    fn parse(segment: &str) -> Option<Self> {
        match segment {
            "{auth.id}" => Some(Self::Id),
            "{auth.tenant}" => Some(Self::Tenant),
            _ => None,
        }
    }

    /// The principal's value when it fills one segment: present, non-empty
    /// and free of the pattern's separator. Never read from the evaluated name.
    fn fill(self, principal: Option<&topgun_core::Principal>, sep: char) -> Option<&str> {
        let value = match self {
            Self::Id => principal.map(|p| p.id.as_str()),
            Self::Tenant => principal.and_then(|p| p.tenant.as_deref()),
        }?;
        (!value.is_empty() && !value.contains(sep)).then_some(value)
    }
}

/// One pattern segment once placeholders are resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Segment<'a> {
    /// Compared byte for byte; a `*` inside it has no glob meaning.
    Literal(&'a str),
    One,
    Any,
}

/// Splits a pattern at `sep` outside braces, so a placeholder's inner `.`
/// does not cut it in two.
fn split_pattern(pattern: &str, sep: char) -> Vec<&str> {
    let mut segments = Vec::new();
    let mut start = 0;
    let mut in_brace = false;
    for (i, c) in pattern.char_indices() {
        match c {
            '{' => in_brace = true,
            '}' => in_brace = false,
            c if c == sep && !in_brace => {
                segments.push(&pattern[start..i]);
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    segments.push(&pattern[start..]);
    segments
}

fn has_placeholder(pattern: &str, sep: char) -> bool {
    split_pattern(pattern, sep)
        .into_iter()
        .any(|seg| Placeholder::parse(seg).is_some())
}

/// Resolves a stored pattern's segments. A placeholder takes the principal's
/// value; one that is not fillable is recorded in `unfilled` and becomes one
/// `*` segment when `unfilled_as_one`, else the pattern matches nothing
/// (`None`).
fn resolve_pattern<'a>(
    pattern: &'a str,
    sep: char,
    principal: Option<&'a topgun_core::Principal>,
    unfilled_as_one: bool,
    unfilled: &mut Option<Placeholder>,
) -> Option<Vec<Segment<'a>>> {
    let mut resolved = Vec::new();
    for seg in split_pattern(pattern, sep) {
        resolved.push(match seg {
            "*" => Segment::One,
            "**" => Segment::Any,
            seg => match Placeholder::parse(seg) {
                None => Segment::Literal(seg),
                Some(placeholder) => {
                    if let Some(value) = placeholder.fill(principal, sep) {
                        Segment::Literal(value)
                    } else {
                        *unfilled = Some(placeholder);
                        if !unfilled_as_one {
                            return None;
                        }
                        Segment::One
                    }
                }
            },
        });
    }
    Some(resolved)
}

/// Whether resolved pattern segments match `input`. A lone `*` matches
/// everything (historical "match everything" behavior).
fn segments_match(pattern: &str, resolved: &[Segment], input: &str, sep: char) -> bool {
    if pattern == "*" {
        return true;
    }
    let inp: Vec<&str> = input.split(sep).collect();
    match_segments(resolved, &inp)
}

/// Matches a map pattern against a map name, every placeholder taken as one
/// `*` segment: the store pre-filter. It returns a superset of the policies
/// whose pattern matches for any principal, so it never drops a deny.
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
    let mut unfilled = None;
    resolve_pattern(pattern, '.', None, true, &mut unfilled)
        .is_some_and(|resolved| segments_match(pattern, &resolved, input, '.'))
}

/// Recursive segment matcher: `Any` matches zero or more input segments, `One`
/// exactly one, and a literal must equal the corresponding input segment.
fn match_segments(pat: &[Segment], inp: &[&str]) -> bool {
    match pat.first() {
        // Pattern exhausted: match iff input is also exhausted.
        None => inp.is_empty(),
        Some(Segment::Any) => {
            // Try consuming 0, 1, 2, ... input segments against the remaining
            // pattern. Zero-consumption first so `users.**` matches `users`.
            for skip in 0..=inp.len() {
                if match_segments(&pat[1..], &inp[skip..]) {
                    return true;
                }
            }
            false
        }
        Some(Segment::One) => !inp.is_empty() && match_segments(&pat[1..], &inp[1..]),
        Some(Segment::Literal(seg)) => {
            !inp.is_empty() && *seg == inp[0] && match_segments(&pat[1..], &inp[1..])
        }
    }
}

/// Why a policy is refused, and the request field it is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyRefusal {
    pub field: &'static str,
    pub message: String,
}

impl PolicyRefusal {
    fn new(field: &'static str, message: String) -> Self {
        Self { field, message }
    }
}

/// Checks one pattern's grammar: each segment is a literal, `*`, `**` or a
/// whole placeholder; a `{` or `}` anywhere else and a partial star are
/// refused. With `tenancy` `Some(false)`, `{auth.tenant}` is refused too.
fn validate_pattern(
    pattern: &str,
    sep: char,
    field: &'static str,
    tenancy: Option<bool>,
) -> Result<(), PolicyRefusal> {
    for seg in split_pattern(pattern, sep) {
        if let Some(placeholder) = Placeholder::parse(seg) {
            if placeholder == Placeholder::Tenant && tenancy == Some(false) {
                return Err(PolicyRefusal::new(
                    field,
                    format!(
                        "{field} uses {{auth.tenant}} but tenancy is off \
                         (TOPGUN_TENANT_CLAIM is unset): no principal carries a tenant"
                    ),
                ));
            }
        } else if seg.contains(['{', '}']) {
            return Err(PolicyRefusal::new(
                field,
                format!(
                    "{field} segment {seg:?} has a brace but is not a whole placeholder; \
                     the placeholders are {{auth.id}} and {{auth.tenant}}, each a whole segment"
                ),
            ));
        } else if seg.contains('*') && seg != "*" && seg != "**" {
            return Err(PolicyRefusal::new(
                field,
                format!(
                    "{field} segment {seg:?} contains `*` but is neither `*` nor `**`; \
                     it would match only a name spelled with that star"
                ),
            ));
        }
    }
    Ok(())
}

/// The pattern rules: grammar of `mapPattern` and `keyPattern`, and
/// `keyPattern` on write rules only. `tenancy` `None` skips the tenancy rule,
/// as loading a stored row does.
fn validate_patterns(
    policy: &PermissionPolicy,
    tenancy: Option<bool>,
) -> Result<(), PolicyRefusal> {
    validate_pattern(&policy.map_pattern, '.', "mapPattern", tenancy)?;
    if let Some(key_pattern) = &policy.key_pattern {
        if key_pattern.is_empty() {
            return Err(PolicyRefusal::new(
                "keyPattern",
                "keyPattern is empty; omit it to match every key".to_string(),
            ));
        }
        match policy.action {
            PermissionAction::Write => {}
            PermissionAction::Remove => {
                return Err(PolicyRefusal::new(
                    "keyPattern",
                    "keyPattern is not accepted on a remove rule: removes are evaluated as \
                     writes, so a remove rule would never apply; put it on a write rule"
                        .to_string(),
                ))
            }
            PermissionAction::Read | PermissionAction::All => {
                return Err(PolicyRefusal::new(
                    "keyPattern",
                    "keyPattern is accepted only on a write rule: reads and keyless \
                     operations carry no key"
                        .to_string(),
                ))
            }
        }
        validate_pattern(key_pattern, ':', "keyPattern", tenancy)?;
    }
    Ok(())
}

/// The one rule an upserted policy must pass (`TG-AUTHZ-003`, `TG-AUTHZ-004`):
/// the pattern grammar, `keyPattern` on write rules only, `{auth.tenant}` only
/// while tenancy is on, and a condition whose references are all known.
///
/// # Errors
///
/// The first rule the policy breaks. A condition refusal names the field
/// `condition`; a caller that took the condition from `conditionExpr` names
/// that field instead.
pub fn validate_policy(policy: &PermissionPolicy, tenancy: bool) -> Result<(), PolicyRefusal> {
    validate_patterns(policy, Some(tenancy))?;
    if let Some(condition) = &policy.condition {
        if let Some(reference) = first_unknown_reference(condition) {
            let message = match reference.kind {
                ReferenceKind::Reserved => {
                    format!("{} is reserved and not supported yet", reference.path)
                }
                ReferenceKind::ResourceField => format!(
                    "{} is not a resource field; the fields are resource.map and resource.key",
                    reference.path
                ),
                ReferenceKind::Namespace | ReferenceKind::ResourceNow => format!(
                    "{} is not in a condition namespace; a reference names auth, data or resource",
                    reference.path
                ),
            };
            return Err(PolicyRefusal::new("condition", message));
        }
    }
    Ok(())
}

/// How a condition reference reads in policy scope, where it differs from
/// earlier versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReferenceKind {
    /// `resource.to`: reserved, unknown.
    Reserved,
    /// Bare `resource` or another `resource.…` field: unknown.
    ResourceField,
    /// A `value_ref` outside `auth`, `data` and `resource`: unknown.
    Namespace,
    /// `resource.map` / `resource.key`: now the evaluated resource.
    ResourceNow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Reference {
    path: String,
    kind: ReferenceKind,
    value_ref: bool,
}

/// Every reference of a condition tree whose policy-scope reading is not that
/// of a record field or a known namespace, in tree order.
fn special_references(node: &PredicateNode, out: &mut Vec<Reference>) {
    use crate::service::domain::predicate::{is_policy_namespace, resource_field, ResourceField};
    let classify = |path: &str, is_value_ref: bool| -> Option<ReferenceKind> {
        match resource_field(path) {
            Some(ResourceField::Map | ResourceField::Key) => Some(ReferenceKind::ResourceNow),
            Some(ResourceField::Other) if path == "resource.to" => Some(ReferenceKind::Reserved),
            Some(ResourceField::Other) => Some(ReferenceKind::ResourceField),
            None if is_value_ref && !is_policy_namespace(path) => Some(ReferenceKind::Namespace),
            None => None,
        }
    };
    for (path, is_value_ref) in [(&node.attribute, false), (&node.value_ref, true)] {
        if let Some(path) = path {
            if let Some(kind) = classify(path, is_value_ref) {
                out.push(Reference {
                    path: path.clone(),
                    kind,
                    value_ref: is_value_ref,
                });
            }
        }
    }
    for child in node.children.as_deref().unwrap_or(&[]) {
        special_references(child, out);
    }
}

fn first_unknown_reference(condition: &PredicateNode) -> Option<Reference> {
    let mut refs = Vec::new();
    special_references(condition, &mut refs);
    refs.into_iter()
        .find(|r| r.kind != ReferenceKind::ResourceNow)
}

/// The startup warning for a stored policy whose condition this version reads
/// differently (`TG-AUTHZ-003`): a reference that is now unknown, or a
/// `resource.map` / `resource.key` that was a record field. `None` for a
/// policy whose meaning is unchanged.
fn condition_meaning_change(policy: &PermissionPolicy) -> Option<String> {
    let mut refs = Vec::new();
    special_references(policy.condition.as_ref()?, &mut refs);
    // An unknown reference changes more than a renamed one, so it is named first.
    let reference = refs
        .iter()
        .find(|r| r.kind != ReferenceKind::ResourceNow)
        .or_else(|| refs.first())?;
    let field = reference.path.trim_start_matches("resource.");
    Some(match reference.kind {
        ReferenceKind::ResourceNow if reference.value_ref => format!(
            "policy {}: {} now names the evaluated {field}; it used to resolve to nothing",
            policy.id, reference.path
        ),
        ReferenceKind::ResourceNow => format!(
            "policy {}: {} now names the evaluated {field}, not a record field",
            policy.id, reference.path
        ),
        _ => format!(
            "policy {}: {} is not a known condition namespace; the condition now fails \
             closed: as an allow it does not match, as a deny it applies",
            policy.id, reference.path
        ),
    })
}

// ---------------------------------------------------------------------------
// InMemoryPolicyStore
// ---------------------------------------------------------------------------

/// Concurrent in-memory policy store backed by `DashMap`.
///
/// Suitable for single-node deployments and testing. Policies are lost on
/// restart; a durable store can be swapped in via the `PolicyStore`
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

/// Policies that have already logged an unfilled placeholder in this process.
/// Process-wide, not per evaluator, because HTTP `/sync` builds an evaluator
/// per request: each policy is logged once, not on every request.
static UNFILLED_LOGGED: std::sync::LazyLock<DashMap<String, ()>> =
    std::sync::LazyLock::new(DashMap::new);

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
    /// Whether the tenant boundary (TG-TEN-001) is enforced by the
    /// authorization middleware that holds this evaluator.
    tenancy: bool,
}

impl PolicyEvaluator {
    /// Creates a new evaluator backed by the given policy store with no admin
    /// subjects configured (no principal receives the RBAC bypass).
    #[must_use]
    pub fn new(store: Arc<dyn PolicyStore>) -> Self {
        Self {
            store,
            admin_subjects: Arc::new(HashSet::new()),
            tenancy: false,
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
            tenancy: false,
        }
    }

    /// Turns the tenant boundary (TG-TEN-001) on or off for the middleware
    /// that holds this evaluator. Set from the same value as
    /// `NetworkConfig.tenant_claim`, so the claim check and the boundary are
    /// either both on or both off.
    #[must_use]
    pub fn with_tenancy(mut self, tenancy: bool) -> Self {
        self.tenancy = tenancy;
        self
    }

    /// Whether the tenant boundary (TG-TEN-001) is enforced.
    #[must_use]
    pub fn tenancy(&self) -> bool {
        self.tenancy
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

    /// Whether `principal` may read `map_name`, decided without waiting: the
    /// gate and the evaluation as the authorization middleware runs them, for
    /// a caller that must not await (the journal's delivery runs under a key
    /// writer). Record conditions see no record, as on every read path. A
    /// store that cannot answer at once denies, as a store that fails does.
    #[must_use]
    pub fn may_read_now(&self, principal: Option<&topgun_core::Principal>, map_name: &str) -> bool {
        use futures_util::FutureExt;

        match self.should_evaluate().now_or_never() {
            Some(GateDecision::AllowAll) => true,
            Some(GateDecision::Evaluate) => {
                self.evaluate(
                    principal,
                    PermissionAction::Read,
                    map_name,
                    &rmpv::Value::Nil,
                )
                .now_or_never()
                    == Some(PolicyDecision::Allow)
            }
            Some(GateDecision::Deny) | None => false,
        }
    }

    /// Evaluates whether the given principal may perform `action` on
    /// `map_name`, for an operation that names no key.
    pub async fn evaluate(
        &self,
        principal: Option<&topgun_core::Principal>,
        action: PermissionAction,
        map_name: &str,
        data: &rmpv::Value,
    ) -> PolicyDecision {
        self.evaluate_key(principal, action, map_name, None, data)
            .await
    }

    /// Evaluates whether the given principal may perform `action` on `key` of
    /// `map_name` (`TG-AUTHZ-004`); `key` is `None` for an operation that
    /// names none, where an allow with a `keyPattern` does not match and a
    /// deny with one applies.
    ///
    /// Policy conditions see the principal as `auth` (`id`, `roles`, `kind`
    /// and, for a tenanted principal, `tenant`), the record as `data`, and the
    /// evaluated resource `{map, key?}` as `resource` (`TG-AUTHZ-002`,
    /// `TG-AUTHZ-003`).
    pub async fn evaluate_key(
        &self,
        principal: Option<&topgun_core::Principal>,
        action: PermissionAction,
        map_name: &str,
        key: Option<&str>,
        data: &rmpv::Value,
    ) -> PolicyDecision {
        use crate::service::domain::predicate::{evaluate_policy_condition_on, EvalContext};

        // Admin bypass is server-anchored: only subjects on the configured
        // allow-list bypass policy checks. A `roles: ["admin"]` JWT claim is
        // deliberately NOT sufficient — privilege must originate from server
        // configuration, not from a (possibly self-granted) token claim. A
        // guest never gets it: its `sub` comes from whoever minted the guest
        // token, so it may collide with an admin subject (TG-AUTHZ-005).
        if let Some(p) = principal {
            if p.kind == topgun_core::types::PrincipalKind::User
                && self.admin_subjects.contains(&p.id)
            {
                return PolicyDecision::Allow;
            }
        }

        let Ok(policies) = self.store.get_policies(map_name).await else {
            return PolicyDecision::Deny;
        };

        // Filter to policies that apply to this action and this resource.
        let action_matches: Vec<&PermissionPolicy> = policies
            .iter()
            .filter(|p| p.action == action || p.action == PermissionAction::All)
            .filter(|p| applies_to(p, principal, map_name, key))
            .collect();

        // Build the auth rmpv::Value once, outside the per-policy loop. A
        // field the principal does not carry is absent, so a condition on it
        // is unknown (TG-AUTHZ-002).
        let auth_value = principal.map(|p| {
            let mut fields = vec![
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
                (
                    rmpv::Value::String("kind".into()),
                    rmpv::Value::String(p.kind.as_str().into()),
                ),
            ];
            if let Some(tenant) = &p.tenant {
                fields.push((
                    rmpv::Value::String("tenant".into()),
                    rmpv::Value::String(tenant.clone().into()),
                ));
            }
            rmpv::Value::Map(fields)
        });
        let mut resource_value = vec![(
            rmpv::Value::String("map".into()),
            rmpv::Value::String(map_name.into()),
        )];
        if let Some(key) = key {
            resource_value.push((
                rmpv::Value::String("key".into()),
                rmpv::Value::String(key.into()),
            ));
        }
        let resource_value = rmpv::Value::Map(resource_value);

        // Evaluate conditions and collect policies that pass.
        let mut effective: Vec<&PermissionPolicy> = Vec::new();
        for policy in action_matches {
            if let Some(ref condition) = policy.condition {
                let ctx = EvalContext {
                    auth: auth_value.as_ref(),
                    data,
                };
                // An unknown reference must neither make an allow match nor
                // keep a deny from applying (TG-AUTHZ-002, TG-AUTHZ-003).
                let unknown = policy.effect == PolicyEffect::Deny;
                if !evaluate_policy_condition_on(condition, &ctx, unknown, &resource_value) {
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

/// Whether `policy`'s patterns match `map_name` and `key` for `principal`
/// (`TG-AUTHZ-003`, `TG-AUTHZ-004`). A placeholder the principal cannot
/// fill never widens an allow and never narrows a deny: an allow does not
/// match, a deny's map segment matches as one `*`, and a deny's
/// `keyPattern` matches every key, as it does on an operation with no key.
fn applies_to(
    policy: &PermissionPolicy,
    principal: Option<&topgun_core::Principal>,
    map_name: &str,
    key: Option<&str>,
) -> bool {
    let deny = policy.effect == PolicyEffect::Deny;
    let mut unfilled = None;
    let fits = resolve_pattern(&policy.map_pattern, '.', principal, deny, &mut unfilled)
        .is_some_and(|resolved| segments_match(&policy.map_pattern, &resolved, map_name, '.'))
        && match (&policy.key_pattern, key) {
            (None, _) => true,
            (Some(_), None) => deny,
            (Some(key_pattern), Some(key)) => {
                match resolve_pattern(key_pattern, ':', principal, false, &mut unfilled) {
                    Some(resolved) => segments_match(key_pattern, &resolved, key, ':'),
                    None => deny,
                }
            }
        };
    if let Some(placeholder) = unfilled {
        if UNFILLED_LOGGED.insert(policy.id.clone(), ()).is_none() {
            tracing::warn!(
                "policy {}: {} has no single-segment value for this principal; \
                 the policy fails closed: as an allow it does not match, as a deny it applies",
                policy.id,
                placeholder.token()
            );
        }
    }
    fits
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- validate_pattern --

    fn refused_segment(pattern: &str) -> Option<String> {
        validate_pattern(pattern, '.', "mapPattern", Some(true))
            .err()
            .map(|refusal| refusal.message)
    }

    #[test]
    fn a_pattern_segment_with_a_partial_star_is_refused_by_name() {
        for (pattern, segment) in [
            ("users/*", "users/*"),
            ("a*", "a*"),
            ("a.b*", "b*"),
            ("a*.b*", "a*"),
        ] {
            let message = refused_segment(pattern).expect(pattern);
            assert!(message.contains(&format!("{segment:?}")), "{message}");
        }
    }

    #[test]
    fn whole_segment_stars_and_literal_patterns_are_accepted() {
        for pattern in ["*", "**", "a.**", "*.orders", "users/x"] {
            assert_eq!(refused_segment(pattern), None, "{pattern}");
        }
    }

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
            key_pattern: None,
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
            key_pattern: None,
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
                key_pattern: None,
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
                key_pattern: None,
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
            tenant: None,
            kind: topgun_core::types::PrincipalKind::User,
        }
    }

    fn user_principal() -> topgun_core::Principal {
        topgun_core::Principal {
            id: "user1".to_string(),
            roles: vec!["user".to_string()],
            tenant: None,
            kind: topgun_core::types::PrincipalKind::User,
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
            key_pattern: None,
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
                key_pattern: None,
                action: PermissionAction::Read,
                effect: PolicyEffect::Allow,
                condition: None,
            },
            PermissionPolicy {
                id: "deny".to_string(),
                map_pattern: "*".to_string(),
                key_pattern: None,
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
            key_pattern: None,
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

    fn principal(id: &str, roles: &[&str]) -> topgun_core::Principal {
        topgun_core::Principal {
            id: id.to_string(),
            roles: roles.iter().map(|r| (*r).to_string()).collect(),
            tenant: None,
            kind: topgun_core::types::PrincipalKind::User,
        }
    }

    fn rec(pairs: Vec<(&str, rmpv::Value)>) -> rmpv::Value {
        rmpv::Value::Map(
            pairs
                .into_iter()
                .map(|(k, v)| (rmpv::Value::String(k.into()), v))
                .collect(),
        )
    }

    fn forged(field: &str, value: rmpv::Value) -> rmpv::Value {
        rec(vec![("auth", rec(vec![(field, value)]))])
    }

    fn s(v: &str) -> rmpv::Value {
        rmpv::Value::String(v.into())
    }

    /// Policies from `(effect, condition expression)`; `None` is unconditional.
    fn evaluator(
        action: PermissionAction,
        policies: &[(PolicyEffect, Option<&str>)],
    ) -> PolicyEvaluator {
        let store = make_store_with(
            policies
                .iter()
                .enumerate()
                .map(|(i, (effect, expr))| PermissionPolicy {
                    id: format!("p{i}"),
                    map_pattern: "*".to_string(),
                    key_pattern: None,
                    action,
                    effect: *effect,
                    condition: expr.map(|e| {
                        expr_parser::parse_permission_expr(e)
                            .unwrap_or_else(|err| panic!("'{e}': {err}"))
                    }),
                })
                .collect(),
        );
        // Configured, as a server that has written a policy is, so the gate
        // evaluates instead of allowing every read.
        store
            .configured
            .store(true, std::sync::atomic::Ordering::SeqCst);
        PolicyEvaluator::new(store)
    }

    async fn write(
        eval: &PolicyEvaluator,
        who: Option<&topgun_core::Principal>,
        data: &rmpv::Value,
    ) -> PolicyDecision {
        eval.evaluate(who, PermissionAction::Write, "m", data).await
    }

    #[tokio::test]
    async fn stored_auth_literal_condition_reads_the_principal() {
        use topgun_core::messages::base::PredicateOp;
        let stored = PredicateNode {
            op: PredicateOp::Eq,
            attribute: Some("auth.id".to_string()),
            value: Some(s("bob")),
            value_ref: None,
            children: None,
        };
        assert_eq!(
            expr_parser::parse_permission_expr("auth.id == 'bob'").unwrap(),
            stored
        );
        let store = make_store_with(vec![PermissionPolicy {
            id: "stored".to_string(),
            map_pattern: "m".to_string(),
            key_pattern: None,
            action: PermissionAction::Write,
            effect: PolicyEffect::Allow,
            condition: Some(stored),
        }]);
        let eval = PolicyEvaluator::new(store);
        let alice = principal("alice", &[]);
        let bob = principal("bob", &[]);
        assert_eq!(
            write(&eval, Some(&alice), &forged("id", s("bob"))).await,
            PolicyDecision::Deny
        );
        assert_eq!(
            write(&eval, Some(&bob), &rec(vec![])).await,
            PolicyDecision::Allow
        );
    }

    #[tokio::test]
    async fn a_condition_needing_a_missing_principal_fails_closed() {
        // (policies, caller, record, decision)
        type Case<'a> = (
            &'a [(PolicyEffect, Option<&'a str>)],
            Option<&'a topgun_core::Principal>,
            rmpv::Value,
            PolicyDecision,
        );
        use PolicyEffect::{Allow, Deny};
        let empty = rec(vec![]);
        let alice = principal("alice", &[]);
        let not_admin_subject = principal("admin", &["admin"]);
        let cases: Vec<Case> = vec![
            (
                &[(Allow, Some("auth.id == 'bob'"))],
                None,
                empty.clone(),
                PolicyDecision::Deny,
            ),
            (
                &[(Allow, None), (Deny, Some("auth.id != 'admin'"))],
                None,
                empty.clone(),
                PolicyDecision::Deny,
            ),
            (
                &[(Allow, None), (Deny, Some("!(auth.id == 'admin')"))],
                None,
                empty.clone(),
                PolicyDecision::Deny,
            ),
            (
                &[(Allow, None), (Deny, Some("data.ownerId != auth.id"))],
                None,
                rec(vec![("ownerId", s("x"))]),
                PolicyDecision::Deny,
            ),
            (
                &[(Allow, Some("data.public == true && auth == null"))],
                None,
                rec(vec![("public", rmpv::Value::Boolean(true))]),
                PolicyDecision::Allow,
            ),
            (
                &[(Allow, Some("auth == null || auth.id == 'bob'"))],
                None,
                empty.clone(),
                PolicyDecision::Allow,
            ),
            (
                &[
                    (Allow, None),
                    (Deny, Some("auth != null && auth.id == 'mallory'")),
                ],
                None,
                empty.clone(),
                PolicyDecision::Allow,
            ),
            (
                &[(Allow, None), (Deny, Some("auth.id != 'admin'"))],
                Some(&alice),
                empty.clone(),
                PolicyDecision::Deny,
            ),
            (
                &[(Allow, None), (Deny, Some("auth.id != 'admin'"))],
                Some(&not_admin_subject),
                empty.clone(),
                PolicyDecision::Allow,
            ),
            (
                &[(Allow, None), (Deny, Some("auth.tenant != 'x'"))],
                Some(&alice),
                empty.clone(),
                PolicyDecision::Deny,
            ),
            (
                &[(Allow, Some("auth.tenant == 'x'"))],
                Some(&alice),
                forged("tenant", s("x")),
                PolicyDecision::Deny,
            ),
        ];
        for (i, (policies, who, data, expected)) in cases.into_iter().enumerate() {
            let eval = evaluator(PermissionAction::Write, policies);
            assert_eq!(
                write(&eval, who, &data).await,
                expected,
                "case {i}: {policies:?}"
            );
        }
    }

    #[tokio::test]
    async fn auth_only_conditions_without_a_record_decide_from_the_principal() {
        let bob = principal("bob", &[]);
        let alice = principal("alice", &[]);
        let mallory = principal("mallory", &[]);

        let reads = evaluator(
            PermissionAction::Read,
            &[(PolicyEffect::Allow, Some("auth.id == 'bob'"))],
        );
        assert!(reads.may_read_now(Some(&bob), "m"));
        assert!(!reads.may_read_now(Some(&alice), "m"));
        assert!(!reads.may_read_now(None, "m"));

        let nil = rmpv::Value::Nil;
        let writes = evaluator(
            PermissionAction::Write,
            &[(PolicyEffect::Allow, Some("auth.id == 'bob'"))],
        );
        assert_eq!(
            write(&writes, Some(&bob), &nil).await,
            PolicyDecision::Allow
        );
        assert_eq!(
            write(&writes, Some(&alice), &nil).await,
            PolicyDecision::Deny
        );

        let denied = evaluator(
            PermissionAction::Write,
            &[
                (PolicyEffect::Allow, None),
                (PolicyEffect::Deny, Some("auth.id == 'mallory'")),
            ],
        );
        assert_eq!(
            write(&denied, Some(&mallory), &nil).await,
            PolicyDecision::Deny
        );
        assert_eq!(
            write(&denied, Some(&bob), &nil).await,
            PolicyDecision::Allow
        );
    }

    #[tokio::test]
    async fn documented_forms_read_record_and_principal_from_their_own_sides() {
        let u1 = principal("u1", &[]);
        let owner = evaluator(
            PermissionAction::Write,
            &[(PolicyEffect::Allow, Some("auth.id == data.ownerId"))],
        );
        let mine = rec(vec![
            ("ownerId", s("u1")),
            ("auth", rec(vec![("id", s("x"))])),
        ]);
        let theirs = rec(vec![
            ("ownerId", s("x")),
            ("auth", rec(vec![("id", s("u1"))])),
        ]);
        assert_eq!(write(&owner, Some(&u1), &mine).await, PolicyDecision::Allow);
        assert_eq!(
            write(&owner, Some(&u1), &theirs).await,
            PolicyDecision::Deny
        );

        let editor = principal("e", &["editor"]);
        let role = evaluator(
            PermissionAction::Write,
            &[(PolicyEffect::Allow, Some("data.role in auth.roles"))],
        );
        assert_eq!(
            write(&role, Some(&editor), &rec(vec![("role", s("editor"))])).await,
            PolicyDecision::Allow
        );
        let forged_admin = rec(vec![
            ("role", s("admin")),
            (
                "auth",
                rec(vec![("roles", rmpv::Value::Array(vec![s("admin")]))]),
            ),
        ]);
        assert_eq!(
            write(&role, Some(&editor), &forged_admin).await,
            PolicyDecision::Deny
        );
    }

    // -- Templated patterns, keyPattern and condition namespaces --

    fn tenanted(id: &str, tenant: Option<&str>) -> topgun_core::Principal {
        topgun_core::Principal {
            id: id.to_string(),
            roles: Vec::new(),
            tenant: tenant.map(str::to_string),
            kind: topgun_core::types::PrincipalKind::User,
        }
    }

    fn rule(
        id: &str,
        map_pattern: &str,
        key_pattern: Option<&str>,
        action: PermissionAction,
        effect: PolicyEffect,
        condition: Option<&str>,
    ) -> PermissionPolicy {
        PermissionPolicy {
            id: id.to_string(),
            map_pattern: map_pattern.to_string(),
            key_pattern: key_pattern.map(str::to_string),
            action,
            effect,
            condition: condition.map(|e| {
                expr_parser::parse_permission_expr(e).unwrap_or_else(|err| panic!("'{e}': {err}"))
            }),
        }
    }

    fn allow_all() -> PermissionPolicy {
        rule(
            "allow-all",
            "*",
            None,
            PermissionAction::All,
            PolicyEffect::Allow,
            None,
        )
    }

    fn configured(policies: Vec<PermissionPolicy>) -> PolicyEvaluator {
        let store = make_store_with(policies);
        store
            .configured
            .store(true, std::sync::atomic::Ordering::SeqCst);
        PolicyEvaluator::new(store)
    }

    async fn read(
        eval: &PolicyEvaluator,
        who: Option<&topgun_core::Principal>,
        map: &str,
    ) -> PolicyDecision {
        eval.evaluate(who, PermissionAction::Read, map, &rmpv::Value::Nil)
            .await
    }

    async fn write_key(
        eval: &PolicyEvaluator,
        who: Option<&topgun_core::Principal>,
        map: &str,
        key: Option<&str>,
    ) -> PolicyDecision {
        eval.evaluate_key(who, PermissionAction::Write, map, key, &rmpv::Value::Nil)
            .await
    }

    use PolicyDecision::{Allow as Allowed, Deny as Denied};

    #[tokio::test]
    async fn placeholder_segment_matches_only_the_principals_own_whole_segment() {
        let eval = configured(vec![rule(
            "conv",
            "t.conv.{auth.id}",
            None,
            PermissionAction::Read,
            PolicyEffect::Allow,
            None,
        )]);
        let alice = tenanted("alice", None);
        assert_eq!(read(&eval, Some(&alice), "t.conv.alice").await, Allowed);
        assert_eq!(read(&eval, Some(&alice), "t.conv.bob").await, Denied);
        assert_eq!(read(&eval, Some(&alice), "t.conv.alice.x").await, Denied);
        let dotted = tenanted("a.b", None);
        assert_eq!(read(&eval, Some(&dotted), "t.conv.a.b").await, Denied);
        let star = tenanted("*", None);
        assert_eq!(read(&eval, Some(&star), "t.conv.*").await, Allowed);
        assert_eq!(read(&eval, Some(&star), "t.conv.bob").await, Denied);
    }

    #[tokio::test]
    async fn unfilled_placeholder_never_widens_an_allow_and_narrows_no_deny() {
        // An allow with a placeholder admits no missing principal and no empty id.
        let allow = configured(vec![rule(
            "conv",
            "t.conv.{auth.id}",
            None,
            PermissionAction::Read,
            PolicyEffect::Allow,
            None,
        )]);
        assert_eq!(read(&allow, None, "t.conv.x").await, Denied);
        let empty = tenanted("", None);
        assert_eq!(read(&allow, Some(&empty), "t.conv.").await, Denied);

        // A deny's unfilled segment matches as exactly one segment.
        let deny = configured(vec![
            allow_all(),
            rule(
                "secret",
                "t.{auth.id}.secret",
                None,
                PermissionAction::All,
                PolicyEffect::Deny,
                None,
            ),
        ]);
        let dotted = tenanted("a.b", None);
        assert_eq!(read(&deny, Some(&dotted), "t.x.secret").await, Denied);
        assert_eq!(read(&deny, Some(&dotted), "t.x.y.secret").await, Allowed);
        assert_eq!(read(&deny, None, "t.x.secret").await, Denied);
        assert_eq!(read(&deny, None, "t.x.public").await, Allowed);
    }

    #[tokio::test]
    async fn unfilled_key_pattern_in_a_deny_denies_every_key() {
        let deny = configured(vec![
            allow_all(),
            rule(
                "presence",
                "t.presence",
                Some("{auth.id}:*"),
                PermissionAction::Write,
                PolicyEffect::Deny,
                None,
            ),
        ]);
        let colon = tenanted("a:b", None);
        for key in ["a:b:1", "a:1", "x", "b:1"] {
            assert_eq!(
                write_key(&deny, Some(&colon), "t.presence", Some(key)).await,
                Denied,
                "{key}"
            );
        }
        assert_eq!(
            write_key(&deny, Some(&colon), "t.other", Some("x")).await,
            Allowed
        );
        // A filled deny reaches only the principal's own keys.
        let alice = tenanted("alice", None);
        assert_eq!(
            write_key(&deny, Some(&alice), "t.presence", Some("alice:1")).await,
            Denied
        );
        assert_eq!(
            write_key(&deny, Some(&alice), "t.presence", Some("bob:1")).await,
            Allowed
        );

        // The matching allow never matches for that principal.
        let allow = configured(vec![rule(
            "presence",
            "t.presence",
            Some("{auth.id}:*"),
            PermissionAction::Write,
            PolicyEffect::Allow,
            None,
        )]);
        for key in ["a:b:1", "a:1", "a:b"] {
            assert_eq!(
                write_key(&allow, Some(&colon), "t.presence", Some(key)).await,
                Denied,
                "{key}"
            );
        }
        assert_eq!(
            write_key(&allow, Some(&alice), "t.presence", Some("alice:tab1")).await,
            Allowed
        );
    }

    #[test]
    fn unfilled_placeholder_logs_once_per_policy() {
        use futures_util::FutureExt;
        use std::sync::{Mutex, PoisonError};

        #[derive(Clone)]
        struct CapturedLog(Arc<Mutex<Vec<u8>>>);
        impl std::io::Write for CapturedLog {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let captured: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let writer = CapturedLog(Arc::clone(&captured));
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_ansi(false)
            .finish();
        // Scoped, never global; a second live dispatcher makes every thread ask
        // all subscribers, so another test reaching the same `warn!` first
        // cannot settle its interest as "nobody listens".
        let dispatch = tracing::Dispatch::new(subscriber);
        let _second_dispatcher = tracing::Dispatch::new(tracing::subscriber::NoSubscriber::new());

        // The id is used by no other test: the logged set is process-wide.
        let store = make_store_with(vec![rule(
            "log-once",
            "t.conv.{auth.id}",
            None,
            PermissionAction::Read,
            PolicyEffect::Allow,
            None,
        )]);
        store
            .configured
            .store(true, std::sync::atomic::Ordering::SeqCst);
        // Two evaluators over one store, as two HTTP `/sync` requests build them.
        let evaluators = [
            PolicyEvaluator::new(store.clone()),
            PolicyEvaluator::new(store),
        ];
        let colon = tenanted("a.b", None);
        tracing::dispatcher::with_default(&dispatch, || {
            for eval in &evaluators {
                for _ in 0..2 {
                    let decision = read(eval, Some(&colon), "t.conv.a")
                        .now_or_never()
                        .expect("an in-memory store answers at once");
                    assert_eq!(decision, Denied);
                }
            }
        });
        let log = String::from_utf8(
            captured
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        )
        .expect("utf-8 log");
        let warnings: Vec<&str> = log
            .lines()
            .filter(|l| l.contains("policy log-once: {auth.id} has no single-segment value"))
            .collect();
        assert_eq!(warnings.len(), 1, "{log}");
    }

    #[tokio::test]
    async fn auth_value_carries_tenant() {
        let acme = tenanted("v1", Some("acme"));
        let globex = tenanted("v1", Some("globex"));
        let none = tenanted("v1", None);
        let allow = configured(vec![rule(
            "tenant",
            "*",
            None,
            PermissionAction::Write,
            PolicyEffect::Allow,
            Some("auth.tenant == 'acme'"),
        )]);
        assert_eq!(write_key(&allow, Some(&acme), "m", None).await, Allowed);
        assert_eq!(write_key(&allow, Some(&globex), "m", None).await, Denied);
        assert_eq!(write_key(&allow, Some(&none), "m", None).await, Denied);
        let forged_tenant = forged("tenant", s("acme"));
        assert_eq!(
            allow
                .evaluate(Some(&none), PermissionAction::Write, "m", &forged_tenant)
                .await,
            Denied
        );

        let deny = configured(vec![
            allow_all(),
            rule(
                "not-acme",
                "*",
                None,
                PermissionAction::Write,
                PolicyEffect::Deny,
                Some("auth.tenant != 'acme'"),
            ),
        ]);
        assert_eq!(write_key(&deny, Some(&acme), "m", None).await, Allowed);
        assert_eq!(write_key(&deny, Some(&globex), "m", None).await, Denied);
        assert_eq!(write_key(&deny, Some(&none), "m", None).await, Denied);
    }

    fn guest(id: &str) -> topgun_core::Principal {
        topgun_core::Principal {
            kind: topgun_core::types::PrincipalKind::Guest,
            ..tenanted(id, Some("acme"))
        }
    }

    #[tokio::test]
    async fn guest_kind_reaches_policy_conditions() {
        let alice = tenanted("alice", Some("acme"));
        let visitor = guest("v1");
        let deny = configured(vec![
            allow_all(),
            rule(
                "no-guest-writes",
                "**",
                None,
                PermissionAction::Write,
                PolicyEffect::Deny,
                Some("auth.kind == 'guest'"),
            ),
        ]);
        // The record cannot speak for the caller's kind.
        let forged_user = forged("kind", s("user"));
        assert_eq!(
            deny.evaluate(
                Some(&visitor),
                PermissionAction::Write,
                "acme.m",
                &forged_user
            )
            .await,
            Denied
        );
        assert_eq!(
            write_key(&deny, Some(&visitor), "acme.m", Some("k")).await,
            Denied
        );
        assert_eq!(
            write_key(&deny, Some(&alice), "acme.m", Some("k")).await,
            Allowed
        );
        // Without a principal `auth.kind` stays unknown, so the deny applies.
        assert_eq!(write_key(&deny, None, "acme.m", None).await, Denied);

        let allow = configured(vec![rule(
            "guest-only",
            "*",
            None,
            PermissionAction::Write,
            PolicyEffect::Allow,
            Some("auth.kind == 'guest'"),
        )]);
        let forged_guest = forged("kind", s("guest"));
        assert_eq!(
            allow
                .evaluate(Some(&alice), PermissionAction::Write, "m", &forged_guest)
                .await,
            Denied
        );
        assert_eq!(write_key(&allow, Some(&visitor), "m", None).await, Allowed);
    }

    #[tokio::test]
    async fn admin_bypass_applies_only_to_users() {
        let admin_user = tenanted("ops", None);
        let admin_guest = topgun_core::Principal {
            kind: topgun_core::types::PrincipalKind::Guest,
            ..tenanted("ops", None)
        };
        let actions = [
            PermissionAction::Read,
            PermissionAction::Write,
            PermissionAction::Remove,
            PermissionAction::All,
        ];
        // No policy at all: only the bypass can allow.
        let eval =
            PolicyEvaluator::with_admin_subjects(make_store_with(vec![]), admin_subjects(&["ops"]));
        for action in actions {
            assert_eq!(
                eval.evaluate(Some(&admin_user), action, "any.map", &rmpv::Value::Nil)
                    .await,
                Allowed,
                "{action:?}: an admin user bypasses"
            );
            assert_eq!(
                eval.evaluate(Some(&admin_guest), action, "any.map", &rmpv::Value::Nil)
                    .await,
                Denied,
                "{action:?}: a guest with an admin subject is held to the policies"
            );
        }
        // With a policy that matches, the guest gets exactly what it grants.
        let store = make_store_with(vec![rule(
            "read-only",
            "*",
            None,
            PermissionAction::Read,
            PolicyEffect::Allow,
            None,
        )]);
        let eval = PolicyEvaluator::with_admin_subjects(store, admin_subjects(&["ops"]));
        assert_eq!(read(&eval, Some(&admin_guest), "m").await, Allowed);
        assert_eq!(
            write_key(&eval, Some(&admin_guest), "m", None).await,
            Denied
        );
        assert_eq!(
            write_key(&eval, Some(&admin_user), "m", None).await,
            Allowed
        );
    }

    #[tokio::test]
    async fn a_condition_on_an_unknown_namespace_fails_closed() {
        let alice = tenanted("alice", None);
        let x = rec(vec![("x", rmpv::Value::Integer(1.into()))]);
        let cases: Vec<(Vec<PermissionPolicy>, rmpv::Value, PolicyDecision)> = vec![
            (
                vec![
                    allow_all(),
                    rule(
                        "d",
                        "*",
                        None,
                        PermissionAction::Write,
                        PolicyEffect::Deny,
                        Some("data.x == foo.y"),
                    ),
                ],
                x.clone(),
                Denied,
            ),
            (
                vec![rule(
                    "a",
                    "*",
                    None,
                    PermissionAction::Write,
                    PolicyEffect::Allow,
                    Some("data.x == foo.y"),
                )],
                x.clone(),
                Denied,
            ),
            (
                vec![rule(
                    "a",
                    "*",
                    None,
                    PermissionAction::Write,
                    PolicyEffect::Allow,
                    Some("!(resource.to == 'x')"),
                )],
                x.clone(),
                Denied,
            ),
            (
                vec![
                    allow_all(),
                    rule(
                        "d",
                        "*",
                        None,
                        PermissionAction::Write,
                        PolicyEffect::Deny,
                        Some("data.a == 1 && data.b == foo.y"),
                    ),
                ],
                rec(vec![("a", rmpv::Value::Integer(2.into()))]),
                Allowed,
            ),
        ];
        for (i, (policies, data, expected)) in cases.into_iter().enumerate() {
            let eval = configured(policies);
            assert_eq!(
                eval.evaluate(Some(&alice), PermissionAction::Write, "m", &data)
                    .await,
                expected,
                "case {i}"
            );
        }
    }

    #[tokio::test]
    async fn tenant_placeholder_is_filled_from_the_principal_not_the_name() {
        let eval = configured(vec![rule(
            "visitor",
            "{auth.tenant}.visitor.{auth.id}.**",
            None,
            PermissionAction::All,
            PolicyEffect::Allow,
            None,
        )]);
        let acme_v1 = tenanted("v1", Some("acme"));
        let no_tenant = tenanted("v1", None);
        for (who, map, expected) in [
            (&acme_v1, "acme.visitor.v1.cart", Allowed),
            (&acme_v1, "globex.visitor.v1.cart", Denied),
            (&acme_v1, "acme.visitor.v2.cart", Denied),
            (&no_tenant, "acme.visitor.v1.cart", Denied),
        ] {
            assert_eq!(read(&eval, Some(who), map).await, expected, "read {map}");
            assert_eq!(
                write_key(&eval, Some(who), map, Some("k")).await,
                expected,
                "write {map}"
            );
        }
    }

    /// The persisted shape of a policy before placeholders existed.
    #[derive(Debug, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct EarlierPolicy {
        id: String,
        map_pattern: String,
        action: PermissionAction,
        effect: PolicyEffect,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        condition: Option<PredicateNode>,
    }

    fn decode(bytes: &[u8]) -> Result<PermissionPolicy, String> {
        rmp_serde::from_slice::<PermissionPolicy>(bytes).map_err(|e| e.to_string())
    }

    #[test]
    fn scoped_policies_are_unreadable_to_older_binaries_and_plain_ones_unchanged() {
        // (a) A plain policy encodes exactly as before.
        for condition in [None, Some("data.ownerId == auth.id")] {
            let plain = rule(
                "plain",
                "users.**",
                None,
                PermissionAction::Write,
                PolicyEffect::Allow,
                condition,
            );
            let earlier = EarlierPolicy {
                id: plain.id.clone(),
                map_pattern: plain.map_pattern.clone(),
                action: plain.action,
                effect: plain.effect,
                condition: plain.condition.clone(),
            };
            assert_eq!(
                rmp_serde::to_vec_named(&plain).unwrap(),
                rmp_serde::to_vec_named(&earlier).unwrap()
            );
        }

        // (b) A scoped row does not decode into the earlier shape.
        for scoped in [
            rule(
                "s1",
                "t.{auth.id}",
                None,
                PermissionAction::Read,
                PolicyEffect::Allow,
                None,
            ),
            rule(
                "s2",
                "t.presence",
                Some("{auth.id}:*"),
                PermissionAction::Write,
                PolicyEffect::Deny,
                None,
            ),
        ] {
            let bytes = rmp_serde::to_vec_named(&scoped).unwrap();
            let err = rmp_serde::from_slice::<EarlierPolicy>(&bytes).unwrap_err();
            assert!(err.to_string().contains("mapPattern"), "{err}");

            // (d) It round-trips through this version.
            let back = decode(&bytes).unwrap();
            assert_eq!(format!("{back:?}"), format!("{scoped:?}"));
        }

        // (c) A plain row with a brace is refused, not read as a template.
        let legacy = rmp_serde::to_vec_named(&EarlierPolicy {
            id: "legacy".to_string(),
            map_pattern: "t.{auth.id}".to_string(),
            action: PermissionAction::Read,
            effect: PolicyEffect::Allow,
            condition: None,
        })
        .unwrap();
        assert!(decode(&legacy).is_err());

        // (e) A scoped row with a keyPattern on a read rule is refused.
        let read_key = rmp_serde::to_vec_named(&StoredPolicy {
            id: "rk".to_string(),
            map_pattern: None,
            scoped_map_pattern: Some("t.presence".to_string()),
            action: PermissionAction::Read,
            effect: PolicyEffect::Allow,
            condition: None,
            key_pattern: Some("{auth.id}:*".to_string()),
        })
        .unwrap();
        assert!(decode(&read_key).is_err());

        // (f) A scoped {auth.tenant} row loads whatever the tenancy setting.
        let tenant = rule(
            "tenant",
            "{auth.tenant}.visitor.**",
            None,
            PermissionAction::Read,
            PolicyEffect::Allow,
            None,
        );
        assert!(decode(&rmp_serde::to_vec_named(&tenant).unwrap()).is_ok());
    }

    #[test]
    fn stored_conditions_on_unknown_namespaces_load_with_one_warning() {
        let stored = |id: &str, expr: &str| {
            let bytes = rmp_serde::to_vec_named(&EarlierPolicy {
                id: id.to_string(),
                map_pattern: "m".to_string(),
                action: PermissionAction::Write,
                effect: PolicyEffect::Deny,
                condition: Some(expr_parser::parse_permission_expr(expr).unwrap()),
            })
            .unwrap();
            decode(&bytes).unwrap_or_else(|e| panic!("{id} must load: {e}"))
        };
        for (id, expr, reference) in [
            ("ns", "data.x == foo.y", "foo.y"),
            ("rx", "resource.x == 'v'", "resource.x"),
            ("rmap", "resource.map == 'm'", "resource.map"),
            ("bare", "resource == 'x'", "resource"),
        ] {
            let warning = condition_meaning_change(&stored(id, expr))
                .unwrap_or_else(|| panic!("{id} must warn"));
            assert!(
                warning.starts_with(&format!("policy {id}: {reference} ")),
                "{warning}"
            );
        }
        assert_eq!(
            condition_meaning_change(&stored("plain", "data.x == auth.id")),
            None
        );
    }
}
