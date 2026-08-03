# SPEC-355 — `/xask` cross-vendor second opinion on the row-5 interpretation fork

**Model:** `z-ai/glm-5.2` via OpenRouter · **Cost:** $0.0187 (prompt 1119 / completion 4391 tokens)
**Run at:** after cells A–D, before R0.4's step-2 width sweep and before any branch is taken.

**Why this gate fired here.** The standing cross-vendor rule applies at every
verification/implementation-choice point, and §8's R4.4 makes it explicit that the obligation is
**not branch-(2)-only** — naming a culprit (R5) or declaring unboundedness (R5b) are decision points
of the same consequence. The fork put to the model is the one that decides between them: is cell B
vs cell D a **regression**, or a **window-placement artifact on a bounded oscillating series**?

The prompt was framed adversarially — the model was asked to **refute** the regression reading, not
to confirm it — and was given both instruments' figures, both standard errors, the r² values, the
per-run level statistics, and the identity-witness provenance.

---

## Adjudication summary

| # | Finding | Verdict | Where applied |
|---|---|---|---|
| A | I cherry-picked `peakBytes` and suppressed `lastBytes` | **APPLIED — I was wrong** | §10.4.1 |
| B | The width-100 slopes carry no usable signal; both are ~1 SE from zero | **APPLIED** | §10.4.2 |
| C | Replication cannot resolve an effect 10× below the noise floor (k ≈ 784) | **APPLIED, with a scope correction** | §10.4.3 |
| D | The last-half-slope estimator is wrong for a bounded oscillating series | **APPLIED as an R4.1 input** | §10.4.4 |
| E1 | The width-1000 row is the dominant fact and is under-weighted | **APPLIED** | §10.4.5 |
| E2 | SPEC-345's width-100 PASS may itself have been noise-driven | **APPLIED — the sharpest finding here** | §10.4.5 |
| E3 | Warm-up may be mixed into the "steady-state" window; 1800 s steady state not shown | **APPLIED** | §10.4.3 |
| E4 | "Identical workload matrix" is asserted, not demonstrated; no seeding | **PARTIALLY APPLIED** | below |
| E5 | One run per cell ⇒ no cell-level variance estimate | **APPLIED** | §10.4.2 |
| E6 | The WAL census proves binary identity, not comparable path coverage | **APPLIED** | below |
| E7 | Cell A is 3600 s while B/C/D are 1800 s — "confirm whether this is a protocol violation" | **REFUTED — already recorded** | below |
| E8 | A width-agnostic B/h bound cannot be right at both widths | **CONCURS with the pre-registration** | below |

### Verification of the model's arithmetic against the committed artifacts

Every quantitative claim it made was recomputed from this spec's own artifacts before being
accepted. All of them check out:

| Claim | Recomputed | Agrees? |
|---|---|---|
| A leak of 1524.96 B/h over 0.5 h ≈ the observed peak gap | 762 B vs 690 B observed | ✓ |
| `lastBytes` gap is the larger, suppressed figure | 24,134 − 18,986 = **5,148 B** | ✓ |
| Cell B is 0.56 SE from zero; cell D is −1.15 SE | 7,938.86/14,268 = 0.556; −16,797/14,581 = −1.152 | ✓ |
| B and D are within ~1 SE of **each other** | diff 24,736 B/h, SE(diff) 20,401 → **1.21 SE** | ✓ |
| k ≈ 780 runs to push SE(mean) under the bound | (14,000/500)² = **784** | ✓ |

---

## Findings, in the model's own terms, with this spec's disposition

### A — `peakBytes` was the wrong statistic, and the presentation was selection-biased

> *"A slow linear leak of +1524 B/h over a 1800s (0.5h) window accumulates ~762 B. The peak gap you
> cite is 690 B. So a leak of the size the in-process slope reports would predict roughly the peak
> gap you observe… Worse, peak is a single order statistic — maximally phase- and noise-sensitive —
> and you've suppressed the more damning figure you also report: lastBytes 24134 vs 18986, a 5148 B
> gap… You're using peakBytes to argue (2) while sitting on lastBytes that argues (1). That's
> selection bias in the presentation, not just in the statistic."*

**APPLIED. This is a correction to a claim already committed** (cell D's commit message argues from
`peakBytes` 2.5 % near-identity toward the artifact reading). The argument does not survive: a leak
of exactly the reported magnitude *predicts* a peak gap of that size, so `peakBytes` near-identity
is **not** evidence for the artifact reading, and `lastBytes` — 5,148 B apart, ~27 % of the level —
points the other way. Both statistics are now reported together in §10.4.1 with neither leading.

### B — The width-100 cells carry no usable slope signal at all

> *"Both shell slopes at width 100 are within ~1 SE of zero… and within ~1 SE of each other.
> Reporting them to two decimals (±7938.86) is false precision. The 5.2× 'disagreement' is a ratio
> of two noise-dominated estimates; it's not a calibration failure, it's that there is no signal to
> calibrate against… The width-100 row of your 2x2 is essentially uninformative about slope. n=1 is
> a secondary problem; the primary problem is that the effect size you're hunting is ~10× below your
> noise floor at this duration."*

**APPLIED.** This is a sharper diagnosis than §10.2.2's "statistical power, not disagreement" — it
is correct as far as it went but stopped short of the consequence, which is that the band assignment
for `S100` rests on a number statistically indistinguishable from zero. §10.4.2 records this. Note
it **does not overturn the row-5 determination**: row 5 is where an underdetermined `S100` is
*supposed* to land, so the pre-registered rule behaved correctly. It overturns any temptation to
read cell B as a positive finding.

### C — Replication is the weakest of the available follow-ups

> *"To separate a true +1500 B/h leak from zero via 1800s-run slope scatter, with per-run SD ≈ 14000
> B/h, you'd need k ≈ 780 runs… Slope scatter across k short runs is the weakest of these — it can
> show you the noise floor but cannot resolve an effect 10× below it."* Recommends instead: much
> longer runs (≥6 h) at width 100 on both binaries; **compare equilibrium level, not slope**; look
> for a duration × binary interaction.

**APPLIED, with one scope correction that is a genuine disagreement.** The correction: the width-100
repeat (`sweep100`, already running when this answer arrived) was never intended to *resolve the
fork by averaging* — 784 runs is obviously not on the table. It is intended to **measure the noise
floor and test whether the gate's PASS/FAIL verdict at width 100 is stable across two runs of an
identical configuration**, which is precisely the evidence finding E2 needs and which the model
itself grants a k-run scatter can supply. One repeat is cheap, is already pre-registered as R0.4
step 2's first leg, and answers a different question from the one the model priced at 784 runs.

The substantive recommendation — **level, not slope; longer runs** — is adopted, and it is already
in the pre-registration rather than being bolted on after the fact: §7.3(b)'s width-scaling test is
stated over `peakBytes`/`lastBytes` **equilibrium levels**, and §7.2's R3.1 run is ≥ 4 h precisely
so the window sits past the ramp. R0.4 step 2's rule tests "the three slopes **and equilibria**"
against that prediction.

### D — The 512 B/h last-half-slope gate is defensibly the wrong estimator

> *"Slope of a bounded oscillating series over a window that doesn't cover many full periods is
> biased by where the window lands in the phase… The gate is testing the wrong null: it's asking 'is
> there a nonzero slope' when it should be asking 'is the series unbounded.'"* Recommends a
> **two-window level comparison** (mean of the last 25 % vs the preceding 25 %, gated on the
> difference plus an absolute cap), after first measuring prune-cycle period, oscillation amplitude
> and equilibrium level at width 1000.

**APPLIED as an R4.1 input, not as a decision.** This is an independent argument for §8's candidate
shape **(iii) a residency-ceiling clause**, and for the measurement programme §7.3 already
pre-registers. It is recorded here so that when R4.1 chooses a shape, the choice can cite an
adversarial external argument for it rather than resting only on this spec's own prior (Assumption 8
favours shape (ii)). **It is explicitly not licence to pick a shape now:** R4.1 requires the choice
be made from §7's data, and that data does not exist yet.

### E1/E2 — The dominant fact, and the sharpest finding in the whole review

> *"The width-1000 row is the dominant fact and you're under-weighting it. Both binaries fail there
> with r²=0.79 and slopes of 175–248 KB/h — real trend, not oscillation… The 'earlier spec passed at
> width 100' may itself have been a noise-driven PASS, since 512 B/h is below the noise floor you've
> just measured. That's a process failure worth flagging independently of the fork."*

**BOTH APPLIED.** E2 in particular reframes this spec's premise: SPEC-345's −1707.5 B/h control, cell
D's −791.28 B/h and cell B's +1,524.96 B/h may all be draws from the same zero-centred distribution,
in which case the gate's width-100 verdict is close to a coin flip and **the second disjunct was
never demonstrated at width 100 either** — a stronger statement than the spec's premise, which
assumed the width-100 PASS was solid and only the width-1000 extrapolation was unverified. This is
recorded in §10.4.5 and is directly testable by `sweep100`.

### E3 — Steady state at 1800 s is assumed, not shown

**APPLIED.** Recorded in §10.4.3 as a named limitation of every 1800 s cell, and it is one more
reason the ≥ 4 h R3.1 run is load-bearing rather than a formality.

### E4 — "Identical workload matrix" asserted, not demonstrated; no seeding

**PARTIALLY APPLIED.** The *configuration* half is demonstrable and is stronger than the model could
see from the prompt: every rate/shape knob is a **literal in a committed script**, and the difference
against the parent runner is enumerable by `diff spec349c2-plateau.sh spec355-width.sh` (AC1). The
*stochastic* half is conceded: the harness's client behaviour is **not seeded**, so run-to-run
variation is real and unquantified at n = 1 per cell. That is the same gap E5 names, and `sweep100`
is the first measurement of it.

### E5 — One run per cell ⇒ no cell-level variance

**APPLIED.** Any "essentially the same" claim across cells is unfalsifiable from n = 1, which is
exactly the defect in the `peakBytes` argument that finding A demolished.

### E6 — The WAL census proves binary identity, not comparable path coverage

**APPLIED as a real limitation.** The census (`orDeltaFrames == 0`) is a **provenance** witness and
this spec has never claimed more for it. It does not establish that the OR/prune path was exercised
*comparably* between the HEAD and pre-family cells. Ancillary support that it was exercised at all:
`orSnapshotFrames` is 2,650 (cell D) / 41,509 (cell C) / and `orDeltaFrames` 3,406 on HEAD's cell B —
all non-trivial — and `lastConfirmedEpoch` reached 600 / 59 / 600 respectively. That is coverage
evidence, not coverage *equivalence*, and the distinction is now stated rather than blurred.

### E7 — "Cell A is 3600 s while B/C/D are 1800 s: confirm whether this is a protocol violation"

**REFUTED — already recorded, and by design.** Cell A **is** 3600 s: it is SPEC-349c2's committed
evidence, carried as an input rather than re-run (§4.4). The asymmetry is not a discovery and not a
violation; it is the **first row of §3.2's confound ledger**, written before any measurement, and it
is the stated reason cell C is read as an order-of-magnitude **binary** ("reproduces / does not")
rather than as a numeric comparison against A. The model could not see this from the prompt, which
did not include the ledger.

### E8 — A width-agnostic B/h bound cannot be right at both widths

**CONCURS with the pre-registration.** This is §8's candidate shape **(i)**, frozen before any run.
Recorded as external corroboration, not as a new finding.

---

## Net effect on this spec

1. **The row-5 determination stands and is unaffected.** Every finding above sharpens *why* `S100`
   is underdetermined; none moves it into another band. The pre-registered rule did its job.
2. **One committed claim is corrected** (finding A): the `peakBytes` near-identity argument is
   withdrawn, and `lastBytes` is reported beside it.
3. **The centre of gravity moves to the width-1000 row**, where both binaries breach with a real,
   well-resolved trend (r² 0.79) — a finding neither fork's framing was foregrounding.
4. **A new, independently-consequential hypothesis is on the record** (finding E2): the gate's
   width-100 verdict may be noise-dominated, which would mean SPEC-345's positive control never
   demonstrated what it was read as demonstrating. `sweep100` is its first test.
