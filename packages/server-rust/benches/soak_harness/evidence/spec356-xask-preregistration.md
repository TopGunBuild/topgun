# `/xask` — cross-vendor adversarial review of the SPEC-356 pre-registration

**Round:** R9(a) of SPEC-356a — the **pre-registration** round.
**Run:** 2026-08-03, **before** the freeze boundary and therefore **before any `spec356-*.soak.json`
exists**. Every finding below is consequently **PRE-DATA**: the data cannot have influenced it, and
`git log --follow` proves the ordering.
**Vendor / model:** OpenRouter `z-ai/glm-5.2`, depth mode (`OPENROUTER_STYLE=full`), the committed
`spec356-manifest.md` embedded as the sole context.
**Scope, deliberately family-scoped (`spec356-`, no half-suffix):** the round is run against the
**pre-registration**, which governs **both** halves. Each `/xreview` is scoped to one half's own diff and
is named `spec356a-` / `spec356b-` accordingly.

**Questions put:** (a) the ordered classification predicate (§2) — *is this instrument capable of being
wrong?*; (b) the neutrality-control design and its stated MDE (§1) — *is the control powered to notice?*

**Status of this document.** Advisory. The other vendor has no memory of this project and reasons only
from the embedded manifest. Every finding below was verified against the manifest and the spec before
disposition; the ones that survived are recorded as **applied** (with the `ADJ-<n>` that carries them),
the ones that did not are recorded as **refuted-with-reason**. No finding is left undisposed.

---

## Disposition ledger

| # | Finding (source: `/xask` glm-5.2) | Disposition | Lands as |
|---|---|---|---|
| **X1** | **`min(L)` is the extreme order statistic, so the Step-2 / Step-3-4 partition is wildly asymmetric.** Over a 2 h last half at 10 s cadence (~720 samples), `min(L) ≤ B` fires if the prune catches up **once**; its complement requires it to catch up **never**. Step 2 becomes the default branch for almost any admissible data — and Step 2 is the branch §2.5 routes to the REGISTRY family. | **APPLIED** — and it is the finding that changes the science rather than the prose. Verified: the cadence, the sample count and the complement structure are exactly as stated. The consequence is that §2.5's "the modal outcome lands at Step 2" is partly **manufactured by the aggregator** rather than measured. | **ADJ-2** |
| **X2** | **A concrete misclassification the predicate would make:** the prune drains its licence *completely* on every pass, but passes become rare because a scan cost grows with the corpus, so the eligible set regrows in the widening inter-pass gap. `min(L) ≤ B` fires right after each pass ⇒ **Step 2 ⇒ REGISTRY**, and the slope test that would have caught the cadence collapse is **never evaluated** because Step 2 short-circuits it. The real fix is a data-structure/cadence change, not a licensing model. | **APPLIED.** Verified as reachable under the frozen text. **ADJ-2 repairs it directly**: under a median aggregator this shape has `L > B` for most of the window and correctly falls through to Steps 3–4. **ADJ-3** adds the reporting backstop for the residual case. | **ADJ-2**, **ADJ-3** |
| **X3** | **The upstream-first ordering is vulnerable to feedback.** A downstream cause (per-pass cost growth) propagates upstream and mimics every upstream symptom — fewer passes ⇒ persistent backlog ⇒ LWM does not advance ⇒ pinned pool grows — so the predicate fires on the first upstream symptom and routes to the upstream fix family. | **APPLIED IN PART.** The general claim is sound and is the correct dual of the risk §2.5 already names (a *flat* table would misroute the same shape the other way). The ordering itself is **NOT** reversed — reversing it would re-open exactly the mis-route §2.5 was built to prevent, and the choice of which direction of error to take is the pre-registration's to make. What is applied is the **visibility** obligation: a Step-2 determination must carry the pass-frequency evidence that would have indicted it. | **ADJ-3** |
| **X4** | **The predicate has no residual category for admissible data.** Steps 1–4 exhaustively partition, Step 5 catches only *admissibility* failure. A cause outside the four hypothesised mechanisms still receives one of the four frames, endorsed by a pre-registered rule. | **APPLIED.** Verified: §2.4's exhaustiveness argument is correct and is precisely the property that creates the gap. Not repairable by adding a fifth branch post-hoc (there is no admissible-data condition that could trigger it without judgement), so it is repaired as a **stated bound on what the determination claims**. | **ADJ-4** |
| **X5** | **Both measured controls are LEVEL controls; the classification reads DYNAMICS.** §1.1/§1.2 compare last-half level means of `tombstone_bytes`. The predicate reads exit share, the `L`/`P` split and a slope. A perturbation that shifts dynamics without shifting the steady-state level is invisible to both controls by construction. | **APPLIED.** Verified against §1.1/§1.2 — both are level-mean comparisons; no dynamics statistic is controlled. | **ADJ-5** |
| **X6** | **§1.2 is structurally blind to the instrument's PRESENCE, as opposed to its ACTIVATION.** `ctl` and `ctloff` are the **same binary**; arming is a runtime flag. Compiled-in branches, inlining and layout changes are identical in both arms, so the higher-powered control cannot see them — while §1.1, which *can*, has its rejection absorbed as "build-lineage effect" with the classification numbers allowed to STAND. | **APPLIED.** This is the sharpest finding on (b) and it is structurally undeniable. The 2×2's logic is not overturned — §1.2 remains the better test of *activation* — but the absorbing cell's disposition is tightened from an unqualified "STAND" to "stand, **bounded and flagged**". | **ADJ-5** |
| **X7** | **The 10 s `prune.csv` sampler's own I/O may be a confound between `ctl` and `ctloff`** if the sampler runs on one arm and not the other. The manifest does not say which. | **APPLIED — and it caught a genuine underspecification.** Verified: R4.2b states the completeness gate unconditionally ("all 35 series … else `exit 9`"), while R4.3a.4 states that on a disarmed cell **no** prune-record column is checked. Read together, an unscoped gate would make a disarmed cell **unrunnable**, and the obvious repair (do not sample disarmed cells) would put the sampler's curl I/O on the armed arm only — confounding the one control that carries the neutrality weight. Resolved explicitly, pre-data. | **ADJ-6** |
| **X8** | **The 10 % exit-share threshold is a fixed point with no trend limb** — a small but persistent non-drop exit problem (say a steady 7 %) is invisible to Step 1 and is absorbed by Steps 2–4. | **REFUTED-WITH-REASON (not adopted).** The criticism is fair as methodology but the remedy is not available here: adding a trend limb introduces a **second** slope-dependent test into a predicate whose one existing slope dependence is already the acknowledged fragility (§2.6), and it would need its own threshold chosen with no prior to choose it from. The 10 % literal is **deliberately the same statistic and the same literal §3 freezes**, so moving it would break that binding. The exit ledger names *which* exit regardless of whether Step 1 fires, so a steady 7 % is **visible in the committed series** even when it does not decide — which is the honest bound, and it is now stated by **ADJ-4**. |  — |
| **X9** | **Step 4 is reached by FAILURE TO REJECT**, so a positive determination (THROUGHPUT) rests on absence of evidence; with a low-power slope this biases toward THROUGHPUT and may make Step 3 nearly unreachable. | **REFUTED-WITH-REASON (already discharged, and the direction is confirmed).** The same vendor confirms *"'reject negative' is the right direction"*. §2.6 already binds the determination to state **which steps turned on a level or a count versus on a slope**, which is exactly the disclosure this finding asks for. Converting Step 4 to a positive/equivalence test (TOST) would require a pre-data equivalence margin that no prior supports — inventing one would be worse than the stated caveat. **ADJ-3**'s reporting obligation additionally puts the slope and its test result beside *every* determination, including the Step-2 ones that short-circuit it. |  — |
| **X10** | **OLS assumes linearity; a convex decay would understate the trend** and fail to reject negative. | **REFUTED-WITH-REASON (noted, not adopted).** Real, but the fit mechanism is deliberately **not forked** (R6): the committed `spec349c2-fit.awk` is reused unchanged, and `git diff --stat` on it being empty is an acceptance criterion. Choosing a different fit shape at pre-registration time, with no data to justify the choice, trades a known-linear estimator for an unvalidated one. Recorded here so the next reader does not re-derive it. |  — |
| **X11** | **n = 2 per arm ⇒ MDE ≈ 17 % ⇒ the control is a catastrophe detector, not a neutrality proof.** A table is offered: n = 4 ⇒ ≈ 6.8 %, n = 6 ⇒ ≈ 5.0 %, n = 8 ⇒ ≈ 4.2 %. | **ALREADY DISCHARGED IN THE FROZEN TEXT; the residue is a SPEND decision, not a methods defect.** §1.1 already states, pre-data and in its own words, that the control is *"honestly weak"* and that *"a smaller perturbation is NOT excluded"* — which is precisely this finding, and the vendor explicitly credits that honesty as *"the right call"*. Raising `n` is not a correction that `/xask` can make: each cell is a 4 h run, so n = 6 per arm is ≈ 48 h of additional control time, and that is the user's call on SPEC-356b's scope. **Recorded and surfaced, not silently absorbed and not silently adopted.** The vendor's table is reproduced verbatim below so the decision has its numbers. |  — |
| **X12** | Confirmations, recorded so they are not re-litigated: the **no-α-correction** decision (§1.4) is correct; the **stale-split fail-closed** rule (ADJ-1) is correct; the **RSS forfeiture** (§0) is correct and *"unusually honest"*; the **commit-order pre-registration proof** is sound; the **refusal to OR the two controls** is correct because they test different things; the **quantile-aliasing** `_sum`/`_count`-primary rule (§5) is correct; the **cell-E provenance checks** (§4) are correct. | **CONFIRMED — no action.** Recorded because a pre-registration that is only ever criticised loses the information that parts of it were independently checked and held. |  — |

**Net effect on the pre-registration:** five addenda (**ADJ-2 … ADJ-6**), all **PRE-DATA**, all appended to
§8A. **No byte of §0–§8 is edited** — that is the mechanism working as designed: the frozen text stands in
the record as what was committed, and the adjudicated form governs.

**One item is escalated to the user rather than adjudicated (X11):** whether SPEC-356b buys more control
replicates. It is a scope/spend question, not a methods question, and the frozen text already states the
honest bound either way.

---

## The vendor's answer, verbatim

*(Reproduced in full and unedited. Where this document's dispositions disagree with it, the disposition
ledger above is what governs — this section is the record of what was said, not of what was accepted.)*

<!-- BEGIN VERBATIM /xask RESPONSE — z-ai/glm-5.2 -->

Some determination **always fires when the data is admissible**. Steps 1–4 are an exhaustive partition by construction (§2.4 is correct on this point), and Step 5 only triggers on *admissibility failure* — stale split, unreadable columns, or a failed control 2×2. There is no "the data is admissible but none of the four causal stories fit" branch. Step 5 is a data-quality gate, not a conceptual escape hatch.

This means the predicate is **not self-falsifying**. It cannot tell you from its own data that it is wrong. It can only be proven wrong downstream, when the fix shape it routes to fails to fix the problem. By that point you have spent engineering effort on the wrong hypothesis, and — worse — you have a pre-registered determination that was *endorsed* by the frozen rule, making it harder to challenge.

The manifest's authors seem to believe that exhaustiveness is a virtue ("every leaf is reached by exactly one path"). It is — for *classification*. But for *diagnosis*, exhaustiveness without a residual category means the predicate will impose one of four frames on any admissible data, including data generated by a mechanism the designers did not think of. The INDETERMINATE branch is too narrow to serve as that residual. It catches broken instruments, not broken theories.

**Verdict:** Falsifiable by downstream fix failure, not by its own data. The missing branch is "admissible but causally outside the four hypothesized mechanisms." That branch would route to §8's follow-on, but no condition in the predicate triggers it from admissible data.

### (2) A physically plausible cause the predicate would MISCLASSIFY

**The prune catches up fully each pass (min(L) ≤ B), but each pass takes O(n) time to scan the growing epoch index, so passes become increasingly infrequent and the eligible backlog regrows between passes.**

Concretely: the prune loop iterates epochs to find eligible ones. At width 1000 over 4 hours, the epoch range grows. Each pass scans the full range, finds a small eligible set, drains it completely, and exits. Right after the pass, eligible refs are near zero — so `min(L) ≤ B` fires. But the next pass doesn't happen for a long time because the scan itself is slow, and during that gap the workload adds new eligible refs. The *net* reclaim fraction falls because the inter-pass gap grows, even though each pass is perfectly efficient.

The predicate's path:
- Exit share ≤ 10% (everything considered gets dropped) → not Step 1 ✓
- min(L) ≤ B (the prune catches up at least once) → **Step 2: SELECTION/FRONTIER (licensing limb)** ✓
- The slope test in Steps 3/4 is **never evaluated** because Step 2 fires first ✓

**Routing consequence:** Step 2 routes to the REGISTRY branch of TODO-634 — `ceiling = min_live_claim − fixed_margin`, `ReclamationRegistry`, claim/retention model. The fix shape is "change what is licensed." But the actual cause is that **pass frequency is too low because scan overhead is O(n)**, and the fix is "index eligible epochs in O(1) or O(log n)" — a data-structure change to the prune loop, not a licensing/registry change. The predicate sends you to the wrong family entirely.

This is not a corner case. It is the natural failure mode of a linear scan over a growing index, and the SPEC-355 observations (prune fires, LWM advances, series ends at maximum) are fully consistent with it.

### (3) Are the thresholds right in shape? Is `min` the right aggregator?

**10% exit share — shape is questionable.** A fixed point threshold (exit share > 10%) cannot distinguish "exit share is steady at 8% and is the cause" from "exit share is steady at 8% and is noise." If the true cause is a small but persistent non-drop exit problem (e.g., AbsentKey at 7% because of a stale ref lookup), the predicate routes to Steps 2–4 and misses it. The shape should incorporate a *trend* (is exit share growing?) or at minimum a lower threshold. The manifest's defense — "arbitrary but fixed, the point is pre-registration" — is honest about the arbitrariness but doesn't address the shape problem. A threshold that is too high is wrong, not merely arbitrary.

**min(L) vs B — this is the predicate's weakest point, and it is a real flaw.** `min(L)` is the extreme order statistic. Over a 2-hour last half at 10s cadence, that is ~720 samples. If the prune catches up in **one** of those 720 samples — a momentary lull, a GC pause that delays the workload, a single empty-drain pass — `min(L) ≤ B` fires and Step 2 triggers. The "PERSISTENT licensed backlog" condition (min(L) > B) requires the prune to *never* catch up across all 720 samples, which is an extraordinarily strong condition. The partition is technically exhaustive but the two branches are wildly asymmetric in how easy they are to trigger.

A better aggregator would be a low percentile — P10 or P25 of L — compared to B. Or a *fraction* of samples where L ≤ B (e.g., "catches up in ≥ 50% of samples" → DRAINS). The manifest uses `min` because it wants "the prune never once catches up" to mean "persistent backlog," but the cost is that "the prune catches up at least once" becomes trivially easy, and Step 2 becomes the default branch for almost any admissible data. Given the routing consequence — Step 2 sends you to the REGISTRY family — this is a high-impact asymmetry.

**OLS slope on passes-per-epoch — shape is OK, direction is right, power is suspect.** OLS assumes linearity; if the degradation is convex (pass frequency drops slowly then accelerates), the slope may understate the trend and fail to reject negative, routing to Step 4 (THROUGHPUT) when Step 3 (SCHEDULING) is correct. The `α = 0.05` test on a single unreplicated series is low-power, but the direction ("reject negative" = "we are confident passes-per-epoch is declining") is the right test for the scheduling hypothesis.

### (4) Is Step 3's reliance on a non-replicating slope acceptable given it is one branch of four?

**Marginally acceptable, but the margin is thin.** The manifest's argument is that the slope is only evaluated in the persistent-backlog branch (min(L) > B), so if the slope is wrong, the worst case is a Step 3↔Step 4 swap. But:

- The routing consequences of Step 3 vs Step 4 are **different fix shapes**: Step 3 → fix pass scheduling; Step 4 → fix cadence/batch size. If the slope is unreliable and the true cause is scheduling (Step 3) but the slope fails to reject negative, you get Step 4 and tune batch size — a fix that cannot work because the prune is already draining everything it selects each pass.

- The manifest acknowledges the fragility (§2.6) and says the caveat is "reported beside the determination." Reporting a caveat does not make the determination correct. A pre-registered wrong answer with a caveat is still a wrong answer, and the pre-registration's authority makes it harder to challenge.

- The slope's known non-replication (2.0×, 4.6×, sign change) is from SPEC-355 at the same width and horizon. The slope is being used in the *exact regime where it was shown to be unstable*. This is not a hypothetical concern; it is a known property of the statistic in the exact context where it is being deployed.

**"Reject negative" is the right direction.** The scheduling hypothesis predicts declining pass frequency. Testing for a significant negative slope is the correct test. The problem is not the direction but the statistic's reliability.

The saving grace is that if min(L) > B (persistent backlog), the distinction between Step 3 and Step 4 is between two fix shapes that are both "make the prune work harder" (one by scheduling more often, one by draining more per pass). They are less catastrophically wrong than a Step 2 vs Step 4 misclassification. But "less catastrophic" is not "acceptable."

### (5) Single strongest reason the ordering could be wrong

**Feedback loops break the upstream-first assumption.**

The ordering assumes causes can be separated by their symptoms because they operate at different stages of a causal chain: exit → licensing → scheduling → throughput. The predicate evaluates upstream-first on the theory that an upstream cause (e.g., refs not being dropped) produces downstream symptoms (backlog, slow passes) but not vice versa.

In a real prune loop, this is false. A throughput bottleneck (Step 4 — each pass takes O(n) time as the index grows) causes the scheduler to run fewer passes (Step 3 symptom — declining passes-per-epoch). Fewer passes cause the eligible backlog to persist (Step 2 symptom — min(L) > B). A persistent backlog means the LWM doesn't advance, which pins more refs (Step 2 symptom — ineligible pool grows). The downstream cause (throughput) propagates upstream and mimics every upstream symptom.

The predicate, evaluating upstream-first, will fire on the *first* upstream symptom it sees — likely Step 2 (licensing) or Step 3 (scheduling) — and route to the upstream fix shape, when the root cause is downstream (throughput). The ordering's design — "cause precedence, evaluated upstream-first" — is exactly backwards for a system with feedback: you need to evaluate **downstream-first** to avoid catching symptoms of a downstream cause in an upstream branch.

The manifest's §2.5 says the modal expected outcome is Step 2 (licensing). If the true cause is a throughput problem with feedback (O(n) scan → fewer passes → persistent backlog → growing pinned pool), Step 2 will fire, routing to the REGISTRY family, and the fix will not work because the root cause is the scan complexity, not the licensing model.

---

## Part (b) — Neutrality Control Design (§1)

### (1) Is a non-rejection with MDE ≈ 17% worth anything, or is it decoration?

It is **not decoration, but it is not load-bearing either.** It rules out catastrophic perturbation — the kind where the instrumentation breaks the prune path entirely and tombstone_bytes shifts by 20%+. For that purpose, n=2 is adequate.

But the manifest's stated purpose is to prove the instrumentation does not perturb `tombstone_bytes`. A 17% MDE means the control is blind to any perturbation up to ~6,400 B. To put that in context: SPEC-355's run-to-run spread is 5.4% (~2,000 B), and the defect manifests as 646,306 B of growth. A 17% perturbation (~6,400 B) is small relative to the defect but large relative to run-to-run noise — it is exactly the range where a subtle instrumentation effect would plausibly live. The control cannot see into that range.

The manifest is honest: "A smaller perturbation is NOT excluded — the control is honestly weak." That honesty is the right call. But it means a non-rejection is a **lower bound on the perturbation size**, not evidence of neutrality. It is worth reporting as "no catastrophic perturbation detected" but should not be read as "the instrument is proven neutral."

**Verdict:** Worth something — rules out catastrophe. Not worth what it is being asked to carry (proving neutrality for classification purposes).

### (2) Is the 2×2's asymmetric absorption defensible?

The cell in question: §1.1 rejects (cross-lineage difference), §1.2 passes (armed ≡ disarmed). The determination: "BUILD-LINEAGE EFFECT — instrument exonerated, classification numbers STAND."

**The logic is sound in principle.** §1.2 is the better-controlled test (same binary, sole difference is arming). If armed ≡ disarmed, the instrumentation's *activation* does not perturb. The cross-lineage difference is then attributed to other code changes, which §0(b) already declared as a new lineage.

**But the absorption is wrong under one specific condition: the instrumentation's *presence* (not its *activation*) perturbs the prune path.** Both armed and disarmed are the same binary — the instrumentation code is compiled in either way. If the instrumentation adds a branch to the hot path, changes inlining decisions, shifts instruction cache layout, or alters the compiler's optimization choices, that perturbation is present in both armed and disarmed. §1.2 would pass (they're identical), and the perturbation would be absorbed as "build-lineage effect" with the classification numbers standing — but the numbers would be read off a *perturbed* run, and the perturbation is caused by the instrumentation, not by unrelated code changes.

For this to be wrong, three things must be true:
1. The instrumentation adds code to a hot path (plausible — the prune recorder fires on every pass, every drain, every drop).
2. The added code changes binary layout or optimization enough to shift `tombstone_bytes` by a meaningful amount (plausible at the 5–10% level, which is below the 17% MDE and thus invisible to §1.1).
3. The shift is large enough to change the classification predicate's determination (possible — the 10% exit-share threshold and the min(L) vs B comparison are both sensitive to shifts of this magnitude).

The absorption is **defensible as a declared assumption** — the manifest declares the lineage break upfront and says a cross-lineage difference is expected. But "expected" and "acceptable for classification" are different claims. The manifest conflates them by allowing the classification numbers to stand without caveat. The right disposition would be: "BUILD-LINEAGE EFFECT — classification numbers stand *with a stated bound on the unexplained shift*, and any determination that turns on a threshold near the shift size is flagged as fragile."

### (3) Is the decision not to α-correct sound?

**Yes, sound.** The cost asymmetry is real and correctly stated:

- False adverse (spurious rejection) → re-run the control. Cost: ~4–8 hours of compute plus analysis.
- False clean (spurious non-rejection) → invalid classification numbers route to the wrong fix shape. Cost: potentially weeks of engineering effort on the wrong solution, plus the authority of a pre-registered determination makes the wrong answer harder to challenge.

Given this asymmetry, making rejection *easier* (no correction, family-wise rate ~9.8%) is the conservative direction. A ~10% chance of a spurious adverse reading that costs a re-run is cheap insurance against a ~5% chance of licensing invalid numbers.

The one caveat: if the control is run repeatedly across multiple specs or cells, the cumulative false-adverse rate grows. The manifest treats this as a one-shot control set (two tests, one run), which is the right scope for the decision. If the control is reused, the no-correction decision should be revisited.

**The manifest is right here. Say so plainly.**

### (4) A perturbation mode both controls are blind to by construction

**The instrumentation's *presence* in the binary, independent of its *activation*.**

§1.2 (armed vs disarmed) controls for the instrumentation's *activation* — whether the recorder body fires, whether the sampler writes series. It does not control for the instrumentation's *existence* — the compiled-in branches, the changed inlining, the shifted cache layout, the additional enum variants in the exit-type enum. These are present in both armed and disarmed because they are the same binary.

§1.1 (new binary vs SPEC-355) can *see* the total effect of all code changes including instrumentation presence, but it cannot *attribute* the difference to the instrumentation versus other code changes. The 2×2's absorption cell attributes the entire cross-lineage difference to "build-lineage effect" and moves on.

The specific perturbation mode: **a branch added to the prune hot path by the recorder hook that the compiler cannot eliminate when disarmed, causing a small but persistent slowdown that shifts pass frequency and thereby shifts the classification predicate's determinations — at a magnitude below the 17% MDE, so §1.1 cannot detect it, and identical in armed and disarmed, so §1.2 cannot detect it.**

This is not theoretical. The prune recorder is specified to increment counters on every pass, every drain, every drop, every LWM advance. Even if the recorder body is a no-op when disarmed, the branch that checks whether to record is still there. At 10s cadence over 4 hours, even a 1% per-pass overhead compounds.

A second, distinct blind spot: **the 10s prune.csv sampler's I/O.** If the sampler runs in both armed and disarmed (writing the inherited columns either way), its disk I/O perturbs the system in both modes, and §1.2 is blind to it. §1.1 sees it but absorbs it as "build effect." The manifest does not specify whether the sampler is suppressed in disarmed mode; if it is not, this is a second constructed blind spot.

### (5) Should n be larger, and what n makes the control load-bearing?

**Yes. n=2 is ceremonial for the cross-lineage control (§1.1).** With SPEC-355's observed sd of 1,479 B, n=2 gives MDE ≈ 17%. To make the control load-bearing — meaning it can detect the kind of perturbation the instrumentation would realistically cause (5–10%) — you need:

| n per arm | df | t_crit | MDE (B) | MDE (% of 37,670) | Total control runtime |
|-----------|-----|--------|---------|---------------------|----------------------|
| 2 | 2 | 4.303 | 6,372 | 16.9% | 16 h |
| 3 | 4 | 2.776 | 3,348 | 8.9% | 24 h |
| 4 | 6 | 2.447 | 2,561 | 6.8% | 32 h |
| 5 | 8 | 2.306 | 2,159 | 5.7% | 40 h |
| 6 | 10 | 2.228 | 1,899 | 5.0% | 48 h |
| 8 | 14 | 2.145 | 1,587 | 4.2% | 64 h |

(Computed as t_crit × s_pooled × √(2/n), with s_pooled ≈ 1,479 B from SPEC-355.)

**n=6 per arm (48 hours total) gives MDE ≈ 5%** — the point at which the control can detect a perturbation in the range where realistic instrumentation effects would manifest. This is the threshold for load-bearing. Below n=4, the control is a catastrophe detector only.

**For the within-lineage control (§1.2), n=2 might be adequate — but only if the within-lineage sd is substantially lower than the cross-lineage sd.** The within-lineage comparison is the same binary with one configuration difference, so run-to-run variability should be primarily system jitter. If the within-lineage sd is, say, 500 B (vs 1,479 B cross-lineage), then n=2 gives MDE ≈ 2,152 B ≈ 5.7% — close to load-bearing. But the manifest does not estimate this sd ahead of time, and the pre-registration locks n=2, so the control's power is a gamble on the observed sd. If the gamble loses (within-lineage sd turns out to be ~1,479 B too), the control is ceremonial at both levels.

**The pre-registration should have set n=4 for §1.1** (MDE ≈ 7%, 32 hours — still a big improvement) **and left §1.2 at n=2 with a conditional escalation to n=4 if the observed within-lineage MDE exceeds 10%.** The current design locks in a known-weak power level and provides no escalation path for the control itself (only for the classification, via §8).

---

## Summary of Structural Concerns

1. **The predicate has no residual category for admissible data.** It will always produce a determination, even when the true cause is outside the four hypothesized mechanisms. INDETERMINATE only catches broken data, not broken theory.

2. **`min(L)` as the aggregator makes Step 2 the default branch.** The prune catching up once in 720 samples routes to "licensing," missing scheduling or throughput causes. A percentile or a fraction-of-samples aggregator would be substantially more discriminating.

3. **The upstream-first ordering is vulnerable to feedback loops.** A throughput bottleneck (O(n) scan) propagates upstream and mimics licensing or scheduling symptoms. The predicate will fire on the upstream symptom and route to the wrong fix family. This is the highest-impact failure mode.

4. **The cross-lineage control at n=2 cannot detect perturbations below 17%** — exactly the range where subtle instrumentation effects would live. The within-lineage control *might* be better, but its power is a gamble on an unobservable (pre-data) sd.

5. **The 2×2's absorption of cross-lineage effects as "build-lineage" is sound in logic but too confident in disposition.** Allowing classification numbers to "STAND" without bounding the unexplained shift licenses reading determinations off a potentially perturbed run.

**What the design gets right:** The no-α-correction decision (§1.4) is correct. The stale-split fail-closed rule (ADJ-1) is correct. The RSS forfeiture (§0(b)/§0(c)) is correct and unusually honest. The commit-order pre-registration proof is sound. The 2×2's refusal to OR the two controls is correct — they test different things. The `_sum`/`_count`-primary quantile-aliasing rule (§5.4) is correct. The cell-E provenance checks (§4) are correct. These should be said plainly rather than inventing objections.

<!-- END VERBATIM /xask RESPONSE -->

---

*Invocation: `OPENROUTER_STYLE=full OPENROUTER_REASONING=low OPENROUTER_MAX_TOKENS=24000` via
`~/Projects/agent-future/scripts/openrouter-ask.sh` with `spec356-manifest.md` embedded. Usage:
prompt 10,775 / completion 16,300 tokens. A first pass at `OPENROUTER_REASONING=high` was discarded
because the reasoning budget truncated the answer; the discarded pass raised no finding absent from the
round recorded here.*
