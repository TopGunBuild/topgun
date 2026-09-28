# SPEC-376 — Linux soak instrument on topgun-bench: pre-registration manifest

## §1 Pre-registration (frozen at commit M; nothing above the APPEND-ONLY marker changes after M)

> **Draft status.** This §1 is the G1 contract draft: the R0 contract (column schema, CSV header, name contract,
> flags order, cell table, absence rule), the column-0 literals and the extraction line ranges. Sections marked
> *to fill* are written by the group named there and completed before M is frozen (G6, after the smoke). Until M
> exists nothing here is frozen, and no reading may cite this file as pre-registered.

### Question and scope
Does a run series on a fresh `topgun-bench` (Hetzner CCX23, 4 dedicated vCPU / 16 GB, Debian 12) produce
allocation/memory cells whose every memory column is populated, honestly named, interpreted by the same awk line as
the frozen macOS programs, and anchored by Linux-native reference rates? This series builds and calibrates the Linux
instrument (a same-binary pair c1/c2 plus a portability control pb/pa). It decides nothing about the server's code
or its allocator; TODO-589 pre-registers the allocator choice against this instrument.

### Lineage
A Linux host is a **new baseline**. No number measured on the M1 transfers as a threshold. Only level-A
(program-determined) quantities — the `bytes_alloc` rate, `BYTES_PER_WRITE`, a/b ratios — are expected to reproduce
approximately, and only as a recorded port-sanity expectation marked `not_a_gate`. A Linux `fp_equiv_mb` or
`AMP_*L` reading is never cited as an M1 `phys_footprint` or `AMP_FP`; `hwm_rss_mb` is a peak RSS, not a peak
footprint.

### Literals (column 0; each exactly once in §1)
CAL_PIN=bee21fcd
PORT_EXPECT=[0.327,0.346] src=SPEC-373b-M1 not_a_gate

`CAL_PIN` is the hex commit the spec branch was created from (`bee21fcdf5c5b591758a678b070d1c2dc1101a2f`, the
merge of #173 and the snapshot image's commit); it is written as the same literal into `spec376-cells.sh`. It is a
commit id, never a movable name. `PORT_EXPECT` is the SPEC-373b M1 a/b rate band, recorded, never a gate.

### R0.1 CSV header (61 columns)
SPEC-373b's 55-column literal (`spec373b-cells.sh:1133`) with exactly five substitutions (positions 7, 8, 9, 10, 41)
and six appended columns (56–61); positions 1–6, 11–40 and 42–55 are unchanged in name and position. The literal,
verbatim (derived mechanically from line 1133 and checked column by column):

```
elapsed_secs,rss_mb,wal_mb,redb_mb,disk_total_mb,tombstone_bytes,fp_equiv_mb,hwm_rss_mb,lazyfree_mb,swap_mb,conj_snapshots_total,conj_current_epoch,conj_ceiling,conj_durable_watermark,durable_watermark_lag,claims,claim_lag_p50,claim_lag_p99,claim_lag_max,ret_epochs_claim_only,ret_epochs_durability_only,ret_epochs_both,ret_epochs_neither,ret_refs_claim_only,ret_refs_durability_only,ret_refs_both,ret_refs_neither,ret_stamped_bytes,ret_epochs_unslotted,ret_refs_open_epoch,ret_stamped_bytes_open_epoch,indexed_refs,considered_total,dropped_total,matched_nothing_total,absent_total,bytes_freed_total,removed_refs_observed_total,removed_bytes_observed_total,stamped_bytes_total,file_mb,alloc_live_bytes,alloc_live_mb,alloc_probe_elapsed_s,alloc_probe_seq,je_allocated,je_active,je_resident,je_retained,je_mapped,je_metadata,je_probe_elapsed_s,je_probe_seq,bytes_alloc,bytes_dealloc,anon_mb,private_dirty_mb,pss_mb,anon_huge_mb,smaps_rss_mb,smaps_read_ms
```

| pos | 373b (macOS) | 376 (Linux) | definition (all MB = KiB / 1024, printed `%.3f`) |
|---|---|---|---|
| 2 | `rss_mb` | `rss_mb` | unchanged: `ps -o rss=`, the harness's own memory series |
| 7 | `phys_footprint_mb` | `fp_equiv_mb` | `smaps_rollup`: `Anonymous − LazyFree + Swap` |
| 8 | `phys_footprint_peak_mb` | `hwm_rss_mb` | `status`: `VmHWM` (a peak RSS, not a peak footprint) |
| 9 | `reclaimable_mb` | `lazyfree_mb` | `smaps_rollup`: `LazyFree` |
| 10 | `compressed_mb` | `swap_mb` | `smaps_rollup`: `Swap` |
| 41 | `clean_mb` | `file_mb` | `smaps_rollup`: `Private_Clean + Shared_Clean` |
| 56 | — | `anon_mb` | `smaps_rollup`: `Anonymous` |
| 57 | — | `private_dirty_mb` | `smaps_rollup`: `Private_Dirty` |
| 58 | — | `pss_mb` | `smaps_rollup`: `Pss` |
| 59 | — | `anon_huge_mb` | `smaps_rollup`: `AnonHugePages` |
| 60 | — | `smaps_rss_mb` | `smaps_rollup`: `Rss` |
| 61 | — | `smaps_read_ms` | wall time of the `smaps_rollup` read, integer ms (sidecar cost) |

The eleven **memory columns**, in header order: `rss_mb, fp_equiv_mb, hwm_rss_mb, lazyfree_mb, swap_mb, file_mb,
anon_mb, private_dirty_mb, pss_mb, anon_huge_mb, smaps_rss_mb` (`smaps_read_ms` is a cost column, not a memory
column). No header produced by any 376 program contains `phys_footprint`, `reclaimable_mb`, `compressed_mb` or
`clean_mb`.

Required source fields (R1.2; missing or non-numeric on a live pid is fatal, never an empty cell):
`smaps_rollup`: `Rss, Pss, Private_Clean, Shared_Clean, Private_Dirty, Anonymous, LazyFree, AnonHugePages, Swap`;
`status`: `VmHWM`; `ps -o rss=`: non-empty. All parsing under `LC_ALL=C`; all arithmetic in integer KiB before the
single `/ 1024`.

### R0.2 Row invariants
Integer KiB, exact: `0 ≤ LazyFree ≤ Anonymous` and `Anonymous − LazyFree + Swap ≤ Rss + Swap`. A violating row is
still written; it increments a per-cell counter file, and the runner console ends with
`mem_invariant_violations=<n>`.

### R0.3 Predicate and reading names (`spec376-predicates.sh`)
**STOP predicates** (enter STOP-V, R5.3): `PV`, `PEL`, `PA`, `PM1`, `PMEM`.

**Recorded platform-neutral predicates** (spec372 logic, printed for the record only; enter neither STOP-V nor
smoke admission): `PR` (emitted as `PR-crashes=` / `PR-class=`, not `PR=`), `P5`, `P6`, `P7`, `PJ` (with its
`PJ-child=` line), `PC`.

**PV hunk (the one logic departure from spec372).** PV keys on a per-cell build label via this fixed map:

| cell | build label | flavour prefix |
|---|---|---|
| `sc`, `c1`, `c2` | `CA-cal` | `CA` |
| `spb`, `pb` | `CA-pin` | `CA` |
| `pa` | `CA-frz` | `CA` |
| `sdh` | `DH-pin` | `DH` |
| `sje` | `JE-cal` | `JE` |
| `ssy` | `SYS-cal` | `SYS` |
| `smi` | `MI-cal` | `MI` |
| harness | `H` | — |

The console line-1 shape regex becomes `flavour=(SYS|JE|MI|CA|DH)`; the line-1 flavour `g` is compared with the
flavour prefix of the mapped label (the text before `-`), never with the full label. PV is TRUE iff the launched
server sha256 on console line 1 equals the `sha256=` of the `spec376-builds.txt` line whose `flavour=` is the
cell's mapped label, and the harness sha256 equals the `H` line. The chain's pre-launch sha assertion stays as well.

**Kept platform-neutral 372 keys** (emitted under their 372 names; not memory-derived): `TOTAL_WRITES`,
`OPS_PER_S`, `OPS_AT_900`, `WRITE_ERRORS`, `JE_CONFIG`, `HOST`, `END_elapsed`, `windows`, `DECISION_SCRAPE`, and the
non-memory readings `TERM_t`, `TERM_src`, `TERM_live`, `TERM_je_allocated`, `TERM_je_metadata`, `TERM_join_lag_s`,
`TERM_redb_mb`, `TERM_R_redb`, `TERM_R_meta`, `TERM_reach_bytes`, `TERM_EST_AGREE`, `TERM_UNMODELLED_*`
(`_MB`, `_SHARE`) and their `DECIDE_*` twins.

**Denied memory-derived 372 keys** (never emitted by any 376 program): `PE`, `FP_*`, `S_*`, `RECLAIM_*`,
`*_AMP_FP`, `*_AMP_S`, `*_AMP_JE`, `*_fp_mb`, `*_s_mb`, `*_DIRTY_SHARE`, `*_FRAG_SHARE`, `*_A0_share`, `TREND`,
`TREND3*`, `TREND_*` (incl. `TREND_NATIVE*`; the 372 TREND family fits `amp_fp`). AC-3's mechanical form: keys are
extracted from each `spec376-*.predicates.txt` line by `^[A-Za-z0-9_-]+=` (key = text before the first `=`) and
piped through
`grep -E '^(PE|FP_.*|S_.*|RECLAIM_.*|.*_AMP_FP|.*_AMP_S|.*_AMP_JE|.*_fp_mb|.*_s_mb|.*_DIRTY_SHARE|.*_FRAG_SHARE|.*_A0_share|TREND|TREND3.*|TREND_.*)$'`,
expected empty. `spec376-calib.txt` is outside that grep (its `S_CA_*` flags are R0.4 names).

**Linux-specific names** (never emitted by `spec371/372-predicates.sh`): `PEL` (PE over `fp_equiv_mb`), `PMEM`
(invariants + no sampler fatal), `FPL_slope*`, `SL_slope*`, `LAZY_slope*`, `FPL_end`, `SL_end`
(`fp_equiv_mb + lazyfree_mb`), `LAZY_end`, `LAZY_RATIO_end`, `HWM_end`, `ANON_HUGE_end`, `TRENDL*`,
`<P>_AMP_FPL`, `<P>_AMP_SL`, `<P>_AMP_JEL`, `<P>_DIRTY_SHAREL`, `<P>_FRAG_SHAREL` (P ∈ {TERM, DECIDE}). Every
`AMP_*L` prints `n/a reason=no_linux_estimator` while `SPEC376_A0_MIB` / `SPEC376_B_LIVE` are unset (this series
leaves them unset).

### R0.4 Calibration flags block (`spec376-calib.sh`, in this order, after every STOP predicate)
1. `STOP=`
2. `INSTRUMENT=`
3. `WRITES_PER_S_REF=`
4. `S_CA_RATE=`
5. `S_CA_LIVE=`
6. `S_CA_FPL=`
7. `BYTES_ALLOC_RATE=`
8. `ALLOC_LIVE=`
9. `BYTES_PER_WRITE=`
10. `WRITE_PARITY=` — the c1/c2 **pair** value (the STOP clause), `%.4f` or `n/a`
11. `WRITE_PARITY_ALL=` — the four-cell value (recorded only), `%.4f` or `n/a`
12. `A0_L_MIB=`
13. `PORT_RATIO=`
14. `PORT_BPW_RATIO=`
15. `PORT_EXPECT=`
16. `PORT_IN_EXPECT=`
17. `STEAL=`
18. `LOAD_AT_START=`

Each key exactly once. `INSTRUMENT=` ∈ {`SOUND`, `NOT_SOUND reason=<stop|smoke>`}; `reason=stop` wins when both
apply.

### R0.5 Cell table

| cell | phase | duration | cadence | flavour | server built at | build label | teardown | role |
|---|---|---|---|---|---|---|---|---|
| sc | smoke | 120 | 20 | CA | `CAL_PIN` | `CA-cal` | SIGKILL | admission |
| spb | smoke | 120 | 20 | CA | `b166719d` | `CA-pin` | SIGKILL | admission (old server + new harness) |
| sdh | smoke | 120 | 20 | DH | `b166719d` | `DH-pin` | SIGTERM | admission: dhat frame paths |
| sje | smoke | 120 | 20 | JE | `CAL_PIN` | `JE-cal` | SIGKILL | admission: `je_*` columns |
| ssy | smoke | 120 | 20 | SYS | `CAL_PIN` | `SYS-cal` | SIGKILL | admission: glibc flavour |
| smi | smoke | 120 | 20 | MI | `CAL_PIN` | `MI-cal` | SIGKILL | admission: mimalloc flavour |
| c1, c2 | cal | 900 | 60 | CA | `CAL_PIN` | `CA-cal` | SIGKILL | calibration pair (same binary) |
| pb | cal | 900 | 60 | CA | `b166719d` (373b pin) | `CA-pin` | SIGKILL | portability control, before |
| pa | cal | 900 | 60 | CA | `e85adb1f` (373b freeze) | `CA-frz` | SIGKILL | portability control, after |

Cal order `c1 → pb → c2 → pa` (interleaved, one host session). Every other matrix literal is the SPEC-373b runner's
own: churn-clients 6, keyspace 200, or-churn true, or-keyspace 48, or-every 5, write-interval-ms 20,
writes-per-life 200, offline-keys 3, confirm-interval 2, steady-interval 300, quiesce 3, mem-sample-interval 5,
wal-fsync batched, mem gate neutralised, jitter seed 20260831, live-copy census at 300 s, journal default.

Port **47376**. Paths (relative to `/opt/topgun`): data dirs `target/spec376-<cell>-data`; checkouts
`target/spec376-src-<rev>`; target dirs `target/spec376-<label>`, one per build label (`CA-cal`, `CA-pin`,
`CA-frz`, `DH-pin`, `JE-cal`, `SYS-cal`, `MI-cal`, `H`); builds file `target/spec376-run/spec376-builds.txt`; smoke
OUT `/opt/topgun/target/spec376-run/smoke`.

Build commands per label: `CA-cal` / `CA-pin` / `CA-frz` `--release --features count-alloc --bin topgun-server`
(at `CAL_PIN` / `b166719d` / `e85adb1f`); `DH-pin` `--profile release-with-debug --features dhat-heap` (at
`b166719d`); `JE-cal` `--features alloc-jemalloc`, `MI-cal` `--features alloc-mimalloc`, `SYS-cal` no feature (at
`CAL_PIN`); `H` `--release --bench soak_harness` (at `CAL_PIN`).

### R0.6 Absence rule (normative for every gate)
No gate reads an absent, empty or non-numeric input as a pass; absence is FALSE and is named. A gate over a line
keyed `KEY=` passes only if exactly one line matches `^KEY=` and its value satisfies the gate; zero occurrences, a
duplicate, an empty value, or a non-numeric value where a number is required make it FALSE, named `=absent`,
`=dup`, `=<value>` or `=missing`. A gate over a set of rows is FALSE when the set is empty.

### Frozen extractions (line ranges fixed in G1; each extraction asserts its closing brace)
- **Rate program (R5.1)** — from `spec373b-verdict.sh` (sha256
  `d5cef0dfe2033f13dc8c5446a49233046a38beeaa43f5ea24817aab1d836dffa`), lines **151–170**: the body of the per-cell
  `awk -F, -v c="$c" '…'` program (line 150 opens the quote; line 151 is `function num(x) …`). The last line is
  trimmed of its trailing `' "$CSV" || FAILED="${FAILED} RATE_${c}"`, after which it must equal `      }` exactly.
  Command, verbatim:
  ```
  sed -n '151,170p' spec373b-verdict.sh | sed '$ s/'\'' "\$CSV" || FAILED="\${FAILED} RATE_\${c}"$//'
  ```
  The extracted text (with its trailing newline) hashes to
  `35fb23008728703b9b2159061ed513d0d040f1554d1f3f47917ea401d2cc01fd`. It prints `SKIPPED_<c>=`,
  `BYTES_ALLOC_RATE_<c>=` and `ALLOC_LIVE_<c>=`, and is run with `-v c=<cell>`.
- **PM1 (R4.2)** — from `spec371-predicates.sh` (sha256
  `7d2ca6214beff1c4c0042879823172a45452ef99c06ca49521d5c96889a61d1b`), lines **154–169**, trimmed of the trailing
  `' "$CSV"` exactly as `spec373b-verdict.sh:106` does; it must end at `      }`.
- **PE/PA source for PEL (R4.2)** — `spec372-predicates.sh` (sha256
  `baf0bce7dd6c29751fb62f525153b631ec9eeb37f0ba858aed7174fd978b1c91`), the `== STOP: PE / PA ==` block from line
  125, with exactly three substitutions: column `phys_footprint_mb → fp_equiv_mb`, label `PE= → PEL=`, detail
  `rows_with_footprint= → rows_with_fp_equiv=`.

### Fixtures
`spec376-fixtures/` (see its `README.md` for the per-case reference values): hand-built kernel-6.1 `smaps_rollup` /
`status` pairs for M1–M5. The real `spec376-smaps-sample.txt` from the `sc` smoke server is added at M.

**Fixture-mode contract (normative).** `procmem_row <pid> [<proc_root>]` (`spec376-procmem.sh`) selects fixture mode
iff `<proc_root>` is not the literal `/proc`. Each fixture case directory is a proc root for pid `4242`:

| path | meaning |
|---|---|
| `<root>/<pid>/smaps_rollup` | stands in for `/proc/<pid>/smaps_rollup` |
| `<root>/<pid>/status` | stands in for `/proc/<pid>/status` |
| `<root>/<pid>.alive` | `0` = the pid is gone; any other content (normally `1`), or no file, = alive; replaces `kill -0 <pid>`. Absence reads as ALIVE on purpose, so a malformed fixture fails closed (status 2) instead of yielding a silently empty row |
| `<root>/<pid>.ps_rss` | the literal `ps -o rss= -p <pid>` output (KiB, right-aligned); absent = `ps` printed nothing |

The live path (`/proc`) never reads `*.alive` or `*.ps_rss`. Fixture files follow Debian 12 / kernel 6.1 field layout:
`smaps_rollup` = the `[rollup]` header line, then `%-16s%8llu kB` per field; `status` = `Key:\t%8lu kB` for the
`Vm*`/`Rss*` lines.

**Proc root: fixtures only, and the chain proves it.** Overriding the proc root is permitted **only** in the synthetic
suite over `spec376-fixtures/`. `spec376-cells.sh` calls `procmem_row "$pid"` with no root argument (default
`/proc`), has no knob that changes it, and records `memory proc root:    /proc (fixed; …)` in every cell's matrix.
Requirement on the chain (implemented in G2b): every `spec376-chain.sh` phase that runs cells (smoke, cal) prints
exactly one line `PROC_ROOT=/proc` into its chain log. Requirement on the calibration reading (implemented in G3):
`spec376-calib.sh` reads the cal chain log (`spec376-chain.log`); unless exactly one `^PROC_ROOT=` line is present
and its value is exactly `/proc`, it raises the STOP-V clause `chain:proc_root=<value|absent>` (a duplicate line is
named `chain:proc_root=dup`, R0.6) — an absent line is FALSE, never a pass.

### Runner console contract (`spec376-cells.sh`, G2a)
Keys the predicates (G3) and calib read from `spec376-<cell>.runner-console.log` (the runner's stdout+stderr; the
chain appends `RUNNER_EXIT=`):
- Console line 1 of the harness console artifact (`spec376-<cell>.harness-console.log`) is the provenance line,
  unchanged in shape: `provenance: server sha256=<hex> flavour=<CA|DH|JE|SYS|MI> built=… run_start=…
  topgun_or_prune_restored_cancelled_total=present harness sha256=<hex> …`.
- Every run that got past launch ends its runner console with exactly these three lines, once each, in this order:
  `steal_pct=<%.4f|n/a>` (R2.6; `n/a` when a `/proc/stat` read fails or the total-tick delta is 0),
  `post_mortem_mem_reads=<n>`, `mem_invariant_violations=<n>` (R0.2). A run refused before launch prints none of
  them (PMEM then reads `violations=absent`).
- A blind memory sampler on a live pid prints `SAMPLER FATAL: memory sampler blind on live pid <pid>: <field>=<absent|dup|non_numeric>[,…] at elapsed=<s>s`.
- `post_mortem_mem_reads` counts memory reads that found the pid gone. In the runner such a read never produces a
  row: the read sits where the parent read `ps -o rss=`, so a gone pid ends sampling (run over) or is a SAMPLER FATAL
  (run not over), exactly as the parent's empty-RSS path. No written row carries an empty memory cell; the post-run
  check fails the cell (`INSTRUMENT DEFECT`) if any of the eleven memory columns is missing or has an empty cell.

### Smoke admission results (pre-registration input) — *to fill (G6, from the smoke chain log)*
Smoke chain log excerpt; builds sha256 lines; awk banner; `SYNTH_PARITY`, `SYNTH376`, `DH_FRAMES`, `SELF_CHECK`,
`SMOKE_ADMISSION`; the smoke's `SMOKE_PROG_SHA=` lines (must equal the program list below file-for-file and
sha-for-sha, else re-smoke before M).

### Programs frozen at M (sha256; `ORDER=OK` re-checks these bytes) — *to fill (G5 candidate list, G6 at M)*
Every `spec376-*` program plus the frozen parents they execute: `spec373b-verdict.sh`, `spec373b-shares.py`,
`spec371-predicates.sh`, `spec349c2-fit.awk`, `spec366-p5.awk`, `spec366-p67.awk` (and `spec373b-order.sh` if
called).

### Hunk maps — *G2a filled (cells); to fill (G2b, G3, G4)*

**`diff spec373b-cells.sh spec376-cells.sh`** (57 hunks; parent line ranges; "item" = the nine-item closed list in
the `spec376-cells.sh` header). Every hunk maps to exactly one R-item:

| parent hunk | what | item | R-item |
|---|---|---|---|
| `1a2,67` | new header block (nine-item difference list) prepended; parent header kept verbatim below it | 9 | R2 (header) |
| `358a425,432` | `uname -s` ≠ `Linux` ⇒ FATAL exit 2, first executable check | 1 | R2.1 |
| `369c443`, `371,373c445,447`, `375,383c449,462`, `387,393c466,471`, `396,399c474,476` | usage text: runner name, nine items, cell list, CAL_PIN, env names | 9 | R2 (header) / R0.5 |
| `416c493` | flavour column doc `CA\|DH\|JE\|SYS\|MI` | 3 | R2.3 |
| `427c504,505`, `429,434c507,515`, `437c518,519` | cell table sc/spb/sdh/sje/ssy/smi (120 s, cadence 20) + c1/c2/pb/pa (900 s, 60); `CELL_SERVER` cal/pin/frz; `CELL_PHASE`; basename `spec376-<cell>` | 8 | R0.5 |
| `443a526,539` | source `spec376-procmem.sh` before any clock; FATAL if absent or without `procmem_row` | 2 | R2.2 |
| `519c615` | data dir `target/spec376-<cell>-data` | 8 | R0.5 |
| `525a622,625` | counter files `PROCMEM_INV_FILE`, `PROCMEM_PM_FILE` | 2 | R2.2 / R0.2 |
| `565a666,671` | a smoke cell refuses the tracked evidence dir | 8 | R0.5 / R3.3 |
| `672,675c778,780` | port 47376 | 8 | R0.5 |
| `700,702c805,807`, `713,714c818,819`, `721c826`, `1000c1107` | freeze literal `CAL_PIN=bee21fcd` and its three guards, renamed | 7 | R2.7 |
| `736c841` | comment: the chain that builds is `spec376-chain.sh` | 9 | R2 (header) |
| `738,739c843,844`, `742c847`, `744c849`, `829c936`, `852c959`, `882c989`, `988c1095` | env names `SPEC376_HARNESS_BIN`, `SPEC376_CHAIN_START_EPOCH` | 8 | R0.5 |
| `750a856`, `752c858`, `755c861`, `757c863`, `1034c1141` | server code-state gate: cal→`CAL_PIN`, pin→`b166719d`, frz→`e85adb1f`; `SPEC376_SERVER_COMMIT` | 7 | R2.7 |
| `767,768c873` | dead non-provenance build hint without `xcrun` | 9 | R2 (no macOS path) |
| `797a903,904` | MI and SYS marker arms | 3 | R2.3 |
| `911c1018` | counter files reset with the other sampler files | 2 | R2.2 |
| `959c1066` | pid via `ss -Hltnp` first (pgrep second, unchanged); no lsof | 4 | R2.4 |
| `970c1077` | pre-launch port check also refuses a pid-less `ss` listener | 4 | R2.4 |
| `983c1090` | matrix banner names the Linux run, cell phase | 9 | R2 (header) |
| `1038a1146,1164` | matrix host block (os-release, kernel, CPU, nproc, glibc, rustc host, THP, swap, overcommit, max_map_count, awk + banner, sampler sha, proc root) + `/proc/loadavg` + first 8 lines of `/proc/meminfo` | 5 / 6 | R2.5 / R2.6 |
| `1118a1245,1255` | `proc_stat_steal()` + steal ticks at T0 | 6 | R2.6 |
| `1124,1133c1261,1276` | CSV header per R0.1 (61 columns) + `MEM_COLUMNS` list | 2 | R0.1 / R2.2 |
| `1196,1240d1338` | `footprint_row()` removed | 2 | R2.2 |
| `1272c1370`, `1294,1302c1392,1415`, `1328c1441` | `emit_row`: `procmem_row` replaces the `ps -o rss=` read (gone ⇒ stop/fatal as parent; blind live pid ⇒ `sampler_fatal`); split into 12 cells; rss dropped from the integer loop (validated by the helper) | 2 | R2.2 / R1.2 |
| `1348,1353d1460` | the footprint call in `emit_row` removed | 2 | R2.2 |
| `1389c1496`, `1391c1498`, `1396c1503`, `1398c1505`, `1401a1509` | row `printf`: rss from the helper; cols 7–10, 41, 56–61 from the helper | 2 | R0.1 / R2.2 |
| `1441a1550,1551` | steal ticks at the end | 6 | R2.6 |
| `1478a1589,1605` | read counters, compute `steal_pct`, record them in the matrix, define `emit_tail` | 2 / 6 | R0.2 / R2.6 |
| `1533a1661,1679` | post-run check of the eleven memory columns by header name (population, not non-zero) | 2 | R2.2 |
| `1600a1747`, `1611a1759` | `emit_tail` before both final exits (console ends with `steal_pct=`, `post_mortem_mem_reads=`, `mem_invariant_violations=`) | 2 / 6 | R0.2 / R2.6 |

Reproduce the hunk list with `diff spec373b-cells.sh spec376-cells.sh | grep -E '^[0-9]'`. `spec376-procmem.sh` is a new
file with no parent (R1).

**Still to fill:** `diff spec373b-chain.sh spec376-chain.sh` → R3 items
(including the cal-phase predicates step, mapped to R3.4); `diff spec373b-order.sh spec376-order.sh`;
`diff spec372-predicates.sh spec376-predicates.sh` → R0.3 / R4 items (including the PV hunk, mapped to R0.3);
`diff spec373b-synth.sh spec376-synth373b-linux.sh` → one hunk, lines 114–115 (R7.2).

### The §1 prefix sha256 — the command
Computed at M and at every later commit by exactly this command (the marker line is included in the hash); M's value
is recorded in the executor report and re-computed by `spec376-order.sh`, never written into §1:
```
git show <commit>:packages/server-rust/benches/soak_harness/evidence/spec376-manifest.md | sed '/^## APPEND-ONLY BELOW/q' | shasum -a 256
```

### ORDER=OK (`spec376-order.sh`; the cal chain at start, calib at start, the data commit and HEAD)
1. M is an ancestor of the commit.
2. The §1 prefix sha256 of the manifest the caller reads equals M's (the command above), and M carries the marker.
3. Every program listed above hashes to its listed sha256.
4. No build input differs from `CAL_PIN` and the working tree is clean over it:
   `git diff --quiet <CAL_PIN>..HEAD -- packages/server-rust/src packages/server-rust/Cargo.toml packages/server-rust/build.rs packages/core-rust Cargo.toml Cargo.lock rust-toolchain.toml`
   and an empty `git status --porcelain` over the same pathspec.

### Carried traps — *to complete at M*
`LC_ALL=C` everywhere a number is parsed; every awk program runs under `original-awk` (BWK) via the chain/parity
shim, whose banner must match `^awk version [0-9]{8}`; program sha binding; flags printed after every STOP
predicate; post-mortem row rule PM1; synthetic cases pin their inputs; smoke over every program path before the
cells; never bisect on `rss_mb`; release builds are not byte-reproducible — only the sha256 of the LAUNCHED binary
counts (console line 1 vs `spec376-builds.txt`); program edits happen only on the Mac, never on the server.

## APPEND-ONLY BELOW
