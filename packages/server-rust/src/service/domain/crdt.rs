//! CRDT domain service handling `ClientOp` and `OpBatch` operations.
//!
//! Merges LWW-Map and OR-Map data into the `RecordStore` and broadcasts
//! `ServerEvent` messages to subscribed client connections.

use std::collections::{BTreeMap, VecDeque};
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
use crate::storage::wal::OrDelta;
use crate::storage::{
    CallerProvenance, ExpiryPolicy, MutateOutcome, RecordStore, RecordStoreFactory,
};
use crate::tombstone_frontier::{Epoch, PruneEpochRecord, PruneExit, PrunePassRecord};
use crate::tombstone_frontier_impl::{TombstoneFrontier, TombstoneRef};
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
    /// which would silently drop one add (a lost-update race).
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
            // Last, so an unauthorised op is refused before its slot is read.
            self.admit_or_op(op, Some(partition_id), true).await?;
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
            self.admit_or_op(op, Some(partition_id), true).await?;
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
            self.admit_or_op(op, Some(partition_id), true).await?;
            Some(self.write_validator.sanitize_hlc())
        } else {
            // Genuine internal/system/forwarded call (trusted origin) — preserve
            // the caller's HLC so cross-node convergence is not perturbed.
            //
            // A trusted OR_ADD keeps its tag verbatim, so this branch is an
            // ingest path and runs the tag admission like the others: the OR-Map
            // Merkle leaf identifies a key's state only while no stored tag is
            // empty or carries a leaf separator (TG-MRK-001). The price falls on
            // whatever copies stored records between nodes: a tag stored before
            // the rule is refused on a node whose slot does not hold it. A
            // future replication, migration or backup-ingest path must therefore
            // not re-validate tags already stored on its source — it either does
            // not route through this admission or carries an explicit exemption.
            self.admit_or_op(op, Some(partition_id), false).await?;
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
            // An empty batch carries no operation id to acknowledge, so it is
            // answered with no frame: any acknowledgement here would name an id
            // the client never sent and retire writes nobody applied (TG-SYNC-004).
            return Ok(OperationResponse::Empty);
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
                // Last, so an unauthorised op is refused before its slot is read.
                self.admit_or_op(op, None, true).await?;
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
                // Last, so an unauthorised op is refused before its slot is read.
                self.admit_or_op(op, None, true).await?;
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
                self.admit_or_op(op, None, true).await?;
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
            //
            // A trusted OR_ADD keeps its tag verbatim, so this branch is an
            // ingest path and runs the tag admission like the others (TG-MRK-001),
            // for every op before the first one applies: a refused batch must
            // have applied nothing. The price falls on whatever copies stored
            // records between nodes: a tag stored before the rule is refused on
            // a node whose slot does not hold it. A future replication, migration
            // or backup-ingest path must therefore not re-validate tags already
            // stored on its source — it either does not route through this
            // admission or carries an explicit exemption.
            for op in ops {
                self.admit_or_op(op, None, false).await?;
            }
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

        // Determine the operation type and build the event payload. The class
        // comes from the one classifier the write admission also reads, so an
        // op can never be admitted as one kind and applied as another.
        let class = classify_op(op);
        let is_remove = class == OpClass::Remove;
        let is_or_add = class == OpClass::OrAdd;
        let is_or_remove = class == OpClass::OrRemove;

        if is_remove {
            // REMOVE/OR_REMOVE: no timestamp sanitization needed (removes are idempotent).
            // Held under the key's writer, the one every in-place write of the key
            // holds: the remove stages its durable delete before it empties the
            // engine, and an in-place write in between would mutate the still-
            // resident slot and re-stage it over that delete (TG-OR-007).
            let key_guard = self.key_writer.acquire(&op.map_name, &op.key).await;
            store
                .remove(&op.key, CallerProvenance::CrdtMerge)
                .await
                .map_err(OperationError::Internal)?;
            drop(key_guard);

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
            // clobbering the first's merge and losing an update. Held
            // across `store.get` through the single `store.put` merge-commit only —
            // does NOT cover the OR_REMOVE RMW below (342b's responsibility).
            let key_guard = self.key_writer.acquire(&op.map_name, &op.key).await;

            // Merge the new entry into the resident OR-Map slot IN PLACE rather
            // than reading a full clone, rebuilding, and re-putting the whole
            // ~130 KB snapshot every op. The add-wins / remove-wins algebra
            // itself lives in `apply_or_delta` — the single implementation every
            // OR op path routes through, so the resident slot and any later replay
            // of the same mutation cannot diverge. SYNC OR ingest
            // (`handle_ormap_push_diff`) merges with its own algebra: on a same-tag
            // conflict it keeps the stored record where `apply_or_delta` replaces it.
            //
            // `Option::take` moves the entry into the delta without a clone; the
            // closure runs exactly once (per key, under the writer lock above),
            // so the take can never come up empty (TG-OR-001).
            let mut new_entry_opt = Some(new_entry);
            // Read the witness demand ONCE per op, before the closure exists: when
            // nothing beneath the store consumes deltas the closure keeps the
            // zero-copy shape above and builds no witness at all, so the hot path
            // pays a predicate call rather than a clone.
            let witness_wanted = store.or_witness_wanted();
            let mut merge_add = move |value: &mut RecordValue| {
                // Upgrade a legacy non-OrMap resident slot (an OrTombstones blob
                // from an older server) to the unified OrMap shape first, matching
                // the prior get -> read_or_map_state -> put path — otherwise the add
                // is dropped and the legacy blob re-persisted unchanged. Its
                // shape-change report is irrelevant on this path: the closure
                // re-persists unconditionally below, so the write is already owed.
                normalize_to_or_map(value);
                let entry = new_entry_opt
                    .take()
                    .expect("OR_ADD merge closure runs exactly once");
                // A tag survives the move so the applied entry can be found again
                // afterwards; one `String`, and only when a consumer asked for a
                // witness at all.
                let witness_tag = if witness_wanted {
                    Some(entry.tag.clone())
                } else {
                    None
                };
                // The apply consumes the delta.
                let outcome = apply_or_delta(OrDelta::Add { entry }, value);
                // A witness, when one is owed, is read back out of the slot the
                // apply just wrote it into, so it is literally the post-image
                // entry rather than anything re-derived from a before/after
                // comparison. Remove-wins may have suppressed the add, and a
                // suppressed op has nothing to record: only `added` yields one.
                // `apply_or_delta` retains-then-pushes by tag, so at most one
                // resident entry can carry it.
                let witness_owed = witness_tag.is_some() && outcome.added;
                let witness = match (witness_tag, outcome.added, &*value) {
                    (Some(tag), true, RecordValue::OrMap { records, .. }) => records
                        .iter()
                        .find(|resident| resident.tag == tag)
                        .map(|applied| OrDelta::Add {
                            entry: applied.clone(),
                        }),
                    _ => None,
                };
                // An owed witness that comes back `None` is the silent-divergence
                // shape: the lookup missed, the caller falls back to a full
                // snapshot, and a consumer sees nothing while believing it is
                // being fed. Only two states can produce it — the apply reported
                // `added` without leaving the tag resident, or it left a shape
                // other than `OrMap` behind — and the algebra admits neither, so
                // the conjunction is enforced here rather than only argued.
                debug_assert!(
                    !witness_owed || witness.is_some(),
                    "an accepted OR_ADD owes a witness: the post-image lookup for \
                     the applied tag must find the entry the apply just wrote"
                );
                // Match the prior path, which always re-persisted the slot even
                // when remove-wins suppressed the add.
                MutateOutcome {
                    changed: true,
                    witness,
                }
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
                // Read the witness demand ONCE per op, before the closure exists
                // (see OR_ADD): with no consumer beneath the store nothing is
                // built.
                let witness_wanted = store.or_witness_wanted();
                let mut apply_remove = |value: &mut RecordValue| {
                    // Upgrade a legacy OrTombstones blob to OrMap first (see OR_ADD);
                    // otherwise the tombstone append is dropped on the upgrade path.
                    // Shape-change report ignored for the same reason as OR_ADD:
                    // this closure returns true unconditionally.
                    normalize_to_or_map(value);
                    // Only a genuinely-new tag is counted and epoch-stamped, which
                    // the apply reports back via `new_tombstone`.
                    let outcome = apply_or_delta(OrDelta::Remove { tag: tag.clone() }, value);
                    if outcome.new_tombstone {
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
                    // Only a genuinely-new tombstone changed the OR state, so only
                    // that op has anything to record; a duplicate remove hands back
                    // nothing.
                    let witness = if witness_wanted && outcome.new_tombstone {
                        Some(OrDelta::Remove { tag: tag.clone() })
                    } else {
                        None
                    };
                    // Match the prior path, which always re-persisted the slot even
                    // for a duplicate (already-tombstoned) remove.
                    MutateOutcome {
                        changed: true,
                        witness,
                    }
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

            // End of the critical section: the tombstone append and its epoch
            // stamp are both committed, and the payload built below only clones
            // locals, so nothing past this point needs the key held.
            drop(key_guard);

            // Ask for a prune pass; do NOT run one here. A pass walks every
            // eligible ref and re-acquires the per-key writer per dropped tag, so
            // on this timeline it would run inside the request's own timeout
            // budget: a writer held anywhere in the eligible set makes the budget
            // elapse, the layer cancels the call mid-pass, and the caller is told
            // the write timed out while the server kept it. This trigger is O(1)
            // and never awaits, so an op's latency no longer depends on the size
            // of the backlog it happens to follow. The write-triggered cadence is
            // unchanged — permits coalesce, so a burst still yields one pass after
            // it — for as long as the task consuming the wake lives; nothing supervises it.
            if let Some(frontier) = self.frontier.as_ref() {
                frontier.request_prune();
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

    /// Refuses `op` when it would store an OR tag the Merkle leaf cannot encode
    /// unambiguously (TG-MRK-001), unless the key's slot already holds that tag.
    ///
    /// Which tag is checked follows the op's class, read from the same
    /// classifier the apply uses, so the admission inspects exactly the tag the
    /// apply would store:
    ///
    /// - a whole-key remove and an LWW put store no tag: nothing is checked;
    /// - an OR add stores the record's tag only when `regenerates` is `false`
    ///   (the trusted branch); on a regenerating branch the server replaces the
    ///   client's tag, so nothing is checked. A co-present `or_tag` is ignored,
    ///   as the apply ignores it;
    /// - an OR remove stores `or_tag` as a tombstone on every branch.
    ///
    /// `partition_id` is the partition the apply will use when the caller
    /// already knows it (`Some`, the single-op path). `None` means the apply
    /// derives it from the key (the batch path), and then so does the lookup —
    /// but only inside the store resolver, which runs for an inadmissible tag
    /// alone. An op that checks nothing, and an op whose tag is admissible,
    /// therefore hash no key, resolve no store and read nothing: the LWW hot
    /// path pays the classification and one branch.
    ///
    /// Must run after the op's own authorisation and schema checks (an
    /// unauthorised op must not cause a store read) and, for a batch, for every
    /// op before the first one applies.
    async fn admit_or_op(
        &self,
        op: &ClientOp,
        partition_id: Option<u32>,
        regenerates: bool,
    ) -> Result<(), OperationError> {
        let tag = match classify_op(op) {
            OpClass::OrAdd if !regenerates => match &op.or_record {
                Some(Some(record)) => record.tag.as_str(),
                _ => return Ok(()),
            },
            OpClass::OrRemove => match &op.or_tag {
                Some(Some(tag)) => tag.as_str(),
                _ => return Ok(()),
            },
            OpClass::Remove | OpClass::OrAdd | OpClass::Lww => return Ok(()),
        };
        admit_or_tags(
            || {
                let partition_id = partition_id.unwrap_or_else(|| hash_to_partition(&op.key));
                self.record_store_factory
                    .get_or_create(&op.map_name, partition_id)
            },
            &op.map_name,
            &op.key,
            std::iter::once(tag),
        )
        .await
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
// Op classification
// ---------------------------------------------------------------------------

/// What a `ClientOp` does to its key's slot, which decides both how it is
/// applied and which of its tags (if any) it would store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpClass {
    /// Whole-key remove: drops the slot and stores no tag.
    Remove,
    /// OR-Map add: stores one live record under a tag.
    OrAdd,
    /// OR-Map remove: stores its tag as a tombstone.
    OrRemove,
    /// Last-Write-Wins put: stores no tag.
    Lww,
}

/// Classifies `op` by the fields it carries, in priority order:
///
/// 1. explicit `op_type == "REMOVE"`, or a tombstone `record: Some(None)` → [`OpClass::Remove`];
/// 2. `or_record: Some(Some(_))` → [`OpClass::OrAdd`], whatever `or_tag` says;
/// 3. `or_tag: Some(Some(_))` with `or_record` absent → [`OpClass::OrRemove`]
///    (`or_record: Some(None)` beside an `or_tag` is not a remove);
/// 4. anything else → [`OpClass::Lww`].
///
/// Field inspection only: it runs once per op on the write hot path.
fn classify_op(op: &ClientOp) -> OpClass {
    if op.op_type.as_deref() == Some("REMOVE") || matches!(&op.record, Some(None)) {
        OpClass::Remove
    } else if matches!(&op.or_record, Some(Some(_))) {
        OpClass::OrAdd
    } else if matches!(&op.or_tag, Some(Some(_))) && op.or_record.is_none() {
        OpClass::OrRemove
    } else {
        OpClass::Lww
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
///
/// Returns `true` iff the slot's shape actually changed (a non-`OrMap` slot was
/// replaced), and `false` when the slot was already `OrMap`. A shape change IS a
/// value change, so a caller whose closure returns "was anything modified?" owes
/// a durable write for it: reporting `false` there would leave the resident slot
/// upgraded with a stale `metadata.cost` while the durable record keeps the legacy
/// shape until some unrelated later write happens to re-persist it.
pub(crate) fn normalize_to_or_map(value: &mut RecordValue) -> bool {
    if matches!(value, RecordValue::OrMap { .. }) {
        return false;
    }
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
    true
}

/// Why `tag` may not enter a slot as an OR-Map tag, or `None` when it may.
///
/// The OR-Map Merkle leaf joins a key's tags with `|` and separates its live
/// tags from its tombstones with `#`. Equal leaves imply equal (live set,
/// tombstone set) for a key only while every stored tag is non-empty and
/// carries neither character (TG-MRK-001), so a tag is admissible iff it is
/// non-empty and contains neither `|` nor `#`.
///
/// This is the only definition of the rule and of its two reason strings;
/// every ingest path reaches it through [`admit_or_tags`]. Pure, and it never
/// returns the tag: a refused tag is caller-chosen text.
#[must_use]
pub(crate) fn or_tag_refusal(tag: &str) -> Option<&'static str> {
    if tag.is_empty() {
        Some("OR tag is empty")
    } else if tag.contains(['|', '#']) {
        Some("OR tag contains a reserved character ('|' or '#')")
    } else {
        None
    }
}

/// Whether a slot's value holds `tag`: as the tag of a live record or as a
/// tombstone of an `OrMap` value, or as a tag of a legacy `OrTombstones` value.
/// An absent key and an `Lww` value hold no tag.
fn slot_holds_or_tag(value: Option<&RecordValue>, tag: &str) -> bool {
    match value {
        Some(RecordValue::OrMap {
            records,
            tombstones,
        }) => {
            records.iter().any(|entry| entry.tag == tag)
                || tombstones.iter().any(|held| held == tag)
        }
        Some(RecordValue::OrTombstones { tags }) => tags.iter().any(|held| held == tag),
        Some(RecordValue::Lww { .. }) | None => false,
    }
}

/// Admits the OR `tags` a request would store verbatim under `key`, refusing
/// the request at the first tag that is inadmissible (see [`or_tag_refusal`])
/// and that `key`'s slot does not already hold.
///
/// A tag the slot already holds is admitted whatever it contains: a tag stored
/// before the rule existed must stay removable and idempotently re-sendable,
/// and admitting it adds nothing the slot did not have. "Holds" is decided on
/// the slot as read here, once, with `RecordStore::get(key, false)`, which
/// also sees a key that is durable but not resident.
///
/// # Cost
///
/// Everything the lookup needs is resolved lazily. For admissible tags the
/// whole call is an emptiness check and a character search per tag: `store` is
/// not called and nothing is read. `store` is called at most once per call,
/// and only once some tag has a refusal reason; the slot is read at most once
/// per call, however many inadmissible tags follow.
///
/// # Contract for the caller
///
/// - Call it for every key of the request **before anything of that request
///   is applied** and before the key's writer is taken. It takes no writer and
///   makes no durable write, so it is safe ahead of the apply; it is not
///   strictly read-only, because reading a non-resident key brings its record
///   into memory.
/// - `store` must resolve the **same store the apply of this request will use
///   for `key`** (the same map and the same partition), or the lookup inspects
///   a slot the apply never touches. It is a plain closure and cannot fail:
///   resolving a store only looks up, or builds in memory, the map's partition
///   store, and involves no I/O. The one fallible step is the read, and this
///   function owns it.
/// - `tags` must yield every tag the request would store verbatim under `key`
///   (for a pushed entry: its record tags, then its tombstone tags), and no tag
///   the server will replace with one it generates.
/// - Return the error as the request's result. Neither error may be turned
///   into a success, and the two must not be confused: one is permanent, the
///   other is retried.
///
/// The verdict describes the slot at the time of the read. A writer that drops
/// the tag between this call and the apply can let the request re-add a tag
/// the slot held at admission; no interleaving lets in a tag it did not hold.
///
/// # Errors
///
/// - [`OperationError::SchemaInvalid`] naming `map_name`, with one error string
///   that gives the key and the reason and never echoes the tag: an
///   inadmissible tag the slot does not hold.
/// - [`OperationError::Internal`]: the slot could not be read. An unreadable
///   slot is never treated as holding the tag, and never as a refusal.
pub(crate) async fn admit_or_tags<'t, S, T>(
    store: S,
    map_name: &str,
    key: &str,
    tags: T,
) -> Result<(), OperationError>
where
    S: FnOnce() -> Arc<dyn RecordStore>,
    T: IntoIterator<Item = &'t str>,
{
    // Taken on the first inadmissible tag, so the store is resolved and the
    // slot read at most once per call.
    let mut resolve = Some(store);
    let mut slot: Option<RecordValue> = None;
    for tag in tags {
        let Some(reason) = or_tag_refusal(tag) else {
            continue;
        };
        if let Some(resolve) = resolve.take() {
            slot = resolve()
                .get(key, false)
                .await
                .map_err(OperationError::Internal)?
                .map(|record| record.value);
        }
        if !slot_holds_or_tag(slot.as_ref(), tag) {
            return Err(OperationError::SchemaInvalid {
                map_name: map_name.to_string(),
                errors: vec![format!("key '{key}': {reason}")],
            });
        }
    }
    Ok(())
}

/// What one [`apply_or_delta`] call did.
///
/// Callers key their own side effects (the tombstone-bytes gauge, the epoch
/// frontier stamp, the "re-persist this slot" signal) off these flags, which is
/// what lets the apply itself stay pure.
///
/// Deliberately carries no copy of the delta: the apply consumes its argument,
/// and an accepted add leaves the resident slot as the entry's only owner. A
/// caller that needs to record what it applied reads its own copy back out of
/// the post-image slot AFTER these flags report an effect, and only when a
/// consumer beneath the store has demanded a witness at all — so the copy that
/// gives an entry a second owner costs one entry per *effective* op under an
/// armed consumer, and nothing whatsoever where no store demands one. The flags
/// below are the only thing a caller cannot derive for itself.
#[derive(Debug)]
pub(crate) struct OrApplyOutcome {
    /// `Add`: the entry was inserted (`false` = suppressed by remove-wins), which
    /// is what tells a witness-building caller a suppressed add apart from an
    /// applied one.
    pub added: bool,
    /// `Remove`: a genuinely-new tombstone tag was appended (`false` = duplicate remove).
    pub new_tombstone: bool,
    /// `Prune`: how many tombstone tags were actually dropped.
    pub pruned: usize,
}

/// Applies exactly ONE [`OrDelta`] to a resident OR-Map slot, consuming it.
///
/// Takes the delta BY VALUE and moves its payload straight into the slot, so an
/// accepted add transfers the entry exactly once with no copy; the delta is gone
/// afterwards, by design. A caller that must durably record what it applied
/// therefore keeps its own delta-sized copy — for an add, the post-image entry
/// recovered from the slot by tag; for a remove or a prune, the tag or tag list
/// it already holds — and hands it back as the mutate closure's witness. That
/// copy is taken only where a consumer beneath the store has demanded a witness
/// and only after this function has reported an effect, so a path with no
/// consumer allocates nothing for it.
///
/// This is the ONE implementation of the add-wins / remove-wins / prune algebra:
/// the live `OR_ADD`, `OR_REMOVE` and epoch-prune paths all route through it, so a
/// recovery fold that delegates here cannot drift from what the write path did
/// (TG-OR-003). The algebra matches core-rust `ORMap` (retain survivors, skip
/// tombstoned re-adds); it lives here rather than wiring core-rust `ORMap`
/// because that primitive owns its own HLC + Merkle and is keyed map-wide.
///
/// PURE: no gauge counters, no tombstone frontier, no I/O. Every side effect
/// belongs to the caller at the position it occupies there — in particular the
/// prune decrement must stay outside this function, because it may only fire
/// once the durable write has succeeded, so that the gauge tracks bytes actually
/// resident rather than bytes removed from an in-memory copy (TG-OR-004).
///
/// A `value` that is not [`RecordValue::OrMap`] is a no-op: callers normalize the
/// resident slot first, because upgrading a legacy storage shape is a concern of
/// the slot, not of the algebra.
pub(crate) fn apply_or_delta(delta: OrDelta, value: &mut RecordValue) -> OrApplyOutcome {
    let mut added = false;
    let mut new_tombstone = false;
    let mut pruned = 0usize;

    if let RecordValue::OrMap {
        records,
        tombstones,
    } = value
    {
        match delta {
            OrDelta::Add { entry } => {
                // Remove-wins: a tag already observed-removed is never resurrected.
                if !tombstones.contains(&entry.tag) {
                    // Remove any existing entry with the same tag (idempotent re-add).
                    records.retain(|e| e.tag != entry.tag);
                    // Moved, not copied: the slot becomes the entry's only owner.
                    records.push(entry);
                    added = true;
                }
            }
            OrDelta::Remove { tag } => {
                // Drop only the matched tag, preserving every concurrent survivor.
                records.retain(|e| e.tag != tag);
                // Dedup: a re-issued remove must not duplicate the tombstone, so
                // only a genuinely-new tag is reported back to the caller (which
                // is what keeps its gauge and epoch stamp exactly-once).
                if !tombstones.contains(&tag) {
                    tombstones.push(tag);
                    new_tombstone = true;
                }
            }
            OrDelta::Prune { tags } => {
                let before = tombstones.len();
                tombstones.retain(|t| !tags.contains(t));
                pruned = before - tombstones.len();
            }
        }
    }

    OrApplyOutcome {
        added,
        new_tombstone,
        pruned,
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
/// the OR write path in this file, all of it inside [`apply_or_delta`]:
///
/// - `Add` builds `records` with `records.retain(|e| e.tag != entry.tag)` then
///   `records.push(...)` — the vector is in **operation-insertion order**, never
///   sorted.
/// - `Remove` appends with `tombstones.push(tag)` — also insertion order.
/// - `Prune` `retain`s in place, preserving whatever order was there.
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

/// Owner of one prune pass's un-settled tombstone refs and of that pass's whole ledger.
///
/// A prune pass is a future, and its caller may stop polling it at any await. A ref that
/// has left the index but has not yet been settled is then named by nobody: the drain
/// removes the index entry BEFORE the tag is dropped from storage, so no later sweep can
/// retry that tag and the tombstone bytes it names stay in storage for the life of the
/// process. Keeping those refs in here is what makes that unreachable — `Drop` runs on the
/// cancellation path exactly as it runs on the normal one, and hands back every ref still
/// inside.
///
/// The ledger lives here for the same reason. A pass that emitted its records from the end
/// of the function body would emit nothing at all when cancelled, so the pass-level series
/// would silently describe only the passes that happened to finish — the one population
/// whose behaviour is already known to be fine.
struct PrunePassGuard<'a> {
    /// The index every un-settled ref goes back into.
    frontier: &'a TombstoneFrontier,
    /// The ref the loop body is working on right now, if any.
    ///
    /// Held here rather than moved out to the body, so a cancellation at any of that
    /// body's three awaits still finds it. This slot is the whole difference between "the
    /// pass stopped while working on a ref" and "that ref is gone".
    in_flight: Option<(Epoch, TombstoneRef)>,
    /// The refs the loop has not begun yet, in drain order.
    pending: VecDeque<(Epoch, TombstoneRef)>,
    /// The pass ledger. Emitted from `Drop`, never from the loop.
    pass: PrunePassRecord,
    /// The per-epoch ledger. Emitted from `Drop`, never from the loop.
    per_epoch: BTreeMap<Epoch, PruneEpochRecord>,
}

impl<'a> PrunePassGuard<'a> {
    /// Take ownership of a drain's refs. Called before the pass can await anything.
    fn new(frontier: &'a TombstoneFrontier, drained: Vec<(Epoch, TombstoneRef)>) -> Self {
        // One record per invocation, empty drains included: a pass that drains nothing is
        // exactly the regime this record has to be able to describe, so the pass is counted
        // here rather than behind any eligibility or per-ref condition. A pass counted
        // inside the loop would read zero during a total stall.
        let empty_drain = drained.is_empty();
        Self {
            frontier,
            in_flight: None,
            pending: drained.into(),
            pass: PrunePassRecord {
                empty_drain,
                ..PrunePassRecord::default()
            },
            per_epoch: BTreeMap::new(),
        }
    }

    /// Begin the next ref and hand the loop body a COPY of it to work with.
    ///
    /// The ref itself stays in `in_flight` until `settle` records its exit, which is what
    /// makes the body's three awaits cancellation-safe: whatever the body was in the middle
    /// of, the ref is still inside the guard. The copy is how that coexists with a body
    /// that also has to call `&mut self` methods on the guard — borrowing the ref out of
    /// the guard across those awaits would forbid it. Three short strings cloned per ref is
    /// far below the cost of the storage read and write the body is about to do with them.
    fn begin_next_ref(&mut self) -> Option<(Epoch, TombstoneRef)> {
        self.in_flight = self.pending.pop_front();
        self.in_flight.clone()
    }

    /// Record one ref's exit. This is the ONLY way a ref leaves this guard.
    ///
    /// `considered` and the exit's own counter move together here, on the pass record and
    /// on the epoch record alike, so `considered == Σ exits` holds after every individual
    /// call rather than only at the end of a completed loop. Counting a ref as considered
    /// where the body BEGINS it would break the identity on precisely the path the
    /// cancelled exit exists for: a pass stopped while awaiting the in-flight ref would
    /// have counted that ref and could never record an exit for it.
    ///
    /// `bytes_freed` is part of the exit rather than a separate credit, because only
    /// [`PruneExit::Dropped`] frees anything and a byte total recorded apart from the exit
    /// that earned it is free to drift from it.
    fn settle(&mut self, epoch: Epoch, exit: PruneExit, bytes_freed: u64) {
        // The in-flight slot empties HERE and nowhere else. On the cancellation path it is
        // already empty (`Drop` took it), and clearing it again is a no-op.
        self.in_flight = None;
        let epoch_record = self.per_epoch.entry(epoch).or_insert(PruneEpochRecord {
            epoch,
            ..PruneEpochRecord::default()
        });
        self.pass.considered += 1;
        epoch_record.considered += 1;
        self.pass.bytes_freed += bytes_freed;
        epoch_record.bytes_freed += bytes_freed;
        // One `+= 1` pair for every exit, so the two ledgers cannot be wired to different
        // exits by an edit that touches only one of them.
        let (pass_exit, epoch_exit) = match exit {
            PruneExit::Dropped => (&mut self.pass.dropped, &mut epoch_record.dropped),
            PruneExit::MatchedNothing => (
                &mut self.pass.matched_nothing,
                &mut epoch_record.matched_nothing,
            ),
            PruneExit::AbsentKey => (&mut self.pass.absent, &mut epoch_record.absent),
            PruneExit::RestoredReadError => (
                &mut self.pass.restored_read_error,
                &mut epoch_record.restored_read_error,
            ),
            PruneExit::RestoredEvicted => (
                &mut self.pass.restored_evicted,
                &mut epoch_record.restored_evicted,
            ),
            PruneExit::RestoredWriteError => (
                &mut self.pass.restored_write_error,
                &mut epoch_record.restored_write_error,
            ),
            PruneExit::RestoredCancelled => (
                &mut self.pass.restored_cancelled,
                &mut epoch_record.restored_cancelled,
            ),
        };
        *pass_exit += 1;
        *epoch_exit += 1;
    }
}

impl Drop for PrunePassGuard<'_> {
    /// The pass's single emission site, and the only part of it that survives a dropped
    /// future.
    ///
    /// Nothing here can panic: the restore is infallible and its lock recovers from
    /// poisoning, and `tracing` and `metrics` calls do not panic. That is a property to
    /// hold rather than an accident — a panic in a `drop` during an unwind aborts the
    /// process, and this runs on a path a caller reaches by giving up on the pass.
    fn drop(&mut self) {
        // Both values come OUT of `self` first, so the chain below borrows nothing while
        // the loop settles back into `self`.
        let in_flight = self.in_flight.take();
        let pending = std::mem::take(&mut self.pending);
        // ONE chained iteration, the in-flight ref ahead of the remainder. Two loops would
        // be two restore call sites, and a second restore path is exactly how a divergent
        // one gets added later.
        for (epoch, r) in in_flight.into_iter().chain(pending) {
            // Settled BEFORE the restore, so a ref has already left the guard by the time
            // it re-enters the index and no ref can be handed back twice.
            self.settle(epoch, PruneExit::RestoredCancelled, 0);
            self.frontier.restore_tombstone_ref(epoch, r);
        }
        if self.pass.restored_cancelled > 0 {
            // Emitted ahead of the pass row below, which stays the last row a pass emits.
            tracing::warn!(
                restored_cancelled = self.pass.restored_cancelled,
                "prune pass cancelled; {} un-settled refs re-indexed for the next pass",
                self.pass.restored_cancelled
            );
        }
        self.pass.epochs_drained = self.per_epoch.len() as u64;
        for epoch_record in self.per_epoch.values() {
            self.frontier
                .prune_observer()
                .observe_drained_epoch(epoch_record);
            // One settlement line per drained epoch, joined to that epoch's exit row by
            // `epoch` — a field populated on every row of both ledgers, so the join needs
            // no wall clock. This is the only place the per-epoch seven-exit identity is
            // observable end to end: `PruneEpochRecord` has no Prometheus series of its
            // own, so without this line the per-epoch counters would be provably correct
            // in-process yet unreadable by anything outside it.
            tracing::info!(
                target: "topgun_server::tombstone_frontier::settlement",
                epoch = epoch_record.epoch,
                considered = epoch_record.considered,
                dropped = epoch_record.dropped,
                matched_nothing = epoch_record.matched_nothing,
                absent = epoch_record.absent,
                restored_read_error = epoch_record.restored_read_error,
                restored_evicted = epoch_record.restored_evicted,
                restored_write_error = epoch_record.restored_write_error,
                restored_cancelled = epoch_record.restored_cancelled,
                bytes_freed = epoch_record.bytes_freed,
                "prune epoch settlement"
            );
        }
        // Exactly one pass observation per invocation — the recorder counts the pass
        // itself, so a second or a conditional call would break the pass identity.
        self.frontier.prune_observer().observe_pass(&self.pass);
        // The pass row: `considered` and `empty_drain` have no other `tracing` transport,
        // so neither term is readable in-process by anything that cannot bind the
        // (permanently no-op, outside its own recorder binding) Prometheus handles. Fires
        // on EVERY pass, empty drains and cancelled passes included, unconditionally —
        // that unconditional placement is what makes this row individuate a pass on the
        // capture: it is always the last row a pass emits.
        tracing::info!(
            target: "topgun_server::tombstone_frontier::residency",
            kind = "prune_pass",
            considered = self.pass.considered,
            empty_drain = self.pass.empty_drain,
            "prune pass"
        );
    }
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
/// `is_epoch_prune_eligible(E)` (derived from the reclamation ceiling, which is
/// the fleet MIN over live claims less the safety margin) AND
/// `durable_epoch_watermark >= E`. Neither is a fixed constant — the ceiling is
/// recomputed from the live claim set per query, so it rises as tracked clients
/// confirm-apply and falls when a laggard rejoins, and the durable watermark
/// advances as the durable backend catches up, so the drained set grows over
/// time in production rather than staying permanently empty. Shared by the OR
/// write path (`crdt.rs`) and the SYNC leaf (`sync.rs`).
///
/// A ref whose storage drop FAILS (read or write error) is handed back to the
/// frontier via `restore_tombstone_ref` so a later sweep retries it — dropping
/// it here would orphan the tag un-prunable in storage forever, since the drain
/// already removed its index entry. The same restore covers the one non-error
/// case that also reclaimed nothing: the key was evicted between the residency
/// check and the in-place write, so the mutate closure never ran. That is told
/// apart from "the closure ran and the tag was already gone" — which must NOT be
/// restored, or the prune loop livelocks on it — by a flag the closure itself
/// sets, because the `Ok(bool)` from `update_in_place` conflates the two.
///
/// Every ref leaves the loop body through exactly one of seven counted exits, so
/// `considered == dropped + matched_nothing + absent + restored_read_error +
/// restored_evicted + restored_write_error + restored_cancelled` holds by
/// construction. The tombstone-byte
/// decrement stays where it already was — in the post-write success arm, behind
/// `dropped` — because a decrement moved to follow the ledger would credit bytes the
/// durable write never actually freed.
///
/// The seventh of those exits is not recorded by this body at all. The pass is a future
/// its caller may stop polling at any of the loop's three awaits, so every ref the drain
/// returned is held by [`PrunePassGuard`] until its own exit is recorded; whatever is
/// still inside the guard when the future is dropped is re-indexed and counted as
/// `restored_cancelled` by the guard's `Drop`. That is also the single site every emission
/// happens from — the per-epoch settlement rows, the pass observation and the pass row all
/// fire from there, on the cancelled path exactly as on the completed one, so a cancelled
/// pass is a recorded pass rather than a silent gap in the series.
///
/// # WHAT THIS PASS RECORD CANNOT TELL YOU (read before drawing a conclusion from it)
///
/// The pass's own terms are built from the drain's RETURN VALUE, so all three of
/// `considered`, `empty_drain` and `epochs_drained` are blind to a distinction that
/// matters:
///
/// - `considered` increments once per ref the drain RETURNED, inside the loop below.
/// - `empty_drain` is `drained.is_empty()`, over that same returned vector.
/// - `epochs_drained` is the size of the per-epoch map, which only the same loop fills.
///
/// So `considered = 0`, `empty_drain = true`, `epochs_drained = 0` is what this pass
/// reports for BOTH of these, and it CANNOT DISTINGUISH THEM:
///
/// 1. no epoch was eligible, so nothing was removed at all; and
/// 2. an eligible epoch WAS removed from the index and returned zero refs.
///
/// Case 2 is not hypothetical — the frontier attributes that removal as a completed
/// drain and reports entry-side bytes for it, while this record reports the pass as
/// having done nothing. The two components are each internally coherent and disagree
/// with each other, because one emits at the removal site and the other reads the
/// return value.
///
/// Nothing here is widened to close that: the discriminating fact is emitted at the
/// removal site itself, on its own `tracing` target, carrying what the removal
/// returned beside what the slot recorded at entry. Read that line when this record
/// says a pass did nothing. Reconciling the two ledgers is deferred and tracked in
/// `TODO-634`.
// Split along exactly one seam, and this is the side that stays: the loop, its three
// awaits and the tombstone-byte decrement live in this body, so the "exactly one
// decrement, in the post-write arm behind `dropped`" siting is still assertable
// against this function. What moved into `PrunePassGuard` is the ledger's STORAGE and
// its EMISSION, because the guard is the only part of a pass that survives a dropped
// future: a ledger owned by this body would be lost on the very path the cancelled
// exit exists to describe, and the refs it had not yet settled would be lost with it.
#[allow(clippy::too_many_lines)]
pub(crate) async fn prune_epoch_tombstones(
    frontier: &TombstoneFrontier,
    factory: &RecordStoreFactory,
    key_writer: &KeyWriterRegistry,
) {
    let drained = frontier.drain_prunable_tombstones();
    // The epoch/watermark state is NOT read here. The drain has already released the
    // frontier lock, so three accessor calls would be three independent acquisitions
    // and could tear against a concurrent ACK; the frontier publishes that state
    // itself, from a snapshot taken under the drain's own lock.
    //
    // The drained vector moves into the guard before this function can await anything:
    // there is no suspension point between the two statements, so there is no instant
    // at which a ref is outside both the index and the guard.
    let mut guard = PrunePassGuard::new(frontier, drained);

    while let Some((epoch, r)) = guard.begin_next_ref() {
        let store = factory.get_or_create(&r.map, hash_to_partition(&r.key));
        // Serialize the drop against concurrent OR writes on this key.
        let _key_guard = key_writer.acquire(&r.map, &r.key).await;
        // Residency check before the in-place drop. A resident key needs no read:
        // the in-place write below mutates it, and materializes it itself should it
        // be evicted in between (TG-OR-007), so a read would only clone the whole
        // slot and drop the copy. A NON-resident key still goes through the store's
        // read, for the two roles the write cannot play: telling a key gone
        // everywhere (`AbsentKey`, ref consumed) apart from one the write failed to
        // reach (`RestoredEvicted`, ref re-indexed) — init=None on an absent key
        // never runs the closure, so without the read that ref would be re-indexed on
        // every pass forever — and surfacing a backend read error as
        // `RestoredReadError` so the ref is retried.
        if !store.exists_in_memory(&r.key) {
            match store.get(&r.key, false).await {
                Ok(Some(_)) => {}
                // Truly gone (no resident and no durable record): nothing to reclaim.
                // The ref is consumed WITHOUT a tombstone-byte decrement, so it is
                // counted apart from a drop rather than folded into a "not dropped"
                // bucket: a growing share here is a candidate mechanism for a falling
                // reclaim fraction that no other instrument can see.
                Ok(None) => {
                    guard.settle(epoch, PruneExit::AbsentKey, 0);
                    continue;
                }
                Err(e) => {
                    tracing::warn!(map = %r.map, key = %r.key, epoch, "prune read failed, re-indexing tombstone for retry: {e}");
                    guard.settle(epoch, PruneExit::RestoredReadError, 0);
                    frontier.restore_tombstone_ref(epoch, r);
                    continue;
                }
            }
        }
        // Drop the tag from the key's tombstone set IN PLACE (init=None → mutate
        // only a record that exists, never create one), owing a durable write
        // only when a tombstone was actually removed.
        let mut dropped = false;
        // Whether the mutate closure ran at all. `update_in_place` reports only
        // `Ok(bool)` here, which cannot tell "the key was evicted between the
        // residency check above and this write, so nothing was mutated" apart from
        // "the closure ran and found no matching tag". Those two need opposite
        // dispositions below, and only the closure itself knows which happened.
        let mut ran = false;
        // Read the witness demand ONCE per op, before the closure exists (see the
        // OR_ADD path): with no consumer beneath the store nothing is built.
        let witness_wanted = store.or_witness_wanted();
        let result = {
            let mut drop_tag = |value: &mut RecordValue| {
                ran = true;
                // Upgrade a legacy OrTombstones blob to OrMap first, as the live
                // add/remove closures do — otherwise the prune would fail its
                // OrMap match and silently re-persist the legacy blob. Not
                // reachable today (the drop below is the only mutation on this
                // path), so this is a defence against a future caller, not a fix.
                let shape_changed = normalize_to_or_map(value);
                // Same apply as the live add/remove paths, so a pruned tag is
                // removed by exactly the algebra the write path defines. The
                // gauge decrement deliberately stays OUT of the closure (see the
                // post-write arm below).
                let tags = vec![r.tag.clone()];
                // The apply consumes the tag list, so a second copy is retained
                // across it -- but only when a consumer asked for a witness.
                let witness_tags = if witness_wanted {
                    Some(tags.clone())
                } else {
                    None
                };
                let outcome = apply_or_delta(OrDelta::Prune { tags }, value);
                dropped = outcome.pruned > 0;
                // A sweep that matched nothing changed no OR state and so has
                // nothing to record, even where it still owes a write for the
                // shape upgrade below.
                let witness = match witness_tags {
                    Some(tags) if outcome.pruned > 0 => Some(OrDelta::Prune { tags }),
                    _ => None,
                };
                // A normalize that upgraded the shape owes a durable write even
                // with nothing pruned: the production store keeps the closure's
                // mutation to the resident slot on a `false` return but skips the
                // re-cost and the write-through, which would leave the slot
                // upgraded with a stale cost and no durable counterpart.
                MutateOutcome {
                    changed: shape_changed || dropped,
                    witness,
                }
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
                    guard.settle(epoch, PruneExit::Dropped, r.tag.len() as u64);
                }
                // The closure never ran: the key left memory between the
                // residency check and this write and the store could not
                // materialize it again (the default store does, and reclaims the
                // tag), so init=None mutated nothing while a durable tombstone
                // may well still exist. Hand the ref
                // back so a later sweep retries it — the drain already removed
                // its index entry, so dropping it here would orphan the tag
                // un-prunable forever. Keyed off `ran` and never off the
                // `update_in_place` bool, so a normalize-only write (which
                // reports "changed" with nothing pruned) cannot livelock the
                // loop by getting the ref restored on every pass. `dropped`
                // implies `ran`, so the gauge above and this arm are exclusive.
                // `dropped` implies `ran`, so these three dispositions are mutually
                // exclusive and together cover every way out of a successful write —
                // which is what makes the exit ledger sum back to `considered`.
                if !ran {
                    tracing::debug!(
                        map = %r.map,
                        key = %r.key,
                        epoch,
                        "prune found key evicted mid-write, re-indexing tombstone for retry"
                    );
                    guard.settle(epoch, PruneExit::RestoredEvicted, 0);
                    frontier.restore_tombstone_ref(epoch, r);
                } else if !dropped {
                    // The closure ran and matched no tag; it may still have owed the
                    // shape-upgrade write above, but no tombstone was reclaimed.
                    guard.settle(epoch, PruneExit::MatchedNothing, 0);
                }
            }
            Err(e) => {
                // Operator-visible: a swallowed storage error on the prune path
                // would silently stall tombstone reclamation.
                tracing::warn!(map = %r.map, key = %r.key, epoch, "prune update failed, re-indexing tombstone for retry: {e}");
                guard.settle(epoch, PruneExit::RestoredWriteError, 0);
                frontier.restore_tombstone_ref(epoch, r);
            }
        }
    }

    // Nothing is emitted here. `guard` goes out of scope on the next line and its
    // `Drop` writes the whole ledger out: on this path with nothing left pending and
    // `restored_cancelled == 0`, on the cancelled path with whatever the pass never
    // settled. One emission site, reached the same way either way.
}

/// The spawned prune task's hold on its frontier's single-flight claim.
///
/// Owned BY the spawned future rather than released by code placed after the loop, because the
/// loop has no normal exit: every way the task ends — `JoinHandle::abort` before or after the
/// first poll, runtime teardown, a panic that escapes the pass boundary — ends it by DROPPING
/// the future. A drop guard is the only release site every one of those routes passes through.
struct PruneTaskLease {
    frontier: Arc<TombstoneFrontier>,
}

impl Drop for PruneTaskLease {
    fn drop(&mut self) {
        // Warned before the release, so the row is emitted while the frontier still reads as
        // claimed. Operator-visible because a prune task that is gone is not a degraded mode
        // that heals: every trigger afterwards only leaves a permit nobody consumes.
        tracing::warn!(
            "the prune task exited; no tombstone reclamation runs for this frontier until a \
             prune task is spawned again. Expected during runtime teardown, which is how a \
             graceful shutdown ends this task"
        );
        // Last statement, and panic-free by contract: a panic in a drop that is itself running
        // during an unwind aborts the process.
        self.frontier.release_prune_task();
    }
}

/// The panic payload's own message, for attribution in the caught-panic row.
///
/// `panic!` with a literal yields a `&'static str` payload and a formatted one a `String`;
/// anything else is a payload this process does not produce, and naming it is more useful than
/// rendering nothing.
fn panic_payload_message(payload: &(dyn std::any::Any + Send)) -> &str {
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        message
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.as_str()
    } else {
        "<panic payload was neither &str nor String>"
    }
}

/// Spawn the ONE long-lived prune task for this frontier.
///
/// Returns `None`, having spawned nothing, when a task was already claimed for this frontier,
/// so a second call cannot put a second concurrent pass over the same index. Two loops would
/// each drain refs the other never sees settle. Must be called from inside a tokio runtime.
///
/// The pass belongs here rather than on a caller's timeline because a request-path pass is a
/// future under that request's own timeout budget: a slow pass makes the budget elapse, the
/// layer drops the future mid-pass, and the work is cancelled for reasons that have nothing to
/// do with the op that happened to trigger it. Nothing on this task wraps the pass in a
/// timeout and nothing here runs under a request budget, so a pass ends when it finishes, when
/// it panics, or at runtime teardown — and never because some unrelated caller ran out of time.
///
/// A frontier wired into a service **without** this spawn never prunes at all: every
/// `TombstoneFrontier::request_prune` then only leaves a permit nobody ever consumes, while
/// each trigger site keeps returning successfully and the wiring keeps looking correct. That is
/// deliberately the shape every unit-test fixture has, and it is why this call is the required
/// partner of the trigger: a production assembly that omits it reclaims nothing, silently.
///
/// The task owns its three `Arc`s for the lifetime of the process. Whatever assembled them
/// keeps them alive anyway, so this closes no reference cycle and needs no `Weak`. Runtime
/// teardown drops the task, and `PrunePassGuard` re-indexes the in-flight pass's unsettled refs
/// as that future is dropped, so a torn-down pass loses no ref from the index — in RAM only;
/// the restart rebuild remains the authoritative recovery.
///
/// # A pass that panics
///
/// The panic is caught at the pass boundary, around the `poll` of ONE pass future, so it ends
/// that pass and not this task. By the time the catch returns, the guard has already re-indexed
/// every ref the pass never settled — counted as restored-cancelled, because what the guard
/// observed is a pass future that stopped being polled, and unwinding is one of the two ways
/// that happens. A `warn!` then names the panic's own message and the number of refs this pass
/// re-indexed, and the task goes back to waiting for the next permit. It does NOT re-trigger
/// itself: a deterministically panicking pass would otherwise become a hot loop, and waiting is
/// the same cadence the task has for every other pass.
///
/// What a LATER pass does then depends on where the panic fired.
///
/// - **After the engine mutation** — anywhere in the post-mutation window of the record store's
///   in-place write: the observer notifications, the datastore write-through and the engine's
///   own mark-stored call all run only once the prune has already been applied to the resident
///   slot. One panic, one `warn!`. On the next pass that reaches the restored ref the closure
///   matches no tag, the store returns early before any write-through, and the ref settles as a
///   match against nothing. The durable tombstone then stays until the restart rebuild — the
///   same shape as a failed prune write.
/// - **Before or inside the engine mutation** — the prune closure, or the engine. That repeats
///   on every pass that reaches the key; the refs drained after it in a pass are re-indexed each
///   time, and the `warn!` repeats with it. A store write that panics is a bug to fix, not a
///   steady state, and there is deliberately no quarantine: isolating such a ref would need an
///   eighth exit and a change to the exit-ledger invariant.
///
/// # When the task ends
///
/// It ends only by being dropped — `JoinHandle::abort`, runtime teardown, or a panic that
/// escapes the catch. An exit lease owned by the future releases the single-flight claim, sets
/// the liveness gauge to `0` and warns on every one of those routes, so a later
/// `spawn_prune_task` over the same frontier succeeds instead of finding it claimed by nothing.
///
/// The catch is inert under `panic = "abort"`, which no profile in this workspace sets; a
/// structural test pins that, because under an aborting profile this supervision would be
/// silently absent rather than visibly broken.
#[must_use]
pub fn spawn_prune_task(
    frontier: Arc<TombstoneFrontier>,
    factory: Arc<RecordStoreFactory>,
    key_writer: Arc<KeyWriterRegistry>,
) -> Option<tokio::task::JoinHandle<()>> {
    if !frontier.claim_prune_task() {
        return None;
    }
    let lease = PruneTaskLease {
        frontier: Arc::clone(&frontier),
    };
    let wake = frontier.prune_wake();
    Some(tokio::spawn(async move {
        // A NAMED binding, so the lease lives as long as this future. A `_` pattern would drop
        // it here and release the claim while the loop below still runs.
        let _lease = lease;
        loop {
            wake.notified().await;
            // Read before the pass future exists, so the delta below covers the whole pass and
            // nothing outside it. Exact because a frontier admits one pass at a time and every
            // production re-index runs inside one.
            let restored_before = frontier.index_conservation_snapshot().restored_refs_total;
            let outcome = {
                // Pinned once per iteration and polled through a catch. Catching HERE rather
                // than running each pass on its own `tokio::spawn` is what preserves single
                // flight: dropping a `JoinHandle` does not abort the task behind it, so a
                // supervisor reading `JoinError` would detach a live pass over the index and
                // could then let a second one start beside it.
                let mut pass =
                    std::pin::pin!(prune_epoch_tombstones(&frontier, &factory, &key_writer));
                std::future::poll_fn(|cx| {
                    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        std::future::Future::poll(pass.as_mut(), cx)
                    })) {
                        Ok(std::task::Poll::Pending) => std::task::Poll::Pending,
                        Ok(std::task::Poll::Ready(())) => std::task::Poll::Ready(Ok(())),
                        // Ready, so this future is never polled again: the pass's state machine
                        // is poisoned by the unwind and polling it again is undefined.
                        Err(payload) => std::task::Poll::Ready(Err(payload)),
                    }
                })
                .await
            };
            if let Err(payload) = outcome {
                let restored = frontier
                    .index_conservation_snapshot()
                    .restored_refs_total
                    .saturating_sub(restored_before);
                // The attribution the counters cannot carry: the guard's own row calls every
                // unwind a cancellation, and this row is what tells the two apart.
                tracing::warn!(
                    restored,
                    panic_message = %panic_payload_message(payload.as_ref()),
                    "prune pass panicked; `restored` counts the refs re-indexed by this pass, \
                     and the prune task keeps running, waiting for the next trigger"
                );
            }
        }
    }))
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
    use std::collections::{HashMap, HashSet};
    use std::sync::{Arc, Weak};

    use metrics_exporter_prometheus::PrometheusBuilder;
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
    use crate::storage::map_data_store::{
        LeafSink, MapDataStore, ScanBatch, ScanCursor, WriteSource,
    };
    use crate::storage::mutation_observer::MutationObserver;
    use crate::storage::record_store::RecordStore;
    use crate::storage::tombstone_gauge::with_isolated_gauge;
    use crate::tombstone_frontier::{
        METRIC_PRUNE_ABSENT_TOTAL, METRIC_PRUNE_CONSIDERED_TOTAL, METRIC_PRUNE_DROPPED_TOTAL,
        METRIC_PRUNE_EMPTY_DRAINS_TOTAL, METRIC_PRUNE_EPOCHS_DRAINED_TOTAL,
        METRIC_PRUNE_MATCHED_NOTHING_TOTAL, METRIC_PRUNE_NONEMPTY_DRAINS_TOTAL,
        METRIC_PRUNE_PASSES_TOTAL, METRIC_PRUNE_RESTORED_CANCELLED_TOTAL,
        METRIC_PRUNE_RESTORED_EVICTED_TOTAL, METRIC_PRUNE_RESTORED_READ_ERROR_TOTAL,
        METRIC_PRUNE_RESTORED_WRITE_ERROR_TOTAL,
    };
    use crate::tombstone_frontier_impl::METRIC_PRUNE_TASK_ALIVE;

    // -----------------------------------------------------------------------
    // Normalized source search
    //
    // A doc-contract assertion that greps the raw file is decided by where a
    // line happens to wrap, not by what the contract says: the same phrase is
    // found or missed depending on indentation, a doc-comment break, a string
    // literal's `\` continuation, or a method call split across a line break.
    // Every one of those has already produced an assertion that was green
    // because it could not see its own subject. These four steps put the source
    // into one shape so a phrase assertion measures the contract instead.
    //
    // The steps are re-implemented here rather than shared: the equivalents in
    // `storage/wal/mod.rs` are private to that file's `mod tests`, there is no
    // test-support module in this package, and adding one is tracked separately
    // (TODO-620). That pointer is load-bearing, not provenance: the contract
    // this instrument is meant to satisfy is ONE normalizer in ONE home, this
    // copy is the third and does NOT satisfy it, and the divergence class the
    // normalizer exists to catch is exactly the class a silently-forked
    // normalizer would reintroduce. Naming the owner keeps the gap honest
    // rather than letting the duplication read as intentional.
    // -----------------------------------------------------------------------

    /// Step 1 — drop leading indentation and one `//`, `///` or `//!` marker, so
    /// a phrase reads the same whether it sits in prose or in a doc-comment.
    fn strip_comment_markers(src: &str) -> String {
        src.lines()
            .map(|line| {
                let trimmed = line.trim_start_matches([' ', '\t']);
                match trimmed.strip_prefix("//") {
                    Some(rest) => rest
                        .strip_prefix('/')
                        .or_else(|| rest.strip_prefix('!'))
                        .unwrap_or(rest)
                        .to_string(),
                    None => line.to_string(),
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Step 2 — drop Rust string-literal line continuations: a trailing `\`
    /// together with the following line's leading whitespace. Without this a
    /// phrase written inside a wrapped `reason = "…"` attribute is unfindable,
    /// because the raw text carries a backslash in the middle of the sentence.
    fn strip_line_continuations(src: &str) -> String {
        let chars: Vec<char> = src.chars().collect();
        let mut out = String::with_capacity(src.len());
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '\\' && chars.get(i + 1) == Some(&'\n') {
                i += 2;
                while chars.get(i).is_some_and(|c| c.is_whitespace()) {
                    i += 1;
                }
                out.push(' ');
                continue;
            }
            out.push(chars[i]);
            i += 1;
        }
        out
    }

    /// Step 3 — collapse every whitespace run, newlines included, to one space.
    fn collapse_whitespace(src: &str) -> String {
        src.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    /// Step 4 — delete every space immediately before a `.`, immediately after a
    /// `.`, and immediately before a `(`.
    ///
    /// This is what makes a call expression split across a line break match its
    /// single-line spelling: step 3 turns the break into one space, so
    /// `foo\n.bar()` would otherwise read as `foo .bar()` and a needle written
    /// the way the code reads on one line measures zero.
    ///
    /// The same rule tightens prose as collateral — a sentence boundary becomes
    /// `by design.This is`, and a prose parenthetical becomes `owners(the`. A
    /// needle spanning either MUST therefore be written in the tightened form.
    fn tighten_call_expressions(src: &str) -> String {
        let chars: Vec<char> = src.chars().collect();
        let mut out = String::with_capacity(src.len());
        let mut i = 0;
        while i < chars.len() {
            match chars[i] {
                ' ' if matches!(chars.get(i + 1), Some('.' | '(')) => {
                    i += 1;
                }
                '.' => {
                    out.push('.');
                    i += 1;
                    if chars.get(i) == Some(&' ') {
                        i += 1;
                    }
                }
                c => {
                    out.push(c);
                    i += 1;
                }
            }
        }
        out
    }

    /// The one normalized view every phrase assertion in this module runs
    /// through: markers, then `\` continuations, then whitespace, then call
    /// tightening. The order is load-bearing — step 2 must run before step 3, or
    /// the continuation's backslash survives with a space glued to it.
    fn normalized(src: &str) -> String {
        tighten_call_expressions(&collapse_whitespace(&strip_line_continuations(
            &strip_comment_markers(src),
        )))
    }

    /// Markers plus whitespace only — the weakest shape in the tree, kept here
    /// solely to demonstrate what it cannot see.
    fn normalized_marker_only(src: &str) -> String {
        collapse_whitespace(&strip_comment_markers(src))
    }

    /// Steps 1-3 — everything but call tightening, kept here solely to
    /// demonstrate that step 4 is doing real work.
    fn normalized_without_call_tightening(src: &str) -> String {
        collapse_whitespace(&strip_line_continuations(&strip_comment_markers(src)))
    }

    /// The slice from the first `start` anchor to the next `end` anchor after
    /// it, so a phrase can be required of ONE contract rather than of the whole
    /// file.
    ///
    /// Panics on a missing anchor, because a moved anchor must fail loudly
    /// rather than silently widen the scope. Panics below `min_len` too: an end
    /// anchor that drifts *earlier* truncates the region without going missing,
    /// which turns a presence assertion red and an absence assertion vacuously
    /// green, and a missing-anchor panic cannot see it.
    fn region<'a>(src: &'a str, start: &str, end: &str, min_len: usize) -> &'a str {
        let from = src
            .find(start)
            .unwrap_or_else(|| panic!("region start anchor not found: {start:?}"));
        let rest = &src[from..];
        let to = rest
            .find(end)
            .unwrap_or_else(|| panic!("region end anchor not found after start: {end:?}"));
        let slice = &rest[..to + end.len()];
        assert!(
            slice.len() >= min_len,
            "region {start:?}..{end:?} is {} bytes, below the {min_len} minimum -- the end anchor \
             drifted earlier and truncated the scope",
            slice.len()
        );
        slice
    }

    /// Non-overlapping occurrence count, so an assertion pins a number rather
    /// than a boolean.
    fn count(haystack: &str, needle: &str) -> usize {
        haystack.matches(needle).count()
    }

    /// Every checked-in `.rs` under this package, as `(path, source)`.
    ///
    /// Re-implemented here rather than shared: the equivalent walkers live in
    /// `wal_harness/cases.rs` as private fns, so they are unreachable from this
    /// file. A cascade that lands in a file the walk never visits would satisfy
    /// every assertion made over the result, which is why the callers below
    /// assert the walk reached named files before reading any zero out of it.
    fn package_rust_sources() -> Vec<(String, String)> {
        fn walk(dir: &std::path::Path, out: &mut Vec<(String, String)>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    // Build output is not checked in, and it contains generated
                    // sources that would pollute every count.
                    if path.file_name().is_some_and(|name| name == "target") {
                        continue;
                    }
                    walk(&path, out);
                } else if path.extension().is_some_and(|ext| ext == "rs") {
                    if let Ok(src) = std::fs::read_to_string(&path) {
                        out.push((path.display().to_string(), src));
                    }
                }
            }
        }
        let mut out = Vec::new();
        walk(std::path::Path::new(env!("CARGO_MANIFEST_DIR")), &mut out);
        out
    }

    /// Count a needle across the package, returning `(total, per-file hits)`.
    fn count_across_package(needle: &str) -> (usize, Vec<(String, usize)>) {
        let sources = package_rust_sources();
        assert!(
            sources
                .iter()
                .any(|(path, _)| path.ends_with("service/domain/crdt.rs")),
            "the walk must reach this file, or every count below is vacuous"
        );
        assert!(
            sources
                .iter()
                .any(|(path, _)| path.ends_with("storage/map_data_store.rs")),
            "the walk must reach the store trait, or every count below is vacuous"
        );
        let mut per_file = Vec::new();
        let mut total = 0;
        for (path, src) in &sources {
            let hits = count(&normalized(src), needle);
            if hits > 0 {
                total += hits;
                per_file.push((path.clone(), hits));
            }
        }
        (total, per_file)
    }

    /// The witness-aware methods are defaulted, and exactly ONE production store
    /// overrides them: the write-behind store, where an OR mutation becomes a
    /// delta WAL frame. Every other production store reaches the defaulted
    /// bodies unchanged.
    ///
    /// "Exactly one" is the bound that keeps the seam reviewable, so it is
    /// counted rather than argued: a second override would be a second place
    /// deciding how an OR write is framed on disk, and only a count stops one
    /// appearing unnoticed. The sanctioned overrides of the two store-side
    /// methods are that emitter and the test spy in this file; the demand
    /// predicate's one sanctioned override is the record store's delegation to
    /// its backend.
    ///
    /// Every needle is rebuilt from parts. That is load-bearing, not stylistic:
    /// this scan is hosted in the same file it counts over, so a needle written
    /// as one literal would find ITSELF — and the demand-predicate needle's
    /// allowed set excludes this file, so a self-match would fail the assertion
    /// on a correct implementation.
    #[test]
    fn the_witness_methods_reach_exactly_one_production_implementor() {
        let store_side = [
            concat!("fn ", "add_with_witness"),
            concat!("fn ", "wants_or_witness"),
        ];
        for needle in store_side {
            let (total, per_file) = count_across_package(needle);
            assert_eq!(
                total, 3,
                "{needle} must exist exactly three times -- the defaulted definition, the one \
                 PRODUCTION override, and the one test spy; anything else is an implementor \
                 cascade. Found: {per_file:?}"
            );
            assert!(
                per_file
                    .iter()
                    .any(|(path, _)| path.ends_with("storage/map_data_store.rs")),
                "{needle} must be found at its definition, or the scan matched nothing \
                 and its zero means nothing. Found: {per_file:?}"
            );
            assert!(
                per_file
                    .iter()
                    .any(|(path, _)| path.ends_with("storage/datastores/write_behind.rs")),
                "{needle}'s one sanctioned PRODUCTION override is the write-behind store, which \
                 consumes the witness by framing it. Found: {per_file:?}"
            );
            assert!(
                per_file
                    .iter()
                    .any(|(path, _)| path.ends_with("service/domain/crdt.rs")),
                "{needle}'s one sanctioned test override is the spy in this file. \
                 Found: {per_file:?}"
            );
        }

        // The demand predicate is asymmetric: its allowed set is the record
        // store's definition plus the ONE delegation, and it explicitly excludes
        // this file.
        let demand = concat!("fn ", "or_witness_wanted");
        let (total, per_file) = count_across_package(demand);
        assert_eq!(
            total, 2,
            "{demand} must exist exactly twice -- the defaulted definition and the one \
             delegation. Found: {per_file:?}"
        );
        assert!(
            per_file
                .iter()
                .any(|(path, _)| path.ends_with("storage/record_store.rs")),
            "{demand} must be found at its definition. Found: {per_file:?}"
        );
        assert!(
            per_file
                .iter()
                .any(|(path, _)| path.ends_with("impls/default_record_store.rs")),
            "{demand}'s one sanctioned delegation lives with the record store. \
             Found: {per_file:?}"
        );
        assert!(
            !per_file
                .iter()
                .any(|(path, _)| path.ends_with("service/domain/crdt.rs")),
            "{demand} must NOT appear in this file -- if it does, the needle matched \
             its own literal. Found: {per_file:?}"
        );
    }

    /// The OR doc-contracts say what the code now does, and none of them still
    /// asserts a claim this seam falsified.
    ///
    /// Three contracts went false when the witness route landed: the apply's
    /// instruction to serialize from a borrow ahead of the call, the outcome
    /// struct's instruction to serialize before handing the delta over together
    /// with the cost rationale that justified it, and the dead-code allowance on
    /// a flag the effect gate now reads. Each is asserted ABSENT, and each has a
    /// shorter sub-phrase asserted too, so the rewrite cannot be satisfied by
    /// deleting only the words the longer phrase adds and leaving the claim
    /// standing in shortened form.
    ///
    /// The prose here deliberately paraphrases rather than quotes those clauses.
    /// An earlier draft quoted one of them across a line break, and the
    /// normalized view joined it back into a match, so this test counted its own
    /// commentary and reported the contract un-rewritten.
    ///
    /// The clauses that must SURVIVE are pinned at the same time, because a
    /// rewrite that quietly dropped the purity clause or the placement rule would
    /// otherwise pass. Every needle is rebuilt from parts: these assertions are
    /// hosted in the file they count over, so a needle written as one literal
    /// would hold every required-0 row at one forever and read every required-1
    /// row as two.
    ///
    /// This assertion was RUN against the unmodified tree before the rewrite and
    /// observed red on all six absent rows; a version of it that had been written
    /// afterwards could not distinguish "the clause is gone" from "the needle
    /// never matched".
    #[test]
    fn or_doc_contracts_carry_no_falsified_clause() {
        const SOURCE: &str = include_str!("crdt.rs");
        let view = normalized(SOURCE);

        let required_absent = [
            ("P1", concat!("serializes from a borrow ", "before calling")),
            (
                "P2",
                concat!(
                    "the live OR_ADD path re-persists ",
                    "unconditionally and so reads no flag"
                ),
            ),
            (
                "P2s",
                concat!("re-persists unconditionally ", "and so reads no flag"),
            ),
            (
                "P5",
                concat!(
                    "already holds the delta and must serialize ",
                    "it BEFORE handing it over"
                ),
            ),
            (
                "P6",
                concat!("must serialize it ", "BEFORE handing it over"),
            ),
            (
                "P7",
                concat!(
                    "charge a value-sized clone ",
                    "to every accepted add on the hot path"
                ),
            ),
        ];

        let survivors: Vec<String> = required_absent
            .iter()
            .map(|(id, phrase)| (id, phrase, count(&view, phrase)))
            .filter(|(_, _, found)| *found != 0)
            .map(|(id, phrase, found)| format!("{id}: {found} x {phrase:?}"))
            .collect();
        assert!(
            survivors.is_empty(),
            "falsified doc-contract clauses still present ({} of {}):\n  {}",
            survivors.len(),
            required_absent.len(),
            survivors.join("\n  ")
        );

        // Clauses that must survive the rewrite — the first four unchanged from
        // before it, the last two the rewrite's own statement of the pinned route.
        let required_present = [
            (
                "P3",
                concat!("PURE: no gauge counters, ", "no tombstone frontier, no I/O"),
            ),
            (
                "P4",
                concat!(
                    "in particular the prune decrement ",
                    "must stay outside this function"
                ),
            ),
            (
                "P8",
                concat!("Deliberately carries ", "no copy of the delta"),
            ),
            (
                "P9",
                concat!(
                    "The flags below are the only thing ",
                    "a caller cannot derive for itself"
                ),
            ),
            (
                "P10",
                concat!("the post-image entry recovered ", "from the slot by tag"),
            ),
            (
                "P11",
                concat!("reads its own copy back out ", "of the post-image slot"),
            ),
        ];

        for (id, phrase) in required_present {
            assert_eq!(
                count(&view, phrase),
                1,
                "{id} must be present exactly once: {phrase:?}"
            );
        }
    }

    /// The unarmed OR_ADD path still MOVES its entry into the apply, and clones
    /// nothing.
    ///
    /// This is the cost claim of the whole seam, asserted structurally rather than
    /// trusted: the entry reaches the apply by move exactly as it did before, and
    /// no clone of it appears outside the demand-gated branch. Region-scoped
    /// because the absence needle is NOT zero file-wide — two unrelated sites
    /// clone an `entry` — so an unscoped assertion would be red at HEAD on code
    /// this seam never touched.
    ///
    /// The `min_len` floor guards the failure a missing-anchor panic cannot see:
    /// an end anchor that drifts EARLIER truncates the region silently, which
    /// would turn the presence needles red and the absence needle vacuously green.
    #[test]
    fn the_unarmed_add_path_moves_its_entry_and_clones_nothing() {
        const SOURCE: &str = include_str!("crdt.rs");
        let view = normalized(SOURCE);
        let scope = region(
            &view,
            concat!("let mut ", "merge_add"),
            concat!(".update_in_", "place("),
            400,
        );

        let moved = concat!("new_entry_opt", ".take()");
        let handed_to_apply = concat!("apply_or_delta(OrDelta::Add ", "{ entry }, value)");
        let cloned = concat!("entry", ".clone()");

        assert_eq!(
            count(scope, moved),
            1,
            "the entry must still leave its slot by move, not by copy"
        );
        assert_eq!(
            count(scope, handed_to_apply),
            1,
            "the entry must still reach the apply by move"
        );
        assert_eq!(
            count(scope, cloned),
            0,
            "nothing may clone the entry outside the demand-gated branch"
        );

        // The region is doing real work: the absence needle is non-zero file-wide,
        // so this would be red at HEAD without the scoping.
        assert_eq!(
            count(&view, cloned),
            2,
            "file-wide clones of an entry are unrelated sites; if this moved, \
             re-check whether the region still isolates the OR_ADD closure"
        );
    }

    /// The in-place write-through reads no pre-image, so no diff can be computed
    /// there.
    ///
    /// A witness is never derived by comparing before and after: doing so would
    /// re-introduce the read-modify-write copy the in-place seam exists to remove,
    /// and would be a second implementation of the OR algebra. The absence of a
    /// read on this path is what makes that structurally impossible.
    #[test]
    fn the_in_place_write_through_reads_no_pre_image() {
        const SOURCE: &str = include_str!("../../storage/impls/default_record_store.rs");
        let view = normalized(SOURCE);
        let scope = region(
            &view,
            concat!("async fn update_in_", "place("),
            concat!("async fn ", "remove("),
            1000,
        );

        // Non-vacuity: the write-through itself must be inside the region, or the
        // zero below is measured over the wrong slice.
        assert!(
            scope.contains(concat!("CallerProvenance", "::Client")),
            "the region must contain the write-through's provenance check"
        );
        assert_eq!(
            count(scope, concat!(".get", "(")),
            0,
            "a pre-image read on this path is what a diff would need; there is none"
        );
    }

    /// There is still exactly ONE implementation of the OR algebra.
    ///
    /// Threading a witness must not spawn a second copy of add-wins/remove-wins/
    /// prune — a recovery fold and the write path drifting apart is the failure
    /// this counts against (TG-OR-003). The long form of the needle is used
    /// deliberately: a shorter one also matches an unrelated harness symbol whose
    /// name merely contains it.
    #[test]
    fn the_or_algebra_has_exactly_one_implementation() {
        let needle = concat!("pub(crate) fn ", "apply_or_delta(");
        let (total, per_file) = count_across_package(needle);
        assert_eq!(
            total, 2,
            "expected the definition plus the one pre-existing source-scanning \
             literal, both in the CRDT service; a third means a second algebra. \
             Found: {per_file:?}"
        );
        assert!(
            per_file.len() == 1 && per_file[0].0.ends_with("service/domain/crdt.rs"),
            "both occurrences must live in the CRDT service. Found: {per_file:?}"
        );

        // And the store-side files gained only types and defaulted methods: no
        // diff or merge helper slipped in alongside them.
        let sources = package_rust_sources();
        for suffix in [
            "storage/record_store.rs",
            "storage/impls/default_record_store.rs",
            "storage/map_data_store.rs",
        ] {
            let (path, src) = sources
                .iter()
                .find(|(path, _)| path.ends_with(suffix))
                .unwrap_or_else(|| panic!("{suffix} must be reachable, or its zero is vacuous"));
            let view = normalized(src);
            for helper in [concat!("fn ", "diff"), concat!("fn ", "merge")] {
                assert_eq!(
                    count(&view, helper),
                    0,
                    "{path} must gain no {helper} helper -- the algebra stays in one place"
                );
            }
        }
    }

    /// The share of OR writes that carry NO witness, measured deterministically.
    ///
    /// This seam does not change which writes happen: an op that took no effect
    /// still re-persists its whole slot, exactly as before. Those writes are the
    /// residual a delta-framing consumer cannot shrink, so the plateau argument
    /// downstream needs the share as a number rather than as a hope. Eliminating
    /// the residual — gating the unconditional re-persist on the same flags — is
    /// tracked separately and deliberately not attempted here.
    ///
    /// Sequential and seeded, not concurrent: whether a given add is suppressed
    /// depends on interleaving, so a concurrent driver would report a different
    /// ratio per run and could not be consumed as a fixed term. The same seed must
    /// therefore reproduce the same counts, which the caller below asserts by
    /// running it twice.
    async fn measure_none_witness_share(seed: u64) -> (usize, usize) {
        const KEYS: u64 = 48;
        const EPOCH_WIDTH: u64 = 100;
        const ROUNDS: u64 = 480;

        let spy = Arc::new(WitnessSpyStore::standalone(true));
        let (svc, factory, frontier) = make_service_with_frontier_and_store(
            Arc::clone(&spy) as Arc<dyn MapDataStore>,
            Vec::new(),
        );
        frontier.set_epoch_width(EPOCH_WIDTH);

        // A pinned linear congruential generator rather than a random source, so
        // the op sequence is a function of the seed alone.
        let mut state = seed;
        let mut next = move || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            state >> 33
        };

        for round in 0..ROUNDS {
            let key = format!("ork-{}", next() % KEYS);
            let tag = format!("t-{round}");

            // The churn pair the soak drives: a unique tag added then removed.
            Arc::clone(&svc)
                .oneshot(or_add_op("m", &key, "v", &tag))
                .await
                .unwrap();
            Arc::clone(&svc)
                .oneshot(or_remove_op("m", &key, &tag))
                .await
                .unwrap();

            // Every fifth round also replays an op that cannot take effect: a
            // re-add of the tag just tombstoned, and a re-issue of the same
            // remove. These are the residual's two add/remove sources.
            if round % 5 == 0 {
                Arc::clone(&svc)
                    .oneshot(or_add_op("m", &key, "v", &tag))
                    .await
                    .unwrap();
                Arc::clone(&svc)
                    .oneshot(or_remove_op("m", &key, &tag))
                    .await
                    .unwrap();
            }
        }

        // Open both prune conjuncts and sweep, so effective prunes are in the mix
        // too rather than being excluded by an unreachable gate.
        let client: String = "a5:alice|dev-1".into();
        frontier.set_delivered(ConnectionId(1), 10_000);
        frontier
            .confirm_apply_ack(&client, frontier.low_water_mark() + 50, ConnectionId(1))
            .await;
        frontier.set_durable_epoch_watermark(100_000);
        prune_epoch_tombstones(&frontier, &factory, &svc.key_writer).await;

        let observed = spy.observations();
        let total = observed.len();
        let none = observed.iter().filter(|o| o.witness.is_none()).count();
        (none, total)
    }

    #[tokio::test]
    async fn the_none_witness_share_is_measured_and_reproducible() {
        const SEED: u64 = 0x5EED_349C;

        let (none_a, total_a) = measure_none_witness_share(SEED).await;
        let (none_b, total_b) = measure_none_witness_share(SEED).await;

        assert!(
            total_a > 0,
            "the driver must have produced OR writes to measure"
        );
        assert_eq!(
            (none_a, total_a),
            (none_b, total_b),
            "the same seed must reproduce the same counts, or the number is not a \
             term anything downstream can consume"
        );
        // Recorded rather than bounded: this seam does not change the residual, so
        // a threshold here would be an invented contract. The number belongs in the
        // completion record.
        // Tenths of a percent in integer arithmetic: a float cast of a usize is
        // lossy on 64-bit targets, and the ratio needs no float to be exact.
        let tenths = none_a * 1000 / total_a;
        println!(
            "none-witness OR writes: {none_a}/{total_a} ({}.{}%) at seed {SEED:#x}",
            tenths / 10,
            tenths % 10
        );
    }

    /// None of the files this change touches may name the delta frame variant.
    ///
    /// A package-wide belt already scans raw file contents for that literal and
    /// admits it only in the WAL's own modules, so a single occurrence here --
    /// in code, a string literal or an assertion message -- turns that belt red,
    /// and every repair available for it is forbidden. Re-asserted locally so
    /// the trap surfaces from this file's own test run rather than from the
    /// harness. Frame-kind assertions are therefore written positively, which
    /// entails the negative: the only other inhabitants are the store frame, the
    /// remove frame, and a test-only variant.
    #[test]
    fn no_file_this_change_touches_names_the_delta_frame_variant() {
        let needle = concat!("WalOp", "::", "OrDelta");
        let touched = [
            "service/domain/crdt.rs",
            "storage/record_store.rs",
            "storage/impls/default_record_store.rs",
            "storage/map_data_store.rs",
            "storage/or_inplace_mutate_proptest.rs",
        ];
        let sources = package_rust_sources();
        for suffix in touched {
            let (path, src) = sources
                .iter()
                .find(|(path, _)| path.ends_with(suffix))
                .unwrap_or_else(|| panic!("{suffix} must be reachable, or its zero is vacuous"));
            assert_eq!(
                src.matches(needle).count(),
                0,
                "{path} names the delta frame variant; the package-wide belt scans raw \
                 contents and admits it only in the WAL's own modules"
            );
        }
    }

    /// Power (a): a phrase that spans a doc-comment line break is found by the
    /// normalized view and is NOT found by a plain `contains` on the raw source.
    ///
    /// The needle is reconstructed from parts so this file never contains its
    /// own literal needle — otherwise the scan counts itself and the number
    /// stops measuring the contract.
    #[test]
    fn normalized_view_sees_through_doc_comment_wrapping() {
        const SOURCE: &str = include_str!("crdt.rs");
        let needle = concat!(
            "in particular the prune decrement ",
            "must stay outside this function"
        );

        assert!(
            !SOURCE.contains(needle),
            "the driver must span a doc-comment line break, or this power proves nothing"
        );
        assert_eq!(
            count(&normalized(SOURCE), needle),
            1,
            "the normalized view must find a phrase the raw source cannot"
        );
    }

    /// Power (b): a phrase that spans a string-literal `\` continuation is found
    /// by the normalized view and is NOT found by the markers-plus-whitespace
    /// shape.
    ///
    /// Mutation proof: drop `strip_line_continuations` from `normalized` and the
    /// helper collapses onto the weak shape, which measures zero here.
    #[test]
    fn normalized_view_sees_through_string_literal_continuations() {
        const SOURCE: &str = include_str!("crdt.rs");
        let needle = concat!("the differential recovery test ", "is the first consumer");

        assert!(
            !SOURCE.contains(needle),
            "the driver must span a `\\` continuation, or this power proves nothing"
        );
        assert_eq!(
            count(&normalized_marker_only(SOURCE), needle),
            0,
            "markers plus whitespace cannot see through a `\\` continuation"
        );
        assert_eq!(
            count(&normalized(SOURCE), needle),
            1,
            "the continuation strip is what makes this phrase measurable"
        );
    }

    /// Power (c), part 1: a region is strictly narrower than the whole source,
    /// so a phrase can be required of one closure rather than of 5000 lines.
    #[test]
    fn region_narrows_the_scanned_scope() {
        const SOURCE: &str = include_str!("crdt.rs");
        let view = normalized(SOURCE);
        let scoped = region(
            &view,
            concat!("let mut ", "merge_add"),
            concat!(".update_in_", "place("),
            400,
        );

        assert!(
            scoped.len() < view.len(),
            "a region that is not strictly shorter than the file is not scoping anything"
        );
    }

    /// Power (c), part 2: a start anchor that has moved fails loudly instead of
    /// silently widening the scope.
    #[test]
    #[should_panic(expected = "region start anchor not found")]
    fn region_panics_on_a_missing_start_anchor() {
        const SOURCE: &str = include_str!("crdt.rs");
        region(
            &normalized(SOURCE),
            concat!("an anchor that ", "no source file carries"),
            concat!(".update_in_", "place("),
            1,
        );
    }

    /// Power (c), part 3: the same for an end anchor.
    #[test]
    #[should_panic(expected = "region end anchor not found")]
    fn region_panics_on_a_missing_end_anchor() {
        const SOURCE: &str = include_str!("crdt.rs");
        region(
            &normalized(SOURCE),
            concat!("let mut ", "merge_add"),
            concat!("an anchor that ", "no source file carries"),
            1,
        );
    }

    /// Power (c), part 4: both anchors present but the slice is too short --
    /// the failure mode a missing-anchor panic cannot detect.
    ///
    /// Mutation proof: weaken or delete the `min_len` assertion and this stops
    /// panicking.
    #[test]
    #[should_panic(expected = "below the")]
    fn region_panics_when_the_slice_is_shorter_than_min_len() {
        const SOURCE: &str = include_str!("crdt.rs");
        region(
            &normalized(SOURCE),
            concat!("let mut ", "merge_add"),
            concat!(".update_in_", "place("),
            100_000,
        );
    }

    /// Power (d): a call expression split across a line break is found in its
    /// single-line spelling, and the one-space spelling is not.
    ///
    /// This is the regression case that motivated step 4 -- an assertion
    /// requiring the entry to reach the apply by move measured zero against a
    /// tree that did exactly that, because the call spans two lines.
    ///
    /// Mutation proof: drop `tighten_call_expressions` from `normalized` and the
    /// last two assertions swap, reddening this test.
    #[test]
    fn normalized_view_sees_a_call_split_across_a_line_break() {
        const SOURCE: &str = include_str!("crdt.rs");
        let single_line = concat!("new_entry_opt", ".take()");
        let one_space = concat!("new_entry_opt ", ".take()");

        assert_eq!(count(SOURCE, single_line), 0, "raw source, tight spelling");
        assert_eq!(count(SOURCE, one_space), 0, "raw source, spaced spelling");

        let without_step_4 = normalized_without_call_tightening(SOURCE);
        assert_eq!(
            count(&without_step_4, single_line),
            0,
            "steps 1-3 leave the line break as a space, so the tight spelling is unfindable"
        );
        assert_eq!(
            count(&without_step_4, one_space),
            1,
            "steps 1-3 yield the spaced spelling nobody would write by hand"
        );

        let full = normalized(SOURCE);
        assert_eq!(
            count(&full, single_line),
            1,
            "step 4 is what makes the call match the way it reads on one line"
        );
        assert_eq!(
            count(&full, one_space),
            0,
            "step 4 must leave no spaced spelling behind"
        );
    }

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

    // -- OpBatch with empty ops produces no frame --

    #[tokio::test]
    async fn op_batch_empty_answers_with_no_frame() {
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
        // A batch with no operations has nothing to acknowledge, and the only id
        // available here is the server's own call id, which is not an op id.
        assert!(
            matches!(resp, OperationResponse::Empty),
            "an empty op batch must be answered with no frame, got {resp:?}"
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
        make_service_with_frontier_and_store(Arc::new(NullDataStore), Vec::new())
    }

    /// Same wiring as [`make_service_with_frontier`], with the backing datastore
    /// and the store observers supplied by the caller.
    ///
    /// A separate constructor rather than a changed signature on the existing
    /// fixtures: the `NullDataStore`-backed tests must keep running against
    /// byte-identical wiring, so a test that needs a real (failable, retaining)
    /// backend cannot perturb them.
    fn make_service_with_frontier_and_store(
        data_store: Arc<dyn MapDataStore>,
        observers: Vec<Arc<dyn MutationObserver>>,
    ) -> (
        Arc<CrdtService>,
        Arc<RecordStoreFactory>,
        Arc<TombstoneFrontier>,
    ) {
        let factory = Arc::new(RecordStoreFactory::new(
            StorageConfig::default(),
            data_store,
            observers,
        ));
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

    /// AC4 (OR write path): the `OR_REMOVE` write path is wired to the wholesale
    /// epoch-drop prune as a TRIGGER — the op wakes the prune task and drains
    /// nothing on its own timeline. With an injected durability watermark and the
    /// low-water-mark STRICTLY past the epoch, the pass the trigger asks for then
    /// drops that epoch's tombstone from storage; a not-strictly-past epoch
    /// survives. DARK by default — the injected watermark exercises the real drop
    /// path.
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

        // Trigger via an OR_REMOVE on a THIRD key. Its own new tombstone lands in
        // epoch 3 (pinned); epoch 2 is pinned too (LWM 2 not strictly past 2);
        // epoch 1's T1 is what the pass is licensed to drop.
        Arc::clone(&svc)
            .oneshot(or_add_op("m", "k3", "v3", "T3"))
            .await
            .unwrap();
        Arc::clone(&svc)
            .oneshot(or_remove_op("m", "k3", "T3"))
            .await
            .unwrap();

        // The op triggered, it did not drain: T1 is still stored on the op's own
        // timeline, and a wake permit is waiting for the task.
        let (_, untouched) = read_or_map(&factory, "m", "k1").await;
        assert!(
            untouched.contains(&"T1".to_string()),
            "the OR_REMOVE wakes the prune task and drains nothing itself"
        );
        let wake = frontier.prune_wake();
        tokio::time::timeout(std::time::Duration::from_millis(1), wake.notified())
            .await
            .expect("the OR_REMOVE must leave a wake permit pending for the prune task");

        // The pass the trigger asked for, run explicitly: this fixture spawns no
        // task, so nothing else would ever consume the permit.
        prune_epoch_tombstones(&frontier, &factory, &svc.key_writer).await;

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

    /// The tombstone-bytes gauge is decremented by the REAL epoch-prune path.
    ///
    /// Reuses `ac4_prune_wired_into_or_write_path`'s eligibility recipe, but
    /// asserts on the gauge rather than on stored contents: the production
    /// `sub_tombstone_bytes` in the prune drain is otherwise reachable only
    /// through a test-local mirror, which cannot detect its removal. Running
    /// inside a private sink makes the net delta exact — three OR_REMOVE adds
    /// minus the one tag the sweep drops — so a missing decrement cannot be
    /// absorbed by ambient traffic.
    #[tokio::test]
    async fn or_prune_decrements_gauge_on_real_prune_path() {
        let (svc, factory, frontier) = make_service_with_frontier();
        let (t1, t2, t3) = ("T1", "T2", "T3");

        let ((), net_delta) = crate::storage::tombstone_gauge::with_isolated_gauge(async {
            // Add + remove T1 on k1 -> epoch 1; T2 on k2 -> epoch 2. Both stored.
            for (key, val, tag) in [("k1", "v1", t1), ("k2", "v2", t2)] {
                Arc::clone(&svc)
                    .oneshot(or_add_op("m", key, val, tag))
                    .await
                    .unwrap();
                Arc::clone(&svc)
                    .oneshot(or_remove_op("m", key, tag))
                    .await
                    .unwrap();
            }
            assert_eq!(frontier.current_epoch(), 2, "epochs 1..=2 stamped");

            // Raise the LWM strictly past epoch 1 (cursor 2 > 1) and open the
            // durability watermark. Both are injected, so exactly which epoch is
            // eligible is deterministic — no wall clock, no background sweeper.
            let c: String = "a5:alice|dev-1".into();
            frontier.set_delivered(ConnectionId(1), 100);
            assert!(frontier.confirm_apply_ack(&c, 2, ConnectionId(1)).await);
            assert_eq!(frontier.low_water_mark(), 2);
            frontier.set_durable_epoch_watermark(1000);

            // Trigger via an OR_REMOVE on a THIRD key: epoch 3 (its own) and
            // epoch 2 stay pinned, epoch 1's T1 is what the pass may drop.
            Arc::clone(&svc)
                .oneshot(or_add_op("m", "k3", "v3", t3))
                .await
                .unwrap();
            Arc::clone(&svc)
                .oneshot(or_remove_op("m", "k3", t3))
                .await
                .unwrap();

            // The pass the trigger asked for, run explicitly inside the isolated
            // gauge scope so its decrement lands in the measured delta. This is
            // the same function the spawned task runs, which is what keeps this a
            // `TG-OR-004` enforcing test of the real prune path.
            prune_epoch_tombstones(&frontier, &factory, &svc.key_writer).await;

            // Pin the drop the decrement accounts for. Without this, a gauge
            // delta alone could also be produced by a prune that never ran plus
            // miscounted adds; pairing the two makes a failure attributable.
            let (_, tombs_k1) = read_or_map(&factory, "m", "k1").await;
            assert!(
                !tombs_k1.contains(&t1.to_string()),
                "epoch-1 tombstone pruned from storage via the OR write path"
            );
            let (_, tombs_k2) = read_or_map(&factory, "m", "k2").await;
            assert!(
                tombs_k2.contains(&t2.to_string()),
                "epoch-2 tombstone still pinned (LWM 2 not strictly past epoch 2)"
            );
        })
        .await;

        // Derived from the tag lengths rather than hard-coded, so renaming a tag
        // cannot silently invalidate the expectation.
        let expected = (t1.len() + t2.len() + t3.len() - t1.len()) as u64;
        assert_eq!(
            net_delta, expected,
            "three OR_REMOVE charges minus the single epoch-1 tag the prune drops"
        );
    }

    // -----------------------------------------------------------------------
    // The residency ledger's independent per-epoch byte oracle — the
    // service-composition arm: a real `OR_REMOVE` through the full write path
    // threads `tag.len()` into the frontier's own conservation counters,
    // independently of the pre-existing tombstone-bytes gauge at `:696`.
    // -----------------------------------------------------------------------

    /// A real `OR_REMOVE`, applied through the full service composition, feeds
    /// `stamped_bytes_total` with EXACTLY `tag.len()` — computed independently of
    /// `add_tombstone_bytes(tag.len() as u64)` (the pre-existing, unrelated
    /// tombstone-bytes gauge this contract must not touch, R2.4(b)). O-0's
    /// conservation identity holds over the resulting snapshot.
    #[tokio::test]
    async fn or_remove_threads_the_independent_byte_oracle_through_the_service_composition() {
        let (svc, _factory, frontier) = make_service_with_frontier();
        let (t1, t2) = ("TAG-ONE", "TAG-TWO-LONGER");

        for (key, val, tag) in [("k1", "v1", t1), ("k2", "v2", t2)] {
            Arc::clone(&svc)
                .oneshot(or_add_op("m", key, val, tag))
                .await
                .unwrap();
            Arc::clone(&svc)
                .oneshot(or_remove_op("m", key, tag))
                .await
                .unwrap();
        }

        let snapshot = frontier.index_conservation_snapshot();
        let expected_bytes = (t1.len() + t2.len()) as u64;
        assert_eq!(
            snapshot.stamped_bytes_total, expected_bytes,
            "the per-epoch byte oracle must equal the tag lengths exactly, independent \
             of the tombstone-bytes gauge's own tag.len() computation at the OR_REMOVE site"
        );
        assert_eq!(
            snapshot.stamped_refs_total, 2,
            "one stamped ref per OR_REMOVE"
        );
        // O-0's identity: nothing has drained or rebuilt yet, so the whole stamped
        // total is still resident.
        assert_eq!(
            snapshot.stamped_refs_total + snapshot.restored_refs_total
                - snapshot.drained_refs_total
                - snapshot.rebuild_cleared_refs_total,
            snapshot.indexed_refs,
            "O-0 must hold over the service-composition path too, got {snapshot:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Tier-1 deterministic discrimination -- the service-composition arm
    // (R7.1/R7.1a): `prune_epoch_tombstones`, driven through the FULL
    // `CrdtService` + `RecordStoreFactory` + `KeyWriterRegistry` write path,
    // reaches the same frozen-walk class the `FrontierState` arm reaches
    // synthetically in `tombstone_frontier_impl.rs` -- class (d)
    // DRAINED-HEALTHY -- but here every conjunct (`entered_index`, licensing,
    // fencing, the actual storage drop) is produced by real writes through the
    // real service, not constructed by hand at the frontier boundary.
    // -----------------------------------------------------------------------

    /// A real `OR_ADD` + `OR_REMOVE` through the full service composition,
    /// followed by the real `prune_epoch_tombstones` entry point (not
    /// `FrontierState::drain_prunable` called directly): the epoch is
    /// licensed and fenced, THEN actually taken by the real prune sweep, and
    /// the tag is durably gone from storage afterward -- exactly class (d)
    /// DRAINED-HEALTHY's definition (R3.3), reached here through the real
    /// write path the `FrontierState` arm's dry-run leg cannot exercise.
    #[tokio::test]
    async fn service_composition_arm_reaches_drained_healthy_through_the_real_write_path() {
        let (svc, factory, frontier) = make_service_with_frontier();

        Arc::clone(&svc)
            .oneshot(or_add_op("m", "k1", "v1", "TAGSC1"))
            .await
            .unwrap();
        Arc::clone(&svc)
            .oneshot(or_remove_op("m", "k1", "TAGSC1"))
            .await
            .unwrap();
        // A second stamp rolls the clock past epoch 1 and fires its entry row
        // (R2.3a) -- the same rollover mechanism the `FrontierState` arm
        // exercises, reached here through the real write path instead.
        Arc::clone(&svc)
            .oneshot(or_add_op("m", "k2", "v2", "TAGSC2"))
            .await
            .unwrap();
        Arc::clone(&svc)
            .oneshot(or_remove_op("m", "k2", "TAGSC2"))
            .await
            .unwrap();

        open_prune_gates_past_epoch_one(&frontier).await;
        // One more stamp advances op_seq strictly past the licensing instant
        // (the residency interval is half-open, `[entered_at_op_seq,
        // exited_at_op_seq)`, R3.2), so the triple-overlap window is a
        // genuinely non-empty interval rather than a zero-width boundary case.
        Arc::clone(&svc)
            .oneshot(or_add_op("m", "k3", "v3", "TAGSC3"))
            .await
            .unwrap();

        prune_epoch_tombstones(&frontier, &factory, &svc.key_writer).await;

        let snapshot = frontier.index_conservation_snapshot();
        assert_eq!(
            snapshot.drained_refs_total, 1,
            "epoch 1 (TAGSC1) is the only licensed-and-fenced epoch"
        );
        assert_eq!(
            snapshot.stamped_refs_total + snapshot.restored_refs_total
                - snapshot.drained_refs_total
                - snapshot.rebuild_cleared_refs_total,
            snapshot.indexed_refs,
            "O-0 must hold over the service-composition drain path too, got {snapshot:?}"
        );

        // The DrainedByPrune disposition is not just a counter: the tag is
        // durably gone from storage, which is what distinguishes a real
        // class-(d) drain from a synthetic one asserted only at the frontier.
        let (_, tombs) = read_or_map(&factory, "m", "k1").await;
        assert!(
            !tombs.contains(&"TAGSC1".to_string()),
            "the real write path must have durably dropped the drained tag"
        );
    }

    // -----------------------------------------------------------------------
    // Where the tombstone-byte counters fire, and what the prune does with the
    // two dispositions a bare `Ok(false)` conflates
    // -----------------------------------------------------------------------

    /// A retaining `MapDataStore` whose writes can be ARMED to fail for named
    /// keys AFTER a test has already seeded through it.
    ///
    /// Arm-ability — not retention — is what makes the counter-position tests
    /// reachable at all. An `OR_REMOVE` propagates its write error before the
    /// tombstone is epoch-stamped, so a store that failed from the start would
    /// leave the frontier index empty and the prune sweep with nothing to drain:
    /// there would be no tombstone in existence to assert anything about. Seeding
    /// un-armed lands the resident record, its durable counterpart and the
    /// frontier ref; arming then fails exactly the write under test.
    ///
    /// Retention is defensive rather than load-bearing: it lets a test compare
    /// what stayed DURABLE against what the byte gauge claims, which is the whole
    /// reason the prune decrement sits behind a successful write (TG-OR-004).
    #[derive(Default)]
    struct ArmableStore {
        data: Mutex<HashMap<(String, String), RecordValue>>,
        reject_keys: Mutex<HashSet<String>>,
        reject_read_keys: Mutex<HashSet<String>>,
        /// Keys whose durable row `load` serves ONCE and then answers absent for,
        /// with the value still present in the backing map. `true` once served.
        absent_after_one_load: Mutex<HashMap<String, bool>>,
        /// Every `load` / `load_all` call, so a test can assert a path never
        /// reached the backend.
        loads: std::sync::atomic::AtomicUsize,
        /// Evicts an armed key during its next `load` (see [`EvictOnRehydrate`]).
        evictor: EvictOnRehydrate,
    }

    impl ArmableStore {
        /// Serve `key`'s durable row to the next `load` only, and answer absent
        /// to every later one while the row stays in the backing map: a store
        /// whose rows the record store's in-place write cannot materialize.
        fn answer_absent_after_one_load(&self, key: &str) {
            self.absent_after_one_load
                .lock()
                .insert(key.to_string(), false);
        }

        /// Fail every subsequent write to `key`. Reads keep working, so a test can
        /// still inspect what survived durably.
        fn reject_writes_to(&self, key: &str) {
            self.reject_keys.lock().insert(key.to_string());
        }

        /// Fail every subsequent rehydrating LOAD of `key`.
        ///
        /// Separate from the write arming because the prune's read failure is a
        /// different exit from its write failure, and the two must be drivable
        /// independently — arming one key for both would collapse two counters
        /// into one observation. [`ArmableStore::durable`] reads the backing map
        /// directly rather than through `load`, so a test can still inspect what
        /// survived on a read-armed key.
        fn reject_reads_to(&self, key: &str) {
            self.reject_read_keys.lock().insert(key.to_string());
        }

        fn durable(&self, map: &str, key: &str) -> Option<RecordValue> {
            self.data
                .lock()
                .get(&(map.to_string(), key.to_string()))
                .cloned()
        }

        /// Place a durable record no resident slot mirrors, so the next read of
        /// that key goes through the rehydration path.
        fn seed_durable(&self, map: &str, key: &str, value: RecordValue) {
            self.data
                .lock()
                .insert((map.to_string(), key.to_string()), value);
        }
    }

    #[async_trait]
    impl MapDataStore for ArmableStore {
        async fn add(
            &self,
            map: &str,
            key: &str,
            value: &RecordValue,
            _exp: i64,
            _now: i64,
        ) -> anyhow::Result<()> {
            if self.reject_keys.lock().contains(key) {
                return Err(anyhow::anyhow!("armed write rejection for {key}"));
            }
            self.data
                .lock()
                .insert((map.to_string(), key.to_string()), value.clone());
            Ok(())
        }

        async fn add_backup(
            &self,
            _: &str,
            _: &str,
            _: &RecordValue,
            _: i64,
            _: i64,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn remove(&self, map: &str, key: &str, _now: i64) -> anyhow::Result<()> {
            // An armed key rejects every durable write, not just `add` — otherwise a
            // path that deletes instead of writing would silently succeed against a
            // store the test believes is failing, and the assertion would pass for
            // the wrong reason.
            if self.reject_keys.lock().contains(key) {
                return Err(anyhow::anyhow!("armed write rejection for {key}"));
            }
            self.data.lock().remove(&(map.to_string(), key.to_string()));
            Ok(())
        }

        async fn remove_backup(&self, _: &str, _: &str, _: i64) -> anyhow::Result<()> {
            Ok(())
        }

        async fn load(&self, map: &str, key: &str) -> anyhow::Result<Option<RecordValue>> {
            self.loads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if self.reject_read_keys.lock().contains(key) {
                return Err(anyhow::anyhow!("armed read rejection for {key}"));
            }
            self.evictor.on_load(key);
            if let Some(served) = self.absent_after_one_load.lock().get_mut(key) {
                if *served {
                    return Ok(None);
                }
                *served = true;
            }
            Ok(self
                .data
                .lock()
                .get(&(map.to_string(), key.to_string()))
                .cloned())
        }

        async fn load_all(
            &self,
            map: &str,
            keys: &[String],
        ) -> anyhow::Result<Vec<(String, RecordValue)>> {
            self.loads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            // A read-armed key fails on every read path, not just the single-key
            // one — otherwise a batched rehydration would silently succeed
            // against a store the test believes is failing.
            {
                let rejected = self.reject_read_keys.lock();
                if let Some(key) = keys.iter().find(|key| rejected.contains(*key)) {
                    return Err(anyhow::anyhow!("armed read rejection for {key}"));
                }
            }
            let guard = self.data.lock();
            Ok(keys
                .iter()
                .filter_map(|key| {
                    guard
                        .get(&(map.to_string(), key.clone()))
                        .map(|value| (key.clone(), value.clone()))
                })
                .collect())
        }

        async fn remove_all(&self, map: &str, keys: &[String]) -> anyhow::Result<()> {
            let mut guard = self.data.lock();
            for key in keys {
                guard.remove(&(map.to_string(), key.clone()));
            }
            Ok(())
        }

        async fn enumerate_leaves(
            &self,
            _: &str,
            _: bool,
            _: &mut dyn LeafSink,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn scan_values(&self, _: &str, _: bool, _: u64) -> anyhow::Result<ScanBatch> {
            Ok(ScanBatch::default())
        }

        async fn scan_values_batched(
            &self,
            _: &str,
            _: bool,
            _: ScanCursor,
            _: u64,
        ) -> anyhow::Result<ScanBatch> {
            Ok(ScanBatch::default())
        }

        fn is_loadable(&self, _: &str) -> bool {
            true
        }

        fn pending_operation_count(&self) -> u64 {
            0
        }

        async fn soft_flush(&self) -> anyhow::Result<u64> {
            Ok(0)
        }

        async fn hard_flush(&self) -> anyhow::Result<()> {
            Ok(())
        }

        async fn flush_key(
            &self,
            _: &str,
            _: &str,
            _: &RecordValue,
            _: bool,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        fn reset(&self) {}

        /// A real (non-null) backend, so the full write-through path runs and a
        /// non-resident key is rehydrated from durable state on read.
        fn is_null(&self) -> bool {
            false
        }
    }

    /// One write as the store boundary saw it, with whatever witness rode along.
    #[derive(Debug, Clone)]
    struct WitnessObservation {
        map: String,
        key: String,
        value: RecordValue,
        witness: Option<OrDelta>,
    }

    /// Observes what actually crosses the store boundary, and can answer the
    /// demand signal either way.
    ///
    /// This is deliberately ONE type covering every configuration the assertions
    /// need — armed and unarmed, standalone and delegating. A second data-store
    /// implementation in this file would keep the file-level allow-list green
    /// while inflating the per-file site count the cascade assertion reads, so
    /// the count would stop meaning "nobody overrode this".
    ///
    /// When `inner` is set every call is forwarded to it, so the spy can sit in
    /// front of a real WAL-backed store and observe without displacing it; when
    /// it is `None` the spy is itself the backend, holding its own map.
    struct WitnessSpyStore {
        inner: Option<Arc<dyn MapDataStore>>,
        armed: bool,
        observed: Mutex<Vec<WitnessObservation>>,
        data: Mutex<HashMap<(String, String), RecordValue>>,
    }

    impl WitnessSpyStore {
        /// Standalone backend. `armed` decides the answer to the demand signal,
        /// which is the whole point of the unarmed configuration: it proves the
        /// demand gate suppresses a witness the effect gate would have produced.
        fn standalone(armed: bool) -> Self {
            Self {
                inner: None,
                armed,
                observed: Mutex::new(Vec::new()),
                data: Mutex::new(HashMap::new()),
            }
        }

        fn observations(&self) -> Vec<WitnessObservation> {
            self.observed.lock().clone()
        }

        /// Every witness the boundary received, in arrival order.
        fn witnesses(&self) -> Vec<Option<OrDelta>> {
            self.observed
                .lock()
                .iter()
                .map(|seen| seen.witness.clone())
                .collect()
        }
    }

    #[async_trait]
    impl MapDataStore for WitnessSpyStore {
        async fn add(
            &self,
            map: &str,
            key: &str,
            value: &RecordValue,
            expiration_time: i64,
            now: i64,
        ) -> anyhow::Result<()> {
            if let Some(inner) = &self.inner {
                return inner.add(map, key, value, expiration_time, now).await;
            }
            self.data
                .lock()
                .insert((map.to_string(), key.to_string()), value.clone());
            Ok(())
        }

        /// Records the pair, then persists exactly as the defaulted body would.
        ///
        /// Recording BOTH the value and the witness is what lets a caller check
        /// they describe the same mutation: a witness folded onto the pre-image
        /// has to reproduce the value that was written beside it.
        async fn add_with_witness(
            &self,
            map: &str,
            key: &str,
            src: WriteSource<'_>,
            expiration_time: i64,
            now: i64,
            witness: Option<&OrDelta>,
        ) -> anyhow::Result<()> {
            let value = src.to_value();
            self.observed.lock().push(WitnessObservation {
                map: map.to_string(),
                key: key.to_string(),
                value: value.clone(),
                witness: witness.cloned(),
            });
            if let Some(inner) = &self.inner {
                return inner
                    .add_with_witness(map, key, src, expiration_time, now, witness)
                    .await;
            }
            self.add(map, key, &value, expiration_time, now).await
        }

        fn wants_or_witness(&self) -> bool {
            self.armed
        }

        async fn add_backup(
            &self,
            _: &str,
            _: &str,
            _: &RecordValue,
            _: i64,
            _: i64,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn remove(&self, map: &str, key: &str, now: i64) -> anyhow::Result<()> {
            if let Some(inner) = &self.inner {
                return inner.remove(map, key, now).await;
            }
            self.data.lock().remove(&(map.to_string(), key.to_string()));
            Ok(())
        }

        async fn remove_backup(&self, _: &str, _: &str, _: i64) -> anyhow::Result<()> {
            Ok(())
        }

        async fn load(&self, map: &str, key: &str) -> anyhow::Result<Option<RecordValue>> {
            if let Some(inner) = &self.inner {
                return inner.load(map, key).await;
            }
            Ok(self
                .data
                .lock()
                .get(&(map.to_string(), key.to_string()))
                .cloned())
        }

        async fn load_all(
            &self,
            map: &str,
            keys: &[String],
        ) -> anyhow::Result<Vec<(String, RecordValue)>> {
            if let Some(inner) = &self.inner {
                return inner.load_all(map, keys).await;
            }
            let guard = self.data.lock();
            Ok(keys
                .iter()
                .filter_map(|key| {
                    guard
                        .get(&(map.to_string(), key.clone()))
                        .map(|value| (key.clone(), value.clone()))
                })
                .collect())
        }

        async fn remove_all(&self, map: &str, keys: &[String]) -> anyhow::Result<()> {
            if let Some(inner) = &self.inner {
                return inner.remove_all(map, keys).await;
            }
            let mut guard = self.data.lock();
            for key in keys {
                guard.remove(&(map.to_string(), key.clone()));
            }
            Ok(())
        }

        async fn enumerate_leaves(
            &self,
            _: &str,
            _: bool,
            _: &mut dyn LeafSink,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn scan_values(&self, _: &str, _: bool, _: u64) -> anyhow::Result<ScanBatch> {
            Ok(ScanBatch::default())
        }

        async fn scan_values_batched(
            &self,
            _: &str,
            _: bool,
            _: ScanCursor,
            _: u64,
        ) -> anyhow::Result<ScanBatch> {
            Ok(ScanBatch::default())
        }

        fn is_loadable(&self, _: &str) -> bool {
            true
        }

        fn pending_operation_count(&self) -> u64 {
            self.inner
                .as_ref()
                .map_or(0, |inner| inner.pending_operation_count())
        }

        async fn soft_flush(&self) -> anyhow::Result<u64> {
            match &self.inner {
                Some(inner) => inner.soft_flush().await,
                None => Ok(0),
            }
        }

        async fn hard_flush(&self) -> anyhow::Result<()> {
            match &self.inner {
                Some(inner) => inner.hard_flush().await,
                None => Ok(()),
            }
        }

        async fn flush_key(
            &self,
            map: &str,
            key: &str,
            value: &RecordValue,
            deleted: bool,
        ) -> anyhow::Result<()> {
            match &self.inner {
                Some(inner) => inner.flush_key(map, key, value, deleted).await,
                None => Ok(()),
            }
        }

        fn reset(&self) {}

        /// A real (non-null) backend, so the full write-through path runs.
        fn is_null(&self) -> bool {
            false
        }
    }

    /// An order-insensitive view of an OR slot, for comparing a folded witness
    /// against the value that was written beside it.
    ///
    /// Deliberately local rather than reusing the delta-fold equivalence oracle
    /// defined earlier in this file: that oracle's own contract states the
    /// differential recovery test is its first consumer, so consuming it here
    /// would make that contract false.
    fn canonical_or_slot(value: &RecordValue) -> (Vec<(String, String)>, Vec<String>) {
        match value {
            RecordValue::OrMap {
                records,
                tombstones,
            } => {
                let mut live: Vec<(String, String)> = records
                    .iter()
                    .map(|entry| (entry.tag.clone(), format!("{:?}", entry.value)))
                    .collect();
                live.sort();
                let mut dead = tombstones.clone();
                dead.sort();
                (live, dead)
            }
            other => (vec![(String::new(), format!("{other:?}"))], Vec::new()),
        }
    }

    fn empty_or_slot() -> RecordValue {
        RecordValue::OrMap {
            records: Vec::new(),
            tombstones: Vec::new(),
        }
    }

    /// Drive at least one EFFECTIVE mutation of each kind — an accepted add, a
    /// genuinely-new tombstone, and a prune that drops a tag — through the real
    /// service over the given spy, and confirm each really took effect.
    ///
    /// Two keys rather than one because the epoch gate only opens once the
    /// low-water mark has moved past the first epoch, and this fixture stamps one
    /// epoch per tombstone.
    ///
    /// Shared so both callers below assert over the SAME effect cases: one proves
    /// the demand gate suppresses every one of them, the other proves the
    /// witnesses they produce describe the writes they rode with.
    async fn drive_effective_or_mutations_of_every_kind(spy: &Arc<WitnessSpyStore>) {
        let (svc, factory, frontier) = make_service_with_frontier_and_store(
            Arc::clone(spy) as Arc<dyn MapDataStore>,
            Vec::new(),
        );

        for (key, value, tag) in [("k1", "v1", "T1"), ("k2", "v2", "T2")] {
            Arc::clone(&svc)
                .oneshot(or_add_op("m", key, value, tag))
                .await
                .unwrap();
            Arc::clone(&svc)
                .oneshot(or_remove_op("m", key, tag))
                .await
                .unwrap();

            // The add was accepted and the remove stamped a NEW tombstone, or
            // two of the three effect kinds never happened here.
            let mid = spy.load("m", key).await.unwrap();
            assert!(
                matches!(&mid, Some(RecordValue::OrMap { tombstones, .. })
                         if tombstones.contains(&tag.to_string())),
                "precondition: the remove must have stamped a new tombstone, got {mid:?}"
            );
        }

        open_prune_gates_past_epoch_one(&frontier).await;
        prune_epoch_tombstones(&frontier, &factory, &svc.key_writer).await;

        // At least one prune dropped a tag, or the third effect kind is a
        // no-effect op wearing an effective op's name.
        let mut pruned_any = false;
        for (key, tag) in [("k1", "T1"), ("k2", "T2")] {
            let after = spy.load("m", key).await.unwrap();
            if matches!(&after, Some(RecordValue::OrMap { tombstones, .. })
                        if !tombstones.contains(&tag.to_string()))
            {
                pruned_any = true;
            }
        }
        assert!(
            pruned_any,
            "precondition: the sweep must have dropped at least one tombstone"
        );
    }

    /// With no consumer armed, the boundary receives NO witness -- not even for
    /// the three mutations that did take effect.
    ///
    /// This is what separates the demand gate from the effect gate
    /// behaviourally: on these three inputs the effect gate alone would have
    /// produced a witness every time, so neither gate can be deleted while the
    /// other silently absorbs its job.
    #[tokio::test]
    async fn an_unarmed_consumer_receives_no_witness_even_for_effective_mutations() {
        let spy = Arc::new(WitnessSpyStore::standalone(false));
        drive_effective_or_mutations_of_every_kind(&spy).await;

        let witnesses = spy.witnesses();
        assert!(
            witnesses.len() >= 3,
            "the boundary must have seen at least the effective writes, saw {}",
            witnesses.len()
        );
        assert!(
            witnesses.iter().all(Option::is_none),
            "an unarmed consumer must receive no witness at all, got {witnesses:?}"
        );
    }

    /// The witness and the value handed to the boundary describe the SAME
    /// mutation.
    ///
    /// Asserted by folding each witness onto the previous value written for that
    /// key and requiring the result to equal the value it arrived with. A witness
    /// that described a different op -- or the same op on a different value --
    /// would not reproduce it.
    #[tokio::test]
    async fn a_witness_folds_onto_the_pre_image_to_give_the_value_it_rode_with() {
        let spy = Arc::new(WitnessSpyStore::standalone(true));
        drive_effective_or_mutations_of_every_kind(&spy).await;

        let seen = spy.observations();
        let mut pre_image: HashMap<(String, String), RecordValue> = HashMap::new();
        let (mut adds, mut removes, mut prunes) = (0usize, 0usize, 0usize);
        for observation in &seen {
            let slot = (observation.map.clone(), observation.key.clone());
            if let Some(delta) = observation.witness.clone() {
                match &delta {
                    OrDelta::Add { .. } => adds += 1,
                    OrDelta::Remove { .. } => removes += 1,
                    OrDelta::Prune { .. } => prunes += 1,
                }
                let mut base = pre_image.get(&slot).cloned().unwrap_or_else(empty_or_slot);
                apply_or_delta(delta, &mut base);
                assert_eq!(
                    canonical_or_slot(&base),
                    canonical_or_slot(&observation.value),
                    "the witness for {slot:?} did not reproduce the value it rode with"
                );
            }
            pre_image.insert(slot, observation.value.clone());
        }
        // All three kinds must be represented, or a kind whose witness is never
        // produced would pass this vacuously.
        assert!(
            adds > 0 && removes > 0 && prunes > 0,
            "every effect kind must have produced a witness to fold; \
             adds={adds} removes={removes} prunes={prunes}"
        );
    }

    /// A witness exists if and only if the mutation actually took effect.
    ///
    /// The two no-effect rows here are the ones that matter: both still owe a
    /// durable write — the closures re-persist the slot unconditionally, exactly
    /// as they did before this seam — so both DO reach the store boundary. What
    /// they must not carry is a witness. Recording a mutation that did not happen
    /// is how a replay would resurrect a suppressed add.
    #[tokio::test]
    async fn only_an_effective_add_or_remove_carries_a_witness() {
        let spy = Arc::new(WitnessSpyStore::standalone(true));
        let (svc, _factory, _frontier) = make_service_with_frontier_and_store(
            Arc::clone(&spy) as Arc<dyn MapDataStore>,
            Vec::new(),
        );

        for op in [
            or_add_op("m", "k1", "v1", "T1"), // effect: inserted
            or_remove_op("m", "k1", "T1"),    // effect: new tombstone
            or_add_op("m", "k1", "v1", "T1"), // NO effect: remove-wins suppressed
            or_remove_op("m", "k1", "T1"),    // NO effect: duplicate remove
        ] {
            Arc::clone(&svc).oneshot(op).await.unwrap();
        }

        let seen: Vec<Option<OrDelta>> = spy
            .observations()
            .into_iter()
            .filter(|obs| obs.map == "m" && obs.key == "k1")
            .map(|obs| obs.witness)
            .collect();
        assert_eq!(
            seen.len(),
            4,
            "all four ops owe a durable write, so all four must reach the boundary; saw {seen:?}"
        );

        assert!(
            matches!(&seen[0], Some(OrDelta::Add { entry }) if entry.tag == "T1"),
            "an accepted add carries its entry, got {:?}",
            seen[0]
        );
        assert!(
            matches!(&seen[1], Some(OrDelta::Remove { tag }) if tag == "T1"),
            "a genuinely-new tombstone carries its tag, got {:?}",
            seen[1]
        );
        assert!(
            seen[2].is_none(),
            "a remove-wins-suppressed add changed nothing and must carry no witness, got {:?}",
            seen[2]
        );
        assert!(
            seen[3].is_none(),
            "a duplicate remove changed nothing and must carry no witness, got {:?}",
            seen[3]
        );
    }

    /// A mutation that took no effect writes a FULL RECORD to the log, never a
    /// per-op mutation frame — and its replay therefore cannot resurrect it.
    ///
    /// Driven end to end against a REAL write-ahead log under an armed store,
    /// because the effect gate lives at the mutation point and the framing arm
    /// lives at the store boundary: only a run that crosses both can show that
    /// deleting the gate changes what reaches the disk.
    ///
    /// The frame-kind claim is written POSITIVELY, which entails the negative:
    /// the only other inhabitants of the op enum are a remove frame, the per-op
    /// mutation frame, and a test-only variant. Writing it as "is not a mutation
    /// frame" would put that variant's name in this file, and a package-wide belt
    /// admits that string only in the log's own modules and in the emitter.
    ///
    /// The resurrection question is MOOT rather than answered: a suppressed add
    /// leaves no per-op frame for a replay to fold, so replaying the window over
    /// a base that does NOT carry the suppressing tombstone still reproduces the
    /// live semantic set — asserted here as the equality of the last frame's
    /// absolute payload with the value the live path holds.
    #[tokio::test]
    async fn a_no_effect_or_write_frames_a_full_record_that_cannot_resurrect_it() {
        use crate::storage::datastores::{WalBootstrap, WriteBehindConfig, WriteBehindDataStore};
        use crate::storage::wal::format::{decode_all, FrameDecodeResult};
        use crate::storage::wal::segment::parse_segment_filename;
        use crate::storage::wal::{Wal, WalEntry, WalFsyncPolicy, WalOp, WalWriter};

        fn frames(dir: &std::path::Path) -> Vec<WalEntry> {
            let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
                .unwrap()
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.is_file()
                        && p.file_name()
                            .and_then(|n| n.to_str())
                            .and_then(parse_segment_filename)
                            .is_some()
                })
                .collect();
            paths.sort();
            let mut out = Vec::new();
            for path in paths {
                let bytes = std::fs::read(&path).unwrap();
                let entries = match decode_all(&bytes) {
                    FrameDecodeResult::Complete(entries) => entries,
                    FrameDecodeResult::TruncatedTail { complete } => complete,
                    FrameDecodeResult::CleanEof => Vec::new(),
                    other => panic!("unexpected decode result: {other:?}"),
                };
                out.extend(entries);
            }
            out.sort_by_key(|e| e.sequence);
            out
        }

        let wal_dir = tempfile::tempdir().unwrap();
        // Every frame is fsynced before its write acks, so a read-back cannot
        // pass by observing an empty segment. The long delays keep the frames
        // un-applied, so watermark-driven segment reclamation cannot remove the
        // very frames under assertion.
        let wal = WalWriter::new(wal_dir.path().to_path_buf(), WalFsyncPolicy::PerOp).unwrap();
        let store = WriteBehindDataStore::new_with_wal(
            Arc::new(NullDataStore) as Arc<dyn MapDataStore>,
            WriteBehindConfig {
                write_delay_ms: 60_000,
                flush_interval_ms: 60_000,
                shutdown_timeout_ms: 5_000,
                or_delta_wal: true,
                ..WriteBehindConfig::default()
            },
            Some(WalBootstrap {
                wal: Arc::clone(&wal) as Arc<dyn Wal>,
                sequence_start: 1,
            }),
        );
        let (svc, _factory, _frontier) = make_service_with_frontier_and_store(
            Arc::clone(&store) as Arc<dyn MapDataStore>,
            Vec::new(),
        );

        for op in [
            or_add_op("m", "k1", "v1", "T1"), // effect: inserted
            or_remove_op("m", "k1", "T1"),    // effect: new tombstone
        ] {
            Arc::clone(&svc).oneshot(op).await.unwrap();
        }
        let effective = frames(wal_dir.path()).len();
        // Non-vacuity, both halves: the effective ops must have reached the log
        // (or the "new frames" set below is not the no-effect ops' frames), and
        // they must NOT have framed a full record — a path that stopped
        // delivering a witness would frame snapshots throughout and satisfy
        // every assertion below. Names the variant the frame must NOT be, never
        // the one it must be, so the belts still hold over this file.
        assert!(
            effective >= 2
                && frames(wal_dir.path())
                    .iter()
                    .all(|f| !matches!(f.op, WalOp::Store { .. })),
            "the two effective writes must have framed something, and never a full record; \
             saw {effective} frames"
        );

        for op in [
            or_add_op("m", "k1", "v1", "T1"), // NO effect: remove-wins suppressed
            or_remove_op("m", "k1", "T1"),    // NO effect: duplicate remove
        ] {
            Arc::clone(&svc).oneshot(op).await.unwrap();
        }

        let all = frames(wal_dir.path());
        let appended = &all[effective..];
        assert!(
            !appended.is_empty(),
            "both no-effect ops still owe a durable write, so both must have framed one"
        );
        for frame in appended {
            assert!(
                matches!(frame.op, WalOp::Store { .. }),
                "a no-effect OR write must re-persist its whole slot as a full record, \
                 got a different frame kind at sequence {}",
                frame.sequence
            );
        }

        // Replay semantics: the newest frame is an ABSOLUTE set, so folding the
        // window over a base that never saw the suppressing tombstone still lands
        // on the live value. A per-op frame for the suppressed add is what would
        // have made this untrue, and there is none to fold.
        let live = store.load("m", "k1").await.unwrap().expect("a live value");
        let replayed = match &all.last().expect("at least one frame").op {
            WalOp::Store { value, .. } => value.clone(),
            other => panic!("the newest frame must be a full record, got {other:?}"),
        };
        assert_eq!(
            format!("{replayed:?}"),
            format!("{:?}", crate::storage::wal::WalStorePayload::Record(live)),
            "replaying the window over an empty base must reproduce the live semantic \
             set -- a resurrected suppressed add would differ here"
        );
    }

    /// A sweep whose tag was already gone reaches the store boundary NOT AT ALL.
    ///
    /// The write-owed answer for this row is `false` — nothing was pruned and no
    /// shape changed — so the in-place seam reports "unchanged" and never performs
    /// a write-through. There is therefore no witness to inspect, and that is the
    /// contract: the absence of a witness here is the absence of a whole write.
    ///
    /// Stated as a boundary-write count rather than as "every witness is None",
    /// because over an empty observation set that phrasing is vacuously true. An
    /// earlier draft asserted exactly that and passed while proving nothing.
    #[tokio::test]
    async fn a_sweep_whose_tag_was_already_gone_writes_nothing_at_all() {
        let spy = Arc::new(WitnessSpyStore::standalone(true));
        let (svc, factory, frontier) = make_service_with_frontier_and_store(
            Arc::clone(&spy) as Arc<dyn MapDataStore>,
            Vec::new(),
        );

        // A resident record whose tombstone set does NOT hold the tag the sweep
        // will look for, so the prune closure runs and matches nothing.
        Arc::clone(&svc)
            .oneshot(or_add_op("m", "k1", "v1", "T1"))
            .await
            .unwrap();
        let ghost = "GHOST";
        assert_eq!(
            frontier.stamp_tombstone("m", "k1", ghost),
            1,
            "the ghost ref sits in epoch 1"
        );
        // Advances the epoch counter so the low-water mark can sit strictly past
        // the ghost's epoch without making this one eligible too.
        assert_eq!(
            frontier.stamp_tombstone("m", "k2", "FILLER"),
            2,
            "the filler pins epoch 2"
        );
        open_prune_gates_past_epoch_one(&frontier).await;

        let before = spy.observations().len();
        prune_epoch_tombstones(&frontier, &factory, &svc.key_writer).await;

        // Non-vacuity: the sweep must actually have consumed the ghost ref, i.e.
        // the closure RAN and reported "already gone". Without this, a sweep that
        // never fired would satisfy the zero below.
        let retryable = frontier.drain_prunable_tombstones();
        assert!(
            !retryable.iter().any(|(_, r)| r.tag == ghost),
            "the sweep must have run and consumed the ghost ref, got {retryable:?}"
        );
        assert_eq!(
            spy.observations().len(),
            before,
            "a sweep that pruned nothing and changed no shape owes no durable write, \
             so it must not reach the store boundary at all"
        );
    }

    /// A shape upgrade that drops no tag DOES write, and carries no witness.
    ///
    /// This is the `pruned == 0` row where the gate is actually observable: a
    /// legacy-shaped slot is upgraded, so the write is owed and performed, and the
    /// boundary sees a real write whose witness must still be absent because
    /// nothing was pruned. The already-gone row above cannot prove this — it
    /// performs no write, so it has no witness to be wrong about.
    #[tokio::test]
    async fn a_shape_upgrade_that_prunes_nothing_writes_without_a_witness() {
        let spy = Arc::new(WitnessSpyStore::standalone(true));
        let (svc, factory, frontier) = make_service_with_frontier_and_store(
            Arc::clone(&spy) as Arc<dyn MapDataStore>,
            Vec::new(),
        );

        // Durable-only, in the pre-OrMap shape an older server wrote, holding a
        // tombstone the sweep is NOT asked to drop. Rehydrating it makes the prune
        // closure normalize the shape while pruning nothing.
        spy.add(
            "m",
            "k1",
            &RecordValue::OrTombstones {
                tags: vec!["OLD".to_string()],
            },
            0,
            0,
        )
        .await
        .unwrap();
        let ghost = "GHOST";
        assert_eq!(frontier.stamp_tombstone("m", "k1", ghost), 1);
        assert_eq!(frontier.stamp_tombstone("m", "k2", "FILLER"), 2);
        open_prune_gates_past_epoch_one(&frontier).await;

        let before = spy.observations().len();
        prune_epoch_tombstones(&frontier, &factory, &svc.key_writer).await;

        let after: Vec<Option<OrDelta>> = spy
            .observations()
            .into_iter()
            .skip(before)
            .map(|obs| obs.witness)
            .collect();
        // Non-vacuity: the upgrade must have owed and performed a real write, or
        // there is no witness to be absent.
        assert!(
            !after.is_empty(),
            "the shape upgrade owes a durable write, so the boundary must have seen one"
        );
        assert!(
            after.iter().all(Option::is_none),
            "nothing was pruned, so the write must carry no witness, got {after:?}"
        );
    }

    /// A prune pass that is cancelled mid-flight must hand every ref it drained
    /// but never settled back to the index.
    ///
    /// `TG-OR-004`: the drain removes a ref from the RAM index BEFORE the pass
    /// tries to drop its tag from storage, so a pass that stops early leaves
    /// those refs named by nobody — no later sweep can retry a tag whose index
    /// entry is gone, and the tombstone bytes it names are stranded in storage
    /// for the life of the process. A held per-key writer makes the stop
    /// deterministic rather than a race: the pass can never get past `K_HELD`.
    #[allow(clippy::too_many_lines)]
    #[tokio::test]
    async fn cancelled_prune_pass_restores_every_unsettled_ref() {
        const N: usize = 4;
        const K_HELD: &str = "kheld";

        let (svc, factory, frontier) = make_service_with_frontier();

        // Epoch width is 1, so each OR_REMOVE lands its own ref in its own
        // epoch and which epochs are eligible is decided by the injected gates
        // below rather than by a clock. The gates are still shut here, so the
        // inline prune every seeding OR_REMOVE runs drains nothing.
        let mut seeded: Vec<(String, String)> = Vec::new();
        for i in 0..N {
            let key = if i == 0 {
                K_HELD.to_string()
            } else {
                format!("k{i}")
            };
            let tag = format!("T{i}");
            Arc::clone(&svc)
                .oneshot(or_add_op("m", &key, "v", &tag))
                .await
                .unwrap();
            Arc::clone(&svc)
                .oneshot(or_remove_op("m", &key, &tag))
                .await
                .unwrap();
            seeded.push((key, tag));
        }
        assert_eq!(
            frontier.current_epoch(),
            N as u64,
            "one stamped tombstone per seeded pair, one epoch each"
        );

        // Pins the epoch counter one past the seeded refs, so the cursor can sit
        // strictly past every seeded epoch without making a further one eligible.
        frontier.stamp_tombstone("m", "kfiller", "FILLER");

        // Both conjuncts open past the seeded epochs only: eligibility is STRICT,
        // so a cursor at N+1 licenses epochs 1..=N and leaves the filler out.
        let client: String = "a5:alice|dev-1".into();
        frontier.set_delivered(ConnectionId(1), 10_000);
        assert!(
            frontier
                .confirm_apply_ack(&client, (N + 1) as u64, ConnectionId(1))
                .await
        );
        assert_eq!(frontier.low_water_mark(), (N + 1) as u64);
        frontier.set_durable_epoch_watermark(1000);

        // The pass takes this same per-key writer per dropped tag, so holding it
        // blocks the pass forever on K_HELD and the budget below always elapses.
        let held = svc.key_writer.acquire("m", K_HELD).await;

        let (guard, sink) = capture_tracing_rows();
        let cancelled = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            prune_epoch_tombstones(&frontier, &factory, &svc.key_writer),
        )
        .await;
        assert!(
            cancelled.is_err(),
            "the pass cannot pass the held writer, so it must be cancelled"
        );
        drop(held);
        // The capture is thread-local and the rows are read off it only once the
        // guard is gone; the index assertion's own drain runs after that, since
        // the drain mutates the very index it inspects.
        drop(guard);
        let rows = sink.lock().unwrap().clone();

        // Conservation is read HERE, ahead of the inspection drain below, because that
        // drain is a measurement artifact rather than part of the property: the drain
        // decrements `indexed_refs` once per REF, while `drained_refs_total` is credited
        // once per epoch SLOT, and a ref handed back into an already-exited epoch has no
        // slot to credit. Re-draining what the cancelled pass restored therefore subtracts
        // from one side of the identity and not the other. Read after it, this would
        // measure that asymmetry instead of whether the cancelled pass conserved the index.
        let snapshot = frontier.index_conservation_snapshot();
        assert_eq!(
            snapshot.stamped_refs_total + snapshot.restored_refs_total
                - snapshot.drained_refs_total
                - snapshot.rebuild_cleared_refs_total,
            snapshot.indexed_refs,
            "O-0 must hold across a cancelled pass too, got {snapshot:?}"
        );

        let retryable = frontier.drain_prunable_tombstones();
        let mut durably_gone = 0usize;
        for (key, tag) in &seeded {
            let (_, tombs) = read_or_map(&factory, "m", key).await;
            if !tombs.contains(tag) {
                durably_gone += 1;
            }
        }

        assert!(
            retryable.iter().any(|(_, r)| r.key == K_HELD),
            "the ref the cancelled pass was blocked on must be back in the index, \
             got {retryable:?}"
        );
        // Order-independent: the drain visits epochs in hash order, so which refs
        // settled before the block is not assumed — only that every seeded ref is
        // either settled durably or still retryable.
        assert_eq!(
            retryable.len() + durably_gone,
            N,
            "every seeded ref must be accounted for, either durably dropped or \
             handed back; got {} retryable and {durably_gone} durably gone",
            retryable.len()
        );

        let pass_rows: Vec<&str> = rows
            .iter()
            .map(String::as_str)
            .filter(|l| {
                is_row(
                    l,
                    "topgun_server::tombstone_frontier::residency",
                    "prune_pass",
                )
            })
            .collect();
        assert_eq!(
            pass_rows.len(),
            1,
            "a cancelled pass still owes exactly one pass row; capture was:\n{rows:#?}"
        );

        // The settlement row carries no `kind` field, so filter on target alone.
        let settlement_rows: Vec<&str> = rows
            .iter()
            .map(String::as_str)
            .filter(|l| l.contains(" target=topgun_server::tombstone_frontier::settlement "))
            .collect();
        let mut considered_sum = 0u64;
        let mut restored_cancelled_sum = 0u64;
        for row in &settlement_rows {
            let considered = row_u64(row, "considered");
            let dropped = row_u64(row, "dropped");
            let matched_nothing = row_u64(row, "matched_nothing");
            let absent = row_u64(row, "absent");
            let restored_read_error = row_u64(row, "restored_read_error");
            let restored_evicted = row_u64(row, "restored_evicted");
            let restored_write_error = row_u64(row, "restored_write_error");
            let restored_cancelled = row_u64(row, "restored_cancelled");
            assert_eq!(
                considered,
                dropped
                    + matched_nothing
                    + absent
                    + restored_read_error
                    + restored_evicted
                    + restored_write_error
                    + restored_cancelled,
                "the per-epoch exit identity must hold on this settlement row: {row:?}"
            );
            considered_sum += considered;
            restored_cancelled_sum += restored_cancelled;
        }
        assert_eq!(
            considered_sum, N as u64,
            "every eligible ref must be considered by the cancelled pass"
        );
        assert_eq!(
            restored_cancelled_sum,
            (N - durably_gone) as u64,
            "every ref the pass did not settle must leave through the cancelled exit"
        );
        assert!(
            restored_cancelled_sum >= 1,
            "the blocked ref alone makes this exit nonempty"
        );
    }

    /// An `OR_REMOVE` carried through the PRODUCTION `TimeoutLayer` must not lose
    /// the refs a prune drained.
    ///
    /// `TG-OR-006`: a pass run on the caller's own timeline sits inside that
    /// request's timeout budget, so a per-key writer held anywhere in the drained
    /// set makes the budget elapse and the layer cancel the call mid-pass. The
    /// op's own remove is applied regardless, so the caller is told the write
    /// timed out while the server kept it — and the drained refs are gone from
    /// the index. Asking for the pass instead of running it is what keeps the op
    /// inside its budget. This is the behavioural proof on the production
    /// mechanism, not on a synthetic pass.
    #[tokio::test]
    async fn or_remove_under_the_timeout_layer_never_loses_drained_refs() {
        const K_HELD: &str = "kheld";
        const K_OP: &str = "kop";
        const BUDGET_MS: u64 = 500;

        let (svc, factory, frontier) = make_service_with_frontier();

        // Two eligible refs, on two distinct keys, one of them the key whose
        // writer the test holds.
        for (key, tag) in [(K_HELD, "THELD"), ("kother", "TOTHER")] {
            Arc::clone(&svc)
                .oneshot(or_add_op("m", key, "v", tag))
                .await
                .unwrap();
            Arc::clone(&svc)
                .oneshot(or_remove_op("m", key, tag))
                .await
                .unwrap();
        }
        // The record the op under test removes from.
        Arc::clone(&svc)
            .oneshot(or_add_op("m", K_OP, "v", "TOP"))
            .await
            .unwrap();

        // Pins the epoch counter one past the seeded refs: the low-water mark
        // cannot advance beyond the epochs that exist, so without this stamp a
        // cursor past epoch 2 would clamp back to 2 and license only epoch 1.
        frontier.stamp_tombstone("m", "kfiller", "FILLER");

        // Past the two seeded epochs only. The filler holds epoch 3 and the op's
        // own stamp lands in epoch 4, neither of which a cursor at 3 licenses, so
        // the drain sees exactly the two seeded refs.
        let client: String = "a5:alice|dev-1".into();
        frontier.set_delivered(ConnectionId(1), 10_000);
        assert!(
            frontier
                .confirm_apply_ack(&client, 3, ConnectionId(1))
                .await
        );
        assert_eq!(frontier.low_water_mark(), 3);
        frontier.set_durable_epoch_watermark(1000);

        // Built here rather than via `or_remove_op`: the budget has to be set on
        // the context BEFORE the op is constructed, and there is no `ctx_mut`.
        let mut ctx = make_ctx_for_key(K_OP);
        ctx.call_timeout_ms = BUDGET_MS;
        let op = Operation::ClientOp {
            ctx,
            payload: topgun_core::messages::ClientOpMessage {
                payload: topgun_core::messages::base::ClientOp {
                    id: Some("rm-TOP".to_string()),
                    map_name: "m".to_string(),
                    key: K_OP.to_string(),
                    op_type: None,
                    record: None,
                    or_record: None,
                    or_tag: Some(Some("TOP".to_string())),
                    write_concern: None,
                    timeout: None,
                },
            },
        };

        let held = svc.key_writer.acquire("m", K_HELD).await;
        let layered = tower::ServiceBuilder::new()
            .layer(crate::service::middleware::TimeoutLayer)
            .service(Arc::clone(&svc));
        let outcome = layered.oneshot(op).await;
        drop(held);

        // Both halves of the witness are printed BEFORE the first assertion, so
        // the applied-then-timed-out pair is readable in the failure output even
        // though the assertion that fails is the one about the returned value.
        let (_, tombs) = read_or_map(&factory, "m", K_OP).await;
        println!("witness: stored tombstones on {K_OP} = {tombs:?}");
        println!("witness: returned = {outcome:?}");

        assert!(
            outcome.is_ok(),
            "an OR_REMOVE must complete within its own budget, got {outcome:?}"
        );
        // What the op did instead of pruning: it left a wake permit. This fixture
        // spawns no task, so the permit is still pending for the assertion to
        // consume.
        let wake = frontier.prune_wake();
        tokio::time::timeout(std::time::Duration::from_millis(1), wake.notified())
            .await
            .expect("the OR_REMOVE must leave a wake permit pending for the prune task");
        let retryable = frontier.drain_prunable_tombstones();
        assert!(
            retryable.iter().any(|(_, r)| r.key == K_HELD),
            "the blocked key's ref must still be indexed after the call, got {retryable:?}"
        );
    }

    /// An `OR_REMOVE` asks for a prune pass; it never runs one.
    ///
    /// The trigger has to stay O(1) whatever the backlog, so with a large
    /// eligible set indexed and both gates open the op must drain NONE of it,
    /// leave a wake permit behind, and grow the index by exactly its own stamp.
    /// The drain counter is re-read after a yield as well: a regression that
    /// spawned a per-op pass would satisfy a single immediate read, because the
    /// spawned pass has not been polled yet. The structural limb closes the same
    /// gap from the other side — the arm names no spawn at all — and the pair is
    /// what makes "O(1)" a property of the code rather than of the wall clock.
    #[tokio::test]
    async fn or_remove_only_wakes_the_prune_task_and_drains_nothing() {
        const BACKLOG: u64 = 1_000;
        const SOURCE: &str = include_str!("crdt.rs");
        let (svc, _factory, frontier) = make_service_with_frontier();

        // The record the op under test removes from, added before the backlog so
        // its own slot is never one of the eligible refs.
        Arc::clone(&svc)
            .oneshot(or_add_op("m", "kop", "v", "TOP"))
            .await
            .unwrap();

        // Epoch width is 1, so each stamp owns an epoch: the backlog fills epochs
        // 1..=BACKLOG and the pin takes the next one, letting the cursor sit
        // strictly past every backlog epoch without exceeding the epochs that
        // exist.
        for i in 1..=BACKLOG {
            frontier.stamp_tombstone("m", &format!("kbacklog{i}"), &format!("TB{i}"));
        }
        assert_eq!(
            frontier.stamp_tombstone("m", "kpin", "TPIN"),
            BACKLOG + 1,
            "the pin owns the epoch past the backlog"
        );
        let client: String = "a5:alice|dev-1".into();
        frontier.set_delivered(ConnectionId(1), 10_000);
        assert!(
            frontier
                .confirm_apply_ack(&client, BACKLOG + 1, ConnectionId(1))
                .await
        );
        assert_eq!(frontier.low_water_mark(), BACKLOG + 1);
        frontier.set_durable_epoch_watermark(1_000_000);

        let before = frontier.index_conservation_snapshot();
        assert!(
            before.indexed_refs >= BACKLOG,
            "precondition: the op runs against a real backlog, got {before:?}"
        );

        Arc::clone(&svc)
            .oneshot(or_remove_op("m", "kop", "TOP"))
            .await
            .expect("the OR_REMOVE must return Ok");

        let after = frontier.index_conservation_snapshot();
        assert_eq!(
            after.drained_refs_total, before.drained_refs_total,
            "the op must drain nothing: reclamation is the task's work, not the \
             caller's, got {after:?}"
        );
        assert_eq!(
            after.indexed_refs,
            before.indexed_refs + 1,
            "the index grows by exactly the op's own stamp, got {after:?}"
        );

        // What the op did instead: it left a wake permit. Nothing consumes it
        // here — the fixture spawns no task — so it is still pending.
        let wake = frontier.prune_wake();
        tokio::time::timeout(std::time::Duration::from_millis(1), wake.notified())
            .await
            .expect("the OR_REMOVE must leave a wake permit pending for the prune task");

        // Re-read after a yield: a spawned per-op pass would be invisible to the
        // read above simply because it had not been polled yet.
        tokio::task::yield_now().await;
        assert_eq!(
            frontier.index_conservation_snapshot().drained_refs_total,
            before.drained_refs_total,
            "still nothing drained once other tasks have had a chance to run"
        );

        // The same claim structurally: no arm of the write path may hand the pass
        // to a task of its own. A per-op task is a per-op pass with a thread hop
        // in front of it, which is the unbounded work this trigger exists to
        // avoid, and it would also put a second writer over the one index.
        let arm = SOURCE
            .split_once("} else if is_or_remove {")
            .expect("the OR_REMOVE arm is in this file")
            .1;
        let arm = arm
            .split_once("\n        } else {")
            .expect("the OR_REMOVE arm closes into the LWW PUT branch")
            .0;
        assert!(
            !arm.contains("spawn"),
            "the OR_REMOVE arm must name no spawn; arm was:\n{arm}"
        );
    }

    /// The prune runs on ONE long-lived task per frontier.
    ///
    /// Single-flight limb: a second `spawn_prune_task` over the same frontier
    /// spawns nothing and hands back `None`. Two loops over one index would each
    /// drain refs the other never sees settle, so the claim is what keeps the
    /// pass a single writer no matter how many wiring sites call this.
    ///
    /// The reclaim and never-blocks-the-op limbs of this contract arrive with the
    /// move of the trigger sites off the request path. They are deliberately not
    /// asserted yet: while the `OR_REMOVE` arm still runs a pass inline, a reclaim
    /// assertion here would be satisfied on the caller's own timeline and would
    /// keep passing even if the spawned task were never polled at all.
    #[tokio::test]
    async fn spawned_prune_task_reclaims_after_an_or_remove_and_never_blocks_the_op() {
        let (svc, factory, frontier) = make_service_with_frontier();

        let first = spawn_prune_task(
            Arc::clone(&frontier),
            Arc::clone(&factory),
            Arc::clone(&svc.key_writer),
        );
        assert!(
            first.is_some(),
            "the first spawn over a fresh frontier must claim the task"
        );

        let second = spawn_prune_task(
            Arc::clone(&frontier),
            Arc::clone(&factory),
            Arc::clone(&svc.key_writer),
        );
        assert!(
            second.is_none(),
            "a second spawn over the same frontier must claim nothing: one index \
             admits exactly one prune loop"
        );

        // Nothing here calls `request_prune`, so the task parks forever on its
        // first wake; aborting it keeps the runtime's shutdown free of a live
        // task holding the fixture's `Arc`s.
        if let Some(task) = first {
            task.abort();
        }
    }

    /// A `MapDataStore` that PANICS — rather than returning `Err` — on a durable
    /// write to an ARMED key.
    ///
    /// Arm-ability, and arming only AFTER seeding, is load-bearing for the same
    /// reason [`ArmableStore`] documents: a store that panicked from the start
    /// would never let a tombstone reach the index, so the pass under test would
    /// have nothing to drain and nothing to panic over. Delegating to an inner
    /// `ArmableStore` keeps retention — and therefore the rehydration path —
    /// identical to the store the other write-path tests here run against; only
    /// the arming verdict differs.
    ///
    /// Arming `add` alone covers BOTH durable write entry points, and that is
    /// why the witness-aware one is deliberately left defaulted: the in-place
    /// write path the prune takes reaches this store through
    /// `add_with_witness`, whose defaulted body drops the witness and calls
    /// `add` on this very type. A second override here would buy nothing and
    /// would put a second witness-aware implementor in the package, which the
    /// implementor-cascade guard above counts.
    ///
    /// That write-through is reached only AFTER the engine has already applied
    /// the prune to the resident slot, so the panic fires inside
    /// `update_in_place`, on the pass's own future — exactly the unwind the
    /// supervision under test has to survive — and it is a post-mutation panic,
    /// so the ref restored behind it settles as a match against nothing on the
    /// next pass rather than panicking again. The message names the key, so the
    /// captured panic row identifies it.
    #[derive(Default)]
    struct PanicOnWriteStore {
        inner: Arc<ArmableStore>,
        panic_keys: Mutex<HashSet<String>>,
    }

    impl PanicOnWriteStore {
        /// Panic on every subsequent durable write to `key`.
        fn arm_panic(&self, key: &str) {
            self.panic_keys.lock().insert(key.to_string());
        }

        /// Let writes to `key` through again.
        fn disarm_panic(&self, key: &str) {
            self.panic_keys.lock().remove(key);
        }

        fn panic_if_armed(&self, key: &str) {
            assert!(
                !self.panic_keys.lock().contains(key),
                "armed panic on the datastore write for {key}"
            );
        }
    }

    #[async_trait]
    impl MapDataStore for PanicOnWriteStore {
        async fn add(
            &self,
            map: &str,
            key: &str,
            value: &RecordValue,
            expiration_time: i64,
            now: i64,
        ) -> anyhow::Result<()> {
            self.panic_if_armed(key);
            self.inner.add(map, key, value, expiration_time, now).await
        }

        async fn add_backup(
            &self,
            map: &str,
            key: &str,
            value: &RecordValue,
            expiration_time: i64,
            now: i64,
        ) -> anyhow::Result<()> {
            self.inner
                .add_backup(map, key, value, expiration_time, now)
                .await
        }

        async fn remove(&self, map: &str, key: &str, now: i64) -> anyhow::Result<()> {
            self.inner.remove(map, key, now).await
        }

        async fn remove_backup(&self, map: &str, key: &str, now: i64) -> anyhow::Result<()> {
            self.inner.remove_backup(map, key, now).await
        }

        async fn load(&self, map: &str, key: &str) -> anyhow::Result<Option<RecordValue>> {
            self.inner.load(map, key).await
        }

        async fn load_all(
            &self,
            map: &str,
            keys: &[String],
        ) -> anyhow::Result<Vec<(String, RecordValue)>> {
            self.inner.load_all(map, keys).await
        }

        async fn remove_all(&self, map: &str, keys: &[String]) -> anyhow::Result<()> {
            self.inner.remove_all(map, keys).await
        }

        async fn enumerate_leaves(
            &self,
            map: &str,
            backup: bool,
            sink: &mut dyn LeafSink,
        ) -> anyhow::Result<()> {
            self.inner.enumerate_leaves(map, backup, sink).await
        }

        async fn scan_values(
            &self,
            map: &str,
            backup: bool,
            limit: u64,
        ) -> anyhow::Result<ScanBatch> {
            self.inner.scan_values(map, backup, limit).await
        }

        async fn scan_values_batched(
            &self,
            map: &str,
            backup: bool,
            cursor: ScanCursor,
            limit: u64,
        ) -> anyhow::Result<ScanBatch> {
            self.inner
                .scan_values_batched(map, backup, cursor, limit)
                .await
        }

        fn is_loadable(&self, key: &str) -> bool {
            self.inner.is_loadable(key)
        }

        fn pending_operation_count(&self) -> u64 {
            self.inner.pending_operation_count()
        }

        async fn soft_flush(&self) -> anyhow::Result<u64> {
            self.inner.soft_flush().await
        }

        async fn hard_flush(&self) -> anyhow::Result<()> {
            self.inner.hard_flush().await
        }

        async fn flush_key(
            &self,
            map: &str,
            key: &str,
            value: &RecordValue,
            backup: bool,
        ) -> anyhow::Result<()> {
            self.inner.flush_key(map, key, value, backup).await
        }

        fn reset(&self) {
            self.inner.reset();
        }

        fn is_null(&self) -> bool {
            self.inner.is_null()
        }
    }

    /// Add then remove one OR tag through the real write path, which stamps its
    /// tombstone into a fresh epoch of its own (the fixtures set width 1).
    async fn seed_or_tombstone(svc: &Arc<CrdtService>, key: &str, val: &str, tag: &str) {
        Arc::clone(svc)
            .oneshot(or_add_op("m", key, val, tag))
            .await
            .unwrap();
        Arc::clone(svc)
            .oneshot(or_remove_op("m", key, tag))
            .await
            .unwrap();
    }

    /// Wait, bounded, until `tag` is gone from `key`'s STORED tombstones.
    ///
    /// Bounded rather than a fixed sleep because the pass runs on a task this
    /// test does not drive: a sleep long enough to be reliable would also be
    /// long enough to hide a task that only starts late.
    async fn await_tombstone_gone(
        factory: &Arc<RecordStoreFactory>,
        key: &str,
        tag: &str,
        why: &str,
    ) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let (_, tombs) = read_or_map(factory, "m", key).await;
            if !tombs.contains(&tag.to_string()) {
                return;
            }
            assert!(std::time::Instant::now() < deadline, "{why}");
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }

    /// A pass panic must not end the prune task: the SAME task consumes the next
    /// permit and reclaims the next eligible epoch.
    ///
    /// RED at the pin. The pass panic in step 3 unwinds out of the spawned
    /// future and ends the task, so nothing is alive to consume the step-4
    /// permit and the step-5 wait times out. The guard's restore in step 3 is
    /// the one limb that already holds at the pin.
    ///
    /// The fixture panics in the datastore write-through, i.e. AFTER the engine
    /// has applied the prune to the resident slot. On the step-5 pass the
    /// restored `kpanic` ref therefore matches no tag, the store returns early
    /// before any write-through, and the ref settles without panicking a second
    /// time — which is why step 6 can require EXACTLY one panic row. Nothing is
    /// asserted about the `kpanic` ref beyond step 3.
    ///
    /// Runs on the default current-thread flavour: the `tracing` capture is
    /// thread-local, so a multi-thread runtime would observe an empty capture
    /// instead of the row this asserts on.
    #[tokio::test]
    async fn prune_pass_panic_is_caught_and_the_task_consumes_the_next_permit() {
        let (tracing_guard, sink) = capture_tracing_rows();
        let store = Arc::new(PanicOnWriteStore::default());
        let recorder = PrometheusBuilder::new().build_recorder();
        let render = recorder.handle();
        // Bind the recorder BEFORE the frontier resolves a single handle: one
        // resolved first would bind to a no-op for its whole lifetime.
        let (svc, factory, frontier) = metrics::with_local_recorder(&recorder, || {
            let data_store: Arc<dyn MapDataStore> = store.clone();
            make_service_with_frontier_and_store(data_store, Vec::new())
        });

        let task = spawn_prune_task(
            Arc::clone(&frontier),
            Arc::clone(&factory),
            Arc::clone(&svc.key_writer),
        )
        .expect("the first spawn over a fresh frontier must claim the task");

        // Step 2 — seed `kpanic`'s tombstone into epoch 1 and a pin into epoch 2,
        // then open the two gates exactly as `ac4_prune_wired_into_or_write_path`
        // does: LWM 2 is strictly past epoch 1, so epoch 1 alone is eligible.
        seed_or_tombstone(&svc, "kpanic", "v1", "TP").await;
        seed_or_tombstone(&svc, "kpin", "v2", "TPIN").await;
        assert_eq!(frontier.current_epoch(), 2, "epochs 1..=2 stamped");
        let c: String = "a5:alice|dev-1".into();
        frontier.set_delivered(ConnectionId(1), 100);
        assert!(frontier.confirm_apply_ack(&c, 2, ConnectionId(1)).await);
        assert_eq!(frontier.low_water_mark(), 2);
        // THE LAST SEEDING STEP, and no `.await` may sit between it and the
        // arming below. The fixture frontier is built with no store, so this
        // synchronous injection is the instant `kpanic` becomes eligible; a
        // yield here would let an earlier OR_REMOVE's own permit prune `kpanic`
        // through an UN-armed store, no panic would fire, and the wait below
        // would time out at the pin AND at the fix.
        frontier.set_durable_epoch_watermark(1000);

        // Step 3 — the pass that panics.
        store.arm_panic("kpanic");
        let before = frontier.index_conservation_snapshot();
        frontier.request_prune();

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let after = loop {
            let now = frontier.index_conservation_snapshot();
            if now.restored_refs_total > before.restored_refs_total {
                break now;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the panicking pass must re-index the refs it never settled"
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        };
        assert_eq!(
            after.indexed_refs, before.indexed_refs,
            "the drained ref is back in the index: drained, then re-indexed by \
             the guard as the pass unwound"
        );

        // Step 4 — a second eligible epoch, with the permit issued only once it
        // IS eligible, so the pass under test cannot be one an earlier
        // OR_REMOVE happened to trigger.
        store.disarm_panic("kpanic");
        seed_or_tombstone(&svc, "kok", "v3", "TOK").await;
        seed_or_tombstone(&svc, "kpin2", "v4", "TPIN2").await;
        assert_eq!(frontier.current_epoch(), 4, "epochs 3..=4 stamped");
        assert!(frontier.confirm_apply_ack(&c, 4, ConnectionId(1)).await);
        assert_eq!(frontier.low_water_mark(), 4);
        frontier.request_prune();

        // Step 5 — the red limb.
        await_tombstone_gone(
            &factory,
            "kok",
            "TOK",
            "the task must survive a pass panic and consume the next permit: \
             the epoch-3 tombstone is still stored",
        )
        .await;

        // Step 6 — the panic is attributed exactly once.
        drop(tracing_guard);
        let rows = sink.lock().unwrap().clone();
        let needle = concat!("prune pass ", "panicked;");
        let panic_rows: Vec<&str> = rows
            .iter()
            .map(String::as_str)
            .filter(|line| line.contains(needle))
            .collect();
        assert_eq!(
            panic_rows.len(),
            1,
            "a caught pass panic must emit exactly one attribution row; capture \
             was:\n{rows:#?}"
        );
        let row = panic_rows[0];
        assert!(
            row_u64(row, "restored") >= 1,
            "the panic row must carry the count of refs this pass re-indexed: {row:?}"
        );
        assert!(
            row.contains(concat!("armed panic on the datastore ", "write for kpanic")),
            "the panic row must carry the panic payload's own message: {row:?}"
        );

        // Step 7 — a CAUGHT pass panic does not move liveness: the task is
        // alive, and the row above is what makes the panic visible.
        let rendered = render.render();
        assert_eq!(
            rendered_series(&rendered, METRIC_PRUNE_TASK_ALIVE),
            Some("1"),
            "the task that survived the panic still holds the claim; render \
             was:\n{rendered}"
        );

        // Step 8.
        task.abort();
        let _ = task.await;
    }

    /// A prune task that EXITS releases its single-flight claim, so a later
    /// spawn can put a live task back over the same frontier.
    ///
    /// RED at the pin: nothing ever stores `false` back into the claim, so the
    /// second spawn returns `None` and reclamation is stopped for the life of
    /// the process. Awaiting the aborted handle is what makes the respawn
    /// assertion non-racy — tokio drops a cancelled task's future before the
    /// join resolves, so the exit lease has already run.
    #[tokio::test]
    async fn prune_task_claim_is_released_when_the_task_exits() {
        let recorder = PrometheusBuilder::new().build_recorder();
        let render = recorder.handle();
        let (svc, factory, frontier) =
            metrics::with_local_recorder(&recorder, make_service_with_frontier);

        // Step 0 — the series is PRESENT, at 0, before any spawn. Absent and
        // zero are the same picture to a human and different facts to a
        // sampler, which is why this reads the option rather than a number.
        let rendered = render.render();
        assert_eq!(
            rendered_series(&rendered, METRIC_PRUNE_TASK_ALIVE),
            Some("0"),
            "a frontier with no prune task must RENDER the absence, not omit \
             it; render was:\n{rendered}"
        );

        let first = spawn_prune_task(
            Arc::clone(&frontier),
            Arc::clone(&factory),
            Arc::clone(&svc.key_writer),
        )
        .expect("the first spawn over a fresh frontier must claim the task");
        let rendered = render.render();
        assert_eq!(
            rendered_series(&rendered, METRIC_PRUNE_TASK_ALIVE),
            Some("1"),
            "the spawn publishes liveness; render was:\n{rendered}"
        );

        first.abort();
        let joined = first.await;
        assert!(
            joined
                .expect_err("an aborted task never returns a value")
                .is_cancelled(),
            "the abort must resolve as a cancellation, which is what guarantees \
             the task's future has already been dropped"
        );
        let rendered = render.render();
        assert_eq!(
            rendered_series(&rendered, METRIC_PRUNE_TASK_ALIVE),
            Some("0"),
            "the exit publishes the absence BEFORE any respawn; render \
             was:\n{rendered}"
        );

        let second = spawn_prune_task(
            Arc::clone(&frontier),
            Arc::clone(&factory),
            Arc::clone(&svc.key_writer),
        );
        assert!(
            second.is_some(),
            "a task that has exited must have released its claim: otherwise the \
             frontier is claimed by nothing forever and reclamation stops \
             process-wide, silently"
        );
        let rendered = render.render();
        assert_eq!(
            rendered_series(&rendered, METRIC_PRUNE_TASK_ALIVE),
            Some("1"),
            "the respawn publishes liveness again; render was:\n{rendered}"
        );

        if let Some(task) = second {
            task.abort();
            let _ = task.await;
        }
    }

    /// No Cargo profile in this workspace sets a panic strategy, so the
    /// per-pass catch the prune task relies on is live in EVERY build.
    ///
    /// Under `panic = "abort"` a `catch_unwind` never runs its handler: the
    /// process dies at the panic instead. The supervision would then be
    /// silently inert — every test that exercises it runs under `test`, which
    /// always unwinds, so no behavioural test in this suite could notice. That
    /// is what makes this a structural pin rather than a redundancy.
    ///
    /// Comment lines are ignored: the release profile's own comment forbids the
    /// strategy in prose, and reading that as a setting would make the check
    /// permanently red.
    ///
    /// The manifest is only one of the routes to an aborting build; `RUSTFLAGS`
    /// and a cargo config's `[build] rustflags` never appear in it. The
    /// `compile_error!` below covers those routes, and it fires while building
    /// rather than while running, because a test binary that aborts on the
    /// first panic cannot report anything about itself.
    ///
    /// Its reach is this module: it lives under `#[cfg(test)]`, so it guards
    /// TEST compilations of this crate and says nothing about a production
    /// build configured to abort. Catching that is a policy-level guard on the
    /// build itself, not something a test can assert.
    #[cfg(panic = "abort")]
    compile_error!(
        "this crate is being compiled with an aborting panic strategy, under which \
         the catch at the prune pass boundary is inert"
    );

    #[test]
    fn no_cargo_profile_sets_panic_abort() {
        const MANIFEST: &str = include_str!("../../../../../Cargo.toml");

        for (index, line) in MANIFEST.lines().enumerate() {
            let code: String = line
                .split('#')
                .next()
                .unwrap_or("")
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect();
            assert!(
                !code.starts_with(concat!("panic", "=")),
                "line {} of the workspace manifest sets a panic strategy ({line:?}); \
                 a catch at the pass boundary is inert under an aborting profile",
                index + 1
            );
        }
    }

    /// Everything above the first column-0 `#[cfg(test)]` attribute — the half
    /// of a scanned file the compiler builds into a NON-test binary.
    ///
    /// The cut is what lets the needles below be counted exactly: this module
    /// sits under it, so a scan that spells the thing it counts cannot count
    /// itself.
    fn production_half(source: &str) -> &str {
        let cut = concat!("\n#[cfg(", "test)]\n");
        source.find(cut).map_or(source, |at| &source[..at])
    }

    /// One item's body, delimited by its own closing brace.
    ///
    /// The delimiter is the caller's, not a constant: a column-0 item closes at
    /// `\n}\n`, while a method inside an `impl` block closes at a column-4
    /// brace. Using the column-0 form on a method would swallow the rest of the
    /// impl block — thousands of lines — and turn every absence limb below
    /// spuriously red. The slice starts at the `impl`/`fn` line, so the item's
    /// own doc comment is deliberately OUTSIDE it.
    fn item_body<'a>(source: &'a str, anchor: &str, close: &str) -> &'a str {
        assert_eq!(
            source.matches(anchor).count(),
            1,
            "the anchor {anchor:?} must occur exactly once in the scanned source"
        );
        let from = source.find(anchor).expect("the anchor was just counted");
        let tail = &source[from..];
        let to = tail
            .find(close)
            .unwrap_or_else(|| panic!("{anchor:?} does not close at {close:?}"));
        &tail[..to]
    }

    /// Every body on the exit path names no construct that can panic: the exit
    /// lease's `Drop`, the release it calls, and the pass guard's own `Drop`.
    ///
    /// The guard's `Drop` belongs here because it runs during the very unwind
    /// the pass catch exists to survive — a fallible construct added there
    /// later would abort the process before the catch could see the panic.
    ///
    /// Each slice is first checked to carry the statement it is about, so a
    /// delimiter that drifted earlier cannot make these absence limbs vacuously
    /// green. Indexing is checked as the absence of any `[`, which is
    /// satisfiable because none of the three bodies indexes, slices or writes
    /// an array literal.
    fn assert_exit_path_is_panic_free(
        crdt_production: &str,
        impl_production: &str,
        lease_drop: &str,
    ) {
        let release_body = item_body(
            impl_production,
            concat!("fn release_prune", "_task("),
            "\n    }\n",
        );
        let guard_drop = item_body(
            crdt_production,
            concat!("impl Drop for ", "PrunePassGuard"),
            "\n}\n",
        );
        for (what, body, present) in [
            (
                "the lease's Drop",
                lease_drop,
                concat!("tracing::", "warn!"),
            ),
            (
                "the release",
                release_body,
                concat!("prune_task_", "claimed"),
            ),
            (
                "the pass guard's Drop",
                guard_drop,
                concat!("restore_tombstone", "_ref("),
            ),
        ] {
            assert!(
                body.contains(present),
                "the sliced body for {what} must contain {present:?}: {body:?}"
            );
            for needle in ["unwrap(", "expect(", "panic!", "debug_assert", "["] {
                assert!(
                    !body.contains(needle),
                    "{what} must name no `{needle}`: it runs on a drop path, \
                     where a panic during an unwind aborts the process"
                );
            }
        }
    }

    /// The supervision is WIRED, asserted over the sources rather than over a
    /// run: two of its six limbs cover properties no behavioural test in this
    /// suite can observe.
    ///
    /// (a)/(b) There is exactly ONE release site, and it is the exit lease's
    /// `Drop`. A second one — anywhere — could free the claim under a live task
    /// and let a second prune loop start beside the first, which is the state
    /// the claim exists to make unrepresentable.
    ///
    /// (c) The doc no longer says supervision is out of scope, and no longer
    /// says the claim is never released. A doc asserting a property the code
    /// does not have is the same defect in the other direction.
    ///
    /// (d) The spawn site reports a swallowed `None`. A `let _` binding there
    /// is how the only failure this call can report became invisible.
    ///
    /// (e) The exit path names no fallible-unwrap construct — the lease's
    /// `Drop`, the release it calls, and the pass guard's own `Drop`. Each runs
    /// inside a `Drop` that may itself be running during an unwind, where a
    /// panic aborts the process — "it happens not to panic today" is not the
    /// property worth having.
    ///
    /// (f) The spawn body neither requests a prune nor notifies the wake. A
    /// caught panic that re-triggered itself would turn a deterministically
    /// panicking pass into a hot loop. T-A cannot see this: with a
    /// post-mutation panic a re-triggered pass matches nothing and emits no
    /// second row, so this limb is the only mechanical check on it.
    #[test]
    fn prune_task_supervision_is_wired() {
        const SOURCE: &str = include_str!("crdt.rs");
        const IMPL_SOURCE: &str = include_str!("../../tombstone_frontier_impl.rs");
        const BIN_SOURCE: &str = include_str!("../../bin/topgun_server.rs");

        let release = concat!("release_prune", "_task(");
        let lease_impl = concat!("impl Drop for ", "PruneTaskLease");
        let spawn_fn = concat!("pub fn spawn_prune", "_task(");

        // (a) — one release call in this file's production half, inside the
        // lease's Drop.
        let crdt_production = production_half(SOURCE);
        assert_eq!(
            normalized(crdt_production).matches(release).count(),
            1,
            "exactly one release call belongs in this file: the exit lease's Drop"
        );
        let lease_drop = item_body(crdt_production, lease_impl, "\n}\n");
        assert!(
            lease_drop.contains(release),
            "the one release call must be the lease's Drop: {lease_drop:?}"
        );

        // (b) — and none at all on the frontier's own side beyond the
        // definition, which the `fn` prefix distinguishes.
        let impl_production = production_half(IMPL_SOURCE);
        assert_eq!(
            normalized(impl_production).matches(release).count(),
            1,
            "the frontier must define the release and never call it itself: a \
             release taken without the task exiting frees the claim under a \
             live pass"
        );

        // (c) — the retired claims are gone from the doc.
        let crdt_normalized = normalized(crdt_production);
        for retired in [
            concat!("Supervision is out ", "of scope"),
            concat!("The claim is never ", "released"),
        ] {
            assert!(
                !crdt_normalized.contains(retired),
                "the spawn doc still claims {retired:?}, which the supervision \
                 above makes false"
            );
        }

        // (d) — the spawn site checks its result and warns.
        let bin = normalized(production_half(BIN_SOURCE));
        assert!(
            !bin.contains(concat!("let _prune", "_task")),
            "a discarding binding is how the only failure this call can report \
             became invisible"
        );
        let call = concat!("spawn_prune", "_task(");
        assert_eq!(
            bin.matches(call).count(),
            1,
            "the bin spawns the prune task exactly once"
        );
        let after_call = &bin[bin.find(call).expect("the call was just counted")..];
        let window = &after_call[..after_call.find("let ").unwrap_or(after_call.len())];
        let none_at = window
            .find(".is_none()")
            .expect("the spawn result must be tested for `None` at the call site");
        let warn_at = window
            .find(concat!("tracing::", "warn!"))
            .expect("a swallowed `None` must be reported");
        assert!(
            none_at < warn_at,
            "the warning must be what the `None` test leads to: {window:?}"
        );

        // (e) — the exit path is panic-free, across all three of its bodies.
        assert_exit_path_is_panic_free(crdt_production, impl_production, lease_drop);

        // (f) — no self re-trigger after a caught panic.
        let spawn_body = item_body(crdt_production, spawn_fn, "\n}\n");
        assert!(
            spawn_body.contains(concat!("catch_", "unwind")),
            "the sliced spawn body must contain the pass catch: {spawn_body:?}"
        );
        for needle in [concat!("request_", "prune"), concat!("notify_", "one")] {
            assert!(
                !spawn_body.contains(needle),
                "the spawn body must not name `{needle}`: a pass that panics \
                 deterministically would become a hot loop"
            );
        }
    }

    /// Evicts one named key from its record store during the next rehydrating
    /// `load` of that key, once.
    ///
    /// This is the in-process lever that manufactures the "evicted between the
    /// rehydrating read and the in-place write" race. The eviction lands after
    /// the reader took its vacancy generation and before its insert, so the
    /// read is answered but never cached (TG-OR-007), and the caller's next
    /// in-place write finds no resident slot. With the default store that write
    /// materializes the key again and reclaims the tag; paired with a data store
    /// that cannot serve the second load, the closure never runs — the state the
    /// prune has to tell apart from "the tag was already gone".
    ///
    /// It runs from the data store, not from a mutation observer: observers may
    /// not call back into the store (see `MutationObserver`), and on the
    /// in-place write path they run under the key's cell lock, where an
    /// eviction of the same key would wait on the lock its own caller holds.
    /// The eviction happens outside every engine lock, and only once, so the
    /// write's own materializing load is served normally.
    #[derive(Default)]
    struct EvictOnRehydrate {
        armed: Mutex<Option<(Weak<dyn RecordStore>, String)>>,
    }

    impl EvictOnRehydrate {
        /// Held as a `Weak` so the data store, which the record store owns, does
        /// not keep that store alive through a cycle.
        fn arm(&self, store: &Arc<dyn RecordStore>, key: &str) {
            *self.armed.lock() = Some((Arc::downgrade(store), key.to_string()));
        }

        /// Evicts the armed key if `key` is it, then disarms. The guard is
        /// released before the eviction, which re-enters the observer chain.
        fn on_load(&self, key: &str) {
            let armed = {
                let mut armed = self.armed.lock();
                if armed.as_ref().is_some_and(|(_, k)| k == key) {
                    armed.take()
                } else {
                    None
                }
            };
            if let Some(store) = armed.and_then(|(weak, _)| weak.upgrade()) {
                store.evict(key, false);
            }
        }
    }

    /// Open both prune gates past epoch 1 only: the low-water mark STRICTLY past
    /// epoch 1 (cursor 2) and the byte-durability fence wide open. Both are
    /// injected, so which epoch drains is deterministic — no wall clock, no
    /// background sweeper.
    async fn open_prune_gates_past_epoch_one(frontier: &TombstoneFrontier) {
        let client: String = "a5:alice|dev-1".into();
        frontier.set_delivered(ConnectionId(1), 100);
        assert!(
            frontier
                .confirm_apply_ack(&client, 2, ConnectionId(1))
                .await
        );
        assert_eq!(frontier.low_water_mark(), 2);
        frontier.set_durable_epoch_watermark(1000);
    }

    /// The apply is PURE: no arm of it moves the tombstone-byte gauge, however
    /// many tombstones it appends or drops.
    ///
    /// This is the property a delta-fold recovery caller depends on — folding a
    /// durable delta must reconstruct state without perturbing a gauge whose
    /// post-recovery truth comes from the boot re-baseline instead.
    #[tokio::test]
    async fn or_apply_moves_no_tombstone_bytes_on_any_arm() {
        let tag = "T1";
        let entry = OrMapEntry {
            value: Value::Int(1),
            tag: tag.to_string(),
            timestamp: make_timestamp(),
        };

        let ((), delta) = with_isolated_gauge(async {
            let mut value = RecordValue::OrMap {
                records: Vec::new(),
                tombstones: Vec::new(),
            };
            // Every arm, in both its effective and its no-op disposition, so a
            // counter hidden on any one of them would show up here.
            assert!(
                apply_or_delta(
                    OrDelta::Add {
                        entry: entry.clone()
                    },
                    &mut value
                )
                .added,
                "a fresh add lands"
            );
            assert!(
                apply_or_delta(
                    OrDelta::Remove {
                        tag: tag.to_string()
                    },
                    &mut value
                )
                .new_tombstone,
                "the first remove appends a genuinely-new tombstone"
            );
            assert!(
                !apply_or_delta(
                    OrDelta::Remove {
                        tag: tag.to_string()
                    },
                    &mut value
                )
                .new_tombstone,
                "a re-issued remove appends nothing"
            );
            assert!(
                !apply_or_delta(OrDelta::Add { entry }, &mut value).added,
                "remove-wins suppresses the re-add"
            );
            assert_eq!(
                apply_or_delta(
                    OrDelta::Prune {
                        tags: vec![tag.to_string()]
                    },
                    &mut value
                )
                .pruned,
                1,
                "the prune drops the tombstone"
            );
            assert_eq!(
                apply_or_delta(
                    OrDelta::Prune {
                        tags: vec!["absent".to_string()]
                    },
                    &mut value
                )
                .pruned,
                0,
                "pruning an unknown tag drops nothing"
            );
        })
        .await;

        assert_eq!(
            delta, 0,
            "the apply is pure: appending and dropping tombstones must move no \
             gauge bytes, because every counter belongs to the caller at the \
             position that knows whether the durable write succeeded"
        );
    }

    /// Structural companion to `or_apply_moves_no_tombstone_bytes_on_any_arm`:
    /// the apply's own body does not so much as NAME a byte counter, so the
    /// purity contract cannot be re-broken by a counter on a path the behavioural
    /// test happens not to drive.
    #[test]
    fn or_apply_body_names_no_tombstone_byte_counter() {
        const SOURCE: &str = include_str!("crdt.rs");

        let start = SOURCE
            .find("pub(crate) fn apply_or_delta(")
            .expect("the apply seam is defined in this file");
        let tail = &SOURCE[start..];
        let end = tail
            .find("\n}\n")
            .expect("the apply's body closes at a column-0 brace");
        let body = &tail[..end];

        // Match the suffix rather than the three current counter names, so a
        // future `*_tombstone_bytes` counter cannot be added to the apply without
        // tripping this. Name-matching still cannot see a counter reached through
        // a differently-named local helper — the behavioural sibling
        // (`or_apply_moves_no_tombstone_bytes_on_any_arm`) is the primary guard
        // and catches that for every arm it drives; this one is the cheap
        // structural backstop.
        assert!(
            !body.contains("_tombstone_bytes"),
            "the apply must stay counter-free, found a `*_tombstone_bytes` call in its body"
        );
    }

    /// A genuinely-new tombstone is charged to the gauge even when the durable
    /// write that follows FAILS.
    ///
    /// The charge is atomic with the resident push, under the engine's per-key
    /// lock, so a failed write plus a client retry counts the tag exactly once:
    /// the retry finds the tag already resident and adds nothing. A charge moved
    /// after the write would be skipped here, and then skipped again on the
    /// retry, leaving the gauge short of a tombstone that IS resident — which
    /// later underflows on prune.
    #[tokio::test]
    async fn or_remove_charges_tombstone_bytes_when_the_durable_write_fails() {
        let store = Arc::new(ArmableStore::default());
        let (svc, factory, _frontier) = make_service_with_frontier_and_store(
            Arc::clone(&store) as Arc<dyn MapDataStore>,
            Vec::new(),
        );
        let tag = "T1";

        // Seed UN-ARMED, so the failure lands on the remove under test and not on
        // the record-creating write before it.
        Arc::clone(&svc)
            .oneshot(or_add_op("m", "k1", "v1", tag))
            .await
            .unwrap();
        store.reject_writes_to("k1");

        let (result, delta) = with_isolated_gauge(async {
            Arc::clone(&svc).oneshot(or_remove_op("m", "k1", tag)).await
        })
        .await;

        assert!(
            result.is_err(),
            "the armed store must fail the OR_REMOVE write-through"
        );
        assert_eq!(
            delta,
            tag.len() as u64,
            "the tombstone is charged inside the mutate closure, before the \
             durable write, so a failed write cannot skip the charge"
        );

        // The tag is resident-but-not-durable: the gauge counts it because the
        // retry will find it resident, not because it reached the backend.
        let (_, resident) = read_or_map(&factory, "m", "k1").await;
        assert!(
            resident.contains(&tag.to_string()),
            "the resident slot keeps the tombstone the closure appended"
        );
        let durable = store.durable("m", "k1");
        assert!(
            matches!(&durable, Some(RecordValue::OrMap { tombstones, .. }) if tombstones.is_empty()),
            "the rejected write must leave the durable copy without the \
             tombstone, got {durable:?}"
        );
    }

    /// A re-issued OR_REMOVE of the same tag charges the gauge ONCE.
    ///
    /// The charge is gated on the apply's "this tombstone is genuinely new"
    /// report; charging per remove instead would inflate the gauge without bound
    /// under client retries, and the tombstone byte-slope hard gate (TG-OR-004)
    /// reads that number.
    #[tokio::test]
    async fn duplicate_or_remove_charges_tombstone_bytes_once() {
        let store = Arc::new(ArmableStore::default());
        let (svc, factory, _frontier) = make_service_with_frontier_and_store(
            Arc::clone(&store) as Arc<dyn MapDataStore>,
            Vec::new(),
        );
        let tag = "T1";

        Arc::clone(&svc)
            .oneshot(or_add_op("m", "k1", "v1", tag))
            .await
            .unwrap();

        let ((), delta) = with_isolated_gauge(async {
            for _ in 0..2 {
                Arc::clone(&svc)
                    .oneshot(or_remove_op("m", "k1", tag))
                    .await
                    .unwrap();
            }
        })
        .await;

        assert_eq!(
            delta,
            tag.len() as u64,
            "two removes of the same tag are one tombstone, so they are one charge"
        );
        // Pins the charge to the tombstone set, so a gauge delta cannot be
        // explained by a second tombstone having really been appended.
        let (_, resident) = read_or_map(&factory, "m", "k1").await;
        assert_eq!(
            resident,
            vec![tag.to_string()],
            "the duplicate remove must not duplicate the tombstone"
        );
    }

    /// A prune whose durable write FAILS leaves the gauge exactly where it was.
    ///
    /// The decrement fires only in the post-write arm: the gauge tracks bytes
    /// that are actually still out there, not bytes removed from an in-memory
    /// copy. Decrementing inside the mutate closure would credit back a tag that
    /// is still durable, and the frontier ref is re-indexed for retry precisely
    /// because the reclaim has NOT happened yet.
    #[tokio::test]
    async fn prune_leaves_tombstone_bytes_unmoved_when_the_durable_write_fails() {
        let store = Arc::new(ArmableStore::default());
        let (svc, factory, frontier) = make_service_with_frontier_and_store(
            Arc::clone(&store) as Arc<dyn MapDataStore>,
            Vec::new(),
        );
        let (t1, t2) = ("T1", "T2");

        let ((), delta) = with_isolated_gauge(async {
            // Seed un-armed: k1's tombstone lands in epoch 1 and its frontier ref
            // is indexed, which is what gives the sweep something to drain.
            for (key, val, tag) in [("k1", "v1", t1), ("k2", "v2", t2)] {
                Arc::clone(&svc)
                    .oneshot(or_add_op("m", key, val, tag))
                    .await
                    .unwrap();
                Arc::clone(&svc)
                    .oneshot(or_remove_op("m", key, tag))
                    .await
                    .unwrap();
            }
            assert_eq!(frontier.current_epoch(), 2, "epochs 1..=2 stamped");
            open_prune_gates_past_epoch_one(&frontier).await;

            // Only NOW arm the rejection, so the drain finds epoch 1's ref and the
            // prune's own write is the one that fails.
            store.reject_writes_to("k1");
            prune_epoch_tombstones(&frontier, &factory, &svc.key_writer).await;

            // Attribution: the closure ran and the in-memory copy lost the tag,
            // while the durable copy still holds it. That divergence is exactly
            // why the decrement may not fire from inside the closure.
            let (_, resident) = read_or_map(&factory, "m", "k1").await;
            assert!(
                !resident.contains(&t1.to_string()),
                "the prune closure ran and dropped the tag from the resident slot"
            );
            let durable = store.durable("m", "k1");
            assert!(
                matches!(&durable, Some(RecordValue::OrMap { tombstones, .. }) if tombstones.contains(&t1.to_string())),
                "the rejected write must leave the tag durable, got {durable:?}"
            );
        })
        .await;

        assert_eq!(
            delta,
            (t1.len() + t2.len()) as u64,
            "both OR_REMOVE charges stand and the failed prune returns nothing: \
             a decrement from inside the mutate closure would credit back bytes \
             that are still durable"
        );
    }

    /// Evicted between the rehydrating read and the in-place write: the write
    /// materializes the key from the data store and reclaims the tag durably at
    /// once (TG-OR-007), so the ref is consumed and nothing is re-indexed.
    #[tokio::test]
    async fn prune_reclaims_a_key_evicted_between_its_read_and_its_write() {
        let store = Arc::new(ArmableStore::default());
        let (svc, factory, frontier) = make_service_with_frontier_and_store(
            Arc::clone(&store) as Arc<dyn MapDataStore>,
            Vec::new(),
        );
        let (t1, t2) = ("T1", "T2");

        for (key, val, tag) in [("k1", "v1", t1), ("k2", "v2", t2)] {
            Arc::clone(&svc)
                .oneshot(or_add_op("m", key, val, tag))
                .await
                .unwrap();
            Arc::clone(&svc)
                .oneshot(or_remove_op("m", key, tag))
                .await
                .unwrap();
        }
        open_prune_gates_past_epoch_one(&frontier).await;

        // Model the race: drop k1 from memory (its durable tombstone survives),
        // then evict it again during the sweep's rehydrating read, so the read
        // is answered but not cached and the in-place write finds no resident
        // slot.
        let k1_store = factory.get_or_create("m", hash_to_partition("k1"));
        assert!(
            k1_store.evict("k1", false).is_some(),
            "precondition: k1 is resident before the modelled eviction"
        );
        store.evictor.arm(&k1_store, "k1");

        prune_epoch_tombstones(&frontier, &factory, &svc.key_writer).await;

        let durable = store.durable("m", "k1");
        assert!(
            matches!(&durable, Some(RecordValue::OrMap { tombstones, .. }) if !tombstones.contains(&t1.to_string())),
            "the materializing write must reclaim the tag durably, got {durable:?}"
        );
        let retryable = frontier.drain_prunable_tombstones();
        assert!(
            !retryable.iter().any(|(_, r)| r.key == "k1"),
            "a reclaimed tombstone must not be re-indexed, got {retryable:?}"
        );
    }

    /// `RestoredEvicted` is reachable only through a store whose in-place write
    /// cannot materialize the key: here the data store serves the prune's
    /// rehydrating read and then answers absent, so after the eviction the
    /// closure never runs while the tombstone is still durable. The ref is
    /// re-indexed for a later sweep, and the pass still settles through exactly
    /// one of its seven exits.
    #[test]
    fn prune_restores_the_ref_when_the_store_cannot_materialize_the_evicted_key() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime");
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        let t1 = "T1";

        // Built inside the recorder binding, so the prune record emits.
        let (store, frontier) = metrics::with_local_recorder(&recorder, || {
            rt.block_on(async {
                let store = Arc::new(ArmableStore::default());
                let (svc, factory, frontier) = make_service_with_frontier_and_store(
                    Arc::clone(&store) as Arc<dyn MapDataStore>,
                    Vec::new(),
                );
                // k2 owns epoch 2, which the gates below keep pinned, so only
                // k1's epoch drains.
                for (key, val, tag) in [("k1", "v1", t1), ("k2", "v2", "T2")] {
                    Arc::clone(&svc)
                        .oneshot(or_add_op("m", key, val, tag))
                        .await
                        .unwrap();
                    Arc::clone(&svc)
                        .oneshot(or_remove_op("m", key, tag))
                        .await
                        .unwrap();
                }
                open_prune_gates_past_epoch_one(&frontier).await;

                let k1_store = factory.get_or_create("m", hash_to_partition("k1"));
                assert!(
                    k1_store.evict("k1", false).is_some(),
                    "precondition: k1 is resident before the modelled eviction"
                );
                store.evictor.arm(&k1_store, "k1");
                store.answer_absent_after_one_load("k1");

                prune_epoch_tombstones(&frontier, &factory, &svc.key_writer).await;
                (store, frontier)
            })
        });
        let rendered = handle.render();

        let durable = store.durable("m", "k1");
        assert!(
            matches!(&durable, Some(RecordValue::OrMap { tombstones, .. }) if tombstones.contains(&t1.to_string())),
            "nothing was reclaimed: the tag is still durable, got {durable:?}"
        );
        let retryable = frontier.drain_prunable_tombstones();
        assert!(
            retryable
                .iter()
                .any(|(epoch, r)| *epoch == 1 && r.key == "k1" && r.tag == t1),
            "an un-reclaimed tombstone must be re-indexed for a later sweep, got {retryable:?}"
        );

        let considered = rendered_counter(&rendered, METRIC_PRUNE_CONSIDERED_TOTAL);
        let restored_evicted = rendered_counter(&rendered, METRIC_PRUNE_RESTORED_EVICTED_TOTAL);
        assert_eq!(
            restored_evicted, 1,
            "the ref leaves through RestoredEvicted; render was:\n{rendered}"
        );
        assert_eq!(
            considered,
            rendered_counter(&rendered, METRIC_PRUNE_DROPPED_TOTAL)
                + rendered_counter(&rendered, METRIC_PRUNE_MATCHED_NOTHING_TOTAL)
                + rendered_counter(&rendered, METRIC_PRUNE_ABSENT_TOTAL)
                + rendered_counter(&rendered, METRIC_PRUNE_RESTORED_READ_ERROR_TOTAL)
                + restored_evicted
                + rendered_counter(&rendered, METRIC_PRUNE_RESTORED_WRITE_ERROR_TOTAL)
                + rendered_counter(&rendered, METRIC_PRUNE_RESTORED_CANCELLED_TOTAL),
            "every considered ref must leave through exactly one counted exit; \
             render was:\n{rendered}"
        );
        assert_eq!(considered, 1, "one ref considered; render was:\n{rendered}");
    }

    /// The tag was genuinely already gone: the closure RAN and removed nothing,
    /// so the frontier ref must NOT come back.
    ///
    /// There is nothing left to reclaim, and re-indexing a ref no write will ever
    /// satisfy livelocks the sweep on it forever. This is the disposition a bare
    /// `Ok(false)` cannot distinguish from the eviction race above, which is why
    /// the decision is keyed off a flag the closure itself sets.
    #[tokio::test]
    async fn prune_does_not_restore_the_tombstone_ref_when_the_tag_is_already_gone() {
        let store = Arc::new(ArmableStore::default());
        let (svc, factory, frontier) = make_service_with_frontier_and_store(
            Arc::clone(&store) as Arc<dyn MapDataStore>,
            Vec::new(),
        );

        // A resident record whose tombstone set does NOT hold the tag the sweep
        // is about to look for.
        Arc::clone(&svc)
            .oneshot(or_add_op("m", "k1", "v1", "T1"))
            .await
            .unwrap();
        let ghost = "GHOST";
        assert_eq!(
            frontier.stamp_tombstone("m", "k1", ghost),
            1,
            "the ghost ref sits in epoch 1"
        );
        // A second stamp advances the epoch counter so the low-water mark can sit
        // strictly past the ghost's epoch without making this one eligible too.
        assert_eq!(
            frontier.stamp_tombstone("m", "k2", "FILLER"),
            2,
            "the filler pins epoch 2"
        );
        open_prune_gates_past_epoch_one(&frontier).await;

        prune_epoch_tombstones(&frontier, &factory, &svc.key_writer).await;

        let retryable = frontier.drain_prunable_tombstones();
        assert!(
            !retryable.iter().any(|(_, r)| r.tag == ghost),
            "a tag that is already gone must not be re-indexed, or the sweep \
             livelocks on it, got {retryable:?}"
        );
    }

    /// A prune that upgrades a legacy-shaped slot but drops no tag still owes —
    /// and performs — a durable write.
    ///
    /// A shape change IS a value change. Reporting "unchanged" would leave the
    /// resident slot upgraded with a stale cost while the durable record kept the
    /// legacy shape until some unrelated later write happened to re-persist it.
    /// The gauge stays put, because nothing was pruned.
    #[tokio::test]
    async fn prune_persists_a_legacy_shape_upgrade_with_nothing_pruned() {
        let store = Arc::new(ArmableStore::default());
        let (svc, factory, frontier) = make_service_with_frontier_and_store(
            Arc::clone(&store) as Arc<dyn MapDataStore>,
            Vec::new(),
        );

        // Durable-only, in the pre-OrMap shape an older server wrote, holding a
        // tombstone the sweep is NOT asked to drop.
        store.seed_durable(
            "m",
            "k1",
            RecordValue::OrTombstones {
                tags: vec!["OLD".to_string()],
            },
        );
        let ghost = "GHOST";
        assert_eq!(
            frontier.stamp_tombstone("m", "k1", ghost),
            1,
            "the ghost ref sits in epoch 1"
        );
        assert_eq!(
            frontier.stamp_tombstone("m", "k2", "FILLER"),
            2,
            "the filler pins epoch 2"
        );
        open_prune_gates_past_epoch_one(&frontier).await;

        let ((), delta) = with_isolated_gauge(async {
            prune_epoch_tombstones(&frontier, &factory, &svc.key_writer).await;
        })
        .await;

        let durable = store.durable("m", "k1");
        assert!(
            matches!(
                &durable,
                Some(RecordValue::OrMap { records, tombstones })
                    if records.is_empty() && tombstones == &vec!["OLD".to_string()]
            ),
            "the shape upgrade must reach the durable record, got {durable:?}"
        );
        let (_, resident) = read_or_map(&factory, "m", "k1").await;
        assert_eq!(
            resident,
            vec!["OLD".to_string()],
            "the upgrade preserves the legacy tombstone set"
        );
        assert_eq!(
            delta, 0,
            "no tag was pruned, so a normalize-only write moves no gauge bytes"
        );
    }

    // -----------------------------------------------------------------------
    // The prune exit ledger: exhaustiveness (TG-OR-006) and instrument
    // neutrality
    // -----------------------------------------------------------------------

    /// The fixture the six-exit workload runs against.
    ///
    /// Kept as a value the caller constructs separately from the workload
    /// because WHERE the frontier is built relative to a bound metrics recorder
    /// is what decides whether its prune record can emit at all: the recorder
    /// resolves every handle once, at construction, and a handle resolved before
    /// a recorder is bound stays a no-op for its whole lifetime.
    struct SixExitFixture {
        store: Arc<ArmableStore>,
        svc: Arc<CrdtService>,
        factory: Arc<RecordStoreFactory>,
        frontier: Arc<TombstoneFrontier>,
    }

    /// What one six-exit prune pass leaves behind, in terms only the prune's own
    /// behaviour can move.
    ///
    /// This is the armed-vs-disarmed comparison's payload: it has to be readable
    /// without any metrics surface at all, because the disarmed arm has none.
    #[derive(Debug, PartialEq, Eq)]
    struct PruneWorkloadOutcome {
        /// Durable tombstone set per workload key, in a fixed order.
        durable_tombstones: Vec<(String, Option<Vec<String>>)>,
        /// Seeded tags the pass actually reclaimed from durable storage.
        dropped_observed: u64,
        /// Refs the pass handed back to the frontier for a later sweep.
        restored_refs: Vec<(Epoch, String, String)>,
    }

    /// The handles the ledger test needs once the workload has returned.
    ///
    /// A struct SEPARATE from [`PruneWorkloadOutcome`], and one that derives
    /// NOTHING. It exists because the ledger test has to keep driving the same
    /// frontier after the shared workload is done — stamping a further pin and
    /// running a second, cancelled pass — and the workload consumes its fixture.
    ///
    /// Keeping the handles out of the outcome is what preserves the
    /// armed-vs-disarmed equality: that comparison is over reclaim behaviour,
    /// and a struct carrying `Arc`s would drag pointer identity into it. A
    /// future reader must not re-add `Debug` here either — `#[derive(Debug)]`
    /// over an `Arc<T>` requires `T: Debug`, and neither [`CrdtService`] nor the
    /// test-local [`ArmableStore`] has one, so the derive would not compile.
    struct SixExitHandles {
        svc: Arc<CrdtService>,
        factory: Arc<RecordStoreFactory>,
        frontier: Arc<TombstoneFrontier>,
        store: Arc<ArmableStore>,
    }

    fn build_six_exit_fixture() -> SixExitFixture {
        let store = Arc::new(ArmableStore::default());
        let (svc, factory, frontier) = make_service_with_frontier_and_store(
            Arc::clone(&store) as Arc<dyn MapDataStore>,
            Vec::new(),
        );
        SixExitFixture {
            store,
            svc,
            factory,
            frontier,
        }
    }

    /// One fixed synthetic prune workload that drives the exits of
    /// `prune_epoch_tombstones` in a single non-empty pass: `Dropped` twice
    /// (`kdrop`, and `kevict`, whose eviction between the read and the write
    /// the in-place write now repairs by materializing the key), then
    /// matched-nothing, absent, read error and write error. `RestoredEvicted`
    /// is reachable only through a store that cannot materialize and is driven
    /// by `prune_restores_the_ref_when_the_store_cannot_materialize_the_evicted_key`;
    /// `RestoredCancelled` by the ledger test's second pass.
    ///
    /// Every exit has to actually fire. An exit nothing reaches contributes zero
    /// to both sides of the exhaustiveness identity, so its increment could be
    /// deleted and the identity would stay green — the identity would then be
    /// asserting nothing about that exit, which is exactly the failure mode a
    /// demonstrated RED is supposed to rule out.
    ///
    /// Epoch width is 1, so each stamp lands its ref in its own epoch and which
    /// epoch drains is decided by the injected low-water mark rather than by a
    /// clock. The failure modes are armed only after seeding, so every setup
    /// write succeeds and the prune's own read/write is the one that fails.
    #[allow(clippy::too_many_lines)]
    async fn run_six_exit_prune_workload(
        fixture: SixExitFixture,
    ) -> (PruneWorkloadOutcome, SixExitHandles) {
        let SixExitFixture {
            store,
            svc,
            factory,
            frontier,
        } = fixture;

        // Four keys that reach the prune holding a real durable tombstone.
        for (epoch, key, tag) in [
            (1, "kdrop", "TDROP"),
            (2, "kwrite", "TWRITE"),
            (3, "kevict", "TEVICT"),
            (4, "kread", "TREAD"),
        ] {
            Arc::clone(&svc)
                .oneshot(or_add_op("m", key, "v", tag))
                .await
                .unwrap();
            Arc::clone(&svc)
                .oneshot(or_remove_op("m", key, tag))
                .await
                .unwrap();
            assert_eq!(
                frontier.current_epoch(),
                epoch,
                "{key}'s tombstone must own epoch {epoch}"
            );
        }
        // A resident record whose tombstone set never held the tag the sweep is
        // about to look for: the mutate closure RUNS and matches nothing.
        Arc::clone(&svc)
            .oneshot(or_add_op("m", "kmatch", "v", "TLIVE"))
            .await
            .unwrap();
        assert_eq!(
            frontier.stamp_tombstone("m", "kmatch", "GHOST"),
            5,
            "the ghost ref sits in epoch 5"
        );
        // Neither resident nor durable: the read comes back `Ok(None)` and the
        // ref is consumed with no byte decrement.
        assert_eq!(
            frontier.stamp_tombstone("m", "kabsent", "TABSENT"),
            6,
            "the absent ref sits in epoch 6"
        );
        // Pins an epoch the drain must NOT take, so the low-water mark can sit
        // strictly past all six workload epochs without the fixture needing an
        // ack cursor beyond the epochs that exist.
        assert_eq!(
            frontier.stamp_tombstone("m", "kpinned", "TPINNED"),
            7,
            "the pin sits in epoch 7"
        );

        // One EXPLICIT pass while both gates are still shut, so the workload
        // owns an empty drain of its own. The empty-drain regime has to be
        // reachable from the workload itself rather than from whatever sweeps
        // happen to run as a side effect of the seeding writes: those sweeps are
        // a property of where the prune is TRIGGERED from, so a test that reads
        // its empty pass off them is pinning the trigger's shape instead of the
        // pass record's. Nothing is eligible here — no cursor has been
        // confirmed, so the low-water mark is still 0 — which is exactly what
        // makes this an empty drain.
        prune_epoch_tombstones(&frontier, &factory, &svc.key_writer).await;

        let client: String = "a5:alice|dev-1".into();
        frontier.set_delivered(ConnectionId(1), 100);
        assert!(
            frontier
                .confirm_apply_ack(&client, 7, ConnectionId(1))
                .await
        );
        assert_eq!(
            frontier.low_water_mark(),
            7,
            "epochs 1..=6 are eligible, epoch 7 is pinned"
        );
        frontier.set_durable_epoch_watermark(1000);

        // The prune's durable write fails.
        store.reject_writes_to("kwrite");
        // Evicted between the rehydrating read and the in-place write: the write
        // materializes the key from the data store and reclaims the tag.
        let kevict_store = factory.get_or_create("m", hash_to_partition("kevict"));
        assert!(
            kevict_store.evict("kevict", false).is_some(),
            "precondition: kevict is resident before the modelled eviction"
        );
        store.evictor.arm(&kevict_store, "kevict");
        // Non-resident with a failing backend read, so the rehydrating read is
        // the call that errors.
        let kread_store = factory.get_or_create("m", hash_to_partition("kread"));
        assert!(
            kread_store.evict("kread", false).is_some(),
            "precondition: kread is resident before it is dropped from memory"
        );
        store.reject_reads_to("kread");

        prune_epoch_tombstones(&frontier, &factory, &svc.key_writer).await;

        let seeded = [
            ("kdrop", "TDROP"),
            ("kwrite", "TWRITE"),
            ("kevict", "TEVICT"),
            ("kread", "TREAD"),
        ];
        let durable_tombstones = ["kdrop", "kwrite", "kevict", "kread", "kmatch", "kabsent"]
            .into_iter()
            .map(|key| {
                let tombstones = match store.durable("m", key) {
                    Some(RecordValue::OrMap { tombstones, .. }) => Some(tombstones),
                    Some(RecordValue::OrTombstones { tags }) => Some(tags),
                    Some(_) => Some(Vec::new()),
                    None => None,
                };
                (key.to_string(), tombstones)
            })
            .collect();
        let dropped_observed = seeded
            .into_iter()
            .filter(|(key, tag)| match store.durable("m", key) {
                Some(RecordValue::OrMap { tombstones, .. }) => {
                    !tombstones.contains(&(*tag).to_string())
                }
                _ => false,
            })
            .count() as u64;
        let mut restored_refs: Vec<(Epoch, String, String)> = frontier
            .drain_prunable_tombstones()
            .into_iter()
            .map(|(epoch, r)| (epoch, r.key, r.tag))
            .collect();
        restored_refs.sort();

        (
            PruneWorkloadOutcome {
                durable_tombstones,
                dropped_observed,
                restored_refs,
            },
            SixExitHandles {
                svc,
                factory,
                frontier,
                store,
            },
        )
    }

    /// Run the six-exit workload once and return its outcome, its isolated
    /// tombstone-gauge delta and the Prometheus render taken afterwards.
    ///
    /// `record_armed` decides only ONE thing: whether the fixture — and with it
    /// the frontier's prune-record handles — is constructed inside the recorder
    /// binding. Constructed inside, the record writes its series; constructed
    /// outside, every handle is a permanent no-op and the render carries no
    /// prune-record series at all, which is the observable surface of the
    /// disarmed `NullPruneRecorder`. Reproducing the disarmed surface this way
    /// rather than by flipping `TOPGUN_PRUNE_RECORD` is forced: the kill-switch
    /// is read from the process environment and this crate's
    /// `env_isolation_guard` forbids a test mutating it. That the env word
    /// selects `NullPruneRecorder`, and that `NullPruneRecorder` registers no
    /// series, are proven in `tombstone_frontier_impl.rs`; what is proven HERE
    /// is that the prune path's gauge movement and reclaim outcome do not depend
    /// on whether the record emits.
    ///
    /// `metrics::with_local_recorder` binds a THREAD-local recorder and takes a
    /// synchronous closure, so the workload is driven from a current-thread
    /// runtime inside the binding rather than from a `#[tokio::test]` body.
    fn six_exit_run(record_armed: bool) -> (PruneWorkloadOutcome, u64, String) {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime");
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        let prebuilt = if record_armed {
            None
        } else {
            Some(rt.block_on(async { build_six_exit_fixture() }))
        };
        let ((outcome, _handles), gauge_delta) = metrics::with_local_recorder(&recorder, || {
            let fixture =
                prebuilt.unwrap_or_else(|| rt.block_on(async { build_six_exit_fixture() }));
            rt.block_on(with_isolated_gauge(run_six_exit_prune_workload(fixture)))
        });
        (outcome, gauge_delta, handle.render())
    }

    /// The rendered value of the bare series `name`, if the render carries that
    /// line at all.
    ///
    /// `None` and `Some("0")` are different facts about a gauge — an
    /// unregistered series versus a registered one reading zero — so this
    /// returns the distinction rather than collapsing it into a number.
    fn rendered_series<'a>(rendered: &'a str, name: &str) -> Option<&'a str> {
        rendered
            .lines()
            .find_map(|line| Some(line.strip_prefix(name)?.strip_prefix(' ')?.trim()))
    }

    /// Read one counter's value out of a Prometheus render.
    fn rendered_counter(rendered: &str, name: &str) -> u64 {
        rendered
            .lines()
            // The space is the exposition format's name/value separator, so
            // requiring it stops a shorter name matching a longer one's prefix.
            .find_map(|line| line.strip_prefix(name)?.strip_prefix(' '))
            .unwrap_or_else(|| panic!("counter {name} is absent from the render:\n{rendered}"))
            .trim()
            .parse()
            .unwrap_or_else(|e| panic!("counter {name} did not render an integer: {e}"))
    }

    /// The prune ledger is exit-path EXHAUSTIVE over all SEVEN exits, and a pass
    /// is counted once per invocation rather than once per ref.
    ///
    /// Two identities, one test, because they share a workload and because the
    /// second is the premise the first's usefulness rests on:
    ///
    /// The exit identity, `considered == dropped + matched_nothing + absent +
    /// restored_read_error + restored_evicted + restored_write_error +
    /// restored_cancelled`. Every ref the loop examines leaves through exactly
    /// one counted exit, so a ref that quietly stops being accounted for — the
    /// mechanism behind a reclaim fraction that falls with no instrument able to
    /// say why — cannot hide.
    ///
    /// The pass identity, `passes == empty_drains + nonempty_drains`, pinned
    /// alongside `nonempty_drains == 2` and `empty_drains >= 1`: the pass
    /// observation is sited at the invocation and outside the loop body. Sited
    /// inside the loop it would read zero in a total stall — precisely the
    /// regime the record exists to describe — and would here count seven passes
    /// for two drains; made conditional on work, it would drop every empty pass.
    ///
    /// The SEVENTH exit is driven here and in none of the tests that share this
    /// workload, because a cancelled pass is not a property of the workload: it
    /// is a second pass this test alone runs, and a shared one would inject an
    /// injected-failure ref into every sibling's counts. The sequence is forced,
    /// and each step is what keeps the next one honest:
    ///
    /// 1. the shared one-pass workload runs unchanged, driving six exits;
    /// 2. its own outcome drain CONSUMES the two refs that pass restored, so
    ///    they cannot re-enter the second pass still carrying their injected
    ///    read / write failures, settle through the wrong exit, and break both
    ///    counts below;
    /// 3. a fresh pin is stamped at epoch 8 and acked, which makes `kpinned`'s
    ///    epoch 7 eligible while the new pin holds the frontier open — leaving
    ///    the index holding exactly ONE drainable ref;
    /// 4. that ref's per-key writer is held and a second pass runs under a short
    ///    timeout, so the pass drains its one ref, blocks on a writer it can
    ///    never take, and is cancelled on it: exactly one `RestoredCancelled`.
    ///
    /// `with_isolated_gauge` wraps the workload AND steps 3–4 rather than the
    /// workload alone. The cancelled pass is where the cancellation happens, so
    /// its tombstone-byte writes have to land in the same task-local sink
    /// instead of on the process gauge. The fixture is built INSIDE the recorder
    /// binding for the reason `six_exit_run` documents: a frontier constructed
    /// before the recorder is bound leaves every prune-record handle a permanent
    /// no-op, and the pins below are read by `rendered_counter`, which PANICS on
    /// an absent counter rather than reading a zero.
    #[test]
    #[allow(clippy::too_many_lines)]
    fn prune_exit_ledger_sums_to_considered() {
        const K_PINNED: &str = "kpinned";

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime");
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();

        let ((outcome, _handles), _gauge_delta) = metrics::with_local_recorder(&recorder, || {
            let fixture = rt.block_on(async { build_six_exit_fixture() });
            rt.block_on(with_isolated_gauge(async {
                let (outcome, handles) = run_six_exit_prune_workload(fixture).await;

                // Step 3. The fresh pin takes epoch 8, so a cursor at 8 licenses
                // epoch 7 — eligibility is STRICT — without licensing the pin
                // itself.
                assert_eq!(
                    handles.frontier.stamp_tombstone("m", "knew", "TNEW"),
                    8,
                    "the fresh pin sits in epoch 8"
                );
                let client: String = "a5:alice|dev-1".into();
                assert!(
                    handles
                        .frontier
                        .confirm_apply_ack(&client, 8, ConnectionId(1))
                        .await
                );
                assert_eq!(
                    handles.frontier.low_water_mark(),
                    8,
                    "epoch 7 is eligible now and epoch 8 is pinned"
                );

                // Step 4. The pass takes the per-key writer for every ref it
                // works on, so holding this one blocks it on its only ref and
                // the budget always elapses — a deterministic stop, not a race.
                let held = handles.svc.key_writer.acquire("m", K_PINNED).await;
                let cancelled = tokio::time::timeout(
                    std::time::Duration::from_millis(50),
                    prune_epoch_tombstones(
                        &handles.frontier,
                        &handles.factory,
                        &handles.svc.key_writer,
                    ),
                )
                .await;
                assert!(
                    cancelled.is_err(),
                    "the second pass cannot pass the held writer, so it must be \
                     cancelled"
                );
                drop(held);

                // WHERE the pass stopped is what decides which exit its one ref
                // takes, and this is the witness for it: the pinned ref names no
                // durable record at all, so a pass that had got as far as the
                // rehydrating read would have settled it through `AbsentKey` and
                // the seventh exit would never have fired. Blocked at the writer,
                // strictly ahead of that read, the cancelled exit is the only one
                // it can take.
                assert!(
                    handles.store.durable("m", K_PINNED).is_none(),
                    "the pinned ref names no durable record, so the exit it takes \
                     is decided by where the pass stopped"
                );

                (outcome, handles)
            }))
        });
        let rendered = handle.render();

        assert_eq!(
            outcome.dropped_observed, 2,
            "two seeded tags are reclaimed durably (kdrop, and kevict through the \
             materializing write); the read- and write-failure exits leave theirs"
        );

        let considered = rendered_counter(&rendered, METRIC_PRUNE_CONSIDERED_TOTAL);
        let dropped = rendered_counter(&rendered, METRIC_PRUNE_DROPPED_TOTAL);
        let matched_nothing = rendered_counter(&rendered, METRIC_PRUNE_MATCHED_NOTHING_TOTAL);
        let absent = rendered_counter(&rendered, METRIC_PRUNE_ABSENT_TOTAL);
        let restored_read_error =
            rendered_counter(&rendered, METRIC_PRUNE_RESTORED_READ_ERROR_TOTAL);
        let restored_evicted = rendered_counter(&rendered, METRIC_PRUNE_RESTORED_EVICTED_TOTAL);
        let restored_write_error =
            rendered_counter(&rendered, METRIC_PRUNE_RESTORED_WRITE_ERROR_TOTAL);
        let restored_cancelled = rendered_counter(&rendered, METRIC_PRUNE_RESTORED_CANCELLED_TOTAL);

        // Each exit fired the pinned number of times. Asserted before the sum so
        // a workload that stopped reaching an exit fails HERE, loudly, instead of
        // leaving the identity below vacuously true for that exit. `Dropped`
        // fires twice (the evicted key is reclaimed by the materializing write);
        // `RestoredEvicted` fires nowhere in this workload — it is driven by
        // `prune_restores_the_ref_when_the_store_cannot_materialize_the_evicted_key`,
        // which checks the same identity over its own pass.
        for (name, observed, expected) in [
            (METRIC_PRUNE_DROPPED_TOTAL, dropped, 2),
            (METRIC_PRUNE_MATCHED_NOTHING_TOTAL, matched_nothing, 1),
            (METRIC_PRUNE_ABSENT_TOTAL, absent, 1),
            (
                METRIC_PRUNE_RESTORED_READ_ERROR_TOTAL,
                restored_read_error,
                1,
            ),
            (METRIC_PRUNE_RESTORED_EVICTED_TOTAL, restored_evicted, 0),
            (
                METRIC_PRUNE_RESTORED_WRITE_ERROR_TOTAL,
                restored_write_error,
                1,
            ),
            (METRIC_PRUNE_RESTORED_CANCELLED_TOTAL, restored_cancelled, 1),
        ] {
            assert_eq!(
                observed, expected,
                "the workload must drive {name} exactly {expected} time(s), or the \
                 exhaustiveness identity asserts nothing about that exit; \
                 render was:\n{rendered}"
            );
        }

        assert_eq!(
            considered,
            dropped
                + matched_nothing
                + absent
                + restored_read_error
                + restored_evicted
                + restored_write_error
                + restored_cancelled,
            "every considered ref must leave through exactly one counted exit; \
             render was:\n{rendered}"
        );
        assert_eq!(considered, 7, "seven refs considered across the two passes");

        let passes = rendered_counter(&rendered, METRIC_PRUNE_PASSES_TOTAL);
        let empty_drains = rendered_counter(&rendered, METRIC_PRUNE_EMPTY_DRAINS_TOTAL);
        let nonempty_drains = rendered_counter(&rendered, METRIC_PRUNE_NONEMPTY_DRAINS_TOTAL);
        assert_eq!(
            passes,
            empty_drains + nonempty_drains,
            "every pass is counted exactly once and lands in exactly one of the \
             two drain buckets; render was:\n{rendered}"
        );
        assert_eq!(
            nonempty_drains, 2,
            "the pass observation is sited at the invocation, so each drain that \
             took work counts ONCE — seven here would mean it had been moved \
             into the per-ref loop; render was:\n{rendered}"
        );
        assert!(
            empty_drains >= 1,
            "the workload runs an explicit pass while its gates are shut, and \
             such empty passes must be counted too — a pass increment inside the \
             loop body would count none of them; render was:\n{rendered}"
        );
        assert_eq!(
            rendered_counter(&rendered, METRIC_PRUNE_EPOCHS_DRAINED_TOTAL),
            7,
            "one epoch per ref at epoch width 1; render was:\n{rendered}"
        );
    }

    /// The guard's `Drop` — the one emission site a cancelled pass ever reaches
    /// — is panic-free and moves no tombstone bytes.
    ///
    /// `TG-OR-004`: this `Drop` runs on a path a caller reaches by GIVING UP on
    /// the pass, and a panic inside a `drop` during an unwind aborts the
    /// process. "It happens not to panic today" is therefore not the property
    /// worth having — the body must name no fallible-unwrap construct at all.
    /// It must equally move no bytes: a restored ref freed nothing, and the
    /// gauge decrement belongs behind a durable write that succeeded.
    ///
    /// The single `restore_tombstone_ref` and single `observe_pass` call sites
    /// are the structural half. One restore path rather than two is what stops a
    /// second, divergent one being added later, and it is satisfiable only
    /// because the in-flight ref and the remainder are handed back in ONE
    /// chained iteration rather than in two loops.
    ///
    /// The slice is delimited exactly as this file's sibling structural tests
    /// delimit theirs — the anchor, then the first column-0 closing brace after
    /// it. Without a fixed delimiter "the body" is undefined: a scan running to
    /// the end of the file would read this very test and make the no-`unwrap(`
    /// and no-`panic!` limbs spuriously RED, while one stopping too early would
    /// make them vacuous.
    #[test]
    fn prune_guard_drop_is_panic_free_and_gauge_neutral() {
        const SOURCE: &str = include_str!("crdt.rs");

        let start = SOURCE
            .find("impl Drop for PrunePassGuard")
            .expect("the guard's Drop is implemented in this file");
        let tail = &SOURCE[start..];
        let end = tail
            .find("\n}\n")
            .expect("the Drop impl closes at a column-0 brace");
        let body = &tail[..end];

        for needle in ["unwrap(", "expect(", "panic!"] {
            assert!(
                !body.contains(needle),
                "the guard's Drop must name no `{needle}`: it runs on the \
                 cancellation path, where a panic during an unwind aborts the \
                 process"
            );
        }
        assert!(
            !body.contains("_tombstone_bytes"),
            "the guard's Drop must move no tombstone bytes: a ref handed back \
             freed nothing, and the decrement belongs behind a successful \
             durable write"
        );
        assert_eq!(
            body.matches("observe_pass").count(),
            1,
            "the pass is observed exactly once per invocation, from the guard's \
             single emission site"
        );
        assert_eq!(
            body.matches("restore_tombstone_ref").count(),
            1,
            "exactly one restore call site: two would be two restore paths, and \
             a second path is how a divergent one gets added later"
        );
    }

    // -----------------------------------------------------------------------
    // A thread-local `tracing` capture: the settlement and pass rows have no
    // Prometheus mirror, so this is the only transport that can read them.
    // -----------------------------------------------------------------------

    /// Renders every field of a captured event as `name=value ` pairs, of the
    /// `network/device_identity.rs:397-429` shape. The visitor overrides no
    /// `record_u64` / `record_bool`, so a `u64` or `bool` field reaches
    /// `record_debug` exactly like every other non-`&str` field — Debug and
    /// Display agree for both, so the rendered digits/`true`/`false` are the
    /// same either way.
    struct RowFieldVisitor(String);

    impl tracing::field::Visit for RowFieldVisitor {
        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            use std::fmt::Write as _;
            let _ = write!(self.0, "{}={value} ", field.name());
        }

        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            use std::fmt::Write as _;
            let _ = write!(self.0, "{}={value:?} ", field.name());
        }
    }

    /// Sink for [`RowFieldVisitor`]: one rendered line PER CAPTURED EVENT, each
    /// prefixed with the emitting event's `target` (metadata, not a `Visit`
    /// field, so it is written directly rather than collected by the visitor) —
    /// a `Vec<String>`, never a single concatenated `String`. Every test below
    /// reads a SPECIFIC row's own fields off the capture, and a concatenated
    /// blob can only individuate rows by inventing a newline convention that
    /// any future field rendering a newline would silently break.
    #[derive(Clone)]
    struct RowCapture(Arc<std::sync::Mutex<Vec<String>>>);

    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for RowCapture {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            let mut v = RowFieldVisitor(format!(" target={} ", event.metadata().target()));
            event.record(&mut v);
            self.0.lock().unwrap().push(v.0);
        }
    }

    /// Bind a thread-local capture for the guard's lifetime. Every drive that
    /// reads rows off it must run on a current-thread runtime with the emission
    /// on the test's own thread: `metrics::with_local_recorder` and
    /// `tracing::subscriber::set_default` are both thread-local (the doc at
    /// `:6129` makes the same point for the metrics recorder), so a
    /// multi-thread runtime observes an empty capture instead of a violation.
    fn capture_tracing_rows() -> (
        tracing::subscriber::DefaultGuard,
        Arc<std::sync::Mutex<Vec<String>>>,
    ) {
        use tracing_subscriber::layer::SubscriberExt;
        let sink: Arc<std::sync::Mutex<Vec<String>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::registry().with(RowCapture(Arc::clone(&sink)));
        let guard = tracing::subscriber::set_default(subscriber);
        (guard, sink)
    }

    /// Whether `line` is a row on `target` carrying `kind = "kind"`.
    fn is_row(line: &str, target: &str, kind: &str) -> bool {
        line.contains(&format!(" target={target} ")) && line.contains(&format!(" kind={kind} "))
    }

    /// Read one `u64`-rendered field out of a captured row.
    fn row_u64(line: &str, name: &str) -> u64 {
        row_field(line, name)
            .parse()
            .unwrap_or_else(|e| panic!("field {name} did not render a u64 in {line:?}: {e}"))
    }

    /// Read one `bool`-rendered field out of a captured row.
    fn row_bool(line: &str, name: &str) -> bool {
        row_field(line, name)
            .parse()
            .unwrap_or_else(|e| panic!("field {name} did not render a bool in {line:?}: {e}"))
    }

    fn row_field<'a>(line: &'a str, name: &str) -> &'a str {
        let needle = format!(" {name}=");
        let start = line
            .find(&needle)
            .unwrap_or_else(|| panic!("field {name} is absent from row {line:?}"))
            + needle.len();
        line[start..].split(' ').next().unwrap_or("")
    }

    /// Split a capture into successive passes: every row strictly after the
    /// previous pass row up to and including its own pass row (Step 1's frozen
    /// "Pass individuation" rule — the pass row is always emitted last).
    fn split_into_passes(rows: &[String]) -> Vec<Vec<&str>> {
        let mut passes = Vec::new();
        let mut current = Vec::new();
        for line in rows {
            current.push(line.as_str());
            if is_row(
                line,
                "topgun_server::tombstone_frontier::residency",
                "prune_pass",
            ) {
                passes.push(std::mem::take(&mut current));
            }
        }
        passes
    }

    /// R2.1/R2.2/AC3: the seven-exit identity holds **per epoch**, exactly as it
    /// already holds per pass — read off the settlement row, the only transport
    /// `PruneEpochRecord` has (it carries no Prometheus series of its own).
    ///
    /// The six-exit fixture drains six epochs of width 1, one ref each, so
    /// every epoch's settlement row has `considered == 1` and exactly one of
    /// the exit terms `== 1` — a per-epoch identity that would read exactly
    /// as "true" on a settlement row wired to the wrong per-epoch counter (all
    /// zero on one side), which is why each epoch's OWN nonzero exit is also
    /// checked below rather than only the sum.
    ///
    /// The cancelled term is summed here too and reads 0 on every row: this
    /// workload's one pass runs to completion, so no ref leaves through it.
    /// Reading it anyway is what keeps the identity a SEVEN-term one — a term
    /// omitted from the sum is a term the identity stops being able to see, and
    /// the exit it names is exactly the one a cancelled pass depends on.
    #[tokio::test]
    async fn per_epoch_seven_exit_identity_holds_on_the_settlement_row() {
        let (guard, sink) = capture_tracing_rows();
        let fixture = build_six_exit_fixture();
        let (_outcome, _handles) = run_six_exit_prune_workload(fixture).await;
        drop(guard);
        let rows = sink.lock().unwrap().clone();

        // The settlement row carries no `kind` field, so filter on target alone
        // (`is_row` requires both target and kind).
        let settlement_rows: Vec<&str> = rows
            .iter()
            .map(String::as_str)
            .filter(|l| l.contains(" target=topgun_server::tombstone_frontier::settlement "))
            .collect();

        assert_eq!(
            settlement_rows.len(),
            6,
            "one settlement row per drained epoch, six epochs drained; \
             capture was:\n{rows:#?}"
        );

        let mut considered_sum = 0u64;
        let mut dropped_sum = 0u64;
        let mut restored_evicted_sum = 0u64;
        for row in &settlement_rows {
            let considered = row_u64(row, "considered");
            let dropped = row_u64(row, "dropped");
            let matched_nothing = row_u64(row, "matched_nothing");
            let absent = row_u64(row, "absent");
            let restored_read_error = row_u64(row, "restored_read_error");
            let restored_evicted = row_u64(row, "restored_evicted");
            let restored_write_error = row_u64(row, "restored_write_error");
            let restored_cancelled = row_u64(row, "restored_cancelled");
            assert_eq!(
                considered,
                dropped
                    + matched_nothing
                    + absent
                    + restored_read_error
                    + restored_evicted
                    + restored_write_error
                    + restored_cancelled,
                "the per-epoch seven-exit identity must hold on this settlement \
                 row: {row:?}"
            );
            assert_eq!(considered, 1, "one ref per epoch at epoch width 1: {row:?}");
            considered_sum += considered;
            dropped_sum += dropped;
            restored_evicted_sum += restored_evicted;
        }
        assert_eq!(considered_sum, 6, "six epochs, one ref each");
        assert_eq!(
            dropped_sum, 2,
            "two of the six epochs reclaim durably (kdrop, and kevict through the \
             materializing write); the other four leave their tag in place"
        );
        assert_eq!(
            restored_evicted_sum, 0,
            "the evicted key is reclaimed, not restored, with the default store"
        );
    }

    /// AC4a: the pass row fires on **every** `prune_epoch_tombstones`
    /// invocation, empty drains included, on the same residency target the
    /// entry/exit rows use, with `kind = "prune_pass"` and the right
    /// `considered` / `empty_drain` values (R2.5, R2.5a, R2.5b).
    ///
    /// The six-exit workload runs an explicit `prune_epoch_tombstones` pass
    /// while its prune gates are still shut, ahead of the draining call at the
    /// end (the same pass `prune_exit_ledger_sums_to_considered` pins with
    /// `empty_drains >= 1`), so a single run of it already exercises both an
    /// empty-drain pass and a draining one — no hand-rolled second invocation
    /// needed, and no exact pass-row count assumed.
    #[tokio::test]
    async fn pass_row_fires_on_every_invocation_including_an_empty_drain() {
        let (guard, sink) = capture_tracing_rows();
        let fixture = build_six_exit_fixture();
        let (_outcome, _handles) = run_six_exit_prune_workload(fixture).await;
        drop(guard);
        let rows = sink.lock().unwrap().clone();

        let pass_rows: Vec<&str> = rows
            .iter()
            .map(String::as_str)
            .filter(|l| {
                is_row(
                    l,
                    "topgun_server::tombstone_frontier::residency",
                    "prune_pass",
                )
            })
            .collect();

        assert!(
            pass_rows.len() >= 2,
            "the workload runs an explicit pass while its prune gates are shut, \
             so at least one pass row is expected on top of its own draining \
             call; capture was:\n{rows:#?}"
        );
        assert!(
            pass_rows
                .iter()
                .any(|r| row_u64(r, "considered") == 0 && row_bool(r, "empty_drain")),
            "at least one pass row must report an empty drain (considered=0, \
             empty_drain=true); capture was:\n{rows:#?}"
        );
        // The workload's own DRAINING `prune_epoch_tombstones` call is the LAST
        // invocation of the run — its shut-gate pass runs strictly before the
        // gates open — so it is deterministically the capture's last pass row.
        // Cited by name, not by line: a same-file line number is falsified by
        // any insertion above it without anything failing.
        let last = pass_rows.last().expect("at least one pass row exists");
        assert_eq!(
            row_u64(last, "considered"),
            6,
            "the workload's draining call is the last invocation and \
             drains all six seeded refs: {last:?}"
        );
        assert!(
            !row_bool(last, "empty_drain"),
            "the workload's draining call is a non-empty drain: {last:?}"
        );
    }

    /// Exactly one `::residency` emit site names this target in this file
    /// (AC4a's mechanical check, `rg -n 'target: "topgun_server::tombstone_\
    /// frontier::residency"' … crdt.rs`) — asserted here too, over this file's
    /// own source, so a second emit site fails the lib suite and not only the
    /// grep the human checklist runs separately.
    #[test]
    fn exactly_one_residency_emit_site_in_this_file() {
        const SOURCE: &str = include_str!("crdt.rs");
        let count = SOURCE
            .matches("target: \"topgun_server::tombstone_frontier::residency\"")
            .count();
        assert_eq!(
            count, 1,
            "AC4a requires exactly one ::residency emit site in this file, \
             beside the existing observe_pass call"
        );
    }

    /// Step 1's bridge identities, evaluated over a driven pass in this file's
    /// own fixture (the full D-T evaluation over `D5` is `sim/tombstone_gc_\
    /// proof.rs`'s job; this is the local confirmation that both terms are
    /// actually reachable together on the ONE `tracing` transport, which is
    /// what makes that later evaluation possible at all):
    ///
    /// - **I1**: `Σ_e R_obs(e) == K_p`, `R_obs` read off each pass's `epoch_\
    ///   exit` rows (`removed_refs_observed`), `K_p` off that SAME pass's pass
    ///   row (`considered`).
    /// - **I2**: `E_p == (Σ_e R_obs(e) == 0)`, `E_p` read off the same pass
    ///   row's `empty_drain`.
    ///
    /// Evaluated over EVERY pass the six-exit workload's run individuates —
    /// split by [`split_into_passes`]'s frozen rule, so each pass's own rows
    /// are summed separately rather than pooled across the whole capture. The
    /// workload runs an explicit `prune_epoch_tombstones` pass while its gates
    /// are shut, ahead of its draining call (see the pass-row test above), so a
    /// single run already produces both an empty-drain pass and a draining pass
    /// without a hand-rolled second invocation or an assumed exact pass count.
    #[tokio::test]
    async fn i1_i2_pass_identity_holds_on_an_empty_and_a_draining_pass() {
        let (guard, sink) = capture_tracing_rows();
        let fixture = build_six_exit_fixture();
        let (_outcome, _handles) = run_six_exit_prune_workload(fixture).await;
        drop(guard);
        let rows = sink.lock().unwrap().clone();

        let passes = split_into_passes(&rows);
        assert!(
            passes.len() >= 2,
            "the workload's explicit shut-gate pass must individuate a pass on \
             top of its own draining call; capture was:\n{rows:#?}"
        );

        let mut saw_empty = false;
        let mut saw_draining = false;
        for pass_rows in &passes {
            let pass_row = pass_rows
                .last()
                .expect("a pass's own rows always end with its pass row");
            assert!(
                is_row(
                    pass_row,
                    "topgun_server::tombstone_frontier::residency",
                    "prune_pass"
                ),
                "a pass's last row must be its own pass row: {pass_row:?}"
            );
            let k_p = row_u64(pass_row, "considered");
            let e_p = row_bool(pass_row, "empty_drain");

            let r_obs_sum: u64 = pass_rows
                .iter()
                .filter(|l| {
                    is_row(
                        l,
                        "topgun_server::tombstone_frontier::residency",
                        "epoch_exit",
                    )
                })
                .map(|l| row_u64(l, "removed_refs_observed"))
                .sum();

            assert_eq!(
                r_obs_sum, k_p,
                "I1: the sum of a pass's own epoch_exit removed_refs_observed \
                 must equal that SAME pass's considered; pass rows were:\n{pass_rows:?}"
            );
            assert_eq!(
                e_p,
                r_obs_sum == 0,
                "I2: empty_drain must equal (Σ removed_refs_observed == 0) for \
                 the same pass; pass rows were:\n{pass_rows:?}"
            );

            if e_p {
                saw_empty = true;
            } else {
                saw_draining = true;
            }
        }

        // Both regimes must actually appear, or I1/I2 above would hold
        // vacuously over only one of them.
        assert!(
            saw_empty,
            "at least one pass in the capture must be an empty drain; \
             capture was:\n{rows:#?}"
        );
        assert!(
            saw_draining,
            "at least one pass in the capture must be a draining pass; \
             capture was:\n{rows:#?}"
        );
        // The workload's own DRAINING `prune_epoch_tombstones` call is the last
        // invocation of the run, so it is deterministically the capture's last
        // pass. Cited by name, not by line, for the reason given at the sibling
        // assertion above.
        assert_eq!(
            row_u64(passes.last().unwrap().last().unwrap(), "considered"),
            6,
            "the last pass in the capture must be the workload's own draining \
             call over all six seeded refs"
        );
    }

    /// The prune record is INSTRUMENT-NEUTRAL: arming it moves no tombstone
    /// bytes and reclaims no differently.
    ///
    /// Behavioural limb: one fixed workload run twice, once with the record
    /// emitting and once with it inert, produces identical isolated gauge deltas
    /// and an identical reclaim outcome — same durable tombstone sets, same
    /// dropped count, same restored refs.
    ///
    /// Structural limb: the gauge decrement is where it was before the record
    /// existed. `prune_epoch_tombstones` still names exactly ONE
    /// `sub_tombstone_bytes` call, still in the post-write `Ok(_)` arm behind
    /// `dropped`; the recorder body names no tombstone-byte counter at all; and
    /// `apply_or_delta` stays counter-free. The behavioural limb cannot see a
    /// counter on a path it happens not to drive, which is what the structural
    /// one is for.
    #[test]
    fn prune_record_armed_disarmed_gauge_neutral() {
        const SOURCE: &str = include_str!("crdt.rs");
        const RECORDER_SOURCE: &str = include_str!("../../tombstone_frontier_impl.rs");

        let (armed_outcome, armed_delta, armed_render) = six_exit_run(true);
        let (disarmed_outcome, disarmed_delta, disarmed_render) = six_exit_run(false);

        // Arming witness: the two runs really did differ in whether the record
        // emitted, so the equalities below are not comparing a run against
        // itself.
        assert_eq!(
            rendered_counter(&armed_render, METRIC_PRUNE_CONSIDERED_TOTAL),
            6,
            "the armed run must write its prune-record series, populated; \
             render was:\n{armed_render}"
        );
        assert!(
            !disarmed_render.contains("topgun_or_prune_"),
            "the disarmed run must write no prune-record series at all; render \
             was:\n{disarmed_render}"
        );
        // The gauge comparison is made over the isolated SINK's delta, not over
        // either render: the sink deliberately replaces the process gauge for
        // the scope's duration, so `topgun_ormap_tombstone_bytes` is absent from
        // both renders by construction. That is what makes the delta exact
        // rather than a reading of whatever else the test binary was doing.
        assert_eq!(
            armed_delta, disarmed_delta,
            "arming the prune record must move no tombstone-gauge bytes"
        );
        assert_eq!(
            armed_outcome, disarmed_outcome,
            "arming the prune record must reclaim exactly what the disarmed run \
             reclaims, ref for ref"
        );
        assert_eq!(
            armed_outcome.dropped_observed, 2,
            "the comparison is over a run that actually reclaimed something"
        );

        let start = SOURCE
            .find("pub(crate) async fn prune_epoch_tombstones(")
            .expect("the prune loop is defined in this file");
        let tail = &SOURCE[start..];
        let end = tail
            .find("\n}\n")
            .expect("the prune body closes at a column-0 brace");
        let body = &tail[..end];

        assert_eq!(
            body.matches("sub_tombstone_bytes").count(),
            1,
            "the prune must name exactly one tombstone-byte decrement; a second \
             one double-counts a reclaim and a zeroth one strands the gauge"
        );
        let after_match = body
            .split_once("match result {")
            .expect("the prune dispatches on the write result")
            .1;
        let ok_arm = after_match
            .find("Ok(_) => {")
            .expect("the write result has a success arm");
        let err_arm = after_match
            .find("Err(e) => {")
            .expect("the write result has a failure arm");
        let call = after_match
            .find("sub_tombstone_bytes")
            .expect("the decrement is inside the result dispatch");
        assert!(
            ok_arm < call && call < err_arm,
            "the decrement must sit in the post-write success arm: the gauge \
             tracks bytes actually gone from storage, not bytes removed from an \
             in-memory copy"
        );
        let guard_to_call = after_match[ok_arm..call]
            .rsplit_once("if dropped {")
            .expect("the decrement sits behind the `dropped` guard")
            .1;
        assert!(
            !guard_to_call.contains(['{', '}', ';']),
            "the decrement must be the first statement of the `if dropped` \
             block, found intervening code: {guard_to_call:?}"
        );

        for anchor in [
            "impl MetricsPruneRecorder {",
            "impl PruneRecordObserver for MetricsPruneRecorder {",
        ] {
            let start = RECORDER_SOURCE
                .find(anchor)
                .unwrap_or_else(|| panic!("`{anchor}` is defined in tombstone_frontier_impl.rs"));
            let recorder_tail = &RECORDER_SOURCE[start..];
            let recorder_end = recorder_tail
                .find("\n}\n")
                .expect("the block closes at a column-0 brace");
            assert!(
                !recorder_tail[..recorder_end].contains("_tombstone_bytes"),
                "the prune recorder must name no tombstone-byte counter, found \
                 one in `{anchor}`"
            );
        }

        // Split needle: `the_or_algebra_has_exactly_one_implementation` counts
        // this literal package-wide to catch a second copy of the OR algebra, and
        // a scanning literal is not an implementation — writing it whole would
        // grow that guard's carve-out instead of leaving its baseline alone.
        let apply_start = SOURCE
            .find(concat!("pub(crate) fn ", "apply_or_delta("))
            .expect("the apply seam is defined in this file");
        let apply_tail = &SOURCE[apply_start..];
        let apply_end = apply_tail
            .find("\n}\n")
            .expect("the apply's body closes at a column-0 brace");
        assert!(
            !apply_tail[..apply_end].contains("_tombstone_bytes"),
            "the apply must stay counter-free: a delta-fold recovery caller \
             reconstructs state through it and must not perturb a gauge whose \
             post-recovery truth comes from the boot re-baseline"
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

        // The whole body runs against a private, zero-based gauge sink, so every
        // snapshot below is this test's own contribution and nothing else's. No
        // ambient OR_REMOVE traffic from parallel tests can reach it, which is
        // what lets the three measurement points be exact equalities.
        let map_name = "gauge_residency_map";
        let key = "item-1";
        let tag = "tag-gauge-residency";

        let ((), net_delta) = crate::storage::tombstone_gauge::with_isolated_gauge(async {
            svc.clone()
                .oneshot(or_add_op(map_name, key, "payload", tag))
                .await
                .expect("or_add must succeed");
            svc.clone()
                .oneshot(or_remove_op(map_name, key, tag))
                .await
                .expect("or_remove must succeed");

            let after_write = crate::storage::record::tombstone_bytes();
            assert_eq!(
                after_write,
                tag.len() as u64,
                "OR_REMOVE must charge the gauge exactly the tag's byte length"
            );

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
            assert_eq!(
                after_eviction, after_write,
                "evicting the record must not move the gauge — the accounting is \
                 residency-independent"
            );

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
            assert_eq!(
                after_rehydration, after_write,
                "rehydrating the record must not re-charge the gauge — a second \
                 charge here is the double-count regression this test pins"
            );
        })
        .await;

        assert_eq!(
            net_delta,
            tag.len() as u64,
            "net scoped delta across the write/evict/rehydrate cycle is one tag"
        );
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

    // N concurrent OR_ADDs on the SAME key must
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

    /// Bytes allocated by exactly the future `f` runs to completion.
    #[cfg(feature = "count-alloc")]
    async fn bytes_allocated_by<F: Future>(f: F) -> (F::Output, usize) {
        let before = stats_alloc::INSTRUMENTED_SYSTEM.stats().bytes_allocated;
        let out = f.await;
        let after = stats_alloc::INSTRUMENTED_SYSTEM.stats().bytes_allocated;
        (out, after - before)
    }

    /// Seeds `key` as a RESIDENT OR slot of `n` records (29-char distinct tags) plus
    /// one tombstone `tomb`, and stamps the tombstone's frontier ref. Returns the
    /// ref's epoch (the fixture runs one epoch per stamp).
    #[cfg(feature = "count-alloc")]
    async fn seed_prunable_slot(
        factory: &Arc<RecordStoreFactory>,
        frontier: &TombstoneFrontier,
        key: &str,
        n: usize,
        tomb: &str,
    ) -> Epoch {
        let ts = Timestamp {
            millis: 1_700_000_000_000,
            counter: 0,
            node_id: "node-a".to_string(),
        };
        let records: Vec<OrMapEntry> = (0..n)
            .map(|i| OrMapEntry {
                value: Value::Null,
                tag: format!("{i:020}:0:node-a"),
                timestamp: ts.clone(),
            })
            .collect();
        let store = factory.get_or_create("m", hash_to_partition(key));
        store
            .put(
                key,
                RecordValue::OrMap {
                    records,
                    tombstones: vec![tomb.to_string()],
                },
                ExpiryPolicy::NONE,
                CallerProvenance::CrdtMerge,
            )
            .await
            .unwrap();
        assert!(store.exists_in_memory(key), "the seeded slot is resident");
        frontier.stamp_tombstone("m", key, tomb)
    }

    /// Raises the low-water mark strictly past `epoch` and no further, so the next
    /// pass has exactly that epoch's ref to drain. The LWM is bounded by the
    /// current epoch, so a later epoch must already be stamped.
    async fn license_epoch(frontier: &TombstoneFrontier, epoch: Epoch) {
        let client: String = "a5:alice|dev-1".into();
        assert!(
            frontier
                .confirm_apply_ack(&client, epoch + 1, ConnectionId(1))
                .await
        );
        assert_eq!(frontier.low_water_mark(), epoch + 1);
    }

    /// One prune pass over a RESIDENT key allocates about one whole-record copy of
    /// its slot (the in-place write's returned record), not two: the residency
    /// probe must not clone the slot it only needs to know is present.
    ///
    /// Per slot size N, `P(N)` is the bytes one pass allocates and `C(N)` the bytes
    /// one engine `get` of the same slot allocates (one whole-record clone). The
    /// slopes `p` and `c` over N ∈ {1 000, 10 000} cancel every O(1) allocation of
    /// the pass (guard, ledger emission, metrics, boxes), so `p ≈ k·c` counts the
    /// whole-record copies the pass makes. Asserted: `p ≤ 1.2 c + 32` (one copy).
    /// Each measured pass must settle its ref `Dropped` and remove the tombstone, so
    /// an emptied drain cannot pass the bound vacuously.
    ///
    /// Every ref is stamped up front, followed by one record-less pin ref that is
    /// never licensed and never drained, and the low-water mark is raised exactly
    /// one epoch before each pass. The pin ref is load-bearing: the LWM cannot pass
    /// the current epoch, so without a later stamp the last measured ref could
    /// never be licensed; stepping one epoch per pass keeps each pass to its own
    /// single ref instead of letting the warm-up drain them all.
    ///
    /// Built on the `NullDataStore` fixture with no observers, so neither a
    /// write-behind copy nor a Merkle leaf hash enters the reading. The fixture is
    /// built inside the local recorder for the reason `six_exit_run` documents.
    ///
    /// A local allocation proof, not a CI guard: CI never enables `count-alloc`,
    /// and the counters are process-global, so it is meaningful only when run
    /// alone and single-threaded:
    /// `cargo test --release -p topgun-server --lib --features count-alloc -- --ignored --test-threads=1 count_alloc_`
    #[cfg(feature = "count-alloc")]
    #[test]
    #[ignore = "local allocation proof: run single-threaded under count-alloc"]
    #[allow(clippy::cast_precision_loss, clippy::too_many_lines)]
    fn count_alloc_prune_probe_resident() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime");
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();

        let readings: Vec<(usize, usize, usize)> = metrics::with_local_recorder(&recorder, || {
            rt.block_on(async {
                let (readings, _gauge) = with_isolated_gauge(async {
                    let (svc, factory, frontier) = make_service_with_frontier();
                    frontier.set_delivered(ConnectionId(1), 1_000_000);
                    frontier.set_durable_epoch_watermark(1_000_000);

                    let restored = |rendered: &str| {
                        [
                            METRIC_PRUNE_RESTORED_READ_ERROR_TOTAL,
                            METRIC_PRUNE_RESTORED_EVICTED_TOTAL,
                            METRIC_PRUNE_RESTORED_WRITE_ERROR_TOTAL,
                            METRIC_PRUNE_RESTORED_CANCELLED_TOTAL,
                        ]
                        .iter()
                        .map(|name| rendered_counter(rendered, name))
                        .sum::<u64>()
                    };

                    // One ref per pass, each in its own epoch, stamped up front;
                    // the trailing pin (no record behind it) keeps the last
                    // measured epoch licensable and is itself never drained.
                    let warm_epoch =
                        seed_prunable_slot(&factory, &frontier, "kwarm", 1_000, "TWARM").await;
                    let mut measured = Vec::new();
                    for n in [1_000_usize, 10_000] {
                        let key = format!("k{n}");
                        let tomb = format!("TOMB{n}");
                        let epoch = seed_prunable_slot(&factory, &frontier, &key, n, &tomb).await;
                        measured.push((n, key, tomb, epoch));
                    }
                    frontier.stamp_tombstone("m", "kpin", "TPIN");

                    // Un-measured warm-up on its own key: one-time lazy
                    // initialisation (metric handles, registry entries) lands here
                    // and not in the first measured pass only.
                    license_epoch(&frontier, warm_epoch).await;
                    prune_epoch_tombstones(&frontier, &factory, &svc.key_writer).await;
                    let (_, warm_tombs) = read_or_map(&factory, "m", "kwarm").await;
                    assert!(
                        warm_tombs.is_empty(),
                        "the warm-up pass drops its tombstone"
                    );

                    let mut readings = Vec::new();
                    for (n, key, tomb, epoch) in measured {
                        license_epoch(&frontier, epoch).await;
                        let store = factory.get_or_create("m", hash_to_partition(&key));

                        let (clone, c_bytes) =
                            bytes_allocated_by(async { store.storage().get(&key) }).await;
                        assert!(clone.is_some(), "the slot is resident in the engine");
                        drop(clone);

                        // Let anything the setup woke run to idle before the
                        // measured call, so only the pass allocates inside it.
                        for _ in 0..8 {
                            tokio::task::yield_now().await;
                        }
                        let before = handle.render();
                        let ((), p_bytes) = bytes_allocated_by(prune_epoch_tombstones(
                            &frontier,
                            &factory,
                            &svc.key_writer,
                        ))
                        .await;
                        let after = handle.render();

                        assert_eq!(
                            rendered_counter(&after, METRIC_PRUNE_DROPPED_TOTAL)
                                - rendered_counter(&before, METRIC_PRUNE_DROPPED_TOTAL),
                            1,
                            "N={n}: the measured pass settles exactly one ref Dropped"
                        );
                        assert_eq!(
                            restored(&after),
                            restored(&before),
                            "N={n}: the measured pass restores no ref"
                        );
                        let (tags, tombs) = read_or_map(&factory, "m", &key).await;
                        assert!(!tombs.contains(&tomb), "N={n}: the tombstone is gone");
                        assert_eq!(tags.len(), n, "N={n}: the live records are untouched");

                        readings.push((n, p_bytes, c_bytes));
                    }
                    readings
                })
                .await;
                readings
            })
        });

        for (n, p_bytes, c_bytes) in &readings {
            println!("count_alloc_prune_probe_resident N={n} P={p_bytes} C={c_bytes}");
        }
        let slope = |pick: fn(&(usize, usize, usize)) -> usize| {
            (pick(&readings[1]) as f64 - pick(&readings[0]) as f64) / 9_000.0
        };
        let p = slope(|r| r.1);
        let c = slope(|r| r.2);
        let before_predicate = p >= 1.8 * c;
        let after_predicate = p <= 1.2 * c + 32.0;
        println!(
            "count_alloc_prune_probe_resident p={p:.3} c={c:.3} p/c={:.3} \
             BEFORE(p>=1.8c)={before_predicate} AFTER(p<=1.2c+32)={after_predicate}",
            p / c
        );
        assert!(
            after_predicate,
            "one prune pass over a resident key makes more than one slot copy: \
             p={p:.3} c={c:.3} (bound 1.2c+32 = {:.3})",
            1.2 * c + 32.0
        );
    }

    /// The prune reaches the backend only for a key that is NOT resident.
    ///
    /// A resident key is dropped in place with zero `load` calls: its residency
    /// check replaces a read that could only clone the slot (the allocation side
    /// of that is proved by `count_alloc_prune_probe_resident`, which CI does not
    /// run; this is the CI-visible guard). The controls keep the read where it is
    /// still needed: a durable-only key IS loaded and its tombstone reclaimed,
    /// and a key gone everywhere settles `AbsentKey` rather than being re-indexed
    /// as `RestoredEvicted` on every pass (TG-OR-006).
    ///
    /// Refs are stamped up front with a trailing, never-drained pin ref and the
    /// low-water mark is raised one epoch per pass, for the reason
    /// `count_alloc_prune_probe_resident` documents.
    #[test]
    #[allow(clippy::too_many_lines)]
    fn prune_loads_only_non_resident_keys() {
        use std::sync::atomic::Ordering;

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime");
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();

        metrics::with_local_recorder(&recorder, || {
            rt.block_on(async {
                let store = Arc::new(ArmableStore::default());
                let (svc, factory, frontier) = make_service_with_frontier_and_store(
                    Arc::clone(&store) as Arc<dyn MapDataStore>,
                    Vec::new(),
                );
                frontier.set_delivered(ConnectionId(1), 1_000);
                frontier.set_durable_epoch_watermark(1_000);

                // Resident: written through the service, so the slot is in memory.
                Arc::clone(&svc)
                    .oneshot(or_add_op("m", "kres", "v", "TRES"))
                    .await
                    .unwrap();
                Arc::clone(&svc)
                    .oneshot(or_remove_op("m", "kres", "TRES"))
                    .await
                    .unwrap();
                assert_eq!(frontier.current_epoch(), 1);
                // Durable only: no resident slot mirrors it.
                store.seed_durable(
                    "m",
                    "kdur",
                    RecordValue::OrMap {
                        records: Vec::new(),
                        tombstones: vec!["TDUR".to_string()],
                    },
                );
                assert_eq!(frontier.stamp_tombstone("m", "kdur", "TDUR"), 2);
                // Gone everywhere: a ref with no record behind it.
                assert_eq!(frontier.stamp_tombstone("m", "kabs", "TABS"), 3);
                // Pin: keeps epoch 3 licensable, never drained itself.
                assert_eq!(frontier.stamp_tombstone("m", "kpin", "TPIN"), 4);

                let pass = |epoch: Epoch| {
                    let (store, factory, frontier, svc, handle) =
                        (&store, &factory, &frontier, &svc, &handle);
                    async move {
                        license_epoch(frontier, epoch).await;
                        let loads_before = store.loads.load(Ordering::SeqCst);
                        let before = handle.render();
                        prune_epoch_tombstones(frontier, factory, &svc.key_writer).await;
                        let after = handle.render();
                        let delta = |name: &str| {
                            rendered_counter(&after, name) - rendered_counter(&before, name)
                        };
                        (
                            store.loads.load(Ordering::SeqCst) - loads_before,
                            delta(METRIC_PRUNE_DROPPED_TOTAL),
                            delta(METRIC_PRUNE_ABSENT_TOTAL),
                            delta(METRIC_PRUNE_RESTORED_EVICTED_TOTAL),
                        )
                    }
                };

                let kres_store = factory.get_or_create("m", hash_to_partition("kres"));
                assert!(kres_store.exists_in_memory("kres"), "precondition: resident");
                let (loads, dropped, absent, evicted) = pass(1).await;
                assert_eq!(loads, 0, "a resident key must not reach the backend");
                assert_eq!((dropped, absent, evicted), (1, 0, 0), "resident: Dropped");
                let (_, tombs) = read_or_map(&factory, "m", "kres").await;
                assert!(tombs.is_empty(), "resident: tombstone reclaimed in memory");
                assert!(
                    matches!(store.durable("m", "kres"), Some(RecordValue::OrMap { tombstones, .. }) if tombstones.is_empty()),
                    "resident: tombstone reclaimed durably"
                );

                let kdur_store = factory.get_or_create("m", hash_to_partition("kdur"));
                assert!(!kdur_store.exists_in_memory("kdur"), "precondition: not resident");
                let (loads, dropped, absent, evicted) = pass(2).await;
                assert!(loads >= 1, "a durable-only key is read from the backend");
                assert_eq!((dropped, absent, evicted), (1, 0, 0), "durable-only: Dropped");
                assert!(
                    matches!(store.durable("m", "kdur"), Some(RecordValue::OrMap { tombstones, .. }) if tombstones.is_empty()),
                    "durable-only: tombstone reclaimed durably"
                );

                let (loads, dropped, absent, evicted) = pass(3).await;
                assert!(loads >= 1, "an absent key is looked up in the backend");
                assert_eq!(
                    (dropped, absent, evicted),
                    (0, 1, 0),
                    "gone everywhere: AbsentKey, not RestoredEvicted"
                );
                assert!(
                    frontier
                        .drain_prunable_tombstones()
                        .iter()
                        .all(|(_, r)| r.key != "kabs"),
                    "the absent key's ref is consumed, not re-indexed"
                );
            });
        });
    }

    // -- Writes on a durable-but-non-resident key --
    //
    // A key can be durable without being resident: after a restart nothing
    // re-hydrates the engine, and eviction drops clean records. An op that lands
    // on such a key must be absorbed into the key's durable state, never replace
    // it with a slot rebuilt from nothing.
    mod non_resident_writes {
        use super::*;
        use crate::storage::datastores::{RedbDataStore, WriteBehindConfig, WriteBehindDataStore};

        const MAP: &str = "nonres_map";
        const KEY: &str = "doc";

        fn redb_stack(
            dir: &tempfile::TempDir,
        ) -> (
            Arc<CrdtService>,
            Arc<RecordStoreFactory>,
            Arc<dyn MapDataStore>,
        ) {
            let data_store: Arc<dyn MapDataStore> =
                Arc::new(RedbDataStore::new(dir.path().join("nonres.redb")).expect("redb open"));
            let factory = Arc::new(RecordStoreFactory::new(
                StorageConfig::default(),
                Arc::clone(&data_store),
                Vec::new(),
            ));
            let svc = Arc::new(CrdtService::new(
                Arc::clone(&factory),
                Arc::new(ConnectionRegistry::new()),
                make_validator(),
                Arc::new(QueryRegistry::new()),
                Arc::new(SchemaService::new()),
            ));
            (svc, factory, data_store)
        }

        fn entry(tag: &str) -> OrMapEntry {
            OrMapEntry {
                value: rmpv_to_value(&rmpv::Value::String(format!("v-{tag}").into())),
                tag: tag.to_string(),
                timestamp: make_timestamp(),
            }
        }

        /// Three live entries and one tombstone, written straight to the durable
        /// store so the engine never sees the key.
        async fn seed_durable_or(ds: &Arc<dyn MapDataStore>) {
            let seeded = RecordValue::OrMap {
                records: vec![entry("t-old-1"), entry("t-old-2"), entry("t-old-3")],
                tombstones: vec!["t-gone".to_string()],
            };
            ds.add(MAP, KEY, &seeded, 0, 0)
                .await
                .expect("seed durable row");
        }

        async fn durable_or(ds: &Arc<dyn MapDataStore>) -> (Vec<String>, Vec<String>) {
            match ds.load(MAP, KEY).await.expect("load durable row") {
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
                other => panic!("durable row is not an OrMap: {other:?}"),
            }
        }

        fn assert_not_resident(factory: &Arc<RecordStoreFactory>) {
            let store = factory.get_or_create(MAP, hash_to_partition(KEY));
            assert!(
                !store.exists_in_memory(KEY),
                "precondition: the key must be durable but NOT resident"
            );
        }

        fn strings(v: &[&str]) -> Vec<String> {
            v.iter().map(|s| (*s).to_string()).collect()
        }

        // (a) OR_ADD on a non-resident key keeps every durable entry and tombstone.
        #[tokio::test]
        async fn or_add_on_non_resident_key_keeps_the_durable_entries() {
            let dir = tempfile::tempdir().expect("tempdir");
            let (svc, factory, ds) = redb_stack(&dir);
            seed_durable_or(&ds).await;
            assert_not_resident(&factory);

            svc.clone()
                .oneshot(or_add_op(MAP, KEY, "new", "t-new"))
                .await
                .expect("or_add must succeed");

            assert_eq!(
                durable_or(&ds).await,
                (
                    strings(&["t-new", "t-old-1", "t-old-2", "t-old-3"]),
                    strings(&["t-gone"])
                ),
                "durable row after OR_ADD on a non-resident key must be old ∪ new"
            );
        }

        // (b) OR_REMOVE on a non-resident key removes one tag and keeps the rest.
        #[tokio::test]
        async fn or_remove_on_non_resident_key_keeps_the_other_durable_entries() {
            let dir = tempfile::tempdir().expect("tempdir");
            let (svc, factory, ds) = redb_stack(&dir);
            seed_durable_or(&ds).await;
            assert_not_resident(&factory);

            svc.clone()
                .oneshot(or_remove_op(MAP, KEY, "t-old-1"))
                .await
                .expect("or_remove must succeed");

            assert_eq!(
                durable_or(&ds).await,
                (
                    strings(&["t-old-2", "t-old-3"]),
                    strings(&["t-gone", "t-old-1"])
                ),
                "durable row after OR_REMOVE on a non-resident key must keep the untouched \
                 entries and the earlier tombstone"
            );
        }

        // (d) The same through the real eviction primitive instead of a seeded row.
        #[tokio::test]
        async fn or_add_after_eviction_keeps_the_durable_entries() {
            let dir = tempfile::tempdir().expect("tempdir");
            let (svc, factory, ds) = redb_stack(&dir);
            for tag in ["t-1", "t-2", "t-3"] {
                svc.clone()
                    .oneshot(or_add_op(MAP, KEY, tag, tag))
                    .await
                    .expect("or_add must succeed");
            }
            assert_eq!(
                durable_or(&ds).await.0,
                strings(&["t-1", "t-2", "t-3"]),
                "precondition: the three adds are durable"
            );

            let store = factory.get_or_create(MAP, hash_to_partition(KEY));
            assert!(
                store.evict_lru(u32::MAX, false) > 0,
                "the clean record must be evicted"
            );
            assert_not_resident(&factory);

            svc.clone()
                .oneshot(or_add_op(MAP, KEY, "t-4", "t-4"))
                .await
                .expect("or_add must succeed");

            assert_eq!(
                durable_or(&ds).await.0,
                strings(&["t-1", "t-2", "t-3", "t-4"]),
                "durable row after eviction + OR_ADD must be old ∪ new"
            );
        }

        // (a) control: hydrate first, then OR_ADD. Separates "the write never
        // materializes the durable row" from any other cause.
        #[tokio::test]
        async fn or_add_on_resident_key_keeps_the_durable_entries_control() {
            let dir = tempfile::tempdir().expect("tempdir");
            let (svc, factory, ds) = redb_stack(&dir);
            seed_durable_or(&ds).await;
            let store = factory.get_or_create(MAP, hash_to_partition(KEY));
            store.get(KEY, false).await.expect("hydrate");

            svc.clone()
                .oneshot(or_add_op(MAP, KEY, "new", "t-new"))
                .await
                .expect("or_add must succeed");

            assert_eq!(
                durable_or(&ds).await,
                (
                    strings(&["t-new", "t-old-1", "t-old-2", "t-old-3"]),
                    strings(&["t-gone"])
                ),
                "resident control: durable row must be old ∪ new"
            );
        }

        // AC-4 (gauge): materializing a key charges the tombstone gauge exactly
        // what the same op charges on a resident key — the loaded tombstones are
        // not charged again.
        #[tokio::test]
        async fn or_remove_on_non_resident_key_charges_the_gauge_like_a_resident_key() {
            use crate::storage::tombstone_gauge::with_isolated_gauge;

            let dir_resident = tempfile::tempdir().expect("tempdir");
            let (svc_resident, factory_resident, ds_resident) = redb_stack(&dir_resident);
            seed_durable_or(&ds_resident).await;
            factory_resident
                .get_or_create(MAP, hash_to_partition(KEY))
                .get(KEY, false)
                .await
                .expect("hydrate");
            let (result, resident_delta) = with_isolated_gauge(
                svc_resident
                    .clone()
                    .oneshot(or_remove_op(MAP, KEY, "t-old-1")),
            )
            .await;
            result.expect("or_remove must succeed");

            let dir = tempfile::tempdir().expect("tempdir");
            let (svc, factory, ds) = redb_stack(&dir);
            seed_durable_or(&ds).await;
            assert_not_resident(&factory);
            let (result, non_resident_delta) =
                with_isolated_gauge(svc.clone().oneshot(or_remove_op(MAP, KEY, "t-old-1"))).await;
            result.expect("or_remove must succeed");

            assert!(resident_delta > 0, "the new tombstone must be charged");
            assert_eq!(
                non_resident_delta, resident_delta,
                "a materializing OR_REMOVE must charge the gauge exactly as a resident one"
            );
        }

        /// How long a test waits for the double to park before it declares
        /// that the setup never reached the park point.
        const PARK_BOUND: std::time::Duration = std::time::Duration::from_secs(2);

        type Gate = (
            tokio::sync::oneshot::Sender<()>,
            tokio::sync::oneshot::Receiver<()>,
        );

        /// The test's side of a one-shot park. Dropping it releases the caller.
        struct ParkHandle {
            parked: Option<tokio::sync::oneshot::Receiver<()>>,
            release: Option<tokio::sync::oneshot::Sender<()>>,
        }

        impl ParkHandle {
            async fn wait_parked(&mut self) {
                let parked = self.parked.take().expect("wait_parked called once");
                tokio::time::timeout(PARK_BOUND, parked)
                    .await
                    .expect("the double never parked: the setup did not reach the park point")
                    .expect("park dropped before parking");
            }

            fn release(&mut self) {
                if let Some(release) = self.release.take() {
                    let _ = release.send(());
                }
            }
        }

        /// Forwards to `inner` and parks ONE call, armed just before the
        /// targeted caller runs: on RETURN from `load` (holding the loaded
        /// value), on RETURN from `remove` (the delete applied) or on ENTRY to
        /// `add` (the caller's in-memory mutation done, its write-through not
        /// yet staged). The first matching call consumes the park; later calls
        /// pass unparked. It can also fail ONE `load` without reading `inner`.
        struct ParkingStore {
            inner: Arc<dyn MapDataStore>,
            after_load: std::sync::Mutex<Option<Gate>>,
            after_remove: std::sync::Mutex<Option<Gate>>,
            before_add: std::sync::Mutex<Option<Gate>>,
            fail_next_load: std::sync::atomic::AtomicBool,
        }

        impl ParkingStore {
            fn new(inner: Arc<dyn MapDataStore>) -> Self {
                Self {
                    inner,
                    after_load: std::sync::Mutex::new(None),
                    after_remove: std::sync::Mutex::new(None),
                    before_add: std::sync::Mutex::new(None),
                    fail_next_load: std::sync::atomic::AtomicBool::new(false),
                }
            }

            fn arm(slot: &std::sync::Mutex<Option<Gate>>) -> ParkHandle {
                let (parked_tx, parked_rx) = tokio::sync::oneshot::channel();
                let (release_tx, release_rx) = tokio::sync::oneshot::channel();
                *slot.lock().unwrap() = Some((parked_tx, release_rx));
                ParkHandle {
                    parked: Some(parked_rx),
                    release: Some(release_tx),
                }
            }

            fn park_after_load(&self) -> ParkHandle {
                Self::arm(&self.after_load)
            }

            fn park_after_remove(&self) -> ParkHandle {
                Self::arm(&self.after_remove)
            }

            fn park_before_add(&self) -> ParkHandle {
                Self::arm(&self.before_add)
            }

            /// The next `load` returns `Err` without reading `inner`.
            fn fail_next_load(&self) {
                self.fail_next_load
                    .store(true, std::sync::atomic::Ordering::SeqCst);
            }

            async fn pass(slot: &std::sync::Mutex<Option<Gate>>) {
                let gate = slot.lock().unwrap().take();
                if let Some((parked, release)) = gate {
                    let _ = parked.send(());
                    let _ = release.await;
                }
            }
        }

        #[async_trait]
        impl MapDataStore for ParkingStore {
            async fn add(
                &self,
                map: &str,
                key: &str,
                value: &RecordValue,
                expiration_time: i64,
                now: i64,
            ) -> anyhow::Result<()> {
                Self::pass(&self.before_add).await;
                self.inner.add(map, key, value, expiration_time, now).await
            }

            async fn add_backup(
                &self,
                map: &str,
                key: &str,
                value: &RecordValue,
                expiration_time: i64,
                now: i64,
            ) -> anyhow::Result<()> {
                self.inner
                    .add_backup(map, key, value, expiration_time, now)
                    .await
            }

            async fn remove(&self, map: &str, key: &str, now: i64) -> anyhow::Result<()> {
                let result = self.inner.remove(map, key, now).await;
                Self::pass(&self.after_remove).await;
                result
            }

            async fn remove_backup(&self, map: &str, key: &str, now: i64) -> anyhow::Result<()> {
                self.inner.remove_backup(map, key, now).await
            }

            async fn load(&self, map: &str, key: &str) -> anyhow::Result<Option<RecordValue>> {
                if self
                    .fail_next_load
                    .swap(false, std::sync::atomic::Ordering::SeqCst)
                {
                    return Err(anyhow::anyhow!("injected load failure"));
                }
                let loaded = self.inner.load(map, key).await;
                Self::pass(&self.after_load).await;
                loaded
            }

            async fn load_all(
                &self,
                map: &str,
                keys: &[String],
            ) -> anyhow::Result<Vec<(String, RecordValue)>> {
                self.inner.load_all(map, keys).await
            }

            async fn enumerate_leaves(
                &self,
                map: &str,
                is_backup: bool,
                sink: &mut dyn LeafSink,
            ) -> anyhow::Result<()> {
                self.inner.enumerate_leaves(map, is_backup, sink).await
            }

            async fn scan_values(
                &self,
                map: &str,
                is_backup: bool,
                max_batch_cost: u64,
            ) -> anyhow::Result<ScanBatch> {
                self.inner.scan_values(map, is_backup, max_batch_cost).await
            }

            async fn scan_values_batched(
                &self,
                map: &str,
                is_backup: bool,
                cursor: ScanCursor,
                max_batch_cost: u64,
            ) -> anyhow::Result<ScanBatch> {
                self.inner
                    .scan_values_batched(map, is_backup, cursor, max_batch_cost)
                    .await
            }

            async fn remove_all(&self, map: &str, keys: &[String]) -> anyhow::Result<()> {
                self.inner.remove_all(map, keys).await
            }

            fn is_loadable(&self, key: &str) -> bool {
                self.inner.is_loadable(key)
            }

            fn pending_operation_count(&self) -> u64 {
                self.inner.pending_operation_count()
            }

            async fn soft_flush(&self) -> anyhow::Result<u64> {
                self.inner.soft_flush().await
            }

            async fn hard_flush(&self) -> anyhow::Result<()> {
                self.inner.hard_flush().await
            }

            async fn flush_key(
                &self,
                map: &str,
                key: &str,
                value: &RecordValue,
                is_backup: bool,
            ) -> anyhow::Result<()> {
                self.inner.flush_key(map, key, value, is_backup).await
            }

            fn reset(&self) {
                self.inner.reset();
            }
        }

        fn remove_op(map: &str, key: &str) -> Operation {
            Operation::ClientOp {
                ctx: make_ctx_for_key(key),
                payload: topgun_core::messages::ClientOpMessage {
                    payload: topgun_core::messages::base::ClientOp {
                        id: Some(format!("remove-{key}")),
                        map_name: map.to_string(),
                        key: key.to_string(),
                        op_type: Some("REMOVE".to_string()),
                        record: None,
                        or_record: None,
                        or_tag: None,
                        write_concern: None,
                        timeout: None,
                    },
                },
            }
        }

        /// The redb stack of [`redb_stack`] with a [`ParkingStore`] between the
        /// record stores and redb. Returns the service, the factory, redb
        /// itself (for seeding and reading the durable row) and the double.
        fn parking_stack(
            dir: &tempfile::TempDir,
        ) -> (
            Arc<CrdtService>,
            Arc<RecordStoreFactory>,
            Arc<dyn MapDataStore>,
            Arc<ParkingStore>,
        ) {
            let redb: Arc<dyn MapDataStore> =
                Arc::new(RedbDataStore::new(dir.path().join("nonres.redb")).expect("redb open"));
            let parking = Arc::new(ParkingStore::new(Arc::clone(&redb)));
            let factory = Arc::new(RecordStoreFactory::new(
                StorageConfig::default(),
                parking.clone() as Arc<dyn MapDataStore>,
                Vec::new(),
            ));
            let svc = Arc::new(CrdtService::new(
                Arc::clone(&factory),
                Arc::new(ConnectionRegistry::new()),
                make_validator(),
                Arc::new(QueryRegistry::new()),
                Arc::new(SchemaService::new()),
            ));
            (svc, factory, redb, parking)
        }

        /// The durable tag set, or `None` when the row is gone.
        async fn durable_tags_or_none(ds: &Arc<dyn MapDataStore>) -> Option<Vec<String>> {
            ds.load(MAP, KEY)
                .await
                .expect("load durable row")
                .map(|value| match value {
                    RecordValue::OrMap { records, .. } => {
                        let mut tags: Vec<String> = records.into_iter().map(|e| e.tag).collect();
                        tags.sort();
                        tags
                    }
                    other => panic!("durable row is not an OrMap: {other:?}"),
                })
        }

        // AC-6b: a REMOVE that lands while an OR_ADD is materializing (parked
        // holding the loaded `D`) must not be undone by that OR_ADD (TG-OR-007).
        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn remove_during_a_materializing_or_add_is_not_undone() {
            let dir = tempfile::tempdir().expect("tempdir");
            let (svc, factory, redb, parking) = parking_stack(&dir);
            seed_durable_or(&redb).await;
            assert_not_resident(&factory);

            let mut writer_park = parking.park_after_load();
            let writer = tokio::spawn(svc.clone().oneshot(or_add_op(MAP, KEY, "new", "t-new")));
            writer_park.wait_parked().await;

            // Spawned, not awaited first: once REMOVE takes the key's writer it
            // waits for the parked OR_ADD, so the park is released on REMOVE's
            // completion or after the bound, whichever comes first.
            let mut remover = tokio::spawn(svc.clone().oneshot(remove_op(MAP, KEY)));
            let remover_done = tokio::time::timeout(PARK_BOUND, &mut remover).await;
            writer_park.release();
            writer
                .await
                .expect("writer task")
                .expect("or_add must succeed");
            match remover_done {
                Ok(done) => {
                    done.expect("remover task").expect("remove must succeed");
                }
                Err(_) => {
                    remover
                        .await
                        .expect("remover task")
                        .expect("remove must succeed");
                }
            }

            let durable = durable_tags_or_none(&redb).await;
            assert!(
                durable == Some(strings(&["t-new"])) || durable.is_none(),
                "durable row must be {{op}} or gone, never the removed entries plus op; got {durable:?}"
            );
        }

        // AC-6d: an OR_ADD on a resident key must not re-stage the key over a
        // REMOVE's pending delete; REMOVE and in-place writes share the key's
        // writer (TG-OR-007).
        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn or_add_between_the_steps_of_a_remove_does_not_resurrect_it() {
            let dir = tempfile::tempdir().expect("tempdir");
            let (svc, factory, redb, parking) = parking_stack(&dir);
            seed_durable_or(&redb).await;
            factory
                .get_or_create(MAP, hash_to_partition(KEY))
                .get(KEY, false)
                .await
                .expect("hydrate");

            let mut remove_park = parking.park_after_remove();
            let remover = tokio::spawn(svc.clone().oneshot(remove_op(MAP, KEY)));
            remove_park.wait_parked().await;

            // Spawned, not awaited first: once REMOVE holds the key's writer the
            // OR_ADD waits for it, so the park is released on the OR_ADD's
            // completion or after the bound, whichever comes first.
            let mut writer = tokio::spawn(svc.clone().oneshot(or_add_op(MAP, KEY, "new", "t-new")));
            let writer_done = tokio::time::timeout(PARK_BOUND, &mut writer).await;
            remove_park.release();
            remover
                .await
                .expect("remover task")
                .expect("remove must succeed");
            match writer_done {
                Ok(done) => {
                    done.expect("writer task").expect("or_add must succeed");
                }
                Err(_) => {
                    writer
                        .await
                        .expect("writer task")
                        .expect("or_add must succeed");
                }
            }

            let durable = durable_tags_or_none(&redb).await;
            assert!(
                durable == Some(strings(&["t-new"])) || durable.is_none(),
                "durable row must be {{op}} or gone, never the removed entries plus op; got {durable:?}"
            );
        }

        /// The redb stack of [`redb_stack`] with a write-behind store directly
        /// between the record stores and redb, flushed only on request, so an
        /// in-place write stays queued and staged as a cell. No wrapper sits in
        /// between: one would answer `load_slot` and `add_with_witness` through
        /// the value-path defaults.
        fn write_behind_stack(
            dir: &tempfile::TempDir,
        ) -> (
            Arc<CrdtService>,
            Arc<RecordStoreFactory>,
            Arc<dyn MapDataStore>,
            Arc<WriteBehindDataStore>,
        ) {
            let redb: Arc<dyn MapDataStore> =
                Arc::new(RedbDataStore::new(dir.path().join("nonres.redb")).expect("redb open"));
            let write_behind = WriteBehindDataStore::new(
                Arc::clone(&redb),
                WriteBehindConfig {
                    write_delay_ms: 600_000,
                    flush_interval_ms: 600_000,
                    ..WriteBehindConfig::default()
                },
            );
            let factory = Arc::new(RecordStoreFactory::new(
                StorageConfig::default(),
                Arc::clone(&write_behind) as Arc<dyn MapDataStore>,
                Vec::new(),
            ));
            let svc = Arc::new(CrdtService::new(
                Arc::clone(&factory),
                Arc::new(ConnectionRegistry::new()),
                make_validator(),
                Arc::new(QueryRegistry::new()),
                Arc::new(SchemaService::new()),
            ));
            (svc, factory, redb, write_behind)
        }

        /// Seeds the durable row, then leaves an OR_ADD of `t-pend` pending: its
        /// write is staged as a cell and the key is evicted from the engine.
        async fn evict_a_staged_cell(
            svc: &Arc<CrdtService>,
            factory: &Arc<RecordStoreFactory>,
            redb: &Arc<dyn MapDataStore>,
            write_behind: &WriteBehindDataStore,
        ) {
            seed_durable_or(redb).await;
            svc.clone()
                .oneshot(or_add_op(MAP, KEY, "pend", "t-pend"))
                .await
                .expect("or_add must succeed");
            assert!(
                write_behind.test_staged_cell(MAP, KEY).is_some(),
                "precondition: the OR_ADD is staged as a cell"
            );
            let store = factory.get_or_create(MAP, hash_to_partition(KEY));
            assert!(
                store.evict_lru(u32::MAX, false) > 0,
                "precondition: the marked-clean pending key is evicted"
            );
            assert_not_resident(factory);
        }

        // Path (i) × REMOVE on a cell: a REMOVE that lands while an OR_ADD is
        // materializing the staged cell must not be undone by that OR_ADD
        // (TG-OR-007).
        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn remove_during_a_materializing_or_add_is_not_undone_on_a_staged_cell() {
            let dir = tempfile::tempdir().expect("tempdir");
            let (svc, factory, redb, write_behind) = write_behind_stack(&dir);
            evict_a_staged_cell(&svc, &factory, &redb, &write_behind).await;

            let mut writer_park = write_behind.test_park_load_slot(MAP, KEY);
            let writer = tokio::spawn(svc.clone().oneshot(or_add_op(MAP, KEY, "new", "t-new")));
            writer_park.wait_parked().await;

            // Spawned, not awaited first: once REMOVE takes the key's writer it
            // waits for the parked OR_ADD, so the park is released on REMOVE's
            // completion or after the bound, whichever comes first.
            let mut remover = tokio::spawn(svc.clone().oneshot(remove_op(MAP, KEY)));
            let remover_done = tokio::time::timeout(PARK_BOUND, &mut remover).await;
            writer_park.release();
            writer
                .await
                .expect("writer task")
                .expect("or_add must succeed");
            match remover_done {
                Ok(done) => {
                    done.expect("remover task").expect("remove must succeed");
                }
                Err(_) => {
                    remover
                        .await
                        .expect("remover task")
                        .expect("remove must succeed");
                }
            }

            assert_eq!(
                write_behind.test_load_slot_park_hits(),
                1,
                "the OR_ADD must have materialized the staged cell"
            );
            write_behind.hard_flush().await.expect("hard_flush");
            let durable = durable_tags_or_none(&redb).await;
            assert!(
                durable == Some(strings(&["t-new"])) || durable.is_none(),
                "durable row must be {{op}} or gone, never the removed entries plus op; got {durable:?}"
            );
        }

        // Path (iv) on a cell: an OR_ADD on a key resident as its adopted cell
        // must not re-stage the cell over a REMOVE's pending delete (TG-OR-007).
        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn or_add_between_the_steps_of_a_remove_does_not_resurrect_it_on_a_staged_cell() {
            let dir = tempfile::tempdir().expect("tempdir");
            let (svc, factory, redb, write_behind) = write_behind_stack(&dir);
            evict_a_staged_cell(&svc, &factory, &redb, &write_behind).await;

            // Hydrate through the cell branch, so the resident slot is the cell
            // the pending entry pins.
            let mut hydrate_park = write_behind.test_park_load_slot(MAP, KEY);
            let hydrate = {
                let store = factory.get_or_create(MAP, hash_to_partition(KEY));
                tokio::spawn(async move { store.get(KEY, false).await })
            };
            hydrate_park.wait_parked().await;
            hydrate_park.release();
            hydrate.await.expect("hydrate task").expect("hydrate");
            assert_eq!(
                write_behind.test_load_slot_park_hits(),
                1,
                "the hydrate must have adopted the staged cell"
            );
            assert!(
                factory
                    .get_or_create(MAP, hash_to_partition(KEY))
                    .exists_in_memory(KEY),
                "precondition: the adopted cell is resident"
            );

            let mut remove_park = write_behind.test_park_after_remove(MAP, KEY);
            let remover = tokio::spawn(svc.clone().oneshot(remove_op(MAP, KEY)));
            remove_park.wait_parked().await;

            // Spawned, not awaited first: once REMOVE holds the key's writer the
            // OR_ADD waits for it, so the park is released on the OR_ADD's
            // completion or after the bound, whichever comes first.
            let mut writer = tokio::spawn(svc.clone().oneshot(or_add_op(MAP, KEY, "new", "t-new")));
            let writer_done = tokio::time::timeout(PARK_BOUND, &mut writer).await;
            remove_park.release();
            remover
                .await
                .expect("remover task")
                .expect("remove must succeed");
            match writer_done {
                Ok(done) => {
                    done.expect("writer task").expect("or_add must succeed");
                }
                Err(_) => {
                    writer
                        .await
                        .expect("writer task")
                        .expect("or_add must succeed");
                }
            }

            write_behind.hard_flush().await.expect("hard_flush");
            let durable = durable_tags_or_none(&redb).await;
            assert!(
                durable == Some(strings(&["t-new"])) || durable.is_none(),
                "durable row must be {{op}} or gone, never the removed entries plus op; got {durable:?}"
            );
        }

        /// A `SyncService` over `factory` that shares `svc`'s per-key writer, as
        /// production wires the two services over one record-store factory.
        fn sync_sharing_writer(
            svc: &Arc<CrdtService>,
            factory: &Arc<RecordStoreFactory>,
        ) -> Arc<crate::service::domain::sync::SyncService> {
            Arc::new(
                crate::service::domain::sync::SyncService::new(
                    Arc::new(MerkleSyncManager::default()),
                    Arc::clone(factory),
                    Arc::new(ConnectionRegistry::new()),
                )
                .with_key_writer(Arc::clone(&svc.key_writer)),
            )
        }

        /// [`sync_sharing_writer`] with the forgotten-client gate wired over a
        /// store-less frontier (watermark 0, gate off). The frontier is returned
        /// so the test can turn the gate on.
        fn gated_sync_sharing_writer(
            svc: &Arc<CrdtService>,
            factory: &Arc<RecordStoreFactory>,
        ) -> (
            Arc<crate::service::domain::sync::SyncService>,
            Arc<crate::tombstone_frontier_impl::TombstoneFrontier>,
        ) {
            let frontier = Arc::new(crate::tombstone_frontier_impl::TombstoneFrontier::new(None));
            let sync = Arc::new(
                crate::service::domain::sync::SyncService::new(
                    Arc::new(MerkleSyncManager::default()),
                    Arc::clone(factory),
                    Arc::new(ConnectionRegistry::new()),
                )
                .with_frontier(Arc::clone(&frontier), Arc::clone(&svc.key_writer)),
            );
            (sync, frontier)
        }

        /// One pushed OR-Map entry carrying live records for `tags` and no
        /// tombstones.
        fn push_entry(key: &str, tags: &[&str]) -> topgun_core::messages::ORMapEntry {
            topgun_core::messages::ORMapEntry {
                key: key.to_string(),
                records: tags
                    .iter()
                    .map(|tag| topgun_core::ORMapRecord {
                        value: rmpv::Value::String(format!("v-{tag}").into()),
                        timestamp: make_timestamp(),
                        tag: (*tag).to_string(),
                        ttl_ms: None,
                    })
                    .collect(),
                tombstones: Vec::new(),
            }
        }

        /// An `ORMapPushDiff` of `entries` on [`MAP`] from a context without a
        /// connection, so the pushing client can never be resolved.
        fn push_op(entries: Vec<topgun_core::messages::ORMapEntry>) -> Operation {
            Operation::ORMapPushDiff {
                ctx: make_ctx_sync(),
                payload: topgun_core::messages::ORMapPushDiff {
                    payload: topgun_core::messages::ORMapPushDiffPayload {
                        map_name: MAP.to_string(),
                        entries,
                    },
                },
            }
        }

        /// The durable row of `key` as sorted (tags, tombstones), or `None`
        /// when the row is gone.
        async fn durable_row(
            ds: &Arc<dyn MapDataStore>,
            key: &str,
        ) -> Option<(Vec<String>, Vec<String>)> {
            ds.load(MAP, key)
                .await
                .expect("load durable row")
                .map(|value| match value {
                    RecordValue::OrMap {
                        records,
                        tombstones,
                    } => {
                        let mut tags: Vec<String> = records.into_iter().map(|e| e.tag).collect();
                        tags.sort();
                        let mut tombs = tombstones;
                        tombs.sort();
                        (tags, tombs)
                    }
                    other => panic!("durable row is not an OrMap: {other:?}"),
                })
        }

        /// The pushed tags, disjoint from the seeded `D` of [`seed_durable_or`].
        const PUSHED: [&str; 2] = ["s-1", "s-2"];

        // AC-1: a REMOVE that lands while a push is materializing the key
        // (parked holding the loaded `D`) must not be undone by the push's
        // write-through (TG-OR-007).
        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn push_during_a_remove_does_not_resurrect_it() {
            let dir = tempfile::tempdir().expect("tempdir");
            let (svc, factory, redb, parking) = parking_stack(&dir);
            let sync = sync_sharing_writer(&svc, &factory);
            seed_durable_or(&redb).await;
            assert_not_resident(&factory);

            let mut push_park = parking.park_after_load();
            let pusher = tokio::spawn(
                sync.clone()
                    .oneshot(push_op(vec![push_entry(KEY, &PUSHED)])),
            );
            push_park.wait_parked().await;

            // Spawned, not awaited first: once the push holds the key's writer
            // REMOVE waits for it, so the park is released on REMOVE's
            // completion or after the bound, whichever comes first.
            let mut remover = tokio::spawn(svc.clone().oneshot(remove_op(MAP, KEY)));
            let remover_done = tokio::time::timeout(PARK_BOUND, &mut remover).await;
            push_park.release();
            pusher.await.expect("push task").expect("push must succeed");
            match remover_done {
                Ok(done) => {
                    done.expect("remover task").expect("remove must succeed");
                }
                Err(_) => {
                    remover
                        .await
                        .expect("remover task")
                        .expect("remove must succeed");
                }
            }

            let durable = durable_tags_or_none(&redb).await;
            assert!(
                durable == Some(strings(&PUSHED)) || durable.is_none(),
                "durable row must be the pushed entries or gone, never the removed entries \
                 plus the pushed ones; got {durable:?}"
            );
        }

        /// AC-2 and its resident control: a push parked holding the loaded `D`
        /// while an OR_ADD of `op` runs; with `evict`, the OR_ADD's clean result
        /// is evicted before the push resumes.
        async fn push_across_an_or_add(evict: bool) -> Option<Vec<String>> {
            let dir = tempfile::tempdir().expect("tempdir");
            let (svc, factory, redb, parking) = parking_stack(&dir);
            let sync = sync_sharing_writer(&svc, &factory);
            seed_durable_or(&redb).await;
            assert_not_resident(&factory);

            let mut push_park = parking.park_after_load();
            let pusher = tokio::spawn(
                sync.clone()
                    .oneshot(push_op(vec![push_entry(KEY, &PUSHED)])),
            );
            push_park.wait_parked().await;

            // Spawned, not awaited first: once the push holds the key's writer
            // the OR_ADD waits for it, so the bound decides which branch runs.
            let mut writer = tokio::spawn(svc.clone().oneshot(or_add_op(MAP, KEY, "op", "t-op")));
            let writer_done = tokio::time::timeout(PARK_BOUND, &mut writer).await;
            if evict {
                let evicted = factory
                    .get_or_create(MAP, hash_to_partition(KEY))
                    .evict_lru(u32::MAX, false);
                // Only an OR_ADD that already completed has left a clean
                // resident to evict; one still waiting on the push's writer
                // has left nothing resident.
                if writer_done.is_ok() {
                    assert!(
                        evicted > 0,
                        "the OR_ADD's clean record must be evicted before the push resumes"
                    );
                }
            }
            push_park.release();
            pusher.await.expect("push task").expect("push must succeed");
            match writer_done {
                Ok(done) => {
                    done.expect("writer task").expect("or_add must succeed");
                }
                Err(_) => {
                    writer
                        .await
                        .expect("writer task")
                        .expect("or_add must succeed");
                }
            }

            durable_tags_or_none(&redb).await
        }

        // AC-2: an acked OR_ADD that materialized, persisted and was evicted
        // while a push held the stale `D` must survive the push (TG-OR-007).
        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn push_across_an_evicted_or_add_keeps_the_acked_add() {
            let durable = push_across_an_or_add(true).await;
            assert_eq!(
                durable,
                Some(strings(&[
                    "s-1", "s-2", "t-old-1", "t-old-2", "t-old-3", "t-op"
                ])),
                "durable tags must be D ∪ S ∪ {{op}}; the acked OR_ADD must not be lost"
            );
        }

        // AC-3 (control): the same interleaving with the OR_ADD's result still
        // resident, where the push merges into the resident value.
        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn push_across_a_resident_or_add_keeps_it_control() {
            let durable = push_across_an_or_add(false).await;
            assert_eq!(
                durable,
                Some(strings(&[
                    "s-1", "s-2", "t-old-1", "t-old-2", "t-old-3", "t-op"
                ])),
                "resident control: durable tags must be D ∪ S ∪ {{op}}"
            );
        }

        // AC-4: a push whose read of the durable row fails must fail, and must
        // not overwrite the durable `D` with the pushed entries alone.
        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn push_with_a_failed_load_keeps_the_durable_value() {
            let dir = tempfile::tempdir().expect("tempdir");
            let (svc, factory, redb, parking) = parking_stack(&dir);
            let sync = sync_sharing_writer(&svc, &factory);
            seed_durable_or(&redb).await;
            assert_not_resident(&factory);

            parking.fail_next_load();
            let result = sync
                .clone()
                .oneshot(push_op(vec![push_entry(KEY, &PUSHED)]))
                .await;

            let durable = durable_row(&redb, KEY).await;
            let seeded = Some((
                strings(&["t-old-1", "t-old-2", "t-old-3"]),
                strings(&["t-gone"]),
            ));
            assert!(
                result.is_err() && durable == seeded,
                "a push whose load failed must return Err and leave the durable row as D; \
                 got result {result:?}, durable {durable:?}"
            );
        }

        // AC-6: a push on a resident key must not re-stage the key over a
        // REMOVE's pending delete; REMOVE and the push share the key's writer
        // (TG-OR-007).
        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn push_between_the_steps_of_a_remove_does_not_resurrect_it() {
            let dir = tempfile::tempdir().expect("tempdir");
            let (svc, factory, redb, parking) = parking_stack(&dir);
            let sync = sync_sharing_writer(&svc, &factory);
            seed_durable_or(&redb).await;
            factory
                .get_or_create(MAP, hash_to_partition(KEY))
                .get(KEY, false)
                .await
                .expect("hydrate");

            let mut remove_park = parking.park_after_remove();
            let remover = tokio::spawn(svc.clone().oneshot(remove_op(MAP, KEY)));
            remove_park.wait_parked().await;

            // Spawned, not awaited first: once REMOVE holds the key's writer the
            // push waits for it, so the park is released on the push's
            // completion or after the bound, whichever comes first.
            let mut pusher = tokio::spawn(
                sync.clone()
                    .oneshot(push_op(vec![push_entry(KEY, &PUSHED)])),
            );
            let pusher_done = tokio::time::timeout(PARK_BOUND, &mut pusher).await;
            remove_park.release();
            remover
                .await
                .expect("remover task")
                .expect("remove must succeed");
            match pusher_done {
                Ok(done) => {
                    done.expect("push task").expect("push must succeed");
                }
                Err(_) => {
                    pusher.await.expect("push task").expect("push must succeed");
                }
            }

            let durable = durable_tags_or_none(&redb).await;
            assert!(
                durable == Some(strings(&PUSHED)) || durable.is_none(),
                "durable row must be the pushed entries or gone, never the removed entries \
                 plus the pushed ones; got {durable:?}"
            );
        }

        // AC-6b: two writers of one key must stage their write-throughs in the
        // order of their in-memory mutations, so the durable row ends equal to
        // the resident one (TG-OR-007).
        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn push_and_or_add_stage_in_mutation_order() {
            let dir = tempfile::tempdir().expect("tempdir");
            let (svc, factory, redb, parking) = parking_stack(&dir);
            let sync = sync_sharing_writer(&svc, &factory);
            seed_durable_or(&redb).await;
            factory
                .get_or_create(MAP, hash_to_partition(KEY))
                .get(KEY, false)
                .await
                .expect("hydrate");

            let mut add_park = parking.park_before_add();
            let pusher = tokio::spawn(
                sync.clone()
                    .oneshot(push_op(vec![push_entry(KEY, &PUSHED)])),
            );
            add_park.wait_parked().await;

            // Spawned, not awaited first: once the push holds the key's writer
            // across its staging the OR_ADD waits for it, so the park is
            // released on the OR_ADD's completion or after the bound.
            let mut writer = tokio::spawn(svc.clone().oneshot(or_add_op(MAP, KEY, "op", "t-op")));
            let writer_done = tokio::time::timeout(PARK_BOUND, &mut writer).await;
            add_park.release();
            pusher.await.expect("push task").expect("push must succeed");
            match writer_done {
                Ok(done) => {
                    done.expect("writer task").expect("or_add must succeed");
                }
                Err(_) => {
                    writer
                        .await
                        .expect("writer task")
                        .expect("or_add must succeed");
                }
            }

            assert_eq!(
                durable_tags_or_none(&redb).await,
                Some(strings(&[
                    "s-1", "s-2", "t-old-1", "t-old-2", "t-old-3", "t-op"
                ])),
                "durable tags must be D ∪ S ∪ {{op}}, equal to the resident; a write-through \
                 staged out of mutation order loses op"
            );
        }

        // AC-8: the forgotten-client gate turning on while a batch is being
        // merged must gate the entries that have not been merged yet.
        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn gate_turning_on_mid_batch_gates_the_rest_of_the_batch() {
            const KEY_2: &str = "doc-2";
            let dir = tempfile::tempdir().expect("tempdir");
            let (svc, factory, redb, parking) = parking_stack(&dir);
            let (sync, frontier) = gated_sync_sharing_writer(&svc, &factory);
            seed_durable_or(&redb).await;
            assert_not_resident(&factory);
            assert!(
                !frontier.is_protection_active(),
                "precondition: the gate starts off"
            );

            let mut load_park = parking.park_after_load();
            let pusher = tokio::spawn(sync.clone().oneshot(push_op(vec![
                push_entry(KEY, &["s1-1"]),
                push_entry(KEY_2, &["s2-1"]),
            ])));
            load_park.wait_parked().await;
            frontier.set_durable_epoch_watermark(1000);
            assert!(
                frontier.is_protection_active(),
                "precondition: the gate is on while the first entry is parked"
            );
            load_park.release();
            pusher
                .await
                .expect("push task")
                .expect("push must return a response");

            let first = durable_row(&redb, KEY).await.map(|(tags, _)| tags);
            let second = durable_row(&redb, KEY_2).await;
            assert!(
                first == Some(strings(&["s1-1", "t-old-1", "t-old-2", "t-old-3"]))
                    && second.is_none(),
                "the entry merged before the gate turned on must be D1 ∪ S1 and the rest of the \
                 batch must be rejected; got first {first:?}, second {second:?}"
            );
        }

        // ---- Two writers of one key, each on its own route ----------------
        //
        // The tests below start two LWW writes of one key from two tasks, park
        // the first inside its write, and let the second run. No sleep orders
        // anything: every wait is either a park's own signal or the choice
        // "the second writer returned, or it is counted as waiting for the
        // key's writer", and the bound on it only detects a broken setup.

        const VX: &str = "x";
        const VY: &str = "y";

        fn lww_put(text: &str) -> topgun_core::messages::base::ClientOp {
            topgun_core::messages::base::ClientOp {
                id: Some(format!("put-{text}")),
                map_name: MAP.to_string(),
                key: KEY.to_string(),
                op_type: None,
                record: Some(Some(topgun_core::LWWRecord {
                    value: Some(rmpv::Value::String(text.into())),
                    timestamp: make_timestamp(),
                    ttl_ms: None,
                })),
                or_record: None,
                or_tag: None,
                write_concern: None,
                timeout: None,
            }
        }

        /// Route X: a single op addressed to the key's partition, as the
        /// partition worker receives it. Anonymous with no connection, so the
        /// service mints the stamp.
        fn route_x(text: &str) -> Operation {
            Operation::ClientOp {
                ctx: make_anon_http_ctx_for_key(KEY),
                payload: topgun_core::messages::ClientOpMessage {
                    payload: lww_put(text),
                },
            }
        }

        /// Route Y: a batch with no partition, which is what classification
        /// produces for an enveloped batch and what the global worker runs.
        fn route_y(text: &str) -> Operation {
            let mut ctx = make_anon_http_ctx_for_key(KEY);
            ctx.partition_id = None;
            Operation::OpBatch {
                ctx,
                payload: topgun_core::messages::sync::OpBatchMessage {
                    payload: topgun_core::messages::sync::OpBatchPayload {
                        ops: vec![lww_put(text)],
                        write_concern: None,
                        timeout: None,
                    },
                },
            }
        }

        fn lww_text(value: Option<RecordValue>) -> Option<String> {
            match value {
                Some(RecordValue::Lww { value, .. }) => {
                    value_to_rmpv(&value).as_str().map(str::to_string)
                }
                _ => None,
            }
        }

        /// Whether the second writer had to wait for the key's writer while the
        /// first one was parked: `false` when it returned instead. Exactly one
        /// of the two happens, so the loop needs no timing margin; the bound
        /// only reports a setup in which neither did.
        async fn second_writer_waited<T>(
            svc: &CrdtService,
            second: &tokio::task::JoinHandle<T>,
        ) -> bool {
            tokio::time::timeout(PARK_BOUND, async {
                loop {
                    if second.is_finished() {
                        return false;
                    }
                    if svc.key_writer.test_waiting() == 1 {
                        return true;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("the second writer neither returned nor waited for the key's writer")
        }

        /// The stamps the journal holds for the key, in the order the writes
        /// were recorded.
        fn journal_stamps(journal: &JournalStore) -> Vec<Timestamp> {
            journal
                .read(0, 100, Some(MAP))
                .0
                .into_iter()
                .filter(|event| event.key == KEY)
                .map(|event| event.timestamp)
                .collect()
        }

        // A write that is parked between its in-memory put and its store write
        // must not let a later write of the same key pass it: the engine would
        // keep the later value while the store, and so every reload, keeps the
        // earlier one.
        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn two_routes_writing_one_key_leave_engine_and_store_on_the_later_stamp() {
            let dir = tempfile::tempdir().expect("tempdir");
            let (svc, factory, redb, parking) = parking_stack(&dir);

            let mut park = parking.park_before_add();
            let x = tokio::spawn(svc.clone().oneshot(route_x(VX)));
            // X has put its value into the engine and is parked on entry to
            // its store write.
            park.wait_parked().await;

            let y = tokio::spawn(svc.clone().oneshot(route_y(VY)));
            let y_waited = second_writer_waited(&svc, &y).await;
            park.release();
            x.await.expect("X task").expect("X must succeed");
            y.await.expect("Y task").expect("Y must succeed");

            let partition = hash_to_partition(KEY);
            let engine = lww_text(
                factory
                    .get_or_create(MAP, partition)
                    .get(KEY, false)
                    .await
                    .expect("engine read")
                    .map(|record| record.value),
            );
            let store = lww_text(redb.load(MAP, KEY).await.expect("store read"));
            let fresh = Arc::new(RecordStoreFactory::new(
                StorageConfig::default(),
                Arc::clone(&redb),
                Vec::new(),
            ));
            let reloaded = lww_text(
                fresh
                    .get_or_create(MAP, partition)
                    .get(KEY, false)
                    .await
                    .expect("reload")
                    .map(|record| record.value),
            );

            println!(
                "two_routes/store: y_waited={y_waited} engine={engine:?} store={store:?} \
                 reloaded={reloaded:?}"
            );
            assert_eq!(
                (engine.as_deref(), store.as_deref(), reloaded.as_deref()),
                (Some(VY), Some(VY), Some(VY)),
                "(engine, store, reloaded by a fresh factory): the write that started later \
                 holds the later stamp, and all three must hold its value"
            );
        }

        /// What [`parking_stack_with_subscription`] builds.
        struct SubscribedStack {
            svc: Arc<CrdtService>,
            factory: Arc<RecordStoreFactory>,
            parking: Arc<ParkingStore>,
            journal: Arc<JournalStore>,
            /// Keeps the subscribed connection's channel open.
            _listener: tokio::sync::mpsc::Receiver<crate::network::connection::OutboundMessage>,
        }

        /// [`parking_stack`] with a journal attached and one query subscription
        /// on the map, so a write reads the key's previous value before it
        /// applies.
        fn parking_stack_with_subscription(dir: &tempfile::TempDir) -> SubscribedStack {
            let redb: Arc<dyn MapDataStore> =
                Arc::new(RedbDataStore::new(dir.path().join("nonres.redb")).expect("redb open"));
            let parking = Arc::new(ParkingStore::new(redb));
            let factory = Arc::new(RecordStoreFactory::new(
                StorageConfig::default(),
                parking.clone() as Arc<dyn MapDataStore>,
                Vec::new(),
            ));
            let conn_registry = Arc::new(ConnectionRegistry::new());
            let query_registry = Arc::new(QueryRegistry::new());
            let listener = subscribe_listener(&conn_registry, &query_registry, MAP);
            let journal = Arc::new(JournalStore::new(100));
            let svc = Arc::new(
                CrdtService::new(
                    Arc::clone(&factory),
                    conn_registry,
                    make_validator(),
                    query_registry,
                    Arc::new(SchemaService::new()),
                )
                .with_journal(Arc::clone(&journal)),
            );
            SubscribedStack {
                svc,
                factory,
                parking,
                journal,
                _listener: listener,
            }
        }

        // A write must not land after a write of the same key that carries a
        // higher stamp. The only point between minting a stamp and applying it
        // where a write can be overtaken is the read of the previous value,
        // taken when the map has a query subscription and the key is not
        // resident; the first writer is parked there.
        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn a_lower_stamped_put_never_lands_after_a_higher_stamped_one() {
            let dir = tempfile::tempdir().expect("tempdir");
            let SubscribedStack {
                svc,
                factory,
                parking,
                journal,
                _listener,
            } = parking_stack_with_subscription(&dir);
            assert_not_resident(&factory);

            let mut park = parking.park_after_load();
            let x = tokio::spawn(svc.clone().oneshot(route_x(VX)));
            // X is parked in the load of the previous value.
            park.wait_parked().await;

            let y = tokio::spawn(svc.clone().oneshot(route_y(VY)));
            let y_waited = second_writer_waited(&svc, &y).await;
            park.release();
            x.await.expect("X task").expect("X must succeed");
            y.await.expect("Y task").expect("Y must succeed");

            let journal_order = journal_stamps(&journal);
            assert_eq!(
                journal_order.len(),
                2,
                "the journal must hold both writes of the key"
            );
            let a = journal_order.iter().min().expect("two stamps").clone();
            let b = journal_order.iter().max().expect("two stamps").clone();
            assert!(a < b, "the two writes must carry distinct stamps");
            let engine_stamp = read_lww_timestamp(&factory, MAP, KEY).await;

            println!(
                "lower_stamped_put: y_waited={y_waited} a={a:?} b={b:?} engine={engine_stamp:?} \
                 journal={journal_order:?}"
            );
            assert_eq!(
                (engine_stamp, journal_order),
                (Some(b.clone()), vec![a, b]),
                "(engine stamp, journal order): the engine must hold the higher stamp and the \
                 journal must record the two writes in stamp order"
            );
        }

        // The same pair of routes over the buffered store, with the first
        // writer parked between taking its entry sequence and entering the
        // queue. What a read through the buffered store answers before the
        // flush, what the flush writes and what is recovered after a restart
        // must all be the value of the write that started later.
        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn two_routes_writing_one_key_leave_store_staging_and_restart_on_the_later_stamp() {
            use crate::storage::datastores::WalBootstrap;
            use crate::storage::wal::{Wal, WalFsyncPolicy, WalRecovery, WalWriter};

            let dir = tempfile::tempdir().expect("tempdir");
            let wal_dir = tempfile::tempdir().expect("wal tempdir");
            let redb_path = dir.path().join("two_routes.redb");
            let journal = Arc::new(JournalStore::new(100));

            let (inner, staged_read, y_waited) = {
                let redb: Arc<dyn MapDataStore> =
                    Arc::new(RedbDataStore::new(&redb_path).expect("redb open"));
                // Every frame is fsynced before its write returns, and the
                // delays keep the flush loop from draining by itself.
                let wal = WalWriter::new(wal_dir.path().to_path_buf(), WalFsyncPolicy::PerOp)
                    .expect("wal open");
                let write_behind = WriteBehindDataStore::new_with_wal(
                    Arc::clone(&redb),
                    WriteBehindConfig {
                        write_delay_ms: 600_000,
                        flush_interval_ms: 600_000,
                        ..WriteBehindConfig::default()
                    },
                    Some(WalBootstrap {
                        wal: Arc::clone(&wal) as Arc<dyn Wal>,
                        sequence_start: 1,
                    }),
                );
                let factory = Arc::new(RecordStoreFactory::new(
                    StorageConfig::default(),
                    Arc::clone(&write_behind) as Arc<dyn MapDataStore>,
                    Vec::new(),
                ));
                let svc = Arc::new(
                    CrdtService::new(
                        factory,
                        Arc::new(ConnectionRegistry::new()),
                        make_validator(),
                        Arc::new(QueryRegistry::new()),
                        Arc::new(SchemaService::new()),
                    )
                    .with_journal(Arc::clone(&journal)),
                );

                let mut park = write_behind.test_park_before_queue_insert(MAP, KEY);
                let x = tokio::spawn(svc.clone().oneshot(route_x(VX)));
                // X holds the lower entry sequence and is in no queue yet.
                park.wait_parked().await;

                let y = tokio::spawn(svc.clone().oneshot(route_y(VY)));
                let y_waited = second_writer_waited(&svc, &y).await;
                park.release();
                x.await.expect("X task").expect("X must succeed");
                y.await.expect("Y task").expect("Y must succeed");

                // Taken before the flush: the flush empties the staging slot,
                // and a read after it only repeats the inner store's answer.
                let staged_read = lww_text(write_behind.load(MAP, KEY).await.expect("staged read"));
                write_behind.hard_flush().await.expect("hard_flush");
                let inner = lww_text(redb.load(MAP, KEY).await.expect("inner read"));
                (inner, staged_read, y_waited)
            };

            // Restart: everything above is dropped; the same redb file and WAL
            // directory are reopened and recovery runs over them.
            let after_restart = {
                let redb: Arc<dyn MapDataStore> =
                    Arc::new(RedbDataStore::new(&redb_path).expect("redb reopen"));
                let wal = WalWriter::new(wal_dir.path().to_path_buf(), WalFsyncPolicy::PerOp)
                    .expect("wal reopen");
                WalRecovery::new(Arc::clone(&wal), Vec::new())
                    .run(Arc::clone(&redb))
                    .await
                    .expect("recovery");
                lww_text(redb.load(MAP, KEY).await.expect("read after restart"))
            };

            let stamps = journal_stamps(&journal);
            println!(
                "two_routes/write_behind: y_waited={y_waited} inner={inner:?} \
                 staged_read={staged_read:?} after_restart={after_restart:?} journal={stamps:?}"
            );
            assert_eq!(
                (
                    inner.as_deref(),
                    staged_read.as_deref(),
                    after_restart.as_deref()
                ),
                (Some(VY), Some(VY), Some(VY)),
                "(inner store, read through the buffered store before the flush, inner store \
                 after a restart): the write that started later holds the later stamp, and all \
                 three must hold its value"
            );
            assert!(
                stamps.len() == 2 && stamps[0] < stamps[1],
                "the journal must record the two writes of the key in stamp order; got {stamps:?}"
            );
            assert!(
                y_waited,
                "the second writer must have waited for the key's writer while the first was parked"
            );
        }

        // ---- Two writers of one key and a refused flush -------------------
        //
        // The buffered store hands the WAL frames of a refused entry to the
        // entry queued behind it for the same key. That is sound only when the
        // queued entry is the newer write of the key. The two tests below let
        // the inner store refuse one attempt while two writes of one key are
        // under way, and then read what a crash at that moment would leave:
        // the inner store as it stands plus whatever recovery replays from the
        // WAL. As above, no sleep orders anything.

        /// What a test decides for one parked store call.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        enum Verdict {
            /// Record the value and return `Ok`.
            Accept,
            /// Return `Err` and record nothing.
            Refuse,
        }

        /// A store call that is waiting for its verdict.
        struct ParkedCall {
            /// The stamp of the value the call carries; `None` for a delete.
            stamp: Option<Timestamp>,
            verdict: tokio::sync::oneshot::Sender<Verdict>,
        }

        #[derive(Default)]
        struct VerdictGate {
            parked: Option<ParkedCall>,
            /// The stamp each refused call carried, in the order of refusal.
            refused: Vec<Option<Timestamp>>,
        }

        fn lww_stamp(value: Option<&RecordValue>) -> Option<Timestamp> {
            match value {
                Some(RecordValue::Lww { timestamp, .. }) => Some(timestamp.clone()),
                _ => None,
            }
        }

        /// An in-memory store that RETAINS what it is told, so that a lost
        /// write and a recovered one read differently. For one key it parks
        /// every `add` and `remove` until the test grants a verdict: a refusing
        /// store alone could not place a second write inside the flush window,
        /// because the window is open only while the flush loop holds a drained
        /// entry. It can also park ONE `load` of that key on return. An
        /// instance that gates no key is a plain store, which is what the
        /// crash image is.
        struct RefusingStore {
            /// `(map, key)` -> value, or `None` for a delete.
            data: std::sync::Mutex<HashMap<(String, String), Option<RecordValue>>>,
            /// The key whose store calls park; `None` parks nothing.
            gated: Option<(String, String)>,
            gate: std::sync::Mutex<VerdictGate>,
            after_load: std::sync::Mutex<Option<Gate>>,
        }

        impl RefusingStore {
            fn with(
                data: HashMap<(String, String), Option<RecordValue>>,
                gated: Option<(String, String)>,
            ) -> Self {
                Self {
                    data: std::sync::Mutex::new(data),
                    gated,
                    gate: std::sync::Mutex::new(VerdictGate::default()),
                    after_load: std::sync::Mutex::new(None),
                }
            }

            /// An empty store whose every store call of `(map, key)` parks.
            fn gating(map: &str, key: &str) -> Self {
                Self::with(HashMap::new(), Some((map.to_string(), key.to_string())))
            }

            /// What a crash would leave of `live`: its content, and no gate.
            fn image_of(live: &Self) -> Self {
                Self::with(live.data.lock().unwrap().clone(), None)
            }

            fn is_gated(&self, map: &str, key: &str) -> bool {
                self.gated
                    .as_ref()
                    .is_some_and(|(gated_map, gated_key)| gated_map == map && gated_key == key)
            }

            /// Parks the next `load` of the gated key on return.
            fn park_after_load(&self) -> ParkHandle {
                ParkingStore::arm(&self.after_load)
            }

            fn has_parked_call(&self) -> bool {
                self.gate.lock().unwrap().parked.is_some()
            }

            /// The stamp carried by the store call that is parked right now;
            /// `None` when no call is parked or the parked one is a delete.
            fn parked_stamp(&self) -> Option<Timestamp> {
                self.gate
                    .lock()
                    .unwrap()
                    .parked
                    .as_ref()
                    .and_then(|call| call.stamp.clone())
            }

            fn refused(&self) -> Vec<Option<Timestamp>> {
                self.gate.lock().unwrap().refused.clone()
            }

            /// Answers the parked call. The parked mark is cleared here rather
            /// than by the released call, so a `settle` issued right after
            /// cannot take that call for a new one.
            fn grant(&self, verdict: Verdict) {
                let call = self
                    .gate
                    .lock()
                    .unwrap()
                    .parked
                    .take()
                    .expect("a verdict was granted with no store call parked");
                let _ = call.verdict.send(verdict);
            }

            /// Parks the calling store call until the test grants its verdict.
            async fn verdict_for(&self, map: &str, key: &str, stamp: Option<Timestamp>) -> Verdict {
                if !self.is_gated(map, key) {
                    return Verdict::Accept;
                }
                let (verdict_tx, verdict_rx) = tokio::sync::oneshot::channel();
                {
                    let mut gate = self.gate.lock().unwrap();
                    assert!(
                        gate.parked.is_none(),
                        "two store calls of the key reached the inner store at once"
                    );
                    gate.parked = Some(ParkedCall {
                        stamp: stamp.clone(),
                        verdict: verdict_tx,
                    });
                }
                // The sender lives in this store, which the call borrows, so
                // the channel cannot close under it; a closed one is accepted.
                let verdict = verdict_rx.await.unwrap_or(Verdict::Accept);
                if verdict == Verdict::Refuse {
                    self.gate.lock().unwrap().refused.push(stamp);
                }
                verdict
            }

            /// Waits until a store call of the key is parked. The flush loop
            /// drains a queued entry by itself, so this is reached whenever an
            /// entry of the key is queued; the bound only reports that none was.
            async fn wait_store_call_parked(&self) {
                tokio::time::timeout(PARK_BOUND, async {
                    while !self.has_parked_call() {
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .expect("no store call of the key reached the inner store");
            }

            /// Where the buffered store stands once nothing further happens
            /// without the test: `true` when a store call is parked, `false`
            /// when the store is idle. A flush pass stays open for as long as
            /// one of its calls is parked or running, so idle cannot be
            /// reported in front of a call that is still to come; the writers
            /// must have returned.
            async fn settle(&self, write_behind: &WriteBehindDataStore) -> bool {
                tokio::time::timeout(PARK_BOUND, async {
                    loop {
                        if self.has_parked_call() {
                            return true;
                        }
                        if write_behind.test_is_idle() {
                            return false;
                        }
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .expect("the buffered store neither parked a store call nor went idle")
            }

            /// Refuses the first parked store call of the key, accepts every
            /// later one, and returns once the buffered store is idle.
            async fn refuse_first_then_accept(&self, write_behind: &WriteBehindDataStore) {
                let mut verdict = Verdict::Refuse;
                while self.settle(write_behind).await {
                    self.grant(verdict);
                    verdict = Verdict::Accept;
                }
            }
        }

        #[async_trait]
        impl MapDataStore for RefusingStore {
            async fn add(
                &self,
                map: &str,
                key: &str,
                value: &RecordValue,
                _expiration_time: i64,
                _now: i64,
            ) -> anyhow::Result<()> {
                if self.verdict_for(map, key, lww_stamp(Some(value))).await == Verdict::Refuse {
                    anyhow::bail!("the inner store refused the write of key={key}");
                }
                self.data
                    .lock()
                    .unwrap()
                    .insert((map.to_string(), key.to_string()), Some(value.clone()));
                Ok(())
            }

            async fn add_backup(
                &self,
                _map: &str,
                _key: &str,
                _value: &RecordValue,
                _expiration_time: i64,
                _now: i64,
            ) -> anyhow::Result<()> {
                Ok(())
            }

            async fn remove(&self, map: &str, key: &str, _now: i64) -> anyhow::Result<()> {
                if self.verdict_for(map, key, None).await == Verdict::Refuse {
                    anyhow::bail!("the inner store refused the delete of key={key}");
                }
                self.data
                    .lock()
                    .unwrap()
                    .insert((map.to_string(), key.to_string()), None);
                Ok(())
            }

            async fn remove_backup(&self, _map: &str, _key: &str, _now: i64) -> anyhow::Result<()> {
                Ok(())
            }

            async fn load(&self, map: &str, key: &str) -> anyhow::Result<Option<RecordValue>> {
                let loaded = self
                    .data
                    .lock()
                    .unwrap()
                    .get(&(map.to_string(), key.to_string()))
                    .cloned()
                    .flatten();
                if self.is_gated(map, key) {
                    ParkingStore::pass(&self.after_load).await;
                }
                Ok(loaded)
            }

            async fn load_all(
                &self,
                _map: &str,
                _keys: &[String],
            ) -> anyhow::Result<Vec<(String, RecordValue)>> {
                Ok(Vec::new())
            }

            async fn enumerate_leaves(
                &self,
                _map: &str,
                _is_backup: bool,
                _sink: &mut dyn LeafSink,
            ) -> anyhow::Result<()> {
                Ok(())
            }

            async fn scan_values(
                &self,
                _map: &str,
                _is_backup: bool,
                _max_batch_cost: u64,
            ) -> anyhow::Result<ScanBatch> {
                Ok(ScanBatch::default())
            }

            async fn scan_values_batched(
                &self,
                _map: &str,
                _is_backup: bool,
                _cursor: ScanCursor,
                _max_batch_cost: u64,
            ) -> anyhow::Result<ScanBatch> {
                Ok(ScanBatch::default())
            }

            async fn remove_all(&self, map: &str, keys: &[String]) -> anyhow::Result<()> {
                for key in keys {
                    self.remove(map, key, 0).await?;
                }
                Ok(())
            }

            fn is_loadable(&self, _key: &str) -> bool {
                true
            }

            fn pending_operation_count(&self) -> u64 {
                0
            }

            async fn soft_flush(&self) -> anyhow::Result<u64> {
                Ok(0)
            }

            async fn hard_flush(&self) -> anyhow::Result<()> {
                Ok(())
            }

            async fn flush_key(
                &self,
                map: &str,
                key: &str,
                value: &RecordValue,
                _is_backup: bool,
            ) -> anyhow::Result<()> {
                self.add(map, key, value, 0, 0).await
            }

            fn reset(&self) {}
        }

        /// What [`refusing_stack`] builds.
        struct RefusingStack {
            svc: Arc<CrdtService>,
            factory: Arc<RecordStoreFactory>,
            write_behind: Arc<WriteBehindDataStore>,
            wal: Arc<crate::storage::wal::WalWriter>,
            inner: Arc<RefusingStore>,
            journal: Arc<JournalStore>,
            /// Keeps the subscribed connection's channel open, when there is one.
            _listener:
                Option<tokio::sync::mpsc::Receiver<crate::network::connection::OutboundMessage>>,
        }

        /// The service over the buffered store with a real WAL, on a
        /// [`RefusingStore`] gating `(MAP, KEY)`, with a journal attached.
        /// Every frame is fsynced before its write returns, the flush loop
        /// drains by itself within milliseconds, and a refused entry is retried
        /// rather than discarded. `subscribed` registers one query subscription
        /// on the map, so a write reads the key's previous value before it
        /// applies.
        fn refusing_stack(wal_dir: &tempfile::TempDir, subscribed: bool) -> RefusingStack {
            use crate::storage::datastores::WalBootstrap;
            use crate::storage::wal::{Wal, WalFsyncPolicy, WalWriter};

            let inner = Arc::new(RefusingStore::gating(MAP, KEY));
            let wal = WalWriter::new(wal_dir.path().to_path_buf(), WalFsyncPolicy::PerOp)
                .expect("wal open");
            let write_behind = WriteBehindDataStore::new_with_wal(
                Arc::clone(&inner) as Arc<dyn MapDataStore>,
                WriteBehindConfig {
                    write_delay_ms: 0,
                    flush_interval_ms: 5,
                    max_retries: 3,
                    backoff_base_ms: 1,
                    backoff_cap_ms: 2,
                    ..WriteBehindConfig::default()
                },
                Some(WalBootstrap {
                    wal: Arc::clone(&wal) as Arc<dyn Wal>,
                    sequence_start: 1,
                }),
            );
            let factory = Arc::new(RecordStoreFactory::new(
                StorageConfig::default(),
                Arc::clone(&write_behind) as Arc<dyn MapDataStore>,
                Vec::new(),
            ));
            let conn_registry = Arc::new(ConnectionRegistry::new());
            let query_registry = Arc::new(QueryRegistry::new());
            let listener =
                subscribed.then(|| subscribe_listener(&conn_registry, &query_registry, MAP));
            let journal = Arc::new(JournalStore::new(100));
            let svc = Arc::new(
                CrdtService::new(
                    Arc::clone(&factory),
                    conn_registry,
                    make_validator(),
                    query_registry,
                    Arc::new(SchemaService::new()),
                )
                .with_journal(Arc::clone(&journal)),
            );
            RefusingStack {
                svc,
                factory,
                write_behind,
                wal,
                inner,
                journal,
                _listener: listener,
            }
        }

        /// The key's stamp after a crash at this moment: the inner store's
        /// content is copied into a fresh image, recovery runs over the same
        /// WAL into that image, and the key is read from it.
        async fn stamp_recovered_after_a_crash(
            inner: &RefusingStore,
            wal: &Arc<crate::storage::wal::WalWriter>,
        ) -> Option<Timestamp> {
            use crate::storage::wal::WalRecovery;

            let image = Arc::new(RefusingStore::image_of(inner));
            WalRecovery::new(Arc::clone(wal), Vec::new())
                .run(Arc::clone(&image) as Arc<dyn MapDataStore>)
                .await
                .expect("recovery");
            lww_stamp(image.load(MAP, KEY).await.expect("read the image").as_ref())
        }

        // Two routes write one key while the first writer is parked between
        // taking its entry sequence and entering the queue, and the first
        // flush attempt of the key is refused. Whichever entry the refusal
        // meets, the entry queued behind it must be the newer write: what is
        // served and what a crash recovers must both be the higher stamp.
        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn two_routes_and_a_refused_flush_recover_the_later_stamp_after_a_crash() {
            let wal_dir = tempfile::tempdir().expect("wal tempdir");
            let RefusingStack {
                svc,
                factory,
                write_behind,
                wal,
                inner,
                journal,
                _listener,
            } = refusing_stack(&wal_dir, false);

            let mut park = write_behind.test_park_before_queue_insert(MAP, KEY);
            let x = tokio::spawn(svc.clone().oneshot(route_x(VX)));
            // X has its stamp, its engine write and its WAL frame, holds the
            // lower entry sequence and is in no queue yet.
            park.wait_parked().await;

            let y = tokio::spawn(svc.clone().oneshot(route_y(VY)));
            let y_waited = second_writer_waited(&svc, &y).await;
            if !y_waited {
                // Y returned, so its entry is queued alone. Its flush attempt
                // must be in flight before X is let into the queue: only then
                // does X arrive as a new entry behind it.
                inner.wait_store_call_parked().await;
            }
            park.release();
            x.await.expect("X task").expect("X must succeed");
            y.await.expect("Y task").expect("Y must succeed");

            inner.refuse_first_then_accept(&write_behind).await;
            let refused = inner.refused();
            assert_eq!(
                refused.len(),
                1,
                "exactly one store call of the key must have been refused; got {refused:?}"
            );

            let served = read_lww_timestamp(&factory, MAP, KEY).await;
            let recovered = stamp_recovered_after_a_crash(&inner, &wal).await;

            let journal_order = journal_stamps(&journal);
            assert_eq!(
                journal_order.len(),
                2,
                "the journal must hold both writes of the key"
            );
            let a = journal_order.iter().min().expect("two stamps").clone();
            let b = journal_order.iter().max().expect("two stamps").clone();
            assert!(a < b, "the two writes must carry distinct stamps");

            println!(
                "two_routes/refused_flush: y_waited={y_waited} served={served:?} \
                 recovered={recovered:?} a={a:?} b={b:?} journal={journal_order:?} \
                 refused_calls={} refused={refused:?}",
                refused.len()
            );
            assert_eq!(
                (served, recovered),
                (Some(b.clone()), Some(b.clone())),
                "(stamp served by the live store, stamp recovered after a crash): both must be \
                 the higher of the two stamps"
            );
            assert!(
                journal_order == vec![a, b],
                "the journal must record the two writes of the key in stamp order"
            );
            assert!(
                y_waited,
                "the second writer must have waited for the key's writer while the first was parked"
            );
        }

        // A writer is parked in the read of the key's previous value while a
        // second write of the key completes and its flush attempt is in
        // flight; that attempt is then refused, with the parked writer's entry
        // queued behind it. The entry that takes over the refused one's frames
        // must not hold an older value: what is served and what a crash
        // recovers must both be the higher stamp.
        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn an_older_value_arriving_behind_a_refused_newer_one_does_not_win_after_a_crash() {
            let wal_dir = tempfile::tempdir().expect("wal tempdir");
            let RefusingStack {
                svc,
                factory,
                write_behind,
                wal,
                inner,
                journal,
                _listener,
            } = refusing_stack(&wal_dir, true);
            assert_not_resident(&factory);

            let mut park = inner.park_after_load();
            let o = tokio::spawn(svc.clone().oneshot(route_x(VX)));
            // O is parked in the load of the previous value.
            park.wait_parked().await;

            // The park is one-shot, so N's own read passes; N writes, queues
            // and returns.
            let n = tokio::spawn(svc.clone().oneshot(route_y(VY)));
            n.await.expect("N task").expect("N must succeed");
            let journal_after_n = journal_stamps(&journal);
            assert_eq!(
                journal_after_n.len(),
                1,
                "only N may have written while O is parked; got {journal_after_n:?}"
            );
            // N's entry is drained and its flush attempt is in flight, so O's
            // entry will be queued behind it as a new one.
            inner.wait_store_call_parked().await;
            let in_flight = inner.parked_stamp();
            assert_eq!(
                in_flight,
                Some(journal_after_n[0].clone()),
                "the store call parked before O is released must carry N's value"
            );
            park.release();
            o.await.expect("O task").expect("O must succeed");

            inner.refuse_first_then_accept(&write_behind).await;
            let refused = inner.refused();
            assert_eq!(
                refused.len(),
                1,
                "exactly one store call of the key must have been refused; got {refused:?}"
            );

            let served = read_lww_timestamp(&factory, MAP, KEY).await;
            let recovered = stamp_recovered_after_a_crash(&inner, &wal).await;

            let journal_order = journal_stamps(&journal);
            assert_eq!(
                journal_order.len(),
                2,
                "the journal must hold both writes of the key"
            );
            let a = journal_order.iter().min().expect("two stamps").clone();
            let b = journal_order.iter().max().expect("two stamps").clone();
            assert!(a < b, "the two writes must carry distinct stamps");

            println!(
                "older_behind_refused_newer: served={served:?} recovered={recovered:?} a={a:?} \
                 b={b:?} journal={journal_order:?} refused_calls={} refused={refused:?} \
                 in_flight_before_release={in_flight:?}",
                refused.len()
            );
            assert_eq!(
                (served, recovered),
                (Some(b.clone()), Some(b.clone())),
                "(stamp served by the live store, stamp recovered after a crash): both must be \
                 the higher of the two stamps"
            );
            assert!(
                journal_order == vec![a, b],
                "the journal must record the two writes of the key in stamp order"
            );
        }

        // An OR push arrives through the sync service and an LWW put through
        // the CRDT service. Parked inside its store write, the push must hold
        // the key's writer the two services share, so the put of the same key
        // waits for it and the engine and the store end on the same value.
        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn an_lww_put_waits_for_a_parked_or_push_of_the_same_key() {
            let dir = tempfile::tempdir().expect("tempdir");
            let (svc, factory, redb, parking) = parking_stack(&dir);
            let sync = sync_sharing_writer(&svc, &factory);

            let mut park = parking.park_before_add();
            let pusher = tokio::spawn(
                sync.clone()
                    .oneshot(push_op(vec![push_entry(KEY, &PUSHED)])),
            );
            // The push has merged its entries into the engine and is parked on
            // entry to its store write.
            park.wait_parked().await;

            let put = tokio::spawn(svc.clone().oneshot(route_x(VY)));
            let put_waited = second_writer_waited(&svc, &put).await;
            park.release();
            pusher.await.expect("push task").expect("push must succeed");
            put.await.expect("put task").expect("put must succeed");

            let engine = factory
                .get_or_create(MAP, hash_to_partition(KEY))
                .get(KEY, false)
                .await
                .expect("engine read")
                .map(|record| record.value);
            let store = redb.load(MAP, KEY).await.expect("store read");
            // Compared in serialised form: `RecordValue` has no `PartialEq`.
            let engine_equals_store = rmp_serde::to_vec_named(&engine).expect("encode engine")
                == rmp_serde::to_vec_named(&store).expect("encode store");
            let kind = |value: &Option<RecordValue>| match value {
                Some(RecordValue::Lww { .. }) => "lww",
                Some(RecordValue::OrMap { .. }) => "or-map",
                Some(RecordValue::OrTombstones { .. }) => "or-tombstones",
                None => "absent",
            };

            println!(
                "pair_d: put_waited={put_waited} engine_equals_store={engine_equals_store} \
                 engine_kind={} store_kind={}",
                kind(&engine),
                kind(&store)
            );
            assert_eq!(
                (put_waited, engine_equals_store),
                (true, true),
                "(the put waited for the key's writer, engine == store): the push holds the \
                 writer the two services share across its store write"
            );
        }
    }

    // -----------------------------------------------------------------------
    // OR-tag admissibility at ingest (TG-MRK-001)
    //
    // The OR-Map Merkle leaf joins a key's tags with `|` and splits its live tags
    // from its tombstones with `#`. The leaf identifies one (live set, tombstone
    // set) per key only while every stored tag is non-empty and carries neither
    // character, so the op path must refuse such a tag wherever it would be
    // stored verbatim, and must keep a tag the slot already holds removable.
    // -----------------------------------------------------------------------

    /// `Ok` with the refusal's one error string when `result` is the refusal of
    /// the inadmissible `tag` on `key`; `Err` with what is wrong otherwise.
    ///
    /// Pins what a caller can rely on: the typed error, the map, the key and
    /// the reason. The wording around them is left free.
    fn inadmissible_tag_refusal(
        result: &Result<OperationResponse, OperationError>,
        map: &str,
        key: &str,
        tag: &str,
    ) -> Result<String, String> {
        let reason = if tag.is_empty() {
            "OR tag is empty"
        } else {
            "OR tag contains a reserved character ('|' or '#')"
        };
        match result {
            Err(OperationError::SchemaInvalid { map_name, errors }) => {
                let [only] = errors.as_slice() else {
                    return Err(format!(
                        "SchemaInvalid with {} error strings, want one: {errors:?}",
                        errors.len()
                    ));
                };
                let mut problems: Vec<String> = Vec::new();
                if map_name != map {
                    problems.push(format!("names map {map_name:?}, want {map:?}"));
                }
                if !only.contains(key) {
                    problems.push(format!("does not name key {key:?}"));
                }
                if !only.contains(reason) {
                    problems.push(format!("does not give the reason {reason:?}"));
                }
                // A refused tag is attacker-chosen text and must not be echoed.
                // The key and the reason are taken out first: a tag as short as
                // one separator character is part of the reason itself.
                let residue = only.replace(reason, "").replace(key, "");
                if !tag.is_empty() && residue.contains(tag) {
                    problems.push(format!("echoes the refused tag {tag:?}"));
                }
                if problems.is_empty() {
                    Ok(only.clone())
                } else {
                    Err(format!("SchemaInvalid {only:?} {}", problems.join(", ")))
                }
            }
            Err(other) => Err(format!(
                "Err({other:?}), want Err(SchemaInvalid) with reason {reason:?}"
            )),
            Ok(_) => Err(format!(
                "Ok (the op was applied), want Err(SchemaInvalid) with reason {reason:?}"
            )),
        }
    }

    /// A compact picture of what `key`'s slot holds, for before/after comparison.
    async fn slot_image(factory: &Arc<RecordStoreFactory>, map: &str, key: &str) -> String {
        let store = factory.get_or_create(map, hash_to_partition(key));
        match store
            .get(key, false)
            .await
            .expect("read the slot")
            .map(|record| record.value)
        {
            None => "absent".to_string(),
            Some(RecordValue::OrMap {
                records,
                tombstones,
            }) => {
                let mut live: Vec<String> = records.into_iter().map(|entry| entry.tag).collect();
                live.sort();
                let mut tombstones = tombstones;
                tombstones.sort();
                format!("live {live:?} / tombstones {tombstones:?}")
            }
            Some(RecordValue::OrTombstones { mut tags }) => {
                tags.sort();
                format!("legacy tombstones {tags:?}")
            }
            Some(lww @ RecordValue::Lww { .. }) => format!("{lww:?}"),
        }
    }

    /// Writes `value` into `key`'s slot through the record store, bypassing the
    /// service, the way a slot written before the rule existed looks.
    async fn seed_slot(
        factory: &Arc<RecordStoreFactory>,
        map: &str,
        key: &str,
        value: RecordValue,
    ) {
        factory
            .get_or_create(map, hash_to_partition(key))
            .put(key, value, ExpiryPolicy::NONE, CallerProvenance::CrdtMerge)
            .await
            .expect("seed the slot through the store");
    }

    fn live_or_slot(tag: &str) -> RecordValue {
        RecordValue::OrMap {
            records: vec![OrMapEntry {
                value: Value::String("seeded".into()),
                tag: tag.to_string(),
                timestamp: make_timestamp(),
            }],
            tombstones: Vec::new(),
        }
    }

    /// An inadmissible tag is refused at every admission site before anything of
    /// its operation is applied, and only where the tag would be stored: a
    /// regenerated `OR_ADD` and a whole-key REMOVE store no client tag and stay
    /// accepted (TG-MRK-001).
    #[tokio::test]
    #[allow(clippy::too_many_lines)] // one table over the eight admission sites plus the guards, asserted once
    async fn or_op_with_an_inadmissible_tag_is_refused_before_the_batch_applies() {
        use topgun_core::messages::sync::OpBatchPayload;

        const MAP: &str = "tag-admission";

        #[derive(Clone, Copy)]
        enum Origin {
            Connection,
            Http,
            Anonymous,
            Trusted,
        }

        #[derive(Clone, Copy)]
        enum Form {
            ClientOp,
            OpBatch,
        }

        fn bare(key: &str) -> ClientOp {
            ClientOp {
                id: Some(format!("op-{key}")),
                map_name: MAP.to_string(),
                key: key.to_string(),
                op_type: None,
                record: None,
                or_record: None,
                or_tag: None,
                write_concern: None,
                timeout: None,
            }
        }

        fn or_remove(key: &str, tag: &str) -> ClientOp {
            ClientOp {
                or_tag: Some(Some(tag.to_string())),
                ..bare(key)
            }
        }

        fn or_add(key: &str, tag: &str) -> ClientOp {
            ClientOp {
                or_record: Some(Some(topgun_core::ORMapRecord {
                    value: rmpv::Value::String("value".into()),
                    timestamp: make_timestamp(),
                    tag: tag.to_string(),
                    ttl_ms: None,
                })),
                ..bare(key)
            }
        }

        fn batch(ctx: OperationContext, ops: Vec<ClientOp>) -> Operation {
            Operation::OpBatch {
                ctx,
                payload: OpBatchMessage {
                    payload: OpBatchPayload {
                        ops,
                        write_concern: None,
                        timeout: None,
                    },
                },
            }
        }

        fn send(form: Form, ctx: OperationContext, op: ClientOp) -> Operation {
            match form {
                Form::ClientOp => Operation::ClientOp {
                    ctx,
                    payload: ClientOpMessage { payload: op },
                },
                Form::OpBatch => batch(ctx, vec![op]),
            }
        }

        let factory = make_factory();
        let registry = Arc::new(ConnectionRegistry::new());
        let frontier = Arc::new(TombstoneFrontier::new(None));
        // One epoch per stamped tombstone, so a tombstone that slips in is also
        // visible as a moved epoch.
        frontier.set_epoch_width(1);
        let svc = Arc::new(
            CrdtService::new(
                Arc::clone(&factory),
                Arc::clone(&registry),
                make_validator(),
                Arc::new(QueryRegistry::new()),
                Arc::new(SchemaService::new()),
            )
            .with_frontier(Arc::clone(&frontier)),
        );
        let (handle, _rx) = registry.register(
            ConnectionKind::Client,
            &crate::network::config::ConnectionConfig::default(),
        );
        handle.metadata.write().await.authenticated = true;
        let conn_id = handle.id;

        let ctx_for = |origin: Origin, key: &str| -> OperationContext {
            match origin {
                Origin::Connection => {
                    let mut ctx = make_ctx_for_key(key);
                    ctx.connection_id = Some(conn_id);
                    ctx
                }
                Origin::Http => make_http_ctx_for_key(key),
                Origin::Anonymous => make_anon_http_ctx_for_key(key),
                // No connection id and a plain `Client` origin: the branch that
                // keeps the caller's timestamp and tag verbatim.
                Origin::Trusted => make_ctx_for_key(key),
            }
        };

        let origins = [
            (Origin::Connection, "client connection"),
            (Origin::Http, "HttpClient"),
            (Origin::Anonymous, "Anonymous"),
            (Origin::Trusted, "trusted"),
        ];
        let forms = [(Form::ClientOp, "ClientOp"), (Form::OpBatch, "OpBatch")];

        // Every sub-case is evaluated and the test asserts once, so a single run
        // names every admission site that lets a tag through.
        let mut failures: Vec<String> = Vec::new();
        let mut checked = 0_usize;

        // (i) Four origins x two operations are the eight admission sites. An
        // OR_REMOVE stores its tag verbatim as a tombstone on every one of them.
        for (o, (origin, origin_name)) in origins.into_iter().enumerate() {
            for (f, (form, form_name)) in forms.into_iter().enumerate() {
                for (t, tag) in ["a|b", "a#b", ""].into_iter().enumerate() {
                    checked += 1;
                    let key = format!("i-{o}-{f}-{t}");
                    // A populated slot makes "unchanged" a real comparison.
                    seed_slot(&factory, MAP, &key, live_or_slot("seed-tag")).await;
                    let slot_before = slot_image(&factory, MAP, &key).await;
                    let epoch_before = frontier.current_epoch();

                    let result = Arc::clone(&svc)
                        .oneshot(send(form, ctx_for(origin, &key), or_remove(&key, tag)))
                        .await;

                    let mut problems: Vec<String> = Vec::new();
                    if let Err(problem) = inadmissible_tag_refusal(&result, MAP, &key, tag) {
                        problems.push(problem);
                    }
                    let slot_after = slot_image(&factory, MAP, &key).await;
                    if slot_after != slot_before {
                        problems.push(format!("the slot changed to {slot_after}"));
                    }
                    let epoch_after = frontier.current_epoch();
                    if epoch_after != epoch_before {
                        problems.push(format!(
                            "current_epoch() moved {epoch_before} -> {epoch_after}"
                        ));
                    }
                    if !problems.is_empty() {
                        failures.push(format!(
                            "(i) {origin_name} / {form_name}, OR_REMOVE or_tag {tag:?}: {}",
                            problems.join("; ")
                        ));
                    }
                }
            }
        }

        // (ii) One operation, an admissible op ahead of the refused one: the
        // refusal must be decided before the first apply, or the per-op
        // re-dispatch that follows a refused batch would apply key X twice.
        for (o, (origin, origin_name)) in origins.into_iter().enumerate() {
            checked += 1;
            let (key_x, key_y) = (format!("ii-{o}-x"), format!("ii-{o}-y"));
            let result = Arc::clone(&svc)
                .oneshot(batch(
                    ctx_for(origin, &key_x),
                    vec![or_add(&key_x, "1:0:n-1"), or_remove(&key_y, "a|b")],
                ))
                .await;

            let mut problems: Vec<String> = Vec::new();
            if let Err(problem) = inadmissible_tag_refusal(&result, MAP, &key_y, "a|b") {
                problems.push(problem);
            }
            let slot_x = slot_image(&factory, MAP, &key_x).await;
            if slot_x != "absent" {
                problems.push(format!("key X was written: {slot_x}"));
            }
            let slot_y = slot_image(&factory, MAP, &key_y).await;
            if slot_y != "absent" {
                problems.push(format!("key Y was written: {slot_y}"));
            }
            if !problems.is_empty() {
                failures.push(format!(
                    "(ii) {origin_name} / one OpBatch [OR_ADD X, OR_REMOVE \"a|b\" on Y]: {}",
                    problems.join("; ")
                ));
            }
        }

        // (iii) The trusted branch keeps an OR_ADD's record tag verbatim, so it
        // is an ingest path too.
        for (f, (form, form_name)) in forms.into_iter().enumerate() {
            checked += 1;
            let key = format!("iii-{f}");
            let result = Arc::clone(&svc)
                .oneshot(send(
                    form,
                    ctx_for(Origin::Trusted, &key),
                    or_add(&key, "a#b"),
                ))
                .await;

            let mut problems: Vec<String> = Vec::new();
            if let Err(problem) = inadmissible_tag_refusal(&result, MAP, &key, "a#b") {
                problems.push(problem);
            }
            let slot = slot_image(&factory, MAP, &key).await;
            if slot != "absent" {
                problems.push(format!("the tag was stored: {slot}"));
            }
            if !problems.is_empty() {
                failures.push(format!(
                    "(iii) trusted / {form_name}, OR_ADD record tag \"a#b\": {}",
                    problems.join("; ")
                ));
            }
        }

        // (iv) Guard: over a client connection the server regenerates the tag, so
        // neither the client's record tag nor a co-present `or_tag` is stored
        // and nothing may be refused.
        for (f, (form, form_name)) in forms.into_iter().enumerate() {
            checked += 1;
            let key = format!("iv-{f}");
            let op = ClientOp {
                or_tag: Some(Some("a|b".to_string())),
                ..or_add(&key, "a|b")
            };
            let result = Arc::clone(&svc)
                .oneshot(send(form, ctx_for(Origin::Connection, &key), op))
                .await;

            let mut problems: Vec<String> = Vec::new();
            if let Err(error) = &result {
                problems.push(format!("Err({error:?}), want Ok"));
            }
            let (live, tombstones) = read_or_map(&factory, MAP, &key).await;
            let regenerated_only = matches!(
                live.as_slice(),
                [tag] if tag.ends_with(":test-node") && !tag.contains(['|', '#'])
            );
            if !regenerated_only || !tombstones.is_empty() {
                problems.push(format!(
                    "want one live record under the regenerated tag and no tombstone, got live \
                     {live:?} / tombstones {tombstones:?}"
                ));
            }
            if !problems.is_empty() {
                failures.push(format!(
                    "(iv) guard, client connection / {form_name}, OR_ADD with client tag \"a|b\" \
                     and or_tag \"a|b\": {}",
                    problems.join("; ")
                ));
            }
        }

        // (v) Guard: a whole-key REMOVE wins the classification and stores no
        // tag, whatever `or_tag` it carries.
        for (f, (form, form_name)) in forms.into_iter().enumerate() {
            checked += 1;
            let key = format!("v-{f}");
            seed_slot(&factory, MAP, &key, live_or_slot("seed-tag")).await;
            let op = ClientOp {
                op_type: Some("REMOVE".to_string()),
                or_tag: Some(Some("a|b".to_string())),
                ..bare(&key)
            };
            let result = Arc::clone(&svc)
                .oneshot(send(form, ctx_for(Origin::Connection, &key), op))
                .await;

            let mut problems: Vec<String> = Vec::new();
            if let Err(error) = &result {
                problems.push(format!("Err({error:?}), want Ok"));
            }
            let slot = slot_image(&factory, MAP, &key).await;
            if slot != "absent" {
                problems.push(format!("the key was not removed: {slot}"));
            }
            if !problems.is_empty() {
                failures.push(format!(
                    "(v) guard, client connection / {form_name}, REMOVE carrying or_tag \"a|b\": {}",
                    problems.join("; ")
                ));
            }
        }

        assert!(
            failures.is_empty(),
            "{} of {checked} sub-cases failed:\n  {}",
            failures.len(),
            failures.join("\n  ")
        );
    }

    /// A tag a slot already holds stays removable under the rule, in either OR
    /// shape; a different inadmissible tag cannot be added beside it, and a slot
    /// that holds no tag admits none. A failed read of the slot is a transient
    /// error, never a refusal and never an acceptance (TG-MRK-001).
    #[tokio::test]
    #[allow(clippy::too_many_lines)] // five stored-state sub-cases, asserted once
    async fn a_stored_inadmissible_tag_stays_removable() {
        use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};

        const MAP: &str = "legacy-tags";
        // The shape a node whose id carried a separator wrote before the rule.
        const LEGACY: &str = "1:0:n#1";

        /// An empty data store that fails ONE load once armed. It is not a null
        /// store, so a non-resident key is looked up in it.
        struct OneLoadFails {
            inner: NullDataStore,
            armed: AtomicBool,
        }

        #[async_trait::async_trait]
        impl MapDataStore for OneLoadFails {
            async fn add(
                &self,
                map: &str,
                key: &str,
                value: &RecordValue,
                expiration_time: i64,
                now: i64,
            ) -> anyhow::Result<()> {
                self.inner.add(map, key, value, expiration_time, now).await
            }

            async fn add_backup(
                &self,
                map: &str,
                key: &str,
                value: &RecordValue,
                expiration_time: i64,
                now: i64,
            ) -> anyhow::Result<()> {
                self.inner
                    .add_backup(map, key, value, expiration_time, now)
                    .await
            }

            async fn remove(&self, map: &str, key: &str, now: i64) -> anyhow::Result<()> {
                self.inner.remove(map, key, now).await
            }

            async fn remove_backup(&self, map: &str, key: &str, now: i64) -> anyhow::Result<()> {
                self.inner.remove_backup(map, key, now).await
            }

            async fn load(&self, map: &str, key: &str) -> anyhow::Result<Option<RecordValue>> {
                if self.armed.swap(false, AtomicOrdering::SeqCst) {
                    return Err(anyhow::anyhow!("injected load failure"));
                }
                self.inner.load(map, key).await
            }

            async fn load_all(
                &self,
                map: &str,
                keys: &[String],
            ) -> anyhow::Result<Vec<(String, RecordValue)>> {
                self.inner.load_all(map, keys).await
            }

            async fn enumerate_leaves(
                &self,
                map: &str,
                is_backup: bool,
                sink: &mut dyn LeafSink,
            ) -> anyhow::Result<()> {
                self.inner.enumerate_leaves(map, is_backup, sink).await
            }

            async fn scan_values(
                &self,
                map: &str,
                is_backup: bool,
                max_batch_cost: u64,
            ) -> anyhow::Result<ScanBatch> {
                self.inner.scan_values(map, is_backup, max_batch_cost).await
            }

            async fn scan_values_batched(
                &self,
                map: &str,
                is_backup: bool,
                cursor: ScanCursor,
                max_batch_cost: u64,
            ) -> anyhow::Result<ScanBatch> {
                self.inner
                    .scan_values_batched(map, is_backup, cursor, max_batch_cost)
                    .await
            }

            async fn remove_all(&self, map: &str, keys: &[String]) -> anyhow::Result<()> {
                self.inner.remove_all(map, keys).await
            }

            fn is_loadable(&self, key: &str) -> bool {
                self.inner.is_loadable(key)
            }

            fn pending_operation_count(&self) -> u64 {
                self.inner.pending_operation_count()
            }

            async fn soft_flush(&self) -> anyhow::Result<u64> {
                self.inner.soft_flush().await
            }

            async fn hard_flush(&self) -> anyhow::Result<()> {
                self.inner.hard_flush().await
            }

            async fn flush_key(
                &self,
                map: &str,
                key: &str,
                value: &RecordValue,
                is_backup: bool,
            ) -> anyhow::Result<()> {
                self.inner.flush_key(map, key, value, is_backup).await
            }

            fn reset(&self) {
                self.inner.reset();
            }
        }

        /// A service over `data_store` with one authenticated client connection.
        /// The map carries no query subscription, so the only read a request
        /// makes of a key is the write path's own.
        async fn connected_service(
            data_store: Arc<dyn MapDataStore>,
        ) -> (
            Arc<CrdtService>,
            Arc<RecordStoreFactory>,
            ConnectionId,
            Arc<ConnectionRegistry>,
        ) {
            let factory = Arc::new(RecordStoreFactory::new(
                StorageConfig::default(),
                data_store,
                Vec::new(),
            ));
            let registry = Arc::new(ConnectionRegistry::new());
            let svc = Arc::new(CrdtService::new(
                Arc::clone(&factory),
                Arc::clone(&registry),
                make_validator(),
                Arc::new(QueryRegistry::new()),
                Arc::new(SchemaService::new()),
            ));
            let (handle, _rx) = registry.register(
                ConnectionKind::Client,
                &crate::network::config::ConnectionConfig::default(),
            );
            handle.metadata.write().await.authenticated = true;
            (svc, factory, handle.id, registry)
        }

        fn or_remove_over(conn: ConnectionId, key: &str, tag: &str) -> Operation {
            let mut op = or_remove_op(MAP, key, tag);
            if let Operation::ClientOp { ctx, .. } = &mut op {
                ctx.connection_id = Some(conn);
            }
            op
        }

        let (svc, factory, conn, _registry) = connected_service(Arc::new(NullDataStore)).await;

        // Every sub-case is evaluated and the test asserts once, so a single run
        // shows which stored states are handled and which are not.
        let mut failures: Vec<String> = Vec::new();
        let mut checked = 0_usize;

        // (a) The stored tag is removable, and removing it again is idempotent.
        seed_slot(&factory, MAP, "a-live", live_or_slot(LEGACY)).await;
        let want_slot = format!("live [] / tombstones {:?}", [LEGACY]);
        for attempt in ["first", "repeated"] {
            checked += 1;
            let result = Arc::clone(&svc)
                .oneshot(or_remove_over(conn, "a-live", LEGACY))
                .await;
            let mut problems: Vec<String> = Vec::new();
            if let Err(error) = &result {
                problems.push(format!("Err({error:?}), want Ok"));
            }
            let slot = slot_image(&factory, MAP, "a-live").await;
            if slot != want_slot {
                problems.push(format!("the slot is {slot}, want {want_slot}"));
            }
            if !problems.is_empty() {
                failures.push(format!(
                    "(a) guard, {attempt} OR_REMOVE of the stored live tag {LEGACY:?}: {}",
                    problems.join("; ")
                ));
            }
        }

        // (b) Holding one inadmissible tag admits no other: beside a live legacy
        // record, and beside the legacy tombstone (a) left behind.
        seed_slot(&factory, MAP, "b-live", live_or_slot(LEGACY)).await;
        for (key, other_tag, beside) in [
            ("b-live", "2:0:n#1", "a live legacy record"),
            ("a-live", "a|b", "a legacy tombstone"),
        ] {
            checked += 1;
            let slot_before = slot_image(&factory, MAP, key).await;
            let result = Arc::clone(&svc)
                .oneshot(or_remove_over(conn, key, other_tag))
                .await;
            let mut problems: Vec<String> = Vec::new();
            if let Err(problem) = inadmissible_tag_refusal(&result, MAP, key, other_tag) {
                problems.push(problem);
            }
            let slot_after = slot_image(&factory, MAP, key).await;
            if slot_after != slot_before {
                problems.push(format!("the slot changed to {slot_after}"));
            }
            if !problems.is_empty() {
                failures.push(format!(
                    "(b) OR_REMOVE of a different inadmissible tag {other_tag:?} beside {beside}: {}",
                    problems.join("; ")
                ));
            }
        }

        // (c) The legacy tombstone-only shape holds its tags too.
        checked += 1;
        seed_slot(
            &factory,
            MAP,
            "c-legacy",
            RecordValue::OrTombstones {
                tags: vec![LEGACY.to_string()],
            },
        )
        .await;
        {
            let result = Arc::clone(&svc)
                .oneshot(or_remove_over(conn, "c-legacy", LEGACY))
                .await;
            let mut problems: Vec<String> = Vec::new();
            if let Err(error) = &result {
                problems.push(format!("Err({error:?}), want Ok"));
            }
            let (live, tombstones) = read_or_map(&factory, MAP, "c-legacy").await;
            if !live.is_empty() || tombstones != [LEGACY] {
                problems.push(format!(
                    "want the tag kept as the only tombstone, got live {live:?} / tombstones \
                     {tombstones:?}"
                ));
            }
            if !problems.is_empty() {
                failures.push(format!(
                    "(c) guard, OR_REMOVE of a tag held in a legacy OrTombstones value: {}",
                    problems.join("; ")
                ));
            }
        }

        // (d) An Lww value and an absent key hold no tag, so nothing is admitted.
        seed_slot(
            &factory,
            MAP,
            "d-lww",
            lww_record_to_record_value(&topgun_core::LWWRecord {
                value: Some(rmpv::Value::String("plain".into())),
                timestamp: make_timestamp(),
                ttl_ms: None,
            }),
        )
        .await;
        for (key, holding) in [("d-lww", "an Lww value"), ("d-absent", "no slot")] {
            checked += 1;
            let slot_before = slot_image(&factory, MAP, key).await;
            let result = Arc::clone(&svc)
                .oneshot(or_remove_over(conn, key, "a#b"))
                .await;
            let mut problems: Vec<String> = Vec::new();
            if let Err(problem) = inadmissible_tag_refusal(&result, MAP, key, "a#b") {
                problems.push(problem);
            }
            let slot_after = slot_image(&factory, MAP, key).await;
            if slot_after != slot_before {
                problems.push(format!("the slot changed to {slot_after}"));
            }
            if !problems.is_empty() {
                failures.push(format!(
                    "(d) OR_REMOVE \"a#b\" on a key holding {holding}: {}",
                    problems.join("; ")
                ));
            }
        }

        // (e) Guard: when the slot cannot be read, the request fails as a
        // transient error. Reporting it as a refusal would make a retryable
        // fault permanent; accepting it would let the tag in unseen.
        checked += 1;
        {
            let failing = Arc::new(OneLoadFails {
                inner: NullDataStore,
                armed: AtomicBool::new(false),
            });
            let (svc, factory, conn, _registry) =
                connected_service(failing.clone() as Arc<dyn MapDataStore>).await;
            failing.armed.store(true, AtomicOrdering::SeqCst);

            let result = svc
                .oneshot(or_remove_over(conn, "e-non-resident", "a#b"))
                .await;
            let mut problems: Vec<String> = Vec::new();
            match &result {
                Err(OperationError::Internal(_)) => {}
                Err(other) => problems.push(format!("Err({other:?}), want Err(Internal)")),
                Ok(_) => problems.push("Ok (the op was applied), want Err(Internal)".to_string()),
            }
            if failing.armed.load(AtomicOrdering::SeqCst) {
                problems.push("the request never loaded the non-resident key".to_string());
            }
            let slot = slot_image(&factory, MAP, "e-non-resident").await;
            if slot != "absent" {
                problems.push(format!("the tag was stored: {slot}"));
            }
            if !problems.is_empty() {
                failures.push(format!(
                    "(e) guard, OR_REMOVE \"a#b\" on a non-resident key whose load fails: {}",
                    problems.join("; ")
                ));
            }
        }

        assert!(
            failures.is_empty(),
            "{} of {checked} sub-cases failed:\n  {}",
            failures.len(),
            failures.join("\n  ")
        );
    }

    /// The predicate itself, on the table both ingest paths are held to: the
    /// reserved inputs are refused with their own reason, and a tag that only
    /// looks unusual is not (TG-MRK-001).
    #[test]
    fn or_tag_refusal_names_exactly_the_reserved_inputs() {
        const EMPTY: &str = "OR tag is empty";
        const RESERVED: &str = "OR tag contains a reserved character ('|' or '#')";

        for (tag, want) in [
            ("", Some(EMPTY)),
            ("|", Some(RESERVED)),
            ("#", Some(RESERVED)),
            ("a|b", Some(RESERVED)),
            ("a#b", Some(RESERVED)),
            ("a|", Some(RESERVED)),
            ("#a", Some(RESERVED)),
            ("TOP", None),
            ("a:b", None),
            ("1:0:n-1", None),
            ("a b", None),
        ] {
            assert_eq!(or_tag_refusal(tag), want, "tag {tag:?}");
        }
    }

    // -----------------------------------------------------------------------
    // The stored-tag lookup is lazy (TG-MRK-001)
    //
    // An inadmissible tag is admitted only when the key's slot already holds
    // it, which takes a read of the slot. That read is resolved lazily, so
    // three properties need their own proof: an admissible tag costs no read,
    // an unreadable slot refuses rather than admits, and the read never runs
    // ahead of the checks that would have refused the op anyway.
    // -----------------------------------------------------------------------

    /// An empty data store that logs every key it is asked to load and can fail
    /// the next load. It is not a null store, so each read of a non-resident
    /// key reaches it: the log is the list of slot reads that missed memory.
    struct LoadProbe {
        inner: NullDataStore,
        loaded: Mutex<Vec<String>>,
        fail_next_load: std::sync::atomic::AtomicBool,
    }

    impl LoadProbe {
        fn loaded(&self) -> Vec<String> {
            self.loaded.lock().clone()
        }

        fn arm_load_failure(&self) {
            self.fail_next_load
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }

        fn load_failure_armed(&self) -> bool {
            self.fail_next_load
                .load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl MapDataStore for LoadProbe {
        async fn add(
            &self,
            map: &str,
            key: &str,
            value: &RecordValue,
            expiration_time: i64,
            now: i64,
        ) -> anyhow::Result<()> {
            self.inner.add(map, key, value, expiration_time, now).await
        }

        async fn add_backup(
            &self,
            map: &str,
            key: &str,
            value: &RecordValue,
            expiration_time: i64,
            now: i64,
        ) -> anyhow::Result<()> {
            self.inner
                .add_backup(map, key, value, expiration_time, now)
                .await
        }

        async fn remove(&self, map: &str, key: &str, now: i64) -> anyhow::Result<()> {
            self.inner.remove(map, key, now).await
        }

        async fn remove_backup(&self, map: &str, key: &str, now: i64) -> anyhow::Result<()> {
            self.inner.remove_backup(map, key, now).await
        }

        async fn load(&self, map: &str, key: &str) -> anyhow::Result<Option<RecordValue>> {
            self.loaded.lock().push(key.to_string());
            if self
                .fail_next_load
                .swap(false, std::sync::atomic::Ordering::SeqCst)
            {
                return Err(anyhow::anyhow!("injected load failure"));
            }
            self.inner.load(map, key).await
        }

        async fn load_all(
            &self,
            map: &str,
            keys: &[String],
        ) -> anyhow::Result<Vec<(String, RecordValue)>> {
            self.inner.load_all(map, keys).await
        }

        async fn enumerate_leaves(
            &self,
            map: &str,
            is_backup: bool,
            sink: &mut dyn LeafSink,
        ) -> anyhow::Result<()> {
            self.inner.enumerate_leaves(map, is_backup, sink).await
        }

        async fn scan_values(
            &self,
            map: &str,
            is_backup: bool,
            max_batch_cost: u64,
        ) -> anyhow::Result<ScanBatch> {
            self.inner.scan_values(map, is_backup, max_batch_cost).await
        }

        async fn scan_values_batched(
            &self,
            map: &str,
            is_backup: bool,
            cursor: ScanCursor,
            max_batch_cost: u64,
        ) -> anyhow::Result<ScanBatch> {
            self.inner
                .scan_values_batched(map, is_backup, cursor, max_batch_cost)
                .await
        }

        async fn remove_all(&self, map: &str, keys: &[String]) -> anyhow::Result<()> {
            self.inner.remove_all(map, keys).await
        }

        fn is_loadable(&self, key: &str) -> bool {
            self.inner.is_loadable(key)
        }

        fn pending_operation_count(&self) -> u64 {
            self.inner.pending_operation_count()
        }

        async fn soft_flush(&self) -> anyhow::Result<u64> {
            self.inner.soft_flush().await
        }

        async fn hard_flush(&self) -> anyhow::Result<()> {
            self.inner.hard_flush().await
        }

        async fn flush_key(
            &self,
            map: &str,
            key: &str,
            value: &RecordValue,
            is_backup: bool,
        ) -> anyhow::Result<()> {
            self.inner.flush_key(map, key, value, is_backup).await
        }

        fn reset(&self) {
            self.inner.reset();
        }
    }

    /// Logs each `(map, partition)` store the factory builds. The factory asks
    /// its observer factories once per store it creates, so on a factory that
    /// has built nothing yet the log shows which stores a request resolved.
    struct StoreCreations(Mutex<Vec<(String, u32)>>);

    impl StoreCreations {
        fn created(&self) -> Vec<(String, u32)> {
            self.0.lock().clone()
        }
    }

    impl crate::storage::factory::ObserverFactory for StoreCreations {
        fn create_observer(
            &self,
            map_name: &str,
            partition_id: u32,
        ) -> Option<Arc<dyn MutationObserver>> {
            self.0.lock().push((map_name.to_string(), partition_id));
            None
        }
    }

    /// A service whose slot reads and store resolutions are observable.
    struct ProbedService {
        svc: Arc<CrdtService>,
        factory: Arc<RecordStoreFactory>,
        loads: Arc<LoadProbe>,
        stores: Arc<StoreCreations>,
        conn: ConnectionId,
        _registry: Arc<ConnectionRegistry>,
    }

    /// Builds a fresh [`ProbedService`] with one client connection. No map
    /// carries a query subscription, so a request reads a slot only on its
    /// write path: in the tag admission and in the apply.
    async fn probed_service(
        validator: Arc<WriteAdmission>,
        schema: Arc<SchemaService>,
        authenticated: bool,
    ) -> ProbedService {
        let loads = Arc::new(LoadProbe {
            inner: NullDataStore,
            loaded: Mutex::new(Vec::new()),
            fail_next_load: std::sync::atomic::AtomicBool::new(false),
        });
        let stores = Arc::new(StoreCreations(Mutex::new(Vec::new())));
        let factory = Arc::new(
            RecordStoreFactory::new(
                StorageConfig::default(),
                loads.clone() as Arc<dyn MapDataStore>,
                Vec::new(),
            )
            .with_observer_factories(vec![
                stores.clone() as Arc<dyn crate::storage::factory::ObserverFactory>
            ]),
        );
        let registry = Arc::new(ConnectionRegistry::new());
        let svc = Arc::new(CrdtService::new(
            Arc::clone(&factory),
            Arc::clone(&registry),
            validator,
            Arc::new(QueryRegistry::new()),
            schema,
        ));
        let (handle, _rx) = registry.register(
            ConnectionKind::Client,
            &crate::network::config::ConnectionConfig::default(),
        );
        handle.metadata.write().await.authenticated = authenticated;
        ProbedService {
            svc,
            factory,
            loads,
            stores,
            conn: handle.id,
            _registry: registry,
        }
    }

    async fn plain_probed_service() -> ProbedService {
        probed_service(make_validator(), Arc::new(SchemaService::new()), true).await
    }

    /// The four origins a write can arrive from; each takes its own admission
    /// branch in both handlers.
    #[derive(Clone, Copy, Debug)]
    enum Via {
        Connection,
        Http,
        Anonymous,
        Trusted,
    }

    const EVERY_ORIGIN: [Via; 4] = [Via::Connection, Via::Http, Via::Anonymous, Via::Trusted];

    fn ctx_via(via: Via, conn: ConnectionId, key: &str) -> OperationContext {
        match via {
            Via::Connection => {
                let mut ctx = make_ctx_for_key(key);
                ctx.connection_id = Some(conn);
                ctx
            }
            Via::Http => make_http_ctx_for_key(key),
            Via::Anonymous => make_anon_http_ctx_for_key(key),
            // No connection id and a plain `Client` origin: the branch that
            // keeps the caller's timestamp and tag verbatim.
            Via::Trusted => make_ctx_for_key(key),
        }
    }

    fn probe_bare_op(map: &str, key: &str) -> ClientOp {
        ClientOp {
            id: Some(format!("op-{key}")),
            map_name: map.to_string(),
            key: key.to_string(),
            op_type: None,
            record: None,
            or_record: None,
            or_tag: None,
            write_concern: None,
            timeout: None,
        }
    }

    fn probe_or_remove(map: &str, key: &str, tag: &str) -> ClientOp {
        ClientOp {
            or_tag: Some(Some(tag.to_string())),
            ..probe_bare_op(map, key)
        }
    }

    fn probe_or_add(map: &str, key: &str, tag: &str) -> ClientOp {
        ClientOp {
            or_record: Some(Some(topgun_core::ORMapRecord {
                value: rmpv::Value::String("value".into()),
                timestamp: make_timestamp(),
                tag: tag.to_string(),
                ttl_ms: None,
            })),
            ..probe_bare_op(map, key)
        }
    }

    fn probe_lww_put(map: &str, key: &str, value: rmpv::Value) -> ClientOp {
        ClientOp {
            record: Some(Some(topgun_core::LWWRecord {
                value: Some(value),
                timestamp: make_timestamp(),
                ttl_ms: None,
            })),
            ..probe_bare_op(map, key)
        }
    }

    /// One operation carrying `ops`: a `ClientOp` (exactly one op) or one
    /// `OpBatch`.
    fn probe_operation(as_batch: bool, ctx: OperationContext, mut ops: Vec<ClientOp>) -> Operation {
        if as_batch {
            Operation::OpBatch {
                ctx,
                payload: OpBatchMessage {
                    payload: topgun_core::messages::sync::OpBatchPayload {
                        ops,
                        write_concern: None,
                        timeout: None,
                    },
                },
            }
        } else {
            assert_eq!(ops.len(), 1, "a ClientOp carries exactly one op");
            Operation::ClientOp {
                ctx,
                payload: ClientOpMessage {
                    payload: ops.remove(0),
                },
            }
        }
    }

    fn form_name(as_batch: bool) -> &'static str {
        if as_batch {
            "OpBatch"
        } else {
            "ClientOp"
        }
    }

    /// An admissible tag is decided on its text alone: the admission resolves no
    /// store and reads no slot for it, at the shared function and through both
    /// handlers on every origin (TG-MRK-001).
    #[tokio::test]
    #[allow(clippy::too_many_lines)] // three views of one cost claim, asserted once
    async fn admissible_or_tag_costs_no_store_resolution_and_no_read() {
        use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

        const MAP: &str = "tag-cost";

        let mut failures: Vec<String> = Vec::new();

        // (A) The shared function, driven directly. COUNTED: calls of the store
        // resolver handed to it, loads the data store saw, stores the factory
        // built. Nothing else runs here, so every count is the admission's.
        {
            let probed = plain_probed_service().await;
            let resolutions = AtomicUsize::new(0);
            let resolver_for = |key: &'static str| {
                let (resolutions, factory) = (&resolutions, &probed.factory);
                move || {
                    resolutions.fetch_add(1, AtomicOrdering::SeqCst);
                    factory.get_or_create(MAP, hash_to_partition(key))
                }
            };

            // Admissible tags only: no resolution, no load, no store built.
            let result = admit_or_tags(
                resolver_for("fn-clean"),
                MAP,
                "fn-clean",
                ["TOP", "a:b", "1:0:n-1", "a b"],
            )
            .await;
            let (resolved, loaded, created) = (
                resolutions.load(AtomicOrdering::SeqCst),
                probed.loads.loaded(),
                probed.stores.created(),
            );
            if result.is_err() || resolved != 0 || !loaded.is_empty() || !created.is_empty() {
                failures.push(format!(
                    "(A) admissible tags: want Ok with 0 resolutions, no load and no store built, \
                     got {result:?}, {resolved} resolutions, loads {loaded:?}, stores {created:?}"
                ));
            }

            // One inadmissible tag behind an admissible one, on a key that is
            // neither resident nor durable: exactly one resolution and one read,
            // and the call stops at that refusal (the third tag is never
            // looked up). This is also the control for the probe: an admission
            // read IS visible in the load log.
            resolutions.store(0, AtomicOrdering::SeqCst);
            let result = admit_or_tags(
                resolver_for("fn-absent"),
                MAP,
                "fn-absent",
                ["TOP", "a#b", "c|d"],
            )
            .await
            .map(|()| OperationResponse::Empty);
            let (resolved, loaded) = (
                resolutions.load(AtomicOrdering::SeqCst),
                probed.loads.loaded(),
            );
            let refusal = inadmissible_tag_refusal(&result, MAP, "fn-absent", "a#b");
            if refusal.is_err() || resolved != 1 || loaded != ["fn-absent"] {
                failures.push(format!(
                    "(A) one inadmissible tag on an absent key: want the refusal with 1 \
                     resolution and loads [\"fn-absent\"], got {refusal:?}, {resolved} \
                     resolutions, loads {loaded:?}"
                ));
            }

            // Several inadmissible tags the slot already holds, so every one of
            // them reaches the lookup: still one resolution for the whole call.
            seed_slot(
                &probed.factory,
                MAP,
                "fn-held",
                RecordValue::OrMap {
                    records: vec![OrMapEntry {
                        value: Value::String("seeded".into()),
                        tag: "1:0:n#1".to_string(),
                        timestamp: make_timestamp(),
                    }],
                    tombstones: vec!["2:0:n#1".to_string(), "a|b".to_string()],
                },
            )
            .await;
            resolutions.store(0, AtomicOrdering::SeqCst);
            let result = admit_or_tags(
                resolver_for("fn-held"),
                MAP,
                "fn-held",
                ["1:0:n#1", "TOP", "2:0:n#1", "a|b"],
            )
            .await;
            let resolved = resolutions.load(AtomicOrdering::SeqCst);
            if result.is_err() || resolved != 1 {
                failures.push(format!(
                    "(A) three held inadmissible tags: want Ok with exactly 1 resolution, got \
                     {result:?} with {resolved}"
                ));
            }
        }

        // (B) Through `handle_op_batch`, with the admission isolated by a
        // refusal. The batch ends in an op that is refused, and a refusal is
        // decided before the first apply, so NOTHING of this batch is applied:
        // every load and every store built during the request is the
        // admission's own. The admissible ops ahead of the refused one must
        // contribute none, which leaves exactly the refused key's read and the
        // refused key's store.
        let (key_tag, key_put, key_add, key_refused) =
            ("cost-tag", "cost-put", "cost-add", "cost-refused");
        let refused_partition = hash_to_partition(key_refused);
        assert!(
            [key_tag, key_put, key_add]
                .iter()
                .all(|key| hash_to_partition(key) != refused_partition),
            "precondition: the admissible keys must live in other partitions than the refused \
             key, or a store resolved for them could not be told from its store"
        );
        for via in EVERY_ORIGIN {
            let probed = plain_probed_service().await;
            let mut ops = vec![
                probe_or_remove(MAP, key_tag, "TOP"),
                probe_lww_put(MAP, key_put, rmpv::Value::String("v".into())),
            ];
            if matches!(via, Via::Trusted) {
                // The one branch that checks an OR_ADD's own tag.
                ops.push(probe_or_add(MAP, key_add, "1:0:n-1"));
            }
            ops.push(probe_or_remove(MAP, key_refused, "a|b"));

            let result = Arc::clone(&probed.svc)
                .oneshot(probe_operation(
                    true,
                    ctx_via(via, probed.conn, key_tag),
                    ops,
                ))
                .await;

            let refusal = inadmissible_tag_refusal(&result, MAP, key_refused, "a|b");
            let (loaded, created) = (probed.loads.loaded(), probed.stores.created());
            if refusal.is_err()
                || loaded != [key_refused]
                || created != [(MAP.to_string(), refused_partition)]
            {
                failures.push(format!(
                    "(B) {via:?} / OpBatch ending in a refused op: want the refusal, loads \
                     [{key_refused:?}] and the one store of partition {refused_partition}, got \
                     {refusal:?}, loads {loaded:?}, stores {created:?}"
                ));
            }
        }

        // (C) Through both handlers, for a request that IS applied. The apply
        // reads the slot itself, so the admission is isolated by subtraction:
        // the same op is run through the whole request on one fresh service and
        // through the apply alone (`apply_single_op` / `apply_batch_op`, called
        // directly, which contain no admission) on an identical fresh service.
        // COUNTED: the keys each data store was asked to load and the stores
        // each factory built. With no query subscription a request is exactly
        // admission + apply, so equal logs mean the admission added no read and
        // resolved no store of its own. The key is neither resident nor
        // durable, so a read by the admission could not hide behind the
        // apply's: an absent key is loaded again on every read.
        for via in EVERY_ORIGIN {
            for as_batch in [false, true] {
                for (what, op) in [
                    (
                        "admissible OR_REMOVE",
                        probe_or_remove(MAP, "cost-applied", "TOP"),
                    ),
                    (
                        "LWW PUT",
                        probe_lww_put(MAP, "cost-applied", rmpv::Value::String("v".into())),
                    ),
                ] {
                    let request = plain_probed_service().await;
                    let result = Arc::clone(&request.svc)
                        .oneshot(probe_operation(
                            as_batch,
                            ctx_via(via, request.conn, &op.key),
                            vec![op.clone()],
                        ))
                        .await;

                    let apply_only = plain_probed_service().await;
                    // The regenerating branches hand the apply a server stamp.
                    let stamp = (!matches!(via, Via::Trusted)).then(make_timestamp);
                    let applied = if as_batch {
                        apply_only
                            .svc
                            .apply_batch_op(&op, stamp.as_ref(), None)
                            .await
                    } else {
                        apply_only
                            .svc
                            .apply_single_op(&op, hash_to_partition(&op.key), stamp.as_ref())
                            .await
                            .map(|_| ())
                    };

                    let request_cost = (request.loads.loaded(), request.stores.created());
                    let apply_cost = (apply_only.loads.loaded(), apply_only.stores.created());
                    if result.is_err() || applied.is_err() || request_cost != apply_cost {
                        failures.push(format!(
                            "(C) {via:?} / {}, {what}: want the request to cost what its apply \
                             alone costs; request {result:?} with (loads, stores) \
                             {request_cost:?}, apply alone {applied:?} with {apply_cost:?}",
                            form_name(as_batch)
                        ));
                    }
                }
            }
        }

        assert!(
            failures.is_empty(),
            "{} sub-cases failed:\n  {}",
            failures.len(),
            failures.join("\n  ")
        );
    }

    /// When the slot cannot be read the admission fails closed: the request
    /// gets the transient error, never an acceptance and never a refusal, and
    /// nothing of it is applied (TG-MRK-001).
    #[tokio::test]
    #[allow(clippy::too_many_lines)] // the function and every ingest site, asserted once
    async fn unreadable_slot_fails_the_or_admission_closed() {
        const MAP: &str = "tag-unreadable";

        let mut failures: Vec<String> = Vec::new();

        // (A) The shared function. Its store resolver cannot fail by type (it
        // returns the store), so the one failure there is to inject is the
        // read's.
        {
            let probed = plain_probed_service().await;
            let resolver = || {
                probed
                    .factory
                    .get_or_create(MAP, hash_to_partition("fn-key"))
            };

            probed.loads.arm_load_failure();
            let result = admit_or_tags(resolver, MAP, "fn-key", ["a#b"]).await;
            if !matches!(result, Err(OperationError::Internal(_))) {
                failures.push(format!(
                    "(A) inadmissible tag, failing read: want Err(Internal), got {result:?}"
                ));
            }
            if probed.loads.load_failure_armed() {
                failures.push("(A) the inadmissible tag never read the slot".to_string());
            }

            // Control: an admissible tag never reaches the failing read.
            probed.loads.arm_load_failure();
            let result = admit_or_tags(resolver, MAP, "fn-key", ["TOP"]).await;
            if result.is_err() || !probed.loads.load_failure_armed() {
                failures.push(format!(
                    "(A) admissible tag, failing read armed: want Ok with the failure still \
                     armed, got {result:?}, armed {}",
                    probed.loads.load_failure_armed()
                ));
            }
        }

        // (B) Through the service. ONE load is failed, on a key that is neither
        // resident nor durable. The admission's read is the first read of the
        // request, so it takes the failure; had the admission let the request
        // through, the apply would have loaded the key a second time,
        // successfully, and stored the tag. So: the error is `Internal`, the
        // load log holds the key exactly once, and the slot is still absent.
        let mut cases: Vec<(Via, bool, &str, ClientOp)> = Vec::new();
        for via in EVERY_ORIGIN {
            for as_batch in [false, true] {
                cases.push((
                    via,
                    as_batch,
                    "OR_REMOVE",
                    probe_or_remove(MAP, "unreadable", "a#b"),
                ));
            }
        }
        for as_batch in [false, true] {
            cases.push((
                Via::Trusted,
                as_batch,
                "OR_ADD",
                probe_or_add(MAP, "unreadable", "a#b"),
            ));
        }
        for (via, as_batch, what, op) in cases {
            let probed = plain_probed_service().await;
            probed.loads.arm_load_failure();

            let result = Arc::clone(&probed.svc)
                .oneshot(probe_operation(
                    as_batch,
                    ctx_via(via, probed.conn, &op.key),
                    vec![op],
                ))
                .await;

            let mut problems: Vec<String> = Vec::new();
            if !matches!(result, Err(OperationError::Internal(_))) {
                problems.push(format!("want Err(Internal), got {result:?}"));
            }
            // Read before `slot_image`, which loads the key itself.
            let loaded = probed.loads.loaded();
            if loaded != ["unreadable"] {
                problems.push(format!(
                    "want the key loaded once, by the admission alone, got {loaded:?}"
                ));
            }
            let slot = slot_image(&probed.factory, MAP, "unreadable").await;
            if slot != "absent" {
                problems.push(format!("the tag was stored: {slot}"));
            }
            if !problems.is_empty() {
                failures.push(format!(
                    "(B) {via:?} / {}, {what} \"a#b\" on an unreadable slot: {}",
                    form_name(as_batch),
                    problems.join("; ")
                ));
            }
        }

        assert!(
            failures.is_empty(),
            "{} sub-cases failed:\n  {}",
            failures.len(),
            failures.join("\n  ")
        );
    }

    /// The tag admission runs after the checks that refuse an op outright: an
    /// op the caller may not write keeps its authorisation error and causes no
    /// slot read, and a batch that fails schema validation keeps its schema
    /// error (TG-MRK-001).
    #[tokio::test]
    #[allow(clippy::too_many_lines)] // every refusing check on every branch that runs it, asserted once
    async fn or_admission_runs_after_authorisation_and_schema_checks() {
        const MAP: &str = "tag-order";
        const KEY: &str = "order-key";

        #[derive(Clone, Copy, Debug)]
        enum Denied {
            /// Strict service, connection never authenticated.
            UnauthenticatedConnection,
            /// A connection whose context carries the anonymous origin.
            AnonymousOverConnection,
            /// Strict service, HTTP origin with no validated identity.
            HttpWithoutPrincipal,
        }

        let mut failures: Vec<String> = Vec::new();

        // (A) Refused by `admit_write`. The op is an OR_REMOVE of an
        // inadmissible tag on a key that is neither resident nor durable: run
        // first, the tag admission would read the slot (one load, one store
        // built) and answer `SchemaInvalid`. COUNTED: loads and stores built
        // during the request. A refused request applies nothing, so anything
        // counted is the admission's.
        for denied in [
            Denied::UnauthenticatedConnection,
            Denied::AnonymousOverConnection,
            Denied::HttpWithoutPrincipal,
        ] {
            for as_batch in [false, true] {
                let probed = match denied {
                    Denied::AnonymousOverConnection => plain_probed_service().await,
                    Denied::UnauthenticatedConnection | Denied::HttpWithoutPrincipal => {
                        probed_service(
                            make_strict_validator(),
                            Arc::new(SchemaService::new()),
                            false,
                        )
                        .await
                    }
                };
                let ctx = match denied {
                    Denied::UnauthenticatedConnection => ctx_via(Via::Connection, probed.conn, KEY),
                    Denied::AnonymousOverConnection => {
                        let mut ctx = ctx_via(Via::Connection, probed.conn, KEY);
                        ctx.caller_origin = CallerOrigin::Anonymous;
                        ctx
                    }
                    Denied::HttpWithoutPrincipal => {
                        let mut ctx = ctx_via(Via::Http, probed.conn, KEY);
                        ctx.principal = None;
                        ctx
                    }
                };

                let result = Arc::clone(&probed.svc)
                    .oneshot(probe_operation(
                        as_batch,
                        ctx,
                        vec![probe_or_remove(MAP, KEY, "a|b")],
                    ))
                    .await;

                let kept_its_error = match denied {
                    Denied::AnonymousOverConnection => matches!(
                        &result,
                        Err(OperationError::Forbidden { map_name }) if map_name == MAP
                    ),
                    Denied::UnauthenticatedConnection | Denied::HttpWithoutPrincipal => {
                        matches!(&result, Err(OperationError::Unauthorized))
                    }
                };
                let (loaded, created) = (probed.loads.loaded(), probed.stores.created());
                if !kept_its_error || !loaded.is_empty() || !created.is_empty() {
                    failures.push(format!(
                        "(A) {denied:?} / {}: want the authorisation error with no load and no \
                         store built, got {result:?}, loads {loaded:?}, stores {created:?}",
                        form_name(as_batch)
                    ));
                }
            }
        }

        // Control for (A): the same op from a caller that may write is refused
        // for its tag, and that refusal's read shows in the load log.
        for as_batch in [false, true] {
            let probed = probed_service(
                make_strict_validator(),
                Arc::new(SchemaService::new()),
                true,
            )
            .await;
            let result = Arc::clone(&probed.svc)
                .oneshot(probe_operation(
                    as_batch,
                    ctx_via(Via::Connection, probed.conn, KEY),
                    vec![probe_or_remove(MAP, KEY, "a|b")],
                ))
                .await;
            let refusal = inadmissible_tag_refusal(&result, MAP, KEY, "a|b");
            let loaded = probed.loads.loaded();
            if refusal.is_err() || loaded != [KEY] {
                failures.push(format!(
                    "(A) control, authenticated connection / {}: want the tag refusal and loads \
                     [{KEY:?}], got {refusal:?}, loads {loaded:?}",
                    form_name(as_batch)
                ));
            }
        }

        // (B) Refused by `validate_schema_for_op`. No single op can fail both
        // checks: the classes whose tag is checked (an OR_REMOVE, a trusted
        // OR_ADD) carry no schema-validated value. So the order is observable
        // across the ops of one batch: an op that fails its schema, ahead of
        // an op whose tag would be refused, keeps the schema error, and the
        // later op's slot is never read. A tag pass run ahead of the schema
        // pass would answer with the tag refusal and one load instead.
        for via in [Via::Connection, Via::Http, Via::Anonymous] {
            let schema = Arc::new(SchemaService::new());
            schema
                .register_schema("typed-map", make_required_string_schema())
                .await
                .expect("register the schema");
            let probed = probed_service(make_validator(), schema, true).await;

            let result = Arc::clone(&probed.svc)
                .oneshot(probe_operation(
                    true,
                    ctx_via(via, probed.conn, "order-typed"),
                    vec![
                        probe_lww_put(
                            "typed-map",
                            "order-typed",
                            make_rmpv_map(vec![("name", rmpv::Value::Integer(42.into()))]),
                        ),
                        probe_or_remove(MAP, KEY, "a|b"),
                    ],
                ))
                .await;

            let kept_its_error = matches!(
                &result,
                Err(OperationError::SchemaInvalid { map_name, errors })
                    if map_name == "typed-map"
                        && errors.iter().all(|error| !error.contains("OR tag"))
            );
            let (loaded, created) = (probed.loads.loaded(), probed.stores.created());
            if !kept_its_error || !loaded.is_empty() || !created.is_empty() {
                failures.push(format!(
                    "(B) {via:?} / OpBatch [schema-invalid PUT, OR_REMOVE \"a|b\"]: want the \
                     schema error of the first op with no load and no store built, got \
                     {result:?}, loads {loaded:?}, stores {created:?}"
                ));
            }
        }

        assert!(
            failures.is_empty(),
            "{} sub-cases failed:\n  {}",
            failures.len(),
            failures.join("\n  ")
        );
    }
}
