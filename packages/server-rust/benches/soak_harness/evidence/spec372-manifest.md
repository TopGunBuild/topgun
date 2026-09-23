# SPEC-372 (carve 9b) — allocator evaluation: pre-registration manifest

## §1 Pre-registration (frozen at commit M; nothing above the APPEND-ONLY marker changes after M)

### Question and scope
SPEC-371 isolated the OR-churn soak's footprint growth as allocator RETENTION: release grows
2107 ± 167 MB/h in `phys_footprint` while the reachable heap grows 221–232 MB/h. This carve asks
whether swapping the server's global allocator removes that retention, and if so which allocator
the NEXT step should make the default. It is an evaluation, not a flip: the shipped default stays
the System allocator, no CRDT / write-behind / WAL / journal / prune code changes, and no gate,
monitor or threshold moves.

Arms: **SYS** (plain release, System allocator — the reference), **JE** (`--features
alloc-jemalloc`: tikv-jemallocator + tikv-jemalloc-ctl 0.7, `stats`, compiled defaults, no
`MALLOC_CONF`, no background threads), **MI** (`--features alloc-mimalloc`: mimalloc 0.1,
`default-features = false`). **CA** (`--features count-alloc`) runs one instrument cell, `k1`, for
the churn reading; it is never an arm.

### Code under test (commit K)
- `901e4b7c` feat(server): the two features, the allocator cfg lattice, `ALLOCATOR_NAME`, the
  `je_config`/`je_probe` lines, `bytes_alloc`/`bytes_dealloc` on `alloc_probe`, the boot-line
  `allocator=` field and its test assertion, and the same lattice in the load harness.
- `08cdef2f` fix(server): tikv-jemalloc-ctl's string reads return the C terminator
  (`raw.rs:371-372`, `slice::from_raw_parts(ptr, len + 1)`), and macOS awk ends a record at a NUL,
  so `je_config` lost every field after the version. The version is trimmed before printing.
  **The runner's freeze literal is `08cdef2f`.**

### Resolved dependency versions (Cargo.lock at K), every cited line re-read against them
| crate | resolved | cited facts re-read |
|---|---|---|
| `tikv-jemallocator` | 0.7.0 | `src/lib.rs:92,94` (unit struct, `unsafe impl GlobalAlloc` inside the crate); features `stats`, default `background_threads_runtime_support` (`Cargo.toml:70-73,85`) |
| `tikv-jemalloc-ctl` | 0.7.0 | `lib.rs:108,130,164,222,229`; `arenas.rs:4`; `opt.rs:57,144,169,194`; `stats.rs:8-15,40,75,105,143,178`; `macros.rs:50,58`; `raw.rs:69,93` (`pub unsafe fn`); `raw.rs:371-372` (the NUL, above) |
| `tikv-jemalloc-sys` | 0.7.1+5.3.1-0-g81034ce1f1373e37dc865038e1bc8eeecf559ce8 | `arena_types.h:8-9` (dirty 10 000 ms, muzzy 0); `pac.c:488-489,491-492,522-541`; `pages.c:519-521`; `configure.ac:740-741,751-753,2836-2838`; `background_thread.c:12,14`; `build.rs:192-204,234-246,327-335` |
| bundled jemalloc | `5.3.1-0-g81034ce1f1373e37dc865038e1bc8eeecf559ce8` | also read at run time off `je_config version=` |
| `mimalloc` | 0.1.52 | `src/lib.rs:46,48` |
| `libmimalloc-sys` | 0.1.49 | `build.rs:8-12` (v3 unless feature `v2`); `c_src/mimalloc/v3/src/options.c:127` (`purge_decommits` 1), `:140` (`purge_delay` 1000 ms); `prim/unix/prim.c:495-498` (decommit = `MADV_FREE_REUSABLE`, fallback `MADV_DONTNEED`), `:522-524` (`#if 0`), `:529-540` (reset `MADV_FREE`) |
| bundled mimalloc | `MI_MALLOC_VERSION 30302` (3.3.2) | `include/mimalloc.h:11` |

No cited fact moved between the spec's read snapshot and the resolved versions. The lock also
re-resolved six `windows-sys` edges (0.48/0.59 → 0.60/0.61); those crates build only for Windows
targets, which this project does not ship, and the pin's lock re-resolves identically without the
new dependencies.

Measured on the JE build at run time (smoke): `je_config version=5.3.1-0-g81034ce1… arenas_narenas=41
opt_narenas=40 opt_background_thread=false background_thread=n/a max_background_threads=n/a
opt_tcache=true opt_tcache_max=32768 opt_dirty_decay_ms=n/a opt_muzzy_decay_ms=n/a opt_retain=n/a`.
`background_thread=n/a` is the ENOENT that R1.4 predicts: macOS compiles no background purge thread.

### The allocator lattice (spec R1.3, verbatim)
| static | cfg predicate |
|---|---|
| `dhat::Alloc` | `all(feature = "dhat-heap", not(feature = "count-alloc"))` *(unchanged)* |
| `&StatsAlloc<System>` | `feature = "count-alloc"` *(unchanged)* |
| `tikv_jemallocator::Jemalloc` | `all(feature = "alloc-jemalloc", not(any(feature = "count-alloc", feature = "dhat-heap")))` |
| `mimalloc::MiMalloc` | `all(feature = "alloc-mimalloc", not(any(feature = "count-alloc", feature = "dhat-heap", feature = "alloc-jemalloc")))` |
| *(none — the System allocator)* | `not(any(feature = "count-alloc", feature = "dhat-heap", feature = "alloc-jemalloc", feature = "alloc-mimalloc"))` |

`ALLOCATOR_NAME` is declared once per row under these predicates (`"count-alloc" | "dhat" |
"jemalloc" | "mimalloc" | "system"`). The `je_probe` task is gated on the JE row, not on the bare
feature: when an instrument allocator wins, jemalloc is linked but idle. Smoke: `cargo check` of the
binary and the load harness passes on all 16 subsets (`LATTICE_TOTAL=TRUE`); the default build's
boot line reads `allocator=system` and the JE build's `allocator=jemalloc` (live runs).

### Flavour markers (read off the built binaries)
| flavour | marker | SYS | JE | MI | CA |
|---|---|---|---|---|---|
| JE | `je_probe elapsed_s=` | 0 | 1 | 0 | 0 |
| MI | `mimalloc: warning: ` (the bundled C's message prefix; the bare word `mimalloc` also sits inside a concatenated Rust string blob in the MI binary, so it is NOT a marker) | 0 | 0 | 1 | 0 |
| CA | `alloc_probe elapsed_s=` and `bytes_alloc=` | 0 | 0 | 0 | ≥1 |
| any | `DHAT_OUT` | 0 | 0 | 0 | 0 |

SYS is the ABSENCE of every other literal. H (the harness) carries `tombstone-byte level ceiling
breached` and `soak: child TOPGUN_JOURNAL_ENABLED=`.

### Cells (spec R4.1 / R5.1)
Stage 1 (`spec372-chain1.sh`), 900 s each, in order: `s1a` SYS, `j1a` JE, `m1a` MI, `k1` CA,
`j1b` JE, `m1b` MI, `s1b` SYS. Stage 2 (`spec372-chain2.sh`), after the conductor's go at STOP 3a:
`s2` SYS 4 h, `j2` JE 4 h iff JE survived, `m2` MI 4 h iff MI survived, `a2j` 900 s journal OFF on
`S1_RANK`'s arm. Matrix: SPEC-371's literals (`spec371-memdiag.sh:515-534`) except the port, which
is 47358 (closed-list item 4). CSV cadence 60 s, live-copy census 300 s, SIGKILL teardown. The host
state (`uptime`, `vm_stat`, `memory_pressure`, `pmset -g therm`) is recorded before every cell;
`OPS_RATIO` is the host-drift guard.

### Budgets (the two tables of the spec, the estimated line now MEASURED at the smoke)
jemalloc's `./configure` + `make` — cargo's timing row "tikv-jemalloc-sys … build script (run)" —
is **31.7 s** (31.3 s on the final smoke), and it runs in parallel with the Rust compile: the smoke's cold builds took SYS 271 s,
JE 289 s, MI 285 s, CA 263 s, H 267 s (chain 1) and SYS 258 s, JE 266 s, MI 258 s, H 268 s
(chain 2).

| chain | builds (measured) | cells | total |
|---|---|---|---|
| 1 | 22 min 55 s | 7 × 15 min 03 s | **≈ 2 h 08 min** |
| 2, both survivors | 17 min 30 s | 3 × (4 h + 2 min 30 s) + 15 min 03 s | **≈ 12 h 40 min** |
| 2, one survivor | ≈ 13 min | 2 × (4 h + 2 min 30 s) + 15 min 03 s | **≈ 8 h 33 min** |

The two chain-2 build lines together are 17 min 30 s, under the 25 min trigger, so **`a2j` stays**
(`A2J_DROPPED_FOR_BUDGET=no`, a literal in `spec372-chain2.sh`). Disk: ≥ 30 GB asserted after each
chain's reclaim (smoke read 97 and 84 GB).

### The gate (spec R5.2–R5.4), and every definition the programs pin
- `reachable_est_bytes = A0 + B_live × live`, **A0 = 16.64 MiB** (mean `alloc_live_mb` at t = 60 over
  SPEC-371 c0 15.607 / c1 17.016 / c2 17.296), **B_live = 886 B** (mean of 912 / 863 / 882; the third
  digit is an interpolation). It models the live OR set ONLY.
- `AMP_FP = phys_footprint / reachable_est` (flagged), `AMP_S = (phys_footprint + reclaimable) /
  reachable_est` (recorded), `AMP_JE = stats.resident / stats.allocated` (recorded).
- **Series points** are census instants only: every `LIVE_COPY` joined to the nearest CSV row with a
  footprint within ±30 s (none ⇒ dropped), `TERMINAL` joined to the last row with a footprint and
  dropped when the lag exceeds 90 s; `CHECKPOINT` rows never enter the series. Every point prints
  its `join_lag_s` in `<cell>.amp.txt`.
- **The deciding point** of a cell is its last USED census point (TERMINAL unless dropped).
  Stage-1 readings named "at TERMINAL" (`TERM_*`, `EST_AGREE_900_*`, the DROP rule's `AMP_FP` for
  `S1_RANK`, `CV`) read `n/a` when TERMINAL was dropped.
- **Fits**: frozen `spec349c2-fit.awk`. `TREND` = `window=last_half` over `<cell>.ampfp.csv`;
  last third = rows `floor(2n/3) … n-1` of the used series fitted with `window=full`; JE native =
  `FP × 2^20 / je_allocated` on the 60 s row clock, `window=last_half`. The slope field is named
  `slope_mb_per_hour`; on these series it reads AMP units per hour.
- `TREND`: `NON_INCREASING` slope ≤ +1 se; `MARGINAL` ≤ +2 se; `INCREASING` above; `n/a
  reason=few_points` when the last half holds fewer than 12 points (the fitter's `n`, or
  `used − floor(used/2)` when the fit failed).
- `S = FP + reclaimable_mb`; `FP_end` / `S_end` at the last row carrying a footprint.
- **DROP rule** (R4.3): an arm is DROPPED iff, in both replicates, `FP_end > max(FP_end s1a, s1b)`
  AND `S_end > max(S_end s1a, s1b)`. `decide.awk stage1` is authoritative; `k.awk` computes the
  same and chain 1 asserts byte-for-byte agreement (a disagreement ends chain 1 with `CHAIN_RC=3`).
- **`S1_RANK`** (selects `a2j` only): lower mean TERMINAL `AMP_FP`; when the two means are within
  10 % of the larger, lower mean `S_slope`; equal slopes ⇒ JE.
- **`CHURN_RATIO`** (k1): one point per DISTINCT `alloc_probe` line; last half = points
  `floor(n/2) … n-1`, first half = points `0 … floor(n/2)` (sharing the boundary so the windows
  abut). Ratio = (Δ`bytes_alloc` / Δ probe seconds) / `alloc_live_bytes` at the window's LAST point —
  the same denominator the spec's predicted 0.89–1.02 /s used. `CHURN_RATIO_DRIFT` = first-half ratio ÷
  last-half ratio; `K_PROVISIONAL=TRUE` outside [0.67, 1.5]. A zero denominator prints a named `n/a`.
  `T_DECAY_UPPER_JE = 10 × CHURN_RATIO`, `_MI = 1 × CHURN_RATIO` (recorded only).
- `CV_<arm>` = sample (n − 1) standard deviation of the two replicates' TERMINAL `AMP_FP` ÷ their mean.
- **Bounds at D2** (`k.awk stage2`, at the arm's own 4 h deciding point): `K_lo = 1 + A0_share +
  R_meta` (JE `stats.metadata / reachable_est`; MI the frozen literal 0.05), `K_hi = K_lo + F_class
  (JE 0.25, MI 0.125) + R_redb` (`redb_mb / reachable_est_mb`); `K_HI_VACUOUS_<arm>` iff `K_hi ≥
  0.50 × AMP_FP_S2`; `LEVEL = WITHIN_K_HI` iff `AMP_FP ≤ K_hi`.
- `VS_SYS_<arm> = AMP_FP_<arm> / AMP_FP_S2` at each cell's own deciding point, `BETTER` iff ≤ 0.50;
  `VS_SYS_S` likewise on `AMP_S` (recorded; clause 8's ordering check).
- **VERDICT** by the first matching clause of R5.4's total order (`PLATEAU` ≻ `BOUNDED_ABOVE_K` ≻
  `MARGINAL` ≻ `BOUNDED_NO_GAIN` ≻ `NOT_BOUNDED` ≻ `n/a`). `n/a` reasons in precedence:
  `dropped_stage1`, `cell_did_not_run`, `ops` (`OPS_RATIO_<arm>_S2 < 0.95`), `missing_reading`,
  `few_points`.
- `JE_ESTIMATOR_AGREE` = `stats.allocated / reachable_est` on `j2` within ±25 %;
  `EST_PROVISIONAL=TRUE` iff that is `FALSE` or `n/a`, **including when JE did not survive** (R5.2,
  verbatim — it therefore routes a sole-MI `PLATEAU` to `ESTIMATOR_UNVERIFIED` by design).
- `RECLAIM_SEMANTICS_<arm>` (recorded): `reclaimable_end / FP_end ≤ 0.02` ⇒ `MADV_FREE`
  (returned, nothing in Darwin's reusable state), `> 0.02` ⇒ `REUSABLE`, unreadable ⇒ `UNKNOWN`.
- `DIRTY_SHARE = (resident − active − metadata) / allocated`, `FRAG_SHARE = (active − allocated) /
  allocated`, `UNMODELLED_REACHABLE = allocated − reachable_est` (MB and share) on every JE cell.
- `OPS_a2j` and `OPS_RATIO_a2j_vs_4h` = a2j's ops/s ÷ the same arm's 4 h cell's ops at the
  `progress.jsonl` checkpoint nearest t = 900 (within 60 s). Recorded; never a verdict input.
- **STOPs**: per cell `PV PR-crashes PR-class P5 P6 P7 PE PA PJ PC PM1`, with `PA=n/a` accepted on
  SYS and MI cells only; `WRITE_ERRORS > 0`; `DISK_FREE_AT_START < 30`; any missing input file;
  Stage 2 additionally the Stage-1 file's own `STOP` ≠ `FALSE`, an unrecognised survivor set, a
  missing `s2` (the same-chain reference is not optional) and any cell chain 2 ran whose predicates
  are absent. `PE`: ≥ N − 1 of N rows carry a footprint, `N = D/cadence + 1` from `matrix.txt`.
  `PA`: ≥ ⌈D/30⌉ − 2 well-formed probe lines (JE: all eight `je_probe` fields numeric; CA:
  `bytes_alloc`/`bytes_dealloc` numeric). `PJ`: the harness echo reads `false` on `a2j` and `true`
  elsewhere — the harness always echoes (`process.rs:318-320`, default `true`), so "absent" in R6
  means the env is unset.
- **PERF_<arm>** (`spec372-perf.sh`; conductor rulings v5 R2): CI's perf-gate shape lengthened to
  the harness default duration — `--scenario throughput --connections 200 --duration 30`, `--interval
  50` (fire-and-wait) and `--interval 0 --fire-and-forget` — **5 runs per mode per block**, blocks SYS →
  JE → MI → SYS; one run per block at CI's own 15 s fire-and-wait parameters is recorded as the
  reference the shape came from, and never gates. Against SYS's RANGE (the two SYS block medians):
  `FAIL` iff the arm's median ops/s `< 0.80 ×` the LOWER SYS block median in either mode, OR its
  fire-and-wait median p99 `> 1.20 ×` the HIGHER SYS block median; `n/a reason=harness_noisy` iff the
  two SYS blocks' median ops/s differ by `≥ 20 %` of the smaller in either mode (ops/s only — the
  SYS-vs-SYS p99 spread is printed, not gated); `n/a reason=run_failed` iff a run reported no numeric
  field; `PASS` otherwise. p50 and the fire-and-forget p99 are recorded.
- **BUILD_<arm>** (`spec372-buildstory.sh`): `OK` iff items 1, 2, 3 build and items 4, 5 exit 0;
  `FAIL` iff item 1 fails; `PARTIAL` otherwise. Docker (item 3) builds for the host's native
  platform (linux/aarch64 here) from a scratch context holding exactly what the Dockerfile COPYs:
  the repo root's `target/` is not dockerignored, so `docker build .` would ship it whole.
- **SIZE_<arm>** = the arm's stripped linux-x64 binary minus SYS's; `SIZE_HEADROOM_<arm>=TIGHT`
  within 1 MiB of the 20 MiB ceiling.
- **DEFAULT_CANDIDATE** and **NEXT**: R8's three steps and 17-position decision list, verbatim, in
  `decide_next()`. Closed literal list: `STOP`, `CONDUCTOR_RULING;ALLOCATORS_WORSE_AT_900S`,
  `CONDUCTOR_RULING;OPS`, `CONDUCTOR_RULING;CELL_DID_NOT_RUN_NA`, `CONDUCTOR_RULING;FEW_POINTS_NA`,
  `CONDUCTOR_RULING;MISSING_READING_NA`, `CONDUCTOR_RULING;BETTER_NOT_BOUNDED`,
  `TODO-591;ALLOCATOR_INSUFFICIENT`, `CONDUCTOR_RULING;BOUNDED_NO_GAIN`,
  `CONDUCTOR_RULING;TREND_MARGINAL`, `CONDUCTOR_RULING;ARM_ORDER_ACCOUNTING`,
  `CONDUCTOR_RULING;BOUNDED_ABOVE_K`, `CONDUCTOR_RULING;K_VACUOUS`,
  `CONDUCTOR_RULING;ESTIMATOR_UNVERIFIED`, `CONDUCTOR_RULING;PERF_NA`,
  `CONDUCTOR_RULING;PERF_VS_MEMORY`, `CONDUCTOR_RULING;BUILD_NA`, `CONDUCTOR_RULING;BUILD_STORY`,
  `TODO-589;THEN;DEFAULT_FLIP;THEN;TODO-591`, `CONDUCTOR_RULING;UNMATCHED`. Every flag prints after
  every STOP predicate; under `STOP=TRUE` every flag reads `STOP`.

### Purge mechanism per arm, the path the DEFAULT configuration takes (spec R5.2)
| arm | default path | expected `reclaimable_mb` |
|---|---|---|
| SYS | libmalloc magazine `madvise(MADV_FREE_REUSABLE)`: pages stay mapped, resident-but-reusable | large |
| MI | purge → decommit (`purge_decommits` 1, `purge_delay` 1000 ms); decommit on macOS is `MADV_FREE_REUSABLE`, falling back to `MADV_DONTNEED` | non-zero |
| JE | `muzzy_decay_ms` 0 ⇒ decayed dirty extents go straight to forced purge / dalloc (demand-zero `mmap` overlay, `retain=1`), landing in `stats.retained` | ≈ 0 |

The smoke's 120 s and 300 s cells already read `RECLAIM_end = 0.000` on every JE cell and 5–14 MB on
SYS and MI — consistent with the table, and decided by nothing.

### Predicted values (spec R5.3, computed from committed artifacts; 2 decimals, half away from zero, from unrounded inputs)
| quantity | predicted |
|---|---|
| `reachable_est` at 4 h | 901.82 MiB |
| `AMP_FP_S2` | 9.13 |
| `BETTER` bar | 4.57 |
| `A0_share` | 0.02 (0.01845) |
| `R_redb` | 0.24 (off the file size; ~70× loose against the 3.10 MB live cache) |
| `CHURN_RATIO` | 0.89 – 1.02 /s |
| `T_DECAY_UPPER_JE` / `_MI` | ≈ 9.6 / ≈ 0.96 (recorded only) |
| `K_lo_JE` / `K_lo_MI` | ≈ 1.04 (assumed `R_meta_JE` ≈ 0.02, replaced by `j2`'s probe) / 1.07 |
| `K_hi_JE` / `K_hi_MI` | 1.53 / 1.43 |
| `K_HI_VACUOUS_JE` / `_MI` | FALSE / FALSE |

The verdict therefore rests on `TREND` + `VS_SYS` + a two-valued `LEVEL`. Every Stage-2 verdict is
n = 1 per arm; the conductor may order ONE replicate 4 h cell of the candidate as a pre-registered
addendum before any `DEFAULT_FLIP` step.

### Statements the readout (§3) must carry (normative, AC-11)
Stage 1 is an early-window screen that never decides; `S` is arm-dependent; each 4 h verdict is
n = 1; `K_hi` carries no external-fragmentation term and the redb page cache is unmodelled in
`reachable_est`, so `LEVEL` is biased upward by an amount `VS_SYS` cancels (two independent
reasons `BOUNDED_ABOVE_K` is not failure); macOS has no jemalloc background purge thread (R1.4), a
difference TODO-589 inherits; `DEFAULT_CANDIDATE` names the candidate for the Linux confirmation,
not a production default; `VS_SYS` is relative to macOS libmalloc and no "reduces footprint by N %"
sentence appears without that qualifier; the default is not called flipped. §3 Deferred carries the
CI clippy coverage hole, the measured CI cost of the jemalloc C build, any `SIZE_HEADROOM_*=TIGHT`,
and the two trackers the conductor opens at `/sf:done` (the default flip; per-arm clippy in CI).

### Runner diff: every hunk maps to one of the nine items
`diff spec371-memdiag.sh spec372-allocdiag.sh` has 36 hunks:

| hunks (by `diff` order) | item |
|---|---|
| 1 (the new header block), 2–6 (usage text), 7–8 (cell-table comment and `case`), 27 (matrix banner) | 1 — the cell table and the text that names it |
| 14, 15, 16, 17, 18, 19, 24, 25, 26, 28, 29 (`SPEC372_CODE_FREEZE` / `_HARNESS_BIN` / `_CHAIN_START_EPOCH` at every code site; the chain-name comment) | 2 — env names |
| 13 (`SPEC372_CODE_FREEZE=08cdef2f` and its placeholder) | 3 (+ 2) — the freeze literal |
| 9 (`BASE="spec372-${CELL}"`), 11 (data dir), 12 (port 47358) | 4 — basename, data dir, port |
| 30–36 (header comment, `CSV_HEADER`, the two probe parsers, the row `printf`) | 5 — CSV columns 46–55 |
| 20–23 (`MI_MARKER_LITERAL`, `JE_HITS`/`MI_HITS`, the flavour `case`, the FATAL text) | 6 — flavour markers |
| — | 7 — console line 1 keeps its shape; only `FLAVOUR`'s value set changes |
| 10 (the dead `READOUT_OUT` and its comment) | 8 — removed |
| — | 9 — `PE` lives in `spec372-predicates.sh` |

The smoke knobs `SPEC362B_SMOKE_DURATION`, `SPEC365_SMOKE_SAMPLE_INTERVAL` and
`SPEC371_SMOKE_LIVE_CENSUS`, and the `SPEC370_/SPEC371_BASE_SUFFIX` refusal, keep their parent
names (item 2).

### Artifacts beyond the spec's Delta list (derived, committed with the data)
`spec372.k-stage1.txt` and `spec372.k-stage2.txt` (k.awk's outputs, which decide.awk reads — each
re-derivable by re-running the frozen program); `spec372-builds2.txt` and `spec372-build2-<FL>.log`
(chain 2's own build record, so re-running Stage-1 predicates after chain 2 still matches chain 1's
shas); `spec372-<cell>.ampfp.csv` is in the Delta.

### The §1 prefix sha256 — the command (conductor rulings v5 R1)
`ORDER=OK` clause 2 compares this value at M, M2, D1, D2 and HEAD; every STOP quotes it, computed at
each named commit by exactly this command (the marker line is included in the hash):

```
git show <commit>:packages/server-rust/benches/soak_harness/evidence/spec372-manifest.md | sed '/^## APPEND-ONLY BELOW/q' | shasum -a 256
```

### Programs frozen at M (sha256; `ORDER=OK` clauses 3–4 re-check these bytes at M2, D1, D2, HEAD)
- `814c7a3b60dffaaf232df737d7d04ae84ead9eb4bdbe54a1552c749cb674cf6c` `packages/server-rust/benches/soak_harness/evidence/spec372-allocdiag.sh`
- `c2bf391dc470df6a6d9e134de16752337b915f56d027bc3171c29ae335302d8e` `packages/server-rust/benches/soak_harness/evidence/spec372-chain1.sh`
- `aa98986cb1b51e1380c457f70c60dc632304637a299a5b7fcc8448d32d4b1764` `packages/server-rust/benches/soak_harness/evidence/spec372-chain2.sh`
- `baf0bce7dd6c29751fb62f525153b631ec9eeb37f0ba858aed7174fd978b1c91` `packages/server-rust/benches/soak_harness/evidence/spec372-predicates.sh`
- `bada887b1b91ba3b4ee9c650e2ac6cf3368ca2815de1380f92196e05aa34aa3f` `packages/server-rust/benches/soak_harness/evidence/spec372-k.awk`
- `606e36f23895d87c3e33ad40cdc96bbe6ddb23789b45a21f9bb8d63be0b1fc0e` `packages/server-rust/benches/soak_harness/evidence/spec372-decide.awk`
- `d58fd6fa667fa04879340dabae64ab9486e1d4d5672e9afcfc488c7c7c10a81d` `packages/server-rust/benches/soak_harness/evidence/spec372-perf.sh`
- `722513c725167c211ddad58981ef7b41beb1939d0841291be710b74f39d6a0d6` `packages/server-rust/benches/soak_harness/evidence/spec372-buildstory.sh`

The parent programs `spec349c2-fit.awk`, `spec366-p5.awk`, `spec366-p67.awk`, `spec370-plateau4h.sh`,
`spec371-memdiag.sh`, `spec371-chain.sh`, `spec371-predicates.sh` and `spec371-decide.awk` are not edited.

## APPEND-ONLY BELOW

## STAGE-1 FROZEN READINGS (computed by spec372-k.awk at M2)

Computed over the Stage-1 artifacts committed at D1 = `b38d3a51` (M = `e92fa977`), from the repo root,
under `LC_ALL=C`, by the program bytes frozen at M:

```
E=packages/server-rust/benches/soak_harness/evidence
awk -v mode=stage1 -f $E/spec372-k.awk $E/spec372-{s1a,j1a,m1a,k1,j1b,m1b,s1b}.predicates.txt $E/spec372-k1.csv
```

Output, verbatim (byte-identical to the chain-1 artifact `spec372.k-stage1.txt`; `decide.awk stage1`,
re-run over the same D1 inputs, reproduces `spec372.stage1.txt` byte-for-byte and prints the same
`S1_SURVIVORS`):

```
S1_SURVIVORS=JE+MI
S1_RANK=JE
CHURN_RATIO=0.8721 points=15 window_s=480-900
CHURN_RATIO_DRIFT=0.8185
K_PROVISIONAL=FALSE
T_DECAY_UPPER_JE=8.7213
T_DECAY_UPPER_MI=0.8721
CV_JE=0.0913
CV_MI=0.0395
EST_AGREE_900_j1a=1.2385
EST_AGREE_900_j1b=1.1821
```

Program sha256 re-printed at M2 (`ORDER=OK` clause 4; identical to the list in section 1):

- `814c7a3b60dffaaf232df737d7d04ae84ead9eb4bdbe54a1552c749cb674cf6c` `packages/server-rust/benches/soak_harness/evidence/spec372-allocdiag.sh`
- `c2bf391dc470df6a6d9e134de16752337b915f56d027bc3171c29ae335302d8e` `packages/server-rust/benches/soak_harness/evidence/spec372-chain1.sh`
- `aa98986cb1b51e1380c457f70c60dc632304637a299a5b7fcc8448d32d4b1764` `packages/server-rust/benches/soak_harness/evidence/spec372-chain2.sh`
- `baf0bce7dd6c29751fb62f525153b631ec9eeb37f0ba858aed7174fd978b1c91` `packages/server-rust/benches/soak_harness/evidence/spec372-predicates.sh`
- `bada887b1b91ba3b4ee9c650e2ac6cf3368ca2815de1380f92196e05aa34aa3f` `packages/server-rust/benches/soak_harness/evidence/spec372-k.awk`
- `606e36f23895d87c3e33ad40cdc96bbe6ddb23789b45a21f9bb8d63be0b1fc0e` `packages/server-rust/benches/soak_harness/evidence/spec372-decide.awk`
- `d58fd6fa667fa04879340dabae64ab9486e1d4d5672e9afcfc488c7c7c10a81d` `packages/server-rust/benches/soak_harness/evidence/spec372-perf.sh`
- `722513c725167c211ddad58981ef7b41beb1939d0841291be710b74f39d6a0d6` `packages/server-rust/benches/soak_harness/evidence/spec372-buildstory.sh`

## §3 — carve 9b readout: no default candidate; the alternatives trade an unpredictable SYS residency for a stable one

Appended 2026-09-23, after D2 = `5ca6ed49` (`docs(soak): record the allocator evaluation Stage-2
cells (D2)`). Nothing above `## APPEND-ONLY BELOW` changed: the §1 command gives
`a4213fca45b2eb6d968d565f338c33a52634320a786ecff80535237ac457626b` at M `e92fa977`, D1 `b38d3a51`,
M2 `2f81851b` and D2 `5ca6ed49`; the eight frozen programs are byte-identical to M; each commit is an
ancestor of the next. `ORDER=OK`. Every number below was recomputed from the committed artifacts under
`LC_ALL=C`.

Chain 1 ran `2026-09-22T11:26:14Z` → `13:32:13Z`, chain 2 `2026-09-22T15:26:21Z` → `2026-09-23T03:59:02Z`,
each launched once; eleven `RUNNER_EXIT=0`, `CHAIN_RC=0` as the last line of both logs, `STOP=FALSE`
in both decision files with no `STOP-reason:` line. `k.awk` and `decide.awk`, re-run over the committed
D1 and D2 bytes, reproduce `spec372.k-stage1.txt`, `spec372.stage1.txt`, `spec372.k-stage2.txt` and
`spec372.decision.txt` byte-for-byte (AC-6, AC-6b).

### 3.1 The decision, verbatim

```
S1_SURVIVORS=JE+MI
S1_RANK=JE
VERDICT_JE=NOT_BOUNDED
VERDICT_MI=BOUNDED_NO_GAIN
PERF_JE=FAIL
PERF_MI=FAIL
BUILD_JE=OK
BUILD_MI=OK
DEFAULT_CANDIDATE=NONE
NEXT=CONDUCTOR_RULING;BOUNDED_NO_GAIN
```

The full flag block is `spec372.decision.txt`. **The default allocator is not flipped**: `default =
["redb"]` is untouched, and `alloc-jemalloc` / `alloc-mimalloc` land as off-by-default features — the
instrument for the next two carves, not a production choice. **Every Stage-2 verdict is n = 1 per arm**
(one 4 h cell each, one same-chain SYS reference).

### 3.2 The four 4 h cells, with the committed 8f SYS cell on the same matrix

`FP` = `phys_footprint_mb`; `S` = `FP + reclaimable_mb`; `reachable_est` and `AMP_*` are §1's
definitions at the TERMINAL census. 8f is `spec370-plateau4h.csv` (SPEC-370's 4 h cell, the same soak
matrix); its `reachable_est` is §1's 901.82 MiB, from its terminal census `live=1047614`.

| 4 h cell | FP at 2 h | FP at 4 h | reclaimable at 2 h → 4 h | reclaimable, max over 60 s rows | S at 4 h | reachable_est | AMP_FP | AMP_S |
|---|---|---|---|---|---|---|---|---|
| `s2` SYS | 362 MB | 418 MB | 6421 → 2412 MB | 6834 MB (t = 8100 s) | 2830 MB | 893 MB | 0.47 | 3.17 |
| 8f SYS (committed) | 4940 MB | 8237 MB | 4035 → 1130 MB | 4067 MB (t = 6960 s) | 9367 MB | 902 MB | 9.13 | 10.39 |
| `j2` JE | 996 MB | 2029 MB | 0 → 0 | 0 | 2029 MB | 918 MB | 2.21 | 2.21 |
| `m2` MI | 1111 MB | 1905 MB | 114 → 215 MB | 324 MB (t = 7740 s) | 2120 MB | 920 MB | 2.07 | 2.31 |

Terminal joins: `s2` 1.8 s, `j2` 2.2 s, `m2` 1.6 s. Ops parity held on the deciding cells:
`OPS_RATIO_JE_S2=1.0284`, `OPS_RATIO_MI_S2=1.0302` (s2 180.2, j2 185.3, m2 185.7 ops/s); `WRITE_ERRORS=0`.

### 3.3 The SYS reference is bimodal at 4 h

Two SYS cells on the same matrix end **20× apart on `FP`** (418 vs 8237 MB) and **3.3× apart on `S`**
(2830 vs 9367 MB). In `s2`, libmalloc parked up to 6.8 GB in reusable pages and then returned about
4.4 GB of them by 4 h; `FP` stayed near 0.5 × reachable. In 8f, the same allocator re-dirtied its
reusable pages after ~2 h and `FP` climbed to 9.1 × reachable. Nothing in the configuration distinguishes
the two cells. On this workload the macOS SYS residency at 4 h is therefore a draw from at least two
regimes, not a number, and one SYS cell per chain cannot say which regime is typical.

### 3.4 `VS_SYS` and `VS_SYS_S`, side by side — neither decides

| arm | `VS_SYS` (`AMP_FP` / `AMP_FP_S2`) | `VS_SYS_S` (`AMP_S` / `AMP_S_S2`) |
|---|---|---|
| JE | 4.7210 `NO_GAIN` | 0.6977 |
| MI | 4.4244 `NO_GAIN` | 0.7275 |

The two accountings point in opposite directions, and **neither is a statement about the allocators**:
both divide by a single draw of the bimodal SYS reference of 3.3, and `s2` landed in the low-`FP`,
high-reclaimable regime. The committed 8f cell, in the other regime, would move both ratios by more
than an order of magnitude; that cross-chain division is not computed here, because §1 forbids it as a
verdict and the reference's own 20× spread (§3.3) already says what it would show. Both ratios are
relative to **macOS libmalloc**, a baseline that does not exist on the Linux/glibc production target.
This readout makes no footprint-reduction claim relative to SYS.

### 3.5 Why §1's `K_HI_VACUOUS=FALSE` prediction missed

§1 predicted `AMP_FP_S2 = 9.13` — the 8f cell's value — hence a `BETTER` bar of 4.57 and
`K_HI_VACUOUS=FALSE` for both arms (`K_hi` 1.53 / 1.43 far below the bar). `s2` read
`AMP_FP_S2 = 0.4683`, so the bar is `0.50 × 0.4683 = 0.234`, and both `K_hi` (1.5525 / 1.4270, within
0.03 of the prediction) sit above it: `K_HI_VACUOUS_JE=TRUE`, `K_HI_VACUOUS_MI=TRUE`. The bounds were
predicted correctly; the reference they are compared against was one draw of §3.3's bimodal SYS. The
guard did what it exists for — it announced that `LEVEL` cannot discriminate on this chain, and both
`LEVEL` readings (`ABOVE_K_HI`) carry no information here. Independently of that, and as AC-11 requires:
`K_hi` carries no external-fragmentation term and the redb page cache is unmodelled in `reachable_est`
(`R_redb` 0.23–0.24 enters `K_hi` off the file size), so `LEVEL` is biased upward by an amount `VS_SYS`
would cancel; `ABOVE_K_HI` would not have read as failure even with a non-vacuous bound.

### 3.6 What the two alternatives do instead: a stable ≈ 2.1–2.2 × reachable

- **Both arms track the live set.** From 2 h to 4 h `reachable_est` grows ≈ 230 MB/h; `FP` grows
  ≈ 516 MB/h on JE and ≈ 397 MB/h on MI, and `AMP_FP` stays at 2.18 → 2.21 (JE) and 2.38 → 2.07 (MI).
  Last-half trends: `TREND_JE=INCREASING` (+0.053 ± 0.022 AMP/h, n = 24), `TREND_MI=NON_INCREASING`
  (−0.114 ± 0.040, n = 24); both last-third fits read `INCREASING` (+0.122 / +0.112, recorded, not
  gating). JE's arm-native ratio `FP / stats.allocated` is flat over 120 points: `TREND_JE_NATIVE`
  −0.030 ± 0.015 /h.
- **The estimator holds at the deciding timescale.** `JE_ESTIMATOR_AGREE=TRUE ratio=1.0735`
  (`EST_PROVISIONAL=FALSE`), against 1.24 / 1.18 at 900 s: the Stage-1 under-count of ~20 % closes to
  7 % at 4 h; `UNMODELLED_REACHABLE_j2` = 67.5 MB (0.073).
- **Where JE's 2.2× goes (measured, not bounded).** `FRAG_SHARE` = (`stats.active − stats.allocated`) /
  `stats.allocated`: j1a 0.853, j1b 0.762, **j2 0.808** — active pages are ≈ 1.8 × allocated.
  `DIRTY_SHARE` = (`stats.resident − stats.active − stats.metadata`) / `stats.allocated`: j1a 0.265,
  j1b 0.213, **j2 0.221**. `AMP_JE` (`stats.resident / stats.allocated`) = 2.079. So most of the excess
  is page-level fragmentation of large, growing records that are cloned about once a second
  (`CHURN_RATIO` 0.8721 /s, drift 0.8185, `K_PROVISIONAL=FALSE`), with dirty-page decay the smaller term.
  `T_DECAY_UPPER_JE` 8.72 / `_MI` 0.87 are the no-reuse bounds and are recorded only.
- **Reclaim semantics confirmed from the run on all three arms** (R5.2's table): `RECLAIM_SEMANTICS_SYS=
  REUSABLE ratio=5.77`, `RECLAIM_SEMANTICS_MI=REUSABLE ratio=0.11`, `RECLAIM_SEMANTICS_JE=MADV_FREE
  ratio=0.000000` — JE returns decayed pages outright, which is also why `S` means something different
  per arm and could never be the cross-arm numerator. On macOS jemalloc has no background purge thread
  at any setting (R1.4; `je_config` `opt_background_thread=false`, `arenas_narenas=41`), so decay runs
  only on allocation paths; TODO-589 inherits that difference.

### 3.7 Performance beside ops parity

| reading | JE | MI | SYS |
|---|---|---|---|
| in-process load harness, fire-and-wait median p99 (5 × 30 s) | 17.1 ms | 15.4 ms | 6.5 / 5.8 ms (two blocks) |
| in-process load harness, fire-and-wait median ops/s vs SYS low | 0.93 | 0.96 | 1.00 |
| in-process load harness, fire-and-forget median ops/s vs SYS low | 1.00 | 1.00 | 1.00 |
| `PERF_<arm>` | **FAIL** `breached=faw_p99` | **FAIL** `breached=faw_p99` | `SYS_QUIET=TRUE` |
| out-of-process soak, `OPS_RATIO_<arm>_S1` (900 s) | 1.0064 | 1.0144 | — |
| out-of-process soak, `OPS_RATIO_<arm>_S2` (4 h) | 1.0284 | 1.0302 | — |

The in-process harness runs client and server in one process under one global allocator (200
connections, many small short-lived allocations, 41 jemalloc arenas), so its p99 leg measures the
harness's allocation profile together with the server's; the soak's paced workload shows parity. The
two readings are printed together because neither alone settles the cost; an out-of-process perf
reading is TODO-696. An earlier perf run on a loaded host (Docker Desktop still up) read the opposite
direction and was excluded under the idle-host rule; it is not part of this evidence. `a2j` (JE, journal
OFF, 900 s): `OPS_a2j=183.044`, `OPS_RATIO_a2j_vs_4h=1.0998`, `AMP_FP=2.456` at its TERMINAL
(`join_lag_s=60.2`, inside the 90 s bound) — n = 1 per side, early window, no causal claim about the
journal.

### 3.8 Stage 1 was a screen and nothing more

The 900 s cells are an early-window screen that never decides. Both arms survived the DROP rule on the
`S` leg alone: on `FP_end` both were above the worst SYS cell in both replicates (JE 180 / 166, MI 252 /
273 vs 84.5 MB), on `S_end` both were below it (JE 180 / 166, MI 276 / 301 vs 546 MB). `S1_RANK=JE`
(mean `AMP_FP` 2.49 vs 3.80) selected `a2j` and nothing else. `CV_JE=0.0913`, `CV_MI=0.0395`.

### 3.9 Build story and binary size

`BUILD_JE=OK`, `BUILD_MI=OK`: darwin-arm64, linux-x64 (`cargo zigbuild`, the npm path) and the Docker
image (linux/aarch64, no new apt package) build for both arms; the 16-subset feature lattice checks;
`cargo audit` names no allocator crate. linux-x64 stripped sizes: SYS 20 994 088 B, JE +566 536 B,
MI +190 168 B ⇒ `SIZE_HEADROOM_JE=TIGHT`, `SIZE_HEADROOM_MI=TIGHT` — the SYS binary at this base is
already over the 20 MiB publish ceiling before any allocator (TODO-695), so TIGHT is not a finding
against either arm. CI cost of jemalloc's C build: 27.3–31.7 s of `./configure` + `make`, overlapped
with the Rust compile; cold `cargo clippy --all-targets --all-features` measured at 82 s (HEAD) vs 97 s
(pin) in one run and +6 / +9 s in the smoke runs — within this host's noise.

### 3.10 Deferred

- **TODO-591** (carve 9c) — reduce the per-write clone churn behind both SYS's retention and JE/MI's
  ≈ 0.8 fragmentation; measured on SYS **and** JE with the features built here. First.
- **TODO-589** — the allocator decision on Linux/glibc, the production target: ≥ 2 SYS replicates at
  4 h (the reference is bimodal, §3.3), an out-of-process perf reading, and the size ceiling fixed first.
  `DEFAULT_CANDIDATE=NONE` here, so this carve hands it no candidate; it hands it the instrument.
- **TODO-695** — the linux-x64 SYS binary is over the 20 MiB publish ceiling at this base; both
  `SIZE_HEADROOM_*=TIGHT` readings are consequences of it.
- **TODO-696** — an out-of-process perf reading, so the p99 leg of `PERF_<arm>` stops sharing one
  allocator between client and server.
- **Default-flip tracker** — to be opened by the conductor at `/sf:done`: the `DEFAULT_FLIP` token in
  NEXT clause 16 names no tracker today; any flip goes through TODO-589 and at least one replicate 4 h
  cell of the candidate.
- **CI per-arm clippy tracker** — to be opened by the conductor at `/sf:done`: CI runs
  `cargo clippy --all-targets --all-features` only (`.github/workflows/rust.yml:93`); under the cfg
  lattice `alloc-jemalloc` outranks `alloc-mimalloc`, so the MI-only row is never linted in CI (the
  build story ran it locally: `ITEM4 clippy=MI rc=0`). CI's test job runs default features
  (`rust.yml:96`) and never pays the jemalloc C build; only clippy `--all-features` does.
