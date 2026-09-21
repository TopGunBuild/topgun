#!/usr/bin/env bash
#
# spec372 Stage-1 chain (screening): one detached launch that builds every
# flavour once, runs the seven 900 s cells through spec372-allocdiag.sh, and
# then runs the programs over the finished artifacts, in this order:
#
#   1. record the chain start epoch
#   2. reclaim this carve's target dirs (guarded_rm), THEN assert >= 30 GB free
#   3. build SYS, JE, MI, CA and H into fresh, chain-owned target dirs; assert
#      recompiled=yes, mtime >= chain start and the flavour markers; write
#      spec372-builds.txt; record each build's wall clock and jemalloc's own C
#      build (cargo --timings) so the budget's one estimated line is measured
#   4. run s1a -> j1a -> m1a -> k1 -> j1b -> m1b -> s1b, recording the host state
#      (uptime, vm_stat, memory_pressure, pmset -g therm) before each cell into
#      this log AND the head of the cell's runner-console.log
#   5. spec372-predicates.sh for each cell
#   6. spec372-k.awk stage1 -> spec372.k-stage1.txt
#   7. spec372-decide.awk stage1 -> spec372.stage1.txt; assert both programs'
#      S1_SURVIVORS agree byte-for-byte (a disagreement is a STOP)
#   8. on S1_SURVIVORS=NONE only: spec372-decide.awk stage2 over the Stage-1
#      artifacts -> spec372.decision.txt; chain 2 is then never launched
#   9. print CHAIN_RC=<rc> as the log's LAST line
#
# A cell that fails does not abort the later ones: a missing input surfaces as
# a FALSE or missing predicate, which is a named STOP in the decision.
#
# SPEC372_SMOKE=1 runs the admission smoke instead, into SPEC365_OUT_DIR (a
# scratch dir): the same five builds, the same seven cells at 120 s (cadence
# 20, census 30), the same programs, and then decide.awk over (a) the full
# product of the printed value domains, checked by an INDEPENDENT awk, (b) one
# out-of-domain input, (c) hand-derived spot checks. Inputs are pinned here;
# every expected output is derived by hand from the spec by the reader, never
# written into this script.
set -uo pipefail
# Every number this chain or its programs parse must use a '.' decimal point;
# on a comma-decimal host locale awk would otherwise read 144.110 as 144.
export LC_ALL=C

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
SERVER_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd -P)"          # packages/server-rust
REPO_ROOT="$(cd "$SERVER_ROOT/../.." && pwd -P)"
SMOKE="${SPEC372_SMOKE:-0}"
CELLS="s1a j1a m1a k1 j1b m1b s1b"

if [ "$SMOKE" = "1" ]; then
  OUT="${SPEC365_OUT_DIR:-}"
  if [ -z "$OUT" ]; then echo "FATAL: smoke needs SPEC365_OUT_DIR (a scratch dir)" >&2; exit 2; fi
  mkdir -p "$OUT"; OUT="$(cd "$OUT" && pwd -P)"
  if [ "$OUT" = "$SCRIPT_DIR" ]; then echo "FATAL: smoke must not write into the evidence dir" >&2; exit 2; fi
else
  OUT="$SCRIPT_DIR"
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
LOG="$OUT/spec372-chain1.log"
: > "$LOG"
say() { echo "$*" | tee -a "$LOG"; }
finish() { say "chain end: $(date -u +%Y-%m-%dT%H:%M:%SZ)"; say "CHAIN_RC=$1"; exit "$1"; }

# ----------------------------------------------------------------- 1. start
CHAIN_START_EPOCH="$(date +%s)"
say "chain start: $(date -u +%Y-%m-%dT%H:%M:%SZ) epoch=${CHAIN_START_EPOCH} smoke=${SMOKE} HEAD=$(git -C "$REPO_ROOT" rev-parse HEAD)"

if [ -z "${SDKROOT:-}" ] && [ -x /usr/bin/xcrun ]; then
  SDKROOT="$(/usr/bin/xcrun --sdk macosx --show-sdk-path 2>/dev/null || true)"
  [ -n "$SDKROOT" ] && export SDKROOT
fi

# ----------------------------------------------------------------- 2. reclaim, then disk
# Under the repo-root target/, which .gitignore covers: build scripts write
# generated .rs files into a target dir, and an un-ignored one would trip the
# runner's dirty-.rs guard on every cell.
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
# Asserted AFTER the reclaim, so space this chain is about to release counts.
DISK_FREE_GB="$(df -g "$T_ROOT" | awk 'NR == 2 { print $4 }')"
say "DISK_FREE_AT_START=${DISK_FREE_GB}"
case "$DISK_FREE_GB" in ''|*[!0-9]*) say "FATAL: free space unreadable"; finish 1 ;; esac
[ "$DISK_FREE_GB" -ge 30 ] || { say "FATAL: ${DISK_FREE_GB} GB free < 30 GB"; finish 1; }

# ----------------------------------------------------------------- 3. builds
build() {   # $1 = flavour, $2 = target dir, $3.. = cargo args
  local fl="$1" td="$2" s rc; shift 2
  say "build ${fl}: CARGO_TARGET_DIR=${td} cargo build $*"
  s="$(date +%s)"
  ( cd "$SERVER_ROOT" && CARGO_TARGET_DIR="$td" cargo build --timings "$@" ) > "$OUT/spec372-build-${fl}.log" 2>&1
  rc=$?
  say "build ${fl}: rc=${rc} wall_s=$(( $(date +%s) - s ))"
  tail -3 "$OUT/spec372-build-${fl}.log" >> "$LOG"
  [ "$rc" -eq 0 ] || { say "FATAL: build ${fl} failed rc=${rc}"; finish 1; }
  if grep -q 'Compiling topgun-server v' "$OUT/spec372-build-${fl}.log"; then echo yes; else echo no; fi > "$OUT/.recompiled-${fl}"
}
build SYS "${T_ROOT}/spec372-sys" --release --bin topgun-server
build JE  "${T_ROOT}/spec372-je"  --release --features alloc-jemalloc --bin topgun-server
build MI  "${T_ROOT}/spec372-mi"  --release --features alloc-mimalloc --bin topgun-server
build CA  "${T_ROOT}/spec372-ca"  --release --features count-alloc --bin topgun-server
build H   "${T_ROOT}/spec372-h"   --release --bench soak_harness
# jemalloc's own ./configure + make is the one budget line with no committed
# precedent; cargo's timing report carries it as the -sys crate's
# "build script (run)" row, whose next table cell is its duration.
JE_C_S="$(awk '/tikv-jemalloc-sys v[^<]* build script \(run\)<\/td>/ { getline; gsub(/<\/?td>|s/, ""); print; exit }' "${T_ROOT}"/spec372-je/cargo-timings/cargo-timing.html 2>/dev/null)"
say "JEMALLOC_C_BUILD_S=${JE_C_S:-n/a}"

BIN_SYS="${T_ROOT}/spec372-sys/release/topgun-server"
BIN_JE="${T_ROOT}/spec372-je/release/topgun-server"
BIN_MI="${T_ROOT}/spec372-mi/release/topgun-server"
BIN_CA="${T_ROOT}/spec372-ca/release/topgun-server"
BIN_H="$(ls -t "${T_ROOT}"/spec372-h/release/deps/soak_harness-* 2>/dev/null | grep -vE '\.(d|o|rcgu)' | head -1 || true)"

# The MI literal was read off the first MI build (the bundled mimalloc's own
# message prefix); SYS is identified by the absence of every other literal.
MI_LIT='mimalloc: warning: '
hits() { strings "$1" | grep -cF "$2" || true; }
BUILDS="$OUT/spec372-builds.txt"
: > "$BUILDS"
for fl in SYS JE MI CA H; do
  case "$fl" in SYS) b="$BIN_SYS" ;; JE) b="$BIN_JE" ;; MI) b="$BIN_MI" ;; CA) b="$BIN_CA" ;; H) b="$BIN_H" ;; esac
  [ -n "$b" ] && [ -x "$b" ] || { say "FATAL: flavour ${fl} binary missing: '${b}'"; finish 1; }
  rec="$(cat "$OUT/.recompiled-${fl}")"; rm -f "$OUT/.recompiled-${fl}"
  mt="$(date -r "$b" '+%s')"
  p="$(hits "$b" 'alloc_probe elapsed_s=')"; d="$(hits "$b" 'DHAT_OUT')"
  j="$(hits "$b" 'je_probe elapsed_s=')"; m="$(hits "$b" "$MI_LIT")"; ba="$(hits "$b" 'bytes_alloc=')"
  case "$fl" in
    SYS) [ "$p" -eq 0 ] && [ "$d" -eq 0 ] && [ "$j" -eq 0 ] && [ "$m" -eq 0 ] ;;
    JE)  [ "$j" -gt 0 ] && [ "$p" -eq 0 ] && [ "$d" -eq 0 ] && [ "$m" -eq 0 ] ;;
    MI)  [ "$m" -gt 0 ] && [ "$p" -eq 0 ] && [ "$d" -eq 0 ] && [ "$j" -eq 0 ] ;;
    CA)  [ "$p" -gt 0 ] && [ "$ba" -gt 0 ] && [ "$d" -eq 0 ] && [ "$j" -eq 0 ] && [ "$m" -eq 0 ] ;;
    H)   [ "$(hits "$b" 'tombstone-byte level ceiling breached')" -gt 0 ] && [ "$(hits "$b" 'soak: child TOPGUN_JOURNAL_ENABLED=')" -gt 0 ] ;;
  esac
  marker=$?
  [ "$rec" = "yes" ] || { say "FATAL: flavour ${fl} recompiled=${rec}"; finish 1; }
  [ "$mt" -ge "$CHAIN_START_EPOCH" ] || { say "FATAL: flavour ${fl} binary predates the chain start"; finish 1; }
  [ "$marker" -eq 0 ] || { say "FATAL: flavour ${fl} marker mismatch (alloc_probe=${p} dhat=${d} je_probe=${j} mimalloc=${m} bytes_alloc=${ba})"; finish 1; }
  echo "flavour=${fl} path=${b} sha256=$(shasum -a 256 "$b" | awk '{print $1}') mtime=${mt} recompiled=${rec} marker=ok" >> "$BUILDS"
done
cat "$BUILDS" >> "$LOG"

# ----------------------------------------------------------------- 4. cells
export SPEC372_CHAIN_START_EPOCH="$CHAIN_START_EPOCH"
export SPEC372_HARNESS_BIN="$BIN_H"
host_state() {
  echo "HOST uptime: $(uptime)"
  echo "HOST memory_pressure: $(memory_pressure 2>/dev/null | tail -1)"
  vm_stat | head -8 | sed 's/^/HOST vm_stat: /'
  pmset -g therm 2>/dev/null | sed 's/^/HOST therm: /'
}
run_cell() {   # $1 = cell
  local c="$1" rc
  case "$c" in s1a|s1b) SOAK_SERVER_BINARY="$BIN_SYS" ;; j1a|j1b) SOAK_SERVER_BINARY="$BIN_JE" ;;
               m1a|m1b) SOAK_SERVER_BINARY="$BIN_MI" ;; k1) SOAK_SERVER_BINARY="$BIN_CA" ;; esac
  export SOAK_SERVER_BINARY
  if [ "$SMOKE" = "1" ]; then
    export SPEC365_DATA_DIR="$OUT/data-${c}"
    export SPEC365_SMOKE_SAMPLE_INTERVAL=20 SPEC362B_SMOKE_DURATION=120 SPEC371_SMOKE_LIVE_CENSUS=30
    export SPEC365_OUT_DIR="$OUT"
  fi
  { echo "--- cell ${c} at $(date -u +%Y-%m-%dT%H:%M:%SZ)"; host_state; } | tee -a "$LOG" > "$OUT/spec372-${c}.runner-console.log"
  bash "$SCRIPT_DIR/spec372-allocdiag.sh" "$c" >> "$OUT/spec372-${c}.runner-console.log" 2>&1
  rc=$?
  echo "RUNNER_EXIT=${rc}" >> "$OUT/spec372-${c}.runner-console.log"
  say "cell ${c}: RUNNER_EXIT=${rc}"
}
for c in $CELLS; do run_cell "$c"; done

# ----------------------------------------------------------------- 5-7. programs
for c in $CELLS; do
  bash "$SCRIPT_DIR/spec372-predicates.sh" "$OUT" "spec372-${c}" "$BUILDS" >> "$LOG" 2>&1
done
k_inputs=""; missing=""
for c in $CELLS; do
  f="spec372-${c}.predicates.txt"
  if [ -f "$OUT/$f" ]; then k_inputs="$k_inputs $OUT/$f"; else missing="$missing $f"; fi
done
[ -f "$OUT/spec372-k1.csv" ] || missing="$missing spec372-k1.csv"
# shellcheck disable=SC2086
awk -v mode=stage1 -f "$SCRIPT_DIR/spec372-k.awk" $k_inputs "$OUT/spec372-k1.csv" > "$OUT/spec372.k-stage1.txt" 2>> "$LOG"
KRC=$?
say "k.awk stage1 rc=${KRC}"
# shellcheck disable=SC2086
awk -v mode=stage1 -v disk_free="$DISK_FREE_GB" -v missing="${missing# }" -f "$SCRIPT_DIR/spec372-decide.awk" \
    $k_inputs "$OUT/spec372.k-stage1.txt" > "$OUT/spec372.stage1.txt" 2>> "$LOG"
DRC=$?
say "decide.awk stage1 rc=${DRC}"
cat "$OUT/spec372.stage1.txt" >> "$LOG"
SV_DECIDE="$(awk -F= '$1 == "S1_SURVIVORS" { print substr($0, index($0, "=") + 1) }' "$OUT/spec372.stage1.txt")"
SV_K="$(awk -F= '$1 == "S1_SURVIVORS" { print substr($0, index($0, "=") + 1) }' "$OUT/spec372.k-stage1.txt")"
STOP1="$(awk -F= '$1 == "STOP" { print $2 }' "$OUT/spec372.stage1.txt")"
say "S1_SURVIVORS decide=${SV_DECIDE} k=${SV_K} STOP=${STOP1}"
CHAIN_RC=0
# A program that failed, or a Stage-1 file without a STOP line or a survivor
# set, is not a result: it must never read as a finished chain.
if [ "$KRC" -ne 0 ] || [ "$DRC" -ne 0 ] || [ -z "$SV_DECIDE" ] || [ -z "$SV_K" ] \
   || { [ "$STOP1" != "TRUE" ] && [ "$STOP1" != "FALSE" ]; }; then
  say "FATAL: a Stage-1 program failed or wrote no result (k rc=${KRC}, decide rc=${DRC}, STOP=${STOP1:-<none>})"
  CHAIN_RC=4
fi
if [ "$STOP1" = "FALSE" ] && [ "$SV_DECIDE" != "$SV_K" ]; then
  say "STOP: decide.awk and k.awk disagree on S1_SURVIVORS"
  CHAIN_RC=3
fi
if [ "$STOP1" = "FALSE" ] && [ "$SV_DECIDE" = "NONE" ] && [ "$CHAIN_RC" -eq 0 ]; then
  awk -v mode=stage2 -v disk_free="$DISK_FREE_GB" -v a2j_state=n/a -f "$SCRIPT_DIR/spec372-decide.awk" \
      "$OUT/spec372.stage1.txt" > "$OUT/spec372.decision.txt" 2>> "$LOG"
  rc=$?
  grep -q '^NEXT=' "$OUT/spec372.decision.txt" || rc=9
  [ "$rc" -eq 0 ] || { say "FATAL: decide.awk stage2 over Stage 1 failed rc=${rc}"; CHAIN_RC=4; }
  say "S1_SURVIVORS=NONE: decision written by decide.awk stage2 over Stage 1; chain 2 is not launched"
  tail -3 "$OUT/spec372.decision.txt" | tee -a "$LOG"
fi

if [ "$SMOKE" != "1" ]; then finish "$CHAIN_RC"; fi

# ----------------------------------------------------------------- smoke: the decision list
SYN="$OUT/synthetic"
rm -rf "$SYN"; mkdir -p "$SYN"

# (a) The full product of the printed value domains, INPUTS ONLY. Per surviving
# arm: every verdict value and every n/a reason, VS_SYS, K_HI_VACUOUS, PERF,
# BUILD; plus the survivor set, STOP, EST_PROVISIONAL, and the AMP_FP pattern
# (either arm lower by more than 10 %, or within 10 %) crossed with whether
# AMP_S orders the arms the same way. A dropped arm carries n/a:dropped_stage1.
say "enumeration: generating, deciding and checking in one stream"
CLOSED_LITERALS="STOP CONDUCTOR_RULING;ALLOCATORS_WORSE_AT_900S CONDUCTOR_RULING;OPS CONDUCTOR_RULING;CELL_DID_NOT_RUN_NA CONDUCTOR_RULING;FEW_POINTS_NA CONDUCTOR_RULING;MISSING_READING_NA CONDUCTOR_RULING;BETTER_NOT_BOUNDED TODO-591;ALLOCATOR_INSUFFICIENT CONDUCTOR_RULING;BOUNDED_NO_GAIN CONDUCTOR_RULING;TREND_MARGINAL CONDUCTOR_RULING;ARM_ORDER_ACCOUNTING CONDUCTOR_RULING;BOUNDED_ABOVE_K CONDUCTOR_RULING;K_VACUOUS CONDUCTOR_RULING;ESTIMATOR_UNVERIFIED CONDUCTOR_RULING;PERF_NA CONDUCTOR_RULING;PERF_VS_MEMORY CONDUCTOR_RULING;BUILD_NA CONDUCTOR_RULING;BUILD_STORY TODO-589;THEN;DEFAULT_FLIP;THEN;TODO-591 CONDUCTOR_RULING;UNMATCHED"
awk 'BEGIN {
  nv = split("PLATEAU BOUNDED_ABOVE_K MARGINAL BOUNDED_NO_GAIN NOT_BOUNDED n/a:ops n/a:few_points n/a:cell_did_not_run n/a:missing_reading", V, " ")
  nb = split("BETTER NO_GAIN", B, " "); nk = split("TRUE FALSE", K, " ")
  np = split("PASS FAIL n/a", PF, " "); nd = split("OK PARTIAL FAIL n/a", BD, " ")
  # AMP_FP patterns (JE,MI) and the AMP_S that agrees / disagrees with each.
  na = split("3.0,5.0 5.0,3.0 3.0,3.2 3.2,3.0", AF, " ")
  ns = split("NONE JE MI JE+MI", SV, " ")
  id = 0
  for (st = 1; st <= 2; st++) for (s = 1; s <= ns; s++) for (e = 1; e <= 2; e++) for (a = 1; a <= na; a++) for (o = 1; o <= 2; o++) {
    split(AF[a], fp, ","); sje = (o == 1) ? fp[1] : fp[2]; smi = (o == 1) ? fp[2] : fp[1]
    base = "STOP=" ((st == 1) ? "FALSE" : "TRUE") " S1_SURVIVORS=" SV[s] " EST_PROVISIONAL=" K[e] " AMP_FP_JE=" fp[1] " AMP_FP_MI=" fp[2] " AMP_S_JE=" sje " AMP_S_MI=" smi
    je = (SV[s] == "JE" || SV[s] == "JE+MI"); mi = (SV[s] == "MI" || SV[s] == "JE+MI")
    nj = je ? nv * nb * nk * np * nd : 1; nm = mi ? nv * nb * nk * np * nd : 1
    for (x = 0; x < nj; x++) for (y = 0; y < nm; y++) {
      print "ID=" (++id) " " base " " arm("JE", je, x) " " arm("MI", mi, y)
    }
  }
}
function arm(n, surv, x,   v, b, k, p, d) {
  if (!surv) return "VERDICT_" n "=n/a:dropped_stage1 VS_SYS_" n "=n/a K_HI_VACUOUS_" n "=n/a PERF_" n "=n/a BUILD_" n "=n/a"
  v = x % nv; x = int(x / nv); b = x % nb; x = int(x / nb); k = x % nk; x = int(x / nk); p = x % np; d = int(x / np)
  return "VERDICT_" n "=" V[v + 1] " VS_SYS_" n "=" B[b + 1] " K_HI_VACUOUS_" n "=" K[k + 1] " PERF_" n "=" PF[p + 1] " BUILD_" n "=" BD[d + 1]
}' \
| awk -v mode=list -f "$SCRIPT_DIR/spec372-decide.awk" \
| awk -v lits="$CLOSED_LITERALS" '
  # The INDEPENDENT checker -- not decide.awk. It recomputes DEFAULT_CANDIDATE
  # from the three steps of R8 and checks the four assertions. Inputs arrive in
  # generation order, so "one output per input" is "IDs run 1, 2, 3, ... gap-free".
  function rk(v) { return (v == "PLATEAU") ? 1 : (v == "BOUNDED_ABOVE_K") ? 2 : (v == "MARGINAL") ? 3 : (v == "BOUNDED_NO_GAIN") ? 4 : (v == "NOT_BOUNDED") ? 5 : 6 }
  BEGIN { n = split(lits, l, " "); for (i = 1; i <= n; i++) ok[l[i]] = 1 }
  { delete f; nn = 0
    for (i = 1; i <= NF; i++) { p = index($i, "="); k = substr($i, 1, p - 1); f[k] = substr($i, p + 1); if (k == "NEXT") nn++ }
    rows++; if (nn != 1) multi++; if (f["ID"] + 0 != rows) order++
    je = (f["S1_SURVIVORS"] ~ /JE/); mi = (f["S1_SURVIVORS"] ~ /MI/)
    rj = je ? rk(f["VERDICT_JE"]) : 9; rm = mi ? rk(f["VERDICT_MI"]) : 9; b = (rj < rm) ? rj : rm
    fj = f["AMP_FP_JE"] + 0; fm = f["AMP_FP_MI"] + 0; sj = f["AMP_S_JE"] + 0; sm = f["AMP_S_MI"] + 0
    c = "NONE"
    if (f["STOP"] == "TRUE") c = "STOP"
    else if (f["S1_SURVIVORS"] != "NONE" && b <= 2) {
      if (rj == b && rm != b) c = "JE"; else if (rm == b && rj != b) c = "MI"
      else if (!((fj < fm && sj > sm) || (fj > fm && sj < sm))) c = ((fj > fm ? fj - fm : fm - fj) <= 0.10 * (fj > fm ? fj : fm)) ? "JE" : ((fj < fm) ? "JE" : "MI")
    }
    if (f["NEXT"] == "CONDUCTOR_RULING;UNMATCHED") unm++
    if (!(f["NEXT"] in ok)) { badlit++; if (badlit < 4) print "not in the closed list: " $0 }
    if (f["DEFAULT_CANDIDATE"] != c) { badc++; if (badc < 4) print "candidate mismatch want=" c ": " $0 }
    cnt[f["NEXT"]]++ }
  END { printf "ENUM inputs=%d one_next_per_input=%s unmatched=%d candidate_mismatches=%d literals_outside_closed_list=%d\n", rows, (multi + order == 0 ? "TRUE" : "FALSE"), unm + 0, badc + 0, badlit + 0
        for (k in cnt) printf "ENUM NEXT=%s count=%d\n", k, cnt[k] }' | sort | tee -a "$LOG" > "$SYN/enum.summary"
# The enumeration is the smoke's executable proof of totality, so its
# assertions gate the smoke's exit code; the spot checks are read by hand.
grep -q '^ENUM inputs=[1-9][0-9]* one_next_per_input=TRUE unmatched=0 candidate_mismatches=0 literals_outside_closed_list=0$' "$SYN/enum.summary" \
  || { say "SMOKE FAIL: an enumeration assertion failed"; CHAIN_RC=5; }

# (b) One explicitly OUT-OF-DOMAIN input: a corrupted verdict token.
echo "ID=ood STOP=FALSE S1_SURVIVORS=JE EST_PROVISIONAL=FALSE AMP_FP_JE=3.0 AMP_FP_MI=5.0 AMP_S_JE=3.0 AMP_S_MI=5.0 VERDICT_JE=PLATEUA VS_SYS_JE=BETTER K_HI_VACUOUS_JE=FALSE PERF_JE=PASS BUILD_JE=OK VERDICT_MI=n/a:dropped_stage1" \
  | awk -v mode=list -f "$SCRIPT_DIR/spec372-decide.awk" | sed 's/^/OOD /' | tee -a "$LOG" > "$SYN/ood.txt"
grep -q '^OOD DEFAULT_CANDIDATE=NONE NEXT=CONDUCTOR_RULING;UNMATCHED ' "$SYN/ood.txt" \
  || { say "SMOKE FAIL: the out-of-domain input did not reach the catch-all"; CHAIN_RC=5; }

# (c) Hand-derived spot checks, through the FULL stage-2 path (predicates-shaped
# files -> k.awk stage2 -> decide.awk stage2). Inputs only; the reader derives
# each expected flag from the spec.
spot_base() {   # $1 = dir; every STOP predicate TRUE; one survivor pair; PERF/BUILD pass
  local d="$1" c
  mkdir -p "$d"
  cat > "$d/spec372.stage1.txt" <<'EOF'
STOP=FALSE
WRITE_ERRORS=0
OPS_RATIO_JE_S1=1.0000
OPS_RATIO_MI_S1=1.0000
S1_SURVIVORS=JE+MI
S1_RANK=JE
CHURN_RATIO=0.9500 points=29 window_s=450-900
CHURN_RATIO_DRIFT=1.0000
K_PROVISIONAL=FALSE
T_DECAY_UPPER_JE=9.5000
T_DECAY_UPPER_MI=0.9500
CV_JE=0.1000
CV_MI=0.1000
DIRTY_SHARE_j1a=0.050000
DIRTY_SHARE_j1b=0.050000
FRAG_SHARE_j1a=0.100000
FRAG_SHARE_j1b=0.100000
EST_AGREE_900_j1a=1.1000
EST_AGREE_900_j1b=1.1000
UNMODELLED_REACHABLE_j1a=6.000 0.100000
UNMODELLED_REACHABLE_j1b=6.000 0.100000
EOF
  for c in s2 j2 m2 a2j; do
    {
      for k in PV PR-crashes PR-class P5 P6 P7 PE PJ PC PM1; do echo "$k=TRUE"; done
      case "$c" in s2|m2) echo "PA=n/a reason=flavour" ;; *) echo "PA=TRUE" ;; esac
      echo "WRITE_ERRORS=0"; echo "OPS_PER_S=180.000"; echo "OPS_AT_900=180.000 checkpoint_elapsed=903"
      echo "TREND=0.001000"; echo "TREND_se=0.010000"; echo "TREND_n=24"; echo "TREND_r2=0.01"
      echo "TREND3=0.001000"; echo "TREND3_se=0.010000"; echo "TREND3_n=16"; echo "TREND3_r2=0.01"
      echo "TREND_dropped=0"; echo "TREND_points_used=48"; echo "RECLAIM_RATIO_end=0.500000"
      echo "DECIDE_src=TERMINAL"; echo "DECIDE_A0_share=0.018500"; echo "DECIDE_R_redb=0.240000"
    } > "$d/spec372-${c}.predicates.txt"
  done
  printf 'DECIDE_AMP_FP=9.000000\nDECIDE_AMP_S=10.000000\n' >> "$d/spec372-s2.predicates.txt"
  printf 'DECIDE_AMP_FP=1.400000\nDECIDE_AMP_S=1.500000\nDECIDE_R_meta=0.020000\nDECIDE_EST_AGREE=1.050000\nDECIDE_AMP_JE=1.100000\nDECIDE_DIRTY_SHARE=0.050000\nDECIDE_FRAG_SHARE=0.100000\nDECIDE_UNMODELLED_MB=45.000 0.050000\nDECIDE_UNMODELLED_SHARE=0.050000\nTREND_NATIVE=0.001\nTREND_NATIVE_se=0.01\nTREND_NATIVE_n=120\nJE_CONFIG=je_config version=synthetic\n' >> "$d/spec372-j2.predicates.txt"
  printf 'DECIDE_AMP_FP=2.000000\nDECIDE_AMP_S=2.200000\n' >> "$d/spec372-m2.predicates.txt"
  printf 'DECIDE_AMP_FP=1.500000\nDECIDE_AMP_S=1.600000\n' >> "$d/spec372-a2j.predicates.txt"
  printf 'PERF_JE=PASS\nPERF_MI=PASS\n' > "$d/spec372-perf.txt"
  printf 'BUILD_JE=OK\nBUILD_MI=OK\nSIZE_JE=100000\nSIZE_HEADROOM_JE=OK\nSIZE_MI=50000\nSIZE_HEADROOM_MI=OK\n' > "$d/spec372-buildstory.txt"
}
setkey() {   # $1 = file, $2 = key, $3 = value: replace the key's line, or append it
  local f="$1" k="$2" v="$3"
  if grep -q "^${k}=" "$f"; then
    awk -v k="$k" -v v="$v" 'index($0, k "=") == 1 { print k "=" v; next } { print }' "$f" > "$f.tmp" && mv "$f.tmp" "$f"
  else echo "${k}=${v}" >> "$f"; fi
}
spot_run() {   # $1 = case name
  local d="$SYN/$1" files="" extra="" c f
  for c in s2 j2 m2 a2j; do [ -f "$d/spec372-${c}.predicates.txt" ] && files="$files $d/spec372-${c}.predicates.txt"; done
  for f in spec372-perf.txt spec372-buildstory.txt; do [ -f "$d/$f" ] && extra="$extra $d/$f"; done
  # shellcheck disable=SC2086
  awk -v mode=stage2 -f "$SCRIPT_DIR/spec372-k.awk" "$d/spec372.stage1.txt" $files > "$d/spec372.k-stage2.txt"
  # shellcheck disable=SC2086
  awk -v mode=stage2 -v disk_free=100 -v a2j_state="${SPOT_A2J:-ran}" -v a2j_flavour=JE -f "$SCRIPT_DIR/spec372-decide.awk" \
      "$d/spec372.stage1.txt" $files "$d/spec372.k-stage2.txt" $extra > "$d/decision.txt"
  say "spot $1: $(grep -E '^(STOP|VERDICT_JE|VERDICT_MI|DEFAULT_CANDIDATE|NEXT)=' "$d/decision.txt" | tr '\n' ' ')"
}
spot() { rm -rf "${SYN:?}/$1"; spot_base "$SYN/$1"; }

spot T01_plateau;            spot_run T01_plateau
spot T02_bounded_above_k;    setkey "$SYN/T02_bounded_above_k/spec372-j2.predicates.txt" DECIDE_AMP_FP 3.000000; setkey "$SYN/T02_bounded_above_k/spec372-j2.predicates.txt" DECIDE_AMP_S 3.100000; setkey "$SYN/T02_bounded_above_k/spec372-m2.predicates.txt" DECIDE_AMP_FP 6.000000; spot_run T02_bounded_above_k
spot T03_marginal;           for c in j2 m2; do setkey "$SYN/T03_marginal/spec372-$c.predicates.txt" TREND 0.015000; done; spot_run T03_marginal
spot T04_bounded_no_gain;    setkey "$SYN/T04_bounded_no_gain/spec372-j2.predicates.txt" DECIDE_AMP_FP 5.800000; setkey "$SYN/T04_bounded_no_gain/spec372-m2.predicates.txt" DECIDE_AMP_FP 6.000000; spot_run T04_bounded_no_gain
spot T05_not_bounded;        for c in j2 m2; do setkey "$SYN/T05_not_bounded/spec372-$c.predicates.txt" TREND 0.050000; setkey "$SYN/T05_not_bounded/spec372-$c.predicates.txt" DECIDE_AMP_FP 6.000000; done; spot_run T05_not_bounded
spot T06_not_bounded_better; for c in j2 m2; do setkey "$SYN/T06_not_bounded_better/spec372-$c.predicates.txt" TREND 0.050000; done; spot_run T06_not_bounded_better
spot T07_stop;               setkey "$SYN/T07_stop/spec372-j2.predicates.txt" P6 "FALSE reason=synthetic"; spot_run T07_stop
spot T08_ops;                setkey "$SYN/T08_ops/spec372-j2.predicates.txt" OPS_PER_S 100.000; setkey "$SYN/T08_ops/spec372-m2.predicates.txt" OPS_PER_S 100.000; spot_run T08_ops
spot T09_few_points;         for c in j2 m2; do setkey "$SYN/T09_few_points/spec372-$c.predicates.txt" TREND_n 8; done; spot_run T09_few_points
spot T10_k_vacuous;          setkey "$SYN/T10_k_vacuous/spec372-j2.predicates.txt" DECIDE_R_redb 3.500000; spot_run T10_k_vacuous
spot T11_est_provisional;    setkey "$SYN/T11_est_provisional/spec372-j2.predicates.txt" DECIDE_EST_AGREE 1.400000; spot_run T11_est_provisional
spot T12_perf_na;            rm -f "$SYN/T12_perf_na/spec372-perf.txt"; spot_run T12_perf_na
spot T13_build_na;           rm -f "$SYN/T13_build_na/spec372-buildstory.txt"; spot_run T13_build_na
spot T14_two_survivors_mi;   for c in j2 m2; do setkey "$SYN/T14_two_survivors_mi/spec372-$c.predicates.txt" DECIDE_R_redb 1.000000; done; setkey "$SYN/T14_two_survivors_mi/spec372-j2.predicates.txt" DECIDE_AMP_FP 2.000000; setkey "$SYN/T14_two_survivors_mi/spec372-j2.predicates.txt" DECIDE_AMP_S 2.300000; setkey "$SYN/T14_two_survivors_mi/spec372-m2.predicates.txt" DECIDE_AMP_FP 1.200000; setkey "$SYN/T14_two_survivors_mi/spec372-m2.predicates.txt" DECIDE_AMP_S 1.300000; spot_run T14_two_survivors_mi
spot T15_accounting;         setkey "$SYN/T15_accounting/spec372-j2.predicates.txt" DECIDE_AMP_FP 3.000000; setkey "$SYN/T15_accounting/spec372-j2.predicates.txt" DECIDE_AMP_S 3.500000; setkey "$SYN/T15_accounting/spec372-m2.predicates.txt" DECIDE_AMP_FP 3.600000; setkey "$SYN/T15_accounting/spec372-m2.predicates.txt" DECIDE_AMP_S 3.200000; setkey "$SYN/T15_accounting/spec372-j2.predicates.txt" DECIDE_R_redb 2.000000; setkey "$SYN/T15_accounting/spec372-m2.predicates.txt" DECIDE_R_redb 2.500000; spot_run T15_accounting
spot T16_mixed_na;           setkey "$SYN/T16_mixed_na/spec372-j2.predicates.txt" OPS_PER_S 100.000; setkey "$SYN/T16_mixed_na/spec372-m2.predicates.txt" TREND_n 8; spot_run T16_mixed_na
spot T17_single_survivor_ops; setkey "$SYN/T17_single_survivor_ops/spec372.stage1.txt" S1_SURVIVORS MI; setkey "$SYN/T17_single_survivor_ops/spec372.stage1.txt" S1_RANK MI; rm -f "$SYN/T17_single_survivor_ops/spec372-j2.predicates.txt"; setkey "$SYN/T17_single_survivor_ops/spec372-m2.predicates.txt" OPS_PER_S 100.000; SPOT_A2J=budget spot_run T17_single_survivor_ops
# S1_SURVIVORS=NONE through decide.awk stage2 over a Stage-1 file alone, as chain 1 runs it.
spot T18_none; setkey "$SYN/T18_none/spec372.stage1.txt" S1_SURVIVORS NONE; setkey "$SYN/T18_none/spec372.stage1.txt" S1_RANK "n/a reason=no_survivor"
awk -v mode=stage2 -v disk_free=100 -v a2j_state=n/a -f "$SCRIPT_DIR/spec372-decide.awk" "$SYN/T18_none/spec372.stage1.txt" > "$SYN/T18_none/decision.txt"
say "spot T18_none: $(grep -E '^(STOP|VERDICT_JE|VERDICT_MI|DEFAULT_CANDIDATE|NEXT)=' "$SYN/T18_none/decision.txt" | tr '\n' ' ')"

say "### SMOKE COMPLETE $(date -u +%Y-%m-%dT%H:%M:%SZ)"
finish "$CHAIN_RC"
