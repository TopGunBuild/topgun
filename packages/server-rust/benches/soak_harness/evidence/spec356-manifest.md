# SPEC-356 evidence manifest — the per-epoch prune record, and the pre-registered classification of why the prune falls behind

**What this document is.** §0–§8, together with §8A's adjudication addenda, are the **SOLE NORMATIVE
SOURCE of SPEC-356's pre-registration**. Every spec body — SPEC-356a and SPEC-356b — carries **pointers**
to this text, never copies. **Where any spec text and this manifest differ, THIS MANIFEST GOVERNS**, and
the difference is recorded as an append-only §8A adjudication addendum — never by editing §0–§8.

**Status of §0–§8 at this commit.** These sections are authored **before any measurement artifact of this
family exists**, and no group after their author edits them. They become **FROZEN** only once **both
SPEC-356a Wave-1 evidence gates** have passed — EG-1 (`spec356a-eager-registration.log`, eager
registration with zero blank cells from the first row) and EG-2 (`spec356a-step0c-fixture.log`, the
captured guard-fire artifact). **The freeze is not declared in this document**, and it is not declared by
the group that wrote these sections; it is reached at SPEC-356a's final content commit, with both gate
artifacts on disk and reading as specified. A gate failure that implicates the frozen text lands as a
**PRE-DATA §8A addendum**, never as an edit to §0–§8.

**Why freezing this text ahead of the data is the whole point.** The question SPEC-356 answers has four
possible causes that route to **opposite fix shapes**, and one of them — the modal one — would be read by
a naive flat table as two different causes at once. A rule chosen after seeing the numbers is not a rule,
it is a preference. So the rule is committed first, in a commit `git log --follow` can be asked to prove
came earlier (this file versus the first `spec356-*.soak.json`, which **SPEC-356b** creates). **The
pre-registration proof is COMMIT ORDER, not a hash written into this file.**

**§9 onward — the executed record — is SPEC-356b's and is deliberately absent here.** Writing it at this
commit would destroy the very commit-order property the classification rests on.

---

## §0 — Lineage, comparability, and build identity

### §0(a) — The lineage declaration

SPEC-355 ran under **R0.6 build identity**: any `.rs` edit invalidates every slope taken before it, and no
slope may be carried across a lineage boundary. **SPEC-356 deliberately breaks that lineage** —
instrumenting the prune path *is* a `.rs` change — and **SPEC-356a is the only half in which the break
happens.**

All runs in SPEC-356b belong to one named lineage, **INSTRUMENTED**, built **once** from a single pinned
post-change commit. The only exception is the conditional **cell E**, which is a **PROVENANCE half-swap**
(instrumented harness, pinned pre-346 server) and is read **corpus-scan-only** (§4).

**The pin is SPEC-356a's merge commit, and this section deliberately carries no hash and no slot for
one.** A spec cannot contain the hash of the commit that contains it, and a placeholder token in a frozen
section would license a post-freeze edit to the pre-registration. **SPEC-356b's B1 resolves the pin from
`git log main` as its first action** and records it in **§9** of this manifest — its own section — **and in
every `matrix.txt`**. **No frozen section (§0–§8, §8A) is touched to record the pin.** That sentence is a
forward reference, not a value, and it is complete as written.

### §0(b) — The comparability ledger

What survives the instrumentation and what does not is stated **once**, here, per statistic. This ledger
is normative; the spec bodies point at it and do not restate it.

| Quantity | Status across the instrumentation |
|---|---|
| Any slope or level read off the new binary | **NEW LINEAGE.** Never quoted as continuous with a SPEC-355 number. |
| **`rss_mb` — the RSS series, on every cell of SPEC-356b** | **FORFEIT. Named explicitly, not left to the blanket rule above.** The new histogram/summary series retain up to **60 s of samples per series inside the exporter** (`3 × 20 s` rolling window), which is resident memory this binary holds and SPEC-355's did not. §1's neutrality proofs are scoped to `tombstone_bytes` **alone** and prove **nothing** about RSS. Therefore **no RSS number — level, slope, plateau or ceiling — may be read off any run in SPEC-356, in either direction** (neither "RSS grew" nor "RSS held"). |
| The width-1000 / 1800 s **level** (SPEC-355 HEAD pair: 38,715 · 36,624, mean **37,670**, sd 1,479) | Comparable **only** as §1's control target, with its MDE stated. Not a baseline to build on. |
| Protocol constants: the matrix literals, the 60 s primary CSV cadence, `spec349c2-fit.awk`, the 8-window slicing one-liner | **Instrument identity, not a measurement.** Carried verbatim; carrying them is what makes the control meaningful. |
| The redb corpus scan (`main.rs` reader over the child's file) and the WAL frame-kind census | Binary-independent by construction; carried. |
| SPEC-355's 4 h series and its 8 windowed fits | Remain the **reference statement of the defect**. SPEC-356b's 4 h run is a **second observation in a new lineage**, not a replacement. |
| `TG-OR-005`'s status and the 512 B/h gate clause | **Untouched by anything measured in SPEC-356.** |

### §0(c) — RSS routing

RSS is the series **TODO-590 / TODO-591 / TODO-593** and **SPEC-348**'s DISK+RSS gate read. Those owners
take RSS from **their own** uninstrumented lineage; **SPEC-356's runs are not an input to any of them.**
The `rss_mb` column stays in the primary CSV only because §5 freezes that header byte-identical — its
presence in a committed artifact is **not** a licence to read it. Concretely: **the diagnostic's 60 s
in-exporter retention can never be quoted as an RSS claim**, favourable or adverse, by SPEC-356 or by
anything citing it.

### §0(d) — Build identity after the pin: no post-pin `.rs`

SPEC-356a's deliverable is a **merged instrument whose final content commit is the freeze boundary and
after which no `.rs` byte changes**. **No `.rs` edit may land in SPEC-356 after SPEC-356a merges** — R0.6
applied to SPEC-356's own lineage: any post-pin `.rs` change invalidates every run taken before it and
forces a re-run of the whole matrix. SPEC-356b introduces **zero `.rs` bytes**, checked mechanically
(`git diff --stat <pin>..HEAD -- '*.rs'` is empty), which is what restores build identity inside the
measuring half. **If SPEC-356b discovers an instrument defect, the remedy is a new spec that re-pins**,
never an edit absorbed under the pin.

---

## §1 — Neutrality: the two measured controls and the 2×2 that reads them

**A red tombstone gate is NOT evidence against `TG-OR-004`.** `TG-OR-004` (`INVARIANTS.md:448`, *decided,
enforced*) **MUST NOT be flipped** by anything in SPEC-356. The invariant is about where the tombstone-byte
counter is called from and that the accounting is not double-counted or dropped; the **gate** is about how
many tombstone bytes remain resident at a given epoch width over a given horizon. A breached gate says the
prune does not keep up — it says nothing about the counter's siting or its correctness, and the two are
not substitutes for one another. **Neutrality in the sense §1 controls for means: the instrumentation does
not perturb `tombstone_bytes`, the add path, or the prune path that invariant covers.** SPEC-356b's B11
reads this paragraph.

**The deterministic proof is already discharged by SPEC-356a and is not re-derived here.** In `crdt.rs`'s
own test module, over one fixed synthetic prune workload under an isolated gauge binding: armed and
disarmed runs produce **identical** gauge deltas and **identical** dropped counts; a **structural**
assertion holds that `prune_epoch_tombstones` still names exactly one `sub_tombstone_bytes` call, in the
post-write `Ok(_)` arm behind `dropped`, and that the recorder body names no tombstone-byte counter at
all; and `apply_or_delta`'s counter-freeness is unchanged. **That is the proof that carries the weight, it
is unconditional, and it passed before any run existed.** The two controls below are the *measured*
complements, and they are the ones that need runs.

### §1.1 — Series control (weak, necessary, and honestly powered)

The `ctl` pair's last-half level mean is compared against SPEC-355's committed HEAD width-1000 pair
(38,715 · 36,624; mean **37,670**; sd 1,479; run-to-run spread 5.4 %). **The predicate is pre-registered as
a FORMULA, not a number:**

> **No perturbation is detected iff the two-sample `t` (df 2, two-sided α = 0.05, critical value 4.303)
> does NOT reject.**

**The minimum detectable effect is computed from the OBSERVED pooled sd, by this formula:**

> **MDE = t_crit(df = 2, two-sided α = 0.05) × s_pooled × √(1/n₁ + 1/n₂) = 4.303 × s_pooled × 1**, for the
> pre-registered `n₁ = n₂ = 2`.

On SPEC-355's observed sd (1,479 B) that is **≈ 6,400 B ≈ 17 % of the level**. **A smaller perturbation is
NOT excluded** — the control is honestly weak, and this sentence is part of the pre-registration precisely
so a non-rejection is not read as a proof of zero effect. The MDE is recomputed from the **observed**
pooled sd of the actual pair and reported with the determination; the formula, not the 6,400 B figure, is
what is frozen.

Reclaim fraction is reported beside it (committed HEAD reference: 80.1 % / 92.2 % at width 1000 / 1800 s)
as **corroboration, never as the deciding statistic**.

### §1.2 — Within-lineage control (higher power, same binary)

`ctl` (armed) versus `ctloff` (disarmed) — **one build, one lineage, one difference**. Same predicate as
§1.1, MDE stated the same way, computed from that pair's own observed pooled sd.

### §1.3 — Failure disposition: a 2×2 keyed on WHICH control rejected

§1.1 and §1.2 do not test the same thing, so **they must not be OR'd**. **§1.1 is CROSS-LINEAGE** (new
binary versus SPEC-355's committed HEAD pair) and confounds *build* with *instrument*. **§1.2 is
WITHIN-LINEAGE** (one build, one lineage, one difference) and isolates the *instrument* alone. §0(b)
**already declares the new binary a new lineage**, so a cross-lineage difference is an *expected*,
*declared* outcome — not a defect.

| | **§1.2 passes** (armed ≡ disarmed) | **§1.2 rejects** |
|---|---|---|
| **§1.1 passes** | **CLEAN.** Both controls hold; MDEs stated; classification proceeds. | **INSTRUMENT PERTURBATION.** The one cell that indicts the instrument: same build, sole difference is arming. **No classification number may be read off any run.** Routes to the named remediation — **move the observation off the hot path** (or shrink it) — then **re-run the control**. |
| **§1.1 rejects** | **BUILD-LINEAGE EFFECT — NOT an instrument defect.** The instrument is exonerated by the higher-powered, better-controlled test; the difference is attributed to the lineage break §0(b) already declared. **The classification numbers STAND.** Recorded as a finding, with both series and the effect size, and quoted beside the determination — never averaged away, never silently absorbed. | **INDETERMINATE — both suspected.** The two causes are not separable by these controls. **No classification number may be read off any run.** Routes to the same named remediation as the §1.1-pass/§1.2-reject cell (move the observation off the hot path), **and** to a re-run of **both** controls; if it repeats, it escalates to the **§8** named follow-on rather than being resolved at the keyboard. |

**A divergence is a finding to record, not to average away** — in every cell of the 2×2.

**Note on the remediation route.** Both blocking cells route to "move the observation off the hot path".
**Under SPEC-356b's pin that remediation is a `.rs` change and therefore CANNOT be executed there** — it is
a **new spec that re-pins**, and SPEC-356b's disposition is to record the cell, the effect sizes and the
route, and to stop. Absorbing it at the keyboard would break the pin and void every run.

### §1.4 — Family-wise error rate, and the reasoned decision NOT to α-correct

The control set is **two** tests, each at α = 0.05, and each is pre-registered as a **non-rejection**
predicate (rejection is the adverse outcome). Under independence the probability that **at least one**
control rejects when the instrument is in fact neutral is **1 − 0.95² = 0.0975 ≈ 9.8 %** — i.e. **roughly
a 1-in-10 chance of a spurious adverse reading across the control set**, not the 5 % a single test
suggests.

**No α correction is applied, deliberately:** these are **safety controls** where a false *adverse* reading
costs a re-run while a false *clean* reading would license invalid numbers, so the conservative direction
is the right one. The 9.8 % figure stands next to the two predicates so that **a single rejection is read
as what it is — one draw from a ~10 % family-wise rate — and not over-interpreted.**

---

## §2 — The ORDERED classification predicate

**This is where the ordered predicate lives. It is not in §0.** It is evaluated over the `long` run's last
half, normalized per epoch, with the `w100` cell as the keeping-up contrast.

**Why an ordered predicate rather than a table of bands.** A flat table of discriminators is not disjoint
here, and the overlap is the *likely* outcome: a prune that drains everything it is **licensed** to drain
while the **pinned** pool grows satisfies a naive "throughput" row (passes steady, `dropped ≈ considered`,
freed < added) **and** a naive "selection" row (ineligible share grows) simultaneously — and SPEC-355
measured exactly that shape (the prune fires, the LWM advances, `confirmErrors ≈ 3 %`, the reclaim
fraction falls anyway). Those two rows route to **opposite fix shapes**. So the discrimination is made by
**cause precedence, evaluated upstream-first**, as an **ORDERED** predicate.

**THE ORDERING IS FROZEN PRE-DATA AND IS UN-CHOOSEABLE AFTER THE DATA LANDS.** It is committed before any
`spec356-*.soak.json` exists, `git log --follow`-provable, exactly as §3's 10 % literal is.

### §2.1 — Pre-registered terms

All read from committed series; **no term is defined after the fact.**

- **`L`** — the **licensed backlog**: indexed tombstone refs in epochs satisfying
  `is_epoch_prune_eligible(E) && durable_epoch_watermark >= E`. This is the work the prune is licensed to
  do.
- **`P`** — the **pinned pool**: indexed refs *not* so satisfying (ineligible — pinned by
  `low_water_mark` or by `durable_epoch_watermark`).
- **`B`** — the **median refs per non-empty drain** over the last half. Self-calibrating: it is one prune
  batch.
- **"licensed work DRAINS"** ⟺ `min(L)` over the last half **≤ `B`** — what is left over is less than a
  single batch, i.e. the prune has caught up with its licence.
- **"PERSISTENT licensed backlog"** ⟺ `min(L)` over the last half **> `B`** — the prune never once catches
  up with its licence. **These two are exact complements on one predicate**, which is what makes Step 2 and
  Steps 3–4 mutually exclusive by construction rather than by judgement.
- **"non-drop exit share"** ⟺ `(matched_nothing + absent + restored_*) ÷ considered` over the last half —
  the **same statistic and the same 10 % literal** §3 freezes, reused deliberately so the two
  pre-registrations cannot drift apart.

### §2.2 — Step 0: admissibility (evaluated BEFORE any step below)

The predicate may be evaluated **only** if:

- **(a)** §1.3's 2×2 landed in its **CLEAN** or **BUILD-LINEAGE-EFFECT** cell, **and**
- **(b)** the frontier split reports **≥ 1 split recompute** in the window being read, **and**
- **(c)** every deciding column passed the runner's population check.

**If any limb fails ⇒ INDETERMINATE (Step 5); no later step is evaluated.**

**Limb (b) is the stale-split rule, and it is fail-closed** (see **§8A ADJ-1**, which adjudicated it
pre-data). The split's recompute triggers — LWM movement and non-empty drains — are exactly the events
that **stop happening** in the Step-3 (scheduling / LWM-stall) regime, i.e. the regime against which the
"ineligible share grows" discriminator has to be read. A split frozen at its last recompute therefore
reads as a fresh *"not growing"* precisely when it is least entitled to. The consuming rule is therefore:

> **Zero recomputes in the window read ⇒ the split is stale for that window and inadmissible. The
> predicate routes to INDETERMINATE (Step 5); NO later step is evaluated.**

The two first-class staleness series the instrument emits alongside the split — the `current_epoch` at
which the split was last computed, and the monotone recompute counter — are what make this limb decidable
from committed data rather than from inference.

### §2.3 — The steps, in their frozen evaluation order

| Step | Predicate (evaluated **in this order**; the FIRST step that holds IS the determination — later steps are not evaluated) | Determination |
|---|---|---|
| **1** | Non-drop exit share **> 10 %** | **SELECTION / FRONTIER** — *exit limb.* Refs are considered and **not dropped**; no downstream cadence or batch-size change can free them. The exit ledger names **which** exit. |
| **2** | (exit share ≤ 10 %) **and** licensed work **DRAINS** (`min(L) ≤ B`) | **SELECTION / FRONTIER** — *licensing limb.* The prune drains everything it is licensed to drain, so the deficit is in **what is licensed**, not in how fast it is worked. `P`'s trajectory and the claim-span / watermark series name **which conjunct pins it**. |
| **3** | (exit share ≤ 10 %) **and** **PERSISTENT** licensed backlog (`min(L) > B`) **and** passes-per-epoch last-half OLS slope rejects **negative** (α = 0.05) | **SCHEDULING / LICENSING** — the prune is licensed but is being run less and less often as the corpus grows. |
| **4** | (exit share ≤ 10 %) **and** **PERSISTENT** licensed backlog **and** passes-per-epoch slope does **not** reject negative | **THROUGHPUT** — churn outpaces a prune that is scheduled, licensed, and frees what it selects. Fix shape: width-relative cadence / batch size. |
| **5** | Step 0 fails, or the deciding columns are unreadable | **INDETERMINATE** — routes to the named follow-on run specified in **§8** **before the data exists**. Never "record a note". |

### §2.4 — Disjointness and exhaustiveness, argued not asserted

Step 1 partitions on one predicate (`> 10 %` vs `≤ 10 %`). Within `≤ 10 %`, Step 2 versus Steps 3–4
partitions on the exact complement pair `min(L) ≤ B` / `min(L) > B`. Within the persistent-backlog branch,
Steps 3 and 4 partition on reject-negative / not-reject-negative. **Every leaf is reached by exactly one
path**, and Step 5 catches the inadmissible case. **There is no cell in which two determinations can both
fire**, and the previously overlapping shape resolves deterministically.

### §2.5 — THE ROUTING CONSEQUENCE, STATED EXPLICITLY

The **modal** expected outcome — the prune drains its licensed work while the pinned pool grows — lands
unambiguously at **Step 2, SELECTION / FRONTIER (licensing limb)**, and **therefore feeds the REGISTRY
branch of the TODO-634 family — NOT a prune-accelerator.** Concretely: it is evidence for
`ceiling = min_live_claim − fixed_margin`, the `ReclamationRegistry` and the claim/retention model, and it
is **not** a licence to touch prune cadence or batch size. Under a flat table that same shape could have
been read as THROUGHPUT and routed to a cadence/batch-size fix — **a fix that cannot work, because the
prune is already draining everything it is permitted to drain.** Preventing exactly that mis-route is why
the ordering is frozen ahead of the data.

### §2.6 — Replication caveat, reported BESIDE the determination

The `long` and `w100` cells are **n = 1**. SPEC-355 §10.4.2's own lesson is that the **level** replicates
(5.4 % spread at width 1000) while the **slope** does not (it moved 2.0×, 4.6× and changed sign). The
classification therefore rests on a **single unreplicated 4 h series**, and any step that turns on a
*slope* — Step 3's passes-per-epoch fit — inherits that fragility directly. **This caveat is stated in the
same paragraph as the determination**, and that statement must say **which steps of the predicate turned
on a level or a count (more robust) versus on a slope (less robust)**.

---

## §3 — The cell-E firing rule

The 2026-07-13 → 2026-07-27 interval matters iff the classification names a mechanism the two OR-path
merges in it could plausibly have introduced — `6c35785a` (sf-346, ormap WAL) and `2769570f` (sf-347,
ormap RSS **in-place mutate**), the latter being the very `update_in_place` shape the prune drop uses.

- **RUN cell E** iff the determination is **SELECTION/FRONTIER**, **or** the non-`Dropped` exits
  (`matched_nothing + absent + restored_*`) exceed **10 %** of considered refs over the `long` run's last
  half. Both are in-place-mutate / storage-residency-shaped.
- **CLOSE cell E as not needed** iff the determination is **THROUGHPUT** or **SCHEDULING** **and** the
  non-`Dropped` exit share is **≤ 10 %** — a prune that frees what it selects cannot be explained by that
  interval's mechanism — recorded **with the ledger numbers that ruled it out**.
- **The 10 % literal is arbitrary-but-fixed. Its role is to make the decision un-chooseable after the data
  lands**, and it is frozen here before the runs exist.
- **Coherence with §2's ordered predicate (checked, not assumed).** §2 Step 1 uses the **same statistic and
  the same 10 % literal**, so the two pre-registrations cannot drift. The consequence is that the rule
  above is **self-consistent by construction**: a THROUGHPUT or SCHEDULING determination can only be
  reached via Step 3 or Step 4, both of which **require** exit share ≤ 10 % — so the CLOSE branch's second
  conjunct is automatically satisfied whenever its first is, and a SELECTION/FRONTIER determination reached
  via Step 1 automatically satisfies the RUN branch. **Both numbers are still quoted explicitly; the
  redundancy is a cross-check, not a licence to omit one.**

---

## §4 — Cell E's protocol, if it fires

Exactly `spec355-manifest.md` §4.6: a **pre-346** binary (last merge before `6c35785a`) at width 1000 for
1800 s, **built and driven by the §4.2 half-swap** (instrumented harness, pinned pre-346 server), with
**both** `spec355-manifest.md` §4.3 identity checks, read **corpus-scan-only** — CSV `tombstone_bytes`
plus the redb corpus scan, and **no slope from that binary's gauge**, because a pre-346 pin predates
`69a5fd1f` (sf-351) and lacks the scoped-sink gauge `TG-OR-004` covers.

**Cell E carries NO prune-record columns**: the pre-346 server has no recorder, and expecting them is a
category error.

**Disposition.** Non-reproduction localizes a regression to sf-346 / sf-347 and is **recorded and spun
off** (SPEC-356 fixes nothing); reproduction widens the no-regression claim back to 2026-07-13 as a
*measured* statement.

**Provenance is fail-closed.** `SOAK_SERVER_BINARY` must be set and executable before the clock; and
afterwards `orDeltaFrames == 0` with `orSnapshotFrames > 0`. Nonzero ⇒ **INVALID**: abort and re-run,
never routed into a decision row.

---

## §5 — CSV schemas, and the PINNED metric names

### §5.1 — The primary CSV

`spec356-<cell>.csv` keeps the SPEC-355 header and cadence **byte-identical**, so the 8-window fit
SPEC-356b computes is produced by the same instrument over the same shape as SPEC-355's:

```
elapsed_secs,rss_mb,wal_mb,redb_mb,disk_total_mb,tombstone_bytes
```

**Cadence: 60 s.** The `rss_mb` column is present **only** because this header is frozen byte-identical;
per §0(b) and §0(c) it is **FORFEIT** and may not be read in either direction.

### §5.2 — The prune-record CSV

A **second** sampler writes `spec356-<cell>.prune.csv` at a **10 s** cadence, carrying the **raw cumulative
counters and instantaneous gauges** — **deltas are computed post-hoc, never in the sampler**. Its columns
are: the inherited `elapsed_secs`; one column per pinned series of §5.3, with **each histogram
contributing both its `_sum` and its `_count` column**; and the inherited monotone
`topgun_ormap_tombstone_bytes_total`, carried so that **added bytes are exact** rather than the
60 s-sampled lower bound SPEC-355 had to caveat.

*Caveat recorded here because it is decision-affecting:* that inherited counter is also driven once at boot
by the `set_tombstone_bytes` recovery seed — with `crash-interval 0` it fires **at most once, before any
add**, guarded by the existing tripwire.

### §5.3 — The pinned metric names (names only — deliberately NO `file:line`)

**These names are the frozen set. They are pinned here, before the emitters exist.** A name that differs
from this list, or a series emitted under the pinned prefix that this list does not name, is a **blocking
defect, repaired by fixing the CODE — never by editing this section.**

**This section freezes the NAMES and carries no `file:line` citation, deliberately**, because the emitters
do not exist when it is written and a citation to code that does not yet exist would be manufactured. The
verification at the emitting `file:line`, and the proof of presence in a live `/metrics` scrape, is a
**second pass** and lands in SPEC-356a's own acceptance criteria and validation checklist — never here.

All 33 names share **one collision-free prefix**, visible in every row below. Naming convention follows the
tree's existing shape: **monotone counters end `_total`; gauges carry no suffix; histograms carry no suffix
and export `_sum` / `_count` totals alongside the exporter's rendered quantiles** (which §5.4 binds).

**Counters (monotone, `_total`) — 15:**

| Name | Answers |
|---|---|
| `topgun_or_prune_passes_total` | prune passes run — incremented on **every** prune-loop invocation, empty drains included, at the invocation and outside the loop body; `passes == empty_drains + nonempty_drains` is test-enforced |
| `topgun_or_prune_considered_total` | refs considered — the exhaustiveness identity's **LHS**, and §2's exit-share **denominator** |
| `topgun_or_prune_dropped_total` | exit: **Dropped** |
| `topgun_or_prune_matched_nothing_total` | exit: **MatchedNothing** |
| `topgun_or_prune_absent_total` | exit: **AbsentKey** (consumes a ref with **no** gauge decrement) |
| `topgun_or_prune_restored_read_error_total` | exit: **RestoredReadError** |
| `topgun_or_prune_restored_evicted_total` | exit: **RestoredEvicted** |
| `topgun_or_prune_restored_write_error_total` | exit: **RestoredWriteError** |
| `topgun_or_prune_bytes_freed_total` | bytes freed by the prune |
| `topgun_or_prune_epochs_drained_total` | epochs drained |
| `topgun_or_prune_empty_drains_total` | empty drains, counted **separately** |
| `topgun_or_prune_nonempty_drains_total` | non-empty drains — the denominator for every per-drain mean, and the perturbation-budget check |
| `topgun_or_prune_lwm_advances_total` | LWM advances |
| `topgun_or_prune_lwm_epochs_advanced_total` | total epochs the LWM advanced |
| `topgun_or_prune_split_recomputes_total` | the monotone recompute counter §2.2 limb (b) reads |

**Gauges — 11:**

| Name | Answers |
|---|---|
| `topgun_or_prune_indexed_refs` | indexed tombstone refs (O(1)-maintained) |
| `topgun_or_prune_indexed_epochs` | indexed epochs (O(1)-maintained) |
| `topgun_or_prune_eligible_refs` | **`L`** — the licensed backlog |
| `topgun_or_prune_ineligible_refs` | **`P`** — the pinned pool |
| `topgun_or_prune_split_computed_epoch` | the `current_epoch` at which the split was last computed (§2.2's staleness marker) |
| `topgun_or_prune_current_epoch` | `current_epoch` |
| `topgun_or_prune_low_water_mark` | `low_water_mark` |
| `topgun_or_prune_durable_epoch_watermark` | `durable_epoch_watermark` |
| `topgun_or_prune_last_drained_epoch` | the id of the most recently drained epoch |
| `topgun_or_prune_lwm_stall_seconds` | current stall — time since the last LWM advance |
| `topgun_or_prune_tracked_claims` | tracked-claim count |

**Histograms — 7.** Each exports `_sum` and `_count`; **the `_sum` / `_count` pair is the PRIMARY reading,
and the rendered quantile is corroboration only** (§5.4).

| Name | Answers |
|---|---|
| `topgun_or_prune_drain_refs` | refs per **non-empty** drain — the series **`B`** (median refs per non-empty drain) is computed from |
| `topgun_or_prune_drain_epochs` | epochs per non-empty drain |
| `topgun_or_prune_claim_span_epochs` | `current_epoch − low_water_mark` at each LWM movement and each non-empty drain — the evidence base for `ceiling = min_live_claim − fixed_margin` |
| `topgun_or_prune_claim_lag_epochs` | per-tracked-claim lag at those same instants |
| `topgun_or_prune_epoch_considered` | per **drained epoch**: refs considered |
| `topgun_or_prune_epoch_dropped` | per **drained epoch**: refs dropped |
| `topgun_or_prune_epoch_bytes_freed` | per **drained epoch**: bytes freed |

**33 names, and THE SET IS CLOSED at this commit.** If execution finds that a requirement genuinely needs a
series this list does not name, the licensed paths are, in order: **(i)** before this section is committed —
surface it as a **split trigger** to the user and add it to the source table and to this section together,
never at one worker's keyboard; **(ii)** after this section is committed but before the first
`spec356-*.soak.json` — a **PRE-DATA §8A adjudication addendum**, which may adjudicate; **(iii)** after the
first measurement artifact — **it is not addable at all**, and routes to a **re-pinning spec**. Emitting an
un-listed series under the pinned prefix is a defect in every one of those windows.

### §5.4 — Quantile aliasing (recorded before the data exists)

The installed exporter renders quantiles over a **`3 × 20 s` = 60 s rolling window**, so the **10 s**
`prune.csv` cadence **over-samples the same window 6×**. Consecutive rows of a quantile column are
therefore **heavily autocorrelated and are NOT independent observations:**

> **No n, no sd, no standard error and no t-statistic may be computed by counting `prune.csv` rows of a
> quantile column.**

The verified companion fact: **`_sum` and `_count` ARE cumulative** while the rendered quantiles are not.
That is why the **per-interval means differenced from the monotone totals are the PRIMARY reading**, and
any exporter-rendered quantile is **corroboration only**. The 10 s cadence is kept deliberately — it makes
the monotone-counter differencing fine-grained — and **the aliasing constraint binds the quantile columns
only**.

---

## §6 — Reserved

RESERVED — intentionally empty at freeze (SPEC-356a R8.0a's section map).

---

## §7 — Reserved

RESERVED — intentionally empty at freeze (SPEC-356a R8.0a's section map).

---

## §8 — The INDETERMINATE routing target

§2 Step 5 and §1.3's INDETERMINATE cell both defer to "the named follow-on run specified in §8 **before the
data exists**". This is that section, and it is pre-registration content like every other one.

### §8.1 — One repeat, at DOUBLED duration AND DOUBLED replicates

An INDETERMINATE outcome (§2 Step 5, or §1.3's INDETERMINATE cell after its own re-run) routes to **exactly
one repeat** of the deciding configuration: **the `long` cell re-run at 28,800 s (2 × 14,400 s) with n = 2
(2 × 1).**

Doubling **both** axes is deliberate, and each axis answers a different failure: doubling **duration**
addresses an effect too slow to separate in 4 h, and doubling **replicates** addresses the n = 1 fragility
SPEC-355 §10.4.2 already named (the level replicates at 5.4 % spread; the slope moved 2.0×, 4.6× and
changed sign). **One axis alone leaves the other cause unaddressed.**

**The repeat is run under the SAME pin and the SAME frozen predicate** — it is a second observation, not a
re-specification, and **it may not adjust a threshold, an ordering or a conditional.**

### §8.2 — If the repeat is STILL INDETERMINATE, the cause escalates as an EXPLICIT INPUT

Owner: **TODO-634** (the ReclamationRegistry family umbrella). The escalation is **not "record a note"**:
it **names the unclassified cause, quotes the evaluated value of every step that failed to fire and the
admissibility limb that blocked**, and is carried into the family's design phase **as a stated open input**.

### §8.3 — And the family is NOT blocked by it, which is why this is a legitimate terminal branch

**The recommended reclamation model closes safety REGARDLESS of which cause it turns out to be.**
`ReclamationRegistry` (cursor-shaped consumers only) + retention SLA **N = 30 d** + the cursor-age fence
with HLC-horizon quarantine + `ceiling = min_live_claim − fixed_margin` bound the reclaimable set by **live
claims**, not by any hypothesis about *why* the current prune falls behind. **A selection defect, a
scheduling defect and a throughput defect are all *contained* by a registry that never reclaims below a
live claim.**

What an unclassified cause costs is **fix-shape efficiency** — the family would design without knowing
which limb to optimize first — **not safety, and not the family's ability to proceed.** A Step-5 outcome is
therefore to be read as **an expensive answer, not a blocked one**, and any Step-5 outcome must be reported
quoting this paragraph beside it.

---

## §8A — Adjudication Addenda

**The frozen text is NEVER edited; findings against it land here.** A pre-registered text whose value is
being un-editable still has to survive being found wrong — by SPEC-356a's `/xask` round, by SPEC-356b's
audit, or by SPEC-356b's execution. This section is the single mechanism that replaces every earlier
precedence rule.

- **Append-only, and the original is NEVER overwritten.** No byte of §0–§8 is ever edited, re-worded or
  deleted — not by SPEC-356a after these sections are authored, not by SPEC-356b, not by any later spec. An
  addendum **quotes the original text VERBATIM** and records the adjudication **beneath it**.
- **Required form of each addendum**, so it is machine-readable and cannot be a paragraph: `ADJ-<n>`
  (sequential) · **date** · **target** (the section and the exact rule) · **ORIGINAL TEXT (verbatim block
  quote)** · **FINDING** (what is wrong, and who found it: `/xask`, an audit, an execution observation) ·
  **ADJUDICATED FORM** (the rule that now governs) · **AUTHORITY** (the ruling, finding id or audit item) ·
  **PRE-DATA / POST-DATA**.
- **Consumers read §0–§8 TOGETHER WITH §8A.** Where an addendum adjudicates a rule, **the adjudicated form
  governs**; the original stands in the record as what was originally committed.
- **PRE-DATA versus POST-DATA is decided by the DATA BOUNDARY, not by authorship and not by self-report.**
  An addendum committed **before** the first `spec356-*.soak.json` exists is **PRE-DATA** and **may
  adjudicate** — a correction made while the data still cannot influence it is strictly better than
  freezing a known-wrong rule. An addendum committed **after** the first measurement artifact exists is
  **POST-DATA** and **MUST NOT alter any predicate, threshold, ordering or conditional**; it may only
  record the finding and route it to a named follow-on. **`git log --follow` decides.** In particular
  SPEC-356b's first-wave addenda are **PRE-DATA**, because that wave runs before its first cell.

### ADJ-1

- **ADJ id:** ADJ-1
- **Date:** 2026-08-03
- **Target:** **§2.2, Step 0 limb (b)** — the stale-split consuming rule.
- **ORIGINAL TEXT (verbatim, from SPEC-356a R1.3a as drafted):**

  > "zero recomputes in a window ⇒ the split is stale for that window and is inadmissible as evidence for
  > or against Step 3, routing to Step 2's discriminator or to INDETERMINATE"

- **FINDING:** the two halves of SPEC-356 stated the same conditional differently. Step 0 is a
  **fail-closed** admissibility gate (*"if any limb fails ⇒ INDETERMINATE (Step 5); no later step is
  evaluated"*), while the text above offers a **fallback route to Step 2's discriminator**. **A stale
  sample routed onward to a *different* discriminator is a live sample by another name.** Raised by the
  SPEC-356a Audit v1, item C2.
- **ADJUDICATED FORM (governs):** **a stale split routes to INDETERMINATE (§2 Step 5). NO later step is
  evaluated.** The looser fallback is **WITHDRAWN**. The instrument obligation is unchanged and
  unconditional: emit the last-computed-epoch marker and the monotone recompute counter, and sample both in
  `prune.csv`.
- **AUTHORITY:** user ruling on SPEC-356a Audit v1 C2, 2026-08-03.
- **PRE-DATA / POST-DATA:** **PRE-DATA** — committed at the freeze boundary, before any
  `spec356-*.soak.json` exists.

### ADJ-2

- **ADJ id:** ADJ-2
- **Date:** 2026-08-03
- **Target:** **§2.1** — the aggregator in the definitions of *"licensed work DRAINS"* and *"PERSISTENT
  licensed backlog"*.
- **ORIGINAL TEXT (verbatim, §2.1):**

  > - **"licensed work DRAINS"** ⟺ `min(L)` over the last half **≤ `B`** — what is left over is less than a
  >   single batch, i.e. the prune has caught up with its licence.
  > - **"PERSISTENT licensed backlog"** ⟺ `min(L)` over the last half **> `B`** — the prune never once catches
  >   up with its licence. **These two are exact complements on one predicate**, which is what makes Step 2 and
  >   Steps 3–4 mutually exclusive by construction rather than by judgement.

- **FINDING:** `min` is the **extreme order statistic**, and the two branches it defines are wildly
  asymmetric in how easily they trigger. The `long` cell's last half is ~2 h sampled at 10 s ⇒ **~720
  samples**. `min(L) ≤ B` fires if the prune catches up **once in 720 samples** — a single lull, one empty
  drain, one moment of workload jitter. Its complement requires the prune to catch up **never**. Step 2 is
  therefore close to a default branch, and Step 2 is the branch §2.5 routes to the **REGISTRY** family — so
  §2.5's claim that *"the modal expected outcome … lands unambiguously at Step 2"* is partly an artefact of
  the **aggregator** rather than a property of the system. A concrete, physically plausible mechanism is
  thereby **misrouted**: a prune that drains its licence completely on every pass but whose passes become
  rare (per-pass scan cost growing with the corpus) has `L ≈ 0` immediately after each pass, so `min(L) ≤ B`
  fires, Step 2 is the determination, and the passes-per-epoch slope test that would have named the real
  cause is **never evaluated** because Step 2 short-circuits it. The fix shape it routes to — the claim /
  retention model — cannot repair a cadence collapse. Raised by the SPEC-356a `/xask` pre-registration round
  (findings **X1**, **X2**), 2026-08-03.
- **ADJUDICATED FORM (governs):** the aggregator on the `L` side becomes the **median**, matching the
  statistic already used on the `B` side:
  > - **"licensed work DRAINS"** ⟺ **`median(L)` over the last half ≤ `B`** — equivalently, `L ≤ B` on **at
  >   least 50 %** of last-half samples: the prune is at or inside one batch of its licence at least half
  >   the time.
  > - **"PERSISTENT licensed backlog"** ⟺ **`median(L)` over the last half > `B`** — its **exact
  >   complement**, so §2.4's disjointness and exhaustiveness argument stands unchanged in structure.
  >
  > **`min(L)` and `max(L)` are RETAINED as reported statistics** beside the determination, so the
  > single-dip case remains visible in the record; they no longer decide.

  **Why the median and not some other percentile:** `B` is already defined in §2.1 as *"the **median** refs
  per non-empty drain"*. The original rule compared an **extreme order statistic against a median**, which is
  the mismatch this addendum removes; median-against-median is the coherent comparison, and it needs no new
  literal to be chosen with no prior to choose it from. Under it, the misrouted mechanism above has `L > B`
  for most of the window and correctly falls through to Steps 3–4.
- **AUTHORITY:** `/xask` cross-vendor pre-registration round, findings X1 and X2; disposition recorded in
  `spec356-xask-preregistration.md`.
- **PRE-DATA / POST-DATA:** **PRE-DATA** — committed before any `spec356-*.soak.json` exists, and before the
  freeze boundary. The data cannot have influenced it.

### ADJ-3

- **ADJ id:** ADJ-3
- **Date:** 2026-08-03
- **Target:** **§2.3** — the short-circuit rule, and what a short-circuited determination must still report.
- **ORIGINAL TEXT (verbatim, §2.3 column header):**

  > Predicate (evaluated **in this order**; the FIRST step that holds IS the determination — later steps are
  > not evaluated)

- **FINDING:** the ordering is **upstream-first**, which assumes an upstream cause produces downstream
  symptoms but not the reverse. In a prune loop with feedback that is false: a downstream cause (per-pass
  cost growth) produces **fewer passes** ⇒ **persistent backlog** ⇒ **LWM does not advance** ⇒ **pinned pool
  grows**, i.e. it mimics every upstream symptom. The predicate fires on the first upstream symptom it
  meets and the evidence that would have indicted it — the passes-per-epoch fit — is **never computed**,
  because "later steps are not evaluated" is read as licensing its omission. Raised by the `/xask` round
  (finding **X3**), 2026-08-03.
- **ADJUDICATED FORM (governs):** **the short-circuit governs the DETERMINATION; it does NOT license
  omitting the EVIDENCE.** The ordering is **not** reversed — reversing it would re-open exactly the
  mis-route §2.5 exists to prevent, and choosing which direction of error to take is the pre-registration's
  to make. What is added is an unconditional reporting obligation:

  > **Whatever step fires, the determination is reported together with ALL of:** (i) the passes-per-epoch
  > last-half OLS fit and its α = 0.05 test result — **computed and reported even when Steps 3/4 are not
  > reached**; (ii) `min(L)`, `median(L)`, `max(L)` and the fraction of last-half samples with `L ≤ B`;
  > (iii) `B`; (iv) the non-drop exit share and the per-exit breakdown from the ledger.
  >
  > **A Step-1 or Step-2 determination that coincides with a significantly NEGATIVE passes-per-epoch slope
  > is reported as CONTESTED**, naming the feedback alternative explicitly. CONTESTED does not change the
  > determination; it forbids reporting it as uncontested.

- **AUTHORITY:** `/xask` round, finding X3; and finding X9's disposition, which relies on this obligation.
- **PRE-DATA / POST-DATA:** **PRE-DATA.**

### ADJ-4

- **ADJ id:** ADJ-4
- **Date:** 2026-08-03
- **Target:** **§2.4** — what exhaustiveness does and does not buy.
- **ORIGINAL TEXT (verbatim, §2.4):**

  > **Every leaf is reached by exactly one path**, and Step 5 catches the inadmissible case. **There is no
  > cell in which two determinations can both fire**, and the previously overlapping shape resolves
  > deterministically.

- **FINDING:** the claim is **true and is exactly what creates the gap**. Steps 1–4 exhaustively partition
  the **admissible** space, and Step 5 catches only **admissibility** failure — a data-quality gate, not a
  conceptual one. There is no *"the data is admissible but none of the four causal stories fit"* branch, so
  a mechanism outside the four hypothesised ones still receives one of the four frames — **endorsed by a
  pre-registered rule**, which makes the wrong answer harder to challenge than an un-pre-registered one
  would be. The predicate is falsifiable only downstream, when the fix it routes to fails. Raised by the
  `/xask` round (finding **X4**), 2026-08-03.
- **ADJUDICATED FORM (governs):** no fifth branch is added — no condition over admissible data could
  trigger one without the judgement the pre-registration exists to remove. The claim strength is bounded
  instead:

  > **A determination is reported as "the best-supported of the FOUR PRE-REGISTERED mechanisms", never as
  > "the cause".** The report states explicitly that **a mechanism outside the four is not excluded**, and
  > names the residual evidence that would indicate one (per-pass cost growth, index-scan cost, allocator
  > or page-cache effects — none of which the 33-series instrument observes).
  >
  > This bound is what finding **X8**'s disposition relies on: a small but persistent non-drop exit share
  > that never crosses the 10 % literal does not decide, but it **is** in the committed series and the
  > per-exit breakdown ADJ-3 mandates puts it in the report.

- **AUTHORITY:** `/xask` round, finding X4 (and X8's disposition).
- **PRE-DATA / POST-DATA:** **PRE-DATA.**

### ADJ-5

- **ADJ id:** ADJ-5
- **Date:** 2026-08-03
- **Target:** **§1.3** — the `§1.1 rejects / §1.2 passes` cell, and the scope of what §1.2 exonerates.
- **ORIGINAL TEXT (verbatim, §1.3, the `§1.1 rejects` / `§1.2 passes` cell):**

  > **BUILD-LINEAGE EFFECT — NOT an instrument defect.** The instrument is exonerated by the higher-powered,
  > better-controlled test; the difference is attributed to the lineage break §0(b) already declared. **The
  > classification numbers STAND.** Recorded as a finding, with both series and the effect size, and quoted
  > beside the determination — never averaged away, never silently absorbed.

- **FINDING:** two structural blind spots, both by construction.
  **(i) §1.2 tests ACTIVATION, not PRESENCE.** `ctl` and `ctloff` are the **same binary**; arming is a
  runtime flag. Compiled-in branches, inlining decisions and code layout are **identical in both arms**, so
  the higher-powered control **cannot see them** — while §1.1, which can, has its rejection absorbed here as
  "build-lineage effect" and the numbers allowed to stand. The exoneration is therefore narrower than the
  cell's wording claims.
  **(ii) Both measured controls are LEVEL controls; the classification reads DYNAMICS.** §1.1 and §1.2 both
  compare **last-half level means** of `tombstone_bytes`. The predicate reads the non-drop exit share, the
  `L`/`P` split and a slope. A perturbation that shifts the **dynamics** without shifting the steady-state
  **level** passes both controls and still moves the determination.
  Raised by the `/xask` round (findings **X5**, **X6**), 2026-08-03.
- **ADJUDICATED FORM (governs):** the 2×2 is **not** restructured — §1.2 remains the better test of
  activation and the two controls are still not OR'd. The cell's disposition is tightened and the controls
  gain a descriptive companion:

  > **"STAND" becomes "STAND, BOUNDED AND FLAGGED".** The cell must report the observed cross-lineage
  > effect size as an **explicit bound on unexplained shift**, and **any determination whose deciding
  > statistic differs from its threshold by less than that bound is reported FRAGILE**.
  >
  > **The exoneration is scoped in words:** §1.2 exonerates the instrument's **activation**. It does **not**
  > and cannot exonerate the instrument's **presence in the binary**, which is common-mode across both its
  > arms. Any residual cross-lineage shift is therefore reported as *"attributable to the declared lineage
  > break **and/or** to instrument presence, not separable by this control set"* — never as
  > *"attributable to the lineage break"* alone.
  >
  > **The armed-vs-disarmed comparison is ALSO reported for the three dynamics statistics the predicate
  > reads** — non-drop exit share, `median(L)/B`, and the passes-per-epoch slope — as **descriptive
  > companions**. **No new predicate and no new α**: these add nothing to the family-wise rate §1.4 computes
  > and cannot reject on their own. They exist so that a dynamics perturbation invisible to the level
  > controls is at least **visible in the record**.

- **AUTHORITY:** `/xask` round, findings X5 and X6.
- **PRE-DATA / POST-DATA:** **PRE-DATA.**

### ADJ-6

- **ADJ id:** ADJ-6
- **Date:** 2026-08-03
- **Target:** **§1.2** — the *"one difference"* claim, and the sampler's scope on a disarmed cell.
- **ORIGINAL TEXT (verbatim, §1.2):**

  > `ctl` (armed) versus `ctloff` (disarmed) — **one build, one lineage, one difference**. Same predicate as
  > §1.1, MDE stated the same way, computed from that pair's own observed pooled sd.

- **FINDING:** *"one difference"* holds only if **everything else is common-mode**, and the second
  (`prune.csv`) sampler's own 10 s scrape is not obviously so. The governing rules collide: the sampler's
  completeness gate is stated **unconditionally** (*a row is written only from a scrape carrying **all 35
  series**; failure to obtain one inside the readiness window is `fail_instrument` / `exit 9`*), while the
  per-cell context states that on a **disarmed** cell **no prune-record column is checked at all**, because
  the arming witness requires those series to be **absent**. Read together, an unscoped gate makes a
  disarmed cell **unrunnable** — the 33 prune-record series are absent by design, so no scrape is ever
  "complete" and the cell exits 9. The obvious repair — *don't sample disarmed cells* — is worse: it puts
  the sampler's `curl` and file I/O on the **armed arm only**, making it a **second** difference and
  falsifying the very claim this control rests on. Raised by the `/xask` round (finding **X7**),
  2026-08-03; the underspecification is real and is resolved here rather than at the keyboard.
- **ADJUDICATED FORM (governs):**

  > **The second sampler runs IDENTICALLY on armed and disarmed cells** — same 10 s cadence, same scrape,
  > same row-writing cadence — so its cost is **common-mode** and *"one difference"* is restored as a true
  > statement.
  >
  > **The 35-series completeness gate is scoped to ARMED cells.** On a **disarmed** cell the sampler writes
  > its row with the prune-record columns **empty**, and **no column check reads them** — which the per-cell
  > context already provides for. **Series absence on a disarmed cell is never `exit 9`**; it is the
  > **arming witness**, checked once and fail-closed, exactly as specified.
  >
  > **Unaffected, stated so no gate is read as weakened:** the zero-empty-field census (**EG-1** limb (b))
  > is taken over the **ARMED** `smoke` cell, where the completeness gate is in full force; the `empty > 0`
  > limb likewise binds only where a column is checked at all. Nothing here relaxes either.

- **AUTHORITY:** `/xask` round, finding X7.
- **PRE-DATA / POST-DATA:** **PRE-DATA.**

*(Subsequent addenda are appended below these as `ADJ-7`, `ADJ-8`, … in the same form. Nothing above is
edited to accommodate them.)*

### ADJ-7

- **ADJ id:** ADJ-7
- **Date:** 2026-08-04
- **Target:** **§2.1** — the definition of `L`, the licensed backlog (and, through it, Step 2 versus Steps
  3–4 in §2.3 and the routing consequence §2.5 draws from them).
- **ORIGINAL TEXT (verbatim, §2.1):**

  > - **`L`** — the **licensed backlog**: indexed tombstone refs in epochs satisfying
  >   `is_epoch_prune_eligible(E) && durable_epoch_watermark >= E`. This is the work the prune is licensed to
  >   do.

- **FINDING — the definition was right and the SERIES did not implement it.** The emitted series
  `topgun_or_prune_eligible_refs` was recomputed at the drain **after** `drain_prunable` had already removed
  every eligible epoch, and that drain is **unbounded** — it takes every ref it just counted. The drain's
  sample was therefore **0 by construction**, on every pass, whatever the backlog had been. The other three
  recompute sites (the low-water-mark movements) can observe a non-zero backlog, but the drain runs orders of
  magnitude more often and its zero is the last writer before any 10 s scrape.
  **The committed evidence already shows it**, which is why this is a measurement rather than an argument:
  `spec356a-eager-registration.log` reports `topgun_or_prune_eligible_refs [POPULATN] n=13 empty=0 min=0
  max=0` on a smoke cell that recorded 3 non-empty drains, 5 LWM advances, 8 split recomputes and an index
  peaking at 2,849 refs. Thirteen samples, flat zero.
  **Consequence under §2 as adjudicated by ADJ-2:** `median(L) ≤ B` holds for **any** `B ≥ 0`, so *"licensed
  work DRAINS"* is true by construction; Step 2 fires whenever the exit share is `≤ 10 %`, Steps 3–4 become
  unreachable, and §2.5's *"the modal outcome lands unambiguously at Step 2 → REGISTRY"* follows from the
  emitter rather than from the prune's behaviour. This is the **structural** form of the misclassification
  `/xask` **X2** described as a contingency: X2 argued the backlog would *often* read near zero right after a
  pass; the emitter guaranteed it *always* would. **ADJ-2 does not reach it** — the bias was in the sampling
  instant, not in the aggregator, so replacing `min` with `median` changes nothing about a series that is
  zero at every instant.
  Found by the SPEC-356a **Review v1, finding C1**, 2026-08-04.
- **ADJUDICATED FORM (governs):**

  > **§2.1's definition of `L` STANDS, unedited.** The predicate was never wrong; the instrument was. This
  > addendum records an **instrument-side repair**, not a change of rule: no threshold, no ordering, no
  > conditional and no aggregator moves.
  >
  > **The drain samples the split BEFORE its removal loop.** `drain_prunable` snapshots the
  > eligible / ineligible split on the near side of the work that consumes it and returns it with the drained
  > refs; the post-loop recompute is deleted. The snapshot is gated on a **non-empty eligible set**, so the
  > per-`OR_REMOVE` path still pays no index-proportional fold and §1's perturbation budget is unchanged. The
  > three low-water-mark recompute sites are untouched.
  >
  > **CONSEQUENCE THE 356b ANALYST MUST READ BEFORE COMPUTING ANY SHARE: the split gauges and the index
  > gauges are now published from TWO DIFFERENT INSTANTS of the same scrape.** Moving the split to the near
  > side of the removal loop is what makes `L` a reading, and it necessarily de-synchronises the split from
  > the index. Explicitly, per scrape:
  >
  > | series | which side of the drain it samples |
  > |---|---|
  > | `topgun_or_prune_eligible_refs` (`L`) | **PRE-drain** — the snapshot taken before the removal loop |
  > | `topgun_or_prune_ineligible_refs` (`P`) | **PRE-drain** — the same snapshot, same instant as `L` |
  > | `topgun_or_prune_indexed_refs` | **POST-drain** — the observation snapshot after the loop |
  > | `topgun_or_prune_indexed_epochs` | **POST-drain** — the same observation snapshot |
  >
  > **Therefore `L + P ≠ indexed_refs`, by design and not by drift.** The directed regression asserts exactly
  > this shape: `3 + 1` against `indexed_refs = 1`. `L` and `P` remain mutually consistent — one snapshot,
  > one instant — so every ratio *between them* is sound; what is NOT sound is a ratio taken **across** the
  > boundary. **An "ineligible share" computed as `P / indexed_refs`, or a "backlog share" as
  > `L / indexed_refs`, is a quantity with no referent and MUST NOT be reported.** Use `L / (L + P)` if a
  > share is wanted.
  >
  > **This costs §2 nothing, which is why the repair is admissible at all:** no term of the frozen
  > classification predicate divides by `indexed_refs` or by `L + P`, so no threshold, ordering or
  > determination in §1–§2 reads across the boundary. The hazard is entirely in *derived* reporting a 356b
  > analyst might invent, which is why it is stated here rather than left in the regression test's comment
  > where only a reader of that file would find it.
  >
  > **A binary that predates this repair emits an `L` that is inadmissible for §2.** Any cell measured
  > against such a build reads `L ≡ 0` and MUST NOT be classified — its Step-2 determination would be a
  > reading of the emitter. SPEC-356b pins the merged SPEC-356a SHA as its build identity, so this is
  > enforced by the pin rather than by vigilance.
  >
  > **The flat-zero census above is retained as the before-picture.** It stays in the record as the evidence
  > that produced this addendum, and the repair carries a directed regression cell in which the licensed
  > backlog reads **3** where the pre-repair emitter would read **0** and a stale split would read **1**.
  >
  > **A ZERO IS NOW A READING, NOT A CONSTRUCTION — and the residency bound is stated here rather than left
  > to be discovered.** The repaired instrument was run on a real armed smoke cell
  > (`spec356a-backlog-and-failpath-demos.log`, Part 1): the licensed backlog reads **1000** — one epoch's
  > refs at width 1000 — at **2 of 12** sample instants, `max = 1000` where the pre-repair census read
  > `max = 0`. **Ten of the twelve rows still read 0, and a second run of the same cell caught none at all.**
  > That is the world, not the defect returning: this prune sweeps on **every `OR_REMOVE`** (4,288 passes in
  > 120 s on that cell), so a newly-licensed ref is drained within milliseconds and the backlog is
  > **transient — under the smoke's churn shape, and that scope is the whole of what has been observed.**
  >
  > **THE EVIDENCE BASE FOR THE TRANSIENCE CLAIM IS A NON-MEASUREMENT CELL, AND THE CLAIM IS SCOPED TO IT.**
  > The two runs cited above are 120 s scratch smoke cells whose duration and cadence differ from every R4.4
  > measurement cell. "Transient" is therefore asserted **for the smoke's churn shape only** — its client
  > count, keyspace, remove rate and 120 s window — and **NOT** for width 1000 in general. **The 4 h `long`
  > cell owns the general claim**, and nothing here pre-empts it: a regime in which the prune cannot keep up
  > would show exactly the persistent backlog this addendum says is contradictable, and the smoke has no
  > standing to rule that out. A SPEC-356b reader must treat the sentence below as a statement about what has
  > been seen, never as a prediction the measurement is expected to confirm.
  >
  > The consequences are pre-registered here so no later reader has to infer them:
  >
  > - **A 10 s gauge sample of a transient quantity is a LOWER BOUND on it.** `median(L)` may therefore
  >   still read 0 in a regime where the prune is keeping up, and **Step 2 may still be the modal outcome
  >   under the smoke's churn shape** — an observation about what was seen there, not a forecast for the
  >   measurement cells.
  >   What has changed is that this is now **contradictable by data**: a prune that cannot keep up leaves a
  >   backlog that persists ACROSS sample instants and is caught, where the pre-repair emitter would have
  >   reported 0 for that regime too.
  > - **A Step-2 determination must be reported together with `max(L)` over the window**, not with the
  >   median alone. A window whose `max(L)` is also 0 is a window in which the sampler never observed a
  >   non-empty drain, and that is an admissibility observation (§2.2 limb (b)'s territory), not evidence
  >   that the prune is licensed-and-draining.

- **AUTHORITY:** SPEC-356a Review v1 finding C1; user ruling 2026-08-04 (*"the post-drain zeroed read
  dies"*).
- **PRE-DATA / POST-DATA:** **PRE-DATA** — committed while no `spec356-*.soak.json` exists anywhere in the
  history (`git log --all --diff-filter=A -- '*spec356*soak.json'` is empty); `git log --follow` decides.

### ADJ-8

- **ADJ id:** ADJ-8
- **Date:** 2026-08-04
- **Target:** **§1.1** — the series control's power, and the disposition of `/xask` finding **X11**
  (`n = 2` per arm).
- **ORIGINAL TEXT (verbatim, §1.1):**

  > On SPEC-355's observed sd (1,479 B) that is **≈ 6,400 B ≈ 17 % of the level**. **A smaller perturbation is
  > NOT excluded** — the control is honestly weak, and this sentence is part of the pre-registration precisely
  > so a non-rejection is not read as a proof of zero effect.

- **FINDING:** the frozen text states the weakness but **imposes no obligation on how it is reported**, and
  X11 — the one `/xask` finding disposed as *"escalated to the user"* — had **no landing site**: not an
  addendum here, not a rule in SPEC-356b, not a tracked item. An escalation with nowhere to land is
  indistinguishable, six weeks later, from a finding that was quietly dropped, and *"escalated"* is a third
  disposition category the round's own preamble does not admit (it recognises **applied** and
  **refuted-with-reason** only). Raised by the SPEC-356a **Review v1, finding M3**, 2026-08-04.
- **ADJUDICATED FORM (governs):**

  > **The role is stated, not implied: at `n = 2` per arm the series control is a CATASTROPHE DETECTOR.** It
  > is powered to reject a gross instrument perturbation (`≈ 17 %` of the level on SPEC-355's observed sd) and
  > nothing finer. A non-rejection is **evidence that no catastrophe occurred**, and is **not** evidence of
  > neutrality.
  >
  > **`n = 6` is DECLINED, with its cost on the record.** Each cell is a 4 h run, so `n = 6` per arm is
  > **≈ 48 h of additional control time** — against a family whose next gate (TODO-484) is itself a 72 h soak.
  > The spend is refused because the control's *job* here is to catch a catastrophe, and `n = 2` discharges
  > that job; buying `MDE ≈ 5.0 %` would not convert the control into a neutrality proof either, only into a
  > better-powered non-proof. The vendor's table (`n = 4 ⇒ ≈ 6.8 %`, `n = 6 ⇒ ≈ 5.0 %`, `n = 8 ⇒ ≈ 4.2 %`)
  > stands in the record so the decision keeps its numbers.
  >
  > **REPORTING BOUND (this is the obligation the escalation lacked).** Every neutrality statement SPEC-356b
  > writes — in its results, its cell-E disposition, or any summary of them — MUST cite, verbatim, both
  > **`MDE ≈ 17 %`** (recomputed from the pair's own observed pooled sd, per §1.1's formula) and **"a smaller
  > perturbation is NOT excluded"**. A neutrality claim published without both is **out of contract**. The
  > companion obligations of **ADJ-5** (bounded-and-flagged, and the dynamics companions) apply unchanged and
  > are not weakened by this.
  >
  > **X11's disposition is hereby APPLIED, landing as ADJ-8.** The *"escalated to the user"* category is
  > withdrawn; the round's two-category preamble holds, and every finding is applied or
  > refuted-with-reason.

- **AUTHORITY:** SPEC-356a Review v1 finding M3; `/xask` round finding X11; user ruling 2026-08-04.
- **PRE-DATA / POST-DATA:** **PRE-DATA** — same boundary as ADJ-7: no measurement artifact exists.

### ADJ-9

- **ADJ id:** ADJ-9
- **Date:** 2026-08-08
- **Target:** **§4** (cell E's protocol) versus the runner's cell table (`spec356-prune.sh`, `cellE` arm).
- **ORIGINAL TEXT (verbatim, §4):**

  > **Cell E carries NO prune-record columns**: the pre-346 server has no recorder, and expecting them is a
  > category error.

- **FINDING:** the runner's cell table set `cellE) ARMED=yes`, and the arming witness branches ONLY on
  `ARMED`: an armed cell whose scrape carries zero `topgun_or_prune_` series is `fail_instrument` → exit 9.
  A pre-346 server structurally emits no such series, so cell E was **guaranteed** to exit 9 — the runner
  embodied exactly the category error §4 forbids. A second, independent exit 9 followed: with `ARMED=yes`
  the column checks do not take the disarmed early return, so the six INSTRUMENT columns fail their NONZERO
  limb. R4.3's *"discarded and re-run"* disposition would loop forever. Raised by SPEC-356b **Audit v3,
  critical C1**.
- **ADJUDICATED FORM (governs):**

  > **Cell E is a DISARMED-expectation cell.** The runner's cell-table entry is repaired (`ARMED=yes` →
  > `ARMED=no`) in the same commit as this addendum, PRE-DATA. The arming witness's disarmed branch — a
  > scrape carrying **zero** `topgun_or_prune_` series — is the §4-consistent check for this cell, and the
  > column completeness gate takes the ADJ-6 disarmed early return. `PROVENANCE=yes` is unchanged: cell E
  > remains the §4.2 half-swap identity-checked cell. **No expected-exit-9 carve-out is created**: exit 9 on
  > cell E (as on any cell) remains a real instrument-defect signal, and the fail-closed STEP 0 taxonomy is
  > intact. The repair-the-instrument disposition was chosen over pre-registering an exit-9 interpretation
  > precisely because teaching the protocol that "exit 9 sometimes means success" would corrode the defect
  > signal for every later consumer.

- **AUTHORITY:** SPEC-356b Audit v3 critical C1; Conductor ruling 2026-08-08.
- **PRE-DATA / POST-DATA:** **PRE-DATA** — committed before any `spec356-*.soak.json` exists.

### ADJ-10

- **ADJ id:** ADJ-10
- **Date:** 2026-08-08
- **Target:** **ADJ-8's adjudicated form** — the costing of the declined `n = 6` series-control extension.
- **ORIGINAL TEXT (verbatim, ADJ-8):**

  > **`n = 6` is DECLINED, with its cost on the record.** Each cell is a 4 h run, so `n = 6` per arm is
  > **≈ 48 h of additional control time** — against a family whose next gate (TODO-484) is itself a 72 h soak.

- **FINDING:** the arithmetic was computed from the 4 h `long` cell, but the series-control arms are the
  1800 s cells (`ctl` / `ctloff`, runner cell table; R4.4). The true incremental cost of `n = 2 → 6` is
  **8 × 1800 s = 4 h** (all twelve control runs from zero: 6 h) — twelve times smaller than the figure the
  decline cited. Raised by SPEC-356b **Audit v3, critical C2**.
- **ADJUDICATED FORM (governs):**

  > The corrected figures govern the record: `n = 2 → 6` costs **≈ 4 h**, not ≈ 48 h. With the cost leg
  > corrected, **the DECLINE of `n = 6` is RE-AFFIRMED on the role leg alone**: ADJ-8's adjudication that the
  > series control is a CATASTROPHE DETECTOR stands, `n = 6` would buy a better-powered **non-proof** of
  > neutrality rather than a neutrality proof, and no downstream consumer of SPEC-356b conditions on an MDE
  > finer than the reporting bound. The **REPORTING BOUND is unchanged**: every neutrality statement still
  > cites `MDE ≈ 17 %` (recomputed from the pair's own pooled sd) and *"a smaller perturbation is NOT
  > excluded"*, verbatim. Any future re-litigation of the extension MUST argue from the corrected 4 h figure.

- **AUTHORITY:** SPEC-356b Audit v3 critical C2; Conductor ruling 2026-08-08.
- **PRE-DATA / POST-DATA:** **PRE-DATA** — committed before any `spec356-*.soak.json` exists.

### ADJ-11

- **ADJ id:** ADJ-11
- **Date:** 2026-08-08
- **Target:** **ADJ-7's `max(L)` reporting obligation** and its interaction with **§2.2 Step 0** routing.
- **ORIGINAL TEXT (verbatim, ADJ-7):**

  > A Step-2 determination must be reported together with `max(L)` over the window, not with the median
  > alone. A window whose `max(L)` is also 0 is a window in which the sampler never observed a non-empty
  > drain, and that is an admissibility observation (§2.2 limb (b)'s territory), not evidence that the prune
  > is licensed-and-draining.

- **FINDING:** the clause leaves a two-sided hazard unadjudicated. (a) If `max(L) = 0` auto-routes to
  INDETERMINATE via limb (b)'s fail-closed rule, a genuinely eligibility-starved system — one whose every
  pass observes zero licensed backlog — becomes structurally unclassifiable: the instrument would
  predetermine against one of the two verdicts it exists to distinguish. (b) `eligible_refs` is a
  pass-retained GAUGE scraped on a 10 s cadence while prune passes may run far more often, so a scrape-level
  `max(L) = 0` cannot by itself distinguish genuine starvation from under-sampling. Raised by SPEC-356b
  Audit v3 recommendation 7; both clauses of the first draft resolution were then broken by an adversarial
  cross-vendor round (glm-5.2, 2026-08-08, artifact `spec356-adj11-xask.md`): flat drains prove only that
  nothing DRAINED, not that nothing was ELIGIBLE, and letting counters "carry the determination" in the
  under-sampled regime is the instrument's own limitation silently picking a verdict.
- **ADJUDICATED FORM (governs — counter-anchored, the gauge alone decides nothing):**

  > Window disposition under `max(L) = 0` is decided by the MONOTONE COUNTERS, whose per-pass identity
  > `passes_total == empty_drains_total + nonempty_drains_total` is pinned in code and test (TG-OR-006's
  > exhaustiveness family). Over the window, with Δ denoting the counter delta:
  >
  > 0. **Conservation first:** if `Δpasses ≠ Δempty_drains + Δnonempty_drains`, the window is
  >    **INDETERMINATE** (unaccounted passes).
  > 1. **Step 0(b) unchanged:** a window failing the split-recompute certification is **INDETERMINATE**.
  > 2. **Eligibility-starved evidence requires per-pass counter proof:** `Δnonempty_drains = 0` AND
  >    `Δempty_drains = Δpasses > 0` AND the error counters are flat (`restored_read_error_total`,
  >    `restored_write_error_total` — a blocked drain can report an empty drain while licensed work exists)
  >    AND the tombstone backlog series grows ⇒ **every pass observed zero licensed backlog**: valid
  >    evidence toward ELIGIBILITY-BOUND, with `max(L) = 0` as corroboration only. ADJ-7's prohibition
  >    stands — this is never read as "the prune is licensed-and-draining". A window with advancing error
  >    counters is **INDETERMINATE**.
  > 3. **Under-sampled regime:** `Δnonempty_drains > 0` while scrape-level `max(L) = 0` ⇒ the `max(L)` cell
  >    is **INADMISSIBLE** for that window (a non-sample: it bounds nothing), and the window's primary
  >    classification is **INDETERMINATE**. The counters are reported as SECONDARY evidence (refs drained,
  >    bytes freed); a backlog stable-or-shrinking while drains advance is an affirmative *keeping-up*
  >    secondary finding. The counters can EXCLUDE pure eligibility-starvation here; they cannot CONFIRM
  >    throughput-boundedness, and no clause may treat that one-sided exclusion as a completed
  >    determination.
  >
  > **Diagnostic obligation:** every window report states `Δpasses` alongside its scrape count, so the
  > under-sampling regime is visible on the record rather than inferred.

- **AUTHORITY:** SPEC-356b Audit v3 recommendation 7; adversarial `/xask` round 2026-08-08 (findings
  adopted: the transient-eligibility false-positive closed by the empty-drains counter anchor; the
  broken-gauge hazard subsumed because clause 2 no longer rests on the gauge; INADMISSIBLE-plus-
  INDETERMINATE adopted verbatim for the under-sampled regime); Conductor ruling 2026-08-08.
- **PRE-DATA / POST-DATA:** **PRE-DATA** — committed before any `spec356-*.soak.json` exists.

### ADJ-12

- **ADJ id:** ADJ-12
- **Date:** 2026-08-08
- **Target:** **§2.1's definition of `B`** and **§5.4's demotion of rendered quantiles**, jointly — the
  committed source of the Step 2 / Steps 3–4 partition's right-hand side.
- **ORIGINAL TEXT (verbatim, §2.1 then §5.4):**

  > **`B`** — the **median refs per non-empty drain** over the last half. Self-calibrating: it is one prune
  > batch.

  > That is why the **per-interval means differenced from the monotone totals are the PRIMARY reading**, and
  > any exporter-rendered quantile is **corroboration only**.

- **FINDING:** `B` is defined as a median, but no committed column could produce one:
  `topgun_or_prune_drain_refs` is a histogram and the CSV sampled only `_sum` / `_count`, from which a
  median is not derivable. The estimator would have been chosen at G4 **with the data visible**, on the
  partition that decides the modal routing. Raised by SPEC-356b **Audit v4, critical C1**. The adversarial
  cross-vendor round (artifact `spec356-adj12-xask.md`) then rejected both repair candidates the audit
  left open: the **mean** (`Δsum/Δcount`) is unboundedly inflated by a heavy right tail — one catch-up
  megabatch can collapse the partition to Step 2 (the modal outcome) regardless of the true `L`, a
  confirmation-shaped failure; a **guard band** on the mean destroys the exact-complement partition and
  introduces an arbitrary constant.
- **ADJUDICATED FORM (governs):**

  > **`B` := the median over the last-half `prune.csv` rows of the committed column
  > `topgun_or_prune_drain_refs_p50`** — the exporter-rendered p50 of the batch-size summary, added to the
  > selection table as column 43 (MEASURAND) in the same commit as this addendum. Bindings:
  >
  > - **Window length is COMMITTED, not assumed:** the rolling window is `3 × 20 s = 60 s` — §5.4's own
  >   figure, re-verified against the locked crate source (`metrics-exporter-prometheus 0.16.2`,
  >   `src/distribution.rs`: `DEFAULT_SUMMARY_BUCKET_COUNT = 3`, `DEFAULT_SUMMARY_BUCKET_DURATION = 20 s`)
  >   and the exporter is constructed with no override (`PrometheusBuilder::new()`).
  > - **The 0-sentinel rule (verified against the live exposition, not assumed):** the summary records only
  >   NON-EMPTY drains, so a populated window renders `p50 ≥ 1`, and an empty or expired window renders
  >   **`0`, not `NaN`**. Sentinel rows are **EXCLUDED** from `B`'s median and the excluded fraction is
  >   **REPORTED** per window.
  > - **Escape hatch, not a third branch:** if more than **50 %** of the last-half rows are sentinel, `B`
  >   is unreliable for that window and the window routes via **Step 0(c) (deciding column unreadable) to
  >   INDETERMINATE**. The Step 2 / Steps 3–4 exact-complement partition is untouched.
  > - **§5.4 STANDS in full:** no `n`, no sd, no standard error and no t-statistic may be computed by
  >   counting rows of the p50 column; `B`'s median is a **location estimate** over an autocorrelated
  >   series (effective independent observations ≈ half-window / 60 s), and the `_sum` / `_count`
  >   differenced means remain the PRIMARY reading for every THROUGHPUT quantity. The "corroboration only"
  >   demotion is adjudicated as governing **counting-based inference**, which this use is not: the p50
  >   column is the one committed order statistic, and `B` is an order-statistic parameter.
  > - **Known-and-accepted caveat on the record:** a median over per-scrape window-medians weights each
  >   window equally regardless of how many drains it contains; if batch sizes covary with drain frequency
  >   across the half-window, `B` carries a bounded weighting distortion of unknown direction. This is
  >   irreducible without `.rs` changes and is accepted as the bounded-unknown over the mean's
  >   unbounded-known.

- **AUTHORITY:** SPEC-356b Audit v4 critical C1; adversarial `/xask` round 2026-08-08
  (`spec356-adj12-xask.md`: mean rejected for unbounded partition collapse, p50 adopted conditional on a
  committed window length and a pre-registered sentinel threshold with a protocol exit); live exposition
  probe 2026-08-08 (0-sentinel semantics); Conductor ruling 2026-08-08.
- **PRE-DATA / POST-DATA:** **PRE-DATA** — committed before any `spec356-*.soak.json` exists.

### ADJ-13

- **ADJ id:** ADJ-13
- **Date:** 2026-08-08
- **Target:** **§2.3 Steps 3–4** — the passes-per-epoch slope test's series construction (mandated
  unconditional by ADJ-3, but never constructed).
- **ORIGINAL TEXT (verbatim, §2.3 Step 3):**

  > (exit share ≤ 10 %) **and** **PERSISTENT** licensed backlog (`min(L) > B`) **and** passes-per-epoch
  > last-half OLS slope rejects **negative** (α = 0.05)

- **FINDING:** neither document says how the passes-per-epoch series is formed from `passes_total` /
  `current_epoch`. Raised by SPEC-356b **Audit v4, critical C2**. The natural per-epoch-delta construction
  (group rows by `current_epoch`, exclude intervals spanning unobserved epoch boundaries) was then broken
  by the adversarial round: the excluded epochs are **mechanically the fast-advancing (busy) ones**, so
  exclusion biases the slope toward flat/negative — toward the SCHEDULING verdict — and the exclusion
  fraction is set by the scrape-cadence-to-epoch-width ratio, an operational parameter with no relation to
  the defect being diagnosed. An instrument limitation would have picked the verdict.
- **ADJUDICATED FORM (governs — the cumulative construction, zero exclusion):**

  > The pass-rate evidence is read from the **cumulative curve**: `passes_total` (y) against
  > `current_epoch` (x) over **ALL** last-half `prune.csv` rows — no grouping, no exclusion. Rows with
  > `Δcurrent_epoch = 0` add intra-epoch weight; rows jumping several epochs are valid coarse steps; no
  > information is discarded. The Step 3 / Step 4 discriminator is a **declining-rate test via split-half
  > slopes of that curve, both fit with the SAME pinned fitter §6 names for `tombstone_bytes`**: let
  > `s_early` be the fitted slope over the first half of the last half and `s_late` over the second half.
  > **Step 3 (SCHEDULING / LICENSING) requires `s_late` significantly BELOW `s_early`** (one-sided,
  > α = 0.05 — the original test's level and direction, recast onto the unbiased construction); otherwise
  > the branch reads **Step 4 (THROUGHPUT)**. Reduced statistical power relative to a per-epoch series is
  > **accepted on the record** in exchange for zero selection bias: under this protocol's ranking, an
  > instrument limitation that silently picks a verdict is strictly worse than one that honestly widens a
  > confidence interval.

- **AUTHORITY:** SPEC-356b Audit v4 critical C2; adversarial `/xask` round 2026-08-08
  (`spec356-adj12-xask.md`: exclusion construction rejected for mechanical selection of busy epochs;
  cumulative construction adopted); Conductor ruling 2026-08-08.
- **PRE-DATA / POST-DATA:** **PRE-DATA** — committed before any `spec356-*.soak.json` exists.

### ADJ-14

- **ADJ id:** ADJ-14
- **Date:** 2026-08-08
- **Target:** **ADJ-11's adjudicated form, clause 2** — the unnamed "tombstone backlog series".
- **ORIGINAL TEXT (verbatim, ADJ-11 clause 2):**

  > AND the tombstone backlog series grows ⇒ **every pass observed zero licensed backlog**: valid
  > evidence toward ELIGIBILITY-BOUND, with `max(L) = 0` as corroboration only.

- **FINDING:** "the tombstone backlog series" names no committed series, and the candidates disagree: the
  soak CSV's gauge is on a 60 s cadence while the prune CSV runs at 10 s, and `indexed_refs` is sampled
  POST-drain (the ADJ-7 asymmetry family). A naïve pick of `topgun_ormap_tombstone_bytes_total` alone
  would also be wrong in the opposite direction: it is a monotone ADDED-bytes counter and grows even under
  healthy reclaim. Raised by SPEC-356b **Audit v4, recommendation 6**.
- **ADJUDICATED FORM (governs):**

  > Backlog growth in ADJ-11 clause 2 is the **difference of two monotone counters from the same
  > `prune.csv` rows**: `Δbacklog_bytes := Δ topgun_ormap_tombstone_bytes_total −
  > Δ topgun_or_prune_bytes_freed_total`, and "the backlog grows" ⟺ `Δbacklog_bytes > 0` over the window.
  > Counter-anchored, same 10 s cadence as every other quantity the clause reads, and immune to the
  > added-bytes-alone confusion: in clause 2's own regime (`Δnonempty_drains = 0`) the freed term is 0 and
  > the test degenerates to "garbage kept arriving while nothing drained", which is exactly the starvation
  > semantics the clause needs. Neither `indexed_refs` nor the 60 s soak-CSV gauge may stand in for it.

- **AUTHORITY:** SPEC-356b Audit v4 recommendation 6; Conductor ruling 2026-08-08.
- **PRE-DATA / POST-DATA:** **PRE-DATA** — committed before any `spec356-*.soak.json` exists.

### ADJ-15

- **ADJ id:** ADJ-15
- **Date:** 2026-08-08
- **Target:** **ADJ-13's adjudicated form** — the pass-rate fit's axis and its Step 3 / Step 4
  discriminator — against the pinned fitter's own contract (`spec349c2-fit.awk`).
- **ORIGINAL TEXT (verbatim, ADJ-13):**

  > both fit with the SAME pinned fitter §6 names for `tombstone_bytes` […] **Step 3 (SCHEDULING /
  > LICENSING) requires `s_late` significantly BELOW `s_early`** (one-sided, α = 0.05 — the original
  > test's level and direction, recast onto the unbiased construction)

- **FINDING:** two defects, both raised by SPEC-356b **Audit v5 (C1, C2)**. (1) The pinned fitter
  hardwired `x = elapsed_secs`: the two-column pass-rate CSV was unfittable (exit 2), and the silent
  workaround — regressing against TIME — yields the OPPOSITE verdict on accelerating-epoch data (probe:
  time-axis `s_early = s_late` → Step 4 while epoch-axis `21600 / 7200` → Step 3). (2) The fitter's own
  doc-contract states that for cumulative autocorrelated series its SE is OPTIMISTIC and *"SE separation
  alone must never carry a discrimination claim; the minimum effect-size floor does"* — yet ADJ-13 made
  SE-separation-alone the sole discriminator with no floor, biasing toward SCHEDULING, the very direction
  it rejected the per-epoch construction for. The third adversarial cross-vendor round
  (`spec356-adj15-xask.md`) then rejected my two-leg repair: an optimistic-SE precondition is **theater
  with a back door** — the only case where it binds is the one where it overrides the material floor
  using the instrument its own contract discredits.
- **ADJUDICATED FORM (governs):**

  > 1. **Axis.** `spec349c2-fit.awk` gains an additive `-v xaxis=<header>` parameter. The default
  >    (`elapsed_secs`, hours conversion, historical field names) is **byte-identical** to the
  >    pre-parameter script — regression-proven over EVERY committed evidence CSV × both windows × all
  >    five columns (190 output lines, `cmp` clean; recorded in `spec356-adj15-xask.md`). The pass-rate
  >    fit runs `-v col=passes_total -v xaxis=current_epoch` with raw x (no hours conversion) and
  >    honestly-renamed output fields (`slope_per_x_unit`), on TWO derived slice CSVs: `early` = first
  >    half of the last-half rows, `late` = second half, each fit with `window=full`.
  > 2. **Discriminator — floor-only; the SE leg is REMOVED.**
  >    **Step 3 (SCHEDULING) ⟺ `s_early ≥ 5` passes/epoch AND `s_late ≤ 0.5 × s_early`.** Otherwise
  >    Step 4 — EXCEPT the degeneracy case `s_early < 5` passes/epoch, which routes via **Step 0(c) to
  >    INDETERMINATE**: at noise-floor pass rates the instrument cannot distinguish scheduling decline
  >    from noise, and defaulting to either branch would be the instrument picking. The `5` is a coarse
  >    fixed margin in this protocol's tradition (not a derived formula): the measured healthy rate on
  >    the armed smoke probe is ≈ 950 passes/epoch, so the guard sits two orders of magnitude below
  >    nominal and can only fire on genuine degeneracy.
  > 3. **Sensitivity — CONTESTED, with an iron no-routing rule.** The verdict at `0.5` GOVERNS. It is
  >    also computed at `0.4` and `0.6`; a flip within that band attaches ADJ-3's **CONTESTED** label.
  >    **CONTESTED changes exactly zero routing decisions** — a consumer that re-runs, escalates,
  >    re-weights or "holds for review" on it is out of contract. The ±20 % band is the same coarseness
  >    class as the floor itself and is pre-registered as coarse.
  > 4. **Warmup.** The last-half window is the pre-registered warmup exclusion (the early slice is hours
  >    2–3 of the 4 h cell). Both raw slopes are REPORTED in §9 so residual transients are visible. Two
  >    accepted limitations stand on the record: a slow compounding decline (≈ 0.8× per half) is below
  >    this floor's detection threshold, and a warmup transient surviving into the early slice would
  >    bias toward Step 3 — the reported raw slopes are the reader's check on both.
  > 5. **No reading was ever taken under the defective text:** no pass-rate fit was executed before this
  >    addendum (PRE-DATA holds), so the correction repairs pre-registration text, not results.

- **AUTHORITY:** SPEC-356b Audit v5 criticals C1 and C2; adversarial `/xask` round 2026-08-08
  (`spec356-adj15-xask.md`: SE leg dropped as theater-with-a-back-door; absolute degeneracy guard added
  with the INDETERMINATE routing chosen over the vendor's default-to-Step-4 to avoid a lean; CONTESTED
  no-routing rule adopted verbatim); regression-proof over all committed CSVs; Conductor ruling
  2026-08-08.
- **PRE-DATA / POST-DATA:** **PRE-DATA** — committed before any `spec356-*.soak.json` exists.

### ADJ-16

- **ADJ id:** ADJ-16
- **Date:** 2026-08-08
- **Target:** **ADJ-5's third obligation** (the armed-vs-disarmed dynamics companions) against
  **ADJ-6 / ADJ-9's disarmed-absence rule**; and, jointly, **ADJ-15's degeneracy routing scope**.
- **ORIGINAL TEXT (verbatim, ADJ-5, the third paragraph of the adjudicated form):**

  > **The armed-vs-disarmed comparison is ALSO reported for the three dynamics statistics the predicate
  > reads** — non-drop exit share, `median(L)/B`, and the passes-per-epoch slope — as **descriptive
  > companions**.

- **FINDING:** structurally unsatisfiable, and the conflict is between two frozen PRE-DATA addenda. All
  three dynamics statistics read `topgun_or_prune_*` columns; the disarmed arm emits none of them —
  `NullPruneRecorder` registers no series, the disarmed arming witness **REQUIRES** their absence, and
  ADJ-6 rules those columns empty **by design and never a finding**. ADJ-5's own finding (ii) — the
  dynamics blind spot — was left unmitigated while AC limb B3(e) still promised the mitigation, so an
  executor would either publish armed-only numbers as a "comparison" or drop the limb silently. Raised by
  SPEC-356b **Audit v6, critical C1** (recommendation 7 raised the routing-scope half).
- **ADJUDICATED FORM (governs):**

  > 1. **The armed-vs-disarmed dynamics COMPARISON is WITHDRAWN** as structurally impossible with the
  >    committed instrument. **X6's dynamics blind spot is thereby UNMITIGATED and OPEN**, and the record
  >    must say so rather than imply coverage: **wherever §9 cites the level controls, it must state
  >    verbatim — "a dynamics-only perturbation from recorder presence or activation is not excluded by
  >    any control in this protocol"** — routed to **TODO-638** (post-356b instrument work; a dynamics
  >    control needs a disarmed-visible statistic source, which is `.rs` work under a later spec).
  > 2. **The ARMED arm's three dynamics statistics are still computed and reported** — as SINGLE-ARM
  >    observations, explicitly labeled *"NOT a comparison; no control arm exists for these"*. Dropping
  >    them entirely would discard real information to mourn a comparison that never existed.
  > 3. **ADJ-15's degeneracy routing is SCOPED** (this resolves the over-foreclosure): `s_early < 5`
  >    fires only at the Step 3 / Step 4 evaluation, which a window reaches only AFTER Step 1
  >    (exit share ≤ 10 %) and Step 2's complement (persistent backlog) are already facts of record.
  >    Those readings STAND. The window's outcome is reported as **"Steps 3–4, NOT SEPARABLE (degenerate
  >    pass rate)"** — a COARSENED leaf, not a new branch: every window still reaches exactly one
  >    reported outcome, and at §2.5's routing level the coarse outcome carries the consequence Steps 3
  >    and 4 SHARE (the accelerator family), leaving the registry-vs-accelerator boundary — the modal
  >    question this spec exists to answer — undamaged.

- **AUTHORITY:** SPEC-356b Audit v6 critical C1 and recommendation 7; Conductor ruling 2026-08-08.
- **PRE-DATA / POST-DATA:** **PRE-DATA** — committed before any `spec356-*.soak.json` exists.

### ADJ-17

- **ADJ id:** ADJ-17
- **Date:** 2026-08-09
- **Target:** **ADJ-15's slice construction** — the pass-rate chain's window and split definitions.
- **ORIGINAL TEXT (verbatim, ADJ-15, the slice clause):**

  > on TWO derived slice CSVs: `early` = first half of the last-half rows, `late` = second half, each fit
  > with `window=full`

- **FINDING:** a ROW-INDEX split makes the ledger's row count a free parameter of the verdict. SPEC-356b
  Audit v12 (critical C1) exhibited the minimal pair: the honest 1440-row ledger reads
  `s_early 600 / s_late 200` (Step 3), while a 1-in-4 subsample of the same cell — preserving span,
  monotonicity and the cadence cap — moves the row-index boundaries in time and reads `200 / 200`
  (Step 4), both green. The v11 repair (deriving slices from the parent) removed the SLICE FILES as an
  input but left the BOUNDARY a function of `n`; the freedom moved rather than disappeared.
- **ADJUDICATED FORM (governs — coordinates, not counts):**

  > Every window and split in the pass-rate chain is defined on the **x-axis**, never on row indices:
  >
  > - **Last-half window:** the rows of the cell's ledger whose `elapsed_secs` exceeds
  >   `(t_first + t_last) / 2` of the FULL ledger.
  > - **Early / late split:** within that window, `early` = rows with
  >   `current_epoch ≤ (e_lo + e_hi) / 2`, `late` = the rest — where `e_lo` / `e_hi` are the window's
  >   first and last `current_epoch` values. The midpoint of the SPAN, not the median row.
  >
  > A point's half-membership now depends only on its own coordinates, so dropping or duplicating rows
  > cannot move any boundary: the subsampling family of attacks loses its lever **by construction**
  > rather than by bound. **Density is guarded separately, both directions:** the provenance limb must
  > require the ledger's row count within **[0.5×, 1.5×] of `span / cadence`** — thinning AND padding are
  > detectable, and the bound is a coarse fixed band in this protocol's tradition. The fitter's own
  > historical row-based `last_half` window is UNTOUCHED for the byte-slope fits it is regression-locked
  > to; the pass-rate driver derives its windows itself from the coordinates above and fits each derived
  > window with `window=full`.

- **AUTHORITY:** SPEC-356b Audit v12 critical C1; Conductor ruling 2026-08-09 (remove-the-freedom over
  bound-the-parameter, the same ruling shape as ADJ-9/ADJ-16/ADJ-17's precedents).
- **PRE-DATA / POST-DATA:** **PRE-DATA** — committed before any `spec356-*.soak.json` exists.

### ADJ-18

- **ADJ id:** ADJ-18
- **Date:** 2026-08-09
- **Target:** **ADJ-17's adjudicated form** — the split axis and the density band.
- **ORIGINAL TEXT (verbatim, ADJ-17):**

  > **Early / late split:** within that window, `early` = rows with `current_epoch ≤ (e_lo + e_hi) / 2`,
  > `late` = the rest […] the provenance limb must require the ledger's row count within **[0.5×, 1.5×] of
  > `span / cadence`**

- **FINDING:** two residuals, surfaced by Response v12's own STOP (the fixes need a manifest byte, which
  the response correctly did not take). (1) The split midpoint reads `e_hi` from the ledger's LAST row, so
  a **monotone forward stretch of one epoch value** — passing monotonicity and the cadence cap — drags the
  boundary and starves the late slice (measured: honest `600/200` → `564.7/195.0` → `436.8/32.7`). The
  epoch axis is the one axis NO provenance limb binds. (2) The `[0.5×, 1.5×]` band is ledger-global, which
  leaves a **50 % deletion budget inside a single half**: coordinate membership stops boundary movement,
  but value-selective thinning within one half still tilts that half's fit.
- **ADJUDICATED FORM (governs):**

  > 1. **The split boundary moves to the provenance-guarded axis:** `early` = rows of the last-half window
  >    with `elapsed_secs ≤ (t_lo + t_hi) / 2` of THAT WINDOW, `late` = the rest. `elapsed_secs` is
  >    already bound by the provenance limb (cadence, span, `matrix.txt` identity), and the temporal
  >    halving is ADJ-13's own semantics — "run less and less often as the run progresses". The FIT axis
  >    is unchanged (`current_epoch`, per ADJ-15/ADJ-17); only slice MEMBERSHIP moves to elapsed.
  > 2. **The epoch axis gains a coarse integrity bound of its own:** within the parent window,
  >    `current_epoch` must be non-decreasing and every single-row jump must be
  >    **≤ 10 × the median positive jump** — a fixed coarse multiple in this protocol's tradition, wide
  >    enough for any legitimate scheduling burst and narrow enough that a boundary-moving stretch of one
  >    point is RED on its own row.
  > 3. **The density band localizes and tightens:** row count within **[0.8×, 1.2×] of `span / cadence`**,
  >    enforced for the ledger AND for EACH derived half separately. The observed legitimate skip rate is
  >    ~1 % (one gated tick per smoke); 20 % headroom is coarse-generous, and a 50 % in-half deletion is
  >    RED by the half's own band.
  > 4. **The graders' bound, restated honestly (the R5.7(f) shape):** these limbs catch honest mistakes,
  >    drift, and cheap launderings. A fully coherent forged ledger — every column rewritten consistently
  >    with its own cadence, spans, identities and digests — is out of scope for content checks by
  >    construction; the defense at that layer is the provenance of the run itself, not arithmetic over
  >    the artifact.

- **AUTHORITY:** SPEC-356b Response v12 STOP items (both residuals measured there); Conductor ruling
  2026-08-09.
- **PRE-DATA / POST-DATA:** **PRE-DATA** — committed before any `spec356-*.soak.json` exists.

### ADJ-19

- **ADJ id:** ADJ-19
- **Date:** 2026-08-09
- **Target:** **ADJ-18's clauses 2 and 3** — the jump bound's small-sample hole and the band's
  concentration hole.
- **ORIGINAL TEXT (verbatim, ADJ-18):**

  > every single-row jump must be **≤ 10 × the median positive jump** […] row count within
  > **[0.8×, 1.2×] of `span / cadence`**, enforced for the ledger AND for EACH derived half separately

- **FINDING:** two measured constructions, surfaced by Response v13's STOP. (1) Below three positive
  jumps the median is dominated by the attack itself: jump set `{1, 1398}` yields median `699.5`, bound
  `6995` — the stretched row passes its own guard. (2) Any density band leaves a budget, and the budget
  can be CONCENTRATED: deleting a contiguous run of rows adjacent to the split boundary stays inside the
  per-half band while tilting that half's fit — the band bounds volume, not distribution.
- **ADJUDICATED FORM (governs):**

  > 1. **Minimum evidence for the jump bound:** the `10 × median positive jump` rule applies only when
  >    the parent window carries **≥ 5 positive jumps**. Below that the epoch axis cannot support a
  >    pass-rate fit at all, and the window routes to **INDETERMINATE via the existing Step 0(c) hatch**
  >    (the ADJ-12/ADJ-15 route — no new mechanism, no default to either branch). A guard whose reference
  >    statistic the attacker's own row can dominate is not a guard.
  > 2. **Distribution is bounded, not just volume:** within EACH derived half, the maximum gap between
  >    consecutive `elapsed_secs` values must be **≤ 5 × cadence**. A legitimate skipped scrape gives
  >    2×; two consecutive skips 3×; the measured skip rate is ~1 %. A concentrated deletion — the
  >    100-row edge run — is a ~100× gap and REDs on its own bytes. Uniform sparse thinning inside the
  >    band is harmless BY CONSTRUCTION under coordinate membership: it moves no boundary and biases no
  >    fit direction. Both constants are coarse fixed margins in this protocol's tradition.

- **AUTHORITY:** SPEC-356b Response v13 STOP items (both constructions measured there); Conductor ruling
  2026-08-09.
- **PRE-DATA / POST-DATA:** **PRE-DATA** — committed before any `spec356-*.soak.json` exists.

### ADJ-20

- **ADJ id:** ADJ-20
- **Date:** 2026-08-10
- **Target:** the **class** behind Response v15's four STOP residuals (TODO-639 median-planting,
  TODO-640 span-band tail deletion, TODO-641 `passes_total` y-axis, TODO-642 control
  `tombstone_bytes`) — against **ADJ-18 clause 4's** stated boundary.
- **ORIGINAL TEXT (verbatim, ADJ-18 clause 4):**

  > A fully coherent forged ledger — every column rewritten consistently with its own cadence, spans,
  > identities and digests — is out of scope for content checks by construction; the defense at that
  > layer is the provenance of the run itself, not arithmetic over the artifact.

- **FINDING:** all four residuals are instances of ONE class — an edit to one column or region of a
  committed artifact that the per-column guards do not bind. The ledger carries 43 columns and the
  protocol was guarding the third; a per-column arms race does not converge. Clause 4 already names the
  correct defense layer — the provenance of the run — but no mechanism implemented it: nothing bound an
  artifact's bytes to the run that produced them.
- **ADJUDICATED FORM (governs):**

  > 1. **The run binds its own artifacts at the moment of production.** The runner's terminal act writes
  >    `${BASE}.artifacts.sha256` — the SHA-256 of every artifact it produced, by BASENAME (so the file
  >    verifies unchanged after the scratch-to-evidence copy). The digest file cannot contain itself; it
  >    is bound by being runner-written and committed **in the same commit** as the artifacts it names.
  > 2. **Graders verify before they read:** every committed artifact a grader or driver consumes is
  >    checked against the run's digest file BEFORE any content check; a mismatch is a **PROVENANCE
  >    failure** (RED), distinct from every content disposition.
  > 3. **TODO-639/640/641/642 are closed AS A CLASS:** a single-column or single-region edit now requires
  >    forging a coherent digest set, which is exactly the coherent-forgery case clause 4 places outside
  >    the content-check layer. The per-column guards (jump bound, density band, gap bound, span, epoch
  >    monotonicity) are NOT weakened — they retain their real role: catching honest mistakes and
  >    instrument faults in artifacts whose provenance is intact.
  > 4. **The embedded programs become committed evidence files** (`spec356-tmplconf.awk`,
  >    `spec356-slottruth.sh`, `spec356-skeleton.txt`), extracted from the spec's bytes by its own marker
  >    commands and digest-verified against its own pins at extraction (`280f7a34…` / `7c14b900…` /
  >    `bfaac337…` — all three matched). The committed file is now the canonical copy; the spec
  >    references path + digest, and the digests are unchanged by the move.

- **AUTHORITY:** SPEC-356b Response v15 STOP residuals TODO-639..642 (each measured there); Conductor
  ruling 2026-08-10.
- **PRE-DATA / POST-DATA:** **PRE-DATA** — committed before any `spec356-*.soak.json` exists.

---

## §9 — The executed record (SPEC-356b)

**This section is SPEC-356b's own. It is created here by B1/G1 carrying the RESOLVED PIN and nothing
else; the executed cells, the controls block, the classification walk and cell E's disposition are
written by the later waves.** No byte of §0–§8 or §8A was touched to record it (§0(a): the frozen
sections carry no SHA and no SHA slot).

### §9.1 — Lineage: the resolved pin

| | Value |
|---|---|
| **Resolved instrument-lineage tip** (`git rev-parse main`, B1's first action) | **`feb85268952001813e502e27f65180855676ac25`** |
| Short form used in the record | **`feb85268`** |
| Agreement with the recorded pin | **AGREES** — resolved value equals the value recorded in SPEC-356b R0.0b, so B1 proceeds rather than stopping |
| Lineage name | **INSTRUMENTED** (§0(a)) |
| Binary | built **once** from that SHA: `cargo build --release --bin topgun-server --bench soak_harness` |
| Every `matrix.txt` | records it as `  repo HEAD:      feb85268952001813e502e27f65180855676ac25` |

**How the pin was obtained, stated so it is a reading rather than a copy (R0.0b).** B1 does not read the
SHA out of the spec: it resolves the tip of the instrument lineage from `git log main` and then checks
the resolved value against the recorded one. The two agree at execution time, so no cell is measured
under an unrecorded build.

### §9.2 — Build identity and freeze proof, as discharged at this pin

- **Zero post-pin `.rs` (§0(d), R0.0a).** `git diff --stat feb85268..HEAD -- '*.rs'` is **empty**, and so
  is the same diff for `INVARIANTS.md`, `scripts/check-invariants.sh`, `spec349c2-fit.awk`,
  `spec356-prune.sh` and all three `spec356a-*` gate artifacts. The working tree is clean.
- **The freeze proof is COMMIT ORDER, not a hash written into this file.** `git log --follow` over this
  manifest returns **NINE** commits — the freeze at `fc95b86d`, then §8A's **eight** PRE-DATA appends
  (`114 0`, `124 0`, `60 0`, `42 0`, `39 0`, `44 0`, `34 0`, `42 0`), insertions only, end-to-end
  `fc95b86d feb85268` → **`499 0`**. Hop 9 (`feb85268`) appends no addendum and touches no manifest byte.
- **PRE-DATA holds at this pin.** `git log --diff-filter=A --format=%H --reverse --
  '…/evidence/spec356-*.soak.json'` is **empty**, and so is the same query over `--all` for
  `'*spec356*soak.json'`: not one measurement cell has run, so **all nine manifest commits precede every
  `spec356-*.soak.json` vacuously**. From the first such artifact onward, a finding is a POST-DATA record
  routed to a named follow-on — never a predicate edit and never an addendum (§8A preamble, R0.0c).
- **Governing text read for this execution:** §0–§8 **together with** §8A's **TWENTY** addenda
  (`ADJ-1` … `ADJ-20`), all PRE-DATA and all governing.

### §9.3 — The executed control cells (G2)

**PRE-DATA CLOSED.** The first `spec356-*.soak.json` in this history is
`…/evidence/spec356-ctl-r1.soak.json`, added at commit `0cc71f06`. From that commit onward every finding
against §0–§8 or §8A is a **POST-DATA record**: it may be recorded and routed to a named owner, and it may
not alter any predicate, threshold, ordering or conditional (§8A preamble, R0.0c). All twenty addenda were
read at this manifest **before** that artifact existed.

**How the runner was invoked, and why it was not invoked from the main checkout.** The runner records
`git -C "$REPO_ROOT" rev-parse HEAD` into every `matrix.txt` (`spec356-prune.sh:865`). §9.1's own mandated
write moved `main`'s tip past the pin, so a cell run from the main checkout would have recorded the *newer*
SHA and been discarded by checklist 2 — the artifact would have been unusable, not merely untidy. Every
cell below was therefore run from a **worktree checked out at the pin itself**, whose `spec356-prune.sh` is
byte-identical to the pin's blob (`fc66a1e6…`). This is the mechanism R8.3b already uses for cell E's
pre-346 build. **The pin was NOT moved**, and each cell's `matrix.txt` carries
`  repo HEAD:      feb85268952001813e502e27f65180855676ac25` exactly once.

| Cell | Arming | Width | Duration | Exit | Arming witness | INSTRUMENT DEFECT | `STEP0C` | Pin needle |
|---|---|---|---|---|---|---|---|---|
| `spec356-ctl-r1` | armed | 1000 (default) | 1800 s | 1 | PASSED (present) | 0 | 0 | 1 |
| `spec356-ctl-r2` | armed | 1000 (default) | 1800 s | 1 | PASSED (present) | 0 | 0 | 1 |
| `spec356-ctloff-r1` | **disarmed** | 1000 (default) | 1800 s | 1 | PASSED (**absent**) | 0 | 0 | 1 |
| `spec356-ctloff-r2` | **disarmed** | 1000 (default) | 1800 s | 1 | PASSED (**absent**) | 0 | 0 | 1 |

**THE EXIT CODE IS ATTRIBUTED, NOT READ AS A VERDICT — and it is not an admissibility event.** All four
cells end `exit 1`, and on all four the runner prints `RESULT: instrument sound`. The code is the inherited
`exit "$HARNESS_RC"` (`spec356-prune.sh:1647`) carrying the harness's own **tombstone-byte gate**:
`finishedReason` on `ctl-r1` reads *"tombstone-byte growth slope 70492.8 bytes/h exceeds 512.0 bytes/h"*.
**That is the defect this family exists to characterize, observed rather than repaired** — and §1's opening
paragraph already rules that a red tombstone gate is not evidence against `TG-OR-004`. It is **not** exit 9,
**not** an `INSTRUMENT DEFECT`, and **not** a `STEP0C ADMISSIBILITY` routing: convergence failures, recovery
failures and pending gates are all empty **on all four of these control cells**, and the memory gate
(neutralized by design) passed. No cell was discarded and no cell was re-run.

**PENDING-GATE SCOPE, STATED SO THE SENTENCE ABOVE CANNOT BE OVER-READ (added post-review).** The
"pending gates are all empty" clause is scoped to **§9.3's four control cells**, where it is exact:
`pendingGates` is the empty array `[]` on `ctl-r1`, `ctl-r2`, `ctloff-r1`, `ctloff-r2` — and on `w100`. It
is **NOT** empty on the `long` cell. `spec356-long.soak.json` carries exactly one entry:

```
"disk growth slope 143.9 MB/h exceeds 50.0 MB/h — EXPECTED until TODO-566 bounds OR-Map tombstones (report-only, did NOT fail the run)"
```

**This is report-only and it decided nothing.** It is the disk-growth slope `TODO-566` already owns, the
gate's own text says it did not fail the run, and `long`'s `exit 1` is attributed to the tombstone-byte
gate exactly as the four controls' is (`finishedReason` on `long`: *"tombstone-byte growth slope 76298.1
bytes/h exceeds 512.0 bytes/h"*). **No admissibility limb, no control, no step of the R8.1 walk and no
published slot reads `pendingGates`.** It is disclosed here because a reader arriving at §9.3's clause and
carrying it forward to the measurement cells would carry it wrongly — not because anything in the record
depended on it. Owner: **`TODO-566`** (report-only disk-growth slope; untouched by this spec).

**The armed cells' column census.** All 43 columns populated on both `ctl` replicates, `empty=0` on every
column, with the six INSTRUMENT-tagged columns nonzero and the 37 MEASURAND columns taking the
population-only shape — the shape R4.3a assigns them.

**The disarmed cells' empty prune-record columns are the SPECIFIED behaviour (ADJ-6, ADJ-9), and are
reported here as such rather than as a finding.** Each `ctloff` replicate's `prune.csv` carries **180 data
rows (`ctloff-r1`) and 179 (`ctloff-r2`) at the same 10 s cadence as the armed arm** — the sampler runs
identically, so its cost stays common-mode and §1.2's *"one difference"* survives — with the
prune-record columns empty and only the
sampler-local `elapsed_secs` and the inherited `topgun_ormap_tombstone_bytes_total` populated. What was
checked on these cells is the arming witness in its **absent** direction and the sampler's own liveness, and
nothing else. **Reporting an instrument finding against the pin on a disarmed cell is forbidden**, and none
is reported.

**`ctloff-r2` ENDED ONE SAMPLER TICK EARLY, AND THE PUBLISHED LEVEL IS UNAFFECTED — corrected
post-review, having originally been stated as a flat `180` for both replicates.** Re-counted from the
committed bytes: `ctloff-r1.prune.csv` carries **180 data rows, last `elapsed_secs` 1790 → 1800**;
`ctloff-r2.prune.csv` carries **179, last `elapsed_secs` 1790**. The same one-tick shortfall appears on
that replicate's **primary** CSV — 30 data rows to the other three cells' 31, last `elapsed_secs` **1740**
against **1800** — while `durationSecsActual` is **1800** on all four, so the run itself was full length
and only the final scrape is missing.

- **The cadence claim stands**: both replicates sample at 10 s (prune) and 60 s (primary), unchanged. The
  shortfall is a missing terminal row, not a different sampling rate, so §1.2's *"one difference"* and the
  common-mode sampler cost are untouched.
- **The published level is arithmetically unaffected, re-derived rather than asserted.** §9.4's reduction
  is the mean of `tombstone_bytes` over the **coordinate** last-half window, and a coordinate window moves
  with the ledger it is read from: `ctloff-r2`'s midpoint is `(0 + 1740)/2 = 870 s` rather than `900 s`, and
  the window it selects still holds **15 rows** — the same count as every other control. Re-running the
  reduction over the real 15-row window returns **32706.666667**, byte-for-byte the level §9.4 publishes.
  Every one of §9.4's six levels was re-derived at this fix and all six reproduce, including SPEC-355's
  `38715.466667` / `36624.133333`.
- **This shape is not novel in the family**: SPEC-355's own committed arm-B CSV `spec355-sweep1000b.csv`
  has the identical geometry — 30 primary rows, last `elapsed_secs` 1740 — and §1.1 publishes its level
  without qualification. The coordinate-window reduction is what makes both robust.
- **No admissibility limb reads a row count.** ADJ-17 exists precisely so no boundary is a row index; the
  ledger's row count is not a free parameter of any window, split or verdict in this record.

**Replicate production (R4.4a).** Each replicate ran in its **own** `SPEC356_OUT_DIR` and its own data dir,
both outside the tracked evidence dir, and was then **copied** — never moved, never renamed in place — to
its `-r1` / `-r2` basenames. `SPEC356_FORCE=1` was never set and no control was re-run in place; either
would have destroyed replicate 1, which is the run the df = 2 two-sample `t` needs. Each run's
`${BASE}.artifacts.sha256` was verified with `shasum -a 256 -c` **in the scratch dir**, where the digest
file's own un-suffixed basenames still resolve, **before** the copy; every artifact matched. The
`scratch OUT_DIR → committed basename` mapping is committed as
`…/evidence/spec356-control-replicate-map.txt`, **six lines at close** — the four control lines below,
written here at G2, plus a `w100` line and a `long` line appended at G3 when those two cells ran from
scratch dirs of their own — and it reconciles green against **all six** copied `matrix.txt` files
(checklist 13: exactly one map line per basename; each `  csv:` and `  console log:` names that line's
dir). **The count was originally stated as "four lines", which was true when this paragraph was written
and stale by the time the wave closed; it is corrected here post-review.** The four rows tabulated below
are §9.3's own subject — the control replicates — and are unchanged.

| Scratch `SPEC356_OUT_DIR` | Committed basename |
|---|---|
| `…/scratchpad/ctl-r1-out` | `spec356-ctl-r1` |
| `…/scratchpad/ctl-r2-out` | `spec356-ctl-r2` |
| `…/scratchpad/ctloff-r1-out` | `spec356-ctloff-r1` |
| `…/scratchpad/ctloff-r2-out` | `spec356-ctloff-r2` |

*(The two G3 lines — `…/scratchpad/w100-out → spec356-w100` and `…/scratchpad/long-out → spec356-long` —
are recorded at §9.5's cell block, which is where those cells are reported. **Checklist 13's inline
`# must be 4` comment is stale for the same reason and is deliberately NOT edited**: it is a graded
checklist surface, and this is a POST-DATA record against it — **the same drifted-own-body class as PD-5,
and it routes with PD-5 to `TODO-637` (§9.11 row 9)** — not a repair. The comment is diagnostic only; the
item's pass condition is `rc`-based, not count-based, so the verbatim repaired form still runs green.
**Re-run at this fix over the committed six-line map it returns `PASS ctl-r1 / ctl-r2 / ctloff-r1 /
ctloff-r2`, `rc=0`, with `wc -l` reporting 6** — i.e. the stale comment and the green `rc` coexist, which
is precisely why the comment is a record and not a defect.)*

**Artifact binding (R4.5 / ADJ-20).** Every cell's nine committed files include its runner-written
`${BASE}.artifacts.sha256`, and each digest file entered history **in the same commit as the bytes it
names** — `0cc71f06`, `0655842a`, `75a801bd`, `b073d9af`. No commit adds one without the other.

### §9.4 — The two measured controls, their MDEs, the family-wise rate, and the §1.3 cell

**The reduction is the one the graded driver re-derives, not a bespoke one.** Each replicate contributes
**one level**: the mean of its `tombstone_bytes` column over the **ADJ-17 coordinate last-half window**
(rows whose `elapsed_secs` exceeds the full ledger's elapsed midpoint), empty cells skipped exactly as the
pinned fitter skips them. Run against SPEC-355's two committed HEAD CSVs the same reduction returns
**38715.466667** and **36624.133333** — i.e. it reproduces the `38,715 · 36,624` pair §1.1 publishes, from
the real bytes, without being told the answer.

| Replicate | Coordinate-window level (B) |
|---|---|
| `spec356-ctl-r1` | 37788.666667 |
| `spec356-ctl-r2` | 38297.600000 |
| `spec356-ctloff-r1` | 38624.666667 |
| `spec356-ctloff-r2` | 32706.666667 |
| `spec355-sweep1000` (arm B of §1.1) | 38715.466667 |
| `spec355-sweep1000b` (arm B of §1.1) | 36624.133333 |

**§1.1 / R5.1 — CROSS-LINEAGE**, this build's `ctl` pair against SPEC-355's committed HEAD pair. Arm means
38043.133 and 37669.800; `s_pooled = 1076.184028`; grand mean `37856.466667`; two-sample
R5.1's `t = 0.346905` against its pre-registered critical value **4.303** (df 2, two-sided α = 0.05) ⇒
**NOT REJECTED**. Recomputed MDE, by §1.1's own formula `4.303 × s_pooled / mean × 100` = **12.232573 %**.

**§1.2 / R5.2 — WITHIN-LINEAGE**, one build, one lineage, one difference: `ctl` (armed) against `ctloff`
(disarmed). Arm means 38043.133 and 35665.667; `s_pooled = 2969.921596`; grand mean `36854.400000`;
`t = 0.800515` against 4.303 ⇒ **NOT REJECTED**. Recomputed MDE = **34.675840 %**.

**Family-wise error rate (§1.4).** Two tests, each at α = 0.05, each pre-registered as a **non-rejection**
predicate. Under independence the probability that at least one control rejects when the instrument is in
fact neutral is `1 − 0.95² = 0.0975` ≈ **9.8 %** — roughly a 1-in-10 chance of a spurious *adverse* reading
across the control set, not the 5 % a single test suggests. **No α correction is applied**, deliberately,
per §1.4: a false adverse reading costs a re-run while a false clean reading would license invalid numbers.
Neither control rejected, so no single rejection has to be read against this rate here.

**THE §1.3 / R5.4 CELL, NAMED: `R5.1 passes / R5.2 passes` ⇒ CLEAN.** Both controls hold, both MDEs are
stated, and **classification proceeds**. **The BUILD-LINEAGE cell did NOT fire**, so ADJ-5's
*"STAND, BOUNDED AND FLAGGED"* qualification is not triggered by this control set and no cross-lineage
bound on unexplained shift is owed from it. **No blocking cell fired**, so nothing routes to `TODO-637` and
G3 is not gated by this walk.

**Reclaim fraction, reported BESIDE the controls as corroboration and never as the deciding statistic
(§1.1).** Over the armed pair, `topgun_or_prune_bytes_freed_total ÷ topgun_ormap_tombstone_bytes_total` at
the ledger's last row is **63.5 %** (`ctl-r1`) and **69.3 %** (`ctl-r2`), against §1.1's committed HEAD
reference of 80.1 % at width 1000 / 1800 s. The disarmed arm emits neither counter, so it contributes no
reclaim figure — by design, not by omission. **This is corroboration only and decides nothing here**; the
deciding statistic is the level mean above, and it did not reject.

**POST-DATA RECORDS — recorded and routed, changing no predicate.** Both are readings taken after the data
boundary closed, so neither may adjust a threshold, an ordering or a conditional.

1. **The within-lineage control came out LOWER-powered than the cross-lineage one on these data**
   (MDE 34.7 % versus 12.2 %), inverting the "higher power" expectation §1.2's own heading carries. The
   cause is visible in the table: `ctloff-r2`'s level (32,706.7) sits ~5,900 B below its own arm-mate's,
   which inflates the pooled sd. The predicate is pre-registered as a **formula over the observed pooled
   sd**, not as a number, so the verdict stands exactly as computed — but a NOT-REJECTED at this MDE
   excludes even less than the reporting bound's `≈ 17 %`. Routed to **`TODO-638`**, which already owns the
   control set's power and blind-spot questions.
2. **The armed pair's reclaim fraction (63.5 % / 69.3 %) runs below §1.1's committed HEAD reference
   (80.1 %).** It is a cross-lineage comparison of a corroborating statistic and decides nothing under
   §1.1. Routed to **`TODO-634`** as an input to the classification waves, which read the reclaim path
   directly.

### §9.5 — The executed measurement cells, and the R8.1 walk in its FROZEN ORDER (G4)

**Governing text read for this walk: §0–§8 TOGETHER WITH §8A's TWENTY addenda, read AT THIS MANIFEST.**
`ADJ-1` … `ADJ-20`, all PRE-DATA, all governing. Six of them supersede another addendum rather than
§0–§8 — ADJ-15 → ADJ-13, ADJ-16 → ADJ-5 and ADJ-15's degeneracy scope, ADJ-17 → ADJ-15's slice
construction, ADJ-18 → ADJ-17's split axis and density band, ADJ-19 → ADJ-18's clauses 2 and 3, ADJ-20 →
ADJ-18's clause 4 — so neither §0–§8 alone nor §8A read piecemeal is the predicate this section walks.

**PRE-DATA IS CLOSED** (§9.3). Every finding below is a **POST-DATA record**: recorded, routed to a named
owner, acting on no predicate, no threshold, no ordering and no conditional.

| Cell | Arming | Width | Duration | Exit | `RESULT:` | Arming witness | INSTRUMENT DEFECT | `STEP0C` | Pin needle | `prune.csv` rows |
|---|---|---|---|---|---|---|---|---|---|---|
| `spec356-w100` | armed | **100** | 1800 s | 1 | `instrument sound` | PASSED (present) | 0 | 0 | 1 | 180 |
| `spec356-long` | armed | 1000 (default) | 14,400 s | 1 | `instrument sound` | PASSED (present) | 0 | 0 | 1 | 1440 |

**The exit code is attributed exactly as it was for the control cells, and it is not an admissibility
event.** Both cells end `exit 1` carrying the harness's inherited **tombstone-byte growth-slope gate**,
and the runner prints `RESULT: instrument sound` on both. It is **not** `exit 9`, **not** an
`INSTRUMENT DEFECT` line and **not** a `STEP0C ADMISSIBILITY` routing. Both cells STAND.

**Artifact binding (R4.5 / ADJ-20), verified BEFORE any content check — checklist 18 limb (0).** Each
cell's seven artifacts were verified against its runner-written `${BASE}.artifacts.sha256` with
`shasum -a 256 -c` in the evidence directory: **`spec356-long` 7/7 GREEN, `spec356-w100` 7/7 GREEN**, and
each digest file entered history in the same commit as the bytes it names. **The classification below
reads the `long` cell, and the `long` cell's provenance limb is GREEN verbatim.** The four control
replicates carry a naming residual recorded as POST-DATA finding **PD-1** in §9.7; their bytes are
GREEN 28/28.

**The graders were run from the COMMITTED SIDECARS, digest-checked before execution.**
`spec356-tmplconf.awk` `7c14b900…`, `spec356-slottruth.sh` `280f7a34…`, `spec356-skeleton.txt`
`bfaac337…` — all three matched their pinned values, and the grader re-verified the driver's digest
itself before executing it (`MODE -- driver sha256 280f7a34…f060 == DRIVER_SHA256`). The re-derivation ran
in the mandated `-v ev=` mode against the real evidence directory; **fixture mode was not used.**

#### §9.5.1 — STEP 0: the FOUR admissibility limbs, each with its evaluated value

Evaluated over the `long` cell's **ADJ-17/ADJ-18 COORDINATE last-half window** — the rows whose
`elapsed_secs` exceeds the FULL ledger's elapsed midpoint. Full ledger: 1440 rows, `t_first = 10 s`,
`t_last = 14,400 s`, midpoint **`7,205 s`**. Window: **720 rows, `t_lo = 7,210 s`, `t_hi = 14,400 s`,
span 7,190 s.** No window and no split anywhere below is a row index (ADJ-17); the ledger's row count is
not a free parameter of any boundary.

| Limb | Rule | Evaluated value | Verdict |
|---|---|---|---|
| **(a)** | §1.3's 2×2 landed in **CLEAN** or **BUILD-LINEAGE-EFFECT** | **`R5.1 passes / R5.2 passes` ⇒ CLEAN** — re-derived by the driver: `R51_RESULT = NOT REJECTED` (`t = 0.346905`), `R52_RESULT = NOT REJECTED` (`t = 0.800515`), against the pre-registered critical value `4.303` | **PASSES** |
| **(b)** | R1.3a reports **≥ 1 split recompute** in the window read (ADJ-1, fail-closed) | **Δ`topgun_or_prune_split_recomputes_total` = 465**; `topgun_or_prune_split_computed_epoch` moves 230 → 462 across the window | **PASSES** |
| **(c)** | every deciding column passed the R4.3 population check **AND**, per ADJ-12, `topgun_or_prune_drain_refs_p50` is READABLE — **`≤ 50 %` of its last-half rows are the 0-sentinel** | population check: all 43 columns `n=1440 empty=0`, zero `STEP0C ADMISSIBILITY`, zero `INSTRUMENT DEFECT` ⇒ **passes**. ADJ-12 readability: **708 of 720 last-half rows are the 0-sentinel ⇒ sentinel fraction `98.333333 %`, against the pre-registered `50 %` threshold** | **FAILS** |
| **(d)** | ADJ-11 clause 0, evaluated FIRST of all: `Δpasses = Δempty_drains + Δnonempty_drains` | **Δpasses = 232,367**; **Δempty_drains = 232,365**; **Δnonempty_drains = 2**; `232,365 + 2 = 232,367` | **HOLDS** |

**LIMB (c) FAILS. THE DETERMINATION IS THEREFORE `INDETERMINATE` (§2 Step 5), AND NO LATER STEP IS
EVALUATED.** This is ADJ-12's escape hatch firing on its own pre-registered threshold, and it is a
**PROTOCOL EXIT, NOT A THIRD BRANCH**: it leaves the ordered predicate exactly as it was, the Step 2 /
Steps 3–4 partition remains the pair of exact complements on `median(L) ≤ B` / `median(L) > B`, and
nothing is added between them. **Reading this hatch as a third determination would be out of contract,
and none is read.**

**Why the hatch is doing its job here rather than obstructing it, stated as a measurement.** The rendered
p50 is a **60 s rolling window** (`3 × 20 s`, committed) over a summary that records **NON-EMPTY drains
only**, so an expired window renders `0`. Over these 7,190 s the prune executed **232,367 passes and
exactly 2 non-empty drains**. The twelve non-sentinel rows are the two 60 s windows those two drains
populate — six scrapes each, at `t = 12,120…12,170 s` and `t = 13,410…13,460 s`, every one of them
reading `999.9447405703887`. **`B` would rest on two observations.** The same column on the `w100`
keeping-up contrast cell reads **0 % sentinel** over its own coordinate last-half window (216 non-empty
drains in 900 s, `B = 99.992983`), so the unreadability is a property of the measured regime at width
1000, **not an instrument fault** — the runner's population check passes on the column in both cells.

**THE PASS-RATE DEGENERACY IS NOT WHAT HALTED THIS WINDOW, and the distinction is load-bearing.** ADJ-16
clause 3 removed `s_early < 5` from Step 0 and scoped it to the Steps 3/4 evaluation. That limb is **not
engaged here at all**: `s_early = 998.589613 ≥ 5`, so the window is nowhere near the pass-rate noise
floor. **Limb (c) carries ADJ-12's sentinel hatch and NOT ADJ-15's degeneracy** — the sentinel hatch is
genuinely an unreadable **deciding column** (`B` appears in Step 2's own predicate, upstream of
everything), and that is the limb that failed.

**ADJ-19 clause 1's minimum-evidence gate did NOT route this window.** The parent window carries **232
positive `current_epoch` jumps** — far above the `≥ 5` floor — every jump exactly `1`, median positive
jump `1.000`, bound `10 × 1.000 = 10.000`, maximum jump `1`. The epoch axis is non-decreasing. **No
classification leaf is published here off a parent window with fewer than five positive epoch jumps**,
and none could have been: this window has 232.

**The remaining coordinate guards, re-derived rather than assumed** (ADJ-18 clauses 1–3, ADJ-19
clause 2). Split midpoint of the window's own elapsed span: `(7,210 + 14,400)/2 = 10,805 s`. Early half
`n = 360`, span 3,590 s, density `1.002786 ×`; late half `n = 360`, span 3,590 s, density `1.002786 ×` —
both inside `[0.8×, 1.2×]`, enforced per half as well as for the ledger (`1.000695 ×`). Maximum gap
between consecutive `elapsed_secs`: **11 s = 1.1 × cadence in each half**, against the `5 ×` bound.
Distinct `current_epoch` values: **117 per slice**, against the floor of 2. The committed
`spec356-long-passrate{,-early,-late}.csv` are convenience copies; the driver materialised the parent and
both slices itself from `spec356-long.prune.csv` and `cmp`-ed them clean.

#### §9.5.2 — The step that fired, the n = 1 caveat, and the routing consequences

**THE STEP THAT FIRED IS STEP 5 — `INDETERMINATE` — via Step 0 limb (c).** Steps 1, 2, 3 and 4 were **not
evaluated**, because Step 0 is evaluated before any step below it and its failure is fail-closed. The
values reported in §9.5.3 are ADJ-3's **unconditional evidence bundle**; they are **not** step
evaluations, and **no leaf is read off any of them**.

**THE n = 1 CAVEAT, IN THIS PARAGRAPH BY OBLIGATION (§2.6).** The `long` and `w100` cells are **n = 1**.
SPEC-355 §10.4.2's lesson is that the **level** replicates (5.4 % spread at width 1000) while the
**slope** does not (it moved 2.0×, 4.6× and changed sign), so this record rests on a **single
unreplicated 4 h series**. **Which kind of statistic decided, stated as §2.6 requires:** the halt turned
on a **COUNT** — the census of 708 sentinel rows in 720 — and on two further counts, the 465 split
recomputes and the conservation identity over three monotone counters. **No slope entered the
determination at all.** The two pass-rate slopes reported below are the fragile quantities, and they are
reported as evidence rather than as a verdict, so the n = 1 slope fragility touches the bundle and not
the outcome. What the n = 1 fragility does bear on directly is the **repeat §8.1 mandates**, which
doubles replicates precisely for it.

**THE REGISTRY-BRANCH ROUTING CONSEQUENCE DOES NOT ATTACH, AND THAT IS THE POINT OF SAYING SO.** §2.5's
routing consequence is triggered by **Step 2 firing** — the modal *"prune drains its licensed work while
the pinned pool grows"* shape, which routes to the **REGISTRY branch of the TODO-634 family and NOT to a
prune-accelerator**. **Step 2 did not fire, because it was never evaluated.** The window's `median(L)`
and `B` are reported below as bundle items; they are **not** a Step-2 determination, they carry **no**
REGISTRY routing, and reading them as one would be reading the ordered predicate past a failed
admissibility gate. **This run routes nothing to the registry-versus-accelerator decision.**

**§8.3 QUOTED BESIDE THIS STEP-5 OUTCOME, as §8.3's own last sentence requires:**

> **The recommended reclamation model closes safety REGARDLESS of which cause it turns out to be.**
> `ReclamationRegistry` (cursor-shaped consumers only) + retention SLA **N = 30 d** + the cursor-age
> fence with HLC-horizon quarantine + `ceiling = min_live_claim − fixed_margin` bound the reclaimable set
> by **live claims**, not by any hypothesis about *why* the current prune falls behind. **A selection
> defect, a scheduling defect and a throughput defect are all *contained* by a registry that never
> reclaims below a live claim.**
>
> What an unclassified cause costs is **fix-shape efficiency** — the family would design without knowing
> which limb to optimize first — **not safety, and not the family's ability to proceed.** A Step-5
> outcome is therefore to be read as **an expensive answer, not a blocked one**.

**THE ROUTING TARGET, PRE-REGISTERED BEFORE THE DATA EXISTED (§8.1).** This outcome routes to **exactly
one repeat of the deciding configuration at DOUBLED duration AND DOUBLED replicates — the `long` cell at
28,800 s with n = 2 — under the SAME pin and the SAME frozen predicate**, which may not adjust a
threshold, an ordering or a conditional. If that repeat is still INDETERMINATE, §8.2 escalates the
unclassified cause as an **explicit input to TODO-634's design phase**, naming it, quoting every step's
evaluated value and the admissibility limb that blocked. **Both are recorded here as owed, not as done:
the repeat is a 8 h measurement this spec does not execute.** Owner: **TODO-634**.

**ADJ-4's BOUND, worded as ADJ-4 requires even though no mechanism is named.** A determination under this
predicate is reported as **"the best-supported of the FOUR PRE-REGISTERED mechanisms", never as "the
cause"**. **This run names NO best-supported mechanism**: the admissibility gate closed upstream of the
discrimination, so SELECTION/FRONTIER, SCHEDULING/LICENSING and THROUGHPUT are all left standing and none
is endorsed. The bound's second obligation is unchanged and is discharged here: **a mechanism outside the
four is NOT excluded**, and the residual evidence that would indicate one is named — **per-pass cost
growth, index-scan cost, allocator and page-cache effects — none of which this instrument observes.**

#### §9.5.3 — ADJ-3's UNCONDITIONAL REPORTING BUNDLE, due whatever step fires

**(i) The pass-rate fit and its FLOOR-ONLY result — computed and reported even though Steps 3/4 were
never reached.** Series: ADJ-13's **cumulative** construction — `passes_total` (y) against
`current_epoch` (x) over **ALL** last-half rows, **no grouping, no exclusion, no row discarded**. The
per-epoch-delta construction ADJ-13 forbids by name was not computed. Fitter: the **unforked**
`spec349c2-fit.awk` at ADJ-15's pinned invocation `-v col=passes_total -v xaxis=current_epoch
-v window=full`, run once per derived slice.

| | `s_early` | `s_late` |
|---|---|---|
| **RAW slope (ADJ-15 clause 4)** | **998.589613 passes/epoch** | **998.845155 passes/epoch** |
| Fitter field | `slope_per_x_unit` | `slope_per_x_unit` |
| x axis / window | `current_epoch` / `full` | `current_epoch` / `full` |
| rows / x span | 360 / 230.0 → 346.0 | 360 / 346.0 → 462.0 |
| r² | 0.999931 | 0.999933 |

**FLOOR-ONLY VERDICT (R8.1d).** `s_early = 998.589613 ≥ 5` ⇒ **the floor is cleared; the window is NOT
degenerate**, so ADJ-16 clause 3's coarsened leaf is not in play. `s_late ≤ 0.5 × s_early` ⇒
`998.845155 ≤ 499.294807` is **FALSE**, so **the decline test does NOT fire.** Ratio
`s_late / s_early = 1.000256` — the pass rate is flat to within 0.03 % across the two halves.

**SENSITIVITY at 0.4 / 0.6, with the iron no-routing rule.** `998.845155 ≤ 0.4 × s_early = 399.435845` is
FALSE; `998.845155 ≤ 0.6 × s_early = 599.153768` is FALSE. **The verdict does not flip anywhere in the
band, so ADJ-3's CONTESTED label does NOT attach on this leg.** The verdict at `0.5` governs, as it would
have regardless: **CONTESTED changes exactly zero routing decisions.**

**AND THE DECLINE-CONTESTED LABEL DOES NOT ATTACH EITHER.** R8.1b's label fires on *"a Step-1 or Step-2
determination that coincides with ADJ-15's floor-only test firing"*. The floor-only test did not fire,
and there is no Step-1 or Step-2 determination to attach it to. **No CONTESTED label is applied anywhere
in this record.**

**Per ADJ-15, no α, no standard error, no confidence interval and no "significantly below" is claimed for
this fit.** The SE leg is REMOVED (ADJ-15 clause 2). The fitter still prints `se_per_x_unit`
(`0.439085` early, `0.432885` late); those numbers are **printed, not used**, and carry no inferential
weight here.

**(ii) `L` statistics over the same window, with the `L ≤ B` fraction.**

| Statistic | Value |
|---|---|
| `min(L)` | **0** |
| **`median(L)`** — the ADJ-2 aggregator, the only one that decides | **0** |
| `max(L)` | **1000** |
| fraction of last-half samples with `L ≤ B` | **403 / 720 = 55.972222 %** |

**`L` is exactly bimodal over this window: 403 rows read `0` and 317 rows read `1000`** — one epoch's
refs at width 1000 — with no intermediate value. **`min(L)` is reported and does NOT decide** (ADJ-2);
any evaluation of Step 2 or Step 3 that read it would be reading the pre-ADJ-2 predicate, and none is
made here in any case.

**R8.1b IS NOT ENGAGED, AND THE REASON IS NAMED RATHER THAN LEFT TO INFERENCE.** R8.1b's counter-anchored
clause table is triggered by **`max(L) = 0`**. Here **`max(L) = 1000 ≠ 0`**: the repaired ADJ-7 emitter
observed non-zero licensed backlog at 317 of 720 sample instants, so the gauge is a reading and not a
construction. **No clause of R8.1b — neither clause 2's ELIGIBILITY-BOUND evidence nor clause 3's
INADMISSIBLE-plus-INDETERMINATE — is taken, and this record leans neither way on a bare `max(L)`.**
R8.1b's clause 0 and clause 1 are separately Step 0's limbs (d) and (b) and both hold. For completeness,
because clause 2's conjuncts are cheap and their absence should be visible: **ADJ-14's named backlog
series `Δbacklog_bytes := Δ topgun_ormap_tombstone_bytes_total − Δ topgun_or_prune_bytes_freed_total` =
`5,349,731 − 46,000` = `+5,303,731` bytes** over the window, both counters from the same 10 s
`prune.csv` rows; and `Δnonempty_drains = 2 ≠ 0`, which is clause 2's own disqualifying conjunct.

**ADJ-7's boundary rule, observed.** `L` and `P` are **PRE-drain**, `indexed_refs` is **POST-drain**, so
**no share is taken across that boundary anywhere in this record.** The within-snapshot share
`L / (L + P)` over the window: **mean 0.269592, min 0.000000, max 0.761035**. `P`
(`topgun_or_prune_ineligible_refs`, PRE-drain) moves **1031 → 537** across the window, range 314…1157 —
a **falling**, not a growing, pinned pool. ADJ-7's prohibited ratios `P / indexed_refs` and
`L / indexed_refs` are **never** reported here; those quantities have no referent.

**(iii) `B`, with ADJ-12's provenance and its quality figure.**

| | Value |
|---|---|
| **`B`** | **999.944741** |
| Source (ADJ-12, and no other estimator is admissible) | the **median** over the coordinate last-half `prune.csv` rows of committed column **43**, `topgun_or_prune_drain_refs_p50` — the exporter-rendered p50 of the batch-size summary over its committed `3 × 20 s = 60 s` rolling window, **0-sentinel rows EXCLUDED** |
| **0-sentinel EXCLUDED FRACTION (unconditional)** | **708 / 720 = 98.333333 %** |
| Observations `B` actually rests on | **12 rows, behind exactly 2 non-empty drains** |
| Non-sentinel values present | `999.9447405703887`, all twelve |

**This is the figure that failed Step 0 limb (c)**, and it is reported here because R8.1a(iii) makes the
fraction unconditional — a `B` published without it is incomplete even when the fraction is 0. **No `n`,
no sd, no standard error and no t-statistic is computed by counting rows of the p50 column** (§5.4,
restated by ADJ-12 and observed here). **`B` was never derived from `Δ_sum / Δ_count` — that estimator is
REJECTED by ADJ-12 and MUST NOT be used for `B`.** The differenced mean is reported in §9.6 as a
THROUGHPUT quantity only, which is the role §5.4 keeps it in.

**(iv) The non-drop exit share and the PER-EXIT breakdown from the ledger.**

| Exit | Δ over the window |
|---|---|
| `topgun_or_prune_matched_nothing_total` | **0** |
| `topgun_or_prune_absent_total` | **0** |
| `topgun_or_prune_restored_read_error_total` | **0** |
| `topgun_or_prune_restored_evicted_total` | **0** |
| `topgun_or_prune_restored_write_error_total` | **0** |
| **non-drop numerator** | **0** |
| `topgun_or_prune_considered_total` (denominator) | **2,000** |
| `topgun_or_prune_dropped_total` | **2,000** (100.000000 % of considered) |
| **non-drop exit share** | **0.000000 %** |

**The denominator is stated beside the ratio because it is small and the reader must see it.** Over a
window carrying 232,367 passes, only **2,000 refs were ever considered** — the two non-empty drains, at
1,000 refs each. Every one of them was `Dropped`. **A zero share over a 2,000-ref denominator is a
different object from a zero share over a large one**, and the pairing is what makes that visible on the
record rather than inferred.

**(v) ADJ-11's DIAGNOSTIC COLUMN — `Δpasses` stated ALONGSIDE the window's scrape count, unconditional.**

| Quantity | Value |
|---|---|
| **`Δpasses` over the window** | **232,367** |
| **window scrape count** | **720** |
| ratio | **322.73 passes per 10 s scrape** |
| `Δempty_drains` | 232,365 |
| `Δnonempty_drains` | **2** |
| `Δrestored_read_error_total` | **0** (flat) |
| `Δrestored_write_error_total` | **0** (flat) |

**The under-sampling regime is therefore VISIBLE rather than inferred**, which is this limb's whole
purpose: at 322.73 passes per scrape a 10 s gauge sample of `L` bounds nothing about the intervening
passes, and a reader shown both numbers can tell that from a window where the prune genuinely idled.

#### §9.5.4 — What was NOT done, listed because each is named out of contract

- **No ROW-INDEX split** — every window and split above is a coordinate (ADJ-17/ADJ-18).
- **No `current_epoch`-MIDPOINT split** — slice membership is `elapsed_secs ≤ (t_lo + t_hi)/2` of the
  window (ADJ-18 clause 1); only the **fit** axis is `current_epoch`.
- **No `slope_mb_per_hour`** on any pass-rate figure — that field belongs to the `tombstone_bytes`
  byte-slope fits alone. Both pass-rate slopes are reported from the field `slope_per_x_unit`, which is
  the rename that makes a wrong-axis fit visible without further checking.
- **No per-epoch-delta series** (ADJ-13, forbidden by name).
- **No α, no SE-based claim, no "significantly below"** (ADJ-15 clause 2 / R8.1d).
- **No classification leaf other than INDETERMINATE** — and the parent window carries 232 positive epoch
  jumps, so ADJ-19 clause 1's floor was never the constraint.
- **No whole-window INDETERMINATE reported off a degenerate pass rate** — the pass rate is not degenerate
  here (`s_early = 998.589613`), and the INDETERMINATE this record publishes comes from ADJ-12's sentinel
  hatch at Step 0 limb (c), which is a different mechanism with a different owner.

**CORRECTION, POST-REVIEW — THIS LIST WAS INCOMPLETE AT MERGE, AND THE OMISSION IT MISSED WAS ITS OWN
SUBJECT.** The bullet above about `slope_mb_per_hour` says the field *"belongs to the `tombstone_bytes`
byte-slope fits alone"*, and at merge **those fits' results were nowhere in this manifest.** R6 obliges
this spec to *"report all 8 slopes with standard errors"* for the `long` cell's eight `tombstone_bytes`
window fits. At the close-out commit `bf07ae3d`:

- **The obligation was UNDISCHARGED.** The eight segments were sliced and committed
  (`spec356-long-seg{1..8}.csv`, commit `37f184f0`) and the fits were run, but **not one slope, standard
  error or r² appeared in §9, in any committed artifact, or in that commit's own message.**
- **AND the omission was UNDISCLOSED.** It was absent from this very list — the section whose job is to
  enumerate what was not done — absent from §9.11's routing table, and absent from §9.13.3's terminal
  state. **Both halves are stated because the second is the worse one:** an undischarged obligation that
  is disclosed is a known gap with an owner; one that is not is a silent hole, and this was the latter.

**It is discharged in full at §9.14**, appended by this fix, with the fitter's verbatim output for all
eight windows. **Nothing above this note moves**: no window, no split, no admissibility limb, no step of
the R8.1 walk and no published slot reads a byte-slope, and §9.5.2 already records that **no slope entered
the determination at all**. This is a reporting gap closed by reporting — not a verdict that changed.
Routed at §9.11 rows 17 and 18.

---

### §9.6 — The DUAL-USE capture (B7a): claim spans, LWM cadence, prune-batch sizes

Same `long` cell, same coordinate last-half window (720 rows, 7,190 s). This capture is **dual-use** — it
is the evidence base the `ReclamationRegistry` claim/retention model is designed against, and it is owed
whatever the classification returns.

| Quantity | Series | Δ over the window | Derived |
|---|---|---|---|
| **Claim span** | `claim_span_epochs_{sum,count}` | sum **0**, count **465** | **mean 0.000000 epochs** over 465 observations |
| **Claim lag** | `claim_lag_epochs_{sum,count}` | sum **0**, count **465** | **mean 0.000000 epochs** over 465 observations |
| **Tracked claims** | `tracked_claims` | — | **constant 1** across all 720 rows (min 1, max 1) |
| **LWM position** | `low_water_mark` | 230 → 462 | advances in lockstep with `current_epoch` (230 → 462) |
| **LWM advances** | `lwm_advances_total` | **232** | **cadence 30.991 s per advance** |
| **LWM epochs advanced** | `lwm_epochs_advanced_total` | **232** | **1.000 epoch per advance** |
| **LWM stall** | `lwm_stall_seconds` | 8 → 26 | **max 34 s** over the window |
| **Prune batch size** | `drain_refs_{sum,count}` | sum **2,000**, count **2** | **1000.000000 refs per non-empty drain** (the `_sum`/`_count` differenced mean, §5.4's PRIMARY reading for a THROUGHPUT quantity) |
| **Drain breadth** | `drain_epochs_{sum,count}` | sum **2**, count **2** | **1.000 epoch per drain** |
| **Epochs drained** | `epochs_drained_total` | **2** | — |
| **Split recomputes** | `split_recomputes_total` | **465** | 0.646 recomputes per scrape |

**THE TWO FREQUENCIES, STATED SEPARATELY BECAUSE THEY ARE THE POINT OF THE TABLE.**

| Frequency | Per scrape | Per pass |
|---|---|---|
| **LWM movement** | **0.322222 advances / scrape** | **0.000998 advances / pass** (232 in 232,367) |
| **Non-empty drain** | **0.002778 drains / scrape** | **0.0000086 drains / pass** (2 in 232,367) |

**What the table shows, reported as observation and routed nowhere.** The low-water mark **moves freely**
— 232 advances, one epoch each, every ~31 s, tracking `current_epoch` exactly — and the tracked claim set
is a **single claim of zero span and zero lag** for the whole window. **So the prune is not LWM-stalled
and is not claim-pinned in this window**, while it converted 232,367 passes into 2 non-empty drains and
freed 46,000 bytes against 5,349,731 bytes of tombstone arrival. **This is evidence, not a
determination**: Step 0 closed upstream of the step that would have read it, and nothing here is routed
to the registry-versus-accelerator decision. It is committed so the repeat §8.1 mandates, and TODO-634's
design phase, inherit it rather than re-measure it. Owner: **TODO-634**.

**The `w100` keeping-up contrast, for the same reason.** Over its own coordinate last-half window (90
rows, 900 s): 29,991 passes, **216 non-empty drains**, 294 LWM advances, `B = 99.992983` at **0 %
sentinel**, `Δbytes_freed = 651,200` against `Δtombstone_bytes = 659,802` — **a backlog delta of +8,602
bytes, versus the `long` cell's +5,303,731.** Non-drop exit share **0.000000 %** there too. The contrast
is committed as capture; it decides nothing here.

---

### §9.7 — Cell E's disposition under R8.2, and the POST-DATA records

#### §9.7.1 — R8.2 applied, with the ledger numbers that ruled it

**R8.2's two branches, evaluated on this run's determination and this run's ledger:**

| Branch | Condition | Evaluated | Fires? |
|---|---|---|---|
| **RUN** | determination is **SELECTION/FRONTIER** | determination is **INDETERMINATE** | **NO** |
| **RUN** | **or** non-`Dropped` exits **> 10 %** of considered over the `long` run's last half | **0.000000 % ≤ 10 %** (numerator 0, denominator 2,000) | **NO** |
| **CLOSE as not needed** | determination is **THROUGHPUT** or **SCHEDULING** | determination is **INDETERMINATE** | **NO** |
| **CLOSE as not needed** | **and** non-`Dropped` exit share **≤ 10 %** | **0.000000 % ≤ 10 %** — **satisfied** | (conjunct met, branch still not reached) |

**BOTH NUMBERS ARE QUOTED, as §3 requires — the redundancy is a cross-check, not a licence to omit one.**
The exit share is **0.000000 %** and the literal is **10 %**; the ledger numbers that ruled it are
`Δconsidered = 2,000`, `Δdropped = 2,000`, and all five non-`Dropped` exit counters flat at **0**.

**THE DISPOSITION: CELL E DOES NOT FIRE. IT IS NOT RUN.** R8.2's RUN branch is a biconditional over a
disjunction both of whose disjuncts are false, so **not running is the rule's own answer**, not a
judgement call. **It is equally NOT "CLOSED as not needed":** that branch's *first* conjunct requires a
THROUGHPUT or SCHEDULING determination, and this walk produced neither. **G5 executes no cell E.**

**§3's coherence argument, checked rather than assumed — and it is where the gap shows.** §3 argues the
rule is self-consistent by construction: a THROUGHPUT/SCHEDULING determination can only be reached via
Step 3 or Step 4, both of which require exit share ≤ 10 %, so CLOSE's second conjunct is automatically
satisfied whenever its first is; and a SELECTION/FRONTIER determination reached via Step 1 automatically
satisfies RUN. **Both implications hold and neither has an antecedent here.** The argument is stated over
the four *mechanism* determinations and is silent on the **fifth outcome the same predicate can
produce** — `INDETERMINATE`, which §2.3 Step 5 and ADJ-12's hatch both route to. Recorded as **PD-2**
below.

#### §9.7.2 — POST-DATA records: recorded, routed, acting on nothing

**Every item here is a reading taken after the data boundary closed (§9.3). None alters a predicate, a
threshold, an ordering or a conditional, and none was acted on.**

**PD-1 — ADJ-20's BASENAME binding and R4.4a's replicate RENAME are in tension, and the verbatim
checklist 18 limb (0) command REDs on the four control replicates.** ADJ-20 clause 1 writes
`${BASE}.artifacts.sha256` *"by BASENAME, so the file verifies unchanged after the scratch-to-evidence
copy"*; R4.4a step 5 requires each replicate to run in its own scratch `SPEC356_OUT_DIR` under the
**un-suffixed** cell name and then be copied to the `-r1` / `-r2` committed basenames. The copy therefore
**renames**, and the committed digest files name `spec356-ctl.csv` where the committed artifact is
`spec356-ctl-r1.csv`. **Measured at this wave:** the verbatim loop reports `RED PROVENANCE` on all four
control replicates on **name resolution**, while the byte check under the committed
`spec356-control-replicate-map.txt` rename passes **28 of 28, byte-identical** — and §9.3 already records
that G2 verified each set with `shasum -a 256 -c` **in the scratch dir, before the copy**, where the
digest file's own basenames still resolve. **The two measurement cells `long` and `w100` are GREEN
verbatim, with no mapping**, and the classification above reads the `long` cell. **Disposition:** the
artifacts' bytes ARE their runs'; `RED PROVENANCE`'s own remedy — *"re-run the cell, do not re-read
it"* — is inapplicable, because a re-run under R4.4a reproduces the rename identically. **This is a
finding against ADJ-20 clause 1's naming construction, which a POST-DATA record may not edit.** Routed to
**TODO-648**, which already owns checklist 18 limb (0)'s mechanics.

**PD-2 — R8.2 and the `CELLE_DISPOSITION` enum are both silent on an INDETERMINATE determination.**
R8.2's RUN and CLOSE branches are stated over SELECTION/FRONTIER, THROUGHPUT and SCHEDULING; §2.3 Step 5
can also return `INDETERMINATE`, and under it **neither branch fires**. R5.7(e)'s closed enum for
`CELLE_DISPOSITION` carries `CLOSED-NOT-NEEDED` | `RUN-NON-REPRODUCTION-RECORDED-AND-SPUN-OFF` |
`RUN-REPRODUCTION-WIDENS-CLAIM` | `RUN-INDETERMINATE`, and **none of the four describes "cell E does not
fire because the classification is INDETERMINATE"**: `CLOSED-NOT-NEEDED` is glossed as *"R8.2's CLOSE
branch"*, whose precondition is unmet, and the three `RUN-` values presuppose the cell ran. **The
disposition itself is unambiguous — cell E is not run — but the normative surface has no value for it.**
Routed to **TODO-634**, as an input to the family that owns the follow-on; the slot is G5's to fill and
this record is what it must fill against.

**PD-3 — the committed driver's `B` is NOT ADJ-12's `B`, and the divergence is what suppressed
`MEDIAN_L_OVER_B`.** ADJ-12 defines `B` as the median of column 43 with **0-sentinel rows EXCLUDED**;
`spec356-slottruth.sh` takes the median over **all** last-half rows of that column and emits the ratio
only when that median is `> 0`. On this window the two differ maximally: **ADJ-12's `B` = 999.944741**
(12 non-sentinel rows), the driver's = **0** (708 of 720 rows are the sentinel), so the driver's `bb > 0`
guard fired and the slot was not emitted. **The driver's refusal is correct in substance** — it declined
to divide by a zero, and the predicate's answer to the identical condition is ADJ-12's hatch, which this
walk took — but the two disagree on *which* quantity `B` is, and only one of them is the pre-registration.
**Consequence at the grading layer, measured:** checklist 18 limb (a)'s required set names
`MEDIAN_L_OVER_B` among eleven, so it reports `FAIL 18 -- truth file is INCOMPLETE -- 1 of 11
re-derivable slots are MISSING and therefore UNGRADED: MEDIAN_L_OVER_B`, and checklist 19 is
`WITHHELD` behind it. **Both are structural on this evidence set and neither can be repaired without
editing a pinned sidecar, which this wave may not do.** Routed to **TODO-648**.

**PD-4 — the exit-share denominator is small in absolute terms, and the record says so rather than
letting the percentage stand alone.** `0.000000 %` is computed over `Δconsidered = 2,000` in a window of
232,367 passes. R8.1e names this coupling for the degenerate-pass-rate leaf — *"a prune that barely runs
considers barely anything"* — and although that leaf is not the outcome here, the same coupling holds and
is reported for the same reason: the two statistics degrade together and the record must show both.
Routed to **TODO-634** as an input to the repeat §8.1 mandates.

#### §9.7.3 — Grader verdicts at this wave, stated as an intermediate state

Run from the committed sidecars in the mandated `-v ev=` mode, after checklist 18 limb (0):

| Item | Verdict at this commit | Why |
|---|---|---|
| **limb (0) provenance** | **GREEN** on `spec356-long` and `spec356-w100` (7/7 each), verbatim | the cells the classification reads |
| **15** — template is the pinned skeleton, byte-exact modulo slots | **PASS** | the block is pasted, not retyped; only slot values moved |
| **16** — every slot well-formed | **FAIL on ONE residual token**, `{{CELLE_DISPOSITION}}` | a wave boundary: R8.2/R8.3's slot is G5's, and PD-2 records that R8.2 supplies no enum value for this outcome |
| **18** — re-derivation | **`SKIP` at this commit** (the grader does not attempt 18 or 19 while a `{{…}}` token stands); **`FAIL` once the residual token is filled** — `truth file is INCOMPLETE -- 1 of 11 re-derivable slots are MISSING and therefore UNGRADED: MEDIAN_L_OVER_B` | structural on this evidence set (PD-3); the driver declined the ratio and the driver is a pinned sidecar this wave may not edit |
| **19** — reconciliation | **`SKIP` at this commit**; then **`NOT APPLICABLE` on its own terms**, reported `WITHHELD` behind item 18 | `STEP_LEAF` is `INDETERMINATE`, so limb (b) has nothing to reconcile — Step 0/5 admissibility is decided upstream of these four slots, and the escape runs in the safe direction only |

**Both verdicts for 18 and 19 are stated because the grader reports the second only after the first is
cleared, and a record that showed only `SKIP` would hide PD-3.** The `FAIL 18` / `WITHHELD 19` line above
was produced by running the same committed grader, in the same `-v ev=` mode, over this manifest with the
one residual token provisionally substituted — a diagnostic run against a scratch copy, changing no
committed byte.

**Recording this as a stated intermediate state is the point, exactly as it was at G2:** a block that
graded green while carrying an ungraded classification would be the defect. **Item 19's
`NOT APPLICABLE` is not a pass and is not claimed as one** — no comparison was performed, because
`INDETERMINATE` reads no classification number off any run and therefore cannot launder one.

---
### §9.C — Controls and dynamics: the PINNED TEMPLATE

**This block is the pinned skeleton `…/evidence/spec356-skeleton.txt` (sha256 `bfaac337…`, 46 lines,
2,288 B), pasted rather than retyped, with numeric slots filled and no other byte edited (R5.7(b)).** The
executor never types the mandated sentences, so they cannot be mistyped, re-wrapped or truncated.

**SLOT FILLING IS A WAVE LEDGER, and it is kept current rather than restated once.** G2 filled the ten
control-derived slots. G3 filled the six the `long` cell's measurement supplies — `EXIT_SHARE_PCT`,
`S_RATIO`, `S_EARLY`, `S_LATE`, `S_EARLY_FIELD`, `S_LATE_FIELD` — and deliberately left
`MEDIAN_L_OVER_B` open, because the committed driver DECLINED to emit it and a statistic a driver
declined to compute is not zero. **G4 fills the two the R8.1 walk derives:** `STEP_LEAF` =
`INDETERMINATE` (the walk halted at Step 0 limb (c) on ADJ-12's sentinel hatch — §9.5.1), and
`MEDIAN_L_OVER_B` = `0.000000`, which is `median(L) = 0` over **ADJ-12's** `B` = `999.944741`, the median
of column 43 with the 0-sentinel rows EXCLUDED as the addendum requires. The excluded fraction —
**98.333333 %** — is reported beside it at §9.5.3(iii), unconditionally, and it is the very figure that
failed limb (c).

**G5 fills the nineteenth and last: `CELLE_DISPOSITION` = `NOT-FIRED-DETERMINATION-INDETERMINATE`.**
That value is **deliberately OUTSIDE R5.7(e)'s four-value closed enum**, and the reason is §9.7.2's
record **PD-2**: under an `INDETERMINATE` determination **neither R8.2 branch fires**, and none of
`CLOSED-NOT-NEEDED` / `RUN-NON-REPRODUCTION-RECORDED-AND-SPUN-OFF` / `RUN-REPRODUCTION-WIDENS-CLAIM` /
`RUN-INDETERMINATE` is true of this run. **All nineteen slots now carry a value; the block is complete
and it is NOT green**, and §9.8.2 states in full which value was written, why, what it costs at
checklist 16, and who owns the gap. **The block is therefore NOT in its graded final state:**
checklist 15 (byte-exactness modulo slots) passes, checklist 16 FAILS on this one out-of-enum value —
by construction, as the honest signal that the normative surface has no value for this outcome — and
checklists 18 and 19 carry the dispositions §9.7.3 and §9.13 tabulate. Recording that as a stated,
measured state is the point; a block that graded green while publishing a claim this run cannot prove
would be the defect.

<!-- TG356B-CTRL BEGIN v2 -->
### 9.C — Controls and dynamics (PINNED TEMPLATE v2 — fill slots only, edit no other byte)

NORMATIVITY: this block is the ONLY surface the acceptance criteria read for the controls, the
single-arm dynamics claims and the published classification leaf. Prose elsewhere in §9 is
NON-NORMATIVE for those claims and carries no adjudicated finding.

NEUTRALITY STATEMENT 1 of 3 — R5.1 (role-swap control)
result: NOT REJECTED
observed pooled sd: 1076.184028
observed pair mean: 37856.466667
recomputed MDE: 12.232573 %
MDE ≈ 17 %
a smaller perturbation is NOT excluded
role: CATASTROPHE DETECTOR; n = 6 DECLINED at ≈ 4 h
a dynamics-only perturbation from recorder presence or activation is not excluded by any control in this protocol

NEUTRALITY STATEMENT 2 of 3 — R5.2 (order control)
result: NOT REJECTED
observed pooled sd: 2969.921596
observed pair mean: 36854.400000
recomputed MDE: 34.675840 %
MDE ≈ 17 %
a smaller perturbation is NOT excluded
role: CATASTROPHE DETECTOR; n = 6 DECLINED at ≈ 4 h
a dynamics-only perturbation from recorder presence or activation is not excluded by any control in this protocol

NEUTRALITY STATEMENT 3 of 3 — cell E disposition
disposition: NOT-FIRED-DETERMINATION-INDETERMINATE
MDE ≈ 17 %
a smaller perturbation is NOT excluded
role: CATASTROPHE DETECTOR; n = 6 DECLINED at ≈ 4 h
a dynamics-only perturbation from recorder presence or activation is not excluded by any control in this protocol

DECLINED n = 6 EXTENSION — COST RE-DERIVED, NOT TYPED (ADJ-10)
8 control cells × 1800 s = 4 h

SINGLE-ARM DYNAMICS OBSERVATIONS (ARMED ARM ONLY)
non-drop exit share = 0.000000 % — NOT a comparison; no control arm exists for these
median(L)/B = 0.000000 — NOT a comparison; no control arm exists for these
s_late / s_early = 1.000256 — NOT a comparison; no control arm exists for these
s_early = 998.589613 passes/epoch raw, from fitter field slope_per_x_unit — NOT a comparison; no control arm exists for these
s_late = 998.845155 passes/epoch raw, from fitter field slope_per_x_unit — NOT a comparison; no control arm exists for these
dynamics blind spot owner: TODO-638

CLASSIFICATION LEAF PUBLISHED BY THIS RUN (R8.1's frozen ordered predicate)
leaf: INDETERMINATE
<!-- TG356B-CTRL END v2 -->

---

### §9.8 — Cell E's recorded disposition, and the nineteenth slot (G5)

**PRE-DATA remains CLOSED** (§9.3). Everything in §9.8 through §9.13 is a **POST-DATA record**: it is
recorded, it is routed to a named owner, and it acts on no predicate, no threshold, no ordering and no
conditional.

#### §9.8.1 — CELL E IS NOT RUN, AND THIS IS THE RECORD OF IT WITH THE LEDGER NUMBERS THAT RULED IT

**G5 executed no cell E.** The decision is R8.2's own, taken on a rule frozen before any measurement
existed, and it is recorded here with its numbers rather than asserted.

**The ledger, RE-DERIVED at this wave rather than copied forward from §9.5.3(iv).** G5 recomputed the
window and every counter delta directly from the committed `spec356-long.prune.csv` under ADJ-17/ADJ-18's
coordinate rule — full ledger 1440 rows, `t_first = 10 s`, `t_last = 14,400 s`, midpoint **`7,205 s`**;
window = the rows whose `elapsed_secs` exceeds that midpoint, **720 rows, `t_lo = 7,210 s`,
`t_hi = 14,400 s`, span 7,190 s**; each Δ taken between the window's first and last rows. Every value
below reproduces §9.5.3(iv) exactly.

| Ledger quantity (coordinate last-half window of the `long` run) | Value |
|---|---|
| `topgun_or_prune_considered_total` — **Δconsidered, the denominator** | **2,000** |
| `topgun_or_prune_dropped_total` — **Δdropped** | **2,000** (100.000000 % of considered) |
| `topgun_or_prune_matched_nothing_total` | 0 |
| `topgun_or_prune_absent_total` | 0 |
| `topgun_or_prune_restored_read_error_total` | 0 |
| `topgun_or_prune_restored_evicted_total` | 0 |
| `topgun_or_prune_restored_write_error_total` | 0 |
| **non-`Dropped` NUMERATOR** (the five counters above, summed) | **0** |
| **NON-`Dropped` EXIT SHARE** | **0.000000 %** |
| **THE PRE-REGISTERED LITERAL it is compared against** | **10 %** |
| `Δpasses` over the same window, quoted so the denominator is read in context | 232,367 |

**THE RULE WAS FROZEN BEFORE THE DATA EXISTED, and that is the whole of its value.** §3's own words:
*"The 10 % literal is arbitrary-but-fixed. Its role is to make the decision un-chooseable after the data
lands."* The manifest carrying it is PRE-DATA by commit order (§9.2), and §3 was never edited: the
`10 %` above is read out of the frozen section, not chosen at this keyboard. **Both numbers are quoted,
as §3 requires** — the share `0.000000 %` and the literal `10 %` — because §3 keeps the redundancy as a
cross-check and calls omitting either a licence it does not grant.

**R8.2 EVALUATED, LIMB BY LIMB — AND NEITHER BRANCH'S ANTECEDENT IS SATISFIED.**

| R8.2 branch | Conjunct / disjunct | Evaluated at this run | Holds? |
|---|---|---|---|
| **RUN** | determination is **SELECTION/FRONTIER** | determination is **INDETERMINATE** (§2 Step 5, via Step 0 limb (c)) | **NO** |
| **RUN** | **or** non-`Dropped` share **> 10 %** | **0.000000 % > 10 %** is FALSE | **NO** |
| ⇒ | RUN fires iff either disjunct holds | both disjuncts FALSE | **RUN DOES NOT FIRE** |
| **CLOSE-as-not-needed** | determination is **THROUGHPUT** or **SCHEDULING** | determination is **INDETERMINATE** | **NO** |
| **CLOSE-as-not-needed** | **and** non-`Dropped` share **≤ 10 %** | **0.000000 % ≤ 10 %** is TRUE | yes — but it is the SECOND conjunct of a conjunction whose FIRST is false |
| ⇒ | CLOSE fires iff both conjuncts hold | first conjunct FALSE | **CLOSE DOES NOT FIRE** |

**SO THE DISPOSITION IS: CELL E DOES NOT FIRE, AND IT IS NOT "CLOSED AS NOT NEEDED".** Not running is
R8.2's own answer to a disjunction both of whose disjuncts are false — it is the rule's output, not a
judgement made at execution time. **It is emphatically not R8.2's CLOSE branch**, whose first conjunct
requires a THROUGHPUT or SCHEDULING determination and which this walk did not produce. Calling this run
`CLOSED-NOT-NEEDED` would assert a determination that was never reached; calling it any `RUN-` outcome
would assert a cell that was never executed. **Neither is written anywhere in this record.**

**WHAT B8 DOES AND DOES NOT GET FROM THIS, STATED PLAINLY RATHER THAN SMOOTHED OVER.** B8 enumerates
exactly two limbs — **(a) RUN** and **(b) CLOSED as not needed** — and **this run takes NEITHER**, for
the reasons the table above measures. What is discharged is B8's binding closing sentence: *"It must not
lapse unrecorded, and the disposition must not be a footnote."* The disposition is recorded here, in a
titled subsection, with the pre-registered rule, the determination, the exit share, the denominator and
the frozen literal all quoted as numbers. **What is NOT discharged is B8's two-limb enumeration itself**,
because the enumeration is silent on the fifth outcome the same predicate can produce. That silence is
**PD-2** (§9.7.2), it is a finding against a normative surface a POST-DATA record may not edit, and its
owner is **TODO-634**.

**And the consequence for the carried question, so it cannot be read as answered:** the
**2026-07-13 → 2026-07-27 interval REMAINS UN-PROBED**. Cell E's *disposition* is discharged; the
*interval* is not, and no claim about `6c35785a` (sf-346) or `2769570f` (sf-347) is made or widened here
in either direction. Ownership returns intact to **TODO-634**, which carried it in from SPEC-355 and
carries it out of SPEC-356b unchanged.

**R8.3b's three provenance exports were NOT set, and that is correct rather than an omission.**
`SPEC356_PIN_SHA`, `SPEC356_PIN_CMD` and `SPEC356_PIN_WORKTREE` exist to make a cell-E `matrix.txt`
provable. No cell E ran, so there is no `matrix.txt` to provenance, no pre-346 binary was built, no
half-swap was performed and no `spec356-cellE.*` artifact exists in this evidence directory. **Exporting
them would have provenanced nothing.**

#### §9.8.2 — THE NINETEENTH SLOT: WHICH VALUE WAS WRITTEN, AND WHY IT IS OUT OF ENUM ON PURPOSE

**Written:** `CELLE_DISPOSITION` = **`NOT-FIRED-DETERMINATION-INDETERMINATE`**.

**It is NOT one of R5.7(e)'s four admissible values, and the choice is deliberate and loud.** Each of the
four would have published something this run cannot prove:

| Enum value | What writing it would have asserted | Why it is false here |
|---|---|---|
| `CLOSED-NOT-NEEDED` | R8.2's CLOSE branch fired | its **first conjunct** requires a THROUGHPUT or SCHEDULING determination; this walk produced **INDETERMINATE** |
| `RUN-NON-REPRODUCTION-RECORDED-AND-SPUN-OFF` | cell E ran and did **not** reproduce | **no cell E ran** |
| `RUN-REPRODUCTION-WIDENS-CLAIM` | cell E ran and reproduced, widening the no-regression claim to 2026-07-13 | **no cell E ran**, and no claim is widened |
| `RUN-INDETERMINATE` | cell E ran and its result was **unreadable** | **no cell E ran**; the INDETERMINATE here is the *classification's*, produced upstream of cell E and before it could have been scheduled |

**The disposition is unambiguous; the normative surface has no value for it.** That asymmetry is exactly
**PD-2**, and the honest way to carry it onto the graded block is to write a value that **says the true
thing and REDs**, rather than one that says a false thing and passes. `NOT-FIRED-DETERMINATION-INDETERMINATE`
is self-describing in the enum's own shape and cannot be mistaken for a run that happened.

**WHAT IT COSTS, MEASURED RATHER THAN PREDICTED.** Run from the committed sidecars in the mandated
`-v ev=` mode, the grader reports:

```
PASS 15 -- block is the pinned skeleton, byte-exact modulo slots
FAIL 16 -- slot CELLE_DISPOSITION = 'NOT-FIRED-DETERMINATION-INDETERMINATE' is not well-formed (grammar ENUM_CELLE)
SKIP 18 -- not attempted: the block is not a well-formed filled skeleton
SKIP 19 -- not attempted: the block is not a well-formed filled skeleton
exit=1
```

**This FAIL is the signal, not the defect.** `spec356-tmplconf.awk:56-57` admits four literals and
nothing else; the value this run must publish is not among them; so the grader's red is the normative
surface reporting its own gap at exactly the point a reader will look. **No sidecar was edited to widen
the enum** — `spec356-tmplconf.awk` is a pinned, digest-checked, un-editable file under this spec's
constraints, and widening a grammar after the data lands is precisely the post-hoc adjustment the
pre-registration exists to prevent. **No R5.7(e) byte was edited either**, for the same reason.

**The counterfactual is stated because it is the tempting one and it is wrong.** Substituting
`RUN-INDETERMINATE` makes the grader print `PASS 16 -- all 19 slots well-formed` and unblocks items 18
and 19 — which then report `FAIL 18 … MEDIAN_L_OVER_B` and `WITHHELD 19` (PD-3, §9.13). **That green on
item 16 would have been bought with a false claim** — that a cell ran which did not — and it would not
have produced a green record anyway. **A green that cannot be proven is the defect; the FAIL above is the
record being honest at the cost of its own colour.**

**Owner of the gap: `TODO-634`**, which owns the cell-E question and the follow-on family. The concrete
ask carried to it is narrow and stated so it can be actioned rather than re-derived: R8.2 needs a limb
for the INDETERMINATE determination, and `CELLE_DISPOSITION`'s enum needs the corresponding value —
**both are changes to a pre-registered surface, so both belong to a spec that RE-PINS, never to an
execution.**

---

### §9.9 — The §8.1 REPEAT OBLIGATION: **OWED, NOT DONE**

**This spec does NOT execute the repeat.** The obligation is recorded here in full, as owed, so that it
cannot be read as discharged by the paragraph that names it.

| | The obligation, as §8.1 pre-registered it before the data existed |
|---|---|
| **What** | **exactly one repeat** of the deciding configuration |
| **Which cell** | the **`long`** cell — the one whose walk returned INDETERMINATE |
| **Duration** | **28,800 s** (2 × 14,400 s) — **DOUBLED** |
| **Replicates** | **n = 2** (2 × 1) — **DOUBLED** |
| **Pin** | the **SAME** pin, `feb85268952001813e502e27f65180855676ac25` |
| **Predicate** | the **SAME** frozen predicate — §0–§8 together with §8A's twenty addenda |
| **Forbidden** | it **may not adjust a threshold, an ordering or a conditional** |
| **STATUS AT THE CLOSE OF SPEC-356b** | **OWED — NOT DONE** |
| **Owner** | **TODO-634** |

**Both axes are doubled, and §8.1's reason for each is carried rather than paraphrased away.** Doubling
**duration** addresses an effect too slow to separate in 4 h; doubling **replicates** addresses the n = 1
fragility SPEC-355 §10.4.2 measured — the level replicates at 5.4 % spread while the slope moved 2.0×,
4.6× and changed sign. **One axis alone leaves the other cause unaddressed.** The repeat is a **second
observation, not a re-specification.**

**Why this spec does not run it, stated as a scope fact rather than an excuse.** It is an **8 h
measurement** at n = 2 — 16 h of cell clock — against a wave budgeted for discharge and close-out. It is
not deferred because it is hard; it is deferred because it is a *new measurement round*, and a
measurement round belongs to the item that owns the follow-on. **Nothing about it is softened here:** no
threshold is loosened in anticipation, no shortcut configuration is proposed, and the deciding
configuration named above is the one that must run.

**IF THE REPEAT IS STILL INDETERMINATE, §8.2 ESCALATES — and the escalation is not "record a note".** It
**names the unclassified cause, quotes the evaluated value of every step that failed to fire and the
admissibility limb that blocked**, and is carried into the family's design phase **as a stated open
input**. Owner: **TODO-634**. This record already supplies what that escalation would quote for round
one: Step 0 limb (c) blocked on ADJ-12's `>50 %` sentinel rule at **708/720 = 98.333333 %**; Steps 1–4
were **never evaluated**; §9.5.3 carries every bundle value.

**§8.3 QUOTED BESIDE THIS OUTCOME, as §8.3's own last sentence requires of ANY Step-5 outcome:**

> **The recommended reclamation model closes safety REGARDLESS of which cause it turns out to be.**
> `ReclamationRegistry` (cursor-shaped consumers only) + retention SLA **N = 30 d** + the cursor-age fence
> with HLC-horizon quarantine + `ceiling = min_live_claim − fixed_margin` bound the reclaimable set by
> **live claims**, not by any hypothesis about *why* the current prune falls behind. **A selection defect,
> a scheduling defect and a throughput defect are all *contained* by a registry that never reclaims below
> a live claim.**
>
> What an unclassified cause costs is **fix-shape efficiency** — the family would design without knowing
> which limb to optimize first — **not safety, and not the family's ability to proceed.** A Step-5 outcome
> is therefore to be read as **an expensive answer, not a blocked one**.

**So the honest summary of this spec's terminal state, in one sentence:** the classification round ran to
completion under a predicate frozen before the data, returned **INDETERMINATE** on a pre-registered
admissibility hatch, routed to a repeat it does **not** perform, and left the reclamation family's safety
argument **untouched and intact** — the registry model does not depend on which mechanism this round
failed to name.

---

### §9.10 — Lineage close-out: the NINE PRE-DATA re-pins, and what each hop obliged

**Recorded here so a reader who diffs `fc95b86d..feb85268` finds an explanation rather than an anomaly.**
All nine moves are **PRE-DATA** — every one landed before the first `spec356-*.soak.json` existed (§9.2,
§9.3) — so each cost nothing: no cell to discard, no matrix to re-run, and **§0–§8 byte-identical across
all nine**.

| Hop | To | PR | What moved | Gate it obliged |
|---|---|---|---|---|
| — | `fc95b86d` | #133 | the FREEZE: SPEC-356a's instrument + manifest §0–§8 | — (the baseline) |
| 1 | `efa2c249` | #134 | manifest §8A (ADJ-9/10/11) **and the RUNNER** — cell E's arming disarmed | both Wave-1 gate artifacts |
| 2 | `f2f72c62` | #135 | manifest §8A (ADJ-12/13/14) **and the RUNNER** — `B`'s committed source | both Wave-1 gate artifacts, re-captured |
| 3 | `cb9682e5` | #136 | manifest §8A (ADJ-15) **and the FITTER only** | the fitter's default-path regression proof (190 lines, `cmp` clean; checklist 6) |
| 4 | `f9d02d3e` | #137 | manifest **only** (ADJ-16) | **none** — re-capturing one would be theatre |
| 5 | `07b1fda6` | #138 | manifest **only** (ADJ-17) | **none** |
| 6 | `1f18b2b3` | #139 | manifest **only** (ADJ-18) | **none** |
| 7 | `55edacb1` | #140 | manifest **only** (ADJ-19) | **none** |
| 8 | `c0d8cc73` | #141 | manifest §8A (ADJ-20), **the RUNNER** (`19  0`), **and three new read-only sidecars** | both Wave-1 gate artifacts, in #135's Part-III shape |
| 9 | **`feb85268`** | #142 | **no program byte and no manifest byte** — it IS hop 8's gate discharge (`PART IV`, `126  0` and `78  0`, taken at the ADJ-20 runner) | **none of its own**; it CLOSES hop 8's, and **TODO-647 is RESOLVED** |

**`TODO-637` DID NOT FIRE ON ANY OF THEM, and §9 records only that observation.** TODO-637 owns exactly
two **POST-DATA** branches — a **blocking R5.4 cell** and a **missing pinned column** — and neither was
taken: §9.4 names the 2×2 cell **CLEAN**, and §9.5.1 limb (c) records all 43 columns present with
`n=1440 empty=0`. **The PRE-DATA re-pin channel above is NOT TODO-637's** — it is this spec's ordinary
correction path. **No acceptance criterion of this spec ticks, edits or closes `TODO-637`, and this wave
edited no byte of `.specflow/todos/TODO-637.md`.** Closure, if it is ever warranted, belongs to whoever
retires the branch.

**Every cell in this record was run from a worktree checked out AT THE PIN**, and every committed
`matrix.txt` carries `  repo HEAD:      feb85268952001813e502e27f65180855676ac25` exactly once
(§9.3, §9.5). **This wave ran no cell**, so it moved nothing in that ledger.

---

### §9.11 — B13: EVERY BRANCH ROUTED TO A NAMED OWNER

**No outcome of this spec terminates in a paragraph of this manifest.** Each row below names what was
produced, where its evidence is, and **which tracker file owns what remains**. A row with no owner would
be the defect this section exists to prevent.

| # | Branch / outcome of this spec | Evidence | NAMED owner |
|---|---|---|---|
| 1 | **The classification determination: `INDETERMINATE`** (Step 5, via Step 0 limb (c), ADJ-12's sentinel hatch at 708/720 = 98.333333 %) | §9.5.1, §9.5.2 | **`TODO-634`** |
| 2 | **The §8.1 repeat — `long` @ 28,800 s, n = 2, same pin, same frozen predicate — OWED, NOT DONE** | §9.9 | **`TODO-634`** |
| 3 | **§8.2's escalation if the repeat is still INDETERMINATE** (name the cause, quote every step's evaluated value and the blocking limb) | §9.9 | **`TODO-634`** |
| 4 | **Cell E: NOT FIRED. The 2026-07-13 → 2026-07-27 interval REMAINS UN-PROBED** and no claim about `6c35785a` / `2769570f` is made or widened | §9.7.1, §9.8.1 | **`TODO-634`** |
| 5 | **PD-1** — ADJ-20's BASENAME binding vs R4.4a's replicate RENAME; verbatim checklist 18 limb (0) REDs on name resolution for the four control replicates (bytes 28/28 identical under the committed map; `long` and `w100` GREEN verbatim) | §9.7.2 | **`TODO-648`** |
| 6 | **PD-2** — R8.2 **and** R5.7(e)'s `CELLE_DISPOSITION` enum are BOTH silent on an INDETERMINATE determination; the slot is filled out of enum and checklist 16 REDs by construction | §9.7.2, §9.8.2 | **`TODO-634`** |
| 7 | **PD-3** — the committed driver's `B` (median over ALL last-half rows) is not ADJ-12's `B` (0-sentinel rows EXCLUDED); `MEDIAN_L_OVER_B` suppressed; **checklist 18 RED and 19 WITHHELD, structurally, unrepairable without editing a pinned sidecar** | §9.7.2, §9.13 | **`TODO-648`** |
| 8 | **PD-4** — the exit-share denominator is small in absolute terms (Δconsidered = 2,000 inside a window carrying 232,367 passes) | §9.7.2, §9.8.1 | **`TODO-634`** |
| 9 | **PD-5** — SPEC-356b's own body has drifted against its governing set: checklist 12's headline still reads *"All NINETEEN §8A addenda"* and its out-of-range probe `ADJ-2[0-9]\|ADJ-[3-9][0-9]` now matches **ADJ-20 itself**; the G1 task-cell says eight commits / nineteen addenda where the manifest carries nine / twenty. **R8.0b already declares all TWENTY in force, so the GOVERNING SET IS NOT IN DOUBT** and no walk above read the stale count. **Editing a graded checklist mid-execution is forbidden — recorded and routed.** | this row | **`TODO-637`** — the named owner for SPEC-356b changes that must land in a spec rather than at an executor's keyboard |
| 10 | **The dynamics blind spot** — *"a dynamics-only perturbation from recorder presence or activation is not excluded by any control in this protocol"*, stated verbatim beside all three neutrality statements | §9.C block | **`TODO-638`** (this spec does not mutate it) |
| 11 | **The control-power inversion** — R5.2's within-lineage MDE (34.675840 %) came out **worse** than R5.1's cross-lineage (12.232573 %), inverting §1.2's own expectation | §9.4 | **`TODO-638`** |
| 12 | **The armed pair's reclaim fraction below §1.1's committed HEAD reference** (63.5 % / 69.3 % vs 80.1 %) — corroboration only, deciding nothing | §9.4 | **`TODO-634`** |
| 13 | **The DUAL-USE capture** — claim spans, LWM cadence, prune-batch sizes, and the `w100` keeping-up contrast — committed so the family inherits it rather than re-measuring it | §9.6 | **`TODO-634`** (design phase) |
| 14 | **The registry-versus-accelerator decision is NOT routed by this run.** Step 2 did not fire because it was never evaluated; `median(L)` and `B` are bundle items, not a Step-2 determination | §9.5.2 | **`TODO-634`** (the decision remains open there) |
| 15 | **The blocking-cell and missing-column branches DID NOT FIRE** — 2×2 CLEAN, all 43 columns present | §9.4, §9.5.1, §9.10 | **`TODO-637`** stays open and unfired; **this spec edits no byte of it** |
| 16 | **`TG-OR-005`, `NAKED_BASELINE`, the bounded-steady-state demonstration, the level/ceiling gate re-derivation and the 630 → 634 → 586 → 484 sequencing statement** — untouched by this spec, by ruling | §9.13 (repo gate), `INVARIANTS.md` unedited | **`TODO-634`** (checkboxes stay OPEN) |
| 17 | **PD-7 — R6/B5's EIGHT `tombstone_bytes` WINDOW FITS WERE UNREPORTED *AND* UNDISCLOSED AT MERGE, and this row is its closure, not its deferral.** R6 obliges *"report all 8 slopes with standard errors"*; at `bf07ae3d` the segments were committed and the fits run, but no slope, SE or r² existed anywhere in §9, in any artifact, or in `37f184f0`'s message — and the gap appeared in neither §9.5.4, §9.11 nor §9.13.3. **DISCHARGED IN FULL at §9.14** from the unforked committed fitter over the committed segments. **No verdict moves:** §9.5.2 records that no slope entered the determination, and no limb, step or slot reads a byte-slope | §9.14, §9.5.4 correction note | **CLOSED HERE** — the reporting obligation is discharged in this manifest; **no tracker inherits the gap** |
| 18 | **PD-8 — THE ROOT CAUSE IS A CHECKLIST THAT CANNOT FAIL ON AN ABSENT TABLE, and it is a PRE-DECLARATION defect the repeat must not inherit.** Checklist 6(a)'s pass condition is *"8 fits reproduce the slopes **quoted in the manifest**"* — **unsatisfiable-when-none-are-quoted, so it cannot distinguish "reproduced" from "never reported"**; checklist 7(i)'s grep was carved out to tolerate the table's formatting and therefore **passes VACUOUSLY on a §9 that has no table at all**. Same vacuous-guard class as Audit v1 C3, checklist 13's first form, and the `INSTRUMENT_DEFECT`-vs-`INSTRUMENT DEFECT` grep. **THE OBLIGATION ON THE REPEAT: its pre-declaration must carry a PRESENCE limb — a checklist item that REDs when a required reporting artifact is ABSENT, evaluated before any reproduce-the-values limb.** **The frozen checklists are NOT patched here**: editing a graded checklist after the data boundary is exactly what a POST-DATA record may not do, and the repeat is a re-pinning spec, which is the only channel that can | this row, §9.5.4 correction note | **`TODO-634`** — which owns the §8.1 repeat (rows 2 and 3), where the pre-declaration is authored |

**Two rows deserve their prohibition restated, because both are places a reader could mistake a record
for an action.** Rows 5 and 7 are findings against **pinned, un-editable surfaces** — ADJ-20's naming
construction and `spec356-slottruth.sh`'s reduction — and a POST-DATA record **may not** edit either.
Row 6 is a finding against **two** pre-registered surfaces at once (R8.2's branch set and R5.7(e)'s
enum), and widening either after the data landed is exactly the post-hoc adjustment pre-registration
exists to prevent. **All three land in a spec that RE-PINS, or they do not land at all.**

---

### §9.12 — R9's cross-vendor gate, and the one finding it produced

**`/xreview` ran on this spec's executed record at G5, before merge, and is committed to the evidence
directory as `spec356-xreview.md`** — model, invocation, reviewed diff, cost, the reviewer's verbatim
assessment, and **every finding marked APPLIED or REFUTED-WITH-REASON**. **SPEC-356a's `/xask` is CITED,
not repeated:** it had to run before the freeze, and re-running it after the data landed is exactly what
it exists to prevent (R9).

**Three findings returned: ONE verified real and applied, TWO refuted as already-recorded.** F2 is PD-3
and F3 is PD-5 — both already carry an owner (§9.11 rows 7 and 9), and the reviewer concurred with both
dispositions. **Zero findings would have required editing a frozen section, a pinned sidecar, a
predicate, a threshold, an ordering or a conditional, and zero verdicts moved.**

**PD-6 — THE TWO CONTROLS ARE NOT INDEPENDENT, SO §1.4's `9.8 %` IS AN UPPER BOUND RATHER THAN AN EXACT
RATE.** R5.1 tests `{ctl-r1, ctl-r2}` against SPEC-355's committed HEAD pair; R5.2 tests the **same**
`{ctl-r1, ctl-r2}` levels against `{ctloff-r1, ctloff-r2}`. **The `ctl` arm is shared**, so the two
`t`-statistics are computed from common data and are **positively dependent**: a high `ctl` pair pushes
both toward rejection together. Since
`P(at least one rejects) = 0.05 + 0.05 − P(both) = 0.0975` only when `P(both) = 0.0025` (independence),
positive dependence gives `P(both) > 0.0025` and therefore a family-wise rate **at or below** the stated
figure.

- **The error is in the CONSERVATIVE direction**, which is the direction §1.4 chose deliberately: its
  reason for declining an α-correction — *"a false adverse reading costs a re-run while a false clean
  reading would license invalid numbers"* — is **strengthened**, not weakened, by the real rate being no
  larger than the quoted one.
- **No verdict moves.** Neither control rejected (`t = 0.346905` and `t = 0.800515`, both against
  `4.303`), so no rejection has to be read against the family-wise rate at all, at either value.
- **§1.4's `9.8 %` and its "under independence" wording are FROZEN, PRE-DATA text**, and §9.4 quotes them
  faithfully. **A POST-DATA record may not edit either, and neither is edited**; §9.4's committed prose is
  byte-unchanged. This is a record beside the number, not a correction of it.
- **Owner: `TODO-638`**, which already owns the control set's power and blind-spot questions (§9.11 rows
  10 and 11).

**The reviewer's clean cardinal-rule assessment is NOT read as a green, and R5.7(f)'s bound is why.** An
advisory pass is evidence the record is internally consistent with its pre-registration; it is **never**
evidence that the run happened as described. Run provenance carries that, and no reviewer can be asked to
carry it instead. **Checklists 16, 18 and 19 are RED at this commit and stay RED** (§9.13).

---

### §9.13 — The grader state at close, and the repo gate (B12 / B2 / B11)

#### §9.13.1 — THE GRADERS, RUN AT THIS COMMIT FROM THE COMMITTED SIDECARS

All three sidecars were copied into `$PWD` and digest-checked **before** execution — `tmplconf.awk`
`7c14b900…`, `slottruth.sh` `280f7a34…`, `skeleton.txt` `bfaac337…`, all three reproducing their pinned
values from the committed files — and the grader was run in the mandated `-v ev=` mode against the real
evidence directory. **Fixture mode was not used.**

```
PASS 15 -- block is the pinned skeleton, byte-exact modulo slots
FAIL 16 -- slot CELLE_DISPOSITION = 'NOT-FIRED-DETERMINATION-INDETERMINATE' is not well-formed (grammar ENUM_CELLE)
SKIP 18 -- not attempted: the block is not a well-formed filled skeleton
SKIP 19 -- not attempted: the block is not a well-formed filled skeleton
exit=1
```

| Item | Verdict at close | Cause, named |
|---|---|---|
| **limb (0) provenance** | **GREEN** on `spec356-long` and `spec356-w100` (7/7 each), verbatim; **RED PROVENANCE on name resolution** for the four control replicates, whose bytes are nonetheless 28/28 identical under the committed map | **PD-1** → `TODO-648` |
| **15** | **PASS** | the block is the pasted skeleton; only slot values moved |
| **16** | **FAIL** — one out-of-enum value, `CELLE_DISPOSITION` | **PD-2** → `TODO-634`. **This RED is deliberate**: R8.2 produced an outcome the enum has no value for, and the alternative was publishing a claim this run cannot prove (§9.8.2) |
| **18** | **SKIP at this commit** (the grader does not attempt 18 or 19 while item 16 is red); **FAIL under the mandated diagnostic substitution** — `truth file is INCOMPLETE -- 1 of 11 re-derivable slots are MISSING and therefore UNGRADED: MEDIAN_L_OVER_B` | **PD-3** → `TODO-648`. Structural on this evidence set: the driver medians column 43 over ALL last-half rows and ADJ-12 excludes the 0-sentinel rows, so the driver's `bb > 0` guard suppressed the slot. **Unrepairable without editing a pinned sidecar, which is forbidden.** |
| **19** | **SKIP at this commit**; **WITHHELD behind item 18** under the same substitution, and **NOT APPLICABLE on its own terms** because `STEP_LEAF` is `INDETERMINATE` | R5.7(f)'s recorded escape, in the **safe direction only** — `INDETERMINATE` reads no classification number off any run, so it cannot launder one |

**Both readings for 18 and 19 are stated, and hiding either would be the defect.** The `SKIP` is what the
grader prints at the committed bytes; the `FAIL 18` / `WITHHELD 19` is what it prints when the one
out-of-enum value is provisionally replaced by an in-enum one — a diagnostic run against a **scratch
copy, changing no committed byte**, whose only purpose is to show that filling the slot legally would
**not** have produced a green either. **A record showing only `SKIP` would hide PD-3.**

**THESE REDS ARE NOT FORCED GREEN, AND NO ATTEMPT WAS MADE TO FORCE THEM.** No sidecar was edited, no
grammar widened, no required-set entry dropped, no `-v truth=` fixture substituted for the mandated
re-derivation. **Three findings against pre-registered surfaces stand RED with a named owner each**,
which is the outcome the pre-registration is for: a green that cannot be proven is worth less than a red
that can be explained.

#### §9.13.2 — THE REPO GATE, AND B2 RE-VERIFIED AT CLOSE

| Gate | Command | Result |
|---|---|---|
| **B2 — zero post-pin `.rs`** | `git diff --stat feb85268..HEAD -- '*.rs'` | **EMPTY** — 0 lines. **The lineage is intact and every cell in this record was measured on the binary this source builds.** |
| **Frozen programs and sidecars** | same diff over `spec356-prune.sh`, `spec349c2-fit.awk`, `spec356-tmplconf.awk`, `spec356-slottruth.sh`, `spec356-skeleton.txt` | **EMPTY** — 0 lines |
| **Catalog and its ratchet** | same diff over `INVARIANTS.md`, `scripts/check-invariants.sh` | **EMPTY** — 0 lines |
| **The gauge instrument** | same diff over `benches/soak_harness/monitor.rs` | **EMPTY** — 0 lines |
| **Manifest — APPEND-ONLY against the pin** | `git diff --numstat feb85268..HEAD -- …/spec356-manifest.md` | **`1071  0`** — **1,071 insertions, ZERO deletions**, measured at the close-out commit. **The ZERO is the load-bearing half**: §0–§8 and every §8A addendum are byte-identical to the pin, and the 1,390-line prefix `cmp`s clean. *(The insertion count is the only figure in this table that cannot be self-consistent while it is being written — a section reporting its own file's growth is stale the instant it is appended — so it is taken at the commit that closes the wave, and the deletion count, which is what the freeze actually rests on, is invariant to that.)* |
| **Invariant catalog green** | `scripts/check-invariants.sh` | **exit 0** — `invariants: 21 entries, 4 NAKED (baseline 4)` |
| **`TG-OR-005` untouched** | `INVARIANTS.md:481` | **`Status: open (TODO-634)`**, still NAKED; `NAKED_BASELINE=4` at `scripts/check-invariants.sh:20` |
| **Formatting** | `cargo fmt --check` | **exit 0** |
| **Release build** | `cargo build --release --bin topgun-server` | **exit 0** — `Finished release profile [optimized] target(s) in 3m 31s` |

**`cargo test --release -p topgun-server` and `cargo clippy --all-targets --all-features -- -D warnings`
are green BY CONSTRUCTION, and the construction is stated rather than assumed:** B2 holds — the `.rs`
diff against the pin is **empty** — so both run over **byte-identical source** to `feb85268`, which is a
merged commit whose own CI ran them. Nothing in this wave could have changed their outcome; the release
build above is executed as the direct check that the source still compiles at this tree.

**`TG-OR-004` IS NOT FLIPPED, and the reason is §1's frozen text, read rather than re-written.** Every
cell in this record ends `exit 1` carrying the harness's tombstone-byte growth-slope gate, and §1's
opening paragraph already rules that **a red tombstone gate is not evidence against `TG-OR-004`** — gauge
fidelity and corpus boundedness are different properties, and `INVARIANTS.md:564-567` says so in the
catalog itself. **Nothing in this spec bears on `TG-OR-005` either:** a second observation in a new
lineage neither closes nor refutes it.

**No frozen manifest section was edited, and §8A was not appended to under the pin.** This spec's
execution contained **no addendum window** — an addendum is always a RE-PIN — and none was opened.

#### §9.13.3 — THE TERMINAL STATE OF SPEC-356b, STATED WITHOUT ROUNDING

- **The classification round is COMPLETE and its answer is `INDETERMINATE`.** No mechanism is named, a
  mechanism outside the four is not excluded, and the named residual evidence that would indicate one is
  not observed by this instrument.
- **Cell E did NOT fire, and the 2026-07-13 → 2026-07-27 interval is still UN-PROBED.**
- **The §8.1 repeat is OWED, NOT DONE** — `long` @ 28,800 s, n = 2, same pin, same frozen predicate.
- **Checklists 16, 18 and 19 are RED**, each with a cause and a named owner; **15 passes and limb (0) is
  GREEN on both measurement cells.**
- **Eleven branches route to `TODO-634`, two to `TODO-648`, two to `TODO-638`, two to `TODO-637`, and
  one — row 17 — closes in this manifest with no tracker inheriting it.** No branch terminates in a
  paragraph. **This tally is corrected post-review (Review v2) and re-counted from §9.11's eighteen rows:
  `TODO-634` rows 1, 2, 3, 4, 6, 8, 12, 13, 14, 16, 18; `TODO-648` rows 5, 7; `TODO-638` rows 10, 11;
  `TODO-637` rows 9, 15; row 17 CLOSED HERE. It originally read "Six … three … three … two", which was
  wrong against the table as it then stood** — see the correction note at §9.14 for what that miscount
  was and was not.
- **The reclamation family is NOT blocked.** §8.3, quoted at §9.5.2 and again at §9.9: the registry model
  closes safety **regardless of which cause it turns out to be**. What an unclassified cause costs is
  **fix-shape efficiency, not safety** — an expensive answer, not a blocked one.

---

### §9.14 — R6 / B5's EIGHT WINDOWED `tombstone_bytes` FITS: THE LATE DISCHARGE, AND WHAT IT DOES NOT MOVE

**Appended post-review (Review v1, critical 1). This section exists because the obligation below was
undischarged AND undisclosed at the close-out commit `bf07ae3d`** — see the correction note at §9.5.4 for
both halves stated plainly, and §9.11 rows 17 and 18 for the routing. **It is a REPORTING discharge and
nothing else:** no predicate, threshold, ordering or conditional is touched, no frozen section or pinned
sidecar is edited, no addendum is opened, the pin is NOT moved, and **no verdict moves** — §9.5.2 already
records that **no slope entered the determination at all**, and no admissibility limb, no step of the R8.1
walk and no published R5.7 slot reads a byte-slope.

#### §9.14.1 — THE OBLIGATION, QUOTED, AND HOW EACH LIMB IS DISCHARGED

R6, verbatim: *"partition the `long` CSV into 8 consecutive equal header-bearing segments with the
committed one-liner (`spec355-manifest.md` §10.5.2), fit each with the **unforked** `spec349c2-fit.awk` at
`-v col=tombstone_bytes -v window=full`, **report all 8 slopes with standard errors**, and commit the
segments as `spec356-long-seg{1..8}.csv`. **Do not fork the fitter and do not design a new one.**"* B5 adds
r² to the reported set.

| Limb | State at `bf07ae3d` | State here |
|---|---|---|
| Segments committed as `spec356-long-seg{1..8}.csv` | **DISCHARGED** (commit `37f184f0`) | unchanged, byte-identical |
| Fitter unforked | **DISCHARGED** — `git diff --stat feb85268..HEAD -- …/spec349c2-fit.awk` is **EMPTY**, re-checked at this fix; the file digests `840813461e3b1bd5c3a79291044d8ac515e09b94333ee530cd6a10de8fa0436f` | unchanged |
| **Report all 8 slopes with standard errors (+ r², B5)** | **UNDISCHARGED and UNDISCLOSED** | **DISCHARGED at §9.14.2** |

#### §9.14.2 — THE EIGHT FITS, RE-RUN AT THIS FIX, THE FITTER'S OWN OUTPUT PASTED VERBATIM

**Invocation, once per segment, exactly as R6 pins it — no fork, no added parameter, no `xaxis`** (an
explicit `xaxis` is R6a's pass-rate invocation and belongs to a different series; these are the byte-slope
fits, so the fields keep their `_mb_per_hour` names):

```
awk -v col=tombstone_bytes -v window=full -f spec349c2-fit.awk spec356-long-seg<N>.csv
```

**The output below is the program's, pasted unedited — not a transcription.**

```
--- W1 (spec356-long-seg1.csv)
col=tombstone_bytes window=full rows_used=30 n=30 skipped_empty=1 t_start_secs=60.0 t_end_secs=1800.0 span_secs=1740.0 y_first=23328.000 y_last=41668.000 slope_mb_per_hour=46117.828699 se_mb_per_hour=8022.681814 intercept_mb=20580.694253 r2=0.541318 sxx_hours2=0.624305556 sse=1125108400.078902245
--- W2 (spec356-long-seg2.csv)
col=tombstone_bytes window=full rows_used=31 n=31 skipped_empty=0 t_start_secs=1860.0 t_end_secs=3660.0 span_secs=1800.0 y_first=38214.000 y_last=164054.000 slope_mb_per_hour=197635.365249 se_mb_per_hour=8679.028215 intercept_mb=-52549.178824 r2=0.947036 sxx_hours2=0.689027987 sse=1505140565.806789875
--- W3 (spec356-long-seg3.csv)
col=tombstone_bytes window=full rows_used=31 n=31 skipped_empty=0 t_start_secs=3720.0 t_end_secs=5520.0 span_secs=1800.0 y_first=165638.000 y_last=230126.000 slope_mb_per_hour=111163.330645 se_mb_per_hour=9657.589626 intercept_mb=50111.327823 r2=0.820423 sxx_hours2=0.688888889 sse=1863308102.451209545
--- W4 (spec356-long-seg4.csv)
col=tombstone_bytes window=full rows_used=31 n=31 skipped_empty=0 t_start_secs=5580.0 t_end_secs=7380.0 span_secs=1800.0 y_first=229712.000 y_last=298597.000 slope_mb_per_hour=144036.387097 se_mb_per_hour=9980.314764 intercept_mb=8312.664516 r2=0.877784 sxx_hours2=0.688888889 sse=1989920174.090322971
--- W5 (spec356-long-seg5.csv)
col=tombstone_bytes window=full rows_used=31 n=31 skipped_empty=0 t_start_secs=7440.0 t_end_secs=9240.0 span_secs=1800.0 y_first=301817.000 y_last=350117.000 slope_mb_per_hour=93919.572581 se_mb_per_hour=10388.807898 intercept_mb=122125.076210 r2=0.738101 sxx_hours2=0.688888889 sse=2156148205.644756317
--- W6 (spec356-long-seg6.csv)
col=tombstone_bytes window=full rows_used=31 n=31 skipped_empty=0 t_start_secs=9300.0 t_end_secs=11100.0 span_secs=1800.0 y_first=376774.000 y_last=399061.000 slope_mb_per_hour=68027.322581 se_mb_per_hour=8028.692292 intercept_mb=191271.951613 r2=0.712279 sxx_hours2=0.688888889 sse=1287765556.251613617
--- W7 (spec356-long-seg7.csv)
col=tombstone_bytes window=full rows_used=31 n=31 skipped_empty=0 t_start_secs=11160.0 t_end_secs=12960.0 span_secs=1800.0 y_first=396278.000 y_last=440185.000 slope_mb_per_hour=73479.435484 se_mb_per_hour=10815.435417 intercept_mb=174390.600806 r2=0.614144 sxx_hours2=0.688888889 sse=2336873451.056453228
--- W8 (spec356-long-seg8.csv)
col=tombstone_bytes window=full rows_used=24 n=24 skipped_empty=0 t_start_secs=13020.0 t_end_secs=14400.0 span_secs=1380.0 y_first=440208.000 y_last=456860.000 slope_mb_per_hour=66993.393061 se_mb_per_hour=11611.265170 intercept_mb=195809.302706 r2=0.602093 sxx_hours2=0.319476926 sse=947591735.097388983
```

**The table R6 asks for, reduced from the block above and from nothing else:**

| Window | `t_start` … `t_end` (s) | rows used | **`slope_mb_per_hour`** | **`se_mb_per_hour`** | **r²** |
|---|---|---|---|---|---|
| **W1** | 60 … 1,800 | 30 (1 empty skipped) | **46,117.828699** | **8,022.681814** | 0.541318 |
| **W2** | 1,860 … 3,660 | 31 | **197,635.365249** | **8,679.028215** | 0.947036 |
| **W3** | 3,720 … 5,520 | 31 | **111,163.330645** | **9,657.589626** | 0.820423 |
| **W4** | 5,580 … 7,380 | 31 | **144,036.387097** | **9,980.314764** | 0.877784 |
| **W5** | 7,440 … 9,240 | 31 | **93,919.572581** | **10,388.807898** | 0.738101 |
| **W6** | 9,300 … 11,100 | 31 | **68,027.322581** | **8,028.692292** | 0.712279 |
| **W7** | 11,160 … 12,960 | 31 | **73,479.435484** | **10,815.435417** | 0.614144 |
| **W8** | 13,020 … 14,400 | 24 | **66,993.393061** | **11,611.265170** | 0.602093 |

**Three properties of the block, named because a reader should not have to re-derive them:**

- **`rows_used=30` on W1 is the fitter's own `skipped_empty=1`, not a slicing error.** The `long` primary
  CSV's first row carries an empty `tombstone_bytes` cell, and the pinned fitter skips empty cells — the
  same skip §9.4's level reduction inherits. W1's window therefore opens at `t=60 s`, not `t=0`.
- **W8 carries 24 rows, not 31, and that is the committed one-liner's arithmetic.** 241 data rows at
  `seg = 31` gives W1…W7 thirty-one rows each and W8 the remaining 24 — the same shape SPEC-355's own
  eight-window series has, stated in `37f184f0`'s message.
- **The segments are the committed ones.** These fits read `spec356-long-seg{1..8}.csv` as committed at
  `37f184f0`; no segment was re-sliced, and the slicing is not re-done by this fix.

#### §9.14.3 — WHAT THESE EIGHT NUMBERS ARE, AND WHAT READING THEM AS A VERDICT WOULD COST

**They are R6's reporting obligation and ADJ-3's evidence class — they are NOT a classification input, and
nothing here promotes them to one.**

- **No step reads them.** The determination is `INDETERMINATE` via Step 0 limb (c) (§9.5.1), and Step 0
  precedes every numbered step. Steps 1–4 were never evaluated. A byte-slope is not a term in any of them.
- **No leaf is published off them.** R8.1's Steps 3/4 discriminator is the **pass-rate** fit
  (`slope_per_x_unit`, per epoch), and §9.5.4's bullet already forbids reporting a pass-rate figure as
  `slope_mb_per_hour`. **These eight are the only figures in this manifest entitled to that field name**,
  which is the property the rename exists to make visible — and it is the reason their absence was
  invisible for as long as it was.
- **The n = 1 fragility applies to them in full, and §2.6's caveat is repeated rather than assumed.** The
  `long` cell is a **single unreplicated 4 h series**, and SPEC-355 §10.4.2 measured that the *level*
  replicates (5.4 % spread at width 1000) while the *slope* does not — it moved 2.0×, 4.6× and changed
  sign. **Eight window slopes off one series are therefore a shape, not a measurement of a shape**, and the
  spread visible above (W2 at 197,635 against W8 at 66,993, a 2.95× range within one run) is exactly the
  instability that lesson predicts. **No claim is made about the trend across these eight**, and the §8.1
  repeat at n = 2 is where such a claim could first be attempted.
- **The harness gate they belong to is already attributed, not re-litigated.** `long`'s `exit 1` carries
  `finishedReason` *"tombstone-byte growth slope 76298.1 bytes/h exceeds 512.0 bytes/h"* — the harness's
  own last-half fit over 2,878 samples, a **different reduction** from these eight windowed fits and
  reported here without reconciliation, because §1's frozen text already rules that a red tombstone gate
  is not evidence against `TG-OR-004` and no gate outcome moves on this section.

#### §9.14.4 — THE LATE-DISCHARGE ACCOUNTING, STATED AGAINST THIS SPEC'S OWN RULES

- **PRE-DATA is and remains CLOSED** (§9.3). This section is a **POST-DATA record** in the strict sense
  the §8A preamble defines: it records and routes, and it alters no predicate, threshold, ordering or
  conditional.
- **Nothing frozen moved.** §0–§8 and all twenty §8A addenda are byte-identical to the pin
  `feb85268952001813e502e27f65180855676ac25`; the 1,390-line prefix `cmp`s clean at this fix as it did at
  close. No sidecar (`spec356-prune.sh`, `spec349c2-fit.awk`, `spec356-tmplconf.awk`,
  `spec356-slottruth.sh`, `spec356-skeleton.txt`) is edited. **No re-pin, and no addendum window opened.**
- **The graded checklists are NOT patched.** Checklist 6(a) remains unsatisfiable-when-none-are-quoted and
  7(i) remains vacuous on an absent table; both are recorded as **PD-8** and routed to the repeat's
  pre-declaration (§9.11 row 18), which is a re-pinning spec and the only channel entitled to fix them.
  **Patching them here would be the post-hoc adjustment the pre-registration exists to prevent** — and it
  would also be self-serving, since the item being patched is the one that failed to catch this section's
  own absence.
- **§9.13.1's grader verdicts are unchanged by this section, and were RE-RUN rather than assumed.** The
  three committed sidecars were copied to a scratch `$PWD` and digest-checked first — `tmplconf.awk`
  `7c14b900c4759cec9350aeddf06bcff23289f90425679ac07f6cb3ab75ac58a5`, `slottruth.sh`
  `280f7a3466a0bffc89059815bb3862bb9581cb95d7efa3fe58b1152749b8f060`, `skeleton.txt`
  `bfaac33728dd6974b9922b46b370a256064f0c9d3708dc195c03120e656ba528`, all three reproducing their pinned
  values — and the grader was run in the **mandated `-v ev=` mode** (never `truth=`) against the real
  evidence directory, over the `TG356B-CTRL` block as it stands in the **post-fix** manifest:

  ```
  PASS 15 -- block is the pinned skeleton, byte-exact modulo slots
  FAIL 16 -- slot CELLE_DISPOSITION = 'NOT-FIRED-DETERMINATION-INDETERMINATE' is not well-formed (grammar ENUM_CELLE)
  SKIP 18 -- not attempted: the block is not a well-formed filled skeleton
  SKIP 19 -- not attempted: the block is not a well-formed filled skeleton
  exit=1
  ```

  **Byte-identical to §9.13.1's recorded transcript.** No slot value moves, because §9.C's nineteen slots
  contain no byte-slope, and this fix edited no byte inside the marker pair. **Checklists 16, 18 and 19
  stay RED with their named owners.**
- **THIS SECTION IS NOT A GREEN.** It closes a reporting gap. The classification round's answer is still
  `INDETERMINATE`, cell E is still un-fired, the 2026-07-13 → 2026-07-27 interval is still un-probed, and
  the §8.1 repeat is still **OWED, NOT DONE**.
- **One accounting figure elsewhere was wrong, and this bullet's first draft misdiagnosed HOW — both are
  corrected here, post-review (Review v2).** §9.13.3's owner tally originally read *"Six branches route to
  `TODO-634`, three to `TODO-648`, three to `TODO-638`, two to `TODO-637`"*. This bullet first claimed the
  sentence merely *predated* rows 17 and 18 — i.e. that it was stale by the single `TODO-634` routing row
  18 adds — and on that basis left it standing. **That diagnosis was itself wrong.** Counted against
  §9.11 as it stood at `bf07ae3d` (rows 1–16, before this fix appended rows 17 and 18) the true tally was
  already **`TODO-634` 10, `TODO-648` 2, `TODO-638` 2, `TODO-637` 2** — so the sentence was not stale by
  one routing, it was a substantial miscount of the table it summarised, and row 18 moved only the first
  figure (10 → 11). **The tally is therefore corrected at its own site in §9.13.3, marked post-review,**
  which is the pattern the numstat bullet below states for exactly this case: a false sentence left
  standing with its correction filed elsewhere is the hazard, not the remedy. **What the miscount does NOT
  touch:** every individual §9.11 row's own named owner is and was correct, so no branch was ever
  unrouted or mis-routed — the defect was confined to one summary sentence's arithmetic. No predicate, no
  threshold, no slot, no checklist grade and no verdict reads this figure.
- **§9.13.2's `1071  0` numstat was measured at the close-out commit and is NOT re-measured here.** Its
  own parenthetical already says a section reporting its own file's growth is stale the instant it is
  appended, and that **the ZERO is the load-bearing half**. That zero is a claim about the **frozen prefix
  against the pin**, and it still holds: the prefix `cmp`s clean. **This fix does produce deletions in the
  manifest's whole-file numstat against `bf07ae3d` — exactly SEVEN — and they are named rather than
  glossed:** two lines at §9.3's pending-gate clause, two at its `ctloff` row-count sentence, and three at
  its replicate-map line-count sentence. **Re-measured post-review (Review v2), that figure is now NINE
  against `bf07ae3d`, not seven:** the two added deletions are the §9.13.3 owner-tally bullet corrected
  above. *(The seven is left standing as what it was — the count measured at `c22bc070`, the commit that
  closed Review v1 — rather than overwritten, because a figure scoped to a named commit is not made false
  by a later commit.)* **Against the PIN `feb85268` the whole-file numstat still shows ZERO
  deletions after both fixes** — the insertion half is deliberately not quoted, for the reason this
  bullet's own parenthetical gives below — which is the half the freeze actually rests on: every line
  either fix removed was a line one of these same post-pin appends had introduced, so no byte present at
  the pin has been touched. *(The matching insertion count is deliberately NOT quoted here:
  §9.13.2's parenthetical already records that a section reporting its own file's growth is stale the
  instant it is appended, and this section proved that rule on itself — a first draft of this bullet quoted
  an insertion count that its own later edits invalidated. The DELETION count is the figure the freeze
  rests on, and it is invariant to further appends.)* **All seven are re-flowed prose at the three sites this
  fix corrects, all inside §9 — this spec's own executed record — and not one is a frozen byte.** They are
  corrections made **at their own sites rather than contradicted from 900 lines away**: a false sentence
  left standing with its correction filed elsewhere is the hazard, not the remedy, and each is marked
  *post-review* in place. The distinction that matters is the one §9.13.2 already draws — **append-only is
  a claim about the frozen prefix against the pin, not about the whole file's numstat**, and the prefix is
  untouched.

---

## §10 — SPEC-356c: the §8.1 repeat, at DOUBLED duration AND DOUBLED replicates

**§9 is closed.** It is SPEC-356b's executed record and no byte of it — nor of §0–§8, nor of §8A — is
edited by this section or by anything below it. §10 is **appended**, and the append is checkable:
`git diff --numstat <pin>..HEAD -- '…/spec356-manifest.md'` shows **insertions only, ZERO deletions**, and
the file as it stood at the pin is a **byte-identical PREFIX** of the file as it stands now (AC12,
checklist 3).

### §10.0 — THE PRE-DATA PRE-DECLARATION

**What this subsection is.** §10.0 is written and committed **before the first `spec356c-*.soak.json`
exists** — before any 28,800 s clock starts, before any cell is run, before any repeat datum can be looked
at. It carries the five observation targets and their dispositions, the PRESENCE limb's closed list, R4.6's
100 %-sentinel reporting disposition, §8.2's escalation template, every sidecar digest, R0.5's tracker
ledger and R3.5's dry-run reference set.

**What this subsection is NOT.** **§10.0 records NO measured result from the repeat.** The cells have not
run. Every figure published below was measured either on SPEC-356b's **already-committed** 14,400 s ledger,
on the **committed instrument bytes themselves**, or on the **live tracker files** — and each is labelled
with which. Anything §10.0 asserted about the repeat's own data would be a fabrication, and the reason the
pre-declaration exists at all is that a disposition chosen with the data visible is a disposition the data
chose.

**Nothing here adjudicates a predicate.** There is no `ADJ-21`; §8A gains no byte. R1.3 governs throughout:
the five targets are **OBSERVATIONS, not classification predicates** — they alter no Step, no threshold, no
ordering and no conditional, and **no leaf, no limb, no step and no slot reads any of them.**

---

#### §10.0.1 — THE PRE-DATA LINEAGE, THE DATA BOUNDARY, AND THE PIN RULE

**The instrument lineage, in order.** SPEC-356b closed at the merge `607a3775` (*"docs(sf-356c): commit the
verdict-adjudication artifact (R3.0/P9 obligation) (#145)"*), which is also the commit that landed
`spec356-verdict-xask.md` — the artifact the five targets are taken from (R3.0). **`607a3775` is the
predecessor state of this lineage**: the tip of `main` at the moment the SPEC-356c PRE-DATA branch opened.
Everything below is the **whole** PRE-DATA delta on top of it.

| # | Commit | What it moved | Clause |
|---|---|---|---|
| 0 | `607a3775` | **predecessor** — `spec356-verdict-xask.md`, the verdict artifact carrying the `T1.`…`T5.` anchors | R3.0 / P9 |
| 1 | `4a339066` | the two **additive** runner arms `long8h_r1` / `long8h_r2` in `spec356-prune.sh` — numstat **`14  0`**, identical to the `long` arm in every literal except duration and basename | R2.1 |
| 2 | `4fbe1784` | **`PART V`** appended to **both** Wave-1 gate logs — `spec356a-eager-registration.log` **`230  0`**, `spec356a-step0c-fixture.log` **`106  0`**. The runner moved at hop 1, so the gates are **re-captured, not inherited** | R0.2 |
| 3 | `dd682eee` | `spec356c-slottruth-v2.sh` — the NEW versioned grader sidecar carrying ADJ-12's `B`. v1 is **not edited** | R4.1 |
| 4 | `32f0f713` | `spec356c-slottruth-v1-v2.diff` — the `-U0` diff with **every hunk classified**, the invocation recorded verbatim | R4.2 |
| 5 | `98e2d240` | `spec356c-slottruth-v1-v2-runs.txt` — runs 1 and 2 of the three-run matrix, over committed bytes | R4.3 |
| 6 | `92acd2a7` | `spec356c-targets.sh` — the **one** target-ledger builder for T1…T5 | R3.4 |
| 7 | `1c21b3ed` | the five `spec356c-dryrun-*-long.csv` — the builder **exercised** over the committed 4 h ledger | R3.5 |
| 8 | `b8dabd5b` | `spec356c-trackergrade.sh` + `spec356c-trackergrade.ref` — the tracker-discipline **content** grader and its reference ledger | R0.5 |
| 9 | `830e53a9` | `spec356c-trackergrade-proofs.txt` — the grader proven in **both directions on all five limbs** | R0.5 pt 3 |
| 10 | *this commit* | **§10.0** — the section you are reading | R3 / R6 / R7 |

**THE DATA BOUNDARY, as R0.3 defines it and by no other test.** The boundary is the **first
`spec356c-*.soak.json`**, and it is decided by

```
git log --follow --diff-filter=A --format=%H --reverse -- '…/spec356c-*.soak.json' | head -1
```

— **never by authorship, and never by self-report.** That commit must be a **descendant** of every row in
the table above. **From it onward, a finding is a POST-DATA RECORD routed to a named follow-on.** It is
**never** a predicate edit, **never** an addendum, and **never** a checklist patch — the last of which is
the exact defect PD-8 indicts and this spec exists in order not to repeat.

**THE PIN RULE, stated honestly rather than as a SHA §10.0 cannot know.** R0.1 says B1 resolves
`git rev-parse main` and **checks it against the value recorded in §10.0**. But **§10.0 is itself part of
the PRE-DATA instrument PR**, so the pin — the merge commit of that PR — **does not exist at the moment
these bytes are written**, and any literal SHA recorded here would either be circular or be a SHA of
something that is not the pin. Both are worse than the rule. So the rule is written, and the SHA is
resolved where it can be:

1. **THE PIN IS THE TIP OF `main` AT THE MOMENT THE RUN WORKTREE IS CREATED** — i.e. **the merge commit of
   this PRE-DATA instrument PR**, the commit whose parents are `607a3775` and hop 10 above.
2. **It is resolved in wave 2, as B1's first action, and recorded in §10.1** — as a literal SHA, in the
   §9.1 shape, where it *can* be a reading rather than a copy.
3. **It is stamped into every `matrix.txt`** by the runner's own `  repo HEAD:` line
   (`spec356-prune.sh:865`), so every cell carries it exactly once.
4. **B1 STOPS if `git rev-parse main` has moved past it.** A tip that no longer equals §10.1's recorded
   value means the lineage moved again, and B1 stops **rather than measuring under an unrecorded build**.
   A cell whose `matrix.txt` names a different SHA is **DISCARDED, not footnoted**.

**Why this is the only non-circular reading, and why it keeps R0.1's property.** R0.1's actual property is
*"no cell runs under a build nobody can name"* — it is a property of the **run**, not of the pre-declaration.
Recording the *rule* PRE-DATA and the *SHA* at B1 satisfies it exactly: the SHA is fixed before the first
cell, it is checked against a live `rev-parse` before the first cell, and it is stamped into every cell's
own provenance. What §10.0 supplies is the **predecessor half of the chain** — `607a3775` and the ten hops
above it — so the lineage is checkable **from both ends**: from `607a3775` forward through this table, and
from §10.1's pin backward to `607a3775`. A reader can verify there is nothing in between that this section
did not name.

**"SAME pin" is honoured in SUBSTANCE, and the substance is one command.** §8.1's *"the SAME pin
`feb85268952001813e502e27f65180855676ac25`"* is honoured by the **measured binary being identical**, proven
by an **EMPTY** `git diff --stat feb85268..<pin> -- '*.rs'`. **ZERO `.rs` is a hard constraint of this
whole spec (R0.4)**, and it is what makes the repeat a second observation of the same thing rather than a
first observation of a different one.

---

#### §10.0.2 — THE FIVE TARGET DISPOSITIONS (R3.3)

**Every target has a disposition; none is implicit; none is narrowed silently.**

| Target | Disposition | Output artifact |
|---|---|---|
| **T1(a)** — per-epoch index membership (*"was epoch e's content ever in the drainable index"*) | **OUT-OF-SCOPE** | a published OUT-OF-SCOPE row in §10.3, with the reason and the named owner |
| **T1(b)** — the LWM-pass ledger over the aggregate index gauges | **DERIVED** | `spec356c-t1-lwmpass-r{1,2}.csv` + §10.3 table |
| **T2** — full content enumeration of every non-empty drain, over the **FULL ledger** | **DERIVED** | `spec356c-t2-drains-r{1,2}.csv` + §10.4 table |
| **T2(exactness)** — *"was `bytes_freed` incremented by the EXACT tombstone size"* | **OUT-OF-SCOPE** | a published OUT-OF-SCOPE row in §10.4, with the reason and the named owner |
| **T3** — Δ(LWM) vs Δ(bytes_freed), windowed every 1000 s | **DERIVED** | `spec356c-t3-windows-r{1,2}.csv` + §10.5 table |
| **T4** — non-empty drain rate vs duration | **DERIVED** | `spec356c-t4-rate-r{1,2}.csv` + §10.6 table |
| **T5** — the epoch content fate ledger | **DERIVED**, with **one column OUT-OF-SCOPE, marked in the header** | `spec356c-t5-fate-r{1,2}.csv` + §10.7 table |

**T1(a) — OUT-OF-SCOPE. Reason and owner, stated here so nobody resolves it at the keyboard.** The direct
test *"were epoch e's refs in the index at the moment LWM passed e"* needs a **per-epoch view of the
index**. The committed surface exposes `topgun_or_prune_indexed_refs` and `_indexed_epochs` as **aggregate
gauges over all indexed epochs**; no committed column, and no derivation from committed columns, can
attribute a ref to an epoch. Producing it is a **new per-epoch labelled emission on the prune path — a
`.rs` change — which R0.4 forbids CATEGORICALLY**, because taking it would destroy the frozen-inherited
build identity that is the entire point of the repeat. **Owner: `TODO-634`**, as a named input to the
family's design phase: *per-epoch index residency* is the single highest-value addition the next instrument
round can make, and this row is where that is recorded. **It is NOT narrowed into T1(b), and T1(b) is NOT
presented as its answer.**

**T2(exactness) — OUT-OF-SCOPE. Reason and owner.** **No committed column carries an independent ground
truth for an epoch's true tombstone byte size.** `bytes_freed` and the per-epoch attribution sums are both
emitted by the same prune path, so comparing them tests the path's **internal consistency** and not its
**correctness**; an exactness claim would need an external size oracle, which is again a `.rs` emission.
What T2 *does* derive — `bytes_freed_matches_attribution` — is published as the internal-consistency
identity it is, and is **never** reported as exactness. **Owner: `TODO-634`.**

**T4 — DERIVED, with the discriminant BOUND and the confound NAMED.** T4 publishes several rates, but
**the discriminant is bound to the COORDINATE-LAST-HALF count ONLY**. The whole-run and full-windows counts
are published and are marked `PUBLISHED_NEVER_THE_DISCRIMINANT` in the builder's own `discriminant_role`
column, for a measured reason: **the drain process on the prior arm is front-loaded and decaying** —
**39 of 53** drains before `t = 1,860 s`, **51 of 53** before `t = 5,090 s`, **2** in the last 9,310 s — so
a whole-run count is dominated by a **startup burst** that does not scale with duration. Comparing a
28,800 s whole-run count against a 14,400 s whole-run count would therefore compare two startup bursts and
report the answer as a rate. **That confound is named here, PRE-DATA, so the last-half binding is not a
choice made once the numbers are visible.**

**T5 — DERIVED, with one column OUT-OF-SCOPE and marked as such IN THE HEADER.** The column carries the
marker **verbatim**:

```
indexed_refs_at_lwm_pass__AGGREGATE_NOT_PER_EPOCH
```

The name is the disposition: the value is the **aggregate** gauge read at the moment LWM passed the epoch,
and it is **not** a per-epoch residency answer (that is T1(a), OUT-OF-SCOPE above). **P7 grades this header
verbatim**, so the marker cannot be quietly renamed into something that reads like an answer.

**R1.3, carried in full:** T1…T5 are **OBSERVATIONS**. They alter no Step, no threshold, no ordering, no
conditional. They are reported **alongside** the frozen classification, in their own §10 subsections, and
**no leaf, limb, step or slot reads any of them.** A §10 that routes a determination through a target is
out of contract.

---

#### §10.0.3 — R4.6's 100 %-SENTINEL DISPOSITION, PRE-DECLARED

**The state is LIVE, not hypothetical.** A coordinate last half with **ZERO** non-empty drains renders
**every** row of column 43 as the 0-sentinel, the non-sentinel population is **empty**, and ADJ-12's `B` is
a median over an empty set. The prior arm's own decay (39/53, 51/53, 2-in-9,310 s — §10.0.2) puts a repeat
whose transient has already finished squarely in range: it produces **0**, not 2.

**THE ROUTING IS ADJ-12'S OWN HATCH, UNCHANGED.** ADJ-12 already governs the window: *"if more than 50 % of
the last-half rows are sentinel, `B` is unreliable for that window and the window routes via Step 0(c)
(deciding column unreadable) to INDETERMINATE."* **100 % is inside `>50 %`.** So: `B` uncomputable ⇒ **Step
0 limb (c) fires** ⇒ **`INDETERMINATE`**, and §8.2's escalation (§10.0.5) is published exactly as it would
be at 98.3 %. **No new branch, no third limb, no threshold moved, no `ADJ-21`.**

**What §10.0 pre-declares is the REPORTING disposition, which the hatch does not itself fix. Five points:**

1. **v2 emits `B` EMPTY — never `0`, never coerced.** `0` **is** the sentinel value; emitting it would
   publish the sentinel as the estimate, and a coerced empty is a defect this lineage has already paid for
   once.
2. **`B_SENTINEL_EXCLUDED_PCT = 100.000000`** — ADJ-12 makes the fraction unconditional, and it is at its
   most informative precisely here.
3. **`MEDIAN_L_OVER_B` is WITHHELD WITH THE REASON PRINTED**, verbatim
   `withheld: 0 non-sentinel rows; B undefined` — not silently absent. v1's bare `bb > 0` suppression is
   what made a suppression indistinguishable from a miss (PD-3).
4. **ADJ-3 bundle limb (iii) publishes the literal** `B = UNDEFINED (100 % sentinel, 0 non-sentinel rows)`
   **together with the excluded fraction and the row count it rests on (`0`)**. The bundle is due **whatever
   step fires**, and *"there was nothing to publish"* is not one of its options.
5. **P2's `≥ 1 value row` limb ACCEPTS that literal as its value row.** **An honest 100 %-sentinel replicate
   must NOT be able to RED the PRESENCE limb** — red-on-compliant-text is the failure mode checklist item
   1's converse limb exists to catch, and a presence limb that punishes correct reporting is worse than
   none.

**None of this is licensed to move anything.** The hatch, the `>50 %` literal, the exact-complement
partition and the evaluation order are **untouched**; this clause adds **reporting only**.

**MEASURED, PRE-DATA, on the instrument itself.** The 100 %-sentinel branch is **not reachable on the prior
arm** — the prior arm's coordinate last half measures **98.333333 %**, not 100 %, so run 2 of the three-run
matrix could not exercise it. It was therefore exercised in run 3 **on synthetic fixtures**, and the
transcript (`spec356c-slottruth-v1-v2-runs.txt`) records the branch **behaving exactly as declared above**:
an all-sentinel window emits `B_SENTINEL_EXCLUDED_PCT 100.000000` with `MEDIAN_L_OVER_B` printing
`withheld: 0 non-sentinel rows; B undefined`; a zero-sentinel window emits `0.000000` / `3.000000`; a mixed
window emits `66.666667` / `2.222222`. **The branch is demonstrated, not asserted** — which is the same
standard R0.5 point 3 applies to the tracker grader.

---

#### §10.0.4 — THE PRESENCE LIMB: THE CLOSED LIST, **P1…P13** (R6)

**THE OBLIGATION, QUOTED** (manifest §9.11 row 18): *"THE OBLIGATION ON THE REPEAT: its pre-declaration must
carry a PRESENCE limb — a checklist item that REDs when a required reporting artifact is ABSENT, evaluated
before any reproduce-the-values limb."*

**THE LIMB.** Validation Checklist item 1 is the PRESENCE limb. It is evaluated **FIRST**, before every
reproduce-the-values limb, and **it REDs on ABSENCE**. It is **non-vacuous by construction**: each entry
asserts an **exact-match anchor** (`grep -qxF`, or a `test -f` plus a required header line) **AND** a
**minimum row/line count or an exact recorded value**, never a tolerant pattern. **And it is TWO-SIDED: no
entry may RED on an honest value** (R6.2) — R4.6's empty-population branch is the case that forced the rule
into writing.

**THE LIST IS CLOSED.** An artifact not on it is not required by this limb; an entry on it cannot be
waived. Because it is closed, **an artifact this spec makes load-bearing and leaves OFF the list is ungraded
by construction** — which is why P10…P13 exist at all.

**FIVE ENTRIES BELOW ARE WRITTEN IN A CORRECTED FORM, and each says so in its own text and says why.** They
are P8, P10, P11 and P13 (and P2's empty-population branch, which R4.6 already carries). The corrections are
**instrument-byte corrections made PRE-DATA under R0.3's routing** — they are not spec revisions, and they
land here because §10.0 is the artifact the limb is graded from.

**P1 — the eight-window `tombstone_bytes` fit table, per replicate (16 fits).**
*Assertion:* §10 contains a table whose header line matches **exactly** and which carries **≥ 8 data rows
per replicate**; each row names its window and carries `slope_mb_per_hour`, `se_mb_per_hour`, `r2`.
*Its absence would otherwise be mistaken for:* *"the fits were run and reproduced"* — the exact confusion
PD-7 measured, where the obligation was undischarged **and** undisclosed for a whole merge.

**P2 — the unconditional bundle, per replicate:** ADJ-3's limbs (i)…(iv) **plus** R8.1a's limb (v) (R1.2
item 11).
*Assertion:* five anchored subsection headings present, each carrying **≥ 1 value row**; limb (iii)
additionally carries the excluded fraction. **EMPTY-POPULATION BRANCH (R4.6):** when the non-sentinel
population is empty, limb (iii)'s value row **IS** the literal
`B = UNDEFINED (100 % sentinel, 0 non-sentinel rows)` with the fraction beside it — **that literal SATISFIES
the `≥ 1 value row` limb and MUST NEVER RED it**; the same applies to any other limb whose honest value on
this round is an explicitly-marked UNDEFINED.
*Its absence would otherwise be mistaken for:* *"the step never fired, so the bundle wasn't due"* — but
ADJ-3 makes it due **whatever step fires**; and, in the other direction, an honest 100 %-sentinel replicate
being marked RED **for reporting correctly**.

**P3 — T1: the `t1-lwmpass` table OR its OUT-OF-SCOPE row.**
*Assertion:* either the CSV exists with its exact header and **≥ 1 data row**, **or** §10.3 carries an
OUT-OF-SCOPE row naming the reason **and an owner**.
*Its absence would otherwise be mistaken for:* *"there were no LWM passes"*, which would be a finding,
versus *"nobody built the table"*, which is not.

**P4 — T2: the `t2-drains` table OR its OUT-OF-SCOPE row.**
*Assertion:* as P3; and the table's **`bytes_freed_matches_attribution` column must be PRESENT even when
every drain matches**.
*Its absence would otherwise be mistaken for:* *"every drain was accounted for"* versus *"the identity was
never computed"*.

**P5 — T3: the `t3-windows` table OR its OUT-OF-SCOPE row.**
*Assertion:* **≥ 28 data rows per replicate** (one per 1,000 s window of a 28,800 s run), exact header;
graded as **28 rows with `window_kind=FULL` and AT MOST ONE `PARTIAL`, which must be LAST**.
*Its absence would otherwise be mistaken for:* *"the divergence did not develop"* versus *"the series was
never built"*.

**P6 — T4: the `t4-rate` table OR its OUT-OF-SCOPE row.**
*Assertion:* table present with the **prior-duration arm row** from `spec356-long` explicitly included.
*Its absence would otherwise be mistaken for:* *"the rate could not be compared"* versus *"the comparison
was skipped"*.

**P7 — T5: the `t5-fate` ledger OR its OUT-OF-SCOPE row.**
*Assertion:* **≥ 1 data row per replicate**, and the column header
`indexed_refs_at_lwm_pass__AGGREGATE_NOT_PER_EPOCH` present **VERBATIM**.
*Its absence would otherwise be mistaken for:* *"the fate ledger says nothing"* versus *"the ledger nobody
built says nothing"* — the single most costly absence, since T5 is the target the user ranked first.

**P8 — the v1→v2 grader diff (`spec356c-slottruth-v1-v2.diff`, produced at `diff -U0`) and BOTH run-1 /
run-2 transcripts. WRITTEN IN CORRECTED FORM.**
*Assertion:* file exists; **its header records the `-U0` invocation verbatim**; **≥ 1 hunk**; **every hunk
carries EXACTLY ONE of `class A` / `class B` / `class C`**; an unlabelled hunk, a doubly-labelled hunk, or a
hunk whose label **fails its own mechanical test** ⇒ **RED, with the hunk quoted**; both transcripts present.
The mechanical tests, stated with **`-U0` hunk-header semantics** — a hunk header reads `@@ -a,b +c,d @@`,
where `-a,b` is the **v1** line range (`b = 0` denotes a pure insertion at position `a`, and `,b` is omitted
when `b = 1`):

- **CLASS A — the executable `B`-carrying region. NOW LINE-BOUNDED.** A `class A` label on a hunk whose
  **v1 line range falls OUTSIDE `:258-265`** ⇒ **RED**.
  ***Why this is corrected here.*** As originally drafted, **class A was an UNBOUNDED CATCH-ALL.** Class B
  was bounded by a five-row table of v1 line numbers and class C by a mechanical text test — but class A by
  **nothing at all**, and the checklist's mechanical tests were written for B and C only. The consequence is
  exact and it is fatal to the taxonomy's stated purpose: **any unauthorised executable edit anywhere in v2
  could be labelled `class A` and nothing refuted it** — P8, AC7 and checklist 12 would all pass. The
  taxonomy exists to make *"a fork wearing a version number"* visible; unbounded, it caught a fork only if
  the fork happened to be a comment or one of the five class-B lines. The bound is not invented: the
  class-A region in v1 is exactly `spec356-slottruth.sh:259-264` (`bs[]` population at `:259`, the shared
  sort loop at `:260-261`, `bb` at `:263`, the `bb > 0` suppression at `:264`), so a bound as tight and as
  checkable as class B's **already existed in the bytes** and was simply never written down.
- **CLASS B — one of the five enumerated v1 lines. IN-PLACE ONLY.** A `class B` label on any line outside
  the five-row table is RED — **and so is a class-B hunk that ADDS a line**: class B is defined **BY v1 LINE
  NUMBER**, so a hunk adding a line has **no v1 line** and is **unlabelable**. Mechanically: a `class B`
  hunk **MUST add zero lines** — with `@@ -a,b +c,d @@`, **RED unless `d == b`**.
  ***Why this is stated here.*** Left unsaid, this REDs on **compliant work** (an added `BASE=${2:-…}`
  binding is the obvious shape), and the alternative — a zero-added-line v2 — **is constructible** via
  inline `${2:-…}` defaults. Which of the two is intended is a design choice, and it is resolved **here**
  rather than at the keyboard.
- **CLASS C — comment-only.** A hunk is class C **iff** every one of its `+` and `-` lines, after leading
  whitespace, begins with `#`. **LINE 1, THE SHEBANG, IS EXCLUDED FROM CLASS C: a change to it is RED.**
  ***Why this is corrected here.*** `#!/bin/sh` begins with `#`, so the mechanical test would admit it as
  class C — while R4.2's own prose says class C *"never carries an executable change"*, and an interpreter
  change is the most executable change a shell script has. The spec explicitly privileges the mechanical
  test over purpose, so without this exclusion a grader would **GREEN an interpreter swap**. Latent rather
  than forced (v2's `${2:-…}` defaults are POSIX `sh`), and closed anyway.

*MEASURED on the committed diff, PRE-DATA:* **4 hunks — 1 × class A** (v1 `:259-264`, **inside** the
published bound `:258-265`), **2 × class B** (v1 `:22-25` → `4 → 4`, and v1 `:77` → `1 → 1`; **zero added
lines** on both), **1 × class C** (v1 `:2-3`, every changed line a comment). **v1 `:1` is byte-identical**
and appears in no hunk. **v1's digest reproduces `280f7a34…f060` unmoved.**
*Its absence would otherwise be mistaken for:* *"v2 is v1 with a small fix"* — an unpublished diff makes a
**fork** indistinguishable from a **fix**; and, in the other direction, a two-class taxonomy REDing on the
unavoidable docstring hunk, which is red-on-compliant.

**P9 — `spec356-verdict-xask.md` and the §8.3 quote beside the outcome.**
*Assertion:* the artifact exists with **≥ 1 line matching each of the five target anchors `T1.`…`T5.`**;
§10 carries **§8.3's paragraph as a block quote adjacent to the published determination**.
*Its absence would otherwise be mistaken for:* *"the targets were invented by the executor"*, and *"a Step-5
outcome is a blocked one"* — §8.3's own last sentence requires the quote precisely so that reading cannot
survive.

**P10 — the target builder's PRE-DATA DRY-RUN set (R3.5): the five `spec356c-dryrun-*-long.csv`. WRITTEN IN
CORRECTED FORM.**
*Assertion:* all five exist with their exact headers, **and each file's data-row count falls inside its
pre-declared BAND**:

| File | Graded bound | Kind |
|---|---|---|
| `spec356c-dryrun-t3-windows-long.csv` | **EXACT 14 `FULL` + 1 `PARTIAL`, graded separately** | exact |
| `spec356c-dryrun-t4-rate-long.csv` | **EXACT 53 whole-run / 2 coordinate-last-half** | exact |
| `spec356c-dryrun-t1-lwmpass-long.csv` | **≥ 200 rows** | range |
| `spec356c-dryrun-t5-fate-long.csv` | **≥ 200 rows** | range |
| `spec356c-dryrun-t2-drains-long.csv` | **≥ 40 rows** | range |

**A count outside its band is RED in G1** — which is where R3.5 already says a builder defect is fixed.
***Why this is corrected here.*** As originally drafted, P10 asserted exact shapes for `t3-windows` and
`t4-rate` but for the other three asked only that *"each file's row count is **published** in §10.0 beside
the shape R3.5 pre-declared"*. **Publishing a number beside a different number is not an assertion that the
two agree.** A T1(b) builder that reads the wrong window and emits **5** rows instead of ~232 satisfied the
drafted form **in full**: the file exists, its header is exact, and its row count is published. That
reproduces the *"a wrong builder reproduces its wrong table byte-for-byte and grades GREEN"* defect
**inside the machinery that defect produced**, and it violates R6.2's own construction rule, which requires
*"an exact-match anchor **and** a minimum row/line count or an exact recorded value, never a tolerant
pattern"* — the drafted form supplied neither a minimum nor an exact value for those three. The bands above
are deliberately **far below** the measured values, so they cannot be tuned to the answer.
*MEASURED on the dry run:* **14 `FULL` + 1 `PARTIAL`; 53 / 2; 232; 233; 53 — EVERY BOUND MET.** (Full table
in §10.0.8.)
*Its absence would otherwise be mistaken for:* *"the builder was validated"* — a builder committed but never
run is exactly the blind check Validation Checklist item 13 cannot catch.

**P11 — BOTH `PART V` gate re-captures (R0.2), in the PART IV shape. WRITTEN IN CORRECTED FORM.**
*Assertion, `spec356a-eager-registration.log`:* the `PART V` block exists and carries **ALL FOUR named
blocks** by exact anchor —
**(a)** the pinned-name registration `limb (a): 33/33`;
**(b)** the zero-empty-field census, anchored on **`empty_fields=0`** over the 43 columns;
**(c)** the **arming witness as its own block** — `41/41` armed **and** `0/41` disarmed, **both** printing
`arming witness PASSED`;
**(d)** the **ADJ-20 artifact-binding block**, with an independent `shasum -a 256 -c` reported **`N/N OK`**.
*Assertion, `spec356a-step0c-fixture.log`:* the guard **fired**; `sort -u` over **all five** captures yields
**ONE** `STEP0C ADMISSIBILITY` line; instrument-defect literal count **0**; exit **0**.

***Why this is corrected here — two residuals, both red-on-compliant or wrong-shape.***
**(i) The limb-(b) anchor is `empty_fields=0`, WITHOUT SPACES.** The drafted anchor was written
`empty_fields = 0` with spaces, but the gate log emits the unspaced form at
`spec356a-eager-registration.log:293`, `:329`, `:531`, `:558`, `:794`, `:920`. P11 is graded *"by exact
anchor"*, so the spaced form would make a **correctly written** PART V fail **its own anchor** — precisely
the red-on-compliant failure mode the limb's converse exists to catch.
**(ii) The target shape is PART IV's, not "PART III / PART IV".** **PART III carries only THREE blocks** —
its verdict at `:788-798` has limb (a), limb (b) and the witness, and **no ADJ-20 binding block, because
ADJ-20 did not exist at that hop**. **PART IV is the four-block shape** (`:916-925`). Naming both as the
target would have licensed a three-block PART V.

*MEASURED in the `PART V` just captured (`4fbe1784`), on `spec356a-eager-registration.log`:*
**limb (a): `33/33`** pinned names in the first render — the `.rs`-side registration set, and **not** the
`41/41` figure;
**limb (b): `empty_fields=0` over 43 columns × 12 rows**;
**arming witness: armed `41/41` PASSED; disarmed direction `0/41` PASSED**;
**ADJ-20 binding: 6 artifacts digested; independent `shasum -a 256 -c` → `6/6 OK`.**
*Why `N` is **6** here and **7** in a measurement cell, recorded so the difference is not read as a defect:*
a **120 s smoke cell writes no `progress.jsonl`**, so its binding manifest names six artifacts, whereas a
measurement cell digests **seven**. **P11 therefore grades a generic `N/N OK` rather than a hardcoded
count** — a hardcoded `6/6` would RED on every measurement cell, and a hardcoded `7/7` on every gate.
*MEASURED on `spec356a-step0c-fixture.log`:* guard **fired**, 1 emitted line; `sort -u` over all **five**
captures yields **ONE** distinct line; instrument-defect literal count **0**; exit **0**.
*Its absence would otherwise be mistaken for:* *"the gate was re-captured"* — a PART V that reports the
arming witness twice and drops the 33/33 name-registration limb looks **identical** to a correct one unless
the four blocks are asserted separately.

**P12 — `spec356c-scratch-map.txt` (R2.3 point 5).**
*Assertion:* file exists; **≥ 2 lines**, one per replicate, each of the exact shape
`<scratch dir> → <evidence dir>` with the **basename identical on both sides**; and **each committed cell's
scratch dir matches the `data dir:` / `prune.csv:` paths inside that cell's own `matrix.txt`**.
*Its absence would otherwise be mistaken for:* *"the provenance residual is explained"* — R2.3 makes this
file the **SOLE** explanation of the surviving residual and says an unmapped replicate is **re-run, not
annotated**, so an absent map silently converts a re-run obligation into a footnote.

**P13 — the tracker-discipline ledger and its discrimination transcripts (R0.5). WRITTEN IN CORRECTED
FORM — FIVE LIMBS, FIVE TRANSCRIPTS.**
*Assertion:* §10.0 carries the **four `shasum -a 256` values**, the §8.1 box's **verbatim pre-text** and the
tick counts **2 / 5** (§10.0.7); §10.x carries the four **post**-digests, the box's **post**-text, the post
tick counts, **and all FIVE PRE-DATA discrimination transcripts** plus wave 5's own real run. The graded
limbs are:

| Limb | What it asserts |
|---|---|
| **(a)** | the three unrelated digests (`TODO-637`, `TODO-638`, `TODO-648`) are **byte-identical** to §10.0.7's |
| **(b)** | `TODO-634.md`'s digest **HAS MOVED** — an unchanged digest means the required edit never happened and is **RED**, not a pass |
| **(c)** | the tick counts are **still 2 and 5** — the §8.1 box is **updated and NOT ticked** |
| **(d)** | the §8.1 box's before/after text is published side by side, and the **after-text contains the outcome and a `spec356-manifest.md §10` pointer** |
| **(e)** | **THE DELTA IS CONSTRAINED** — `diff` the pre-edit fixture against the edited file and assert **every changed line falls inside the §8.1 box** |

***Why limb (e) was added.*** **AC15 requires *"every other box in that file is unchanged"*, and limbs
(a)–(d) cannot express it.** (a) reads **other files**. (b) asserts only that the digest **moved** — any edit
whatsoever satisfies that. (c) **counts ticks**, so it catches a tick elsewhere but not a **text** change.
(d) inspects the **§8.1 box only**. So an executor who updated the §8.1 box correctly **and also rewrote an
unrelated paragraph** passed all four limbs while violating AC15's own words. Limb (e) is the limb that
reads the delta.

***Why the transcript count went from two to five.*** R0.5's drafted proof ran the grader once against an
unedited fixture and once against the edited file, and the RED run demonstrates RED on limbs **(b)** and
**(d)** only. That leaves **(a)** and **(c)** demonstrated **green-on-compliant and never RED on a genuinely
broken input** — which is exactly what R0.5's own closing sentence refuses: *"a discrimination that is
asserted rather than demonstrated is the thing this clause exists to refuse."*

*THE FIVE PRE-DATA PROOFS — ALL OBSERVED MATCHING THEIR PRE-DECLARED VERDICTS* — committed in
`spec356c-trackergrade-proofs.txt`:

| # | Input | Pre-declared verdict | Observed |
|---|---|---|---|
| 1 | unedited `TODO-634.md` (provenanced fixture) | **RED (b), (d)** | `VERDICT: RED (red limbs: b d)` |
| 2 | §8.1 box **ticked** | **RED (c)** — **and (d)**, a true finding about that input, **reported as observed** rather than suppressed to match a prediction | `VERDICT: RED (red limbs: c d)` |
| 3 | `TODO-637.md` one byte changed | **RED (a)**, naming the file | `VERDICT: RED (red limbs: a)` |
| 4 | correct §8.1 edit **plus** an unrelated paragraph rewritten | **RED (e)**, with **(a)–(d) all GREEN** | `VERDICT: RED (red limbs: e)` |
| 5 | synthetic correctly-edited copy | **GREEN on all five** | GREEN, all five limbs |

**Input 4 is the whole argument for limb (e)**: four green limbs and an AC15 violation, in one input.
**The SIXTH transcript — the real GREEN run against the actually-edited `TODO-634.md` — is WAVE 5's and is
graded there**, not here; §10.0 cannot hold it, because the edit it grades has not been made.
*Its absence would otherwise be mistaken for:* *"tracker discipline was checked"* — the **git-diff** form of
this check is **vacuously GREEN for every possible execution** (`.specflow/` is gitignored at
`.gitignore:111` and no path under it is in the index), so without the content ledger **AC15 is ungraded in
fact**.

**R6.4 — THE VERDICT IS PUBLISHED WHATEVER IT IS.** The PRESENCE limb's own verdict — **GREEN or RED, PER
ENTRY** — is published in **§10.2**, **BEFORE** any reproduce-the-values result. **A RED entry is not a
reason to delay publication — IT IS THE FINDING**, and it names the missing artifact. A limb that can only
be published when it is green is the vacuous guard PD-8 indicts, wearing a different hat.

---

#### §10.0.5 — §8.2's ESCALATION, PRE-DECLARED (R7)

**Why it is pre-declared.** R1.4 pre-declares the expectation that Step 0 limb (c) fires again and the
determination is `INDETERMINATE`. §8.2 then obliges an escalation that is **"not 'record a note'"**. Writing
that escalation **after** the data lands is writing it to taste. So its form is fixed **now**, and wave 5
fills it.

**IF THE OUTCOME IS `INDETERMINATE`, §10 PUBLISHES ALL SIX OF THESE, IN THIS ORDER:**

1. **THE UNCLASSIFIED CAUSE, NAMED** — the name is the object §8.2 asks for:
   > *the mechanism by which the OR tombstone prune falls behind at `TOPGUN_EPOCH_WIDTH=1000` remains
   > unclassified among SELECTION/FRONTIER, SCHEDULING/LICENSING and THROUGHPUT, and a mechanism outside
   > those four is not excluded* — **ADJ-4's bound, quoted, not paraphrased.**
2. **EVERY STEP'S EVALUATED VALUE, PER REPLICATE, IN THE FROZEN ORDER** — Step 0's four limbs with their
   numbers, then Steps 1, 2, 3, 4, 5, **each** either with its evaluated value **or** marked verbatim
   **`NOT EVALUATED — Step 0 is fail-closed and precedes every numbered step`**. **A step left off the table
   is the omission §8.2 exists to prevent.**
3. **THE BLOCKING ADMISSIBILITY LIMB, NAMED BY LETTER**, with its evaluated value against its **frozen**
   threshold — e.g. *limb (c): sentinel fraction `X / Y = Z %` against the pre-registered `50 %`*.
4. **PD-4'S COUPLING, RESTATED ON THIS ROUND'S OWN NUMBERS** — **the exit-share denominator beside the
   percentage.** The two statistics degrade together and **neither is quoted without the other**.
5. **§8.3, QUOTED VERBATIM AS A BLOCK QUOTE, ADJACENT TO THE DETERMINATION.** §8.3's own last sentence
   requires it of any Step-5 outcome, and **P9 checks it**. The block to quote is manifest §8.3 in full —
   *"The recommended reclamation model closes safety REGARDLESS of which cause it turns out to be … A
   selection defect, a scheduling defect and a throughput defect are all contained by a registry that never
   reclaims below a live claim. … A Step-5 outcome is therefore to be read as an expensive answer, not a
   blocked one"* — reproduced **verbatim**, not summarised as it is here.
6. **THE ROUTING SENTENCE:** the unclassified cause is carried into **`TODO-634`'s design phase as a STATED
   OPEN INPUT**, together with **T1…T5's tables and T1(a)'s OUT-OF-SCOPE row**. That is what turns *"we
   still don't know"* into a **design input** rather than a dead end.

**R7.3 — THE FOUR PROHIBITIONS. The escalation does NOT:**

1. **block the family** (§8.3);
2. **propose a fix shape**;
3. **endorse any of the four mechanisms**;
4. **re-run the cell hoping for a different window**, and it **adjusts nothing**.

**What an unclassified cause costs is fix-shape efficiency, not safety.**

**IF THE OUTCOME IS NOT `INDETERMINATE` (R7.4).** Then the walk continued into Steps 1–4 and one leaf
fired. §10 publishes:

- the leaf **with ADJ-4's wording bound** — *"the best-supported of the FOUR PRE-REGISTERED mechanisms"*,
  **never** *"the cause"*;
- the **n = 2 replication statement** (R2.4), including **whether both replicates produced the same leaf**;
- if they did not, **the disagreement AS THE HEADLINE**, not as a footnote — and **a split determination
  across two replicates is reported as `INDETERMINATE` on the replication axis and escalated under the six
  items above**, because §8.1 doubled replicates precisely to expose it;
- **AND — ADJ-3's `CONTESTED` DISPOSITION, NAMED HERE SO THE EXECUTOR IS NOT DECIDING IT LIVE.** Frozen
  §8A ADJ-3 closes with:
  > **A Step-1 or Step-2 determination that coincides with a significantly NEGATIVE passes-per-epoch slope
  > is reported as CONTESTED**, naming the feedback alternative explicitly. CONTESTED does not change the
  > determination; it forbids reporting it as uncontested.

  R7.4's enumeration names ADJ-4's wording bound, the n = 2 statement and the split-determination route —
  **but not CONTESTED**, and a closed enumeration that omits an obligation is the shape this lineage has
  already been bitten by. **Two things are true and both are recorded.** *(i)* **Reachability is low:** it
  needs the **~30× rise in drain rate** R1.4 pre-declares against (≈ 120 non-empty drains in the coordinate
  last half, against the 2 measured), **and** a Step-1/Step-2 leaf, **and** a significantly negative
  passes-per-epoch slope. *(ii)* **NO LICENCE IS CREATED BY THE SILENCE:** R1.2 item 11 binds **ADJ-3
  whole**, so the obligation survives R7.4's enumeration regardless of what that enumeration lists. This
  clause therefore **adds no rule** — it makes an existing one visible at the point of use. **If a Step-1
  or Step-2 leaf fires on a replicate whose passes-per-epoch slope is significantly negative, §10 labels
  that determination `CONTESTED`, names the feedback alternative explicitly, and does NOT report it as
  uncontested.** CONTESTED **changes zero routing decisions**.

---

#### §10.0.6 — EVERY SIDECAR DIGEST, `shasum -a 256`, COMPUTED AT THESE BYTES

**These are the graders and builders the executed record is produced by, pinned before the data so that a
derivation chosen with the data visible is not available.**

| File | `shasum -a 256` |
|---|---|
| `spec356c-slottruth-v2.sh` | `a95f49fa99f7f39ad777b0789ec778eda1ffec3aa3221031d98b587b62a4f348` |
| `spec356c-slottruth-v1-v2.diff` | `0101a7d676b5ab4ee35e017a72e8bf92a4e8aaf513a846da24f3facf512fc42e` |
| `spec356c-slottruth-v1-v2-runs.txt` | `4790f0cf80d820d9f7fd42d081ce17c771ea6589020d132dfe24bdcaa9973d6f` |
| `spec356c-targets.sh` | `2301ca20fec26400826d3d10444b2a7fca773339d3266e29b04092d0ad33cd08` |
| `spec356c-dryrun-t1-lwmpass-long.csv` | `51e1466195ebcfb272c62cc568ea833f32fa82fd3cd21f3f255a829ad666289e` |
| `spec356c-dryrun-t2-drains-long.csv` | `24c0ae51a772770555497a65d351dc68de0fcf2088e71595a101f31f9dcecca3` |
| `spec356c-dryrun-t3-windows-long.csv` | `0bbb9cae9e0e5708ebeadd999a03fcffa877a72f7638b940189e6a6773b9abe4` |
| `spec356c-dryrun-t4-rate-long.csv` | `8f76b89cf2ae77b7fa21ba098d41c3a2291d73f01564e31e08d032a7d0227a88` |
| `spec356c-dryrun-t5-fate-long.csv` | `d0095b01107636b087490075e88b629b5c10b24cf0ceac3c1f48ff0266685ad9` |
| `spec356c-trackergrade.sh` | `879dadb1c89eea9fdcb6a1b90292e58de50cb9e5bc00c64d7d40d92c8c6c42db` |
| `spec356c-trackergrade.ref` | `e3dcd977d7a8c538245a376a019568fe635ba04be6d81be330b256f3dfb23577` |
| `spec356c-trackergrade-proofs.txt` | `6e361bc16ea102daaf1b158d38361a7da001b8bc2f97de7a9e3335b8055ea563` |

**AND THE TWO INHERITED SIDECARS, WHOSE DIGESTS MUST NOT MOVE:**

| File | `shasum -a 256` | Disposition |
|---|---|---|
| `spec356-slottruth.sh` (**v1**) | `280f7a3466a0bffc89059815bb3862bb9581cb95d7efa3fe58b1152749b8f060` | **UNMOVED.** v1 is **not edited** — v2 is a new file (R4.1). Editing v1 would edit a pinned sidecar mid-lineage, which §9.11 rows 5 and 7 record as forbidden and route to `TODO-648` |
| `spec349c2-fit.awk` | `840813461e3b1bd5c3a79291044d8ac515e09b94333ee530cd6a10de8fa0436f` | **UNMOVED.** The fitter is invoked **UNFORKED** — the eight-window fits of P1 are produced by these exact bytes, not by a variant |

---

#### §10.0.7 — R0.5's TRACKER-DISCIPLINE LEDGER (the PRE side)

**Why a content ledger and not a `git diff`.** `.specflow/` is **gitignored at `.gitignore:111`** and **no
path under it is in the index** — searching the repository's own `.git/index` for the string `specflow`
returns **zero** occurrences, while the control string `spec356-long.prune.csv` returns one. So
`git diff <pin>..HEAD -- '.specflow/todos/'` is **EMPTY for every possible execution**, and any checklist
item resting on it is **vacuously GREEN** — PD-8's own defect, reproduced inside the checklist authored to
replace it. **Tracking `.specflow/` in git is barred by standing project policy and is NOT the remedy.** The
remedy is to **read the files and assert their content**, which is what this ledger is. **These are
measurements, taken at these bytes.**

| Tracker file | `shasum -a 256` (PRE) |
|---|---|
| `.specflow/todos/TODO-634.md` | `555c3aa92a569c030a7fbacfafdee5933bf4beda668d79fff126f1a2c0e74f8c` |
| `.specflow/todos/TODO-637.md` | `48a16fbdfa923770b673882ae58b7a1aa5f15e5a1d3b8a1ee36ce3142b02940f` |
| `.specflow/todos/TODO-638.md` | `756082197b6efd7b9158fea53c859724f78c82d5a2e3684eaa38e206571825df` |
| `.specflow/todos/TODO-648.md` | `56fb3f59f7159b3acd0afa3a66f04df74e3bf9096614ae5357940d149847ef2c` |

**TICK COUNTS AND THE CENSUS THAT MAKES THEM EXHAUSTIVE, on `TODO-634.md`:**

| Quantity | Command | Value |
|---|---|---|
| ticked boxes | `grep -c '^- \[x\] '` | **2** |
| unticked boxes | `grep -c '^- \[ \] '` | **5** |
| top-level boxes (census) | — | **7** |
| indented boxes (census) | — | **0** |

**The census is the point:** the file carries **7** top-level boxes and **NO** indented ones, so **ticked +
unticked = 2 + 5 = 7 covers every box the file has** and the two counts are **exhaustive**. Without the
census, a count of 2 and 5 would be consistent with a file that also had indented boxes nobody was reading.

**THE §8.1 REPEAT BOX — VERBATIM BEFORE-TEXT** (`.specflow/todos/TODO-634.md:213-233`, transcribed from the
live file at these bytes). §10.x publishes the **after**-text beside it:

```
- [ ] **NEW, CARRIED IN FROM SPEC-356b — the §8.1 REPEAT: `OWED, NOT DONE`.**
      **CARVED TO `SPEC-356c` (2026-08-11, `/sf:plan`) — carved, NOT done and NOT ticked.** That spec
      authors ONLY this repeat increment (the `long` cell at 28,800 s × n = 2 under the frozen
      predicate), plus PD-8's PRESENCE limb, the v2 grader sidecar carrying TODO-648's `B`-sentinel
      fix, the stated worktree procedure, and the FIVE pre-declared observation targets that hunt the
      **fifth mechanism** (the LWM advancing past never-freed content). **The registry family is NOT in
      its scope**, and this box ticks only when SPEC-356c is done. Exactly **one** repeat of
      the deciding configuration at **DOUBLED duration AND DOUBLED replicates** — the `long` cell at
      **28,800 s with n = 2** — under the **SAME pin** `feb85268952001813e502e27f65180855676ac25` and the
      **SAME frozen predicate** (manifest §0–§8 plus §8A's twenty addenda). **It may not adjust a
      threshold, an ordering or a conditional**: it is a second observation, not a re-specification.
      Both axes are doubled for different reasons — duration for an effect too slow to separate in 4 h,
      replicates for the n = 1 fragility SPEC-355 §10.4.2 measured — and **one axis alone leaves the
      other cause unaddressed.** **If the repeat is STILL INDETERMINATE, §8.2 escalates** the
      unclassified cause as an explicit input to this item's design phase, naming it and quoting every
      step's evaluated value and the admissibility limb that blocked.
      **This does NOT block the family.** Manifest §8.3, verbatim: *"The recommended reclamation model
      closes safety REGARDLESS of which cause it turns out to be … A selection defect, a scheduling
      defect and a throughput defect are all contained by a registry that never reclaims below a live
      claim."* What an unclassified cause costs is **fix-shape efficiency, not safety** — an expensive
      answer, not a blocked one.
```

**`spec356c-trackergrade.ref` IS THE PINNABLE FORM OF THIS LEDGER.** The grader reads its reference values
from that committed file rather than from an invocation line, because *a reference typed at the keyboard
makes the grade as strong as the operator's memory, which is an assertion and not a measurement*. **The
manifest and the `.ref` file must not be allowed to disagree**: every digest, both tick counts and both
census figures above appear in `spec356c-trackergrade.ref` with the same values, the `.ref` file's own
digest is pinned in §10.0.6, and **a disagreement between the two is itself a defect to be resolved before
wave 5 runs** — not a discrepancy to be explained afterwards.

**WAVE 5'S CONSTRAINT ON THE REAL EDIT, stated PRE-DATA so it is not decided at the keyboard.** The edit to
`.specflow/todos/TODO-634.md`:

1. **puts the repeat's OUTCOME and the `spec356-manifest.md §10` pointer INSIDE the §8.1 box, as
   continuation lines** — appended to the box's own indented body, not placed after it;
2. **touches NO other line of the file** — every changed line must fall inside the box (limb (e));
3. **does NOT tick the box** — the tick counts must still read **2 / 5** (limb (c)). The box ticks only when
   `SPEC-356c` is done, and publishing the repeat's outcome is not the same act as closing `TODO-634`.

**Two limits of the instrument, restated here so they are on the manifest's record and not only in the
script header.** *(1)* Limb (d)'s outcome-token sub-check is satisfied by the **PRE**-text as well — the
unedited box already contains the token `INDETERMINATE`, inside *"If the repeat is STILL INDETERMINATE"*.
Proof input 1 shows this directly: the token is **PRESENT** and (d) still **REDs**, on the missing pointer
alone. The grader therefore reports, **as evidence and not as a verdict term**, whether each literal was
already in the before-text, and **a reader should look at that line before believing a green (d)**. *(2)*
Limb (e) is **vacuously green on an empty diff** — proof input 1 shows 0 hunks and (e) GREEN. That is
deliberate: *"nothing happened"* is **limb (b)'s** finding, and input 1's (b) REDs on it. **(e) constrains a
delta that exists; it does not assert that one does.**

---

#### §10.0.8 — THE PRE-DATA DRY-RUN REFERENCE SET (R3.5)

**A builder committed before the data but never RUN before the data buys nothing.** Validation Checklist
item 13 asks only that the ten target CSVs be **REPRODUCIBLE** from the committed builder — and a builder
with the wrong window reproduces its wrong table **byte-for-byte** and grades GREEN. So `spec356c-targets.sh`
was run, **in G1, before any 28,800 s clock started**, over SPEC-356b's **committed, unmodified**
`spec356-long.prune.csv` (14,400 s, 43 columns, 1,440 rows) — the only real 43-column ledger that existed at
that moment — and its five outputs are committed.

| Dry-run file | Shape R3.5 pre-declared | **MEASURED** | P10 bound | Verdict |
|---|---|---|---|---|
| `spec356c-dryrun-t3-windows-long.csv` | 14 `FULL` + 1 `PARTIAL` (R3's tail rule at 14,400 s) | **14 `FULL` + 1 `PARTIAL`** (15 data rows) | EXACT, graded separately | **MET** |
| `spec356c-dryrun-t4-rate-long.csv` | two counts reading 53 whole-run / 2 coordinate-last-half | **53 whole-run / 2 coordinate-last-half** (20 data rows) | EXACT | **MET** |
| `spec356c-dryrun-t1-lwmpass-long.csv` | ~232 LWM-pass rows | **232** | ≥ 200 | **MET** |
| `spec356c-dryrun-t5-fate-long.csv` | ~233 epoch rows | **233** | ≥ 200 | **MET** |
| `spec356c-dryrun-t2-drains-long.csv` | the whole-run drain rows of the prior arm, on the order of 53 | **53** | ≥ 40 | **MET** |

*(Row counts are DATA rows: non-`#` lines minus the header line. Each file carries a `#` provenance preamble
that states, in the file itself, that it is reference and not evidence.)*

**T2's window, T3's origin and tail rule, and T4's bound count were each exercised against real bytes here.
There is no path on which they are settled with the repeat's data visible** — which is the whole reason
R3.5 is a gate in G1 rather than an option.

**THESE CSVs ARE REFERENCE, NEVER EVIDENCE.** Three independent reasons, and each is checkable:

1. **they carry the prior arm's basename in their own filename** (`…-long.csv`, not `…-r1.csv` / `…-r2.csv`);
2. **they are produced from an ALREADY-COMMITTED ledger** — SPEC-356b's `spec356-long.prune.csv`, which
   predates this spec entirely;
3. **NO §10 classification limb, step, slot or target table reads them.** Not one. They exist to prove the
   builder was exercised over real bytes before the 16 h of wall-clock was spent, and for nothing else.

---

#### §10.0.9 — FOUR MEASURED CORRECTIONS FOUND IN G1, RECORDED NOW RATHER THAN DISCOVERED LATER

**These are PRE-DATA findings about the INSTRUMENT and about the PRIOR ARM.** None is a finding about the
repeat, which has not run. Each corrects or completes a claim made in this spec's own prose, and each is
recorded here — at authoring time, inside the pre-declaration — rather than surfacing in wave 5 where it
would be a POST-DATA record about a claim that was already load-bearing.

**(1) THE `current_epoch ≡ low_water_mark` IDENTITY IS NEAR-TOTAL, NOT TOTAL.** R3's prose says the two
gauges are *"numerically identical at every inspected row"* — a statement resting on **spot checks**.
**Measured over the WHOLE committed prior-arm ledger:**

| Regime | Rows where `topgun_or_prune_current_epoch == topgun_or_prune_low_water_mark` | Rate |
|---|---|---|
| full ledger | **1,403 / 1,440** | **0.974306** |
| coordinate last half (`elapsed_secs ≥ 7,210`) | **703 / 720** | **0.976389** |

So **T1(b)'s rows are a NEAR-projection of T5's, not an exact one**, and a derivation that treats the two
gauges as interchangeable is wrong on roughly **1 row in 40**. **Consequence, binding on the later
subsections: §10.3 and §10.7 must publish the identity rate PER REPLICATE (AC8) and must state WHICH REGIME
is being read** — full ledger or coordinate last half — beside any table that leans on the correspondence.
The correction is recorded here so that the rate is a **published quantity** rather than an assumption
inherited from prose.

**(2) `MEDIAN_L_OVER_B` ON THE PRIOR ARM IS `0.000000`, AND THE CAUSE IS THE NUMERATOR, NOT `B`.** Run 2 of
the three-run matrix reproduced ADJ-12's figures **exactly** — `B = 999.944741` and
`B_SENTINEL_EXCLUDED_PCT = 98.333333` — and yet the ratio prints `0.000000`. The reason is that
**`median(L) = 0`**: **403 of the 720 coordinate-last-half rows carry `topgun_or_prune_eligible_refs = 0`**,
a clear majority, so the median of `L` over all last-half rows is zero. That is **v1's own untouched ADJ-2
aggregator over ALL rows** doing exactly what it is specified to do, so **it is a fact about the ledger, not
a defect of v2.**

**What v2 actually bought, stated explicitly so a reader does not mistake `0.000000` for a failed fix:**
under v1 the ratio was **structurally WITHHELD** — the bare `bb > 0` suppression made a suppression
indistinguishable from a miss (PD-3), and no reader could tell whether the quantity was zero, undefined or
simply never computed. Under v2 the ratio is **EMITTED AND THEREFORE GRADEABLE**: `0.000000` is a value a
reader can check, argue with and reproduce. **The fix was to the reporting channel, and the channel now
carries an honest zero.**

**(3) ALL 53 DRAINS ON THE PRIOR ARM REPORT `drain_epochs = 1`.** Every one of the 53 non-empty-drain rows
in the dry-run T2 ledger carries `d_drain_epochs_sum / d_drain_epochs_count = 1/1`. **Consequence: T5's
stated contiguity inference — that when `k > 1`, the drained set is the `k` epochs ending at
`last_drained_epoch` — was NEVER EXERCISED on the prior arm**, so on that subject the drained set is
**exact** and rests on no inference at all. The inference remains pre-declared for the repeat, where a
longer run may produce `k > 1`, and it is recorded here that it is **an untested branch of the derivation**
rather than a validated one. Also measured: **`bytes_freed_matches_attribution` is `true` on all 53 rows**,
and — per P4 — **the column is present regardless of whether every drain matches**, because *"every drain
was accounted for"* and *"the identity was never computed"* must not look the same.

**(4) THE GATE'S ADJ-20 BINDING IS `6/6`; A MEASUREMENT CELL'S IS `7/7`.** A **120 s smoke cell writes no
`progress.jsonl`**, so the artifact-binding manifest of a gate capture names **six** artifacts, while a
measurement cell digests **seven**. Both report an independent `shasum -a 256 -c` clean. **Consequence: P11
grades a generic `N/N OK`, never a hardcoded count** — a hardcoded `6/6` would RED on every measurement
cell, and a hardcoded `7/7` on every gate re-capture. This is recorded as a measurement rather than left as
a convention, because the difference between the two numbers is exactly the kind of residual that gets read
as a defect the first time somebody notices it without the explanation.

---

### §10.1 — THE EXECUTED RECORD'S LINEAGE: THE PIN, THE RUN TREE, THE TWO CELLS, THE DATA BOUNDARY

**What this subsection is.** §10.0 was the pre-declaration. §10.1 is the **lineage of the record that was
then executed against it** — the pin B1 resolved, the four checks B1 ran before any clock started, the run
worktree the cells ran from, the two cells' own provenance, the boundary that separates instrument from
data, and the sixteen `tombstone_bytes` fits. Every figure below is a **reading**, and every reading names
the command that produces it, so a reader with the repository can reproduce the section rather than trust
it.

**What this subsection is NOT.** It adjudicates nothing. No predicate, threshold, ordering or conditional
is touched here or anywhere below; §8A gains no byte and there is no `ADJ-21`.

---

#### §10.1.1 — THE RESOLVED PIN, AND B1'S FOUR CHECKS WITH THEIR REAL OUTPUTS

**THE PIN, resolved in wave 2 as B1's first action, in the §9.1 shape:**

```
8a60f1357f4702f4c5393538ef6b4ae6cebd247f
  subject: feat(sf-356c): the PRE-DATA instrument PR for the §8.1 repeat (G1) (#146)
  parent:  607a37759ac921f04aac2aee7487b08f6d19cb79   (§10.0.1's declared predecessor)
  SPEC-356b's pin (§8.1's literal): feb85268952001813e502e27f65180855676ac25
```

**CHECK 1 — TIP.** `git rev-parse main` must equal the pin, or B1 stops rather than measuring under an
unrecorded build.

```
$ git rev-parse main
8a60f1357f4702f4c5393538ef6b4ae6cebd247f
$ git rev-parse --abbrev-ref HEAD
main
$ git status --porcelain | head        <- EMPTY (0 lines)
```

**PASS.** The lineage had not moved between §10.0's merge and B1; B1 did not stop.

**CHECK 2 — EMPTY `.rs` DIFF.** This is what makes *"the SAME pin"* true **in substance**: the measured
binary is identical to SPEC-356b's.

```
$ git diff --stat feb85268..8a60f135 -- '*.rs'          <- no output
$ git diff --stat feb85268..8a60f135 -- '*.rs' | wc -c
       0
```

**PASS.** R0.4's hard ZERO-`.rs` constraint holds across the whole PRE-DATA instrument PR.

**CHECK 3 — MANIFEST NUMSTAT, insertions only, ZERO deletions.**

```
$ git diff --numstat feb85268..8a60f135 -- '…/spec356-manifest.md'
2066	0	packages/server-rust/benches/soak_harness/evidence/spec356-manifest.md
```

**PASS.** 2,066 insertions, **0** deletions.

**CHECK 4 — PREFIX `cmp`, RUN TWICE, AND WHY THE SECOND LEG EXISTS.**

```
CHECK 4a — against feb85268
$ M=packages/server-rust/benches/soak_harness/evidence/spec356-manifest.md
$ git show feb85268:$M > /tmp/pin.md ; wc -l < /tmp/pin.md
    1390
$ head -n $(wc -l < /tmp/pin.md) $M | cmp - /tmp/pin.md      <- SILENT, exit 0

CHECK 4b — against 607a3775, the pin's own predecessor
$ git show 607a3775:$M > /tmp/pin607.md ; wc -l < /tmp/pin607.md
    2727
$ head -n $(wc -l < /tmp/pin607.md) $M | cmp - /tmp/pin607.md  <- SILENT, exit 0

$ wc -l < $M          # manifest at the pin
    3456
```

**Both legs silent. PASS.** The second leg is not ceremony and AC3 says why in one measurement:
**`feb85268`'s copy of the manifest is 1,390 lines and contains NO §9** — `grep -c '^## §9'` over it
returns **0**. So the `feb85268` leg proves only that the first 1,390 lines survive unedited; it says
nothing at all about the 1,337 lines of §9 that landed after it, and **§9 could be interpolated anywhere
inside that range without the `feb85268` leg noticing.** The `607a3775` leg — 2,727 lines, §9 complete —
is what closes interpolation. **Only the pair protects §9 against deletion AND against interpolation**;
either leg alone is a guard with a hole in it.

**THE FOUR SUPPLEMENTARY ASSERTIONS, in the same B1 style:**

| Assertion | Command | Observed |
|---|---|---|
| §10 is a SINGLE section | `grep -c '^## §10' <manifest>` | **1** (at line 2731) |
| the seven pinned surfaces are UNMOVED from `feb85268` | `git diff feb85268..8a60f135 -- <each>` | **7/7 EMPTY** — `INVARIANTS.md`, `scripts/check-invariants.sh`, `spec349c2-fit.awk`, `spec356-slottruth.sh`, `spec356-tmplconf.awk`, `spec356-skeleton.txt`, `benches/soak_harness/monitor.rs` |
| v1's grader digest is UNMOVED | `shasum -a 256 spec356-slottruth.sh` | `280f7a3466a0bffc89059815bb3862bb9581cb95d7efa3fe58b1152749b8f060` — **matches §10.0.6** |
| the four tracker digests equal §10.0.7's ledger | `shasum -a 256 .specflow/todos/TODO-{634,637,638,648}.md` | **4/4 MATCH** |

**>>> B1 GATE: GREEN on all four checks and all four supplementary assertions.** A cell could then be run
under a build that can be named.

---

#### §10.1.2 — THE SQUASH-MERGE READING, RECORDED RATHER THAN RECONCILED

§10.0.1 rule 1 describes the pin as *"the merge commit of this PRE-DATA instrument PR, the commit whose
parents are `607a3775` and hop 10 above"*. **PR #146 landed as a SQUASH merge.** So:

```
$ git log --format='%H %P %s' -1 main
8a60f1357f4702f4c5393538ef6b4ae6cebd247f 607a37759ac921f04aac2aee7487b08f6d19cb79 feat(sf-356c): the PRE-DATA instrument PR for the §8.1 repeat (G1) (#146)
```

**The pin has ONE parent, `607a3775`, and hops 1–10 are its CONTENT rather than its second parent.** The
ten hop SHAs §10.0.1 tabulates are **not ancestors of `main`** — `git merge-base --is-ancestor <hop> main`
fails for all nine of `4a339066`, `4fbe1784`, `dd682eee`, `32f0f713`, `98e2d240`, `92acd2a7`, `1c21b3ed`,
`b8dabd5b`, `830e53a9`. **This is recorded, not repaired, and it is load-bearing for §10.1.5:** a data-
boundary check written against the ten hop SHAs would fail on every one of them, for a reason that has
nothing to do with ordering. The check that answers the question is the one over the commits that
introduced each artifact **into `main`'s own history**, and that is how §10.1.5 runs it.

**The property §10.0.1 actually asserts HOLDS and is proven.** Its stated property is that `607a3775` is
the predecessor and **nothing unnamed sits between it and the pin**: the `.rs` diff across the span is
EMPTY (§10.1.1 check 2), the manifest diff is additive-only with a byte-identical prefix (checks 3 and 4),
and the whole-tree delta `607a3775..8a60f135` is exactly the ten hops' surfaces. **A description of the
commit's PARENTAGE turned out to be a description of a merge STRATEGY nobody had fixed.** The lineage is
checkable from both ends exactly as §10.0.1 intended; only the sentence about parents reads differently
than it was written.

---

#### §10.1.3 — THE RUN WORKTREE AS EXECUTED (R5.2), AND THE ONE OBLIGATION THE RUNNER DOES NOT CATCH

**The worktree, created at the pin and never moved:**

```
$ git worktree add /private/tmp/spec356c-runtree 8a60f1357f4702f4c5393538ef6b4ae6cebd247f
Preparing worktree (detached HEAD 8a60f135)

R5.2 step 6, BEFORE the first cell:
$ git -C /private/tmp/spec356c-runtree status --porcelain     <- EMPTY (0 lines)
$ git -C /private/tmp/spec356c-runtree rev-parse HEAD
8a60f1357f4702f4c5393538ef6b4ae6cebd247f
```

**Clean AND at the pin.** *"A dirty run tree is a build nobody can name"* — it was not dirty, and the build
is named. **Nothing was ever committed from that worktree; it was never pulled, fetched, rebased or
re-checked-out for the whole 16 h of measurement.** Manifest and target-table authoring happened in the
main checkout, concurrently and freely; the main checkout's tip moved as the work required and the run tree
did not move at all. The two replicates ran **SEQUENTIALLY, never concurrently** (R2.2) — the harness is
machine-bound and two 8 h cells sharing a host perturb the very throughput that is the measurand.

**THE SINGLE BUILD AT THE PIN.** Both binaries were built once, in one session, from the run tree:

| Binary | Path | Built (UTC) |
|---|---|---|
| soak harness | `…/spec356c-runtree/target/release/deps/soak_harness-9abf891ab72dea71` | `2026-08-11T17:15:26Z` |
| server | `…/spec356c-runtree/target/release/topgun-server` | `2026-08-11T17:16:44Z` |

**`BUILD_GAP = 78 s.`** The runner warns only above 600 s on a non-provenance cell, so 78 s is well inside
the gate shape's requirement that the two binaries come from one build session at the pin. Exactly ONE
`soak_harness-*` binary existed under `target/release/deps/`, so the runner's `ls -t | head -1`
auto-resolution was unambiguous — and `SPEC356_SOAK_BIN` was set explicitly anyway, so the resolved binary
could not drift. `cargo fmt --check` in the run tree: exit 0. **`status --porcelain` re-verified EMPTY
after the build** (`target/` is gitignored, so building does not dirty the tree).

**R5.2 LIMB 5, PER COMMITTED CELL — BOTH LIMBS, MEASURED:**

| Cell | `grep -c '^  repo HEAD:      8a60f135…d247f$'` | `grep -c '^  repo HEAD:'` | verdict |
|---|---|---|---|
| `spec356c-long-r1.matrix.txt` | **1** | **1** | the pin is named **exactly once**, and no other SHA is named at all |
| `spec356c-long-r2.matrix.txt` | **1** | **1** | same |

**THE RUNNER'S `OUT_DIR = EVIDENCE_DIR` REFUSAL IS SMOKE-ONLY, AND THAT IS AN OPERATOR OBLIGATION THE
RUNNER DOES NOT CATCH.** The hard refusal at `spec356-prune.sh:361-366` sits **inside the SMOKE branch**
and fires for SMOKE only. **On a MEASUREMENT cell an unset `SPEC356_OUT_DIR` writes straight into the
tracked evidence dir with no refusal at all**; overriding it emits only `WARNING: artifact dir overridden
to <dir>` (`:383-385`). So writing a measurement cell's artifacts outside the tracked tree is something the
operator must *do*, not something the instrument enforces — and had it been forgotten, a half-written 8 h
cell would have been interleaved with the tracked evidence dir with no diagnostic. It was set explicitly on
both cells (`/private/tmp/spec356c-out-r1`, `/private/tmp/spec356c-out-r2`, both empty before the run, both
outside the evidence dir), and the residual that leaves — committed `matrix.txt` files self-describing
paths that do not exist in the repo — is explained by `spec356c-scratch-map.txt` and by nothing else
(P12). **Recorded here as a property of the instrument, not as a near-miss narrative:** the next round's
runner can close it, and until it does, the obligation is the operator's.

---

#### §10.1.4 — THE TWO EXECUTED CELLS: PROVENANCE, THE `exit 1` ATTRIBUTION, THE ADJ-20 BINDING

**PROVENANCE FIELDS, read from each cell's own `matrix.txt`** (both cells identical except for basename,
port-free knobs and the replicate suffix):

| Field | `long8h_r1` | `long8h_r2` |
|---|---|---|
| `repo HEAD:` | `8a60f135…d247f` ×1 | `8a60f135…d247f` ×1 |
| `dirty tree:` | **no** | **no** |
| `cell class:` | MEASUREMENT | MEASUREMENT |
| `duration:` | **28800s** | **28800s** |
| `TOPGUN_EPOCH_WIDTH:` | `<unset: production default 1000>` | `<unset: production default 1000>` |
| `TOPGUN_PRUNE_RECORD:` | `true (cell expects armed=yes)` | `true (cell expects armed=yes)` |
| `prune.csv cadence:` | 10 s | 10 s |
| `primary csv cadence:` | 60 s | 60 s |
| `soak binary:` / `server binary:` | the run tree's, built at the pin | the same two binaries |
| cell-name line (`long8h_r<n>`) | **×1** | **×1** |

**The width line is the production default and is UNSET rather than pinned to a literal**, which is the
configuration §8.1's repeat is a repeat *of*; `SOAK_SERVER_BINARY` is `<unset>` on both, so no
provenance-cell binary override was in play.

**ARMING AND COLUMN-SHAPE CENSUS, per cell, from its own `runner-console.log`:**

| Census | r1 | r2 |
|---|---|---|
| `arming witness PASSED` | **1** | **1** |
| disarmed literal `0/41` | **0** | **0** |
| `INSTRUMENT DEFECT` | **0** | **0** |
| anchored `^STEP0C ADMISSIBILITY: ` | **0** | **0** |
| unanchored `STEP0C ADMISSIBILITY` (cross-check on the banner's own `N`) | **0** | **0** |
| `SMOKE` | **0** | **0** |

**Zero anchored occurrences means there is no column to name** — checklist 7 requires the count published
*with the column each occurrence names*, and with a count of 0 the list of named columns is empty on both
replicates, which is the honest form of that report rather than an omission.

**THE `exit 1` ATTRIBUTION, IN FULL — AND WHAT IT IS NOT.** Both cells ended `exit 1`. The runner's own
closing lines say what that is:

```
RESULT: instrument sound; harness exit code 1.
NOTE: a non-zero harness exit is NOT automatically a failed characterization.
      The memory gate is neutralized by design for these runs, while the
      tombstone-byte gate, both blind-monitor clauses, convergence/recovery
      and panic capture stay armed. Read 'finishedReason' … and record the
      attribution, rather than reading the flag as a verdict.
```

`finishedReason`, read from each cell's own `soak.json`:

| Cell | `finishedReason` |
|---|---|
| r1 | `tombstone-byte growth slope 1182732.5 bytes/h exceeds 512.0 bytes/h (total growth 4994851 bytes over 5754 samples, last-half window 14403s)` |
| r2 | `tombstone-byte growth slope 1012162.6 bytes/h exceeds 512.0 bytes/h (total growth 4169864 bytes over 5753 samples, last-half window 14405s)` |

**The exit is the SPEC-345 tombstone byte-growth HARD gate firing on the measurand itself, with
`RESULT: instrument sound` printed beside it. Under R2.4 that is NOT an admissibility event.** It is not
`exit 9`, not `INSTRUMENT DEFECT`, not a `STEP0C ADMISSIBILITY` routing, and it is not a discard
disposition: **both cells STAND.** The census above is the proof that none of the four discarding
conditions occurred — `INSTRUMENT DEFECT` 0, `exit 9` never reached, `STEP0C ADMISSIBILITY` 0, arming
witness PASSED once per cell with the disarmed literal absent. A run that measures a backlog growing is a
run whose gate fires; reading the flag as a verdict is exactly what the runner's own NOTE forbids.

**THE ADJ-20 ARTIFACT BINDING, VERIFIED BEFORE ANY CONTENT CHECK, FROM INSIDE THE EVIDENCE DIR, VERBATIM,
WITH NO MAPPING APPLIED:**

```
$ shasum -a 256 -c spec356c-long-r1.artifacts.sha256      -> 7/7 OK
$ shasum -a 256 -c spec356c-long-r2.artifacts.sha256      -> 7/7 OK
```

Seven artifacts each — `.csv`, `.prune.csv`, `.soak.json`, `.progress.jsonl`, `.mechanism.json`,
`.matrix.txt`, `.harness-console.log` — and **the count is 7 rather than the gate captures' 6 for the
reason §10.0.9 (4) measured PRE-DATA: a 120 s smoke cell writes no `progress.jsonl`.** No rename occurs
anywhere in this lineage: each cell ran under its final committed basename, so the runner-written
`${BASE}.artifacts.sha256` verifies from the evidence dir with no mapping needed for name resolution
(R2.3, AC5). **No `RED PROVENANCE` on either cell**, so neither is re-run.

**THE COMMITTED LEDGERS' OWN SHAPE, re-derived from the committed bytes:**

| | r1 | r2 |
|---|---|---|
| `prune.csv` rows (excluding header) | **2,881** | **2,880** |
| first / last `elapsed_secs` | **0 / 28,800** | **10 / 28,800** |
| elapsed midpoint, and the coordinate last half it defines | 14,400 → rows 1,442–2,881, t = [14,410 … 28,800] | 14,405 → rows 1,441–2,880, t = [14,410 … 28,800] |
| window scrape count | **1,440** | **1,440** |
| non-empty drains, whole run | **39** | **80** |
| **non-empty drains, coordinate last half** | **0** | **0** |
| 0-sentinel fraction, committed column 43, that half | **1,440/1,440 = 100.000000 %** | **1,440/1,440 = 100.000000 %** |
| `current_epoch == low_water_mark`, full ledger | 2,779/2,881 = **0.964596** | 2,796/2,880 = **0.970833** |
| `current_epoch == low_water_mark`, coordinate last half | 1,390/1,440 = **0.965278** | 1,401/1,440 = **0.972917** |
| final `tombstone_bytes` / `bytes_freed` | **19,914,164 / 848,707** | **20,642,504 / 1,810,354** |
| final `low_water_mark` | **874** | **906** |

**The two replicates' windows are the SAME coordinate span and the same 1,440 scrapes, reached from
different first ticks** (r1's first scrape is 0, r2's is 10). That is a property of the coordinate rule
(ADJ-17 as adjudicated by ADJ-18) and is recorded rather than assumed.

---

#### §10.1.5 — THE DATA BOUNDARY, PROVEN BY `git log --follow --diff-filter=A`

R0.3 fixes the boundary as the **first `spec356c-*.soak.json`**, decided by the command and by no other
test — **never by authorship and never by self-report.**

```
$ git log --follow --diff-filter=A --format=%H --reverse \
      -- 'packages/server-rust/benches/soak_harness/evidence/spec356c-*.soak.json' | head -1
0b96e63e306a7e51fa04a022eeaff7762497ac1d
  test(sf-356c): land replicate r1 of the §8.1 repeat — 28,800 s at the pin   (2026-08-12 12:29:32 +0300)
```

Both adds, for completeness: `0b96e63e` (r1) then `e13e9755` (r2). **THE BOUNDARY IS `0b96e63e`.**

**DESCENDANCY, run against the commits that introduced each PRE-DATA artifact INTO `main`'s HISTORY** —
which, per §10.1.2, is where the squash makes the difference:

| PRE-DATA instrument byte | Commit that ADDED it to `main` | Ancestor of `0b96e63e`? |
|---|---|---|
| **§10.0** (`git log -S'#### §10.0.1' -- <manifest>`) | **`8a60f135`** (the pin) | **YES** |
| the two additive runner arms (`-S'long8h_r1'` over `spec356-prune.sh`) | **`8a60f135`** | **YES** |
| `spec356c-slottruth-v2.sh` | **`8a60f135`** | **YES** |
| `spec356c-targets.sh` | **`8a60f135`** | **YES** |
| `spec356-verdict-xask.md` | **`607a3775`** (the predecessor; R3.0) | **YES** |
| both `PART V` gate re-captures (`-S'PART V -- RE-CAPTURE AT THE SPEC-356c RUNNER'`) | **`8a60f135`** | **YES** |
| `spec356c-slottruth-v1-v2.diff`, `…-runs.txt`, `spec356c-trackergrade.sh`, the five dry-run CSVs | **`8a60f135`** | **YES** |

```
$ git merge-base --is-ancestor 8a60f1357f4702f4c5393538ef6b4ae6cebd247f 0b96e63e   -> exit 0
```

**Every PRE-DATA instrument byte entered `main` at or before the pin, and the boundary is a strict
descendant of the pin. NOT ONE of them landed after the first datum. The data boundary is GREEN**, and the
finding checklist item 4 exists to catch — *the instrument was chosen with data visible* — did not occur.

**Stated because it is the honest form of this check after a squash:** running the same descendancy test
against §10.0.1's ten hop SHAs returns *not an ancestor* for all nine that are not the pin, and that answer
carries **no** information about ordering. The table above is the test that carries information, because
each row asks which commit in `main`'s own history first contains the bytes.

---

#### §10.1.6 — THE SIXTEEN `tombstone_bytes` FITS, AND ADJ-19'S ADMISSIBILITY PER WINDOW

**Instrument:** the **UNFORKED** pinned fitter `spec349c2-fit.awk`, digest
`840813461e3b1bd5c3a79291044d8ac515e09b94333ee530cd6a10de8fa0436f` — **unmoved** from the pin
(`git diff --stat 8a60f135..HEAD -- '…/spec349c2-fit.awk'` EMPTY).
**Subject:** the **primary soak ledger** `spec356c-long-r{1,2}.csv`, which is the only committed artifact
carrying a `tombstone_bytes` header at all; the prune ledger carries `topgun_ormap_tombstone_bytes_total`
and is a different series on a different cadence, and the two are **not** interchanged anywhere.
**Windows:** eight equal **3,600 s coordinate windows** over `[0, 28800]`, committed as
`spec356c-long-r{1,2}-seg{1..8}.csv`.
**Invocation, per window:** `awk -v col=tombstone_bytes -v window=full -f spec349c2-fit.awk <segment>`.

**THE WRITING OBLIGATION IS OBSERVED THROUGHOUT §10.1 AND §10.2** (checklist 10, carried from SPEC-356b
checklist 7(i)): in these two subsections **every line containing `slope_mb_per_hour` also carries `tombstone_bytes`**,
this one included — the field name is the fitter's, the unit here is
**BYTES per hour**, and the fitter's own header comment says so.
**MEASURED, AND REPORTED RATHER THAN IMPLIED, BECAUSE THE OBLIGATION'S OWN GREP IS SCOPED TO THE WHOLE §10
RANGE:** the manifest carries **THREE** lines containing that field name without `tombstone_bytes` beside
it, and **all three are OUTSIDE §10.1 / §10.2** — two in §9.14's own fit table and its surrounding prose,
and **one in §10.0.4's P1 entry**, which is a **PRE-DATA byte committed at the pin**. **It is NOT edited
here.** §0–§8, §8A, §9 and §10.0 are byte-frozen (AC12, checklist 3), and R0.3 forbids repairing a
pre-declaration byte after the data boundary regardless of how small the repair is. **Recorded so the
reading is a measurement rather than a claim**, and so that the §10-range grep's result is explained by the
record instead of being a surprise to whoever runs it.

| window | `t_start_secs` – `t_end_secs` | rows_used / skipped_empty | `slope_mb_per_hour` over `tombstone_bytes` (BYTES/h) | `se_mb_per_hour` | `r2` | ADJ-19 |
|---|---|---|---|---|---|---|
| **r1 W1** | 0 – 3,540 | 60 / 0 | **117827.534983** | 5561.508565 | 0.885569 | ADMISSIBLE |
| **r1 W2** | 3,600 – 7,140 | 60 / 0 | **207967.947007** | 3266.104991 | 0.985897 | ADMISSIBLE |
| **r1 W3** | 7,200 – 10,740 | 60 / 0 | **88150.151884** | 3442.957481 | 0.918712 | ADMISSIBLE |
| **r1 W4** | 10,800 – 14,340 | 60 / 0 | **133648.561563** | 5596.911525 | 0.907674 | ADMISSIBLE |
| **r1 W5** | 14,400 – 17,940 | 60 / 0 | **729248.519033** | 15399.789678 | 0.974787 | ADMISSIBLE |
| **r1 W6** | 18,001 – 21,540 | 60 / 0 | **1215872.193703** | 2377.783789 | 0.999778 | ADMISSIBLE |
| **r1 W7** | 21,600 – 25,140 | 60 / 0 | **1274493.610299** | 2072.247309 | 0.999847 | ADMISSIBLE |
| **r1 W8** | 25,200 – 28,800 | 61 / 0 | **1204363.419783** | 3194.869218 | 0.999585 | ADMISSIBLE |
| **r2 W1** | 60 – 3,540 | **59 / 1** | **229593.089421** | 4992.546736 | 0.973755 | ADMISSIBLE |
| **r2 W2** | 3,600 – 7,140 | 60 / 0 | **90794.533878** | 3245.547210 | 0.931002 | ADMISSIBLE |
| **r2 W3** | 7,200 – 10,740 | 60 / 0 | **45528.841901** | 3503.186329 | 0.744388 | ADMISSIBLE |
| **r2 W4** | 10,800 – 14,340 | 60 / 0 | **49869.596953** | 3892.783355 | 0.738875 | ADMISSIBLE |
| **r2 W5** | 14,400 – 17,940 | 60 / 0 | **297201.265800** | 9866.497135 | 0.939918 | ADMISSIBLE |
| **r2 W6** | 18,000 – 21,540 | 60 / 0 | **973491.571502** | 9790.809924 | 0.994167 | ADMISSIBLE |
| **r2 W7** | 21,600 – 25,140 | 60 / 0 | **1216356.036210** | 2379.217275 | 0.999778 | ADMISSIBLE |
| **r2 W8** | 25,200 – 28,800 | 61 / 0 | **1266853.932705** | 1991.241245 | 0.999854 | ADMISSIBLE |

**`r2 W1`'s `skipped_empty = 1` is the fitter reporting a gated first tick, not a defect** — the harness's
`tombstone_bytes` series is empty on r2's first scrape and the fitter skips it and says so in its own
output. It is the same empty field the r2 runner console names three times, and **none of those three is a
`prune.csv` column** (§10.2's limb (c) leg 1 records the same reading independently).

**ADJ-19'S ADMISSIBILITY, MEASURED RATHER THAN ASSUMED.** ADJ-19 clause 2 bounds **distribution, not just
volume**: within each derived slice the maximum gap between consecutive `elapsed_secs` values must be
**≤ 5 × cadence**. The primary ledger's cadence is **60 s**, so the bound is **300 s**:

| | max consecutive `elapsed_secs` gap, per window | bound | outcome |
|---|---|---|---|
| r1 W1…W8 | 61, 61, 61, 61, 60, 60, 61, 61 | 300 | **8/8 ADMISSIBLE** |
| r2 W1…W8 | 60, 61, 60, 61, 61, 61, 61, 61 | 300 | **8/8 ADMISSIBLE** |
| r1 / r2 whole ledger (481 rows each) | 61 / 61 | 300 | **both ADMISSIBLE** |

**All 16 windows and both parent ledgers are ADMISSIBLE, and NO window coarsens.**

**ADJ-18 clause 2's epoch integrity bound is NOT EVALUABLE on the byte windows, and that is a scoping fact
rather than a skipped check:** the primary ledger carries **no `current_epoch` column at all**, so the
monotonicity-and-jump bound has no axis to read there. The clause is scoped to the **pass-rate chain**,
whose fit axis *is* `current_epoch`, and it was evaluated there — both derived slices of both replicates
are monotone non-decreasing in `current_epoch`, with **226 / 188** positive jumps on r1's early / late
slice and **229 / 217** on r2's, all far above ADJ-19 clause 1's **≥ 5** minimum-evidence gate; and the
parent prune ledger's coordinate last half has a maximum consecutive `elapsed_secs` gap of **11 s** on both
replicates against that chain's **50 s** bound (10 s cadence × 5). **Both replicates ADMISSIBLE on the
pass-rate chain.**

---

### §10.2 — THE PRESENCE VERDICT (PUBLISHED FIRST), THE FROZEN WALK, AND THE DETERMINATION

**ORDER IS PART OF THE OBLIGATION.** R6.2 makes the PRESENCE limb **Validation Checklist item 1**, and it
is evaluated **FIRST, before every reproduce-the-values limb**. So §10.2 opens with the per-entry
PRESENCE verdict and only then walks the predicate. **R6.4 governs the publication:** the verdict appears
**whatever it is** — *"a RED entry is not a reason to delay publication; it IS the finding, and it names
the missing artifact."* **This section carries TWO RED entries. They are published here, at the front,
not footnoted at the back.**

**R6.2's OTHER SIDE IS GRADED TOO: no entry may RED on an honest value.** Where a limb's honest value on
this round is an explicitly-marked `UNDEFINED`, that literal **satisfies** the limb — R4.6's
empty-population branch is the case that forced the rule into writing, and it is live on both replicates
here.

**Each entry is graded against §10.0.4's own P-text**, which is the artifact the limb is graded from — not
against a paraphrase of it, and not against the spec's shorter table.

---

#### §10.2.1 — THE PRESENCE LIMB, ENTRY BY ENTRY, P1…P13

| # | Verdict | One-line reason |
|---|---|---|
| **P1** | **GREEN** | 16 fit rows in §10.1.6, exact field names, ≥ 8 per replicate |
| **P2** | **GREEN** | five anchored limb headings below, each ≥ 1 value row; limb (iii)'s value row is R4.6's `UNDEFINED` literal, which **satisfies** the limb |
| **P3** | **GREEN** | `spec356c-t1-lwmpass-r{1,2}.csv`, exact header, 414 / 446 data rows |
| **P4** | **GREEN** | `spec356c-t2-drains-r{1,2}.csv`, 39 / 80 data rows, `bytes_freed_matches_attribution` present |
| **P5** | **GREEN** | `spec356c-t3-windows-r{1,2}.csv`, 28 `FULL` + 1 `PARTIAL` **last**, both replicates |
| **P6** | **GREEN** | `spec356c-t4-rate-r{1,2}.csv`, prior-duration arm rows from `spec356-long` explicitly included |
| **P7** | **GREEN** | `spec356c-t5-fate-r{1,2}.csv`, 415 / 447 data rows, marker column header **verbatim** |
| **P8** | **GREEN** | the `-U0` diff with its invocation verbatim, 4 hunks, each exactly one class, each passing its own mechanical test; **both** run-1 / run-2 transcripts present |
| **P9** | **RED** | **the five `T1.`…`T5.` anchors do NOT occur in `spec356-verdict-xask.md` — measured count ZERO.** The §8.3 block-quote leg is GREEN |
| **P10** | **GREEN** | all five dry-run CSVs, every band met: 14 `FULL` + 1 `PARTIAL`; 53 / 2; 232; 233; 53 |
| **P11** | **GREEN** | both `PART V` re-captures, **all four named blocks** by exact anchor in each |
| **P12** | **GREEN** | `spec356c-scratch-map.txt`, 2 map lines, basenames identical, each matching that cell's own `matrix.txt` |
| **P13** | **RED** | **the POST half is ABSENT from the record at the instant this verdict is written.** The PRE half (§10.0.7) is GREEN |

**TOTALS: 11 GREEN, 2 RED.**

**P1 — the eight-window `tombstone_bytes` fit table, per replicate (16 fits).**
*Assertion (§10.0.4):* §10 contains a table whose header line matches exactly and which carries **≥ 8 data
rows per replicate**; each row names its window and carries, for `tombstone_bytes`, `slope_mb_per_hour`
plus that fit's `se_mb_per_hour` and its `r2`.
*Observed:* §10.1.6 carries one table, header line carrying all three field names exactly as the pinned
fitter prints them, **8 data rows for r1 and 8 for r2**, each naming its window (`W1`…`W8`) and its
coordinate span, each carrying all three values at the fitter's own printed precision, all 16 reproduced
from the committed segments by the unforked fitter at digest `840813461e…436f`.
**GREEN.**

**P2 — the unconditional bundle, per replicate: ADJ-3's limbs (i)…(iv) plus R8.1a's limb (v).**
*Assertion:* five anchored subsection headings present, each carrying **≥ 1 value row**; limb (iii)
additionally carries the excluded fraction. **EMPTY-POPULATION BRANCH (R4.6):** when the non-sentinel
population is empty, limb (iii)'s value row **IS** the literal
`B = UNDEFINED (100 % sentinel, 0 non-sentinel rows)` with the fraction beside it, and **that literal
SATISFIES the `≥ 1 value row` limb and must never RED it.**
*Observed:* §10.2.4 carries all five limb headings, each with a per-replicate value table. Limb (iii)'s
value row **is** that literal on both replicates, with `B_SENTINEL_EXCLUDED_PCT = 100.000000` and the row
count it rests on (`0`) beside it. **This is precisely the case R6.2's two-sidedness was written for: an
honest 100 %-sentinel replicate reporting correctly must not be punished for it.** Limb (ii)'s `L ≤ B`
fraction is likewise published as an explicitly-marked `UNDEFINED`, and that too is an honest value, not an
absence.
**GREEN.**

**P3 — the `t1-lwmpass` table or its OUT-OF-SCOPE row.**
*Assertion:* either the CSV exists with its exact header and ≥ 1 data row, **or** §10.3 carries an
OUT-OF-SCOPE row naming the reason and an owner.
*Observed:* both CSVs exist and are committed; the header line resolves exactly
(`t_secs,lwm_before,lwm_after,epochs_passed,current_epoch,indexed_refs,indexed_epochs,…`); **414 data rows
on r1 and 446 on r2**, both ≫ 1. The CSV limb is satisfied on its own, independently of the OUT-OF-SCOPE
row that also exists for the per-epoch-residency half of that target.
**GREEN.**

**P4 — the `t2-drains` table or its OUT-OF-SCOPE row.**
*Assertion:* as P3; and the `bytes_freed_matches_attribution` column **must be present even when every
drain matches**.
*Observed:* both CSVs exist; **39 data rows on r1, 80 on r2** — which is exactly the whole-run non-empty
drain count re-derived independently from each `prune.csv` in §10.1.4, so the enumeration is complete
rather than sampled. `bytes_freed_matches_attribution` is present as **column 15** of the header on both.
**GREEN.**

**P5 — the `t3-windows` table or its OUT-OF-SCOPE row.**
*Assertion:* **≥ 28 data rows per replicate**, exact header, graded as **28 rows with `window_kind=FULL`
and AT MOST ONE `PARTIAL`, which must be LAST**.
*Observed:* both CSVs carry **29 data rows: 28 `FULL` + 1 `PARTIAL`, and the `PARTIAL` is the last row**
on both replicates — the 28,800 s run divided into 1,000 s coordinate windows, with the ragged remainder
marked rather than silently folded into a neighbour.
**GREEN.**

**P6 — the `t4-rate` table or its OUT-OF-SCOPE row.**
*Assertion:* table present with the **prior-duration arm row** from `spec356-long` explicitly included.
*Observed:* both CSVs exist with 34 data rows; each carries **`PRIOR_ARM_WHOLE_RUN,spec356-long,…`** and
**`PRIOR_ARM_COORD_LAST_HALF,spec356-long,…`** as explicit rows, the second carrying
`THE_DISCRIMINANT` in its `discriminant_role` column and the first `PUBLISHED_NEVER_THE_DISCRIMINANT`.
The prior arm is **in the table**, not referenced from prose.
**GREEN.**

**P7 — the `t5-fate` ledger or its OUT-OF-SCOPE row.**
*Assertion:* **≥ 1 data row per replicate**, and the column header
`indexed_refs_at_lwm_pass__AGGREGATE_NOT_PER_EPOCH` present **VERBATIM**.
*Observed:* both ledgers exist — **415 data rows on r1, 447 on r2** — and the marker resolves as an exact
whole-field match on the header line of both (`grep -cx` over the comma-split header returns **1**). The
marker was not renamed into anything that reads like an answer, which is the whole point of grading it
verbatim.
**GREEN.**

**P8 — the v1→v2 grader diff and BOTH run-1 / run-2 transcripts.**
*Assertion:* the file exists; **its header records the `-U0` invocation verbatim**; **≥ 1 hunk**; **every
hunk carries EXACTLY ONE of `class A` / `class B` / `class C`**, and a hunk whose label fails its own
mechanical test ⇒ RED with the hunk quoted; **both transcripts present**.
*Observed:* `spec356c-slottruth-v1-v2.diff` exists and records
`diff -U0 spec356-slottruth.sh spec356c-slottruth-v2.sh` verbatim in its header. **FOUR hunks, each
carrying exactly one label, each passing its own test:**

| hunk | label | mechanical test | outcome |
|---|---|---|---|
| `@@ -2,2 +2,8 @@` | `class C` | every `+`/`-` line begins with `#` after leading whitespace; **v1 `:1` (the shebang) not in the hunk** | passes |
| `@@ -22,4 +28,4 @@` | `class B` | v1 `:22-25`, inside the five-row table; `d == b` (4 → 4), **zero added lines** | passes |
| `@@ -77 +83 @@` | `class B` | v1 `:77`, inside the table; `d == b` (1 → 1), **zero added lines** | passes |
| `@@ -259,6 +265,23 @@` | `class A` | v1 `:259-264`, **inside** the published bound `:258-265` | passes |

Both transcripts are present in `spec356c-slottruth-v1-v2-runs.txt` (`RUN 1 — v1, UNMODIFIED, over
spec356-long`; `RUN 2 — v2, DEFAULT PARAMETERS, over the SAME spec356-long bytes`), together with the
run-1-vs-run-2 regression diff.
**GREEN — and the neighbouring failure does NOT touch this entry, for a reason worth stating exactly rather
than leaving to inference.** §10.2.6's finding 1 is that **run 3** of R4.3's three-run matrix refused on
both replicates. **`RUN 3` IS NOT IN P8's TEXT.** P8 names *"the v1→v2 grader diff … and BOTH run-1 /
run-2 transcripts"*, and all three of those artifacts exist, are committed, and pass every limb P8 states.
Run 3 lives in **AC7's run-3 leg** and in **Validation Checklist item 12's run-3 leg**, and **both of those
are RED** — they are graded where they live, in this section's checklist, and the finding is published in
§10.2.6 with its routing. **Charging P8 with run 3 would make the PRESENCE limb report a failure it does
not define, which is as much a defect as missing one it does** — the limb's list is CLOSED in both
directions, and a closed list that absorbs neighbouring failures stops being a list.

**P9 — `spec356-verdict-xask.md` and the §8.3 quote beside the outcome. — RED.**
*Assertion (§10.0.4, verbatim):* *"the artifact exists with **≥ 1 line matching each of the five target
anchors `T1.`…`T5.`**; §10 carries **§8.3's paragraph as a block quote adjacent to the published
determination**."*
*Observed, leg 1 — the artifact and its anchors:* the artifact **exists** and is committed (17 KB, added to
`main` at `607a3775`, PRE-DATA, §10.1.5). **But the anchors are ABSENT.** `grep -nE 'T[1-5]'` over the
whole file returns **exactly ONE line** — line 5, the descriptive phrase *"pre-declared observation targets
(T1-T5)"* — and **`T1.`, `T2.`, `T3.`, `T4.` and `T5.` each occur ZERO times.** The five targets **are** in
the artifact, in the vendor's own words, enumerated as `1.` … `5.` at lines 69, 71, 73, 75 and 77 inside
the answer block quote:

```
> 1. **indexed_refs vs added_tombstones, per-epoch, at the moment LWM passes that epoch.** …
> 2. **Full content enumeration of every non-empty drain** …
> 3. **delta(LWM) vs delta(bytes_freed), windowed every 1000s.** …
> 4. **Non-empty drain rate vs duration.** …
> 5. **Epoch content fate ledger** …
```

**`T1`…`T5` is §10.0.2's OWN naming applied to that enumeration; the source artifact never carried it.**
*Observed, leg 2 — the block quote:* **GREEN.** §10.2.3 carries §8.3's paragraph **verbatim as a block
quote, adjacent to the published determination**, which is where §8.3's own last sentence requires it.
*Verdict:* **RED**, because the limb is a conjunction and leg 1 fails on the limb's own standard. **Two
readings exist and both are published rather than one being chosen quietly.** Under the **mechanical**
reading — the one R6.2 mandates (*"an exact-match anchor (`grep -qxF`, or a `test -f` plus a required
header line)"*) and the one §10.0.4's P8 entry explicitly privileges over purpose — the anchor is absent
and the entry is RED. Under a **substance** reading the entry's stated purpose is served completely: its
absence *"would otherwise be mistaken for 'the targets were invented by the executor'"*, and they
demonstrably were not — they are the vendor's own five, quoted above. **So the RED is a defect of the
PRE-DECLARED ANCHOR, not evidence about the targets' provenance, and it must not be read as the latter.**
*What is NOT done about it, and why:* the artifact is **not** edited to carry `T1.`…`T5.`, the P9 entry is
**not** re-worded, and no `ADJ-21` is authored. **R0.3 is categorical**: the boundary is `0b96e63e`
(§10.1.5), this reading was taken after it, and after the boundary a finding is *"a POST-DATA RECORD routed
to a named follow-on — never a predicate edit, never an addendum, never a checklist patch."* Editing either
side of a mismatch discovered at grading time is exactly the defect PD-8 indicts. **Routing: `TODO-648`**
(the pinned-artifact defect class — a defect found in a committed artifact that post-data rules forbid
repairing) **and `TODO-634`** (as an input to the next round's pre-declaration: an anchor asserted against
an artifact must be **measured against that artifact when the entry is written**, which is the discipline
§10.0.4 already applied to P8, P10 and P11 and did not apply to P9 — those three carry a `MEASURED` line
and **P9 carries none**). **No tracker byte is written by this section; the §10 record IS the routing, and
that is a limitation of it, not a closed loop.**

**P10 — the target builder's PRE-DATA DRY-RUN set: the five `spec356c-dryrun-*-long.csv`.**
*Assertion:* all five exist with their exact headers, **and each file's data-row count falls inside its
pre-declared BAND** — `t3-windows` EXACT 14 `FULL` + 1 `PARTIAL`; `t4-rate` EXACT 53 whole-run / 2
coordinate-last-half; `t1-lwmpass` ≥ 200; `t5-fate` ≥ 200; `t2-drains` ≥ 40.
*Observed:* all five committed. **`t3-windows`: 14 `FULL` + 1 `PARTIAL` — exact. `t4-rate`: 53 and 2 —
exact, on both the `SUBJECT_*` and the `PRIOR_ARM_*` row pairs. `t1-lwmpass`: 232 ≥ 200. `t5-fate`:
233 ≥ 200. `t2-drains`: 53 ≥ 40. EVERY BOUND MET**, and every one re-derived here from the committed files
rather than copied from §10.0.8.
**GREEN.**

**P11 — BOTH `PART V` gate re-captures, in the PART IV four-block shape.**
*Assertion:* for `spec356a-eager-registration.log`, all four named blocks by exact anchor — **(a)**
`limb (a): 33/33`; **(b)** the census anchored on **`empty_fields=0`** (unspaced) over the 43 columns;
**(c)** the arming witness as its own block, `41/41` armed **and** `0/41` disarmed, both `PASSED`;
**(d)** the ADJ-20 binding with an independent `shasum -a 256 -c` reported **`N/N OK`**. For
`spec356a-step0c-fixture.log`: the guard fired, `sort -u` over **all five** captures yields **ONE**
`STEP0C ADMISSIBILITY` line, instrument-defect literal count **0**, exit **0**.
*Observed, `spec356a-eager-registration.log`* — the `PART V` block opens at `:929` and its verdict block
carries, by exact anchor and as four separate blocks:
`limb (a): 33/33 pinned names`; `limb (b): empty_fields=0 over 43 columns x 12 rows (47 empty=0 column
lines counted on the run)`; `witness: armed 41/41 PASSED; disarmed direction 0/41 PASSED`;
`binding: 6 artifacts digested; independent shasum -c 6/6 OK`. **The `N` is 6 here and 7 on a measurement
cell for the reason §10.0.9 (4) measured PRE-DATA — a 120 s smoke cell writes no `progress.jsonl` — and
P11 grades a generic `N/N OK` precisely so neither number REDs the other.**
*Observed, `spec356a-step0c-fixture.log`* — `PART V` opens at `:590`; the guard **fired** with 1 emitted
line; `grep '^STEP0C ADMISSIBILITY' | sort -u | wc -l` over the whole file returns **1** across all five
captures; `INSTRUMENT DEFECT` count **0**; exit **0**.
**GREEN, all four blocks in each log, asserted separately rather than as a shape.**

**P12 — `spec356c-scratch-map.txt`.**
*Assertion:* file exists; **≥ 2 lines**, one per replicate, each of the exact shape
`<scratch dir> → <evidence dir>` with the **basename identical on both sides**; and **each committed cell's
scratch dir matches the `data dir:` / `prune.csv:` paths inside that cell's own `matrix.txt`**.
*Observed:* the file exists and carries **exactly two lines bearing the arrow** — the file states, and it
is true, that the arrow character appears ONLY on the map lines, so a count of it is a count of committed
replicates:

```
/private/tmp/spec356c-out-r1 → …/evidence   basename spec356c-long-r1.*   data dir /private/tmp/spec356c-data-r1
/private/tmp/spec356c-out-r2 → …/evidence   basename spec356c-long-r2.*   data dir /private/tmp/spec356c-data-r2
```

Cross-checked against each cell's own `matrix.txt`: r1's `prune.csv:` line reads
`/private/tmp/spec356c-out-r1/spec356c-long-r1.prune.csv` and its `data dir:` line
`/private/tmp/spec356c-data-r1`; r2's read the `-r2` equivalents. **Basename identical on both sides, no
rename anywhere in the lineage, both replicates mapped** — so R2.3 point 5's *"re-run, not annotated"*
disposition is not engaged for either cell.
**GREEN.**

**P13 — the tracker-discipline ledger and its discrimination transcripts. — RED.**
*Assertion:* §10.0 carries the four `shasum -a 256` values, the §8.1 box's verbatim pre-text and the tick
counts **2 / 5**; **§10.x carries the four POST-digests, the box's POST-text, the post tick counts, and all
five PRE-DATA discrimination transcripts plus wave 5's own real run**, graded on limbs (a)…(e).
*Observed, the PRE half:* **GREEN and re-measured here.** §10.0.7 carries all four digests; reading the
live files reproduces them exactly —
`555c3aa9…4f8c` (`TODO-634.md`), `48a16fbd…940f` (`TODO-637.md`), `756082197…25df` (`TODO-638.md`),
`56fb3f59…ef2c` (`TODO-648.md`) — **4/4 MATCH**, and `spec356c-trackergrade-proofs.txt` carries all five
PRE-DATA transcripts with their observed verdicts.
*Observed, the POST half:* **ABSENT at the instant this verdict is written.** `TODO-634.md`'s digest is
**still `555c3aa9…4f8c`, i.e. UNMOVED** — and §10.0.4's limb **(b)** is explicit that *"an unchanged digest
means the required edit never happened and is **RED**, not a pass"*. The four post-digests, the box's
post-text, the post tick counts and the **sixth** transcript (the real GREEN run against the actually-edited
file, which §10.0.4 assigns to wave 5) are not in the record.
*Verdict:* **RED**, on the POST half only, and published rather than deferred. **R6.4 is the whole reason
this reads RED instead of being held back:** a PRESENCE limb that is *"evaluated FIRST"* can only honestly
speak about the record **as it stands at that instant**, and pre-crediting bytes that are due later is the
vacuous-guard failure PD-8 indicts wearing a different hat. **When §10's tracker-discipline ledger lands
the post-digests, the post-text, the post tick counts and the sixth transcript, the POST half is graded
THERE, against limbs (a)…(e), at its own bytes** — a later GREEN there is not a contradiction of this RED,
it is the next reading of the same limb, and this entry says so explicitly so that no reader has to
reconcile the two by guessing.

---

#### §10.2.2 — THE R8.1 WALK, PER REPLICATE, IN ITS FROZEN ORDER

**The predicate is FROZEN: it is WALKED here, not interpreted.** The governing text is §0–§8 **together
with §8A's twenty addenda**, read at this manifest. **Step 0's four limbs are evaluated first, in the
frozen order, with limb (d) FIRST of all (R1.2 item 1), and Step 0 is FAIL-CLOSED.**

**THE WINDOW EACH WALK READS** — the **COORDINATE** last half (ADJ-17 as adjudicated by ADJ-18): rows whose
`elapsed_secs` **exceeds** the full ledger's elapsed midpoint `(t_first + t_last)/2`. A coordinate, never a
row index. Both replicates resolve to the same span **[14,410 … 28,800]** and **1,440 scrapes** (§10.1.4).
Deltas are `value(last row of window) − value(first row of window)`, the convention verified against the
prior arm where it reproduces §9.5.3's published `Δpasses = 232,367` exactly.

**REPLICATE r1**

| Limb, in the frozen order | Evaluated value | Result |
|---|---|---|
| **(d)** ADJ-11 clause 0's conservation identity, **FIRST** | `Δpasses` **414,110** = `Δempty_drains` **414,110** + `Δnonempty_drains` **0** | **HOLDS** — no unaccounted passes |
| **(a)** §1.3's 2×2 in CLEAN or BUILD-LINEAGE-EFFECT | `R51_T = 0.346905` NOT REJECTED and `R52_T = 0.800515` NOT REJECTED against the frozen critical value 4.303 ⇒ §1.3 cell **CLEAN** | **HOLDS** |
| **(b)** ≥ 1 split recompute in the window read | `split_recomputes_total` 917 → 1,746, **Δ = 829 ≥ 1** (`split_computed_epoch` 460 → 874) | **HOLDS** — the split is not stale |
| **(c)** population check on every deciding column **AND** ADJ-12's ≤ 50 % sentinel readability on committed column 43 | **LEG 1 PASSES**: `STEP0C ADMISSIBILITY` 0, `INSTRUMENT DEFECT` 0, arming witness `41/41 PASSED` ×1, and an independent census over 43 columns × 2,881 rows finds **0** empty-or-NaN fields. **LEG 2 FAILS**: 0-sentinel rows **1,440 / 1,440 = 100.000000 %**, **0** non-sentinel rows, against the frozen `> 50 %` | **FAILS** (conjunction) |

| Step | Evaluated value |
|---|---|
| **Step 1** (non-drop exit share > 10 %) | **NOT EVALUATED — Step 0 is fail-closed and precedes every numbered step** |
| **Step 2** (exit share ≤ 10 % and `median(L) ≤ B`) | **NOT EVALUATED — Step 0 is fail-closed and precedes every numbered step** |
| **Step 3** (exit share ≤ 10 %, persistent backlog, slope rejects negative) | **NOT EVALUATED — Step 0 is fail-closed and precedes every numbered step** |
| **Step 4** (exit share ≤ 10 %, persistent backlog, slope does not reject) | **NOT EVALUATED — Step 0 is fail-closed and precedes every numbered step** |
| **Step 5** (Step 0 fails, or the deciding columns are unreadable) | **FIRED ⇒ `INDETERMINATE`, via Step 0 limb (c)** |

**REPLICATE r2** — evaluated independently, same frozen order.

| Limb, in the frozen order | Evaluated value | Result |
|---|---|---|
| **(d)** conservation identity, **FIRST** | `Δpasses` **445,682** = `Δempty_drains` **445,682** + `Δnonempty_drains` **0** | **HOLDS** |
| **(a)** §1.3's 2×2 | same controls over the same inherited bytes: `R51_T = 0.346905` and `R52_T = 0.800515`, both NOT REJECTED against 4.303 ⇒ **CLEAN** | **HOLDS** |
| **(b)** ≥ 1 split recompute | `split_recomputes_total` 918 → 1,810, **Δ = 892 ≥ 1** (`split_computed_epoch` 460 → 906) | **HOLDS** |
| **(c)** population check **AND** ADJ-12 sentinel readability | **LEG 1 PASSES**: `STEP0C ADMISSIBILITY` 0, `INSTRUMENT DEFECT` 0, arming witness `41/41 PASSED` ×1; independent census over 43 columns × 2,880 rows finds **0** empty-or-NaN fields. *(Three `empty=1` strings do appear in that cell's runner console; **all three belong to the harness's primary `spec356c-long-r2.csv` ledger's `tombstone_bytes` series** — one gated first tick, the same one the fitter reports as `skipped_empty=1` in §10.1.6 — and **none is a `prune.csv` column**. Recorded so the reading is checkable rather than asserted.)* **LEG 2 FAILS**: **1,440 / 1,440 = 100.000000 %**, **0** non-sentinel rows | **FAILS** |

| Step | Evaluated value |
|---|---|
| **Step 1** | **NOT EVALUATED — Step 0 is fail-closed and precedes every numbered step** |
| **Step 2** | **NOT EVALUATED — Step 0 is fail-closed and precedes every numbered step** |
| **Step 3** | **NOT EVALUATED — Step 0 is fail-closed and precedes every numbered step** |
| **Step 4** | **NOT EVALUATED — Step 0 is fail-closed and precedes every numbered step** |
| **Step 5** | **FIRED ⇒ `INDETERMINATE`, via Step 0 limb (c)** |

**THE BLOCKING ADMISSIBILITY LIMB, NAMED BY LETTER, WITH ITS VALUE AGAINST ITS FROZEN THRESHOLD:**
**limb (c)** — sentinel fraction **`1,440 / 1,440 = 100.000000 %`** against the pre-registered **`> 50 %`**,
on **both** replicates. **100 % is inside `> 50 %`.** This is **ADJ-12's own escape hatch firing on its own
pre-registered threshold**, reached exactly as §10.0.3 pre-declared the 100 %-sentinel case would reach it.
**It is a PROTOCOL EXIT, not a third branch:** the `> 50 %` literal, the Step 2 / Steps 3–4
exact-complement partition and the evaluation order are untouched, no threshold moved, and no `ADJ-21` is
authored.

**WHAT THE WALK DID NOT DO, named because each would be out of contract.** No observation target moved a
verdict — the five altered no Step, no threshold, no ordering and no conditional, no limb/step/slot above
reads any of their tables, and **no identifier of theirs is written into this walk or into §10.2.4's
bundle** (R1.3, checklist 14). The ADJ-7 boundary was not crossed: `L` and `P` are PRE-drain,
`indexed_refs` is POST-drain, and `P / indexed_refs` and `L / indexed_refs` are computed nowhere.
ADJ-11's `max(L) = 0` clause table is **not triggered** on either replicate (`max(L) = 1,000 ≠ 0` on both),
so neither the eligibility-bound-evidence clause nor the under-sampled-INADMISSIBLE clause is engaged;
clause 0 is limb (d) and clause 1 is limb (b), and both hold on both. No row-index split and no
`current_epoch`-midpoint split: every window is a coordinate on `elapsed_secs`.

---

#### §10.2.3 — THE DETERMINATION

> ### `INDETERMINATE` — on **BOTH** replicates, via **Step 0 limb (c)**, at Step 5.

**ADJ-4'S BOUND, QUOTED AND NOT PARAPHRASED.** No mechanism is named by this determination. **Nothing here
is *"the cause"***; the mechanism by which the OR tombstone prune falls behind at
`TOPGUN_EPOCH_WIDTH=1000` **remains unclassified among the FOUR PRE-REGISTERED mechanisms** —
SELECTION/FRONTIER (exit limb), SELECTION/FRONTIER (licensing limb), SCHEDULING/LICENSING and THROUGHPUT —
**no one of the four is endorsed, no one of the four is excluded, and a mechanism OUTSIDE the four is NOT
excluded.** The residual evidence that would indicate one — per-pass cost growth, index-scan cost,
allocator and page-cache effects — **is not observed by this instrument**, and this record does not pretend
otherwise. Any wording anywhere downstream that reads *"the best-supported mechanism is X"* is out of
contract on this outcome, because **no numbered step was evaluated at all.**

**§8.3, QUOTED VERBATIM AS A BLOCK QUOTE, ADJACENT TO THE DETERMINATION** — §8.3's own last sentence
requires it of any Step-5 outcome, and P9's second leg grades it here:

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

---

#### §10.2.4 — ADJ-3's UNCONDITIONAL REPORTING BUNDLE, PER REPLICATE

**Due WHATEVER step fires** — *"there was nothing to publish"* is not one of its options. FIVE limbs:
ADJ-3's four, plus R8.1a's fifth, carried on SPEC-356b R8.1a's authority and not on ADJ-3's (R1.2 item 11).

**(i) THE PASSES-PER-EPOCH COORDINATE-SLICE FIT.** Produced by the **UNFORKED** pinned fitter at
`-v col=passes_total -v xaxis=current_epoch -v window=full` over both derived **coordinate** slices,
reported as **RAW `slope_per_x_unit`** — the fitter renames its output fields under an explicit `xaxis`
precisely so a per-epoch slope can never be misread as a per-hour one.

| | early slice | late slice | late / early |
|---|---|---|---|
| **r1** | **998.978686** | **998.812325** | 0.999834 |
| **r2** | **998.875930** | **998.977301** | 1.000102 |

**FLOOR LEG (ADJ-15 / ADJ-16, FLOOR-ONLY — the SE leg is REMOVED here; no standard error, no significance
test and no threshold is computed on this leg):** `s_early ≥ 5` on **both** replicates (998.978686 and
998.875930), so **ADJ-16's degeneracy guard does NOT fire** and neither window is coarsened.
**SENSITIVITY at the pre-registered `k = 0.4 / 0.5 / 0.6`:** `s_late ≤ k · s_early` is **NOT satisfied at
any of the three `k` on either replicate**, so there is **no flip anywhere in the band** and ADJ-3's
`CONTESTED` label does not attach on this leg. *(It could not attach in any case — `CONTESTED` fires only
on a Step-1 or Step-2 determination, and neither replicate reached a numbered step.)*

**(ii) `min(L)` / `median(L)` / `max(L)`, AND THE `L ≤ B` FRACTION.** `L` = `topgun_or_prune_eligible_refs`,
**PRE-drain** (ADJ-7), over the coordinate last half, n = 1,440 rows per replicate:

| | `min(L)` | **`median(L)`** | `max(L)` | rows with `L = 0` |
|---|---|---|---|---|
| **r1** | 0 | **1,000** | 1,000 | 719 / 1,440 = **49.930556 %** |
| **r2** | 0 | **0** | 1,000 | 775 / 1,440 = **53.819444 %** |

**ADJ-2 GOVERNS THE READING: `median(L)` is the ONLY aggregator that decides; `min(L)` is REPORTED and does
NOT decide.** Neither aggregator decided anything on this round — Step 0 fired first — but the attribution
is stated so that a later reader cannot mistake `min(L) = 0` for a partition input. `L` is **two-valued**
on both replicates (0 or 1,000 refs at width 1000, with no intermediate value), so `median(L)` is settled
entirely by which side of 50 % the zero rows fall on.

**THE `L ≤ B` FRACTION IS `UNDEFINED` ON BOTH REPLICATES**, because `B` is undefined (limb (iii)). It is
**not 0 %, not 100 %, and not substituted** — a fraction against an undefined bound has no referent, and
emitting a number here would publish exactly the coercion §10.0.3 point 1 forbids for `B` itself.

`P` (`ineligible_refs`, PRE-drain, the same snapshot instant as `L`), reported and **never** divided by
`indexed_refs`:

| | first | last | min | max | direction |
|---|---|---|---|---|---|
| **r1** | 1,032 | 394 | 266 | 1,167 | **falling** across the window |
| **r2** | 424 | 555 | 339 | 1,171 | **rising** across the window |

**(iii) `B`, WITH ADJ-12 PROVENANCE AND ITS EXCLUDED FRACTION.** Source, and no other estimator is
admissible: the **median over the coordinate last-half rows of committed column 43,
`topgun_or_prune_drain_refs_p50`** — the exporter-rendered p50 of the batch-size summary over its committed
`3 × 20 s = 60 s` rolling window, **0-sentinel rows EXCLUDED**. `B` was never derived from
`Δ_sum / Δ_count`; ADJ-12 rejects that estimator and it is used for `B` nowhere.

| | the literal published | `B_SENTINEL_EXCLUDED_PCT` | non-sentinel rows the median would rest on | window rows |
|---|---|---|---|---|
| **r1** | **`B = UNDEFINED (100 % sentinel, 0 non-sentinel rows)`** | **100.000000** | **0** | 1,440 |
| **r2** | **`B = UNDEFINED (100 % sentinel, 0 non-sentinel rows)`** | **100.000000** | **0** | 1,440 |

`B` is **EMPTY — never `0`, never coerced**: `0` **is** the sentinel value, and publishing it would publish
the sentinel as the estimate. **`MEDIAN_L_OVER_B` is WITHHELD WITH THE REASON PRINTED** — verbatim
`withheld: 0 non-sentinel rows; B undefined` — not silently absent, because a bare suppression is
indistinguishable from a miss (PD-3). **This is §10.0.3's pre-declared reporting disposition, discharged on
both replicates, and P2's `≥ 1 value row` limb ACCEPTS this literal as its value row.**

**PROVENANCE CAVEAT ON THIS LIMB, STATED RATHER THAN PAPERED OVER.** The three figures above were
re-derived **directly from each replicate's committed column 43** — which is what ADJ-12 itself defines —
and **not** by the digest-pinned v2 driver, because **the driver refused to run on either replicate**. The
full statement of that refusal, its localisation and its routing is §10.2.6 finding 1. **The determination
is unaffected: the predicate defines the quantity and the driver is a re-derivation convenience.** But the
re-derivation **layer** is unavailable for these cells, and that is said plainly here rather than left to
be noticed.

**(iv) THE NON-DROP EXIT SHARE AND THE PER-EXIT BREAKDOWN, WITH ITS DENOMINATOR BESIDE IT.** PD-4's
coupling: **the two statistics degrade together and NEITHER is quoted without the other.**

| Exit, Δ over the coordinate last half | r1 | r2 |
|---|---|---|
| `topgun_or_prune_matched_nothing_total` | 0 | 0 |
| `topgun_or_prune_absent_total` | 0 | 0 |
| `topgun_or_prune_restored_read_error_total` | 0 | 0 |
| `topgun_or_prune_restored_evicted_total` | 0 | 0 |
| `topgun_or_prune_restored_write_error_total` | 0 | 0 |
| **non-drop numerator** | **0** | **0** |
| `topgun_or_prune_dropped_total` | 0 | 0 |
| **`topgun_or_prune_considered_total` (THE DENOMINATOR)** | **0** | **0** |
| **NON-DROP EXIT SHARE** | **UNDEFINED — denominator 0** | **UNDEFINED — denominator 0** |

**THE SHARE IS NOT `0.000000 %`; IT IS UNDEFINED, AND THE DENOMINATOR IS WHY.** With zero non-empty drains
in the window, **no ref was ever considered**, so the share has no referent at all. **This is PD-4's
coupling at its limit and it must be stated as such:** on the prior arm the same share read
**`0.000000 %` over a denominator of 2,000** (two drains × 1,000 refs, every one `Dropped`); here the
**denominator itself has collapsed to 0**, and quoting a percentage without it would present *an absence of
observation* as *an observation of absence*. Step 1's `> 10 %` predicate would therefore have been
unevaluable on both replicates even had Step 0 admitted the window — recorded for completeness, and **Step
1 was NOT EVALUATED regardless, because Step 0 precedes it.**

**(v) ADJ-11's DIAGNOSTIC COLUMN — `Δpasses` BESIDE THE WINDOW SCRAPE COUNT.**

| Quantity | r1 | r2 |
|---|---|---|
| **`Δpasses` over the window** | **414,110** | **445,682** |
| **window scrape count** | **1,440** | **1,440** |
| **ratio (passes per 10 s scrape)** | **287.58** | **309.50** |
| `Δempty_drains` | 414,110 | 445,682 |
| `Δnonempty_drains` | **0** | **0** |
| `Δrestored_read_error_total` / `Δrestored_write_error_total` | 0 / 0 (both flat) | 0 / 0 (both flat) |

The under-sampling regime is **VISIBLE rather than inferred**, which is this limb's whole purpose: at ~288
and ~310 passes per 10 s scrape, a scrape-level gauge sample of `L` bounds nothing about the passes between
scrapes, and a reader shown both numbers can tell that apart from a window in which the prune genuinely
idled.

**BACKLOG CONTEXT (ADJ-14's named series), reported beside the bundle:**

| Δ over the coordinate last half | r1 | r2 |
|---|---|---|
| `Δtombstone_bytes` | 9,534,075 | 10,260,944 |
| `Δbytes_freed` | **0** | **0** |
| **`Δbacklog_bytes` (ADJ-14)** | **9,534,075** | **10,260,944** |
| whole-run reclaim fraction | 0.042618 | 0.087700 |
| `Δlow_water_mark` over the window | 414 | 446 |

**Units differ on purpose and are NOT reconciled:** `indexed_refs`, `L` and `P` are **ref counts**,
`tombstone_bytes` is **bytes**, and **no bytes-per-ref constant is assumed, fitted or implied** anywhere.

---

#### §10.2.5 — THE n = 2 REPLICATION STATEMENT (R2.4)

**This replaces §2.6's n = 1 caveat rather than deleting it.**

**WHICH KIND OF STATISTIC DECIDED: A COUNT.** The determination on both replicates was reached by Step 0
limb (c), whose deciding quantity is a **count of rows** — the number of coordinate-last-half rows of
committed column 43 carrying the 0-sentinel, against the window's row count. It is
**1,440 / 1,440 = 100.000000 %** on **both** replicates, with **0** non-sentinel rows on both. **No LEVEL
decided** (`median(L)`, `B` and the exit share were all either undefined or never reached) and **no SLOPE
decided** (the pass-rate fit is ADJ-3's unconditional evidence, not a step input, and Steps 3 / 4 were never
evaluated).

**DO THE TWO REPLICATES AGREE? ON THE DECIDING STATISTIC, YES — EXACTLY.** Both route to `INDETERMINATE`
via the same limb, at the same limb-(c) leg, on the identical measured value, with limbs (d), (a) and (b)
all holding on both. **The determination replicates.**

**WHAT DOES NOT REPLICATE — REPORTED AS DISAGREEMENT, NEVER AVERAGED AWAY:**

| Quantity | Kind | r1 | r2 | Read |
|---|---|---|---|---|
| **`median(L)`** | **LEVEL** | **1,000** | **0** | **DISAGREES — opposite sides of ADJ-2's aggregator.** Zero-row fractions 49.930556 % vs 53.819444 %: a ~3.9-point shift across the median's own knife-edge on a two-valued series. See §10.2.6 finding 2 |
| non-empty drains, whole run | COUNT | 39 | 80 | **DISAGREES, ~2.05×.** Both are the front-loaded startup regime; the whole-run count is published and is never a discriminant |
| non-empty drains, coordinate last half | COUNT | **0** | **0** | **AGREES**, and it is 0 on both against the prior arm's 2 |
| `Δpasses` per scrape | LEVEL | 287.58 | 309.50 | agrees to ~7 % |
| `Δbacklog_bytes` over the window | LEVEL | 9,534,075 | 10,260,944 | agrees to ~7.6 % |
| whole-run reclaim fraction | LEVEL | 0.042618 | 0.087700 | **DISAGREES, ~2.06×** |
| `P` trajectory over the window | direction | **falling** (1,032 → 394) | **rising** (424 → 555) | **DISAGREES IN DIRECTION** |
| pass-rate late/early ratio | SLOPE | 0.999834 | 1.000102 | agrees; both essentially flat, opposite in sign of the negligible departure |
| `current_epoch == low_water_mark` rate, coordinate half | LEVEL | 0.965278 | 0.972917 | agrees to ~0.8 % |

**NO `t`, NO α, NO SE LEG AND NO SIGNIFICANCE CLAIM IS COMPUTED OVER n = 2 FOR ANY QUANTITY.** The two
replicates are printed side by side; agreement is reported as agreement and disagreement as disagreement.
This follows SPEC-355 §10.4.2's lesson — the level replicates and the slope does not — and this round adds
a case of its own: **a knife-edge LEVEL did not replicate**, which is exactly the fragility §8.1 doubled
the replicates to expose.

---

#### §10.2.6 — THE TWO POST-DATA FINDINGS, RECORDED AND ROUTED

**NEITHER IS REPAIRED, AND THAT IS THE RULE RATHER THAN A CHOICE.** The data boundary is `0b96e63e`
(§10.1.5); both findings were taken after it; **R0.3 is categorical** — after the boundary a finding is
*"a POST-DATA RECORD routed to a named follow-on. It is never a predicate edit, never an addendum, and
never a checklist patch."* **No sidecar was edited, no cell re-run, no checklist patched, no `ADJ-21`
authored, and no byte of §0–§8, §8A, §9 or §10.0 changed.**

**FINDING 1 — `spec356c-slottruth-v2.sh` REFUSED RUN 3 ON BOTH REPLICATES.**
*Observed:* the digest-pinned v2 driver, verified unedited against §10.0.6
(`a95f49fa…a348`) **before** execution, exits **3** on both cells at provenance limb **(d)(i)**:

```
RUN 3 / replicate r1 :  SLOTTRUTH FAIL: spec356c-long-r1.matrix.txt does not name cell long    exit=3
RUN 3 / replicate r2 :  SLOTTRUTH FAIL: spec356c-long-r2.matrix.txt does not name cell long    exit=3
```

*Cause, localised rather than guessed:* the subject's `prov` call parameterises the **basename**
(`"${2:-spec356-long}"`) and the **duration** (`"${3:-14400}"`) but leaves the **cell-name literal `long`
hardcoded** —

```
-prov spec356-long      long   "<unset: production default 1000>" 14400  "$PR"                      10
+prov "${2:-spec356-long}" long   "<unset: production default 1000>" "${3:-14400}" "$PR"     10
```

— while the repeat's cells self-describe as `long8h_r1` / `long8h_r2`. **R4.2's own class-B table named all
THREE literals of the provenance triple** — *"`:77` the subject cell's provenance triple — basename, cell
name and the `14400` duration literal"* — **so the forced set was right and the implementation missed one
of the three.** *Localised by a control run of the same driver bytes on the default subject over the prior
arm, which completes clean (exit 0), so the refusal is specific to the parameterised subject and not to the
driver.*
*What P8 does and does not see, stated because the gap is instructive:* the class-B mechanical test asks
that the line be **touched in place with zero added lines**, and this hunk is. **A line-level taxonomy
cannot see a line that was parameterised INCOMPLETELY**, so P8 grades GREEN correctly and this defect is
outside its reach by construction — a fact about the taxonomy's resolution, recorded, not a charge against
P8.
*Consequence, stated rather than worked around:* **AC7's run-3 leg and Validation Checklist item 12's
run-3 leg are RED** — R4.3's run 3 is **structurally unrunnable as committed**. **R4.6 / §10.0.3's
empty-population branch was therefore NOT exercised by the driver on the repeat's real bytes**; it stays
demonstrated where §10.0.3 already records it, on the synthetic fixtures of
`spec356c-slottruth-v1-v2-runs.txt`. Limb (c) leg 2's and bundle limb (iii)'s inputs were re-derived
**directly from committed column 43, which is what ADJ-12 itself defines**, with the caveat stated at limb
(iii). **The determination is unaffected — the predicate defines the quantity, and the driver is a
re-derivation convenience — but the re-derivation LAYER is unavailable for the repeat's cells. That is
PD-3's shape recurring one level up:** the round before, a suppression was indistinguishable from a miss;
this round, the channel built to make it distinguishable did not run on the bytes that mattered.
*Routing:* **`TODO-648`** (the pinned-sidecar defect class — a defect in a committed, digest-pinned artifact
that post-data rules forbid repairing) **and `TODO-634`** (as a design-phase input: the next instrument
round parameterises the provenance triple completely, or the grader's own provenance limb is what blocks
the grading).
*The limitation of this routing, stated instead of implied:* **no tracker byte is written by this spec for
this finding. The §10 record IS the routing.** A reader who wants the loop closed must open `TODO-648`
and `TODO-634` and find it there, and it will only be there if somebody puts it there. **This section does
not claim a closed loop and must not be read as one.** *(The single tracker edit this spec does make is
AC15's update to `TODO-634.md`'s §8.1-repeat box with the outcome — an outcome update, not a repair of any
finding, and graded at P13's POST half.)*

**FINDING 2 — `median(L)` DID NOT REPLICATE.**
*Observed:* `median(L)` over the coordinate last half is **1,000 on r1** and **0 on r2**. The mechanism is
fully visible in the committed bytes: `L` is **two-valued** at width 1000 — 0 or 1,000 refs, with no
intermediate value — and the zero-row fraction **straddles 50 %**, at **49.930556 %** on r1 (719 / 1,440,
median 1,000) and **53.819444 %** on r2 (775 / 1,440, median 0). **A ~3.9-point shift moves the median the
whole width of the series.**
*What it decided this round: NOTHING.* Step 0 fired first on both replicates and no numbered step was
evaluated, so no determination rests on it.
*Why it is a finding anyway, and not a curiosity:* **`median(L)` is the Step 2 / Steps 3–4 EXACT-COMPLEMENT
partition input.** On an admissible window — one where the deciding column is readable and Step 0 lets the
walk through — **these two replicates of the same configuration, at the same pin, on the same binary, would
have partitioned DIFFERENTLY**, on a quantity whose two possible values sit at the extremes of its own
range. That is a fragility in the predicate's own partition input, exposed exactly as intended: **§8.1
doubled the replicates in order to see this, and at n = 1 it would have been invisible.**
*What is NOT concluded:* no threshold is proposed, no aggregator is changed, `min(L)` does not become a
decider, ADJ-2 is untouched, and nothing about the four pre-registered mechanisms follows from it.
*Routing:* **`TODO-634`**, as a stated open input to the design phase — the same limitation applies as
above: the record is the routing.

---

### §10.3 — T1(b): THE LWM-PASS LEDGER, PER REPLICATE — AND T1(a)'s OUT-OF-SCOPE ROW

**Status, restated before a single number is read: this is an OBSERVATION (R1.3).** Nothing in this
subsection alters a Step, a threshold, an ordering or a conditional; **no limb, step, slot or leaf of
§10.2's walk reads any column below**, and the determination in §10.2.3 stands exactly as walked whether or
not this table exists.

**Artifacts:** `spec356c-t1-lwmpass-r1.csv` (**414** data rows) and `spec356c-t1-lwmpass-r2.csv` (**446**),
both produced by the single committed builder `spec356c-targets.sh` (R3.4), one run per replicate, over the
**coordinate last half** `[14,410 … 28,800]`.

**THE RESOLUTION CAVEAT (C1), CARRIED HERE AND NOT ONLY IN THE CSV HEADER.** LWM advances one epoch at a
time roughly every 31 s; the ledger scrapes every 10 s. **Every "at the moment LWM passes" reading below is
therefore the FIRST SCRAPE AT OR AFTER the advance — a BUCKET, not an instant, with up to one full cadence
of lag.** No row below is an event record.

| Quantity | **r1** | **r2** |
|---|---|---|
| LWM-pass rows | **414** | **446** |
| first / last pass bucket (`t_secs`) | 14,440 s / 28,760 s | 14,440 s / 28,790 s |
| `lwm_before` (first) → `lwm_after` (last) | **460 → 874** | **460 → 906** |
| `epochs_passed` | **1 on all 414 rows** (max jump 1; published, not assumed) | **1 on all 446 rows** |
| `EVENTS_IN_BUCKET` (C3) | **1 on all 414 rows** — no multi-event bucket occurred, so nothing was smoothed | **1 on all 446 rows** |
| `indexed_refs` (**POST-drain**) | 1,010 … 1,458 | 1,007 … 1,439 |
| `indexed_epochs` (**POST-drain**) | **2 at every pass row** | **2 at every pass row** |
| `refs_per_indexed_epoch` — **a MEAN over the index, NOT a per-epoch value** | 505.000000 … 729.000000 | 503.500000 … 719.500000 |
| `eligible_refs` (`L`, **PRE-drain**) | **0 at every pass row** | **0 at every pass row** |
| `ineligible_refs` (`P`, **PRE-drain**) | 1,001 … 1,167 | 1,001 … 1,171 |
| Σ `d_tombstone_bytes_since_prev_pass` | **9,494,354 B** | **10,230,929 B** |
| Σ `d_bytes_freed_since_prev_pass` | **0 B** | **0 B** |
| Σ `d_nonempty_drains_since_prev_pass` | **0** | **0** |

**Sampled rows, so the table above is checkable against the committed bytes rather than trusted** (row
index into the CSV's data rows; columns in header order):

```
r1   1: 14440,460,461,1,461,1361,2,680.500000,0,1037,,,,1
r1 103: 17650,562,563,1,563,1222,2,611.000000,0,1082,21275,0,0,1
r1 207: 20990,666,667,1,667,1387,2,693.500000,0,1036,29762,0,0,1
r1 310: 24370,769,770,1,770,1116,2,558.000000,0,1045,20424,0,0,1
r1 414: 28760,873,874,1,874,1158,2,579.000000,0,1018,22839,0,0,1

r2   1: 14440,460,461,1,461,1373,2,686.500000,0,1070,,,,1
r2 111: 17850,570,571,1,571,1060,2,530.000000,0,1018,22701,0,0,1
r2 223: 21410,682,683,1,683,1271,2,635.500000,0,1021,20493,0,0,1
r2 334: 25040,793,794,1,794,1257,2,628.500000,0,1006,28129,0,0,1
r2 446: 28790,905,906,1,906,1195,2,597.500000,0,1016,20424,0,0,1
```

*(The three empty delta fields on row 1 of each replicate are the builder's honest empty: there is no
previous pass row to difference against. R3.4 forbids a substitute, and none was emitted.)*

**WHAT THE LEDGER SAYS, IN DIRECTION ONLY.** Across the whole deciding window the LWM walked **414** epochs
on r1 and **446** on r2 while `d_bytes_freed_since_prev_pass` was **0 at every single pass row on both
replicates**, and `indexed_epochs` sat at **2** throughout — the index never grew a backlog of epochs
behind the watermark, and the watermark never passed an epoch at a moment when a drain was crediting bytes.
**This addresses hypothesis (a) in DIRECTION ONLY.** The ledger **cannot distinguish** *"epoch e's refs
were never indexed"* from *"they were indexed and left the index earlier"*, because the two gauges it reads
are aggregates over all indexed epochs. **A trajectory is not a membership test, and this sentence exists
so it is not read as one.**

**THE ADJ-7 BOUNDARY IS NOT CROSSED.** `eligible_refs` (`L`) and `ineligible_refs` (`P`) are **PRE-drain**;
`indexed_refs` is **POST-drain**. They are published side by side and **never divided across that
boundary**: `P / indexed_refs` and `L / indexed_refs` are computed nowhere in this subsection, in the
committed CSV, or anywhere in §10.

**UNITS DIFFER ON PURPOSE AND ARE NOT RECONCILED.** `indexed_refs` is a **ref count**; `tombstone_bytes` is
**bytes**. **No bytes-per-ref constant is assumed, fitted or implied** — the two are published beside each
other and the reader is told, here, that they are different units.

#### §10.3.1 — T1(b) AND T5 ARE **ONE** OBSERVATION, NOT TWO — WITH THE IDENTITY'S MEASURED RATE (AC8)

**The one sentence, stated plainly:** under the `current_epoch ≡ low_water_mark` identity, **T1(b)'s
LWM-pass rows and T5's epoch rows are the SAME rows reached by two routes** — T5's `lwm_passed_at_t`
degenerates to `t_last_seen` and T1(b)'s pass rows are a projection of T5's epoch rows — so **agreement
between §10.3's table and §10.7's ledger is a RESTATEMENT, NOT CORROBORATION, and the diagnostic value is
banked ONCE.**

**The identity is NEAR-TOTAL, NOT TOTAL, and the measured rate is published per replicate so the reader
knows which regime they are reading** — re-derived here from the committed prune ledgers (columns 22 and
23), not carried over from the prior arm:

| `current_epoch == low_water_mark` equality rate | **r1** | **r2** |
|---|---|---|
| full ledger | **2,779 / 2,881 = 0.964596** | **2,796 / 2,880 = 0.970833** |
| **coordinate last half (the deciding window)** | **1,390 / 1,440 = 0.965278** | **1,401 / 1,440 = 0.972917** |

**~3.5 % of rows on r1 and ~2.7 % on r2 break the identity**, so the projection is a *near*-projection: the
row counts differ by exactly one on each replicate (414 pass rows vs 415 epoch rows on r1; 446 vs 447 on
r2), and the extra epoch row is the window's last epoch, which the watermark had not yet passed when the
ledger ended. **The identity did NOT break into independence** — it is a near-total identity in both
replicates — and that is what is claimed, no more.

#### §10.3.2 — **T1(a) — OUT-OF-SCOPE.** THE ROW, THE REASON, THE OWNER

| Target | Disposition | Reason | Owner |
|---|---|---|---|
| **T1(a)** — per-epoch index membership (*"was epoch e's content EVER in the drainable index"*) | **OUT-OF-SCOPE** | The committed surface exposes `topgun_or_prune_indexed_refs` and `_indexed_epochs` as **aggregate gauges over all indexed epochs**; **no committed column, and no derivation from committed columns, can attribute a ref to an epoch.** Producing it needs a **new per-epoch labelled emission on the prune path — a `.rs` change — which R0.4 forbids CATEGORICALLY**, because taking it would destroy the **frozen-inherited build identity that is the entire point of the repeat** | **`TODO-634`** |

**T1(a) IS NOT NARROWED INTO T1(b), AND T1(b) IS NOT PRESENTED AS ITS ANSWER.** §10.3's table answers
*"what did the AGGREGATE index look like as the watermark walked past"*. It does **not** answer *"were
epoch e's refs in the index at the moment the watermark passed e"*. That question is unanswered by this
round, is recorded as unanswered here, and is carried to `TODO-634`'s design phase as the **single
highest-value instrument addition the next round can make** (§10.8 item 6 routes it).

---

### §10.4 — T2: FULL-LEDGER ENUMERATION OF EVERY NON-EMPTY DRAIN — AND T2(exactness)'s OUT-OF-SCOPE ROW

**Status: an OBSERVATION (R1.3).** No limb, step, slot or leaf reads it.

**THE WINDOW IS THE FULL LEDGER, NAMED BEFORE THE NUMBERS.** T2's own words are *"FULL content enumeration
of every non-empty drain"*, so this table spans **every row from the ledger's first `elapsed_secs` to its
last** — **not** the coordinate last half. R3 pre-declared that the two readings differ by more than 25× on
the prior arm; they differ here too, and §10.6 is where the last-half count does its work.

**Artifacts:** `spec356c-t2-drains-r1.csv` (**39** data rows) and `spec356c-t2-drains-r2.csv` (**80**).

| Quantity, over the FULL ledger | **r1** | **r2** |
|---|---|---|
| non-empty drain rows | **39** | **80** |
| first / last drain bucket | 60 s / **10,470 s** | 60 s / **13,470 s** |
| `drains_in_bucket` | **1 on all 39 rows** — every drain is separable; **no bucket carried more than one event**, so C3's non-smoothing clause never had to bite | **1 on all 80 rows** |
| `last_drained_epoch` first-before → last-after | **0 → 332** | **2 → 428** |
| Σ `d_drain_refs_sum` / Σ `d_drain_refs_count` — **§5.4's PRIMARY reading for a THROUGHPUT quantity** | 39,000 / 39 = **1000.000000 refs per drain**, and the per-row quotient is **exactly `1000.000000` on all 39 rows** | 80,000 / 80 = **1000.000000**, exactly `1000.000000` on all 80 rows |
| Σ `d_drain_epochs_sum` / Σ `d_drain_epochs_count` | 39 / 39 = **1.000 epoch per drain**, **on every row** | 80 / 80 = **1.000 epoch per drain**, **on every row** |
| Σ `d_epoch_considered_sum` | 39,000 | 80,000 |
| Σ `d_epoch_dropped_sum` | 39,000 — **considered == dropped on every drain** | 80,000 — **considered == dropped on every drain** |
| `d_epoch_bytes_freed_sum` per drain | 19,820 … 23,000 B | 19,819 … 23,000 B |
| Σ `d_epoch_bytes_freed_sum` | **848,707 B** (mean 21,761.7 B/drain) | **1,810,354 B** (mean 22,629.4 B/drain) |
| Σ `d_bytes_freed_total` | **848,707 B** | **1,810,354 B** |
| **`bytes_freed_matches_attribution`** | **`true` on 39 / 39 rows** | **`true` on 80 / 80 rows** |

**Sampled rows** (header order), so the summary is checkable:

```
r1 first: 60,1,1000,1,1000.000000,1,1,0,2,1000,1000,19820,1,19820,true
r1  last: 10470,1,1000,1,1000.000000,1,1,331,332,1000,1000,23000,1,23000,true
r2 first: 60,1,1000,1,1000.000000,1,1,2,2,1000,1000,19819,1,19819,true
r2  last: 13470,1,1000,1,1000.000000,1,1,428,428,1000,1000,23000,1,23000,true
```

**THE IDENTITY COLUMN MATCHED EVERYWHERE — AND ITS BOUND IS STATED HERE RATHER THAN LEFT TO BE ASSUMED,
BECAUSE A UNANIMOUS MATCH IS EXACTLY THE RESULT MOST EASILY OVER-READ.**
`bytes_freed_matches_attribution` is **an INTERNAL CONSISTENCY CHECK BETWEEN TWO COMMITTED COUNTERS** —
`Δtopgun_or_prune_bytes_freed_total` against `Δtopgun_or_prune_epoch_bytes_freed_sum`. **It CANNOT compare
either counter against an independent ground truth of an epoch's true tombstone byte size**, because no
committed column carries one. Therefore:

- **a MISMATCH would have been evidence of an accounting gap** — and there was none, on 119 drains across
  two replicates;
- **the MATCH excludes ONLY the gap between those two counters.** It does **not** exclude every reclaim
  path, it does **not** establish that the attributed bytes equal the epoch's true tombstone bytes, and it
  is **not** a test of hypothesis (c) at exactness. It is the internal-consistency identity it is named
  after, and it is reported as nothing more.

**WHERE THE DRAINS SIT IN TIME — AN OBSERVATION, NOT A RE-DERIVATION OF ANY LIMB.** Every one of the 39
drains on r1 (last at **10,470 s**) and every one of the 80 on r2 (last at **13,470 s**) falls **strictly
before the coordinate last half's lower bound of 14,410 s**. §10.2.2's limb (c) was evaluated on the prune
ledger's own committed column 43 and reads **nothing** from this table; the coincidence is noted here
because a reader will see it, and naming it is cheaper than letting it look like a routing that happened
off-record. **It routes nothing.**

**THE STARTUP BURST IS VISIBLE IN THIS TABLE AND THE TWO REPLICATES DISAGREE ABOUT ITS SHAPE.** On r1,
**28 of 39** drains land in the first 1,000 s window and the remaining 11 by 10,470 s. On r2, only **15 of
80** land in the first window, with the bulk — **61 of 80** — arriving in the six windows spanning
7,000 s … 12,990 s, i.e. **in the middle of the run rather than at its start**. **Same
configuration, same binary, same pin — and a materially different distribution of drains in time.** That
disagreement is reported as disagreement; §10.6 is where it disqualifies the whole-run count as a
discriminant.

#### §10.4.1 — **T2(exactness) — OUT-OF-SCOPE.** THE ROW, THE REASON, THE OWNER

| Target | Disposition | Reason | Owner |
|---|---|---|---|
| **T2(exactness)** — *"was `bytes_freed` incremented by the EXACT tombstone size"*, against an **independent** ground truth of the epoch's true byte size | **OUT-OF-SCOPE** | **No committed column carries an independent ground truth for an epoch's true tombstone byte size.** `topgun_ormap_tombstone_bytes_total` is a **whole-store monotone total**, not a per-epoch size; `topgun_or_prune_epoch_bytes_freed_sum` is **the prune path's OWN attribution — i.e. the very quantity under suspicion.** Comparing them tests the path's internal consistency, never its correctness. An exactness claim needs an external size oracle, which is a **per-epoch byte-size emission on the tombstone path — a `.rs` change — which R0.4 forbids CATEGORICALLY** | **`TODO-634`** |

**THE `bytes_freed_matches_attribution` COLUMN IS NOT PRESENTED AS THIS ROW'S ANSWER.** A stated narrowing
that carries no row is a silent one; this row is carried so the narrowing is not silent, and the 119
unanimous `true` values above are **not** reported as exactness anywhere in §10.

---

### §10.5 — T3: Δ(LWM) vs Δ(bytes_freed), WINDOWED EVERY 1,000 s, BOTH REPLICATES SIDE BY SIDE

**Status: an OBSERVATION (R1.3).** No limb, step, slot or leaf reads it.

**Artifacts:** `spec356c-t3-windows-r1.csv` and `spec356c-t3-windows-r2.csv`, **29 data rows each —
28 `FULL` + 1 `PARTIAL`**, exactly the shape R3's tail rule pre-declared for a 28,800 s ledger
(`28,800 / 1,000 = 28.8`).

**ORIGIN AND TAIL, AS EXECUTED.** The builder's origin rule is a literal — `t0` := the ledger's **first
`elapsed_secs` value** — and the two replicates resolve it differently because their ledgers begin
differently: **r1's first scrape is `elapsed_secs = 0`** (2,881 rows) and **r2's is `elapsed_secs = 10`**
(2,880 rows). **The rule is identical; only its resolved literal differs**, so r2's window *k* covers
`[10 + 1000k, 10 + 1000(k+1))` and its bounds sit **+10 s** from the column printed below (which shows
r1's). Both partitions yield **28 `FULL` windows of 100 rows each plus one marked `PARTIAL` tail** —
81 rows on r1 (`28,000 … 28,800`), 80 on r2 (`28,010 … 28,800`).

**THE `PARTIAL` ROW IS EMITTED, MARKED, AND EXCLUDED FROM EVERY RATE.** It is row `w_index = 28` below, it
carries `window_kind = PARTIAL`, and **no per-hour rate anywhere in §10.5 or §10.6 is computed from it**
(§10.6's `SUBJECT_WHOLE_RUN` row carries the exclusion in its own `rate_excluded_reason` column).

| `w` | kind | window (r1 bounds; r2 = +10 s) | r1 ΔLWM | r1 Δfreed | r1 Δbacklog | r1 ND | r2 ΔLWM | r2 Δfreed | r2 Δbacklog | r2 ND | drains disagree? |
|---|---|---|---|---|---|---|---|---|---|---|
| 0 | `FULL` | 0–990 | 33 | 599707 | 85250 | 28 | 30 | 315354 | 333771 | 15 | **28 vs 15** |
| 1 | `FULL` | 1000–1990 | 32 | 88000 | 604054 | 4 | 30 | 0 | 665764 | 0 | **4 vs 0** |
| 2 | `FULL` | 2000–2990 | 31 | 0 | 685828 | 0 | 31 | 0 | 673090 | 0 | — |
| 3 | `FULL` | 3000–3990 | 31 | 0 | 684222 | 0 | 31 | 0 | 679624 | 0 | — |
| 4 | `FULL` | 4000–4990 | 31 | 0 | 691812 | 0 | 31 | 23000 | 677463 | 1 | **0 vs 1** |
| 5 | `FULL` | 5000–5990 | 32 | 0 | 726202 | 0 | 32 | 0 | 734045 | 0 | — |
| 6 | `FULL` | 6000–6990 | 31 | 0 | 726754 | 0 | 32 | 46000 | 692300 | 2 | **0 vs 2** |
| 7 | `FULL` | 7000–7990 | 32 | 0 | 737403 | 0 | 33 | 138000 | 608281 | 6 | **0 vs 6** |
| 8 | `FULL` | 8000–8990 | 32 | 0 | 735011 | 0 | 32 | 437000 | 311926 | 19 | **0 vs 19** |
| 9 | `FULL` | 9000–9990 | 33 | 0 | 745039 | 0 | 33 | 207000 | 541834 | 9 | **0 vs 9** |
| 10 | `FULL` | 10000–10990 | 32 | 161000 | 583786 | 7 | 32 | 207000 | 538614 | 9 | **7 vs 9** |
| 11 | `FULL` | 11000–11990 | 32 | 0 | 738944 | 0 | 33 | 207000 | 544686 | 9 | **0 vs 9** |
| 12 | `FULL` | 12000–12990 | 32 | 0 | 736690 | 0 | 32 | 207000 | 537717 | 9 | **0 vs 9** |
| 13 | `FULL` | 13000–13990 | 32 | 0 | 739749 | 0 | 33 | 23000 | 721855 | 1 | **0 vs 1** |
| 14 | `FULL` | 14000–14990 | 32 | 0 | 739266 | 0 | 32 | 0 | 744395 | 0 | — |
| 15 | `FULL` | 15000–15990 | 32 | 0 | 728571 | 0 | 32 | 0 | 743429 | 0 | — |
| 16 | `FULL` | 16000–16990 | 32 | 0 | 732320 | 0 | 32 | 0 | 734896 | 0 | — |
| 17 | `FULL` | 17000–17990 | 31 | 0 | 720636 | 0 | 32 | 0 | 735862 | 0 | — |
| 18 | `FULL` | 18000–18990 | 32 | 0 | 721809 | 0 | 32 | 0 | 730664 | 0 | — |
| 19 | `FULL` | 19000–19990 | 31 | 0 | 719532 | 0 | 32 | 0 | 727766 | 0 | — |
| 20 | `FULL` | 20000–20990 | 31 | 0 | 709941 | 0 | 31 | 0 | 720705 | 0 | — |
| 21 | `FULL` | 21000–21990 | 30 | 0 | 702236 | 0 | 31 | 0 | 710884 | 0 | — |
| 22 | `FULL` | 22000–22990 | 31 | 0 | 703708 | 0 | 31 | 0 | 714449 | 0 | — |
| 23 | `FULL` | 23000–23990 | 30 | 0 | 694232 | 0 | 31 | 0 | 704444 | 0 | — |
| 24 | `FULL` | 24000–24990 | 30 | 0 | 684043 | 0 | 30 | 0 | 686044 | 0 | — |
| 25 | `FULL` | 25000–25990 | 28 | 0 | 639400 | 0 | 29 | 0 | 683997 | 0 | — |
| 26 | `FULL` | 26000–26990 | 22 | 0 | 510968 | 0 | 30 | 0 | 689977 | 0 | — |
| 27 | `FULL` | 27000–27990 | 21 | 0 | 478814 | 0 | 30 | 0 | 688666 | 0 | — |
| 28 | `PARTIAL` | 28000–28800 | 15 | 0 | 359237 | 0 | 24 | 0 | 548343 | 0 | — |

*(`ΔLWM` = `d_low_water_mark`; `Δfreed` = `d_bytes_freed_total`; **`Δbacklog` = `d_backlog_bytes`,
ADJ-14's named series, `Δtombstone_bytes − Δbytes_freed`**; `ND` = `d_nonempty_drains`. The committed CSVs
also carry `rows`, `d_tombstone_bytes_total`, `d_empty_drains`, `d_passes` and
`bytes_freed_per_lwm_epoch`, which are not reproduced here only for width.)*

**THE SHAPE — A DESCRIPTION OF THE PLOTTED SERIES, EXPLICITLY NOT A FITTED CLAIM.**
**No OLS fit, no slope, no standard error, no r², no α, no t-statistic and no significance claim is
computed on this series, on either replicate, or across the two.** With that said, and as a description
only:

- **The per-window `d_backlog_bytes` increment is essentially FLAT across the bulk of both runs** — roughly
  **0.64 – 0.75 MB per 1,000 s** on r1 from `w2` through `w25` (excluding `w10`, the one window in that
  span carrying drains, at 583,786 B), and roughly **0.67 – 0.74 MB** on r2 across the `FULL` windows in
  which nothing drained. A flat per-window increment describes a **cumulative backlog that grows
  LINEARLY**, not one that accelerates and not one that saturates. Cumulative `d_backlog_bytes` at the end
  of the `FULL` windows reads **18,706,220 B on r1** and **18,277,148 B on r2**; adding the `PARTIAL` tail
  gives **19,065,457 B** and **18,825,491 B**. On r1 that total equals `final tombstone_bytes − final
  bytes_freed` **exactly** (**19,914,164 − 848,707 = 19,065,457**). On r2 the same difference reads
  **20,642,504 − 1,810,354 = 18,832,150**, i.e. **6,659 B more than the windowed sum** — and **6,659 B is
  precisely the `tombstone_bytes` value already standing at r2's FIRST scrape** (`t = 10 s`), which no
  within-window delta can see because there is no earlier row to difference against. **r1's first scrape
  reads `tombstone_bytes = 0` at `t = 0 s`, which is why its two totals coincide.** The gap is an artefact
  of where each ledger starts, it is arithmetic rather than accounting, and it is stated rather than
  quietly rounded away.
- **The two final `FULL` windows on r1 (`w26`, `w27`) show a REDUCED increment** — 510,968 B and
  478,814 B against a ~700,000 B body — and the reduction tracks a fall in the **tombstone-add rate
  itself** (`ΔLWM` drops 31 → 22 → 21 over the same windows), **not** any reclaim: `Δfreed` is **0** in
  both, and in the `PARTIAL` tail as well. **A falling increment that is driven by a falling input rate is
  not saturation of the backlog process, and it is not described as such here.** r2 shows no comparable
  fall (`ΔLWM` holds at 30 through `w27`), which is itself a disagreement between the replicates and is
  reported as one.
- **`bytes_freed_per_lwm_epoch` is 0.000000 in 25 of 28 `FULL` windows on r1 and in 18 of 28 on r2** — the
  watermark advanced ~30 epochs per window while the prune credited **zero bytes** in those windows.

**DISAGREEMENT IS REPORTED AS DISAGREEMENT (R2.4), NOT RECONCILED.** The two replicates are the **same
configuration at the same pin on the same binary**, and their drain distributions do not match: the
`drains disagree?` column above is non-empty in **11 of 29 windows**. r1 drains in `w0` (28), `w1` (4) and
`w10` (7); r2 drains in `w0` (15), `w4`, `w6`, `w7`, `w8` (19), `w9`, `w10`, `w11`, `w12` and `w13`.
**Whole-run totals differ 39 vs 80 — a factor of ~2.05 between two replicates of one configuration.**
**No statistic is computed over n = 2, no replicate is preferred, and neither is averaged into the other.**
**What the two replicates DO agree on, in every window from `w14` onward: `Δfreed = 0` and `ND = 0` on
both.**

---

### §10.6 — T4: NON-EMPTY DRAIN RATE vs DURATION — THE ROUND'S MOST ACTIONABLE OBSERVATION

**Status: an OBSERVATION (R1.3).** It moves no Step, no threshold, no ordering and no conditional; **no
limb, step, slot or leaf reads it**, and the `INDETERMINATE` determination in §10.2.3 is unchanged by every
number below.

**Artifacts:** `spec356c-t4-rate-r1.csv` and `spec356c-t4-rate-r2.csv`, **34 data rows each** — 29
per-window rows (28 `FULL` + 1 `PARTIAL`, reusing T3's windows) and 5 summary rows, each carrying the
builder's own `discriminant_role` column.

**THE BINDING, STATED IN THE SAME BREATH AS THE NUMBERS, BECAUSE THAT IS WHERE IT IS LOAD-BEARING: THE
DISCRIMINANT IS READ *ONLY* OFF THE COORDINATE-LAST-HALF COUNT.** The whole-run and full-windows counts are
**PUBLISHED AND ARE NEVER THE DISCRIMINANT** — the builder marks them
`PUBLISHED_NEVER_THE_DISCRIMINANT` in its own column, pre-data.

| Count | Prior arm (`spec356-long`, 14,400 s, **n = 1**) | **r1** | **r2** |
|---|---|---|---|
| **coordinate last half — THE DISCRIMINANT** | **2** in 7,190 s of sampled ledger (**~1 / h**) | **0** in the half `14,410 … 28,800 s` (1,440 scrapes, **0.000000 / h**) | **0** in the same half (1,440 scrapes, **0.000000 / h**) |
| whole run — **PUBLISHED, NEVER THE DISCRIMINANT** | **53** in 14,400 s | **39** in 28,800 s | **80** in 28,800 s |
| all `FULL` windows (rate admissible; `PARTIAL` excluded) | — | 39 in 28,000 s = **5.014286 / h** | 80 in 28,000 s = **10.285714 / h** |

*(The `SUBJECT_WHOLE_RUN` rows publish no per-hour rate at all: their `rate_excluded_reason` column reads
**"span crosses the PARTIAL tail; R3 excludes PARTIAL from every rate"**, which is R3's tail rule enforced
by the builder rather than by the keyboard.)*

**WHAT THE DISCRIMINANT READS, PLAINLY.** The pre-declared reading, fixed before the data: **a fixed-rate
periodic trigger (~1 / h) predicts ≈ 4 non-empty drains in the repeat's doubled coordinate half; a one-time
transient predicts a count that does not grow.** **OBSERVED: 0 on r1 and 0 on r2.** The count did not grow;
it went to **zero**, on **both** replicates, over a half that is **twice the prior arm's duration**.

**AND EQUALLY PLAINLY, WHAT THAT IS NOT.** **This is an OBSERVATION, NOT AN INFERENCE.** The prior arm is
**n = 1** and each new arm is **n = 1**; **no test statistic is computed** — no `t`, no α, no significance
claim, and nothing is fitted. **Neither branch of the discriminant is declared to have won:** an observed 0
is not the ≈ 4 a fixed-rate trigger predicts and it is not the ≈ 2 a one-time transient predicts either, so
what the round licenses is the statement *"the drain count in the doubled half did not grow; it fell to
zero on both replicates"* — and nothing about which of the four pre-registered mechanisms produces that.
The comparison is admissible at all **only because** the **measured binary is identical** (AC2's empty
`.rs` diff, verified — §10.1.1) **and the runner's sampling path is untouched by the additive arms** — both
checked, neither assumed.

**THE STARTUP-BURST CONFOUND, NAMED EXPLICITLY BESIDE BOTH COUNTS — AND NOW DEMONSTRATED BY THE REPEAT
ITSELF.** The whole-run count is disqualified as a discriminant for a **measured** reason, not a stylistic
one: **the drain process is non-stationary.** On the prior arm it was front-loaded — **39 of 53** before
`t = 1,860 s`, **51 of 53** before `t = 5,090 s`, leaving **2** in the last 9,310 s — so a whole-run count
compares two startup bursts and reports the answer as a rate.

**The repeat demonstrates the confound rather than merely restating it.** Two replicates of the **same
configuration, the same pin, the same binary**:

| | **r1** | **r2** | ratio |
|---|---|---|---|
| whole-run non-empty drains | **39** | **80** | **~2.05×** |
| **coordinate-last-half non-empty drains** | **0** | **0** | **identical** |

**The count that is NOT the discriminant differs by a factor of two between two replicates of one
configuration; the count that IS the discriminant is identical on both.** The binding was made **PRE-DATA**
(§10.0.2), and **n = 2 has now vindicated it** — which is the strongest thing this round says about its own
method, and it is said about the method, not about the mechanism.

**THE PRIOR-ARM ROW IS PRESENT AND READ FROM COMMITTED, UNMODIFIED BYTES.** Both replicates' T4 tables
carry `PRIOR_ARM_WHOLE_RUN` (`spec356-long`, `10 … 14,400 s`, 0 → 53, **53**, marked
`PUBLISHED_NEVER_THE_DISCRIMINANT` with the reason **"startup-burst confound: the count is front-loaded and
does not scale with duration"**) and `PRIOR_ARM_COORD_LAST_HALF` (`7,210 … 14,400 s`, 51 → 53, **Δ = 2**,
**1.000000 / h**, marked `THE_DISCRIMINANT`). Nothing about the prior arm was re-run, re-derived or edited
to produce them.

---

### §10.7 — T5: THE EPOCH CONTENT FATE LEDGER — THE TARGET CARRIED IF ONLY ONE COULD BE

**Status: an OBSERVATION (R1.3).** No limb, step, slot or leaf reads it.

**Artifacts:** `spec356c-t5-fate-r1.csv` (**415** epoch rows) and `spec356c-t5-fate-r2.csv` (**447**), over
the **coordinate last half** `[14,410 … 28,800]` — **the same window the §10.2.2 walk reads**, so the
ledger and the walk describe the same rows.

**THE COLUMN HEADER, CARRIED VERBATIM AS COMMITTED** (P7 grades it verbatim; it is reproduced here
character-for-character so a rename cannot pass unnoticed):

```
epoch,t_first_seen,t_last_seen,scrapes_in_epoch,added_bytes,indexed_refs_at_lwm_pass__AGGREGATE_NOT_PER_EPOCH,was_drained_ever,bytes_freed_attributed,lwm_passed_at_t
```

| Quantity | **r1** | **r2** |
|---|---|---|
| epoch rows | **415** | **447** |
| epochs observed | **460 … 874** | **460 … 906** |
| window (`t_first_seen` … `t_last_seen`) | 14,410 s … 28,800 s | 14,410 s … 28,800 s |
| **`scrapes_in_epoch`** — published **beside** `added_bytes` so the sampling uncertainty is VISIBLE, not inferred | **3** ×267, **4** ×104, **5** ×41, **6** ×3 | **2** ×1, **3** ×346, **4** ×100 |
| `added_bytes` — sum / min / max / mean | 9,539,733 B / 17,089 / 31,188 / **22,987.3** | 10,267,292 B / 16,123 / 30,981 / **22,969.3** |
| `indexed_refs_at_lwm_pass__AGGREGATE_NOT_PER_EPOCH` | 1,010 … 1,458 (**1 row empty**) | 1,007 … 1,439 (**1 row empty**) |
| **`was_drained_ever`** | **`false` on ALL 415 rows** | **`false` on ALL 447 rows** |
| **`bytes_freed_attributed`** | **empty on ALL 415 rows** — nothing drained, so nothing was attributed, and the builder emitted the column **empty rather than `0`** (R3.4: no substitute for a value it cannot compute) | **empty on ALL 447 rows** |
| `lwm_passed_at_t` | present on 414 rows, **empty on 1** | present on 446 rows, **empty on 1** |

**Sampled rows** (header order):

```
r1 first: 460,14410,14430,3,19412,1361,false,,14440
r1  last: 874,28760,28800,5,21850,,false,,
r2 first: 460,14410,14430,3,19021,1373,false,,14440
r2  last: 906,28790,28800,2,16422,,false,,
```

**THE ONE EMPTY ROW ON EACH REPLICATE IS THE WINDOW'S LAST EPOCH** — the watermark had not passed it when
the ledger ended, so both `indexed_refs_at_lwm_pass__…` and `lwm_passed_at_t` are legitimately empty. This
is also **exactly why T5 carries one row more than T1(b)'s pass ledger on each replicate** (415 vs 414;
447 vs 446).

**THE SAMPLING UNCERTAINTY, STATED.** At `TOPGUN_EPOCH_WIDTH=1000` an epoch is current for ≈ 31 s ≈ **3
scrapes**, which the measured `scrapes_in_epoch` distribution confirms (mode 3 on both replicates).
`added_bytes` therefore carries **up to one scrape of boundary attribution at each end ⇒ ~⅓ relative
uncertainty**, and the per-epoch `scrapes_in_epoch` is published **on the same row** so the reader can see
that uncertainty rather than reconstruct it. The one r2 row with `scrapes_in_epoch = 2` is the final,
truncated epoch.

**`was_drained_ever` — THE INFERENCE RULE, STATED, AND WHETHER THIS ROUND EVER EXERCISED IT.** The rule:
when a drain reports **`drain_epochs == 1`**, its epoch is exactly `last_drained_epoch` after it and the
drained set is **EXACT**; when **`drain_epochs == k > 1`**, the drained set is inferred as the `k` epochs
**ending at** `last_drained_epoch`, which **ASSUMES CONTIGUITY**. **This round never exercised the
assumption.** §10.4 measures `drain_epochs` at **1.000 epoch per drain on every one of the 39 drains on r1
and every one of the 80 on r2** — 119 drains, `k = 1` on all of them — matching what the PRE-DATA dry-run
subject reported for that same column. **So every `was_drained_ever` value in both ledgers rests on the
EXACT limb of the rule and none on the contiguity assumption**, and had a `k > 1` drain occurred, §10.4's
published `d_drain_epochs_sum` / `_count` columns are where the reader would have seen it.

**THE T1(b) ≡ T5 IDENTITY STATEMENT — THE MIRROR OF §10.3.1's, SO NEITHER SUBSECTION BANKS THE VALUE
TWICE.** Under the `current_epoch ≡ low_water_mark` identity, **T5's epoch rows and T1(b)'s LWM-pass rows
are the SAME rows reached by two routes**: `lwm_passed_at_t` degenerates to `t_last_seen`, and T1(b)'s
ledger is a projection of this one. **Agreement between §10.7 and §10.3 is therefore a RESTATEMENT, NOT
CORROBORATION.** The identity is **NEAR-TOTAL, NOT TOTAL** — measured at **0.965278 (1,390 / 1,440)** on r1
and **0.972917 (1,401 / 1,440)** on r2 over the deciding window (**0.964596** and **0.970833** over the
full ledgers) — so the two tables are near-projections of one another rather than independent observations,
and the reader is told which regime this is rather than left to assume totality.

**WHAT THE LEDGER DOES ANSWER, AND WHAT IT DOES NOT — IN ONE SENTENCE EACH, BECAUSE THIS IS THE TARGET
WHOSE VALUE IS EASIEST TO OVER-CLAIM.**

- **IT DOES ANSWER:** *which content arrived (`added_bytes`, with `scrapes_in_epoch` beside it), when the
  watermark passed it (`lwm_passed_at_t`, a bucket — C1), whether anything ever drained it
  (`was_drained_ever`), and what the prune attributed (`bytes_freed_attributed`).* On this round the answer
  is uniform and stark: **9.54 MB of tombstone content on r1 and 10.27 MB on r2 arrived in the deciding
  window, the watermark passed essentially all of it, NOTHING drained ANY of it, and the prune attributed
  NOTHING to any of it.**
- **IT DOES NOT ANSWER:** *whether that content was ever in the drainable index.* That is **T1(a) —
  OUT-OF-SCOPE, reason and owner in §10.3.2, owner `TODO-634`** — and the
  `indexed_refs_at_lwm_pass__AGGREGATE_NOT_PER_EPOCH` column **is the aggregate gauge read at the pass
  bucket, NOT a per-epoch residency answer.** Its header carries that marker verbatim precisely so the
  column cannot be quietly read as the answer to the question it sits next to.

---

### §10.8 — §8.2's ESCALATION (R7.2), SINCE THE OUTCOME **IS** `INDETERMINATE`

**The determination published in §10.2.3 is `INDETERMINATE` on BOTH replicates, via Step 0 limb (c), at
Step 5.** §8.2 therefore obliges an escalation that is **"not 'record a note'"**, and its form was fixed
**PRE-DATA** in §10.0.5. **All SIX items follow, in that frozen order.** Nothing in §10.3 – §10.7 is a
premise of any of them: the targets are observations, they moved nothing, and this escalation would read
identically had none of them been derived.

#### §10.8.1 — ITEM 1: THE UNCLASSIFIED CAUSE, **NAMED**

> **The mechanism by which the OR tombstone prune falls behind at `TOPGUN_EPOCH_WIDTH=1000` remains
> unclassified among SELECTION/FRONTIER, SCHEDULING/LICENSING and THROUGHPUT, and a mechanism outside
> those four is not excluded.**

That sentence **is** the object §8.2 asks for. **ADJ-4's bound is quoted, not paraphrased**: no one of the
four pre-registered mechanisms is endorsed, no one of them is excluded, and **a fifth mechanism outside the
four is NOT excluded either.** Nothing anywhere in §10 reads *"the best-supported mechanism is X"* on this
outcome, because **no numbered step was evaluated at all.**

#### §10.8.2 — ITEM 2: **EVERY** STEP'S EVALUATED VALUE, PER REPLICATE, IN THE FROZEN ORDER

**A step left off this table is the omission §8.2 exists to prevent**, so every one is present — Step 0's
four limbs with their numbers first (limb **(d)** first of all, R1.2 item 1), then Steps 1 – 5. *(§10.2.2
walked these; they are restated here in the escalation's own form because the escalation must stand on its
own page.)*

| Frozen order | **r1** — evaluated value | **r2** — evaluated value |
|---|---|---|
| **Step 0 limb (d)** — ADJ-11 clause 0's conservation identity, **FIRST** | `Δpasses` **414,110** = `Δempty_drains` **414,110** + `Δnonempty_drains` **0** ⇒ **HOLDS** | `Δpasses` **445,682** = `Δempty_drains` **445,682** + `Δnonempty_drains` **0** ⇒ **HOLDS** |
| **Step 0 limb (a)** — §1.3's 2×2 in CLEAN or BUILD-LINEAGE-EFFECT | `R51_T = 0.346905` NOT REJECTED, `R52_T = 0.800515` NOT REJECTED against the frozen critical value **4.303** ⇒ cell **CLEAN** ⇒ **HOLDS** | same controls over the same inherited bytes: `R51_T = 0.346905`, `R52_T = 0.800515`, both NOT REJECTED against **4.303** ⇒ **CLEAN** ⇒ **HOLDS** |
| **Step 0 limb (b)** — ≥ 1 split recompute in the window read | `split_recomputes_total` 917 → 1,746, **Δ = 829 ≥ 1** ⇒ **HOLDS** | `split_recomputes_total` 918 → 1,810, **Δ = 892 ≥ 1** ⇒ **HOLDS** |
| **Step 0 limb (c)** — population check **AND** ADJ-12's sentinel readability | **LEG 1 PASSES** (0 admissibility flags, 0 instrument defects, arming witness `41/41 PASSED`, 0 empty-or-NaN fields over 43 × 2,881). **LEG 2 FAILS**: 0-sentinel rows **1,440 / 1,440 = 100.000000 %**, 0 non-sentinel rows, against the frozen `> 50 %` ⇒ **FAILS (conjunction)** | **LEG 1 PASSES** (same, over 43 × 2,880). **LEG 2 FAILS**: **1,440 / 1,440 = 100.000000 %**, 0 non-sentinel rows ⇒ **FAILS (conjunction)** |
| **Step 0 overall** — **FAIL-CLOSED**, and it precedes every numbered step | **FAILS** | **FAILS** |
| **Step 1** — non-drop exit share > 10 % | **NOT EVALUATED — Step 0 is fail-closed and precedes every numbered step** | **NOT EVALUATED — Step 0 is fail-closed and precedes every numbered step** |
| **Step 2** — exit share ≤ 10 % and `median(L) ≤ B` | **NOT EVALUATED — Step 0 is fail-closed and precedes every numbered step** | **NOT EVALUATED — Step 0 is fail-closed and precedes every numbered step** |
| **Step 3** — exit share ≤ 10 %, persistent backlog, slope rejects negative | **NOT EVALUATED — Step 0 is fail-closed and precedes every numbered step** | **NOT EVALUATED — Step 0 is fail-closed and precedes every numbered step** |
| **Step 4** — exit share ≤ 10 %, persistent backlog, slope does not reject | **NOT EVALUATED — Step 0 is fail-closed and precedes every numbered step** | **NOT EVALUATED — Step 0 is fail-closed and precedes every numbered step** |
| **Step 5** — Step 0 fails, or the deciding columns are unreadable | **FIRED ⇒ `INDETERMINATE`, via Step 0 limb (c)** | **FIRED ⇒ `INDETERMINATE`, via Step 0 limb (c)** |

#### §10.8.3 — ITEM 3: THE **BLOCKING** ADMISSIBILITY LIMB, NAMED BY LETTER, AGAINST ITS FROZEN THRESHOLD

> **limb (c): sentinel fraction `1,440 / 1,440 = 100.000000 %` against the pre-registered `50 %`** — on
> **both** replicates.

The frozen literal is **`> 50 %`** and **100.000000 % is inside it**, with **0** non-sentinel rows
available on either replicate. This is **ADJ-12's own escape hatch firing on its own pre-registered
threshold**, reached exactly as §10.0.3 pre-declared the 100 %-sentinel case would reach it. **It is a
PROTOCOL EXIT, not a third branch:** no threshold moved, the Step 2 / Steps 3–4 exact-complement partition
is untouched, the evaluation order is untouched, and **no `ADJ-21` is authored.**

#### §10.8.4 — ITEM 4: **PD-4's COUPLING, ON THIS ROUND'S OWN NUMBERS** — WHERE IT BITES HARDEST

PD-4 couples the non-drop exit **share** to its **denominator**: the two statistics degrade together, and
**neither is ever quoted without the other.** On this round the coupling is not a caution — **it is the
whole reading**, because **the denominator collapsed to zero.**

| Over the coordinate last half | **r1** | **r2** | Prior arm (`spec356-long`) |
|---|---|---|---|
| `Δconsidered` — **THE DENOMINATOR** | **0** | **0** | **2,000** |
| `Δdropped` | **0** | **0** | 2,000 |
| `Δmatched_nothing` | 0 | 0 | 0 |
| `Δabsent` | 0 | 0 | 0 |
| `Δrestored_read_error` | 0 | 0 | 0 |
| `Δrestored_evicted` | 0 | 0 | 0 |
| `Δrestored_write_error` | 0 | 0 | 0 |
| **non-drop exit share** | **UNDEFINED — denominator 0** | **UNDEFINED — denominator 0** | **0.000000 % over a denominator of 2,000** |

**All five non-drop exits are 0, `dropped` is 0, and `considered` is 0**, so the share on both replicates is
**UNDEFINED, denominator 0 — it is NOT `0.000000 %`**, and writing it as `0.000000 %` would manufacture a
measured zero out of an absence of measurement. On the prior arm the same share genuinely read
**`0.000000 %`, over a denominator of 2,000** — a real zero over real observations. **The two are printed
together above, and neither is quoted anywhere without the other.** This is precisely the degradation PD-4
anticipated: **the share and its denominator failed at the same time, and the record says so.**

#### §10.8.5 — ITEM 5: **§8.3, QUOTED VERBATIM AS A BLOCK QUOTE, ADJACENT TO THE DETERMINATION**

§8.3's own last sentence requires this of **any** Step-5 outcome, and **P9 checks it**. It is carried in
§10.2.3 beside the determination and it is carried again **here**, inside the escalation, because P9's text
requires adjacency to the published determination and **the duplication is deliberate, not an editing
slip**:

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

#### §10.8.6 — ITEM 6: THE ROUTING SENTENCE

**The unclassified cause is carried into `TODO-634`'s design phase as a STATED OPEN INPUT** — not as a
closed question, not as a resolved one — **together with T1(b)'s LWM-pass ledgers, T2's drain
enumerations, T3's windowed backlog series, T4's rate comparison, T5's epoch content fate ledgers, and
T1(a)'s OUT-OF-SCOPE row with its reason and its owner** (and T2(exactness)'s, dispositioned identically).
**That is what turns *"we still don't know"* into a DESIGN INPUT rather than a dead end:** the design phase
inherits a named open question, five tables of what the instrument could see, and **two explicit statements
of what it could not** — per-epoch index residency and per-epoch true tombstone byte size, both requiring a
`.rs` emission this round was forbidden to make, both owned by `TODO-634`. **The §10 record IS the
routing**; no byte of any tracker file is written by this subsection.

#### §10.8.7 — R7.3: WHAT THIS ESCALATION DOES **NOT** DO

1. **It does NOT block the family.** §8.3, quoted verbatim above, is the reason: the recommended
   reclamation model bounds the reclaimable set by **live claims**, not by any hypothesis about *why* the
   current prune falls behind, so a selection defect, a scheduling defect and a throughput defect are all
   **contained** by it. **A Step-5 outcome is an expensive answer, not a blocked one.**
2. **It does NOT propose a fix shape.** No design, no ceiling, no predicate, no schedule and no threshold
   is proposed here — the observations are handed to `TODO-634`'s design phase, which is where fix shapes
   are chosen.
3. **It does NOT endorse any of the four mechanisms.** SELECTION/FRONTIER (exit limb), SELECTION/FRONTIER
   (licensing limb), SCHEDULING/LICENSING and THROUGHPUT are **all** left unendorsed and **all** left
   un-excluded, and **a mechanism outside the four is not excluded either.**
4. **It does NOT re-run the cell hoping for a different window, and it adjusts NOTHING.** No threshold
   moves, no window is re-sliced, no aggregator changes, no `ADJ-21` is authored, and neither replicate is
   re-executed. §10.2.6's two POST-DATA findings are recorded and routed, **not repaired** — R0.3 is
   categorical.

**WHAT AN UNCLASSIFIED CAUSE COSTS IS FIX-SHAPE EFFICIENCY, NOT SAFETY.**

---

### §10.9 — R0.5's TRACKER-DISCIPLINE LEDGER, **THE POST SIDE** — AND THE SIXTH TRANSCRIPT

**This subsection discharges the POST half of §10.0.7's ledger and of §10.0.4's `P13`.** It is a
**measurement of file content**, not a `git diff`: `.specflow/` is gitignored at `.gitignore:111` and no
path under it is in the index, so `git diff <pin>..HEAD -- '.specflow/todos/'` is **empty for every
possible execution** and any check resting on it is **vacuously GREEN** — PD-8's own defect reproduced
inside the checklist authored to replace it. Tracking `.specflow/` in git is barred by standing project
policy and is **not** the remedy. Reading the files and asserting their content is, and that is what
follows.

#### §10.9.1 — THE FOUR POST-DIGESTS, BESIDE §10.0.7's PRE VALUES

`shasum -a 256`, run on the live files after the edit:

| Tracker file | PRE (§10.0.7) | POST (here) | Required | Observed |
|---|---|---|---|---|
| `.specflow/todos/TODO-634.md` | `555c3aa92a569c030a7fbacfafdee5933bf4beda668d79fff126f1a2c0e74f8c` | `a1b17d97ca55ae4c3e24bbf19509f0e981c5b052c1e7f4d56d734a57a0219428` | **MOVED** | **MOVED — limb (b) GREEN** |
| `.specflow/todos/TODO-637.md` | `48a16fbdfa923770b673882ae58b7a1aa5f15e5a1d3b8a1ee36ce3142b02940f` | `48a16fbdfa923770b673882ae58b7a1aa5f15e5a1d3b8a1ee36ce3142b02940f` | **UNMOVED** | **IDENTICAL** |
| `.specflow/todos/TODO-638.md` | `756082197b6efd7b9158fea53c859724f78c82d5a2e3684eaa38e206571825df` | `756082197b6efd7b9158fea53c859724f78c82d5a2e3684eaa38e206571825df` | **UNMOVED** | **IDENTICAL** |
| `.specflow/todos/TODO-648.md` | `56fb3f59f7159b3acd0afa3a66f04df74e3bf9096614ae5357940d149847ef2c` | `56fb3f59f7159b3acd0afa3a66f04df74e3bf9096614ae5357940d149847ef2c` | **UNMOVED** | **IDENTICAL** |

**Three unchanged, one moved — and the moved one is the one that was obliged to move.** §10.0.4's limb
**(b)** is explicit that *"an unchanged digest means the required edit never happened and is **RED**, not a
pass"*; the converse obligation is limb **(a)**'s, and the three unrelated files satisfy it byte for byte.

#### §10.9.2 — THE POST TICK COUNTS AND THE CENSUS — **STILL 2 / 5**

| Quantity | Command | PRE (§10.0.7) | POST | Verdict |
|---|---|---|---|---|
| ticked boxes | `grep -c '^- \[x\] '` | **2** | **2** | unchanged |
| unticked boxes | `grep -c '^- \[ \] '` | **5** | **5** | unchanged |
| top-level boxes (census) | `grep -c '^- \[[ x]\] '` | **7** | **7** | unchanged |
| indented boxes (census) | `grep -c '^[ \t][ \t]*[-*+] \['` | **0** | **0** | unchanged |
| strict anchor `- [ ] **NEW, CARRIED IN FROM SPEC-356b — the §8.1 REPEAT:` | `grep -qF` | matches | **matches** | unchanged |

**`TODO-634` IS NOT CLOSED, NOT TICKED, AND NOT DELETED.** The §8.1 box carries the round's outcome and
stays **unticked**, which is the disposition §10.0.7's wave-5 constraint 3 pre-declared: *"the box ticks
only when `SPEC-356c` is done, and publishing the repeat's outcome is not the same act as closing
`TODO-634`."* Eleven of SPEC-356b's routed branches still name this item; the census figures make the two
tick counts **exhaustive** over the file's box set, so "still 2 / 5" is a statement about **every** box the
file has, not about the seven somebody remembered to look at.

#### §10.9.3 — THE §8.1 BOX, **BEFORE AND AFTER, SIDE BY SIDE** (limb (d))

**BEFORE** — `.specflow/todos/TODO-634.md:213-233` at the PRE-DATA bytes, i.e. §10.0.7's verbatim
before-text, reproduced by the grader from the provenanced fixture:

```
- [ ] **NEW, CARRIED IN FROM SPEC-356b — the §8.1 REPEAT: `OWED, NOT DONE`.**
      **CARVED TO `SPEC-356c` (2026-08-11, `/sf:plan`) — carved, NOT done and NOT ticked.** That spec
      authors ONLY this repeat increment (the `long` cell at 28,800 s × n = 2 under the frozen
      predicate), plus PD-8's PRESENCE limb, the v2 grader sidecar carrying TODO-648's `B`-sentinel
      fix, the stated worktree procedure, and the FIVE pre-declared observation targets that hunt the
      **fifth mechanism** (the LWM advancing past never-freed content). **The registry family is NOT in
      its scope**, and this box ticks only when SPEC-356c is done. Exactly **one** repeat of
      the deciding configuration at **DOUBLED duration AND DOUBLED replicates** — the `long` cell at
      **28,800 s with n = 2** — under the **SAME pin** `feb85268952001813e502e27f65180855676ac25` and the
      **SAME frozen predicate** (manifest §0–§8 plus §8A's twenty addenda). **It may not adjust a
      threshold, an ordering or a conditional**: it is a second observation, not a re-specification.
      Both axes are doubled for different reasons — duration for an effect too slow to separate in 4 h,
      replicates for the n = 1 fragility SPEC-355 §10.4.2 measured — and **one axis alone leaves the
      other cause unaddressed.** **If the repeat is STILL INDETERMINATE, §8.2 escalates** the
      unclassified cause as an explicit input to this item's design phase, naming it and quoting every
      step's evaluated value and the admissibility limb that blocked.
      **This does NOT block the family.** Manifest §8.3, verbatim: *"The recommended reclamation model
      closes safety REGARDLESS of which cause it turns out to be … A selection defect, a scheduling
      defect and a throughput defect are all contained by a registry that never reclaims below a live
      claim."* What an unclassified cause costs is **fix-shape efficiency, not safety** — an expensive
      answer, not a blocked one.
```

**AFTER** — `.specflow/todos/TODO-634.md:213-249` at the edited bytes. **The 21 lines above are byte-
identical; the 16 lines below are the whole of the delta**, appended as indented continuation lines of the
same list item (§10.0.7's wave-5 constraint 1):

```
      **OUTCOME (2026-08-12, `SPEC-356c` EXECUTED) — the repeat RAN, and the determination is
      `INDETERMINATE` on BOTH replicates. Recorded here; this box is deliberately NOT ticked.** The
      `long` cell ran at **28,800 s × n = 2** under the frozen predicate (manifest §0–§8 plus §8A's
      twenty addenda, unadjusted) at the executed pin `8a60f1357f4702f4c5393538ef6b4ae6cebd247f` —
      SPEC-356b's `feb85268…` **in substance**, the `.rs` diff between them being empty, so the
      measured binary is the same one. Both replicates exit at **Step 0 limb (c)** on a
      **1,440 / 1,440 = 100.000000 %** admissibility sentinel, and **Steps 1–4 are `NOT EVALUATED`**
      on both — so this is a **concordant** `INDETERMINATE`, not a split across replicates, and
      **no mechanism is named**: none of the four pre-registered mechanisms is endorsed and none is
      excluded. **§8.2's escalation is published in consequence**, all six R7.2 items in order with
      §8.3 quoted verbatim beside the determination. **This item stays OPEN.** SPEC-356c authored the
      repeat increment only; the registry family remains outside its scope, and this box ticks when
      that family closes — recording a second observation is not the same act as closing it.
      Evidence: `spec356-manifest.md` §10 — §10.1 the lineage and the pin, §10.2 the PRESENCE verdict,
      the frozen walk and the determination, §10.3–§10.7 the five observation targets, §10.8 §8.2's
      escalation.
```

**The after-text carries the round's OUTCOME (`INDETERMINATE`, both replicates, Step 0 limb (c)) and the
literal `spec356-manifest.md §10` pointer**, which is what limb (d) asserts. It states the outcome, the
concordance, and the item's continuing **OPEN** status — and it names no mechanism, because §10.2.3's
determination names none.

#### §10.9.4 — THE SIXTH TRANSCRIPT, AND WHY IT IS ITS OWN FILE

The grader was **digest-checked before it was run** — `spec356c-trackergrade.sh`
`879dadb1…c42db`, `spec356c-trackergrade.ref` `e3dcd977…3577`, `spec356c-trackergrade-proofs.txt`
`6e361bc1…ea563`, **3/3 matching §10.0.6** — and the fixture was digest-checked against §10.0.7's PRE value
before any limb was graded, the grader **refusing** (exit 2, not a verdict) on an unprovenanced fixture.

```
$ sh …/evidence/spec356c-trackergrade.sh /private/tmp/spec356c-todo634-fixture.md .specflow/todos
…
VERDICT: GREEN (a, b, c, d, e)
$ echo $?
0
```

| # | Input | Pre-declared (§10.0.4, P13) | Observed | Where |
|---|---|---|---|---|
| 1 | unedited `TODO-634.md` (provenanced fixture) | **RED (b), (d)** | `RED (red limbs: b d)` | `spec356c-trackergrade-proofs.txt` |
| 2 | §8.1 box **ticked** | **RED (c)** — and (d) | `RED (red limbs: c d)` | `spec356c-trackergrade-proofs.txt` |
| 3 | `TODO-637.md` one byte changed | **RED (a)**, naming the file | `RED (red limbs: a)` | `spec356c-trackergrade-proofs.txt` |
| 4 | correct §8.1 edit **plus** an unrelated paragraph rewritten | **RED (e)**, (a)–(d) green | `RED (red limbs: e)` | `spec356c-trackergrade-proofs.txt` |
| 5 | synthetic correctly-edited copy | **GREEN on all five** | GREEN, all five | `spec356c-trackergrade-proofs.txt` |
| **6** | **the ACTUALLY-edited `.specflow/todos/TODO-634.md`** | **GREEN on all five** | **`VERDICT: GREEN (a, b, c, d, e)`, exit 0** | **`spec356c-trackergrade-run6.txt`** |

**WHY THE SIXTH TRANSCRIPT IS A NEW FILE RATHER THAN AN APPEND.** `spec356c-trackergrade-proofs.txt`'s
`shasum -a 256` is **pinned PRE-DATA in §10.0.6**. Appending wave 5's transcript to it would **move a
pinned sidecar digest after the data landed** — the act R0.2 forbids — and would make the five PRE-DATA
proofs unverifiable against their own pin. The five stay byte-identical; the sixth is committed as
`spec356c-trackergrade-run6.txt`, which also carries the digest-check preamble, the fixture provenance and
the post-measurements above in their raw form.

**THE ONE READING NOT SUPPRESSED.** §10.0.7's stated instrument limit 1 predicted that limb (d)'s
outcome-token sub-check would be satisfied by the **PRE**-text as well, since the unedited box already
contains `INDETERMINATE` inside *"If the repeat is STILL INDETERMINATE"*. The transcript's own evidence
lines report exactly that — *"outcome token `INDETERMINATE` was ALREADY in the before-text"*, *"pointer
literal is NEW in the after-text"* — so **(d)'s discrimination on this input rests on the pointer**, and a
reader should read those two lines before believing the green. Limb (e) reports **one** hunk,
`@@ -233,0 +234,16 @@`, **inside the §8.1 box, 0 hunks outside**: the delta is 16 added lines and **zero
deleted**, so AC15's *"every other box in that file is unchanged"* is measured and not asserted.

#### §10.9.5 — **P13's RED IN §10.2.1 IS NOT CONTRADICTED HERE. IT IS THE PREVIOUS READING OF THIS LIMB.**

§10.2.1 grades `P13` **RED**, on the POST half only. That verdict stands as written and is not amended:
**it was the reading of the record at the instant the PRESENCE limb was evaluated**, and at that instant
`TODO-634.md`'s digest was still `555c3aa9…4f8c` — **unmoved** — the four post-digests, the post-text, the
post tick counts and the sixth transcript were **not in the record**, and §10.0.4's limb (b) is explicit
that an unchanged digest is **RED, not a pass**. Publishing that RED is R6.4's requirement: *"a RED entry
is not a reason to delay publication — IT IS THE FINDING."* Item 1 of the checklist is **evaluated FIRST**;
a limb evaluated first can only speak about the record as it stands then, and pre-crediting bytes that are
due later is the vacuous-guard failure PD-8 indicts wearing a different hat.

**§10.2.1's own P13 entry says this in advance**, and this subsection is the event it named:

> *"When §10's tracker-discipline ledger lands the post-digests, the post-text, the post tick counts and
> the sixth transcript, the POST half is graded THERE, against limbs (a)…(e), at its own bytes — a later
> GREEN there is not a contradiction of this RED, it is the next reading of the same limb, and this entry
> says so explicitly so that no reader has to reconcile the two by guessing."*

**So the two readings are consistent by construction, and the record carries both:**

| Reading | When | `TODO-634.md` digest | Verdict |
|---|---|---|---|
| §10.2.1's `P13` | at the PRESENCE limb, **before** the edit | `555c3aa9…4f8c` (**unmoved**) | **RED**, POST half absent |
| §10.9's ledger (here) | at the tracker ledger, **after** the edit | `a1b17d97…9428` (**moved**) | **GREEN**, all five limbs |

**Neither reading is retracted, and neither is back-dated.** §10.2.1's RED is not edited to green — that
would be exactly the back-dating this manifest's append-only discipline exists to prevent — and this
GREEN is not claimed to have been true earlier than its own bytes. **The PRESENCE verdict as published in
§10.2.1 therefore remains 11 GREEN / 2 RED (P9 and P13)**, and it is the verdict of record for that limb.

---

### §10.10 — THE FULL VALIDATION CHECKLIST, RUN — ALL NINETEEN ITEMS, **INCLUDING THE THREE THAT ARE RED**

**ORDERING IS NORMATIVE and it was observed.** Item 1 (PRESENCE) was evaluated **first**, and is published
at §10.2.1; items 2–5 (lineage and provenance) were evaluated before any content item. **No item below is
reported green on the strength of an artifact item 1 found absent.**

**THE HEADLINE, STATED BEFORE THE DETAIL: 16 GREEN, 3 RED — items 1, 10 and 12.** All three REDs are
published as REDs. **A checklist that greens on a known failure is the exact defect this spec exists to not
repeat** — PD-8's *"a checklist that cannot fail on an ABSENT table"* — and none of the three is repaired,
because R0.3 is categorical and all three touch bytes that are frozen or pinned.

| # | Item | Verdict |
|---|---|---|
| 1 | PRESENCE, P1…P13, evaluated FIRST | **RED** — 11 GREEN / 2 RED (**P9**, **P13**), as published in §10.2.1 |
| 2 | frozen-path diffs, both needles, both directions | **GREEN** |
| 3 | manifest freeze, zero deletions, both prefix `cmp`s | **GREEN** |
| 4 | data boundary, `--diff-filter=A` ancestry | **GREEN** |
| 5 | pin provenance, per cell | **GREEN** |
| 6 | artifact binding 7/7, and the scratch-map residual | **GREEN** |
| 7 | arming witness and column shapes | **GREEN** |
| 8 | the frozen walk, per replicate | **GREEN** |
| 9 | ADJ-3's bundle, limbs (i)–(v) | **GREEN** |
| 10 | wrong-axis and inference greps over §10 | **RED** — one line, `2957`, PRE-DATA and at the pin |
| 11 | the sixteen eight-window fits | **GREEN** |
| 12 | the v1/v2 matrix and the diff's taxonomy | **RED** — the **run-3** leg; v2 refuses the repeat's cells |
| 13 | target-builder reproduction **and** the PRE-DATA dry run | **GREEN** |
| 14 | target/predicate separation | **GREEN**, with the reading stated and both counts published |
| 15 | escalation form | **GREEN** |
| 16 | tracker discipline, BY CONTENT | **GREEN** — five limbs, six transcripts |
| 17 | catalog | **GREEN** |
| 18 | build | **GREEN** |
| 19 | cross-vendor | **GREEN** |

---

#### §10.10.1 — ITEM 1 (PRESENCE) — **RED**, RESTATED FROM §10.2.1 AND NOT RE-GRADED

**Verdict of record: 11 GREEN, 2 RED — `P9` and `P13`.** It is restated, not recomputed: item 1 is
*"evaluated FIRST"*, so its verdict is the one taken at that instant and re-running it later would replace
a measurement with a more convenient one.

- **`P9` — RED.** The five target anchors `T1.`…`T5.` occur **ZERO** times in `spec356-verdict-xask.md`;
  the §8.3 block-quote leg is GREEN. The artifact is **not edited** to carry them and the entry is **not
  softened** — §10.2.1 records that `T1`…`T5` is §10.0.2's own naming applied to an enumeration the source
  artifact never carried under those labels.
- **`P13` — RED**, on the **POST half only**, at the instant §10.2.1 was written. **§10.9 is the next
  reading of that same limb and it is GREEN on all five** — §10.9.5 sets the two side by side, retracts
  neither and back-dates neither. **Item 1's verdict stays RED**, because item 1 is the reading taken
  first; item 16's verdict below is the reading taken at the tracker ledger's own bytes.

---

#### §10.10.2 — ITEM 2 — **GREEN.** BOTH NEEDLES, AND BOTH DIRECTIONS

`git diff --stat <needle>..HEAD -- <path>`, run at **`feb85268`** (SPEC-356b's pin, checklist 2's named
needle) **and** at **`8a60f135`** (SPEC-356c's own resolved pin):

| Path | from `feb85268` | from the SPEC-356c pin |
|---|---|---|
| `*.rs` (repo-wide) | **EMPTY** | **EMPTY** |
| `INVARIANTS.md` | **EMPTY** | **EMPTY** |
| `scripts/check-invariants.sh` | **EMPTY** | **EMPTY** |
| `…/spec349c2-fit.awk` | **EMPTY** | **EMPTY** |
| `…/spec356-slottruth.sh` | **EMPTY** | **EMPTY** |
| `…/spec356-tmplconf.awk` | **EMPTY** | **EMPTY** |
| `…/spec356-skeleton.txt` | **EMPTY** | **EMPTY** |
| `…/benches/soak_harness/monitor.rs` | **EMPTY** | **EMPTY** |

**AND THE PAIR THAT MUST DIFF NON-EMPTY FROM `feb85268` AND EMPTY FROM THE PIN** — the leg that
distinguishes *"the instrument moved PRE-DATA"* from *"the instrument moved under the pin"*, and the reason
a single-needle check would be worthless here:

| Path | `feb85268`..HEAD `--numstat` | pin..HEAD |
|---|---|---|
| `…/spec356-prune.sh` (the runner, R2.1's two additive arms) | **`14  0`** | **EMPTY** |
| `…/spec356a-eager-registration.log` (gate 1, R0.2 `PART V`) | **`230  0`** | **EMPTY** |
| `…/spec356a-step0c-fixture.log` (gate 2, R0.2 `PART V`) | **`106  0`** | **EMPTY** |

**Both directions asserted, and both hold.** Every deliberate move is **additive** (`0` deletions) and
every one of them is **behind the pin**.

---

#### §10.10.3 — ITEM 3 — **GREEN.** THE MANIFEST FREEZE, WITH BOTH PREFIX `cmp`s

```
$ git diff --numstat feb85268..HEAD -- …/spec356-manifest.md      ->  3786   0     (ZERO deletions)
$ git diff --numstat 8a60f135..HEAD -- …/spec356-manifest.md      ->  1720   0     (ZERO deletions)
$ git show feb85268:…/spec356-manifest.md > /tmp/pin.md           ->  1,390 lines
$ head -n 1390 …/spec356-manifest.md | cmp - /tmp/pin.md          ->  SILENT
$ git show 8a60f135:…/spec356-manifest.md > /tmp/pin356c.md       ->  3,456 lines
$ head -n 3456 …/spec356-manifest.md | cmp - /tmp/pin356c.md      ->  SILENT
$ grep -c '^## §10' …/spec356-manifest.md                         ->  1
```

**THE PAIR IS WHAT CLOSES IT, exactly as the item says.** `grep -c '^## §9'` in the `feb85268` copy is
**0** — that copy has no §9 section at all (its four `§9` hits are forward references in prose) — so the
`feb85268` leg protects §9 against **deletion** but **not against interpolation**. The second `cmp`, against
SPEC-356c's own pin whose copy already carries §9 in full, is what closes interpolation. Everything this
spec wrote is strictly **after** line 3,456.

---

#### §10.10.4 — ITEM 4 — **GREEN.** THE DATA BOUNDARY

```
$ git log --follow --diff-filter=A --format=%H --reverse -- '…/spec356c-*.soak.json' | head -1
0b96e63e306a7e51fa04a022eeaff7762497ac1d
  test(sf-356c): land replicate r1 of the §8.1 repeat — 28,800 s at the pin
```

| Instrument that must precede the boundary | added at | ancestor of `0b96e63e`? |
|---|---|---|
| §10.0 / the manifest (added `fc95b86d`, §10.0 written at `8a60f135`) | `8a60f135` | **YES** |
| the two runner arms (`spec356-prune.sh`, `long8h_r1` / `long8h_r2`) | `8a60f135` | **YES** |
| `spec356c-slottruth-v2.sh` | `8a60f135` | **YES** |
| `spec356c-targets.sh` | `8a60f135` | **YES** |
| `spec356-verdict-xask.md` | `607a3775` | **YES** |

**Not one of them landed after the first `soak.json`**, so the finding item 4 exists to make — *the
instrument was chosen with data visible* — does not arise. *(Method note, recorded because it bit during
execution: `git log --follow --reverse | head -1` does **not** reliably name the adding commit — `--follow`
reorders — and the honest form is `git log --diff-filter=A --format=%H -- <path> | tail -1`, which is what
the table above uses.)*

---

#### §10.10.5 — ITEM 5 — **GREEN.** PIN PROVENANCE, PER CELL

| Check | `spec356c-long-r1.matrix.txt` | `spec356c-long-r2.matrix.txt` |
|---|---|---|
| `grep -c '^  repo HEAD:      8a60f135…247f$'` | **1** | **1** |
| `grep -c '^  repo HEAD:'` (no second, unpinned line) | **1** | **1** |
| `grep -c '^  duration:            28800s$'` | **1** | **1** |
| its own cell line, exactly once | `=== spec356 prune-record run: cell long8h_r1 ===` → **1** | `… cell long8h_r2 ===` → **1** |
| any other cell header in the file | **0** | **0** |

---

#### §10.10.6 — ITEM 6 — **GREEN.** ARTIFACT BINDING, THEN THE DIRECTORY RESIDUAL

`shasum -a 256 -c spec356c-long-r<n>.artifacts.sha256`, run in the evidence directory, **verbatim, with no
mapping applied**: **7/7 `OK` on r1 and 7/7 `OK` on r2** (`csv`, `prune.csv`, `soak.json`,
`progress.jsonl`, `mechanism.json`, `matrix.txt`, `harness-console.log`). No `RED PROVENANCE` on either, so
no cell is re-run.

**And the residual is checked, not assumed.** `spec356c-scratch-map.txt` carries **exactly one line per
committed replicate**, basenames identical on both sides, and each line's scratch dir is the one that
cell's own `matrix.txt` names:

| | map line | the cell's `matrix.txt` says |
|---|---|---|
| **r1** | `/private/tmp/spec356c-out-r1 → …/evidence  basename spec356c-long-r1.*  data dir /private/tmp/spec356c-data-r1` | `data dir: /private/tmp/spec356c-data-r1`, `prune.csv: /private/tmp/spec356c-out-r1/spec356c-long-r1.prune.csv` |
| **r2** | `/private/tmp/spec356c-out-r2 → …/evidence  basename spec356c-long-r2.*  data dir /private/tmp/spec356c-data-r2` | `data dir: /private/tmp/spec356c-data-r2`, `prune.csv: /private/tmp/spec356c-out-r2/spec356c-long-r2.prune.csv` |

**No committed replicate is absent from the map**, so R2.3 point 5's re-run disposition does not fire.

---

#### §10.10.7 — ITEM 7 — **GREEN.** ARMING AND COLUMN SHAPES

| Check, per `runner-console.log` | r1 | r2 |
|---|---|---|
| `arming witness PASSED` | **1** | **1** |
| the disarmed literal | **0** | **0** |
| `INSTRUMENT DEFECT` | **0** | **0** |
| anchored `^STEP0C ADMISSIBILITY: ` | **0** | **0** |
| unanchored `STEP0C ADMISSIBILITY` (the cross-check on the banner's own `N`) | **0** | **0** |

**The anchored count is 0, so there is no occurrence whose column could be named** — the item's *"published
with the column each occurrence named"* is discharged **vacuously in the only honest sense**: the set is
empty and the emptiness is measured on both anchored and unanchored patterns, which is why the pair is run.
The same 0 / 0 pair is published at §10.1.3 and reproduces here.

---

#### §10.10.8 — ITEM 8 — **GREEN.** THE FROZEN WALK, PER REPLICATE

§10.2.2 shows, for **each** replicate, Step 0's four limbs **each with an evaluated value in the frozen
order with limb (d) FIRST** —

| order | limb | r1 | r2 |
|---|---|---|---|
| 1 | **(d)** ADJ-11 clause 0's conservation identity | `Δpasses 414,110 = 414,110 + 0` → **HOLDS** | `Δpasses 445,682 = 445,682 + 0` → **HOLDS** |
| 2 | **(a)** §1.3's 2×2 | `R51_T 0.346905`, `R52_T 0.800515`, both NOT REJECTED vs 4.303 → **HOLDS** | same inherited controls → **HOLDS** |
| 3 | **(b)** ≥ 1 split recompute | `Δ = 829 ≥ 1` → **HOLDS** | `Δ = 892 ≥ 1` → **HOLDS** |
| 4 | **(c)** population **AND** ADJ-12 readability | leg 1 passes; **leg 2 FAILS** at `1,440/1,440 = 100.000000 %` | leg 1 passes; **leg 2 FAILS** at `1,440/1,440 = 100.000000 %` |

— then **Steps 1, 2, 3, 4 each carrying `NOT EVALUATED` with its reason** (*"Step 0 is fail-closed and
precedes every numbered step"*) and **Step 5 `FIRED ⇒ INDETERMINATE, via Step 0 limb (c)`**. **There is no
published leaf with an unevaluated Step 0 limb**, which is the RED condition the item names.

---

#### §10.10.9 — ITEM 9 — **GREEN.** ADJ-3's BUNDLE, AND TWO LIMBS RE-DERIVED RATHER THAN READ

All five limbs are present at §10.2.4: **(i)** the pass-rate fit, **(ii)** `min`/`median`/`max(L)` with the
`L ≤ B` fraction, **(iii)** `B` with ADJ-12 provenance and its excluded fraction, **(iv)** the per-exit
ledger with its denominator, **(v)** ADJ-11's `Δpasses` beside the window scrape count.

**Limb (i) was re-run rather than re-read**, from the unforked pinned fitter at exactly
`-v col=passes_total -v xaxis=current_epoch -v window=full` over **both derived coordinate slices**, and
all four values reproduce as **RAW `slope_per_x_unit`**:

| | early | late | published |
|---|---|---|---|
| **r1** | `998.978686` | `998.812325` | **both MATCH** |
| **r2** | `998.875930` | `998.977301` | **both MATCH** |

The `k = 0.4 / 0.5 / 0.6` sensitivity is published with no flip at any `k` on either replicate; `B` is
`UNDEFINED (100 % sentinel, 0 non-sentinel rows)` with its excluded fraction beside it; the exit ledger
carries its denominator; `Δpasses` is beside the scrape count. **No limb is omitted on the ground that the
walk stopped at Step 0** — that is precisely what "unconditional" means.

---

#### §10.10.10 — ITEM 10 — **RED.** ONE LINE, AND IT IS A PRE-DATA BYTE AT THE PIN

**The item is a PAIR and must be read as one** — *"one fails by going red on compliant text, the other by
going green on a violation."* Both legs were run over the whole **§10 range (lines 2731 … 5176)**:

**LEG A — every line containing `slope_mb_per_hour` must carry `tombstone_bytes` on the SAME line. FAILS,
on exactly one line of four.**

```
$ awk 'NR>=2731 && /slope_mb_per_hour/ && $0 !~ /tombstone_bytes/'  …/spec356-manifest.md
2957: per replicate**; each row names its window and carries `slope_mb_per_hour`, `se_mb_per_hour`, `r2`.
```

Total `slope_mb_per_hour` lines in range: **4**. Compliant: **3**. Offending: **1** — **line 2957**.

**WHAT LINE 2957 IS.** It is inside **§10.0.4's `P1` entry** — the PRE-DATA presence pre-declaration, whose
*previous* line reads *"the eight-window `tombstone_bytes` fit table, per replicate (16 fits)"*. So the
entry **does** name the series; it names it on the line above, and the obligation's grep is **line-scoped**.
The line is **byte-identical in the manifest at the pin** (`git show 8a60f135:… | sed -n '2957p'` returns
the same bytes), i.e. it entered the record **before the data boundary `0b96e63e`**.

**WHY IT IS NOT FIXED.** §0–§8, §8A, §9 and §10.0 are **byte-frozen** (AC12, checklist item 3, whose
prefix `cmp` would break the instant this line moved), and **R0.3 forbids repairing a pre-declaration byte
after the data boundary regardless of how small the repair is**. Editing it would trade a one-line
imprecision for a broken freeze proof — a strictly worse record. **§10.1.6 measured and disclosed this
line before the checklist was run**, in its own words: *"the manifest carries THREE lines containing that
field name without `tombstone_bytes` beside it, and all three are OUTSIDE §10.1 / §10.2 — two in §9.14 …
and one in §10.0.4's P1 entry, which is a PRE-DATA byte committed at the pin. It is NOT edited here."*
**The checklist's job was to find it independently, and it did.**

**LEG B — `significantly (below|BELOW)|α *= *0\.05|p-value` must be EMPTY over the §10 range. PASSES —
zero matches, and no neutrality-control carve-out was needed.**

```
$ awk 'NR>=2731 && /significantly (below|BELOW)|α *= *0\.05|p-value/'  …/spec356-manifest.md
                                                                       (no output)
```

**THE PAIR-READING, WHICH IS THE ITEM'S ACTUAL PRODUCT.** Leg A **RED on one PRE-DATA line whose neighbour
carries the series name**; leg B **GREEN with an empty result on a genuinely inference-free record**. The
item's own framing — one leg fails by reddening on compliant text, the other by greening on a violation —
resolves here as: **leg A's RED is the "reddening on compliant-in-substance text" mode, and it is reported
as a RED anyway rather than argued down**, because an item that can be talked out of its own output is the
vacuous guard this checklist replaced. **Item 10 is RED.**

*Routing:* **`TODO-634`**, design-phase input — a future spec that re-pins may write P1's assertion with
the series name on the same line, and may consider whether a line-scoped grep is the right instrument for
a two-line sentence. **Neither question is settled here, and no byte moves.**

---

#### §10.10.11 — ITEM 11 — **GREEN.** SIXTEEN FITS, ALL SIXTEEN REPRODUCED

`git diff --stat 8a60f135..HEAD -- '…/spec349c2-fit.awk'` is **EMPTY**; the fitter's digest reads
`840813461e3b1bd5c3a79291044d8ac515e09b94333ee530cd6a10de8fa0436f`, unmoved. Re-running
`awk -v col=tombstone_bytes -v window=full -f spec349c2-fit.awk <segment>` over the sixteen committed
`spec356c-long-r{1,2}-seg{1..8}.csv` reproduces **§10.1.6's published `slope_mb_per_hour`,
`se_mb_per_hour` and `r2` to the printed precision on all sixteen**, together with each window's
`rows_used` / `skipped_empty` (including `r2 W1`'s `59 / 1`).

**16 / 16 EXACT. Zero discrepancies.**

---

#### §10.10.12 — ITEM 12 — **RED.** FOUR LEGS STAND; THE **RUN-3** LEG DOES NOT

**RUN 1 — v1, UNMODIFIED, over `spec356-long`. GREEN.** `sh ./spec356-slottruth.sh .`, exit **0**.
**PD-3 reproduced:** `EXIT_SHARE_PCT 0.000000`, **no `MEDIAN_L_OVER_B` line**, driver `B` absent.
**v1's digest reproduces `280f7a3466a0bffc89059815bb3862bb9581cb95d7efa3fe58b1152749b8f060`.**

**RUN 2 — v2, DEFAULT PARAMETERS, the SAME committed bytes. GREEN.** Exit **0**, and it emits
**`B = 999.944741`**, **`B_SENTINEL_EXCLUDED_PCT = 98.333333`** and **`MEDIAN_L_OVER_B = 0.000000`**.

**AND THE REGRESSION PROOF: `diff` of run 1 against run 2 shows ONLY the `B`-derived lines.**

```
3a4,6
> B                        999.944741
> B_SENTINEL_EXCLUDED_PCT  98.333333
> MEDIAN_L_OVER_B          0.000000
```

Three added lines, **zero changed, zero deleted** — every other slot is byte-identical, which is exactly
what R4.3's matrix exists to demonstrate.

**RUN 3 — v2 over the REPEAT's cells. RED, exit 3 on both replicates.**

```
$ sh ./spec356c-slottruth-v2.sh . spec356c-long-r1 28800
SLOTTRUTH FAIL: spec356c-long-r1.matrix.txt does not name cell long        exit=3
$ sh ./spec356c-slottruth-v2.sh . spec356c-long-r2 28800
SLOTTRUTH FAIL: spec356c-long-r2.matrix.txt does not name cell long        exit=3
```

**This is §10.2.6's Finding 1, reproduced independently by the checklist.** The subject's `prov` call
parameterises the **basename** and the **duration** but left the **cell-name literal `long` hardcoded**,
while the repeat's cells self-describe as `long8h_r1` / `long8h_r2`. **The driver is NOT edited and run 3
is NOT re-run** — it is a digest-pinned sidecar (`a95f49fa…a348`, verified unedited *before* execution) and
R0.3 forbids repairing an instrument after the data boundary. **So the item's own words — "Run 3 produces
the truth file §10 is graded against" — are NOT satisfied, and the item is RED.**

**THE `-U0` REGENERATION LEG — GREEN.**

```
$ diff -U0 spec356-slottruth.sh spec356c-slottruth-v2.sh          -> 55 lines
$ grep -v '^#' spec356c-slottruth-v1-v2.diff                      -> 53 lines
$ tail -n +3 <regenerated> | cmp - <committed body>               -> SILENT
```

The **only** two lines not present in the committed artifact are the `---` / `+++` file header, which
carries filesystem **mtimes** and is not reproducible by construction; every remaining byte of the
committed diff body **is** `diff -U0` output. *(The committed `.diff` interleaves `#`-prefixed annotation
with the raw body; at `-U0` there are no context lines, so stripping `^#` isolates the body exactly. A
changed comment line still begins `-` or `+` and survives the strip.)* Digest
`0101a7d676b5ab4ee35e017a72e8bf92a4e8aaf513a846da24f3facf512fc42e`, matching §10.0.6.

**THE TAXONOMY LEG — GREEN. Four hunks, exactly one label each, every label passing its own mechanical
test:**

| hunk | header | labels standing before it | label | its own test | result |
|---|---|---|---|---|---|
| 1 | `@@ -2,2 +2,8 @@` | **1** | **C** | every changed line is a comment after leading whitespace → **0 non-comment changed lines of 10** | **PASSES** |
| 2 | `@@ -22,4 +28,4 @@` | **1** | **B** | v1 lines **22–25** ⊂ R4.2's five (`:22 :23 :24 :25 :77`), and `d == b` (4 → 4, **zero added**) | **PASSES** |
| 3 | `@@ -77 +83 @@` | **1** | **B** | v1 line **77** ∈ the five, and `1 → 1`, **zero added** | **PASSES** |
| 4 | `@@ -259,6 +265,23 @@` | **1** | **A** | v1 range **259–264** ⊂ the published class-A bound `:258-265` | **PASSES** |

`grep -c '^@@'` = **4** and `grep -cE '^# -+ class [ABC] -+$'` = **4**. **No unlabelled, doubly-labelled or
unlabelable hunk.**

**NET: four legs GREEN, the run-3 leg RED ⇒ ITEM 12 IS RED.** *Routing:* `TODO-648` (the pinned-sidecar
defect class) and `TODO-634` (design-phase input), identically to §10.2.6's Finding 1. **Nothing is
repaired.**

---

#### §10.10.13 — ITEM 13 — **GREEN.** BOTH LIMBS: REPRODUCTION **AND** THE PRE-DATA DRY RUN

**Builder digest checked BEFORE execution**, exactly as the graders are:
`2301ca20fec26400826d3d10444b2a7fca773339d3266e29b04092d0ad33cd08`, matching §10.0.6.

**LIMB 1 — the ten target CSVs, re-run from the committed builder over the committed `prune.csv` into a
scratch directory and compared byte-for-byte:**

```
spec356c-targets.sh <scratch> spec356c-long-r1 spec356c- -r1     exit 0
spec356c-targets.sh <scratch> spec356c-long-r2 spec356c- -r2     exit 0
```

| | T1 | T2 | T3 | T4 | T5 |
|---|---|---|---|---|---|
| **r1** | `cmp` OK | `cmp` OK | `cmp` OK | `cmp` OK | `cmp` OK |
| **r2** | `cmp` OK | `cmp` OK | `cmp` OK | `cmp` OK | `cmp` OK |

**10 / 10 byte-identical.**

**LIMB 2 — R3.5's dry run, the limb that reproduction alone cannot supply** (*a wrong window reproduces its
wrong table perfectly*): the five `spec356c-dryrun-*-long.csv` re-run from the **same** committed builder
over the committed `spec356-long.prune.csv` — **5 / 5 byte-identical**, so **15 / 15 overall** — with their
**pre-declared shapes measured, not assumed**:

| Pre-declared at §10.0.8 | Observed now |
|---|---|
| **T3: 14 `FULL` + 1 `PARTIAL`**, the `PARTIAL` **last** | **14 `FULL`, 1 `PARTIAL`**, and the last data row's `window_kind` **is** `PARTIAL` |
| **T4: 53 whole-run / 2 coordinate-last-half** | `SUBJECT_WHOLE_RUN` **53**, `PRIOR_ARM_WHOLE_RUN` **53**, `SUBJECT_COORD_LAST_HALF` **2**, `PRIOR_ARM_COORD_LAST_HALF` **2**; the two `COORD_LAST_HALF` rows carry `discriminant_role = THE_DISCRIMINANT`, the whole-run rows `PUBLISHED_NEVER_THE_DISCRIMINANT` |

**AND THE ORDERING:** all five dry-run CSVs were added at **`8a60f135`**, a **strict ancestor** of the
boundary `0b96e63e`. **No repeat clock started before this dry run existed**, so AC19's RED condition does
not arise.

---

#### §10.10.14 — ITEM 14 — **GREEN**, WITH THE READING JUSTIFIED RATHER THAN ASSERTED

**THE MEASUREMENT FIRST, THE READING SECOND.** `grep 'T1|T2|T3|T4|T5'` over the whole §10 range returns
**61** lines. Where they fall is the entire question:

| Region | matches | Item 14's status for that region |
|---|---|---|
| **§10.2.2 – §10.2.5 — the WALK, the DETERMINATION, the BUNDLE, the n = 2 statement** | **0** | **the assertion, and it holds absolutely** |
| §10.3 – §10.7 — the target subsections | 17 | **carved out by the item** |
| §10.8.6 — the escalation's routing sentence | 3 | **carved out by the item, by name** |
| §10.0.1 / §10.0.2 / §10.0.4 / §10.0.5 / §10.0.8 / §10.0.9 — the PRE-DATA pre-declaration | 29 | frozen bytes at the pin (see below) |
| §10.1.3 | 2 | **regex false positives** — the ISO-8601 build timestamps `2026-08-11T17:15:26Z` and `…T17:16:44Z`, in which `T1` matches the hour field. **No T-table is referred to.** |
| **§10.2.1 — the PRESENCE limb, `P9`'s entry** | **5** | **the question the item forces; answered below** |

**THE SUBSTANTIVE ASSERTION IS MET WITHOUT ARGUMENT: the walk, the determination, the bundle and the
replication statement contain ZERO occurrences.** No limb reads a T-table, no step reads one, no slot reads
one. That is what target/predicate separation *is*.

**THE §10.2.1 QUESTION, DECIDED AND JUSTIFIED.** The five matches are `P9`'s PRESENCE entry, and every one
of them refers to the **literal anchor strings** `T1.`…`T5.` whose occurrence count **in a different
artifact** (`spec356-verdict-xask.md`) was measured as **ZERO**. **The reading taken here: a
presence-grading observation about a source artifact is NOT a target citation inside a limb, a step or a
slot.** Four reasons, offered as reasons and not as an assertion:

1. **It carries no T-table value.** The item's hazard is a **T-table column** reaching a limb, step or
   slot. `P9`'s five lines carry no column, no row and no number from any T-table — the only number they
   carry is a **count of string occurrences in a markdown file**, and that count is `0`.
2. **`P9` decides nothing about the round.** R6.4 makes the PRESENCE limb categorically separate: it is
   *"evaluated FIRST"*, before any content is read, and its output is a verdict about **artifacts**. A
   determination cannot be contaminated by a limb that runs before the walk and feeds nothing into it.
3. **The structural check is independent of the reading.** Whatever one decides about §10.2.1, the walk's
   own count is **0**. The separation the item protects is intact **either way** — which is why this
   subsection publishes the region table before the argument.
4. **The strict-literal reading is self-refuting, and that is decisive.** Read *"empty outside the target
   subsections and the escalation's routing sentence"* with no further latitude, and the item REDs on
   **§10.0.2 — a section whose own title is "THE FIVE TARGET DISPOSITIONS"** and which carries 15 matches
   as **PRE-DATA bytes committed at the pin**. Under that reading item 14 **cannot be green for any
   possible execution**, which is PD-8's vacuity defect with its sign flipped: a check that cannot pass is
   as uninformative as one that cannot fail. **A reading that makes the item unsatisfiable by construction
   is not the reading the item's author can have meant**, and the item's own first sentence — *"no sentence
   in §10's **classification subsections** cites a T-table"* — names the scope that makes it satisfiable.

**VERDICT: GREEN.** *Recorded and routed, not repaired:* the item's grep pattern is **unanchored and
matches ISO-8601 hour fields** (§10.1.3's two hits), so it over-matches on any record carrying build
timestamps. That is an **instrument** observation, POST-DATA, and it is routed to **`TODO-634`**'s design
phase — a future spec that re-pins may anchor the pattern (`T[1-5][.: ]`) and scope it to the classification
subsections by name. **The checklist is not patched here.**

---

#### §10.10.15 — ITEM 15 — **GREEN.** ESCALATION FORM

The determination **is** `INDETERMINATE`, so R7.2's six items are the applicable branch, and **all six are
present, in order**: §10.8.1 item 1 (the unclassified cause, named) · §10.8.2 item 2 (every step's evaluated
value, per replicate, in the frozen order) · §10.8.3 item 3 (the blocking admissibility limb, named by
letter, against its frozen threshold) · §10.8.4 item 4 (PD-4's coupling on this round's numbers) · §10.8.5
item 5 (§8.3 quoted verbatim) · §10.8.6 item 6 (the routing sentence).

**§8.3 IS VERBATIM, AND IT WAS CHECKED BY `diff` RATHER THAN BY EYE.** Extracting §8.3's source (manifest
lines 467–478) and each block quote, stripping only the `> ` marker:

```
$ diff <§8.3 source> <§10.2.3's block quote>   -> SILENT   (adjacent to the determination)
$ diff <§8.3 source> <§10.8.5's block quote>   -> SILENT   (escalation item 5)
```

**Byte-identical in both places.** ADJ-4's bound is quoted rather than paraphrased at §10.2.3, and the
n = 2 replication statement is at §10.2.5 — published **in addition**, not instead, since both replicates
returned the same determination.

---

#### §10.10.16 — ITEM 16 — **GREEN.** GRADED BY CONTENT, WITH FIVE LIMBS AND SIX TRANSCRIPTS

**The git form of this item is structurally vacuous and was not used.** `.specflow/` is gitignored at
`.gitignore:111`, no path under it is in the index, and `git diff <pin>..HEAD -- '.specflow/todos/'` is
**empty for every possible execution** — it could not detect a ticked box, a moved box, or an edit to
`TODO-637`/`638`/`648`. **The content grade is §10.9, and it is GREEN on all five limbs:**

| limb | assertion | observed |
|---|---|---|
| **(a)** | the three unrelated digests byte-identical to §10.0.7's | **3/3 MATCH** |
| **(b)** | `TODO-634.md`'s digest **HAS MOVED** | `555c3aa9…4f8c` → **`a1b17d97…9428`** |
| **(c)** | tick counts still **2 / 5**, census **7 / 0**, strict anchor still matching | **all five sub-checks hold** |
| **(d)** | before/after published side by side; after-text carries the outcome **and** the `spec356-manifest.md §10` pointer | **published at §10.9.3; both literals present** |
| **(e)** | every changed line inside the §8.1 box | **1 hunk `@@ -233,0 +234,16 @@`, 0 hunks outside** |

**BOTH DIRECTIONS ARE DEMONSTRATED, WITH ALL SIX TRANSCRIPTS ON THE RECORD** — five PRE-DATA in
`spec356c-trackergrade-proofs.txt` (RED on (b)(d); RED on (c)(d); RED on (a); RED on (e) with (a)–(d)
green; GREEN on all five) and the sixth, wave 5's real run, in `spec356c-trackergrade-run6.txt`
(`VERDICT: GREEN (a, b, c, d, e)`, exit 0). **An item 16 reported green without both transcripts on the
record would itself be RED**; both are on the record, and the grader was digest-checked (3/3 against
§10.0.6) before it was run.

**AC15's other clauses, checked directly:** `TODO-634` is **not closed, not deleted, not ticked**;
`TODO-637`, `TODO-638` and `TODO-648` are **byte-unchanged**.

---

#### §10.10.17 — ITEM 17 — **GREEN.** CATALOG

```
$ scripts/check-invariants.sh
invariants: 21 entries, 4 NAKED (baseline 4)
$ echo $?
0
$ git diff --stat 8a60f135..HEAD -- INVARIANTS.md      -> EMPTY
$ git diff --stat feb85268..HEAD -- INVARIANTS.md      -> EMPTY
$ grep -A8 'TG-OR-005' INVARIANTS.md | grep Status
- **Status:** `open (TODO-634)`.
```

`TG-OR-005` still reads **`open (TODO-634)`**, which is the correct state: this round produced a second
observation, not a closure, and the tracker it names stays open.

---

#### §10.10.18 — ITEM 18 — **GREEN.** BUILD

```
$ cargo fmt --check                                 -> exit 0
$ cargo build --release --bin topgun-server         -> exit 0   (Finished `release` profile)
```

This is the direct check the item exists for: AC2 makes the test and clippy outcomes identical to the pin's
**by construction** (the `.rs` diff is empty in both directions, §10.10.2), so the build is the check that
actually adds information about the merge tree.

---

#### §10.10.19 — ITEM 19 — **GREEN.** CROSS-VENDOR

`spec356c-xreview.md` is committed. It names the **model** (`z-ai/glm-5.2` via OpenRouter), the **exact
invocation**, the **reviewed diff range** (`8a60f135...HEAD`, 2,168 lines / 153,403 bytes over 6 files,
with the excluded pathspec stated and justified), the **cost** ($0.11292806, 81,270 tokens), and the
reviewer's **verbatim** assessment reproduced in full.

**Every finding is marked: 4 findings, 0 `APPLIED`, 4 `RECORDED-AND-ROUTED`** (all four re-verified against
the committed bytes by the executor and all four **true**; each would have required editing a frozen
section, a pinned sidecar or a `.rs` file barred by R0.4), plus **5 attempted cardinal-rule breaks marked
`REFUTED-WITH-REASON`**. **Zero findings were applied to a frozen surface** — which is not a happy accident
but the reason none was applied. The artifact also states its own weakness: no HIGH and no MED finding on a
2,168-line record is a result to be suspicious of.

---

#### §10.10.20 — WHAT THE THREE REDs MEAN, AND WHAT THEY DO NOT

**16 GREEN, 3 RED.** The three REDs are **items 1, 10 and 12**, and they share one property that is worth
stating plainly: **every one of them fires on a byte this spec is forbidden to touch.**

| Item | The RED | The frozen byte behind it | Routed to |
|---|---|---|---|
| **1** | `P9` (anchors absent from `spec356-verdict-xask.md`) and `P13` (POST half absent at that instant) | a **committed sidecar** and the **ordering rule** that item 1 runs first | `TODO-634`; `P13` re-read GREEN at §10.9 |
| **10** | one `slope_mb_per_hour` line without `tombstone_bytes` beside it | manifest **line 2957**, §10.0.4's `P1` entry, **PRE-DATA, at the pin** | `TODO-634` |
| **12** | v2 refuses the repeat's cells, exit 3 | `spec356c-slottruth-v2.sh`, a **digest-pinned sidecar** | `TODO-648` + `TODO-634` |

**WHAT THEY DO NOT MEAN.** None of the three touches the **determination**. The walk's four Step-0 limbs
were all evaluated, in the frozen order, on both replicates, from **committed column 43 read directly** —
not through the refused driver — and Step 0 limb (c) fails on **1,440 / 1,440 = 100.000000 %** on both.
Items 2–9, 11 and 13–19 are green, and they are the items that carry the lineage, the pin, the artifact
binding, the walk, the bundle, the fits and the target derivations. **`INDETERMINATE` on both replicates is
not weakened by these three REDs, and it is not strengthened by the sixteen greens either** — it is what
the frozen predicate returned.

**AND WHAT PUBLISHING THEM COSTS, WHICH IS THE POINT.** Three REDs on a nineteen-item checklist is a worse
headline than nineteen greens, and it is the only honest one available. **A checklist that greens on a
known failure is the exact defect this spec exists to not repeat.** All three were known before the
checklist ran — item 12's from §10.2.6, item 10's from §10.1.6, item 1's from §10.2.1 — and **the checklist
found all three independently anyway**, which is the strongest evidence available that it is an instrument
and not a formality.

---

#### §10.10.21 — **THE GREPS ARE NOT IDEMPOTENT UNDER PUBLICATION, AND THIS IS THE RECORD OF IT RATHER THAN A REWRITE TO HIDE IT**

Items 10 and 14 are **greps over the §10 range**, and §10.10 is **part of §10**. So the act of *publishing
what the greps found* — quoting the offending line, quoting the pattern, naming the T-tables by name —
**changes what the greps find**. This was measured after §10.10 landed, and it is recorded here because the
alternative is worse in a specific and disqualifying way.

**MEASURED, BOTH WAYS:**

| Grep | over §10.1 – §10.9 (the bytes the checklist graded) | over §10.1 – §10.10 (after this subsection's own publication) |
|---|---|---|
| item 10 leg A — `slope_mb_per_hour` lines lacking the series name on the same line | **1** (manifest line 2957, PRE-DATA, at the pin) | **4** — the original, plus **3 inside §10.10** |
| item 10 leg B — the inference vocabulary | **0** | **2** — both inside §10.10, and both are the **pattern itself** being quoted in §10.10.10's report of leg B |
| item 14 — `T1|T2|T3|T4|T5` inside the walk subsections §10.2.2 – §10.2.5 | **0** | **0** — unchanged; §10.10's own 6 matches are in the checklist report, not in a limb, step or slot |

**WHICH READING THE VERDICTS REST ON, STATED EXPLICITLY: the first column.** Items 10 and 12's verdicts are
**measurements of the record the checklist was run against**, which is §10.1 – §10.9. A grep cannot grade
the report of its own output without regress — the report of the report would move it again — and the
verdicts are already published as **RED** on the first column, so nothing here converts a RED into a GREEN
or the reverse. **Item 10 is RED either way. Item 14's core assertion (0 in the walk) is GREEN either way.**

**WHY THE OBVIOUS ALTERNATIVE WAS REFUSED.** §10.10.10 could have been written so its own text does not
trip the grep — paraphrase the offending line instead of quoting it, describe the leg-B pattern instead of
printing it. **That is writing the record to satisfy the instrument rather than to report the measurement,
and it is the same failure class as a checklist that greens on a known failure.** A reader who cannot see
the offending line verbatim cannot check the finding; a reader who cannot see the pattern cannot re-run the
leg. **The quotes stay, and the arithmetic they cause is published here instead of being quietly absorbed.**

**AND THE HONEST RESIDUE:** this means **item 10's grep, run naively over the whole §10 range at the merge
tree, returns 4 rather than 1**, and a future reader who runs it will get 4. That reader should land here.
*Routing:* **`TODO-634`**, design-phase input — a checklist grep whose subject includes its own report needs
either an explicit exclusion of the checklist subsection or a snapshot boundary, and **which of those is
right is not settled here**, because settling it would be patching a frozen checklist, which R0.3 forbids
and this spec's own hard constraints bar.
