# SPEC-356c — CROSS-VENDOR ADVERSARIAL REVIEW OF THE EXECUTED RECORD (R8)

**R8 is a standing project gate, and this is its discharge for SPEC-356c.** It records the model, the
invocation, the reviewed diff range, the cost, the reviewer's **verbatim** assessment, and **every finding
marked `APPLIED`, `REFUTED-WITH-REASON`, or `RECORDED-AND-ROUTED`.**

**SPEC-356a's `/xask` and the 2026-08-11 verdict adjudication are CITED, NOT RE-RUN** — R8 says so in as
many words, and the reason is the whole point of pre-registration: *a pre-registration round re-run after
the data lands is exactly what pre-registration exists to prevent.* They live at
`spec356-xask-preregistration.md`, `spec356-verdict-xask.md` and `spec356-adj{11,12,15}-xask.md`.

---

## 1. THE INVOCATION, VERBATIM

```
$ OPENROUTER_TIMEOUT=900 \
  CONTEXT="TopGun SPEC-356c: the EXECUTED RECORD of a pre-registered measurement round …" \
  RULE="POST-DATA, NOTHING MAY BE REPAIRED OR RE-CHOSEN. …" \
  bash /Users/koristuvac/Projects/agent-future/scripts/openrouter-review.sh \
       8a60f1357f4702f4c5393538ef6b4ae6cebd247f \
       ':(exclude)*.csv' ':(exclude)*.log' ':(exclude)*.jsonl' \
       ':(exclude)*.json' ':(exclude)*.sha256'
→ openrouter z-ai/glm-5.2 reviewing 8a60f1357f4702f4c5393538ef6b4ae6cebd247f...HEAD (2168 diff lines)…
[usage] prompt=50449 completion=30821 total=81270 cost=$0.11292806
```

| Field | Value |
|---|---|
| **Model** | `z-ai/glm-5.2` via OpenRouter (the script's default; open-weights, 1 M context, chosen for a large prose+data diff) |
| **Script** | `/Users/koristuvac/Projects/agent-future/scripts/openrouter-review.sh` |
| **Timeout** | `OPENROUTER_TIMEOUT=900` — bumped from the 300 s default because the diff is large |
| **Reviewed range** | `8a60f1357f4702f4c5393538ef6b4ae6cebd247f...HEAD` (three-dot: exactly what HEAD adds since it diverged from the pin) |
| **Diff size reviewed** | **2,168 lines / 153,403 bytes** across 6 files |
| **Tokens** | prompt 50,449 · completion 30,821 · total 81,270 |
| **Cost** | **$0.11292806** |
| **Date** | 2026-08-12 |

**THE REVIEWED FILE SET, AND WHAT WAS EXCLUDED AND WHY.** The base is **SPEC-356c's own resolved pin**, so
the reviewer sees **the executed record and nothing else**: §10.0's PRE-DATA pre-declaration is *at* the
pin and therefore *not* in the diff — which is the correct framing, because §10.0 is the frozen thing the
record is graded against, not part of what is under review. The pathspec excludes `*.csv`, `*.log`,
`*.json`, `*.jsonl` and `*.sha256`:

```
 .../evidence/spec356-manifest.md            1720 ++++   (§10.1 … §10.9)
 .../evidence/spec356c-long-r1.matrix.txt      36 +
 .../evidence/spec356c-long-r2.matrix.txt      36 +
 .../evidence/spec356c-scratch-map.txt         42 +
 .../evidence/spec356c-slottruth-run3.txt      88 +
 .../evidence/spec356c-trackergrade-run6.txt  208 +
 6 files changed, 2130 insertions(+)
```

The excluded 18k lines are **bulk measurement rows** (the two 28,800 s `prune.csv`/`csv` series, the ten
target tables, the progress and console logs). They carry no argument for an adversarial reader to break;
the record's *claims about* them are all in the manifest, which is reviewed in full. **The exclusion is
stated here rather than left implicit, because a reviewed-range that quietly drops files is the same defect
class this spec exists to refuse.** Total diff without the exclusion: 20,248 lines.

**ONE THING GIT CANNOT SHOW THE REVIEWER, DISCLOSED IN THE PROMPT.** The AC15 edit to
`.specflow/todos/TODO-634.md` is **invisible to any diff** — `.specflow/` is gitignored at `.gitignore:111`
and no path under it is in the index. The `CONTEXT` therefore named the edit, named the grader, and pointed
at `spec356c-trackergrade-run6.txt`, which *is* in the diff and carries the before/after box text, the
digests and the five-limb verdict. The reviewer's finding-2 analysis shows it read them.

**THE CARDINAL RULE PUT TO THE MODEL, VERBATIM:**

> POST-DATA, NOTHING MAY BE REPAIRED OR RE-CHOSEN. No threshold, predicate, ordering, conditional, window,
> aggregator or grader may be adjusted after the data landed; a defect found in the instrument after the
> data is RECORDED AND ROUTED with a named owner, never fixed. Equally: no claim may be stronger than what
> the frozen walk actually evaluated — with Steps 1–4 NOT EVALUATED, naming or ranking ANY mechanism is out
> of contract, and so is any inferential language (significance, p-values) or wrong-axis reporting (an
> MB/hour slope quoted without naming the series it was fitted on). Hunt for ANY sentence in this record
> that quietly repairs something, back-dates a verdict, upgrades a RED to a GREEN without new bytes, or
> asserts more than the walk evaluated.

---

## 2. THE REVIEWER'S ASSESSMENT — **VERBATIM, UNEDITED, IN FULL**

Nothing below this line to the end of §2 is paraphrased, reordered, summarised or trimmed. Reproducing a
review selectively is how a review becomes a rubber stamp.

<!-- BEGIN VERBATIM REVIEWER OUTPUT -->

I attempted hard to break the cardinal rule and could not find a clear violation. The record is meticulously disciplined about post-data non-repair, non-back-dating, and non-over-claiming. Below are the issues I did find, all below HIGH severity, plus the invariants I verified.

---

## Attempted Cardinal-Rule Breaks — All Checked, None Confirmed

**1. P13 RED→GREEN as back-dating.** §10.2.1 grades P13 RED (edit not yet made); §10.9 grades it GREEN (edit made, grader passed). I checked: §10.9.5 explicitly preserves the RED ("§10.2.1's RED is not edited to green — that would be exactly the back-dating this manifest's append-only discipline exists to prevent"), and the GREEN is not claimed to have been true earlier. The PRESENCE verdict remains "11 GREEN / 2 RED." **Not a violation.**

**2. The TODO-634.md edit as a post-data repair.** The edit records the walk's outcome (INDETERMINATE on both replicates). The grader (spec356c-trackergrade.sh) was pre-declared and digest-pinned (§10.0.6). The edit changes no threshold, predicate, ordering, conditional, window, or aggregator. It is the execution of pre-declared obligation AC15. **Not a violation.**

**3. The v2 driver refusal (Finding 1) forcing a fallback B derivation.** The driver refused (exit 3); B was derived directly from committed column 43. The record says "the predicate defines the quantity and the driver is a re-derivation convenience." I verified: the walk's Step 0 limb (c) reads column 43 directly (§10.2.2: "on committed column 43"), not through the driver — so the walk's limbs are unaffected. For the unconditional bundle, B = UNDEFINED is the same result the driver would have produced (0 non-sentinel rows → empty median), and §10.0.3 pre-declared exactly this case. The fallback uses the same definition, a different implementation. The record discloses the unavailable layer. **Not a violation.**

**4. The §10.6 "vindicated" claim as inferential language.** "n = 2 has now vindicated it" refers to the pre-data method choice (coordinate-last-half count as discriminant), not a mechanism. The observation is direct: the discriminant is 0/0 (identical) while the non-discriminant is 39/80 (differs 2×). The record explicitly limits the claim: "said about the method, not about the mechanism." No significance, p-value, or t-statistic is computed. **Not a violation.**

**5. The R51_T / R52_T t-statistics in Step 0 limb (a) as inferential language.** These are part of the pre-registered Step 0 evaluation (the walk evaluates all four Step 0 limbs). "NOT REJECTED against the frozen critical value 4.303" is the pre-registered test result, not a post-data addition. The determination (INDETERMINATE) does not rest on them — limb (c) fails independently. **Not a violation.**

---

## Findings

### LOW

**L1 — §10.8.3 blockquote misquotes the frozen threshold, dropping the `>` operator**

`spec356-manifest.md`, §10.8.3, the blockquote:
> **limb (c): sentinel fraction `1,440 / 1,440 = 100.000000 %` against the pre-registered `50 %`**

The frozen literal is `> 50 %` (correctly stated in §10.2.2, §10.8.2, and the very next sentence in §10.8.3: "The frozen literal is **`> 50 %`**"). The blockquote — which is the escalation's authoritative statement of "the blocking admissibility limb, named by letter, against its frozen threshold" — drops the operator. A reader who reads only the blockquote sees a threshold of `50 %` (absolute) rather than `> 50 %` (one-sided condition for failure).

**Consequence:** Minor misquotation of the frozen threshold in the one place specifically designated to state it. The correction follows immediately, so practical impact is low.

**Fix:** The blockquote should read `against the pre-registered \`> 50 %\``. However, since §10.8 is post-data new text and R0.3 forbids post-data edits, this is a recorded defect — it should be routed, not fixed in place.

---

**L2 — Hostnames differ between replicates; same-machine claim is implied but unverified**

`spec356c-long-r1.matrix.txt:4` and `spec356c-long-r2.matrix.txt:4`:
- r1: `host/OS: Darwin MacBookPro 25.5.0 …`
- r2: `host/OS: Darwin MacBook-Pro-koristuvac.local 25.5.0 …`

§10.1.3 says: *"the harness is machine-bound and two 8 h cells sharing a host perturb the very throughput that is the measurand"* — justifying sequential execution. The kernel version, build date, and architecture (T6000/arm64) are identical, suggesting the same physical machine with a hostname change between runs. But "MacBookPro" (default macOS HostName) vs "MacBook-Pro-koristuvac.local" (custom LocalHostName) is a material enough difference that a reader cannot confirm same-machine from the committed bytes alone.

**Consequence:** If these are different machines, the sequential-execution justification is moot and the "same binary, same pin, same configuration" framing understates a hardware variance. More likely the same machine, but the record doesn't verify or explain it.

**Fix:** Add one line to §10.1.3 noting the hostname change (e.g., "the hostname was changed on the host between runs; the kernel build string is identical") so the reader doesn't have to infer it.

---

**L3 — §10.8.1 lists three mechanism names but says "four"**

`spec356-manifest.md`, §10.8.1:
> "remains unclassified among SELECTION/FRONTIER, SCHEDULING/LICENSING and THROUGHPUT, and a mechanism outside those four is not excluded."

Three names are listed (SELECTION/FRONTIER appears once, collapsing its exit-limb and licensing-limb variants), but "four" is asserted. §10.8.7 item 3 correctly enumerates all four. The collapsed listing in §10.8.1 is not wrong per se (the two SELECTION/FRONTIER variants share a name), but the count mismatch could confuse a reader cross-checking.

**Consequence:** Cosmetic inconsistency; no mechanism is named or ranked.

**Fix:** Either list all four (as §10.8.7 does) or say "those three families" to match the listing.

---

**L4 — The `mechanism.json` rename is annotated but not machine-verified in the sha256 chain**

`spec356c-long-r1.matrix.txt:13`: `mechanism.json: … (harness writes spec356c-long-r1.soak.mechanism.json; renamed after the run)`

The harness writes `${BASE}.soak.mechanism.json`; a human renamed it to `${BASE}.mechanism.json` post-run, pre-commit. The `artifacts.sha256` file (7/7 OK) verifies against the renamed file. But the rename is a manual post-run step — if it had been forgotten or botched, the sha256 check would have caught it, but the matrix.txt annotation is the only evidence the content is unchanged (the sha256 was presumably recomputed after the rename). There is no byte-level proof the rename preserved content (only the sha256 chain, which starts from the renamed file).

**Consequence:** Low risk — the sha256 chain anchors the committed file, and a botched rename would fail the check. But the "content unchanged" claim rests on operator honesty, not on a pre-rename digest.

**Fix:** Record the pre-rename sha256 alongside the post-rename one in a future round, or have the harness write the final name directly.

---

## Invariants Verified (No Violation Found)

1. **No post-data repair.** No threshold, predicate, ordering, conditional, window, aggregator, or grader is adjusted after the data boundary (`0b96e63e`). Both findings in §10.2.6 are recorded and routed, not fixed. No `ADJ-21` is authored.

2. **No back-dating.** P13's RED in §10.2.1 is preserved verbatim. §10.9's GREEN is explicitly a separate, later reading. §10.9.5 states: "Neither reading is retracted, and neither is back-dated."

3. **No mechanism named or ranked.** The determination is INDETERMINATE via Step 0 limb (c). Steps 1–4 are NOT EVALUATED. No mechanism is endorsed, excluded, or ranked. §10.2.3 and §10.8.1/§10.8.7 state this explicitly.

4. **No inferential language at the determination level.** No p-values, no significance claims, no t-statistics reported as determinations. The R51_T/R52_T values are Step 0 limb (a) inputs (pre-registered), not determination outputs. §10.2.5 and §10.6 explicitly disclaim significance over n=2.

5. **No wrong-axis slope reporting.** Every slope names its series: `tombstone_bytes` (§10.1.6), `passes_total` over `current_epoch` (§10.2.4(i)), `tombstone-byte growth` (§10.1.4 finishedReason). The `slope_mb_per_hour` field-name-vs-bytes-unit discrepancy is disclosed (§10.1.6: "the unit here is BYTES per hour").

6. **Data boundary holds.** §10.1.5: every pre-data instrument byte entered `main` at or before the pin `8a60f135`, which is a strict ancestor of the boundary `0b96e63e`. The squash-merge discrepancy is recorded, not repaired (§10.1.2).

7. **Build identity holds.** §10.1.1 CHECK 2: empty `.rs` diff between `feb85268` and `8a60f135`. Both replicates used the same binaries (same paths, same build timestamps).

8. **The walk's coordinate last half is a coordinate, not a row index.** §10.2.2: rows with `elapsed_secs > (t_first + t_last)/2`. Both replicates resolve to [14410, 28800], 1440 scrapes. Verified against the prune.csv row counts and cadence.

9. **The ADJ-7 boundary is not crossed.** `L` and `P` are PRE-drain; `indexed_refs` is POST-drain. `P/indexed_refs` and `L/indexed_refs` are computed nowhere (§10.2.2, §10.3).

10. **The unconditional bundle is reported regardless of the walk outcome.** §10.2.4: all five limbs published despite INDETERMINATE. B = UNDEFINED (100% sentinel, 0 non-sentinel rows) is the honest value, not coerced. The non-drop exit share is UNDEFINED (denominator 0), not 0.000000%.

11. **The grader was digest-checked before execution.** §10.9.4 / `spec356c-trackergrade-run6.txt`: three pre-declared digests verified (3/3 match §10.0.6). The fixture was provenanced against §10.0.7's PRE value before any limb was graded.

12. **The sixth transcript is a new file, not an append.** Appending to `spec356c-trackergrade-proofs.txt` would move a pinned sidecar digest (R0.2 violation). The new file `spec356c-trackergrade-run6.txt` avoids this. The five pre-data transcripts stay byte-identical.

<!-- END VERBATIM REVIEWER OUTPUT -->

---

## 3. DISPOSITION OF EVERY FINDING

**THREE LABELS, AND THE THIRD IS R8's OWN.** R8 requires each finding marked `APPLIED` or
`REFUTED-WITH-REASON`, **and in the same sentence creates the third**: *"Zero findings may edit a frozen
section, a pinned sidecar, a predicate, a threshold, an ordering or a conditional; a finding that would is
**recorded and routed, not applied**."* R0.3 stacks on top: we are **POST-DATA**, so a finding about the
**instrument** is *"a POST-DATA RECORD routed to a named follow-on — never a predicate edit, never an
addendum, never a checklist patch."* `RECORDED-AND-ROUTED` is therefore **not a softer "refuted"**: it
means *the finding is accepted as TRUE and the record is forbidden from absorbing it.*

**THE HONEST SUMMARY UP FRONT: 0 APPLIED, 4 RECORDED-AND-ROUTED (all four ACCEPTED AS TRUE), 5 ATTEMPTED
CARDINAL-RULE BREAKS REFUTED-WITH-REASON.** Not one finding was dismissed as wrong. Every one of the four
was **re-verified against the committed bytes by the executor** before being labelled — a disposition taken
on the reviewer's say-so is not a disposition — and every one of them landed on a surface this spec is
barred from touching.

### 3.1 The four findings

| # | Finding | Severity | Verified against bytes? | Disposition |
|---|---|---|---|---|
| **L1** | §10.8.3's block quote drops the `>` from the frozen `> 50 %` threshold | LOW | **YES — confirmed true** | **RECORDED-AND-ROUTED** |
| **L2** | The two `matrix.txt` files carry different hostnames for the same machine | LOW | **YES — confirmed true** | **RECORDED-AND-ROUTED** |
| **L3** | §10.8.1 lists three mechanism names while saying *"those four"* | LOW | **YES — confirmed true** | **RECORDED-AND-ROUTED** |
| **L4** | The `mechanism.json` rename is annotated but has no pre-rename digest | LOW | **YES — confirmed true** | **RECORDED-AND-ROUTED** |

---

**L1 — §10.8.3's block quote drops the `>` operator. `RECORDED-AND-ROUTED`. The finding is CORRECT.**

*Re-verified at the bytes:* `spec356-manifest.md` §10.8.3's block quote reads *"against the pre-registered
`50 %`"*; the frozen literal is `> 50 %`, and §10.2.2, §10.8.2 **and the sentence immediately following the
block quote** all state it correctly — that next sentence is literally *"The frozen literal is **`> 50 %`**
and 100.000000 % is inside it."* So the record is **imprecise in one quotation and correct one line later**,
never wrong about the threshold anywhere it is evaluated.

*Why it is NOT applied:* §10.8 is **frozen** — no byte of §0–§8, §8A, §9 or §10.0–§10.8 may change. And the
surface at issue is the escalation's authoritative restatement of **a frozen threshold**, which is on R8's
own explicit not-editable list. Editing a threshold quotation POST-DATA — even to *improve* it — is the
precise act R0.3 makes categorical, and *"but the edit made it more accurate"* is exactly the argument that
would let any later round re-touch a frozen number.

*Routing:* **`TODO-634`**, as a design-phase wording input to the next spec that **RE-PINS**; that spec may
restate the limb with its operator intact, because it is authoring a new surface rather than editing a
frozen one.

*Honest cost of not applying:* a reader who reads the block quote and stops one line early sees a bare
`50 %`. That is a real, if small, cost, and it is carried rather than argued away.

---

**L2 — the two `matrix.txt` files disagree on the hostname. `RECORDED-AND-ROUTED`. The finding is CORRECT.**

*Re-verified at the bytes:* `spec356c-long-r1.matrix.txt:5` reads `Darwin MacBookPro 25.5.0 …`,
`spec356c-long-r2.matrix.txt:5` reads `Darwin MacBook-Pro-koristuvac.local 25.5.0 …`. Kernel version, build
string (`Tue Jun 9 22:18:58 PDT 2026`), XNU revision and `RELEASE_ARM64_T6000 arm64` are **byte-identical**
on both. The reviewer's reading is precisely right: **the committed bytes do not, by themselves, prove
same-machine**, and §10.1.3's sequential-execution justification (*"the harness is machine-bound"*) reads as
if they did.

*What the executor can add as a POST-DATA record — and explicitly NOT as a repair:* the executing host at
the time of this review reports `Darwin MacBookPro 25.5.0 … RELEASE_ARM64_T6000 arm64`, i.e. the macOS
`HostName` / `LocalHostName` pair on one M1 Max machine, which is the ordinary cause of exactly this
divergence. **That is a statement of what was observed now; it does not retroactively prove what the host
was during r2, and it is offered as a record, not as evidence that closes the finding.**

*Why it is NOT applied:* the fix the reviewer proposes is *"add one line to §10.1.3"* — a **frozen**
subsection. And the underlying bytes are the two **committed `matrix.txt` cell artifacts**, which are
pinned sidecars whose line counts checklist item 5 grades; editing either would break its own provenance
check. Both routes are barred.

*Routing:* **`TODO-648`** (the instrument class: the runner should capture a **stable** machine identity —
hardware UUID or `sysctl hw.model` — rather than a mutable hostname, so a future round proves
same-machine mechanically) **and `TODO-634`** as a design-phase input.

---

**L3 — §10.8.1 lists three names but says *"those four"*. `RECORDED-AND-ROUTED`. The finding is CORRECT as
a readability defect, and is NOT a contract defect.**

*Re-verified at the bytes:* §10.8.1's block quote names `SELECTION/FRONTIER`, `SCHEDULING/LICENSING` and
`THROUGHPUT` — **three labels** — and then says *"a mechanism outside those four"*. The four pre-registered
mechanisms are `SELECTION/FRONTIER (exit limb)`, `SELECTION/FRONTIER (licensing limb)`,
`SCHEDULING/LICENSING` and `THROUGHPUT`, which §10.2.3 and §10.8.7 item 3 both enumerate in full. So the
**referent count of four is right** and the **listing collapses two same-named limbs** — a cross-checking
reader counts 3 against a stated 4 and has to reconcile it.

*Why this is not a contract defect, stated rather than assumed:* the cardinal rule bars **naming or ranking
a mechanism**. The collapsed list names **all** the families and endorses **none**, and the sentence's own
job is to say the cause is *unclassified*. Nothing is asserted that the walk did not evaluate.

*Why it is NOT applied:* §10.8 is frozen, and this is the escalation's **item 1** — the one sentence §8.2
requires verbatim. *Routing:* **`TODO-634`**, design-phase wording input.

---

**L4 — the `mechanism.json` rename has no pre-rename digest. `RECORDED-AND-ROUTED`. The finding is CORRECT,
and it is an INSTRUMENT finding, which R0.3 makes categorical.**

*Re-verified at the bytes:* `spec356c-long-r1.matrix.txt:12` reads
`mechanism.json: /private/tmp/spec356c-out-r1/spec356c-long-r1.mechanism.json (harness writes
spec356c-long-r1.soak.mechanism.json; renamed after the run)`. The `artifacts.sha256` chain (7/7 OK on both
replicates) anchors the **post**-rename bytes. The reviewer is right that **no pre-rename digest exists**,
so *"the rename preserved content"* rests on the operator, not on a measurement — a botched rename would
have been caught (the file would be absent or mismatched), but a **swapped** file of the right name would
not have been.

*Why it is NOT applied:* the proposed fix is a **harness change** (*"have the harness write the final name
directly"*) — a `.rs` edit under R0.4, which **destroys the frozen-inherited build identity that is the
entire point of the repeat**. R0.4 settles this fork PRE-DATA precisely so no executor resolves it at the
keyboard: *such a target is routed `OUT-OF-SCOPE` with the reason stated, NOT taken.* The alternative fix
(*"record the pre-rename sha256"*) is a statement about bytes that no longer exist in that form.

*Routing:* **`TODO-648`** (the instrument-hygiene class — the harness should emit its final filename, or the
runner should digest before the rename) **and `TODO-634`** as a design-phase input, *"the next instrument
should …"*.

---

### 3.2 The five attempted cardinal-rule breaks — `REFUTED-WITH-REASON`

The reviewer opened by trying five concrete routes to a rule violation and closed each itself. **Each is
recorded here as a refutation with its own reason, and each was re-checked by the executor against the
committed bytes — a self-refuting reviewer is not automatically a correct one.**

| # | Attempted break | Refutation, and the bytes that carry it |
|---|---|---|
| **B1** | *P13's RED → GREEN is back-dating.* | **REFUTED.** §10.2.1's RED is **unedited** — the prefix `cmp` against the pin is silent and `git diff --numstat` shows **0 deletions** across the whole manifest, so the RED is byte-identical to what wave 5a published. §10.9.5 states both readings in one table and retracts neither. And §10.2.1 **pre-announced** this exact event: *"a later GREEN there is not a contradiction of this RED, it is the next reading of the same limb."* The PRESENCE verdict of record stays **11 GREEN / 2 RED**. |
| **B2** | *The `TODO-634.md` edit is a post-data repair.* | **REFUTED.** The edit adds **16 lines, deletes 0**, all inside the §8.1 box (limb (e): one hunk `@@ -233,0 +234,16 @@`, 0 hunks outside), touches no threshold/predicate/ordering/conditional, and does **not tick** the box (2 / 5 unchanged, census 7 / 0). It executes AC15, an obligation pre-declared at §10.0.7 **before the data**, with the constraint on the edit's shape also pre-declared there. |
| **B3** | *v2's refusal forced a fallback `B` derivation — an instrument re-choice.* | **REFUTED.** Step 0 limb (c) reads committed **column 43** directly; the driver is a re-derivation convenience, not the definition. `B = UNDEFINED (100 % sentinel, 0 non-sentinel rows)` is the **pre-declared** §10.0.3 branch and is the same value the driver would emit from an empty population. The refusal itself is published as §10.2.6's Finding 1 and **routed, not repaired** — the driver is not edited, run 3 is not re-run, and R4.6 accepts the literal as a value row. |
| **B4** | *§10.6's "vindicated" is inferential language.* | **REFUTED.** The predicate of "vindicated" is a **method choice made PRE-DATA**, not a mechanism; the underlying observation is a direct count (the discriminant identical across replicates, the non-discriminant differing 2×), and §10.6 bounds it in its own words: *said about the method, not about the mechanism*. No significance, no p-value, no test statistic. |
| **B5** | *The `R51_T` / `R52_T` t-statistics are inferential language.* | **REFUTED.** They are Step 0 limb (a) **inputs**, pre-registered with a frozen critical value of `4.303`; the walk evaluates all four Step 0 limbs. They are reported as evaluated values, never as a determination, and the determination does **not** rest on them — limb (c) fails independently on both replicates. |

### 3.3 The twelve invariants the reviewer verified

The reviewer's closing section lists twelve invariants it checked and found intact (post-data non-repair;
no back-dating; no mechanism named or ranked; no inferential language at the determination; no wrong-axis
slope; the data boundary; build identity; the coordinate last half; the ADJ-7 boundary; the unconditional
bundle reported regardless of outcome; the grader digest-checked before execution; the sixth transcript as
a new file rather than an append). **They are reproduced verbatim in §2 and are not re-argued here** — an
executor restating a reviewer's greens in his own words is the least load-bearing text a record can carry.

---

## 4. THE R8 CLOSURE STATEMENT

- **Findings total: 4.** `APPLIED`: **0**. `REFUTED-WITH-REASON`: **0 of the 4 findings** (all four are
  accepted as true) **and 5 of the 5 attempted cardinal-rule breaks**. `RECORDED-AND-ROUTED`: **4**.
- **ZERO findings edited a frozen section, a pinned sidecar, a predicate, a threshold, an ordering or a
  conditional.** All four would have, which is exactly why none was applied.
- **No `ADJ-21` was authored. No cell was re-run. No checklist was patched. No `.rs` file was touched.**
- **No tracker file was edited by this review.** In particular `TODO-634.md` is **not** touched again: its
  post-edit digest `a1b17d97…9428` is already published in §10.9.1, and a second edit would falsify a
  published measurement. **The routing above IS the routing** — the same disposition §10.2.6 states for its
  own two findings: *"a reader who wants the loop closed must open `TODO-648` and `TODO-634` and find it
  there, and it will only be there if somebody puts it there."*
- **The one honest weakness of this review, stated rather than left for a reader to notice:** the reviewer
  found **no HIGH and no MED finding**, and a review that returns only LOWs on a 2,168-line record is a
  result to be suspicious of, not proud of. Two things bound that suspicion and neither eliminates it —
  (i) the reviewer tried **five** specific routes to a cardinal-rule violation and showed its work on each,
  which is a stronger signal than a bare "looks good"; (ii) it produced **four** findings that are all
  independently verifiable and all turned out true when checked against the bytes, so it was reading the
  actual text and not summarising it. What it cannot do is see `.specflow/`, the 18 k excluded data lines,
  or §10.0 — and a defect living in exactly those places would not appear here.
