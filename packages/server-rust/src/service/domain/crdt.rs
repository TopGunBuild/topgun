//! CRDT domain service handling `ClientOp` and `OpBatch` operations.
//!
//! Merges LWW-Map and OR-Map data into the `RecordStore` and broadcasts
//! `ServerEvent` messages to subscribed client connections.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use async_trait::async_trait;
use tower::Service;

use topgun_core::messages::{
    ClientOp, ClientOpMessage, Message, OpAckMessage, OpAckPayload,
    OpBatchMessage, ServerEventPayload, ServerEventType, WriteConcern,
};
use topgun_core::types::Value;
use topgun_core::{hash_to_partition, LWWRecord, Timestamp};
#[cfg(test)]
use topgun_core::ORMapRecord;

use tracing::Instrument;

use crate::network::connection::{ConnectionId, ConnectionMetadata, ConnectionRegistry};
use crate::service::domain::predicate::{evaluate_predicate, evaluate_where, value_to_rmpv, EvalContext};
use crate::service::domain::query::QueryRegistry;
use crate::service::operation::{
    service_names, Operation, OperationContext, OperationError, OperationResponse,
};
use crate::service::registry::{ManagedService, ServiceContext};
use crate::service::security::WriteValidator;
use crate::storage::record::{OrMapEntry, RecordValue};
use crate::storage::{CallerProvenance, ExpiryPolicy, RecordStoreFactory};
use crate::traits::SchemaProvider;

// ---------------------------------------------------------------------------
// Query predicate matching
// ---------------------------------------------------------------------------

/// Evaluates whether an `rmpv::Value` matches a query's predicate or where clause.
///
/// Used by `broadcast_query_updates` to determine ENTER/UPDATE/LEAVE events
/// without depending on `QueryMutationObserver`.
fn matches_query_predicate(query: &topgun_core::messages::base::Query, data: &rmpv::Value) -> bool {
    if let Some(pred) = &query.predicate {
        evaluate_predicate(pred, &EvalContext::data_only(data))
    } else if let Some(wh) = &query.r#where {
        evaluate_where(wh, data)
    } else {
        // No filter: match all
        true
    }
}

// ---------------------------------------------------------------------------
// CrdtService
// ---------------------------------------------------------------------------

/// Real CRDT domain service handling `ClientOp` and `OpBatch` operations.
///
/// Replaces the `domain_stub!(CrdtService, ...)` macro-generated stub.
/// Merges LWW and OR-Map data into the `RecordStore` and broadcasts
/// `ServerEvent` messages to connected clients.
///
/// Validation order: auth/ACL/size (`WriteValidator`) → schema (`SchemaProvider`) → CRDT merge.
pub struct CrdtService {
    record_store_factory: Arc<RecordStoreFactory>,
    connection_registry: Arc<ConnectionRegistry>,
    write_validator: Arc<WriteValidator>,
    query_registry: Arc<QueryRegistry>,
    schema_provider: Arc<dyn SchemaProvider>,
}

impl CrdtService {
    /// Creates a new `CrdtService` with its required dependencies.
    #[must_use]
    pub fn new(
        record_store_factory: Arc<RecordStoreFactory>,
        connection_registry: Arc<ConnectionRegistry>,
        write_validator: Arc<WriteValidator>,
        query_registry: Arc<QueryRegistry>,
        schema_provider: Arc<dyn SchemaProvider>,
    ) -> Self {
        Self {
            record_store_factory,
            connection_registry,
            write_validator,
            query_registry,
            schema_provider,
        }
    }
}

// ---------------------------------------------------------------------------
// ManagedService implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl ManagedService for CrdtService {
    fn name(&self) -> &'static str {
        service_names::CRDT
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

impl Service<Operation> for Arc<CrdtService> {
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
                    Operation::ClientOp { ctx, payload } => {
                        svc.handle_client_op(&ctx, payload).await
                    }
                    Operation::OpBatch { ctx, payload } => {
                        svc.handle_op_batch(&ctx, payload).await
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

impl CrdtService {
    /// Handles a single `ClientOp` message: validates, applies CRDT merge, and broadcasts event.
    async fn handle_client_op(
        &self,
        ctx: &OperationContext,
        msg: ClientOpMessage,
    ) -> Result<OperationResponse, OperationError> {
        let op = &msg.payload;
        let partition_id = ctx.partition_id.unwrap_or(0);

        // Acquire metadata snapshot if a connection_id is present.
        // None means internal/system call — skip validation.
        let sanitized_ts = if let Some(conn_id) = ctx.connection_id {
            let metadata_snapshot = self.snapshot_metadata(conn_id).await?;
            let value_size = estimate_value_size(op);
            self.write_validator.validate_write(ctx, &metadata_snapshot, &op.map_name, value_size)?;
            // Schema validation runs after auth/ACL/size checks.
            self.validate_schema_for_op(op)?;
            Some(self.write_validator.sanitize_hlc())
        } else {
            None
        };

        // Read old value before mutation for query broadcast filtering.
        let old_rmpv_value = self.read_old_value_for_queries(&op.map_name, &op.key, partition_id).await;

        let event_payload = self.apply_single_op(op, partition_id, sanitized_ts.as_ref()).await?;

        self.broadcast_event(&event_payload, ctx.connection_id)?;
        self.broadcast_query_updates(&event_payload, old_rmpv_value.as_ref(), ctx.connection_id);

        let last_id = op.id.clone().unwrap_or_else(|| "unknown".to_string());
        Ok(OperationResponse::Message(Box::new(Message::OpAck(
            OpAckMessage {
                payload: OpAckPayload {
                    last_id,
                    // CRDT merge succeeded in memory — report APPLIED durability
                    achieved_level: Some(WriteConcern::APPLIED),
                    results: None,
                },
            },
        ))))
    }

    /// Handles an `OpBatch` message: validates all ops atomically, then applies each sequentially.
    ///
    /// Atomic rejection: if any op fails validation, no ops are applied.
    /// Each op gets its own sanitized HLC timestamp (monotonically increasing via successive calls).
    async fn handle_op_batch(
        &self,
        ctx: &OperationContext,
        msg: OpBatchMessage,
    ) -> Result<OperationResponse, OperationError> {
        let ops = &msg.payload.ops;

        if ops.is_empty() {
            return Ok(OperationResponse::Ack {
                call_id: ctx.call_id,
            });
        }

        let mut last_id = "unknown".to_string();

        // Validate all ops before applying any (atomic batch rejection).
        // Snapshot metadata once at batch start to avoid per-op lock acquisition.
        if let Some(conn_id) = ctx.connection_id {
            let metadata_snapshot = self.snapshot_metadata(conn_id).await?;
            for op in ops {
                let value_size = estimate_value_size(op);
                self.write_validator.validate_write(ctx, &metadata_snapshot, &op.map_name, value_size)?;
                // Schema validation runs after auth/ACL/size checks, before any apply.
                self.validate_schema_for_op(op)?;
            }
            // All ops validated — apply them sequentially with sanitized timestamps.
            // Each op gets its own partition based on its key (OpBatch ctx has
            // partition_id=None because the batch contains keys for many partitions).
            for op in ops {
                let sanitized_ts = self.write_validator.sanitize_hlc();
                let partition_id = hash_to_partition(&op.key);
                // Read old value before mutation for query broadcast filtering.
                let old_rmpv_value = self.read_old_value_for_queries(&op.map_name, &op.key, partition_id).await;
                let event_payload = self.apply_single_op(op, partition_id, Some(&sanitized_ts)).await?;
                self.broadcast_event(&event_payload, ctx.connection_id)?;
                self.broadcast_query_updates(&event_payload, old_rmpv_value.as_ref(), ctx.connection_id);
                if let Some(id) = &op.id {
                    last_id = id.clone();
                }
            }
        } else {
            // Internal/system call (no connection_id) — skip validation.
            for op in ops {
                let partition_id = hash_to_partition(&op.key);
                // Read old value before mutation for query broadcast filtering.
                let old_rmpv_value = self.read_old_value_for_queries(&op.map_name, &op.key, partition_id).await;
                let event_payload = self.apply_single_op(op, partition_id, None).await?;
                self.broadcast_event(&event_payload, ctx.connection_id)?;
                self.broadcast_query_updates(&event_payload, old_rmpv_value.as_ref(), ctx.connection_id);
                if let Some(id) = &op.id {
                    last_id = id.clone();
                }
            }
        }

        Ok(OperationResponse::Message(Box::new(Message::OpAck(
            OpAckMessage {
                payload: OpAckPayload {
                    last_id,
                    // All ops in the batch merged successfully in memory — report APPLIED
                    achieved_level: Some(WriteConcern::APPLIED),
                    results: None,
                },
            },
        ))))
    }

    /// Snapshots connection metadata by ID, releasing the read lock immediately.
    ///
    /// Returns `Err(OperationError::Unauthorized)` if the connection is not found
    /// (e.g., disconnected between routing and handling).
    async fn snapshot_metadata(
        &self,
        conn_id: ConnectionId,
    ) -> Result<ConnectionMetadata, OperationError> {
        let handle = self
            .connection_registry
            .get(conn_id)
            .ok_or(OperationError::Unauthorized)?;
        // Clone out of the lock immediately so we don't hold the read guard across async ops.
        let snapshot = handle.metadata.read().await.clone();
        Ok(snapshot)
    }

    /// Applies a single `ClientOp` to the `RecordStore` and returns the `ServerEventPayload`
    /// to broadcast. Called by both `handle_client_op` and `handle_op_batch`.
    ///
    /// `sanitized_ts` — when `Some`, replaces client-provided timestamps in stored records.
    /// When `None` (internal/test calls with no `connection_id`), the client timestamp is used as-is.
    #[allow(clippy::too_many_lines)]
    async fn apply_single_op(
        &self,
        op: &ClientOp,
        partition_id: u32,
        sanitized_ts: Option<&Timestamp>,
    ) -> Result<ServerEventPayload, OperationError> {
        let store = self
            .record_store_factory
            .get_or_create(&op.map_name, partition_id);

        // Determine the operation type and build the event payload.
        // Priority: explicit op_type REMOVE / tombstone -> REMOVE
        //           or_record present   -> OR_ADD
        //           or_tag present      -> OR_REMOVE
        //           otherwise           -> LWW PUT

        let is_remove = op.op_type.as_deref() == Some("REMOVE")
            || matches!(&op.record, Some(None));

        let is_or_add = matches!(&op.or_record, Some(Some(_)));
        let is_or_remove = matches!(&op.or_tag, Some(Some(_))) && op.or_record.is_none();

        if is_remove {
            // REMOVE/OR_REMOVE: no timestamp sanitization needed (removes are idempotent)
            store
                .remove(&op.key, CallerProvenance::CrdtMerge)
                .await
                .map_err(OperationError::Internal)?;

            Ok(ServerEventPayload {
                map_name: op.map_name.clone(),
                event_type: ServerEventType::REMOVE,
                key: op.key.clone(),
                record: None,
                or_record: None,
                or_tag: None,
            })
        } else if is_or_add {
            // Safe: matched Some(Some(_)) above
            let or_rec = op
                .or_record
                .as_ref()
                .and_then(|o| o.as_ref())
                .expect("or_record is Some(Some(_))");

            // Replace client timestamp with sanitized server timestamp if provided.
            let (new_entry, stored_or_rec) = if let Some(ts) = sanitized_ts {
                let mut sanitized_rec = or_rec.clone();
                sanitized_rec.timestamp = ts.clone();
                // Regenerate tag from sanitized timestamp: "{millis}:{counter}:{node_id}"
                sanitized_rec.tag = format!("{}:{}:{}", ts.millis, ts.counter, ts.node_id);
                let entry = OrMapEntry {
                    value: rmpv_to_value(&sanitized_rec.value),
                    tag: sanitized_rec.tag.clone(),
                    timestamp: sanitized_rec.timestamp.clone(),
                };
                (entry, sanitized_rec)
            } else {
                let entry = OrMapEntry {
                    value: rmpv_to_value(&or_rec.value),
                    tag: or_rec.tag.clone(),
                    timestamp: or_rec.timestamp.clone(),
                };
                (entry, or_rec.clone())
            };

            // Read existing OR-Map entries so the new entry is merged in rather than replacing.
            // OR-Map add-wins semantics require all concurrent additions to be preserved.
            let record_value = {
                let existing = store.get(&op.key, false).await.map_err(OperationError::Internal)?;
                let mut records: Vec<OrMapEntry> = match existing.map(|r| r.value) {
                    Some(RecordValue::OrMap { records }) => records,
                    _ => Vec::new(),
                };
                // Remove any existing entry with the same tag (idempotent re-add).
                records.retain(|e| e.tag != new_entry.tag);
                records.push(new_entry);
                RecordValue::OrMap { records }
            };

            store
                .put(&op.key, record_value, ExpiryPolicy::NONE, CallerProvenance::CrdtMerge)
                .await
                .map_err(OperationError::Internal)?;

            Ok(ServerEventPayload {
                map_name: op.map_name.clone(),
                event_type: ServerEventType::OR_ADD,
                key: op.key.clone(),
                record: None,
                or_record: Some(stored_or_rec.clone()),
                or_tag: Some(stored_or_rec.tag.clone()),
            })
        } else if is_or_remove {
            // Safe: matched Some(Some(_)) above
            let tag = op
                .or_tag
                .as_ref()
                .and_then(|o| o.as_ref())
                .expect("or_tag is Some(Some(_))");

            // OR_REMOVE is tag-based; no timestamp sanitization needed
            let record_value = RecordValue::OrTombstones {
                tags: vec![tag.clone()],
            };
            store
                .put(&op.key, record_value, ExpiryPolicy::NONE, CallerProvenance::CrdtMerge)
                .await
                .map_err(OperationError::Internal)?;

            Ok(ServerEventPayload {
                map_name: op.map_name.clone(),
                event_type: ServerEventType::OR_REMOVE,
                key: op.key.clone(),
                record: None,
                or_record: None,
                or_tag: Some(tag.clone()),
            })
        } else {
            // LWW PUT: record may be None (no-op put with no value) or Some(Some(rec))
            let lww_rec = op.record.as_ref().and_then(|o| o.as_ref());

            let broadcast_rec = if let Some(rec) = lww_rec {
                // Replace client timestamp with sanitized server timestamp if provided.
                let (record_value, stored_rec) = if let Some(ts) = sanitized_ts {
                    let mut sanitized_rec = rec.clone();
                    sanitized_rec.timestamp = ts.clone();
                    let rv = lww_record_to_record_value(&sanitized_rec);
                    (rv, sanitized_rec)
                } else {
                    (lww_record_to_record_value(rec), rec.clone())
                };
                store
                    .put(&op.key, record_value, ExpiryPolicy::NONE, CallerProvenance::CrdtMerge)
                    .await
                    .map_err(OperationError::Internal)?;
                Some(stored_rec)
            } else {
                None
            };

            Ok(ServerEventPayload {
                map_name: op.map_name.clone(),
                event_type: ServerEventType::PUT,
                key: op.key.clone(),
                // Broadcast the sanitized record (with server timestamp), not the original client record
                record: broadcast_rec,
                or_record: None,
                or_tag: None,
            })
        }
    }

    /// Serializes a `ServerEventPayload` as `MsgPack` and sends only to connections
    /// with active query subscriptions for the affected map.
    ///
    /// Skips serialization entirely when no subscribers exist, avoiding
    /// unnecessary `rmp_serde::to_vec_named` calls.
    fn broadcast_event(
        &self,
        payload: &ServerEventPayload,
        exclude_connection_id: Option<ConnectionId>,
    ) -> Result<(), OperationError> {
        let mut ids = self
            .query_registry
            .get_subscribed_connection_ids(&payload.map_name);

        if ids.is_empty() {
            return Ok(());
        }

        // Exclude the writing client so it does not receive its own event back
        if let Some(exclude_id) = exclude_connection_id {
            ids.remove(&exclude_id);
        }

        if ids.is_empty() {
            return Ok(());
        }

        let msg = Message::ServerEvent {
            payload: payload.clone(),
        };
        let bytes = rmp_serde::to_vec_named(&msg)
            .map_err(|e| OperationError::Internal(anyhow::anyhow!("serialize error: {e}")))?;
        self.connection_registry.send_to_connections(&ids, &bytes);
        Ok(())
    }

    /// Reads the old record value for a key before mutation, for query broadcast filtering.
    ///
    /// Returns `None` if no queries are active for this map, if no record exists,
    /// or if the record is not an LWW record (OR-Map records are skipped).
    async fn read_old_value_for_queries(
        &self,
        map_name: &str,
        key: &str,
        partition_id: u32,
    ) -> Option<rmpv::Value> {
        let has_queries = !self.query_registry.get_subscriptions_for_map(map_name).is_empty();
        if !has_queries {
            return None;
        }

        let store = self.record_store_factory.get_or_create(map_name, partition_id);
        let old_record = store.get(key, false).await.ok().flatten()?;
        if let RecordValue::Lww { ref value, .. } = old_record.value {
            Some(value_to_rmpv(value))
        } else {
            None
        }
    }

    /// Broadcasts `QUERY_UPDATE` messages to subscribers of queries targeting the mutated map.
    ///
    /// This method:
    /// - Evaluates each standing query subscription against the old and new values
    /// - Determines ENTER/UPDATE/LEAVE change type
    /// - Applies field projection if the subscription has `fields`
    /// - Skips the writing connection (writer exclusion)
    /// - Updates `previous_result_keys` for accurate future change detection
    fn broadcast_query_updates(
        &self,
        event_payload: &ServerEventPayload,
        old_rmpv_value: Option<&rmpv::Value>,
        exclude_connection_id: Option<ConnectionId>,
    ) {
        let subs = self.query_registry.get_subscriptions_for_map(&event_payload.map_name);
        if subs.is_empty() {
            return;
        }

        // Extract the new value from the event payload.
        let new_rmpv_value: Option<rmpv::Value> = event_payload
            .record
            .as_ref()
            .and_then(|r| r.value.clone());

        for sub in &subs {
            // Skip the writing connection so it does not receive its own updates.
            if let Some(exclude_id) = exclude_connection_id {
                if sub.connection_id == exclude_id {
                    continue;
                }
            }

            // Evaluate old/new against the query predicate.
            let old_matches = old_rmpv_value.is_some_and(|v| {
                matches_query_predicate(&sub.query, v)
            });
            let new_matches = new_rmpv_value.as_ref().is_some_and(|v| {
                matches_query_predicate(&sub.query, v)
            });

            let change_type = match (old_matches, new_matches) {
                (false, true) => {
                    sub.previous_result_keys.insert(event_payload.key.clone());
                    topgun_core::messages::base::ChangeEventType::ENTER
                }
                (true, true) => topgun_core::messages::base::ChangeEventType::UPDATE,
                (true, false) => {
                    sub.previous_result_keys.remove(&event_payload.key);
                    topgun_core::messages::base::ChangeEventType::LEAVE
                }
                (false, false) => continue,
            };

            // Apply field projection if the subscription has fields.
            let value = if new_matches {
                let raw = new_rmpv_value.clone().unwrap_or(rmpv::Value::Nil);
                if let Some(ref fields) = sub.fields {
                    super::query::project_fields(fields, &raw)
                } else {
                    raw
                }
            } else {
                rmpv::Value::Nil
            };

            let payload = topgun_core::messages::client_events::QueryUpdatePayload {
                query_id: sub.query_id.clone(),
                key: event_payload.key.clone(),
                value,
                change_type,
            };
            let msg = topgun_core::messages::Message::QueryUpdate { payload };
            if let Ok(bytes) = rmp_serde::to_vec_named(&msg) {
                use crate::network::connection::OutboundMessage;
                if let Some(handle) = self.connection_registry.get(sub.connection_id) {
                    let _ = handle.try_send(OutboundMessage::Binary(bytes));
                }
            }
        }
    }

    /// Validates a single `ClientOp` against the registered schema for its map.
    ///
    /// Returns `Ok(())` immediately for:
    /// - REMOVE operations (no value to validate): detected via `op_type == "REMOVE"` or
    ///   `record == Some(None)` (tombstone pattern), mirroring `apply_single_op`.
    /// - `OR_REMOVE` operations (tag-based, no value).
    /// - LWW records where the inner value is `None` (partial tombstone).
    /// - Maps with no registered schema (optional mode: passthrough).
    ///
    /// Returns `Err(OperationError::SchemaInvalid)` when the value fails validation.
    fn validate_schema_for_op(&self, op: &ClientOp) -> Result<(), OperationError> {
        // Mirror the same REMOVE detection as apply_single_op.
        let is_remove = op.op_type.as_deref() == Some("REMOVE")
            || matches!(&op.record, Some(None));
        let is_or_remove = matches!(&op.or_tag, Some(Some(_))) && op.or_record.is_none();

        if is_remove || is_or_remove {
            return Ok(());
        }

        // Extract the rmpv::Value to validate.
        let rmpv_val: Option<rmpv::Value> = if let Some(Some(or_rec)) = &op.or_record {
            // OR_ADD: validate the value field of the ORMapRecord.
            Some(or_rec.value.clone())
        } else if let Some(Some(lww_rec)) = &op.record {
            // LWW PUT: LWWRecord.value is Option<rmpv::Value>.
            // None inner value means no data to validate — skip.
            lww_rec.value.clone()
        } else {
            // No record payload — nothing to validate.
            None
        };

        let Some(rmpv_val) = rmpv_val else {
            return Ok(());
        };

        let value = topgun_core::types::Value::from(rmpv_val);
        match self.schema_provider.validate(&op.map_name, &value) {
            topgun_core::ValidationResult::Valid => Ok(()),
            topgun_core::ValidationResult::Invalid { errors } => {
                Err(OperationError::SchemaInvalid {
                    map_name: op.map_name.clone(),
                    errors,
                })
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Value size estimation
// ---------------------------------------------------------------------------

/// Estimates the serialized byte length of the record payload in a `ClientOp`.
///
/// Uses `rmp_serde::to_vec_named()` on the `record` or `or_record` field.
/// For REMOVE and `OR_REMOVE` operations, returns 0 (removes are never rejected on size).
/// If serialization fails, returns `u64::MAX` so the op is rejected by size check.
fn estimate_value_size(op: &ClientOp) -> u64 {
    let is_remove = op.op_type.as_deref() == Some("REMOVE") || matches!(&op.record, Some(None));
    let is_or_remove = matches!(&op.or_tag, Some(Some(_))) && op.or_record.is_none();

    if is_remove || is_or_remove {
        return 0;
    }

    if let Some(Some(or_rec)) = &op.or_record {
        return rmp_serde::to_vec_named(or_rec)
            .map(|v| v.len() as u64)
            .unwrap_or(u64::MAX);
    }

    if let Some(Some(rec)) = &op.record {
        return rmp_serde::to_vec_named(rec)
            .map(|v| v.len() as u64)
            .unwrap_or(u64::MAX);
    }

    // No record payload (e.g., LWW PUT with no value)
    0
}

// ---------------------------------------------------------------------------
// Conversion helpers
// ---------------------------------------------------------------------------

/// Recursively converts an `rmpv::Value` (wire format) into a `topgun_core::types::Value`
/// (storage format).
///
/// No `From<rmpv::Value>` conversion exists between these types, so this
/// manual recursive conversion is required.
pub(crate) fn rmpv_to_value(v: &rmpv::Value) -> Value {
    match v {
        rmpv::Value::Boolean(b) => Value::Bool(*b),
        rmpv::Value::Integer(i) => {
            // Try signed first; fall back to unsigned (values exceeding i64::MAX).
            let n = if let Some(s) = i.as_i64() {
                s
            } else {
                // Intentional wrap: values > i64::MAX map to negative i64.
                #[allow(clippy::cast_possible_wrap)]
                let u = i.as_u64().unwrap_or(0) as i64;
                u
            };
            Value::Int(n)
        }
        rmpv::Value::F32(f) => Value::Float(f64::from(*f)),
        rmpv::Value::F64(f) => Value::Float(*f),
        rmpv::Value::String(s) => Value::String(s.as_str().unwrap_or("").to_string()),
        rmpv::Value::Binary(b) => Value::Bytes(b.clone()),
        rmpv::Value::Array(arr) => Value::Array(arr.iter().map(rmpv_to_value).collect()),
        rmpv::Value::Map(map) => {
            let btree: BTreeMap<String, Value> = map
                .iter()
                .map(|(k, v): &(rmpv::Value, rmpv::Value)| {
                    // Extract the raw string from rmpv::Value::String to avoid
                    // the Display impl which wraps strings in quotes.
                    let key = match k {
                        rmpv::Value::String(s) => {
                            s.as_str().unwrap_or("").to_string()
                        }
                        other => other.to_string(),
                    };
                    (key, rmpv_to_value(v))
                })
                .collect();
            Value::Map(btree)
        }
        // Nil and Extension types are not represented in topgun_core::types::Value;
        // fall back to Null rather than panicking.
        rmpv::Value::Nil | rmpv::Value::Ext(_, _) => Value::Null,
    }
}

/// Converts a wire-format `LWWRecord<rmpv::Value>` into a storage `RecordValue::Lww`.
fn lww_record_to_record_value(record: &LWWRecord<rmpv::Value>) -> RecordValue {
    let value = record
        .value
        .as_ref()
        .map_or(Value::Null, rmpv_to_value);
    RecordValue::Lww {
        value,
        timestamp: record.timestamp.clone(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use parking_lot::Mutex;
    use topgun_core::messages::Message;
    use topgun_core::{HLC, SystemClock, Timestamp};
    use tower::ServiceExt;

    use super::*;
    use crate::network::connection::{ConnectionKind, ConnectionRegistry};
    use crate::service::domain::query::QueryRegistry;
    use crate::service::domain::schema::SchemaService;
    use crate::service::operation::{service_names, OperationContext, OperationResponse};
    use crate::service::security::{SecurityConfig, WriteValidator};
    use crate::storage::datastores::NullDataStore;
    use crate::storage::factory::RecordStoreFactory;
    use crate::storage::impls::StorageConfig;

    fn make_factory() -> Arc<RecordStoreFactory> {
        Arc::new(RecordStoreFactory::new(
            StorageConfig::default(),
            Arc::new(NullDataStore),
            Vec::new(),
        ))
    }

    fn make_validator() -> Arc<WriteValidator> {
        let hlc = Arc::new(Mutex::new(HLC::new("test-node".to_string(), Box::new(SystemClock))));
        Arc::new(WriteValidator::new(Arc::new(SecurityConfig::default()), hlc))
    }

    fn make_service() -> Arc<CrdtService> {
        let factory = make_factory();
        let registry = Arc::new(ConnectionRegistry::new());
        let query_registry = Arc::new(QueryRegistry::new());
        Arc::new(CrdtService::new(factory, registry, make_validator(), query_registry, Arc::new(SchemaService::new())))
    }

    fn make_timestamp() -> Timestamp {
        Timestamp {
            millis: 1_700_000_000_000,
            counter: 1,
            node_id: "test-node".to_string(),
        }
    }

    fn make_ctx() -> OperationContext {
        OperationContext::new(1, service_names::CRDT, make_timestamp(), 5000)
    }

    // -- AC17: ManagedService name is "crdt" --

    #[test]
    fn managed_service_name() {
        let factory = make_factory();
        let registry = Arc::new(ConnectionRegistry::new());
        let query_registry = Arc::new(QueryRegistry::new());
        let svc = CrdtService::new(factory, registry, make_validator(), query_registry, Arc::new(SchemaService::new()));
        assert_eq!(svc.name(), "crdt");
    }

    // -- LWW PUT --

    #[tokio::test]
    async fn lww_put_returns_op_ack() {
        let svc = make_service();
        let record = topgun_core::LWWRecord {
            value: Some(rmpv::Value::String("Alice".into())),
            timestamp: make_timestamp(),
            ttl_ms: None,
        };
        let op = Operation::ClientOp {
            ctx: make_ctx(),
            payload: topgun_core::messages::ClientOpMessage {
                payload: topgun_core::messages::base::ClientOp {
                    id: Some("op-1".to_string()),
                    map_name: "users".to_string(),
                    key: "user-1".to_string(),
                    op_type: None,
                    record: Some(Some(record)),
                    or_record: None,
                    or_tag: None,
                    write_concern: None,
                    timeout: None,
                },
            },
        };

        let resp = svc.oneshot(op).await.unwrap();
        assert!(
            matches!(resp, OperationResponse::Message(ref msg) if matches!(**msg, Message::OpAck(_))),
            "expected OpAck, got {resp:?}"
        );
    }

    // -- LWW REMOVE (tombstone) --

    #[tokio::test]
    async fn lww_remove_via_tombstone_returns_op_ack() {
        let svc = make_service();
        let op = Operation::ClientOp {
            ctx: make_ctx(),
            payload: topgun_core::messages::ClientOpMessage {
                payload: topgun_core::messages::base::ClientOp {
                    id: Some("op-remove".to_string()),
                    map_name: "users".to_string(),
                    key: "user-1".to_string(),
                    op_type: None,
                    record: Some(None), // tombstone
                    or_record: None,
                    or_tag: None,
                    write_concern: None,
                    timeout: None,
                },
            },
        };

        let resp = svc.oneshot(op).await.unwrap();
        assert!(
            matches!(resp, OperationResponse::Message(ref msg) if matches!(**msg, Message::OpAck(_))),
            "expected OpAck, got {resp:?}"
        );
    }

    // -- LWW REMOVE (op_type) --

    #[tokio::test]
    async fn lww_remove_via_op_type_returns_op_ack() {
        let svc = make_service();
        let op = Operation::ClientOp {
            ctx: make_ctx(),
            payload: topgun_core::messages::ClientOpMessage {
                payload: topgun_core::messages::base::ClientOp {
                    id: Some("op-remove-2".to_string()),
                    map_name: "users".to_string(),
                    key: "user-2".to_string(),
                    op_type: Some("REMOVE".to_string()),
                    record: None,
                    or_record: None,
                    or_tag: None,
                    write_concern: None,
                    timeout: None,
                },
            },
        };

        let resp = svc.oneshot(op).await.unwrap();
        assert!(
            matches!(resp, OperationResponse::Message(ref msg) if matches!(**msg, Message::OpAck(_))),
            "expected OpAck, got {resp:?}"
        );
    }

    // -- OR_ADD --

    #[tokio::test]
    async fn or_add_returns_op_ack() {
        let svc = make_service();
        let or_rec = topgun_core::ORMapRecord {
            value: rmpv::Value::String("important".into()),
            timestamp: make_timestamp(),
            tag: "1700000000000:1:test-node".to_string(),
            ttl_ms: None,
        };
        let op = Operation::ClientOp {
            ctx: make_ctx(),
            payload: topgun_core::messages::ClientOpMessage {
                payload: topgun_core::messages::base::ClientOp {
                    id: Some("op-or-add".to_string()),
                    map_name: "tags".to_string(),
                    key: "item-1".to_string(),
                    op_type: None,
                    record: None,
                    or_record: Some(Some(or_rec)),
                    or_tag: None,
                    write_concern: None,
                    timeout: None,
                },
            },
        };

        let resp = svc.oneshot(op).await.unwrap();
        assert!(
            matches!(resp, OperationResponse::Message(ref msg) if matches!(**msg, Message::OpAck(_))),
            "expected OpAck, got {resp:?}"
        );
    }

    // -- OR_REMOVE --

    #[tokio::test]
    async fn or_remove_returns_op_ack() {
        let svc = make_service();
        let op = Operation::ClientOp {
            ctx: make_ctx(),
            payload: topgun_core::messages::ClientOpMessage {
                payload: topgun_core::messages::base::ClientOp {
                    id: Some("op-or-remove".to_string()),
                    map_name: "tags".to_string(),
                    key: "item-1".to_string(),
                    op_type: None,
                    record: None,
                    or_record: None,
                    or_tag: Some(Some("1700000000000:1:test-node".to_string())),
                    write_concern: None,
                    timeout: None,
                },
            },
        };

        let resp = svc.oneshot(op).await.unwrap();
        assert!(
            matches!(resp, OperationResponse::Message(ref msg) if matches!(**msg, Message::OpAck(_))),
            "expected OpAck, got {resp:?}"
        );
    }

    // -- OpBatch with multiple ops --

    #[tokio::test]
    async fn op_batch_processes_all_ops_and_returns_single_ack() {
        let svc = make_service();
        let ops = vec![
            topgun_core::messages::base::ClientOp {
                id: Some("op-1".to_string()),
                map_name: "users".to_string(),
                key: "user-1".to_string(),
                op_type: None,
                record: None,
                or_record: None,
                or_tag: None,
                write_concern: None,
                timeout: None,
            },
            topgun_core::messages::base::ClientOp {
                id: Some("op-2".to_string()),
                map_name: "users".to_string(),
                key: "user-2".to_string(),
                op_type: None,
                record: None,
                or_record: None,
                or_tag: None,
                write_concern: None,
                timeout: None,
            },
            topgun_core::messages::base::ClientOp {
                id: Some("op-3".to_string()),
                map_name: "users".to_string(),
                key: "user-3".to_string(),
                op_type: None,
                record: None,
                or_record: None,
                or_tag: None,
                write_concern: None,
                timeout: None,
            },
        ];

        let op = Operation::OpBatch {
            ctx: make_ctx(),
            payload: topgun_core::messages::sync::OpBatchMessage {
                payload: topgun_core::messages::sync::OpBatchPayload {
                    ops,
                    write_concern: None,
                    timeout: None,
                },
            },
        };

        let resp = svc.oneshot(op).await.unwrap();
        match resp {
            OperationResponse::Message(msg) => match *msg {
                Message::OpAck(ack) => {
                    assert_eq!(ack.payload.last_id, "op-3", "last_id should be op-3");
                }
                other => panic!("expected OpAck, got {other:?}"),
            },
            other => panic!("expected Message, got {other:?}"),
        }
    }

    // -- Wrong service returns WrongService error --

    #[tokio::test]
    async fn wrong_service_returns_error() {
        let svc = make_service();
        let op = Operation::GarbageCollect {
            ctx: make_ctx(),
        };

        let result = svc.oneshot(op).await;
        assert!(
            matches!(result, Err(OperationError::WrongService)),
            "expected WrongService, got {result:?}"
        );
    }

    // -- OpBatch with empty ops returns Ack --

    #[tokio::test]
    async fn op_batch_empty_returns_ack() {
        let svc = make_service();
        let mut ctx = make_ctx();
        ctx.call_id = 42;
        let op = Operation::OpBatch {
            ctx,
            payload: topgun_core::messages::sync::OpBatchMessage {
                payload: topgun_core::messages::sync::OpBatchPayload {
                    ops: vec![],
                    write_concern: None,
                    timeout: None,
                },
            },
        };

        let resp = svc.oneshot(op).await.unwrap();
        assert!(
            matches!(resp, OperationResponse::Ack { call_id: 42 }),
            "expected Ack with call_id=42, got {resp:?}"
        );
    }

    // ---------------------------------------------------------------------------
    // Security integration tests (AC1, AC2, AC3, AC8, AC18, AC19, AC20)
    // ---------------------------------------------------------------------------

    fn make_strict_validator() -> Arc<WriteValidator> {
        let hlc = Arc::new(Mutex::new(HLC::new("server-node".to_string(), Box::new(SystemClock))));
        let config = SecurityConfig {
            require_auth: true,
            max_value_bytes: 0,
            ..SecurityConfig::default()
        };
        Arc::new(WriteValidator::new(Arc::new(config), hlc))
    }

    fn make_strict_service() -> (Arc<CrdtService>, Arc<ConnectionRegistry>) {
        let factory = make_factory();
        let registry = Arc::new(ConnectionRegistry::new());
        let validator = make_strict_validator();
        let query_registry = Arc::new(QueryRegistry::new());
        let svc = Arc::new(CrdtService::new(
            factory,
            Arc::clone(&registry),
            validator,
            query_registry,
            Arc::new(SchemaService::new()),
        ));
        (svc, registry)
    }

    fn make_ctx_with_conn(conn_id: ConnectionId) -> OperationContext {
        let mut ctx = make_ctx();
        ctx.connection_id = Some(conn_id);
        ctx
    }

    fn make_lww_put_op(ctx: OperationContext, map_name: &str) -> Operation {
        let record = topgun_core::LWWRecord {
            value: Some(rmpv::Value::String("value".into())),
            timestamp: make_timestamp(),
            ttl_ms: None,
        };
        Operation::ClientOp {
            ctx,
            payload: topgun_core::messages::ClientOpMessage {
                payload: topgun_core::messages::base::ClientOp {
                    id: Some("op-1".to_string()),
                    map_name: map_name.to_string(),
                    key: "key-1".to_string(),
                    op_type: None,
                    record: Some(Some(record)),
                    or_record: None,
                    or_tag: None,
                    write_concern: None,
                    timeout: None,
                },
            },
        }
    }

    // -- AC18: existing tests still pass with default (permissive) SecurityConfig --
    // (All tests above using `make_service()` use SecurityConfig::default() which is permissive)

    // -- AC19: connection_id = Some(id) but connection not found => Unauthorized --

    #[tokio::test]
    async fn missing_connection_returns_unauthorized() {
        let (svc, _registry) = make_strict_service();
        // Use a connection_id that was never registered
        let ctx = make_ctx_with_conn(ConnectionId(9999));
        let op = make_lww_put_op(ctx, "my-map");
        let result = svc.oneshot(op).await;
        assert!(
            matches!(result, Err(OperationError::Unauthorized)),
            "expected Unauthorized for missing connection, got {result:?}"
        );
    }

    // -- AC1: unauthenticated connection + require_auth => Unauthorized --

    #[tokio::test]
    async fn unauthenticated_write_rejected_when_require_auth() {
        let (svc, registry) = make_strict_service();
        let config = crate::network::config::ConnectionConfig::default();
        let (handle, _rx) = registry.register(ConnectionKind::Client, &config);
        // Connection defaults to authenticated=false

        let ctx = make_ctx_with_conn(handle.id);
        let op = make_lww_put_op(ctx, "my-map");
        let result = svc.oneshot(op).await;
        assert!(
            matches!(result, Err(OperationError::Unauthorized)),
            "expected Unauthorized for unauthenticated write, got {result:?}"
        );
    }

    // -- AC3: authenticated + write perm => Ok --

    #[tokio::test]
    async fn authenticated_write_succeeds() {
        let (svc, registry) = make_strict_service();
        let config = crate::network::config::ConnectionConfig::default();
        let (handle, _rx) = registry.register(ConnectionKind::Client, &config);
        // Mark connection as authenticated
        handle.metadata.write().await.authenticated = true;

        let ctx = make_ctx_with_conn(handle.id);
        let op = make_lww_put_op(ctx, "my-map");
        let result = svc.oneshot(op).await;
        assert!(
            matches!(result, Ok(OperationResponse::Message(_))),
            "expected OpAck for authenticated write, got {result:?}"
        );
    }

    // -- AC2: authenticated + no write perm => Forbidden --

    #[tokio::test]
    async fn no_write_permission_returns_forbidden() {
        use crate::network::connection::MapPermissions;
        let (svc, registry) = make_strict_service();
        let config = crate::network::config::ConnectionConfig::default();
        let (handle, _rx) = registry.register(ConnectionKind::Client, &config);
        {
            let mut meta = handle.metadata.write().await;
            meta.authenticated = true;
            meta.map_permissions.insert(
                "locked-map".to_string(),
                MapPermissions { read: true, write: false },
            );
        }

        let ctx = make_ctx_with_conn(handle.id);
        let op = make_lww_put_op(ctx, "locked-map");
        let result = svc.oneshot(op).await;
        assert!(
            matches!(result, Err(OperationError::Forbidden { .. })),
            "expected Forbidden for no-write-perm connection, got {result:?}"
        );
    }

    // -- AC8: batch atomic rejection: if 2nd op fails, 1st op's data not written --
    // (Verified via the validate-all-then-apply-all logic in handle_op_batch)

    #[tokio::test]
    async fn op_batch_atomic_rejection_when_second_op_fails() {
        use crate::network::connection::MapPermissions;

        let hlc = Arc::new(Mutex::new(HLC::new("server-node".to_string(), Box::new(SystemClock))));
        let config = SecurityConfig {
            require_auth: true,
            max_value_bytes: 0,
            ..SecurityConfig::default()
        };
        let validator = Arc::new(WriteValidator::new(Arc::new(config), hlc));
        let factory = make_factory();
        let registry = Arc::new(ConnectionRegistry::new());
        let query_registry = Arc::new(QueryRegistry::new());
        let svc = Arc::new(CrdtService::new(
            Arc::clone(&factory),
            Arc::clone(&registry),
            Arc::clone(&validator),
            query_registry,
            Arc::new(SchemaService::new()),
        ));

        let conn_config = crate::network::config::ConnectionConfig::default();
        let (handle, _rx) = registry.register(ConnectionKind::Client, &conn_config);
        {
            let mut meta = handle.metadata.write().await;
            meta.authenticated = true;
            // Grant write to "open-map" but deny write to "locked-map"
            meta.map_permissions.insert(
                "locked-map".to_string(),
                MapPermissions { read: true, write: false },
            );
        }

        let ctx = {
            let mut ctx = make_ctx();
            ctx.connection_id = Some(handle.id);
            ctx
        };

        let ops = vec![
            // op-1 targets "open-map" — would succeed if applied alone
            topgun_core::messages::base::ClientOp {
                id: Some("op-1".to_string()),
                map_name: "open-map".to_string(),
                key: "key-1".to_string(),
                op_type: None,
                record: None,
                or_record: None,
                or_tag: None,
                write_concern: None,
                timeout: None,
            },
            // op-2 targets "locked-map" — will fail validation
            topgun_core::messages::base::ClientOp {
                id: Some("op-2".to_string()),
                map_name: "locked-map".to_string(),
                key: "key-2".to_string(),
                op_type: None,
                record: None,
                or_record: None,
                or_tag: None,
                write_concern: None,
                timeout: None,
            },
        ];

        let op = Operation::OpBatch {
            ctx,
            payload: topgun_core::messages::sync::OpBatchMessage {
                payload: topgun_core::messages::sync::OpBatchPayload {
                    ops,
                    write_concern: None,
                    timeout: None,
                },
            },
        };

        // The batch must fail due to op-2
        let result = svc.oneshot(op).await;
        assert!(
            matches!(result, Err(OperationError::Forbidden { .. })),
            "expected Forbidden for batch with locked op, got {result:?}"
        );
    }

    // -- AC20: REMOVE ops are never rejected due to value size --

    #[tokio::test]
    async fn remove_op_not_rejected_by_size_limit() {
        let hlc = Arc::new(Mutex::new(HLC::new("server-node".to_string(), Box::new(SystemClock))));
        let config = SecurityConfig {
            require_auth: false,
            max_value_bytes: 1, // very small limit
            ..SecurityConfig::default()
        };
        let validator = Arc::new(WriteValidator::new(Arc::new(config), hlc));
        let factory = make_factory();
        let registry = Arc::new(ConnectionRegistry::new());
        let query_registry = Arc::new(QueryRegistry::new());
        let svc = Arc::new(CrdtService::new(factory, Arc::clone(&registry), validator, query_registry, Arc::new(SchemaService::new())));

        let conn_config = crate::network::config::ConnectionConfig::default();
        let (handle, _rx) = registry.register(ConnectionKind::Client, &conn_config);

        let ctx = make_ctx_with_conn(handle.id);
        // REMOVE via tombstone
        let op = Operation::ClientOp {
            ctx,
            payload: topgun_core::messages::ClientOpMessage {
                payload: topgun_core::messages::base::ClientOp {
                    id: Some("op-remove".to_string()),
                    map_name: "my-map".to_string(),
                    key: "key-1".to_string(),
                    op_type: None,
                    record: Some(None), // tombstone = REMOVE
                    or_record: None,
                    or_tag: None,
                    write_concern: None,
                    timeout: None,
                },
            },
        };
        let result = svc.oneshot(op).await;
        assert!(
            matches!(result, Ok(_)),
            "expected Ok for REMOVE op regardless of size limit, got {result:?}"
        );
    }

    // -- rmpv_to_value conversion tests --

    #[test]
    fn rmpv_to_value_nil_is_null() {
        assert_eq!(rmpv_to_value(&rmpv::Value::Nil), Value::Null);
    }

    #[test]
    fn rmpv_to_value_bool() {
        assert_eq!(rmpv_to_value(&rmpv::Value::Boolean(true)), Value::Bool(true));
        assert_eq!(rmpv_to_value(&rmpv::Value::Boolean(false)), Value::Bool(false));
    }

    #[test]
    fn rmpv_to_value_integer_signed() {
        assert_eq!(
            rmpv_to_value(&rmpv::Value::Integer((-42i64).into())),
            Value::Int(-42)
        );
    }

    #[test]
    fn rmpv_to_value_integer_unsigned_large() {
        // Value larger than i64::MAX should fall back to as_u64() -> cast to i64.
        let large: u64 = u64::MAX;
        let v = rmpv::Value::Integer(large.into());
        // as_i64() returns None for u64::MAX; as_u64() as i64 = -1.
        assert_eq!(rmpv_to_value(&v), Value::Int(-1i64));
    }

    #[test]
    fn rmpv_to_value_string() {
        let v = rmpv::Value::String("hello".into());
        assert_eq!(rmpv_to_value(&v), Value::String("hello".to_string()));
    }

    #[test]
    fn rmpv_to_value_array() {
        let v = rmpv::Value::Array(vec![
            rmpv::Value::Integer(1i64.into()),
            rmpv::Value::Boolean(true),
        ]);
        assert_eq!(
            rmpv_to_value(&v),
            Value::Array(vec![Value::Int(1), Value::Bool(true)])
        );
    }

    // ---------------------------------------------------------------------------
    // Subscription-aware broadcast tests (AC5, AC6, AC7)
    // ---------------------------------------------------------------------------

    use crate::service::domain::query::QuerySubscription;
    use dashmap::DashSet;
    use topgun_core::messages::base::Query;

    /// Helper: build a CrdtService with shared registries for broadcast testing.
    fn make_broadcast_test_setup() -> (
        Arc<CrdtService>,
        Arc<ConnectionRegistry>,
        Arc<QueryRegistry>,
    ) {
        let factory = make_factory();
        let conn_registry = Arc::new(ConnectionRegistry::new());
        let query_registry = Arc::new(QueryRegistry::new());
        let svc = Arc::new(CrdtService::new(
            factory,
            Arc::clone(&conn_registry),
            make_validator(),
            Arc::clone(&query_registry),
            Arc::new(SchemaService::new()),
        ));
        (svc, conn_registry, query_registry)
    }

    /// AC5+AC6: subscriber receives event, writer does not.
    #[tokio::test]
    async fn broadcast_sends_only_to_subscribers_and_excludes_writer() {
        let (svc, conn_registry, query_registry) = make_broadcast_test_setup();
        let config = crate::network::config::ConnectionConfig::default();

        // Register two client connections
        let (conn1_handle, mut conn1_rx) =
            conn_registry.register(ConnectionKind::Client, &config);
        let (conn2_handle, mut conn2_rx) =
            conn_registry.register(ConnectionKind::Client, &config);

        // Subscribe conn1 to "users" via QueryRegistry
        query_registry.register(QuerySubscription {
            query_id: "q-1".to_string(),
            connection_id: conn1_handle.id,
            map_name: "users".to_string(),
            query: Query {
                predicate: None,
                r#where: None,
                sort: None,
                limit: None,
                cursor: None,
            group_by: None,
            },
            previous_result_keys: DashSet::new(),
            fields: None,
        });

        // conn2 writes to "users" — conn1 should receive, conn2 should NOT
        let mut ctx = make_ctx();
        ctx.connection_id = Some(conn2_handle.id);
        let op = make_lww_put_op(ctx, "users");

        let _resp = svc.oneshot(op).await.unwrap();

        // conn1 subscribed to "users" => should receive the ServerEvent
        let msg1 = conn1_rx.try_recv();
        assert!(
            msg1.is_ok(),
            "subscriber conn1 should have received the event"
        );

        // conn2 is the writer => excluded from broadcast
        let msg2 = conn2_rx.try_recv();
        assert!(
            msg2.is_err(),
            "writer conn2 should NOT have received its own event"
        );
    }

    /// AC7: zero subscribers => no serialization, no bytes sent.
    #[tokio::test]
    async fn broadcast_skips_serialization_when_no_subscribers() {
        let (svc, conn_registry, _query_registry) = make_broadcast_test_setup();
        let config = crate::network::config::ConnectionConfig::default();

        // Register a connection but do NOT subscribe it to any map
        let (_conn_handle, mut conn_rx) =
            conn_registry.register(ConnectionKind::Client, &config);

        // Write to "orders" with zero subscribers
        let ctx = make_ctx();
        let op = make_lww_put_op(ctx, "orders");
        let _resp = svc.oneshot(op).await.unwrap();

        // No connection should receive anything
        let msg = conn_rx.try_recv();
        assert!(
            msg.is_err(),
            "no bytes should be sent when zero subscribers exist"
        );
    }

    /// AC5: non-subscriber for a different map does not receive events.
    #[tokio::test]
    async fn broadcast_does_not_leak_to_other_map_subscribers() {
        let (svc, conn_registry, query_registry) = make_broadcast_test_setup();
        let config = crate::network::config::ConnectionConfig::default();

        let (conn1_handle, mut conn1_rx) =
            conn_registry.register(ConnectionKind::Client, &config);

        // Subscribe conn1 to "products" (NOT "users")
        query_registry.register(QuerySubscription {
            query_id: "q-products".to_string(),
            connection_id: conn1_handle.id,
            map_name: "products".to_string(),
            query: Query {
                predicate: None,
                r#where: None,
                sort: None,
                limit: None,
                cursor: None,
            group_by: None,
            },
            previous_result_keys: DashSet::new(),
            fields: None,
        });

        // Write to "users" — conn1 is subscribed to "products", not "users"
        let ctx = make_ctx();
        let op = make_lww_put_op(ctx, "users");
        let _resp = svc.oneshot(op).await.unwrap();

        let msg = conn1_rx.try_recv();
        assert!(
            msg.is_err(),
            "conn1 subscribed to 'products' should not receive 'users' event"
        );
    }

    // ---------------------------------------------------------------------------
    // Schema validation tests (AC3, AC4, AC5, AC6, AC7, AC8)
    // ---------------------------------------------------------------------------

    use topgun_core::{FieldDef, FieldType, MapSchema};

    fn make_required_string_schema() -> MapSchema {
        MapSchema {
            version: 1,
            fields: vec![FieldDef {
                name: "name".to_string(),
                required: true,
                field_type: FieldType::String,
                constraints: None,
            }],
            strict: false,
        }
    }

    /// Builds a CrdtService with a SchemaService that has a schema registered for "typed-map".
    /// Also registers a client connection and returns its ID so tests can set connection_id
    /// to trigger schema validation (internal calls with no connection_id bypass it).
    async fn make_schema_service() -> (Arc<CrdtService>, Arc<ConnectionRegistry>, ConnectionId) {
        let factory = make_factory();
        let registry = Arc::new(ConnectionRegistry::new());
        let query_registry = Arc::new(QueryRegistry::new());
        let schema_svc = Arc::new(SchemaService::new());
        schema_svc
            .register_schema("typed-map", make_required_string_schema())
            .await
            .unwrap();
        let svc = Arc::new(CrdtService::new(
            factory,
            Arc::clone(&registry),
            make_validator(),
            query_registry,
            schema_svc,
        ));
        // Register a client connection so tests can use its ID as connection_id.
        let config = crate::network::config::ConnectionConfig::default();
        let (handle, _rx) = registry.register(ConnectionKind::Client, &config);
        let conn_id = handle.id;
        (svc, registry, conn_id)
    }

    fn make_ctx_with_connection(conn_id: ConnectionId) -> OperationContext {
        let mut ctx = make_ctx();
        ctx.connection_id = Some(conn_id);
        ctx
    }

    fn make_lww_put_with_value(ctx: OperationContext, map_name: &str, value: rmpv::Value) -> Operation {
        let record = topgun_core::LWWRecord {
            value: Some(value),
            timestamp: make_timestamp(),
            ttl_ms: None,
        };
        Operation::ClientOp {
            ctx,
            payload: topgun_core::messages::ClientOpMessage {
                payload: topgun_core::messages::base::ClientOp {
                    id: Some("schema-op".to_string()),
                    map_name: map_name.to_string(),
                    key: "key-1".to_string(),
                    op_type: None,
                    record: Some(Some(record)),
                    or_record: None,
                    or_tag: None,
                    write_concern: None,
                    timeout: None,
                },
            },
        }
    }

    /// AC3: PUT with valid data to schema-registered map succeeds.
    #[tokio::test]
    async fn schema_valid_put_succeeds() {
        let (svc, _registry, conn_id) = make_schema_service().await;
        // Map with required "name" field — provide it as a Map value.
        let value = rmpv::Value::Map(vec![(
            rmpv::Value::String("name".into()),
            rmpv::Value::String("Alice".into()),
        )]);
        let op = make_lww_put_with_value(make_ctx_with_connection(conn_id), "typed-map", value);
        let result = svc.oneshot(op).await;
        assert!(
            matches!(result, Ok(OperationResponse::Message(_))),
            "expected OpAck for valid data, got {result:?}"
        );
    }

    /// AC3: PUT with invalid data (missing required field) returns SchemaInvalid.
    #[tokio::test]
    async fn schema_invalid_put_rejected() {
        let (svc, _registry, conn_id) = make_schema_service().await;
        // Send an empty map — missing required "name" field.
        let value = rmpv::Value::Map(vec![]);
        let op = make_lww_put_with_value(make_ctx_with_connection(conn_id), "typed-map", value);
        let result = svc.oneshot(op).await;
        assert!(
            matches!(result, Err(OperationError::SchemaInvalid { .. })),
            "expected SchemaInvalid for missing required field, got {result:?}"
        );
    }

    /// AC5: PUT to map with no registered schema passes through (optional mode).
    #[tokio::test]
    async fn schema_no_schema_registered_passes_through() {
        let (svc, _registry, conn_id) = make_schema_service().await;
        // "untyped-map" has no registered schema — any value is valid.
        let value = rmpv::Value::String("anything".into());
        let op = make_lww_put_with_value(make_ctx_with_connection(conn_id), "untyped-map", value);
        let result = svc.oneshot(op).await;
        assert!(
            matches!(result, Ok(OperationResponse::Message(_))),
            "expected OpAck for unschema'd map, got {result:?}"
        );
    }

    /// AC7: REMOVE (tombstone) bypasses schema validation.
    #[tokio::test]
    async fn schema_remove_tombstone_bypasses_validation() {
        let (svc, _registry, conn_id) = make_schema_service().await;
        let op = Operation::ClientOp {
            ctx: make_ctx_with_connection(conn_id),
            payload: topgun_core::messages::ClientOpMessage {
                payload: topgun_core::messages::base::ClientOp {
                    id: Some("remove-op".to_string()),
                    map_name: "typed-map".to_string(),
                    key: "key-1".to_string(),
                    op_type: None,
                    record: Some(None), // tombstone REMOVE
                    or_record: None,
                    or_tag: None,
                    write_concern: None,
                    timeout: None,
                },
            },
        };
        let result = svc.oneshot(op).await;
        assert!(
            matches!(result, Ok(_)),
            "expected Ok for REMOVE op bypassing schema, got {result:?}"
        );
    }

    /// AC7: REMOVE via op_type bypasses schema validation.
    #[tokio::test]
    async fn schema_remove_via_op_type_bypasses_validation() {
        let (svc, _registry, conn_id) = make_schema_service().await;
        let op = Operation::ClientOp {
            ctx: make_ctx_with_connection(conn_id),
            payload: topgun_core::messages::ClientOpMessage {
                payload: topgun_core::messages::base::ClientOp {
                    id: Some("remove-op-2".to_string()),
                    map_name: "typed-map".to_string(),
                    key: "key-1".to_string(),
                    op_type: Some("REMOVE".to_string()),
                    record: None,
                    or_record: None,
                    or_tag: None,
                    write_concern: None,
                    timeout: None,
                },
            },
        };
        let result = svc.oneshot(op).await;
        assert!(
            matches!(result, Ok(_)),
            "expected Ok for op_type=REMOVE bypassing schema, got {result:?}"
        );
    }

    /// AC8: internal call (no connection_id) bypasses schema validation.
    #[tokio::test]
    async fn schema_internal_call_bypasses_validation() {
        let (svc, _registry, _conn_id) = make_schema_service().await;
        // No connection_id = internal/system call — validation is skipped.
        let ctx = make_ctx(); // connection_id is None
        let value = rmpv::Value::Map(vec![]); // would fail schema (missing "name")
        let op = make_lww_put_with_value(ctx, "typed-map", value);
        let result = svc.oneshot(op).await;
        assert!(
            matches!(result, Ok(_)),
            "expected Ok for internal call bypassing schema, got {result:?}"
        );
    }

    /// AC6: OpBatch with one invalid op rejects the entire batch atomically.
    #[tokio::test]
    async fn schema_op_batch_atomic_rejection_on_schema_failure() {
        let factory = make_factory();
        let registry = Arc::new(ConnectionRegistry::new());
        let query_registry = Arc::new(QueryRegistry::new());
        let schema_svc = Arc::new(SchemaService::new());
        schema_svc
            .register_schema("typed-map", make_required_string_schema())
            .await
            .unwrap();
        let hlc = Arc::new(Mutex::new(HLC::new("server-node".to_string(), Box::new(SystemClock))));
        let config = SecurityConfig {
            require_auth: false,
            ..SecurityConfig::default()
        };
        let validator = Arc::new(WriteValidator::new(Arc::new(config), hlc));
        let svc = Arc::new(CrdtService::new(
            Arc::clone(&factory),
            Arc::clone(&registry),
            validator,
            query_registry,
            Arc::clone(&schema_svc) as Arc<dyn crate::traits::SchemaProvider>,
        ));

        let conn_config = crate::network::config::ConnectionConfig::default();
        let (handle, _rx) = registry.register(ConnectionKind::Client, &conn_config);
        let mut ctx = make_ctx();
        ctx.connection_id = Some(handle.id);

        // valid map with schema-conforming data.
        let valid_value = rmpv::Value::Map(vec![(
            rmpv::Value::String("name".into()),
            rmpv::Value::String("Alice".into()),
        )]);
        // invalid: missing required "name" field.
        let invalid_value = rmpv::Value::Map(vec![]);

        let ops = vec![
            topgun_core::messages::base::ClientOp {
                id: Some("op-1".to_string()),
                map_name: "typed-map".to_string(),
                key: "key-1".to_string(),
                op_type: None,
                record: Some(Some(topgun_core::LWWRecord {
                    value: Some(valid_value),
                    timestamp: make_timestamp(),
                    ttl_ms: None,
                })),
                or_record: None,
                or_tag: None,
                write_concern: None,
                timeout: None,
            },
            topgun_core::messages::base::ClientOp {
                id: Some("op-2".to_string()),
                map_name: "typed-map".to_string(),
                key: "key-2".to_string(),
                op_type: None,
                record: Some(Some(topgun_core::LWWRecord {
                    value: Some(invalid_value),
                    timestamp: make_timestamp(),
                    ttl_ms: None,
                })),
                or_record: None,
                or_tag: None,
                write_concern: None,
                timeout: None,
            },
        ];

        let op = Operation::OpBatch {
            ctx,
            payload: topgun_core::messages::sync::OpBatchMessage {
                payload: topgun_core::messages::sync::OpBatchPayload {
                    ops,
                    write_concern: None,
                    timeout: None,
                },
            },
        };

        let result = svc.oneshot(op).await;
        assert!(
            matches!(result, Err(OperationError::SchemaInvalid { .. })),
            "expected SchemaInvalid for batch with one invalid op, got {result:?}"
        );
    }

    /// AC3: SchemaInvalid error has correct map_name and non-empty errors.
    #[tokio::test]
    async fn schema_invalid_error_contains_field_details() {
        let (svc, _registry, conn_id) = make_schema_service().await;
        let value = rmpv::Value::Map(vec![]);
        let op = make_lww_put_with_value(make_ctx_with_connection(conn_id), "typed-map", value);
        let result = svc.oneshot(op).await;
        match result {
            Err(OperationError::SchemaInvalid { map_name, errors }) => {
                assert_eq!(map_name, "typed-map");
                assert!(!errors.is_empty(), "expected at least one error message");
            }
            other => panic!("expected SchemaInvalid, got {other:?}"),
        }
    }

    fn make_lww_put_with_map_value(
        ctx: OperationContext,
        map_name: &str,
        key: &str,
        value: rmpv::Value,
    ) -> Operation {
        let record = topgun_core::LWWRecord {
            value: Some(value),
            timestamp: make_timestamp(),
            ttl_ms: None,
        };
        Operation::ClientOp {
            ctx,
            payload: topgun_core::messages::ClientOpMessage {
                payload: topgun_core::messages::base::ClientOp {
                    id: Some("op-1".to_string()),
                    map_name: map_name.to_string(),
                    key: key.to_string(),
                    op_type: None,
                    record: Some(Some(record)),
                    or_record: None,
                    or_tag: None,
                    write_concern: None,
                    timeout: None,
                },
            },
        }
    }

    fn make_rmpv_map(pairs: Vec<(&str, rmpv::Value)>) -> rmpv::Value {
        rmpv::Value::Map(
            pairs
                .into_iter()
                .map(|(k, v)| (rmpv::Value::String(k.into()), v))
                .collect(),
        )
    }

    // ---------------------------------------------------------------------------
    // broadcast_query_updates writer exclusion test
    // ---------------------------------------------------------------------------

    /// Helper: drain all QUERY_UPDATE messages from a connection receiver.
    /// Returns (change_type, key, value) triples.
    fn drain_query_updates(rx: &mut tokio::sync::mpsc::Receiver<crate::network::connection::OutboundMessage>) -> Vec<(topgun_core::messages::base::ChangeEventType, String, rmpv::Value)> {
        let mut updates = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            if let crate::network::connection::OutboundMessage::Binary(bytes) = msg {
                if let Ok(decoded) = rmp_serde::from_slice::<topgun_core::messages::Message>(&bytes) {
                    if let topgun_core::messages::Message::QueryUpdate { payload } = decoded {
                        updates.push((payload.change_type, payload.key, payload.value));
                    }
                }
            }
        }
        updates
    }

    /// AC3 (SPEC-143): QUERY_UPDATE is NOT sent to the connection that originated the write.
    /// This tests broadcast_query_updates() writer exclusion + field projection.
    #[tokio::test]
    async fn broadcast_query_updates_writer_exclusion_and_projection() {
        let (svc, conn_registry, query_registry) = make_broadcast_test_setup();
        let config = crate::network::config::ConnectionConfig::default();

        // Register two client connections
        let (writer_handle, mut writer_rx) =
            conn_registry.register(ConnectionKind::Client, &config);
        let (sub_handle, mut sub_rx) =
            conn_registry.register(ConnectionKind::Client, &config);

        // Subscribe sub_handle to "users" with field projection ["name"]
        query_registry.register(QuerySubscription {
            query_id: "q-proj".to_string(),
            connection_id: sub_handle.id,
            map_name: "users".to_string(),
            query: Query {
                predicate: None,
                r#where: None,
                sort: None,
                limit: None,
                cursor: None,
            group_by: None,
            },
            previous_result_keys: DashSet::new(),
            fields: Some(vec!["name".to_string()]),
        });

        // Writer writes to "users"
        let value = make_rmpv_map(vec![
            ("name", rmpv::Value::String("Alice".into())),
            ("age", rmpv::Value::Integer(30.into())),
        ]);
        let mut ctx = make_ctx();
        ctx.connection_id = Some(writer_handle.id);
        ctx.partition_id = Some(0);
        let op = make_lww_put_with_map_value(ctx, "users", "user-1", value);
        let _ = svc.oneshot(op).await.unwrap();

        // Drain ServerEvent messages first (both connections may get them)
        // Then look specifically for QUERY_UPDATE messages

        // Writer should NOT have received any QueryUpdate
        let writer_updates = drain_query_updates(&mut writer_rx);
        assert!(
            writer_updates.is_empty(),
            "writer should not receive QUERY_UPDATE, got {} updates",
            writer_updates.len()
        );

        // Subscriber should have received a QueryUpdate with projected fields
        let sub_updates = drain_query_updates(&mut sub_rx);
        assert_eq!(
            sub_updates.len(),
            1,
            "subscriber should receive exactly 1 QUERY_UPDATE"
        );
        let (change_type, key, value) = &sub_updates[0];
        assert_eq!(*change_type, topgun_core::messages::base::ChangeEventType::ENTER);
        assert_eq!(key, "user-1");

        // The value should be projected to only include "name"
        let map = value.as_map().expect("projected value should be a map");
        assert_eq!(map.len(), 1, "projected value should have only 1 field");
        assert_eq!(map[0].0.as_str().unwrap(), "name");
        assert_eq!(map[0].1.as_str().unwrap(), "Alice");
    }

    // -- achieved_level reporting --

    #[tokio::test]
    async fn single_op_ack_reports_applied_level() {
        let svc = make_service();
        let record = topgun_core::LWWRecord {
            value: Some(rmpv::Value::String("Alice".into())),
            timestamp: make_timestamp(),
            ttl_ms: None,
        };
        let op = Operation::ClientOp {
            ctx: make_ctx(),
            payload: topgun_core::messages::ClientOpMessage {
                payload: topgun_core::messages::base::ClientOp {
                    id: Some("op-ack-level".to_string()),
                    map_name: "users".to_string(),
                    key: "user-1".to_string(),
                    op_type: None,
                    record: Some(Some(record)),
                    or_record: None,
                    or_tag: None,
                    write_concern: None,
                    timeout: None,
                },
            },
        };

        let resp = svc.oneshot(op).await.unwrap();
        match resp {
            OperationResponse::Message(msg) => match *msg {
                Message::OpAck(ack) => {
                    assert_eq!(
                        ack.payload.achieved_level,
                        Some(WriteConcern::APPLIED),
                        "single-op ack must report APPLIED after successful CRDT merge"
                    );
                }
                other => panic!("expected OpAck, got {other:?}"),
            },
            other => panic!("expected Message, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn op_batch_ack_reports_applied_level() {
        let svc = make_service();
        let ops = vec![
            topgun_core::messages::base::ClientOp {
                id: Some("batch-op-1".to_string()),
                map_name: "items".to_string(),
                key: "item-1".to_string(),
                op_type: None,
                record: None,
                or_record: None,
                or_tag: None,
                write_concern: None,
                timeout: None,
            },
            topgun_core::messages::base::ClientOp {
                id: Some("batch-op-2".to_string()),
                map_name: "items".to_string(),
                key: "item-2".to_string(),
                op_type: None,
                record: None,
                or_record: None,
                or_tag: None,
                write_concern: None,
                timeout: None,
            },
        ];

        let op = Operation::OpBatch {
            ctx: make_ctx(),
            payload: topgun_core::messages::sync::OpBatchMessage {
                payload: topgun_core::messages::sync::OpBatchPayload {
                    ops,
                    write_concern: None,
                    timeout: None,
                },
            },
        };

        let resp = svc.oneshot(op).await.unwrap();
        match resp {
            OperationResponse::Message(msg) => match *msg {
                Message::OpAck(ack) => {
                    assert_eq!(
                        ack.payload.achieved_level,
                        Some(WriteConcern::APPLIED),
                        "batch ack must report APPLIED after all ops merged successfully"
                    );
                }
                other => panic!("expected OpAck, got {other:?}"),
            },
            other => panic!("expected Message, got {other:?}"),
        }
    }
}
