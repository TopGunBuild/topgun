#!/usr/bin/env bash
#
# spec373a cell chain (carve 9c part a): one detached launch that
#
#   1. records the chain start epoch
#   2. builds the CA server at the pin 61f84658 (from the clean pin checkout
#      target/spec373a-src-61f84658, HEAD and cleanliness asserted), the CA
#      server at this checkout (whose .rs tree the runner asserts equals the
#      freeze commit), and the soak harness at this checkout, into fresh
#      chain-owned target dirs; asserts recompiled=yes, mtime >= chain start
#      and the flavour markers; writes spec373a-builds.txt
#   3. runs b1 -> a1 -> b2 -> a2 through spec373a-cells.sh (interleaved, so a
#      slow host drift lands on both sides), each 900 s, SIGKILL teardown
#   4. runs spec373a-verdict.sh over the four cells and the manifest ->
#      spec373a.verdict.txt
#
# SPEC373A_SMOKE=1 runs the admission smoke instead, into SPEC365_OUT_DIR (a
# scratch dir): the same three builds, b1 and a1 at 120 s / cadence 20, the
# verdict program over them (b2/a2 absent, so STOP=V is EXPECTED), and the seven
# synthetic verdict cases of spec373a-synth.sh.
set -uo pipefail
export LC_ALL=C

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
SERVER_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd -P)"          # packages/server-rust
REPO_ROOT="$(cd "$SERVER_ROOT/../.." && pwd -P)"
PIN=61f84658
FREEZE="$(awk -F= '/^SPEC373A_CODE_FREEZE=/ { print $2; exit }' "$SCRIPT_DIR/spec373a-cells.sh")"
SMOKE="${SPEC373A_SMOKE:-0}"
if [ "$SMOKE" = "1" ]; then
  OUT="${SPEC365_OUT_DIR:-}"
  [ -n "$OUT" ] || { echo "FATAL: smoke needs SPEC365_OUT_DIR (a scratch dir)" >&2; exit 2; }
  mkdir -p "$OUT"; OUT="$(cd "$OUT" && pwd -P)"
  [ "$OUT" != "$SCRIPT_DIR" ] || { echo "FATAL: smoke must not write into the evidence dir" >&2; exit 2; }
  CELLS="b1 a1"
else
  OUT="$SCRIPT_DIR"
  CELLS="b1 a1 b2 a2"
fi
LOG="$OUT/spec373a-chain.log"
: > "$LOG"
say() { echo "$*" | tee -a "$LOG"; }

# ----------------------------------------------------------------- 1. start
CHAIN_START_EPOCH="$(date +%s)"
say "chain start: $(date -u +%Y-%m-%dT%H:%M:%SZ) epoch=${CHAIN_START_EPOCH} smoke=${SMOKE} HEAD=$(git -C "$REPO_ROOT" rev-parse HEAD) freeze=${FREEZE}"
if [ -z "${SDKROOT:-}" ] && [ -x /usr/bin/xcrun ]; then
  SDKROOT="$(/usr/bin/xcrun --sdk macosx --show-sdk-path 2>/dev/null || true)"
  [ -n "$SDKROOT" ] && export SDKROOT
fi

# ----------------------------------------------------------------- 2. builds
T_ROOT="${REPO_ROOT}/target"
SRC_PIN="${T_ROOT}/spec373a-src-${PIN}"
if [ ! -d "$SRC_PIN" ]; then
  git -C "$REPO_ROOT" worktree add --detach "$SRC_PIN" "$PIN" >> "$LOG" 2>&1 \
    || { say "FATAL: cannot create the pin checkout at $SRC_PIN"; exit 1; }
fi
PIN_HEAD="$(git -C "$SRC_PIN" rev-parse HEAD)"
PIN_FULL="$(git -C "$REPO_ROOT" rev-parse "${PIN}^{commit}")"
[ "$PIN_HEAD" = "$PIN_FULL" ] || { say "FATAL: pin checkout HEAD ${PIN_HEAD} != ${PIN_FULL}"; exit 1; }
[ -z "$(git -C "$SRC_PIN" status --porcelain)" ] || { say "FATAL: pin checkout is dirty"; exit 1; }
[ -z "$(git -C "$REPO_ROOT" diff --stat "$FREEZE"..HEAD -- '*.rs')" ] || { say "FATAL: .rs at HEAD differs from the freeze ${FREEZE}"; exit 1; }
[ -z "$(git -C "$REPO_ROOT" status --porcelain -- '*.rs')" ] || { say "FATAL: the .rs working tree is dirty"; exit 1; }
say "pin checkout: ${SRC_PIN} HEAD=${PIN_HEAD} clean; head .rs == freeze ${FREEZE}"

guarded_rm() {
  local cand="$1" parent resolved
  parent="$(dirname "$cand")"; mkdir -p "$parent"
  resolved="$(cd "$parent" && pwd -P)/$(basename "$cand")"
  case "$resolved" in
    "${REPO_ROOT}/target/spec373a-ca-pin"|"${REPO_ROOT}/target/spec373a-ca-head"|"${REPO_ROOT}/target/spec373a-h") rm -rf "$resolved" ;;
    *) say "FATAL: refusing to remove '$resolved': not a chain-owned target dir"; exit 1 ;;
  esac
}
build() {   # $1 = label, $2 = source crate dir, $3 = target dir, $4.. = cargo args
  local fl="$1" src="$2" td="$3"; shift 3
  guarded_rm "$td"
  say "build ${fl}: (cd ${src} && CARGO_TARGET_DIR=${td} cargo build $*)"
  ( cd "$src" && CARGO_TARGET_DIR="$td" cargo build "$@" ) > "$OUT/spec373a-build-${fl}.log" 2>&1
  local rc=$?
  tail -3 "$OUT/spec373a-build-${fl}.log" >> "$LOG"
  [ "$rc" -eq 0 ] || { say "FATAL: build ${fl} failed rc=${rc}"; exit 1; }
  if grep -q 'Compiling topgun-server v' "$OUT/spec373a-build-${fl}.log"; then echo yes; else echo no; fi > "$OUT/.recompiled-${fl}"
}
build CA-pin  "${SRC_PIN}/packages/server-rust" "${T_ROOT}/spec373a-ca-pin"  --release --features count-alloc --bin topgun-server
build CA-head "$SERVER_ROOT"                    "${T_ROOT}/spec373a-ca-head" --release --features count-alloc --bin topgun-server
build H       "$SERVER_ROOT"                    "${T_ROOT}/spec373a-h"       --release --bench soak_harness

BIN_CA_PIN="${T_ROOT}/spec373a-ca-pin/release/topgun-server"
BIN_CA_HEAD="${T_ROOT}/spec373a-ca-head/release/topgun-server"
BIN_H="$(ls -t "${T_ROOT}"/spec373a-h/release/deps/soak_harness-* 2>/dev/null | grep -vE '\.(d|o|rcgu)' | head -1 || true)"
HEAD_FULL="$(git -C "$REPO_ROOT" rev-parse HEAD)"
hits() { strings "$1" | grep -c "$2" || true; }
BUILDS="$OUT/spec373a-builds.txt"
: > "$BUILDS"
for fl in CA-pin CA-head H; do
  case "$fl" in
    CA-pin)  b="$BIN_CA_PIN";  code="$PIN_FULL" ;;
    CA-head) b="$BIN_CA_HEAD"; code="$HEAD_FULL" ;;
    H)       b="$BIN_H";       code="$HEAD_FULL" ;;
  esac
  [ -n "$b" ] && [ -x "$b" ] || { say "FATAL: ${fl} binary missing: '${b}'"; exit 1; }
  rec="$(cat "$OUT/.recompiled-${fl}")"; rm -f "$OUT/.recompiled-${fl}"
  mt="$(date -r "$b" '+%s')"
  case "$fl" in
    CA-*) [ "$(hits "$b" 'alloc_probe elapsed_s=')" -gt 0 ] && [ "$(hits "$b" 'DHAT_OUT')" -eq 0 ] ;;
    H)    [ "$(hits "$b" 'tombstone-byte level ceiling breached')" -gt 0 ] && [ "$(hits "$b" 'soak: child TOPGUN_JOURNAL_ENABLED=')" -gt 0 ] ;;
  esac
  marker=$?
  [ "$rec" = "yes" ] || { say "FATAL: ${fl} recompiled=${rec}"; exit 1; }
  [ "$mt" -ge "$CHAIN_START_EPOCH" ] || { say "FATAL: ${fl} binary predates the chain start"; exit 1; }
  [ "$marker" -eq 0 ] || { say "FATAL: ${fl} marker mismatch"; exit 1; }
  echo "flavour=${fl} code=${code} path=${b} sha256=$(shasum -a 256 "$b" | awk '{print $1}') mtime=${mt} recompiled=${rec} marker=ok" >> "$BUILDS"
done
cat "$BUILDS" >> "$LOG"

# ----------------------------------------------------------------- 3. cells
export SPEC373A_CHAIN_START_EPOCH="$CHAIN_START_EPOCH"
export SPEC373A_HARNESS_BIN="$BIN_H"
run_cell() {   # $1 = cell
  local c="$1" rc
  case "$c" in
    b1|b2) SOAK_SERVER_BINARY="$BIN_CA_PIN";  SPEC373A_SERVER_COMMIT="$PIN" ;;
    a1|a2) SOAK_SERVER_BINARY="$BIN_CA_HEAD"; SPEC373A_SERVER_COMMIT="$FREEZE" ;;
  esac
  export SOAK_SERVER_BINARY SPEC373A_SERVER_COMMIT
  { echo "--- cell ${c} at $(date -u +%Y-%m-%dT%H:%M:%SZ)"; uptime; vm_stat | head -8; } >> "$LOG"
  if [ "$SMOKE" = "1" ]; then
    export SPEC365_DATA_DIR="$OUT/data-${c}"
    export SPEC365_SMOKE_SAMPLE_INTERVAL=20
    export SPEC362B_SMOKE_DURATION=120
    export SPEC365_OUT_DIR="$OUT"
  fi
  bash "$SCRIPT_DIR/spec373a-cells.sh" "$c" > "$OUT/spec373a-${c}.runner-console.log" 2>&1
  rc=$?
  echo "RUNNER_EXIT=${rc}" >> "$OUT/spec373a-${c}.runner-console.log"
  say "cell ${c}: RUNNER_EXIT=${rc}"
}
for c in $CELLS; do run_cell "$c"; done

# ----------------------------------------------------------------- 4. verdict
MANIFEST="$SCRIPT_DIR/spec373a-manifest.md"
bash "$SCRIPT_DIR/spec373a-verdict.sh" "$OUT" "$MANIFEST" > "$OUT/spec373a.verdict.txt" 2>&1
say "verdict rc=$?"; sed -n '/^== flags ==/,$p' "$OUT/spec373a.verdict.txt" | tee -a "$LOG"
if [ "$SMOKE" = "1" ]; then
  bash "$SCRIPT_DIR/spec373a-synth.sh" "$OUT/synthetic" 2>&1 | tee -a "$LOG"
  say "### SMOKE COMPLETE $(date -u +%Y-%m-%dT%H:%M:%SZ)"
fi
say "chain end: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
