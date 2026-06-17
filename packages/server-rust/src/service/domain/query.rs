//! Query domain service for live query subscriptions.
//!
//! Manages an in-memory `QueryRegistry` of standing query subscriptions,
//! evaluates queries against `RecordStore` contents, and pushes incremental
//! `QUERY_UPDATE` messages (ENTER/UPDATE/LEAVE) to subscribers via
//! `CrdtService::broadcast_query_updates()` when data changes.

use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use async_trait::async_trait;
use dashmap::{DashMap, DashSet};
use tower::Service;

use topgun_core::messages::base::{ChangeEventType, Query};
use topgun_core::messages::client_events::QueryUpdatePayload;
use topgun_core::messages::query::{QueryRespMessage, QueryRespPayload, QueryResultEntry};
use topgun_core::messages::vector::{
    VectorSearchPayload, VectorSearchRespPayload, VectorSearchResult,
};
use topgun_core::messages::{Message, SyncRespRootMessage, SyncRespRootPayload};
use topgun_core::vector::distance::DistanceMetric;

use crate::dag::coordinator::{run_dag_local, ClusterQueryCoordinator};
use crate::query::cursor::{
    build_next_cursor, classify_cursor_status, cursor_query_hashes, decode_cursor, SortValue,
};
use crate::query::window::LiveWindow;

use tracing::Instrument;

use crate::network::connection::{ConnectionId, ConnectionRegistry, OutboundMessage};
use crate::service::domain::index::attribute::AttributeExtractor;
use crate::service::domain::index::query_optimizer::index_aware_evaluate;
use crate::service::domain::index::IndexObserverFactory;
use crate::service::domain::predicate::{
    evaluate_predicate, evaluate_where, execute_query as predicate_execute_query, value_to_rmpv,
    EvalContext,
};
use crate::service::operation::{service_names, Operation, OperationError, OperationResponse};
use crate::service::registry::{ManagedService, ServiceContext};
use crate::storage::mutation_observer::MutationObserver;
use crate::storage::record::{Record, RecordValue};
use crate::storage::RecordStoreFactory;

// ---------------------------------------------------------------------------
// QuerySubscription
// ---------------------------------------------------------------------------

/// A standing query subscription registered by a client.
pub struct QuerySubscription {
    /// Unique identifier for this subscription.
    pub query_id: String,
    /// Connection that owns this subscription.
    pub connection_id: ConnectionId,
    /// Map being queried.
    pub map_name: String,
    /// The query parameters (filter, sort, pagination).
    pub query: Query,
    /// Keys that matched on the last evaluation (for ENTER/UPDATE/LEAVE detection).
    pub previous_result_keys: DashSet<String>,
    /// Sorted top-N window maintaining live membership for this subscription.
    ///
    /// Interior-mutable (its state lives behind a `Mutex`), so mutations route through
    /// `apply_mutation(&self, …)` over the `Arc<QuerySubscription>` with no `&mut` path.
    /// This is the single authoritative window algorithm — the observer derives every
    /// ENTER/UPDATE/LEAVE (including top-N displacement and promotion) from it.
    ///
    /// Wrapped in `Arc` so the mutation broadcast path can clone the handle cheaply out
    /// of the `Arc<QuerySubscription>` and call `apply_mutation` without holding the
    /// subscription borrow.
    pub live_window: Arc<LiveWindow>,
    /// Optional field projection list. When `Some`, only these fields are
    /// included in `QUERY_RESP` and `QUERY_UPDATE` payloads sent to this subscriber.
    pub fields: Option<Vec<String>>,
}

// ---------------------------------------------------------------------------
// QueryRegistry
// ---------------------------------------------------------------------------

/// In-memory registry of standing query subscriptions.
///
/// Thread-safe via `DashMap`. Keyed by `map_name` for efficient lookup
/// during mutation observation.
pub struct QueryRegistry {
    /// `map_name` -> { `query_id` -> `QuerySubscription` }
    subscriptions: DashMap<String, DashMap<String, Arc<QuerySubscription>>>,
}

impl QueryRegistry {
    /// Creates a new empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            subscriptions: DashMap::new(),
        }
    }

    /// Registers a standing query subscription.
    pub fn register(&self, sub: QuerySubscription) {
        let map_name = sub.map_name.clone();
        let query_id = sub.query_id.clone();
        let sub = Arc::new(sub);

        self.subscriptions
            .entry(map_name)
            .or_default()
            .insert(query_id, sub);
    }

    /// Removes a subscription by `query_id`. Returns `true` if found.
    #[must_use]
    pub fn unregister(&self, query_id: &str) -> bool {
        let mut found = false;
        // Iterate all maps to find the subscription
        self.subscriptions.retain(|_, inner| {
            if inner.contains_key(query_id) {
                inner.remove(query_id);
                found = true;
            }
            // Remove the outer entry if the inner map is now empty
            !inner.is_empty()
        });
        found
    }

    /// Removes all subscriptions for a disconnected connection.
    pub fn unregister_by_connection(&self, conn_id: ConnectionId) {
        self.subscriptions.retain(|_, inner| {
            inner.retain(|_, sub| sub.connection_id != conn_id);
            !inner.is_empty()
        });
    }

    /// Returns all subscriptions targeting a specific map.
    #[must_use]
    pub fn get_subscriptions_for_map(&self, map_name: &str) -> Vec<Arc<QuerySubscription>> {
        match self.subscriptions.get(map_name) {
            Some(inner) => inner.iter().map(|entry| entry.value().clone()).collect(),
            None => Vec::new(),
        }
    }

    /// Returns the set of unique connection IDs with active subscriptions for `map_name`.
    ///
    /// Used by `CrdtService` to target broadcast events only to subscribers
    /// instead of all connected clients.
    #[must_use]
    pub fn get_subscribed_connection_ids(&self, map_name: &str) -> HashSet<ConnectionId> {
        self.get_subscriptions_for_map(map_name)
            .into_iter()
            .map(|sub| sub.connection_id)
            .collect()
    }

    /// Looks up a subscription by `query_id` across all maps.
    ///
    /// Used by `handle_query_sync_init` to resolve the `map_name` for a query.
    #[must_use]
    pub fn get_subscription(&self, query_id: &str) -> Option<Arc<QuerySubscription>> {
        for entry in &self.subscriptions {
            if let Some(sub) = entry.value().get(query_id) {
                return Some(sub.value().clone());
            }
        }
        None
    }

    /// Total subscription count across all maps (for testing).
    #[must_use]
    pub fn subscription_count(&self) -> usize {
        self.subscriptions
            .iter()
            .map(|entry| entry.value().len())
            .sum()
    }
}

impl Default for QueryRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// QueryMutationObserver
// ---------------------------------------------------------------------------

/// Mutation observer that re-evaluates standing queries when data changes.
///
/// Implements `MutationObserver` to receive notifications from `RecordStore`
/// and pushes `QUERY_UPDATE` messages to subscribers via `ConnectionRegistry`.
pub struct QueryMutationObserver {
    registry: Arc<QueryRegistry>,
    connection_registry: Arc<ConnectionRegistry>,
    /// Map name for this observer instance (set per-RecordStore).
    map_name: String,
    /// Partition ID for this observer instance.
    #[allow(dead_code)]
    partition_id: u32,
}

impl QueryMutationObserver {
    /// Creates a new observer for a specific map and partition.
    #[must_use]
    pub fn new(
        registry: Arc<QueryRegistry>,
        connection_registry: Arc<ConnectionRegistry>,
        map_name: String,
        partition_id: u32,
    ) -> Self {
        Self {
            registry,
            connection_registry,
            map_name,
            partition_id,
        }
    }

    /// Evaluates a single key against a subscription's query predicate.
    fn matches_query(sub: &QuerySubscription, data: &rmpv::Value) -> bool {
        if let Some(pred) = &sub.query.predicate {
            evaluate_predicate(pred, &EvalContext::data_only(data))
        } else if let Some(wh) = &sub.query.r#where {
            evaluate_where(wh, data)
        } else {
            // No filter: match all
            true
        }
    }

    /// Sends a `QUERY_UPDATE` to a subscriber's connection.
    fn send_update(
        &self,
        sub: &QuerySubscription,
        key: &str,
        value: rmpv::Value,
        change_type: ChangeEventType,
    ) {
        let payload = QueryUpdatePayload {
            query_id: sub.query_id.clone(),
            key: key.to_string(),
            value,
            change_type,
        };
        let msg = Message::QueryUpdate { payload };
        if let Ok(bytes) = rmp_serde::to_vec_named(&msg) {
            if let Some(handle) = self.connection_registry.get(sub.connection_id) {
                let _ = handle.try_send(OutboundMessage::Binary(bytes));
            }
        }
    }

    /// Common logic for `on_put` and `on_update`: re-evaluates a key against
    /// standing queries and sends appropriate change events.
    fn evaluate_change(&self, key: &str, record: &Record, is_backup: bool) {
        if is_backup {
            return;
        }

        let subs = self.registry.get_subscriptions_for_map(&self.map_name);
        if subs.is_empty() {
            return;
        }

        let rmpv_value = extract_rmpv_value(&record.value);

        for sub in &subs {
            let matches_now = Self::matches_query(sub, &rmpv_value);

            // Route the mutation through the single window algorithm. It returns the
            // complete delta set the subscriber must receive — predicate ENTER/UPDATE/LEAVE
            // for unbounded subscriptions, plus top-N displacement LEAVEs and promotion
            // ENTERs when the query has a limit. There is no second predicate-only path:
            // membership is owned by the window, and previous_result_keys is kept in sync
            // with the deltas the window emits.
            let deltas = sub
                .live_window
                .apply_mutation(key, Some(&rmpv_value), matches_now);

            for delta in deltas {
                match delta.event {
                    ChangeEventType::ENTER | ChangeEventType::UPDATE => {
                        sub.previous_result_keys.insert(delta.key.clone());
                    }
                    ChangeEventType::LEAVE => {
                        sub.previous_result_keys.remove(&delta.key);
                    }
                }
                self.send_update(sub, &delta.key, delta.value, delta.event);
            }
        }
    }
}

impl MutationObserver for QueryMutationObserver {
    fn on_put(
        &self,
        key: &str,
        record: &Record,
        _old_value: Option<&RecordValue>,
        is_backup: bool,
    ) {
        self.evaluate_change(key, record, is_backup);
    }

    fn on_update(
        &self,
        key: &str,
        record: &Record,
        _old_value: &RecordValue,
        _new_value: &RecordValue,
        is_backup: bool,
    ) {
        self.evaluate_change(key, record, is_backup);
    }

    fn on_remove(&self, key: &str, _record: &Record, is_backup: bool) {
        if is_backup {
            return;
        }
        let subs = self.registry.get_subscriptions_for_map(&self.map_name);

        for sub in &subs {
            // A delete routes through the same window algorithm as a mutation: removing a
            // key can both emit a LEAVE for that key and promote a previously-displaced row
            // into the top-N window (an ENTER). The window owns membership; we mirror its
            // deltas into previous_result_keys so on_clear/on_reset/Merkle stay correct.
            let deltas = sub.live_window.apply_mutation(key, None, false);
            for delta in deltas {
                match delta.event {
                    ChangeEventType::ENTER | ChangeEventType::UPDATE => {
                        sub.previous_result_keys.insert(delta.key.clone());
                    }
                    ChangeEventType::LEAVE => {
                        sub.previous_result_keys.remove(&delta.key);
                    }
                }
                self.send_update(sub, &delta.key, delta.value, delta.event);
            }
        }
    }

    fn on_evict(&self, _key: &str, _record: &Record, _is_backup: bool) {
        // No-op for query purposes
    }

    fn on_load(&self, _key: &str, _record: &Record, _is_backup: bool) {
        // No-op for query purposes
    }

    fn on_replication_put(&self, _key: &str, _record: &Record, _populate_index: bool) {
        // No-op for query purposes
    }

    fn on_clear(&self) {
        let subs = self.registry.get_subscriptions_for_map(&self.map_name);
        for sub in &subs {
            // Send LEAVE for every key in previous_result_keys
            for key_ref in sub.previous_result_keys.iter() {
                let key = key_ref.key().clone();
                self.send_update(sub, &key, rmpv::Value::Nil, ChangeEventType::LEAVE);
            }
            sub.previous_result_keys.clear();
        }
    }

    fn on_reset(&self) {
        let subs = self.registry.get_subscriptions_for_map(&self.map_name);
        for sub in &subs {
            for key_ref in sub.previous_result_keys.iter() {
                let key = key_ref.key().clone();
                self.send_update(sub, &key, rmpv::Value::Nil, ChangeEventType::LEAVE);
            }
            sub.previous_result_keys.clear();
        }
    }

    fn on_destroy(&self, _is_shutdown: bool) {
        // No-op for query purposes
    }
}

// ---------------------------------------------------------------------------
// QueryService
// ---------------------------------------------------------------------------

/// Real query domain service handling `QuerySubscribe` and `QueryUnsubscribe`.
///
/// Replaces the `domain_stub!(QueryService, ...)` macro-generated stub.
/// Evaluates queries against `RecordStore` contents, registers standing
/// subscriptions in `QueryRegistry`, and returns initial results.
pub struct QueryService {
    query_registry: Arc<QueryRegistry>,
    record_store_factory: Arc<RecordStoreFactory>,
    /// Retained for `unregister_by_connection` on client disconnect (module wiring deferred).
    #[allow(dead_code)]
    connection_registry: Arc<ConnectionRegistry>,
    /// Per-query Merkle manager for delta sync init.
    /// `None` when query Merkle sync is not wired (test ergonomics).
    query_merkle_manager: Option<Arc<crate::storage::query_merkle::QueryMerkleSyncManager>>,
    /// Maximum records returned in a single `QUERY_RESP`. Queries matching more
    /// records are clamped to this limit with `has_more: true`.
    max_query_records: u32,
    /// Index observer factory for index-accelerated predicate evaluation.
    /// `None` when index wiring is not enabled (sim/test call sites).
    index_observer_factory: Option<Arc<IndexObserverFactory>>,
    #[cfg(feature = "datafusion")]
    sql_query_backend: Option<Arc<dyn crate::service::domain::query_backend::SqlQueryBackend>>,
    /// Optional DAG coordinator, reserved for distributed (cluster) execution.
    /// `None` in single-node mode; the single-node WS path runs the DAG locally
    /// via `run_dag_local` and does not require a coordinator.
    coordinator: Option<Arc<ClusterQueryCoordinator>>,
    /// Tests-only opt-out: when `true`, the handler uses the linear predicate
    /// engine (single-field sort, optional index acceleration) instead of the
    /// canonical local DAG engine. Defaults to `false` — prod and most tests run
    /// the DAG. Set via `with_linear_engine_for_tests()` only where a unit test
    /// must exercise the linear path.
    linear_engine_for_tests: bool,
}

/// Maps raw DAG output rows to `QueryResultEntry` values.
///
/// `ScanProcessor` injects the real record key as `_key` into every row, and wraps
/// non-Map (scalar) record values as `{_key, _value}`. GROUP BY aggregate rows instead
/// carry `__key` (the bucket key). This recovers the entry key (preferring the real
/// `_key`, then the group `__key`, then a synthetic `row-{i}`) and strips the internal
/// `_key`/`_value` fields so the returned value equals the originally stored value.
fn map_dag_rows_to_entries(raw: Vec<rmpv::Value>) -> Vec<QueryResultEntry> {
    raw.into_iter()
        .enumerate()
        .map(|(i, val)| match val {
            rmpv::Value::Map(pairs) => {
                let mut real_key: Option<String> = None;
                let mut group_key: Option<String> = None;
                let mut unwrapped_value: Option<rmpv::Value> = None;
                let mut kept: Vec<(rmpv::Value, rmpv::Value)> = Vec::with_capacity(pairs.len());

                for (k, v) in pairs {
                    // Own the field name so we can move (k, v) into `kept` without
                    // holding a borrow of `k` across the match.
                    let kname = k.as_str().map(std::string::ToString::to_string);
                    match kname.as_deref() {
                        Some("_key") => real_key = v.as_str().map(str::to_string),
                        Some("_value") => unwrapped_value = Some(v),
                        Some("__key") => {
                            // Keep `__key` in the value for GROUP BY aggregate rows.
                            group_key = v.as_str().map(str::to_string);
                            kept.push((k, v));
                        }
                        _ => kept.push((k, v)),
                    }
                }

                let key = real_key.or(group_key).unwrap_or_else(|| format!("row-{i}"));
                // A `_value` wrapper means the original record value was a scalar;
                // unwrap it. Otherwise the value is the row minus internal `_key`.
                let value = unwrapped_value.unwrap_or(rmpv::Value::Map(kept));
                QueryResultEntry { key, value }
            }
            other => QueryResultEntry {
                key: format!("row-{i}"),
                value: other,
            },
        })
        .collect()
}

impl QueryService {
    /// Creates a new `QueryService` with its required dependencies.
    ///
    /// Pass `Some(query_merkle_manager)` to enable per-query Merkle sync init.
    /// Pass `None` to keep existing call sites working unchanged.
    ///
    /// Pass `Some(index_observer_factory)` to enable index-accelerated predicate
    /// evaluation. Pass `None` to fall back to full-scan (sim/test call sites).
    #[must_use]
    pub fn new(
        query_registry: Arc<QueryRegistry>,
        record_store_factory: Arc<RecordStoreFactory>,
        connection_registry: Arc<ConnectionRegistry>,
        query_merkle_manager: Option<Arc<crate::storage::query_merkle::QueryMerkleSyncManager>>,
        max_query_records: u32,
        index_observer_factory: Option<Arc<IndexObserverFactory>>,
        #[cfg(feature = "datafusion")] sql_query_backend: Option<
            Arc<dyn crate::service::domain::query_backend::SqlQueryBackend>,
        >,
    ) -> Self {
        Self {
            query_registry,
            record_store_factory,
            connection_registry,
            query_merkle_manager,
            max_query_records,
            index_observer_factory,
            #[cfg(feature = "datafusion")]
            sql_query_backend,
            coordinator: None,
            linear_engine_for_tests: false,
        }
    }

    /// Attaches a `ClusterQueryCoordinator` to enable GROUP BY (DAG) queries.
    ///
    /// Uses a builder pattern so existing `new()` call sites require no modification.
    #[must_use]
    pub fn with_coordinator(mut self, coordinator: Arc<ClusterQueryCoordinator>) -> Self {
        self.coordinator = Some(coordinator);
        self
    }

    /// Tests-only: forces the handler onto the linear predicate engine instead of
    /// the canonical local DAG engine. Used by unit tests that specifically exercise
    /// the linear path (single-field sort, index-accelerated evaluation).
    #[must_use]
    pub fn with_linear_engine_for_tests(mut self) -> Self {
        self.linear_engine_for_tests = true;
        self
    }

    /// Returns a reference to the underlying `QueryRegistry`.
    #[must_use]
    pub fn registry(&self) -> &Arc<QueryRegistry> {
        &self.query_registry
    }
}

// ---------------------------------------------------------------------------
// ManagedService implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl ManagedService for QueryService {
    fn name(&self) -> &'static str {
        service_names::QUERY
    }

    async fn init(&self, _ctx: &ServiceContext) -> anyhow::Result<()> {
        Ok(())
    }

    async fn reset(&self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn shutdown(&self, _terminate: bool) -> anyhow::Result<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// tower::Service<Operation> implementation
// ---------------------------------------------------------------------------

impl Service<Operation> for Arc<QueryService> {
    type Response = OperationResponse;
    type Error = OperationError;
    type Future = Pin<Box<dyn Future<Output = Result<OperationResponse, OperationError>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, op: Operation) -> Self::Future {
        let svc = Arc::clone(self);
        let service_name = op.ctx().service_name;
        let call_id = op.ctx().call_id;
        let caller_origin = format!("{:?}", op.ctx().caller_origin);

        let span = tracing::info_span!(
            "domain_op",
            service = service_name,
            call_id = call_id,
            caller_origin = %caller_origin,
        );

        Box::pin(
            async move {
                match op {
                    Operation::QuerySubscribe { ctx, payload } => {
                        svc.handle_query_subscribe(&ctx, &payload).await
                    }
                    Operation::QueryUnsubscribe { ctx, payload } => {
                        svc.handle_query_unsubscribe(&ctx, &payload)
                    }
                    Operation::QuerySyncInit { ctx, payload } => {
                        svc.handle_query_sync_init(&ctx, payload).await
                    }
                    Operation::SqlQuery { ctx, payload } => {
                        svc.handle_sql_query(&ctx, &payload).await
                    }
                    Operation::VectorSearch { ctx, payload } => {
                        svc.handle_vector_search(&ctx, &payload).await
                    }
                    _ => Err(OperationError::WrongService),
                }
            }
            .instrument(span),
        )
    }
}

// ---------------------------------------------------------------------------
// Handler implementations
// ---------------------------------------------------------------------------

impl QueryService {
    /// Handles a `QuerySubscribe` operation.
    ///
    /// 1. Extracts `connection_id` from context (error if missing).
    /// 2. Scans all partitions, collecting entries and Merkle key-hash pairs.
    /// 3. When a coordinator is wired, delegates to the DAG single-node path for
    ///    filtering, sorting, and limiting. Without a coordinator (tests only),
    ///    falls back to the predicate engine directly.
    /// 4. Applies `max_query_records` clamping with `has_more` flag.
    /// 5. Applies field projection if `fields` is specified.
    /// 6. Initializes per-query Merkle trees and computes aggregate root hash.
    /// 7. Registers a standing `QuerySubscription` in the registry.
    /// 8. Returns `QueryResp` with initial results.
    #[allow(clippy::too_many_lines)]
    async fn handle_query_subscribe(
        &self,
        ctx: &crate::service::operation::OperationContext,
        payload: &topgun_core::messages::query::QuerySubMessage,
    ) -> Result<OperationResponse, OperationError> {
        let connection_id = ctx.connection_id.ok_or_else(|| {
            OperationError::Internal(anyhow::anyhow!(
                "QuerySubscribe requires connection_id in OperationContext"
            ))
        })?;

        let query_id = payload.payload.query_id.clone();
        let map_name = payload.payload.map_name.clone();
        let query = payload.payload.query.clone();
        let fields = payload.payload.fields.clone();

        // Scan ALL partitions for this map to aggregate entries and Merkle key-hash pairs.
        // Keys are deterministically mapped to partitions via hash_to_partition,
        // so there is no risk of duplicates across partitions.
        let stores = self.record_store_factory.get_all_for_map(&map_name);

        let mut entries: Vec<(String, rmpv::Value)> = Vec::new();
        // Collect (partition_id, key, hash) for Merkle tree initialization.
        let mut key_hash_pairs_by_partition: Vec<(u32, Vec<(String, u32)>)> = Vec::new();

        for store in &stores {
            let partition_id = store.partition_id();
            let mut partition_hashes: Vec<(String, u32)> = Vec::new();

            store.for_each_boxed(
                &mut |key, record| {
                    if let RecordValue::Lww {
                        ref value,
                        ref timestamp,
                    } = record.value
                    {
                        let rmpv_value = value_to_rmpv(value);
                        entries.push((key.to_string(), rmpv_value));

                        // Compute hash for Merkle tree.
                        let item_hash = topgun_core::hash::fnv1a_hash(&format!(
                            "{}:{}:{}:{}",
                            key, timestamp.millis, timestamp.counter, timestamp.node_id
                        ));
                        partition_hashes.push((key.to_string(), item_hash));
                    }
                    // Skip OrMap/OrTombstones records for query evaluation
                },
                false, // not backup
            );

            if !partition_hashes.is_empty() {
                key_hash_pairs_by_partition.push((partition_id, partition_hashes));
            }
        }

        // Capture cursor-emission inputs before `query` is moved into QuerySubscription.
        // Hashes are computed via the single authoritative source shared with the
        // consume-side path in converter.rs, making hash divergence impossible.
        let (predicate_hash, sort_hash) = cursor_query_hashes(&query);
        let query_limit = query.limit;
        // Decode the incoming cursor (if any) before `query` is moved, so the emission
        // site can report an accurate cursor_status. Validation reuses the same helpers
        // the DAG's CursorProcessor runs, against the same hashes computed above — so the
        // status reported here agrees with the DAG's accept/reject decision by construction.
        let input_cursor = query.cursor.as_deref().and_then(decode_cursor);
        // Build a sort_values template from the query's sort spec for cursor construction.
        let sort_values_template: Vec<SortValue> = query
            .sort
            .as_ref()
            .map(|sf| {
                sf.iter()
                    .map(|f| SortValue {
                        field: f.field.clone(),
                        value: serde_json::Value::Null, // placeholder; real values extracted per-entry
                        direction: f.direction.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default();

        // `coordinator_has_more` is set by the coordinator branch, which already absorbs
        // the limit+1 sentinel internally. The local/predicate branches detect has_more
        // from the raw DAG output length after running with limit+1.
        let mut coordinator_has_more = false;
        // For local/predicate branches, track whether more records existed beyond the page.
        let mut local_has_more = false;

        // Canonical single-node engine: run the structured query through the DAG
        // pipeline locally over this map's partitions (Scan→Filter→Cursor→Sort→Limit,
        // or group-by aggregate). Multi-field sort, limit, and the cursor stage are
        // all handled here. No coordinator is required for single-node execution.
        //
        // The linear predicate engine remains available behind an explicit
        // tests-only opt-out (`with_linear_engine_for_tests`) — it supports only
        // single-field sort, plus index-accelerated narrowing, and is used by unit
        // tests that target that path directly.
        let mut results: Vec<QueryResultEntry> = if self.linear_engine_for_tests {
            // Narrow candidate entries using index-accelerated evaluation when an
            // IndexObserverFactory is wired and the query has a predicate. This
            // reduces the entries passed to execute_query without altering the
            // query semantics — the predicate is re-evaluated inside the backend
            // and inside index_aware_evaluate itself, so no correctness risk.
            let entries = if let (Some(factory), Some(predicate)) = (
                self.index_observer_factory.as_ref(),
                query.predicate.as_ref(),
            ) {
                if let Some(registry) = factory.get_registry(&map_name) {
                    // Build a lookup map so the optimizer can fetch record values by key.
                    let entry_map: std::collections::HashMap<&str, &rmpv::Value> =
                        entries.iter().map(|(k, v)| (k.as_str(), v)).collect();
                    let all_keys: Vec<String> = entries.iter().map(|(k, _)| k.clone()).collect();

                    let matching_keys =
                        index_aware_evaluate(&registry, predicate, &all_keys, |key| {
                            entry_map.get(key).map(|v| (*v).clone())
                        });

                    let matching_set: HashSet<&str> =
                        matching_keys.iter().map(String::as_str).collect();
                    entries
                        .into_iter()
                        .filter(|(k, _)| matching_set.contains(k.as_str()))
                        .collect()
                } else {
                    entries
                }
            } else {
                entries
            };

            // Linear engine does not use the limit+1 sentinel; has_more comes from
            // max_query_records clamping only for this legacy path.
            predicate_execute_query(entries, &query)
        } else if let Some(ref coordinator) = self.coordinator {
            // Distributed path: the coordinator fans out to all owning nodes, collects
            // per-node results, and applies the global sort+limit merge (SPEC-301).
            // The coordinator's single-node bypass routes back through run_dag_local
            // when only one member is active, keeping single-node behaviour identical.
            let dist_result = coordinator
                .execute_distributed(&query, &map_name)
                .await
                .map_err(|e| OperationError::Internal(anyhow::anyhow!("{e}")))?;

            coordinator_has_more = dist_result.has_more;
            map_dag_rows_to_entries(dist_result.rows)
        } else {
            let partition_ids: Vec<u32> = stores.iter().map(|s| s.partition_id()).collect();
            let raw = run_dag_local(
                &query,
                &map_name,
                partition_ids,
                Arc::clone(&self.record_store_factory),
                &crate::dag::types::QueryConfig::default(),
            )
            .await
            .map_err(|e| OperationError::Internal(anyhow::anyhow!("{e}")))?;

            // The DAG ran with limit+1 (converter.rs), so if we got limit+1 rows it
            // means more records exist. Detect and absorb the sentinel before mapping.
            let (raw_truncated, more) = if let Some(lim) = query_limit {
                let has_sentinel = raw.len() > lim as usize;
                let mut r = raw;
                r.truncate(lim as usize);
                (r, has_sentinel)
            } else {
                (raw, false)
            };
            local_has_more = more;

            map_dag_rows_to_entries(raw_truncated)
        };

        // Reconcile per-branch has_more signals: coordinator branch already absorbed
        // its sentinel and set coordinator_has_more; local/predicate branches set
        // local_has_more from the limit+1 sentinel detection above.
        let page_has_more = coordinator_has_more || local_has_more;

        // Apply max_query_records clamping — both sources of has_more are unified here.
        let total_count = results.len();
        let max = self.max_query_records as usize;
        let has_more = if total_count > max {
            tracing::info!(
                query_id = %query_id,
                total_count = total_count,
                max_query_records = max,
                "Clamping query results to max_query_records limit"
            );
            results.truncate(max);
            Some(true)
        } else if page_has_more {
            Some(true)
        } else {
            None
        };

        // Seed the live window from THIS initial DAG result page (before field projection,
        // so sort-field values are still present for ordering). Membership and ordering are
        // taken from the page the DAG already produced — we do not recompute. Every page row
        // matched the predicate by construction, so each seeds as an in-window match.
        let live_window = Arc::new(LiveWindow::new(
            query.sort.clone().unwrap_or_default(),
            query.limit,
        ));
        for entry in &results {
            let _ = live_window.apply_mutation(&entry.key, Some(&entry.value), true);
        }

        // Apply field projection if specified
        if let Some(ref proj_fields) = fields {
            for entry in &mut results {
                entry.value = project_fields(proj_fields, &entry.value);
            }
        }

        // Build previous_result_keys from results (after clamping, before Merkle).
        // The page that seeds previous_result_keys is the same page that seeds the window,
        // so the two membership views start consistent.
        let previous_keys = DashSet::new();
        for entry in &results {
            previous_keys.insert(entry.key.clone());
        }

        // Initialize per-query Merkle trees for matching records
        if let Some(ref merkle) = self.query_merkle_manager {
            // Only insert keys that are in the result set into Merkle trees.
            // Build a set of result keys for fast lookup.
            let result_key_set: HashSet<&str> = results.iter().map(|e| e.key.as_str()).collect();

            for (partition_id, partition_hashes) in &key_hash_pairs_by_partition {
                let matching: Vec<(String, u32)> = partition_hashes
                    .iter()
                    .filter(|(k, _)| result_key_set.contains(k.as_str()))
                    .cloned()
                    .collect();
                if !matching.is_empty() {
                    merkle.init_tree(&query_id, &map_name, *partition_id, &matching);
                }
            }
        }

        // Compute aggregate Merkle root hash across all partitions
        let merkle_root_hash = self
            .query_merkle_manager
            .as_ref()
            .map(|m| m.aggregate_query_root_hash(&query_id, &map_name));

        // Emit a real keyset cursor when the query has a limit, more records exist, and
        // a sort shape is available to derive a keyset position. The cursor is built from
        // the last entry in the result page using the single authoritative emission helper.
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
            .unwrap_or(0);

        let next_cursor = if has_more == Some(true)
            && query_limit.is_some()
            && !sort_values_template.is_empty()
        {
            results.last().map(|last| {
                build_next_cursor(
                    &last.key,
                    &last.value,
                    &sort_values_template,
                    predicate_hash,
                    sort_hash,
                    now_ms,
                )
            })
        } else {
            None
        };

        // Reflect the input cursor processing outcome. Re-validate the decoded cursor with
        // the same checks the DAG's CursorProcessor applies (expiry first, then hash-match
        // against this query's predicate/sort hashes), so the client can distinguish a stale
        // token (restart pagination) from genuine exhaustion. This is deterministic — it does
        // NOT infer rejection from an empty result page, which would mislabel a legitimately
        // empty final page as expired/invalid.
        let cursor_status = Some(classify_cursor_status(
            input_cursor.as_ref(),
            now_ms,
            predicate_hash,
            sort_hash,
        ));

        // Register standing subscription (with fields for future QUERY_UPDATE projection).
        // Must happen AFTER hashes are captured (query is moved here).
        let subscription = QuerySubscription {
            query_id: query_id.clone(),
            connection_id,
            map_name,
            query,
            previous_result_keys: previous_keys,
            live_window,
            fields,
        };
        self.query_registry.register(subscription);

        // Build response
        let resp = Message::QueryResp(QueryRespMessage {
            payload: QueryRespPayload {
                query_id,
                results,
                next_cursor,
                has_more,
                cursor_status,
                merkle_root_hash,
            },
        });

        Ok(OperationResponse::Message(Box::new(resp)))
    }

    /// Handles a `QueryUnsubscribe` operation.
    #[allow(clippy::unnecessary_wraps)]
    fn handle_query_unsubscribe(
        &self,
        _ctx: &crate::service::operation::OperationContext,
        payload: &topgun_core::messages::query::QueryUnsubMessage,
    ) -> Result<OperationResponse, OperationError> {
        // Clean up Merkle trees for this query
        if let Some(ref merkle) = self.query_merkle_manager {
            merkle.cleanup_query(&payload.payload.query_id);
        }
        let _ = self.query_registry.unregister(&payload.payload.query_id);
        Ok(OperationResponse::Empty)
    }

    /// Handles a `QuerySyncInit` operation.
    ///
    /// Client sends its stored query Merkle root hash. The server computes the aggregate
    /// query root hash across all partitions and responds with `SyncRespRootMessage`.
    ///
    /// If the hashes differ, the client drives traversal via `MerkleReqBucket` messages
    /// with query-prefixed paths (e.g. `"query:<query_id>/<partition_id>/<sub_path>"`).
    /// Parsing of query-prefixed bucket paths in `SyncService` is deferred to a follow-up spec.
    ///
    /// Returns `OperationResponse::Empty` when query Merkle sync is not wired
    /// (i.e. `query_merkle_manager` is `None`).
    #[allow(clippy::unused_async)]
    async fn handle_query_sync_init(
        &self,
        ctx: &crate::service::operation::OperationContext,
        payload: topgun_core::messages::query::QuerySyncInitMessage,
    ) -> Result<OperationResponse, OperationError> {
        let Some(query_merkle) = self.query_merkle_manager.as_ref() else {
            return Ok(OperationResponse::Empty);
        };

        let query_id = payload.payload.query_id;

        // Look up map_name for this query from the registry.
        let Some(sub) = self.query_registry.get_subscription(&query_id) else {
            return Ok(OperationResponse::Empty);
        };
        let map_name = sub.map_name.clone();

        // Compute the server's aggregate query root hash across all partitions.
        let root_hash = query_merkle.aggregate_query_root_hash(&query_id, &map_name);

        Ok(OperationResponse::Message(Box::new(Message::SyncRespRoot(
            SyncRespRootMessage {
                payload: SyncRespRootPayload {
                    map_name,
                    root_hash,
                    timestamp: ctx.timestamp.clone(),
                },
            },
        ))))
    }

    /// Handles a `SqlQuery` operation.
    ///
    /// When the `datafusion` feature is enabled and a `SqlQueryBackend` is
    /// configured, executes the SQL string and converts Arrow `RecordBatch`es
    /// to `rmpv::Value` rows for wire transport. Without the feature or backend,
    /// returns an error indicating SQL is not available.
    #[cfg(feature = "datafusion")]
    async fn handle_sql_query(
        &self,
        _ctx: &crate::service::operation::OperationContext,
        payload: &topgun_core::messages::query::SqlQueryPayload,
    ) -> Result<OperationResponse, OperationError> {
        let sql_backend = self.sql_query_backend.as_ref().ok_or_else(|| {
            OperationError::Internal(anyhow::anyhow!("SQL requires datafusion feature"))
        })?;

        match sql_backend.execute_sql(&payload.sql).await {
            Ok(batches) => {
                let (columns, rows) = record_batches_to_rows(&batches);
                let resp_payload = topgun_core::messages::query::SqlQueryRespPayload {
                    query_id: payload.query_id.clone(),
                    columns,
                    rows,
                    error: None,
                };
                Ok(OperationResponse::Message(Box::new(
                    Message::SqlQueryResp {
                        payload: resp_payload,
                    },
                )))
            }
            Err(e) => {
                let resp_payload = topgun_core::messages::query::SqlQueryRespPayload {
                    query_id: payload.query_id.clone(),
                    columns: vec![],
                    rows: vec![],
                    error: Some(e.to_string()),
                };
                Ok(OperationResponse::Message(Box::new(
                    Message::SqlQueryResp {
                        payload: resp_payload,
                    },
                )))
            }
        }
    }

    /// Handles a `SqlQuery` operation when the `datafusion` feature is disabled.
    #[cfg(not(feature = "datafusion"))]
    #[allow(clippy::unused_async)]
    async fn handle_sql_query(
        &self,
        _ctx: &crate::service::operation::OperationContext,
        _payload: &topgun_core::messages::query::SqlQueryPayload,
    ) -> Result<OperationResponse, OperationError> {
        Err(OperationError::Internal(anyhow::anyhow!(
            "SQL requires datafusion feature"
        )))
    }

    /// Handles a `VectorSearch` operation.
    ///
    /// Resolves the vector index from `IndexObserverFactory`, runs HNSW ANN
    /// search, converts distance → score per metric, applies `min_score`
    /// and optional post-filter, populates `value` per `include_value`, and
    /// returns a `VectorSearchResp` with top-k results sorted descending by score.
    #[allow(clippy::too_many_lines)]
    pub(crate) async fn handle_vector_search(
        &self,
        _ctx: &crate::service::operation::OperationContext,
        payload: &VectorSearchPayload,
    ) -> Result<OperationResponse, OperationError> {
        // Elapsed milliseconds, saturated to u64::MAX (unreachable for any realistic request).
        fn elapsed_ms(start: std::time::Instant) -> u64 {
            u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
        }

        let start = std::time::Instant::now();

        // Helper macro that uses `start` for timing.
        macro_rules! error_resp {
            ($msg:expr) => {{
                return Ok(OperationResponse::Message(Box::new(
                    Message::VectorSearchResp {
                        payload: VectorSearchRespPayload {
                            id: payload.id.clone(),
                            results: vec![],
                            total_candidates: 0,
                            search_time_ms: elapsed_ms(start),
                            error: Some($msg.to_string()),
                        },
                    },
                )));
            }};
        }

        // 1. Require index registry.
        let factory = self.index_observer_factory.as_ref().ok_or_else(|| {
            OperationError::Internal(anyhow::anyhow!(
                "VectorSearch requires IndexObserverFactory"
            ))
        })?;
        let Some(registry) = factory.get_registry(&payload.map_name) else {
            error_resp!(format!(
                "map not registered for indexing: {}",
                payload.map_name
            ));
        };

        // 2. Resolve vector index attribute.
        let attribute = if let Some(ref attr) = payload.index_name {
            attr.clone()
        } else {
            // Auto-resolve: need exactly one vector index.
            let count = registry.vector_index_count();
            if count == 0 {
                error_resp!(format!("no vector index on map {}", payload.map_name));
            }
            if count > 1 {
                error_resp!(format!(
                    "map {} has multiple vector indexes; specify indexName",
                    payload.map_name
                ));
            }
            match registry.first_vector_index_attribute() {
                Some(a) => a,
                None => error_resp!(format!("no vector index on map {}", payload.map_name)),
            }
        };

        // 3. Fetch vector index handle.
        let Some(vector_index) = registry.get_vector_index(&attribute) else {
            error_resp!(format!("attribute {attribute} is not a vector index"));
        };

        // 4. Decode query vector.
        let dim = vector_index.dimension();
        let query_f32 = match payload.decode_query_vector(Some(dim)) {
            Ok(v) => v,
            Err(e) => error_resp!(e),
        };

        // 5. Run ANN search in spawn_blocking to keep tokio worker responsive.
        let k = payload.k as usize;
        let ef = payload.ef_search.map_or(k * 2, |v| v as usize).max(k);
        let overfetch = (k * 4).max(k);
        let raw: Vec<(String, f64)> = tokio::task::spawn_blocking({
            let vi = Arc::clone(&vector_index);
            let qf = query_f32.clone();
            move || vi.search_nearest(&qf, overfetch, ef)
        })
        .await
        .map_err(|e| OperationError::Internal(anyhow::anyhow!("vector search task join: {e}")))?;
        let total_candidates = u32::try_from(raw.len()).unwrap_or(u32::MAX);

        // 6. Convert distance → score per metric (R3 policy).
        let metric = vector_index.distance_metric();
        let mut scored: Vec<(String, f64)> = raw
            .into_iter()
            .map(|(key, dist)| {
                let score = match metric {
                    DistanceMetric::Cosine => 1.0 - dist,
                    DistanceMetric::Euclidean | DistanceMetric::Manhattan => 1.0 / (1.0 + dist),
                    DistanceMetric::DotProduct => -dist,
                };
                (key, score)
            })
            .collect();

        // 7. Apply min_score filter.
        let min_score = payload.options.as_ref().and_then(|o| o.min_score);
        if let Some(threshold) = min_score {
            scored.retain(|(_, score)| *score >= threshold);
        }

        // 8. Apply post-filter predicate (best-effort: scalar leaf ops only).
        let filter = payload.options.as_ref().and_then(|o| o.filter.as_ref());
        let stores = self.record_store_factory.get_all_for_map(&payload.map_name);

        // Build a lookup table (key → rmpv::Value) once if we need it for filtering or include_value.
        let include_value = payload
            .options
            .as_ref()
            .and_then(|o| o.include_value)
            .unwrap_or(true);
        let include_vectors = payload
            .options
            .as_ref()
            .and_then(|o| o.include_vectors)
            .unwrap_or(false);
        let need_values = filter.is_some() || include_value || include_vectors;

        let value_map: std::collections::HashMap<String, rmpv::Value> = if need_values {
            let mut map = std::collections::HashMap::new();
            for store in &stores {
                store.for_each_boxed(
                    &mut |key, record| {
                        if let RecordValue::Lww { ref value, .. } = record.value {
                            map.insert(key.to_string(), value_to_rmpv(value));
                        }
                    },
                    false,
                );
            }
            map
        } else {
            std::collections::HashMap::new()
        };

        if let Some(pred) = filter {
            scored.retain(|(key, _)| {
                if let Some(val) = value_map.get(key) {
                    evaluate_predicate(pred, &EvalContext::data_only(val))
                } else {
                    false
                }
            });
        }

        // 9. Sort by descending score, truncate to top-k.
        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        scored.truncate(k);

        // 10. Build result entries.
        let extractor = AttributeExtractor::new(attribute.clone());
        let results: Vec<VectorSearchResult> = scored
            .into_iter()
            .map(|(key, score)| {
                let value = if include_value {
                    value_map.get(&key).cloned()
                } else {
                    None
                };
                let vector = if include_vectors {
                    value_map
                        .get(&key)
                        .and_then(|rec| extract_vector_bytes_le(rec, &extractor))
                } else {
                    None
                };
                VectorSearchResult {
                    key,
                    score,
                    value,
                    vector,
                }
            })
            .collect();

        let resp_payload = VectorSearchRespPayload {
            id: payload.id.clone(),
            results,
            total_candidates,
            search_time_ms: elapsed_ms(start),
            error: None,
        };
        Ok(OperationResponse::Message(Box::new(
            Message::VectorSearchResp {
                payload: resp_payload,
            },
        )))
    }
}

// ---------------------------------------------------------------------------
// Vector extraction helper
// ---------------------------------------------------------------------------

/// Extracts the vector attribute binary from a record and returns its raw
/// little-endian f32 bytes, suitable for `VectorSearchResult.vector`.
///
/// Returns `None` (with a warn log) on any failure: missing attribute,
/// non-Binary field, or `MsgPack` decode error. Non-fatal: callers continue
/// building the response with `vector: None` for that row.
fn extract_vector_bytes_le(
    record: &rmpv::Value,
    extractor: &AttributeExtractor,
) -> Option<Vec<u8>> {
    let field = extractor.extract(record);
    let bytes = if let rmpv::Value::Binary(b) = &field {
        b.as_slice()
    } else {
        tracing::warn!(
            attribute = extractor.attribute_name(),
            "include_vectors: attribute field is not Binary"
        );
        return None;
    };
    match rmp_serde::from_slice::<topgun_core::vector::Vector>(bytes) {
        Ok(v) => Some(v.to_f32_bytes_le()),
        Err(e) => {
            tracing::warn!(
                attribute = extractor.attribute_name(),
                error = %e,
                "include_vectors: failed to decode vector binary"
            );
            None
        }
    }
}

// ---------------------------------------------------------------------------
// RecordBatch to MsgPack row conversion
// ---------------------------------------------------------------------------

/// Converts Arrow `RecordBatch`es to column names and rows of `rmpv::Value`.
///
/// Used to serialize SQL query results for wire transport via `rmpv::Value`
/// rows instead of Arrow IPC, ensuring cross-language client compatibility.
#[cfg(feature = "datafusion")]
fn record_batches_to_rows(
    batches: &[arrow::array::RecordBatch],
) -> (Vec<String>, Vec<Vec<rmpv::Value>>) {
    if batches.is_empty() {
        return (vec![], vec![]);
    }

    let schema = batches[0].schema();
    let columns: Vec<String> = schema.fields().iter().map(|f| f.name().clone()).collect();
    let mut rows: Vec<Vec<rmpv::Value>> = Vec::new();

    for batch in batches {
        let num_rows = batch.num_rows();
        for row_idx in 0..num_rows {
            let mut row = Vec::with_capacity(batch.num_columns());
            for col_idx in 0..batch.num_columns() {
                let col = batch.column(col_idx);
                let value = arrow_value_to_rmpv(col.as_ref(), row_idx);
                row.push(value);
            }
            rows.push(row);
        }
    }

    (columns, rows)
}

/// Converts a single Arrow array value at `row_idx` to `rmpv::Value`.
#[cfg(feature = "datafusion")]
fn arrow_value_to_rmpv(array: &dyn arrow::array::Array, row_idx: usize) -> rmpv::Value {
    use arrow::array::{
        BinaryArray, BooleanArray, Float32Array, Float64Array, Int32Array, Int64Array,
        LargeStringArray, StringArray, TimestampMicrosecondArray, UInt32Array, UInt64Array,
    };
    use arrow::datatypes::DataType;

    if array.is_null(row_idx) {
        return rmpv::Value::Nil;
    }

    match array.data_type() {
        DataType::Int32 => {
            let arr = array.as_any().downcast_ref::<Int32Array>().unwrap();
            rmpv::Value::Integer(i64::from(arr.value(row_idx)).into())
        }
        DataType::Int64 => {
            let arr = array.as_any().downcast_ref::<Int64Array>().unwrap();
            rmpv::Value::Integer(arr.value(row_idx).into())
        }
        DataType::UInt32 => {
            let arr = array.as_any().downcast_ref::<UInt32Array>().unwrap();
            rmpv::Value::Integer(u64::from(arr.value(row_idx)).into())
        }
        DataType::UInt64 => {
            let arr = array.as_any().downcast_ref::<UInt64Array>().unwrap();
            rmpv::Value::Integer(arr.value(row_idx).into())
        }
        DataType::Float32 => {
            let arr = array.as_any().downcast_ref::<Float32Array>().unwrap();
            rmpv::Value::F64(f64::from(arr.value(row_idx)))
        }
        DataType::Float64 => {
            let arr = array.as_any().downcast_ref::<Float64Array>().unwrap();
            rmpv::Value::F64(arr.value(row_idx))
        }
        DataType::Boolean => {
            let arr = array.as_any().downcast_ref::<BooleanArray>().unwrap();
            rmpv::Value::Boolean(arr.value(row_idx))
        }
        DataType::Utf8 => {
            let arr = array.as_any().downcast_ref::<StringArray>().unwrap();
            rmpv::Value::String(arr.value(row_idx).into())
        }
        DataType::LargeUtf8 => {
            let arr = array.as_any().downcast_ref::<LargeStringArray>().unwrap();
            rmpv::Value::String(arr.value(row_idx).into())
        }
        DataType::Binary => {
            let arr = array.as_any().downcast_ref::<BinaryArray>().unwrap();
            rmpv::Value::Binary(arr.value(row_idx).to_vec())
        }
        DataType::Timestamp(arrow::datatypes::TimeUnit::Microsecond, _) => {
            let arr = array
                .as_any()
                .downcast_ref::<TimestampMicrosecondArray>()
                .unwrap();
            rmpv::Value::Integer(arr.value(row_idx).into())
        }
        // Fallback: use debug representation for unsupported types
        _ => {
            let fmt_opts = arrow::util::display::FormatOptions::default();
            match arrow::util::display::ArrayFormatter::try_new(array, &fmt_opts) {
                Ok(fmt) => {
                    let display = fmt.value(row_idx);
                    rmpv::Value::String(display.to_string().into())
                }
                Err(_) => {
                    rmpv::Value::String(format!("<unsupported: {:?}>", array.data_type()).into())
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Extracts an `rmpv::Value` from a `RecordValue`.
///
/// For LWW records, converts the inner `Value` to `rmpv::Value`.
/// For `OrMap`/`OrTombstones`, returns Nil (not supported in v1.0 query evaluation).
fn extract_rmpv_value(record_value: &RecordValue) -> rmpv::Value {
    match record_value {
        RecordValue::Lww { ref value, .. } => value_to_rmpv(value),
        RecordValue::OrMap { .. } | RecordValue::OrTombstones { .. } => rmpv::Value::Nil,
    }
}

// ---------------------------------------------------------------------------
// Field projection helper
// ---------------------------------------------------------------------------

/// Projects a record to include only the specified fields.
///
/// Strips non-projected fields from a Map value, returning the projected subset.
/// If the record is not a Map, returns it unchanged (cloned).
#[must_use]
pub(crate) fn project_fields(fields: &[String], record: &rmpv::Value) -> rmpv::Value {
    let Some(map) = record.as_map() else {
        return record.clone();
    };

    let projected: Vec<(rmpv::Value, rmpv::Value)> = map
        .iter()
        .filter(|(k, _)| {
            k.as_str()
                .is_some_and(|key_str| fields.iter().any(|f| f == key_str))
        })
        .cloned()
        .collect();

    rmpv::Value::Map(projected)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(
    clippy::doc_markdown,
    clippy::doc_lazy_continuation,
    clippy::uninlined_format_args
)]
mod tests {
    use std::collections::BTreeMap;

    use dashmap::DashSet;
    use topgun_core::hlc::Timestamp;
    use topgun_core::messages::base::{PredicateNode, PredicateOp, Query};
    use topgun_core::messages::query::{
        QuerySubMessage, QuerySubPayload, QueryUnsubMessage, QueryUnsubPayload,
    };
    use topgun_core::types::Value;
    use tower::ServiceExt;

    use super::*;
    use crate::network::config::ConnectionConfig;
    use crate::network::connection::{ConnectionId, ConnectionKind, ConnectionRegistry};
    use crate::service::operation::{service_names, OperationContext};
    use crate::storage::datastores::NullDataStore;
    use crate::storage::impls::StorageConfig;
    use crate::storage::record::{Record, RecordMetadata, RecordValue};
    use crate::storage::RecordStoreFactory;

    fn make_timestamp() -> Timestamp {
        Timestamp {
            millis: 1_700_000_000_000,
            counter: 0,
            node_id: "test-node".to_string(),
        }
    }

    fn make_ctx(conn_id: Option<ConnectionId>) -> OperationContext {
        let mut ctx = OperationContext::new(1, service_names::QUERY, make_timestamp(), 5000);
        ctx.connection_id = conn_id;
        ctx
    }

    fn make_record(value: Value) -> Record {
        Record {
            value: RecordValue::Lww {
                value,
                timestamp: make_timestamp(),
            },
            metadata: RecordMetadata::new(1_700_000_000_000, 64),
        }
    }

    fn make_value_map(pairs: Vec<(&str, Value)>) -> Value {
        let mut map = BTreeMap::new();
        for (k, v) in pairs {
            map.insert(k.to_string(), v);
        }
        Value::Map(map)
    }

    fn make_factory() -> Arc<RecordStoreFactory> {
        Arc::new(RecordStoreFactory::new(
            StorageConfig::default(),
            Arc::new(NullDataStore),
            Vec::new(),
        ))
    }

    fn test_config() -> ConnectionConfig {
        ConnectionConfig::default()
    }

    /// Builds a `live_window` for a test subscription whose membership matches the supplied
    /// pre-existing keys. Tests simulate "this key was already in the result set" by
    /// pre-populating `previous_result_keys`; the window must start with the same membership
    /// or the observer (which now derives ENTER/UPDATE/LEAVE from the window) would emit
    /// ENTER where the test expects UPDATE/LEAVE. Placeholder values are sufficient because
    /// these tests use unbounded (`limit = None`) queries where membership is presence-only.
    fn seeded_window(query: &Query, present_keys: &[&str]) -> Arc<LiveWindow> {
        let window = Arc::new(LiveWindow::new(
            query.sort.clone().unwrap_or_default(),
            query.limit,
        ));
        for key in present_keys {
            let _ = window.apply_mutation(key, Some(&rmpv::Value::Nil), true);
        }
        window
    }

    // ---- QueryRegistry tests ----

    #[test]
    fn registry_register_and_count() {
        let registry = QueryRegistry::new();
        assert_eq!(registry.subscription_count(), 0);

        let sub = QuerySubscription {
            query_id: "q-1".to_string(),
            connection_id: ConnectionId(1),
            map_name: "users".to_string(),
            live_window: seeded_window(&Query::default(), &[]),
            query: Query::default(),
            previous_result_keys: DashSet::new(),
            fields: None,
        };
        registry.register(sub);
        assert_eq!(registry.subscription_count(), 1);
    }

    #[test]
    fn registry_unregister_returns_true_when_found() {
        let registry = QueryRegistry::new();
        registry.register(QuerySubscription {
            query_id: "q-1".to_string(),
            connection_id: ConnectionId(1),
            map_name: "users".to_string(),
            live_window: seeded_window(&Query::default(), &[]),
            query: Query::default(),
            previous_result_keys: DashSet::new(),
            fields: None,
        });

        assert!(registry.unregister("q-1"));
        assert_eq!(registry.subscription_count(), 0);
    }

    #[test]
    fn registry_unregister_returns_false_when_not_found() {
        let registry = QueryRegistry::new();
        assert!(!registry.unregister("nonexistent"));
    }

    #[test]
    fn registry_unregister_by_connection() {
        let registry = QueryRegistry::new();
        registry.register(QuerySubscription {
            query_id: "q-1".to_string(),
            connection_id: ConnectionId(1),
            map_name: "users".to_string(),
            live_window: seeded_window(&Query::default(), &[]),
            query: Query::default(),
            previous_result_keys: DashSet::new(),
            fields: None,
        });
        registry.register(QuerySubscription {
            query_id: "q-2".to_string(),
            connection_id: ConnectionId(2),
            map_name: "users".to_string(),
            live_window: seeded_window(&Query::default(), &[]),
            query: Query::default(),
            previous_result_keys: DashSet::new(),
            fields: None,
        });
        registry.register(QuerySubscription {
            query_id: "q-3".to_string(),
            connection_id: ConnectionId(1),
            map_name: "orders".to_string(),
            live_window: seeded_window(&Query::default(), &[]),
            query: Query::default(),
            previous_result_keys: DashSet::new(),
            fields: None,
        });

        registry.unregister_by_connection(ConnectionId(1));
        assert_eq!(registry.subscription_count(), 1);

        let subs = registry.get_subscriptions_for_map("users");
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].query_id, "q-2");
    }

    #[test]
    fn registry_get_subscriptions_for_map() {
        let registry = QueryRegistry::new();
        registry.register(QuerySubscription {
            query_id: "q-1".to_string(),
            connection_id: ConnectionId(1),
            map_name: "users".to_string(),
            live_window: seeded_window(&Query::default(), &[]),
            query: Query::default(),
            previous_result_keys: DashSet::new(),
            fields: None,
        });
        registry.register(QuerySubscription {
            query_id: "q-2".to_string(),
            connection_id: ConnectionId(1),
            map_name: "orders".to_string(),
            live_window: seeded_window(&Query::default(), &[]),
            query: Query::default(),
            previous_result_keys: DashSet::new(),
            fields: None,
        });

        let subs = registry.get_subscriptions_for_map("users");
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].query_id, "q-1");

        let subs = registry.get_subscriptions_for_map("nonexistent");
        assert!(subs.is_empty());
    }

    // ---- QueryMutationObserver tests ----

    #[test]
    fn observer_skips_backup_partitions() {
        let registry = Arc::new(QueryRegistry::new());
        let conn_registry = Arc::new(ConnectionRegistry::new());
        let observer =
            QueryMutationObserver::new(registry.clone(), conn_registry, "users".to_string(), 0);

        // Register a subscription
        let prev_keys = DashSet::new();
        registry.register(QuerySubscription {
            query_id: "q-1".to_string(),
            connection_id: ConnectionId(1),
            map_name: "users".to_string(),
            live_window: seeded_window(&Query::default(), &[]),
            query: Query::default(),
            previous_result_keys: prev_keys,
            fields: None,
        });

        let record = make_record(make_value_map(vec![(
            "name",
            Value::String("Alice".to_string()),
        )]));

        // on_put with is_backup=true should be a no-op
        observer.on_put("key1", &record, None, true);

        // The subscription's previous_result_keys should still be empty
        let subs = registry.get_subscriptions_for_map("users");
        assert!(subs[0].previous_result_keys.is_empty());
    }

    #[test]
    fn observer_enter_on_new_match() {
        let registry = Arc::new(QueryRegistry::new());
        let conn_registry = Arc::new(ConnectionRegistry::new());
        let config = test_config();
        let (handle, mut rx) = conn_registry.register(ConnectionKind::Client, &config);
        let conn_id = handle.id;

        let observer =
            QueryMutationObserver::new(registry.clone(), conn_registry, "users".to_string(), 0);

        // Register subscription with no predicate (match all)
        registry.register(QuerySubscription {
            query_id: "q-1".to_string(),
            connection_id: conn_id,
            map_name: "users".to_string(),
            live_window: seeded_window(&Query::default(), &[]),
            query: Query::default(),
            previous_result_keys: DashSet::new(),
            fields: None,
        });

        let record = make_record(make_value_map(vec![(
            "name",
            Value::String("Alice".to_string()),
        )]));
        observer.on_put("key1", &record, None, false);

        // Should have received an ENTER event
        let msg = rx.try_recv().expect("should have received a message");
        match msg {
            OutboundMessage::Binary(bytes) => {
                let decoded: Message = rmp_serde::from_slice(&bytes).expect("decode");
                match decoded {
                    Message::QueryUpdate { payload } => {
                        assert_eq!(payload.query_id, "q-1");
                        assert_eq!(payload.key, "key1");
                        assert_eq!(payload.change_type, ChangeEventType::ENTER);
                    }
                    _ => panic!("expected QueryUpdate message"),
                }
            }
            OutboundMessage::Close(_) => panic!("expected Binary message"),
        }

        // Key should now be in previous_result_keys
        let subs = registry.get_subscriptions_for_map("users");
        assert!(subs[0].previous_result_keys.contains("key1"));
    }

    #[test]
    fn observer_update_on_existing_match() {
        let registry = Arc::new(QueryRegistry::new());
        let conn_registry = Arc::new(ConnectionRegistry::new());
        let config = test_config();
        let (handle, mut rx) = conn_registry.register(ConnectionKind::Client, &config);
        let conn_id = handle.id;

        let observer =
            QueryMutationObserver::new(registry.clone(), conn_registry, "users".to_string(), 0);

        // Register subscription with key already in previous_result_keys
        let prev_keys = DashSet::new();
        prev_keys.insert("key1".to_string());
        let query = Query::default(); // match all
        registry.register(QuerySubscription {
            query_id: "q-1".to_string(),
            connection_id: conn_id,
            map_name: "users".to_string(),
            live_window: seeded_window(&query, &["key1"]),
            query,
            previous_result_keys: prev_keys,
            fields: None,
        });

        let old_value = RecordValue::Lww {
            value: make_value_map(vec![("name", Value::String("Alice".to_string()))]),
            timestamp: make_timestamp(),
        };
        let new_value = RecordValue::Lww {
            value: make_value_map(vec![("name", Value::String("Alice Updated".to_string()))]),
            timestamp: make_timestamp(),
        };
        let record = Record {
            value: new_value.clone(),
            metadata: RecordMetadata::new(1_700_000_000_000, 64),
        };

        observer.on_update("key1", &record, &old_value, &new_value, false);

        let msg = rx.try_recv().expect("should have received a message");
        match msg {
            OutboundMessage::Binary(bytes) => {
                let decoded: Message = rmp_serde::from_slice(&bytes).expect("decode");
                match decoded {
                    Message::QueryUpdate { payload } => {
                        assert_eq!(payload.change_type, ChangeEventType::UPDATE);
                        assert_eq!(payload.key, "key1");
                    }
                    _ => panic!("expected QueryUpdate"),
                }
            }
            OutboundMessage::Close(_) => panic!("expected Binary"),
        }
    }

    #[test]
    fn observer_leave_on_no_longer_matching() {
        let registry = Arc::new(QueryRegistry::new());
        let conn_registry = Arc::new(ConnectionRegistry::new());
        let config = test_config();
        let (handle, mut rx) = conn_registry.register(ConnectionKind::Client, &config);
        let conn_id = handle.id;

        let observer =
            QueryMutationObserver::new(registry.clone(), conn_registry, "users".to_string(), 0);

        // Subscription requires age >= 18
        let prev_keys = DashSet::new();
        prev_keys.insert("key1".to_string());
        let query = Query {
            predicate: Some(PredicateNode {
                op: PredicateOp::Gte,
                attribute: Some("age".to_string()),
                value: Some(rmpv::Value::Integer(18.into())),
                ..Default::default()
            }),
            ..Query::default()
        };
        registry.register(QuerySubscription {
            query_id: "q-1".to_string(),
            connection_id: conn_id,
            map_name: "users".to_string(),
            live_window: seeded_window(&query, &["key1"]),
            query,
            previous_result_keys: prev_keys,
            fields: None,
        });

        // Update to age=10 (no longer matches)
        let record = make_record(make_value_map(vec![("age", Value::Int(10))]));
        observer.on_put("key1", &record, None, false);

        let msg = rx.try_recv().expect("should have received a message");
        match msg {
            OutboundMessage::Binary(bytes) => {
                let decoded: Message = rmp_serde::from_slice(&bytes).expect("decode");
                match decoded {
                    Message::QueryUpdate { payload } => {
                        assert_eq!(payload.change_type, ChangeEventType::LEAVE);
                        assert_eq!(payload.key, "key1");
                    }
                    _ => panic!("expected QueryUpdate"),
                }
            }
            OutboundMessage::Close(_) => panic!("expected Binary"),
        }

        // Key should have been removed from previous_result_keys
        let subs = registry.get_subscriptions_for_map("users");
        assert!(!subs[0].previous_result_keys.contains("key1"));
    }

    #[test]
    fn observer_leave_on_remove() {
        let registry = Arc::new(QueryRegistry::new());
        let conn_registry = Arc::new(ConnectionRegistry::new());
        let config = test_config();
        let (handle, mut rx) = conn_registry.register(ConnectionKind::Client, &config);
        let conn_id = handle.id;

        let observer =
            QueryMutationObserver::new(registry.clone(), conn_registry, "users".to_string(), 0);

        let prev_keys = DashSet::new();
        prev_keys.insert("key1".to_string());
        let query = Query::default();
        registry.register(QuerySubscription {
            query_id: "q-1".to_string(),
            connection_id: conn_id,
            map_name: "users".to_string(),
            live_window: seeded_window(&query, &["key1"]),
            query,
            previous_result_keys: prev_keys,
            fields: None,
        });

        let record = make_record(make_value_map(vec![(
            "name",
            Value::String("Alice".to_string()),
        )]));
        observer.on_remove("key1", &record, false);

        let msg = rx.try_recv().expect("should have received a message");
        match msg {
            OutboundMessage::Binary(bytes) => {
                let decoded: Message = rmp_serde::from_slice(&bytes).expect("decode");
                match decoded {
                    Message::QueryUpdate { payload } => {
                        assert_eq!(payload.change_type, ChangeEventType::LEAVE);
                    }
                    _ => panic!("expected QueryUpdate"),
                }
            }
            OutboundMessage::Close(_) => panic!("expected Binary"),
        }
    }

    #[test]
    fn observer_on_clear_sends_leave_for_all_previous_keys() {
        let registry = Arc::new(QueryRegistry::new());
        let conn_registry = Arc::new(ConnectionRegistry::new());
        let config = test_config();
        let (handle, mut rx) = conn_registry.register(ConnectionKind::Client, &config);
        let conn_id = handle.id;

        let observer =
            QueryMutationObserver::new(registry.clone(), conn_registry, "users".to_string(), 0);

        let prev_keys = DashSet::new();
        prev_keys.insert("k1".to_string());
        prev_keys.insert("k2".to_string());
        let query = Query::default();
        registry.register(QuerySubscription {
            query_id: "q-1".to_string(),
            connection_id: conn_id,
            map_name: "users".to_string(),
            live_window: seeded_window(&query, &["k1", "k2"]),
            query,
            previous_result_keys: prev_keys,
            fields: None,
        });

        observer.on_clear();

        // Should receive 2 LEAVE messages
        let mut leave_count = 0;
        while let Ok(msg) = rx.try_recv() {
            match msg {
                OutboundMessage::Binary(bytes) => {
                    let decoded: Message = rmp_serde::from_slice(&bytes).expect("decode");
                    match decoded {
                        Message::QueryUpdate { payload } => {
                            assert_eq!(payload.change_type, ChangeEventType::LEAVE);
                            assert_eq!(payload.value, rmpv::Value::Nil);
                            leave_count += 1;
                        }
                        _ => panic!("expected QueryUpdate"),
                    }
                }
                OutboundMessage::Close(_) => panic!("expected Binary"),
            }
        }
        assert_eq!(leave_count, 2);

        // previous_result_keys should be cleared
        let subs = registry.get_subscriptions_for_map("users");
        assert!(subs[0].previous_result_keys.is_empty());
    }

    #[test]
    fn observer_noop_no_matching_key_not_in_previous() {
        let registry = Arc::new(QueryRegistry::new());
        let conn_registry = Arc::new(ConnectionRegistry::new());
        let config = test_config();
        let (handle, mut rx) = conn_registry.register(ConnectionKind::Client, &config);
        let conn_id = handle.id;

        let observer =
            QueryMutationObserver::new(registry.clone(), conn_registry, "users".to_string(), 0);

        // Subscription requires age >= 18; key1 not in previous
        registry.register(QuerySubscription {
            query_id: "q-1".to_string(),
            connection_id: conn_id,
            map_name: "users".to_string(),
            live_window: seeded_window(&Query::default(), &[]),
            query: Query {
                predicate: Some(PredicateNode {
                    op: PredicateOp::Gte,
                    attribute: Some("age".to_string()),
                    value: Some(rmpv::Value::Integer(18.into())),
                    ..Default::default()
                }),
                ..Query::default()
            },
            previous_result_keys: DashSet::new(),
            fields: None,
        });

        // Put a record that does NOT match (age=10)
        let record = make_record(make_value_map(vec![("age", Value::Int(10))]));
        observer.on_put("key1", &record, None, false);

        // No message should be sent
        assert!(rx.try_recv().is_err());
    }

    // ---- QueryService tests ----

    #[tokio::test]
    async fn query_service_managed_service_name() {
        let registry = Arc::new(QueryRegistry::new());
        let factory = make_factory();
        let conn_registry = Arc::new(ConnectionRegistry::new());
        let svc = QueryService::new(
            registry,
            factory,
            conn_registry,
            None,
            10_000,
            None,
            #[cfg(feature = "datafusion")]
            None,
        );
        assert_eq!(svc.name(), "query");
    }

    #[tokio::test]
    async fn query_service_wrong_operation_returns_wrong_service() {
        let registry = Arc::new(QueryRegistry::new());
        let factory = make_factory();
        let conn_registry = Arc::new(ConnectionRegistry::new());
        let svc = Arc::new(QueryService::new(
            registry,
            factory,
            conn_registry,
            None,
            10_000,
            None,
            #[cfg(feature = "datafusion")]
            None,
        ));

        let ctx = OperationContext::new(1, service_names::QUERY, make_timestamp(), 5000);
        let op = Operation::GarbageCollect { ctx };
        let result = svc.oneshot(op).await;
        assert!(matches!(result, Err(OperationError::WrongService)));
    }

    #[tokio::test]
    async fn query_subscribe_missing_connection_id_returns_error() {
        let registry = Arc::new(QueryRegistry::new());
        let factory = make_factory();
        let conn_registry = Arc::new(ConnectionRegistry::new());
        let svc = Arc::new(QueryService::new(
            registry,
            factory,
            conn_registry,
            None,
            10_000,
            None,
            #[cfg(feature = "datafusion")]
            None,
        ));

        let ctx = make_ctx(None); // no connection_id
        let payload = QuerySubMessage {
            payload: QuerySubPayload {
                query_id: "q-1".to_string(),
                map_name: "users".to_string(),
                query: Query::default(),
                fields: None,
            },
        };
        let op = Operation::QuerySubscribe { ctx, payload };
        let result = svc.oneshot(op).await;
        assert!(matches!(result, Err(OperationError::Internal(_))));
    }

    #[tokio::test]
    async fn query_subscribe_empty_store_returns_empty_results() {
        let registry = Arc::new(QueryRegistry::new());
        let factory = make_factory();
        let conn_registry = Arc::new(ConnectionRegistry::new());
        let config = test_config();
        let (handle, _rx) = conn_registry.register(ConnectionKind::Client, &config);
        let conn_id = handle.id;

        let svc = Arc::new(QueryService::new(
            registry.clone(),
            factory,
            conn_registry,
            None,
            10_000,
            None,
            #[cfg(feature = "datafusion")]
            None,
        ));

        let ctx = make_ctx(Some(conn_id));
        let payload = QuerySubMessage {
            payload: QuerySubPayload {
                query_id: "q-1".to_string(),
                map_name: "users".to_string(),
                query: Query::default(),
                fields: None,
            },
        };
        let op = Operation::QuerySubscribe { ctx, payload };
        let result = svc.oneshot(op).await.unwrap();

        match result {
            OperationResponse::Message(msg) => match *msg {
                Message::QueryResp(resp) => {
                    assert_eq!(resp.payload.query_id, "q-1");
                    assert!(resp.payload.results.is_empty());
                }
                _ => panic!("expected QueryResp"),
            },
            _ => panic!("expected Message response"),
        }

        // Subscription should be registered
        assert_eq!(registry.subscription_count(), 1);
    }

    /// End-to-end through the handler: a multi-field sort query runs via the local
    /// DAG engine (`run_dag_local`) and returns real record keys, clean values, and
    /// the correct lexicographic multi-field order (group asc, then rank asc).
    ///
    /// This exercises the production path `Operation::QuerySubscribe` →
    /// `handle_query_subscribe` → `run_dag_local` with NO coordinator (single-node
    /// default) — unlike the sim test, which calls the coordinator directly. It is
    /// the capability Phase 1 adds to the WS single-node path: multi-field sort,
    /// which the linear predicate engine cannot do.
    #[tokio::test]
    async fn query_subscribe_multi_field_sort_via_dag() {
        use crate::storage::record_store::{CallerProvenance, ExpiryPolicy};
        use topgun_core::messages::base::{SortDirection, SortField};

        let registry = Arc::new(QueryRegistry::new());
        let factory = make_factory();
        let map_name = "events";

        // Tie in the primary field (group) broken by the secondary (rank).
        // Insertion order is deliberately NOT the sorted order.
        let rows = [
            ("k-a2", "a", 2i64),
            ("k-b1", "b", 1),
            ("k-a1", "a", 1),
            ("k-a3", "a", 3),
            ("k-b0", "b", 0),
        ];
        for (key, group, rank) in rows {
            let partition_id = topgun_core::hash_to_partition(key);
            let value = make_value_map(vec![
                ("group", Value::String(group.to_string())),
                ("rank", Value::Int(rank)),
            ]);
            factory
                .get_or_create(map_name, partition_id)
                .put(
                    key,
                    RecordValue::Lww {
                        value,
                        timestamp: make_timestamp(),
                    },
                    ExpiryPolicy::NONE,
                    CallerProvenance::Client,
                )
                .await
                .expect("put should succeed");
        }

        let conn_registry = Arc::new(ConnectionRegistry::new());
        let config = test_config();
        let (handle, _rx) = conn_registry.register(ConnectionKind::Client, &config);
        let conn_id = handle.id;

        // No coordinator → single-node `run_dag_local` is the engine.
        let svc = Arc::new(QueryService::new(
            registry,
            factory,
            conn_registry,
            None,
            10_000,
            None,
            #[cfg(feature = "datafusion")]
            None,
        ));

        let ctx = make_ctx(Some(conn_id));
        let payload = QuerySubMessage {
            payload: QuerySubPayload {
                query_id: "ms-1".to_string(),
                map_name: map_name.to_string(),
                query: Query {
                    sort: Some(vec![
                        SortField {
                            field: "group".to_string(),
                            direction: SortDirection::Asc,
                        },
                        SortField {
                            field: "rank".to_string(),
                            direction: SortDirection::Asc,
                        },
                    ]),
                    ..Query::default()
                },
                fields: None,
            },
        };
        let op = Operation::QuerySubscribe { ctx, payload };
        let resp = match svc.oneshot(op).await.unwrap() {
            OperationResponse::Message(msg) => match *msg {
                Message::QueryResp(resp) => resp,
                _ => panic!("expected QueryResp"),
            },
            _ => panic!("expected Message response"),
        };

        let results = resp.payload.results;
        assert_eq!(results.len(), 5);

        // Real record keys recovered from `_key` (not synthetic `row-{i}`),
        // ordered by group asc then rank asc.
        let keys: Vec<&str> = results.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(
            keys,
            vec!["k-a1", "k-a2", "k-a3", "k-b0", "k-b1"],
            "multi-field sort (group asc, rank asc) with real record keys"
        );

        // Values are clean: the internal `_key` field must be stripped.
        for e in &results {
            match &e.value {
                rmpv::Value::Map(pairs) => assert!(
                    pairs.iter().all(|(k, _)| k.as_str() != Some("_key")),
                    "internal _key must be stripped from the returned value"
                ),
                other => panic!("expected map value, got: {other:?}"),
            }
        }
    }

    /// The tests-only linear-engine opt-out (`with_linear_engine_for_tests`) routes
    /// the handler through the predicate engine instead of the DAG, still returning
    /// real keys and clean values for a simple filter query.
    #[tokio::test]
    async fn query_subscribe_linear_engine_opt_out_filters() {
        use crate::storage::record_store::{CallerProvenance, ExpiryPolicy};

        let registry = Arc::new(QueryRegistry::new());
        let factory = make_factory();
        let map_name = "people";

        for (key, age) in [("p-young", 15i64), ("p-old", 40)] {
            let partition_id = topgun_core::hash_to_partition(key);
            let value = make_value_map(vec![("age", Value::Int(age))]);
            factory
                .get_or_create(map_name, partition_id)
                .put(
                    key,
                    RecordValue::Lww {
                        value,
                        timestamp: make_timestamp(),
                    },
                    ExpiryPolicy::NONE,
                    CallerProvenance::Client,
                )
                .await
                .expect("put should succeed");
        }

        let conn_registry = Arc::new(ConnectionRegistry::new());
        let config = test_config();
        let (handle, _rx) = conn_registry.register(ConnectionKind::Client, &config);
        let conn_id = handle.id;

        let svc = Arc::new(
            QueryService::new(
                registry,
                factory,
                conn_registry,
                None,
                10_000,
                None,
                #[cfg(feature = "datafusion")]
                None,
            )
            .with_linear_engine_for_tests(),
        );

        let ctx = make_ctx(Some(conn_id));
        let payload = QuerySubMessage {
            payload: QuerySubPayload {
                query_id: "lin-1".to_string(),
                map_name: map_name.to_string(),
                query: Query {
                    predicate: Some(PredicateNode {
                        op: PredicateOp::Gte,
                        attribute: Some("age".to_string()),
                        value: Some(rmpv::Value::Integer(18.into())),
                        ..Default::default()
                    }),
                    ..Query::default()
                },
                fields: None,
            },
        };
        let resp = match svc
            .oneshot(Operation::QuerySubscribe { ctx, payload })
            .await
            .unwrap()
        {
            OperationResponse::Message(msg) => match *msg {
                Message::QueryResp(resp) => resp,
                _ => panic!("expected QueryResp"),
            },
            _ => panic!("expected Message response"),
        };

        assert_eq!(resp.payload.results.len(), 1);
        assert_eq!(resp.payload.results[0].key, "p-old");
    }

    /// AC1: `QuerySubscribe` returns `QUERY_RESP` with initial matching results.
    ///
    /// Since `RecordStoreFactory::create()` returns independent stores per call
    /// (each with its own `HashMapStorage`), this test verifies the subscribe
    /// handler by putting data into the factory-created store and then testing
    /// `execute_query` directly. The end-to-end integration with a shared backing
    /// store is tested at a higher integration level.
    #[test]
    fn query_subscribe_execute_query_returns_matching_results() {
        use super::super::predicate::execute_query;

        // Simulate entries as (key, rmpv::Value) that would come from for_each_boxed
        let entries = vec![
            (
                "user-1".to_string(),
                rmpv::Value::Map(vec![
                    (
                        rmpv::Value::String("name".into()),
                        rmpv::Value::String("Alice".into()),
                    ),
                    (
                        rmpv::Value::String("age".into()),
                        rmpv::Value::Integer(30.into()),
                    ),
                ]),
            ),
            (
                "user-2".to_string(),
                rmpv::Value::Map(vec![
                    (
                        rmpv::Value::String("name".into()),
                        rmpv::Value::String("Bob".into()),
                    ),
                    (
                        rmpv::Value::String("age".into()),
                        rmpv::Value::Integer(15.into()),
                    ),
                ]),
            ),
        ];

        let query = Query {
            predicate: Some(PredicateNode {
                op: PredicateOp::Gte,
                attribute: Some("age".to_string()),
                value: Some(rmpv::Value::Integer(18.into())),
                ..Default::default()
            }),
            ..Query::default()
        };

        let results = execute_query(entries, &query);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].key, "user-1");
    }

    #[tokio::test]
    async fn query_unsubscribe_removes_subscription() {
        let registry = Arc::new(QueryRegistry::new());
        let factory = make_factory();
        let conn_registry = Arc::new(ConnectionRegistry::new());
        let config = test_config();
        let (handle, _rx) = conn_registry.register(ConnectionKind::Client, &config);
        let conn_id = handle.id;

        // First subscribe
        let svc = Arc::new(QueryService::new(
            registry.clone(),
            factory,
            conn_registry,
            None,
            10_000,
            None,
            #[cfg(feature = "datafusion")]
            None,
        ));

        let ctx = make_ctx(Some(conn_id));
        let sub_payload = QuerySubMessage {
            payload: QuerySubPayload {
                query_id: "q-1".to_string(),
                map_name: "users".to_string(),
                query: Query::default(),
                fields: None,
            },
        };
        let op = Operation::QuerySubscribe {
            ctx: ctx.clone(),
            payload: sub_payload,
        };
        let _ = svc.clone().oneshot(op).await.unwrap();
        assert_eq!(registry.subscription_count(), 1);

        // Now unsubscribe
        let unsub_payload = QueryUnsubMessage {
            payload: QueryUnsubPayload {
                query_id: "q-1".to_string(),
            },
        };
        let op = Operation::QueryUnsubscribe {
            ctx,
            payload: unsub_payload,
        };
        let result = svc.oneshot(op).await.unwrap();
        assert!(matches!(result, OperationResponse::Empty));
        assert_eq!(registry.subscription_count(), 0);
    }

    // ---- get_subscribed_connection_ids tests (AC1, AC2) ----

    #[test]
    fn get_subscribed_connection_ids_empty_when_no_subscriptions() {
        let registry = QueryRegistry::new();
        let ids = registry.get_subscribed_connection_ids("users");
        assert!(ids.is_empty(), "expected empty set for unsubscribed map");
    }

    #[test]
    fn get_subscribed_connection_ids_returns_subscribers() {
        let registry = QueryRegistry::new();

        let conn1 = ConnectionId(1);
        let conn2 = ConnectionId(2);

        registry.register(QuerySubscription {
            query_id: "q-1".to_string(),
            connection_id: conn1,
            map_name: "users".to_string(),
            live_window: seeded_window(&Query::default(), &[]),
            query: Query {
                predicate: None,
                r#where: None,
                sort: None,
                limit: None,
                cursor: None,
                group_by: None,
                aggregations: None,
            },
            previous_result_keys: DashSet::new(),
            fields: None,
        });

        registry.register(QuerySubscription {
            query_id: "q-2".to_string(),
            connection_id: conn2,
            map_name: "users".to_string(),
            live_window: seeded_window(&Query::default(), &[]),
            query: Query {
                predicate: None,
                r#where: None,
                sort: None,
                limit: None,
                cursor: None,
                group_by: None,
                aggregations: None,
            },
            previous_result_keys: DashSet::new(),
            fields: None,
        });

        let ids = registry.get_subscribed_connection_ids("users");
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&conn1));
        assert!(ids.contains(&conn2));
    }

    #[test]
    fn get_subscribed_connection_ids_deduplicates_same_connection() {
        let registry = QueryRegistry::new();

        let conn1 = ConnectionId(1);

        // Same connection with two different queries on the same map
        registry.register(QuerySubscription {
            query_id: "q-1".to_string(),
            connection_id: conn1,
            map_name: "users".to_string(),
            live_window: seeded_window(&Query::default(), &[]),
            query: Query {
                predicate: None,
                r#where: None,
                sort: None,
                limit: None,
                cursor: None,
                group_by: None,
                aggregations: None,
            },
            previous_result_keys: DashSet::new(),
            fields: None,
        });
        registry.register(QuerySubscription {
            query_id: "q-2".to_string(),
            connection_id: conn1,
            map_name: "users".to_string(),
            live_window: seeded_window(&Query::default(), &[]),
            query: Query {
                predicate: None,
                r#where: None,
                sort: None,
                limit: None,
                cursor: None,
                group_by: None,
                aggregations: None,
            },
            previous_result_keys: DashSet::new(),
            fields: None,
        });

        let ids = registry.get_subscribed_connection_ids("users");
        assert_eq!(ids.len(), 1, "same connection should appear only once");
        assert!(ids.contains(&conn1));
    }

    // ---- G4: Field projection, clamping, Merkle sync init tests ----

    /// AC1: QUERY_SUB with `fields: ["name"]` returns QUERY_RESP where every
    /// result value has only the `name` field.
    #[test]
    fn field_projection_on_query_resp() {
        // Simulate a record with multiple fields
        let value = rmpv::Value::Map(vec![
            (
                rmpv::Value::String("name".into()),
                rmpv::Value::String("Alice".into()),
            ),
            (
                rmpv::Value::String("age".into()),
                rmpv::Value::Integer(30.into()),
            ),
            (
                rmpv::Value::String("email".into()),
                rmpv::Value::String("alice@test.com".into()),
            ),
        ]);

        let fields = vec!["name".to_string()];
        let projected = project_fields(&fields, &value);

        // Projected value should only contain "name"
        let map = projected.as_map().expect("projected should be a map");
        assert_eq!(map.len(), 1);
        assert_eq!(map[0].0.as_str().unwrap(), "name");
        assert_eq!(map[0].1.as_str().unwrap(), "Alice");
    }

    /// AC7: QUERY_RESP with results exceeding `max_query_records` returns
    /// exactly `max_query_records` entries with `has_more: true`.
    ///
    /// Mirrors the exact clamping-then-projection order in `handle_query_subscribe()`:
    /// 1. `execute_query` returns all matches
    /// 2. Truncate to `max_query_records` → `has_more: true`
    /// 3. Apply field projection on the clamped set
    /// This ensures a regression in ordering (e.g., projecting before clamping)
    /// would be caught.
    #[test]
    fn max_query_records_clamping() {
        use super::super::predicate::execute_query;
        // Create 15 entries with two fields each
        let entries: Vec<(String, rmpv::Value)> = (0..15)
            .map(|i| {
                (
                    format!("key-{i}"),
                    rmpv::Value::Map(vec![
                        (
                            rmpv::Value::String("id".into()),
                            rmpv::Value::Integer(i.into()),
                        ),
                        (
                            rmpv::Value::String("name".into()),
                            rmpv::Value::String(format!("item-{i}").into()),
                        ),
                    ]),
                )
            })
            .collect();

        let query = Query::default(); // no filter: match all
        let mut results = execute_query(entries, &query);
        assert_eq!(results.len(), 15);

        // Step 1: Clamp (same as handle_query_subscribe)
        let max = 10_usize;
        let has_more = if results.len() > max {
            results.truncate(max);
            Some(true)
        } else {
            None
        };

        assert_eq!(results.len(), 10);
        assert_eq!(has_more, Some(true));

        // Step 2: Project fields on the clamped set (same order as service)
        let proj_fields = vec!["name".to_string()];
        for entry in &mut results {
            entry.value = project_fields(&proj_fields, &entry.value);
        }

        // Verify projection applied to clamped results
        assert_eq!(results.len(), 10);
        for entry in &results {
            let map = entry.value.as_map().expect("projected value should be Map");
            assert_eq!(map.len(), 1, "projected entry should have exactly 1 field");
            assert_eq!(map[0].0.as_str().unwrap(), "name");
        }
    }

    /// AC8: `ServerConfig::default().max_query_records` equals `10_000`.
    #[test]
    fn server_config_default_max_query_records() {
        use crate::service::config::ServerConfig;
        let config = ServerConfig::default();
        assert_eq!(config.max_query_records, 10_000);
    }

    /// AC4: QUERY_RESP includes `merkle_root_hash` computed from per-query Merkle trees.
    /// AC5: QUERY_SYNC_INIT with matching root_hash returns SyncRespRoot with same hash.
    #[tokio::test]
    async fn query_sync_init_matching_hash_returns_same_hash() {
        use crate::storage::query_merkle::QueryMerkleSyncManager;
        use topgun_core::messages::query::{QuerySyncInitMessage, QuerySyncInitPayload};

        let registry = Arc::new(QueryRegistry::new());
        let factory = make_factory();
        let conn_registry = Arc::new(ConnectionRegistry::new());
        let config = test_config();
        let (_handle, _rx) = conn_registry.register(ConnectionKind::Client, &config);

        let merkle_mgr = Arc::new(QueryMerkleSyncManager::new());

        let svc = Arc::new(QueryService::new(
            registry.clone(),
            factory,
            conn_registry,
            Some(Arc::clone(&merkle_mgr)),
            10_000,
            None,
            #[cfg(feature = "datafusion")]
            None,
        ));

        // Register a subscription manually so QUERY_SYNC_INIT can find it.
        registry.register(QuerySubscription {
            query_id: "q-sync-1".to_string(),
            connection_id: ConnectionId(1),
            map_name: "users".to_string(),
            live_window: seeded_window(&Query::default(), &[]),
            query: Query::default(),
            previous_result_keys: DashSet::new(),
            fields: None,
        });

        // Init a Merkle tree for this query
        merkle_mgr.init_tree("q-sync-1", "users", 0, &[("k1".to_string(), 42)]);
        let server_hash = merkle_mgr.aggregate_query_root_hash("q-sync-1", "users");
        assert_ne!(server_hash, 0);

        // Send QUERY_SYNC_INIT with the server's hash (matching).
        let ctx = make_ctx(Some(ConnectionId(1)));
        let payload = QuerySyncInitMessage {
            payload: QuerySyncInitPayload {
                query_id: "q-sync-1".to_string(),
                root_hash: server_hash,
            },
        };
        let op = Operation::QuerySyncInit { ctx, payload };
        let result = svc.oneshot(op).await.unwrap();

        // Should get SyncRespRoot with the same hash.
        match result {
            OperationResponse::Message(msg) => {
                if let Message::SyncRespRoot(resp) = *msg {
                    assert_eq!(resp.payload.root_hash, server_hash);
                    assert_eq!(resp.payload.map_name, "users");
                } else {
                    panic!("Expected SyncRespRoot, got {:?}", msg);
                }
            }
            other => panic!("Expected Message response, got {:?}", other),
        }
    }

    /// AC6: QUERY_SYNC_INIT with stale root_hash returns SyncRespRoot with server's current hash.
    #[tokio::test]
    async fn query_sync_init_different_hash_returns_server_hash() {
        use crate::storage::query_merkle::QueryMerkleSyncManager;
        use topgun_core::messages::query::{QuerySyncInitMessage, QuerySyncInitPayload};

        let registry = Arc::new(QueryRegistry::new());
        let factory = make_factory();
        let conn_registry = Arc::new(ConnectionRegistry::new());
        let config = test_config();
        let (_handle, _rx) = conn_registry.register(ConnectionKind::Client, &config);

        let merkle_mgr = Arc::new(QueryMerkleSyncManager::new());

        let svc = Arc::new(QueryService::new(
            registry.clone(),
            factory,
            conn_registry,
            Some(Arc::clone(&merkle_mgr)),
            10_000,
            None,
            #[cfg(feature = "datafusion")]
            None,
        ));

        // Register a subscription
        registry.register(QuerySubscription {
            query_id: "q-sync-2".to_string(),
            connection_id: ConnectionId(1),
            map_name: "users".to_string(),
            live_window: seeded_window(&Query::default(), &[]),
            query: Query::default(),
            previous_result_keys: DashSet::new(),
            fields: None,
        });

        // Init a Merkle tree
        merkle_mgr.init_tree("q-sync-2", "users", 0, &[("k1".to_string(), 42)]);
        let server_hash = merkle_mgr.aggregate_query_root_hash("q-sync-2", "users");

        // Send QUERY_SYNC_INIT with a different (stale) hash.
        let stale_hash = server_hash.wrapping_add(1);
        let ctx = make_ctx(Some(ConnectionId(1)));
        let payload = QuerySyncInitMessage {
            payload: QuerySyncInitPayload {
                query_id: "q-sync-2".to_string(),
                root_hash: stale_hash,
            },
        };
        let op = Operation::QuerySyncInit { ctx, payload };
        let result = svc.oneshot(op).await.unwrap();

        // Should get SyncRespRoot with the server's actual hash (different from client's).
        match result {
            OperationResponse::Message(msg) => {
                if let Message::SyncRespRoot(resp) = *msg {
                    assert_eq!(resp.payload.root_hash, server_hash);
                    assert_ne!(resp.payload.root_hash, stale_hash);
                } else {
                    panic!("Expected SyncRespRoot, got {:?}", msg);
                }
            }
            other => panic!("Expected Message response, got {:?}", other),
        }
    }

    /// Verify `QueryRegistry::get_subscription` lookup by query_id.
    #[test]
    fn registry_get_subscription_by_query_id() {
        let registry = QueryRegistry::new();
        registry.register(QuerySubscription {
            query_id: "q-lookup".to_string(),
            connection_id: ConnectionId(1),
            map_name: "users".to_string(),
            live_window: seeded_window(&Query::default(), &[]),
            query: Query::default(),
            previous_result_keys: DashSet::new(),
            fields: Some(vec!["name".to_string()]),
        });

        let sub = registry.get_subscription("q-lookup");
        assert!(sub.is_some());
        let sub = sub.unwrap();
        assert_eq!(sub.query_id, "q-lookup");
        assert_eq!(sub.map_name, "users");
        assert_eq!(sub.fields, Some(vec!["name".to_string()]));

        // Non-existent query returns None
        assert!(registry.get_subscription("nonexistent").is_none());
    }

    /// Verifies `map_dag_rows_to_entries`: the entry key prefers the real `_key`
    /// injected by `ScanProcessor`, falls back to the group `__key`, then to a
    /// synthetic `row-{i}`; internal `_key` is stripped from the value, and a
    /// `_value`-wrapped scalar is unwrapped back to its scalar value.
    #[test]
    fn map_dag_rows_to_entries_recovers_keys_and_strips_internal_fields() {
        // 1. Non-group-by Map row with real `_key` → key from `_key`, `_key` stripped.
        let row_with_key = rmpv::Value::Map(vec![
            (
                rmpv::Value::String("name".into()),
                rmpv::Value::String("Alice".into()),
            ),
            (
                rmpv::Value::String("_key".into()),
                rmpv::Value::String("rec-1".into()),
            ),
        ]);
        // 2. Group-by aggregate row with `__key` (no `_key`) → key from `__key`,
        //    `__key` retained in the value.
        let row_group = rmpv::Value::Map(vec![
            (
                rmpv::Value::String("__key".into()),
                rmpv::Value::String("active".into()),
            ),
            (
                rmpv::Value::String("__count".into()),
                rmpv::Value::Integer(42.into()),
            ),
        ]);
        // 3. Scalar record wrapped by ScanProcessor as {_key, _value} → key from
        //    `_key`, value unwrapped to the scalar.
        let row_scalar = rmpv::Value::Map(vec![
            (
                rmpv::Value::String("_key".into()),
                rmpv::Value::String("rec-3".into()),
            ),
            (
                rmpv::Value::String("_value".into()),
                rmpv::Value::Integer(99.into()),
            ),
        ]);
        // 4. Row with neither key → synthetic `row-{i}`.
        let row_no_key = rmpv::Value::Map(vec![(
            rmpv::Value::String("x".into()),
            rmpv::Value::Integer(1.into()),
        )]);

        let entries =
            map_dag_rows_to_entries(vec![row_with_key, row_group, row_scalar, row_no_key]);

        assert_eq!(entries.len(), 4);

        // (1) real key recovered; value has only `name` (no `_key`).
        assert_eq!(entries[0].key, "rec-1");
        assert_eq!(
            entries[0].value,
            rmpv::Value::Map(vec![(
                rmpv::Value::String("name".into()),
                rmpv::Value::String("Alice".into()),
            )])
        );

        // (2) group key recovered; `__key` retained for aggregate rows.
        assert_eq!(entries[1].key, "active");
        assert_eq!(entries[1].value, row_group_expected());

        // (3) scalar unwrapped.
        assert_eq!(entries[2].key, "rec-3");
        assert_eq!(entries[2].value, rmpv::Value::Integer(99.into()));

        // (4) synthetic fallback at index 3.
        assert_eq!(entries[3].key, "row-3");
    }

    fn row_group_expected() -> rmpv::Value {
        rmpv::Value::Map(vec![
            (
                rmpv::Value::String("__key".into()),
                rmpv::Value::String("active".into()),
            ),
            (
                rmpv::Value::String("__count".into()),
                rmpv::Value::Integer(42.into()),
            ),
        ])
    }

    // ---------------------------------------------------------------------------
    // handle_vector_search tests
    // ---------------------------------------------------------------------------

    use crate::service::domain::index::{Index, IndexObserverFactory};
    use topgun_core::messages::vector::VectorSearchPayload;
    use topgun_core::vector::{DistanceMetric, Vector};

    /// Builds an rmpv::Value record with the attribute field containing the encoded vector.
    fn make_vector_record(attr: &str, data: &[f32]) -> rmpv::Value {
        let v = Vector::F32(data.to_vec());
        let encoded = rmp_serde::to_vec_named(&v).unwrap();
        rmpv::Value::Map(vec![(
            rmpv::Value::String(rmpv::Utf8String::from(attr)),
            rmpv::Value::Binary(encoded),
        )])
    }

    fn make_query_vec(floats: &[f32]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(floats.len() * 4);
        for &f in floats {
            bytes.extend_from_slice(&f.to_le_bytes());
        }
        bytes
    }

    fn make_svc_with_vector_index(
        map_name: &str,
        attr: &str,
        dim: u16,
        metric: DistanceMetric,
    ) -> (
        Arc<QueryService>,
        Arc<crate::service::domain::index::registry::IndexRegistry>,
    ) {
        let registry = Arc::new(QueryRegistry::new());
        let factory = make_factory();
        let conn_registry = Arc::new(ConnectionRegistry::new());
        let observer_factory = Arc::new(IndexObserverFactory::new());
        let index_registry = observer_factory.register_map(map_name);
        index_registry.add_vector_index(attr, dim, metric);

        let svc = Arc::new(QueryService::new(
            registry,
            factory,
            conn_registry,
            None,
            10_000,
            Some(observer_factory),
            #[cfg(feature = "datafusion")]
            None,
        ));
        (svc, index_registry)
    }

    fn extract_vector_resp(
        result: OperationResponse,
    ) -> topgun_core::messages::vector::VectorSearchRespPayload {
        match result {
            OperationResponse::Message(msg) => match *msg {
                Message::VectorSearchResp { payload } => payload,
                other => panic!("expected VectorSearchResp, got {:?}", other),
            },
            other => panic!("expected Message response, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn handle_vector_search_returns_top_k() {
        let (svc, index_registry) =
            make_svc_with_vector_index("maps", "embedding", 2, DistanceMetric::Cosine);
        let vi = index_registry.get_vector_index("embedding").unwrap();

        // Insert 10 vectors: [0.1*i, 0.2*i] for i in 1..=10
        for i in 1u8..=10 {
            let fi = f32::from(i);
            let rec = make_vector_record("embedding", &[0.1 * fi, 0.2 * fi]);
            vi.insert(&format!("k{i}"), &rec);
        }
        vi.commit_pending();

        let ctx = make_ctx(None);
        let payload = VectorSearchPayload {
            id: "vs-topk".to_string(),
            map_name: "maps".to_string(),
            index_name: Some("embedding".to_string()),
            query_vector: make_query_vec(&[0.9f32, 1.8f32]),
            k: 3,
            ef_search: None,
            options: None,
        };
        let op = Operation::VectorSearch { ctx, payload };
        let resp = extract_vector_resp(svc.oneshot(op).await.unwrap());
        assert_eq!(resp.results.len(), 3);
        assert!(resp.error.is_none());
        // Verify monotonic descending order
        for i in 0..resp.results.len() - 1 {
            assert!(
                resp.results[i].score >= resp.results[i + 1].score,
                "results not sorted descending: {:?}",
                resp.results.iter().map(|r| r.score).collect::<Vec<_>>()
            );
        }
    }

    #[tokio::test]
    async fn handle_vector_search_unknown_map_returns_error_resp() {
        let (svc, _) = make_svc_with_vector_index("maps", "embedding", 2, DistanceMetric::Cosine);
        let ctx = make_ctx(None);
        let payload = VectorSearchPayload {
            id: "vs-unknown".to_string(),
            map_name: "nonexistent_map".to_string(),
            index_name: None,
            query_vector: make_query_vec(&[1.0f32, 0.0f32]),
            k: 3,
            ef_search: None,
            options: None,
        };
        let op = Operation::VectorSearch { ctx, payload };
        let resp = extract_vector_resp(svc.oneshot(op).await.unwrap());
        assert!(resp.error.is_some(), "expected error for unknown map");
        assert!(resp.results.is_empty());
    }

    #[tokio::test]
    async fn handle_vector_search_missing_index_returns_error_resp() {
        let (svc, index_registry) =
            make_svc_with_vector_index("maps", "embedding", 2, DistanceMetric::Cosine);
        // Remove the vector index so the map has none
        let _ = index_registry.remove_index("embedding");

        let ctx = make_ctx(None);
        let payload = VectorSearchPayload {
            id: "vs-no-idx".to_string(),
            map_name: "maps".to_string(),
            index_name: None,
            query_vector: make_query_vec(&[1.0f32, 0.0f32]),
            k: 3,
            ef_search: None,
            options: None,
        };
        let op = Operation::VectorSearch { ctx, payload };
        let resp = extract_vector_resp(svc.oneshot(op).await.unwrap());
        assert!(resp.error.is_some(), "expected error for missing index");
        assert!(resp.results.is_empty());
    }

    #[tokio::test]
    async fn handle_vector_search_dimension_mismatch_returns_error_resp() {
        let (svc, index_registry) =
            make_svc_with_vector_index("maps", "embedding", 4, DistanceMetric::Cosine);
        let vi = index_registry.get_vector_index("embedding").unwrap();
        let rec = make_vector_record("embedding", &[0.1, 0.2, 0.3, 0.4]);
        vi.insert("k1", &rec);
        vi.commit_pending();

        let ctx = make_ctx(None);
        let payload = VectorSearchPayload {
            id: "vs-dim-mismatch".to_string(),
            map_name: "maps".to_string(),
            index_name: Some("embedding".to_string()),
            // 2 floats instead of 4 (dimension = 4)
            query_vector: make_query_vec(&[0.1f32, 0.2f32]),
            k: 1,
            ef_search: None,
            options: None,
        };
        let op = Operation::VectorSearch { ctx, payload };
        let resp = extract_vector_resp(svc.oneshot(op).await.unwrap());
        assert!(
            resp.error.is_some(),
            "expected error for dimension mismatch"
        );
        assert!(resp.results.is_empty());
    }

    #[tokio::test]
    async fn handle_vector_search_honors_min_score() {
        let (svc, index_registry) =
            make_svc_with_vector_index("maps", "embedding", 2, DistanceMetric::Euclidean);
        let vi = index_registry.get_vector_index("embedding").unwrap();

        // Insert vectors at varying distances from query [1.0, 0.0]
        for i in 1u8..=5 {
            let x = f32::from(i);
            let rec = make_vector_record("embedding", &[x, 0.0]);
            vi.insert(&format!("k{i}"), &rec);
        }
        vi.commit_pending();

        let ctx = make_ctx(None);
        let payload = VectorSearchPayload {
            id: "vs-minscore".to_string(),
            map_name: "maps".to_string(),
            index_name: Some("embedding".to_string()),
            query_vector: make_query_vec(&[1.0f32, 0.0f32]),
            k: 10,
            ef_search: None,
            options: Some(topgun_core::messages::vector::VectorSearchOptions {
                min_score: Some(0.5),
                ..Default::default()
            }),
        };
        let op = Operation::VectorSearch { ctx, payload };
        let resp = extract_vector_resp(svc.oneshot(op).await.unwrap());
        // All returned results must have score >= 0.5
        for r in &resp.results {
            assert!(
                r.score >= 0.5,
                "result score {} below threshold 0.5",
                r.score
            );
        }
    }

    #[tokio::test]
    async fn handle_vector_search_include_value_default_true() {
        use crate::storage::record_store::{CallerProvenance, ExpiryPolicy};

        let (svc, index_registry) =
            make_svc_with_vector_index("maps", "embedding", 2, DistanceMetric::Cosine);
        let vi = index_registry.get_vector_index("embedding").unwrap();
        let rec = make_vector_record("embedding", &[1.0, 0.0]);
        vi.insert("k1", &rec);
        vi.commit_pending();

        // Populate RecordStore so include_value (default true) can return a value.
        let store = svc.record_store_factory.get_or_create("maps", 0);
        store
            .put(
                "k1",
                RecordValue::Lww {
                    value: make_value_map(vec![("name", Value::String("Alice".to_string()))]),
                    timestamp: make_timestamp(),
                },
                ExpiryPolicy::NONE,
                CallerProvenance::Client,
            )
            .await
            .unwrap();

        let ctx = make_ctx(None);
        let payload = VectorSearchPayload {
            id: "vs-iv-default".to_string(),
            map_name: "maps".to_string(),
            index_name: Some("embedding".to_string()),
            query_vector: make_query_vec(&[1.0f32, 0.0f32]),
            k: 1,
            ef_search: None,
            options: None, // defaults: include_value = true
        };
        let op = Operation::VectorSearch { ctx, payload };
        let resp = extract_vector_resp(svc.oneshot(op).await.unwrap());
        assert_eq!(resp.results.len(), 1);
        assert!(resp.error.is_none());
        assert!(
            resp.results[0].value.is_some(),
            "expected value to be populated when include_value defaults to true"
        );
    }

    #[tokio::test]
    async fn handle_vector_search_include_value_false() {
        let (svc, index_registry) =
            make_svc_with_vector_index("maps", "embedding", 2, DistanceMetric::Cosine);
        let vi = index_registry.get_vector_index("embedding").unwrap();
        let rec = make_vector_record("embedding", &[1.0, 0.0]);
        vi.insert("k1", &rec);
        vi.commit_pending();

        let ctx = make_ctx(None);
        let payload = VectorSearchPayload {
            id: "vs-iv-false".to_string(),
            map_name: "maps".to_string(),
            index_name: Some("embedding".to_string()),
            query_vector: make_query_vec(&[1.0f32, 0.0f32]),
            k: 1,
            ef_search: None,
            options: Some(topgun_core::messages::vector::VectorSearchOptions {
                include_value: Some(false),
                ..Default::default()
            }),
        };
        let op = Operation::VectorSearch { ctx, payload };
        let resp = extract_vector_resp(svc.oneshot(op).await.unwrap());
        assert_eq!(resp.results.len(), 1);
        assert!(
            resp.results[0].value.is_none(),
            "value should be None when include_value = false"
        );
    }

    #[tokio::test]
    async fn handle_vector_search_ef_search_default() {
        let (svc, index_registry) =
            make_svc_with_vector_index("maps", "embedding", 2, DistanceMetric::Cosine);
        let vi = index_registry.get_vector_index("embedding").unwrap();
        for i in 1u8..=5 {
            let rec = make_vector_record("embedding", &[f32::from(i), 0.0]);
            vi.insert(&format!("k{i}"), &rec);
        }
        vi.commit_pending();

        let ctx = make_ctx(None);
        let payload = VectorSearchPayload {
            id: "vs-ef-default".to_string(),
            map_name: "maps".to_string(),
            index_name: Some("embedding".to_string()),
            query_vector: make_query_vec(&[1.0f32, 0.0f32]),
            k: 3,
            ef_search: None, // server uses k * 2 = 6
            options: None,
        };
        let op = Operation::VectorSearch { ctx, payload };
        let resp = extract_vector_resp(svc.oneshot(op).await.unwrap());
        // Should return top 3 results without error (behavioural check)
        assert_eq!(resp.results.len(), 3, "expected 3 results");
        assert!(resp.error.is_none());
    }

    #[tokio::test]
    async fn handle_vector_search_dot_product_score_convention() {
        // DotProductDistance::compute returns -dot(a, b), so score = -distance = dot(a, b).
        // Insert [1.0, 0.0] with attribute "embedding" into a DotProduct index.
        let (svc, index_registry) =
            make_svc_with_vector_index("maps", "embedding", 2, DistanceMetric::DotProduct);
        let vi = index_registry.get_vector_index("embedding").unwrap();

        let a = [1.0f32, 0.0f32];
        let rec = make_vector_record("embedding", &a);
        vi.insert("k1", &rec);
        vi.commit_pending();

        // Query with q = [1.0, 0.0]; expected dot = 1.0*1.0 + 0.0*0.0 = 1.0
        let ctx = make_ctx(None);
        let payload = VectorSearchPayload {
            id: "vs-dot".to_string(),
            map_name: "maps".to_string(),
            index_name: Some("embedding".to_string()),
            query_vector: make_query_vec(&[1.0f32, 0.0f32]),
            k: 1,
            ef_search: Some(2),
            options: None,
        };
        let op = Operation::VectorSearch { ctx, payload };
        let resp = extract_vector_resp(svc.oneshot(op).await.unwrap());
        assert_eq!(resp.results.len(), 1);
        // score = -distance = -(-dot) = dot = 1.0
        let score = resp.results[0].score;
        assert!(
            (score - 1.0).abs() < 1e-5,
            "expected dot product score ~1.0, got {score}"
        );
    }

    #[tokio::test]
    async fn handle_vector_search_include_vectors_populated() {
        use crate::storage::record_store::{CallerProvenance, ExpiryPolicy};
        use std::collections::BTreeMap;
        use topgun_core::{vector::Vector, Value};

        let (svc, index_registry) =
            make_svc_with_vector_index("maps", "embedding", 2, DistanceMetric::Cosine);
        let vi = index_registry.get_vector_index("embedding").unwrap();
        // make_vector_record returns rmpv::Value for HNSW insertion
        let rec_rmpv = make_vector_record("embedding", &[1.0, 0.0]);
        vi.insert("k1", &rec_rmpv);
        vi.commit_pending();

        // RecordValue::Lww requires topgun_core::Value, not rmpv::Value.
        // Build a Value::Map containing the embedding as a MsgPack-encoded binary blob,
        // matching the wire contract.
        let embedding_bytes = rmp_serde::to_vec_named(&Vector::F32(vec![1.0, 0.0])).unwrap();
        let stored_value = Value::Map(BTreeMap::from([(
            "embedding".to_string(),
            Value::Bytes(embedding_bytes),
        )]));

        // Populate the record store so the handler can reach the attribute binary
        // for include_vectors via the value_map pass.
        let store = svc.record_store_factory.get_or_create("maps", 0);
        store
            .put(
                "k1",
                RecordValue::Lww {
                    value: stored_value,
                    timestamp: make_timestamp(),
                },
                ExpiryPolicy::NONE,
                CallerProvenance::Client,
            )
            .await
            .unwrap();

        let ctx = make_ctx(None);
        let payload = VectorSearchPayload {
            id: "vs-incvec".to_string(),
            map_name: "maps".to_string(),
            index_name: Some("embedding".to_string()),
            query_vector: make_query_vec(&[1.0f32, 0.0f32]),
            k: 1,
            ef_search: None,
            options: Some(topgun_core::messages::vector::VectorSearchOptions {
                include_vectors: Some(true),
                ..Default::default()
            }),
        };
        let op = Operation::VectorSearch { ctx, payload };
        let resp = extract_vector_resp(svc.oneshot(op).await.unwrap());

        assert_eq!(resp.results.len(), 1);
        assert!(resp.error.is_none());
        assert!(
            resp.results[0].vector.is_some(),
            "expected vector to be populated when include_vectors = true"
        );
        let bytes = resp.results[0].vector.as_ref().unwrap();
        assert_eq!(bytes.len(), 2 * 4, "expected dimension * 4 bytes");
        // Round-trip check: bytes decode back to [1.0, 0.0] f32
        let mut floats = Vec::with_capacity(2);
        for chunk in bytes.chunks_exact(4) {
            floats.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
        }
        assert_eq!(floats, vec![1.0f32, 0.0f32]);
    }

    #[tokio::test]
    async fn handle_vector_search_include_vectors_default_none() {
        let (svc, index_registry) =
            make_svc_with_vector_index("maps", "embedding", 2, DistanceMetric::Cosine);
        let vi = index_registry.get_vector_index("embedding").unwrap();
        let rec_rmpv = make_vector_record("embedding", &[1.0, 0.0]);
        vi.insert("k1", &rec_rmpv);
        vi.commit_pending();

        let ctx = make_ctx(None);
        let payload = VectorSearchPayload {
            id: "vs-defnone".to_string(),
            map_name: "maps".to_string(),
            index_name: Some("embedding".to_string()),
            query_vector: make_query_vec(&[1.0f32, 0.0f32]),
            k: 1,
            ef_search: None,
            options: None,
        };
        let op = Operation::VectorSearch { ctx, payload };
        let resp = extract_vector_resp(svc.oneshot(op).await.unwrap());

        assert_eq!(resp.results.len(), 1);
        assert!(
            resp.results[0].vector.is_none(),
            "expected vector to be None when options is None"
        );
    }
}
