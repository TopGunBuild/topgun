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
//! production epoch-stamping provider exists yet (that is SPEC-342b's Wave-3
//! deliverable), so at Wave 2 `current_max_epoch` defaults to `u64::MAX` (the
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

use std::collections::HashMap;
use std::sync::Mutex;

use tracing::debug;

use topgun_core::hlc::Timestamp;
use topgun_core::types::Value;

use crate::network::connection::ConnectionId;
use crate::storage::map_data_store::MapDataStore;
use crate::storage::record::RecordValue;
use crate::tombstone_frontier::{CausalFrontier, ClientId, Epoch, GateToken, PruneSafety};
use std::sync::Arc;

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
/// `_v2`: one-shot poison-purge, version-bumped by the SPEC-342b cross-map
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
    /// hot path; unclean recovery rebuilds it in SPEC-342j). Keyed by the ACTUAL
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
        // Record the durability bound for this epoch: the highest write sequence
        // the store had assigned at stamp time. The epoch is byte-durable only
        // once the store's flushed watermark reaches this value.
        let slot = self.epoch_max_seq.entry(epoch).or_insert(0);
        *slot = (*slot).max(write_seq);
        epoch
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
        if !live.is_empty() {
            self.epoch_max_seq.insert(e_rec, 0);
            self.epoch_tags.insert(e_rec, live);
        }
        // Recomputes from the fresh index on the next watermark read.
        self.durable_epoch_watermark = 0;
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
    fn drain_prunable(&mut self) -> Vec<(Epoch, TombstoneRef)> {
        let watermark = self.durable_epoch_watermark;
        // Fast-path: a 0 watermark (no epoch byte-durable yet, or dark before the
        // recovery rebuild) means NO stamped epoch (all `>= 1`) can pass the
        // conjunction, so skip the per-epoch low-water-mark fold entirely — this
        // runs on every OR_REMOVE and every SYNC-leaf request.
        if watermark == 0 {
            return Vec::new();
        }
        let eligible: Vec<Epoch> = self
            .epoch_tags
            .keys()
            .copied()
            // Cheap watermark conjunct first so it short-circuits the LWM fold.
            .filter(|&e| watermark >= e && self.is_epoch_prune_eligible(e))
            .collect();
        let mut drained = Vec::new();
        for e in eligible {
            if let Some(refs) = self.epoch_tags.remove(&e) {
                drained.extend(refs.into_iter().map(|r| (e, r)));
            }
            self.epoch_max_seq.remove(&e);
        }
        drained
    }

    /// Re-insert a drained tombstone ref whose storage drop FAILED, so the tag is
    /// retried on a later sweep instead of being orphaned un-prunable in storage.
    /// The `epoch_max_seq` entry is re-created best-effort (the index is a pure
    /// RAM cache — SPEC-342j's unclean-recovery rebuild is the authoritative
    /// recovery for any imprecision here).
    fn restore(&mut self, epoch: Epoch, tombstone_ref: TombstoneRef) {
        self.epoch_tags
            .entry(epoch)
            .or_default()
            .push(tombstone_ref);
        self.epoch_max_seq.entry(epoch).or_insert(0);
    }
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
}

impl std::fmt::Debug for TombstoneFrontier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let tracked = self.state.lock().map(|s| s.cursors.len()).unwrap_or(0);
        f.debug_struct("TombstoneFrontier")
            .field("tracked_clients", &tracked)
            .field("has_store", &self.store.is_some())
            .finish()
    }
}

impl TombstoneFrontier {
    /// Build a frontier over an optional persistence backend.
    #[must_use]
    pub fn new(store: Option<Arc<dyn MapDataStore>>) -> Self {
        Self {
            state: Mutex::new(FrontierState::new()),
            store,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, FrontierState> {
        // A poisoned frontier mutex means a prior panic while holding it; recover the
        // guard rather than propagate the panic — the frontier is best-effort and a
        // stale-but-consistent snapshot is safe (cursor-loss degrades to resync).
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
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
    pub async fn confirm_apply_ack(
        &self,
        client: &ClientId,
        claimed: Epoch,
        conn: ConnectionId,
    ) -> bool {
        // Compute + apply the advance under the lock, then drop it BEFORE any await
        // (the std Mutex guard is not held across the persist await).
        let advanced = self.lock().advance_on_ack(client, claimed, conn);
        let Some(epoch) = advanced else {
            return false;
        };
        if let Some(store) = self.store.as_ref() {
            if let Err(e) = persist_cursor(store.as_ref(), client, epoch).await {
                // Best-effort: a failed persist is safe (cursor-loss → resync). The
                // in-memory advance already happened, so the live LWM is correct until
                // restart; only durability across restart is affected.
                debug!(client = %client, epoch, "cursor persist failed: {e}");
            }
        }
        true
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
            Ok(Some(epoch)) => self.lock().rehydrate(client, epoch),
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
    /// watermark is non-zero (SPEC-342j activation). While it is 0 (dark by
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
        self.lock().forget_client(client);
        if let Some(store) = self.store.as_ref() {
            let now = i64::try_from(now_millis()).unwrap_or(i64::MAX);
            if let Err(e) = store.remove(CURSOR_MAP, client, now).await {
                debug!(client = %client, "cursor forget delete failed: {e}");
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
        self.lock().stamp_tombstone(map, key, tag, write_seq)
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
        self.lock().rebuild_into_epoch(e_rec, live);
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
        let mut state = self.lock();
        if let Some(flushed) = flushed {
            let computed = state.compute_durable_epoch_watermark(flushed);
            state.durable_epoch_watermark = state.durable_epoch_watermark.max(computed);
        }
        state.drain_prunable()
    }

    /// Re-insert a drained tombstone ref whose storage drop FAILED (see
    /// [`Self::drain_prunable_tombstones`]). The index entry is restored so a later
    /// sweep retries the drop; `epoch_max_seq` is re-created best-effort (pure RAM
    /// cache — SPEC-342j's rebuild is the authoritative recovery).
    pub fn restore_tombstone_ref(&self, epoch: Epoch, tombstone_ref: TombstoneRef) {
        self.lock().restore(epoch, tombstone_ref);
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
                    // recovery epoch, so it can never become prune-eligible. An
                    // explicit no-op arm keeps that invariant visible at the point
                    // of change — a future "handle every variant" refactor cannot
                    // silently pull legacy blobs into the epoch index. (A later OR
                    // write to the key upconverts the record to `OrMap`, after
                    // which its tags join the protected regime — expected.)
                    RecordValue::OrTombstones { .. } => {}
                    RecordValue::Lww { .. } => {}
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

    const CONN_A: ConnectionId = ConnectionId(1);
    const CONN_B: ConnectionId = ConnectionId(2);

    fn frontier() -> TombstoneFrontier {
        // No persistence: exercises the in-memory advance logic. The global bound is
        // inert (u64::MAX) so `delivered_conn` is the operative clamp, matching Wave 2.
        TombstoneFrontier::new(None)
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

    /// One-shot poison-purge (SPEC-342b Review v1 Major fix): a cursor persisted
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
}
