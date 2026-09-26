#!/usr/bin/env bash
#
# spec373b cell chain (carve 9c part b): one detached launch per phase.
#
#   SPEC373B_PHASE=ca (default) -- the gating count-alloc cells:
#     1. records the chain start epoch; checks ORDER=OK (spec373b-order.sh)
#        against the manifest commit M named by SPEC373B_MANIFEST_COMMIT
#        (required) and refuses to run on any mismatch
#     2. builds the CA server at the pin b166719d (clean detached checkout
#        target/spec373b-src-b166719d), the CA server at the freeze literal of
#        spec373b-cells.sh (clean detached checkout target/spec373b-src-freeze,
#        whose build inputs ORDER asserted equal to HEAD's), and the soak
#        harness at the freeze checkout, into fresh chain-owned target dirs;
#        asserts recompiled=yes, mtime >= chain start and the flavour markers;
#        writes spec373b-builds.txt
#     3. runs b1 -> a1 -> b2 -> a2 through spec373b-cells.sh (interleaved in ONE
#        host session, so a host drift lands on both sides), 900 s each, SIGKILL
#     4. runs spec373b-verdict.sh over the four cells and the manifest ->
#        spec373b.verdict.txt (its exit status is logged; non-zero = no flags)
#   SPEC373B_PHASE=je -- the RECORDED jemalloc pair (n = 1 per side, no class):
#     1. as above
#     2. builds the JE server (--features alloc-jemalloc) at the pin and at the
#        freeze, and the harness; writes spec373b-builds-je.txt
#     3. runs jb -> ja, 14400 s each, SIGKILL
#     4. runs spec373b-je.sh over each cell (the frozen spec372-predicates.sh,
#        unedited, through a copy under a JE cell name it knows) ->
#        spec373b-<cell>.predicates.txt
#
# SPEC373B_SMOKE=1 runs the admission smoke instead, into SPEC365_OUT_DIR (a
# scratch dir): every build of both phases, b1, a1 and jb at 120 s / cadence 20,
# the verdict program over b1/a1 (b2/a2 absent, so STOP=V is EXPECTED), the JE
# reading over jb, the E program's self-check, and the synthetic verdict cases.
set -uo pipefail
export LC_ALL=C

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
SERVER_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd -P)"          # packages/server-rust
REPO_ROOT="$(cd "$SERVER_ROOT/../.." && pwd -P)"
PIN=b166719d
FREEZE="$(awk -F= '/^SPEC373B_CODE_FREEZE=/ { print $2; exit }' "$SCRIPT_DIR/spec373b-cells.sh")"
[ -n "$FREEZE" ] || { echo "FATAL: no SPEC373B_CODE_FREEZE literal in spec373b-cells.sh" >&2; exit 1; }
PHASE="${SPEC373B_PHASE:-ca}"
case "$PHASE" in ca|je) ;; *) echo "FATAL: SPEC373B_PHASE must be ca or je (got '${PHASE}')" >&2; exit 2 ;; esac
SMOKE="${SPEC373B_SMOKE:-0}"
if [ "$SMOKE" = "1" ]; then
  OUT="${SPEC365_OUT_DIR:-}"
  [ -n "$OUT" ] || { echo "FATAL: smoke needs SPEC365_OUT_DIR (a scratch dir)" >&2; exit 2; }
  mkdir -p "$OUT"; OUT="$(cd "$OUT" && pwd -P)"
  [ "$OUT" != "$SCRIPT_DIR" ] || { echo "FATAL: smoke must not write into the evidence dir" >&2; exit 2; }
  CELLS="b1 a1 jb"; FLAVOURS="CA-pin CA-head JE-pin H"
  LOG="$OUT/spec373b-chain.log"
elif [ "$PHASE" = "ca" ]; then
  OUT="$SCRIPT_DIR"; CELLS="b1 a1 b2 a2"; FLAVOURS="CA-pin CA-head H"
  LOG="$OUT/spec373b-chain.log"
else
  OUT="$SCRIPT_DIR"; CELLS="jb ja"; FLAVOURS="JE-pin JE-head H"
  LOG="$OUT/spec373b-chain-je.log"
fi
: > "$LOG"
say() { echo "$*" | tee -a "$LOG"; }

# ----------------------------------------------------------------- 1. start
CHAIN_START_EPOCH="$(date +%s)"
say "chain start: $(date -u +%Y-%m-%dT%H:%M:%SZ) epoch=${CHAIN_START_EPOCH} phase=${PHASE} smoke=${SMOKE} HEAD=$(git -C "$REPO_ROOT" rev-parse HEAD) freeze=${FREEZE}"
# spec373b-order.sh cannot vouch for itself: check its bytes against M's
# section-1 listing before it is trusted.
order_sha_ok() {   # $1 = M; prints nothing, returns 0 iff order.sh hashes as M lists it
  local want got
  want="$(git -C "$REPO_ROOT" show "${1}:packages/server-rust/benches/soak_harness/evidence/spec373b-manifest.md" 2>/dev/null \
    | sed '/^## APPEND-ONLY BELOW/q' | sed -nE 's/^- `([0-9a-f]{64})` `packages\/server-rust\/benches\/soak_harness\/evidence\/spec373b-order\.sh`.*/\1/p')"
  got="$(shasum -a 256 "$SCRIPT_DIR/spec373b-order.sh" 2>/dev/null | awk '{print $1}')"
  [ -n "$want" ] && [ "$want" = "$got" ]
}
MC="${SPEC373B_MANIFEST_COMMIT:-}"
if order_sha_ok "$MC"; then
  ORDER_LINE="$(bash "$SCRIPT_DIR/spec373b-order.sh" "$MC")"
  orc=$?
else
  ORDER_LINE="ORDER=FAIL spec373b-order.sh does not hash as M '${MC}' lists it"; orc=3
fi
say "$ORDER_LINE"
[ "$orc" -eq 0 ] || { say "FATAL: ORDER is not OK; refusing to run"; exit 1; }
if [ -z "${SDKROOT:-}" ] && [ -x /usr/bin/xcrun ]; then
  SDKROOT="$(/usr/bin/xcrun --sdk macosx --show-sdk-path 2>/dev/null || true)"
  [ -n "$SDKROOT" ] && export SDKROOT
fi

# ----------------------------------------------------------------- 2. builds
T_ROOT="${REPO_ROOT}/target"
checkout() {   # $1 = dir, $2 = revision; a clean detached checkout at exactly $2
  local dir="$1" rev="$2" full head
  if [ ! -d "$dir" ]; then
    git -C "$REPO_ROOT" worktree add --detach "$dir" "$rev" >> "$LOG" 2>&1 \
      || { say "FATAL: cannot create the checkout ${dir} at ${rev}"; exit 1; }
  fi
  full="$(git -C "$REPO_ROOT" rev-parse "${rev}^{commit}")"
  head="$(git -C "$dir" rev-parse HEAD)"
  [ "$head" = "$full" ] || { say "FATAL: checkout ${dir} HEAD ${head} != ${full}"; exit 1; }
  [ -z "$(git -C "$dir" status --porcelain)" ] || { say "FATAL: checkout ${dir} is dirty"; exit 1; }
  say "checkout: ${dir} HEAD=${head} clean"
}
SRC_PIN="${T_ROOT}/spec373b-src-${PIN}"
SRC_FRZ="${T_ROOT}/spec373b-src-freeze"
checkout "$SRC_PIN" "$PIN"
if [ -d "$SRC_FRZ" ] && [ "$(git -C "$SRC_FRZ" rev-parse HEAD)" != "$(git -C "$REPO_ROOT" rev-parse "${FREEZE}^{commit}")" ]; then
  git -C "$REPO_ROOT" worktree remove --force "$SRC_FRZ" >> "$LOG" 2>&1 || { say "FATAL: cannot drop the stale freeze checkout"; exit 1; }
fi
checkout "$SRC_FRZ" "$FREEZE"
PIN_FULL="$(git -C "$REPO_ROOT" rev-parse "${PIN}^{commit}")"
FRZ_FULL="$(git -C "$REPO_ROOT" rev-parse "${FREEZE}^{commit}")"

guarded_rm() {
  local cand="$1" parent resolved
  parent="$(dirname "$cand")"; mkdir -p "$parent"
  resolved="$(cd "$parent" && pwd -P)/$(basename "$cand")"
  case "$resolved" in
    "${REPO_ROOT}/target/spec373b-ca-pin"|"${REPO_ROOT}/target/spec373b-ca-head"|"${REPO_ROOT}/target/spec373b-je-pin"|"${REPO_ROOT}/target/spec373b-je-head"|"${REPO_ROOT}/target/spec373b-h") rm -rf "$resolved" ;;
    *) say "FATAL: refusing to remove '$resolved': not a chain-owned target dir"; exit 1 ;;
  esac
}
build() {   # $1 = label, $2 = source crate dir, $3 = target dir, $4.. = cargo args
  local fl="$1" src="$2" td="$3"; shift 3
  guarded_rm "$td"
  say "build ${fl}: (cd ${src} && CARGO_TARGET_DIR=${td} cargo build $*)"
  ( cd "$src" && CARGO_TARGET_DIR="$td" cargo build "$@" ) > "$OUT/spec373b-build-${fl}.log" 2>&1
  local rc=$?
  tail -3 "$OUT/spec373b-build-${fl}.log" >> "$LOG"
  [ "$rc" -eq 0 ] || { say "FATAL: build ${fl} failed rc=${rc}"; exit 1; }
  if grep -q 'Compiling topgun-server v' "$OUT/spec373b-build-${fl}.log"; then echo yes; else echo no; fi > "$OUT/.recompiled-${fl}"
}
for fl in $FLAVOURS; do
  case "$fl" in
    CA-pin)  build CA-pin  "${SRC_PIN}/packages/server-rust" "${T_ROOT}/spec373b-ca-pin"  --release --features count-alloc --bin topgun-server ;;
    CA-head) build CA-head "${SRC_FRZ}/packages/server-rust" "${T_ROOT}/spec373b-ca-head" --release --features count-alloc --bin topgun-server ;;
    JE-pin)  build JE-pin  "${SRC_PIN}/packages/server-rust" "${T_ROOT}/spec373b-je-pin"  --release --features alloc-jemalloc --bin topgun-server ;;
    JE-head) build JE-head "${SRC_FRZ}/packages/server-rust" "${T_ROOT}/spec373b-je-head" --release --features alloc-jemalloc --bin topgun-server ;;
    H)       build H       "${SRC_FRZ}/packages/server-rust" "${T_ROOT}/spec373b-h"       --release --bench soak_harness ;;
  esac
done

H_CANDS="$(ls "${T_ROOT}"/spec373b-h/release/deps/soak_harness-* 2>/dev/null | grep -vE '\.(d|o|rcgu)' || true)"
[ "$(printf '%s\n' "$H_CANDS" | grep -c .)" -eq 1 ] || { say "FATAL: expected exactly one soak_harness binary, got: ${H_CANDS}"; exit 1; }
BIN_H="$H_CANDS"
hits() { strings "$1" | grep -c "$2" || true; }
if [ "$PHASE" = "je" ] && [ "$SMOKE" != "1" ]; then BUILDS="$OUT/spec373b-builds-je.txt"; else BUILDS="$OUT/spec373b-builds.txt"; fi
: > "$BUILDS"
for fl in $FLAVOURS; do
  case "$fl" in
    CA-pin)  b="${T_ROOT}/spec373b-ca-pin/release/topgun-server";  code="$PIN_FULL" ;;
    CA-head) b="${T_ROOT}/spec373b-ca-head/release/topgun-server"; code="$FRZ_FULL" ;;
    JE-pin)  b="${T_ROOT}/spec373b-je-pin/release/topgun-server";  code="$PIN_FULL" ;;
    JE-head) b="${T_ROOT}/spec373b-je-head/release/topgun-server"; code="$FRZ_FULL" ;;
    H)       b="$BIN_H";                                           code="$FRZ_FULL" ;;
  esac
  [ -n "$b" ] && [ -x "$b" ] || { say "FATAL: ${fl} binary missing: '${b}'"; exit 1; }
  rec="$(cat "$OUT/.recompiled-${fl}")"; rm -f "$OUT/.recompiled-${fl}"
  mt="$(date -r "$b" '+%s')"
  case "$fl" in
    CA-*) [ "$(hits "$b" 'alloc_probe elapsed_s=')" -gt 0 ] && [ "$(hits "$b" 'DHAT_OUT')" -eq 0 ] && [ "$(hits "$b" 'je_probe elapsed_s=')" -eq 0 ] ;;
    JE-*) [ "$(hits "$b" 'je_probe elapsed_s=')" -gt 0 ] && [ "$(hits "$b" 'alloc_probe elapsed_s=')" -eq 0 ] && [ "$(hits "$b" 'DHAT_OUT')" -eq 0 ] ;;
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
export SPEC373B_CHAIN_START_EPOCH="$CHAIN_START_EPOCH"
export SPEC373B_HARNESS_BIN="$BIN_H"
run_cell() {   # $1 = cell
  local c="$1" rc
  case "$c" in
    b1|b2) SOAK_SERVER_BINARY="${T_ROOT}/spec373b-ca-pin/release/topgun-server";  SPEC373B_SERVER_COMMIT="$PIN" ;;
    a1|a2) SOAK_SERVER_BINARY="${T_ROOT}/spec373b-ca-head/release/topgun-server"; SPEC373B_SERVER_COMMIT="$FREEZE" ;;
    jb)    SOAK_SERVER_BINARY="${T_ROOT}/spec373b-je-pin/release/topgun-server";  SPEC373B_SERVER_COMMIT="$PIN" ;;
    ja)    SOAK_SERVER_BINARY="${T_ROOT}/spec373b-je-head/release/topgun-server"; SPEC373B_SERVER_COMMIT="$FREEZE" ;;
  esac
  export SOAK_SERVER_BINARY SPEC373B_SERVER_COMMIT
  { echo "--- cell ${c} at $(date -u +%Y-%m-%dT%H:%M:%SZ)"; vm_stat | head -8; } >> "$LOG"
  say "LOAD_AT_START_${c}=$(uptime | sed 's/.*load averages*: *//')"
  if [ "$SMOKE" = "1" ]; then
    export SPEC365_DATA_DIR="$OUT/data-${c}"
    export SPEC365_SMOKE_SAMPLE_INTERVAL=20
    export SPEC362B_SMOKE_DURATION=120
    export SPEC365_OUT_DIR="$OUT"
  fi
  bash "$SCRIPT_DIR/spec373b-cells.sh" "$c" > "$OUT/spec373b-${c}.runner-console.log" 2>&1
  rc=$?
  echo "RUNNER_EXIT=${rc}" >> "$OUT/spec373b-${c}.runner-console.log"
  say "cell ${c}: RUNNER_EXIT=${rc}"
}
for c in $CELLS; do run_cell "$c"; done

# ----------------------------------------------------------------- 4. readings
MANIFEST="$SCRIPT_DIR/spec373b-manifest.md"
if [ "$PHASE" = "ca" ] || [ "$SMOKE" = "1" ]; then
  SPEC373B_MANIFEST_COMMIT="$MC" bash "$SCRIPT_DIR/spec373b-verdict.sh" "$OUT" "$MANIFEST" > "$OUT/spec373b.verdict.txt" 2>&1
  VRC=$?
  say "verdict rc=${VRC}$( [ "$VRC" -ne 0 ] && echo ' (NO flags: see spec373b.verdict.txt)')"
  sed -n '/^== flags ==/,$p' "$OUT/spec373b.verdict.txt" | tee -a "$LOG"
fi
if [ "$PHASE" = "je" ] || [ "$SMOKE" = "1" ]; then
  for c in $CELLS; do
    case "$c" in jb|ja) ;; *) continue ;; esac
    bash "$SCRIPT_DIR/spec373b-je.sh" "$OUT" "$c" "$BUILDS" > "$OUT/spec373b-${c}.je-reading.log" 2>&1
    say "je reading ${c} rc=$?"
    grep -E '^(TERM|DECIDE)_(AMP_FP|AMP_JE|DIRTY_SHARE|FRAG_SHARE|je_allocated)=' "$OUT/spec373b-${c}.predicates.txt" 2>/dev/null | sed "s/^/  ${c} /" | tee -a "$LOG"
  done
fi
if [ "$SMOKE" = "1" ]; then
  ( cd "$SCRIPT_DIR" && python3 spec373b-shares.py --pin 46dcc12a spec371-c3e.dhat.json.gz spec371-c3l.dhat.json.gz ) > "$OUT/spec373b-selfcheck.txt" 2>&1
  say "shares self-check rc=$? $(grep '^SELF_CHECK=' "$OUT/spec373b-selfcheck.txt")"
  bash "$SCRIPT_DIR/spec373b-synth.sh" "$OUT/synthetic" 2>&1 | tee -a "$LOG"
  say "synth rc=${PIPESTATUS[0]}"
  say "### SMOKE COMPLETE $(date -u +%Y-%m-%dT%H:%M:%SZ)"
fi
say "chain end: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
