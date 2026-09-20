#!/usr/bin/env bash
#
# spec371 memory-diagnosis chain: one detached launch that builds every server
# flavour once, runs the cells in a fixed order through spec371-memdiag.sh, and
# then runs the programs over the finished artifacts, in this order:
#
#   1. record the chain start epoch
#   2. build R, CA, DH and H into fresh, chain-owned target dirs; assert
#      recompiled=yes, mtime >= chain start and the flavour markers; write
#      spec371-builds.txt
#   3. run the cells r0 -> c0 -> c1 -> c2 -> c3e -> c3l, capturing each runner's
#      output as spec371-<cell>.runner-console.log + RUNNER_EXIT=<rc>
#   4. spec371-gref.txt (after r0, whose slope is one of its members)
#   5. spec371-dhat-diff.py -> spec371-dhat-diff.txt
#   6. spec371-predicates.sh for each cell
#   7. spec371-decide.awk -> spec371.decision.txt
#
# A step that fails does not abort the later ones: a missing input surfaces as
# a FALSE or missing predicate, which is a named STOP in the decision.
#
# SPEC371_SMOKE=1 runs the admission smoke instead: the same four builds, three
# short cells (c0 120 s with the live-copy census at 30 s, c3e 60 s, c3l 120 s,
# all at cadence 20), gref over the two committed members only, the dhat diff,
# the predicates, and decide.awk over three SYNTHETIC input sets whose expected
# flags are derived by hand from the spec. Output goes to SPEC365_OUT_DIR,
# which must be a scratch dir.
set -uo pipefail
# Every number this chain or its programs parse must use a '.' decimal point;
# on a comma-decimal host locale awk would otherwise read 144.110 as 144.
export LC_ALL=C

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
SERVER_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd -P)"          # packages/server-rust
REPO_ROOT="$(cd "$SERVER_ROOT/../.." && pwd -P)"
SMOKE="${SPEC371_SMOKE:-0}"

if [ "$SMOKE" = "1" ]; then
  OUT="${SPEC365_OUT_DIR:-}"
  if [ -z "$OUT" ]; then echo "FATAL: smoke needs SPEC365_OUT_DIR (a scratch dir)" >&2; exit 2; fi
  mkdir -p "$OUT"; OUT="$(cd "$OUT" && pwd -P)"
  if [ "$OUT" = "$SCRIPT_DIR" ]; then echo "FATAL: smoke must not write into the evidence dir" >&2; exit 2; fi
  CELLS="c0 c3e c3l"
else
  OUT="$SCRIPT_DIR"
  CELLS="r0 c0 c1 c2 c3e c3l"
fi
LOG="$OUT/spec371-chain.log"
: > "$LOG"
say() { echo "$*" | tee -a "$LOG"; }

# ----------------------------------------------------------------- 1. start
CHAIN_START_EPOCH="$(date +%s)"
say "chain start: $(date -u +%Y-%m-%dT%H:%M:%SZ) epoch=${CHAIN_START_EPOCH} smoke=${SMOKE} HEAD=$(git -C "$REPO_ROOT" rev-parse HEAD)"

if [ -z "${SDKROOT:-}" ] && [ -x /usr/bin/xcrun ]; then
  SDKROOT="$(/usr/bin/xcrun --sdk macosx --show-sdk-path 2>/dev/null || true)"
  [ -n "$SDKROOT" ] && export SDKROOT
fi

# ----------------------------------------------------------------- 2. builds
# Under the repo-root target/, which .gitignore covers: build scripts write
# generated .rs files into a target dir, and an un-ignored one would trip the
# runner's dirty-.rs guard on every cell.
T_ROOT="${REPO_ROOT}/target"
guarded_rm() {   # $1 = candidate target dir; removes it only if it is one of the four chain-owned paths
  local cand parent resolved
  cand="$1"
  [ -n "$cand" ] || { say "FATAL: guarded_rm got an empty path"; exit 1; }
  parent="$(dirname "$cand")"
  mkdir -p "$parent"
  resolved="$(cd "$parent" && pwd -P)/$(basename "$cand")"
  case "$resolved" in
    "${REPO_ROOT}/target/spec371-r"|\
    "${REPO_ROOT}/target/spec371-ca"|\
    "${REPO_ROOT}/target/spec371-dh"|\
    "${REPO_ROOT}/target/spec371-h") rm -rf "$resolved" ;;
    *) say "FATAL: refusing to remove '$resolved': not a chain-owned target dir"; exit 1 ;;
  esac
}

build() {   # $1 = flavour, $2 = target dir, $3.. = cargo args
  local fl="$1" td="$2"; shift 2
  guarded_rm "$td"
  say "build ${fl}: CARGO_TARGET_DIR=${td} cargo build $*"
  ( cd "$SERVER_ROOT" && CARGO_TARGET_DIR="$td" cargo build "$@" ) > "$OUT/spec371-build-${fl}.log" 2>&1
  local rc=$?
  tail -3 "$OUT/spec371-build-${fl}.log" >> "$LOG"
  [ "$rc" -eq 0 ] || { say "FATAL: build ${fl} failed rc=${rc}"; exit 1; }
  if grep -q 'Compiling topgun-server v' "$OUT/spec371-build-${fl}.log"; then echo yes; else echo no; fi > "$OUT/.recompiled-${fl}"
}

build R  "${T_ROOT}/spec371-r"  --release --bin topgun-server
build CA "${T_ROOT}/spec371-ca" --release --features count-alloc --bin topgun-server
build DH "${T_ROOT}/spec371-dh" --profile release-with-debug --features dhat-heap --bin topgun-server
build H  "${T_ROOT}/spec371-h"  --release --bench soak_harness

BIN_R="${T_ROOT}/spec371-r/release/topgun-server"
BIN_CA="${T_ROOT}/spec371-ca/release/topgun-server"
BIN_DH="${T_ROOT}/spec371-dh/release-with-debug/topgun-server"
BIN_H="$(ls -t "${T_ROOT}"/spec371-h/release/deps/soak_harness-* 2>/dev/null | grep -vE '\.(d|o|rcgu)' | head -1 || true)"

hits() { strings "$1" | grep -c "$2" || true; }
BUILDS="$OUT/spec371-builds.txt"
: > "$BUILDS"
for fl in R CA DH H; do
  case "$fl" in R) b="$BIN_R" ;; CA) b="$BIN_CA" ;; DH) b="$BIN_DH" ;; H) b="$BIN_H" ;; esac
  [ -n "$b" ] && [ -x "$b" ] || { say "FATAL: flavour ${fl} binary missing: '${b}'"; exit 1; }
  rec="$(cat "$OUT/.recompiled-${fl}")"; rm -f "$OUT/.recompiled-${fl}"
  mt="$(date -r "$b" '+%s')"
  p="$(hits "$b" 'alloc_probe elapsed_s=')"; d="$(hits "$b" 'DHAT_OUT')"
  case "$fl" in
    R)  [ "$p" -eq 0 ] && [ "$d" -eq 0 ] ;;
    CA) [ "$p" -gt 0 ] && [ "$d" -eq 0 ] ;;
    DH) [ "$d" -gt 0 ] && [ "$p" -eq 0 ] ;;
    H)  [ "$(hits "$b" 'tombstone-byte level ceiling breached')" -gt 0 ] && [ "$(hits "$b" 'soak: child TOPGUN_JOURNAL_ENABLED=')" -gt 0 ] ;;
  esac
  marker=$?
  [ "$rec" = "yes" ] || { say "FATAL: flavour ${fl} recompiled=${rec}"; exit 1; }
  [ "$mt" -ge "$CHAIN_START_EPOCH" ] || { say "FATAL: flavour ${fl} binary predates the chain start"; exit 1; }
  [ "$marker" -eq 0 ] || { say "FATAL: flavour ${fl} marker mismatch (probe=${p} dhat=${d})"; exit 1; }
  echo "flavour=${fl} path=${b} sha256=$(shasum -a 256 "$b" | awk '{print $1}') mtime=${mt} recompiled=${rec} marker=ok" >> "$BUILDS"
done
cat "$BUILDS" >> "$LOG"

# ----------------------------------------------------------------- 3. cells
export SPEC371_CHAIN_START_EPOCH="$CHAIN_START_EPOCH"
export SPEC371_HARNESS_BIN="$BIN_H"
run_cell() {   # $1 = cell
  local c="$1" rc
  case "$c" in r0) SOAK_SERVER_BINARY="$BIN_R" ;; c0|c1|c2) SOAK_SERVER_BINARY="$BIN_CA" ;; c3e|c3l) SOAK_SERVER_BINARY="$BIN_DH" ;; esac
  export SOAK_SERVER_BINARY
  { echo "--- cell ${c} at $(date -u +%Y-%m-%dT%H:%M:%SZ)"; uptime; vm_stat | head -8; } >> "$LOG"
  if [ "$SMOKE" = "1" ]; then
    export SPEC365_DATA_DIR="$OUT/data-${c}"
    export SPEC365_SMOKE_SAMPLE_INTERVAL=20
    case "$c" in
      c0)  export SPEC362B_SMOKE_DURATION=120; export SPEC371_SMOKE_LIVE_CENSUS=30 ;;
      c3e) export SPEC362B_SMOKE_DURATION=60;  unset SPEC371_SMOKE_LIVE_CENSUS ;;
      c3l) export SPEC362B_SMOKE_DURATION=120; unset SPEC371_SMOKE_LIVE_CENSUS ;;
    esac
    export SPEC365_OUT_DIR="$OUT"
  fi
  bash "$SCRIPT_DIR/spec371-memdiag.sh" "$c" > "$OUT/spec371-${c}.runner-console.log" 2>&1
  rc=$?
  echo "RUNNER_EXIT=${rc}" >> "$OUT/spec371-${c}.runner-console.log"
  say "cell ${c}: RUNNER_EXIT=${rc}"
}
for c in $CELLS; do run_cell "$c"; done

# ----------------------------------------------------------------- 4. gref
FIT="$SCRIPT_DIR/spec349c2-fit.awk"
gref_member() {   # $1 = member, $2 = csv, $3 = rows filter ("all" | "le900")
  local tmp="$OUT/.gref-$1.csv" out
  if [ ! -s "$2" ]; then echo "G_ref member=$1 slope=missing source=$(basename "$2")"; return; fi
  if [ "$3" = "le900" ]; then awk -F, 'NR == 1 || $1 + 0 <= 900' "$2" > "$tmp"; else cp "$2" "$tmp"; fi
  out="$(awk -f "$FIT" -v col=phys_footprint_mb -v window=last_half "$tmp" 2>&1)" || out="FIT_ERROR"
  rm -f "$tmp"
  printf '%s\n' "$out" | awk -v m="$1" -v s="$(basename "$2")" -v filt="$3" '
    { for (i = 1; i <= NF; i++) { split($i, kv, "="); f[kv[1]] = kv[2] } }
    END { if (!("slope_mb_per_hour" in f)) print "G_ref member=" m " slope=FIT_ERROR source=" s
          else print "G_ref member=" m " slope=" f["slope_mb_per_hour"] " se=" f["se_mb_per_hour"] " n=" f["n"] " rows=" filt " source=" s }'
}
{
  [ "$SMOKE" = "1" ] || gref_member r0 "$OUT/spec371-r0.csv" all
  gref_member 8e "$SCRIPT_DIR/spec368-plateau4h.csv" le900
  gref_member 8f "$SCRIPT_DIR/spec370-plateau4h.csv" le900
} > "$OUT/spec371-gref.txt"
cat "$OUT/spec371-gref.txt" >> "$LOG"

# ----------------------------------------------------------------- 5. dhat diff
if [ -s "$OUT/spec371-c3e.dhat.json.gz" ] && [ -s "$OUT/spec371-c3l.dhat.json.gz" ]; then
  python3 "$SCRIPT_DIR/spec371-dhat-diff.py" "$OUT/spec371-c3e.dhat.json.gz" "$OUT/spec371-c3l.dhat.json.gz" > "$OUT/spec371-dhat-diff.txt" 2>&1
  say "dhat diff rc=$?"
else
  rm -f "$OUT/spec371-dhat-diff.txt"
  say "dhat diff SKIPPED: a profile is missing (PD will read FALSE reason=no_diff_file)"
fi

# ----------------------------------------------------------------- 6. predicates
for c in $CELLS; do
  bash "$SCRIPT_DIR/spec371-predicates.sh" "$OUT" "spec371-${c}" "$BUILDS" >> "$LOG" 2>&1
done

# ----------------------------------------------------------------- 7. decide
decide() {   # $1 = dir holding the inputs, $2 = output file
  local d="$1" out="$2" missing="" args=""
  for f in spec371-r0.predicates.txt spec371-c0.predicates.txt spec371-c1.predicates.txt \
           spec371-c2.predicates.txt spec371-c3e.predicates.txt spec371-c3l.predicates.txt \
           spec371-gref.txt spec371-dhat-diff.txt; do
    if [ -f "$d/$f" ]; then args="$args $d/$f"; else missing="$missing $f"; fi
  done
  # shellcheck disable=SC2086
  awk -v missing="${missing# }" -f "$SCRIPT_DIR/spec371-decide.awk" $args > "$out"
}

if [ "$SMOKE" != "1" ]; then
  decide "$OUT" "$OUT/spec371.decision.txt"
  say "decision:"; tail -8 "$OUT/spec371.decision.txt" | tee -a "$LOG"
  say "chain end: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  exit 0
fi

# ----------------------------------------------------------------- smoke: synthetic decide cases
# Inputs are pinned here; the expected flags are derived by hand from the spec
# (R8 (e)) and compared by the reader, never by this script.
SYN="$OUT/synthetic"
rm -rf "$SYN"
synthetic_base() {   # $1 = dir: every STOP predicate TRUE, PA=n/a where pre-declared
  local d="$1" c f k
  mkdir -p "$d"
  for c in r0 c0 c1 c2 c3e c3l; do
    f="$d/spec371-$c.predicates.txt"; : > "$f"
    for k in PV PR-crashes PR-class PE PJ PC PM1; do echo "$k=TRUE" >> "$f"; done
    case "$c" in r0|c0|c1|c2) echo "OPS_PER_S=180" >> "$f" ;; esac
    case "$c" in r0|c3e|c3l) echo "PA=n/a reason=synthetic" >> "$f" ;; *) echo "PA=TRUE" >> "$f" ;; esac
    case "$c" in r0|c0|c1|c2) printf 'P5=TRUE\nP6=TRUE\nP7=TRUE\n' >> "$f" ;; esac
    [ "$c" = "c3l" ] && echo "PD=TRUE" >> "$f"
  done
  printf 'G=1000\nG_se=50\nL=600\nL_se=30\nLIVE_LH_mean=100\nAMP_fp_terminal=200\n' >> "$d/spec371-c0.predicates.txt"
  printf 'G=1000\nG_se=50\nL=300\nL_se=30\nLIVE_LH_mean=110\nAMP_fp_terminal=210\n' >> "$d/spec371-c2.predicates.txt"
  printf 'G=700\nG_se=40\nL=100\nL_se=20\nLIVE_LH_mean=95\n' >> "$d/spec371-c1.predicates.txt"
  printf 'AMP_fp_terminal=100\n' >> "$d/spec371-r0.predicates.txt"
  printf 'G_ref member=r0 slope=1000 source=synthetic\nG_ref member=8e slope=1400 source=synthetic\nG_ref member=8f slope=1700 source=synthetic\n' > "$d/spec371-gref.txt"
  printf 'PD-format=TRUE\nPD-sym=TRUE\nPD-crate=TRUE\nC3-top1-lever=591\nLEVER_CONTESTED=FALSE\nC3-top1-all-std=FALSE\n' > "$d/spec371-dhat-diff.txt"
}
synthetic_base "$SYN/S1"
synthetic_base "$SYN/S2"; sed -i '' 's/^P6=TRUE$/P6=FALSE reason=synthetic/' "$SYN/S2/spec371-c2.predicates.txt"
synthetic_base "$SYN/S3"
sed -i '' 's/^G=1000$/G=150/' "$SYN/S3/spec371-c0.predicates.txt" "$SYN/S3/spec371-c2.predicates.txt"
printf 'G_ref member=r0 slope=1500 source=synthetic\nG_ref member=8e slope=1400 source=synthetic\nG_ref member=8f slope=1700 source=synthetic\n' > "$SYN/S3/spec371-gref.txt"
# S4: S3's inputs with the count-alloc cell serving half the release cell's ops.
cp -r "$SYN/S3" "$SYN/S4"
sed -i '' 's/^OPS_PER_S=180$/OPS_PER_S=90/' "$SYN/S4/spec371-c0.predicates.txt"
for s in S1 S2 S3 S4; do
  decide "$SYN/$s" "$SYN/$s.decision.txt"
  say "synthetic ${s}:"; tail -8 "$SYN/$s.decision.txt" | tee -a "$LOG"
done
say "### SMOKE COMPLETE $(date -u +%Y-%m-%dT%H:%M:%SZ)"
