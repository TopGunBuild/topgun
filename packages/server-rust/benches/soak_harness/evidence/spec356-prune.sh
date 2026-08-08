#!/usr/bin/env bash
#
# Prune-record measurement runner for the OR tombstone reclamation record.
#
# Derived from the committed spec355-width.sh. THE RATE-, SHAPE- AND
# DURATION-DETERMINING MATRIX IS IDENTICAL to that file's (which in turn is
# spec349c2-plateau.sh's), so this runner's cells and SPEC-355's committed
# width-1000 arms are the same matrix. The ONLY permitted differences are:
#
#   TOPGUN_EPOCH_WIDTH    the width axis            (per-cell literal below)
#   TOPGUN_PRUNE_RECORD   the ARMING axis           (per-cell literal below)
#   --duration            per-cell literal          (per-cell literal below)
#   --server-port,
#   --data-dir, outputs   run isolation             (derived from the cell id)
#   SOAK_SERVER_BINARY    PROVENANCE ARM ONLY       (fail-closed; section 0b)
#
# The difference against the parent is enumerable with:
#   diff spec355-width.sh spec356-prune.sh
#
# THE MATRIX IS EXECUTED, NOT TRANSCRIBED. Every knob is a literal in this
# file, so the record of what was run is this committed script rather than an
# operator's memory of a command line.
#
# WHAT THIS RUNNER ADDS OVER ITS PARENT
#
#   1. A SECOND sampler, at a 10s cadence, writing spec356-<cell>.prune.csv:
#      the raw cumulative counters and instantaneous gauges of the prune
#      record. Deltas are computed POST HOC, never in the sampler.
#   2. The SCRAPE GATE: no row is written from a failed or incomplete scrape.
#   3. The ARMING WITNESS: on an armed cell the prune-record series must be
#      present; on a disarmed cell they must be absent. Fail-closed, exit 9.
#   4. Both of the parent's column-check shapes, GENERALIZED over
#      (file, column index, name), with a per-column selection table and two
#      distinct failure dispositions.
#
# Bash 3.2 (macOS system bash) compatible: no mapfile, no associative arrays,
# no ${x@Q}.
#
# Env overrides (all documented, all logged loudly when active):
#   SPEC356_DATA_DIR              data dir for this run (default: target/)
#   SPEC356_OUT_DIR               artifact dir     (default: this script's dir)
#   SPEC356_SOAK_BIN              prebuilt soak_harness bench binary
#   SPEC356_SMOKE_DURATION        SMOKE ONLY: override --duration (seconds)
#   SPEC356_SMOKE_SAMPLE_INTERVAL SMOKE ONLY: override the primary CSV cadence
#   SPEC356_COLCHECK_FIXTURE      SMOKE ONLY: run the column-check path over a
#                                 fixture prune.csv and exit without starting a
#                                 harness (the guard-fire demonstration)
#   SPEC356_WITNESS_DEMO_ARMING   SMOKE ONLY: force the CHILD's arming to a
#                                 value that contradicts the cell's literal, so
#                                 the arming witness can be SEEN to fire in both
#                                 directions. Never usable on a measurement cell.
#   SPEC356_FORCE=1               overwrite pre-existing artifacts
#   SOAK_SERVER_BINARY            REQUIRED on a provenance cell, REFUSED
#                                 elsewhere (see section 0b)
#
set -euo pipefail

# ---------------------------------------------------------------------------
# 0. Argument: the CELL id. Every output path, the width, the arming, the
#    duration and the provenance discipline are derived from it. There is no
#    free-form knob: a cell that is not in this table cannot be run. Exactly
#    ONE positional argument is taken, and there is no --smoke flag: smoke is
#    entered through SPEC356_SMOKE_DURATION and nothing else.
# ---------------------------------------------------------------------------
CELL="${1:-}"

usage() {
  cat >&2 <<'EOF'
usage: spec356-prune.sh <cell>

  The measurement matrix. NO ROW OF THIS TABLE IS RUN BY SPEC-356a; every one
  of them is executed by SPEC-356b, against this committed runner unedited.

    ctl        armed,    width 1000,  1800s, n=2  -- series control vs SPEC-355
    ctloff     DISARMED, width 1000,  1800s, n=2  -- armed-vs-disarmed control
    w100       armed,    width  100,  1800s, n=1  -- the keeping-up reference
    long       armed,    width 1000, 14400s, n=1  -- the classification run
    cellE      PROVENANCE pre-346 server,
               width 1000, 1800s, n=1             -- CONDITIONAL gap probe

  The two NON-MEASUREMENT cells. They exist to demonstrate the schema and the
  fail-closed witnesses, they carry basenames no SPEC-356b artifact can ever
  have, and they REFUSE to run without SPEC356_SMOKE_DURATION:

    smoke      armed,    width 1000   -- schema + population demonstration
    smokeoff   DISARMED, width 1000   -- arming witness, absent direction

  A provenance cell (cellE) REQUIRES SOAK_SERVER_BINARY to be exported and to
  name an existing executable. It never falls back to the binary the bench was
  compiled beside.
EOF
  exit 2
}

if [ "$#" -gt 1 ]; then
  echo "FATAL: spec356-prune.sh takes exactly ONE positional argument, the cell id." >&2
  echo "       There is no --smoke flag and no second positional argument: smoke is" >&2
  echo "       entered ONLY through SPEC356_SMOKE_DURATION (R4.5, carried forward" >&2
  echo "       from spec355-width.sh's one-argument contract)." >&2
  exit 2
fi

# The per-cell literals. Columns:
#   width      -- "" means UNSET, i.e. the PRODUCTION default (1000)
#   arming     -- yes|no  => TOPGUN_PRUNE_RECORD true|false on the child
#   duration   -- seconds
#   cadence    -- PRIMARY CSV sample interval, seconds (the prune.csv cadence
#                 is pinned at 10s for every cell and is NOT a per-cell knob)
#   provenance -- yes|no
#   measurement-- yes|no  (no => a smoke cell: not in the R4.4 table)
#   base       -- artifact basename
case "$CELL" in
  ctl)       WIDTH="";   ARMED=yes; DURATION=1800;  SAMPLE_INTERVAL=60; PROVENANCE=no
             MEASUREMENT=yes; BASE="spec356-ctl" ;;
  ctloff)    WIDTH="";   ARMED=no;  DURATION=1800;  SAMPLE_INTERVAL=60; PROVENANCE=no
             MEASUREMENT=yes; BASE="spec356-ctloff" ;;
  w100)      WIDTH=100;  ARMED=yes; DURATION=1800;  SAMPLE_INTERVAL=60; PROVENANCE=no
             MEASUREMENT=yes; BASE="spec356-w100" ;;
  long)      WIDTH="";   ARMED=yes; DURATION=14400; SAMPLE_INTERVAL=60; PROVENANCE=no
             MEASUREMENT=yes; BASE="spec356-long" ;;
  cellE)     WIDTH="";   ARMED=no;  DURATION=1800;  SAMPLE_INTERVAL=60; PROVENANCE=yes
             MEASUREMENT=yes; BASE="spec356-cellE" ;;
  # The two NON-MEASUREMENT cells. Their basenames are deliberately outside
  # SPEC-356b's committed set (ctl / ctloff / w100 / long / cellE), so a stray
  # scratch file can never be mistaken for evidence.
  smoke)     WIDTH="";   ARMED=yes; DURATION=120;   SAMPLE_INTERVAL=10; PROVENANCE=no
             MEASUREMENT=no;  BASE="spec356-smoke" ;;
  smokeoff)  WIDTH="";   ARMED=no;  DURATION=120;   SAMPLE_INTERVAL=10; PROVENANCE=no
             MEASUREMENT=no;  BASE="spec356-smokeoff" ;;
  *)         usage ;;
esac

# The prune-record CSV cadence. Pinned at 10s for EVERY cell (manifest §5.2);
# it is not a per-cell literal and the smoke override does not move it, because
# the 10s figure is what makes the monotone-counter differencing fine-grained.
PRUNE_SAMPLE_INTERVAL=10

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
EVIDENCE_DIR="$SCRIPT_DIR"
SERVER_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"     # packages/server-rust
REPO_ROOT="$(cd "$SERVER_ROOT/../.." && pwd)"

OUT_DIR="${SPEC356_OUT_DIR:-$EVIDENCE_DIR}"
CSV_OUT="${OUT_DIR}/${BASE}.csv"
PRUNE_CSV="${OUT_DIR}/${BASE}.prune.csv"
JSON_OUT="${OUT_DIR}/${BASE}.soak.json"
PROGRESS_OUT="${OUT_DIR}/${BASE}.progress.jsonl"
MATRIX_OUT="${OUT_DIR}/${BASE}.matrix.txt"
# The harness derives the mechanism report's path from --json-output via Rust's
# `Path::with_extension`, which replaces the LAST extension only: it writes
# "<base>.soak.mechanism.json". The ledger for these runs names
# "<base>.mechanism.json", so the runner normalizes the name after the run
# rather than leaving the committed tree disagreeing with the ledger.
MECH_RAW="${OUT_DIR}/${BASE}.soak.mechanism.json"
MECH_OUT="${OUT_DIR}/${BASE}.mechanism.json"
CONSOLE_OUT="${OUT_DIR}/${BASE}.harness-console.log"

# ---------------------------------------------------------------------------
# 0a. THE PINNED METRIC NAMES and the per-column SELECTION TABLE.
#
#     The names are the frozen set of manifest §5.3 — 15 counters, 11 gauges,
#     7 histograms (each contributing BOTH its _sum and its _count column) —
#     plus the two inherited prune.csv columns: the sampler's own elapsed_secs
#     and the monotone topgun_ormap_tombstone_bytes_total that makes ADDED
#     bytes exact rather than a 60s-sampled lower bound.
#
#     Column order is manifest §5.2's: elapsed_secs, then §5.3 in its frozen
#     order, then the inherited counter.
#
#     43 COLUMNS FROM 35 NAMES, AND WHY THE TWO COUNTS ARE THE SAME TABLE.
#     SPEC-356a R4.3a.2 tags 35 rows — the 33 pinned §5.3 names plus the two
#     inherited columns — and this table has one row per COLUMN, so it has 43:
#     each of the 7 histogram names contributes its `_sum` and its `_count`
#     column, and both inherit their name's tag; the batch-size histogram
#     additionally contributes its rendered `_p50` column (ADJ-12, the
#     committed source of B). The tag totals follow the same arithmetic and
#     are checkable rather than asserted: R4.3a.2's 6 INSTRUMENT / 29 MEASURAND
#     over names becomes 6 INSTRUMENT / 37 MEASURAND over columns, because all
#     7 expanded names are MEASURAND and so is the p50 (29 + 7 + 1 = 37). A
#     reader who counts one table against the other and finds 35 vs 43 is
#     looking at names versus columns, not at a drift.
#
#     THE TAG COLUMN IS THE CLASSIFICATION RULE'S OUTPUT, NOT A PREFERENCE.
#     A column is INSTRUMENT only if its zero is impossible under a working
#     instrument REGARDLESS of which branch of the frozen classification
#     predicate is true. Every other column is a MEASURAND: its zero is a
#     legal reading of the world and routes through the predicate, never
#     through a discard-and-re-run loop. When in doubt the column is a
#     MEASURAND, because the permissive shape can never fail a correct
#     instrument while a genuinely dead column is still caught by n == 0 (and
#     by the empty > 0 delta).
#
#     Justification per row lives in SPEC-356a R4.3a.2; the one-line reason is
#     carried here so a reader of the runner can check the tag against its own
#     stated reason without leaving the table.
# ---------------------------------------------------------------------------
PRUNE_COLUMNS='
1|elapsed_secs|INSTRUMENT|sampler-local, never scrape-derived; all-zero means the sampler is not advancing
2|topgun_or_prune_passes_total|INSTRUMENT|counted on EVERY prune-loop invocation, empty drains included; all-zero means the ledger call site never executed
3|topgun_or_prune_considered_total|MEASURAND|zero considered is the licensing / stall reading; the exit share is then 0/0 and routes to Step 0(c)
4|topgun_or_prune_dropped_total|MEASURAND|zero drops is what a fully pinned corpus looks like
5|topgun_or_prune_matched_nothing_total|MEASURAND|may be negligible for a whole run; the record must be able to say so with a number
6|topgun_or_prune_absent_total|MEASURAND|the candidate mechanism this record exists to make readable; its zero is a finding, not a fault
7|topgun_or_prune_restored_read_error_total|MEASURAND|an error-free run legitimately reads 0
8|topgun_or_prune_restored_evicted_total|MEASURAND|an error-free run legitimately reads 0
9|topgun_or_prune_restored_write_error_total|MEASURAND|an error-free run legitimately reads 0
10|topgun_or_prune_bytes_freed_total|MEASURAND|freed bytes are the measurand of the defect itself; freeing nothing is the extreme, not a fault
11|topgun_or_prune_epochs_drained_total|MEASURAND|zero drained epochs is the licensing / stall reading
12|topgun_or_prune_empty_drains_total|MEASURAND|no outcome-independent impossibility proof exists for its zero; the conservative shape governs
13|topgun_or_prune_nonempty_drains_total|MEASURAND|zero non-empty drains IS the total-stall regime and leaves B undefined
14|topgun_or_prune_lwm_advances_total|MEASURAND|no LWM advance IS the scheduling-stall regime the predicate exists to name
15|topgun_or_prune_lwm_epochs_advanced_total|MEASURAND|same regime; zero exactly when lwm_advances_total is
16|topgun_or_prune_split_recomputes_total|MEASURAND|its zero is a pre-registered admissibility limb; a NONZERO limb would convert a predicate leaf into an instrument defect
17|topgun_or_prune_indexed_refs|INSTRUMENT|maintained O(1) on the OR WRITE path; added tombstone bytes at this width are an established input, so all-zero means the stamp observation is dead
18|topgun_or_prune_indexed_epochs|INSTRUMENT|same write-path derivation; refs without epochs is not a branch, it is a broken index
19|topgun_or_prune_eligible_refs|MEASURAND|L = 0 for a whole run is the licensing limb at its extreme, a pre-registered determination input
20|topgun_or_prune_ineligible_refs|MEASURAND|P = 0 is the keeping-up shape the w100 contrast cell exists to exhibit
21|topgun_or_prune_split_computed_epoch|MEASURAND|0 is the reserved no-epoch SENTINEL and means the split was never recomputed - a pre-registered stale-split route
22|topgun_or_prune_current_epoch|INSTRUMENT|the epoch clock advances on the write path independently of the prune; no branch is expressible in a run whose epoch counter never leaves 0
23|topgun_or_prune_low_water_mark|MEASURAND|0 is a legal value; an LWM pinned at 0 is the total-stall regime
24|topgun_or_prune_durable_epoch_watermark|MEASURAND|a watermark pinned at 0 is one of the two conjuncts the pinning question asks for
25|topgun_or_prune_last_drained_epoch|MEASURAND|the same reserved sentinel: it reads "no epoch has drained yet", a pre-registered stall signal
26|topgun_or_prune_lwm_stall_seconds|MEASURAND|zero stall is the HEALTHY reading; both extremes of this cadence measurand are legal
27|topgun_or_prune_tracked_claims|MEASURAND|zero tracked claims is a legal frontier state
28|topgun_or_prune_drain_refs_sum|MEASURAND|recorded only on non-empty drains; the stall regime legitimately has none
29|topgun_or_prune_drain_refs_count|MEASURAND|recorded only on non-empty drains; a zero count leaves B undefined and routes to Step 0(c)
30|topgun_or_prune_drain_epochs_sum|MEASURAND|same trigger, same regime
31|topgun_or_prune_drain_epochs_count|MEASURAND|same trigger, same regime
32|topgun_or_prune_claim_span_epochs_sum|MEASURAND|recorded at LWM movement and non-empty drain; both legitimately absent in the stall regime
33|topgun_or_prune_claim_span_epochs_count|MEASURAND|recorded at LWM movement and non-empty drain; both legitimately absent in the stall regime
34|topgun_or_prune_claim_lag_epochs_sum|MEASURAND|same instants, and additionally zero whenever no claim is tracked
35|topgun_or_prune_claim_lag_epochs_count|MEASURAND|same instants, and additionally zero whenever no claim is tracked
36|topgun_or_prune_epoch_considered_sum|MEASURAND|per DRAINED epoch: no drained epoch means no observation
37|topgun_or_prune_epoch_considered_count|MEASURAND|per DRAINED epoch: no drained epoch means no observation
38|topgun_or_prune_epoch_dropped_sum|MEASURAND|same trigger
39|topgun_or_prune_epoch_dropped_count|MEASURAND|same trigger
40|topgun_or_prune_epoch_bytes_freed_sum|MEASURAND|same trigger; also zero whenever drained epochs free nothing
41|topgun_or_prune_epoch_bytes_freed_count|MEASURAND|same trigger; also zero whenever drained epochs free nothing
42|topgun_ormap_tombstone_bytes_total|INSTRUMENT|monotone ADDED-bytes counter on the OR add path; added bytes are an established input and an all-zero reading makes the reclaim fraction uncomputable. It is boot-seeded before the listener accepts connections and the scrape gate forbids a row from an incomplete scrape, so empty > 0 is unreachable here on a correct instrument
43|topgun_or_prune_drain_refs_p50|MEASURAND|the exporter-rendered p50 of the batch-size summary over its 3x20s rolling window, sampled for ADJ-12 as the committed source of B. The summary records only NON-EMPTY drains, so a populated window renders p50 >= 1 and 0 is the RESERVED empty-window sentinel (verified against the live exposition: an expired or never-filled window renders 0, not NaN); the stall regime legitimately renders the sentinel for the whole run
'

# The scrape-derived names, in column order, space-separated: every column
# except the sampler-local elapsed_secs. 42 names. The one synthetic name,
# topgun_or_prune_drain_refs_p50, is resolved by BOTH scrape readers (the
# sampler and the arming witness) from the labelled exposition line
# topgun_or_prune_drain_refs{quantile="0.5"} -- the same synthesis in both
# places, so the witness cannot demand a name the sampler cannot see.
PRUNE_SCRAPE_NAMES="$(printf '%s\n' "$PRUNE_COLUMNS" \
  | awk -F'|' 'NF >= 3 && $1 != 1 { printf "%s ", $2 } END { print "" }')"
# The header the sampler writes and the post-run check re-reads.
PRUNE_HEADER="$(printf '%s\n' "$PRUNE_COLUMNS" \
  | awk -F'|' 'NF >= 3 { if (out == "") out = $2; else out = out "," $2 } END { print out }')"
PRUNE_COL_COUNT="$(printf '%s\n' "$PRUNE_COLUMNS" | awk -F'|' 'NF >= 3 { n++ } END { print n + 0 }')"
if [ "$PRUNE_COL_COUNT" != "43" ]; then
  echo "FATAL: the selection table must govern exactly 43 columns; it governs ${PRUNE_COL_COUNT}." >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# 0b. FAIL-CLOSED SERVER-BINARY RESOLUTION -- the provenance guard.
#
#     This is a REFUSAL, not a knob: it determines nothing about rate, shape or
#     duration. It exists because the Rust resolver fails OPEN. With
#     SOAK_SERVER_BINARY absent from the bench process's environment,
#     `resolve_server_binary()` silently returns the bench's compile-time
#     default server path with no existence check and no warning -- so a
#     variable exported in the wrong subshell, or a worktree whose `cargo
#     build` landed in the shared target/, turns the provenance cell into a
#     second armed HEAD cell while every console line still looks correct.
#
#     A cell that silently measures the wrong binary is the one failure that
#     looks most like success, so on the provenance path this runner refuses to
#     start unless the variable is set to an existing executable file. There is
#     deliberately NO fallback here.
# ---------------------------------------------------------------------------
if [ "$PROVENANCE" = "yes" ]; then
  if [ -z "${SOAK_SERVER_BINARY:-}" ]; then
    echo "FATAL: cell '${CELL}' is a PROVENANCE cell and SOAK_SERVER_BINARY is unset." >&2
    echo "       Refusing to run: the Rust resolver fails OPEN and would silently" >&2
    echo "       use the HEAD binary the bench was compiled beside, making this" >&2
    echo "       cell a second armed HEAD cell." >&2
    echo "       Export SOAK_SERVER_BINARY=<worktree>/target/release/topgun-server" >&2
    exit 3
  fi
  if [ ! -f "$SOAK_SERVER_BINARY" ] || [ ! -x "$SOAK_SERVER_BINARY" ]; then
    echo "FATAL: SOAK_SERVER_BINARY does not name an existing executable file:" >&2
    echo "       '${SOAK_SERVER_BINARY}'" >&2
    exit 3
  fi
  export SOAK_SERVER_BINARY
  PROV_BIN="$SOAK_SERVER_BINARY"
else
  # Unset => the child is the topgun-server the harness was compiled beside.
  # A stray export from a previous provenance run in the same shell would
  # otherwise silently make a HEAD cell a provenance cell.
  if [ -n "${SOAK_SERVER_BINARY:-}" ]; then
    echo "WARNING: SOAK_SERVER_BINARY was exported ('${SOAK_SERVER_BINARY}') but cell" >&2
    echo "         '${CELL}' is NOT a provenance cell. Unsetting it: only cellE may" >&2
    echo "         vary the server binary." >&2
  fi
  unset SOAK_SERVER_BINARY || true
  PROV_BIN=""
fi

DATA_DIR="${SPEC356_DATA_DIR:-${REPO_ROOT}/target/spec356-${CELL}-data}"
META_DIR="${DATA_DIR}.meta"      # sibling: NEVER inside the measured data dir
CONSOLE_LOG="${META_DIR}/harness-console.log"
STOP_FILE="${META_DIR}/sampler.stop"
FAIL_FILE="${META_DIR}/sampler.fail"
PRUNE_FAIL_FILE="${META_DIR}/prune-sampler.fail"
PRUNE_SKIP_LOG="${META_DIR}/prune-sampler.skipped-ticks.log"
# The first-complete-scrape marker is a FILE, not a shell variable: the prune
# sampler runs as a background job, so a variable set inside it would never be
# visible to the post-run check that reads it.
PRUNE_FIRST_FILE="${META_DIR}/prune-sampler.first-complete"
WITNESS_SCRAPE="${META_DIR}/arming-witness.scrape"

# ---------------------------------------------------------------------------
# 2. Smoke override -- only ever changes the duration (and, with it, the
#    PRIMARY CSV cadence, which would otherwise yield 3 rows). The real runs
#    must not be able to take this path by accident, so the defaults are the
#    per-cell literals above, the override is loud, and it is REFUSED if the
#    artifacts would land in the tracked evidence directory.
#
#    "Smoke" is DEFINED BY THIS OVERRIDE PATH, not by any duration threshold.
#
#    This replaces "the executor remembers to rm the artifacts" with a
#    mechanism that fails closed. There is NO manual-deletion step anywhere in
#    this spec: the guard, not an executor's memory, is what keeps
#    spec356-*.{csv,prune.csv,soak.json} out of the tracked tree.
# ---------------------------------------------------------------------------
SMOKE=0
if [ -n "${SPEC356_SMOKE_DURATION:-}" ]; then
  SMOKE=1
  DURATION="$SPEC356_SMOKE_DURATION"
  if [ -n "${SPEC356_SMOKE_SAMPLE_INTERVAL:-}" ]; then
    SAMPLE_INTERVAL="$SPEC356_SMOKE_SAMPLE_INTERVAL"
  fi
  echo "############################################################"
  echo "## SMOKE MODE -- THIS IS NOT A CHARACTERIZATION RUN       ##"
  echo "##   --duration          = ${DURATION}s (override)        "
  echo "##   primary CSV cadence = ${SAMPLE_INTERVAL}s            "
  echo "##   prune.csv cadence   = ${PRUNE_SAMPLE_INTERVAL}s (pinned)"
  echo "## Its artifacts MUST NOT be committed as evidence.       ##"
  echo "############################################################"
  if [ "$OUT_DIR" = "$EVIDENCE_DIR" ]; then
    echo "REFUSING: smoke mode would write into the tracked evidence dir" >&2
    echo "  $EVIDENCE_DIR" >&2
    echo "Set SPEC356_OUT_DIR to a scratch directory." >&2
    exit 2
  fi
elif [ -n "${SPEC356_SMOKE_SAMPLE_INTERVAL:-}" ]; then
  echo "WARNING: SPEC356_SMOKE_SAMPLE_INTERVAL ignored outside smoke mode;" >&2
  echo "         this cell's primary CSV cadence is pinned at ${SAMPLE_INTERVAL}s." >&2
fi

# The two NON-MEASUREMENT cells cannot be run as if they were measurement
# cells. They are not in the R4.4 table and they have no pinned duration of
# their own, so without the smoke override there is nothing to run.
if [ "$MEASUREMENT" = "no" ] && [ "$SMOKE" != "1" ]; then
  echo "REFUSING: cell '${CELL}' is a NON-MEASUREMENT cell and SPEC356_SMOKE_DURATION" >&2
  echo "          is unset. It is not in the measurement table and it must not be run" >&2
  echo "          as if it were: set SPEC356_SMOKE_DURATION (and SPEC356_OUT_DIR to a" >&2
  echo "          scratch directory) to run it as the schema/witness demonstration it is." >&2
  exit 2
fi

if [ "$OUT_DIR" != "$EVIDENCE_DIR" ]; then
  echo "WARNING: artifact dir overridden to $OUT_DIR"
  echo "         A characterization run's artifacts belong in $EVIDENCE_DIR"
  echo "         (git-tracked); artifacts written elsewhere are not evidence."
fi

# ---------------------------------------------------------------------------
# 2b. THE COLUMN-CHECK SHAPES, generalized over (file, column index, name).
#
#     Both shapes are the parent runner's, carried forward with their FAILING
#     LIMBS VERBATIM. What is NOT carried forward is the parameterization: the
#     parent's tombstone_col_report takes no parameters at all and hardcodes
#     v = $6, the literal name "tombstone_bytes" and the primary CSV's path,
#     while col_report hardcodes the primary CSV's path. Every prune-record
#     column lives in the SECOND file, so those three hardcodings die. An
#     executor who copies either shape verbatim onto column 6 of the wrong file
#     has MIS-carried it; the parameterization is required, not a deviation.
#
#     ONE named delta, and it is the only one: the POPULATION-ONLY shape gains
#     an `empty > 0` failing limb. The inherited fork PRINTS empty= but never
#     fails on it. prune.csv has no ps/du companion column that would trip on a
#     blank row, so without the added limb nothing enforces the zero-blank
#     condition and a silently-blank deciding column would read as coverage
#     while providing none. The limb is only safe -- and is only armed -- now
#     that both causes of a blank scrape-derived cell are unrepresentable:
#     eager registration of all 33 series at recorder construction removes the
#     "not yet registered" cause, and the scrape gate below removes the "the
#     scrape failed" cause.
# ---------------------------------------------------------------------------

# NONZERO shape. Failing limb carried VERBATIM from spec355-width.sh's
# col_report: `if (empty > 0 || n == 0 || nonzero == 0) exit 1`.
# Asserts: populated AND not all-zero.
col_report_nonzero() {   # $1 = file, $2 = 1-based column index, $3 = name
  awk -F, -v idx="$2" -v name="$3" '
    NR == 1 { next }
    {
      v = $idx
      gsub(/[ \t\r]/, "", v)
      if (v == "") empty++
      else {
        n++
        if (v + 0 != 0) nonzero++
        if (n == 1 || v + 0 < min) min = v + 0
        if (n == 1 || v + 0 > max) max = v + 0
      }
    }
    END {
      printf "  %-44s [NONZERO ] n=%d empty=%d nonzero=%d min=%.0f max=%.0f\n",
             name, n + 0, empty + 0, nonzero + 0, min + 0, max + 0
      if (empty > 0 || n == 0 || nonzero == 0) exit 1
    }
  ' "$1"
}

# POPULATION-ONLY shape. The fork is deliberate and the parent says why in its
# own words: "POPULATION check, not non-zero: a genuinely bounded tombstone
# corpus may legitimately read 0 for a whole run." Asserts: populated ONLY.
col_report_population() {   # $1 = file, $2 = 1-based column index, $3 = name
  awk -F, -v idx="$2" -v name="$3" '
    NR == 1 { next }
    {
      v = $idx
      gsub(/[ \t\r]/, "", v)
      if (v == "") empty++
      else {
        n++
        if (n == 1 || v + 0 < min) min = v + 0
        if (n == 1 || v + 0 > max) max = v + 0
      }
    }
    END {
      printf "  %-44s [POPULATN] n=%d empty=%d min=%.0f max=%.0f\n",
             name, n + 0, empty + 0, min + 0, max + 0
      # The inherited failing limb, verbatim.
      if (n == 0) exit 1
      # THE ONE NAMED DELTA: a blank cell is unreachable on a correct
      # instrument once eager registration and the scrape gate are in force,
      # so a blank here is a genuine sampler fault rather than a boot property.
      if (empty > 0) exit 1
    }
  ' "$1"
}

# ---------------------------------------------------------------------------
# 2c. THE TWO DISPOSITIONS, and the FORBIDDANCE that keeps them apart.
#
#   - An INSTRUMENT-tagged column that fails (either limb) is an INSTRUMENT
#     DEFECT: the run's series must not be recorded as evidence, and the cell
#     is discarded and re-run. That is licensed PRECISELY BECAUSE the column is
#     outcome-independent -- re-running on it cannot select on the
#     determination.
#
#   - A MEASURAND-tagged column that fails emits a DISTINCT line,
#     "STEP0C ADMISSIBILITY: <column> ...", which does NOT contain the string
#     INSTRUMENT DEFECT, does NOT set INSTRUMENT_OK and does NOT exit 9. Its
#     exit status is PINNED, not merely constrained: this path sets no exit
#     status of its own and alters none -- the runner falls through to the
#     inherited `exit "$HARNESS_RC"`, which on an otherwise-clean cell is 0.
#     The line is a RECORD, not a failure. The cell STANDS, and the failure is
#     carried into the classification walk as an admissibility failure that
#     routes the predicate to INDETERMINATE.
#
#   - fail_instrument, exit 9 and discard-and-re-run are FORBIDDEN for all 29
#     MEASURAND rows of the table above. A legitimately-zero measurand must
#     never be reclassified as an instrument defect, because discard-and-re-run
#     on a deterministic property of the measurand is SELECTION ON THE OUTCOME:
#     the re-run loop ends only when the data stops saying the inadmissible
#     thing.
#
#   The emitted literal is "STEP0C ADMISSIBILITY" -- same case, SINGLE space,
#   colon outside the pattern -- because a downstream mechanical grep depends
#   on those bytes and a mis-typed emitter returns 0, indistinguishable from a
#   clean run. Keeping them character-for-character identical is a constraint
#   on any future edit to this runner.
# ---------------------------------------------------------------------------
INSTRUMENT_OK=1
A9_OK=1
STEP0C_COUNT=0
fail_instrument() { echo "INSTRUMENT DEFECT: $1" >&2; INSTRUMENT_OK=0; }
step0c_admissibility() {   # $1 = column name, $2 = reason
  echo "STEP0C ADMISSIBILITY: $1 failed the population check ($2) -- the cell STANDS;"
  echo "                      this is an admissibility failure that routes the"
  echo "                      classification predicate to INDETERMINATE. It is NOT an"
  echo "                      instrument defect and it changes no exit status."
  STEP0C_COUNT=$((STEP0C_COUNT + 1))
}
a9_population_defect() {   # $1 = column name, $2 = reason
  echo "A9 POPULATION DEFECT: $1 failed the population check ($2) on a" >&2
  echo "                      NON-MEASUREMENT cell. There is no predicate here to route" >&2
  echo "                      to, so this is a BLOCKING defect repaired in CODE or in" >&2
  echo "                      the sampler before merge -- never by relaxing the check." >&2
  A9_OK=0
}

# The driver. SHAPE is selected by the table's tag AND the cell context;
# DISPOSITION is selected by the context alone.
#
#   shape:
#     - disarmed cell            => NO prune-record column is checked at all.
#       The arming witness requires the series to be ABSENT, and a column check
#       on an absent series is a category error.
#     - non-measurement cell     => POPULATION-ONLY on every column, tag
#       notwithstanding. A short scratch smoke cannot guarantee an LWM advance
#       or a non-empty drain, so the NONZERO limb is written into this runner
#       here but not exercised here.
#     - measurement cell (armed) => the table's tag governs: NONZERO on the 6
#       INSTRUMENT-tagged columns, POPULATION-ONLY on the 29 MEASURAND rows.
#
#   disposition:
#     - INSTRUMENT failure       => fail_instrument / exit 9.
#     - MEASURAND failure on a measurement cell (or on the fixture path, which
#       exists to demonstrate exactly that disposition) => the STEP0C line.
#     - MEASURAND failure on a real smoke run => the blocking A9 defect.
run_prune_column_checks() {   # $1 = file, $2 = disposition context (measurement|smoke)
  local file="$1" context="$2"
  local idx name tag reason shape

  if [ "$ARMED" != "yes" ]; then
    echo "  (disarmed cell: NO prune-record column is checked -- the arming witness"
    echo "   requires these series to be ABSENT, and a column check on an absent"
    echo "   series is a category error)"
    return 0
  fi

  while IFS='|' read -r idx name tag reason; do
    [ -z "${idx:-}" ] && continue
    [ -z "${name:-}" ] && continue
    shape=POPULATION
    if [ "$tag" = "INSTRUMENT" ] && [ "$MEASUREMENT" = "yes" ]; then
      shape=NONZERO
    fi
    if [ "$shape" = "NONZERO" ]; then
      col_report_nonzero "$file" "$idx" "$name" \
        || fail_instrument "prune column '${name}' (index ${idx}) is blank, unpopulated or all-zero"
    else
      col_report_population "$file" "$idx" "$name" \
        || {
          if [ "$context" = "measurement" ]; then
            step0c_admissibility "$name" "n == 0 or empty > 0 at column index ${idx}"
          else
            a9_population_defect "$name" "n == 0 or empty > 0 at column index ${idx}"
          fi
        }
    fi
  done <<EOF
$(printf '%s\n' "$PRUNE_COLUMNS")
EOF
}

# ---------------------------------------------------------------------------
# 2d. THE FIXTURE PATH -- the guard-fire demonstration.
#
#     Honoured ONLY in smoke mode, evaluated BEFORE the clock starts, and it
#     starts no harness. There is no new positional argument and no --flag: the
#     inherited contract is exactly one positional cell id and this runner does
#     not widen it.
#
#     Why a hand-built fixture is the ONLY way this guard can ever be seen to
#     fire, and why that is a CONSEQUENCE rather than a workaround: once eager
#     registration and the scrape gate make a blank cell unreachable on a
#     correct instrument, no produced artifact can trigger the branch, so the
#     trigger must be synthesized. Build the fixture by blanking ONE cell of
#     ONE MEASURAND column of a produced prune.csv -- an ALL-ZERO column does
#     NOT fire the line, because passing an all-zero column is the entire point
#     of the POPULATION-ONLY fork.
#
#     The shape run here is the smoke cell's (POPULATION-ONLY on every column);
#     the DISPOSITION is pinned to the measurement one, because demonstrating
#     that disposition is the whole purpose of the path.
# ---------------------------------------------------------------------------
if [ -n "${SPEC356_COLCHECK_FIXTURE:-}" ]; then
  if [ "$SMOKE" != "1" ]; then
    echo "FATAL: SPEC356_COLCHECK_FIXTURE is honoured in SMOKE MODE ONLY." >&2
    echo "       Set SPEC356_SMOKE_DURATION (and SPEC356_OUT_DIR to a scratch dir)." >&2
    exit 2
  fi
  if [ ! -s "$SPEC356_COLCHECK_FIXTURE" ]; then
    echo "FATAL: fixture is missing or empty: ${SPEC356_COLCHECK_FIXTURE}" >&2
    exit 2
  fi
  echo
  echo "=== COLUMN-CHECK FIXTURE PATH (no harness is started) ==="
  echo "  fixture:      ${SPEC356_COLCHECK_FIXTURE}"
  echo "  cell:         ${CELL} (armed=${ARMED})"
  echo "  shape:        POPULATION-ONLY on every column (non-measurement cell)"
  # The instrument-defect literal is deliberately NOT spelled on this path: a
  # downstream consumer greps the captured console for those exact bytes and
  # expects ZERO, so a banner that merely TALKS about the literal would be
  # indistinguishable from an emitter that fired it.
  echo "  disposition:  MEASURAND -- the STEP0C admissibility line, which is what"
  echo "                this path exists to demonstrate. A MEASURAND failure must NOT"
  echo "                carry the instrument-defect literal, must NOT write"
  echo "                INSTRUMENT_OK and must NOT exit 9."
  FIXTURE_HEADER="$(head -1 "$SPEC356_COLCHECK_FIXTURE" 2>/dev/null || true)"
  if [ "$FIXTURE_HEADER" != "$PRUNE_HEADER" ]; then
    echo "FATAL: fixture header does not match the pinned prune.csv schema." >&2
    exit 2
  fi
  echo "  fixture header: matches the pinned schema (${PRUNE_COL_COUNT} columns)"
  echo
  echo "prune.csv columns (fixture):"
  run_prune_column_checks "$SPEC356_COLCHECK_FIXTURE" measurement
  echo
  echo "  STEP0C ADMISSIBILITY lines emitted: ${STEP0C_COUNT}"
  echo "  INSTRUMENT_OK:                      ${INSTRUMENT_OK} (unchanged by a MEASURAND failure)"
  # The exit status on this path is the R4.3a.3 fall-through: the STEP0C path
  # sets no exit status of its own and alters none. No harness ran, so the
  # inherited harness return code is 0 and that is what is returned -- never 1
  # and never 9.
  HARNESS_RC=0
  echo "  exit status:                        ${HARNESS_RC} (the pinned fall-through, exit \"\$HARNESS_RC\")"
  exit "$HARNESS_RC"
fi

# ---------------------------------------------------------------------------
# 3. Environment discipline.
#
#    The harness spawns the real topgun-server as a child and sets a fixed
#    block of env on it, WITHOUT env_clear() -- so the child also inherits this
#    shell's environment. Anything below that were already exported in an
#    operator's shell would silently change what these runs measure, so they
#    are ACTIVELY UNSET rather than merely not set.
#
#    TWO deliberate departures from the parent runner, and they are the two
#    axes of this design: TOPGUN_EPOCH_WIDTH (the width axis) and
#    TOPGUN_PRUNE_RECORD (the arming axis) are SET from this cell's literals
#    rather than unconditionally unset. Every other line here is the parent's,
#    verbatim.
# ---------------------------------------------------------------------------
if [ -n "$WIDTH" ]; then
  export TOPGUN_EPOCH_WIDTH="$WIDTH"
else
  # Unset => the server applies the PRODUCTION default epoch width (1000).
  unset TOPGUN_EPOCH_WIDTH || true
fi

# The arming axis. It is set EXPLICITLY on both arms rather than left to the
# shipped default on the armed arm, so the armed-vs-disarmed control varies one
# named variable in one direction and the console line reports which.
if [ "$ARMED" = "yes" ]; then
  CHILD_ARMING=true
else
  CHILD_ARMING=false
fi
# SMOKE-ONLY defect injection, so the arming witness can be SEEN to fire. It
# changes the CHILD's arming while leaving this cell's EXPECTED arming at its
# literal, which is exactly the contradiction the witness exists to catch. It
# is refused outside smoke mode: on a measurement cell it would silently
# produce a cell whose console line disagrees with what ran.
WITNESS_DEMO=""
if [ -n "${SPEC356_WITNESS_DEMO_ARMING:-}" ]; then
  if [ "$SMOKE" != "1" ]; then
    echo "FATAL: SPEC356_WITNESS_DEMO_ARMING is a SMOKE-ONLY defect injection." >&2
    echo "       It deliberately contradicts the cell's arming literal so the arming" >&2
    echo "       witness can be seen to fire; on a measurement cell that would be a" >&2
    echo "       silently mislabelled cell." >&2
    exit 2
  fi
  WITNESS_DEMO="$SPEC356_WITNESS_DEMO_ARMING"
  CHILD_ARMING="$SPEC356_WITNESS_DEMO_ARMING"
  echo "############################################################"
  echo "## ARMING-WITNESS DEMONSTRATION -- DELIBERATE DEFECT      ##"
  echo "##   cell '${CELL}' expects armed=${ARMED}                 "
  echo "##   the CHILD is forced to TOPGUN_PRUNE_RECORD=${CHILD_ARMING}"
  echo "##   The arming witness MUST fail this run with exit 9.   ##"
  echo "############################################################"
fi
export TOPGUN_PRUNE_RECORD="$CHILD_ARMING"

# Unset => the harness supplies its own 100ms / 5000 write-behind cadence, which
# is the instrument-identity choice shared with every other soak run.
unset TOPGUN_WRITEBEHIND_FLUSH_INTERVAL_MS || true
unset TOPGUN_WRITEBEHIND_BATCH_SIZE || true
# Unset => the child runs at RUST_LOG=warn, the pinned instrument-identity log
# level (server-side info logging is a measurable cost on this write path).
unset SOAK_SERVER_LOG || true
# Unset => teardown is SIGKILL, as every other soak run's is.
unset TOPGUN_SOAK_GRACEFUL_SHUTDOWN || true
# Unset => production memory ceiling and eviction water marks.
unset TOPGUN_MAX_RAM_MB || true
unset TOPGUN_EVICTION_HIGH_PCT || true
unset TOPGUN_EVICTION_LOW_PCT || true
unset TOPGUN_EVICTION_INTERVAL_MS || true
# Unset => the OR delta emitter is ARMED, the shipped default and the state the
# reference lineage was measured in. This spec varies WIDTH and ARMING, never
# the WAL emitter.
unset TOPGUN_OR_DELTA_WAL || true

# NOTE: TOPGUN_WAL_FSYNC_POLICY is deliberately NOT managed here. The harness
# OVERWRITES it on the child unconditionally from --wal-fsync, so exporting it
# does nothing at all; the policy comes from the flag below and is recorded in
# soak.json as `walFsync`.
if [ -n "${TOPGUN_WAL_FSYNC_POLICY:-}" ]; then
  echo "note: TOPGUN_WAL_FSYNC_POLICY='${TOPGUN_WAL_FSYNC_POLICY}' is inherited but IGNORED"
  echo "      (the harness overwrites it on the child from --wal-fsync)"
fi

# ---------------------------------------------------------------------------
# 1. The pinned matrix. These are literals on purpose, and they are the
#    spec355-width.sh literals verbatim (which are the spec349c2-plateau.sh
#    literals verbatim): every rate- and shape-determining knob below is
#    byte-identical to the parent runners', so this spec's cells and the
#    committed width-1000 arms of both parents are the same matrix.
# ---------------------------------------------------------------------------
CHURN_CLIENTS=6
KEYSPACE=200            # LWW keyspace
OR_CHURN=true
OR_KEYSPACE=48          # 48 % 6 == 0 satisfies the harness's single-writer assert
OR_EVERY=5
WRITE_INTERVAL_MS=20
WRITES_PER_LIFE=200
OFFLINE_KEYS=3
CONFIRM_INTERVAL=2      # keeps the low-water-mark advancing so the prune fires
CRASH_INTERVAL=0        # 0 parses to None: NO kill -9 during the run
STEADY_INTERVAL=300     # ~1% of the run spent quiesced instead of ~10%
QUIESCE=3
MEM_SAMPLE_INTERVAL=5
WAL_FSYNC=batched
# The harness's live memory gate is NEUTRALIZED, not sharpened: the verdict for
# these runs is a post-hoc fit over the CSV, and at the shipped defaults the
# live gate fires on every run in this regime.
MEM_MIN_GROWTH_MB=1000000
MEM_THRESHOLD_MB_PER_HOUR=1000000
MEM_CEILING_MB=1000000
SERVER_PORT=47356       # fixed, so the child server has a reliable handle

# ---------------------------------------------------------------------------
# 4. Pre-flight. Fail closed BEFORE the clock starts.
# ---------------------------------------------------------------------------
TARGET_DIR="${CARGO_TARGET_DIR:-}"
if [ -z "$TARGET_DIR" ]; then
  if [ -d "${REPO_ROOT}/target/release" ]; then
    TARGET_DIR="${REPO_ROOT}/target"
  else
    TARGET_DIR="${SERVER_ROOT}/target"
  fi
fi

if [ "$PROVENANCE" = "yes" ]; then
  SERVER_BIN="$PROV_BIN"
else
  SERVER_BIN="${TARGET_DIR}/release/topgun-server"
  if [ ! -x "$SERVER_BIN" ]; then
    echo "FATAL: release server binary not found at $SERVER_BIN" >&2
    echo "  build: (cd $SERVER_ROOT && SDKROOT=\$(xcrun --sdk macosx --show-sdk-path) \\" >&2
    echo "          cargo build --release --bin topgun-server --bench soak_harness)" >&2
    exit 1
  fi
fi

SOAK_BIN="${SPEC356_SOAK_BIN:-}"
if [ -z "$SOAK_BIN" ]; then
  SOAK_BIN="$(ls -t "${TARGET_DIR}"/release/deps/soak_harness-* 2>/dev/null \
              | grep -vE '\.(d|o|rcgu)' | head -1 || true)"
fi
if [ -z "$SOAK_BIN" ] || [ ! -x "$SOAK_BIN" ]; then
  echo "FATAL: could not locate a built soak_harness bench binary under" >&2
  echo "       ${TARGET_DIR}/release/deps/" >&2
  exit 1
fi

SOAK_MTIME="$(date -r "$SOAK_BIN" '+%s')"
SERVER_MTIME="$(date -r "$SERVER_BIN" '+%s')"
echo "soak binary:   $SOAK_BIN"
echo "  built:       $(date -r "$SOAK_BIN" -u '+%Y-%m-%dT%H:%M:%SZ')"
echo "server binary: $SERVER_BIN"
echo "  built:       $(date -r "$SERVER_BIN" -u '+%Y-%m-%dT%H:%M:%SZ')"
BUILD_GAP=$((SOAK_MTIME - SERVER_MTIME))
if [ "$BUILD_GAP" -lt 0 ]; then
  BUILD_GAP=$((-BUILD_GAP))
fi
if [ "$PROVENANCE" = "yes" ]; then
  # A large gap is the POINT here: this cell is a deliberate half-swap, HEAD
  # instrument against an older server.
  echo "  NOTE: this is a PROVENANCE cell -- a deliberate half-swap. The"
  echo "        instrument (bench binary, scrape, CSV columns, fit, assessment)"
  echo "        is at HEAD; only the SERVER is the pinned older build. The"
  echo "        ${BUILD_GAP}s build gap is expected, not a defect."
elif [ "$BUILD_GAP" -gt 600 ]; then
  echo "  WARNING: the two binaries were linked ${BUILD_GAP}s apart, so they are"
  echo "           probably NOT from one 'cargo build --release --bin topgun-server"
  echo "           --bench soak_harness'. Both runs must use one binary."
fi

# The census is only sound over a fresh corpus.
if [ -e "$DATA_DIR" ]; then
  if [ ! -d "$DATA_DIR" ]; then
    echo "FATAL: data dir path exists and is not a directory: $DATA_DIR" >&2
    exit 1
  fi
  if [ -n "$(ls -A "$DATA_DIR" 2>/dev/null || true)" ]; then
    echo "FATAL: data dir is NOT empty: $DATA_DIR" >&2
    echo "       Each run needs its own fresh, empty dir -- retained frames from a" >&2
    echo "       previous run would answer this run's frame-kind census." >&2
    exit 1
  fi
fi
mkdir -p "$DATA_DIR" "$META_DIR" "$OUT_DIR"
rm -f "$STOP_FILE" "$FAIL_FILE" "$PRUNE_FAIL_FILE" "$PRUNE_SKIP_LOG" \
      "$PRUNE_FIRST_FILE" "$WITNESS_SCRAPE"

# Refuse to silently overwrite artifacts: a re-run that clobbers a recorded
# series destroys the only copy of a measurement.
for f in "$CSV_OUT" "$PRUNE_CSV" "$JSON_OUT" "$PROGRESS_OUT" "$MECH_OUT" "$MECH_RAW" "$MATRIX_OUT" "$CONSOLE_OUT"; do
  if [ -e "$f" ] && [ "${SPEC356_FORCE:-0}" != "1" ]; then
    echo "FATAL: artifact already exists: $f" >&2
    echo "       Move it aside, or re-run with SPEC356_FORCE=1 to overwrite." >&2
    exit 1
  fi
done
for f in "$CSV_OUT" "$PRUNE_CSV" "$JSON_OUT" "$PROGRESS_OUT" "$MATRIX_OUT" "$CONSOLE_OUT"; do
  d="$(dirname "$f")"
  if [ ! -w "$d" ]; then
    echo "FATAL: artifact directory is not writable: $d" >&2
    exit 1
  fi
  rm -f "$f"
  if ! : > "$f" 2>/dev/null; then
    echo "FATAL: cannot write artifact: $f" >&2
    exit 1
  fi
  rm -f "$f"
done

# ---------------------------------------------------------------------------
# 5. PID resolution. The thing being sampled is the CHILD topgun-server the
#    harness spawns, NOT the harness itself.
# ---------------------------------------------------------------------------
resolve_server_pid() {   # prints the pid, or nothing; never fails the shell
  local pids
  pids="$(lsof -nP -iTCP:"$SERVER_PORT" -sTCP:LISTEN -t 2>/dev/null | sort -u || true)"
  if [ -z "$pids" ]; then
    pids="$(pgrep -f "topgun-server --port ${SERVER_PORT}" 2>/dev/null | sort -u || true)"
  fi
  local count
  count="$(printf '%s\n' "$pids" | grep -c '[0-9]' || true)"
  if [ "$count" = "1" ]; then
    printf '%s' "$(printf '%s\n' "$pids" | grep '[0-9]' | head -1)"
  fi
}

if [ -n "$(resolve_server_pid)" ]; then
  echo "FATAL: something is already listening on port ${SERVER_PORT}" >&2
  echo "       The sampler resolves the server by that port; a stranger there" >&2
  echo "       would be sampled instead of this run's server." >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# 6. Report the effective matrix before starting. `tee`d into a COMMITTED
#    artifact, not merely printed.
# ---------------------------------------------------------------------------
{
  echo
  echo "=== spec356 prune-record run: cell ${CELL} ==="
  echo "  repo HEAD:      $(git -C "$REPO_ROOT" rev-parse HEAD 2>/dev/null || echo '<not a git checkout>')"
  echo "  dirty tree:     $(test -n "$(git -C "$REPO_ROOT" status --porcelain 2>/dev/null || true)" && echo yes || echo no)"
  echo "  host/OS:        $(uname -a)"
  echo "  lineage:        $( [ "$PROVENANCE" = "yes" ] && echo 'PROVENANCE (HEAD harness + pinned older server)' || echo 'HEAD harness + HEAD server' )"
  echo "  cell class:     $( [ "$MEASUREMENT" = "yes" ] && echo 'MEASUREMENT' || echo 'NON-MEASUREMENT (schema/witness demonstration only)' )"
  echo "  data dir:       $DATA_DIR"
  echo "  csv:            $CSV_OUT"
  echo "  prune.csv:      $PRUNE_CSV"
  echo "  soak.json:      $JSON_OUT"
  echo "  mechanism.json: $MECH_OUT (harness writes $(basename "$MECH_RAW"); renamed after the run)"
  echo "  progress.jsonl: $PROGRESS_OUT"
  echo "  console log:    $CONSOLE_OUT"
  echo "  matrix:         $MATRIX_OUT"
  echo "  soak binary:    $SOAK_BIN"
  echo "    built:        $(date -r "$SOAK_BIN" -u '+%Y-%m-%dT%H:%M:%SZ')"
  echo "  server binary:  $SERVER_BIN"
  echo "    built:        $(date -r "$SERVER_BIN" -u '+%Y-%m-%dT%H:%M:%SZ')"
  echo "  --- varied knobs (everything else is the spec355-width.sh literal) ---"
  echo "  TOPGUN_EPOCH_WIDTH:  ${TOPGUN_EPOCH_WIDTH:-<unset: production default 1000>}"
  echo "  TOPGUN_PRUNE_RECORD: ${TOPGUN_PRUNE_RECORD} (cell expects armed=${ARMED})"
  if [ -n "$WITNESS_DEMO" ]; then
    echo "                       <-- SMOKE-ONLY WITNESS DEMONSTRATION: deliberately"
    echo "                           contradicts the cell literal; the arming witness"
    echo "                           MUST fail this run with exit 9."
  fi
  echo "  duration:            ${DURATION}s$( [ "$SMOKE" = "1" ] && echo '  <-- SMOKE OVERRIDE')"
  echo "  primary csv cadence: ${SAMPLE_INTERVAL}s"
  echo "  prune.csv cadence:   ${PRUNE_SAMPLE_INTERVAL}s (pinned for every cell)"
  echo "  server port:         ${SERVER_PORT}"
  echo "  SOAK_SERVER_BINARY:  ${SOAK_SERVER_BINARY:-<unset: HEAD binary, fail-closed guard not on this path>}"
  if [ "$PROVENANCE" = "yes" ]; then
    echo "  --- provenance arm: identity checks ---"
    echo "  fail-closed resolution: PASSED (SOAK_SERVER_BINARY set and executable;"
    echo "                          this path has NO fallback to the bench's compile-time"
    echo "                          default server binary -- an unset variable is a hard"
    echo "                          refusal before the clock starts, exit 3)"
    echo "  census identity witness: pending -- read orDeltaFrames from $(basename "$MECH_OUT")"
    echo "                          after the run. It MUST be 0 (with orSnapshotFrames > 0)."
    echo "                          A nonzero value means the swap was BOTCHED: the cell is"
    echo "                          INVALID, is NOT a decision-table row, and is re-run."
    echo "  pinned SHA:            ${SPEC356_PIN_SHA:-<record it here: export SPEC356_PIN_SHA>}"
    echo "  pin resolved by:       ${SPEC356_PIN_CMD:-<record it here: export SPEC356_PIN_CMD>}"
    echo "  worktree path:         ${SPEC356_PIN_WORKTREE:-<record it here: export SPEC356_PIN_WORKTREE>}"
  fi
  echo "  --- pinned matrix (identical to spec355-width.sh) ---"
  echo "  churn-clients ${CHURN_CLIENTS}; keyspace ${KEYSPACE}; or-churn ${OR_CHURN};"
  echo "  or-keyspace ${OR_KEYSPACE}; or-every ${OR_EVERY}; write-interval-ms ${WRITE_INTERVAL_MS};"
  echo "  writes-per-life ${WRITES_PER_LIFE}; offline-keys ${OFFLINE_KEYS};"
  echo "  confirm-interval ${CONFIRM_INTERVAL}; crash-interval ${CRASH_INTERVAL};"
  echo "  steady-interval ${STEADY_INTERVAL}; quiesce ${QUIESCE};"
  echo "  mem-sample-interval ${MEM_SAMPLE_INTERVAL}; wal-fsync ${WAL_FSYNC};"
  echo "  memory gate NEUTRALIZED (${MEM_MIN_GROWTH_MB}/${MEM_THRESHOLD_MB_PER_HOUR}/${MEM_CEILING_MB})"
  echo
} | tee "$MATRIX_OUT"

# ---------------------------------------------------------------------------
# 7. Launch.
# ---------------------------------------------------------------------------
HARNESS_PID=""
SAMPLER_PID=""
PRUNE_SAMPLER_PID=""
cleanup() {
  for p in "$SAMPLER_PID" "$PRUNE_SAMPLER_PID" "$HARNESS_PID"; do
    if [ -n "$p" ] && kill -0 "$p" 2>/dev/null; then
      kill "$p" 2>/dev/null || true
    fi
  done
}
trap cleanup EXIT INT TERM

"$SOAK_BIN" \
  --duration "$DURATION" \
  --churn-clients "$CHURN_CLIENTS" \
  --keyspace "$KEYSPACE" \
  --or-churn "$OR_CHURN" \
  --or-keyspace "$OR_KEYSPACE" \
  --or-every "$OR_EVERY" \
  --write-interval-ms "$WRITE_INTERVAL_MS" \
  --writes-per-life "$WRITES_PER_LIFE" \
  --offline-keys "$OFFLINE_KEYS" \
  --confirm-interval "$CONFIRM_INTERVAL" \
  --crash-interval "$CRASH_INTERVAL" \
  --steady-interval "$STEADY_INTERVAL" \
  --quiesce "$QUIESCE" \
  --mem-sample-interval "$MEM_SAMPLE_INTERVAL" \
  --mem-min-growth-mb "$MEM_MIN_GROWTH_MB" \
  --mem-threshold-mb-per-hour "$MEM_THRESHOLD_MB_PER_HOUR" \
  --mem-ceiling-mb "$MEM_CEILING_MB" \
  --wal-fsync "$WAL_FSYNC" \
  --server-port "$SERVER_PORT" \
  --data-dir "$DATA_DIR" \
  --json-output "$JSON_OUT" \
  --progress-output "$PROGRESS_OUT" \
  --mechanism-report \
  > "$CONSOLE_LOG" 2>&1 &
HARNESS_PID=$!
echo "harness pid $HARNESS_PID; follow with: tail -f $CONSOLE_LOG"

# ---------------------------------------------------------------------------
# 8. Wait for server-ready. This is BOTH samplers' time origin.
# ---------------------------------------------------------------------------
READY_TIMEOUT=180
SERVER_PID=""
waited=0
while [ "$waited" -lt "$READY_TIMEOUT" ]; do
  SERVER_PID="$(resolve_server_pid)"
  [ -n "$SERVER_PID" ] && break
  if ! kill -0 "$HARNESS_PID" 2>/dev/null; then
    echo "FATAL: harness exited before the server became ready" >&2
    tail -40 "$CONSOLE_LOG" >&2 || true
    cp -f "$CONSOLE_LOG" "$CONSOLE_OUT" 2>/dev/null || true
    exit 1
  fi
  sleep 1
  waited=$((waited + 1))
done
if [ -z "$SERVER_PID" ]; then
  echo "FATAL: no single listener on port ${SERVER_PORT} after ${READY_TIMEOUT}s" >&2
  if [ "$PROVENANCE" = "yes" ]; then
    echo "       This is a PROVENANCE cell. A pinned server that cannot reach" >&2
    echo "       readiness is a RECORDED FINDING that forces a later pin -- it is" >&2
    echo "       never a reason to degrade the run to the HEAD binary." >&2
  fi
  tail -40 "$CONSOLE_LOG" >&2 || true
  cp -f "$CONSOLE_LOG" "$CONSOLE_OUT" 2>/dev/null || true
  exit 1
fi
T0="$(date +%s)"
echo "server ready: pid $SERVER_PID (t0 = server-ready)"

# ---------------------------------------------------------------------------
# 9. The PRIMARY per-minute sampler. Its CSV header and its cadence are
#    byte-identical to spec355-width.sh's, so the windowed fit SPEC-356b
#    computes is produced by the same instrument over the same shape as
#    SPEC-355's. Its inherited tolerance of a blank column 6 at elapsed_secs=0
#    is NOT touched: the primary CSV is untouched by the scrape gate below.
# ---------------------------------------------------------------------------
kib_to_mb() { awk -v k="${1:-0}" 'BEGIN { printf "%.3f", k / 1024 }'; }

TOMBSTONE_METRIC='topgun_ormap_tombstone_bytes'
tombstone_bytes() {
  local body
  body="$(curl -fsS --max-time 5 "http://127.0.0.1:${SERVER_PORT}/metrics" 2>/dev/null)" || {
    printf 'ABSENT'
    return 0
  }
  printf '%s' "$body" | awk -v m="$TOMBSTONE_METRIC" '
    /^[[:space:]]*#/ { next }
    {
      name = $1
      if (name == m || index(name, m "{") == 1) {
        printf "%d", int($2 + 0)
        found = 1
        exit
      }
    }
    END { if (!found) printf "ABSENT" }
  '
}

du_kib() {  # $1 = path
  if [ ! -e "$1" ]; then
    printf 'ABSENT'
    return 0
  fi
  local v
  v="$(du -sk "$1" 2>/dev/null | awk 'NR == 1 { print $1 }')"
  if [ -z "$v" ]; then
    sleep 1
    v="$(du -sk "$1" 2>/dev/null | awk 'NR == 1 { print $1 }')"
  fi
  printf '%s' "$v"
}

sampler_fatal() {   # $1 = reason
  echo "$1" > "$FAIL_FILE"
  echo "SAMPLER FATAL: $1" >&2
  kill "$HARNESS_PID" 2>/dev/null || true
}

run_is_over() {   # $1 = elapsed seconds since server-ready
  if ! kill -0 "$HARNESS_PID" 2>/dev/null; then
    return 0
  fi
  if [ "$1" -ge $((DURATION - 10)) ]; then
    return 0
  fi
  return 1
}

emit_row() {
  local now elapsed pid rss_kib tries
  now="$(date +%s)"
  elapsed=$((now - T0))

  pid=""
  tries=0
  while [ "$tries" -lt 3 ]; do
    pid="$(resolve_server_pid)"
    [ -n "$pid" ] && break
    if run_is_over "$elapsed"; then
      touch "$STOP_FILE"
      return 0
    fi
    tries=$((tries + 1))
    sleep 2
  done
  if [ -z "$pid" ]; then
    sampler_fatal "server PID on port ${SERVER_PORT} did not resolve to exactly one process at elapsed=${elapsed}s"
    return 1
  fi
  SERVER_PID="$pid"

  rss_kib="$(ps -o rss= -p "$pid" 2>/dev/null | tr -d '[:space:]')"
  if [ -z "$rss_kib" ]; then
    if run_is_over "$elapsed"; then
      touch "$STOP_FILE"
      return 0
    fi
    sampler_fatal "ps returned no RSS for pid ${pid} at elapsed=${elapsed}s (blind column)"
    return 1
  fi

  local total_kib wal_kib redb_kib
  total_kib="$(du_kib "$DATA_DIR")"
  case "$total_kib" in
    ''|ABSENT)
      sampler_fatal "du gave no size for data dir ${DATA_DIR} at elapsed=${elapsed}s"
      return 1
      ;;
  esac
  wal_kib="$(du_kib "${DATA_DIR}/wal")"
  if [ "$wal_kib" = "ABSENT" ]; then
    wal_kib=0
  elif [ -z "$wal_kib" ]; then
    sampler_fatal "du failed on ${DATA_DIR}/wal at elapsed=${elapsed}s"
    return 1
  fi
  redb_kib="$(du_kib "${DATA_DIR}/topgun.redb")"
  if [ "$redb_kib" = "ABSENT" ]; then
    redb_kib=0
  elif [ -z "$redb_kib" ]; then
    sampler_fatal "du failed on ${DATA_DIR}/topgun.redb at elapsed=${elapsed}s"
    return 1
  fi

  local v
  for v in "$rss_kib" "$wal_kib" "$redb_kib" "$total_kib"; do
    case "$v" in
      ''|*[!0-9]*)
        sampler_fatal "non-integer sample '${v}' at elapsed=${elapsed}s"
        return 1
        ;;
    esac
  done

  local tomb
  tomb="$(tombstone_bytes)"
  case "$tomb" in
    ''|ABSENT|*[!0-9]*) tomb="" ;;
  esac

  printf '%d,%s,%s,%s,%s,%s\n' \
    "$elapsed" \
    "$(kib_to_mb "$rss_kib")" \
    "$(kib_to_mb "$wal_kib")" \
    "$(kib_to_mb "$redb_kib")" \
    "$(kib_to_mb "$total_kib")" \
    "$tomb" \
    >> "$CSV_OUT"
}

echo 'elapsed_secs,rss_mb,wal_mb,redb_mb,disk_total_mb,tombstone_bytes' > "$CSV_OUT"

sampler_loop() {
  local next="$T0"
  while [ ! -f "$STOP_FILE" ]; do
    local now
    now="$(date +%s)"
    if [ "$now" -lt "$next" ]; then
      sleep 1
      continue
    fi
    emit_row || return 1
    next=$((next + SAMPLE_INTERVAL))
    now="$(date +%s)"
    if [ "$next" -le "$now" ]; then
      next=$((now + SAMPLE_INTERVAL))
    fi
  done
}

sampler_loop &
SAMPLER_PID=$!

# ---------------------------------------------------------------------------
# 9b. The SECOND sampler: the prune record, at a 10s cadence.
#
#     It writes RAW CUMULATIVE COUNTERS and INSTANTANEOUS GAUGES. Deltas are
#     computed POST HOC, never here -- a sampler that differences in-flight
#     cannot be re-read against a different window later.
#
#     THE SCRAPE GATE (the rule that makes a blank cell unreachable):
#
#       On an ARMED cell, a row is written ONLY from a scrape that (a) returned
#       successfully and (b) carries ALL 42 scrape-derived series. A tick whose
#       scrape fails either test writes NO ROW AT ALL, logs the tick, and
#       retries on the next tick -- so the first row of prune.csv is the first
#       COMPLETE scrape, and its elapsed_secs is whatever the sampler's clock
#       reads at that instant (it is NOT required to be 0). Failing to obtain a
#       complete scrape inside the readiness window is an INSTRUMENT DEFECT.
#
#       On a DISARMED cell the gate is NOT in force. The sampler runs
#       IDENTICALLY -- same 10s cadence, same scrape, same row-writing cadence,
#       so its cost is COMMON-MODE and the armed-vs-disarmed control still
#       varies exactly one thing -- but it writes its row with the prune-record
#       columns EMPTY, and no column check reads them. Series absence on a
#       disarmed cell is NEVER an instrument defect: it is precisely what the
#       arming witness requires, checked once and fail-closed.
#
#     Why the gate is not merely belt-and-braces on eager registration: eager
#     registration makes the SERIES exist; the gate makes the SCRAPE complete.
#     A curl that cannot connect returns nothing for every column alike, which
#     eager registration cannot help with, and the gate cannot help if a name is
#     never registered. Together they make "an empty field in prune.csv"
#     unreachable on a correct instrument -- which is what the empty > 0 limb
#     presupposes.
# ---------------------------------------------------------------------------
# The readiness window for the FIRST complete scrape, inherited from the
# server-readiness timeout and capped by the run's own duration so a short
# non-measurement run still reaches its verdict.
PRUNE_READY_TIMEOUT=180
if [ "$DURATION" -lt "$PRUNE_READY_TIMEOUT" ]; then
  PRUNE_READY_TIMEOUT="$DURATION"
fi

scrape_prune_series() {   # prints "COMPLETE|<missing>|<v1>,<v2>,..." or "SCRAPE_FAIL||"
  local body
  body="$(curl -fsS --max-time 5 "http://127.0.0.1:${SERVER_PORT}/metrics" 2>/dev/null)" || {
    printf 'SCRAPE_FAIL||'
    return 0
  }
  printf '%s' "$body" | awk -v names="$PRUNE_SCRAPE_NAMES" '
    BEGIN { want = split(names, a, " ") }
    /^[[:space:]]*#/ { next }
    {
      nm = $1
      # The one synthetic name: the labelled p50 line of the batch summary is
      # captured under <base>_p50 BEFORE the label-stripping fold, mirroring
      # the arming witness exactly.
      if (nm ~ /\{quantile="0\.5"\}$/) {
        q = nm
        sub(/\{.*$/, "", q)
        q = q "_p50"
        if (!(q in val)) val[q] = $2
      }
      sub(/\{.*$/, "", nm)
      if (!(nm in val)) val[nm] = $2
    }
    END {
      out = ""; missing = ""; nmiss = 0
      for (i = 1; i <= want; i++) {
        k = a[i]
        if (k in val) {
          v = val[k]
          gsub(/[ \t\r]/, "", v)
        } else {
          v = ""
          nmiss++
          if (missing == "") missing = k
        }
        out = out "," v
      }
      status = (nmiss == 0) ? "COMPLETE" : "INCOMPLETE"
      printf "%s|%d:%s|%s", status, nmiss, missing, substr(out, 2)
    }
  '
}

prune_emit_row() {
  local now elapsed raw status missing values
  now="$(date +%s)"
  elapsed=$((now - T0))

  raw="$(scrape_prune_series)"
  status="${raw%%|*}"
  raw="${raw#*|}"
  missing="${raw%%|*}"
  values="${raw#*|}"

  if [ "$status" = "SCRAPE_FAIL" ]; then
    echo "elapsed=${elapsed}s scrape FAILED -- no row written, retrying next tick" >> "$PRUNE_SKIP_LOG"
    return 0
  fi

  if [ "$ARMED" = "yes" ]; then
    if [ "$status" != "COMPLETE" ]; then
      echo "elapsed=${elapsed}s scrape INCOMPLETE (${missing}) -- no row written, retrying next tick" \
        >> "$PRUNE_SKIP_LOG"
      return 0
    fi
    if [ ! -s "$PRUNE_FIRST_FILE" ]; then
      printf '%s' "$elapsed" > "$PRUNE_FIRST_FILE"
    fi
  fi
  # On a disarmed cell every prune-record field of `values` is already empty,
  # which is exactly what the adjudicated rule requires: the row is written,
  # the columns are blank, and no column check reads them.

  printf '%d,%s\n' "$elapsed" "$values" >> "$PRUNE_CSV"
}

printf '%s\n' "$PRUNE_HEADER" > "$PRUNE_CSV"

prune_sampler_loop() {
  local next="$T0"
  while [ ! -f "$STOP_FILE" ]; do
    local now
    now="$(date +%s)"
    if [ "$now" -lt "$next" ]; then
      sleep 1
      continue
    fi
    prune_emit_row
    if [ "$ARMED" = "yes" ] && [ ! -s "$PRUNE_FIRST_FILE" ]; then
      if [ $(( $(date +%s) - T0 )) -ge "$PRUNE_READY_TIMEOUT" ]; then
        echo "no COMPLETE scrape (all ${PRUNE_COL_COUNT} columns) within the ${PRUNE_READY_TIMEOUT}s readiness window" \
          > "$PRUNE_FAIL_FILE"
        echo "PRUNE SAMPLER FATAL: no complete scrape within ${PRUNE_READY_TIMEOUT}s" >&2
        kill "$HARNESS_PID" 2>/dev/null || true
        return 1
      fi
    fi
    next=$((next + PRUNE_SAMPLE_INTERVAL))
    now="$(date +%s)"
    if [ "$next" -le "$now" ]; then
      next=$((now + PRUNE_SAMPLE_INTERVAL))
    fi
  done
}

prune_sampler_loop &
PRUNE_SAMPLER_PID=$!

# The ARMING WITNESS's own scrape, taken once, early, and kept. It is separate
# from the sampler's so that the witness has an answer even on a cell whose
# sampler wrote no row -- which is the disarmed cell's expected shape and would
# otherwise make the witness vacuous.
sleep 2
curl -fsS --max-time 5 "http://127.0.0.1:${SERVER_PORT}/metrics" 2>/dev/null \
  > "$WITNESS_SCRAPE" || true

# ---------------------------------------------------------------------------
# 10. Wait out the run.
# ---------------------------------------------------------------------------
set +e
wait "$HARNESS_PID"
HARNESS_RC=$?
set -e
HARNESS_PID=""
touch "$STOP_FILE"
set +e
wait "$SAMPLER_PID" 2>/dev/null
wait "$PRUNE_SAMPLER_PID" 2>/dev/null
set -e
SAMPLER_PID=""
PRUNE_SAMPLER_PID=""

echo
echo "harness exited with code ${HARNESS_RC}"
tail -25 "$CONSOLE_LOG" || true

# ---------------------------------------------------------------------------
# 11. Post-run: land the console log in its COMMITTED home, normalize the
#     mechanism report's name, then validate the series.
# ---------------------------------------------------------------------------
cp -f "$CONSOLE_LOG" "$CONSOLE_OUT"
echo "console log: $CONSOLE_OUT"

if [ -f "$MECH_RAW" ]; then
  mv -f "$MECH_RAW" "$MECH_OUT"
  echo "mechanism report: $MECH_OUT"
fi

if [ -s "$FAIL_FILE" ]; then
  fail_instrument "sampler aborted: $(cat "$FAIL_FILE")"
fi
if [ -s "$PRUNE_FAIL_FILE" ]; then
  fail_instrument "prune sampler aborted: $(cat "$PRUNE_FAIL_FILE")"
fi

# --- the PRIMARY CSV checks, inherited UNCHANGED -----------------------------
HEADER="$(head -1 "$CSV_OUT" 2>/dev/null || true)"
if [ "$HEADER" != 'elapsed_secs,rss_mb,wal_mb,redb_mb,disk_total_mb,tombstone_bytes' ]; then
  fail_instrument "CSV header is '$HEADER'"
fi
ROWS="$(( $(wc -l < "$CSV_OUT") - 1 ))"
echo "csv rows: $ROWS"
if [ "$ROWS" -lt 2 ]; then
  fail_instrument "CSV has $ROWS data rows"
fi

echo "csv columns:"
col_report_nonzero "$CSV_OUT" 2 rss_mb        || fail_instrument "rss_mb column is empty or all-zero (blind sampler)"
col_report_nonzero "$CSV_OUT" 3 wal_mb        || fail_instrument "wal_mb column is empty or all-zero (wrong WAL path?)"
col_report_nonzero "$CSV_OUT" 4 redb_mb       || fail_instrument "redb_mb column is empty or all-zero (wrong redb path?)"
col_report_nonzero "$CSV_OUT" 5 disk_total_mb || fail_instrument "disk_total_mb column is empty or all-zero"
# POPULATION check, not non-zero: a genuinely bounded tombstone corpus may
# legitimately read 0 for a whole run. The primary CSV's inherited tolerance of
# a blank at elapsed_secs = 0 is preserved by NOT applying the empty > 0 delta
# here -- that delta belongs to prune.csv, where the scrape gate makes it safe.
tombstone_col_report() {
  awk -F, '
    NR == 1 { next }
    {
      v = $6
      gsub(/[ \t\r]/, "", v)
      if (v == "") empty++
      else {
        n++
        if (n == 1 || v + 0 < min) min = v + 0
        if (n == 1 || v + 0 > max) max = v + 0
      }
    }
    END {
      printf "  %-14s n=%d empty=%d min=%.0f max=%.0f\n",
             "tombstone_bytes", n + 0, empty + 0, min + 0, max + 0
      if (n == 0) exit 1
    }
  ' "$CSV_OUT"
}
tombstone_col_report || fail_instrument "tombstone_bytes column has no readings (blind gauge scrape)"

# --- the PRUNE-RECORD CSV ----------------------------------------------------
echo
echo "prune.csv:      $PRUNE_CSV"
PRUNE_HEADER_READ="$(head -1 "$PRUNE_CSV" 2>/dev/null || true)"
if [ "$PRUNE_HEADER_READ" != "$PRUNE_HEADER" ]; then
  fail_instrument "prune.csv header does not match the pinned ${PRUNE_COL_COUNT}-column schema"
fi
PRUNE_ROWS="$(( $(wc -l < "$PRUNE_CSV") - 1 ))"
echo "prune.csv rows: $PRUNE_ROWS  (cadence ${PRUNE_SAMPLE_INTERVAL}s)"
if [ -s "$PRUNE_SKIP_LOG" ]; then
  echo "prune.csv ticks SKIPPED by the scrape gate: $(wc -l < "$PRUNE_SKIP_LOG" | tr -d ' ')"
  sed 's/^/    /' "$PRUNE_SKIP_LOG"
fi
if [ "$ARMED" = "yes" ]; then
  echo "first COMPLETE scrape at elapsed = $(cat "$PRUNE_FIRST_FILE" 2>/dev/null || echo '<none>')s"
  if [ "$PRUNE_ROWS" -lt 1 ]; then
    fail_instrument "prune.csv has no data rows: no COMPLETE scrape was ever obtained on an ARMED cell"
  fi
else
  if [ "$PRUNE_ROWS" -lt 1 ]; then
    fail_instrument "prune.csv has no data rows at all (the sampler must run identically on a disarmed cell)"
  fi
fi

echo "prune.csv columns:"
if [ "$MEASUREMENT" = "yes" ]; then
  PRUNE_DISPOSITION=measurement
else
  PRUNE_DISPOSITION=smoke
fi
run_prune_column_checks "$PRUNE_CSV" "$PRUNE_DISPOSITION"
if [ "$STEP0C_COUNT" -gt 0 ]; then
  echo
  echo "STEP0C ADMISSIBILITY lines emitted: ${STEP0C_COUNT}. The cell STANDS; these are"
  echo "  admissibility inputs to the classification walk, not instrument defects, and"
  echo "  they alter no exit status."
fi

for f in "$JSON_OUT" "$MECH_OUT"; do
  if [ ! -s "$f" ]; then
    fail_instrument "missing or empty artifact: $f"
  fi
done

if [ "$DURATION" -gt "$STEADY_INTERVAL" ]; then
  if [ ! -s "$PROGRESS_OUT" ]; then
    fail_instrument "missing or empty artifact: $PROGRESS_OUT"
  else
    echo "  progress.jsonl checkpoints: $(wc -l < "$PROGRESS_OUT" | tr -d ' ')"
  fi
else
  echo "  progress.jsonl: not required -- duration ${DURATION}s <= steady interval ${STEADY_INTERVAL}s,"
  echo "                  so the harness reached no checkpoint to snapshot."
fi

if [ -s "$JSON_OUT" ]; then
  for key in walFsync epochWidth crashes churnClients keyspace durationSecsActual; do
    line="$(grep -m1 "\"${key}\"" "$JSON_OUT" || true)"
    if [ -z "$line" ]; then
      fail_instrument "soak.json has no '${key}' key"
    else
      echo "  soak.json $(printf '%s' "$line" | sed 's/^[[:space:]]*//')"
    fi
  done
fi
if [ -s "$MECH_OUT" ]; then
  for key in orDeltaFrames orSnapshotFrames; do
    line="$(grep -m1 "\"${key}\"" "$MECH_OUT" || true)"
    if [ -z "$line" ]; then
      fail_instrument "mechanism.json has no '${key}' key"
    else
      echo "  mechanism.json $(printf '%s' "$line" | sed 's/^[[:space:]]*//')"
    fi
  done
fi

# ---------------------------------------------------------------------------
# 11a. THE ARMING WITNESS -- fail-closed, in both directions.
#
#      On an armed cell the prune-record series MUST be present in the scrape;
#      on a disarmed cell they MUST be absent. Either violation means the run
#      did not measure the arm its console line claims, which is the failure
#      that looks most like success, so it is an INSTRUMENT DEFECT and the run
#      is discarded and re-run. The witness is outcome-independent -- whether
#      the series EXIST is not a reading of the measurand -- so a re-run on it
#      cannot select on the determination.
# ---------------------------------------------------------------------------
echo
echo "arming witness:"
if [ ! -s "$WITNESS_SCRAPE" ]; then
  fail_instrument "arming witness has no scrape to read (the /metrics endpoint never answered)"
else
  PRUNE_SERIES_SEEN="$(grep -c '^topgun_or_prune_' "$WITNESS_SCRAPE" || true)"
  PRUNE_NAMES_SEEN="$(awk -v names="$PRUNE_SCRAPE_NAMES" '
    BEGIN { want = split(names, a, " ") }
    /^[[:space:]]*#/ { next }
    {
      nm = $1
      # Same p50 synthesis as the sampler: the witness must never demand a
      # name the sampler cannot resolve from the identical exposition.
      if (nm ~ /\{quantile="0\.5"\}$/) {
        q = nm; sub(/\{.*$/, "", q); seen[q "_p50"] = 1
      }
      sub(/\{.*$/, "", nm); seen[nm] = 1
    }
    END {
      c = 0
      for (i = 1; i <= want; i++) {
        if (a[i] ~ /^topgun_or_prune_/ && (a[i] in seen)) c++
      }
      print c
    }
  ' "$WITNESS_SCRAPE")"
  PRUNE_NAMES_WANTED="$(awk -v names="$PRUNE_SCRAPE_NAMES" '
    BEGIN {
      want = split(names, a, " "); c = 0
      for (i = 1; i <= want; i++) if (a[i] ~ /^topgun_or_prune_/) c++
      print c
    }' </dev/null)"
  echo "  cell expects:        armed=${ARMED}"
  echo "  TOPGUN_PRUNE_RECORD: ${TOPGUN_PRUNE_RECORD} (on the child)"
  echo "  topgun_or_prune_ lines in the scrape: ${PRUNE_SERIES_SEEN}"
  echo "  pinned prune columns resolved:        ${PRUNE_NAMES_SEEN}/${PRUNE_NAMES_WANTED}"
  echo "  topgun_ormap_tombstone_bytes present: $(grep -c '^topgun_ormap_tombstone_bytes' "$WITNESS_SCRAPE" || true) line(s)"
  if [ "$ARMED" = "yes" ]; then
    if [ "$PRUNE_SERIES_SEEN" -eq 0 ]; then
      fail_instrument "arming witness: cell '${CELL}' is ARMED but the scrape carries NO topgun_or_prune_ series -- the process that ran was DISARMED"
    elif [ "$PRUNE_NAMES_SEEN" != "$PRUNE_NAMES_WANTED" ]; then
      fail_instrument "arming witness: cell '${CELL}' is ARMED but only ${PRUNE_NAMES_SEEN}/${PRUNE_NAMES_WANTED} pinned prune columns resolved in the scrape"
    else
      echo "  => arming witness PASSED: armed cell, all pinned series present."
    fi
  else
    if [ "$PRUNE_SERIES_SEEN" -ne 0 ]; then
      fail_instrument "arming witness: cell '${CELL}' is DISARMED but the scrape carries ${PRUNE_SERIES_SEEN} topgun_or_prune_ line(s) -- the process that ran was ARMED"
    else
      echo "  => arming witness PASSED: disarmed cell, no prune-record series present."
    fi
  fi
fi

# ---------------------------------------------------------------------------
# 11b. PROVENANCE cells only: the census IDENTITY WITNESS.
#
#      The pinned SHA precedes the OR-delta emitter, so a pre-346 server cannot
#      write a single OR delta frame. A nonzero orDeltaFrames therefore proves
#      the swap did NOT happen and this cell is really a second HEAD cell. That
#      outcome is INVALID -- it is not a decision-table row, it is not
#      "reproduces", and it must never be reasoned about in the manifest. The
#      run is re-done.
# ---------------------------------------------------------------------------
if [ "$PROVENANCE" = "yes" ] && [ -s "$MECH_OUT" ]; then
  ODF="$(awk -F'[:,]' '/"orDeltaFrames"/ { gsub(/[^0-9-]/, "", $2); print $2; exit }' "$MECH_OUT")"
  OSF="$(awk -F'[:,]' '/"orSnapshotFrames"/ { gsub(/[^0-9-]/, "", $2); print $2; exit }' "$MECH_OUT")"
  echo
  echo "provenance identity witness (WAL frame-kind census):"
  echo "  orDeltaFrames    = ${ODF:-<unreadable>}   (MUST be 0)"
  echo "  orSnapshotFrames = ${OSF:-<unreadable>}   (MUST be > 0)"
  if [ "${ODF:-x}" != "0" ]; then
    echo "CELL INVALID: orDeltaFrames = ${ODF:-<unreadable>} != 0." >&2
    echo "  The pinned pre-346 server CANNOT emit OR delta frames, so this run" >&2
    echo "  was driven by the HEAD binary: the swap was BOTCHED." >&2
    echo "  Do NOT route this cell into a decision-table row. Fix the swap and re-run." >&2
    INSTRUMENT_OK=0
  elif [ -z "${OSF:-}" ] || [ "$OSF" -le 0 ] 2>/dev/null; then
    echo "CELL INVALID: orSnapshotFrames = ${OSF:-<unreadable>} is not > 0, so the OR" >&2
    echo "  path was not exercised and orDeltaFrames == 0 proves nothing." >&2
    INSTRUMENT_OK=0
  else
    echo "  => identity witness PASSED: this cell ran the pre-346 server."
  fi
fi

# ---------------------------------------------------------------------------
# 12. The POST-HOC FIT INVOCATION.
#
#     The slice-and-fit mechanism is REUSED, NOT REDESIGNED: spec349c2-fit.awk
#     is used UNFORKED, and this runner neither forks it nor writes a new one.
#
#     THIS HALF EXECUTES NO FIT. SPEC-356a runs only the two non-measurement
#     cells, no spec356-*.csv exists in it at all, so there is nothing to fit
#     and producing a slope here would be a measurement claim under a spec that
#     forbids reporting one. The invocation below is WRITTEN here and is
#     EXECUTED for the first time by SPEC-356b, against the `long` CSV that
#     spec creates.
#
#     The `long` cell's 8-window fit uses the committed one-liner (below) to
#     partition the CSV into 8 consecutive equal header-bearing segments, each
#     fitted at -v col=tombstone_bytes -v window=full.
# ---------------------------------------------------------------------------
FIT="${SCRIPT_DIR}/spec349c2-fit.awk"
print_fit_invocation() {
  echo "post-hoc fit invocation (spec349c2-fit.awk, UNFORKED):"
  echo "  # whole-window and last-half fits over the primary CSV"
  echo "  for w in full last_half; do"
  echo "    for c in rss_mb wal_mb redb_mb disk_total_mb tombstone_bytes; do"
  echo "      awk -v col=\"\$c\" -v window=\"\$w\" -f ${FIT} ${CSV_OUT}"
  echo "    done"
  echo "  done"
  echo "  # the 8-window slice-and-fit, for the 'long' cell only"
  echo "  awk -F, 'NR==1{h=\$0; next} {rows[++n]=\$0}"
  echo "    END{seg=int((n+7)/8);"
  echo "        for(i=1;i<=8;i++){f=sprintf(\"${BASE}-seg%d.csv\",i); print h > f;"
  echo "          for(j=(i-1)*seg+1; j<=i*seg && j<=n; j++) print rows[j] > f; close(f)}}' ${CSV_OUT}"
  echo "  for i in 1 2 3 4 5 6 7 8; do"
  echo "    awk -v col=tombstone_bytes -v window=full -f ${FIT} ${BASE}-seg\${i}.csv"
  echo "  done"
}

echo
if [ "$MEASUREMENT" != "yes" ]; then
  echo "NO FIT IS EXECUTED on a non-measurement cell. The invocation is written here"
  echo "and is executed for the first time by SPEC-356b, against the cells it runs:"
  print_fit_invocation | sed 's/^/  /'
elif [ -f "$FIT" ] && [ "$ROWS" -ge 4 ]; then
  echo "post-hoc OLS fits (see the SE caveat in $(basename "$FIT")):"
  echo "  units: the field is named slope_mb_per_hour for every column; on the"
  echo "  tombstone_bytes row the fit is identical but the unit is BYTES/hour."
  for w in full last_half; do
    for c in rss_mb wal_mb redb_mb disk_total_mb tombstone_bytes; do
      awk -v col="$c" -v window="$w" -f "$FIT" "$CSV_OUT" 2>&1 | sed 's/^/  /' || true
    done
  done
  echo
  echo "the 8-window slice-and-fit for the 'long' cell is run post-hoc:"
  print_fit_invocation | sed 's/^/  /'
fi

echo
if [ "$INSTRUMENT_OK" != "1" ]; then
  echo "RESULT: INSTRUMENT DEFECT -- this run's series must not be recorded as evidence." >&2
  exit 9
fi
if [ "$A9_OK" != "1" ]; then
  echo "RESULT: population defect on a NON-MEASUREMENT cell -- a deciding column carries" >&2
  echo "        no readings, or carries a blank. This is a BLOCKING defect repaired in" >&2
  echo "        code or in the sampler; relaxing a column check is not an available" >&2
  echo "        repair. It is deliberately NOT reported as an instrument defect and NOT" >&2
  echo "        exit 9: there is no measurement cell here to discard." >&2
  exit 1
fi
echo "RESULT: instrument sound; harness exit code ${HARNESS_RC}."
if [ "$HARNESS_RC" != "0" ]; then
  echo "NOTE: a non-zero harness exit is NOT automatically a failed characterization."
  echo "      The memory gate is neutralized by design for these runs, while the"
  echo "      tombstone-byte gate, both blind-monitor clauses, convergence/recovery"
  echo "      and panic capture stay armed. Read 'finishedReason' in $JSON_OUT and"
  echo "      record the attribution, rather than reading the flag as a verdict."
fi
exit "$HARNESS_RC"
