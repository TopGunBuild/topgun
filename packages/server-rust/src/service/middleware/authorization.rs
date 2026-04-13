//! Authorization middleware — RBAC policy enforcement for the operation pipeline.
//!
//! Intercepts every client `Operation` before it reaches domain services and
//! checks it against the configured `PolicyEvaluator`. Operations from trusted
//! origins (Forwarded, System, Backup, Wan) bypass policy evaluation entirely.
//! When no policies are configured the middleware passes all operations through,
//! preserving backward compatibility with deployments that do not use RBAC.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use tower::{Layer, Service};

use crate::service::operation::{CallerOrigin, Operation, OperationError, OperationResponse};
use crate::service::policy::{PermissionAction, PolicyDecision, PolicyEvaluator};

// ---------------------------------------------------------------------------
// AuthorizationLayer
// ---------------------------------------------------------------------------

/// Tower `Layer` that wraps an inner service with RBAC policy enforcement.
///
/// When `PolicyEvaluator` is provided, every `CallerOrigin::Client` operation
/// is checked before being forwarded to the inner service. A `None` evaluator
/// means RBAC is not configured — the layer is a no-op passthrough.
#[derive(Clone)]
pub struct AuthorizationLayer {
    evaluator: Arc<PolicyEvaluator>,
}

impl AuthorizationLayer {
    /// Creates a new layer with the given policy evaluator.
    #[must_use]
    pub fn new(evaluator: Arc<PolicyEvaluator>) -> Self {
        Self { evaluator }
    }
}

impl<S> Layer<S> for AuthorizationLayer {
    type Service = AuthorizationService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        AuthorizationService {
            inner,
            evaluator: Arc::clone(&self.evaluator),
        }
    }
}

// ---------------------------------------------------------------------------
// AuthorizationService
// ---------------------------------------------------------------------------

/// Tower `Service` produced by `AuthorizationLayer`.
///
/// Holds a reference to the `PolicyEvaluator` (shared across all workers via
/// `Arc`). All transport handlers set `ctx.principal` eagerly before pipeline
/// dispatch, so the middleware reads only `ctx.principal`.
pub struct AuthorizationService<S> {
    inner: S,
    evaluator: Arc<PolicyEvaluator>,
}

impl<S> AuthorizationService<S>
where
    S: Service<Operation, Response = OperationResponse, Error = OperationError> + Send + 'static,
    S::Future: Send + 'static,
{
    /// Evaluates policies with a pre-resolved principal.
    fn call_with_principal(
        &mut self,
        op: Operation,
        principal: Option<topgun_core::Principal>,
    ) -> Pin<Box<dyn Future<Output = Result<OperationResponse, OperationError>> + Send>> {
        let evaluator = Arc::clone(&self.evaluator);
        let (action, map_name) = classify_operation(&op);
        let Some(action) = action else {
            return Box::pin(self.inner.call(op));
        };
        let batch_ops_data = extract_batch_ops_data(&op);
        let data = extract_data(&op);
        let fut = self.inner.call(op);
        Box::pin(async move {
            if !evaluator.has_policies().await {
                return fut.await;
            }
            evaluate_and_dispatch(
                evaluator,
                principal,
                action,
                map_name,
                batch_ops_data,
                data,
                fut,
            )
            .await
        })
    }
}

impl<S> Service<Operation> for AuthorizationService<S>
where
    S: Service<Operation, Response = OperationResponse, Error = OperationError> + Send + 'static,
    S::Future: Send + 'static,
{
    type Response = OperationResponse;
    type Error = OperationError;
    type Future = Pin<Box<dyn Future<Output = Result<OperationResponse, OperationError>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, op: Operation) -> Self::Future {
        let ctx = op.ctx();
        let caller_origin = ctx.caller_origin;
        // Trusted origins bypass policy evaluation entirely.
        if !matches!(
            caller_origin,
            CallerOrigin::Client | CallerOrigin::HttpClient | CallerOrigin::Anonymous
        ) {
            return Box::pin(self.inner.call(op));
        }

        // Anonymous callers have no principal; pass None through RBAC evaluation.
        if caller_origin == CallerOrigin::Anonymous {
            return self.call_with_principal(op, None);
        }

        // All transport handlers (WebSocket, HTTP) set ctx.principal eagerly before
        // pipeline dispatch, so the middleware reads only ctx.principal.
        let Some(principal) = ctx.principal.clone() else {
            return Box::pin(async { Err(OperationError::Unauthorized) });
        };

        self.call_with_principal(op, Some(principal))
    }
}

// ---------------------------------------------------------------------------
// Operation classification helpers
// ---------------------------------------------------------------------------

/// Extracts per-op `(map_name, data)` pairs for `OpBatch` operations.
fn extract_batch_ops_data(op: &Operation) -> Option<Vec<(String, rmpv::Value)>> {
    match op {
        Operation::OpBatch { payload, .. } => Some(
            payload
                .payload
                .ops
                .iter()
                .map(|client_op| (client_op.map_name.clone(), extract_op_data(client_op)))
                .collect(),
        ),
        _ => None,
    }
}

/// Evaluates policies against a resolved principal and dispatches to the inner service.
async fn evaluate_and_dispatch<F>(
    evaluator: Arc<PolicyEvaluator>,
    principal: Option<topgun_core::Principal>,
    action: PermissionAction,
    map_name: String,
    batch_ops_data: Option<Vec<(String, rmpv::Value)>>,
    data: rmpv::Value,
    fut: F,
) -> Result<OperationResponse, OperationError>
where
    F: Future<Output = Result<OperationResponse, OperationError>>,
{
    // For OpBatch, evaluate each op individually. If any op is denied,
    // reject the entire batch (fail-closed atomicity).
    if let Some(ops_data) = batch_ops_data {
        for (op_map_name, op_data) in &ops_data {
            let decision = evaluator
                .evaluate(principal.as_ref(), action, op_map_name, op_data)
                .await;
            if decision == PolicyDecision::Deny {
                return Err(OperationError::Forbidden {
                    map_name: op_map_name.clone(),
                });
            }
        }
        return fut.await;
    }

    let decision = evaluator
        .evaluate(principal.as_ref(), action, &map_name, &data)
        .await;
    match decision {
        PolicyDecision::Allow => fut.await,
        PolicyDecision::Deny => Err(OperationError::Forbidden { map_name }),
    }
}

/// Maps an `Operation` variant to a `PermissionAction` and a `map_name`.
///
/// Returns `(None, _)` for operations in the bypass group — these pass through
/// without any policy evaluation (e.g., Ping, `PartitionMapRequest`, `GarbageCollect`).
fn classify_operation(op: &Operation) -> (Option<PermissionAction>, String) {
    match op {
        // --- Write actions ---
        Operation::ClientOp { payload, .. } => (
            Some(PermissionAction::Write),
            payload.payload.map_name.clone(),
        ),
        Operation::OpBatch { payload, .. } => {
            // Use the map_name from the first op in the batch; fall back to empty.
            let map_name = payload
                .payload
                .ops
                .first()
                .map(|op| op.map_name.clone())
                .unwrap_or_default();
            (Some(PermissionAction::Write), map_name)
        }
        Operation::TopicPublish { payload, .. } => {
            // Topics use a topic name rather than a map_name; use topic as the resource.
            (Some(PermissionAction::Write), payload.topic.clone())
        }
        Operation::EntryProcess { payload, .. } => {
            (Some(PermissionAction::Write), payload.map_name.clone())
        }
        Operation::EntryProcessBatch { payload, .. } => {
            (Some(PermissionAction::Write), payload.map_name.clone())
        }
        Operation::CounterSync { payload, .. } => {
            // Counters use a name field; treat as the resource identifier.
            (Some(PermissionAction::Write), payload.name.clone())
        }
        Operation::ORMapPushDiff { payload, .. } => (
            Some(PermissionAction::Write),
            payload.payload.map_name.clone(),
        ),

        // --- Read actions ---
        Operation::QuerySubscribe { payload, .. } | Operation::DagQuery { payload, .. } => (
            Some(PermissionAction::Read),
            payload.payload.map_name.clone(),
        ),
        // QuerySyncInit resumes by query_id; SqlQuery and VectorSearch may span multiple maps.
        Operation::QuerySyncInit { .. }
        | Operation::SqlQuery { .. }
        | Operation::VectorSearch { .. } => (Some(PermissionAction::Read), String::new()),
        Operation::Search { payload, .. } => {
            (Some(PermissionAction::Read), payload.map_name.clone())
        }
        Operation::SearchSubscribe { payload, .. } => {
            (Some(PermissionAction::Read), payload.map_name.clone())
        }
        Operation::SyncInit { payload, .. } => {
            // SyncInitMessage is flat (no payload wrapper).
            (Some(PermissionAction::Read), payload.map_name.clone())
        }
        Operation::MerkleReqBucket { payload, .. } => (
            Some(PermissionAction::Read),
            payload.payload.map_name.clone(),
        ),
        Operation::ORMapSyncInit { payload, .. } => {
            // ORMapSyncInit is flat (no payload wrapper).
            (Some(PermissionAction::Read), payload.map_name.clone())
        }
        Operation::ORMapMerkleReqBucket { payload, .. } => (
            Some(PermissionAction::Read),
            payload.payload.map_name.clone(),
        ),
        Operation::ORMapDiffRequest { payload, .. } => (
            Some(PermissionAction::Read),
            payload.payload.map_name.clone(),
        ),

        Operation::HybridSearch { payload, .. } => {
            (Some(PermissionAction::Read), payload.map_name.clone())
        }
        Operation::HybridSearchSubscribe { payload, .. } => {
            (Some(PermissionAction::Read), payload.map_name.clone())
        }

        // --- Bypass group (no policy check) ---
        Operation::Ping { .. }
        | Operation::PartitionMapRequest { .. }
        | Operation::GarbageCollect { .. }
        | Operation::LockRequest { .. }
        | Operation::LockRelease { .. }
        | Operation::TopicSubscribe { .. }
        | Operation::TopicUnsubscribe { .. }
        | Operation::QueryUnsubscribe { .. }
        | Operation::SearchUnsubscribe { .. }
        | Operation::HybridSearchUnsubscribe { .. }
        | Operation::JournalSubscribe { .. }
        | Operation::JournalUnsubscribe { .. }
        | Operation::JournalRead { .. }
        | Operation::RegisterResolver { .. }
        | Operation::UnregisterResolver { .. }
        | Operation::ListResolvers { .. }
        | Operation::CounterRequest { .. } => (None, String::new()),
    }
}

/// Extracts the LWW record value from a single `ClientOp`.
///
/// Returns the record's value if present, or `Nil` for tombstones (deleted
/// records where value is `None`) and operations without record data.
fn extract_op_data(op: &topgun_core::messages::base::ClientOp) -> rmpv::Value {
    op.record
        .as_ref()
        .and_then(|outer| outer.as_ref())
        .and_then(|r| r.value.clone())
        .unwrap_or(rmpv::Value::Nil)
}

/// Extracts record data from write operations for record-level condition evaluation.
///
/// For `ClientOp` operations the record value is extracted via `extract_op_data`.
/// For `OpBatch`, returns `Nil` — per-op evaluation handles individual ops directly.
/// For all other operations `rmpv::Value::Nil` is returned because either no
/// record data is present (reads, meta-ops) or extraction is not meaningful
/// at the middleware level.
fn extract_data(op: &Operation) -> rmpv::Value {
    match op {
        Operation::ClientOp { payload, .. } => extract_op_data(&payload.payload),
        _ => rmpv::Value::Nil,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::doc_markdown)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::task::{Context, Poll};

    use topgun_core::Timestamp;
    use tower::{Layer, Service, ServiceExt};

    use super::*;
    use crate::service::operation::{service_names, OperationContext};
    use crate::service::policy::{InMemoryPolicyStore, PolicyStore};

    /// Stub inner service that always succeeds.
    struct AlwaysOkService;

    impl Service<Operation> for AlwaysOkService {
        type Response = OperationResponse;
        type Error = OperationError;
        type Future =
            Pin<Box<dyn Future<Output = Result<OperationResponse, OperationError>> + Send>>;

        fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, op: Operation) -> Self::Future {
            let call_id = op.ctx().call_id;
            let name = op.ctx().service_name;
            Box::pin(async move {
                Ok(OperationResponse::NotImplemented {
                    service_name: name,
                    call_id,
                })
            })
        }
    }

    fn make_timestamp() -> Timestamp {
        Timestamp {
            millis: 0,
            counter: 0,
            node_id: "test".to_string(),
        }
    }

    /// Builds a Ping operation with the given `CallerOrigin`.
    fn ping_op(origin: CallerOrigin) -> Operation {
        let mut ctx = OperationContext::new(1, service_names::COORDINATION, make_timestamp(), 5000);
        ctx.caller_origin = origin;
        Operation::Ping {
            ctx,
            payload: topgun_core::messages::PingData { timestamp: 0 },
        }
    }

    /// Trusted origins (Forwarded, System, Backup, Wan) must pass through without
    /// calling the evaluator — even when policies exist. This is verified by
    /// using a PolicyEvaluator backed by a store with a Deny-all policy and
    /// confirming the operation still succeeds.
    #[tokio::test]
    async fn trusted_origin_bypasses_policy_evaluation() {
        use crate::service::policy::{PermissionPolicy, PolicyEffect};

        let store = Arc::new(InMemoryPolicyStore::new());
        // Deny-all policy to detect any call to evaluate().
        store
            .upsert_policy(PermissionPolicy {
                id: "deny-all".to_string(),
                map_pattern: "*".to_string(),
                action: crate::service::policy::PermissionAction::All,
                effect: PolicyEffect::Deny,
                condition: None,
            })
            .await
            .unwrap();

        let evaluator = Arc::new(PolicyEvaluator::new(store));

        let layer = AuthorizationLayer::new(evaluator);
        let mut svc = layer.layer(AlwaysOkService);

        for origin in [
            CallerOrigin::Forwarded,
            CallerOrigin::System,
            CallerOrigin::Backup,
            CallerOrigin::Wan,
        ] {
            let op = ping_op(origin);
            let resp = ServiceExt::ready(&mut svc).await.unwrap().call(op).await;
            assert!(
                resp.is_ok(),
                "trusted origin {origin:?} should bypass policy evaluation but got {resp:?}"
            );
        }
    }

    /// When no policies are configured (empty store), client operations pass through.
    #[tokio::test]
    async fn no_policies_allows_all() {
        let store = Arc::new(InMemoryPolicyStore::new());
        let evaluator = Arc::new(PolicyEvaluator::new(store));

        let layer = AuthorizationLayer::new(evaluator);
        let mut svc = layer.layer(AlwaysOkService);

        let mut ctx = OperationContext::new(2, service_names::COORDINATION, make_timestamp(), 5000);
        ctx.caller_origin = CallerOrigin::Client;
        ctx.principal = Some(Principal {
            id: "user1".to_string(),
            roles: vec![],
        });
        let op = Operation::Ping {
            ctx,
            payload: topgun_core::messages::PingData { timestamp: 0 },
        };

        let resp = ServiceExt::ready(&mut svc).await.unwrap().call(op).await;
        assert!(
            resp.is_ok(),
            "empty store should allow all ops, got {resp:?}"
        );
    }

    /// Missing principal on a Client-origin operation returns Unauthorized.
    #[tokio::test]
    async fn missing_principal_returns_unauthorized() {
        let store = Arc::new(InMemoryPolicyStore::new());
        let evaluator = Arc::new(PolicyEvaluator::new(store));

        let layer = AuthorizationLayer::new(evaluator);
        let mut svc = layer.layer(AlwaysOkService);

        let mut ctx = OperationContext::new(3, service_names::COORDINATION, make_timestamp(), 5000);
        ctx.caller_origin = CallerOrigin::Client;
        // principal deliberately left as None
        let op = Operation::Ping {
            ctx,
            payload: topgun_core::messages::PingData { timestamp: 0 },
        };

        let result = ServiceExt::ready(&mut svc).await.unwrap().call(op).await;
        assert!(
            matches!(result, Err(OperationError::Unauthorized)),
            "missing principal should return Unauthorized, got {result:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Record-level condition tests (owner restriction, tombstone, batch atomicity)
    // -----------------------------------------------------------------------

    use crate::service::policy::{
        expr_parser::parse_permission_expr, PermissionPolicy, PolicyEffect,
    };
    use topgun_core::{LWWRecord, Principal};

    /// Helper: builds a policy with `auth.id == data.ownerId` condition.
    fn owner_condition_policy() -> PermissionPolicy {
        PermissionPolicy {
            id: "owner-write".to_string(),
            map_pattern: "*".to_string(),
            action: PermissionAction::Write,
            effect: PolicyEffect::Allow,
            condition: Some(
                parse_permission_expr("auth.id == data.ownerId")
                    .expect("owner condition should parse"),
            ),
        }
    }

    /// Helper: builds a ClientOp Operation with a record containing the given ownerId value.
    /// The principal is set eagerly on ctx so the authorization middleware can read it directly.
    fn client_op_with_owner(principal: Principal, owner_id: &str) -> Operation {
        let record = LWWRecord {
            value: Some(rmpv::Value::Map(vec![(
                rmpv::Value::String("ownerId".into()),
                rmpv::Value::String(owner_id.into()),
            )])),
            timestamp: make_timestamp(),
            ttl_ms: None,
        };
        let mut ctx = OperationContext::new(10, service_names::CRDT, make_timestamp(), 5000);
        ctx.caller_origin = CallerOrigin::Client;
        ctx.principal = Some(principal);
        Operation::ClientOp {
            ctx,
            payload: topgun_core::messages::sync::ClientOpMessage {
                payload: topgun_core::messages::base::ClientOp {
                    map_name: "docs".to_string(),
                    key: "doc1".to_string(),
                    record: Some(Some(record)),
                    ..Default::default()
                },
            },
        }
    }

    /// Helper: builds a ClientOp Operation representing a tombstone (deleted record).
    /// The principal is set eagerly on ctx so the authorization middleware can read it directly.
    fn client_op_tombstone(principal: Principal) -> Operation {
        let record = LWWRecord {
            value: None,
            timestamp: make_timestamp(),
            ttl_ms: None,
        };
        let mut ctx = OperationContext::new(11, service_names::CRDT, make_timestamp(), 5000);
        ctx.caller_origin = CallerOrigin::Client;
        ctx.principal = Some(principal);
        Operation::ClientOp {
            ctx,
            payload: topgun_core::messages::sync::ClientOpMessage {
                payload: topgun_core::messages::base::ClientOp {
                    map_name: "docs".to_string(),
                    key: "doc1".to_string(),
                    record: Some(Some(record)),
                    ..Default::default()
                },
            },
        }
    }

    /// Helper: builds an OpBatch Operation with the given list of (map_name, owner_id) pairs.
    /// The principal is set eagerly on ctx so the authorization middleware can read it directly.
    fn op_batch_with_owners(principal: Principal, ops: &[(&str, &str)]) -> Operation {
        let client_ops: Vec<topgun_core::messages::base::ClientOp> = ops
            .iter()
            .enumerate()
            .map(|(i, (map_name, owner_id))| {
                let record = LWWRecord {
                    value: Some(rmpv::Value::Map(vec![(
                        rmpv::Value::String("ownerId".into()),
                        rmpv::Value::String((*owner_id).into()),
                    )])),
                    timestamp: make_timestamp(),
                    ttl_ms: None,
                };
                topgun_core::messages::base::ClientOp {
                    map_name: (*map_name).to_string(),
                    key: format!("key{i}"),
                    record: Some(Some(record)),
                    ..Default::default()
                }
            })
            .collect();

        let mut ctx = OperationContext::new(12, service_names::CRDT, make_timestamp(), 5000);
        ctx.caller_origin = CallerOrigin::Client;
        ctx.principal = Some(principal);
        Operation::OpBatch {
            ctx,
            payload: topgun_core::messages::sync::OpBatchMessage {
                payload: topgun_core::messages::sync::OpBatchPayload {
                    ops: client_ops,
                    ..Default::default()
                },
            },
        }
    }

    /// Owner condition allows a write when the record's ownerId matches auth.id.
    #[tokio::test]
    async fn owner_condition_allows_matching_owner() {
        let store = Arc::new(InMemoryPolicyStore::new());
        store.upsert_policy(owner_condition_policy()).await.unwrap();

        let evaluator = Arc::new(PolicyEvaluator::new(store));
        let principal = Principal {
            id: "user1".to_string(),
            roles: vec!["user".to_string()],
        };

        let layer = AuthorizationLayer::new(evaluator);
        let mut svc = layer.layer(AlwaysOkService);

        // Record ownerId matches the principal's id
        let op = client_op_with_owner(principal, "user1");
        let result = ServiceExt::ready(&mut svc).await.unwrap().call(op).await;
        assert!(
            result.is_ok(),
            "matching owner should be allowed, got {result:?}"
        );
    }

    /// Owner condition denies a write when the record's ownerId does NOT match auth.id.
    #[tokio::test]
    async fn owner_condition_denies_non_owner() {
        let store = Arc::new(InMemoryPolicyStore::new());
        store.upsert_policy(owner_condition_policy()).await.unwrap();

        let evaluator = Arc::new(PolicyEvaluator::new(store));
        let principal = Principal {
            id: "user1".to_string(),
            roles: vec!["user".to_string()],
        };

        let layer = AuthorizationLayer::new(evaluator);
        let mut svc = layer.layer(AlwaysOkService);

        // Record ownerId does NOT match the principal's id
        let op = client_op_with_owner(principal, "user2");
        let result = ServiceExt::ready(&mut svc).await.unwrap().call(op).await;
        assert!(
            matches!(result, Err(OperationError::Forbidden { .. })),
            "non-owner should be denied, got {result:?}"
        );
    }

    /// OpBatch is denied when any op in the batch fails the owner condition.
    #[tokio::test]
    async fn op_batch_denied_when_any_op_fails_condition() {
        let store = Arc::new(InMemoryPolicyStore::new());
        store.upsert_policy(owner_condition_policy()).await.unwrap();

        let evaluator = Arc::new(PolicyEvaluator::new(store));
        let principal = Principal {
            id: "user1".to_string(),
            roles: vec!["user".to_string()],
        };

        let layer = AuthorizationLayer::new(evaluator);
        let mut svc = layer.layer(AlwaysOkService);

        // Batch: first op matches owner, second does not
        let op = op_batch_with_owners(principal, &[("docs", "user1"), ("docs", "other_user")]);
        let result = ServiceExt::ready(&mut svc).await.unwrap().call(op).await;
        assert!(
            matches!(result, Err(OperationError::Forbidden { .. })),
            "batch with any non-owner op should be denied, got {result:?}"
        );
    }

    /// OpBatch is allowed when ALL ops in the batch satisfy the owner condition.
    #[tokio::test]
    async fn op_batch_allowed_when_all_ops_pass_condition() {
        let store = Arc::new(InMemoryPolicyStore::new());
        store.upsert_policy(owner_condition_policy()).await.unwrap();

        let evaluator = Arc::new(PolicyEvaluator::new(store));
        let principal = Principal {
            id: "user1".to_string(),
            roles: vec!["user".to_string()],
        };

        let layer = AuthorizationLayer::new(evaluator);
        let mut svc = layer.layer(AlwaysOkService);

        // Batch: both ops match owner
        let op = op_batch_with_owners(principal, &[("docs", "user1"), ("notes", "user1")]);
        let result = ServiceExt::ready(&mut svc).await.unwrap().call(op).await;
        assert!(
            result.is_ok(),
            "batch where all ops match owner should be allowed, got {result:?}"
        );
    }

    /// Helper: builds a SyncInit (read) operation with CallerOrigin::Anonymous (no connection_id).
    fn anon_query_op(map_name: &str) -> Operation {
        let mut ctx = OperationContext::new(20, service_names::SYNC, make_timestamp(), 5000);
        ctx.caller_origin = CallerOrigin::Anonymous;
        // No connection_id — anonymous HTTP callers have none.
        Operation::SyncInit {
            ctx,
            payload: topgun_core::messages::SyncInitMessage {
                map_name: map_name.to_string(),
                last_sync_timestamp: None,
            },
        }
    }

    /// Helper: builds a ClientOp write operation with CallerOrigin::Anonymous.
    fn anon_write_op(map_name: &str) -> Operation {
        let record = LWWRecord {
            value: Some(rmpv::Value::Boolean(true)),
            timestamp: make_timestamp(),
            ttl_ms: None,
        };
        let mut ctx = OperationContext::new(21, service_names::CRDT, make_timestamp(), 5000);
        ctx.caller_origin = CallerOrigin::Anonymous;
        Operation::ClientOp {
            ctx,
            payload: topgun_core::messages::sync::ClientOpMessage {
                payload: topgun_core::messages::base::ClientOp {
                    map_name: map_name.to_string(),
                    key: "k".to_string(),
                    record: Some(Some(record)),
                    ..Default::default()
                },
            },
        }
    }

    /// Anonymous callers are allowed through when no policies are configured.
    #[tokio::test]
    async fn anonymous_passes_through_when_no_policies() {
        let store = Arc::new(InMemoryPolicyStore::new());
        let evaluator = Arc::new(PolicyEvaluator::new(store));

        let layer = AuthorizationLayer::new(evaluator);
        let mut svc = layer.layer(AlwaysOkService);

        let op = anon_query_op("public-map");
        let resp = ServiceExt::ready(&mut svc).await.unwrap().call(op).await;
        assert!(
            resp.is_ok(),
            "anonymous with no policies should pass through, got {resp:?}"
        );
    }

    /// Anonymous callers can read when an unconditional Allow-Read policy exists.
    #[tokio::test]
    async fn anonymous_read_allowed_by_unconditional_allow_policy() {
        let store = Arc::new(InMemoryPolicyStore::new());
        store
            .upsert_policy(PermissionPolicy {
                id: "allow-read-all".to_string(),
                map_pattern: "*".to_string(),
                action: PermissionAction::Read,
                effect: PolicyEffect::Allow,
                condition: None,
            })
            .await
            .unwrap();

        let evaluator = Arc::new(PolicyEvaluator::new(store));

        let layer = AuthorizationLayer::new(evaluator);
        let mut svc = layer.layer(AlwaysOkService);

        let op = anon_query_op("public-map");
        let resp = ServiceExt::ready(&mut svc).await.unwrap().call(op).await;
        assert!(
            resp.is_ok(),
            "anonymous read with unconditional allow-read policy should pass, got {resp:?}"
        );
    }

    /// Anonymous write operations are denied when policies exist (default-deny with no principal).
    #[tokio::test]
    async fn anonymous_write_denied_when_policies_exist() {
        let store = Arc::new(InMemoryPolicyStore::new());
        // Only a Read allow policy — write has no Allow, so default-deny applies.
        store
            .upsert_policy(PermissionPolicy {
                id: "allow-read-all".to_string(),
                map_pattern: "*".to_string(),
                action: PermissionAction::Read,
                effect: PolicyEffect::Allow,
                condition: None,
            })
            .await
            .unwrap();

        let evaluator = Arc::new(PolicyEvaluator::new(store));

        let layer = AuthorizationLayer::new(evaluator);
        let mut svc = layer.layer(AlwaysOkService);

        let op = anon_write_op("public-map");
        let result = ServiceExt::ready(&mut svc).await.unwrap().call(op).await;
        assert!(
            matches!(result, Err(OperationError::Forbidden { .. })),
            "anonymous write should be denied when no Allow-Write policy matches, got {result:?}"
        );
    }

    /// Tombstone writes (record value None) are denied by owner-condition policies
    /// because there is no data to evaluate the condition against.
    #[tokio::test]
    async fn tombstone_write_denied_by_owner_condition() {
        let store = Arc::new(InMemoryPolicyStore::new());
        store.upsert_policy(owner_condition_policy()).await.unwrap();

        let evaluator = Arc::new(PolicyEvaluator::new(store));
        let principal = Principal {
            id: "user1".to_string(),
            roles: vec!["user".to_string()],
        };

        let layer = AuthorizationLayer::new(evaluator);
        let mut svc = layer.layer(AlwaysOkService);

        // Tombstone: record value is None, so data is Nil and condition cannot match
        let op = client_op_tombstone(principal);
        let result = ServiceExt::ready(&mut svc).await.unwrap().call(op).await;
        assert!(
            matches!(result, Err(OperationError::Forbidden { .. })),
            "tombstone write against owner condition should be denied, got {result:?}"
        );
    }
}
