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
use crate::tombstone_frontier::{CausalFrontier, ClientId, Epoch};
use std::sync::Arc;

/// Reserved redb map namespace for the durable confirmed-apply cursors.
///
/// A NEW additive keyspace — it does NOT repurpose the delta-sync
/// `last_sync_timestamp` hint. Kept clear of the user-map namespace by the
/// `_topgun_` convention (matches `is_valid_map_name`, does not end in
/// `__backup`). The record KEY is the opaque `ClientId` (`frontier_client_id`
/// encoding).
pub const CURSOR_MAP: &str = "_topgun_tombstone_cursors";

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
    /// defaults to `u64::MAX` (inert) until 342b provides the real counter.
    current_max_epoch: Epoch,
}

impl FrontierState {
    fn new() -> Self {
        Self {
            cursors: HashMap::new(),
            delivered: HashMap::new(),
            current_max_epoch: Epoch::MAX,
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
}

#[cfg(all(test, feature = "redb"))]
mod persistence_tests {
    use super::*;
    use crate::storage::datastores::RedbDataStore;

    const CONN_A: ConnectionId = ConnectionId(1);

    fn temp_store() -> (std::path::PathBuf, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("frontier.redb");
        (path, dir)
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
}
