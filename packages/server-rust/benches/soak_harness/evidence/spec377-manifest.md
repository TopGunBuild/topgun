# SPEC-377 — Linux OR-churn memory plateau and allocator candidate on topgun-bench: pre-registration manifest

## §1 Pre-registration (frozen at commit M; nothing above the APPEND-ONLY marker changes after M)

> **Draft status.** This §1 is the G1 contract draft: the R0 contract (CSV header and memory columns, column-0
> literals, runner console keys, flags order, cell table with runtime allocator env, absence rule), the closed
> `NEXT` literal list and the conductor rule, the predicted readings and the budget. Sections marked *to fill* are
> written by the group named there and completed before M is frozen (G5, after the smoke). Until M exists nothing
> here is frozen, and no reading may cite this file as pre-registered.

### Question and scope
On the production OS (Linux/glibc) and a quiet dedicated host (Hetzner CCX23 `topgun-bench`, 4 dedicated vCPU /
16 GB, Debian 12, created from snapshot `437171250`), under the SPEC-376 OR-churn workload run for 6 h per cell:

1. Does the server's resident memory **per live entry** plateau under the default allocator (glibc malloc, `SYS`),
   judged by two SYS replicates that bracket the series?
2. Which allocator, if any, should the follow-up evaluate as the default candidate: jemalloc with background threads
   (`JE`), mimalloc v3 (`MI3`) or mimalloc v2 (`MI2`) — each treatment proven from the running process?

The series changes no server code and does not change the shipped default allocator. Out of scope: out-of-process
performance (TODO-696), the `je_config` decay/retain printout (TODO-717), the linux-x64 binary size ceiling
(TODO-695), the allocator flip itself, Linux `per_op` fsync (TODO-560), the 72 h soak (TODO-484), E calibration
(TODO-709), glibc-tuned arms (TODO-719). Every `NEXT` literal that names a flip routes through TODO-696 first.

### Lineage
A Linux host is its own baseline; only SPEC-376's Linux constants are used (`A0_L_MIB`, `WRITES_PER_S_REF=257.647`,
the SPEC-376 per-row spread); **no M1 number is a threshold**. SPEC-372's 4-hour M1 cells enter this manifest only
as the input of the *predicted* readings (below), which are expectations, never gates. `SERIES_PIN` is the hex of
`origin/main` at spec time; its server inputs are byte-identical to SPEC-376's `CAL_PIN` `bee21fcd` over
`packages/server-rust/src`, `packages/server-rust/Cargo.toml` and `packages/core-rust` (docs-only diff).

### R0.2 Literals (column 0; each exactly once in §1)
SERIES_PIN=34007e19
A0_L_MIB=13.349 src=SPEC-376-calib-c1c2-t60
FLAT_BAR_PER_H=0.010
BETTER_BAR=0.80
SYS_AGREE_BAR=0.25
OPS_PARITY_MIN=0.95
TIE_BAND=0.10
STAGE2_MAX_H=24
LEVEL_WINDOW_S=1800
MI_POSTINIT_BOUND=5
LAZY_DRIFT_BAR=0.01
PRICE_EUR_PER_H=0.138

- `SERIES_PIN` — the hex of **`origin/main`** when the spec branch was created
  (`git rev-parse origin/main` = `34007e197224d4db870ba2c842716316622580de`, the merge of #174); never the local
  `main` ref (stale at `bee21fcd` on the conductor's Mac) and never a movable name. The spec branch
  `spec-377-linux-allocator` was created from exactly this commit.
- `A0_L_MIB` — SPEC-376 `spec376-calib.txt` §3.1 (`A0_L_MIB=13.349`, CA pair c1/c2 at t = 60 s). Read from here by
  every program; never a literal in a program (AC-14a).
- `FLAT_BAR_PER_H` — `B`, the equivalence bar on the per-entry trend, as a fraction per hour (1 %/h; R5.7).
- `BETTER_BAR` — an arm is `BETTER` iff `PE_LEVEL_arm / min(PE_LEVEL_s1, PE_LEVEL_s2) ≤ 0.80` (R7.5).
- `SYS_AGREE_BAR` — the two SYS replicates agree iff `|PE_LEVEL_s1 − PE_LEVEL_s2| / mean ≤ 0.25` (≈ 2× SPEC-376's
  only Linux same-binary spread `S_CA_FPL=0.137`, a count-alloc pair at 900 s; SYS over 6 h is unmeasured on Linux).
- `OPS_PARITY_MIN` — an arm whose `OPS_RATIO` is below 0.95 gets no verdict (`n/a reason=ops`).
- `TIE_BAND` — set-based tie: every candidate within 10 % of the minimum level, then preference `JE, MI3, MI2`.
- `STAGE2_MAX_H` — the largest Stage-2 cell length (hours) `STAGE2_FEASIBLE` accepts.
- `LEVEL_WINDOW_S` — `PE_LEVEL` = mean of `pe` over rows with `elapsed_secs ∈ [D − 1800, D)`.
- `MI_POSTINIT_BOUND` — more post-init `mimalloc` console lines than this in an MI cell ⇒ a recorded
  `MI_POSTINIT_NOTE`, never a STOP.
- `LAZY_DRIFT_BAR` — `LAZY_DRIFT=TRUE` iff any series cell's `LAZY_MAX / anon` exceeds 0.01 (a config-drift canary;
  recorded).
- `PRICE_EUR_PER_H` — CCX23 on-demand price, used for `STAGE2_COST_EUR` and the budget below.

### R0.1 CSV header and memory columns
SPEC-376's 61-column header (`spec376-manifest.md` §1 R0.1), **unchanged in name and position**, produced by the
frozen `spec376-procmem.sh` sourced unchanged (its sha256 asserted at source time, listed under "Frozen parents"
below). The literal, verbatim:

```
elapsed_secs,rss_mb,wal_mb,redb_mb,disk_total_mb,tombstone_bytes,fp_equiv_mb,hwm_rss_mb,lazyfree_mb,swap_mb,conj_snapshots_total,conj_current_epoch,conj_ceiling,conj_durable_watermark,durable_watermark_lag,claims,claim_lag_p50,claim_lag_p99,claim_lag_max,ret_epochs_claim_only,ret_epochs_durability_only,ret_epochs_both,ret_epochs_neither,ret_refs_claim_only,ret_refs_durability_only,ret_refs_both,ret_refs_neither,ret_stamped_bytes,ret_epochs_unslotted,ret_refs_open_epoch,ret_stamped_bytes_open_epoch,indexed_refs,considered_total,dropped_total,matched_nothing_total,absent_total,bytes_freed_total,removed_refs_observed_total,removed_bytes_observed_total,stamped_bytes_total,file_mb,alloc_live_bytes,alloc_live_mb,alloc_probe_elapsed_s,alloc_probe_seq,je_allocated,je_active,je_resident,je_retained,je_mapped,je_metadata,je_probe_elapsed_s,je_probe_seq,bytes_alloc,bytes_dealloc,anon_mb,private_dirty_mb,pss_mb,anon_huge_mb,smaps_rss_mb,smaps_read_ms
```

The eleven **memory columns**, in header order: `rss_mb, fp_equiv_mb, hwm_rss_mb, lazyfree_mb, swap_mb, file_mb,
anon_mb, private_dirty_mb, pss_mb, anon_huge_mb, smaps_rss_mb` (`smaps_read_ms` is a cost column). Their
definitions, the row invariants (`0 ≤ LazyFree ≤ Anonymous`, `Anonymous − LazyFree + Swap ≤ Rss + Swap`, counted in
`mem_invariant_violations=`) and the fatal-on-missing source fields are SPEC-376's R0.1/R0.2, unchanged.

**The flagged quantity** is `fp_equiv_mb` (= `Anonymous − LazyFree + Swap`) per live entry,
`pe = (fp_equiv_mb − A0_L_MIB) × 1048576 / live(t)` (R5.5); `rss_mb` gives the recorded twin `rss_pe`. Neither an
absolute resident slope nor a single point is a gate.

### Row and census accounting (R5.4–R5.7 expectations derived from R0.5)
- Duration `D` = 21600 s, cadence 60 s ⇒ 361 CSV rows expected per series cell (`elapsed_secs` 0 … 21600), of which
  the rows with `elapsed_secs < D` are **live** rows; the `elapsed = D` row is never a census join target.
- Live-copy census every 300 s ⇒ 72 census instants per cell, plus `TERMINAL` (taken at `t ≈ D + 2 s`).
- One census join rule, `TERMINAL` included: nearest live row with a non-empty `fp_equiv_mb` within ± 30 s,
  otherwise dropped and named (`CENSUS_DROPPED_<cell>=<t>`, counted in `CENSUS_DROPPED_N_<cell>=`). `TERMINAL` is
  therefore dropped in every cell by construction (the last live row is at `D − 60 s`), so `CENSUS_DROPPED_N ≥ 1` is
  expected, not a defect.
- `live(t)` for a row is the linear interpolation of the two retained census counts around it; rows outside the
  first/last retained census are not in the series. The fit window is the frozen fitter's own last half
  (`spec349c2-fit.awk:128`, `start = int(n / 2)` over the numeric `pe.csv` rows), ≈ 180 rows expected; fewer than
  90 ⇒ `TREND=n/a reason=few_rows`.

### R0.3 Runner console keys (`spec377-cells.sh`, per cell)
**Inherited unchanged from SPEC-376** (`spec376-manifest.md` §1 "Runner console contract"): harness-console line 1
provenance `provenance: server sha256=<hex> flavour=<SYS|JE|MI> built=… harness sha256=<hex> …`; the closing three
lines `steal_pct=<%.4f|n/a>`, `post_mortem_mem_reads=<n>`, `mem_invariant_violations=<n>` (once each, in that order,
on every run that got past launch); `SAMPLER FATAL: …` on a blind sampler; the chain appends `RUNNER_EXIT=<rc>`.

**Added**, exactly once each on every run that got past launch (`<cell>` ∈ `ssy sje smi3 smi2 s1 je mi3 mi2 s2`):

| key | value | source |
|---|---|---|
| `ALLOC_ENV_<cell>=` | `none` or `k=v[;k=v…]`, sorted | `/proc/<pid>/environ` entries whose name is in the R2.3 unset list or matches `^MIMALLOC_`, read at the first sample with `elapsed ≥ 60 s`; an unreadable `environ` on a live pid is `SAMPLER FATAL`, never an empty value |
| `JE_BG_THREADS_<cell>=` | integer | count of `/proc/<pid>/task/*/comm` equal to `jemalloc_bg_thd`, same instant (SYS/MI expect 0) |
| `THP_<cell>=` | `<enabled>/<defrag>` | the bracketed values of the host THP knobs at launch |
| `ALLOC_PROOF_AT_<cell>=` | elapsed s | when the proof above was read |
| `BIN_MARKERS_<cell>=` | `je=<n> mi=<n>` | counts, in `strings` of the LAUNCHED binary (the file whose sha256 is on console line 1), of the jemalloc literal `<jemalloc>: malloc_conf #` and the mimalloc literal `mimalloc: ` |
| `MI_POSTINIT_LINES_<cell>=` | integer | harness-console lines containing `mimalloc` or matching `^\[server\] option '` after the last option line of the startup block (every cell) |

The server's `tracing` boot line (`allocator=`) is **not** an input: the runner's `SOAK_SERVER_LOG` filters it out
(no committed SPEC-376 console contains `allocator=`). Every console regex is matched against the harness console,
where server stderr carries the prefix `[server] `, and allows trailing spaces (` *$`). The allocator proof items
(`PALLOC`, R3.3: SYS `s1`–`s6` + `marker_control`, JE `j1`–`j6`, MI `m1`–`m6`, `mi_version`, `mi_thread_prefix`)
are the G2a/G3 contract and are frozen here at M with the programs.

### R0.4 Flags block (`spec377.decision.txt`, in this order, after `STOP=` and every STOP clause)
1. `STOP=`
2. `WRITE_ERRORS=`
3. `OPS_<cell>=` — five: `s1 je mi3 mi2 s2`
4. `OPS_RATIO_<arm>=` — three: `JE MI3 MI2`
5. `LIVE_END_<cell>=`
6. `PE_END_<cell>=` — census, recorded
7. `PE_LEVEL_<cell>=`
8. `TREND_<cell>=` — class with `slope_rel`, `se_rel` naive, `r1`, `se_rel_adj`, n rows, r², census points dropped
9. `TREND3_<cell>=` (last third, recorded) + `TREND_CORR_<cell>=` (recorded, R5.9)
10. `STAGE2_T_<cell>=`
11. `FIXED_EST_<cell>=` + `FIXED_NOTE_<cell>=` — recorded only, R5.9
12. `RSS_PE_LEVEL_<cell>=`
13. `LAZY_MAX_<cell>=` + `LAZY_DRIFT=`
14. `HWM_END_<cell>=`, `ANON_HUGE_END_<cell>=`
15. `JE_CONFIG=` (verbatim) + `JE_CONFIRM_CONF=`
16. `AMP_JE=`, `FRAG_SHAREL_je=`, `DIRTY_SHAREL_je=`, `REACH_PE_je=`
17. `MI_POSTINIT_LINES_<mi3|mi2>=` + `MI_POSTINIT_NOTE=`
18. `DISK_SLOPE_<cell>=` — wal + redb, recorded
19. `SYS_AGREE=`
20. `PLATEAU_SYS=`
21. `STAGE2_FEASIBLE=` + `STAGE2_COST_EUR=`
22. `VS_SYS_<arm>=`
23. `ORDER_AGREE=`
24. `VERDICT_<arm>=`
25. `DEFAULT_CANDIDATE=`
26. `NEXT=`
27. `CONDUCTOR_RULE=` — the last key (R8.1)

Each key exactly once. When `STOP≠none` every decision flag (items 19–27) reads `STOP`; the readings (1–18) are
still printed. **Decision inputs vs recorded readings:** only `TREND_*`, `PE_LEVEL_*`, `RSS_PE_LEVEL_*` (for
`ORDER_AGREE`), `OPS_*`, `STAGE2_T_*` and the SYS-derived flags route; `PE_END`, `TREND3`, `TREND_CORR`,
`FIXED_EST`, `FIXED_NOTE`, `LAZY_*`, `HWM_*`, `ANON_HUGE_*`, the JE-native readings, `MI_POSTINIT_*` and
`DISK_SLOPE` are descriptive and never enter `VERDICT`, `DEFAULT_CANDIDATE`, `NEXT` or `CONDUCTOR_RULE`.

`TREND` domain (first match, `s` = slope / `PE_LEVEL` per hour, `e` = AR(1)-adjusted relative se, `B` =
`FLAT_BAR_PER_H`): `n/a reason=few_rows` · `n/a reason=pe_nonpositive` · `n/a reason=fit_mismatch` (program slope vs
the frozen fitter's `%.6f` print differ by more than 5e-7) · `RISING` (`s − 2e > B`) · `FALLING` (`s + 2e < −B`) ·
`FLAT` (`|s| + 2e ≤ B`) · `UNDERPOWERED` (`2e > B`) · `MARGINAL` (otherwise). A `TREND` of `n/a` (any reason) and an
absent `TREND` line are the same thing to decide: `missing:<cell>:TREND`.

### R0.5 Cell table and runtime allocator env

| cell | phase | duration s | cadence s | label | build | runtime allocator env | teardown | role |
|---|---|---|---|---|---|---|---|---|
| ssy | smoke | 120 | 20 | `SYS-ser` | no feature | none | SIGKILL | admission |
| sje | smoke | 120 | 20 | `JE-ser` | `--features alloc-jemalloc` | `_RJEM_MALLOC_CONF=background_thread:true,confirm_conf:true` | SIGKILL | admission |
| smi3 | smoke | 120 | 20 | `MI3-ser` | `--features alloc-mimalloc` | `MIMALLOC_VERBOSE=1` | SIGKILL | admission |
| smi2 | smoke | 120 | 20 | `MI2-ser` | `--features alloc-mimalloc,mimalloc/v2` | `MIMALLOC_VERBOSE=1` | SIGKILL | admission |
| s1 | series | 21600 | 60 | `SYS-ser` | as ssy | none | SIGKILL | SYS replicate 1 |
| je | series | 21600 | 60 | `JE-ser` | as sje | as sje (incl. `confirm_conf:true`) | SIGKILL | candidate |
| mi3 | series | 21600 | 60 | `MI3-ser` | as smi3 | as smi3 | SIGKILL | candidate |
| mi2 | series | 21600 | 60 | `MI2-ser` | as smi2 | as smi2 | SIGKILL | candidate |
| s2 | series | 21600 | 60 | `SYS-ser` | as ssy | none | SIGKILL | SYS replicate 2 |

- **Builds** (all at `SERIES_PIN`, fresh target dirs): `SYS-ser`, `JE-ser`, `MI3-ser`, `MI2-ser` =
  `--release --bin topgun-server` with the feature string above; harness `H` = `--release --bench soak_harness`.
  Before every build the chain unsets `JEMALLOC_SYS_WITH_MALLOC_CONF` and
  `X86_64_UNKNOWN_LINUX_GNU_JEMALLOC_SYS_WITH_MALLOC_CONF` and prints `BUILD_ENV_MALLOC_CONF=unset`.
- **Label map** (PV and flavour prefix): `ssy s1 s2 → SYS-ser (SYS)`, `sje je → JE-ser (JE)`,
  `smi3 mi3 → MI3-ser (MI)`, `smi2 mi2 → MI2-ser (MI)`, harness `H`.
- **Allocator env discipline** (before launch, every cell): `unset MALLOC_CONF _RJEM_MALLOC_CONF MALLOC_ARENA_MAX
  MALLOC_ARENA_TEST MALLOC_TOP_PAD_ MALLOC_TRIM_THRESHOLD_ MALLOC_MMAP_THRESHOLD_ MALLOC_MMAP_MAX_ MALLOC_PERTURB_
  MALLOC_CHECK_ GLIBC_TUNABLES LD_PRELOAD` and every exported name matching `^MIMALLOC_` (via `compgen -e`); then
  export exactly the runtime env of the row. No glibc tunable in any arm; jemalloc decay at defaults; THP pinned
  `madvise` by the preflight for the whole series.
- **Series order** `s1 → je → mi3 → mi2 → s2` (SYS brackets the series so host drift shows as `SYS_AGREE`); smoke
  order `ssy → sje → smi3 → smi2`. A settle step before every series cell after the first (1-min load < 0.5 or
  10 min; `SETTLE_<cell>=`, recorded, not a STOP).
- **Matrix literals** (SPEC-376's, unchanged): churn-clients 6, keyspace 200, or-churn true, or-keyspace 48,
  or-every 5, write-interval-ms 20, writes-per-life 200, offline-keys 3, confirm-interval 2, steady-interval 300,
  quiesce 3, mem-sample-interval 5, wal-fsync batched, mem gate neutralised, jitter seed 20260831, live-copy census
  at 300 s, journal default.
- **Port** 47376. **Paths** (relative to `/opt/topgun`): data dirs `target/spec377-<cell>-data`; target dirs
  `target/spec377-<label>`; builds file `target/spec377-run/spec377-builds.txt`; smoke OUT
  `target/spec377-run/smoke`; series evidence in `packages/server-rust/benches/soak_harness/evidence/`.

### R0.6 Absence rule (normative for every gate) — SPEC-376 R0.6, verbatim
No gate reads an absent, empty or non-numeric input as a pass; absence is FALSE and is named. A gate over a line
keyed `KEY=` passes only if exactly one line matches `^KEY=` and its value satisfies the gate; zero occurrences, a
duplicate, an empty value, or a non-numeric value where a number is required make it FALSE, named `=absent`,
`=dup`, `=<value>` or `=missing`. A gate over a set of rows is FALSE when the set is empty.

Applied here additionally (R7.5): every number decide compares is read through one `num()` helper that returns
"absent" for an empty, `n/a…` or non-numeric field, and a comparison is evaluated only on two numbers; every
SYS-derived quantity (`SYS_AGREE`, `PLATEAU_SYS`, `VS_SYS_<arm>`, `OPS_RATIO_<arm>`) needs **both** `s1` and `s2`
present and valid, never one survivor; a missing input prints `reason=missing:<cell>:<field>`, an input present but
outside its printed domain `reason=out_of_domain:<cell>:<field>`.

### NEXT — the closed literal list (R8; first match, order load-bearing)

| # | condition | `NEXT` |
|---|---|---|
| 1 | `STOP ≠ none` | `STOP` |
| 1a | any flag carries `reason=out_of_domain:…`, or any `VERDICT_<arm>=n/a reason=unmatched` | `CONDUCTOR_RULING;UNMATCHED` |
| 2 | `PLATEAU_SYS = INDETERMINATE reason=missing:…` | `CONDUCTOR_RULING;SYS_MISSING` |
| 3 | `PLATEAU_SYS = INDETERMINATE reason=disagree` | `CONDUCTOR_RULING;SYS_BIMODAL;SEE=TODO-719` |
| 4 | every arm `VERDICT = n/a` with `reason=ops` or `reason=missing:<cell>:OPS` | `CONDUCTOR_RULING;OPS` |
| 5 | `DEFAULT_CANDIDATE ∈ {JE, MI3, MI2}` | `TODO-696+TODO-695;THEN;DEFAULT_FLIP=<arm>` |
| 6 | `C` non-empty ∧ `ORDER_AGREE ≠ TRUE` | `CONDUCTOR_RULING;ARM_ORDER_ACCOUNTING` |
| 8 | `DEFAULT_CANDIDATE = SYS` | `KEEP_SYSTEM;SYS_FLAT_WITHIN_BAR` |
| 9 | `PLATEAU_SYS = UNDERPOWERED` ∧ `STAGE2_FEASIBLE = TRUE` | `STAGE2;SYS_LONG_CELLS` |
| 10 | `PLATEAU_SYS = UNDERPOWERED` | `CONDUCTOR_RULING;SYS_UNRESOLVABLE_BY_SLOPE` |
| 11 | `PLATEAU_SYS = RISING` ∧ some arm `FLAT_NO_GAIN` | `CONDUCTOR_RULING;FLAT_ARM_VS_RISING_SYS` |
| 12 | `PLATEAU_SYS = RISING` | `CONDUCTOR_RULING;NOTHING_PLATEAUS;SEE=TODO-719` |
| 13 | `PLATEAU_SYS = INDETERMINATE` with `reason=falling` or `reason=marginal` | `CONDUCTOR_RULING;SYS_INDETERMINATE` |
| 14 | catch-all | `CONDUCTOR_RULING;UNMATCHED` |

**The closed set** of values `NEXT=` may print (13 literal forms, 15 values; `<arm>` ∈ `JE MI3 MI2`):
`STOP` · `CONDUCTOR_RULING;UNMATCHED` · `CONDUCTOR_RULING;SYS_MISSING` · `CONDUCTOR_RULING;SYS_BIMODAL;SEE=TODO-719` ·
`CONDUCTOR_RULING;OPS` · `TODO-696+TODO-695;THEN;DEFAULT_FLIP=JE` · `TODO-696+TODO-695;THEN;DEFAULT_FLIP=MI3` ·
`TODO-696+TODO-695;THEN;DEFAULT_FLIP=MI2` · `CONDUCTOR_RULING;ARM_ORDER_ACCOUNTING` ·
`KEEP_SYSTEM;SYS_FLAT_WITHIN_BAR` · `STAGE2;SYS_LONG_CELLS` · `CONDUCTOR_RULING;SYS_UNRESOLVABLE_BY_SLOPE` ·
`CONDUCTOR_RULING;FLAT_ARM_VS_RISING_SYS` · `CONDUCTOR_RULING;NOTHING_PLATEAUS;SEE=TODO-719` ·
`CONDUCTOR_RULING;SYS_INDETERMINATE`. (Clauses 7/7a of an earlier revision were removed; the other clause numbers
are kept so the spec's audit history stays valid.) Any other printed value is a defect of the decide program.

**Order is load-bearing:** a missing SYS input (clause 2 — an absent or `n/a` SYS `TREND` included) stops before the
flip (clause 5); an out-of-domain token anywhere stops at 1a. Literals 5 and 8 are the only ones that do not stop at
the conductor. **Literal 5's written precondition (TODO-696 scope):** one replicate 6 h memory cell of the candidate
on the same host must reproduce `VERDICT = CANDIDATE`, and TODO-695 must be resolved, before any flip. Literal 9 is
a spend decision: the conductor asks the user before any Stage-2 pre-registration. Literal 8 closes TODO-589's Linux
question only in the bounded sense of R5.7 (per-entry change below 1 %/h over the last 3 h of a 6 h cell); the 72 h
soak stays the fine test.

**Supporting definitions** (what the clauses read):
- `PLATEAU_SYS`, first match: `INDETERMINATE reason=missing|out_of_domain:<cell>:<TREND|PE_LEVEL>` (checked
  cell-major `s1 TREND, s1 PE_LEVEL, s2 TREND, s2 PE_LEVEL`) · `INDETERMINATE reason=disagree` (`SYS_AGREE ≠ TRUE`) ·
  `RISING` (either SYS `RISING`) · `INDETERMINATE reason=falling` (either `FALLING`) · `PLATEAU` (both `FLAT`) ·
  `UNDERPOWERED` (either `UNDERPOWERED`) · `INDETERMINATE reason=marginal`.
- `STAGE2_FEASIBLE` (only when `PLATEAU_SYS=UNDERPOWERED`): `FALSE reason=missing:<cell>:STAGE2_T` first, then
  `FALSE reason=>STAGE2_MAX_H`, else `TRUE`; otherwise `n/a reason=not_underpowered`.
  `STAGE2_COST_EUR = 2 × max(STAGE2_T) × PRICE_EUR_PER_H` when `TRUE`.
- `VERDICT_<arm>`, first match: `n/a reason=missing:<cell>:OPS` (non-numeric `OPS_RATIO`, checked first) ·
  `n/a reason=ops` (`< OPS_PARITY_MIN`) · `n/a reason=missing|out_of_domain:<…>` (arm `TREND` absent/`n/a`/out of
  domain, or `PE_LEVEL` / `VS_SYS` absent/`n/a`) · `CANDIDATE` (`FLAT` ∧ `BETTER`) · `FLAT_NO_GAIN` (`FLAT` ∧
  `NO_GAIN`) · `UNRESOLVED(<FALLING|UNDERPOWERED|MARGINAL>)` · `RISING` · `n/a reason=unmatched`.
- `DEFAULT_CANDIDATE`: `C` = arms with `VERDICT = CANDIDATE`; empty ⇒ `SYS` if `PLATEAU_SYS = PLATEAU`, else `NONE`;
  otherwise `m = min(PE_LEVEL over C)`, `T = {a ∈ C : PE_LEVEL_a ≤ m × (1 + TIE_BAND)}`, choice = first of `T` in
  `JE, MI3, MI2`; `ORDER_AGREE` repeats this with `RSS_PE_LEVEL` (`FALSE reason=missing:<arm>:RSS_PE_LEVEL` on a
  non-numeric input; vacuously `TRUE` when `|C| ≤ 1`); `ORDER_AGREE ≠ TRUE` ⇒ `NONE`.

### Conductor rule for `CONDUCTOR_RULING` outcomes (R8.1) — verbatim, pre-registered
From `reference/SPEC-377-conductor-rulings-v3.md`:

> - If no arm is FLAT: DEFAULT_CANDIDATE is chosen by LEVEL only — `VS_SYS` (candidate PE_END ≤ 0.80·min(SYS PE_END),
>   both SYS present, OPS_RATIO ≥ 0.95) with the set-based tie rule; the plateau claim stays OPEN.
> - Such a candidate is a PROVISIONAL default: the flip happens only through TODO-696 (out-of-process perf) + a
>   replicate cell of the candidate of ≥ 6 h (TODO-696 precondition), and the plateau question moves to a Stage-2 long
>   cell sized by the formula from this series' measured e.
> - If no candidate clears VS_SYS: KEEP_SYSTEM, plateau OPEN, TODO-719 (glibc tuning) becomes next.
> No other reading may be used by the conductor to decide; anything else is descriptive.

Amendment, from `reference/SPEC-377-conductor-rulings-v4.md`:

> ## Rec 3 — outcomes with a FLAT arm
> - No candidate clears VS_SYS (whatever the TRENDs, FLAT included) ⇒ KEEP_SYSTEM; plateau stays OPEN for SYS; a FLAT
>   candidate that is not better is recorded, not adopted.
> - `ARM_ORDER_ACCOUNTING` (and any other outcome not covered by R8.1) ⇒ `CONDUCTOR_RULE=n/a reason=judgement:<NEXT>`,
>   listed in §1 as conductor judgement; no default change can follow from it without a new pre-registered cell.
> ## Rec 4 — RISING excluded
> A RISING arm can never be the provisional default. Write the replicate pass criterion into TODO-696: the replicate
> (≥ 6 h, same pin/order position) must itself clear VS_SYS against the series' min(SYS PE_END) and must not be RISING.

**Term mapping (conductor-acknowledged, rulings v4 rec 9; no change of meaning):** "PE_END" in the rule is the level
`VS_SYS` reads, i.e. `PE_LEVEL` (the last-30-min mean); the census `PE_END` is a recorded reading only. "0.80" is
`BETTER_BAR`; "0.95" is `OPS_PARITY_MIN`; "the set-based tie rule" is the `DEFAULT_CANDIDATE` step with `TIE_BAND`;
"both SYS present" is the two-SYS rule; "the formula" is R5.8's `STAGE2_T`.

**Mechanical form** (`CONDUCTOR_RULE=`, printed after `NEXT=`; first match):
1. `STOP ≠ none` ⇒ `CONDUCTOR_RULE=STOP`.
2. `NEXT` not of the form `CONDUCTOR_RULING;…` ⇒ `n/a reason=not_conductor_ruling`.
3. `NEXT ∈ {CONDUCTOR_RULING;UNMATCHED, CONDUCTOR_RULING;SYS_MISSING, CONDUCTOR_RULING;OPS}` ⇒
   `n/a reason=excluded:<literal>`.
3a. Any arm's `TREND` absent, `n/a` (any reason) or out of domain ⇒ `n/a reason=missing:<arm>:TREND` (first such arm
   in the order `JE, MI3, MI2`). "No arm is FLAT" is never inferred from a missing line.
4. `Lset` = arms with a numeric `VS_SYS_<arm>` labelled `BETTER`, a numeric `OPS_RATIO_<arm> ≥ OPS_PARITY_MIN`, and
   `TREND_<arm> ≠ RISING`.
5. `Lset` empty ⇒ `KEEP_SYSTEM;PLATEAU=OPEN;NEXT=TODO-719` — whatever the TRENDs, FLAT included.
6. `Lset` non-empty ∧ no arm `FLAT` ⇒ choice by the tie rule over `Lset` ⇒
   `PROVISIONAL_DEFAULT=<choice>;STAGE2_T=<v>;THEN;TODO-696+REPLICATE_6H;THEN;STAGE2_PLATEAU;PLATEAU=OPEN`
   (`<v>` = `STAGE2_T_<choice>` exactly as printed: hours, `>STAGE2_MAX_H`, or `n/a reason=<…>`).
7. Otherwise ⇒ `n/a reason=judgement:<NEXT>`.

The rule reads only `STOP`, `NEXT`, `TREND_*`, `VS_SYS_*`, `OPS_RATIO_*`, `PE_LEVEL_*` and `STAGE2_T_*`.

**The closed set of `CONDUCTOR_RULE=` forms:** `STOP` · `n/a reason=not_conductor_ruling` ·
`n/a reason=excluded:<CONDUCTOR_RULING;UNMATCHED|CONDUCTOR_RULING;SYS_MISSING|CONDUCTOR_RULING;OPS>` ·
`n/a reason=missing:<je|mi3|mi2>:TREND` · `KEEP_SYSTEM;PLATEAU=OPEN;NEXT=TODO-719` ·
`PROVISIONAL_DEFAULT=<JE|MI3|MI2>;STAGE2_T=<h|>STAGE2_MAX_H|n/a reason=…>;THEN;TODO-696+REPLICATE_6H;THEN;STAGE2_PLATEAU;PLATEAU=OPEN` ·
`n/a reason=judgement:<NEXT>`.

**Conductor-judgement outcomes** (`CONDUCTOR_RULE=n/a reason=judgement:<NEXT>`, step 7 — `Lset` non-empty and some
arm `FLAT`): possible with `NEXT` = `CONDUCTOR_RULING;ARM_ORDER_ACCOUNTING` (always has a FLAT arm),
`CONDUCTOR_RULING;FLAT_ARM_VS_RISING_SYS` (always has a FLAT arm), and `CONDUCTOR_RULING;SYS_BIMODAL;SEE=TODO-719`,
`CONDUCTOR_RULING;SYS_UNRESOLVABLE_BY_SLOPE`, `CONDUCTOR_RULING;NOTHING_PLATEAUS;SEE=TODO-719`,
`CONDUCTOR_RULING;SYS_INDETERMINATE` whenever a FLAT arm sits beside a non-FLAT arm that clears `VS_SYS`. **No
default change can follow from a `judgement:` outcome without a new pre-registered cell.**

### Predicted readings and power (written before any cell; expectations, never gates)
**Source.** Reproduced in G1 (2026-09-29) by the seed `.specflow/reference/SPEC-377-power-derivation.py` (sha256
`e3f9a5f2f9a18cd48dfde3315c59972503b47cb0bb1380824bd49218e0a07fbc`, `LC_ALL=C python3`), which runs the gate's own
estimator (R5.4–R5.7: census join ± 30 s, interpolated `live`, `pe`, `PE_LEVEL`, OLS over rows
`[int(n/2) .. n−1]`, AR(1) `e`) over the committed SPEC-372 4-hour M1 cells (`phys_footprint_mb`, 60 s rows, census
every 300 s; M1 `A0` = 16.64 MiB used inside the program for these cells only). Every printed digit of the spec's
R7.8 table reproduced (`s2`'s `2e₆` prints 19.24; the spec rounds it to 19.2). The committed program
`evidence/spec377-power.py` (the seed in evidence form) and its sha256 are *to fill* before M; at M its output must
equal the table below digit for digit, or M is not frozen.

Two ways to the 6 h window (`W` = 3 h): **projected** `e₆ = e(W=2h) × (2/3)^1.5`; **empirical** `e` over the last
3 h of the same 4-hour cells (a cross-check, not the prediction).

| cell (M1) | `PE_LEVEL` | `s` (%/h, W=2h) | `r1` | `e` W=2h | `e` W=3h empirical | **`e₆` projected** | `2e₆` vs `B`=1 | max `\|s\|` for FLAT | predicted class at 6 h (with the 2–4 h `s`) |
|---|---|---|---|---|---|---|---|---|---|
| `j2` (JE) | 2 020 B | +1.34 | 0.21 | 0.56 | 0.44 | **0.30** | 0.61 < 1 | 0.39 | `MARGINAL` ⇒ `UNRESOLVED(MARGINAL)` |
| `m2` (MI v3) | 1 895 B | −6.35 | 0.87 | 3.05 | 2.59 | **1.66** | 3.32 > 1 | none | `FALLING` ⇒ `UNRESOLVED(FALLING)` |
| `s2` (SYS) | 430 B | −10.42 | 0.91 | 17.67 | 8.45 | **9.62** | 19.24 > 1 | none | `UNDERPOWERED`; `STAGE2_T` = 50 h ⇒ infeasible |

- **Slope caveat.** The 2–4 h slopes include whatever `1/live`-shaped term the data holds; in the 3–6 h window such
  a term is ≈ (2/3)² of its 2–4 h size, so the 6 h raw slopes may be smaller in magnitude. The predicted classes
  are stated for the 2–4 h slope and are not a gate. The fixed-memory fit (`FIXED_EST`) is not identifiable while
  `live` grows near-linearly and is recorded only; the earlier statement that `j2`'s rise is "mostly fixed memory"
  is retracted.
- **Linux per-row spread** (descriptive; SPEC-376 900 s CA cells, `fp_equiv_mb`, live rows with `elapsed ≥ 450 s`,
  residual sd around an OLS line relative to the last live row, seven rows each): `c1` 3.59 %, `c2` 3.22 %, `pa`
  3.71 %, `pb` 5.76 % — the order of M1 `j2`/`m2`, far below M1 `s2`; no usable `r1` from seven rows. MI v2 has no
  committed long series and is predicted as MI v3.
- **Is FLAT reachable at 6 h?** For a jemalloc-like arm, yes by `e` (`2e₆` = 0.61 < 1), **if** its raw slope over the
  last 3 h is within ±0.39 %/h. For mimalloc-like and System-like noise, no; the SYS-like Stage 2 (≈ 50 h) exceeds
  `STAGE2_MAX_H`, so the SYS answer is expected to go through `CONDUCTOR_RULING;SYS_UNRESOLVABLE_BY_SLOPE` and the
  conductor rule — an honest result, not a failure.

**Predicted flags** (M1-transferred; Linux noise and slopes are unmeasured, and the Linux JE arm runs background
threads the M1 could not):

| flag / reading | predicted | basis |
|---|---|---|
| `STOP` | `none` | SPEC-376 host: steal 0, `INSTRUMENT=SOUND` |
| `PLATEAU_SYS` | `UNDERPOWERED` (or `INDETERMINATE reason=falling`) | `s2` table row; research: glibc "early-low, then stepped" curve, level set by the arenas the threads touched |
| `SYS_AGREE` | no transferable prediction | M1 SYS was bimodal (footprint 0.4 GB vs 8.2 GB); on Linux the glibc stepped curve may reproduce it — `FALSE` routes to `SYS_BIMODAL` |
| `STAGE2_FEASIBLE` | `FALSE reason=>STAGE2_MAX_H` | `STAGE2_T` ≈ 50 h > 24 |
| `VERDICT_JE` | `UNRESOLVED(MARGINAL)` (or `CANDIDATE` if the Linux JE slope is small) | `j2` row |
| `VERDICT_MI3`, `VERDICT_MI2` | `UNRESOLVED(FALLING)` | `m2` row |
| `DEFAULT_CANDIDATE` | `NONE` | no arm predicted `FLAT` |
| `VS_SYS_je` | no transferable prediction; recorded range expectation `[0.3, 1.2]`, not a gate | M1 SYS bimodal (`AMP_FP` 0.47 vs 9.13) |
| `OPS_<cell>` | ≈ 258 writes/s on every cell; every `OPS_RATIO` in `[0.95, 1.05]` | paced workload; SPEC-376 `WRITES_PER_S_REF=257.647` |
| `LIVE_END_<cell>` | ≈ 2.1–2.3 M | SPEC-376 Linux rate ≈ 370 k live entries/h × 6 h |
| `CENSUS_DROPPED_N_<cell>` | ≥ 1 (`TERMINAL`) | "Row and census accounting" above, by construction |
| `LAZY_MAX_<cell>`, `LAZY_DRIFT` | ≈ 0 on every arm; `LAZY_DRIFT=FALSE` | no default arm purges with `MADV_FREE` on Linux (research) |
| `JE_BG_THREADS_je` | 4 | SPEC-376 `sje`: `max_background_threads=4` |
| `JE_CONFIRM_CONF` | `PASS` (#4 = the env value, #1/#2/#3/#5 `""`) | vendored `conf.c:442-445` |
| `MI_POSTINIT_NOTE` | no prediction; recorded | `verbose` stays on for the process lifetime |
| `NEXT` (most likely) | `CONDUCTOR_RULING;SYS_UNRESOLVABLE_BY_SLOPE` (literal 10), or `CONDUCTOR_RULING;SYS_INDETERMINATE` (13) | rows above |
| `CONDUCTOR_RULE` (most likely) | `PROVISIONAL_DEFAULT=<arm>;STAGE2_T=<v>;…` if an arm clears `VS_SYS`, else `KEEP_SYSTEM;PLATEAU=OPEN;NEXT=TODO-719` | R8.1 steps 5–6 |

If the cells contradict these predictions, the contradiction is itself a §3 finding.

### Budget and server lifecycle (R12)

| item | estimate | source |
|---|---|---|
| step 0: packages, parity, preflight | ≈ 15 min | SPEC-376 G6 |
| five builds | ≈ 40 min | SPEC-376: eight builds 3 638 s ≈ 7.6 min each |
| smoke (4 × 120 s + predicates/synth/parity) | ≈ 20 min | SPEC-376 smoke |
| fetch smoke, freeze M on the Mac, server re-checkout, preflight | ≈ 20 min | SPEC-376 G6 |
| series: 5 × (6 h + ≈ 3 min overhead + ≤ 10 min settle) | ≈ 31 h 05 min | R0.5, settle step |
| predicates, decide, artifact fetch + sha check | ≈ 15 min | |
| **total** | **≈ 33 h** | |

- **Cost** ≈ 33 h × €0.138 ≈ **€4.55**. **Cap: 37 h (≈ €5.11), counted from server creation**
  (`CAP_CROSS_UTC` = creation + 37 h). Crossing the cap without a finished series is a STOP for the
  conductor/user, who decide whether to keep the server. The conductor records the running total at each HANDOFF.
- **Disk:** the series phase asserts ≥ 40 GiB free on `/opt` at start (`DISK_FREE_AT_START=`).
- **One rule for cap and start time:** the cap is the STOP; the working-hours start is scheduling only and never
  overrides it. The conductor may delay the series launch so that `PREDICTED_END_UTC` (series start + ≈ 31 h 05 min)
  falls inside the user's working hours (the window the user states at the G5 launch, recorded in §3 with its time
  zone) **only if** `PREDICTED_END_UTC` + 1 h (fetch + delete) stays before `CAP_CROSS_UTC`; otherwise it launches
  immediately. Any delay is billed idle time and counts toward the cap.
- **Lifecycle (conductor only):** the conductor creates `topgun-bench` from snapshot `437171250` on the user's
  confirmation and hands over the series IP; no agent creates, resizes, snapshots or deletes a server; nothing
  touches `topgun-new`. After the data commit the conductor deletes the server on confirmation and records a
  post-delete list showing only `topgun-new` (§3).
- The G5 HANDOFF prints `SERIES_START_UTC=`, `PREDICTED_END_UTC=` and `CAP_CROSS_UTC=` and ends the turn (no
  polling).

### Frozen parents executed (sha256 checked in G1 against `spec376-manifest.md` §1 — all equal)
- `d05727f002087cb37fb8abce8f5f935dce11af5e0e0373c873e6a393a8a6f7b4` `packages/server-rust/benches/soak_harness/evidence/spec376-procmem.sh`
- `7dd646bcc0e22c6dd48bdfab5d11da628b279abecb6acec727e7e56e87a5cd61` `packages/server-rust/benches/soak_harness/evidence/spec376-parity.sh`
- `5ab7bdafd3a4e9b9ef9b690bdc0865b44f245901d6963b1a93cfeaaeaebbc011` `packages/server-rust/benches/soak_harness/evidence/spec376-synth373b-linux.sh`
- `840813461e3b1bd5c3a79291044d8ac515e09b94333ee530cd6a10de8fa0436f` `packages/server-rust/benches/soak_harness/evidence/spec349c2-fit.awk`
- `2e3ba4f4c0429d77d7f1cf267112706ddf95b095b2a14a6b05460cfa5d018c33` `packages/server-rust/benches/soak_harness/evidence/spec366-p5.awk`
- `ba65ffc4076307ffdbfb014565edaf1f17e185ef987ca6b3fe2565d544400215` `packages/server-rust/benches/soak_harness/evidence/spec366-p67.awk`
- `7d2ca6214beff1c4c0042879823172a45452ef99c06ca49521d5c96889a61d1b` `packages/server-rust/benches/soak_harness/evidence/spec371-predicates.sh` — unconditional: the derived predicates program executes its PM1 lines exactly as `spec376-predicates.sh:163-166` does

Data input (not a program): `5dbe02258b05ebcddfefc5be6d945d050c74597fa60edc172c2d58d1fcb87f1a`
`packages/server-rust/benches/soak_harness/evidence/spec376-synth-ref-darwin.txt`.

### Programs frozen at M — *to fill: G4 (M0 candidate), confirmed at M by G5*
Every `spec377-*` program (`spec377-cells.sh`, `spec377-chain.sh`, `spec377-order.sh`, `spec377-preflight.sh`,
`spec377-predicates.sh`, `spec377-decide.sh`, `spec377-decide.awk`, `spec377-synth.sh`, `spec377-power.py`) with its
sha256, equal file-for-file to the admitting smoke's `SMOKE_PROG_SHA=` lines.

### Build records — *to fill: G5 (from `spec377-builds.txt`)*
The five flavour lines (`flavour=<label> … sha256=… recompiled=yes marker=ok`), `rustc -vV`, glibc and awk banners;
`BUILD_ENV_MALLOC_CONF=unset` × 5; `BIN_MARKERS_<label>=` for the four server labels; `TREE_RC_<label>=` and
`TREE_PROOF_<label>=PASS` for the four labels (R1.3 whole-line regexes).

### Smoke admission results and frozen allocator literals — *to fill: G5*
`SMOKE_ADMISSION=`, `SYNTH_PARITY=`, `SYNTH377=`; the sha256 of `evidence/spec377-mi-literals.txt` and
`evidence/spec377-je-conf.txt` (committed byte-for-byte from the smoke), each line shown with trailing spaces rendered
as `␠` (documentation only; the committed file is the literal `PALLOC` keys on).

### Synthetic enumeration parameters — *to fill: G4*
E1's fixed seed and case count (R10).

### Hunk maps — *to fill: G2a (cells), G2b (chain, order, preflight), G3 (predicates)*
Every hunk of `diff spec376-<x> spec377-<x>` mapped to one R item.

**`diff spec376-cells.sh spec377-cells.sh`** (G2a; 45 hunks; parent line ranges; "item" = the seven-item closed
list in the `spec377-cells.sh` header). A hunk whose new-side lines belong to two items is split by new-side line
range, so every line maps to exactly one item and one R-item. Reproduce the list with
`diff spec376-cells.sh spec377-cells.sh | grep -E '^[0-9]'`.

| parent hunk | new-side lines → what | item | R-item |
|---|---|---|---|
| `2a3,60` | the seven-item header; the parent's header follows verbatim | 7 | R2.5 |
| `430c488` | the Linux-only FATAL names this runner | 7 | R2.1 |
| `443c501`, `445,446c503,504` | usage: runner name, parent and closed-list count | 7 | R2.5 |
| `449,450c507,508`, `452,462c510,519` | usage: the cell table (every cell SERIES_PIN + SIGKILL; smoke `ssy sje smi3 smi2`, series `s1 je mi3 mi2 s2`, labels, runtime env) | 1 | R0.5 |
| `466,471c523,530` | usage: the refusal guards (SERIES_PIN, widened freeze paths, sampler hash, allocator env) | 7 | R2.5 |
| `473,476c532,535` | usage: required env names and the chain that exports them | 2 | R0.5 |
| `503,504c562,564` | table doc: `LABEL` and `ALLOC_TREATMENT` replace `CELL_SERVER` | 1 | R0.5 |
| `507,515c567,574` | the nine cell rows (duration, cadence, label, flavour, treatment, phase) | 1 | R0.5, R2.2 |
| `517a577` | `CELL_GRACEFUL=no` for every cell (all SIGKILL) | 1 | R0.5 |
| `519c579` | basename `spec377-<cell>` | 2 | R0.5 |
| `532a593,628` | `manifest_procmem_sha` + the sampler sha256 assertion against this §1 before sourcing | 4 | R2.1 |
| `615c711` | data dir `target/spec377-<cell>-data` | 2 | R0.5 |
| `625a722,724` | the proof hand-back file path | 6 | R3.1 |
| `743a843,880` | the allocator env block: unset list + every exported `MIMALLOC_*`, per-treatment export, shell self-check (FATAL on mismatch) | 5 | R2.3 |
| `805,809c942,946` | `SERIES_PIN=34007e19` + placeholder refusal | 3 | R0.2, R2.4 |
| `818,820c955,961` | freeze diff over `'*.rs' Cargo.toml '*/Cargo.toml' Cargo.lock` | 3 | R2.4 |
| `826c967` | freeze-diff FATAL message | 3 | R2.4 |
| `830,831c971,977` | clean-tree check over the same paths; a failing `git status` is FATAL, never "clean" | 3 | R2.4, R0.6 |
| `833,834c979,980`, `836c982`, `839c985` | dirty-tree messages and state echo | 3 | R2.4 |
| `841c987` | comment names `spec377-chain.sh` | 7 | R2.5 |
| `843,844c989,990`, `847c993`, `849c995` | `SPEC377_HARNESS_BIN`, `SPEC377_CHAIN_START_EPOCH` | 2 | R0.5 |
| `853c999` | comment: the server code state is item 3 | 3 | R2.4 |
| `855,859c1001` | every server is built at `SERIES_PIN` | 3 | R0.5 |
| `861c1003`, `863c1005` | `SPEC377_SERVER_COMMIT` | 2 | R0.5 |
| `911a1054,1084` | `bin_marker_counts` (`BIN_MARKERS`, unreadable ⇒ named, never 0) and `thp_selected` (`THP` at launch) | 6 | R0.3, R3.2 |
| `936c1109`, `959c1132`, `989c1162` | `SPEC377_CHAIN_START_EPOCH`, `SPEC377_HARNESS_BIN` | 2 | R0.5 |
| `1018c1191` | a stale proof file is removed before the run | 6 | R3.1 |
| `1090c1263` | matrix banner names the runner and the cell's label | 1 | R0.5 |
| `1095c1268` | `SPEC377_CHAIN_START_EPOCH` in the matrix | 2 | R0.5 |
| `1107c1280` | matrix: code freeze `SERIES_PIN` | 3 | R2.4 |
| `1141c1314` | matrix: server code `SERIES_PIN` | 3 | R0.5 |
| `1144a1318,1325` | 1318, 1320–1322: allocator section, unset list, expected and runner-shell env → item 5 (R2.3); 1319: build label → item 1 (R0.5); 1323–1325: binary literals, THP, proof timing → item 6 (R0.3) | 5 / 1 / 6 | R2.3 / R0.5 / R0.3 |
| `1358a1540,1588` | `ALLOC_PROOF_MIN_ELAPSED=60`, `read_alloc_env` (unreadable or empty `environ` ⇒ rc 2), `count_je_bg_threads`, `alloc_proof_read` (atomic write; rc 1 pid gone, rc 2 blind on a live pid) | 6 | R3.1, R3.2 |
| `1416a1647,1668` | sampler hook: proof once at the first sample with `elapsed ≥ 60 s`; blind on a live pid ⇒ `SAMPLER FATAL` | 6 | R3.1 |
| `1598a1851,1901` | `mi_postinit_lines`, the proof fields (`unread` when never written), unread proof ⇒ INSTRUMENT DEFECT, `emit_alloc_proof` (six keys) | 6 | R0.3, R3.3 m6 |
| `1601a1905` | the six keys print immediately before the closing three lines | 6 | R0.3 |

**`MI_POSTINIT_LINES` boundary (G2a reading of R3.3 m6, for M to confirm).** The startup print of both mimalloc
majors ends with build-configuration lines after the last option line (`debug level :`, `secure level:`,
`mem tracking:`, vendored `v3/src/options.c:247-249`, `v2/src/options.c:233-235`); v2 prints them through
`_mi_message`, i.e. with the `mimalloc: ` prefix, v3 without. Counting strictly after the last option line would
therefore add a constant 3 to every MI2 cell and 0 to MI3 — lines of the startup print, not post-init output. The
runner closes the block at the end of that trailer instead (the contiguous run from the first option line through
option and trailer lines); everything after it that contains `mimalloc` or is an option line counts. The reading is
recorded, never a STOP; the smoke's captured literals (R6.2) show the real trailer before M.

### The §1 prefix sha256 — the command
Computed at M and at every later commit by exactly this command (the marker line is included in the hash); M's value
is recorded in the executor report and re-computed by `spec377-order.sh`, never written into §1:
```
git show <commit>:packages/server-rust/benches/soak_harness/evidence/spec377-manifest.md | sed '/^## APPEND-ONLY BELOW/q' | shasum -a 256
```

### ORDER=OK (`spec377-order.sh`; series chain start, decide start on the server, the data commit and HEAD)
1. M is an ancestor of the commit.
2. The §1 prefix sha256 of the manifest the caller reads equals M's (the command above), and M carries the marker.
3. Every program and data input listed in §1 hashes to its listed sha256.
4. No build input differs from `SERIES_PIN` and the working tree is clean over it:
   `git diff --quiet <SERIES_PIN>..HEAD -- packages/server-rust/src packages/server-rust/Cargo.toml packages/server-rust/build.rs packages/core-rust Cargo.toml Cargo.lock rust-toolchain.toml`
   and an empty `git status --porcelain` over the same pathspec.

### Carried traps — *to complete at M*
`LC_ALL=C` everywhere a number is parsed (the host locale is `ru_RU`); every awk program runs under `original-awk`
(BWK) via the chain/parity shim, whose banner must match `^awk version [0-9]{8}`; program sha binding; flags printed
after every STOP predicate; synthetic cases pin their inputs; smoke over every program path before the cells;
jemalloc here is `_rjem_`-prefixed, so only `_RJEM_MALLOC_CONF` configures it (plain `MALLOC_CONF` is silently
ignored); mimalloc v3 prints its option block unprefixed and v2 with `mimalloc: `; never bisect on `rss_mb`; release
builds are not byte-reproducible — only the sha256 of the LAUNCHED binary counts; program edits happen only on the
Mac, never on the server; `systemd-run` needs explicit `--setenv=HOME=… --setenv=PATH=…`; re-smoke hygiene moves
`target/spec377-s*-data` **and** `target/spec377-s*-data.meta` to `.runN` (never `rm`).

## APPEND-ONLY BELOW
