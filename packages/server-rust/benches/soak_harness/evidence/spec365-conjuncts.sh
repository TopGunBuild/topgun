#!/usr/bin/env bash
#
# Prune-conjunct readout cell runner.
#
# Derived from the committed spec362b-durable.sh. THE RATE-, SHAPE- AND
# DURATION-DETERMINING MATRIX IS IDENTICAL to that file's 4-hour deciding
# cell, and the ONLY permitted differences are the ones enumerated below.
# No parent runner is edited; the difference list against spec362b-durable.sh
# is CLOSED:
#
#   (a) IDENTITY. Artifact prefix spec365-conj900 and, with it, the renamed
#       SPEC365_* env-var override surface -- the COMPLETE renamed variable
#       surface, the three dead provenance-branch echo variables included --
#       the run's data dir, and the matrix banner. ONE override keeps the
#       parent's literal name rather than being renamed: SPEC362B_SMOKE_DURATION
#       (see (c) below). The parent's own commit-pin variable is not merely
#       renamed but restructured; see (g).
#   (b) THE CELL. A single label, conj900: 900s duration, 60s CSV cadence,
#       width unset (production default), every other matrix literal is the
#       parent's 4-hour-cell literal. --durable-reading stays armed, as on
#       every cell this lineage ships.
#   (c) THE INSTRUMENT IS ARMED. TOPGUN_PRUNE_RECORD is exported true, and the
#       target-scoped log directive widens from the parent's one target
#       (removal) to three (removal, settlement, conjunct), armed together so
#       a reconciliation reader sees all three kinds of line from one run. The
#       harness's own log-passthrough switch is set so those lines land in the
#       committed console log this file's readout reads. The residency target
#       is deliberately NOT armed here: it fires once per OR write, and this
#       cell does not need a per-write log. For the short local smoke this
#       runner's own conj900 cell is not built to carry, the smoke override
#       knob keeps the parent's literal name (SPEC362B_SMOKE_DURATION) rather
#       than defining a new one, so smoke tooling written against the parent
#       needs no relearning.
#   (d) ONE SCRAPE PER ROW. The sampler's one HTTP GET against /metrics now
#       supplies both the inherited tombstone-bytes gauge and every new
#       counter/gauge column this file adds, parsed from that single response
#       body. A name absent from the body renders an EMPTY cell, never a
#       zero -- a gap in a series must stay visible as a gap.
#   (e) A PER-ROW PROCESS MEMORY BREAKDOWN. After the existing ps-based RSS
#       read, one additional per-process sample is taken and parsed into four
#       more columns. Its parser was written and exercised against a
#       committed real capture before this runner was written, so field-name
#       drift in that external tool's output is caught before the CSV header
#       promises columns nothing can fill.
#   (f) THE HEADER WIDENS. The CSV gains new columns to the right of the
#       inherited six; the inherited six stay byte-identical in content and
#       position.
#   (g) THE FREEZE GATE REPLACES THE PIN. The parent's commit-pin literal
#       becomes SPEC365_CODE_FREEZE here, with one added behaviour the parent
#       never needed: while this literal still reads its own placeholder
#       value (because the code it would pin does not exist yet), the runner
#       refuses immediately, before any git call that would otherwise need to
#       resolve a nonsense revision. Once a real commit lands in its place,
#       the gate behaves exactly like the parent's pin: a fail-closed
#       pre-flight against a dirty or drifted .rs tree, asserted before any
#       build.
#   (h) A READOUT RUN. After a sound instrument run, this file additionally
#       invokes the companion mechanical readout on this cell's own
#       artifacts.
#
# NO OTHER RATE- OR SHAPE-DETERMINING LITERAL CHANGES. The server port, the
# environment-discipline block, the build-from-HEAD step and its dirty-.rs
# guard, the artifact-overwrite refusal, the smoke-mode refusal into the
# tracked evidence dir, the six inherited CSV columns' sampler logic, the
# post-run instrument checks and the spec349c2-fit.awk fits are all carried
# over unchanged in shape, from spec362b-durable.sh. The departure is
# enumerable with:
#   diff spec362b-durable.sh spec365-conjuncts.sh
#
# THE FREEZE GATE (g) PINS THE .rs TREE. SPEC365_CODE_FREEZE names the commit
# at which the instrument this file samples was complete and the full gate
# matrix green. The runner refuses to start if the literal ever reads its
# placeholder again, or if any .rs file at HEAD differs from that commit, so
# the series it records can only come from the frozen instrument.
#
# THE MATRIX IS EXECUTED, NOT TRANSCRIBED. Every knob is a literal in this
# file, so the record of what was run is this committed script rather than an
# operator's memory of a command line.
#
# Bash 3.2 (macOS system bash) compatible: no mapfile, no associative arrays,
# no ${x@Q}.
#
# Env overrides (all documented, all logged loudly when active):
#   SPEC365_DATA_DIR                data dir for this run (default: target/)
#   SPEC365_OUT_DIR                 artifact dir     (default: this script's dir)
#   SPEC365_SOAK_BIN                prebuilt soak_harness bench binary
#   SPEC362B_SMOKE_DURATION         SMOKE ONLY: override --duration (seconds);
#                                    kept under the parent's literal name, see (c)
#   SPEC365_SMOKE_SAMPLE_INTERVAL   SMOKE ONLY: override the CSV cadence
#   SPEC365_FORCE=1                 overwrite pre-existing artifacts
#   SOAK_SERVER_BINARY              REQUIRED on a provenance cell, REFUSED
#                                    elsewhere (see section 0b); no cell in
#                                    this file's table is a provenance cell
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
usage: spec365-conjuncts.sh <cell>

  Derived from spec362b-durable.sh, which is not edited. The difference list
  against that file is CLOSED and is enumerated in the header block above.

  This runner ships ONE cell:
    conj900    900s, csv cadence 60s, crash-interval 0, live-census DISARMED,
               log directive ARMED across three targets (removal, settlement,
               conjunct), TOPGUN_PRUNE_RECORD armed, TOPGUN_EPOCH_WIDTH left
               unset (production default 1000). Every other matrix literal is
               the parent's 4-hour-cell literal, unchanged.

  Any other argument falls through to this text and exits 2.

  The run REFUSES TO START, before the build and before any clock, if
  SPEC365_CODE_FREEZE still reads its own placeholder value, or unless the
  .rs tree at HEAD is identical to the commit that literal names once it is
  written, or unless the .rs working tree is clean. Three guards, three
  distinct messages; none has an override.

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
#                 that sampler DISARMED
#   armlog     -- yes|no: SOAK_SERVER_LOG carries the removal/settlement/conjunct
#                 target directive
#   extra      -- extra harness flags
#   base       -- artifact basename
case "$CELL" in
  conj900)    WIDTH="";  DURATION=900; SAMPLE_INTERVAL=60; PROVENANCE=no
              CELL_CRASH_INTERVAL=0; ARM_LOG=yes
              CELL_LIVE_CENSUS=0
              EXTRA_FLAGS=""; BASE="spec365-conj900" ;;
  *)          usage ;;
esac

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
EVIDENCE_DIR="$SCRIPT_DIR"
SERVER_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"     # packages/server-rust
REPO_ROOT="$(cd "$SERVER_ROOT/../.." && pwd)"

OUT_DIR="${SPEC365_OUT_DIR:-$EVIDENCE_DIR}"
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
# readout reads this raw basename, and a normalize hunk here would silently
# break that.
DURABLE_OUT="${OUT_DIR}/${BASE}.soak.durable.json"
# The harness's own stdout, committed IN THE EVIDENCE DIRECTORY.
CONSOLE_OUT="${OUT_DIR}/${BASE}.harness-console.log"
# The readout's own output, written by spec365-readout.sh, not by this file.
READOUT_OUT="${OUT_DIR}/${BASE}.readout.txt"

# ---------------------------------------------------------------------------
# 0b. FAIL-CLOSED SERVER-BINARY RESOLUTION -- the provenance guard.
#
#     This is a REFUSAL, not a knob: it determines nothing about rate, shape or
#     duration. It exists because the Rust resolver fails OPEN. With
#     SOAK_SERVER_BINARY absent from the bench process's environment,
#     `resolve_server_binary()` silently returns the bench's compile-time
#     default server path with no existence check and no warning -- so a
#     variable exported in the wrong subshell, or a worktree whose
#     `cargo build` landed in the shared target/, turns the provenance cell
#     into a second cell A while every console line still looks correct.
#
#     A cell that silently measures the wrong binary is the one failure that
#     looks most like success, so on the provenance path this runner refuses
#     to start unless the variable is set to an existing executable file.
#     There is deliberately NO fallback here.
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
  if [ -n "${SOAK_SERVER_BINARY:-}" ]; then
    echo "WARNING: SOAK_SERVER_BINARY was exported ('${SOAK_SERVER_BINARY}') but cell" >&2
    echo "         '${CELL}' is NOT a provenance cell. Unsetting it: only a" >&2
    echo "         provenance cell may vary the server binary." >&2
  fi
  unset SOAK_SERVER_BINARY || true
  PROV_BIN=""
fi

DATA_DIR="${SPEC365_DATA_DIR:-${REPO_ROOT}/target/spec365-${CELL}-data}"
META_DIR="${DATA_DIR}.meta"      # sibling: NEVER inside the measured data dir
CONSOLE_LOG="${META_DIR}/harness-console.log"
STOP_FILE="${META_DIR}/sampler.stop"
FAIL_FILE="${META_DIR}/sampler.fail"

# ---------------------------------------------------------------------------
# 2. Smoke override -- only ever changes the duration (and, with it, the CSV
#    cadence, which would otherwise yield 3 rows). The real run must not be
#    able to take this path by accident, so the defaults are the per-cell
#    literals above, the override is loud, and it is REFUSED if the artifacts
#    would land in the tracked evidence directory.
# ---------------------------------------------------------------------------
SMOKE=0
if [ -n "${SPEC362B_SMOKE_DURATION:-}" ]; then
  SMOKE=1
  DURATION="$SPEC362B_SMOKE_DURATION"
  if [ -n "${SPEC365_SMOKE_SAMPLE_INTERVAL:-}" ]; then
    SAMPLE_INTERVAL="$SPEC365_SMOKE_SAMPLE_INTERVAL"
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
    echo "Set SPEC365_OUT_DIR to a scratch directory." >&2
    exit 2
  fi
elif [ -n "${SPEC365_SMOKE_SAMPLE_INTERVAL:-}" ]; then
  echo "WARNING: SPEC365_SMOKE_SAMPLE_INTERVAL ignored outside smoke mode;" >&2
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
#    operator's shell would silently change what this run measures, so they
#    are ACTIVELY UNSET rather than merely not set.
# ---------------------------------------------------------------------------
if [ -n "$WIDTH" ]; then
  export TOPGUN_EPOCH_WIDTH="$WIDTH"
else
  # Unset => the server applies the PRODUCTION default epoch width (1000).
  unset TOPGUN_EPOCH_WIDTH || true
fi
# Unset => the harness supplies its own 100ms / 5000 write-behind cadence, which
# is the instrument-identity choice shared with every other soak run.
unset TOPGUN_WRITEBEHIND_FLUSH_INTERVAL_MS || true
unset TOPGUN_WRITEBEHIND_BATCH_SIZE || true
# The target-scoped log directive: `warn` for everything, `info` for the
# removal, settlement and conjunct targets, all three armed together so one
# run's console log carries every line kind the readout joins.
ORIGIN_LOG_DIRECTIVE='warn,topgun_server::tombstone_frontier::removal=info,topgun_server::tombstone_frontier::settlement=info,topgun_server::tombstone_frontier::conjunct=info'
if [ "$ARM_LOG" = "yes" ]; then
  export SOAK_SERVER_LOG="$ORIGIN_LOG_DIRECTIVE"
  # The harness mirrors matched lines to its own stderr only under this
  # switch, and the harness's stdout+stderr are both redirected into
  # CONSOLE_LOG below -- so this is what makes the [server] -prefixed lines
  # the readout parses actually land in the committed console log.
  export SOAK_SERVER_LOG_PASSTHROUGH=1
else
  unset SOAK_SERVER_LOG || true
  unset SOAK_SERVER_LOG_PASSTHROUGH || true
fi
# Arms the scrape-time snapshot's metric family. The family's own default is
# already armed when this is unset, but the run states it explicitly rather
# than leaning on that default, matching every other line in this block.
export TOPGUN_PRUNE_RECORD=true
# Unset => the fmt layer renders the human format the origin parser reads.
unset TOPGUN_LOG_FORMAT || true
# Unset => teardown is SIGKILL, as every other soak run's is.
unset TOPGUN_SOAK_GRACEFUL_SHUTDOWN || true
# Unset => production memory ceiling and eviction water marks. Eviction cadence
# is not a thing this cell varies.
unset TOPGUN_MAX_RAM_MB || true
unset TOPGUN_EVICTION_HIGH_PCT || true
unset TOPGUN_EVICTION_LOW_PCT || true
unset TOPGUN_EVICTION_INTERVAL_MS || true
# Unset => the emitter is ARMED, the shipped default.
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
# 1. The pinned matrix. Byte-identical to spec362b-durable.sh's long4h cell,
#    except CRASH_INTERVAL, which is per-cell and 0 on both files' one
#    shipped cell anyway.
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
CRASH_INTERVAL="$CELL_CRASH_INTERVAL"
STEADY_INTERVAL=300     # ~1% of the run spent quiesced instead of ~10%
QUIESCE=3
MEM_SAMPLE_INTERVAL=5
WAL_FSYNC=batched
# The harness's live memory gate is NEUTRALIZED, not sharpened: this cell's
# verdict is read from committed artifacts by a separate mechanical readout,
# and at the shipped defaults the live gate fires on every run in this regime.
MEM_MIN_GROWTH_MB=1000000
MEM_THRESHOLD_MB_PER_HOUR=1000000
MEM_CEILING_MB=1000000
SERVER_PORT=47356       # fixed, distinct from spec362b-durable.sh's 47355 so
                         # the two runners' cells never collide on one host
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
# 4a. THE FREEZE GATE. This spec's readout is pre-registered before data, and
#     the runner and readout are not edited after that pre-registration. So
#     the commit at which the instrument this file samples is complete and
#     green is recorded here as a literal, exactly as the parent recorded its
#     own commit pin -- with one behaviour the parent never needed: while this
#     literal still names its own placeholder, the instrument does not exist
#     yet, and the run refuses immediately rather than attempting to diff
#     against a value that is not a revision.
# ---------------------------------------------------------------------------
SPEC365_CODE_FREEZE=9fdaaf5a141bf24ee3b7801cb6a16c861ab228cc
if [ "$SPEC365_CODE_FREEZE" = "PENDING_SPEC365_CODE_FREEZE" ]; then
  echo "FATAL: SPEC365_CODE_FREEZE still reads its placeholder value." >&2
  echo "       The prune-conjunct instrument this runner samples is not yet" >&2
  echo "       committed under a named freeze commit. Refusing to start." >&2
  exit 2
fi
SOAK_BIN_COMMIT="$(git -C "$REPO_ROOT" rev-parse HEAD 2>/dev/null || true)"
if [ -z "$SOAK_BIN_COMMIT" ]; then
  echo "FATAL: $REPO_ROOT is not a git checkout, so this run cannot be pinned" >&2
  echo "       to the commit that produced it. Refusing to start." >&2
  exit 1
fi
if ! FREEZE_RS_DIFF="$(git -C "$REPO_ROOT" diff --stat "$SPEC365_CODE_FREEZE"..HEAD -- '*.rs' 2>&1)"; then
  echo "FATAL: the .rs diff against the freeze commit ${SPEC365_CODE_FREEZE} could" >&2
  echo "       not be computed, so the freeze cannot be asserted:" >&2
  printf '%s\n' "$FREEZE_RS_DIFF" >&2
  echo "       Refusing to start." >&2
  exit 1
fi
if [ -n "$FREEZE_RS_DIFF" ]; then
  echo "FATAL: the .rs tree at HEAD differs from the freeze commit ${SPEC365_CODE_FREEZE}; this run would not be filed under it" >&2
  printf '%s\n' "$FREEZE_RS_DIFF" >&2
  exit 1
fi
FREEZE_DIFF_STATE="EMPTY (asserted before the build)"
RS_DIRTY="$(git -C "$REPO_ROOT" status --porcelain -- '*.rs' 2>/dev/null || true)"
if [ -n "$RS_DIRTY" ]; then
  echo "FATAL: the .rs working tree is DIRTY, so the binary this run would" >&2
  echo "       build is not the commit it would be filed under:" >&2
  printf '%s\n' "$RS_DIRTY" >&2
  echo "       Commit or stash the .rs changes and re-run." >&2
  exit 1
fi
RS_TREE_STATE="CLEAN (asserted before the build)"

if [ -n "${SPEC365_SOAK_BIN:-}" ]; then
  echo "WARNING: SPEC365_SOAK_BIN is set, so this runner did NOT build the bench" >&2
  echo "         binary from HEAD. The freeze gate is NOT discharged for this run" >&2
  echo "         and matrix.txt will say so." >&2
else
  if [ -z "${SDKROOT:-}" ] && [ -x /usr/bin/xcrun ]; then
    SDKROOT="$(/usr/bin/xcrun --sdk macosx --show-sdk-path 2>/dev/null || true)"
    [ -n "$SDKROOT" ] && export SDKROOT
  fi
  BUILD_TARGETS="--bench soak_harness"
  if [ "$PROVENANCE" != "yes" ]; then
    BUILD_TARGETS="--bin topgun-server $BUILD_TARGETS"
  fi
  echo "building from HEAD ${SOAK_BIN_COMMIT}: cargo build --release ${BUILD_TARGETS}"
  # shellcheck disable=SC2086
  if ! ( cd "$SERVER_ROOT" && cargo build --release $BUILD_TARGETS ); then
    echo "FATAL: the release build failed; nothing was run." >&2
    exit 1
  fi
  if [ -z "${CARGO_TARGET_DIR:-}" ] && [ -d "${REPO_ROOT}/target/release" ]; then
    TARGET_DIR="${REPO_ROOT}/target"
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
SERVER_BIN_SHA256="$(shasum -a 256 "$SERVER_BIN" 2>/dev/null | awk '{print $1}')"

SOAK_BIN="${SPEC365_SOAK_BIN:-}"
if [ -n "$SOAK_BIN" ]; then
  SOAK_BIN_COMMIT="<UNPROVEN: SPEC365_SOAK_BIN override, not built by this run>"
else
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
if [ "$BUILD_GAP" -gt 600 ]; then
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
    echo "       Each run needs its own fresh, empty dir." >&2
    exit 1
  fi
fi
mkdir -p "$DATA_DIR" "$META_DIR" "$OUT_DIR"
rm -f "$STOP_FILE" "$FAIL_FILE"

# Refuse to silently overwrite artifacts: a re-run that clobbers a recorded
# series destroys the only copy of a measurement.
for f in "$CSV_OUT" "$JSON_OUT" "$PROGRESS_OUT" "$MECH_OUT" "$MECH_RAW" "$DURABLE_OUT" "$MATRIX_OUT" "$CONSOLE_OUT"; do
  if [ -e "$f" ] && [ "${SPEC365_FORCE:-0}" != "1" ]; then
    echo "FATAL: artifact already exists: $f" >&2
    echo "       Move it aside, or re-run with SPEC365_FORCE=1 to overwrite." >&2
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
  echo "=== spec365 conjunct-readout run: cell ${CELL} ==="
  echo "  repo HEAD:      $(git -C "$REPO_ROOT" rev-parse HEAD 2>/dev/null || echo '<not a git checkout>')"
  echo "  dirty tree:     $(test -n "$(git -C "$REPO_ROOT" status --porcelain 2>/dev/null || true)" && echo yes || echo no)"
  echo "  host/OS:        $(uname -a)"
  echo "  lineage:        HEAD harness + HEAD server"
  echo "  data dir:       $DATA_DIR"
  echo "  csv:            $CSV_OUT"
  echo "  soak.json:      $JSON_OUT"
  echo "  mechanism.json: $MECH_OUT (harness writes $(basename "$MECH_RAW"); renamed after the run)"
  echo "  durable.json:   $DURABLE_OUT (raw basename: NOT renamed, by design)"
  echo "  progress.jsonl: $PROGRESS_OUT"
  echo "  console log:    $CONSOLE_OUT"
  echo "  matrix:         $MATRIX_OUT"
  echo "  readout:        $READOUT_OUT (written by spec365-readout.sh, not this file)"
  echo "  soak binary:    $SOAK_BIN"
  echo "  soak binary commit: ${SOAK_BIN_COMMIT}"
  echo "  .rs working tree:   ${RS_TREE_STATE}"
  echo "  code freeze:            ${SPEC365_CODE_FREEZE}"
  echo "  code freeze diff (.rs): ${FREEZE_DIFF_STATE}"
  echo "    built:        $(date -r "$SOAK_BIN" -u '+%Y-%m-%dT%H:%M:%SZ')"
  echo "  server binary:  $SERVER_BIN"
  echo "    built:        $(date -r "$SERVER_BIN" -u '+%Y-%m-%dT%H:%M:%SZ')"
  echo "    sha256:       ${SERVER_BIN_SHA256:-<unavailable>}"
  echo "  --- varied knobs; everything else is the spec362b-durable.sh long4h"
  echo "      literal ---"
  echo "  TOPGUN_EPOCH_WIDTH:  ${TOPGUN_EPOCH_WIDTH:-<unset: production default 1000>}"
  echo "  duration:            ${DURATION}s$( [ "$SMOKE" = "1" ] && echo '  <-- SMOKE OVERRIDE')"
  echo "  csv cadence:         ${SAMPLE_INTERVAL}s"
  echo "  extra harness flags: ${EXTRA_FLAGS:-<none>}"
  echo "  --durable-reading:   ARMED"
  echo "  --live-census-interval: ${CELL_LIVE_CENSUS}$( [ "$CELL_LIVE_CENSUS" = "0" ] && echo ' (DISARMED)' || echo ' (ARMED: the live-copy census)' )"
  echo "  --sampler-jitter-seed:  ${JITTER_SEED}"
  echo "  SOAK_SERVER_LOG:     ${SOAK_SERVER_LOG:-<unset>}"
  echo "  SOAK_SERVER_LOG_PASSTHROUGH: ${SOAK_SERVER_LOG_PASSTHROUGH:-<unset>}"
  echo "  TOPGUN_PRUNE_RECORD: ${TOPGUN_PRUNE_RECORD:-<unset>}"
  echo "  TOPGUN_LOG_FORMAT:   ${TOPGUN_LOG_FORMAT:-<unset: human fmt, which is what the origin parser reads>}"
  echo "  server port:         ${SERVER_PORT}"
  echo "  SOAK_SERVER_BINARY:  ${SOAK_SERVER_BINARY:-<unset: HEAD binary, fail-closed guard not on this path>}"
  echo "  --- pinned matrix (identical to spec362b-durable.sh's long4h cell;"
  echo "      crash-interval is per-cell and is 0 on the one cell this runner"
  echo "      ships) ---"
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
# own case table (always empty here), never operator input.
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

# The CSV header literal (41 columns; column 41, clean_mb, is the footprint
# Clean column, appended last so 1-40 keep their positions). The inherited six stay byte-identical
# in content and position; columns 7-10 are the per-process memory breakdown;
# columns 11-31 are the conjunct snapshot's counter and 20 gauges, in the
# order the record's fields are defined; columns 32-40 are the pre-existing
# prune _total/indexed_refs series this cell now also samples.
CSV_HEADER='elapsed_secs,rss_mb,wal_mb,redb_mb,disk_total_mb,tombstone_bytes,phys_footprint_mb,phys_footprint_peak_mb,reclaimable_mb,compressed_mb,conj_snapshots_total,conj_current_epoch,conj_ceiling,conj_durable_watermark,durable_watermark_lag,claims,claim_lag_p50,claim_lag_p99,claim_lag_max,ret_epochs_claim_only,ret_epochs_durability_only,ret_epochs_both,ret_epochs_neither,ret_refs_claim_only,ret_refs_durability_only,ret_refs_both,ret_refs_neither,ret_stamped_bytes,ret_epochs_unslotted,ret_refs_open_epoch,ret_stamped_bytes_open_epoch,indexed_refs,considered_total,dropped_total,matched_nothing_total,absent_total,bytes_freed_total,removed_refs_observed_total,removed_bytes_observed_total,stamped_bytes_total,clean_mb'

# The ordered metric-name list behind the one /metrics scrape below. Position
# 1 is the inherited tombstone-bytes gauge (CSV column 6); positions 2-31 are
# CSV columns 11-40 in that order. A name this list carries but the scrape
# body does not renders as an EMPTY field, never a zero.
PRUNE_METRIC_NAMES="topgun_ormap_tombstone_bytes topgun_or_prune_conjunct_snapshots_total topgun_or_prune_conjunct_current_epoch topgun_or_prune_conjunct_ceiling topgun_or_prune_conjunct_durable_watermark topgun_or_prune_conjunct_durable_watermark_lag topgun_or_prune_conjunct_claims topgun_or_prune_conjunct_claim_lag_p50 topgun_or_prune_conjunct_claim_lag_p99 topgun_or_prune_conjunct_claim_lag_max topgun_or_prune_conjunct_retained_epochs_claim_only topgun_or_prune_conjunct_retained_epochs_durability_only topgun_or_prune_conjunct_retained_epochs_both topgun_or_prune_conjunct_retained_epochs_neither topgun_or_prune_conjunct_retained_refs_claim_only topgun_or_prune_conjunct_retained_refs_durability_only topgun_or_prune_conjunct_retained_refs_both topgun_or_prune_conjunct_retained_refs_neither topgun_or_prune_conjunct_retained_stamped_bytes topgun_or_prune_conjunct_retained_epochs_unslotted topgun_or_prune_conjunct_retained_refs_open_epoch topgun_or_prune_conjunct_retained_stamped_bytes_open_epoch topgun_or_prune_indexed_refs topgun_or_prune_considered_total topgun_or_prune_dropped_total topgun_or_prune_matched_nothing_total topgun_or_prune_absent_total topgun_or_prune_bytes_freed_total topgun_or_prune_removed_refs_observed_total topgun_or_prune_removed_bytes_observed_total topgun_or_prune_stamped_bytes_total"

# One curl /metrics per row. Prints the 31 values above as one comma-joined
# string, in list order. A metric absent from the body -- including every
# name in this list, if the scrape itself fails -- becomes an empty field.
scrape_prune_metrics() {
  local body
  body="$(curl -fsS --max-time 5 "http://127.0.0.1:${SERVER_PORT}/metrics" 2>/dev/null)" || body=""
  printf '%s' "$body" | awk -v names="$PRUNE_METRIC_NAMES" '
    BEGIN { n = split(names, want, " ") }
    /^[[:space:]]*#/ { next }
    {
      name = $1
      b = index(name, "{")
      if (b > 0) name = substr(name, 1, b - 1)
      if (!(name in seen)) { val[name] = $2 + 0; seen[name] = 1 }
    }
    END {
      out = ""
      for (i = 1; i <= n; i++) {
        if (want[i] in seen) v = sprintf("%.0f", val[want[i]]); else v = ""
        out = (i == 1) ? v : out "," v
      }
      print out
    }
  '
}

# The per-process physical-memory breakdown, one `footprint --swapped` call
# per row, parsed into MB with 3 decimals. Its field positions were pinned
# against a real captured sample (spec365-footprint-sample.txt) before this
# runner was committed: the TOTAL row's columns are Dirty, Swapped, Clean,
# Reclaimable, Regions, Category (--swapped's own doc names the second column
# "swapped/compressed"), and the Auxiliary-data block below the table carries
# phys_footprint and phys_footprint_peak. The TOTAL row's Clean column is
# carried too: rss counts resident clean pages (mostly __TEXT) that
# phys_footprint does not, so rss can only be reconstructed with it. Any parse
# miss -- a vanished pid, a tool-output shape this awk does not recognise --
# yields five empty cells, never a fatal: the sampler must survive a process
# racing to exit under it.
footprint_row() {   # $1 = pid; prints "phys_footprint_mb,phys_footprint_peak_mb,reclaimable_mb,compressed_mb,clean_mb"
  local out
  out="$(/usr/bin/footprint --swapped -p "$1" 2>/dev/null)" || out=""
  printf '%s' "$out" | awk '
    function to_mb(v, unit,   n) {
      n = v + 0
      if (unit == "KB") return n / 1024
      if (unit == "MB") return n
      if (unit == "GB") return n * 1024
      return n / 1048576   # bytes, no unit suffix
    }
    /^[[:space:]]*phys_footprint:/ { pf = to_mb($2, $3); pf_ok = 1 }
    /^[[:space:]]*phys_footprint_peak:/ { pfp = to_mb($2, $3); pfp_ok = 1 }
    $NF == "TOTAL" && NF == 10 {
      compressed = to_mb($3, $4)
      clean = to_mb($5, $6)
      reclaimable = to_mb($7, $8)
      total_ok = 1
    }
    END {
      printf "%s,%s,%s,%s,%s\n", \
        (pf_ok  ? sprintf("%.3f", pf)  : ""), \
        (pfp_ok ? sprintf("%.3f", pfp) : ""), \
        (total_ok ? sprintf("%.3f", reclaimable) : ""), \
        (total_ok ? sprintf("%.3f", compressed)  : ""), \
        (total_ok ? sprintf("%.3f", clean)       : "")
    }
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

  # One /metrics scrape per row supplies the inherited tombstone-bytes gauge
  # (first field) and the 30 new metric columns (the remainder), from the
  # same response body.
  local prune_row tomb prune_rest
  prune_row="$(scrape_prune_metrics)"
  tomb="${prune_row%%,*}"
  prune_rest="${prune_row#*,}"
  case "$tomb" in
    ''|*[!0-9]*) tomb="" ;;
  esac

  local footprint_all footprint_cols clean_col
  footprint_all="$(footprint_row "$pid")"
  # Clean goes last so columns 1-40 keep the positions every readout rule cites.
  footprint_cols="${footprint_all%,*}"
  clean_col="${footprint_all##*,}"

  printf '%d,%s,%s,%s,%s,%s,%s,%s,%s\n' \
    "$elapsed" \
    "$(kib_to_mb "$rss_kib")" \
    "$(kib_to_mb "$wal_kib")" \
    "$(kib_to_mb "$redb_kib")" \
    "$(kib_to_mb "$total_kib")" \
    "$tomb" \
    "$footprint_cols" \
    "$prune_rest" \
    "$clean_col" \
    >> "$CSV_OUT"
}

printf '%s\n' "$CSV_HEADER" > "$CSV_OUT"

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
if [ "$HEADER" != "$CSV_HEADER" ]; then
  fail_instrument "CSV header is '$HEADER'"
fi
HEADER_PREFIX="$(printf '%s' "$HEADER" | cut -d, -f1-6)"
if [ "$HEADER_PREFIX" != 'elapsed_secs,rss_mb,wal_mb,redb_mb,disk_total_mb,tombstone_bytes' ]; then
  fail_instrument "CSV header's first six columns are not byte-identical to the parent's: '$HEADER_PREFIX'"
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
# 12. Convenience: the post-hoc fits. Unedited from the parent; it resolves
#     columns by header name, so the widened header does not affect it.
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
  echo "      The memory gate is neutralized by design for this run, while the"
  echo "      tombstone-byte gate, both blind-monitor clauses, convergence/recovery"
  echo "      and panic capture stay armed. Read 'finishedReason' in $JSON_OUT and"
  echo "      record the attribution, rather than reading the flag as a verdict."
fi

# ---------------------------------------------------------------------------
# 13. The mechanical readout, run once on this cell's own artifacts.
# ---------------------------------------------------------------------------
READOUT_SCRIPT="${SCRIPT_DIR}/spec365-readout.sh"
if [ -f "$READOUT_SCRIPT" ]; then
  echo
  echo "invoking readout: bash $READOUT_SCRIPT $BASE"
  bash "$READOUT_SCRIPT" "$BASE"
else
  echo "WARNING: $READOUT_SCRIPT not found; skipping the readout invocation." >&2
fi

exit "$HARNESS_RC"
