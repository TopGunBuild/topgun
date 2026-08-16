# SPEC-359 — cross-vendor adjudication of the P9 / N1 / N2 fork

Date: 2026-08-16
Model: z-ai/glm-5.2 (OpenRouter), OPENROUTER_STYLE=full
Harness: ~/Projects/agent-future/scripts/openrouter-ask.sh
Tree: HEAD = cc382fd8 (spec not yet executed)
Round: SPEC-359 Audit v2 -> Response v2 (round 3 of the cap)

Status: ADVISORY. Claude remains the source of truth; this record exists because
the round's ruling was adjudicated cross-vendor and the pin requires the raw
output to ship beside the round's evidence artifacts.

## Prompt (verbatim)

```
Adjudicate a normative design fork in a Rust CRDT server. Answer as a distributed-systems
reviewer. Be concrete; where you disagree, give the counterexample.

## Setting

We are adding a `ReclamationRegistry`: a registration-and-min-reduction protocol that decides how
far an OR-Map tombstone GC ("prune sweep") may reclaim. Clients register a CLAIM = the epoch
cursor they still need. The registry publishes a boundary; a sweep reclaims strictly below it.
Nothing may be reclaimed at or above a live claim, or a client resurrects deleted data.

The current (audited) model is TWO-PHASE:

- Phase 1, ceiling proposal, recomputed per query, MAY FALL:
    min_live_claim(Global)       = min over ALL recorded live claims in EVERY scope
    min_live_claim(Partition(p)) = min over claims in Partition(p) union Global
    prune_ceiling(S) = min_live_claim(S) - margin, or boot_floor if no claim
  Querying it also updates max_published(S) := max(max_published(S), prune_ceiling(S)).
- Phase 2, executed watermark, MONOTONE, never falls:
    note_executed(S, b): executed(S) := max(executed(S), min(b, max_published(S)))
  The consumer calls this after a sweep with the boundary it actually reclaimed at.
- Admission (N2): register_claim(claimant, S, claim): if claim < executed(S) -> REFUSE
  ("BelowExecuted" = resync fence, content is physically gone). Otherwise RECORD
  unconditionally, even far below the current ceiling (a laggard rejoining the fold must be
  recorded, not evicted; the boundary is the fleet MIN, not a first-claimant latch).

## The defect an audit found

A proptest property `executed(S) <= min_live_claim(S)` (whenever claims exist) is FALSE.
Two counterexample families:

(A) stale max_published, single scope, margin 0:
    register(c1, Global, 10); query(Global) -> max_published(Global)=10; release(c1, Global);
    register(c2, Global, 3)  [admitted: 3 >= executed=0];
    note_executed(Global, 10) -> executed = min(10, max_published=10) = 10.
    Now min_live_claim(Global)=3 < executed=10. The clamp is against a HISTORICAL high-water
    of past proposals, not the CURRENT min.

(B) cross-scope admission hole:
    register(c1, Global, 10); query(Global); note_executed(Global,10) -> executed(Global)=10;
    register(c2, Partition(0), 3) -- admitted, because N2 checks executed(Partition(0))=0.
    But min_live_claim(Global) is defined over ALL scopes, so it is now 3 < executed(Global)=10.
    Worse than a red property: c2 is RECORDED and believes it is protected while the content it
    needs is already reclaimed.

The audit offered three shapes: (a) clamp note_executed to the CURRENT min instead of
max_published; (b) make N2 check the max executed over every scope whose min the claim joins;
(c) narrow the registry to a single scope (Global) for this increment and route per-partition to
the tracker.

## The ruling we intend to adopt — critique it

RULING 1 — BLOCK-DURING-SWEEP. Delete `max_published` from the model entirely. Instead:
  (i)   at sweep START the registry takes its lock and derives
        ceiling := min(live claims) - margin from the CURRENT claim set;
  (ii)  admissions are BLOCKED (or queued) for the duration of the sweep --
        `sweep_in_progress` is registry STATE, not a convention;
  (iii) the registry records `executed` ITSELF when the sweep completes; the executor reports
        success/failure only, never a number. So there is no caller-supplied boundary to clamp,
        and family (A)'s stale-proposal state does not exist;
  (iv)  after the sweep the admission boundary is EXECUTED: a claim below executed is refused ->
        resync fence.
  Restated property: executed(S) <= min_live_claim(S) AT THE SWEEP-START SNAPSHOT, and no claim
  is admitted below executed. The proptest generator gains concurrent-arrival-during-sweep
  interleavings.

RULING 2 — SINGLE SCOPE. The frozen surface is Global-only, and says so explicitly ("multi-scope
admission is NOT solved by this surface" is a sentence in the frozen contract, not an omission).
`ClaimScope::Partition`, the cross-scope admission rule, and a lease/proposal model for long or
distributed sweeps are routed to the tracker as NAMED work items, to become live when a consumer
exists to witness them. Family (B) dies with the scope.

Context that may matter: the sweep here is in-process, single-node, holds a frontier lock already,
and is short (it drains a bounded per-epoch tombstone index). The registry lock is a leaf lock;
lock order is frontier -> registry. The margin is fixed at 0 in this increment, and a separate
byte-identity witness fences the prune's decisions to be identical pre/post change.

## Questions

1. Does BLOCK-DURING-SWEEP actually close family (A)? Is there any residual interleaving where
   `executed` can end up above the current min? Consider: claim released DURING the sweep; claim
   registered at sweep-start-minus-epsilon; sweep fails partway; two sweeps racing.
2. Is "block admissions during the sweep" sound, or does it trade a correctness bug for a
   liveness/latency bug? The blocked path is a client ACK on the websocket read loop. Would
   QUEUEING be materially better than BLOCKING, or does queueing reintroduce the same race at the
   dequeue point?
3. Is deriving `executed` inside the registry (executor reports success/failure only) actually
   stronger than clamping a caller-supplied number, or does it just relocate the trust? Note the
   real consumer's boundary is min(ceiling, durable_watermark + 1) -- the sweep can be held back
   by a durability fence BELOW the ceiling, so the registry cannot know what was reclaimed
   without being told something. Does that break ruling (iii)? If so, what is the minimal honest
   fix that preserves its spirit?
4. Is single-scope-with-an-explicit-non-guarantee better engineering than shipping the
   per-partition shape with the cross-scope admission rule (audit option b)? We prefer not to
   freeze a surface no consumer witnesses.
5. What property set would you proptest for this model? Name the properties that would have
   caught (A) and (B), and any property the restated one still misses.
6. Anything else that is actually wrong here.
```

## Response (verbatim, unedited)

→ openrouter z-ai/glm-5.2 answering (0 file(s) embedded)…

[usage] prompt=1599 completion=15550 total=17149 cost=$0.03884624
## 1. Does BLOCK-DURING-SWEEP close family (A)? Residual interleavings.

**Family (A) is closed**, but only because you deleted `max_published` *and* you derive `executed` from the sweep-start snapshot. The original defect had two ingredients: (1) a clamp target (`max_published`) that was a historical high-water, not the current min, and (2) an admission rule that admitted a below-water-mark claim because `executed` had not yet caught up. The ruling removes ingredient (1) by construction. Ingredient (2) is handled by the admission fence in (iv). So the counterexample as literally stated cannot be reconstructed.

But there are residual issues. I'll work each interleaving you named:

### (a) Claim released DURING the sweep

At sweep start, claims = {c1: 10}, ceiling = 10, snapshot taken under lock. During the sweep, c1 is released. The sweep reclaims strictly below 10. After the sweep, `executed = max(executed, …)` = 10. The claim set is now empty. `min_live_claim` over an empty set is undefined/vacuous — the property "whenever claims exist" does not fire. A subsequent registration of claim 3 hits the fence: 3 < executed = 10 → REFUSE. Correct.

If instead claims = {c1: 10, c2: 3} at sweep start, ceiling = 3, and c2 is released during the sweep: the sweep reclaimed below 3, `executed = 3`, and `min_live_claim = 10 > 3`. The invariant holds. Releases during the sweep are always safe because they can only *raise* the min, and the ceiling was already bounded by the pre-release min.

**No residual here.**

### (b) Claim registered at sweep-start-minus-ε

This reduces to: is the snapshot atomic with respect to admission? If the registry acquires the lock, reads the claim set, and sets `sweep_in_progress` all under the same lock acquisition — and admission takes the same lock — then "minus-ε" means "before the lock," which means the claim is in the snapshot. There is no gap. The claim is accounted for in the ceiling.

If instead `sweep_in_progress` is a flag checked *outside* the lock (a "convention"), then there is a TOCTOU window: the sweep reads claims, sets the flag, but a registration races between the read and the flag-set. The ruling explicitly says "sweep_in_progress is registry STATE, not a convention," which I read as: it lives inside the lock. **This is the load-bearing sentence. If the implementation checks the flag without holding the lock, family (A) reappears in a new costume.**

### (c) Sweep fails partway

This is where I have a real concern. The ruling says the executor reports success/failure only, never a number, and the registry records `executed` itself. Two sub-cases:

**Total failure (nothing reclaimed):** The registry should not advance `executed`. Fine — but the ruling must say this explicitly. "Records `executed` when the sweep completes" is ambiguous about failure. On failure, `executed` must be `max(executed, 0)` = unchanged, *not* `executed = ceiling`. If the implementation sets `executed = ceiling` unconditionally on "completion" (including failure), and the sweep reclaimed nothing, then claims below ceiling are refused against content that still exists. Safe but lossy — you fence clients into resyncs for no reason.

**Partial failure (some tombstones reclaimed, then error):** This is the dangerous case. Some content below the partial boundary is physically gone. If the registry does not advance `executed` (because the sweep "failed"), a subsequent claim at an epoch in the reclaimed-but-unrecorded range is *admitted* — it passes the fence because `executed` didn't move — but the content is gone. **This is a correctness bug, the same class as family (B): a client believes it is protected while content is physically absent.**

Your context says the sweep "drains a bounded per-epoch tombstone index" and a "byte-identity witness fences the prune's decisions to be identical pre/post change." If the drain is *not* atomic — if it removes tombstones incrementally and can fail mid-drain — then partial failure is real, and ruling (iii) as stated is unsound. The fix is one of:

- Make the drain atomic (all-or-nothing per sweep). Then failure = total failure, and the above is fine.
- On *any* failure, conservatively set `executed = max(executed, ceiling)`. This over-approximates what was reclaimed, which is safe (excess resyncs, no resurrection), and relies on the next sweep being idempotent to finish the job. The byte-identity witness makes this viable.
- Have the executor report the actual partial boundary on failure. This contradicts (iii)'s "never a number" and is the honest fix I recommend in Q3.

### (d) Two sweeps racing

If only one sweep can be in progress (enforced by `sweep_in_progress` under the lock), then the second sweep trigger blocks until the first completes. After the first sweep, `executed` is advanced. The second sweep snapshots claims, computes a new ceiling. Because admissions were blocked during the first sweep (and the second doesn't start until the first completes), any claim admitted between the two sweeps satisfied `claim ≥ executed(after sweep 1)`. So the new ceiling = min(live claims) − margin ≥ executed − margin. With margin 0, ceiling ≥ executed. The second sweep can only advance. **Monotone, no residual.**

**Critical caveat:** `executed` must be updated with `max`, not overwrite. If the claim set is empty at sweep 2 (all claims released), `ceiling = boot_floor`. If `boot_floor < executed(after sweep 1)` and the registry *overwrites* `executed = boot_floor`, then `executed` goes backward. A subsequent claim at an epoch between boot_floor and the old executed is admitted, but content is gone. **This is a correctness bug.** The original model had `max(executed, …)` explicitly. The ruling's prose ("the registry records `executed` ITSELF") does not mention `max`. This must be pinned: `executed := max(executed, clamp(actual_reclaimed, ceiling))`. If you overwrite, you reintroduce a resurrection bug via the empty-claim → boot_floor path.

### Verdict on Q1

Family (A) is closed *if and only if*:
1. The sweep-start snapshot and `sweep_in_progress` are set atomically under the registry lock.
2. `executed` is updated with `max(executed, …)`, never overwritten.
3. Partial sweep failure is handled (atomic drain, or conservative `executed = ceiling` on any failure, or executor reports the partial boundary).

The ruling as written satisfies (1) by intent but does not state (2) or (3). Both are silent bugs.

---

## 2. Block vs. queue vs. liveness

**Blocking is correct but is a liveness trade.** The question is whether it's an acceptable one.

The sweep is in-process, single-node, short (drains a bounded index). The registry lock is a leaf lock. If the sweep holds the lock for the full duration, every admission across all connections blocks for the sweep's wall-clock time. If that's sub-millisecond, this is fine for a CRDT server. If the index drain touches disk or does nontrivial work, "short" can spike.

The more serious concern: the admission path is on the websocket *read loop*. If blocking means the read loop can't process *any* frames during the sweep — not just admission frames, but CRDT operation frames too — then a sweep stalls live CRDT replication for all connections whose admission is pending. That's a head-of-line block. In an async Rust model, each connection typically has its own task, so a blocked admission on connection A doesn't block connection B's read loop. But if admission is the first thing a reconnecting client does, the client can't proceed until the sweep finishes.

**Is queueing materially better?** Yes, *if the read loop must continue processing non-admission frames during the sweep.* Queueing lets the read loop accept and process CRDT ops while the admission is deferred. The client gets its ACK after the sweep.

**Does queueing reintroduce the race at dequeue?** No, *if and only if* the dequeue re-checks against `executed`. The queue stores pending registration requests; after the sweep completes and `executed` is advanced, the queue is drained with each request checked: `if claim < executed → REFUSE, else RECORD`. This is identical to blocking — the client waits either way — but the read loop is not stalled. The race only reappears if queued requests are *grandfathered* (admitted without re-checking, on the theory that they were submitted "before" the sweep). That would be wrong. Do not do that.

**The deeper issue you're not asking about:** blocking vs. queueing is the wrong frame. The real race is between a client *deciding* it needs an epoch and the registry *learning* about it. If a client reconnects and needs epoch 3, but hasn't registered yet, and the sweep starts and reclaims epoch 3, the client's subsequent registration is refused and it must resync. Blocking/queueing the *registration* doesn't help — the client hadn't registered yet. The only thing that helps is the client registering *before* it needs the content (proactive registration), or the sweep being conservative enough to not reclaim epochs any connected client might need. This is a protocol-level question about claim registration timing, and the ruling doesn't address it. The resync fence is the correct fallback, but it's a fallback, not a prevention.

**Recommendation:** Queue admissions during the sweep (set `sweep_in_progress`, let admission requests enqueue, drain the queue after the sweep with re-check against `executed`). Don't hold the registry lock for the full sweep duration — hold it only at start (snapshot + set flag) and end (clear flag + advance `executed` + drain queue). This keeps the read loop responsive and the sweep short. Blocking is acceptable as a first cut if the sweep is truly sub-millisecond, but queueing is strictly better and costs little.

---

## 3. Deriving `executed` inside the registry vs. clamping a caller-supplied number

**Ruling (iii) is broken by the durability fence, and it doesn't just relocate trust — it *misplaces* it.**

The real reclaim boundary is `min(ceiling, durable_watermark + 1)`. The sweep can be held below the ceiling by a durability fence. If the registry sets `executed = ceiling` unconditionally (because the executor "reports success/failure only, never a number"), then `executed` overstates what was reclaimed. Consequences:

- Claims with `durable_watermark + 1 ≤ claim < ceiling` are refused — the fence fires — but the content at those epochs still physically exists (the sweep was held back). The client is forced into an unnecessary resync. **Safe but lossy.**
- This is not a correctness violation (no resurrection), but it's a *false fence*. You're telling clients to resync when they don't need to. In a CRDT server, unnecessary resyncs are expensive — they're the whole reason the registry exists.

So (iii) does not "derive `executed` inside the registry" in any meaningful sense — it *sets `executed` to the ceiling*, which is an upper bound on what was reclaimed, not the actual value. The trust hasn't been removed; it's been replaced with an over-approximation. That's fine for safety but not for honesty.

**Does the executor reporting a number reintroduce the original bug?** No — *if* the clamp target is the current sweep-start ceiling, not `max_published`. Family (A) existed because the clamp was against a *historical* high-water. Audit option (a) — clamp `note_executed` against the current min — fixes exactly that. The ruling's (iii) goes further by eliminating the number entirely, but the further step doesn't buy correctness; it buys a false sense of "no caller-supplied input." The caller (executor) is in-process, trusted code. The original problem was never that the executor lied — it was that the clamp target was stale.

**Minimal honest fix that preserves the spirit of (iii):**

```
// At sweep start (under lock):
ceiling = min(live_claims) - margin   // snapshot
sweep_in_progress = true
release lock

// Sweep runs, reclaims up to:
actual_b = min(ceiling, durable_watermark + 1)

// At sweep end (under lock):
sweep_in_progress = false
executed = max(executed, min(actual_b, ceiling))
// Drain admission queue, checking against executed
```

The executor reports `actual_b` (a number). The registry clamps it against `ceiling` (the sweep-start snapshot, *not* a historical max). The executor cannot lie *above* the ceiling (clamped). The executor *can* lie below, but that makes `executed` lower, which is safe *only if the executor is trusted* — and it is, it's in-process. If you don't trust the executor, you're in a different world (out-of-process GC, distributed sweep), and you need a lease/proposal model, which is exactly the work item you're deferring.

This is audit option (a) with the current-min fix. It closes family (A) because the clamp target is the current snapshot, not `max_published`. It preserves the spirit of (iii) (the registry doesn't blindly trust a caller-supplied number — it clamps) while being honest about what was actually reclaimed.

**The ruling's (iii) as written ("executor reports success/failure only, never a number") is wrong for any deployment where `durable_watermark + 1 < ceiling` can happen.** Your context says the sweep can be "held back by a durability fence below the ceiling," so this is not hypothetical. Either accept the lossy over-approximation (document it: "executed is an upper bound on reclamation; clients may be asked to resync unnecessarily when the durability fence holds the sweep below the ceiling") or accept the executor's number and clamp it.

---

## 4. Single-scope-with-explicit-non-guarantee vs. per-partition with cross-scope admission (option b)

**Single-scope is the right call for this increment.** Here's the reasoning, not just YAGNI:

Option (b) — "check the max executed over every scope whose min the claim joins" — is *correct in principle* but has a combinatorial correctness hazard. `min_live_claim(Global)` is defined over all scopes. So a claim in `Partition(p)` affects `min_live_claim(Global)`. Therefore admission in `Partition(p)` must check `max(executed(Global), executed(Partition(p)))`. But does a `Partition(p)` claim also affect `min_live_claim(Partition(q))`? If partition mins are *independent* (partition min only includes claims in that partition), then no. If partition mins are *hierarchical* (partition p's min includes sub-partitions), then yes, and the admission check is a tree traversal. The "every scope whose min the claim joins" set is only statically knowable if the scope lattice is fixed. If partitions are dynamic (created at runtime), the set is dynamic, and the admission rule becomes a graph query on the hot path. This is a complexity bomb disguised as a one-line fix.

The audit found family (B) — a cross-scope hole — which proves the multi-scope surface is *already* buggy in subtle ways. Shipping it without a consumer means shipping a buggy, untested-by-use API that you'll have to fix later anyway. The frozen contract saying "multi-scope admission is NOT solved by this surface" is more honest than shipping option (b) and pretending it's solved.

**The risk of single-scope:** when a consumer for per-partition arrives, the Global-only API may not extend cleanly. Specifically, the tracker (which handles per-partition claims) must coordinate with the Global registry — if a per-partition claim is recorded in the tracker but the Global registry's sweep doesn't consult the tracker, family (B) reappears in a different form. The non-guarantee must be enforced *operationally*, not just documented. The frozen contract should say: "the Global registry does not consult any per-partition tracker; if a per-partition tracker exists, it MUST fence the Global sweep, or the Global sweep may reclaim content a per-partition claim needs." This is the sentence that prevents the bug from migrating.

**Counter-argument for option (b):** if you *know* per-partition is coming soon and the scope lattice is fixed and flat (Global + independent partitions, no nesting), then option (b) is a small, correct addition: `admission_check(S, claim) = claim ≥ max(executed(Global), executed(S))` for any S. That's two lookups. If you're confident the lattice stays flat, ship it. But the audit just proved you're not confident about multi-scope. So don't.

**Verdict:** single-scope with the explicit non-guarantee, plus the operational fence sentence above. Route per-partition to the tracker. Ship when a consumer witnesses it.

---

## 5. Proptest property set

### Properties that would have caught (A) and (B)

**Catches (A):**

- **P-exec-below-current-min:** For any scope S, after any operation sequence, if live claims exist in S, then `executed(S) ≤ min_live_claim(S)`. This is the original property. It catches (A) directly: after `note_executed(Global, 10)` with `min_live_claim = 3`, the property fires.

- **P-clamp-target-is-current:** After `note_executed(S, b)`, `executed(S) ≤ min_live_claim(S)` evaluated at the *time of the note_executed call*, not at the time of any prior query. This would catch the stale-`max_published` mechanism specifically, because it forces the model to reason about the current claim set, not a cached high-water.

**Catches (B):**

- **P-cross-scope-admission:** After `register_claim(c, S, claim)` succeeds (returns ADMITTED, not REFUSED), for every scope S' such that `min_live_claim(S')` includes claims from S, `claim ≥ executed(S')`. In the buggy model, `register(c2, Partition(0), 3)` succeeds, but `min_live_claim(Global)` includes Partition(0) claims, and `executed(Global) = 10 > 3`. Property fires.

- **P-no-admit-below-any-executed:** No claim is ever recorded with `claim < max(executed(S')) for all S' in the scope lattice that intersect S`. This is the generalized admission fence. In single-scope, it reduces to `claim ≥ executed(Global)`.

### Properties for the new (block-during-sweep, single-scope) model

- **P1-exec-below-min-always:** `executed(Global) ≤ min_live_claim(Global)` whenever claims exist, after any interleaving including concurrent arrivals during sweep. This subsumes the original property. It should *always* hold in the new model (not just at sweep-start snapshot), because admissions are blocked and releases only raise the min.

- **P2-exec-monotone:** `executed` never decreases across any operation. Catches the missing-`max` bug (overwrite vs. max). Generator must include sequences with empty claim sets (boot_floor path) and sweep failures.

- **P3-admission-fence:** `register_claim(c, Global, claim)` returns REFUSED iff `claim < executed(Global)`. No claim below `executed` is ever recorded. Catches any dequeue/queue grandfathering bug.

- **P4-sweep-boundary-below-ceiling:** The actual reclaim boundary ≤ ceiling at sweep start. Catches a sweep that reclaims above its authorized ceiling. (This is enforced by the byte-identity witness, but the proptest should verify the registry's `executed` is consistent with it.)

- **P5-concurrent-admission-during-sweep:** Generate interleavings where `register_claim` arrives during a sweep. Assert: the registration is either blocked/queued and resolved after the sweep, or refused. It is never silently recorded with `claim < executed(after sweep)`. Catches the flag-without-lock TOCTOU.

- **P6-no-resurrection:** After any sweep, for every live claim `c` with `claim = E`, no tombstone at epoch ≥ E was reclaimed. This is the end-to-end safety property. It's the one that actually matters. The others are structural proxies.

### What the restated model still misses (properties that would fail)

- **P-exec-equals-actual-reclaimed:** `executed = actual_reclaimed_boundary` after a successful sweep. This would FAIL under ruling (iii) whenever `durable_watermark + 1 < ceiling`, because `executed = ceiling > actual_b`. The property as stated is too strong (you don't need exact equality, you need `executed ≥ actual_b` for safety and `executed ≤ actual_b` for no-false-fence). Split it:

  - **P6a-exec-ge-actual:** `executed ≥ actual_reclaimed` (safety: no admission into reclaimed range). This holds under (iii).
  - **P6b-exec-le-actual:** `executed ≤ actual_reclaimed` (no false fence). This FAILS under (iii) when the durability fence holds. This is the property that exposes the lossy over-approximation.

- **P-partial-failure-fence:** After a partial sweep failure, `executed ≥ max partial boundary reclaimed`. This would FAIL if the registry doesn't advance `executed` on failure and the sweep partially reclaimed. The generator must include partial-failure interleavings.

---

## 6. Anything else wrong

**(a) The restated property is weaker than what the model achieves.** You wrote: "executed(S) ≤ min_live_claim(S) AT THE SWEEP-START SNAPSHOT." But in the new model, admissions are blocked during the sweep and releases only raise the min, so after the sweep, `min_live_claim(current) ≥ min_live_claim(sweep-start) ≥ executed`. The original property (`executed ≤ min_live_claim` whenever claims exist) actually *still holds* in the new model. By restating it weaker (at-snapshot), you're under-claiming your own guarantee and leaving room for a future change to break the stronger property without a property firing. State the stronger property and prove it.

**(b) Claim lifecycle is unspecified.** How are claims released? If a client disconnects, is the claim released immediately? Is there a lease? If a client crashes without releasing, the claim persists forever, and `min_live_claim` is stuck at that client's epoch. The sweep can never advance past it. This is a liveness deadlock. The ruling doesn't address claim expiry/leases. If there's no lease mechanism, a single crashed client permanently prevents tombstone GC. This is probably out of scope for this increment, but it should be flagged as a known limitation in the frozen contract, not silently ignored.

**(c) `boot_floor` must be monotone across restarts.** When no claims exist, `ceiling = boot_floor`. If the server restarts and `boot_floor` regresses (e.g., it's derived from local state that was GC'd), a post-restart sweep could reclaim below a previous `executed`, and without the `max`, `executed` would regress. Even with `max`, `boot_floor` should be the highest epoch the server knows it has safely processed, not a static constant. Flag this.

**(d) The sweep-start snapshot must include claims that were registered but not yet durable.** If claim registration writes to a durable store and the sweep reads from memory, a crash between registration and durability loses the claim. On restart, the sweep could reclaim content the (now-lost) claim protected. This is a durability-ordering issue: claims must be durable before the sweep can use them, and the sweep must read the durable claim set. If the registry is in-memory only (no persistence), then claims are lost on restart and `executed` must reset to a safe floor (e.g., 0 or boot_floor), accepting that post-restart clients must resync. This is fine but must be a conscious decision.

**(e) The lock order (frontier → registry) is correct only if the sweep doesn't take the registry lock while holding the frontier lock in a code path that also takes them in the other order elsewhere.** The context says the sweep "holds a frontier lock already" and the registry lock is a leaf. So the sweep takes frontier, then registry. Admission takes registry only (no frontier). This is consistent. But if any path takes registry then frontier, you have a deadlock. Audit the admission path — does it ever need the frontier lock? If admission triggers a sweep (e.g., "register and then sweep if needed"), and the sweep takes frontier → registry, but admission holds registry → tries to take frontier, that's deadlock. Make sure admission never takes the frontier lock.

**(f) "Sweep fails partway" is not just a Q3 issue — it's a Q1 issue.** I covered this above, but to be blunt: ruling (iii)'s "executor reports success/failure only" is unsound if the sweep can partially reclaim and then fail. The partial reclaim is physical state change that `executed` doesn't track. The byte-identity witness helps make the sweep idempotent (re-running finishes the job), but between the partial failure and the re-run, `executed` is stale. If a claim is admitted in that window (the sweep is not in progress, so admissions are not blocked), it can be admitted into the partially-reclaimed range. Fix: either make the drain atomic, or conservatively advance `executed = max(executed, ceiling)` on *any* failure (accept the false-fence cost), or have the executor report the partial boundary.

**(g) You should explicitly state what `executed` means after a restart.** If the registry is in-memory, `executed` resets to 0 (or boot_floor) on restart. The tombstone index may have been partially drained before the crash. Post-restart, a sweep could re-derive a ceiling and reclaim, but claims from before the crash are gone. This is fine if the protocol is "reconnect and re-register, accept possible resync." But it must be stated. If the registry is persisted, `executed` must be persisted *after* the sweep's effects are durable, not before — otherwise a crash between persisting `executed` and durably reclaiming tombstones leaves `executed` ahead of reality (safe but lossy) or behind reality (dangerous, if tombstones were reclaimed but not yet persisted as reclaimed). The ordering is: (1) reclaim tombstones durably, (2) persist `executed`. Not the reverse.

---

### Summary of concrete recommendations

1. **Family (A) is closed** — but pin three things in the contract: (i) snapshot + flag under the same lock, (ii) `executed := max(executed, …)` never overwrite, (iii) partial-failure handling (atomic drain, or conservative `executed = ceiling` on any failure, or executor reports partial boundary).

2. **Queue, don't block.** Queue admissions during the sweep, drain with re-check against `executed`. Don't hold the lock for the full sweep. This is strictly better and costs little.

3. **Ruling (iii) is broken by the durability fence.** Accept the executor's number and clamp against the *current sweep-start ceiling* (not `max_published`). This is audit option (a) with the current-min fix. Document `executed` as clamped upper bound. If you refuse to take a number, document the false-fence cost.

4. **Single-scope is correct.** Add the operational fence sentence: "the Global registry does not consult per-partition trackers; a per-partition tracker MUST fence the Global sweep or content may be reclaimed under a per-partition claim."

5. **Proptest P1–P6 above.** Specifically add P6b (`executed ≤ actual_reclaimed`) to expose the false-fence, and P-partial-failure-fence to expose partial-failure unsoundness.

6. **Flag the unspecifieds:** claim lifecycle/leases, `boot_floor` monotonicity, durability ordering of `executed` vs. reclamation, restart semantics, lock-order audit of the admission path.
