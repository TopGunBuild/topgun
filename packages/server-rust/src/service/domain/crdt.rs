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
    ClientOp, ClientOpMessage, JournalEventData, JournalEventType, Message, OpAckMessage,
    OpAckPayload, OpBatchMessage, ServerEventPayload, ServerEventType, WriteConcern,
};
use topgun_core::types::Value;
use topgun_core::{hash_to_partition, LWWRecord, Timestamp};

use tracing::Instrument;

use crate::network::connection::{ConnectionId, ConnectionMetadata, ConnectionRegistry};
use crate::service::domain::journal::JournalStore;
use crate::service::domain::key_writer::KeyWriterRegistry;
use crate::service::domain::predicate::{
    evaluate_predicate, evaluate_where, value_to_rmpv, EvalContext,
};
use crate::service::domain::query::QueryRegistry;
use crate::service::operation::{
    service_names, CallerOrigin, Operation, OperationContext, OperationError, OperationResponse,
};
use crate::service::registry::{ManagedService, ServiceContext};
use crate::service::security::WriteAdmission;
use crate::storage::record::{OrMapEntry, RecordValue};
use crate::storage::{CallerProvenance, ExpiryPolicy, RecordStoreFactory};
use crate::tombstone_frontier_impl::TombstoneFrontier;
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
/// Validation order: auth/size (`WriteAdmission`) → schema (`SchemaProvider`) → CRDT merge.
pub struct CrdtService {
    record_store_factory: Arc<RecordStoreFactory>,
    connection_registry: Arc<ConnectionRegistry>,
    write_validator: Arc<WriteAdmission>,
    query_registry: Arc<QueryRegistry>,
    schema_provider: Arc<dyn SchemaProvider>,
    /// Optional Event Journal sink. When present (production wiring), every
    /// applied mutation is appended to the shared `JournalStore` and pushed to
    /// matching `JournalSubscribe` connections as a `JOURNAL_EVENT`. `None` in
    /// unit tests that do not exercise the journal — `record_journal` is then a
    /// no-op, so the journal never perturbs CRDT-only test behaviour.
    journal: Option<Arc<JournalStore>>,
    /// Per-KEY single-writer registry. Serializes the `OR_ADD` apply RMW
    /// (`store.get` -> merge -> `store.put`) so concurrent `OR_ADD`s on the
    /// SAME key cannot both read the pre-mutation state and race to `put`,
    /// which would silently drop one add (SPEC-333b lost-update race).
    /// Internal-only: not exposed via `new()` so existing call sites are
    /// unaffected.
    ///
    /// Correctness precondition: this registry is owned per `CrdtService`, and a
    /// `CrdtService` is 1:1 with its backing store (`record_store_factory`) — one
    /// service is constructed over each store and registered as the router's sole
    /// CRDT handle. Serialization therefore covers every `OR_ADD` that can reach
    /// that store. If a future deployment ever constructs a SECOND `CrdtService`
    /// over the SAME store (e.g. in-process sharding), it MUST share this same
    /// registry — two registries over one store would mint distinct mutexes per
    /// key and reopen the lost-update race.
    key_writer: Arc<KeyWriterRegistry>,
    /// Optional shared causal frontier. When present (production wiring), each
    /// genuinely-new tombstone is stamped with the current server epoch at
    /// `OR_REMOVE` apply, and the wholesale epoch-drop prune is run over the OR
    /// write path. `None` in unit tests that do not exercise epoch stamping — the
    /// stamp/prune then no-op. This MUST be the SAME `Arc<TombstoneFrontier>`
    /// held in `AppState` and shared with `SyncService`, so the epoch counter and
    /// the low-water-mark it reads are one authority (a second frontier would
    /// stamp epochs no client ever ACKs).
    frontier: Option<Arc<TombstoneFrontier>>,
}

impl CrdtService {
    /// Creates a new `CrdtService` with its required dependencies.
    #[must_use]
    pub fn new(
        record_store_factory: Arc<RecordStoreFactory>,
        connection_registry: Arc<ConnectionRegistry>,
        write_validator: Arc<WriteAdmission>,
        query_registry: Arc<QueryRegistry>,
        schema_provider: Arc<dyn SchemaProvider>,
    ) -> Self {
        Self {
            record_store_factory,
            connection_registry,
            write_validator,
            query_registry,
            schema_provider,
            journal: None,
            key_writer: Arc::new(KeyWriterRegistry::new()),
            frontier: None,
        }
    }

    /// Attaches the shared Event Journal sink, enabling write-path journaling.
    ///
    /// Production wiring calls this with the same `Arc<JournalStore>` held by the
    /// `PersistenceService` so appended events are readable via `JournalRead`.
    #[must_use]
    pub fn with_journal(mut self, journal: Arc<JournalStore>) -> Self {
        self.journal = Some(journal);
        self
    }

    /// Attaches the shared causal frontier, enabling server-authoritative epoch
    /// stamping at `OR_REMOVE` apply and the dark wholesale prune over the OR
    /// write path. Production wiring MUST pass the SAME `Arc<TombstoneFrontier>`
    /// held in `AppState` and shared with `SyncService`.
    #[must_use]
    pub fn with_frontier(mut self, frontier: Arc<TombstoneFrontier>) -> Self {
        self.frontier = Some(frontier);
        self
    }

    /// Replaces the internal per-key writer with a SHARED registry, so a prune
    /// sweep run from `SyncService` and an `OR` write run here serialize per key
    /// against each other. Production wiring passes the SAME
    /// `Arc<KeyWriterRegistry>` into both services; without sharing, a SYNC-leaf
    /// prune and an OR write on the same key would mint distinct mutexes and race.
    #[must_use]
    pub fn with_key_writer(mut self, key_writer: Arc<KeyWriterRegistry>) -> Self {
        self.key_writer = key_writer;
        self
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
                    Operation::ClientOp { ctx, payload } => {
                        svc.handle_client_op(&ctx, payload).await
                    }
                    Operation::OpBatch { ctx, payload } => svc.handle_op_batch(&ctx, payload).await,
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
            self.write_validator
                .admit_write(ctx, &metadata_snapshot, &op.map_name, value_size)?;
            // Schema validation runs after auth/size admission checks.
            self.validate_schema_for_op(op)?;
            Some(self.write_validator.sanitize_hlc())
        } else if ctx.caller_origin == CallerOrigin::HttpClient {
            // HTTP /sync carries no per-connection handle, but the JWT-validated
            // identity is on ctx.principal (set eagerly by the HTTP handler before
            // dispatch). Derive an honest authenticated flag from it so admit_write's
            // auth gate passes for legitimate writes and fail-closes if a principal is
            // ever absent. Then re-stamp the HLC so a forged client
            // timestamp cannot win Last-Write-Wins forever.
            let metadata_snapshot = ConnectionMetadata {
                authenticated: ctx.principal.is_some(),
                principal: ctx.principal.clone(),
                ..Default::default()
            };
            let value_size = estimate_value_size(op);
            self.write_validator
                .admit_write(ctx, &metadata_snapshot, &op.map_name, value_size)?;
            self.validate_schema_for_op(op)?;
            Some(self.write_validator.sanitize_hlc())
        } else if ctx.caller_origin == CallerOrigin::Anonymous {
            // Anonymous HTTP /sync write (no connection_id, no JWT identity).
            // Auth admission for this path is enforced at the HTTP handler
            // (require_auth / enforce_auth) BEFORE dispatch, so admit_write is not
            // re-run here — it would unconditionally reject Anonymous and break the
            // no-auth dev/demo tier. The HLC, however, is still client-supplied and
            // MUST be re-stamped: otherwise a forged millis:u64::MAX wins
            // Last-Write-Wins forever and survives a later auth upgrade. HLC
            // re-stamp is an integrity control gated on transport (client/http),
            // not on auth state.
            self.validate_schema_for_op(op)?;
            Some(self.write_validator.sanitize_hlc())
        } else {
            // Genuine internal/system/forwarded call (trusted origin) — preserve
            // the caller's HLC so cross-node convergence is not perturbed.
            None
        };

        // Deliberately NO forgotten-client gate on the op path. Client-originated
        // tags are regenerated server-side above, so a pruned tombstone's tag can
        // never be re-presented here — the path is resurrection-proof by
        // construction and a gate protects nothing. Worse, a gate keyed on the
        // frontier's "unknown == forgotten" would silently drop writes from every
        // device that has not yet completed its first ACK round (a fresh device
        // flushing its pending oplog on connect), while still returning OP_ACK —
        // the client then clears the op from its local oplog and the write is
        // permanently lost on both sides. The verbatim-tag path (ORMapPushDiff)
        // keeps its load-bearing gate.

        // Read old value before mutation for query broadcast filtering.
        let old_rmpv_value = self
            .read_old_value_for_queries(&op.map_name, &op.key, partition_id)
            .await;

        let event_payload = self
            .apply_single_op(op, partition_id, sanitized_ts.as_ref())
            .await?;

        self.broadcast_event(&event_payload, ctx.connection_id)?;
        self.broadcast_query_updates(&event_payload, old_rmpv_value.as_ref(), ctx.connection_id);
        self.record_journal(&event_payload, sanitized_ts.as_ref());

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
    #[allow(clippy::too_many_lines)]
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
                self.write_validator.admit_write(
                    ctx,
                    &metadata_snapshot,
                    &op.map_name,
                    value_size,
                )?;
                // Schema validation runs after auth/ACL/size checks, before any apply.
                self.validate_schema_for_op(op)?;
            }
            // All ops validated — apply them sequentially with sanitized timestamps.
            // Each op gets its own partition based on its key (OpBatch ctx has
            // partition_id=None because the batch contains keys for many partitions).
            for op in ops {
                let sanitized_ts = self.write_validator.sanitize_hlc();
                self.apply_batch_op(op, Some(&sanitized_ts), ctx.connection_id)
                    .await?;
                if let Some(id) = &op.id {
                    last_id = id.clone();
                }
            }
        } else if ctx.caller_origin == CallerOrigin::HttpClient {
            // HTTP /sync batch: no per-connection handle, but the JWT-validated
            // identity is on ctx.principal (set eagerly by the HTTP handler before
            // dispatch). Snapshot an honest authenticated flag from it once at batch
            // start so admit_write's auth gate passes for legitimate writes and
            // fail-closes if a principal is ever absent. Then re-stamp every op's HLC so
            // a forged client timestamp cannot win Last-Write-Wins forever.
            let metadata_snapshot = ConnectionMetadata {
                authenticated: ctx.principal.is_some(),
                principal: ctx.principal.clone(),
                ..Default::default()
            };
            for op in ops {
                let value_size = estimate_value_size(op);
                self.write_validator.admit_write(
                    ctx,
                    &metadata_snapshot,
                    &op.map_name,
                    value_size,
                )?;
                // Schema validation runs after auth/ACL/size checks, before any apply.
                self.validate_schema_for_op(op)?;
            }
            // All ops validated — apply them sequentially with sanitized timestamps.
            for op in ops {
                let sanitized_ts = self.write_validator.sanitize_hlc();
                self.apply_batch_op(op, Some(&sanitized_ts), ctx.connection_id)
                    .await?;
                if let Some(id) = &op.id {
                    last_id = id.clone();
                }
            }
        } else if ctx.caller_origin == CallerOrigin::Anonymous {
            // Anonymous HTTP /sync batch (no connection_id, no JWT identity). Auth
            // admission is enforced at the HTTP handler before dispatch, so
            // admit_write is not re-run here (it would reject Anonymous and break
            // the no-auth tier). Every op's client-supplied HLC is still re-stamped
            // so a forged timestamp cannot win Last-Write-Wins — integrity gated on
            // transport, not auth.
            for op in ops {
                self.validate_schema_for_op(op)?;
            }
            for op in ops {
                let sanitized_ts = self.write_validator.sanitize_hlc();
                self.apply_batch_op(op, Some(&sanitized_ts), ctx.connection_id)
                    .await?;
                if let Some(id) = &op.id {
                    last_id = id.clone();
                }
            }
        } else {
            // Genuine internal/system/forwarded call (trusted origin) — preserve
            // the caller's HLC so cross-node convergence is not perturbed.
            for op in ops {
                self.apply_batch_op(op, None, ctx.connection_id).await?;
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

    /// This path deliberately carries no forgotten-client gate on OR-bearing ops
    /// arriving over a direct client connection (see the inline comment above the
    /// admission check inside this function's caller for the full reasoning):
    /// client-originated tags are server-regenerated below, so a pruned
    /// tombstone's tag can never be re-presented here, and gating on
    /// unknown-connection == forgotten would silently drop first-sync writes from
    /// any device that has not yet completed its first ACK round while still
    /// returning `OP_ACK` — turning that ack into permanent client-side data loss.
    /// The verbatim-tag `ORMapPushDiff` sync path keeps the load-bearing gate
    /// instead.
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

        let is_remove = op.op_type.as_deref() == Some("REMOVE") || matches!(&op.record, Some(None));

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

            // Serialize the compound read-modify-write per key: without this, two
            // concurrent OR_ADDs on the SAME key could each read the pre-mutation
            // state below and race to `store.put`, with the second `put` silently
            // clobbering the first's merge and losing an update (SPEC-333b). Held
            // across `store.get` through the single `store.put` merge-commit only —
            // does NOT cover the OR_REMOVE RMW below (342b's responsibility).
            let key_guard = self.key_writer.acquire(&op.map_name, &op.key).await;

            // Merge the new entry into the resident OR-Map slot IN PLACE rather
            // than reading a full clone, rebuilding, and re-putting the whole
            // ~130 KB snapshot every op. OR-Map add-wins semantics require all
            // concurrent additions to be preserved, while observed-remove
            // semantics require an already-tombstoned tag to stay suppressed
            // (remove-wins: no resurrection). The algebra below matches core-rust
            // ORMap (retain survivors, skip tombstoned re-adds) exactly; it is
            // inlined here rather than wiring core-rust ORMap because that
            // primitive owns its own HLC + Merkle and is keyed map-wide.
            //
            // `Option::take` moves the entry into the closure without a clone; the
            // closure runs exactly once (per key, under the writer lock above).
            let mut new_entry_opt = Some(new_entry);
            let mut merge_add = move |value: &mut RecordValue| {
                // Upgrade a legacy non-OrMap resident slot (an OrTombstones blob
                // from an older server) to the unified OrMap shape first, matching
                // the prior get -> read_or_map_state -> put path — otherwise the add
                // is dropped and the legacy blob re-persisted unchanged.
                normalize_to_or_map(value);
                if let RecordValue::OrMap {
                    records,
                    tombstones,
                } = value
                {
                    let entry = new_entry_opt
                        .take()
                        .expect("OR_ADD merge closure runs exactly once");
                    // Remove-wins: a tag already observed-removed is never resurrected.
                    if !tombstones.contains(&entry.tag) {
                        // Remove any existing entry with the same tag (idempotent re-add).
                        records.retain(|e| e.tag != entry.tag);
                        records.push(entry);
                    }
                }
                // Match the prior path, which always re-persisted the slot even
                // when remove-wins suppressed the add.
                true
            };
            store
                .update_in_place(
                    &op.key,
                    Some(RecordValue::OrMap {
                        records: Vec::new(),
                        tombstones: Vec::new(),
                    }),
                    ExpiryPolicy::NONE,
                    CallerProvenance::CrdtMerge,
                    &mut merge_add,
                )
                .await
                .map_err(OperationError::Internal)?;

            // Release the per-key writer lock the instant the merge-commit `put`
            // returns: the critical region is exactly `store.get` -> `store.put`.
            // The payload construction below only clones already-owned locals and
            // touches no shared store state, so holding the lock across it would
            // needlessly serialize unrelated writers to this key.
            drop(key_guard);

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

            // OR_REMOVE is tag-based; no timestamp sanitization needed.
            // Serialize the OR_REMOVE RMW per key (the same primitive as OR_ADD) so
            // the tombstone append and any concurrent prune sweep on this key preserve
            // tombstone-set monotonicity (no pruned tag flickering back in mid-window).
            let key_guard = self.key_writer.acquire(&op.map_name, &op.key).await;

            // Read-modify-write over the unified OrMap shape IN PLACE: drop only the
            // matched tag from records (preserving every concurrent survivor) and
            // append the removed tag to the tombstone set, mutating the resident
            // slot rather than cloning + rebuilding + re-putting the whole snapshot.
            // Writing the legacy destructive OrTombstones blob here would clobber
            // all concurrent records, which is the data-loss bug.
            let mut stamped_new_tombstone = false;
            {
                let mut apply_remove = |value: &mut RecordValue| {
                    // Upgrade a legacy OrTombstones blob to OrMap first (see OR_ADD);
                    // otherwise the tombstone append is dropped on the upgrade path.
                    normalize_to_or_map(value);
                    if let RecordValue::OrMap {
                        records,
                        tombstones,
                    } = value
                    {
                        records.retain(|e| e.tag != *tag);
                        // Dedup: a re-issued remove must not duplicate the tombstone.
                        // Only a genuinely-new tag is counted and epoch-stamped.
                        if !tombstones.contains(tag) {
                            tombstones.push(tag.clone());
                            // Feeds the residency-independent soak leak gauge. Counted
                            // here, atomically with the resident push (under the engine's
                            // per-key lock) rather than after the durable write, so a
                            // failed write + client retry counts the tag exactly once
                            // (the retry sees it already resident): a post-write
                            // increment would miss it on retry and later underflow the
                            // gauge on prune. Eviction/rehydration never move this number.
                            crate::storage::record::add_tombstone_bytes(tag.len() as u64);
                            stamped_new_tombstone = true;
                        }
                    }
                    // Match the prior path, which always re-persisted the slot even
                    // for a duplicate (already-tombstoned) remove.
                    true
                };
                store
                    .update_in_place(
                        &op.key,
                        Some(RecordValue::OrMap {
                            records: Vec::new(),
                            tombstones: Vec::new(),
                        }),
                        ExpiryPolicy::NONE,
                        CallerProvenance::CrdtMerge,
                        &mut apply_remove,
                    )
                    .await
                    .map_err(OperationError::Internal)?;
            }

            // Stamp the genuinely-new tombstone with the current server epoch —
            // server-authoritative, derived from the epoch counter, NEVER from the
            // client tag's `millis`. The wire `tombstones: Vec<String>` byte layout is
            // unchanged; the epoch lives only in the server-side frontier index.
            if stamped_new_tombstone {
                if let Some(frontier) = self.frontier.as_ref() {
                    frontier.stamp_tombstone(&op.map_name, &op.key, tag);
                }
            }

            // Release the per-key writer before the prune sweep re-acquires it: the
            // sweep takes the same per-key writer per dropped tag, so holding it here
            // would self-deadlock on this key.
            drop(key_guard);

            // Wholesale epoch-drop prune over the OR write path. Active whenever the
            // low-water mark has advanced past a stamped epoch AND the durable epoch
            // watermark has caught up to it — both conjuncts move forward as tracked
            // clients confirm-apply and the durable backend flushes, so this drains
            // real tombstones once those two conditions line up.
            if let Some(frontier) = self.frontier.as_ref() {
                prune_epoch_tombstones(frontier, &self.record_store_factory, &self.key_writer)
                    .await;
            }

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
                    .put(
                        &op.key,
                        record_value,
                        ExpiryPolicy::NONE,
                        CallerProvenance::CrdtMerge,
                    )
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

    /// Applies one op from an `OpBatch` and runs the standard write fanout:
    /// server-event broadcast, live-query updates, and journal recording.
    ///
    /// Partition is derived from the key because a batch spans many partitions.
    /// Centralizes what were four byte-identical loop bodies (one per caller
    /// origin); the per-origin validation that precedes the apply loop stays at
    /// the call site.
    async fn apply_batch_op(
        &self,
        op: &ClientOp,
        sanitized_ts: Option<&Timestamp>,
        exclude_connection_id: Option<ConnectionId>,
    ) -> Result<(), OperationError> {
        let partition_id = hash_to_partition(&op.key);
        // Read old value before mutation for query broadcast filtering.
        let old_rmpv_value = self
            .read_old_value_for_queries(&op.map_name, &op.key, partition_id)
            .await;
        let event_payload = self.apply_single_op(op, partition_id, sanitized_ts).await?;
        self.broadcast_event(&event_payload, exclude_connection_id)?;
        self.broadcast_query_updates(
            &event_payload,
            old_rmpv_value.as_ref(),
            exclude_connection_id,
        );
        self.record_journal(&event_payload, sanitized_ts);
        Ok(())
    }

    /// Appends an applied mutation to the Event Journal and pushes a
    /// `JOURNAL_EVENT` to every matching `JournalSubscribe` connection.
    ///
    /// Called on the write path immediately after `apply_single_op`, in apply
    /// order, so the journal's monotonic sequence reflects mutation order. The
    /// append always happens (so `JournalRead` sees history even with no live
    /// subscribers); serialization + push happen only when subscribers exist.
    /// A no-op when no journal is attached (unit tests) or the journal is
    /// disabled via `TOPGUN_JOURNAL_ENABLED=false`.
    ///
    /// Push delivery is best-effort (at-most-once): a subscriber on a closed or
    /// backpressured channel may miss the live event. The event is still in the
    /// ring buffer, so subscribers recover gaps via `JournalRead`/`readFrom`.
    ///
    /// `value` is captured from the applied record; `previous_value` is not yet
    /// populated (reserved — capturing it requires a dedicated pre-read on the
    /// hot path, tracked as a follow-up). Event type collapses the CRDT op kind
    /// to the journal's coarser vocabulary: `PUT`/`OR_ADD` → `PUT`,
    /// `REMOVE`/`OR_REMOVE` → `DELETE`.
    fn record_journal(&self, event_payload: &ServerEventPayload, sanitized_ts: Option<&Timestamp>) {
        let Some(journal) = self.journal.as_ref() else {
            return;
        };
        if !journal.is_enabled() {
            return;
        }

        let event_type = match event_payload.event_type {
            ServerEventType::PUT | ServerEventType::OR_ADD => JournalEventType::PUT,
            ServerEventType::REMOVE | ServerEventType::OR_REMOVE => JournalEventType::DELETE,
        };

        let (value, ts) = match event_payload.event_type {
            ServerEventType::PUT => (
                event_payload.record.as_ref().and_then(|r| r.value.clone()),
                event_payload.record.as_ref().map(|r| r.timestamp.clone()),
            ),
            ServerEventType::OR_ADD => (
                event_payload.or_record.as_ref().map(|r| r.value.clone()),
                event_payload
                    .or_record
                    .as_ref()
                    .map(|r| r.timestamp.clone()),
            ),
            ServerEventType::REMOVE | ServerEventType::OR_REMOVE => (None, None),
        };

        // Prefer the stored record's HLC; fall back to the sanitized server HLC
        // (removes carry no record); last-resort zero stamp for trusted internal
        // removes that supply neither.
        let timestamp = ts.or_else(|| sanitized_ts.cloned()).unwrap_or(Timestamp {
            millis: 0,
            counter: 0,
            node_id: String::new(),
        });
        let node_id = timestamp.node_id.clone();

        let mut entry = JournalEventData {
            sequence: String::new(),
            event_type: event_type.clone(),
            map_name: event_payload.map_name.clone(),
            key: event_payload.key.clone(),
            value,
            previous_value: None,
            timestamp,
            node_id,
            metadata: None,
        };

        let seq = journal.append(entry.clone());

        let subscribers = journal.subscribers_for(&event_payload.map_name, &event_type);
        if subscribers.is_empty() {
            return;
        }

        entry.sequence = seq.to_string();
        let msg = Message::JournalEvent { event: entry };
        match rmp_serde::to_vec_named(&msg) {
            Ok(bytes) => {
                let ids: std::collections::HashSet<ConnectionId> =
                    subscribers.into_iter().collect();
                self.connection_registry.send_to_connections(&ids, &bytes);
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to serialize JOURNAL_EVENT");
            }
        }
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
        let has_queries = !self
            .query_registry
            .get_subscriptions_for_map(map_name)
            .is_empty();
        if !has_queries {
            return None;
        }

        let store = self
            .record_store_factory
            .get_or_create(map_name, partition_id);
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
    /// - Routes each standing subscription's mutation through its `LiveWindow`, which is the
    ///   single authoritative top-N algorithm and returns the complete delta set (ENTER,
    ///   displacement LEAVE, promotion ENTER, in-window UPDATE, delete LEAVE)
    /// - Suppresses deltas for rows outside the subscription's active cursor page
    /// - Applies field projection if the subscription has `fields`
    /// - Skips the writing connection (writer exclusion)
    /// - Mirrors window membership into `previous_result_keys` for `on_remove`/`on_clear`/Merkle
    ///
    /// The old value is no longer needed for event derivation (the window tracks prior
    /// membership), so `_old_rmpv_value` is retained only for call-site signature stability.
    fn broadcast_query_updates(
        &self,
        event_payload: &ServerEventPayload,
        _old_rmpv_value: Option<&rmpv::Value>,
        exclude_connection_id: Option<ConnectionId>,
    ) {
        let subs = self
            .query_registry
            .get_subscriptions_for_map(&event_payload.map_name);
        if subs.is_empty() {
            return;
        }

        // Extract the new value from the event payload.
        let new_rmpv_value: Option<rmpv::Value> =
            event_payload.record.as_ref().and_then(|r| r.value.clone());

        for sub in &subs {
            // Skip the writing connection so it does not receive its own updates.
            if let Some(exclude_id) = exclude_connection_id {
                if sub.connection_id == exclude_id {
                    continue;
                }
            }

            // The predicate result the window needs for this mutation. A delete
            // (`new_rmpv_value == None`) is a non-match, modelled by passing `None` through.
            let new_matches = new_rmpv_value
                .as_ref()
                .is_some_and(|v| matches_query_predicate(&sub.query, v));

            // Single shared top-N algorithm: the window returns the COMPLETE delta set for
            // this mutation — the new ENTER, any displacement LEAVE, any promotion ENTER, an
            // in-window UPDATE, or a LEAVE for deletes/predicate-false rows. An empty result
            // naturally sends nothing, so no `(false,false)` short-circuit is needed.
            let deltas = sub.live_window.apply_mutation(
                &event_payload.key,
                new_rmpv_value.as_ref(),
                new_matches,
            );

            // Decode this subscription's active page bound once (if any) so out-of-page
            // deltas can be suppressed. `sub.query.cursor` is the only cursor state reachable
            // from the subscription here; a row strictly before the cursor is on an earlier
            // page the subscriber is not currently observing.
            let page_cursor = sub
                .query
                .cursor
                .as_deref()
                .and_then(crate::query::cursor::decode_cursor);

            for delta in deltas {
                // Cursor out-of-window filter: drop deltas whose row falls outside this
                // subscription's active page. LEAVE carries Nil (no row value to test), so it
                // is always delivered — the subscriber must drop a row it may currently hold.
                if let Some(ref cursor) = page_cursor {
                    if !matches!(
                        delta.event,
                        topgun_core::messages::base::ChangeEventType::LEAVE
                    ) && !crate::query::cursor::is_after_cursor(&delta.key, &delta.value, cursor)
                    {
                        continue;
                    }
                }

                // Mirror window membership into `previous_result_keys`, which is still the
                // source of truth for on_remove/on_clear/on_reset and Merkle init.
                match delta.event {
                    topgun_core::messages::base::ChangeEventType::ENTER => {
                        sub.previous_result_keys.insert(delta.key.clone());
                    }
                    topgun_core::messages::base::ChangeEventType::LEAVE => {
                        sub.previous_result_keys.remove(&delta.key);
                    }
                    topgun_core::messages::base::ChangeEventType::UPDATE => {}
                }

                // Apply field projection to ENTER/UPDATE values; LEAVE carries Nil.
                let value = if matches!(
                    delta.event,
                    topgun_core::messages::base::ChangeEventType::LEAVE
                ) {
                    rmpv::Value::Nil
                } else if let Some(ref fields) = sub.fields {
                    super::query::project_fields(fields, &delta.value)
                } else {
                    delta.value.clone()
                };

                let payload = topgun_core::messages::client_events::QueryUpdatePayload {
                    query_id: sub.query_id.clone(),
                    key: delta.key.clone(),
                    value,
                    change_type: delta.event,
                };
                let msg = topgun_core::messages::Message::QueryUpdate { payload };
                if let Ok(bytes) = rmp_serde::to_vec_named(&msg) {
                    use crate::network::connection::OutboundMessage;
                    if let Some(handle) = self.connection_registry.get(sub.connection_id) {
                        let _ = handle.try_send_broadcast(OutboundMessage::Binary(bytes));
                    }
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
        let is_remove = op.op_type.as_deref() == Some("REMOVE") || matches!(&op.record, Some(None));
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
                        rmpv::Value::String(s) => s.as_str().unwrap_or("").to_string(),
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
    let value = record.value.as_ref().map_or(Value::Null, rmpv_to_value);
    RecordValue::Lww {
        value,
        timestamp: record.timestamp.clone(),
    }
}

/// Reads the current OR-Map state (active records + observed-remove tombstones)
/// from a stored value, normalizing every prior storage shape into the unified pair.
///
/// Folds the retained read-only `OrTombstones` legacy blob into the tombstone set on
/// read so legacy records participate in remove-wins on first touch (a subsequent
/// add then correctly sees the tombstones and does not resurrect a removed tag).
/// An absent or non-OR value yields empty state.
fn read_or_map_state(value: Option<RecordValue>) -> (Vec<OrMapEntry>, Vec<String>) {
    match value {
        Some(RecordValue::OrMap {
            records,
            tombstones,
        }) => (records, tombstones),
        Some(RecordValue::OrTombstones { tags }) => (Vec::new(), tags),
        _ => (Vec::new(), Vec::new()),
    }
}

/// Normalize a resident slot to the unified `OrMap` shape in place before an
/// in-place OR merge. A legacy `OrTombstones` blob persisted by an older server
/// is converted to `OrMap { records: [], tombstones: tags }`, exactly as the
/// prior get -> `read_or_map_state` -> put path did; a slot that is already
/// `OrMap` is left untouched. Without this the in-place merge closure would fail
/// its `OrMap` pattern match, silently drop the mutation, and re-persist the
/// legacy blob unchanged — losing an acked write on the upgrade path.
fn normalize_to_or_map(value: &mut RecordValue) {
    if !matches!(value, RecordValue::OrMap { .. }) {
        let (records, tombstones) = read_or_map_state(Some(std::mem::replace(
            value,
            RecordValue::OrMap {
                records: Vec::new(),
                tombstones: Vec::new(),
            },
        )));
        *value = RecordValue::OrMap {
            records,
            tombstones,
        };
    }
}

/// Order-independent semantic view of an OR-Map slot: the live `(tag, value)`
/// set and the tombstone set, each canonicalized by sorting, so two slots that
/// hold the same CRDT state but in a different `Vec` order compare equal.
///
/// This is the equivalence oracle for the planned delta-fold recovery path (see
/// [`crate::storage::wal::OrDeltaFold`]): the differential recovery test folds
/// a random OR op sequence through BOTH the delta path and the full-snapshot path
/// and asserts their views are equal under this type.
///
/// Values are canonicalized to their `{:?}` debug string rather than kept as
/// `Value`, so equality on the view is a **total, reflexive** relation. A raw
/// `Value` is only `PartialEq`, and a float `NaN` is not equal to itself — a view
/// holding a `NaN`-valued entry would then compare unequal to its own recovery,
/// producing a false-positive "data loss" signal in the differential test.
/// Debug-string canonicalization removes that hole (`NaN` maps to the same
/// string as itself). This relies on `Value::Debug` being injective across live
/// variants (distinct values → distinct strings), which holds for the current
/// `Value` enum; if a future variant summarizes or truncates in `Debug`, the
/// oracle must switch to a dedicated canonical key on `Value` instead.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(
    dead_code,
    reason = "recovery-equivalence oracle for the delta-fold seam; the differential \
              recovery test is the first consumer — defined here as the \
              interface, not yet wired to a recovery path"
)]
pub(crate) struct OrMapSemanticView {
    /// Live entries as `(tag, canonical-value-string)` pairs, sorted, for a
    /// canonical, order-independent comparison. The value is its `{:?}` string so
    /// the relation stays reflexive under float `NaN` (see the type doc). The HLC
    /// timestamp is excluded: a tag is unique per add, so `(tag, value)` already
    /// identifies a survivor, and the fold must reproduce the same survivors
    /// regardless of insertion order.
    pub live: Vec<(String, String)>,
    /// Observed-remove tombstone tags, sorted.
    pub tombstones: Vec<String>,
}

/// Extract the [`OrMapSemanticView`] equivalence key from any OR-carrying
/// `RecordValue`.
///
/// ## Recovery-equivalence invariant decision: SEMANTIC-SET (not byte-for-byte)
///
/// The resident `RecordValue::OrMap` is NOT canonically ordered. Evidence from
/// the OR write path in this file:
///
/// - `OR_ADD` builds `records` with `records.retain(|e| e.tag != new_entry.tag)`
///   then `records.push(new_entry)` — the vector is in **operation-insertion
///   order**, never sorted.
/// - `OR_REMOVE` appends with `tombstones.push(tag.clone())` — also insertion order.
/// - The prune path (`prune_epoch_tombstones`) `retain`s in place, preserving
///   whatever order was there.
/// - `storage/record.rs` declares `records: Vec<OrMapEntry>` / `tombstones:
///   Vec<String>` with no ordering invariant, and OR-Map cross-node convergence
///   is set-based (add-wins / remove-wins), so no canonical byte ordering is
///   required or guaranteed anywhere.
///
/// A delta-fold could therefore reconstruct a semantically-identical slot whose
/// `Vec` order differs from the full-snapshot slot, so **byte-for-byte equality
/// would be a false-positive "data loss" signal** and is rejected as the
/// invariant. The testable invariant the differential test asserts is
/// semantic-set equivalence: same live `(tag, value)` set, same tombstone set
/// (and, for a prune, the same pruned-tag set — observable as the removed
/// tombstones). Byte-for-byte would only be defensible if a canonical ordering
/// were imposed on the resident representation, which today it is not.
#[allow(
    dead_code,
    reason = "equivalence oracle for the delta-fold seam; first consumed by the \
              differential recovery test — the interface is defined here only"
)]
pub(crate) fn or_map_semantic_view(value: Option<RecordValue>) -> OrMapSemanticView {
    let (records, mut tombstones) = read_or_map_state(value);
    let mut live: Vec<(String, String)> = records
        .into_iter()
        .map(|e| (e.tag, format!("{:?}", e.value)))
        .collect();
    live.sort();
    tombstones.sort();
    OrMapSemanticView { live, tombstones }
}

/// Run the wholesale epoch-drop prune over the storage backing `factory`.
///
/// Drains every currently prune-eligible epoch's tombstone refs out of the
/// frontier index (BOTH call-site conjuncts —
/// `is_epoch_prune_eligible(E) && durable_epoch_watermark >= E`) and drops each
/// tag from its OR-Map record in storage (RAM + redb) under the per-key writer,
/// so a concurrent OR write on the same key cannot flicker a pruned tag back in.
///
/// The real gate is the conjunction of both call-site checks:
/// `is_epoch_prune_eligible(E)` (derived from the low-water mark) AND
/// `durable_epoch_watermark >= E`. Neither is a fixed constant — the low-water
/// mark advances as tracked clients confirm-apply, and the durable watermark
/// advances as the durable backend catches up, so the drained set grows over
/// time in production rather than staying permanently empty. Shared by the OR
/// write path (`crdt.rs`) and the SYNC leaf (`sync.rs`).
///
/// A ref whose storage drop FAILS (read or write error) is handed back to the
/// frontier via `restore_tombstone_ref` so a later sweep retries it — dropping
/// it here would orphan the tag un-prunable in storage forever, since the drain
/// already removed its index entry.
pub(crate) async fn prune_epoch_tombstones(
    frontier: &TombstoneFrontier,
    factory: &RecordStoreFactory,
    key_writer: &KeyWriterRegistry,
) {
    for (epoch, r) in frontier.drain_prunable_tombstones() {
        let store = factory.get_or_create(&r.map, hash_to_partition(&r.key));
        // Serialize the drop against concurrent OR writes on this key.
        let _guard = key_writer.acquire(&r.map, &r.key).await;
        // Ensure the key is resident before the in-place drop: init=None only mutates
        // an already-resident slot, so an evicted key's durable tombstone would
        // otherwise never be reclaimed and its frontier ref would be consumed without
        // retry. Hydrating first (as the prior get -> put path did) also reclaims
        // evicted keys and surfaces a backend read error so the ref can be re-indexed.
        match store.get(&r.key, false).await {
            Ok(Some(_)) => {}
            // Truly gone (no resident and no durable record): nothing to reclaim.
            Ok(None) => continue,
            Err(e) => {
                tracing::warn!(map = %r.map, key = %r.key, epoch, "prune read failed, re-indexing tombstone for retry: {e}");
                frontier.restore_tombstone_ref(epoch, r);
                continue;
            }
        }
        // Drop the tag from the now-resident tombstone set IN PLACE (init=None →
        // mutate only the present record, never create one), owing a durable write
        // only when a tombstone was actually removed.
        let mut dropped = false;
        let result = {
            let mut drop_tag = |value: &mut RecordValue| {
                if let RecordValue::OrMap { tombstones, .. } = value {
                    let before = tombstones.len();
                    tombstones.retain(|t| t != &r.tag);
                    dropped = tombstones.len() != before;
                }
                dropped
            };
            store
                .update_in_place(
                    &r.key,
                    None,
                    ExpiryPolicy::NONE,
                    CallerProvenance::CrdtMerge,
                    &mut drop_tag,
                )
                .await
        };
        match result {
            // The tag is durably gone from storage only once the write-through
            // succeeds — decrement here so the gauge tracks bytes actually
            // resident, not bytes merely removed from an in-memory copy.
            Ok(_) => {
                if dropped {
                    crate::storage::record::sub_tombstone_bytes(r.tag.len() as u64);
                }
            }
            Err(e) => {
                // Operator-visible: a swallowed storage error on the prune path
                // would silently stall tombstone reclamation.
                tracing::warn!(map = %r.map, key = %r.key, epoch, "prune update failed, re-indexing tombstone for retry: {e}");
                frontier.restore_tombstone_ref(epoch, r);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(
    clippy::doc_markdown,
    clippy::redundant_pattern_matching,
    clippy::collapsible_match
)]
mod tests {
    use std::sync::Arc;

    use parking_lot::Mutex;
    use topgun_core::messages::Message;
    use topgun_core::{SystemClock, Timestamp, HLC};
    use tower::ServiceExt;

    use super::*;
    use crate::network::connection::{ConnectionKind, ConnectionRegistry};
    use crate::network::device_identity::frontier_client_id;
    use crate::service::domain::query::QueryRegistry;
    use crate::service::domain::schema::SchemaService;
    use crate::service::operation::{service_names, OperationContext, OperationResponse};
    use crate::service::security::{SecurityConfig, WriteAdmission};
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

    fn make_validator() -> Arc<WriteAdmission> {
        let hlc = Arc::new(Mutex::new(HLC::new(
            "test-node".to_string(),
            Box::new(SystemClock),
        )));
        Arc::new(WriteAdmission::new(
            Arc::new(SecurityConfig::default()),
            hlc,
        ))
    }

    fn make_service() -> Arc<CrdtService> {
        let factory = make_factory();
        let registry = Arc::new(ConnectionRegistry::new());
        let query_registry = Arc::new(QueryRegistry::new());
        Arc::new(CrdtService::new(
            factory,
            registry,
            make_validator(),
            query_registry,
            Arc::new(SchemaService::new()),
        ))
    }

    fn make_service_with_journal() -> (Arc<CrdtService>, Arc<JournalStore>) {
        let factory = make_factory();
        let registry = Arc::new(ConnectionRegistry::new());
        let query_registry = Arc::new(QueryRegistry::new());
        let journal = Arc::new(JournalStore::new(100));
        let svc = Arc::new(
            CrdtService::new(
                factory,
                registry,
                make_validator(),
                query_registry,
                Arc::new(SchemaService::new()),
            )
            .with_journal(Arc::clone(&journal)),
        );
        (svc, journal)
    }

    fn make_timestamp() -> Timestamp {
        Timestamp {
            millis: 1_700_000_000_000,
            counter: 1,
            node_id: "test-node".to_string(),
        }
    }

    #[test]
    fn or_map_semantic_view_order_independent_and_reflexive_under_nan() {
        let ts = make_timestamp();
        let entry = |tag: &str, v: Value| OrMapEntry {
            value: v,
            tag: tag.to_string(),
            timestamp: ts.clone(),
        };

        // Same live set + tombstone set, opposite Vec order → equal views. This is
        // the order-independence the delta-fold path relies on (the resident slot
        // is in operation-insertion order, never canonically sorted).
        let a = RecordValue::OrMap {
            records: vec![
                entry("t1", Value::Int(1)),
                entry("t2", Value::String("x".into())),
            ],
            tombstones: vec!["z".to_string(), "a".to_string()],
        };
        let b = RecordValue::OrMap {
            records: vec![
                entry("t2", Value::String("x".into())),
                entry("t1", Value::Int(1)),
            ],
            tombstones: vec!["a".to_string(), "z".to_string()],
        };
        assert_eq!(
            or_map_semantic_view(Some(a)),
            or_map_semantic_view(Some(b)),
            "order-differing but set-equal OR-Map slots must compare equal"
        );

        // A NaN-valued slot must equal ITSELF: a derived PartialEq that delegated
        // to Value would break reflexivity (NaN != NaN) and report a slot as
        // unequal to its own recovery. Debug-string canonicalization fixes it.
        let nan = RecordValue::OrMap {
            records: vec![entry("t1", Value::Float(f64::NAN))],
            tombstones: vec![],
        };
        assert_eq!(
            or_map_semantic_view(Some(nan.clone())),
            or_map_semantic_view(Some(nan)),
            "the equivalence oracle must be reflexive even for NaN float values"
        );

        // A genuine live-value difference must still be detected (non-vacuous).
        let c = RecordValue::OrMap {
            records: vec![entry("t1", Value::Int(1))],
            tombstones: vec![],
        };
        let d = RecordValue::OrMap {
            records: vec![entry("t1", Value::Int(2))],
            tombstones: vec![],
        };
        assert_ne!(
            or_map_semantic_view(Some(c)),
            or_map_semantic_view(Some(d)),
            "distinct survivor values must produce distinct views"
        );
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
        let svc = CrdtService::new(
            factory,
            registry,
            make_validator(),
            query_registry,
            Arc::new(SchemaService::new()),
        );
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

    // -- Event Journal: write path appends --

    #[tokio::test]
    async fn write_path_appends_to_journal() {
        let (svc, journal) = make_service_with_journal();
        let record = topgun_core::LWWRecord {
            value: Some(rmpv::Value::String("Alice".into())),
            timestamp: make_timestamp(),
            ttl_ms: None,
        };
        let put = Operation::ClientOp {
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
        svc.clone().oneshot(put).await.unwrap();

        let remove = Operation::ClientOp {
            ctx: make_ctx(),
            payload: topgun_core::messages::ClientOpMessage {
                payload: topgun_core::messages::base::ClientOp {
                    id: Some("op-2".to_string()),
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
        svc.oneshot(remove).await.unwrap();

        let (events, _) = journal.read(0, 100, None);
        assert_eq!(events.len(), 2, "both writes recorded in the journal");
        assert_eq!(events[0].event_type, JournalEventType::PUT);
        assert_eq!(events[0].map_name, "users");
        assert_eq!(events[0].key, "user-1");
        assert_eq!(events[1].event_type, JournalEventType::DELETE);
        // Sequences are monotonic in apply order.
        assert_eq!(events[0].sequence, "1");
        assert_eq!(events[1].sequence, "2");
    }

    #[tokio::test]
    async fn write_path_without_journal_is_noop() {
        // A service built without `.with_journal` must apply writes normally and
        // never touch a journal — guards the Option<journal> no-op branch.
        let svc = make_service();
        let record = topgun_core::LWWRecord {
            value: Some(rmpv::Value::String("Bob".into())),
            timestamp: make_timestamp(),
            ttl_ms: None,
        };
        let put = Operation::ClientOp {
            ctx: make_ctx(),
            payload: topgun_core::messages::ClientOpMessage {
                payload: topgun_core::messages::base::ClientOp {
                    id: Some("op-1".to_string()),
                    map_name: "users".to_string(),
                    key: "user-2".to_string(),
                    op_type: None,
                    record: Some(Some(record)),
                    or_record: None,
                    or_tag: None,
                    write_concern: None,
                    timeout: None,
                },
            },
        };
        let resp = svc.oneshot(put).await.unwrap();
        assert!(
            matches!(resp, OperationResponse::Message(ref msg) if matches!(**msg, Message::OpAck(_))),
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
        let op = Operation::GarbageCollect { ctx: make_ctx() };

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

    fn make_strict_validator() -> Arc<WriteAdmission> {
        let hlc = Arc::new(Mutex::new(HLC::new(
            "server-node".to_string(),
            Box::new(SystemClock),
        )));
        let config = SecurityConfig {
            require_auth: true,
            max_value_bytes: 0,
        };
        Arc::new(WriteAdmission::new(Arc::new(config), hlc))
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

    // -- AC8: batch atomic rejection: if 2nd op fails, 1st op's data not written --
    // (Verified via the validate-all-then-apply-all logic in handle_op_batch)

    #[tokio::test]
    async fn op_batch_atomic_rejection_when_second_op_fails() {
        // Atomicity is driven by a surviving admission check (value-size limit):
        // op-1 is a tombstone REMOVE (size 0, admitted) while op-2 carries an
        // oversized value that trips `max_value_bytes`. If admission were applied
        // per-op-then-write instead of validate-all-then-apply-all, op-1's data
        // would already be persisted when op-2 fails — this test fails the batch
        // and (below) asserts op-1 left no record behind.
        let hlc = Arc::new(Mutex::new(HLC::new(
            "server-node".to_string(),
            Box::new(SystemClock),
        )));
        let config = SecurityConfig {
            require_auth: false,
            max_value_bytes: 8,
        };
        let validator = Arc::new(WriteAdmission::new(Arc::new(config), hlc));
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

        let ctx = {
            let mut ctx = make_ctx();
            ctx.connection_id = Some(handle.id);
            ctx
        };

        // op-2 carries a value that serializes well beyond the 8-byte limit.
        let oversized_record = topgun_core::LWWRecord {
            value: Some(rmpv::Value::String(
                "this-value-is-far-larger-than-eight-bytes".into(),
            )),
            timestamp: make_timestamp(),
            ttl_ms: None,
        };

        let ops = vec![
            // op-1 is a tombstone REMOVE on "open-map" — size 0, admitted alone.
            topgun_core::messages::base::ClientOp {
                id: Some("op-1".to_string()),
                map_name: "open-map".to_string(),
                key: "key-1".to_string(),
                op_type: None,
                record: Some(None),
                or_record: None,
                or_tag: None,
                write_concern: None,
                timeout: None,
            },
            // op-2 targets "open-map" with an oversized value — fails admission.
            topgun_core::messages::base::ClientOp {
                id: Some("op-2".to_string()),
                map_name: "open-map".to_string(),
                key: "key-2".to_string(),
                op_type: None,
                record: Some(Some(oversized_record)),
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

        // The batch must fail due to op-2's oversized value.
        let result = svc.oneshot(op).await;
        assert!(
            matches!(result, Err(OperationError::ValueTooLarge { .. })),
            "expected ValueTooLarge for batch with oversized op, got {result:?}"
        );

        // Atomicity: op-1's tombstone must NOT have been applied. Because the batch
        // is validated-all-then-applied-all, a rejection leaves no store touched for
        // the target map (no record for key-1 in any partition).
        for store in factory.get_all_for_map("open-map") {
            assert!(
                store.get("key-1", false).await.unwrap().is_none(),
                "op-1 must not be persisted when the batch is rejected"
            );
        }
    }

    // -- AC20: REMOVE ops are never rejected due to value size --

    #[tokio::test]
    async fn remove_op_not_rejected_by_size_limit() {
        let hlc = Arc::new(Mutex::new(HLC::new(
            "server-node".to_string(),
            Box::new(SystemClock),
        )));
        let config = SecurityConfig {
            require_auth: false,
            max_value_bytes: 1, // very small limit
        };
        let validator = Arc::new(WriteAdmission::new(Arc::new(config), hlc));
        let factory = make_factory();
        let registry = Arc::new(ConnectionRegistry::new());
        let query_registry = Arc::new(QueryRegistry::new());
        let svc = Arc::new(CrdtService::new(
            factory,
            Arc::clone(&registry),
            validator,
            query_registry,
            Arc::new(SchemaService::new()),
        ));

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
        assert_eq!(
            rmpv_to_value(&rmpv::Value::Boolean(true)),
            Value::Bool(true)
        );
        assert_eq!(
            rmpv_to_value(&rmpv::Value::Boolean(false)),
            Value::Bool(false)
        );
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

    use crate::query::delta_buffer::DeltaBuffer;
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
        let (conn1_handle, mut conn1_rx) = conn_registry.register(ConnectionKind::Client, &config);
        let (conn2_handle, mut conn2_rx) = conn_registry.register(ConnectionKind::Client, &config);

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
                aggregations: None,
            },
            previous_result_keys: DashSet::new(),
            live_window: Arc::new(crate::query::window::LiveWindow::new(vec![], None)),
            fields: None,
            delta_buffer: Arc::new(DeltaBuffer::new(64)),
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
        let (_conn_handle, mut conn_rx) = conn_registry.register(ConnectionKind::Client, &config);

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

        let (conn1_handle, mut conn1_rx) = conn_registry.register(ConnectionKind::Client, &config);

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
                aggregations: None,
            },
            previous_result_keys: DashSet::new(),
            live_window: Arc::new(crate::query::window::LiveWindow::new(vec![], None)),
            fields: None,
            delta_buffer: Arc::new(DeltaBuffer::new(64)),
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

    fn make_lww_put_with_value(
        ctx: OperationContext,
        map_name: &str,
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
        let hlc = Arc::new(Mutex::new(HLC::new(
            "server-node".to_string(),
            Box::new(SystemClock),
        )));
        let config = SecurityConfig {
            require_auth: false,
            ..SecurityConfig::default()
        };
        let validator = Arc::new(WriteAdmission::new(Arc::new(config), hlc));
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
    fn drain_query_updates(
        rx: &mut tokio::sync::mpsc::Receiver<crate::network::connection::OutboundMessage>,
    ) -> Vec<(
        topgun_core::messages::base::ChangeEventType,
        String,
        rmpv::Value,
    )> {
        let mut updates = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            if let crate::network::connection::OutboundMessage::Binary(bytes) = msg {
                if let Ok(decoded) = rmp_serde::from_slice::<topgun_core::messages::Message>(&bytes)
                {
                    if let topgun_core::messages::Message::QueryUpdate { payload } = decoded {
                        updates.push((payload.change_type, payload.key, payload.value));
                    }
                }
            }
        }
        updates
    }

    /// AC3: QUERY_UPDATE is NOT sent to the connection that originated the write.
    /// This tests broadcast_query_updates() writer exclusion + field projection.
    #[tokio::test]
    async fn broadcast_query_updates_writer_exclusion_and_projection() {
        let (svc, conn_registry, query_registry) = make_broadcast_test_setup();
        let config = crate::network::config::ConnectionConfig::default();

        // Register two client connections
        let (writer_handle, mut writer_rx) =
            conn_registry.register(ConnectionKind::Client, &config);
        let (sub_handle, mut sub_rx) = conn_registry.register(ConnectionKind::Client, &config);

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
                aggregations: None,
            },
            previous_result_keys: DashSet::new(),
            live_window: Arc::new(crate::query::window::LiveWindow::new(vec![], None)),
            fields: Some(vec!["name".to_string()]),
            delta_buffer: Arc::new(DeltaBuffer::new(64)),
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
        assert_eq!(
            *change_type,
            topgun_core::messages::base::ChangeEventType::ENTER
        );
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

    // -----------------------------------------------------------------------
    // OR-Map data-loss regression (add-wins / remove-wins / convergence).
    //
    // These assert the FIXED behaviour at the SERVER boundary: ops flow through
    // CrdtService::apply_single_op (via oneshot) and the inbound
    // SyncService::handle_ormap_push_diff ingest path, and the stored
    // RecordValue::OrMap { records, tombstones } is read back and compared.
    // The earlier audit repro demonstrated OR_REMOVE of one tag destroying every
    // concurrent value under the key; the inverted forms below lock in survival.
    // -----------------------------------------------------------------------

    use topgun_core::hash_to_partition;

    /// Builds a CRDT op context routed to the key's hash partition so the single
    /// ClientOp path (which honours `ctx.partition_id`) and the SyncService
    /// push-diff path (which uses `hash_to_partition(key)`) land on the same store.
    fn make_ctx_for_key(key: &str) -> OperationContext {
        let mut ctx = make_ctx();
        ctx.partition_id = Some(hash_to_partition(key));
        ctx
    }

    fn or_add_op(map: &str, key: &str, value: &str, tag: &str) -> Operation {
        let or_rec = topgun_core::ORMapRecord {
            value: rmpv::Value::String(value.into()),
            timestamp: make_timestamp(),
            tag: tag.to_string(),
            ttl_ms: None,
        };
        Operation::ClientOp {
            // connection_id = None -> tags used as-is, no sanitize/regeneration.
            ctx: make_ctx_for_key(key),
            payload: topgun_core::messages::ClientOpMessage {
                payload: topgun_core::messages::base::ClientOp {
                    id: Some(format!("add-{tag}")),
                    map_name: map.to_string(),
                    key: key.to_string(),
                    op_type: None,
                    record: None,
                    or_record: Some(Some(or_rec)),
                    or_tag: None,
                    write_concern: None,
                    timeout: None,
                },
            },
        }
    }

    fn or_remove_op(map: &str, key: &str, tag: &str) -> Operation {
        Operation::ClientOp {
            ctx: make_ctx_for_key(key),
            payload: topgun_core::messages::ClientOpMessage {
                payload: topgun_core::messages::base::ClientOp {
                    id: Some(format!("rm-{tag}")),
                    map_name: map.to_string(),
                    key: key.to_string(),
                    op_type: None,
                    record: None,
                    or_record: None,
                    or_tag: Some(Some(tag.to_string())),
                    write_concern: None,
                    timeout: None,
                },
            },
        }
    }

    fn make_service_with_factory() -> (Arc<CrdtService>, Arc<RecordStoreFactory>) {
        let factory = make_factory();
        let registry = Arc::new(ConnectionRegistry::new());
        let query_registry = Arc::new(QueryRegistry::new());
        let svc = Arc::new(CrdtService::new(
            Arc::clone(&factory),
            registry,
            make_validator(),
            query_registry,
            Arc::new(SchemaService::new()),
        ));
        (svc, factory)
    }

    fn make_service_with_frontier() -> (
        Arc<CrdtService>,
        Arc<RecordStoreFactory>,
        Arc<TombstoneFrontier>,
    ) {
        let factory = make_factory();
        let registry = Arc::new(ConnectionRegistry::new());
        let query_registry = Arc::new(QueryRegistry::new());
        let frontier = Arc::new(TombstoneFrontier::new(None));
        frontier.set_epoch_width(1); // one epoch per stamped tombstone
        let svc = Arc::new(
            CrdtService::new(
                Arc::clone(&factory),
                registry,
                make_validator(),
                query_registry,
                Arc::new(SchemaService::new()),
            )
            .with_frontier(Arc::clone(&frontier)),
        );
        (svc, factory, frontier)
    }

    /// AC4 (OR write path): the wholesale epoch-drop prune is wired into the
    /// `OR_REMOVE` write path (the sweep runs after each apply). With an injected
    /// durability watermark and the low-water-mark STRICTLY past the epoch, a
    /// subsequent `OR_REMOVE` fires the sweep and drops the earlier epoch's
    /// tombstone from storage; a not-strictly-past epoch survives. DARK by
    /// default — the injected watermark exercises the real drop path.
    #[tokio::test]
    async fn ac4_prune_wired_into_or_write_path() {
        let (svc, factory, frontier) = make_service_with_frontier();

        // Add + remove T1 on k1 -> epoch 1; T2 on k2 -> epoch 2. Both stored.
        for (key, val, tag) in [("k1", "v1", "T1"), ("k2", "v2", "T2")] {
            Arc::clone(&svc)
                .oneshot(or_add_op("m", key, val, tag))
                .await
                .unwrap();
            Arc::clone(&svc)
                .oneshot(or_remove_op("m", key, tag))
                .await
                .unwrap();
        }
        let (_, tombs) = read_or_map(&factory, "m", "k1").await;
        assert!(
            tombs.contains(&"T1".to_string()),
            "tombstone stored after OR_REMOVE (dark: watermark 0 -> no prune yet)"
        );
        assert_eq!(frontier.current_epoch(), 2, "epochs 1..=2 stamped");

        // Raise the LWM strictly past epoch 1 (cursor 2 > 1) and open the
        // durability watermark.
        let c: String = "a5:alice|dev-1".into();
        frontier.set_delivered(ConnectionId(1), 100);
        assert!(frontier.confirm_apply_ack(&c, 2, ConnectionId(1)).await);
        assert_eq!(frontier.low_water_mark(), 2);
        frontier.set_durable_epoch_watermark(1000);

        // Fire the sweep via an OR_REMOVE on a THIRD key. Its own new tombstone
        // lands in epoch 3 (pinned); epoch 2 is pinned too (LWM 2 not strictly
        // past 2); epoch 1's T1 is dropped.
        Arc::clone(&svc)
            .oneshot(or_add_op("m", "k3", "v3", "T3"))
            .await
            .unwrap();
        Arc::clone(&svc)
            .oneshot(or_remove_op("m", "k3", "T3"))
            .await
            .unwrap();

        let (_, tombs_k1) = read_or_map(&factory, "m", "k1").await;
        assert!(
            !tombs_k1.contains(&"T1".to_string()),
            "epoch-1 tombstone pruned from storage via the OR write path"
        );
        let (_, tombs_k2) = read_or_map(&factory, "m", "k2").await;
        assert!(
            tombs_k2.contains(&"T2".to_string()),
            "epoch-2 tombstone still pinned (LWM 2 not strictly past epoch 2)"
        );
    }

    /// Op-path data-loss guard: the OR write path has NO forgotten-client gate,
    /// so a NOT-yet-ACKed (untracked) device's `CLIENT_OP` / `OP_BATCH` OR writes
    /// are APPLIED and acked even with tombstone protection active — never
    /// silently acked-and-dropped. This pins a regression class: `OP_ACK` clears
    /// the client oplog, so any future op-path gate keyed on unknown==forgotten
    /// would turn an ack into permanent client-side data loss.
    /// Build a frontier-gated `CrdtService` with protection ARMED and an
    /// IDENTIFIABLE-but-untracked device connection bound (device_id set, no
    /// frontier cursor → reads as forgotten). A future op-path gate keyed on
    /// connection → resolve_client_id → is_forgotten would consult the frontier
    /// for this writer and (wrongly) reject — the guard below asserts it must not.
    async fn armed_service_with_untracked_device(
    ) -> (Arc<CrdtService>, Arc<RecordStoreFactory>, ConnectionId) {
        let factory = make_factory();
        let registry = Arc::new(ConnectionRegistry::new());
        let query_registry = Arc::new(QueryRegistry::new());
        let frontier = Arc::new(TombstoneFrontier::new(None));
        frontier.set_epoch_width(1);
        let svc = Arc::new(
            CrdtService::new(
                Arc::clone(&factory),
                Arc::clone(&registry),
                make_validator(),
                query_registry,
                Arc::new(SchemaService::new()),
            )
            .with_frontier(Arc::clone(&frontier)),
        );
        let (handle, _rx) = registry.register(
            ConnectionKind::Client,
            &crate::network::config::ConnectionConfig::default(),
        );
        handle.metadata.write().await.device_id = Some("dev-untracked".to_string());
        let conn = handle.id;
        frontier.stamp_tombstone("m", "seed", "seed-tag"); // current_epoch = 1
        frontier.set_durable_epoch_watermark(1000); // protection active
        assert!(frontier.is_protection_active());
        assert!(
            frontier.is_forgotten(&frontier_client_id(None, "dev-untracked")),
            "the writer is untracked (forgotten) from the frontier's view"
        );
        (svc, factory, conn)
    }

    #[tokio::test]
    async fn oppath_or_writes_from_untracked_device_are_applied_not_dropped() {
        let (svc, factory, conn) = armed_service_with_untracked_device().await;

        // CLIENT_OP OR_ADD from the untracked, identified device → applied + acked
        // (never a silent drop; `.expect` fails on any error/reject).
        let or_rec1 = topgun_core::ORMapRecord {
            value: rmpv::Value::String("v1".into()),
            timestamp: make_timestamp(),
            tag: "R1".to_string(),
            ttl_ms: None,
        };
        let mut ctx1 = make_ctx_for_key("k1");
        ctx1.connection_id = Some(conn);
        Arc::clone(&svc)
            .oneshot(Operation::ClientOp {
                ctx: ctx1,
                payload: topgun_core::messages::ClientOpMessage {
                    payload: topgun_core::messages::base::ClientOp {
                        id: Some("add-R1".to_string()),
                        map_name: "m".to_string(),
                        key: "k1".to_string(),
                        op_type: None,
                        record: None,
                        or_record: Some(Some(or_rec1)),
                        or_tag: None,
                        write_concern: None,
                        timeout: None,
                    },
                },
            })
            .await
            .expect("client_op must not error");
        // The write lands (a live record exists). The op path re-stamps the OR tag
        // from the sanitized server HLC, so the surviving tag is server-issued, not
        // the client's "R1" — what matters for the data-loss guard is that the write
        // is applied, never silently acked-and-dropped.
        assert_eq!(
            read_or_map(&factory, "m", "k1").await.0.len(),
            1,
            "untracked device's CLIENT_OP OR_ADD is applied even under active protection"
        );

        // OP_BATCH OR_ADD from the same untracked device → OpAck + applied.
        let or_rec2 = topgun_core::ORMapRecord {
            value: rmpv::Value::String("v2".into()),
            timestamp: make_timestamp(),
            tag: "R2".to_string(),
            ttl_ms: None,
        };
        let mut ctx2 = make_ctx_for_key("k2");
        ctx2.connection_id = Some(conn);
        let batch = Operation::OpBatch {
            ctx: ctx2,
            payload: topgun_core::messages::sync::OpBatchMessage {
                payload: topgun_core::messages::sync::OpBatchPayload {
                    ops: vec![topgun_core::messages::base::ClientOp {
                        id: Some("batch-R2".to_string()),
                        map_name: "m".to_string(),
                        key: "k2".to_string(),
                        op_type: None,
                        record: None,
                        or_record: Some(Some(or_rec2)),
                        or_tag: None,
                        write_concern: None,
                        timeout: None,
                    }],
                    write_concern: None,
                    timeout: None,
                },
            },
        };
        let resp = Arc::clone(&svc)
            .oneshot(batch)
            .await
            .expect("op_batch must not error");
        assert!(
            matches!(resp, OperationResponse::Message(ref m) if matches!(**m, Message::OpAck(_))),
            "op_batch is acked (the ack that clears the client oplog)"
        );
        assert_eq!(
            read_or_map(&factory, "m", "k2").await.0.len(),
            1,
            "untracked device's OP_BATCH OR_ADD is applied — the acked write is durable, not dropped"
        );
    }

    /// Reads back the stored OR-Map for a key at its hash partition.
    async fn read_or_map(
        factory: &Arc<RecordStoreFactory>,
        map: &str,
        key: &str,
    ) -> (Vec<String>, Vec<String>) {
        let store = factory.get_or_create(map, hash_to_partition(key));
        let value = store.get(key, false).await.unwrap().map(|r| r.value);
        match value {
            Some(RecordValue::OrMap {
                records,
                tombstones,
            }) => {
                let mut tags: Vec<String> = records.into_iter().map(|e| e.tag).collect();
                tags.sort();
                let mut tombs = tombstones;
                tombs.sort();
                (tags, tombs)
            }
            Some(RecordValue::OrTombstones { tags }) => {
                let mut tombs = tags;
                tombs.sort();
                (Vec::new(), tombs)
            }
            Some(RecordValue::Lww { .. }) | None => (Vec::new(), Vec::new()),
        }
    }

    /// Like `read_or_map` but pairs each surviving tag with its record value
    /// (debug-formatted), so convergence assertions catch value divergence and
    /// not merely tag/tombstone-set divergence. The reused-tag proptest pool makes
    /// this matter: two stores could agree on the surviving tag set yet disagree on
    /// which value won for a given tag.
    async fn read_or_map_full(
        factory: &Arc<RecordStoreFactory>,
        map: &str,
        key: &str,
    ) -> (Vec<(String, String)>, Vec<String>) {
        let store = factory.get_or_create(map, hash_to_partition(key));
        match store.get(key, false).await.unwrap().map(|r| r.value) {
            Some(RecordValue::OrMap {
                records,
                tombstones,
            }) => {
                let mut pairs: Vec<(String, String)> = records
                    .into_iter()
                    .map(|e| (e.tag, format!("{:?}", e.value)))
                    .collect();
                pairs.sort();
                let mut tombs = tombstones;
                tombs.sort();
                (pairs, tombs)
            }
            Some(RecordValue::OrTombstones { tags }) => {
                let mut tombs = tags;
                tombs.sort();
                (Vec::new(), tombs)
            }
            Some(RecordValue::Lww { .. }) | None => (Vec::new(), Vec::new()),
        }
    }

    /// Returns the survivor values (not tags) for a key, for human-readable
    /// assertions like "play survives".
    async fn read_or_map_values(
        factory: &Arc<RecordStoreFactory>,
        map: &str,
        key: &str,
    ) -> Vec<String> {
        let store = factory.get_or_create(map, hash_to_partition(key));
        match store.get(key, false).await.unwrap().map(|r| r.value) {
            Some(RecordValue::OrMap { records, .. }) => records
                .into_iter()
                .map(|e| format!("{:?}", e.value))
                .collect(),
            _ => Vec::new(),
        }
    }

    // -- Tombstone-bytes gauge is residency-independent --
    //
    // Drives the REAL evict -> rehydrate path via `RecordStore::evict_lru` (the
    // same primitive `EvictionOrchestrator` calls in production) rather than the
    // documented read-path surrogate, since the existing store/factory test
    // infrastructure (a real `RedbDataStore` + `evict_lru`/`get`) makes it
    // practical here — this exercises the actual eviction blind-spot the gauge
    // exists to defend against, not a stand-in for it.
    #[tokio::test]
    #[serial_test::serial(tombstone_gauge)]
    async fn or_remove_tombstone_gauge_survives_real_eviction_and_rehydration() {
        let dir = tempfile::tempdir().expect("tempdir");
        let redb_path = dir.path().join("gauge_residency.redb");
        let data_store: Arc<dyn crate::storage::MapDataStore> = Arc::new(
            crate::storage::datastores::RedbDataStore::new(&redb_path).expect("redb open"),
        );
        let factory = Arc::new(RecordStoreFactory::new(
            StorageConfig::default(),
            data_store,
            Vec::new(),
        ));
        let registry = Arc::new(ConnectionRegistry::new());
        let query_registry = Arc::new(QueryRegistry::new());
        let svc = Arc::new(CrdtService::new(
            Arc::clone(&factory),
            registry,
            make_validator(),
            query_registry,
            Arc::new(SchemaService::new()),
        ));

        // The gauge is a process-global static: the serial(tombstone_gauge) group
        // serializes the gauge-asserting tests against each other, but UNMARKED
        // parallel tests still mutate it via ordinary OR_REMOVE applies and prune
        // drains. Exact-equality snapshots are therefore retried on a fresh map:
        // ambient noise shifts between attempts, while a genuine double-count /
        // dropped-gauge regression is deterministic and fails every attempt.
        let mut attempt = 0usize;
        'attempt: loop {
            attempt += 1;
            let map_name = &format!("gauge_residency_map_{attempt}");
            let key = "item-1";
            let tag = "tag-gauge-residency";

            let before_write = crate::storage::record::tombstone_bytes();

            svc.clone()
                .oneshot(or_add_op(map_name, key, "payload", tag))
                .await
                .expect("or_add must succeed");
            svc.clone()
                .oneshot(or_remove_op(map_name, key, tag))
                .await
                .expect("or_remove must succeed");

            let after_write = crate::storage::record::tombstone_bytes();
            if after_write - before_write != tag.len() as u64 {
                assert!(
                    attempt < 5,
                    "gauge delta never settled to exactly the tag length across 5 quiet-window attempts                      (deterministic gauge regression, not parallel-test noise)"
                );
                continue 'attempt;
            }

            // Force the record non-resident via the real eviction primitive. The
            // OR_REMOVE above wrote through with `CallerProvenance::CrdtMerge` over a
            // real (non-null) data store, which marks the record clean and therefore
            // evictable.
            let store = factory.get_or_create(map_name, hash_to_partition(key));
            let evicted = store.evict_lru(u32::MAX, false);
            assert!(evicted > 0, "the clean record must be evicted");
            assert!(
                !store.exists_in_memory(key),
                "record must be non-resident after eviction"
            );

            let after_eviction = crate::storage::record::tombstone_bytes();
            if after_eviction != after_write {
                assert!(
                    attempt < 5,
                    "eviction kept moving the gauge across 5 attempts — a deterministic                      evict-drops-gauge regression, not parallel-test noise"
                );
                continue 'attempt;
            }

            // Rehydrate: `get()` transparently reloads the non-resident record from
            // the datastore. Confirm the tombstone survived the round trip and the
            // gauge did not move.
            let (_, tombstones) = read_or_map(&factory, map_name, key).await;
            assert_eq!(
                tombstones,
                vec![tag.to_string()],
                "tombstone must still be present after rehydration"
            );

            let after_rehydration = crate::storage::record::tombstone_bytes();
            if after_rehydration != after_write {
                assert!(
                    attempt < 5,
                    "rehydration kept moving the gauge across 5 attempts — a deterministic                      double-count regression, not parallel-test noise"
                );
                continue 'attempt;
            }
            break 'attempt;
        }
    }

    // AC1: add-wins — OR_REMOVE of one tag preserves concurrent survivors.
    #[tokio::test]
    async fn or_remove_preserves_concurrent_value_add_wins() {
        let (svc, factory) = make_service_with_factory();

        svc.clone()
            .oneshot(or_add_op("tags", "item-1", "work", "t1"))
            .await
            .unwrap();
        svc.clone()
            .oneshot(or_add_op("tags", "item-1", "play", "t2"))
            .await
            .unwrap();

        let (tags_before, _) = read_or_map(&factory, "tags", "item-1").await;
        assert_eq!(
            tags_before,
            vec!["t1", "t2"],
            "both adds present before remove"
        );

        svc.clone()
            .oneshot(or_remove_op("tags", "item-1", "t1"))
            .await
            .unwrap();

        let (records, tombstones) = read_or_map(&factory, "tags", "item-1").await;
        assert_eq!(
            records,
            vec!["t2"],
            "removing t1 must NOT destroy t2 (add-wins); stored records={records:?}"
        );
        assert_eq!(tombstones, vec!["t1"], "t1 must be tombstoned");

        let survivors = read_or_map_values(&factory, "tags", "item-1").await;
        assert!(
            survivors.iter().any(|v| v.contains("play")),
            "\"play\" (t2) must survive the remove of t1; survivors={survivors:?}"
        );
    }

    // AC2: remove-wins — a tombstoned tag is never resurrected by a later OR_ADD.
    #[tokio::test]
    async fn or_add_after_remove_does_not_resurrect_remove_wins() {
        let (svc, factory) = make_service_with_factory();

        svc.clone()
            .oneshot(or_add_op("tags", "item-2", "hello", "tag-x"))
            .await
            .unwrap();
        svc.clone()
            .oneshot(or_remove_op("tags", "item-2", "tag-x"))
            .await
            .unwrap();
        // Re-add the SAME tag after it was removed — observed-remove forbids resurrection.
        svc.clone()
            .oneshot(or_add_op("tags", "item-2", "hello", "tag-x"))
            .await
            .unwrap();

        let (records, tombstones) = read_or_map(&factory, "tags", "item-2").await;
        assert!(
            !records.contains(&"tag-x".to_string()),
            "tag-x must NOT reappear in visible records after remove (remove-wins); records={records:?}"
        );
        assert_eq!(
            tombstones,
            vec!["tag-x"],
            "tag-x must remain tombstoned; tombstones={tombstones:?}"
        );
    }

    // AC6 (SPEC-342d / SPEC-333b): N concurrent OR_ADDs on the SAME key must
    // all survive — the per-key writer lock now serializes the
    // `store.get` -> merge -> `store.put` RMW inside `apply_single_op`'s
    // OR_ADD branch, so no concurrent add can read stale pre-mutation state
    // and clobber another add's merge on `put`. This proves OR_ADD-vs-OR_ADD
    // ONLY — it does NOT prove OR_ADD-vs-OR_REMOVE interleaving is race-free
    // (that remains 342b's responsibility, see module Context).
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_or_adds_on_same_key_lose_no_update() {
        for _ in 0..10 {
            let (svc, factory) = make_service_with_factory();
            let key = "concurrent-or-add-key";
            let n = 20usize;

            let mut handles = Vec::new();
            for i in 0..n {
                let svc = Arc::clone(&svc);
                let tag = format!("t{i}");
                let value = format!("v{i}");
                handles.push(tokio::spawn(async move {
                    svc.oneshot(or_add_op("tags", key, &value, &tag))
                        .await
                        .unwrap();
                }));
            }

            futures_util::future::join_all(handles)
                .await
                .into_iter()
                .for_each(|r| r.expect("task panicked"));

            let (tags, tombstones) = read_or_map(&factory, "tags", key).await;
            assert_eq!(
                tags.len(),
                n,
                "all {n} concurrent OR_ADDs on the same key must survive with no lost update, \
                 got {} survivors: {tags:?}",
                tags.len()
            );
            assert!(
                tombstones.is_empty(),
                "no removes issued; tombstones must stay empty"
            );
        }
    }

    // AC3 (i): apply_single_op convergence — same op set delivered in DIFFERENT
    // orders to two independent stores yields byte-identical OrMap state.
    #[tokio::test]
    async fn convergence_apply_single_op_order_independent() {
        let (svc_a, factory_a) = make_service_with_factory();
        let (svc_b, factory_b) = make_service_with_factory();

        // An interleaved op set with duplicate tags and an add-after-remove.
        let key = "conv-1";
        let ops_order_a: Vec<Operation> = vec![
            or_add_op("tags", key, "a", "ta"),
            or_add_op("tags", key, "b", "tb"),
            or_remove_op("tags", key, "ta"),
            or_add_op("tags", key, "a-dup", "ta"), // resurrection attempt
            or_add_op("tags", key, "c", "tc"),
            or_remove_op("tags", key, "tc"),
        ];
        // Different delivery order to store B (removes before some adds, dup tags).
        let ops_order_b: Vec<Operation> = vec![
            or_add_op("tags", key, "c", "tc"),
            or_add_op("tags", key, "b", "tb"),
            or_remove_op("tags", key, "tc"),
            or_add_op("tags", key, "a", "ta"),
            or_remove_op("tags", key, "ta"),
            or_add_op("tags", key, "a-dup", "ta"),
        ];

        for op in ops_order_a {
            svc_a.clone().oneshot(op).await.unwrap();
        }
        for op in ops_order_b {
            svc_b.clone().oneshot(op).await.unwrap();
        }

        let state_a = read_or_map(&factory_a, "tags", key).await;
        let state_b = read_or_map(&factory_b, "tags", key).await;
        assert_eq!(
            state_a, state_b,
            "apply_single_op convergence: stores must agree on (records, tombstones)"
        );

        // Byte-identical after canonical (sorted) ordering: serialize the sorted
        // unified OrMap and compare bytes, not merely the visible state.
        let canonical = |s: &(Vec<String>, Vec<String>)| rmp_serde::to_vec_named(s).unwrap();
        assert_eq!(
            canonical(&state_a),
            canonical(&state_b),
            "serialized OrMap bytes must be identical after canonical ordering"
        );
        // Sanity: the convergent state is ta-removed, tc-removed, tb survives.
        assert_eq!(state_a.0, vec!["tb"], "only tb survives");
        assert_eq!(state_a.1, vec!["ta", "tc"], "ta and tc tombstoned");

        // Values (not just tags) must converge too: a tag winning with a different
        // value on each store would pass the tag-set check but is still divergence.
        let full_a = read_or_map_full(&factory_a, "tags", key).await;
        let full_b = read_or_map_full(&factory_b, "tags", key).await;
        assert_eq!(
            full_a, full_b,
            "apply_single_op convergence: records (tag→value) AND tombstones must agree"
        );
    }

    // AC3 (ii) + AC4: inbound handle_ormap_push_diff convergence — one store emits
    // an ORMapEntry diff, the other ingests it (and vice-versa); both converge to
    // byte-identical state, tombstones are not discarded, concurrent records are
    // not clobbered.
    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn convergence_handle_ormap_push_diff_both_directions() {
        use crate::service::domain::sync::SyncService;
        use crate::storage::merkle_sync::MerkleSyncManager;
        use topgun_core::messages::{ORMapEntry, ORMapPushDiff, ORMapPushDiffPayload};

        let (svc_a, factory_a) = make_service_with_factory();
        let (svc_b, factory_b) = make_service_with_factory();

        let sync_a = Arc::new(SyncService::new(
            Arc::new(MerkleSyncManager::default()),
            Arc::clone(&factory_a),
            Arc::new(ConnectionRegistry::new()),
        ));
        let sync_b = Arc::new(SyncService::new(
            Arc::new(MerkleSyncManager::default()),
            Arc::clone(&factory_b),
            Arc::new(ConnectionRegistry::new()),
        ));

        let key = "conv-diff";
        // Store A: add t1, add t2, remove t1  -> records {t2}, tombstones {t1}
        svc_a
            .clone()
            .oneshot(or_add_op("tags", key, "work", "t1"))
            .await
            .unwrap();
        svc_a
            .clone()
            .oneshot(or_add_op("tags", key, "play", "t2"))
            .await
            .unwrap();
        svc_a
            .clone()
            .oneshot(or_remove_op("tags", key, "t1"))
            .await
            .unwrap();

        // Store B: concurrent add t3 (a survivor that must NOT be clobbered on ingest).
        svc_b
            .clone()
            .oneshot(or_add_op("tags", key, "concurrent", "t3"))
            .await
            .unwrap();

        // Build A's diff entry from its stored OR-Map.
        let store_a = factory_a.get_or_create("tags", hash_to_partition(key));
        let (a_records, a_tombs) = match store_a.get(key, false).await.unwrap().unwrap().value {
            RecordValue::OrMap {
                records,
                tombstones,
            } => (records, tombstones),
            other => panic!("expected OrMap, got {other:?}"),
        };
        let a_entry = ORMapEntry {
            key: key.to_string(),
            records: a_records
                .iter()
                .map(|e| topgun_core::ORMapRecord {
                    value: value_to_rmpv(&e.value),
                    timestamp: e.timestamp.clone(),
                    tag: e.tag.clone(),
                    ttl_ms: None,
                })
                .collect(),
            tombstones: a_tombs.clone(),
        };
        // AC4: the emitted entry carries BOTH surviving records AND tombstones.
        assert!(
            !a_entry.records.is_empty(),
            "emitted ORMapEntry must carry surviving records (t2)"
        );
        assert_eq!(
            a_entry.tombstones,
            vec!["t1".to_string()],
            "emitted ORMapEntry must carry the tombstone set (t1)"
        );

        // Ingest A's diff into B via handle_ormap_push_diff (the inbound path).
        sync_b
            .clone()
            .oneshot(Operation::ORMapPushDiff {
                ctx: make_ctx_sync(),
                payload: ORMapPushDiff {
                    payload: ORMapPushDiffPayload {
                        map_name: "tags".to_string(),
                        entries: vec![a_entry],
                    },
                },
            })
            .await
            .unwrap();

        // Build B's diff (now t2 + t3 survive, t1 tombstoned) and ingest into A.
        let store_b = factory_b.get_or_create("tags", hash_to_partition(key));
        let (b_records, b_tombs) = match store_b.get(key, false).await.unwrap().unwrap().value {
            RecordValue::OrMap {
                records,
                tombstones,
            } => (records, tombstones),
            other => panic!("expected OrMap, got {other:?}"),
        };
        // AC4: a client joining after the remove does NOT see t1 and DOES see survivors.
        let b_tags: Vec<&str> = b_records.iter().map(|e| e.tag.as_str()).collect();
        assert!(
            !b_tags.contains(&"t1"),
            "ingest must NOT resurrect removed t1; tags={b_tags:?}"
        );
        assert!(
            b_tags.contains(&"t2") && b_tags.contains(&"t3"),
            "ingest must preserve survivor t2 AND concurrent local t3; tags={b_tags:?}"
        );
        assert!(b_tombs.contains(&"t1".to_string()), "t1 tombstone retained");

        let b_entry = ORMapEntry {
            key: key.to_string(),
            records: b_records
                .iter()
                .map(|e| topgun_core::ORMapRecord {
                    value: value_to_rmpv(&e.value),
                    timestamp: e.timestamp.clone(),
                    tag: e.tag.clone(),
                    ttl_ms: None,
                })
                .collect(),
            tombstones: b_tombs.clone(),
        };
        sync_a
            .clone()
            .oneshot(Operation::ORMapPushDiff {
                ctx: make_ctx_sync(),
                payload: ORMapPushDiff {
                    payload: ORMapPushDiffPayload {
                        map_name: "tags".to_string(),
                        entries: vec![b_entry],
                    },
                },
            })
            .await
            .unwrap();

        // Both stores must now be byte-identical: records {t2,t3}, tombstones {t1}.
        let state_a = read_or_map(&factory_a, "tags", key).await;
        let state_b = read_or_map(&factory_b, "tags", key).await;
        assert_eq!(
            state_a, state_b,
            "handle_ormap_push_diff convergence: stores must agree after bidirectional ingest"
        );
        assert_eq!(
            rmp_serde::to_vec_named(&state_a).unwrap(),
            rmp_serde::to_vec_named(&state_b).unwrap(),
            "serialized OrMap bytes must be identical after bidirectional diff ingest"
        );
        assert_eq!(state_a.0, vec!["t2", "t3"], "t2 and t3 survive");
        assert_eq!(state_a.1, vec!["t1"], "t1 tombstoned");

        // Values (not just tags) must converge after bidirectional ingest.
        let full_a = read_or_map_full(&factory_a, "tags", key).await;
        let full_b = read_or_map_full(&factory_b, "tags", key).await;
        assert_eq!(
            full_a, full_b,
            "handle_ormap_push_diff convergence: records (tag→value) AND tombstones must agree"
        );
    }

    // AC6: tombstoning a tag CHANGES the OR-Map merkle hash, and a key reduced to
    // tombstones-only is NOT dropped from peer-visible suppression state.
    #[tokio::test]
    async fn merkle_hash_changes_on_tombstone_and_retains_tombstone_only_key() {
        use crate::storage::merkle_sync::{MerkleMutationObserver, MerkleSyncManager};

        let key = "merkle-1";
        let partition = hash_to_partition(key);
        let merkle = Arc::new(MerkleSyncManager::default());
        let observer = Arc::new(MerkleMutationObserver::new(
            Arc::clone(&merkle),
            "tags".to_string(),
            partition,
        ));
        let factory = Arc::new(RecordStoreFactory::new(
            StorageConfig::default(),
            Arc::new(NullDataStore),
            vec![observer as Arc<dyn crate::storage::mutation_observer::MutationObserver>],
        ));
        let registry = Arc::new(ConnectionRegistry::new());
        let svc = Arc::new(CrdtService::new(
            Arc::clone(&factory),
            registry,
            make_validator(),
            Arc::new(QueryRegistry::new()),
            Arc::new(SchemaService::new()),
        ));

        svc.clone()
            .oneshot(or_add_op("tags", key, "v", "m1"))
            .await
            .unwrap();
        let hash_after_add = merkle.with_ormap_tree("tags", partition, |t| t.get_root_hash());
        assert_ne!(hash_after_add, 0, "add must produce a non-zero merkle hash");

        svc.clone()
            .oneshot(or_remove_op("tags", key, "m1"))
            .await
            .unwrap();
        let hash_after_remove = merkle.with_ormap_tree("tags", partition, |t| t.get_root_hash());

        assert_ne!(
            hash_after_remove, hash_after_add,
            "tombstoning a tag must CHANGE the OR-Map merkle hash"
        );
        // Key reduced to tombstones-only must still be peer-visible (non-zero hash),
        // not silently dropped — otherwise remove-wins suppression cannot replicate.
        assert_ne!(
            hash_after_remove, 0,
            "tombstone-only key must NOT be dropped from suppression state"
        );

        // Confirm the stored value really is tombstones-only.
        let (records, tombstones) = read_or_map(&factory, "tags", key).await;
        assert!(records.is_empty(), "no active records after remove");
        assert_eq!(tombstones, vec!["m1"], "m1 tombstoned");
    }

    fn make_ctx_sync() -> OperationContext {
        OperationContext::new(1, service_names::SYNC, make_timestamp(), 5000)
    }

    // AC9: concurrent-OR_REMOVE convergence proptest.
    //
    // The SimCluster harness only expresses LWW writes (no OR_ADD/OR_REMOVE) and
    // adding an or_remove helper to it is out of scope, so the concurrent-OR_REMOVE
    // scenario cannot be reached through the sim proptests. Instead this proptest
    // mirrors the two-store convergence oracle: it generates random interleaved
    // OR_ADD/OR_REMOVE sequences with a small reused tag pool (so concurrent adds,
    // removes, and resurrection attempts collide), delivers the SAME multiset of ops
    // in two DIFFERENT orders to two independent CrdtService/RecordStore pairs, and
    // asserts the stored OrMap (records + tombstones) converges byte-identically AND
    // that the OR-Map merkle hash agrees. This is the proptest that would have caught
    // the concurrent-OR_REMOVE add-wins/remove-wins data-loss regression.
    use crate::storage::merkle_sync::{MerkleMutationObserver, MerkleSyncManager};
    use proptest::prelude::*;

    #[derive(Debug, Clone)]
    enum OrAction {
        Add { tag: u8, value: u8 },
        Remove { tag: u8 },
    }

    fn arb_or_action() -> impl Strategy<Value = OrAction> {
        // Tag pool deliberately tiny (0..4) so adds/removes of the SAME tag
        // interleave, exercising add-wins, remove-wins, and resurrection paths.
        prop_oneof![
            (0u8..4, 0u8..16).prop_map(|(tag, value)| OrAction::Add { tag, value }),
            (0u8..4).prop_map(|tag| OrAction::Remove { tag }),
        ]
    }

    fn action_to_op(action: &OrAction, map: &str, key: &str) -> Operation {
        match action {
            OrAction::Add { tag, value } => {
                or_add_op(map, key, &format!("v{value}"), &format!("tag-{tag}"))
            }
            OrAction::Remove { tag } => or_remove_op(map, key, &format!("tag-{tag}")),
        }
    }

    /// Builds a CrdtService whose RecordStore feeds a MerkleSyncManager, so the
    /// OR-Map merkle hash can be read back for the key's partition.
    fn make_service_with_merkle(
        map: &str,
        partition: u32,
    ) -> (
        Arc<CrdtService>,
        Arc<RecordStoreFactory>,
        Arc<MerkleSyncManager>,
    ) {
        let merkle = Arc::new(MerkleSyncManager::default());
        let observer = Arc::new(MerkleMutationObserver::new(
            Arc::clone(&merkle),
            map.to_string(),
            partition,
        ));
        let factory = Arc::new(RecordStoreFactory::new(
            StorageConfig::default(),
            Arc::new(NullDataStore),
            vec![observer as Arc<dyn crate::storage::mutation_observer::MutationObserver>],
        ));
        let svc = Arc::new(CrdtService::new(
            Arc::clone(&factory),
            Arc::new(ConnectionRegistry::new()),
            make_validator(),
            Arc::new(QueryRegistry::new()),
            Arc::new(SchemaService::new()),
        ));
        (svc, factory, merkle)
    }

    #[test]
    fn proptest_concurrent_or_remove_convergence() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();

        let mut runner = proptest::test_runner::TestRunner::new(proptest::test_runner::Config {
            cases: 64,
            ..proptest::test_runner::Config::default()
        });

        let map = "tags";
        let key = "conv-prop";
        let partition = hash_to_partition(key);

        runner
            .run(
                &(
                    proptest::collection::vec(arb_or_action(), 1..24),
                    any::<u64>(),
                ),
                |(actions, shuffle_seed)| {
                    runtime.block_on(async {
                        let (svc_a, factory_a, merkle_a) = make_service_with_merkle(map, partition);
                        let (svc_b, factory_b, merkle_b) = make_service_with_merkle(map, partition);

                        // Deliver to A in generated order.
                        for action in &actions {
                            svc_a
                                .clone()
                                .oneshot(action_to_op(action, map, key))
                                .await
                                .unwrap();
                        }

                        // Deliver the SAME multiset to B in a different order: a
                        // deterministic rotation derived from the case seed.
                        let mut reordered = actions.clone();
                        if !reordered.is_empty() {
                            // Modulo by the (usize) length bounds the result below
                            // reordered.len(), so the narrowing back to usize cannot
                            // truncate.
                            let rot = usize::try_from(shuffle_seed).unwrap_or(usize::MAX)
                                % reordered.len();
                            reordered.rotate_left(rot);
                        }
                        for action in &reordered {
                            svc_b
                                .clone()
                                .oneshot(action_to_op(action, map, key))
                                .await
                                .unwrap();
                        }

                        let state_a = read_or_map(&factory_a, map, key).await;
                        let state_b = read_or_map(&factory_b, map, key).await;

                        prop_assert_eq!(
                            &state_a,
                            &state_b,
                            "OrMap (records, tombstones) must converge across orders"
                        );
                        prop_assert_eq!(
                            rmp_serde::to_vec_named(&state_a).unwrap(),
                            rmp_serde::to_vec_named(&state_b).unwrap(),
                            "serialized OrMap bytes must be identical after convergence"
                        );

                        // Convergence here is asserted on the (tag, tombstone) sets,
                        // NOT on per-tag values. The pool deliberately re-adds the
                        // SAME tag with DIFFERENT values, and OR_ADD has no value
                        // tiebreak (first-arrival wins), so a reused tag's value is
                        // order-dependent across the two delivery orders. This is not
                        // a production concern: real OR_ADD tags are globally unique
                        // (HLC-based), so a tag never carries two values. Per-tag
                        // value convergence IS asserted in the deterministic
                        // convergence_* tests above, which use unique tags.

                        // Merkle consistency: convergent stores must hash identically.
                        let hash_a =
                            merkle_a.with_ormap_tree(map, partition, |t| t.get_root_hash());
                        let hash_b =
                            merkle_b.with_ormap_tree(map, partition, |t| t.get_root_hash());
                        prop_assert_eq!(
                            hash_a,
                            hash_b,
                            "OR-Map merkle hashes must agree after convergence"
                        );

                        // Remove-wins invariant: no surviving record may be tombstoned.
                        for tag in &state_a.0 {
                            prop_assert!(
                                !state_a.1.contains(tag),
                                "a tombstoned tag must never appear in the visible record set"
                            );
                        }

                        Ok(())
                    })
                },
            )
            .unwrap();
    }

    // -----------------------------------------------------------------------
    // HTTP /sync HLC timestamp-forgery re-stamp.
    //
    // A malicious HTTP client can send a forged HLC (millis = u64::MAX) that
    // would win Last-Write-Wins forever. The HttpClient-only middle arm in
    // handle_client_op / handle_op_batch must re-stamp the timestamp with a
    // fresh server-side HLC (server node_id, plausible millis), and the OR_ADD
    // tag must be regenerated from the sanitized timestamp. These tests assert
    // on BOTH the stored RecordStore state and the broadcast ServerEventPayload.
    // -----------------------------------------------------------------------

    const FORGED_MILLIS: u64 = u64::MAX;
    const FORGED_NODE_ID: &str = "forged-client";
    const SERVER_NODE_ID: &str = "test-node";

    fn make_principal() -> topgun_core::Principal {
        topgun_core::Principal {
            id: "user-http".to_string(),
            roles: vec!["user".to_string()],
        }
    }

    /// Builds an HTTP-origin context (caller_origin = HttpClient, connection_id =
    /// None, principal present) routed to the key's hash partition.
    fn make_http_ctx_for_key(key: &str) -> OperationContext {
        let mut ctx = OperationContext::new(1, service_names::CRDT, make_timestamp(), 5000);
        ctx.caller_origin = CallerOrigin::HttpClient;
        ctx.connection_id = None;
        ctx.principal = Some(make_principal());
        ctx.partition_id = Some(hash_to_partition(key));
        ctx
    }

    fn forged_timestamp() -> Timestamp {
        Timestamp {
            millis: FORGED_MILLIS,
            counter: 7,
            node_id: FORGED_NODE_ID.to_string(),
        }
    }

    /// Drains the broadcast ServerEvent messages from a connection receiver,
    /// returning the decoded payloads.
    fn drain_server_events(
        rx: &mut tokio::sync::mpsc::Receiver<crate::network::connection::OutboundMessage>,
    ) -> Vec<ServerEventPayload> {
        let mut events = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            if let crate::network::connection::OutboundMessage::Binary(bytes) = msg {
                if let Ok(Message::ServerEvent { payload }) =
                    rmp_serde::from_slice::<Message>(&bytes)
                {
                    events.push(payload);
                }
            }
        }
        events
    }

    /// Subscribes a fresh connection to `map_name` so broadcast_event fires, and
    /// returns the connection's receiver for ServerEvent capture.
    fn subscribe_listener(
        conn_registry: &Arc<ConnectionRegistry>,
        query_registry: &Arc<QueryRegistry>,
        map_name: &str,
    ) -> tokio::sync::mpsc::Receiver<crate::network::connection::OutboundMessage> {
        let config = crate::network::config::ConnectionConfig::default();
        let (handle, rx) = conn_registry.register(ConnectionKind::Client, &config);
        query_registry.register(QuerySubscription {
            query_id: format!("q-{map_name}"),
            connection_id: handle.id,
            map_name: map_name.to_string(),
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
            live_window: Arc::new(crate::query::window::LiveWindow::new(vec![], None)),
            fields: None,
            delta_buffer: Arc::new(DeltaBuffer::new(64)),
        });
        rx
    }

    /// Reads back the stored LWW timestamp for a key at its hash partition.
    async fn read_lww_timestamp(
        factory: &Arc<RecordStoreFactory>,
        map: &str,
        key: &str,
    ) -> Option<Timestamp> {
        let store = factory.get_or_create(map, hash_to_partition(key));
        match store.get(key, false).await.unwrap().map(|r| r.value) {
            Some(RecordValue::Lww { timestamp, .. }) => Some(timestamp),
            _ => None,
        }
    }

    // AC1: HTTP-origin LWW PUT with a forged timestamp is re-stamped in both the
    // stored record and the broadcast ServerEventPayload.
    #[tokio::test]
    async fn http_lww_put_restamps_forged_timestamp() {
        let factory = make_factory();
        let conn_registry = Arc::new(ConnectionRegistry::new());
        let query_registry = Arc::new(QueryRegistry::new());
        let svc = Arc::new(CrdtService::new(
            Arc::clone(&factory),
            Arc::clone(&conn_registry),
            make_validator(),
            Arc::clone(&query_registry),
            Arc::new(SchemaService::new()),
        ));

        let key = "user-http-1";
        let mut listener = subscribe_listener(&conn_registry, &query_registry, "users");

        let record = topgun_core::LWWRecord {
            value: Some(rmpv::Value::String("Alice".into())),
            timestamp: forged_timestamp(),
            ttl_ms: None,
        };
        let op = Operation::ClientOp {
            ctx: make_http_ctx_for_key(key),
            payload: topgun_core::messages::ClientOpMessage {
                payload: topgun_core::messages::base::ClientOp {
                    id: Some("http-put".to_string()),
                    map_name: "users".to_string(),
                    key: key.to_string(),
                    op_type: None,
                    record: Some(Some(record)),
                    or_record: None,
                    or_tag: None,
                    write_concern: None,
                    timeout: None,
                },
            },
        };

        svc.clone().oneshot(op).await.unwrap();

        // Stored record carries the server-re-stamped timestamp, not the forgery.
        let stored = read_lww_timestamp(&factory, "users", key)
            .await
            .expect("LWW record must be stored");
        assert_eq!(
            stored.node_id, SERVER_NODE_ID,
            "stored timestamp must carry the server node_id, not the forged one"
        );
        assert_ne!(
            stored.millis, FORGED_MILLIS,
            "stored timestamp must NOT keep the forged u64::MAX millis"
        );

        // Broadcast ServerEventPayload carries the same re-stamped timestamp.
        let events = drain_server_events(&mut listener);
        let put_event = events
            .iter()
            .find(|e| e.event_type == ServerEventType::PUT)
            .expect("a PUT ServerEvent must be broadcast");
        let broadcast_ts = &put_event
            .record
            .as_ref()
            .expect("PUT event must carry the record")
            .timestamp;
        assert_eq!(
            broadcast_ts.node_id, SERVER_NODE_ID,
            "broadcast timestamp must carry the server node_id"
        );
        assert_ne!(
            broadcast_ts.millis, FORGED_MILLIS,
            "broadcast timestamp must NOT keep the forged u64::MAX millis"
        );
    }

    // AC2: HTTP-origin OR_ADD with a forged timestamp is re-stamped, and the
    // regenerated OR tag derives from the sanitized timestamp.
    #[tokio::test]
    async fn http_or_add_restamps_forged_timestamp_and_regenerates_tag() {
        let factory = make_factory();
        let conn_registry = Arc::new(ConnectionRegistry::new());
        let query_registry = Arc::new(QueryRegistry::new());
        let svc = Arc::new(CrdtService::new(
            Arc::clone(&factory),
            Arc::clone(&conn_registry),
            make_validator(),
            Arc::clone(&query_registry),
            Arc::new(SchemaService::new()),
        ));

        let key = "item-http-1";
        let mut listener = subscribe_listener(&conn_registry, &query_registry, "tags");

        // Forged tag derived from the forged timestamp; must NOT survive.
        let forged_tag = format!("{FORGED_MILLIS}:7:{FORGED_NODE_ID}");
        let or_rec = topgun_core::ORMapRecord {
            value: rmpv::Value::String("important".into()),
            timestamp: forged_timestamp(),
            tag: forged_tag.clone(),
            ttl_ms: None,
        };
        let op = Operation::ClientOp {
            ctx: make_http_ctx_for_key(key),
            payload: topgun_core::messages::ClientOpMessage {
                payload: topgun_core::messages::base::ClientOp {
                    id: Some("http-or-add".to_string()),
                    map_name: "tags".to_string(),
                    key: key.to_string(),
                    op_type: None,
                    record: None,
                    or_record: Some(Some(or_rec)),
                    or_tag: None,
                    write_concern: None,
                    timeout: None,
                },
            },
        };

        svc.clone().oneshot(op).await.unwrap();

        // Stored OR-Map: the surviving tag must be the re-stamped one, not the forgery.
        let store = factory.get_or_create("tags", hash_to_partition(key));
        let stored_entry = match store.get(key, false).await.unwrap().map(|r| r.value) {
            Some(RecordValue::OrMap { mut records, .. }) => {
                assert_eq!(records.len(), 1, "exactly one OR entry must be stored");
                records.pop().unwrap()
            }
            other => panic!("expected OrMap, got {other:?}"),
        };
        assert_ne!(
            stored_entry.tag, forged_tag,
            "stored tag must NOT be the forged tag"
        );
        assert_eq!(
            stored_entry.timestamp.node_id, SERVER_NODE_ID,
            "stored OR timestamp must carry the server node_id"
        );
        assert_ne!(
            stored_entry.timestamp.millis, FORGED_MILLIS,
            "stored OR timestamp must NOT keep the forged u64::MAX millis"
        );
        // Tag is regenerated as "{millis}:{counter}:{node_id}" from the sanitized ts.
        let expected_tag = format!(
            "{}:{}:{}",
            stored_entry.timestamp.millis,
            stored_entry.timestamp.counter,
            stored_entry.timestamp.node_id
        );
        assert_eq!(
            stored_entry.tag, expected_tag,
            "stored tag must derive from the sanitized timestamp"
        );

        // Broadcast ServerEventPayload carries the same re-stamped tag + timestamp.
        let events = drain_server_events(&mut listener);
        let add_event = events
            .iter()
            .find(|e| e.event_type == ServerEventType::OR_ADD)
            .expect("an OR_ADD ServerEvent must be broadcast");
        assert_eq!(
            add_event.or_tag.as_deref(),
            Some(expected_tag.as_str()),
            "broadcast or_tag must be the re-stamped tag"
        );
        let broadcast_or = add_event
            .or_record
            .as_ref()
            .expect("OR_ADD event must carry the or_record");
        assert_eq!(
            broadcast_or.tag, expected_tag,
            "broadcast or_record.tag must be the re-stamped tag"
        );
        assert_eq!(
            broadcast_or.timestamp.node_id, SERVER_NODE_ID,
            "broadcast OR timestamp must carry the server node_id"
        );
        assert_ne!(
            broadcast_or.timestamp.millis, FORGED_MILLIS,
            "broadcast OR timestamp must NOT keep the forged u64::MAX millis"
        );
    }

    // AC3: HTTP-origin OR_REMOVE is tag-based and applies with no timestamp
    // sanitization — identical behavior to today (drop the matched tag, append
    // tombstone, preserving concurrent survivors).
    #[tokio::test]
    async fn http_or_remove_is_tag_based_and_preserves_survivors() {
        let (svc, factory) = make_service_with_factory();
        let key = "item-http-rm";

        // Seed two concurrent adds via the non-HTTP path (tags used verbatim).
        svc.clone()
            .oneshot(or_add_op("tags", key, "work", "keep-a"))
            .await
            .unwrap();
        svc.clone()
            .oneshot(or_add_op("tags", key, "play", "drop-b"))
            .await
            .unwrap();

        // HTTP-origin OR_REMOVE of one tag.
        let op = Operation::ClientOp {
            ctx: make_http_ctx_for_key(key),
            payload: topgun_core::messages::ClientOpMessage {
                payload: topgun_core::messages::base::ClientOp {
                    id: Some("http-or-remove".to_string()),
                    map_name: "tags".to_string(),
                    key: key.to_string(),
                    op_type: None,
                    record: None,
                    or_record: None,
                    or_tag: Some(Some("drop-b".to_string())),
                    write_concern: None,
                    timeout: None,
                },
            },
        };
        svc.clone().oneshot(op).await.unwrap();

        let (records, tombstones) = read_or_map(&factory, "tags", key).await;
        assert_eq!(
            records,
            vec!["keep-a"],
            "HTTP OR_REMOVE of drop-b must preserve the concurrent survivor keep-a"
        );
        assert_eq!(
            tombstones,
            vec!["drop-b"],
            "the removed tag must be tombstoned verbatim (no sanitization)"
        );
    }

    // -----------------------------------------------------------------------
    // Anonymous HTTP /sync HLC re-stamp (audit F3 / TODO-485).
    //
    // Under the default no-auth posture, an anonymous HTTP write reaches the
    // CRDT service with caller_origin = Anonymous, connection_id = None, and no
    // principal. Before the fix this landed in the trusted "internal/system"
    // branch and the client HLC was stored verbatim — a forged millis:u64::MAX
    // would win Last-Write-Wins forever. The anonymous arm must re-stamp the HLC
    // exactly like the authenticated HttpClient arm, while genuine internal
    // (System/Forwarded) origins must still preserve their timestamp.
    // -----------------------------------------------------------------------

    /// Builds an anonymous-HTTP-origin context (caller_origin = Anonymous,
    /// connection_id = None, no principal) routed to the key's hash partition.
    fn make_anon_http_ctx_for_key(key: &str) -> OperationContext {
        let mut ctx = OperationContext::new(1, service_names::CRDT, make_timestamp(), 5000);
        ctx.caller_origin = CallerOrigin::Anonymous;
        ctx.connection_id = None;
        ctx.principal = None;
        ctx.partition_id = Some(hash_to_partition(key));
        ctx
    }

    // F3: anonymous HTTP LWW PUT with a forged timestamp is re-stamped in both the
    // stored record and the broadcast event — closing the no-auth LWW-poison hole.
    #[tokio::test]
    async fn anon_http_lww_put_restamps_forged_timestamp() {
        let factory = make_factory();
        let conn_registry = Arc::new(ConnectionRegistry::new());
        let query_registry = Arc::new(QueryRegistry::new());
        let svc = Arc::new(CrdtService::new(
            Arc::clone(&factory),
            Arc::clone(&conn_registry),
            make_validator(),
            Arc::clone(&query_registry),
            Arc::new(SchemaService::new()),
        ));

        let key = "anon-1";
        let mut listener = subscribe_listener(&conn_registry, &query_registry, "users");

        let record = topgun_core::LWWRecord {
            value: Some(rmpv::Value::String("Mallory".into())),
            timestamp: forged_timestamp(),
            ttl_ms: None,
        };
        let op = Operation::ClientOp {
            ctx: make_anon_http_ctx_for_key(key),
            payload: topgun_core::messages::ClientOpMessage {
                payload: topgun_core::messages::base::ClientOp {
                    id: Some("anon-put".to_string()),
                    map_name: "users".to_string(),
                    key: key.to_string(),
                    op_type: None,
                    record: Some(Some(record)),
                    or_record: None,
                    or_tag: None,
                    write_concern: None,
                    timeout: None,
                },
            },
        };

        svc.clone().oneshot(op).await.unwrap();

        let stored = read_lww_timestamp(&factory, "users", key)
            .await
            .expect("LWW record must be stored");
        assert_eq!(
            stored.node_id, SERVER_NODE_ID,
            "stored timestamp must carry the server node_id, not the forged one"
        );
        assert_ne!(
            stored.millis, FORGED_MILLIS,
            "anonymous HTTP write must NOT keep the forged u64::MAX millis"
        );

        let events = drain_server_events(&mut listener);
        let put_event = events
            .iter()
            .find(|e| e.event_type == ServerEventType::PUT)
            .expect("a PUT ServerEvent must be broadcast");
        let broadcast_ts = &put_event
            .record
            .as_ref()
            .expect("PUT event must carry the record")
            .timestamp;
        assert_ne!(
            broadcast_ts.millis, FORGED_MILLIS,
            "broadcast timestamp must NOT keep the forged millis"
        );
    }

    // F3: same protection on the batch path (anonymous OpBatch re-stamps each op).
    #[tokio::test]
    async fn anon_http_op_batch_restamps_forged_timestamp() {
        let factory = make_factory();
        let conn_registry = Arc::new(ConnectionRegistry::new());
        let query_registry = Arc::new(QueryRegistry::new());
        let svc = Arc::new(CrdtService::new(
            Arc::clone(&factory),
            Arc::clone(&conn_registry),
            make_validator(),
            Arc::clone(&query_registry),
            Arc::new(SchemaService::new()),
        ));

        let key = "anon-batch-1";
        let record = topgun_core::LWWRecord {
            value: Some(rmpv::Value::String("Trudy".into())),
            timestamp: forged_timestamp(),
            ttl_ms: None,
        };
        let mut ctx = make_anon_http_ctx_for_key(key);
        // OpBatch ctx carries no single partition_id (keys span partitions).
        ctx.partition_id = None;
        let op = Operation::OpBatch {
            ctx,
            payload: topgun_core::messages::sync::OpBatchMessage {
                payload: topgun_core::messages::sync::OpBatchPayload {
                    ops: vec![topgun_core::messages::base::ClientOp {
                        id: Some("anon-batch-put".to_string()),
                        map_name: "users".to_string(),
                        key: key.to_string(),
                        op_type: None,
                        record: Some(Some(record)),
                        or_record: None,
                        or_tag: None,
                        write_concern: None,
                        timeout: None,
                    }],
                    write_concern: None,
                    timeout: None,
                },
            },
        };

        svc.clone().oneshot(op).await.unwrap();

        let stored = read_lww_timestamp(&factory, "users", key)
            .await
            .expect("LWW record must be stored");
        assert_ne!(
            stored.millis, FORGED_MILLIS,
            "anonymous batch write must NOT keep the forged u64::MAX millis"
        );
        assert_eq!(stored.node_id, SERVER_NODE_ID);
    }

    // F3 negative control: a GENUINE internal/system origin (System) still has its
    // client-supplied timestamp preserved verbatim — the re-stamp is gated on the
    // untrusted client/http transports, not on every connection-less call. This
    // guards the cross-node convergence path that broke in earlier re-stamp work.
    #[tokio::test]
    async fn internal_system_origin_preserves_timestamp() {
        let factory = make_factory();
        let conn_registry = Arc::new(ConnectionRegistry::new());
        let query_registry = Arc::new(QueryRegistry::new());
        let svc = Arc::new(CrdtService::new(
            Arc::clone(&factory),
            Arc::clone(&conn_registry),
            make_validator(),
            Arc::clone(&query_registry),
            Arc::new(SchemaService::new()),
        ));

        let key = "system-1";
        let mut ctx = OperationContext::new(1, service_names::CRDT, make_timestamp(), 5000);
        ctx.caller_origin = CallerOrigin::System;
        ctx.connection_id = None;
        ctx.principal = None;
        ctx.partition_id = Some(hash_to_partition(key));

        let record = topgun_core::LWWRecord {
            value: Some(rmpv::Value::String("from-peer".into())),
            timestamp: forged_timestamp(),
            ttl_ms: None,
        };
        let op = Operation::ClientOp {
            ctx,
            payload: topgun_core::messages::ClientOpMessage {
                payload: topgun_core::messages::base::ClientOp {
                    id: Some("system-put".to_string()),
                    map_name: "users".to_string(),
                    key: key.to_string(),
                    op_type: None,
                    record: Some(Some(record)),
                    or_record: None,
                    or_tag: None,
                    write_concern: None,
                    timeout: None,
                },
            },
        };

        svc.clone().oneshot(op).await.unwrap();

        let stored = read_lww_timestamp(&factory, "users", key)
            .await
            .expect("LWW record must be stored");
        assert_eq!(
            stored.millis, FORGED_MILLIS,
            "genuine internal/system origin must preserve the caller-supplied HLC \
             (convergence path), not re-stamp it"
        );
        assert_eq!(stored.node_id, FORGED_NODE_ID);
    }
}
