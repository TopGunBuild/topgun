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
- **PERF_<arm>** (`spec372-perf.sh`, CI's perf-gate parameters, 3 runs per mode, blocks SYS → JE →
  MI → SYS): `FAIL` iff, in either mode, the median ops/s < 0.80 × the FIRST SYS block's or the
  median p99 > 1.20 × SYS's (p99 compared only where SYS's p99 is non-zero; fire-and-forget reports
  p99 = 0); `n/a reason=harness_noisy` iff the two SYS blocks' medians differ by ≥ 20 % on ops/s or
  p99 in either mode (Validation item 5: a harness that noisy cannot disqualify); `n/a
  reason=run_failed` iff a run reported no numeric field; `PASS` otherwise.
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

### Programs frozen at M (sha256; `ORDER=OK` clauses 3–4 re-check these bytes at M2, D1, D2, HEAD)
- `814c7a3b60dffaaf232df737d7d04ae84ead9eb4bdbe54a1552c749cb674cf6c` `packages/server-rust/benches/soak_harness/evidence/spec372-allocdiag.sh`
- `c2bf391dc470df6a6d9e134de16752337b915f56d027bc3171c29ae335302d8e` `packages/server-rust/benches/soak_harness/evidence/spec372-chain1.sh`
- `aa98986cb1b51e1380c457f70c60dc632304637a299a5b7fcc8448d32d4b1764` `packages/server-rust/benches/soak_harness/evidence/spec372-chain2.sh`
- `baf0bce7dd6c29751fb62f525153b631ec9eeb37f0ba858aed7174fd978b1c91` `packages/server-rust/benches/soak_harness/evidence/spec372-predicates.sh`
- `bada887b1b91ba3b4ee9c650e2ac6cf3368ca2815de1380f92196e05aa34aa3f` `packages/server-rust/benches/soak_harness/evidence/spec372-k.awk`
- `606e36f23895d87c3e33ad40cdc96bbe6ddb23789b45a21f9bb8d63be0b1fc0e` `packages/server-rust/benches/soak_harness/evidence/spec372-decide.awk`
- `98660dfde3ce9c6eecb3b20e6c2c277818e4e511c08475986d2f743adb6a0859` `packages/server-rust/benches/soak_harness/evidence/spec372-perf.sh`
- `722513c725167c211ddad58981ef7b41beb1939d0841291be710b74f39d6a0d6` `packages/server-rust/benches/soak_harness/evidence/spec372-buildstory.sh`

The parent programs `spec349c2-fit.awk`, `spec366-p5.awk`, `spec366-p67.awk`, `spec370-plateau4h.sh`,
`spec371-memdiag.sh`, `spec371-chain.sh`, `spec371-predicates.sh` and `spec371-decide.awk` are not edited.

## APPEND-ONLY BELOW
