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
        let ctx = op.ctx();
        if ctx.caller_origin != CallerOrigin::Client {
            return Box::pin(self.inner.call(op));
        }

        // Fail-closed: operations from client connections without a connection_id
        // cannot be attributed to a principal and are denied.
        let Some(connection_id) = ctx.connection_id else {
            return Box::pin(async { Err(OperationError::Unauthorized) });
        };

        let evaluator = Arc::clone(&self.evaluator);
        let registry = Arc::clone(&self.registry);

        // Classify action and extract map_name before moving `op` into the future.
        let (action, map_name) = classify_operation(&op);

        // For operations with no policy-relevant action (bypass group), pass through.
        let Some(action) = action else {
            return Box::pin(self.inner.call(op));
        };

        // Extract data payload for record-level condition evaluation.
        // For write operations, attempt to extract record data from the payload.
        // For read operations and all others, pass Nil (no record data available).
        let data = extract_data(&op);

        let fut = self.inner.call(op);

        Box::pin(async move {
            // When no policies are configured, pass through for backward compatibility.
            if !evaluator.has_policies().await {
                return fut.await;
            }

            // Look up the principal associated with this connection.
            let principal = if let Some(handle) = registry.get(connection_id) {
                let metadata = handle.metadata.read().await;
                metadata.principal.clone()
            } else {
                // Connection was removed between dispatch and authorization — fail closed.
                return Err(OperationError::Unauthorized);
            };

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

/// Extracts record data from write operations for record-level condition evaluation.
///
/// For `ClientOp` operations the first available record value is returned.
/// For all other operations `rmpv::Value::Nil` is returned because either no
/// record data is present (reads, meta-ops) or extraction is not meaningful
/// at the middleware level.
fn extract_data(op: &Operation) -> rmpv::Value {
    match op {
        Operation::ClientOp { payload, .. } => {
            // Extract the LWW record value for record-level condition evaluation.
            // `record` is `Option<Option<LWWRecord<rmpv::Value>>>` due to serde double-option.
            // The inner Option<V> on LWWRecord represents tombstones (None = deleted).
            payload
                .payload
                .record
                .as_ref()
                .and_then(|outer| outer.as_ref())
                .and_then(|r| r.value.clone())
                .unwrap_or(rmpv::Value::Nil)
        }
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
}
