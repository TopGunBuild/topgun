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

// ---------------------------------------------------------------------------
// Prune-record observation contract
// ---------------------------------------------------------------------------
//
// Types and traits only, consistent with this module's charter. The implementing recorder, the
// arming read and the observation call sites live in the sibling implementation module; the
// prune-loop ledger lives in the CRDT domain service.

/// How a single tombstone ref left the prune loop body.
///
/// The set is **CLOSED**: every ref the loop considers leaves through exactly one of these, and the
/// identity `considered == dropped + matched_nothing + absent + restored_read_error +
/// restored_evicted + restored_write_error` MUST hold. Exhaustiveness is load-bearing rather than
/// tidiness: [`PruneExit::AbsentKey`] consumes a ref **without** a tombstone-byte decrement, so a
/// growing `AbsentKey` share is a candidate mechanism for a falling reclaim fraction that no other
/// instrument in the tree can see. This contract takes no position on whether that happens; it
/// requires that the record be able to say.
///
/// Modelled as an enum rather than a string because the exit set is known and closed, and because
/// the summing identity above is only mechanically checkable against a fixed set of counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PruneExit {
    /// The prune closure pruned at least one tag AND the durable write succeeded.
    Dropped,
    /// The closure ran but matched no tag. It may still owe a shape-upgrade write; no tag left.
    MatchedNothing,
    /// The backing read returned `Ok(None)` — the key is gone. The ref is **consumed** and **no
    /// tombstone-byte decrement occurs**, which is why this exit must be counted separately from
    /// [`PruneExit::Dropped`] instead of being folded into a generic "not dropped" bucket.
    AbsentKey,
    /// The backing read errored; the ref is re-indexed for a later pass.
    RestoredReadError,
    /// The closure never ran because the key was evicted mid-write; the ref is re-indexed.
    RestoredEvicted,
    /// The in-place update errored; the ref is re-indexed.
    RestoredWriteError,
}

/// One prune pass — one whole invocation of the epoch-tombstone prune loop, empty drains included.
///
/// # Field widths
/// Every field is a count or a byte total and is therefore an unsigned integer, never a float.
/// `u64` is used uniformly because the metrics counter API takes `u64`, so the emit boundary needs
/// no cast; only the gauge/histogram boundary converts, and it does so at the emit call site.
///
/// `empty_drain` is a boolean predicate about the drain, not a count, which is why it is the one
/// non-integral field: the pass identity `passes == empty_drains + nonempty_drains` is derived from
/// it, and folding it into a count would make a single pass increment two different counters.
///
/// # Where the pass increment is sited
/// A pass is counted on **every** invocation, before any eligibility test and **outside** the loop
/// body — never inside the per-ref loop and never behind a `dropped` / `ran` condition. An
/// increment sited inside the loop body would read zero during a legitimate total stall, which is
/// precisely the regime the record has to be able to describe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PrunePassRecord {
    /// Refs the pass considered — the exhaustiveness identity's left-hand side and the denominator
    /// of every exit share.
    pub considered: u64,
    /// Refs that left through [`PruneExit::Dropped`].
    pub dropped: u64,
    /// Refs that left through [`PruneExit::MatchedNothing`].
    pub matched_nothing: u64,
    /// Refs that left through [`PruneExit::AbsentKey`].
    pub absent: u64,
    /// Refs that left through [`PruneExit::RestoredReadError`].
    pub restored_read_error: u64,
    /// Refs that left through [`PruneExit::RestoredEvicted`].
    pub restored_evicted: u64,
    /// Refs that left through [`PruneExit::RestoredWriteError`].
    pub restored_write_error: u64,
    /// Tombstone bytes the pass freed.
    pub bytes_freed: u64,
    /// Epochs the pass drained.
    pub epochs_drained: u64,
    /// Whether the drain came back empty. Empty and non-empty drains are counted separately so
    /// neither pollutes the other's distribution: a per-drain mean taken over a denominator that
    /// includes empty drains measures scheduling frequency, not batch size.
    pub empty_drain: bool,
}

/// One drained epoch inside a pass.
///
/// # The join is deliberately out of scope
/// The `(current_epoch, low_water_mark, durable_epoch_watermark)` triple is carried here so the
/// caller snapshots it **atomically with the drain** rather than re-reading it later. The joined
/// per-epoch record — epoch id tied to its own counts tied to the triple in force when *that* epoch
/// drained — is **not representable** over the Prometheus transport: a histogram carries only
/// `_sum` / `_count` and rendered quantiles and loses the per-observation association, and a gauge
/// is last-value at scrape granularity, so two epochs draining inside one scrape interval are
/// indistinguishable. That is a property of the transport, not an omission, and no consumer of this
/// contract needs the join — every decision term downstream is an aggregate. An implementation
/// emits the counts as distributions and the triple through the last-value epoch gauges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PruneEpochRecord {
    /// The drained epoch's id.
    pub epoch: Epoch,
    /// Refs this epoch contributed to the pass's `considered`.
    pub considered: u64,
    /// Refs of this epoch that left through [`PruneExit::Dropped`].
    pub dropped: u64,
    /// Tombstone bytes this epoch freed.
    pub bytes_freed: u64,
    /// `current_epoch` in force when this epoch drained.
    pub current_epoch: Epoch,
    /// [`CausalFrontier::low_water_mark`] in force when this epoch drained.
    pub low_water_mark: Epoch,
    /// The durable epoch watermark in force when this epoch drained.
    pub durable_epoch_watermark: Epoch,
}

/// The claim span observed at an instant that moves the frontier — an LWM movement or a non-empty
/// drain.
///
/// The span is the empirical evidence base for a retention ceiling expressed as
/// `min_live_claim − fixed_margin`: without a measured distribution of how far the fleet's slowest
/// tracked claim lags the current epoch, such a ceiling can only be guessed.
///
/// `span_epochs` is carried explicitly rather than left to be recomputed from the two epochs
/// because the subtraction is saturating at the observation point — a caller that recomputes it
/// later can observe a different, non-atomic pair.
///
/// Per-tracked-claim lags are NOT a field: they are a variable-length sequence, and holding them in
/// the record would force an allocation on a path whose whole point is to stay cheap. They are
/// passed as a borrowed slice to [`PruneRecordObserver::observe_claim_span`] instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PruneClaimSpanRecord {
    /// `current_epoch` at the observation instant.
    pub current_epoch: Epoch,
    /// [`CausalFrontier::low_water_mark`] at the observation instant.
    pub low_water_mark: Epoch,
    /// `current_epoch − low_water_mark`, saturating, computed at the observation instant.
    pub span_epochs: u64,
    /// How many claims were tracked at the observation instant — the denominator that makes the
    /// per-claim lag distribution readable.
    pub tracked_claims: u64,
}

/// Whether the prune record is recording.
///
/// Read **once**, at frontier construction, from `TOPGUN_PRUNE_RECORD`, and never per pass — so the
/// arming gate and the recording branch cannot observe different answers for the same operation.
/// The parse follows the established kill-switch discipline in this tree: the default is
/// [`PruneRecordArming::Armed`], only an explicit falsey word (`false` / `0` / `no` / `off`,
/// case-insensitive) disarms, and an unrecognised value stays ARMED rather than quietly turning the
/// instrument off on a typo.
///
/// The effective arming is provable mechanically from a `/metrics` scrape — armed implies the
/// prune-record series are present, disarmed implies they are absent — which is why no boot-summary
/// field carries it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PruneRecordArming {
    /// Recording. The observer resolves and touches every pinned series at construction.
    Armed,
    /// Not recording. The observer is a null implementation that performs no allocation, no atomic
    /// write and no metrics call.
    Disarmed,
}

/// Sink for the prune record.
///
/// The prune loop and the frontier code against this trait, never against a concrete recorder, so
/// the disarmed path is a null implementation rather than a branch at every call site.
///
/// # Gauge neutrality (HARD — the instrument MUST NOT move what it measures)
///
/// This contract exists to observe reclamation, not to participate in it. `TG-OR-004` — the
/// tombstone-bytes accounting invariant — MUST NOT be perturbed by anything an implementation of
/// this trait does. Concretely, and mechanically checkable:
///
/// - An implementation's body names **no** tombstone-byte counter or gauge at all. Adjusting the
///   tombstone-byte accounting is the prune path's job; an observer that also adjusts it would
///   double-count, and a red tombstone gate would then be evidence about the instrument rather than
///   about the system.
/// - An armed run and a disarmed run over the same workload MUST produce **identical** tombstone
///   byte-gauge deltas and **identical** dropped counts. Observation is the only difference between
///   them.
/// - The tombstone-byte decrement stays exactly where the prune loop already sites it — in the
///   post-write success arm, behind `dropped` — and adding an exit ledger MUST NOT move it, because
///   a decrement that follows the ledger instead of the write would credit bytes that were never
///   freed.
///
/// # Perturbation budget
///
/// - On the hot path (every remove-apply, every sync leaf): at most a fixed, small number of atomic
///   increments. No allocation, no formatting, no additional lock acquisition, no index fold.
/// - Work proportional to the indexed epoch count is permitted **only** on the LWM-movement path
///   and on non-empty drains.
/// - Metric handles are resolved **once**, at recorder construction, and stored. Per-call-site
///   `counter!` / `gauge!` / `histogram!` macro invocations are forbidden because each performs a
///   registry lookup by name.
///
/// # Construction-order precondition (a property this contract DEPENDS ON, not one it establishes)
///
/// A metric handle resolved **before** the Prometheus recorder is installed binds to a no-op for
/// that handle's whole lifetime — it never re-resolves. Caching handles at construction is
/// therefore only sound if the recorder is installed first. Every implementation MUST be
/// constructed after observability initialisation, and any test that constructs one MUST install a
/// recorder first, so a test-order inversion cannot mask the hazard.
///
/// # Registration is eager, so an absent series is unrepresentable
///
/// Resolving the handle is what registers a series with this exporter; a `describe_*` call alone
/// registers nothing. An implementation therefore resolves a handle for **every** name pinned below
/// at construction and touches each one, so the very first scrape already renders all of them —
/// counters and gauges at zero, histograms with a zero `_sum` and `_count`. A series that is
/// unobserved must read as a zero, never as an absence, because a downstream sampler cannot
/// distinguish an absent series from a stalled one.
///
/// # Emitting
///
/// Implementations emit **only** through the name constants declared below. A string literal at an
/// emit call site is a defect: binding the emitter to the constant is what makes it impossible for
/// an emitted name to drift from the pinned one, and it makes the compiler rather than a document
/// review the thing that enforces it.
pub trait PruneRecordObserver: Send + Sync {
    /// One completed prune pass.
    ///
    /// Feeds the pass counter, the six exit counters, `considered`, `bytes_freed`,
    /// `epochs_drained`, exactly one of the empty / non-empty drain counters, and — on a non-empty
    /// drain only — the refs-per-drain and epochs-per-drain distributions.
    fn observe_pass(&self, record: &PrunePassRecord);

    /// One epoch drained inside a pass. Feeds the three per-drained-epoch distributions.
    fn observe_drained_epoch(&self, record: &PruneEpochRecord);

    /// The claim span at an LWM movement or a non-empty drain.
    ///
    /// Feeds the claim-span distribution, the tracked-claim gauge, and the per-claim lag
    /// distribution — one observation per element of `claim_lags`. The lags are borrowed rather
    /// than owned so this call allocates nothing.
    fn observe_claim_span(&self, record: &PruneClaimSpanRecord, claim_lags: &[Epoch]);

    /// The low-water-mark advanced by `epochs_advanced` epochs.
    ///
    /// Feeds both LWM-advance counters: one advance, and the epoch distance it covered. Called only
    /// on an actual advance, so the advance count stays a count of movements rather than of
    /// confirmations.
    fn observe_lwm_advance(&self, epochs_advanced: u64);

    /// Time elapsed since the last low-water-mark advance.
    ///
    /// Separate from [`PruneRecordObserver::observe_lwm_advance`] on purpose: the interesting
    /// regime is the one in which the LWM is **not** advancing, and a stall that is only refreshed
    /// on an advance would freeze at its last value exactly when it matters.
    fn observe_lwm_stall(&self, stall_seconds: u64);

    /// The O(1)-maintained size of the tombstone index.
    ///
    /// Both values are maintained incrementally at stamp / drain / restore; an implementation MUST
    /// NOT fold the index to produce them.
    fn observe_index_state(&self, indexed_refs: u64, indexed_epochs: u64);

    /// The frontier's epoch state, as last-value gauges.
    fn observe_epoch_state(
        &self,
        current_epoch: Epoch,
        low_water_mark: Epoch,
        durable_epoch_watermark: Epoch,
        last_drained_epoch: Epoch,
    );

    /// The eligible / ineligible split of the indexed corpus, with its staleness marker.
    ///
    /// The split is computed **only** on the LWM-movement path and on non-empty drains — never per
    /// remove — which is exactly why it needs a staleness marker: those triggers are the events that
    /// stop happening during a scheduling or LWM stall, so a split frozen at its last recompute
    /// reads as a fresh "not growing" precisely when it is least entitled to. `computed_at_epoch`
    /// and the monotone recompute counter this call increments are what let a reader tell a fresh
    /// sample from a frozen one; a reader that sees no recompute across its window MUST treat the
    /// split as inadmissible rather than as a measurement.
    fn observe_eligibility_split(
        &self,
        eligible_refs: u64,
        ineligible_refs: u64,
        computed_at_epoch: Epoch,
    );
}

// ---------------------------------------------------------------------------
// Pinned metric names — 15 counters, 11 gauges, 7 histograms
// ---------------------------------------------------------------------------
//
// The name set is CLOSED. Emitting a series under this prefix that is not named here, or emitting
// one of these under a literal instead of its constant, is a defect: the set is frozen in a
// pre-registered evidence manifest that cannot be edited afterwards, so a divergence between code
// and manifest is only ever repairable in the code. Adding a name is not an executor's decision.
//
// Naming follows the shape already in the tree (`topgun_ormap_tombstone_bytes_total`): monotone
// counters end `_total`, gauges carry no suffix, histograms carry no suffix and export `_sum` /
// `_count` totals alongside the exporter's rendered quantiles. The `_sum` / `_count` pair is the
// primary reading — the installed exporter renders histograms as SLIDING-WINDOW summaries, so a
// rendered quantile is a window statistic, not an over-the-run distribution, and reading one as if
// it were is the defect these totals exist to prevent.

/// Counter: prune passes run, including empty drains.
pub const METRIC_PRUNE_PASSES_TOTAL: &str = "topgun_or_prune_passes_total";
/// Counter: tombstone refs considered — the exhaustiveness identity's left-hand side.
pub const METRIC_PRUNE_CONSIDERED_TOTAL: &str = "topgun_or_prune_considered_total";
/// Counter: refs that left through [`PruneExit::Dropped`].
pub const METRIC_PRUNE_DROPPED_TOTAL: &str = "topgun_or_prune_dropped_total";
/// Counter: refs that left through [`PruneExit::MatchedNothing`].
pub const METRIC_PRUNE_MATCHED_NOTHING_TOTAL: &str = "topgun_or_prune_matched_nothing_total";
/// Counter: refs that left through [`PruneExit::AbsentKey`] — consumed with no byte decrement.
pub const METRIC_PRUNE_ABSENT_TOTAL: &str = "topgun_or_prune_absent_total";
/// Counter: refs that left through [`PruneExit::RestoredReadError`].
pub const METRIC_PRUNE_RESTORED_READ_ERROR_TOTAL: &str =
    "topgun_or_prune_restored_read_error_total";
/// Counter: refs that left through [`PruneExit::RestoredEvicted`].
pub const METRIC_PRUNE_RESTORED_EVICTED_TOTAL: &str = "topgun_or_prune_restored_evicted_total";
/// Counter: refs that left through [`PruneExit::RestoredWriteError`].
pub const METRIC_PRUNE_RESTORED_WRITE_ERROR_TOTAL: &str =
    "topgun_or_prune_restored_write_error_total";
/// Counter: tombstone bytes freed by the prune.
pub const METRIC_PRUNE_BYTES_FREED_TOTAL: &str = "topgun_or_prune_bytes_freed_total";
/// Counter: epochs drained.
pub const METRIC_PRUNE_EPOCHS_DRAINED_TOTAL: &str = "topgun_or_prune_epochs_drained_total";
/// Counter: empty drains, counted separately from non-empty ones.
pub const METRIC_PRUNE_EMPTY_DRAINS_TOTAL: &str = "topgun_or_prune_empty_drains_total";
/// Counter: non-empty drains — the denominator for every per-drain mean.
pub const METRIC_PRUNE_NONEMPTY_DRAINS_TOTAL: &str = "topgun_or_prune_nonempty_drains_total";
/// Counter: low-water-mark advances.
pub const METRIC_PRUNE_LWM_ADVANCES_TOTAL: &str = "topgun_or_prune_lwm_advances_total";
/// Counter: total epochs the low-water-mark advanced.
pub const METRIC_PRUNE_LWM_EPOCHS_ADVANCED_TOTAL: &str =
    "topgun_or_prune_lwm_epochs_advanced_total";
/// Counter: eligible / ineligible split recomputes — the split's staleness marker.
pub const METRIC_PRUNE_SPLIT_RECOMPUTES_TOTAL: &str = "topgun_or_prune_split_recomputes_total";

/// Gauge: indexed tombstone refs, maintained incrementally.
pub const METRIC_PRUNE_INDEXED_REFS: &str = "topgun_or_prune_indexed_refs";
/// Gauge: indexed epochs, maintained incrementally.
pub const METRIC_PRUNE_INDEXED_EPOCHS: &str = "topgun_or_prune_indexed_epochs";
/// Gauge: the licensed backlog — indexed refs in prune-eligible epochs.
pub const METRIC_PRUNE_ELIGIBLE_REFS: &str = "topgun_or_prune_eligible_refs";
/// Gauge: the pinned pool — indexed refs no tracked claim licenses reclaiming yet.
pub const METRIC_PRUNE_INELIGIBLE_REFS: &str = "topgun_or_prune_ineligible_refs";
/// Gauge: the `current_epoch` at which the eligible / ineligible split was last computed.
pub const METRIC_PRUNE_SPLIT_COMPUTED_EPOCH: &str = "topgun_or_prune_split_computed_epoch";
/// Gauge: the current server epoch.
pub const METRIC_PRUNE_CURRENT_EPOCH: &str = "topgun_or_prune_current_epoch";
/// Gauge: the fleet-wide low-water-mark.
pub const METRIC_PRUNE_LOW_WATER_MARK: &str = "topgun_or_prune_low_water_mark";
/// Gauge: the durable epoch watermark.
pub const METRIC_PRUNE_DURABLE_EPOCH_WATERMARK: &str = "topgun_or_prune_durable_epoch_watermark";
/// Gauge: the id of the most recently drained epoch.
pub const METRIC_PRUNE_LAST_DRAINED_EPOCH: &str = "topgun_or_prune_last_drained_epoch";
/// Gauge: seconds since the last low-water-mark advance.
pub const METRIC_PRUNE_LWM_STALL_SECONDS: &str = "topgun_or_prune_lwm_stall_seconds";
/// Gauge: tracked-claim count.
pub const METRIC_PRUNE_TRACKED_CLAIMS: &str = "topgun_or_prune_tracked_claims";

/// Histogram: refs per non-empty drain.
pub const METRIC_PRUNE_DRAIN_REFS: &str = "topgun_or_prune_drain_refs";
/// Histogram: epochs per non-empty drain.
pub const METRIC_PRUNE_DRAIN_EPOCHS: &str = "topgun_or_prune_drain_epochs";
/// Histogram: `current_epoch − low_water_mark` at each LWM movement and each non-empty drain.
pub const METRIC_PRUNE_CLAIM_SPAN_EPOCHS: &str = "topgun_or_prune_claim_span_epochs";
/// Histogram: per-tracked-claim lag at those same instants.
pub const METRIC_PRUNE_CLAIM_LAG_EPOCHS: &str = "topgun_or_prune_claim_lag_epochs";
/// Histogram: refs considered, per drained epoch.
pub const METRIC_PRUNE_EPOCH_CONSIDERED: &str = "topgun_or_prune_epoch_considered";
/// Histogram: refs dropped, per drained epoch.
pub const METRIC_PRUNE_EPOCH_DROPPED: &str = "topgun_or_prune_epoch_dropped";
/// Histogram: tombstone bytes freed, per drained epoch.
pub const METRIC_PRUNE_EPOCH_BYTES_FREED: &str = "topgun_or_prune_epoch_bytes_freed";
