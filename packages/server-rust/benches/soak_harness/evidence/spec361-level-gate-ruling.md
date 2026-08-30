# SPEC-361a — the durable-layer LEVEL/CEILING gate: ruling artifact

Two layers, and the distinction is load-bearing.

- **The FROZEN layer** (below, between the `FROZEN-LAYER` sentinels) is recorded **before** any
  calibration test is written and **before** any control run is executed. Its digest is pinned in
  the sidecar `spec361-level-gate-ruling.frozen.sha256`, which enters history in the same commit as
  the bytes it names.
- **The APPEND-ONLY post-section**, under its own separate digest, is written **after** `G5`. At the
  time of this commit it is **present and EMPTY** — the marker exists so that a later append is
  visibly an append and not a rewrite.

**Digest command** (the frozen layer is delimited by sentinels, so the digest is stable under any
later append and under any line-number shift):

```
awk '/^<!-- FROZEN-LAYER-BEGIN -->$/{f=1;next} /^<!-- FROZEN-LAYER-END -->$/{f=0} f' \
  packages/server-rust/benches/soak_harness/evidence/spec361-level-gate-ruling.md \
  | shasum -a 256
```

A file cannot contain its own digest, which is why the value lives in the sidecar.

<!-- FROZEN-LAYER-BEGIN -->

## §F0 — what is frozen, and what freezing means

Everything in this layer is decided **in the open and in advance**. Once its digest is recorded:

- no parameter in §F3 may be retuned to make a control run pass **or** fail (`C11`, `KL-4`);
- no `AT` row in §F8 may be re-written after seeing a verdict;
- no witness arm in §F9 may be re-sited after seeing a RED;
- a divergence between a pre-registered text and a delivered text is **recorded in the post-section
  with its reason**, and the frozen table is corrected in the same edit. The pre-registration is a
  freeze, not a trap.

## §F1 — THE DECISION RULE, in evaluation order

`L0` is evaluated first and is fail-closed.

| Clause | Predicate (frozen) | Verdict on breach | Notes |
|---|---|---|---|
| **`L0` — INSTRUMENT** | `scans_failed > 0` **OR** `samples == 0` | **FAIL**, `disposition = InstrumentFailed`, `L1`/`L2` **NOT EVALUATED** | A scan that returned `None` — missing file, corrupt header, a table-open error other than `TableDoesNotExist` — **or a byte copy that failed (§F15)** is a **blind instrument**, and a gate whose instrument was blind must not report bounded growth. `Some(0)` from a never-written table (`main.rs:1238`) is an **honest zero** and does NOT breach. |
| **`L1` — LEVEL / FLATNESS** | evaluated only when `samples >= min_samples` **and** `span_secs >= min_span_secs`; breaches when `last_half_peak_bytes.saturating_sub(first_half_peak_bytes) > headroom_bytes` | **FAIL**, `disposition = LevelEvaluated` | The hard gate's decision function. `saturating_sub` means a **falling** corpus yields `rise = 0` and passes — correct for a ceiling test. When the guards are not met: `disposition = LevelSuppressed`, `passed` unaffected by this clause, and the suppression is **rendered with both numbers** (`KL-5`). |
| **`L2` — ABSOLUTE CEILING** | evaluated only when `ceiling_bytes == Some(c)`; breaches when `peak_bytes > c` | **FAIL** | **DISARMED BY DEFAULT (`None`)**: arming it honestly requires a validated ceiling, which requires a plateau demonstration that `PIN 2` puts out of scope. It is implemented, **unit-tested (`W9`)**, renderable and armable from the CLI, and both CLI knobs are proven effective over the rendered transport by §F13's probe pair (`X21-d`). Its `None` renders as `ceiling=disarmed` on the report line and as an EXPLICIT `"ceilingBytes": null` in the JSON — **never an omitted key**; its `Some(c)` renders `ceiling=<c>` and `"ceilingBytes": <c>`, and **both** branches are exercised (`W9`; the census-target fixture, §F11). |

The window partition is the **shared split index** factored out of `last_half_window`
(`monitor.rs:281`), so the new clause and the demoted slope clause split any series identically and
a reader comparing them is comparing like with like.

## §F2 — TWO SPANS, NAMED APART; AND THE ADMISSIBLE DURATION RANGE

`span_secs` on the assessment is the **FULL series span** — `last.elapsed_secs − first.elapsed_secs`
— and that is what `min_span_secs = 600.0` guards. The **rise** `L1` tests is the difference between
two half-peaks, so the growth it accumulates spans roughly **half** the series. The two are never
interchanged, and neither is ever quoted as the cell duration.

At `--crash-interval 120` (the harness default, `main.rs:205`) a cell of duration `D` puts its first
checkpoint at ≈ 120 s and its terminal scan at ≈ `D`, so:

```
span_secs  ≈  D − 120              (terminal minus the FIRST checkpoint, not minus t = 0)
samples    =  floor(D / 120) + 1   (checkpoints + the terminal scan)
```

`L1`'s two guards are `span_secs >= 600` and `samples >= 4`. The sample guard binds at `D >= 360`;
the **span guard binds at `D − 120 >= 600`, i.e. `D >= 720 s`**, and it is therefore the binding
one. The lower bound is pinned at **730 s**, adding a **10 s cushion** over the derived 720 s for
the strict `>=` comparison and for a first checkpoint that fires marginally late.

> **ADMISSIBLE DURATION RANGE: `730 s ≤ D ≤ 900 s`. The three control cells are RUN at `D = 900 s`**
> — the `C9` ceiling — because that maximises churn and leaves the largest guard margin, and
> **all three cells use the identical `D`** (the neutral run is the attribution control and a
> duration difference would destroy it).

At `D = 900`: **`samples = 8`, `span_secs ≈ 780 s`**, clearing the 600 s guard with ≈ 180 s of
margin and giving a **≈ 390 s** rise window — the window every magnitude in §F3 is scaled to. At the
lower bound `D = 730`: `samples = 7`, `span ≈ 610 s` — still admissible. **A cell below 730 s is
inadmissible BY CONSTRUCTION**, so spending `AT-0`'s single permitted re-run on one would waste the
only re-run the row allows.

## §F3 — THE PARAMETERS AND THEIR DERIVATION

```rust
pub const DEFAULT_TOMBSTONE_CORPUS_HEADROOM_BYTES: u64 = 65_536;
pub const DEFAULT_TOMBSTONE_CORPUS_MIN_SPAN_SECS: f64 = 600.0;
pub const DEFAULT_TOMBSTONE_CORPUS_MIN_SAMPLES: usize = 4;
pub const DEFAULT_TOMBSTONE_CORPUS_CEILING_BYTES: Option<u64> = None;
```

*An estimator whose discrimination is inside an 8,756 B/h spread is not an estimator* — so the
derivation is published as arithmetic, not as a preference. **Every number below is either quoted
from a committed source with its `file:line`, or computed from two such numbers with the arithmetic
shown on this page. The derivation is satisfiable from the stated numbers ALONE — no external
lookup, no unstated bridging quantity.**

**The common denominator, stated once so all steps compare like with like.** `L1`'s statistic is a
**level rise over the LAST-HALF window** of the CORPUS SERIES. The control cells run at `D = 900 s`
at `--crash-interval 120` (§F2, §F8), so the series spans ≈ 780 s and its last-half window is
**≈ 390 s = 0.108333 h**. Every step is normalised to that 390 s window; where a source number was
measured over a different window, the conversion is shown. **The `450 s` an earlier draft used is
superseded** (it assumed the series span equalled the cell duration, at the old `--crash-interval
60`); every figure derived from it is re-computed below and none of the conclusions moves, because
the healthy-to-unhealthy separation is a **ratio** and is window-independent.

### Step 0 — THE MEASURED DURABLE-CORPUS MAGNITUDES

Every committed harness console log renders a terminal `tombstone_corpus_redb_scan:` line, and the
evidence directory holds **29 of them across 29 files / 21 distinct runs** at HEAD `20edabde`:

```
grep -rn 'tombstone_corpus_redb_scan:' \
  packages/server-rust/benches/soak_harness/evidence
```
⇒ **29 lines.** Each renders the scanned corpus **and** the last gauge value, so each is a paired
observation of exactly the substitution this derivation makes. Three, with their `file:line`:

- `spec355-w1000.harness-console.log:270` — **628,903 B scanned vs 646,306 B gauge**, absolute gap
  **17,403 B**, i.e. `17,403 / 646,306 =` **2.7 %**;
- `spec356c-long-r1.runner-console.log:57` — **4,969,274 vs 4,994,851**, gap **25,577 B**,
  `25,577 / 4,994,851 =` **0.5 %**. *(An earlier draft attributed this pair to `spec349c2`;
  CORRECTED — `spec349c2-emitter-on.harness-console.log:80` is **200,860 vs 194,260** and
  `spec349c2-emitter-off…:80` is **239,256 vs 245,809**.)*
- the `w100` class, e.g. `spec356-w100.harness-console.log:48` — **20,108 vs 17,996**.

**What these DO and DO NOT license.** Each is a **single terminal scalar**, so **no durable-corpus
NOISE FLOOR has been measured** and the honest-gap statement below survives verbatim. What they
*do* bound is the **proxy-substitution error** — the error incurred by deriving a byte headroom from
GAUGE magnitudes and applying it to CORPUS magnitudes. Over all 21 runs the **absolute**
corpus-vs-gauge gap never exceeds **27,925 B** (`spec357-diag-r1.runner-console.log:57`, 4,659,548
vs 4,687,473); excluding the two `SPEC-357` cells it never exceeds **25,577 B**. So
`65,536 / 27,925 =` **2.3×**: the headroom is over twice the largest substitution error ever
observed, across three orders of magnitude of corpus size (19,536 B → 5,234,623 B). *(Quoting
`spec357-diag-r1`'s **scan line** imports none of the `PD-F12` counter-family figures struck in
step 3: the scan is an independent redb read, not a counter.)*

**The relative gap is NOT uniform** — 0.5–3 % on the ≥ 400 KB cells, but tens of percent on the
~20–50 KB cells (e.g. `spec356-ctloff-r1…:48`, 50,204 vs 37,400). That is why the bound is stated in
**BYTES and not as a percentage**: a percentage bound would be false, an absolute one is what `L1`
actually consumes (`KL-1`).

> **THIS STEP CORROBORATES; IT DOES NOT RETUNE. `headroom_bytes` stays 65,536 — the number was
> fixed by steps 1–3, these lines are NOT a floor and NOT a retune input, and `C11` forbids moving
> the constant to fit an observation.**

### Step 1 — the recorded rate spread converted to a LEVEL

The 8,756 B/h width-100 spread was fitted over last-half windows of **900 s = 0.25 h** — attributed,
not assumed: the `spec356-w100` cell ran **1,800 s** (`spec356-manifest.md:1634`) and its coordinate
last-half window is **90 rows / 900 s** (`spec356-manifest.md:1961-1962`). A rate spread `S` over a
window of `T` hours is a level spread of `S × T`:

```
8,756 × 0.25      = 2,189 bytes      (at the source's own 900 s window)
8,756 × 0.108333  =   948.6 bytes    (normalised to the control leg's 390 s window)
```

**2,189 B, not 8,756, is the primitive noise magnitude.** The headroom sits
`65,536 / 2,189 =` **29.9×** above the as-measured figure and `65,536 / 948.6 =` **69.1×** above the
normalised one.

### Step 2 — above the healthy signal

`SPEC-356b`'s `w100` keeping-up cell recorded a backlog delta of **+8,602 B**, and the **window is
the bridging fact**: that delta is over the cell's **coordinate last-half window of 90 rows /
900 s**, not over the whole cell (`spec356-manifest.md:1961-1964`).

```
at the source's own 900 s window:   65,536 / 8,602               = 7.6×
pro-rated to 390 s:                 8,602 × 390 / 900 = 3,727.5 B
                                    65,536 / 3,727.5             = 17.6×
```

**The `~150×` an earlier draft carried is DELETED — no committed number produces it.**

### Step 3 — below the unhealthy signal, at the control leg's own window

The `long` cell's coordinate last-half window carried **+5,303,731 B** of backlog growth. That
window's span is **7,190 s** (`spec356-manifest.md:1660-1661` — the figure completes the sentence at
`:1661`; also `:1927`, `:2173-2174`), which is the *"~2 h"* an earlier draft left unattributed. So:

```
5,303,731 / (7,190 / 3,600)  = 2,655,565 B/h  ≈ 2.66 MB/h
2,655,565 × 0.108333         =   287,686 B    (over the control leg's 390 s window)
287,686 / 65,536             =        4.4×    the headroom
```

*(`SPEC-357`'s `+10,131,707 / +8,250,744 B` diagnosis figures are **NOT** derivation inputs and are
struck from this step: they sit inside `PD-F12`'s unreconciled counter-family contradiction
(`spec357-fixshape-ruling.md:173-183`), so quoting them would import a contested number into a
frozen derivation.)*

### The separation

All three at the control leg's own 390 s window:

```
healthy ≈ 3,727.5 B   <   headroom 65,536 B   <   unhealthy ≈ 287,686 B
17.6× above healthy       4.4× below unhealthy
total separation:   287,686 / 3,727.5 = 77.2×
```

Roughly **one order of magnitude of margin on each side**, justified **before** any control is run.
**The 77× separation is window-INVARIANT** — it is a ratio of two quantities pro-rated by the same
factor — which is why re-basing 450 s → 390 s moved the two one-sided margins
(`15.2×` → `17.6×`, `5.1×` → `4.4×`) and left the separation untouched. **No claim of "two orders of
magnitude" survives; the arithmetic above does not support one.**

### The honest gap, stated rather than glossed

Every number in steps 1–3 is a **gauge / counter** magnitude. **No durable-corpus noise floor has
ever been measured in this lineage.** Step 0's 29 committed scan lines are single terminal scalars,
one per run, so they bound the **substitution error** (≤ 27,925 B, i.e. `2.3×` inside the headroom)
but constitute **no floor**, because a floor requires repeated samples of the same configuration.
They are used as the best available proxy, and one property of the new instrument makes the proxy
conservative: the durable scan is an **exact byte count of a deterministic on-disk state**, not a
fit — it has no estimator variance at all, and its only per-sample wobble is genuine state change
between scans. Revising `headroom_bytes` on measured durable-corpus data is `TODO-654`'s to
propose, and any revision is a **spec revision, recorded**, never a keyboard retune (`C11`).

### The other three parameters

- `min_span_secs = 600.0` — five times `DEFAULT_TOMBSTONE_BYTES_MIN_WINDOW_SECS`
  (`monitor.rs:365`), far above the 25 s blocking smoke, far below any real soak. It is the binding
  guard behind §F2's admissible range.
- `min_samples = 4` — two per half, so no half-peak is a single point: the degenerate case the
  existing `last_half_window_span_secs` guard excludes for the same reason — the **executable**
  guard, the `match` at `monitor.rs:309-312`, not the doc paragraph at `:290-306` an earlier draft
  cited.
- `ceiling_bytes = None` — `L2` disarmed by default, for the reason §F1 states.

## §F4 — THE STATISTIC CHOICE: PEAK, NOT MEAN, NOT MEDIAN, NOT A FIT

- **A LEVEL, in the quantity's own units (`KL-1`).** The 8,756 B/h spread is the `1/T` amplification
  of a level spread, not an independent noise source. Any statistic that normalises by time
  re-imports it. `L1` compares **bytes to bytes** and fits nothing.
- **THE PEAK, not the mean and not the median (`KL-2`).** A ceiling is an **upper envelope**: prune
  legitimately produces large downward excursions, which would drag a mean or a median and hide a
  rising envelope. `L2` is likewise `peak_bytes > c` and not `last_bytes > c` — an envelope test,
  not a level-vs-final test; `W9`'s mutation arm is exactly that substitution.
- Using the peak also leaves `R-5`'s `median(L)` successor rule (*"publish the zero-row fraction and
  RED within ±5 points of 50 %"*) with **no subject here**, which is stated so the rule is visibly
  honoured rather than silently skipped.
- **NEITHER DISCREDITED FORM IS REVIVED (`§7 F3` of the extraction synthesis).** There is no
  last-half OLS **slope stick** and no **`f(span, width, churn)`**: `L1` is a level comparison in
  bytes, and none of its three parameters is a function of span, width or churn.

## §F5 — WHAT IS DEMOTED, AND WHAT IS NOT

**THE DEMOTION IS CONDITIONAL ON REPLACEMENT, NEVER UNCONDITIONAL. This is the governing rule and it
outranks every convenience below it.**

> **NO-UNGATED-WINDOW RULE (normative).** *There exists no run configuration in which neither the
> old hard slope clause nor a live, evaluable `L1` gates tombstone growth.* A demotion that opens an
> ungated run class is the same defect class as deleting a fence because a better one is planned,
> and this spec refuses it.

- **CONDITIONALLY DEMOTED:** the tombstone-byte **SLOPE** clause. `assess_tombstone_bytes`'s
  `passed` (`monitor.rs:470`) stops being ANDed at `main.rs:853` **exactly and only when
  `corpus.disposition == LevelEvaluated`** — only where a live `L1` decided over the *same run* and
  therefore covers the same class. Its value is still computed, printed and serialized in every
  case (`C8`).
- **STAYS HARD — the class `L1` cannot cover.** When `corpus.disposition != LevelEvaluated` the
  slope clause is ANDed exactly as at HEAD. **The run class, explicitly:** any run whose corpus
  series fails `L1`'s guards — `samples < min_samples` **or** `span_secs < min_span_secs`.
  Concretely: every run with `--crash-interval` disabled, or shorter than `min_samples − 1` crash
  intervals, or shorter than `min_span_secs`. The corpus sampler is coupled to the recovery
  checkpoint (§F15), so such a run yields **one** sample and `L1` is **structurally SUPPRESSED**.
- **NOT DEMOTED:** the gauge's **blind-monitor** clause (`main.rs:830`, `:863-867`). It stays hard
  unconditionally — it asserts **harness health** (a dead `/metrics` scrape), not the tombstone
  property.
- **NOT TOUCHED:** the RSS gate (`assess`), the disk gate (`assess_disk`) and its blind-monitor
  clause, the convergence gates, the recovery gates and the panic gate (`C7`; `SPEC-348` owns
  promoting the disk + RSS slopes).
- **NEW HARD CLAUSES:** `L0` (unconditional) and `L1` (whenever evaluable).

Net, the run verdict at `main.rs:849-855` becomes:

```
convergence ∧ recovery ∧ mem ∧ ¬blind_monitor
  ∧ corpus.passed                                             ← L0 always; L1 when evaluable
  ∧ (¬slope_clause_stays_hard(corpus.disposition) ∨ tombstones.passed)
                                                              ← the fallback: the OLD hard clause
  ∧ ¬disk_blind_monitor ∧ ¬panic
```

`L1` evaluated ⇒ the slope is report-only; `L1` not evaluated ⇒ the slope is hard, unchanged from
HEAD. `L0` failing already forces `corpus.passed = false`, so `InstrumentFailed` fails the run
through the **first** conjunct and never reaches the second.

## §F6 — THE EXHAUSTIVE PREDICATE, AND WHY IT IS SITED IN `monitor.rs`

`slope_clause_stays_hard` is a `const fn` whose body is a `match` over all three
`CorpusLevelDisposition` variants **with no `_` wildcard arm**. `main.rs` re-types **no** disposition
comparison of its own. Three properties, each load-bearing:

1. **A FOURTH VARIANT FAILS TO COMPILE (E0004).** The guard is `rustc`, not a reviewer's reading.
   An inline `corpus.disposition == LevelEvaluated ∨ tombstones.passed` would enumerate nothing: a
   fourth variant would silently fall into the *"slope stays hard"* side.
2. **IT IS SITED WHERE AN INCLUDER ALREADY RUNS IT.** The graded assertion is a calibration test,
   `calibration_slope_clause_hard_for_every_non_evaluated_disposition`, in `monitor.rs`'s existing
   inline `#[cfg(test)] mod tests`. `monitor.rs` is `#[path]`-included by
   `tests/soak_monitor_calibration.rs:15-17` (and, under §F11, by `tests/soak_wal_census.rs`), so
   the test runs under `cargo test --all-targets` with **no** new `mod`, **no** new `.rs` and **no**
   edit to any includer.
3. **IT COULD NOT HAVE BEEN SITED OVER `main.rs`'s GATE EXPRESSION — a VERIFIED FACT, not a
   preference.** At HEAD **no `tests/` target `#[path]`-includes `main.rs`** (the complete includer
   set is `soak_or_noloss.rs` → `or_noloss.rs`; `soak_wal_census.rs` → `report.rs`;
   `soak_monitor_calibration.rs` → `monitor.rs`; `soak_tombstone_restart.rs` → `or_noloss.rs` +
   `client.rs` + `process.rs`), and `main.rs` has **no** inline `#[cfg(test)] mod tests`.

The test asserts, over an array literal of all three variants, that
`slope_clause_stays_hard(LevelEvaluated) == false` and that it is `true` for `LevelSuppressed` and
`InstrumentFailed`; and separately that an `InstrumentFailed` assessment already carries
`passed == false`, so that variant fails through the FIRST conjunct and never depends on the
fallback.

**NO NEW MUTATION ARM IS OWED HERE, and that is stated rather than left as a gap.** The fallback
conjunct is a **wiring** term, not an estimator term:

- **Constructibility:** the 25 s blocking Soak Smoke run reaches the fallback branch on every
  execution — one corpus sample ⇒ `LevelSuppressed` ⇒ the slope conjunct is live. `AC21` asserts
  that over the rendered line, so the branch is demonstrably reachable, never vacuous.
- **Fireability:** the slope clause's ability to FAIL is already proven by the four pre-existing,
  unchanged calibration tests (`monitor.rs:877`, `:897`, `:916`, `:1032`), kept green and
  arithmetic-unchanged (`C8`).
- **Exhaustiveness:** the falsifier is the compiler. An arm on an exhaustiveness property would
  have to *remove* exhaustiveness (add a `_ =>` arm), which is a refactor, not a mutation of an
  assertion, and would RED nothing. **The honest statement is that this carries a compile-time
  falsifier and no mutation arm — a different claim from "unfireable by construction", and not
  that one.**

## §F7 — THE `"HARD gate"` DOC-CONTRACT CENSUS

The sweep is **census-driven**, not memory-driven: a demotion that corrects the sites an author
happens to remember and leaves the rest asserting a gate that no longer exists produces exactly the
false-invariant hazard this lineage keeps catching. Both files are `SPEC-361a`'s, so the census does
not straddle the carve.

> **COUNTS ARE NORMATIVE; LINE NUMBERS ARE DESCRIPTIVE.** Every HEAD line number below **will
> shift** once the prefix constant, the `FINISHED_REASON_DURATION_REACHED` constant, the shared
> `Vec<CorpusSample>` and the sampler land. A shifted number is **NOT** a review failure. **A census
> site is identified by its TEXT and its owning item, never by its line number**; what is graded is
> the COUNT plus the classification.

**The pinned command and its pinned HEAD count at `20edabde`:**

```
grep -nE 'HARD gate|hard gate|hard-gate' \
  packages/server-rust/benches/soak_harness/monitor.rs \
  packages/server-rust/benches/soak_harness/main.rs
```
⇒ **30 matching lines across 2 files at HEAD: `monitor.rs` 12, `main.rs` 18.**

### TABLE 1 — THE HEAD SITES (30), classified

| File | Lines (grep hits, DESCRIPTIVE) | Disposition |
|---|---|---|
| `monitor.rs` | `:69`, `:81`, `:88`, `:91` (passage `:91-94`) | **CORRECT** — module-doc *"Note on gating responsibility"*; must state the conditional role, and must keep the blind-monitor sentence (`:88`) unchanged in substance |
| `monitor.rs` | `:325`, `:330` | **CORRECT** — `DEFAULT_TOMBSTONE_BYTES_THRESHOLD_PER_HOUR`'s doc-contract |
| `monitor.rs` | `:345`, `:355`, `:357` | **CORRECT** — `DEFAULT_TOMBSTONE_BYTES_MIN_WINDOW_SECS`'s doc-contract; the guard is now *also* what selects the fallback class |
| `monitor.rs` | `:1029`, `:1046` | **CORRECT** — `calibration_additive_only_gauge_never_plateaus`'s doc + assert message |
| `monitor.rs` | `:1236` | **OUT OF SCOPE — DISK** (`C7`). Byte-unedited. |
| `main.rs` | `:25`, `:34` | **CORRECT** — module-doc property 3; `:34` is the blind-monitor sentence and stays true |
| `main.rs` | `:45` | **CORRECT** — module-doc, the two controls |
| `main.rs` | `:165` | **CORRECT** — `Config::no_ack`'s doc |
| `main.rs` | `:790` | **CORRECT** — the assess-tombstone-bytes call-site comment |
| `main.rs` | `:803`, `:826` | **CORRECT** — the rationale block `:803-829`; `:826`'s blind-monitor sentence stays true |
| `main.rs` | `:869` | **CORRECT** — the FAIL arm `:868-880`. Becomes **rank 4 of the single ranked writer** (§F14); re-worded onto the conditional role, assertion text kept verbatim in substance. **One arm, not two.** |
| `main.rs` | `:1051`, `:1056` | **CORRECT** — the summary-line comment and `let tombstone_role = "slope + blind-monitor both hard-gate";`, whose **string value** must now render the conditional role (`X21-a`) |
| `main.rs` | `:1095` | **CORRECT — TEXT ONLY.** A DISK comment making a claim *about the tombstone slope*, so the claim goes false. Only that clause is re-worded; **no disk predicate, constant, assessment or gate conjunct is touched** (`C7`). |
| `main.rs` | `:2426` | **CORRECT** — the `--help` usage text (`# tombstone hard-gate must FAIL`), operator-visible |
| `main.rs` | `:2128`, `:2185` | **REVIEW; correct if false, else record as already-true.** Tracked-client comments naming *"the promoted hard gate"* generically. |
| `main.rs` | `:16` | **OUT OF SCOPE — QUERY/recovery gate**, not tombstone. Byte-unedited. |
| `main.rs` | `:838`, `:1099`, `:1100` | **OUT OF SCOPE — DISK** (`C7`). Byte-unedited. |

**In scope: 25 lines (11 `monitor.rs` + 14 `main.rs`). Out of scope: 5 lines.**
`11 + 14 + 1 + 4 = 30`, so **Table 1 is exhaustive over the pinned grep AT HEAD**.

**NO HEAD SITE IS DEMOTED OUT OF THE CENSUS — `demoted = 0`, and that is a RULE, not an
observation.** Every one of the 25 in-scope sites is re-worded **in place**, and **each re-wording
MUST RETAIN one of the three pinned phrases** (`HARD gate` / `hard gate` / `hard-gate`). A
correction that deletes the phrase would silently shrink the census and remove the site from every
future re-run of the sweep. The 5 out-of-scope sites are byte-unedited and retain their phrases
trivially. **Therefore the HEAD contribution to the post-edit count is `30 − 0 = 30`.**

### TABLE 2 — `AUTHORED-BY-361a`: THE SIX LINES THIS SPEC ITSELF WRITES

Pre-registered with their **exact text**, because a HEAD-only census would make a correct
implementation RED its own gate.

| # | File | Item that carries it | EXACT authored text (the grep-matching line) |
|---|---|---|---|
| `A1` | `monitor.rs` | `assess_tombstone_corpus_level`'s doc-contract | `/// L0 and L1 are HARD gate clauses; L1 only when its two guards are met.` |
| `A2` | `monitor.rs` | `slope_clause_stays_hard`'s doc-contract | `/// Whether the tombstone-byte SLOPE clause still hard-gates a run that` |
| `A3` | `monitor.rs` | `CorpusLevelDisposition::LevelSuppressed`'s variant doc | `/// L1 was NOT evaluated, so the slope clause hard-gates instead. Never "ok".` |
| `A4` | `main.rs` | the comment above the re-pointed verdict conjunction | `// The durable-corpus verdict is the HARD gate; the slope clause hard-gates` |
| `A5` | `main.rs` | `--tombstone-corpus-ceiling-bytes`'s `--help` usage line | `//   --tombstone-corpus-ceiling-bytes <n>  arm L2's absolute HARD gate ceiling` |
| `A6` | `main.rs` | the ranked writer's rank-4 branch comment | `// Rank 4: the slope clause, which hard-gates only when L1 did not decide.` |

**Distribution: `monitor.rs` +3 (`A1`–`A3`), `main.rs` +3 (`A4`–`A6`).** Each states the
**conditional** role by construction; none asserts an unconditional hard gate.

### THE POST-EDIT COUNT IS NORMATIVE

**`30 − demoted + authored = 30 − 0 + 6 =` 36.**

> **Re-running the pinned grep on the branch MUST return exactly `36` matching lines across the same
> 2 files — `monitor.rs` `12 + 3 = 15`, `main.rs` `18 + 3 = 21`, `15 + 21 = 36`. The count is
> NORMATIVE. Every line number in either table is DESCRIPTIVE.**

Exhaustiveness arithmetic over the branch: **25 in-scope HEAD sites (re-worded, phrase retained) + 6
authored sites + 5 out-of-scope byte-unedited sites = 36.**

**THE CENSUS RULE, NORMATIVE:**

> **Every hit the pinned grep returns on the branch is classified either by TABLE 1 (a HEAD site,
> matched by its TEXT and owning item, never by its line number) or by TABLE 2
> (`AUTHORED-BY-361a`). An UNCLASSIFIED hit is a RED, not a judgement call — but a line THIS SPEC
> ITSELF WROTE is never unclassified, because authoring it is what classifies it. What the rule
> forbids is an unaccounted line, not a new one.**

Two corollaries: a Table 1 line classified `CORRECT` that still reads *"is now a HARD gate"*
unconditionally is a **RED**; and an authored line whose delivered text differs from its
pre-registered text is **still classified** (it is `AUTHORED-BY-361a` by construction), but the
divergence is **recorded in this artifact's post-section with its reason** and Table 2 is corrected
in the same edit.

*(`assess_tombstone_bytes`'s own doc-contract `monitor.rs:390-401`, incl. `:397`, already says
*report-only* and contains no grep hit — recorded so a reader does not mistake its absence for an
omission. It is **already correct** in the `L1`-evaluated case.)*

## §F8 — THE ANTI-TAUTOLOGY TABLE `AT`

*A gate that cannot fail is worse than no gate.* Frozen before any control run.

**How the bar is demonstrated — BOTH, split by what each can prove:**

- **Synthetically**, as calibration unit tests in `monitor.rs` (`W1`–`W4`, the slow-ramp test `W5`,
  the armed-ceiling test `W9`). Cheap, deterministic, runs in CI forever — but proves the
  **estimator**, not the wiring.
- **Live**, as **three SHORT control runs of the real harness** (`D = 900 s` each, the same `D` for
  all three, inside §F2's admissible range, at the harness **default `--crash-interval 120`**,
  `main.rs:205`): `--no-ack`, `--inject-slow-leak`, and a **NEUTRAL** run with neither flag at the
  identical duration and crash interval. Proves the **wiring**. The neutral run is the attribution
  control: without it, a `--no-ack` FAIL cannot be distinguished from *"any run of this duration
  fails."*
- **And, for the two CLI knobs only,** §F13's two 25 s PROBE invocations — not a control leg, not a
  measurement, bounded by `C9`.

**Why `--crash-interval 120` and not 60, decided in the open and before the runs.** Each checkpoint
costs a quiesce + `kill -9` + restart + ready-wait, and every second of it is wall time **not** spent
driving OR churn — which is the growth the `--no-ack` cell must produce. At `D = 900 s`,
`--crash-interval 60` forces ~15 such cycles; the default 120 forces 7, halving the overhead and
returning it to churn, while still clearing **both** `L1` guards with margin: **8 samples** against
`min_samples = 4`, and **`span_secs ≈ 780 s`** against `min_span_secs = 600`. Since `C11` forbids
retuning `headroom_bytes` after the fact, the **cell parameters are the only honest lever**, and
they are chosen for maximum churn **subject to admissibility** — never to change a verdict.

**These control runs are NOT a measurement lineage (`C9`).** No pin, no matrix, no width sweep, no
fit, no CSV segments, no manifest §-append, no replicate. Their **only** publishable outputs are the
exit code, the rendered `tombstone_corpus_redb_scan:` line and the `finishedReason`. **Their numbers
may not be quoted as evidence about the plateau, the reclaim fraction, or any width** (`C10`).

### HOW `AT` IS GRADED — CONJUNCTIVE, over three transports

No one transport is sufficient. An exit code alone cannot distinguish an `L1` breach from an `L0`
breach or from a `mem` / `convergence` / `recovery` / `panic` failure, so a row reading *"FAILS on
`L1`"* is **not** decidable on the exit code by itself. Every `AT` row's *"FAILS on `L1`"* means the
**conjunction** of all three, and any row where the three disagree is `AT-0` (inadmissible), never a
judgement call:

1. **exit code** (`X21-c`) — `1` for FAIL, `0` for PASS (`i32::from(!passed)`, `main.rs:1010`);
2. **rendered `disposition`** (`X21-a`) — must read `LevelEvaluated` on the rendered
   `tombstone_corpus_redb_scan:` line (`InstrumentFailed` ⇒ the FAIL is `L0`'s, not `L1`'s;
   `LevelSuppressed` ⇒ `L1` decided nothing ⇒ `AT-0`);
3. **`finishedReason`** (`X21-b`, persisted in `SoakReport`) — must name the level clause.

### THE PERMITTED AGGREGATE TRANSPORT

Pre-registered here because `AT-2` is graded on it and `C9` forbids the per-sample series. **The only
series-derived quantities any `AT` row may rest on** are the four aggregates frozen on the assessment
and rendered on the line — **count** (`samples`), **min** (`min_bytes`), **max** (`peak_bytes`) and
**last** (`last_bytes`) — plus `first_bytes`, the two half-peaks, `rise_bytes` and `span_secs`. **No
`AT` row may be stated over anything else.** A conjunct that cannot be expressed over this list is
re-scoped to what the list carries, or struck — no unevaluable conjunct survives to grading.

### The table

| Row | Antecedent (frozen) | Verdict | Consequence |
|---|---|---|---|
| **`AT-0`** | Any control run is **INADMISSIBLE**: its corpus series has `samples < min_samples` or `span_secs < min_span_secs` (so `L1` was SUPPRESSED and decided nothing), or the harness itself failed for a reason unrelated to the gate | `NOT-DEMONSTRATED-LIVE` | **Evaluated FIRST, fail-closed.** The permitted single re-run depends on WHICH guard fell short: a **`samples` shortfall** re-runs **once** at `--crash-interval 60` (≈ 15 checkpoints, same span); a **`span_secs` shortfall** cannot be fixed by any crash interval — the lever is the cell's duration, and the cells already run at the `C9` ceiling `D = 900 s` (the TOP of §F2's range), so **there is no duration lever left**: a `span_secs` shortfall at `D = 900` means something other than the duration is wrong and the re-run is spent at the SAME `D = 900` after that cause is identified. **A re-run at any `D < 730 s` is FORBIDDEN — inadmissible by construction.** If still inadmissible, take the **non-promotion** disposition. A `SUPPRESSED` clause is not a pass, and this row exists so it can never be read as one. |
| **`AT-1`** | `--no-ack` **FAILS** on `L1` ∧ `--inject-slow-leak` **FAILS** on `L1` ∧ NEUTRAL **PASSES** | `BAR-MET` | **PROMOTE.** `L1` ships as the hard gate, **with §F5's fallback conjunct intact** — promotion never removes the slope clause from the class `L1` cannot cover. Publish the three rendered lines. |
| **`AT-2`** | `--no-ack` **FAILS** on `L1` ∧ NEUTRAL **PASSES** ∧ `--inject-slow-leak` **PASSES** | `BAR-MET-PARTIAL — SLOW-LEAK RECLASSIFIED` | **PROMOTE, conditionally**, with the reclassification published as the headline secondary finding. Rationale, pre-registered: `--inject-slow-leak` is a **bounded ramp-then-catch-up** (`main.rs:121-126` — cadence doc `:121-125`, `SLOW_LEAK_ACK_INTERVAL` const `:126`; flag doc `:167-173`) authored to calibrate a **RATE** detector's floor. A bounded sawtooth **attains a ceiling**, so a ceiling gate passing it is the semantically correct answer, not blindness. **Conditional on two conjuncts, both mechanical and both stated over the PERMITTED AGGREGATE TRANSPORT above:** (a) the slow-leak run's rendered line shows `rise_bytes ≤ headroom_bytes` **AND** (`peak_bytes > last_bytes` **OR** `min_bytes < first_bytes`) — the evaluable form of *"the level fell at least once"*; a monotone non-decreasing series has `peak == last` and `min == first`, so the pair is a genuine discriminator, not a tautology, and the pass must be *earned* by boundedness; (b) the slow-ramp discrimination test (`W5`) is green, proving an *unbounded* ramp of the same per-hour magnitude, run long enough, DOES breach `L1`. Route to `TODO-634` **by id** whether a bounded-but-elevated plateau should red the gate — that is `L2`'s job, and `L2` is disarmed here for want of a validated ceiling. **`headroom_bytes` is NOT lowered to force a FAIL.** |
| **`AT-3`** | `--no-ack` **PASSES**, **or** NEUTRAL **FAILS** | `ANTI-TAUTOLOGY-FAILED` | **NO PROMOTION.** The estimator ships **REPORT-ONLY** and the slope clause **stays hard UNCONDITIONALLY** at `main.rs:853` — the fallback conjunct becomes the *only* form, i.e. `tombstones.passed` is ANDed with **no `slope_clause_stays_hard(...)` guard in front of it** (the helper stays, unused by the gate but still asserted by its test, so the exhaustiveness proof is not lost with the guard) — so the branch is a strict addition and the gate is not weakened in any run class. Publish the failure with all three rendered lines, and route the cause to `TODO-634` by id. **Do NOT retune any parameter** — retuning to make a control fail is the mirror image of retuning to make one pass, and both are the anti-suppression defect (`C11`, `KL-4`). |

**A PASSING gate on the `--no-ack` control run is a RED for this spec**, and `AT-3` is what that RED
does. Written before the runs so it is not decided at the keyboard.

**`AT-3` and `Assumptions 6` are NOT in tension.** `Assumptions 6` says the gate is EXPECTED to RED
at the **PRODUCTION epoch width** until the prune fix lands. **The control leg is not run at the
production epoch width** — it runs at the harness default (`effective_epoch_width()`), at
`D = 900 s`, with the default keyspace and churn. So a NEUTRAL **FAIL** on the control leg is **not**
the expected production-width RED; it means the cell fails without any control applied, which is
precisely what destroys attribution and is what `AT-3` fires on. Conversely, a production-width RED
**cannot** be quoted as an `AT-3` event, because no production-width cell is run here (`C9`).

**The third transport is §F14's SINGLE ranked writer and nothing else.** Every `AT` row's *"FAILS on
`L1`"* reads `finishedReason` as produced by that one writer, whose rank-3b branch is the level
clause. There is **one** reason-producing arm for the corpus verdict, not two, so a `--no-ack` cell
that breaches `L1` **and** the slope clause names the **level** clause — the three-way agreement
`AT` grading requires is a property of the writer's pinned ranking, not a coincidence of source
order.

## §F9 — THE WITNESS REGISTER `W1`–`W9`

*A green test is an earned negative only if its mutation arm REDs the assertion it is sited on.*
The register below is the **whole** register across **both halves** — restated here in full,
including `W6` and `W7`, which `SPEC-361b` owns, specifically so neither half can silently inherit
the struck figure "fourteen".

> **LIVE FIGURE: `9 witnesses / 9 arms / 18 legs / 9 transcripts`, split `6 + 3 = 9` /
> `12 + 6 = 18` / `6 + 3 = 9`. Read the SPLIT, not the totals** — the totals coincide with
> `Response v1`'s while the composition does not.
>
> **STRUCK everywhere, in both halves: "fourteen"; the `5 + 4` / `10 + 8` splits; the `4 + 5` /
> `8 + 10` splits; and the interim `8 / 16 / 8`.**

| W | Owner | What it proves | Site | Mutation | Must RED |
|---|---|---|---|---|---|
| `W1` | **361a** | `L1` discriminates a linear durable-corpus leak | `monitor.rs` calibration | replace `last_half_peak − first_half_peak` with `last_half_peak − last_half_peak` (identically 0) | `calibration_fails_linear_corpus_level` |
| `W2` | **361a** | `L1` does not false-RED a genuine ramp-then-plateau | `monitor.rs` calibration, **FROZEN FIXTURE `F-W2`** | compare `last_half_peak` against the series' **first** sample instead of the first-half peak (a level-vs-origin test) | `calibration_passes_plateau_after_ramp` |
| `W3` | **361a** | `L0` fails closed on a failed scan | `monitor.rs` calibration, **FROZEN FIXTURE `F-W3`** | replace the `scans_failed > 0` disjunct with `false` | `calibration_fails_on_failed_scan` |
| `W4` | **361a** | `L0` distinguishes an honest `Some(0)` from a blind `None` | `monitor.rs` calibration | treat a `bytes == 0` sample as a failed scan | `calibration_passes_honest_empty_corpus` |
| `W5` | **361a** | `L1` is not blind to a **slow** unbounded leak — the conjunct `AT-2` rests on | `monitor.rs` calibration | raise the synthetic ramp's duration/rate coupling so the series' rise falls under the headroom | `calibration_fails_slow_unbounded_ramp` |
| `W6` | **361b** | pin 7: one epoch of implicit retention at margin 0, and the delivered clamp | `tombstone_frontier_impl.rs` `mod tests` | (a) `ceiling > e` → `ceiling >= e` at `:963`; (b) drop `.min(delivered)` at `:420` | (a) **assertion 2 ONLY** — the `D`-retained / `D − 1`-drained read of the DRAINED SET; (b) **assertion 3** — the same-ceiling-under-over-claim read |
| `W7` | **361b** | pin 6: an under-stating boot floor is the safe direction | `reclamation_registry.rs` proptest | **(b) ONLY** — `None => self.boot_floor` → `None => Epoch::MAX.saturating_sub(self.boot_floor)` at `:656`. **Two mutants RETIRED by `SPEC-361b`'s Response v2**: the fold's `min → max` at `:646`, and the former arm (a) | **`P2` and nothing else.** `P1` and `P3` are inherited coverage and carry NO registered arm |
| `W8` | **361a** | the live gate is attributable to the control, not to the duration | the control leg | none — `W8` is the **NEUTRAL run itself**. Its non-firing is the arm. | `AT-3` fires if NEUTRAL FAILS |
| `W9` | **361a** | **`L2` fires as an ENVELOPE test (peak, not last), and both CLI knobs are effective over the rendered transport** | `monitor.rs` calibration, **FROZEN FIXTURE `F-W9`**; transport limbs at `tests/soak_wal_census.rs`'s fixture (`ceilingBytes` non-null) and §F13's probe pair | replace `L2`'s breach predicate `peak_bytes > c` with **`last_bytes > c`** | `calibration_fails_armed_ceiling` |

### FROZEN FIXTURE MAGNITUDES

Pinned here because this layer's digest closes them under `C11`, and because an unfrozen fixture
makes an arm silently non-discriminating. Each is stated as the property the arm needs, then as the
magnitude that delivers it.

- **`F-W2` (ramp-then-plateau; must NOT false-RED, and its arm MUST RED).** The arm compares the
  last-half peak against the **first sample** instead of the first-half peak, so it only REDs if
  **the ramp's total height from the first sample exceeds `headroom_bytes` while the half-to-half
  rise does not.** Frozen: **8 samples, 90 s apart** (`span = 630 s ≥ 600`); first sample
  **10,000 B**; the series ramps to **300,000 B** by sample 4 and then plateaus flat at
  **300,000 B** for samples 5–8. Half-peaks: first-half **300,000**, last-half **300,000** ⇒
  `rise = 0 ≤ 65,536` ⇒ the test PASSES as required. Under the arm:
  `300,000 − 10,000 = 290,000 > 65,536` ⇒ **REDs**.
  **Frozen relationship: `ramp_height − first_sample > headroom_bytes` AND
  `last_half_peak − first_half_peak = 0`.**
- **`F-W3` (failed scan; `L0` fail-closed).** The arm replaces the `scans_failed > 0` disjunct with
  `false`, so it only REDs if **the surviving `samples == 0` disjunct does NOT also fire.** Frozen:
  **`scans_attempted = 5`, `scans_failed = 1`, `samples = 4`** — explicitly `samples >= 1`, and in
  fact `>= min_samples`, with `span = 630 s ≥ 600` — so with `L0`'s scan disjunct disarmed the
  assessment would evaluate `L1` on a flat series and **PASS**. An *"all scans failed, no samples"*
  fixture is **FORBIDDEN for `W3`**: it leaves `samples == 0` firing and the arm silently
  non-discriminating — exactly the defect the register exists to catch.
- **`F-W9` (armed ceiling; envelope semantics).** The arm replaces `peak_bytes > c` with
  `last_bytes > c`, so it only REDs if **the peak exceeds the ceiling while the last sample does
  not**, and the FAIL must be attributable to `L2` and not to `L1`. Frozen: **8 samples, 90 s
  apart** (`span = 630 s ≥ 600`); `ceiling_bytes = Some(100_000)`; the series peaks at
  **200,000 B** inside the **first** half and decreases monotonically to **50,000 B** at the last
  sample. Then `first_half_peak = 200,000` and `last_half_peak < 200,000` — on a non-increasing tail
  the last-half peak cannot exceed the first-half peak — so `rise = 0` (saturating) ⇒ **`L1`
  PASSES**, `disposition = LevelEvaluated`, and `peak_bytes = 200,000 > 100,000` ⇒ **`L2` FAILS the
  run**. Under the arm, `last_bytes = 50,000 ≤ 100,000` ⇒ no breach ⇒ **REDs**.
  **Frozen relationship: `peak_bytes > ceiling >= last_bytes` AND
  `last_half_peak − first_half_peak = 0`.**

### THE ARM ARITHMETIC — the only correct count, IDENTICAL in both halves

The register holds **9 witnesses** and **9 mutation arms**: `W1`–`W5` one arm each (5), `W6` two
(a/b), `W7` **one** (b, after `SPEC-361b`'s retirement of arm (a)), `W8` none, **`W9` one**.
`5 + 2 + 1 + 0 + 1 = 9`, derived from the rows above and from nothing else. Under **apply + revert**
counting that is **18 legs**.

**A "transcript" is defined here so the count is not re-derivable two ways:** one transcript = **one
ARM**, and it contains **both** legs — the applied mutation with its RED output, and the revert with
its green re-run. So the two halves together hold **9 transcripts covering 18 legs**, not 18
transcripts.

### ARM OWNERSHIP AND EXECUTION HOME

`G2` **authors** this half's witnesses (the tests) and writes no transcript and applies no mutation;
**`G6` is the sole execution home** for this half's arms and the sole writer of
`spec361a-witness-arm.txt`.

- **THIS HALF (`SPEC-361a`) OWNS AND EXECUTES 6 ARMS** — `W1`–`W5` one each, plus `W9`'s — recorded
  as **6 transcripts covering 12 legs** in `spec361a-witness-arm.txt`.
- **`SPEC-361b` OWNS AND EXECUTES 3 ARMS** — `W6` (a)/(b) and `W7` (b) — recorded as **3 transcripts
  covering 6 legs** in `spec361b-witness-arm.txt`. *(Its former `W7` arm (a) is RETIRED.)*
- `W8` has no arm in either half; it is the NEUTRAL control run, which is **this half's**.
- **`6 + 3 = 9` arms, `12 + 6 = 18` legs, `6 + 3 = 9` transcripts.** Neither half may report the
  joint figure as its own, and neither may report only its own figure as the register's total.

## §F10 — TYPE-MAPPING RULES, AND WHAT SERIALIZES

`CorpusSample.bytes`, every `*_bytes` field and `headroom_bytes` / `ceiling_bytes` are **`u64`** —
counted byte totals are non-negative integers, never `f64`. `elapsed_secs` and `span_secs` are
`f64` (genuinely fractional wall-clock). `samples` / `scans_*` are `usize` (in-process collection
counts, never serialized as a wire integer type). `CorpusLevelDisposition` is an **enum**, not a
`String`. No struct carries a `type` / `r#type` field.

`TombstoneCorpusReport` derives `Serialize` with **`#[serde(rename_all = "camelCase")]`**, mirroring
`TombstoneReport` (`report.rs:43`). It does **not** cross the MsgPack wire (JSON report only), so
`PROJECT.md`'s `to_vec_named()` clause has no subject. **Its frozen field list:**

```rust
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TombstoneCorpusReport {
    pub scans_attempted: usize,
    pub scans_failed: usize,
    pub samples: usize,
    pub first_bytes: u64,
    pub min_bytes: u64,
    pub peak_bytes: u64,
    pub last_bytes: u64,
    pub first_half_peak_bytes: u64,
    pub last_half_peak_bytes: u64,
    pub rise_bytes: u64,
    pub span_secs: f64,
    /// Serialized as an explicit string, never omitted. Typed as the ENUM, not
    /// a `String` — see the serializer note below.
    #[serde(serialize_with = "serialize_disposition")]
    pub disposition: CorpusLevelDisposition,
    /// `null` when L2 is DISARMED. EXPLICIT — no `skip_serializing_if`
    /// (PROJECT.md: no skip on a disposition-bearing field).
    pub ceiling_bytes: Option<u64>,
    pub passed: bool,
    /// `null` when the assessment named no reason. EXPLICIT.
    pub reason: Option<String>,
}
```

### The `std`-only hazard, and its resolution

`CorpusLevelDisposition` reaches the JSON as an **enum-valued field, not a `String` field — and
WITHOUT putting serde into `monitor.rs`.** The enum **derives no `serde`**, because `monitor.rs` is
`#[path]`-included by `tests/soak_monitor_calibration.rs:15` and that includer's header records the
module as **`std`-only** (`:10`); a `Serialize` derive there would break that constraint and `AC1`
with it. Instead the enum carries a hand-written `pub const fn as_str(self) -> &'static str`, and
`report.rs` — which already depends on serde — serializes the field through
`#[serde(serialize_with = "serialize_disposition")]`, a three-line fn that emits `value.as_str()`.

The JSON therefore reads `"LEVEL_EVALUATED"` / `"LEVEL_SUPPRESSED"` / `"INSTRUMENT_FAILED"`, the
**field stays enum-typed** (`PROJECT.md`'s *enums over strings* honoured), and `main.rs`'s console
line renders the **same** `as_str()` token, so the two transports cannot disagree and neither retypes
a literal. The **assessment** and the **enum** derive no `serde`; only the **report** does, and it
reaches the enum through a serializer fn on its own side of the `std`-only boundary.
`TombstoneCorpusAssessment` remains in-process only.

**The symmetry (`KL-6`), stated as a general rule rather than a one-off:**

> **ANY new cross-module reference between two `benches/soak_harness` modules obliges EVERY `tests/`
> target that `#[path]`-includes either module to declare the sibling `mod` — and that declaration
> is a COUNTED `.rs` edit, never a `PROJECT.md` shape.**

Moving `CorpusLevelDisposition` into `report.rs` would not remove the obligation; it would move the
break onto the calibration target and drag serde across the `std`-only boundary. The break is
**symmetric**; only the direction of the required `mod` moves. This half keeps the type in
`monitor.rs`, which is the direction that keeps `monitor.rs` `std`-only.

### `#[derive(Default)]` IS DELETED — the ONE recorded departure

`PROJECT.md`'s *"`Default` derived on payload structs with 2+ optional fields"* item would apply
(`ceiling_bytes`, `reason`). It is dropped for two reasons:

- **Mechanical:** a derived struct `Default` requires **every** field type to implement `Default`,
  and `CorpusLevelDisposition` deliberately does not. As frozen it would not compile (E0277). No
  sibling report struct derives `Default` either (`report.rs:42`, `:56`, `:80`, `:89`).
- **Semantic, and decisive:** a **defaulted report is itself the invisible-pass hazard `KL-5`
  forbids.** A `TombstoneCorpusReport` that exists without an explicitly-constructed disposition
  claims *something* about a gate that decided *nothing*. **Every `TombstoneCorpusReport` is
  constructed explicitly, with its disposition, from a `TombstoneCorpusAssessment`** — one
  construction site in `main.rs`, one fixture site in `tests/soak_wal_census.rs`, both naming every
  field.

`PROJECT.md`'s Auditor Checklist item says *"should"*, so this is a **named, reasoned departure**,
and it is the **ONE** departure this spec records.

**DECLARED FALLBACK, to be taken ONLY if a serde/builder path genuinely requires `Default`:**
`#[derive(Default)]` on `CorpusLevelDisposition` with **`#[default]` on `LevelSuppressed`** — the
explicit NOT-EVALUATED variant, which is the honest neutral. **`#[default]` on `LevelEvaluated` is
FORBIDDEN in every circumstance — that is precisely the invisible pass.** **WHICH ARM APPLIED: the
PRIMARY (derive deleted).** After this digest, taking the fallback is a spec revision recorded in
the post-section with its reason, surfaced to the user — never a keyboard substitution.

### NO `skip_serializing_if` ON `ceiling_bytes` OR `reason` — COMPLIANCE, NOT A DEPARTURE

`PROJECT.md`'s Auditor Checklist for Rust Specs carries the rule **"No `skip_serializing_if` on
disposition-bearing fields"**: a field whose *absence* would be read as a disposition must always
serialize. `ceiling_bytes`'s `None` **is** a disposition (*"L2 disarmed"*) and `reason`'s `None`
**is** a disposition (*"the assessment named no reason"*), so both fall squarely inside the
carve-out and **this spec cites the rule instead of recording a per-spec departure**.
`skip_serializing_if` **omits the key entirely**, making *"L2 disarmed"* indistinguishable from
*"this report predates the field"* — the invisible-suppression defect `KL-5` / `PD-F9` exist to
forbid. §F1's `L2` row states the same shape, and the two agree.

The rule was **promoted to `PROJECT.md` on its THIRD recurrence**: the archived parent recorded the
departure, `C4a` re-recorded it, and Audit v3 found it being paid for a third time. `C4a` now
records **ONE** departure — the `Default` derive — and this item as **compliance with the new
rule**.

## §F11 — THE CENSUS TARGET, AND ITS WAVE PLACEMENT

`packages/server-rust/tests/soak_wal_census.rs` `#[path]`-includes `report.rs` as a standalone
`mod report;` (`:43-45`) with **no sibling `mod monitor;`**. This half breaks that target in **two
independent ways**, and both must be repaired in the same wave that lands them:

1. **The type.** `TombstoneCorpusReport.disposition` is `CorpusLevelDisposition` (enum-typed — no
   `String` downgrade) and `serialize_disposition` calls `as_str()`; both name a type defined in
   `monitor.rs`.
2. **The fixture, which breaks even WITHOUT the type.** `fn sample_report` (**`:319-378`**; its
   exhaustive `SoakReport` literal is **`:320-377`**, `finished_reason` at `:375`) builds
   `SoakReport` by **exhaustive struct literal** with no `..Default::default()` — its own comment
   (`:347-349`) demands every value be *"DISTINCT and non-default on purpose"*. Adding **any** field
   to `SoakReport` breaks it. This is why a `String`-typed field would not have saved the target,
   and why `#[derive(Default)]` would not have either.

**The repair, and its exact extent.** The target gains the three-line include block immediately
above its existing `mod report;`, matching that file's own idiom (and
`soak_monitor_calibration.rs:15-17`'s):

```rust
#[path = "../benches/soak_harness/monitor.rs"]
#[allow(dead_code)]
mod monitor;
```

plus the `tombstone_corpus:` fixture value and the `tombstoneCorpus` assertions in
`soak_report_emits_every_hard_anded_verdict_tracking_its_input` (`:390`). **Its `ceiling_bytes` is
`Some(<distinct non-default n>)`, not `None`** — the one place in this half that drives the
**non-null** `ceilingBytes` branch, and a `W9` limb: the `null` branch is graded by the control
leg's rendered JSON, the non-null branch by this fixture, so **both** `L2` states reach a transport.

**Ledger:** this is **counted headroom, NOT an exemption.** The cap is **5** and this half was at 3,
so it goes to **4/5 with ZERO exemptions**. **No `PROJECT.md` shape is invoked, cited or needed — a
COUNTED file needs no shape.** In particular it is **not shape 4**, which explicitly excludes *"a
new `mod`"* (`PROJECT.md:205`) and is bounded to a single additive `#[test]` fn with no production
edit; this repair adds a `mod`, edits an existing fixture fn and an existing test fn, so **three**
of shape 4's bounds are exceeded.

**Known, accepted side effect:** with `mod monitor;` declared here, `monitor.rs`'s inline
`#[cfg(test)] mod tests` compiles and RUNS in this target too, so the calibration tests execute in
**two** targets. That is duplicated execution of identical assertions — no new assertion, no new
arm, no change to `AC4`/`AC5`, whose subject stays `tests/soak_monitor_calibration.rs`.

> **WAVE PLACEMENT IS NORMATIVE: this edit lands in `G4`, segment `S2`, in the SAME wave as
> `report.rs`'s `TombstoneCorpusReport` and the `SoakReport` field. It may NOT be deferred to a
> later wave** — between the wave that adds the `SoakReport` field and the wave that repairs the
> fixture, `cargo test --all-targets` does not build, and a spec whose own gate cannot run between
> waves is not orchestrable. `report.rs` and its `#[path]` includer travel together and may not be
> split across segments.

## §F12 — `X21`, DISCHARGED IN FOUR LIMBS

*A knob's effective value is proven over the transport its consumer actually reads, not over an
in-process accessor. `PD-F8` is the cautionary class.*

- **`X21-a` — THE RENDERED SUMMARY LINE, anchored to the line that ACTUALLY EXISTS.** The gate's
  verdict, its `disposition`, `first/min/peak/last`, both half-peaks, the `rise`, the `span`,
  `scans_ok/failed`, **`headroom=<n>`** and `ceiling=disarmed|<n>` are asserted over the **rendered
  line whose prefix at HEAD is `tombstone_corpus_redb_scan:`** (`main.rs:1082` for the `Some` arm,
  `:1090` for the `None` arm), captured from a control run's stdout — not over
  `TombstoneCorpusAssessment` in memory.
  - **The name is NOT changed.** An earlier draft graded on a `tombstone_corpus:` prefix that does
    not exist at HEAD and is not even a substring of the real one, so a grader grepping stdout would
    have found nothing. Keeping `tombstone_corpus_redb_scan:` also avoids breaking any existing
    operator grep. **What DOES change is the line's content and meaning** (report-only
    positive-control cross-check → the gate's own verdict line), recorded in the Delta as the
    operator-visible contract change it is.
  - **NEVER a retyped literal.** Every assertion — Rust-side and shell-side — obtains the prefix from
    the single source constant `TOMBSTONE_CORPUS_LINE_PREFIX`, **by name**, so a future rename REDs
    loudly instead of missing silently. A grep whose pattern is a hand-typed copy of the prefix is a
    **RED** at review, not a style note.
- **`X21-b` — THE JSON REPORT.** `SoakReport.tombstoneCorpus` is asserted over the written report
  file, because a failed run's console lives under `target/` and the committed artifact is the JSON.
  It also carries `finishedReason`, the third conjunct of `AT` grading.
- **`X21-c` — THE PROCESS EXIT CODE.** The one transport a CI gate actually consumes
  (`i32::from(!passed)`, `main.rs:1010`). **NECESSARY but NOT SUFFICIENT for `AT` grading:** an exit
  code cannot distinguish an `L1` breach from an `L0` breach or from a `mem` / `convergence` /
  `recovery` / `panic` failure. What is forbidden is grading on an internal boolean: **a control leg
  graded on an in-process `passed` field would be exactly `PD-F8`'s defect.**
- **`X21-d` — THE TWO NEW CLI KNOBS**, `--tombstone-corpus-headroom-bytes` and
  `--tombstone-corpus-ceiling-bytes`, discharged by §F13's probe pair over the transports their
  consumers read: the **rendered line** carries `headroom=<n>` **and** `ceiling=disarmed|<n>` and
  the pair renders **different** values for both tokens, so each knob is individually attributable
  on the line; and the **exit code** flips between the two invocations, attributable to the
  **ceiling** knob alone. `SPEC-361b`'s `X21` subject (the margin gauge) is untouched by this limb.

*(The reclamation margin knob's own `X21` discharge — `topgun_reclamation_margin_epochs` on
`/metrics` — was `SPEC-359`'s and belongs to `SPEC-361b`'s subject. This half neither re-discharges
nor restates it.)*

## §F13 — THE KNOB-EFFECTIVENESS PROBE PAIR (`X21-d`), AND IT IS NOT A MEASUREMENT

Two invocations of the existing 25 s smoke shape (`--duration 25 --crash-interval 0`, the shape
`rust.yml:470-478` already runs), differing **only** in the two knobs:

| Probe | Flags | Expected on the rendered `TOMBSTONE_CORPUS_LINE_PREFIX` line | Expected exit |
|---|---|---|---|
| `P-armed` | `--tombstone-corpus-headroom-bytes 1 --tombstone-corpus-ceiling-bytes 1` | `headroom=1`, `ceiling=1`, `disposition=LEVEL_SUPPRESSED(n=1, span=…)` | **non-zero** — `L2` breaches (`peak_bytes > 1`) |
| `P-slack` | `--tombstone-corpus-headroom-bytes 262144 --tombstone-corpus-ceiling-bytes 99999999999` | `headroom=262144`, `ceiling=99999999999`, same `disposition` | **0** — `L2` armed and not breached |

**What the pair proves, and what it deliberately does NOT.**

- **Both knobs are individually attributable ON THE LINE:** each renders its own token, and the two
  invocations render different values for both tokens.
- **The EXIT-CODE flip is attributable to the CEILING knob ALONE.** At `--crash-interval 0` there is
  exactly one (terminal) corpus sample ⇒ `L1` is `SUPPRESSED` ⇒ **the headroom knob has no subject
  in this probe**, stated rather than left to be inferred. The headroom knob's discharge is the
  rendered `headroom=<n>` token, not the verdict.
- **`C9` and `C10` bind these two invocations exactly as they bind the control leg.** Their **only**
  publishable outputs are the two rendered tokens and the exit code. **No corpus magnitude from
  either probe may be quoted as evidence about the plateau, the reclaim fraction, or any width**,
  and neither is a replicate, a cell, a pin or a lineage.
- **They are NOT `AT` inputs.** `AT` is graded on the three control cells and nothing else; a probe
  exit code may never be substituted for a control cell's.
- **`W9`'s mutation arm is NOT sited here** — it is sited on `calibration_fails_armed_ceiling` in
  `monitor.rs`. The probes are the transport limb; the arm is the estimator limb. *A witness that
  cannot fail is theater*, and `W9`'s ability to fail lives in the calibration test.

## §F14 — `finished_reason`: ONE RANKED WRITER

`AT`'s third transport (`X21-b`) is `finishedReason`, so its value must be a **decided** property of
the run, not an artifact of source order.

**What HEAD does, verified.** `finished_reason` is initialised to `"duration reached"` (`:618`) and
then assigned by **independent, un-ranked `if` blocks, the last of which wins**: four in the verdict
block — `:858` (memory, guard `:857`), `:864` (tombstone-byte monitoring blind, guard `:863`),
`:874` (tombstone slope, guard `:868`), `:882` (disk monitoring blind, guard `:881`) — **and four
EARLIER, in the run loop: `:629`, `:734`, `:738`, `:742`** (panic detected, convergence divergence,
crash-recovery mismatch, panic detected). At HEAD the verdict block **overwrites** the earlier four,
which is the same attribution defect one level up. The value reaches `SoakReport.finishedReason` at
`:956` and the console at `:1024`.

**What lands: the four verdict-block writes become ONE ranked, disposition-switched writer** — a
single expression evaluated once, taking the **first arm that fires**. The four earlier-phase writes
are **byte-unchanged**; the ranking governs whether the verdict writer may overwrite them.

### THE `:618` DEFAULT BECOMES A NAMED CONSTANT

Rank 0's guard switches on the sentinel *"has an earlier phase already written a reason?"*, and an
earlier draft expressed that as `finished_reason != "duration reached"` — **a retyped magic
string**, in the one spec whose rule is *never a retyped literal*. **VERIFIED AT HEAD:
`"duration reached"` is a bare string literal at `main.rs:618` and there is NO named constant.** So:

```rust
/// The `finished_reason` a run carries when nothing wrote a breach reason.
/// Single source of truth: the initialiser and the ranked writer's rank-0
/// guard both take it from here, so rewording the default cannot silently
/// disable rank 0.
const FINISHED_REASON_DURATION_REACHED: &str = "duration reached";
```

`main.rs:618` becomes `let mut finished_reason = FINISHED_REASON_DURATION_REACHED.to_string();` and
rank 0's guard becomes `finished_reason != FINISHED_REASON_DURATION_REACHED`. `main.rs` is already
counted 2/4, so this costs nothing, and it follows the pattern `TOMBSTONE_CORPUS_LINE_PREFIX`
establishes in the same file.

**RECONCILIATION, because this contradicts an earlier claim of this spec.** `AC26` and Validation
Checklist 8 previously asserted `:618` is byte-unchanged. It is **not**, and cannot be, if the
sentinel comes from the source constant. The corrected split:

- **`:618` — CHANGES**, in exactly one way: its literal is replaced by a reference to the constant.
  **The VALUE is byte-identical** (`"duration reached"`), so no run's `finishedReason` string
  changes and `C8` holds.
- **the four EARLIER-PHASE writes at `:629`, `:734`, `:738`, `:742` — BYTE-UNCHANGED IN SUBSTANCE.**
  They write breach reasons, not the default, so the constant has no subject there.
- **`grep -c 'finished_reason = '` still returns 6** — the initialiser, the four earlier-phase
  writes, and the single verdict writer. `6` is the post-edit count; HEAD's `9` is the baseline.
- **`grep -c '"duration reached"'` ⇒ 1** (the constant's definition, and nowhere else).
- **Line numbers in `AC26` and Checklist 8 are DESCRIPTIVE** — the constant lands above `:618` and
  shifts all five.

### THE PRECEDENCE, in full

**BREACH REASONS TAKE PRECEDENCE OVER NORMAL COMPLETION, and an unrelated harness failure takes
precedence over a gate verdict.**

| Rank | Arm | Guard | Produces a reason? |
|---|---|---|---|
| **0** | **an EARLIER-PHASE reason is already set** (convergence divergence, crash-recovery mismatch, server panic — `:629`/`:734`/`:738`/`:742`) | `finished_reason != FINISHED_REASON_DURATION_REACHED` on entry to the verdict block — **the sentinel comes from the source constant, never a retyped literal** | **the writer does not fire at all**; the earlier reason survives |
| **1** | **memory-growth failure** | `!mem.passed` | yes — `:858`'s text, unchanged |
| **2** | **tombstone-byte MONITORING BLIND** | `blind_monitor` | yes — `:864`'s text, unchanged |
| **3** | **the DURABLE-CORPUS arm**, itself disposition-switched: **3a** `InstrumentFailed` ⇒ `L0`'s reason; **3b** `LevelEvaluated` ⇒ the **level clause's** reason | `!corpus.passed` | yes — **NEW**; 3b is the text `AT` grades on |
| **4** | **tombstone SLOPE breach — the FALLBACK arm** | `corpus.disposition != LevelEvaluated && !tombstones.passed` | yes — `:874`'s text, kept **verbatim in substance**, as the ONE writer's rank-4 branch |
| **5** | **disk MONITORING BLIND** | `disk_blind_monitor` | yes — `:882`'s text, unchanged |
| **6** | **disk SLOPE** | `!disk.passed` | **NO** — report-only; it appends to `pending_gates` (`:886-896`), is not a `finished_reason` writer at HEAD and is not made one (`C7`; `SPEC-348` owns it) |
| **7** | **panic report** | `panic_report.is_some()` | **NO** — its transports are the `panicReport` field and the stderr dump (`:897-902`); unchanged. Ranked for exhaustiveness so no reader assumes a silent arm |
| **8** | **normal completion** | no arm above fired | **NO** — the `:618` default, now `FINISHED_REASON_DURATION_REACHED`, stands with its value byte-identical |

**Three properties, each load-bearing:**

- **Rank 4's guard is what makes the demotion coherent on the reason transport.** When
  `disposition == LevelEvaluated`, the slope is report-only and the run may **PASS** despite
  `!tombstones.passed`; naming a slope breach in `finishedReason` on a passing run would be a failure
  reason attached to a green run. The guard is exactly §F5's fallback conjunct, so the **gate** and
  the **reason** switch on the same predicate and cannot disagree.
- **Rank 3 sits ABOVE rank 4 and rank 5, which is what `AT` needs.** A `--no-ack` cell is expected to
  breach `L1` **and** the slope clause — both instruments measure the same growth — so without a
  ranking the three `AT` transports could disagree and send a **correct** demonstration to `AT-0`.
  With rank 3 pinned above rank 4, a `LevelEvaluated` breach names the level clause, and the three
  transports agree by construction.
- **Ranks 0, 1 and 2 sit ABOVE rank 3, which is what `AT-0` needs.** An unrelated harness failure —
  divergence, recovery mismatch, panic, a memory breach, a dead `/metrics` scrape — must remain
  **visible** in `finishedReason`, or a broken control run would read as a clean `L1` demonstration.

**DELIBERATE, RECORDED BEHAVIOUR CHANGE.** HEAD's last-writer-wins resolves multi-failure runs as
`disk-blind > tombstone > memory > earlier-phase`; this ranking resolves them
`earlier-phase > memory > tombstone-blind > corpus > slope > disk-blind`. On any run with a
**single** failure the string is **byte-identical to HEAD**; only multi-failure runs re-order, and
they re-order **toward** the unrelated-failure cause. No clause's contribution to `passed` changes,
no exit code changes, and no arm's TEXT changes — this is a precedence pin, not a re-wording.

## §F15 — THE SAMPLER'S BYTE-COPY NEUTRALITY MECHANISM AND SCRATCH-PATH LIFECYCLE

> **KL-7 — AN INSTRUMENT MAY NOT MUTATE WHAT IT MEASURES, AND MAY NOT MUTATE WHAT ANOTHER GATE
> MEASURES.** This is the governing rule of the sampler's siting and it outranks every convenience
> in it.

A sampler placed between `kill9()` and `start()` sits **in front of the server's own crash
recovery**, which is the input of an existing hard gate (`recovery_failures`) and of two CI
assertions (`rust.yml:494-503`, `:641-647`). **`redb::Database::open` on an uncleanly closed file
runs `do_repair` and COMMITS — a write** (`redb` 2, `Cargo.toml:64`: `Database::open` →
`builder().open` → `Database::new`, whose `needs_repair()` gate runs `do_repair` + `commit`). Opening
the server's file there would make the harness perform the server's recovery — the
instrument-neutrality class `TG-OR-006` names (`INVARIANTS.md:529`, the three-way split at
`:563-567`).

> **The corpus scan NEVER opens the server's own file at a live checkpoint. It opens a BYTE COPY on
> a scratch path outside `data_dir`, and the copy is discarded. Neutrality is not traded for sample
> count: if the copy's cost does not fit the cadence, the SAMPLE COUNT is reduced, never the
> neutrality.**

**The four steps at each checkpoint**, replacing the single `supervisor.restart(config.ready_timeout)`
at `main.rs:1745` with an explicit pair around them, preserving `restart`'s existing 250 ms
socket-release sleep (`process.rs:287`) and its error handling verbatim, including the
deliberately-left-open boot gap on a failed restart (`:1749-1755`):

1. `std::fs::copy({data_dir}/topgun.redb, {scratch}/corpus-<k>.redb)` where `<k>` is the checkpoint
   ordinal. **The original is read-only in this step and is not opened by redb at all.**
2. `scan_redb_tombstone_corpus({scratch}, OR_MAP)` — reading the **copy**. Any `needs_repair` work
   redb performs happens **on the copy**.
3. The copy is **deleted** immediately after the scan returns, before `supervisor.start(...)`.
4. `supervisor.start(...)` — the server is the **FIRST opener of the ORIGINAL**, so redb's
   repair-and-commit path runs exactly where it ran at HEAD.

**`C8`** (*"no existing metric series', constant's or assessment field's VALUE or MEANING
changes"*) **and §F5's *"this spec weakens no existing hard clause"* are UNCHANGED IN TEXT and are
now TRUE**: with the copy in place, no existing gate's input is perturbed by the instrument.

**SCRATCH-PATH LIFECYCLE, specified rather than left to the keyboard.**

- **Created ONCE**, at harness start, beside the run's other scratch state and **OUTSIDE
  `data_dir`** — because `sample_disk_mb` is `du -sk` over `data_dir` (`monitor.rs:583-596`, called
  at `main.rs:584`) and a copy written inside it would step the DISK gate's own input, a second
  instrument-neutrality breach against a gate `C7` forbids this spec from touching.
- **UNIQUE PER SAMPLE:** the file name carries the checkpoint ordinal, so two samples can never
  alias, and a delete that failed cannot leave a previous sample's bytes to be re-scanned as this
  one's. `std::fs::copy` truncates an existing destination, so an aliasing bug cannot produce a
  partial-overwrite hybrid either.
- **DELETED per sample**, immediately after the scan; **and the whole scratch directory is removed at
  teardown**, on both the pass and the fail path, so a failing run leaves nothing behind.
- **A failed delete is NOT a scan failure** — the sample was obtained. It is logged and the run
  continues; the teardown sweep removes the residue.

**THE COPY'S FAILURE MODE FOLDS INTO `scans_failed`, EXACTLY AS A SCAN FAILURE DOES (`KL-3`, `L0`).**
Each checkpoint increments `scans_attempted` **once**. Then: a failed **copy** (source missing, I/O
error, no space) increments `scans_failed` and pushes no sample; a successful copy followed by a
`None` **scan** likewise increments `scans_failed` and pushes no sample; a successful copy followed
by `Some(b)` pushes a `CorpusSample`. **An instrument that could not obtain the state is blind
whichever step failed, and `L0` fails the run either way.** There is no third outcome and no silent
skip.

**The redb `Database` handle the scan opens is DROPPED before `supervisor.start(...)`.**
`scan_redb_tombstone_corpus` already owns and drops its handle within its own body (`main.rs:1230`+,
`db` is a local), so the property holds by construction. Under the copy the handle is on the
**scratch** file, so a leak could no longer block the server's restart — but the requirement is
kept, because a leaked handle would also block the per-sample **delete** on platforms that lock open
files. No `let _db = …` binding may be hoisted out of the scan.

**The existing terminal scan (`main.rs:778`) keeps reading `data_dir` DIRECTLY and takes no copy** —
it is post-teardown and **nothing boots after it**, so there is nothing left to be neutral toward.
This is the one siting whose semantics are unchanged from HEAD. It contributes the final sample on
the same counters.

**`ServerSupervisor::restart` is NOT deleted** — other call sites and its own contract stay intact;
only this one site is expanded. `process.rs` is **not** edited and is **not** counted.

**THE DOC-CONTRACT THIS FALSIFIES, corrected in the same commit.**
`scan_redb_tombstone_corpus`'s doc (`main.rs:1217-1224`) reads *"MUST run only after
`ServerSupervisor::shutdown` has reaped the child process … This is therefore a post-teardown
assertion, **not a live sampler** alongside RSS/tombstone-bytes/disk above."* The sampler makes that
sentence (`:1219-1221`) FALSE the moment it lands, and **a false doc-contract may not survive this
spec**. The contract is rewritten to state the properties that are actually load-bearing: the
precondition is **the CHILD BEING REAPED**, not shutdown specifically (`kill9()`,
`process.rs:270-280`, awaits `child.wait()` and satisfies it exactly as `shutdown` does); it **IS** a
live sampler, called once per recovery checkpoint plus the terminal post-teardown scan; and **at the
checkpoint site its argument is a scratch directory holding a byte copy, never the server's own
`data_dir`** — with the *reason* stated, because it is the reason the function may be called live at
all.

## §F16 — THE FROZEN TYPE SURFACE

Signatures and doc-contracts. Total over its inputs; no panic path; no interior mutability; no I/O;
**no dependency beyond `std`** (the file is `#[path]`-included by
`tests/soak_monitor_calibration.rs:15`).

```rust
/// One durable-layer OR tombstone corpus scan, taken while the server process is
/// DEAD (redb is single-writer, so its file lock must be free) and therefore
/// reading the PRE-RECOVERY on-disk state.
#[derive(Debug, Clone, Copy)]
pub struct CorpusSample {
    pub elapsed_secs: f64,
    pub bytes: u64,
}

/// Which clause actually decided a corpus assessment. Rendered verbatim, so a
/// clause that did not fire is visible rather than indistinguishable from a pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CorpusLevelDisposition {
    /// L1 was evaluated and decided.
    LevelEvaluated,
    /// L1 was NOT evaluated, so the slope clause hard-gates instead. Never "ok".
    LevelSuppressed,
    /// L0 failed. L1 and L2 are NOT EVALUATED (fail-closed order).
    InstrumentFailed,
}

impl CorpusLevelDisposition {
    /// The single rendered token for this disposition. Consumed BOTH by the
    /// console line in `main.rs` and by the JSON serializer in `report.rs`, so
    /// the two transports can never disagree and no site retypes a literal.
    /// Deliberately hand-written rather than serde-derived: this file is
    /// `#[path]`-included by an integration target and must stay `std`-only.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LevelEvaluated => "LEVEL_EVALUATED",
            Self::LevelSuppressed => "LEVEL_SUPPRESSED",
            Self::InstrumentFailed => "INSTRUMENT_FAILED",
        }
    }
}

#[derive(Debug, Clone)]
pub struct TombstoneCorpusAssessment {
    pub scans_attempted: usize,
    pub scans_failed: usize,
    pub samples: usize,
    /// PRE-REGISTERED AGGREGATES of the corpus series. These four — count
    /// (`samples`), min, max (`peak_bytes`) and last — are the ONLY shape in
    /// which the series reaches a consumer: `C9` forbids publishing the
    /// per-sample series, so every predicate any downstream row is graded on
    /// (notably `AT-2`'s conjunct (a)) MUST be statable over exactly these.
    pub first_bytes: u64,
    pub min_bytes: u64,
    pub peak_bytes: u64,
    pub last_bytes: u64,
    pub first_half_peak_bytes: u64,
    pub last_half_peak_bytes: u64,
    pub rise_bytes: u64,
    pub span_secs: f64,
    pub disposition: CorpusLevelDisposition,
    /// `None` = L2 DISARMED. Serialized EXPLICITLY as `null` (§F10) — absence
    /// must never be indistinguishable from suppression (KL-5).
    pub ceiling_bytes: Option<u64>,
    pub passed: bool,
    pub reason: Option<String>,
}

/// L0 and L1 are HARD gate clauses; L1 only when its two guards are met.
#[must_use]
pub fn assess_tombstone_corpus_level(
    samples: &[CorpusSample],
    scans_attempted: usize,
    scans_failed: usize,
    headroom_bytes: u64,
    min_span_secs: f64,
    min_samples: usize,
    ceiling_bytes: Option<u64>,
) -> TombstoneCorpusAssessment;

/// Whether the tombstone-byte SLOPE clause still hard-gates a run that
/// produced this disposition. EXHAUSTIVE BY CONSTRUCTION: a fourth variant
/// does not compile here, which is what makes the NO-UNGATED-WINDOW coverage
/// argument structural rather than a source-read. `main.rs`'s verdict
/// expression consumes this and re-types no comparison of its own.
#[must_use]
pub const fn slope_clause_stays_hard(disposition: CorpusLevelDisposition) -> bool {
    match disposition {
        CorpusLevelDisposition::LevelEvaluated => false,
        CorpusLevelDisposition::LevelSuppressed => true,
        CorpusLevelDisposition::InstrumentFailed => true,
    }
}
```

**DELIVERY NOTE, recorded here because a frozen text that the code contradicts is the very hazard
this artifact exists to refuse.** The workspace lints `clippy::unimplemented` and
`clippy::match_same_arms` at `warn`, and CI runs `clippy -D warnings`, so the surface as delivered in
`monitor.rs` carries two mechanical additions the block above omits: `assess_tombstone_corpus_level`
takes an `unimplemented!()` body under a scoped `allow` until `G2` supplies the clause arithmetic
(the signature and its doc-contract are the frozen object, not the body), and
`slope_clause_stays_hard` carries a scoped `allow(clippy::match_same_arms)` so its three arms stay
**one per variant** rather than being merged into a `|` pattern — merging would still be exhaustive,
but it would stop each variant from being classified in its own right. Both are scoped `allow`s with
stated reasons, neither changes a predicate, and `#[allow(dead_code)]` on the unwired items is
removed as each is wired.

<!-- FROZEN-LAYER-END -->

<!-- POST-SECTION-BEGIN -->

## POST-SECTION — APPEND-ONLY, written after `G5`, under its own separate digest

**EMPTY at the time of the frozen layer's digest.** Nothing above this marker may be edited once its
digest is recorded; everything below it is appended, never rewritten.

When `G5` completes, this section carries: the executed control-run transcripts' verdict rows, the
`AT` row that fired, the promotion decision, and the routings handed off **by id** — plus any
recorded divergence between a pre-registered text (§F7 Table 2) or fixture magnitude (§F9) and what
was delivered, with its reason.

### G5 EXECUTION RECORD — the live control leg, the probe pair, and the promotion decision

Executed on this branch at HEAD, release build, all cells run **strictly sequentially**. Every
rendered line and exit code below was copied from a captured log **by script**, never retyped; the
full transcripts are in `spec361-control-runs.txt`. `C9`/`C10` bind all of it: no magnitude here may
be quoted as evidence about the plateau, the reclaim fraction, or any width.

#### 1. `AT` GRADED IN ITS FROZEN ORDER — the verdict is `AT-3`, `ANTI-TAUTOLOGY-FAILED`

Graded **conjunctively over `X21-a` (rendered `disposition`) + `X21-b` (`finishedReason`) +
`X21-c` (exit code)**, `AT-0` first and fail-closed.

| Cell (`D = 900 s`, `--crash-interval 120`) | exit (`X21-c`) | rendered `disposition` (`X21-a`) | `finishedReason` names (`X21-b`) | three transports agree? | `L1` outcome |
|---|---|---|---|---|---|
| **NEUTRAL** (attribution control) | `1` | `LEVEL_EVALUATED` | the **level** clause | yes | **FAILS on `L1`** |
| `--no-ack` | `1` | `LEVEL_EVALUATED` | the **level** clause | yes | **FAILS on `L1`** |
| `--inject-slow-leak` | `1` | `LEVEL_EVALUATED` | the **level** clause | yes | **FAILS on `L1`** |

**`AT-0` — DOES NOT FIRE.** Every cell is admissible: `samples = 8` against `min_samples = 4`, and
`span_secs` of **778.4 / 779.8 / 782.9 s** against `min_span_secs = 600` — all three within a second
or two of §F2's predicted `≈ 780 s`, with `scans_ok = 8` and `scans_failed = 0` everywhere, so `L0`
passed and the instrument was never blind. No cell was suppressed and no harness failure unrelated
to the gate occurred. **The single permitted re-run is therefore NOT spent, and must not be** — there
is no guard shortfall for it to repair.

**`AT-1` — NOT MET.** It requires `NEUTRAL` **PASSES**. `NEUTRAL` failed.

**`AT-2` — NOT MET.** It also requires `NEUTRAL` **PASSES**, and additionally `--inject-slow-leak`
**PASSES**; neither holds. Conjunct (a) is therefore never reached and is not evaluated.

**`AT-3` — FIRES.** Its antecedent is *"`--no-ack` **PASSES**, **or** NEUTRAL **FAILS**"*. The second
disjunct is satisfied: **the neutral cell, with no injection flag applied at all, breached `L1`**
(rise `142418 B` between half-peaks against the `65536 B` headroom). This is exactly the condition
§F8 pre-registered as destroying attribution: *"it means the cell fails without any control applied."*
A `--no-ack` FAIL cannot be attributed to `--no-ack` when the neutral cell of identical duration and
crash interval fails the same way.

> **Verdict: `ANTI-TAUTOLOGY-FAILED`. The bar was NOT met live.**

**This is the pre-registered negative outcome, not a surprise and not a judgement call.** §F8 was
frozen — and its digest recorded — before any calibration test was written and before any control
run was executed, precisely so this could not be decided at the keyboard. It is recorded as it fell.

#### 2. THE THREE RENDERED LINES (`X21-a`), copied verbatim

```
NEUTRAL:
tombstone_corpus_redb_scan: first=45009 min=45009 peak=316159 last=316159 first_half_peak=173741 last_half_peak=316159 rise=142418 span=778s scans_ok=8 scans_failed=0 headroom=65536 ceiling=disarmed disposition=LEVEL_EVALUATED -> FAIL (terminal scan 316159 bytes; last gauge value 328334 bytes -> DIVERGED) reason=durable-corpus level rose 142418 B between half-peaks (173741 B -> 316159 B) over 778s, above the 65536 B headroom

--no-ack:
tombstone_corpus_redb_scan: first=48995 min=48995 peak=386362 last=386362 first_half_peak=195198 last_half_peak=386362 rise=191164 span=780s scans_ok=8 scans_failed=0 headroom=65536 ceiling=disarmed disposition=LEVEL_EVALUATED -> FAIL (terminal scan 386362 bytes; last gauge value 395089 bytes -> DIVERGED) reason=durable-corpus level rose 191164 B between half-peaks (195198 B -> 386362 B) over 780s, above the 65536 B headroom

--inject-slow-leak:
tombstone_corpus_redb_scan: first=58674 min=58674 peak=307004 last=307004 first_half_peak=172137 last_half_peak=307004 rise=134867 span=783s scans_ok=8 scans_failed=0 headroom=65536 ceiling=disarmed disposition=LEVEL_EVALUATED -> FAIL (terminal scan 307004 bytes; last gauge value 307488 bytes -> DIVERGED) reason=durable-corpus level rose 134867 B between half-peaks (172137 B -> 307004 B) over 783s, above the 65536 B headroom
```

All three persisted `"ceilingBytes": null` — the **`null` branch** of `AC27`'s JSON limb, reached
over the real transport, as `X21-b` requires. (The non-`null` branch is the census-target fixture's.)

#### 3. THE TWO PROBE LINES AND THEIR EXIT CODES (`N22`, `X21-d`, `AC27` KNOB limb)

```
P-armed  (--tombstone-corpus-headroom-bytes 1 --tombstone-corpus-ceiling-bytes 1)       exit = 1
tombstone_corpus_redb_scan: first=5161 min=5161 peak=5161 last=5161 first_half_peak=0 last_half_peak=5161 rise=5161 span=0s scans_ok=1 scans_failed=0 headroom=1 ceiling=1 disposition=LEVEL_SUPPRESSED(n=1, span=0s) -> FAIL (terminal scan 5161 bytes; last gauge value 5781 bytes -> DIVERGED) reason=durable-corpus peak 5161 B exceeds the armed 1 B ceiling

P-slack  (--tombstone-corpus-headroom-bytes 262144 --tombstone-corpus-ceiling-bytes 99999999999)  exit = 0
tombstone_corpus_redb_scan: first=4660 min=4660 peak=4660 last=4660 first_half_peak=0 last_half_peak=4660 rise=4660 span=0s scans_ok=1 scans_failed=0 headroom=262144 ceiling=99999999999 disposition=LEVEL_SUPPRESSED(n=1, span=0s) -> ok (terminal scan 4660 bytes; last gauge value 5580 bytes -> DIVERGED)
```

`X21-d` is **DISCHARGED, exactly as pre-registered**: both knobs are individually attributable on the
rendered line (`headroom=1` vs `headroom=262144`; `ceiling=1` vs `ceiling=99999999999` — different
values for **both** tokens), and the **exit code flips `1 → 0`**. Both probes render
`LEVEL_SUPPRESSED(n=1, span=0s)`, so `L1` decided nothing in either and **the flip is attributable to
the ceiling knob alone** — the headroom knob has no subject here, and its discharge is the rendered
token, not the verdict, precisely as §F13 states. **Neither probe is an `AT` input**, and neither
probe's magnitude is quoted as evidence about anything (`C9`, `C10`).

#### 4. THE PROMOTION DECISION — **NO PROMOTION**

Per `AT-3`, and taken as the frozen table dictates rather than at the keyboard:

- **`L1` does NOT ship as the hard gate.** The estimator ships **REPORT-ONLY**.
- **The slope clause stays hard UNCONDITIONALLY** at `main.rs`'s fallback conjunct — the conditional
  demotion is withdrawn, so `tombstones.passed` is ANDed with **no `slope_clause_stays_hard(...)`
  guard in front of it**. The helper itself **stays**, unused by the gate but still asserted by its
  test, so `N4b`'s exhaustiveness proof (and `AC24` limb 2's compile-time falsifier) is not lost.
- **NO parameter is retuned.** `headroom_bytes` stays at `65536` and no cell duration or crash
  interval is changed. Retuning to make the neutral control pass is the mirror image of retuning to
  make a cell fail, and `C11`/`KL-4` forbid both. **This is the single most important line in this
  record.**
- The failure is published here with all three rendered lines, as `AT-3` requires.

> **CARRIED OBLIGATION — `G5` COULD NOT AND DID NOT IMPLEMENT THIS.** `AT-3`'s consequence is a
> **source change** to `main.rs`'s fallback conjunct, and `G5` is a run-and-record group: this half's
> counted `.rs` ledger is **FULL at 4 counted / 0 exemptions**, and `G5` is forbidden from editing
> any `.rs` file. The decision above is **recorded and routed**, not applied. **The demotion to
> REPORT-ONLY is an open obligation for the orchestrator to site**, and until it is applied the tree
> still carries `L1` as a deciding clause. It is called out here rather than left to be discovered.

**What the leg actually established, stated without inflation.** The wiring works end-to-end: the
sampler took 8 clean samples per cell across `kill -9` recovery checkpoints with zero scan failures,
the estimator evaluated, and all three transports agreed in all three cells. What was NOT established
is that `L1` **discriminates** — at this headroom, duration and workload it fires on everything,
including the untreated control. A gate that fires on the control is not a gate.

**A pre-registration observation, recorded rather than glossed.** §F3 derived `headroom_bytes =
65536` expecting the neutral cell to sit below it. The neutral cell rose `142418 B` — roughly `2.2×`
the headroom — over a `≈ 390 s` rise window. The three cells' rises (`142418` / `191164` / `134867 B`)
are of the **same order**, with the neutral cell neither lowest nor separated from the treated ones.
That is the substance of the `AT-3` finding and the material fact the routing below carries. **No
conclusion about the plateau or any width is drawn from these numbers** (`C10`).

#### 5. `AC23` ROUTINGS — BY ID, AND NO TRACKER FILE IS EDITED

| Item | Routed to |
|---|---|
| `L2`'s production **arming** (it stays `None`/disarmed here; armed only in the probe and in `W9`) | **`TODO-654`** |
| The **durable-corpus headroom revision** — `65536 B` is refuted as a discriminating threshold at this duration and workload by the neutral cell above | **`TODO-654`** |
| **The `AT-3` cause itself**: why the untreated neutral cell accrues a corpus rise of the same order as the treated cells, and what a discriminating level clause would have to key on instead | **`TODO-634`** |
| The width-1000 **plateau demonstration** | **`TODO-634`** |
| **`TG-OR-005`**'s closure | **`TODO-634`** |
| **`NAKED_BASELINE`** 4 → 3 | **`TODO-634`** |
| The **bounded-but-elevated-plateau** question (`AT-2`'s conditional routing) | **not routed — `AT-2` did not fire**, so its conditional routing has no antecedent. Recorded so its absence is deliberate rather than an omission. |
| The intermittent `soak-loaded-crash` **LWW merkle-root divergence** across recovery observed in `Part 3(iv)` — not this half's subject and not caused by it (`src/` is byte-unchanged, so the server binary is identical to base) | **`TODO-634`** |

**No tracker file is edited by this routing** (`C1`, `AC20`); the routing lives here, by id.

#### 6. `AC21` / `AC24` CONSUMER RE-RUN — SUMMARY (full detail in `spec361-control-runs.txt` Part 3)

| Surface | Blocking? | `disposition` | Verdict |
|---|---|---|---|
| 25 s Soak Smoke G4b (`rust.yml:470-478`) | **BLOCKING** | `LEVEL_SUPPRESSED` | exit `0`, `.passed == true` — **unchanged from HEAD**. `AC24` limb 1's executed constructibility witness. |
| boundary cell (`rust.yml:480-491`) | **BLOCKING** | `LEVEL_SUPPRESSED` | all three OR-no-loss greps hold; wall-clock budget holds. `AC21`(ii) satisfied. |
| `soak-loaded` (`rust.yml:592`) | non-blocking (`:548`) | `LEVEL_SUPPRESSED` | `.passed == true` in both runs — verdict unchanged. The CI conjunct `totalWrites > 20000` was not reached **on this host** (`16725`, `18874`); a local throughput shortfall, not a verdict change, and not attributable to the corpus gate. |
| `soak-loaded-crash` (`rust.yml:641`) | non-blocking (`:548`) | `LEVEL_SUPPRESSED` | **intermittent**: FAILED once (LWW merkle root changed across recovery), PASSED on two repeats. `tombstoneCorpus.passed == true` in all three, so the corpus clause contributed nothing. Cannot be a regression from this half — `src/` is byte-unchanged against base `20edabde`. |

**No cell's corpus clause decided any consumer verdict**: every one of the four rendered
`LEVEL_SUPPRESSED`, so `N4`'s fallback conjunct was live throughout and the slope clause hard-gated
them exactly as at HEAD.

#### 7. DIVERGENCES FROM PRE-REGISTRATION

- **§F7 Table 2 texts and §F9 fixture magnitudes:** authored in earlier waves, **not** re-authored
  here; `G5` observed no divergence in its own scope and changed neither.
- **The `AT` outcome is not a divergence** — `AT-3` is one of the four pre-registered rows, and it
  fired on its stated antecedent.
- **The one substantive divergence** is the §F3 parameter expectation recorded in §4 above: the
  neutral control was expected to sit below `65536 B` and did not. Its reason is unknown and is
  **routed to `TODO-634`, not patched over by retuning** (`C11`).

<!-- POST-SECTION-END -->

<!-- SITING-SECTION-BEGIN -->

## SITING RECORD — appended by G6, AFTER the post-section, under its OWN digest

**This section is NOT part of the frozen layer and NOT part of G5's post-section.**
It sits outside both sentinel pairs, so both earlier digests reproduce unchanged
and can be re-verified against `.frozen.sha256` and `.post.sha256` after reading
this. Its own digest is in `spec361-level-gate-ruling.siting.sha256`.

It records one thing: that the consequence `AT-3` and `AC12` attach to the
recorded verdict was actually **sited in the code**, and how.

### What fired, restated from the post-section rather than re-decided here

`AT-3` fired. Verdict `ANTI-TAUTOLOGY-FAILED`. Decision: **NO PROMOTION**. The
disjunct that fired is the second one: the **NEUTRAL** attribution control — no
injection flag at all — breached `L1` on its own (rise `142418 B` against the
`65536 B` headroom, `disposition=LEVEL_EVALUATED`, exit 1). `AT-0` did not fire:
all three 900 s cells at `--crash-interval 120` were admissible (`samples = 8`,
span 778–783 s, `scans_failed = 0`) and the single permitted re-run is unspent.

### What `AC12` and the `AT-3` row require, and what was done

Both texts agree on the consequence and were read together before editing:

- `AC12`: *"`L1` ships **REPORT-ONLY** and the slope clause stays hard
  **UNCONDITIONALLY** at `main.rs:853` (the `slope_clause_stays_hard(...)` guard
  is dropped, not the clause; the helper and its test survive)"*.
- The `AT-3` row: *"The estimator ships **REPORT-ONLY** … the `N4` fallback
  conjunct becomes the only form, i.e. `tombstones.passed` is ANDed with **no
  `slope_clause_stays_hard(...)` guard in front of it**"*.

Taken together these are two changes to one expression, and both were made:

1. **`corpus.passed` is no longer ANDed into the run verdict.** The `AT-3` row
   says *the estimator* ships report-only, not merely `L1`, and the row's own
   gloss — *"the branch is a strict addition and the gate is not weakened in any
   run class"* — is only true if the estimator adds no gating at all. Retaining
   `L0` alone as a hard clause was considered and rejected: the assessment
   exposes one `passed` folding `L0`/`L1`/`L2`, so splitting it would mean
   editing the `N1` types the frozen layer above closes under `C11`.
2. **The `slope_clause_stays_hard(...)` guard is dropped from in front of
   `tombstones.passed`,** which is therefore ANDed in every run class.

The resulting verdict expression is exactly `HEAD`'s tombstone gating:

```
convergence ∧ recovery ∧ mem ∧ ¬blind_monitor
  ∧ tombstones.passed          ← the OLD hard clause, now with no guard
  ∧ ¬disk_blind_monitor ∧ ¬panic
```

**NO-UNGATED-WINDOW (`N4`) holds, and by the simplest possible route:** the
slope clause gates *every* run configuration, so there is no run class in which
neither it nor a live `L1` decides. The rule was never at risk under `AT-3`; it
is at risk only under a promotion that removes the fallback.

**`C11` was not touched.** No headroom, no ceiling, no `min_samples`, no
`min_span_secs`, no duration and no crash interval was changed in either
direction. The demotion is a change to which verdicts `passed` reads, and to the
doc-contracts that describe it — nothing else.

### Three consequential details, recorded because they are judgement calls

- **The estimator still runs on every run.** It is computed, rendered on the
  `tombstone_corpus_redb_scan:` line with its full aggregate transport, and
  persisted as `tombstoneCorpus` in the JSON report. REPORT-ONLY means it
  decides nothing, not that it is switched off.
- **A corpus breach is recorded on `pending_gates`,** the harness's existing
  report-only channel (the disk slope already uses it, with the same *"did NOT
  fail the run"* phrasing). Without this the demotion would have made a breach
  invisible to anyone reading only the run's verdict.
- **The ranked `finished_reason` writer's rank-3 corpus arm no longer writes a
  reason.** A clause that cannot fail a run must not hand that run a reason
  saying it failed — which is the rule `N18b`'s own rank-4 doc already states.
  Rank 3 therefore joins ranks 6–8 as non-reason-producing. The rank numbers
  themselves are left as `N18b` froze them; nothing was renumbered.

`slope_clause_stays_hard` survives in `monitor.rs` with its exhaustive `match`
and its calibration test, so `N4b`'s compile-time falsifier (`E0004` on a fourth
variant) is intact, exactly as `AC12` requires. `main.rs` still calls it — to
name *which* corpus clause a report-only breach came from — and still re-types
no disposition comparison of its own.

### Pinned counts, re-verified AFTER the siting edit

| Pin | Required | Observed |
|---|---|---|
| `N4a` census, `monitor.rs` + `main.rs` | 36 (15 + 21) | **36** (15 + 21) |
| `grep -c slope_clause_stays_hard main.rs` | ≥ 1 | **2** |
| `grep -c 'CorpusLevelDisposition::LevelEvaluated' main.rs` | 0 | **0** |
| `grep -c 'finished_reason = ' main.rs` | 6 | **6** |
| `grep -c '"duration reached"' main.rs` | 1 | **1** |

### One `AC` whose PROSE the demotion outruns, disclosed rather than absorbed

`AC9` requires the census's in-scope and authored lines to *"read as the
CONDITIONAL role `N4` gives the slope clause"*. Under `AT-3` that role does not
exist: the slope is unconditional. Leaving those lines asserting a conditional
gate would have been a false doc-contract in the one spec whose census exists to
stop exactly that. The lines were therefore re-pointed to the **unconditional**
role. `AC9`'s **normative** element — *"the POST-EDIT COUNT is NORMATIVE while
every LINE NUMBER is DESCRIPTIVE"* — is held exactly: **36**, split 15 / 21,
every line still carrying one of the three pinned phrases and still classified
by Table 1 or Table 2. The same applies to the rendered role string, which now
reads `slope + blind-monitor both hard-gate; durable-corpus clauses are
report-only`.

### Routings, by id, no tracker edited

Unchanged from the post-section: the cause of the NEUTRAL breach — an estimator
that fires on an unperturbed run — and the durable-corpus headroom revision that
would have to precede any future promotion attempt are routed to **`TODO-654`**;
the bounded-plateau question and the width-1000 plateau demonstration to
**`TODO-634`**. This section edits no tracker file.

<!-- SITING-SECTION-END -->
