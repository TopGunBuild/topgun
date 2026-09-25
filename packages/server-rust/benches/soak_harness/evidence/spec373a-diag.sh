#!/usr/bin/env bash
#
# spec373a diagnostic dhat chain (pre-registration input, not cell data): one
# detached launch that
#
#   1. records the chain start epoch
#   2. prepares a clean, detached checkout of the pin 61f84658 under target/
#      (asserting its HEAD and a clean tree), builds the DH server there and
#      the soak harness at this checkout, into fresh chain-owned target dirs;
#      asserts recompiled=yes, mtime >= chain start and the flavour markers;
#      writes spec373a-diag-builds.txt
#   3. runs d3e -> d3l through spec373a-cells.sh (the c3e/c3l shape of
#      spec371: 300 s and 900 s, SIGTERM teardown, dhat profile gzipped)
#   4. runs the E program shares_61f.py: the --pin 46dcc12a self-check over the
#      committed spec371-c3{e,l} pair, then --pin 61f84658 over d3e/d3l with the
#      371 pair as --old; and the context table tb2_61f.py over d3e and d3l
#
# The shares outputs are READ at STOP 3c; nothing here decides a flag beyond
# what shares_61f.py prints. A failing step does not abort the later ones.
set -uo pipefail
export LC_ALL=C

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
SERVER_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd -P)"          # packages/server-rust
REPO_ROOT="$(cd "$SERVER_ROOT/../.." && pwd -P)"
OUT="$SCRIPT_DIR"
PIN=61f84658
PROG_DIR="${REPO_ROOT}/.specflow/research/spec373-dhat-attribution"
LOG="$OUT/spec373a-diag.log"
: > "$LOG"
say() { echo "$*" | tee -a "$LOG"; }

# ----------------------------------------------------------------- 1. start
CHAIN_START_EPOCH="$(date +%s)"
say "chain start: $(date -u +%Y-%m-%dT%H:%M:%SZ) epoch=${CHAIN_START_EPOCH} HEAD=$(git -C "$REPO_ROOT" rev-parse HEAD)"
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
say "pin checkout: ${SRC_PIN} HEAD=${PIN_HEAD} clean"

guarded_rm() {   # removes only the chain-owned target dirs
  local cand="$1" parent resolved
  parent="$(dirname "$cand")"; mkdir -p "$parent"
  resolved="$(cd "$parent" && pwd -P)/$(basename "$cand")"
  case "$resolved" in
    "${REPO_ROOT}/target/spec373a-dh-pin"|"${REPO_ROOT}/target/spec373a-h") rm -rf "$resolved" ;;
    *) say "FATAL: refusing to remove '$resolved': not a chain-owned target dir"; exit 1 ;;
  esac
}
build() {   # $1 = flavour, $2 = source crate dir, $3 = target dir, $4.. = cargo args
  local fl="$1" src="$2" td="$3"; shift 3
  guarded_rm "$td"
  say "build ${fl}: (cd ${src} && CARGO_TARGET_DIR=${td} cargo build $*)"
  ( cd "$src" && CARGO_TARGET_DIR="$td" cargo build "$@" ) > "$OUT/spec373a-diag-build-${fl}.log" 2>&1
  local rc=$?
  tail -3 "$OUT/spec373a-diag-build-${fl}.log" >> "$LOG"
  [ "$rc" -eq 0 ] || { say "FATAL: build ${fl} failed rc=${rc}"; exit 1; }
  if grep -q 'Compiling topgun-server v' "$OUT/spec373a-diag-build-${fl}.log"; then echo yes; else echo no; fi > "$OUT/.recompiled-${fl}"
}
build DH "${SRC_PIN}/packages/server-rust" "${T_ROOT}/spec373a-dh-pin" --profile release-with-debug --features dhat-heap --bin topgun-server
build H  "$SERVER_ROOT"                    "${T_ROOT}/spec373a-h"      --release --bench soak_harness

BIN_DH="${T_ROOT}/spec373a-dh-pin/release-with-debug/topgun-server"
BIN_H="$(ls -t "${T_ROOT}"/spec373a-h/release/deps/soak_harness-* 2>/dev/null | grep -vE '\.(d|o|rcgu)' | head -1 || true)"
hits() { strings "$1" | grep -c "$2" || true; }
BUILDS="$OUT/spec373a-diag-builds.txt"
: > "$BUILDS"
for fl in DH H; do
  case "$fl" in DH) b="$BIN_DH"; code="$PIN_FULL" ;; H) b="$BIN_H"; code="$(git -C "$REPO_ROOT" rev-parse HEAD)" ;; esac
  [ -n "$b" ] && [ -x "$b" ] || { say "FATAL: flavour ${fl} binary missing: '${b}'"; exit 1; }
  rec="$(cat "$OUT/.recompiled-${fl}")"; rm -f "$OUT/.recompiled-${fl}"
  mt="$(date -r "$b" '+%s')"
  case "$fl" in
    DH) [ "$(hits "$b" 'DHAT_OUT')" -gt 0 ] && [ "$(hits "$b" 'alloc_probe elapsed_s=')" -eq 0 ] ;;
    H)  [ "$(hits "$b" 'tombstone-byte level ceiling breached')" -gt 0 ] && [ "$(hits "$b" 'soak: child TOPGUN_JOURNAL_ENABLED=')" -gt 0 ] ;;
  esac
  marker=$?
  [ "$rec" = "yes" ] || { say "FATAL: flavour ${fl} recompiled=${rec}"; exit 1; }
  [ "$mt" -ge "$CHAIN_START_EPOCH" ] || { say "FATAL: flavour ${fl} binary predates the chain start"; exit 1; }
  [ "$marker" -eq 0 ] || { say "FATAL: flavour ${fl} marker mismatch"; exit 1; }
  echo "flavour=${fl} code=${code} path=${b} sha256=$(shasum -a 256 "$b" | awk '{print $1}') mtime=${mt} recompiled=${rec} marker=ok" >> "$BUILDS"
done
cat "$BUILDS" >> "$LOG"

# ----------------------------------------------------------------- 3. cells
export SPEC373A_CHAIN_START_EPOCH="$CHAIN_START_EPOCH"
export SPEC373A_HARNESS_BIN="$BIN_H"
export SPEC373A_SERVER_COMMIT="$PIN"
export SOAK_SERVER_BINARY="$BIN_DH"
for c in d3e d3l; do
  { echo "--- cell ${c} at $(date -u +%Y-%m-%dT%H:%M:%SZ)"; uptime; vm_stat | head -8; } >> "$LOG"
  bash "$SCRIPT_DIR/spec373a-cells.sh" "$c" > "$OUT/spec373a-${c}.runner-console.log" 2>&1
  rc=$?
  echo "RUNNER_EXIT=${rc}" >> "$OUT/spec373a-${c}.runner-console.log"
  say "cell ${c}: RUNNER_EXIT=${rc}"
done

# ----------------------------------------------------------------- 4. programs
say "shares_61f.py sha256=$(shasum -a 256 "$PROG_DIR/shares_61f.py" | awk '{print $1}')"
say "tb2_61f.py sha256=$(shasum -a 256 "$PROG_DIR/tb2_61f.py" | awk '{print $1}')"
python3 "$PROG_DIR/shares_61f.py" --pin 46dcc12a \
  "$OUT/spec371-c3e.dhat.json.gz" "$OUT/spec371-c3l.dhat.json.gz" > "$OUT/spec373a-selfcheck.txt" 2>&1
say "self-check rc=$?"; cat "$OUT/spec373a-selfcheck.txt" >> "$LOG"
if [ -s "$OUT/spec373a-d3e.dhat.json.gz" ] && [ -s "$OUT/spec373a-d3l.dhat.json.gz" ]; then
  python3 "$PROG_DIR/shares_61f.py" --pin 61f84658 \
    "$OUT/spec373a-d3e.dhat.json.gz" "$OUT/spec373a-d3l.dhat.json.gz" \
    --old "$OUT/spec371-c3e.dhat.json.gz" "$OUT/spec371-c3l.dhat.json.gz" > "$OUT/spec373a-shares.txt" 2>&1
  say "shares rc=$?"; cat "$OUT/spec373a-shares.txt" >> "$LOG"
  for c in d3e d3l; do
    python3 "$PROG_DIR/tb2_61f.py" "$OUT/spec373a-${c}.dhat.json.gz" > "$OUT/spec373a-${c}.tb2.txt" 2>&1
  done
else
  say "shares SKIPPED: a d3 profile is missing (STOP-D)"
fi
say "chain end: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
