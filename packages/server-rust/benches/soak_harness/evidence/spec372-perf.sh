#!/usr/bin/env bash
#
# spec372 load-harness arm matrix. The allocator sits on every hot path, so
# each arm is measured against the System allocator in the same session, on an
# idle host, at CI's perf-gate shape (.github/workflows/rust.yml) lengthened to
# the harness default duration, because 15 s runs were noise-limited:
#   fire-and-wait:   --scenario throughput --connections 200 --duration 30 --interval 50
#   fire-and-forget: --scenario throughput --connections 200 --duration 30 --interval 0 --fire-and-forget
# 5 runs of each per block. Block order SYS, JE, MI, SYS: the two SYS blocks
# bracket the arms, and their spread is the range an arm is compared with. One
# run at CI's own 15 s fire-and-wait parameters is recorded per block, the
# reference the pinned shape came from; it never gates.
#
# Verdict, per arm, against SYS's RANGE (the two SYS blocks' medians):
#   PERF_<arm>=FAIL  iff the arm's median ops/s < 0.80 x the LOWER SYS block
#                    median in either mode, or its fire-and-wait median p99 >
#                    1.20 x the HIGHER SYS block median;
#   PERF_<arm>=n/a   iff the two SYS blocks' median ops/s differ by >= 20 % of
#                    the smaller in either mode (a harness that noisy cannot
#                    disqualify anything), or a run failed to report;
#   PERF_<arm>=PASS  otherwise.
# p50 and the fire-and-forget p99 are recorded; the SYS-vs-SYS p99 spread is
# printed and gates nothing.
# The load harness serves in-process, so the arm's allocator reaches it only
# through the allocator lattice in benches/load_harness/main.rs.
#
# SPEC372_SMOKE=1: one run of each, 5 s each, into SPEC365_OUT_DIR (scratch).
set -uo pipefail
export LC_ALL=C

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
SERVER_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd -P)"
REPO_ROOT="$(cd "$SERVER_ROOT/../.." && pwd -P)"
SMOKE="${SPEC372_SMOKE:-0}"
if [ "$SMOKE" = "1" ]; then
  OUT="${SPEC365_OUT_DIR:-}"
  [ -n "$OUT" ] || { echo "FATAL: smoke needs SPEC365_OUT_DIR (a scratch dir)" >&2; exit 2; }
  mkdir -p "$OUT"; OUT="$(cd "$OUT" && pwd -P)"
  [ "$OUT" != "$SCRIPT_DIR" ] || { echo "FATAL: smoke must not write into the evidence dir" >&2; exit 2; }
  RUNS=1; DUR=5; DEF_DUR=5
else
  OUT="$SCRIPT_DIR"; RUNS=5; DUR=30; DEF_DUR=15
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
TXT="$OUT/spec372-perf.txt"
RAW="$OUT/.spec372-perf-raw"; rm -rf "$RAW"; mkdir -p "$RAW"
: > "$TXT"
say() { echo "$*" | tee -a "$TXT"; }

if [ -z "${SDKROOT:-}" ] && [ -x /usr/bin/xcrun ]; then
  SDKROOT="$(/usr/bin/xcrun --sdk macosx --show-sdk-path 2>/dev/null || true)"
  [ -n "$SDKROOT" ] && export SDKROOT
fi
say "perf start: $(date -u +%Y-%m-%dT%H:%M:%SZ) smoke=${SMOKE} HEAD=$(git -C "$REPO_ROOT" rev-parse HEAD) runs=${RUNS} duration=${DUR}"
say "host: $(uptime)"
say "baseline.json: $(tr -d ' \n' < "$SERVER_ROOT/benches/load_harness/baseline.json" | sed 's/"vector_search".*//')"

T_ROOT="${REPO_ROOT}/target"
features() { case "$1" in SYS) echo "" ;; JE) echo "--features alloc-jemalloc" ;; MI) echo "--features alloc-mimalloc" ;; esac; }
tdir() { case "$1" in SYS) echo "$T_ROOT/spec372-perf-sys" ;; JE) echo "$T_ROOT/spec372-perf-je" ;; MI) echo "$T_ROOT/spec372-perf-mi" ;; esac; }
for arm in SYS JE MI; do
  # shellcheck disable=SC2046
  ( cd "$SERVER_ROOT" && CARGO_TARGET_DIR="$(tdir $arm)" cargo bench --bench load_harness $(features $arm) --no-run ) > "$RAW/build-$arm.log" 2>&1
  say "build arm=${arm} rc=$? target=$(tdir $arm)"
done

run() {   # $1 arm, $2 block, $3 mode (faw|faf|default), $4 run index
  local arm="$1" blk="$2" mode="$3" n="$4" json="$RAW/$1-$2-$3-$4.json" args rc
  case "$mode" in
    faw) args="--scenario throughput --connections 200 --duration $DUR --interval 50" ;;
    faf) args="--scenario throughput --connections 200 --duration $DUR --interval 0 --fire-and-forget" ;;
    ci15) args="--scenario throughput --connections 200 --duration $DEF_DUR --interval 50" ;;
  esac
  rm -f "$json"
  # shellcheck disable=SC2046,SC2086
  ( cd "$SERVER_ROOT" && CARGO_TARGET_DIR="$(tdir $arm)" cargo bench --bench load_harness $(features $arm) -- $args --json-output "$json" ) > "$RAW/$arm-$blk-$mode-$n.log" 2>&1
  rc=$?
  # A report missing a field (jq prints "null") is a run that failed to report,
  # never a zero that would read as a regression.
  local o p50 p99
  o="$(jq -r '.ops_per_sec' "$json" 2>/dev/null)"; p50="$(jq -r '.latency.p50_us' "$json" 2>/dev/null)"; p99="$(jq -r '.latency.p99_us' "$json" 2>/dev/null)"
  if [ -s "$json" ] && [[ "$o" =~ ^[0-9]+(\.[0-9]+)?$ ]] && [[ "$p99" =~ ^[0-9]+$ ]]; then
    say "RUN arm=${arm} block=${blk} mode=${mode} run=${n} ops_per_sec=${o} p50_us=${p50} p99_us=${p99} total_ops=$(jq -r '.total_ops' "$json") rc=${rc}"
  else
    say "RUN arm=${arm} block=${blk} mode=${mode} run=${n} ops_per_sec=n/a p50_us=n/a p99_us=n/a rc=${rc}"
  fi
}
block() {   # $1 arm, $2 block number
  local i
  for i in $(seq 1 "$RUNS"); do run "$1" "$2" faw "$i"; done
  for i in $(seq 1 "$RUNS"); do run "$1" "$2" faf "$i"; done
  run "$1" "$2" ci15 1
}
block SYS 1; block JE 1; block MI 1; block SYS 2

# Medians, ratios and the verdict, from the RUN lines above only (a snapshot,
# so the program never reads the file it is appending to).
grep '^RUN ' "$TXT" > "$RAW/runs.txt"
awk '
  function med(a, n,   i, j, t) { for (i = 2; i <= n; i++) { t = a[i]; for (j = i - 1; j >= 1 && a[j] > t; j--) a[j + 1] = a[j]; a[j + 1] = t }
                                  return (n % 2) ? a[(n + 1) / 2] : (a[n / 2] + a[n / 2 + 1]) / 2 }
  /^RUN / { delete f; for (i = 2; i <= NF; i++) { split($i, kv, "="); f[kv[1]] = kv[2] }
    if (f["mode"] == "ci15") next
    g = f["arm"] SUBSEP f["block"] SUBSEP f["mode"]
    if (f["ops_per_sec"] == "n/a") { bad[f["arm"]] = 1; if (f["arm"] == "SYS") bad["JE"] = bad["MI"] = 1; next }
    n[g]++; ops[g, n[g]] = f["ops_per_sec"] + 0; p99[g, n[g]] = f["p99_us"] + 0 }
  function m(arm, blk, mode, which,   k, i, a) { k = arm SUBSEP blk SUBSEP mode; for (i = 1; i <= n[k]; i++) a[i] = (which == "ops") ? ops[k, i] : p99[k, i]; return (n[k] ? med(a, n[k]) : "") }
  END {
    split("faw faf", modes, " "); noisy = 0
    for (mi = 1; mi <= 2; mi++) {
      md = modes[mi]
      for (b = 1; b <= 2; b++) printf "MEDIAN arm=SYS block=%d mode=%s ops_per_sec=%s p99_us=%s\n", b, md, m("SYS", b, md, "ops"), m("SYS", b, md, "p99")
      o1 = m("SYS", 1, md, "ops"); o2 = m("SYS", 2, md, "ops"); q1 = m("SYS", 1, md, "p99"); q2 = m("SYS", 2, md, "p99")
      if (o1 == "" || o2 == "" || o1 <= 0 || o2 <= 0) { noisy = 1; continue }
      lo[md] = (o1 < o2) ? o1 : o2; hi[md] = (o1 > o2) ? o1 : o2
      phi[md] = (q1 > q2) ? q1 : q2
      do_ = (hi[md] - lo[md]) / lo[md]
      dq = (q1 > 0 && q2 > 0) ? ((q2 > q1 ? q2 - q1 : q1 - q2) / (q1 < q2 ? q1 : q2)) : 0
      printf "SYS_RANGE mode=%s ops_lo=%s ops_hi=%s ops_spread=%.4f p99_hi=%s p99_spread=%.4f (p99 spread recorded, not gated)\n", md, lo[md], hi[md], do_, phi[md], dq
      if (do_ >= 0.20) noisy = 1
    }
    print "SYS_QUIET=" (noisy ? "FALSE" : "TRUE")
    for (ai = 1; ai <= 2; ai++) {
      arm = (ai == 1) ? "JE" : "MI"; fail = 0; why = ""
      for (mi = 1; mi <= 2; mi++) {
        md = modes[mi]; ao = m(arm, 1, md, "ops"); ap = m(arm, 1, md, "p99")
        printf "MEDIAN arm=%s block=1 mode=%s ops_per_sec=%s p99_us=%s\n", arm, md, ao, ap
        if (ao == "" || !(md in lo)) { bad[arm] = 1; continue }
        ro = ao / lo[md]
        printf "RATIO arm=%s mode=%s ops_vs_sys_lo=%.4f", arm, md, ro
        if (ro < 0.80) { fail = 1; why = why " " md "_ops" }
        if (md == "faw") {
          if (phi[md] > 0) { rp = ap / phi[md]; printf " p99_vs_sys_hi=%.4f", rp; if (rp > 1.20) { fail = 1; why = why " faw_p99" } }
          else printf " p99_vs_sys_hi=n/a"
        }
        printf "\n"
      }
      if (bad[arm]) print "PERF_" arm "=n/a reason=run_failed"
      else if (noisy) print "PERF_" arm "=n/a reason=harness_noisy"
      else print "PERF_" arm "=" (fail ? "FAIL" : "PASS") (why != "" ? " breached=" substr(why, 2) : "")
    }
  }' "$RAW/runs.txt" | tee -a "$TXT"
say "perf end: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
