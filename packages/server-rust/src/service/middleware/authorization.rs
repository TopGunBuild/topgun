//! Authorization middleware — RBAC policy enforcement for the operation pipeline.
//!
//! Intercepts every client `Operation` before it reaches domain services and
//! checks it against the configured `PolicyEvaluator`. Operations from trusted
//! origins (Forwarded, System, Backup, Wan) bypass policy evaluation entirely.
//! When no policies are configured the middleware passes all operations through,
//! preserving backward compatibility with deployments that do not use RBAC.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use tower::{Layer, Service};

use crate::service::operation::{CallerOrigin, Operation, OperationError, OperationResponse};
use crate::service::policy::{GateDecision, PermissionAction, PolicyDecision, PolicyEvaluator};
use crate::storage::map_data_store::{check_map_name, MapNameViolation};

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
        // Reserved internal keyspace guard (resource-keyed, not op-family-keyed).
        // No client-origin op — Client / HttpClient / Anonymous, all of which reach
        // this method — may target the server's own control-plane maps
        // (`_topgun_*` device credentials + tombstone cursors, `__topgun_*` RBAC
        // policies). This sits at the classification point so it fires for EVERY
        // client op family that names a map: writes (`OpBatch`/`ClientOp`/
        // `ORMapPushDiff`) AND reads (a NO_AUTH client must not enumerate the
        // credential-hash rows either). It runs BEFORE the `should_evaluate` gate
        // below, so the NO_AUTH `GateDecision::AllowAll` passthrough cannot skip it;
        // trusted origins never reach here (they early-return in `call`), so the
        // server's own internal writes to these maps stay exempt. Ops with an empty
        // `map_name` (bypass group, `SqlQuery`/`VectorSearch`/`QuerySyncInit`)
        // cannot name a reserved map, so they fall through unaffected. For an
        // `OpBatch` EVERY op is inspected (not just the classified first one) —
        // each op is applied by its own `map_name`, so a reserved map in a
        // non-first position must still be rejected.
        if let Some(reserved) = reserved_target_map_name(&op, &map_name) {
            return Box::pin(async move { Err(OperationError::Forbidden { map_name: reserved }) });
        }
        // Map-name admission (TG-NAME-002). A name the shared rule refuses must
        // not reach a domain service: the write would be appended to the WAL and
        // acknowledged, and then refused by the store. Like the guard above this
        // sits BEFORE the `should_evaluate` gate, so the NO_AUTH
        // `GateDecision::AllowAll` passthrough cannot skip it, and it reads the
        // operation itself rather than the classified name, so an `OpBatch` is
        // refused for an inadmissible name in any position.
        if let Some((name, violation)) = inadmissible_map_name(&op) {
            let refusal = OperationError::invalid_map_name(name, violation);
            // The single-message paths answer a refusal with no frame, so this
            // line is the only place an operator sees it.
            MAP_NAME_REFUSAL_LOG.record(&refusal, Instant::now());
            return Box::pin(async move { Err(refusal) });
        }
        let Some(action) = action else {
            return Box::pin(self.inner.call(op));
        };
        let batch_ops_data = extract_batch_ops_data(&op);
        let data = extract_data(&op);
        let fut = self.inner.call(op);
        Box::pin(async move {
            // Single fail-closed gate shared with the HTTP sync read path.
            // AllowAll = never-configured store (backward-compat passthrough);
            // Evaluate = configured store, run policy enforcement (default-deny
            // even if all rules were since deleted); Deny = store read failure,
            // so reject rather than silently open access on a backend outage.
            match evaluator.should_evaluate().await {
                GateDecision::AllowAll => fut.await,
                GateDecision::Evaluate => {
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
                }
                GateDecision::Deny => Err(OperationError::Forbidden { map_name }),
            }
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
        //
        // When auth is disabled at the server level (TOPGUN_NO_AUTH=true), WebSocket
        // connections arrive as CallerOrigin::Client with no principal. Rather than
        // returning Unauthorized immediately, fall through to call_with_principal with
        // None so the per-operation bypass group check in classify_operation() can
        // allow operations like PartitionMapRequest that do not require auth.
        let principal = ctx.principal.clone();
        if principal.is_none() {
            return self.call_with_principal(op, None);
        }

        self.call_with_principal(op, principal)
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

/// Reserved internal map-name prefixes. Maps under these belong to the server's
/// own control plane — `_topgun_*` (device credentials, tombstone cursors) and
/// `__topgun_*` (RBAC policies) — and must never be the target of a client-origin
/// operation. A single `_topgun_` match does NOT cover `__topgun_` (the second
/// character differs), so both are listed explicitly.
const RESERVED_MAP_PREFIXES: [&str; 2] = ["_topgun_", "__topgun_"];

/// Whether `map_name` names a reserved internal map (see [`RESERVED_MAP_PREFIXES`]).
/// Deliberately narrow: only the `_topgun_`/`__topgun_` families are reserved, so
/// other leading-underscore names (`_internal`, `tags`, …) stay valid. The prefix
/// match is ASCII-case-insensitive so a `_TOPGUN_*` variant cannot slip past the
/// guard if any downstream layer folds map-name case.
fn is_reserved_map_name(map_name: &str) -> bool {
    let bytes = map_name.as_bytes();
    RESERVED_MAP_PREFIXES.iter().any(|prefix| {
        let p = prefix.as_bytes();
        bytes.len() >= p.len() && bytes[..p.len()].eq_ignore_ascii_case(p)
    })
}

/// The reserved map name a client-origin op would target, if any.
///
/// `classify_operation` reports only `ops.first()`'s map for an `OpBatch`, but
/// `handle_op_batch` applies EVERY op by its own `map_name` — so the guard must
/// scan all of them, else a batch like `[{tags}, {_topgun_device_credentials}]`
/// smuggles a reserved write past the single-map check. For every other op family
/// the classified `map_name` is the one resource, so it is checked directly.
fn reserved_target_map_name(op: &Operation, classified_map_name: &str) -> Option<String> {
    if let Operation::OpBatch { payload, .. } = op {
        return payload
            .payload
            .ops
            .iter()
            .find(|client_op| is_reserved_map_name(&client_op.map_name))
            .map(|client_op| client_op.map_name.clone());
    }
    is_reserved_map_name(classified_map_name).then(|| classified_map_name.to_string())
}

/// The first map name `op` carries that the shared rule refuses, with the
/// clause it violates (TG-NAME-002).
///
/// Keyed by variant, with no `_` arm on purpose: a new `Operation` variant must
/// fail to compile here until someone decides whether the name it carries can
/// reach a store. The names are borrowed — an admitted operation allocates
/// nothing here.
fn inadmissible_map_name(op: &Operation) -> Option<(&str, MapNameViolation)> {
    let name: &str = match op {
        // Each op of a batch is applied under its own map name, so every one
        // is checked, not just the first.
        Operation::OpBatch { payload, .. } => {
            return payload.payload.ops.iter().find_map(|client_op| {
                check_map_name(&client_op.map_name)
                    .err()
                    .map(|violation| (client_op.map_name.as_str(), violation))
            });
        }

        Operation::ClientOp { payload, .. } => &payload.payload.map_name,
        Operation::ORMapPushDiff { payload, .. } => &payload.payload.map_name,
        Operation::QuerySubscribe { payload, .. } => &payload.payload.map_name,
        Operation::MerkleReqBucket { payload, .. } => &payload.payload.map_name,
        Operation::ORMapMerkleReqBucket { payload, .. } => &payload.payload.map_name,
        Operation::ORMapDiffRequest { payload, .. } => &payload.payload.map_name,
        Operation::EntryProcess { payload, .. } => &payload.map_name,
        Operation::EntryProcessBatch { payload, .. } => &payload.map_name,
        Operation::Search { payload, .. } => &payload.map_name,
        Operation::SearchSubscribe { payload, .. } => &payload.map_name,
        Operation::HybridSearch { payload, .. } => &payload.map_name,
        Operation::HybridSearchSubscribe { payload, .. } => &payload.map_name,
        Operation::SyncInit { payload, .. } => &payload.map_name,
        Operation::ORMapSyncInit { payload, .. } => &payload.map_name,

        // Not checked. A topic and a counter are named, but not by a map name;
        // `QuerySyncInit` and `SqlQuery` name no map; `VectorSearch`,
        // `RegisterResolver` and `JournalSubscribe` carry one but make no
        // durable store write under it, so a refused shape there cannot become
        // an acknowledged write the store then drops. The rest name no map.
        Operation::TopicPublish { .. }
        | Operation::CounterSync { .. }
        | Operation::QuerySyncInit { .. }
        | Operation::SqlQuery { .. }
        | Operation::VectorSearch { .. }
        | Operation::Ping { .. }
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
        | Operation::CounterRequest { .. } => return None,
    };
    check_map_name(name)
        .err()
        .map(|violation| (name, violation))
}

/// Decides when a refused map name writes its operator line, and writes it.
///
/// A client holding an inadmissible name repeats the refused operation on
/// every sync, and a raw peer can send refusals as fast as it likes, so the
/// line is limited to one per [`MapNameRefusalLog::WINDOW`] for the whole
/// process; the refusals that wrote none are counted and reported by the next
/// line. This decides logging only: the operation is refused either way, and
/// every refusal is still counted as an operation error.
///
/// It counts ingress refusal events; an op refused inside a batch is seen
/// twice (batch, then singleton re-dispatch).
struct MapNameRefusalLog {
    /// When the last line was written, and the refusals that have written none
    /// since then.
    state: Mutex<(Option<Instant>, u64)>,
}

impl MapNameRefusalLog {
    const WINDOW: Duration = Duration::from_secs(60);

    const fn new() -> Self {
        Self {
            state: Mutex::new((None, 0)),
        }
    }

    /// `Some(suppressed)` when a refusal at `now` should write the line:
    /// the refusals that wrote no line since the previous one.
    ///
    /// The time is an argument so the decision can be driven without waiting.
    fn line_due(&self, now: Instant) -> Option<u64> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let (last_line, suppressed) = &mut *state;
        if last_line.is_some_and(|last| now.saturating_duration_since(last) < Self::WINDOW) {
            *suppressed += 1;
            return None;
        }
        *last_line = Some(now);
        Some(std::mem::take(suppressed))
    }

    /// Writes the operator line for `refusal` if one is due at `now`.
    ///
    /// The name is chosen by the client, so it is written with `Debug`
    /// formatting — control characters, line breaks and U+0000 arrive escaped
    /// and cannot break or forge a log line — and it is the copy the error
    /// already cut short, never the whole name.
    fn record(&self, refusal: &OperationError, now: Instant) {
        let OperationError::InvalidMapName {
            map_name,
            violation,
        } = refusal
        else {
            return;
        };
        if let Some(suppressed) = self.line_due(now) {
            tracing::warn!(
                map_name = ?map_name,
                violation = ?violation,
                suppressed,
                "refused a client operation that names an inadmissible map"
            );
        }
    }
}

/// The one limiter of the process: a flood of refused names from any number of
/// connections writes one line per window.
static MAP_NAME_REFUSAL_LOG: MapNameRefusalLog = MapNameRefusalLog::new();

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
        Operation::QuerySubscribe { payload, .. } => (
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

    /// A Client-origin operation with `principal = None` is the `TOPGUN_NO_AUTH=true`
    /// posture: the WebSocket handler accepted the connection without an auth token.
    /// The middleware no longer rejects the request outright — instead it forwards
    /// to `call_with_principal(op, None)` and lets `classify_operation()` decide
    /// per-operation. Bypass-group operations (Ping, `PartitionMapRequest`, ...)
    /// pass through, which is what no-auth cluster routing depends on.
    #[tokio::test]
    async fn no_auth_client_with_bypass_op_passes_through() {
        let store = Arc::new(InMemoryPolicyStore::new());
        let evaluator = Arc::new(PolicyEvaluator::new(store));

        let layer = AuthorizationLayer::new(evaluator);
        let mut svc = layer.layer(AlwaysOkService);

        let mut ctx = OperationContext::new(3, service_names::COORDINATION, make_timestamp(), 5000);
        ctx.caller_origin = CallerOrigin::Client;
        // principal deliberately left as None (no-auth-server posture).
        let op = Operation::Ping {
            ctx,
            payload: topgun_core::messages::PingData { timestamp: 0 },
        };

        let result = ServiceExt::ready(&mut svc).await.unwrap().call(op).await;
        assert!(
            result.is_ok(),
            "Client+None on a bypass-group op (Ping) should pass through, got {result:?}"
        );
    }

    /// A Client-origin write with `principal = None` must still be denied when a
    /// policy-gated path is exercised and no Allow-Write policy matches. Proves
    /// that the no-auth bypass for the bypass-group does not open a security
    /// hole for policy-gated operations: writes without a principal cannot
    /// satisfy any write policy, so default-deny applies.
    #[tokio::test]
    async fn no_auth_client_with_policy_gated_write_is_denied() {
        use crate::service::policy::{PermissionPolicy, PolicyEffect};

        let store = Arc::new(InMemoryPolicyStore::new());
        // Only a Read allow policy — write has no Allow, default-deny applies.
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

        // Build a ClientOp write with CallerOrigin::Client and no principal.
        let record = topgun_core::LWWRecord {
            value: Some(rmpv::Value::Boolean(true)),
            timestamp: make_timestamp(),
            ttl_ms: None,
        };
        let mut ctx = OperationContext::new(22, service_names::CRDT, make_timestamp(), 5000);
        ctx.caller_origin = CallerOrigin::Client;
        // principal deliberately left as None.
        let op = Operation::ClientOp {
            ctx,
            payload: topgun_core::messages::sync::ClientOpMessage {
                payload: topgun_core::messages::base::ClientOp {
                    map_name: "public-map".to_string(),
                    key: "k".to_string(),
                    record: Some(Some(record)),
                    ..Default::default()
                },
            },
        };

        let result = ServiceExt::ready(&mut svc).await.unwrap().call(op).await;
        assert!(
            matches!(result, Err(OperationError::Forbidden { .. })),
            "Client+None on a policy-gated write should be Forbidden when no Allow-Write matches, got {result:?}"
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

    // -----------------------------------------------------------------------
    // R4: reserved internal keyspace guard at the authz checkpoint.
    // -----------------------------------------------------------------------

    /// Helper: an Anonymous `OpBatch` (write) whose single op targets `map_name`.
    fn anon_op_batch_op(map_name: &str) -> Operation {
        let mut ctx = OperationContext::new(30, service_names::CRDT, make_timestamp(), 5000);
        ctx.caller_origin = CallerOrigin::Anonymous;
        Operation::OpBatch {
            ctx,
            payload: topgun_core::messages::sync::OpBatchMessage {
                payload: topgun_core::messages::sync::OpBatchPayload {
                    ops: vec![topgun_core::messages::base::ClientOp {
                        map_name: map_name.to_string(),
                        key: "k".to_string(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            },
        }
    }

    /// Helper: an Anonymous `ORMapPushDiff` (write) targeting `map_name` — the
    /// message-type-swap door that bypasses the `crdt.rs` data-plane arms.
    fn anon_ormap_push_diff_op(map_name: &str) -> Operation {
        let mut ctx = OperationContext::new(31, service_names::SYNC, make_timestamp(), 5000);
        ctx.caller_origin = CallerOrigin::Anonymous;
        Operation::ORMapPushDiff {
            ctx,
            payload: topgun_core::messages::sync::ORMapPushDiff {
                payload: topgun_core::messages::sync::ORMapPushDiffPayload {
                    map_name: map_name.to_string(),
                    entries: Vec::new(),
                },
            },
        }
    }

    /// Helper: a TRUSTED-origin (`Forwarded`) `ClientOp` write targeting `map_name`.
    /// Trusted origins early-return before the guard, so an internal write to a
    /// reserved map must still succeed.
    fn trusted_write_op(map_name: &str) -> Operation {
        let record = LWWRecord {
            value: Some(rmpv::Value::Boolean(true)),
            timestamp: make_timestamp(),
            ttl_ms: None,
        };
        let mut ctx = OperationContext::new(32, service_names::CRDT, make_timestamp(), 5000);
        ctx.caller_origin = CallerOrigin::Forwarded;
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

    /// AC7: at the authz checkpoint a CLIENT-origin op targeting a reserved
    /// internal map is REJECTED across op families — `OpBatch`/`ClientOp` writes,
    /// `ORMapPushDiff` (the Audit-v2 C2 message-swap door), AND a read — while a
    /// non-reserved map passes and a TRUSTED-origin write to a reserved map still
    /// succeeds. An EMPTY policy store makes `should_evaluate` return `AllowAll`
    /// (the `TOPGUN_NO_AUTH` posture), so this proves the guard fires BEFORE that
    /// permit — no policy store configured.
    #[tokio::test]
    async fn reserved_namespace_rejected_for_client_origin_under_no_auth() {
        let store = Arc::new(InMemoryPolicyStore::new()); // empty → AllowAll (NO_AUTH)
        let evaluator = Arc::new(PolicyEvaluator::new(store));
        let layer = AuthorizationLayer::new(evaluator);
        let mut svc = layer.layer(AlwaysOkService);

        let reserved_ops = [
            anon_op_batch_op("_topgun_device_credentials"), // OpBatch write
            anon_write_op("_topgun_tombstone_cursors_v2"),  // ClientOp write
            anon_ormap_push_diff_op("_topgun_device_credentials"), // ORMapPushDiff door
            anon_query_op("__topgun_policies"),             // read + double-underscore prefix
        ];
        for op in reserved_ops {
            let resp = ServiceExt::ready(&mut svc).await.unwrap().call(op).await;
            assert!(
                matches!(resp, Err(OperationError::Forbidden { .. })),
                "client-origin op to a reserved map must be Forbidden, got {resp:?}"
            );
        }

        // Non-reserved leading-underscore / plain maps are unaffected (guard narrow).
        for accepted in [anon_write_op("_internal"), anon_write_op("tags")] {
            let resp = ServiceExt::ready(&mut svc)
                .await
                .unwrap()
                .call(accepted)
                .await;
            assert!(
                resp.is_ok(),
                "client op to a non-reserved map must pass, got {resp:?}"
            );
        }

        // A trusted-origin write to a reserved map still succeeds — internal cursor
        // persistence + device-auth are not broken.
        let resp = ServiceExt::ready(&mut svc)
            .await
            .unwrap()
            .call(trusted_write_op("_topgun_tombstone_cursors_v2"))
            .await;
        assert!(
            resp.is_ok(),
            "trusted-origin write to a reserved map must succeed, got {resp:?}"
        );
    }

    /// AC7b: read-side closure — a client-origin READ of the credential map is
    /// Forbidden, so a NO_AUTH client cannot enumerate the stored credential hashes.
    #[tokio::test]
    async fn reserved_namespace_read_rejected_for_client_origin() {
        let store = Arc::new(InMemoryPolicyStore::new());
        let evaluator = Arc::new(PolicyEvaluator::new(store));
        let layer = AuthorizationLayer::new(evaluator);
        let mut svc = layer.layer(AlwaysOkService);

        let resp = ServiceExt::ready(&mut svc)
            .await
            .unwrap()
            .call(anon_query_op("_topgun_device_credentials"))
            .await;
        assert!(
            matches!(resp, Err(OperationError::Forbidden { .. })),
            "client-origin read of a reserved credential map must be Forbidden, got {resp:?}"
        );
    }

    /// Helper: an Anonymous `OpBatch` whose ops target `maps` in order (multi-op).
    fn anon_op_batch_multi(maps: &[&str]) -> Operation {
        let mut ctx = OperationContext::new(33, service_names::CRDT, make_timestamp(), 5000);
        ctx.caller_origin = CallerOrigin::Anonymous;
        Operation::OpBatch {
            ctx,
            payload: topgun_core::messages::sync::OpBatchMessage {
                payload: topgun_core::messages::sync::OpBatchPayload {
                    ops: maps
                        .iter()
                        .map(|m| topgun_core::messages::base::ClientOp {
                            map_name: (*m).to_string(),
                            key: "k".to_string(),
                            ..Default::default()
                        })
                        .collect(),
                    ..Default::default()
                },
            },
        }
    }

    /// AC7 (multi-op regression): `classify_operation` reports only `ops.first()`,
    /// so a reserved map in a NON-FIRST batch position must still be rejected —
    /// otherwise `[{tags}, {_topgun_…}]` smuggles the reserved write past the
    /// single-map check (the device-hijack door R4 exists to close). Also asserts a
    /// case-variant prefix cannot evade the guard, and an all-non-reserved batch
    /// still passes. RED without the per-op scan in `reserved_target_map_name`.
    #[tokio::test]
    async fn reserved_namespace_rejected_when_not_first_op_in_batch() {
        let store = Arc::new(InMemoryPolicyStore::new()); // empty → AllowAll (NO_AUTH)
        let evaluator = Arc::new(PolicyEvaluator::new(store));
        let layer = AuthorizationLayer::new(evaluator);
        let mut svc = layer.layer(AlwaysOkService);

        // Reserved map in a non-first position: the innocuous first op must not shield it.
        for smuggle in [
            anon_op_batch_multi(&["tags", "_topgun_device_credentials"]),
            anon_op_batch_multi(&["tags", "_internal", "__topgun_policies"]),
            // Case-variant prefix must not evade the ASCII-case-insensitive check.
            anon_op_batch_multi(&["tags", "_TOPGUN_device_credentials"]),
        ] {
            let resp = ServiceExt::ready(&mut svc)
                .await
                .unwrap()
                .call(smuggle)
                .await;
            assert!(
                matches!(resp, Err(OperationError::Forbidden { .. })),
                "reserved map in a non-first batch position must be Forbidden, got {resp:?}"
            );
        }

        // An all-non-reserved multi-op batch still passes.
        let clean = anon_op_batch_multi(&["tags", "_internal"]);
        let resp = ServiceExt::ready(&mut svc).await.unwrap().call(clean).await;
        assert!(
            resp.is_ok(),
            "all-non-reserved batch must pass, got {resp:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Map-name admission at the authz checkpoint (TG-NAME-002).
    // -----------------------------------------------------------------------

    use std::sync::atomic::{AtomicUsize, Ordering};

    use topgun_core::messages::{base, hybrid, messaging, query, search, sync, vector};

    /// Inner service that counts the operations that reach it.
    struct CountingService(Arc<AtomicUsize>);

    impl Service<Operation> for CountingService {
        type Response = OperationResponse;
        type Error = OperationError;
        type Future =
            Pin<Box<dyn Future<Output = Result<OperationResponse, OperationError>> + Send>>;

        fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, op: Operation) -> Self::Future {
            self.0.fetch_add(1, Ordering::SeqCst);
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

    /// The layer over a counting inner service, under a policy store that was
    /// never configured (`AllowAll`, the NO_AUTH posture) or under a configured
    /// one whose single policy allows everything — so in both a refusal can
    /// only come from the name check, never from a policy.
    async fn counted_service(
        configured: bool,
    ) -> (AuthorizationService<CountingService>, Arc<AtomicUsize>) {
        let store = Arc::new(InMemoryPolicyStore::new());
        if configured {
            store
                .upsert_policy(PermissionPolicy {
                    id: "allow-all".to_string(),
                    map_pattern: "*".to_string(),
                    action: PermissionAction::All,
                    effect: PolicyEffect::Allow,
                    condition: None,
                })
                .await
                .unwrap();
        }
        let calls = Arc::new(AtomicUsize::new(0));
        let svc = AuthorizationLayer::new(Arc::new(PolicyEvaluator::new(store)))
            .layer(CountingService(Arc::clone(&calls)));
        (svc, calls)
    }

    fn client_ctx(service_name: &'static str, origin: CallerOrigin) -> OperationContext {
        let mut ctx = OperationContext::new(40, service_name, make_timestamp(), 5000);
        ctx.caller_origin = origin;
        ctx
    }

    fn noop_processor() -> messaging::EntryProcessor {
        messaging::EntryProcessor {
            name: "noop".to_string(),
            code: String::new(),
            args: None,
        }
    }

    /// One operation of every family the name check covers, each naming `map`.
    // One literal per family, kept in one list so the count is checkable.
    #[allow(clippy::too_many_lines)]
    fn checked_family_ops(map: &str, origin: CallerOrigin) -> Vec<(&'static str, Operation)> {
        let map_name = map.to_string();
        vec![
            (
                "ClientOp",
                Operation::ClientOp {
                    ctx: client_ctx(service_names::CRDT, origin),
                    payload: sync::ClientOpMessage {
                        payload: base::ClientOp {
                            map_name: map_name.clone(),
                            key: "k".to_string(),
                            ..Default::default()
                        },
                    },
                },
            ),
            (
                "OpBatch",
                Operation::OpBatch {
                    ctx: client_ctx(service_names::CRDT, origin),
                    payload: sync::OpBatchMessage {
                        payload: sync::OpBatchPayload {
                            ops: vec![base::ClientOp {
                                map_name: map_name.clone(),
                                key: "k".to_string(),
                                ..Default::default()
                            }],
                            ..Default::default()
                        },
                    },
                },
            ),
            (
                "EntryProcess",
                Operation::EntryProcess {
                    ctx: client_ctx(service_names::PERSISTENCE, origin),
                    payload: messaging::EntryProcessData {
                        request_id: "r".to_string(),
                        map_name: map_name.clone(),
                        key: "k".to_string(),
                        processor: noop_processor(),
                    },
                },
            ),
            (
                "EntryProcessBatch",
                Operation::EntryProcessBatch {
                    ctx: client_ctx(service_names::PERSISTENCE, origin),
                    payload: messaging::EntryProcessBatchData {
                        request_id: "r".to_string(),
                        map_name: map_name.clone(),
                        keys: vec!["k".to_string()],
                        processor: noop_processor(),
                    },
                },
            ),
            (
                "ORMapPushDiff",
                Operation::ORMapPushDiff {
                    ctx: client_ctx(service_names::SYNC, origin),
                    payload: sync::ORMapPushDiff {
                        payload: sync::ORMapPushDiffPayload {
                            map_name: map_name.clone(),
                            entries: Vec::new(),
                        },
                    },
                },
            ),
            (
                "QuerySubscribe",
                Operation::QuerySubscribe {
                    ctx: client_ctx(service_names::QUERY, origin),
                    payload: query::QuerySubMessage {
                        payload: query::QuerySubPayload {
                            query_id: "q".to_string(),
                            map_name: map_name.clone(),
                            query: base::Query::default(),
                            fields: None,
                        },
                    },
                },
            ),
            (
                "Search",
                Operation::Search {
                    ctx: client_ctx(service_names::SEARCH, origin),
                    payload: search::SearchPayload {
                        request_id: "r".to_string(),
                        map_name: map_name.clone(),
                        query: "text".to_string(),
                        options: None,
                    },
                },
            ),
            (
                "SearchSubscribe",
                Operation::SearchSubscribe {
                    ctx: client_ctx(service_names::SEARCH, origin),
                    payload: search::SearchSubPayload {
                        subscription_id: "s".to_string(),
                        map_name: map_name.clone(),
                        query: "text".to_string(),
                        options: None,
                    },
                },
            ),
            (
                "HybridSearch",
                Operation::HybridSearch {
                    ctx: client_ctx(service_names::SEARCH, origin),
                    payload: hybrid::HybridSearchPayload {
                        request_id: "r".to_string(),
                        map_name: map_name.clone(),
                        query_text: "text".to_string(),
                        methods: vec![hybrid::SearchMethod::Exact],
                        k: 1,
                        query_vector: None,
                        predicate: None,
                        include_value: None,
                        min_score: None,
                    },
                },
            ),
            (
                "HybridSearchSubscribe",
                Operation::HybridSearchSubscribe {
                    ctx: client_ctx(service_names::SEARCH, origin),
                    payload: hybrid::HybridSearchSubPayload {
                        subscription_id: "s".to_string(),
                        map_name: map_name.clone(),
                        query_text: "text".to_string(),
                        methods: vec![hybrid::SearchMethod::Exact],
                        k: 1,
                        query_vector: None,
                        predicate: None,
                        include_value: None,
                        min_score: None,
                    },
                },
            ),
            (
                "SyncInit",
                Operation::SyncInit {
                    ctx: client_ctx(service_names::SYNC, origin),
                    payload: sync::SyncInitMessage {
                        map_name: map_name.clone(),
                        last_sync_timestamp: None,
                    },
                },
            ),
            (
                "MerkleReqBucket",
                Operation::MerkleReqBucket {
                    ctx: client_ctx(service_names::SYNC, origin),
                    payload: sync::MerkleReqBucketMessage {
                        payload: sync::MerkleReqBucketPayload {
                            map_name: map_name.clone(),
                            path: String::new(),
                        },
                    },
                },
            ),
            (
                "ORMapSyncInit",
                Operation::ORMapSyncInit {
                    ctx: client_ctx(service_names::SYNC, origin),
                    payload: sync::ORMapSyncInit {
                        map_name: map_name.clone(),
                        root_hash: 0,
                        bucket_hashes: std::collections::HashMap::new(),
                        last_sync_timestamp: None,
                        claimed_epoch: None,
                    },
                },
            ),
            (
                "ORMapMerkleReqBucket",
                Operation::ORMapMerkleReqBucket {
                    ctx: client_ctx(service_names::SYNC, origin),
                    payload: sync::ORMapMerkleReqBucket {
                        payload: sync::ORMapMerkleReqBucketPayload {
                            map_name: map_name.clone(),
                            path: String::new(),
                        },
                    },
                },
            ),
            (
                "ORMapDiffRequest",
                Operation::ORMapDiffRequest {
                    ctx: client_ctx(service_names::SYNC, origin),
                    payload: sync::ORMapDiffRequest {
                        payload: sync::ORMapDiffRequestPayload {
                            map_name,
                            keys: vec!["k".to_string()],
                        },
                    },
                },
            ),
        ]
    }

    /// One name per clause of the shared rule, in the rule's order.
    fn refused_names() -> Vec<(String, MapNameViolation)> {
        vec![
            (String::new(), MapNameViolation::Empty),
            (
                "x__backup".to_string(),
                MapNameViolation::ReservedBackupSuffix,
            ),
            ("a\0b".to_string(), MapNameViolation::ContainsNul),
            ("a".repeat(513), MapNameViolation::TooLong),
        ]
    }

    const CLIENT_ORIGINS: [CallerOrigin; 3] = [
        CallerOrigin::Client,
        CallerOrigin::HttpClient,
        CallerOrigin::Anonymous,
    ];

    /// TG-NAME-002: for every checked family, every refused name and every
    /// client origin, the operation is refused with `InvalidMapName` and the
    /// inner service is never called — whether the policy store was never
    /// configured (where the layer otherwise passes everything through) or is
    /// configured with a policy that allows everything.
    #[tokio::test]
    async fn a_name_outside_the_admissible_set_never_reaches_the_inner_service() {
        for configured in [false, true] {
            let (mut svc, calls) = counted_service(configured).await;
            for (name, expected) in refused_names() {
                for origin in CLIENT_ORIGINS {
                    let ops = checked_family_ops(&name, origin);
                    assert_eq!(ops.len(), 15, "every checked family is exercised");
                    for (family, op) in ops {
                        let result = ServiceExt::ready(&mut svc).await.unwrap().call(op).await;
                        let case = format!(
                            "{family} / {expected:?} / {origin:?} / configured={configured}"
                        );
                        match result {
                            Err(OperationError::InvalidMapName {
                                map_name,
                                violation,
                            }) => {
                                assert_eq!(violation, expected, "{case}");
                                assert!(map_name.len() <= 128, "{case}: echoed name is cut");
                                assert!(name.starts_with(&map_name), "{case}");
                            }
                            other => panic!("{case}: expected InvalidMapName, got {other:?}"),
                        }
                        assert_eq!(calls.load(Ordering::SeqCst), 0, "{case}: inner was called");
                    }
                }
            }
        }
    }

    /// TG-NAME-002: each op of a batch is applied under its own map name, so an
    /// inadmissible name in a non-first position refuses the whole dispatch —
    /// the admissible ops ahead of it must not shield it, and none of them
    /// reaches the inner service in that dispatch.
    #[tokio::test]
    async fn a_refused_op_in_any_batch_position_refuses_the_whole_dispatch() {
        for configured in [false, true] {
            let (mut svc, calls) = counted_service(configured).await;
            for (name, expected) in refused_names() {
                for maps in [
                    vec!["tags", name.as_str()],
                    vec!["tags", name.as_str(), "notes"],
                    vec!["tags", "user-profiles", "notes", name.as_str()],
                ] {
                    let result = ServiceExt::ready(&mut svc)
                        .await
                        .unwrap()
                        .call(anon_op_batch_multi(&maps))
                        .await;
                    let case = format!("{expected:?} at {} of {}", maps.len(), configured);
                    assert!(
                        matches!(
                            result,
                            Err(OperationError::InvalidMapName { violation, .. })
                                if violation == expected
                        ),
                        "{case}: got {result:?}"
                    );
                    assert_eq!(calls.load(Ordering::SeqCst), 0, "{case}: inner was called");
                }
            }

            // The same batches without the offending op are dispatched.
            let result = ServiceExt::ready(&mut svc)
                .await
                .unwrap()
                .call(anon_op_batch_multi(&["tags", "user-profiles", "notes"]))
                .await;
            assert!(result.is_ok(), "an admissible batch passes, got {result:?}");
            assert_eq!(calls.load(Ordering::SeqCst), 1);
        }
    }

    /// The families outside the check carry a topic, a counter name, no name at
    /// all, or a map name no durable write is made under. An empty name there
    /// is not a violation: the operation reaches the inner service.
    #[tokio::test]
    async fn unchecked_families_pass_an_empty_classified_name() {
        let (mut svc, calls) = counted_service(false).await;
        let origin = CallerOrigin::Anonymous;
        let ops = vec![
            (
                "TopicPublish",
                Operation::TopicPublish {
                    ctx: client_ctx(service_names::MESSAGING, origin),
                    payload: messaging::TopicPubPayload {
                        topic: String::new(),
                        data: rmpv::Value::Nil,
                    },
                },
            ),
            (
                "CounterSync",
                Operation::CounterSync {
                    ctx: client_ctx(service_names::PERSISTENCE, origin),
                    payload: messaging::CounterStatePayload {
                        name: String::new(),
                        state: messaging::PNCounterState {
                            p: std::collections::HashMap::new(),
                            n: std::collections::HashMap::new(),
                        },
                    },
                },
            ),
            (
                "QuerySyncInit",
                Operation::QuerySyncInit {
                    ctx: client_ctx(service_names::QUERY, origin),
                    payload: query::QuerySyncInitMessage {
                        payload: query::QuerySyncInitPayload {
                            query_id: "q".to_string(),
                            root_hash: 0,
                        },
                    },
                },
            ),
            (
                "SqlQuery",
                Operation::SqlQuery {
                    ctx: client_ctx(service_names::QUERY, origin),
                    payload: query::SqlQueryPayload {
                        sql: "SELECT 1".to_string(),
                        query_id: "q".to_string(),
                    },
                },
            ),
            (
                "VectorSearch",
                Operation::VectorSearch {
                    ctx: client_ctx(service_names::SEARCH, origin),
                    payload: vector::VectorSearchPayload {
                        id: "v".to_string(),
                        map_name: String::new(),
                        index_name: None,
                        query_vector: Vec::new(),
                        k: 1,
                        ef_search: None,
                        options: None,
                    },
                },
            ),
            (
                "JournalSubscribe",
                Operation::JournalSubscribe {
                    ctx: client_ctx(service_names::COORDINATION, origin),
                    payload: messaging::JournalSubscribeData {
                        request_id: "j".to_string(),
                        map_name: Some(String::new()),
                        ..Default::default()
                    },
                },
            ),
        ];
        for (expected_calls, (family, op)) in ops.into_iter().enumerate() {
            let result = ServiceExt::ready(&mut svc).await.unwrap().call(op).await;
            assert!(result.is_ok(), "{family}: got {result:?}");
            assert_eq!(
                calls.load(Ordering::SeqCst),
                expected_calls + 1,
                "{family}: inner was not called"
            );
        }
    }

    /// The rule refuses four things and nothing else: no character class, and
    /// the length bound admits a name of exactly 512 bytes.
    #[tokio::test]
    async fn admissible_names_reach_the_inner_service() {
        let (mut svc, calls) = counted_service(false).await;
        let at_bound = "a".repeat(512);
        let mut expected_calls = 0;
        for name in [
            "user-profiles",
            "users/profiles",
            "notes:abc",
            at_bound.as_str(),
        ] {
            for (family, op) in checked_family_ops(name, CallerOrigin::Client) {
                let result = ServiceExt::ready(&mut svc).await.unwrap().call(op).await;
                expected_calls += 1;
                assert!(result.is_ok(), "{family} on {name:?}: got {result:?}");
                assert_eq!(
                    calls.load(Ordering::SeqCst),
                    expected_calls,
                    "{family} on {name:?}: inner was not called"
                );
            }
        }
    }

    /// The operator line is limited to one per window, reports what the
    /// limiter dropped, and cannot be broken or forged by the client-chosen
    /// name. Driven on its own limiter with injected instants: the process-wide
    /// one is shared with every other test that refuses a name.
    #[test]
    fn map_name_refusals_log_once_per_window_with_a_suppressed_count() {
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
        // Scoped, never a global install: this test binary is shared and runs in
        // parallel, so a global subscriber would leak into every other test.
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_ansi(false)
            .finish();
        let lines = || -> Vec<String> {
            let bytes = captured
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            String::from_utf8(bytes)
                .expect("utf-8 log")
                .lines()
                .map(str::to_string)
                .collect()
        };

        let log = MapNameRefusalLog::new();
        let start = Instant::now();
        let at = |millis: u64| start + Duration::from_millis(millis);

        let hostile = "a\nb\0c__backup";
        let refusal =
            OperationError::invalid_map_name(hostile, MapNameViolation::ReservedBackupSuffix);
        let oversized =
            OperationError::invalid_map_name(&"z".repeat(4096), MapNameViolation::TooLong);

        tracing::subscriber::with_default(subscriber, || {
            // 100 refusals inside one window: the first writes, 99 do not.
            for i in 0..100 {
                log.record(&refusal, at(i * 500));
            }
            assert_eq!(lines().len(), 1, "one line per window: {:#?}", lines());

            // An error that is not a refused name writes nothing and takes no window.
            log.record(
                &OperationError::Forbidden {
                    map_name: "m".to_string(),
                },
                at(60_000),
            );
            assert_eq!(lines().len(), 1);

            // The next window's line reports the 99 the limiter dropped.
            log.record(&refusal, at(60_000));
            assert_eq!(lines().len(), 2);
            log.record(&refusal, at(60_001));
            assert_eq!(lines().len(), 2);

            // A later window: one suppressed, and an oversized name is cut.
            log.record(&oversized, at(120_000));
        });

        let lines = lines();
        assert_eq!(lines.len(), 3, "{lines:#?}");
        for line in &lines {
            assert!(line.contains("WARN"), "{line}");
        }
        // The line break and the NUL of the name arrive escaped, on one line.
        assert!(
            lines[0].contains(r#"map_name="a\nb\0c__backup""#),
            "{}",
            lines[0]
        );
        assert!(!lines[0].contains('\0'), "{}", lines[0]);
        assert!(
            lines[0].contains("violation=ReservedBackupSuffix"),
            "{}",
            lines[0]
        );
        assert!(lines[0].contains("suppressed=0"), "{}", lines[0]);
        assert!(lines[1].contains("suppressed=99"), "{}", lines[1]);
        assert!(lines[2].contains("violation=TooLong"), "{}", lines[2]);
        assert!(lines[2].contains("suppressed=1"), "{}", lines[2]);
        assert!(lines[2].contains(&"z".repeat(128)), "{}", lines[2]);
        assert!(!lines[2].contains(&"z".repeat(129)), "{}", lines[2]);
    }
}
