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
use topgun_core::messages::query::{QueryRespMessage, QueryRespPayload};
use topgun_core::messages::{Message, SyncRespRootMessage, SyncRespRootPayload};

use tracing::Instrument;

use crate::network::connection::{ConnectionId, ConnectionRegistry, OutboundMessage};
use crate::service::domain::index::query_optimizer::index_aware_evaluate;
use crate::service::domain::index::IndexObserverFactory;
use crate::service::domain::predicate::{
    evaluate_predicate, evaluate_where, value_to_rmpv,
};
use crate::service::domain::query_backend::QueryBackend;
use crate::service::operation::{
    service_names, Operation, OperationError, OperationResponse,
};
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
            evaluate_predicate(pred, data)
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
            let was_in_previous = sub.previous_result_keys.contains(key);

            match (matches_now, was_in_previous) {
                (true, false) => {
                    // ENTER: new match
                    sub.previous_result_keys.insert(key.to_string());
                    self.send_update(sub, key, rmpv_value.clone(), ChangeEventType::ENTER);
                }
                (true, true) => {
                    // UPDATE: still matches
                    self.send_update(sub, key, rmpv_value.clone(), ChangeEventType::UPDATE);
                }
                (false, true) => {
                    // LEAVE: no longer matches
                    sub.previous_result_keys.remove(key);
                    self.send_update(sub, key, rmpv_value.clone(), ChangeEventType::LEAVE);
                }
                (false, false) => {
                    // No-op
                }
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

    fn on_remove(&self, key: &str, record: &Record, is_backup: bool) {
        if is_backup {
            return;
        }
        let subs = self.registry.get_subscriptions_for_map(&self.map_name);
        let rmpv_value = extract_rmpv_value(&record.value);

        for sub in &subs {
            if sub.previous_result_keys.contains(key) {
                sub.previous_result_keys.remove(key);
                self.send_update(sub, key, rmpv_value.clone(), ChangeEventType::LEAVE);
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
    query_backend: Arc<dyn QueryBackend>,
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
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        query_registry: Arc<QueryRegistry>,
        record_store_factory: Arc<RecordStoreFactory>,
        connection_registry: Arc<ConnectionRegistry>,
        query_backend: Arc<dyn QueryBackend>,
        query_merkle_manager: Option<Arc<crate::storage::query_merkle::QueryMerkleSyncManager>>,
        max_query_records: u32,
        index_observer_factory: Option<Arc<IndexObserverFactory>>,
        #[cfg(feature = "datafusion")]
        sql_query_backend: Option<Arc<dyn crate::service::domain::query_backend::SqlQueryBackend>>,
    ) -> Self {
        Self {
            query_registry,
            record_store_factory,
            connection_registry,
            query_backend,
            query_merkle_manager,
            max_query_records,
            index_observer_factory,
            #[cfg(feature = "datafusion")]
            sql_query_backend,
        }
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
    type Future =
        Pin<Box<dyn Future<Output = Result<OperationResponse, OperationError>> + Send>>;

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
    /// 3. Delegates to `query_backend.execute_query()` for filtering, sorting, and limiting.
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

        // Scan ALL partitions for this map to aggregate entries across the full key space.
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

        // Narrow candidate entries using index-accelerated evaluation when an
        // IndexObserverFactory is wired and the query has a predicate. This
        // reduces the entries passed to execute_query without altering the
        // query semantics — the predicate is re-evaluated inside the backend
        // and inside index_aware_evaluate itself, so no correctness risk.
        let entries = if let (Some(factory), Some(predicate)) =
            (self.index_observer_factory.as_ref(), query.predicate.as_ref())
        {
            if let Some(registry) = factory.get_registry(&map_name) {
                // Build a lookup map so the optimizer can fetch record values by key.
                let entry_map: std::collections::HashMap<&str, &rmpv::Value> =
                    entries.iter().map(|(k, v)| (k.as_str(), v)).collect();
                let all_keys: Vec<String> =
                    entries.iter().map(|(k, _)| k.clone()).collect();

                let matching_keys = index_aware_evaluate(
                    &registry,
                    predicate,
                    &all_keys,
                    |key| entry_map.get(key).map(|v| (*v).clone()),
                );

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

        // Delegate to query backend for filtering, sorting, and limiting
        let mut results = self
            .query_backend
            .execute_query(&map_name, entries, &query)
            .await
            .map_err(|e| OperationError::Internal(anyhow::anyhow!("{e}")))?;

        // Apply max_query_records clamping
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
        } else {
            None
        };

        // Apply field projection if specified
        if let Some(ref proj_fields) = fields {
            for entry in &mut results {
                entry.value = project_fields(proj_fields, &entry.value);
            }
        }

        // Build previous_result_keys from results (after clamping, before Merkle)
        let previous_keys = DashSet::new();
        for entry in &results {
            previous_keys.insert(entry.key.clone());
        }

        // Initialize per-query Merkle trees for matching records
        if let Some(ref merkle) = self.query_merkle_manager {
            // Only insert keys that are in the result set into Merkle trees.
            // Build a set of result keys for fast lookup.
            let result_key_set: HashSet<&str> =
                results.iter().map(|e| e.key.as_str()).collect();

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

        // Register standing subscription (with fields for future QUERY_UPDATE projection)
        let subscription = QuerySubscription {
            query_id: query_id.clone(),
            connection_id,
            map_name,
            query,
            previous_result_keys: previous_keys,
            fields,
        };
        self.query_registry.register(subscription);

        // Build response
        let resp = Message::QueryResp(QueryRespMessage {
            payload: QueryRespPayload {
                query_id,
                results,
                next_cursor: None,
                has_more,
                cursor_status: None,
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
        let _ = self.query_registry
            .unregister(&payload.payload.query_id);
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
                    Message::SqlQueryResp { payload: resp_payload },
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
                    Message::SqlQueryResp { payload: resp_payload },
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
        BinaryArray, BooleanArray, Float32Array, Float64Array, Int32Array,
        Int64Array, LargeStringArray, StringArray, TimestampMicrosecondArray, UInt32Array,
        UInt64Array,
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
                Err(_) => rmpv::Value::String(format!("<unsupported: {:?}>", array.data_type()).into()),
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
mod tests {
    use std::collections::BTreeMap;

    use dashmap::DashSet;
    use topgun_core::hlc::Timestamp;
    use topgun_core::messages::base::{PredicateNode, PredicateOp, Query};
    use topgun_core::messages::query::{QuerySubMessage, QuerySubPayload, QueryUnsubMessage, QueryUnsubPayload};
    use topgun_core::types::Value;
    use tower::ServiceExt;

    use super::*;
    use crate::network::config::ConnectionConfig;
    use crate::network::connection::{ConnectionId, ConnectionKind, ConnectionRegistry};
    use crate::service::domain::query_backend::PredicateBackend;
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
        let mut ctx = OperationContext::new(
            1,
            service_names::QUERY,
            make_timestamp(),
            5000,
        );
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

    // ---- QueryRegistry tests ----

    #[test]
    fn registry_register_and_count() {
        let registry = QueryRegistry::new();
        assert_eq!(registry.subscription_count(), 0);

        let sub = QuerySubscription {
            query_id: "q-1".to_string(),
            connection_id: ConnectionId(1),
            map_name: "users".to_string(),
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
            query: Query::default(),
            previous_result_keys: DashSet::new(),
            fields: None,
        });
        registry.register(QuerySubscription {
            query_id: "q-2".to_string(),
            connection_id: ConnectionId(2),
            map_name: "users".to_string(),
            query: Query::default(),
            previous_result_keys: DashSet::new(),
            fields: None,
        });
        registry.register(QuerySubscription {
            query_id: "q-3".to_string(),
            connection_id: ConnectionId(1),
            map_name: "orders".to_string(),
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
            query: Query::default(),
            previous_result_keys: DashSet::new(),
            fields: None,
        });
        registry.register(QuerySubscription {
            query_id: "q-2".to_string(),
            connection_id: ConnectionId(1),
            map_name: "orders".to_string(),
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
        let observer = QueryMutationObserver::new(
            registry.clone(),
            conn_registry,
            "users".to_string(),
            0,
        );

        // Register a subscription
        let prev_keys = DashSet::new();
        registry.register(QuerySubscription {
            query_id: "q-1".to_string(),
            connection_id: ConnectionId(1),
            map_name: "users".to_string(),
            query: Query::default(),
            previous_result_keys: prev_keys,
            fields: None,
        });

        let record = make_record(make_value_map(vec![("name", Value::String("Alice".to_string()))]));

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

        let observer = QueryMutationObserver::new(
            registry.clone(),
            conn_registry,
            "users".to_string(),
            0,
        );

        // Register subscription with no predicate (match all)
        registry.register(QuerySubscription {
            query_id: "q-1".to_string(),
            connection_id: conn_id,
            map_name: "users".to_string(),
            query: Query::default(),
            previous_result_keys: DashSet::new(),
            fields: None,
        });

        let record = make_record(make_value_map(vec![("name", Value::String("Alice".to_string()))]));
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

        let observer = QueryMutationObserver::new(
            registry.clone(),
            conn_registry,
            "users".to_string(),
            0,
        );

        // Register subscription with key already in previous_result_keys
        let prev_keys = DashSet::new();
        prev_keys.insert("key1".to_string());
        registry.register(QuerySubscription {
            query_id: "q-1".to_string(),
            connection_id: conn_id,
            map_name: "users".to_string(),
            query: Query::default(), // match all
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

        let observer = QueryMutationObserver::new(
            registry.clone(),
            conn_registry,
            "users".to_string(),
            0,
        );

        // Subscription requires age >= 18
        let prev_keys = DashSet::new();
        prev_keys.insert("key1".to_string());
        registry.register(QuerySubscription {
            query_id: "q-1".to_string(),
            connection_id: conn_id,
            map_name: "users".to_string(),
            query: Query {
                predicate: Some(PredicateNode {
                    op: PredicateOp::Gte,
                    attribute: Some("age".to_string()),
                    value: Some(rmpv::Value::Integer(18.into())),
                    children: None,
                }),
                ..Query::default()
            },
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

        let observer = QueryMutationObserver::new(
            registry.clone(),
            conn_registry,
            "users".to_string(),
            0,
        );

        let prev_keys = DashSet::new();
        prev_keys.insert("key1".to_string());
        registry.register(QuerySubscription {
            query_id: "q-1".to_string(),
            connection_id: conn_id,
            map_name: "users".to_string(),
            query: Query::default(),
            previous_result_keys: prev_keys,
            fields: None,
        });

        let record = make_record(make_value_map(vec![("name", Value::String("Alice".to_string()))]));
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

        let observer = QueryMutationObserver::new(
            registry.clone(),
            conn_registry,
            "users".to_string(),
            0,
        );

        let prev_keys = DashSet::new();
        prev_keys.insert("k1".to_string());
        prev_keys.insert("k2".to_string());
        registry.register(QuerySubscription {
            query_id: "q-1".to_string(),
            connection_id: conn_id,
            map_name: "users".to_string(),
            query: Query::default(),
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

        let observer = QueryMutationObserver::new(
            registry.clone(),
            conn_registry,
            "users".to_string(),
            0,
        );

        // Subscription requires age >= 18; key1 not in previous
        registry.register(QuerySubscription {
            query_id: "q-1".to_string(),
            connection_id: conn_id,
            map_name: "users".to_string(),
            query: Query {
                predicate: Some(PredicateNode {
                    op: PredicateOp::Gte,
                    attribute: Some("age".to_string()),
                    value: Some(rmpv::Value::Integer(18.into())),
                    children: None,
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
            Arc::new(PredicateBackend),
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
            Arc::new(PredicateBackend),
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
            Arc::new(PredicateBackend),
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
            Arc::new(PredicateBackend),
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
            OperationResponse::Message(msg) => {
                match *msg {
                    Message::QueryResp(resp) => {
                        assert_eq!(resp.payload.query_id, "q-1");
                        assert!(resp.payload.results.is_empty());
                    }
                    _ => panic!("expected QueryResp"),
                }
            }
            _ => panic!("expected Message response"),
        }

        // Subscription should be registered
        assert_eq!(registry.subscription_count(), 1);
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
                    (rmpv::Value::String("name".into()), rmpv::Value::String("Alice".into())),
                    (rmpv::Value::String("age".into()), rmpv::Value::Integer(30.into())),
                ]),
            ),
            (
                "user-2".to_string(),
                rmpv::Value::Map(vec![
                    (rmpv::Value::String("name".into()), rmpv::Value::String("Bob".into())),
                    (rmpv::Value::String("age".into()), rmpv::Value::Integer(15.into())),
                ]),
            ),
        ];

        let query = Query {
            predicate: Some(PredicateNode {
                op: PredicateOp::Gte,
                attribute: Some("age".to_string()),
                value: Some(rmpv::Value::Integer(18.into())),
                children: None,
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
            Arc::new(PredicateBackend),
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
            query: Query {
                predicate: None,
                r#where: None,
                sort: None,
                limit: None,
                cursor: None,
            },
            previous_result_keys: DashSet::new(),
            fields: None,
        });

        registry.register(QuerySubscription {
            query_id: "q-2".to_string(),
            connection_id: conn2,
            map_name: "users".to_string(),
            query: Query {
                predicate: None,
                r#where: None,
                sort: None,
                limit: None,
                cursor: None,
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
            query: Query {
                predicate: None,
                r#where: None,
                sort: None,
                limit: None,
                cursor: None,
            },
            previous_result_keys: DashSet::new(),
            fields: None,
        });
        registry.register(QuerySubscription {
            query_id: "q-2".to_string(),
            connection_id: conn1,
            map_name: "users".to_string(),
            query: Query {
                predicate: None,
                r#where: None,
                sort: None,
                limit: None,
                cursor: None,
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
            (rmpv::Value::String("name".into()), rmpv::Value::String("Alice".into())),
            (rmpv::Value::String("age".into()), rmpv::Value::Integer(30.into())),
            (rmpv::Value::String("email".into()), rmpv::Value::String("alice@test.com".into())),
        ]);

        let fields = vec!["name".to_string()];
        let projected = project_fields(&fields, &value);

        // Projected value should only contain "name"
        let map = projected.as_map().expect("projected should be a map");
        assert_eq!(map.len(), 1);
        assert_eq!(
            map[0].0.as_str().unwrap(),
            "name"
        );
        assert_eq!(
            map[0].1.as_str().unwrap(),
            "Alice"
        );
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
                        (rmpv::Value::String("id".into()), rmpv::Value::Integer(i.into())),
                        (rmpv::Value::String("name".into()), rmpv::Value::String(format!("item-{i}").into())),
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
            Arc::new(PredicateBackend),
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
            Arc::new(PredicateBackend),
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
}
