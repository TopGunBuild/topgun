#!/usr/bin/env bash
#
# spec373b diagnostic dhat chain (pre-registration input, not cell data): one
# detached launch that
#
#   1. records the chain start epoch
#   2. prepares a clean, detached checkout of the pin b166719d under target/
#      (asserting its HEAD and a clean tree), builds the DH server there and
#      the soak harness at this checkout, into fresh chain-owned target dirs;
#      asserts recompiled=yes, mtime >= chain start and the flavour markers;
#      writes spec373b-diag-builds.txt
#   3. runs de -> dl through spec373b-cells.sh (the c3e/c3l shape of spec371:
#      300 s and 900 s, SIGTERM teardown, dhat profile gzipped), recording the
#      host load average at each cell's start
#   4. reads each cell's totalWrites and writes/s (totalWrites / the nominal
#      duration, the unit of the spec371 reference) and prints the write-rate
#      STOP: de < 60.9 writes/s or dl < 46.6 writes/s (0.8 x the spec371 c3e
#      76.1 and c3l 58.3 writes/s); the DH/CA ratio against SPEC-373a b1/b2
#      (mean 174.0 writes/s) is printed, not gated
#   5. runs the E program spec373b-shares.py: the in-program self-check over
#      the committed spec371-c3{e,l} pair, then the base reading over de/dl
#
# The outputs are READ at STOP 5a; nothing here decides a flag beyond what
# spec373b-shares.py and step 4 print. A failing step does not abort the later
# ones.
set -uo pipefail
export LC_ALL=C

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
SERVER_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd -P)"          # packages/server-rust
REPO_ROOT="$(cd "$SERVER_ROOT/../.." && pwd -P)"
OUT="$SCRIPT_DIR"
PIN=b166719d
LOG="$OUT/spec373b-diag.log"
: > "$LOG"
say() { echo "$*" | tee -a "$LOG"; }

# ----------------------------------------------------------------- 1. start
CHAIN_START_EPOCH="$(date +%s)"
say "chain start: $(date -u +%Y-%m-%dT%H:%M:%SZ) epoch=${CHAIN_START_EPOCH} HEAD=$(git -C "$REPO_ROOT" rev-parse HEAD)"
say "host load at chain start: $(uptime)"
if [ -z "${SDKROOT:-}" ] && [ -x /usr/bin/xcrun ]; then
  SDKROOT="$(/usr/bin/xcrun --sdk macosx --show-sdk-path 2>/dev/null || true)"
  [ -n "$SDKROOT" ] && export SDKROOT
fi

# ----------------------------------------------------------------- 2. builds
T_ROOT="${REPO_ROOT}/target"
SRC_PIN="${T_ROOT}/spec373b-src-${PIN}"
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
    "${REPO_ROOT}/target/spec373b-dh-pin"|"${REPO_ROOT}/target/spec373b-dh-h") rm -rf "$resolved" ;;
    *) say "FATAL: refusing to remove '$resolved': not a chain-owned target dir"; exit 1 ;;
  esac
}
build() {   # $1 = flavour, $2 = source crate dir, $3 = target dir, $4.. = cargo args
  local fl="$1" src="$2" td="$3"; shift 3
  guarded_rm "$td"
  say "build ${fl}: (cd ${src} && CARGO_TARGET_DIR=${td} cargo build $*)"
  ( cd "$src" && CARGO_TARGET_DIR="$td" cargo build "$@" ) > "$OUT/spec373b-diag-build-${fl}.log" 2>&1
  local rc=$?
  tail -3 "$OUT/spec373b-diag-build-${fl}.log" >> "$LOG"
  [ "$rc" -eq 0 ] || { say "FATAL: build ${fl} failed rc=${rc}"; exit 1; }
  if grep -q 'Compiling topgun-server v' "$OUT/spec373b-diag-build-${fl}.log"; then echo yes; else echo no; fi > "$OUT/.recompiled-${fl}"
}
build DH "${SRC_PIN}/packages/server-rust" "${T_ROOT}/spec373b-dh-pin" --profile release-with-debug --features dhat-heap --bin topgun-server
build H  "$SERVER_ROOT"                    "${T_ROOT}/spec373b-dh-h"   --release --bench soak_harness

BIN_DH="${T_ROOT}/spec373b-dh-pin/release-with-debug/topgun-server"
H_CANDS="$(ls "${T_ROOT}"/spec373b-dh-h/release/deps/soak_harness-* 2>/dev/null | grep -vE '\.(d|o|rcgu)' || true)"
[ "$(printf '%s\n' "$H_CANDS" | grep -c .)" -eq 1 ] || { say "FATAL: expected exactly one soak_harness binary, got: ${H_CANDS}"; exit 1; }
BIN_H="$H_CANDS"
hits() { strings "$1" | grep -c "$2" || true; }
BUILDS="$OUT/spec373b-diag-builds.txt"
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
export SPEC373B_CHAIN_START_EPOCH="$CHAIN_START_EPOCH"
export SPEC373B_HARNESS_BIN="$BIN_H"
export SPEC373B_SERVER_COMMIT="$PIN"
export SOAK_SERVER_BINARY="$BIN_DH"
for c in de dl; do
  { echo "--- cell ${c} at $(date -u +%Y-%m-%dT%H:%M:%SZ)"; vm_stat | head -8; } >> "$LOG"
  say "LOAD_AT_START_${c}=$(uptime | sed 's/.*load averages*: *//')"
  bash "$SCRIPT_DIR/spec373b-cells.sh" "$c" > "$OUT/spec373b-${c}.runner-console.log" 2>&1
  rc=$?
  echo "RUNNER_EXIT=${rc}" >> "$OUT/spec373b-${c}.runner-console.log"
  say "cell ${c}: RUNNER_EXIT=${rc}"
done

# ----------------------------------------------------------------- 4. write rate
# Rate = totalWrites / the NOMINAL duration (300 / 900 s), the unit of the
# spec371 reference (c3e 22833 / 300 = 76.1, c3l 52457 / 900 = 58.3).
RATE_STOP=""
for c in de dl; do
  case "$c" in de) dur=300; floor=60.9 ;; dl) dur=900; floor=46.6 ;; esac
  tw="$(sed -nE 's/.*"totalWrites": *([0-9]+).*/\1/p' "$OUT/spec373b-${c}.soak.json" 2>/dev/null | head -1)"
  if ! printf '%s' "$tw" | grep -Eq '^[0-9]+$'; then
    say "TOTAL_WRITES_${c}=n/a"; RATE_STOP="${RATE_STOP} ${c}:totalWrites=n/a"; continue
  fi
  wps="$(awk -v t="$tw" -v d="$dur" 'BEGIN { printf "%.1f", t / d }')" || { say "FATAL: awk (rate ${c})"; RATE_STOP="${RATE_STOP} ${c}:awk"; continue; }
  low="$(awk -v r="$wps" -v f="$floor" 'BEGIN { print (r + 0 < f + 0) ? 1 : 0 }')" || { say "FATAL: awk (floor ${c})"; RATE_STOP="${RATE_STOP} ${c}:awk"; continue; }
  dhca="$(awk -v r="$wps" 'BEGIN { printf "%.3f", r / 174.0 }')" || dhca=n/a
  say "TOTAL_WRITES_${c}=${tw} WRITES_PER_S_${c}=${wps} FLOOR_${c}=${floor} DH_CA_RATIO_${c}=${dhca} (recorded, not gated)"
  [ "$low" = "0" ] || RATE_STOP="${RATE_STOP} ${c}:${wps}<${floor}"
done
if [ -n "$RATE_STOP" ]; then say "WRITE_RATE_STOP=TRUE (${RATE_STOP# })"; else say "WRITE_RATE_STOP=FALSE"; fi

# ----------------------------------------------------------------- 5. programs
say "spec373b-shares.py sha256=$(shasum -a 256 "$SCRIPT_DIR/spec373b-shares.py" | awk '{print $1}')"
if [ -s "$OUT/spec373b-de.dhat.json.gz" ] && [ -s "$OUT/spec373b-dl.dhat.json.gz" ]; then
  ( cd "$OUT" && python3 "$SCRIPT_DIR/spec373b-shares.py" --pin "$PIN" \
      spec373b-de.dhat.json.gz spec373b-dl.dhat.json.gz \
      --self spec371-c3e.dhat.json.gz spec371-c3l.dhat.json.gz ) > "$OUT/spec373b-shares.txt" 2>&1
  say "shares rc=$?"; cat "$OUT/spec373b-shares.txt" >> "$LOG"
else
  say "shares SKIPPED: a d profile is missing (STOP-D)"
fi
say "chain end: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
