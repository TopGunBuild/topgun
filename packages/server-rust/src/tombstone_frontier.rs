//! Prune-safety contract for bounding OR-Map tombstone growth (traits/types only).
//!
//! This module is the **design contract** produced by the tombstone-bound mechanism-selection
//! decision. It carries NO implementation body, NO write-path wiring, and NO prune logic —
//! those land in the downstream implementation children. It exists so the downstream implementer
//! fills in these signatures against a doc-contract that cannot be misread, and so the
//! tombstone-bytes gauge and soak-trust threshold work can depend on a fixed representation.
//!
//! It lives in `server-rust` (not `core-rust`) because the sole implementer is entirely
//! server-side — the single-node authority owns the epoch counter, the per-client frontier, and
//! the reconnect gate — and nothing in `core-rust` consumes the contract. The wire tombstone
//! shape (`Vec<String>`) is unchanged and stays in `core-rust`; only the server-side epoch
//! metadata contract lives here.
//!
//! # Selected mechanism
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
    /// when un-pruned tombstone cost approaches `TOPGUN_MAX_RAM_MB`. Ranking tracked clients by lag
    /// to pick which to forget is what [`CausalFrontierRanking::tracked_cursors`] supplies.
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
/// defines its representation; this contract only fixes that such a token flows gate → commit
/// exactly once.
///
/// Deliberately **not** `Clone` (and not `Copy`): [`PruneSafety::gate_decision_holds_at_commit`]
/// consumes it **by value**, so the type being non-`Clone` makes the by-value move a genuine
/// single-use / linear value — a `Clone` would let a caller clone-then-consume and submit two
/// merge-commits off one gate pass, making the by-value signature theater. If an audit/log path
/// needs the gate-time field after the move, read it off the token *before* consuming it (the
/// field is readable — see the accepted-forgeability note below).
///
/// Note: this single-use shape hardens the *stated intent*; it is NOT required for RE-ADMISSION
/// correctness. Re-admission safety comes from the commit-time RE-CHECK in
/// [`PruneSafety::gate_decision_holds_at_commit`], which does a LIVE `is_forgotten` re-read of the
/// client against the CURRENT epoch (a replayed/reconstructed token re-runs a fresh freshness
/// predicate against the current world = idempotent CRDT merge, not resurrection), not from token
/// unforgeability. The token carries only the client identity; the freshness signal is read live
/// at commit, never from a snapshotted gate-time epoch.
#[derive(Debug, PartialEq, Eq)]
pub struct GateToken {
    /// The client whose not-forgotten status the gate certified.
    ///
    /// Kept `pub` by an **accepted-forgeability decision**: a holder can reconstruct a `GateToken`
    /// from this field regardless of the dropped `Clone`. This is accepted because forgeability
    /// is **not** load-bearing — RE-ADMISSION safety is the commit-time RE-CHECK under the per-key
    /// single-writer (see [`PruneSafety::gate_decision_holds_at_commit`]), not token
    /// unforgeability. The public field also serves as the sanctioned post-consume read path (read
    /// the client off the token before the by-value move).
    pub client: ClientId,
}

/// Per-client causal frontier: the tracked cursors whose minimum licenses pruning.
///
/// Implemented downstream over the per-client high-water-marks. The doc-contract on each method is
/// the load-bearing part — an implementer MUST honour it.
///
/// # Tracking is implicit — no `track_client` / register method (and none is needed)
///
/// A client becomes **tracked** implicitly on its **first** [`confirm_apply`](Self::confirm_apply)
/// call; there is deliberately NO `track_client` / register method on this trait or any sub-trait,
/// and none is needed for safety. An unconfirmed (never-seen) client and a forgotten
/// (previously-tracked-then-dropped) client collapse into the SAME conservatively-gated
/// RE-ADMISSION path — both are untracked ("unknown == forgotten"), both are gated, and neither
/// pins the prune-side low-water-mark (an unconfirmed client pins nothing). This is safe by
/// construction: the gate never under-gates a client it does not recognise, so no register step is
/// required to close a resurrection hole. Downstream children MUST NOT widen this trait — or any
/// sub-trait — with a `track_client` / register method.
pub trait CausalFrontier {
    /// Record that `client` has **confirmed receive-AND-apply** up to `epoch`.
    ///
    /// # Contract
    /// - This is also the **implicit tracking trigger**: a client becomes tracked on its first
    ///   `confirm_apply`. There is no separate register step (see the trait-level note); a
    ///   never-confirmed client is untracked and gated ("unknown == forgotten"), and pins nothing
    ///   on the prune side.
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
    /// as forgotten for gating purposes ("unknown == forgotten"): a never-seen client and a
    /// forgotten client are indistinguishable and both take the conservatively-gated RE-ADMISSION
    /// path (the "new clients start empty" claim is server-unverifiable). A client transitions to
    /// tracked implicitly on its first [`confirm_apply`](Self::confirm_apply) — never via a
    /// register method, which does not exist and is not needed for safety.
    fn is_tracked(&self, client: &ClientId) -> bool;

    /// Forget a client whose cursor lags past [`MaxRetention`]. Forgetting is the explicit
    /// availability-vs-correctness sacrifice: a forgotten client returning CAN resurrect (a
    /// fleet-wide event) unless the pre-apply gate blocks its reconnect push.
    fn forget_client(&mut self, client: &ClientId);
}

/// Per-client cursor **ranking** for the RAM-pressure forget-driver — deliberately a SEPARATE
/// sub-trait, kept OFF the general [`CausalFrontier`], and restricted to `pub(crate)`.
///
/// # Why a sub-trait, and why `pub(crate)`
///
/// The RAM-pressure driver described on [`MaxRetention`] must **rank** tracked clients by lag to
/// pick which to forget, which needs per-client cursor visibility — but exposing that on the
/// general [`CausalFrontier`] would leak a **subset-MIN prune capability**: any holder of a bare
/// `&CausalFrontier` could compute a MIN over an arbitrary SUBSET of tracked clients (naturally
/// "all-tracked-except-the-one-I'm-forgetting"), yielding a value strictly greater than
/// [`CausalFrontier::low_water_mark`] — a per-subset prune the prune-side invariant forbids.
/// Confining per-client enumeration to this sub-trait means general prune/merge call sites holding
/// `&CausalFrontier` cannot even *express* that subset-MIN.
///
/// The `pub(crate)` visibility is defense-in-depth on top of the sub-trait split: it encodes "only
/// the forget-driver ranks" in Rust's visibility system rather than a doc claim, so no out-of-crate
/// holder can call the ranking accessor at all. This is orthogonal to correctness (ranking does not
/// prune; the invariant-relevant subset-MIN leak is already closed by keeping this off
/// `CausalFrontier`).
///
/// The point-lookup `client_cursor(&ClientId) -> Option<Epoch>` is deliberately NOT on this
/// surface — the ranking iterator is sufficient for lag-ranking, and a point-lookup is the shape
/// that most directly invites a "prune against this one client" misuse.
// No implementer exists yet — this is the trait-first Wave-1 contract surface; the RAM-pressure
// forget-driver that ranks via `tracked_cursors` lands in a downstream child.
#[allow(dead_code)]
pub(crate) trait CausalFrontierRanking: CausalFrontier {
    /// Snapshot of every tracked client's current cursor, for lag-ranking by the forget-driver.
    ///
    /// Returned by value (`Vec`, not `impl Iterator`) to keep [`CausalFrontierRanking`]
    /// object-safe, so the downstream forget-driver may dispatch over `&dyn CausalFrontierRanking`
    /// if it chooses. Callers MUST use this only to rank/select clients to forget — it does NOT
    /// license pruning; only [`PruneSafety::is_epoch_prune_eligible`] does.
    fn tracked_cursors(&self) -> Vec<(ClientId, Epoch)>;
}

/// Prune-safety predicate + the reconnect/merge ordering precondition.
///
/// The two methods encode, respectively, the PRUNE side (which epochs may be reclaimed) and the
/// RE-ADMISSION side (that a gate-time not-forgotten decision still holds at merge-commit). Both
/// are required for the module invariant to hold; a design that satisfies only one still admits
/// fleet-wide resurrection.
///
/// Declared as a **supertrait of [`CausalFrontier`]** so `is_epoch_prune_eligible` can read
/// [`CausalFrontier::low_water_mark`] and a struct cannot impl `PruneSafety` with no frontier to
/// read from. The supertrait only **exposes** the low-water-mark; in a types-only surface it
/// cannot *type-enforce* that `is_epoch_prune_eligible` actually folds over the fleet-wide MIN
/// (Rust cannot bind one method body to call a sibling without a default body or a sealed trait) —
/// so the fleet-wide-MIN property stays **doc-enforced and reviewer-checkable**, not type-enforced.
pub trait PruneSafety: CausalFrontier {
    /// Returns `true` **iff** `epoch` is prune-eligible.
    ///
    /// # Contract (HARD — reviewer-checkable, NOT type-enforced)
    /// An implementation MUST compute eligibility as a fold over
    /// [`CausalFrontier::low_water_mark`] — i.e. eligible **only** once the low-water-mark across
    /// **ALL tracked (non-forgotten) clients** has advanced past `epoch`. **NEVER** license pruning
    /// from a single client's cursor, a subset of clients, or the creating client's frontier — a
    /// single tracked client behind `epoch` MUST keep it un-prunable for the whole fleet. The
    /// [`CausalFrontier`] supertrait only *exposes* `low_water_mark`; it does not, and in a
    /// types-only surface cannot, *enforce* the fleet-wide fold — a reviewer MUST mechanically
    /// confirm the impl reads the fleet-wide MIN and never a subset.
    fn is_epoch_prune_eligible(&self, epoch: Epoch) -> bool;

    /// Returns `true` **iff** the not-forgotten decision certified by `token` at gate time still
    /// holds now, at merge-commit time. Consumes `token` **by value** to enforce single-use at the
    /// call site (one gate pass → one merge-commit).
    ///
    /// # Contract (gate-validity-to-merge-commit ordering)
    /// The pre-apply gate and the remove-wins merge are separated by `.await` yield points, so a
    /// timer-driven prune sweep can interleave: a client passes the gate, crosses the forget
    /// threshold mid-handler, its covered epochs are pruned, and the resumed handler would re-admit
    /// a stale record against the post-prune set. The downstream implementation MUST guarantee that
    /// a client's not-forgotten status (and the presence of the tombstones its payload is checked
    /// against) established at gate time still holds when the merge commits — e.g. by serializing
    /// the prune sweep against in-flight gated pushes under a **per-key** single-writer (the same
    /// primitive that closes the `OR_ADD` unlocked-RMW race — a joint fix). Per-key, not
    /// per-partition: serializing unrelated keys in a partition would collapse fleet write
    /// concurrency on a hot key. This MUST also preserve tombstone-set monotonicity (no pruned
    /// tombstone flickering back in during the window).
    ///
    /// # Safety comes from the commit-time RE-CHECK, not token unforgeability
    /// This method is a predicate that RE-CHECKS current state (LWM, tracked-set, tombstone
    /// presence) at commit time. A replayed or reconstructed token re-runs a FRESH freshness
    /// predicate against the current world, so two commits off one gate pass are idempotent CRDT
    /// merge, NOT resurrection. Re-admission safety therefore does **not** depend on the token being
    /// unforgeable or single-use — the downstream implementer MUST implement the commit-time
    /// re-check and MUST NOT over-rely on token unforgeability. The by-value / non-`Clone` token is
    /// defense-in-depth on the *stated single-use intent*, orthogonal to this correctness argument.
    fn gate_decision_holds_at_commit(&self, token: GateToken) -> bool;
}
