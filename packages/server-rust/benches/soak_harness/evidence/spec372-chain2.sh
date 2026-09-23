#!/usr/bin/env bash
#
# spec372 Stage-2 chain (the deciding cells): one detached launch, run only
# after the conductor's go at STOP 3a, in this order:
#
#   1. record the chain start epoch; read S1_SURVIVORS and S1_RANK from the
#      committed spec372.stage1.txt and refuse to start unless its STOP=FALSE
#      and at least one arm survived (the NONE path never launches this chain)
#   2. reclaim this carve's target dirs -- chain 1's builds -- (guarded_rm),
#      THEN assert >= 30 GB free
#   3. build SYS, H and each SURVIVING arm into fresh, chain-owned target dirs;
#      assert recompiled=yes, mtime >= chain start and the flavour markers;
#      write spec372-builds2.txt
#   4. run s2 -> j2 (iff JE survived) -> m2 (iff MI survived) -> a2j (900 s,
#      journal OFF, on S1_RANK's arm; dropped first when the smoke measured the
#      two build lines above 25 min -- A2J_DROPPED_FOR_BUDGET below), recording
#      the host state before each cell
#   5. spec372-predicates.sh for each cell that ran
#   6. spec372-k.awk stage2 -> spec372.k-stage2.txt
#   7. spec372-decide.awk stage2 -> spec372.decision.txt
#   8. print CHAIN_RC=<rc> as the log's LAST line
#
# SPEC372_SMOKE=1 runs the same steps into SPEC365_OUT_DIR (a scratch dir that
# already holds the chain-1 smoke's spec372.stage1.txt), with the 4 h cells at
# 300 s (cadence 20, census 10, so the TREND fit has >= 12 last-half points)
# and a2j at 120 s. SPEC372_SMOKE_SURVIVORS may force the survivor set so the
# smoke exercises every cell shape; it is refused outside smoke mode.
set -uo pipefail
# Every number this chain or its programs parse must use a '.' decimal point;
# on a comma-decimal host locale awk would otherwise read 144.110 as 144.
export LC_ALL=C

# Frozen at M from the smoke: the smoke measured the two build lines (Rust
# builds + jemalloc's C build) within 25 min, so a2j stays in the chain.
A2J_DROPPED_FOR_BUDGET=no

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
SERVER_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd -P)"          # packages/server-rust
REPO_ROOT="$(cd "$SERVER_ROOT/../.." && pwd -P)"
SMOKE="${SPEC372_SMOKE:-0}"

if [ "$SMOKE" = "1" ]; then
  OUT="${SPEC365_OUT_DIR:-}"
  if [ -z "$OUT" ]; then echo "FATAL: smoke needs SPEC365_OUT_DIR (a scratch dir)" >&2; exit 2; fi
  mkdir -p "$OUT"; OUT="$(cd "$OUT" && pwd -P)"
  if [ "$OUT" = "$SCRIPT_DIR" ]; then echo "FATAL: smoke must not write into the evidence dir" >&2; exit 2; fi
else
  OUT="$SCRIPT_DIR"
  if [ -n "${SPEC372_SMOKE_SURVIVORS:-}" ]; then echo "FATAL: SPEC372_SMOKE_SURVIVORS is a smoke-only knob" >&2; exit 2; fi
fi
# One spec372 program at a time: every one of them builds into, or reclaims,
# this carve's target dirs and needs the host to itself, so an overlap could
# delete a running cell's binary or contaminate a measurement.
LOCK="${REPO_ROOT}/target/spec372.lock"
mkdir -p "${REPO_ROOT}/target"
if ! mkdir "$LOCK" 2>/dev/null; then
  echo "FATAL: another spec372 program holds ${LOCK} ($(cat "$LOCK/owner" 2>/dev/null || echo unknown))" >&2; exit 2
fi
echo "pid=$$ program=$(basename "$0") since=$(date -u +%Y-%m-%dT%H:%M:%SZ)" > "$LOCK/owner"
trap 'rm -rf "$LOCK"' EXIT
LOG="$OUT/spec372-chain2.log"
: > "$LOG"
say() { echo "$*" | tee -a "$LOG"; }
finish() { say "chain end: $(date -u +%Y-%m-%dT%H:%M:%SZ)"; say "CHAIN_RC=$1"; exit "$1"; }

# ----------------------------------------------------------------- 1. start
CHAIN_START_EPOCH="$(date +%s)"
say "chain start: $(date -u +%Y-%m-%dT%H:%M:%SZ) epoch=${CHAIN_START_EPOCH} smoke=${SMOKE} HEAD=$(git -C "$REPO_ROOT" rev-parse HEAD)"
STAGE1="$OUT/spec372.stage1.txt"
[ -s "$STAGE1" ] || { say "FATAL: no ${STAGE1}"; finish 1; }
s1() { awk -v k="$1" 'index($0, k "=") == 1 { print substr($0, length(k) + 2); exit }' "$STAGE1"; }
STOP1="$(s1 STOP)"; SURV="$(s1 S1_SURVIVORS)"; RANK="$(s1 S1_RANK)"
if [ "$SMOKE" = "1" ] && [ -n "${SPEC372_SMOKE_SURVIVORS:-}" ]; then
  say "SMOKE override: S1_SURVIVORS ${SURV} -> ${SPEC372_SMOKE_SURVIVORS}"
  SURV="$SPEC372_SMOKE_SURVIVORS"
  case "$RANK" in JE|MI) ;; *) RANK="${SURV%%+*}" ;; esac
  # The programs read the survivor set from the Stage-1 file, so the override
  # must reach them too, or the stage-2 path would never see the forced arms.
  # Same basename in a subdir: the programs recognise their inputs by name.
  mkdir -p "$OUT/smoke-override"
  awk -v sv="$SURV" -v rk="$RANK" 'index($0, "S1_SURVIVORS=") == 1 { print "S1_SURVIVORS=" sv; next }
       index($0, "S1_RANK=") == 1 { print "S1_RANK=" rk; next } { print }' "$STAGE1" > "$OUT/smoke-override/spec372.stage1.txt"
  STAGE1="$OUT/smoke-override/spec372.stage1.txt"
fi
say "stage1: STOP=${STOP1} S1_SURVIVORS=${SURV} S1_RANK=${RANK}"
[ "$STOP1" = "FALSE" ] || { say "FATAL: spec372.stage1.txt reads STOP=${STOP1}"; finish 1; }
case "$SURV" in JE|MI|JE+MI) ;; *) say "FATAL: S1_SURVIVORS=${SURV}: chain 2 runs only over a survivor set"; finish 1 ;; esac

if [ -z "${SDKROOT:-}" ] && [ -x /usr/bin/xcrun ]; then
  SDKROOT="$(/usr/bin/xcrun --sdk macosx --show-sdk-path 2>/dev/null || true)"
  [ -n "$SDKROOT" ] && export SDKROOT
fi

# ----------------------------------------------------------------- 2. reclaim, then disk
T_ROOT="${REPO_ROOT}/target"
guarded_rm() {   # $1 = candidate target dir; removes it only if it is one of this carve's flavour dirs
  local cand parent resolved
  cand="$1"
  [ -n "$cand" ] || { say "FATAL: guarded_rm got an empty path"; finish 1; }
  parent="$(dirname "$cand")"
  mkdir -p "$parent"
  resolved="$(cd "$parent" && pwd -P)/$(basename "$cand")"
  case "$resolved" in
    "${REPO_ROOT}/target/spec372-sys"|\
    "${REPO_ROOT}/target/spec372-je"|\
    "${REPO_ROOT}/target/spec372-mi"|\
    "${REPO_ROOT}/target/spec372-ca"|\
    "${REPO_ROOT}/target/spec372-h") rm -rf "$resolved" ;;
    *) say "FATAL: refusing to remove '$resolved': not a chain-owned target dir"; finish 1 ;;
  esac
}
for fl in sys je mi ca h; do guarded_rm "${T_ROOT}/spec372-${fl}"; done
# Asserted AFTER the reclaim, so the space chain 1's builds release counts.
DISK_FREE_GB="$(df -g "$T_ROOT" | awk 'NR == 2 { print $4 }')"
say "DISK_FREE_AT_START=${DISK_FREE_GB}"
case "$DISK_FREE_GB" in ''|*[!0-9]*) say "FATAL: free space unreadable"; finish 1 ;; esac
[ "$DISK_FREE_GB" -ge 30 ] || { say "FATAL: ${DISK_FREE_GB} GB free < 30 GB"; finish 1; }

# ----------------------------------------------------------------- 3. builds
build() {   # $1 = flavour, $2 = target dir, $3.. = cargo args
  local fl="$1" td="$2" s rc; shift 2
  say "build ${fl}: CARGO_TARGET_DIR=${td} cargo build $*"
  s="$(date +%s)"
  ( cd "$SERVER_ROOT" && CARGO_TARGET_DIR="$td" cargo build --timings "$@" ) > "$OUT/spec372-build2-${fl}.log" 2>&1
  rc=$?
  say "build ${fl}: rc=${rc} wall_s=$(( $(date +%s) - s ))"
  tail -3 "$OUT/spec372-build2-${fl}.log" >> "$LOG"
  [ "$rc" -eq 0 ] || { say "FATAL: build ${fl} failed rc=${rc}"; finish 1; }
  if grep -q 'Compiling topgun-server v' "$OUT/spec372-build2-${fl}.log"; then echo yes; else echo no; fi > "$OUT/.recompiled-${fl}"
}
FLAVOURS="SYS"
build SYS "${T_ROOT}/spec372-sys" --release --bin topgun-server
case "$SURV" in JE|JE+MI) FLAVOURS="$FLAVOURS JE"; build JE "${T_ROOT}/spec372-je" --release --features alloc-jemalloc --bin topgun-server ;; esac
case "$SURV" in MI|JE+MI) FLAVOURS="$FLAVOURS MI"; build MI "${T_ROOT}/spec372-mi" --release --features alloc-mimalloc --bin topgun-server ;; esac
FLAVOURS="$FLAVOURS H"
build H "${T_ROOT}/spec372-h" --release --bench soak_harness

BIN_SYS="${T_ROOT}/spec372-sys/release/topgun-server"
BIN_JE="${T_ROOT}/spec372-je/release/topgun-server"
BIN_MI="${T_ROOT}/spec372-mi/release/topgun-server"
BIN_H="$(ls -t "${T_ROOT}"/spec372-h/release/deps/soak_harness-* 2>/dev/null | grep -vE '\.(d|o|rcgu)' | head -1 || true)"

# The MI literal was read off the first MI build (the bundled mimalloc's own
# message prefix); SYS is identified by the absence of every other literal.
MI_LIT='mimalloc: warning: '
hits() { strings "$1" | grep -cF "$2" || true; }
BUILDS="$OUT/spec372-builds2.txt"
: > "$BUILDS"
for fl in $FLAVOURS; do
  case "$fl" in SYS) b="$BIN_SYS" ;; JE) b="$BIN_JE" ;; MI) b="$BIN_MI" ;; H) b="$BIN_H" ;; esac
  [ -n "$b" ] && [ -x "$b" ] || { say "FATAL: flavour ${fl} binary missing: '${b}'"; finish 1; }
  rec="$(cat "$OUT/.recompiled-${fl}")"; rm -f "$OUT/.recompiled-${fl}"
  mt="$(date -r "$b" '+%s')"
  p="$(hits "$b" 'alloc_probe elapsed_s=')"; d="$(hits "$b" 'DHAT_OUT')"
  j="$(hits "$b" 'je_probe elapsed_s=')"; m="$(hits "$b" "$MI_LIT")"
  case "$fl" in
    SYS) [ "$p" -eq 0 ] && [ "$d" -eq 0 ] && [ "$j" -eq 0 ] && [ "$m" -eq 0 ] ;;
    JE)  [ "$j" -gt 0 ] && [ "$p" -eq 0 ] && [ "$d" -eq 0 ] && [ "$m" -eq 0 ] ;;
    MI)  [ "$m" -gt 0 ] && [ "$p" -eq 0 ] && [ "$d" -eq 0 ] && [ "$j" -eq 0 ] ;;
    H)   [ "$(hits "$b" 'tombstone-byte level ceiling breached')" -gt 0 ] && [ "$(hits "$b" 'soak: child TOPGUN_JOURNAL_ENABLED=')" -gt 0 ] ;;
  esac
  marker=$?
  [ "$rec" = "yes" ] || { say "FATAL: flavour ${fl} recompiled=${rec}"; finish 1; }
  [ "$mt" -ge "$CHAIN_START_EPOCH" ] || { say "FATAL: flavour ${fl} binary predates the chain start"; finish 1; }
  [ "$marker" -eq 0 ] || { say "FATAL: flavour ${fl} marker mismatch (alloc_probe=${p} dhat=${d} je_probe=${j} mimalloc=${m})"; finish 1; }
  echo "flavour=${fl} path=${b} sha256=$(shasum -a 256 "$b" | awk '{print $1}') mtime=${mt} recompiled=${rec} marker=ok" >> "$BUILDS"
done
cat "$BUILDS" >> "$LOG"

# ----------------------------------------------------------------- 4. cells
CELLS="s2"
case "$SURV" in JE|JE+MI) CELLS="$CELLS j2" ;; esac
case "$SURV" in MI|JE+MI) CELLS="$CELLS m2" ;; esac
A2J_STATE=ran
if [ "$A2J_DROPPED_FOR_BUDGET" = "yes" ]; then A2J_STATE=budget
else
  case "$RANK" in JE|MI) CELLS="$CELLS a2j" ;; *) A2J_STATE=no_s1_rank; say "a2j not run: S1_RANK=${RANK}" ;; esac
fi
export SPEC372_A2J_FLAVOUR="$RANK"
say "cells: ${CELLS} (a2j_state=${A2J_STATE}, a2j flavour=${RANK})"

export SPEC372_CHAIN_START_EPOCH="$CHAIN_START_EPOCH"
export SPEC372_HARNESS_BIN="$BIN_H"
host_state() {
  echo "HOST uptime: $(uptime)"
  echo "HOST memory_pressure: $(memory_pressure 2>/dev/null | tail -1)"
  vm_stat | head -8 | sed 's/^/HOST vm_stat: /'
  pmset -g therm 2>/dev/null | sed 's/^/HOST therm: /'
}
run_cell() {   # $1 = cell
  local c="$1" rc fl
  case "$c" in s2) fl=SYS ;; j2) fl=JE ;; m2) fl=MI ;; a2j) fl="$RANK" ;; esac
  case "$fl" in SYS) SOAK_SERVER_BINARY="$BIN_SYS" ;; JE) SOAK_SERVER_BINARY="$BIN_JE" ;; MI) SOAK_SERVER_BINARY="$BIN_MI" ;; esac
  export SOAK_SERVER_BINARY
  if [ "$SMOKE" = "1" ]; then
    export SPEC365_DATA_DIR="$OUT/data-${c}" SPEC365_OUT_DIR="$OUT" SPEC365_SMOKE_SAMPLE_INTERVAL=20
    if [ "$c" = "a2j" ]; then export SPEC362B_SMOKE_DURATION=120 SPEC371_SMOKE_LIVE_CENSUS=30
    else export SPEC362B_SMOKE_DURATION=300 SPEC371_SMOKE_LIVE_CENSUS=10; fi
  fi
  { echo "--- cell ${c} at $(date -u +%Y-%m-%dT%H:%M:%SZ)"; host_state; } | tee -a "$LOG" > "$OUT/spec372-${c}.runner-console.log"
  bash "$SCRIPT_DIR/spec372-allocdiag.sh" "$c" >> "$OUT/spec372-${c}.runner-console.log" 2>&1
  rc=$?
  echo "RUNNER_EXIT=${rc}" >> "$OUT/spec372-${c}.runner-console.log"
  say "cell ${c}: RUNNER_EXIT=${rc}"
}
for c in $CELLS; do run_cell "$c"; done

# ----------------------------------------------------------------- 5-7. programs
files=""
for c in $CELLS; do
  bash "$SCRIPT_DIR/spec372-predicates.sh" "$OUT" "spec372-${c}" "$BUILDS" >> "$LOG" 2>&1
  [ -f "$OUT/spec372-${c}.predicates.txt" ] && files="$files $OUT/spec372-${c}.predicates.txt"
done
kfiles=""
for c in s2 j2 m2; do [ -f "$OUT/spec372-${c}.predicates.txt" ] && case " $CELLS " in *" $c "*) kfiles="$kfiles $OUT/spec372-${c}.predicates.txt" ;; esac; done
# shellcheck disable=SC2086
awk -v mode=stage2 -f "$SCRIPT_DIR/spec372-k.awk" "$STAGE1" $kfiles > "$OUT/spec372.k-stage2.txt" 2>> "$LOG"
KRC=$?
say "k.awk stage2 rc=${KRC}"
# Every cell this chain ran must have produced its predicates; one that did not
# is a named missing input (a STOP), never a cell that "did not run".
missing=""
for c in $CELLS; do [ -f "$OUT/spec372-${c}.predicates.txt" ] || missing="$missing spec372-${c}.predicates.txt"; done
extra=""
for f in spec372-perf.txt spec372-buildstory.txt; do [ -f "$OUT/$f" ] && extra="$extra $OUT/$f"; done
# shellcheck disable=SC2086
awk -v mode=stage2 -v disk_free="$DISK_FREE_GB" -v a2j_state="$A2J_STATE" -v a2j_flavour="$RANK" -v missing="${missing# }" \
    -f "$SCRIPT_DIR/spec372-decide.awk" "$STAGE1" $files "$OUT/spec372.k-stage2.txt" $extra > "$OUT/spec372.decision.txt" 2>> "$LOG"
DRC=$?
say "decide.awk stage2 rc=${DRC}"
cat "$OUT/spec372.decision.txt" >> "$LOG"
# The decision file is the one artifact the conductor reads: a failed program
# or a file without its STOP and NEXT lines must not end as a finished chain.
RC=0
if [ "$KRC" -ne 0 ] || [ "$DRC" -ne 0 ] || ! grep -qE '^STOP=(TRUE|FALSE)$' "$OUT/spec372.decision.txt" || ! grep -q '^NEXT=' "$OUT/spec372.decision.txt"; then
  say "FATAL: a Stage-2 program failed or wrote no decision (k rc=${KRC}, decide rc=${DRC})"; RC=4
fi
[ "$SMOKE" = "1" ] && say "### SMOKE COMPLETE $(date -u +%Y-%m-%dT%H:%M:%SZ)"
finish "$RC"
