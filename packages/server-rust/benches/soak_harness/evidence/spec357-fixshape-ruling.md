# SPEC-357b — THE FIX-SHAPE RULING (R9, G9 S1)

**WHAT THIS FILE IS.** The artifact R9.1 names: the written answer to *"does the fix land as a
**registry-consumer** (per `.specflow/artifacts/extraction-reclamation-synthesis-2026-08-02.md` §3), or as a
**standalone correctness fix the registry then builds on**?"* — applied under **R9.2's rule, which
`SPEC-357a` froze in `spec356-manifest.md` §11.0.13 before this half's data existed and before the mechanism
was knowable.**

**WHAT THIS FILE RECORDS.**

> ### **THE RULING IS NOT WRITTEN.**
>
> The antecedent that fired is R9.2's third branch — **any `INDETERMINATE-*`**. The published determination
> is **`INDETERMINATE-INSTRUMENT`, cause `CONSERVATION`, on BOTH replicates; the mechanism is NOT NAMED.**
> **No fix shape is proposed here** — not registry-consumer, not standalone correctness fix, and not the two
> shapes R9.3 forbids outright.

**AND THAT IS THE SATISFYING CONTENT OF THIS ARTIFACT, NOT AN ABSENCE.** PRESENCE entry **Q10**
(`spec356-manifest.md` §11.0.15, restated at SPEC-357b R6.4) declares this in its own honest-out-of-band
column, PRE-DATA:

> **Q10** | `spec357-fixshape-ruling.md` | its rule, its antecedent and its reasoning | **an
> `INDETERMINATE-*` outcome makes "the ruling is NOT WRITTEN, because …" the SATISFYING content** (R9.2), not
> an absence | R9.2's frozen antecedent table

This file therefore carries all three things Q10's band requires — **the rule**, **the antecedent** and
**the reasoning** — and stops exactly where R9.2 says to stop.

---

## §1 — THE RULE, QUOTED FROM ITS FROZEN SOURCE

`spec356-manifest.md` **§11.0.13**, authored by `SPEC-357a` PRE-DATA, verbatim:

> #### §11.0.13 — R9.2 / R9.3: the fix-shape ruling rule, and its standing prohibition
>
> **Authored here so the ruling follows a rule frozen before the mechanism is known, rather than the ruler's
> preference wearing a ruling's name. `SPEC-357b` executes this rule; it does not restate it differently.**
>
> - a named **(a)- or (b)-class** mechanism ⇒ **STANDALONE CORRECTNESS FIX, which the `ReclamationRegistry`
>   family then builds on**;
> - a named **(c)-class** mechanism ⇒ **REGISTRY-CONSUMER**;
> - any **`INDETERMINATE-*`** ⇒ **THE RULING IS NOT WRITTEN.** §11.1+ escalates under §11.0.9's template, the
>   obligation returns to `TODO-634` with the round's observations attached, and **no fix shape is
>   proposed.** An `INDETERMINATE-*` outcome makes *"the ruling is NOT WRITTEN, because …"* the SATISFYING
>   content of PRESENCE **Q10** (§11.0.15) — not an absence.
>
> **R9.3, the standing prohibition, authored here because the reclamation-extraction synthesis already
> adjudicated it:** the ruling may propose **neither** the slope stick **nor** the `f(span, width, churn)`
> formula (both discredited — `.specflow/artifacts/extraction-reclamation-synthesis-2026-08-02.md` §7 F3).
> **The gate returns on `ceiling = min_live_claim − fixed_margin`, and nothing else.**

### §1.1 — THE ANTECEDENT TABLE, AND WHICH BRANCH FIRED

| # | Frozen antecedent | Consequent | Fired? | The evaluated value that decides it |
|---|---|---|---|---|
| 1 | a **named (a)- or (b)-class** mechanism | STANDALONE CORRECTNESS FIX, which the registry family then builds on | **NO** | No class was named. R4.3's naming rule was **never reached** — it reads over class shares of `P`, and no hypothesis step was evaluated (Step 0 is fail-closed and failed) |
| 2 | a **named (c)-class** mechanism | REGISTRY-CONSUMER | **NO** | Same. The (c) *counts* exist and are published (439/439 of `P₂`, 357/357), but a count published as R3.4's unconditional reporting limb is **not** a named class — see §3 |
| 3 | any **`INDETERMINATE-*`** | **THE RULING IS NOT WRITTEN** | **YES — THIS ONE** | **`INDETERMINATE-INSTRUMENT`, cause `CONSERVATION`**, on both replicates (`spec357b-walk-r1r2.txt` PART V.0), under R3.0 limb 3 / §11.0.4 limb 3, routed through §11.0.6's fail-closed Step 0 row and §11.0.8 limb 1 |

**The evaluated values behind branch 3**, quoted from the committed walk so this file re-derives nothing:

- r1: **2,484 violations over 2,484 `CONSISTENT` scrapes of 2,880**; the identity holds on **0** of them.
- r2: **2,458 violations over 2,458 `CONSISTENT` scrapes of 2,880**; the identity holds on **0** of them.
- cause **`CONSERVATION`**, not `SAMPLING`: both replicates cleared R3.0 limb 5's 100-`CONSISTENT`-scrape
  collapse bound by more than 24×, so the `SAMPLING` cause was evaluated and did **not** fire.
- Every published violation sits on a scrape whose two renders agree in all eight quantities (0 of 2,484 and
  0 of 2,458 disagreements), so R3.0 limb 4's structural property holds on this round's own data and the
  13.750000 % / 14.652778 % torn fractions did not reach the verdict by any path.

**`DIRECTIONAL` (R4.4a) does not change this branch, and did not apply in the first place.** Its antecedent
requires `INDETERMINATE-MIXED`; the published value is `INDETERMINATE-INSTRUMENT`, so the first conjunct is
false, and no plurality reading exists because no hypothesis step was evaluated. R9.2 states the point
independently: *"A `DIRECTIONAL` label (R4.4a) does not change this branch — the underlying published value
is still `INDETERMINATE-MIXED`, so the ruling is still NOT WRITTEN."* Here the underlying value is not even
`INDETERMINATE-MIXED`, so the branch is reached by the shorter of the two routes.

---

## §2 — WHAT IS NOT PROPOSED, ENUMERATED RATHER THAN LEFT IMPLICIT

**Nothing in this file proposes a fix shape.** Specifically:

1. **NOT registry-consumer.** That consequent requires a named **(c)**-class mechanism. None was named.
2. **NOT a standalone correctness fix.** That consequent requires a named **(a)**- or **(b)**-class
   mechanism. None was named — and both classes measured **0** on both replicates in the reporting bundle,
   which is itself not a naming either way.
3. **NOT the slope stick**, and **NOT `f(span, width, churn)`.** R9.3 forbids both outright, on the
   synthesis's own adjudication (§7 F3: *"the bound formula KILLED … f(span, width, churn) with global churn
   is a false-precision machine (hot-key skew makes per-partition peak 10-100× average). Replaced: **ceiling
   = min_live_claim − fixed_margin** (one honest, empirically-validated knob, default 1-2 epoch widths)"*).
   This file does not propose them **and would not have been permitted to even on a named mechanism** — the
   prohibition is standing, not contingent on the determination.
4. **NOT a repair of the frozen predicate.** O-0's identity is mis-specified relative to the transport this
   round could scrape (PD-F7, PD-F8, and G8's applied correction to PD-F8's attribution). **This half may
   not edit `§11.0`**, and it does not propose the successor's replacement identity as a ruling either —
   the *observation* is routed to `TODO-634`; choosing the successor's predicate is that spec's PRE-DATA
   decision, not this one's POST-DATA output.
5. **NOT any part of the `ReclamationRegistry` family.** R9.4 scopes a ruling to *where the fix belongs and
   why*; this file does not reach even that, so it certainly designs no registry, SLA, fence, quarantine,
   sweep or gate estimator.

---

## §3 — THE REASONING, AGAINST SYNTHESIS §3: WHY NAMING THE SHAPE NOW WOULD BE UNSOUND

R9.2's third branch is a rule, and this half obeys rules it did not choose. But Q10's band asks for the
**reasoning**, and the reasoning matters here for a specific reason: **the data looks, at first glance, as
though it names a shape.** One discriminating class — **(c) FRONTIER RACE** — holds **439/439 = 100.000000 %
of `P₂` and 439/440 = 99.772727 % of `P`** on r1, and **357/357 = 100.000000 % of `P₂` and 357/358 =
99.720670 % of `P`** on r2, concordant across replicates, independently re-derived with **0 mismatches over
899 (r1) and 822 (r2)** classifier rows. Synthesis §3's registry-consumer branch is exactly the branch a
named **(c)** would select. A reader could ask why the ruling is not simply written.

**It is not written because the one class that dominated is, on this round's own evidence, an instrument
artefact rather than a mechanism.**

### §3.1 — The (c) concentration is STRUCTURAL, established at the source by G8

`spec357-mechanism-xask.md` X10 and the finding it opened, **PD-F14 limb (a)**, establish this mechanically
rather than by inference, and it is the strongest single result of the cross-vendor round:

- `drain_prunable` calls `refresh_epoch_licensing` at its **top** (`tombstone_frontier_impl.rs:855`), which
  stamps `fence_passed_at_op_seq = self.op_seq` for any tracked epoch whose fence **already holds**;
- the **same call** then removes the eligible epochs' tags and emits their exit rows with
  `exited_at_op_seq = self.op_seq` (`:708`);
- `op_seq` **does not advance inside a pass** — its sole increment is in `stamp_tombstone` (`:461`).

**Therefore any epoch drained in the same pass in which its fence is first observed true carries
`fence_passed_at_op_seq == exited_at_op_seq` NECESSARILY.** Measured: that equality holds on **439/439** and
**357/357** window `P₂` rows, and on **897/897** and **820/820** whole-cell rows. With
`RESIDENT(e) = [entered_at_op_seq, exited_at_op_seq)` **half-open**, the three-way intersection
`T = RESIDENT ∩ LICENSED ∩ FENCED` is then empty **by exactly one op-seq, for that reason alone** — and
`(c) ≡ ¬T` is precisely the rule that put every one of those rows in class (c).

**PD-F13 limb (b) is what makes this a specific claim rather than a general worry.** The companion stamp
behaves entirely differently: `lwm_passed_at_op_seq == exited_at_op_seq` on **0/897** and **0/820** rows,
with `exited − lwm_passed` at min/median/max **238 / 637 / 1,051** (r1) and **151 / 581 / 1,129** (r2). So
the "all three windows trivially collapse onto one instant" reading is measured **FALSE**: `LICENSED` opens
hundreds of op-seqs earlier, and **the fence is the sole term landing on the exit instant** — which is
exactly the term a pass-granular detection stamp would put there.

**And the classification is uniform across a boundary at which the drain's behaviour is not** (X16, X14 /
PD-F14 limb (b)): 100 % of the discriminating rows are (c) in **both** halves of **both** cells, while the
drain counters change completely across that same boundary — whole-cell `nonempty_drains_total` runs
**0 → 35** (r1) and **0 → 99** (r2) with **all** of it accumulating at or before 14,400 s, and **Δ0** over
the deciding window. A discriminant that reads identically on a population where the subject's behaviour
demonstrably changed is tracking the **stamping convention**, not the drain.

**The conclusion this half draws, and the one it refuses.** It draws: *a discriminant that a pass-granular
licensing stamp plus a half-open interval convention can manufacture on its own does not carry the system
property it names.* It refuses: any reading of the above as a re-classification. The frozen definitions of
`RESIDENT`, `T` and the six classes are **untouched**, the published counts stand exactly as the bundle
published them, and PD-F14 is a **predicate-sensitivity** record, `RECORDED-AND-ROUTED`, nothing else.

### §3.2 — Writing the ruling on that reading would select synthesis §3's branch on an artefact

Synthesis §3 says, of the registry:

> with a registry, prune is registry-gated batch work with a derivable ceiling, not per-op piggyback racing
> accumulation. The mechanism investigation stays (why does the fraction fall?), but the fix lands as
> registry-consumer, not as a bespoke prune accelerator.

That branch is the right one **if** the defect is in the claim/licensing arithmetic — the LWM consuming
epochs as fast as it licenses them — because that arithmetic is precisely what `ReclamationRegistry`
replaces (per-partition claims, boot floor = persisted checkpoint never head, strictly monotonic prune
watermark, two-phase expiry; synthesis §7 F2). **It is the wrong branch if the observed (c) uniformity is a
recording convention**, because then the fix would be aimed at arithmetic the data never actually indicted,
while the real defect — whatever it is — survives the registry untouched and re-reds the gate. That is
exactly the failure the user's diagnostic-first ruling of 2026-08-03 exists to prevent: *building 5+ registry
specs before the mechanism is named risks a wrong-shaped fix that still reds the gate.*

**And there is a second, independent reason not to write it: the round's own most important observation is
an unresolved contradiction about whether the prune removes anything at all.** PD-F12, sharpened by G8's X9
and PD-F14 limb (c): over the deciding window every genuinely independent prune-path counter is **frozen**
(`nonempty_drains_total`, `considered_total`, `dropped_total`, `bytes_freed_total`, `epochs_drained_total`
all **Δ0**, while `passes_total` and `empty_drains_total` advance by **+440,069** / **+358,370** and the
store's tombstone bytes **grow** by **+10,131,707 B** / **+8,250,744 B**) — and over the **same** window the
new per-epoch exit ledger records **439** / **357** exits, **all** `DrainedByPrune`, attributing
**10,097,000 B** / **8,211,000 B** freed. **Both readings cannot describe the same events.** The round
publishes three candidate readings and picks none, because picking one **would be a naming** and Step 0 is
fail-closed. A ruling written on top of an unreconciled contradiction about the subject's most basic
behaviour would be a preference wearing a ruling's name — the precise thing R9.2 was frozen to prevent.

**Also load-bearing: the T2(exactness) number that looks like corroboration is not one.** The 0-byte
exactness bound over 439/439 and 357/357 rows is exact **by construction** —
`bytes_freed_attributed := slot.stamped_bytes` (`tombstone_frontier_impl.rs:693-698`), so the two compared
quantities are the same field — and the companion credit `drained_refs_total += slot.refs_at_entry`
(`:669`) is likewise an entry-side copy. **PD-F12 limb (a) is published beside the number rather than
suppressed**, and it is one more reason the round has less independent purchase on the drain's behaviour
than the class table's tidiness suggests.

### §3.3 — What the argument is NOT

- It is **not** an argument that hypothesis (c) is false. **(c) is neither endorsed nor excluded** — no
  hypothesis step was evaluated at all. What is refuted is the *claim that the data names it* (xask X19,
  H-c.), not the hypothesis.
- It is **not** a back-door naming of some other class. (a) and (b) measured **0**, and a zero published as
  a reporting limb is not a naming either.
- It is **not** a relaxation of the gate. A pre-registered gate that can be relaxed after its data exists is
  not a gate (xask X6). The successor may **scope** the fail-closed rule before its own freeze; this round
  may not, and did not.

---

## §4 — WHERE THE OBLIGATION GOES

**The ruling obligation returns to `TODO-634`** — the `ReclamationRegistry` family umbrella, and the owner
R9.2 names — **with this round's observations attached.** The attached package, in the priority the
cross-vendor round argued for (X17, X20) rather than in the order the round happened to produce it:

| # | Attached observation | Why it is the successor's input |
|---|---|---|
| 1 | **PD-F12** — the prune-path counters and the exit ledger contradict each other over the deciding window; three candidate readings, none picked | The successor's **FIRST** question: *is the prune removing anything at all?* Its three readings are mechanism hypotheses, not instrument notes |
| 2 | **PD-F14 / PD-F13 / PD-F11** — the fence/exit coincidence is structural; the (c)/(d) split rests on one op-seq at a half-open boundary; the two licensing stamps have different characters | The successor's **SECOND** question: *is the fence stamp a transition or a detection?* Record the instant a licensing condition **becomes true** (or use a closed interval, or both), and state which convention the (c)/(d) split rests on **before** freezing |
| 3 | **PD-F8** (with G8's applied correction) and **PD-F7** — O-0's named subject has no production exporter; the identity is unsatisfiable over the only transport the round had | The successor's O-0 must be evaluated over the snapshot accessor (export it, or write the ledger server-side), **or** carry an explicit open-epoch term — the wider fix set the corrected, predicate-authoring attribution opens (X4) |
| 4 | **X21's named rule** — three instances (PD-F5, PD-F8, PD-F9) of one defect class | *Every frozen surface — predicate, tool and join — is validated against the exact transport the DATA half will consume, before the boundary.* Three instances in one round is a pattern |
| 5 | **PD-F9** — the pinned join's `cell_start_unix_ms` exists in neither named artifact, and `entered_at_unix_ms` is 0 exactly on `P₀` | A latent **vacuous-limb** hazard: under a strong hypothesis (a), `P₀` would fall outside the window by construction and (a) would be unreachable. Zero blast radius measured this round; key the successor's join on a field populated on every entry row |
| 6 | **The torn-fraction sampling finding** — 396/2,880 = 13.750000 % (r1), 422/2,880 = 14.652778 % (r2), both above R3.0 limb 4's 2 % routing threshold | Routed **without touching the verdict**, which tearing structurally cannot reach. The successor's scrape needs an atomic multi-quantity endpoint or more than one retry |
| 7 | **PD-F3 / PD-F14 limb (b)** — a third and a fourth non-replication at n = 2 (RSS slope; 35 vs 99 non-empty drains, 2.83×) | Direct input for whoever sizes the replicate count against R4.3's own n = 2 concordance bar |
| 8 | **X1 / X2 / X6** — the limb's label is a category error; `CONSERVATION` should split into `-GAUGE` and `-SPECIFICATION`; the fail-closed rule is unscoped | Successor-**predicate** design inputs. `RECORDED-AND-ROUTED` is the only available disposition: the frozen antecedent fired as written and relabelling it would be a post-boundary predicate edit |

**The tracker byte that carries this routing is written by this half's segment 2 under R5.7** — this round
does not leave the routing to a manifest section. `TODO-634` is **updated, not closed and not ticked**.

---

## §5 — CELL E's DISPOSITION INPUT

R4.6(i)'s third limb, quoted from its frozen source (`spec356-manifest.md` §11.0.12(i)):

> - any **`INDETERMINATE-*`** outcome ⇒ Cell E is **`DEFERRED-PENDING-DIAGNOSIS`** — an explicit third
>   branch, **taken rather than un-taken**, with `TODO-634` as its ownership statement.

**ANTECEDENT AS DETERMINED THIS ROUND:** `INDETERMINATE-INSTRUMENT`, cause `CONSERVATION`, both replicates
— an `INDETERMINATE-*`.

⇒ **CELL E: `DEFERRED-PENDING-DIAGNOSIS`**, which is also the value of R4.6(ii)'s successor
`CELLE_DISPOSITION` enum (of `RUN-PER-355-4.6` | `CLOSED-NOT-NEEDED` | `DEFERRED-PENDING-DIAGNOSIS` |
`NOT-FIRED-DETERMINATION-INDETERMINATE`). **Ownership: `TODO-634`.**

**The fourth enum value is NOT the one here.** `NOT-FIRED-DETERMINATION-INDETERMINATE` would record a branch
left un-taken; R4.6(i)'s third limb fires **explicitly** on an `INDETERMINATE-*`, so the branch was **TAKEN**.
**Cell E is not run by this half and no pre-346 binary is built** — a disposition is not a run. **The
MANIFEST's own §9 Checklist 16 keeps its by-construction RED and is not retroactively greened.**

The disposition itself is **recorded** in `spec356-manifest.md` §11.13; this file supplies it as the
ruling's own input, so a reader of the ruling does not have to leave it to find the branch that was taken.

---

## §6 — WHAT THIS OUTCOME COSTS, AND WHAT IT DOES NOT

Quoted verbatim, as Q15 requires wherever any `INDETERMINATE-*` is reported —
`spec356-manifest.md` §8.3:

> ### §8.3 — And the family is NOT blocked by it, which is why this is a legitimate terminal branch
>
> **The recommended reclamation model closes safety REGARDLESS of which cause it turns out to be.**
> `ReclamationRegistry` (cursor-shaped consumers only) + retention SLA **N = 30 d** + the cursor-age fence
> with HLC-horizon quarantine + `ceiling = min_live_claim − fixed_margin` bound the reclaimable set by **live
> claims**, not by any hypothesis about *why* the current prune falls behind. **A selection defect, a
> scheduling defect and a throughput defect are all *contained* by a registry that never reclaims below a
> live claim.**
>
> What an unclassified cause costs is **fix-shape efficiency** — the family would design without knowing
> which limb to optimize first — **not safety, and not the family's ability to proceed.** A Step-5 outcome is
> therefore to be read as **an expensive answer, not a blocked one**, and any Step-5 outcome must be reported
> quoting this paragraph beside it.

**Read against this round, without softening:** the family is not blocked, and the gate this diagnosis feeds
returns on `ceiling = min_live_claim − fixed_margin` regardless of which limb the successor eventually
optimizes first. What is genuinely lost is one 16 h measurement pair's worth of fix-shape efficiency, plus
the design-phase cost of the eight routed items in §4. What is genuinely gained is that the successor now
knows **which of its own instruments cannot be trusted** — the identity's transport (PD-F8), the exactness
oracle (PD-F12 limb (a)), the licensing stamps' granularity (PD-F13, PD-F14) and the join's key (PD-F9) —
which is not nothing, and is the reason the round is an expensive answer rather than a blocked one.

---

**END — SPEC-357b, R9 / G9 S1. The fix-shape ruling is NOT WRITTEN; the antecedent that fired is R9.2's
third branch (`any INDETERMINATE-*`); no fix shape is proposed, and neither the slope stick nor
`f(span, width, churn)` is proposed under any reading. The obligation returns to `TODO-634` with the
observations of §4 attached. Cell E: `DEFERRED-PENDING-DIAGNOSIS`.**
