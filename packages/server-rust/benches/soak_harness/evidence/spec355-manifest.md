# SPEC-355 evidence manifest — classifying the tombstone-byte gate breach at the production epoch width

**Status of this document at the pre-registration commit:** §0–§9 are **FROZEN BEFORE ANY
MEASUREMENT RUN EXISTS**. §10 onward are the executed record and are written as the runs land.

The point of freezing §0–§9 is not ceremony. The question this spec answers has exactly two
answers, one of which ("width-scaled prune math") makes a red gate go away and the other of which
("a regression") does not. A rule chosen after seeing the numbers is not a rule, it is a
preference. So the rule is committed first, in a commit that git can be asked to prove came
earlier (`git log --follow` on this file vs the first `spec355-*.soak.json`).

---

## §0 — What is being decided, and what is out of scope

SPEC-349c2 ran the first two live 60-minute soaks at the **production default epoch width**
(`TOPGUN_EPOCH_WIDTH` unset → 1000). Both breached SPEC-345's promoted tombstone-byte HARD gate by
two and a half orders of magnitude:

| Run (commit `d6922f08`, `walFsync=batched`, churn 6, keyspace 200) | tombstone-byte slope | bound | over |
|---|---|---|---|
| emitter ON | 248,148.9 B/h | 512 B/h | **485×** |
| emitter OFF | 283,066.2 B/h | 512 B/h | **553×** |

SPEC-345 recorded that the gate's PASS is demonstrable "with either `TOPGUN_EPOCH_WIDTH=100` +
≥15 min (done: −1707 B/h), **or ≥30–60 min at the default width**". SPEC-349c2 is the first live
test of that **second disjunct**, and it fails. Exactly one of two things is true:

- **branch (1)** — a **regression** landed after SPEC-345's measurement, or
- **branch (2)** — the second disjunct was an **unverified extrapolation**: the bound was only ever
  measured at width 100, and tombstone residency/ramp scales with epoch width.

**Explicitly out of scope.**

- This is **not** "make the gate green". A re-derivation chosen because it turns the gate green is
  the failure mode this document exists to prevent (§8, §9, and the AC8/AC8b anti-tautology gates).
- SPEC-348's **disk (WAL + redb) gate is unaffected** either way. It derives from `du` over real
  paths and never reads the tombstone gauge (`spec349c2-manifest.md` §7.2). Only the 72 h soak
  (TODO-484) and TODO-586 are gated on this item.
- If the fork resolves to a regression, this spec **names the culprit and hands the fix to a
  spun-off spec**. It does not fix a prune regression inline — that would invalidate the very
  measurements it exists to produce, which is why SPEC-349c2 deferred in the first place.
- If tombstone bytes turn out to be **genuinely unbounded at the production width**, that is not a
  shrug either: §9's R5b disposition gives it a spun-off prune fix-or-redesign spec with a named
  owner and pre-soak-blocker status, plus an immediate `INVARIANTS.md` entry. **No outcome of this
  spec is allowed to terminate in a paragraph of this file.**

---

## §1 — Settled inputs (carried from SPEC-349c2; NOT re-derived here)

These four preconditions are **inputs**, established by SPEC-349c2 Review v1 and its committed
artifacts. This spec does not re-measure them, and time spent re-measuring them is time not spent
on the fork.

| Precondition | Settled finding | Provenance |
|---|---|---|
| Tracked confirm-apply client present and working | `no_ack` defaults `false`, the runner passes no `--no-ack`, the call site is `if !cfg.no_ack`; `confirms=1727`, `confirmErrors=51` (~3 %, which cannot account for a 485× overshoot) | `spec349c2-manifest.md` §7.1 / §7.3; `spec349c2-emitter-{on,off}.harness-console.log` line 75 |
| Epochs cycled | `lastConfirmedEpoch` advanced monotonically across all 11 checkpoints in both arms, reaching **113**; last-half window 1797 s ≈ 56 epochs; `samples=720` (not a blind monitor) | same logs; checkpoint lines cited in `spec349c2-manifest.md` §7.3 |
| The low-water mark actually advanced | Corroborated **gauge-independently** by the post-run redb corpus scan (ON 200,860 B, OFF 239,256 B) — a stuck gauge would not reproduce in a second instrument | `spec349c2-emitter-{on,off}.mechanism.json` |
| Not emitter-attributable | Both arms fire at the same order (OFF is 14.1 % *higher* than ON); the census shows `removeFrames == 0` in both | `spec349c2-manifest.md` §3, §7.1 |

### §1.1 — The one prior observation that is explicitly WEAK, and is NOT used

A **90-second** run at width 1000 was executed during SPEC-349c2 **only to validate the CSV
instrument**. It showed the gauge rise then fall (`peakBytes=41930`, `lastBytes=20664`,
`samples=18`).

> **WEAK — NOT EVIDENCE.** Ninety seconds at width 1000 is roughly 1.4 epochs; the run is
> warm-up-dominated and settles nothing about plateau, bound or regression. **It is not cited as
> evidence anywhere in this document**, and it played no part in any determination recorded below.
> The discriminating experiment is the §4 identification matrix.

---

## §2 — R0.1: the pinned base matrix, and the complete list of knobs that may vary

Every run in this spec is executed by the committed `spec355-width.sh`, whose rate-, shape- and
duration-determining literals are **the `spec349c2-plateau.sh` literals verbatim**:

```
duration 3600 (→ per-cell literal)   churn-clients 6        keyspace 200
or-churn true                        or-keyspace 48         or-every 5
write-interval-ms 20                 writes-per-life 200    offline-keys 3
confirm-interval 2                   crash-interval 0       steady-interval 300
quiesce 3                            mem-sample-interval 5  wal-fsync batched
memory gate NEUTRALIZED (1000000 / 1000000 / 1000000)       sample-interval 60
```

plus the same **env-discipline block** (variables actively `unset`, not merely absent, because the
harness spawns the child server without `env_clear()` and the child therefore inherits the
operator's shell).

The pinned matrix **also includes the three non-rate flags `plateau.sh` passes unconditionally** —
`--json-output`, `--progress-output`, `--mechanism-report` (`spec349c2-plateau.sh:379-381`). These
are enumerated rather than left to "verbatim" because `mechanism_report` **defaults `false`**:
without the flag there is no `mechanism.json`, and then §6's mechanism artifact and the
gauge-independent redb corpus scan — the second instrument this whole classification rests on —
silently do not exist.

**The only knobs `spec355-width.sh` may vary:**

| Knob | Why it may vary |
|---|---|
| `TOPGUN_EPOCH_WIDTH` | the axis under test |
| `--duration` | per-cell literal, recorded in `matrix.txt` |
| the CSV cadence | only where the duration makes the 60 s cadence degenerate (the 360 s / 420 s control cells; see §2.1) |
| `--no-ack` / `--inject-slow-leak` | §9's R4.3 control runs |
| `--server-port`, `--data-dir`, the output paths | run isolation |
| `SOAK_SERVER_BINARY` | **provenance arm only** (§4 cells C/D/E). `plateau.sh:160` actively unsets it. Setting it is a loud, echoed deviation, and on that path it is **fail-closed** — see §4.3(b). |

Every varied knob is echoed into that run's committed `matrix.txt`.

### §2.1 — The one cadence departure, stated in advance

`negctl` (360 s) and `leakctl` (420 s) run the shell sampler at a **20 s** cadence rather than 60 s.
At 60 s those runs would yield 6 and 7 data rows, i.e. a three-point last-half fit — a committed
series with no usable shape. Nothing else moves with it: the cadence belongs to the **shell**
sampler, while the gate's own verdict is computed by the harness's **in-process** sampler, which
this does not touch. Both cells are non-override (non-smoke) runs, so their artifacts belong in the
tracked evidence directory (AC1).

---

## §3 — R0.2: the confound ledger

### §3.1 — Against SPEC-345's width-100 positive control (−1707.5 B/h)

SPEC-345's positive control was **not** run at the 349c2 matrix. What the archive actually records
is two knobs: **duration 900 s** and **`TOPGUN_EPOCH_WIDTH=100`**
(`.specflow/archive/SPEC-345.md:597`, `:630`). Everything else has to be reconstructed from the
harness's `Config::default()` **at `68d0d255`**, and that reconstruction rests on a premise that is
itself **unrecoverable**: whether that run passed any flags beyond `--duration`.

> **UNRECOVERABLE CONFOUND (the root one).** The flag set of SPEC-345's positive-control invocation
> is not recorded in the archive, the spec, or any committed artifact. The column below is
> therefore *the harness's defaults at that commit*, i.e. the best available reconstruction under
> the assumption that the run passed only `--duration` — **not** a record of what ran. Every row
> marked ✗ below is a difference **if that assumption holds**, and an unknown otherwise.

| Knob | SPEC-345 control (reconstructed: defaults @ `68d0d255`) | This spec's cells | Same? |
|---|---|---|---|
| `duration` | 900 s *(recorded)* | 1800 s (cells B/C/D), 3600 s (cell A) | ✗ |
| `TOPGUN_EPOCH_WIDTH` | 100 *(recorded)* | 100 (cell B) | ✓ |
| `churn_clients` | **16** | **6** | ✗ |
| `or_keyspace` | **32** | **48** | ✗ |
| `crash_interval` | **`Some(120 s)` — `kill -9` every 2 min** | **0 → `None`, no crashes** | ✗ |
| `steady_interval` | **30 s** | **300 s** | ✗ |
| `wal_fsync` | **`per_op`** | **`batched`** | ✗ |
| `mem_ceiling_mb` | 1800 | neutralized (1 000 000) | ✗ |
| `keyspace` | 200 | 200 | ✓ |
| `write_interval` | 20 ms | 20 ms | ✓ |
| `writes_per_life` | 200 | 200 | ✓ |
| `offline_keys` | 3 | 3 | ✓ |
| `quiesce` | 3 s | 3 s | ✓ |
| `or_churn` / `or_every` | true / 5 | true / 5 | ✓ |
| `mem_sample_interval` | 5 s | 5 s | ✓ |
| `confirm_interval` | 2 s | 2 s | ✓ |

**Consequence, and it is the reason this ledger exists.** Seven knobs differ under the stated
assumption, two of them heavily load-bearing for tombstone residency (`crash_interval` — a
crash-every-2-min run repeatedly restarts the server mid-ramp; `churn_clients` 16 vs 6 — a ~2.7×
difference in OR churn rate). **Cell B therefore does NOT attempt to reproduce −1707.5 B/h
numerically, and no determination in this document depends on it doing so.** What cell B reproduces
is the **direction and bound-compliance** of that control — which is precisely what §4's decision
table bands on (`S100 ≤ 512 B/h`, i.e. the gate PASSES), and never on numeric equality.

*(Rationale for using the 349c2 matrix rather than reconstructing SPEC-345's: it yields a **paired**
comparison against the two committed width-1000 arms in which width is the only difference. That is
a strictly stronger discriminator than approximating a number from a matrix that is only partially
recoverable — see the spec's Assumption 2.)*

### §3.2 — Confounds internal to this spec's own matrix

| Confound | Cells it sits between | Why it is material |
|---|---|---|
| **Duration** — 1800 s (cells B, C) vs **3600 s** (cell A, the committed 349c2 arms) | A ↔ B, A ↔ C | The whole branch-(2) hypothesis is that the measured series is **ramp-dominated**. For a ramp-dominated series the fitted last-half slope is a function of *where the window sits on the ramp*, so 1800 s and 3600 s runs are not interchangeable. **Cell C is therefore read as an order-of-magnitude binary ("reproduces / does not"), never as a numeric comparison against A.** |
| **Binary provenance** — HEAD harness + HEAD server (A, B) vs HEAD harness + pre-family server (C) | A ↔ C | Cell C deliberately holds the *instrument* fixed and varies only the *server*, so the gauge, the CSV column, the fit and the assessment are byte-identically the same code in all three cells. What it does **not** control is any server-side change outside the prune path that could move tombstone residency; §9's R5 bisect is what narrows that if cell C fires. |
| **Cell A was measured at `d6922f08`, not at this spec's base SHA** | A | Addressed by §3.3's re-attestation. It is not assumed away. |

### §3.3 — R0.2a: the cell-A re-attestation, executed

`spec349c2-manifest.md:41`/`:53` states its enumeration as `git diff --name-only d6922f08..HEAD --
'*.rs'`, and `..HEAD` is not `..<spec-base>` once this document is committed. The command actually
run, with this spec's base SHA substituted:

```bash
git diff --name-only d6922f08..bd41ccf5 -- '*.rs'
```

**`<spec-base> = bd41ccf5c11ce9ff168e34f76a2d58ee3ddf6eb8`** (repo HEAD at spec start; the merge of
PR #131, `feat/sf-349c2-plateau-proof`).

**Output — exactly four paths:**

```
packages/server-rust/benches/soak_harness/main.rs
packages/server-rust/benches/soak_harness/report.rs
packages/server-rust/src/storage/datastores/write_behind.rs
packages/server-rust/tests/soak_wal_census.rs
```

### §3.4 — R0.2b: "emission-only", defined mechanically, and the per-path adjudication

The term decides a 3600 s re-run, so it is not left to the judgment of the person who would have to
do the re-run. A path is **emission-only iff BOTH hold**:

1. **It is one of the four paths already adjudicated** in `spec349c2-manifest.md` §1 (`:46-51`):
   `benches/soak_harness/main.rs`, `benches/soak_harness/report.rs`,
   `src/storage/datastores/write_behind.rs`, `tests/soak_wal_census.rs`; **and**
2. **its hunks in the interval add no read** by (a) a sampler, (b) an assessment, or (c) the write
   path — i.e. no added or modified call that *consumes* a measured quantity: the tombstone gauge
   scrape, `assess_*`, the CSV/report population of a gated field, or a server-side write/prune.
   Hunks that only *emit* (add a field to a report struct, print a line, widen a log, add a test)
   satisfy this; a hunk that changes what a gate or a sampler reads does not.

**Any path outside those four is NOT emission-only, full stop.** No case-by-case adjudication is
available for it, and cell A **is re-run** (PRE-CHANGE build, 3600 s) with this document naming the
path that forced it. The asymmetry is deliberate: the escape hatch is closed on the side the
executor's incentive points, which is the same pressure §9's AC8/AC8b exist to resist.

| # | Path | Clause (1) | Clause (2) — adjudication | Emission-only? |
|---|---|---|---|---|
| 1 | `benches/soak_harness/main.rs` | ✓ adjudicated at `spec349c2-manifest.md:46-51` | *(to be adjudicated hunk-by-hunk in §10 before cell B runs)* | pending |
| 2 | `benches/soak_harness/report.rs` | ✓ adjudicated | *(as above)* | pending |
| 3 | `src/storage/datastores/write_behind.rs` | ✓ adjudicated | *(as above)* | pending |
| 4 | `tests/soak_wal_census.rs` | ✓ adjudicated | *(as above)* | pending |

**Clause (1) discharges now:** the returned set is *exactly* the four adjudicated paths, with no
fifth path, so the "any other path ⇒ re-run cell A" trigger does **not** fire. Clause (2) is
adjudicated hunk-by-hunk in §10 (wave 2), before cell B's clock starts; a failure there re-runs
cell A at 3600 s on the PRE-CHANGE build.

---

## §4 — R0.3: the 2×2 identification matrix and its decision table (PRE-REGISTERED)

### §4.1 — Why two axes and not one

A **single-axis** predicate on the width-100 slope alone **cannot identify the fork**. A
**width-dependent** regression — which is exactly the shape of the SPEC-349c1a/349c1b OR-delta line
named as first suspect — produces a compliant width-100 slope *and* a breaching width-1000 slope.
On that one axis it is **observationally identical** to "width-scaled prune math". Under a
single-axis rule it would be classified as branch (2), and §8/§9 would then re-derive a bound
**over a live regression**. The anti-tautology controls do not catch that: they prove the
re-derived gate still reds a gross leak, not that its bound is not accommodating a regression.

Identification therefore needs **two** axes:
`{epoch width 100, 1000} × {HEAD binary, pre-family binary}`.

### §4.2 — The pre-family binary, and the pin

Built by the **SPEC-354 provenance pattern**: `git worktree add` a detached checkout at a pinned
SHA, `cargo build --release --bin topgun-server` there (into the worktree's **own** target dir), and
drive it from the **HEAD** harness via `SOAK_SERVER_BINARY` (`process.rs:418-426`).

Holding the instrument at HEAD is the whole point: the gauge scrape, the CSV column,
`spec349c2-fit.awk` and `assess_tombstone_bytes` are then byte-identically the same code in every
cell, so a difference **between** cells is a difference in the **server**. This is a deliberate
**half-swap**, and this document says so rather than describing cell C as "a run at the old commit".

**The pin, by procedure:** the last merge before the first SPEC-349-family merge that touches the
OR/prune path, and at or after the SPEC-345 measurement of 2026-07-13 so the gauge under
measurement is the same instrument.

**Resolved:** `181723d0` (2026-07-27) — the last merge before `3fe5a2c0`
(`sf-349a-or-apply-seam`, 2026-07-27), which is the first SPEC-349-family merge touching the
OR/prune path.

**The pin's date is 2026-07-27, not 2026-07-13, and this matters.** Every conclusion drawn from
cell C is scoped to **2026-07-27** — see row 1 and the E-row. The pin also sits **after**
`69a5fd1f` (sf-351 tombstone-gauge isolation, 2026-07-19), so the pre-family binary already carries
the SPEC-351 scoped-sink gauge that `TG-OR-004` covers: the one pre-family-owned instrument in the
half-swap is the gauge, and it is the same gauge.

The resolved SHA, the command that resolved it, and the worktree path are recorded in cell C's
`matrix.txt`.

### §4.3 — Pre-flight on the provenance path: IDENTITY checks, not compatibility checks

Two checks an earlier draft carried — a `PORT=` readiness line and a non-`None`
`topgun_ormap_tombstone_bytes` scrape — are **deliberately absent**. Both pass *identically on the
HEAD binary*, so neither can detect the operator who believes they swapped the server and did not.
They are replaced by two checks that answer *which binary actually ran*.

**(a) Identity witness = the WAL frame-kind census.** The pin **precedes the OR-delta emitter**
(merged `7142d4dc`, `sf-349c1b-or-delta-emitter`). A pre-emitter server therefore cannot write a
single OR delta frame, so cell C's committed `mechanism.json` **MUST show `orDeltaFrames == 0`**
(with `orSnapshotFrames > 0`, i.e. the OR path was actually exercised). The contrast anchor is
committed and unambiguous: the HEAD-binary emitter-on arm shows `orDeltaFrames: 157134`
(`spec349c2-emitter-on.mechanism.json:11`). The census is free — it is already in §6's required
artifact set — and it is written by the **HEAD** decoder (`report.rs:375-469`) reading
**pre-family-written** frames, so it is an observation of the server, not of the instrument.

> **Any nonzero `orDeltaFrames` in cell C means the swap was botched, and the cell is therefore
> INVALID: abort the run, fix the swap, re-run. It is NOT routed into a row of the decision table.**
>
> This distinction is the whole point of the check. A botched swap makes cell C a second cell A
> (`Spre ≈ S1000` → "reproduces" → row 1 → branch (2)), and §8 would then re-derive a bound over the
> very regression the matrix believed it had ruled out. A botched swap must never silently become
> evidence — an **INVALID** cell has no determination, and INVALID is not a band of the table.

`spec355-width.sh` reads the census itself at the end of every provenance run and exits `9`
(`INSTRUMENT DEFECT`) on a nonzero `orDeltaFrames`, so the operator learns it at the end of the run
that produced it rather than from a later `jq`.

**(b) Fail-closed binary resolution on the provenance path.** `resolve_server_binary()`
(`process.rs:421-426`) **fails OPEN**: with `SOAK_SERVER_BINARY` absent from the bench process's own
environment it returns the bench's compile-time default server path with **no existence check and no
warning** (`:422-425`). A variable exported in the wrong subshell, or a worktree whose `cargo build`
landed in the shared `target/`, is exactly how cell C silently becomes cell A.

So on the provenance path `spec355-width.sh` **refuses to run** unless `SOAK_SERVER_BINARY` is set
and names an existing executable file — a hard non-zero exit (`3`) **before the clock starts**, never
a warning, and with **no fallback whatsoever** on that path. The guard lives in the **shell runner**,
not in `process.rs`: it is a non-`.rs` edit, so the Rust ceiling is untouched (still 2 of 5) and **no
`.rs` edit is introduced into a measurement lineage** (§7). The Rust resolver therefore still fails
open at the end of this spec; the provenance path never reaches it unguarded.

The resolved `SOAK_SERVER_BINARY` value and both checks' outcomes are recorded in the run's
`matrix.txt`.

A pre-family binary that genuinely cannot produce the run — e.g. it never emits the readiness line
and the harness times out — is a **recorded finding that forces a later pin**, never a silently
degraded run. The runner says so explicitly at that failure point.

### §4.4 — The cells

| Cell | Config | Status | Duration | Role |
|---|---|---|---|---|
| **A** | HEAD binary @ width 1000 | **ALREADY MEASURED — no new run** | 3600 s | The two committed SPEC-349c2 arms (`spec349c2-emitter-{on,off}.*`, commit `d6922f08`): 248,148.9 and 283,066.2 B/h. Reference `S1000` = **248,148.9 B/h** (the ON arm — the shipped default configuration). Subject to §3.3/§3.4's re-attestation. |
| **B** | HEAD binary @ width 100 | **ADD** | 1800 s | Gross-regression check: does SPEC-345's width-100 PASS still hold at HEAD at all? (SPEC-345 allots ≥15 min; 1800 s is used so B and C are duration-matched and B's last-half window is ~900 s, well clear of the 120 s `DEFAULT_TOMBSTONE_BYTES_MIN_WINDOW_SECS` floor at `monitor.rs:365`.) |
| **C** | **pre-family** binary @ width 1000 | **ADD** | 1800 s | **THE regression discriminator.** At a 485× effect, 1800 s is ample to see whether the breach reproduces at the same order; read as a **binary** ("reproduces / does not"), not as a numeric comparison against A — see §3.2's duration confound. |
| **D** | pre-family binary @ width 100 | **NOT REQUIRED** | — | It tests nothing A+B+C leave open: it would only re-assert that the pre-family binary also cleared the bound at width 100, which SPEC-345's committed −1707.5 B/h control already asserts (at SPEC-345's own matrix — §3.1). **It becomes REQUIRED** as the first extra run of §5's tie-break, if C's result is ambiguous. Recording it as "not required, and here is why" rather than omitting it is the point. |
| **E** | **pre-346** binary @ width 1000 | **CONDITIONAL — not run in the default path** | 1800 s | The 2026-07-13 → 2026-07-27 gap probe. See §4.6: fires **only if** row 1 lands **and** §8's R4.1a magnitude reconciliation cannot account for the observed magnitude. |

### §4.5 — The decision table (PRE-REGISTERED)

Let `S100` be cell B's fitted last-half slope, `Spre` cell C's, and `S1000 = 248,148.9 B/h`.

The three bands are **disjoint and exhaustive by construction**:

- **"reproduces"** = `Spre ≥ 0.1 × S1000` (= 24,814.89 B/h)
- **"does not reproduce"** = `Spre ≤ 512 B/h` (the gate's own bound — the gate PASSES)
- **ambiguous** = `512 B/h < Spre < 0.1 × S1000`, which is row 5

`S100` is banded identically against the same two thresholds.

**Rows 1–5 are the complete, mutually exclusive partition of the `(S100, Spre)` product** — nine of
nine band combinations, no overlap — **and the E-row does not disturb that.** The E-row is a
**conditional escalation hanging off row 1**, entered only *after* row 1 has been taken and only
when a second, later condition holds. It is not a sixth outcome of the matrix and it competes with
no row: the matrix's exhaustiveness is a property of rows 1–5 alone, and remains so.

| # | Cell B (`S100`) | Cell C (`Spre`) | Determination | Routes to |
|---|---|---|---|---|
| 1 | `≤ 512 B/h` (bound-compliant, incl. negative) | **reproduces** | **BRANCH (2) — width-scaled prune math.** The breach pre-dates the **pinned SHA `181723d0`**, whose date is **2026-07-27**; SPEC-345's second disjunct was an unverified extrapolation. **The claim stops at the pin's own date of 2026-07-27.** It says nothing whatever about the 2026-07-13 → 2026-07-27 interval — that interval is the E-row's and cell E's, never row 1's. | §7→§8 (or §9's R5b) |
| 2 | `≤ 512 B/h` | **does NOT reproduce** | **BRANCH (1) — WIDTH-DEPENDENT REGRESSION.** Precisely the case a single-axis predicate misclassifies as (2). The breach is new *and* only visible at production width. | §9's R5 |
| 3 | `> 512 B/h` and `≥ 0.1 × S1000` | **reproduces** | **BRANCH (2), with its premise broken at BOTH widths.** Not a regression, but width 100 no longer clears the bound either, so §8's ramp premise must be tested against the width-100 series too, and R5b is the live outcome if no window plateaus. | §7 (both widths) → §8 **or** §9's R5b |
| 4 | `> 512 B/h` and `≥ 0.1 × S1000` | **does NOT reproduce** | **BRANCH (1) — WIDTH-INDEPENDENT REGRESSION.** The strongest branch-(1) signal: the breach is new and shows at both widths. | §9's R5 |
| 5 | `512 < S100 < 0.1 × S1000`, **or** cell C ambiguous (`512 < Spre < 0.1 × S1000`) | — | **INDETERMINATE.** No branch is taken on this matrix alone. | §5 |
| **E** | *(not a band — see §4.6)* | *(not a band)* | **CONDITIONAL ESCALATION OFF ROW 1.** Entered only when **both** conjuncts hold: **(i) row 1 has been taken**, **and (ii) §8's R4.1a prune-math re-derivation cannot account for the observed magnitude.** Probes the 2026-07-13 → 2026-07-27 interval that row 1's conclusion deliberately does not cover. Not an outcome of the `(S100, Spre)` matrix and not exclusive with rows 1–5. | §4.6 → §9's R5 if it fires |

**No row of this table ends in "record a note".** Rows 1 and 3 can land on §9's R5b, which is a full
disposition with its own owner, spin-off and catalog flip.

### §4.6 — Cell E: the conditional 2026-07-13 → 2026-07-27 gap probe

**Why the gap exists.** Row 1's honest claim stops at the pin (`181723d0`, 2026-07-27). The interval
between SPEC-345's measurement (`68d0d255`, 2026-07-13) and that pin contains **seven merges, two of
them on the OR path**: `6c35785a` (sf-346, ormap WAL, 2026-07-14) and `2769570f` (sf-347, ormap RSS
**in-place mutate**, 2026-07-16), plus sf-350/351/352/353/354. A regression introduced by SPEC-346
or SPEC-347 **satisfies row 1's antecedent** (cell C reproduces), would be classified as
width-scaled prune math, and — because row 1 routes to no bisect — nothing downstream would narrow
it. Cell E is what makes that interval measurable rather than assumed.

**Configuration.** A **pre-346** binary (the last merge before `6c35785a`) at **width 1000**,
duration 1800 s, built and driven by the identical §4.2 half-swap procedure as cell C, including
**both** of §4.3's identity checks — with one substitution stated below.

**Instrumentation: corpus-scan-only, deliberately.** Cell E's read-out is the CSV `tombstone_bytes`
column plus `mechanism.json`'s **redb corpus scan**, and **not** the in-process `tombstones` gauge
object. This is safe and it is not a weakening:

- the CSV column is written by the **shell runner**, which scrapes the metric itself — it is
  **binary-independent by construction**;
- the redb corpus scan is a **HEAD** reader (`main.rs:1225-1238`) over the pre-family server's own
  file, and does not depend on the pre-family server's gauge at all.

That matters because a pre-346 pin sits **before** `69a5fd1f` (sf-351 tombstone-gauge isolation,
2026-07-19), so — unlike cell C — cell E's server does **not** carry the SPEC-351 scoped-sink gauge
that `TG-OR-004` covers. Cell E is therefore read **gauge-free**: it never quotes a slope from that
binary's gauge, and its identity check (a) uses the WAL frame-kind census exactly as cell C does
(`orDeltaFrames == 0`; nonzero ⇒ **INVALID**, abort and re-run), which is likewise gauge-free.

**Firing condition — both conjuncts required.** Cell E runs **only if**:

1. §6's determination is **row 1**, **and**
2. §8's R4.1a chosen prune-math re-derivation **cannot account for the observed magnitude** — i.e.
   §7's width-scaling model predicts a width-1000 residency/slope materially below what cell A
   measured, so "width-scaled prune math" does not arithmetically reach 248,148.9 B/h.

If either conjunct fails, cell E is **NOT-APPLICABLE**, recorded with the number that ruled it out
(the decision-table row, or §7's predicted-vs-observed ratio pair).

**Cost: none in the default path.** Cell E adds no unconditional run.

**Disposition if it fires.** If cell E's breach **does not reproduce** at the pre-346 pin, the
regression is localized to the 2026-07-13 → 2026-07-27 interval with sf-346 / sf-347 as the named
live candidates, the determination is **re-routed to branch (1)**, and §9's R5 applies in full. If
it **reproduces**, row 1's claim widens back to the 2026-07-13 measurement as a **measured**
statement, and branch (2) proceeds unchanged.

---

## §5 — R0.4: the tie-break (PRE-REGISTERED)

On INDETERMINATE (row 5), run, in this order:

1. **cell D** (pre-family @ width 100, ≥1800 s) — completing the 2×2, which resolves the ambiguity
   whenever it is cell C that is ambiguous; then, if still ambiguous,
2. **a width sweep at a single fixed duration** — widths 100 / 300 / 1000 on the **HEAD** binary,
   duration 1800 s each (`sweep100`, `sweep300`, `sweep1000`) — tested against §7's pre-registered
   width-scaling prediction. If the three slopes and equilibria are consistent with that prediction,
   the determination is **(2)**; if not, it is **(1)**.

Every tie-break run is committed like any other, with the full §6 artifact set.

---

## §6 — R0.5: the evidence protocol, as the harness emits it AT HEAD

*Deliberately **not** "inherited verbatim from SPEC-349c2": that spec's committed CSVs carry a
**5-column** header `elapsed_secs,rss_mb,wal_mb,redb_mb,disk_total_mb` with **no**
`tombstone_bytes`, and its committed `soak.json` carry `walFsync` and `epochWidth` but **no**
`tombstones` / `disk` / `confirmApply` objects — those landed after those runs
(`spec349c2-manifest.md` §7.3, "from this run forward"). Nothing here implies 349c2's tombstone
slope is CSV-re-derivable; it is not, and §7.3 of that manifest is where its provenance lives.*

For **every** run in this spec:

- the **CSV** is committed and carries the `tombstone_bytes` column, **populated** — the runner's
  post-run population check is a hard failure of the run's admissibility as evidence;
- **`soak.json`** is committed and carries `tombstones`, `disk`, `confirmApply`, `walFsync`,
  `epochWidth`, `crashes`;
- the per-checkpoint confirm-apply series is committed in **`progress.jsonl`**;
- **`mechanism.json`** (the gauge-independent redb corpus scan + the WAL frame-kind census) is
  committed — which requires `--mechanism-report` per §2;
- **`harness-console.log`** is committed **in the evidence directory**, never left under `target/`
  (the SPEC-349c2 §7.3 lesson). `spec355-width.sh` copies it into its committed home at the end of
  every run, including the early-failure paths;
- the run's **`matrix.txt`** records the repo HEAD SHA, the dirty-tree flag, `uname -a`, both binary
  build times, every varied knob, and — for the provenance arm — the pinned pre-family SHA, the
  command that resolved it, and the worktree path;
- the slope quoted here is **cross-reproduced** by `spec349c2-fit.awk` over the committed CSV and
  must agree with the harness's own `tombstones.slopeBytesPerHour` to within the documented
  instrument difference (different sampler origins and cadences). **A disagreement larger than that
  is a finding to record, not to average away** — the CSV column and the in-process gauge object are
  two independent instruments over the same gauge, and a divergence there is the one failure that
  would silently move a verdict.

### §6.1 — R0.5a: the schema-check table (the durable fix for a recurring defect class)

Naming a serialized key the harness does not emit has occurred **three times** in the 349/355 spec
family, and it is load-bearing every time. Standing rule for this spec and this document:

> **Any AC, requirement or checklist item that names a serialized key MUST have that key verified
> against the `#[serde(...)]` derive at its source, and MUST cite the `file:line` of that derive,
> before the item is considered written.** A key quoted from another document, from a sibling
> struct, or from the Rust field name is **not** verified.

Two traps this rule exists to catch, both verified in `report.rs` at HEAD:

- **The derive is per-struct, not per-file.** `MemoryReport` (`report.rs:24-25`) carries **no**
  `rename_all`, so its keys are **snake_case** (`memory.slope_mb_per_hour`), while `TombstoneReport`,
  `DiskReport`, `ConfirmApplyReport`, `SoakReport` and `ProgressSnapshot` all carry
  `rename_all = "camelCase"`. Assuming one convention across the file produces a wrong key in either
  direction.
- **Nesting differs between artifacts.** The same three confirm-apply counters are nested under
  `confirmApply` in `soak.json` and **flat** in `progress.jsonl`.

**Every serialized key this document and the spec name, verified at `<spec-base>`:**

| Key (as serialized) | Owning struct | `rename_all` derive | Rust field | Artifact |
|---|---|---|---|---|
| `tombstones.slopeBytesPerHour` | `TombstoneReport` | `report.rs:43` camelCase | `slope_bytes_per_hour` `report.rs:49` | `soak.json` |
| `tombstones.firstBytes` | `TombstoneReport` | `report.rs:43` camelCase | `first_bytes` `report.rs:46` | `soak.json` |
| `tombstones.peakBytes` | `TombstoneReport` | `report.rs:43` camelCase | `peak_bytes` `report.rs:47` | `soak.json` |
| `tombstones.lastBytes` | `TombstoneReport` | `report.rs:43` camelCase | `last_bytes` `report.rs:48` | `soak.json` |
| `tombstones.passed` | `TombstoneReport` | `report.rs:43` camelCase | `passed` | `soak.json` |
| `disk`, `confirmApply`, `walFsync`, `epochWidth`, `crashes`, `passed`, `finishedReason` | `SoakReport` | `report.rs:90` camelCase | `disk`, `confirm_apply`, `wal_fsync` `report.rs:104`, `epoch_width` `report.rs:112`, `crashes`, `passed`, `finished_reason` | `soak.json` |
| `confirmApply.{confirms,lastConfirmedEpoch,confirmErrors}` | `ConfirmApplyReport` | `report.rs:81` camelCase | `report.rs:83-85` | `soak.json` **only** |
| `confirms`, `lastConfirmedEpoch`, `confirmErrors` — **FLAT, not nested** | `ProgressSnapshot` | `report.rs:141` camelCase | `report.rs:163-165` | `progress.jsonl` |
| `memory.slope_mb_per_hour` — **snake_case** | `MemoryReport` | `report.rs:24-25` — **NO rename** | `slope_mb_per_hour` | `soak.json` |
| `orDeltaFrames`, `orSnapshotFrames` | `MechanismReport` | `main.rs:1314` camelCase | `or_delta_frames` `main.rs:1334`, `or_snapshot_frames` `main.rs:1338` | `mechanism.json` |

> **There is no `confirm_apply` object in `progress.jsonl`.** `jq -e '.confirm_apply'` over a
> `progress.jsonl` line is expected to exit **non-zero**, and that is the check, not a defect.

---

## §7 — R0.6 build/measurement identity, and R3.2's pre-registered plateau/scaling test

### §7.1 — The three build lineages

The binary that produced a slope is the binary on disk at the recorded SHA. **Any `.rs` edit
invalidates every slope taken before it.** Runs are partitioned into three named lineages and each
run states which one it belongs to. **No slope may be carried across a boundary.**

| Lineage | What it is | Runs |
|---|---|---|
| **PRE-CHANGE** | one pinned pre-change build at this spec's base SHA (`bd41ccf5`) | cells B and D, §5's sweep, §7.2's long run |
| **PROVENANCE** | the **HEAD harness bench binary from the PRE-CHANGE build**, driving a `topgun-server` built in a detached worktree at the pinned pre-family SHA | cells C and E only |
| **POST-CHANGE** | one pinned post-change build, after §8's R4.2 edits | §8's four revalidation runs |

Cell A belongs to none of them: it is SPEC-349c2's committed evidence at `d6922f08`, carried as
input under §3.3/§3.4's re-attestation obligation.

The PROVENANCE lineage is deliberately a **half-swap**: only the server binary is old.

### §7.2 — R3.1: the long run (branch (2) only)

One run at `TOPGUN_EPOCH_WIDTH` **unset** (production default 1000), duration **14,400 s (4 h)**,
full §6 artifact set, cell id `long`, artifacts `spec355-w1000.*`.

**Rationale for 4 h:** SPEC-345's own note records ~1000 OR_REMOVEs ≈ one epoch ≈ ~66 s at width
1000, and SPEC-349c2's 60-min run reached epoch 113 — so 4 h is the shortest run that puts a
*last-quarter* window several hundred epochs past any plausible ramp while remaining a run that can
actually be repeated.

### §7.3 — R3.2: the plateau and width-scaling tests (PRE-REGISTERED — neither may be introduced after seeing the data)

**(a) Windowed decay.** Partition the run into 8 consecutive equal windows `W1..W8`; fit each with
`spec349c2-fit.awk`; report all 8 slopes with standard errors.

*Mechanism, pinned, because the instrument cannot do this directly.* `spec349c2-fit.awk` accepts
`-v window=` with **`last_half` (default) or `full` only** — it rejects anything else
(`spec349c2-fit.awk:20-21`, `:69-71`), and the instrument **is not forked** (Assumption 7). So the 8
fits are produced by **slicing the committed CSV into 8 header-bearing segments** — each segment
file is the CSV's header row followed by that window's data rows — and fitting each segment with
`-v col=tombstone_bytes -v window=full`. The slicing is done by a committed one-liner recorded in
§10 (not retyped per window), the 8 segment files are committed beside the CSV as
`spec355-w1000-seg<N>.csv`, and every segment must carry ≥2 valued rows or the fit dies by design
(`spec349c2-fit.awk:120`).

**Plateau predicate.** The series is declared to have reached plateau at `Wi` **iff** the slopes of
`Wi..W8` are all within the bound under test, **or** their monotone decay fits an asymptote at or
below it.

**If NO window plateaus**, that is a legitimate, reportable outcome, and it means branch (2)'s
premise ("the 60-min window measured a ramp") is **refuted at width 1000**. It is recorded and
**escalated to §9's R5b**, the branch-(2)-UNBOUNDED disposition — **not** forced into a bound.
(R5b, not R5: R5 is branch-(1)-only, so routing this outcome there would leave the most consequential
plausible finding with no owner and no AC.)

**(b) Width scaling.** The mechanism hypothesis under test is that equilibrium tombstone residency
≈ *(retained epochs) × (epoch width) × (mean bytes per tombstone)*, i.e. residency should scale
~linearly in width and the ramp needed to reach it should lengthen ~linearly in width.

Tested against cell B's width-100 equilibrium: **predict `peakBytes`/`lastBytes` at width 1000 ≈ 10×
the width-100 figure, and the ramp duration ≈ 10× longer.** The predicted and observed ratios are
both recorded. **A prediction that fails is evidence *against* the width explanation and is reported
as such**, not smoothed over.

---

## §8 — Branch (2): the re-derivation, PRE-REGISTERED shapes and anti-tautology gates

**R4.1 — exactly one of three candidate shapes is selected**, with the §7 data that supports it
cited, and with the two rejected shapes explicitly disposed of:

- **(i) A width-scaled slope bound** — the bound becomes a function of `epoch_width` (available at
  the call site via `effective_epoch_width()`, already imported at `main.rs:103`).
- **(ii) Bound unchanged, admissibility guard made epoch-relative** — the fit window must span at
  least *K* epochs (and/or start after a width-derived ramp) before the slope clause may hard-gate,
  replacing/augmenting the fixed 120 s `DEFAULT_TOMBSTONE_BYTES_MIN_WINDOW_SECS`.
- **(iii) A residency-ceiling clause** — a bound on absolute resident tombstone bytes derived from
  §7.3(b)'s model, augmenting the slope clause.

This document must state **why the selected shape is what the data supports**, and **must not present
"it makes the gate green" as a reason**. If the data supports *no* shape (§7.3(a) found no plateau),
R4 is recorded **not-derivable** and **§9's R5b applies in full**, including its spin-off, its named
owner and its catalog flip.

*Stated prior, not a conclusion (spec Assumption 8):* shape **(ii)** is the most likely, since at a
true plateau the slope should be ~0 at any width and what scales with width is the *ramp length*.
Shape (i) or (iii) winning is an equally valid outcome, and the choice is made from §7's data.

**R4.1a — the magnitude reconciliation, and the cell-E trigger.** Before a shape is implemented,
this document must state **arithmetically** whether the chosen prune math accounts for cell A's
measured **248,148.9 B/h** at width 1000: take §7.3(b)'s width-scaling model, feed it cell B's
width-100 equilibrium, and record the predicted width-1000 slope/residency **beside** the observed
one. Two outcomes, both recordable:

- **It accounts for the magnitude** (predicted and observed agree within the stated
  order-of-magnitude tolerance) → **cell E is NOT-APPLICABLE**, recorded with this ratio pair as the
  measurement that ruled it out, and R4 proceeds.
- **It does not** → **run cell E** (§4.6) **before R4.2 lands a single line.** A shape implemented
  over an unexplained magnitude gap is a bound fitted to a residual.

This is the **only** trigger for cell E, and it is conjunctive with row 1 having been taken.

**R4.2 — implement it** in `monitor.rs` (bound/guard + its doc-rationale, which must **name the
width the bound was measured at**, plus the calibration tests in that file's own
`#[cfg(test)] mod tests`, which the change necessarily reaches) and `main.rs` (call site + breach
message carrying the effective bound and its width). No provenance markers in code (`SPEC-NNN` /
`TODO-NNN` forbidden); a `TG-<DOMAIN>-<NNN>` invariant ID or a tracker pointer inside a
known-false/deferred doc-contract are the only sanctioned citations.

**R4.5 — unit-level anti-tautology: the calibration tests stay MUTATION-CONTROLLED.** The ~10
calibration call sites of `assess_tombstone_bytes` inside `monitor.rs`'s test module are what make
the gate's own arithmetic checkable, and the cheapest way to keep them green under a loosened bound
is to loosen the tests. **"Green" is therefore not the bar:**

- The synthetic **leak** series must be shown to **still go RED under the OLD bound/guard** — i.e.
  re-running the *post-change* test bodies against the *pre-change* constants must fail. A green test
  is not evidence until its RED is demonstrated.
- Any test whose **series** (not merely its expected constant) is changed must have that change
  justified from §7's data here. **Widening a synthetic leak's magnitude so it still trips a looser
  bound is a rewrite of the discriminator, not a re-calibration, and is rejected.**
- Recorded per changed test: the old constant, the new constant, the mutation run, and its RED output.

**R4.3 — revalidation, on a single new pinned POST-CHANGE build.** All four runs, full §6 artifact
set each, recorded in one table:

| Run | Cell id | Config | Required verdict |
|---|---|---|---|
| Negative control | `negctl` | `--no-ack`, 360 s | tombstone gate **FAILS** |
| Slow-leak control | `leakctl` | `--inject-slow-leak`, 420 s | tombstone gate **FAILS** |
| Positive control, width 100 | `posctl` | cell B's matrix, 1800 s | tombstone gate **PASSES** |
| Production width | `prodctl` | §7.2's matrix, 3600 s | verdict recorded, whatever it is |

**A re-derived bound under which either control run passes is rejected outright** — that is the
SPEC-342h false-green class, and SPEC-345 exists precisely because a gate that cannot fail is worse
than no gate.

**R4.4 — cross-vendor check.** `/xask` on the chosen re-derivation *before* implementing it (is this
the right shape, or a bound fitted to make a red go away?) and `/xreview` on the resulting diff. Both
outputs committed into this directory, each finding marked applied or refuted-with-reason.

**The cross-vendor gate is not branch-(2)-only.** Naming a culprit commit and a mechanism (R5) and
declaring tombstone bytes unbounded at the production width (R5b) are decision points of the same
consequence — each redirects a spun-off spec and a pre-soak blocker. **Every branch of this spec
passes through the cross-vendor gate before its verdict is handed off.**

---

## §9 — The two non-(2) dispositions, pre-registered so neither can degrade into a note

### §9.1 — R5, branch (1): bisect, name, hand off

- **The bisect interval is exactly one interval: `68d0d255` (the SPEC-345 measurement, 2026-07-13)
  .. HEAD.** Why the wider one and not `(pin, HEAD]`: cell C only establishes the breach's absence at
  `181723d0` (2026-07-27), so the narrower reading leaves the seven-merge 2026-07-13 → 2026-07-27
  interval — including `6c35785a` (sf-346) and `2769570f` (sf-347) — outside the search, and R5 is
  reachable *through* that gap via cell E. A bisect scoped narrowly could be handed a culprit that
  provably is not in its own search space.
- **The anchors are free and seed the search; they do not redefine it.** Cell C (and cell E, if it
  ran) are committed measurements at known SHAs inside the interval: each that did **not** reproduce
  is a `git bisect good` seed, each that did is a `git bisect bad` seed. The interval and the seeds
  are recorded **separately** so the two are not conflated.
- **First suspects:** the SPEC-349c1a/349c1b OR-delta line; `6c35785a` (sf-346) and `2769570f`
  (sf-347) if cell E fired; anything touching `packages/server-rust/src/tombstone_frontier_impl.rs`
  or the epoch prune path in `packages/server-rust/src/service/domain/crdt.rs`.
- **The probe** is cell B's width-100 protocol at a **reduced but pre-registered** duration — stated
  here *before* the bisect starts, with the reason it is still post-ramp at width 100 — run at each
  bisect point on a fresh data dir. Every probe's slope + SHA goes in a bisect series table; the full
  §6 artifact set is committed for the two **boundary** commits (last good / first bad).
- Under row 2 the culprit is expected to be **width-dependent**, so every probe runs at width 100
  **and** the boundary commits are additionally probed at **width 1000** — a width-dependent culprit
  is invisible to a width-100-only probe, which is the same identification failure §4.1 exists to
  remove.
- **`/xask` on the bisect verdict, before the fix spec is spun off**, stating the boundary commits,
  both boundary artifact sets and the proposed mechanism, and asking whether the series actually
  *isolates* that commit or merely correlates with it. Committed, findings applied or
  refuted-with-reason.
- **Name the culprit commit and the mechanism, and spin off a fix spec with a NAMED OWNER.** This
  spec does **not** fix the regression — fixing it here would invalidate the measurements it exists
  to produce.
- Record the gate's disposition: the bound stays as-is, the gate stays hard, and TODO-586 / TODO-484
  remain blocked on the spun-off fix spec rather than on this one.

### §9.2 — R5b, branch (2)-UNBOUNDED: tombstone bytes are unbounded at the production width

*Applies when branch (2) is determined **and** §7.3(a)'s plateau test finds no plateau (or §8's R4.1
finds the data supports no shape). NOT-APPLICABLE otherwise, recorded with the windowed-fit series
that ruled it out.*

This is the most consequential plausible outcome of this spec and it **must not end in a note here**.
**Both** of the following are mandatory.

**R5b.1 — spin off a prune fix-or-redesign spec, at the production width, with a named owner and
pre-soak-blocker status.**

- `/sf:plan` a new TODO whose subject is **prune fix or redesign at `epoch_width = 1000`** — *not* a
  bound re-derivation, because §7.3(a) has just shown there is no bound to derive.
- **Named owner:** the TODO-566 / SPEC-345 tombstone-GC line — the same owner TODO-630 carries, so
  the handoff invents nobody new.
- Slotted as **TODO-630 → \<the new spec\> → TODO-586 → TODO-484**, with TODO-630 **re-pointed at
  it rather than closed**.
- **`/xask` before the spin-off:** is "unbounded at production width" the honest reading of the
  8-window series, or is the series still ramping over a 4 h horizon?

**R5b.2 — flip the catalog immediately, and move the NAKED/ratchet accounting with it.**

*Verified finding this is written around:* **`INVARIANTS.md` at `<spec-base>` contains no row
asserting that tombstone bytes are bounded.** The nearest row, `TG-OR-004` (`INVARIANTS.md:448`), is
an **instrument-fidelity** invariant — "the tombstone-bytes gauge tracks the REAL add and prune
paths, test-isolatable", status *decided, enforced* — whose violation consequence is explicitly
"*the SPEC-345 tombstone hard gate reads a fiction*". That is a claim about the **gauge**, not about
**boundedness**. **`TG-OR-004` MUST NOT be flipped**: it is not the claim this finding falsifies, and
flipping it would misreport a working instrument as broken.

The honest flip is therefore an **addition**:

- Add **`TG-OR-005`** (next free ordinal) stating the property the finding refutes: *resident OR
  tombstone bytes are bounded under sustained churn at the production epoch width*.
- **Status:** `open (TODO-630)`, plus a one-line statement of the measured refutation and a pointer
  to this file.
- **The row sites the `TG-OR-004` distinction in its own body**, because this spec is archivable and
  the catalog row is not: *"Distinct from `TG-OR-004`, which is gauge **fidelity** (does the counter
  track the real add/prune paths), not **boundedness** (do the bytes stay bounded). A red tombstone
  gate is not evidence against `TG-OR-004`; do not flip it."*
- **Enforcing test: `NAKED`, UNCONDITIONALLY** — not "if the row lands NAKED". R5b fires only when
  the property has been **refuted by measurement**, and a refuted property cannot have a passing
  enforcing test, so the marker is **entailed**, not contingent. It is also the only form the gate
  accepts: the non-NAKED branch (`scripts/check-invariants.sh:56-78`) demands a greppable `fn <name>`
  under `packages/server-rust/{src,benches}` or an existing `*.rs` filename, neither of which a
  citation of this manifest can satisfy.
- **The literal form, pinned to the window the script actually reads.**
  `scripts/check-invariants.sh:38` builds the enforcing block as `grep -A3 '\*\*Enforcing test:\*\*'`
  — the field line **plus three following lines** — and `:45-51` requires the literal `NAKED` **and**
  a `(TODO|SPEC)-[0-9]+` match **within that same window**. `Status: open (TODO-630)` sits several
  fields below and is **outside** the window, so a row carrying `NAKED` on the enforcing line while
  relying on `Status` for the tracking reference **fails CI** at the exact moment R5b requires the
  flip to land. The field is therefore written with **both tokens inside the window**:

  ```markdown
  - **Enforcing test:** `NAKED — no test proves resident OR tombstone bytes are bounded at the
    production epoch width; refuted by measurement (TODO-630)`.
  ```

  If the wording changes, the invariant to preserve is the **window**, not the sentence.
- **The ratchet moves in the same commit.** `NAKED_BASELINE` is `3` at
  `scripts/check-invariants.sh:20` and the check is an **exact-match ratchet in both directions**
  (`:85-94`) — a grown count fails, and an un-lowered baseline after a closure also fails. Because
  the new row is NAKED unconditionally, **`NAKED_BASELINE=4` is required in the same commit**, and
  `scripts/check-invariants.sh` must exit 0.
- Both edits are non-`.rs` and consume no part of the Rust ceiling.

**Timing is part of the requirement:** the flip lands **the moment branch (2)-UNBOUNDED is
confirmed — in the same commit as the determination** — not at `/sf:done`. A catalog that still reads
as though the property holds, while a committed 4 h measurement says it does not, is the
false-invariant hazard the catalog exists to prevent.

---

## §10 — EXECUTED RECORD

*§10.0 below **is** part of the pre-registration commit — it records the wave-1 gate, which by
construction runs before wave 2 and therefore cannot be written later. **§10.1 onward did not exist
at the pre-registration commit** and are added as their runs land. The distinction is stated rather
than left to the reader because a manifest that misdescribes its own commit boundary is the
documentary defect SPEC-349c2's Review v2 blocked on.*

### §10.0 — Wave-1 gate (G1), executed at the pre-registration commit

**No measurement run is in this section.** Everything here is either a static check or an
explicitly-labelled smoke.

**(1) The four-assertion micro-check — all PASS.**

| # | Assertion | Result |
|---|---|---|
| 1 | Fail-closed resolver present on the provenance path; no compile-time-default fallback | **PASS** — `spec355-width.sh:168-186` refuses; the compile-time default's macro name has **zero** occurrences in the runner; `env -u SOAK_SERVER_BINARY bash spec355-width.sh cellC` exits **3** before spawning anything |
| 2 | Census identity assertion pre-registered: `orDeltaFrames == 0`, and nonzero ⇒ **INVALID** (abort/re-run), never a decision-table row | **PASS** — §4.3(a) |
| 3 | Row 1 scopes its claim to the pin's actual date, 2026-07-27 | **PASS** — §4.5 row 1; the over-claiming phrasing appears nowhere in this file |
| 4 | E-row present with both firing conjuncts and its "not a sixth band" statement | **PASS** — §4.5 E-row, §4.6 |

**(2) The runner was SMOKED, not merely written** (AC1's "executes, not transcribes").

- **Refusal into the tracked directory:** `SPEC355_SMOKE_DURATION=90 spec355-width.sh cellB`
  refuses with exit 2 while `SPEC355_OUT_DIR` is unset, i.e. while the artifacts would land here.
- **HEAD path**, 120 s override into a scratch dir: instrument sound, exit 0; `soak.json`
  `epochWidth: 100` — which is the live proof that the width axis actually reaches the child server;
  all five size columns populated; `tombstone_bytes` populated.
- **PROVENANCE path**, 120 s override into a scratch dir, `SOAK_SERVER_BINARY` = the `181723d0`
  worktree build: instrument sound, exit 0; `epochWidth: 1000` (unset → production default); and the
  **identity witness fired correctly on a real pre-family run — `orDeltaFrames: 0`,
  `orSnapshotFrames: 2367`**.

> **These two smokes are NOT evidence and are cited nowhere else in this document.** They are
> instrument validation, and they are recorded here for one reason: the third bullet establishes
> *before* 1800 s of machine time is spent that the pinned binary reaches readiness, that the
> half-swap actually swaps, and that the census discriminates. A pre-family binary that could not
> reach readiness would have been a §4.3 recorded finding forcing a later pin — and finding that at
> minute 30 of cell C rather than at second 120 is the avoidable cost this smoke buys out.
>
> The same weak-observation discipline §1.1 applies to the 90 s smoke applies here: **no slope from
> either smoke is quoted, compared, or used in any determination.**

**(3) The pin and its build.**

| Item | Value |
|---|---|
| Pinned pre-family SHA | `181723d0` (2026-07-27) |
| Resolved by | last merge before `3fe5a2c0` (`sf-349a-or-apply-seam`), the first SPEC-349-family merge touching the OR/prune path — see §4.2 |
| Worktree | `/tmp/spec355-pin-181723d0` (detached), built with its **own** `CARGO_TARGET_DIR` so the build could not land in the shared `target/` and quietly overwrite the HEAD binary — the precise accident §4.3(b) exists to catch |
| PRE-CHANGE build | `<spec-base>` = `bd41ccf5`, `cargo build --release --bin topgun-server --bench soak_harness` |

### §10.1 — Cell A re-attestation, clause (2) adjudication

**Adjudicated before cell B's clock started.** Clause (1) already discharged in §3.3: the
enumeration returns *exactly* the four adjudicated paths and no fifth, so the "any other path ⇒
re-run cell A" trigger does not fire. What remains is clause (2), hunk by hunk.

| # | Path | Change in `d6922f08..bd41ccf5` | Adds a read by a sampler / an assessment / the write path? | Emission-only? |
|---|---|---|---|---|
| 1 | `benches/soak_harness/report.rs` | +69/−0. Three **new** report structs (`TombstoneReport`, `DiskReport`, `ConfirmApplyReport`), the three fields that hang them off `SoakReport`, and the three confirm-apply fields on `ProgressSnapshot` — plus their doc-comments. Pure serialization surface. | **No.** Nothing here samples, assesses, or writes; every added item is a `Serialize` target. | **YES** |
| 2 | `benches/soak_harness/main.rs` | +32/−31. The `use` line; populating `ProgressSnapshot`'s three confirm-apply fields from **pre-existing** atomics; populating the three new report structs from the **already-computed** `tombstones` / `disk` assessment values; and a `print_summary` refactor that takes `&report` instead of loose args. | **No** — see the ruling below. | **YES** |
| 3 | `src/storage/datastores/write_behind.rs` | +8/−0, **all eight lines are `///`**: the TODO-628 caller-obligation pointer on `flush_key`'s doc-contract. No executable line whatsoever. | **No.** A doc-comment is not compiled into the item's semantics. | **YES** |
| 4 | `tests/soak_wal_census.rs` | +110/−1. One added `#[test]` fn, distinct-valued fixture fields on the existing `sample_report` helper, and a widened `use`. Test-only; no production item. | **No.** "add a test" is named in clause (2) as an emitting change. | **YES** |

**The one ruling that needed making, stated rather than glossed.** Clause (2)'s enumeration lists
"the CSV/report population of a gated field" among the *consuming* reads, while its very next
sentence lists "add a field to a report struct" among the *emitting* changes. Rows 1 and 2 sit
between those two phrases, so a literal reading can be pointed either way, and under the stricter
one cell A would be re-run at 3600 s. **The governing text is clause (2)'s closing sentence — "a
hunk that changes what a gate or a sampler reads does not [satisfy this]" — because that is the
sentence that names the hazard the clause exists to catch.** These hunks do not change what any gate
or sampler reads; they change only how an already-computed verdict is *serialized*.

**And it is verified mechanically, not argued:**

- `benches/soak_harness/monitor.rs` — which owns `DEFAULT_TOMBSTONE_BYTES_THRESHOLD_PER_HOUR`,
  `DEFAULT_TOMBSTONE_BYTES_MIN_WINDOW_SECS` and `assess_tombstone_bytes` — is **not in the interval
  at all**: `git diff --name-only d6922f08..bd41ccf5 -- .../monitor.rs` is **empty**.
- No hunk in `main.rs` touches the gauge scrape, the tombstone sampler, or the
  `assess_tombstone_bytes` **call site**: a `[-+]` grep over that file's diff for
  `assess_tombstone_bytes|scrape_tombstone|parse_tombstone|TombstoneSample|tombstone_samples`
  returns **no hit**.
- The server-side prune path is untouched: `src/tombstone_frontier_impl.rs` and
  `src/service/domain/crdt.rs` are **both empty** in the interval.

So the quantity cell A reports and the quantity cells B and C will report are computed by the **same
arithmetic over the same sampler**; only its serialization changed. That is precisely the property
the re-attestation exists to establish.

**Determination: all four paths are emission-only. Cell A STANDS — it is not re-run**, and its
`S1000 = 248,148.9 B/h` (the ON arm) is carried into §4.5's decision table as pre-registered.

*(Ledger note, not a caveat on the above: cell A's own `soak.json` carries no `tombstones` object —
that struct is one of the additions adjudicated in row 1 — so its slope's provenance is
`spec349c2-manifest.md` §7.3, exactly as §6 of this document states. The re-attestation is about
whether the arithmetic drifted, and it did not.)*

### §10.2 — The completed identification matrix

| Cell | Config | Lineage | Duration | Harness `tombstones.slopeBytesPerHour` | CSV `last_half` fit | Gate |
|---|---|---|---|---|---|---|
| **A** | HEAD server @ width 1000 | SPEC-349c2 input @ `d6922f08` | 3600 s | **248,148.9 B/h** (ON arm — `S1000`) / 283,066.2 B/h (OFF arm) | n/a — 349c2 CSVs carry no `tombstone_bytes` column (§6) | **FAIL** |
| **B** | HEAD server @ width 100 | PRE-CHANGE (`bd41ccf5`) | 1800 s | 1,524.96 B/h (360 samples, window 896 s) | **7,938.86 B/h** = `S100` (15 pts, span 840 s, se 14,268, r² 0.023) | **FAIL** |
| **C** | **pre-family** server (`181723d0`) @ width 1000 | PROVENANCE | 1800 s | 175,367.59 B/h (360 samples, window 896 s) | **176,954.34 B/h** = `Spre` (15 pts, span 840 s, se 25,261, r² 0.791) | **FAIL** |
| **D** | pre-family server @ width 100 | — | — | **NOT RUN at this point** — see §10.3's routing | | |
| **E** | pre-346 server @ width 1000 | — | — | **NOT-APPLICABLE** — see §10.3 | | |

`S100` and `Spre` are the **CSV `last_half` fits**, as pre-registered (§4.5 "cell B's fitted
last-half slope"; the Validation Checklist reproduces them from the committed CSVs). The harness's
in-process figures are the cross-reproduction required by §6 and are stated beside them.

#### §10.2.1 — Cell C's identity checks (AC3b)

| Check | Outcome |
|---|---|
| (a) Census identity witness | **PASS** — `spec355-cellC.mechanism.json`: `orDeltaFrames = 0`, `orSnapshotFrames = 41509`. The pin precedes the OR-delta emitter (`7142d4dc`), so zero is what a genuine pre-family server must produce, and the nonzero OR-snapshot count proves the OR path *was* exercised. Contrast anchor, committed and unchanged: `spec349c2-emitter-on.mechanism.json` → `orDeltaFrames: 157134`. The census is written by the **HEAD** decoder reading **pre-family-written** frames, so it observes the server, not the instrument. |
| (b) Fail-closed resolution | **PASS** — recorded in `spec355-cellC.matrix.txt`: `SOAK_SERVER_BINARY` set to `/tmp/spec355-pin-181723d0/target/release/topgun-server`, with **no** fallback to the bench's compile-time default on that path (an unset variable is a hard exit 3 before the clock starts). |
| Pinned SHA / resolver / worktree | `181723d0`; "last merge before `3fe5a2c0` (`sf-349a-or-apply-seam`), the first SPEC-349-family merge touching the OR/prune path"; `/tmp/spec355-pin-181723d0` — all three in `matrix.txt`. |

**Only the server binary is pre-family.** The harness, the gauge scrape, the CSV column,
`spec349c2-fit.awk` and `assess_tombstone_bytes` are all at HEAD, byte-identically the same code as
in cells A and B. This is the deliberate half-swap of §4.2, not "a run at the old commit".

#### §10.2.2 — The instrument-agreement finding (§6 requires this be recorded, not averaged away)

| Cell | Harness | CSV fit | Ratio | CSV r² |
|---|---|---|---|---|
| B | 1,524.96 B/h | 7,938.86 B/h | **5.2×** | 0.023 |
| C | 175,367.59 B/h | 176,954.34 B/h | **1.009×** | 0.791 |

The two instruments **converge to 0.9 % when the signal is strong and diverge 5.2× when it is not**,
and that is the whole explanation. Cell B's CSV standard error (14,268 B/h) is **9.4× its own point
estimate** and comfortably contains the harness's 1,524.96 B/h, so the two are **statistically
compatible rather than contradictory**: at r² = 0.023 a 15-point fit is resolving no trend at all.
The difference is **statistical power** — the harness samples 360 times against the CSV's 30, giving
180 last-half points against 15 — not a divergence between two readings of the same gauge.

**It does not move a verdict, and that is checkable rather than asserted:** both of cell B's readings
fall in the same decision-table band (`512 < S < 24,814.89`), and both of cell C's fall in
"reproduces" (`≥ 24,814.89`). Every determination below is invariant to which instrument is used.

#### §10.2.3 — Corroboration: the epoch clock scales with width, measured

`lastConfirmedEpoch` at the end of two runs of **identical duration and matrix**, differing only in
width: **cell B (width 100) = 600**, **cell C (width 1000) = 59**. A ratio of **10.2×**, against a
width ratio of 10×. The epoch clock scales with epoch width as §7.3(b)'s model assumes — measured
here rather than asserted. Both runs also confirm the low-water mark advanced throughout
(`confirms = 865`, `confirmErrors = 25` ≈ 2.9 % in both), so neither breach is the stalled-cursor
cause that a climbing gauge can otherwise have.

### §10.3 — The determination

> ## DETERMINATION: **ROW 5 — INDETERMINATE.** No branch is taken on this matrix.
>
> Routed to **§5's tie-break (R0.4)**.

**The two numbers that produced it**, banded against the pre-registered thresholds
(`512 B/h` and `0.1 × S1000 = 24,814.89 B/h`):

| Quantity | Value | Band |
|---|---|---|
| `S100` (cell B) | **7,938.86 B/h** | `512 < S100 < 24,814.89` → **ambiguous middle** |
| `Spre` (cell C) | **176,954.34 B/h** | `≥ 24,814.89` → **reproduces** |

Row 5's first disjunct — `512 < S100 < 0.1 × S1000` — is satisfied, so the determination is
INDETERMINATE.

**Why no other row is reachable, and why this is not a near miss.** Rows 1 and 2 both require
`S100 ≤ 512 B/h`; cell B fails the gate, so both are excluded. Rows 3 and 4 both require
`S100 ≥ 0.1 × S1000`; `S100` is **3.1× below** that threshold, so both are excluded. **`S100` alone
therefore forces row 5, and no value of `Spre` could have changed it.** Cell C was still run — R1.2
makes it mandatory, and its value is what seeds the tie-break and (under §9.1) a bisect — but the
honest statement is that it could not have moved the row, and this document says so rather than
presenting a five-cell matrix as though every cell were load-bearing for the determination.

**This outcome is invariant to the instrument choice** (§10.2.2): under the harness's figures
`S100 = 1,524.96` is in the same ambiguous band and `Spre = 175,367.59` is still "reproduces".

#### §10.3.1 — What the matrix DID establish, stated separately from the determination

These are measurements, not a branch call. Keeping them apart from the determination is the point.

1. **SPEC-345's width-100 PASS does not still hold at HEAD on this matrix.** The control that
   measured −1707.5 B/h now runs **positive** and breaches the bound ~3× (harness) to ~15.5× (CSV).
   **This is not by itself evidence of a regression**, and §3.1 is why: cell B runs the 349c2 matrix,
   while SPEC-345's control ran a matrix whose flag set is recorded nowhere, and under the defaults
   reconstruction **seven knobs differ** — `crash_interval` (120 s vs none) and `churn_clients`
   (16 vs 6) being the two most load-bearing for tombstone residency. Attributing the sign flip to a
   code change rather than to those seven knobs is exactly the inference the ledger exists to block.
2. **The breach at the production width is NOT new since the pin.** Cell C reproduces it at
   `181723d0` at the same order as cell A (`Spre / S1000 = 0.71`). **Scoped to 2026-07-27**, the pin's
   own date. It says nothing about the 2026-07-13 → 2026-07-27 interval — that interval is cell E's.
3. **Width matters, and by much more than the bound's slack.** At the same matrix and duration,
   width 1000 runs **22×** (CSV) to **115×** (harness) hotter than width 100. Whether that is
   *width-scaled prune math* or a *width-dependent regression* is precisely what row 5 declines to
   decide on the evidence in hand.

#### §10.3.2 — The un-taken branches, with the measurements that rule them out (AC4)

| Branch | Status at this point | Ruling-out measurement |
|---|---|---|
| Branch (1) — regression | **NOT DETERMINED** (not refuted, not established) | Cell C **reproduces** at `Spre = 176,954.34 B/h`, which is evidence *against* a regression newer than 2026-07-27 — but `S100 = 7,938.86 B/h` sits in the ambiguous band, so the matrix does not carry a branch call. |
| Branch (2) — width-scaled prune math | **NOT DETERMINED** | Same pair. Cell C's reproduction is consistent with (2), but cell B's breach means (2)'s clean form ("width 100 still passes") is *already false*, and the row-3 band that would have carried "(2) with its premise broken at both widths" is not met. |
| Cell D | **REQUIRED** — it is §5's step 1 under row 5 | Was "not required" only while the determination was expected to be rows 1–4. |
| Cell E | **NOT-APPLICABLE** | Its firing condition is conjunctive and its **first** conjunct fails: cell E fires only if **row 1 has been taken**, and row 1 was not taken. The second conjunct (§8's R4.1a magnitude gap) is not reached. Recorded here with the measurement that ruled it out: `S100 = 7,938.86 B/h`, which excludes row 1 by itself. |

#### §10.3.3 — Routing

Per §5 (R0.4), in this order:

1. **Cell D** — pre-family server @ width 100, 1800 s, PROVENANCE lineage, both identity checks.
   §5 motivates it as the resolver "whenever it is cell C that is ambiguous"; here it is **cell B**
   that is ambiguous, and cell D is still the correct next probe — and for a sharper reason than the
   one §5 anticipated. Cell B's breach at width 100 is the single fact that forced row 5, and cell D
   is the *only* run in the design that says whether that breach is **new since the pin**: it holds
   the width at 100 and varies only the server. That is a direct test of §10.3.1's finding 1 against
   the code, with the seven-knob matrix confound held fixed by construction, since cell B and cell D
   share this spec's matrix exactly.
2. If still ambiguous — the **width sweep** (`sweep100` / `sweep300` / `sweep1000`, HEAD, 1800 s
   each) against §7.3(b)'s pre-registered width-scaling prediction.

*(§4.5's E-row remains unentered: row 1 was not taken, so its first conjunct is false.)*

### §10.4 — R0.4's tie-break, and the branch determination

#### §10.4.0 — Correction to a claim already committed

Cell D's commit argued from `peakBytes` near-identity (28,138 vs 27,448, 2.5 % apart) that both
binaries reach the same equilibrium at width 100. **That argument is withdrawn.** A leak of exactly
the magnitude cell B reported — 1,524.96 B/h over 0.5 h = **762 B** — *predicts* a peak gap of that
size, and the observed gap is **690 B**. So `peakBytes` near-identity is not evidence for the
artifact reading. It is also the statistic that flatters that reading, while `lastBytes` (24,134 vs
18,986, a **5,148 B** gap ≈ 27 % of the level) points the other way. Selecting the first and not
reporting the second was selection bias in the presentation. Both travel together from here, and
the conclusion is now carried by the **last-half mean** (§10.4.2), not by an order statistic.

*(Surfaced by `spec355-xask-row5.md`, finding A.)*

#### §10.4.1 — Cell D and the width-100 repeat

| Run | Binary | Width | Harness slope | CSV `last_half` | Gate |
|---|---|---|---|---|---|
| cell B | HEAD | 100 | +1,524.96 | +7,938.86 | FAIL |
| **`sweep100`** | **HEAD (repeat of cell B)** | 100 | **+7,048.07** | **−7,330.71** | FAIL |
| cell D | pre-family | 100 | −791.28 | −16,797.00 | **PASS** |
| `sweep300` | HEAD | 300 | −7,106.77 | −16,330.29 | **PASS** |
| `sweep1000` | HEAD | 1000 | +87,413.90 | +48,547.71 | FAIL |
| **`sweep1000b`** | **HEAD (repeat)** | 1000 | **+42,684.14** | — | FAIL |
| **`cellC2`** | **pre-family (repeat of cell C)** | 1000 | **+208,554.75** | — | FAIL |

**The slope estimator does not survive replication.** `sweep100` is a bit-identical repeat of cell B
— same binary, width, matrix, duration and lineage — and the harness slope moves **4.6×** while the
CSV estimator **flips sign**. At width 1000 the same repeat moves the slope **2.0×**. And the gate's
verdict is **non-monotonic in width**: FAIL(100), PASS(300), FAIL(1000), an ordering no model of
tombstone residency predicts.

**Every width-100 slope ever measured, across both binaries and both specs**, spans **8,756 B/h**:

```
SPEC-345 −1707.5 │ cellD −791.3 │ cellB +1525.0 │ sweep100 +7048.1        bound: 512 B/h
```

a spread **17× the bound being tested against**. **SPEC-345's −1707.5 B/h "positive control" sits
inside that noise**, so the width-100 PASS it recorded never demonstrated bound-compliance. That is
a *stronger* statement than this spec's own premise, which assumed the width-100 PASS was solid and
only the width-1000 extrapolation unverified.

By contrast the **epoch clock behaves exactly as modelled** — `lastConfirmedEpoch` over identical
durations: **600 / 588** (w100), **191** (w300), **58 / 59** (w1000), giving ratios **3.14** and
**3.24** against a predicted 3.0 and 3.33. It is the *slope statistic over an 840 s window* that
carries no signal, not the epoch machinery.

#### §10.4.2 — The level statistic, and the deciding comparison

Last-half **mean** of the committed `tombstone_bytes` column (computed over the committed CSVs; the
instrument is not forked):

| Width | HEAD | pre-family |
|---|---|---|
| 100 | 20,765 · 18,961 (mean **19,863**) | **20,771** |
| 300 | 20,981 | — |
| 1000 | 38,715 · 36,624 (mean **37,670**) | 56,898 · 54,676 (mean **55,787**) |

**Run-to-run spread of the level is 5.4 % (w1000) and 9.0 % (w100)** — against a slope that moved
2.0×, 4.6× and changed sign. Level is the statistic that survives replication.

**At width 100 the two binaries are indistinguishable:** HEAD 19,863 vs pre-family 20,771 — **4.6 %
apart**, inside the HEAD run-to-run spread. Taken with the slope comparison (HEAD {+1525, +7048} vs
pre-family −791: difference of means 5,077.8, SE 4,783.2, **t = 1.06**), **the width-100 regression
reading is refuted on both the noisy and the stable statistic.**

**At width 1000, n = 2 vs n = 2 — the deciding comparison:**

| Statistic | HEAD | pre-family | Difference | t (df 2) |
|---|---|---|---|---|
| Level | 37,670 (sd 1,479) | 55,787 (sd 1,571) | pre-family **+48 %** | **11.88** |
| Slope | 65,049 (sd 31,629) | 191,961 (sd 23,467) | pre-family **2.95×** | **4.56** |

**Ranges are disjoint on both measures** — no HEAD run overlaps any pre-family run:
level HEAD [36,624 – 38,715] vs pre-family [54,676 – 56,898]; slope HEAD [42,684 – 87,414] vs
pre-family [175,368 – 208,555].

#### §10.4.3 — The deviation from R0.4 step 2, stated plainly

**The pre-registered rule fired, and its literal verdict is (1).** §7.3(b) predicts residency ∝
width; HEAD measured **1.06×** for a 3× width change and **1.90×** for a 10× change. Not consistent
⇒ the rule says (1), regression, bisect.

**I first argued the rule "had not fired" because two of its three inputs did not exist at 1800 s.
That argument is WITHDRAWN as rationalization**, on the reasoning in `spec355-xask-tiebreak.md`: the
rule's `≥ 1800 s` is a *floor condition*, and rereading it as a *quality gate* exploits a design
defect I wrote myself, to escape a verdict I disliked.

**The gap that is real is a different one.** R0.4 step 2 tests **HEAD only** against the prediction —
and the prediction fails on the **pre-family baseline too**:

| Binary | w1000 / w100 level ratio | predicted |
|---|---|---|
| HEAD | 37,670 / 19,863 = **1.90×** | ~10× |
| **pre-family** | 55,787 / 20,771 = **2.69×** | ~10× |

*Pairing note.* These are `n = 2` means on **both** sides wherever two runs exist (pre-family/width
100 is cell D alone). `spec355-xask-tiebreak.md:63-66` — the committed transcript, deliberately not
retro-edited — carries **1.95× / 2.74×**, computed before `sweep1000b` and `cellC2` landed and
pairing a single width-1000 run against an `n = 2` width-100 mean. The consistent recomputation is
what is quoted here and in §10.5.3; the difference is 2–3 % and no verdict moves (both are ≪ 10×).

§7.3(b)'s model is wrong **universally**, not as a HEAD-specific symptom. So "inconsistent with the
prediction" diagnoses **the model is wrong**, not **a regression landed** — and R0.4 step 2's
dichotomy has no branch for that case. The rule is a classifier with a blind spot, and this data
landed in it.

**The falsification condition was fixed BEFORE the deciding runs** and committed in `e3ce61aa`:

> *If HEAD remains no worse than pre-family across n = 2 vs 2, declare (2) on the ground that the
> rule's dichotomy is unsound for this case. **If HEAD is worse, accept (1) and bisect.***

§10.4.2 resolves it: HEAD is **not** worse. It is substantially **better**, on both statistics, with
disjoint ranges, on data collected **after** the condition was committed.

#### §10.4.4 — DETERMINATION

> ## BRANCH (2) — width-scaled prune math. **NO REGRESSION.**
>
> Routed to **§7's R3 characterization** (the ≥ 4 h width-1000 run and its 8-window plateau test),
> and thence to **§8's R4** or **§9.2's R5b**.

**The evidence, in one place:**

1. **At the production width the breach reproduces on the pre-family binary** (cell C 175,368 B/h,
   cellC2 208,555 B/h) at the same order as cell A's 248,148.9 B/h. The breach **pre-dates the pin**.
2. **HEAD is better than the pre-family baseline at the production width**, decisively (level +48 %,
   t = 11.88; slope 2.95×, t = 4.56; disjoint ranges). The SPEC-349 family's OR-delta work moved
   resident tombstone bytes in the **opposite direction from a regression**.
3. **At width 100 the two binaries are indistinguishable** (level 4.6 % apart; slope t = 1.06).
4. **SPEC-345's second disjunct** — "≥30–60 min at the default width" — **was an unverified
   extrapolation**, as TODO-630's fork (2) proposed. And §10.4.1 adds a finding beyond that fork:
   **the first disjunct's width-100 PASS was itself inside the noise.**

**Scope of the no-regression claim — it stops at the pin's own date, 2026-07-27.** It says nothing
about the 2026-07-13 → 2026-07-27 interval; that interval is cell E's, and cell E's disposition is
§10.4.5.

**Branch (1) is recorded NOT-APPLICABLE**, with the measurements that ruled it out: the width-1000
n = 2 vs n = 2 comparison (HEAD better, disjoint ranges) and the width-100 comparison (t = 1.06,
levels 4.6 % apart). Per §9.1 no bisect is run and no prune-path `.rs` file is modified.

#### §10.4.5 — Cell E's disposition at this point

**PENDING R4.1a**, which §2's R2 explicitly admits as a disposition rather than an omission when
row 1 is in play. Cell E's firing condition is conjunctive, and its status differs from §10.3.2's:

- **Conjunct (i) — "row 1 has been taken":** the determination is branch (2) by the §10.4.3 route
  rather than by row 1 of the table. The *substance* of row 1 (branch (2), no regression, scoped to
  the pin's date) is what landed, so this conjunct is treated as **satisfied in substance** and is
  **not** used to dismiss cell E. Dismissing the 2026-07-13 → 2026-07-27 gap on the technicality
  that the branch arrived via the tie-break rather than via row 1 would be exactly the evasion
  §4.6 exists to prevent.
- **Conjunct (ii) — "R4.1a's prune math cannot account for cell A's magnitude":** not yet
  evaluable. It requires §7.3(b)'s width-scaling model fed with cell B's width-100 equilibrium,
  which requires the ≥ 4 h run.

**Cell E is therefore PENDING R4.1a and MUST reach a determinate disposition before `/sf:done`
(AC3c, R6).** A live complication is already on the record: §10.4.3 shows the width-scaling model
**fails on both binaries**, so R4.1a's arithmetic is likely to find the model *cannot* account for
the magnitude — which would **fire cell E**. That is recorded here in advance so the trigger cannot
later be quietly read as not having fired.

*(Resolved in §10.5.4 — and **not** by a ruling-out measurement: R4.1a is never reached because R4
is not-derivable, so cell E is recorded **NOT RUN, DEFERRED WITH AN OWNER** (`TODO-634`) rather than
NOT-APPLICABLE. The trigger written down here is honoured, not quietly read as un-fired.)*

---

### §10.5 — R3 characterization, and the R5b disposition

#### §10.5.1 — R3.1: the ≥4 h run at the production width

One run, `long` cell, PRE-CHANGE lineage, full §6 artifact set (`spec355-w1000.*`).

| | |
|---|---|
| Duration | **14,401 s** (4 h), `epochWidth` 1000, `walFsync` batched, 0 crashes |
| Gauge samples | 2,878 in-process · 240 committed CSV rows |
| Epochs | `lastConfirmedEpoch` **458**; `confirms` 6,901; `confirmErrors` 205 (**3.0 %**) |
| Series | `firstBytes` 0 → **`peakBytes` 646,306 = `lastBytes` 646,306** |
| Harness verdict | `tombstones.passed: false`, slope 130,353.3 B/h over a 7,195 s last-half window |

**The series ends at its maximum.** The low-water mark advanced throughout, so the epoch-scoped
prune was licensed for the whole run — this is not the stalled-cursor artifact a climbing gauge can
otherwise have (§1's settled inputs).

#### §10.5.2 — R3.2(a): the 8 windowed fits, and the plateau predicate

Produced by the **pre-registered mechanism**: the committed CSV sliced into 8 header-bearing
segments by one committed `awk` one-liner (below), each fitted with the **unforked**
`spec349c2-fit.awk` at `-v col=tombstone_bytes -v window=full`. All 8 segments are committed as
`spec355-w1000-seg{1..8}.csv`.

```awk
awk -F, 'NR==1{h=$0; next} {rows[++n]=$0}
  END{seg=int((n+7)/8);
      for(i=1;i<=8;i++){f=sprintf("spec355-w1000-seg%d.csv",i); print h > f;
        for(j=(i-1)*seg+1; j<=i*seg && j<=n; j++) print rows[j] > f; close(f)}}' spec355-w1000.csv
```

| Window | span (s) | slope B/h | se | r² |
|---|---|---|---|---|
| W1 | 60 – 1,801 | 113,657.12 | 13,941.33 | 0.704 |
| W2 | 1,860 – 3,660 | **244,197.34** | 7,573.42 | 0.973 |
| W3 | 3,720 – 5,520 | 151,075.57 | 9,213.40 | 0.903 |
| W4 | 5,580 – 7,380 | 166,532.06 | 8,971.47 | 0.922 |
| W5 | 7,440 – 9,240 | **118,189.21** | 9,703.11 | 0.836 |
| W6 | 9,300 – 11,100 | 132,688.50 | 10,151.52 | 0.855 |
| W7 | 11,160 – 12,960 | 133,598.47 | 9,651.38 | 0.869 |
| W8 | 13,020 – 14,400 | 155,726.40 | 12,099.61 | 0.883 |

**The plateau predicate fails on BOTH disjuncts:**

- **(i) "the slopes of `Wi..W8` are all within the bound"** — **NO.** The *smallest* window slope is
  113,657 B/h = **222× the 512 B/h bound**; the largest is **477×**. No suffix of the series comes
  near it.
- **(ii) "their monotone decay fits an asymptote at or below it"** — **NO.** The series is not
  monotone (W1 < W2, W3 < W4, W5 < W6 < W7 < W8) and **W8 is 37 % ABOVE W1**. Every window has
  r² between 0.70 and 0.97, so the trend *inside* each window is real and well-resolved — this is
  sustained growth, not oscillation around a level.

> **CORRECTION to an interim observation.** From the coarse 1,200 s samples mid-run I reported that
> growth was decelerating and looked like a ramp approaching an asymptote. **The windowed fit
> refutes that** — the apparent deceleration was sampling noise. Recorded because the interim read
> was stated aloud, and it was wrong.

**Branch (2)'s premise — "the 60-min window measured a ramp" — is REFUTED at the production width.**
Per R3.2(a) this is a legitimate, reportable outcome that **escalates to R5b**.

#### §10.5.3 — R3.2(b): the width-scaling prediction, and what the reclaim fraction shows instead

§7.3(b) pre-registers **two** observations, and **both are recorded here against it**: *(i)*
`peakBytes`/`lastBytes` at width 1000 ≈ **10×** the width-100 figure, and *(ii)* the ramp duration
≈ **10×** longer. Neither limb is re-specified — where a different statistic is used it is reported
*beside* the pre-registered one, not in place of it.

**(i) On the PRE-REGISTERED statistic, like-for-like at 1800 s.** `n = 2` means at each width except
pre-family/width-100, where cell D is the only run:

| Binary | statistic | w100 | w1000 | **observed** | predicted |
|---|---|---|---|---|---|
| HEAD | `peakBytes` | 26,807 | 58,418 | **2.18×** | ~10× |
| HEAD | `lastBytes` | 22,319 | 50,179 | **2.25×** | ~10× |
| pre-family | `peakBytes` | 27,448 | 92,114 | **3.36×** | ~10× |
| pre-family | `lastBytes` | 18,986 | 70,598 | **3.72×** | ~10× |

**The prediction fails on the pre-registered statistic, on both binaries** — so it is not a HEAD
symptom. §10.4.0 withdrew `peakBytes` as the *deciding* statistic because it had been selected while
its counterpart was suppressed; it is reported here **beside** `lastBytes` precisely because §7.3(b)
pre-registered the pair, and quoting one without the other is the defect §10.4.0 named.

The **last-half level mean** (§10.4.2) is the statistic the *determination* is carried on. It is a
presentation change for the deciding comparison, not a re-specification of this test, and it returns
the same verdict:

| Binary | w1000 / w100 **level** (last-half mean, `n = 2` both sides where available) | predicted |
|---|---|---|
| HEAD | 37,670 / 19,863 = **1.90×** | ~10× |
| pre-family | 55,787 / 20,771 = **2.69×** | ~10× |

**(ii) The ramp-duration limb: RIGHT-CENSORED — no observed ratio exists, and the censoring bound
already excludes the prediction.** A ramp duration requires a ramp that *terminates*. At width 100
one does: first crossing of the run's own last-half mean at **60 s / 120 s** (HEAD, `n = 2`) and
**180 s** (pre-family), with the peak occurring inside the run. At width 1000 **no window plateaued
over 4 h** (§10.5.2) and the 4 h series **ends at its maximum** (`peakBytes` = `lastBytes` =
646,306) — the ramp had not terminated when observation stopped. The observation is therefore
right-censored at **14,400 s** and no ratio is formable. Predicted ≈ 10 × (60–180 s) =
**600–1,800 s**; observed **> 14,400 s**, at least **8×** the top of the predicted range. **This limb
fails too, in the OPPOSITE direction from limb (i)**: residency scaled far *less* than predicted
while the ramp ran far *longer*.

**A caveat that travels with limb (i), rather than being omitted.** Because the width-1000 ramp had
not terminated at 1800 s, limb (i) compares a width-100 run that *has* levelled against a width-1000
run that has *not*. Its ratios are **lower bounds** on any equilibrium ratio, so "fails low" must
**not** be read as "residency is sub-linear in width". Two things follow, and neither rescues
§7.3(b)'s model:

- against the same width-100 baseline the **4 h** width-1000 figures stand at **24.1×** (`peakBytes`)
  and **29.0×** (`lastBytes`) — *overshooting* 10× while still climbing. This is a
  **duration-mismatched** comparison (no 4 h width-100 run exists) and is offered only to show the
  miss is not one-directional, never as a test of the model;
- decisively for §10.4.3's *use* of this test: **both binaries are equally unconverged at 1800 s**,
  so the HEAD-vs-pre-family comparison stays like-for-like and the deviation's discriminating-power
  argument is untouched by the censoring.

**No growth-class claim is made.** Two durations cannot fit a model, so "superlinear" is not
written anywhere. What the data *does* support: the **average rate roughly doubled** between the
1800 s and 4 h runs (**75 → 161 KB/h**, decimal KB throughout: 37,670 B / 0.5 h and 646,306 B / 4 h;
ratio 2.14), which is inconsistent with approaching a nearby asymptote.

**The mechanism finding — the prune fires, it falls behind.** Prompted by `/xask` finding 2, and
computed from the already-committed CSVs at zero extra cost:

| Run | added | freed | net | **reclaim** |
|---|---|---|---|---|
| w100 HEAD (cell B) | 72,243 | 69,910 | +2,333 | **96.8 %** |
| w100 HEAD (`sweep100`) | 79,602 | 75,471 | +4,131 | **94.8 %** |
| w100 pre-family (cell D) | 66,876 | 57,979 | +8,897 | **86.7 %** |
| w300 HEAD | 74,958 | 73,539 | +1,419 | **98.1 %** |
| w1000 HEAD (`sweep1000`) | 108,255 | 86,753 | +21,502 | **80.1 %** |
| w1000 HEAD (`sweep1000b`) | 121,524 | 111,994 | +9,530 | **92.2 %** |
| w1000 pre-family (cell C) | 103,449 | 68,686 | +34,763 | **66.4 %** |
| w1000 pre-family (cellC2) | 130,790 | 105,834 | +24,956 | **80.9 %** |
| **w1000 HEAD, 4 h** | **904,435** | **299,349** | **+605,086** | **33.1 %** |

Over the 4 h run the gauge **decrements on 80 of 239 steps (33.5 %)**, freeing 299,349 B, largest
single drop 23,115 B. **The prune is not dead and was never unlicensed** — the hypothesis `/xask`
named as leading is refuted by this spec's own committed data. What is happening is that the
**reclaim fraction degrades with both epoch width and elapsed time**: ≈95–98 % at widths 100/300,
≈80–92 % at width 1000 over 1800 s, **33.1 %** at width 1000 over 4 h.

*Caveat carried with the number:* the shell CSV samples at 60 s, so intra-interval oscillation is
invisible and the gross added/freed columns are **lower bounds**. The **net** column is exact, and
the cadence is identical across every run in the table, so the comparison is like-for-like.

#### §10.5.4 — R4 is NOT-DERIVABLE; cell E's final disposition

**R4 (re-derive the bound) is recorded NOT-DERIVABLE**, per §8's own instruction: *"If the data
supports no shape (§7.3(a)'s plateau test found none), R4 is recorded as not-derivable and R5b
applies in full."* There is no bound to derive from a series that does not bound. Consequently:

- **R4.1's three candidate shapes** — (i) width-scaled bound, (ii) epoch-relative admissibility
  guard, (iii) residency ceiling — are **all disposed of as premature, not chosen**. Each presumes a
  steady state whose existence §10.5.2 failed to establish. Assumption 8's prior (shape (ii)) is
  **not** vindicated and is recorded as unresolved rather than quietly carried.
- **R4.2 lands no code.** The Rust ceiling consumed by this spec is therefore **0 of 5** — neither
  `monitor.rs` nor `main.rs` is modified, and the Delta entries for both are NOT-APPLICABLE.
- **R4.3/R4.5/AC8/AC8b (the revalidation runs and the mutation control) are NOT-APPLICABLE**, with
  §10.5.2's 8-window series as the measurement that ruled them out. There is no re-derived bound to
  revalidate and no calibration constant was touched.
- **Cell E: NOT RUN — DEFERRED WITH A NAMED OWNER (`TODO-634`).** Determinate, no longer PENDING,
  and deliberately **not** dressed up as AC3c's disposition (a).

  **Why (a) is unavailable on the branch actually taken.** AC3c admits exactly two ruling-out
  measurements. The first — *a decision-table row other than 1* — was refused in §10.4.5, correctly:
  row 1 landed in substance and dismissing the gap on the route the branch arrived by is the evasion
  §4.6 exists to prevent. The second — *R4.1a's predicted-vs-observed magnitude pair* — **cannot
  exist**: R4.1a is a step inside R4, and R4 is not-derivable. §10.5.2's windowed-fit series is what
  made R4 not-derivable; it bears on **the bound**, not on whether sf-346 / sf-347 moved tombstone
  residency, so it is **not** offered here as a ruling-out measurement. AC3c's two-way disposition
  set is incomplete for a branch that leaves R4 entirely — the same class of gap as R0.4 step 2's
  (Deviation 1) — so a third, determinate disposition is recorded rather than a nearest fit forced
  into (a).

  **The owner.** The **2026-07-13 → 2026-07-27 interval is NOT probed by this spec** and is carried,
  named, in `TODO-634` as a **diagnostic-on-demand** task. Cell E's protocol is already written and
  runnable as specified in §4.6 (pre-346 pin, width 1000, 1800 s, both identity checks,
  corpus-scan-only), so nothing is lost but the running.

  **Why deferral is the right call.** *(i)* The defect being chased — no plateau at width 1000 —
  is present in **every binary measured here**, HEAD and pre-family alike, so localizing it inside
  an earlier interval cannot change any conclusion this spec records or any gate it leaves in place.
  *(ii)* `TODO-634` **supersedes** the question: it re-runs the width-1000 matrix against a per-epoch
  prune record, and the historical mechanism becomes relevant **only if** that redesign needs it — at
  which point cell E is its first task, not a re-derivation.

  **The argument NOT relied on, withdrawn.** An earlier draft of this bullet reasoned that a
  regression in that interval "would have to have been *improved upon* twice over", because HEAD is
  better than `181723d0`. That is a **non-sequitur for localization** — HEAD < `181723d0` says
  nothing about `181723d0` vs `68d0d255` — and it is withdrawn. §10.4.4's no-regression claim
  continues to stop at the pin's own date, exactly as it always did.

#### §10.5.5 — DETERMINATION: R5b — branch (2)-UNBOUNDED

> ## No plateau at the production epoch width within a 4 h horizon.
> **R5b applies in full.** R4 not-derivable; the disposition is a spun-off spec + a catalog entry,
> **not a paragraph in this file.**

**The claim is horizon-scoped, deliberately.** `/xask` finding 1 is applied: finite observation
cannot prove unboundedness, and writing "REFUTED" would invite a future reader to treat the question
as settled and stop looking for the real bound. So the recorded claim is **"no plateau found within
the measured horizon"**, and the catalog says the same.

**R5b.1 — the spin-off (AC9b.1):**

| | |
|---|---|
| Spec/TODO | **`.specflow/todos/TODO-634.md`** — *fix or redesign the epoch-scoped prune at `TOPGUN_EPOCH_WIDTH=1000`* — **not** a bound re-derivation |
| Named owner | **the TODO-566 / SPEC-345 tombstone-GC line** — the owner TODO-630 already carried; no new one invented |
| Status | **pre-72 h-soak blocker** |
| Sequencing | **TODO-630 (resolved) → TODO-634 → TODO-586 → TODO-484** |
| TODO-630 | **re-pointed at TODO-634, not closed out** — the blocker moves, it does not disappear |
| First task carried into it | the per-epoch prune record (`/xask` finding 5), which separates a *selection*, *scheduling* or *throughput* defect — three different fixes. Not done here: instrumenting the prune path is a `.rs` change, and §7.1's R0.6 forbids one inside a measurement lineage. |

**R5b.2 — the catalog flip (AC9b.3/AC9b.4), landing in the SAME commit as this determination:**

- **`TG-OR-005`** added at **`Status: open (TODO-634)`**, stating the property the measurement did
  not establish, with the 4 h numbers and the reclaim-fraction mechanism in its body.
- **`TG-OR-004` NOT flipped** — it stays `decided, enforced`. It is gauge **fidelity**, not
  **boundedness**, and this measurement *depends* on it holding. The row sites that distinction **in
  its own body**, because this spec is archivable and the catalog row is not.
- **Enforcing test: `NAKED` unconditionally** — a property not established by measurement cannot
  have a passing enforcing test. Both the literal `NAKED` and the literal `TODO-634` sit **inside
  the `grep -A3` window** `scripts/check-invariants.sh:38` reads (verified by running that exact
  extraction). The pre-registered wording ended *"refuted by measurement"*; it now reads *"no
  plateau found in a 4 h measurement"* — the pre-registration explicitly pinned **the window, not
  the sentence**, and the reworded form is the honest one.
- **`NAKED_BASELINE` moved 3 → 4** in the same commit. `bash scripts/check-invariants.sh` prints
  `invariants: 20 entries, 4 NAKED (baseline 4)` and **exits 0**.

> **Checklist defect found while verifying this (worth recording, it is not a row defect).**
> Validation Checklist item 10's own extraction, `awk '/^### TG-OR-005/,/^### TG-(OR-006|[A-Z]+)/'`,
> is **self-terminating**: `TG-([A-Z]+)` matches `TG-OR` in the *start* line, so the range closes on
> the line it opens and the greps return 0. The row is fine — verified with the extraction
> `check-invariants.sh` actually uses, which is the one that gates CI.

**R5b's `/xask` (AC9b.2)** is committed at `spec355-xask-unbounded.md`, run **before** the spin-off
and the flip, with all five findings adjudicated. Two of them changed this section: the horizon-scoped
wording, and the reclaim-fraction check that turned "unbounded" into "the prune falls behind" — a
defect with a mechanism and a testable next step.

---

## §11 — R6: disposition and handoff

### §11.1 — What now unblocks the 72 h soak, and what still does not

| Item | Before SPEC-355 | After SPEC-355 |
|---|---|---|
| **TODO-630** | open — the fork unknown | **RESOLVED.** Fork (2): width-scaled prune math, no regression. **Re-pointed at TODO-634, not closed out** — the blocker moves, it does not disappear. |
| **TODO-634** *(new)* | — | **The pre-soak blocker.** Prune fix-or-redesign at `TOPGUN_EPOCH_WIDTH=1000`. Owner: the TODO-566 / SPEC-345 tombstone-GC line. |
| **TODO-586** | gated on TODO-630 | gated on **TODO-634**; carries a SPEC-355 finding of its own (design the redb cross-check against a *level* estimator, not the current rate detector). |
| **TODO-484** (72 h soak) | blocked | **STILL BLOCKED**, now on TODO-634. It would red on the tombstone clause by construction, exactly as before — the difference is the cause is now named, measured and owned. |
| **SPEC-348** (disk gate) | unblocked | **Unblocked, unaffected** — restated below. |

**Sequencing: TODO-630 (resolved) → TODO-634 → TODO-586 → TODO-484.**

**Nothing this spec produced unblocks the 72 h soak.** It converts an unexplained red into a named,
owned defect with a mechanism — which is what the spec set out to do (§0), not to clear the clause.

### §11.2 — SPEC-348's disk gate: unaffected (a restatement, not a re-derivation)

SPEC-348's disk (WAL + redb) gate derives from `du` over real paths (`spec349c2-manifest.md` §7.2)
and **never reads the tombstone gauge**. Nothing in this spec touches that derivation: no `.rs` file
was modified, no committed SPEC-349c2 artifact was altered, and the tombstone finding is confined to
the gauge-backed clause. **SPEC-348's disk gate remains unblocked either way**, exactly as §0 said
it would be at the outset.

### §11.3 — The 90 s smoke observation

Mentioned in §1.1 only, carrying its **WEAK — NOT EVIDENCE** label, and **used in no determination**.
The two instrument smokes this spec itself ran (§10.0) carry the same label and the same treatment:
no slope from any of the three is quoted, compared or relied on anywhere.

### §11.4 — Findings this spec produced that were NOT in its own brief

Recorded because they outlive the spec and each has a home:

1. **The gate's estimator is unreliable at this workload** — two identical width-100 runs gave
   slopes 4.6× apart, a second instrument flipped sign, the verdict is non-monotonic in width, and
   every width-100 slope ever measured spans 8,756 B/h against a 512 B/h bound. **Consequence:
   SPEC-345's −1707.5 B/h "positive control" sits inside the noise and never demonstrated
   bound-compliance** — i.e. the *first* disjunct was as unverified as the second. Carried into
   TODO-634 (level re-derivation required) and TODO-586 (do not tolerance-tune against a rate
   detector), and sited in `TG-OR-005`'s body.
2. **The prune's reclaim fraction, and its degradation with width and time** — the mechanism that
   makes TODO-634 a specific piece of work rather than "make the gate green".
3. **A defect in this spec's own Validation Checklist item 10** — its `awk` range is
   self-terminating (§10.5.5). Recorded so a future reader does not mistake a correct catalog row
   for a broken one.
