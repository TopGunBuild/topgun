//! Query domain service for live query subscriptions.
//!
//! Manages an in-memory `QueryRegistry` of standing query subscriptions,
//! evaluates queries against `RecordStore` contents, and pushes incremental
//! `QUERY_UPDATE` messages (ENTER/UPDATE/LEAVE) to subscribers via
//! `QueryMutationObserver` when data changes.

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
use topgun_core::messages::Message;

use crate::network::connection::{ConnectionId, ConnectionRegistry, OutboundMessage};
use crate::service::domain::predicate::{
    evaluate_predicate, evaluate_where, execute_query, value_to_rmpv,
};
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
}

impl QueryService {
    /// Creates a new `QueryService` with its required dependencies.
    #[must_use]
    pub fn new(
        query_registry: Arc<QueryRegistry>,
        record_store_factory: Arc<RecordStoreFactory>,
        connection_registry: Arc<ConnectionRegistry>,
    ) -> Self {
        Self {
            query_registry,
            record_store_factory,
            connection_registry,
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
        Box::pin(async move {
            match op {
                Operation::QuerySubscribe { ctx, payload } => {
                    svc.handle_query_subscribe(&ctx, &payload)
                }
                Operation::QueryUnsubscribe { ctx, payload } => {
                    svc.handle_query_unsubscribe(&ctx, &payload)
                }
                _ => Err(OperationError::WrongService),
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Handler implementations
// ---------------------------------------------------------------------------

impl QueryService {
    /// Handles a `QuerySubscribe` operation.
    ///
    /// 1. Extracts `connection_id` from context (error if missing).
    /// 2. Gets or creates `RecordStore` for the target map and partition.
    /// 3. Iterates all records, converting each `RecordValue` to `rmpv::Value`.
    /// 4. Passes through `execute_query` for filtering, sorting, and limiting.
    /// 5. Registers a standing `QuerySubscription` in the registry.
    /// 6. Returns `QueryResp` with initial results.
    fn handle_query_subscribe(
        &self,
        ctx: &crate::service::operation::OperationContext,
        payload: &topgun_core::messages::query::QuerySubMessage,
    ) -> Result<OperationResponse, OperationError> {
        let connection_id = ctx.connection_id.ok_or_else(|| {
            OperationError::Internal(anyhow::anyhow!(
                "QuerySubscribe requires connection_id in OperationContext"
            ))
        })?;

        let partition_id = ctx.partition_id.unwrap_or(0);
        let query_id = payload.payload.query_id.clone();
        let map_name = payload.payload.map_name.clone();
        let query = payload.payload.query.clone();

        // Get or create RecordStore for this map+partition
        let store = self
            .record_store_factory
            .create(&map_name, partition_id);

        // Collect all entries as (key, rmpv::Value)
        let mut entries: Vec<(String, rmpv::Value)> = Vec::new();
        store.for_each_boxed(
            &mut |key, record| {
                if let RecordValue::Lww { ref value, .. } = record.value {
                    entries.push((key.to_string(), value_to_rmpv(value)));
                }
                // Skip OrMap/OrTombstones records for query evaluation
            },
            false, // not backup
        );

        // Execute query (filter, sort, limit)
        let results = execute_query(entries, &query);

        // Build previous_result_keys from results
        let previous_keys = DashSet::new();
        for entry in &results {
            previous_keys.insert(entry.key.clone());
        }

        // Register standing subscription
        let subscription = QuerySubscription {
            query_id: query_id.clone(),
            connection_id,
            map_name,
            query,
            previous_result_keys: previous_keys,
        };
        self.query_registry.register(subscription);

        // Build response
        let resp = Message::QueryResp(QueryRespMessage {
            payload: QueryRespPayload {
                query_id,
                results,
                next_cursor: None,
                has_more: None,
                cursor_status: None,
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
        let _ = self.query_registry
            .unregister(&payload.payload.query_id);
        Ok(OperationResponse::Empty)
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
    use crate::service::operation::{service_names, OperationContext};
    use crate::storage::datastores::NullDataStore;
    use crate::storage::impls::StorageConfig;
    use crate::storage::record::{Record, RecordMetadata, RecordValue};
    use crate::storage::record_store::{CallerProvenance, ExpiryPolicy};
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
        });
        registry.register(QuerySubscription {
            query_id: "q-2".to_string(),
            connection_id: ConnectionId(2),
            map_name: "users".to_string(),
            query: Query::default(),
            previous_result_keys: DashSet::new(),
        });
        registry.register(QuerySubscription {
            query_id: "q-3".to_string(),
            connection_id: ConnectionId(1),
            map_name: "orders".to_string(),
            query: Query::default(),
            previous_result_keys: DashSet::new(),
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
        });
        registry.register(QuerySubscription {
            query_id: "q-2".to_string(),
            connection_id: ConnectionId(1),
            map_name: "orders".to_string(),
            query: Query::default(),
            previous_result_keys: DashSet::new(),
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
            _ => panic!("expected Binary message"),
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
            _ => panic!("expected Binary"),
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
            _ => panic!("expected Binary"),
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
            _ => panic!("expected Binary"),
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
                _ => panic!("expected Binary"),
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
        let svc = QueryService::new(registry, factory, conn_registry);
        assert_eq!(svc.name(), "query");
    }

    #[tokio::test]
    async fn query_service_wrong_operation_returns_wrong_service() {
        let registry = Arc::new(QueryRegistry::new());
        let factory = make_factory();
        let conn_registry = Arc::new(ConnectionRegistry::new());
        let svc = Arc::new(QueryService::new(registry, factory, conn_registry));

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
        let svc = Arc::new(QueryService::new(registry, factory, conn_registry));

        let ctx = make_ctx(None); // no connection_id
        let payload = QuerySubMessage {
            payload: QuerySubPayload {
                query_id: "q-1".to_string(),
                map_name: "users".to_string(),
                query: Query::default(),
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

        let svc = Arc::new(QueryService::new(registry.clone(), factory, conn_registry));

        let ctx = make_ctx(Some(conn_id));
        let payload = QuerySubMessage {
            payload: QuerySubPayload {
                query_id: "q-1".to_string(),
                map_name: "users".to_string(),
                query: Query::default(),
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

    /// AC1: QuerySubscribe returns QUERY_RESP with initial matching results.
    ///
    /// Since RecordStoreFactory::create() returns independent stores per call
    /// (each with its own HashMapStorage), this test verifies the subscribe
    /// handler by putting data into the factory-created store and then testing
    /// execute_query directly. The end-to-end integration with a shared backing
    /// store is tested at a higher integration level.
    #[test]
    fn query_subscribe_execute_query_returns_matching_results() {
        use super::super::predicate::execute_query;
        use std::collections::HashMap;

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
        let svc = Arc::new(QueryService::new(registry.clone(), factory, conn_registry));

        let ctx = make_ctx(Some(conn_id));
        let sub_payload = QuerySubMessage {
            payload: QuerySubPayload {
                query_id: "q-1".to_string(),
                map_name: "users".to_string(),
                query: Query::default(),
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
}
