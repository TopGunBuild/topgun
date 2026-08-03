# SPEC-355 — `/xask` cross-vendor check on how R0.4 step 2's rule should be applied

**Model:** `z-ai/glm-5.2` via OpenRouter · **Cost:** $0.0159 (prompt 1446 / completion 6110 tokens)
**Run at:** after the R0.4 step-2 width sweep completed, **before** any branch was declared.

**Why this gate fired here.** R0.4 step 2's rule returned a literal verdict I had an obvious
incentive to dislike, and I proposed a reading that avoided it. That is exactly the situation the
standing cross-vendor rule exists for, so the prompt **named my incentive explicitly** and asked the
model to attack my reading rather than evaluate it neutrally.

**Outcome: my proposed reading was substantially rejected, and I accepted the rejection.** The
disposition below is not a formality — it changed the next measurement.

---

## What the rule says, and what it literally returns

> *"…a width sweep at a single fixed duration — widths 100 / 300 / 1000 on the **HEAD** binary,
> duration ≥ 1800 s each — tested against the pre-registered width-scaling prediction of R3.2. If
> the three slopes and equilibria are consistent with that prediction, the determination is **(2)**;
> if not, it is **(1)**."*

§7.3(b)'s prediction is residency ∝ width (≈10× from width 100 to 1000). Measured on HEAD:
**1.06×** for a 3× width change and **1.95×** for a 10× change. **Not consistent.** Read literally,
the rule returns **(1) — regression → bisect.**

## The reading I proposed, and why it was rejected

I argued the rule **had not fired in either direction**, because two of the three inputs it names do
not exist at 1800 s: the slopes carry no signal at widths 100/300, and the width-1000 *equilibrium*
is not reached. I proposed to obtain the missing input via the ≥4 h R3.1 run.

> *"**Mostly goalpost-moving, with one defensible crack.** … You are now proposing to treat the '≥'
> as 'run until you have good data,' which is a retroactive reinterpretation of a floor condition as
> a quality gate. … You set the duration. You pre-registered it knowing your own prediction said
> ramp scales 10× with width. If 1800 s was too short for width 1000, that is a defect you wrote
> into the rule, and you do not get to exploit your own defect to avoid a verdict you dislike."*
>
> *"You wrote a prediction saying ramp scales ~10×, then set the duration at the boundary where
> width 1000 might not have equilibrated, then discovered it didn't, and now want to use the failure
> of your own experimental design as a reason to keep measuring. That is the textbook shape of
> rationalization."*

**ACCEPTED. The "inputs unavailable" argument is withdrawn.** It is a floor condition being
retroactively reread as a quality gate, and the design defect it exploits is one I wrote myself.

## The gap that IS real — and it is not the one I argued

> *"The rule's step 2 tests **HEAD's** slopes and equilibria against the prediction. It never asks
> whether **pre-family** also violates the prediction. Your pre-family width-1000 data also
> massively violates the 10×-scaling prediction … So the prediction was wrong **before any
> regression**. The rule assumes the prediction is a valid discriminator … But if the prediction
> fails on the pre-regression baseline too, then 'inconsistent' diagnoses **'model is wrong'**, not
> 'regression landed.' … This is not a convenient third option you invented. It's a coverage failure
> in the rule."*
>
> *"**However:** the strength of this argument rests entirely on the pre-family width-1000 data,
> which the rule did not ask you to collect. … The pre-family comparison is what saves you. The
> 'inputs unavailable' argument does not."*

**Verified against this spec's own committed artifacts before acceptance:**

| Binary | width-100 level | width-1000 level | ratio | predicted |
|---|---|---|---|---|
| HEAD | 19,863 (n=2) | 38,715 | **1.95×** | ~10× |
| **pre-family** | 20,771 | 56,898 | **2.74×** | ~10× |

The prediction fails on **both** binaries. §7.3(b)'s model is wrong **universally**, not as a
HEAD-specific symptom — so "inconsistent with the prediction" cannot discriminate a regression here,
which is precisely what R0.4 step 2 assumes it does.

## The number I had been under-weighting

> *"Width 1000: HEAD 38715, pre-family 56898. Pre-family is **47 % worse** than HEAD. This is the
> single most important number in your dataset, and it points **away** from regression: the baseline
> is worse than HEAD."*

Verified: level **56,898 / 38,715 = 1.47×**; slope **175,367.59 / 87,413.90 = 2.01×**. At the
production width, on both statistics, **the OLD binary is worse than HEAD.**

## What it would declare, and its own strongest counter-argument

> *"**Branch (2), no regression, ~60 % confidence.** … **Strongest argument against my own choice:**
> The literal rule fired. … A purist reading says (1), and I am choosing to override the literal text
> of a pre-registered rule using a logical-gap argument that, while valid, is convenient for the
> outcome I prefer. If I'm wrong about the model being universally wrong (e.g. if pre-family's poor
> width-1000 performance is itself an artifact of non-equilibrium), my branch (2) call rests on a
> transient artifact."*

## The recommendation that changed the next measurement

> *"**Not the 4 h width-1000 run. Something cheaper and more discriminating.** The 4 h run gives you
> equilibria, but equilibria for a model that's already demonstrably wrong. That doesn't resolve
> regression — it refines a model violation you already know about. … The question that actually
> matters is: **is HEAD worse than pre-family at width 1000?** You have one unreplicated comparison
> that says no."*
>
> Recommended: replicate width-1000 **HEAD**, replicate width-1000 **pre-family**, get n = 2 vs 2
> and a real variance estimate; only then consider the long run, and run it for **both** binaries,
> "because the discrimination comes from the comparison, not from the absolute equilibrium."

**ACCEPTED, and acted on.** The ≥4 h R3.1 run had already been started when this answer arrived; it
was **stopped ~15 minutes in and its partial artifacts deleted** (no partial series is committed),
and the two width-1000 replicates were run instead. The reasoning is sound and it corrects a real
ordering error on my part: the branch determination gates everything, R3 is **branch-(2)-only**, and
running R3's characterization before the branch is settled risks 4 h of work for a branch that does
not apply.

Two additive cells (`cellC2`, `sweep1000b`) were added to `spec355-width.sh` for the replicates.
The edit is **provably additive** — `git diff` shows **zero removed lines**, so no existing cell's
literals moved and no committed run's provenance is disturbed.

**One point where the paired design is stronger than the model credits:** a paired HEAD-vs-pre-family
comparison does **not** require either arm to be at equilibrium. It requires both arms at the *same
point on their ramp*, which duration-matching supplies by construction. That is why replicating at
1800 s is the discriminating experiment and a longer run is not.

---

## The deviation, stated plainly (adapted from the model's own suggested framing)

> **The pre-registered rule fired, and its literal verdict is (1).** I am not following it, and the
> reason is not that its inputs were unavailable — that argument is withdrawn as rationalization.
> The reason is that **R0.4 step 2 has a coverage gap**: it tests HEAD against a model that also
> fails on the pre-family baseline, so "inconsistent with the prediction" diagnoses *the model is
> wrong*, not *a regression landed*. The rule's dichotomy has no branch for that case, and this
> spec's data landed in it.
>
> **Falsification condition, fixed in advance of the runs:** I am collecting replication at width
> 1000 on both binaries. **If HEAD remains no worse than pre-family across n = 2 vs 2, I declare (2)
> on the ground that the rule's dichotomy is unsound for this case. If HEAD is worse, I accept (1)
> and bisect.**

This deviation is recorded here, in the manifest's determination section, and in the commit that
carries the replicates — never as a silent re-reading of the rule.
