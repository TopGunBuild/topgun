# SPEC-356b — R9 cross-vendor gate (`/xreview`)

**Round:** post-execution adversarial review of this spec's executed record, run at G5 (the discharge and
close wave), before merge.

**SPEC-356a's `/xask` is CITED, NOT REPEATED.** R9's first limb is explicit: `/xask` ran **before the
freeze**, on the pre-registered classification decision table and the neutrality-control design + MDE
(`spec356-xask-preregistration.md`, plus the three targeted rounds `spec356-adj11-xask.md`,
`spec356-adj12-xask.md`, `spec356-adj15-xask.md`). Running it again now would be running it **after** the
freeze, which is what it exists to prevent. This file is the `/xreview` limb only.

## Invocation

```
OPENROUTER_TIMEOUT=1200 \
RULE="A record may state only what it can prove from committed bytes: no green that is not earned,
      no enum value that asserts a run that did not happen, no predicate/threshold/ordering/conditional
      altered after the data boundary, and every branch routed to a NAMED owner." \
  ~/Projects/agent-future/scripts/openrouter-review.sh \
    feb85268 packages/server-rust/benches/soak_harness/evidence/spec356-manifest.md
```

- **Model:** `z-ai/glm-5.2` (cross-vendor: not Anthropic).
- **Diff reviewed:** `feb85268...HEAD` restricted to `spec356-manifest.md` — i.e. **the whole of §9**, the
  executed record: controls, the R8.1 walk, ADJ-3's bundle, the dual-use capture, cell E's disposition,
  the POST-DATA records, the OWED repeat and the routing table. **954 diff lines.**
- **Why the pathspec is narrowed to the manifest:** the unrestricted `feb85268..HEAD` diff is dominated by
  ~600 KB of committed measurement CSVs and console logs, which carry no reviewable prose and would
  crowd out the record itself. The manifest **is** the executed record (R5.7(a): the pinned block is the
  only surface the ACs read, and §9 is where every claim lands).
- **The TODO-634 update is NOT in the reviewed diff, and the reason is mechanical:** `.specflow/` is
  git-ignored in this repository, so `TODO-634.md` is a local-only tracker file and cannot appear in any
  `git diff`. Its content is nonetheless derived from §9 §§9.5–9.9, all of which **were** reviewed.
- **Cost:** `prompt=22723 completion=16696 total=39419 → $0.034493272`.

**ADVISORY ONLY.** The reviewer is a third opinion, not an authority: every finding below was verified
against the actual committed bytes before it was applied or refuted.

## Cardinal-rule assessment returned by the reviewer (verbatim)

> I could not construct a scenario where the cardinal rule is violated. I verified each limb:
>
> - **No unearned green:** The graded block (§9.C/§9.8.2) explicitly is NOT green — `FAIL 16`,
>   `SKIP 18/19`. The control verdicts are `NOT REJECTED` (the expected result for a catastrophe
>   detector), not `PASS`. No acceptance criterion ticks green off an unproven claim.
> - **No false enum value:** `CELLE_DISPOSITION = NOT-FIRED-DETERMINATION-INDETERMINATE` is deliberately
>   outside R5.7(e)'s four-value closed enum. It does not assert a run; it says "NOT-FIRED." All four
>   canonical values are individually rejected with reasons (§9.8.2 table). The grader REDs on it by
>   construction.
> - **No post-data predicate alteration:** Every POST-DATA finding (PD-1…PD-5, the power inversion, the
>   reclaim fraction, the small denominator) is recorded and routed. None changes a threshold, ordering,
>   or conditional. §9.5.2 explicitly refuses to read `median(L)` and `B` as a Step-2 determination
>   because Step 2 was never evaluated.
> - **Every branch routed:** §9.11's 16-row table names an owner for every outcome (`TODO-634`,
>   `TODO-637`, `TODO-638`, `TODO-648`). No row terminates in unnamed prose.
>
> I also spot-checked the arithmetic:
> - R5.1 `t = 0.346905` against `4.303` (df=2, α=0.05 two-sided): ✓
> - R5.2 `t = 0.800515` against `4.303`: ✓
> - Conservation identity `232,365 + 2 = 232,367`: ✓
> - Sentinel fraction `708/720 = 98.333% > 50%` threshold: ✓
> - `median(L) = 0` (403 zeros > 360th percentile in 720 sorted values): ✓
> - `MEDIAN_L_OVER_B = 0/999.944741 = 0.000000`: ✓

**A clean cardinal-rule assessment is NOT a green on this spec, and is not read as one.** The graders'
bound (R5.7(f), ADJ-18 clause 4) applies to a reviewer exactly as it applies to an arithmetic limb: an
advisory pass is evidence the record is internally consistent with its pre-registration, **never** evidence
that the run happened as described. Checklists 16, 18 and 19 remain RED at this commit (§9.13).

## Findings, each APPLIED or REFUTED-WITH-REASON

### F1 — MED — "Family-wise error rate stated under independence while the two tests share an arm"

**Reviewer's trigger (verbatim):** *"R5.1 (cross-lineage: `ctl` vs SPEC-355) and R5.2 (within-lineage:
`ctl` vs `ctloff`) both use the `ctl` arm's data. The record states `1 − 0.95² = 0.0975 ≈ 9.8%`
explicitly 'under independence.' Because the `ctl` measurements appear in both test statistics, the tests
are positively correlated … the stated figure is conservative … the record presents 9.8% as a computed
quantity without flagging the dependence."*

**VERIFIED AGAINST THE BYTES — the finding is REAL.** §9.4's two tests do share an arm: R5.1 is
`{ctl-r1, ctl-r2}` against `{spec355-sweep1000, spec355-sweep1000b}`, and R5.2 is `{ctl-r1, ctl-r2}`
against `{ctloff-r1, ctloff-r2}`. Both `t`-statistics are computed from the same two `ctl` levels
(37788.666667, 38297.600000), so they are **not independent**, and a high `ctl` pair pushes both toward
rejection in the same direction. The arithmetic of the correction direction also checks out:
`P(at least one) = 0.05 + 0.05 − P(both)`, which is `0.0975` at independence
(`P(both) = 0.0025`) and **strictly smaller** whenever `P(both) > 0.0025`. **So `9.8 %` is an UPPER BOUND
under positive dependence, not an exact rate.**

**APPLIED — as a POST-DATA record (PD-6, §9.12), and in NO other form.** The `9.8 %` figure and the
"under independence" wording are **§1.4's own frozen, PRE-DATA text**, quoted faithfully by §9.4; a
POST-DATA record may not edit them, and §9.4's committed prose is left byte-unchanged. What the record
gains is the observation, stated where a reader meets the number, plus its direction: **the error is in
the conservative direction** — the real FWER is at or below the stated one — so §1.4's own reason for
declining an α-correction (*"a false adverse reading costs a re-run while a false clean reading would
license invalid numbers"*) is strengthened, not weakened, and **no verdict moves**: neither control
rejected, so no rejection has to be read against the rate at all. **Owner: `TODO-638`**, which already
owns the control set's power and blind-spot questions.

### F2 — LOW — "`MEDIAN_L_OVER_B` published from a computation the committed grader cannot re-derive"

**REFUTED AS A NEW FINDING — it is PD-3, already recorded and already routed, and the reviewer says so
itself** (*"No fix to this record — the transparency is correct and the grader's FAIL is the honest
signal. The fix belongs to TODO-648"*). §9.7.2 PD-3 states the divergence in full (ADJ-12 excludes the
0-sentinel rows and yields `B = 999.944741`; `spec356-slottruth.sh` medians all last-half rows and yields
`0`, so its `bb > 0` guard suppressed the slot), §9.13 tabulates the resulting `FAIL 18` / `WITHHELD 19`,
and the owner is `TODO-648`. **No new action.** The one clause worth pinning, because the reviewer's
phrasing could be read as a licence: the repair requires editing `spec356-slottruth.sh`, a **pinned,
digest-checked sidecar** — `280f7a34…` — and this wave **may not** edit it. The FAIL stands.

### F3 — LOW — "Checklist 12's headline and out-of-range probe are stale against ADJ-20"

**REFUTED AS A NEW FINDING — it is PD-5, recorded at §9.11 row 9 and routed to `TODO-637`**, and the
reviewer confirms the disposition (*"Already routed to TODO-637. No action available to this wave …
Confirmed correct disposition"*). **One factual correction to the reviewer's framing, recorded so the
record is not made wrong by its own review:** the reviewer places the stale headline *"within the frozen
sections"* of the manifest. **It is not in the manifest at all** — checklist 12 lives in the SPEC-356b
**body** (`.specflow/specs/SPEC-356b.md`), and the manifest's §8A carries all twenty addenda with no
count claim of its own. The distinction matters for the routing: fixing the manifest would be an §8A
edit and therefore a re-pin of the pre-registration, whereas fixing the spec body is a spec change with
no predicate consequence — which is why `TODO-637`, the named owner for SPEC-356b changes that must land
in a spec rather than at an executor's keyboard, is the right destination and the manifest is untouched.
**No new action.**

## Tally

| | Count |
|---|---|
| Findings returned | **3** |
| Verified real and **APPLIED** | **1** (F1 → PD-6, §9.12, owner `TODO-638`) |
| **REFUTED with reason** (already-recorded findings re-surfaced; no new action) | **2** (F2 → PD-3/`TODO-648`; F3 → PD-5/`TODO-637`) |
| Findings that would have required editing a frozen section, a pinned sidecar, a predicate, a threshold, an ordering or a conditional | **0** |
| Verdicts moved by this review | **0** |

**No finding altered a predicate, a threshold, an ordering or a conditional, and none could have:** every
applied change is an append-only POST-DATA record in §9, which is the only thing a POST-DATA record is
permitted to be.
