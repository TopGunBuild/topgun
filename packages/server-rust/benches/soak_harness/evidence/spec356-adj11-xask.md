# ADJ-11 adversarial cross-vendor round (glm-5.2, 2026-08-08)

Question put to the vendor (verbatim):

> CONTEXT. A pre-registered measurement protocol (frozen manifest, append-only adjudication addenda) classifies why a tombstone-prune mechanism reclaims a falling fraction of garbage as epoch width grows: THROUGHPUT-BOUND (prune licensed but can't keep up) vs ELIGIBILITY-BOUND (garbage exists but never becomes licensed/eligible for prune). Instrument: a per-prune-pass record exported as Prometheus metrics, scraped every 10s into a CSV.
> 
> Key metrics:
> - topgun_or_prune_eligible_refs — GAUGE "L": licensed (eligible) backlog, sampled PRE-drain inside each prune pass (set at each pass, retains value until next pass; scraped at 10s cadence). Prune passes may run more often than 10s, so scrape-max(L) samples a subset of passes.
> - topgun_or_prune_nonempty_drains_total, drain_refs_sum, bytes_freed_total — MONOTONE COUNTERS: cumulative drains that removed >0 refs, cumulative refs drained, bytes freed.
> - topgun_or_prune_split_recomputes_total — counter certifying the eligible/ineligible split recompute actually ran (admissibility gate "Step 0(b)": if this is flat in a window, the window is INDETERMINATE, fail-closed).
> 
> THE FORK TO ADJUDICATE (pre-data, must be frozen before any measurement exists):
> An earlier adjudication (ADJ-7) says: "A Step-2 determination must be reported together with max(L) over the window... A window whose max(L) is also 0 is a window in which the sampler never observed a non-empty drain, and that is an admissibility observation, not evidence that the prune is licensed-and-draining."
> The audit found a tension: the admissibility gate (Step 0) is fail-closed (any limb fails => whole window INDETERMINATE, no classification), but if max(L)=0 auto-routes to INDETERMINATE, then a genuinely eligibility-starved system (nothing ever becomes licensed => L=0 at every pass) could NEVER be classified as eligibility-bound — the instrument would structurally predetermine against one of the two verdicts it exists to distinguish.
> 
> MY PROPOSED ADJUDICATED FORM (attack this):
> max(L)=0 alone routes nothing to INDETERMINATE. Window disposition is decided by the monotone counters:
> (i) Step 0(b) fails (split_recomputes_total flat) => INDETERMINATE (existing rule, untouched).
> (ii) Step 0(b) passes AND nonempty_drains_total FLAT over the window AND the tombstone backlog series grows => max(L)=0 is a VALID measurement consistent with eligibility-bound; flows to Step 2 as evidence. ADJ-7's prohibition stands: it must never be read as "prune is licensed-and-draining".
> (iii) Step 0(b) passes AND nonempty_drains_total ADVANCES while scrape-max(L)=0 => the 10s gauge cadence under-sampled the pre-drain instants; scrape-max(L) is INADMISSIBLE as a bound for this window; the determination is carried by the counters (drain_refs_sum delta, nonempty_drains_total delta) and the max(L) cell is reported as UNDERSAMPLED, not INDETERMINATE.
> 
> QUESTIONS:
> 1. Does this adjudication predetermine either verdict (throughput-bound or eligibility-bound) in any regime you can construct? Construct concrete adversarial regimes (pass frequency vs scrape cadence, bursty licensing, drain-within-pass timing) that break clauses (ii) or (iii).
> 2. Is there a hole where a BROKEN instrument (recorder running but gauge wired wrong, e.g. always exporting 0) would be classified as eligibility-bound instead of caught? What cheap cross-check closes it?
> 3. Is "UNDERSAMPLED" the right disposition in (iii), or should (iii) escalate differently? Keep in mind the protocol's ethos: fail-closed for instrument defects, but never let the instrument's own limitations silently pick a scientific verdict.
> Answer concisely, findings-first.

Vendor answer (verbatim, z-ai/glm-5.2 via openrouter, cost $0.0094):

> → openrouter z-ai/glm-5.2 answering (0 file(s) embedded)…
> 
> [usage] prompt=950 completion=11607 total=12557 cost=$0.0093797432
> ## Findings
> 
> ### Q1: Yes — clause (ii) can false-positive eligibility-bound; clause (iii) loses discriminative power.
> 
> **Clause (ii) break — "transient eligibility + stuck drain":**
> 
> Passes at 20 Hz (50 ms), scrape at 0.1 Hz (10 s) → 200 passes per scrape. Every 12 s, a recompute finds ~500 eligible refs (L←500), but the drain step is blocked by I/O contention or lock timeout — a genuine throughput pathology. By the next pass (50 ms later), eligibility has expired or been reclassified by a concurrent writer; L←0. The scraper has ~0.4% chance per scrape of catching L>0. Over a 60 s window (6 scrapes), P(all miss) ≈ 97.6%. Result: `nonempty_drains_total` flat, backlog grows, max(L)=0 → clause (ii) fires → **eligibility-bound**. Truth: eligible refs repeatedly appear but prune can't catch them. This is throughput-bound (or prune-broken), misclassified as eligibility-starved. The root cause is that clause (ii) uses flat drains as a proxy for "nothing was eligible," but flat drains only proves "nothing drained," not "nothing was eligible."
> 
> **Clause (iii) break — "counters can't finish the job":**
> 
> Passes at 100 Hz. Every 5 s, 1000 refs become eligible and drain within one pass (L spikes for ~10 ms then returns to 0). Meanwhile ineligible garbage accumulates at 500 refs/s. Scraper likely misses all spikes. max(L)=0, `nonempty_drains_total` advances → clause (iii) → UNDERSAMPLED, "determination carried by counters." But the counters show only that 6000 refs drained and backlog grew by 15000. Is the growing backlog eligible (throughput-bound) or ineligible (eligibility-bound for that portion)? **The counters cannot answer this.** They exclude pure eligibility-bound but cannot confirm throughput-bound. Clause (iii) silently leans toward "not eligibility-bound" — which is the instrument's sampling limitation picking a side, not a scientific determination.
> 
> **Structural bias:** The adjudication doesn't make either verdict *impossible*, but it systematically under-detects throughput-bound in undersampled regimes, and clause (ii) can structurally false-positive eligibility-bound when eligibility is real but transient and drains fail for reasons unrelated to eligibility.
> 
> ---
> 
> ### Q2: Yes — a broken gauge (always 0) escapes detection in the flat-drain regime.
> 
> If the gauge is hard-wired to 0:
> 
> | System state | Drains | Clause fired | Disposition | Correct? |
> |---|---|---|---|---|
> | Throughput-bound, draining | advancing | (iii) | UNDERSAMPLED | Mislabels defect as undersampling, but counters partially compensate |
> | Throughput-bound, prune stuck | flat | (ii) | eligibility-bound | **Wrong. Eligible refs exist but gauge hides them.** |
> | Genuinely eligibility-bound | flat | (ii) | eligibility-bound | Correct by coincidence |
> 
> **Cheap cross-check using existing metrics:** You cannot drain >0 refs without L>0 at some pass. Therefore: **if `drain_refs_sum` has *ever* advanced in any historical window where max(L)=0, set a persistent `GAUGE_DEFECT` flag.** Once set, all subsequent clause (ii) dispositions downgrade from "eligibility-bound evidence" to INDETERMINATE(gauge suspect). This closes the hole: a broken gauge will eventually produce a draining window (unless the system is permanently and genuinely eligibility-bound — in which case the classification is correct anyway and the flag never fires).
> 
> **Additional cross-check (no new metrics):** Correlate `split_recomputes_total` rate with scrape cadence. If the recompute runs at 100 Hz and the scraper samples at 0.1 Hz, the scraper catches 1 in 1000 passes. In that regime, max(L)=0 is overwhelmingly more likely to be undersampling than genuine eligibility starvation. If the recompute rate is high and L has been 0 for many windows, escalate to INDETERMINATE(instrument suspect) regardless of drain state.
> 
> ---
> 
> ### Q3: UNDERSAMPLED is too soft. It should split into INADMISSIBLE (for the gauge) + INDETERMINATE (for the primary fork), with counters as secondary evidence only.
> 
> The protocol ethos says: fail-closed for instrument *defects*, but never let instrument *limitations* silently pick a verdict. Undersampling due to pass-frequency exceeding scrape cadence is a limitation, not a defect. But clause (iii) currently lets that limitation redirect the verdict to the counters, which then lean toward "not eligibility-bound" (drains happened → something was eligible). That lean is the instrument's sampling gap picking a side.
> 
> The counters can **exclude** pure eligibility-bound (drains advanced → eligibility existed at some point). They **cannot confirm** throughput-bound (need to know whether the *growing* backlog is eligible, which requires L). That's a one-sided determination masquerading as a complete one.
> 
> **Recommended disposition for (iii):**
> 
> - The max(L) cell is **INADMISSIBLE** — not "undersampled," which implies it's a noisy version of the truth. It's a non-sample: it provides no bound, not even a lower bound, on the eligible backlog in this window.
> - The primary fork classification (throughput-bound vs eligibility-bound) is **INDETERMINATE** for this window. The counters are reported as secondary evidence ("N refs drained, M bytes freed") but cannot carry the primary determination.
> - If the tombstone backlog series is *stable or shrinking* while drains advance, that's affirmative evidence the system is keeping up (neither verdict in the problematic sense) — report that as a secondary finding. If the backlog is *growing*, the counters alone cannot disambiguate.
> 
> This keeps the ethos intact: the instrument's sampling limitation produces an honest "I don't know" rather than a silent lean.
> 
> ---
> 
> ### One thing I'm uncertain about
> 
> I don't have context on what the "tombstone backlog series" actually measures (total tombstones? eligible+ineligible? something else?) or whether it's a Prometheus metric with the same sampling issues as L. If it's also scrape-sampled at 10 s and can miss transient states, my analysis of clause (ii)'s backlog-grows check needs adjustment — the backlog series could also under-sample and mask growth or shrinkage within a window. If it's a monotone counter (cumulative tombstones created), it's more reliable and my analysis holds.
