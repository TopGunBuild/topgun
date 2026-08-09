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
