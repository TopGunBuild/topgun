#!/usr/bin/env bash
#
# Plateau-characterization runner for the OR delta-WAL emitter.
#
# Runs ONE 60-minute soak at the pinned characterization matrix and records the
# per-minute (elapsed, rss, wal, redb, disk) series beside the harness's own
# JSON artifacts. Run it twice -- once `on`, once `off` -- on separate fresh
# data dirs; the OFF run is the comparator, not a supplementary check.
#
#   ./spec349c2-plateau.sh on
#   ./spec349c2-plateau.sh off
#
# THE MATRIX IS EXECUTED, NOT TRANSCRIBED. Every rate-, shape- and
# duration-determining knob below is a literal in this file, so the record of
# what was run is this committed script rather than an operator's memory of a
# command line. Nothing here may be softened for a real run; the only
# sanctioned override is the smoke path (see SPEC349C2_SMOKE_DURATION), which
# is refused if it would write into the tracked evidence directory.
#
# Bash 3.2 (macOS system bash) compatible: no mapfile, no associative arrays,
# no ${x@Q}.
#
# Env overrides (all documented, all logged loudly when active):
#   SPEC349C2_DATA_DIR              data dir for this run (default: target/)
#   SPEC349C2_OUT_DIR               artifact dir     (default: this script's dir)
#   SPEC349C2_SOAK_BIN              prebuilt soak_harness bench binary
#   SPEC349C2_SMOKE_DURATION        SMOKE ONLY: override --duration (seconds)
#   SPEC349C2_SMOKE_SAMPLE_INTERVAL SMOKE ONLY: override the CSV cadence
#   SPEC349C2_FORCE=1               overwrite pre-existing artifacts
#
set -euo pipefail

# ---------------------------------------------------------------------------
# 0. Argument: the emitter state. Every output path is derived from it.
# ---------------------------------------------------------------------------
STATE="${1:-}"
case "$STATE" in
  on|off) ;;
  *)
    echo "usage: $(basename "$0") on|off" >&2
    echo "  on  -- TOPGUN_OR_DELTA_WAL left UNSET (the shipped default: armed)" >&2
    echo "  off -- TOPGUN_OR_DELTA_WAL=false (kill-switch: full snapshot framing)" >&2
    exit 2
    ;;
esac

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
EVIDENCE_DIR="$SCRIPT_DIR"
SERVER_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"     # packages/server-rust
REPO_ROOT="$(cd "$SERVER_ROOT/../.." && pwd)"

OUT_DIR="${SPEC349C2_OUT_DIR:-$EVIDENCE_DIR}"
BASE="spec349c2-emitter-${STATE}"
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

# ---------------------------------------------------------------------------
# 1. The pinned matrix. These are literals on purpose.
# ---------------------------------------------------------------------------
DURATION=3600           # 60 min
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
# live gate fires on every run in this regime (2.0 MB/h slope clause AND a
# 1800 MB peak-RSS ceiling clause, which are two independent routes to a red).
# All three knobs are existing flags -- no harness constant is edited.
MEM_MIN_GROWTH_MB=1000000
MEM_THRESHOLD_MB_PER_HOUR=1000000
MEM_CEILING_MB=1000000
SERVER_PORT=47349       # fixed, so the child server has a reliable handle
SAMPLE_INTERVAL=60      # CSV cadence, seconds

DATA_DIR="${SPEC349C2_DATA_DIR:-${REPO_ROOT}/target/spec349c2-${STATE}-data}"
META_DIR="${DATA_DIR}.meta"      # sibling: NEVER inside the measured data dir
CONSOLE_LOG="${META_DIR}/harness-console.log"
STOP_FILE="${META_DIR}/sampler.stop"
FAIL_FILE="${META_DIR}/sampler.fail"

# ---------------------------------------------------------------------------
# 2. Smoke override -- only ever changes the duration (and, with it, the CSV
#    cadence, which would otherwise yield 3 rows). The real runs must not be
#    able to take this path by accident, so the default is 3600, the override
#    is loud, and it is REFUSED if the artifacts would land in the tracked
#    evidence directory.
# ---------------------------------------------------------------------------
SMOKE=0
if [ -n "${SPEC349C2_SMOKE_DURATION:-}" ]; then
  SMOKE=1
  DURATION="$SPEC349C2_SMOKE_DURATION"
  if [ -n "${SPEC349C2_SMOKE_SAMPLE_INTERVAL:-}" ]; then
    SAMPLE_INTERVAL="$SPEC349C2_SMOKE_SAMPLE_INTERVAL"
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
    echo "Set SPEC349C2_OUT_DIR to a scratch directory." >&2
    exit 2
  fi
elif [ -n "${SPEC349C2_SMOKE_SAMPLE_INTERVAL:-}" ]; then
  echo "WARNING: SPEC349C2_SMOKE_SAMPLE_INTERVAL ignored outside smoke mode;" >&2
  echo "         the characterization CSV cadence is pinned at ${SAMPLE_INTERVAL}s." >&2
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
# ---------------------------------------------------------------------------
# Must be unset => the server applies the PRODUCTION default epoch width (1000).
# The harness records the effective width in soak.json; a stray export here
# would make that record report a non-production regime.
unset TOPGUN_EPOCH_WIDTH || true
# Unset => the harness supplies its own 100ms / 5000 write-behind cadence, which
# is the instrument-identity choice shared with every other soak run. An export
# here silently changes when bytes land in redb vs the WAL.
unset TOPGUN_WRITEBEHIND_FLUSH_INTERVAL_MS || true
unset TOPGUN_WRITEBEHIND_BATCH_SIZE || true
# Unset => the child is the topgun-server the harness was compiled beside,
# which is what makes the harness-side epoch-width derivation a valid proxy for
# the server's behaviour.
unset SOAK_SERVER_BINARY || true
# Unset => the child runs at RUST_LOG=warn, the pinned instrument-identity log
# level (server-side info logging is a measurable cost on this write path).
unset SOAK_SERVER_LOG || true
# Unset => teardown is SIGKILL, as every other soak run's is.
unset TOPGUN_SOAK_GRACEFUL_SHUTDOWN || true
# Unset => production memory ceiling and eviction water marks. Eviction cadence
# is one of the things these runs are measuring the effect of; overriding it
# would answer a question about a different system.
unset TOPGUN_MAX_RAM_MB || true
unset TOPGUN_EVICTION_HIGH_PCT || true
unset TOPGUN_EVICTION_LOW_PCT || true
unset TOPGUN_EVICTION_INTERVAL_MS || true

# NOTE: TOPGUN_WAL_FSYNC_POLICY is deliberately NOT managed here. The harness
# OVERWRITES it on the child unconditionally from --wal-fsync, so exporting it
# does nothing at all; the policy comes from the flag above and is recorded in
# soak.json as `walFsync`. It is reported below only so a reader is not misled
# by seeing it in their own environment.
if [ -n "${TOPGUN_WAL_FSYNC_POLICY:-}" ]; then
  echo "note: TOPGUN_WAL_FSYNC_POLICY='${TOPGUN_WAL_FSYNC_POLICY}' is inherited but IGNORED"
  echo "      (the harness overwrites it on the child from --wal-fsync=${WAL_FSYNC})"
fi

# The one input that differs between the two runs.
if [ "$STATE" = "on" ]; then
  unset TOPGUN_OR_DELTA_WAL || true
else
  export TOPGUN_OR_DELTA_WAL=false
fi

# ---------------------------------------------------------------------------
# 4. Pre-flight. Fail closed BEFORE the clock starts -- everything below would
#    otherwise only be discovered at minute 59.
# ---------------------------------------------------------------------------
TARGET_DIR="${CARGO_TARGET_DIR:-}"
if [ -z "$TARGET_DIR" ]; then
  if [ -d "${REPO_ROOT}/target/release" ]; then
    TARGET_DIR="${REPO_ROOT}/target"
  else
    TARGET_DIR="${SERVER_ROOT}/target"
  fi
fi

SERVER_BIN="${TARGET_DIR}/release/topgun-server"
if [ ! -x "$SERVER_BIN" ]; then
  echo "FATAL: release server binary not found at $SERVER_BIN" >&2
  echo "  build: (cd $SERVER_ROOT && SDKROOT=\$(xcrun --sdk macosx --show-sdk-path) \\" >&2
  echo "          cargo build --release --bin topgun-server --bench soak_harness)" >&2
  exit 1
fi

SOAK_BIN="${SPEC349C2_SOAK_BIN:-}"
if [ -z "$SOAK_BIN" ]; then
  SOAK_BIN="$(ls -t "${TARGET_DIR}"/release/deps/soak_harness-* 2>/dev/null \
              | grep -vE '\.(d|o|rcgu)' | head -1 || true)"
fi
if [ -z "$SOAK_BIN" ] || [ ! -x "$SOAK_BIN" ]; then
  echo "FATAL: could not locate a built soak_harness bench binary under" >&2
  echo "       ${TARGET_DIR}/release/deps/" >&2
  exit 1
fi

# A stale bench binary measures a binary nobody asked about, and the newest
# soak_harness-* in deps/ is only the right one if it came from the same build
# as the server. Surface both mtimes, and flag a gap large enough that they
# cannot have (a link step apart) come from one build.
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

# The census is only sound over a fresh corpus: an OFF run over the ON run's
# directory would find that run's delta frames still retained and report a
# false ARMED.
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
# series destroys the only copy of a 60-minute measurement.
for f in "$CSV_OUT" "$JSON_OUT" "$PROGRESS_OUT" "$MECH_OUT" "$MECH_RAW" "$MATRIX_OUT"; do
  if [ -e "$f" ] && [ "${SPEC349C2_FORCE:-0}" != "1" ]; then
    echo "FATAL: artifact already exists: $f" >&2
    echo "       Move it aside, or re-run with SPEC349C2_FORCE=1 to overwrite." >&2
    exit 1
  fi
done
for f in "$CSV_OUT" "$JSON_OUT" "$PROGRESS_OUT" "$MATRIX_OUT"; do
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
#    harness spawns, NOT the harness itself. The run pins --server-port, so the
#    listener on that port is the handle. A `ps` against a wrong or absent PID
#    yields an empty column -- a blind series -- so anything other than exactly
#    one match is fatal.
# ---------------------------------------------------------------------------
resolve_server_pid() {   # prints the pid, or nothing; never fails the shell
  local pids
  pids="$(lsof -nP -iTCP:"$SERVER_PORT" -sTCP:LISTEN -t 2>/dev/null | sort -u || true)"
  if [ -z "$pids" ]; then
    # Fallback for hosts without a usable lsof: the child's argv is
    # "<...>/topgun-server --port <port>".
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
# 6. Report the effective matrix before starting.
#
#    This block is `tee`d into a COMMITTED artifact, not merely printed. It used
#    to go to this script's own stdout only -- $CONSOLE_LOG captures the
#    redirected harness child, not the runner -- which left one field (the
#    dirty-tree flag) attested by the run record rather than observable in any
#    artifact, and left the whole block living in a scratch file under target/.
#    The matrix that describes a run has to outlive the run's working directory.
# ---------------------------------------------------------------------------
{
  echo
  echo "=== spec349c2 plateau run: emitter ${STATE} ==="
  echo "  repo HEAD:      $(git -C "$REPO_ROOT" rev-parse HEAD 2>/dev/null || echo '<not a git checkout>')"
  echo "  dirty tree:     $(test -n "$(git -C "$REPO_ROOT" status --porcelain 2>/dev/null || true)" && echo yes || echo no)"
  echo "  host/OS:        $(uname -a)"
  echo "  data dir:       $DATA_DIR"
  echo "  csv:            $CSV_OUT"
  echo "  soak.json:      $JSON_OUT"
  echo "  mechanism.json: $MECH_OUT (harness writes $(basename "$MECH_RAW"); renamed after the run)"
  echo "  progress.jsonl: $PROGRESS_OUT"
  echo "  matrix:         $MATRIX_OUT"
  echo "  console log:    $CONSOLE_LOG"
  echo "  duration:       ${DURATION}s$( [ "$SMOKE" = "1" ] && echo '  <-- SMOKE OVERRIDE')"
  echo "  csv cadence:    ${SAMPLE_INTERVAL}s"
  echo "  TOPGUN_OR_DELTA_WAL: ${TOPGUN_OR_DELTA_WAL:-<unset: armed by default>}"
  echo "  TOPGUN_EPOCH_WIDTH:  ${TOPGUN_EPOCH_WIDTH:-<unset: production default 1000>}"
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
# 8. Wait for server-ready. This is the CSV's time origin: the server binds its
#    listener only after WAL recovery completes, which is the same event the
#    harness waits for before it starts its own clock. (The harness's own
#    sampler origin is LATER still -- it starts after the churn clients spawn --
#    so the CSV carries at most one extra leading sample, never fewer.)
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
    exit 1
  fi
  sleep 1
  waited=$((waited + 1))
done
if [ -z "$SERVER_PID" ]; then
  echo "FATAL: no single listener on port ${SERVER_PORT} after ${READY_TIMEOUT}s" >&2
  tail -40 "$CONSOLE_LOG" >&2 || true
  exit 1
fi
T0="$(date +%s)"
echo "server ready: pid $SERVER_PID (t0 = server-ready)"

# ---------------------------------------------------------------------------
# 9. The per-minute sampler. Same two shell commands the harness's own samplers
#    shell, so this is the same measurement and not a second opinion:
#      rss_mb        ps -o rss= -p <server pid>          / 1024
#      wal_mb        du -sk <data-dir>/wal               / 1024
#      redb_mb       du -sk <data-dir>/topgun.redb       / 1024
#      disk_total_mb du -sk <data-dir>                   / 1024
# ---------------------------------------------------------------------------
kib_to_mb() { awk -v k="${1:-0}" 'BEGIN { printf "%.3f", k / 1024 }'; }

# Prints the path's size in KiB; prints the literal ABSENT when the path does
# not exist yet; prints NOTHING when `du` failed on a path that DOES exist.
# Those three are kept apart on purpose: "not created yet" is a real 0, while
# "du failed" recorded as 0 would punch a spurious hole in a rising series and
# drag the fit toward zero -- a silent measurement error, which is the one class
# of failure these runs cannot tolerate.
# Scrape the server's own decrementable OR-Map tombstone-bytes gauge off the
# same /metrics endpoint the harness scrapes. Recorded as a CSV column so the
# series that decides the tombstone-byte HARD gate survives the run in a
# committed artifact -- the verdict used to be reconstructable only from the
# harness's stdout, which lives in a scratch file under target/ and is gone by
# the time anyone re-reads a failed run.
#
# A failed scrape yields ABSENT rather than 0: a zero is a real measurement
# ("no tombstone bytes") and must never be forged from a transport error. The
# post-run column check below is what turns an all-ABSENT column into a
# declared instrument defect.
TOMBSTONE_METRIC='topgun_ormap_tombstone_bytes'
tombstone_bytes() {
  local body
  body="$(curl -fsS --max-time 5 "http://127.0.0.1:${SERVER_PORT}/metrics" 2>/dev/null)" || {
    printf 'ABSENT'
    return 0
  }
  # Take the first non-comment sample line for the metric, bare or labelled.
  # The value is truncated to a whole count with int(), NOT by stripping a
  # trailing ".0": the gauge is logically u64 bytes, but Prometheus is free to
  # render it as a float, and a regex that only handles ".0" would turn any
  # other fractional rendering into a non-integer that the caller then discards
  # as an unreadable cell. int() matches what the harness's own Rust parser
  # does (parse u64, else parse f64 and truncate), so the two instruments read
  # the same wire text the same way.
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
    # WAL segments rotate and unlink underneath the walk; retry once before
    # calling it a fault.
    sleep 1
    v="$(du -sk "$1" 2>/dev/null | awk 'NR == 1 { print $1 }')"
  fi
  printf '%s' "$v"
}

sampler_fatal() {   # $1 = reason
  echo "$1" > "$FAIL_FILE"
  echo "SAMPLER FATAL: $1" >&2
  # A blind series is worthless, and so is the rest of the run: stop now rather
  # than spend the remaining wall clock producing a column nobody can use.
  kill "$HARNESS_PID" 2>/dev/null || true
}

# The harness SIGKILLs the server at teardown and then STAYS ALIVE for a while:
# it scans the redb tombstone corpus, builds the report and walks every retained
# WAL segment for the frame census before it writes soak.json and
# mechanism.json. So "the server is gone while the harness still runs" is the
# NORMAL end of a run, and a sampler that treated it as a blind-series fault
# would kill the harness in the middle of writing the run's two most important
# artifacts -- at minute 60 of 60.
run_is_over() {   # $1 = elapsed seconds since server-ready
  if ! kill -0 "$HARNESS_PID" 2>/dev/null; then
    return 0
  fi
  # This clock starts at server-ready, which is EARLIER than the harness's own
  # duration clock (it starts after the churn clients spawn), so reaching
  # DURATION here always means the harness is at or past its own end phase.
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

  # The data dir itself is created before the server starts, so it is never
  # legitimately ABSENT.
  local total_kib wal_kib redb_kib
  total_kib="$(du_kib "$DATA_DIR")"
  case "$total_kib" in
    ''|ABSENT)
      sampler_fatal "du gave no size for data dir ${DATA_DIR} at elapsed=${elapsed}s"
      return 1
      ;;
  esac
  # The WAL dir and the redb file are created by the SERVER, and redb in
  # particular may not exist until its first write, so a genuinely absent path
  # early in the run is recorded as 0. A column that stays 0 for the WHOLE run
  # is caught by the post-run validation below -- that is where a wrong path
  # actually shows up.
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

  # Last guard before the row is written: every field must be a plain integer
  # count. Anything else means one of the two shell commands answered something
  # this script did not anticipate, and a malformed row would either break the
  # fit or, worse, parse as a number that was never measured.
  local v
  for v in "$rss_kib" "$wal_kib" "$redb_kib" "$total_kib"; do
    case "$v" in
      ''|*[!0-9]*)
        sampler_fatal "non-integer sample '${v}' at elapsed=${elapsed}s"
        return 1
        ;;
    esac
  done

  # The gauge column is held to a weaker rule than the four above: a scrape
  # that fails is written as an empty cell, NOT as a fatal. A transient /metrics
  # miss must not abort a 72-hour run over a diagnostic column, and an empty
  # cell is honestly distinguishable from a measured zero. Systematic failure
  # is caught after the run, where an all-empty column is an instrument defect.
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
# 11. Post-run: normalize the mechanism report's name, then validate the series
#     the run just produced. A defect found here is a defect found; a defect
#     not looked for is a 60-minute run thrown away.
# ---------------------------------------------------------------------------
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

# Column-by-column emptiness / all-zero check. An empty column is the blind
# sampler; an all-zero wal or disk column means the path being measured is not
# the path the server is writing.
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

# The gauge column is checked for POPULATION, not for non-zero: a genuinely
# bounded tombstone corpus may legitimately read 0 for a whole run, and failing
# on that would be an instrument that refuses to record the passing case. What
# is never legitimate is a column with no readings at all -- that is a blind
# diagnostic, and a blind diagnostic on the series that decides the hard gate
# is exactly the artifact-mortality this column exists to end.
#
# Deliberately a population check and NOT a missing-sample ratio: this column is
# CHARACTERIZATION, not the gate's input. The verdict is computed by the
# harness's own in-process sampler and lands in soak.json's `tombstones` object;
# a degraded scrape here therefore cannot move a verdict, only leave the
# committed series thinner. Holes are visible without a threshold -- `empty=` is
# printed below, and the fit reports `skipped_empty=` beside every slope.
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

for f in "$JSON_OUT" "$MECH_OUT"; do
  if [ ! -s "$f" ]; then
    fail_instrument "missing or empty artifact: $f"
  fi
done

# progress.jsonl gets ONE line per checkpoint, not per sample, so a run shorter
# than its own steady interval legitimately produces no file at all. Requiring
# it unconditionally would red the smoke path for a reason that cannot occur on
# a characterization run (3600s against a 300s steady interval => ~11 lines),
# so the requirement is stated against the condition that actually makes the
# file mandatory.
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

# The two fields that make an after-the-fact check of the matrix possible at
# all: both are emitted by the server only at info level, against the harness's
# pinned RUST_LOG=warn, so soak.json is the only place they are observable.
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
# 12. Convenience: the post-hoc fits, so the operator sees the numbers this run
#     produced without a second command. The recorded verdict still comes from
#     the committed CSV via the fit script -- this is the same computation, run
#     early.
# ---------------------------------------------------------------------------
FIT="${SCRIPT_DIR}/spec349c2-fit.awk"
if [ -f "$FIT" ] && [ "$ROWS" -ge 4 ]; then
  echo
  echo "post-hoc OLS fits (see the SE caveat in $(basename "$FIT")):"
  for w in full last_half; do
    for c in rss_mb wal_mb redb_mb disk_total_mb; do
      awk -v col="$c" -v window="$w" -f "$FIT" "$CSV_OUT" 2>&1 | sed 's/^/  /'
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
