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
