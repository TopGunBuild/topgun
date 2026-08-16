//! Concrete [`CausalFrontier`] implementation — the first Wave-2 consumer of the
//! traits-only contract in [`crate::tombstone_frontier`].
//!
//! This is the durable per-device confirmed-apply cursor store that feeds the
//! prune low-water-mark. It is populated ONLY by authenticated client→server
//! confirmed-apply ACKs (see the websocket handler), never on receive or on
//! sync-initiation, and every advance is loss-conservative, delivered-clamped,
//! globally-bounded, and connection-ownership-fenced.
//!
//! # Identity
//!
//! A cursor is keyed by the opaque [`ClientId`] produced by the LIVE
//! `network::device_identity::frontier_client_id(principal, deviceId)` encoding —
//! a fully server-authenticated `(principal, deviceId)` replica identity. There is
//! NO client-asserted identity anywhere in this store, and no identity field on
//! the ACK wire message; the caller derives the `ClientId` from the connection
//! state before touching the frontier.
//!
//! # Advance rule (R6c)
//!
//! `new = max(stored, min(claimed, delivered_conn, current_max_epoch))`, applied
//! monotonically. A claim is clamped to the highest epoch actually DELIVERED on
//! the connection the ACK arrived on (`delivered_conn`, per-connection, initialized to 0) and
//! then bounded by the server's current max stamped epoch (`current_max_epoch`,
//! the belt-and-suspenders final bound). A connection that has been delivered
//! nothing (`delivered_conn == 0`) therefore cannot establish OR advance any
//! cursor — closing the mint→ack-low→abandon low-water-mark-pinning denial of
//! service and the re-track-during-pending-resync hole. A cursor value of 0 is
//! never tracked (it
//! pins nothing); tracked cursors are always `>= 1`.
//!
//! Both `delivered_conn` and `current_max_epoch` are **settable/injectable**: no
//! production epoch-stamping provider exists yet, so at Wave 2
//! `current_max_epoch` defaults to `u64::MAX` (the
//! global bound is inert — `delivered_conn` is the operative clamp) and every
//! `delivered_conn` defaults to 0. The delta-delivery path and 342b later feed
//! real values. The settable fields make the delivered-clamp and global-bound
//! rejection tests unit-testable in isolation before 342b lands.
//!
//! # Fencepost (R6, inclusive)
//!
//! `confirm_apply(E)` is INCLUSIVE: it asserts the client has applied everything
//! up to AND INCLUDING epoch `E`. The low-water-mark is the MIN cursor across all
//! tracked clients — so a client whose cursor is `E` has applied `E`, and its
//! predecessor `E-1` can never be resurrected on it.
//!
//! # Persistence
//!
//! The cursor is persisted best-effort into a NEW additive redb keyspace (the
//! reserved [`CURSOR_MAP`] namespace), distinct from the delta-sync
//! `last_sync_timestamp` hint and from the device-credential keyspace. Cursor loss
//! is SAFE: a client whose cursor is gone reads as unknown → forgotten → full
//! resync (the durability of the prune itself is fenced at prune time in 342b, not
//! here). On connection establishment for a KNOWN identity the persisted cursor is
//! rehydrated into the in-memory frontier BEFORE any ACK, so a reconnecting device
//! pins the low-water-mark at its true confirmed-apply cursor instead of falling
//! through the "unknown == forgotten" path (which would let the LWM jump forward).

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use tracing::{debug, info, warn};

use topgun_core::hlc::Timestamp;
use topgun_core::types::Value;

use crate::network::connection::ConnectionId;
use crate::reclamation_registry::{
    ClaimAdmission, ClaimScope, ReclamationBoundary, ReclamationRegistry,
};
use crate::storage::map_data_store::MapDataStore;
use crate::storage::record::RecordValue;
use crate::tombstone_frontier::{
    CausalFrontier, ClientId, Epoch, EpochExitKind, GateToken, PruneClaimSpanRecord,
    PruneEpochEntryRecord, PruneEpochRecord, PruneEpochResidencyRecord, PrunePassRecord,
    PruneRecordArming, PruneRecordObserver, PruneSafety, METRIC_PRUNE_ABSENT_TOTAL,
    METRIC_PRUNE_BYTES_FREED_TOTAL, METRIC_PRUNE_CLAIM_LAG_EPOCHS, METRIC_PRUNE_CLAIM_SPAN_EPOCHS,
    METRIC_PRUNE_CONSIDERED_TOTAL, METRIC_PRUNE_CURRENT_EPOCH, METRIC_PRUNE_DRAINED_REFS_TOTAL,
    METRIC_PRUNE_DRAIN_EPOCHS, METRIC_PRUNE_DRAIN_REFS, METRIC_PRUNE_DROPPED_TOTAL,
    METRIC_PRUNE_DURABLE_EPOCH_WATERMARK, METRIC_PRUNE_ELIGIBLE_REFS,
    METRIC_PRUNE_EMPTY_DRAINS_TOTAL, METRIC_PRUNE_EPOCHS_DRAINED_TOTAL,
    METRIC_PRUNE_EPOCHS_ENTERED_TOTAL, METRIC_PRUNE_EPOCHS_EXITED_TOTAL,
    METRIC_PRUNE_EPOCH_BYTES_FREED, METRIC_PRUNE_EPOCH_CONSIDERED, METRIC_PRUNE_EPOCH_DROPPED,
    METRIC_PRUNE_INDEXED_EPOCHS, METRIC_PRUNE_INDEXED_REFS, METRIC_PRUNE_INELIGIBLE_REFS,
    METRIC_PRUNE_LAST_DRAINED_EPOCH, METRIC_PRUNE_LOW_WATER_MARK, METRIC_PRUNE_LWM_ADVANCES_TOTAL,
    METRIC_PRUNE_LWM_EPOCHS_ADVANCED_TOTAL, METRIC_PRUNE_LWM_STALL_SECONDS,
    METRIC_PRUNE_MATCHED_NOTHING_TOTAL, METRIC_PRUNE_NONEMPTY_DRAINS_TOTAL,
    METRIC_PRUNE_PASSES_TOTAL, METRIC_PRUNE_REBUILD_CLEARED_REFS_TOTAL,
    METRIC_PRUNE_REMOVED_BYTES_OBSERVED_TOTAL, METRIC_PRUNE_REMOVED_REFS_OBSERVED_TOTAL,
    METRIC_PRUNE_RESTORED_EVICTED_TOTAL, METRIC_PRUNE_RESTORED_READ_ERROR_TOTAL,
    METRIC_PRUNE_RESTORED_REFS_TOTAL, METRIC_PRUNE_RESTORED_WRITE_ERROR_TOTAL,
    METRIC_PRUNE_SPLIT_COMPUTED_EPOCH, METRIC_PRUNE_SPLIT_RECOMPUTES_TOTAL,
    METRIC_PRUNE_STAMPED_BYTES_TOTAL, METRIC_PRUNE_STAMPED_REFS_TOTAL, METRIC_PRUNE_TRACKED_CLAIMS,
};
use metrics::{Counter, Gauge, Histogram};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

/// Default epoch width (stamped tombstone ops per epoch) when
/// `TOPGUN_EPOCH_WIDTH` is unset. The server-authoritative epoch counter
/// advances one step per this many genuinely-new tombstones, in lockstep with
/// the cursor-tracked op sequence — never by a timer.
pub const DEFAULT_EPOCH_WIDTH: u64 = 1000;

/// Default max cursor-lag (in epochs) before a tracked client is treated as
/// forgotten by the re-admission gate. Retention is expressed in cursor LAG, not
/// wall-clock (a lagging cursor is what pins tombstones). A tracked client whose
/// cursor lags MORE than this many epochs behind the current server epoch is
/// forgotten (its reconnect push is gated → full-resync). RAM pressure MAY
/// dynamically tighten this at runtime; the operator override is a follow-up env
/// wiring (`TOPGUN_FORGET_MAX_LAG_EPOCHS`) — the default is a safe implementation
/// detail here.
pub const DEFAULT_FORGET_LAG_EPOCHS: u64 = 1000;

/// The storage location of a stamped tombstone: the `(map, key)` its OR-Map
/// record lives under plus the tombstone `tag`. The server-side `epoch → tags`
/// index stores these so a wholesale epoch-drop prune can remove each tag from
/// its record in storage (RAM + redb).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TombstoneRef {
    /// The OR-Map name the tombstone belongs to.
    pub map: String,
    /// The key within the map whose OR-Map record holds the tombstone.
    pub key: String,
    /// The observed-remove tombstone tag (`"millis:counter:nodeId"`).
    pub tag: String,
}

/// Per-epoch bookkeeping backing the residency ledger's two emissions (R2.3a / R2.3b).
///
/// Created on an epoch's first stamp and held until its exit row fires, at which point it is
/// removed from [`FrontierState::epoch_slots`] — the slot's whole reason to exist is bounded,
/// per-tracked-epoch bookkeeping, never an unbounded audit trail. The number of these alive at
/// once is bounded by `indexed_epochs`, which stays small in practice, so this map does not
/// grow with total tombstone volume.
#[derive(Debug, Clone, Copy, Default)]
struct EpochResidencySlot {
    /// Refs stamped into this epoch over its whole lifetime so far.
    stamped_refs: u64,
    /// Tombstone bytes stamped into this epoch over its whole lifetime so far.
    stamped_bytes: u64,
    /// The op-seq at which this epoch's first ref entered the index.
    entered_at_op_seq: u64,
    /// Wall-clock (Unix ms) at which this epoch's first ref entered the index.
    entered_at_unix_ms: i64,
    /// `stamped_refs`, frozen at the instant the entry row was emitted (rollover). `0` while
    /// the epoch is still current and this slot is still accumulating.
    refs_at_entry: u64,
    /// Whether the entry row has fired for this epoch yet. `false` while the epoch is still
    /// current; once the exit row fires the slot is removed entirely, so `true` here always
    /// means "tracked: entry emitted, exit pending".
    entry_emitted: bool,
    /// The first op-seq at which `low_water_mark > epoch` was observed, set the first time
    /// [`FrontierState::refresh_epoch_licensing`] sees it become true (R3.2's LICENSED term).
    lwm_passed_at_op_seq: Option<u64>,
    /// The first op-seq at which `durable_epoch_watermark >= epoch` was observed (R3.2's
    /// FENCED term).
    fence_passed_at_op_seq: Option<u64>,
}

/// What the bookkeeping itself observed causing a tracked epoch's absence from
/// `epoch_tags` (or, for [`FinalExitKind::StillResidentAtShutdown`], its still-resident
/// state at teardown) — passed to [`FrontierState::detect_epoch_exit`] /
/// [`FrontierState::force_shutdown_exits`] as a hint from the removal call site.
///
/// `None` at [`FrontierState::detect_epoch_exit`]'s call site means "no attributable
/// cause on record for this particular epoch" — which is exactly
/// [`EpochExitKind::Unclassified`]'s reachability condition (R2.3b): an absence detected
/// with no accompanying hint is what a removal reached by an unenumerated path looks
/// like to this bookkeeping. A one-to-one mirror of [`EpochExitKind`]'s three named
/// variants, kept as a separate (non-`pub`) type because the fourth, `Unclassified`, is
/// never a HINT — it is [`FrontierState::finalize_epoch_exit`]'s OWN conclusion when no
/// hint applies, never something a caller asserts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FinalExitKind {
    DrainedByPrune,
    ClearedByRebuild,
    StillResidentAtShutdown,
}

/// The eight quantities O-0's index-conservation identity is evaluated over
/// (`stamped_refs_total + restored_refs_total − drained_refs_total − rebuild_cleared_refs_total
/// ≡ indexed_refs`), read as one internally-coherent tuple under the frontier's own lock — the
/// accessor the double-read sampling rule's two renders are taken from (R3.0 limb 6).
///
/// Maintained as `FrontierState`'s own fields, updated at the exact same sites that already
/// maintain `indexed_refs` incrementally (stamp / drain / restore / rebuild), so this snapshot
/// and `indexed_refs` can never observe two different instants of the same mutation sequence —
/// unlike two independent Prometheus counter reads, which are two independent atomics and can
/// tear against a concurrent mutation. Two calls to
/// [`FrontierState::index_conservation_snapshot`] with no intervening mutation are therefore
/// equal BY CONSTRUCTION, which is the property the identity test exercises.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IndexConservationSnapshot {
    /// Refs stamped into the index over the frontier's whole lifetime.
    pub stamped_refs_total: u64,
    /// Tombstone bytes stamped into the index over the frontier's whole lifetime.
    pub stamped_bytes_total: u64,
    /// Refs removed from the index by an observed prune drain.
    pub drained_refs_total: u64,
    /// Refs re-inserted into the index by an observed restore.
    pub restored_refs_total: u64,
    /// Refs cleared from the index by an observed rebuild, credited as a reset rather than a
    /// drain against the conservation identity.
    pub rebuild_cleared_refs_total: u64,
    /// Epochs that emitted an entry row.
    pub epochs_entered_total: u64,
    /// Epochs that emitted an exit row.
    pub epochs_exited_total: u64,
    /// The carried, incrementally-maintained index size — the identity's right-hand side.
    pub indexed_refs: u64,
}

/// Reserved redb map namespace for the durable confirmed-apply cursors.
///
/// A NEW additive keyspace — it does NOT repurpose the delta-sync
/// `last_sync_timestamp` hint. Kept clear of the user-map namespace by the
/// `_topgun_` convention (matches `is_valid_map_name`, does not end in
/// `__backup`). The record KEY is the opaque `ClientId` (`frontier_client_id`
/// encoding).
///
/// `_v2`: one-shot poison-purge, version-bumped by the cross-map
/// covering-epoch fix. Before that fix a client's device-wide cursor could be
/// confirmed off a SINGLE OR-Map's sync completion while the epoch counter and
/// cursor are GLOBAL across all OR-Maps — an inflated claim that could outrun
/// what the client actually received for other held maps. Those inflated
/// cursors are already durably persisted under the pre-bump keyspace and would
/// otherwise survive a clean restart straight into 342j's prune activation.
/// Renaming the keyspace makes every pre-bump row permanently unreachable
/// (never migrated, never read again) rather than trusted — a "loss" here is
/// the SAME safe fallback the whole cursor model already relies on (unknown →
/// forgotten → full resync), so no migration is needed. Orphaned pre-bump rows
/// are reclaimed later by 342f's TTL sweep.
pub const CURSOR_MAP: &str = "_topgun_tombstone_cursors_v2";

/// In-memory state of the causal frontier. Implements the [`CausalFrontier`]
/// contract from 342a; guarded by a `Mutex` inside [`TombstoneFrontier`] for
/// shared `&self` access across connections.
struct FrontierState {
    /// Per-client confirmed-apply high-water-mark. A client is TRACKED iff it has
    /// an entry here; entries are always `>= 1` (a 0 cursor pins nothing and is
    /// never stored). Its MIN is the prune low-water-mark.
    cursors: HashMap<ClientId, Epoch>,
    /// Highest epoch the server has DELIVERED on a given connection (per-connection,
    /// in-memory, initialized to 0 on first use). Clamps a claim so a connection
    /// cannot ACK past what it received. Loss on reconnect/crash is conservative —
    /// it can only suppress advances, never permit a bad one.
    delivered: HashMap<ConnectionId, Epoch>,
    /// The server's current max stamped epoch — the final belt-and-suspenders
    /// bound against a claim for an epoch the server never stamped. Injectable;
    /// defaults to `u64::MAX` (inert) until the first tombstone is stamped, after
    /// which each stamp sets it to the real (highest) stamped epoch.
    current_max_epoch: Epoch,
    /// Server-authoritative op sequence: increments once per genuinely-new
    /// tombstone stamped. The epoch is derived from this so "cursor past epoch N"
    /// implies "delivered-and-applied every tombstone stamped ≤ N" (epoch↔seq
    /// lockstep — never a timer, never a non-`OR_REMOVE` trigger). Starts at 0;
    /// the first stamp makes it 1.
    op_seq: u64,
    /// Width of an epoch in stamped ops (`TOPGUN_EPOCH_WIDTH`, clamped `>= 1`).
    /// The epoch counter advances one step per this many stamped tombstones.
    epoch_width: u64,
    /// Highest epoch the server has stamped (0 = none yet). Conveyed to clients
    /// as the covering epoch and fed into `current_max_epoch` as the ACK bound.
    /// Always `>= 1` once any tombstone is stamped — 0 is the reserved
    /// "no/uncomputable epoch" sentinel and no tombstone is ever stamped 0.
    current_epoch: Epoch,
    /// RAM-only `epoch → tombstone refs` index (pure CACHE — never durable on the
    /// hot path; unclean recovery rebuilds it). Keyed by the ACTUAL
    /// stamped epoch; key 0 is never inserted, so the prune sweep (which iterates
    /// these keys, never a `0..=max` range) can never touch the sentinel.
    epoch_tags: HashMap<Epoch, Vec<TombstoneRef>>,
    /// RAM-only `epoch → max assigned write-sequence` index: the highest
    /// write-behind entry sequence (`MapDataStore::assigned_write_sequence`)
    /// snapshotted when a tombstone was stamped into each epoch. This is the
    /// bridge between the epoch counter and byte durability: epoch `E` is
    /// byte-durable once the store's prefix-complete `flushed_watermark()` has
    /// reached `max(epoch_max_seq[e] for e <= E)`. Since a tombstone's own
    /// byte-write is enqueued strictly before its stamp, the snapshot is an
    /// upper bound on that write's sequence — a conservative, never-premature
    /// fence.
    epoch_max_seq: HashMap<Epoch, u64>,
    /// Cached byte-durability watermark: `max E such that every stamped epoch
    /// e <= E has epoch_max_seq[e] <= flushed_watermark`. Recomputed on demand
    /// from [`FrontierState::compute_durable_epoch_watermark`] against the store's
    /// live flushed watermark (see [`TombstoneFrontier::refreshed_watermark`]);
    /// 0 from construction until either the first byte-durable epoch or the
    /// unclean-recovery rebuild fills the index (R12(e): 0 until the pre-listener
    /// rebuild completes). Tests with no store inject it directly via
    /// `set_durable_epoch_watermark` to exercise the drop path in isolation.
    ///
    /// This watermark ALSO gates the re-admission gate's active blocking (see
    /// [`TombstoneFrontier::is_protection_active`]): a forgotten client's push can
    /// only resurrect a value whose tombstone was PRUNED, and pruning is licensed
    /// only once this watermark is non-zero — so gate and prune activate together
    /// (gate-before-activation, no prune-without-gate window).
    durable_epoch_watermark: Epoch,
    /// Max cursor-lag (epochs) before a tracked client is forgotten by the gate.
    /// Defaults to [`DEFAULT_FORGET_LAG_EPOCHS`]; settable so RAM pressure / an
    /// operator override can tighten it.
    forget_lag_epochs: u64,
    /// Total tombstone refs held across every entry of `epoch_tags`, maintained
    /// INCREMENTALLY at stamp / drain / restore / rebuild. Summing the per-epoch
    /// vector lengths would be a fold over the whole index on a path that runs on
    /// every `OR_REMOVE`, so the count is carried rather than derived. (The epoch
    /// count needs no companion field — `HashMap::len` is already O(1).)
    indexed_refs: u64,
    /// The highest epoch any drain has removed from the index, 0 before the first
    /// drain. Last-value only; the per-epoch join is not representable over the
    /// metrics transport anyway.
    last_drained_epoch: Epoch,
    /// The low-water-mark as of the last cursor mutation. Cached so the per-remove
    /// path can publish the LWM without folding the cursor map: the MIN can only
    /// change when a cursor is inserted, raised or removed, and every such site
    /// refreshes this through [`Self::refresh_low_water_mark`].
    observed_lwm: Epoch,
    /// Wall-clock millis of the last low-water-mark ADVANCE, seeded at construction
    /// so the stall is measured from process start rather than from the epoch (which
    /// would render a ~57-year stall until the first advance).
    last_lwm_advance_millis: u64,
    /// Per-epoch residency bookkeeping: tracked from an epoch's first stamp until its exit
    /// row fires (R2.3a / R2.3b). Bounded by `indexed_epochs`, never by tombstone volume.
    epoch_slots: HashMap<Epoch, EpochResidencySlot>,
    /// O-0's conservation-identity counters, maintained at the exact same sites that already
    /// maintain `indexed_refs` incrementally. See [`IndexConservationSnapshot`].
    stamped_refs_total: u64,
    stamped_bytes_total: u64,
    drained_refs_total: u64,
    restored_refs_total: u64,
    rebuild_cleared_refs_total: u64,
    epochs_entered_total: u64,
    epochs_exited_total: u64,
    /// The reclamation boundary this frontier's prune side folds over, shared with the
    /// [`TombstoneFrontier`] wrapper that owns it. Every cursor writer here is also a claim site
    /// on it, so the claim set and the cursor map move together; the prune side then reads the
    /// ceiling from this ONE authority instead of re-deriving a boundary of its own.
    registry: Arc<ReclamationRegistry>,
}

impl FrontierState {
    fn new(registry: Arc<ReclamationRegistry>) -> Self {
        Self {
            cursors: HashMap::new(),
            delivered: HashMap::new(),
            current_max_epoch: Epoch::MAX,
            op_seq: 0,
            epoch_width: DEFAULT_EPOCH_WIDTH,
            current_epoch: 0,
            epoch_tags: HashMap::new(),
            epoch_max_seq: HashMap::new(),
            durable_epoch_watermark: 0,
            forget_lag_epochs: DEFAULT_FORGET_LAG_EPOCHS,
            indexed_refs: 0,
            last_drained_epoch: 0,
            observed_lwm: 0,
            last_lwm_advance_millis: now_millis(),
            epoch_slots: HashMap::new(),
            stamped_refs_total: 0,
            stamped_bytes_total: 0,
            drained_refs_total: 0,
            restored_refs_total: 0,
            rebuild_cleared_refs_total: 0,
            epochs_entered_total: 0,
            epochs_exited_total: 0,
            registry,
        }
    }

    /// The reclamation ceiling: the fleet boundary the prune side is licensed to reclaim strictly
    /// below.
    ///
    /// This is an OBSERVING call — the registry counts the query and republishes its gauges — so
    /// callers hoist it into a local and reuse it rather than calling it once per epoch, which
    /// would report the prune's own internal arithmetic as operator-visible query volume.
    fn reclamation_ceiling(&self) -> Epoch {
        self.registry.prune_ceiling(ClaimScope::Global)
    }

    /// Record a claim for `client` at `epoch` on the reclamation boundary.
    ///
    /// Every caller sits AFTER its own zero guard: a 0 cursor pins nothing and is never stored, so
    /// offering one here would record a claim that pins the ceiling at 0 forever while rendering
    /// indistinguishably from "no claim at all" on the fleet-min gauge.
    fn register_claim(&self, client: &ClientId, epoch: Epoch) -> ClaimAdmission {
        self.registry
            .register_claim(client, ClaimScope::Global, epoch)
    }

    /// Whether `client` is FORGOTTEN for re-admission-gate purposes: either
    /// UNKNOWN (never tracked — "unknown == forgotten" per the 342a contract) OR a
    /// tracked client whose cursor has lagged MORE than `forget_lag_epochs` behind
    /// the current server epoch. This is the lag-aware predicate the gate uses at
    /// gate time AND re-checks at commit time (via `gate_decision_holds_at_commit`),
    /// so a client that crosses the lag threshold mid-handler (a concurrent stamp
    /// advancing `current_epoch`) is caught at commit — closing the lag-driven
    /// gate→commit TOCTOU the stock `is_tracked`-only check would miss.
    fn is_forgotten(&self, client: &ClientId) -> bool {
        match self.cursors.get(client) {
            None => true,
            Some(&cursor) => self.current_epoch.saturating_sub(cursor) > self.forget_lag_epochs,
        }
    }

    /// Applies the bounded, monotone advance rule for a confirmed-apply ACK.
    ///
    /// `new = max(stored, min(claimed, delivered_conn, current_max_epoch))`. Returns
    /// `Some(new)` when the stored cursor actually advanced (the caller persists it),
    /// or `None` on a no-op (dropped/replayed/reordered/over-claimed/delivered-clamped
    /// ACK). A delivered-nothing connection (`delivered_conn == 0`) can never
    /// establish or advance a cursor: the bound is 0, so a fresh device stays
    /// untracked.
    fn advance_on_ack(
        &mut self,
        client: &ClientId,
        claimed: Epoch,
        conn: ConnectionId,
    ) -> Option<Epoch> {
        let delivered = self.delivered.get(&conn).copied().unwrap_or(0);
        let bound = claimed.min(delivered).min(self.current_max_epoch);
        let stored = self.cursors.get(client).copied();
        let new = match stored {
            Some(s) => s.max(bound),
            None => bound,
        };
        // A 0 cursor pins nothing; tracking it would let a delivered-nothing device
        // pin the LWM at 0 (the mint→ACK→abandon DoS). Never establish/keep a 0.
        if new == 0 {
            return None;
        }
        match stored {
            // Replay / reorder / clamp: cursor did not move forward.
            Some(s) if new <= s => None,
            _ => match self.register_claim(client, new) {
                // The cursor is written at the frontier's OWN bounded value, never at the position
                // the registry echoes back: the two are the same max over the same sequence, and
                // taking the registry's would be the one direction that could raise a cursor above
                // what the connection actually delivered.
                ClaimAdmission::Honoured { .. } => {
                    self.cursors.insert(client.clone(), new);
                    Some(new)
                }
                // The claim sits below content already recorded as reclaimed, so no boundary
                // movement can serve it. Leave the client UNTRACKED: "unknown == forgotten" then
                // routes it through the existing conservative re-admission gate, which is the same
                // direction a resync fence will later formalise.
                ClaimAdmission::BelowExecuted { .. } => None,
            },
        }
    }

    /// Whether `client`'s stored cursor has regressed below `claim` (a clone /
    /// backup-restore). Read-only — NEVER rolls the stored cursor back (342a
    /// monotonicity). A regressed replica is served through the full-resync path by
    /// the caller and its ACKs stay no-ops (delivered clamp) until a genuine resync
    /// sets `delivered_conn`.
    fn is_regressed(&self, client: &ClientId, claim: Epoch) -> bool {
        self.cursors
            .get(client)
            .is_some_and(|&stored| claim < stored)
    }

    /// Rehydrate a persisted cursor for a KNOWN identity into the in-memory frontier
    /// (the reconnect/restart tracking trigger). Monotone: never lowers an existing
    /// tracked cursor. A 0 is ignored (pins nothing).
    ///
    /// This is a CLAIM SITE, not merely a cursor writer, and that is load-bearing: a rehydrated
    /// laggard must contribute a claim at its rehydrated position. Registering only on the ACK path
    /// would leave the laggard sitting in the cursor map holding the fleet MIN down while
    /// contributing no claim, so the reclamation ceiling would advance straight past the epochs it
    /// has not applied — the fleet-wide-MIN regression this module exists to prevent.
    fn rehydrate(&mut self, client: &ClientId, epoch: Epoch) {
        if epoch == 0 {
            return;
        }
        if let ClaimAdmission::BelowExecuted { .. } = self.register_claim(client, epoch) {
            // Below already-reclaimed content: leave the client untracked so the re-admission gate
            // treats it as forgotten and it takes the full-resync path.
            return;
        }
        let entry = self.cursors.entry(client.clone()).or_insert(0);
        *entry = (*entry).max(epoch);
    }

    /// Stamp the current server epoch onto a genuinely-new tombstone at the moment
    /// the server applies its `OR_REMOVE`. Server-authoritative: the epoch is
    /// derived from the op sequence this stamp advances — NEVER from the client
    /// tag's `millis`. Records the tombstone ref under its epoch and updates the
    /// max-seq index. Returns the stamped epoch (always `>= 1`: 0 is the reserved
    /// "no/uncomputable epoch" sentinel and is never stamped) together with the
    /// per-epoch ENTRY row (R2.3a), if this stamp rolled the clock past the
    /// PREVIOUS epoch.
    ///
    /// `tag.len() as u64` is computed HERE, independently of the identical
    /// computation `crdt.rs`'s `OR_REMOVE` apply site already performs for the
    /// (unrelated) tombstone-bytes gauge — this is what makes `stamped_bytes`
    /// `T2(exactness)`'s INDEPENDENT oracle rather than a value plumbed through
    /// from that other call site (R2.4(b): this contract must not touch that
    /// gauge at all, and it does not).
    fn stamp_tombstone(
        &mut self,
        map: &str,
        key: &str,
        tag: &str,
        write_seq: u64,
    ) -> (Epoch, Option<PruneEpochEntryRecord>) {
        // Captured BEFORE the op-seq/epoch update: this is the epoch the stamping
        // clock is ABOUT to leave, if this stamp turns out to roll it over. 0 before
        // the first stamp — never a real epoch, so it is excluded below.
        let prev_epoch = self.current_epoch;
        // Pre-increment BEFORE deriving the epoch so op_seq is `>= 1` here and the
        // first stamp lands in epoch 1, never epoch 0 (R3(g-i)).
        self.op_seq += 1;
        let width = self.epoch_width.max(1);
        // (op_seq - 1) / width + 1 is `>= 1` for every op_seq `>= 1` — no tombstone
        // is ever stamped 0. Advances ONE step per `width` stamped ops, in lockstep
        // with the cursor-tracked op sequence (never a timer).
        let epoch = (self.op_seq - 1) / width + 1;
        self.current_epoch = epoch;
        // Feed the ACK clamp bound with the real counter: a client can never confirm
        // past what the server has actually stamped. (Before the first stamp
        // `current_max_epoch` is the inert `u64::MAX`; now it tracks the counter.)
        self.current_max_epoch = epoch;
        self.epoch_tags
            .entry(epoch)
            .or_default()
            .push(TombstoneRef {
                map: map.to_string(),
                key: key.to_string(),
                tag: tag.to_string(),
            });
        // One ref in, one increment — the whole index-size accounting on this path.
        self.indexed_refs += 1;
        let tag_bytes = u64::try_from(tag.len()).unwrap_or(u64::MAX);
        // O-0's conservation counters, maintained at the SAME instant as `indexed_refs`
        // above so the two can never observe two different instants of the same
        // mutation (see `IndexConservationSnapshot`'s own doc).
        self.stamped_refs_total += 1;
        self.stamped_bytes_total += tag_bytes;
        // Per-epoch residency bookkeeping (R2.3a): O(1) — three integer writes into a
        // per-epoch slot this stamp already touches, never an index fold.
        let entry_slot = self
            .epoch_slots
            .entry(epoch)
            .or_insert_with(|| EpochResidencySlot {
                entered_at_op_seq: self.op_seq,
                entered_at_unix_ms: now_millis_i64(),
                ..EpochResidencySlot::default()
            });
        entry_slot.stamped_refs += 1;
        entry_slot.stamped_bytes += tag_bytes;
        // Record the durability bound for this epoch: the highest write sequence
        // the store had assigned at stamp time. The epoch is byte-durable only
        // once the store's flushed watermark reaches this value.
        let slot = self.epoch_max_seq.entry(epoch).or_insert(0);
        *slot = (*slot).max(write_seq);

        // ENTRY emission: fires once, at the moment the stamping clock rolls PAST
        // `prev_epoch` — never per stamp. `prev_epoch != 0` excludes the reserved
        // sentinel, which is never itself stamped and never rolls over.
        let entry_record = if epoch != prev_epoch && prev_epoch != 0 {
            let record = match self.epoch_slots.get(&prev_epoch) {
                Some(finished) => PruneEpochEntryRecord {
                    epoch: prev_epoch,
                    entered_index: true,
                    refs_at_entry: finished.stamped_refs,
                    stamped_refs: finished.stamped_refs,
                    stamped_bytes: finished.stamped_bytes,
                    entered_at_op_seq: finished.entered_at_op_seq,
                    entered_at_unix_ms: finished.entered_at_unix_ms,
                    rolled_over_at_op_seq: self.op_seq,
                    rolled_over_at_unix_ms: now_millis_i64(),
                    current_lwm_at_rollover: self.observed_lwm,
                    durable_watermark_at_rollover: self.durable_epoch_watermark,
                },
                // Control class (f) EMPTY-EPOCH: the clock passed through `prev_epoch`
                // with nothing ever stamped into it (only reachable via an
                // `epoch_width` change or a rebuild-induced jump, never via ordinary
                // sequential stamping). `entered_index == false` on a row that EXISTS
                // is exactly what makes this value observable at all (R2.3a).
                None => PruneEpochEntryRecord {
                    epoch: prev_epoch,
                    entered_index: false,
                    rolled_over_at_op_seq: self.op_seq,
                    rolled_over_at_unix_ms: now_millis_i64(),
                    current_lwm_at_rollover: self.observed_lwm,
                    durable_watermark_at_rollover: self.durable_epoch_watermark,
                    ..PruneEpochEntryRecord::default()
                },
            };
            // Freeze the slot's entry-side fields and mark it tracked for exit
            // detection. A NOT-APPLICABLE (control class (f)) epoch has no slot to
            // freeze — there is nothing to detect an exit for, since it never entered.
            if let Some(slot) = self.epoch_slots.get_mut(&prev_epoch) {
                slot.refs_at_entry = slot.stamped_refs;
                slot.entry_emitted = true;
            }
            self.epochs_entered_total += 1;
            Some(record)
        } else {
            None
        };

        (epoch, entry_record)
    }

    /// Epochs currently carrying tombstone refs. O(1) — `HashMap::len` is a stored
    /// count, so this is a read rather than the index fold the observation contract
    /// forbids.
    fn indexed_epochs(&self) -> u64 {
        u64::try_from(self.epoch_tags.len()).unwrap_or(u64::MAX)
    }

    /// Recompute the fleet-wide low-water-mark, refresh the cache, and report the
    /// distance it ADVANCED, if it advanced.
    ///
    /// Called from every site that mutates a cursor — an ACK advance, a forget, a
    /// rehydrate — and from nowhere else, so the cursor fold is paid once per
    /// cursor mutation instead of once per tombstone. The MIN can also FALL (a
    /// reconnecting laggard rejoins the fold), which is a legitimate movement but
    /// not an advance: the cache follows it down, and only an increase counts.
    fn refresh_low_water_mark(&mut self, now: u64) -> Option<u64> {
        let lwm = self.low_water_mark();
        let advanced = lwm.checked_sub(self.observed_lwm).filter(|&d| d > 0);
        self.observed_lwm = lwm;
        if advanced.is_some() {
            self.last_lwm_advance_millis = now;
            // An actual LWM ADVANCE is one of the two triggers the perturbation budget
            // licenses index-proportional work on (R2.4); tracked-epoch count is what
            // bounds this, not tombstone volume.
            self.refresh_epoch_licensing();
        }
        advanced
    }

    /// Seconds since the last low-water-mark advance.
    fn lwm_stall_seconds(&self, now: u64) -> u64 {
        now.saturating_sub(self.last_lwm_advance_millis) / 1000
    }

    /// Record the FIRST op-seq at which each tracked-but-unexited epoch's slot observes
    /// `low_water_mark > epoch` (R3.2's LICENSED term) or `durable_epoch_watermark >=
    /// epoch` (its FENCED term), the first time each becomes true. A no-op for a slot
    /// where both are already set. Called only from [`Self::refresh_low_water_mark`] (on
    /// an actual advance) and from [`Self::drain_prunable`] (already O(epochs) whenever
    /// the drain is not dark) — the two sites the perturbation budget already licenses
    /// index-proportional work on, so this adds no new licensed-work site.
    fn refresh_epoch_licensing(&mut self) {
        let lwm = self.observed_lwm;
        let fence = self.durable_epoch_watermark;
        let op_seq = self.op_seq;
        for (&epoch, slot) in &mut self.epoch_slots {
            if slot.lwm_passed_at_op_seq.is_none() && lwm > epoch {
                slot.lwm_passed_at_op_seq = Some(op_seq);
            }
            if slot.fence_passed_at_op_seq.is_none() && fence >= epoch {
                slot.fence_passed_at_op_seq = Some(op_seq);
            }
        }
    }

    /// Detection check (R2.3b): if `epoch`'s tracked slot has entry-emitted and is now
    /// ABSENT from `epoch_tags`, builds and returns its exit row, attributed to
    /// `attribution` if given or [`EpochExitKind::Unclassified`] if not — the reachable
    /// escape for an epoch that left the index by a path this call has no recorded cause
    /// for. A still-PRESENT epoch is left untouched (not yet an exit); an untracked or
    /// already-exited epoch (no slot) is a no-op.
    ///
    /// This is the ONE shared emission site every removal path funnels through — never
    /// three separate hooks that each independently construct an exit record — so a
    /// removal reached by a path nobody has enumerated yet still lands here and is
    /// still detected, exactly as R2.3b's doc-contract requires.
    ///
    /// `observed` is the `(refs, bytes)` pair `drain_prunable`'s `Some(refs)` arm read from
    /// the removed vector itself (K1), keyed by `epoch`, for a caller that has one; every
    /// other caller passes `None` because it never removed anything through that arm.
    fn detect_epoch_exit(
        &mut self,
        epoch: Epoch,
        attribution: Option<FinalExitKind>,
        observed: Option<(u64, u64)>,
    ) -> Option<PruneEpochResidencyRecord> {
        if self.epoch_tags.contains_key(&epoch) {
            return None;
        }
        self.finalize_epoch_exit(epoch, attribution, observed)
    }

    /// Force an exit row for every still-tracked epoch, attributed
    /// [`EpochExitKind::StillResidentAtShutdown`] regardless of current presence — the
    /// one exit kind that does NOT correspond to an observed absence, because process
    /// teardown is the only occasion this record needs to describe an epoch that never
    /// left the index at all. A `SIGKILL` teardown (the soak lineage's own teardown
    /// signal, R2.3c) never reaches this — it is reachable only on a graceful shutdown.
    fn force_shutdown_exits(&mut self) -> Vec<PruneEpochResidencyRecord> {
        let epochs: Vec<Epoch> = self.epoch_slots.keys().copied().collect();
        epochs
            .into_iter()
            .filter_map(|e| {
                // Shutdown never removed anything through `drain_prunable`'s
                // `Some(refs)` arm, so there is no observation to carry.
                self.finalize_epoch_exit(e, Some(FinalExitKind::StillResidentAtShutdown), None)
            })
            .collect()
    }

    /// Shared exit-row construction: removes `epoch`'s slot (retiring it — the ledger
    /// carries no unbounded audit trail) and builds its [`PruneEpochResidencyRecord`].
    ///
    /// `observed` is the `(refs, bytes)` pair read at the removal site inside
    /// `drain_prunable`'s `Some(refs)` arm (K1, R1.4) — the two OBSERVATION terms.
    /// Callers other than that arm's sweep pass `None`.
    fn finalize_epoch_exit(
        &mut self,
        epoch: Epoch,
        kind_hint: Option<FinalExitKind>,
        observed: Option<(u64, u64)>,
    ) -> Option<PruneEpochResidencyRecord> {
        let slot = self.epoch_slots.remove(&epoch)?;
        if !slot.entry_emitted {
            // The epoch is still current (never rolled over) — nothing to exit yet.
            // Put it back; this path is not expected to be hit by any call site today
            // but a slot must never be silently dropped.
            self.epoch_slots.insert(epoch, slot);
            return None;
        }
        let refs_at_exit = self
            .epoch_tags
            .get(&epoch)
            .map_or(0, |v| u64::try_from(v.len()).unwrap_or(u64::MAX));
        let exit_kind = match kind_hint {
            Some(FinalExitKind::DrainedByPrune) => {
                self.drained_refs_total += slot.refs_at_entry;
                EpochExitKind::DrainedByPrune
            }
            Some(FinalExitKind::ClearedByRebuild) => EpochExitKind::ClearedByRebuild,
            Some(FinalExitKind::StillResidentAtShutdown) => EpochExitKind::StillResidentAtShutdown,
            None => {
                let observed_refs_delta = i64::try_from(refs_at_exit).unwrap_or(i64::MAX)
                    - i64::try_from(slot.refs_at_entry).unwrap_or(i64::MAX);
                EpochExitKind::Unclassified {
                    observed_refs_delta,
                    observed_lwm: self.observed_lwm,
                    observed_durable_watermark: self.durable_epoch_watermark,
                    observed_current_epoch: self.current_epoch,
                    note: format!(
                        "epoch {epoch} absent from epoch_tags with no attributable \
                         removal path recorded at the detection point"
                    ),
                }
            }
        };
        // "Tombstone bytes this epoch's removal is attributed as having freed" (the
        // struct's own doc): the whole epoch's stamped content leaves the RAM index
        // atomically on a drain, so `stamped_bytes` IS the freed total; every other
        // exit kind frees nothing.
        let bytes_freed_attributed = if matches!(exit_kind, EpochExitKind::DrainedByPrune) {
            slot.stamped_bytes
        } else {
            0
        };
        // R1.5: the OBSERVATION pair. `0 / 0` for every exit kind that is not an
        // observed drain, and `0 / 0` for a `DrainedByPrune` exit whose epoch never
        // showed up in the carried map -- reachable only if a caller ever attributes
        // `DrainedByPrune` without having gone through `drain_prunable`'s `Some(refs)`
        // arm, which is this map's only writer (K1). Left as an explicit branch,
        // rather than folded behind a default, so that possibility stays visible.
        let (removed_refs_observed, removed_bytes_observed) =
            if matches!(exit_kind, EpochExitKind::DrainedByPrune) {
                observed.unwrap_or((0, 0))
            } else {
                (0, 0)
            };
        self.epochs_exited_total += 1;
        Some(PruneEpochResidencyRecord {
            epoch,
            refs_at_entry: slot.refs_at_entry,
            refs_at_exit,
            stamped_bytes: slot.stamped_bytes,
            bytes_freed_attributed,
            exit_kind,
            entered_at_op_seq: slot.entered_at_op_seq,
            entered_at_unix_ms: slot.entered_at_unix_ms,
            exited_at_op_seq: self.op_seq,
            lwm_passed_at_op_seq: slot.lwm_passed_at_op_seq,
            fence_passed_at_op_seq: slot.fence_passed_at_op_seq,
            lwm_at_exit: self.observed_lwm,
            durable_watermark_at_exit: self.durable_epoch_watermark,
            current_epoch_at_exit: self.current_epoch,
            removed_refs_observed,
            removed_bytes_observed,
        })
    }

    /// O-0's eight quantities, read as one internally-coherent tuple under the caller's
    /// lock acquisition (R3.0 limb 6). O(1) — every field is a stored counter.
    fn index_conservation_snapshot(&self) -> IndexConservationSnapshot {
        IndexConservationSnapshot {
            stamped_refs_total: self.stamped_refs_total,
            stamped_bytes_total: self.stamped_bytes_total,
            drained_refs_total: self.drained_refs_total,
            restored_refs_total: self.restored_refs_total,
            rebuild_cleared_refs_total: self.rebuild_cleared_refs_total,
            epochs_entered_total: self.epochs_entered_total,
            epochs_exited_total: self.epochs_exited_total,
            indexed_refs: self.indexed_refs,
        }
    }

    /// The byte-durability watermark: the greatest `E` such that EVERY stamped
    /// epoch `e <= E` has its recorded max write-sequence at or below `flushed`.
    /// Walks the stamped epochs in ascending order and stops at the first whose
    /// bytes are not yet durable; epochs with no entry (e.g. the empty span an
    /// `E_rec` recovery restamp leaves below the recovery epoch) hold no
    /// tombstones and are vacuously durable, so they never block the walk.
    fn compute_durable_epoch_watermark(&self, flushed: u64) -> Epoch {
        let mut keys: Vec<Epoch> = self.epoch_max_seq.keys().copied().collect();
        keys.sort_unstable();
        let mut watermark = 0;
        for e in keys {
            // Index lookup is infallible — `e` came from the key set.
            if self.epoch_max_seq.get(&e).copied().unwrap_or(u64::MAX) <= flushed {
                watermark = e;
            } else {
                break;
            }
        }
        watermark
    }

    /// Unclean-recovery rebuild (index-as-cache): drop the RAM epoch index and
    /// re-stamp EVERY live tombstone into one fresh maximally-lagging recovery
    /// epoch `e_rec`. All older epochs become empty, so nothing is prunable until
    /// every tracked client re-confirms past `e_rec`. The recovery epoch's bytes
    /// are already durable (WAL-replayed into the inner store before this runs),
    /// so its `epoch_max_seq` is 0 — the low-water-mark, not byte durability, is
    /// the operative gate.
    ///
    /// O-0's conservation identity treats a rebuild as a RESET (R3.0): the pre-rebuild
    /// `indexed_refs` is credited to `rebuild_cleared_refs_total` (a reset, not a drain)
    /// and the re-stamped `live.len()` is credited to `stamped_refs_total`, so the
    /// identity stays continuous across a recovery rather than reading as a fabricated
    /// violation. Every epoch still TRACKED for residency purposes at the moment of the
    /// clear was, by construction, resident just before it (an entry-emitted, not-yet-
    /// exited slot means its epoch had not left `epoch_tags`) — so the wholesale clear
    /// is unconditionally what caused each of their absences, and the returned exit rows
    /// are attributed [`EpochExitKind::ClearedByRebuild`] with no further disambiguation
    /// needed.
    fn rebuild_into_epoch(
        &mut self,
        e_rec: Epoch,
        live: Vec<TombstoneRef>,
    ) -> Vec<PruneEpochResidencyRecord> {
        self.rebuild_cleared_refs_total += self.indexed_refs;
        self.epoch_tags.clear();
        self.epoch_max_seq.clear();
        let width = self.epoch_width.max(1);
        self.current_epoch = e_rec;
        self.current_max_epoch = e_rec;
        // Position op_seq so the NEXT genuinely-new tombstone lands in e_rec + 1,
        // keeping every epoch below e_rec empty.
        self.op_seq = e_rec.saturating_mul(width);
        // The rebuild replaces the index wholesale, so the carried ref count is
        // re-seeded from the restamped set rather than adjusted.
        self.indexed_refs = u64::try_from(live.len()).unwrap_or(u64::MAX);
        self.stamped_refs_total += self.indexed_refs;
        if !live.is_empty() {
            self.epoch_max_seq.insert(e_rec, 0);
            self.epoch_tags.insert(e_rec, live);
        }
        // Recomputes from the fresh index on the next watermark read.
        self.durable_epoch_watermark = 0;
        // Re-baseline the advance cache against the recovered fleet position. Without
        // this the first post-recovery read subtracts a stale baseline (0 on a fresh
        // process) and reports the whole recovered low-water mark as one advance —
        // inflating the advance counters by a phantom burst that no client earned.
        self.observed_lwm = self.low_water_mark();

        // Detection point (R2.3b): every tracked epoch was resident before the clear
        // above and is unconditionally absent now, attributed to this rebuild.
        let tracked: Vec<Epoch> = self.epoch_slots.keys().copied().collect();
        tracked
            .into_iter()
            // A wholesale rebuild clear never went through `drain_prunable`'s
            // `Some(refs)` arm, so there is no per-epoch observation to carry.
            .filter_map(|e| self.detect_epoch_exit(e, Some(FinalExitKind::ClearedByRebuild), None))
            .collect()
    }

    /// Drain the tombstone refs of every currently prune-eligible epoch out of the
    /// RAM index for the caller to drop from storage, under the FULL call-site
    /// conjunction `ceiling > E && durable_epoch_watermark >= E` (with epoch 0
    /// rejected), where `ceiling` is the reclamation ceiling the sweep was licensed
    /// by — the SAME predicate [`PruneSafety::is_epoch_prune_eligible`] applies, read
    /// once from the sweep token instead of re-queried per epoch.
    /// Each ref is returned WITH its epoch so a caller whose storage drop fails
    /// can re-insert it via [`Self::restore`] — a drained-but-not-dropped tag must
    /// never lose its index entry (that would orphan it un-prunable forever).
    ///
    /// Iterates the index's ACTUAL keys — NEVER a `0..=max` range — so the reserved
    /// sentinel epoch 0 (never inserted) can never be swept even if a bound
    /// evaluated true at 0. DARK by construction: with `durable_epoch_watermark ==
    /// 0` the conjunction is false for every stamped epoch (all `>= 1`), so this
    /// returns empty in production; tests inject a watermark to exercise the drop.
    ///
    /// Returns the drained refs TOGETHER WITH the eligible/ineligible split taken
    /// **before** the removal loop consumes it. The split's eligible side is the
    /// licensed backlog — the work the prune was permitted to do at the instant it
    /// started — and this drain is UNBOUNDED: it takes every ref it just counted.
    /// A split recomputed after the loop would therefore report an eligible side of
    /// exactly 0 at every drain, on every cell, whatever the backlog had been — a
    /// series that cannot tell a prune that has caught up from one that is
    /// starving. Snapshotting before the loop is what makes the published series
    /// the quantity it is named for.
    ///
    /// Also returns every per-epoch EXIT row this pass fires (R2.3b): the detection
    /// sweep below runs over every tracked epoch (not only this pass's own removals),
    /// so an epoch that left the index by a path OTHER than this drain — including one
    /// this method has never been told about — still surfaces here, as
    /// [`EpochExitKind::Unclassified`] (`AC6a`'s reachability requirement).
    ///
    /// A non-dark pass is BRACKETED by the sweep protocol: it opens with `begin_sweep`, filters on
    /// the ceiling the returned token carries, and hands the token back to `end_sweep` with the
    /// durable watermark it OBSERVED — it proposes no boundary of its own. A refused `begin_sweep`
    /// holds no token, so it runs no pass and ends no sweep; it returns exactly what the dark path
    /// returns rather than proceeding on an unlicensed boundary.
    fn drain_prunable(
        &mut self,
    ) -> (
        Vec<(Epoch, TombstoneRef)>,
        Option<SplitObservation>,
        Vec<PruneEpochResidencyRecord>,
    ) {
        let watermark = self.durable_epoch_watermark;
        // Fast-path: a 0 watermark (no epoch byte-durable yet, or dark before the
        // recovery rebuild) means NO stamped epoch (all `>= 1`) can pass the
        // conjunction, so skip the per-epoch fold entirely — this runs on every
        // OR_REMOVE and every SYNC-leaf request. Strictly FIRST, so the dark path
        // touches the reclamation registry not at all: it takes no lock, opens no
        // sweep and reports no boundary.
        if watermark == 0 {
            return (Vec::new(), None, Vec::new());
        }
        // A second sweep cannot be in flight in this tree — the whole pass runs inside one
        // acquisition of the frontier lock — but a consumer without that lock could try, and it
        // must get a pass that does not run rather than one licensed by a boundary it never
        // obtained.
        let Some(token) = self.registry.begin_sweep() else {
            return (Vec::new(), None, Vec::new());
        };
        // Hoisted once: the eligibility filter, the split and the whole pass see ONE ceiling, and
        // it is the licence the token carries rather than a fresh query whose answer can differ.
        let ceiling = token.ceiling();
        // One of the two sites the perturbation budget licenses index-proportional
        // work on: this fold already runs whenever the drain is not dark.
        self.refresh_epoch_licensing();
        let eligible: Vec<Epoch> = self
            .epoch_tags
            .keys()
            .copied()
            // Cheap watermark conjunct first so it short-circuits the rest. Epoch 0 is rejected
            // here for the same belt-and-suspenders reason the trait predicate rejects it.
            .filter(|&e| watermark >= e && e != 0 && ceiling > e)
            .collect();
        // Gated on a non-empty eligible set, so the per-remove path — where the
        // drain finds nothing — still pays no index-proportional fold. This is the
        // same budget the post-loop recompute honoured; only the INSTANT moves.
        let pre_drain_split =
            (!eligible.is_empty()).then(|| self.split_observation(self.low_water_mark(), ceiling));
        let mut drained = Vec::new();
        let mut drained_epochs: HashSet<Epoch> = HashSet::new();
        // K1 / R1.4: the OBSERVATION terms, keyed by epoch. Read from the vector the
        // index removal itself returned, inside this SAME `Some(refs)` arm and
        // strictly before `drained.extend(...)` consumes it below — this is the ONLY
        // writer of this map, and it is never re-read from `epoch_tags` afterward
        // (which no longer holds the epoch at that point).
        let mut removed_observed: HashMap<Epoch, (u64, u64)> = HashMap::new();
        for e in eligible {
            if let Some(refs) = self.epoch_tags.remove(&e) {
                // Decrement by what this epoch actually held, so the carried count
                // stays exact without ever re-reading the rest of the index.
                self.indexed_refs = self
                    .indexed_refs
                    .saturating_sub(u64::try_from(refs.len()).unwrap_or(u64::MAX));
                self.last_drained_epoch = self.last_drained_epoch.max(e);
                drained_epochs.insert(e);
                // R1.3: `Σ tag.len()` — the same byte quantity `stamp_tombstone`
                // credits per ref (`:482`), so the two sides are comparable with no
                // unit conversion.
                let removed_refs = u64::try_from(refs.len()).unwrap_or(u64::MAX);
                let removed_bytes: u64 = refs
                    .iter()
                    .map(|r| u64::try_from(r.tag.len()).unwrap_or(u64::MAX))
                    .sum();
                removed_observed.insert(e, (removed_refs, removed_bytes));
                drained.extend(refs.into_iter().map(|r| (e, r)));
            }
            self.epoch_max_seq.remove(&e);
        }

        // Detection point (R2.3b): sweep EVERY tracked epoch, not just this pass's own
        // removals. An epoch this pass just drained is attributed accordingly; any
        // OTHER tracked epoch found absent here was NOT removed by this call, so it
        // gets no hint — Unclassified, by construction, is what an absence with no
        // recorded cause looks like.
        let tracked: Vec<Epoch> = self.epoch_slots.keys().copied().collect();
        let mut exits = Vec::new();
        for e in tracked {
            let attribution = drained_epochs
                .contains(&e)
                .then_some(FinalExitKind::DrainedByPrune);
            let observed = removed_observed.get(&e).copied();
            if let Some(record) = self.detect_epoch_exit(e, attribution, observed) {
                exits.push(record);
            }
        }

        // Close the bracket on the ONLY exit path that holds a token, handing back the SAME
        // watermark local the filter applied rather than re-reading the field — the two are equal
        // while the whole pass runs under one frontier-lock acquisition, and they diverge the
        // moment it does not, at which point a re-read would fence claimants above what this pass
        // could actually have reclaimed. The registry derives the boundary from the token's
        // ceiling and this observed watermark; nothing here proposes one.
        self.registry.end_sweep(token, watermark);

        (drained, pre_drain_split, exits)
    }

    /// Re-insert a drained tombstone ref whose storage drop FAILED, so the tag is
    /// retried on a later sweep instead of being orphaned un-prunable in storage.
    /// The `epoch_max_seq` entry is re-created best-effort (the index is a pure
    /// RAM cache — the unclean-recovery rebuild is the authoritative
    /// recovery for any imprecision here).
    fn restore(&mut self, epoch: Epoch, tombstone_ref: TombstoneRef) {
        self.restored_refs_total += 1;
        self.epoch_tags
            .entry(epoch)
            .or_default()
            .push(tombstone_ref);
        // The drain already decremented this ref; putting it back re-adds it, so a
        // drained-then-restored ref is never double-counted in either direction.
        self.indexed_refs += 1;
        self.epoch_max_seq.entry(epoch).or_insert(0);
    }

    /// Snapshot of the frontier state the record publishes as last-value gauges.
    ///
    /// Every field is a stored value: the ref count is carried, the epoch count is a
    /// `HashMap::len`, and the low-water-mark comes from the cursor-path cache. The
    /// snapshot is therefore O(1) and safe on the per-remove path.
    fn observation_snapshot(&self) -> FrontierObservation {
        FrontierObservation {
            indexed_refs: self.indexed_refs,
            indexed_epochs: self.indexed_epochs(),
            current_epoch: self.current_epoch,
            low_water_mark: self.observed_lwm,
            durable_epoch_watermark: self.durable_epoch_watermark,
            last_drained_epoch: self.last_drained_epoch,
        }
    }

    /// The eligible / ineligible split of the indexed corpus, plus the claim span
    /// and per-claim lags observed at the same instant.
    ///
    /// O(indexed epochs + tracked claims), which the perturbation budget permits
    /// ONLY on the low-water-mark-movement path and on non-empty drains — never per
    /// `OR_REMOVE`. Both boundaries are hoisted out of the epoch loop: calling
    /// `is_epoch_prune_eligible` per epoch would re-query the reclamation ceiling once
    /// per epoch — an OBSERVING call that counts every query — for an identical answer.
    /// `ceiling` and `lwm` are therefore passed in rather than re-read here. A drain
    /// passes the ceiling its own sweep token carries, so the split is computed under
    /// exactly the boundary that pass was licensed by rather than under a fresher one;
    /// the cursor-movement callers have just folded `lwm` in `refresh_low_water_mark`,
    /// and folding it again under the same lock would lengthen the hold in the
    /// cursor-count dimension for an identical answer.
    fn split_observation(&self, lwm: Epoch, ceiling: Epoch) -> SplitObservation {
        let watermark = self.durable_epoch_watermark;
        let current_epoch = self.current_epoch;
        let mut eligible_refs = 0u64;
        let mut ineligible_refs = 0u64;
        for (&epoch, refs) in &self.epoch_tags {
            let held = u64::try_from(refs.len()).unwrap_or(u64::MAX);
            // The FULL call-site conjunction the drain applies, with epoch 0 rejected
            // for the same belt-and-suspenders reason: a split computed under a weaker
            // predicate than the drain's would report refs as eligible that no pass
            // would ever take. The claim-span record below keeps carrying the LWM, which
            // is a different quantity and is not what licenses reclamation.
            if epoch != 0 && watermark >= epoch && ceiling > epoch {
                eligible_refs += held;
            } else {
                ineligible_refs += held;
            }
        }
        let claim_lags: Vec<Epoch> = self
            .cursors
            .values()
            .map(|&cursor| current_epoch.saturating_sub(cursor))
            .collect();
        SplitObservation {
            eligible_refs,
            ineligible_refs,
            computed_at_epoch: current_epoch,
            claim_span: PruneClaimSpanRecord {
                current_epoch,
                low_water_mark: lwm,
                span_epochs: current_epoch.saturating_sub(lwm),
                tracked_claims: u64::try_from(self.cursors.len()).unwrap_or(u64::MAX),
            },
            claim_lags,
        }
    }
}

/// The frontier state published as last-value gauges at an observation point.
///
/// Gathered under the state lock and emitted after it drops, so no observation call
/// ever runs while the frontier is held.
#[derive(Debug, Clone, Copy)]
struct FrontierObservation {
    indexed_refs: u64,
    indexed_epochs: u64,
    current_epoch: Epoch,
    low_water_mark: Epoch,
    durable_epoch_watermark: Epoch,
    last_drained_epoch: Epoch,
}

/// The eligible / ineligible split with its staleness marker, plus the claim span
/// captured at the same instant.
///
/// The split is only ever recomputed on the events that STOP happening during a
/// scheduling or low-water-mark stall, which is why `computed_at_epoch` travels with
/// it: a reader that cannot tell a fresh sample from one frozen at its last recompute
/// would read a stalled split as a fresh "not growing" exactly when it is least
/// entitled to.
///
/// The per-claim lags are a `Vec` here and NOT a field of the record struct: the
/// record stays allocation-free and copyable, and this vector is built only on the
/// two paths the budget already licenses index-proportional work on.
#[derive(Debug, Clone)]
struct SplitObservation {
    eligible_refs: u64,
    ineligible_refs: u64,
    computed_at_epoch: Epoch,
    claim_span: PruneClaimSpanRecord,
    claim_lags: Vec<Epoch>,
}

impl PruneSafety for FrontierState {
    fn is_epoch_prune_eligible(&self, epoch: Epoch) -> bool {
        // Fold over the reclamation ceiling ONLY. The durability fence is the
        // CALL-SITE second conjunct (`drain_prunable`), NEVER here. Epoch 0 is
        // the reserved "no/uncomputable epoch" sentinel — reject it at the trait
        // level too (belt-and-suspenders per R3(g)) so a future consumer that
        // bypasses the call-site conjunction cannot prune the sentinel.
        if epoch == 0 {
            return false;
        }
        // STRICT `>` per the 342a contract ("advanced PAST epoch"): the conveyed
        // covering epoch is `current_epoch`, which may still be ACCUMULATING new
        // tombstones (width > 1) — a cursor AT epoch E therefore proves delivery
        // complete only through E-1. Inclusive `>=` would let a still-open epoch be
        // pruned after a new tombstone lands in it post-ACK. Pruning N requires the
        // ceiling >= N+1, i.e. every claimant in the fold applied all of N.
        self.reclamation_ceiling() > epoch
    }

    fn gate_decision_holds_at_commit(&self, token: GateToken) -> bool {
        // Lag-aware commit-time re-check: the gate-time not-forgotten decision still
        // holds only if the client is STILL not forgotten NOW — i.e. still tracked AND
        // not lagged past the forget threshold. Using the lag-aware `is_forgotten`
        // (not the bare `is_tracked`) closes BOTH TOCTOU surfaces: an active
        // `forget_client` eviction (untracked → forgotten) AND a client crossing the
        // cursor-lag-K threshold between gate and commit because a concurrent stamp
        // advanced `current_epoch`. The stock `is_tracked`-only check caught only the
        // first; the second is the live surface in this child (prune is dark).
        !self.is_forgotten(&token.client)
    }
}

impl CausalFrontier for FrontierState {
    fn confirm_apply(&mut self, client: &ClientId, epoch: Epoch) {
        // Raw monotone-max insert (the 342a contract). The bounded ACK rule lives in
        // `advance_on_ack`; this is the underlying monotone primitive it and
        // rehydration share. A 0 is never tracked.
        if epoch == 0 {
            return;
        }
        // Gated on the reclamation boundary like every other cursor writer, even though nothing in
        // the tree calls this primitive today: a later wiring that reached for it must not be able
        // to establish a cursor the boundary never admitted.
        if let ClaimAdmission::BelowExecuted { .. } = self.register_claim(client, epoch) {
            return;
        }
        let entry = self.cursors.entry(client.clone()).or_insert(0);
        *entry = (*entry).max(epoch);
    }

    fn low_water_mark(&self) -> Epoch {
        // MIN across ALL tracked clients — a single lagging device pins the whole
        // fleet. Vacuous case (no tracked clients): 0, i.e. prune NOTHING. This is the
        // loss-conservative direction the whole protocol rests on. Rehydration is lazy
        // — a KNOWN client is only re-tracked when it reconnects — so an empty in-memory
        // frontier does NOT mean "no client needs protection": post-restart it means
        // "no client has reconnected yet." Returning the current max epoch here would
        // license the prune (342b) to drop tombstones a not-yet-reconnected laggard has
        // not applied → resurrection on that honest device. Only a genuinely tracked
        // client's cursor may ever raise the LWM above 0.
        self.cursors.values().copied().min().unwrap_or(0)
    }

    fn is_tracked(&self, client: &ClientId) -> bool {
        self.cursors.contains_key(client)
    }

    fn forget_client(&mut self, client: &ClientId) {
        self.cursors.remove(client);
        // Release in the same act that drops the cursor: an explicit release is one of only two
        // ways a claim ever leaves the fold, so a forget that dropped the cursor but kept the claim
        // would pin the ceiling on a client nothing is tracking any more.
        self.registry.release_claim(client, ClaimScope::Global);
    }
}

/// Thread-safe durable per-device causal frontier.
///
/// Wraps [`FrontierState`] behind a `Mutex` for shared `&self` access from the
/// concurrent websocket read loops, plus an optional [`MapDataStore`] for
/// best-effort redb persistence. Held in `AppState` as an `Arc`; consumed later by
/// 342b (`low_water_mark`) and 342c (gate / forget / `set_delivered`).
pub struct TombstoneFrontier {
    state: Mutex<FrontierState>,
    /// Best-effort persistence backend. `None` in tests that exercise only the
    /// in-memory advance logic; persistence then no-ops (cursor-loss is safe).
    store: Option<Arc<dyn MapDataStore>>,
    /// Sender to the single background cursor-persistence worker. Cursor durability
    /// (advance-persist + forget-delete) is offloaded here so the per-connection ACK
    /// read loop never awaits a redb write. A single FIFO consumer serializes all
    /// durability ops per client, which is what makes the persisted cursor monotone
    /// (a stale racing advance is dropped by the worker's high-water check) AND keeps
    /// a `forget` delete ordered strictly after every prior advance for that client
    /// (no resurrection). `None` when there is no store (durability no-ops).
    persist_tx: Mutex<Option<mpsc::UnboundedSender<PersistMsg>>>,
    /// Join handle for the worker, so `shutdown` can await its exit (releasing the
    /// store `Arc` it holds — required before a redb file can be reopened).
    persist_worker: Mutex<Option<JoinHandle<()>>>,
    /// Sink for the prune record, chosen once at construction from the arming
    /// kill-switch. Held as a trait object so the disarmed path is a null
    /// implementation rather than a branch at every observation call site.
    prune_observer: Box<dyn PruneRecordObserver>,
    /// The reclamation boundary the prune side folds over, shared with the guarded
    /// [`FrontierState`] so the claim sites and the ceiling read see one authority. Held here
    /// as well so it is readable without taking the frontier lock, and so the lock order stays
    /// one-way: frontier lock, then registry lock, never the reverse.
    registry: Arc<ReclamationRegistry>,
}

/// A unit of work for the background cursor-persistence worker.
enum PersistMsg {
    /// Persist `client`'s advanced cursor (monotone: dropped if not above the
    /// worker's high-water for that client).
    Advance { client: ClientId, epoch: Epoch },
    /// Delete `client`'s durable cursor row. FIFO ordering guarantees this runs
    /// after every prior `Advance` for the client; `done` reports the durable-delete
    /// OUTCOME (`true` = row removed, `false` = the store delete failed) so the caller
    /// can fall back to a direct retry when the worker's own delete did not land.
    Forget {
        client: ClientId,
        done: oneshot::Sender<bool>,
    },
    /// Drain barrier: signals `done` once every message enqueued before it has been
    /// processed. Lets a caller await outstanding persists without stopping the worker.
    Barrier { done: oneshot::Sender<()> },
}

impl std::fmt::Debug for TombstoneFrontier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let tracked = self.state.lock().map(|s| s.cursors.len()).unwrap_or(0);
        f.debug_struct("TombstoneFrontier")
            .field("tracked_clients", &tracked)
            .field("has_store", &self.store.is_some())
            .finish_non_exhaustive()
    }
}

/// Recover a poisoned mutex guard instead of propagating the panic, logging the
/// recovery so the underlying panic is not silent. Safe for every mutex this frontier
/// holds: they guard best-effort state (a stale-but-consistent frontier snapshot, or an
/// `Option` handle), and cursor-loss only degrades to a resync — but a poisoned lock
/// still signals a prior panic worth surfacing.
fn recover_poisoned<T>(guard: &'static str) -> impl FnOnce(std::sync::PoisonError<T>) -> T {
    move |poison| {
        warn!(
            guard,
            "recovered a poisoned tombstone-frontier mutex (a prior holder panicked)"
        );
        poison.into_inner()
    }
}

impl TombstoneFrontier {
    /// Build a frontier over an optional persistence backend.
    ///
    /// When a store is present a single background persistence worker is spawned;
    /// this must therefore be called from within a tokio runtime (it always is —
    /// server init and every `#[tokio::test]` provide one).
    ///
    /// # Construction-order precondition — the metrics recorder MUST already be installed
    ///
    /// This constructor resolves and caches the prune record's metric handles (see
    /// [`MetricsPruneRecorder::new`]). In `metrics` 0.24 a handle resolved **before** a
    /// recorder is installed binds to a no-op for that handle's entire lifetime and never
    /// re-resolves, so a frontier built ahead of observability initialisation would record
    /// nothing forever while still looking armed. Every production construction site must
    /// therefore run after the Prometheus recorder is installed. Both of them do today, and
    /// the property is a precondition each site owns, not one this constructor can enforce:
    ///
    /// - `bin/topgun_server.rs` builds the frontier on the server boot path, after the same
    ///   path has already called `init_observability()` (which installs the recorder in
    ///   `service/middleware/observability.rs`).
    /// - `network/module.rs`'s `build_app` installs nothing itself; it receives an
    ///   already-constructed observability handle through `AppServices`, so by the time it
    ///   constructs the frontier the recorder is necessarily installed.
    ///
    /// Any new construction site inherits the same obligation, and any test that constructs
    /// one and then asserts on a render MUST bind a recorder first — otherwise a test-order
    /// inversion masks exactly the hazard this contract exists to name.
    #[must_use]
    pub fn new(store: Option<Arc<dyn MapDataStore>>) -> Self {
        // Spawn the persistence worker only when a store is wired AND we are inside
        // a tokio runtime. Every production construction is on the async server
        // boot path (`serve` / `#[tokio::main]`), so the worker is always spawned
        // there; the runtime guard only covers a synchronous non-runtime unit test
        // that builds the router without ever exercising cursor persistence — it
        // must not panic in `tokio::spawn`.
        let (persist_tx, persist_worker) = match store.as_ref() {
            Some(store) if tokio::runtime::Handle::try_current().is_ok() => {
                // Unbounded on purpose: the messages are tiny (client id + epoch), the
                // single worker drains them fast (one buffered store write each), and
                // the `Forget`/`Barrier` variants carry oneshots that must never be
                // dropped — so a bounded channel could not simply drop-on-full. A
                // best-effort cursor persist whose loss only costs a resync does not
                // justify back-pressuring the ACK read loop. Bounding it under a real
                // sustained-burst OOM budget is a post-soak hardening follow-up.
                let (tx, rx) = mpsc::unbounded_channel();
                let handle = tokio::spawn(cursor_persist_worker(Arc::clone(store), rx));
                (Mutex::new(Some(tx)), Mutex::new(Some(handle)))
            }
            _ => (Mutex::new(None), Mutex::new(None)),
        };
        // The arming kill-switch is read HERE and nowhere else, and exactly once per
        // frontier: reading it again per pass would let the arming gate and the recording
        // branch observe two different answers for the same operation.
        let prune_observer: Box<dyn PruneRecordObserver> = match prune_record_arming_from_env() {
            PruneRecordArming::Armed => Box::new(MetricsPruneRecorder::new()),
            PruneRecordArming::Disarmed => Box::new(NullPruneRecorder),
        };
        // Boot floor 0, never a head: no durable prune checkpoint exists in this tree, and seeding
        // the floor from any recovered high-water would license reclamation up to nearly head right
        // after a recovery — the one direction that resurrects. A consumer that starts persisting a
        // checkpoint passes it here instead. The registry reads its margin from the environment
        // once, at this construction, so the parse and the arithmetic cannot disagree later.
        let registry = Arc::new(ReclamationRegistry::new(0));
        Self {
            state: Mutex::new(FrontierState::new(Arc::clone(&registry))),
            store,
            persist_tx,
            persist_worker,
            prune_observer,
            registry,
        }
    }

    /// The reclamation boundary this frontier registers its claims on and folds its prune
    /// eligibility over.
    ///
    /// Exposed so a consumer can read the boundary — the ceiling, the executed watermark, the live
    /// claim count — without taking the frontier lock. It is the SINGLE authority: a consumer that
    /// computed a second boundary of its own would be free to disagree with the one the drain
    /// actually applies.
    #[must_use]
    pub fn reclamation(&self) -> &Arc<ReclamationRegistry> {
        &self.registry
    }

    /// The frontier's prune-record sink.
    ///
    /// The prune loop lives in the CRDT domain service and reaches the observer through the
    /// frontier it already borrows, so the ledger needs no second wiring path and cannot end
    /// up observing a different arming decision than the frontier made at construction.
    #[must_use]
    pub fn prune_observer(&self) -> &dyn PruneRecordObserver {
        self.prune_observer.as_ref()
    }

    /// Publish the frontier's index and epoch state.
    ///
    /// Both calls are a fixed handful of gauge stores over already-resolved handles,
    /// which is what keeps this affordable on the per-`OR_REMOVE` stamp path. The
    /// snapshot is taken under the lock by the caller and this runs after it drops.
    fn publish_frontier_state(&self, snapshot: &FrontierObservation) {
        let observer = self.prune_observer.as_ref();
        observer.observe_index_state(snapshot.indexed_refs, snapshot.indexed_epochs);
        observer.observe_epoch_state(
            snapshot.current_epoch,
            snapshot.low_water_mark,
            snapshot.durable_epoch_watermark,
            snapshot.last_drained_epoch,
        );
    }

    /// Publish the eligible / ineligible split with its staleness marker, and the
    /// claim span captured at the same instant.
    ///
    /// The recompute counter behind `observe_eligibility_split` is what makes a
    /// stale split detectable, so this is called on every recompute and on nothing
    /// else: a split published without a recompute would inflate the counter and
    /// present a frozen sample as a fresh one.
    fn publish_split(&self, split: &SplitObservation) {
        let observer = self.prune_observer.as_ref();
        observer.observe_eligibility_split(
            split.eligible_refs,
            split.ineligible_refs,
            split.computed_at_epoch,
        );
        observer.observe_claim_span(&split.claim_span, &split.claim_lags);
    }

    /// Publish a low-water-mark advance together with the split it triggered.
    fn publish_lwm_movement(&self, epochs_advanced: u64, split: &SplitObservation) {
        self.prune_observer.observe_lwm_advance(epochs_advanced);
        self.publish_split(split);
    }

    /// Publish an epoch's ENTRY row (R2.3a): the observer call (feeds the metrics-side
    /// completeness witness) and the structured `tracing::info!` line on the dedicated
    /// `topgun_server::tombstone_frontier::residency` target (R2.2) — never a blanket
    /// `info`, per `spec356-prune.sh:697-699`'s measured cost finding. Both calls run
    /// AFTER the frontier lock has dropped, exactly like every other publish helper here.
    fn publish_epoch_entry(&self, record: &PruneEpochEntryRecord) {
        self.prune_observer.observe_epoch_entry(record);
        info!(
            target: "topgun_server::tombstone_frontier::residency",
            kind = "epoch_entry",
            epoch = record.epoch,
            entered_index = record.entered_index,
            refs_at_entry = record.refs_at_entry,
            stamped_refs = record.stamped_refs,
            stamped_bytes = record.stamped_bytes,
            entered_at_op_seq = record.entered_at_op_seq,
            entered_at_unix_ms = record.entered_at_unix_ms,
            rolled_over_at_op_seq = record.rolled_over_at_op_seq,
            rolled_over_at_unix_ms = record.rolled_over_at_unix_ms,
            current_lwm_at_rollover = record.current_lwm_at_rollover,
            durable_watermark_at_rollover = record.durable_watermark_at_rollover,
            "prune epoch entry"
        );
    }

    /// Publish an epoch's EXIT row (R2.3b) — see [`Self::publish_epoch_entry`] for the
    /// shared rationale. `exit_kind` is rendered with `{:?}` so `Unclassified`'s raw
    /// context (R2.3c) is captured verbatim in the line rather than collapsed to a bare
    /// variant name.
    fn publish_epoch_exit(&self, record: &PruneEpochResidencyRecord) {
        self.prune_observer.observe_epoch_residency(record);
        info!(
            target: "topgun_server::tombstone_frontier::residency",
            kind = "epoch_exit",
            epoch = record.epoch,
            refs_at_entry = record.refs_at_entry,
            refs_at_exit = record.refs_at_exit,
            stamped_bytes = record.stamped_bytes,
            bytes_freed_attributed = record.bytes_freed_attributed,
            exit_kind = ?record.exit_kind,
            entered_at_op_seq = record.entered_at_op_seq,
            entered_at_unix_ms = record.entered_at_unix_ms,
            exited_at_op_seq = record.exited_at_op_seq,
            lwm_passed_at_op_seq = ?record.lwm_passed_at_op_seq,
            fence_passed_at_op_seq = ?record.fence_passed_at_op_seq,
            lwm_at_exit = record.lwm_at_exit,
            durable_watermark_at_exit = record.durable_watermark_at_exit,
            current_epoch_at_exit = record.current_epoch_at_exit,
            removed_refs_observed = record.removed_refs_observed,
            removed_bytes_observed = record.removed_bytes_observed,
            "prune epoch exit"
        );
    }

    /// O-0's eight-quantity conservation snapshot (R3.0 limb 6), read as one
    /// internally-coherent tuple under a single lock acquisition. This is the accessor
    /// the double-read sampling rule's two `/metrics`-tick renders are taken from.
    #[must_use]
    pub fn index_conservation_snapshot(&self) -> IndexConservationSnapshot {
        self.lock().index_conservation_snapshot()
    }

    /// Enqueue a durability message onto the worker if one is wired. Returns
    /// `false` if there is no worker (no store, or already shut down).
    fn enqueue_persist(&self, msg: PersistMsg) -> bool {
        let guard = self
            .persist_tx
            .lock()
            .unwrap_or_else(recover_poisoned("persist_tx"));
        match guard.as_ref() {
            Some(tx) => tx.send(msg).is_ok(),
            None => false,
        }
    }

    /// Enqueue a raw `Advance` directly, bypassing the in-memory monotone advance.
    /// Test-only: lets a test deliver advances to the worker OUT OF ORDER (as a
    /// displaced-owner race would) to prove the worker's high-water drops a stale
    /// lower epoch.
    #[cfg(test)]
    fn enqueue_advance_for_test(&self, client: &ClientId, epoch: Epoch) {
        self.enqueue_persist(PersistMsg::Advance {
            client: client.clone(),
            epoch,
        });
    }

    /// Await every cursor-durability message enqueued so far (advances + forgets)
    /// without stopping the worker. Used by tests/consumers that read the durable
    /// cursor state right after an ACK, and anywhere a durability checkpoint is
    /// needed before reading the store back.
    pub async fn quiesce_persists(&self) {
        let rx = {
            let (done_tx, done_rx) = oneshot::channel();
            if self.enqueue_persist(PersistMsg::Barrier { done: done_tx }) {
                Some(done_rx)
            } else {
                None
            }
        };
        if let Some(rx) = rx {
            // A dropped sender (worker gone) resolves to Err — never hang.
            let _ = rx.await;
        }
    }

    /// Stop the background persistence worker and await its exit. Closing the
    /// channel drains any buffered messages first (tokio mpsc delivers them before
    /// `recv` returns `None`), so this is also a full flush; awaiting the handle
    /// then releases the store `Arc` the worker holds (required before reopening a
    /// redb file). Idempotent.
    pub async fn shutdown(&self) {
        // The one occasion `StillResidentAtShutdown` fires (R2.3c): every epoch still
        // tracked (entry-emitted, exit pending) at a GRACEFUL shutdown gets its exit
        // row here, describing an epoch that never left the index at all. A `SIGKILL`
        // teardown — this lineage's own soak-cell teardown signal — never reaches this
        // method, so it emits nothing there, exactly as R2.3c states.
        let exits = self.lock().force_shutdown_exits();
        for record in &exits {
            self.publish_epoch_exit(record);
        }
        // Drop the sender to close the channel so the worker's recv loop ends.
        // Recover from a poisoned lock rather than panic — a poisoned mutex only
        // means a prior panic while holding it; the Option is still consistent.
        let _ = self
            .persist_tx
            .lock()
            .unwrap_or_else(recover_poisoned("persist_tx"))
            .take();
        let handle = self
            .persist_worker
            .lock()
            .unwrap_or_else(recover_poisoned("persist_worker"))
            .take();
        if let Some(handle) = handle {
            let _ = handle.await;
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, FrontierState> {
        // A poisoned frontier mutex means a prior panic while holding it; recover the
        // guard rather than propagate the panic — the frontier is best-effort and a
        // stale-but-consistent snapshot is safe (cursor-loss degrades to resync).
        self.state.lock().unwrap_or_else(recover_poisoned("state"))
    }

    /// Inject the server's current max stamped epoch (the global bound). Settable so
    /// the global-bound rejection test is unit-testable before 342b's counter lands.
    pub fn set_current_max_epoch(&self, epoch: Epoch) {
        self.lock().current_max_epoch = epoch;
    }

    /// Record the highest epoch DELIVERED on `conn` (monotone). Injectable — fed by
    /// the delta-delivery path and by 342c's full-resync snapshot completion; at
    /// Wave 2 only tests set it. Loss is conservative (suppresses advances only).
    pub fn set_delivered(&self, conn: ConnectionId, epoch: Epoch) {
        let mut state = self.lock();
        let entry = state.delivered.entry(conn).or_insert(0);
        *entry = (*entry).max(epoch);
    }

    /// The highest epoch delivered on `conn` (0 if none). Test/introspection helper.
    #[must_use]
    pub fn delivered(&self, conn: ConnectionId) -> Epoch {
        self.lock().delivered.get(&conn).copied().unwrap_or(0)
    }

    /// Reset `conn`'s delivered watermark to 0 — the NOT-YET-ADMITTED signal.
    ///
    /// Called when a sync-init routes the connection through the gated
    /// full-snapshot REPLACE path: a REUSED connection may carry `delivered > 0`
    /// from an earlier healthy round on the same socket, which would let the
    /// continuation/push gates (which key on `delivered == 0`) treat a
    /// now-gated client as already admitted mid-resync. Resetting is strictly
    /// conservative: it can only suppress ACK admission until the REPLACE
    /// snapshot completes and a fresh `CLIENT_APPLY_ACK` re-admits — never
    /// widen it.
    pub fn reset_delivered(&self, conn: ConnectionId) {
        self.lock().delivered.insert(conn, 0);
    }

    /// Confirmed-apply ACK: advance `client`'s cursor under the bounded monotone rule
    /// for an ACK arriving on connection `conn`, persisting the new value best-effort.
    /// Returns `true` iff the stored cursor advanced.
    ///
    /// The caller MUST have already verified the ACK came from the current owner of
    /// `client` (connection-ownership fencing) — this method does not re-check
    /// ownership (it has no registry handle), only the delivered/global bounds.
    // Kept `async` deliberately: the durable persist that used to be awaited here is
    // now offloaded to the background worker (non-blocking enqueue), so the body no
    // longer awaits — but this is a stable public API awaited by the websocket ACK
    // read loop and by sim/domain tests across other files. Dropping `async` would
    // ripple `.await` removals through every caller for no behavioural gain.
    #[allow(clippy::unused_async)]
    pub async fn confirm_apply_ack(
        &self,
        client: &ClientId,
        claimed: Epoch,
        conn: ConnectionId,
    ) -> bool {
        // Compute + apply the advance AND enqueue the offloaded persist while STILL
        // holding the state lock. The durable write itself is offloaded (the ACK read
        // loop never awaits redb — that stays off the hot path), but the enqueue must
        // be under the lock so the worker observes advances and forgets in the SAME
        // order as their in-memory effects. If the enqueue happened after the lock
        // dropped, a concurrent `forget_client` could interleave and enqueue its
        // delete BEFORE this advance, letting the worker delete then re-persist and
        // resurrect a forgotten cursor. `enqueue_persist` is a non-blocking channel
        // send, so holding the std Mutex across it is safe (no await under the lock).
        // Read the clock BEFORE the lock: it feeds the stall gauge, and a syscall
        // under the frontier lock would lengthen the hold for every concurrent ACK.
        let now = now_millis();
        let (advanced, movement, stall_seconds) = {
            let mut state = self.lock();
            let advanced = state.advance_on_ack(client, claimed, conn);
            if let Some(epoch) = advanced {
                if !self.enqueue_persist(PersistMsg::Advance {
                    client: client.clone(),
                    epoch,
                }) {
                    // Distinguish the two no-worker cases: with no durable store the
                    // miss is expected (memory-only mode, debug), but with a store
                    // configured it means the worker already shut down while ACKs are
                    // still arriving — every such advance is non-durable and will be
                    // re-earned by resync after restart, which operators should see.
                    if self.store.is_some() {
                        warn!(client = %client, epoch, "cursor advance not persisted (persistence worker stopped)");
                    } else {
                        debug!(client = %client, epoch, "cursor advance not enqueued (no durable store)");
                    }
                }
            }
            // Only a cursor that actually moved can move the fleet-wide MIN, so a
            // replayed / clamped / delivered-bounded ACK pays no fold at all here.
            let movement = advanced
                .and_then(|_| state.refresh_low_water_mark(now))
                .map(|epochs_advanced| {
                    // Read the ceiling ONCE for this observation: it is an observing call, so a
                    // second read would report the split's own arithmetic as query volume.
                    let ceiling = state.reclamation_ceiling();
                    (
                        epochs_advanced,
                        state.split_observation(state.observed_lwm, ceiling),
                    )
                });
            // Refreshed on EVERY ack, advance or not: the regime worth seeing is the
            // one where the low-water-mark is not moving, and a stall gauge that only
            // ticked on an advance would freeze exactly when it starts to matter.
            let stall_seconds = state.lwm_stall_seconds(now);
            (advanced, movement, stall_seconds)
        };
        self.prune_observer.observe_lwm_stall(stall_seconds);
        if let Some((epochs_advanced, split)) = movement {
            self.publish_lwm_movement(epochs_advanced, &split);
        }
        advanced.is_some()
    }

    /// Whether `client` has regressed below `claim` reported at sync-initiation
    /// (clone / backup-restore). Never rolls the stored cursor back. The caller
    /// routes a regressed replica through the full-resync path; its ACKs remain
    /// no-ops (delivered clamp) until a genuine resync sets `delivered_conn`.
    #[must_use]
    pub fn is_regressed(&self, client: &ClientId, claim: Epoch) -> bool {
        self.lock().is_regressed(client, claim)
    }

    /// Rehydrate a KNOWN identity's persisted cursor into the frontier BEFORE any
    /// ACK, on connection establishment. Loads from the durable store and tracks the
    /// cursor (monotone). A freshly-minted identity has no persisted cursor and is
    /// correctly left untracked (unknown → gated). No-op if no store is wired.
    pub async fn rehydrate(&self, client: &ClientId) {
        let Some(store) = self.store.as_ref() else {
            return;
        };
        match load_cursor(store.as_ref(), client).await {
            Ok(Some(epoch)) => {
                let now = now_millis();
                // A rehydrate is a cursor mutation, so it is one of the three sites
                // that can move the low-water-mark. Usually it moves it DOWN (a
                // laggard rejoins the fold) and reports no advance; refreshing here
                // anyway is what keeps the cached MIN from going stale and making a
                // later ACK report an advance this reconnect actually caused.
                let movement = {
                    let mut state = self.lock();
                    state.rehydrate(client, epoch);
                    state.refresh_low_water_mark(now).map(|epochs_advanced| {
                        let ceiling = state.reclamation_ceiling();
                        (
                            epochs_advanced,
                            state.split_observation(state.observed_lwm, ceiling),
                        )
                    })
                };
                if let Some((epochs_advanced, split)) = movement {
                    self.publish_lwm_movement(epochs_advanced, &split);
                }
            }
            Ok(None) => {}
            Err(e) => debug!(client = %client, "cursor rehydrate load failed: {e}"),
        }
    }

    /// The prune low-water-mark: MIN cursor across all tracked clients (vacuous case
    /// = 0, i.e. prune nothing — the conservative direction, since rehydration is lazy
    /// and an empty frontier post-restart does not mean "no client to protect").
    /// Consumed by 342b's prune gate.
    #[must_use]
    pub fn low_water_mark(&self) -> Epoch {
        self.lock().low_water_mark()
    }

    /// Whether `client` is currently tracked (known and not forgotten).
    #[must_use]
    pub fn is_tracked(&self, client: &ClientId) -> bool {
        self.lock().is_tracked(client)
    }

    /// Whether `client` is FORGOTTEN for re-admission-gate purposes — UNKNOWN
    /// (never tracked → "unknown == forgotten") OR lagged more than
    /// `forget_lag_epochs` behind the current server epoch. The re-admission gate
    /// reads this at gate time; `gate_decision_holds_at_commit` re-checks the same
    /// predicate at commit time under the per-key writer. See
    /// [`FrontierState::is_forgotten`].
    #[must_use]
    pub fn is_forgotten(&self, client: &ClientId) -> bool {
        self.lock().is_forgotten(client)
    }

    /// Whether re-admission protection is ACTIVE. True once the durability
    /// watermark is non-zero (prune activation). While it is 0 (dark by
    /// construction) the forgotten-client gate is fully wired but transparent: no
    /// tombstone can be pruned, so a re-admission cannot resurrect anything and
    /// blocking would only break an un-migrated client. The gate's active blocking
    /// goes live together with the prune (gate-before-activation — no
    /// prune-without-gate window ever exists, and equally no
    /// gratuitous-block-without-prune window).
    #[must_use]
    pub fn is_protection_active(&self) -> bool {
        self.refreshed_watermark() > 0
    }

    /// Set the max cursor-lag (epochs) before a tracked client is forgotten by the
    /// gate (RAM-pressure tightening / operator override). Clamped implicitly by
    /// the caller. See [`DEFAULT_FORGET_LAG_EPOCHS`].
    pub fn set_forget_lag_epochs(&self, lag: u64) {
        self.lock().forget_lag_epochs = lag;
    }

    /// The tracked cursor for `client`, if any. Test/introspection helper.
    #[must_use]
    pub fn cursor(&self, client: &ClientId) -> Option<Epoch> {
        self.lock().cursors.get(client).copied()
    }

    /// Forget a client (RAM-pressure / max-retention sacrifice). Consumed by 342c.
    ///
    /// Removes the client from the in-memory frontier AND deletes its durable cursor,
    /// so a forget is DURABLE. The whole cursor-loss-is-safe model requires a forgotten
    /// client to read as unknown → forgotten → full resync on its next connection: if
    /// the durable row outlived the forget, `rehydrate` on reconnect would silently
    /// re-track the client at its stale cursor and drop the low-water-mark below an
    /// already-pruned watermark → resurrection on that device. The delete is
    /// best-effort and safe in the OTHER direction — a failed delete only lets the row
    /// linger, re-tracking the client at a real cursor it genuinely reached (no
    /// premature prune), and 342f's orphan TTL is the backstop for a genuinely
    /// abandoned row.
    pub async fn forget_client(&self, client: &ClientId) {
        // Do the in-memory forget AND enqueue the durable delete under the SAME state
        // lock (see `confirm_apply_ack`): FIFO then orders the delete strictly after
        // every prior offloaded advance for this client — a late advance can no longer
        // land after the delete and resurrect the stale row. Enqueue is a non-blocking
        // send; the `done` oneshot is awaited AFTER the lock drops.
        let now = now_millis();
        let (rx, movement) = {
            let mut state = self.lock();
            state.forget_client(client);
            // Dropping the fleet's laggard is the single largest source of
            // low-water-mark movement there is; leaving it unobserved would put a
            // hole in the advance cadence exactly where the retention ceiling gets
            // its headroom.
            let movement = state.refresh_low_water_mark(now).map(|epochs_advanced| {
                let ceiling = state.reclamation_ceiling();
                (
                    epochs_advanced,
                    state.split_observation(state.observed_lwm, ceiling),
                )
            });
            let (done_tx, done_rx) = oneshot::channel();
            let rx = if self.enqueue_persist(PersistMsg::Forget {
                client: client.clone(),
                done: done_tx,
            }) {
                Some(done_rx)
            } else {
                None
            };
            (rx, movement)
        };
        if let Some((epochs_advanced, split)) = movement {
            self.publish_lwm_movement(epochs_advanced, &split);
        }
        // Whether the durable delete still needs a direct fallback. Three cases:
        //   * `None`         — never enqueued (no store, or worker already shut down).
        //   * `Some(Err(_))` — enqueued, but the worker died/was dropped before
        //     processing it (the oneshot sender was dropped without sending), so the
        //     delete may never have run.
        //   * `Some(Ok(false))` — the worker RAN but its `store.remove` FAILED
        //     (transient store error). Without this branch the failure would be
        //     silently reported as success and the row would linger.
        // Only `Some(Ok(true))` — a confirmed durable delete — skips the fallback.
        // The fallback is the same direct `store.remove`; it is idempotent, so running
        // it even when the worker did delete first is harmless. A forgotten client's
        // row must never survive to be rehydrated at its stale cursor on restart.
        let needs_fallback = match rx {
            Some(rx) => !rx.await.unwrap_or(false),
            None => true,
        };
        if needs_fallback {
            if let Some(store) = self.store.as_ref() {
                let now = i64::try_from(now_millis()).unwrap_or(i64::MAX);
                if let Err(e) = store.remove(CURSOR_MAP, client, now).await {
                    // Both the worker delete AND this fallback failed — a genuinely
                    // stuck durable row. Surface at `warn!` (not `debug!`) so operators
                    // can detect it; 342f's orphan TTL is the eventual backstop.
                    warn!(client = %client, "cursor forget fallback delete failed: {e}");
                }
            }
        }
    }

    /// Release a connection's per-connection `delivered` state on disconnect so the
    /// map stays bounded. The stored cursors are UNAFFECTED (they are per-identity,
    /// not per-connection, and survive across reconnects via rehydration).
    pub fn remove_connection(&self, conn: ConnectionId) {
        self.lock().delivered.remove(&conn);
    }

    /// Stamp a genuinely-new tombstone with the current server epoch at `OR_REMOVE`
    /// apply time. Server-authoritative — NEVER derived from the client tag's
    /// `millis`. Returns the stamped epoch (`>= 1`). See
    /// [`FrontierState::stamp_tombstone`].
    pub fn stamp_tombstone(&self, map: &str, key: &str, tag: &str) -> Epoch {
        // Snapshot the store's highest assigned write sequence as this epoch's
        // byte-durability bound. Read BEFORE taking the frontier lock (the store
        // call is independent) and outside it. With no store (tests / Null
        // backend) the bound is 0; those paths inject the watermark directly.
        let write_seq = self
            .store
            .as_ref()
            .map_or(0, |s| s.assigned_write_sequence());
        let (epoch, snapshot, entry_record) = {
            let mut state = self.lock();
            let (epoch, entry_record) = state.stamp_tombstone(map, key, tag, write_seq);
            (epoch, state.observation_snapshot(), entry_record)
        };
        // The stamp is the ONE path that grows the index, so it is where the index
        // and epoch gauges have to be refreshed for them to mean anything. Everything
        // published here is a stored value (carried ref count, `HashMap::len`, cached
        // low-water-mark), so the per-remove cost is a fixed handful of gauge stores
        // and no fold — the eligible/ineligible split deliberately does NOT run here.
        self.publish_frontier_state(&snapshot);
        // The entry line's formatting cost falls only at rollover (R2.4) — `None` on
        // every stamp that does not roll the clock past the PREVIOUS epoch.
        if let Some(record) = entry_record.as_ref() {
            self.publish_epoch_entry(record);
        }
        epoch
    }

    /// Recompute the cached byte-durability watermark from the store's live
    /// prefix-complete flushed watermark, then return it. Monotone: the cache
    /// only ever advances. With no store wired (tests / Null backend) the cache
    /// is left as-is so a test-injected watermark is honored.
    fn refreshed_watermark(&self) -> Epoch {
        let flushed = self.store.as_ref().map(|s| s.flushed_watermark());
        let mut state = self.lock();
        if let Some(flushed) = flushed {
            let computed = state.compute_durable_epoch_watermark(flushed);
            state.durable_epoch_watermark = state.durable_epoch_watermark.max(computed);
        }
        state.durable_epoch_watermark
    }

    /// The recovered/clamped low-water-mark every consumer should read
    /// (R12(d)): `min(persisted_LWM, durable_epoch_watermark)`. The clamp keeps a
    /// consumer from acting on an LWM the durable data cannot back — after an
    /// unclean recovery the byte-durability watermark is `E_rec` and the
    /// persisted LWM is 0 until clients reconnect, so the clamp is the
    /// persisted LWM; on the clean-restart continuity path it prevents pruning
    /// past what is byte-durable.
    #[must_use]
    pub fn effective_low_water_mark(&self) -> Epoch {
        let watermark = self.refreshed_watermark();
        self.lock().low_water_mark().min(watermark)
    }

    /// Unclean-recovery rebuild of the epoch index (R12(c)), invoked in the
    /// pre-listener WAL-recovery window (strictly before `accept()`). Scans the
    /// durable store for every live OR-Map tombstone and re-stamps them all into
    /// one fresh maximally-lagging recovery epoch:
    ///
    /// `E_rec = 1 + max(persisted counter hint, max epoch referenced by any
    /// persisted cursor, ceil(flushed_watermark / EPOCH_WIDTH))`.
    ///
    /// The max-cursor term is load-bearing: it guarantees no tracked client is
    /// ever considered already-past `E_rec`, killing the stale-counter-hint
    /// resurrection trace. The RAM index is never persisted on the hot path, so
    /// the counter hint is 0 (a clean-shutdown persist could supply one — an
    /// optimization, never a correctness input). Returns the chosen `E_rec` (0
    /// when there is no durable backend, e.g. the Null store or a store-less
    /// test frontier).
    ///
    /// # Errors
    ///
    /// Returns an error if the durable keyspace scan (cursor namespace or live
    /// tombstones) fails; the caller MUST fail closed (an empty index with an
    /// un-bumped counter would let a stale-high cursor prune a fresh epoch).
    pub async fn rebuild_from_durable_store(&self) -> anyhow::Result<Epoch> {
        let Some(store) = self.store.as_ref() else {
            return Ok(0);
        };
        if store.is_null() {
            return Ok(0);
        }
        // Load-bearing term: the highest epoch any persisted cursor references
        // (keyspace scan over the 342e cursor namespace).
        let max_cursor_epoch = scan_max_cursor_epoch(store.as_ref()).await?;
        let flushed = store.flushed_watermark();
        let width = self.lock().epoch_width.max(1);
        let flushed_epochs = flushed.div_ceil(width);
        // No persisted counter hint (index is RAM-only on the hot path).
        let counter_hint = 0u64;
        let e_rec = 1 + max_cursor_epoch.max(flushed_epochs).max(counter_hint);

        let live = scan_live_tombstones(store.as_ref()).await?;
        let restamped = live.len();
        let (snapshot, exits) = {
            let mut state = self.lock();
            let exits = state.rebuild_into_epoch(e_rec, live);
            (state.observation_snapshot(), exits)
        };
        // The rebuild replaces the whole index in the pre-listener recovery window,
        // so publishing here is what stops the first post-recovery scrape reporting
        // the pre-crash index size.
        self.publish_frontier_state(&snapshot);
        // Detection point (R2.3b): every epoch the wholesale clear above just
        // orphaned gets its exit row, attributed to this rebuild.
        for record in &exits {
            self.publish_epoch_exit(record);
        }
        debug!(
            e_rec,
            max_cursor_epoch,
            flushed_epochs,
            restamped,
            "tombstone epoch index rebuilt into a maximally-lagging recovery epoch"
        );
        Ok(e_rec)
    }

    /// The current (highest) server-stamped epoch, or 0 if none stamped yet. This is
    /// the covering epoch conveyed in OR-Map sync responses.
    #[must_use]
    pub fn current_epoch(&self) -> Epoch {
        self.lock().current_epoch
    }

    /// The live byte-durability watermark: `max E such that every stamped epoch
    /// `e <= E` is durable in the inner store`, recomputed from the store's
    /// prefix-complete flushed watermark. 0 until the first epoch's bytes are
    /// durable (or, after an unclean recovery, until the pre-listener rebuild
    /// fills the index). With no store wired it returns the last injected value.
    #[must_use]
    pub fn durable_epoch_watermark(&self) -> Epoch {
        self.refreshed_watermark()
    }

    /// Test-only injection of the durability watermark to exercise the drop path
    /// on a store-less frontier (`new(None)`), where `refreshed_watermark` leaves
    /// the cache untouched. Production wires a real store, so the watermark is
    /// always the computed byte-durability value, never this override.
    #[cfg(test)]
    pub fn set_durable_epoch_watermark(&self, watermark: Epoch) {
        self.lock().durable_epoch_watermark = watermark;
    }

    /// Whether `epoch` is prune-eligible under the reclamation ceiling ONLY (STRICT:
    /// eligible once the ceiling has advanced PAST `epoch`). The ceiling is the fleet
    /// MIN over the registered claims, less the configured margin, and — like the
    /// low-water mark it replaces — it may FALL when a laggard rejoins the fold;
    /// monotonicity lives on the registry's executed watermark, not here. The
    /// durability fence is the SECOND call-site conjunct in
    /// [`Self::drain_prunable_tombstones`], never here.
    #[must_use]
    pub fn is_epoch_prune_eligible(&self, epoch: Epoch) -> bool {
        self.lock().is_epoch_prune_eligible(epoch)
    }

    /// Commit-time re-check for the push-diff re-admission gate: whether the
    /// not-forgotten decision certified at gate time STILL holds now. Consumes the
    /// `GateToken` by value (single-use). Lag-aware — see
    /// [`FrontierState::is_forgotten`] and the extended
    /// `gate_decision_holds_at_commit` impl. Called under the per-key writer held
    /// from the gate decision through the merge-commit `store.put`.
    #[must_use]
    pub fn gate_decision_holds_at_commit(&self, token: GateToken) -> bool {
        self.lock().gate_decision_holds_at_commit(token)
    }

    /// Drain every currently prune-eligible epoch's tombstone refs (BOTH call-site
    /// conjuncts — the reclamation ceiling AND the durability watermark) out of the RAM
    /// index, tagged with their epoch, for the caller to drop from storage (RAM + redb)
    /// under the per-key writer. A ref whose storage
    /// drop fails MUST be handed back via [`Self::restore_tombstone_ref`] so it is
    /// retried later rather than orphaned un-prunable. DARK by construction:
    /// returns empty in production (`durable_epoch_watermark == 0`), and the dark path
    /// returns before the sweep protocol is entered at all. A non-dark pass is bracketed
    /// by that protocol: it filters on the ceiling its sweep token carries and reports the
    /// watermark it observed, so the boundary is derived by the registry rather than
    /// proposed here.
    #[must_use]
    pub fn drain_prunable_tombstones(&self) -> Vec<(Epoch, TombstoneRef)> {
        // Refresh the cached byte-durability watermark from the store's live
        // flushed watermark, then drain under BOTH call-site conjuncts. Reading
        // the store's watermark outside the lock keeps the frontier lock hold
        // short; the field is then updated and consumed under one lock.
        let flushed = self.store.as_ref().map(|s| s.flushed_watermark());
        let (drained, snapshot, split, last_advance_millis, exits) = {
            let mut state = self.lock();
            if let Some(flushed) = flushed {
                let computed = state.compute_durable_epoch_watermark(flushed);
                state.durable_epoch_watermark = state.durable_epoch_watermark.max(computed);
            }
            // A non-empty drain is the second of the two events the budget licenses
            // index-proportional work on. An empty drain — which is every drain while
            // the prune is dark, i.e. the per-remove case — takes the cheap path and
            // recomputes nothing, which is also why the split needs its staleness
            // marker to be readable at all. The split travels OUT of the drain because
            // it is taken before the removal: the licensed backlog only exists to be
            // read on the near side of the work that consumes it.
            let (drained, pre_drain_split, exits) = state.drain_prunable();
            let split = if drained.is_empty() {
                None
            } else {
                pre_drain_split
            };
            (
                drained,
                state.observation_snapshot(),
                split,
                state.last_lwm_advance_millis,
                exits,
            )
        };
        self.publish_frontier_state(&snapshot);
        if let Some(split) = split {
            self.publish_split(&split);
            // The clock is read only on the non-empty branch, so the per-remove path
            // pays no syscall for a gauge the ACK path already refreshes.
            self.prune_observer
                .observe_lwm_stall(now_millis().saturating_sub(last_advance_millis) / 1000);
        }
        // Detection point (R2.3b): fires whether or not this pass itself drained
        // anything — an epoch removed by some OTHER path is still detected here.
        for record in &exits {
            self.publish_epoch_exit(record);
        }
        drained
    }

    /// Re-insert a drained tombstone ref whose storage drop FAILED (see
    /// [`Self::drain_prunable_tombstones`]). The index entry is restored so a later
    /// sweep retries the drop; `epoch_max_seq` is re-created best-effort (pure RAM
    /// cache — the unclean-recovery rebuild is the authoritative recovery).
    pub fn restore_tombstone_ref(&self, epoch: Epoch, tombstone_ref: TombstoneRef) {
        let snapshot = {
            let mut state = self.lock();
            state.restore(epoch, tombstone_ref);
            state.observation_snapshot()
        };
        // A restore puts a ref back into the index, so the index gauge has to follow
        // it back up: a drain that decremented and a restore that did not would make
        // the gauge drift down by exactly the refs a failing store keeps handing back.
        self.publish_frontier_state(&snapshot);
    }

    /// Set the epoch width (stamped ops per epoch, clamped `>= 1`). Wired from the
    /// bin's `TOPGUN_EPOCH_WIDTH`; also settable in tests.
    pub fn set_epoch_width(&self, width: u64) {
        self.lock().epoch_width = width.max(1);
    }

    /// The configured epoch width (for the startup config log line / tests).
    #[must_use]
    pub fn epoch_width(&self) -> u64 {
        self.lock().epoch_width
    }
}

/// Encode an epoch as a fixed 8-byte big-endian blob for lossless redb storage.
fn encode_epoch(epoch: Epoch) -> Vec<u8> {
    epoch.to_be_bytes().to_vec()
}

/// Decode an 8-byte big-endian epoch blob. Returns `None` on a malformed length.
fn decode_epoch(bytes: &[u8]) -> Option<Epoch> {
    let arr: [u8; 8] = bytes.try_into().ok()?;
    Some(Epoch::from_be_bytes(arr))
}

/// Background worker draining one frontier's cursor-persistence queue.
///
/// A SINGLE FIFO consumer serializes all cursor durability, which is what makes
/// the persisted cursor monotone under a blind-clobber store (`MapDataStore::add`
/// is last-write-by-arrival, not a timestamp LWW merge): it keeps an in-memory
/// high-water per client and writes only on a STRICT advance, so a stale racing
/// advance is dropped rather than clobbering a higher value. The high-water is
/// SEEDED from the durable store the first time a client is seen, so a fresh
/// worker after a restart can never regress a cursor a previous process persisted
/// higher. `Forget` deletes the durable row and clears the high-water; FIFO
/// ordering runs it strictly after every prior advance for that client, so a
/// forgotten client cannot be resurrected by a late persist.
async fn cursor_persist_worker(
    store: Arc<dyn MapDataStore>,
    mut rx: mpsc::UnboundedReceiver<PersistMsg>,
) {
    let mut high_water: HashMap<ClientId, Epoch> = HashMap::new();
    // Clients whose seed load has already warned. A PERSISTENT store read fault
    // stalls a client's cursor durability indefinitely (each advance defers), and
    // this arm runs per-ACK — so surface the FIRST failure per client at `warn!`
    // for operators and keep the per-advance repeats at `debug!` to avoid flooding
    // the hot path. A later successful seed clears the flag so a NEW fault warns
    // again. Bounded by client count, like `high_water`.
    let mut seed_load_warned: HashSet<ClientId> = HashSet::new();
    while let Some(msg) = rx.recv().await {
        match msg {
            PersistMsg::Advance { client, epoch } => {
                if !high_water.contains_key(&client) {
                    match load_cursor(store.as_ref(), &client).await {
                        Ok(seed) => {
                            seed_load_warned.remove(&client);
                            high_water.insert(client.clone(), seed.unwrap_or(0));
                        }
                        Err(e) => {
                            // Do NOT seed 0 on a transient load error — that would let
                            // this advance clobber a higher cursor a previous process
                            // persisted (monotonicity regression). Skip; a later advance
                            // re-attempts the seed. Best-effort and safe: a missed
                            // advance leaves a lower persisted cursor = less prune.
                            if seed_load_warned.insert(client.clone()) {
                                warn!(client = %client, epoch, "cursor seed load failed: {e}; deferring advance (repeats logged at debug)");
                            } else {
                                debug!(client = %client, epoch, "cursor seed load failed: {e}; deferring advance");
                            }
                            continue;
                        }
                    }
                }
                let current = high_water.get(&client).copied().unwrap_or(0);
                if epoch > current {
                    match persist_cursor(store.as_ref(), &client, epoch).await {
                        Ok(()) => {
                            high_water.insert(client, epoch);
                        }
                        Err(e) => {
                            // Best-effort: leave the high-water unchanged so a later
                            // equal-or-higher advance retries the durable write.
                            debug!(client = %client, epoch, "cursor persist failed: {e}");
                        }
                    }
                }
            }
            PersistMsg::Forget { client, done } => {
                let now = i64::try_from(now_millis()).unwrap_or(i64::MAX);
                // Report the DURABLE-DELETE OUTCOME, not merely "the worker saw the
                // message". A swallowed store error here previously still signalled
                // success, so `forget_client` skipped its idempotent fallback delete
                // exactly when the delete had failed — leaving the forgotten row alive
                // to be rehydrated at its stale cursor (the resurrection hazard the
                // fallback exists to close). Report `false` on failure so the caller
                // retries.
                let removed = match store.remove(CURSOR_MAP, &client, now).await {
                    Ok(()) => {
                        // Drop the in-memory high-water ONLY once the durable row is
                        // actually gone. If the delete FAILED the stale row still lives
                        // in the store, so retaining its high-water keeps a later
                        // re-admission `Advance`'s seed consistent with the store:
                        // otherwise the worker would re-seed from the store (or from 0
                        // if the failed delete nonetheless removed the row) and a LOWER
                        // re-admission cursor could clobber the higher value the client
                        // previously reached — a durable monotonicity regression that
                        // can drop the low-water-mark below an already-pruned watermark.
                        high_water.remove(&client);
                        true
                    }
                    Err(e) => {
                        warn!(client = %client, "cursor forget delete failed: {e}");
                        false
                    }
                };
                let _ = done.send(removed);
            }
            PersistMsg::Barrier { done } => {
                let _ = done.send(());
            }
        }
    }
}

/// Persist `client`'s cursor into the reserved redb keyspace as an LWW record.
///
/// A cursor row is a single-writer server artifact (not a merged CRDT value), so
/// the LWW timestamp is unimportant; a monotone millis is used so a later write
/// wins.
async fn persist_cursor(
    store: &dyn MapDataStore,
    client: &ClientId,
    epoch: Epoch,
) -> anyhow::Result<()> {
    let now = now_millis();
    let record = RecordValue::Lww {
        value: Value::Bytes(encode_epoch(epoch)),
        timestamp: Timestamp {
            millis: now,
            counter: 0,
            node_id: String::new(),
        },
    };
    store
        .add(
            CURSOR_MAP,
            client,
            &record,
            0,
            i64::try_from(now).unwrap_or(i64::MAX),
        )
        .await
}

/// Load `client`'s persisted cursor from the reserved redb keyspace, if any.
async fn load_cursor(store: &dyn MapDataStore, client: &ClientId) -> anyhow::Result<Option<Epoch>> {
    match store.load(CURSOR_MAP, client).await? {
        Some(RecordValue::Lww {
            value: Value::Bytes(bytes),
            ..
        }) => Ok(decode_epoch(&bytes)),
        _ => Ok(None),
    }
}

/// Scan the persisted cursor keyspace ([`CURSOR_MAP`]) and return the highest
/// epoch any client cursor references, or 0 if none. This is the load-bearing
/// `max-cursor-epoch` term of `E_rec`: `E_rec` must exceed it so no persisted
/// client is ever considered already-past the fresh recovery epoch.
async fn scan_max_cursor_epoch(store: &dyn MapDataStore) -> anyhow::Result<Epoch> {
    let mut max_epoch: Epoch = 0;
    let mut batch = store.scan_values(CURSOR_MAP, false, 0).await?;
    loop {
        for (_key, value) in &batch.records {
            if let RecordValue::Lww {
                value: Value::Bytes(bytes),
                ..
            } = value
            {
                if let Some(epoch) = decode_epoch(bytes) {
                    max_epoch = max_epoch.max(epoch);
                }
            }
        }
        match batch.next_cursor.take() {
            None => break,
            Some(cursor) => {
                batch = store
                    .scan_values_batched(CURSOR_MAP, false, cursor, 0)
                    .await?;
            }
        }
    }
    Ok(max_epoch)
}

/// Scan the durable keyspace for every live OR-Map tombstone (post WAL-replay),
/// returning a [`TombstoneRef`] per `(map, key, tag)`. The unclean-recovery
/// rebuild re-stamps all of these into the fresh recovery epoch. The reserved
/// internal keyspaces ([`CURSOR_MAP`] and other `_topgun_`-prefixed maps) hold
/// no OR-Map tombstones (their records are LWW), so they contribute nothing and
/// are handled by the explicit `Lww` no-op arm. Legacy `OrTombstones` blobs are
/// Merkle-invisible (TODO-559) and out of this child's prune scope, so they are
/// deliberately NOT re-stamped here — an explicit no-op arm makes that exclusion
/// visible to any future refactor.
async fn scan_live_tombstones(store: &dyn MapDataStore) -> anyhow::Result<Vec<TombstoneRef>> {
    let mut live = Vec::new();
    for map in store.list_maps().await? {
        let mut batch = store.scan_values(&map, false, 0).await?;
        loop {
            for (key, value) in &batch.records {
                match value {
                    RecordValue::OrMap { tombstones, .. } => {
                        for tag in tombstones {
                            live.push(TombstoneRef {
                                map: map.clone(),
                                key: key.clone(),
                                tag: tag.clone(),
                            });
                        }
                    }
                    // Legacy tombstone blobs are out of the prune scope: an
                    // untouched legacy row is deliberately NEVER re-stamped into a
                    // recovery epoch, so it can never become prune-eligible. LWW
                    // records carry no OR-Map tombstones. Both are explicit no-op
                    // arms so the exclusion is visible at the point of change — a
                    // future "handle every variant" refactor confronts the named
                    // `OrTombstones` pattern instead of silently pulling legacy
                    // blobs into the epoch index. (A later OR write to the key
                    // upconverts the record to `OrMap`, after which its tags join
                    // the protected regime — expected.)
                    RecordValue::OrTombstones { .. } | RecordValue::Lww { .. } => {}
                }
            }
            match batch.next_cursor.take() {
                None => break,
                Some(cursor) => {
                    batch = store.scan_values_batched(&map, false, cursor, 0).await?;
                }
            }
        }
    }
    Ok(live)
}

// ---------------------------------------------------------------------------
// Prune record — arming and the two observers
// ---------------------------------------------------------------------------

/// The kill-switch that arms or disarms the prune record.
///
/// Declared once so the read site below is the only place the name appears as a value; the
/// parse discipline and the default are documented on [`PruneRecordArming`].
const PRUNE_RECORD_ARMING_ENV: &str = "TOPGUN_PRUNE_RECORD";

/// Reads the prune-record arming decision from the process environment.
///
/// Called exactly once, from [`TombstoneFrontier::new`]. An unset variable arms the record,
/// which is what makes the instrument present by default on a node nobody configured.
fn prune_record_arming_from_env() -> PruneRecordArming {
    match std::env::var(PRUNE_RECORD_ARMING_ENV) {
        Ok(raw) => parse_prune_record_arming(&raw),
        Err(_) => PruneRecordArming::Armed,
    }
}

/// Parses one arming value.
///
/// Only an explicit falsey word disarms. An unrecognised value stays ARMED rather than
/// silently turning the instrument off on a typo — a disarmed run that an operator believes
/// is armed produces a whole measurement window of empty series, and the cost of that is paid
/// long after the typo. Staying armed is deliberate; staying silent about it is not, so an
/// unrecognised value is warned about with the offending text.
fn parse_prune_record_arming(raw: &str) -> PruneRecordArming {
    let normalized = raw.trim().to_lowercase();
    if matches!(normalized.as_str(), "false" | "0" | "no" | "off") {
        return PruneRecordArming::Disarmed;
    }
    if !matches!(normalized.as_str(), "true" | "1" | "yes" | "on") {
        warn!(
            target: "topgun_server::tombstone_frontier",
            var = PRUNE_RECORD_ARMING_ENV,
            value = %raw,
            "Unrecognised value; leaving the prune record ARMED. Only false/0/no/off \
             (case-insensitive) disarm it — check for a typo if you meant to turn the \
             instrument off"
        );
    }
    PruneRecordArming::Armed
}

/// Resolve a counter handle and touch it, so the series is registered before any observation.
fn touched_counter(name: &'static str) -> Counter {
    let handle = metrics::counter!(name);
    // An `increment(0)` is what puts the series in the exporter's registry; a `describe_*`
    // call would not, because descriptions are only attached to a metric already present in
    // the snapshot. Without this the series is absent until its first real observation, and a
    // downstream sampler cannot tell an absent series from a stalled one.
    handle.increment(0);
    handle
}

/// Resolve a gauge handle and touch it, so the series is registered before any observation.
fn touched_gauge(name: &'static str) -> Gauge {
    let handle = metrics::gauge!(name);
    handle.set(0.0);
    handle
}

/// Resolve a histogram handle.
///
/// Resolving is the whole registration act for a histogram in this exporter — the render pass
/// creates a distribution entry for every registered handle unconditionally, before any sample
/// is drained, so an unobserved histogram renders `_sum 0` and `_count 0` rather than nothing.
/// Recording a synthetic zero would be worse than useless: it would put a fabricated
/// observation into the distribution the record is supposed to describe.
fn registered_histogram(name: &'static str) -> Histogram {
    metrics::histogram!(name)
}

/// The armed prune-record observer: one cached metric handle per pinned series.
///
/// Handles are resolved once, at construction, and never re-resolved: a `counter!` / `gauge!` /
/// `histogram!` macro invocation at an observation call site performs a registry lookup by
/// name, and this record sits close enough to the prune loop that a per-observation lookup
/// would be a perturbation of the very thing it measures.
///
/// Every field is emitted through its pinned name constant. A string literal at an emit site
/// is a defect: binding the emitter to the constant is what makes the compiler, rather than a
/// document review, the thing that stops an emitted name drifting from the pinned one.
pub struct MetricsPruneRecorder {
    passes: Counter,
    considered: Counter,
    dropped: Counter,
    matched_nothing: Counter,
    absent: Counter,
    restored_read_error: Counter,
    restored_evicted: Counter,
    restored_write_error: Counter,
    bytes_freed: Counter,
    epochs_drained: Counter,
    empty_drains: Counter,
    nonempty_drains: Counter,
    lwm_advances: Counter,
    lwm_epochs_advanced: Counter,
    split_recomputes: Counter,
    stamped_refs: Counter,
    stamped_bytes: Counter,
    drained_refs: Counter,
    // Eagerly registered (AC4/R2.6(i)) but not incremented: a restore does not
    // correspond to a per-epoch entry/exit event, and this trait — frozen by G1 for
    // this half — carries no method a restore call site could feed. O-0's own
    // conservation identity does not depend on this Prometheus mirror: it reads
    // FrontierState's own `restored_refs_total`, incremented precisely on every
    // `restore` (see `IndexConservationSnapshot`), which is unaffected by this gap.
    #[allow(dead_code)]
    restored_refs: Counter,
    rebuild_cleared_refs: Counter,
    epochs_entered: Counter,
    epochs_exited: Counter,
    // The two OBSERVATION counters (R1.1, R3): read from the vector the index removal
    // itself returned, at the removal site -- never a copy of `refs_at_entry` /
    // `bytes_freed_attributed`. Credited on the `DrainedByPrune` arm only (R1.6, R3.2).
    removed_refs_observed: Counter,
    removed_bytes_observed: Counter,

    indexed_refs: Gauge,
    indexed_epochs: Gauge,
    eligible_refs: Gauge,
    ineligible_refs: Gauge,
    split_computed_epoch: Gauge,
    current_epoch: Gauge,
    low_water_mark: Gauge,
    durable_epoch_watermark: Gauge,
    last_drained_epoch: Gauge,
    lwm_stall_seconds: Gauge,
    tracked_claims: Gauge,

    drain_refs: Histogram,
    drain_epochs: Histogram,
    claim_span_epochs: Histogram,
    claim_lag_epochs: Histogram,
    epoch_considered: Histogram,
    epoch_dropped: Histogram,
    epoch_bytes_freed: Histogram,
}

impl std::fmt::Debug for MetricsPruneRecorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The handles carry no readable state; naming the type is the whole useful content.
        f.debug_struct("MetricsPruneRecorder")
            .finish_non_exhaustive()
    }
}

impl MetricsPruneRecorder {
    /// Resolve — and thereby register — every pinned series.
    ///
    /// # Construction-order precondition
    ///
    /// The Prometheus recorder MUST already be installed when this runs. A handle resolved
    /// before installation binds to a no-op for its whole lifetime and never re-resolves, so a
    /// recorder built too early is permanently silent while still reporting itself as armed.
    /// [`TombstoneFrontier::new`] carries the audit of the production construction sites that
    /// depend on this; a test that renders after constructing this type MUST bind a recorder
    /// first.
    ///
    /// Registration is deliberately eager rather than lazy-on-first-use. Every one of the
    /// pinned series exists in the FIRST scrape, so a downstream sampler never has to
    /// distinguish "this series has not moved" from "this series does not exist" — the second
    /// case is unrepresentable by construction.
    #[must_use]
    pub fn new() -> Self {
        Self {
            passes: touched_counter(METRIC_PRUNE_PASSES_TOTAL),
            considered: touched_counter(METRIC_PRUNE_CONSIDERED_TOTAL),
            dropped: touched_counter(METRIC_PRUNE_DROPPED_TOTAL),
            matched_nothing: touched_counter(METRIC_PRUNE_MATCHED_NOTHING_TOTAL),
            absent: touched_counter(METRIC_PRUNE_ABSENT_TOTAL),
            restored_read_error: touched_counter(METRIC_PRUNE_RESTORED_READ_ERROR_TOTAL),
            restored_evicted: touched_counter(METRIC_PRUNE_RESTORED_EVICTED_TOTAL),
            restored_write_error: touched_counter(METRIC_PRUNE_RESTORED_WRITE_ERROR_TOTAL),
            bytes_freed: touched_counter(METRIC_PRUNE_BYTES_FREED_TOTAL),
            epochs_drained: touched_counter(METRIC_PRUNE_EPOCHS_DRAINED_TOTAL),
            empty_drains: touched_counter(METRIC_PRUNE_EMPTY_DRAINS_TOTAL),
            nonempty_drains: touched_counter(METRIC_PRUNE_NONEMPTY_DRAINS_TOTAL),
            lwm_advances: touched_counter(METRIC_PRUNE_LWM_ADVANCES_TOTAL),
            lwm_epochs_advanced: touched_counter(METRIC_PRUNE_LWM_EPOCHS_ADVANCED_TOTAL),
            split_recomputes: touched_counter(METRIC_PRUNE_SPLIT_RECOMPUTES_TOTAL),
            stamped_refs: touched_counter(METRIC_PRUNE_STAMPED_REFS_TOTAL),
            stamped_bytes: touched_counter(METRIC_PRUNE_STAMPED_BYTES_TOTAL),
            drained_refs: touched_counter(METRIC_PRUNE_DRAINED_REFS_TOTAL),
            restored_refs: touched_counter(METRIC_PRUNE_RESTORED_REFS_TOTAL),
            rebuild_cleared_refs: touched_counter(METRIC_PRUNE_REBUILD_CLEARED_REFS_TOTAL),
            epochs_entered: touched_counter(METRIC_PRUNE_EPOCHS_ENTERED_TOTAL),
            epochs_exited: touched_counter(METRIC_PRUNE_EPOCHS_EXITED_TOTAL),
            removed_refs_observed: touched_counter(METRIC_PRUNE_REMOVED_REFS_OBSERVED_TOTAL),
            removed_bytes_observed: touched_counter(METRIC_PRUNE_REMOVED_BYTES_OBSERVED_TOTAL),

            indexed_refs: touched_gauge(METRIC_PRUNE_INDEXED_REFS),
            indexed_epochs: touched_gauge(METRIC_PRUNE_INDEXED_EPOCHS),
            eligible_refs: touched_gauge(METRIC_PRUNE_ELIGIBLE_REFS),
            ineligible_refs: touched_gauge(METRIC_PRUNE_INELIGIBLE_REFS),
            split_computed_epoch: touched_gauge(METRIC_PRUNE_SPLIT_COMPUTED_EPOCH),
            current_epoch: touched_gauge(METRIC_PRUNE_CURRENT_EPOCH),
            low_water_mark: touched_gauge(METRIC_PRUNE_LOW_WATER_MARK),
            durable_epoch_watermark: touched_gauge(METRIC_PRUNE_DURABLE_EPOCH_WATERMARK),
            last_drained_epoch: touched_gauge(METRIC_PRUNE_LAST_DRAINED_EPOCH),
            lwm_stall_seconds: touched_gauge(METRIC_PRUNE_LWM_STALL_SECONDS),
            tracked_claims: touched_gauge(METRIC_PRUNE_TRACKED_CLAIMS),

            drain_refs: registered_histogram(METRIC_PRUNE_DRAIN_REFS),
            drain_epochs: registered_histogram(METRIC_PRUNE_DRAIN_EPOCHS),
            claim_span_epochs: registered_histogram(METRIC_PRUNE_CLAIM_SPAN_EPOCHS),
            claim_lag_epochs: registered_histogram(METRIC_PRUNE_CLAIM_LAG_EPOCHS),
            epoch_considered: registered_histogram(METRIC_PRUNE_EPOCH_CONSIDERED),
            epoch_dropped: registered_histogram(METRIC_PRUNE_EPOCH_DROPPED),
            epoch_bytes_freed: registered_histogram(METRIC_PRUNE_EPOCH_BYTES_FREED),
        }
    }
}

impl Default for MetricsPruneRecorder {
    fn default() -> Self {
        Self::new()
    }
}

impl PruneRecordObserver for MetricsPruneRecorder {
    // The gauge and histogram APIs take `f64` by signature, so the conversion happens at the
    // emit call and nowhere else — the record structs stay integral. Precision is exact below
    // 2^53; no ref count, epoch id or byte total one process accumulates approaches that.
    #[allow(clippy::cast_precision_loss)]
    fn observe_pass(&self, record: &PrunePassRecord) {
        // A pass is counted on every invocation, empty drains included: a pass count that only
        // moved when work happened would read zero during a total stall, which is precisely the
        // regime this record has to be able to describe.
        self.passes.increment(1);
        self.considered.increment(record.considered);
        self.dropped.increment(record.dropped);
        self.matched_nothing.increment(record.matched_nothing);
        self.absent.increment(record.absent);
        self.restored_read_error
            .increment(record.restored_read_error);
        self.restored_evicted.increment(record.restored_evicted);
        self.restored_write_error
            .increment(record.restored_write_error);
        self.bytes_freed.increment(record.bytes_freed);
        self.epochs_drained.increment(record.epochs_drained);
        if record.empty_drain {
            self.empty_drains.increment(1);
        } else {
            self.nonempty_drains.increment(1);
            // The per-drain distributions describe batch SIZE, so they take no observation
            // from an empty drain: an empty drain measures scheduling frequency, and folding
            // it in would drag the batch-size mean toward zero for a reason unrelated to
            // reclamation.
            self.drain_refs.record(record.considered as f64);
            self.drain_epochs.record(record.epochs_drained as f64);
        }
    }

    #[allow(clippy::cast_precision_loss)]
    fn observe_drained_epoch(&self, record: &PruneEpochRecord) {
        self.epoch_considered.record(record.considered as f64);
        self.epoch_dropped.record(record.dropped as f64);
        self.epoch_bytes_freed.record(record.bytes_freed as f64);
        // The epoch/watermark gauges are deliberately NOT written here. They are published from a
        // snapshot taken under the drain's own lock (`observe_epoch_state`); the pruning caller
        // holds no lock, so anything it could pass in would be three independent reads able to tear
        // against a concurrent ACK — and writing it here would let that torn copy win.
    }

    #[allow(clippy::cast_precision_loss)]
    fn observe_claim_span(&self, record: &PruneClaimSpanRecord, claim_lags: &[Epoch]) {
        self.claim_span_epochs.record(record.span_epochs as f64);
        self.tracked_claims.set(record.tracked_claims as f64);
        self.current_epoch.set(record.current_epoch as f64);
        self.low_water_mark.set(record.low_water_mark as f64);
        // Borrowed, so the per-claim lag distribution costs no allocation on a path whose
        // whole point is to stay cheap.
        for lag in claim_lags {
            self.claim_lag_epochs.record(*lag as f64);
        }
    }

    fn observe_lwm_advance(&self, epochs_advanced: u64) {
        // Called only on an actual advance, so this stays a count of movements rather than of
        // confirmations.
        self.lwm_advances.increment(1);
        self.lwm_epochs_advanced.increment(epochs_advanced);
    }

    #[allow(clippy::cast_precision_loss)]
    fn observe_lwm_stall(&self, stall_seconds: u64) {
        self.lwm_stall_seconds.set(stall_seconds as f64);
    }

    #[allow(clippy::cast_precision_loss)]
    fn observe_index_state(&self, indexed_refs: u64, indexed_epochs: u64) {
        self.indexed_refs.set(indexed_refs as f64);
        self.indexed_epochs.set(indexed_epochs as f64);
    }

    #[allow(clippy::cast_precision_loss)]
    fn observe_epoch_state(
        &self,
        current_epoch: Epoch,
        low_water_mark: Epoch,
        durable_epoch_watermark: Epoch,
        last_drained_epoch: Epoch,
    ) {
        self.current_epoch.set(current_epoch as f64);
        self.low_water_mark.set(low_water_mark as f64);
        self.durable_epoch_watermark
            .set(durable_epoch_watermark as f64);
        self.last_drained_epoch.set(last_drained_epoch as f64);
    }

    #[allow(clippy::cast_precision_loss)]
    fn observe_eligibility_split(
        &self,
        eligible_refs: u64,
        ineligible_refs: u64,
        computed_at_epoch: Epoch,
    ) {
        self.eligible_refs.set(eligible_refs as f64);
        self.ineligible_refs.set(ineligible_refs as f64);
        // The split is recomputed only on the events that stop happening during a stall, so a
        // reader needs both the epoch it was computed at and a monotone recompute count to tell
        // a fresh sample from one frozen at its last recompute.
        self.split_computed_epoch.set(computed_at_epoch as f64);
        self.split_recomputes.increment(1);
    }

    fn observe_epoch_entry(&self, record: &PruneEpochEntryRecord) {
        self.epochs_entered.increment(1);
        // Batched to rollover rather than per-stamp: the entry row fires exactly once
        // per epoch, carrying that epoch's FINAL accumulated total, so crediting the
        // cumulative counters here (rather than per individual stamp) yields the same
        // final total at the cost of zero extra Prometheus calls on the hot stamp path
        // — consistent with R2.4's costing, which prices the entry line's formatting
        // at rollover and nothing per `OR_REMOVE`. A control-class (f) EMPTY-EPOCH row
        // (`entered_index == false`) contributes 0 to both, which is correct: nothing
        // was ever stamped into it.
        self.stamped_refs.increment(record.stamped_refs);
        self.stamped_bytes.increment(record.stamped_bytes);
    }

    fn observe_epoch_residency(&self, record: &PruneEpochResidencyRecord) {
        self.epochs_exited.increment(1);
        // `refs_at_entry` is the epoch's whole resident set at the moment of removal —
        // a drain or a rebuild takes it atomically, so it is exactly what left the
        // index on that exit, never an approximation.
        match record.exit_kind {
            EpochExitKind::DrainedByPrune => {
                self.drained_refs.increment(record.refs_at_entry);
                // The two OBSERVATION counters, credited on this arm only (R1.6, R3.2):
                // read from the vector the index removal returned, never from the
                // by-construction attribution `refs_at_entry` this arm already credits
                // above.
                self.removed_refs_observed
                    .increment(record.removed_refs_observed);
                self.removed_bytes_observed
                    .increment(record.removed_bytes_observed);
            }
            EpochExitKind::ClearedByRebuild => {
                self.rebuild_cleared_refs.increment(record.refs_at_entry);
            }
            EpochExitKind::StillResidentAtShutdown | EpochExitKind::Unclassified { .. } => {}
        }
    }
}

/// The disarmed prune-record observer.
///
/// Every method is empty on purpose: disarmed means no allocation, no atomic write and no
/// metrics call, so the disarmed lineage is a genuine control rather than a cheaper version of
/// the armed one. It is a unit struct so constructing it allocates nothing either, and it
/// resolves no handle, so a disarmed process registers none of the pinned series — which is
/// what makes the effective arming provable from a scrape.
#[derive(Debug, Clone, Copy, Default)]
pub struct NullPruneRecorder;

impl PruneRecordObserver for NullPruneRecorder {
    fn observe_pass(&self, _record: &PrunePassRecord) {}

    fn observe_drained_epoch(&self, _record: &PruneEpochRecord) {}

    fn observe_claim_span(&self, _record: &PruneClaimSpanRecord, _claim_lags: &[Epoch]) {}

    fn observe_lwm_advance(&self, _epochs_advanced: u64) {}

    fn observe_lwm_stall(&self, _stall_seconds: u64) {}

    fn observe_index_state(&self, _indexed_refs: u64, _indexed_epochs: u64) {}

    fn observe_epoch_state(
        &self,
        _current_epoch: Epoch,
        _low_water_mark: Epoch,
        _durable_epoch_watermark: Epoch,
        _last_drained_epoch: Epoch,
    ) {
    }

    fn observe_eligibility_split(
        &self,
        _eligible_refs: u64,
        _ineligible_refs: u64,
        _computed_at_epoch: Epoch,
    ) {
    }

    fn observe_epoch_entry(&self, _record: &PruneEpochEntryRecord) {}

    fn observe_epoch_residency(&self, _record: &PruneEpochResidencyRecord) {}
}

/// Wall-clock milliseconds since the Unix epoch (0 on a clock error).
fn now_millis() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

/// Wall-clock milliseconds since the Unix epoch as `i64` (0 on a clock error), for the
/// residency ledger's `*_unix_ms: i64` fields (PROJECT.md's timestamp row).
fn now_millis_i64() -> i64 {
    i64::try_from(now_millis()).unwrap_or(i64::MAX)
}

/// Render one prune-decision corpus row as `fixture | epochs drained | tags dropped`.
///
/// The corpus fixtures assert their own outcomes in prose-shaped `assert_eq!` messages, which
/// cannot be compared mechanically between two builds of the tree. This renders the same outcome
/// as one stable, sorted, machine-readable line so a prune-authority change can be shown to move
/// no decision. It READS the drained vector and formats it — it decides nothing.
#[cfg(test)]
fn prune_decision_line(fixture: &str, drained: &[(Epoch, TombstoneRef)]) -> String {
    let mut epochs: Vec<Epoch> = drained.iter().map(|(e, _)| *e).collect();
    epochs.sort_unstable();
    epochs.dedup();
    let mut tags: Vec<&str> = drained.iter().map(|(_, r)| r.tag.as_str()).collect();
    tags.sort_unstable();
    let epochs = epochs
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "PRUNE-DECISION | {fixture} | epochs=[{epochs}] | tags=[{}]",
        tags.join(",")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use metrics_exporter_prometheus::PrometheusBuilder;

    const CONN_A: ConnectionId = ConnectionId(1);
    const CONN_B: ConnectionId = ConnectionId(2);

    fn frontier() -> TombstoneFrontier {
        // No persistence: exercises the in-memory advance logic. The global bound is
        // inert (u64::MAX) so `delivered_conn` is the operative clamp, matching Wave 2.
        TombstoneFrontier::new(None)
    }

    // -----------------------------------------------------------------------
    // Prune record — arming parse and eager registration
    // -----------------------------------------------------------------------

    /// The 22 pinned counters (15 pre-existing + the 7 this half adds, R2.1 / `AC2a`).
    const PRUNE_COUNTER_NAMES: [&str; 22] = [
        METRIC_PRUNE_PASSES_TOTAL,
        METRIC_PRUNE_CONSIDERED_TOTAL,
        METRIC_PRUNE_DROPPED_TOTAL,
        METRIC_PRUNE_MATCHED_NOTHING_TOTAL,
        METRIC_PRUNE_ABSENT_TOTAL,
        METRIC_PRUNE_RESTORED_READ_ERROR_TOTAL,
        METRIC_PRUNE_RESTORED_EVICTED_TOTAL,
        METRIC_PRUNE_RESTORED_WRITE_ERROR_TOTAL,
        METRIC_PRUNE_BYTES_FREED_TOTAL,
        METRIC_PRUNE_EPOCHS_DRAINED_TOTAL,
        METRIC_PRUNE_EMPTY_DRAINS_TOTAL,
        METRIC_PRUNE_NONEMPTY_DRAINS_TOTAL,
        METRIC_PRUNE_LWM_ADVANCES_TOTAL,
        METRIC_PRUNE_LWM_EPOCHS_ADVANCED_TOTAL,
        METRIC_PRUNE_SPLIT_RECOMPUTES_TOTAL,
        METRIC_PRUNE_STAMPED_REFS_TOTAL,
        METRIC_PRUNE_STAMPED_BYTES_TOTAL,
        METRIC_PRUNE_DRAINED_REFS_TOTAL,
        METRIC_PRUNE_RESTORED_REFS_TOTAL,
        METRIC_PRUNE_REBUILD_CLEARED_REFS_TOTAL,
        METRIC_PRUNE_EPOCHS_ENTERED_TOTAL,
        METRIC_PRUNE_EPOCHS_EXITED_TOTAL,
    ];

    /// The 11 pinned gauges.
    const PRUNE_GAUGE_NAMES: [&str; 11] = [
        METRIC_PRUNE_INDEXED_REFS,
        METRIC_PRUNE_INDEXED_EPOCHS,
        METRIC_PRUNE_ELIGIBLE_REFS,
        METRIC_PRUNE_INELIGIBLE_REFS,
        METRIC_PRUNE_SPLIT_COMPUTED_EPOCH,
        METRIC_PRUNE_CURRENT_EPOCH,
        METRIC_PRUNE_LOW_WATER_MARK,
        METRIC_PRUNE_DURABLE_EPOCH_WATERMARK,
        METRIC_PRUNE_LAST_DRAINED_EPOCH,
        METRIC_PRUNE_LWM_STALL_SECONDS,
        METRIC_PRUNE_TRACKED_CLAIMS,
    ];

    /// The 7 pinned histograms.
    const PRUNE_HISTOGRAM_NAMES: [&str; 7] = [
        METRIC_PRUNE_DRAIN_REFS,
        METRIC_PRUNE_DRAIN_EPOCHS,
        METRIC_PRUNE_CLAIM_SPAN_EPOCHS,
        METRIC_PRUNE_CLAIM_LAG_EPOCHS,
        METRIC_PRUNE_EPOCH_CONSIDERED,
        METRIC_PRUNE_EPOCH_DROPPED,
        METRIC_PRUNE_EPOCH_BYTES_FREED,
    ];

    /// The rendered value of the bare series `name`, if the render carries that line.
    ///
    /// The space is load-bearing: it is what stops `topgun_or_prune_epoch_considered` from
    /// matching its own `_sum` line, and what excludes the quantile lines (which continue with
    /// `{`) from being read as the series value.
    fn rendered_value<'a>(rendered: &'a str, name: &str) -> Option<&'a str> {
        rendered.lines().find_map(|line| {
            line.strip_prefix(name)
                .and_then(|rest| rest.strip_prefix(' '))
        })
    }

    /// Only an explicit falsey word disarms the prune record; everything else stays armed,
    /// including a typo, because a run that is silently disarmed produces a whole empty
    /// measurement window before anyone notices.
    #[test]
    fn prune_record_arming_disarms_only_on_an_explicit_falsey_word() {
        for raw in ["false", "0", "no", "off", "FALSE", "Off", "  no  "] {
            assert_eq!(
                parse_prune_record_arming(raw),
                PruneRecordArming::Disarmed,
                "{raw:?} is an explicit falsey word and must disarm"
            );
        }
        for raw in [
            "true",
            "1",
            "yes",
            "on",
            "TRUE",
            "",
            "flase",
            "\"false\"",
            "2",
        ] {
            assert_eq!(
                parse_prune_record_arming(raw),
                PruneRecordArming::Armed,
                "{raw:?} is not an explicit falsey word and must stay armed"
            );
        }
    }

    /// Eager registration: with a recorder bound FIRST and **no** observations taken at all,
    /// the very first render already carries every pinned series — the 15 counters at `0`, the
    /// 11 gauges at `0`, and each of the 7 histograms rendering both `_sum` and `_count`.
    ///
    /// This is what makes an absent series unrepresentable, and therefore what stops a
    /// downstream sampler from ever having to distinguish "has not moved" from "does not
    /// exist". A name missing from the render is a defect in the recorder, not in this test.
    #[test]
    fn every_pinned_prune_series_is_present_in_the_first_render() {
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        // Bind the recorder BEFORE resolving a single handle. A handle resolved first would
        // bind to a no-op for its whole lifetime, and the inverted order would quietly turn
        // this test into an assertion about nothing.
        metrics::with_local_recorder(&recorder, || {
            let _observer = MetricsPruneRecorder::new();
            // Deliberately no observations.
        });

        let rendered = handle.render();
        // Emitted so `--nocapture` yields the first-scrape render itself as an artifact: the
        // evidence for eager registration is the rendered bytes, not a claim about them.
        println!("--- FIRST SCRAPE (no observations) ---\n{rendered}");

        for name in PRUNE_COUNTER_NAMES {
            assert_eq!(
                rendered_value(&rendered, name),
                Some("0"),
                "counter {name} must render at 0 in the first scrape; render was:\n{rendered}"
            );
        }
        for name in PRUNE_GAUGE_NAMES {
            assert_eq!(
                rendered_value(&rendered, name),
                Some("0"),
                "gauge {name} must render at 0 in the first scrape; render was:\n{rendered}"
            );
        }
        for name in PRUNE_HISTOGRAM_NAMES {
            let sum = format!("{name}_sum");
            let count = format!("{name}_count");
            assert_eq!(
                rendered_value(&rendered, &sum),
                Some("0"),
                "histogram {name} must render {sum} in the first scrape; render was:\n{rendered}"
            );
            assert_eq!(
                rendered_value(&rendered, &count),
                Some("0"),
                "histogram {name} must render {count} in the first scrape; render was:\n{rendered}"
            );
        }
    }

    /// The disarmed observer registers nothing and emits nothing: after constructing it and
    /// calling every method, no series under the prune-record prefix exists at all. That
    /// absence is what makes the effective arming provable from a scrape rather than from a
    /// boot line nobody reads.
    #[test]
    fn the_disarmed_observer_registers_no_series() {
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        metrics::with_local_recorder(&recorder, || {
            let observer = NullPruneRecorder;
            observer.observe_pass(&PrunePassRecord {
                considered: 7,
                dropped: 3,
                ..PrunePassRecord::default()
            });
            observer.observe_drained_epoch(&PruneEpochRecord {
                epoch: 4,
                considered: 7,
                ..PruneEpochRecord::default()
            });
            observer.observe_claim_span(&PruneClaimSpanRecord::default(), &[1, 2, 3]);
            observer.observe_lwm_advance(5);
            observer.observe_lwm_stall(11);
            observer.observe_index_state(9, 2);
            observer.observe_epoch_state(4, 1, 3, 2);
            observer.observe_eligibility_split(6, 1, 4);
            observer.observe_epoch_entry(&PruneEpochEntryRecord {
                epoch: 4,
                entered_index: true,
                stamped_refs: 3,
                ..PruneEpochEntryRecord::default()
            });
            observer.observe_epoch_residency(&PruneEpochResidencyRecord {
                epoch: 4,
                exit_kind: EpochExitKind::DrainedByPrune,
                ..PruneEpochResidencyRecord::default()
            });
        });

        let rendered = handle.render();
        for name in PRUNE_COUNTER_NAMES
            .iter()
            .chain(PRUNE_GAUGE_NAMES.iter())
            .chain(PRUNE_HISTOGRAM_NAMES.iter())
        {
            assert!(
                !rendered.contains(name),
                "the disarmed observer must register no series, but {name} is present; \
                 render was:\n{rendered}"
            );
        }
    }

    // -----------------------------------------------------------------------
    // Prune record — the frontier observation call sites
    // -----------------------------------------------------------------------

    /// Bind a fresh Prometheus recorder, run `body` under it, and return the render.
    ///
    /// Binding BEFORE `body` is load-bearing rather than stylistic: the frontier
    /// resolves its metric handles at construction, and a handle resolved before a
    /// recorder is bound binds to a no-op for its whole lifetime — the inverted order
    /// would silently turn every assertion below into an assertion about nothing.
    fn rendered_under_a_recorder(body: impl FnOnce()) -> String {
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        metrics::with_local_recorder(&recorder, body);
        handle.render()
    }

    /// Drive an async frontier call from a synchronous test body.
    ///
    /// `metrics::with_local_recorder` binds a THREAD-LOCAL recorder and takes a
    /// synchronous closure, so a `#[tokio::test]` body cannot be wrapped in one. A
    /// current-thread runtime driven from inside the binding keeps the recorder
    /// visible to everything the call touches.
    fn block_on<F: std::future::Future>(fut: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("current-thread runtime")
            .block_on(fut)
    }

    /// The index gauges are maintained incrementally at stamp / drain / restore, and
    /// the carried count agrees with what a fold over the index would produce.
    ///
    /// The fold is the reference implementation and it lives HERE, in a test, which is
    /// the whole point: on the hot path it would be work proportional to the index on
    /// every `OR_REMOVE`, and the carried count exists so that fold never has to run
    /// in production.
    #[test]
    fn index_state_gauges_track_the_index_without_folding_it() {
        let mut folded = 0;
        let rendered = rendered_under_a_recorder(|| {
            let f = frontier();
            f.set_epoch_width(1);
            for k in ["k1", "k2", "k3"] {
                f.stamp_tombstone("m", k, "t");
            }
            // Epochs 1 and 2 are eligible (watermark 2, LWM 3), epoch 3 is not.
            f.set_durable_epoch_watermark(2);
            f.set_delivered(CONN_A, 100);
            let c: ClientId = "a5:alice|dev-1".into();
            assert!(block_on(f.confirm_apply_ack(&c, 3, CONN_A)));

            let drained = f.drain_prunable_tombstones();
            assert_eq!(drained.len(), 2, "epochs 1 and 2 drain, epoch 3 does not");
            // One drop failed: the ref comes back and the index must follow it back up.
            let (epoch, tombstone_ref) = drained.into_iter().next().expect("a drained ref");
            f.restore_tombstone_ref(epoch, tombstone_ref);

            let state = f.lock();
            folded = state.epoch_tags.values().map(Vec::len).sum::<usize>();
            assert_eq!(
                u64::try_from(folded).unwrap(),
                state.indexed_refs,
                "the carried ref count must equal the fold it exists to avoid"
            );
        });

        assert_eq!(folded, 2, "one epoch-3 ref plus the restored one");
        assert_eq!(
            rendered_value(&rendered, METRIC_PRUNE_INDEXED_REFS),
            Some("2"),
            "render was:\n{rendered}"
        );
        assert_eq!(
            rendered_value(&rendered, METRIC_PRUNE_INDEXED_EPOCHS),
            Some("2"),
            "render was:\n{rendered}"
        );
        assert_eq!(
            rendered_value(&rendered, METRIC_PRUNE_LAST_DRAINED_EPOCH),
            Some("2"),
            "render was:\n{rendered}"
        );
    }

    // -----------------------------------------------------------------------
    // Per-epoch residency ledger — entry / exit emissions, O-0's accessor, and
    // the `Unclassified` detection path (AC6a)
    // -----------------------------------------------------------------------

    /// The ENTRY row fires once, at ROLLOVER — never per stamp — and the EXIT row
    /// fires once a drain actually removes the epoch, attributed
    /// `DrainedByPrune`. Both feed their respective metrics-side completeness
    /// counters exactly once each.
    #[test]
    fn epoch_entry_fires_at_rollover_and_exit_fires_on_drain() {
        let rendered = rendered_under_a_recorder(|| {
            let f = frontier();
            f.set_epoch_width(1);
            // k1 stamps epoch 1; k2's stamp is what rolls the clock PAST epoch 1 and
            // fires its entry row. A third stamp rolls past epoch 2 too, so both
            // entry rows exist before the drain runs.
            f.stamp_tombstone("m", "k1", "TAG1");
            f.stamp_tombstone("m", "k2", "TAG2");
            f.stamp_tombstone("m", "k3", "TAG3");

            // License epoch 1 for the drain: watermark >= 1 and LWM > 1.
            f.set_durable_epoch_watermark(1);
            f.set_delivered(CONN_A, 100);
            let c: ClientId = "a5:alice|dev-1".into();
            assert!(block_on(f.confirm_apply_ack(&c, 2, CONN_A)));

            let drained = f.drain_prunable_tombstones();
            assert_eq!(drained.len(), 1, "only epoch 1 clears both conjuncts");
        });

        assert_eq!(
            rendered_value(&rendered, METRIC_PRUNE_EPOCHS_ENTERED_TOTAL),
            Some("2"),
            "epochs 1 and 2 both rolled over; render was:\n{rendered}"
        );
        assert_eq!(
            rendered_value(&rendered, METRIC_PRUNE_STAMPED_REFS_TOTAL),
            Some("2"),
            "credited once per entry row, at rollover; render was:\n{rendered}"
        );
        assert_eq!(
            rendered_value(&rendered, METRIC_PRUNE_STAMPED_BYTES_TOTAL),
            Some("8"),
            "TAG1 + TAG2 = 4 + 4 bytes; render was:\n{rendered}"
        );
        assert_eq!(
            rendered_value(&rendered, METRIC_PRUNE_EPOCHS_EXITED_TOTAL),
            Some("1"),
            "only epoch 1 drained; render was:\n{rendered}"
        );
        assert_eq!(
            rendered_value(&rendered, METRIC_PRUNE_DRAINED_REFS_TOTAL),
            Some("1"),
            "epoch 1 held exactly one ref; render was:\n{rendered}"
        );
    }

    /// O-0's accessor: the eight quantities are read as one internally-coherent
    /// tuple (two calls with no intervening mutation agree BY CONSTRUCTION), and
    /// the conservation identity holds across a stamp → drain → restore sequence.
    #[test]
    fn index_conservation_snapshot_is_coherent_and_the_identity_holds() {
        let f = frontier();
        f.set_epoch_width(1);
        f.stamp_tombstone("m", "k1", "T1");
        f.stamp_tombstone("m", "k2", "T2");
        f.stamp_tombstone("m", "k3", "T3"); // rolls past epochs 1 and 2

        // Two renders of the SAME snapshot, no mutation between them: equal by
        // construction (a single locked struct copy, not two independent reads).
        let a = f.index_conservation_snapshot();
        let b = f.index_conservation_snapshot();
        assert_eq!(
            a, b,
            "two renders of one snapshot must agree by construction"
        );

        f.set_durable_epoch_watermark(2);
        f.set_delivered(CONN_A, 100);
        let c: ClientId = "a5:alice|dev-1".into();
        assert!(block_on(f.confirm_apply_ack(&c, 3, CONN_A)));
        let drained = f.drain_prunable_tombstones();
        assert_eq!(drained.len(), 2, "epochs 1 and 2 drain");
        let (epoch, tombstone_ref) = drained.into_iter().next().expect("a drained ref");
        f.restore_tombstone_ref(epoch, tombstone_ref);

        let snap = f.index_conservation_snapshot();
        assert_eq!(
            snap.stamped_refs_total + snap.restored_refs_total
                - snap.drained_refs_total
                - snap.rebuild_cleared_refs_total,
            snap.indexed_refs,
            "O-0: stamped + restored - drained - rebuild_cleared == indexed_refs, got {snap:?}"
        );
    }

    /// `AC6a` — the `Unclassified` escape is REACHABLE, not merely declared. An
    /// epoch's slot is removed directly, bypassing `drain_prunable` /
    /// `rebuild_into_epoch` / `restore` entirely; the next detection point (a
    /// drain pass, even one that eligibility-drains nothing else) surfaces
    /// exactly one exit row with `exit_kind == Unclassified` and every
    /// `observed_*` field populated.
    #[test]
    fn unenumerated_epoch_removal_surfaces_as_unclassified_at_the_next_detection_point() {
        let f = frontier();
        f.set_epoch_width(1);
        f.stamp_tombstone("m", "k1", "TAG1"); // epoch 1
        f.stamp_tombstone("m", "k2", "TAG2"); // rolls past epoch 1: entry row fires

        // Remove epoch 1's slot directly — NOT through drain_prunable,
        // rebuild_into_epoch or restore. This is the unenumerated path R2.3b's
        // escape variant exists for.
        {
            let mut state = f.lock();
            assert!(
                state.epoch_tags.remove(&1).is_some(),
                "epoch 1 must be resident before the bypass removal"
            );
        }

        // Any subsequent detection point picks it up. A nonzero watermark alone
        // is enough to arm the sweep — no eligibility is required.
        f.set_durable_epoch_watermark(1);
        let drained = f.drain_prunable_tombstones();
        assert!(
            drained.is_empty(),
            "nothing legitimately eligible this pass; epoch 1 was already gone"
        );

        // Assert on the record content directly against FrontierState, since
        // `exit_kind`'s raw context is not representable over the metrics
        // transport (that is the whole reason the residency ledger is a
        // `tracing` line and not a metric, R2.2).
        let f2 = frontier();
        f2.set_epoch_width(1);
        f2.stamp_tombstone("m", "k1", "TAG1");
        f2.stamp_tombstone("m", "k2", "TAG2");
        let record = {
            let mut state = f2.lock();
            state.epoch_tags.remove(&1);
            state.detect_epoch_exit(1, None, None)
        }
        .expect("the bypassed epoch must surface an exit row at the next detection point");
        match record.exit_kind {
            EpochExitKind::Unclassified {
                observed_refs_delta,
                observed_lwm,
                observed_durable_watermark,
                observed_current_epoch,
                ref note,
            } => {
                assert_eq!(
                    observed_refs_delta, -1,
                    "one ref vanished with no attributable removal"
                );
                // Every `observed_*` field is POPULATED (not a default sentinel
                // masquerading as absent) — the record carries real state.
                assert_eq!(observed_lwm, 0);
                assert_eq!(observed_durable_watermark, 0);
                assert_eq!(observed_current_epoch, 2);
                assert!(!note.is_empty(), "the breadcrumb must be non-empty");
            }
            other => panic!("expected Unclassified, got {other:?}"),
        }
    }

    /// THE REPRODUCING TEST (R7.3(a), AC12, Q11). The named mechanism: a
    /// `DrainedByPrune` exit's `bytes_freed_attributed` is EXACTLY the epoch's
    /// own stamped byte total — never a placeholder, never zero, never an
    /// approximation. This is the exact accounting step T2(exactness) exists
    /// to make observable, and reproduces the precise historical symptom the
    /// prior lineage's committed record names: `bytes_freed_attributed` empty
    /// on every one of 415/447 epoch rows over the deciding window. The
    /// mechanism reproduces entirely at the `FrontierState` boundary — no
    /// service composition and no interleaving-fault scenario is needed to
    /// exhibit it — so R7.3(a) sites this test in counted file 2's own inline
    /// test module. Its non-vacuity is proven by the committed mutation arm
    /// (`spec357-reproducer-mutation.patch` / `.txt`): the production edit that
    /// zeros `bytes_freed_attributed` unconditionally makes this exact
    /// assertion false in a throwaway worktree.
    #[test]
    fn drained_epoch_exit_attributes_exactly_its_own_stamped_byte_total() {
        let f = frontier();
        f.set_epoch_width(1);
        let tag = "REPRODUCER-TOMBSTONE-TAG";
        let expected_bytes = u64::try_from(tag.len()).expect("fits u64");
        {
            let mut state = f.lock();
            let (_epoch, rec0) = state.stamp_tombstone("m", "krepro1", tag, 0);
            assert!(rec0.is_none(), "first stamp never rolls anything over");
            let (_epoch, rec1) = state.stamp_tombstone("m", "krepro2", "FILLER", 0);
            assert!(rec1.is_some(), "second stamp rolls past epoch 1");
        }
        f.set_durable_epoch_watermark(1);
        f.set_delivered(CONN_A, 100);
        let c: ClientId = "a5:repro-t2|dev-1".into();
        assert!(block_on(f.confirm_apply_ack(&c, 2, CONN_A)));
        // The residency interval is HALF-OPEN, `[entered_at_op_seq,
        // exited_at_op_seq)` (R3.2): one more (discarded) stamp advances the
        // clock strictly past the licensing instant before the drain runs, so
        // T(e) is a genuinely non-empty overlap (same reason as the (d)
        // DRAINED-HEALTHY Tier-1 scenario below).
        {
            let mut state = f.lock();
            state.stamp_tombstone("m", "krepro-spacer", "SPACERTAG", 0);
        }
        let exits = {
            let mut state = f.lock();
            let (_drained, _split, exits) = state.drain_prunable();
            exits
        };
        assert_eq!(exits.len(), 1, "only epoch 1 was eligible");
        let exit = exits.into_iter().next().expect("checked len == 1");
        assert!(matches!(exit.exit_kind, EpochExitKind::DrainedByPrune));
        assert_eq!(
            exit.bytes_freed_attributed,
            expected_bytes,
            "a DrainedByPrune exit must attribute EXACTLY the epoch's own \
             stamped byte total, got {got}",
            got = exit.bytes_freed_attributed
        );
    }

    // -----------------------------------------------------------------------
    // Tier-1 deterministic discrimination — the `FrontierState` arm (R7.1/
    // R7.1a). Five independent, engineered scenarios drive every class of the
    // frozen §11.0 walk this boundary can reach, feed the resulting
    // entry/residency population through the COMMITTED `spec357-classify.sh`
    // builder (this is what makes it "the frozen walk run against harness
    // output" rather than an eyeballed hand-classification), and assert the
    // one-row-per-class result. Class (a) INDEX-POPULATION-GAP is not
    // exercised here: this file's `stamp_tombstone` only ever constructs an
    // `entered_index == false` entry row via `PruneEpochEntryRecord::default()`
    // (control class (f)'s branch), which is `stamped_bytes == 0` by
    // construction — (a) needs `stamped_bytes > 0` on that same branch, which
    // this implementation cannot produce. That is not a harness gap; it is the
    // mechanical form of R3.1's own expectation ("EXCLUDED" under the pin's
    // weakening), and the transcript states it as such rather than leaving a
    // silent absence.
    // -----------------------------------------------------------------------

    /// Epoch-offset an entry row so five independently-numbered scenarios (each
    /// its own tiny frontier starting at epoch 1) can share one JSONL
    /// population with no epoch collision. Only the JSON payload's `epoch`
    /// field is remapped; the frontier's own internal numbering is untouched.
    fn offset_entry(mut r: PruneEpochEntryRecord, base: Epoch) -> PruneEpochEntryRecord {
        r.epoch += base;
        r
    }

    /// See [`offset_entry`] — the residency-side counterpart, applied with the
    /// SAME base as its scenario's entry row so the classify.sh join still
    /// resolves by epoch number.
    fn offset_residency(
        mut r: PruneEpochResidencyRecord,
        base: Epoch,
    ) -> PruneEpochResidencyRecord {
        r.epoch += base;
        r
    }

    /// One O-0 conservation-ledger row, both renders identical (a single
    /// `index_conservation_snapshot()` call written twice) — the accessor's own
    /// coherence property (Checklist 6a) is what licenses this, not an
    /// assumption.
    fn conservation_csv_row(elapsed_secs: u64, s: IndexConservationSnapshot) -> String {
        format!(
            "{elapsed_secs},{a},{a},{b},{b},{c},{c},{d},{d},{e},{e},{f},{f},{g},{g},{h},{h}",
            a = s.stamped_refs_total,
            b = s.stamped_bytes_total,
            c = s.drained_refs_total,
            d = s.restored_refs_total,
            e = s.rebuild_cleared_refs_total,
            f = s.epochs_entered_total,
            g = s.epochs_exited_total,
            h = s.indexed_refs,
        )
    }

    /// Dry-run leg 2 (R6.3) — exercises the committed `spec357-classify.sh`
    /// builder over this Tier-1 harness's own output and asserts the frozen
    /// walk's per-class result. `--nocapture` prints both produced CSVs
    /// verbatim so their bytes can be transcribed into the committed
    /// `spec357-dryrun-{class,conservation}-tier1.csv` evidence artifacts.
    // Kept as one body deliberately: the five engineered scenarios share the
    // `entries`/`residency` accumulators and the single classify.sh
    // invocation they all feed, so splitting them into helper fns would move
    // the per-class assertions away from the exact fixture that produced
    // each row, which is the property a reviewer needs to check this test
    // against the frozen walk at all.
    #[allow(clippy::too_many_lines)]
    #[test]
    fn tier1_frontierstate_arm_drives_every_reachable_class_of_the_frozen_walk() {
        const D_BASE: Epoch = 100_000; // (d) DRAINED-HEALTHY
        const B_BASE: Epoch = 200_000; // (b) SELECTION / SPLIT MISMATCH
        const C_BASE: Epoch = 300_000; // (c) FRONTIER RACE (+ Unclassified)
        const E_BASE: Epoch = 400_000; // (e) NO-EXIT-RECORD
        const F_BASE: Epoch = 500_000; // (f) EMPTY-EPOCH

        let mut entries: Vec<PruneEpochEntryRecord> = Vec::new();
        let mut residency: Vec<PruneEpochResidencyRecord> = Vec::new();

        // ---- (d) DRAINED-HEALTHY: resident, licensed, fenced, THEN actually
        // taken by a legitimate prune drain — T(e) holds and D(e) holds.
        {
            let f = frontier();
            f.set_epoch_width(1);
            let e1 = {
                let mut state = f.lock();
                let (_epoch, rec0) = state.stamp_tombstone("m", "kd1", "TAGD1", 0);
                assert!(rec0.is_none(), "first stamp never rolls anything over");
                let (_epoch, rec1) = state.stamp_tombstone("m", "kd2", "TAGD2", 0);
                rec1.expect("second stamp rolls past epoch 1")
            };
            f.set_durable_epoch_watermark(1);
            f.set_delivered(CONN_A, 100);
            let c: ClientId = "a5:tier1-d|dev-1".into();
            assert!(block_on(f.confirm_apply_ack(&c, 2, CONN_A)));
            // The residency interval is HALF-OPEN, `[entered_at_op_seq,
            // exited_at_op_seq)` (R3.2): draining in the SAME op-seq tick that
            // licensing landed on would make `exited_at_op_seq` equal to (not
            // strictly greater than) `lwm_passed_at_op_seq`, which is an EMPTY
            // overlap under that definition, not a non-empty one — so one more
            // (discarded) stamp advances the clock past the licensing instant
            // before the drain runs, giving T(e) a genuinely non-empty window.
            {
                let mut state = f.lock();
                state.stamp_tombstone("m", "kd-spacer", "TAGDX", 0);
            }
            let exits = {
                let mut state = f.lock();
                let (_drained, _split, exits) = state.drain_prunable();
                exits
            };
            assert_eq!(exits.len(), 1, "only epoch 1 was eligible");
            let exit = exits.into_iter().next().expect("checked len == 1");
            assert!(matches!(exit.exit_kind, EpochExitKind::DrainedByPrune));
            entries.push(offset_entry(e1, D_BASE));
            residency.push(offset_residency(exit, D_BASE));
        }

        // ---- (b) SELECTION / SPLIT MISMATCH: licensed AND fenced, then
        // removed by a REBUILD instead of a legitimate drain — T(e) holds,
        // D(e) does not: "eligible and not taken by the prune" (R3.2).
        {
            let f = frontier();
            f.set_epoch_width(1);
            let e1 = {
                let mut state = f.lock();
                let (_epoch, rec0) = state.stamp_tombstone("m", "kb1", "TAGB1", 0);
                assert!(rec0.is_none());
                let (_epoch, rec1) = state.stamp_tombstone("m", "kb2", "TAGB2", 0);
                rec1.expect("second stamp rolls past epoch 1")
            };
            f.set_durable_epoch_watermark(1);
            f.set_delivered(CONN_A, 100);
            let c: ClientId = "a5:tier1-b|dev-1".into();
            assert!(block_on(f.confirm_apply_ack(&c, 2, CONN_A)));
            let exits = {
                let mut state = f.lock();
                state.rebuild_into_epoch(100, Vec::new())
            };
            assert_eq!(exits.len(), 1, "only epoch 1 had an entry-emitted slot");
            let exit = exits.into_iter().next().expect("checked len == 1");
            assert!(matches!(exit.exit_kind, EpochExitKind::ClearedByRebuild));
            entries.push(offset_entry(e1, B_BASE));
            residency.push(offset_residency(exit, B_BASE));
        }

        // ---- (c) FRONTIER RACE: no licensing, no fencing at all — T(e) is
        // empty by construction. Removed via the unenumerated bypass (same
        // mechanism AC6a's dedicated test proves), which is ALSO how this
        // scenario doubles as an aggregate-population `Unclassified` data
        // point distinct from AC6a's own dedicated, single-purpose test.
        {
            let f = frontier();
            f.set_epoch_width(1);
            let e1 = {
                let mut state = f.lock();
                let (_epoch, rec0) = state.stamp_tombstone("m", "kc1", "TAGC1", 0);
                assert!(rec0.is_none());
                let (_epoch, rec1) = state.stamp_tombstone("m", "kc2", "TAGC2", 0);
                rec1.expect("second stamp rolls past epoch 1")
            };
            let exit = {
                let mut state = f.lock();
                state.epoch_tags.remove(&1);
                state.detect_epoch_exit(1, None, None)
            }
            .expect("bypassed epoch surfaces at the next detection point");
            assert!(matches!(exit.exit_kind, EpochExitKind::Unclassified { .. }));
            entries.push(offset_entry(e1, C_BASE));
            residency.push(offset_residency(exit, C_BASE));
        }

        // ---- (e) NO-EXIT-RECORD: entered, then deliberately left alone — no
        // drain, no rebuild, no detection sweep. Still resident with an entry
        // row and no exit row at Tier-1 snapshot time.
        {
            let f = frontier();
            f.set_epoch_width(1);
            let e1 = {
                let mut state = f.lock();
                let (_epoch, rec0) = state.stamp_tombstone("m", "ke1", "TAGE1", 0);
                assert!(rec0.is_none());
                let (_epoch, rec1) = state.stamp_tombstone("m", "ke2", "TAGE2", 0);
                rec1.expect("second stamp rolls past epoch 1")
            };
            entries.push(offset_entry(e1, E_BASE));
        }

        // ---- (f) EMPTY-EPOCH — the D3 obligation carried into this segment.
        // A rebuild jump leaves the landing epoch with no residency slot; the
        // next rollover past it therefore takes the
        // `PruneEpochEntryRecord::default()` branch (R2.3a), exactly the path
        // documented at the `stamp_tombstone` rollover site: "only reachable
        // via an `epoch_width` change or a rebuild-induced jump, never via
        // ordinary sequential stamping."
        {
            let f = frontier();
            f.set_epoch_width(1);
            {
                let mut state = f.lock();
                state.rebuild_into_epoch(50, Vec::new());
            }
            let e_empty = {
                let mut state = f.lock();
                let (_epoch, rec) = state.stamp_tombstone("m", "kf1", "TAGF1", 0);
                rec.expect("rolls past the empty landing epoch 50")
            };
            assert!(
                !e_empty.entered_index,
                "class (f): the landing epoch never entered the index"
            );
            assert_eq!(e_empty.stamped_bytes, 0);
            assert_eq!(e_empty.epoch, 50);
            entries.push(offset_entry(e_empty, F_BASE));
        }

        assert_eq!(entries.len(), 5, "one entry row per engineered scenario");
        assert_eq!(
            residency.len(),
            3,
            "(d), (b) and (c) each produced an exit row; (e) and (f) did not, by design"
        );

        // ---- O-0 conservation-ledger rows for the synthetic prune.csv, over
        // an unrelated sixth frontier: stamp → drain → restore, each snapshot
        // read twice into identical `_a`/`_b` renders (Checklist 6a's own
        // coherence property licenses this).
        let cons_rows = {
            let f = frontier();
            f.set_epoch_width(1);
            let mut rows = Vec::new();
            f.stamp_tombstone("m", "ko1", "T1");
            f.stamp_tombstone("m", "ko2", "T2");
            f.stamp_tombstone("m", "ko3", "T3"); // rolls past epochs 1 and 2
            rows.push(conservation_csv_row(0, f.index_conservation_snapshot()));
            f.set_durable_epoch_watermark(2);
            f.set_delivered(CONN_A, 100);
            let c: ClientId = "a5:tier1-cons|dev-1".into();
            assert!(block_on(f.confirm_apply_ack(&c, 3, CONN_A)));
            let drained = f.drain_prunable_tombstones();
            assert_eq!(drained.len(), 2);
            rows.push(conservation_csv_row(10, f.index_conservation_snapshot()));
            let (epoch, tombstone_ref) = drained.into_iter().next().expect("a drained ref");
            f.restore_tombstone_ref(epoch, tombstone_ref);
            rows.push(conservation_csv_row(20, f.index_conservation_snapshot()));
            rows
        };

        // ---- Write the harness output in the exact wire shape
        // `spec357-classify.sh` documents, run the builder over it (this IS
        // "the frozen walk run against harness output" — the walk lives in
        // the committed builder, not re-derived by eye here), and assert the
        // per-class result.
        let dir = tempfile::tempdir().expect("tempdir");
        let entry_path = dir.path().join("tier1.entry.jsonl");
        let residency_path = dir.path().join("tier1.residency.jsonl");
        let prune_path = dir.path().join("tier1.prune.csv");
        let out_prefix = dir.path().join("tier1-dryrun");

        let entry_jsonl: String = entries
            .iter()
            .map(|r| serde_json::to_string(r).expect("entry row serializes"))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        let residency_jsonl: String = residency
            .iter()
            .map(|r| serde_json::to_string(r).expect("residency row serializes"))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        std::fs::write(&entry_path, &entry_jsonl).expect("write entry.jsonl");
        std::fs::write(&residency_path, &residency_jsonl).expect("write residency.jsonl");

        let m_refs = METRIC_PRUNE_STAMPED_REFS_TOTAL;
        let m_bytes = METRIC_PRUNE_STAMPED_BYTES_TOTAL;
        let m_drained = METRIC_PRUNE_DRAINED_REFS_TOTAL;
        let m_restored = METRIC_PRUNE_RESTORED_REFS_TOTAL;
        let m_rebuilt = METRIC_PRUNE_REBUILD_CLEARED_REFS_TOTAL;
        let m_entered = METRIC_PRUNE_EPOCHS_ENTERED_TOTAL;
        let m_exited = METRIC_PRUNE_EPOCHS_EXITED_TOTAL;
        let m_indexed = METRIC_PRUNE_INDEXED_REFS;
        let prune_header = format!(
            "elapsed_secs,{m_refs}_a,{m_refs}_b,{m_bytes}_a,{m_bytes}_b,{m_drained}_a,{m_drained}_b,\
             {m_restored}_a,{m_restored}_b,{m_rebuilt}_a,{m_rebuilt}_b,{m_entered}_a,{m_entered}_b,\
             {m_exited}_a,{m_exited}_b,{m_indexed}_a,{m_indexed}_b"
        );
        let prune_csv = format!("{prune_header}\n{}\n", cons_rows.join("\n"));
        std::fs::write(&prune_path, &prune_csv).expect("write prune.csv");

        println!("--- TIER-1 HARNESS: entry.jsonl ---\n{entry_jsonl}");
        println!("--- TIER-1 HARNESS: residency.jsonl ---\n{residency_jsonl}");
        println!("--- TIER-1 HARNESS: prune.csv ---\n{prune_csv}");

        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("benches/soak_harness/evidence/spec357-classify.sh");
        let output = std::process::Command::new("sh")
            .arg(&script)
            .arg(&entry_path)
            .arg(&residency_path)
            .arg(&prune_path)
            .arg(&out_prefix)
            .output()
            .expect("spec357-classify.sh runs");
        println!(
            "--- spec357-classify.sh stdout ---\n{}",
            String::from_utf8_lossy(&output.stdout)
        );
        println!(
            "--- spec357-classify.sh stderr ---\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.status.success(), "spec357-classify.sh must exit 0");

        let class_csv = std::fs::read_to_string(format!(
            "{}-class.csv",
            out_prefix.to_str().expect("utf8 path")
        ))
        .expect("read class.csv");
        let cons_csv = std::fs::read_to_string(format!(
            "{}-conservation.csv",
            out_prefix.to_str().expect("utf8 path")
        ))
        .expect("read conservation.csv");
        println!("--- TIER-1 HARNESS: spec357-*-class.csv ---\n{class_csv}");
        println!("--- TIER-1 HARNESS: spec357-*-conservation.csv ---\n{cons_csv}");

        let data_rows: Vec<&str> = class_csv
            .lines()
            .skip(1)
            .filter(|l| !l.is_empty())
            .collect();
        assert_eq!(data_rows.len(), 5, "one classification row per entry row");
        let count_col = |idx: usize, needle: &str| -> usize {
            data_rows
                .iter()
                .filter(|row| row.split(',').nth(idx) == Some(needle))
                .count()
        };
        // Columns: epoch,entered_index,stamped_bytes,refs_at_entry,has_exit_row,
        // exit_kind_variant,unclassified_exit,class_a,class_b,class_c,class_d,
        // class_e,class_f (0-indexed 0..12).
        assert_eq!(
            count_col(7, "1"),
            0,
            "class_a: mechanically unreachable, see this test's own doc-comment"
        );
        assert_eq!(count_col(8, "1"), 1, "class_b: exactly the (b) scenario");
        assert_eq!(count_col(9, "1"), 1, "class_c: exactly the (c) scenario");
        assert_eq!(count_col(10, "1"), 1, "class_d: exactly the (d) scenario");
        assert_eq!(count_col(11, "1"), 1, "class_e: exactly the (e) scenario");
        assert_eq!(
            count_col(12, "1"),
            1,
            "class_f: exactly the (f) scenario (D3)"
        );
        assert_eq!(
            data_rows
                .iter()
                .filter(|row| row.split(',').nth(6) == Some("1"))
                .count(),
            1,
            "unclassified_exit: exactly the (c) scenario's bypass removal"
        );

        let cons_data_rows: Vec<&str> =
            cons_csv.lines().skip(1).filter(|l| !l.is_empty()).collect();
        assert_eq!(
            cons_data_rows.len(),
            3,
            "one conservation row per prune.csv scrape row"
        );
        assert!(
            cons_data_rows.iter().all(|row| row.split(',').nth(1) == Some("CONSISTENT")),
            "no concurrent mutation between the two synthetic renders: every row is CONSISTENT, never TORN"
        );
        assert!(
            cons_data_rows
                .iter()
                .all(|row| row.split(',').nth(4) == Some("1")),
            "O-0 holds on every CONSISTENT scrape of this harness run"
        );
    }

    /// A low-water-mark advance publishes the advance cadence, the eligible /
    /// ineligible split, the split's staleness marker, and the claim span.
    #[test]
    fn an_lwm_advance_publishes_the_split_and_the_claim_span() {
        let rendered = rendered_under_a_recorder(|| {
            let f = frontier();
            f.set_epoch_width(1);
            // Injected BEFORE the stamps: the durability watermark is republished by
            // the stamp / drain / restore path, and production recomputes it from the
            // store on every drain — this test takes neither after injecting it.
            f.set_durable_epoch_watermark(2);
            for k in ["k1", "k2", "k3"] {
                f.stamp_tombstone("m", k, "t");
            }
            f.set_delivered(CONN_A, 100);
            let c: ClientId = "a5:alice|dev-1".into();
            assert!(block_on(f.confirm_apply_ack(&c, 2, CONN_A)));
        });

        // Watermark 2 and LWM 2: only epoch 1 clears BOTH conjuncts.
        for (name, value) in [
            (METRIC_PRUNE_ELIGIBLE_REFS, "1"),
            (METRIC_PRUNE_INELIGIBLE_REFS, "2"),
            // The staleness marker: the epoch the split was computed at.
            (METRIC_PRUNE_SPLIT_COMPUTED_EPOCH, "3"),
            (METRIC_PRUNE_CURRENT_EPOCH, "3"),
            (METRIC_PRUNE_LOW_WATER_MARK, "2"),
            (METRIC_PRUNE_DURABLE_EPOCH_WATERMARK, "2"),
            (METRIC_PRUNE_TRACKED_CLAIMS, "1"),
            // ... and its monotone companion: one recompute.
            (METRIC_PRUNE_SPLIT_RECOMPUTES_TOTAL, "1"),
            (METRIC_PRUNE_LWM_ADVANCES_TOTAL, "1"),
            (METRIC_PRUNE_LWM_EPOCHS_ADVANCED_TOTAL, "2"),
        ] {
            assert_eq!(
                rendered_value(&rendered, name),
                Some(value),
                "{name} must render {value}; render was:\n{rendered}"
            );
        }
        // The claim span is `current_epoch - low_water_mark` = 1, over one tracked
        // claim whose own lag is the same 1.
        for name in [
            METRIC_PRUNE_CLAIM_SPAN_EPOCHS,
            METRIC_PRUNE_CLAIM_LAG_EPOCHS,
        ] {
            assert_eq!(
                rendered_value(&rendered, &format!("{name}_sum")),
                Some("1"),
                "render was:\n{rendered}"
            );
            assert_eq!(
                rendered_value(&rendered, &format!("{name}_count")),
                Some("1"),
                "render was:\n{rendered}"
            );
        }
    }

    /// A stale split is DETECTABLE, and the recompute counter is what detects it.
    ///
    /// After the last recompute the split gauges keep rendering their old values
    /// while the corpus they describe keeps growing — read alone they say "not
    /// growing" precisely when they are least entitled to. The recompute counter is
    /// frozen alongside them, which is the signal that makes the sample inadmissible
    /// rather than reassuring.
    #[test]
    fn a_split_frozen_at_its_last_recompute_is_detectable_from_the_recompute_counter() {
        let rendered = rendered_under_a_recorder(|| {
            let f = frontier();
            f.set_epoch_width(1);
            f.stamp_tombstone("m", "k1", "t");
            f.set_durable_epoch_watermark(1);
            f.set_delivered(CONN_A, 100);
            let c: ClientId = "a5:alice|dev-1".into();
            assert!(block_on(f.confirm_apply_ack(&c, 1, CONN_A)));

            // The LWM stops moving; removes keep arriving. Every drain from here is
            // empty (the new epochs are all above the watermark), so nothing
            // recomputes the split.
            for k in ["k2", "k3", "k4"] {
                f.stamp_tombstone("m", k, "t");
                assert!(f.drain_prunable_tombstones().is_empty());
            }
        });

        assert_eq!(
            rendered_value(&rendered, METRIC_PRUNE_SPLIT_RECOMPUTES_TOTAL),
            Some("1"),
            "the split must NOT be recomputed per remove; render was:\n{rendered}"
        );
        assert_eq!(
            rendered_value(&rendered, METRIC_PRUNE_SPLIT_COMPUTED_EPOCH),
            Some("1"),
            "the marker must still name the epoch of the LAST recompute; \
             render was:\n{rendered}"
        );
        // The frozen split still claims one ineligible ref while the index actually
        // holds four — exactly the misreading the marker exists to expose.
        assert_eq!(
            rendered_value(&rendered, METRIC_PRUNE_INELIGIBLE_REFS),
            Some("1"),
            "render was:\n{rendered}"
        );
        assert_eq!(
            rendered_value(&rendered, METRIC_PRUNE_INDEXED_REFS),
            Some("4"),
            "the index gauge, unlike the split, IS refreshed per stamp; \
             render was:\n{rendered}"
        );
        assert_eq!(
            rendered_value(&rendered, METRIC_PRUNE_CURRENT_EPOCH),
            Some("4"),
            "render was:\n{rendered}"
        );
    }

    /// A NON-empty drain is the second recompute trigger, and it moves the marker.
    #[test]
    fn a_nonempty_drain_recomputes_the_split() {
        let rendered = rendered_under_a_recorder(|| {
            let f = frontier();
            f.set_epoch_width(1);
            f.stamp_tombstone("m", "k1", "t");
            f.stamp_tombstone("m", "k2", "t");
            f.set_durable_epoch_watermark(1);
            f.set_delivered(CONN_A, 100);
            let c: ClientId = "a5:alice|dev-1".into();
            // Recompute 1: the LWM advance.
            assert!(block_on(f.confirm_apply_ack(&c, 2, CONN_A)));
            // Recompute 2: the non-empty drain.
            assert_eq!(f.drain_prunable_tombstones().len(), 1);
            // Still 2: an empty drain recomputes nothing.
            assert!(f.drain_prunable_tombstones().is_empty());
        });

        assert_eq!(
            rendered_value(&rendered, METRIC_PRUNE_SPLIT_RECOMPUTES_TOTAL),
            Some("2"),
            "render was:\n{rendered}"
        );
        assert_eq!(
            rendered_value(&rendered, METRIC_PRUNE_LAST_DRAINED_EPOCH),
            Some("1"),
            "render was:\n{rendered}"
        );
        // The split published by a drain is the backlog the drain was licensed to
        // take, read on the near side of taking it — epoch 1's single ref. Reading it
        // on the far side would render 0 here and at every other drain, because this
        // drain is unbounded.
        assert_eq!(
            rendered_value(&rendered, METRIC_PRUNE_ELIGIBLE_REFS),
            Some("1"),
            "render was:\n{rendered}"
        );
        assert_eq!(
            rendered_value(&rendered, METRIC_PRUNE_INELIGIBLE_REFS),
            Some("1"),
            "render was:\n{rendered}"
        );
    }

    /// The eligible side of a drain's split is the backlog the drain was LICENSED to
    /// take, sampled on the near side of taking it.
    ///
    /// This is a directed cell: the split published at the ACK reports **1** eligible
    /// ref, the watermark then rises without publishing anything, and the drain that
    /// follows is licensed for **3**. Three distinct values are therefore
    /// distinguishable at the gauge — `3` is the drain's own pre-drain computation,
    /// `1` would be the ACK's stale split, and `0` is what a recompute made AFTER the
    /// removal loop reports on every drain of an unbounded prune, whatever the backlog
    /// was. Only `3` is the licensed backlog the classification's terms name.
    #[test]
    fn the_licensed_backlog_is_sampled_before_the_drain_consumes_it() {
        let rendered = rendered_under_a_recorder(|| {
            let f = frontier();
            f.set_epoch_width(1);
            for k in ["k1", "k2", "k3", "k4"] {
                f.stamp_tombstone("m", k, "t");
            }
            // Only epoch 1 is byte-durable when the ACK lands, so the ACK's split sees
            // a licensed backlog of one ref out of four.
            f.set_durable_epoch_watermark(1);
            f.set_delivered(CONN_A, 100);
            let c: ClientId = "a5:alice|dev-1".into();
            assert!(block_on(f.confirm_apply_ack(&c, 4, CONN_A)));

            // Durability catches up to epoch 3. Injection publishes nothing, so the
            // gauge still holds the ACK's value until the drain recomputes it.
            f.set_durable_epoch_watermark(3);
            assert_eq!(
                f.drain_prunable_tombstones().len(),
                3,
                "epochs 1-3 clear both conjuncts, epoch 4 is above the watermark"
            );
        });

        for (name, value) in [
            (METRIC_PRUNE_ELIGIBLE_REFS, "3"),
            // Epoch 4's ref, pinned by the watermark at the same instant.
            (METRIC_PRUNE_INELIGIBLE_REFS, "1"),
            // The ACK's recompute plus the drain's.
            (METRIC_PRUNE_SPLIT_RECOMPUTES_TOTAL, "2"),
            // The index gauge is the far-side reading, and it MOVED — which is what
            // makes the eligible gauge's 3 a different instant rather than a stale
            // copy of the same one.
            (METRIC_PRUNE_INDEXED_REFS, "1"),
            (METRIC_PRUNE_LAST_DRAINED_EPOCH, "3"),
        ] {
            assert_eq!(
                rendered_value(&rendered, name),
                Some(value),
                "{name} must render {value}; render was:\n{rendered}"
            );
        }
    }

    /// The body of an item that starts at `anchor` and closes at the first brace at
    /// `indent` columns — the source-scan primitive the two structural tests below
    /// share.
    fn item_body<'a>(source: &'a str, anchor: &str, indent: &str) -> &'a str {
        let start = source
            .find(anchor)
            .unwrap_or_else(|| panic!("`{anchor}` is defined in the scanned file"));
        let tail = &source[start..];
        let close = format!("\n{indent}}}\n");
        let end = tail
            .find(&close)
            .unwrap_or_else(|| panic!("`{anchor}` closes at a brace in column {}", indent.len()));
        &tail[..end]
    }

    /// The epoch triple the pruning caller reads is taken under ONE acquisition of the
    /// drain's own lock, and the record never republishes it.
    ///
    /// Both halves are structural because neither is observable from a single-threaded
    /// test: a triple re-read through a second `lock()` tears only against a concurrent
    /// ACK, and a republish from the record is indistinguishable from the locked
    /// publish whenever the two happen to agree. A test that can only catch the
    /// interleaving is a test that reports green on the defect, so the guard is sited
    /// where the defect actually lives — in the shape of the code.
    #[test]
    fn the_drain_snapshot_is_one_acquisition_and_the_record_never_republishes_it() {
        const SOURCE: &str = include_str!("tombstone_frontier_impl.rs");
        const CONTRACT: &str = include_str!("tombstone_frontier.rs");

        let drain = item_body(
            SOURCE,
            "pub fn drain_prunable_tombstones(&self) -> Vec<(Epoch, TombstoneRef)> {",
            "    ",
        );
        assert_eq!(
            drain.matches("self.lock()").count(),
            1,
            "the drain must take its state lock exactly once: the drained refs, the \
             frontier snapshot and the split are one observation of one instant, and a \
             second acquisition lets a concurrent ACK move the epochs between them"
        );

        let record = item_body(
            SOURCE,
            "fn observe_drained_epoch(&self, record: &PruneEpochRecord) {",
            "    ",
        );
        assert!(
            !record.contains(".set("),
            "the per-epoch record must write no gauge: the epoch gauges are published \
             from the drain's locked snapshot, and a second writer here would publish \
             whatever the unlocked pruning caller happened to read — letting the torn \
             copy win. Body was:\n{record}"
        );

        let declared = item_body(CONTRACT, "pub struct PruneEpochRecord {", "");
        for field in [
            "current_epoch",
            "low_water_mark",
            "durable_epoch_watermark",
            "last_drained_epoch",
        ] {
            assert!(
                !declared.contains(field),
                "`PruneEpochRecord` must not carry `{field}`: a caller that holds no \
                 lock cannot fill an epoch field without reading it separately, so \
                 carrying one re-creates the tearable triple by construction. \
                 Declaration was:\n{declared}"
            );
        }
    }

    /// The two writers of `current_epoch` / `low_water_mark` agree BY CONSTRUCTION,
    /// and this is what asserts it.
    ///
    /// The gauges are written both from the frontier snapshot (`observe_epoch_state`)
    /// and from the claim span the split carries (`observe_claim_span`). The snapshot
    /// reads the CACHED low-water mark and the split's caller reads the LIVE fold, so
    /// the two agree only while every cursor mutation refreshes the cache under the
    /// same lock that mutated it. That is an invariant of three call sites, and an
    /// invariant nobody checks is a coincidence — so it is checked here across all
    /// three cursor mutations plus the drain, and the writer count is pinned so a
    /// third writer cannot be added silently.
    #[test]
    fn the_two_epoch_gauge_writers_agree_on_one_locked_reading() {
        const SOURCE: &str = include_str!("tombstone_frontier_impl.rs");
        for field in ["current_epoch", "low_water_mark"] {
            // Composed rather than written whole: a scanning literal that spells the
            // thing it counts counts itself, and this test would then be measuring its
            // own source.
            let writer = format!("self.{field}.set(");
            assert_eq!(
                SOURCE.matches(writer.as_str()).count(),
                2,
                "`{writer}` must have exactly the two known writers — the claim span \
                 and the frontier snapshot; a third would publish a reading neither \
                 this test nor the drain's lock covers"
            );
        }
        for publisher in [
            "fn publish_split(&self, split: &SplitObservation) {",
            "fn publish_frontier_state(&self, snapshot: &FrontierObservation) {",
        ] {
            assert!(
                !item_body(SOURCE, publisher, "    ").contains("self.lock()"),
                "`{publisher}` must publish from the caller's snapshot and take no \
                 lock of its own: a publisher that re-reads the state is a second \
                 instant wearing the first one's name"
            );
        }

        let f = frontier();
        f.set_epoch_width(1);
        let cached_matches_the_fold = |stage: &str| {
            let state = f.lock();
            assert_eq!(
                state.observed_lwm,
                state.low_water_mark(),
                "the cached low-water mark must equal the live fold after {stage}: \
                 the claim span publishes the fold and the frontier snapshot publishes \
                 the cache, and they are the same gauge"
            );
        };

        for k in ["k1", "k2", "k3"] {
            f.stamp_tombstone("m", k, "t");
        }
        f.set_durable_epoch_watermark(2);
        f.set_delivered(CONN_A, 100);
        f.set_delivered(CONN_B, 100);
        let laggard: ClientId = "a5:alice|dev-1".into();
        let leader: ClientId = "a5:alice|dev-2".into();

        block_on(async {
            assert!(f.confirm_apply_ack(&laggard, 1, CONN_A).await);
            cached_matches_the_fold("an ACK that advances");
            assert!(f.confirm_apply_ack(&leader, 3, CONN_B).await);
            cached_matches_the_fold("an ACK by a second client");
            // Dropping the fleet laggard is the largest single LWM movement there is.
            f.forget_client(&laggard).await;
            cached_matches_the_fold("forgetting the laggard");
            // No durable store is wired, so this rehydrate loads nothing — it still
            // has to leave the cache and the fold in agreement.
            f.rehydrate(&laggard).await;
            cached_matches_the_fold("a rehydrate");
        });
        assert_eq!(
            f.drain_prunable_tombstones().len(),
            2,
            "epochs 1 and 2 clear both conjuncts once the laggard is gone"
        );
        cached_matches_the_fold("a non-empty drain");
    }

    /// A replayed ACK moves no cursor, so it triggers no split recompute and reports
    /// no advance — the advance counter stays a count of MOVEMENTS. The stall gauge is
    /// refreshed anyway, because the regime worth seeing is the one where the
    /// low-water-mark is not moving.
    #[test]
    fn a_replayed_ack_reports_no_advance_and_recomputes_no_split() {
        let rendered = rendered_under_a_recorder(|| {
            let f = frontier();
            f.set_delivered(CONN_A, 100);
            let c: ClientId = "a5:alice|dev-1".into();
            assert!(block_on(f.confirm_apply_ack(&c, 5, CONN_A)));
            assert!(!block_on(f.confirm_apply_ack(&c, 3, CONN_A)), "replay");
            assert!(!block_on(f.confirm_apply_ack(&c, 5, CONN_A)), "duplicate");
        });

        assert_eq!(
            rendered_value(&rendered, METRIC_PRUNE_LWM_ADVANCES_TOTAL),
            Some("1"),
            "render was:\n{rendered}"
        );
        assert_eq!(
            rendered_value(&rendered, METRIC_PRUNE_SPLIT_RECOMPUTES_TOTAL),
            Some("1"),
            "render was:\n{rendered}"
        );
        assert!(
            rendered_value(&rendered, METRIC_PRUNE_LWM_STALL_SECONDS).is_some(),
            "the stall gauge must be refreshed on every ack; render was:\n{rendered}"
        );
    }

    /// Forgetting the fleet's laggard is a low-water-mark ADVANCE and is observed as
    /// one: leaving it out would put a hole in the advance cadence exactly where the
    /// retention ceiling gets its headroom.
    #[test]
    fn forgetting_the_laggard_is_observed_as_an_lwm_advance() {
        let rendered = rendered_under_a_recorder(|| {
            let f = frontier();
            f.set_delivered(CONN_A, 100);
            f.set_delivered(CONN_B, 100);
            let fast: ClientId = "a5:alice|dev-1".into();
            let slow: ClientId = "a5:bob|dev-2".into();
            assert!(block_on(f.confirm_apply_ack(&fast, 9, CONN_A)));
            assert!(block_on(f.confirm_apply_ack(&slow, 2, CONN_B)));
            assert_eq!(f.low_water_mark(), 2, "the laggard pins the fleet");
            block_on(f.forget_client(&slow));
            assert_eq!(f.low_water_mark(), 9);
        });

        // Two advances: 0→9 (fast) and 2→9 (forget). The laggard's own ack moved its
        // cursor but dropped the fleet MIN from 9 to 2 — a genuine movement, and
        // deliberately NOT an advance, so it neither increments the counter nor is
        // reported as negative epochs advanced.
        assert_eq!(
            rendered_value(&rendered, METRIC_PRUNE_LWM_ADVANCES_TOTAL),
            Some("2"),
            "render was:\n{rendered}"
        );
        assert_eq!(
            rendered_value(&rendered, METRIC_PRUNE_LWM_EPOCHS_ADVANCED_TOTAL),
            Some("16"),
            "9 epochs on the first advance, 7 more on the forget; render was:\n{rendered}"
        );
        assert_eq!(
            rendered_value(&rendered, METRIC_PRUNE_LOW_WATER_MARK),
            Some("9"),
            "render was:\n{rendered}"
        );
    }

    /// A dropped ACK (one that never arrives) never advances any frontier: with no
    /// `confirm_apply_ack` call the client stays untracked and the LWM is vacuous.
    #[tokio::test]
    async fn dropped_ack_does_not_advance() {
        let f = frontier();
        let c: ClientId = "a5:alice|dev-1".into();
        // Deliver 10 on the connection, but the ACK is "dropped" — never sent.
        f.set_delivered(CONN_A, 10);
        assert!(!f.is_tracked(&c), "no ACK → untracked");
        assert_eq!(f.cursor(&c), None);
    }

    /// A replayed / reordered cursor <= the current one is a no-op (monotone-max).
    #[tokio::test]
    async fn replay_or_reorder_is_noop() {
        let f = frontier();
        let c: ClientId = "a5:alice|dev-1".into();
        f.set_delivered(CONN_A, 100);
        assert!(
            f.confirm_apply_ack(&c, 10, CONN_A).await,
            "first advance to 10"
        );
        assert_eq!(f.cursor(&c), Some(10));
        // Replay a lower cursor: no-op.
        assert!(
            !f.confirm_apply_ack(&c, 5, CONN_A).await,
            "replay < current"
        );
        assert_eq!(f.cursor(&c), Some(10));
        // Re-send the same cursor: no-op.
        assert!(
            !f.confirm_apply_ack(&c, 10, CONN_A).await,
            "replay == current"
        );
        assert_eq!(f.cursor(&c), Some(10));
    }

    /// A claim above the server's current max epoch is rejected (global bound), even
    /// when the connection claims (and was "delivered") the forged-future value.
    #[tokio::test]
    async fn global_bound_rejects_forged_future() {
        let f = frontier();
        let c: ClientId = "a5:alice|dev-1".into();
        f.set_current_max_epoch(50);
        // Deliver + claim 100, but the server only ever stamped up to 50.
        f.set_delivered(CONN_A, 100);
        assert!(f.confirm_apply_ack(&c, 100, CONN_A).await);
        assert_eq!(
            f.cursor(&c),
            Some(50),
            "cursor clamped to the global max epoch, not the forged 100"
        );
    }

    /// The delivered clamp: a fresh connection delivered NOTHING cannot establish or
    /// advance any cursor, even when it acks a high value — it stays untracked.
    #[tokio::test]
    async fn delivered_clamp_fresh_device_ack_high_stays_untracked() {
        let f = frontier();
        let c: ClientId = "a5:alice|dev-1".into();
        // No set_delivered → delivered_conn == 0.
        assert!(
            !f.confirm_apply_ack(&c, 1_000_000, CONN_A).await,
            "delivered-nothing ACK cannot advance"
        );
        assert!(!f.is_tracked(&c), "fresh device stays untracked");
        assert_eq!(
            f.low_water_mark(),
            0,
            "no tracked client → LWM 0 → prune nothing (conservative vacuous case)"
        );
    }

    /// The vacuous low-water-mark is 0 (prune NOTHING), NOT the current max epoch.
    /// Rehydration is lazy, so an empty in-memory frontier post-restart means "no
    /// client has reconnected yet", not "no client to protect" — returning the max
    /// epoch would license 342b to prune tombstones a not-yet-reconnected laggard
    /// still needs. Even with a high injected global bound, empty → 0.
    #[tokio::test]
    async fn empty_frontier_lwm_is_zero_prunes_nothing() {
        let f = frontier();
        assert_eq!(
            f.low_water_mark(),
            0,
            "empty frontier prunes nothing (default)"
        );
        // A set global bound must NOT leak into the vacuous LWM.
        f.set_current_max_epoch(1_000_000);
        assert_eq!(
            f.low_water_mark(),
            0,
            "empty frontier still prunes nothing even with a high global bound"
        );
    }

    /// A connection can advance ONLY the cursor of the identity it names; identity is
    /// connection-derived, and two principals get disjoint keys — one principal's ACK
    /// cannot touch another principal's cursor.
    #[tokio::test]
    async fn cross_principal_cannot_advance() {
        let f = frontier();
        let a: ClientId = "a5:alice|dev-1".into();
        let b: ClientId = "a3:bob|dev-1".into();
        f.set_delivered(CONN_A, 100);
        assert!(f.confirm_apply_ack(&a, 30, CONN_A).await);
        // An ACK for alice's key never changes bob's cursor (disjoint keys).
        assert_eq!(f.cursor(&a), Some(30));
        assert_eq!(f.cursor(&b), None, "bob's cursor untouched by alice's ACK");
    }

    /// Two DEVICES under ONE principal have INDEPENDENT cursors, and the LWM is the
    /// MIN of both — a lagging device pins the epoch fleet-wide.
    #[tokio::test]
    async fn two_devices_one_principal_independent_cursors_lwm_is_min() {
        let f = frontier();
        let d1: ClientId = "a5:alice|dev-1".into();
        let d2: ClientId = "a5:alice|dev-2".into();
        f.set_delivered(CONN_A, 100);
        f.set_delivered(CONN_B, 100);
        assert!(f.confirm_apply_ack(&d1, 20, CONN_A).await);
        assert!(f.confirm_apply_ack(&d2, 5, CONN_B).await);
        assert_eq!(f.cursor(&d1), Some(20));
        assert_eq!(f.cursor(&d2), Some(5));
        assert_eq!(f.low_water_mark(), 5, "the lagging device pins the LWM");
        // Advancing the laggard raises the LWM.
        assert!(f.confirm_apply_ack(&d2, 25, CONN_B).await);
        assert_eq!(f.low_water_mark(), 20, "now dev-1 is the laggard");
    }

    /// Fencepost: `confirm_apply(E)` is inclusive — a client whose cursor is exactly
    /// `E` has applied `E`, and its predecessor `E-1` is below the LWM (not
    /// resurrectable on it).
    #[tokio::test]
    async fn fencepost_inclusive() {
        let f = frontier();
        let c: ClientId = "a5:alice|dev-1".into();
        f.set_delivered(CONN_A, 100);
        assert!(f.confirm_apply_ack(&c, 42, CONN_A).await);
        // Applied ≤ 42: the cursor sits at 42, so LWM == 42; an epoch at exactly the
        // cursor is treated as applied, and 41 (its predecessor) is strictly below.
        assert_eq!(f.low_water_mark(), 42);
        assert!(f.low_water_mark() >= 42, "epoch 42 is applied (inclusive)");
        assert!(41 < f.low_water_mark(), "predecessor 41 is below the LWM");
    }

    /// The regression-claim gate flags a replica whose sync-init claim is below its
    /// stored cursor, NEVER rolls the stored cursor back, and its ACKs stay no-ops
    /// (delivered clamp) until a genuine resync sets `delivered_conn`.
    #[tokio::test]
    async fn regression_claim_gates_full_resync() {
        let f = frontier();
        let c: ClientId = "a5:alice|dev-1".into();
        // Establish a real cursor at 100.
        f.set_delivered(CONN_A, 100);
        assert!(f.confirm_apply_ack(&c, 100, CONN_A).await);
        assert_eq!(f.cursor(&c), Some(100));

        // Reconnect on a FRESH connection (delivered == 0), claiming a regressed 5.
        assert!(f.is_regressed(&c, 5), "claim 5 < stored 100 → regressed");
        assert_eq!(f.cursor(&c), Some(100), "stored cursor never rolled back");

        // Its ACKs are no-ops until a genuine resync delivers on the new connection.
        assert!(
            !f.confirm_apply_ack(&c, 100, CONN_B).await,
            "cannot re-track at the stale-high cursor without resyncing (delivered==0)"
        );
        assert_eq!(
            f.cursor(&c),
            Some(100),
            "still pinned at 100, not re-advanced"
        );

        // A genuine resync sets delivered_conn on the new connection; ACK now applies.
        f.set_delivered(CONN_B, 120);
        assert!(f.confirm_apply_ack(&c, 120, CONN_B).await);
        assert_eq!(f.cursor(&c), Some(120));
    }

    /// A claim >= stored at sync-init is informational only (NOT regressed, sync-init
    /// is not an ACK — it does not advance the cursor).
    #[tokio::test]
    async fn sync_init_claim_at_or_above_stored_is_not_regressed_and_not_an_ack() {
        let f = frontier();
        let c: ClientId = "a5:alice|dev-1".into();
        f.set_delivered(CONN_A, 100);
        assert!(f.confirm_apply_ack(&c, 30, CONN_A).await);
        assert!(!f.is_regressed(&c, 30), "claim == stored is not regressed");
        assert!(!f.is_regressed(&c, 50), "claim > stored is not regressed");
        assert_eq!(
            f.cursor(&c),
            Some(30),
            "sync-init claim did not advance the cursor"
        );
    }

    /// An unknown client (never confirmed) is untracked and pins nothing.
    #[tokio::test]
    async fn unknown_client_is_untracked_and_pins_nothing() {
        let f = frontier();
        let c: ClientId = "a5:alice|dev-1".into();
        assert!(!f.is_tracked(&c));
        assert!(!f.is_regressed(&c, 5), "an unknown client cannot regress");
    }

    /// `remove_connection` drops the per-connection delivered state but LEAVES the
    /// per-identity cursors intact (they survive reconnect via rehydration).
    #[tokio::test]
    async fn remove_connection_drops_delivered_not_cursors() {
        let f = frontier();
        let c: ClientId = "a5:alice|dev-1".into();
        f.set_delivered(CONN_A, 50);
        assert!(f.confirm_apply_ack(&c, 30, CONN_A).await);
        f.remove_connection(CONN_A);
        assert_eq!(f.delivered(CONN_A), 0, "delivered dropped on disconnect");
        assert_eq!(f.cursor(&c), Some(30), "cursor survives the disconnect");
    }

    // -- Epoch machinery (342b): stamping, epoch↔sequence lockstep, and the
    //    dark-by-construction prune conjunction. --

    /// AC2: the epoch is stamped server-authoritatively from the op sequence,
    /// NEVER derived from the client tag's `millis` — a wildly skewed-clock tag
    /// lands in the SAME sequential bucket a monotonic-clock tag would.
    #[tokio::test]
    async fn stamp_is_server_authoritative_not_from_tag_millis() {
        let f = frontier();
        f.set_epoch_width(1); // one epoch per stamp, so buckets are 1, 2, 3, ...
                              // A far-future skewed tag then a far-past tag: their millis differ by eons,
                              // yet the epochs are strictly sequential (server-authoritative).
        let e1 = f.stamp_tombstone("m", "k1", "99999999999999:0:skewed");
        let e2 = f.stamp_tombstone("m", "k2", "1:0:past");
        assert_eq!(
            e1, 1,
            "first stamp is epoch 1 (never the reserved sentinel 0)"
        );
        assert_eq!(
            e2, 2,
            "second stamp is epoch 2 — sequential, not tag-millis-derived"
        );
        assert_eq!(f.current_epoch(), 2);
    }

    /// The epoch counter advances in lockstep with the op sequence at
    /// `EPOCH_WIDTH` granularity: `width` stamps share one epoch, the next
    /// `width` roll to the next. The first epoch is always 1 (never 0).
    #[tokio::test]
    async fn epoch_advances_in_lockstep_with_op_sequence() {
        let f = frontier();
        f.set_epoch_width(3);
        let epochs: Vec<Epoch> = (0..7)
            .map(|i| f.stamp_tombstone("m", &format!("k{i}"), &format!("{i}:0:n")))
            .collect();
        assert_eq!(epochs, vec![1, 1, 1, 2, 2, 2, 3]);
    }

    /// AC3(i): a single tracked-and-behind client pins an epoch fleet-wide — an
    /// epoch above the lagging client's cursor is NOT prune-eligible even with
    /// the durability watermark wide open, because the low-water-mark is the MIN
    /// across ALL tracked clients.
    #[tokio::test]
    async fn one_behind_client_pins_epoch_fleet_wide() {
        let f = frontier();
        f.set_epoch_width(1);
        for i in 0..5 {
            f.stamp_tombstone("m", &format!("k{i}"), &format!("{i}:0:n")); // epochs 1..=5
        }
        let ahead: ClientId = "a5:alice|dev-ahead".into();
        let behind: ClientId = "a5:alice|dev-behind".into();
        f.set_delivered(CONN_A, 100);
        f.set_delivered(CONN_B, 100);
        assert!(f.confirm_apply_ack(&ahead, 5, CONN_A).await);
        assert!(f.confirm_apply_ack(&behind, 3, CONN_B).await);
        assert_eq!(f.low_water_mark(), 3, "the behind client pins the LWM at 3");
        assert_eq!(
            f.reclamation().prune_ceiling(ClaimScope::Global),
            3,
            "the reclamation boundary the drain folds over is pinned at 3 as well"
        );
        // Open the durability watermark fully so ONLY the LWM half gates.
        f.set_durable_epoch_watermark(1000);
        assert!(
            f.is_epoch_prune_eligible(2),
            "LWM 3 > epoch 2 (strictly past)"
        );
        assert!(
            !f.is_epoch_prune_eligible(3),
            "epoch 3 pinned fleet-wide: the behind cursor AT 3 is not strictly past it"
        );
        let drained = f.drain_prunable_tombstones();
        let dropped: Vec<&str> = drained.iter().map(|(_, r)| r.tag.as_str()).collect();
        assert_eq!(
            drained.len(),
            2,
            "only epochs 1..=2 (strictly below the LWM) drained; 3..=5 pinned fleet-wide"
        );
        assert!(dropped.contains(&"0:0:n") && dropped.contains(&"1:0:n"));
    }

    /// AC3(ii): the watermark conjunct is load-bearing — with the LWM past every
    /// stamped epoch but the injected durability watermark BELOW some of them,
    /// the epochs above the watermark stay un-pruned (byte-durability fence).
    #[tokio::test]
    async fn lwm_past_but_watermark_behind_keeps_epoch_unpruned() {
        let f = frontier();
        f.set_epoch_width(1);
        for i in 0..5 {
            f.stamp_tombstone("m", &format!("k{i}"), &format!("{i}:0:n"));
        }
        let c: ClientId = "a5:alice|dev-1".into();
        f.set_delivered(CONN_A, 100);
        assert!(f.confirm_apply_ack(&c, 5, CONN_A).await);
        assert_eq!(f.low_water_mark(), 5, "LWM strictly past epochs 1..=4");
        // Epochs 1..=4 are LWM-eligible (strict >), but the watermark only reaches
        // epoch 2: epochs 3..=4 must stay despite being LWM-eligible — the
        // watermark conjunct is load-bearing, not decorative.
        f.set_durable_epoch_watermark(2);
        let drained = f.drain_prunable_tombstones();
        assert_eq!(
            drained.len(),
            2,
            "only epochs 1..=2 (<= watermark) drop; 3..=4 LWM-eligible but watermark-fenced"
        );
    }

    /// `AC3a`: dark-by-construction — with the production watermark (constant 0),
    /// NOTHING is ever prune-eligible even when the only tracked client has
    /// confirmed PAST every stamped epoch. Tombstones only accumulate (today's
    /// behavior, now with epochs stamped and ACKs flowing). Also asserts the
    /// first stamped epoch is `>= 1` and that the reserved sentinel epoch 0 is
    /// structurally safe (never eligible; no tag indexed under key 0).
    #[tokio::test]
    async fn dark_by_construction_no_prune_with_zero_watermark() {
        let f = frontier();
        f.set_epoch_width(1);
        let first = f.stamp_tombstone("m", "k0", "0:0:n");
        for i in 1..5 {
            f.stamp_tombstone("m", &format!("k{i}"), &format!("{i}:0:n"));
        }
        assert_eq!(
            first, 1,
            "first stamped epoch is 1, never the reserved sentinel 0"
        );
        assert_eq!(
            f.durable_epoch_watermark(),
            0,
            "production durability watermark is constant 0 (dark by construction)"
        );
        let c: ClientId = "a5:alice|dev-1".into();
        f.set_delivered(CONN_A, 100);
        // Confirm PAST every stamped epoch — the claim is clamped by the real
        // counter (current_max_epoch == 5) to exactly 5.
        assert!(f.confirm_apply_ack(&c, 100, CONN_A).await);
        assert_eq!(
            f.low_water_mark(),
            5,
            "client confirmed past every stamped epoch (clamped to the max stamped epoch)"
        );
        // Dark: the watermark conjunct blocks — NOTHING drains, even though every
        // tracked client is past every stamped epoch.
        assert!(
            f.drain_prunable_tombstones().is_empty(),
            "no prune fires while the durability watermark is 0 — tombstones only accumulate"
        );
        // The reserved sentinel epoch 0 is safe by structure: never eligible at the
        // trait level, and nothing is ever indexed under key 0.
        assert!(
            !f.is_epoch_prune_eligible(0),
            "epoch 0 is never prune-eligible (reserved sentinel)"
        );
    }

    /// Eligibility is STRICT per the 342a contract ("advanced PAST epoch"): a
    /// cursor AT epoch E does not make E eligible — E may still be accumulating
    /// tombstones (width > 1) the client never received. Only LWM == E+1 proves
    /// all of E is delivered fleet-wide.
    #[tokio::test]
    async fn eligibility_is_strictly_past_not_inclusive() {
        let f = frontier();
        f.set_epoch_width(1);
        // Stamp epochs 1..=3 so the counter (and the ACK clamp) reach 3.
        for i in 0..3 {
            f.stamp_tombstone("m", &format!("k{i}"), &format!("{i}:0:n"));
        }
        let c: ClientId = "a5:alice|dev-1".into();
        f.set_delivered(CONN_A, 100);
        assert!(f.confirm_apply_ack(&c, 2, CONN_A).await);
        assert_eq!(f.low_water_mark(), 2);
        // Eligibility folds over the reclamation boundary, so the strictness under test is that
        // boundary's: asserting only the mark would leave the fixture green on a tree where the
        // two had drifted apart.
        assert_eq!(
            f.reclamation().min_live_claim(ClaimScope::Global),
            Some(2),
            "the ACK is recorded as a claim at the cursor it established"
        );
        assert_eq!(f.reclamation().prune_ceiling(ClaimScope::Global), 2);
        assert!(
            !f.is_epoch_prune_eligible(2),
            "LWM == epoch is NOT eligible (strict)"
        );
        assert!(
            f.is_epoch_prune_eligible(1),
            "LWM == epoch + 1 is eligible (strictly past)"
        );
    }

    /// A drained ref whose storage drop failed is handed back via
    /// `restore_tombstone_ref` and re-drained on the next sweep — a
    /// drained-but-not-dropped tag must never lose its index entry (that would
    /// orphan it un-prunable in storage forever).
    #[tokio::test]
    async fn restore_tombstone_ref_round_trips_through_drain() {
        let f = frontier();
        f.set_epoch_width(1);
        f.stamp_tombstone("m", "k1", "T1"); // epoch 1
        f.stamp_tombstone("m", "k2", "T2"); // epoch 2 (keeps counter/clamp at 2)
        let c: ClientId = "a5:alice|dev-1".into();
        f.set_delivered(CONN_A, 100);
        assert!(f.confirm_apply_ack(&c, 2, CONN_A).await); // LWM 2 > 1
        f.set_durable_epoch_watermark(1000);

        let drained = f.drain_prunable_tombstones();
        assert_eq!(drained.len(), 1, "epoch 1's ref drained");
        let (epoch, r) = drained.into_iter().next().unwrap();
        assert_eq!((epoch, r.tag.as_str()), (1, "T1"));

        // Index entry is gone: a second sweep finds nothing.
        assert!(f.drain_prunable_tombstones().is_empty());

        // Simulate a failed storage drop: hand the ref back; the next sweep
        // returns it again (retry instead of permanent orphan).
        f.restore_tombstone_ref(epoch, r);
        let retried = f.drain_prunable_tombstones();
        assert_eq!(retried.len(), 1);
        assert_eq!(retried[0].0, 1);
        assert_eq!(retried[0].1.tag, "T1");
    }

    /// With no device tracked, the boundary is the BOOT FLOOR and never the head epoch: a pass
    /// that runs over five stamped epochs reclaims none of them.
    ///
    /// The pinned watermark is what makes this measurable instead of vacuous. At watermark 0 the
    /// drain takes its dark fast path and returns before it ever consults the boundary, so an
    /// empty drain there would say nothing about where the boundary sits — the completed-sweep
    /// counter asserted below is the proof that the pass actually reached it. With the pass live,
    /// a boundary derived from the head epoch — or from anything above the floor — drains every
    /// stamped epoch here.
    #[test]
    fn an_untracked_fleet_reclaims_nothing_on_a_pass_that_really_runs() {
        use crate::reclamation_registry::METRIC_RECLAMATION_SWEEPS_TOTAL;

        let rendered = rendered_under_a_recorder(|| {
            let f = frontier();
            f.set_epoch_width(1);
            for i in 0..5 {
                f.stamp_tombstone("m", &format!("k{i}"), &format!("{i}:0:n"));
            }
            assert_eq!(f.current_epoch(), 5, "the counter is far past the floor");
            // Wide open, so the durability conjunct cannot be what holds the drain empty.
            f.set_durable_epoch_watermark(1000);

            let registry = f.reclamation();
            assert_eq!(registry.live_claims(ClaimScope::Global), 0);
            assert_eq!(
                registry.min_live_claim(ClaimScope::Global),
                None,
                "no device has claimed anything"
            );
            assert_eq!(
                registry.prune_ceiling(ClaimScope::Global),
                0,
                "an unclaimed fleet proposes the boot floor, not the epoch the server reached"
            );
            assert!(
                !f.is_epoch_prune_eligible(1),
                "not even the oldest stamped epoch is eligible under an unclaimed boundary"
            );
            assert!(
                f.drain_prunable_tombstones().is_empty(),
                "a live pass over an unclaimed fleet must reclaim nothing"
            );
            assert_eq!(
                registry.executed_watermark(ClaimScope::Global),
                0,
                "a pass that reclaimed nothing must fence nothing"
            );
        });

        assert_eq!(
            rendered_value(&rendered, METRIC_RECLAMATION_SWEEPS_TOTAL),
            Some("1"),
            "the drain must have reached the boundary protocol — an empty drain that never \
             began a sweep would prove nothing; render was:\n{rendered}"
        );
    }

    /// The boundary is the fleet MIN and a laggard LOWERS it: two devices ACK at 5 and at 3, BOTH
    /// claims are recorded, and the pass reclaims exactly the epochs strictly below the laggard.
    ///
    /// Being behind another device is never a reason to refuse a claim — only sitting below
    /// content already recorded as reclaimed is — so neither device is fenced and the rejection
    /// counter stays flat. The same two ACKs then run in the REVERSE order and must land on an
    /// identical end state: a claim leaves the fold by an explicit release and by nothing else,
    /// least of all by the order the devices happened to arrive in.
    #[test]
    fn a_laggard_lowers_the_boundary_and_is_recorded_in_either_ack_order() {
        use crate::reclamation_registry::METRIC_RECLAMATION_CLAIMS_REJECTED_BELOW_EXECUTED_TOTAL;

        fn two_devices_ack(behind_first: bool) -> (Vec<Epoch>, String) {
            let mut drained_epochs = Vec::new();
            let rendered = rendered_under_a_recorder(|| {
                let f = frontier();
                f.set_epoch_width(1);
                for i in 0..5 {
                    f.stamp_tombstone("m", &format!("k{i}"), &format!("{i}:0:n"));
                }
                let ahead: ClientId = "a5:alice|dev-ahead".into();
                let behind: ClientId = "a5:alice|dev-behind".into();
                f.set_delivered(CONN_A, 100);
                f.set_delivered(CONN_B, 100);
                let mut acks = vec![(&ahead, 5, CONN_A), (&behind, 3, CONN_B)];
                if behind_first {
                    acks.reverse();
                }
                for (client, claim, conn) in acks {
                    assert!(
                        block_on(f.confirm_apply_ack(client, claim, conn)),
                        "both ACKs advance a cursor: neither device is behind reclaimed content"
                    );
                }

                let registry = f.reclamation();
                let margin = registry.margin_epochs();
                assert_eq!(
                    registry.live_claims(ClaimScope::Global),
                    2,
                    "both devices hold a claim: the trailing one is recorded, not displaced"
                );
                assert_eq!(registry.min_live_claim(ClaimScope::Global), Some(3));
                assert_eq!(
                    registry.prune_ceiling(ClaimScope::Global),
                    3 - margin,
                    "the proposal is the fleet MIN less the margin"
                );
                assert!(
                    f.is_tracked(&ahead) && f.is_tracked(&behind),
                    "neither device is fenced"
                );
                assert_eq!(f.low_water_mark(), 3);

                f.set_durable_epoch_watermark(1000);
                drained_epochs = f
                    .drain_prunable_tombstones()
                    .iter()
                    .map(|(e, _)| *e)
                    .collect();
                drained_epochs.sort_unstable();
            });
            (drained_epochs, rendered)
        }

        let (forward, forward_render) = two_devices_ack(false);
        let (reverse, reverse_render) = two_devices_ack(true);

        assert_eq!(
            forward,
            vec![1, 2],
            "only the epochs strictly below the trailing device's cursor are reclaimed"
        );
        assert_eq!(
            reverse, forward,
            "ACK order must not change which epochs a pass reclaims"
        );
        for rendered in [&forward_render, &reverse_render] {
            assert_eq!(
                rendered_value(
                    rendered,
                    METRIC_RECLAMATION_CLAIMS_REJECTED_BELOW_EXECUTED_TOTAL
                ),
                Some("0"),
                "a device that is merely behind another is never refused; render was:\n{rendered}"
            );
        }
    }

    /// The executed watermark fences a device below content the pass recorded as reclaimed, and
    /// only one sweep runs at a time.
    ///
    /// The pair is what makes the fence a boundary rather than a latch: a device arriving one
    /// epoch below the watermark is refused and left untracked (nothing the boundary can move
    /// would serve it, so the conservative re-admission path is the honest answer), while a device
    /// arriving AT the watermark is recorded normally. The refusal must leave the claim map
    /// exactly as it found it — a refusal that half-recorded would pin the boundary on a device
    /// nothing is tracking.
    #[test]
    fn the_watermark_fences_below_reclaimed_content_and_one_sweep_runs_at_a_time() {
        use crate::reclamation_registry::{
            METRIC_RECLAMATION_CLAIMS_REJECTED_BELOW_EXECUTED_TOTAL,
            METRIC_RECLAMATION_SWEEPS_TOTAL, METRIC_RECLAMATION_SWEEP_IN_PROGRESS,
        };

        let recorder = PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        metrics::with_local_recorder(&recorder, || {
            let f = frontier();
            f.set_epoch_width(1);
            for i in 0..5 {
                f.stamp_tombstone("m", &format!("k{i}"), &format!("{i}:0:n"));
            }
            let pinned: ClientId = "a5:alice|dev-pinned".into();
            f.set_delivered(CONN_A, 100);
            assert!(block_on(f.confirm_apply_ack(&pinned, 5, CONN_A)));

            let registry = f.reclamation();
            assert_eq!(registry.prune_ceiling(ClaimScope::Global), 5);

            let token = registry.begin_sweep().expect("the first sweep is granted");
            assert_eq!(
                token.ceiling(),
                5,
                "the token carries the fleet MIN taken at sweep start"
            );

            // Single-sweep guard: the second call is refused while the token is outstanding, and
            // refusing it must issue no snapshot and move neither the in-progress gauge nor the
            // completed-sweep counter.
            let mid_sweep = handle.render();
            assert!(
                registry.begin_sweep().is_none(),
                "a second sweep must be refused while a token is outstanding"
            );
            let after_refusal = handle.render();
            for rendered in [&mid_sweep, &after_refusal] {
                assert_eq!(
                    rendered_value(rendered, METRIC_RECLAMATION_SWEEP_IN_PROGRESS),
                    Some("1"),
                    "exactly one sweep is in progress across the refusal; render:\n{rendered}"
                );
            }
            assert_eq!(
                rendered_value(&mid_sweep, METRIC_RECLAMATION_SWEEPS_TOTAL),
                rendered_value(&after_refusal, METRIC_RECLAMATION_SWEEPS_TOTAL),
                "a refused sweep completes nothing and must not be counted as one"
            );

            // The pass observed a durable watermark BELOW the ceiling, so what it could actually
            // have reclaimed — and therefore what it may fence against — is min(5, 4 + 1).
            registry.end_sweep(token, 4);
            assert_eq!(registry.executed_watermark(ClaimScope::Global), 5);

            let next = registry
                .begin_sweep()
                .expect("the guard clears when the sweep ends");
            registry.end_sweep(next, 4);
            assert_eq!(
                registry.executed_watermark(ClaimScope::Global),
                5,
                "a second pass over the same content advances the fence no further"
            );

            let claims_before = registry.live_claims(ClaimScope::Global);
            let min_before = registry.min_live_claim(ClaimScope::Global);
            let fenced: ClientId = "a5:alice|dev-fenced".into();
            f.set_delivered(CONN_B, 100);
            assert!(
                !block_on(f.confirm_apply_ack(&fenced, 4, CONN_B)),
                "a device below reclaimed content is refused, so its cursor is not written"
            );
            assert!(
                !f.is_tracked(&fenced),
                "the refused device stays untracked and takes the full-resync path"
            );
            assert_eq!(
                (
                    registry.live_claims(ClaimScope::Global),
                    registry.min_live_claim(ClaimScope::Global)
                ),
                (claims_before, min_before),
                "a refusal records nothing: the claim map is exactly what it was"
            );

            let at_boundary: ClientId = "a5:alice|dev-at-boundary".into();
            assert!(
                block_on(f.confirm_apply_ack(&at_boundary, 5, CONN_B)),
                "a device AT the watermark is above reclaimed content and is recorded"
            );
            assert!(f.is_tracked(&at_boundary));
            assert_eq!(registry.live_claims(ClaimScope::Global), claims_before + 1);
        });

        let rendered = handle.render();
        assert_eq!(
            rendered_value(
                &rendered,
                METRIC_RECLAMATION_CLAIMS_REJECTED_BELOW_EXECUTED_TOTAL
            ),
            Some("1"),
            "exactly the one device below the watermark is refused; render was:\n{rendered}"
        );
        assert_eq!(
            rendered_value(&rendered, METRIC_RECLAMATION_SWEEPS_TOTAL),
            Some("2"),
            "two sweeps completed and the refused one is not among them; render:\n{rendered}"
        );
    }

    /// Capture the prune-decision corpus as machine-readable lines, one per fixture.
    ///
    /// This is a DECISION-NEUTRAL probe: it re-runs the store-less prune fixtures' scenarios and
    /// prints what the drain decided (`fixture | epochs drained | tags dropped`). It asserts
    /// nothing and re-points no predicate, so running it on two builds of the tree and diffing the
    /// two outputs is a direct measurement of whether a change to the prune's authority moved any
    /// prune decision. Run it with `-- --nocapture` to read the lines.
    ///
    /// The store-backed rows of the same corpus — the rehydrate and forget fixtures — need a redb
    /// store, so they are captured by `persistence_tests::capture_prune_decision_corpus_durable`
    /// in the same line format.
    #[tokio::test]
    async fn capture_prune_decision_corpus() {
        // The two-device fleet-MIN case: the behind device pins epochs at and above its cursor.
        {
            let f = frontier();
            f.set_epoch_width(1);
            for i in 0..5 {
                f.stamp_tombstone("m", &format!("k{i}"), &format!("{i}:0:n"));
            }
            let ahead: ClientId = "a5:alice|dev-ahead".into();
            let behind: ClientId = "a5:alice|dev-behind".into();
            f.set_delivered(CONN_A, 100);
            f.set_delivered(CONN_B, 100);
            let _ = f.confirm_apply_ack(&ahead, 5, CONN_A).await;
            let _ = f.confirm_apply_ack(&behind, 3, CONN_B).await;
            f.set_durable_epoch_watermark(1000);
            let drained = f.drain_prunable_tombstones();
            println!(
                "{}",
                prune_decision_line("one_behind_client_pins_epoch_fleet_wide", &drained)
            );
        }
        // The durability fence held below the claim boundary: the fence is the binding conjunct.
        {
            let f = frontier();
            f.set_epoch_width(1);
            for i in 0..5 {
                f.stamp_tombstone("m", &format!("k{i}"), &format!("{i}:0:n"));
            }
            let c: ClientId = "a5:alice|dev-1".into();
            f.set_delivered(CONN_A, 100);
            let _ = f.confirm_apply_ack(&c, 5, CONN_A).await;
            f.set_durable_epoch_watermark(2);
            let drained = f.drain_prunable_tombstones();
            println!(
                "{}",
                prune_decision_line(
                    "lwm_past_but_watermark_behind_keeps_epoch_unpruned",
                    &drained
                )
            );
        }
        // Dark by construction: a 0 watermark takes the fast path and decides nothing.
        {
            let f = frontier();
            f.set_epoch_width(1);
            for i in 0..5 {
                f.stamp_tombstone("m", &format!("k{i}"), &format!("{i}:0:n"));
            }
            let c: ClientId = "a5:alice|dev-1".into();
            f.set_delivered(CONN_A, 100);
            let _ = f.confirm_apply_ack(&c, 100, CONN_A).await;
            let drained = f.drain_prunable_tombstones();
            println!(
                "{}",
                prune_decision_line(
                    "dark_by_construction_no_prune_with_zero_watermark",
                    &drained
                )
            );
        }
        // Strict, not inclusive: a cursor AT an epoch does not license draining that epoch.
        {
            let f = frontier();
            f.set_epoch_width(1);
            for i in 0..3 {
                f.stamp_tombstone("m", &format!("k{i}"), &format!("{i}:0:n"));
            }
            let c: ClientId = "a5:alice|dev-1".into();
            f.set_delivered(CONN_A, 100);
            let _ = f.confirm_apply_ack(&c, 2, CONN_A).await;
            f.set_durable_epoch_watermark(1000);
            let drained = f.drain_prunable_tombstones();
            println!(
                "{}",
                prune_decision_line("eligibility_is_strictly_past_not_inclusive", &drained)
            );
        }
        // A restored ref is re-decided by the next pass rather than orphaned.
        {
            let f = frontier();
            f.set_epoch_width(1);
            f.stamp_tombstone("m", "k1", "T1");
            f.stamp_tombstone("m", "k2", "T2");
            let c: ClientId = "a5:alice|dev-1".into();
            f.set_delivered(CONN_A, 100);
            let _ = f.confirm_apply_ack(&c, 2, CONN_A).await;
            f.set_durable_epoch_watermark(1000);
            let drained = f.drain_prunable_tombstones();
            println!(
                "{}",
                prune_decision_line("restore_tombstone_ref_round_trips_through_drain", &drained)
            );
            for (epoch, r) in drained {
                f.restore_tombstone_ref(epoch, r);
            }
            let retried = f.drain_prunable_tombstones();
            println!(
                "{}",
                prune_decision_line(
                    "restore_tombstone_ref_round_trips_through_drain#after_restore",
                    &retried
                )
            );
        }
    }

    // -----------------------------------------------------------------------
    // The two OBSERVATION fields (R1) -- unit coverage, the rendered `/metrics`
    // proof (X21-c), the rendered-field-text proof (X21-b), the pre-freeze
    // sanity row `S1` (R1.7, C12) and Step 0 leg (c) (R1.7, X21-a, C11).
    // -----------------------------------------------------------------------

    /// R1.5 (first case): every exit kind other than `DrainedByPrune` carries `0` in both
    /// new fields, even though `refs_at_entry` (the attribution the observation pair exists
    /// to be checked against) is nonzero on the same row.
    #[test]
    fn finalize_epoch_exit_zeroes_observation_fields_for_non_drained_exit_kinds() {
        let f = frontier();
        f.set_epoch_width(1);
        f.stamp_tombstone("m", "k1", "TAG1"); // epoch 1
        f.stamp_tombstone("m", "k2", "TAG2"); // rolls past epoch 1: refs_at_entry = 1

        let exits = {
            let mut state = f.lock();
            state.rebuild_into_epoch(100, Vec::new())
        };
        assert_eq!(exits.len(), 1, "only epoch 1 had an entry-emitted slot");
        let exit = exits.into_iter().next().expect("checked len == 1");

        assert!(matches!(exit.exit_kind, EpochExitKind::ClearedByRebuild));
        assert!(
            exit.refs_at_entry > 0,
            "R_ent must be > 0 for the zero below to be a meaningful check: {exit:?}"
        );
        assert_eq!(
            exit.removed_refs_observed, 0,
            "R1.5: not a DrainedByPrune exit: {exit:?}"
        );
        assert_eq!(
            exit.removed_bytes_observed, 0,
            "R1.5: not a DrainedByPrune exit: {exit:?}"
        );
    }

    /// R1.5 (second case): a `DrainedByPrune` attribution whose epoch is absent from the
    /// carried per-epoch map -- reachable only via a direct bypass, because
    /// `drain_prunable`'s `Some(refs)` arm is the map's only writer (K1) and always supplies
    /// an entry for any epoch it attributes `DrainedByPrune` to. Exercised directly here so
    /// the branch stays observable rather than merely declared.
    #[test]
    fn finalize_epoch_exit_zeroes_observation_fields_when_the_epoch_is_absent_from_the_carried_map()
    {
        let f = frontier();
        f.set_epoch_width(1);
        f.stamp_tombstone("m", "k1", "TAG1"); // epoch 1
        f.stamp_tombstone("m", "k2", "TAG2"); // rolls past epoch 1: refs_at_entry = 1

        let exit = {
            let mut state = f.lock();
            state.epoch_tags.remove(&1);
            state.detect_epoch_exit(1, Some(FinalExitKind::DrainedByPrune), None)
        }
        .expect("epoch 1 is absent from epoch_tags and must surface an exit row");

        assert!(matches!(exit.exit_kind, EpochExitKind::DrainedByPrune));
        assert!(exit.refs_at_entry > 0, "R_ent must be > 0: {exit:?}");
        assert_eq!(
            exit.removed_refs_observed, 0,
            "R1.5: epoch absent from the carried map: {exit:?}"
        );
        assert_eq!(
            exit.removed_bytes_observed, 0,
            "R1.5: epoch absent from the carried map: {exit:?}"
        );
    }

    /// AC5/X21-c -- the two new pinned counters reach a rendered `/metrics` scrape and carry
    /// the drain's own observed totals, through the full public pipeline
    /// (`drain_prunable_tombstones` → `publish_epoch_exit` → `observe_epoch_residency`).
    /// Sited here, not in `src/sim`, because the metrics recorder binding is thread-local
    /// (C13, X21-d).
    #[test]
    fn removed_observation_counters_reach_the_rendered_metrics_scrape() {
        let rendered = rendered_under_a_recorder(|| {
            let f = frontier();
            f.set_epoch_width(1);
            f.stamp_tombstone("m", "k1", "TAG1"); // epoch 1: 1 ref, 4 bytes
            f.stamp_tombstone("m", "k2", "TAG2"); // rolls past epoch 1

            f.set_durable_epoch_watermark(1);
            f.set_delivered(CONN_A, 100);
            let c: ClientId = "a5:metrics-observed|dev-1".into();
            assert!(block_on(f.confirm_apply_ack(&c, 2, CONN_A)));

            let drained = f.drain_prunable_tombstones();
            assert_eq!(drained.len(), 1, "only epoch 1 clears both conjuncts");
        });

        assert_eq!(
            rendered_value(&rendered, METRIC_PRUNE_REMOVED_REFS_OBSERVED_TOTAL),
            Some("1"),
            "epoch 1's drain observed exactly one removed ref; render was:\n{rendered}"
        );
        assert_eq!(
            rendered_value(&rendered, METRIC_PRUNE_REMOVED_BYTES_OBSERVED_TOTAL),
            Some("4"),
            "TAG1 is 4 bytes; render was:\n{rendered}"
        );
    }

    /// R1.6/R3.2 -- `MetricsPruneRecorder::observe_epoch_residency` credits the two new
    /// OBSERVATION counters on the `DrainedByPrune` arm ONLY: every other exit kind must move
    /// neither counter, even when the record itself carries nonzero observation fields (R1.5
    /// guarantees that never happens in production, but the observer's own crediting arm must
    /// not depend on that guarantee to stay correct).
    #[test]
    fn metrics_recorder_credits_removed_observation_counters_on_the_drained_arm_only() {
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        metrics::with_local_recorder(&recorder, || {
            let observer = MetricsPruneRecorder::new();
            for exit_kind in [
                EpochExitKind::ClearedByRebuild,
                EpochExitKind::StillResidentAtShutdown,
            ] {
                observer.observe_epoch_residency(&PruneEpochResidencyRecord {
                    exit_kind,
                    removed_refs_observed: 99,
                    removed_bytes_observed: 999,
                    ..PruneEpochResidencyRecord::default()
                });
            }
            observer.observe_epoch_residency(&PruneEpochResidencyRecord {
                exit_kind: EpochExitKind::DrainedByPrune,
                removed_refs_observed: 3,
                removed_bytes_observed: 12,
                ..PruneEpochResidencyRecord::default()
            });
        });

        let rendered = handle.render();
        assert_eq!(
            rendered_value(&rendered, METRIC_PRUNE_REMOVED_REFS_OBSERVED_TOTAL),
            Some("3"),
            "only the DrainedByPrune row's 3 refs must be credited; render was:\n{rendered}"
        );
        assert_eq!(
            rendered_value(&rendered, METRIC_PRUNE_REMOVED_BYTES_OBSERVED_TOTAL),
            Some("12"),
            "only the DrainedByPrune row's 12 bytes must be credited; render was:\n{rendered}"
        );
    }

    /// A thread-local `tracing` capture layer (X21-b, X21-d(iv)) -- this file's test module
    /// carried none before this segment. One entry per captured event, never a single
    /// concatenated `String`: both `S1` and the rendered-field-text proof below read PER-ROW
    /// terms off a SPECIFIC exit row, and a concatenated sink cannot individuate one row from
    /// the next.
    ///
    /// The `network/device_identity.rs:397-429` shape, adapted for a `Vec<String>` sink.
    struct FieldTextVisitor(String);

    impl tracing::field::Visit for FieldTextVisitor {
        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            use std::fmt::Write as _;
            let _ = write!(self.0, "{}={value} ", field.name());
        }

        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            use std::fmt::Write as _;
            let _ = write!(self.0, "{}={value:?} ", field.name());
        }
    }

    #[derive(Clone)]
    struct EventCapture(Arc<Mutex<Vec<String>>>);

    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for EventCapture {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            // The target is metadata, not a recorded field -- prefixed by hand so a
            // captured row stays attributable to a specific `tracing` target rather than
            // just whatever field text it happens to carry.
            let mut visitor = FieldTextVisitor(format!("target={} ", event.metadata().target()));
            event.record(&mut visitor);
            self.0.lock().unwrap().push(visitor.0);
        }
    }

    /// Bind a fresh thread-local `tracing` capture layer, run `body`, and return every
    /// captured event's rendered field text, one entry per event (X21-d(iv)).
    fn captured_tracing_events(body: impl FnOnce()) -> Vec<String> {
        use tracing_subscriber::layer::SubscriberExt as _;
        let sink: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::registry().with(EventCapture(Arc::clone(&sink)));
        let _guard = tracing::subscriber::set_default(subscriber);
        body();
        let events = sink.lock().unwrap().clone();
        events
    }

    /// X21-b -- the two new fields reach the residency EXIT row's `tracing` transport BY
    /// NAME: a capture over a real `DrainedByPrune` exit shows `removed_refs_observed=` and
    /// `removed_bytes_observed=` in the rendered field text (the `name=value` pairs a `Visit`
    /// collector accumulates), proven independently of whether the two terms happen to
    /// diverge from their attribution twins -- `S1` below is this limb's SECOND executed
    /// proof, the one that also diverges.
    #[test]
    fn removed_observation_fields_appear_by_name_on_the_captured_exit_row() {
        let f = frontier();
        f.set_epoch_width(1);
        f.stamp_tombstone("m", "k1", "TAG1"); // epoch 1: 1 ref, 4 bytes
        f.stamp_tombstone("m", "k2", "TAG2"); // rolls past epoch 1

        f.set_durable_epoch_watermark(1);
        f.set_delivered(CONN_A, 100);
        let c: ClientId = "a5:field-text|dev-1".into();
        assert!(block_on(f.confirm_apply_ack(&c, 2, CONN_A)));

        let events = captured_tracing_events(|| {
            let drained = f.drain_prunable_tombstones();
            assert_eq!(drained.len(), 1);
        });

        let exit_row = events
            .iter()
            .find(|e| e.contains("kind=epoch_exit") && e.contains("epoch=1 "))
            .unwrap_or_else(|| {
                panic!("no epoch_exit row for epoch 1 in captured events:\n{events:#?}")
            });

        assert!(
            exit_row.contains("removed_refs_observed=1"),
            "row: {exit_row}"
        );
        assert!(
            exit_row.contains("removed_bytes_observed=4"),
            "row: {exit_row}"
        );
    }

    /// `S1` -- THE PRE-FREEZE SANITY ROW (X21 applied to the PREDICATE itself, C12).
    ///
    /// Establishes that `removed_refs_observed` / `removed_bytes_observed` CAN diverge from
    /// `refs_at_entry` / `bytes_freed_attributed` on a REAL `DrainedByPrune` exit row,
    /// constructed entirely through PUBLIC entry points (bar the `#[cfg(test)]` watermark
    /// injector every test in this file already uses) -- a discriminator nobody has shown to
    /// discriminate is exactly the failure class Audit v1 C1 caught.
    ///
    /// `drain_prunable_tombstones` (the only PUBLIC drain entry point) returns
    /// `Vec<(Epoch, TombstoneRef)>` and no record (R1.7, X21-a), so this test reads the exit
    /// row off the declared `tracing` capture rather than off a return value; the returned
    /// `Vec` is asserted separately, as a count check only.
    #[test]
    fn s1_pre_freeze_sanity_row_diverges_through_public_entry_points_only() {
        let f = frontier();
        // 1. one epoch per stamp.
        f.set_epoch_width(1);
        // 2. epoch 1 holds 1 ref; slot created.
        f.stamp_tombstone("m", "k1", "TAG1");
        // 3. rolls past epoch 1 -> entry row fires; slot.refs_at_entry = 1, entry_emitted =
        // true.
        f.stamp_tombstone("m", "k2", "TAG2");
        // 4. epoch_tags[1] now holds 2 refs; epoch_slots[1] (and slot.refs_at_entry) is
        // untouched -- this is C12's divergence mechanism.
        f.restore_tombstone_ref(
            1,
            TombstoneRef {
                map: "m".to_string(),
                key: "k1".to_string(),
                tag: "TAGX".to_string(),
            },
        );
        // 5. REQUIRED PREREQUISITE: `advance_on_ack` clamps by `delivered.unwrap_or(0)`; with
        // no prior `set_delivered` the bound is 0, step 6 returns false, and step 8 drains
        // nothing.
        f.set_delivered(CONN_A, 100);
        let c: ClientId = "a5:s1-sanity|dev-1".into();
        // 6. bound = min(claimed 2, delivered 100, current_max_epoch 2) = 2; LWM 2 > 1 ->
        // epoch 1 prune-eligible.
        assert!(block_on(f.confirm_apply_ack(&c, 2, CONN_A)));
        // 7. second call-site conjunct.
        f.set_durable_epoch_watermark(1);

        // 8. drain via the PUBLIC entry point; observed off the captured exit row (X21-b).
        let events = captured_tracing_events(|| {
            let drained = f.drain_prunable_tombstones();
            assert_eq!(
                drained.len(),
                2,
                "epoch 1's two refs (stamped TAG1, restored TAGX) both drain"
            );
        });

        let exit_row = events
            .iter()
            .find(|e| e.contains("kind=epoch_exit") && e.contains("epoch=1 "))
            .unwrap_or_else(|| {
                panic!("no epoch_exit row for epoch 1 in captured events:\n{events:#?}")
            });

        assert!(
            exit_row.contains("exit_kind=DrainedByPrune"),
            "row: {exit_row}"
        );
        // R_ent == 1 (only TAG1 was ever STAMPED into epoch 1 before rollover).
        assert!(exit_row.contains("refs_at_entry=1"), "row: {exit_row}");
        // R_obs == 2 (TAG1 stamped + TAGX restored, both resident at drain time):
        // R_obs == R_ent + 1, the discriminating assertion (C12). The message names that
        // assertion literally, because the mutation arm's whole proof is that THIS assertion
        // is the one the mutation kills — a failure that did not name it would not show that.
        assert!(
            exit_row.contains("removed_refs_observed=2"),
            "discriminating assertion R_obs == R_ent + 1 does not hold; row: {exit_row}"
        );
        // B_att == 4 (TAG1's own stamped bytes; TAGX was restored, never stamped).
        assert!(
            exit_row.contains("bytes_freed_attributed=4"),
            "row: {exit_row}"
        );
        // B_obs == 8 == B_att + len("TAGX") == 4 + 4.
        assert!(
            exit_row.contains("removed_bytes_observed=8"),
            "row: {exit_row}"
        );
    }

    /// Step 0 leg (c) -- the V1 POSITIVE CONTROL (C11). `R_obs < R_ent` is not constructible
    /// through the public API at all (C4's four-site mutation enumeration), so this plants the
    /// antecedent directly via the same in-module `epoch_tags` bypass this file already uses
    /// elsewhere (e.g. the tier-1 walk's class (c), the unenumerated-removal test above), then
    /// drains IN-MODULE (`f.lock().drain_prunable()`, the one in-crate call that returns the
    /// exit rows BY VALUE) and asserts on the RETURNED record -- X21-a's sole workable proof.
    ///
    /// This proves the V1 predicate FIRES when its antecedent exists; it is NOT evidence about
    /// the prune itself, and its rows are excluded from every Decision-Table universe (the
    /// frozen scoping in Step 0).
    #[test]
    fn step0_leg_c_v1_positive_control_planted_ref_loss_fires_on_the_returned_record() {
        let f = frontier();
        f.set_epoch_width(1);
        f.stamp_tombstone("m", "k1", "TAG1"); // epoch 1
        f.stamp_tombstone("m", "k2", "TAG2"); // rolls past epoch 1: refs_at_entry = 1
        f.set_durable_epoch_watermark(1);
        f.set_delivered(CONN_A, 100);
        let c: ClientId = "a5:leg-c|dev-1".into();
        assert!(block_on(f.confirm_apply_ack(&c, 2, CONN_A)));

        let exits = {
            let mut state = f.lock();
            // Plant the V1 antecedent (C11): epoch 1's index content is replaced with an
            // EMPTY vector -- entry-emitted and still eligible, so the drain's own eligible
            // fold genuinely removes it (`epoch_tags.remove(&1)` returns `Some`, a real
            // `DrainedByPrune` attribution) while observing NOTHING in the vector it
            // removed.
            state.epoch_tags.insert(1, Vec::new());
            let (_drained, _split, exits) = state.drain_prunable();
            exits
        };
        assert_eq!(exits.len(), 1, "only epoch 1 was eligible");
        let exit = exits.into_iter().next().expect("checked len == 1");

        assert!(
            matches!(exit.exit_kind, EpochExitKind::DrainedByPrune),
            "the eligible fold genuinely removed epoch 1 (an empty Vec, but Some), so this \
             is a real DrainedByPrune exit, not Unclassified: {exit:?}"
        );
        assert!(exit.refs_at_entry > 0, "R_ent must be > 0: {exit:?}");
        assert_eq!(
            exit.removed_refs_observed, 0,
            "R_obs must be 0: the planted vector carried nothing for drain_prunable to \
             observe: {exit:?}"
        );
    }
}

#[cfg(all(test, feature = "redb"))]
mod persistence_tests {
    use super::*;
    use crate::storage::datastores::{RedbDataStore, WriteBehindConfig, WriteBehindDataStore};

    const CONN_A: ConnectionId = ConnectionId(1);

    fn temp_store() -> (std::path::PathBuf, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("frontier.redb");
        (path, dir)
    }

    fn ormap_with_tombstones(tags: &[&str]) -> RecordValue {
        RecordValue::OrMap {
            records: Vec::new(),
            tombstones: tags.iter().map(|t| (*t).to_string()).collect(),
        }
    }

    /// The persisted cursor survives a full store close + reopen (redb durability),
    /// and rehydration loads it back into a fresh frontier.
    #[tokio::test]
    async fn cursor_survives_restart_via_redb() {
        let (path, _dir) = temp_store();
        let c: ClientId = "a5:alice|dev-1".into();

        // First "process": advance + persist, then drop the store (release lockfile).
        {
            let store = Arc::new(RedbDataStore::new(&path).expect("open"));
            let f = TombstoneFrontier::new(Some(store));
            f.set_delivered(CONN_A, 100);
            assert!(f.confirm_apply_ack(&c, 77, CONN_A).await);
            // Cursor persistence is offloaded to a background worker; stop it and
            // await its exit so the write lands AND the store handle is released
            // before the file is reopened below.
            f.shutdown().await;
        }

        // Second "process": a brand-new frontier over the same file. Before rehydrate
        // the client is unknown; after rehydrate its cursor is restored.
        let store = Arc::new(RedbDataStore::new(&path).expect("reopen"));
        let f = TombstoneFrontier::new(Some(store));
        assert!(!f.is_tracked(&c), "unknown before rehydrate");
        f.rehydrate(&c).await;
        assert_eq!(
            f.cursor(&c),
            Some(77),
            "cursor restored from redb after restart"
        );
    }

    /// On reconnect, a KNOWN identity is rehydrated into the frontier BEFORE any ACK,
    /// so the LWM is pinned at its true cursor and does not spuriously jump forward
    /// (the reconnect-before-ACK window must not fall through unknown==forgotten).
    #[tokio::test]
    async fn reconnect_rehydrates_before_ack_does_not_advance_lwm() {
        let (path, _dir) = temp_store();
        let lagging: ClientId = "a5:alice|dev-lag".into();
        let ahead: ClientId = "a5:alice|dev-ahead".into();

        {
            let store = Arc::new(RedbDataStore::new(&path).expect("open"));
            let f = TombstoneFrontier::new(Some(store));
            f.set_delivered(CONN_A, 1000);
            assert!(f.confirm_apply_ack(&lagging, 5, CONN_A).await);
            assert!(f.confirm_apply_ack(&ahead, 500, CONN_A).await);
            // Offloaded persistence: stop the worker and release the store handle
            // before the redb file is reopened below.
            f.shutdown().await;
        }

        // Fresh frontier (server restarted). The lagging device reconnects; rehydrate
        // MUST run before any ACK so the LWM is pinned at 5, not jumped to the ahead
        // device's cursor / vacuous MAX.
        let store = Arc::new(RedbDataStore::new(&path).expect("reopen"));
        let f = TombstoneFrontier::new(Some(store));
        f.rehydrate(&lagging).await;
        assert_eq!(
            f.low_water_mark(),
            5,
            "reconnect-before-ACK rehydration pins the LWM at the lagging cursor"
        );
        assert!(
            f.is_tracked(&lagging),
            "the reconnecting known device is tracked"
        );
        // A rehydrate is a claim site, not merely a cursor writer: a reconnecting laggard that
        // held the mark down while contributing no claim would leave the boundary free to walk
        // past the epochs it has not applied.
        assert_eq!(
            f.reclamation().live_claims(ClaimScope::Global),
            1,
            "the rehydrated device contributes a claim"
        );
        assert_eq!(
            f.reclamation().prune_ceiling(ClaimScope::Global),
            5,
            "and the claim sits at its rehydrated position, so the boundary is pinned there"
        );
    }

    /// A forget is DURABLE: `forget_client` deletes the persisted cursor, so a later
    /// `rehydrate` finds nothing and the client stays untracked (unknown → forgotten →
    /// full resync). If the durable row survived a forget, rehydrate would silently
    /// re-track the client at its stale cursor and drop the LWM below an already-pruned
    /// watermark → resurrection — this test guards that vector.
    #[tokio::test]
    async fn forget_client_deletes_durable_cursor_so_rehydrate_is_noop() {
        let (path, _dir) = temp_store();
        let c: ClientId = "a5:alice|dev-1".into();

        let store = Arc::new(RedbDataStore::new(&path).expect("open"));
        let f = TombstoneFrontier::new(Some(store));
        f.set_delivered(CONN_A, 100);
        assert!(f.confirm_apply_ack(&c, 50, CONN_A).await);
        assert_eq!(f.cursor(&c), Some(50), "cursor established + persisted");

        // Forget the client (342c RAM-pressure sacrifice). Must clear BOTH in-memory
        // and durable state.
        f.forget_client(&c).await;
        assert!(!f.is_tracked(&c), "forgotten client untracked in memory");
        // The claim leaves the fold in the same act: a forget that dropped the cursor but kept
        // the claim would pin the boundary on a device nothing tracks any more.
        assert_eq!(f.reclamation().live_claims(ClaimScope::Global), 0);
        assert_eq!(
            f.reclamation().min_live_claim(ClaimScope::Global),
            None,
            "the forgotten device's claim is released, not merely orphaned"
        );

        // Rehydrate must be a no-op — the durable row is gone, so the client does NOT
        // resurrect at its stale cursor.
        f.rehydrate(&c).await;
        assert!(
            !f.is_tracked(&c),
            "durable cursor deleted on forget → rehydrate cannot re-track the stale cursor"
        );
        assert_eq!(f.cursor(&c), None, "no stale cursor resurrected");
    }

    /// With no margin configured, the reclamation boundary and the fleet low-water mark decide
    /// every epoch identically — over the sequence where a boundary that did not track every
    /// cursor writer would part company with the mark.
    ///
    /// The steps are chosen for the cases that can break the agreement rather than the cases that
    /// cannot: a device joining BELOW the current proposal, a rehydrate re-admitting a laggard
    /// below it, and forgets raising it again. A cursor writer that recorded no claim would leave
    /// its device holding the mark down while the boundary walked past it, and the per-epoch
    /// comparison reds on every epoch between the two; a boundary that latched at its highest
    /// past value would red on every step where the mark falls.
    #[tokio::test]
    async fn boundary_and_low_water_mark_decide_every_epoch_alike_with_no_margin() {
        let (path, _dir) = temp_store();
        let laggard: ClientId = "a5:alice|dev-laggard".into();
        let middle: ClientId = "a5:alice|dev-middle".into();
        let leader: ClientId = "a5:alice|dev-leader".into();

        // Persist the laggard's cursor first, so the fresh frontier below has something to
        // rehydrate FROM — a rehydrate is the reconnect path and it is one of the claim sites
        // this agreement depends on.
        {
            let store = Arc::new(RedbDataStore::new(&path).expect("open"));
            let f = TombstoneFrontier::new(Some(store));
            f.set_delivered(CONN_A, 1000);
            assert!(f.confirm_apply_ack(&laggard, 5, CONN_A).await);
            f.shutdown().await;
        }

        let store = Arc::new(RedbDataStore::new(&path).expect("reopen"));
        let f = TombstoneFrontier::new(Some(store));
        f.set_delivered(CONN_A, 1000);
        assert_eq!(
            f.reclamation().margin_epochs(),
            0,
            "the agreement is an identity claim only while no margin is configured"
        );

        let agree = |step: &str, expected: Epoch| {
            // Hoisted: reading the ceiling is an observing call that republishes gauges, so a
            // per-epoch re-read would report one query per epoch compared.
            let ceiling = f.reclamation().prune_ceiling(ClaimScope::Global);
            let lwm = f.low_water_mark();
            assert_eq!(
                (ceiling, lwm),
                (expected, expected),
                "both boundaries must sit at {expected} after {step}"
            );
            for epoch in 0..=60 {
                assert_eq!(
                    ceiling > epoch,
                    lwm > epoch,
                    "the two boundaries disagree about epoch {epoch} after {step}"
                );
            }
        };

        agree("no device has reconnected yet", 0);
        assert!(f.confirm_apply_ack(&leader, 50, CONN_A).await);
        agree("the leading device ACKed", 50);
        assert!(f.confirm_apply_ack(&middle, 20, CONN_A).await);
        agree("a second device joined below the proposal", 20);
        f.rehydrate(&laggard).await;
        agree("a rehydrated laggard rejoined below the proposal", 5);
        f.forget_client(&laggard).await;
        agree("the rehydrated laggard was forgotten", 20);
        f.forget_client(&middle).await;
        agree("only the leading device is left", 50);
    }

    /// Capture the STORE-BACKED half of the prune-decision corpus, in the same line format as
    /// `tests::capture_prune_decision_corpus`: `fixture | epochs drained | tags dropped`.
    ///
    /// Both rows are cursor-writer fixtures whose own assertions stop at the cursor, so each one
    /// is followed here by the prune decision that cursor state licenses — that is the cell a
    /// change to the prune's authority could move. DECISION-NEUTRAL: it asserts nothing and
    /// re-points no predicate. Run with `-- --nocapture` to read the lines.
    #[tokio::test]
    async fn capture_prune_decision_corpus_durable() {
        // A rehydrated laggard must keep pinning the epochs it has not applied.
        {
            let (path, _dir) = temp_store();
            let lagging: ClientId = "a5:alice|dev-lag".into();
            let ahead: ClientId = "a5:alice|dev-ahead".into();
            {
                let store = Arc::new(RedbDataStore::new(&path).expect("open"));
                let f = TombstoneFrontier::new(Some(store));
                f.set_delivered(CONN_A, 1000);
                let _ = f.confirm_apply_ack(&lagging, 5, CONN_A).await;
                let _ = f.confirm_apply_ack(&ahead, 500, CONN_A).await;
                f.shutdown().await;
            }
            let store = Arc::new(RedbDataStore::new(&path).expect("reopen"));
            let f = TombstoneFrontier::new(Some(store));
            f.rehydrate(&lagging).await;
            // Stamp AFTER the rehydrate so the laggard's position is the only thing gating.
            f.set_epoch_width(1);
            for i in 0..7 {
                f.stamp_tombstone("m", &format!("k{i}"), &format!("{i}:0:n"));
            }
            f.set_durable_epoch_watermark(1000);
            let drained = f.drain_prunable_tombstones();
            println!(
                "{}",
                prune_decision_line(
                    "reconnect_rehydrates_before_ack_does_not_advance_lwm",
                    &drained
                )
            );
            f.shutdown().await;
        }
        // A forgotten client releases its pin, and an empty fold prunes NOTHING (vacuous case).
        {
            let (path, _dir) = temp_store();
            let c: ClientId = "a5:alice|dev-1".into();
            let store = Arc::new(RedbDataStore::new(&path).expect("open"));
            let f = TombstoneFrontier::new(Some(store));
            f.set_delivered(CONN_A, 100);
            let _ = f.confirm_apply_ack(&c, 50, CONN_A).await;
            f.forget_client(&c).await;
            f.rehydrate(&c).await;
            f.set_epoch_width(1);
            for i in 0..7 {
                f.stamp_tombstone("m", &format!("k{i}"), &format!("{i}:0:n"));
            }
            f.set_durable_epoch_watermark(1000);
            let drained = f.drain_prunable_tombstones();
            println!(
                "{}",
                prune_decision_line(
                    "forget_client_deletes_durable_cursor_so_rehydrate_is_noop",
                    &drained
                )
            );
            f.shutdown().await;
        }
    }

    /// The worker is ALIVE and processes the forget, but its `store.remove` FAILS
    /// (transient durable error). `forget_client` must still run its idempotent
    /// fallback delete: the worker reports the delete OUTCOME through the oneshot, so a
    /// failed durable delete is no longer silently signalled as success. Pre-fix the
    /// worker sent `()` unconditionally, so the fallback was skipped and the forgotten
    /// row could survive → stale-cursor resurrection on the next rehydrate.
    #[tokio::test]
    async fn forget_fallback_fires_when_worker_store_remove_fails() {
        use std::sync::atomic::Ordering::SeqCst;
        let (path, _dir) = temp_store();
        let c: ClientId = "a5:alice|dev-1".into();
        let inner: Arc<dyn MapDataStore> = Arc::new(RedbDataStore::new(&path).expect("open"));

        let faulty = GatedAddStore::new_failing_remove(Arc::clone(&inner));
        let remove_calls = faulty.remove_calls_handle();
        let store: Arc<dyn MapDataStore> = Arc::new(faulty);
        let f = TombstoneFrontier::new(Some(store));

        // Establish + persist a cursor (the persist path uses `add`, never `remove`).
        f.set_delivered(CONN_A, 100);
        assert!(f.confirm_apply_ack(&c, 50, CONN_A).await);
        f.quiesce_persists().await;

        // Count only the removes issued by the forget path.
        remove_calls.store(0, SeqCst);
        f.forget_client(&c).await;

        // The worker attempts the durable delete (fails), reports `false`, and
        // `forget_client` retries via its idempotent fallback → at least TWO attempts.
        // Pre-fix this was exactly ONE (worker signalled success → fallback skipped).
        let attempts = remove_calls.load(SeqCst);
        assert!(
            attempts >= 2,
            "worker delete + fallback delete must both be attempted when the worker's \
             store.remove fails; got {attempts} attempt(s)"
        );
        f.shutdown().await;
    }

    /// A freshly-minted identity has no persisted cursor: rehydrate is a no-op and it
    /// stays untracked (unknown → gated), pinning nothing.
    #[tokio::test]
    async fn fresh_identity_rehydrate_is_noop_stays_untracked() {
        let (path, _dir) = temp_store();
        let store = Arc::new(RedbDataStore::new(&path).expect("open"));
        let f = TombstoneFrontier::new(Some(store));
        let fresh: ClientId = "a5:alice|brand-new".into();
        f.rehydrate(&fresh).await;
        assert!(!f.is_tracked(&fresh), "no persisted cursor → untracked");
    }

    /// One-shot poison-purge: a cursor persisted
    /// under the PRE-bump keyspace name (before the cross-map ACK-inflation fix)
    /// is never read back after the version bump — an inflated pre-barrier cursor
    /// cannot silently resurrect via rehydration. The row is orphaned, not
    /// migrated; 342f's TTL sweep reclaims it later.
    #[tokio::test]
    async fn pre_bump_keyspace_cursor_is_never_rehydrated_after_version_bump() {
        const PRE_BUMP_CURSOR_MAP: &str = "_topgun_tombstone_cursors";
        assert_ne!(
            CURSOR_MAP, PRE_BUMP_CURSOR_MAP,
            "guard against an accidental revert of the keyspace version bump"
        );

        let (path, _dir) = temp_store();
        let c: ClientId = "a5:alice|dev-1".into();

        // Simulate an inflated cursor left over from BEFORE the version bump,
        // written directly under the retired pre-bump map name (not the live
        // `CURSOR_MAP` constant).
        let store = Arc::new(RedbDataStore::new(&path).expect("open"));
        let record = RecordValue::Lww {
            value: Value::Bytes(encode_epoch(999)),
            timestamp: Timestamp {
                millis: 1,
                counter: 0,
                node_id: String::new(),
            },
        };
        store
            .add(PRE_BUMP_CURSOR_MAP, &c, &record, 0, 1)
            .await
            .expect("write pre-bump row");

        let f = TombstoneFrontier::new(Some(store));
        f.rehydrate(&c).await;
        assert!(
            !f.is_tracked(&c),
            "a cursor under the retired pre-bump keyspace must never be rehydrated \
             into the live frontier — it is unreachable by construction, not honored"
        );
        assert_eq!(
            f.cursor(&c),
            None,
            "no inflated pre-bump cursor resurrected"
        );
    }

    /// `AC3d`: kill -9 recovery. The RAM epoch index is lost across a restart; the
    /// rebuild re-stamps every live tombstone into a fresh maximally-lagging
    /// `E_rec` that exceeds every persisted cursor epoch (the load-bearing term)
    /// and `ceil(flushed/EPOCH_WIDTH)`. Nothing is prune-eligible until clients
    /// re-confirm past `E_rec`; `effective_low_water_mark` is the durable-backed
    /// clamp every consumer reads.
    #[tokio::test]
    async fn ac3d_kill9_recovery_rebuilds_into_maximally_lagging_e_rec() {
        let (path, _dir) = temp_store();
        let store: Arc<dyn MapDataStore> = Arc::new(RedbDataStore::new(&path).expect("open"));

        // Durable live tombstones (survive the crash in redb): 3 tags over 2 keys.
        store
            .add("mymap", "k1", &ormap_with_tombstones(&["T1", "T2"]), 0, 1)
            .await
            .unwrap();
        store
            .add("mymap", "k2", &ormap_with_tombstones(&["T3"]), 0, 1)
            .await
            .unwrap();

        // A client confirmed up to epoch 50 pre-crash (durable cursor).
        let client: ClientId = "a5:alice|dev-1".into();
        {
            let f = TombstoneFrontier::new(Some(Arc::clone(&store)));
            f.set_delivered(CONN_A, 1000);
            assert!(f.confirm_apply_ack(&client, 50, CONN_A).await);
            assert_eq!(f.cursor(&client), Some(50));
            // The cursor persist is offloaded; checkpoint it so epoch 50 is durable
            // in the shared store before the recovery frontier rebuilds from it.
            f.quiesce_persists().await;
            // kill -9: drop the frontier — the RAM epoch index is gone.
        }

        // Fresh frontier over the same durable store: the RAM index is empty, so
        // the watermark is 0 (prune dark, gate transparent) BEFORE the rebuild —
        // the recovery-ordering invariant (R12(e)).
        let f = TombstoneFrontier::new(Some(Arc::clone(&store)));
        assert_eq!(
            f.durable_epoch_watermark(),
            0,
            "empty index → watermark 0 (dark) until the pre-listener rebuild completes"
        );
        assert!(
            !f.is_protection_active(),
            "protection is transparent until the rebuild runs"
        );

        // Rebuild (invoked in the pre-listener window by the bin).
        let e_rec = f.rebuild_from_durable_store().await.unwrap();

        // E_rec exceeds every persisted cursor epoch (50) AND ceil(flushed/WIDTH)=0.
        let width = f.epoch_width();
        assert!(
            e_rec > 50,
            "E_rec {e_rec} must exceed the max persisted cursor epoch 50 (the load-bearing term)"
        );
        assert!(
            e_rec > 0u64.div_ceil(width),
            "E_rec exceeds ceil(flushed/EPOCH_WIDTH)"
        );
        assert_eq!(
            e_rec, 51,
            "E_rec = 1 + max(cursor 50, flushed-epochs 0, hint 0)"
        );

        // The recovery epoch's bytes are already durable (redb), so the watermark
        // computes to E_rec — protection is now ACTIVE (gate + prune go live).
        assert_eq!(
            f.durable_epoch_watermark(),
            e_rec,
            "recovery epoch is byte-durable; the watermark is E_rec"
        );
        assert!(
            f.is_protection_active(),
            "protection active after the rebuild"
        );

        // No client has re-confirmed yet: LWM 0 → nothing prunable.
        assert_eq!(f.low_water_mark(), 0, "no client reconnected yet");
        assert!(
            f.drain_prunable_tombstones().is_empty(),
            "nothing prunable until every tracked client re-confirms past E_rec"
        );

        // The rehydrated client sits at its STALE cursor 50 (< E_rec): the whole
        // corpus stays pinned (no premature prune of a freshly-numbered epoch).
        f.rehydrate(&client).await;
        assert_eq!(
            f.low_water_mark(),
            50,
            "rehydrated at the stale pre-crash cursor"
        );
        assert!(
            f.drain_prunable_tombstones().is_empty(),
            "a stale cursor below E_rec pins the maximally-lagging recovery epoch"
        );

        // effective_LWM is the durable-backed clamp min(persisted_LWM, watermark).
        assert_eq!(
            f.effective_low_water_mark(),
            50,
            "effective LWM = min(persisted_LWM 50, durable_epoch_watermark E_rec) = 50"
        );
    }

    /// `AC3e`: activation end-to-end. With the REAL prefix-complete watermark, a
    /// full loop (write → remove → clients ACK past the epoch → bytes durable)
    /// actually PRUNES — inverting the 342b `AC3a` dark-mode test: dark while the
    /// tombstone is still buffered, then a genuine prune once its bytes flush.
    #[tokio::test]
    async fn ac3e_activation_end_to_end_prune_fires_with_real_watermark() {
        let (path, _dir) = temp_store();
        let inner: Arc<dyn MapDataStore> = Arc::new(RedbDataStore::new(&path).expect("open"));
        // 60s delays: writes stay buffered (not byte-durable) until we hard_flush.
        let config = WriteBehindConfig {
            write_delay_ms: 60_000,
            flush_interval_ms: 60_000,
            shutdown_timeout_ms: 5_000,
            ..WriteBehindConfig::default()
        };
        let store = WriteBehindDataStore::new(inner, config);
        let store_dyn: Arc<dyn MapDataStore> = Arc::clone(&store) as Arc<dyn MapDataStore>;
        let f = TombstoneFrontier::new(Some(Arc::clone(&store_dyn)));
        f.set_epoch_width(1);

        // Mirror the crdt OR_REMOVE path: write the tombstone bytes, then stamp.
        store_dyn
            .add("m", "k1", &ormap_with_tombstones(&["T1"]), 0, 1)
            .await
            .unwrap();
        let e1 = f.stamp_tombstone("m", "k1", "T1");
        store_dyn
            .add("m", "k2", &ormap_with_tombstones(&["T2"]), 0, 1)
            .await
            .unwrap();
        let e2 = f.stamp_tombstone("m", "k2", "T2");
        assert_eq!((e1, e2), (1, 2));

        // A client confirms PAST every stamped epoch (clamped to the max, 2).
        let c: ClientId = "a5:alice|dev-1".into();
        f.set_delivered(CONN_A, 100);
        assert!(f.confirm_apply_ack(&c, 100, CONN_A).await);
        assert_eq!(
            f.low_water_mark(),
            2,
            "client confirmed past every stamped epoch"
        );

        // DARK before byte durability: the tombstones are still buffered, so the
        // flushed watermark has not advanced — nothing prunes (this is the AC3a
        // conjunct, now gated on the REAL watermark, not a constant 0).
        assert!(
            f.drain_prunable_tombstones().is_empty(),
            "with the real watermark an un-flushed tombstone is NOT prunable"
        );
        assert!(
            !f.is_protection_active(),
            "no epoch is byte-durable yet → protection still transparent"
        );

        // Make the tombstone bytes durable in the inner store.
        store.hard_flush().await.unwrap();

        // ACTIVATION: LWM strictly past epoch 1 AND its bytes durable → epoch 1
        // PRUNES; epoch 2 is the current epoch (LWM not strictly past it), retained.
        assert!(
            f.is_protection_active(),
            "a byte-durable epoch activates protection"
        );
        let drained = f.drain_prunable_tombstones();
        let tags: Vec<&str> = drained.iter().map(|(_, r)| r.tag.as_str()).collect();
        assert_eq!(
            drained.len(),
            1,
            "epoch 1 (strictly below LWM 2, byte-durable) actually prunes with the real watermark"
        );
        assert_eq!(tags, vec!["T1"], "the drained tombstone is epoch 1's tag");
    }

    /// An UNTOUCHED legacy `OrTombstones` blob (never rewritten by any OR op) is
    /// excluded from the frontier epoch scan and NEVER becomes prune-eligible:
    /// (i) pre-prune, it is absent from the live set `scan_live_tombstones` builds;
    /// (ii) post-prune, it survives a sweep that reclaims a genuinely stamped epoch.
    #[tokio::test]
    async fn ac2_untouched_legacy_ortombstones_never_prune_eligible() {
        let (path, _dir) = temp_store();
        let store: Arc<dyn MapDataStore> = Arc::new(RedbDataStore::new(&path).expect("open"));

        // A modern OR-Map tombstone and an untouched legacy blob. The write path
        // never emits `OrTombstones`, but older servers persisted it — seed it
        // directly to model a pre-epoch corpus rehydrated on restart.
        let legacy = RecordValue::OrTombstones {
            tags: vec!["LEG".to_string()],
        };
        store
            .add("m", "modern", &ormap_with_tombstones(&["MOD"]), 0, 1)
            .await
            .unwrap();
        store.add("m", "legacy", &legacy, 0, 1).await.unwrap();

        // (i) Pre-prune: the epoch scan admits only the modern tag. The legacy blob
        // never enters the live set, so it can never be stamped into any epoch.
        let live = scan_live_tombstones(store.as_ref()).await.unwrap();
        let live_tags: Vec<&str> = live.iter().map(|r| r.tag.as_str()).collect();
        assert_eq!(
            live_tags,
            vec!["MOD"],
            "the untouched legacy OrTombstones tag is absent from the frontier live set"
        );

        // Rebuild stamps ONLY the modern tag into the recovery epoch. redb bytes
        // are durable, so the watermark reaches E_rec and protection activates.
        let f = TombstoneFrontier::new(Some(Arc::clone(&store)));
        f.set_epoch_width(1);
        let e_rec = f.rebuild_from_durable_store().await.unwrap();

        // Drive a client PAST E_rec so the stamped recovery epoch is prune-eligible.
        f.set_current_max_epoch(e_rec + 1);
        let c: ClientId = "a5:alice|dev-1".into();
        f.set_delivered(CONN_A, 1000);
        assert!(f.confirm_apply_ack(&c, e_rec + 1, CONN_A).await);

        // (ii) Post-prune: the sweep reclaims the stamped epoch, draining ONLY the
        // modern tag. The legacy blob was never stamped, so no sweep can reach it.
        let drained = f.drain_prunable_tombstones();
        let drained_tags: Vec<&str> = drained.iter().map(|(_, r)| r.tag.as_str()).collect();
        assert_eq!(
            drained_tags,
            vec!["MOD"],
            "only the stamped modern epoch is reclaimed; the legacy tag is never drained"
        );
        assert_eq!(
            store.load("m", "legacy").await.unwrap(),
            Some(legacy),
            "the untouched legacy blob survives a sweep that reclaims a stamped epoch"
        );
    }

    // -----------------------------------------------------------------------
    // R3(a) — persisted cursor is monotone by construction. Two advances reach
    // the worker OUT OF ORDER (a displaced owner's delayed persist racing the new
    // owner's higher persist). The single FIFO worker's high-water drops the
    // lower one, so the durable cursor never regresses. RED without the high-water
    // check: the worker would write 100 then 50 and the durable cursor would be 50.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn persist_cursor_monotone_out_of_order_lower_epoch_loses() {
        let (path, _dir) = temp_store();
        let store: Arc<dyn MapDataStore> = Arc::new(RedbDataStore::new(&path).expect("open"));
        let c: ClientId = "a5:alice|dev-1".into();
        let f = TombstoneFrontier::new(Some(Arc::clone(&store)));

        f.enqueue_advance_for_test(&c, 100);
        f.enqueue_advance_for_test(&c, 50);
        f.quiesce_persists().await;

        assert_eq!(
            load_cursor(store.as_ref(), &c).await.unwrap(),
            Some(100),
            "a lower racing advance must never overwrite a higher persisted cursor"
        );
        f.shutdown().await;
    }

    // -----------------------------------------------------------------------
    // R3(a) — cross-restart monotonicity. A previous process persisted a high
    // cursor; a fresh frontier's worker starts with an empty high-water. A lower
    // advance must not clobber the durable higher value: the worker SEEDS its
    // high-water from the store on first sight. RED without the seed: an empty
    // high-water treats 50 as an advance and clobbers the persisted 100.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn persist_cursor_seeds_high_water_from_store_no_restart_regression() {
        let (path, _dir) = temp_store();
        let store: Arc<dyn MapDataStore> = Arc::new(RedbDataStore::new(&path).expect("open"));
        let c: ClientId = "a5:alice|dev-1".into();
        persist_cursor(store.as_ref(), &c, 100).await.unwrap();

        let f = TombstoneFrontier::new(Some(Arc::clone(&store)));
        f.enqueue_advance_for_test(&c, 50);
        f.quiesce_persists().await;

        assert_eq!(
            load_cursor(store.as_ref(), &c).await.unwrap(),
            Some(100),
            "worker seeds high-water from the durable store → no cross-restart regression"
        );
        f.shutdown().await;
    }

    // -----------------------------------------------------------------------
    // R3(a) — a forget whose durable delete FAILS must RETAIN the worker's
    // high-water. Modelled with a "lost-ack" delete: the row IS removed durably
    // but the store reports Err, so the worker observes a failed forget delete
    // over an actually-emptied store. If the worker cleared its high-water on
    // that failure, a later LOWER advance for the same client would re-seed from
    // the now-empty store (seed 0) and persist the regressed cursor — dropping
    // the durable cursor below the value the client previously reached (and below
    // an already-pruned watermark). Clearing high-water only on a confirmed Ok
    // delete prevents that. RED without the fix: the durable cursor comes back
    // as Some(50).
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn forget_failed_ack_delete_retains_high_water_no_lower_reseed_regression() {
        let (path, _dir) = temp_store();
        let inner: Arc<dyn MapDataStore> = Arc::new(RedbDataStore::new(&path).expect("open"));
        // `remove` durably deletes the row but reports Err (lost ack) → the worker sees
        // a failed forget delete over an actually-emptied store.
        let store: Arc<dyn MapDataStore> =
            Arc::new(GatedAddStore::new_lossy_remove(Arc::clone(&inner)));
        let c: ClientId = "a5:alice|dev-1".into();
        let f = TombstoneFrontier::new(Some(store));

        // Establish a high durable cursor (100).
        f.enqueue_advance_for_test(&c, 100);
        f.quiesce_persists().await;
        assert_eq!(load_cursor(inner.as_ref(), &c).await.unwrap(), Some(100));

        // Forget: the worker's delete lands durably (row gone) but reports Err, so the
        // worker must RETAIN high-water[c]=100 rather than clearing it.
        f.forget_client(&c).await;
        f.quiesce_persists().await;
        assert_eq!(
            load_cursor(inner.as_ref(), &c).await.unwrap(),
            None,
            "the lost-ack delete still removed the durable row"
        );

        // A later LOWER advance (50) must NOT regress the durable cursor: the retained
        // high-water (100) drops it. Without the fix the cleared high-water re-seeds
        // from the emptied store (0) and persists 50.
        f.enqueue_advance_for_test(&c, 50);
        f.quiesce_persists().await;
        assert_eq!(
            load_cursor(inner.as_ref(), &c).await.unwrap(),
            None,
            "a failed-ack forget retains high-water so a lower re-advance cannot re-seed \
             from the emptied store and persist a regressed cursor"
        );
        f.shutdown().await;
    }

    // -----------------------------------------------------------------------
    // R3(b) — the durable persist is OFFLOADED off the ACK read loop. The
    // in-memory LWM advances synchronously in `confirm_apply_ack` (returns true,
    // cursor == 50) while the durable write is still parked inside the store's
    // `add`, proving the ACK path does not await the redb write. After releasing
    // the write and a durability checkpoint, the value is persisted.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn confirm_apply_ack_advances_lwm_without_awaiting_the_persist() {
        let (path, _dir) = temp_store();
        let inner: Arc<dyn MapDataStore> = Arc::new(RedbDataStore::new(&path).expect("open"));
        let gate = Arc::new(AddGate::new());
        let store: Arc<dyn MapDataStore> =
            Arc::new(GatedAddStore::new(Arc::clone(&inner), Arc::clone(&gate)));
        let c: ClientId = "a5:alice|dev-1".into();
        let f = TombstoneFrontier::new(Some(store));
        f.set_delivered(CONN_A, 100);

        // The in-memory cursor (live LWM) advances immediately, even though the
        // offloaded durable write is about to block.
        assert!(f.confirm_apply_ack(&c, 50, CONN_A).await);
        assert_eq!(
            f.cursor(&c),
            Some(50),
            "in-memory LWM advanced synchronously in confirm_apply_ack"
        );

        // The worker has reached the durable write and is parked inside add().
        gate.entered.notified().await;
        assert_eq!(
            load_cursor(inner.as_ref(), &c).await.unwrap(),
            None,
            "durable write has not completed while the in-memory LWM already reads 50"
        );

        // Release the write and checkpoint: now it is durable.
        gate.release.notify_one();
        f.quiesce_persists().await;
        assert_eq!(
            load_cursor(inner.as_ref(), &c).await.unwrap(),
            Some(50),
            "the offloaded persist lands after release + durability checkpoint"
        );
        f.shutdown().await;
    }

    // -----------------------------------------------------------------------
    // R3(a) — a transient seed-load error must NOT seed the worker high-water to
    // 0. If it did, a lower advance would clobber a higher cursor a previous
    // process persisted (a cross-restart monotonicity regression). The worker
    // instead defers the advance. RED without the fix: seed=0 → 50 > 0 → clobbers
    // the durable 100 down to 50.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn worker_defers_advance_on_seed_load_error_no_regression() {
        let (path, _dir) = temp_store();
        let inner: Arc<dyn MapDataStore> = Arc::new(RedbDataStore::new(&path).expect("open"));
        let c: ClientId = "a5:alice|dev-1".into();
        // A previous process persisted a high cursor.
        persist_cursor(inner.as_ref(), &c, 100).await.unwrap();

        // The worker's store fails every `load`, so the seed load errors.
        let store: Arc<dyn MapDataStore> =
            Arc::new(GatedAddStore::new_failing_load(Arc::clone(&inner)));
        let f = TombstoneFrontier::new(Some(store));

        f.enqueue_advance_for_test(&c, 50);
        f.quiesce_persists().await;

        // Read the true durable value directly from the inner store: unchanged.
        assert_eq!(
            load_cursor(inner.as_ref(), &c).await.unwrap(),
            Some(100),
            "a seed-load error defers the advance; the higher durable cursor is not clobbered"
        );
        f.shutdown().await;
    }

    // -----------------------------------------------------------------------
    // R3 forget-wins under concurrency (STRESS test): an ACK advance racing a
    // forget for the same client must never resurrect the durable cursor. Both
    // methods enqueue their durability op WHILE holding the frontier lock, so the
    // worker sees advances and forgets in the same order as their in-memory effects
    // — whenever the client ends untracked in memory, its durable row is gone too.
    // This exercises the concurrent path and guards against gross regressions; it is
    // NOT a deterministic RED (the pre-fix advance→enqueue window is only a few
    // instructions, so it is rarely hit). The fix's real guarantee is structural:
    // the enqueue is inside the frontier lock, so enqueue order == in-memory order.
    // -----------------------------------------------------------------------

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_confirm_then_forget_never_resurrects_durable_cursor() {
        for _ in 0..200 {
            let (path, _dir) = temp_store();
            let store: Arc<dyn MapDataStore> = Arc::new(RedbDataStore::new(&path).expect("open"));
            let c: ClientId = "a5:alice|dev-1".into();
            let f = Arc::new(TombstoneFrontier::new(Some(Arc::clone(&store))));
            f.set_delivered(CONN_A, 1000);

            let f1 = Arc::clone(&f);
            let c1 = c.clone();
            let t1 = tokio::spawn(async move {
                f1.confirm_apply_ack(&c1, 50, CONN_A).await;
            });
            let f2 = Arc::clone(&f);
            let c2 = c.clone();
            let t2 = tokio::spawn(async move {
                f2.forget_client(&c2).await;
            });
            let _ = tokio::join!(t1, t2);
            // Drain any racing advance the worker has not yet processed.
            f.quiesce_persists().await;

            // Whenever the forget won in memory (client untracked), the durable row
            // must be absent — a late advance must not have resurrected it. If the
            // advance re-tracked the client instead, that is legitimate re-admission
            // and the durable row is allowed.
            if !f.is_tracked(&c) {
                assert_eq!(
                    load_cursor(store.as_ref(), &c).await.unwrap(),
                    None,
                    "a forgotten client's durable cursor must not be resurrected by a racing advance"
                );
            }
            f.shutdown().await;
        }
    }

    // A one-shot gate letting a test park the worker inside the store's `add`.
    struct AddGate {
        /// Fired when a gated `add` is entered (before it delegates).
        entered: tokio::sync::Notify,
        /// A gated `add` awaits this before delegating to the inner store.
        release: tokio::sync::Notify,
    }

    impl AddGate {
        fn new() -> Self {
            Self {
                entered: tokio::sync::Notify::new(),
                release: tokio::sync::Notify::new(),
            }
        }
    }

    /// Test store wrapper that parks the FIRST `add` on a gate so a test can observe
    /// the in-memory LWM advancing while the durable write is still in flight. Every
    /// other method delegates to the inner store unchanged.
    struct GatedAddStore {
        inner: Arc<dyn MapDataStore>,
        gate: Arc<AddGate>,
        gate_add: std::sync::atomic::AtomicBool,
        gated_once: std::sync::atomic::AtomicBool,
        fail_load: std::sync::atomic::AtomicBool,
        /// When set, every `remove` fails (models a transient durable-delete error).
        fail_remove: std::sync::atomic::AtomicBool,
        /// When set, `remove` DELETES the row via the inner store but STILL reports
        /// `Err` — models a delete that landed durably but whose ack was lost (a
        /// transient error on the response path). Exercises the worker's
        /// clear-high-water-only-on-`Ok` invariant: the row is gone yet the worker saw
        /// a failure, so it must NOT re-seed a re-admission from the emptied store.
        lossy_remove: std::sync::atomic::AtomicBool,
        /// Counts `remove` attempts so a test can prove the forget fallback fired.
        remove_calls: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl GatedAddStore {
        fn new(inner: Arc<dyn MapDataStore>, gate: Arc<AddGate>) -> Self {
            Self {
                inner,
                gate,
                gate_add: std::sync::atomic::AtomicBool::new(true),
                gated_once: std::sync::atomic::AtomicBool::new(false),
                fail_load: std::sync::atomic::AtomicBool::new(false),
                fail_remove: std::sync::atomic::AtomicBool::new(false),
                lossy_remove: std::sync::atomic::AtomicBool::new(false),
                remove_calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            }
        }

        /// A wrapper whose `load` always errors (no add-gate) — for exercising the
        /// worker's seed-load-error path.
        fn new_failing_load(inner: Arc<dyn MapDataStore>) -> Self {
            Self {
                inner,
                gate: Arc::new(AddGate::new()),
                gate_add: std::sync::atomic::AtomicBool::new(false),
                gated_once: std::sync::atomic::AtomicBool::new(false),
                fail_load: std::sync::atomic::AtomicBool::new(true),
                fail_remove: std::sync::atomic::AtomicBool::new(false),
                lossy_remove: std::sync::atomic::AtomicBool::new(false),
                remove_calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            }
        }

        /// A wrapper whose `remove` always errors (no add-gate) — for exercising the
        /// worker-alive-but-`store.remove`-fails forget path. The returned counter
        /// counts every `remove` attempt.
        fn new_failing_remove(inner: Arc<dyn MapDataStore>) -> Self {
            Self {
                inner,
                gate: Arc::new(AddGate::new()),
                gate_add: std::sync::atomic::AtomicBool::new(false),
                gated_once: std::sync::atomic::AtomicBool::new(false),
                fail_load: std::sync::atomic::AtomicBool::new(false),
                fail_remove: std::sync::atomic::AtomicBool::new(true),
                lossy_remove: std::sync::atomic::AtomicBool::new(false),
                remove_calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            }
        }

        /// A wrapper whose `remove` DELETES the row through the inner store but then
        /// reports `Err` (delete landed, ack lost). Drives the worker's
        /// clear-high-water-only-on-`Ok` invariant: the durable row is gone yet the
        /// worker observed a failure.
        fn new_lossy_remove(inner: Arc<dyn MapDataStore>) -> Self {
            Self {
                inner,
                gate: Arc::new(AddGate::new()),
                gate_add: std::sync::atomic::AtomicBool::new(false),
                gated_once: std::sync::atomic::AtomicBool::new(false),
                fail_load: std::sync::atomic::AtomicBool::new(false),
                fail_remove: std::sync::atomic::AtomicBool::new(false),
                lossy_remove: std::sync::atomic::AtomicBool::new(true),
                remove_calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            }
        }

        /// Shared handle to the `remove` attempt counter (clone before wrapping the
        /// store in `Arc<dyn MapDataStore>`).
        fn remove_calls_handle(&self) -> Arc<std::sync::atomic::AtomicUsize> {
            Arc::clone(&self.remove_calls)
        }
    }

    #[async_trait::async_trait]
    impl MapDataStore for GatedAddStore {
        async fn add(
            &self,
            map: &str,
            key: &str,
            value: &RecordValue,
            expiration_time: i64,
            now: i64,
        ) -> anyhow::Result<()> {
            // Gate only the first add so the barrier/quiesce path is not blocked.
            if self.gate_add.load(std::sync::atomic::Ordering::SeqCst)
                && !self
                    .gated_once
                    .swap(true, std::sync::atomic::Ordering::SeqCst)
            {
                self.gate.entered.notify_one();
                self.gate.release.notified().await;
            }
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
            self.remove_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if self.fail_remove.load(std::sync::atomic::Ordering::SeqCst) {
                anyhow::bail!("GatedAddStore: simulated remove failure");
            }
            if self.lossy_remove.load(std::sync::atomic::Ordering::SeqCst) {
                // The delete DOES land durably, but the ack is "lost": report Err.
                self.inner.remove(map, key, now).await?;
                anyhow::bail!("GatedAddStore: simulated lost-ack remove (row deleted)");
            }
            self.inner.remove(map, key, now).await
        }
        async fn remove_backup(&self, map: &str, key: &str, now: i64) -> anyhow::Result<()> {
            self.inner.remove_backup(map, key, now).await
        }
        async fn load(&self, map: &str, key: &str) -> anyhow::Result<Option<RecordValue>> {
            if self.fail_load.load(std::sync::atomic::Ordering::SeqCst) {
                anyhow::bail!("GatedAddStore: simulated load failure");
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
            sink: &mut dyn crate::storage::map_data_store::LeafSink,
        ) -> anyhow::Result<()> {
            self.inner.enumerate_leaves(map, is_backup, sink).await
        }
        async fn scan_values(
            &self,
            map: &str,
            is_backup: bool,
            max_batch_cost: u64,
        ) -> anyhow::Result<crate::storage::map_data_store::ScanBatch> {
            self.inner.scan_values(map, is_backup, max_batch_cost).await
        }
        async fn scan_values_batched(
            &self,
            map: &str,
            is_backup: bool,
            cursor: crate::storage::map_data_store::ScanCursor,
            max_batch_cost: u64,
        ) -> anyhow::Result<crate::storage::map_data_store::ScanBatch> {
            self.inner
                .scan_values_batched(map, is_backup, cursor, max_batch_cost)
                .await
        }
        async fn remove_all(&self, map: &str, keys: &[String]) -> anyhow::Result<()> {
            self.inner.remove_all(map, keys).await
        }
        async fn list_maps(&self) -> anyhow::Result<Vec<String>> {
            self.inner.list_maps().await
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
        fn assigned_write_sequence(&self) -> u64 {
            self.inner.assigned_write_sequence()
        }
        fn flushed_watermark(&self) -> u64 {
            self.inner.flushed_watermark()
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
}
