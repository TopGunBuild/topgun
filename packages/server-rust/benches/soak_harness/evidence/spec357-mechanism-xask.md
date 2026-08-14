# SPEC-357b — the CROSS-VENDOR ADVERSARIAL ROUND at the mechanism-naming point (R8, G8)

**WHAT THIS FILE IS.** The cross-vendor round R8.1 places at the moment this half declares which of
(a)/(b)/(c) — or an `INDETERMINATE-*` limb — is the answer. It ran **AFTER** the determination (G7 S2,
committed as `spec357b-walk-r1r2.txt`) and **BEFORE** the fix-shape ruling (G9, not yet written). It is an
**advisory** round: the consultant is a second vendor's model, this half remains the source of truth, and
**no finding here changes the published determination.**

**WHAT THIS ROUND DID NOT DO, stated up front because the constraints are categorical.** It wrote **no
`.rs` byte**, **no byte of `spec356-manifest.md`** (neither `§11.0`, which is `SPEC-357a`'s frozen PRE-DATA
surface, nor `§11.1+`, which is G9's), **no byte of any pinned sidecar**, and **no `.specflow/todos/` byte**
(also G9's). It **did not re-walk the predicate**, **did not re-run a builder**, and **did not move a
determination**. The published value is, and remains:

> **`INDETERMINATE-INSTRUMENT`, cause `CONSERVATION`, on BOTH replicates. THE MECHANISM IS NOT NAMED.**

**The disposition rule this file obeys (R8.3).** Every finding carries exactly one of `APPLIED`,
`REFUTED-WITH-REASON`, `RECORDED-AND-ROUTED`. **After the data boundary, `RECORDED-AND-ROUTED` is the ONLY
available disposition for a finding against the frozen predicate** — `APPLIED` may not touch a frozen
surface, and is used below only where a finding was a *check to run* or a correction to **this half's own
un-frozen POST-DATA register**.

---

## §1 — THE INVOCATION (R8.2, R8.3)

**Model.** `z-ai/glm-5.2` (the script's default; open-weights, 1M context), via OpenRouter
chat-completions.

**Script, absolute path, as R8.2 requires.** `~/Projects/agent-future/scripts/openrouter-ask.sh` —
i.e. `/Users/koristuvac/Projects/agent-future/scripts/openrouter-ask.sh`.

**Invocation, verbatim (run 2, the one this file is written against):**

```bash
PROMPT="$(cat <the question, reproduced in §3>)" \
OPENROUTER_STYLE=full OPENROUTER_REASONING=high \
OPENROUTER_MAX_TOKENS=24000 OPENROUTER_TIMEOUT=560 \
  bash /Users/koristuvac/Projects/agent-future/scripts/openrouter-ask.sh \
    packages/server-rust/benches/soak_harness/evidence/spec357b-walk-r1r2.txt \
    target/spec357-scratch/POST-DATA-FINDINGS.md \
    <scratch>/frozen-predicate-11.0.md
```

**Two runs, and the first one is reported rather than hidden.**

| run | `OPENROUTER_MAX_TOKENS` | outcome | prompt / completion tokens | cost |
|---|---|---|---|---|
| 1 | 9,000 | **TRUNCATED** — the script's own `[warn] ANSWER TRUNCATED at max_tokens (finish_reason=length)`; the answer stopped mid-axis A5. With `OPENROUTER_REASONING=high` the reasoning tokens count against the same budget | 48,241 / 9,000 | **$0.020944777** |
| 2 | 24,000 | **COMPLETE** — all seven axes plus four additional findings and a summary | 48,241 / 15,269 | **$0.030791754** |

**Total cost of the round: $0.051736531.** Run 1 was not a transport failure (the API answered; the budget
was too small), so R8.2's "retry once on a transient failure" was not the branch taken — the retry was a
**budget** retry with an identical prompt and identical attachments. **Run 1's partial answer is not quoted
as evidence anywhere below**; run 2 is the reviewed text, and its content on the axes run 1 reached is
consistent with run 1's.

**The three attachments, and why each.**

| # | File | Why |
|---|---|---|
| 1 | `packages/server-rust/benches/soak_harness/evidence/spec357b-walk-r1r2.txt` | **THE SUBJECT.** G7 S2's committed walk: O-0 first, the seven-limb bundle, the determination, R4.5's escalation, the post-data findings list. Essential — the round exists to attack this |
| 2 | `target/spec357-scratch/POST-DATA-FINDINGS.md` | the twelve findings `PD-F1…PD-F12` with their measurements. Without it the consultant cannot attack the round's own account of why O-0 failed |
| 3 | `<scratch>/frozen-predicate-11.0.md` | a **verbatim mechanical excerpt** of the frozen predicate — `sed -n '5779,6614p' packages/server-rust/benches/soak_harness/evidence/spec356-manifest.md`, i.e. the whole `## §11` heading through EOF, 836 lines. Attached so the consultant could judge the limb against its own frozen wording rather than against a paraphrase. **A read-only extraction into a scratch path; the manifest itself was not touched** |

**Deliberately NOT attached:** the two `prune.csv` cells and the four derived ledgers (≈40 MB of rows the
consultant cannot verify by eye, and whose census is already in the subject), the spec body (71 k tokens,
and its normative layer is reproduced in attachment 3 where it is frozen), and the reclamation synthesis
(the ruling is G9's, not this round's).

**Reviewed subject, named exactly.** The determination in `spec357b-walk-r1r2.txt` PART V.0, together with
the walk that produced it (PARTS I–IV), its escalation (PART V), its `DIRECTIONAL` check (PART VI), its
cell-E antecedent (PART VII) and its five post-data findings (PART VIII).

---

## §2 — THE FRAME (adversarial, and stated as such to the consultant)

The prompt opened: *"You are an ADVERSARIAL reviewer. Your job is to ATTACK a diagnostic determination, not
to praise it. I do not want balance; I want the strongest case that this round got something wrong,
mislabelled it, or missed a check that was sitting on the committed artifacts."*

It stated the three hard rules that change what is **useful** without changing what is **true**: the
predicate is frozen and the determination cannot be changed by this round (an argument that the limb is
wrong is *recorded and routed*, never a licence to re-walk); the slope stick and `f(span, width, churn)`
are already adjudicated as discredited and may not be proposed; and where the round is defensible the
consultant was told to say so plainly rather than manufacture balance.

**The seven axes put to it** (the prompt's own numbering, kept below in the dispositions):

- **A1** — is `INDETERMINATE-INSTRUMENT` / `CONSERVATION` the RIGHT limb, when the instrument is internally
  consistent and only the identity is mis-specified?
- **A2** — does `PD-F8` justify the `INSTRUMENT` label, or indict the predicate?
- **A3** — is the refusal to read the concordant (c) counts as evidence correct discipline, or over-strict?
- **A4** — does `PD-F12`'s contradiction undermine the class counts themselves?
- **A5** — is `PD-F11`'s one-op-seq boundary a property of the SYSTEM or of the instrument's RECORDING
  CONVENTION, and is that in tension with A3?
- **A6** — what did this round MISS that was available on the committed artifacts?
- **A7** — what is the single weakest claim in the walk?

plus an open "anything I did not ask" tail.

---

## §3 — THE CONSULTANT'S FINDINGS, EVERY ONE DISPOSITIONED

Twenty-one findings, `X1…X21`. Each carries exactly one label.

### X1 — the limb is a category error (A1) — **RECORDED-AND-ROUTED**

*The finding.* `INDETERMINATE-INSTRUMENT` indicts the instrument, but every check the round itself ran
establishes the instrument is sound: seven counters monotone with zero decreases over 2,880 rows in both
cells, `restored`/`rebuild_cleared` identically 0, ledger censuses agreeing exactly with the transport
(899/899, 897/897, 822/822, 820/820). What failed is the **identity** — a predicate-authored equation
relating a rollover-batched counter to a per-ref gauge without the open-epoch term that reconciles them.
Calling that `CONSERVATION` attributes a specification gap to the gauge.

*My position: ACCEPTED IN SUBSTANCE.* The round says as much itself in `PD-F7` (*"It is not evidence of an
instrument defect in the prune, which is what `INDETERMINATE-INSTRUMENT`'s cause `CONSERVATION` is worded to
indict"*). The consultant's contribution is to press it to the label rather than leave it in a finding.

*Why the disposition is `RECORDED-AND-ROUTED` and can be nothing else.* The frozen antecedent — *"a
violation on a `CONSISTENT` scrape"* — is literally satisfied on 2,484/2,484 and 2,458/2,458 scrapes. The
limb fires as written. Relabelling it would be a post-boundary predicate edit, which is categorical.
**Owner `TODO-634`.**

### X2 — split `CONSERVATION` into `-GAUGE` and `-SPECIFICATION` (A1) — **RECORDED-AND-ROUTED**

*The finding.* The successor's O-0 should carry two sub-causes: `CONSERVATION-GAUGE` (the counters
themselves disagree — the original hazard model) and `CONSERVATION-SPECIFICATION` (the identity names a
subject or a semantics the transport does not carry). Both satisfy the same antecedent; they route to
different owners and imply different repairs.

*My position: ACCEPTED as a successor-predicate design input, with one correction.* The consultant frames
this as a routing improvement; it is more than that — the two sub-causes have **different downstream
consequences for the walk**, which is the substance of X6 below. **Owner `TODO-634`.**

### X3 — `PD-F8`'s *"substantively correct"* sentence is unsupported (A2, A7) — **APPLIED**

*The finding.* `PD-F8` ends: *"What it does do is make that limb's own wording substantively correct rather
than merely procedurally binding — the defect really is in the instrument's transport wiring, and it is not
in the prune."* The consultant names this the single weakest claim in the record and attacks it: the mirror
is credited per rollover **by design** (the source's own comment, `:2502-2513`), the accessor is correct for
its own quantity, and neither is defective in isolation. The defect is the **failure to align** what the
predicate names with what the transport carries — which is at least as much a predicate-authoring defect as
an instrument one. The word *"really"* does epistemic work the evidence does not support.

*My position: ACCEPTED, and this is the one finding this round APPLIES.* The disposition is legitimate
because the surface it touches is **not frozen**: `POST-DATA-FINDINGS.md` is this half's own working
register (untracked, `target/`-ignored), it is append-only by its own convention, and `PD-F8`'s **verdict**
does not move — only its attribution sentence, which was mine to over-claim and is mine to withdraw.

*What was applied.* A `CORRECTION (G8, cross-vendor round)` block appended beneath `PD-F8` withdrawing the
"substantively correct / really in the transport wiring" clause and restating the attribution as
**predicate-authoring: the predicate was frozen without verifying that its named subject is reachable from
the round's only transport.** **`PD-F8`'s status, measurements and routing are unchanged**, and the
committed walk artifact is **not edited** — it quotes `PD-F8`'s *mechanics* (PART I.8(d)/(e)), never the
withdrawn sentence.

### X4 — the predicate-authoring attribution opens a wider fix set (A2) — **RECORDED-AND-ROUTED**

*The finding.* Under "instrument-wiring" the fix collapses to one option — export the accessor to
`/metrics` — which adds a new exporter with its own atomicity surface and may manufacture exactly the
tearing the double-read rule exists to catch. Under "predicate-authoring" the successor may instead **carry
an explicit open-epoch term** (already computable from the scraped columns) or **write the ledger
server-side** rather than scraping it.

*My position: ACCEPTED.* This is the round's most useful single sentence for the successor's design phase.
**Owner `TODO-634`.**

### X5 — the refusal to award `DIRECTIONAL` is procedurally correct (A3, the "for" side) — **RECORDED-AND-ROUTED**

*The finding.* R4.4a's antecedent is false on **both** conjuncts (the published value is not
`INDETERMINATE-MIXED`, and no plurality reading exists because no hypothesis step was evaluated), so
withholding the label is exactly what the rule requires.

*My position: ACCEPTED — a concurrence, recorded so the round's own agreement is on the record rather than
assumed.* No action follows. **Owner `TODO-634`** as a rule-design input only.

### X6 — the fail-closed rule's hazard model does not match this case (A3, the "against" side) — **RECORDED-AND-ROUTED**

*The finding.* The fail-closed rule's stated purpose is *"a diagnosis built on a mis-read gauge would name
the wrong mechanism with full ceremony"* — a hazard model about the **gauge**. Here the gauge is sound and
only the identity is mis-specified. The rule is **unscoped**: any Step-0 failure halts everything, including
readings computed from ledgers that contain none of O-0's eight quantities. The successor should scope the
fail-closed to the components each step actually depends on, and admit a Mode-2 branch (identity
mis-specified + downstream observations demonstrably independent ⇒ proceed **with a published caveat**).

*My position: ACCEPTED AS A SUCCESSOR-PREDICATE INPUT, REJECTED AS A CRITICISM OF THIS ROUND'S CONDUCT.* A
pre-registered gate that can be relaxed after its data exists is not a gate. The consultant agrees the round
had to walk it as written; what it adds is that the *rule* should have been scoped before the freeze, which
is true and is exactly the kind of finding this round exists to route. **Owner `TODO-634`.**

### X7 — PART VI's justification is overbroad and "suppresses" the observation (A3) — **REFUTED-WITH-REASON**

*The finding.* That PART VI's *"reading a bundle limb as a step evaluation is precisely the substitution
Step 0's fail-closed rule exists to prevent"* assumes the rule blocks **any** downstream reading, and that
the discipline therefore **suppresses** a near-unanimous, replicated, independently-verified classification.

*Refuted, on the artifact.* Two errors of fact.

1. **PART VI does not rest on a general "block everything" reading.** It rests on R4.4a's own two conjuncts,
   each checked and each literally false, and it says so in its own text. The sentence the consultant
   attacks is PART VI's *second, independent* observation, not its ground.
2. **Nothing was suppressed.** The (c) counts are **published in full** — 439/439 and 357/357, with both
   denominators, with the whole-cell counts beside them, with an independent re-derivation (0 mismatches
   over 899 / 822 rows) — as R3.4's UNCONDITIONAL bundle limb (ii). What was withheld is a **label**, not an
   observation. A round that publishes the number and withholds the badge is not suppressing anything; it is
   declining to let a reporting limb masquerade as a step evaluation.

The distinction matters because the consultant's own A5 (X10 below) then argues the (c) reading may be a
recording artifact — which is precisely why publishing the counts *without* the badge was the correct
shape.

### X8 — `PD-F12` leaves (c) untouched and exposes only (b)/(d), which are zero (A4) — **RECORDED-AND-ROUTED**

*The finding.* `(c) = ¬T` depends only on the four timing fields, not on `D ≡ (exit_kind ==
DrainedByPrune)`; `(b) = T ∧ ¬D` and `(d) = T ∧ D` both depend on `D`. `PD-F12`'s contradiction therefore
exposes (b) and (d) — both measured **0** on both replicates — and cannot reach (c) directly. The indirect
exposure runs through the **timing** fields, which is `PD-F11`'s channel, not `PD-F12`'s.

*My position: ACCEPTED, and the decomposition is correct.* Recorded rather than applied because it changes
no published count and no step reading — Step 0 halted the walk before any class was read as a step value.
**Owner `TODO-634`.**

### X9 — the artifacts favour `PD-F12` reading 2, "`DrainedByPrune` is assigned on LWM passage, not on observed removal" (A4) — **REFUTED-WITH-REASON**

*The finding, as worded.* *"The most natural reading is that the bookkeeping assigns `DrainedByPrune` based
on the LWM having passed the epoch (making it 'eligible for drain'), not based on observing an actual
non-empty drain. The label is an attribution of cause, not an observation of effect."*

*Refuted at the source, and this is the round's sharpest correction to the consultant.*
`tombstone_frontier_impl.rs:869-895`: the attribution is `drained_epochs.contains(&e).then_some(
FinalExitKind::DrainedByPrune)`, and `drained_epochs` is populated **only** inside
`if let Some(refs) = self.epoch_tags.remove(&e)` — i.e. only for an epoch whose tags this **same
`drain_prunable` call** removed from the RAM index, whose refs are then `extend`ed into the very vector the
call returns. The label is therefore **not** an LWM-eligibility attribution: an epoch that is licensed,
fenced and eligible but whose `epoch_tags` entry is absent gets **no hint** and lands on the `Unclassified`
escape by construction (the source comment says so in as many words).

The only route by which a `DrainedByPrune` exit could coexist with a **zero-length** returned vector is an
`epoch_tags` entry that was present but **empty**; the sole insert path that could produce one is the
rebuild at `:791`, and rebuild is measured **never taken** — `rebuild_cleared_refs_total` is identically 0
on 2,880/2,880 rows of both cells.

**This does not resolve `PD-F12`; it sharpens it**, by removing the escape that would have made the
contradiction benign. It is refuted **as a reading of the source**, and the residual contradiction stays
`RECORDED-AND-ROUTED` under `PD-F12` and the new `PD-F14` below. **No mechanism is named by this paragraph**
and none may be: Step 0 is fail-closed.

### X10 — `PD-F11`'s coincidence is a recording convention, so (c) may be an artifact (A5) — **RECORDED-AND-ROUTED**

*The finding.* `fence_passed_at_op_seq` is stamped when the bookkeeping **observes** the fence, not when the
fence **becomes true**; since `RESIDENT = [entered, exited)` is half-open and the fence stamp lands exactly
on `exited_at_op_seq`, `T` is empty by one op-seq on every row — so the 100 % (c) reading may be measuring
the instrument's stamping convention rather than the system's frontier behaviour. The consultant states
plainly that this is in tension with its own A3 attack, and resolves it as: the (c) reading is not
invalidated, but it is **not confirmed** either.

*My position: ACCEPTED, and this round CONFIRMED THE MECHANISM AT THE SOURCE — see `PD-F14`.* Both licensing
stamps are set by `refresh_epoch_licensing` (`:596-608`), which runs at the **top of the drain pass**, and
`op_seq` advances **only** in `stamp_tombstone` (`:461`) — so it is constant for the whole pass. An epoch
drained in the same pass in which its fence is first observed true therefore has `fence_passed_at_op_seq ==
exited_at_op_seq` **necessarily**, not coincidentally. Measured: that holds on 897/897 (r1) and 820/820
(r2) whole-cell rows.

**This is the strongest single result of the round**, and it cuts **against** relaxing the gate rather than
for it: had the walk proceeded past Step 0 and read (c) at 100 %, it would have named a mechanism on a
discriminant that a half-open interval plus a pass-granular stamp can produce on their own. **Owner
`TODO-634`.**

### X11 — check `refs_at_exit`; it discriminates `PD-F12`'s three readings (A6.1) — **REFUTED-WITH-REASON**

*The finding.* *"If `refs_at_exit > 0` … directly confirming reading 2. If `refs_at_exit == 0`, the refs
were genuinely absent … supporting reading 1."* Ranked by the consultant as its highest-value, lowest-effort
missed check.

*The check was RUN.* `refs_at_exit == 0` on **897 of 897** (r1) and **820 of 820** (r2) exit rows — the
whole cell, not just the window.

*Refuted as a discriminator.* The field cannot discriminate what the consultant claims, because the source
computes it as `self.epoch_tags.get(&epoch).map_or(0, |v| v.len())` (`:663-666`): **a missing key and a
present-but-empty vector produce the same literal 0.** And the exit row is only ever built after
`detect_epoch_exit` has established the key is **absent** (`:626`), so on this code path `refs_at_exit` is
**0 by construction on every row of every exit kind**. The proposed discriminator has one attainable value.
The observation is real and is worth recording — *for the successor's field design*, not as a discriminator
— and it is recorded as `PD-F13` limb (a).

### X12 — publish whether `lwm_passed_at_op_seq == exited_at_op_seq` (A6.2) — **APPLIED**

*The finding.* The bundle publishes `fence_passed == exited` (439/439, 357/357) and
`lwm_passed == None` (0/439, 0/357) but never whether the **LWM** stamp also lands on the exit instant. If
it does, all three windows collapse onto one op-seq and the emptiness of `T` is trivially conventional; if
it does not, the fence's coincidence is the sole signal and is "a different and more interesting" one.

*APPLIED — the check was run and its result is published here.* Whole cell, both replicates:

| | `lwm_passed == exited` | `exited − lwm_passed`: min / median / max | `fence_passed == exited` |
|---|---|---|---|
| r1 (897 rows) | **0 / 897** | 238 / 637 / 1,051 | 897 / 897 |
| r2 (820 rows) | **0 / 820** | 151 / 581 / 1,129 | 820 / 820 |

**The "collapse" variant is measured FALSE.** `LICENSED` opens hundreds of op-seqs before the exit — a
median of 637 (r1) / 581 (r2) stamped refs earlier — while `FENCED` opens exactly on it, on every row. The
deciding-window figures are a subset of these and inherit the same 0-count. Published as `PD-F13` limb (b).
**No class count moves and no step is evaluated by this table**; it is a raw observation on the same ledgers
the bundle already published.

### X13 — join the scrape-level `durable_epoch_watermark` against `fence_passed_at_op_seq` (A6.3) — **RECORDED-AND-ROUTED**

*The finding.* Ranked medium-effort: if a 10 s scrape shows `durable_epoch_watermark >= e` **before** the
recorded `fence_passed_at_op_seq`, the fence transitioned earlier and the stamp is provably detection-time.

*Not run, and the reason is that a cheaper and STRICTLY STRONGER answer was available.* The source settles
it without a join: `refresh_epoch_licensing` (`:596-608`) sets the stamp on the first **pass** at which
`fence >= epoch` already holds, so the stamp is *by construction* a detection-time stamp at
pass granularity — no cross-artifact join can make it anything else, and the scrape join could only bound
the lag, not change its nature. The join remains a legitimate way to **quantify** that lag for a successor
that wants the distribution, so it is routed rather than discarded. **Owner `TODO-634`.**

### X14 — check `considered_total` over the WHOLE cell, not just the deciding window (A6.4) — **APPLIED**

*The finding.* `PD-F12` reports the window Δ as 0. If the whole-cell value is also 0 the prune never drained
at all — a stronger finding; if not, the window's zero is a **transition**.

*APPLIED — the check was run.* Whole cell, from `prune.csv` col 3 (`topgun_or_prune_considered_total`) and
col 13 (`nonempty_drains_total`):

| | `considered_total` 0 → end | `nonempty_drains_total` 0 → end | value at 14,400 s |
|---|---|---|---|
| r1 | **0 → 35,000** | **0 → 35** | **35,000** (i.e. 100 % of it is first-half) |
| r2 | **0 → 99,000** | **0 → 99** | **99,000** (likewise) |

**Every non-empty drain in both cells happens in the first half; the deciding window's Δ0 is a transition
into a zero-drain steady state, not a never-drained state.** The replicate spread on the whole-cell count
is **2.83×** (35 vs 99), which is a fourth instance of this lineage's non-replication at n = 2 (after
`median(L)`, the 39-vs-80 drain spread and `PD-F3`'s RSS slope). Published as `PD-F14`. **This decides
nothing**: it is a bundle-adjacent observation over `UNCHANGED_EMISSION` columns, and Step 0 remains
fail-closed.

### X15 — correlate the torn scrapes with rollover events (A6.5) — **REFUTED-WITH-REASON**

*The finding.* If torn scrapes cluster on the scrapes where `stamped_refs_total` jumps by 1000, tearing and
the identity violation are the same phenomenon seen twice.

*The check was RUN, and the hypothesis is not supported.* Partitioning all 2,879 scrape-to-scrape
transitions by whether `stamped_refs_total` advanced:

| | scrapes with an advance | of them non-`CONSISTENT` | scrapes with no advance | of them non-`CONSISTENT` |
|---|---|---|---|---|
| r1 | 898 | 145 = **16.147 %** | 1,981 | 251 = **12.670 %** |
| r2 | 821 | 139 = **16.931 %** | 2,058 | 283 = **13.751 %** |

A rollover raises the torn rate by ~3.5 points, but **the majority of torn scrapes (251/396 and 283/422)
occur on scrapes where `stamped_refs_total` did not advance at all.** Rollover is a mild contributor, not
the cause; the round's own routing in V.7 — that at ~29 stamps/s a two-render pair tears on roughly one
scrape in seven regardless — survives. Refuted as an explanation; the underlying sampling finding is already
routed under R3.0 limb 4 and needs no new register entry.

### X16 — publish the first-half class distribution separately (A6.6) — **REFUTED-WITH-REASON**

*The finding.* If the first half shows a different class distribution, the transition into the steady state
is visible in the classification.

*Refuted from the already-published counts, without a new computation.* The walk publishes both the
whole-cell counts and the window counts, and the first half is their difference:
r1 whole cell `(c) 897, (e) 1, (f) 1 = 899` minus window `(c) 439, (e) 1 = 440` ⇒ first half
**`(c) 458, (f) 1 = 459`**; r2 `(c) 820, (e) 1, (f) 1 = 822` minus `(c) 357, (e) 1 = 358` ⇒
**`(c) 463, (f) 1 = 464`**. The distribution is **uniform** — 100 % of the discriminating rows are (c) in
both halves of both cells — so the requested table contains no information the record lacks. Worth noting
beside X14: the classification is (c)-uniform across a boundary at which the *drain counters* change
behaviour completely (35/99 non-empty drains before, 0 after), which is itself an argument that the class
assignment is tracking a stamping convention rather than the drain's behaviour.

### X17 — `PD-F12` is diagnostically more important than O-0, and the round buries it (tail 1) — **RECORDED-AND-ROUTED**

*The finding.* O-0's failure is a specification defect that says nothing about the system; `PD-F12`'s
contradiction is a direct observation of the system bearing on the mechanism, and routing it as an
"instrument-design input" understates it. Its three candidate readings are mechanism hypotheses.

*My position: ACCEPTED, with the correction that the *routing* is right even though the *priority* is
understated.* `PD-F12` cannot be anything but routed from inside this half — resolving it would be a naming,
and Step 0 is fail-closed. What this round can do, and does, is record the priority explicitly and add the
source-level sharpening of X9 and the whole-cell numbers of X14 to the routed package. **Owner `TODO-634`,
flagged as the successor's FIRST question.**

### X18 — the `residual == indexed_refs mod 1000` identity is a discovered invariant, not just a failure (tail 2) — **RECORDED-AND-ROUTED**

*The finding.* It holds on 2,880/2,880 rows of both cells; the successor could pin it as a **secondary**
check that validates the mirror's rollover-batching discipline, separate from the primary conservation
identity.

*My position: ACCEPTED as a successor-instrument input.* Recorded, not applied — this half authors no
predicate byte. **Owner `TODO-634`.**

### X19 — "the data names (c) with high confidence" (tail 3) — **REFUTED-WITH-REASON**

*The finding.* That the successor should read this round as: the data strongly supports (c), and the
fail-closed rule merely deferred a naming the evidence already carries.

*Refuted, on the consultant's own analysis and on this round's source reading.* X10/A5 — the consultant's
own axis — establishes that the (c) assignment rests on a one-op-seq gap at a half-open boundary, and
`PD-F14` now shows that gap is **structurally produced** by a pass-granular licensing stamp plus an
in-pass exit. A discriminant that a stamping convention can manufacture on its own does not carry "high
confidence" in the system property it names. R4.3's naming rule was never reached, and this round is
**not** a back door to it: the published value stays `INDETERMINATE-INSTRUMENT` and the mechanism stays
**NOT NAMED**.

### X20 — the successor's first two questions are `PD-F12` and `PD-F11`, not a re-walk (tail 3, second half) — **RECORDED-AND-ROUTED**

*The finding.* Before any re-walk with a repaired identity, the successor must settle (i) whether the prune
is actually removing anything and (ii) whether the fence stamp is a transition or a detection instant —
because those two decide whether (c) is real.

*My position: ACCEPTED, and it is the round's best sequencing advice.* **Owner `TODO-634`.**

### X21 — generalise `PD-F5`/`PD-F8`/`PD-F9` into a blanket pre-data validation rule (tail 4) — **RECORDED-AND-ROUTED**

*The finding.* Three instances of one defect class: a frozen surface exercised only against a synthetic or
proxy artifact rather than the one the DATA half would actually meet — the extractor never run against a
real console log, the identity never evaluated over a real `/metrics` render, the join's `cell_start`
literal never checked to exist in its named artifacts. The round names each instance and never names the
**pattern**. The successor should adopt: *every frozen surface — predicate, tool and join — is validated
against the exact transport the DATA half will consume, before the boundary.*

*My position: ACCEPTED, and this is the finding with the largest effect on what the successor must build.*
It also explains the Tier-1 gate's miss without excusing it: R7.2's CLEAN condition (i) was evaluated over
the accessor, so the gate could return CLEAN on a transport the DATA half would never use. **Owner
`TODO-634`.**

---

## §4 — THE ANCHOR LINES (R5.6, Q9 — authored HERE, natively, each carrying a disposition)

The five measured truths of the Context table and the three hypotheses, each engaged and each dispositioned.

**T1.** — *per-epoch index membership* — **RECORDED-AND-ROUTED**: the target SPEC-356c had to mark
`OUT-OF-SCOPE` is delivered on the entry side by this round (an entry row for
every epoch the clock passed through, `entered_index` as a **value**, `refs_at_entry` = 1000 on every row,
`|P₀| = 0` on both replicates), and the consultant did not contest it — but X11's measurement shows the
**exit** side of the same record is weaker than it looks: `refs_at_exit` is `0` on 897/897 and 820/820 by
construction, because the field conflates "key absent" with "key present but empty" and is only ever read
after absence is established. The membership truth is answered on the entry side and is **not** answered on
the exit side; routed to `TODO-634` as `PD-F13` limb (a).

**T2.** — *drain content enumeration, and exactness* — **RECORDED-AND-ROUTED**: the bundle's 0-byte
exactness bound over 439/439 and 357/357 rows is exact **by
construction** (`bytes_freed_attributed := slot.stamped_bytes`, `:693-698`), which `PD-F12` already records
and the consultant's A4 confirms; X9 adds that the *companion* credit `drained_refs_total +=
slot.refs_at_entry` (`:669`) is likewise an entry-side copy, so **both** exactness terms are tautological on
the `DrainedByPrune` path. Routed: the successor's exit record needs an observed-removal term — the length
of the vector `drain_prunable` returns and the bytes the store actually dropped.

**T3.** — *Δ(LWM) versus Δ(bytes_freed), windowed* — **RECORDED-AND-ROUTED**: the lineage's T3 shape
reproduces at this pin without a new derivation — over the deciding window the LWM moves freely (the
`current_epoch ≡ low_water_mark` equality holds on 1,389/1,440 and 1,407/1,440 window scrapes) while
`bytes_freed_total` moves **0 bytes** on both replicates against ~10 MB of arrival. The consultant did not
raise this axis at all; it is recorded here so the round's own account of what this target says at the pin is on the
record, and routed as continuity input rather than read as a step value.

**T4.** — *non-empty drain rate versus duration — the non-stationary, front-loaded drain* — **APPLIED**:
the consultant's A6.4 check was executed by this round and its result published in X14 — whole-cell
`considered_total` **0 → 35,000 (r1) / 0 → 99,000 (r2)** and `nonempty_drains_total` **0 → 35 / 0 → 99**,
with **100 % of both accumulating before 14,400 s**. That non-stationarity is therefore reproduced at this
pin on this round's own cells, with a **2.83×** replicate spread on the whole-cell count — the fourth
non-replication at n = 2 in this lineage. Applied as a measurement, not as a determination: no class count,
share or limb moves on it.

**T5.** — *the epoch content fate ledger — the starkest number in the lineage* — **RECORDED-AND-ROUTED**:
SPEC-356c's ledger said `was_drained_ever = false` on all 415/447 epoch rows with `bytes_freed_attributed`
empty; this round's new per-epoch exit ledger says the **opposite in form and the same in substance** —
every epoch exits `DrainedByPrune` with a non-zero `bytes_freed_attributed` (897/897, 820/820), while the
store-side counters record **0 bytes freed** and the store's tombstone bytes **grow** by 10.1 MB / 8.25 MB
over the same window. Two ledgers, one system, opposite readings: that is `PD-F12`, sharpened by X9 and
routed to `TODO-634` as the successor's first question.

**H-a.** — *INDEX-POPULATION GAP* — **RECORDED-AND-ROUTED**: not evaluated as a step (Step 0 is
fail-closed), and its sub-population is **empty** — `|P₀| = 0` on both replicates, `entered_index == true`
on every row of both entry ledgers, so the reporting bundle publishes (a) as `0 / 0 UNDEFINED` over `P₀` and
`0/440`, `0/358` over `P`. The consultant did not attack it. What is routed is `PD-F9`'s latent hazard: the
pinned join's `entered_at_unix_ms` is 0 exactly on the rows a strong (a) would produce, so in a world where
(a) were true, `P₀` would fall outside the window **by construction** and the hypothesis would be
unreachable. Zero blast radius here, measured; a vacuous-limb hazard for the successor.

**H-b.** — *SELECTION / SPLIT MISMATCH at `drain_prunable`* — **RECORDED-AND-ROUTED**: not evaluated as a
step; its count is **0** on both replicates, and the durability fence — R3.2's own named "live candidate for
the disqualifying conjunct" — is measured to pass **every** epoch (`fence_passed_at_op_seq == None` on 0 of
439 and 0 of 357). X8 notes that (b), being `T ∧ ¬D`, is exposed to `PD-F12`'s unreliability of `D`; the
exposure is moot at a count of 0. Routed: the successor cannot leave (b)'s discriminant resting on a term
(`D`) that its own instrument populates from entry-side copies.

**H-c.** — *FRONTIER RACE* — **REFUTED-WITH-REASON**, as a *reading*, not as a hypothesis: the reporting
bundle's (c) count is 439/439 and 357/357 (99.77 % / 99.72 % of `P`), concordant across replicates, and the
consultant's tail-3 finding urged reading that as a high-confidence naming — **which this round refuses.**
`PD-F11` shows the assignment turns on exactly one op-seq at a half-open boundary; `PD-F14` now shows that
gap is **structurally produced** — both licensing stamps are written by `refresh_epoch_licensing` at the
top of the drain pass, `op_seq` advances only in `stamp_tombstone`, and an epoch drained in the same pass in
which its fence is first observed true therefore has `fence_passed_at_op_seq == exited_at_op_seq`
necessarily. The **hypothesis** is neither endorsed nor excluded — no step was evaluated — but the claim
that the data *names* it is refuted. The mechanism stays **NOT NAMED**.

---

## §5 — MY SYNTHESIS: what I accept, what I reject, and why

**The consultant is advisory. This half remains the source of truth.** Nothing below moves the
determination, and nothing below is a naming.

**What I accept.**

1. **The limb's LABEL is a category error, and the record should say so plainly** (X1, X3, X4). The
   instrument is sound on every measurement the round took; the identity is mis-specified relative to the
   transport's semantics. The frozen antecedent still fires — that is not in dispute — but the successor
   must not inherit `INDETERMINATE-INSTRUMENT` as a finding *about the prune's instrument*. This is the one
   place I applied a correction, and I applied it to my own un-frozen register, not to a frozen surface.
2. **The fail-closed rule is unscoped, and that is a predicate-authoring defect** (X6). A rule whose hazard
   model is "the gauge may be lying" should not halt readings computed from fields the gauge does not touch.
   It must be **scoped before the freeze**, never relaxed after the data — and the successor should carry
   an explicit Mode-2 branch with a published caveat rather than an all-or-nothing halt.
3. **`PD-F12` is the round's most important output and `PD-F8` is only the most *mechanical* one** (X17,
   X20). The successor's first question is "is the prune removing anything at all?", and its second is "is
   the fence stamp a transition or a detection?" — not "how do we repair the identity".
4. **The pattern behind `PD-F5`, `PD-F8` and `PD-F9` deserves a named rule** (X21): every frozen surface is
   validated against the exact transport the DATA half will consume, before the boundary. Three independent
   instances in one round is a pattern, not bad luck.

**What I reject, and why.**

1. **That the discipline "suppressed" the (c) observation** (X7). It published every count with both
   denominators and withheld only a label whose antecedent was false on two independent conjuncts. Naming
   the withholding a suppression misdescribes the artifact.
2. **That `DrainedByPrune` is an LWM-eligibility attribution** (X9). The source assigns it only for an epoch
   whose `epoch_tags` entry this very call removed, and the sole insert path that could make that removal
   empty is measured never taken. The consultant's reading would have made `PD-F12` benign; the source makes
   it sharper.
3. **That `refs_at_exit` discriminates `PD-F12`'s readings** (X11). It has one attainable value on the exit
   path, by construction.
4. **That the data names (c) with high confidence** (X19). Its own A5 says otherwise and my source reading
   settles it. This is the finding I would have been most tempted to accept, and it is the one it was most
   important to refuse: accepting it would have converted an advisory round into a back-door naming, which
   is precisely the substitution the ordering "naming → cross-vendor → ruling" exists to prevent.
5. **That the first-half class distribution is missing** (X16) and **that rollover explains the tearing**
   (X15) — both answered from the record, the first arithmetically and the second by measurement.

**What the consultant MISSED — the round's own adversarial reading of its adversary.** Recorded so this
section is not a one-way audit:

- It did not attack **`PD-F9`'s join** beyond repeating the round's own account — in particular it never
  noticed that the deciding population is keyed on a field (`entered_at_unix_ms`) that the record's own
  `Default` arm zeroes, which is the one defect in this round that could make a **future** hypothesis
  unreachable rather than merely mis-stated.
- It did not challenge **`|P₁| = 1`** — the smallest denominator in the record — nor the r2 exit-side
  `|Δ| = 2` in the completeness cross-check, both of which a hostile reviewer of the *population* (rather
  than of the *determination*) would open with.
- It did not connect **`PD-F3`'s replicate non-replication** to R4.3's own **n = 2 concordance bar**, which
  is the standing structural weakness of every determination this family publishes — and X14 has now added
  a fourth instance (35 vs 99 non-empty drains).
- It accepted the walk's **census and completeness cross-checks at face value** (899/899, 897/897 …), which
  is reasonable but means the round received no independent pressure on its own transport-agreement claims.

**Net.** The round did not change a determination, and did not need to. It produced one strong new result
(the structural origin of the fence/exit coincidence, `PD-F14`), one strong correction to my own record
(`PD-F8`'s attribution, applied), two measurements the walk had not taken (X12, X14), four refutations of
consultant claims, and one named rule for the successor (X21). The published value stands:
**`INDETERMINATE-INSTRUMENT`, cause `CONSERVATION`, both replicates; the mechanism is NOT NAMED.**

---

## §6 — NEW POST-DATA FINDINGS OPENED BY THIS ROUND

Both are appended verbatim to `target/spec357-scratch/POST-DATA-FINDINGS.md` (this half's untracked working
register) and are reproduced here in full, because that register is not in git and this artifact is.

### `PD-F13` — the exit record's three residency fields: one is constant by construction, and the two licensing stamps have different characters

**Status:** RECORDED-AND-ROUTED. **Owner:** `TODO-634`. **Surfaced:** 2026-08-14, at G8's cross-vendor
round, from consultant axes A6.1 and A6.2.

**Limb (a) — `refs_at_exit` is `0` on every row, by construction, and conflates two different facts.**
Measured `0` on **897/897** (r1) and **820/820** (r2) exit rows. The source computes it as
`self.epoch_tags.get(&epoch).map_or(0, |v| v.len())` (`tombstone_frontier_impl.rs:663-666`), and the exit
row is only built after `detect_epoch_exit` (`:626`) has established the key is **absent** — so the field
has exactly one attainable value on the exit path, and "key absent" and "key present but empty" are
indistinguishable in it. It cannot discriminate between `PD-F12`'s candidate readings. **Routed:** the
successor's record needs `Option<usize>` (or an explicit absent/empty discriminator) plus the length of the
vector the drain actually returned.

**Limb (b) — `lwm_passed_at_op_seq` precedes the exit by hundreds of op-seqs; `fence_passed_at_op_seq`
coincides with it exactly.** Whole cell: `lwm_passed == exited` on **0/897** and **0/820**, with
`exited − lwm_passed` at min/median/max **238 / 637 / 1,051** (r1) and **151 / 581 / 1,129** (r2); while
`fence_passed == exited` on **897/897** and **820/820**. So the "all three windows collapse onto one
op-seq" reading is FALSE: `LICENSED` opens far earlier, and the fence is the sole term landing on the exit
instant. This is the measurement `PD-F11` did not take, and it is what makes `PD-F14` a specific claim
rather than a general worry.

**Limb (c) — both stamps are written by the same pass-granular call, so neither is a transition instant.**
`refresh_epoch_licensing` (`:596-608`) sets each stamp to `self.op_seq` on the first pass at which the
condition **already holds**; `op_seq` advances only in `stamp_tombstone` (`:461`). Both stamps are therefore
**detection-time at pass granularity** — the difference in limb (b) is a difference in *how many passes
earlier the condition became true*, not in the stamps' character.

**Decides nothing.** Step 0 is fail-closed; no class count, share, denominator or limb moves on any of this.

### `PD-F14` — the fence/exit coincidence is STRUCTURAL, and the non-empty drains are entirely first-half

**Status:** RECORDED-AND-ROUTED. **Owner:** `TODO-634`. **Surfaced:** 2026-08-14, at G8's cross-vendor
round, from consultant axes A5 and A6.4.

**Limb (a) — why `fence_passed_at_op_seq == exited_at_op_seq` on 100 % of rows is close to forced.**
`drain_prunable` calls `refresh_epoch_licensing` at its top (`:855`), which stamps
`fence_passed_at_op_seq = self.op_seq` for any tracked epoch whose fence already holds; the same call then
removes the eligible epochs' tags and emits their exit rows with `exited_at_op_seq = self.op_seq` (`:708`);
and `op_seq` does not advance inside a pass (its sole increment is `stamp_tombstone`, `:461`). **Therefore
any epoch drained in the same pass in which its fence is first observed true carries
`fence_passed_at_op_seq == exited_at_op_seq` necessarily.** With `RESIDENT = [entered, exited)` half-open,
`T = RESIDENT ∩ LICENSED ∩ FENCED` is then empty **for that reason alone** — which is exactly the assignment
rule that put 439/439 and 357/357 window rows in class (c).

**What this is, and what it is NOT.** It is a **predicate-sensitivity** finding: the discriminant separating
(c) from (d) can be produced by a pass-granular licensing stamp plus a half-open convention, independently
of whether the system exhibits a frontier race. It is **NOT** a re-classification, **NOT** a naming, and
**NOT** an edit: the frozen definitions of `RESIDENT`, `T` and the six classes are untouched, and the
published counts stand exactly as the bundle published them. **Routed:** the successor must record the
instant a licensing condition **becomes true** (or use a closed interval, or both), and must state which
convention its (c)/(d) split rests on **before** it freezes.

**Limb (b) — every non-empty drain in both cells is first-half.** Whole-cell
`topgun_or_prune_considered_total` runs **0 → 35,000** (r1) and **0 → 99,000** (r2), and
`nonempty_drains_total` **0 → 35** and **0 → 99**, with the whole of each accumulating at or before
14,400 s; the deciding window's Δ0 (`PD-F12`) is therefore a **transition into** a zero-drain steady state.
Over the whole cell the exit ledger nonetheless attributes **897,000** (r1) / **820,000** (r2) refs drained
(1000 per `DrainedByPrune` exit) against **35,000** / **99,000** refs ever `considered` by the store-side
sweep — a ~25× / ~8× discrepancy that is `PD-F12`'s contradiction in its whole-cell form. The **2.83×**
replicate spread on the non-empty drain count (35 vs 99) is a fourth n = 2 non-replication in this lineage.

**Limb (c) — the escape that would have made `PD-F12` benign is closed.** `DrainedByPrune` is assigned only
via `drained_epochs.contains(&e)` (`:890-894`), populated only inside
`if let Some(refs) = self.epoch_tags.remove(&e)` (`:871`) — so the attribution requires an actual removal by
that same call, and an epoch that is licensed, fenced and eligible but absent gets **no hint** and lands on
the `Unclassified` escape instead. The only way a `DrainedByPrune` exit could coexist with a zero-length
returned vector is a present-but-empty `epoch_tags` entry, whose sole insert path is the rebuild at `:791`,
and rebuild is measured **never taken** (`rebuild_cleared_refs_total` ≡ 0 on 2,880/2,880 rows, both cells).
**The contradiction is sharpened, not resolved, and this round resolves nothing** — resolving it would be a
naming, and Step 0 is fail-closed.

---

## §7 — REPRODUCTION: every number in this file, by command

From the repository root unless stated otherwise.

1. **The consultation itself** — §1's invocation, run 2, with the three named attachments. The scratch
   attachment is `sed -n '5779,6614p' packages/server-rust/benches/soak_harness/evidence/spec356-manifest.md`.
2. **X11 / `PD-F13` limb (a), X12 / limb (b)** — one Python pass over
   `packages/server-rust/benches/soak_harness/evidence/spec357-residency-r{1,2}.jsonl`, counting
   `refsAtExit`, `exitKind`, `fencePassedAtOpSeq == exitedAtOpSeq`, `lwmPassedAtOpSeq == exitedAtOpSeq` and
   the `exitedAtOpSeq − lwmPassedAtOpSeq` order statistics.
3. **X14 / `PD-F14` limb (b), X15** — one Python pass over
   `…/evidence/spec357-diag-r{1,2}.prune.csv`, resolving columns **by header name**
   (`topgun_or_prune_considered_total`, `topgun_or_prune_nonempty_drains_total`,
   `topgun_or_prune_stamped_refs_total_a`, `o0_tear_class`, `elapsed_secs`), taking first/last values, the
   value at `elapsed_secs ≤ 14,400`, and the tear-class partition over scrape-to-scrape
   `stamped_refs_total` advances.
4. **X9, X13, `PD-F13` limb (c), `PD-F14` limbs (a)/(c)** — source reading only, at
   `packages/server-rust/src/tombstone_frontier_impl.rs` lines `:461`, `:596-608`, `:626`, `:663-666`,
   `:669`, `:693-698`, `:708`, `:791`, `:855-895`.
5. **X16** — arithmetic over the walk artifact's own published whole-cell and window class counts; no new
   computation.

**No number in this file is copied from `SPEC-356c`'s record**, and no number here re-derives a value the
walk already published — where a walk figure is quoted (the class counts, the O-0 census, the exactness
bound) it is quoted as the walk published it.

---

**END — SPEC-357b G8. The cross-vendor round is complete. The determination is UNCHANGED:
`INDETERMINATE-INSTRUMENT`, cause `CONSERVATION`, both replicates; the mechanism is NOT NAMED. The
fix-shape ruling is G9's and is not written here.**
