//! Prune-safety contract for bounding OR-Map tombstone growth (traits/types only).
//!
//! This module is the **design contract** produced by the SPEC-339 mechanism-selection
//! decision. It carries NO implementation body, NO write-path wiring, and NO prune logic —
//! those land in the downstream implementation spec. It exists so 565 (tombstone-bytes counter
//! placement) and 563 (harness-trust threshold) can depend on a fixed representation, and so the
//! downstream implementer fills in these signatures against a doc-contract that cannot be
//! misread.
//!
//! # Selected mechanism (SPEC-339)
//!
//! **M4 — epoch/generation GC.** The single-node authority rotates a server-authoritative epoch
//! counter unilaterally. Each tombstone is associated with the server epoch current **at the
//! moment the server applies its `OR_REMOVE`** (server-side metadata; the wire tombstone shape is
//! unchanged). An entire epoch's tombstone set is dropped wholesale once every tracked client has
//! advanced past that epoch. Rejected alternatives: M2 (ORSWOT/dot-store — does not bound growth;
//! version-vector compression is unsound over HLC-reset gapped counters), M3 (spill-to-disk —
//! bounds RAM only, unbounded disk). The epoch is stamped server-side at remove-apply time, NOT
//! derived from the client-supplied tag `millis`, because the tag's `millis` is the *add* time
//! (not the remove-delivery time) and is client-verbatim on the push-diff path — deriving the
//! epoch from it would mis-bucket under clock skew and admit resurrection.
//!
//! # The ONE invariant this contract protects
//!
//! No resurrection of a removed value for a **known, monotone** client (monotone = local replica
//! only moves forward, never rolled back — assumption A6; backup-restore regression is out of
//! scope). A resurrection is fleet-wide silent CRDT corruption: a re-admitted record broadcasts
//! to every connected client and CRDT merge has no quarantine.
//!
//! # Prune-side safety condition (HARD — all tracked clients, never per-client)
//!
//! A tombstone / epoch is prune-eligible **ONLY** once the *low-water-mark* — the **minimum**
//! per-client high-water-mark taken across **ALL tracked (non-forgotten) clients** — has advanced
//! past it. This is distinct from the per-client *advancement*-safety condition (which governs
//! WHEN one client's cursor moves): the prune-safety condition governs WHOSE confirmation licenses
//! reclamation. It is **NEVER** a per-creating-client or single-client frontier — a single tracked
//! client whose cursor is behind an epoch pins that epoch for the whole fleet. See
//! [`PruneSafety::is_epoch_prune_eligible`].

/// Server-authoritative monotonic epoch / generation counter.
///
/// Owned by the single-node authority and stamped onto a tombstone at the moment the server
/// applies the `OR_REMOVE` that created it. Never derived from a client-supplied tag `millis`.
pub type Epoch = u64;

/// A tracked client's identity for frontier bookkeeping.
///
/// This is the **server-authenticated connection identity** (JWT principal / connection
/// registry) — NOT a tag-embedded, client-asserted nodeId. Keying the frontier and the reconnect
/// gate off a spoofable, client-asserted identity would let a forgotten client re-present as a
/// known-non-forgotten client and defeat the safety condition (see the module invariant and
/// assumption A6's adjacent authentication precondition).
pub type ClientId = String;

/// A per-client high-water-mark: the highest [`Epoch`] a client has **confirmed
/// receive-and-apply** of.
///
/// See [`CausalFrontier::confirm_apply`] for the advancement contract.
pub type EpochCursor = Epoch;

/// An OR-Map tombstone tag, the verbatim `"millis:counter:nodeId"` string used as the
/// observed-remove identity. Carried unchanged on the wire; the epoch association is server-side
/// metadata alongside it.
pub type TombstoneTag = String;

/// Max-retention policy for the forgotten-client sacrifice.
///
/// Retention is expressed in **cursor lag (epoch count)**, NOT a fixed wall-clock duration: lag is
/// what actually pins tombstones, it decouples from the epoch width, and it lets RAM pressure act
/// as a real lever (a fixed wall-clock `R` gives RAM pressure no counterweight and would lock out
/// a cursor-current-but-idle client for no benefit).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaxRetention {
    /// Forget a tracked client once its cursor lags more than this many epochs behind the current
    /// server epoch. RAM pressure MAY dynamically tighten this (forget laggier clients sooner)
    /// when un-pruned tombstone cost approaches `TOPGUN_MAX_RAM_MB`.
    pub max_lag_epochs: u32,
    /// Optional wall-clock ceiling (ms of silence) as a secondary bound against a client that
    /// never advances at all. `None` = no secondary ceiling.
    pub wall_clock_ceiling_ms: Option<u64>,
}

/// Opaque token proving a reconnect payload passed the pre-apply gate's not-forgotten check at
/// gate time.
///
/// Handed from the gate to the merge-commit so the merge can re-assert that the gate-time decision
/// still holds (see [`PruneSafety::gate_decision_holds_at_commit`]). The downstream implementation
/// defines its representation; this contract only fixes that such a token flows gate → commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateToken {
    /// The client whose not-forgotten status the gate certified.
    pub client: ClientId,
    /// The current epoch observed at gate time; the merge-commit re-checks that no prune sweep has
    /// since advanced the low-water-mark past a tombstone this payload still needs.
    pub epoch_at_gate: Epoch,
}

/// Per-client causal frontier: the tracked cursors whose minimum licenses pruning.
///
/// Implemented downstream over the per-client high-water-marks. The doc-contract on each method is
/// the load-bearing part — an implementer MUST honour it.
pub trait CausalFrontier {
    /// Record that `client` has **confirmed receive-AND-apply** up to `epoch`.
    ///
    /// # Contract
    /// - Advances **only** on confirmed receive-**and-apply** of the covered tombstone/epoch state
    ///   — **never** on sync-initiation and **never** on mere receive/delivery (an apply-lagging
    ///   client that confirmed on receive could still push a pre-apply diff and resurrect).
    /// - Monotone: a cursor never moves backward. A replayed or reordered confirmation carrying a
    ///   cursor `<=` the current one is a no-op (the ACK protocol is a cumulative monotonic cursor,
    ///   not per-tombstone ACKs, precisely so replay/reorder cannot drive a premature advance).
    /// - The confirmation MUST be bound to the server-authenticated [`ClientId`]; a client cannot
    ///   advance another client's cursor.
    fn confirm_apply(&mut self, client: &ClientId, epoch: Epoch);

    /// The low-water-mark: the **minimum** [`EpochCursor`] across **all tracked** clients.
    ///
    /// This — not any single client's cursor — is what licenses pruning. If there are no tracked
    /// clients the implementation defines the vacuous case downstream (e.g. the current epoch);
    /// the safety-relevant case is that any tracked-and-behind client holds the LWM down.
    fn low_water_mark(&self) -> Epoch;

    /// Whether `client` is currently tracked (known and not forgotten). Unknown clients are treated
    /// as forgotten for gating purposes (the "new clients start empty" claim is server-unverifiable).
    fn is_tracked(&self, client: &ClientId) -> bool;

    /// Forget a client whose cursor lags past [`MaxRetention`]. Forgetting is the explicit
    /// availability-vs-correctness sacrifice: a forgotten client returning CAN resurrect (a
    /// fleet-wide event) unless the pre-apply gate blocks its reconnect push.
    fn forget_client(&mut self, client: &ClientId);
}

/// Prune-safety predicate + the reconnect/merge ordering precondition.
///
/// The two methods encode, respectively, the PRUNE side (which epochs may be reclaimed) and the
/// RE-ADMISSION side (that a gate-time not-forgotten decision still holds at merge-commit). Both
/// are required for the module invariant to hold; a design that satisfies only one still admits
/// fleet-wide resurrection.
pub trait PruneSafety {
    /// Returns `true` **iff** `epoch` is prune-eligible.
    ///
    /// # Contract (HARD)
    /// Eligible **only** once the low-water-mark across **ALL tracked clients**
    /// ([`CausalFrontier::low_water_mark`]) has advanced past `epoch`. **NEVER** license pruning
    /// from a single client's cursor or the creating client's frontier — a single tracked client
    /// behind `epoch` MUST keep it un-prunable for the whole fleet. Reading [`CausalFrontier`]'s
    /// per-client advancement condition in isolation, as if one client's confirmation sufficed,
    /// would violate the module invariant.
    fn is_epoch_prune_eligible(&self, epoch: Epoch) -> bool;

    /// Returns `true` **iff** the not-forgotten decision certified by `token` at gate time still
    /// holds now, at merge-commit time.
    ///
    /// # Contract (gate-validity-to-merge-commit ordering, D2 (iv))
    /// The pre-apply gate and the remove-wins merge are separated by `.await` yield points, so a
    /// timer-driven prune sweep can interleave: a client passes the gate, crosses the forget
    /// threshold mid-handler, its covered epochs are pruned, and the resumed handler would re-admit
    /// a stale record against the post-prune set. The downstream implementation MUST guarantee that
    /// a client's not-forgotten status (and the presence of the tombstones its payload is checked
    /// against) established at gate time still holds when the merge commits — e.g. by serializing
    /// the prune sweep against in-flight gated pushes under a **per-key** single-writer (the same
    /// primitive that closes the SPEC-333b `OR_ADD` unlocked-RMW race — a joint fix). Per-key, not
    /// per-partition: serializing unrelated keys in a partition would collapse fleet write
    /// concurrency on a hot key. This MUST also preserve tombstone-set monotonicity (no pruned
    /// tombstone flickering back in during the window).
    fn gate_decision_holds_at_commit(&self, token: &GateToken) -> bool;
}
