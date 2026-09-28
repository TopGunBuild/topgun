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

### Data inputs frozen at M (sha256; `spec376-parity.sh` and `ORDER=OK` check these bytes)
Not programs (outside the smoke's `SMOKE_PROG_SHA=` binding, which draws only `is_program` files), but inputs a
gate trusts. `spec376-parity.sh` refuses the reference unless its sha256 equals exactly one listing below
(`SYNTH_PARITY=FAIL reason=ref_sha_manifest|ref_sha_unlisted|ref_sha_dup|ref_sha_mismatch`); `spec376-order.sh`
hashes every `- \`<sha256>\` \`<path>\`` line of §1, so it re-checks this one too.
- `5dbe02258b05ebcddfefc5be6d945d050c74597fa60edc172c2d58d1fcb87f1a` `packages/server-rust/benches/soak_harness/evidence/spec376-synth-ref-darwin.txt` — Darwin reference transcript of the frozen `spec373b-synth.sh` (R7.3; captured G4)

### Hunk maps — *G2a (cells), G2b (chain, order), G3 (predicates), G4 (synth copy) filled*

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

**`diff spec373b-chain.sh spec376-chain.sh`** (20 hunks; "item" = the nine-item closed list in the
`spec376-chain.sh` header). The copy is mostly new code, so `diff` anchors on generic lines (`fi`, `}`, blank) and
several hunks span more than one item. Where a hunk does, its **new-side** line ranges are attributed separately,
so every line of every hunk maps to exactly one item and one R-item:

| parent hunk | new lines → what | item | R-item |
|---|---|---|---|
| `3c3,4`, `5,27c6,10`, `29,32c12,69` | header: title, why a copy exists, nine-item list, the column-0 key rule | 9 | R3 (header) |
| `35a73,79` | `uname -s` ≠ `Linux` ⇒ FATAL exit 2, before any file is created | 1 | R3.1 |
| `40,47c84,118` | 84–92 literals (pin, 373b freeze, port 47376, run dir, builds file outside EV, smoke OUT pin, labels) | 4 | R3.2 / R0.5 |
| | 94–100 `CAL_PIN` read from the one `^CAL_PIN=` line of `spec376-cells.sh`, hex-checked | 4 | R3.2 / R0.5 |
| | 102–103 `SPEC376_PHASE` ∈ build/smoke/cal, no default | 3 | R3.2–R3.4 |
| | 105–109 inherited per-run knobs unset | 3 | R3.3 / R3.4 |
| | 111–118 smoke OUT: textual pin to `target/spec376-run/smoke`, evidence dir and anything under it refused before any mkdir | 6 | R3.3 |
| `49,54c120,127` | smoke OUT re-checked after resolution; `SPEC365_OUT_DIR` ← `SPEC376_OUT_DIR`; smoke cells and log | 6 | R3.3 |
| `56,57c129,131` | cal / build OUT and log names (`spec376-chain.log`, `spec376-chain-build.log`) | 3 | R3.2 / R3.4 |
| `64,86c138` | start line (`cal_pin=`); the parent's start-of-chain ORDER block moves to the cal gates (hunk `206,211`); the `xcrun`/`SDKROOT` block is dropped | 3 / 1 | R3.4 / R3.1 |
| `88,110c140,151` | awk shim: `original-awk` required, `target/spec376-awkbin/awk`, `PATH` prepend, resolution + `^awk version [0-9]{8}` banner asserted, banner logged | 2 | R3.1 / R7.1 |
| `112,118c153,164` | `cell_label`: the per-cell build-label map (R0.3 PV map) | 5 | R3.3 / R3.4 |
| `121,129c167,172` | `label_commit`: the commit each label is built at | 4 | R3.2 |
| `131,137c174,208` | 174–194 `builds_field` / `builds_count` / `label_sha_reason` (exactly one builds line per label, 64-hex sha, launched = built) | 5 | R3.3 / R3.4 |
| | 195–208 `hits` / `marker_ok`: the runner's five-flavour marker rule + harness literals | 4 | R3.2 |
| `139,153c210,223` | 210 `marker_ok` tail | 4 | R3.2 |
| | 211–213 `is_program`: the one rule drawing both the smoke's `SMOKE_PROG_SHA=` lines and M's bound list | 6 / 7 | R3.3 / R3.4 |
| | 214–223 `key_once`: exactly-once presence (`absent` / `dup`) | 6 | R6 / R0.6 |
| `155,194d224` | the parent's builds-file loop and `run_cell` removed at this position (rewritten in the build phase and in `4. cells`) | 3 | R3.2 |
| `196c226,238` | `iso_epoch`: `PREFLIGHT_AT=` → epoch by arithmetic (no `date` dialect) | 7 | R3.4 |
| `198,204c240,324` | phase `build`: three clean detached checkouts, `guarded_rm` over the eight label dirs, eight builds with the R3.2 commands, one harness binary, builds file (`build_start_epoch=`, eight `flavour=` lines, `rustc:`, `glibc:`, `awk:`), recompiled / mtime / marker asserted per label | 4 | R3.2 |
| `206,211c326,428` | 326–351 phase `cal`: `spec376-order.sh` sha checked against M's listing, then `ORDER=OK` required | 7 | R3.4 |
| | 352–385 smoke → M binding (`SMOKE_BINDING=PASS|FAIL <file>=<absent|dup|changed|unlisted>`) | 7 | R3.4 |
| | 386–404 preflight gate (newest log, last line `^PREFLIGHT=PASS( |$)`, one `PREFLIGHT_AT=`, age 0..3600 s), `PREFLIGHT_LOG=` | 7 | R3.4 |
| | 406–423 builds file read: one `build_start_epoch=`, every needed label's sha + `marker=ok` asserted before any cell | 5 | R3.3 / R3.4 |
| | 424–428 `SMOKE_PROG_SHA=` lines (smoke) | 6 | R3.3 / R6.6 |
| `214,217c431,588` | 431–433 `PROC_ROOT=/proc` | 6 / 7 | R3.3 / R3.4 |
| | 435–438 host log, `SPEC376_CHAIN_START_EPOCH` ← build start, `SPEC376_HARNESS_BIN` | 8 / 4 | R3.5 / R3.2 |
| | 440–460 `capture_smaps` (sc's server, ~60 s after its listener appears) | 6 | R6.2 |
| | 462–492 `run_cell`: host sidecar, `LOAD_AT_START_<cell>=`, pre-launch sha assertion (`RUNNER_EXIT=98`, not launched), launch, `RUNNER_EXIT=` | 8 / 5 | R3.5 / R3.3–R3.4 |
| | 494–505 `run_predicates` after each cell, `PREDICATES_EXIT_<cell>=<rc>` | 7 | **R3.4** (the cal-phase predicates step; the same step runs per smoke cell, R3.3) |
| | 507–516 cal reading: `spec376-calib.sh <EV> spec376-manifest.md <EV>/spec376-smoke` → `spec376-calib.txt`, rc logged, flags echoed indented | 7 | R3.4 |
| | 518–548 smoke: calib self-run (rc logged), `smaps_sample_check` (fixture-mode replay) | 6 | R3.3 / R6.2 |
| | 550–588 dhat frame check (`DH_FRAME_<site>=`, `DH_FRAMES=`), shares self-check, parity, synth | 6 | R6.3–R6.5 |
| `219c590,591` | the synth-missing branch (no `SYNTH376=` line, so admission names it) | 6 | R6.5 |
| `220a593,678` | smoke admission: every R6 item under R0.6, one `SMOKE_ADMISSION=` line | 6 | R6 |

**`diff spec373b-order.sh spec376-order.sh`** (11 hunks; "item" = the four-item closed list in the
`spec376-order.sh` header):

| parent hunk | what | item | R-item |
|---|---|---|---|
| `3,4c3,5`, `6c7,28`, `9c31`, `18,20c40,43`, `25c48`, `27c50` | header: title, why a copy exists, four-item list, usage and check text | 4 | R10 (header) |
| `34c57,58` | manifest path `spec376-manifest.md` (via `EVREL`) | 1 | R10 |
| `36c60,63` | `MIN_PROGS` → `REQUIRED_PARENTS` (the six frozen parents the Linux programs execute) | 3 | R10 ORDER item 3 |
| `63c90,102` | program coverage: non-empty list; every `spec376-*.{sh,awk,py}` in the evidence dir and every required parent listed exactly once | 3 | R10 ORDER item 3 |
| `65,69c104,109` | freeze literal `CAL_PIN=` read from `spec376-cells.sh`, required exactly once, hex, a commit, `.rs`/build inputs equal to it, tree clean | 2 | R10 ORDER item 4 / R0.5 |
| `72c112` | success line `… cal_pin=<F>` | 4 | R10 |

**`diff spec372-predicates.sh spec376-predicates.sh`** (49 hunks; parent line ranges; "item" = the nine-item closed
list in the `spec376-predicates.sh` header). A hunk whose new-side lines belong to two items is split by new-side
line range; every line maps to exactly one item and one R-item:

| parent hunk | new-side lines → what | item | R-item |
|---|---|---|---|
| `1a2,74` | new header block (nine-item difference list, usage, exit statuses); the parent header is kept verbatim below it | 9 | R4 (header) |
| `36c109` | usage text names `spec376-predicates.sh` | 1 | R4.1 |
| `43c116,119` | 116–118 frozen-source paths + sha256 literals (`spec371-predicates.sh`, `spec349c2-fit.awk`); 119 base prefix `spec376-` | 4 / 6 ; 1 | R4.2 / R4.4 ; R4.1 |
| `48d123` | no `PRED` path: the block goes to stdout | 9 | R3.4 |
| `51,52c126,129` | 126 series file `ampfpl.csv`; 127 truncation without `PRED`; 128–129 exit-status accumulator | 6 ; 9 ; 9 | R0.3 ; R3.4 ; R4 (exit) |
| `54,58c131,137` | M1 constants removed; `A0`/`B_live` from `SPEC376_A0_MIB`/`SPEC376_B_LIVE`, both numeric or neither | 7 | R4.5 |
| `61,69c140,144` | cell → flavour map (sc spb c1 c2 pb pa CA, sdh DH, sje JE, ssy SYS, smi MI); a2j branch gone | 1 | R0.5 / R4.1 |
| `72c147,156` | 147–155 cell → build-label map; 156 journal expected on every cell | 2 ; 1 | R0.3 (PV) ; R0.5 |
| `78a163,172` | 163–169 PM1 program extracted from the frozen `spec371-predicates.sh` 154–169 (sha256, marker text, closing brace asserted); 170–171 fitter sha256 check; 172 blank | 4 ; 6 | R4.2 ; R4.4 |
| `81,83c175` | a2j `UNKNOWN` branch gone | 1 | R4.1 |
| `86c178`, `91c183`, `94a187,188`, `97c191`, `104c198`, `106c200` | PV by build label: label passed in, builds label counted and required exactly once (label and `H`), line-1 regex adds `DH`, server sha compared with the label's line, label printed | 2 | R0.3 (PV hunk) |
| `108c202` | PV awk exit status checked | 9 | R4 (exit) |
| `112,113c206,207` | `PR-crashes` reads `soak.json` with `sed` (no `jq` on the image) | 8 | R0.3 (recorded) |
| `127c221` | `PE=FALSE reason=no_matrix_or_csv` → `PEL=` (the line outside the awk program) | 3 | R4.2 |
| `131,132c225,226` | column `phys_footprint_mb` → `fp_equiv_mb`; label `PE=` → `PEL=` | 3 | R4.2 |
| `136,137c230,231` | 230 label `PE=` → `PEL=`, detail `rows_with_footprint=` → `rows_with_fp_equiv=`; 231 awk exit status checked | 3 ; 9 | R4.2 ; R4 (exit) |
| `156c250` | PA awk exit status checked | 9 | R4 (exit) |
| `177,178c271,282` | `post_mortem_rows=` exactly once and a plain count, else named; PM1 FALSE when the frozen source fails its checks | 4 | R4.2 / R0.6 |
| `181,197c285` | the inline PM1 program replaced by the extracted frozen text | 4 | R4.2 |
| `199a288,302` | PMEM block | 5 | R4.3 |
| `225,230c328,331` | `HOST` from the matrix's single `/proc/loadavg` line | 8 | R2.6 / R0.3 |
| `236c337` | block to stdout | 9 | R3.4 |
| `240a342` | fitter sha mismatch ⇒ `FIT_ERROR reason=frozen_fit_sha` | 6 | R4.4 |
| `244,248c346,350` | `SL = fp_equiv_mb + lazyfree_mb` stream | 6 | R4.4 |
| `252,254c354,356` | JE native series over `fp_equiv_mb` (`amp_nativel`) | 6 | R0.3 |
| `255a358` | exit 3 on a fitter sha mismatch | 6 | R4.4 |
| `257,259c360,362` | fits over `fp_equiv_mb`, `lazyfree_mb`, `sl_mb` | 6 | R4.4 |
| `264c367` | `amp_nativel` fit | 6 | R0.3 |
| `274,275c377,380` | census join comment + estimator flag passed in | 7 | R4.5 |
| `281,282c386,387` | census join reads `fp_equiv_mb` / `lazyfree_mb` | 6 | R0.3 |
| `290c395` | series header `amp_fpl` | 6 | R0.3 |
| `302,306c407,413` | 407–408 Linux column names in the census line; 409–413 reach / `AMP_FPL` / `AMP_SL` only with an estimator | 6 ; 7 | R0.3 ; R4.5 |
| `311,312c418,419` | 418 series point only with an estimator; 419 awk exit status checked | 7 ; 9 | R4.5 ; R4 (exit) |
| `314c421`, `316,317c423,424` | `amp_fpl` fits (last half, last third) | 6 | R0.3 |
| `330,336c437,452` | 437–439 `FPL_slope`, `SL_slope`, `LAZY_slope`; 440–446 `TRENDL`/`TRENDL3` or `n/a reason=no_linux_estimator`; 447 `TRENDL_NATIVE`; 448–452 `TRENDL_dropped`, `TRENDL_points_used` = rows of the `amp_fpl` series | 6 ; 7 ; 6 ; 6 | R0.3 ; R4.5 ; R0.3 ; R0.3 |
| `338c454`, `341c457`, `343c459`, `345,348c461,466` | end levels `FPL_end`, `SL_end`, `LAZY_end`, `LAZY_RATIO_end`, `HWM_end`, `ANON_HUGE_end` | 6 | R0.3 |
| `351c469` | estimator flag into the census-point program | 7 | R4.5 |
| `357,361c475,479` | 475 `_fp_mb`/`_s_mb`/`_A0_share` dropped; 476 `_reach_bytes` needs an estimator; 477 `_redb_mb`; 478–479 `_AMP_FPL`/`_AMP_SL`/`_R_redb` need an estimator | 6 ; 7 ; 6 ; 7 | R0.3 ; R4.5 ; R0.3 ; R4.5 |
| `365,371c483,494` | 483 `_AMP_JEL` (n/a without an estimator, as every `AMP_*L`); 484–485 `_DIRTY_SHAREL`, `_FRAG_SHAREL`; 486–494 `_R_meta`/`_EST_AGREE`/`_UNMODELLED_*` need an estimator | 7 ; 6 ; 7 | R4.5 ; R0.3 ; R4.5 |
| `378,380c501,503` | census awk exit status checked; block to stdout; exit status | 9 | R4 (exit) / R3.4 |

**`diff spec373b-synth.sh spec376-synth373b-linux.sh`** (G4): exactly **one** hunk, `114,115c114,115`; no header
comment line changed (the copy keeps the parent's header, usage text and case table verbatim).

| parent hunk | what | R-item |
|---|---|---|
| `114,115c114,115` | S10 setup: BSD `sed -i ''` → GNU `sed -i`; the FATAL text names GNU sed (`S10 setup (GNU sed -i) failed`) | R7.2 |

### Preflight and awk-parity contract (`spec376-preflight.sh`, `spec376-parity.sh`, G4)
- **Preflight order (R9).** Linux-only guard (FATAL, exit 2, before any file) → usage → log created (noclobber) →
  `identity` → `isolation` → if either FAILs: log `REFUSED: …`, last line `PREFLIGHT=FAIL failed=<identity|isolation|
  identity,isolation> PREFLIGHT_AT=…`, exit 1, nothing changed, no further check → else (`--apply` only) `mutate
  systemctl stop <5 timers + unattended-upgrades.service>` and `printf madvise | mutate tee <THP knob>` for
  `enabled`, `defrag` (before/after recorded) → `os, timers, load, steal, swap, thp, memory, disk, clock, tools` →
  recorded-only block → last line. Exit 0 = PASS, 1 = FAIL, 2 = guard/usage/log collision.
- **Log.** `spec376-preflight-<YYYYMMDDTHHMMSSZ>.log` in `SPEC376_PREFLIGHT_LOG_DIR` (default: the evidence dir);
  a second run in the same second refuses (exit 2) rather than overwrite. Per check one `CHECK <name>=PASS|FAIL
  <detail>` line; the only column-0 `PREFLIGHT=` line is the last line. The log is written by redirection only —
  never through `tee`, which is a mutating command here.
- **Identity** = `hostname` prints `topgun-bench` and `nproc` prints `4`. **Isolation** = `pgrep -f
  'dokploy|traefik|dockerd'` exits 1 (0 = found, other = command failure, both FAIL); no file named `dokploy*`,
  `traefik*`, `docker*` (depth ≤ 2) in `/etc/systemd/system`, `/lib/systemd/system`, `/usr/lib/systemd/system`,
  `/run/systemd/system`, of which at least one must exist (`unit_dirs=absent` otherwise); `ss -Hltn` exits 0 and no
  local port is 8080 or 47376 (`ss` absent ⇒ FAIL). Units are read from the unit directories, not via `systemctl`,
  so isolation stays a pure read under the F1 stubs.
- **Test hooks.** `SPEC376_PREFLIGHT_SYS_ROOT` prefixes the THP knob dir (`<root>/sys/kernel/mm/transparent_hugepage`)
  **and** the four systemd unit directories; `SPEC376_PREFLIGHT_CMDLOG` receives each `mutate` argv line before it
  runs. **For G5's F1:** the scratch root must hold an (empty) `lib/systemd/system/` besides the two THP files, and
  the stub dir needs an `ss` stub (exit 0, no output) in addition to `uname`/`hostname`/`nproc`/`systemctl`/`tee` —
  without them isolation FAILs on the Mac and the last line reads `failed=identity,isolation`. Verified shape (G4, on
  stubs): exit 1, last line `PREFLIGHT=FAIL failed=identity PREFLIGHT_AT=…`, `MUTATIONS=0`, both THP files `cmp`
  identical.
- **Thresholds** (R9 table): load1 < 0.5 (one read; `--apply`: up to 21 reads 30 s apart = 10 min); steal share of
  `/proc/stat` `cpu` fields 2..9 over 10 s < 1 %; `swapon --show` empty with rc 0 and exactly one `SwapTotal: 0 kB`;
  both THP knobs bracket `[madvise]`; exactly one `MemAvailable:` ≥ 14680064 kB; `df -Pk /opt` available ≥ 41943040
  KiB; `timedatectl show -p NTPSynchronized` prints `NTPSynchronized=yes`; `command -v` for the eleven R9 tools.
  Timers: `systemctl is-active` must print exactly `inactive` for each unit.
- **Parity (R7.4).** `spec376-parity.sh <scratch>` refuses a scratch path in or under the evidence dir **before
  creating anything**; builds its own shim `<scratch>/awkbin-original/awk → original-awk`, asserts it resolves first
  and that the banner matches `^awk version [0-9]{8}`; validates the reference; runs the Linux copy; substitutes the
  scratch path (as passed and physical) by `@OUT@` as a literal; prints **exactly one** `SYNTH_PARITY=` line:
  `PASS cases=19 awk='<banner>'`, or `FAIL reason=<usage|scratch|no_original_awk|awk_shim|awk_banner|ref_missing|
  ref_empty|ref_sha_manifest|ref_sha_unlisted|ref_sha_dup|ref_sha_mismatch|ref_invalid|transcript_empty|diff lines=<n>|synth_rc=<n>>` (a diff is echoed with the `  | ` prefix);
  then `SYNTH_PARITY_MAWK=` / `SYNTH_PARITY_GAWK=` (`PASS rc=` / `FAIL diff_lines= rc=` / `n/a reason=absent`,
  recorded only; gawk via the two-line `exec gawk --posix "$@"` wrapper). Exit 0 PASS, 1 FAIL, 2 usage.
- **Reference validity (interpretation of R7.4 "no line matching `FATAL:`").** Valid = the section headers are
  exactly `--- synthetic S1` … `--- synthetic S19` in order, there are exactly 19 `^rc=[0-9]+$` lines, and no
  **column-0** `FATAL:` line. The frozen synth's cases S11, S12, S18, S19 *expect* the verdict program to FATAL; the
  synth echoes those lines indented (`  FATAL: …`), and they are four of the reference's 317 lines. A literal
  any-position match would reject the only correct reference; the guarded hazard — a synth that stops at its own
  setup (`FATAL: S10 setup …`, usage, evidence-dir refusal: all column 0, and each leaves < 19 sections) — is
  caught by both the column-0 rule and the section count.
- **Reference (R7.3).** `spec376-synth-ref-darwin.txt` = `spec376-parity.sh --darwin-ref <scratch>` (macOS only,
  BWK `awk version 20200816`, runs the FROZEN `spec373b-synth.sh`, validates, prints the substituted transcript).
  Captured from the main tree and from a `git worktree` at `d09325fd` with different scratch dirs: `cmp` identical,
  317 lines, 19 sections, sha256 `5dbe0225…7f1a`; no `@OUT@` occurs (the frozen synth prints no path). Transcript:
  `spec376-g4-mac.txt`. It is a data input, not a program (`is_program` does not match `.txt`); decided in G5
  (conductor ruling): §1 lists its sha256 under "Data inputs frozen at M", parity checks the reference against that
  line before trusting it, and ORDER re-hashes it.

### Predicates and calibration-reading contract (`spec376-predicates.sh`, `spec376-calib.sh`, G3)
- **Predicates output.** `spec376-predicates.sh <EV> spec376-<cell> <BUILDS>` prints the predicates block on stdout;
  the caller writes it to `spec376-<cell>.predicates.txt` (the chain: `> … 2>&1`). It writes
  `spec376-<cell>.{fits.txt,amp.txt,ampfpl.csv}` into EV. Exit 0 = every block ran; 2 = usage / unknown cell; 3 = a
  frozen source (`spec371-predicates.sh` for PM1, `spec349c2-fit.awk`) failed its sha256 — the dependent lines read
  `PM1=FALSE reason=frozen_source …` / `FIT_ERROR`, never a pass; 4 = an awk step failed. The chain logs it as
  `PREDICATES_EXIT_<cell>=`.
- **PEL.** Exactly the three substitutions on the parent's PE/PA block (column, `PE=`→`PEL=` in the awk program and in
  the `no_matrix_or_csv` line, detail name); the block's `== STOP: PE / PA ==` section header is left as the parent
  wrote it (it is not a key). No line of any predicates output starts with `PE=`.
- **PM1.** The counter `post_mortem_rows=` must occur exactly once in the runner console and be a plain count; absent
  / dup / non-numeric ⇒ `PM1=FALSE reason=no_counter_or_csv post_mortem_rows=<absent|dup|value>`.
- **PMEM.** `PMEM=TRUE mem_invariant_violations=0 sampler_fatal=0`, or `PMEM=FALSE reason=<console_missing|
  violations=absent|violations=dup|violations=<value>|sampler_fatal=<n>>` (a `SAMPLER FATAL` anywhere on a line of the
  runner console counts).
- **Calib inputs.** EV: `spec376-chain.log` (`PROC_ROOT=`, `PREFLIGHT_LOG=`, `PREDICATES_EXIT_<cell>=`,
  `LOAD_AT_START_<cell>=`), the preflight log it names (a basename matching `spec376-preflight-*.log`, read in EV), and per
  cal cell `spec376-<cell>.{predicates.txt,runner-console.log,csv,matrix.txt,soak.json}`; SMOKE_DIR:
  `spec376-chain.log`. `RUNNER_EXIT=` and `steal_pct=` are read from the runner console, `DURATION_<cell>` from the
  matrix's single `  duration: <n>s` line, `totalWrites` from the single `"totalWrites":` of `soak.json`, `FPL_end` from
  the predicates file; each exactly once, else named `absent` / `dup`.
- **Manifest.** The section-1 prefix (through `## APPEND-ONLY BELOW`; a manifest without the marker is refused, exit
  3) must carry exactly one `^CAL_PIN=` (hex, 7–40) and one `^PORT_EXPECT=[<lo>,<hi>] src=<src> not_a_gate` line.
  The band is read from it, never written into the program; `PORT_EXPECT=` prints the manifest's value verbatim.
- **Synthetic rule.** `SPEC376_SYNTHETIC=1` skips ORDER (printing `ORDER=SKIPPED (synthetic)`) only when EV, the
  manifest's directory and SMOKE_DIR all lie outside the evidence dir (equal or below counts as inside). Otherwise an
  unset `SPEC376_MANIFEST_COMMIT` is `ORDER=FAIL no manifest commit …`, exit 3.
- **Frozen rate program.** `spec373b-verdict.sh` sha256 asserted, lines 151–170 extracted by the command above, the
  last line must be `      }`, and the extracted text must hash to `35fb2300…01fd`; any failure exits 3.
- **STOP clause order.** H: `preflight_log=<absent|dup|value>` or `preflight=<absent|empty|FAIL|…>` first, then
  `<cell>:steal=<value|missing>` in cal order. V: `chain:proc_root=<value|absent|dup>` first, then per cell in cal
  order c1 pb c2 pa: `predicates_missing` or `PV PEL PA PM1 PMEM`, `predicates_rc`, `runner_exit`, `skipped`, `rate`,
  `live`, `totalWrites`; the pair clause `c1c2:write_parity=<%.4f|n/a>` last. `STOP=H (…)` wins over `STOP=V (…)`.
  Before the flags the reading prints `STOP_H_CLAUSES=` and `STOP_V_CLAUSES=` (both lists, `none` when empty) and
  `SMOKE_ADMISSION_SEEN=<PASS|FAIL|absent|dup|log_absent>`.
- **Flag formats.** `WRITES_PER_S_REF` `%.3f`; `S_CA_*` `%.6f`; `BYTES_ALLOC_RATE` / `ALLOC_LIVE` / `BYTES_PER_WRITE`
  (`%.1f`) / `STEAL` / `LOAD_AT_START` as `c1=<v> pb=<v> c2=<v> pa=<v>`; `WRITE_PARITY` / `WRITE_PARITY_ALL` /
  `PORT_RATIO` / `PORT_BPW_RATIO` `%.4f`; `A0_L_MIB` `%.3f` (mean over c1/c2 of `alloc_live_mb` on the row nearest
  t = 60 s within 30 s). Every missing input prints `n/a` (with `reason=` where one input decides it); `LOAD_AT_START`
  prints `absent` / `dup`, never a number, for a missing line; `PORT_IN_EXPECT=FALSE` whenever `PORT_RATIO` is `n/a`.
- **Intermediate.** A fresh `mktemp` under `TMPDIR`; a `TMPDIR` inside EV or the evidence dir exits 4. Nothing
  follows the flags block.

### Chain log contract (`spec376-chain.sh`, G2b)
Every line a reading keys on is printed at column 0, exactly once per log; anything echoed from another program
that is not such a key is prefixed `  | `. Logs: build `target/spec376-run/spec376-chain-build.log`; smoke
`target/spec376-run/smoke/spec376-chain.log` (committed at M as `evidence/spec376-smoke/spec376-chain.log`); cal
`evidence/spec376-chain.log`. Paths derive from the checkout root (`/opt/topgun` on the server).

| key | phase | value |
|---|---|---|
| `SMOKE_PROG_SHA=<file> sha256=<64 hex>` | smoke | one line per evidence-dir file matching `spec376-*.sh`, `spec376-*.awk`, `spec376-*.py` (the chain's `is_program`), sorted by name |
| `PROC_ROOT=/proc` | smoke, cal | one line, before the first cell |
| `PREFLIGHT_LOG=<basename>` | cal | the accepted preflight log (the lexicographically newest `spec376-preflight-*.log` in the evidence dir) |
| `LOAD_AT_START_<cell>=<1-min load or n/a>` | smoke, cal | one per cell, immediately before the pre-launch assertion and launch; `n/a` when `/proc/loadavg` is unreadable (calib reads it as `<cell>=absent`) |
| `PREDICATES_EXIT_<cell>=<rc>` | smoke, cal | one per cell, right after that cell; a non-zero rc does not stop the chain |
| `cell <cell>: RUNNER_EXIT=<rc>` | smoke, cal | the runner console carries `RUNNER_EXIT=<rc>` as its last line; `98` = not launched (pre-launch sha assertion failed) |
| `calib rc=<rc>` | cal | after the four `PREDICATES_EXIT_` lines |
| `ORDER=OK …`, `SMOKE_BINDING=PASS programs=<n>` | cal | before `PREFLIGHT_LOG=`; any failure refuses the phase before any cell |
| `DH_FRAME_<site literal>=<n>`, `DH_FRAMES=PASS|FAIL [reason=…]`, `SELF_CHECK=` | smoke | as R6.3 / R6.4 |
| `SMOKE_ADMISSION=PASS failed=none` / `SMOKE_ADMISSION=FAIL failed=<comma list>` | smoke | one line, last before `### SMOKE COMPLETE` |

Interfaces the chain fixes for later groups:
- **Builds file** (`target/spec376-run/spec376-builds.txt`): line 1 `build_start_epoch=<epoch>` (exported to every
  cell as `SPEC376_CHAIN_START_EPOCH`, because smoke and cal are later launches than the builds); eight lines
  `flavour=<label> code=<full sha> path=<abs> sha256=<hex> mtime=<epoch> recompiled=yes marker=ok`; then `rustc: …`,
  `glibc: …`, `awk: <banner>` lines. A reader selects a label's line by `$1 == "flavour=<label>"`, exactly once.
- **Build commands:** `DH-pin`, `JE-cal`, `MI-cal` and `SYS-cal` carry `--bin topgun-server` as the CA builds do,
  so only the server is built; `JE-cal` / `MI-cal` / `SYS-cal` build `--release`.
- **Preflight log (G4):** named `spec376-preflight-<stamp>.log` with a stamp that sorts lexicographically in time
  order (`YYYYMMDDTHHMMSSZ`); `PREFLIGHT_AT=` in `YYYY-MM-DDTHH:MM:SSZ`, exactly once on the last line.
- **smaps sample (R6.2):** `spec376-smaps-sample.txt` = `== pid=<pid> captured_at=<UTC> after_listener_s=<s>`,
  `== smaps_rollup` + the file, `== status` + the file, `== ps_rss` + `ps -o rss=`; written only whole. The admission
  replays it by splitting the sections into a fixture proc root (`<pid>/smaps_rollup`, `<pid>/status`,
  `<pid>.ps_rss`) and calling `procmem_row` there: rc 0 and twelve non-empty cells ⇒ PASS.
- **Smoke admission item definitions:** a `sje` probe row is a CSV row with a non-empty `je_probe_seq`; the
  `je_config` line is exactly one `^\[server\] je_config ` line in `spec376-sje.harness-console.log`; the `ssy`/`smi`
  marker-ok line is harness-console line 1 matching `^provenance: server sha256=<64 hex> flavour=<SYS|MI>( |$)`
  (the runner writes it only after its flavour-marker assertion passed) together with the label's `marker=ok`
  builds line; memory rows use the cell matrix's single `  duration: <n>s` line.
- **Program binding (G5/G6):** M's §1 must list exactly the `is_program` files the smoke printed, each once; a
  non-program input such as `spec376-synth-ref-darwin.txt` is outside the binding; §1 lists it separately under
  "Data inputs frozen at M" (parity and ORDER check it, G5).

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
