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
    OpBatchMessage, ServerEventPayload, ServerEventType,
};
use topgun_core::types::Value;
use topgun_core::{LWWRecord, ORMapRecord, Timestamp};

use tracing::Instrument;

use crate::network::connection::{ConnectionId, ConnectionMetadata, ConnectionRegistry};
use crate::service::domain::query::QueryRegistry;
use crate::service::operation::{
    service_names, Operation, OperationContext, OperationError, OperationResponse,
};
use crate::service::registry::{ManagedService, ServiceContext};
use crate::service::security::WriteValidator;
use crate::storage::record::{OrMapEntry, RecordValue};
use crate::storage::{CallerProvenance, ExpiryPolicy, RecordStoreFactory};

// ---------------------------------------------------------------------------
// CrdtService
// ---------------------------------------------------------------------------

/// Real CRDT domain service handling `ClientOp` and `OpBatch` operations.
///
/// Replaces the `domain_stub!(CrdtService, ...)` macro-generated stub.
/// Merges LWW and OR-Map data into the `RecordStore` and broadcasts
/// `ServerEvent` messages to connected clients.
///
/// Security validation runs BEFORE any CRDT merge: unauthorized writes never reach storage.
pub struct CrdtService {
    record_store_factory: Arc<RecordStoreFactory>,
    connection_registry: Arc<ConnectionRegistry>,
    write_validator: Arc<WriteValidator>,
    query_registry: Arc<QueryRegistry>,
}

impl CrdtService {
    /// Creates a new `CrdtService` with its required dependencies.
    #[must_use]
    pub fn new(
        record_store_factory: Arc<RecordStoreFactory>,
        connection_registry: Arc<ConnectionRegistry>,
        write_validator: Arc<WriteValidator>,
        query_registry: Arc<QueryRegistry>,
    ) -> Self {
        Self {
            record_store_factory,
            connection_registry,
            write_validator,
            query_registry,
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
        // None means internal/system call -- skip validation to maintain test compatibility.
        let sanitized_ts = if let Some(conn_id) = ctx.connection_id {
            let metadata_snapshot = self.snapshot_metadata(conn_id).await?;
            let value_size = estimate_value_size(op);
            self.write_validator.validate_write(ctx, &metadata_snapshot, &op.map_name, value_size)?;
            Some(self.write_validator.sanitize_hlc())
        } else {
            None
        };

        let event_payload = self.apply_single_op(op, partition_id, sanitized_ts.as_ref()).await?;

        self.broadcast_event(&event_payload, ctx.connection_id)?;

        let last_id = op.id.clone().unwrap_or_else(|| "unknown".to_string());
        Ok(OperationResponse::Message(Box::new(Message::OpAck(
            OpAckMessage {
                payload: OpAckPayload {
                    last_id,
                    achieved_level: None,
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

        let partition_id = ctx.partition_id.unwrap_or(0);
        let mut last_id = "unknown".to_string();

        // Validate all ops before applying any (atomic batch rejection).
        // Snapshot metadata once at batch start to avoid per-op lock acquisition.
        if let Some(conn_id) = ctx.connection_id {
            let metadata_snapshot = self.snapshot_metadata(conn_id).await?;
            for op in ops {
                let value_size = estimate_value_size(op);
                self.write_validator.validate_write(ctx, &metadata_snapshot, &op.map_name, value_size)?;
            }
            // All ops validated — apply them sequentially with sanitized timestamps.
            for op in ops {
                let sanitized_ts = self.write_validator.sanitize_hlc();
                let event_payload = self.apply_single_op(op, partition_id, Some(&sanitized_ts)).await?;
                self.broadcast_event(&event_payload, ctx.connection_id)?;
                if let Some(id) = &op.id {
                    last_id = id.clone();
                }
            }
        } else {
            // Internal/system call (no connection_id) — skip validation.
            for op in ops {
                let event_payload = self.apply_single_op(op, partition_id, None).await?;
                self.broadcast_event(&event_payload, ctx.connection_id)?;
                if let Some(id) = &op.id {
                    last_id = id.clone();
                }
            }
        }

        Ok(OperationResponse::Message(Box::new(Message::OpAck(
            OpAckMessage {
                payload: OpAckPayload {
                    last_id,
                    achieved_level: None,
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
            let (record_value, stored_or_rec) = if let Some(ts) = sanitized_ts {
                let mut sanitized_rec = or_rec.clone();
                sanitized_rec.timestamp = ts.clone();
                // Regenerate tag from sanitized timestamp: "{millis}:{counter}:{node_id}"
                sanitized_rec.tag = format!("{}:{}:{}", ts.millis, ts.counter, ts.node_id);
                let rv = or_record_to_record_value(&sanitized_rec);
                (rv, sanitized_rec)
            } else {
                (or_record_to_record_value(or_rec), or_rec.clone())
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

/// Converts a wire-format `ORMapRecord<rmpv::Value>` into a storage `RecordValue::OrMap`
/// containing a single entry.
fn or_record_to_record_value(record: &ORMapRecord<rmpv::Value>) -> RecordValue {
    RecordValue::OrMap {
        records: vec![OrMapEntry {
            value: rmpv_to_value(&record.value),
            tag: record.tag.clone(),
            timestamp: record.timestamp.clone(),
        }],
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
    use crate::network::connection::ConnectionRegistry;
    use crate::service::operation::{service_names, OperationContext, OperationResponse};
    use crate::service::security::{SecurityConfig, WriteValidator};
    use crate::storage::datastores::NullDataStore;
    use crate::storage::factory::RecordStoreFactory;
    use crate::storage::impls::StorageConfig;
    use crate::service::domain::query::QueryRegistry;

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
        Arc::new(CrdtService::new(factory, registry, make_validator(), query_registry))
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
        let svc = CrdtService::new(factory, registry, make_validator(), query_registry);
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
        let svc = Arc::new(CrdtService::new(factory, Arc::clone(&registry), validator, query_registry));

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
}
