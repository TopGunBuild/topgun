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
