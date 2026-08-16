//! Reclamation registry — the single authority over *what may be reclaimed*.
//!
//! A reclamation boundary is a promise made to every party that still needs the content below it.
//! Historically each consumer folded its own boundary out of whatever state it happened to hold,
//! which is how two consumers end up disagreeing about what is gone. This module replaces that
//! with a registration-and-min-reduction protocol: parties **register claims**, the registry
//! reduces them, and a consumer that wants to reclaim **consumes** the registry's boundary rather
//! than computing one of its own.
//!
//! # The two boundary values, and why conflating them is a bug
//!
//! There are **two** values here and they move differently:
//!
//! 1. The **ceiling proposal** ([`ReclamationBoundary::prune_ceiling`]) — recomputed from scratch
//!    on every query as the fleet MIN over live claims, less a fixed margin. It **MAY FALL**,
//!    because a reconnecting laggard rejoining the fold lowers the MIN, and that fall is how the
//!    laggard regains protection. It is not a ratchet and there is no first-claimant latch.
//! 2. The **executed watermark** ([`ReclamationBoundary::executed_watermark`]) — monotone,
//!    advanced by exactly one act (the completion of a sweep) and only with `max`, never assigned.
//!    It records what the registry believes has already been reclaimed, and it is the **only**
//!    boundary a claim is measured against for refusal.
//!
//! Monotonicity lives on the watermark, never on the proposal. A test or caller that asserts the
//! proposal never falls has asserted that the first claimant sets the fleet floor, which is the
//! defect this split exists to prevent.
//!
//! # Sweeps are token-shaped
//!
//! [`ReclamationBoundary::begin_sweep`] returns an [`Option<SweepToken>`]; a refused begin yields
//! nothing to end. That makes *"end a sweep you did not begin"* a **compile error** rather than a
//! runtime rule, and it is why the ceiling snapshot travels inside the token instead of sitting in
//! a single registry slot a second sweep could overwrite.
//!
//! # Scope
//!
//! Exactly one scope is admitted — see [`ClaimScope`], whose doc-contract states the limit and the
//! operational fence a future second scope must honour.

use crate::tombstone_frontier::Epoch;

/// The identity of a party holding a reclamation claim.
///
/// This is the **server-authenticated identity** — the same shape and the same provenance as
/// [`crate::tombstone_frontier::ClientId`]. Keying claims off a client-asserted identity would let
/// a party forge or squat another party's claim and move the fleet boundary, so the authenticated
/// identity is the only admissible key.
pub type ClaimantId = String;

/// The scope a claim is registered under.
///
/// # Frozen limit (HARD)
///
/// **Multi-scope admission is NOT solved by this surface.** The enum admits exactly one variant,
/// so every quantifier in this module's contract collapses to [`ClaimScope::Global`]: there is no
/// cross-scope union, no cross-scope admission rule, and no scope-local/global mismatch. The enum
/// itself survives as the forward seam — every wired call passes `Global`, so the seam is
/// witnessed — but a second inhabitant may not be added without the cross-scope admission rule
/// that goes with it (a claim must then check the MAX executed watermark over every scope whose
/// min it joins).
///
/// # Operational fence for a future per-partition tracker (HARD)
///
/// *The `Global` registry does not consult any per-partition tracker. A future per-partition
/// tracker MUST fence the `Global` sweep — or the `Global` sweep may reclaim content a
/// per-partition claim needs.* This sentence is part of the contract, not commentary: without it
/// the deferred hazard does not defer, it **migrates**, reappearing as a silent cross-scope hole
/// the moment a second scope is added.
///
/// Per-partition claims, the partition variant and its identifier type, the cross-scope admission
/// rule, the lease/proposal model for long-running or distributed sweeps (with queued admissions),
/// and instance-binding of [`SweepToken`] are all deferred and owned by TODO-634. They become live
/// when a consumer exists to witness them; freezing them witness-less is the defect this deferral
/// avoids.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ClaimScope {
    /// The whole fleet: one claim set, one MIN, one boundary.
    Global,
}

/// The verdict returned by [`ReclamationBoundary::register_claim`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimAdmission {
    /// The claim was **recorded**. The stored position is returned, which may be higher than the
    /// requested one because a claimant's own claim is monotone (a claimant never walks its own
    /// claim backwards).
    ///
    /// Recording is unconditional for every valid claim at or above the executed watermark —
    /// **regardless of where the claim sits relative to the current ceiling proposal or to any
    /// other claimant's position.** A claim below the proposal but above the watermark is the
    /// ordinary case of a laggard rejoining the fold: it is recorded, and it lowers the proposal.
    Honoured {
        /// The position now recorded for this claimant.
        claim: Epoch,
    },

    /// The claim was **refused** and **nothing was recorded**: it lies below content the registry
    /// has already recorded as reclaimed.
    ///
    /// # This is the designed resync-fence signal (HARD)
    ///
    /// It fires **only** when the claim is strictly below the executed watermark (or when the
    /// claim is `0`, which is not a valid position). Being *behind another claimant* is **NEVER**
    /// a refusal reason — no claim is ever refused, dropped or displaced for being behind, for
    /// arrival order, or for the ceiling proposal's current value.
    ///
    /// The caller **MUST NOT** treat the claimant as protected. The content it needs is, in the
    /// general case, physically gone, so no boundary movement can serve it and a resync is the
    /// only honest answer. The cursor-age fence and the forced resync that turn this signal into a
    /// protocol action are deferred and owned by TODO-634; a consumer today reports the condition
    /// and leaves the claimant on its existing conservative re-admission path.
    ///
    /// # `executed` is an upper bound, not a certificate
    ///
    /// A drain is not atomic: a reference whose storage drop fails is handed back for
    /// re-insertion, so an epoch below the watermark may be back in the index. The refusal is
    /// therefore an **over**-refusal in that case. The direction is deliberately the safe one —
    /// the watermark gates admission only and never licenses reclamation — so the cost of the gap
    /// is an unnecessary resync, never a resurrection.
    BelowExecuted {
        /// The refused position.
        claim: Epoch,
        /// The executed watermark the claim was measured against.
        executed: Epoch,
    },
}

/// Proof that the holder, and only the holder, has an in-flight sweep.
///
/// Issued **only** by [`ReclamationBoundary::begin_sweep`] and consumed **only** by
/// [`ReclamationBoundary::end_sweep`], both by value. It carries the ceiling snapshot taken at
/// sweep start, which is private to this module and cannot be read, forged or edited by a
/// consumer.
///
/// # Why this is a type and not a rule (HARD)
///
/// A refused `begin_sweep` yields nothing to end, so *"end a sweep you did not begin"* — clearing
/// another sweep's in-progress flag, or advancing the executed watermark from a snapshot you do
/// not own — **cannot be compiled**, let alone executed. And because the snapshot travels *in* the
/// token rather than in registry state, there is no single-slot snapshot field for a second sweep
/// to overwrite. The type is not [`Clone`], not [`Copy`] and is `#[must_use]`, so it cannot be
/// forged, duplicated or silently discarded. Type-level safety is the only kind that survives
/// being frozen for consumers this module cannot see.
///
/// # Two bounded facts a consumer needs
///
/// - **No [`Drop`] safety-net.** A `Drop` impl that cleared the in-progress flag would have to
///   borrow the registry (making the return type lifetime-bound in every consumer's signature),
///   and clearing the flag *without* advancing the watermark would silently convert a missed
///   `end_sweep` into a sweep whose reclamation is never fenced. A leaked token instead leaves the
///   sweep flagged in progress, which is **loud**: the in-progress gauge stays pinned at `1` on
///   `/metrics` while the completed-sweep counter stays flat, and every later `begin_sweep` is
///   refused.
/// - **No instance binding.** The token is not bound to the registry instance that issued it, so
///   with two registries a token could be handed to the wrong one. Exactly one registry exists
///   today; instance-binding is deferred and owned by TODO-634 alongside the shared-registry work.
#[must_use = "hand the token back to end_sweep, or the sweep stays flagged in progress \
              and every later begin_sweep is refused"]
#[derive(Debug)]
pub struct SweepToken {
    // Module-private on purpose: the snapshot is the sweep's licence, and a consumer that could
    // read or rebuild it could fence the watermark from a boundary it did not obtain under the
    // lock. No implementer exists yet — this is the trait-first contract surface, and the
    // registry's `end_sweep` (the field's only reader) lands with the implementation.
    #[allow(dead_code)]
    ceiling_snapshot: Epoch,
}

/// The reclamation boundary protocol: register claims, reduce them, bracket sweeps.
///
/// # Never a head, never a caller-supplied boundary (HARD)
///
/// An implementation takes a `boot_floor` at construction and **no method accepts a current epoch,
/// a head or a watermark as a BOUNDARY, in any form.** The registry derives the executed watermark
/// itself, from two inputs it can defend: its **own** sweep-start ceiling snapshot, taken from live
/// claims under the lock, and the durable watermark the consumer reports at sweep end as an
/// **observed fact**. That watermark is a **clamp input only** — it appears inside a `min(…)` and
/// can therefore only LOWER the derived value, never raise it — so a head cannot be smuggled in
/// through it. A recovery high-water such as a rebuild's recovered epoch **MUST NOT** be used as
/// the floor: that would license reclamation up to nearly head after recovery, which is exactly the
/// hazard this clause forbids.
///
/// With an empty claim set the ceiling proposal equals the boot floor, and on a registry that has
/// never swept the executed watermark equals the boot floor.
///
/// # Restart and durability semantics (HARD)
///
/// 1. **The registry is IN-MEMORY ONLY.** Claims and the executed watermark are constructed fresh
///    at every registry construction and survive no restart. With a boot floor of `0` this is
///    benign and reproduces the pre-registry behaviour exactly: a restarted registry starts with no
///    fence and licenses nothing until a claim is registered and a sweep completes.
/// 2. **`boot_floor` MUST be non-decreasing across restarts.** A floor derived from state that can
///    regress lets a post-restart sweep reclaim below a pre-restart watermark, and the in-memory
///    `max`-advance cannot defend against it because the watermark reset too. `0` is trivially
///    non-decreasing; the first consumer that passes anything else inherits this obligation.
/// 3. **A restart drops the whole fold, and the resync fence is the fallback, not the
///    prevention.** A claimant protected before a restart is unprotected after it and MUST
///    re-register before it needs the content. This is a conscious decision, not an oversight.
/// 4. **A consumer that PERSISTS the executed watermark MUST persist it strictly AFTER the
///    reclamation it records is durable, never before.** The reverse ordering leaves the watermark
///    BEHIND reality after a crash, which admits a claimant into a range whose content is
///    physically gone — the dangerous direction. The watermark ahead of reality is the safe
///    direction.
///
/// Items 2–4 constrain consumers that do not exist yet; delivering them is owned by TODO-634, and
/// they are stated here because the surface they constrain is frozen here.
///
/// # How a claim leaves the fold
///
/// A recorded live claim may be removed from the min-reduction by exactly two acts: an explicit
/// [`ReclamationBoundary::release_claim`], and — when it lands — a cursor-age retention fence with
/// its horizon quarantine, which is out of scope here and owned by TODO-634. **No claim is ever
/// displaced by arrival order, by another claimant's position, or by the ceiling proposal.** The
/// honest consequence of the retention fence being deferred: an abandoned laggard that is never
/// released pins the boundary indefinitely. That is the pre-existing behaviour of the fleet MIN
/// this boundary replaces, with the same explicit-release escape, so nothing new is exposed.
pub trait ReclamationBoundary: Send + Sync {
    /// Register (or raise) `claimant`'s claim at `claim` under `scope`.
    ///
    /// # Caller obligation: admissions and sweeps are MUTUALLY EXCLUSIVE (HARD)
    ///
    /// A consumer **MUST NOT** call this concurrently with a sweep it has begun. Every
    /// registration must either complete **before** [`Self::begin_sweep`] takes its ceiling
    /// snapshot — in which case the claim is IN that snapshot — or begin **after**
    /// [`Self::end_sweep`] has advanced the watermark — in which case it is measured against the
    /// NEW watermark. There is no third, in-between outcome, and therefore nothing to queue: the
    /// registry maintains **no pending-admission queue**, and a consumer that cannot serialise its
    /// own admissions against its own sweep needs the queued-admission model deferred to TODO-634.
    ///
    /// The tombstone consumer discharges this structurally and for free: it holds one frontier
    /// mutex across the entire drain, and every claim site takes that same mutex first, so a
    /// concurrent acknowledgement blocks for the sweep's duration and can never reach this method
    /// while a sweep is in progress. Blocking is the mechanism, and the consumer already pays for
    /// it.
    ///
    /// This obligation is **normative, not advisory**: the strong fence guarantee (the executed
    /// watermark never exceeds the minimum live claim, at all times rather than merely at a
    /// sweep-start snapshot) is conditioned on it. A consumer that admits a claim concurrently
    /// with its own running sweep can register below the snapshot's ceiling and then have
    /// `end_sweep` fence past it.
    ///
    /// [`Self::release_claim`] carries **no such obligation** and is safe during a sweep: a release
    /// can only RAISE the minimum, and the snapshot was already bounded by the pre-release minimum.
    ///
    /// # Admission rule
    ///
    /// 1. `claim == 0` is **not a valid claim** and is refused, recorded as a rejection. A `0`
    ///    claim would pin the ceiling proposal at `0` permanently while rendering
    ///    indistinguishably from *"no live claim"* on the minimum-claim gauge, whose `0` is the
    ///    no-claim sentinel precisely because this refusal makes `0` unrepresentable as a
    ///    recorded claim.
    /// 2. A claim strictly below the executed watermark records **nothing** and returns
    ///    [`ClaimAdmission::BelowExecuted`].
    /// 3. Otherwise the claim is stored as `max(existing, claim)` — monotone per claimant — and
    ///    returns [`ClaimAdmission::Honoured`], **regardless** of where it sits relative to the
    ///    ceiling proposal or to any other claimant.
    fn register_claim(
        &self,
        claimant: &ClaimantId,
        scope: ClaimScope,
        claim: Epoch,
    ) -> ClaimAdmission;

    /// Remove `claimant`'s claim under `scope` from the min-reduction.
    ///
    /// Safe to call during a sweep: a release can only RAISE the minimum, so it cannot invalidate
    /// a snapshot already taken from the pre-release claim set.
    fn release_claim(&self, claimant: &ClaimantId, scope: ClaimScope);

    /// Begin a sweep, taking the ceiling snapshot the sweep is licensed to reclaim below.
    ///
    /// # Contract (HARD)
    ///
    /// - The in-progress check, the ceiling snapshot and the setting of the in-progress flag
    ///   happen in **ONE** lock acquisition. A flag checked or set outside the acquisition that
    ///   produced the snapshot is a TOCTOU window in which two calls both return `Some` and each
    ///   fences the watermark from its own snapshot. The flag is registry **state**, not a calling
    ///   convention.
    /// - A second concurrent sweep is **REFUSED**, not serialised: this returns `None` while a
    ///   sweep is in progress. The choice is not left to the implementer.
    /// - A `None` return means the caller **does not run a pass** — it holds no token, so it must
    ///   not call [`Self::end_sweep`], and under the token's type it cannot.
    fn begin_sweep(&self) -> Option<SweepToken>;

    /// End the sweep `token` licensed, reporting the durable watermark the pass observed.
    ///
    /// # Contract (HARD)
    ///
    /// - This **CONSUMES** the token, by value.
    /// - `durable_watermark` is the **observed** durable watermark and **NEVER** a chosen
    ///   boundary. The caller reports a fact; it proposes no number.
    /// - It **MUST** be the same value the caller's own eligibility filter applied — the hoisted
    ///   local, never a re-read of the field the filter read. A re-read is equal to the filter's
    ///   value only while the whole pass runs inside the caller's own lock; the moment the bracket
    ///   moves outside it, the two diverge and the watermark is driven above what the pass could
    ///   have reclaimed.
    /// - The registry derives `max(executed, min(token_snapshot, durable_watermark + 1))`
    ///   **itself**. The clamp is required, not decorative: a drain's filter is
    ///   `durable_watermark >= e && ceiling > e`, so the reclaimed set is exactly
    ///   `{ e : e <= min(ceiling - 1, durable_watermark) }` and a claim at `v` is safe iff
    ///   `v >= min(ceiling, durable_watermark + 1)`. Using the bare snapshot when the durability
    ///   fence held the pass BELOW the ceiling would fence claimants against content that still
    ///   physically exists — a false fence: safe, but it forces resyncs nobody needs.
    /// - The advance is `max`, **never an assignment**. An assignment would drive the watermark
    ///   BACKWARDS whenever a later sweep runs against an empty claim set (where the snapshot falls
    ///   to the boot floor) and re-admit claims into an already-reclaimed range. That is a
    ///   resurrection bug, not a style preference.
    /// - The recorded value is an **UPPER BOUND** on what was reclaimed, never a certificate: a
    ///   reference whose storage drop failed is handed back for re-insertion, so an epoch below the
    ///   boundary may be back in the index. The direction is safe — the watermark gates admission
    ///   only and never licenses reclamation — so the cost is an unnecessary resync.
    /// - There is deliberately **no success/failure discriminant**. Under the clause above, a
    ///   partial-drop failure still leaves content gone, so it takes the **same** conservative
    ///   advance a clean pass does; a discriminant would be a frozen surface no wired path could
    ///   discriminate on.
    /// - It **MUST** run on every path out of a pass that holds a token, including the
    ///   error/partial-drop path, so the sweep cannot be left flagged in progress. A pass that
    ///   never began a sweep, and a pass whose `begin_sweep` returned `None`, hold no token and
    ///   have nothing to unwind.
    fn end_sweep(&self, token: SweepToken, durable_watermark: Epoch);

    /// The minimum over live claims under `scope`, or `None` when no claim is recorded.
    ///
    /// # Observability ONLY — MUST NOT gate reclamation (HARD)
    ///
    /// This exists for metrics and diagnostics. Reclamation is licensed by
    /// [`Self::prune_ceiling`] and by nothing else. Folding a boundary out of this value at a call
    /// site is the same subset-MIN hazard the frontier's own ranking split already forbids: a
    /// consumer that computes its own boundary is a second authority, which is precisely what this
    /// registry exists to remove.
    fn min_live_claim(&self, scope: ClaimScope) -> Option<Epoch>;

    /// The ceiling **proposal** under `scope`: `min_live_claim - margin`, or the boot floor when no
    /// claim is live.
    ///
    /// # The single authoritative boundary accessor (HARD)
    ///
    /// This is the **only** accessor whose return value a consumer may reclaim strictly below. Any
    /// sweep **consumes** it — through [`Self::begin_sweep`]'s token for a bracketed pass — rather
    /// than computing its own boundary from claims, cursors or watermarks it happens to hold.
    ///
    /// # It MAY FALL
    ///
    /// The value is recomputed from scratch on every query as the fleet MIN, never latched from
    /// the first claimant. A newly registering laggard LOWERS the minimum and is recorded, so the
    /// proposal falls with it — that fall is correct behaviour and is how the laggard regains
    /// protection. **A caller that caches this as a ratchet has reintroduced the first-claimant
    /// latch this design forbids.** Monotonicity is carried by
    /// [`Self::executed_watermark`] instead.
    ///
    /// # It publishes metrics and mutates no boundary state
    ///
    /// A query updates the ceiling, minimum-claim and live-claim gauges and increments the query
    /// counters. It changes no boundary value whatsoever: there is no high-water of past proposals
    /// to retain, because a retained stale proposal is exactly what could later license a
    /// watermark above the current minimum.
    fn prune_ceiling(&self, scope: ClaimScope) -> Epoch;

    /// The executed watermark under `scope` — the monotone boundary, advanced only by a completed
    /// sweep and only with `max`.
    ///
    /// This is the value a claim is measured against for refusal, and it is an **upper bound** on
    /// what has been reclaimed rather than a certificate of it.
    fn executed_watermark(&self, scope: ClaimScope) -> Epoch;

    /// The number of live claims recorded under `scope`.
    fn live_claims(&self, scope: ClaimScope) -> usize;

    /// The effective margin, in epochs, subtracted from the minimum live claim to form the ceiling
    /// proposal.
    ///
    /// Resolved **once** at construction, never per query, so the parse and the arithmetic cannot
    /// observe different answers.
    fn margin_epochs(&self) -> u64;
}

/// Default reclamation margin, in **epochs**, subtracted from the minimum live claim to form the
/// ceiling proposal. Overridden by `TOPGUN_RECLAMATION_MARGIN_EPOCHS`.
///
/// `0` is a valid value meaning *no margin*, and it is the default deliberately: at margin `0` the
/// ceiling is arithmetically the same fleet MIN the pre-registry fold already computed, so
/// introducing the registry moves the **authority** over the boundary without moving the boundary
/// itself. A non-zero margin is a behavioural change and belongs with the consumer that needs it;
/// choosing one on evidence is deferred and owned by TODO-634.
pub const DEFAULT_RECLAMATION_MARGIN_EPOCHS: u64 = 0;

/// Gauge: the last ceiling **proposal** published by [`ReclamationBoundary::prune_ceiling`]. MAY
/// FALL — it is not a ratchet.
pub const METRIC_RECLAMATION_PRUNE_CEILING: &str = "topgun_reclamation_prune_ceiling";

/// Gauge: the executed watermark. Monotone.
pub const METRIC_RECLAMATION_EXECUTED_WATERMARK: &str = "topgun_reclamation_executed_watermark";

/// Gauge: the minimum over live claims.
///
/// `0` means *"no live claim"*, and the sentinel is unambiguous because a `0` claim is refused at
/// the admission surface — a `0` is never a recorded claim, so it cannot collide with a real
/// position.
pub const METRIC_RECLAMATION_MIN_LIVE_CLAIM: &str = "topgun_reclamation_min_live_claim";

/// Gauge: the number of live claims.
pub const METRIC_RECLAMATION_LIVE_CLAIMS: &str = "topgun_reclamation_live_claims";

/// Gauge: the effective margin, in epochs.
pub const METRIC_RECLAMATION_MARGIN_EPOCHS: &str = "topgun_reclamation_margin_epochs";

/// Counter: claims recorded.
pub const METRIC_RECLAMATION_CLAIMS_REGISTERED_TOTAL: &str =
    "topgun_reclamation_claims_registered_total";

/// Counter: claims released.
pub const METRIC_RECLAMATION_CLAIMS_RELEASED_TOTAL: &str =
    "topgun_reclamation_claims_released_total";

/// Counter: claims refused for lying below already-reclaimed content (the skip counter).
///
/// Named for the executed watermark rather than the ceiling because a below-**ceiling** claim is
/// recorded, not refused — a ceiling-named counter would describe a refusal that does not exist.
pub const METRIC_RECLAMATION_CLAIMS_REJECTED_BELOW_EXECUTED_TOTAL: &str =
    "topgun_reclamation_claims_rejected_below_executed_total";

/// Counter: [`ReclamationBoundary::prune_ceiling`] calls.
pub const METRIC_RECLAMATION_CEILING_QUERIES_TOTAL: &str =
    "topgun_reclamation_ceiling_queries_total";

/// Counter: ceiling queries where a live claim, rather than the boot floor, determined the
/// proposal.
pub const METRIC_RECLAMATION_CEILING_PINNED_QUERIES_TOTAL: &str =
    "topgun_reclamation_ceiling_pinned_queries_total";

/// Counter: total epochs the executed watermark has advanced.
pub const METRIC_RECLAMATION_EXECUTED_EPOCHS_ADVANCED_TOTAL: &str =
    "topgun_reclamation_executed_epochs_advanced_total";

/// Counter: completed sweeps — [`ReclamationBoundary::end_sweep`] calls.
///
/// A sweep whose [`ReclamationBoundary::begin_sweep`] returned `None` never runs and never counts
/// here; a stuck sweep is visible on the in-progress gauge rather than as a missing count.
pub const METRIC_RECLAMATION_SWEEPS_TOTAL: &str = "topgun_reclamation_sweeps_total";

/// Gauge: `1` while a sweep is running, `0` otherwise.
///
/// This is the loud signal for a leaked [`SweepToken`] — pinned at `1` with
/// [`METRIC_RECLAMATION_SWEEPS_TOTAL`] flat.
pub const METRIC_RECLAMATION_SWEEP_IN_PROGRESS: &str = "topgun_reclamation_sweep_in_progress";
