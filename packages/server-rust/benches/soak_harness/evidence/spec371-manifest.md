# SPEC-371 (carve 9a) — memory-growth diagnosis: pre-registration manifest

## §1 Pre-registration (frozen at commit M; nothing above the APPEND-ONLY marker changes after M)

### Question and scope
Why does the OR-churn soak's `phys_footprint` grow 1.7–1.8 GB/h while the live tag set is ~35 MB?
This carve measures, and changes nothing. It splits footprint growth into allocator RETENTION and
REACHABLE growth, measures the event journal's churn share, names the reachable holders by a dhat
growth-diff, and routes the next carve. No fix, gate, monitor or invariant change is made here.

### Fixed facts carried into this pre-registration
- **Journal entries are small.** `crdt.rs:928`: an OR_ADD journal entry carries the value of the ONE
  added tag (an i64 in the soak), never the resident record. The ring holds 10,000 entries and is
  bounded at single-digit MB, so the "10,000 × 370 KB ≈ 3.7 GB" hypothesis is refuted by the code.
  The journal-off cell c1 therefore measures the journal's per-op CHURN share, not a level.
- **The soak harness forced the journal on** (`process.rs:347`). K1 (`e69cb0c6`, `process.rs` only)
  passes the operator's value through (default `true`) and echoes it once per spawn.
- **No per-scrape live-bytes gauge exists.** Live tag bytes come from the census only: TERMINAL
  (mandatory) and LIVE_COPY (armed at 300 s here). `live_tag_bytes` counts tag STRINGS only, which is
  why `B_per_entry` and `AMP_redb` are reported beside AMP. A future gauge is TODO-693.
- **The count-alloc probe transport:** stderr `alloc_probe` every 30 s → the harness's `[server]`
  mirror → the harness console → four CSV columns (42–45).

### Statements the readout must carry (normative)
- **CLASS classifies the count-alloc regime.** The `stats_alloc` wrapper keeps libmalloc, but it
  moves allocator timing: SPEC-347 measured the wrapper alone cutting the RSS slope 7×, before delta
  framing. How far the CA regime sits from release is `CA_REGIME` over `G_ref = {G_r0, G_8e, G_8f}`,
  and it is a DECISION INPUT through `RELEASE_RETENTION`. That reading is **conditional on ops
  parity** (`OPS_RATIO = min(OPS_c0, OPS_c2)/OPS_r0 ≥ 0.80`), not true by construction: two atomic
  RMWs per allocation could also lower the ops served, and every slope is per hour. Below 0.80 the
  regime flags read `n/a reason=ops` and NEXT falls back to a ruling.
- **EARLY-WINDOW SCOPE.** All six cells read minutes 7–15 of a run, while the committed 4 h cells
  show a regime change near 2 h (`reclaimable_mb` peaks ≈ 4 GB and then falls while the footprint
  keeps rising). `CLASS`, `CA_REGIME` and `LEVER` are therefore early-window claims, and the
  acceptance gate of the carve this decision routes to is a ≥ 4 h plain-release cell on the same
  matrix, not a 900 s re-measure.
- **Post-mortem rows.** The row due at `T0 + D` lands after the harness has killed the server; such
  a row writes no scrape file, is counted, and is bounded by the STOP predicate `PM1` (count ≤ 1,
  late, and a live scrape within 2 cadences of the end). The frozen P6/P7 awk is untouched.
- **What `alloc_live` does not see:** memory outside the Rust global allocator (thread stacks, mmap
  regions made outside `GlobalAlloc`, kernel/IOKit accounting that `phys_footprint` includes) and
  libmalloc size-class rounding (`stats_alloc` counts requested sizes). This is why RETENTION is read
  at `R ≤ 0.10` rather than at zero.
- **dhat depth limit:** the profiler keeps the crate default of 10 frames per backtrace, so a 5-frame
  signature cannot reach below frame 10 of an allocation stack.
- **The census asymmetry inside `G_ref`:** `G_r0` is measured with the live-copy census ARMED, while
  `G_8e` and `G_8f` come from cells with it DISARMED. §3 records this beside `CA_REGIME` /
  `RELEASE_RETENTION`, and reports rather than averages away a `G_r0` far below both committed members.
- **Locale:** every program that parses a number runs under `LC_ALL=C`. On this host
  (`LANG=ru_RU.UTF-8`) the same frozen fit over the same committed CSV reads 1682.7 instead of
  1680.5 MB/h without it. Smoke item (f) proves the pin is live in the chain.

### Cells (spec R3.1, verbatim)
1. **Cell table.** `spec371-memdiag.sh <cell>`, with 900 s cadence 60, crash-interval 0, width unset
   and `live-census 300` in every row. The plain-release cell `r0` is a conductor scope choice, ruled
   at STOP 2 (rulings v1 R1): it supplies a same-chain, same-host release reference slope and the
   release-regime AMP, which a cross-day comparison with the committed 8e/8f cells cannot.
   **`c3e` runs 300 s with the census interval also at 300 s**, and the committed precedent fires
   that sampler at `elapsedSecs≈300.0007` (F3), i.e. at or after teardown begins: a `c3e` run with
   no LIVE_COPY row is EXPECTED, and AC-5 already tolerates it. Its TERMINAL census is unaffected.

| cell | duration | server flavour | `TOPGUN_JOURNAL_ENABLED` | teardown | `DHAT_OUT` | basename |
|---|---|---|---|---|---|---|
| r0  | 900 | R  | unset (harness default `true`) | SIGKILL (graceful unset) | unset | `spec371-r0` |
| c0  | 900 | CA | unset (harness default `true`) | SIGKILL (graceful unset) | unset | `spec371-c0` |
| c1  | 900 | CA | `false` (exported) | SIGKILL | unset | `spec371-c1` |
| c2  | 900 | CA | unset | SIGKILL | unset | `spec371-c2` |
| c3e | 300 | DH | unset | `TOPGUN_SOAK_GRACEFUL_SHUTDOWN=1` | `$META_DIR/spec371-c3e.dhat.json` (absolute) | `spec371-c3e` |
| c3l | 900 | DH | unset | `TOPGUN_SOAK_GRACEFUL_SHUTDOWN=1` | `$META_DIR/spec371-c3l.dhat.json` (absolute) | `spec371-c3l` |

   Every other matrix literal stays SPEC-370's (churn-clients 6, keyspace 200, or-churn true,
   or-keyspace 48, or-every 5, write-interval-ms 20, writes-per-life 200, offline-keys 3,
   confirm-interval 2, steady-interval 300, quiesce 3, mem-sample-interval 5, wal-fsync batched,
   mem gate neutralised at 1,000,000, `--mechanism-report --durable-reading`, jitter seed 20260831,
   `TOPGUN_PRUNE_RECORD=true`, the three-target log directive, `SOAK_SERVER_LOG_PASSTHROUGH=1`).
   `TOPGUN_JOURNAL_CAPACITY`, `TOPGUN_MAX_RAM_MB` and the eviction knobs are unset, as in SPEC-370.

### Per-cell predicates and readings (spec R4, verbatim)
### R4 — per-cell program `spec371-predicates.sh <EV> <BASE>` (truncates its outputs first)

**Locale (normative for EVERY spec371 program that parses a number).** `export LC_ALL=C` is
exported BEFORE ANY NUMERIC PARSE in the runner `spec371-memdiag.sh` (its sampler parses probe and
footprint decimals; R3 item 16), in `spec371-predicates.sh` (as the parent does at
`spec370-predicates.sh:10`, after its argument handling), in the chain `spec371-chain.sh` (which covers its gref step, R7 step 4,
and every program it invokes), and `spec371-decide.awk` is invoked only from that exported
environment. `spec371-dhat-diff.py` parses JSON numbers in Python and is locale-independent. This is load-bearing, not hygiene: under a
comma-decimal locale `awk` parses `144.110` as `144`, and the same fit over the same committed CSV
then reads 1682.7 instead of 1680.5 MB/h. Every fit, predicate and flag in this spec is computed
under `LC_ALL=C`.

**Where each value lands** (the split matters, because `decide.awk` reads only the first file):
- `<BASE>.predicates.txt` — EVERY `NAME=VALUE` line that `decide.awk` consumes: all STOP
  predicates, and the derived readings `G`, `G_se`, `G_n`, `L`, `L_se`, `L_n`, `LIVE_LH_mean`,
  `FP_end`, `LIVE_end`, `RET_end`, and the recorded `P5_c3*`, `PJ-child`, `AMP_*` summary lines it
  needs. One line per name; a missing STOP line counts as FALSE.
- `<BASE>.fits.txt` — the raw fitter records, as the parent writes them
  (`spec370-predicates.sh:20-26`). Human-readable provenance for the numbers above; not a decide
  input.
- `<BASE>.amp.txt` — the AMP table (below). Not a decide input, except through the `AMP_*` summary
  lines copied into `.predicates.txt`.

The program takes the builds file as its third argument (`spec371-predicates.sh <EV> <BASE>
<BUILDS>`), so `PV` never has to guess where `spec371-builds.txt` lives.

**STOP predicates (per cell):**
- `PV`: line 1 matches the R3.6 regex. The server sha equals `spec371-builds.txt` for the cell's flavour,
  and the harness sha equals flavour H.
- `PR-crashes`: 0 unexpected exits.
- `PR-class`: `spec371-<cell>.runner-console.log` (captured by the chain, R7) contains
  `RESULT: instrument sound; harness exit code N.` and no `RESULT: INSTRUMENT DEFECT`. A missing file
  is FALSE.
- `P5`, `P6`, `P7`: the FROZEN `spec366-p5.awk` and `spec366-p67.awk`, invoked exactly as
  `spec370-predicates.sh:33-34` invokes them.
  - They are **STOP on r0, c0, c1 and c2** (pre-audit D1). The release cell is the 8e/8f regime,
    and count-alloc keeps the THROUGHPUT regime even though it moves allocator timing (F4) — the
    two are different claims. A P5/P6/P7 FALSE on a CA cell caused by that timing shift would STOP
    the whole decision; it is recorded as a live risk here rather than carved out, because a
    settlement-discipline breach is not something this carve may ignore.
  - **On c3e and c3l they are RECORDED only**, printed as `P5_c3e=…`, `P6_c3l=…` and so on, and
    excluded from the STOP conjunction. They are SPEC-366 mechanism predicates (settlement
    discipline, prune windows). dhat takes a backtrace on every allocation and changes the regime,
    and C3 never enters CLASS or JOURNAL, so a FALSE there says nothing about naming holders.
- `PE`: evaluable rows ≥ ⌈0.95·E⌉, where E = ⌊duration/cadence⌋, both read from the cell's `matrix.txt` as the LEADING
  INTEGER of their lines (a smoke line reads `duration: 120s  <-- SMOKE OVERRIDE`)
  (15 for 900 s at 60 s, 5 for 300 s at 60 s; a smoke at cadence 20 gets its own E). A row is
  evaluable when `phys_footprint_mb` is non-empty and, in the CA cells only, the four alloc columns
  are non-empty. On r0 and the C3 cells the alloc columns are EMPTY by construction, so `PE` there
  counts `phys_footprint_mb` alone.
- `PA` (CA cells only; `PA=n/a` on r0 and on c3e/c3l): every evaluable row has `alloc_probe_elapsed_s ≥ elapsed_secs − 35`,
  `alloc_probe_seq` never decreases, and the last row's `alloc_probe_seq ≥ ⌊duration/30⌋ − 1`.
- `PJ`: the console has at least one `soak: child TOPGUN_JOURNAL_ENABLED=<v>` line, and every such line
  has v = `false` for c1 and v = `true` for every other cell.
- `PJ-child` (RECORDED, not STOP): the child-side confirmation. The server logs
  `event journal initialized` with `journal_enabled` at `info` (`topgun_server.rs:1837-1841`).
  - If a `[server] … event journal initialized … journal_enabled=<v>` line is present, the
    predicate prints `PJ-child=TRUE|FALSE`: v must be `false` on c1 and `true` elsewhere.
  - If the line is absent, it prints `PJ-child=n/a reason=filtered`.
  - Expected: `n/a`. The event's target is the binary's root module `topgun_server`, and the cell's
    directive `warn,topgun_server::tombstone_frontier::{removal,settlement,conjunct}=info` does not
    admit `info` for that target. The directive is NOT widened for this, because widening it would
    change the console volume against the SPEC-370 matrix.
- `PM1` (ruling v3 R3 + v3a A2): `post_mortem_rows ≤ 1` **and** every post-mortem row has
  `elapsed_secs ≥ D` **and** `last_live_scrape_elapsed ≥ D − 2·cadence`, where the last live scrape
  is the last CSV row with a non-empty `tombstone_bytes` (that column is filled from the scrape, so
  it is empty exactly when the scrape produced nothing). A post-mortem row before `D` means the
  server died early, which is a `PR-crashes` matter, and the staleness clause stops a cell whose
  scrapes went blind well before the end from passing on the counter alone.
- `PC`: a `TERMINAL` census line is present with `live_tag_bytes > 0` and `live > 0`.
- `PD` (checked on c3l, over both C3 cells): both `.dhat.json.gz` decode, and each has
  `dhatFileVersion == 2`, `mode == "rust-heap"` and `Σ eb > 0`. Two further conjuncts:
  - `PD-sym`: at least 80 % of the top-50 program points by `eb` in c3l have NO frame matching
    `\?\?\?:0:0|__mh_execute_header`. That is the unsymbolised-build signature from the stripped
    release profile.
  - `PD-crate`: at least one of the top-10 `Δeb` signatures (R5) contains `topgun_server`.

  `Profiler::builder()` is called without `trim_backtraces`, so the crate default of 10 frames
  applies (`dhat-0.3.x/src/lib.rs:1020`). An allocation deep inside redb or tokio can therefore
  legitimately carry no `topgun_server` frame: SPEC-347's top-10 had such rows. For that reason no
  per-pp crate rule is applied. `PD` = the conjunction of all of the above.

**Recorded readings:**
- Fits with the frozen `spec349c2-fit.awk`, window `last_half`, on `phys_footprint_mb`, `rss_mb`,
  `reclaimable_mb` and (CA only) `alloc_live_mb`. The outputs are `G` = footprint slope (MB/h),
  `L` = live slope, their `se`, and `r2`.
- **Ops parity** (v3a A1), per cell: `OPS_PER_S = totalWrites / durationSecsActual` and
  `WRITE_ERRORS`, read from `<BASE>.soak.json`. The soak load is paced, so parity across flavours is
  expected — but the wrapper's two atomic RMWs per allocation could also lower the ops served, and
  every slope is per hour, so this is read rather than assumed.
- `G_ref` (CHAIN-LEVEL, not per-cell: the chain computes it once at R7 step 4, after r0 and before
  the per-cell programs; it is described here because it is a fitter reading) is the SET
  `{G_r0, G_8e, G_8f}`, written to `spec371-gref.txt` with one line per member naming its source:
  - `G_r0` — this chain's own plain-release cell, the frozen fitter (`last_half`,
    `phys_footprint_mb`) over `spec371-r0.csv`;
  - `G_8e`, `G_8f` — the same fitter over the rows with `elapsed_secs ≤ 900` of the committed
    release cells `spec368-plateau4h.csv` and `spec370-plateau4h.csv`. At plan time these read
    1412.2 ± 214.1 and 1680.5 ± 241.9 MB/h, n = 8 each, **computed under `LC_ALL=C`**; the program
    recomputes them rather than trusting these numbers. Under a comma-decimal locale the same fit
    over the same file reads 1415.0 / 1682.7, because `awk` truncates `144.110` to `144` — which is
    why the locale export is a requirement and not hygiene.

  `min(G_ref)` and `max(G_ref)` range over all three. r0 is written FIRST in the file, because it is
  the only member from this chain's host, day and pin.
- `AMP_ratio_CA` (recorded, never a decision input; CROSS-CELL, so its owner is `decide.awk`, which
  prints it before the flag block from the `AMP_fp_terminal=` summary lines of r0/c0/c2):
  `AMP_fp(TERMINAL, c0) / AMP_fp(TERMINAL, r0)` and the same for c2 — how much more resident memory
  the count-alloc build holds per live tag byte than the release build.
- Levels: `FP_end` = last row `phys_footprint_mb`; `LIVE_end` = last row `alloc_live_mb`;
  `RET_end = FP_end − LIVE_end` (bytes the allocator holds beyond what is reachable);
  `LIVE_LH_mean` = mean `alloc_live_mb` over the last-half rows.
- **AMP** (`<BASE>.amp.txt`): for every census row (LIVE_COPY and TERMINAL) that joins to a CSV row
  within ±30 s, taking the nearest row; **TERMINAL joins to the LAST row whose `phys_footprint_mb`
  is non-empty**, because the final row of a run can be sampled while the server is being torn down.
  Every AMP line prints `row=<elapsed>` and `join_lag_s = t_census − row`:
  `AMP_fp = phys_footprint_bytes / live_tag_bytes`,
  `AMP_live = alloc_live_bytes / live_tag_bytes` (CA),
  `B_per_entry = phys_footprint_bytes / live`,
  `AMP_redb = phys_footprint_bytes / (redb_mb × 1048576)` at the same joined row. CSV column 4
  already exists. The redb file holds the full serialised records, whereas tag strings are only
  ~34 B of an entry: in 8f's terminal row `redb_mb = 214.5`, which gives `AMP_redb ≈ 40×` against
  `AMP_fp ≈ 240×`. Both are recorded, and neither decides anything.
  `copy_smear_s = copy_done − t`. Also `AMP_trend = AMP_fp(TERMINAL) / AMP_fp(first LIVE_COPY)`.
  Each line names its source (`LIVE_COPY` or `TERMINAL`). A census without a joinable row is printed
  with `row=none`, never dropped.

### dhat growth-diff (spec R5, verbatim)
### R5 — dhat growth-diff `spec371-dhat-diff.py <c3e.json.gz> <c3l.json.gz>`

1. Load `ftbl` and `pps` from each file. The two files come from the SAME DH binary, so the frame
   strings are comparable.
2. For each pp, the frame list is `[ftbl[i] for i in pp.fs]`, with any leading `0x…: ` address
   stripped.
3. Drop the leading allocator AND standard-library frames: every frame from the top whose function
   path starts with `[root]`, `dhat::`, `__rust_`, `alloc::`, `core::`, `std::`, `<alloc::`,
   `<core::`, `<std::`, `<T as alloc::` or `<T as core::`. Without the std prefixes the five kept
   frames are container internals (`Arc::allocate_for_slice`, `Vec::push`, `slice::to_vec`) and the
   token table cannot see the holder — measured on the smoke-2 diff, whose top grower read
   `UNMAPPED` for that reason alone.
4. **Signature** = the next 5 frames, joined with ` ← `. **Fallback:** when stripping leaves NO
   frame (dhat's 10-frame trim ended inside std), the signature is the literal `ALL_STD:` plus the
   first 3 raw frames, its lever is `UNMAPPED`, and the diff prints `all_std_pps=<count>` with their
   share of `Δeb`. If the top-1 `Δeb` signature is `ALL_STD`, `LEVER` is taken from the first
   non-`ALL_STD` signature and the diff prints `C3-top1-all-std=TRUE`, so a deep-std top grower
   cannot route a REACHABLE result to a ruling by construction. **Known depth limit:** dhat keeps at most
   10 frames per backtrace, because `trim_backtraces` is not set and the crate default `Some(10)`
   applies. The signature can therefore never reach deeper than frame 10 of the allocation stack,
   and a holder whose distinguishing frame lies deeper shows up under a shared allocator-side
   signature. Manifest §1 records this limit.
5. Aggregate `eb`, `ebk` and `gb` per signature per file. Growth is `Δeb = eb_late − eb_early`.
6. Emit `spec371-dhat-diff.txt`: the totals (`Σeb`, `Σgb`, `te` of each file), the top-10 by `Δeb`
   (rank, Δeb MB, Δebk, eb_early, eb_late, signature), and the top-10 by `eb_late` (END snapshot).
7. **Lever tokens (pre-registered).** The top-1 `Δeb` signature is matched as ONE string (all five
   frames joined). The tokens are tried in the LIST ORDER below, and the first token that occurs
   anywhere in the string wins; frame order does not matter:
   - `WriteBehindDataStore` → `591`
   - `record_journal|JournalStore` → `JOURNAL`
   - `apply_or_delta|update_in_place|merge_add` → `593`
   - `redb::` → `592`
   - `broadcast|ServerEventPayload|or_record` → `588`
   - otherwise `UNMAPPED`

   The diff prints `C3-top1-lever=<token>`, the top-3 levers, and **`lever_share`** (v3a A3): the
   sum of positive `Δeb` per lever token over the top-10 growers, including `UNMAPPED` and
   `ALL_STD`. `LEVER` stays top-1 as pre-registered; when the top-1 lever is not also the
   `lever_share` leader the diff prints `LEVER_CONTESTED=TRUE`. Both are recorded, never routing.

dhat throttles throughput, so C3 is used ONLY to NAME holders and never to compare slopes. Its CSV
slopes are recorded and do not enter the decision.

### Decision (spec R6, verbatim — the flags are computed after the STOP block)
### R6 — decision `spec371-decide.awk` (flags LAST)

**Closed input set** — the program reads exactly these files, and nothing else:
`spec371-{r0,c0,c1,c2,c3e,c3l}.predicates.txt`, `spec371-gref.txt` and `spec371-dhat-diff.txt`.
A missing input is named in the STOP block; it is never silently defaulted. For `G_ref`
specifically: a missing `spec371-gref.txt` or a missing member line is STOP with
`reason=gref_missing`; a member that is present but non-numeric or `FIT_ERROR` is treated as ≤ 0
(the R6.2 `n/a reason=gref` → `CONDUCTOR_RULING` path). It runs under
`LC_ALL=C` (R4 locale rule): the chain exports it, and the smoke invokes `decide.awk` through the
same exported environment.

1. `STOP=TRUE` if any STOP predicate in any cell fails. A STOP predicate PASSES when it reads
   `TRUE`, or when it reads the literal `n/a` **and R4 pre-declares `n/a` for that predicate on that
   cell** — today exactly `PA` on r0, c3e and c3l, where the alloc columns are empty by
   construction. Anything else fails, including `FALSE`, a missing line and an `n/a` that R4 does
   not pre-declare. All flags then print as `…=STOP` and the reason lines are listed.
   - The STOP set per cell is exactly the R4 STOP list: `P5`/`P6`/`P7` count on r0/c0/c1/c2 only, and
     `PD` counts once, on c3l.
   - Recorded lines (`P5_c3*`, `PJ-child`, fits, AMP) never enter it, and a missing recorded line
     is printed as missing but is not STOP.
   - The decision is computed AFTER this block, so no decision flag can read anything other than
     `STOP` under a STOP.
2. **Inputs.** `decide.awk` reads each predicates file by FIRST TOKEN: each line's first
   space-delimited token is `NAME=VALUE`; the key is the text before its first `=`, the value the
   text after it. Fields after the first space are extras and never part of the key or value.
   - `GMIN = min(G_c0, G_c2)`, `GMAX = max(G_c0, G_c2)`.
   - Every division is guarded: `R_c = L_c / G_c` for c ∈ {c0, c2} is computed ONLY after the
     growth floor in step 3 has passed, so `G_c ≥ 200` there.
   - `G_c1` is never a divisor; no `R_c1` is computed.
   - **Ops guard (v3a A1), evaluated BEFORE the regime:** `OPS_RATIO = min(OPS_c0, OPS_c2) / OPS_r0`
     (and `OPS_RATIO_c1` recorded). If `OPS_RATIO < 0.80`, or `OPS_r0 ≤ 0`, or a reading is
     missing, then `CA_REGIME=n/a reason=ops`, `RELEASE_RETENTION=n/a`, and NEXT takes the same
     fallback as a bad `G_ref` (`CONDUCTOR_RULING`). `OPS_RATIO` prints next to `RET_SHARE_REL`.
   - `CA_REGIME` divides by `min(G_ref)` and `max(G_ref)`; if any member of `G_ref` is ≤ 0, it
     prints `CA_REGIME=n/a reason=gref`, `RELEASE_RETENTION=n/a reason=gref`, and NEXT falls back to
     `CONDUCTOR_RULING` (a missing regime reading must not silently drop the retention prefix).
3. **CLASS**
   - `INDETERMINATE_NO_GROWTH` if `GMIN < 200` MB/h. A 900 s window that shows no footprint growth
     cannot split it, so the conductor rules the next step.
   - else `RETENTION` if `R_c0 ≤ 0.10 ∧ R_c2 ≤ 0.10`. Live bytes are flat while the footprint rises.
   - else `REACHABLE` if `R_c0 ≥ 0.50 ∧ R_c2 ≥ 0.50`.
   - else `MIXED`, printed with `reachable_share=[min(R_c0,R_c2), max(R_c0,R_c2)]` and
     `reachable_mid=(R_c0+R_c2)/2`.
   - `REPLICATE_AGREE=TRUE|FALSE` is computed after the floor: TRUE iff `band(R_c0) == band(R_c2)`,
     where `band(R) = RETENTION` if `R ≤ 0.10`, `REACHABLE` if `R ≥ 0.50`, and `MIXED` otherwise. On `INDETERMINATE_NO_GROWTH` it prints
     `n/a`.
   - **What `alloc_live` does not see** (stated in manifest §1): memory outside the Rust global
     allocator, meaning thread stacks, mmap'd regions made outside `GlobalAlloc`, and kernel/IOKit
     accounting that `phys_footprint` includes; and the size-class rounding inside libmalloc
     (`stats_alloc` counts requested sizes). That is why RETENTION is read at `R ≤ 0.10` rather than
     at zero.
   - **Regime:** CLASS classifies the count-alloc regime (F4, A6).
   - `R_c` uses point estimates of `G` and `L` (pre-registered). §3 quotes `G ± se`, `L ± se` and
     the last-half row count `n` of each fit, per cell, and states in one line whether CLASS would
     differ at ±1 se. That line is a reading, not a flag.
4. **JOURNAL** is a **churn-share reading, not a level reading.** At ~300 ops/s the 10,000-entry
   ring fills within ~35 s, so its level is constant over every last-half window and cannot enter a
   slope. `HOLDER` means "the journal's per-op churn drives at least half of the footprint slope",
   and `NOT_HOLDER` must NOT be read as "the ring is small": the ring's level is `JLIVE`, recorded
   below. The comparison is against the baseline RANGE, never against one run.
   - `HOLDER` if `G_c1 ≤ 0.5·GMIN`;
   - `NOT_HOLDER` if `G_c1 ≥ GMIN`;
   - `UNRESOLVED` otherwise.
   - On `INDETERMINATE_NO_GROWTH`, JOURNAL prints `n/a`.
   - Also recorded: `JLIVE = LIVE_LH_mean(c1) − min(LIVE_LH_mean(c0), LIVE_LH_mean(c2))` in MB. This
     is the ring's reachable LEVEL, and a negative value of about the ring's size is expected.
   - `F1_FORECAST_HELD = TRUE` iff `JOURNAL == NOT_HOLDER`, `FALSE` iff `JOURNAL == HOLDER`, and
     `UNRESOLVED` otherwise (including `n/a`).
5. **CA_REGIME** — a DECISION INPUT (ruled at STOP 2, rulings v1 R2), printed in the final flag
   block:
   - `COMPARABLE` if `GMIN ≥ 0.5·min(G_ref)`;
   - `SUPPRESSED` if `GMAX < 0.5·min(G_ref)`;
   - `PARTIAL` otherwise.

   It is printed together with both ratios `GMIN/max(G_ref)` and `GMAX/min(G_ref)`.
   - **`RELEASE_RETENTION`** = `TRUE` iff `CA_REGIME = SUPPRESSED`, `FALSE` iff `COMPARABLE`,
     `PARTIAL` iff `PARTIAL`. The reasoning is mechanical: the wrapper changes allocator timing and
     nothing else, so growth that the wrapper alone removes was retention by construction.
   - Recorded beside it: `RET_SHARE_REL = max(0, 1 − GMAX/min(G_ref))` — a lower bound on the
     retention share of the RELEASE slope.
   - §3 reads `SUPPRESSED` as "the release binary's extra growth is allocator retention, and CLASS
     understates retention".
   - **`CLASS_FRAGILE`** (v3a A3, recorded before the flag block, changes no flag): TRUE iff for c0
     or c2 the band of `(L − se) / (G + se)` differs from the band of `(L + se) / (G − se)`; `n/a`
     when a standard error is missing or CLASS is `INDETERMINATE_NO_GROWTH`. §3 states it, and a
     MIXED or REACHABLE result under `CLASS_FRAGILE=TRUE` is read as provisional.
   - `LEVER_CONTESTED` and `C3-top1-all-std` are copied from the diff and printed beside it.
6. **LEVER** = `C3-top1-lever` (R5.7) when CLASS ∈ {REACHABLE, MIXED}, and `n/a` otherwise.
7. **NEXT** is a literal for every combination; there is no free text.
   - The lever maps to a route through `route(l)`:

     | `l` | `route(l)` |
     |---|---|
     | `591` | `TODO-591` |
     | `593` | `TODO-593` |
     | `592` | `TODO-592` |
     | `588` | `TODO-588` |
     | `JOURNAL` | `JOURNAL-CAP-SLIM` |
     | `UNMAPPED` | `CONDUCTOR_RULING` |

   - The allocator route is `RET = TODO-590+TODO-591`.
   - The reachable route is `REACH = JOURNAL-CAP-SLIM` when `JOURNAL=HOLDER`, and `route(LEVER)`
     otherwise.

   | CLASS | JOURNAL / RELEASE_RETENTION | NEXT (base value) |
   |---|---|---|
   | STOP (any) | any | `STOP` |
   | INDETERMINATE_NO_GROWTH | `RELEASE_RETENTION=TRUE` | `TODO-590+TODO-591` |
   | INDETERMINATE_NO_GROWTH | `RELEASE_RETENTION ∈ {FALSE, PARTIAL}` | `CONDUCTOR_RULING` |
   | RETENTION | JOURNAL=HOLDER | `JOURNAL-CAP-SLIM;THEN;TODO-590+TODO-591` |
   | RETENTION | JOURNAL=NOT_HOLDER / UNRESOLVED | `TODO-590+TODO-591` |
   | REACHABLE | any | `REACH` (as defined above) |
   | MIXED | any, `reachable_mid ≥ 0.5` | `REACH;THEN;TODO-590+TODO-591` |
   | MIXED | any, `reachable_mid < 0.5` | `TODO-590+TODO-591;THEN;REACH` |

   - **No-growth is no longer an open branch.** A `GMIN < 200` result with `RELEASE_RETENTION=TRUE`
     is exactly the A6(ii) signature — the release reference grew while the CA cells did not — so it
     routes to the allocator carve without a ruling. With `FALSE` or `PARTIAL` the release reference
     did not grow either, or grew only partly, which is a workload question, so it routes to the
     conductor.
   - **`REACH` is a placeholder in this table only.** The printed string always carries the
     RESOLVED route (`TODO-591`, `TODO-593`, `TODO-592`, `TODO-588`, `JOURNAL-CAP-SLIM` or
     `CONDUCTOR_RULING`); the literal token `REACH` is never printed.
   - **Retention prefix.** For every CLASS other than `INDETERMINATE_NO_GROWTH`, if
     `RELEASE_RETENTION=TRUE` then NEXT = `TODO-590+TODO-591;THEN;<base value>` — UNLESS the base
     value already contains the token `TODO-590+TODO-591` ANYWHERE, in which case the base value is
     printed unchanged. The token therefore appears at most once in NEXT. (Containment, not a
     prefix test: `RETENTION × JOURNAL=HOLDER` and `MIXED, mid ≥ 0.5` both carry the token at the
     END of the base value.)
   - **Suffix order is fixed:** `;JOURNAL_UNRESOLVED` first (when JOURNAL=UNRESOLVED), then
     `;CA_PARTIAL` (when `RELEASE_RETENTION=PARTIAL`). Both may appear, in that order and no other.
   - The complete grammar of NEXT is therefore
     `STOP` alone, or `<route>(;THEN;<route>)*(;JOURNAL_UNRESOLVED)?(;CA_PARTIAL)?`.
     `CONDUCTOR_RULING` is a `<route>` like any other, so it CAN carry suffixes (e.g.
     `CONDUCTOR_RULING;CA_PARTIAL` on no-growth × PARTIAL); only `STOP` is suffix-free.
   - The ordering in the MIXED rows is by `reachable_mid`, the midpoint of the replicate R values.
   - `JOURNAL-CAP-SLIM` names the journal carve, which belongs to the TODO-588 family; the literal
     stays token-shaped so that a machine reader can split it on `;`.
8. The final lines, in order, are `CLASS=`, `REPLICATE_AGREE=`, `JOURNAL=`, `F1_FORECAST_HELD=`,
   `CA_REGIME=`, `RELEASE_RETENTION=`, `LEVER=`, `NEXT=` — eight flags. Under STOP every one of
   them reads `STOP`.

### Programs frozen at M
- `spec371-memdiag.sh` — the per-cell runner (a copy of `spec370-plateau4h.sh`, closed difference
  list of 17 items; hunk map below)
- `spec371-chain.sh` — builds, cells, gref, dhat diff, predicates, decide; `SPEC371_SMOKE=1` runs
  the R8 admission smoke
- `spec371-predicates.sh`, `spec371-decide.awk`, `spec371-dhat-diff.py`
- Unedited inputs: `spec349c2-fit.awk`, `spec366-p5.awk`, `spec366-p67.awk`,
  `spec368-plateau4h.csv` (8e) and `spec370-plateau4h.csv` (8f) for `G_ref`

### Runner diff: every hunk maps to one of the 17 items
See `diff spec370-plateau4h.sh spec371-memdiag.sh`; the map is the table below, extended by item 17
(post-mortem rows) and by the alloc-probe read moving to the end of `emit_row`.

| diff hunk (`diff spec370-plateau4h.sh spec371-memdiag.sh`) | item |
|---|---|
| `2a3,67` | 9 — this runner's header (the parent's header follows verbatim) |
| `157a223` | 16 — `export LC_ALL=C` before any numeric parse |
| `168c234`, `170,171c236,237`, `174,182c240,248`, `186,187c252,253`, `191c257,258`, `193,195c260,262` | 9 — usage text |
| `211a279,288` | 1 (flavour/journal/graceful column doc) + 11 (base-suffix refusal) |
| `213,217c290,296`, `218a298` | 1 — the six-cell table and the fixed basename |
| `302c382` | 4 — data dir `target/spec371-<cell>-data` |
| `305a386,387` | 17 — `PM_FILE`, the post-mortem counter's file |
| `337a420,427` | 15 — smoke-only live-census override, refused outside smoke mode |
| `386c476,491` | 10 — per-cell env block (graceful, journal, DHAT_OUT; capacity unset) |
| `430c535` | 4 — port 47357 |
| `456,458c561,563`, `469,470c574,575`, `477c582` | 2 + 3 — freeze variable renamed, literal = K1 `e69cb0c6` |
| `492,513c597,601`, `514a603,608` | 5 — the runner builds nothing; `SPEC371_HARNESS_BIN` and `SPEC371_CHAIN_START_EPOCH` required |
| `539a634,647` | 12 — server flavour-marker assertion |
| `560,563c668,670`, `565,566c672,673` | 5 — server freshness clause (b) against the chain start |
| `579c686` | 6 — `flavour=` on console line 1 |
| `588,599c695` | 5 — the harness is `SPEC371_HARNESS_BIN` |
| `621,625c717,726` | 12 (journal-echo literal) + 5 (harness freshness clause (b)) |
| `635,636d735`, `641,649d739` | 14 — BUILD_GAP warning and its two mtime assignments removed |
| `664c754` | 17 — the post-mortem counter file is reset with the other sampler state |
| `736c826`, `740c830,831`, `751c841`, `785a876,880` | 6 — matrix banner, lineage, chain start, flavour/shas/per-cell env echo |
| `749d839` | 8 — the matrix no longer names a readout file |
| `753c843` | 2 — freeze variable name in the matrix |
| `871c966,967`, `877c973` | 7 — the CSV header gains the four probe columns |
| `888c984`, `890a987,1007` | 17 — `scrape_prune_metrics` takes the pid and skips the scrape file for a post-mortem row |
| `1064c1181` | 17 — the pid is passed to the scrape |
| `1077c1194,1211`, `1086a1221` | 7 — the probe read (now the LAST step of `emit_row`, ruling v3 R3) and the widened row `printf` |
| `1159a1295,1298` | 17 — `post_mortem_rows` on the runner console and in `matrix.txt` |
| `1269a1409,1417` | 8 + 13 — dhat profile gzip; missing profile = INSTRUMENT DEFECT |
| `1284,1295d1431` | 8 — the parent's readout invocation dropped |

## APPEND-ONLY BELOW

## §3 — carve 9a readout: the release regime grows, the count-alloc regime does not

Appended 2026-09-20, after the data commit D = `48e31816`
(`docs(soak): record the carve 9a memory diagnosis cells`, 161 artifacts, 0 `.rs`/`.sh`/`.awk`/`.py`).
Nothing above `## APPEND-ONLY BELOW` changed: M = `12627ad3` is an ancestor of D, and the §1 prefix
sha256 (lines 1 through the marker, inclusive) is
`7efeb27782811684c9b947fb996660c78d3120fa9987114877d9bef54cda36a9` at M and at HEAD. `ORDER=OK`.

The chain ran once, detached, from `chain start: 2026-09-20T09:55:30Z` to
`chain end: 2026-09-20T11:35:21Z` (`spec371-chain.log`). **What carries the weight is the six
committed `RUNNER_EXIT=0` lines**, one per `spec371-<cell>.runner-console.log`, each beside a
`RESULT: instrument sound`. The chain script does not print its own exit code, so no `CHAIN_RC`
appears in any committed artifact and none is claimed here; that is a residue for the next runner
lineage to fix when it is copied, not a re-cut of this one.

Everything below was recomputed under `LC_ALL=C`, which is the only locale in which the committed
fits reproduce (§1: under `ru_RU` the same fit over the same file reads 1682.7 instead of 1680.5).

### 3.1 The eight flags, verbatim

`spec371.decision.txt`, `STOP=FALSE`, no `STOP-reason:` lines:

```
CLASS=INDETERMINATE_NO_GROWTH
REPLICATE_AGREE=n/a
JOURNAL=n/a
F1_FORECAST_HELD=UNRESOLVED
CA_REGIME=SUPPRESSED
RELEASE_RETENTION=TRUE
LEVER=n/a
NEXT=TODO-590+TODO-591
```

This is branch A6(ii) firing exactly as pre-registered — `INDETERMINATE_NO_GROWTH` ∧
`RELEASE_RETENTION=TRUE`, with ops parity held — so the route is a table literal from R6.7 and is
not re-derived here. `NEXT = TODO-590 + TODO-591`.

**The result in one line, qualified.** *On the two pre-registered baseline cells* c0 and c2, the
counting wrapper removes 85–97 % of the release-regime growth at the same ops rate. That is a
statement about c0 and c2 only: c1 ran the SAME count-alloc binary and grew 1858 MB/h, so
suppression is not a property of the binary. What the chain shows across all three count-alloc cells
is the retention reading of §3.5.

### 3.2 The recorded readings above the flags, verbatim

```
GMIN=68.3343 GMAX=211.731 G_c0=211.731 G_c2=68.3343 G_c1=1857.97 L_c0=225.604 L_c2=221.094
OPS_PER_S r0=183.05 c0=179.227 c1=184.14 c2=181.213
OPS_RATIO=0.979 OPS_RATIO_c1=1.006
G_ref min=1412.23 max=2107.07 ratio_GMIN_over_max=0.032 ratio_GMAX_over_min=0.150
RET_SHARE_REL=0.850
JLIVE=0.362
AMP_ratio_CA c0=0.246
AMP_ratio_CA c2=0.159
CLASS_FRAGILE=n/a
LEVER_CONTESTED=FALSE
C3-top1-all-std=FALSE
```

`G_ref`, all three members with their sources (`spec371-gref.txt`):

```
G_ref member=r0 slope=2107.069286 se=167.477578 n=8 rows=all    source=spec371-r0.csv
G_ref member=8e slope=1412.225714 se=214.120481 n=8 rows=le900  source=spec368-plateau4h.csv
G_ref member=8f slope=1680.520255 se=241.860652 n=8 rows=le900  source=spec370-plateau4h.csv
```

`G_r0` sits ABOVE both committed members rather than below them, so the census asymmetry §1 flagged
(`G_r0` measured with the live-copy census ARMED, `G_8e`/`G_8f` with it DISARMED) did not depress
this chain's own release reference. The three members are reported, not averaged.

Ops parity was evaluated before the regime, as v3a A1 requires: `OPS_RATIO = 0.979` against the
0.80 floor, `WRITE_ERRORS=0` in every cell. The `SUPPRESSED` reading is therefore conditional on a
condition that was met, not true by construction.

### 3.3 Per-cell fits, `G ± se (n)` and `L ± se (n)`

Frozen `spec349c2-fit.awk`, window `last_half`, `G` on `phys_footprint_mb`, `L` on `alloc_live_mb`:

| cell | `G ± se (n)` MB/h | `L ± se (n)` MB/h | `FP_end` | `LIVE_end` | `RET_end` | `LIVE_LH_mean` |
|---|---|---|---|---|---|---|
| r0  | 2107.07 ± 167.48 (8) | — (release: no alloc columns) | 387.0 @840 | — | — | — |
| c0  | 211.73 ± 61.50 (8)   | 225.60 ± 14.08 (8) | 93.2 | 66.9 | 26.3 | 52.6 |
| c1  | 1857.97 ± 117.96 (8) | 232.12 ± 21.48 (8) | 356.0 @840 | 67.1 | — | 52.9 |
| c2  | 68.33 ± 49.76 (8)    | 221.09 ± 24.93 (8) | 60.8 | 67.4 | −6.7 | 52.9 |
| c3e | 209.55 ± 78.76 (3)   | — (dhat) | 50.5 | — | — | — |
| c3l | 60.59 ± 27.55 (8)    | — (dhat) | 64.6 | — | — | — |

`FP_end` for r0 and c1 is the last row that CARRIES a footprint, t = 840; both cells' t = 900 row has
an empty `phys_footprint_mb` (r0's is the post-mortem row `PM1` counted, c1's is a live scrape whose
`ps` sample did not land — `tombstone_bytes=25124` is present on that row). **c1's `RET_end` is absent rather than computed:**
`FP_end` and `LIVE_end` would come from different rows there (840 and 900), so the predicate program
printed no `RET_end=` line at all rather than subtract across a row boundary. c3e's fit has n = 3,
which is why its slope is a level marker and nothing more. The dhat cells' slopes are recorded and
never enter the decision (§1 R5).

`CLASS_FRAGILE=n/a`, and the reason is definitional, not a missing input: the fragility test compares
the band of `(L − se)/(G + se)` against the band of `(L + se)/(G − se)`, and R6.5 defines it only for
a banded CLASS. `INDETERMINATE_NO_GROWTH` is the growth-floor branch, decided before any `R_c` is
computed (R6.2 forbids the division below the floor), so there is no band to perturb. The
±1 se question that R6.3 asks §3 to answer therefore has a one-line answer: at ±1 se, `GMAX` spans
150–273 MB/h and `GMIN` spans 19–118 MB/h, both below the 200 MB/h floor at the low end and `GMIN`
below it at every point — CLASS does not change at ±1 se.

`RET_end = −6.7 MB` on c2 means `alloc_live` exceeds `phys_footprint` there. That is expected when
the allocator has returned pages the counter still counts as live; it is a recorded level, never a
decision input.

### 3.4 Amplification at TERMINAL

Per §1 R4: `AMP_fp = phys_footprint_bytes / live_tag_bytes`, `AMP_redb = phys_footprint_bytes /
(redb_mb × 1048576)`, `AMP_live = alloc_live_bytes / live_tag_bytes`, `B_per_entry =
phys_footprint_bytes / live`. `live_tag_bytes` counts tag STRINGS only (~34 B/entry), which is why
`AMP_redb` and `B_per_entry` travel beside `AMP_fp`.

| cell | joined `row=` | `join_lag_s` | `live` | `AMP_fp` | `AMP_redb` | `AMP_live` | `B_per_entry` |
|---|---|---|---|---|---|---|---|
| r0  | 840 | 60.3 | 64,602 | 184.8 | 23.35 | n/a  | 6,282 |
| c0  | 900 |  0.3 | 63,115 |  45.5 |  5.62 | 32.7 | 1,548 |
| c1  | 840 | 60.2 | 65,193 | 168.4 | 21.48 | 30.0 | 5,726 |
| c2  | 900 |  0.1 | 63,781 |  29.4 |  3.67 | 32.6 |   999 |
| c3e | 300 |  7.6 |  9,011 | 172.7 | 14.31 | n/a  | 5,871 |
| c3l | 900 | 11.3 | 20,972 |  95.1 |  9.98 | n/a  | 3,232 |

`AMP_ratio_CA` = 0.246 (c0) and 0.159 (c2): the count-alloc build holds 4–6× LESS resident memory
per live tag byte than the release build. **r0 and c1 joined at row 840**, one cadence back, because
their t = 900 rows carry no footprint; their `join_lag_s` of ~60 s is that fact, not a census smear.
Note `AMP_live` is flat at 30–33 across all three count-alloc cells while `AMP_fp` spans 29 to 168 —
the reachable side does not move, the resident side does.

### 3.5 c1 — the strongest retention reading in the chain, and what it is NOT

c1 is the journal-OFF cell. It ran the same count-alloc binary as c0 and c2, at parity
(`OPS_RATIO_c1 = 1.006`, `WRITE_ERRORS=0`, `PJ=TRUE echoes=1 value=false`), and:

- **reachable bytes are identical to the baselines:** `LIVE_end` 67.1 MB against 66.9 (c0) and 67.4
  (c2); `LIVE_LH_mean` 52.9 against 52.6 and 52.9; `JLIVE = +0.36 MB`;
- **the footprint is 3.7× and 5.9× the baselines at the matched row t = 840** (356.0 MB against
  96.8 MB on c0 and 60.8 MB on c2), with `G_c1 = 1858 ± 118` against 212 ± 62 and 68 ± 50. The
  comparison is made at a row all three cells carry: c1's t = 900 row has no footprint (below), so
  reading c1@840 against c0/c2@900 would flatter the ratio at its low end.

Same reachable heap, ~4–6× the footprint, one binary. That is the retention reading, and
it is the plainest one the chain produced.

**§3 makes no causal claim about the journal.** n = 1; c1's allocator span equals c0's (§3.6: 569 vs
533 MB, while c2 reads 187 — the span variance is not aligned with the journal switch); and the
ring's reachable level is `JLIVE = 0.36 MB`, which F1's code reading (`crdt.rs:928`: an OR_ADD entry
carries the ONE added tag's value, never the resident record) predicted at exactly that order of
magnitude. Turning the journal OFF did not lower the footprint. `JOURNAL=n/a` by the R6.4 table,
because CLASS took the no-growth branch and nothing was routed on it. **No replicate is ordered:**
it cannot change `NEXT`, which is already the allocator carve.

### 3.6 POST HOC — the allocator's touched span `S` (decides nothing)

Labelled POST HOC: this definition was not pre-registered, it changes no flag, and it is recorded
because it is what the next carve's gate has to be designed against.

The runner already records `reclaimable_mb` — pages the allocator has marked reusable, which sit
OUTSIDE `phys_footprint`. Define the allocator's touched span as

    S = phys_footprint_mb + reclaimable_mb

Re-derivable from the committed CSVs with awk alone, using the same floor-biased last-half split as
the frozen fitter:

```awk
# spec371 §3.6 — S = phys_footprint_mb + reclaimable_mb; last-half OLS slope in MB/h.
# usage: LC_ALL=C awk -f this.awk spec371-<cell>.csv
BEGIN { FS = "," }
NR == 1 { for (i = 1; i <= NF; i++) { if ($i == "elapsed_secs")      e = i
                                      if ($i == "phys_footprint_mb") f = i
                                      if ($i == "reclaimable_mb")    r = i }
          next }
$f != "" && $r != "" { t[n] = $e + 0; fp[n] = $f + 0; rc[n] = $r + 0
                       s[n] = fp[n] + rc[n]; n++ }
END { st = int(n / 2); m = n - st                  # same split as spec349c2-fit.awk
      for (i = st; i < n; i++) { x[i] = t[i] / 3600.0; sx += x[i]; sy += s[i] }
      xb = sx / m; yb = sy / m
      for (i = st; i < n; i++) { d = x[i] - xb; sxx += d * d; sxy += d * (s[i] - yb) }
      printf "rows=%d n=%d t_end=%d fp_end=%.0f reclaimable_end=%.0f S_end=%.0f S_slope_mb_per_h=%.0f\n",
             n, m, t[n-1], fp[n-1], rc[n-1], s[n-1], sxy / sxx }
```

Its output on the four 900 s cells (the dhat cells are excluded: dhat runs at a third of the ops
rate and its span is not comparable):

| cell | `fp_end` | `reclaimable_end` | `S_end` | `S` slope (MB/h) |
|---|---|---|---|---|
| r0 | 387 | 466 | 853 | 5106 |
| c0 |  93 | 440 | 533 | 2847 |
| c1 | 356 | 213 | 569 | 3167 |
| c2 |  61 | 126 | 187 |  508 |

Three readings, none of them a decision:

1. **c1 is NOT an outlier in span** (569 against c0's 533). What differs between c1 and c0 is the
   SHARE the allocator marked reusable — 213 MB against 440 MB — not how much memory it touched.
   That is the arithmetic reason §3.5 forbids a journal-causality reading.
2. **c0 and c2 differ 2.9× in span on identical configuration** (533 vs 187). The known run-to-run
   dirty-set variance lives in `S`, not only in the footprint, and it is the reason the spec never
   compares single runs.
3. **In every cell `S` grows at 0.5–5 GB/h while reachable bytes grow at ≈ 0.22 GB/h**
   (`L_c0 = 225.6`, `L_c2 = 221.1`). The gap is the retention this carve was built to find.

**The same definition on the committed 8f 4 h cell**, rows at 2400 / 4800 / 7200 / 9600 / 12000 /
14400 s of `spec370-plateau4h.csv`:

| `t` (s) | 2400 | 4800 | 7200 | 9600 | 12000 | 14400 |
|---|---|---|---|---|---|---|
| `phys_footprint_mb` | 1540 | 3237 | 4940 | 5275 | 6889 | 8237 |
| `reclaimable_mb`    | 2908 | 2938 | 4035 | 2980 | 1547 | 1130 |
| `S` | 4449 | 6175 | 8975 | 8256 | 8436 | 9367 |

After ~2 h `S` is roughly flat at 8.3–9.4 GB while the footprint keeps rising INTO it, as reusable
pages are re-dirtied. The "+1.7 GB/h footprint over the last half" of 8e/8f is therefore largely a
CONVERSION inside a span that had already stopped growing — which is the early-window caveat
(rulings v3a A4, §1) made concrete. This is a reading for the next carve's gate design. **It is not a
plateau claim**, and §3 does not make one.

### 3.7 POST HOC — reachable bytes per live OR entry (decides nothing)

`alloc_live` rose ≈ 50–51 MB between t = 60 and the last row on all three count-alloc cells, while
the census gained ≈ 59–61 k live entries. The live count at t = 60 is not measured — no census fires
there — so it is interpolated linearly from the origin through the first `LIVE_COPY`, which is the
only live-count series a cell records. The third digit of the result is that interpolation, not a
measurement:

```awk
# spec371 §3.7 — reachable bytes per live OR entry, t=60 -> last row, count-alloc cells.
# usage: LC_ALL=C awk -f this.awk spec371-<cell>.csv spec371-<cell>.amp.txt
BEGIN { FS = "," }
FILENAME ~ /\.amp\.txt$/ { FS = " " }
FNR == 1 && FILENAME ~ /\.csv$/ { for (i = 1; i <= NF; i++) {
        if ($i == "elapsed_secs") e = i; if ($i == "alloc_live_mb") a = i } ; next }
FILENAME ~ /\.csv$/ && $a != "" { if (($e + 0) == 60) a60 = $a + 0; aN = $a + 0 }
FILENAME ~ /\.amp\.txt$/ {
        for (i = 1; i <= NF; i++) { split($i, kv, "="); k[NR, kv[1]] = kv[2] }
        if (k[NR, "source"] == "LIVE_COPY" && c1t == 0) { c1t = k[NR, "t"] + 0; c1l = k[NR, "live"] + 0 }
        if (k[NR, "source"] == "TERMINAL")              { tl  = k[NR, "live"] + 0 } }
END { l60 = c1l * 60.0 / c1t
      printf "d_alloc_live_mb=%.2f live_t60_est=%.0f live_terminal=%d d_live=%.0f B_per_live_entry=%.0f\n",
             aN - a60, l60, tl, tl - l60, (aN - a60) * 1048576 / (tl - l60) }
```

```
c0  d_alloc_live_mb=51.31 live_t60_est=4150 live_terminal=63115 d_live=58965 B_per_live_entry=912
c1  d_alloc_live_mb=50.13 live_t60_est=4299 live_terminal=65193 d_live=60894 B_per_live_entry=863
c2  d_alloc_live_mb=50.15 live_t60_est=4141 live_terminal=63781 d_live=59640 B_per_live_entry=882
```

≈ 0.86–0.91 KB reachable per live OR entry — read as ≈ 0.9 KB; the conductor's v4 figure of ≈ 0.87 KB
is the same quantity under a slightly different t = 60 estimate, and the two agree to the precision
the interpolation supports. The stored payload is a 34 B tag, an i64 and a timestamp, so ≈ 0.9 KB is
roughly an order of magnitude above the data.

Scaled to 8f's terminal census (`live=1047614`, `live_tag_bytes=35618878`) that is ≈ 0.9 GB of
reachable live set against an ≈ 9.4 GB span (§3.6) — roughly a tenth. Route: **TODO-593**, BEHIND the
allocator carve, not before it.

### 3.8 dhat (recorded; C3 never enters CLASS)

`LEVER=n/a` by the R6.6 table, because CLASS is not `REACHABLE` or `MIXED`. What the diff recorded:

```
PD-format=TRUE  PD-sym=TRUE symbolised=46/50  PD-crate=TRUE  PD=TRUE  C3-top1-lever=592
top-3 levers: 1:592, 2:593, 3:UNMAPPED
lever_share: 592=1.52MB(38%), 593=1.50MB(38%), UNMAPPED=0.92MB(23%), 591=0.03MB(1%)
all_std_pps=0 all_std_share=0%   C3-top1-all-std=FALSE   LEVER_CONTESTED=FALSE
total early: sum_eb=13633359 (13.00 MB) sum_gb=24057368 (22.94 MB) pps=4097
total late:  sum_eb=15689110 (14.96 MB) sum_gb=36003410 (34.34 MB) pps=4663
```

The top three `Δeb` signatures start at non-std frames, so the v3 R2 strip is doing its job:
`1 redb::…PagedCachedFile::write` (1.51 MB), `2 topgun_server::…crdt::apply_or_delta`
(`crdt.rs:1371`, 1.50 MB), `3 topgun_server::…CrdtService::apply_single_op` (`crdt.rs:530`, 0.38 MB).

Two things this does and does not say. 592 and 593 are within 1.3 % of each other on the underlying MB (1.52 vs 1.50; equal at the
printed share precision of 38 %), so
the top-1 token names a holder and not a winner; and the reachable heap moved only 13.0 → 15.0 MB
across the two profiles, under a 3× throughput throttle (57–74 ops/s against 180/s on the CA cells).
A reachable heap that small, moving that little, is consistent with the count-alloc cells' flat
`L` and with §3.7 — it is further evidence that the growth is not on the reachable side, and it
names holders only.

### 3.9 Console WARN inventory — every family, all six cells

There are **no ERROR lines, no panics, no `INSTRUMENT DEFECT` lines, no `AbandonedWrite` lines and no
stalled-watermark alarms anywhere in the chain**. The WARN lines are exhaustively these four families,
and the per-cell counts below sum to each console's total WARN count (1 / 1 / 1 / 1 / 211 / 234):

| family | r0 | c0 | c1 | c2 | c3e | c3l | window |
|---|---|---|---|---|---|---|---|
| `WAL fsync policy is Batched (default)` (boot) | 1 | 1 | 1 | 1 | 1 | 1 | server boot |
| `prune update failed, re-indexing tombstone for retry` | 0 | 0 | 0 | 0 | **185** | **232** | teardown, 11:20:01.24–11:20:02.56 (c3e) / 11:35:09.59–11:35:12.74 (c3l) |
| `the prune task exited; no tombstone reclamation runs …` | 0 | 0 | 0 | 0 | 1 | 1 | teardown, 11:20:06.074096Z (c3e) / 11:35:19.551883Z (c3l) |
| `Failed to mark WAL watermark applied` | 0 | 0 | 0 | 0 | **24** | 0 | teardown, 11:20:06.126248Z–11:20:06.643162Z (c3e) |

**No flag, fit, predicate or reading in this manifest depends on any of them.** All four families are
outside the last-half fit windows: the boot line precedes t = 0, and every teardown line lands after
the cell's last CSV row.

**1 — the documented boot WARN**, once per cell, in all six. It is the line CLAUDE.md specifies the
server must emit whenever the effective policy is the `batched` default with a durable backend, so its
presence is the configuration being correct, not an anomaly:

```
[server] 2026-09-20T10:14:50.610685Z  WARN topgun_server: WAL fsync policy is Batched (default): acked writes inside the ~10ms group-commit window are NOT durable under an unclean shutdown. Set TOPGUN_WAL_FSYNC_POLICY=per_op for acked-implies-durable.
```

**2 and 3 — the prune/write-behind teardown pair**, in the two graceful cells only (c3e and c3l are the
only cells that shut down with `TOPGUN_SOAK_GRACEFUL_SHUTDOWN=1`; the four SIGKILL cells have zero).
First line of each, verbatim (ANSI escapes stripped):

```
[server] 2026-09-20T11:20:01.244681Z  WARN topgun_server::service::domain::crdt: prune update failed, re-indexing tombstone for retry: write-behind store is shutting down; write rejected for map=soak_or key=ork-25 map=soak_or key=ork-25 epoch=5
[server] 2026-09-20T11:20:06.074096Z  WARN topgun_server::service::domain::crdt: the prune task exited; no tombstone reclamation runs for this frontier until a prune task is spawned again. Expected during runtime teardown, which is how a graceful shutdown ends this task
```

**4 — the WAL-watermark family**, c3e only, 24 lines inside 0.52 s of its SIGTERM drain: 20 read
`Cannot unlink sealed WAL segment`, 4 read `WAL fsync failed`. The first of each, verbatim:

```
[server] 2026-09-20T11:20:06.126248Z  WARN topgun_server::storage::datastores::write_behind: Failed to mark WAL watermark applied; next restart will re-replay (safe but redundant) partition=232 watermark=43807 error=WAL fsync failed for /Users/koristuvac/Projects/topgun/topgun/target/spec371-c3e-data/wal/partition-232-00000000000000042190.log: background task failed
[server] 2026-09-20T11:20:06.142858Z  WARN topgun_server::storage::datastores::write_behind: Failed to mark WAL watermark applied; next restart will re-replay (safe but redundant) partition=162 watermark=44069 error=Cannot unlink sealed WAL segment /Users/koristuvac/Projects/topgun/topgun/target/spec371-c3e-data/wal/partition-162-00000000000000041072.log: background task failed
```

**Classification (no fix here).** Families 2 and 3 are one symptom: **graceful-shutdown ordering — the
prune task keeps issuing updates after the write-behind store has begun rejecting writes**, which the
third line names in its own text (`Expected during runtime teardown`). They route to **TODO-694**, not
to TODO-689. Family 4 is the WAL-watermark surface **TODO-689** owns, and it is recorded there as an
observation: it is a DISTINCT symptom from the 238 `AbandonedWrite { origin: Live }` ERROR alarms that
TODO-689 was opened on — WARN not ERROR, at graceful-shutdown drain rather than in steady state, with
`background task failed` as the inner error, i.e. the WAL executor was already down when the watermark
write was attempted. Benign by its own text (`next restart will re-replay (safe but redundant)`).

### 3.10 Scope of every claim above

**All six cells read minutes 7–15 of a run.** `CLASS`, `CA_REGIME`, `RELEASE_RETENTION` and the §3.6
/ §3.7 readings are early-window claims, and §3.6's 8f series is the direct evidence that the
regime changes after ~2 h: the span stops growing and the footprint converts into it.

The **acceptance gate of the carve this decision routes to (TODO-590 + TODO-591) is a ≥ 4 h
plain-release cell on this matrix**, read on BOTH `phys_footprint` and `S`, with ops parity
recorded. It is not a 900 s re-measure, and it is not a count-alloc cell: this chain has just
demonstrated that the count-alloc regime does not reproduce the growth the fix has to remove.

### 3.11 Deferred, with ids — no fix claims

| id | what | when |
|---|---|---|
| **TODO-590 + TODO-591** | the routed next step: alternative global allocator, and the per-op record clone in write-behind / observer fan-out | **next**, per `NEXT=` |
| TODO-593 | resident per-key OrMap live-set growth — ≈ 0.9 KB reachable per live OR entry (§3.7), ≈ 0.9 GB at 8f scale | behind the allocator carve |
| TODO-592 | cap the redb read-path page cache — the top dhat grower (§3.8) at 1.52 MB, i.e. small | behind the allocator carve |
| TODO-693 | per-scrape live-OR-bytes gauge on `/metrics`, so a CSV row carries live bytes at every sample instead of only at census instants | after the memory FIX carve |
| TODO-694 | graceful-shutdown ordering: quiesce the prune task before the write-behind store rejects writes — §3.9 families 2 and 3, 185 / 232 WARNs per graceful teardown | with the next shutdown-path carve |
| TODO-689 | the WAL-watermark surface; §3.9's 24 c3e WARN lines (family 4) are recorded against it as a distinct symptom | after the memory carve |

**Residue without a TODO:** a dead `READOUT_OUT` assignment at `spec371-memdiag.sh:332`, left by R3
item 8 when the parent's `spec365-readout.sh` invocation was dropped. It moves no measurement, and M
is frozen under D, so it is NOT fixed here — re-cutting M beneath a committed D would break the
provenance chain this carve rests on. Drop the line when the next lineage copies this runner.

This carve measured and changed nothing: 1 `.rs` file (`process.rs`, K1 `e69cb0c6`, passing
`TOPGUN_JOURNAL_ENABLED` through instead of forcing it), evidence programs, and artifacts. No fix,
gate, monitor or invariant moved.
