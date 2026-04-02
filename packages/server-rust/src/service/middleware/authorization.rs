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

use crate::network::connection::ConnectionRegistry;
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
    registry: Arc<ConnectionRegistry>,
}

impl AuthorizationLayer {
    /// Creates a new layer with the given evaluator and connection registry.
    #[must_use]
    pub fn new(evaluator: Arc<PolicyEvaluator>, registry: Arc<ConnectionRegistry>) -> Self {
        Self { evaluator, registry }
    }
}

impl<S> Layer<S> for AuthorizationLayer {
    type Service = AuthorizationService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        AuthorizationService {
            inner,
            evaluator: Arc::clone(&self.evaluator),
            registry: Arc::clone(&self.registry),
        }
    }
}

// ---------------------------------------------------------------------------
// AuthorizationService
// ---------------------------------------------------------------------------

/// Tower `Service` produced by `AuthorizationLayer`.
///
/// Holds references to the `PolicyEvaluator` (shared across all workers via
/// `Arc`) and the `ConnectionRegistry` (for principal lookup by `connection_id`).
pub struct AuthorizationService<S> {
    inner: S,
    evaluator: Arc<PolicyEvaluator>,
    registry: Arc<ConnectionRegistry>,
}

impl<S> Service<Operation> for AuthorizationService<S>
where
    S: Service<Operation, Response = OperationResponse, Error = OperationError>
        + Send
        + 'static,
    S::Future: Send + 'static,
{
    type Response = OperationResponse;
    type Error = OperationError;
    type Future = Pin<Box<dyn Future<Output = Result<OperationResponse, OperationError>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, op: Operation) -> Self::Future {
        // Trusted origins bypass policy evaluation entirely. Forwarded operations
        // come from peer nodes that have already been authorized at the cluster level.
        // HttpClient is NOT trusted: it goes through the same RBAC checks as Client.
        let ctx = op.ctx();
        if !matches!(ctx.caller_origin, CallerOrigin::Client | CallerOrigin::HttpClient) {
            return Box::pin(self.inner.call(op));
        }

        // Resolve the principal: WebSocket ops carry a connection_id for registry
        // lookup; HTTP ops carry ctx.principal set directly by the handler.
        // Fail-closed when neither is available.
        let principal_opt: Option<topgun_core::Principal> = if let Some(connection_id) = ctx.connection_id {
            // WebSocket path: principal resolved lazily from registry inside the async block
            let evaluator = Arc::clone(&self.evaluator);
            let registry = Arc::clone(&self.registry);
            let (action, map_name) = classify_operation(&op);
            let Some(action) = action else {
                return Box::pin(self.inner.call(op));
            };
            let batch_ops_data: Option<Vec<(String, rmpv::Value)>> = match &op {
                Operation::OpBatch { payload, .. } => Some(
                    payload
                        .payload
                        .ops
                        .iter()
                        .map(|client_op| {
                            (client_op.map_name.clone(), extract_op_data(client_op))
                        })
                        .collect(),
                ),
                _ => None,
            };
            let data = extract_data(&op);
            let fut = self.inner.call(op);
            return Box::pin(async move {
                if !evaluator.has_policies().await {
                    return fut.await;
                }
                let principal = if let Some(handle) = registry.get(connection_id) {
                    let metadata = handle.metadata.read().await;
                    metadata.principal.clone()
                } else {
                    return Err(OperationError::Unauthorized);
                };
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
            });
        } else if let Some(p) = ctx.principal.clone() {
            // HTTP path: principal was set directly by the handler after JWT validation.
            Some(p)
        } else {
            // No connection_id and no ctx.principal — fail closed.
            return Box::pin(async { Err(OperationError::Unauthorized) });
        };

        let evaluator = Arc::clone(&self.evaluator);

        // Classify action and extract map_name before moving `op` into the future.
        let (action, map_name) = classify_operation(&op);

        // For operations with no policy-relevant action (bypass group), pass through.
        let Some(action) = action else {
            return Box::pin(self.inner.call(op));
        };

        // Extract per-op (map_name, data) pairs for OpBatch before moving `op`.
        // Each op in the batch needs individual record-level condition evaluation.
        let batch_ops_data: Option<Vec<(String, rmpv::Value)>> = match &op {
            Operation::OpBatch { payload, .. } => Some(
                payload
                    .payload
                    .ops
                    .iter()
                    .map(|client_op| {
                        (client_op.map_name.clone(), extract_op_data(client_op))
                    })
                    .collect(),
            ),
            _ => None,
        };

        // Extract data payload for record-level condition evaluation (non-batch path).
        let data = extract_data(&op);

        let fut = self.inner.call(op);

        // HTTP path: principal already resolved above from ctx.principal.
        Box::pin(async move {
            // When no policies are configured, pass through for backward compatibility.
            if !evaluator.has_policies().await {
                return fut.await;
            }

            // Use the principal resolved before entering the async block (HTTP path).
            let principal = principal_opt;

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
        })
    }
}

// ---------------------------------------------------------------------------
// Operation classification helpers
// ---------------------------------------------------------------------------

/// Maps an `Operation` variant to a `PermissionAction` and a `map_name`.
///
/// Returns `(None, _)` for operations in the bypass group — these pass through
/// without any policy evaluation (e.g., Ping, `PartitionMapRequest`, `GarbageCollect`).
fn classify_operation(op: &Operation) -> (Option<PermissionAction>, String) {
    match op {
        // --- Write actions ---
        Operation::ClientOp { payload, .. } => {
            (Some(PermissionAction::Write), payload.payload.map_name.clone())
        }
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
        Operation::ORMapPushDiff { payload, .. } => {
            (Some(PermissionAction::Write), payload.payload.map_name.clone())
        }

        // --- Read actions ---
        Operation::QuerySubscribe { payload, .. } | Operation::DagQuery { payload, .. } => {
            (Some(PermissionAction::Read), payload.payload.map_name.clone())
        }
        // QuerySyncInit resumes by query_id; SqlQuery spans multiple maps — no single map_name.
        Operation::QuerySyncInit { .. } | Operation::SqlQuery { .. } => {
            (Some(PermissionAction::Read), String::new())
        }
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
        Operation::MerkleReqBucket { payload, .. } => {
            (Some(PermissionAction::Read), payload.payload.map_name.clone())
        }
        Operation::ORMapSyncInit { payload, .. } => {
            // ORMapSyncInit is flat (no payload wrapper).
            (Some(PermissionAction::Read), payload.map_name.clone())
        }
        Operation::ORMapMerkleReqBucket { payload, .. } => {
            (Some(PermissionAction::Read), payload.payload.map_name.clone())
        }
        Operation::ORMapDiffRequest { payload, .. } => {
            (Some(PermissionAction::Read), payload.payload.map_name.clone())
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
mod tests {
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::task::{Context, Poll};

    use topgun_core::Timestamp;
    use tower::{Layer, Service, ServiceExt};

    use super::*;
    use crate::network::config::ConnectionConfig;
    use crate::network::connection::ConnectionKind;
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
            Box::pin(async move { Ok(OperationResponse::NotImplemented { service_name: name, call_id }) })
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
        let mut ctx =
            OperationContext::new(1, service_names::COORDINATION, make_timestamp(), 5000);
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
        let registry = Arc::new(ConnectionRegistry::new());

        let layer = AuthorizationLayer::new(evaluator, registry);
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

        let registry = Arc::new(ConnectionRegistry::new());
        let config = ConnectionConfig::default();
        let (handle, _rx) = registry.register(ConnectionKind::Client, &config);
        let conn_id = handle.id;

        let layer = AuthorizationLayer::new(evaluator, registry);
        let mut svc = layer.layer(AlwaysOkService);

        let mut ctx =
            OperationContext::new(2, service_names::COORDINATION, make_timestamp(), 5000);
        ctx.caller_origin = CallerOrigin::Client;
        ctx.connection_id = Some(conn_id);
        let op = Operation::Ping {
            ctx,
            payload: topgun_core::messages::PingData { timestamp: 0 },
        };

        let resp = ServiceExt::ready(&mut svc).await.unwrap().call(op).await;
        assert!(resp.is_ok(), "empty store should allow all ops, got {resp:?}");
    }

    /// Missing connection_id on a Client-origin operation returns Unauthorized.
    #[tokio::test]
    async fn missing_connection_id_returns_unauthorized() {
        let store = Arc::new(InMemoryPolicyStore::new());
        let evaluator = Arc::new(PolicyEvaluator::new(store));
        let registry = Arc::new(ConnectionRegistry::new());

        let layer = AuthorizationLayer::new(evaluator, registry);
        let mut svc = layer.layer(AlwaysOkService);

        let mut ctx =
            OperationContext::new(3, service_names::COORDINATION, make_timestamp(), 5000);
        ctx.caller_origin = CallerOrigin::Client;
        // connection_id deliberately left as None
        let op = Operation::Ping {
            ctx,
            payload: topgun_core::messages::PingData { timestamp: 0 },
        };

        let result = ServiceExt::ready(&mut svc).await.unwrap().call(op).await;
        assert!(
            matches!(result, Err(OperationError::Unauthorized)),
            "missing connection_id should return Unauthorized, got {result:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Record-level condition tests (owner restriction, tombstone, batch atomicity)
    // -----------------------------------------------------------------------

    use crate::network::connection::ConnectionId;
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

    /// Helper: registers a connection with the given principal.
    async fn register_with_principal(
        registry: &ConnectionRegistry,
        principal: Principal,
    ) -> ConnectionId {
        let config = ConnectionConfig::default();
        let (handle, _rx) = registry.register(ConnectionKind::Client, &config);
        let conn_id = handle.id;
        {
            let mut meta = handle.metadata.write().await;
            meta.principal = Some(principal);
        }
        conn_id
    }

    /// Helper: builds a ClientOp Operation with a record containing the given ownerId value.
    fn client_op_with_owner(conn_id: ConnectionId, owner_id: &str) -> Operation {
        let record = LWWRecord {
            value: Some(rmpv::Value::Map(vec![(
                rmpv::Value::String("ownerId".into()),
                rmpv::Value::String(owner_id.into()),
            )])),
            timestamp: make_timestamp(),
            ttl_ms: None,
        };
        let mut ctx =
            OperationContext::new(10, service_names::CRDT, make_timestamp(), 5000);
        ctx.caller_origin = CallerOrigin::Client;
        ctx.connection_id = Some(conn_id);
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
    fn client_op_tombstone(conn_id: ConnectionId) -> Operation {
        let record = LWWRecord {
            value: None,
            timestamp: make_timestamp(),
            ttl_ms: None,
        };
        let mut ctx =
            OperationContext::new(11, service_names::CRDT, make_timestamp(), 5000);
        ctx.caller_origin = CallerOrigin::Client;
        ctx.connection_id = Some(conn_id);
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
    fn op_batch_with_owners(conn_id: ConnectionId, ops: &[(&str, &str)]) -> Operation {
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

        let mut ctx =
            OperationContext::new(12, service_names::CRDT, make_timestamp(), 5000);
        ctx.caller_origin = CallerOrigin::Client;
        ctx.connection_id = Some(conn_id);
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
        let registry = Arc::new(ConnectionRegistry::new());
        let principal = Principal { id: "user1".to_string(), roles: vec!["user".to_string()] };
        let conn_id = register_with_principal(&registry, principal).await;

        let layer = AuthorizationLayer::new(evaluator, registry);
        let mut svc = layer.layer(AlwaysOkService);

        // Record ownerId matches the principal's id
        let op = client_op_with_owner(conn_id, "user1");
        let result = ServiceExt::ready(&mut svc).await.unwrap().call(op).await;
        assert!(result.is_ok(), "matching owner should be allowed, got {result:?}");
    }

    /// Owner condition denies a write when the record's ownerId does NOT match auth.id.
    #[tokio::test]
    async fn owner_condition_denies_non_owner() {
        let store = Arc::new(InMemoryPolicyStore::new());
        store.upsert_policy(owner_condition_policy()).await.unwrap();

        let evaluator = Arc::new(PolicyEvaluator::new(store));
        let registry = Arc::new(ConnectionRegistry::new());
        let principal = Principal { id: "user1".to_string(), roles: vec!["user".to_string()] };
        let conn_id = register_with_principal(&registry, principal).await;

        let layer = AuthorizationLayer::new(evaluator, registry);
        let mut svc = layer.layer(AlwaysOkService);

        // Record ownerId does NOT match the principal's id
        let op = client_op_with_owner(conn_id, "user2");
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
        let registry = Arc::new(ConnectionRegistry::new());
        let principal = Principal { id: "user1".to_string(), roles: vec!["user".to_string()] };
        let conn_id = register_with_principal(&registry, principal).await;

        let layer = AuthorizationLayer::new(evaluator, registry);
        let mut svc = layer.layer(AlwaysOkService);

        // Batch: first op matches owner, second does not
        let op = op_batch_with_owners(conn_id, &[("docs", "user1"), ("docs", "other_user")]);
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
        let registry = Arc::new(ConnectionRegistry::new());
        let principal = Principal { id: "user1".to_string(), roles: vec!["user".to_string()] };
        let conn_id = register_with_principal(&registry, principal).await;

        let layer = AuthorizationLayer::new(evaluator, registry);
        let mut svc = layer.layer(AlwaysOkService);

        // Batch: both ops match owner
        let op = op_batch_with_owners(conn_id, &[("docs", "user1"), ("notes", "user1")]);
        let result = ServiceExt::ready(&mut svc).await.unwrap().call(op).await;
        assert!(result.is_ok(), "batch where all ops match owner should be allowed, got {result:?}");
    }

    /// Tombstone writes (record value None) are denied by owner-condition policies
    /// because there is no data to evaluate the condition against.
    #[tokio::test]
    async fn tombstone_write_denied_by_owner_condition() {
        let store = Arc::new(InMemoryPolicyStore::new());
        store.upsert_policy(owner_condition_policy()).await.unwrap();

        let evaluator = Arc::new(PolicyEvaluator::new(store));
        let registry = Arc::new(ConnectionRegistry::new());
        let principal = Principal { id: "user1".to_string(), roles: vec!["user".to_string()] };
        let conn_id = register_with_principal(&registry, principal).await;

        let layer = AuthorizationLayer::new(evaluator, registry);
        let mut svc = layer.layer(AlwaysOkService);

        // Tombstone: record value is None, so data is Nil and condition cannot match
        let op = client_op_tombstone(conn_id);
        let result = ServiceExt::ready(&mut svc).await.unwrap().call(op).await;
        assert!(
            matches!(result, Err(OperationError::Forbidden { .. })),
            "tombstone write against owner condition should be denied, got {result:?}"
        );
    }
}
