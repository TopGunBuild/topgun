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

use tracing::{debug, warn};

use topgun_core::hlc::Timestamp;
use topgun_core::types::Value;

use crate::network::connection::ConnectionId;
use crate::storage::map_data_store::MapDataStore;
use crate::storage::record::RecordValue;
use crate::tombstone_frontier::{
    CausalFrontier, ClientId, Epoch, GateToken, PruneClaimSpanRecord, PruneEpochRecord,
    PrunePassRecord, PruneRecordArming, PruneRecordObserver, PruneSafety,
    METRIC_PRUNE_ABSENT_TOTAL, METRIC_PRUNE_BYTES_FREED_TOTAL, METRIC_PRUNE_CLAIM_LAG_EPOCHS,
    METRIC_PRUNE_CLAIM_SPAN_EPOCHS, METRIC_PRUNE_CONSIDERED_TOTAL, METRIC_PRUNE_CURRENT_EPOCH,
    METRIC_PRUNE_DRAIN_EPOCHS, METRIC_PRUNE_DRAIN_REFS, METRIC_PRUNE_DROPPED_TOTAL,
    METRIC_PRUNE_DURABLE_EPOCH_WATERMARK, METRIC_PRUNE_ELIGIBLE_REFS,
    METRIC_PRUNE_EMPTY_DRAINS_TOTAL, METRIC_PRUNE_EPOCHS_DRAINED_TOTAL,
    METRIC_PRUNE_EPOCH_BYTES_FREED, METRIC_PRUNE_EPOCH_CONSIDERED, METRIC_PRUNE_EPOCH_DROPPED,
    METRIC_PRUNE_INDEXED_EPOCHS, METRIC_PRUNE_INDEXED_REFS, METRIC_PRUNE_INELIGIBLE_REFS,
    METRIC_PRUNE_LAST_DRAINED_EPOCH, METRIC_PRUNE_LOW_WATER_MARK, METRIC_PRUNE_LWM_ADVANCES_TOTAL,
    METRIC_PRUNE_LWM_EPOCHS_ADVANCED_TOTAL, METRIC_PRUNE_LWM_STALL_SECONDS,
    METRIC_PRUNE_MATCHED_NOTHING_TOTAL, METRIC_PRUNE_NONEMPTY_DRAINS_TOTAL,
    METRIC_PRUNE_PASSES_TOTAL, METRIC_PRUNE_RESTORED_EVICTED_TOTAL,
    METRIC_PRUNE_RESTORED_READ_ERROR_TOTAL, METRIC_PRUNE_RESTORED_WRITE_ERROR_TOTAL,
    METRIC_PRUNE_SPLIT_COMPUTED_EPOCH, METRIC_PRUNE_SPLIT_RECOMPUTES_TOTAL,
    METRIC_PRUNE_TRACKED_CLAIMS,
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
}

impl FrontierState {
    fn new() -> Self {
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
        }
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
            _ => {
                self.cursors.insert(client.clone(), new);
                Some(new)
            }
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
    fn rehydrate(&mut self, client: &ClientId, epoch: Epoch) {
        if epoch == 0 {
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
    /// "no/uncomputable epoch" sentinel and is never stamped).
    fn stamp_tombstone(&mut self, map: &str, key: &str, tag: &str, write_seq: u64) -> Epoch {
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
        // Record the durability bound for this epoch: the highest write sequence
        // the store had assigned at stamp time. The epoch is byte-durable only
        // once the store's flushed watermark reaches this value.
        let slot = self.epoch_max_seq.entry(epoch).or_insert(0);
        *slot = (*slot).max(write_seq);
        epoch
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
        }
        advanced
    }

    /// Seconds since the last low-water-mark advance.
    fn lwm_stall_seconds(&self, now: u64) -> u64 {
        now.saturating_sub(self.last_lwm_advance_millis) / 1000
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
    fn rebuild_into_epoch(&mut self, e_rec: Epoch, live: Vec<TombstoneRef>) {
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
    }

    /// Drain the tombstone refs of every currently prune-eligible epoch out of the
    /// RAM index for the caller to drop from storage, under the FULL call-site
    /// conjunction `is_epoch_prune_eligible(E) && durable_epoch_watermark >= E`.
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
    fn drain_prunable(&mut self) -> (Vec<(Epoch, TombstoneRef)>, Option<SplitObservation>) {
        let watermark = self.durable_epoch_watermark;
        // Fast-path: a 0 watermark (no epoch byte-durable yet, or dark before the
        // recovery rebuild) means NO stamped epoch (all `>= 1`) can pass the
        // conjunction, so skip the per-epoch low-water-mark fold entirely — this
        // runs on every OR_REMOVE and every SYNC-leaf request.
        if watermark == 0 {
            return (Vec::new(), None);
        }
        let eligible: Vec<Epoch> = self
            .epoch_tags
            .keys()
            .copied()
            // Cheap watermark conjunct first so it short-circuits the LWM fold.
            .filter(|&e| watermark >= e && self.is_epoch_prune_eligible(e))
            .collect();
        // Gated on a non-empty eligible set, so the per-remove path — where the
        // drain finds nothing — still pays no index-proportional fold. This is the
        // same budget the post-loop recompute honoured; only the INSTANT moves.
        let pre_drain_split =
            (!eligible.is_empty()).then(|| self.split_observation(self.low_water_mark()));
        let mut drained = Vec::new();
        for e in eligible {
            if let Some(refs) = self.epoch_tags.remove(&e) {
                // Decrement by what this epoch actually held, so the carried count
                // stays exact without ever re-reading the rest of the index.
                self.indexed_refs = self
                    .indexed_refs
                    .saturating_sub(u64::try_from(refs.len()).unwrap_or(u64::MAX));
                self.last_drained_epoch = self.last_drained_epoch.max(e);
                drained.extend(refs.into_iter().map(|r| (e, r)));
            }
            self.epoch_max_seq.remove(&e);
        }
        (drained, pre_drain_split)
    }

    /// Re-insert a drained tombstone ref whose storage drop FAILED, so the tag is
    /// retried on a later sweep instead of being orphaned un-prunable in storage.
    /// The `epoch_max_seq` entry is re-created best-effort (the index is a pure
    /// RAM cache — the unclean-recovery rebuild is the authoritative
    /// recovery for any imprecision here).
    fn restore(&mut self, epoch: Epoch, tombstone_ref: TombstoneRef) {
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
    /// `OR_REMOVE`. The cursor fold is hoisted out of the epoch loop: calling
    /// `is_epoch_prune_eligible` per epoch would re-fold the cursor map once per
    /// epoch and turn this into O(epochs × claims) for an identical answer.
    /// `lwm` is passed in rather than re-folded: the LWM-movement callers have just
    /// computed it in `refresh_low_water_mark`, and folding the cursor map a second
    /// time under the same lock would triple the hold time in the cursor-count
    /// dimension for an identical answer.
    fn split_observation(&self, lwm: Epoch) -> SplitObservation {
        let watermark = self.durable_epoch_watermark;
        let current_epoch = self.current_epoch;
        let mut eligible_refs = 0u64;
        let mut ineligible_refs = 0u64;
        for (&epoch, refs) in &self.epoch_tags {
            let held = u64::try_from(refs.len()).unwrap_or(u64::MAX);
            // The FULL call-site conjunction the drain applies, with epoch 0 rejected
            // for the same belt-and-suspenders reason: a split computed under a weaker
            // predicate than the drain's would report refs as eligible that no pass
            // would ever take.
            if epoch != 0 && watermark >= epoch && lwm > epoch {
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
        // The 342a contract: fold over the low-water-mark ONLY. The durability fence
        // is the CALL-SITE second conjunct (`drain_prunable`), NEVER here. Epoch 0 is
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
        // fleet-wide MIN cursor >= N+1, i.e. every tracked client applied all of N.
        self.low_water_mark() > epoch
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
        Self {
            state: Mutex::new(FrontierState::new()),
            store,
            persist_tx,
            persist_worker,
            prune_observer,
        }
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
                    (epochs_advanced, state.split_observation(state.observed_lwm))
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
                        (epochs_advanced, state.split_observation(state.observed_lwm))
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
                (epochs_advanced, state.split_observation(state.observed_lwm))
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
        let (epoch, snapshot) = {
            let mut state = self.lock();
            let epoch = state.stamp_tombstone(map, key, tag, write_seq);
            (epoch, state.observation_snapshot())
        };
        // The stamp is the ONE path that grows the index, so it is where the index
        // and epoch gauges have to be refreshed for them to mean anything. Everything
        // published here is a stored value (carried ref count, `HashMap::len`, cached
        // low-water-mark), so the per-remove cost is a fixed handful of gauge stores
        // and no fold — the eligible/ineligible split deliberately does NOT run here.
        self.publish_frontier_state(&snapshot);
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
        let snapshot = {
            let mut state = self.lock();
            state.rebuild_into_epoch(e_rec, live);
            state.observation_snapshot()
        };
        // The rebuild replaces the whole index in the pre-listener recovery window,
        // so publishing here is what stops the first post-recovery scrape reporting
        // the pre-crash index size.
        self.publish_frontier_state(&snapshot);
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

    /// Whether `epoch` is prune-eligible under the low-water-mark fold ONLY (the
    /// 342a contract — STRICT: eligible once the LWM advanced PAST `epoch`). The
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
    /// conjuncts) out of the RAM index, tagged with their epoch, for the caller to
    /// drop from storage (RAM + redb) under the per-key writer. A ref whose storage
    /// drop fails MUST be handed back via [`Self::restore_tombstone_ref`] so it is
    /// retried later rather than orphaned un-prunable. DARK by construction:
    /// returns empty in production (`durable_epoch_watermark == 0`).
    #[must_use]
    pub fn drain_prunable_tombstones(&self) -> Vec<(Epoch, TombstoneRef)> {
        // Refresh the cached byte-durability watermark from the store's live
        // flushed watermark, then drain under BOTH call-site conjuncts. Reading
        // the store's watermark outside the lock keeps the frontier lock hold
        // short; the field is then updated and consumed under one lock.
        let flushed = self.store.as_ref().map(|s| s.flushed_watermark());
        let (drained, snapshot, split, last_advance_millis) = {
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
            let (drained, pre_drain_split) = state.drain_prunable();
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

    /// The 15 pinned counters.
    const PRUNE_COUNTER_NAMES: [&str; 15] = [
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

        // Rehydrate must be a no-op — the durable row is gone, so the client does NOT
        // resurrect at its stale cursor.
        f.rehydrate(&c).await;
        assert!(
            !f.is_tracked(&c),
            "durable cursor deleted on forget → rehydrate cannot re-track the stale cursor"
        );
        assert_eq!(f.cursor(&c), None, "no stale cursor resurrected");
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
