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

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use metrics::{Counter, Gauge};
use tracing::warn;

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
    // lock. Its only reader is `ReclamationRegistry::end_sweep`, and its only writer is
    // `begin_sweep`, both in this module.
    ceiling_snapshot: Epoch,
}

impl SweepToken {
    /// Mint a token over a ceiling snapshot taken under the registry lock.
    ///
    /// Module-private: minting outside the acquisition that took the snapshot would hand a
    /// consumer a licence the registry never granted, which is exactly what the token exists to
    /// make unrepresentable.
    fn new(ceiling_snapshot: Epoch) -> Self {
        Self { ceiling_snapshot }
    }

    /// The ceiling this sweep is licensed by: the proposal as it stood in the acquisition that
    /// admitted the sweep.
    ///
    /// A sweep MUST filter on THIS value and not on a fresh [`ReclamationBoundary::prune_ceiling`]
    /// query, for two reasons. First, a re-query is a different number the moment the sweep's
    /// bracket stops excluding admissions, and reclaiming above the snapshot the watermark is later
    /// fenced from is the fence-past-a-live-claim hazard the token exists to prevent. Second,
    /// `prune_ceiling` is an OBSERVING call — it counts the query and republishes the gauges — so
    /// re-querying it per epoch would inflate operator-visible counters with the sweep's own
    /// internal arithmetic.
    ///
    /// Reading the licence is safe where minting one is not: this hands back the number the
    /// registry already granted, and no number can be turned back into a token.
    #[must_use]
    pub fn ceiling(&self) -> Epoch {
        self.ceiling_snapshot
    }
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

/// Environment variable carrying the operator-chosen reclamation margin, in epochs.
const ENV_RECLAMATION_MARGIN_EPOCHS: &str = "TOPGUN_RECLAMATION_MARGIN_EPOCHS";

/// Resolve the effective margin from the environment, **once**, at registry construction.
///
/// Never per query: a per-query read would let the parse and the ceiling arithmetic observe
/// different answers within one pass, so the boundary a sweep snapshotted and the boundary its
/// `end_sweep` fenced from could disagree for no reason a consumer could see.
///
/// An absent variable takes the default silently. Anything else is routed through
/// [`parse_margin_epochs`], which never panics — this runs inside every `TombstoneFrontier`
/// construction, including every test's, so aborting over a typo in one variable would take down
/// far more than the knob it describes.
fn resolve_margin_epochs() -> u64 {
    match std::env::var(ENV_RECLAMATION_MARGIN_EPOCHS) {
        Ok(raw) => parse_margin_epochs(&raw),
        Err(_) => DEFAULT_RECLAMATION_MARGIN_EPOCHS,
    }
}

/// Parse one raw margin value, falling back to the default **loudly**.
///
/// A silent fallback would leave an operator who set a value believing it took effect, so an
/// unparseable value warns with the rejected text rather than disappearing.
///
/// `0` is VALID here and means *no margin* — it is this increment's default. That is the one
/// place this parse deliberately differs from `TOPGUN_EPOCH_WIDTH`'s, which rejects `0` because a
/// zero-width epoch is meaningless; a zero-width margin is merely the absence of slack.
fn parse_margin_epochs(raw: &str) -> u64 {
    if let Ok(margin) = raw.trim().parse::<u64>() {
        return margin;
    }

    warn!(
        rejected_value = %raw,
        default = DEFAULT_RECLAMATION_MARGIN_EPOCHS,
        "invalid TOPGUN_RECLAMATION_MARGIN_EPOCHS (expected a non-negative integer); \
         using the default"
    );
    DEFAULT_RECLAMATION_MARGIN_EPOCHS
}

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

/// Resolve a counter handle and touch it, so the series is registered before any observation.
fn touched_counter(name: &'static str) -> Counter {
    let handle = metrics::counter!(name);
    // An `increment(0)` is what puts the series in the exporter's registry; merely resolving the
    // handle does not. Without it the series is absent until its first real observation, and a
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

/// One cached handle per pinned registry series — six gauges and seven counters, **thirteen**.
///
/// Handles are resolved once, at registry construction, and never re-resolved: a `counter!` /
/// `gauge!` macro invocation at an emit site performs a registry lookup by name, and the ceiling
/// query sits on the prune path.
///
/// Every field is emitted through its pinned name constant. A string literal at an emit site is a
/// defect: binding the emitter to the constant is what makes the compiler, rather than a document
/// review, the thing that stops an emitted name drifting from the pinned one.
///
/// # Construction-order precondition (depended on, not established here)
///
/// A handle resolved **before** a recorder is installed binds to a no-op for that handle's whole
/// lifetime — it never re-resolves. Caching at construction is therefore only sound if the
/// recorder is installed first, at every production site and in every test that renders.
///
/// Registration is eager rather than lazy-on-first-use, so every pinned series exists in the
/// FIRST scrape and *"has not moved"* is never confusable with *"does not exist"*.
struct RegistryMetrics {
    prune_ceiling: Gauge,
    executed_watermark: Gauge,
    min_live_claim: Gauge,
    live_claims: Gauge,
    margin_epochs: Gauge,
    sweep_in_progress: Gauge,
    claims_registered: Counter,
    claims_released: Counter,
    claims_rejected_below_executed: Counter,
    ceiling_queries: Counter,
    ceiling_pinned_queries: Counter,
    executed_epochs_advanced: Counter,
    sweeps: Counter,
}

impl RegistryMetrics {
    /// Resolve and touch all thirteen series, then publish the effective margin.
    ///
    /// The margin is published here and nowhere else: it is fixed for the registry's lifetime, so
    /// a re-publish on any later path could only ever restate the same number.
    #[allow(clippy::cast_precision_loss)]
    fn new(margin_epochs: u64) -> Self {
        let metrics = Self {
            prune_ceiling: touched_gauge(METRIC_RECLAMATION_PRUNE_CEILING),
            executed_watermark: touched_gauge(METRIC_RECLAMATION_EXECUTED_WATERMARK),
            min_live_claim: touched_gauge(METRIC_RECLAMATION_MIN_LIVE_CLAIM),
            live_claims: touched_gauge(METRIC_RECLAMATION_LIVE_CLAIMS),
            margin_epochs: touched_gauge(METRIC_RECLAMATION_MARGIN_EPOCHS),
            sweep_in_progress: touched_gauge(METRIC_RECLAMATION_SWEEP_IN_PROGRESS),

            claims_registered: touched_counter(METRIC_RECLAMATION_CLAIMS_REGISTERED_TOTAL),
            claims_released: touched_counter(METRIC_RECLAMATION_CLAIMS_RELEASED_TOTAL),
            claims_rejected_below_executed: touched_counter(
                METRIC_RECLAMATION_CLAIMS_REJECTED_BELOW_EXECUTED_TOTAL,
            ),
            ceiling_queries: touched_counter(METRIC_RECLAMATION_CEILING_QUERIES_TOTAL),
            ceiling_pinned_queries: touched_counter(
                METRIC_RECLAMATION_CEILING_PINNED_QUERIES_TOTAL,
            ),
            executed_epochs_advanced: touched_counter(
                METRIC_RECLAMATION_EXECUTED_EPOCHS_ADVANCED_TOTAL,
            ),
            sweeps: touched_counter(METRIC_RECLAMATION_SWEEPS_TOTAL),
        };
        metrics.margin_epochs.set(margin_epochs as f64);
        metrics
    }
}

/// The registry's whole mutable state, behind one leaf mutex.
///
/// What this struct **does not** carry is as load-bearing as what it does:
///
/// - **no ceiling latch and no high-water of past proposals** — the ceiling is recomputed from
///   the claim set on every query, so there is no retained stale proposal that could later license
///   a watermark above the current minimum;
/// - **no `ceiling_snapshot` slot** — a sweep's snapshot travels inside its [`SweepToken`], so
///   there is no single field a second sweep could overwrite;
/// - **no pending-admission queue** — sweep/admission exclusion is a caller obligation, and a
///   queue no caller can enter would be a surface with no user.
struct RegistryState {
    /// Live claims, keyed by claimant and scope. Absence is *"no claim"*; a recorded value is
    /// always `>= 1`, because a `0` claim is refused at admission.
    claims: HashMap<(ClaimantId, ClaimScope), Epoch>,
    /// The monotone executed watermark per scope. A missing entry reads as the boot floor.
    executed: HashMap<ClaimScope, Epoch>,
    /// The floor the registry starts from, for both the empty-claim-set ceiling and the
    /// never-swept watermark.
    boot_floor: Epoch,
    /// Resolved once at construction, never per query, so the resolution and the arithmetic
    /// cannot observe different answers.
    margin_epochs: u64,
    /// `true` between a granted `begin_sweep` and its `end_sweep`. The single-sweep guard, and
    /// the value behind the in-progress gauge — registry state, not a calling convention.
    sweep_in_progress: bool,
}

impl RegistryState {
    /// The fleet MIN over live claims under `scope`, or `None` when none is recorded.
    ///
    /// Folded from scratch over the claim set on every call. Caching or clamping the result
    /// against a previous one would reintroduce the first-claimant latch the two-phase split
    /// exists to prevent.
    fn min_live_claim(&self, scope: ClaimScope) -> Option<Epoch> {
        self.claims
            .iter()
            .filter(|((_, claim_scope), _)| *claim_scope == scope)
            .map(|(_, claim)| *claim)
            .min()
    }

    /// The ceiling proposal under `scope`, recomputed from the current claim set.
    ///
    /// Saturating, so a margin wider than the minimum claim floors the proposal at `0` rather
    /// than wrapping into a boundary near `u64::MAX`.
    fn prune_ceiling(&self, scope: ClaimScope) -> Epoch {
        match self.min_live_claim(scope) {
            Some(min) => min.saturating_sub(self.margin_epochs),
            None => self.boot_floor,
        }
    }

    /// The executed watermark under `scope`; the boot floor on a registry that has never swept.
    fn executed_watermark(&self, scope: ClaimScope) -> Epoch {
        self.executed
            .get(&scope)
            .copied()
            .unwrap_or(self.boot_floor)
    }

    /// The number of live claims recorded under `scope`.
    fn live_claims(&self, scope: ClaimScope) -> usize {
        self.claims
            .keys()
            .filter(|(_, claim_scope)| *claim_scope == scope)
            .count()
    }
}

/// The in-memory [`ReclamationBoundary`] implementation: one leaf mutex over [`RegistryState`].
///
/// # Locking
///
/// The mutex is a **leaf**: the registry acquires no other lock and never calls back into a
/// consumer. A sweep holds it at exactly two points — `begin_sweep` (check, snapshot and flag in
/// one acquisition) and `end_sweep` (advance and clear) — and never for the pass's duration, so a
/// long pass cannot block an unrelated ceiling query behind it.
///
/// # Metrics
///
/// The metric handles live beside the mutex rather than inside the state, because they are
/// immutable after construction and therefore need no lock of their own. That siting is
/// load-bearing: an emit from inside a critical section stays a plain atomic increment, and a
/// helper that publishes a gauge takes the already-held guard by reference instead of re-entering
/// a non-reentrant `std::sync::Mutex`.
pub struct ReclamationRegistry {
    state: Mutex<RegistryState>,
    metrics: RegistryMetrics,
}

impl ReclamationRegistry {
    /// Build a registry over `boot_floor`, resolving the margin from the environment.
    ///
    /// `TOPGUN_RECLAMATION_MARGIN_EPOCHS` is read **once**, here, and never re-read per query; an
    /// unparseable value warns and takes the default rather than panicking. The effective value is
    /// then published as a gauge, so it is confirmable on a `/metrics` scrape rather than only on
    /// a boot line.
    ///
    /// The registry is **in-memory only**: claims and the executed watermark are fresh at every
    /// construction and survive no restart. With `boot_floor = 0` that is benign — a restarted
    /// registry starts with no fence and licenses nothing until a claim is registered and a sweep
    /// completes. A caller that passes a non-zero floor inherits the obligation that the floor is
    /// non-decreasing across restarts.
    #[must_use]
    pub fn new(boot_floor: Epoch) -> Self {
        Self::with_margin(boot_floor, resolve_margin_epochs())
    }

    /// Build a registry over `boot_floor` with an explicitly resolved `margin_epochs`.
    ///
    /// For callers that resolve the margin themselves (and for tests that need a deterministic
    /// one). Same once-at-construction rule as [`Self::new`]: the value is stored, never re-read.
    ///
    /// All thirteen metric series are resolved and touched here, so the first scrape after
    /// construction renders every one of them at zero. A caller that constructs a registry before
    /// installing its recorder gets thirteen no-op handles for the process's lifetime.
    #[must_use]
    pub fn with_margin(boot_floor: Epoch, margin_epochs: u64) -> Self {
        Self {
            state: Mutex::new(RegistryState {
                claims: HashMap::new(),
                executed: HashMap::new(),
                boot_floor,
                margin_epochs,
                sweep_in_progress: false,
            }),
            metrics: RegistryMetrics::new(margin_epochs),
        }
    }

    /// Acquire the state lock, recovering from poisoning rather than propagating a panic.
    ///
    /// The state is a plain claim map with no partially-applied invariant, so a guard recovered
    /// from a panicking holder is usable; taking the server down over it would turn a bounded
    /// failure into an outage.
    fn lock(&self) -> MutexGuard<'_, RegistryState> {
        self.state.lock().unwrap_or_else(|poison: PoisonError<_>| {
            warn!("recovered a poisoned reclamation-registry mutex (a prior holder panicked)");
            poison.into_inner()
        })
    }

    /// Publish the claim-set gauges from an ALREADY-HELD guard.
    ///
    /// Takes the state by reference rather than re-locking. Re-entering the registry mutex from
    /// inside a critical section would self-deadlock on a non-reentrant `std::sync::Mutex`, and
    /// re-reading it after releasing the guard would publish a claim set that a concurrent writer
    /// may already have moved past the one the caller acted on.
    #[allow(clippy::cast_precision_loss)]
    fn publish_claim_gauges(&self, state: &RegistryState, scope: ClaimScope) {
        // The `0` published for an empty claim set is the no-claim sentinel, and it is unambiguous
        // only because a `0` claim is refused at admission — no recorded claim can collide with
        // it.
        self.metrics
            .min_live_claim
            .set(state.min_live_claim(scope).map_or(0.0, |min| min as f64));
        self.metrics
            .live_claims
            .set(state.live_claims(scope) as f64);
    }
}

impl ReclamationBoundary for ReclamationRegistry {
    fn register_claim(
        &self,
        claimant: &ClaimantId,
        scope: ClaimScope,
        claim: Epoch,
    ) -> ClaimAdmission {
        let mut state = self.lock();

        // An admission that lands while this registry's own sweep is in flight can record a claim
        // below that sweep's ceiling snapshot, which `end_sweep` would then fence past — so the
        // consumer must serialise the two, and this catches a consumer that does not. It is a
        // `debug_assert` on purpose: the exclusion is a caller obligation rather than an
        // invariant the registry can enforce, and aborting a release binary over a caller's
        // scheduling would be a worse failure than the over-refusal the breach can cause.
        debug_assert!(
            !state.sweep_in_progress,
            "register_claim ran concurrently with a sweep: admissions and sweeps are mutually \
             exclusive, and the caller must serialise them"
        );

        // A 0 claim would pin the ceiling proposal at 0 for good while reading exactly like
        // "no live claim" on the minimum-claim gauge, whose 0 is the no-claim sentinel.
        if claim == 0 {
            // Both refusal exits bump the same skip counter. Counting only the watermark exit
            // would under-report the zero guard, and the two are indistinguishable to an operator
            // reading the returned `BelowExecuted` anyway.
            self.metrics.claims_rejected_below_executed.increment(1);
            return ClaimAdmission::BelowExecuted {
                claim,
                executed: state.executed_watermark(scope),
            };
        }

        let executed = state.executed_watermark(scope);
        if claim < executed {
            self.metrics.claims_rejected_below_executed.increment(1);
            return ClaimAdmission::BelowExecuted { claim, executed };
        }

        // Monotone per claimant, and unconditional above the watermark: where the claim sits
        // relative to the ceiling proposal or to any other claimant is not an admission input.
        let stored = *state
            .claims
            .entry((claimant.clone(), scope))
            .and_modify(|existing| *existing = (*existing).max(claim))
            .or_insert(claim);

        self.metrics.claims_registered.increment(1);
        self.publish_claim_gauges(&state, scope);

        ClaimAdmission::Honoured { claim: stored }
    }

    fn release_claim(&self, claimant: &ClaimantId, scope: ClaimScope) {
        let mut state = self.lock();
        let removed = state.claims.remove(&(claimant.clone(), scope)).is_some();

        // Counted only when a live claim actually left the fold. Releasing a claimant that never
        // held one is the frontier's ordinary `forget_client` for a device that never claimed, and
        // counting it would report claim churn that never happened.
        if removed {
            self.metrics.claims_released.increment(1);
        }
        self.publish_claim_gauges(&state, scope);
    }

    fn begin_sweep(&self) -> Option<SweepToken> {
        // ONE acquisition covers the flag check, the snapshot and the flag set. Splitting them
        // across two acquisitions opens a window in which two callers both observe the flag clear
        // and both receive a token, and each then fences the watermark from its own snapshot.
        let mut state = self.lock();

        if state.sweep_in_progress {
            return None;
        }

        // The inherent helper, NOT the trait method: the trait method takes the lock, and calling
        // it from inside this critical section would self-deadlock.
        let ceiling_snapshot = state.prune_ceiling(ClaimScope::Global);
        state.sweep_in_progress = true;

        // Set on the granted path only. A refused `begin_sweep` starts no sweep, so moving the
        // gauge there would report a second sweep running that never began — and the gauge would
        // then be pinned at `1` by the very call the guard turned away.
        //
        // Deliberately does NOT move the ceiling gauge or bump the ceiling-query counters: both
        // are pinned to `prune_ceiling` CALLS, and folding this internal snapshot into them would
        // make an operator's query rate report sweeps as consumer queries. The snapshot is still
        // observable — it is what `end_sweep` advances the executed watermark from.
        self.metrics.sweep_in_progress.set(1.0);

        Some(SweepToken::new(ceiling_snapshot))
    }

    #[allow(clippy::cast_precision_loss)]
    fn end_sweep(&self, token: SweepToken, durable_watermark: Epoch) {
        let mut state = self.lock();

        // The pass could only reclaim epochs its filter admitted, i.e. below the ceiling AND at
        // or below the durable watermark, so the safe claim boundary is min(ceiling, w + 1).
        // Deriving the bare ceiling when the durability fence held the pass back would fence
        // claimants against content that still physically exists.
        let candidate = token
            .ceiling_snapshot
            .min(durable_watermark.saturating_add(1));

        let boot_floor = state.boot_floor;
        let executed = state
            .executed
            .entry(ClaimScope::Global)
            .or_insert(boot_floor);
        // `max`, never assignment: a later sweep over an empty claim set snapshots the boot floor,
        // and assigning that would drag the watermark backwards and re-admit claims into an
        // already-reclaimed range.
        let previous = *executed;
        *executed = previous.max(candidate);
        let advanced = *executed - previous;
        let watermark = *executed;

        state.sweep_in_progress = false;
        drop(state);

        self.metrics.sweeps.increment(1);
        self.metrics.sweep_in_progress.set(0.0);
        // The DELTA, not the new value: a sweep that advanced nothing must leave this counter
        // flat, which is what lets an operator separate "sweeps are running" from "sweeps are
        // reclaiming".
        self.metrics.executed_epochs_advanced.increment(advanced);
        self.metrics.executed_watermark.set(watermark as f64);
    }

    fn min_live_claim(&self, scope: ClaimScope) -> Option<Epoch> {
        self.lock().min_live_claim(scope)
    }

    #[allow(clippy::cast_precision_loss)]
    fn prune_ceiling(&self, scope: ClaimScope) -> Epoch {
        let state = self.lock();
        let ceiling = state.prune_ceiling(scope);
        // A live claim, rather than the boot floor, determined the proposal.
        let pinned = state.min_live_claim(scope).is_some();
        self.publish_claim_gauges(&state, scope);
        drop(state);

        // Observation only — this query moves no boundary state. Every emit below is a metric
        // handle; a boundary write here would make reading the ceiling change the ceiling.
        self.metrics.ceiling_queries.increment(1);
        if pinned {
            self.metrics.ceiling_pinned_queries.increment(1);
        }
        self.metrics.prune_ceiling.set(ceiling as f64);

        ceiling
    }

    fn executed_watermark(&self, scope: ClaimScope) -> Epoch {
        self.lock().executed_watermark(scope)
    }

    fn live_claims(&self, scope: ClaimScope) -> usize {
        self.lock().live_claims(scope)
    }

    fn margin_epochs(&self) -> u64 {
        self.lock().margin_epochs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};
    use std::thread;

    fn claimant(name: &str) -> ClaimantId {
        name.to_string()
    }

    #[test]
    fn empty_claim_set_proposes_the_boot_floor_and_reports_no_minimum() {
        let registry = ReclamationRegistry::new(7);

        assert_eq!(registry.min_live_claim(ClaimScope::Global), None);
        assert_eq!(registry.prune_ceiling(ClaimScope::Global), 7);
        assert_eq!(registry.executed_watermark(ClaimScope::Global), 7);
        assert_eq!(registry.live_claims(ClaimScope::Global), 0);
    }

    #[test]
    fn the_proposal_is_the_fleet_min_over_every_live_claim() {
        let registry = ReclamationRegistry::new(0);

        registry.register_claim(&claimant("a"), ClaimScope::Global, 40);
        registry.register_claim(&claimant("b"), ClaimScope::Global, 12);
        registry.register_claim(&claimant("c"), ClaimScope::Global, 25);

        assert_eq!(registry.min_live_claim(ClaimScope::Global), Some(12));
        assert_eq!(registry.prune_ceiling(ClaimScope::Global), 12);
        assert_eq!(registry.live_claims(ClaimScope::Global), 3);
    }

    #[test]
    fn a_rejoining_laggard_lowers_the_proposal_and_is_recorded() {
        let registry = ReclamationRegistry::new(0);

        registry.register_claim(&claimant("leader"), ClaimScope::Global, 50);
        assert_eq!(registry.prune_ceiling(ClaimScope::Global), 50);

        let verdict = registry.register_claim(&claimant("laggard"), ClaimScope::Global, 9);

        assert_eq!(verdict, ClaimAdmission::Honoured { claim: 9 });
        assert_eq!(
            registry.prune_ceiling(ClaimScope::Global),
            9,
            "the proposal is a fleet MIN and must fall when a laggard rejoins the fold"
        );
    }

    #[test]
    fn the_proposal_does_not_depend_on_registration_order() {
        let forward = ReclamationRegistry::new(0);
        forward.register_claim(&claimant("a"), ClaimScope::Global, 5);
        forward.register_claim(&claimant("b"), ClaimScope::Global, 30);

        let reverse = ReclamationRegistry::new(0);
        reverse.register_claim(&claimant("b"), ClaimScope::Global, 30);
        reverse.register_claim(&claimant("a"), ClaimScope::Global, 5);

        assert_eq!(
            forward.prune_ceiling(ClaimScope::Global),
            reverse.prune_ceiling(ClaimScope::Global)
        );
        assert_eq!(forward.prune_ceiling(ClaimScope::Global), 5);
    }

    #[test]
    fn a_claim_is_monotone_per_claimant() {
        let registry = ReclamationRegistry::new(0);

        registry.register_claim(&claimant("a"), ClaimScope::Global, 30);
        let verdict = registry.register_claim(&claimant("a"), ClaimScope::Global, 11);

        assert_eq!(
            verdict,
            ClaimAdmission::Honoured { claim: 30 },
            "a claimant never walks its own claim backwards"
        );
        assert_eq!(registry.min_live_claim(ClaimScope::Global), Some(30));
        assert_eq!(registry.live_claims(ClaimScope::Global), 1);
    }

    #[test]
    fn a_release_removes_the_claim_and_raises_the_minimum() {
        let registry = ReclamationRegistry::new(0);

        registry.register_claim(&claimant("a"), ClaimScope::Global, 4);
        registry.register_claim(&claimant("b"), ClaimScope::Global, 20);
        assert_eq!(registry.prune_ceiling(ClaimScope::Global), 4);

        registry.release_claim(&claimant("a"), ClaimScope::Global);

        assert_eq!(registry.live_claims(ClaimScope::Global), 1);
        assert_eq!(registry.prune_ceiling(ClaimScope::Global), 20);
    }

    #[test]
    fn the_margin_is_subtracted_and_saturates_at_zero() {
        let registry = ReclamationRegistry::with_margin(0, 5);
        registry.register_claim(&claimant("a"), ClaimScope::Global, 12);
        assert_eq!(registry.margin_epochs(), 5);
        assert_eq!(registry.prune_ceiling(ClaimScope::Global), 7);

        let narrow = ReclamationRegistry::with_margin(0, 100);
        narrow.register_claim(&claimant("a"), ClaimScope::Global, 3);
        assert_eq!(
            narrow.prune_ceiling(ClaimScope::Global),
            0,
            "a margin wider than the minimum claim floors the proposal, never wraps it"
        );
    }

    #[test]
    fn a_zero_claim_is_always_refused_and_records_nothing() {
        let registry = ReclamationRegistry::new(0);

        let verdict = registry.register_claim(&claimant("a"), ClaimScope::Global, 0);

        assert_eq!(
            verdict,
            ClaimAdmission::BelowExecuted {
                claim: 0,
                executed: 0
            }
        );
        assert_eq!(registry.live_claims(ClaimScope::Global), 0);
        assert_eq!(registry.min_live_claim(ClaimScope::Global), None);
    }

    #[test]
    fn the_refusal_boundary_is_the_executed_watermark_not_the_ceiling() {
        let registry = ReclamationRegistry::new(0);
        registry.register_claim(&claimant("a"), ClaimScope::Global, 10);
        let token = registry.begin_sweep().expect("first sweep is granted");
        registry.end_sweep(token, 100);
        assert_eq!(registry.executed_watermark(ClaimScope::Global), 10);

        // At the watermark: admitted. One below it: refused.
        assert_eq!(
            registry.register_claim(&claimant("at"), ClaimScope::Global, 10),
            ClaimAdmission::Honoured { claim: 10 }
        );
        assert_eq!(
            registry.register_claim(&claimant("under"), ClaimScope::Global, 9),
            ClaimAdmission::BelowExecuted {
                claim: 9,
                executed: 10
            }
        );
        assert_eq!(registry.live_claims(ClaimScope::Global), 2);

        // Behind another claimant, but at or above the watermark, is not a refusal reason.
        assert_eq!(
            registry.register_claim(&claimant("behind"), ClaimScope::Global, 11),
            ClaimAdmission::Honoured { claim: 11 }
        );
        assert_eq!(registry.prune_ceiling(ClaimScope::Global), 10);
    }

    #[test]
    fn end_sweep_clamps_the_snapshot_by_the_reported_durable_watermark() {
        let registry = ReclamationRegistry::new(0);
        registry.register_claim(&claimant("a"), ClaimScope::Global, 40);

        let token = registry.begin_sweep().expect("first sweep is granted");
        registry.end_sweep(token, 6);

        assert_eq!(
            registry.executed_watermark(ClaimScope::Global),
            7,
            "a pass held below the ceiling by the durability fence must not fence past what it \
             could have reclaimed"
        );
    }

    #[test]
    fn end_sweep_advances_with_max_and_never_overwrites() {
        let registry = ReclamationRegistry::new(0);
        registry.register_claim(&claimant("a"), ClaimScope::Global, 30);

        let token = registry.begin_sweep().expect("first sweep is granted");
        registry.end_sweep(token, 100);
        assert_eq!(registry.executed_watermark(ClaimScope::Global), 30);

        // An empty claim set snapshots the boot floor; an assignment would drag the watermark
        // back to 0 and re-admit claims into the reclaimed range.
        registry.release_claim(&claimant("a"), ClaimScope::Global);
        let token = registry.begin_sweep().expect("second sweep is granted");
        registry.end_sweep(token, 100);

        assert_eq!(registry.executed_watermark(ClaimScope::Global), 30);
        assert_eq!(registry.prune_ceiling(ClaimScope::Global), 0);
    }

    #[test]
    fn a_lower_later_sweep_cannot_lower_the_watermark() {
        let registry = ReclamationRegistry::new(0);
        registry.register_claim(&claimant("a"), ClaimScope::Global, 50);

        let token = registry.begin_sweep().expect("first sweep is granted");
        registry.end_sweep(token, 100);
        assert_eq!(registry.executed_watermark(ClaimScope::Global), 50);

        let token = registry.begin_sweep().expect("second sweep is granted");
        registry.end_sweep(token, 3);

        assert_eq!(registry.executed_watermark(ClaimScope::Global), 50);
    }

    #[test]
    fn a_second_sweep_is_refused_while_one_is_in_progress() {
        let registry = ReclamationRegistry::new(0);
        registry.register_claim(&claimant("a"), ClaimScope::Global, 20);

        let token = registry.begin_sweep().expect("first sweep is granted");
        assert!(
            registry.begin_sweep().is_none(),
            "a second concurrent sweep is refused, not serialised"
        );

        registry.end_sweep(token, 100);

        let token = registry
            .begin_sweep()
            .expect("a sweep is grantable again once ended");
        registry.end_sweep(token, 100);
    }

    #[test]
    fn a_release_during_a_sweep_cannot_drop_the_minimum_below_the_watermark() {
        let registry = ReclamationRegistry::new(0);
        registry.register_claim(&claimant("low"), ClaimScope::Global, 10);
        registry.register_claim(&claimant("high"), ClaimScope::Global, 60);

        let token = registry.begin_sweep().expect("first sweep is granted");
        registry.release_claim(&claimant("low"), ClaimScope::Global);
        registry.end_sweep(token, 100);

        let min = registry
            .min_live_claim(ClaimScope::Global)
            .expect("one claim survives the release");
        assert!(
            registry.executed_watermark(ClaimScope::Global) <= min,
            "the watermark must stay at or below the minimum live claim at all times"
        );
        assert_eq!(registry.executed_watermark(ClaimScope::Global), 10);
    }

    /// Two threads released from a barrier into `begin_sweep`: exactly one may win.
    ///
    /// A sequential driver cannot discriminate a check-then-set split across two lock
    /// acquisitions — the flag is set by the time the first call returns either way, so a later
    /// sequential call still sees it. Only concurrency reaches the window, which is why this
    /// probe exists alongside the sequential coverage.
    #[test]
    fn concurrent_begin_sweep_calls_yield_exactly_one_token() {
        const ITERATIONS: usize = 1_000;

        let registry = Arc::new(ReclamationRegistry::new(0));
        registry.register_claim(&claimant("a"), ClaimScope::Global, 32);

        // Both workers live for the whole run and the driver joins their barriers, so every round
        // releases two ALREADY-RUNNING threads into the call at once. Spawning a fresh pair per
        // round instead staggers them by the spawn cost — far wider than the window under test —
        // and the probe then passes over a registry that grants two tokens.
        let round_start = Arc::new(Barrier::new(3));
        let round_end = Arc::new(Barrier::new(3));
        let granted: Arc<Mutex<Vec<SweepToken>>> = Arc::new(Mutex::new(Vec::new()));

        let workers: Vec<_> = (0..2)
            .map(|_| {
                let registry = Arc::clone(&registry);
                let round_start = Arc::clone(&round_start);
                let round_end = Arc::clone(&round_end);
                let granted = Arc::clone(&granted);
                thread::spawn(move || {
                    for _ in 0..ITERATIONS {
                        round_start.wait();
                        if let Some(token) = registry.begin_sweep() {
                            granted.lock().expect("granted-token mutex").push(token);
                        }
                        round_end.wait();
                    }
                })
            })
            .collect();

        for round in 0..ITERATIONS {
            round_start.wait();
            round_end.wait();

            // Taken out of the shared vector before the assertion, so a failing round reports the
            // count instead of poisoning the workers' mutex first.
            let mut tokens = std::mem::take(&mut *granted.lock().expect("granted-token mutex"));

            assert_eq!(
                tokens.len(),
                1,
                "exactly one of two concurrent begin_sweep calls may be granted a token \
                 (round {round})"
            );

            let token = tokens
                .pop()
                .expect("the winning token was just asserted present");
            registry.end_sweep(token, 100);
        }

        for worker in workers {
            worker.join().expect("a probe thread panicked");
        }

        assert_eq!(registry.executed_watermark(ClaimScope::Global), 32);
    }

    /// A well-formed value is taken verbatim, and `0` is a VALUE rather than an unset-like
    /// sentinel — it is this increment's default and means *no margin*.
    #[test]
    fn a_well_formed_margin_is_taken_verbatim_including_zero() {
        for (raw, expected) in [("0", 0), ("1", 1), ("7", 7), ("  12  ", 12)] {
            assert_eq!(
                parse_margin_epochs(raw),
                expected,
                "{raw:?} is a well-formed margin and must parse to {expected}"
            );
        }
    }

    /// Anything unparseable falls back to the default instead of panicking: this parse runs inside
    /// every `TombstoneFrontier` construction, so a panic would take down every test in the tree
    /// over a typo in one variable.
    #[test]
    fn an_unparseable_margin_falls_back_to_the_default_without_panicking() {
        for raw in ["", "   ", "banana", "-1", "1.5", "7epochs", "0x10"] {
            assert_eq!(
                parse_margin_epochs(raw),
                DEFAULT_RECLAMATION_MARGIN_EPOCHS,
                "{raw:?} is unparseable and must take the default"
            );
        }
    }

    /// With no override in the environment, the constructor resolves the compiled-in default.
    #[test]
    fn a_registry_built_without_the_override_takes_the_default_margin() {
        if std::env::var(ENV_RECLAMATION_MARGIN_EPOCHS).is_ok() {
            // An operator-set variable in the test process would make this an assertion about
            // their shell rather than about the default. The end-to-end override is proven on a
            // real server start instead, and the effective value is readable on `/metrics`.
            return;
        }
        assert_eq!(
            ReclamationRegistry::new(0).margin_epochs(),
            DEFAULT_RECLAMATION_MARGIN_EPOCHS
        );
    }

    mod metrics_transport {
        use super::*;
        use metrics_exporter_prometheus::PrometheusBuilder;

        const RECLAMATION_GAUGE_NAMES: [&str; 6] = [
            METRIC_RECLAMATION_PRUNE_CEILING,
            METRIC_RECLAMATION_EXECUTED_WATERMARK,
            METRIC_RECLAMATION_MIN_LIVE_CLAIM,
            METRIC_RECLAMATION_LIVE_CLAIMS,
            METRIC_RECLAMATION_MARGIN_EPOCHS,
            METRIC_RECLAMATION_SWEEP_IN_PROGRESS,
        ];

        const RECLAMATION_COUNTER_NAMES: [&str; 7] = [
            METRIC_RECLAMATION_CLAIMS_REGISTERED_TOTAL,
            METRIC_RECLAMATION_CLAIMS_RELEASED_TOTAL,
            METRIC_RECLAMATION_CLAIMS_REJECTED_BELOW_EXECUTED_TOTAL,
            METRIC_RECLAMATION_CEILING_QUERIES_TOTAL,
            METRIC_RECLAMATION_CEILING_PINNED_QUERIES_TOTAL,
            METRIC_RECLAMATION_EXECUTED_EPOCHS_ADVANCED_TOTAL,
            METRIC_RECLAMATION_SWEEPS_TOTAL,
        ];

        /// The series name the queue would have carried. It has no name constant on purpose —
        /// this literal exists so its ABSENCE from a render is asserted rather than assumed.
        const DELETED_QUEUE_SERIES: &str = "topgun_reclamation_admissions_queued_total";

        fn rendered_value<'a>(rendered: &'a str, name: &str) -> Option<&'a str> {
            rendered.lines().find_map(|line| {
                line.strip_prefix(name)
                    .and_then(|rest| rest.strip_prefix(' '))
            })
        }

        fn assert_series(rendered: &str, name: &str, expected: &str) {
            assert_eq!(
                rendered_value(rendered, name),
                Some(expected),
                "{name} must render {expected}; render was:\n{rendered}"
            );
        }

        /// Value lines (not `# HELP` / `# TYPE`) under the registry's prefix.
        fn reclamation_value_lines(rendered: &str) -> Vec<&str> {
            rendered
                .lines()
                .filter(|line| line.starts_with("topgun_reclamation_"))
                .collect()
        }

        /// The count is frozen at thirteen, so both a silent addition and a silent deletion fail
        /// here rather than at a downstream dashboard.
        fn assert_exactly_thirteen_series(rendered: &str) {
            let lines = reclamation_value_lines(rendered);
            assert_eq!(
                lines.len(),
                13,
                "exactly thirteen reclamation series must render; got {lines:#?}\n\
                 render was:\n{rendered}"
            );
            assert!(
                rendered_value(rendered, DELETED_QUEUE_SERIES).is_none(),
                "{DELETED_QUEUE_SERIES} was deleted with the queue and must not render; \
                 render was:\n{rendered}"
            );
        }

        /// Eager registration: constructing the registry is enough for all thirteen series to
        /// render, so a sampler never has to tell *"has not moved"* from *"does not exist"*.
        #[test]
        fn every_pinned_reclamation_series_is_present_in_the_first_render() {
            let recorder = PrometheusBuilder::new().build_recorder();
            let handle = recorder.handle();
            // Bind the recorder BEFORE resolving a single handle. A handle resolved first binds
            // to a no-op for its whole lifetime, and the inverted order would quietly turn this
            // into an assertion about nothing.
            metrics::with_local_recorder(&recorder, || {
                let _registry = ReclamationRegistry::with_margin(0, 0);
                // Deliberately nothing driven.
            });

            let rendered = handle.render();
            println!("--- FIRST SCRAPE (no claims, margin 0) ---\n{rendered}");

            for name in RECLAMATION_GAUGE_NAMES {
                assert_series(&rendered, name, "0");
            }
            for name in RECLAMATION_COUNTER_NAMES {
                assert_series(&rendered, name, "0");
            }
            assert_exactly_thirteen_series(&rendered);
        }

        /// The scripted sequence, read off the exporter's own text rather than off an in-process
        /// accessor — an accessor would prove the field moved, not that a consumer can see it.
        ///
        /// Three things the sequence exists to exhibit:
        /// (i) a laggard lowering the proposal, so the ceiling is observed FALLING while the
        ///     executed watermark beside it does not;
        /// (ii) one refusal below the executed watermark, so the skip counter is non-zero;
        /// (iii) the sweep leg rendered MID-SWEEP, while the token is still held, with a second
        ///     `begin_sweep` observed to return `None` on the transport rather than only
        ///     in-process.
        #[test]
        fn the_scripted_sequence_renders_every_series_with_its_predicted_value() {
            let recorder = PrometheusBuilder::new().build_recorder();
            let handle = recorder.handle();

            metrics::with_local_recorder(&recorder, || {
                // A non-default margin, so the rendered value discriminates the effective setting
                // instead of agreeing with the compiled-in default by coincidence.
                let registry = ReclamationRegistry::with_margin(0, 3);

                let first = handle.render();
                println!("--- SCRIPT 0: construction only ---\n{first}");
                assert_series(&first, METRIC_RECLAMATION_MARGIN_EPOCHS, "3");
                assert_exactly_thirteen_series(&first);

                assert_eq!(
                    registry.register_claim(&claimant("ahead"), ClaimScope::Global, 20),
                    ClaimAdmission::Honoured { claim: 20 }
                );
                let high_ceiling = registry.prune_ceiling(ClaimScope::Global);
                assert_eq!(high_ceiling, 17);

                let armed = handle.render();
                println!("--- SCRIPT 1: one claim at 20, ceiling queried ---\n{armed}");
                assert_series(&armed, METRIC_RECLAMATION_PRUNE_CEILING, "17");
                assert_series(&armed, METRIC_RECLAMATION_MIN_LIVE_CLAIM, "20");
                assert_series(&armed, METRIC_RECLAMATION_LIVE_CLAIMS, "1");
                assert_series(&armed, METRIC_RECLAMATION_CLAIMS_REGISTERED_TOTAL, "1");
                assert_series(&armed, METRIC_RECLAMATION_CEILING_QUERIES_TOTAL, "1");
                assert_series(&armed, METRIC_RECLAMATION_CEILING_PINNED_QUERIES_TOTAL, "1");
                assert_series(&armed, METRIC_RECLAMATION_EXECUTED_WATERMARK, "0");
                assert_series(&armed, METRIC_RECLAMATION_SWEEP_IN_PROGRESS, "0");

                let token = registry
                    .begin_sweep()
                    .expect("the first sweep on an idle registry is granted");

                let mid = handle.render();
                println!("--- SCRIPT 2: MID-SWEEP, token still held ---\n{mid}");
                assert_series(&mid, METRIC_RECLAMATION_SWEEP_IN_PROGRESS, "1");
                assert_series(&mid, METRIC_RECLAMATION_SWEEPS_TOTAL, "0");

                assert!(
                    registry.begin_sweep().is_none(),
                    "a second begin_sweep with a token outstanding must be refused"
                );

                let refused = handle.render();
                println!("--- SCRIPT 3: after the refused second begin_sweep ---\n{refused}");
                // The refused call started no sweep, so it neither counts nor re-arms the gauge.
                assert_series(&refused, METRIC_RECLAMATION_SWEEP_IN_PROGRESS, "1");
                assert_series(&refused, METRIC_RECLAMATION_SWEEPS_TOTAL, "0");

                registry.end_sweep(token, 100);

                let swept = handle.render();
                println!("--- SCRIPT 4: sweep ended, watermark advanced ---\n{swept}");
                assert_series(&swept, METRIC_RECLAMATION_SWEEP_IN_PROGRESS, "0");
                assert_series(&swept, METRIC_RECLAMATION_SWEEPS_TOTAL, "1");
                assert_series(&swept, METRIC_RECLAMATION_EXECUTED_WATERMARK, "17");
                assert_series(
                    &swept,
                    METRIC_RECLAMATION_EXECUTED_EPOCHS_ADVANCED_TOTAL,
                    "17",
                );

                // A claimant strictly below already-reclaimed content is refused, and the refusal
                // is what the skip counter counts.
                assert_eq!(
                    registry.register_claim(&claimant("laggard"), ClaimScope::Global, 10),
                    ClaimAdmission::BelowExecuted {
                        claim: 10,
                        executed: 17
                    }
                );

                // A laggard ABOVE the watermark is recorded and LOWERS the proposal.
                assert_eq!(
                    registry.register_claim(&claimant("behind"), ClaimScope::Global, 18),
                    ClaimAdmission::Honoured { claim: 18 }
                );
                assert_eq!(
                    registry.register_claim(&claimant("spare"), ClaimScope::Global, 25),
                    ClaimAdmission::Honoured { claim: 25 }
                );
                // Released above the minimum, so the release counter moves without disturbing the
                // minimum this render also asserts.
                registry.release_claim(&claimant("spare"), ClaimScope::Global);

                let low_ceiling = registry.prune_ceiling(ClaimScope::Global);
                assert!(
                    low_ceiling < high_ceiling,
                    "the proposal must be observed falling: {low_ceiling} !< {high_ceiling}"
                );

                let last = handle.render();
                println!("--- SCRIPT 5: proposal fallen, watermark held ---\n{last}");
                assert_series(&last, METRIC_RECLAMATION_PRUNE_CEILING, "15");
                // Fell from 17 to 15 while THIS did not move: the two-phase split, on the wire.
                assert_series(&last, METRIC_RECLAMATION_EXECUTED_WATERMARK, "17");
                assert_series(&last, METRIC_RECLAMATION_MIN_LIVE_CLAIM, "18");
                assert_series(&last, METRIC_RECLAMATION_LIVE_CLAIMS, "2");
                assert_series(&last, METRIC_RECLAMATION_MARGIN_EPOCHS, "3");
                assert_series(&last, METRIC_RECLAMATION_SWEEP_IN_PROGRESS, "0");
                assert_series(&last, METRIC_RECLAMATION_CLAIMS_REGISTERED_TOTAL, "3");
                assert_series(&last, METRIC_RECLAMATION_CLAIMS_RELEASED_TOTAL, "1");
                assert_series(
                    &last,
                    METRIC_RECLAMATION_CLAIMS_REJECTED_BELOW_EXECUTED_TOTAL,
                    "1",
                );
                assert_series(&last, METRIC_RECLAMATION_CEILING_QUERIES_TOTAL, "2");
                assert_series(&last, METRIC_RECLAMATION_CEILING_PINNED_QUERIES_TOTAL, "2");
                assert_series(
                    &last,
                    METRIC_RECLAMATION_EXECUTED_EPOCHS_ADVANCED_TOTAL,
                    "17",
                );
                assert_series(&last, METRIC_RECLAMATION_SWEEPS_TOTAL, "1");
                assert_exactly_thirteen_series(&last);
            });
        }
    }
}
