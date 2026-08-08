# ADJ-12/ADJ-13 adversarial cross-vendor round (glm-5.2, 2026-08-08)

Question put to the vendor (verbatim):

> CONTEXT. Frozen pre-registered protocol (append-only adjudications) classifies a tombstone-prune deficit. The Step 2 vs Steps 3-4 partition is a pair of EXACT COMPLEMENTS on one predicate (celebrated as "mutually exclusive by construction rather than by judgement"):
> - "licensed work DRAINS" <=> median(L) over the last half <= B  (deficit is in WHAT IS LICENSED -> routes to a claim-registry redesign)
> - "PERSISTENT licensed backlog" <=> median(L) > B  (further split by a passes-per-epoch slope test into SCHEDULING vs THROUGHPUT -> routes to a prune accelerator)
> where L = licensed (eligible) backlog gauge sampled pre-drain each pass, scraped every 10s; median(L) = median over scrapes in the last half of the window. B is DEFINED as "the median refs per non-empty drain over the last half. Self-calibrating: it is one prune batch." An earlier adjudication (ADJ-2) deliberately replaced min(L) with median(L) BECAUSE "B is already a median; median-against-median is the coherent comparison".
> 
> THE DEFECT (audit, pre-data): B has NO committed source. The metric topgun_or_prune_drain_refs is a histogram, but the committed CSV samples only _sum and _count. A median is not derivable from sum+count. The estimator must be pre-registered NOW (before any data) because the choice decides the modal routing (registry vs accelerator).
> 
> AVAILABLE SOURCES (no Rust changes possible; shell runner changes ARE possible with a ~15min evidence-gate re-capture):
> 1. Monotone counters: drain_refs_sum, nonempty_drains_total -> per-window MEAN batch = Δsum/Δcount. Exact, counter-anchored. But: batch sizes are plausibly right-skewed (occasional huge drains) -> mean > median -> inflated B makes "median(L) <= B" MORE likely -> systematically biases toward the Step 2 verdict, which happens to be the modal/expected outcome (confirmation-shaped bias). Also breaks ADJ-2's median-vs-median coherence.
> 2. The Prometheus exporter ALREADY renders quantile lines for the histogram (e.g. drain_refs{quantile="0.5"}) at zero code cost; the runner could add this p50 line as one more CSV column (shell-only change + re-capture of the column-census gate artifact). CAVEAT in the instrument's own docs: "the rendered quantile is a window statistic, not an over-the-run distribution" (rolling summary window). Proposed aggregation: B := median over the last-half scrapes of the per-scrape rendered p50 — structurally symmetric with median(L), which is likewise a median-over-scrapes of an instantaneous statistic. Sub-caveats: (i) rolling-window semantics of the exporter's summary (window length is an exporter internal); (ii) scrapes whose window saw no drains render NaN -> need a pre-registered NaN rule; excluding NaN scrapes conditions B on "recent drain occurred" (selection effect).
> 3. A guard-band on option 1: e.g. verdict requires order-of-magnitude separation (median(L) <= B/10 or > 10B, middle band -> INDETERMINATE). But this DESTROYS the exact-complement partition (a celebrated structural property) and introduces an arbitrary constant (this project has repeatedly killed false-precision constants).
> 
> MY LEAN: option 2 (sample the rendered p50; B = median-over-scrapes of p50), with a pre-registered NaN rule, preserving both the partition structure and median-vs-median coherence.
> 
> SECOND QUESTION (same protocol, second defect): the "passes-per-epoch" series used by the Step 3/4 slope test has no pre-registered construction. Data: passes_total (counter), current_epoch (monotone-ish gauge), scrape cadence 10s, epochs advance ~every 31s at the widest setting (so Δcurrent_epoch=0 in ~2/3 of scrape intervals; at other widths epochs can be FASTER than scrapes, so one interval can span multiple epochs). Proposed construction: group scrape rows by current_epoch value; passes-in-epoch e = Δpasses_total between the first row at epoch e and the first row at epoch e+1; intervals where Δcurrent_epoch >= 2 span unobserved epoch boundaries -> those epochs are EXCLUDED from the series but COUNTED, with the exclusion fraction reported; the (already-mandated) fit runs on the observed-boundary epochs with the same pinned fitter the protocol uses for its byte-slope. Attack this: does exclusion bias the slope (are excluded epochs systematically the busy ones?), what should the disposition be when the exclusion fraction is large, and is there a construction with no exclusion at all from these two series?
> 
> QUESTIONS:
> 1. Attack option 2: concrete regimes where median-over-scrapes-of-p50 misclassifies the partition (rolling-window aliasing, sparse drains, NaN rule variants). Is the NaN selection effect fatal? Propose the least-bad NaN rule.
> 2. Is option 1's confirmation-shaped bias actually disqualifying, or would you take mean+declared-bias over p50's window semantics? Which failure is worse for a pre-registered instrument?
> 3. Second question above: attack the passes-per-epoch construction.
> 4. Anything in either proposal that lets the instrument's limitations silently pick a verdict (the protocol's cardinal sin)?
> Concise, findings-first.

Vendor answer (verbatim, z-ai/glm-5.2 via openrouter, cost $0.0172):

> → openrouter z-ai/glm-5.2 answering (0 file(s) embedded)…
> 
> [usage] prompt=1245 completion=6810 total=8055 cost=$0.01723871
> ##1. Attacking Option 2 (median-over-scrapes of rendered p50)
> 
> **Median-of-medians ≠ global median.** Each scrape's p50 is the median of batch sizes *within the exporter's rolling window*. Taking the median of these window-medians weights each window equally regardless of how many drains it contains. A window with 1 drain and a window with 50 drains contribute identically. If windows with few drains have systematically different batch sizes than windows with many drains, B is silently biased. Direction is unknown pre-data — could go either way. This is distinct from the NaN issue and is the deeper structural defect.
> 
> **Rolling-window aliasing regimes:**
> 
> - **Long window (≫ scrape interval):** Adjacent scrapes' p50 values are near-identical (overlapping windows) → pseudoreplication. The median-over-scrapes collapses to ~one observation. You lose the benefit of multiple scrapes and are at the mercy of where the window sits relative to a regime change.
> - **Regime change within the half-window:** The rolling window smears the transition. B is neither the old nor new regime's median but a blend, with the blend ratio determined by the exporter's internal window length — an instrument parameter you don't control.
> - **Bursty drains with window length ≈ burst period:** The p50 oscillates between "burst phase" and "gap phase" depending on scrape-window alignment. Whether more scrapes land in burst or gap phase is essentially random w.r.t. drain dynamics → B is a coin flip near the boundary.
> 
> **NaN rule — the selection effect is NOT fatal, but has a subtlety:**
> 
> B is defined as "median refs per *non-empty* drain." Conditioning on "drain occurred in window" (NaN exclusion) is therefore *correct population selection*, not bias. The definition itself requires excluding empty-drain observations. The concern is narrower: if drain *frequency* correlates with drain *size* (e.g., big drains only happen during backlog spikes), then NaN exclusion conditions on a covariate of batch size, which biases B. But this is a property of the drain process, not the NaN rule per se.
> 
> **Least-bad NaN rule:** Exclude NaN scrapes from the B computation. Report the NaN fraction as a pre-registered quality gate. Pre-register a threshold (e.g., NaN fraction > 50% of last-half scrapes → the B estimate is unreliable). The disposition above the threshold should be escalation/INDETERMINATE as a *protocol escape hatch* (separate from the statistical partition), not a guard band on B itself. This preserves the exact complement while admitting that the instrument can fail to produce sufficient evidence.
> 
> **Bottom line on option 2:** The median-of-medians weighting problem and the unknown rolling-window length are the real risks, not the NaN rule. Both are instrument internals that affect the verdict near the boundary. You must pre-register the exporter's window length (read it from config, commit it) and the NaN threshold.
> 
> ## 2. Option 1 (mean) vs Option 2 (p50): Which failure is worse?
> 
> **Option 1's bias:** Known direction (toward Step 2), unknown magnitude (depends on skewness, unobservable pre-data from sum+count). Declared but not correctable. Breaks median-vs-median coherence (ADJ-2's rationale), which was an aesthetic choice, not a structural requirement — so this is minor.
> 
> **Option 2's bias:** Unknown direction (median-of-medians can go either way), unknown magnitude (depends on drain-count-per-window distribution and window length). Structurally median-like, preserving coherence.
> 
> **My judgment: Option 2 is better, but not for the reason you lean toward it.**
> 
> The median-vs-median coherence argument (ADJ-2) is aesthetic and I wouldn't weight it. The real reason to prefer option 2 is that its bias, while unknown in direction, is *bounded* — median-of-medians is a robust statistic that can't be dominated by outliers, and the worst case is a weighting distortion, not an unbounded inflation. Option 1's mean is *unboundedly* biased by heavy tails: a single enormous drain (e.g., a catch-up batch after a stall) inflates the mean arbitrarily, and right-skewed batch distributions are the expectable case for prune workloads.
> 
> For a pre-registered instrument, the question is: "can I characterize the failure mode before data?" Option 1: yes, the direction is known, magnitude is not. Option 2: neither direction nor magnitude is fully characterizable pre-data, but the *type* of failure (weighting distortion, not unbounded inflation) is characterizable, and robust statistics are less brittle. I'd take the bounded-unknown over the unbounded-known.
> 
> **The disqualifying concern with option 1 is not the bias direction per se — it's the unbounded magnitude.** If the batch distribution has a heavy right tail (plausible for prune workloads — occasional catch-up megabatches), the mean can be 10× the median, which doesn't just "lean toward" Step 2, it *collapses* the partition to Step 2 regardless of the true L. That's the instrument picking the verdict, which is the cardinal sin.
> 
> ## 3. Passes-per-epoch construction
> 
> **The exclusion is biased, and the bias direction is bad.**
> 
> Excluded epochs (Δcurrent_epoch ≥ 2, i.e., multiple epochs per scrape interval) occur precisely when epochs are advancing fast — which is when the system is processing many passes per unit time. If passes-per-epoch is the quantity of interest, fast-advancing epochs are plausibly the *high-pass* epochs (the system is busy). Excluding them biases the observed series toward low-pass epochs → the slope is biased toward flat/negative → **systematically biases toward the SCHEDULING verdict** and away from THROUGHPUT.
> 
> This is the same confirmation-shaped concern as option 1 for B, and it's worse because the selection mechanism (fast epochs excluded) is *mechanically* tied to the quantity being measured (passes per epoch).
> 
> **Disposition for large exclusion fraction:** Pre-register a threshold (e.g., >25% of epochs excluded). Above it, the slope test is unreliable. Disposition should NOT be to default to either verdict — that's the instrument picking. It should be an escalation hatch (the protocol re-runs with a faster scrape cadence, or a human adjudicates). If the protocol absolutely requires a binary verdict (no escalation), default to the *more aggressive* diagnosis (THROUGHPUT) as a conservative hedge against the downward exclusion bias — but document this as a known-biased default, not a clean adjudication.
> 
> **No-exclusion construction exists:** Regress passes_total directly on current_epoch using all scrape points. The cumulative curve's slope is passes-per-epoch. Test for convexity (second derivative > 0 = increasing passes-per-epoch = THROUGHPUT). 
> 
> - Points with Δcurrent_epoch = 0 (same epoch, multiple scrapes): these are repeated x-values with vertical scatter — they carry intra-epoch pass-timing information and naturally add weight to that epoch's contribution. No exclusion.
> - Points with Δcurrent_epoch ≥ 2 (multiple epochs in one interval): the cumulative curve jumps from epoch e to e+k, which is valid — the curve is still monotone and correctly positioned. The fitter sees a coarser step but no information is lost.
> 
> This tests the *same hypothesis* (is passes-per-epoch increasing?) via cumulative curvature rather than per-epoch slope. It uses the same pinned fitter. The trade-off is reduced statistical power (curvature is harder to detect than slope), but **zero selection bias and zero exclusion**. Given that the protocol's cardinal sin is letting the instrument pick the verdict, the no-exclusion construction is strongly preferable even at the cost of power.
> 
> If the protocol insists on per-epoch deltas (for interpretability), the proportional-distribution alternative (assign Δpasses / Δepochs to each unobserved epoch) is less bad than exclusion but still flattens the series when busy epochs are coarsely observed. I'd reject it in favor of the cumulative approach.
> 
> ## 4. Silent verdict-picking audit
> 
> **B (option 2):** Three silent pickers:
> 1. *Exporter rolling-window length* — determines smoothing resolution and transition smearing. If unknown/variable, this is an instrument internal deciding the verdict near regime boundaries. **Mitigation:** commit the window length from config into the pre-registration; if it's not configurable or discoverable, option 2 is unsafe.
> 2. *NaN fraction* — if high, B is computed from few observations → high variance → noise picks the verdict near boundary. **Mitigation:** pre-registered NaN threshold + escalation hatch.
> 3. *Median-of-medians weighting* — if few-drain windows have different batch sizes than many-drain windows, B is silently weighted toward the few-drain regime. **No mitigation available without Rust changes.** This is an irreducible limitation; document it as a known bias with unknown direction, accept it, and flag that the partition is unreliable when drain frequency varies substantially across the half-window.
> 
> **B (option 1):** One silent picker:
> 1. *Unknown skewness* — the mean-vs-median gap is unobservable from the committed data and determines whether the partition is shifted or collapsed. If heavy-tailed, the partition collapses to Step 2 always. **No mitigation.** This is worse than option 2's pickers because the collapse is total, not bounded.
> 
> **Passes-per-epoch (proposed construction):** One silent picker:
> 1. *Scrape cadence vs epoch width* — the exclusion fraction is mechanically determined by the ratio of scrape interval to epoch duration. At the widest setting (31s epochs, 10s scrapes), exclusion should be rare (~3% of intervals have Δepoch≥2 if Poisson). At narrower settings, exclusion could dominate. The epoch-width setting — an operational parameter unrelated to the defect being diagnosed — silently determines the slope bias. **This is the instrument picking the verdict.** The cumulative-curve construction eliminates this entirely.
> 
> **Summary of cardinal-sin exposure:**
> 
> | Defect | Construction | Silent picker? | Bounded? |
> |--------|-------------|---------------|----------|
> | B | Option 1 (mean) | Unknown skewness collapses partition | No — unbounded |
> | B | Option 2 (p50) | Window length, NaN fraction, median-of-medians | Yes — bounded distortion |
> | Passes/epoch | Proposed (exclude Δepoch≥2) | Scrape cadence vs epoch width biases slope | Yes — bounded but systematic |
> | Passes/epoch | Cumulative curve | None | N/A |
> 
> **Recommendation:** Option 2 for B (with committed window length, NaN threshold, and documented median-of-medians caveat). Cumulative-curve construction for passes-per-epoch (no exclusion, no selection, trades power for unbiasedness). Pre-register escalation hatches for insufficient-data cases (high NaN fraction, uncharacterizable window length) as protocol exits distinct from the statistical partition.

## Post-round empirical correction (Conductor, same day)

The round was posed with a NaN-on-empty-window assumption. A live exposition probe
(server booted with TOPGUN_PRUNE_RECORD=true, /metrics scraped before any drain)
showed the installed exporter renders an empty summary window as 0, not NaN:

    topgun_or_prune_drain_refs{quantile="0.5"} 0

Since the summary records only NON-EMPTY drains, a populated window renders p50 >= 1,
so 0 is a clean reserved sentinel. ADJ-12 adopts the vendor NaN rule with s/NaN/0-sentinel/:
same exclusion, same reported fraction, same 50% threshold, same Step 0(c) escape hatch.
