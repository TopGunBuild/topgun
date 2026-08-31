# SPEC-362 evidence manifest — the durable-layer instrument, and its frozen pre-registration

**What this document is.** §0–§8 are the **SOLE NORMATIVE SOURCE of SPEC-362's pre-registration**.
The spec bodies — `SPEC-362` and the follow-on measuring half — carry **pointers** to this text,
never copies. **Where any spec text and this manifest differ, THIS MANIFEST GOVERNS**, and the
difference is recorded as an append-only post-section entry — never by editing §0–§8. The
governance sentence is scoped to **spec** text; it does not reach **tracker** text, which is why §1
carries its own explicit clause about `TODO-654.md`'s stale predicate list.

**Status of §0–§8 at this commit: FROZEN.** They are authored **before any measurement artifact of
this family exists** and before the instrument they describe is written, and **no group after their
author edits them**. A correction to a frozen section lands as an **appended post-section addendum
under its own digest**, exactly as `spec356-manifest.md` §8A does. There is no placeholder, no
`<TBD>` slot and no value in §0–§8 that did not exist at the moment they were frozen; a slot for a
value that does not yet exist would license a post-freeze edit, which is the mechanism this freeze
exists to deny.

**How the freeze is proved.** Two independent mechanisms, neither of which takes a parameter from
the step being checked:

1. **COMMIT ORDER.** This file's first commit is strictly earlier than the first commit that
   introduces any `spec362-*.csv`, verified with `git merge-base --is-ancestor`. A rule chosen after
   seeing the numbers is not a rule, it is a preference.
2. **DIGEST.** The post-section header records the `shasum -a 256` of the **§0–§8 byte range**,
   computed at G1 with the identical one-liner the acceptance criterion re-runs later. The range is
   **self-delimiting**: it is the file up to the last byte before the frozen literal marker line
   `## POST-SECTION (append-only)`. The header sits **outside** the digested range, so recording it
   cannot change what it digests, and nobody supplies a line count at verification time.

**§9 onward — the executed record of the deciding cell — belongs to the measuring half
(`SPEC-362b`) and is deliberately absent here.** Writing it at this commit would destroy the
commit-order property the whole pre-registration rests on.

**What this document may not conclude.** This spec builds an instrument, freezes this
pre-registration, proves the instrument with short controls, and **concludes nothing about the
plateau**. Nothing in this manifest claims that the plateau is reached, that the plateau is met, or
that the durable layer is bounded. Where those phrases appear below they appear in exactly two
sanctioned uses: **quoted in order to be forbidden**, and in the **attributed restatement of
`TODO-634` §C's H2** — the prior framing this increment exists to test. The test a reader applies is
*"is this document making the claim, or naming it?"* Naming is permitted; claiming is not.

---

## §0 — Lineage, comparability, and build identity

### §0(a) — The lineage declaration and the pin

The instrument half's **merge commit IS the freeze boundary, after which no `.rs` byte may change**
(`spec356-manifest.md` §0(d), normative for this lineage).

**This section deliberately carries no hash and no slot for one.** `spec356-manifest.md` §0(a) states
the reason and it is carried verbatim: *"a spec cannot contain the hash of the commit that contains
it, and a placeholder token in a frozen section would license a post-freeze edit to the
pre-registration."*

**`SPEC-362b`'s FIRST action resolves the pin from `git log main`** and records it in **§9** of this
manifest — its own section — **and in every `matrix.txt`** it writes. **No frozen section (§0–§8) is
touched to record the pin.** The sentence above is a forward reference, not a value, and it is
complete as written.

Precedent for the seam: 356a → 356b, 361a → 361b.

### §0(b) — The comparability ledger

What survives into this reading and what does not is stated **once**, here, one row per statistic.
This ledger is normative; the spec bodies point at it and do not restate it.

| Quantity | Status for this reading |
|---|---|
| **356-lineage protocol constants** — the `spec355-width.sh` matrix literals, the **60 s primary CSV cadence**, `spec349c2-fit.awk` **unforked**, and the **8-window slicing one-liner** (by pointer: `spec355-manifest.md` §10.5.2) | **INSTRUMENT IDENTITY, not a measurement. Carried VERBATIM.** Carrying them unchanged is what makes the deciding cell read as a re-measurement of the same protocol rather than as a new lineage. The literals are enumerated in §5. |
| **The `spec355-w1000` reference series** — 0 → 646,306 B over 14,401 s, the series **ending at its maximum**, eight equal-window fits **113,657–244,197 B/h with no decay**, against a **512 B/h** bound | **The REFERENCE STATEMENT OF THE DEFECT — not a baseline to build on.** Quoted with its **channel clause** (below), which names the source of each number. *"Ended at its maximum"* holds on **BOTH** channels, which is why §2's classification of the reference does not turn on which channel a re-reader picks up. |
| **The redb corpus scan** (harness reader over a copy of the child's file) and the **WAL frame-kind census** | **Binary-independent by construction. Carried.** |
| **`TG-OR-005`'s status and the 512 B/h gate clause** | **UNTOUCHED by anything in this spec.** `TG-OR-005` stays `open (TODO-634)` and NAKED; `NAKED_BASELINE` stays **4**. Nothing here promotes or demotes a shipped gate. |
| **The `rss_kib` series of this lineage** | **READABLE — and it is readable only because this spec puts ZERO bytes under `packages/server-rust/src/` or `packages/core-rust/src/`.** `spec356-manifest.md` §0(b) FORFEITED that lineage's entire `rss_mb` series (*"no RSS number — level, slope, plateau or ceiling — may be read off any run in SPEC-356, in either direction"*) because its instrumentation retained ~60 s of samples per series **inside the exporter, in the server process**. RSS is at the top of `TODO-654`'s predicate list, so an in-server instrument would forfeit the very quantity this round exists to read. Every instrument this spec adds lives in `benches/soak_harness/`. |
| **The one declared departure** — `SOAK_SERVER_LOG` set to the target-scoped origin directive instead of unset | **DECLARED, PRICED, NOT ABSORBED.** It is the only difference from the parent runner that changes what the **server** does on the deciding cell and on the two pricing arms. Recorded in §0(c); design and controls in §6. |
| **The seeded ±0–1 s sampler jitter** (three `fs` series only) | **INSTRUMENT-SIDE, DECLARED.** It changes only **WHEN** the harness observes; it issues no request to the server the unjittered harness did not, so it is not rate-, shape- or duration-relevant to the workload. It is therefore a ledger row here and **not** a second §6 departure. Armed only under `--durable-reading`. |
| **`--durable-reading` itself** | **DECLARED-AND-ARGUED-NEUTRAL — argued, NOT measured.** See the argument immediately below. |

**The `--durable-reading` neutrality row, argued in the open.** The reference `spec355-w1000` ran
**without** the flag; all three controls run **with** it. **No control prices it**, so its neutrality
is an **argument**, not a measurement, and the argument is written down rather than implied. The flag
adds:

1. `fs::metadata` / directory-listing reads that **open no file**;
2. **no additional `/metrics` request** — the write-behind lag and
   `topgun_or_prune_epochs_exited_total` are parsed from the **ONE** body the existing scrape already
   fetches;
3. **all retention in the HARNESS process**, whose RSS nothing measures.

This family forfeited an entire `rss_mb` series once to an unledgered instrument cost
(`spec356-manifest.md` §0(b)); this row is cheap insurance. It is a **declared argument whose
falsification is a finding that routes to `TODO-634` by id** — it is not a claim of proven zero cost.

**The channel clause (frozen).** The reference run has **two** channels and they legitimately differ;
stated once here so a later reader does not read a contradiction into the numbers the ledger rests
on:

- **14,401 s** — the run's **DURATION** as recorded in the manifest (`spec355-manifest.md:1168`).
- **14400** — the last `elapsed_secs` **ROW** of the committed CSV (`spec355-w1000.csv`, 240 rows at
  the 60 s primary cadence). It is one sample tick short of the duration; that is the ordinary
  relationship between a run's wall clock and its last sample, not a discrepancy.
- **646,306 B** — the **in-process gauge** series (2,878 samples; `spec355-manifest.md:1171`:
  `firstBytes 0 → peakBytes 646,306 = lastBytes 646,306`).
- **629,654 B** — the **committed CSV's** last `tombstone_bytes`, which is also that column's
  **maximum**. Different channel, different sampling rate, therefore a different number for the same
  physical quantity.
- **"Ended at its maximum" holds on BOTH channels** — `last == peak` on the in-process gauge and
  `last == max` on the committed CSV.

**Channel-mixing is forbidden wherever two numbers are compared.** §6's pricing percentage takes both
of its numbers from the **same** channel (the 5 s durable series), for the same reason this clause
exists.

### §0(c) — The declared departure row, and its consequence

**The departure.** `spec355-width.sh:283-285` actively **unsets** `SOAK_SERVER_LOG`, with the comment
*"the child runs at `RUST_LOG=warn`, the pinned instrument-identity log level (server-side info
logging is a measurable cost on this write path)"*. The origin read (§4) requires that one line to be
emitted, so `spec362-durable.sh` **sets** the target-scoped directive instead of unsetting the
variable.

**Its consequence, stated here and designed in §6:**

- It is **declared**, not absorbed: it is priced by two ≤ 900 s arms (`logctl-on` / `logctl-off`) and
  its numbers are transcribed into the post-section.
- The pricing control is **honestly weak** and says so before the data: with n = 1 per arm it has
  **no** inferential power against a small perturbation and can only surface a gross one.
- Its **named accepted residual confound** is that the arms bound a **SHIFT IN THE LEVEL** of RSS and
  **not a DISTORTION OF THE GROWTH SHAPE**.
- A difference **at or above** SPEC-355's recorded 5.4 % run-to-run spread is a **recorded finding
  that routes to `TODO-634` by id** and, per §0(d), is grounds for a **re-pinning spec** — never for
  retuning the departure until the control passes.

**`crashctl`'s non-zero `--crash-interval` is NOT a §0(c)/§6 departure.** It is an
**instrument-branch parameter** of that control. The distinction is substantive, not bookkeeping: a
departure is a difference carried by the **measuring** configuration and therefore priced against the
reference lineage, whereas `crashctl`'s crash interval exists solely to **execute two instrument
branches** that the frozen cell's `--crash-interval 0` guarantees are never reached. It is never
applied to the deciding cell, it prices nothing, and its verdict is about the **INSTRUMENT** only.
The *"only difference that changes what the server does"* sentence in §0(b) is scoped to the deciding
cell and the two pricing arms precisely so that this third control does not textually contradict the
departure ledger.

### §0(d) — Build identity after the pin: no post-pin `.rs`

- **No `.rs` byte may land in this lineage after the instrument half merges.** Any post-pin `.rs`
  change invalidates every run taken before it.
- **`SPEC-362b` introduces ZERO `.rs` bytes**, checked mechanically:
  `git diff --stat <pin>..HEAD -- '*.rs'` is **empty**. That is what restores build identity inside
  the measuring half, and it is why the two halves share no `.rs` file.
- **If an instrument defect is discovered after this spec's own merge, the remedy is a NEW SPEC THAT
  RE-PINS** — never an edit absorbed under the pin, and never an edit to a frozen section here.

---

## §1 — The deciding series, and what may never be a predicate input

### §1.1 — The four deciding series (FROZEN)

All durable-layer, all `u64`, and — this is the property that does the work — **ALL EXTERNALLY
OBSERVABLE**: every one is read by the harness from `ps` or `std::fs` metadata; **none is the server
process's own self-report**.

| name | unit | source | site |
|---|---|---|---|
| `rss_kib` | KiB | `ps -o rss=` on the CHILD pid, derived from the existing `MemSample` stream | `main.rs` |
| `redb_bytes` | bytes | `fs::metadata({data_dir}/topgun.redb).len()` — metadata only, the file is never opened | `monitor::sample_redb_bytes` |
| `wal_bytes` | bytes | Σ `len()` of `*.log` segments under `{data_dir}/wal` | `monitor::sample_wal_retention` |
| `wal_segment_files` | count | number of `*.log` segments retained | `monitor::sample_wal_retention` |

**`rss_kib` is an EXACT round-trip, so the byte-frozen `sample_rss_mb` must NOT be "fixed" to produce
it.** `MemSample` carries only `rss_mb: f64` (`monitor.rs:133-136`) and `sample_rss_mb` computes
`kib / 1024.0` (`monitor.rs:152-163`); both are byte-for-byte untouched. `rss_kib = (rss_mb * 1024.0)
as u64` recovers the original integer KiB **exactly** — `ps -o rss=` yields an integer, and
multiplying or dividing a binary floating-point value by a power of two is exact (it moves the
exponent and leaves the mantissa alone). The one plausible reason an implementer would reach for the
untouchable function — to obtain a "lossless" KiB — **therefore does not exist**, and reaching for it
would break `tests/soak_monitor_calibration.rs` for nothing.

### §1.2 — Sampling cadence (FROZEN, and deliberately NOT uniform)

All four ride the existing `sampler_start` clock at a nominal `--mem-sample-interval` of **5 s**.

**The seeded ±0–1 s jitter is carried by THREE of them:** `redb_bytes`, `wal_bytes` and
`wal_segment_files` — the three `fs` series, which share one `std::fs` sampler task.
**`rss_kib` deliberately does NOT carry the jitter: it stays on the un-jittered `MemSample`
cadence** (its own `tokio::spawn` loop with a fixed `sleep(interval)`, `main.rs:548-577`, which this
spec does not touch). The asymmetry is **named, not implied**.

**Why the jitter exists — aliasing.** The write-behind flush default is 1,000 ms and the nominal
interval is 5 s, so the nominal cadence divides the flush period exactly, and **file sizes move at
flush boundaries**. A periodic sampler in fixed phase with a periodic mutation can capture the same
phase of every flush cycle for 4 h. That is a real hazard for the three `fs` series.

**`rss_kib`'s exemption rests on TWO reasons, both recorded because either alone would be weaker:**

1. **From the physics: the hazard does not apply to it.** RSS does not step at flush boundaries — the
   resident set moves **continuously**, under allocation, page-in and reclaim, with no periodic
   mutation for a periodic sampler to lock phase with. There is no phase to capture, so jittering RSS
   would buy nothing the aliasing argument can name.
2. **From the gate: it is the same series the memory gate of EVERY run reads**, including the
   blocking Soak Smoke G4b invocation. Changing its cadence would change the memory gate's input on
   every harness run, flagged and unflagged alike — an unledgered instrument change no key-set
   comparison can detect.

**The jitter is armed ONLY under `--durable-reading`.** Without the flag no jittered sampler task is
spawned at all, so every unflagged run — the Soak Smoke G4b invocation included — is
**byte-identical to today's** in its sampling behaviour.

**The seed reaches the binary as an explicit CLI flag, from the same shell variable the runner echoes
into `matrix.txt`.** `spec362-durable.sh` passes `--sampler-jitter-seed "$JITTER_SEED"`;
`matrix.txt` echoes `$JITTER_SEED`. **One source**, so the recorded seed and the seed the sampler
used cannot drift, and the seed is provably fixed **before** the run produces data rather than
derived at run time and reported afterwards. This holds for every run, controls included.

### §1.3 — `writebehind_lag_max` is an OBSERVATION column, not a deciding series

It is `max` over label sets of `topgun_wal_applied_watermark_lag`
(`monitor::parse_labelled_gauge`) — write-behind occupancy in `TODO-654`'s sense: the per-partition
gap between the highest assigned WAL sequence and the applied watermark
(`src/storage/wal/mod.rs:584-591`). It is recorded as observation-only for three stated reasons:

1. It is the **only** candidate deciding series read from the server's own `/metrics` — a
   **self-report by the process under measurement** — while the four above are purely external.
   Keeping the deciding set externally observable is worth more than the fifth column.
2. Its **durable consequence is already decided elsewhere**: retained-because-unapplied segments show
   up in `wal_bytes` and `wal_segment_files`, which **are** deciding.
3. Queue depth under a **peak** rule is asymmetrically noisy — a single late stall would produce a
   `MonotoneRising` shape and therefore a false `PlateauNotMet` for the whole run.

**The allowance survives as a RECORDED OBSERVATION POLICY, not as a predicate admission.**
`TODO-654` lists write-behind occupancy alongside RSS, redb file size and WAL retention, and this
spec does **not** drop it — it **records it, renders it, serializes it and routes it**. What it
declines to do is let it **decide**.

**The tracker clause, stated in the open.** `TODO-654.md`'s own predicate list still names
write-behind buffer occupancy. That file is **deliberately NOT updated** — this spec must not edit
either tracker file — and the demotion is **fixated here, in this manifest's routing record**. **The
manifest governs.** This clause is sited in §1 rather than only in spec text because the governance
sentence at the head of this document is scoped to *spec* text, not *tracker* text: without it, a
reader who opens the tracker first would be misled by a stale predicate list nobody is allowed to
correct.

### §1.4 — The EXCLUSION CLAUSE (FROZEN)

**No metric whose name begins `topgun_or_prune_` or `topgun_reclamation_`, and no quantity derived
from one, may enter ANY plateau predicate.** Those are **observation-only columns**. This is
mechanically enforced.

**The single carve, stated here so it cannot be read as contradicting the clause:**
`topgun_or_prune_epochs_exited_total` is used **only** as the origin reading's qualifier (§4) — that
is `TODO-654`'s own *"while epochs exit"* conjunct — and **never** as a plateau predicate input.

### §1.5 — The OBSERVATION-ONLY list (closed; recorded, rendered, serialized, never a predicate input)

- `writebehind_lag_max`
- the `topgun_ormap_tombstone_bytes` gauge series
- `topgun_or_prune_epochs_exited_total`
- every other `topgun_or_prune_*` / `topgun_reclamation_*` series
- every census field (§3)
- every `LiveCopy` census record (§3)
- the shell runner's `du`-based `wal_mb` / `redb_mb` / `disk_total_mb` columns — kept as the
  **independent cross-check**. `du -sk` measures **allocated blocks** and `metadata.len()` **apparent
  size**, so the two channels are **expected to differ and neither corrects the other**; only the
  harness channel is ever a predicate input.

**The list is CLOSED and every entry has a producer** — each name is either an existing in-tree
series, a field the instrument sites, or a column the shell runner already writes. A name with no
producer was struck rather than left standing: an observation column this clause obliges the harness
to record, render and serialize, but which nothing produces and no acceptance criterion reads, is a
no-field-home defect.

**`writebehind_lag_max` keeps its ABSENCE WITNESS even as an observation column**, so a silent
absence cannot masquerade as a flat series: `parse_labelled_gauge` returns `None` when the metric is
absent from the body, and that `None` is serialized as an **explicit `null`**, never as a zero.
Because the column is observation-only, its absence **no longer fail-closes the reading** — it is a
visible gap in an observation, which is exactly the disposition an explicit `null` carries.

---

## §2 — The frozen reading rule: threshold-free shapes over peaks and troughs

The rule is committed **before any data exists**, and it deliberately manufactures **no headroom
constant**. It reads **envelopes, not rates**: **peaks, never means or medians** — the same reason
`assess_tombstone_corpus_level` gives at `monitor.rs:699-703` (prune produces large downward
excursions that would drag a mean and hide a rising envelope) — and it compares **like against like**
with **no `3600/T` amplification**, which is the term carve 7 identified as the manufacturer of
measured slope noise.

**Rule version tag (frozen):** **`DURABLE-SHAPE-RULE v1`** — four deciding series, peak **and**
trough envelopes, 1/16-of-raw-span warmup exclusion, guards inherited from the corpus constants. Any
number characterizing this rule (§2.6) must name this tag.

### §2.1 — Frozen warmup exclusion

Before any split, the first **1/16 of the raw span** is dropped (`warmup_exclusion_index`): a
process's RSS warm-up peak lands in the first half and can make a genuinely late-rising series read
as `Levelled` by inflating `first_half_peak`. **The fraction is frozen here, pre-data, at a value
chosen for being a coarse binary fraction rather than for anything it does to a number.**

**Honest comparability note (§0(b)):** the `spec355-w1000` reference was computed with **no** such
exclusion, so the two are **not shape-comparable clause-for-clause**; the fraction is defined so that
it applies **identically to the reference on re-read**, which is the only form of comparability being
claimed.

This is a **separate mechanism** from the existing `exclude_boot_gap_samples`, which is
restart-oriented and — on the deciding cell's `--crash-interval 0` — has an **empty** exclusion list.
The warmup fraction is **not** a reuse of it, and **neither one may stand in for the other**.

### §2.2 — Splits and terms

For the retained series `S` with `n` samples over `span` seconds, split with the **SAME** index
function the existing clauses use (`last_half_split_index`, `monitor.rs:294`) applied **twice**, so
halves and quarters partition identically to every other consumer:

```
h = last_half_split_index(n)                      q = h + last_half_split_index(n - h)

first_half_peak      = max over S[..h]            first_half_trough      = min over S[..h]
last_half_peak       = max over S[h..]            last_half_trough       = min over S[h..]
third_quarter_peak   = max over S[h..q]           third_quarter_trough   = min over S[h..q]
last_quarter_peak    = max over S[q..]            last_quarter_trough    = min over S[q..]
```

**Why troughs as well as peaks (frozen, additive, still threshold-free).** The peak rule was chosen
against prune's downward excursions — but that makes it blind to a **saw with a rising floor**: prune
returns the series to a floor that is itself climbing while the peaks stand still. That is exactly a
leak's signature, and a peaks-only rule would read it as `Levelled`. The trough clauses are the
**mirror image** of the peak clauses — same splits, same strict `>`, same two conjuncts, **no
constant introduced**. A series genuinely on a shelf is **not** caught by them: its
`last_quarter_trough ≈ third_quarter_trough`, so the second conjunct does not fire.

```
rising_peaks_full   = last_half_peak    > first_half_peak
rising_peaks_tail   = last_quarter_peak > third_quarter_peak
rising_floor_full   = last_half_trough    > first_half_trough
rising_floor_tail   = last_quarter_trough > third_quarter_trough
```

### §2.3 — `SeriesShape` (FROZEN, evaluated in this order — fail-closed first)

1. **`Indeterminate`** — `n < min_samples` **OR** `span < min_span_secs` **OR** the sampler recorded
   zero samples. Carries a **named reason**. **Never "ok".**
2. **`MonotoneRising`** — `(rising_peaks_full AND rising_peaks_tail)` **OR** `(rising_floor_full AND
   rising_floor_tail)`. An envelope — the **peak** envelope or the **trough** floor — rose across the
   run **and was still rising at the end**. The reading records **WHICH** of the two fired (peaks,
   floor, or both), so the culprit is always **named, never inferred**.
3. **`RisingDecelerating`** — `rising_peaks_full OR rising_floor_full`, and not case 2: something
   rose across the run but was no longer rising at the end.
4. **`Levelled`** — neither the peak envelope nor the trough floor rose across the run.
   Threshold-free and unambiguous.

### §2.4 — `DurableReading` (FROZEN, evaluated in this order)

1. **`IndeterminateInstrument`** — ANY deciding series is `Indeterminate`. Fail-closed, with the
   offending series **named in the reason**.
2. **`PlateauNotMet`** — ANY deciding series is `MonotoneRising`.
3. **`NoRisingEnvelopeObserved`** — every deciding series is `Levelled` or `RisingDecelerating`.

**The third name is deliberately weak.** *"No rising envelope observed over this horizon"* is what
the data can support. **The quoted claim "the plateau is reached" is NOT a verdict this instrument
may emit**, and `SPEC-362b` may not upgrade it.

**The horizon clause (frozen).** A **non-`MonotoneRising` verdict at 14,400 s is explicitly NOT
DECIDING for the asymptote.** SPEC-355 is the precedent — its own 4 h horizon did not separate a slow
asymptote — so `RisingDecelerating` and `Levelled` at the frozen cell mean *"this horizon did not
show it"*, **never** *"there is nothing to show"*. **This sentence is the WHOLE of the horizon
treatment:** no ε floor, no discrimination margin, and no additional "flat but undiscriminated" state
is introduced anywhere in this rule. Fabricating such a constant is exactly the error carve 7's
`AT-3` punished, and it is **refused here by name**.

### §2.5 — The guards, and what they are not

`min_samples` and `min_span_secs` are the two guard parameters, defaulted to the existing
`DEFAULT_TOMBSTONE_CORPUS_MIN_SAMPLES` (`monitor.rs:590` = **4**) and
`DEFAULT_TOMBSTONE_CORPUS_MIN_SPAN_SECS` (`monitor.rs:582` = **600.0**), and evaluated on the
**retained** (post-warmup-exclusion) series. **They are guards, not thresholds:** they decide whether
a clause is **evaluated**, never which side it falls on. Two things are stated explicitly rather than
left to inference:

- **They check ONLY the sample count and the span — never the number of DISTINCT values.** A
  perfectly constant series is a healthy `Levelled`, **not** an `Indeterminate`. A guard that fired
  on low variance would convert the very outcome the instrument is looking for into an instrument
  failure.
- **They are inherited across a CATEGORY CHANGE, deliberately.** The constants were calibrated for an
  ~8-sample crash-checkpoint corpus series; the deciding series sample every ~5 s (≈ 2,880 samples
  over 4 h), where `min_samples = 4` is near-vacuous. That is acceptable **because they are guards**
  — but it is written down here, so **a shared constant is never read as a shared calibration**.

**Read against the reference series.** `spec355-w1000`'s tombstone series *"ended at its maximum"* on
**BOTH** channels (`last == peak == 646,306 B` in-process; `last == max == 629,654 B` on the
committed CSV — §0(b)'s channel clause), so **under this rule its shape is `MonotoneRising`**, and
the classification does not turn on which channel is re-read. **Pre-registering a rule that the known
reference classifies unambiguously is the point — the rule was not chosen to make a number come
out.**

**The two-DISTINCT-VALUES note, named as a property and not patched.** The note is about **two
effective points — two DISTINCT VALUES at a full sample count** — and **NOT** about two retained
samples. Two properties, recorded because they are **different terminal states on different routes**:

1. **Two distinct values, thousands of samples ⇒ `MonotoneRising`.** A series that holds one value
   for most of the run and **steps up once**, with the step landing between the third-quarter and
   last-quarter split points, has thousands of retained samples but only **two** distinct values. Its
   splits are all well-defined, both guards are cleared with enormous margin, and both peak conjuncts
   fire, so it classifies **`MonotoneRising`**. On `redb_bytes` this is **correct fail-closed
   behaviour**: the file really did grow. This is the shape the tie-heavy deciding series actually
   produce, so it is **designed, not a defect to be rediscovered**.
2. **Fewer than `min_samples` retained samples ⇒ `Indeterminate` ⇒ `IndeterminateInstrument`.** A
   genuinely short series hits the fail-closed FIRST rule. That is a **different terminal state on a
   different route**: `IndeterminateInstrument` carries §7's doubled-repeat obligation (instrument
   reasons), whereas `MonotoneRising` ⇒ `PlateauNotMet` routes to the `TODO-634` umbrella. **The two
   must never be conflated.**

**NO GUARD IS RELAXED to make either property hold.** `min_samples` stays at **4** and
`min_span_secs` at **600.0**, and the split arithmetic's degenerate two-sample behaviour is
**unreachable under those guards** — a fact about the arithmetic, not about this rule as frozen, and
it is **not claimed as one**. Relaxing `min_samples` so that a two-sample fixture could classify
`MonotoneRising` while still being described as *"the rule as frozen"* would be the
**satisfied-by-relaxing-the-parameter** class this lineage has treated as critical three times; it is
**refused here by name**.

### §2.6 — The pre-registered asymmetry, and the obligation to CHARACTERIZE its null side

**The asymmetry (frozen, recorded before the data).** Under exchangeability, any strict comparison
between two equal-length windows of the same series is near a coin flip; with four series and two
envelopes each, the rule's error is **deliberately placed on the `PlateauNotMet` side**. A false
`PlateauNotMet` **routes** — with its full per-shape numbers and the culprit series and envelope
named — to the `TODO-634` umbrella, which is competent to adjudicate noise-versus-growth and to
authorize a re-pinning spec (§7 forbids the measuring round from spawning another measurement round
*by itself*; it does not disable the umbrella). A false `NoRisingEnvelopeObserved` is the error the
rule is **built against**, and it has no such sanctioned recovery. **The asymmetry is the point, not
an oversight.**

**Its null side is CHARACTERIZED, not assumed.** Because the asymmetry is deliberate, its magnitude
must be a **stated number** rather than an intuition. A **seeded, deterministic `#[test]`** —
`null_characterization_stationary_exchangeable` — measures **`P(MonotoneRising)`** for
**`DURABLE-SHAPE-RULE v1` exactly as frozen: trough mirror included, warmup exclusion included, the
same guards, the same splits**, over **≥ 1,000 stationary exchangeable fixtures**.

**The design (PINNED here, not chosen at the keyboard).** This section is frozen while the test is
authored later, so a generator shape picked later could never be reflected back into the protocol it
instantiates, and the seed alone reproduces a number without pinning the design that produced it.
**Three parameters are PINNED:**

- **fixture length = 2,880** — the deciding cell's nominal sample count (14,400 s / 5 s), so the
  characterization runs at the scale the rule will actually be read at rather than at a convenient
  one.
- **value model = tie-rare** — the seeded PRNG draws `u64` from a wide range, so the probability of
  two equal values inside a fixture is negligible.
- **time axis = uniform 5 s step, span 14,395 s** — the 2,880 samples sit at `t = 0, 5, …, 14,395`
  seconds. The axis is pinned **beside** the length and the value model because **the frozen rule
  consumes the span TWICE and neither use is derivable from the length**: the `min_span_secs` guard
  (600.0) and the warmup exclusion, which drops the first 1/16 **of the raw span**. A cadence other
  than the cell's 5 s changes how many samples that drop removes, so the number would stop being a
  characterization of the rule at the scale it will be read at.
- **base multiset — exactly ONE**, permuted ≥ 1,000 times. **PINNED, not free:** it is what makes
  every fixture share identical order statistics, so *"the rule fired on ORDER alone"* is the thing
  being measured.

**Why tie-rare is the CONSERVATIVE choice for a false-alarm question (mathematics, not a threshold).**
Every `MonotoneRising` clause is a strict `>`, so equal values make a clause **FAIL**. A tie-rare
null therefore fires the rule **at least as often** as a quantized, tie-heavy one — and the real
deciding series are exactly the tie-heavy kind (`redb_bytes`, `wal_bytes`, `wal_segment_files` repeat
their value whenever nothing changed between two samples). The measured number is consequently an
**UPPER** per-series characterization, which is the direction this question wants its error in.

**The measurement MUST be NON-VACUOUS.** Because the characterization deliberately asserts nothing
about the rate, a rate of `0.000` produced by fixtures that all fell into `Indeterminate` would pass
and then be transcribed as this rule's declared false-alarm character — **a vacuity, not a
measurement**. So: **every fixture must clear the `Indeterminate` guards**, which the pinned axis
guarantees by construction (14,395 s ≥ 600.0 s; 2,880 ≥ 4), and **a fixture that lands in
`Indeterminate` is a GENERATOR DEFECT to be fixed, never a measurement to be reported.** The test
prints the `Indeterminate` count and a non-zero one **fails the characterization as a generator
defect**. This **introduces no threshold**: it asserts that the instrument was pointed at something,
exactly as the arming witness does, and it is compared against nothing. The no-assertion rule below
is about the **RATE**; an `Indeterminate` fixture produces no rate at all.

**Every other parameter the test chooses is declared FREE, with its reason. This list is CLOSED**, so
nothing the test picks is left to silent inference in a section that cannot be amended:

- **fixture count** — free **ABOVE the pinned floor of ≥ 1,000**. Reason: nothing is compared against
  the rate, so the count only buys resolution; the append records the exact value used.
- **the PRNG's identity and its SEED VALUE** — free in *value*, **obligatory in kind**: the PRNG must
  be `std`-only and in-tree, and the seed must be a **CONSTANT frozen in the test source** (never
  time-, environment- or run-derived), so the number reproduces byte-for-byte on re-run and the test
  is not flaky by construction. Reason: the algorithm and the seed choose *which* sample of the null
  you see, not what the null is; determinism is what is actually needed. The append names **both**,
  so the number reproduces without reading the test.
- **the value range's endpoints** — free, subject to being wide enough for tie-rare. Reason: tie-rare
  is the property that makes the measurement an upper characterization; any range that achieves it
  gives the same direction. The append records the endpoints used.
- **the reported rate's decimal precision** — free, because the append records the **raw counts
  (`fired` and `total`) beside the decimal**. Reason: a bare `0.000` cannot be distinguished from a
  small non-zero rate; the counts remove the ambiguity without pinning a format.

**The compound surface is BOUNDED, and series independence is explicitly NOT claimed.** The number is
**per-series** while the reading ORs over **four** series, so the quantity a `PlateauNotMet` actually
arrives at the umbrella with is the compound. This section states the **bound** rather than
manufacturing a measurement for it: by the **union bound** the compound false-alarm rate is
**≤ 4 × the per-series rate**, and that holds **without any independence assumption** — which is
precisely why it is written as a bound and **not as a product**. No constant is introduced.

**The test PRINTS the rate and asserts NOTHING about it. There is no pass threshold, no bound and no
margin** on the measured quantity — inventing one would be the fabricated constant carve 7's `AT-3`
refuted, and it is **refused here by name**. Neither the per-series rate, nor the compound bound, nor
any generator parameter is compared against anything.

**Where the number is filed.** This section (frozen) carries the **obligation, the protocol, the
pinned generator parameters, the constant-seed requirement, the rule version measured, the absence of
any threshold, and the pointer naming the entry**. The **measured rate itself** — together with the
generator's **FULL** parameters (PRNG identity and seed, fixture length, cadence 5 s and span
14,395 s, value range, fixture count, raw `fired` / `total` counts, the `Indeterminate` count, which
must be zero, and the rule version tag `DURABLE-SHAPE-RULE v1`) and the union bound beside it — is
transcribed **VERBATIM** into the append-only post-section, under its own digest, in the entry named:

> **Post-section entry `P1` — null characterization of `DURABLE-SHAPE-RULE v1`**

This split is not a weakening: it is the same discipline §0(a) applies to the commit hash. **A frozen
section may not carry a value that does not exist when it is frozen, because a slot for one licenses
a post-freeze edit.** The number is a measurement; measurements live in the append. **Both halves are
obligatory** — a missing half is a failure, and a `<TBD>`-style slot in this section would be a
freeze violation, not a compliance route.

**What the number is FOR (frozen).** It makes the pre-registered asymmetry a **stated magnitude**
rather than an intuition, so a `PlateauNotMet` from the deciding cell arrives at the `TODO-634`
umbrella with the rule's own null rate already on the record, next to the per-shape numbers and the
named culprit series. **It is not a gate and nothing is compared against it.**

**The characterization measures the FINAL rule as a whole.** Two design choices bear on the number in
**opposite** directions and neither may be measured in isolation: demoting `writebehind_lag_max`
(§1.3) shrinks the false-alarm surface from five deciding series to four, while the trough mirror
(§2.2) widens each series from one envelope to two. **A number measured against a peaks-only rule, or
against five series, does not satisfy this obligation.**

---

## §3 — The store-level census, and the observation fence

`scan_redb_tombstone_corpus` already iterates every row of `map__{map}` and decodes `RecordValue`
(`main.rs:1616-1655`). It **keeps that structure and its whole doc-contract** — the caller obligation
to scan a **COPY at a live checkpoint**, the `None`-on-failure contract, and the honest `Some(0)` from
a never-written table — and returns a **census** instead of one `u64`.

### §3.1 — `DurableCensus` — every field answerable BY THE STORE ALONE (all `u64`)

| field | definition |
|---|---|
| `keys_scanned` | rows iterated |
| `keys_undecodable` | rows whose `RecordValue` failed to decode (today silently `continue`d; now counted) |
| `or_map_keys` / `or_tombstones_keys` / `lww_keys` | unified vs legacy vs LWW variant share |
| `live_entries` | Σ `records.len()` |
| `live_tag_bytes` | Σ `entry.tag.len()` — the same unit as the tombstone bytes, so live and dead are comparable without conversion |
| `tombstone_entries` | Σ tombstone tags across BOTH variants |
| `tombstone_bytes` | Σ `tag.len()` across BOTH variants — **numerically identical to today's return value** |
| `tombstone_dup_entries` | Σ per-key (`len` − distinct); duplicates are counted **within a key**, never across keys, so no cross-key set is ever built |
| `keys_with_tombstones` | keys with a non-empty tombstone vector |
| `keys_all_dead` | keys with `records` empty **and** `tombstones` non-empty |
| `max_tombstones_per_key` | the per-key maximum |

### §3.2 — Why the census is defined against the durable store ALONE

Verified in the tree, not assumed:

- `src/storage/record.rs:550` — `RecordValue::OrMap { records: Vec<OrMapEntry>, tombstones:
  Vec<String> }` (`:541` is the `pub enum RecordValue` line, not the variant). **The durable
  tombstone blob is bare tags with NO epoch attribution**; the `tombstones` field's own doc-comment
  (`:553-567`, the epoch clause at `:560-567`) says the epoch association is **server-side metadata**
  and that migrating blobs to the epoch-indexed form is `TODO-566`'s **unlanded** obligation.
- `RecordValue::OrTombstones { tags }` (`:577`) is the **legacy, read-only** variant.
- `src/tombstone_frontier_impl.rs` persists only the **cursor** (`:49`, `:1274` `persist_tx`); the
  `epoch → tags` index is **NOT durable**.

**Consequence, stated in the open:** *"dead = below the reclamation ceiling"* is **NOT computable from
durable state**, and a census that joined durable tags against the in-memory `epoch_tags` index would
**import the very instrument under suspicion into the predicate**. So the census classifies **only
over what the store can answer by itself**, and **any in-memory-index join is an OBSERVATION column
that may not enter a predicate.**

### §3.3 — What the census may and may not decide

`tombstone_entries` **as a SERIES** is the direct read on **H2** — the prior framing this increment
exists to test, stated by `TODO-634` §C and attributed to it, not asserted here: *"RAM is bounded, the
durable layer is suspect"*, i.e. *does a store-level delete ever happen?* — and its `ever_fell` observation is exactly that question. **But at the frozen cell's
`--crash-interval 0` there is NO recovery checkpoint, so the census has exactly ONE sample (the
terminal scan).** Stated here rather than papered over:

- **The census is a STRUCTURAL end-of-run read on the frozen cell, not a series.**
- **No census quantity is a deciding-series input** (§1.5 lists every census field as
  observation-only).
- The `--live-census-interval` sampler exists so the series read is *available*, and it **ships
  DISARMED (0)** with its reason stated — mirroring `DEFAULT_TOMBSTONE_CORPUS_CEILING_BYTES`
  (`monitor.rs:592-602`; its doc-comment opens at `:592` and the const is `:602`, while `:590` is
  `DEFAULT_TOMBSTONE_CORPUS_MIN_SAMPLES`, a different constant).

### §3.4 — The OBSERVATION FENCE (frozen)

**`LiveCopy` census records are OBSERVATION ONLY and may not enter any predicate.** A byte copy of a
**live** redb file is a **smeared image**, so its scan is best-effort by construction.

**The fence is a TYPED PROPERTY of the record, not a convention:** every census record carries its
`CensusSource`, and `LiveCopy` records land in **their own tally** and are **NEVER** added to
`CorpusScanTally.samples` / `scans_attempted` / `scans_failed`. That is what keeps carve 7's
estimator's input **byte-identical whether the sampler is armed or not**.

---

## §4 — The origin reading: a LOG-SINK read, not a code change

SPEC-360 already landed the removal-site line at `src/tombstone_frontier_impl.rs:1015-1026` (the
macro itself; its WHY-comment is `:1004-1014`) — eight fields, target
`topgun_server::tombstone_frontier::removal`, inside the `Some(refs)` arm — and **routed the
production read to `TODO-654` by id**. **This spec READS that instrument. It changes no server
byte.**

### §4.1 — Arming (frozen)

The child's `RUST_LOG` comes from `SOAK_SERVER_LOG` (`process.rs:227-230`). The frozen directive is
**target-scoped**:

```
SOAK_SERVER_LOG="warn,topgun_server::tombstone_frontier::removal=info"
```

The target is the one SPEC-360 chose precisely so *"the line [is] selectable without a `kind`
discriminant field"* (`tombstone_frontier_impl.rs:1012-1014`). **Nothing else moves off `warn`**, so
no other write-path logging is enabled.

### §4.2 — Capture and parse (frozen)

- **Capture.** `OriginCapture` in `process.rs`, fed from the existing per-line reader beside
  `PanicWatch::record_line`. It retains lines containing the origin target, up to
  `ORIGIN_CAPTURE_CAPACITY` = **50,000** — roughly two orders of magnitude above the ~1 line per
  epoch-exit cadence the 8 h data implies for a 4 h cell — and counts `dropped` beyond it. **A drop
  is NEVER silent: `dropped > 0` forces `IndeterminateInstrument`.** Retention is in the **HARNESS**
  process, whose RSS nothing measures.
- **Parse.** `monitor::parse_origin_line` extracts the eight `key=value` fields (`ts`, `op_seq`,
  `epoch`, `refs_returned`, `refs_at_entry`, `bytes_returned`, `watermark`, `ceiling`) from the
  **rendered** line by **whitespace-token scan**, returning `None` unless **all eight** parse. Token
  scanning rather than a format-position match, so the parser is **not coupled to the `tracing` fmt
  layer's field/message order**.

**Every classifier input is a TYPED FIELD on the aggregate; none is inferred.**
`classify_origin_reading` reads `matched`, `unparsed`, `dropped`, `lines`, and three that are sited
explicitly: **`restarts: u64`** (threaded from the recovery-checkpoint counter), **`armed: bool`**
(derived from `process::effective_server_log_filter()`) and **`epochs_exited: u64`** (threaded in
from the widened one-body scrape). The aggregate also carries the reading's **named reason**, which
serializes as `origin_reason`.

### §4.3 — `OriginReading` (FROZEN, evaluated in this order — fail-closed first)

1. **`IndeterminateInstrument`** — the filter was **not armed** for this run (`armed == false`), **OR**
   `dropped > 0`, **OR** `unparsed > 0`, **OR** the server **restarted** during the run — read as
   `restarts > 0`, from the recovery-checkpoint counter **and from nothing else** (the
   `epochs_exited` qualifier resets on restart and would be misread).
2. **`ReachedInProduction`** — ≥ 1 line with `refs_returned == 0 && refs_at_entry > 0`. This is
   `TODO-654` reading **(a)**: the `Some(vec![])` state is reached in production, and **the origin
   question becomes a code-level fix spec**.
3. **`PartialDivergence`** — ≥ 1 line with `refs_returned != refs_at_entry`, none of them of the
   reading-(a) shape. **A fourth physical state the two pre-registered readings do not cover**;
   naming it is what stops it being silently folded into (b). **Routes, does not diagnose.**
4. **`NotReachedEqualRefs`** — ≥ 1 line and **every** line has `refs_returned == refs_at_entry`. This
   is reading **(b)**: not reached under this load; **PD-F12's 8 h divergence needs a different
   account — route, do not diagnose.**
5. **`NoLinesWhileEpochsExited`** — zero lines, filter armed, and
   `topgun_or_prune_epochs_exited_total > 0`. This is reading **(c)**: the drain arm is not reached
   (dark path / gate).
6. **`NotObservedAtHead`** — zero lines, filter armed, and `epochs_exited_total == 0`. **SPEC-360's
   own `NOT-OBSERVED-AT-HEAD` pre-registration, carried verbatim**: nothing exited, so the arm could
   not have been reached.

### §4.4 — Dispositions carried from SPEC-360 (normative)

- **The null-read disposition:** **the instrument SHIPS regardless; a null read ROUTES.** It does not
  stall this spec, and **it does not reopen the diagnosis line that `E-C` HARD-STOPPED at SPEC-358**.
- `ReachedInProduction` **routes to a code-level fix spec**; `PartialDivergence`,
  `NotReachedEqualRefs`, `NoLinesWhileEpochsExited` and `NotObservedAtHead` **route, and are not
  explained here**.
- **The qualifier carve.** `topgun_or_prune_epochs_exited_total` is used **only** as this reading's
  qualifier — `TODO-654`'s own *"while epochs exit"* conjunct — and **never** as a plateau predicate
  input. §1.4 states the same carve from the exclusion clause's side; the two are one rule seen from
  two directions.
- **Naming discipline.** In `monitor.rs` the qualifier is named **BY PARAMETER** (`epochs_exited`),
  **never by metric literal**; the literal lives in `main.rs`, which is where the scrape that reads
  it lives. The exclusion clause's mechanical grep is therefore **not softened**. If a `monitor.rs`
  doc-contract ever genuinely needs the literal, the check is amended by **enumerating the permitted
  site explicitly**, never by loosening the pattern.

---

## §5 — The frozen cell parameters

**Frozen here; EXECUTED BY `SPEC-362b`, not by this spec.**

- **duration 14,400 s, n = 1, `TOPGUN_EPOCH_WIDTH=1000`, on this machine (M1).**
- **Byte-comparable to `spec355-w1000`** — 0 → 646,306 B over 14,401 s, series ended at its maximum,
  8 equal-window fits 113,657–244,197 B/h with no decay, against a 512 B/h bound — **with §0(b)'s
  channel clause naming the source of every one of those numbers.**
- **The duration is PINNED BY COMPARABILITY, and a longer cell is REFUSED, not deferred.** A 28,800 s
  variant was considered and **rejected**: the reference is exactly 14,400 s (its last `elapsed_secs`
  is `14400` — the CSV channel), and byte-comparability against that reference is the whole basis on
  which this cell reads as a **re-measurement** rather than as a new lineage. Doubling the duration
  would buy horizon at the cost of the comparison the cell exists to make. **Recorded here so it is
  not re-litigated at audit.**

**The 356-lineage protocol constants, carried VERBATIM** (§0(b) classes them as instrument identity,
not measurement):

```
CHURN_CLIENTS=6      KEYSPACE=200        OR_CHURN=true       OR_KEYSPACE=48
OR_EVERY=5           WRITE_INTERVAL_MS=20                    WRITES_PER_LIFE=200
OFFLINE_KEYS=3       CONFIRM_INTERVAL=2  CRASH_INTERVAL=0    STEADY_INTERVAL=300
QUIESCE=3            MEM_SAMPLE_INTERVAL=5                   WAL_FSYNC=batched
the neutralized memory gate
```

together with the **60 s primary CSV cadence**, `spec349c2-fit.awk` **unforked**, and the **8-window
slicing one-liner** (`spec355-manifest.md` §10.5.2).

**The cadence clause, carried from §1.2 verbatim including its asymmetry.** The seeded ±0–1 s sampler
jitter rides on the `MEM_SAMPLE_INTERVAL=5` nominal and is **declared, not absorbed**: it changes only
**WHEN** the harness observes and issues no request to the server the unjittered harness did not, so
it is **not rate-, shape- or duration-relevant to the workload** — which is why it is a §0(b)
instrument-side row and **not** a second §6 departure. The jitter is on the **THREE `fs` series**
(`redb_bytes`, `wal_bytes`, `wal_segment_files`); **`rss_kib` stays on the un-jittered `MemSample`
cadence** for the two recorded reasons — RSS moves continuously, so the flush-boundary aliasing the
jitter exists against has **no phase to capture** on it; and it is the same series the memory gate of
**every** run reads, including the blocking Soak Smoke G4b, whose input cadence may not change. The
jitter is armed **only under `--durable-reading`**, so an unflagged run is **byte-identical to
today's**. Its seed is supplied to the binary by the runner as `--sampler-jitter-seed "$JITTER_SEED"`
and echoed into `matrix.txt` **from that same shell variable**, for every run, controls included.

**Artifact shape:**

- The **primary CSV header stays BYTE-IDENTICAL**:
  `elapsed_secs,rss_mb,wal_mb,redb_mb,disk_total_mb,tombstone_bytes`. The new durable quantities go to
  a sibling `*.durable.json`, **never as a column on this header**.
- `--live-census-interval` is **DISARMED (0)** on the deciding cell (§3.3).

**Acknowledged risk, recorded in the open.** SPEC-355 showed a 4 h horizon **may not separate a slow
asymptote**, so **INDETERMINATE-for-physics is a LIVE OUTCOME** of the deciding cell. §7 governs it.

---

## §6 — The one declared departure, and the three short controls

### §6.1 — The departure, and its scope

`spec355-width.sh:283-285` actively unsets `SOAK_SERVER_LOG`. `spec362-durable.sh` **sets** the
target-scoped directive (§4.1) instead.

**Scoped precisely: on the DECIDING CELL and on the two PRICING ARMS (`logctl-on` / `logctl-off`) this
is the ONLY difference from the parent runner that changes what the SERVER does.** The two
harness-side differences — the seeded sampler jitter and `--durable-reading` itself — issue no work
the parent runner did not; **each carries its own declared §0(b) row rather than hiding behind that
sentence.** `crashctl`'s non-zero `--crash-interval` is an **instrument-branch parameter of that
control**, not a departure (§0(c)) — which is why the sentence is scoped as it is, rather than
freezing a departure ledger its own third control contradicts.

**Environment discipline: `TOPGUN_LOG_FORMAT` is unset alongside the parent's seven.**
`spec362-durable.sh`'s discipline block (inherited from `spec355-width.sh:256-268`) adds
**`unset TOPGUN_LOG_FORMAT || true`**. Reason, stated because this is the **FIRST** spec in the
lineage that **parses a rendered server log line**: `src/service/middleware/observability.rs:130-132`
switches the fmt layer to `.json()` when `TOPGUN_LOG_FORMAT=json`; the harness never calls
`env_clear()`, so the child inherits the operator's shell; and an exported `json` would turn every
origin line into `unparsed > 0` ⇒ `IndeterminateInstrument` **and would change the cost of the one
declared departure silently, and differently between the control arms and the deciding cell** if the
two were run from different shells. The failure is fail-closed and the arming witness would catch it
on the armed arm, which is why this is **discipline rather than a new guard**; the `unset` removes the
shell-dependence at its source. **The line is part of the runner's enumerated diff**, so the closed
list does not reject its own fix.

**Why this does NOT repeat SPEC-356's RSS forfeiture.** That forfeiture was caused by ~60 s of
**in-exporter sample retention inside the server process**. This departure adds **no retention
whatsoever** — one formatted event per drained epoch, written to a pipe the harness drains, at a
cadence the 8 h data puts near 1/minute at width 1000. **The difference in kind is stated in the
ledger; it is not offered as a proof.**

### §6.2 — The pricing control (pre-registered, honestly weak)

Two runs at **≤ 900 s**, same binary, same matrix: one arm with the directive **ARMED**
(`spec362-logctl-on`), one with it **UNSET** (`spec362-logctl-off`).

**Reported: both arms' `rss_kib` last-half PEAK and last-half MEAN, the observed difference as a
percentage, and SPEC-355's recorded run-to-run spread of 5.4 %.**

**The CHANNEL is named explicitly, and it is ONE channel for both numbers:** the peak is
`SeriesShapeRow.last_half_peak` and the mean is `SeriesShapeRow.last_half_mean`, **both** read off the
`rss_kib` row of that arm's `*.soak.durable.json` — i.e. the **5 s durable series**, on the same
retained samples and the same `last_half_split_index` split. **Neither number is taken from the
primary CSV's 60 s `rss_mb` column**, which stays the independent cross-check (§1.5) and is never
mixed into this comparison: a percentage whose two halves came from two cadences is exactly the
channel-mixing hazard §0(b)'s channel clause closed for the reference numbers.

**Pre-registered honesty clause (verbatim, part of the pre-registration precisely so a non-difference
is not read as a proof of zero effect):**

> With n = 1 per arm this control has **no** inferential power against a small perturbation; it can
> only surface a gross one.

**Named accepted residual confound (verbatim, frozen):**

> These controls bound only a **SHIFT IN THE LEVEL** of RSS, not a **DISTORTION OF THE GROWTH SHAPE**.
> A log sink's allocator behaviour over 4 h is not observable in a ≤ 900 s arm, so a departure that
> leaves the level alone while bending the slope would pass this control unseen.

It is named as an **accepted residual**, not argued away.

**Disposition.** A difference **at or above** the 5.4 % spread is a **recorded finding routed to
`TODO-634` by id** in the same append — **never resolved by retuning the departure until the control
passes.**

### §6.3 — The arming witness (decisive, two-directional)

- The **ARMED** arm must show `matched > 0` and `unparsed == 0` (and `dropped == 0`).
- The **UNSET** arm must show `matched == 0`.

Together they prove **the capture path is live** and that **a zero count under the ARMED filter is a
real null read rather than a dark instrument**. This is a **mechanical** check, not a statistical
one, and unlike §6.2 it **is** decisive.

### §6.4 — The third control: `spec362-crashctl`, a DOUBLE witness

**≤ 900 s, `--crash-interval` non-zero, same binary, same matrix, run with restarts.** It exists
because two instrument branches are otherwise **never executed anywhere in this spec**:

1. The census's **`Checkpoint`** branch — the copy-then-scan path at the recovery checkpoint — which
   the deciding cell's `--crash-interval 0` guarantees is never reached, and which would therefore
   **ship having never run once**.
2. The origin reading's **`restarts > 0` ⇒ `IndeterminateInstrument`** guard, which is asserted over
   fixtures but **never observed live**.

**Expected result, recorded PRE-DATA:** at least one census record with `CensusSource::Checkpoint`
present, and `originReading == "INDETERMINATE_INSTRUMENT"` **with the restart named as the reason**
(not the filter, not a drop — the run is executed with the directive **ARMED**, so this isolates the
restart guard), **and the root-level `restarts` count itself > 0**, so *"the reason names the
restart"* has provenance in the **artifact** rather than only in a reason string.

**Its verdict is about the INSTRUMENT only.**

### §6.5 — What the controls put on the record, and what they may never be quoted for

**All three arms record their FULL shape set into the post-section:** each arm's four `SeriesShape`
values (with the **firing envelope named**) and its resulting `DurableReading`.

**Honest caveat, stated WITH the data and not after it:**

> The controls carry a **real** load and therefore real growth, so a `MonotoneRising` or a
> `PlateauNotMet` observed on a control arm is **NOT per se a false positive** and may not be quoted
> as one.

The control record is a record of **how the frozen rule behaved on runs whose physics is not
controlled**; the rule's **null** side is characterized by §2.6's seeded fixtures, which is a
different instrument for a different question, and **neither substitutes for the other**.

**`--durable-reading` is priced by ARGUMENT, not by these controls** — all three arms run with it
armed, so **none of them prices it**. Its declared-and-argued-neutral row lives in §0(b) and is
repeated here so **the gap is visible where the controls are described, not only where the ledger
is.**

**`X8` discipline is carried:** this spec contains **no 4 h cell, no width-1000 deciding matrix and no
new measurement lineage**. **Short controls (≤ 900 s) are the only measurement it may hold** — three
of them, each ≤ 900 s individually, with **no combined-budget allowance and no fourth control** —
their verdicts are about the **INSTRUMENT** and never about the plateau, and if a long cell ever looks
unavoidable it is **pre-shaped and routed to `SPEC-362b`, never run here**.

### §6.6 — The post-section entries this section obliges

> - **`P2 — pricing control record`** — both arms' `rss_kib` last-half peak and mean (one channel),
>   the observed difference as a percentage, the 5.4 % spread, the §6.2 honesty clause verbatim, and
>   the level-not-shape residual-confound clause verbatim.
> - **`P3 — control arm shape sets`** — for **all three** arms (`logctl-on`, `logctl-off`,
>   `crashctl`): the four `SeriesShape` values with the firing envelope named, and the resulting
>   `DurableReading`, with §6.5's caveat restated verbatim alongside them.
> - **`P4 — crash control double witness`** — the `CensusSource::Checkpoint` record, the
>   `INDETERMINATE_INSTRUMENT` reading, its restart-naming reason and the root-level `restarts`
>   count.

---

## §7 — The stopping rule, and the doubled-repeat cap

`TODO-654`'s own stopping rule, carried **verbatim in scope and consequence**:

> This is a MEASUREMENT increment, not a diagnosis round: whatever it reads, the ReclamationRegistry
> family proceeds. A null or surprising read routes to a fix spec or to `TODO-634`; **it never spawns
> another measurement round by itself.**

**The cap.** The §8.1-style **doubled-repeat obligation applies ONLY if the reading is INDETERMINATE
for INSTRUMENT reasons, never for physics.**

**The physics branch, with its addressee NAMED.** An **`INDETERMINATE-for-physics`** outcome — the
4 h horizon not separating a slow asymptote (§5's acknowledged risk, §2.4's horizon clause) — is a
**recorded reading that ROUTES**, not a trigger for a longer run. The quoted stopping rule names *"a
fix spec or `TODO-634`"* for null or surprising reads, but this branch needs its **own** named target
or the family's terminal state is merely recorded rather than routed. **It routes BY ID to the
`TODO-634` umbrella**, which is the body competent to decide what a non-separating horizon warrants.
**It does not authorize this round, or the measuring round, to run a longer cell.**

**Routing is by ID only**, in this manifest's post-section. **No tracker file is created, edited or
deleted by this spec.**

---

## §8 — `SPEC-362b`'s obligations and prohibitions

### §8.1 — MUST

1. **Resolve the pin from `git log main` as its FIRST action**, and record it in **§9** and in **every
   `matrix.txt`**.
2. **Verify `git diff --stat <pin>..HEAD -- '*.rs'` is EMPTY.**
3. **Execute `spec362-durable.sh`'s frozen cell** (§5).
4. **Commit the full artifact set.**
5. **Write `§9`, and only `§9`.**

### §8.2 — MAY NOT

1. **Edit any frozen section (§0–§8)** or **any existing post-section entry**.
2. **Introduce any `.rs` byte.**
3. **Upgrade `NoRisingEnvelopeObserved` into a plateau verdict.** `NoRisingEnvelopeObserved` is the
   strongest reading the instrument may emit, and only the deciding cell may emit it at all; *"the
   plateau is reached"* is quoted here **solely in order to forbid it**.
4. **Promote or demote any gate.**
5. **Touch `INVARIANTS.md`, `scripts/check-invariants.sh`, or either TODO file.**

### §8.3 — A naming note that is NOT a licence to edit this section

If the conductor names the measuring half something other than `SPEC-362b`, **the rename lands as an
appended post-section entry naming the substitute id** — **NOT** as an edit to this section. §0–§8
are append-only; a rename is exactly the kind of "small, obviously harmless" edit the freeze exists to
refuse. **Nothing in the code moves either way**; the seam is identified by its obligations above,
not by its name.

### §8.4 — Tracker integrity record (recorded at G1; not part of §8.1/§8.2's normative content)

`.specflow/` is not git-tracked, so the *"neither TODO file is edited"* prohibition is checked **by
digest**. Recorded here, at the freeze, so it is inside the digested range and cannot be quietly
restated later:

| file | `shasum -a 256` at G1 |
|---|---|
| `.specflow/todos/TODO-634.md` | `127ad250bcd468c10368dea371b29cbbedf3536d8a1be8f41cefda8261293e07` |
| `.specflow/todos/TODO-654.md` | `19ff3419ade1eda7b43f2588dd32a6460498c34c93aaf6c611c23363b9140e0a` |

**`TODO-634.md`'s checkbox census at G1** — recomputed at completion and required to be unchanged:

| census | value | command |
|---|---|---|
| top-level boxes | **7** | `grep -cE '^- \[' .specflow/todos/TODO-634.md` |
| ticked | **2** | `grep -cE '^- \[x\]' .specflow/todos/TODO-634.md` |
| unticked | **5** | `grep -cE '^- \[ \]' .specflow/todos/TODO-634.md` |
| indented | **0** | `grep -cE '^  +- \[' .specflow/todos/TODO-634.md` |

**`.specflow/todos/TODO-654.md` MUST NOT BE DELETED when this spec completes.** It is treated as an
**umbrella**, exactly as `TODO-634` is: it still owns the deciding cell, the measuring half's
obligations and the routing target for every reading this instrument produces, and the digest above
is asserted to **reproduce** at completion — an assertion a deleted file cannot satisfy. Completing
this spec closes the **INSTRUMENT half only**.

---

**End of the frozen sections.** Everything below the marker line is append-only and is **not** part of
the digested range.

## POST-SECTION (append-only)

**Digest of §0–§8, recorded at G1.** This header sits **OUTSIDE** the digested range — the range is
the file **up to the last byte before the marker line `## POST-SECTION (append-only)`** — so writing
this header cannot change what it digests. The digest was produced at G1 with the **identical**
one-liner the freeze check re-runs later; the **range and the COMMAND are both pinned**, so a later
mismatch can only mean the frozen bytes moved, never that the two steps digested the same bytes two
different ways.

```
awk 'BEGIN{p=1} /^## POST-SECTION \(append-only\)$/{p=0} p' \
    packages/server-rust/benches/soak_harness/evidence/spec362-manifest.md \
| shasum -a 256
```

```
frozen-sections  c7f3373fb44bd80beb9e9c955e5d6e46e4d9a88a3abbd1579b802bc5e4882f54
```

**Rules for this section.** Entries are **APPENDED ONLY**. No existing entry is ever edited; a
correction to an entry is a **further entry** naming what it corrects. A correction to a **frozen**
section (§0–§8) is likewise an entry here — never an edit above the marker. Each appended batch
carries its **own** `shasum -a 256`, recorded beside it, and the frozen-sections digest above is
**re-verified as still reproducing** at the moment of each append.

**The entries the frozen sections oblige, by name** (a checklist, not a set of slots — nothing here
is a value):

- **`P1` — null characterization of `DURABLE-SHAPE-RULE v1`** (obliged by §2.6): the measured
  `P(MonotoneRising)`, the raw `fired` / `total` counts, the `Indeterminate` count (which must be
  zero), the PRNG identity and its constant seed, the fixture count, the fixture length, the cadence
  and span, the value range, the rule version tag, and the `≤ 4 ×` union bound beside the rate.
- **`P2 — pricing control record`** (obliged by §6.2 / §6.6).
- **`P3 — control arm shape sets`** (obliged by §6.5 / §6.6).
- **`P4 — crash control double witness`** (obliged by §6.4 / §6.6).
- **`P5 — routing record`** (obliged by §1.3, §6.2, §7): every finding routed by **id**, including the
  fixation of `writebehind_lag_max`'s demotion to an observation column, and any pricing finding at or
  above the 5.4 % spread.

*(No entries yet. Written empty at G1.)*
