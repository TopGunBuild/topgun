# SPEC-355 — `/xask` on the "unbounded at the production width" reading (R5b.1's mandatory gate)

**Model:** `z-ai/glm-5.2` via OpenRouter · **Cost:** $0.0087 (prompt 1282 / completion 3183 tokens)
**Run at:** after R3.1's 4 h run and R3.2's 8-window fits, **before** the spin-off spec and before
the `INVARIANTS.md` flip — which is the ordering R5b.1 requires.

**Framing:** adversarial. The prompt said *"I am about to declare a property REFUTED and flip an
invariant catalog entry. Try to stop me if I am wrong."*

**Outcome: it changed the catalog wording and it produced a free check that changed the finding's
whole character.** Neither was a formality.

---

## Adjudication summary

| # | Finding | Verdict |
|---|---|---|
| 1 | "REFUTED" overstates what finite observation can establish | **APPLIED — wording changed** |
| 2 | Check whether the gauge ever *decrements*; a strictly monotone series would mean the prune never fires | **APPLIED — ran it, and it changed the finding** |
| 3 | Do not write "superlinear"; n = 2 durations cannot fit a growth model | **APPLIED** |
| 4 | Declining to re-derive the bound is right *for now*, but leave a note that the estimator itself is inadequate | **APPLIED** |
| 5 | Cheapest next measurement: log prune-fire events per epoch | **APPLIED — routed into the spin-off spec** |

---

### 1 — "REFUTED" overstates; the honest claim is horizon-scoped

> *"You cannot prove unboundedness from finite observation. This is not pedantry — it changes what a
> catalog entry should say. … That is strong evidence of growth and failure to find a plateau. It is
> **not** proof of unboundedness. The series could plateau at 700 KB, 2 MB, or 50 MB — you have no
> data distinguishing those."*
>
> *"The distinction matters because someone reading 'REFUTED' in 3 years will treat boundedness as
> settled false and may stop looking for the actual bound (which may exist at a higher level or
> longer horizon). 'Not established within 4 h' keeps the right question alive."*

**APPLIED.** R5b.2's pre-registered literal for the enforcing field ended `refuted by measurement
(TODO-630)`. The pre-registration anticipated rewording and pinned what actually matters: *"If the
wording is changed, the invariant to preserve is the **window**, not the sentence"* — both `NAKED`
and a `(TODO|SPEC)-[0-9]+` token must stay inside the `grep -A3` window the gate reads. The field
therefore reads **`no plateau found in a 4 h measurement (TODO-630)`**, which keeps both tokens in
the window and drops a claim the data does not support.

The catalog row is likewise stated as **`open`**, not as a refutation — and its Statement records
the measurement rather than a verdict about all horizons.

### 2 — The free check, and it inverted the leading hypothesis

> *"The most important signal: **is the gauge ever decremented by even 1 byte across all 2878
> samples?** … If the series is strictly monotone (never decreases), prune is almost certainly **not
> firing at all**, not 'firing too slowly.' … That costs nothing and may already settle the
> harness-vs-server question."*

**APPLIED, and it is the single most useful thing in this review.** Run over the committed CSVs:

| Run | added | freed | net | **reclaim** |
|---|---|---|---|---|
| w100 HEAD (cell B) | 72,243 | 69,910 | +2,333 | **96.8 %** |
| w100 HEAD (`sweep100`) | 79,602 | 75,471 | +4,131 | **94.8 %** |
| w100 pre-family (cell D) | 66,876 | 57,979 | +8,897 | **86.7 %** |
| w300 HEAD | 74,958 | 73,539 | +1,419 | **98.1 %** |
| w1000 HEAD (`sweep1000`) | 108,255 | 86,753 | +21,502 | **80.1 %** |
| w1000 HEAD (`sweep1000b`) | 121,524 | 111,994 | +9,530 | **92.2 %** |
| w1000 pre-family (cell C) | 103,449 | 68,686 | +34,763 | **66.4 %** |
| w1000 pre-family (cellC2) | 130,790 | 105,834 | +24,956 | **80.9 %** |
| **w1000 HEAD, 4 h** | **904,435** | **299,349** | **+605,086** | **33.1 %** |

**The prune fires throughout.** Over the 4 h run the gauge decremented on **80 of 239 steps
(33.5 %)**, freeing **299,349 bytes**, with a largest single drop of 23,115 B. So the leading
hypothesis the model named — *prune never licensed at width 1000* — is **refuted by this spec's own
committed data**, and so are its variants "gauge not decremented by the prune path" and "prune
no-op'd".

What is happening instead is **sharper and more actionable**: the prune's **reclaim fraction
degrades with both epoch width and elapsed time** — ~95–98 % at widths 100/300, ~80–92 % at width
1000 over 1800 s, and **33.1 % at width 1000 over 4 h**. The prune is not broken; it progressively
**falls behind**.

*Caveat kept with the number:* the shell CSV samples at 60 s, so oscillations inside a sampling
interval are invisible and the gross added/freed columns are **lower bounds**. The **net** column is
exact (first sample to last), and the cadence is identical across every run in the table, so the
comparison is like-for-like even though the absolute gross figures are not.

### 3 — Do not call it superlinear

> *"Two data points (37 KB at 1800 s, 646 KB at 14400 s) cannot fit a growth model. You have zero
> power to distinguish O(n), O(n log n), O(n^1.4), or linear-with-offset. … Do not write
> 'superlinear' in the catalog — it's unfalsifiable with n = 2. What it does argue: the growth is not
> decelerating … 646 KB / 4 h = 161 KB/h vs 37 KB / 0.5 h = 74 KB/h — the average rate doubled, which
> is inconsistent with approach to a nearby asymptote."*

**APPLIED.** No growth-class claim is made anywhere. The claim made instead is the one the data
supports: **the average rate roughly doubled (74 → 161 KB/h) between the 1800 s and 4 h runs, which
is inconsistent with approaching a nearby asymptote** — and, independently, the 8 window slopes show
no decay across the 4 h run itself.

### 4 — Decline to re-derive the bound, but say why the estimator is wrong

> *"A last-half OLS slope is a rate detector, not a bound detector. A series approaching a high
> asymptote can have a near-zero last-half slope (false pass) … Your run correctly fails the gate, so
> the gate isn't your problem here, but it will be your problem when someone fixes the prune and the
> series plateaus at 50 KB with a noisy residual slope of ±2000 B/h."*
>
> *"**Do both** … Refusing to re-derive a bound is defensible for now — you can't derive a bound from
> a non-plateauing series. But you should leave a note that once a prune fix lands, the bound must be
> re-derived with a proper estimator. Otherwise the gate gets re-enabled with the same broken
> estimator and you're back here."*

**APPLIED.** R4 is recorded **not-derivable** per §8's own instruction ("If the data supports *no*
shape … R4 is recorded as not-derivable and R5b applies in full"). The estimator finding does not
vanish with it: this spec has *measured* the estimator's inadequacy — two identical width-100 runs
gave slopes 4.6× apart with a second instrument flipping sign, and the gate's verdict is
non-monotonic in width — and that measurement, plus the requirement to re-derive with a **level**
estimator when a prune fix lands, is carried explicitly into the spun-off spec so the gate cannot be
re-armed on the same broken statistic.

### 5 — The cheapest next measurement belongs to the spin-off, not to this spec

> *"**Log prune-fire events from the server during the run.** Even a single boolean per epoch ('did
> prune attempt to run? did it free >0 bytes?') across 458 epochs would distinguish 'never licensed'
> from 'licensed but ineffective' from 'licensed and working but overwhelmed' … 'prune never fires at
> width 1000' vs 'prune fires but frees 1 KB per epoch' are completely different specs."*

**APPLIED, with its premise partly discharged already.** Finding 2 has settled the coarse question
from free data — the prune fires and reclaims 33 % at width 1000 over 4 h — so the remaining question
is the per-epoch one: *how much does each prune pass free, and why does that fraction fall as the
corpus grows?* That is exactly a first task for the spun-off spec, and it is recorded there as such
rather than being done here, because instrumenting the prune path is a `.rs` change and §7.1's R0.6
forbids putting one into this spec's measurement lineage.

---

## Net effect

1. **The catalog wording is horizon-scoped**, not a refutation claim.
2. **The finding's character changed**: from "tombstone bytes are unbounded" to "**the prune fires
   but its reclaim fraction degrades with width and time — 33 % at the production width over 4 h**",
   which is a defect with a mechanism and a testable next step.
3. **No growth-class claim** is made from n = 2 durations.
4. **R4 is not-derivable**, and the estimator's measured inadequacy is handed to the spin-off so the
   gate is not re-armed on a rate detector when a level detector is what the property needs.
