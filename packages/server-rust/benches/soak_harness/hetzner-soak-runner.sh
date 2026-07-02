#!/usr/bin/env bash
#
# Hetzner 72h soak runner (G4b / TODO-484).
#
# Runs ON a Hetzner *scratch* instance (NOT the live demo-VPS), launching the
# soak harness detached via nohup so it survives the SSH session, with a JSON
# progress stream you can tail remotely and a final JSON report.
#
# Usage (on the box, from the repo root):
#   packages/server-rust/benches/soak_harness/hetzner-soak-runner.sh
#
# Override any knob via env, e.g.:
#   DURATION=259200 CRASH_INTERVAL=300 ./hetzner-soak-runner.sh   # 72h
#
# Monitoring from your laptop:
#   ssh root@<box> 'tail -f /var/soak/run-*/progress.jsonl'
#   ssh root@<box> 'cat /var/soak/run-*/report.json | jq .passed'
#
set -euo pipefail

# --- Tunables (env-overridable) ---------------------------------------------
DURATION="${DURATION:-259200}"            # 72h
CRASH_INTERVAL="${CRASH_INTERVAL:-300}"   # kill -9 + restart every 5 min
STEADY_INTERVAL="${STEADY_INTERVAL:-120}" # steady convergence check every 2 min
CHURN_CLIENTS="${CHURN_CLIENTS:-32}"
KEYSPACE="${KEYSPACE:-500}"
QUIESCE="${QUIESCE:-4}"
# per_op fdatasyncs each WAL frame before the ingress write acks, so acked == durable
# on an unclean kill (perop/per-op are accepted aliases; per_op is canonical).
WAL_FSYNC="${WAL_FSYNC:-per_op}"
# Set NO_PRE_KILL_DRAIN=1 to kill -9 WITHOUT first flushing the write-behind buffer,
# so recovery must rebuild every acked write from the WAL alone — the honest
# acked == durable assertion (does not depend on a pre-kill drain).
NO_PRE_KILL_DRAIN="${NO_PRE_KILL_DRAIN:-0}"
# The no-drain validator must run the PRODUCTION write-behind flush cadence
# (1000ms): the harness's default fast flush (100ms) would drain the buffer to
# redb inside the brief pre-snapshot ack-settle and mask the WAL durability path,
# making acked == durable trivially true for the wrong reason. Pin production
# cadence here so the WAL is genuinely on the critical recovery path.
if [ "${NO_PRE_KILL_DRAIN}" = "1" ]; then
  export TOPGUN_WRITEBEHIND_FLUSH_INTERVAL_MS="${TOPGUN_WRITEBEHIND_FLUSH_INTERVAL_MS:-1000}"
fi
# Calibrated slope ceiling for the 72h run. The OR tombstone leak the soak drives
# is ~3-5 MB/h; the former 25 MB/h sat above it and false-GREENed a real leak.
# 2 MB/h sits below the leak band and above a genuine in-place plateau (~0 MB/h);
# the harness min-growth guard (80 MB, in-code default) keeps short runs green.
# Mirrors monitor::DEFAULT_MEM_THRESHOLD_MB_PER_HOUR.
MEM_THRESHOLD="${MEM_THRESHOLD:-2}"       # MB/hour slope ceiling over a 72h run
MEM_CEILING="${MEM_CEILING:-2048}"
OUT_ROOT="${OUT_ROOT:-/var/soak}"

# --- Sanity: refuse to run on the live demo box -----------------------------
# The live demo VPS serves demo.topgun.build; a crash-looping soak there would
# take it down. Set ALLOW_DEMO_BOX=1 only if you are CERTAIN this is a scratch box.
if [[ "${ALLOW_DEMO_BOX:-0}" != "1" ]]; then
  if hostname -f 2>/dev/null | grep -qiE 'demo|topgun\.build' \
     || curl -fsS --max-time 2 http://127.0.0.1:8080/ >/dev/null 2>&1; then
    echo "REFUSING: this looks like a live/demo box (a soak crash-loop would disrupt it)." >&2
    echo "Run on a dedicated scratch instance, or set ALLOW_DEMO_BOX=1 if you are sure." >&2
    exit 1
  fi
fi

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
cd "$REPO_ROOT"
echo "repo root: $REPO_ROOT"

STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
RUN_DIR="${OUT_ROOT}/run-${STAMP}"
mkdir -p "$RUN_DIR"
DATA_DIR="${RUN_DIR}/data"
mkdir -p "$DATA_DIR"

# --- Build release bin + bench ----------------------------------------------
echo "building release server binary + soak bench ..."
cargo build --release --bin topgun-server --bench soak_harness 2>&1 | tail -3

SOAK_BIN="$(ls -t target/release/deps/soak_harness-* 2>/dev/null | grep -vE '\.(d|o|rcgu)' | head -1)"
if [[ -z "${SOAK_BIN}" || ! -x "${SOAK_BIN}" ]]; then
  echo "FATAL: could not locate built soak_harness binary" >&2
  exit 1
fi
echo "soak binary: ${SOAK_BIN}"

# --- Pre-flight: prove the harness can fail before the long run -------------
echo "pre-flight negative controls (must report RED / exit non-zero) ..."
if "${SOAK_BIN}" --inject-panic; then
  echo "FATAL: inject-panic did not go RED — aborting (harness cannot detect panics)" >&2
  exit 1
fi
if "${SOAK_BIN}" --inject-divergence; then
  echo "FATAL: inject-divergence did not go RED — aborting (harness cannot detect divergence)" >&2
  exit 1
fi
echo "negative controls OK (both went RED as required)."

# --- Launch the soak detached ------------------------------------------------
LOG="${RUN_DIR}/soak.log"
PROGRESS="${RUN_DIR}/progress.jsonl"
REPORT="${RUN_DIR}/report.json"

echo "launching ${DURATION}s soak (crash every ${CRASH_INTERVAL}s) ..."
echo "  run dir:  ${RUN_DIR}"
echo "  log:      ${LOG}"
echo "  progress: ${PROGRESS}"
echo "  report:   ${REPORT}"

DRAIN_FLAG=()
if [ "${NO_PRE_KILL_DRAIN}" = "1" ]; then
  DRAIN_FLAG=(--no-pre-kill-drain)
fi

nohup "${SOAK_BIN}" \
  --duration "${DURATION}" \
  --crash-interval "${CRASH_INTERVAL}" \
  --steady-interval "${STEADY_INTERVAL}" \
  --churn-clients "${CHURN_CLIENTS}" \
  --keyspace "${KEYSPACE}" \
  --quiesce "${QUIESCE}" \
  --wal-fsync "${WAL_FSYNC}" \
  "${DRAIN_FLAG[@]+"${DRAIN_FLAG[@]}"}" \
  --mem-threshold-mb-per-hour "${MEM_THRESHOLD}" \
  --mem-ceiling-mb "${MEM_CEILING}" \
  --data-dir "${DATA_DIR}" \
  --json-output "${REPORT}" \
  --progress-output "${PROGRESS}" \
  >"${LOG}" 2>&1 &

SOAK_PID=$!
echo "${SOAK_PID}" > "${RUN_DIR}/soak.pid"
echo
echo "soak running as PID ${SOAK_PID}."
echo "monitor:   tail -f ${PROGRESS}"
echo "live log:  tail -f ${LOG}"
echo "result:    jq .passed ${REPORT}   (written when the run ends)"
echo "stop:      kill ${SOAK_PID}"
