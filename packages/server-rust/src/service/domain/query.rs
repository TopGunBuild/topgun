//! Query domain service for live query subscriptions.
//!
//! Manages an in-memory `QueryRegistry` of standing query subscriptions,
//! evaluates queries against `RecordStore` contents, and pushes incremental
//! `QUERY_UPDATE` messages (ENTER/UPDATE/LEAVE) to subscribers through
//! [`QueryRegistry::deliver_mutation`], the one live-delivery path, which
//! `CrdtService` calls under the key's writer for every applied write.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use async_trait::async_trait;
use dashmap::{DashMap, DashSet};
use tokio::sync::mpsc::error::TrySendError;
use topgun_core::LWWRecord;
use tower::Service;

use topgun_core::messages::base::{ChangeEventType, Query};
use topgun_core::messages::client_events::QueryUpdatePayload;
use topgun_core::messages::query::{QueryRespMessage, QueryRespPayload, QueryResultEntry};
use topgun_core::messages::vector::{
    VectorSearchPayload, VectorSearchRespPayload, VectorSearchResult,
};
use topgun_core::messages::Message;
use topgun_core::vector::distance::DistanceMetric;

use crate::dag::coordinator::{run_dag_local, ClusterQueryCoordinator};
use crate::query::cursor::{
    build_next_cursor, classify_cursor_status, cursor_query_hashes, decode_cursor, is_after_cursor,
    SortValue,
};
use crate::query::delta_buffer::{
    Captured, Closing, DeltaBuffer, DeltaEntry, QUERY_SNAPSHOT_OVERFLOW,
};
use crate::query::window::{LiveWindow, WindowDelta};

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
use crate::storage::expiry::lww_is_live;
use crate::storage::record::RecordValue;
use crate::storage::RecordStoreFactory;

/// Machine-distinguishable `SqlQueryResp.code` returned when SQL is unavailable
/// (the `datafusion` feature is compiled out, or compiled in without a wired
/// backend). Clients match on this code rather than the human-readable message.
pub const SQL_FEATURE_DISABLED_CODE: &str = "FEATURE_DISABLED";

/// Distinct keys a live subscribe's snapshot buffer captures before it
/// overflows (TG-LIVE-001(c)).
const SNAPSHOT_BUFFER_CAPACITY: usize = 4096;

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
    /// Nothing reads or writes this set: window membership lives in
    /// `live_window`. It is still declared because test constructors name it.
    pub previous_result_keys: DashSet<String>,
    /// Sorted top-N window maintaining live membership for this subscription.
    ///
    /// Interior-mutable (its state lives behind a `Mutex`), so mutations route through
    /// `apply_mutation(&self, …)` over the `Arc<QuerySubscription>` with no `&mut` path.
    /// This is the single authoritative window algorithm — live delivery and the
    /// snapshot fold derive every ENTER/UPDATE/LEAVE (including top-N displacement
    /// and promotion) from it, through [`QuerySubscription::apply_and_project`].
    pub live_window: Arc<LiveWindow>,
    /// Optional field projection list. When `Some`, only these fields are
    /// included in `QUERY_RESP` and `QUERY_UPDATE` payloads sent to this subscriber.
    pub fields: Option<Vec<String>>,
    /// The snapshot buffer (TG-LIVE-001): active from before the subscription
    /// is registered until its subscribe enqueues the snapshot. While active,
    /// live delivery records every write of the map in it, the subscriber's
    /// own included, and the subscribe folds them into the snapshot.
    pub delta_buffer: Arc<DeltaBuffer>,
}

impl QuerySubscription {
    /// Runs writes through this subscription's window at once (one write on
    /// the live path, every captured write in the snapshot fold) and returns
    /// the deltas the subscriber is sent: the window's delta set (ENTER,
    /// displacement LEAVE, promotion ENTER, in-window UPDATE, delete LEAVE),
    /// without the rows outside the subscription's cursor page, with ENTER and
    /// UPDATE values projected to `fields` and LEAVE values `Nil`.
    ///
    /// Live delivery and the snapshot fold share it, so a write folded into
    /// the snapshot leaves the window and the rows exactly as the live path
    /// would have (TG-LIVE-001(b)).
    #[must_use]
    pub fn apply_and_project<'a>(
        &self,
        mutations: impl IntoIterator<Item = (&'a str, Option<&'a rmpv::Value>, bool)>,
    ) -> Vec<WindowDelta> {
        let deltas = self.live_window.apply_batch(mutations);
        if deltas.is_empty() {
            return deltas;
        }
        // A row strictly before the cursor is on an earlier page the subscriber
        // is not observing: one that enters there is not sent, and one updated
        // to there leaves this page, so the subscriber drops the row it holds. A
        // LEAVE carries no row value to test, so it is always delivered.
        let page_cursor = self.query.cursor.as_deref().and_then(decode_cursor);
        deltas
            .into_iter()
            .filter_map(|mut delta| {
                if let Some(cursor) = &page_cursor {
                    let before = !matches!(delta.event, ChangeEventType::LEAVE)
                        && !is_after_cursor(&delta.key, &delta.value, cursor);
                    match delta.event {
                        ChangeEventType::ENTER if before => return None,
                        ChangeEventType::UPDATE if before => delta.event = ChangeEventType::LEAVE,
                        _ => {}
                    }
                }
                if matches!(delta.event, ChangeEventType::LEAVE) {
                    delta.value = rmpv::Value::Nil;
                } else if let Some(fields) = &self.fields {
                    delta.value = project_fields(fields, &delta.value);
                }
                Some(delta)
            })
            .collect()
    }
}

/// Whether `data` matches the query's predicate or where clause; a query with
/// neither matches every value.
fn matches_query(query: &Query, data: &rmpv::Value) -> bool {
    if let Some(pred) = &query.predicate {
        evaluate_predicate(pred, &EvalContext::data_only(data))
    } else if let Some(wh) = &query.r#where {
        evaluate_where(wh, data)
    } else {
        true
    }
}

// ---------------------------------------------------------------------------
// QueryRegistry
// ---------------------------------------------------------------------------

/// In-memory registry of standing query subscriptions.
///
/// Thread-safe via `DashMap`. Keyed by `map_name` for efficient lookup
/// during mutation observation, then by the owning connection and the
/// client-chosen `query_id`: two connections may pick the same id, and neither
/// may replace or remove the other's subscription.
///
/// Lock order (TG-LIVE-001): a subscription's buffer lock may be taken before a
/// registry lock, never after. Nothing here holds a registry guard while it
/// takes a buffer lock.
pub struct QueryRegistry {
    /// `map_name` -> { (`connection_id`, `query_id`) -> `QuerySubscription` }
    subscriptions: DashMap<String, DashMap<(ConnectionId, String), Arc<QuerySubscription>>>,
}

impl QueryRegistry {
    /// Creates a new empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            subscriptions: DashMap::new(),
        }
    }

    /// Registers a standing query subscription, replacing one of the same
    /// connection with the same `query_id` on the same map.
    pub fn register(&self, sub: QuerySubscription) {
        let map_name = sub.map_name.clone();
        let key = (sub.connection_id, sub.query_id.clone());
        let sub = Arc::new(sub);

        self.subscriptions
            .entry(map_name)
            .or_default()
            .insert(key, sub);
    }

    /// Removes `connection_id`'s subscription `query_id`. Returns `true` if
    /// found; another connection's subscription of the same id is untouched.
    #[must_use]
    pub fn unregister(&self, connection_id: ConnectionId, query_id: &str) -> bool {
        let key = (connection_id, query_id.to_string());
        let mut found = false;
        // Iterate all maps to find the subscription
        self.subscriptions.retain(|_, inner| {
            if inner.remove(&key).is_some() {
                found = true;
            }
            // Remove the outer entry if the inner map is now empty
            !inner.is_empty()
        });
        found
    }

    /// Removes `connection_id`'s subscription `query_id` on `map_name` only if
    /// it is still the registration that owns `buffer`, so a subscribe that
    /// gives up never removes a newer registration of the same id, nor one of
    /// the same id on another map. Returns whether it removed one; drops the
    /// map's entry once it holds no subscription.
    pub fn remove_if_same(
        &self,
        map_name: &str,
        connection_id: ConnectionId,
        query_id: &str,
        buffer: &Arc<DeltaBuffer>,
    ) -> bool {
        let key = (connection_id, query_id.to_string());
        // The outer read guard must be gone before the outer `remove_if` below,
        // which takes the write lock of the same shard: DashMap shard locks are
        // not reentrant.
        let removed = {
            let Some(inner) = self.subscriptions.get(map_name) else {
                return false;
            };
            inner
                .remove_if(&key, |_, sub| Arc::ptr_eq(&sub.delta_buffer, buffer))
                .is_some()
        };
        self.subscriptions
            .remove_if(map_name, |_, inner| inner.is_empty());
        removed
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

    /// Delivers one applied write of `key` in `map_name` to every live query
    /// of the map (TG-LIVE-001). The caller holds the key's writer, so one
    /// key's writes are delivered in the order the store applied them.
    ///
    /// `new_record` is the write event's record: the stored record of an LWW
    /// put, else `None` or a value-less record (an LWW remove, an expiry
    /// remove, an OR add or remove), both a non-match. So is a record already
    /// expired at `now`, the store clock (TG-TTL-001), as the fold treats it.
    ///
    /// Per subscription the write is first offered to its snapshot buffer: a
    /// subscription whose snapshot is still being built captures it, the
    /// writer's own included, and the snapshot carries it. Otherwise the
    /// window's deltas are sent as `QUERY_UPDATE`s, except to `exclude`, the
    /// writing connection. Holds no registry guard while it takes a buffer
    /// lock: it walks a cloned list of the map's subscriptions.
    pub fn deliver_mutation(
        &self,
        connections: &ConnectionRegistry,
        map_name: &str,
        key: &str,
        new_record: Option<&LWWRecord<rmpv::Value>>,
        exclude: Option<ConnectionId>,
        now: u64,
    ) {
        let subs = self.get_subscriptions_for_map(map_name);
        if subs.is_empty() {
            return;
        }
        let new_value = new_record
            .filter(|r| lww_is_live(&r.timestamp, r.ttl_ms, now))
            .and_then(|r| r.value.as_ref());
        for sub in &subs {
            let matches = new_value.is_some_and(|v| matches_query(&sub.query, v));
            if sub.delta_buffer.route(key, new_record, matches) {
                continue;
            }
            if exclude == Some(sub.connection_id) {
                continue;
            }
            let deltas = sub.apply_and_project([(key, new_value, matches)]);
            if deltas.is_empty() {
                continue;
            }
            let Some(handle) = connections.get(sub.connection_id) else {
                continue;
            };
            for delta in deltas {
                let msg = Message::QueryUpdate {
                    payload: QueryUpdatePayload {
                        query_id: sub.query_id.clone(),
                        key: delta.key,
                        value: delta.value,
                        change_type: delta.event,
                    },
                };
                if let Ok(bytes) = rmp_serde::to_vec_named(&msg) {
                    let _ = handle.try_send_broadcast(OutboundMessage::Binary(bytes));
                }
            }
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

    /// Looks up `connection_id`'s subscription `query_id` on `map_name`.
    #[must_use]
    pub fn get_subscription(
        &self,
        map_name: &str,
        connection_id: ConnectionId,
        query_id: &str,
    ) -> Option<Arc<QuerySubscription>> {
        self.subscriptions.get(map_name).and_then(|inner| {
            inner
                .get(&(connection_id, query_id.to_string()))
                .map(|sub| Arc::clone(sub.value()))
        })
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
// Live subscribe: registration guard and snapshot fold
// ---------------------------------------------------------------------------

/// Owns a live subscribe's registration from `register` until the snapshot is
/// delivered or refused. Dropped while armed — the handler returned an error,
/// was cancelled by the pipeline's timeout, or its future was dropped — it
/// seals the buffer and removes the registration by identity, so a subscribe
/// that ends before delivery leaves no registration (TG-LIVE-001(d)).
struct SubscribeGuard {
    registry: Arc<QueryRegistry>,
    map_name: String,
    connection_id: ConnectionId,
    query_id: String,
    buffer: Arc<DeltaBuffer>,
    armed: bool,
}

impl Drop for SubscribeGuard {
    fn drop(&mut self) {
        if self.armed {
            // Seal first and release the buffer lock before the registry lock
            // (lock order: buffer before registry, never the other way round).
            self.buffer.seal();
            self.registry.remove_if_same(
                &self.map_name,
                self.connection_id,
                &self.query_id,
                &self.buffer,
            );
        }
    }
}

/// Folds the writes captured during a subscribe's scan into its snapshot
/// rows: they go through the window in one batch, which leaves it as the
/// live path would applying them one by one, and the deltas edit `results` — ENTER and UPDATE upsert the projected value
/// by key (an existing key keeps its position, a new key is appended), LEAVE
/// removes the key. A captured value expired at `now` (TG-TTL-001) is folded as
/// "no row", as the scan would have left it out. Returns whether a delta
/// changed the rows; their order is then the caller's to restore.
fn fold_captured(
    results: &mut Vec<QueryResultEntry>,
    sub: &QuerySubscription,
    entries: &[(String, DeltaEntry)],
    now: u64,
) -> bool {
    let mut slots: Vec<Option<QueryResultEntry>> = results.drain(..).map(Some).collect();
    let mut index: HashMap<String, usize> = slots
        .iter()
        .enumerate()
        .filter_map(|(i, slot)| slot.as_ref().map(|e| (e.key.clone(), i)))
        .collect();
    let mutations = entries.iter().map(|(key, entry)| {
        let live_value = entry.record.as_ref().and_then(|record| {
            record
                .value
                .as_ref()
                .filter(|_| lww_is_live(&record.timestamp, record.ttl_ms, now))
        });
        (
            key.as_str(),
            live_value,
            live_value.is_some() && entry.matches,
        )
    });
    let deltas = sub.apply_and_project(mutations);
    let changed = !deltas.is_empty();
    for delta in deltas {
        match delta.event {
            ChangeEventType::ENTER | ChangeEventType::UPDATE => {
                let row = QueryResultEntry {
                    key: delta.key,
                    value: delta.value,
                };
                if let Some(&i) = index.get(&row.key) {
                    slots[i] = Some(row);
                } else {
                    index.insert(row.key.clone(), slots.len());
                    slots.push(Some(row));
                }
            }
            ChangeEventType::LEAVE => {
                if let Some(&i) = index.get(&delta.key) {
                    slots[i] = None;
                }
            }
        }
    }
    results.extend(slots.into_iter().flatten());
    changed
}

/// The frame a subscriber is sent for `snapshot` with the `entries` captured
/// during its scan folded in. Runs under the snapshot buffer's lock.
fn folded_snapshot_frame(
    mut snapshot: QueryRespPayload,
    page_cursor: &PageCursor,
    sub: &QuerySubscription,
    entries: &[(String, DeltaEntry)],
    now: u64,
) -> Result<Vec<u8>, rmp_serde::encode::Error> {
    if fold_captured(&mut snapshot.results, sub, entries, now) {
        page_cursor.repaginate(&mut snapshot, &sub.live_window);
    }
    rmp_serde::to_vec_named(&Message::QueryResp(QueryRespMessage { payload: snapshot }))
}

/// What a snapshot page's `next_cursor` is built from, kept so that a fold
/// that changes the page can rebuild it.
struct PageCursor {
    limit: Option<u32>,
    template: Vec<SortValue>,
    predicate_hash: u64,
    sort_hash: u64,
    now_ms: i64,
}

impl PageCursor {
    /// The cursor after `last`, the page's last row in the query's order, when
    /// more rows follow a page of a sorted, limited query.
    fn next(&self, has_more: Option<bool>, last: Option<&QueryResultEntry>) -> Option<String> {
        if has_more != Some(true) || self.limit.is_none() || self.template.is_empty() {
            return None;
        }
        last.map(|last| {
            build_next_cursor(
                &last.key,
                &last.value,
                &self.template,
                self.predicate_hash,
                self.sort_hash,
                self.now_ms,
            )
        })
    }

    /// Restores a folded page: its rows in the query's order, `has_more` also
    /// when the fold pushed rows past the page, and the cursor after its new
    /// last row, so paging on from it skips no row.
    fn repaginate(&self, page: &mut QueryRespPayload, window: &LiveWindow) {
        page.results
            .sort_by(|a, b| window.compare((&a.key, &a.value), (&b.key, &b.value)));
        if window.holds_past_limit() {
            page.has_more = Some(true);
        }
        page.next_cursor = self.next(page.has_more, page.results.last());
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
    /// The connections a live subscribe enqueues its snapshot on.
    connection_registry: Arc<ConnectionRegistry>,
    /// Maximum records returned in a single `QUERY_RESP`. Queries matching more
    /// records are clamped to this limit with `has_more: true`.
    max_query_records: u32,
    /// The index registries. Read by `VECTOR_SEARCH` and by the tests-only
    /// linear engine; the production DAG and cluster coordinator paths do not
    /// read it, so passing it changes no scalar query result. `None` on
    /// sim/test call sites, where `VECTOR_SEARCH` answers with an error.
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
    #[cfg(test)]
    /// The hold a test may arm right after a live subscribe registers, before
    /// its scan.
    register_hold: DrainHold,
    #[cfg(test)]
    /// The hold a test may arm between the end of a live subscribe's scan and
    /// the seeding of its window, with its buffer active.
    scan_hold: DrainHold,
}

/// `QUERY_RESP.code` of a refused live subscription that carries a non-empty
/// `group_by` or `aggregations`: the live fan-out sends per-record deltas, which
/// would mix raw records into a grouped result.
pub const QUERY_LIVE_AGGREGATE_UNSUPPORTED: &str = "QUERY_LIVE_AGGREGATE_UNSUPPORTED";

/// Whether a query groups or aggregates. Non-empty is the DAG's own test: an empty
/// `group_by` / `aggregations` changes nothing in the result, so it stays live.
fn is_live_aggregate(query: &Query) -> bool {
    query.group_by.as_ref().is_some_and(|g| !g.is_empty())
        || query.aggregations.as_ref().is_some_and(|a| !a.is_empty())
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
    /// `index_observer_factory` drives `VECTOR_SEARCH` and the tests-only
    /// linear engine; the production DAG and cluster coordinator paths do not
    /// read it, so it changes no scalar query result. With `None` (sim/test
    /// call sites) `VECTOR_SEARCH` answers with an error.
    #[must_use]
    pub fn new(
        query_registry: Arc<QueryRegistry>,
        record_store_factory: Arc<RecordStoreFactory>,
        connection_registry: Arc<ConnectionRegistry>,
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
            max_query_records,
            index_observer_factory,
            #[cfg(feature = "datafusion")]
            sql_query_backend,
            coordinator: None,
            linear_engine_for_tests: false,
            #[cfg(test)]
            register_hold: DrainHold::default(),
            #[cfg(test)]
            scan_hold: DrainHold::default(),
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

    /// Returns a clone of the inner `Arc<QueryRegistry>`.
    ///
    /// Used by production wiring in `build_app()` so `AppState` can hold an
    /// `Arc<QueryRegistry>` directly without going through the service at
    /// disconnect time (avoids racing with service shutdown).
    #[must_use]
    pub fn query_registry_arc(&self) -> Arc<QueryRegistry> {
        Arc::clone(&self.query_registry)
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
    /// A live request (no `snapshot_only`) that groups or aggregates is refused
    /// first with `QUERY_LIVE_AGGREGATE_UNSUPPORTED`, before anything is
    /// registered. A `snapshot_only` request runs the scan, the clamp and the
    /// projection and returns its `QUERY_RESP`: it builds no window or buffer
    /// and registers nothing.
    ///
    /// A live subscribe (TG-LIVE-001):
    /// 1. Builds an empty `LiveWindow` and a `DeltaBuffer`, activates the buffer
    ///    and only then registers the subscription, so every fan-out that finds
    ///    the registration finds a capturing buffer. A guard armed right after
    ///    `register` removes the registration if the handler ends before step 5.
    /// 2. Scans the map (DAG scan — no per-query Merkle on the full-scan path)
    ///    and clamps to `max_query_records`.
    /// 3. Seeds the window from the snapshot rows, projects them, and
    ///    serializes the scan snapshot.
    /// 4. Closes the buffer under its lock: checks that this registration is
    ///    still the registered one, reserves a slot on the connection, folds the
    ///    captured writes into the snapshot and sends it through the slot. A
    ///    write routed meanwhile waits on the lock and is sent live after the
    ///    snapshot. Returns `Empty`: the snapshot is already enqueued.
    /// 5. On a buffer overflow or a full channel, seals the buffer, removes this
    ///    registration and returns `[scan snapshot, QUERY_SNAPSHOT_OVERFLOW]` on
    ///    the response path; on a closed connection, or a registration that is
    ///    gone, sends nothing.
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
        let snapshot_only = payload.payload.snapshot_only;

        // A live subscription's fan-out sends per-record deltas, which cannot keep a
        // grouped result coherent; refuse it before anything is built or registered,
        // so a refusal leaves no subscription (and no buffer) behind. A one-shot
        // request (`snapshot_only`) is served: it is never registered.
        if !snapshot_only && is_live_aggregate(&query) {
            return Ok(OperationResponse::Message(Box::new(Message::QueryResp(
                QueryRespMessage {
                    payload: QueryRespPayload {
                        query_id,
                        error: Some(
                            "a live query cannot carry groupBy or aggregations; run it as a \
                             one-shot query (queryOnce / queryOncePaged)"
                                .to_string(),
                        ),
                        code: Some(QUERY_LIVE_AGGREGATE_UNSUPPORTED.to_string()),
                        ..Default::default()
                    },
                },
            ))));
        }

        // A value-ORDER-BY without a LIMIT is an in-memory sort over the matched set.
        // It is O(result) — the same memory profile as a no-LIMIT full scan, which is
        // allowed — and is soft-capped by `max_query_records`. Rejecting it outright
        // (while permitting the equally-O(N) no-LIMIT scan) was an asymmetric overreach,
        // so it is permitted here. The genuine OOM exposure is sorting over a
        // non-resident / larger-than-RAM match set, which is the streaming-source
        // problem (TODO-532, the TODO-530 family): the protection belongs there as a
        // SIZE/RESIDENCY-gated reject, not as a blanket reject of the sort clause. The
        // `QUERY_UNBOUNDED_SORT` code is retained for that gated reject.

        // A one-shot request needs no window, buffer or registration: nothing is
        // registered for it, so no live update can follow its snapshot, and a
        // subscription already registered under the same id is left untouched.
        let live = if snapshot_only {
            None
        } else {
            let live_window = Arc::new(LiveWindow::new(
                query.sort.clone().unwrap_or_default(),
                query.limit,
            ));
            let delta_buffer = Arc::new(DeltaBuffer::new(SNAPSHOT_BUFFER_CAPACITY));
            // Active before it is registered: a fan-out that finds the registration
            // must find a capturing buffer, or its update could precede the snapshot.
            delta_buffer.activate();
            self.query_registry.register(QuerySubscription {
                query_id: query_id.clone(),
                connection_id,
                map_name: map_name.clone(),
                query: query.clone(),
                previous_result_keys: DashSet::new(),
                live_window: Arc::clone(&live_window),
                fields: fields.clone(),
                delta_buffer: Arc::clone(&delta_buffer),
            });
            let guard = SubscribeGuard {
                registry: Arc::clone(&self.query_registry),
                map_name: map_name.clone(),
                connection_id,
                query_id: query_id.clone(),
                buffer: Arc::clone(&delta_buffer),
                armed: true,
            };
            #[cfg(test)]
            self.register_hold.pass(&query_id).await;
            Some((live_window, delta_buffer, guard))
        };

        // Stream the snapshot (no per-query Merkle on the full-scan path).
        // Per-query Merkle is retired for full-scans (SPEC-322c SYNC/QUERY separation).
        // The map-level Merkle tree-walk SYNC is a separate track.

        // Capture cursor-emission inputs before `query` is moved into QuerySubscription.
        let (predicate_hash, sort_hash) = cursor_query_hashes(&query);
        let query_limit = query.limit;
        let input_cursor = query.cursor.as_deref().and_then(decode_cursor);
        let sort_values_template: Vec<SortValue> = query
            .sort
            .as_ref()
            .map(|sf| {
                sf.iter()
                    .map(|f| SortValue {
                        field: f.field.clone(),
                        value: serde_json::Value::Null,
                        direction: f.direction.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default();

        let stores = self.record_store_factory.get_all_for_map(&map_name);

        let mut coordinator_has_more = false;
        let mut local_has_more = false;

        let mut results: Vec<QueryResultEntry> = if self.linear_engine_for_tests {
            // The linear test engine evaluates predicates directly over a flat
            // in-memory snapshot. Only this path needs the materialised entry list;
            // the production DAG and coordinator paths re-scan via the streaming
            // `ScanProcessor`, so collecting `entries` for them would be a wasted
            // O(resident) copy.
            let mut entries: Vec<(String, rmpv::Value)> = Vec::new();
            for store in &stores {
                store.for_each_boxed(
                    &mut |key, record| {
                        if let RecordValue::Lww { ref value, .. } = record.value {
                            entries.push((key.to_string(), value_to_rmpv(value)));
                        }
                    },
                    false,
                );
            }
            let entries = if let (Some(factory), Some(predicate)) = (
                self.index_observer_factory.as_ref(),
                query.predicate.as_ref(),
            ) {
                if let Some(registry) = factory.get_registry(&map_name) {
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
            predicate_execute_query(entries, &query)
        } else if let Some(ref coordinator) = self.coordinator {
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

        let page_has_more = coordinator_has_more || local_has_more;

        // Apply max_query_records clamping.
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

        #[cfg(test)]
        if live.is_some() {
            self.scan_hold.pass(&query_id).await;
        }

        // Seed the window from the snapshot page, before field projection so
        // sort-field values are present for ordering.
        if let Some((live_window, _, _)) = &live {
            let _ = live_window.apply_batch(
                results
                    .iter()
                    .map(|entry| (entry.key.as_str(), Some(&entry.value), true)),
            );
        }

        // Apply field projection.
        if let Some(ref proj_fields) = fields {
            for entry in &mut results {
                entry.value = project_fields(proj_fields, &entry.value);
            }
        }

        // Cursor emission.
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
            .unwrap_or(0);

        let page_cursor = PageCursor {
            limit: query_limit,
            template: sort_values_template,
            predicate_hash,
            sort_hash,
            now_ms,
        };
        let next_cursor = page_cursor.next(has_more, results.last());

        let cursor_status = Some(classify_cursor_status(
            input_cursor.as_ref(),
            now_ms,
            predicate_hash,
            sort_hash,
        ));

        let snapshot = QueryRespPayload {
            query_id: query_id.clone(),
            results,
            next_cursor,
            has_more,
            cursor_status,
            // Per-query Merkle is retired for full-scans; root hash is omitted.
            merkle_root_hash: None,
            ..Default::default()
        };

        let Some((_, delta_buffer, mut guard)) = live else {
            return Ok(OperationResponse::Message(Box::new(Message::QueryResp(
                QueryRespMessage { payload: snapshot },
            ))));
        };

        self.deliver_live_snapshot(snapshot, &page_cursor, &delta_buffer, &mut guard)
    }

    /// Delivers a live subscribe's snapshot (TG-LIVE-001): closes the buffer
    /// and, under its lock, either enqueues the snapshot with the captured
    /// writes folded in, or refuses the registration. Disarms the guard once
    /// the registration is delivered or removed.
    fn deliver_live_snapshot(
        &self,
        snapshot: QueryRespPayload,
        page_cursor: &PageCursor,
        delta_buffer: &Arc<DeltaBuffer>,
        guard: &mut SubscribeGuard,
    ) -> Result<OperationResponse, OperationError> {
        let map_name = guard.map_name.as_str();
        let connection_id = guard.connection_id;
        let query_id = guard.query_id.as_str();

        // Serialized outside the lock: when nothing was captured, these bytes are
        // what is sent, so no serialization runs under the lock.
        let scan_msg = Message::QueryResp(QueryRespMessage { payload: snapshot });
        let scan_bytes = rmp_serde::to_vec_named(&scan_msg)
            .map_err(|e| OperationError::Internal(anyhow::anyhow!("{e}")))?;
        let Message::QueryResp(QueryRespMessage { payload: snapshot }) = scan_msg else {
            return Err(OperationError::Internal(anyhow::anyhow!(
                "a live snapshot is a QUERY_RESP"
            )));
        };

        let mut outcome: Result<OperationResponse, OperationError> = Ok(OperationResponse::Empty);
        let mut overflow_reason: Option<&'static str> = None;
        let remove = || {
            self.query_registry
                .remove_if_same(map_name, connection_id, query_id, delta_buffer);
        };
        let overflow = |snapshot: QueryRespPayload, reason: &'static str| {
            let error = if reason == "buffer" {
                "Too many concurrent writes during snapshot; \
                 please resubscribe to get a fresh snapshot."
            } else {
                "The connection's outbound queue was full; \
                 please resubscribe to get a fresh snapshot."
            };
            OperationResponse::Messages(vec![
                Message::QueryResp(QueryRespMessage { payload: snapshot }),
                Message::QueryResp(QueryRespMessage {
                    payload: QueryRespPayload {
                        query_id: query_id.to_string(),
                        error: Some(error.to_string()),
                        code: Some(QUERY_SNAPSHOT_OVERFLOW.to_string()),
                        ..Default::default()
                    },
                }),
            ])
        };

        // Nothing below awaits: every writer of this map waits on the buffer lock.
        delta_buffer.close(|captured| {
            // An unsubscribe or a disconnect removed this registration during the
            // scan (or a resubscribe replaced it): send nothing and remove nothing.
            let Some(sub) = self
                .query_registry
                .get_subscription(map_name, connection_id, query_id)
                .filter(|sub| Arc::ptr_eq(&sub.delta_buffer, delta_buffer))
            else {
                return Closing::Sealed;
            };
            let entries = match captured {
                Captured::Overflowed => {
                    remove();
                    overflow_reason = Some("buffer");
                    outcome = Ok(overflow(snapshot, "buffer"));
                    return Closing::Sealed;
                }
                Captured::Entries(entries) => entries,
            };
            let Some(handle) = self.connection_registry.get(connection_id) else {
                remove();
                return Closing::Sealed;
            };
            // Reserved before the fold, so a full or closed channel skips a fold
            // whose output would be discarded.
            let permit = match handle.tx.try_reserve() {
                Ok(permit) => permit,
                Err(TrySendError::Full(())) => {
                    remove();
                    overflow_reason = Some("channel");
                    outcome = Ok(overflow(snapshot, "channel"));
                    return Closing::Sealed;
                }
                Err(TrySendError::Closed(())) => {
                    remove();
                    return Closing::Sealed;
                }
            };
            let bytes = if entries.is_empty() {
                scan_bytes
            } else {
                let now = self.record_store_factory.now_ms();
                match folded_snapshot_frame(snapshot, page_cursor, &sub, &entries, now) {
                    Ok(bytes) => bytes,
                    Err(e) => {
                        remove();
                        outcome = Err(OperationError::Internal(anyhow::anyhow!("{e}")));
                        return Closing::Sealed;
                    }
                }
            };
            permit.send(OutboundMessage::Binary(bytes));
            Closing::Live
        });
        guard.armed = false;

        if let Some(reason) = overflow_reason {
            metrics::counter!("topgun_query_snapshot_overflow_total", "reason" => reason)
                .increment(1);
        }
        outcome
    }

    /// Handles a `QueryUnsubscribe` operation: removes the calling
    /// connection's subscription of that id. An id the caller does not hold —
    /// another connection's, or none at all — is a no-op with the same answer.
    #[allow(clippy::unnecessary_wraps)]
    fn handle_query_unsubscribe(
        &self,
        ctx: &crate::service::operation::OperationContext,
        payload: &topgun_core::messages::query::QueryUnsubMessage,
    ) -> Result<OperationResponse, OperationError> {
        if let Some(connection_id) = ctx.connection_id {
            let _ = self
                .query_registry
                .unregister(connection_id, &payload.payload.query_id);
        }
        Ok(OperationResponse::Empty)
    }

    /// Handles a `QuerySyncInit` operation.
    ///
    /// The per-query Merkle sync path was retired when QUERY switched to bounded
    /// streaming full-scan (SYNC/QUERY separation). The wire operation is still
    /// dispatched for protocol compatibility; the handler is a no-op stub.
    #[allow(clippy::unused_async)]
    async fn handle_query_sync_init(
        &self,
        _ctx: &crate::service::operation::OperationContext,
        _payload: topgun_core::messages::query::QuerySyncInitMessage,
    ) -> Result<OperationResponse, OperationError> {
        Ok(OperationResponse::Empty)
    }

    /// Handles a `SqlQuery` operation.
    ///
    /// When the `datafusion` feature is enabled and a `SqlQueryBackend` is
    /// configured, executes the SQL string and converts Arrow `RecordBatch`es
    /// to `rmpv::Value` rows for wire transport.
    ///
    /// When SQL is not available — the `datafusion` feature is compiled out, or
    /// it is compiled in but no `SqlQueryBackend` is wired — this returns a
    /// `SqlQueryResp` carrying `code = FEATURE_DISABLED` rather than an
    /// `OperationError`. That distinction matters: a single-message dispatch
    /// error is logged and dropped (the client would hang until its request
    /// timeout, indistinguishable from a network stall), whereas a `SqlQueryResp`
    /// is correlated back to the caller by `query_id` and surfaces immediately as
    /// a machine-distinguishable "feature disabled" signal — not an opaque
    /// "internal error".
    #[cfg(feature = "datafusion")]
    async fn handle_sql_query(
        &self,
        _ctx: &crate::service::operation::OperationContext,
        payload: &topgun_core::messages::query::SqlQueryPayload,
    ) -> Result<OperationResponse, OperationError> {
        let Some(sql_backend) = self.sql_query_backend.as_ref() else {
            return Ok(Self::sql_feature_disabled_resp(
                &payload.query_id,
                "SQL is not available: the DataFusion backend is not configured on this server",
            ));
        };

        match sql_backend.execute_sql(&payload.sql).await {
            Ok(batches) => {
                let (columns, rows) = record_batches_to_rows(&batches);
                let resp_payload = topgun_core::messages::query::SqlQueryRespPayload {
                    query_id: payload.query_id.clone(),
                    columns,
                    rows,
                    error: None,
                    code: None,
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
                    code: None,
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
    ///
    /// Returns a `SqlQueryResp` with `code = FEATURE_DISABLED` (see the
    /// feature-enabled variant for why this is a response, not an error).
    #[cfg(not(feature = "datafusion"))]
    #[allow(clippy::unused_async)]
    async fn handle_sql_query(
        &self,
        _ctx: &crate::service::operation::OperationContext,
        payload: &topgun_core::messages::query::SqlQueryPayload,
    ) -> Result<OperationResponse, OperationError> {
        Ok(Self::sql_feature_disabled_resp(
            &payload.query_id,
            "SQL is not available: this server was built without the datafusion feature",
        ))
    }

    /// Builds a `SqlQueryResp` signalling that SQL is unavailable, tagged with the
    /// machine-distinguishable `FEATURE_DISABLED` code so the client can tell a
    /// disabled feature apart from a query/execution failure without string
    /// matching the human-readable message.
    fn sql_feature_disabled_resp(query_id: &str, message: &str) -> OperationResponse {
        OperationResponse::Message(Box::new(Message::SqlQueryResp {
            payload: topgun_core::messages::query::SqlQueryRespPayload {
                query_id: query_id.to_string(),
                columns: vec![],
                rows: vec![],
                error: Some(message.to_string()),
                code: Some(SQL_FEATURE_DISABLED_CODE.to_string()),
            },
        }))
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

        // 1. Require index registry. An error response, not an `Err`: a failed
        // dispatch is dropped and the client would wait for its timeout.
        let Some(factory) = self.index_observer_factory.as_ref() else {
            error_resp!(
                "vector search is not available on this server: no index registry is configured"
            );
        };
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

        // 8. Sort by descending score, then point-read the candidates in that
        // order until `k` are visible at `now` and pass the filter
        // (TG-VEC-001, TG-TTL-001). Only candidate keys are read, resident or
        // stored; an absent, deleted, expired or unreadable key is dropped
        // before the cut and never takes a slot.
        let filter = payload.options.as_ref().and_then(|o| o.filter.as_ref());
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
        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        let now = self.record_store_factory.now_ms();
        let mut kept: Vec<(String, f64, rmpv::Value)> = Vec::with_capacity(k);
        for (key, score) in scored {
            if kept.len() == k {
                break;
            }
            let Some(RecordValue::Lww { value, .. }) =
                crate::service::domain::search::read_visible(
                    &self.record_store_factory,
                    &payload.map_name,
                    &key,
                    now,
                )
                .await
            else {
                continue;
            };
            let value = value_to_rmpv(&value);
            if filter.is_some_and(|pred| !evaluate_predicate(pred, &EvalContext::data_only(&value)))
            {
                continue;
            }
            kept.push((key, score, value));
        }

        // 9. Build result entries.
        let extractor = AttributeExtractor::new(attribute.clone());
        let results: Vec<VectorSearchResult> = kept
            .into_iter()
            .map(|(key, score, value)| {
                let vector = if include_vectors {
                    extract_vector_bytes_le(&value, &extractor)
                } else {
                    None
                };
                let value = include_value.then_some(value);
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
/// The held subscribe's ends of the two signals of a hold: it sends "reached"
/// and awaits "release".
type DrainHoldGate = (
    tokio::sync::oneshot::Sender<()>,
    tokio::sync::oneshot::Receiver<()>,
);

#[cfg(test)]
/// At most one armed hold, for one `query_id`, at one point of a live
/// subscribe, so a test can write while the snapshot buffer is active,
/// deterministically. It belongs to one service, so tests running side by side
/// in one process cannot consume each other's hold. A hold must stay far below
/// the pipeline's operation timeout (30 s by default), which would cancel the
/// handler.
#[derive(Default)]
struct DrainHold(std::sync::Mutex<Option<(String, DrainHoldGate)>>);

#[cfg(test)]
impl DrainHold {
    /// Holds the caller iff the hold is armed for this query id: reports
    /// "reached", then waits until the test releases it or drops its end. The
    /// gate is taken out of the slot, which is what makes the hold fire once.
    async fn pass(&self, query_id: &str) {
        let gate = {
            let mut slot = self
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if slot.as_ref().is_some_and(|(id, _)| id.as_str() == query_id) {
                slot.take().map(|(_, gate)| gate)
            } else {
                None
            }
        };
        if let Some((reached, release)) = gate {
            let _ = reached.send(());
            let _ = release.await;
        }
    }
}

#[cfg(test)]
/// The test's side of a hold. Dropping it releases the held subscribe,
/// so a failing test never leaves a task held.
struct DrainHoldHandle {
    reached: Option<tokio::sync::oneshot::Receiver<()>>,
    release: Option<tokio::sync::oneshot::Sender<()>>,
}

#[cfg(test)]
impl DrainHoldHandle {
    /// Waits until the subscribe is held; panics after two seconds, which means
    /// the setup never reached the hold point.
    async fn wait_reached(&mut self) {
        let reached = self.reached.take().expect("wait_reached called once");
        tokio::time::timeout(std::time::Duration::from_secs(2), reached)
            .await
            .expect("the subscribe never reached the hold")
            .expect("hold dropped before it was reached");
    }

    fn release(&mut self) {
        if let Some(release) = self.release.take() {
            let _ = release.send(());
        }
    }
}

#[cfg(test)]
impl QueryService {
    /// Arms the hold for the next live subscribe of `query_id` on this service:
    /// it stops between the end of its scan (after the `max_query_records`
    /// clamp) and the seeding of its window, with the buffer active, the scan
    /// rows fixed and the window still empty.
    fn test_hold_after_scan(&self, query_id: &str) -> DrainHoldHandle {
        Self::arm_hold(&self.scan_hold, query_id)
    }

    /// Arms the hold for the next live subscribe of `query_id` on this service:
    /// it stops right after `register`, before the scan, with the buffer active.
    fn test_hold_after_register(&self, query_id: &str) -> DrainHoldHandle {
        Self::arm_hold(&self.register_hold, query_id)
    }

    fn arm_hold(hold: &DrainHold, query_id: &str) -> DrainHoldHandle {
        let (reached_tx, reached_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        *hold
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some((query_id.to_string(), (reached_tx, release_rx)));
        DrainHoldHandle {
            reached: Some(reached_rx),
            release: Some(release_tx),
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::doc_markdown,
    clippy::doc_lazy_continuation,
    clippy::uninlined_format_args
)]
mod tests {
    use crate::storage::key_writer::KeyWriteToken;
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
    use crate::storage::record::RecordValue;
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

    /// Returns a new inactive `DeltaBuffer` for test subscriptions that do not
    /// exercise the register-first → fence → drain lifecycle. The buffer is
    /// capacity 64 (generous for any in-test write count).
    fn inactive_buffer() -> Arc<DeltaBuffer> {
        Arc::new(DeltaBuffer::new(64))
    }

    /// The snapshot a live subscribe enqueued on its connection. The handler
    /// returns `Empty` (TG-LIVE-001): the `QUERY_RESP` is on the receiver, and
    /// no `QUERY_UPDATE` of the query precedes it.
    fn live_snapshot(
        resp: &OperationResponse,
        rx: &mut tokio::sync::mpsc::Receiver<OutboundMessage>,
    ) -> QueryRespMessage {
        assert!(
            matches!(resp, OperationResponse::Empty),
            "a live subscribe enqueues its snapshot itself"
        );
        while let Ok(OutboundMessage::Binary(bytes)) = rx.try_recv() {
            match rmp_serde::from_slice::<Message>(&bytes).expect("decodes") {
                Message::QueryResp(resp) => return resp,
                Message::QueryUpdate { payload } => {
                    panic!("a QUERY_UPDATE precedes the snapshot: {payload:?}")
                }
                _ => {}
            }
        }
        panic!("no QUERY_RESP was enqueued")
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
            delta_buffer: inactive_buffer(),
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
            delta_buffer: inactive_buffer(),
        });

        assert!(!registry.unregister(ConnectionId(2), "q-1"));
        assert!(registry.unregister(ConnectionId(1), "q-1"));
        assert_eq!(registry.subscription_count(), 0);
    }

    #[test]
    fn registry_unregister_returns_false_when_not_found() {
        let registry = QueryRegistry::new();
        assert!(!registry.unregister(ConnectionId(1), "nonexistent"));
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
            delta_buffer: inactive_buffer(),
        });
        registry.register(QuerySubscription {
            query_id: "q-2".to_string(),
            connection_id: ConnectionId(2),
            map_name: "users".to_string(),
            live_window: seeded_window(&Query::default(), &[]),
            query: Query::default(),
            previous_result_keys: DashSet::new(),
            fields: None,
            delta_buffer: inactive_buffer(),
        });
        registry.register(QuerySubscription {
            query_id: "q-3".to_string(),
            connection_id: ConnectionId(1),
            map_name: "orders".to_string(),
            live_window: seeded_window(&Query::default(), &[]),
            query: Query::default(),
            previous_result_keys: DashSet::new(),
            fields: None,
            delta_buffer: inactive_buffer(),
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
            delta_buffer: inactive_buffer(),
        });
        registry.register(QuerySubscription {
            query_id: "q-2".to_string(),
            connection_id: ConnectionId(1),
            map_name: "orders".to_string(),
            live_window: seeded_window(&Query::default(), &[]),
            query: Query::default(),
            previous_result_keys: DashSet::new(),
            fields: None,
            delta_buffer: inactive_buffer(),
        });

        let subs = registry.get_subscriptions_for_map("users");
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].query_id, "q-1");

        let subs = registry.get_subscriptions_for_map("nonexistent");
        assert!(subs.is_empty());
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
            10_000,
            None,
            #[cfg(feature = "datafusion")]
            None,
        );
        assert_eq!(svc.name(), "query");
    }

    /// SQL is not wired by default (no `SqlQueryBackend`), and the default build
    /// has the `datafusion` feature compiled out. Either way `handle_sql_query`
    /// must return a `SqlQueryResp` tagged `FEATURE_DISABLED` — correlated by
    /// `query_id` — rather than an `OperationError` that single-message dispatch
    /// would silently drop (leaving the client to hang until its timeout).
    #[tokio::test]
    async fn sql_query_unavailable_returns_feature_disabled_resp() {
        let registry = Arc::new(QueryRegistry::new());
        let factory = make_factory();
        let conn_registry = Arc::new(ConnectionRegistry::new());
        let svc = QueryService::new(
            registry,
            factory,
            conn_registry,
            10_000,
            None,
            #[cfg(feature = "datafusion")]
            None,
        );

        let ctx = make_ctx(None);
        let payload = topgun_core::messages::query::SqlQueryPayload {
            sql: "SELECT * FROM users".to_string(),
            query_id: "sq-test-1".to_string(),
        };

        let resp = svc
            .handle_sql_query(&ctx, &payload)
            .await
            .expect("unavailable SQL must be a response, not an error");

        match resp {
            OperationResponse::Message(msg) => match *msg {
                Message::SqlQueryResp { payload: p } => {
                    assert_eq!(p.query_id, "sq-test-1");
                    assert!(p.columns.is_empty());
                    assert!(p.rows.is_empty());
                    assert_eq!(p.code.as_deref(), Some(SQL_FEATURE_DISABLED_CODE));
                    assert!(p.error.is_some(), "human-readable message must be set");
                }
                other => panic!("expected SqlQueryResp, got: {other:?}"),
            },
            other => panic!("expected a single Message response, got: {other:?}"),
        }
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
                snapshot_only: false,
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
        let (handle, mut rx) = conn_registry.register(ConnectionKind::Client, &config);
        let conn_id = handle.id;

        let svc = Arc::new(QueryService::new(
            registry.clone(),
            factory,
            conn_registry,
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
                snapshot_only: false,
            },
        };
        let op = Operation::QuerySubscribe { ctx, payload };
        let resp = live_snapshot(&svc.oneshot(op).await.unwrap(), &mut rx);
        assert_eq!(resp.payload.query_id, "q-1");
        assert!(resp.payload.results.is_empty());

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
        use crate::storage::record_store::CallerProvenance;
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
                    &KeyWriteToken::for_test(map_name, key),
                    key,
                    RecordValue::Lww {
                        value,
                        timestamp: make_timestamp(),
                        ttl_ms: None,
                    },
                    CallerProvenance::Client,
                )
                .await
                .expect("put should succeed");
        }

        let conn_registry = Arc::new(ConnectionRegistry::new());
        let config = test_config();
        let (handle, mut rx) = conn_registry.register(ConnectionKind::Client, &config);
        let conn_id = handle.id;

        // No coordinator → single-node `run_dag_local` is the engine.
        let svc = Arc::new(QueryService::new(
            registry,
            factory,
            conn_registry,
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
                    // Bounded top-N: a limit exercises the bounded-heap scan path.
                    limit: Some(100),
                    ..Query::default()
                },
                fields: None,
                snapshot_only: false,
            },
        };
        let op = Operation::QuerySubscribe { ctx, payload };
        let resp = live_snapshot(&svc.oneshot(op).await.unwrap(), &mut rx);

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

    /// SPEC-322b F1 reproduce / SPEC-322c acceptance guard.
    ///
    /// A full-scan QUERY over a map whose durable record set exceeds the in-memory
    /// RAM ceiling must surface EVERY durable key (the TODO-530 completeness
    /// invariant) WITHOUT materialising the whole map resident at once (the L2
    /// RAM-ceiling invariant). The executor now uses `scan_via_datastore` to stream
    /// records directly from the durable backend without making them resident first,
    /// so completeness no longer requires full residency. This guard asserts the
    /// streaming end-state (SPEC-322c): all N keys returned AND peak residency
    /// bounded well below N.
    #[tokio::test(flavor = "multi_thread")]
    async fn full_scan_over_oversized_map_streams_within_ceiling() {
        use crate::storage::datastores::RedbDataStore;
        use crate::storage::map_data_store::MapDataStore;

        const N: usize = 400;
        // "One batch worth" — peak residency must NOT scale with the durable set.
        const RESIDENT_BOUND: usize = N / 4;

        let dir = tempfile::tempdir().expect("tempdir");
        let data_store =
            Arc::new(RedbDataStore::new(dir.path().join("oversized.redb")).expect("redb open"));
        let factory = Arc::new(RecordStoreFactory::new(
            StorageConfig::default(),
            data_store.clone(),
            Vec::new(),
        ));
        let map_name = "oversized";

        // Durable-but-non-resident: written straight to the datastore, not the engine.
        for i in 0..N {
            let value = RecordValue::Lww {
                value: Value::String(format!("v{i:04}")),
                timestamp: make_timestamp(),
                ttl_ms: None,
            };
            data_store
                .add(map_name, &format!("k{i:04}"), &value, 1000)
                .await
                .expect("durable write");
        }

        let conn_registry = Arc::new(ConnectionRegistry::new());
        let config = test_config();
        let (handle, mut rx) = conn_registry.register(ConnectionKind::Client, &config);
        let conn_id = handle.id;

        let svc = Arc::new(QueryService::new(
            Arc::new(QueryRegistry::new()),
            factory.clone(),
            conn_registry,
            10_000,
            None,
            #[cfg(feature = "datafusion")]
            None,
        ));

        let ctx = make_ctx(Some(conn_id));
        let payload = QuerySubMessage {
            payload: QuerySubPayload {
                query_id: "oversized-1".to_string(),
                map_name: map_name.to_string(),
                query: Query::default(), // full-scan: no predicate, sort, or limit
                fields: None,
                snapshot_only: false,
            },
        };
        let op = Operation::QuerySubscribe { ctx, payload };
        let resp = live_snapshot(&svc.oneshot(op).await.unwrap(), &mut rx);

        // (1) COMPLETENESS — the TODO-530 invariant (holds for both designs).
        assert_eq!(
            resp.payload.results.len(),
            N,
            "full-scan must surface every durable key"
        );

        // (2) BOUNDEDNESS — the L2 RAM-ceiling invariant. Surfacing all N keys must
        // NOT require holding all N resident. hydrate-then-scan FAILS here
        // (residency == N); a datastore-streaming response (SPEC-322c) keeps peak
        // residency bounded by one batch.
        let resident: usize = factory.all_stores().iter().map(|s| s.size()).sum();
        assert!(
            resident <= RESIDENT_BOUND,
            "F1: full-scan made {resident} of {N} records resident — completeness is \
             coupled to full residency, so the RAM ceiling cannot hold for maps larger \
             than it. SPEC-322c must stream from the datastore (peak residency <= {RESIDENT_BOUND})."
        );
    }

    /// Bounded top-N companion to the AC1 gate: a LIMIT query over a map whose durable
    /// set is far larger than the page returns the correct page (first `LIMIT` keys in
    /// ascending key order) sourced from non-resident records, WITHOUT making the whole
    /// map resident and WITHOUT buffering every non-resident row. The scan feeds the
    /// streamed datastore rows through a capacity-(limit+1) heap, so the page is
    /// produced from O(resident)+O(limit)+one batch — never O(durable map). The
    /// pre-fix behaviour materialised all N rows in the scan buffer to return a 10-row
    /// page; this test pins the bounded end-state (SPEC-322c C1 / Review v3).
    #[tokio::test(flavor = "multi_thread")]
    async fn bounded_full_scan_limit_returns_topn_without_full_residency() {
        use crate::storage::datastores::RedbDataStore;
        use crate::storage::map_data_store::MapDataStore;

        const N: usize = 400;
        const LIMIT: u32 = 10;
        const RESIDENT_BOUND: usize = N / 4;

        let dir = tempfile::tempdir().expect("tempdir");
        let data_store =
            Arc::new(RedbDataStore::new(dir.path().join("bounded.redb")).expect("redb open"));
        let factory = Arc::new(RecordStoreFactory::new(
            StorageConfig::default(),
            data_store.clone(),
            Vec::new(),
        ));
        let map_name = "bounded";

        // Durable-but-non-resident records, written straight to the datastore.
        for i in 0..N {
            let value = RecordValue::Lww {
                value: Value::String(format!("v{i:04}")),
                timestamp: make_timestamp(),
                ttl_ms: None,
            };
            data_store
                .add(map_name, &format!("k{i:04}"), &value, 1000)
                .await
                .expect("durable write");
        }

        let conn_registry = Arc::new(ConnectionRegistry::new());
        let config = test_config();
        let (handle, mut rx) = conn_registry.register(ConnectionKind::Client, &config);
        let conn_id = handle.id;

        let svc = Arc::new(QueryService::new(
            Arc::new(QueryRegistry::new()),
            factory.clone(),
            conn_registry,
            10_000,
            None,
            #[cfg(feature = "datafusion")]
            None,
        ));

        // LIMIT without sort/filter/cursor → the planner pushes the limit into the
        // scan (ascending key order).
        let query = Query {
            limit: Some(LIMIT),
            ..Query::default()
        };
        let ctx = make_ctx(Some(conn_id));
        let payload = QuerySubMessage {
            payload: QuerySubPayload {
                query_id: "bounded-1".to_string(),
                map_name: map_name.to_string(),
                query,
                fields: None,
                snapshot_only: false,
            },
        };
        let op = Operation::QuerySubscribe { ctx, payload };
        let resp = live_snapshot(&svc.oneshot(op).await.unwrap(), &mut rx);

        // (1) PAGE CORRECTNESS — exactly LIMIT rows, the first LIMIT keys ascending.
        let keys: Vec<String> = resp.payload.results.iter().map(|e| e.key.clone()).collect();
        let expected: Vec<String> = (0..LIMIT as usize).map(|i| format!("k{i:04}")).collect();
        assert_eq!(
            keys, expected,
            "bounded page must be the first LIMIT keys ascending"
        );
        assert_eq!(
            resp.payload.has_more,
            Some(true),
            "more pages exist beyond LIMIT"
        );

        // (2) BOUNDEDNESS — returning the page did NOT hydrate the whole map; engine
        // residency stays at one batch, never the full durable set.
        let resident: usize = factory.all_stores().iter().map(|s| s.size()).sum();
        assert!(
            resident <= RESIDENT_BOUND,
            "bounded top-N page made {resident} of {N} records resident — the page must \
             stream from the datastore through the bounded heap (peak residency <= {RESIDENT_BOUND})."
        );
    }

    /// The tests-only linear-engine opt-out (`with_linear_engine_for_tests`) routes
    /// the handler through the predicate engine instead of the DAG, still returning
    /// real keys and clean values for a simple filter query.
    #[tokio::test]
    async fn query_subscribe_linear_engine_opt_out_filters() {
        use crate::storage::record_store::CallerProvenance;

        let registry = Arc::new(QueryRegistry::new());
        let factory = make_factory();
        let map_name = "people";

        for (key, age) in [("p-young", 15i64), ("p-old", 40)] {
            let partition_id = topgun_core::hash_to_partition(key);
            let value = make_value_map(vec![("age", Value::Int(age))]);
            factory
                .get_or_create(map_name, partition_id)
                .put(
                    &KeyWriteToken::for_test(map_name, key),
                    key,
                    RecordValue::Lww {
                        value,
                        timestamp: make_timestamp(),
                        ttl_ms: None,
                    },
                    CallerProvenance::Client,
                )
                .await
                .expect("put should succeed");
        }

        let conn_registry = Arc::new(ConnectionRegistry::new());
        let config = test_config();
        let (handle, mut rx) = conn_registry.register(ConnectionKind::Client, &config);
        let conn_id = handle.id;

        let svc = Arc::new(
            QueryService::new(
                registry,
                factory,
                conn_registry,
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
                snapshot_only: false,
            },
        };
        let resp = live_snapshot(
            &svc.oneshot(Operation::QuerySubscribe { ctx, payload })
                .await
                .unwrap(),
            &mut rx,
        );

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
                snapshot_only: false,
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

    /// Subscribes `connection` to every record of `users` as `query_id`
    /// through the query service.
    async fn subscribe_all_users(
        svc: &Arc<QueryService>,
        connection: ConnectionId,
        query_id: &str,
    ) {
        let op = Operation::QuerySubscribe {
            ctx: make_ctx(Some(connection)),
            payload: QuerySubMessage {
                payload: QuerySubPayload {
                    query_id: query_id.to_string(),
                    map_name: "users".to_string(),
                    query: Query::default(),
                    fields: None,
                    snapshot_only: false,
                },
            },
        };
        svc.clone().oneshot(op).await.unwrap();
    }

    /// The `(query_id, key)` of every `QUERY_UPDATE` waiting on `rx`.
    fn received_updates(
        rx: &mut tokio::sync::mpsc::Receiver<OutboundMessage>,
    ) -> Vec<(String, String)> {
        let mut updates = Vec::new();
        while let Ok(OutboundMessage::Binary(bytes)) = rx.try_recv() {
            if let Message::QueryUpdate { payload } = rmp_serde::from_slice(&bytes).unwrap() {
                updates.push((payload.query_id, payload.key));
            }
        }
        updates
    }

    type TestConnection = (ConnectionId, tokio::sync::mpsc::Receiver<OutboundMessage>);

    /// Delivers writes of `users` through the production delivery function,
    /// over the registries a test's query service uses.
    struct UsersWriter {
        registry: Arc<QueryRegistry>,
        connections: Arc<ConnectionRegistry>,
    }

    /// A query service over a fresh registry with two client connections, and
    /// a writer of `users` that delivers into that registry.
    fn two_connection_query_service() -> (
        Arc<QueryService>,
        UsersWriter,
        TestConnection,
        TestConnection,
    ) {
        let registry = Arc::new(QueryRegistry::new());
        let conn_registry = Arc::new(ConnectionRegistry::new());
        let (a, rx_a) = conn_registry.register(ConnectionKind::Client, &test_config());
        let (b, rx_b) = conn_registry.register(ConnectionKind::Client, &test_config());
        let svc = Arc::new(QueryService::new(
            registry.clone(),
            make_factory(),
            Arc::clone(&conn_registry),
            10_000,
            None,
            #[cfg(feature = "datafusion")]
            None,
        ));
        let writer = UsersWriter {
            registry,
            connections: conn_registry,
        };
        (svc, writer, (a.id, rx_a), (b.id, rx_b))
    }

    fn query_unsubscribe(connection: ConnectionId, query_id: &str) -> Operation {
        Operation::QueryUnsubscribe {
            ctx: make_ctx(Some(connection)),
            payload: QueryUnsubMessage {
                payload: QueryUnsubPayload {
                    query_id: query_id.to_string(),
                },
            },
        }
    }

    /// An LWW record holding `value`, as a write event carries it.
    fn lww_record(value: rmpv::Value) -> LWWRecord<rmpv::Value> {
        LWWRecord {
            value: Some(value),
            timestamp: make_timestamp(),
            ttl_ms: None,
        }
    }

    fn write_user(writer: &UsersWriter, key: &str) {
        let record = lww_record(rmpv::Value::Map(vec![(
            rmpv::Value::from("n"),
            rmpv::Value::from(1),
        )]));
        writer
            .registry
            .deliver_mutation(&writer.connections, "users", key, Some(&record), None, 0);
    }

    fn update(query_id: &str, key: &str) -> Vec<(String, String)> {
        vec![(query_id.to_string(), key.to_string())]
    }

    /// An unsubscribe of an id the caller does not hold leaves the holder's
    /// subscription delivering.
    #[tokio::test]
    async fn a_connection_cannot_remove_another_connections_query_subscription() {
        let (svc, writer, (a, mut rx_a), (b, mut rx_b)) = two_connection_query_service();
        subscribe_all_users(&svc, a, "q-a").await;

        svc.clone()
            .oneshot(query_unsubscribe(b, "q-a"))
            .await
            .unwrap();
        write_user(&writer, "k1");

        assert_eq!(received_updates(&mut rx_a), update("q-a", "k1"));
        assert!(received_updates(&mut rx_b).is_empty());
    }

    /// Two connections that choose the same query id each keep their own
    /// subscription: both are delivered a matching write, and one's
    /// unsubscribe stops only its own delivery.
    #[tokio::test]
    async fn connections_choosing_the_same_query_id_keep_separate_subscriptions() {
        let (svc, writer, (a, mut rx_a), (b, mut rx_b)) = two_connection_query_service();
        subscribe_all_users(&svc, a, "shared").await;
        subscribe_all_users(&svc, b, "shared").await;

        write_user(&writer, "k1");
        assert_eq!(received_updates(&mut rx_a), update("shared", "k1"));
        assert_eq!(received_updates(&mut rx_b), update("shared", "k1"));

        svc.clone()
            .oneshot(query_unsubscribe(a, "shared"))
            .await
            .unwrap();
        write_user(&writer, "k2");
        assert!(received_updates(&mut rx_a).is_empty());
        assert_eq!(received_updates(&mut rx_b), update("shared", "k2"));
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
            delta_buffer: inactive_buffer(),
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
            delta_buffer: inactive_buffer(),
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
            delta_buffer: inactive_buffer(),
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
            delta_buffer: inactive_buffer(),
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

    /// Verify `QueryRegistry::get_subscription` lookup by map, connection and query_id.
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
            delta_buffer: inactive_buffer(),
        });

        let sub = registry.get_subscription("users", ConnectionId(1), "q-lookup");
        assert!(sub.is_some());
        let sub = sub.unwrap();
        assert_eq!(sub.query_id, "q-lookup");
        assert_eq!(sub.map_name, "users");
        assert_eq!(sub.fields, Some(vec!["name".to_string()]));

        // Another connection's id, another map, or an unknown id returns None
        assert!(registry
            .get_subscription("users", ConnectionId(2), "q-lookup")
            .is_none());
        assert!(registry
            .get_subscription("orders", ConnectionId(1), "q-lookup")
            .is_none());
        assert!(registry
            .get_subscription("users", ConnectionId(1), "nonexistent")
            .is_none());
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
    // Live aggregate refusal and snapshot-only subscriptions
    // ---------------------------------------------------------------------------

    /// Seeds `orders` with two `a` rows (price 10, 25) and one `b` row (price 50).
    async fn seed_orders(factory: &Arc<RecordStoreFactory>) {
        use crate::storage::record_store::CallerProvenance;
        for (key, category, price) in [("o1", "a", 10i64), ("o2", "a", 25), ("o3", "b", 50)] {
            let value = make_value_map(vec![
                ("category", Value::String(category.to_string())),
                ("price", Value::Int(price)),
            ]);
            factory
                .get_or_create("orders", topgun_core::hash_to_partition(key))
                .put(
                    &KeyWriteToken::for_test("orders", key),
                    key,
                    RecordValue::Lww {
                        value,
                        timestamp: make_timestamp(),
                        ttl_ms: None,
                    },
                    CallerProvenance::Client,
                )
                .await
                .expect("put should succeed");
        }
    }

    fn category_is(category: &str) -> Query {
        Query {
            predicate: Some(PredicateNode {
                op: PredicateOp::Eq,
                attribute: Some("category".to_string()),
                value: Some(rmpv::Value::String(category.into())),
                ..Default::default()
            }),
            ..Query::default()
        }
    }

    fn group_by_category() -> Query {
        Query {
            group_by: Some(vec!["category".to_string()]),
            ..Query::default()
        }
    }

    struct AggFixture {
        registry: Arc<QueryRegistry>,
        conn_registry: Arc<ConnectionRegistry>,
        svc: Arc<QueryService>,
        conn_id: ConnectionId,
        rx: tokio::sync::mpsc::Receiver<OutboundMessage>,
    }

    async fn agg_fixture() -> AggFixture {
        let registry = Arc::new(QueryRegistry::new());
        let factory = make_factory();
        seed_orders(&factory).await;
        let conn_registry = Arc::new(ConnectionRegistry::new());
        let (handle, rx) = conn_registry.register(ConnectionKind::Client, &test_config());
        let svc = Arc::new(QueryService::new(
            registry.clone(),
            factory,
            conn_registry.clone(),
            10_000,
            None,
            #[cfg(feature = "datafusion")]
            None,
        ));
        AggFixture {
            registry,
            conn_registry,
            svc,
            conn_id: handle.id,
            rx,
        }
    }

    /// Subscribes `fx`'s connection to `orders`. A live subscribe's snapshot is
    /// read from the connection; a one-shot or refused request is answered on
    /// the response path and enqueues no frame.
    async fn subscribe_orders(
        fx: &mut AggFixture,
        query_id: &str,
        query: Query,
        snapshot_only: bool,
    ) -> QueryRespPayload {
        let op = Operation::QuerySubscribe {
            ctx: make_ctx(Some(fx.conn_id)),
            payload: QuerySubMessage {
                payload: QuerySubPayload {
                    query_id: query_id.to_string(),
                    map_name: "orders".to_string(),
                    query,
                    fields: None,
                    snapshot_only,
                },
            },
        };
        match fx.svc.clone().oneshot(op).await.unwrap() {
            OperationResponse::Message(msg) => {
                assert!(
                    fx.rx.try_recv().is_err(),
                    "a one-shot or refused request enqueues no frame"
                );
                match *msg {
                    Message::QueryResp(resp) => resp.payload,
                    other => panic!("expected QueryResp, got {other:?}"),
                }
            }
            live => live_snapshot(&live, &mut fx.rx).payload,
        }
    }

    fn assert_live_aggregate_refused(resp: &QueryRespPayload) {
        assert!(resp.results.is_empty(), "a refusal carries no rows");
        assert_eq!(resp.code.as_deref(), Some(QUERY_LIVE_AGGREGATE_UNSUPPORTED));
        assert!(resp.error.is_some(), "a refusal names its reason");
    }

    /// Delivers a matching `orders` write through the production delivery
    /// function and asserts that no frame reaches the connection.
    fn assert_write_sends_nothing(fx: &mut AggFixture) {
        let record = lww_record(rmpv::Value::Map(vec![
            (rmpv::Value::from("category"), rmpv::Value::from("a")),
            (rmpv::Value::from("price"), rmpv::Value::from(5)),
        ]));
        fx.registry
            .deliver_mutation(&fx.conn_registry, "orders", "o4", Some(&record), None, 0);
        assert!(fx.rx.try_recv().is_err(), "no frame for a refused query");
    }

    /// `__count` of the group row whose `category` is `group`.
    fn group_count(resp: &QueryRespPayload, group: &str) -> i64 {
        let field = |pairs: &[(rmpv::Value, rmpv::Value)], name: &str| {
            pairs
                .iter()
                .find(|(k, _)| k.as_str() == Some(name))
                .map(|(_, v)| v.clone())
        };
        resp.results
            .iter()
            .find_map(|e| {
                let rmpv::Value::Map(pairs) = &e.value else {
                    return None;
                };
                (field(pairs, "category")?.as_str() == Some(group))
                    .then(|| field(pairs, "__count")?.as_i64())
                    .flatten()
            })
            .unwrap_or_else(|| panic!("group {group} missing: {:?}", resp.results))
    }

    #[tokio::test]
    async fn live_group_by_subscription_is_refused_and_registers_nothing() {
        let mut fx = agg_fixture().await;
        let resp = subscribe_orders(&mut fx, "g-1", group_by_category(), false).await;
        assert_live_aggregate_refused(&resp);
        assert_eq!(fx.registry.subscription_count(), 0);
        assert_write_sends_nothing(&mut fx);
    }

    #[tokio::test]
    async fn live_aggregations_only_subscription_is_refused() {
        use topgun_core::messages::base::{AggFunc, Aggregation};
        let mut fx = agg_fixture().await;
        let query = Query {
            aggregations: Some(vec![Aggregation {
                func: AggFunc::Sum,
                field: Some("price".to_string()),
            }]),
            ..Query::default()
        };
        let resp = subscribe_orders(&mut fx, "g-2", query, false).await;
        assert_live_aggregate_refused(&resp);
        assert_eq!(fx.registry.subscription_count(), 0);
        assert_write_sends_nothing(&mut fx);
    }

    #[tokio::test]
    async fn empty_group_by_and_aggregations_stay_live() {
        let mut fx = agg_fixture().await;
        let query = Query {
            group_by: Some(vec![]),
            aggregations: Some(vec![]),
            ..Query::default()
        };
        let resp = subscribe_orders(&mut fx, "g-3", query, false).await;
        assert_eq!(resp.code, None);
        assert_eq!(resp.results.len(), 3, "raw records");
        assert_eq!(fx.registry.subscription_count(), 1);
    }

    #[tokio::test]
    async fn snapshot_only_serves_grouped_rows_and_registers_nothing() {
        let mut fx = agg_fixture().await;
        let resp = subscribe_orders(&mut fx, "g-4", group_by_category(), true).await;
        assert_eq!(resp.code, None);
        assert_eq!(resp.results.len(), 2, "two groups: {:?}", resp.results);
        assert_eq!(group_count(&resp, "a"), 2);
        assert_eq!(group_count(&resp, "b"), 1);
        assert_eq!(fx.registry.subscription_count(), 0);

        let resp = subscribe_orders(&mut fx, "g-5", Query::default(), true).await;
        assert_eq!(resp.code, None);
        assert_eq!(resp.results.len(), 3);
        assert_eq!(fx.registry.subscription_count(), 0);
    }

    #[tokio::test]
    async fn refused_live_aggregate_keeps_an_existing_subscription_of_the_same_id() {
        let mut fx = agg_fixture().await;
        let resp = subscribe_orders(&mut fx, "q1", category_is("a"), false).await;
        assert_eq!(resp.code, None);
        assert_eq!(fx.registry.subscription_count(), 1);

        let resp = subscribe_orders(&mut fx, "q1", group_by_category(), false).await;
        assert_live_aggregate_refused(&resp);
        assert_eq!(fx.registry.subscription_count(), 1);
        let sub = fx
            .registry
            .get_subscription("orders", fx.conn_id, "q1")
            .expect("the live subscription stays registered");
        assert_eq!(sub.query, category_is("a"));
    }

    #[tokio::test]
    async fn snapshot_only_leaves_a_live_subscription_of_the_same_id_untouched() {
        let mut fx = agg_fixture().await;
        let live = subscribe_orders(&mut fx, "q1", category_is("a"), false).await;
        let mut live_keys: Vec<&str> = live.results.iter().map(|e| e.key.as_str()).collect();
        live_keys.sort_unstable();
        assert_eq!(live_keys, vec!["o1", "o2"]);
        let before = fx
            .registry
            .get_subscription("orders", fx.conn_id, "q1")
            .expect("registered");

        // Same map, and the snapshot holds `o3`, a key q1 does not hold. The
        // helper asserts that no frame follows the live snapshot for it.
        let resp = subscribe_orders(&mut fx, "q1", category_is("b"), true).await;
        let keys: Vec<&str> = resp.results.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(keys, vec!["o3"], "the snapshot is the new query's");

        assert_eq!(fx.registry.subscription_count(), 1);
        let after = fx
            .registry
            .get_subscription("orders", fx.conn_id, "q1")
            .expect("still registered");
        assert!(Arc::ptr_eq(&before, &after), "the same registration");
        assert_eq!(after.query, category_is("a"));
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

    /// A stored record value: `name` = `key`, and `attr` = the encoded vector.
    fn vector_value(key: &str, attr: &str, data: &[f32]) -> Value {
        make_value_map(vec![
            ("name", Value::String(key.to_string())),
            (
                attr,
                Value::Bytes(rmp_serde::to_vec_named(&Vector::F32(data.to_vec())).unwrap()),
            ),
        ])
    }

    /// Writes `value` under `key` into the store of the key's partition, where
    /// the point read of a vector search looks for it.
    async fn put_lww(
        factory: &RecordStoreFactory,
        map: &str,
        key: &str,
        value: Value,
        timestamp: Timestamp,
        ttl_ms: Option<u64>,
    ) {
        use crate::storage::record_store::CallerProvenance;
        let store = factory.get_or_create(map, topgun_core::hash_to_partition(key));
        store
            .put(
                &KeyWriteToken::for_test(map, key),
                key,
                RecordValue::Lww {
                    value,
                    timestamp,
                    ttl_ms,
                },
                CallerProvenance::Client,
            )
            .await
            .unwrap();
    }

    /// Indexes `key` with `data` and stores its record.
    async fn index_and_store(
        svc: &QueryService,
        vi: &crate::service::domain::index::vector_index::VectorIndex,
        key: &str,
        data: &[f32],
    ) {
        vi.insert(key, &make_vector_record("embedding", data));
        put_lww(
            &svc.record_store_factory,
            "maps",
            key,
            vector_value(key, "embedding", data),
            make_timestamp(),
            None,
        )
        .await;
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
        make_svc_with_vector_index_over(make_factory(), map_name, attr, dim, metric)
    }

    fn make_svc_with_vector_index_over(
        factory: Arc<RecordStoreFactory>,
        map_name: &str,
        attr: &str,
        dim: u16,
        metric: DistanceMetric,
    ) -> (
        Arc<QueryService>,
        Arc<crate::service::domain::index::registry::IndexRegistry>,
    ) {
        let registry = Arc::new(QueryRegistry::new());
        let conn_registry = Arc::new(ConnectionRegistry::new());
        let observer_factory = Arc::new(IndexObserverFactory::new());
        let index_registry = observer_factory.register_map(map_name);
        index_registry.add_vector_index(attr, dim, metric);

        let svc = Arc::new(QueryService::new(
            registry,
            factory,
            conn_registry,
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
            index_and_store(&svc, &vi, &format!("k{i}"), &[0.1 * fi, 0.2 * fi]).await;
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
            index_and_store(&svc, &vi, &format!("k{i}"), &[x, 0.0]).await;
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
        use crate::storage::record_store::CallerProvenance;

        let (svc, index_registry) =
            make_svc_with_vector_index("maps", "embedding", 2, DistanceMetric::Cosine);
        let vi = index_registry.get_vector_index("embedding").unwrap();
        let rec = make_vector_record("embedding", &[1.0, 0.0]);
        vi.insert("k1", &rec);
        vi.commit_pending();

        // Populate RecordStore so include_value (default true) can return a value.
        let store = svc
            .record_store_factory
            .get_or_create("maps", topgun_core::hash_to_partition("k1"));
        store
            .put(
                &KeyWriteToken::for_test(store.name(), "k1"),
                "k1",
                RecordValue::Lww {
                    value: make_value_map(vec![("name", Value::String("Alice".to_string()))]),
                    timestamp: make_timestamp(),
                    ttl_ms: None,
                },
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
        index_and_store(&svc, &vi, "k1", &[1.0, 0.0]).await;
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
            index_and_store(&svc, &vi, &format!("k{i}"), &[f32::from(i), 0.0]).await;
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
        index_and_store(&svc, &vi, "k1", &a).await;
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
        use crate::storage::record_store::CallerProvenance;
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
        let store = svc
            .record_store_factory
            .get_or_create("maps", topgun_core::hash_to_partition("k1"));
        store
            .put(
                &KeyWriteToken::for_test(store.name(), "k1"),
                "k1",
                RecordValue::Lww {
                    value: stored_value,
                    timestamp: make_timestamp(),
                    ttl_ms: None,
                },
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
        index_and_store(&svc, &vi, "k1", &[1.0, 0.0]).await;
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

    /// TG-TTL-001: a resident key that has expired is dropped before the top-k
    /// cut, whether or not values are wanted, so the nearest key, expired,
    /// never takes a slot; with the clock before expiry it is served.
    #[tokio::test]
    async fn vector_search_hides_expired_resident() {
        use crate::storage::expiry::ManualClock;
        use crate::storage::factory::ttl_fixture::{
            expired_now, live_now, live_stamp, ts, wall_stamp, TTL_MS,
        };
        use crate::storage::record_store::CallerProvenance;

        async fn search(stamp: u64, now: u64, include_value: Option<bool>) -> Vec<(String, bool)> {
            let factory = Arc::new(
                RecordStoreFactory::new(
                    StorageConfig::default(),
                    Arc::new(NullDataStore),
                    Vec::new(),
                )
                .with_clock(Arc::new(ManualClock::new(now))),
            );
            let (svc, index_registry) = make_svc_with_vector_index_over(
                Arc::clone(&factory),
                "maps",
                "embedding",
                2,
                DistanceMetric::Cosine,
            );
            let vi = index_registry.get_vector_index("embedding").unwrap();
            for (key, vector, ttl_ms) in [
                ("k1", [1.0f32, 0.0], Some(TTL_MS)),
                ("k2", [0.9, 0.1], None),
                ("k3", [0.5, 0.5], None),
            ] {
                let rec = make_vector_record("embedding", &vector);
                vi.insert(key, &rec);
                let store = factory.get_or_create("maps", topgun_core::hash_to_partition(key));
                store
                    .put(
                        &KeyWriteToken::for_test("maps", key),
                        key,
                        RecordValue::Lww {
                            value: make_value_map(vec![("name", Value::String(key.to_string()))]),
                            timestamp: ts(stamp),
                            ttl_ms,
                        },
                        CallerProvenance::Client,
                    )
                    .await
                    .unwrap();
            }
            vi.commit_pending();
            let payload = VectorSearchPayload {
                id: "vs-ttl".to_string(),
                map_name: "maps".to_string(),
                index_name: Some("embedding".to_string()),
                query_vector: make_query_vec(&[1.0f32, 0.0f32]),
                k: 2,
                ef_search: None,
                options: Some(topgun_core::messages::vector::VectorSearchOptions {
                    include_value,
                    ..Default::default()
                }),
            };
            let op = Operation::VectorSearch {
                ctx: make_ctx(None),
                payload,
            };
            let resp = extract_vector_resp(svc.oneshot(op).await.unwrap());
            assert!(resp.error.is_none());
            resp.results
                .into_iter()
                .map(|r| (r.key, r.value.is_some()))
                .collect()
        }

        let w = wall_stamp();
        for include_value in [Some(false), Some(true)] {
            let wanted = include_value == Some(true);
            assert_eq!(
                search(w, expired_now(), include_value).await,
                vec![("k2".to_string(), wanted), ("k3".to_string(), wanted)],
                "expired, include_value {include_value:?}"
            );
            let stamp = live_stamp();
            assert_eq!(
                search(stamp, live_now(stamp), include_value).await,
                vec![("k1".to_string(), wanted), ("k2".to_string(), wanted)],
                "live, include_value {include_value:?}"
            );
        }
    }

    /// A query service over `store` (a `TestStore`) with an `embedding` index
    /// of dimension 2 on `maps`, and `clock_ms` as the factory clock.
    fn svc_over_test_store(
        store: Arc<crate::storage::factory::test_store::TestStore>,
        clock_ms: Option<u64>,
    ) -> (
        Arc<QueryService>,
        Arc<crate::service::domain::index::vector_index::VectorIndex>,
    ) {
        let mut factory = RecordStoreFactory::new(StorageConfig::default(), store, Vec::new());
        if let Some(now) = clock_ms {
            factory = factory.with_clock(Arc::new(crate::storage::expiry::ManualClock::new(now)));
        }
        let (svc, index_registry) = make_svc_with_vector_index_over(
            Arc::new(factory),
            "maps",
            "embedding",
            2,
            DistanceMetric::Cosine,
        );
        let vi = index_registry.get_vector_index("embedding").unwrap();
        (svc, vi)
    }

    fn stored_row(
        key: &str,
        data: &[f32],
        extra: Vec<(&str, Value)>,
        millis: u64,
        ttl_ms: Option<u64>,
    ) -> RecordValue {
        let Value::Map(mut map) = vector_value(key, "embedding", data) else {
            unreachable!()
        };
        for (k, v) in extra {
            map.insert(k.to_string(), v);
        }
        RecordValue::Lww {
            value: Value::Map(map),
            timestamp: crate::storage::factory::ttl_fixture::ts(millis),
            ttl_ms,
        }
    }

    async fn vector_search(
        svc: &Arc<QueryService>,
        query: &[f32],
        k: u32,
        options: Option<topgun_core::messages::vector::VectorSearchOptions>,
    ) -> topgun_core::messages::vector::VectorSearchRespPayload {
        let payload = VectorSearchPayload {
            id: "vs".to_string(),
            map_name: "maps".to_string(),
            index_name: Some("embedding".to_string()),
            query_vector: make_query_vec(query),
            k,
            ef_search: None,
            options,
        };
        let op = Operation::VectorSearch {
            ctx: make_ctx(None),
            payload,
        };
        extract_vector_resp(Arc::clone(svc).oneshot(op).await.unwrap())
    }

    fn result_keys(resp: &topgun_core::messages::vector::VectorSearchRespPayload) -> Vec<String> {
        resp.results.iter().map(|r| r.key.clone()).collect()
    }

    /// A candidate that is only in the data store is read and returned with
    /// its value; nothing was resident before the search.
    #[tokio::test]
    async fn vector_search_returns_a_non_resident_candidate_with_its_value() {
        use crate::storage::factory::test_store::TestStore;
        let store = Arc::new(TestStore::new(100));
        store.put_row(
            "maps",
            "k1",
            stored_row("k1", &[1.0, 0.0], vec![], 1_000, None),
        );
        let (svc, vi) = svc_over_test_store(Arc::clone(&store), None);
        vi.insert("k1", &make_vector_record("embedding", &[1.0, 0.0]));

        let resp = vector_search(&svc, &[1.0, 0.0], 1, None).await;
        assert!(resp.error.is_none());
        assert_eq!(result_keys(&resp), vec!["k1"]);
        let value = resp.results[0]
            .value
            .as_ref()
            .expect("value of a stored record");
        let name = value
            .as_map()
            .and_then(|m| m.iter().find(|(k, _)| k.as_str() == Some("name")))
            .map(|(_, v)| v.clone());
        assert_eq!(name, Some(rmpv::Value::from("k1")));
    }

    /// TG-TTL-001: a stored, non-resident candidate whose TTL has passed is not
    /// returned and takes no slot.
    #[tokio::test]
    async fn vector_search_hides_expired_non_resident() {
        use crate::storage::factory::test_store::TestStore;
        use crate::storage::factory::ttl_fixture::{expired_now, wall_stamp, TTL_MS};
        let store = Arc::new(TestStore::new(100));
        let stamp = wall_stamp();
        store.put_row(
            "maps",
            "k1",
            stored_row("k1", &[1.0, 0.0], vec![], stamp, Some(TTL_MS)),
        );
        store.put_row(
            "maps",
            "k2",
            stored_row("k2", &[0.9, 0.1], vec![], stamp, None),
        );
        let (svc, vi) = svc_over_test_store(Arc::clone(&store), Some(expired_now()));
        vi.insert("k1", &make_vector_record("embedding", &[1.0, 0.0]));
        vi.insert("k2", &make_vector_record("embedding", &[0.9, 0.1]));

        let resp = vector_search(&svc, &[1.0, 0.0], 1, None).await;
        assert!(resp.error.is_none());
        assert_eq!(result_keys(&resp), vec!["k2"]);
    }

    /// The `filter` is evaluated on the value read for a non-resident
    /// candidate, not dropped for lack of a resident copy.
    #[tokio::test]
    async fn vector_search_filter_applies_to_non_resident_candidates() {
        use crate::storage::factory::test_store::TestStore;
        let store = Arc::new(TestStore::new(100));
        for (key, data, status) in [
            ("k1", [1.0f32, 0.0], "inactive"),
            ("k2", [0.9, 0.1], "active"),
            ("k3", [0.5, 0.5], "active"),
        ] {
            store.put_row(
                "maps",
                key,
                stored_row(
                    key,
                    &data,
                    vec![("status", Value::String(status.to_string()))],
                    1_000,
                    None,
                ),
            );
        }
        let (svc, vi) = svc_over_test_store(Arc::clone(&store), None);
        for (key, data) in [
            ("k1", [1.0f32, 0.0]),
            ("k2", [0.9, 0.1]),
            ("k3", [0.5, 0.5]),
        ] {
            vi.insert(key, &make_vector_record("embedding", &data));
        }
        let filter = PredicateNode {
            op: PredicateOp::Eq,
            attribute: Some("status".to_string()),
            value: Some(rmpv::Value::from("active")),
            ..PredicateNode::default()
        };
        let resp = vector_search(
            &svc,
            &[1.0, 0.0],
            2,
            Some(topgun_core::messages::vector::VectorSearchOptions {
                filter: Some(filter),
                ..Default::default()
            }),
        )
        .await;
        assert!(resp.error.is_none());
        assert_eq!(result_keys(&resp), vec!["k2", "k3"]);
    }

    /// Only candidate keys are read: no scan of the map, and at most `4k`
    /// point loads.
    #[tokio::test]
    async fn vector_search_reads_only_candidate_keys() {
        use crate::storage::factory::test_store::TestStore;
        let store = Arc::new(TestStore::new(100));
        let (svc, vi) = svc_over_test_store(Arc::clone(&store), None);
        for i in 0u8..50 {
            let key = format!("k{i}");
            let data = [1.0, f32::from(i) / 50.0];
            store.put_row("maps", &key, stored_row(&key, &data, vec![], 1_000, None));
            vi.insert(&key, &make_vector_record("embedding", &data));
        }
        let calls_before = store.calls();

        let resp = vector_search(&svc, &[1.0, 0.0], 2, None).await;
        assert!(resp.error.is_none());
        assert_eq!(resp.results.len(), 2);
        assert_eq!(store.scans("maps"), 0, "the map must not be scanned");
        let loads = store.calls() - calls_before;
        assert!(loads <= 8, "at most 4k point loads, got {loads}");
    }

    /// The top-scored candidate has expired: it is skipped before the cut, so
    /// `k` visible results are still returned.
    #[tokio::test]
    async fn vector_search_skips_invisible_candidates_before_the_k_cut() {
        use crate::storage::expiry::ManualClock;
        use crate::storage::factory::ttl_fixture::{expired_now, ts, wall_stamp, TTL_MS};
        let factory = Arc::new(
            RecordStoreFactory::new(
                StorageConfig::default(),
                Arc::new(NullDataStore),
                Vec::new(),
            )
            .with_clock(Arc::new(ManualClock::new(expired_now()))),
        );
        let (svc, index_registry) = make_svc_with_vector_index_over(
            Arc::clone(&factory),
            "maps",
            "embedding",
            2,
            DistanceMetric::Cosine,
        );
        let vi = index_registry.get_vector_index("embedding").unwrap();
        let stamp = wall_stamp();
        for (key, data, ttl_ms) in [
            ("k1", [1.0f32, 0.0], Some(TTL_MS)),
            ("k2", [0.9, 0.1], None),
            ("k3", [0.5, 0.5], None),
            ("k4", [0.0, 1.0], None),
        ] {
            vi.insert(key, &make_vector_record("embedding", &data));
            put_lww(
                &factory,
                "maps",
                key,
                vector_value(key, "embedding", &data),
                ts(stamp),
                ttl_ms,
            )
            .await;
        }

        let resp = vector_search(&svc, &[1.0, 0.0], 2, None).await;
        assert!(resp.error.is_none());
        assert_eq!(result_keys(&resp), vec!["k2", "k3"]);
    }

    /// Without an index registry the search is answered with an error, not
    /// dropped as a failed dispatch that leaves the client to time out.
    #[tokio::test]
    async fn vector_search_without_factory_answers_with_an_error() {
        let svc = Arc::new(QueryService::new(
            Arc::new(QueryRegistry::new()),
            make_factory(),
            Arc::new(ConnectionRegistry::new()),
            10_000,
            None,
            #[cfg(feature = "datafusion")]
            None,
        ));
        let resp = vector_search(&svc, &[1.0, 0.0], 1, None).await;
        assert_eq!(
            resp.error.as_deref(),
            Some("vector search is not available on this server: no index registry is configured")
        );
        assert!(resp.results.is_empty());
    }

    /// Sort by `age` desc, then `status` asc.
    fn by_age() -> Vec<topgun_core::messages::base::SortField> {
        use topgun_core::messages::base::{SortDirection, SortField};
        vec![
            SortField {
                field: "age".to_string(),
                direction: SortDirection::Desc,
            },
            SortField {
                field: "status".to_string(),
                direction: SortDirection::Asc,
            },
        ]
    }

    /// eq, neq, range, `and`/`or`/`not`, and sort + limit.
    fn scalar_equivalence_queries() -> Vec<Query> {
        let leaf = |op: PredicateOp, attribute: &str, value: rmpv::Value| PredicateNode {
            op,
            attribute: Some(attribute.to_string()),
            value: Some(value),
            ..PredicateNode::default()
        };
        let node = |op: PredicateOp, children: Vec<PredicateNode>| PredicateNode {
            op,
            children: Some(children),
            ..PredicateNode::default()
        };
        vec![
            Query {
                predicate: Some(leaf(PredicateOp::Eq, "status", rmpv::Value::from("active"))),
                ..Query::default()
            },
            Query {
                predicate: Some(leaf(
                    PredicateOp::Neq,
                    "status",
                    rmpv::Value::from("active"),
                )),
                ..Query::default()
            },
            Query {
                predicate: Some(leaf(PredicateOp::Gte, "age", rmpv::Value::from(35))),
                ..Query::default()
            },
            Query {
                predicate: Some(node(
                    PredicateOp::And,
                    vec![
                        leaf(PredicateOp::Eq, "status", rmpv::Value::from("idle")),
                        node(
                            PredicateOp::Or,
                            vec![
                                leaf(PredicateOp::Lt, "age", rmpv::Value::from(30)),
                                node(
                                    PredicateOp::Not,
                                    vec![leaf(PredicateOp::Lte, "age", rmpv::Value::from(45))],
                                ),
                            ],
                        ),
                    ],
                )),
                ..Query::default()
            },
            Query {
                sort: Some(by_age()),
                limit: Some(5),
                ..Query::default()
            },
        ]
    }

    async fn run_query_page(
        svc: &Arc<QueryService>,
        conn: ConnectionId,
        rx: &mut tokio::sync::mpsc::Receiver<OutboundMessage>,
        query: Query,
    ) -> (Vec<(String, rmpv::Value)>, Option<bool>, Option<String>) {
        let payload = QuerySubMessage {
            payload: QuerySubPayload {
                query_id: "q".to_string(),
                map_name: "users".to_string(),
                query,
                fields: None,
                snapshot_only: false,
            },
        };
        let op = Operation::QuerySubscribe {
            ctx: make_ctx(Some(conn)),
            payload,
        };
        let resp = live_snapshot(&Arc::clone(svc).oneshot(op).await.unwrap(), rx);
        let mut results: Vec<(String, rmpv::Value)> = resp
            .payload
            .results
            .into_iter()
            .map(|e| (e.key, e.value))
            .collect();
        if resp.payload.next_cursor.is_none() && resp.payload.has_more != Some(true) {
            // An unsorted, unbounded result has no defined order.
            results.sort_by(|a, b| a.0.cmp(&b.0));
        }
        (results, resp.payload.has_more, resp.payload.next_cursor)
    }

    /// Regression guard: the production DAG path gives the same scalar query
    /// results with and without the index factory (it does not read it).
    #[tokio::test]
    async fn scalar_query_results_equal_with_and_without_index_factory() {
        use crate::storage::factory::ObserverFactory;

        let index_factory = Arc::new(IndexObserverFactory::new());
        let index_registry = index_factory.register_map("users");
        index_registry.add_hash_index("status");
        index_registry.add_navigable_index("age");
        index_registry.add_inverted_index("bio");
        let factory = Arc::new(
            RecordStoreFactory::new(
                StorageConfig::default(),
                Arc::new(NullDataStore),
                Vec::new(),
            )
            .with_observer_factories(vec![Arc::clone(&index_factory) as Arc<dyn ObserverFactory>]),
        );
        for i in 0..20i64 {
            let key = format!("u{i:02}");
            let value = make_value_map(vec![
                (
                    "status",
                    Value::String(if i % 3 == 0 { "active" } else { "idle" }.to_string()),
                ),
                ("age", Value::Int(20 + (i * 7) % 30)),
                (
                    "bio",
                    Value::String(format!("likes rust and item{}", i % 4)),
                ),
            ]);
            put_lww(&factory, "users", &key, value, make_timestamp(), None).await;
        }
        assert!(
            index_registry.get_index("status").unwrap().entry_count() > 0,
            "the observer populates the indexes"
        );

        let conn_registry = Arc::new(ConnectionRegistry::new());
        let (handle, mut rx) = conn_registry.register(ConnectionKind::Client, &test_config());
        let make = |with: bool| {
            Arc::new(QueryService::new(
                Arc::new(QueryRegistry::new()),
                Arc::clone(&factory),
                Arc::clone(&conn_registry),
                10_000,
                with.then(|| Arc::clone(&index_factory)),
                #[cfg(feature = "datafusion")]
                None,
            ))
        };
        let (plain, indexed) = (make(false), make(true));

        for query in scalar_equivalence_queries() {
            let a = run_query_page(&plain, handle.id, &mut rx, query.clone()).await;
            let b = run_query_page(&indexed, handle.id, &mut rx, query.clone()).await;
            assert_eq!(a.0, b.0, "results differ for {query:?}");
            assert_eq!(a.1, b.1, "has_more differs for {query:?}");
            assert!(!a.0.is_empty() || query.predicate.is_some());
        }

        // Cursor page: the second page after a sorted, limited first page.
        let first = Query {
            sort: Some(by_age()),
            limit: Some(4),
            ..Query::default()
        };
        let (_, _, cursor) = run_query_page(&plain, handle.id, &mut rx, first.clone()).await;
        let cursor = cursor.expect("a limited first page carries a cursor");
        let second = Query {
            cursor: Some(cursor),
            ..first
        };
        let a = run_query_page(&plain, handle.id, &mut rx, second.clone()).await;
        let b = run_query_page(&indexed, handle.id, &mut rx, second).await;
        assert_eq!(a.0, b.0, "second page differs");
        assert_eq!(a.1, b.1);
        assert!(!a.0.is_empty());
    }

    // ---------------------------------------------------------------------------
    // Live subscribe snapshot (TG-LIVE-001): writes through `CrdtService` over the
    // registries the query service uses, frames read from the connections.
    // ---------------------------------------------------------------------------

    use crate::service::domain::crdt::CrdtService;
    use crate::service::domain::schema::SchemaService;
    use crate::service::security::{SecurityConfig, WriteAdmission};
    use crate::storage::expiry::ManualClock;
    use metrics_exporter_prometheus::PrometheusBuilder;
    use topgun_core::messages::base::ClientOp;
    use topgun_core::messages::ClientOpMessage;
    use topgun_core::ORMapRecord;

    const LIVE_MAP: &str = "live";
    const LIVE_ID: &str = "q-live";

    type Rx = tokio::sync::mpsc::Receiver<OutboundMessage>;

    /// A query service and a CRDT service over one query registry and one
    /// connection registry, with a subscriber `s` and a writer `w`.
    struct Live {
        svc: Arc<QueryService>,
        crdt: Arc<CrdtService>,
        registry: Arc<QueryRegistry>,
        conns: Arc<ConnectionRegistry>,
        factory: Arc<RecordStoreFactory>,
        s: ConnectionId,
        s_rx: Rx,
        w: ConnectionId,
        _w_rx: Rx,
    }

    fn live_over(factory: Arc<RecordStoreFactory>, s_capacity: usize) -> Live {
        let registry = Arc::new(QueryRegistry::new());
        let conns = Arc::new(ConnectionRegistry::new());
        let s_config = ConnectionConfig {
            outbound_channel_capacity: s_capacity,
            ..test_config()
        };
        let (s, s_rx) = conns.register(ConnectionKind::Client, &s_config);
        let (w, w_rx) = conns.register(ConnectionKind::Client, &test_config());
        let svc = Arc::new(QueryService::new(
            Arc::clone(&registry),
            Arc::clone(&factory),
            Arc::clone(&conns),
            10_000,
            None,
            #[cfg(feature = "datafusion")]
            None,
        ));
        let hlc = Arc::new(parking_lot::Mutex::new(topgun_core::HLC::new(
            "test-node".to_string(),
            Box::new(topgun_core::SystemClock),
        )));
        let crdt = Arc::new(CrdtService::new(
            Arc::clone(&factory),
            Arc::clone(&conns),
            Arc::new(WriteAdmission::new(
                Arc::new(SecurityConfig::default()),
                hlc,
            )),
            Arc::clone(&registry),
            Arc::new(SchemaService::new()),
        ));
        Live {
            svc,
            crdt,
            registry,
            conns,
            factory,
            s: s.id,
            s_rx,
            w: w.id,
            _w_rx: w_rx,
        }
    }

    fn live() -> Live {
        live_over(make_factory(), test_config().outbound_channel_capacity)
    }

    /// `{"v": n}`.
    fn v(n: i64) -> rmpv::Value {
        rmpv::Value::Map(vec![(rmpv::Value::from("v"), rmpv::Value::from(n))])
    }

    fn put(key: &str, value: rmpv::Value) -> ClientOp {
        ClientOp {
            map_name: LIVE_MAP.to_string(),
            key: key.to_string(),
            record: Some(Some(LWWRecord {
                value: Some(value),
                timestamp: make_timestamp(),
                ttl_ms: None,
            })),
            ..ClientOp::default()
        }
    }

    fn remove(key: &str) -> ClientOp {
        ClientOp {
            map_name: LIVE_MAP.to_string(),
            key: key.to_string(),
            record: Some(None),
            ..ClientOp::default()
        }
    }

    fn or_add(key: &str, value: rmpv::Value) -> ClientOp {
        ClientOp {
            map_name: LIVE_MAP.to_string(),
            key: key.to_string(),
            or_record: Some(Some(ORMapRecord {
                value,
                timestamp: make_timestamp(),
                tag: "client-tag".to_string(),
                ttl_ms: None,
            })),
            ..ClientOp::default()
        }
    }

    fn or_remove(key: &str, tag: String) -> ClientOp {
        ClientOp {
            map_name: LIVE_MAP.to_string(),
            key: key.to_string(),
            or_tag: Some(Some(tag)),
            ..ClientOp::default()
        }
    }

    impl Live {
        /// Applies `op` as a client op of `conn` (its stamp is minted by the
        /// server, and `conn` is the excluded writer).
        async fn client(&self, conn: ConnectionId, op: ClientOp) {
            let mut ctx = OperationContext::new(1, service_names::CRDT, make_timestamp(), 5000);
            ctx.connection_id = Some(conn);
            ctx.partition_id = Some(topgun_core::hash_to_partition(&op.key));
            Arc::clone(&self.crdt)
                .oneshot(Operation::ClientOp {
                    ctx,
                    payload: ClientOpMessage { payload: op },
                })
                .await
                .expect("client op applies");
        }

        /// Applies a trusted LWW put that keeps `stamp` (`StampPolicy::Verbatim`).
        async fn trusted_put(&self, key: &str, value: rmpv::Value, stamp: Timestamp) {
            self.trusted_record(key, value, stamp, None).await;
        }

        /// Applies a trusted LWW put that keeps `stamp` and `ttl_ms`.
        async fn trusted_record(
            &self,
            key: &str,
            value: rmpv::Value,
            stamp: Timestamp,
            ttl_ms: Option<u64>,
        ) {
            let mut ctx = OperationContext::new(1, service_names::CRDT, make_timestamp(), 5000);
            ctx.caller_origin = crate::service::operation::CallerOrigin::System;
            ctx.partition_id = Some(topgun_core::hash_to_partition(key));
            let mut op = put(key, value);
            op.record = Some(Some(LWWRecord {
                value: op.record.flatten().and_then(|r| r.value),
                timestamp: stamp,
                ttl_ms,
            }));
            Arc::clone(&self.crdt)
                .oneshot(Operation::ClientOp {
                    ctx,
                    payload: ClientOpMessage { payload: op },
                })
                .await
                .expect("trusted op applies");
        }

        /// The stored record of `key`.
        async fn stored(&self, key: &str) -> Option<RecordValue> {
            self.factory
                .get_or_create(LIVE_MAP, topgun_core::hash_to_partition(key))
                .get(key, false)
                .await
                .expect("read")
                .map(|r| r.value)
        }

        fn subscribe_op(
            &self,
            query_id: &str,
            map: &str,
            fields: Option<Vec<String>>,
        ) -> Operation {
            Operation::QuerySubscribe {
                ctx: make_ctx(Some(self.s)),
                payload: QuerySubMessage {
                    payload: QuerySubPayload {
                        query_id: query_id.to_string(),
                        map_name: map.to_string(),
                        query: Query::default(),
                        fields,
                        snapshot_only: false,
                    },
                },
            }
        }

        fn spawn_subscribe(
            &self,
            fields: Option<Vec<String>>,
        ) -> tokio::task::JoinHandle<Result<OperationResponse, OperationError>> {
            tokio::spawn(
                Arc::clone(&self.svc).oneshot(self.subscribe_op(LIVE_ID, LIVE_MAP, fields)),
            )
        }

        /// Spawns a live subscribe of `query` on the live map.
        fn spawn_subscribe_query(
            &self,
            query: Query,
        ) -> tokio::task::JoinHandle<Result<OperationResponse, OperationError>> {
            let mut op = self.subscribe_op(LIVE_ID, LIVE_MAP, None);
            if let Operation::QuerySubscribe { payload, .. } = &mut op {
                payload.payload.query = query;
            }
            tokio::spawn(Arc::clone(&self.svc).oneshot(op))
        }

        /// What a fresh one-shot query of `query` answers now.
        async fn fresh(&self, query: Query) -> QueryRespPayload {
            let mut op = self.subscribe_op("fresh", LIVE_MAP, None);
            if let Operation::QuerySubscribe { payload, .. } = &mut op {
                payload.payload.query = query;
                payload.payload.snapshot_only = true;
            }
            let OperationResponse::Message(msg) =
                Arc::clone(&self.svc).oneshot(op).await.expect("one-shot")
            else {
                panic!("a one-shot answers on the response path");
            };
            let Message::QueryResp(resp) = *msg else {
                panic!("a QUERY_RESP");
            };
            resp.payload
        }

        /// The rows a fresh one-shot query of `query` returns now.
        async fn fresh_rows(&self, query: Query) -> BTreeMap<String, rmpv::Value> {
            let results = self.fresh(query).await.results;
            results.into_iter().map(|e| (e.key, e.value)).collect()
        }

        fn buffer(&self) -> Arc<DeltaBuffer> {
            let sub = self
                .registry
                .get_subscription(LIVE_MAP, self.s, LIVE_ID)
                .expect("registered");
            Arc::clone(&sub.delta_buffer)
        }
    }

    /// Every `QUERY_RESP` and `QUERY_UPDATE` of `query_id` waiting on `rx`, in
    /// order (`SERVER_EVENT`s and other queries' frames are skipped).
    fn query_frames(rx: &mut Rx, query_id: &str) -> Vec<Message> {
        let mut frames = Vec::new();
        while let Ok(OutboundMessage::Binary(bytes)) = rx.try_recv() {
            let msg: Message = rmp_serde::from_slice(&bytes).expect("decodes");
            let id = match &msg {
                Message::QueryResp(resp) => resp.payload.query_id.as_str(),
                Message::QueryUpdate { payload } => payload.query_id.as_str(),
                _ => continue,
            };
            if id == query_id {
                frames.push(msg);
            }
        }
        frames
    }

    /// The snapshot rows of the first frame, which must be the `QUERY_RESP`
    /// (no `QUERY_UPDATE` of the query precedes it), and the frames after it.
    fn snapshot_first(frames: Vec<Message>) -> (BTreeMap<String, rmpv::Value>, Vec<Message>) {
        let mut frames = frames.into_iter();
        match frames.next() {
            Some(Message::QueryResp(resp)) => {
                assert_eq!(resp.payload.code, None, "a delivered snapshot");
                let rows = resp
                    .payload
                    .results
                    .into_iter()
                    .map(|e| (e.key, e.value))
                    .collect();
                (rows, frames.collect())
            }
            other => panic!("the first frame of the query is not its snapshot: {other:?}"),
        }
    }

    /// Releases the held subscribe and returns its response.
    async fn release(
        mut hold: DrainHoldHandle,
        task: tokio::task::JoinHandle<Result<OperationResponse, OperationError>>,
    ) -> OperationResponse {
        hold.release();
        task.await.expect("join").expect("subscribe")
    }

    fn stored_value(record: Option<RecordValue>) -> rmpv::Value {
        match record {
            Some(RecordValue::Lww { value, .. }) => value_to_rmpv(&value),
            other => panic!("expected an LWW record, got {other:?}"),
        }
    }

    /// A write made during the scan is in the snapshot, whatever its stamp, and
    /// the last arrival of a key is folded; no update precedes the snapshot, and
    /// a later write arrives as an update after it.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_write_during_the_scan_is_in_the_snapshot_and_no_update_precedes_it() {
        let mut fx = live();
        fx.client(fx.w, put("k1", v(1))).await;
        fx.client(fx.w, put("k2", v(1))).await;
        let hold = fx.svc.test_hold_after_scan(LIVE_ID);
        let task = fx.spawn_subscribe(None);
        let mut hold = hold;
        hold.wait_reached().await;

        fx.client(fx.w, put("k1", v(2))).await;
        fx.client(fx.w, put("k3", v(1))).await;
        let wall = make_timestamp().millis;
        let high = Timestamp {
            millis: wall + 10_000_000_000_000,
            counter: 0,
            node_id: "peer".to_string(),
        };
        let low = Timestamp {
            millis: 1,
            counter: 0,
            node_id: "peer".to_string(),
        };
        fx.trusted_put("k2", v(20), high).await;
        fx.trusted_put("k2", v(21), low).await;
        let k2_stored = stored_value(fx.stored("k2").await);

        let returned = release(hold, task).await;
        assert!(matches!(returned, OperationResponse::Empty));
        let (rows, after) = snapshot_first(query_frames(&mut fx.s_rx, LIVE_ID));
        assert!(after.is_empty(), "nothing follows yet: {after:?}");
        assert_eq!(rows.get("k1"), Some(&v(2)));
        assert_eq!(rows.get("k3"), Some(&v(1)));
        assert_eq!(rows.get("k2"), Some(&k2_stored), "k2 is the store's value");

        fx.client(fx.w, put("k1", v(3))).await;
        let frames = query_frames(&mut fx.s_rx, LIVE_ID);
        assert!(
            matches!(&frames[..], [Message::QueryUpdate { payload }] if payload.key == "k1" && payload.value == v(3)),
            "a later write is a live update: {frames:?}"
        );
    }

    /// A folded ENTER carries only the projected fields.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_fold_projects_a_projected_subscription() {
        let mut fx = live();
        let mut hold = fx.svc.test_hold_after_scan(LIVE_ID);
        let task = fx.spawn_subscribe(Some(vec!["v".to_string()]));
        hold.wait_reached().await;
        let full = rmpv::Value::Map(vec![
            (rmpv::Value::from("v"), rmpv::Value::from(7)),
            (rmpv::Value::from("secret"), rmpv::Value::from("x")),
        ]);
        fx.client(fx.w, put("kp", full)).await;
        release(hold, task).await;
        let (rows, _) = snapshot_first(query_frames(&mut fx.s_rx, LIVE_ID));
        assert_eq!(rows.get("kp"), Some(&v(7)), "only the projected field");
    }

    /// An overflow answers with the scan snapshot, then the overflow frame, on
    /// the response path; the subscription is removed, a subscription of the
    /// same id on another map stays, and a resubscribe holds what a fresh
    /// one-shot scan holds. It is not compared with the store: overflowing
    /// needs more distinct keys than the local DAG scan returns rows, and
    /// that scan stops at 4096 rows (TODO-1001).
    #[test]
    fn an_overflow_sends_the_snapshot_then_the_overflow_and_drops_the_subscription() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let recorder = PrometheusBuilder::new().build_recorder();
        let render = recorder.handle();
        metrics::with_local_recorder(&recorder, || {
            rt.block_on(async {
                // Each write also sends S a SERVER_EVENT: 8192 slots keep the
                // slow-consumer threshold from cancelling it.
                let mut fx = live_over(make_factory(), 8192);
                for i in 0..3 {
                    fx.client(fx.w, put(&format!("seed-{i}"), v(i))).await;
                }
                Arc::clone(&fx.svc)
                    .oneshot(fx.subscribe_op(LIVE_ID, "other", None))
                    .await
                    .expect("other map");
                let _ = query_frames(&mut fx.s_rx, LIVE_ID);

                let mut hold = fx.svc.test_hold_after_scan(LIVE_ID);
                let task = fx.spawn_subscribe(None);
                hold.wait_reached().await;
                for i in 0..=4096 {
                    fx.client(fx.w, put(&format!("w-{i}"), v(i))).await;
                }
                let returned = release(hold, task).await;

                let OperationResponse::Messages(frames) = returned else {
                    panic!("expected [snapshot, overflow], got {returned:?}");
                };
                let [Message::QueryResp(snapshot), Message::QueryResp(overflow)] = &frames[..]
                else {
                    panic!("expected [snapshot, overflow], got {frames:?}");
                };
                assert_eq!(snapshot.payload.code, None);
                let mut keys: Vec<&str> = snapshot
                    .payload
                    .results
                    .iter()
                    .map(|e| e.key.as_str())
                    .collect();
                keys.sort_unstable();
                assert_eq!(keys, vec!["seed-0", "seed-1", "seed-2"], "the scan rows");
                assert_eq!(overflow.payload.query_id, LIVE_ID);
                assert_eq!(
                    overflow.payload.code.as_deref(),
                    Some(QUERY_SNAPSHOT_OVERFLOW)
                );
                assert!(overflow
                    .payload
                    .error
                    .as_ref()
                    .is_some_and(|e| e.starts_with("Too many")));
                assert!(overflow.payload.results.is_empty());

                assert!(
                    query_frames(&mut fx.s_rx, LIVE_ID).is_empty(),
                    "nothing of the query was enqueued"
                );
                assert!(fx
                    .registry
                    .get_subscription(LIVE_MAP, fx.s, LIVE_ID)
                    .is_none());
                assert!(
                    fx.registry
                        .get_subscription("other", fx.s, LIVE_ID)
                        .is_some(),
                    "the same id on another map stays"
                );
                fx.client(fx.w, put("late", v(1))).await;
                assert!(query_frames(&mut fx.s_rx, LIVE_ID).is_empty());

                let again = Arc::clone(&fx.svc)
                    .oneshot(fx.subscribe_op(LIVE_ID, LIVE_MAP, None))
                    .await
                    .expect("resubscribe");
                assert!(matches!(again, OperationResponse::Empty));
                let (rows, _) = snapshot_first(query_frames(&mut fx.s_rx, LIVE_ID));
                let fresh = fx.fresh_rows(Query::default()).await;
                assert_eq!(rows, fresh, "the resubscribe holds what a fresh scan holds");
                assert_eq!(fx.registry.get_subscriptions_for_map(LIVE_MAP).len(), 1);
            });
        });
        assert_eq!(
            rendered_counter(
                &render.render(),
                "topgun_query_snapshot_overflow_total{reason=\"buffer\"}"
            ),
            1
        );
    }

    /// The value of the counter line `series` (name and labels) in a render.
    fn rendered_counter(rendered: &str, series: &str) -> u64 {
        rendered
            .lines()
            .find_map(|line| line.strip_prefix(series)?.strip_prefix(' '))
            .unwrap_or_else(|| panic!("{series} is absent from the render:\n{rendered}"))
            .trim()
            .parse()
            .expect("an integer")
    }

    /// A full outbound channel answers with the snapshot and the overflow on
    /// the response path and removes the registration.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_full_outbound_channel_answers_with_the_snapshot_and_the_overflow() {
        let fx = live_over(make_factory(), 2);
        let handle = fx.conns.get(fx.s).expect("s");
        assert!(handle.try_send(OutboundMessage::Binary(vec![1])));
        assert!(handle.try_send(OutboundMessage::Binary(vec![2])));
        let returned = Arc::clone(&fx.svc)
            .oneshot(fx.subscribe_op(LIVE_ID, LIVE_MAP, None))
            .await
            .expect("subscribe");
        let OperationResponse::Messages(frames) = returned else {
            panic!("expected [snapshot, overflow], got {returned:?}");
        };
        assert!(
            matches!(&frames[..], [Message::QueryResp(s), Message::QueryResp(o)]
            if s.payload.code.is_none()
                && o.payload.code.as_deref() == Some(QUERY_SNAPSHOT_OVERFLOW)
                && o.payload.error.as_ref().is_some_and(|e| e.starts_with("The connection's outbound queue was full")))
        );
        assert_eq!(fx.registry.subscription_count(), 0);
    }

    /// A disconnect during the scan frees the buffer and leaves no registration.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_disconnect_during_the_scan_frees_the_buffer() {
        let fx = live();
        let mut hold = fx.svc.test_hold_after_scan(LIVE_ID);
        let task = fx.spawn_subscribe(None);
        hold.wait_reached().await;
        let weak = Arc::downgrade(&fx.buffer());
        let Live {
            svc: _svc,
            registry,
            conns,
            s,
            s_rx,
            ..
        } = fx;
        drop(s_rx);
        conns.remove(s);
        registry.unregister_by_connection(s);
        let returned = release(hold, task).await;
        assert!(matches!(returned, OperationResponse::Empty));
        assert!(weak.upgrade().is_none(), "the buffer is freed");
        assert_eq!(registry.subscription_count(), 0);
        assert!(!registry.subscriptions.contains_key(LIVE_MAP));
    }

    /// An unsubscribe during the scan, then a resubscribe of the same id that
    /// completes before the scan does: the held subscribe sends nothing, the
    /// resubscribe's registration stays, and the held buffer is freed.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_unsubscribe_during_the_scan_sends_nothing_and_frees_the_buffer() {
        let mut fx = live();
        let mut hold = fx.svc.test_hold_after_scan(LIVE_ID);
        let task = fx.spawn_subscribe(None);
        hold.wait_reached().await;
        let weak = Arc::downgrade(&fx.buffer());
        Arc::clone(&fx.svc)
            .oneshot(query_unsubscribe(fx.s, LIVE_ID))
            .await
            .expect("unsubscribe");
        let again = Arc::clone(&fx.svc)
            .oneshot(fx.subscribe_op(LIVE_ID, LIVE_MAP, None))
            .await
            .expect("resubscribe");
        assert!(matches!(again, OperationResponse::Empty));
        assert_eq!(query_frames(&mut fx.s_rx, LIVE_ID).len(), 1);
        let returned = release(hold, task).await;
        assert!(matches!(returned, OperationResponse::Empty));
        assert!(
            query_frames(&mut fx.s_rx, LIVE_ID).is_empty(),
            "the held subscribe sends no QUERY_RESP"
        );
        assert!(
            fx.registry
                .get_subscription(LIVE_MAP, fx.s, LIVE_ID)
                .is_some(),
            "the resubscribe's registration stays"
        );
        assert!(weak.upgrade().is_none(), "the buffer is freed");
    }

    /// A subscribe cancelled during its scan leaves no registration.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_cancelled_subscribe_leaves_no_registration() {
        let mut fx = live();
        let mut hold = fx.svc.test_hold_after_scan(LIVE_ID);
        let task = fx.spawn_subscribe(None);
        hold.wait_reached().await;
        let weak = Arc::downgrade(&fx.buffer());
        task.abort();
        let joined = task.await;
        assert!(joined.is_err_and(|e| e.is_cancelled()));
        drop(hold);
        assert_eq!(fx.registry.subscription_count(), 0);
        fx.client(fx.w, put("k", v(1))).await;
        assert!(query_frames(&mut fx.s_rx, LIVE_ID).is_empty());
        assert!(weak.upgrade().is_none(), "the buffer is freed");
    }

    /// An LWW remove during the scan leaves the snapshot.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_lww_remove_during_the_scan_leaves_the_snapshot() {
        let mut fx = live();
        fx.client(fx.w, put("ka", v(1))).await;
        let mut hold = fx.svc.test_hold_after_scan(LIVE_ID);
        let task = fx.spawn_subscribe(None);
        hold.wait_reached().await;
        fx.client(fx.w, remove("ka")).await;
        release(hold, task).await;
        let (rows, _) = snapshot_first(query_frames(&mut fx.s_rx, LIVE_ID));
        assert!(!rows.contains_key("ka"), "{rows:?}");
    }

    /// An OR op that turns a scanned LWW key into an OR slot during the scan
    /// leaves the snapshot: an OR slot is never a row.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_or_op_on_a_scanned_lww_key_during_the_scan_leaves_the_snapshot() {
        let mut fx = live();
        fx.client(fx.w, put("kb", v(1))).await;
        let mut hold = fx.svc.test_hold_after_scan(LIVE_ID);
        let task = fx.spawn_subscribe(None);
        hold.wait_reached().await;
        fx.client(fx.w, or_add("kb", v(2))).await;
        release(hold, task).await;
        let (rows, _) = snapshot_first(query_frames(&mut fx.s_rx, LIVE_ID));
        assert!(!rows.contains_key("kb"), "{rows:?}");
    }

    /// OR ops on new keys during the scan add no row and send no update.
    #[tokio::test(flavor = "multi_thread")]
    async fn or_ops_on_a_new_key_during_the_scan_add_no_row() {
        let mut fx = live();
        let mut hold = fx.svc.test_hold_after_scan(LIVE_ID);
        let task = fx.spawn_subscribe(None);
        hold.wait_reached().await;
        fx.client(fx.w, or_add("o1", v(1))).await;
        let tag = match fx.stored("o1").await {
            Some(RecordValue::OrMap { records, .. }) => records[0].tag.clone(),
            other => panic!("expected an OR slot, got {other:?}"),
        };
        fx.client(fx.w, or_remove("o1", tag)).await;
        fx.client(fx.w, or_add("o2", v(2))).await;
        release(hold, task).await;
        let (rows, after) = snapshot_first(query_frames(&mut fx.s_rx, LIVE_ID));
        assert!(
            !rows.contains_key("o1") && !rows.contains_key("o2"),
            "{rows:?}"
        );
        assert!(after.is_empty(), "no QUERY_UPDATE: {after:?}");
    }

    /// The subscriber's own write during the scan is in the snapshot; its
    /// later own write is not echoed live.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_subscribers_own_write_during_the_scan_is_in_the_snapshot() {
        let mut fx = live();
        fx.client(fx.w, put("kc", v(1))).await;
        let mut hold = fx.svc.test_hold_after_scan(LIVE_ID);
        let task = fx.spawn_subscribe(None);
        hold.wait_reached().await;
        fx.client(fx.w, put("kc", v(2))).await;
        fx.client(fx.s, put("kc", v(3))).await;
        release(hold, task).await;
        let (rows, _) = snapshot_first(query_frames(&mut fx.s_rx, LIVE_ID));
        assert_eq!(rows.get("kc"), Some(&v(3)));
        fx.client(fx.s, put("kc", v(4))).await;
        assert!(query_frames(&mut fx.s_rx, LIVE_ID).is_empty(), "no echo");
    }

    /// A write right after `register`, before the scan, is captured: no update
    /// precedes the snapshot.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_write_right_after_register_is_captured() {
        let mut fx = live();
        let mut hold = fx.svc.test_hold_after_register(LIVE_ID);
        let task = fx.spawn_subscribe(None);
        hold.wait_reached().await;
        fx.client(fx.w, put("kd", v(1))).await;
        release(hold, task).await;
        let (rows, _) = snapshot_first(query_frames(&mut fx.s_rx, LIVE_ID));
        assert_eq!(rows.get("kd"), Some(&v(1)));
    }

    /// A captured value that expires before the fold is not folded (TG-TTL-001).
    #[tokio::test(flavor = "multi_thread")]
    async fn a_value_that_expires_before_the_fold_is_not_folded() {
        let wall = make_timestamp().millis;
        let clock = Arc::new(ManualClock::new(wall));
        let factory = Arc::new(
            RecordStoreFactory::new(
                StorageConfig::default(),
                Arc::new(NullDataStore),
                Vec::new(),
            )
            .with_clock(Arc::clone(&clock) as _),
        );
        let mut fx = live_over(factory, test_config().outbound_channel_capacity);
        let mut hold = fx.svc.test_hold_after_scan(LIVE_ID);
        let task = fx.spawn_subscribe(None);
        hold.wait_reached().await;
        let mut op = put("ke", v(1));
        if let Some(Some(record)) = op.record.as_mut() {
            record.ttl_ms = Some(1000);
        }
        fx.client(fx.w, op).await;
        let Some(RecordValue::Lww {
            timestamp, ttl_ms, ..
        }) = fx.stored("ke").await
        else {
            panic!("ke is stored");
        };
        assert_eq!(ttl_ms, Some(1000));
        clock.set(timestamp.millis + 1001);
        release(hold, task).await;
        let (rows, _) = snapshot_first(query_frames(&mut fx.s_rx, LIVE_ID));
        assert!(!rows.contains_key("ke"), "{rows:?}");
    }

    /// The first `limit` rows by `v`.
    fn by_v(limit: u32) -> Query {
        use topgun_core::messages::base::{SortDirection, SortField};
        Query {
            sort: Some(vec![SortField {
                field: "v".to_string(),
                direction: SortDirection::Asc,
            }]),
            limit: Some(limit),
            ..Query::default()
        }
    }

    /// Applies the `QUERY_UPDATE`s among `frames` to `rows` as the client does.
    fn apply_updates(rows: &mut BTreeMap<String, rmpv::Value>, frames: Vec<Message>) {
        for frame in frames {
            if let Message::QueryUpdate { payload } = frame {
                match payload.change_type {
                    ChangeEventType::ENTER | ChangeEventType::UPDATE => {
                        rows.insert(payload.key, payload.value);
                    }
                    ChangeEventType::LEAVE => {
                        rows.remove(&payload.key);
                    }
                }
            }
        }
    }

    /// Writes `a`..`d` = 1..4 and returns [`by_v`]`(2)` from the cursor a
    /// one-shot of its first page hands out: the page `c`, `d`.
    async fn second_page(fx: &Live) -> Query {
        for (key, n) in [("a", 1), ("b", 2), ("c", 3), ("d", 4)] {
            fx.client(fx.w, put(key, v(n))).await;
        }
        let mut query = by_v(2);
        query.cursor = fx.fresh(by_v(2)).await.next_cursor;
        assert!(query.cursor.is_some(), "the first page hands out a cursor");
        query
    }

    /// A captured put that enters a full page pushes its last row out: the
    /// snapshot says more rows follow and hands out the cursor a fresh one-shot
    /// query hands out, so paging on skips no row.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_captured_put_into_a_full_page_moves_its_cursor() {
        let mut fx = live();
        for (key, n) in [("a", 1), ("b", 2), ("c", 3)] {
            fx.client(fx.w, put(key, v(n))).await;
        }
        let mut hold = fx.svc.test_hold_after_scan(LIVE_ID);
        let task = fx.spawn_subscribe_query(by_v(3));
        hold.wait_reached().await;
        fx.client(fx.w, put("x", v(0))).await;
        release(hold, task).await;
        let Some(Message::QueryResp(snapshot)) =
            query_frames(&mut fx.s_rx, LIVE_ID).into_iter().next()
        else {
            panic!("a snapshot");
        };
        let fresh = fx.fresh(by_v(3)).await;
        let keys = |page: &QueryRespPayload| {
            page.results
                .iter()
                .map(|e| e.key.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(keys(&snapshot.payload), ["x", "a", "b"]);
        assert_eq!(keys(&snapshot.payload), keys(&fresh));
        assert_eq!(snapshot.payload.has_more, Some(true));
        assert_eq!(fresh.has_more, Some(true));
        let position = |cursor: Option<&String>| {
            let cursor = decode_cursor(cursor.expect("a next cursor")).expect("decodes");
            (
                cursor.last_key,
                format!("{:?}", cursor.sort_values),
                cursor.predicate_hash,
                cursor.sort_hash,
            )
        };
        assert_eq!(
            position(snapshot.payload.next_cursor.as_ref()),
            position(fresh.next_cursor.as_ref())
        );
    }

    /// A live write of a record already expired on the store clock
    /// (TG-TTL-001) is not sent, as a fresh one-shot query leaves it out.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_live_write_of_an_expired_record_is_not_sent() {
        let mut fx = live();
        let subscribed = fx.spawn_subscribe(None).await;
        assert!(matches!(subscribed, Ok(Ok(OperationResponse::Empty))));
        let (mut rows, _) = snapshot_first(query_frames(&mut fx.s_rx, LIVE_ID));
        // Stamped in 2023 with a one-second TTL: expired on the system clock.
        fx.trusted_record("x", v(1), make_timestamp(), Some(1000))
            .await;
        apply_updates(&mut rows, query_frames(&mut fx.s_rx, LIVE_ID));
        assert!(rows.is_empty(), "{rows:?}");
        assert_eq!(rows, fx.fresh_rows(Query::default()).await);
    }

    /// A live update that moves a row of the page before its cursor takes the
    /// row off the subscriber.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_live_update_to_before_the_cursor_leaves_the_page() {
        let mut fx = live();
        let query = second_page(&fx).await;
        let subscribed = fx.spawn_subscribe_query(query.clone()).await;
        assert!(matches!(subscribed, Ok(Ok(OperationResponse::Empty))));
        let (mut rows, _) = snapshot_first(query_frames(&mut fx.s_rx, LIVE_ID));
        assert_eq!(rows.keys().collect::<Vec<_>>(), ["c", "d"]);
        fx.client(fx.w, put("c", v(0))).await;
        apply_updates(&mut rows, query_frames(&mut fx.s_rx, LIVE_ID));
        assert_eq!(rows.keys().collect::<Vec<_>>(), ["d"]);
        assert_eq!(rows, fx.fresh_rows(query).await);
    }

    /// A captured update that moves a row of the page before its cursor takes
    /// the row out of the snapshot.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_captured_update_to_before_the_cursor_leaves_the_snapshot() {
        let mut fx = live();
        let query = second_page(&fx).await;
        let mut hold = fx.svc.test_hold_after_scan(LIVE_ID);
        let task = fx.spawn_subscribe_query(query.clone());
        hold.wait_reached().await;
        fx.client(fx.w, put("c", v(0))).await;
        release(hold, task).await;
        let (rows, _) = snapshot_first(query_frames(&mut fx.s_rx, LIVE_ID));
        assert_eq!(rows.keys().collect::<Vec<_>>(), ["d"]);
        assert_eq!(rows, fx.fresh_rows(query).await);
    }

    /// Subscribes to [`by_v`]`(3)` over `a`=1, `b`=2, `c`=3, makes the `captured`
    /// writes during the scan, and returns the snapshot rows and the rows of a
    /// fresh one-shot query made after it.
    async fn top3_fold(
        captured: Vec<ClientOp>,
    ) -> (BTreeMap<String, rmpv::Value>, BTreeMap<String, rmpv::Value>) {
        let mut fx = live();
        for (key, n) in [("a", 1), ("b", 2), ("c", 3)] {
            fx.client(fx.w, put(key, v(n))).await;
        }
        let mut hold = fx.svc.test_hold_after_scan(LIVE_ID);
        let task = fx.spawn_subscribe_query(by_v(3));
        hold.wait_reached().await;
        for op in captured {
            fx.client(fx.w, op).await;
        }
        release(hold, task).await;
        let (rows, _) = snapshot_first(query_frames(&mut fx.s_rx, LIVE_ID));
        (rows, fx.fresh_rows(by_v(3)).await)
    }

    /// A captured remove of a row in the window takes it out of the snapshot.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_captured_remove_of_a_windowed_row_leaves_the_snapshot() {
        let (rows, fresh) = top3_fold(vec![remove("b")]).await;
        assert_eq!(rows.keys().collect::<Vec<_>>(), ["a", "c"]);
        assert_eq!(rows, fresh);
    }

    /// A captured put that enters the window pushes its last row out.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_captured_put_that_enters_the_window_pushes_the_last_row_out() {
        let (rows, fresh) = top3_fold(vec![put("x", v(0))]).await;
        assert_eq!(rows.keys().collect::<Vec<_>>(), ["a", "b", "x"]);
        assert_eq!(rows, fresh);
    }

    /// A captured put that sorts a windowed row past the limit takes it out,
    /// and the row it had pushed out comes back.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_captured_put_that_leaves_the_window_promotes_the_retained_row() {
        let (rows, fresh) = top3_fold(vec![put("x", v(0)), put("a", v(9))]).await;
        assert_eq!(rows.keys().collect::<Vec<_>>(), ["b", "c", "x"]);
        assert_eq!(rows, fresh);
    }

    /// A captured put into the window that expires before the fold is left
    /// out, and the row it would have pushed out stays (TG-TTL-001).
    #[tokio::test(flavor = "multi_thread")]
    async fn a_captured_put_into_the_window_that_expires_before_the_fold_is_left_out() {
        let clock = Arc::new(ManualClock::new(make_timestamp().millis));
        let factory = Arc::new(
            RecordStoreFactory::new(
                StorageConfig::default(),
                Arc::new(NullDataStore),
                Vec::new(),
            )
            .with_clock(Arc::clone(&clock) as _),
        );
        let mut fx = live_over(factory, test_config().outbound_channel_capacity);
        for (key, n) in [("a", 1), ("b", 2), ("c", 3)] {
            fx.client(fx.w, put(key, v(n))).await;
        }
        let mut hold = fx.svc.test_hold_after_scan(LIVE_ID);
        let task = fx.spawn_subscribe_query(by_v(3));
        hold.wait_reached().await;
        let mut op = put("x", v(0));
        if let Some(Some(record)) = op.record.as_mut() {
            record.ttl_ms = Some(1000);
        }
        fx.client(fx.w, op).await;
        let Some(RecordValue::Lww { timestamp, .. }) = fx.stored("x").await else {
            panic!("x is stored");
        };
        clock.set(timestamp.millis + 1001);
        release(hold, task).await;
        let (rows, _) = snapshot_first(query_frames(&mut fx.s_rx, LIVE_ID));
        assert_eq!(rows.keys().collect::<Vec<_>>(), ["a", "b", "c"]);
        assert_eq!(rows, fx.fresh_rows(by_v(3)).await);
    }

    /// `remove_if_same` releases the outer read guard before it drops an empty
    /// map entry (it would deadlock otherwise), drops that entry, and never
    /// removes a newer registration of the same id.
    #[test]
    fn remove_if_same_drops_the_outer_entry_and_returns() {
        let registry = Arc::new(QueryRegistry::new());
        let register = |buffer: &Arc<DeltaBuffer>| {
            registry.register(QuerySubscription {
                query_id: "q".to_string(),
                connection_id: ConnectionId(1),
                map_name: "m".to_string(),
                query: Query::default(),
                previous_result_keys: DashSet::new(),
                live_window: Arc::new(LiveWindow::new(vec![], None)),
                fields: None,
                delta_buffer: Arc::clone(buffer),
            });
        };
        let old = Arc::new(DeltaBuffer::new(1));
        register(&old);

        let (tx, rx) = std::sync::mpsc::channel();
        let (thread_registry, thread_buffer) = (Arc::clone(&registry), Arc::clone(&old));
        std::thread::spawn(move || {
            let removed = thread_registry.remove_if_same("m", ConnectionId(1), "q", &thread_buffer);
            let _ = tx.send(removed);
        });
        let removed = rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("remove_if_same returned (no deadlock)");
        assert!(removed);
        assert!(
            !registry.subscriptions.contains_key("m"),
            "the empty entry is dropped"
        );

        let new = Arc::new(DeltaBuffer::new(1));
        register(&new);
        assert!(!registry.remove_if_same("m", ConnectionId(1), "q", &old));
        let kept = registry
            .get_subscription("m", ConnectionId(1), "q")
            .expect("the newer registration stays");
        assert!(Arc::ptr_eq(&kept.delta_buffer, &new));
    }

    /// Report-only: the whole time the snapshot buffer's lock is held to close
    /// it with 4096 captured writes (a third updates, a third new keys, a third
    /// removes) over a 10 000-row projected, sorted, limited snapshot: the fold,
    /// the page's cursor, the encoding and the send on a reserved slot, by the
    /// code the subscribe runs.
    #[test]
    #[ignore = "measurement; run with --ignored --nocapture"]
    fn fold_lock_hold_of_4096_entries_over_10000_rows() {
        use topgun_core::messages::base::{SortDirection, SortField};
        const ROWS: i64 = 10_000;
        let row = |n: i64| {
            rmpv::Value::Map(vec![
                (rmpv::Value::from("v"), rmpv::Value::from(n)),
                (
                    rmpv::Value::from("pad"),
                    rmpv::Value::from("0123456789abcdef"),
                ),
            ])
        };
        let mut runs = Vec::new();
        for _ in 0..5 {
            let query = Query {
                sort: Some(vec![SortField {
                    field: "v".to_string(),
                    direction: SortDirection::Asc,
                }]),
                limit: Some(10_000),
                ..Query::default()
            };
            let sub = QuerySubscription {
                query_id: "m".to_string(),
                connection_id: ConnectionId(1),
                map_name: "m".to_string(),
                live_window: Arc::new(LiveWindow::new(
                    query.sort.clone().unwrap_or_default(),
                    query.limit,
                )),
                query,
                previous_result_keys: DashSet::new(),
                fields: Some(vec!["v".to_string()]),
                delta_buffer: Arc::new(DeltaBuffer::new(SNAPSHOT_BUFFER_CAPACITY)),
            };
            let rows: Vec<(String, rmpv::Value)> =
                (0..ROWS).map(|n| (format!("r{n:05}"), row(n))).collect();
            let _ = sub
                .live_window
                .apply_batch(rows.iter().map(|(k, v)| (k.as_str(), Some(v), true)));
            let results = rows
                .iter()
                .map(|(key, value)| QueryResultEntry {
                    key: key.clone(),
                    value: project_fields(&["v".to_string()], value),
                })
                .collect();
            let snapshot = QueryRespPayload {
                query_id: "m".to_string(),
                results,
                ..Default::default()
            };
            sub.delta_buffer.activate();
            for i in 0..4096_i64 {
                let (key, record) = match i % 3 {
                    0 => (format!("r{:05}", i * 2), Some(lww_record(row(ROWS + i)))),
                    1 => (format!("n{i:05}"), Some(lww_record(row(-i)))),
                    _ => (format!("r{:05}", i * 2 + 1), None),
                };
                assert!(sub
                    .delta_buffer
                    .route(&key, record.as_ref(), record.is_some()));
            }
            let page_cursor = PageCursor {
                limit: Some(10_000),
                template: vec![SortValue {
                    field: "v".to_string(),
                    value: serde_json::Value::Null,
                    direction: SortDirection::Asc,
                }],
                predicate_hash: 0,
                sort_hash: 0,
                now_ms: 0,
            };
            let (tx, mut rx) = tokio::sync::mpsc::channel(1);
            let start = std::time::Instant::now();
            sub.delta_buffer.close(|captured| {
                let Captured::Entries(entries) = captured else {
                    panic!("no overflow");
                };
                let permit = tx.try_reserve().expect("a free slot");
                let frame =
                    folded_snapshot_frame(snapshot, &page_cursor, &sub, &entries, u64::MAX / 2);
                permit.send(OutboundMessage::Binary(frame.expect("encodes")));
                Closing::Live
            });
            runs.push(start.elapsed());
            assert!(rx.try_recv().is_ok());
        }
        for (i, run) in runs.iter().enumerate() {
            println!("close_lock_hold run {i}: {:.3} ms", run.as_secs_f64() * 1e3);
        }
        runs.sort();
        println!(
            "close_lock_hold median: {:.3} ms",
            runs[runs.len() / 2].as_secs_f64() * 1e3
        );
    }
}
