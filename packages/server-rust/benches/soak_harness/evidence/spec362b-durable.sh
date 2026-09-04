#!/usr/bin/env bash
#
# Durable-layer reading runner for the tombstone-corpus instrument.
#
# Derived from the committed spec355-width.sh. THE RATE-, SHAPE- AND
# DURATION-DETERMINING MATRIX IS IDENTICAL to that file's, and the ONLY
# permitted differences are the ones the spec's AC16 enumerates:
#
#   SOAK_SERVER_LOG      THE ONE DECLARED DEPARTURE (set, not unset; see below)
#   TOPGUN_LOG_FORMAT    added to the discipline block (see below)
#   --durable-reading    the instrument this runner exists to drive
#   --live-census-interval per-cell literal: 0, i.e. DISARMED, on the one
#                        cell this runner ships, which is the sampler set
#                        spec355-width.sh itself ran
#   --sampler-jitter-seed                           (JITTER_SEED, echoed into
#                                                    matrix.txt from that same
#                                                    shell variable)
#   --duration           per-cell literal           (per-cell literal below)
#   --crash-interval     per-cell literal: 0 on the one cell this runner
#                        ships, so there is no kill -9 during the run
#   --data-dir, outputs  run isolation              (derived from the cell id)
#
# THE ONE DECLARED DEPARTURE. spec355-width.sh actively unsets SOAK_SERVER_LOG
# so the child runs at RUST_LOG=warn. The origin read requires the removal
# target's line to be EMITTED, so this runner SETS the target-scoped directive
# instead, on the deciding cell. It is declared, not absorbed, and its price
# was measured by the pricing pair spec363-durable.sh ran; that pair is not
# re-run from here.
#
# TOPGUN_LOG_FORMAT is unset alongside the parent's seven. This is the first
# runner in the lineage that PARSES a rendered server log line; the fmt layer
# switches to .json() when TOPGUN_LOG_FORMAT=json, and the harness never calls
# env_clear(), so the child inherits this shell. An exported `json` would make
# every origin line unparsed and would change the cost of the one declared
# departure silently. The unset removes the shell-dependence at its source.
#
# The difference against the grandparent is enumerable with:
#   diff spec355-width.sh spec362b-durable.sh
#
# DERIVATIVE OF spec363-durable.sh, WHICH IS NOT EDITED. The lineage is
# spec355-width.sh -> spec362-durable.sh -> spec363-durable.sh -> this file,
# and every parent stays byte-unchanged. This runner exists to execute the ONE
# deciding cell that spec363-durable.sh deliberately refuses to carry, on the
# instrument spec363-durable.sh pinned. THE DIFFERENCE LIST AGAINST THAT
# PARENT IS CLOSED: EXACTLY six enumerated differences, and no others.
#
#   (a) identity: artifact prefix spec362b-* and, with it, the SPEC362B_* env
#       overrides -- the COMPLETE renamed variable surface, the three
#       provenance-branch variables included, dead on this cell though they
#       are -- the run's data dir, and the matrix banner. The deciding
#       artifacts stay trivially separable from both control lineages in
#       evidence/;
#   (b) THE 4h DECIDING CELL IS RESTORED, carrying the frozen row's literals
#       from spec362-manifest.md section 5 and one knob that row predates:
#       width unset (production default 1000), duration 14400, cadence 60,
#       provenance no, crash-interval 0, log directive ARMED, no extra flags
#       -- plus live-census 0, which is in substance that same section's
#       "--live-census-interval is DISARMED on the deciding cell". A longer
#       cell is REFUSED, not deferred;
#   (c) THE PIN IS A LITERAL AND IT IS ENFORCED, not merely echoed. The parent
#       carried a branch-point literal that reached matrix.txt and gated
#       nothing. Here one variable, SPEC362B_PIN, is used three times and
#       re-typed nowhere: a fail-closed pre-flight that refuses to start when
#       the .rs tree at HEAD differs from it, the matrix.txt echo of the pin,
#       and the matrix.txt echo of that pre-flight's verdict. The gate stands
#       BEFORE the inherited dirty-tree guard and BEFORE the build, so
#       "asserted before the build" is true by construction and a refusal
#       costs no compile;
#   (d) THE THREE <= 900s INSTRUMENT CONTROLS ARE DELETED rather than merely
#       un-invoked; an explicit refusal comment stands in their place in the
#       cell table. They belong to spec362-durable.sh and spec363-durable.sh,
#       they have been witnessed over two cycles, and re-running them from
#       here would create a fourth control set and a new witness cycle that
#       nothing authorizes. Asking for one falls through to `usage`, exit 2;
#   (e) this header block, and the `usage` prose, that describe the derivation
#       and the closed list AS A WHOLE rather than any single member of it;
#   (f) one inherited uppercase spec marker in the pinned-matrix comment
#       block, rewritten to the lowercase form that same block already uses.
#       Prose only: no knob, no literal value, no control flow.
#
# NO RATE- OR SHAPE-DETERMINING LITERAL CHANGES. The jitter seed, the server
# port, every matrix literal, the environment-discipline block, the
# build-from-HEAD step and its dirty-.rs-tree refusal, the fail-closed
# SOAK_SERVER_BINARY guard, the artifact-overwrite refusal, the smoke-mode
# refusal into the tracked evidence dir, the CSV header literal, the post-run
# column reports and the spec349c2-fit.awk fits are all carried verbatim.
# The departure is enumerable with:
#   diff spec363-durable.sh spec362b-durable.sh
#
# THE MATRIX IS EXECUTED, NOT TRANSCRIBED. Every knob is a literal in this
# file, so the record of what was run is this committed script rather than an
# operator's memory of a command line.
#
# Bash 3.2 (macOS system bash) compatible: no mapfile, no associative arrays,
# no ${x@Q}.
#
# Env overrides (all documented, all logged loudly when active):
#   SPEC362B_DATA_DIR              data dir for this run (default: target/)
#   SPEC362B_OUT_DIR               artifact dir     (default: this script's dir)
#   SPEC362B_SOAK_BIN              prebuilt soak_harness bench binary
#   SPEC362B_SMOKE_DURATION        SMOKE ONLY: override --duration (seconds)
#   SPEC362B_SMOKE_SAMPLE_INTERVAL SMOKE ONLY: override the CSV cadence
#   SPEC362B_FORCE=1               overwrite pre-existing artifacts
#   SOAK_SERVER_BINARY            REQUIRED on a provenance cell, REFUSED
#                                 elsewhere (see section 0b)
#
set -euo pipefail

# ---------------------------------------------------------------------------
# 0. Argument: the CELL id. Every output path, the width, the duration and the
#    provenance discipline are derived from it. There is no free-form knob: a
#    cell that is not in this table cannot be run.
# ---------------------------------------------------------------------------
CELL="${1:-}"

usage() {
  cat >&2 <<'EOF'
usage: spec362b-durable.sh <cell>

  Derived from spec363-durable.sh, which is not edited. The difference list
  against that file is CLOSED and is enumerated in the header block above.

  This runner ships ONE cell, and it is the deciding one:
    long4h     14400s, csv cadence 60s, crash-interval 0, live-census
               DISARMED, log directive ARMED, TOPGUN_EPOCH_WIDTH left unset
               (production default 1000). The duration is fixed: a longer
               cell is REFUSED, not deferred, because byte-comparability
               against the reference row is the whole basis on which this
               reads as a re-measurement rather than a new lineage.

  It ships NO instrument control, and cannot be made to run one: the cell
  table has no label for logctl-on, logctl-off or crashctl. Those three
  belong to spec362-durable.sh and spec363-durable.sh, they have been
  witnessed over two cycles, and running them again from here would create a
  fourth control set that nothing authorizes. Asking for one falls through to
  this text and exits 2.

  The run REFUSES TO START, before the build and before any clock, unless the
  .rs tree at HEAD is identical to the commit named by SPEC362B_PIN, and
  unless the .rs working tree is clean. Two guards, two distinct messages;
  neither has an override.

  A provenance cell REQUIRES SOAK_SERVER_BINARY to be exported and to name an
  existing executable. No cell in this runner's table is one; the guard is
  carried verbatim from the parent so a stray export still fails closed.
EOF
  exit 2
}

# The per-cell literals. Columns:
#   width      -- "" means UNSET, i.e. the PRODUCTION default (1000)
#   duration   -- seconds
#   cadence    -- CSV sample interval, seconds
#   provenance -- yes|no
#   crash      -- --crash-interval literal (0 => None: no kill -9 during the run)
#   live       -- the live-copy census sampler's cadence in seconds; 0 leaves
#                 that sampler DISARMED, as every cell of spec362-durable.sh
#                 did
#   armlog     -- yes|no: SOAK_SERVER_LOG carries the removal target's directive
#   extra      -- extra harness flags
#   base       -- artifact basename
case "$CELL" in
  # THE 4h DECIDING CELL, RESTORED. spec363-durable.sh deleted this label
  # because a 14400s cell was out of scope for the controls it shipped; here
  # it is the whole point of the file, and it is the ONLY label in the table.
  # Its literals are the frozen row's from spec362-manifest.md section 5,
  # plus CELL_LIVE_CENSUS, a knob that row predates and whose value 0 is in
  # substance that same section's "--live-census-interval is DISARMED on the
  # deciding cell". The duration is not a knob to tune: a longer cell is
  # REFUSED, not deferred.
  long4h)     WIDTH="";  DURATION=14400; SAMPLE_INTERVAL=60; PROVENANCE=no
              CELL_CRASH_INTERVAL=0; ARM_LOG=yes
              # --live-census-interval 0: DISARMED on the deciding cell, so
              # the census has exactly one (terminal) sample, which is the
              # structural end-of-run read that was pre-registered for it.
              CELL_LIVE_CENSUS=0
              EXTRA_FLAGS=""; BASE="spec362b-long4h" ;;
  # THE THREE <= 900s INSTRUMENT CONTROLS ARE DELETED, DELIBERATELY AND WITH
  # THEIR REASON WRITTEN DOWN -- the same argument spec363-durable.sh applied
  # to the 4h cell, applied here in the mirror direction. logctl-on,
  # logctl-off and crashctl are spec362-durable.sh's and spec363-durable.sh's
  # controls; they have already been witnessed over two cycles, and running
  # them from here would create a fourth control set and a new witness cycle
  # that nothing authorizes. A forbidden cell that is one keystroke away in a
  # runner this file ships is a hazard; deleting the labels converts the
  # temptation into a documented refusal. Asking for one falls through to
  # `usage` and exits 2. The controls live in spec363-durable.sh, which is
  # not edited and remains runnable for them.
  *)         usage ;;
esac

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
EVIDENCE_DIR="$SCRIPT_DIR"
SERVER_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"     # packages/server-rust
REPO_ROOT="$(cd "$SERVER_ROOT/../.." && pwd)"

OUT_DIR="${SPEC362B_OUT_DIR:-$EVIDENCE_DIR}"
CSV_OUT="${OUT_DIR}/${BASE}.csv"
JSON_OUT="${OUT_DIR}/${BASE}.soak.json"
PROGRESS_OUT="${OUT_DIR}/${BASE}.progress.jsonl"
# The effective-matrix echo, committed beside the series it describes.
MATRIX_OUT="${OUT_DIR}/${BASE}.matrix.txt"
# The harness derives the mechanism report's path from --json-output via Rust's
# `Path::with_extension`, which replaces the LAST extension only: it writes
# "<base>.soak.mechanism.json". The ledger for these runs names
# "<base>.mechanism.json", so the runner normalizes the name after the run
# rather than leaving the committed tree disagreeing with the ledger.
MECH_RAW="${OUT_DIR}/${BASE}.soak.mechanism.json"
MECH_OUT="${OUT_DIR}/${BASE}.mechanism.json"
# The durable reading's sibling artifact, derived the same way. It is
# DELIBERATELY NOT given the symmetric rename the mechanism report gets: the
# spec's jq paths read this raw basename, and a normalize hunk here would
# silently break all of them. The asymmetry is intended.
DURABLE_OUT="${OUT_DIR}/${BASE}.soak.durable.json"
# The harness's own stdout, committed IN THE EVIDENCE DIRECTORY. The parent
# runner left it under target/, which is the artifact-mortality the 349c2
# manifest's 7.3 records; here it is written straight to its committed home.
CONSOLE_OUT="${OUT_DIR}/${BASE}.harness-console.log"

# ---------------------------------------------------------------------------
# 0b. FAIL-CLOSED SERVER-BINARY RESOLUTION -- the provenance guard.
#
#     This is a REFUSAL, not a knob: it determines nothing about rate, shape or
#     duration. It exists because the Rust resolver fails OPEN. With
#     SOAK_SERVER_BINARY absent from the bench process's environment,
#     `resolve_server_binary()` (src/.../process.rs:421-426) silently returns
#     the bench's compile-time default server path (process.rs:422-425) with no
#     existence check and no warning -- so a variable exported in the wrong
#     subshell, or a worktree whose `cargo build` landed in the shared target/,
#     turns the provenance cell into a second cell A while every console line
#     still looks correct.
#
#     The compile-time default's macro name is deliberately NOT spelled out
#     anywhere in this file: the spec's Validation Checklist item 14 greps this
#     runner for that token and requires zero hits on the cell-C path, and a
#     check that has to distinguish a comment from a fallback is not mechanical.
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
    echo "       cell a second cell A." >&2
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
  # Unset => the child is the topgun-server the harness was compiled beside,
  # which is what makes the harness-side epoch-width derivation a valid proxy
  # for the server's behaviour. A stray export from a previous provenance run
  # in the same shell would otherwise silently make a HEAD cell a provenance
  # cell -- the same failure in the other direction.
  if [ -n "${SOAK_SERVER_BINARY:-}" ]; then
    echo "WARNING: SOAK_SERVER_BINARY was exported ('${SOAK_SERVER_BINARY}') but cell" >&2
    echo "         '${CELL}' is NOT a provenance cell. Unsetting it: only cellC /" >&2
    echo "         cellD / cellE may vary the server binary." >&2
  fi
  unset SOAK_SERVER_BINARY || true
  PROV_BIN=""
fi

DATA_DIR="${SPEC362B_DATA_DIR:-${REPO_ROOT}/target/spec362b-${CELL}-data}"
META_DIR="${DATA_DIR}.meta"      # sibling: NEVER inside the measured data dir
CONSOLE_LOG="${META_DIR}/harness-console.log"
STOP_FILE="${META_DIR}/sampler.stop"
FAIL_FILE="${META_DIR}/sampler.fail"

# ---------------------------------------------------------------------------
# 2. Smoke override -- only ever changes the duration (and, with it, the CSV
#    cadence, which would otherwise yield 3 rows). The real runs must not be
#    able to take this path by accident, so the defaults are the per-cell
#    literals above, the override is loud, and it is REFUSED if the artifacts
#    would land in the tracked evidence directory.
#
#    "Smoke" is DEFINED BY THIS OVERRIDE PATH, not by any duration threshold.
#    A duration threshold would refuse R4.3's 360s / 420s control runs, whose
#    artifacts DO belong in the tracked directory.
# ---------------------------------------------------------------------------
SMOKE=0
if [ -n "${SPEC362B_SMOKE_DURATION:-}" ]; then
  SMOKE=1
  DURATION="$SPEC362B_SMOKE_DURATION"
  if [ -n "${SPEC362B_SMOKE_SAMPLE_INTERVAL:-}" ]; then
    SAMPLE_INTERVAL="$SPEC362B_SMOKE_SAMPLE_INTERVAL"
  fi
  echo "############################################################"
  echo "## SMOKE MODE -- THIS IS NOT A CHARACTERIZATION RUN       ##"
  echo "##   --duration      = ${DURATION}s (override)            "
  echo "##   CSV cadence     = ${SAMPLE_INTERVAL}s                 "
  echo "## Its artifacts MUST NOT be committed as evidence.       ##"
  echo "############################################################"
  if [ "$OUT_DIR" = "$EVIDENCE_DIR" ]; then
    echo "REFUSING: smoke mode would write into the tracked evidence dir" >&2
    echo "  $EVIDENCE_DIR" >&2
    echo "Set SPEC362B_OUT_DIR to a scratch directory." >&2
    exit 2
  fi
elif [ -n "${SPEC362B_SMOKE_SAMPLE_INTERVAL:-}" ]; then
  echo "WARNING: SPEC362B_SMOKE_SAMPLE_INTERVAL ignored outside smoke mode;" >&2
  echo "         this cell's CSV cadence is pinned at ${SAMPLE_INTERVAL}s." >&2
fi

if [ "$OUT_DIR" != "$EVIDENCE_DIR" ]; then
  echo "WARNING: artifact dir overridden to $OUT_DIR"
  echo "         A characterization run's artifacts belong in $EVIDENCE_DIR"
  echo "         (git-tracked); artifacts written elsewhere are not evidence."
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
#    The ONE deliberate departure from the parent runner: TOPGUN_EPOCH_WIDTH is
#    the axis under test, so it is SET from this cell's literal rather than
#    unconditionally unset. Every other line here is the parent's, verbatim.
# ---------------------------------------------------------------------------
if [ -n "$WIDTH" ]; then
  export TOPGUN_EPOCH_WIDTH="$WIDTH"
else
  # Unset => the server applies the PRODUCTION default epoch width (1000),
  # which is the width the gate actually runs at and the width cell A was
  # measured at.
  unset TOPGUN_EPOCH_WIDTH || true
fi
# Unset => the harness supplies its own 100ms / 5000 write-behind cadence, which
# is the instrument-identity choice shared with every other soak run. An export
# here silently changes when bytes land in redb vs the WAL.
unset TOPGUN_WRITEBEHIND_FLUSH_INTERVAL_MS || true
unset TOPGUN_WRITEBEHIND_BATCH_SIZE || true
# The target-scoped log directive of the one declared departure. Target-scoped
# on purpose: `warn` for everything, `info` for the removal target alone. It
# is defined HERE, in the discipline block that consumes it, because the
# discipline block runs before the pinned-matrix literals below.
ORIGIN_LOG_DIRECTIVE='warn,topgun_server::tombstone_frontier::removal=info'
# THE ONE DECLARED DEPARTURE from the parent runner, which unsets this
# variable so the child runs at RUST_LOG=warn. The origin read needs the
# removal target's line to be EMITTED, so on an ARM_LOG cell the directive is
# SET, target-scoped: everything else stays at warn, and only the one target
# is raised to info. The UNSET arm keeps the parent's behaviour verbatim,
# which is what makes the pair a price rather than an assertion.
if [ "$ARM_LOG" = "yes" ]; then
  export SOAK_SERVER_LOG="$ORIGIN_LOG_DIRECTIVE"
else
  unset SOAK_SERVER_LOG || true
fi
# Unset => the fmt layer renders the human format the origin parser reads.
# TOPGUN_LOG_FORMAT=json switches it to .json(), the harness never calls
# env_clear(), so an exported `json` in the operator's shell would turn every
# origin line into an unparsed one -- fail-closed, but it would also change
# the cost of the departure above silently and differently between arms run
# from different shells. Unset at the source rather than guarded downstream.
unset TOPGUN_LOG_FORMAT || true
# Unset => teardown is SIGKILL, as every other soak run's is.
unset TOPGUN_SOAK_GRACEFUL_SHUTDOWN || true
# Unset => production memory ceiling and eviction water marks. Eviction cadence
# is one of the things these runs are measuring the effect of; overriding it
# would answer a question about a different system.
unset TOPGUN_MAX_RAM_MB || true
unset TOPGUN_EVICTION_HIGH_PCT || true
unset TOPGUN_EVICTION_LOW_PCT || true
unset TOPGUN_EVICTION_INTERVAL_MS || true
# Unset => the emitter is ARMED, the shipped default, which is the state the
# reference cell A (the 349c2 ON arm) was measured in. This spec varies WIDTH,
# never the emitter.
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
#    spec349c2-plateau.sh literals verbatim: every rate- and shape-determining
#    knob below is byte-identical to the parent runner's, so this spec's cells
#    and spec349c2's committed width-1000 arms are the same matrix.
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
# Per-cell: 0 on the deciding cell (0 parses to None: NO kill -9 during the
# run). The non-zero setting belongs to the crash control spec363-durable.sh
# carries, where it is an instrument-branch parameter rather than a matrix
# change; nothing on this runner's table reaches it.
CRASH_INTERVAL="$CELL_CRASH_INTERVAL"
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
SERVER_PORT=47355       # fixed, so the child server has a reliable handle
# The sampler-jitter seed. ONE shell variable: it is passed to the binary as
# --sampler-jitter-seed AND echoed into matrix.txt below from this same
# source, so the recorded seed and the used seed cannot diverge.
JITTER_SEED=20260831

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

# ---------------------------------------------------------------------------
# 4a. THE COMMIT PIN. The whole point of this runner's spec is that its
#     witnesses belong to the NEW instrument, and an mtime-selected bench
#     binary cannot show that: `ls -t deps/soak_harness-*` answers "the newest
#     one", not "the one built from the tree these artifacts are filed under".
#     So the binary is BUILT HERE, from HEAD, after refusing to start on a
#     dirty .rs working tree -- and the commit is echoed into matrix.txt, where
#     it is committed beside the series it produced.
# ---------------------------------------------------------------------------
SOAK_BIN_COMMIT="$(git -C "$REPO_ROOT" rev-parse HEAD 2>/dev/null || true)"
if [ -z "$SOAK_BIN_COMMIT" ]; then
  echo "FATAL: $REPO_ROOT is not a git checkout, so this run cannot be pinned" >&2
  echo "       to the commit that produced it. Refusing to start." >&2
  exit 1
fi
# The pin these artifacts are filed under. A literal, because
# `git merge-base origin/main HEAD` moves as origin/main moves, and a symbol an
# acceptance check compares against must not drift after the run. It is used
# THREE times from this one variable and re-typed nowhere: the fail-closed
# pre-flight immediately below, the matrix.txt echo of the pin, and the
# matrix.txt echo of that pre-flight's verdict -- so the pin the artifact
# records, the pin the run enforced and the verdict the artifact reports
# cannot diverge.
SPEC362B_PIN=eddf50ea6c1fae3f01171d63925cf9e93ceb24e9
# Fail-closed pre-flight, and it stands HERE on purpose: before the inherited
# dirty-tree guard below and before the build, so a refusal costs no compile
# and "asserted before the build" is true by construction rather than by the
# author's care. A .rs byte that landed after the pin means the artifacts
# would be filed under a commit that did not produce them. -C anchors the
# pathspec to the repo root, as every sibling git call in this runner does.
if ! PIN_RS_DIFF="$(git -C "$REPO_ROOT" diff --stat "$SPEC362B_PIN"..HEAD -- '*.rs' 2>&1)"; then
  echo "FATAL: the .rs diff against the pin ${SPEC362B_PIN} could not be" >&2
  echo "       computed, so the pin cannot be asserted:" >&2
  printf '%s\n' "$PIN_RS_DIFF" >&2
  echo "       Refusing to start." >&2
  exit 1
fi
if [ -n "$PIN_RS_DIFF" ]; then
  echo "FATAL: the .rs tree at HEAD differs from the pin ${SPEC362B_PIN}; this run would not be filed under the pin" >&2
  printf '%s\n' "$PIN_RS_DIFF" >&2
  exit 1
fi
PIN_DIFF_STATE="EMPTY (asserted before the build)"
RS_DIRTY="$(git -C "$REPO_ROOT" status --porcelain -- '*.rs' 2>/dev/null || true)"
if [ -n "$RS_DIRTY" ]; then
  echo "FATAL: the .rs working tree is DIRTY, so the binary this run would" >&2
  echo "       build is not the commit it would be filed under:" >&2
  printf '%s\n' "$RS_DIRTY" >&2
  echo "       Commit or stash the .rs changes and re-run." >&2
  exit 1
fi
RS_TREE_STATE="CLEAN (asserted before the build)"

if [ -n "${SPEC362B_SOAK_BIN:-}" ]; then
  echo "WARNING: SPEC362B_SOAK_BIN is set, so this runner did NOT build the bench" >&2
  echo "         binary from HEAD. The commit pin is NOT discharged for this run" >&2
  echo "         and matrix.txt will say so." >&2
else
  if [ -z "${SDKROOT:-}" ] && [ -x /usr/bin/xcrun ]; then
    SDKROOT="$(/usr/bin/xcrun --sdk macosx --show-sdk-path 2>/dev/null || true)"
    [ -n "$SDKROOT" ] && export SDKROOT
  fi
  BUILD_TARGETS="--bench soak_harness"
  if [ "$PROVENANCE" != "yes" ]; then
    # One build produces both binaries, which is what the build-gap check below
    # exists to confirm.
    BUILD_TARGETS="--bin topgun-server $BUILD_TARGETS"
  fi
  echo "building from HEAD ${SOAK_BIN_COMMIT}: cargo build --release ${BUILD_TARGETS}"
  # shellcheck disable=SC2086
  if ! ( cd "$SERVER_ROOT" && cargo build --release $BUILD_TARGETS ); then
    echo "FATAL: the release build failed; nothing was run." >&2
    exit 1
  fi
  # cargo writes into the WORKSPACE target dir, which may not have existed when
  # TARGET_DIR was derived above.
  if [ -z "${CARGO_TARGET_DIR:-}" ] && [ -d "${REPO_ROOT}/target/release" ]; then
    TARGET_DIR="${REPO_ROOT}/target"
  fi
fi

if [ "$PROVENANCE" = "yes" ]; then
  # The server under test is the PROVENANCE binary, and it is the only one the
  # child will be. The HEAD binary's presence is irrelevant to this cell and is
  # deliberately not checked, so a reader of the matrix echo cannot mistake it
  # for the binary that ran.
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

SOAK_BIN="${SPEC362B_SOAK_BIN:-}"
if [ -n "$SOAK_BIN" ]; then
  # An overridden binary was not built by the block above, so the pin is not
  # proven for it. Say so in the artifact rather than printing a commit this
  # runner cannot vouch for.
  SOAK_BIN_COMMIT="<UNPROVEN: SPEC362B_SOAK_BIN override, not built by this run>"
else
  # Newest under deps/ -- but, unlike the parent, it is newest immediately
  # after this runner built it from a clean tree at the commit recorded above.
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
  # instrument against an older server. Reporting it as a warning would train
  # the operator to ignore the warning that matters on the HEAD cells.
  echo "  NOTE: this is a PROVENANCE cell -- a deliberate half-swap. The"
  echo "        instrument (bench binary, gauge scrape, CSV column, fit,"
  echo "        assessment) is at HEAD; only the SERVER is the pinned older"
  echo "        build. The ${BUILD_GAP}s build gap is expected, not a defect."
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
rm -f "$STOP_FILE" "$FAIL_FILE"

# Refuse to silently overwrite artifacts: a re-run that clobbers a recorded
# series destroys the only copy of a measurement.
for f in "$CSV_OUT" "$JSON_OUT" "$PROGRESS_OUT" "$MECH_OUT" "$MECH_RAW" "$DURABLE_OUT" "$MATRIX_OUT" "$CONSOLE_OUT"; do
  if [ -e "$f" ] && [ "${SPEC362B_FORCE:-0}" != "1" ]; then
    echo "FATAL: artifact already exists: $f" >&2
    echo "       Move it aside, or re-run with SPEC362B_FORCE=1 to overwrite." >&2
    exit 1
  fi
done
for f in "$CSV_OUT" "$JSON_OUT" "$PROGRESS_OUT" "$MATRIX_OUT" "$CONSOLE_OUT"; do
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
  echo "=== spec362b durable run: cell ${CELL} ==="
  echo "  repo HEAD:      $(git -C "$REPO_ROOT" rev-parse HEAD 2>/dev/null || echo '<not a git checkout>')"
  echo "  dirty tree:     $(test -n "$(git -C "$REPO_ROOT" status --porcelain 2>/dev/null || true)" && echo yes || echo no)"
  echo "  host/OS:        $(uname -a)"
  echo "  lineage:        $( [ "$PROVENANCE" = "yes" ] && echo 'PROVENANCE (HEAD harness + pinned older server)' || echo 'HEAD harness + HEAD server' )"
  echo "  data dir:       $DATA_DIR"
  echo "  csv:            $CSV_OUT"
  echo "  soak.json:      $JSON_OUT"
  echo "  mechanism.json: $MECH_OUT (harness writes $(basename "$MECH_RAW"); renamed after the run)"
  echo "  durable.json:   $DURABLE_OUT (raw basename: NOT renamed, by design)"
  echo "  progress.jsonl: $PROGRESS_OUT"
  echo "  console log:    $CONSOLE_OUT"
  echo "  matrix:         $MATRIX_OUT"
  echo "  soak binary:    $SOAK_BIN"
  echo "  soak binary commit: ${SOAK_BIN_COMMIT}"
  echo "  .rs working tree:   ${RS_TREE_STATE}"
  echo "  pin:                ${SPEC362B_PIN}"
  echo "  pin diff (.rs):     ${PIN_DIFF_STATE}"
  echo "    built:        $(date -r "$SOAK_BIN" -u '+%Y-%m-%dT%H:%M:%SZ')"
  echo "  server binary:  $SERVER_BIN"
  echo "    built:        $(date -r "$SERVER_BIN" -u '+%Y-%m-%dT%H:%M:%SZ')"
  echo "  --- varied knobs (AC16's enumerated list; everything else is the"
  echo "      spec355-width.sh literal) ---"
  echo "  TOPGUN_EPOCH_WIDTH:  ${TOPGUN_EPOCH_WIDTH:-<unset: production default 1000>}"
  echo "  duration:            ${DURATION}s$( [ "$SMOKE" = "1" ] && echo '  <-- SMOKE OVERRIDE')"
  echo "  csv cadence:         ${SAMPLE_INTERVAL}s"
  echo "  extra harness flags: ${EXTRA_FLAGS:-<none>}"
  echo "  --durable-reading:   ARMED (every cell in this table)"
  echo "  --live-census-interval: ${CELL_LIVE_CENSUS}$( [ "$CELL_LIVE_CENSUS" = "0" ] && echo ' (DISARMED)' || echo ' (ARMED: the live-copy census)' )"
  echo "  --sampler-jitter-seed:  ${JITTER_SEED}"
  echo "  SOAK_SERVER_LOG:     ${SOAK_SERVER_LOG:-<unset: parent behaviour, RUST_LOG=warn>}"
  echo "  TOPGUN_LOG_FORMAT:   ${TOPGUN_LOG_FORMAT:-<unset: human fmt, which is what the origin parser reads>}"
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
    echo "  pinned SHA:            ${SPEC362B_PIN_SHA:-<record it here: export SPEC362B_PIN_SHA>}"
    echo "  pin resolved by:       ${SPEC362B_PIN_CMD:-<record it here: export SPEC362B_PIN_CMD>}"
    echo "  worktree path:         ${SPEC362B_PIN_WORKTREE:-<record it here: export SPEC362B_PIN_WORKTREE>}"
  fi
  echo "  --- pinned matrix (identical to spec355-width.sh; crash-interval is"
  echo "      per-cell and is 0 on the one cell this runner ships) ---"
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
cleanup() {
  if [ -n "$SAMPLER_PID" ] && kill -0 "$SAMPLER_PID" 2>/dev/null; then
    kill "$SAMPLER_PID" 2>/dev/null || true
  fi
  if [ -n "$HARNESS_PID" ] && kill -0 "$HARNESS_PID" 2>/dev/null; then
    kill "$HARNESS_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT INT TERM

# EXTRA_FLAGS is deliberately unquoted: it is a fixed literal from this file's
# own case table (either empty, "--no-ack" or "--inject-slow-leak"), never
# operator input, and bash 3.2 has no arrays worth threading for one token.
# shellcheck disable=SC2086
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
  --durable-reading \
  --live-census-interval "$CELL_LIVE_CENSUS" \
  --sampler-jitter-seed "$JITTER_SEED" \
  $EXTRA_FLAGS \
  > "$CONSOLE_LOG" 2>&1 &
HARNESS_PID=$!
echo "harness pid $HARNESS_PID; follow with: tail -f $CONSOLE_LOG"

# ---------------------------------------------------------------------------
# 8. Wait for server-ready. This is the CSV's time origin.
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
# 9. The per-minute sampler.
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
set -e
SAMPLER_PID=""

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

INSTRUMENT_OK=1
fail_instrument() { echo "INSTRUMENT DEFECT: $1" >&2; INSTRUMENT_OK=0; }

if [ -s "$FAIL_FILE" ]; then
  fail_instrument "sampler aborted: $(cat "$FAIL_FILE")"
fi

HEADER="$(head -1 "$CSV_OUT" 2>/dev/null || true)"
if [ "$HEADER" != 'elapsed_secs,rss_mb,wal_mb,redb_mb,disk_total_mb,tombstone_bytes' ]; then
  fail_instrument "CSV header is '$HEADER'"
fi
ROWS="$(( $(wc -l < "$CSV_OUT") - 1 ))"
echo "csv rows: $ROWS"
if [ "$ROWS" -lt 2 ]; then
  fail_instrument "CSV has $ROWS data rows"
fi

col_report() {   # $1 = 1-based column index, $2 = name
  awk -F, -v idx="$1" -v name="$2" '
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
      printf "  %-14s n=%d empty=%d nonzero=%d min=%.3f max=%.3f\n",
             name, n, empty + 0, nonzero + 0, min + 0, max + 0
      if (empty > 0 || n == 0 || nonzero == 0) exit 1
    }
  ' "$CSV_OUT"
}
echo "csv columns:"
col_report 2 rss_mb        || fail_instrument "rss_mb column is empty or all-zero (blind sampler)"
col_report 3 wal_mb        || fail_instrument "wal_mb column is empty or all-zero (wrong WAL path?)"
col_report 4 redb_mb       || fail_instrument "redb_mb column is empty or all-zero (wrong redb path?)"
col_report 5 disk_total_mb || fail_instrument "disk_total_mb column is empty or all-zero"

# POPULATION check, not non-zero: a genuinely bounded tombstone corpus may
# legitimately read 0 for a whole run.
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

for f in "$JSON_OUT" "$MECH_OUT" "$DURABLE_OUT"; do
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
# 11b. PROVENANCE cells only: the census IDENTITY WITNESS.
#
#      The pinned SHA precedes the OR-delta emitter (merged 7142d4dc), so a
#      pre-family server cannot write a single OR delta frame. A nonzero
#      orDeltaFrames therefore proves the swap did NOT happen and this cell is
#      really a second cell A. That outcome is INVALID -- it is not a
#      decision-table row, it is not "reproduces", and it must never be
#      reasoned about in the manifest. The run is re-done.
#
#      This is reported here rather than left to a post-hoc jq so that the
#      operator learns it at the end of the run that produced it.
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
    echo "  The pinned pre-family server CANNOT emit OR delta frames, so this run" >&2
    echo "  was driven by the HEAD binary: the swap was BOTCHED." >&2
    echo "  Do NOT route this cell into a decision-table row. Fix the swap and re-run." >&2
    INSTRUMENT_OK=0
  elif [ -z "${OSF:-}" ] || [ "$OSF" -le 0 ] 2>/dev/null; then
    echo "CELL INVALID: orSnapshotFrames = ${OSF:-<unreadable>} is not > 0, so the OR" >&2
    echo "  path was not exercised and orDeltaFrames == 0 proves nothing." >&2
    INSTRUMENT_OK=0
  else
    echo "  => identity witness PASSED: this cell ran the pre-family server."
  fi
fi

# ---------------------------------------------------------------------------
# 12. Convenience: the post-hoc fits.
# ---------------------------------------------------------------------------
FIT="${SCRIPT_DIR}/spec349c2-fit.awk"
if [ -f "$FIT" ] && [ "$ROWS" -ge 4 ]; then
  echo
  echo "post-hoc OLS fits (see the SE caveat in $(basename "$FIT")):"
  echo "  units: the field is named slope_mb_per_hour for every column; on the"
  echo "  tombstone_bytes row the fit is identical but the unit is BYTES/hour."
  for w in full last_half; do
    for c in rss_mb wal_mb redb_mb disk_total_mb tombstone_bytes; do
      awk -v col="$c" -v window="$w" -f "$FIT" "$CSV_OUT" 2>&1 | sed 's/^/  /' || true
    done
  done
fi

echo
if [ "$INSTRUMENT_OK" != "1" ]; then
  echo "RESULT: INSTRUMENT DEFECT -- this run's series must not be recorded as evidence." >&2
  exit 9
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
