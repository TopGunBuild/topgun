# SPEC-356b verdict adjudication — the pinned cross-vendor round (glm-5.2, 2026-08-11)

This is the artifact SPEC-356c R3.0 obligates and PRESENCE entry P9 grades: the
cross-vendor adjudication of SPEC-356b's INDETERMINATE verdict, source of the FIVE
pre-declared observation targets (T1-T5) and of the ruling that the verdict is TRUE
(the physics matches none of the four pre-registered mechanisms), not procedural.

Question put to the vendor (verbatim):

> VERDICT ADJUDICATION REQUEST. A pre-registered classification experiment (frozen protocol, 20 pre-data adjudications) has produced its verdict. You are the pinned cross-vendor adjudicator: attack the verdict's reading and shape what happens next. All numbers below were independently re-derived by me from the raw committed artifacts.
> 
> THE EXPERIMENT. Question: why does a tombstone-prune mechanism reclaim a falling fraction of garbage as epoch width grows (measured earlier: 95-98% reclaim at width 100 vs 33% at width 1000/4h)? Four pre-registered mechanisms: SELECTION/FRONTIER-exit (refs considered but not droppable), SELECTION/FRONTIER-licensing (deficit in what gets licensed), SCHEDULING (prune runs less often over time), THROUGHPUT (prune runs but can't keep up). Discriminators: non-drop exit share; median(L) vs B (L=licensed backlog gauge sampled pre-drain each pass; B=median batch size from a rendered p50 column with 0-sentinel-exclusion, >50% sentinel => escape hatch to INDETERMINATE); pass-rate decline test.
> 
> THE VERDICT: INDETERMINATE at Step 0(c) — the 0-sentinel escape hatch fired at 98.33% (708/720 last-half scrapes had an expired batch-summary window). Steps 1-4 never evaluated. Determination wording: best-supported mechanism NONE; all four left standing; a mechanism outside the four is NOT excluded.
> 
> THE PHYSICS (deciding 7,190s window, width 1000, all verified by me):
> - 232,367 prune passes; 232,365 EMPTY drains; exactly 2 NON-EMPTY drains (conservation holds exactly).
> - Epochs advanced 230 -> 462 (232 epochs). The prune's low-water-mark tracked them with ~ZERO lag (230 -> 462); durable watermark lag 1-2 epochs. So LICENSING IS NOT STALLED — the watermarks advance briskly.
> - bytes_freed delta = 46,000 B (~= exactly the 2 non-empty drains ~ 2 epochs' content). tombstone bytes ADDED delta = 5,349,731 B. Marginal reclaim in the window: 0.86%. Backlog grew 5.3 MB.
> - Per-epoch nominal content ~23 KB (5.3MB/232). max(L) observed = 1000 refs (one full epoch's worth) at least once. Yet 230 of 232 licensed epochs drained EMPTY.
> - Control contrast (width 100): 0% sentinel, 438 non-empty drains in 900s, B=99.99, reclaim healthy. The instrument demonstrably works in the keeping-up regime.
> - Both level controls CLEAN (armed-vs-disarmed t=0.35/0.80 vs critical 4.303).
> 
> THE PUZZLE THE FOUR MECHANISMS MAY NOT COVER: LWM advances past 230 epochs whose nominal content was never freed. Either (a) those epochs' tombstones were not in the drainable index when their turn came (residency/index-membership gap — the instrument records indexed_refs but the classification predicate never reads its trajectory), or (b) the LWM advances without subsuming content (a known unsafe-direction class in this codebase's history), or (c) content was reclaimed by a path that doesn't increment bytes_freed. The run's own residual-evidence note names: per-pass cost growth, index-scan cost, allocator effects — none observed by this instrument.
> 
> PRE-REGISTERED OBLIGATIONS NOW OWED:
> - §8.1: exactly ONE repeat of the deciding configuration at DOUBLED duration and DOUBLED replicates (8h x n=2 = 16h machine), same pin, same frozen predicate, NO threshold/ordering adjustments. If still INDETERMINATE -> §8.2 escalates into the reclamation-registry design phase (TODO-634).
> - §8.3 (frozen, quoted beside the outcome): the reclamation-registry model closes SAFETY regardless of cause; the cost of INDETERMINATE is fix-shape efficiency, not safety.
> - POST-DATA constraint: no addendum may alter any predicate/threshold now. Six findings routed to named follow-ons, including TODO-648: the committed grader medians the p50 column over ALL rows while the pre-registration defines B with sentinel rows EXCLUDED — they disagree maximally here (0 vs 999.94); repair requires editing a pinned sidecar, forbidden post-data.
> 
> QUESTIONS (attack, findings-first, concise):
> 1. Is INDETERMINATE the honest reading, or did the protocol misroute a physically-classifiable regime? Note: the counter-anchored starvation clause (eligibility-bound evidence when Delta-nonempty=0 AND every pass empty AND backlog grows) did NOT fire because Delta-nonempty=2, not 0 — two drains in two hours blocked the starvation classification. Was that clause's =0 strictness correct pre-registration discipline, or an instrument limitation silently converting a classifiable regime into INDETERMINATE? (Careful: the honest answer may be "correct — the LWM-advance physics genuinely does not match ANY of the four mechanisms, so INDETERMINATE is true, not just procedural.")
> 2. The owed 16h repeat: given the physics, is it informative or predictably-INDETERMINATE? POST-DATA rules forbid changing predicates, but OBSERVATION targets can be declared. Name the 3-5 specific series/derived quantities the repeat's report should carry so the 16h buys mechanism-identification even if the classification repeats INDETERMINATE (e.g., indexed_refs trajectory vs added-tombstones; per-drain content of the rare non-empty drains; LWM-advance events vs bytes_freed correlation; does drain frequency scale with duration).
> 3. Does the physics already constrain the registry design direction regardless of classification (§8.3)? Specifically: if LWM can advance past never-freed content, does the planned design (prune ceiling = min live claim - fixed margin; monotone prune watermark; two-phase expiry) need an additional invariant — e.g., watermark-advance implies content-accounted (freed or provably-absent), the F4-axis rule?
> 4. TODO-648 repair channel: the grader (committed, digest-pinned sidecar) disagrees with the pre-registered definition of B. Post-data, the sidecar is frozen. What is the honest repair shape for the REPEAT run — grade with the as-pinned grader and report the discrepancy, or pre-declare (before the repeat's data exists) a corrected grader as a NEW versioned sidecar with both run side-by-side? The repeat has not started; its data does not exist yet.

Vendor answer (verbatim, z-ai/glm-5.2 via openrouter, cost $0.0115):

> → openrouter z-ai/glm-5.2 answering (0 file(s) embedded)…
> 
> [usage] prompt=1534 completion=4266 total=5800 cost=$0.01148956
> ##1. INDETERMINATE: honest reading, or misrouted classification?
> 
> **INDETERMINATE is the correct verdict, and it is correct for two independent reasons — only one of which the protocol actually invoked.**
> 
> The protocol fired on the 0-sentinel escape hatch (B uncomputable). That alone is procedurally sufficient. But even if B were perfectly computable, the classification would still land INDETERMINATE because the physics matches none of the four mechanisms:
> 
> - **FRONTIER-exit**: ruled out — LWM advances briskly (230→462), no frontier stall.
> - **FRONTIER-licensing**: ruled out — watermarks advance, licensing is not stalled. max(L)=1000 shows full epochs get licensed.
> - **SCHEDULING**: ruled out — 232,367 prune passes in the window; the scheduler is running constantly.
> - **THROUGHPUT**: ruled out — drains are EMPTY, not full-but-overwhelmed. Throughput starvation produces full drains that can't drain fast enough. Here the drains are empty. The prune is not keeping up with nothing.
> 
> The actual signature — *prune runs constantly, drains are empty, LWM advances past never-freed content, backlog grows* — is a fifth mechanism. The verdict's wording ("a mechanism outside the four is NOT excluded") is the honest framing of this.
> 
> **On the =0 strictness of the starvation clause**: this was correct pre-registration discipline, not a silent instrument limitation. The clause was designed to catch *absolute* starvation (Delta-nonempty=0). The physics is *near-total* starvation (2 non-empty drains in ~2h). These are different regimes: 2 drains in 2 hours could be a periodic catch-up sweep, a boundary artifact, or a qualitatively different trigger than "never drains at all." Pre-registering =0 commits you to a sharp claim. Relaxing to ≤2 post-data is textbook threshold massage. The clause correctly refused to fire on evidence it was not designed to classify.
> 
> The deeper point: even if the clause had fired and classified "starvation," that label would be *wrong* for the physics. The LWM advances past content that was never freed — that is not starvation (the prune is not failing to run, it is running on empty sets while the watermark marches forward). A starvation label would misdirect the fix toward "make prune run more" when the problem is "the prune's notion of what to reclaim has diverged from the watermark's notion of what has been reclaimed." The clause's failure to fire is a feature, not a bug.
> 
> **Verdict: INDETERMINATE is true, not merely procedural.** The escape hatch is the proximate cause; the lack of a matching mechanism is the substantive cause. Both point the same direction.
> 
> ---
> 
> ## 2. The 16h repeat: informative or predictably-INDETERMINATE?
> 
> **Predictably INDETERMINATE for the classification.** The sentinel rate will not improve with longer duration — expired batch-summary windows are a duration-driven phenomenon, and doubling to 4h per replicate will make the expiry worse, not better. The classification predicate is frozen and will fire the same escape hatch. The four mechanisms still won't match. The §8.1 repeat is an obligation, not a bet.
> 
> **But the 16h can buy mechanism identification through observation targets** that the frozen classification predicate does not consume. These must be declared before data collection (they are observations, not classification predicates — the distinction matters). Five targets:
> 
> 1. **indexed_refs vs added_tombstones, per-epoch, at the moment LWM passes that epoch.** This is the direct test of hypothesis (a). If indexed_refs < added_tombstones for the epochs that LWM advances past, the content was never in the drainable index — residency gap confirmed. If indexed_refs = added_tombstones but drains are still empty, the content is in the index but the drain predicate doesn't select it — hypothesis (b) territory.
> 
> 2. **Full content enumeration of every non-empty drain** (there will be ~4-6 of them across 16h). For each: which epochs' tombstones, how many bytes, was bytes_freed incremented by the exact tombstone size? This distinguishes "periodic catch-up" from "boundary artifact" and tests hypothesis (c) (reclaim path that doesn't increment bytes_freed).
> 
> 3. **delta(LWM) vs delta(bytes_freed), windowed every 1000s.** If LWM advances N epochs worth of nominal content but bytes_freed is ~0, the watermark-advance-without-content-accounting signature is quantified, not just observed once. The slope of this divergence over 4h tells you whether it's accelerating, linear, or saturating.
> 
> 4. **Non-empty drain rate vs duration.** 2 drains in 2h, ~4 in 4h would suggest a fixed-rate periodic trigger (every ~1h). 2 drains in 2h, 2 in 4h would suggest a one-time transient. This is the cheapest discriminant and the most actionable for the registry design.
> 
> 5. **Epoch content fate ledger**: for each epoch 230-462, record (added_bytes, indexed_refs_at_lwm_pass, was_drained_ever, bytes_freed_attributed). This is the single artifact that would let the §8.2 design phase proceed without guessing. It costs nothing to record and is the difference between "we know LWM advances past unfreed content" and "we know *which* content, *when*, and *why not*."
> 
> Target 5 subsumes most of the diagnostic value. If you can only carry one, carry that.
> 
> ---
> 
> ## 3. Does the physics already constrain the registry design?
> 
> **Yes, and the constraint is safety-grade, not efficiency-grade.**
> 
> §8.3 says the reclamation-registry model closes safety regardless of cause. That claim needs to be tested against the observed physics, not assumed.
> 
> The observed physics: **LWM can advance past content that was never freed.** The planned design has:
> - Prune ceiling = min(live_claim) - fixed_margin
> - Monotone prune watermark
> - Two-phase expiry
> 
> The monotone watermark is the critical risk. If the new registry's watermark can advance past unfreed content — same as the old LWM — then the safety closure depends entirely on the two-phase expiry being correct. But the observed physics is evidence that the *current* two-phase expiry (mark-then-sweep) has a hole: the mark completes, the sweep produces empty drains, and the watermark advances anyway. If the new registry inherits this two-phase structure, it inherits the hole.
> 
> **The F4-axis invariant is necessary**: watermark-advance must imply content-accounted (freed or provably-absent). Without it, "monotone prune watermark" is a license to forget unfreed content. The fix shape is:
> 
> - Watermark advance gated on a *completed* sweep of the span being advanced past, not just on the mark phase.
> - Or: watermark advance gated on indexed_refs == 0 for the span (provably-absent).
> - Or: watermark advance gated on bytes_freed == bytes_added for the span (content-accounted).
> 
> The current instrument cannot distinguish these because it doesn't record the indexed_refs trajectory (target 1 above). The repeat should, and the §8.2 design phase should require this invariant as a design precondition, not a post-hoc check.
> 
> **Trade-off**: gating watermark advance on completed sweeps could stall the watermark if the sweep is slow or if content is genuinely absent (not in index, so sweep sees nothing but can't prove absence). The invariant needs a "provably-absent" escape path, or it becomes a liveness hazard. This is a real design tension, not a checkbox.
> 
> ---
> 
> ## 4. TODO-648 repair channel
> 
> **Pre-declare a corrected grader as a new versioned sidecar before the repeat's data exists. Run both side-by-side.**
> 
> Reasoning:
> 
> The pre-registered definition of B (sentinel rows excluded) is the spec. The committed grader (sentinel rows included) is a bug. Fixing a bug to match the spec is not post-data manipulation — it is correction. The constraint that forbids changes is "no addendum may alter any predicate/threshold now." A bug fix to match the pre-registered definition does not alter the predicate; it *restores* it.
> 
> The alternative — "grade with as-pinned grader and report the discrepancy" — is honest but wastes the 16h. The discrepancy is already known to be maximal (0 vs 999.94). Re-confirming it at 16h adds no information. And the as-pinned grader will fire the escape hatch at ~98%+ sentinel rate regardless, so the classification outcome is identical under both graders. The only value of running the broken grader is audit trail completeness.
> 
> **Concrete repair shape:**
> 
> 1. Before the repeat starts, commit a new versioned sidecar (v2) that implements the pre-registered definition of B exactly (sentinel rows excluded, escape hatch at >50% sentinel). Version it, digest-pin it, and publish the diff from v1 showing the one-line correction.
> 2. The repeat's classification runs under v2 (the corrected grader). v1 (the broken grader) runs in parallel and its output is reported as "known-bug reference, not used for classification."
> 3. Both graders consume the same raw data. The classification verdict is v2's. The v1 output is appendix-only.
> 4. Document in the repeat report: "v1 grader contained a defect (sentinel inclusion) identified in TODO-648; v2 corrects to match §pre-registered definition; no predicate or threshold was changed; the correction was pre-declared before data collection."
> 
> **The one risk**: if someone argues that "correcting the grader" is a predicate change in disguise. It is not — the predicate is the pre-registered definition. The grader is an implementation of the predicate. A grader that computes B differently from the definition is not a different predicate; it is a broken implementation of the same predicate. But this argument needs to be stated explicitly in the repair record, not assumed.
> 
> **What NOT to do**: do not relax the >50% sentinel escape hatch threshold. The sentinel rate is real and will fire regardless. The escape hatch is correctly designed — it correctly identifies that B is uncomputable in this regime. The problem is the regime, not the hatch.
