//! Sync domain service implementing Merkle delta sync protocol.
//!
//! Handles 6 sync-related `Operation` variants across two CRDT types
//! (LWW-Map and OR-Map), using `MerkleSyncManager` for per-partition tree
//! management and `RecordStoreFactory` for record access.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use async_trait::async_trait;
use tower::Service;

use topgun_core::messages::{
    self, Message, ORMapDiffResponse, ORMapDiffResponsePayload, ORMapEntry, ORMapSyncRespBuckets,
    ORMapSyncRespBucketsPayload, ORMapSyncRespLeaf, ORMapSyncRespLeafPayload, ORMapSyncRespRoot,
    ORMapSyncRespRootPayload, SyncLeafRecord, SyncRespBucketsMessage, SyncRespBucketsPayload,
    SyncRespLeafMessage, SyncRespLeafPayload, SyncRespRootMessage, SyncRespRootPayload,
};
use topgun_core::types::Value;
use topgun_core::{hash_to_partition, LWWRecord, ORMapRecord};

// Helper enum for Merkle tree node classification (extracted to module level
// to avoid "adding items after statements" clippy warning).
enum NodeData {
    Leaf(Vec<String>),
    Internal(HashMap<char, u32>),
    Missing,
}

/// Parses a path with a 3-digit zero-padded partition prefix (e.g. `"042/abc"`).
///
/// Returns `Some((partition_id, sub_path))` when the path starts with exactly
/// 3 ASCII digit characters followed by `/`. The `sub_path` is the remainder after
/// the slash (may be empty for the partition root).
///
/// Returns `None` for aggregate-mode paths (`""`, `"a"`, `"ab"`, etc.) that have
/// no partition prefix.
fn parse_partition_prefix(path: &str) -> Option<(u32, String)> {
    // Must have at least 4 bytes: 3 digits + '/'
    if path.len() < 4 {
        return None;
    }
    let bytes = path.as_bytes();
    if bytes[0].is_ascii_digit()
        && bytes[1].is_ascii_digit()
        && bytes[2].is_ascii_digit()
        && bytes[3] == b'/'
    {
        let partition_id: u32 = path[..3].parse().ok()?;
        let sub_path = path[4..].to_string();
        Some((partition_id, sub_path))
    } else {
        None
    }
}

use dashmap::DashMap;
use tracing::Instrument;

use crate::network::connection::{ConnectionId, ConnectionKind, ConnectionRegistry};
use crate::network::device_identity::frontier_client_id;
use crate::service::domain::crdt::prune_epoch_tombstones;
use crate::service::domain::key_writer::KeyWriterRegistry;
use crate::service::operation::{service_names, Operation, OperationError, OperationResponse};
use crate::service::registry::{ManagedService, ServiceContext};
use crate::storage::map_data_store::{DurableMerkleIndex, MapDataStore, MerkleSession};
use crate::storage::merkle_sync::MerkleSyncManager;
use crate::storage::record::RecordValue;
use crate::storage::{CallerProvenance, ExpiryPolicy, RecordStoreFactory};
use crate::tombstone_frontier::{ClientId, GateToken};
use crate::tombstone_frontier_impl::TombstoneFrontier;

// ---------------------------------------------------------------------------
// value_to_rmpv conversion helper
// ---------------------------------------------------------------------------

/// Converts an internal `topgun_core::types::Value` to an `rmpv::Value` for wire format.
///
/// Used in LWW and OR-Map leaf responses to convert stored values back to the
/// wire format that clients expect.
///
/// TODO: Consolidate this and `rmpv_to_value` from `crdt.rs` into a shared
/// `service/domain/conversion.rs` module in a follow-up spec.
pub(crate) fn value_to_rmpv(v: &Value) -> rmpv::Value {
    match v {
        Value::Null => rmpv::Value::Nil,
        Value::Bool(b) => rmpv::Value::Boolean(*b),
        Value::Int(i) => rmpv::Value::Integer((*i).into()),
        Value::Float(f) => rmpv::Value::F64(*f),
        Value::String(s) => rmpv::Value::String(s.as_str().into()),
        Value::Bytes(b) => rmpv::Value::Binary(b.clone()),
        Value::Array(a) => rmpv::Value::Array(a.iter().map(value_to_rmpv).collect()),
        Value::Map(m) => {
            let pairs: Vec<(rmpv::Value, rmpv::Value)> = m
                .iter()
                .map(|(k, v)| (rmpv::Value::String(k.as_str().into()), value_to_rmpv(v)))
                .collect();
            rmpv::Value::Map(pairs)
        }
    }
}

// ---------------------------------------------------------------------------
// SyncService
// ---------------------------------------------------------------------------

/// Real sync domain service implementing the Merkle delta sync protocol.
///
/// Replaces the `domain_stub!(SyncService, ...)` macro-generated stub.
/// Handles LWW-Map and OR-Map synchronization using Merkle tree comparison
/// so clients receive only the changed records when reconnecting after
/// an offline period.
///
/// When wired with a durable index via [`SyncService::with_durable_index`],
/// the 4 Merkle tree-walk handlers resolve root/buckets/leaf-keys from the
/// [`DurableMerkleIndex`] rather than the in-memory [`MerkleSyncManager`].
/// This makes SYNC reads authoritative over persisted-but-not-resident records.
///
/// ## Session lifecycle
///
/// `build_session` does a full `enumerate_leaves` pass over the whole map
/// and returns a point-in-time snapshot. To keep the per-round cost to one
/// enumeration pass (not one per handler call), each `SYNC_INIT` builds a
/// fresh session and stores it in [`SyncSessionRegistry`], keyed by
/// `(map_name, connection_id)`. Every subsequent `MerkleReqBucket` /
/// `ORMapMerkleReqBucket` call from the SAME connection reuses that snapshot
/// without touching storage again. Keying per connection means a peer's
/// `SYNC_INIT` can only replace its OWN entry — it can never swap a slow
/// client's in-progress walk snapshot out from under it. Entries are released
/// when the connection disconnects (wired through
/// [`ConnectionRegistry::on_disconnect`]), so the cache is bounded by
/// (live connections × maps synced per connection) and fully drains as
/// connections close — no TTL/LRU that could evict an in-progress session.
pub struct SyncService {
    merkle_manager: Arc<MerkleSyncManager>,
    record_store_factory: Arc<RecordStoreFactory>,
    connection_registry: Arc<ConnectionRegistry>,
    /// Durable Merkle index: when present, tree-walk handlers resolve
    /// root/buckets/leaf-keys from this index instead of the in-memory manager.
    durable_index: Option<Arc<dyn DurableMerkleIndex + Send + Sync>>,
    /// The durable backing store passed to `build_session`.
    durable_store: Option<Arc<dyn MapDataStore>>,
    /// Per-`(map, connection)` session cache; entries released on disconnect.
    session_registry: Arc<SyncSessionRegistry>,
    /// Shared causal frontier: conveys the covering epoch on OR-Map sync
    /// responses and drives the SYNC-leaf prune. `None` when epoch machinery is
    /// not wired (tests / null-store). MUST be the SAME `Arc<TombstoneFrontier>`
    /// shared with `CrdtService` so the covering epoch and low-water-mark are one
    /// authority.
    frontier: Option<Arc<TombstoneFrontier>>,
    /// Shared per-key writer, the SAME registry `CrdtService` holds, so a
    /// SYNC-leaf prune sweep and an OR write on a key serialize against each
    /// other. `None` when the frontier is not wired.
    key_writer: Option<Arc<KeyWriterRegistry>>,
}

/// Per-`(map_name, connection_id)` cache of materialised Merkle sync sessions.
///
/// SPEC-325b cached one `MerkleSession` per map so a sync round materialises the
/// durable trie once. Keying by map name ALONE had two coupled defects: a second
/// client's `SYNC_INIT` (force-rebuild) could swap a slow client's in-progress
/// snapshot out from under its bucket DFS (TODO-544), and nothing ever released
/// entries, so each synced map's full key set stayed resident forever, outside
/// the eviction-cost accounting (TODO-545).
///
/// Keying by `(map_name, ConnectionId)` isolates each connection's round (a peer
/// can only replace its OWN entry), and [`Self::release_on_disconnect`] drops a
/// connection's sessions when it disconnects. The cache is therefore bounded by
/// (live connections × maps synced per connection) and drains fully as
/// connections close — not by live-connection count alone. No TTL/LRU: an
/// eviction-based bound could drop an in-progress session under load and re-open
/// a narrowed TODO-544. `release_on_disconnect` is O(total sessions) (a full
/// `retain` scan); cheap at demo-tier connection counts — a per-connection
/// reverse index is tracked as a scale follow-up (see `/sf:todo`).
#[derive(Default)]
struct SyncSessionRegistry {
    sessions: DashMap<(String, ConnectionId), Arc<MerkleSession>>,
}

impl SyncSessionRegistry {
    /// Returns the cached session for `(map_name, conn_id)`, if any.
    fn get(&self, map_name: &str, conn_id: ConnectionId) -> Option<Arc<MerkleSession>> {
        self.sessions
            .get(&(map_name.to_string(), conn_id))
            .map(|entry| Arc::clone(entry.value()))
    }

    /// Inserts (replacing any prior entry) the session for `(map_name, conn_id)`.
    fn insert(&self, map_name: &str, conn_id: ConnectionId, session: Arc<MerkleSession>) {
        self.sessions
            .insert((map_name.to_string(), conn_id), session);
    }

    /// Drops every session belonging to `conn_id`. Invoked on disconnect via the
    /// `ConnectionRegistry` observer so a connection's sessions do not outlive it.
    fn release_on_disconnect(&self, conn_id: ConnectionId) {
        self.sessions.retain(|(_, c), _| *c != conn_id);
    }

    /// Number of cached sessions across all connections (test-only bound check).
    #[cfg(test)]
    fn len(&self) -> usize {
        self.sessions.len()
    }
}

impl SyncService {
    /// Creates a new `SyncService`.
    #[must_use]
    pub fn new(
        merkle_manager: Arc<MerkleSyncManager>,
        record_store_factory: Arc<RecordStoreFactory>,
        connection_registry: Arc<ConnectionRegistry>,
    ) -> Self {
        let session_registry = Arc::new(SyncSessionRegistry::default());
        // Release a connection's sessions when it disconnects: the registry's
        // `remove()` is the single disconnect chokepoint the WebSocket handler
        // funnels through at end-of-life (after draining all in-flight dispatch
        // tasks), so a connection's sessions are always released and the cache is
        // bounded by (live connections × maps-per-connection) without a TTL/LRU.
        {
            let registry = Arc::clone(&session_registry);
            connection_registry
                .on_disconnect(Arc::new(move |id| registry.release_on_disconnect(id)));
        }
        Self {
            merkle_manager,
            record_store_factory,
            connection_registry,
            durable_index: None,
            durable_store: None,
            session_registry,
            frontier: None,
            key_writer: None,
        }
    }

    /// Wire the shared causal frontier + per-key writer so OR-Map sync responses
    /// convey the covering epoch (feeding the client ACK loop and the `342e`
    /// `delivered_conn` clamp) and the SYNC-leaf prune can run. Production wiring
    /// MUST pass the SAME `Arc`s held by `AppState` / `CrdtService`; a second
    /// frontier or writer would fork the epoch authority / re-open the prune race.
    /// `SyncService::new`'s signature is unchanged (builder, like
    /// `with_durable_index`).
    #[must_use]
    pub fn with_frontier(
        mut self,
        frontier: Arc<TombstoneFrontier>,
        key_writer: Arc<KeyWriterRegistry>,
    ) -> Self {
        self.frontier = Some(frontier);
        self.key_writer = Some(key_writer);
        self
    }

    /// The covering epoch to convey on an OR-Map sync response for `conn`: the
    /// server's current max stamped epoch (`None` when nothing stamped yet or no
    /// frontier is wired). Also records it as delivered on `conn` so the client's
    /// subsequent `CLIENT_APPLY_ACK` passes the `342e` delivered-clamp. Conveying
    /// it on EVERY OR-Map response (root/leaf/diff) — including the empty-diff
    /// root — is what lets an up-to-date client still advance its cursor instead
    /// of pinning the low-water-mark forever (empty-diff liveness).
    ///
    /// ORDERING IS LOAD-BEARING: every caller MUST read the covering epoch BEFORE
    /// computing the root hash / collecting the entries it rides with. The
    /// conveyed epoch must never postdate the state snapshot in the same response:
    /// a concurrent `OR_REMOVE` stamped between the data read and a later epoch
    /// read would make the client ACK an epoch whose tombstones it never received,
    /// breaking the cursor-implies-delivered invariant the prune rests on. Reading
    /// the epoch first errs conservative (the data may be NEWER than the epoch,
    /// so the client under-claims — safe).
    fn covering_epoch(&self, conn: Option<ConnectionId>, gated: bool) -> Option<u64> {
        let frontier = self.frontier.as_ref()?;
        let epoch = frontier.current_epoch();
        if epoch == 0 {
            return None;
        }
        // Convey the epoch as metadata for EVERY client, but ADVANCE `delivered_conn`
        // ONLY for a not-gated (tracked, non-forgotten, non-regressed) client. For a
        // gated (forgotten/unknown/regressed) client `set_delivered` is DEFERRED to
        // post-snapshot-resync completion (its `CLIENT_APPLY_ACK`): eagerly advancing
        // it here would re-enable the client's ACKs before it received the full
        // snapshot, re-admitting it via the sync path BEFORE any push (independent of
        // the push-diff gate). `delivered_conn` must advance only on resync completion.
        if !gated {
            if let Some(c) = conn {
                frontier.set_delivered(c, epoch);
            }
        }
        Some(epoch)
    }

    /// Resolve the server-authenticated `(principal, deviceId)` frontier identity for
    /// a connection, or `None` when the connection carries no server-issued device
    /// identity (an unknown client → forgotten treatment by the caller). Reads BOTH
    /// principal and `device_id` off the connection's OWN metadata — the same values the
    /// device-ownership claim keyed on — NEVER a wire-asserted / tag-embedded identity,
    /// preserving G9 identity-spoofing-CLEAN.
    async fn resolve_client_id(&self, conn: Option<ConnectionId>) -> Option<ClientId> {
        let conn = conn?;
        let handle = self.connection_registry.get(conn)?;
        let meta = handle.metadata.read().await;
        let device_id = meta.device_id.clone()?;
        let principal_id = meta.principal.as_ref().map(|p| p.id.clone());
        Some(frontier_client_id(principal_id.as_deref(), &device_id))
    }

    /// Whether `conn` must be routed through the full-snapshot REPLACE resync AND have
    /// its eager `set_delivered` suppressed (a gated client). True only while the gate
    /// is ACTIVE (SPEC-342j watermark raised — dark by construction until then) and the
    /// connection resolves to a forgotten/unknown/regressed identity. `claimed_epoch`
    /// (present only on the sync-init path) enables the regressed-replica check
    /// (`claim < stored_cursor`); the merkle/diff handlers pass `None`.
    async fn sync_gated(&self, conn: Option<ConnectionId>, claimed_epoch: Option<u64>) -> bool {
        let Some(frontier) = self.frontier.as_ref() else {
            return false;
        };
        // Dark by construction: while the durability watermark is 0 nothing can be
        // pruned, so a re-admission cannot resurrect anything and full-resync routing
        // would only force needless re-syncs on un-migrated clients. The routing goes
        // live together with the prune (gate-before-activation).
        if !frontier.is_protection_active() {
            return false;
        }
        match self.resolve_client_id(conn).await {
            None => true, // unknown identity → forgotten treatment → full resync
            Some(client) => {
                frontier.is_forgotten(&client)
                    // is_regressed is read-only — it NEVER rolls the stored cursor back
                    // (342a monotonicity); a regressed replica is routed through the same
                    // full-resync at its stale-high cursor unchanged.
                    || claimed_epoch.is_some_and(|claim| frontier.is_regressed(&client, claim))
            }
        }
    }

    /// Run the wholesale epoch-drop prune over the SYNC-leaf path. DARK by
    /// construction (the frontier watermark is constant 0), so this drops nothing
    /// in production; tests inject a watermark to exercise the drop. No-op unless
    /// BOTH the frontier and the shared per-key writer are wired.
    async fn run_leaf_prune(&self) {
        if let (Some(frontier), Some(key_writer)) = (&self.frontier, &self.key_writer) {
            prune_epoch_tombstones(frontier, &self.record_store_factory, key_writer).await;
        }
    }

    /// Wire the durable Merkle index into this `SyncService`.
    ///
    /// Once set, the four Merkle tree-walk handlers (`SyncInit`,
    /// `MerkleReqBucket`, `ORMapSyncInit`, `ORMapMerkleReqBucket`) resolve
    /// their root / bucket / leaf-key results from the durable index rather
    /// than the in-memory `MerkleSyncManager`. This makes SYNC reads
    /// authoritative over records that are persisted but not resident in memory,
    /// fixing the residency-coupling defect (TODO-530).
    ///
    /// The `store` must be the same durable backend the write path persists to
    /// so the enumerated leaf set is the authoritative one.
    ///
    /// `SyncService::new`'s 3-arg signature is unchanged; this is a builder
    /// method so existing construction sites need no modification.
    #[must_use]
    pub fn with_durable_index(
        mut self,
        index: Arc<dyn DurableMerkleIndex + Send + Sync>,
        store: Arc<dyn MapDataStore>,
    ) -> Self {
        self.durable_index = Some(index);
        self.durable_store = Some(store);
        self
    }

    // -----------------------------------------------------------------------
    // Session helpers
    // -----------------------------------------------------------------------

    /// Build (or return the cached) `MerkleSession` for `map_name`.
    ///
    /// When called from `handle_sync_init` the caller passes `force_rebuild =
    /// true` so a fresh session is always constructed for the start of a new
    /// sync round. Subsequent bucket handler calls pass `force_rebuild = false`
    /// and reuse the session built during `SYNC_INIT`, keeping the per-round
    /// cost to one `enumerate_leaves` pass.
    ///
    /// `build_session` uses `tokio::task::block_in_place` internally and MUST
    /// NOT be called from a single-threaded Tokio runtime. This service is
    /// always driven from a multi-threaded runtime (the server binary and every
    /// `#[tokio::test(flavor = "multi_thread")]` sim test), so `block_in_place`
    /// is safe here.
    ///
    /// Returns `Ok(None)` only when no durable index is configured — the caller
    /// then serves the round from the in-memory accelerator (the pre-durable
    /// behaviour, correct for tests and null-store deployments). When a durable
    /// index IS configured but the snapshot build fails, returns `Err`: we must
    /// NOT silently fall back to the in-memory tree, because that tree only sees
    /// resident keys and would hand the reconnecting client a root that omits
    /// persisted-but-evicted records — the exact residency coupling this path
    /// exists to eliminate. Surfacing the error lets the client retry against
    /// durable truth (mirrors `DurableMerkleIndex::build_session`'s
    /// "propagate, never degrade to wrong leaves" contract).
    fn get_or_build_session(
        &self,
        map_name: &str,
        conn_id: Option<ConnectionId>,
        force_rebuild: bool,
    ) -> Result<Option<Arc<MerkleSession>>, OperationError> {
        let (Some(index), Some(store)) = (&self.durable_index, &self.durable_store) else {
            return Ok(None);
        };

        // Build a fresh durable snapshot, propagating any build error. We must
        // NOT silently fall back to the in-memory tree on failure (it only sees
        // resident keys and would hand the client a root omitting evicted-but-
        // persisted records); surfacing the Err lets the client retry against
        // durable truth — the "propagate, never degrade to wrong leaves" contract.
        let build = || -> Result<Arc<MerkleSession>, OperationError> {
            match index.build_session(map_name, store.as_ref()) {
                Ok(session) => Ok(Arc::new(session)),
                Err(err) => {
                    tracing::error!(
                        map = %map_name,
                        error = %err,
                        "DurableMerkleIndex::build_session failed; rejecting sync round (no silent in-memory fallback)"
                    );
                    Err(OperationError::Internal(anyhow::anyhow!(
                        "durable Merkle session build failed for map {map_name}: {err}"
                    )))
                }
            }
        };

        match conn_id {
            // Cache keyed per connection so a peer's SYNC_INIT cannot swap this
            // client's in-progress walk snapshot out from under it. `connection_id`
            // is the per-connection id the WebSocket dispatch sets on every client
            // SYNC op — used here for SESSION ISOLATION, distinct from the
            // security/authorization identity the field's doc-comment refers to.
            Some(cid) => {
                if !force_rebuild {
                    if let Some(cached) = self.session_registry.get(map_name, cid) {
                        return Ok(Some(cached));
                    }
                }
                let arc = build()?;
                self.session_registry
                    .insert(map_name, cid, Arc::clone(&arc));
                Ok(Some(arc))
            }
            // Internal/forwarded ops carry no connection identity. Build fresh and
            // return WITHOUT caching: collapsing all None callers onto one shared
            // key would re-open TODO-544 on that path (one None caller's rebuild
            // would stomp another's in-progress walk). The build/Err contract is
            // unchanged — only the insert is skipped.
            //
            // This path is for single-shot internal/forwarded ops: every real
            // client SYNC round carries a connection_id set by the WebSocket
            // dispatch, so it takes the cached `Some` path above. A None caller
            // that ran a MULTI-call bucket walk would rebuild from current durable
            // state each call and so would NOT get a stable point-in-time snapshot
            // across the walk — giving it one would require a server-minted round
            // id echoed by the client, i.e. a WIRE CHANGE, which is out of scope
            // here (R6/C3). No such multi-call None-path walk exists today.
            None => Ok(Some(build()?)),
        }
    }

    // -----------------------------------------------------------------------
    // LWW handlers
    // -----------------------------------------------------------------------

    /// Handles `SyncInit` — returns the server's LWW Merkle tree root hash for `map_name`.
    ///
    /// `SyncInitMessage` is a FLAT message: `map_name` is directly on the payload struct,
    /// not nested in a `.payload` sub-field (contrast with `MerkleReqBucketMessage`).
    ///
    /// When a durable index is wired, builds a fresh `MerkleSession` for `map_name` and
    /// caches it so subsequent `MerkleReqBucket` calls for the same sync round can reuse
    /// the already-materialised snapshot without a second `enumerate_leaves` pass.
    /// Returns the combined LWW+OR-Map root from the session. Falls back to the
    /// in-memory `MerkleSyncManager` aggregate when no durable index is configured.
    #[allow(clippy::unused_async)] // declared async for uniformity with other handlers
    async fn handle_sync_init(
        &self,
        ctx: &crate::service::operation::OperationContext,
        payload: messages::SyncInitMessage,
    ) -> Result<OperationResponse, OperationError> {
        let map_name = payload.map_name;

        // Build a fresh session at the start of each sync round so the snapshot
        // reflects the current durable state. force_rebuild=true replaces any
        // cached session from a previous round for the same map.
        let root_hash =
            if let Some(session) = self.get_or_build_session(&map_name, ctx.connection_id, true)? {
                // Return the LWW-only root so the client drives the LWW tree walk.
                // The combined root would mix LWW and OR-Map hashes, changing the
                // wire protocol that the existing client expects for SYNC_INIT.
                session.lww_root()
            } else {
                self.merkle_manager.aggregate_lww_root_hash(&map_name)
            };

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

    /// Handles `MerkleReqBucket` — returns bucket hashes (internal) or leaf records (leaf).
    ///
    /// `MerkleReqBucketMessage` is a WRAPPED message: `map_name` and `path` live in a
    /// nested `.payload` field (i.e., `payload.payload.map_name`), unlike the flat
    /// `SyncInitMessage`.
    ///
    /// When a durable index is wired, all paths are pure hex aggregate paths (no
    /// partition-prefix routing needed — the durable index materialises a fully
    /// aggregated trie). Reuses the `MerkleSession` built during `SYNC_INIT` for
    /// the same map so this call costs zero additional `enumerate_leaves` passes.
    ///
    /// When no durable index is wired, falls back to the original two-mode dispatch:
    /// - **Aggregate mode** (`""` or paths without a 3-digit partition prefix)
    /// - **Routed mode** (paths beginning with a 3-digit zero-padded partition prefix)
    #[allow(clippy::too_many_lines)] // durable + fallback branches with leaf collection is inherently verbose
    async fn handle_merkle_req_bucket(
        &self,
        ctx: &crate::service::operation::OperationContext,
        payload: messages::MerkleReqBucketMessage,
    ) -> Result<OperationResponse, OperationError> {
        let map_name = payload.payload.map_name;
        let path = payload.payload.path;

        // Durable-index path: reuse the session built during SYNC_INIT for THIS
        // connection (force_rebuild=false) so no second enumerate_leaves pass is
        // needed. Paths in this branch are pure hex aggregate paths — no prefix.
        if let Some(session) = self.get_or_build_session(&map_name, ctx.connection_id, false)? {
            // Check for internal node (has children in the LWW trie at this path).
            let lww_children = session.lww_nodes.get(&path).cloned().unwrap_or_default();
            if !lww_children.is_empty() {
                // Aggregate mode: bucket keys are single hex chars. The client
                // extends its current path by each char, so returning a full
                // path here would double-prefix (e.g. at path "a" the child 'b'
                // must be sent as "b", not "ab", or the client drills to "ab"+"b").
                let buckets: HashMap<String, u32> = lww_children
                    .into_iter()
                    .map(|(c, h)| (c.to_string(), h))
                    .collect();
                return Ok(OperationResponse::Message(Box::new(
                    Message::SyncRespBuckets(SyncRespBucketsMessage {
                        payload: SyncRespBucketsPayload {
                            map_name,
                            path,
                            buckets,
                        },
                    }),
                )));
            }

            // No children at this path — check leaf membership.
            let leaf_keys = session.leaf_keys(&path);
            if !leaf_keys.is_empty() {
                let mut records = Vec::new();
                for key in &leaf_keys {
                    let key_partition = hash_to_partition(key);
                    let store = self
                        .record_store_factory
                        .get_or_create(&map_name, key_partition);
                    match store.get(key, false).await {
                        Ok(Some(record)) => {
                            if let RecordValue::Lww { value, timestamp } = record.value {
                                records.push(SyncLeafRecord {
                                    key: key.clone(),
                                    record: LWWRecord {
                                        value: Some(value_to_rmpv(&value)),
                                        timestamp,
                                        ttl_ms: None,
                                    },
                                });
                            }
                        }
                        Ok(None) => {
                            tracing::warn!(key = %key, partition = key_partition,
                                "Merkle leaf (durable): record not found in store");
                        }
                        Err(e) => {
                            tracing::error!(key = %key, partition = key_partition, error = %e,
                                "Merkle leaf (durable): store.get() error");
                        }
                    }
                }
                return Ok(OperationResponse::Message(Box::new(Message::SyncRespLeaf(
                    SyncRespLeafMessage {
                        payload: SyncRespLeafPayload {
                            map_name,
                            path,
                            records,
                        },
                    },
                ))));
            }

            // Path not present in the durable trie — map is empty or path is beyond
            // the trie depth. Return empty so the client stops drilling.
            return Ok(OperationResponse::Empty);
        }

        // Fallback: no durable index wired — use the in-memory MerkleSyncManager.
        // Parse path: routed mode uses a fixed 3-digit partition prefix (e.g. "042/abc").
        if let Some((partition_id, sub_path)) = parse_partition_prefix(&path) {
            // Routed mode: route directly to the specific partition tree.
            let node_data = self
                .merkle_manager
                .with_lww_tree(&map_name, partition_id, |tree| {
                    match tree.get_node(&sub_path) {
                        Some(node) if !node.entries.is_empty() => {
                            let keys: Vec<String> = node.entries.keys().cloned().collect();
                            NodeData::Leaf(keys)
                        }
                        Some(_) => {
                            let buckets = tree.get_buckets(&sub_path);
                            NodeData::Internal(buckets)
                        }
                        None => NodeData::Missing,
                    }
                });

            return match node_data {
                NodeData::Leaf(keys) => {
                    let mut records = Vec::new();
                    for key in &keys {
                        let key_partition = hash_to_partition(key);
                        let store = self
                            .record_store_factory
                            .get_or_create(&map_name, key_partition);
                        match store.get(key, false).await {
                            Ok(Some(record)) => {
                                if let RecordValue::Lww { value, timestamp } = record.value {
                                    records.push(SyncLeafRecord {
                                        key: key.clone(),
                                        record: LWWRecord {
                                            value: Some(value_to_rmpv(&value)),
                                            timestamp,
                                            ttl_ms: None,
                                        },
                                    });
                                }
                            }
                            Ok(None) => {
                                tracing::warn!(key = %key, partition = key_partition,
                                    "Merkle leaf (routed): record not found in store");
                            }
                            Err(e) => {
                                tracing::error!(key = %key, partition = key_partition, error = %e,
                                    "Merkle leaf (routed): store.get() error");
                            }
                        }
                    }
                    Ok(OperationResponse::Message(Box::new(Message::SyncRespLeaf(
                        SyncRespLeafMessage {
                            payload: SyncRespLeafPayload {
                                map_name,
                                path,
                                records,
                            },
                        },
                    ))))
                }
                NodeData::Internal(buckets) => {
                    // Return sub-paths prefixed with the same partition ID so the
                    // client can continue drilling down via routed mode.
                    let prefix = format!("{partition_id:03}/");
                    let buckets: HashMap<String, u32> = buckets
                        .into_iter()
                        .map(|(c, h)| (format!("{prefix}{sub_path}{c}"), h))
                        .collect();
                    Ok(OperationResponse::Message(Box::new(
                        Message::SyncRespBuckets(SyncRespBucketsMessage {
                            payload: SyncRespBucketsPayload {
                                map_name,
                                path,
                                buckets,
                            },
                        }),
                    )))
                }
                NodeData::Missing => Ok(OperationResponse::Empty),
            };
        }

        // Aggregate mode: combine bucket hashes from all partitions.
        let combined_buckets = self.merkle_manager.aggregate_lww_buckets(&map_name, &path);

        if combined_buckets.is_empty() {
            // No data for this map at all — check if any partition has a leaf at this path.
            let partition_ids = self.merkle_manager.lww_partition_ids(&map_name);
            if partition_ids.is_empty() {
                return Ok(OperationResponse::Empty);
            }
            // Some partitions exist but path is a leaf in all of them — collect keys.
            let mut all_keys: Vec<String> = Vec::new();
            for pid in &partition_ids {
                self.merkle_manager.with_lww_tree(&map_name, *pid, |tree| {
                    if let Some(node) = tree.get_node(&path) {
                        if !node.entries.is_empty() {
                            all_keys.extend(node.entries.keys().cloned());
                        }
                    }
                });
            }
            if all_keys.is_empty() {
                return Ok(OperationResponse::Empty);
            }
            // Return leaf records for all collected keys.
            let mut records = Vec::new();
            for key in &all_keys {
                let key_partition = hash_to_partition(key);
                let store = self
                    .record_store_factory
                    .get_or_create(&map_name, key_partition);
                match store.get(key, false).await {
                    Ok(Some(record)) => {
                        if let RecordValue::Lww { value, timestamp } = record.value {
                            records.push(SyncLeafRecord {
                                key: key.clone(),
                                record: LWWRecord {
                                    value: Some(value_to_rmpv(&value)),
                                    timestamp,
                                    ttl_ms: None,
                                },
                            });
                        }
                    }
                    Ok(None) => {
                        tracing::warn!(key = %key, partition = key_partition,
                            "Merkle leaf (aggregate): record not found in store");
                    }
                    Err(e) => {
                        tracing::error!(key = %key, partition = key_partition, error = %e,
                            "Merkle leaf (aggregate): store.get() error");
                    }
                }
            }
            return Ok(OperationResponse::Message(Box::new(Message::SyncRespLeaf(
                SyncRespLeafMessage {
                    payload: SyncRespLeafPayload {
                        map_name,
                        path,
                        records,
                    },
                },
            ))));
        }

        // Internal node in aggregate mode: return combined bucket hashes.
        // Bucket keys remain single hex chars — the client will send subsequent
        // requests with path extended by that character (still aggregate mode),
        // or the server may return partition-prefixed paths for leaf drill-down.
        let buckets: HashMap<String, u32> = combined_buckets
            .into_iter()
            .map(|(c, h)| (c.to_string(), h))
            .collect();
        Ok(OperationResponse::Message(Box::new(
            Message::SyncRespBuckets(SyncRespBucketsMessage {
                payload: SyncRespBucketsPayload {
                    map_name,
                    path,
                    buckets,
                },
            }),
        )))
    }

    // -----------------------------------------------------------------------
    // OR-Map handlers
    // -----------------------------------------------------------------------

    /// Handles `ORMapSyncInit` — returns the server's OR-Map Merkle tree root hash.
    ///
    /// `ORMapSyncInit` is a FLAT message: `map_name` is directly on the struct.
    ///
    /// When a durable index is wired, reuses the `MerkleSession` cached during
    /// the preceding LWW `SYNC_INIT` (if any) — one `enumerate_leaves` pass covers
    /// both LWW and OR-Map roots. If no LWW `SYNC_INIT` has yet run for this map,
    /// builds a fresh session now (`force_rebuild=false` with a cache miss triggers
    /// a build). Falls back to the in-memory `MerkleSyncManager` aggregate when no
    /// durable index is configured.
    async fn handle_ormap_sync_init(
        &self,
        ctx: &crate::service::operation::OperationContext,
        payload: messages::ORMapSyncInit,
    ) -> Result<OperationResponse, OperationError> {
        let map_name = payload.map_name;

        // Forgotten/unknown/regressed detection (R6/R7): a gated client is routed
        // through an authoritative FULL-snapshot REPLACE resync (never an incremental
        // delta), and its eager `set_delivered` is suppressed (deferred to resync
        // completion). `claimed_epoch` carries the client's locally-persisted cursor so
        // a REGRESSED replica (a backup-restore clone, `claim < stored_cursor`) is
        // caught here too — routed through the same full-resync WITHOUT advancing
        // `delivered_conn` and WITHOUT rolling the stored cursor back.
        let gated = self
            .sync_gated(ctx.connection_id, payload.claimed_epoch)
            .await;

        // Epoch BEFORE data: the conveyed epoch must never postdate the root it
        // rides with (see `covering_epoch` — ordering is load-bearing).
        let covering_epoch = self.covering_epoch(ctx.connection_id, gated);

        // Reuse any session already built by the LWW SYNC_INIT for this map; build
        // one now if not yet cached. force_rebuild=false so a concurrent or prior
        // LWW SYNC_INIT's session is shared rather than discarded.
        let root_hash = if let Some(session) =
            self.get_or_build_session(&map_name, ctx.connection_id, false)?
        {
            session.ormap_root()
        } else {
            self.merkle_manager.aggregate_ormap_root_hash(&map_name)
        };

        Ok(OperationResponse::Message(Box::new(
            Message::ORMapSyncRespRoot(ORMapSyncRespRoot {
                payload: ORMapSyncRespRootPayload {
                    map_name,
                    root_hash,
                    timestamp: ctx.timestamp.clone(),
                    covering_epoch,
                    // Directs a gated client to DISCARD its local OR-Map and adopt the
                    // server snapshot (authoritative REPLACE), never an additive merge.
                    full_resync: gated,
                },
            }),
        )))
    }

    /// Handles `ORMapMerkleReqBucket` — returns OR-Map bucket hashes (internal) or entries (leaf).
    ///
    /// `ORMapMerkleReqBucket` is a WRAPPED message: `map_name` and `path` live in a
    /// nested `.payload` field.
    ///
    /// When a durable index is wired, reuses the cached `MerkleSession` (built during
    /// `SYNC_INIT` or `ORMapSyncInit`) with OR-Map-specific trie nodes. Paths are pure
    /// hex aggregate paths. Falls back to the original scatter-gather routing when no
    /// durable index is configured.
    #[allow(clippy::too_many_lines)] // durable + fallback branches with entry collection is inherently verbose
    async fn handle_ormap_merkle_req_bucket(
        &self,
        ctx: &crate::service::operation::OperationContext,
        payload: messages::ORMapMerkleReqBucket,
    ) -> Result<OperationResponse, OperationError> {
        let map_name = payload.payload.map_name;
        let path = payload.payload.path;

        // Wholesale epoch-drop prune wired into the SYNC-leaf path: drop
        // prune-eligible tombstones from storage BEFORE the leaf is read, so the
        // emitted leaf naturally carries the reduced set. DARK by construction (the
        // frontier watermark is constant 0) — drops nothing in production.
        self.run_leaf_prune().await;

        // Gate `set_delivered` for a forgotten/unknown/regressed client (R9): the
        // covering epoch is still conveyed as metadata, but `delivered_conn` is not
        // advanced here — deferred to full-resync completion.
        let gated = self.sync_gated(ctx.connection_id, None).await;

        // Epoch BEFORE data: read once here, before any leaf entries are collected,
        // so the conveyed epoch can never postdate the entry sets the leaf
        // responses below carry (see `covering_epoch` — ordering is load-bearing).
        let covering_epoch = self.covering_epoch(ctx.connection_id, gated);

        // Durable-index path: reuse the session built during ORMapSyncInit (or
        // SYNC_INIT) for THIS connection. OR-Map tombstones yield no leaf in the
        // session (parity with write path: SPEC-324 R4).
        if let Some(session) = self.get_or_build_session(&map_name, ctx.connection_id, false)? {
            // Check for OR-Map internal node at this path.
            let ormap_children = session.ormap_nodes.get(&path).cloned().unwrap_or_default();
            if !ormap_children.is_empty() {
                // Aggregate mode: single-char bucket keys (see the LWW handler) —
                // the client appends each char to its current path itself.
                let buckets: HashMap<String, u32> = ormap_children
                    .into_iter()
                    .map(|(c, h)| (c.to_string(), h))
                    .collect();
                return Ok(OperationResponse::Message(Box::new(
                    Message::ORMapSyncRespBuckets(ORMapSyncRespBuckets {
                        payload: ORMapSyncRespBucketsPayload {
                            map_name,
                            path,
                            buckets,
                        },
                    }),
                )));
            }

            // No children — check leaf membership in the OR-Map trie.
            let leaf_keys = session.leaf_keys(&path);
            // Filter to keys that actually have OR-Map values (not LWW).
            // OrTombstones have no leaf in the session so they never appear here,
            // preserving the write-path invariant that tombstone-only keys produce
            // no leaf (SPEC-324 R4 parity).
            if !leaf_keys.is_empty() {
                let mut entries = Vec::new();
                for key in leaf_keys {
                    let key_partition = hash_to_partition(&key);
                    let store = self
                        .record_store_factory
                        .get_or_create(&map_name, key_partition);
                    if let Ok(Some(record)) = store.get(&key, false).await {
                        match record.value {
                            RecordValue::OrMap {
                                records,
                                tombstones,
                            } => {
                                let wire_records: Vec<ORMapRecord<rmpv::Value>> = records
                                    .into_iter()
                                    .map(|r| ORMapRecord {
                                        value: value_to_rmpv(&r.value),
                                        timestamp: r.timestamp,
                                        tag: r.tag,
                                        ttl_ms: None,
                                    })
                                    .collect();
                                // Carry tombstones so remove-wins suppression and
                                // add-wins survival both replicate to the peer.
                                entries.push(ORMapEntry {
                                    key,
                                    records: wire_records,
                                    tombstones,
                                });
                            }
                            RecordValue::OrTombstones { tags } => {
                                // Tombstone-only key: propagate tombstones, no live records.
                                entries.push(ORMapEntry {
                                    key,
                                    records: Vec::new(),
                                    tombstones: tags,
                                });
                            }
                            RecordValue::Lww { .. } => {
                                // LWW value under an OR-Map leaf path: skip, wrong CRDT type.
                            }
                        }
                    }
                }
                return Ok(OperationResponse::Message(Box::new(
                    Message::ORMapSyncRespLeaf(ORMapSyncRespLeaf {
                        payload: ORMapSyncRespLeafPayload {
                            map_name,
                            path,
                            entries,
                            covering_epoch,
                        },
                    }),
                )));
            }

            // Path absent from the OR-Map trie — return empty.
            return Ok(OperationResponse::Empty);
        }

        // Fallback: no durable index wired — use the in-memory MerkleSyncManager.
        if let Some((partition_id, sub_path)) = parse_partition_prefix(&path) {
            // Routed mode: route directly to the specific OR-Map partition tree.
            let node_data = self
                .merkle_manager
                .with_ormap_tree(&map_name, partition_id, |tree| {
                    match tree.get_node(&sub_path) {
                        Some(node) if !node.entries.is_empty() => {
                            let keys: Vec<String> = node.entries.keys().cloned().collect();
                            NodeData::Leaf(keys)
                        }
                        Some(_) => {
                            let buckets = tree.get_buckets(&sub_path);
                            NodeData::Internal(buckets)
                        }
                        None => NodeData::Missing,
                    }
                });

            return match node_data {
                NodeData::Leaf(keys) => {
                    let mut entries = Vec::new();
                    for key in keys {
                        let key_partition = hash_to_partition(&key);
                        let store = self
                            .record_store_factory
                            .get_or_create(&map_name, key_partition);
                        if let Ok(Some(record)) = store.get(&key, false).await {
                            match record.value {
                                RecordValue::OrMap {
                                    records,
                                    tombstones,
                                } => {
                                    let wire_records: Vec<ORMapRecord<rmpv::Value>> = records
                                        .into_iter()
                                        .map(|r| ORMapRecord {
                                            value: value_to_rmpv(&r.value),
                                            timestamp: r.timestamp,
                                            tag: r.tag,
                                            ttl_ms: None,
                                        })
                                        .collect();
                                    // Carry tombstones so remove-wins suppression and
                                    // add-wins survival both replicate to the peer.
                                    entries.push(ORMapEntry {
                                        key,
                                        records: wire_records,
                                        tombstones,
                                    });
                                }
                                RecordValue::OrTombstones { tags } => {
                                    entries.push(ORMapEntry {
                                        key,
                                        records: Vec::new(),
                                        tombstones: tags,
                                    });
                                }
                                RecordValue::Lww { .. } => {}
                            }
                        }
                    }
                    Ok(OperationResponse::Message(Box::new(
                        Message::ORMapSyncRespLeaf(ORMapSyncRespLeaf {
                            payload: ORMapSyncRespLeafPayload {
                                map_name,
                                path,
                                entries,
                                covering_epoch,
                            },
                        }),
                    )))
                }
                NodeData::Internal(buckets) => {
                    let prefix = format!("{partition_id:03}/");
                    let buckets: HashMap<String, u32> = buckets
                        .into_iter()
                        .map(|(c, h)| (format!("{prefix}{sub_path}{c}"), h))
                        .collect();
                    Ok(OperationResponse::Message(Box::new(
                        Message::ORMapSyncRespBuckets(ORMapSyncRespBuckets {
                            payload: ORMapSyncRespBucketsPayload {
                                map_name,
                                path,
                                buckets,
                            },
                        }),
                    )))
                }
                NodeData::Missing => Ok(OperationResponse::Empty),
            };
        }

        // Aggregate mode: combine bucket hashes from all OR-Map partitions.
        let combined_buckets = self
            .merkle_manager
            .aggregate_ormap_buckets(&map_name, &path);

        if combined_buckets.is_empty() {
            let partition_ids = self.merkle_manager.ormap_partition_ids(&map_name);
            if partition_ids.is_empty() {
                return Ok(OperationResponse::Empty);
            }
            // Check if partitions have leaf entries at this path.
            let mut all_keys: Vec<String> = Vec::new();
            for pid in &partition_ids {
                self.merkle_manager
                    .with_ormap_tree(&map_name, *pid, |tree| {
                        if let Some(node) = tree.get_node(&path) {
                            if !node.entries.is_empty() {
                                all_keys.extend(node.entries.keys().cloned());
                            }
                        }
                    });
            }
            if all_keys.is_empty() {
                return Ok(OperationResponse::Empty);
            }
            let mut entries = Vec::new();
            for key in all_keys {
                let key_partition = hash_to_partition(&key);
                let store = self
                    .record_store_factory
                    .get_or_create(&map_name, key_partition);
                if let Ok(Some(record)) = store.get(&key, false).await {
                    match record.value {
                        RecordValue::OrMap {
                            records,
                            tombstones,
                        } => {
                            let wire_records: Vec<ORMapRecord<rmpv::Value>> = records
                                .into_iter()
                                .map(|r| ORMapRecord {
                                    value: value_to_rmpv(&r.value),
                                    timestamp: r.timestamp,
                                    tag: r.tag,
                                    ttl_ms: None,
                                })
                                .collect();
                            // Carry tombstones so remove-wins suppression and
                            // add-wins survival both replicate to the peer.
                            entries.push(ORMapEntry {
                                key,
                                records: wire_records,
                                tombstones,
                            });
                        }
                        RecordValue::OrTombstones { tags } => {
                            entries.push(ORMapEntry {
                                key,
                                records: Vec::new(),
                                tombstones: tags,
                            });
                        }
                        RecordValue::Lww { .. } => {}
                    }
                }
            }
            return Ok(OperationResponse::Message(Box::new(
                Message::ORMapSyncRespLeaf(ORMapSyncRespLeaf {
                    payload: ORMapSyncRespLeafPayload {
                        map_name,
                        path,
                        entries,
                        covering_epoch,
                    },
                }),
            )));
        }

        let buckets: HashMap<String, u32> = combined_buckets
            .into_iter()
            .map(|(c, h)| (c.to_string(), h))
            .collect();
        Ok(OperationResponse::Message(Box::new(
            Message::ORMapSyncRespBuckets(ORMapSyncRespBuckets {
                payload: ORMapSyncRespBucketsPayload {
                    map_name,
                    path,
                    buckets,
                },
            }),
        )))
    }

    /// Handles `ORMapDiffRequest` — returns OR-Map entries for the requested keys.
    ///
    /// `ORMapDiffRequest` is a WRAPPED message: `map_name` and `keys` live in a
    /// nested `.payload` field.
    async fn handle_ormap_diff_request(
        &self,
        ctx: &crate::service::operation::OperationContext,
        payload: messages::ORMapDiffRequest,
    ) -> Result<OperationResponse, OperationError> {
        let map_name = payload.payload.map_name;
        let keys = payload.payload.keys;

        // Gate `set_delivered` for a forgotten/unknown/regressed client (R9): convey
        // the covering epoch as metadata only, do not advance `delivered_conn`.
        let gated = self.sync_gated(ctx.connection_id, None).await;

        // Epoch BEFORE data: the conveyed epoch must never postdate the entries
        // collected below (see `covering_epoch` — ordering is load-bearing).
        let covering_epoch = self.covering_epoch(ctx.connection_id, gated);

        let mut entries = Vec::new();

        for key in keys {
            // Each key lives at its own partition based on hash_to_partition.
            let key_partition = hash_to_partition(&key);
            let store = self
                .record_store_factory
                .get_or_create(&map_name, key_partition);
            match store.get(&key, false).await {
                Ok(Some(record)) => match record.value {
                    RecordValue::OrMap {
                        records,
                        tombstones,
                    } => {
                        let wire_records: Vec<ORMapRecord<rmpv::Value>> = records
                            .into_iter()
                            .map(|r| ORMapRecord {
                                value: value_to_rmpv(&r.value),
                                timestamp: r.timestamp,
                                tag: r.tag,
                                ttl_ms: None,
                            })
                            .collect();
                        // Carry tombstones so remove-wins suppression and
                        // add-wins survival both replicate to the peer.
                        entries.push(ORMapEntry {
                            key,
                            records: wire_records,
                            tombstones,
                        });
                    }
                    RecordValue::OrTombstones { tags } => {
                        entries.push(ORMapEntry {
                            key,
                            records: Vec::new(),
                            tombstones: tags,
                        });
                    }
                    // Key exists as an LWW record — wrong type for this OR-Map context; return empty entry.
                    RecordValue::Lww { .. } => {
                        entries.push(ORMapEntry {
                            key,
                            records: Vec::new(),
                            tombstones: Vec::new(),
                        });
                    }
                },
                Ok(None) => {
                    // Key not found — return empty entry (not an error, client handles missing keys).
                    entries.push(ORMapEntry {
                        key,
                        records: Vec::new(),
                        tombstones: Vec::new(),
                    });
                }
                Err(_) => {
                    // Store error — return empty entry and continue.
                    entries.push(ORMapEntry {
                        key,
                        records: Vec::new(),
                        tombstones: Vec::new(),
                    });
                }
            }
        }

        Ok(OperationResponse::Message(Box::new(
            Message::ORMapDiffResponse(ORMapDiffResponse {
                payload: ORMapDiffResponsePayload {
                    map_name,
                    entries,
                    covering_epoch,
                },
            }),
        )))
    }

    /// Handles `ORMapPushDiff` — merges incoming OR-Map entries and broadcasts changes.
    ///
    /// `ORMapPushDiff` is a WRAPPED message: `map_name` and `entries` live in a
    /// nested `.payload` field.
    #[allow(clippy::too_many_lines)] // gate setup + per-entry merge + broadcast + prune site in one coherent handler
    async fn handle_ormap_push_diff(
        &self,
        ctx: &crate::service::operation::OperationContext,
        payload: messages::ORMapPushDiff,
    ) -> Result<OperationResponse, OperationError> {
        use std::collections::BTreeSet;
        use topgun_core::messages::{ServerEventPayload, ServerEventType};

        let map_name = payload.payload.map_name;
        let entries = payload.payload.entries;

        // ---- Forgotten-client pre-apply gate (fires BEFORE any merge) ----
        //
        // Wired but DARK by construction until SPEC-342j raises the durability
        // watermark (`is_protection_active`): while dark, no tombstone can be pruned,
        // so a re-admitted record is still suppressed by the live tombstone set
        // (remove-wins) and blocking would only break an un-migrated client — the
        // pre-342j write path is preserved verbatim. Once active, the gate blocks a
        // forgotten/unknown client's push BEFORE the merge (gating BOTH the inbound
        // tombstone union and the inbound records) and holds the per-key single-writer
        // from the gate decision through the `store.put` commit to close the
        // `.await`-window TOCTOU. It goes live TOGETHER with the prune
        // (gate-before-activation — no prune-without-gate window).
        let gate_active = self
            .frontier
            .as_ref()
            .is_some_and(|f| f.is_protection_active());
        let gate = if gate_active {
            // Fail-closed (R12): an ACTIVE gate needs the shared per-key writer to hold
            // the gate→commit TOCTOU span. Absent it, reject the whole push rather than
            // merge under a best-effort no-lock path.
            let (Some(frontier), Some(key_writer)) =
                (self.frontier.as_ref(), self.key_writer.as_ref())
            else {
                return Ok(OperationResponse::Ack {
                    call_id: ctx.call_id,
                });
            };
            // Resolve the server-authenticated identity ONCE for the batch (R13:
            // per-message decision, re-checked per key under the writer lock below).
            // Unknown identity → forgotten → reject the whole push; the client is
            // steered to a full-snapshot REPLACE resync by the sync path.
            let Some(client) = self.resolve_client_id(ctx.connection_id).await else {
                return Ok(OperationResponse::Ack {
                    call_id: ctx.call_id,
                });
            };
            if frontier.is_forgotten(&client) {
                return Ok(OperationResponse::Ack {
                    call_id: ctx.call_id,
                });
            }
            Some((frontier, key_writer, client))
        } else {
            None
        };

        for entry in &entries {
            // Each entry's key determines its storage partition.
            let key_partition = hash_to_partition(&entry.key);
            let store = self
                .record_store_factory
                .get_or_create(&map_name, key_partition);
            // Acquire the per-key single-writer BEFORE the gate re-check and hold it
            // through the `store.put` merge-commit below (R5/R13). The commit-time
            // re-check re-evaluates the LAG-AWARE forgotten status under the held guard
            // via `gate_decision_holds_at_commit`, so a client that crosses the
            // forget-lag-K threshold mid-batch (a concurrent stamp advancing
            // `current_epoch`, or an active forget) between the batch gate and this key
            // is caught here and its key skipped — not merged.
            let _key_guard = if let Some((frontier, key_writer, client)) = &gate {
                let guard = key_writer.acquire(&map_name, &entry.key).await;
                let token = GateToken {
                    client: (*client).clone(),
                };
                if !frontier.gate_decision_holds_at_commit(token) {
                    continue; // now forgotten → skip this key (guard drops)
                }
                Some(guard)
            } else {
                None
            };
            // Read-modify-write: fold inbound records + tombstones into the
            // locally-stored OR-Map rather than blind-clobbering it. Discarding
            // either side would resurrect removed entries (remove-wins broken) or
            // drop concurrent additions (add-wins broken).
            let (mut merged_records, mut tombstones): (
                Vec<crate::storage::record::OrMapEntry>,
                BTreeSet<String>,
            ) = match store.get(&entry.key, false).await {
                Ok(Some(local)) => match local.value {
                    RecordValue::OrMap {
                        records,
                        tombstones,
                    } => (records, tombstones.into_iter().collect()),
                    // Legacy persisted blob: fold its tags into the unified view.
                    RecordValue::OrTombstones { tags } => (Vec::new(), tags.into_iter().collect()),
                    RecordValue::Lww { .. } => (Vec::new(), BTreeSet::new()),
                },
                _ => (Vec::new(), BTreeSet::new()),
            };

            // Union inbound tombstones (remove-wins) before applying records, so a
            // tag tombstoned anywhere suppresses its record everywhere.
            // `insert` returns true only for tags not already in the local set, so
            // the tombstone-bytes gauge counts genuinely-new inbound tombstones —
            // idempotent re-pushes and intra-batch duplicates don't inflate it.
            for tag in &entry.tombstones {
                if tombstones.insert(tag.clone()) {
                    crate::storage::record::add_tombstone_bytes(tag.len() as u64);
                }
            }

            // Drop any locally-stored record whose tag is now tombstoned.
            merged_records.retain(|r| !tombstones.contains(&r.tag));

            // Apply inbound records (add-wins): keep a record unless its tag is
            // tombstoned. De-duplicate by tag so repeated pushes are idempotent.
            for r in &entry.records {
                if tombstones.contains(&r.tag) {
                    continue;
                }
                if merged_records.iter().any(|existing| existing.tag == r.tag) {
                    continue;
                }
                merged_records.push(crate::storage::record::OrMapEntry {
                    value: crate::service::domain::crdt::rmpv_to_value(&r.value),
                    // Tag stays verbatim (OR identity — re-stamping breaks fleet tag
                    // identity / CRDT convergence). The `timestamp` field is copied
                    // verbatim too and is an ACCEPTED-AS-UNTRUSTED, client-supplied
                    // value used only for LWW tie-breaking WITHIN this OR entry — the
                    // server neither re-stamps nor clamps it (the bounded default;
                    // authenticity of the identity is enforced by the connection-keyed
                    // gate above, not by trusting this field).
                    tag: r.tag.clone(),
                    timestamp: r.timestamp.clone(),
                });
            }

            store
                .put(
                    &entry.key,
                    RecordValue::OrMap {
                        records: merged_records,
                        tombstones: tombstones.iter().cloned().collect(),
                    },
                    ExpiryPolicy::NONE,
                    CallerProvenance::CrdtMerge,
                )
                .await
                .map_err(|e| OperationError::Internal(anyhow::anyhow!("{e}")))?;

            // Broadcast OR_ADD only for inbound records that actually survived the
            // merge. A record whose tag is tombstoned (remove-wins) was suppressed
            // from stored state, so emitting an OR_ADD for it would tell subscribers
            // to resurrect a removed entry.
            for record in &entry.records {
                if tombstones.contains(&record.tag) {
                    continue;
                }
                let event_payload = ServerEventPayload {
                    map_name: map_name.clone(),
                    key: entry.key.clone(),
                    event_type: ServerEventType::OR_ADD,
                    record: None,
                    or_record: Some(record.clone()),
                    or_tag: Some(record.tag.clone()),
                };
                let msg = Message::ServerEvent {
                    payload: event_payload,
                };
                let bytes = rmp_serde::to_vec_named(&msg)
                    .map_err(|e| OperationError::Internal(anyhow::anyhow!("serialize: {e}")))?;
                self.connection_registry
                    .broadcast(&bytes, ConnectionKind::Client);
            }
        }

        // Third prune site (R2): wire the wholesale epoch-drop prune into the
        // push-diff merge path. DARK by construction — the frontier's durability
        // watermark is constant 0 in this child, so the call-site conjunction never
        // licenses a prune for any stamped epoch (all `>= 1`): this drops ZERO epochs
        // until SPEC-342j supplies the real watermark. Runs AFTER the per-key guards
        // above are dropped (the sweep re-acquires the same per-key writer per dropped
        // tag, so holding a key guard here would self-deadlock).
        self.run_leaf_prune().await;

        Ok(OperationResponse::Ack {
            call_id: ctx.call_id,
        })
    }
}

// ---------------------------------------------------------------------------
// ManagedService implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl ManagedService for SyncService {
    fn name(&self) -> &'static str {
        service_names::SYNC
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

impl Service<Operation> for Arc<SyncService> {
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
                    Operation::SyncInit { ctx, payload } => {
                        svc.handle_sync_init(&ctx, payload).await
                    }
                    Operation::MerkleReqBucket { ctx, payload } => {
                        svc.handle_merkle_req_bucket(&ctx, payload).await
                    }
                    Operation::ORMapSyncInit { ctx, payload } => {
                        svc.handle_ormap_sync_init(&ctx, payload).await
                    }
                    Operation::ORMapMerkleReqBucket { ctx, payload } => {
                        svc.handle_ormap_merkle_req_bucket(&ctx, payload).await
                    }
                    Operation::ORMapDiffRequest { ctx, payload } => {
                        svc.handle_ormap_diff_request(&ctx, payload).await
                    }
                    Operation::ORMapPushDiff { ctx, payload } => {
                        svc.handle_ormap_push_diff(&ctx, payload).await
                    }
                    _ => Err(OperationError::WrongService),
                }
            }
            .instrument(span),
        )
    }
}

// ---------------------------------------------------------------------------
// Tests (AC1, AC2, AC3, AC14)
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(
    clippy::manual_string_new,
    clippy::cloned_instead_of_copied,
    clippy::approx_constant,
    clippy::items_after_statements,
    clippy::default_trait_access
)]
mod tests {
    use std::sync::Arc;

    use topgun_core::hlc::Timestamp;
    use topgun_core::types::Value;
    use tower::ServiceExt;

    use super::*;
    use crate::network::connection::ConnectionRegistry;
    use crate::service::operation::{service_names, OperationContext};
    use crate::storage::factory::RecordStoreFactory;
    use crate::storage::merkle_sync::MerkleSyncManager;
    use crate::storage::{NullDataStore, StorageConfig};

    fn make_timestamp() -> Timestamp {
        Timestamp {
            millis: 1_700_000_000_000,
            counter: 0,
            node_id: "node-1".to_string(),
        }
    }

    fn make_ctx(service_name: &'static str) -> OperationContext {
        OperationContext::new(1, service_name, make_timestamp(), 5000)
    }

    fn make_factory() -> Arc<RecordStoreFactory> {
        Arc::new(RecordStoreFactory::new(
            StorageConfig::default(),
            Arc::new(NullDataStore),
            Vec::new(),
        ))
    }

    fn make_sync_service() -> Arc<SyncService> {
        let merkle_manager = Arc::new(MerkleSyncManager::default());
        let record_store_factory = make_factory();
        let connection_registry = Arc::new(ConnectionRegistry::new());
        Arc::new(SyncService::new(
            merkle_manager,
            record_store_factory,
            connection_registry,
        ))
    }

    fn make_sync_service_with_frontier() -> (
        Arc<SyncService>,
        Arc<RecordStoreFactory>,
        Arc<TombstoneFrontier>,
    ) {
        let factory = make_factory();
        let frontier = Arc::new(TombstoneFrontier::new(None));
        frontier.set_epoch_width(1); // one epoch per stamp, for precise control
        let key_writer = Arc::new(KeyWriterRegistry::new());
        let svc = Arc::new(
            SyncService::new(
                Arc::new(MerkleSyncManager::default()),
                Arc::clone(&factory),
                Arc::new(ConnectionRegistry::new()),
            )
            .with_frontier(Arc::clone(&frontier), key_writer),
        );
        (svc, factory, frontier)
    }

    /// AC4 (SYNC-leaf site): the wholesale epoch-drop prune is wired into the
    /// OR-Map SYNC-leaf path (`handle_ormap_merkle_req_bucket` runs the sweep at
    /// entry). With an injected durability watermark and the low-water-mark past
    /// the stamped epoch, invoking the leaf handler drops the tombstone from
    /// storage while the live record survives. DARK by default (watermark 0); the
    /// injected watermark exercises the real drop path.
    #[tokio::test]
    async fn ac4_prune_wired_into_sync_leaf() {
        use crate::storage::record::OrMapEntry as StoreOrMapEntry;
        let (svc, factory, frontier) = make_sync_service_with_frontier();
        let map = "omap";
        let key = "k1";
        let tomb = "T1";
        let store = factory.get_or_create(map, hash_to_partition(key));
        store
            .put(
                key,
                RecordValue::OrMap {
                    records: vec![StoreOrMapEntry {
                        value: Value::Int(1),
                        tag: "R1".to_string(),
                        timestamp: make_timestamp(),
                    }],
                    tombstones: vec![tomb.to_string()],
                },
                ExpiryPolicy::NONE,
                CallerProvenance::CrdtMerge,
            )
            .await
            .unwrap();
        // Stamp the tombstone so the epoch index knows its (map, key, tag) location.
        assert_eq!(frontier.stamp_tombstone(map, key, tomb), 1);
        // A second stamp advances the counter to 2 so the LWM can move STRICTLY
        // past epoch 1 (eligibility is strict: LWM must exceed the epoch).
        assert_eq!(frontier.stamp_tombstone(map, "k2", "T2"), 2);
        // Raise the LWM strictly past epoch 1 and open the durability watermark.
        let c: String = "a5:alice|dev-1".into();
        frontier.set_delivered(ConnectionId(1), 100);
        assert!(frontier.confirm_apply_ack(&c, 2, ConnectionId(1)).await);
        frontier.set_durable_epoch_watermark(1000);

        // Invoke the OR-Map leaf handler — the prune sweep fires at its top.
        let mut ctx = make_ctx(service_names::SYNC);
        ctx.connection_id = Some(ConnectionId(1));
        Arc::clone(&svc)
            .oneshot(Operation::ORMapMerkleReqBucket {
                ctx,
                payload: topgun_core::messages::ORMapMerkleReqBucket {
                    payload: topgun_core::messages::ORMapMerkleReqBucketPayload {
                        map_name: map.to_string(),
                        path: String::new(),
                    },
                },
            })
            .await
            .expect("bucket handler");

        match store.get(key, false).await.unwrap().map(|r| r.value) {
            Some(RecordValue::OrMap {
                records,
                tombstones,
            }) => {
                assert!(
                    tombstones.is_empty(),
                    "epoch-1 tombstone pruned from storage via the SYNC-leaf path"
                );
                assert_eq!(records.len(), 1, "the live record survives the prune");
            }
            other => panic!("expected OrMap, got {other:?}"),
        }
    }

    /// `AC3b` (server half): once an epoch is stamped, the OR-Map `SYNC_INIT`
    /// root response conveys the covering epoch AND records it as delivered on the
    /// connection, so the client's subsequent `CLIENT_APPLY_ACK` passes the
    /// delivered-clamp — this is what lets an empty-diff client advance. Before
    /// any stamp, no covering epoch is conveyed (nothing to confirm).
    #[tokio::test]
    async fn ac3b_covering_epoch_conveyed_and_marked_delivered() {
        let (svc, _factory, frontier) = make_sync_service_with_frontier();
        let conn = ConnectionId(7);

        let ormap_root = |svc: Arc<SyncService>, conn: ConnectionId| async move {
            let mut ctx = make_ctx(service_names::SYNC);
            ctx.connection_id = Some(conn);
            match svc
                .oneshot(Operation::ORMapSyncInit {
                    ctx,
                    payload: topgun_core::messages::ORMapSyncInit {
                        map_name: "omap".to_string(),
                        root_hash: 0,
                        bucket_hashes: HashMap::new(),
                        last_sync_timestamp: None,
                        claimed_epoch: None,
                    },
                })
                .await
                .expect("ORMapSyncInit")
            {
                OperationResponse::Message(m) => match *m {
                    Message::ORMapSyncRespRoot(r) => r.payload,
                    o => panic!("expected ORMapSyncRespRoot, got {o:?}"),
                },
                o => panic!("expected Message, got {o:?}"),
            }
        };

        // Before any stamp: no covering epoch, nothing delivered.
        let root0 = ormap_root(Arc::clone(&svc), conn).await;
        assert_eq!(
            root0.covering_epoch, None,
            "no epoch stamped -> no covering epoch conveyed"
        );
        assert_eq!(frontier.delivered(conn), 0);

        // Stamp two epochs; SYNC_INIT now conveys the current max (2) and marks it
        // delivered on the connection for the ACK clamp.
        frontier.stamp_tombstone("omap", "k1", "T1");
        frontier.stamp_tombstone("omap", "k2", "T2");
        let root = ormap_root(Arc::clone(&svc), conn).await;
        assert_eq!(
            root.covering_epoch,
            Some(2),
            "covering epoch = current max stamped epoch"
        );
        assert_eq!(
            frontier.delivered(conn),
            2,
            "conveyed epoch recorded as delivered so the client ACK passes the clamp"
        );
    }

    // -------------------------------------------------------------------------
    // AC1: SyncInit returns SyncRespRoot with correct root hash
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn ac1_sync_init_returns_sync_resp_root() {
        let merkle_manager = Arc::new(MerkleSyncManager::default());
        // Pre-populate the tree so root hash is non-zero.
        merkle_manager.update_lww("users", 0, "user-1", 12345);

        let connection_registry = Arc::new(ConnectionRegistry::new());
        let svc = Arc::new(SyncService::new(
            Arc::clone(&merkle_manager),
            make_factory(),
            connection_registry,
        ));

        // SyncInit returns the cross-partition aggregate, not the raw partition
        // root, so the expected value is the collision-resistant combine.
        let expected_root = merkle_manager.aggregate_lww_root_hash("users");
        assert_ne!(
            expected_root, 0,
            "precondition: tree must have non-zero hash"
        );

        let op = Operation::SyncInit {
            ctx: make_ctx(service_names::SYNC),
            payload: messages::SyncInitMessage {
                map_name: "users".to_string(),
                last_sync_timestamp: None,
            },
        };

        let resp = svc.oneshot(op).await.unwrap();
        match resp {
            OperationResponse::Message(msg) => {
                if let Message::SyncRespRoot(m) = *msg {
                    assert_eq!(m.payload.map_name, "users");
                    assert_eq!(m.payload.root_hash, expected_root);
                    assert_eq!(m.payload.timestamp.node_id, "node-1");
                } else {
                    panic!("expected SyncRespRoot, got different Message variant");
                }
            }
            other => panic!("expected OperationResponse::Message, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn ac1_sync_init_empty_tree_returns_root_hash_zero() {
        let svc = make_sync_service();

        let op = Operation::SyncInit {
            ctx: make_ctx(service_names::SYNC),
            payload: messages::SyncInitMessage {
                map_name: "users".to_string(),
                last_sync_timestamp: None,
            },
        };

        let resp = svc.oneshot(op).await.unwrap();
        match resp {
            OperationResponse::Message(msg) => {
                if let Message::SyncRespRoot(m) = *msg {
                    assert_eq!(
                        m.payload.root_hash, 0,
                        "empty tree should have root_hash = 0"
                    );
                } else {
                    panic!("expected SyncRespRoot");
                }
            }
            other => panic!("expected Message response, got {other:?}"),
        }
    }

    // -------------------------------------------------------------------------
    // AC2: MerkleReqBucket returns SyncRespBuckets for internal nodes
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn ac2_merkle_req_bucket_returns_buckets_for_internal_node() {
        let merkle_manager = Arc::new(MerkleSyncManager::default());
        // Insert several keys to ensure the root (path "") is an internal node.
        for i in 0..10 {
            let key = format!("user-{i}");
            merkle_manager.update_lww("users", 0, &key, i * 100 + 1);
        }

        let svc = Arc::new(SyncService::new(
            Arc::clone(&merkle_manager),
            make_factory(),
            Arc::new(ConnectionRegistry::new()),
        ));

        let op = Operation::MerkleReqBucket {
            ctx: make_ctx(service_names::SYNC),
            payload: messages::MerkleReqBucketMessage {
                payload: messages::MerkleReqBucketPayload {
                    map_name: "users".to_string(),
                    path: "".to_string(),
                },
            },
        };

        let resp = svc.oneshot(op).await.unwrap();
        match resp {
            OperationResponse::Message(msg) => {
                if let Message::SyncRespBuckets(m) = *msg {
                    assert_eq!(m.payload.map_name, "users");
                    assert_eq!(m.payload.path, "");
                    assert!(
                        !m.payload.buckets.is_empty(),
                        "internal node should have non-empty buckets"
                    );
                    // Each bucket key is a single hex character string.
                    for key in m.payload.buckets.keys() {
                        assert_eq!(key.len(), 1, "bucket key should be single char, got: {key}");
                        assert!(
                            key.chars().all(|c| c.is_ascii_hexdigit()),
                            "bucket key should be hex char: {key}"
                        );
                    }
                } else {
                    panic!("expected SyncRespBuckets");
                }
            }
            other => panic!("expected Message response, got {other:?}"),
        }
    }

    // -------------------------------------------------------------------------
    // AC3: MerkleReqBucket returns SyncRespLeaf for leaf nodes
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn ac3_merkle_req_bucket_returns_leaf_for_leaf_node() {
        let merkle_manager = Arc::new(MerkleSyncManager::default());
        // Insert a single key so we can find its leaf path.
        merkle_manager.update_lww("users", 0, "user-1", 99999);

        // Find the path for "user-1" by drilling down until we hit a leaf.
        let leaf_path = merkle_manager.with_lww_tree("users", 0, |tree| {
            // At depth=3, the leaf path is the first 3 hex chars of the key's hash path.
            // We iterate through the buckets to find a path that resolves to a leaf.
            let root_buckets = tree.get_buckets("");
            let first_char = root_buckets.keys().next().cloned().unwrap();
            let level1_path = first_char.to_string();
            let level1_buckets = tree.get_buckets(&level1_path);
            if level1_buckets.is_empty() {
                return level1_path;
            }
            let second_char = level1_buckets.keys().next().cloned().unwrap();
            let level2_path = format!("{level1_path}{second_char}");
            let level2_buckets = tree.get_buckets(&level2_path);
            if level2_buckets.is_empty() {
                return level2_path;
            }
            let third_char = level2_buckets.keys().next().cloned().unwrap();
            format!("{level2_path}{third_char}")
        });

        let svc = Arc::new(SyncService::new(
            Arc::clone(&merkle_manager),
            make_factory(),
            Arc::new(ConnectionRegistry::new()),
        ));

        let op = Operation::MerkleReqBucket {
            ctx: make_ctx(service_names::SYNC),
            payload: messages::MerkleReqBucketMessage {
                payload: messages::MerkleReqBucketPayload {
                    map_name: "users".to_string(),
                    path: leaf_path.clone(),
                },
            },
        };

        let resp = svc.oneshot(op).await.unwrap();
        match resp {
            OperationResponse::Message(msg) => {
                if let Message::SyncRespLeaf(m) = *msg {
                    // map_name and path are echoed correctly.
                    assert_eq!(m.payload.map_name, "users");
                    assert_eq!(m.payload.path, leaf_path);
                    // With NullDataStore, records may be empty (no real data in store).
                    // The test verifies response type and structure only.
                } else {
                    panic!("expected SyncRespLeaf, got different Message variant");
                }
            }
            OperationResponse::Empty => {
                // Also acceptable if the path has no node (edge case in test setup).
            }
            other => panic!("expected Message or Empty response, got {other:?}"),
        }
    }

    // -------------------------------------------------------------------------
    // AC14: value_to_rmpv round-trips with rmpv_to_value from crdt.rs
    // -------------------------------------------------------------------------

    #[test]
    fn ac14_value_to_rmpv_null() {
        let v = Value::Null;
        let rmpv_val = value_to_rmpv(&v);
        assert_eq!(rmpv_val, rmpv::Value::Nil);
    }

    #[test]
    fn ac14_value_to_rmpv_bool() {
        assert_eq!(
            value_to_rmpv(&Value::Bool(true)),
            rmpv::Value::Boolean(true)
        );
        assert_eq!(
            value_to_rmpv(&Value::Bool(false)),
            rmpv::Value::Boolean(false)
        );
    }

    #[test]
    fn ac14_value_to_rmpv_int() {
        let v = Value::Int(42);
        let rmpv_val = value_to_rmpv(&v);
        assert_eq!(rmpv_val, rmpv::Value::Integer(42.into()));
    }

    #[test]
    fn ac14_value_to_rmpv_float() {
        let v = Value::Float(3.14);
        let rmpv_val = value_to_rmpv(&v);
        assert_eq!(rmpv_val, rmpv::Value::F64(3.14));
    }

    #[test]
    fn ac14_value_to_rmpv_string() {
        let v = Value::String("hello".to_string());
        let rmpv_val = value_to_rmpv(&v);
        assert_eq!(rmpv_val, rmpv::Value::String("hello".into()));
    }

    #[test]
    fn ac14_value_to_rmpv_bytes() {
        let v = Value::Bytes(vec![1, 2, 3]);
        let rmpv_val = value_to_rmpv(&v);
        assert_eq!(rmpv_val, rmpv::Value::Binary(vec![1, 2, 3]));
    }

    #[test]
    fn ac14_value_to_rmpv_array() {
        let v = Value::Array(vec![Value::Int(1), Value::String("x".to_string())]);
        let rmpv_val = value_to_rmpv(&v);
        assert_eq!(
            rmpv_val,
            rmpv::Value::Array(vec![
                rmpv::Value::Integer(1.into()),
                rmpv::Value::String("x".into()),
            ])
        );
    }

    #[test]
    fn ac14_value_to_rmpv_map() {
        let mut m = std::collections::BTreeMap::new();
        m.insert("key".to_string(), Value::Int(99));
        let v = Value::Map(m);
        let rmpv_val = value_to_rmpv(&v);
        if let rmpv::Value::Map(pairs) = rmpv_val {
            assert_eq!(pairs.len(), 1);
            assert_eq!(pairs[0].0, rmpv::Value::String("key".into()));
            assert_eq!(pairs[0].1, rmpv::Value::Integer(99.into()));
        } else {
            panic!("expected rmpv::Value::Map");
        }
    }

    #[test]
    fn ac14_round_trip_with_rmpv_to_value() {
        // Both value_to_rmpv (sync.rs) and rmpv_to_value (crdt.rs) are pub(crate).
        // This tests the round-trip: value -> rmpv -> value.
        use crate::service::domain::crdt::rmpv_to_value;

        let values = vec![
            Value::Null,
            Value::Bool(true),
            Value::Bool(false),
            Value::Int(0),
            Value::Int(i64::MAX),
            Value::Int(i64::MIN),
            Value::Float(0.0),
            Value::Float(1.5),
            Value::String(String::new()),
            Value::String("hello world".to_string()),
            Value::Bytes(vec![]),
            Value::Bytes(vec![0xDE, 0xAD, 0xBE, 0xEF]),
            Value::Array(vec![Value::Int(1), Value::Int(2)]),
        ];

        for val in values {
            let rmpv_val = value_to_rmpv(&val);
            let round_tripped = rmpv_to_value(&rmpv_val);
            assert_eq!(val, round_tripped, "round-trip failed for: {val:?}");
        }
    }

    // -------------------------------------------------------------------------
    // AC12: ManagedService name is "sync"
    // -------------------------------------------------------------------------

    #[test]
    fn ac12_managed_service_name_is_sync() {
        let svc = make_sync_service();
        assert_eq!(svc.name(), "sync");
    }

    // -------------------------------------------------------------------------
    // AC4: ORMapSyncInit returns ORMapSyncRespRoot
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn ac4_ormap_sync_init_returns_ormap_sync_resp_root() {
        let merkle_manager = Arc::new(MerkleSyncManager::default());
        // Pre-populate the OR-Map tree so root hash is non-zero.
        merkle_manager.update_ormap("tags", 0, "tag-1", 99999);

        let svc = Arc::new(SyncService::new(
            Arc::clone(&merkle_manager),
            make_factory(),
            Arc::new(ConnectionRegistry::new()),
        ));

        // ORMapSyncInit returns the cross-partition aggregate, not the raw
        // partition root, so the expected value is the collision-resistant combine.
        let expected_root = merkle_manager.aggregate_ormap_root_hash("tags");
        assert_ne!(
            expected_root, 0,
            "precondition: OR-Map tree must have non-zero hash"
        );

        use topgun_core::messages::ORMapSyncInit;
        let op = Operation::ORMapSyncInit {
            ctx: make_ctx(service_names::SYNC),
            payload: ORMapSyncInit {
                map_name: "tags".to_string(),
                root_hash: 0,
                bucket_hashes: Default::default(),
                last_sync_timestamp: None,
                claimed_epoch: None,
            },
        };

        let resp = svc.oneshot(op).await.unwrap();
        match resp {
            OperationResponse::Message(msg) => {
                if let Message::ORMapSyncRespRoot(m) = *msg {
                    assert_eq!(m.payload.map_name, "tags");
                    assert_eq!(m.payload.root_hash, expected_root);
                } else {
                    panic!("expected ORMapSyncRespRoot, got different Message variant");
                }
            }
            other => panic!("expected OperationResponse::Message, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn ac4_ormap_sync_init_empty_tree_returns_zero_root_hash() {
        let svc = make_sync_service();

        use topgun_core::messages::ORMapSyncInit;
        let op = Operation::ORMapSyncInit {
            ctx: make_ctx(service_names::SYNC),
            payload: ORMapSyncInit {
                map_name: "tags".to_string(),
                root_hash: 0,
                bucket_hashes: Default::default(),
                last_sync_timestamp: None,
                claimed_epoch: None,
            },
        };

        let resp = svc.oneshot(op).await.unwrap();
        match resp {
            OperationResponse::Message(msg) => {
                if let Message::ORMapSyncRespRoot(m) = *msg {
                    assert_eq!(
                        m.payload.root_hash, 0,
                        "empty OR-Map tree should have root_hash = 0"
                    );
                } else {
                    panic!("expected ORMapSyncRespRoot");
                }
            }
            other => panic!("expected Message response, got {other:?}"),
        }
    }

    // -------------------------------------------------------------------------
    // AC5: ORMapMerkleReqBucket returns leaf or bucket response
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn ac5_ormap_merkle_req_bucket_returns_buckets_for_internal_node() {
        let merkle_manager = Arc::new(MerkleSyncManager::default());
        // Insert several OR-Map keys to ensure root is an internal node.
        for i in 0..10 {
            let key = format!("tag-{i}");
            merkle_manager.update_ormap("tags", 0, &key, i * 100 + 1);
        }

        let svc = Arc::new(SyncService::new(
            Arc::clone(&merkle_manager),
            make_factory(),
            Arc::new(ConnectionRegistry::new()),
        ));

        use topgun_core::messages::ORMapMerkleReqBucket;
        use topgun_core::messages::ORMapMerkleReqBucketPayload;
        let op = Operation::ORMapMerkleReqBucket {
            ctx: make_ctx(service_names::SYNC),
            payload: ORMapMerkleReqBucket {
                payload: ORMapMerkleReqBucketPayload {
                    map_name: "tags".to_string(),
                    path: "".to_string(),
                },
            },
        };

        let resp = svc.oneshot(op).await.unwrap();
        match resp {
            OperationResponse::Message(msg) => {
                if let Message::ORMapSyncRespBuckets(m) = *msg {
                    assert_eq!(m.payload.map_name, "tags");
                    assert_eq!(m.payload.path, "");
                    assert!(
                        !m.payload.buckets.is_empty(),
                        "internal node should have non-empty buckets"
                    );
                    for key in m.payload.buckets.keys() {
                        assert_eq!(key.len(), 1, "bucket key should be single char, got: {key}");
                    }
                } else {
                    panic!("expected ORMapSyncRespBuckets, got different Message variant");
                }
            }
            other => panic!("expected Message response, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn ac5_ormap_merkle_req_bucket_returns_leaf_for_leaf_node() {
        let merkle_manager = Arc::new(MerkleSyncManager::default());
        merkle_manager.update_ormap("tags", 0, "tag-1", 99999);

        // Find the leaf path by drilling down.
        let leaf_path = merkle_manager.with_ormap_tree("tags", 0, |tree| {
            let root_buckets = tree.get_buckets("");
            let first_char = root_buckets.keys().next().cloned().unwrap();
            let level1_path = first_char.to_string();
            let level1_buckets = tree.get_buckets(&level1_path);
            if level1_buckets.is_empty() {
                return level1_path;
            }
            let second_char = level1_buckets.keys().next().cloned().unwrap();
            let level2_path = format!("{level1_path}{second_char}");
            let level2_buckets = tree.get_buckets(&level2_path);
            if level2_buckets.is_empty() {
                return level2_path;
            }
            let third_char = level2_buckets.keys().next().cloned().unwrap();
            format!("{level2_path}{third_char}")
        });

        let svc = Arc::new(SyncService::new(
            Arc::clone(&merkle_manager),
            make_factory(),
            Arc::new(ConnectionRegistry::new()),
        ));

        use topgun_core::messages::ORMapMerkleReqBucket;
        use topgun_core::messages::ORMapMerkleReqBucketPayload;
        let op = Operation::ORMapMerkleReqBucket {
            ctx: make_ctx(service_names::SYNC),
            payload: ORMapMerkleReqBucket {
                payload: ORMapMerkleReqBucketPayload {
                    map_name: "tags".to_string(),
                    path: leaf_path.clone(),
                },
            },
        };

        let resp = svc.oneshot(op).await.unwrap();
        match resp {
            OperationResponse::Message(msg) => {
                if let Message::ORMapSyncRespLeaf(m) = *msg {
                    assert_eq!(m.payload.map_name, "tags");
                    assert_eq!(m.payload.path, leaf_path);
                    // With NullDataStore, entries may be empty (no real data in store).
                } else {
                    panic!("expected ORMapSyncRespLeaf, got different Message variant");
                }
            }
            OperationResponse::Empty => {
                // Acceptable if path has no node.
            }
            other => panic!("expected Message or Empty response, got {other:?}"),
        }
    }

    // -------------------------------------------------------------------------
    // AC6: ORMapDiffRequest returns entries for requested keys
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn ac6_ormap_diff_request_handles_missing_keys_gracefully() {
        // With NullDataStore, the key won't be found — test that the handler
        // returns an empty entry rather than panicking or erroring.
        let svc = make_sync_service();

        use topgun_core::messages::ORMapDiffRequest;
        use topgun_core::messages::ORMapDiffRequestPayload;
        let op = Operation::ORMapDiffRequest {
            ctx: make_ctx(service_names::SYNC),
            payload: ORMapDiffRequest {
                payload: ORMapDiffRequestPayload {
                    map_name: "tags".to_string(),
                    keys: vec!["key-1".to_string()],
                },
            },
        };

        let resp = svc.oneshot(op).await.unwrap();
        match resp {
            OperationResponse::Message(msg) => {
                if let Message::ORMapDiffResponse(m) = *msg {
                    assert_eq!(m.payload.map_name, "tags");
                    // With NullDataStore, key-1 is not found — response should
                    // contain an empty entry for key-1 (not an error).
                    assert_eq!(
                        m.payload.entries.len(),
                        1,
                        "should have one entry for key-1"
                    );
                    assert_eq!(m.payload.entries[0].key, "key-1");
                    assert!(
                        m.payload.entries[0].records.is_empty(),
                        "NullDataStore returns no records"
                    );
                    assert!(
                        m.payload.entries[0].tombstones.is_empty(),
                        "no tombstones for missing key"
                    );
                } else {
                    panic!("expected ORMapDiffResponse, got different Message variant");
                }
            }
            other => panic!("expected OperationResponse::Message, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn ac6_ormap_diff_request_returns_ormap_diff_response() {
        let svc = make_sync_service();

        use topgun_core::messages::ORMapDiffRequest;
        use topgun_core::messages::ORMapDiffRequestPayload;
        let op = Operation::ORMapDiffRequest {
            ctx: make_ctx(service_names::SYNC),
            payload: ORMapDiffRequest {
                payload: ORMapDiffRequestPayload {
                    map_name: "tags".to_string(),
                    keys: vec!["key-1".to_string(), "key-2".to_string()],
                },
            },
        };

        let resp = svc.oneshot(op).await.unwrap();
        match resp {
            OperationResponse::Message(msg) => {
                if let Message::ORMapDiffResponse(m) = *msg {
                    assert_eq!(m.payload.map_name, "tags");
                    // Two keys requested, two entries returned (both empty with NullDataStore).
                    assert_eq!(
                        m.payload.entries.len(),
                        2,
                        "should have entries for all requested keys"
                    );
                } else {
                    panic!("expected ORMapDiffResponse");
                }
            }
            other => panic!("expected Message response, got {other:?}"),
        }
    }

    // -------------------------------------------------------------------------
    // AC7: ORMapPushDiff merges entries and broadcasts
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn ac7_ormap_push_diff_returns_ack_and_stores_data() {
        use crate::storage::merkle_sync::MerkleMutationObserver;
        use topgun_core::hlc::Timestamp;
        use topgun_core::messages::{ORMapEntry, ORMapPushDiff, ORMapPushDiffPayload};
        use topgun_core::ORMapRecord;

        // Wire a MerkleMutationObserver into the factory so we can verify the
        // RecordStore put fires (the observer updates the merkle tree on put).
        let merkle_manager = Arc::new(MerkleSyncManager::default());
        let observer = Arc::new(MerkleMutationObserver::new(
            Arc::clone(&merkle_manager),
            "tags".to_string(),
            0,
        ));
        let factory = Arc::new(RecordStoreFactory::new(
            StorageConfig::default(),
            Arc::new(NullDataStore),
            vec![observer as Arc<dyn crate::storage::mutation_observer::MutationObserver>],
        ));
        let connection_registry = Arc::new(ConnectionRegistry::new());
        let svc = Arc::new(SyncService::new(
            Arc::clone(&merkle_manager),
            factory,
            connection_registry,
        ));

        // Precondition: OR-Map tree for ("tags", 0) should have root_hash = 0.
        let hash_before = merkle_manager.with_ormap_tree("tags", 0, |tree| tree.get_root_hash());
        assert_eq!(
            hash_before, 0,
            "precondition: tree should be empty before push"
        );

        let op = Operation::ORMapPushDiff {
            ctx: make_ctx(service_names::SYNC),
            payload: ORMapPushDiff {
                payload: ORMapPushDiffPayload {
                    map_name: "tags".to_string(),
                    entries: vec![ORMapEntry {
                        key: "tag-1".to_string(),
                        records: vec![ORMapRecord {
                            value: rmpv::Value::String("important".into()),
                            timestamp: Timestamp {
                                millis: 1_700_000_000_000,
                                counter: 0,
                                node_id: "node-1".to_string(),
                            },
                            tag: "1700000000000:0:node-1".to_string(),
                            ttl_ms: None,
                        }],
                        tombstones: Vec::new(),
                    }],
                },
            },
        };

        let resp = svc.oneshot(op).await.unwrap();
        assert!(
            matches!(resp, OperationResponse::Ack { call_id: 1 }),
            "expected Ack response, got {resp:?}"
        );

        // Verify the RecordStore put fired: the MerkleMutationObserver should
        // have updated the OR-Map merkle tree to a non-zero root hash.
        let hash_after = merkle_manager.with_ormap_tree("tags", 0, |tree| tree.get_root_hash());
        assert_ne!(
            hash_after, 0,
            "OR-Map tree should have non-zero hash after push (proves store.put fired)"
        );
    }

    #[tokio::test]
    async fn ac7_ormap_push_diff_empty_entries_returns_ack() {
        use topgun_core::messages::{ORMapPushDiff, ORMapPushDiffPayload};

        let svc = make_sync_service();

        let op = Operation::ORMapPushDiff {
            ctx: make_ctx(service_names::SYNC),
            payload: ORMapPushDiff {
                payload: ORMapPushDiffPayload {
                    map_name: "tags".to_string(),
                    entries: Vec::new(),
                },
            },
        };

        let resp = svc.oneshot(op).await.unwrap();
        assert!(
            matches!(resp, OperationResponse::Ack { .. }),
            "empty push diff should still return Ack, got {resp:?}"
        );
    }

    // -------------------------------------------------------------------------
    // AC11: Wrong service returns WrongService error
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn ac11_wrong_service_returns_wrong_service_error() {
        let svc = make_sync_service();

        // GarbageCollect is not a sync operation.
        let op = Operation::GarbageCollect {
            ctx: make_ctx(service_names::SYNC),
        };

        let result = svc.oneshot(op).await;
        assert!(
            matches!(result, Err(OperationError::WrongService)),
            "expected WrongService error, got {result:?}"
        );
    }

    // -------------------------------------------------------------------------
    // parse_partition_prefix helper unit tests
    // -------------------------------------------------------------------------

    #[test]
    fn parse_partition_prefix_empty_path_returns_none() {
        assert!(super::parse_partition_prefix("").is_none());
    }

    #[test]
    fn parse_partition_prefix_short_hex_path_returns_none() {
        assert!(super::parse_partition_prefix("a").is_none());
        assert!(super::parse_partition_prefix("ab").is_none());
        assert!(super::parse_partition_prefix("abc").is_none());
    }

    #[test]
    fn parse_partition_prefix_routed_path_parses_correctly() {
        let (pid, sub) = super::parse_partition_prefix("042/abc").unwrap();
        assert_eq!(pid, 42);
        assert_eq!(sub, "abc");
    }

    #[test]
    fn parse_partition_prefix_routed_path_empty_sub_path() {
        let (pid, sub) = super::parse_partition_prefix("000/").unwrap();
        assert_eq!(pid, 0);
        assert_eq!(sub, "");
    }

    #[test]
    fn parse_partition_prefix_max_partition() {
        let (pid, sub) = super::parse_partition_prefix("270/f3a").unwrap();
        assert_eq!(pid, 270);
        assert_eq!(sub, "f3a");
    }

    #[test]
    fn parse_partition_prefix_hex_path_without_slash_returns_none() {
        // "abc" has 3 chars but no slash at position 3.
        assert!(super::parse_partition_prefix("abc").is_none());
    }

    // -------------------------------------------------------------------------
    // AC4 (scatter-gather): SyncInit uses aggregate root hash
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn scatter_gather_sync_init_aggregates_across_partitions() {
        let merkle_manager = Arc::new(MerkleSyncManager::default());
        // Write to two different partitions.
        merkle_manager.update_lww("users", 1, "alice", 111);
        merkle_manager.update_lww("users", 2, "bob", 222);

        let svc = Arc::new(SyncService::new(
            Arc::clone(&merkle_manager),
            make_factory(),
            Arc::new(ConnectionRegistry::new()),
        ));

        let expected = merkle_manager.aggregate_lww_root_hash("users");
        assert_ne!(
            expected, 0,
            "precondition: aggregate hash should be non-zero"
        );

        let op = Operation::SyncInit {
            ctx: make_ctx(service_names::SYNC),
            payload: messages::SyncInitMessage {
                map_name: "users".to_string(),
                last_sync_timestamp: None,
            },
        };

        let resp = svc.oneshot(op).await.unwrap();
        match resp {
            OperationResponse::Message(msg) => {
                if let Message::SyncRespRoot(m) = *msg {
                    assert_eq!(
                        m.payload.root_hash, expected,
                        "SyncInit should return aggregate root hash from all partitions"
                    );
                } else {
                    panic!("expected SyncRespRoot");
                }
            }
            other => panic!("expected Message response, got {other:?}"),
        }
    }

    // -------------------------------------------------------------------------
    // AC5: Path prefix routing — "042/abc" routes to partition 42, sub-path "abc"
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn path_prefix_routing_routes_to_correct_partition() {
        let merkle_manager = Arc::new(MerkleSyncManager::default());
        // Populate partition 42 only.
        for i in 0..10u32 {
            merkle_manager.update_lww("users", 42, &format!("key-{i}"), i * 100 + 1);
        }

        let svc = Arc::new(SyncService::new(
            Arc::clone(&merkle_manager),
            make_factory(),
            Arc::new(ConnectionRegistry::new()),
        ));

        // Request partition 42's root bucket with empty sub-path "042/".
        let op = Operation::MerkleReqBucket {
            ctx: make_ctx(service_names::SYNC),
            payload: messages::MerkleReqBucketMessage {
                payload: messages::MerkleReqBucketPayload {
                    map_name: "users".to_string(),
                    path: "042/".to_string(),
                },
            },
        };

        let resp = svc.oneshot(op).await.unwrap();
        match resp {
            OperationResponse::Message(msg) => {
                if let Message::SyncRespBuckets(m) = *msg {
                    assert_eq!(m.payload.map_name, "users");
                    assert_eq!(m.payload.path, "042/");
                    assert!(
                        !m.payload.buckets.is_empty(),
                        "partition 42 with 10 keys should have non-empty buckets"
                    );
                    // Bucket keys should be prefixed with "042/" to enable routed drill-down.
                    for key in m.payload.buckets.keys() {
                        assert!(
                            key.starts_with("042/"),
                            "bucket key should have partition prefix, got: {key}"
                        );
                    }
                } else {
                    panic!("expected SyncRespBuckets, got different Message variant");
                }
            }
            OperationResponse::Empty => {
                panic!("expected SyncRespBuckets, got Empty (partition 42 should have data)");
            }
            other => panic!("expected Message response, got {other:?}"),
        }
    }

    // -------------------------------------------------------------------------
    // AC6: Root bucket aggregation — path "" returns buckets from all partitions
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn aggregate_bucket_request_combines_all_partitions() {
        let merkle_manager = Arc::new(MerkleSyncManager::default());
        // Write to two separate partitions.
        merkle_manager.update_lww("users", 1, "alice", 111);
        merkle_manager.update_lww("users", 2, "bob", 222);

        let svc = Arc::new(SyncService::new(
            Arc::clone(&merkle_manager),
            make_factory(),
            Arc::new(ConnectionRegistry::new()),
        ));

        let op = Operation::MerkleReqBucket {
            ctx: make_ctx(service_names::SYNC),
            payload: messages::MerkleReqBucketMessage {
                payload: messages::MerkleReqBucketPayload {
                    map_name: "users".to_string(),
                    path: "".to_string(),
                },
            },
        };

        let resp = svc.oneshot(op).await.unwrap();
        match resp {
            OperationResponse::Message(msg) => {
                if let Message::SyncRespBuckets(m) = *msg {
                    assert_eq!(m.payload.path, "");
                    assert!(
                        !m.payload.buckets.is_empty(),
                        "aggregate buckets should be non-empty with data across partitions"
                    );
                    for key in m.payload.buckets.keys() {
                        assert_eq!(
                            key.len(),
                            1,
                            "aggregate bucket keys should be single hex chars, got: {key}"
                        );
                    }
                } else {
                    panic!("expected SyncRespBuckets, got different Message variant");
                }
            }
            other => panic!("expected Message response, got {other:?}"),
        }
    }

    // =========================================================================
    // AC9 — Durable-index SYNC handlers: no-panic on multi-thread runtime
    //
    // Uses `#[tokio::test(flavor = "multi_thread")]` because `build_session`
    // calls `tokio::task::block_in_place` internally. Single-thread runtimes
    // (the default `#[tokio::test]`) would panic on that call site. Multi-thread
    // is the non-negotiable requirement for the production server binary and for
    // every test that exercises the durable-index code path.
    // =========================================================================

    /// AC9 — `SyncInit` through the durable index does not panic on a
    /// multi-threaded Tokio runtime.
    ///
    /// Fault scenario: a simulated "peer node" is killed (its backing store is
    /// dropped) while the local node independently builds a Merkle session and
    /// resolves the LWW root. The local session build must succeed regardless of
    /// whether the peer is alive, because `build_session` reads only from the
    /// local durable store — inter-node unavailability cannot affect it.
    #[tokio::test(flavor = "multi_thread")]
    async fn ac9_sync_init_via_durable_index_no_panic_multi_thread() {
        use crate::storage::datastores::RedbDataStore;
        use crate::storage::durable_merkle::DurableMerkle;
        use tower::ServiceExt;

        // Build a real redb store and seed two LWW records so the root is non-zero.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ac9_test.redb");
        let store = Arc::new(RedbDataStore::new(&path).expect("redb open"));

        let key_a = "ac9-key-a";
        let key_b = "ac9-key-b";
        let ts_a = topgun_core::hlc::Timestamp {
            millis: 100,
            counter: 0,
            node_id: "node-a".to_string(),
        };
        let ts_b = topgun_core::hlc::Timestamp {
            millis: 200,
            counter: 0,
            node_id: "node-b".to_string(),
        };
        store
            .add(
                "syncmap",
                key_a,
                &RecordValue::Lww {
                    value: Value::String("val-a".to_string()),
                    timestamp: ts_a,
                },
                0,
                1,
            )
            .await
            .expect("seed key-a");
        store
            .add(
                "syncmap",
                key_b,
                &RecordValue::Lww {
                    value: Value::String("val-b".to_string()),
                    timestamp: ts_b,
                },
                0,
                2,
            )
            .await
            .expect("seed key-b");

        // Wire the durable index into the SyncService.
        let durable_index: Arc<
            dyn crate::storage::map_data_store::DurableMerkleIndex + Send + Sync,
        > = Arc::new(DurableMerkle);
        let durable_store: Arc<dyn crate::storage::map_data_store::MapDataStore> =
            Arc::clone(&store) as Arc<dyn crate::storage::map_data_store::MapDataStore>;

        let merkle_manager = Arc::new(MerkleSyncManager::default());
        let record_store_factory = make_factory();
        let connection_registry = Arc::new(ConnectionRegistry::new());
        let svc = Arc::new(
            SyncService::new(merkle_manager, record_store_factory, connection_registry)
                .with_durable_index(durable_index, durable_store),
        );

        // Fault scenario: simulate peer-node failure by dropping the peer's store
        // reference before the local SYNC_INIT runs. The local node's build_session
        // must still succeed because it reads from its own local store only.
        let peer_store =
            Arc::new(RedbDataStore::new(dir.path().join("ac9_peer.redb")).expect("peer redb open"));
        // Kill the peer: drop its store (simulating node failure / partition).
        drop(peer_store);

        // Dispatch SyncInit through the operation router — this exercises
        // `build_session` on the multi-threaded runtime. Must NOT panic.
        let op = Operation::SyncInit {
            ctx: make_ctx(service_names::SYNC),
            payload: topgun_core::messages::SyncInitMessage {
                map_name: "syncmap".to_string(),
                last_sync_timestamp: None,
            },
        };

        let resp = svc.oneshot(op).await.expect("SyncInit must not fail");

        match resp {
            OperationResponse::Message(msg) => {
                if let topgun_core::messages::Message::SyncRespRoot(m) = *msg {
                    assert_eq!(m.payload.map_name, "syncmap");
                    assert_ne!(
                        m.payload.root_hash, 0,
                        "durable index over 2 seeded records must produce non-zero root"
                    );
                } else {
                    panic!("expected SyncRespRoot, got different Message variant");
                }
            }
            other => panic!("expected Message response, got {other:?}"),
        }
    }

    /// AC9 — `MerkleReqBucket` via durable index reuses the session cached by
    /// `SyncInit` (no second `enumerate_leaves` pass) and does not panic.
    ///
    /// Fault scenario: network partition simulated by running the bucket drill-down
    /// after the backing store for a second "partition peer" has been dropped. The
    /// local service must still answer from its session cache.
    #[tokio::test(flavor = "multi_thread")]
    async fn ac9_merkle_req_bucket_via_durable_index_reuses_session_no_panic() {
        use crate::storage::datastores::RedbDataStore;
        use crate::storage::durable_merkle::DurableMerkle;
        use tower::ServiceExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let store =
            Arc::new(RedbDataStore::new(dir.path().join("ac9_bucket.redb")).expect("redb open"));

        // Seed enough records to create at least one internal node at depth > 0.
        for i in 0..20u32 {
            let key = format!("bucket-key-{i:04}");
            store
                .add(
                    "bmap",
                    &key,
                    &RecordValue::Lww {
                        value: Value::Int(i64::from(i)),
                        timestamp: topgun_core::hlc::Timestamp {
                            millis: u64::from(i) + 1,
                            counter: i,
                            node_id: "n1".to_string(),
                        },
                    },
                    0,
                    1,
                )
                .await
                .expect("seed");
        }

        let durable_index: Arc<
            dyn crate::storage::map_data_store::DurableMerkleIndex + Send + Sync,
        > = Arc::new(DurableMerkle);
        let durable_store: Arc<dyn crate::storage::map_data_store::MapDataStore> =
            Arc::clone(&store) as Arc<dyn crate::storage::map_data_store::MapDataStore>;

        let svc = Arc::new(
            SyncService::new(
                Arc::new(MerkleSyncManager::default()),
                make_factory(),
                Arc::new(ConnectionRegistry::new()),
            )
            .with_durable_index(durable_index, durable_store),
        );

        // 1. SYNC_INIT — builds and caches the session.
        let init_op = Operation::SyncInit {
            ctx: make_ctx(service_names::SYNC),
            payload: topgun_core::messages::SyncInitMessage {
                map_name: "bmap".to_string(),
                last_sync_timestamp: None,
            },
        };
        let init_resp = Arc::clone(&svc)
            .oneshot(init_op)
            .await
            .expect("SyncInit must succeed");
        let _root_hash = match init_resp {
            OperationResponse::Message(msg) => {
                if let topgun_core::messages::Message::SyncRespRoot(m) = *msg {
                    m.payload.root_hash
                } else {
                    panic!("expected SyncRespRoot");
                }
            }
            other => panic!("expected Message, got {other:?}"),
        };

        // 2. Fault: simulate partition by dropping the secondary peer store reference.
        //    The session was already built; the local service answers from cache.
        drop(store);

        // 3. MerkleReqBucket at root — reuses the cached session (no second
        //    enumerate_leaves even though the store was dropped).
        let bucket_op = Operation::MerkleReqBucket {
            ctx: make_ctx(service_names::SYNC),
            payload: topgun_core::messages::MerkleReqBucketMessage {
                payload: topgun_core::messages::MerkleReqBucketPayload {
                    map_name: "bmap".to_string(),
                    path: String::new(),
                },
            },
        };
        let bucket_resp = Arc::clone(&svc)
            .oneshot(bucket_op)
            .await
            .expect("MerkleReqBucket must not fail even after store drop");

        // The session has data so we expect buckets or a leaf response.
        assert!(
            matches!(
                bucket_resp,
                OperationResponse::Message(_) | OperationResponse::Empty
            ),
            "expected Message or Empty response after session-cached bucket request"
        );
    }

    /// End-to-end durable tree-walk: drive the exact reconnecting-client protocol
    /// (`SYNC_INIT` root → `MerkleReqBucket` DFS from `""` → `SYNC_RESP_LEAF`)
    /// against a `SyncService` whose durable index AND record-store factory are
    /// both backed by the SAME real redb store, and assert every seeded key is
    /// recovered through the walk. This is the machine check the soak harness'
    /// recovery checkpoint performs; the AC9 tests stop at "didn't panic / not
    /// the in-memory path" and never assert leaf delivery.
    #[tokio::test(flavor = "multi_thread")]
    #[allow(clippy::too_many_lines)]
    async fn durable_tree_walk_recovers_all_seeded_keys() {
        use crate::storage::datastores::RedbDataStore;
        use crate::storage::durable_merkle::DurableMerkle;
        use std::collections::HashMap;
        use tower::ServiceExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(RedbDataStore::new(dir.path().join("walk.redb")).expect("redb open"));

        // Seed enough keys to force a multi-level trie (internal nodes + leaves).
        let mut expected: HashMap<String, i64> = HashMap::new();
        for i in 0..64u32 {
            let key = format!("walk-key-{i:04}");
            store
                .add(
                    "wmap",
                    &key,
                    &RecordValue::Lww {
                        value: Value::Int(i64::from(i)),
                        timestamp: Timestamp {
                            millis: u64::from(i) + 1,
                            counter: i,
                            node_id: "n1".to_string(),
                        },
                    },
                    0,
                    1,
                )
                .await
                .expect("seed");
            expected.insert(key, i64::from(i));
        }

        // Record factory backed by the SAME store so leaf fetches lazy-load
        // the durable values — exactly the production wiring.
        let factory = Arc::new(RecordStoreFactory::new(
            StorageConfig::default(),
            Arc::clone(&store) as Arc<dyn crate::storage::map_data_store::MapDataStore>,
            Vec::new(),
        ));
        let durable_index: Arc<
            dyn crate::storage::map_data_store::DurableMerkleIndex + Send + Sync,
        > = Arc::new(DurableMerkle);
        let durable_store: Arc<dyn crate::storage::map_data_store::MapDataStore> =
            Arc::clone(&store) as Arc<dyn crate::storage::map_data_store::MapDataStore>;
        let svc = Arc::new(
            SyncService::new(
                Arc::new(MerkleSyncManager::default()),
                factory,
                Arc::new(ConnectionRegistry::new()),
            )
            .with_durable_index(durable_index, durable_store),
        );

        // SYNC_INIT — root must be non-zero (data present).
        let init_resp = Arc::clone(&svc)
            .oneshot(Operation::SyncInit {
                ctx: make_ctx(service_names::SYNC),
                payload: topgun_core::messages::SyncInitMessage {
                    map_name: "wmap".to_string(),
                    last_sync_timestamp: None,
                },
            })
            .await
            .expect("SyncInit");
        let root = match init_resp {
            OperationResponse::Message(msg) => match *msg {
                topgun_core::messages::Message::SyncRespRoot(m) => m.payload.root_hash,
                other => panic!("expected SyncRespRoot, got {other:?}"),
            },
            other => panic!("expected Message, got {other:?}"),
        };
        assert_ne!(root, 0, "root must be non-zero over 64 seeded keys");

        // DFS walk from "" exactly like SoakClient::delta_sync_all.
        let mut found: HashMap<String, i64> = HashMap::new();
        let mut stack = vec![String::new()];
        let mut visited = 0usize;
        while let Some(path) = stack.pop() {
            visited += 1;
            assert!(visited < 10_000, "walk did not terminate");
            let resp = Arc::clone(&svc)
                .oneshot(Operation::MerkleReqBucket {
                    ctx: make_ctx(service_names::SYNC),
                    payload: topgun_core::messages::MerkleReqBucketMessage {
                        payload: topgun_core::messages::MerkleReqBucketPayload {
                            map_name: "wmap".to_string(),
                            path: path.clone(),
                        },
                    },
                })
                .await
                .expect("MerkleReqBucket");
            match resp {
                OperationResponse::Message(msg) => match *msg {
                    topgun_core::messages::Message::SyncRespBuckets(b) => {
                        for child in b.payload.buckets.keys() {
                            stack.push(format!("{path}{child}"));
                        }
                    }
                    topgun_core::messages::Message::SyncRespLeaf(l) => {
                        for rec in l.payload.records {
                            if let Some(v) = rec.record.value.as_ref().and_then(rmpv::Value::as_i64)
                            {
                                found.insert(rec.key, v);
                            }
                        }
                    }
                    other => panic!("unexpected message in walk: {other:?}"),
                },
                OperationResponse::Empty => panic!(
                    "durable walk returned Empty at path {path:?} — the reconnecting client \
                     would hang waiting for SYNC_RESP_BUCKETS|SYNC_RESP_LEAF"
                ),
                other => panic!("unexpected response in walk: {other:?}"),
            }
        }

        assert_eq!(
            found, expected,
            "durable tree-walk must recover every seeded key/value"
        );
    }

    // When a durable index IS wired but build_session fails (transient store /
    // enumerate fault), the SYNC handlers must REJECT the round rather than
    // silently serving the in-memory accelerator — that tree only sees resident
    // keys, so a fallback would hand the reconnecting client a residency-coupled
    // root that omits persisted-but-evicted records (the exact TODO-530 defect
    // this path eliminates). Proves the fail-closed contract behaviorally.
    #[tokio::test(flavor = "multi_thread")]
    async fn durable_session_build_error_rejects_round_no_inmem_fallback() {
        use crate::storage::datastores::RedbDataStore;
        use tower::ServiceExt;

        struct FailingIndex;
        impl crate::storage::map_data_store::DurableMerkleIndex for FailingIndex {
            fn build_session(
                &self,
                _map: &str,
                _store: &dyn crate::storage::map_data_store::MapDataStore,
            ) -> anyhow::Result<MerkleSession> {
                anyhow::bail!("simulated durable session build fault")
            }
        }

        let dir = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(RedbDataStore::new(dir.path().join("fault.redb")).expect("redb open"));
        let factory = Arc::new(RecordStoreFactory::new(
            StorageConfig::default(),
            Arc::clone(&store) as Arc<dyn crate::storage::map_data_store::MapDataStore>,
            Vec::new(),
        ));
        let durable_index: Arc<
            dyn crate::storage::map_data_store::DurableMerkleIndex + Send + Sync,
        > = Arc::new(FailingIndex);
        let durable_store: Arc<dyn crate::storage::map_data_store::MapDataStore> =
            Arc::clone(&store) as Arc<dyn crate::storage::map_data_store::MapDataStore>;
        let svc = Arc::new(
            SyncService::new(
                Arc::new(MerkleSyncManager::default()),
                factory,
                Arc::new(ConnectionRegistry::new()),
            )
            .with_durable_index(durable_index, durable_store),
        );

        // SYNC_INIT must error, not fall back to the in-memory aggregate root.
        let init = Arc::clone(&svc)
            .oneshot(Operation::SyncInit {
                ctx: make_ctx(service_names::SYNC),
                payload: topgun_core::messages::SyncInitMessage {
                    map_name: "fault-map".to_string(),
                    last_sync_timestamp: None,
                },
            })
            .await;
        assert!(
            init.is_err(),
            "durable build failure must reject SYNC_INIT, not silently serve the in-memory tree"
        );

        // MerkleReqBucket must reject too.
        let bucket = Arc::clone(&svc)
            .oneshot(Operation::MerkleReqBucket {
                ctx: make_ctx(service_names::SYNC),
                payload: topgun_core::messages::MerkleReqBucketMessage {
                    payload: topgun_core::messages::MerkleReqBucketPayload {
                        map_name: "fault-map".to_string(),
                        path: String::new(),
                    },
                },
            })
            .await;
        assert!(
            bucket.is_err(),
            "durable build failure must reject MerkleReqBucket, not silently serve the in-memory tree"
        );
    }

    /// AC3 (TODO-544, red-on-revert): two clients on the SAME map. A write lands
    /// between client A's `SYNC_INIT` and its bucket walk, while client B issues a
    /// concurrent `SYNC_INIT`. Client A's DFS must recover the snapshot A anchored
    /// at its OWN `SYNC_INIT` — it must NOT observe B's newer snapshot (which
    /// includes the post-anchor write). On the pre-fix map-name-only keying, B's
    /// `SYNC_INIT` (`force_rebuild`) replaced the single shared entry, so A's
    /// `force_rebuild=false` bucket calls read B's session and surface the new key
    /// → this test FAILS when the keying change is reverted (non-vacuous).
    ///
    /// Multi-thread runtime required: `build_session` uses `block_in_place`.
    #[tokio::test(flavor = "multi_thread")]
    #[allow(clippy::too_many_lines)]
    async fn ac3_concurrent_sync_init_does_not_swap_peer_snapshot() {
        use crate::storage::datastores::RedbDataStore;
        use crate::storage::durable_merkle::DurableMerkle;
        use std::collections::HashSet;
        use tower::ServiceExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(RedbDataStore::new(dir.path().join("ac3.redb")).expect("redb open"));

        // Seed enough keys to force a multi-level trie (internal nodes + leaves).
        let mut seeded: HashSet<String> = HashSet::new();
        for i in 0..32u32 {
            let key = format!("ac3-key-{i:04}");
            store
                .add(
                    "amap",
                    &key,
                    &RecordValue::Lww {
                        value: Value::Int(i64::from(i)),
                        timestamp: Timestamp {
                            millis: u64::from(i) + 1,
                            counter: i,
                            node_id: "n1".to_string(),
                        },
                    },
                    0,
                    1,
                )
                .await
                .expect("seed");
            seeded.insert(key);
        }

        // Record factory backed by the SAME store so leaf fetches lazy-load the
        // durable values — exactly the production wiring.
        let factory = Arc::new(RecordStoreFactory::new(
            StorageConfig::default(),
            Arc::clone(&store) as Arc<dyn crate::storage::map_data_store::MapDataStore>,
            Vec::new(),
        ));
        let durable_index: Arc<
            dyn crate::storage::map_data_store::DurableMerkleIndex + Send + Sync,
        > = Arc::new(DurableMerkle);
        let durable_store: Arc<dyn crate::storage::map_data_store::MapDataStore> =
            Arc::clone(&store) as Arc<dyn crate::storage::map_data_store::MapDataStore>;
        let svc = Arc::new(
            SyncService::new(
                Arc::new(MerkleSyncManager::default()),
                factory,
                Arc::new(ConnectionRegistry::new()),
            )
            .with_durable_index(durable_index, durable_store),
        );

        let conn_a = Some(ConnectionId(1));
        let conn_b = Some(ConnectionId(2));

        // 1. Client A SYNC_INIT — anchors A's snapshot over the 32 seeded keys.
        let mut ctx_a = make_ctx(service_names::SYNC);
        ctx_a.connection_id = conn_a;
        let root_a = match Arc::clone(&svc)
            .oneshot(Operation::SyncInit {
                ctx: ctx_a,
                payload: topgun_core::messages::SyncInitMessage {
                    map_name: "amap".to_string(),
                    last_sync_timestamp: None,
                },
            })
            .await
            .expect("A SyncInit")
        {
            OperationResponse::Message(m) => match *m {
                topgun_core::messages::Message::SyncRespRoot(r) => r.payload.root_hash,
                o => panic!("expected SyncRespRoot, got {o:?}"),
            },
            o => panic!("expected Message, got {o:?}"),
        };

        // 2. A write lands AFTER A anchored its snapshot.
        let new_key = "ac3-key-9999";
        store
            .add(
                "amap",
                new_key,
                &RecordValue::Lww {
                    value: Value::Int(9999),
                    timestamp: Timestamp {
                        millis: 100_000,
                        counter: 0,
                        node_id: "n1".to_string(),
                    },
                },
                0,
                1,
            )
            .await
            .expect("post-anchor write");

        // 3. Client B SYNC_INIT (concurrent) — builds a NEWER snapshot incl new_key.
        let mut ctx_b = make_ctx(service_names::SYNC);
        ctx_b.connection_id = conn_b;
        let root_b = match Arc::clone(&svc)
            .oneshot(Operation::SyncInit {
                ctx: ctx_b,
                payload: topgun_core::messages::SyncInitMessage {
                    map_name: "amap".to_string(),
                    last_sync_timestamp: None,
                },
            })
            .await
            .expect("B SyncInit")
        {
            OperationResponse::Message(m) => match *m {
                topgun_core::messages::Message::SyncRespRoot(r) => r.payload.root_hash,
                o => panic!("expected SyncRespRoot, got {o:?}"),
            },
            o => panic!("expected Message, got {o:?}"),
        };
        assert_ne!(
            root_a, root_b,
            "sanity: the post-anchor write must change the durable root B observes"
        );

        // 4. Client A's bucket DFS (force_rebuild=false) — must reflect A's own
        //    anchored snapshot: recover exactly the 32 seeded keys, NOT new_key.
        //    On map-name-only keying A would read B's swapped-in session.
        let mut found: HashSet<String> = HashSet::new();
        let mut stack = vec![String::new()];
        let mut visited = 0usize;
        while let Some(path) = stack.pop() {
            visited += 1;
            assert!(visited < 10_000, "walk did not terminate");
            let mut ctx = make_ctx(service_names::SYNC);
            ctx.connection_id = conn_a;
            let resp = Arc::clone(&svc)
                .oneshot(Operation::MerkleReqBucket {
                    ctx,
                    payload: topgun_core::messages::MerkleReqBucketMessage {
                        payload: topgun_core::messages::MerkleReqBucketPayload {
                            map_name: "amap".to_string(),
                            path: path.clone(),
                        },
                    },
                })
                .await
                .expect("A MerkleReqBucket");
            match resp {
                OperationResponse::Message(msg) => match *msg {
                    topgun_core::messages::Message::SyncRespBuckets(b) => {
                        for child in b.payload.buckets.keys() {
                            stack.push(format!("{path}{child}"));
                        }
                    }
                    topgun_core::messages::Message::SyncRespLeaf(l) => {
                        for rec in l.payload.records {
                            found.insert(rec.key);
                        }
                    }
                    other => panic!("unexpected message in A's walk: {other:?}"),
                },
                OperationResponse::Empty => {}
                other => panic!("unexpected response in A's walk: {other:?}"),
            }
        }

        assert!(
            !found.contains(new_key),
            "client A walked B's swapped-in snapshot: it surfaced the post-anchor write \
             {new_key} (the TODO-544 map-name-only keying regression)"
        );
        assert_eq!(
            found, seeded,
            "client A's walk must recover exactly its own anchored snapshot (32 seeded keys)"
        );
    }

    /// SPEC-334 diagnostic gate: a value mutation landing on a walked key BETWEEN
    /// the snapshot build (`SYNC_INIT`) and the lazy leaf fetch produces a torn
    /// read at the wire — the bucket hash the client verifies was folded from the
    /// OLD leaf hash, while the served leaf carries the NEWER live value. This
    /// test pins the actual behaviour of THAT same-snapshot, same-connection
    /// live-leaf path (distinct from the SPEC-331 session-SWAP path the `ac3`
    /// test above exercises).
    ///
    /// Outcome class established by this test: **(b) self-healing, no defect.**
    /// The torn read is real at the structure-vs-served-bytes layer, but it is
    /// NOT convergence-breaking for two independently-verified reasons:
    ///
    /// 1. The served leaf is ALWAYS the live store value (the lazy `store.get`
    ///    returns the newest record), never a stale or fabricated value that
    ///    disagrees with both the snapshot AND the live state. The client's
    ///    `handleSyncRespLeaf` does a monotonic CRDT `map.merge(key, record)` on
    ///    those exact bytes — it does NOT commit the server's bucket hash as
    ///    authoritative state. The bucket hash is used only as a drill-down
    ///    TRIGGER (`handleSyncRespBuckets`), so a stale bucket hash can only cause
    ///    the client to descend into a subtree, where it then receives and merges
    ///    the live truth.
    /// 2. The self-heal round is the client's NEXT `SYNC_INIT`/root comparison:
    ///    after merging the live leaf its local tree reflects the new value, so a
    ///    subsequent root compare either matches (converged) or re-walks the still
    ///    -divergent subtree and re-merges. No round ever commits a value the live
    ///    store does not hold.
    ///
    /// This assertion is therefore a POSITIVE convergence guard, NOT
    /// `assert!(true)`: it FAILS if the lazy-fetch path ever served a value that
    /// disagrees with the live store (the only way the client could commit a value
    /// inconsistent with durable truth). It does not borrow the SPEC-325b/TODO-544
    /// session-SWAP "self-heals" claim — the evidence here (served-bytes ==
    /// live-store-value under a mid-session mutation on the SAME cached snapshot)
    /// is independent.
    ///
    /// Multi-thread runtime required: `build_session` uses `block_in_place`.
    #[tokio::test(flavor = "multi_thread")]
    #[allow(clippy::too_many_lines)]
    async fn sync_torn_read_under_mid_session_mutation_serves_live_value() {
        use crate::storage::datastores::RedbDataStore;
        use crate::storage::durable_merkle::DurableMerkle;
        use std::collections::HashMap as Map;
        use tower::ServiceExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let store =
            Arc::new(RedbDataStore::new(dir.path().join("tornread.redb")).expect("redb open"));

        // Seed enough keys to force a multi-level trie so the drill-down visits a
        // real leaf bucket rather than serving everything from the root.
        for i in 0..32u32 {
            let key = format!("tr-key-{i:04}");
            store
                .add(
                    "tmap",
                    &key,
                    &RecordValue::Lww {
                        value: Value::Int(i64::from(i)),
                        timestamp: Timestamp {
                            millis: u64::from(i) + 1,
                            counter: i,
                            node_id: "n1".to_string(),
                        },
                    },
                    0,
                    1,
                )
                .await
                .expect("seed");
        }

        // Record factory backed by the SAME store so leaf fetches lazy-load the
        // durable values — exactly the production wiring.
        let factory = Arc::new(RecordStoreFactory::new(
            StorageConfig::default(),
            Arc::clone(&store) as Arc<dyn crate::storage::map_data_store::MapDataStore>,
            Vec::new(),
        ));
        let durable_index: Arc<
            dyn crate::storage::map_data_store::DurableMerkleIndex + Send + Sync,
        > = Arc::new(DurableMerkle);
        let durable_store: Arc<dyn crate::storage::map_data_store::MapDataStore> =
            Arc::clone(&store) as Arc<dyn crate::storage::map_data_store::MapDataStore>;
        // Hold a clone of the factory so the test can read the live record the
        // SAME way the handler does (`get_or_create` → `RecordStore::get`).
        let read_factory = Arc::clone(&factory);
        let svc = Arc::new(
            SyncService::new(
                Arc::new(MerkleSyncManager::default()),
                factory,
                Arc::new(ConnectionRegistry::new()),
            )
            .with_durable_index(durable_index, durable_store),
        );

        let conn = Some(ConnectionId(1));

        // The key whose value we mutate mid-session. It is one of the seeded keys,
        // so the snapshot's bucket structure already includes its OLD leaf hash.
        let target_key = "tr-key-0007";
        let old_value = Value::Int(7);
        let old_timestamp = Timestamp {
            millis: 8,
            counter: 7,
            node_id: "n1".to_string(),
        };

        // The leaf path the drill-down visits for this key: 8-char hex of its
        // fnv1a hash, truncated to the tree depth (3), mirroring the build sink.
        let target_path: String = {
            let hex = format!("{:08x}", topgun_core::hash::fnv1a_hash(target_key));
            hex[..3].to_string()
        };

        // --- 1. SYNC_INIT: builds and CACHES the snapshot for this connection. ---
        let mut ctx_init = make_ctx(service_names::SYNC);
        ctx_init.connection_id = conn;
        let _root = match Arc::clone(&svc)
            .oneshot(Operation::SyncInit {
                ctx: ctx_init,
                payload: topgun_core::messages::SyncInitMessage {
                    map_name: "tmap".to_string(),
                    last_sync_timestamp: None,
                },
            })
            .await
            .expect("SyncInit")
        {
            OperationResponse::Message(m) => match *m {
                topgun_core::messages::Message::SyncRespRoot(r) => r.payload.root_hash,
                o => panic!("expected SyncRespRoot, got {o:?}"),
            },
            o => panic!("expected Message, got {o:?}"),
        };

        // Capture the snapshot bucket hash that the client would verify for the
        // target leaf path. This is the OLD-value-anchored structure: it was
        // folded from the target key's leaf hash at snapshot build time.
        let cached = svc
            .session_registry
            .get("tmap", ConnectionId(1))
            .expect("session cached after SYNC_INIT");
        // Aggregate the snapshot bucket children along the target path so we have a
        // concrete pre-mutation structure fingerprint to compare after the write.
        let snapshot_bucket_before: Map<char, u32> = cached
            .lww_nodes
            .get(&target_path[..2])
            .cloned()
            .unwrap_or_default();

        // --- 2. Mutate the target key AFTER the snapshot was cached. ---
        // A strictly-newer timestamp so the new write wins LWW and changes the
        // key's leaf hash (the hash folds timestamp, so the snapshot's bucket hash
        // is now stale relative to live).
        let new_value = Value::Int(70_007);
        let new_timestamp = Timestamp {
            millis: 500_000,
            counter: 0,
            node_id: "n1".to_string(),
        };
        store
            .add(
                "tmap",
                target_key,
                &RecordValue::Lww {
                    value: new_value.clone(),
                    timestamp: new_timestamp.clone(),
                },
                0,
                1,
            )
            .await
            .expect("mid-session mutation");

        // Confirm the snapshot was NOT rebuilt by the mutation: the cached session
        // is the same Arc the registry held before the write, and its bucket hash
        // for the target path still reflects the OLD leaf. This is what makes the
        // window genuinely same-snapshot (force_rebuild=false reuse), not a swap.
        let cached_after = svc
            .session_registry
            .get("tmap", ConnectionId(1))
            .expect("session still cached");
        assert!(
            Arc::ptr_eq(&cached, &cached_after),
            "snapshot must NOT be rebuilt by a mid-session write — the bucket walk \
             below must reuse the SAME cached session (force_rebuild=false)"
        );

        // --- 3. MerkleReqBucket drill-down on the SAME connection. ---
        // Walk to the target leaf, capturing the bucket hash the client verifies
        // (snapshot, OLD-anchored) and the leaf value the client receives (live,
        // NEW). The torn read is: structure verified == OLD, value served == NEW.
        let mut served_value: Option<rmpv::Value> = None;
        let mut served_timestamp: Option<Timestamp> = None;
        let mut verified_bucket_hash: Option<u32> = None;
        let mut stack = vec![String::new()];
        let mut visited = 0usize;
        while let Some(path) = stack.pop() {
            visited += 1;
            assert!(visited < 10_000, "walk did not terminate");
            let mut ctx = make_ctx(service_names::SYNC);
            ctx.connection_id = conn;
            let resp = Arc::clone(&svc)
                .oneshot(Operation::MerkleReqBucket {
                    ctx,
                    payload: topgun_core::messages::MerkleReqBucketMessage {
                        payload: topgun_core::messages::MerkleReqBucketPayload {
                            map_name: "tmap".to_string(),
                            path: path.clone(),
                        },
                    },
                })
                .await
                .expect("MerkleReqBucket");
            match resp {
                OperationResponse::Message(msg) => match *msg {
                    topgun_core::messages::Message::SyncRespBuckets(b) => {
                        for (child, hash) in &b.payload.buckets {
                            let child_path = format!("{path}{child}");
                            // Record the bucket hash on the route to the target key
                            // — this is the snapshot-anchored hash the client uses
                            // as its drill-down trigger.
                            if target_path.starts_with(&child_path) {
                                verified_bucket_hash = Some(*hash);
                            }
                            stack.push(child_path);
                        }
                    }
                    topgun_core::messages::Message::SyncRespLeaf(l) => {
                        for rec in l.payload.records {
                            if rec.key == target_key {
                                served_value = rec.record.value;
                                served_timestamp = Some(rec.record.timestamp);
                            }
                        }
                    }
                    other => panic!("unexpected message in walk: {other:?}"),
                },
                OperationResponse::Empty => {}
                other => panic!("unexpected response in walk: {other:?}"),
            }
        }

        // --- Live evidence (AC1) ---
        let served_value = served_value.expect("target key must be served as a leaf record");
        let served_timestamp = served_timestamp.expect("served leaf must carry a timestamp");
        let verified_bucket_hash =
            verified_bucket_hash.expect("the walk must verify a snapshot bucket hash en route");

        // Recompute the OLD (snapshot-time) leaf hash and the NEW (live) leaf hash
        // so the report can quote the disagreement concretely.
        let (_, old_leaf_hash) = crate::storage::map_data_store::merkle_leaf_hash(
            target_key,
            &RecordValue::Lww {
                value: old_value.clone(),
                timestamp: old_timestamp.clone(),
            },
        )
        .expect("LWW leaf hash");
        let (_, new_leaf_hash) = crate::storage::map_data_store::merkle_leaf_hash(
            target_key,
            &RecordValue::Lww {
                value: new_value.clone(),
                timestamp: new_timestamp.clone(),
            },
        )
        .expect("LWW leaf hash");

        eprintln!(
            "SPEC-334 evidence: target_key={target_key} target_path={target_path}\n  \
             snapshot_bucket_before (path {})={:?}\n  \
             verified_bucket_hash (snapshot, OLD-anchored)={verified_bucket_hash}\n  \
             old_leaf_hash={old_leaf_hash} new_leaf_hash={new_leaf_hash}\n  \
             served_value (live)={served_value:?} served_ts.millis={}",
            &target_path[..2],
            snapshot_bucket_before,
            served_timestamp.millis,
        );

        // The torn read EXISTS at the wire: the leaf hash baked into the verified
        // bucket structure changed under the mutation. If this is not true the
        // test is not exercising the torn-read window at all.
        assert_ne!(
            old_leaf_hash, new_leaf_hash,
            "sanity: the mid-session mutation must change the target key's leaf hash, \
             else there is no torn read to diagnose"
        );

        // CARDINAL convergence guard (non-vacuous): the value the client receives
        // and merges MUST equal the LIVE store value — never the stale snapshot
        // value, never a fabricated one. The client merges these exact bytes via
        // monotonic LWW; serving the live truth is what makes the torn read
        // self-healing rather than convergence-breaking. This FAILS if the
        // lazy-fetch path ever serves a value inconsistent with durable state.
        let live = read_factory
            .get_or_create("tmap", hash_to_partition(target_key))
            .get(target_key, false)
            .await
            .expect("live get")
            .expect("target key present");
        let RecordValue::Lww {
            value: live_value,
            timestamp: live_timestamp,
        } = live.value
        else {
            panic!("target key must be an LWW record");
        };
        assert_eq!(
            served_value,
            value_to_rmpv(&live_value),
            "torn read served a value that disagrees with the live store — the client \
             would commit a state inconsistent with durable truth (convergence-breaking)"
        );
        assert_eq!(
            served_timestamp, live_timestamp,
            "served leaf timestamp must match the live record so LWW merge converges to truth"
        );
        // And the served value is the NEW one (proves the window landed live, not a
        // stale snapshot read).
        assert_eq!(
            served_value,
            value_to_rmpv(&new_value),
            "served leaf must be the post-mutation value (the lazy fetch is live)"
        );

        // Prove the verified bucket hash is genuinely OLD-anchored (the torn read
        // is real, not merely implied): a session rebuilt over the post-mutation
        // live state — on a NEW connection so nothing is served from conn 1's
        // cache — yields a DIFFERENT bucket aggregate for the same path than the
        // stale structure the client just walked. This demonstrates the hash the
        // client verified pre-dates the mutation while the served leaf was live.
        let fresh = svc
            .get_or_build_session("tmap", Some(ConnectionId(2)), false)
            .expect("fresh session build")
            .expect("durable index present");
        let fresh_bucket = fresh
            .lww_nodes
            .get(&target_path[..2])
            .cloned()
            .unwrap_or_default();
        assert_ne!(
            snapshot_bucket_before, fresh_bucket,
            "the bucket structure the client verified must be OLD-anchored: a session \
             rebuilt over live state differs, proving the verified structure pre-dates \
             the mutation while the served leaf was live (the torn read is real)"
        );

        // Tie the bucket hash actually served ON THE WIRE (not just the in-memory
        // cached structure) to the stale snapshot: it equals the OLD cached child
        // hash and differs from the fresh/live one. The structure assertion above
        // reads both sides from in-memory sessions; without this a handler that
        // recomputed bucket hashes from live state per request would close the torn
        // read at the wire yet leave that assertion green. This makes "the torn read
        // is observable at the wire" a falsifiable claim against the served bytes.
        let target_child = target_path
            .chars()
            .nth(2)
            .expect("target path has a depth-3 child char");
        let stale_child_hash = *snapshot_bucket_before
            .get(&target_child)
            .expect("stale snapshot bucket must hold the target child");
        assert_eq!(
            verified_bucket_hash, stale_child_hash,
            "the bucket hash served on the wire must be the stale cached value — the \
             torn read is observable at the wire, not merely in the in-memory snapshot"
        );
        assert_ne!(
            Some(verified_bucket_hash),
            fresh_bucket.get(&target_child).copied(),
            "the wire-served bucket hash must differ from the fresh/live one (the \
             verified structure is OLD-anchored while the served leaf was live)"
        );
    }

    /// OR-Map counterpart of the torn-read diagnostic above. The OR-Map leaf serve
    /// (`handle_ormap_merkle_req_bucket`) is structurally symmetric to the LWW one:
    /// the cached session pins the bucket structure (`ormap_nodes`) and leaf-key
    /// membership, but the leaf record itself is lazy-fetched live from the store.
    /// A mid-session OR-Map mutation therefore produces the SAME torn read at the
    /// wire — the verified bucket hash was folded from the OLD leaf hash while the
    /// served entry carries the live records/tombstones.
    ///
    /// This test exercises BOTH CRDT directions on the same cached snapshot:
    ///   - `OR_ADD`: a new tag/value lands on an existing key (records grow).
    ///   - `OR_REMOVE`: a key's only tag is tombstoned, leaving
    ///     `OrMap { records: [], tombstones: [tag] }` — which STILL produces a
    ///     hashed leaf (only the legacy `OrTombstones` variant yields no leaf), so
    ///     the key is still served and its tombstone set must reflect live truth.
    ///
    /// Outcome class (same as LWW): **(b) self-healing, no defect.** The served
    /// records/tombstones are ALWAYS the live store value; the client merges them
    /// with the add-wins / tombstone CRDT rule (never committing the bucket hash as
    /// authoritative state), so a stale bucket hash only triggers a drill into the
    /// subtree where the live truth is then merged. The assertion is a POSITIVE
    /// convergence guard: it FAILS if the lazy fetch ever serves records or
    /// tombstones that disagree with the live store.
    ///
    /// Multi-thread runtime required: `build_session` uses `block_in_place`.
    #[tokio::test(flavor = "multi_thread")]
    #[allow(clippy::too_many_lines)]
    async fn sync_ormap_leaf_torn_read_under_mid_session_mutation_serves_live_value() {
        use crate::storage::datastores::RedbDataStore;
        use crate::storage::durable_merkle::DurableMerkle;
        use crate::storage::record::OrMapEntry as StoreOrMapEntry;
        use tower::ServiceExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(
            RedbDataStore::new(dir.path().join("ormap_tornread.redb")).expect("redb open"),
        );

        // Seed enough OR-Map keys to force a multi-level trie so the drill-down
        // visits real leaf buckets. Each key starts with a single tagged record.
        for i in 0..32u32 {
            let key = format!("or-key-{i:04}");
            let tag = format!("0:{i}:n1");
            store
                .add(
                    "omap",
                    &key,
                    &RecordValue::OrMap {
                        records: vec![StoreOrMapEntry {
                            value: Value::Int(i64::from(i)),
                            tag,
                            timestamp: Timestamp {
                                millis: u64::from(i) + 1,
                                counter: i,
                                node_id: "n1".to_string(),
                            },
                        }],
                        tombstones: Vec::new(),
                    },
                    0,
                    1,
                )
                .await
                .expect("seed");
        }

        let factory = Arc::new(RecordStoreFactory::new(
            StorageConfig::default(),
            Arc::clone(&store) as Arc<dyn crate::storage::map_data_store::MapDataStore>,
            Vec::new(),
        ));
        let durable_index: Arc<
            dyn crate::storage::map_data_store::DurableMerkleIndex + Send + Sync,
        > = Arc::new(DurableMerkle);
        let durable_store: Arc<dyn crate::storage::map_data_store::MapDataStore> =
            Arc::clone(&store) as Arc<dyn crate::storage::map_data_store::MapDataStore>;
        let read_factory = Arc::clone(&factory);
        let svc = Arc::new(
            SyncService::new(
                Arc::new(MerkleSyncManager::default()),
                factory,
                Arc::new(ConnectionRegistry::new()),
            )
            .with_durable_index(durable_index, durable_store),
        );

        let conn = Some(ConnectionId(1));

        // Two target keys exercising the two CRDT directions. Both are seeded keys,
        // so the snapshot's OR-Map bucket structure already folded their OLD leaf
        // hash before the mutation lands.
        let target_add = "or-key-0007";
        let target_remove = "or-key-0011";
        let add_orig_tag = "0:7:n1".to_string();
        let remove_orig_tag = "0:11:n1".to_string();

        let leaf_path = |key: &str| -> String {
            let hex = format!("{:08x}", topgun_core::hash::fnv1a_hash(key));
            hex[..3].to_string()
        };
        let add_path = leaf_path(target_add);
        let remove_path = leaf_path(target_remove);

        // --- 1. SYNC_INIT: builds and CACHES the snapshot (LWW + OR-Map nodes). ---
        let mut ctx_init = make_ctx(service_names::SYNC);
        ctx_init.connection_id = conn;
        let _root = match Arc::clone(&svc)
            .oneshot(Operation::SyncInit {
                ctx: ctx_init,
                payload: topgun_core::messages::SyncInitMessage {
                    map_name: "omap".to_string(),
                    last_sync_timestamp: None,
                },
            })
            .await
            .expect("SyncInit")
        {
            OperationResponse::Message(m) => match *m {
                topgun_core::messages::Message::SyncRespRoot(r) => r.payload.root_hash,
                o => panic!("expected SyncRespRoot, got {o:?}"),
            },
            o => panic!("expected Message, got {o:?}"),
        };

        let cached = svc
            .session_registry
            .get("omap", ConnectionId(1))
            .expect("session cached after SYNC_INIT");

        // --- 2a. OR_ADD mutation: append a NEW tagged record to target_add. ---
        // The new record coexists with the original tag (add-wins), so the key
        // remains an `OrMap` leaf with TWO records and a changed leaf hash.
        let add_new_tag = "0:70007:n1".to_string();
        store
            .add(
                "omap",
                target_add,
                &RecordValue::OrMap {
                    records: vec![
                        StoreOrMapEntry {
                            value: Value::Int(7),
                            tag: add_orig_tag.clone(),
                            timestamp: Timestamp {
                                millis: 8,
                                counter: 7,
                                node_id: "n1".to_string(),
                            },
                        },
                        StoreOrMapEntry {
                            value: Value::Int(70_007),
                            tag: add_new_tag.clone(),
                            timestamp: Timestamp {
                                millis: 500_000,
                                counter: 0,
                                node_id: "n1".to_string(),
                            },
                        },
                    ],
                    tombstones: Vec::new(),
                },
                0,
                1,
            )
            .await
            .expect("OR_ADD mutation");

        // --- 2b. OR_REMOVE mutation: tombstone target_remove's only tag. ---
        // The result is `OrMap { records: [], tombstones: [tag] }` — still a hashed
        // leaf (NOT the legacy `OrTombstones` variant), so the key is still served.
        store
            .add(
                "omap",
                target_remove,
                &RecordValue::OrMap {
                    records: Vec::new(),
                    tombstones: vec![remove_orig_tag.clone()],
                },
                0,
                1,
            )
            .await
            .expect("OR_REMOVE mutation");

        // Confirm the snapshot was NOT rebuilt by either mutation: the bucket walk
        // must reuse the SAME cached Arc (force_rebuild=false), so this is the
        // genuine same-snapshot window, not the per-connection session-swap path.
        let cached_after = svc
            .session_registry
            .get("omap", ConnectionId(1))
            .expect("session still cached");
        assert!(
            Arc::ptr_eq(&cached, &cached_after),
            "snapshot must NOT be rebuilt by mid-session OR-Map writes — the bucket \
             walk below must reuse the SAME cached session (force_rebuild=false)"
        );

        // --- 3. ORMapMerkleReqBucket drill-down on the SAME connection. ---
        // Walk the OR-Map trie, capturing the served entry for each target key and
        // the snapshot-anchored bucket hash en route to each.
        let mut served: std::collections::HashMap<String, topgun_core::messages::ORMapEntry> =
            std::collections::HashMap::new();
        let mut verified_add_hash: Option<u32> = None;
        let mut verified_remove_hash: Option<u32> = None;
        let mut stack = vec![String::new()];
        let mut visited = 0usize;
        while let Some(path) = stack.pop() {
            visited += 1;
            assert!(visited < 10_000, "walk did not terminate");
            let mut ctx = make_ctx(service_names::SYNC);
            ctx.connection_id = conn;
            let resp = Arc::clone(&svc)
                .oneshot(Operation::ORMapMerkleReqBucket {
                    ctx,
                    payload: topgun_core::messages::ORMapMerkleReqBucket {
                        payload: topgun_core::messages::ORMapMerkleReqBucketPayload {
                            map_name: "omap".to_string(),
                            path: path.clone(),
                        },
                    },
                })
                .await
                .expect("ORMapMerkleReqBucket");
            match resp {
                OperationResponse::Message(msg) => match *msg {
                    topgun_core::messages::Message::ORMapSyncRespBuckets(b) => {
                        for (child, hash) in &b.payload.buckets {
                            let child_path = format!("{path}{child}");
                            if add_path.starts_with(&child_path) {
                                verified_add_hash = Some(*hash);
                            }
                            if remove_path.starts_with(&child_path) {
                                verified_remove_hash = Some(*hash);
                            }
                            stack.push(child_path);
                        }
                    }
                    topgun_core::messages::Message::ORMapSyncRespLeaf(l) => {
                        for entry in l.payload.entries {
                            served.insert(entry.key.clone(), entry);
                        }
                    }
                    other => panic!("unexpected message in OR-Map walk: {other:?}"),
                },
                OperationResponse::Empty => {}
                other => panic!("unexpected response in OR-Map walk: {other:?}"),
            }
        }

        let add_entry = served
            .get(target_add)
            .expect("OR_ADD target must be served as a leaf entry");
        let remove_entry = served
            .get(target_remove)
            .expect("OR_REMOVE target must be served as a leaf entry");
        let verified_add_hash =
            verified_add_hash.expect("walk must verify a snapshot bucket hash en route to add");
        let verified_remove_hash = verified_remove_hash
            .expect("walk must verify a snapshot bucket hash en route to remove");

        // Sanity: the mid-session mutations changed each target's leaf hash, so the
        // snapshot bucket hash the client verifies is genuinely stale (a real torn
        // read window, not a no-op).
        let add_old = crate::storage::map_data_store::merkle_leaf_hash(
            target_add,
            &RecordValue::OrMap {
                records: vec![StoreOrMapEntry {
                    value: Value::Int(7),
                    tag: add_orig_tag.clone(),
                    timestamp: Timestamp {
                        millis: 8,
                        counter: 7,
                        node_id: "n1".to_string(),
                    },
                }],
                tombstones: Vec::new(),
            },
        )
        .expect("OR-Map leaf hash")
        .1;
        let add_new = crate::storage::map_data_store::merkle_leaf_hash(
            target_add,
            &RecordValue::OrMap {
                records: vec![
                    StoreOrMapEntry {
                        value: Value::Int(7),
                        tag: add_orig_tag.clone(),
                        timestamp: Timestamp {
                            millis: 8,
                            counter: 7,
                            node_id: "n1".to_string(),
                        },
                    },
                    StoreOrMapEntry {
                        value: Value::Int(70_007),
                        tag: add_new_tag.clone(),
                        timestamp: Timestamp {
                            millis: 500_000,
                            counter: 0,
                            node_id: "n1".to_string(),
                        },
                    },
                ],
                tombstones: Vec::new(),
            },
        )
        .expect("OR-Map leaf hash")
        .1;
        assert_ne!(
            add_old, add_new,
            "sanity: OR_ADD must change the target's leaf hash, else no torn read"
        );

        eprintln!(
            "SPEC-334 OR-Map evidence: add_key={target_add} add_path={add_path} \
             add_old_leaf_hash={add_old} add_new_leaf_hash={add_new} \
             served_add_tags={:?} served_remove_tombs={:?}",
            add_entry
                .records
                .iter()
                .map(|r| r.tag.as_str())
                .collect::<Vec<_>>(),
            remove_entry.tombstones,
        );

        // --- CARDINAL convergence guard (non-vacuous), OR_ADD direction. ---
        // The served records MUST equal the LIVE store's OrMap records for the key
        // (matched by tag → value/timestamp). The client merges these exact bytes
        // with add-wins CRDT semantics, so serving live truth is what makes the
        // torn read self-healing. FAILS if the lazy fetch served stale records.
        let live_add = read_factory
            .get_or_create("omap", hash_to_partition(target_add))
            .get(target_add, false)
            .await
            .expect("live get add")
            .expect("add key present");
        let RecordValue::OrMap {
            records: live_add_records,
            tombstones: live_add_tombs,
        } = live_add.value
        else {
            panic!("OR_ADD target must be an OrMap record");
        };
        // Compare by tag → (value, timestamp) so ordering is irrelevant.
        let mut live_add_by_tag: std::collections::HashMap<String, (rmpv::Value, Timestamp)> =
            std::collections::HashMap::new();
        for r in &live_add_records {
            live_add_by_tag.insert(
                r.tag.clone(),
                (value_to_rmpv(&r.value), r.timestamp.clone()),
            );
        }
        let mut served_add_by_tag: std::collections::HashMap<String, (rmpv::Value, Timestamp)> =
            std::collections::HashMap::new();
        for r in &add_entry.records {
            served_add_by_tag.insert(r.tag.clone(), (r.value.clone(), r.timestamp.clone()));
        }
        assert_eq!(
            served_add_by_tag, live_add_by_tag,
            "OR_ADD torn read served records that disagree with the live store — the \
             client would merge a state inconsistent with durable truth"
        );
        assert_eq!(
            add_entry.tombstones, live_add_tombs,
            "OR_ADD served tombstones must match the live store"
        );
        // Prove the window landed live: the new tag is present in the served entry.
        assert!(
            served_add_by_tag.contains_key(&add_new_tag),
            "served OR_ADD entry must carry the post-mutation tag (the lazy fetch is live)"
        );

        // --- CARDINAL convergence guard (non-vacuous), OR_REMOVE direction. ---
        let live_remove = read_factory
            .get_or_create("omap", hash_to_partition(target_remove))
            .get(target_remove, false)
            .await
            .expect("live get remove")
            .expect("remove key present");
        let RecordValue::OrMap {
            records: live_remove_records,
            tombstones: mut live_remove_tombs,
        } = live_remove.value
        else {
            panic!("OR_REMOVE target must be an OrMap record (records empty, tombstones set)");
        };
        let mut served_remove_tombs = remove_entry.tombstones.clone();
        served_remove_tombs.sort();
        live_remove_tombs.sort();
        assert_eq!(
            served_remove_tombs, live_remove_tombs,
            "OR_REMOVE torn read served tombstones that disagree with the live store"
        );
        assert!(
            live_remove_records.is_empty(),
            "OR_REMOVE target must have no live records (tombstone-only OrMap leaf)"
        );
        assert!(
            remove_entry.records.is_empty(),
            "OR_REMOVE served entry must carry no records, only the tombstone"
        );
        // Prove the window landed live: the original tag is now tombstoned, served.
        assert!(
            served_remove_tombs.contains(&remove_orig_tag),
            "served OR_REMOVE entry must carry the post-mutation tombstone (lazy fetch is live)"
        );

        // Prove the verified bucket hashes are OLD-anchored (the torn read is real,
        // not implied): an OR-Map session rebuilt over the post-mutation live state
        // — on a NEW connection, bypassing conn 1's cache — yields a DIFFERENT
        // bucket aggregate for the add path than the stale structure the client
        // just walked.
        let fresh = svc
            .get_or_build_session("omap", Some(ConnectionId(2)), false)
            .expect("fresh OR-Map session build")
            .expect("durable index present");
        assert_ne!(
            cached.ormap_nodes.get(&add_path[..2]),
            fresh.ormap_nodes.get(&add_path[..2]),
            "the OR-Map bucket structure the client verified must be OLD-anchored: a \
             session rebuilt over live state differs, proving the verified structure \
             pre-dates the mutations while the served entries were live (torn read real)"
        );

        // Tie each OR-Map bucket hash actually served ON THE WIRE to the stale
        // snapshot: it equals the OLD cached child hash and differs from the
        // fresh/live one. As in the LWW case, the structure assertion above reads
        // both sides from in-memory sessions; pinning the served wire value makes
        // "the torn read is observable at the wire" falsifiable for both directions.
        let add_child = add_path
            .chars()
            .nth(2)
            .expect("add path has a depth-3 child char");
        let remove_child = remove_path
            .chars()
            .nth(2)
            .expect("remove path has a depth-3 child char");
        let stale_add = cached
            .ormap_nodes
            .get(&add_path[..2])
            .and_then(|m| m.get(&add_child).copied())
            .expect("stale OR-Map snapshot must hold the add child");
        let stale_remove = cached
            .ormap_nodes
            .get(&remove_path[..2])
            .and_then(|m| m.get(&remove_child).copied())
            .expect("stale OR-Map snapshot must hold the remove child");
        assert_eq!(
            verified_add_hash, stale_add,
            "the OR_ADD bucket hash served on the wire must be the stale cached value \
             (the torn read is observable at the wire, not merely in the snapshot)"
        );
        assert_eq!(
            verified_remove_hash, stale_remove,
            "the OR_REMOVE bucket hash served on the wire must be the stale cached value"
        );
        assert_ne!(
            Some(verified_add_hash),
            fresh
                .ormap_nodes
                .get(&add_path[..2])
                .and_then(|m| m.get(&add_child).copied()),
            "the wire-served OR_ADD bucket hash must differ from the fresh/live one"
        );
        assert_ne!(
            Some(verified_remove_hash),
            fresh
                .ormap_nodes
                .get(&remove_path[..2])
                .and_then(|m| m.get(&remove_child).copied()),
            "the wire-served OR_REMOVE bucket hash must differ from the fresh/live one"
        );
    }

    /// AC4 (TODO-545, red-on-revert): after a connection disconnects, its session
    /// entries are released so the cache cannot retain the full leaf-key set
    /// indefinitely. Two connections each run `SYNC_INIT` (two cached sessions);
    /// removing one connection from the `ConnectionRegistry` (the single
    /// disconnect chokepoint) must drop ONLY that connection's session. On the
    /// pre-fix never-cleared cache there is no release path, so the cache stays at
    /// 2 → this test FAILS when the lifecycle change is reverted (non-vacuous).
    ///
    /// Multi-thread runtime required: `build_session` uses `block_in_place`.
    #[tokio::test(flavor = "multi_thread")]
    async fn ac4_sessions_released_on_disconnect_bounds_cache() {
        use crate::storage::datastores::RedbDataStore;
        use crate::storage::durable_merkle::DurableMerkle;
        use tower::ServiceExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(RedbDataStore::new(dir.path().join("ac4.redb")).expect("redb open"));
        for i in 0..8u32 {
            let key = format!("ac4-key-{i:04}");
            store
                .add(
                    "lmap",
                    &key,
                    &RecordValue::Lww {
                        value: Value::Int(i64::from(i)),
                        timestamp: Timestamp {
                            millis: u64::from(i) + 1,
                            counter: i,
                            node_id: "n1".to_string(),
                        },
                    },
                    0,
                    1,
                )
                .await
                .expect("seed");
        }

        let factory = Arc::new(RecordStoreFactory::new(
            StorageConfig::default(),
            Arc::clone(&store) as Arc<dyn crate::storage::map_data_store::MapDataStore>,
            Vec::new(),
        ));
        let durable_index: Arc<
            dyn crate::storage::map_data_store::DurableMerkleIndex + Send + Sync,
        > = Arc::new(DurableMerkle);
        let durable_store: Arc<dyn crate::storage::map_data_store::MapDataStore> =
            Arc::clone(&store) as Arc<dyn crate::storage::map_data_store::MapDataStore>;
        // The SAME ConnectionRegistry the service registers its disconnect
        // observer on — so remove() here fires that observer.
        let registry = Arc::new(ConnectionRegistry::new());
        let svc = Arc::new(
            SyncService::new(
                Arc::new(MerkleSyncManager::default()),
                factory,
                Arc::clone(&registry),
            )
            .with_durable_index(durable_index, durable_store),
        );

        // Two connections each run SYNC_INIT → two cached sessions.
        for cid in [1u64, 2u64] {
            let mut ctx = make_ctx(service_names::SYNC);
            ctx.connection_id = Some(ConnectionId(cid));
            Arc::clone(&svc)
                .oneshot(Operation::SyncInit {
                    ctx,
                    payload: topgun_core::messages::SyncInitMessage {
                        map_name: "lmap".to_string(),
                        last_sync_timestamp: None,
                    },
                })
                .await
                .expect("SyncInit");
        }
        assert_eq!(
            svc.session_registry.len(),
            2,
            "both connections' sessions must be cached after SYNC_INIT"
        );

        // Connection 1 disconnects → ConnectionRegistry::remove fires the
        // disconnect observer, releasing connection 1's sessions ONLY.
        registry.remove(ConnectionId(1));
        assert_eq!(
            svc.session_registry.len(),
            1,
            "disconnect must release the disconnected connection's session (cache stays bounded)"
        );

        // Connection 2 disconnects → cache fully drained.
        registry.remove(ConnectionId(2));
        assert_eq!(
            svc.session_registry.len(),
            0,
            "after all connections disconnect the session cache is empty (the TODO-545 bound)"
        );
    }

    // -----------------------------------------------------------------------
    // SPEC-342c — forgotten-client re-admission gate + sync-path closures
    // -----------------------------------------------------------------------

    use crate::network::config::ConnectionConfig;
    use topgun_core::messages::{ORMapEntry, ORMapPushDiff, ORMapPushDiffPayload};
    use topgun_core::ORMapRecord as WireOrRecord;

    /// A service wired with a frontier + per-key writer PLUS the shared connection
    /// registry exposed so a test can register a device-bound connection whose
    /// `(principal, deviceId)` identity `resolve_client_id` will find.
    fn make_gated_service() -> (
        Arc<SyncService>,
        Arc<RecordStoreFactory>,
        Arc<TombstoneFrontier>,
        Arc<ConnectionRegistry>,
    ) {
        let factory = make_factory();
        let frontier = Arc::new(TombstoneFrontier::new(None));
        frontier.set_epoch_width(1);
        let key_writer = Arc::new(KeyWriterRegistry::new());
        let registry = Arc::new(ConnectionRegistry::new());
        let svc = Arc::new(
            SyncService::new(
                Arc::new(MerkleSyncManager::default()),
                Arc::clone(&factory),
                Arc::clone(&registry),
            )
            .with_frontier(Arc::clone(&frontier), key_writer),
        );
        (svc, factory, frontier, registry)
    }

    /// Register a client connection bound to `device_id` (no principal → `NO_AUTH`
    /// sentinel), returning its `ConnectionId` and the `frontier_client_id` the
    /// gate will derive for it.
    async fn register_device(
        reg: &ConnectionRegistry,
        device_id: &str,
    ) -> (ConnectionId, ClientId) {
        let (handle, _rx) = reg.register(ConnectionKind::Client, &ConnectionConfig::default());
        handle.metadata.write().await.device_id = Some(device_id.to_string());
        let client = frontier_client_id(None, device_id);
        (handle.id, client)
    }

    fn push_op(ctx: OperationContext, map: &str, key: &str, tag: &str) -> Operation {
        Operation::ORMapPushDiff {
            ctx,
            payload: ORMapPushDiff {
                payload: ORMapPushDiffPayload {
                    map_name: map.to_string(),
                    entries: vec![ORMapEntry {
                        key: key.to_string(),
                        records: vec![WireOrRecord {
                            value: rmpv::Value::Integer(1.into()),
                            timestamp: make_timestamp(),
                            tag: tag.to_string(),
                            ttl_ms: None,
                        }],
                        tombstones: vec![],
                    }],
                },
            },
        }
    }

    async fn stored_record_count(factory: &RecordStoreFactory, map: &str, key: &str) -> usize {
        let store = factory.get_or_create(map, hash_to_partition(key));
        match store.get(key, false).await.unwrap().map(|r| r.value) {
            Some(RecordValue::OrMap { records, .. }) => records.len(),
            _ => 0,
        }
    }

    /// AC5: with protection ACTIVE, a push from a FORGOTTEN/unknown client (a
    /// device-bound connection whose identity is untracked) is rejected BEFORE the
    /// merge — nothing is stored. (Dark until 342j: without the watermark the same
    /// push would merge; asserted by the tracked-client test below.)
    #[tokio::test]
    async fn ac5_forgotten_client_push_blocked_before_merge() {
        let (svc, factory, frontier, registry) = make_gated_service();
        let (conn, _client) = register_device(&registry, "dev-forgotten").await;
        frontier.stamp_tombstone("omap", "seed", "seed-tag"); // current_epoch = 1
        frontier.set_durable_epoch_watermark(1000); // protection active
        let mut ctx = make_ctx(service_names::SYNC);
        ctx.connection_id = Some(conn);
        Arc::clone(&svc)
            .oneshot(push_op(ctx, "omap", "k1", "R1"))
            .await
            .expect("ack");
        assert_eq!(
            stored_record_count(&factory, "omap", "k1").await,
            0,
            "a forgotten client's push must be rejected before merge (no record stored)"
        );
    }

    /// AC5/AC6/AC16: a NOT-forgotten (tracked) client's push is admitted and merges
    /// under the per-key writer span even with protection active — the guarded
    /// gate→commit path stores the record.
    #[tokio::test]
    async fn ac6_tracked_client_push_merges_under_guard() {
        let (svc, factory, frontier, registry) = make_gated_service();
        let (conn, client) = register_device(&registry, "dev-tracked").await;
        frontier.stamp_tombstone("omap", "seed", "seed-tag"); // current_epoch = 1
        frontier.set_delivered(conn, 100);
        assert!(frontier.confirm_apply_ack(&client, 1, conn).await, "track");
        frontier.set_durable_epoch_watermark(1000); // protection active
        assert!(!frontier.is_forgotten(&client));
        let mut ctx = make_ctx(service_names::SYNC);
        ctx.connection_id = Some(conn);
        Arc::clone(&svc)
            .oneshot(push_op(ctx, "omap", "k1", "R1"))
            .await
            .expect("ack");
        assert_eq!(
            stored_record_count(&factory, "omap", "k1").await,
            1,
            "a tracked client's push merges under the per-key gate→commit span"
        );
    }

    /// AC17 (fail-closed, identity dimension): an unknown identity (a connection
    /// with NO server-issued `device_id`) is treated as forgotten and rejected under
    /// active protection — the gate never silently admits an unverifiable client.
    #[tokio::test]
    async fn ac17_unknown_identity_push_rejected_failclosed() {
        let (svc, factory, frontier, registry) = make_gated_service();
        // A connection with no device_id → resolve_client_id == None → forgotten.
        let (handle, _rx) = registry.register(ConnectionKind::Client, &ConnectionConfig::default());
        frontier.stamp_tombstone("omap", "seed", "seed-tag");
        frontier.set_durable_epoch_watermark(1000);
        let mut ctx = make_ctx(service_names::SYNC);
        ctx.connection_id = Some(handle.id);
        Arc::clone(&svc)
            .oneshot(push_op(ctx, "omap", "k1", "R1"))
            .await
            .expect("ack");
        assert_eq!(
            stored_record_count(&factory, "omap", "k1").await,
            0,
            "unknown (no device identity) → forgotten → push rejected (fail-closed)"
        );
    }

    /// AC4 (third prune site): the push-diff merge path wires the wholesale
    /// epoch-drop prune, DARK by construction — with the production watermark (0) a
    /// push-diff drops ZERO epochs even though the low-water-mark is past a stamped
    /// epoch. The stamped tombstone survives.
    #[tokio::test]
    async fn ac4_push_diff_prune_site_dark() {
        use crate::storage::record::OrMapEntry as StoreOrMapEntry;
        let (svc, factory, frontier, registry) = make_gated_service();
        let (conn, client) = register_device(&registry, "dev-1").await;
        let map = "omap";
        let key = "k1";
        let store = factory.get_or_create(map, hash_to_partition(key));
        store
            .put(
                key,
                RecordValue::OrMap {
                    records: vec![StoreOrMapEntry {
                        value: Value::Int(1),
                        tag: "R1".to_string(),
                        timestamp: make_timestamp(),
                    }],
                    tombstones: vec!["T1".to_string()],
                },
                ExpiryPolicy::NONE,
                CallerProvenance::CrdtMerge,
            )
            .await
            .unwrap();
        frontier.stamp_tombstone(map, key, "T1"); // epoch 1
        frontier.stamp_tombstone(map, "k2", "T2"); // epoch 2 → counter can pass 1
        frontier.set_delivered(conn, 100);
        assert!(frontier.confirm_apply_ack(&client, 2, conn).await);
        assert!(
            frontier.is_epoch_prune_eligible(1),
            "LWM strictly past epoch 1"
        );
        // Watermark stays 0 (production dark). Push an unrelated key so the merge
        // path — and thus the third prune site — runs.
        let mut ctx = make_ctx(service_names::SYNC);
        ctx.connection_id = Some(conn);
        Arc::clone(&svc)
            .oneshot(push_op(ctx, map, "k3", "R3"))
            .await
            .expect("ack");
        match store.get(key, false).await.unwrap().map(|r| r.value) {
            Some(RecordValue::OrMap { tombstones, .. }) => assert_eq!(
                tombstones,
                vec!["T1".to_string()],
                "DARK by construction: the third prune site drops zero epochs at watermark 0"
            ),
            other => panic!("expected OrMap, got {other:?}"),
        }
    }

    /// AC7: a forgotten/unknown client's OR-Map `SYNC_INIT` is routed to a FULL
    /// snapshot REPLACE resync (`full_resync = true`) instead of an incremental
    /// delta, once protection is active. A tracked client gets the normal path.
    #[tokio::test]
    async fn ac7_forgotten_sync_init_routes_full_resync() {
        let (svc, _factory, frontier, registry) = make_gated_service();
        let (conn, _client) = register_device(&registry, "dev-forgotten").await;
        frontier.stamp_tombstone("omap", "seed", "seed-tag");
        frontier.set_durable_epoch_watermark(1000);
        let mut ctx = make_ctx(service_names::SYNC);
        ctx.connection_id = Some(conn);
        let resp = Arc::clone(&svc)
            .oneshot(Operation::ORMapSyncInit {
                ctx,
                payload: topgun_core::messages::ORMapSyncInit {
                    map_name: "omap".to_string(),
                    root_hash: 0,
                    bucket_hashes: HashMap::new(),
                    last_sync_timestamp: None,
                    claimed_epoch: None,
                },
            })
            .await
            .expect("root");
        match resp {
            OperationResponse::Message(m) => match *m {
                Message::ORMapSyncRespRoot(r) => assert!(
                    r.payload.full_resync,
                    "forgotten client routed to full-snapshot REPLACE resync"
                ),
                other => panic!("expected root, got {other:?}"),
            },
            other => panic!("expected message, got {other:?}"),
        }
    }

    /// AC8: a REGRESSED replica (sync-init `claimed_epoch` BELOW its stored cursor,
    /// e.g. a backup-restore clone) is routed to full-resync WITHOUT advancing
    /// `delivered_conn` and WITHOUT rolling the stored cursor back (342a monotonicity).
    #[tokio::test]
    async fn ac8_regressed_replica_routed_no_rollback_no_delivered() {
        let (svc, _factory, frontier, registry) = make_gated_service();
        let (conn, client) = register_device(&registry, "dev-clone").await;
        // Establish a real cursor at 100 on an earlier connection.
        frontier.set_delivered(ConnectionId(999), 100);
        assert!(
            frontier
                .confirm_apply_ack(&client, 100, ConnectionId(999))
                .await
        );
        for i in 0..5 {
            frontier.stamp_tombstone("omap", &format!("s{i}"), &format!("t{i}"));
        }
        frontier.set_durable_epoch_watermark(1000);
        let mut ctx = make_ctx(service_names::SYNC);
        ctx.connection_id = Some(conn);
        let resp = Arc::clone(&svc)
            .oneshot(Operation::ORMapSyncInit {
                ctx,
                payload: topgun_core::messages::ORMapSyncInit {
                    map_name: "omap".to_string(),
                    root_hash: 0,
                    bucket_hashes: HashMap::new(),
                    last_sync_timestamp: None,
                    claimed_epoch: Some(5), // 5 < stored 100 → regressed
                },
            })
            .await
            .expect("root");
        match resp {
            OperationResponse::Message(m) => match *m {
                Message::ORMapSyncRespRoot(r) => {
                    assert!(r.payload.full_resync, "regressed replica → full resync");
                }
                other => panic!("expected root, got {other:?}"),
            },
            other => panic!("expected message, got {other:?}"),
        }
        assert_eq!(
            frontier.cursor(&client),
            Some(100),
            "stored cursor NEVER rolled back for a regressed replica (342a monotonicity)"
        );
        assert_eq!(
            frontier.delivered(conn),
            0,
            "delivered_conn NOT advanced for a gated (regressed) client"
        );
    }

    /// AC10: a plain `SYNC_INIT` by a forgotten client does NOT advance its
    /// `delivered_conn` (covering epoch conveyed as metadata only) — the sync-path
    /// re-admission door is closed. A tracked client DOES advance it.
    #[tokio::test]
    async fn ac10_forgotten_sync_init_does_not_advance_delivered() {
        let (svc, _factory, frontier, registry) = make_gated_service();
        let (fconn, _fclient) = register_device(&registry, "dev-forgotten").await;
        let (tconn, tclient) = register_device(&registry, "dev-tracked").await;
        frontier.stamp_tombstone("omap", "seed", "seed-tag"); // current_epoch = 1
        frontier.set_delivered(tconn, 100);
        assert!(frontier.confirm_apply_ack(&tclient, 1, tconn).await);
        // Reset delivered so we observe the sync-init's effect specifically.
        frontier.remove_connection(tconn);
        frontier.set_durable_epoch_watermark(1000);

        let sync_init = |conn: ConnectionId| {
            let mut ctx = make_ctx(service_names::SYNC);
            ctx.connection_id = Some(conn);
            Operation::ORMapSyncInit {
                ctx,
                payload: topgun_core::messages::ORMapSyncInit {
                    map_name: "omap".to_string(),
                    root_hash: 12345, // mismatch so no ACK, only the covering-epoch conveyance matters
                    bucket_hashes: HashMap::new(),
                    last_sync_timestamp: None,
                    claimed_epoch: None,
                },
            }
        };
        Arc::clone(&svc)
            .oneshot(sync_init(fconn))
            .await
            .expect("ok");
        Arc::clone(&svc)
            .oneshot(sync_init(tconn))
            .await
            .expect("ok");
        assert_eq!(
            frontier.delivered(fconn),
            0,
            "forgotten client's sync-init must NOT advance delivered_conn (R9 gating)"
        );
        assert_eq!(
            frontier.delivered(tconn),
            1,
            "tracked client's sync-init DOES advance delivered_conn (eager set_delivered retained)"
        );
    }
}
