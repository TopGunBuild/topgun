# SPEC-373b (carve 9c, part b) — one slot cell per key: pre-registration manifest

## §1 Pre-registration (frozen at commit M; nothing above the APPEND-ONLY marker changes after M)

### Question and scope
Does sharing one slot cell between the engine, the write-behind queue entry and the staging slot — which removes
the three per-op whole-record copies on the resident OR write path (engine `hashmap.rs:136`, queue
`write_behind.rs:2463`, staging `write_behind.rs:2518` at the pin) — lower the count-alloc allocation rate of the
SPEC-370/371/372/373a OR-churn cell by the share dhat attributes to those copies? The mechanism is proven by
deterministic tests (AC-1 slopes, AC-2a/2b/2d `Arc::ptr_eq` and strong counts, AC-6 clone count); these cells read
the aggregate.

### Code under test
- Pin (before, "b"): `b166719d` (SPEC-373a's merge, #171). Every b-server is built from the clean detached checkout
  `target/spec373b-src-b166719d`.
- Freeze (after, "a"): the literal `SPEC373B_CODE_FREEZE=e85adb1f` of `spec373b-cells.sh` — the last commit on
  the branch that changes a build input. Every a-server is built from the clean detached checkout
  `target/spec373b-src-freeze` at exactly that commit.
- After-cell rule (carried from SPEC-373a rulings v6/v7): if any BUILD INPUT changes after the freeze —
  `packages/server-rust/{src,Cargo.toml,build.rs}`, `packages/core-rust`, the root `Cargo.toml`, `Cargo.lock`, and
  the root `rust-toolchain.toml` (added to the spec's six paths after the cross-vendor review: it selects the
  compiler, so it is a build input) — the a-cells are re-run. ORDER item 4 enforces it before every chain and every
  verdict. The freeze literal must be a hex commit id, never a movable name.

### Cells
`spec373b-cells.sh <cell>` (runner), launched by `spec373b-diag.sh` (de/dl) and `spec373b-chain.sh`
(`SPEC373B_PHASE=ca`: b1 → a1 → b2 → a2, interleaved, ONE host session; `SPEC373B_PHASE=je`: jb → ja). Every cell:
cadence 60 s, crash-interval 0, live-copy census at 300 s, log directive armed, journal default; every other matrix
literal is the parent runners' own (churn-clients 6, keyspace 200, or-churn true, or-keyspace 48, or-every 5,
write-interval-ms 20, writes-per-life 200, offline-keys 3, confirm-interval 2, steady-interval 300, quiesce 3,
mem-sample-interval 5, wal-fsync batched, mem gate neutralised, jitter seed 20260831).

| cell | duration | flavour | server built at | teardown | role |
|---|---|---|---|---|---|
| de | 300 | DH | `b166719d` | SIGTERM (dhat flush) | pre-registration input (dhat, c3e shape) |
| dl | 900 | DH | `b166719d` | SIGTERM | pre-registration input (dhat, c3l shape) |
| b1, b2 | 900 | CA | `b166719d` | SIGKILL | before pair (gating) |
| a1, a2 | 900 | CA | freeze | SIGKILL | after pair (gating) |
| jb | 14400 | JE | `b166719d` | SIGKILL | recorded only |
| ja | 14400 | JE | freeze | SIGKILL | recorded only |

SPEC-373a's a1/a2 (`spec373a-a{1,2}.*`, server `bf40ad79…579c`) are recorded context only, not gating: the before
pair is re-run here, interleaved with the after pair, so a cross-day host offset lands in the spreads the thresholds
see instead of in `I`.

### Pre-registration input: this spec's own diagnostic dhat pair (run, committed with M)
`spec373b-diag.sh`, chain start 2026-09-26T14:50:38Z, end 15:21:06Z (`spec373b-diag.log`), both `RUNNER_EXIT=0`,
`RESULT: instrument sound`, `post_mortem_rows=0`. Builds (`spec373b-diag-builds.txt`):
- DH `sha256=69f0654ab03311ac5731ff7c814e983cff5f76534b981beaa4cdc816ea3ecde8`, built from the clean detached
  checkout `target/spec373b-src-b166719d` (HEAD `b166719d9569…`), `--profile release-with-debug --features dhat-heap`;
  console line 1 of both cells carries this sha256.
- H (harness) `sha256=74317bd3e7076d0a90957eaa727cf28c7db3053d8b9130da08d2648f04914138`, built from this checkout
  while its HEAD moved `0511796a → 56e3c669 → ef72aea1` (INVARIANTS.md and evidence scripts only; the `.rs` tree was
  `6a67d99e`'s throughout, which the runner asserted before each cell).

| cell | load average at start (1/5/15 min) | `totalWrites` | writes/s (÷ nominal 300 / 900 s) | vs spec371 ref | floor (0.8 × ref) | DH/CA ratio (vs 174.0, recorded) | dhat `te` µs |
|---|---|---|---|---|---|---|---|
| de | 2.78 / 5.31 / 5.18 | 20 031 | 66.8 | 0.88 × 76.1 | 60.9 | 0.384 | 310 844 442 |
| dl | 7.78 / 7.21 / 6.13 | 46 303 | 51.4 | 0.88 × 58.3 | 46.6 | 0.295 | 910 571 976 |

**Write-rate STOP: FALSE** (`WRITE_RATE_STOP=FALSE`). Host: macOS, M1 Max; chain-start load 2.97 / 3.11 / 3.56
(WindowServer, Telegram and a terminal app in the background, no build and no other cell running during either cell;
dl's start load includes de's own server and harness, which exited seconds before). Recorded, not gated.

**Program bytes at the diag run vs at M.** The diag ran `spec373b-cells.sh` with the freeze literal `6a67d99e` (the
de/dl matrices say so) and `spec373b-shares.py` at sha256 `b12542c2…5d62`. Before M, after the cross-vendor review,
the runner's freeze literal moved to `e85adb1f` (the one later build-input commit, a test doc comment) and the E
program gained a window-sign guard, a `MULTI_SITE_MB` line and a refusal of `--self` in self-check mode. Neither
change can move a number: the runner's DH path does not read the freeze beyond the `.rs` gate it passed, and the
self-check and base outputs quoted below were re-produced by the M bytes of `spec373b-shares.py` with the verbatim
commands, equal line for line to the diag log's (`E = 0.3315803`, self-check `0.3222957`).

### The E program (`spec373b-shares.py`, committed, sha-bound)
Run under `LC_ALL=C`. **Attribution = chain-contains per copy site:** a dhat program point counts for a site when
ANY frame of its stack ends in `(<site file>:<site line>:<col>)`; the innermost topgun frame of every
`RecordValue::clone()` is the derived `Clone` (`storage/record.rs`, `core-rust/src/types.rs`, `hlc.rs`), never a call
site, so an innermost-frame rule cannot see the copies. Window = late − early profile (dhat `tb`); every share =
bytes / window total; `E` = (engine + queue + staging) / window.
- **STOP-O**: any program point, in either profile of a pair, whose stack contains two or more sites.
- **STOP-D**: a site that matches no program point in either profile, or whose window bytes are ≤ 0.
- **STOP-S**: the self-check misses — over the committed `spec371-c3{e,l}` pair with the `46dcc12a` literals, `E`
  must be **0.322296 ± 0.000005**. It runs in-program BEFORE the base pair is opened.
- Precedence D > O > S. Output: per-site `_SITE`, `_PPS` (early/late), `_MB`, `_SHARE`, `_GROW_MB`;
  `MULTI_SITE_PPS`, `E`, `SHARES_STOP`, `E_FROZEN` (4 decimals; `WITHHELD` under any STOP).

Site literals, read from the code by these commands (outputs at each pin):
```
git show <pin>:packages/server-rust/src/storage/engines/hashmap.rs | grep -n 'record: record.clone(),'
git show <pin>:packages/server-rust/src/storage/datastores/write_behind.rs | grep -n '                value: value.clone(),'
git show <pin>:packages/server-rust/src/storage/datastores/write_behind.rs | grep -n 'self.stage(&smap, &skey, entry_seq, Some(value.clone()));'
```
| pin | engine | queue | staging |
|---|---|---|---|
| `46dcc12a` | `hashmap.rs:107` | `write_behind.rs:2424` | `write_behind.rs:2479` |
| `b166719d` | `hashmap.rs:136` | `write_behind.rs:2463` | `write_behind.rs:2518` |

The engine literal is the Occupied-arm clone of `update_in_place` only; the Vacant-arm clone (`:158` at the pin,
`:123` at `46dcc12a`, the materialize path) is not in `E`, and the known answer 0.322296 is the value without it.

**Self-check** — `LC_ALL=C python3 spec373b-shares.py --pin 46dcc12a spec371-c3e.dhat.json.gz spec371-c3l.dhat.json.gz`
(`spec373b-selfcheck.txt`):
```
  SELF_EARLY=spec371-c3e.dhat.json.gz
  SELF_LATE=spec371-c3l.dhat.json.gz
  SELF_PIN=46dcc12a
  SELF_WINDOW_MB=5251.3476
  SELF_ENGINE_SITE=storage/engines/hashmap.rs:107
  SELF_ENGINE_PPS=30/30
  SELF_ENGINE_MB=552.7394
  SELF_ENGINE_SHARE=0.105257
  SELF_ENGINE_GROW_MB=0.0000
  SELF_QUEUE_SITE=storage/datastores/write_behind.rs:2424
  SELF_QUEUE_PPS=36/36
  SELF_QUEUE_MB=569.4847
  SELF_QUEUE_SHARE=0.108445
  SELF_QUEUE_GROW_MB=0.0000
  SELF_STAGING_SITE=storage/datastores/write_behind.rs:2479
  SELF_STAGING_PPS=61/63
  SELF_STAGING_MB=570.2629
  SELF_STAGING_SHARE=0.108594
  SELF_STAGING_GROW_MB=0.0000
  SELF_MULTI_SITE_PPS=0
  SELF_MULTI_SITE_MB=0.0000
  SELF_WINDOW_OK=TRUE
  SELF_E=0.322296
  SELF_E_UNROUNDED=0.3222957
  SELF_CHECK=PASS (E 0.3222957 vs 0.322296, tol 0.000005)
  SHARES_STOP=none
  E_FROZEN=SELF_CHECK_ONLY
```

**Base reading** — `LC_ALL=C python3 spec373b-shares.py --pin b166719d spec373b-de.dhat.json.gz spec373b-dl.dhat.json.gz --self spec371-c3e.dhat.json.gz spec371-c3l.dhat.json.gz`
(`spec373b-shares.txt`, lines after the in-program self-check):
```
  EARLY=spec373b-de.dhat.json.gz
  LATE=spec373b-dl.dhat.json.gz
  PIN=b166719d
  WINDOW_MB=4004.9391
  ENGINE_SITE=storage/engines/hashmap.rs:136
  ENGINE_PPS=30/30
  ENGINE_MB=446.7137
  ENGINE_SHARE=0.111541
  ENGINE_GROW_MB=0.0000
  QUEUE_SITE=storage/datastores/write_behind.rs:2463
  QUEUE_PPS=35/36
  QUEUE_MB=440.2769
  QUEUE_SHARE=0.109933
  QUEUE_GROW_MB=0.0000
  STAGING_SITE=storage/datastores/write_behind.rs:2518
  STAGING_PPS=61/62
  STAGING_MB=440.9683
  STAGING_SHARE=0.110106
  STAGING_GROW_MB=0.0000
  MULTI_SITE_PPS=0
  MULTI_SITE_MB=0.0000
  WINDOW_OK=TRUE
  E=0.331580
  E_UNROUNDED=0.3315803
  SHARES_STOP=none
  E_FROZEN=0.3316
```
Context only (not an input): the same program over SPEC-373a's `spec373a-d3{e,l}` pair (server `61f84658`, whose
three site lines are byte-identical to the pin's) gives `E = 0.286801` (engine 0.092678, queue 0.096985, staging
0.097138, `MULTI_SITE_PPS = 0`).

### Frozen values (read by the verdict program; never recomputed)
SHARES_STOP=none
E_FROZEN=0.3316

`E_FROZEN = 0.3316` (0.3315803 unrounded; engine 0.111541, queue 0.109933, staging 0.110106; `MULTI_SITE_PPS = 0`;
`_GROW_MB = 0` on every site). Thresholds it sets: `E/2 = 0.1658`; INDETERMINATE iff `s_b ≥ min(0.1658, 0.05) = 0.05`;
CONFIRMED iff `max I ≤ 0.8342`; `EXCESS` iff `1 − min I > 0.4974`. Prediction: `I ≈ 1 − E = 0.668`; carve total vs
`61f84658` ≈ `0.8841 × 0.6684 = 0.591`.

### Units (normative for the readout; quoted again before any NOT_MET is read as a miss)
`E` is a dhat-`tb` share: dhat adds the FULL new size on every `realloc`, stats_alloc (the CA probe's
`bytes_allocated`) only the growth. **Per site the two units agree:** all three sites are one-shot
`RecordValue::clone()`s — every allocation under a site's frame is a `Vec`/`String` clone at its final capacity —
and the program measures it: `_GROW_MB` (the window bytes of a site's program points whose stack also holds a
`finish_grow` / `realloc` frame) is `0.0000` for engine, queue and staging on the self-check pair, on the base pair
and on the 373a d3 pair. So a site's dhat bytes equal its stats_alloc bytes. The window **denominator** stays
dhat-inflated by the other (realloc-growing) lines, which biases `E` LOW in stats_alloc units; stated, not corrected.

### Prediction (recorded; decides nothing)
The b-cells run the base, so `BYTES_ALLOC_RATE_a / _b ≈ 1 − E` directly (`I` centred near `1 − E_FROZEN`). The carve
total against `61f84658` ≈ `P_BYTES_FROZEN(373a) × (1 − E)` = `0.8841 × (1 − E_FROZEN)`, recorded. The materialize
path's savings (R3) are not in the prediction (see "Materialize path presence").

### Verdict program (`spec373b-verdict.sh <EV> spec373b-manifest.md`, `SPEC373B_MANIFEST_COMMIT=M` required)
Definitions as SPEC-373a M′: one point per DISTINCT `alloc_probe_elapsed_s` (adjacent repeats dropped), `n` points,
1-based; **the last half starts at point `h = int((n−1)/2) + 1`** (0-based `ceil(n/2) − 1`; one point earlier than
`floor(n/2)` for even n — byte-identical to `spec372-k.awk:104`) and ends at `n`; `BYTES_ALLOC_RATE` =
(`bytes_alloc`[n] − `bytes_alloc`[h]) / (`e`[n] − `e`[h]), `ALLOC_LIVE` = `alloc_live_bytes`[n], printed `%.6f` /
integer with `points=`, `h=`, `window_s=`. `CHURN_RATIO` is not computed (recorded in no gate).
0. **ORDER=OK re-checked at the verdict's own start** (`spec373b-order.sh M <manifest>`), else **exit 3, no flags**.
   The one exception is a synthetic case: `SPEC373B_SYNTHETIC=1` skips it only when neither the cell dir nor the
   manifest is in the evidence dir, and prints `ORDER=SKIPPED (synthetic)`; a real run (cells and manifest in the
   evidence dir) and the smoke (manifest in the evidence dir) cannot skip it. Then exactly one `SHARES_STOP=` and one
   `E_FROZEN=` line (column 0), `SHARES_STOP ∈ {none, D, O, S}`, `E_FROZEN` numeric — else **exit 3, no flags**; a
   missing line is never read as a STOP. STOP-D/O/S = `SHARES_STOP`.
1. **STOP-V** for each of b1, b2, a1, a2 on any of: `PA` false; `PM1` false; `SKIPPED_<cell> > 0` (rows whose three
   probe fields are neither all empty nor all numeric); a `BYTES_ALLOC_RATE`, `ALLOC_LIVE` or `totalWrites`
   (`spec373b-<cell>.soak.json`) that is n/a or not > 0; **`WRITE_PARITY`**: `|w_c − mean(w)| / mean(w) > 0.05` for
   any cell's `totalWrites`. The STOP line names every failing clause, e.g. `STOP=V (a2:write_parity=0.0732)`.
   `RUNNER_EXIT` is recorded per cell, not gated. **PA and PM1 are executed from the frozen file, not transcribed:**
   the verdict asserts `sha256(spec371-predicates.sh) = 7d2ca6214beff1c4c0042879823172a45452ef99c06ca49521d5c96889a61d1b`,
   takes the PE/PA awk program from its lines 101–131 and PM1 from 154–169, strips only the closing `' "$CSV"`,
   asserts each ends at its closing brace, and runs them with `fl=CA` / `pm=` exactly as the frozen wrapper does
   (the byte-identical extraction of SPEC-373a M′, whose `diff` proof §1 of `spec373a-manifest.md` quotes).
2. `R_ij = rate(a_i) / rate(b_j)`, `I = [min, max]`; likewise `LIVE_I`. 3. `s = |x1 − x2| / mean(x1, x2)` per side and
   metric (`s_b`, `s_a`, `s_b_live`, `s_a_live`).
4. **STOP-R**, increase only: `min I > 1 + max(s_b, s_a)` or `min LIVE_I > 1 + max(s_b_live, s_a_live)`. A drop never
   fires it.
5. **VERDICT_BYTES**: INDETERMINATE iff `s_b ≥ min(E/2, 0.05)`; else CONFIRMED iff `max I ≤ 1 − E/2`; else NOT_MET.
   The cap keeps the noise guard from loosening as `E` grows; AC-1 gates the mechanism, the cells confirm integration.
6. **VERDICT_LIVE = DESCRIPTIVE** (no unbiased per-site live source: dhat writes its profile after `hard_flush`, so
   `eb` sees empty queue/staging, and `gb` is a different instant). Printed with `LIVE_I`, `S_B_LIVE` and
   `LIVE_SIGN` = `down` iff `max LIVE_I < 1 − max(s_b_live, s_a_live)`, `up` iff
   `min LIVE_I > 1 + max(s_b_live, s_a_live)` (which is also STOP-R), else `flat`. Not a class, not a gate. Sharing is
   proven deterministically instead (AC-2a/2b/2d).
7. Precedence **D > O > S > V > R**; under any STOP, `VERDICT_BYTES=WITHHELD`. There is no STOP-L.
8. **Recorded, not a STOP and not a class:** `EXCESS=TRUE` iff `1 − min I > 1.5 E` (a large overshoot is flagged for
   the conductor, never read as success); `BYTES_PER_WRITE_<cell>` = rate ÷ (`totalWrites` / the cell's matrix
   `duration:` — 900 s for every gating cell), and its a/b ratio range; the four `totalWrites` and their max relative
   deviation (`WRITE_DEV_<cell>`).
9. Flags, after every STOP predicate: `STOP=`, `VERDICT_BYTES=`, `VERDICT_LIVE=DESCRIPTIVE`, `LIVE_SIGN=`, `I=`,
   `S_B=`, `E=`, `LIVE_I=`, `S_B_LIVE=`, `EXCESS=`, `BYTES_PER_WRITE=`, `WRITE_PARITY=`. **Exit status:** 0 = the flags
   block was printed; 3 = ORDER not OK, an invalid pre-M line or frozen predicate source; 4 = a program step failed
   (every awk's exit status is checked; the intermediate file is kept). The chain logs the status and says "NO flags"
   when it is non-zero.
10. **Merge:** with the ACs green, merge on CONFIRMED, NOT_MET or INDETERMINATE with `STOP=none`; any STOP → the
    conductor.

**Synthetic cases** (`spec373b-synth.sh`; pinned inputs, expectations derived by hand in its header), run against
this verdict program before M — all nineteen as expected:

| case | output (flags block, verbatim fields) |
|---|---|
| S1 | rc=0 STOP=none CONFIRMED LIVE_SIGN=flat I=[0.8515,0.8700] S_B=0.009950 EXCESS=FALSE WRITE_PARITY=OK max_dev=0.0000 |
| S2 | rc=0 STOP=none NOT_MET LIVE_SIGN=flat I=[0.9505,0.9650] S_B=0.009950 EXCESS=FALSE WRITE_PARITY=OK max_dev=0.0000 |
| S3 | rc=0 STOP=none INDETERMINATE LIVE_SIGN=flat I=[0.7818,0.8700] S_B=0.095238 EXCESS=TRUE WRITE_PARITY=OK max_dev=0.0000 |
| S4 | rc=0 STOP=R WITHHELD LIVE_SIGN=flat I=[1.2871,1.3100] S_B=0.009950 EXCESS=FALSE WRITE_PARITY=OK max_dev=0.0000 |
| S5 | rc=0 STOP=V (b2:PM1) WITHHELD LIVE_SIGN=flat I=[0.8515,0.8700] S_B=0.009950 EXCESS=FALSE WRITE_PARITY=OK max_dev=0.0000 |
| S6 | rc=0 STOP=S WITHHELD LIVE_SIGN=flat I=[0.8515,0.8700] S_B=0.009950 EXCESS=FALSE WRITE_PARITY=OK max_dev=0.0000 |
| S7 | rc=0 STOP=R WITHHELD LIVE_SIGN=up I=[0.8515,0.8700] S_B=0.009950 EXCESS=FALSE WRITE_PARITY=OK max_dev=0.0000 |
| S8 | rc=0 STOP=none CONFIRMED LIVE_SIGN=flat I=[0.8500,0.8500] S_B=0.000000 EXCESS=FALSE WRITE_PARITY=OK max_dev=0.0000 |
| S9 | rc=0 STOP=V (a1:rate=0.000000 a2:rate=0.000000) WITHHELD LIVE_SIGN=n/a I=n/a S_B=n/a EXCESS=n/a WRITE_PARITY=OK max_dev=0.0000 |
| S10 | rc=0 STOP=V (b1:skipped=1) WITHHELD LIVE_SIGN=flat I=[0.8515,0.8700] S_B=0.009950 EXCESS=FALSE WRITE_PARITY=OK max_dev=0.0000 |
| S11 | rc=3, no flags — FATAL: the manifest must carry exactly one SHARES_STOP= and one E_FROZEN= line (found 0 / 1) |
| S12 | rc=3, no flags — FATAL: E_FROZEN='abc' is not a number |
| S13 | rc=0 STOP=none INDETERMINATE LIVE_SIGN=flat I=[0.5660,0.6100] S_B=0.058252 EXCESS=FALSE WRITE_PARITY=OK max_dev=0.0000 |
| S14 | rc=0 STOP=none CONFIRMED LIVE_SIGN=flat I=[0.6931,0.7050] S_B=0.009950 EXCESS=TRUE WRITE_PARITY=OK max_dev=0.0000 |
| S15 | rc=0 STOP=V (a2:write_parity=0.0732) WITHHELD LIVE_SIGN=flat I=[0.8515,0.8700] S_B=0.009950 EXCESS=FALSE WRITE_PARITY=FAIL max_dev=0.0732 |
| S16 | rc=0 STOP=none CONFIRMED LIVE_SIGN=down I=[0.8515,0.8700] S_B=0.009950 EXCESS=FALSE WRITE_PARITY=OK max_dev=0.0000 |
| S17 | rc=0 STOP=none CONFIRMED LIVE_SIGN=flat I=[0.8515,0.8700] S_B=0.009950 EXCESS=FALSE WRITE_PARITY=OK max_dev=0.0000 |
| S18 | rc=3, no flags — FATAL: the manifest must carry exactly one SHARES_STOP= and one E_FROZEN= line (found 1 / 0) |
| S19 | rc=3, no flags — ORDER=FAIL spec373b-order.sh does not hash as M '' lists it; FATAL: ORDER is not OK (rc=3); no flags printed |

Case inputs and hand-derived expectations: the header of `spec373b-synth.sh`; every output above matches them.

### Recorded JE pair (not gating, no class, n = 1 per side)
`SPEC373B_PHASE=je`: jb (pin) then ja (freeze), 14400 s each. Each cell is read by `spec373b-je.sh`, which runs the
frozen `spec372-predicates.sh` (sha256 asserted, unedited) over a copy of the cell's artifacts named `spec372-j2`
(the 4 h JE cell name that program knows) with a builds stub carrying THIS chain's JE server and harness sha256, and
copies `predicates/fits/amp/ampfp` back as `spec373b-<cell>.*`. Recorded: `TERM_` and `DECIDE_` `AMP_FP`, `AMP_JE`,
`DIRTY_SHARE`, `FRAG_SHARE` (and `je_allocated`), "n = 1, descriptive"; no within-side mode rule. Checked before M:
over a copy of the committed `spec372-j2` artifacts the adapter reproduces the committed `spec372-j2.predicates.txt`
readings exactly (`PV=TRUE`, `TERM_AMP_FP=2.210888`, `TERM_DIRTY_SHARE=0.220766`, `TERM_FRAG_SHARE=0.808387`).

### Materialize path presence (measured statement for the readout)
Per gating cell the readout quotes, from the last scrape, every existing counter that counts loads,
materializations or evictions. At `b166719d` **no counter counts materializations or evictions**; the nearest are
`topgun_or_prune_absent_total`, `topgun_or_prune_restored_evicted_total` and
`topgun_update_in_place_materialize_exhausted_total` (registered on first increment — its absence from a scrape reads
as 0). The readout states this and relies on AC-1's second case (materialize of a staged evicted key) for the path;
no new production metric in this spec.

### Runner diff: every hunk maps to one of the six items (`diff spec373a-cells.sh spec373b-cells.sh`)
| hunk | item |
|---|---|
| `2a3,42` | 1–6 — this runner's header (the parent's header follows verbatim) |
| `329c369`, `331,332c371,372` | 1 — usage text: name, lineage, item count |
| `337,339c377,379`, `340a381,382` | 1 + 6 — usage cell list (de/dl/b/a at pin b166719d, jb/ja) |
| `346c388`, `354,356c396,398` | 2 — env names in the usage text |
| `374c416` | 1 — flavour column doc (`CA|DH|JE`) |
| `385c427` | 2 + 6 — the pin and freeze names in the cell-table comment |
| `387,390c429,434` | 1 — the cell table |
| `393c437` | 4 — basename `spec373b-<cell>` |
| `475c519` | 4 — data dir `target/spec373b-<cell>-data` |
| `628,630c672,675` | 4 — port 47360 |
| `655,657c700,702` | 2 + 3 — freeze variable and its literal |
| `668,669c713,714`, `676c721`, `693,694c738,739`, `697c742`, `699c744`, `710c755`, `712c757`, `783c829`, `806c852`, `836c882`, `942c988`, `954c1000`, `988c1034` | 2 — env names |
| `691c736` | 2 — the chain names in the "builds nothing" comment |
| `706,707c751,752` | 6 + 2 — the pin literal of the server code-state assertion, freeze variable name |
| `749a795` | 5 — the JE flavour marker line (spec372-allocdiag.sh's own) |
| `937c983` | 1 — matrix banner |

### Programs frozen at M (sha256; `ORDER=OK` re-checks these bytes)
- `8d2db8745dfa9c62ff432d77898fa0f40ad69634626cff82139c593b9ccbb5e1` `packages/server-rust/benches/soak_harness/evidence/spec373b-cells.sh`
- `7dd8290f2499b82f5d11b9fbf3ac931b0fd54619876dbd2e1f0bef95e6e0a6ea` `packages/server-rust/benches/soak_harness/evidence/spec373b-diag.sh`
- `48b8297b35112d6aece01d57c3b87a14585a103f7ad25c43dbc3ef87e72e8b1c` `packages/server-rust/benches/soak_harness/evidence/spec373b-chain.sh`
- `4ce11cff147d3db5783df440a56e3a72b071696aa90310796c5aa9ecb695c88b` `packages/server-rust/benches/soak_harness/evidence/spec373b-order.sh`
- `d5cef0dfe2033f13dc8c5446a49233046a38beeaa43f5ea24817aab1d836dffa` `packages/server-rust/benches/soak_harness/evidence/spec373b-verdict.sh`
- `1ce3bbb1744b0eed8ab8d6de82e9c363b7d3db11510e6bc02c0cca0e101285b0` `packages/server-rust/benches/soak_harness/evidence/spec373b-synth.sh`
- `8a792edb10e4c13f8c2948e496db80ba9a28101012bbea25149e22483945dda3` `packages/server-rust/benches/soak_harness/evidence/spec373b-je.sh`
- `e7931e2c02b11c72f98f51604c4a7428c762552be8e4d1fdfcb880e5b1247c67` `packages/server-rust/benches/soak_harness/evidence/spec373b-shares.py`
- `8f9e99abc62160b73235bba3689b8f843013420fcdd255e99e0e4201d041bafd` `packages/server-rust/benches/soak_harness/evidence/spec373a-cells.sh`
- `7d2ca6214beff1c4c0042879823172a45452ef99c06ca49521d5c96889a61d1b` `packages/server-rust/benches/soak_harness/evidence/spec371-predicates.sh`
- `baf0bce7dd6c29751fb62f525153b631ec9eeb37f0ba858aed7174fd978b1c91` `packages/server-rust/benches/soak_harness/evidence/spec372-predicates.sh`
- `840813461e3b1bd5c3a79291044d8ac515e09b94333ee530cd6a10de8fa0436f` `packages/server-rust/benches/soak_harness/evidence/spec349c2-fit.awk`
- `2e3ba4f4c0429d77d7f1cf267112706ddf95b095b2a14a6b05460cfa5d018c33` `packages/server-rust/benches/soak_harness/evidence/spec366-p5.awk`
- `ba65ffc4076307ffdbfb014565edaf1f17e185ef987ca6b3fe2565d544400215` `packages/server-rust/benches/soak_harness/evidence/spec366-p67.awk`

Commands, verbatim:
```
shasum -a 256 packages/server-rust/benches/soak_harness/evidence/spec373b-{cells,diag,chain,order,verdict,synth,je}.sh packages/server-rust/benches/soak_harness/evidence/spec373b-shares.py
shasum -a 256 packages/server-rust/benches/soak_harness/evidence/spec373a-cells.sh packages/server-rust/benches/soak_harness/evidence/spec371-predicates.sh packages/server-rust/benches/soak_harness/evidence/spec372-predicates.sh
shasum -a 256 packages/server-rust/benches/soak_harness/evidence/{spec349c2-fit,spec366-p5,spec366-p67}.awk
```
The parent programs `spec373a-cells.sh`, `spec371-predicates.sh`, `spec372-predicates.sh` and its three awk helpers
are not edited.

### The §1 prefix sha256 — the command
Computed at M and at every later commit by exactly this command (the marker line is included in the hash); M's value
is recorded in the executor report and re-computed by `spec373b-order.sh`, never written into §1:
```
git show <commit>:packages/server-rust/benches/soak_harness/evidence/spec373b-manifest.md | sed '/^## APPEND-ONLY BELOW/q' | shasum -a 256
```

### ORDER=OK (`spec373b-order.sh`; the chain at start, the verdict at start, the data commit and HEAD)
1. M is an ancestor of the commit.
2. The §1 prefix sha256 of the manifest the caller reads equals M's (the command above), and M carries the marker.
3. Every program listed above hashes to its listed sha256, and M lists at least 10.
4. No build input differs from the freeze literal and the working tree is clean over it:
   `git diff --quiet <freeze>..HEAD -- packages/server-rust/src packages/server-rust/Cargo.toml packages/server-rust/build.rs packages/core-rust Cargo.toml Cargo.lock rust-toolchain.toml`
   and an empty `git status --porcelain` over the same pathspec (else the a-cells are re-run); the freeze literal is a
   hex commit id.
`spec373b-order.sh` cannot vouch for itself: the chain and the verdict first check its sha256 against M's listing
below and refuse to run it otherwise.
The chain prints `ORDER=OK manifest_commit=… prefix_sha256=… programs=… freeze=…` into its log and refuses to run on
any mismatch; the verdict prints the same line first and exits 3 on any mismatch.

### Cross-vendor review of the programs (before M)
`/xreview` (glm-5.3 via OpenRouter) over the evidence programs at `ef72aea1`, in two passes (verdict + synth + order;
shares + chain + diag + je; `spec373b-cells.sh` is reviewed through its hunk map above). Fixed before M: the
freeze pathspec gains `rust-toolchain.toml` and the freeze literal must be hex; the chain and the verdict check
`spec373b-order.sh`'s sha256 against M before running it; the verdict's intermediate file is a fresh `mktemp` outside
the evidence dir, its creation checked (a stale intermediate can no longer be read); `post_mortem_rows` must be a
plain count before it reaches the frozen PM1 program; `S_B`/`S_B_LIVE` printed `%.6f` (the class compares full
precision); the synthetic suite refuses any output dir under the evidence dir and checks its S10 corruption step;
the chain logs the synth exit status; `spec373b-shares.py` fires STOP-D on a non-positive window, prints
`MULTI_SITE_MB`, and refuses `--self` in self-check mode. Refuted or out of scope, with reasons, in the executor
report (STOP 5a).

### Carried traps
`LC_ALL=C` everywhere a number is parsed (host is `ru_RU`); literals from the code (commands above); program sha
binding; flags printed after every STOP predicate; post-mortem row rule PM1; synthetic cases pin their inputs; smoke
over every program path before the cells (`SPEC373B_SMOKE=1`); never bisect on `rss_mb`; release builds are not
byte-reproducible — only the sha256 of the LAUNCHED binary counts (console line 1 vs `spec373b-builds.txt`); the
write-behind WAL partition is `fnv1a("{map}:{key}") % 271`, not `hash_to_partition(key)`.

## APPEND-ONLY BELOW
