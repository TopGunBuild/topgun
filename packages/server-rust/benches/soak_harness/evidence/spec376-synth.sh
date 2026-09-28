#!/usr/bin/env bash
#
# Synthetic cases for the Linux soak instrument's own programs: the memory
# sampler (spec376-procmem.sh), the per-cell predicates
# (spec376-predicates.sh), the calibration reading (spec376-calib.sh) and the
# host preflight's refusal path (spec376-preflight.sh). Every input is built
# here from pinned literals (or read from a committed file, named at the
# case); every expectation is written here; every comparison is mechanical.
#
# usage: spec376-synth.sh <SCRATCH_DIR>
#   SCRATCH_DIR must not be the evidence dir or lie under it, and must be
#   absent or empty (a leftover from an earlier run could otherwise be read as
#   this run's output).
#
# Output: one "CASE <id> PASS|FAIL" line per case, then exactly one column-0
#   SYNTH376=PASS cases=<n>   or   SYNTH376=FAIL failed=<ids>
# SYNTH376=PASS iff every case below printed exactly one "CASE <id> PASS" line.
# Exit 0 on PASS, 1 on FAIL, 2 on usage or a refused scratch dir.
#
# WHY EVERY ECHOED LINE IS PREFIXED "  | ": this stdout is tee'd into the
# smoke chain log, where the smoke admission and the calibration reading
# count column-0 KEY= lines (SMOKE_ADMISSION=, SYNTH_PARITY=, SYNTH376=,
# INSTRUMENT=). A synthetic calib run prints INSTRUMENT= lines of its own, so
# nothing this file echoes from a fixture or a program under test may start
# at column 0; the only column-0 KEY= line is the final SYNTH376= verdict.
#
# Cases (the spec's R8 table):
#   M1-M5  procmem_row over the committed spec376-fixtures/ proc roots.
#   P1-P4  spec376-predicates.sh over one synthetic 900 s CA cell; P3 runs the
#          FROZEN spec372-predicates.sh over the same cell (fails closed on a
#          Linux CSV).
#   C1-C14 C1 = four good synthetic cal cells (c1 pb c2 pa), a preflight log
#          ending PREFLIGHT=PASS, a smoke log with SMOKE_ADMISSION=PASS and a
#          portability ratio in band; every other C case is C1 with one edit.
#          Every C case except C10 runs the FULL pipeline: spec376-predicates.sh
#          over the four cells with a synthetic builds file, the cal chain log
#          written from the predicates' real exit statuses, then
#          spec376-calib.sh. The edit is applied to the predicates' inputs
#          unless the case names a predicates output or the chain log.
#          C10 runs only the rate program calib extracts from the frozen
#          spec373b-verdict.sh, over the committed spec373b-{b1,b2,a1,a2}
#          cells, against the committed spec373b.verdict.txt.
#   F1     spec376-preflight.sh --apply on stubs with a wrong hostname:
#          refused, nothing changed.
set -uo pipefail
export LC_ALL=C

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
SCR="${1:-}"
if [ "$#" -ne 1 ] || [ -z "$SCR" ]; then
  echo "usage: spec376-synth.sh <SCRATCH_DIR>" >&2; exit 2
fi
# The physical path of $1 without creating anything: the nearest existing
# ancestor resolved with pwd -P, plus the not-yet-existing tail.
canon() {
  local p="$1" tail=""
  case "$p" in /*) ;; *) p="$PWD/$p" ;; esac
  while [ ! -d "$p" ]; do tail="/$(basename "$p")${tail}"; p="$(dirname "$p")"; done
  printf '%s%s\n' "$(cd "$p" && pwd -P)" "$tail"
}
case "$(canon "$SCR")/" in
  "$SCRIPT_DIR"/*) echo "FATAL: the scratch dir must not be the evidence dir or under it" >&2; exit 2 ;;
esac
if [ -d "$SCR" ] && [ -n "$(ls -A "$SCR" 2>/dev/null)" ]; then
  echo "FATAL: the scratch dir ${SCR} is not empty" >&2; exit 2
fi
mkdir -p "$SCR" || { echo "FATAL: cannot create ${SCR}" >&2; exit 2; }
SCR="$(cd "$SCR" && pwd -P)"
mkdir -p "$SCR/tmp"
# calib keeps its intermediate under TMPDIR and refuses one inside the
# evidence dir; the scratch dir is outside it by the check above.
export TMPDIR="$SCR/tmp"
# The environment must not steer a case: the estimator constants, the
# manifest commit and the synthetic switch are set per call, never inherited.
unset SPEC376_A0_MIB SPEC376_B_LIVE SPEC376_MANIFEST_COMMIT SPEC376_SYNTHETIC

PROCMEM="$SCRIPT_DIR/spec376-procmem.sh"
PRED="$SCRIPT_DIR/spec376-predicates.sh"
CALIB="$SCRIPT_DIR/spec376-calib.sh"
PREFLIGHT="$SCRIPT_DIR/spec376-preflight.sh"
PRED372="$SCRIPT_DIR/spec372-predicates.sh"
FIXTURES="$SCRIPT_DIR/spec376-fixtures"
MANIFEST_REAL="$SCRIPT_DIR/spec376-manifest.md"
CASES="M1 M2 M3 M4 M5 P1 P2 P3 P4 C1 C2 C3 C4 C5 C6 C7 C8 C9 C10 C11 C12 C13 C14 F1"

show() { sed 's/^/  | /'; }
# One CASE line per case; $2 = 0 for PASS; $3 = the reason on FAIL.
RESULTS="$SCR/results.txt"; : > "$RESULTS"
verdict() {
  if [ "$2" -eq 0 ]; then echo "CASE $1 PASS"; echo "CASE $1 PASS" >> "$RESULTS"
  else echo "  | $1 failed: $3"; echo "CASE $1 FAIL"; echo "CASE $1 FAIL" >> "$RESULTS"; fi
}
# Exactly one line of $1 equals $2 (a whole-line literal comparison).
has_line() { [ "$(printf '%s\n' "$1" | awk -v w="$2" '$0 == w { n++ } END { print n + 0 }')" -eq 1 ]; }
# Exactly one line of $1 starts with $2 (a literal prefix).
has_prefix() { [ "$(printf '%s\n' "$1" | awk -v w="$2" 'index($0, w) == 1 { n++ } END { print n + 0 }')" -eq 1 ]; }
# No line of $1 starts with any of the flag keys calib prints only when it
# reached its flags block.
no_flags() { ! printf '%s\n' "$1" | grep -Eq '^(STOP|INSTRUMENT|PORT_RATIO|WRITE_PARITY)='; }

# ============================================================ M1-M5: procmem
# Reference values are the hand computations of spec376-fixtures/README.md
# (integer KiB of the committed fixture files, divided once by 1024).
M1_WANT="597.992,576.051,640.000,0.000,0.000,21.902,576.051,576.090,593.867,336.000,597.992"
M2_WANT="500.000,416.750,584.000,64.000,12.000,95.250,468.750,404.750,492.188,0.000,500.000"
m_run() {   # $1 = case dir name; sets M_OUT, M_RC, M_INV, M_PM
  local d="$SCR/m/$1"
  mkdir -p "$d"
  M_OUT="$(PROCMEM_INV_FILE="$d/inv" PROCMEM_PM_FILE="$d/pm" bash -c '. "$1" && procmem_row 4242 "$2"' _ "$PROCMEM" "$FIXTURES/$1")"
  M_RC=$?
  M_INV="$(cat "$d/inv" 2>/dev/null || echo 0)"
  M_PM="$(cat "$d/pm" 2>/dev/null || echo 0)"
  printf '%s rc=%s inv=%s pm=%s out=%s\n' "$1" "$M_RC" "$M_INV" "$M_PM" "$M_OUT" | show
}
# The eleven memory cells (the twelfth, smaps_read_ms, is a wall time).
mem11() { printf '%s' "$1" | cut -d, -f1-11; }
ms_ok() { printf '%s' "$1" | cut -d, -f12 | grep -Eq '^[0-9]+$'; }

m_run m1-alive-nolazy
r=1; why="rc=${M_RC}"
if [ "$M_RC" -eq 0 ] && [ "$(mem11 "$M_OUT")" = "$M1_WANT" ] && ms_ok "$M_OUT" && [ "$M_INV" = "0" ] && [ "$M_PM" = "0" ]; then
  fp="$(printf '%s' "$M_OUT" | cut -d, -f2)"; an="$(printf '%s' "$M_OUT" | cut -d, -f7)"
  if [ "$fp" = "$an" ]; then r=0; else why="fp_equiv_mb=${fp} anon_mb=${an}"; fi
else why="rc=${M_RC} cells=$(mem11 "$M_OUT") inv=${M_INV} pm=${M_PM}"; fi
verdict M1 "$r" "$why"

m_run m2-lazy-swap
if [ "$M_RC" -eq 0 ] && [ "$(mem11 "$M_OUT")" = "$M2_WANT" ] && ms_ok "$M_OUT" && [ "$M_INV" = "0" ] \
   && [ "$(printf '%s' "$M_OUT" | cut -d, -f2)" = "$(awk 'BEGIN { printf "%.3f", (480000 - 65536 + 12288) / 1024 }')" ]; then r=0; else r=1; fi
verdict M2 "$r" "rc=${M_RC} cells=$(mem11 "$M_OUT") inv=${M_INV}"

m_run m3-missing-lazyfree
if [ "$M_RC" -eq 2 ] && printf '%s' "$M_OUT" | grep -q 'LazyFree=absent' && [ "$M_PM" = "0" ]; then r=0; else r=1; fi
verdict M3 "$r" "rc=${M_RC} (want 2, the runner's sampler_fatal status) out=${M_OUT} pm=${M_PM}"

m_run m4-gone
if [ "$M_RC" -eq 1 ] && [ "$M_OUT" = ",,,,,,,,,,," ] && [ "$M_PM" = "1" ] && [ "$M_INV" = "0" ]; then r=0; else r=1; fi
verdict M4 "$r" "rc=${M_RC} out=${M_OUT} pm=${M_PM} (want rc 1, twelve empty cells, pm 1)"

m_run m5-lazy-gt-anon
if [ "$M_RC" -eq 0 ] && [ "$(printf '%s' "$M_OUT" | cut -d, -f2)" = "-19.531" ] && [ "$M_INV" = "1" ]; then r=0; else r=1; fi
verdict M5 "$r" "rc=${M_RC} out=${M_OUT} inv=${M_INV} (want the row written and inv 1)"

# ================================================== synthetic cell generator
# The CSV header is read from the runner itself, so a rename of a column in
# spec376-cells.sh fails these cases instead of passing against a stale copy.
N_HDR="$(grep -c "^CSV_HEADER='" "$SCRIPT_DIR/spec376-cells.sh")"
HEADER="$(sed -n "s/^CSV_HEADER='\(.*\)'$/\1/p" "$SCRIPT_DIR/spec376-cells.sh")"
if [ "$N_HDR" != "1" ] || [ -z "$HEADER" ]; then
  echo "  | FATAL: spec376-cells.sh must define CSV_HEADER='...' exactly once (found ${N_HDR})"
  HEADER=""
fi
sha64() { printf '%064d' 0 | tr 0 "$1"; }
SHA_CAL="$(sha64 a)"; SHA_PIN="$(sha64 b)"; SHA_FRZ="$(sha64 c)"; SHA_H="$(sha64 d)"; SHA_BAD="$(sha64 e)"
PREFLIGHT_NAME="spec376-preflight-20260928T120000Z.log"
CAL_CELLS="c1 pb c2 pa"

write_builds() {   # $1 = file
  {
    echo "build_start_epoch=1790000000"
    echo "flavour=CA-cal code=$(printf '%040d' 1) path=/synthetic/CA-cal sha256=${SHA_CAL} mtime=1790000100 recompiled=yes marker=ok"
    echo "flavour=CA-pin code=$(printf '%040d' 2) path=/synthetic/CA-pin sha256=${SHA_PIN} mtime=1790000200 recompiled=yes marker=ok"
    echo "flavour=CA-frz code=$(printf '%040d' 3) path=/synthetic/CA-frz sha256=${SHA_FRZ} mtime=1790000300 recompiled=yes marker=ok"
    echo "flavour=H code=$(printf '%040d' 1) path=/synthetic/H sha256=${SHA_H} mtime=1790000400 recompiled=yes marker=ok"
  } > "$1"
}

# mkcell <ev> <cell> <server sha256> <bytes_alloc per second> <totalWrites>
# A 900 s CA cell sampled every 60 s (16 rows, 0..900), every column the
# predicates and the calibration reading key on filled; 31 alloc_probe lines
# 30 s apart; a LIVE_COPY and a TERMINAL census point; a clean runner console.
mkcell() {
  local ev="$1" c="$2" sha="$3" rate="$4" tw="$5" b="$1/spec376-$2"
  mkdir -p "$b.scrapes"
  awk -v hdr="$HEADER" -v rate="$rate" 'BEGIN {
    n = split(hdr, h, ","); print hdr
    for (e = 0; e <= 900; e += 60) {
      delete v
      fp = 500 + e / 60; lz = 10
      v["elapsed_secs"] = e; v["rss_mb"] = "600.000"; v["wal_mb"] = "1.000"; v["redb_mb"] = "2.000"
      v["disk_total_mb"] = "3.000"; v["tombstone_bytes"] = 1000
      v["fp_equiv_mb"] = sprintf("%.3f", fp); v["hwm_rss_mb"] = "650.000"; v["lazyfree_mb"] = sprintf("%.3f", lz)
      v["swap_mb"] = "0.000"; v["file_mb"] = "20.000"; v["anon_mb"] = sprintf("%.3f", fp + lz)
      v["private_dirty_mb"] = "480.000"; v["pss_mb"] = "590.000"; v["anon_huge_mb"] = "100.000"
      v["smaps_rss_mb"] = "600.000"; v["smaps_read_ms"] = 3
      lb = 100000000 + e * 1000
      v["alloc_live_bytes"] = lb; v["alloc_live_mb"] = sprintf("%.3f", lb / 1048576)
      v["alloc_probe_elapsed_s"] = e; v["alloc_probe_seq"] = e / 60 + 1
      v["bytes_alloc"] = rate * e; v["bytes_dealloc"] = rate * e - lb
      line = ""
      for (i = 1; i <= n; i++) line = line (i > 1 ? "," : "") ((h[i] in v) ? v[h[i]] : "")
      print line
    } }' > "$b.csv"
  {
    echo "provenance: server sha256=${sha} flavour=CA built=2026-09-28T10:00:00Z run_start=2026-09-28T12:05:00Z topgun_or_prune_restored_cancelled_total=present harness sha256=${SHA_H} harness_built=2026-09-28T10:30:00Z tombstone_level_ceiling_gate=present"
    echo "soak: child TOPGUN_JOURNAL_ENABLED=true"
    awk -v rate="$rate" 'BEGIN { for (e = 0; e <= 900; e += 30) printf "[server] alloc_probe elapsed_s=%d bytes_alloc=%d bytes_dealloc=%d\n", e, rate * e, rate * e / 2 }'
    echo "  LIVE_COPY  t=300.0s copy_done=300.2s live=1000 live_tag_bytes=50000"
    echo "  TERMINAL  t=900.0s live=1000 live_tag_bytes=50000"
  } > "$b.harness-console.log"
  {
    echo "cell ${c} (synthetic)"
    echo "  duration:            900s"
    echo "  csv cadence:         60s"
    echo "  /proc/loadavg:       0.05 0.10 0.12 1/200 1234"
  } > "$b.matrix.txt"
  {
    echo "post_mortem_rows=0"
    echo "post_mortem_mem_reads=0"
    echo "steal_pct=0.1000"
    echo "mem_invariant_violations=0"
    echo "RESULT: instrument sound; harness exit code 0."
    echo "RUNNER_EXIT=0"
  } > "$b.runner-console.log"
  echo "{\"totalWrites\": ${tw}, \"writeErrors\": 0, \"durationSecsActual\": 900, \"crashes\": 0}" > "$b.soak.json"
  echo "{\"elapsedSecs\": 900, \"totalWrites\": ${tw}}" > "$b.progress.jsonl"
}

# The C1 world: EV (four cells, preflight log), builds file, smoke dir,
# manifest. The portability pair sits at pa/pb = 15e6 / 45e6 = 0.3333, inside
# the manifest's band; c1 and c2 are the same binary at the same rate.
mkworld() {   # $1 = world dir
  local w="$1"
  mkdir -p "$w/ev" "$w/smoke"
  write_builds "$w/builds.txt"
  mkcell "$w/ev" c1 "$SHA_CAL" 30000000 100000
  mkcell "$w/ev" pb "$SHA_PIN" 45000000 100000
  mkcell "$w/ev" c2 "$SHA_CAL" 30000000 100000
  mkcell "$w/ev" pa "$SHA_FRZ" 15000000 100000
  { echo "== spec376-preflight start (synthetic)"; echo "CHECK identity=PASS hostname=topgun-bench nproc=4"
    echo "PREFLIGHT=PASS PREFLIGHT_AT=2026-09-28T12:00:00Z"; } > "$w/ev/$PREFLIGHT_NAME"
  { echo "  | synthetic smoke"; echo "SMOKE_ADMISSION=PASS failed=none"; } > "$w/smoke/spec376-chain.log"
  # The two section-1 literals are copied from the real manifest (never
  # retyped), so the synthetic band is the pre-registered one.
  { echo "# synthetic manifest"; echo
    grep '^CAL_PIN=' "$MANIFEST_REAL"; grep '^PORT_EXPECT=' "$MANIFEST_REAL"
    echo; echo "## APPEND-ONLY BELOW"; } > "$w/manifest.md"
}
# Runs the predicates over the four cells into EV; PRC_<cell> = exit status.
run_predicates() {   # $1 = world dir
  local c
  for c in $CAL_CELLS; do
    bash "$PRED" "$1/ev" "spec376-$c" "$1/builds.txt" > "$1/ev/spec376-$c.predicates.txt" 2>&1
    eval "PRC_$c=\$?"
  done
}
# The cal chain log, in the chain's own order, from the real exit statuses.
write_chain_log() {   # $1 = world dir
  local c rc
  {
    echo "PROC_ROOT=/proc"
    echo "PREFLIGHT_LOG=${PREFLIGHT_NAME}"
    for c in $CAL_CELLS; do
      eval "rc=\$PRC_$c"
      echo "LOAD_AT_START_${c}=0.05"
      echo "  | cell ${c}: RUNNER_EXIT=0"
      echo "PREDICATES_EXIT_${c}=${rc}"
    done
    echo "calib rc=(synthetic)"
  } > "$1/ev/spec376-chain.log"
}
# Runs calib; sets CAL_OUT and CAL_RC. $2 = 1 for the synthetic ORDER skip.
run_calib() {   # $1 = world dir, $2 = synthetic switch
  if [ "$2" = "1" ]; then
    CAL_OUT="$(SPEC376_SYNTHETIC=1 bash "$CALIB" "$1/ev" "$1/manifest.md" "$1/smoke" 2>&1)"
  else
    CAL_OUT="$(bash "$CALIB" "$1/ev" "$1/manifest.md" "$1/smoke" 2>&1)"
  fi
  CAL_RC=$?
  printf '%s\n' "$CAL_OUT" | sed -n '/^== flags ==/,$p;/^ORDER=/p;/^FATAL/p;/^SKIPPED_/p' | show
  echo "calib rc=${CAL_RC}" | show
}
flag() { printf '%s\n' "$CAL_OUT" | sed -n "s/^$1=//p"; }
# The R0.4 flags block, each key exactly once and in this order.
R04="STOP INSTRUMENT WRITES_PER_S_REF S_CA_RATE S_CA_LIVE S_CA_FPL BYTES_ALLOC_RATE ALLOC_LIVE BYTES_PER_WRITE WRITE_PARITY WRITE_PARITY_ALL A0_L_MIB PORT_RATIO PORT_BPW_RATIO PORT_EXPECT PORT_IN_EXPECT STEAL LOAD_AT_START"
flags_in_order() {
  local got
  got="$(printf '%s\n' "$CAL_OUT" | sed -n '/^== flags ==/,$p' | sed -n 's/^\([A-Z_0-9][A-Z_0-9]*\)=.*/\1/p' | tr '\n' ' ' | sed 's/ $//')"
  [ "$got" = "$R04" ]
}
# Checks the C-case outcome: rc 0, the flags in order, and exact STOP /
# INSTRUMENT lines. $1 = STOP value, $2 = INSTRUMENT value.
flags_are() {
  [ "$CAL_RC" -eq 0 ] && flags_in_order && has_line "$CAL_OUT" "STOP=$1" && has_line "$CAL_OUT" "INSTRUMENT=$2"
}
# c_case <id>: builds a fresh world in $W, runs the predicates, writes the
# chain log. The caller edits inputs between c_world and c_pipeline.
c_world() { W="$SCR/c/$1"; mkworld "$W"; }
c_pipeline() { run_predicates "$W"; write_chain_log "$W"; }
setkey() {   # $1 = file, $2 = key, $3 = new value: rewrite the one KEY= line
  awk -v k="$2" -v v="$3" 'index($0, k "=") == 1 { print k "=" v; next } { print }' "$1" > "$1.new" && mv "$1.new" "$1"
}

# ============================================================ P1-P4: predicates
P="$SCR/p"; mkdir -p "$P/ev"
write_builds "$P/builds.txt"
mkcell "$P/ev" c1 "$SHA_CAL" 30000000 100000
P1_OUT="$(bash "$PRED" "$P/ev" spec376-c1 "$P/builds.txt" 2>&1)"; P1_RC=$?
printf '%s\n' "$P1_OUT" | grep -E '^(PV|PEL|PA|PM1|PMEM)=' | show
DENIED='^(PE|FP_.*|S_.*|RECLAIM_.*|.*_AMP_FP|.*_AMP_S|.*_AMP_JE|.*_fp_mb|.*_s_mb|.*_DIRTY_SHARE|.*_FRAG_SHARE|.*_A0_share|TREND|TREND3.*|TREND_.*)$'
P1_DENIED="$(printf '%s\n' "$P1_OUT" | sed -nE 's/^([A-Za-z0-9_-]+)=.*/\1/p' | grep -E "$DENIED")"
r=1
if [ "$P1_RC" -eq 0 ] && [ -z "$HEADER" ]; then :
elif [ "$P1_RC" -eq 0 ]; then
  r=0
  for k in PV PEL PA PM1 PMEM; do
    [ "$(printf '%s\n' "$P1_OUT" | grep -c "^${k}=")" -eq 1 ] && printf '%s\n' "$P1_OUT" | grep -Eq "^${k}=TRUE( |$)" || r=1
  done
  [ -z "$P1_DENIED" ] || r=1
fi
verdict P1 "$r" "rc=${P1_RC} (want PV PEL PA PM1 PMEM each exactly once TRUE, no denied key; denied='${P1_DENIED}')"

P2="$SCR/p2"; mkdir -p "$P2/ev"
mkcell "$P2/ev" c1 "$SHA_CAL" 30000000 100000
awk -F, -v OFS=, 'NR == 1 { for (i = 1; i <= NF; i++) if ($i == "fp_equiv_mb") c = i; print; next }
                  NR >= 3 && NR <= 5 { $c = "" } { print }' "$P2/ev/spec376-c1.csv" > "$P2/x" && mv "$P2/x" "$P2/ev/spec376-c1.csv"
P2_OUT="$(bash "$PRED" "$P2/ev" spec376-c1 "$P/builds.txt" 2>&1)"
printf '%s\n' "$P2_OUT" | grep '^PEL=' | show
if [ "$(printf '%s\n' "$P2_OUT" | grep -c '^PEL=')" -eq 1 ] && printf '%s\n' "$P2_OUT" | grep -Eq '^PEL=FALSE( |$)'; then r=0; else r=1; fi
verdict P2 "$r" "want exactly one PEL=FALSE (13 of 16 rows carry fp_equiv_mb, need 15)"

# P3: the frozen macOS predicates, given the same Linux cell under a cell name
# they know (k1 = CA), must fail closed on the missing footprint column.
P3="$SCR/p3"; mkdir -p "$P3/ev"
for f in "$P/ev"/spec376-c1.*; do
  [ -f "$f" ] && cp "$f" "$P3/ev/spec372-k1.${f##*/spec376-c1.}"
done
mkdir -p "$P3/ev/spec372-k1.scrapes"
bash "$PRED372" "$P3/ev" spec372-k1 "$P/builds.txt" > "$P3/stdout.txt" 2>&1
P3_RC=$?
grep '^PE=' "$P3/ev/spec372-k1.predicates.txt" 2>/dev/null | show
if [ "$(grep -c '^PE=' "$P3/ev/spec372-k1.predicates.txt" 2>/dev/null)" = "1" ] \
   && grep -qx 'PE=FALSE reason=missing_csv_column' "$P3/ev/spec372-k1.predicates.txt"; then r=0; else r=1; fi
verdict P3 "$r" "rc=${P3_RC}; want exactly one 'PE=FALSE reason=missing_csv_column' from the frozen spec372-predicates.sh"

# P4: P1 ran with SPEC376_A0_MIB / SPEC376_B_LIVE unset (unset at the top).
P4_AMP="$(printf '%s\n' "$P1_OUT" | grep -E '^[A-Z]+_AMP_[A-Z]+L=')"
P4_N="$(printf '%s\n' "$P4_AMP" | grep -c .)"
P4_BAD="$(printf '%s\n' "$P4_AMP" | grep -vE '^[A-Z]+_AMP_[A-Z]+L=n/a reason=no_linux_estimator$' | grep -c .)"
printf '%s\n' "$P4_AMP" | show
if [ "$P4_N" -ge 4 ] && [ "$P4_BAD" -eq 0 ] && has_line "$P1_OUT" "TRENDL=n/a reason=no_linux_estimator"; then r=0; else r=1; fi
verdict P4 "$r" "AMP_*L lines=${P4_N} (want >= 4: TERM/DECIDE x FPL/SL) not n/a=${P4_BAD}"

# ============================================================ C1-C14: calib
c_world C1; c_pipeline; run_calib "$W" 1
if flags_are none SOUND && has_line "$CAL_OUT" "PORT_IN_EXPECT=TRUE" && has_line "$CAL_OUT" "ORDER=SKIPPED (synthetic)" \
   && has_line "$CAL_OUT" "WRITE_PARITY=0.0000" && has_line "$CAL_OUT" "PORT_RATIO=0.3333" \
   && [ "$PRC_c1$PRC_pb$PRC_c2$PRC_pa" = "0000" ]; then r=0; else r=1; fi
verdict C1 "$r" "want rc 0, the R0.4 block in order, STOP=none INSTRUMENT=SOUND PORT_IN_EXPECT=TRUE PORT_RATIO=0.3333; predicates rc=${PRC_c1}${PRC_pb}${PRC_c2}${PRC_pa}"

# C2: pa's rate moved to 0.90 x pb's; the ratio is recorded, never a gate.
c_world C2; mkcell "$W/ev" pa "$SHA_FRZ" 40500000 100000; c_pipeline; run_calib "$W" 1
if flags_are none SOUND && has_line "$CAL_OUT" "PORT_RATIO=0.9000" && has_line "$CAL_OUT" "PORT_IN_EXPECT=FALSE"; then r=0; else r=1; fi
verdict C2 "$r" "want STOP=none INSTRUMENT=SOUND PORT_RATIO=0.9000 PORT_IN_EXPECT=FALSE"

# C3: c2's totalWrites +8 % in its soak.json: |100000-108000| / 104000 = 0.0769.
c_world C3
sed 's/"totalWrites": 100000/"totalWrites": 108000/' "$W/ev/spec376-c2.soak.json" > "$W/x" && mv "$W/x" "$W/ev/spec376-c2.soak.json"
c_pipeline; run_calib "$W" 1
if flags_are "V (c1c2:write_parity=0.0769)" "NOT_SOUND reason=stop"; then r=0; else r=1; fi
verdict C3 "$r" "want STOP=V (c1c2:write_parity=0.0769) INSTRUMENT=NOT_SOUND reason=stop"

c_world C4; setkey "$W/ev/spec376-pb.runner-console.log" steal_pct 2.0000; c_pipeline; run_calib "$W" 1
if flags_are "H (pb:steal=2.0000)" "NOT_SOUND reason=stop"; then r=0; else r=1; fi
verdict C4 "$r" "want STOP=H (pb:steal=2.0000) INSTRUMENT=NOT_SOUND reason=stop"

c_world C5
{ sed '$d' "$W/ev/$PREFLIGHT_NAME"; echo "PREFLIGHT=FAIL failed=swap PREFLIGHT_AT=2026-09-28T12:00:00Z"; } > "$W/x" && mv "$W/x" "$W/ev/$PREFLIGHT_NAME"
c_pipeline; run_calib "$W" 1
if flags_are "H (preflight=FAIL)" "NOT_SOUND reason=stop"; then r=0; else r=1; fi
verdict C5 "$r" "want STOP=H (preflight=FAIL) INSTRUMENT=NOT_SOUND reason=stop"

c_world C6; setkey "$W/ev/spec376-c1.runner-console.log" mem_invariant_violations 2; c_pipeline; run_calib "$W" 1
if flags_are "V (c1:PMEM=FALSE)" "NOT_SOUND reason=stop"; then r=0; else r=1; fi
verdict C6 "$r" "want STOP=V (c1:PMEM=FALSE) INSTRUMENT=NOT_SOUND reason=stop"

c_world C7; grep -v '^SMOKE_ADMISSION=' "$W/smoke/spec376-chain.log" > "$W/x"; mv "$W/x" "$W/smoke/spec376-chain.log"
c_pipeline; run_calib "$W" 1
if flags_are none "NOT_SOUND reason=smoke"; then r=0; else r=1; fi
verdict C7 "$r" "want STOP=none INSTRUMENT=NOT_SOUND reason=smoke"

c_world C8; grep -v '^CAL_PIN=' "$W/manifest.md" > "$W/x"; mv "$W/x" "$W/manifest.md"
c_pipeline; run_calib "$W" 1
if [ "$CAL_RC" -eq 3 ] && no_flags "$CAL_OUT"; then r=0; else r=1; fi
verdict C8 "$r" "rc=${CAL_RC}; want rc 3 and no flags"

c_world C9; c_pipeline; run_calib "$W" 0
if [ "$CAL_RC" -eq 3 ] && no_flags "$CAL_OUT" && [ "$(printf '%s\n' "$CAL_OUT" | grep -c '^ORDER=FAIL')" -eq 1 ]; then r=0; else r=1; fi
verdict C9 "$r" "rc=${CAL_RC}; want rc 3, one ORDER=FAIL line and no flags"

# C10: the rate program exactly as calib extracts it (same range, same trim,
# same two sha256 assertions), over the committed 373b cells; the expected
# strings are read from the committed spec373b.verdict.txt, never retyped.
VERDICT="$SCRIPT_DIR/spec373b-verdict.sh"
VREF="$SCRIPT_DIR/spec373b.verdict.txt"
RATE_PROG="$(sed -n '151,170p' "$VERDICT" | sed '$ s/'\'' "\$CSV" || FAILED="\${FAILED} RATE_\${c}"$//')"
r=0; why=""
[ "$(shasum -a 256 "$VERDICT" | awk '{print $1}')" = d5cef0dfe2033f13dc8c5446a49233046a38beeaa43f5ea24817aab1d836dffa ] || { r=1; why="${why} verdict_sha"; }
[ "$(printf '%s\n' "$RATE_PROG" | shasum -a 256 | awk '{print $1}')" = 35fb23008728703b9b2159061ed513d0d040f1554d1f3f47917ea401d2cc01fd ] || { r=1; why="${why} rate_prog_sha"; }
for c in b1 b2 a1 a2; do
  got="$(awk -F, -v c="$c" "$RATE_PROG" "$SCRIPT_DIR/spec373b-$c.csv" | sed -n "s/^BYTES_ALLOC_RATE_${c}=\([^ ]*\).*/\1/p")"
  n_want="$(grep -c "^BYTES_ALLOC_RATE_${c}=" "$VREF")"
  want="$(sed -n "s/^BYTES_ALLOC_RATE_${c}=\([^ ]*\).*/\1/p" "$VREF")"
  echo "${c} got=${got:-absent} want=${want:-absent}" | show
  if [ "$n_want" != "1" ] || [ -z "$got" ] || [ "$got" != "$want" ]; then r=1; why="${why} ${c}"; fi
done
verdict C10 "$r" "mismatch:${why}"

# C11: pa's launched binary is not the CA-frz build (console line 1 sha256).
c_world C11
sed "1s/server sha256=${SHA_FRZ}/server sha256=${SHA_BAD}/" "$W/ev/spec376-pa.harness-console.log" > "$W/x" && mv "$W/x" "$W/ev/spec376-pa.harness-console.log"
c_pipeline; run_calib "$W" 1
if grep -Eq '^PV=FALSE( |$)' "$W/ev/spec376-pa.predicates.txt" && [ "$(grep -c '^PV=' "$W/ev/spec376-pa.predicates.txt")" -eq 1 ] \
   && flags_are "V (pa:PV=FALSE)" "NOT_SOUND reason=stop"; then r=0; else r=1; fi
verdict C11 "$r" "want PV=FALSE in spec376-pa.predicates.txt and STOP=V (pa:PV=FALSE)"

# C12: pa's predicates output truncated after its PV line (a crashed run) and
# the chain log's PREDICATES_EXIT_pa=4.
c_world C12; c_pipeline
awk '{ print } /^PV=/ { exit }' "$W/ev/spec376-pa.predicates.txt" > "$W/x" && mv "$W/x" "$W/ev/spec376-pa.predicates.txt"
setkey "$W/ev/spec376-chain.log" PREDICATES_EXIT_pa 4
run_calib "$W" 1
if flags_are "V (pa:PEL=absent pa:PA=absent pa:PM1=absent pa:PMEM=absent pa:predicates_rc=4)" "NOT_SOUND reason=stop"; then r=0; else r=1; fi
verdict C12 "$r" "want STOP=V (pa:PEL=absent pa:PA=absent pa:PM1=absent pa:PMEM=absent pa:predicates_rc=4)"

c_world C13; c_pipeline; rm -f "$W/ev/spec376-pb.predicates.txt"; run_calib "$W" 1
if flags_are "V (pb:predicates_missing)" "NOT_SOUND reason=stop" && grep -q '^PREDICATES_EXIT_pb=0$' "$W/ev/spec376-chain.log"; then r=0; else r=1; fi
verdict C13 "$r" "want STOP=V (pb:predicates_missing) with PREDICATES_EXIT_pb=0 kept"

# C14: one c2 row whose three probe fields are mixed (numeric, empty, x).
c_world C14
awk -F, -v OFS=, 'NR == 1 { for (i = 1; i <= NF; i++) { if ($i == "bytes_alloc") b = i; if ($i == "alloc_live_bytes") l = i; if ($i == "alloc_probe_elapsed_s") e = i } print; next }
                  NR == 9 { $l = ""; $e = "x" } { print }' "$W/ev/spec376-c2.csv" > "$W/x" && mv "$W/x" "$W/ev/spec376-c2.csv"
c_pipeline; run_calib "$W" 1
if has_line "$CAL_OUT" "SKIPPED_c2=1" && flags_are "V (c2:skipped=1)" "NOT_SOUND reason=stop"; then r=0; else r=1; fi
verdict C14 "$r" "want SKIPPED_c2=1 and STOP=V (c2:skipped=1)"

# ============================================================ F1: preflight
# --apply on a host that is not topgun-bench: identity fails first, so
# nothing may be stopped or written. Every host command the preflight could
# use to change the host (systemctl, tee) is a stub that records its argv;
# pgrep and ss are stubs too, so isolation reads PASS on any machine and the
# refusal is attributed to identity alone.
F="$SCR/f1"; mkdir -p "$F/bin" "$F/root/lib/systemd/system" "$F/root/sys/kernel/mm/transparent_hugepage" "$F/log"
stub() { printf '%s\n' '#!/bin/sh' "$2" > "$F/bin/$1"; chmod +x "$F/bin/$1"; }
stub uname 'if [ "${1:-}" = "-s" ] || [ "$#" -eq 0 ]; then echo Linux; else echo 6.1.0-synthetic; fi'
stub hostname 'echo topgun-new'
stub nproc 'echo 4'
stub ss 'exit 0'
stub pgrep 'exit 1'
stub systemctl "echo \"systemctl \$*\" >> '$F/record'"
stub tee "echo \"tee \$*\" >> '$F/record'; cat > /dev/null"
for k in enabled defrag; do
  echo '[always] madvise never' > "$F/root/sys/kernel/mm/transparent_hugepage/$k"
  cp "$F/root/sys/kernel/mm/transparent_hugepage/$k" "$F/$k.pre"
done
F_OUT="$(PATH="$F/bin:$PATH" SPEC376_PREFLIGHT_SYS_ROOT="$F/root" SPEC376_PREFLIGHT_CMDLOG="$F/cmdlog" \
  SPEC376_PREFLIGHT_LOG_DIR="$F/log" bash "$PREFLIGHT" --apply 2>&1)"
F_RC=$?
printf '%s\n' "$F_OUT" | sed "s|$F|@F1@|g" | show
F_LOGS="$(ls "$F/log" | grep -c '^spec376-preflight-.*\.log$')"
F_LAST="$( [ "$F_LOGS" = "1" ] && tail -n 1 "$F/log"/spec376-preflight-*.log)"
MUT=0
[ -s "$F/cmdlog" ] && MUT=$((MUT + $(wc -l < "$F/cmdlog")))
[ -s "$F/record" ] && MUT=$((MUT + $(wc -l < "$F/record")))
echo "MUTATIONS=${MUT}" | show
THP_OK=1
for k in enabled defrag; do cmp -s "$F/root/sys/kernel/mm/transparent_hugepage/$k" "$F/$k.pre" || THP_OK=0; done
echo "last line: ${F_LAST}" | show
if [ "$F_RC" -ne 0 ] && [ "$F_LOGS" = "1" ] && printf '%s\n' "$F_LAST" | grep -Eq '^PREFLIGHT=FAIL failed=identity PREFLIGHT_AT=[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$' \
   && [ "$MUT" -eq 0 ] && [ "$THP_OK" = "1" ]; then r=0; else r=1; fi
verdict F1 "$r" "rc=${F_RC} logs=${F_LOGS} MUTATIONS=${MUT} thp_untouched=${THP_OK}"

# ============================================================ verdict
# Counted from this run's own CASE records: a case that recorded nothing, or
# more than once, or FAIL, fails the suite (absence is never a pass).
FAILED=""; N=0
for id in $CASES; do
  N=$((N + 1))
  np="$(grep -cx "CASE ${id} PASS" "$RESULTS")"
  nall="$(grep -c "^CASE ${id} " "$RESULTS")"
  [ "$np" = "1" ] && [ "$nall" = "1" ] || FAILED="${FAILED:+${FAILED},}${id}"
done
if [ -z "$FAILED" ]; then
  echo "SYNTH376=PASS cases=${N}"
  exit 0
fi
echo "SYNTH376=FAIL failed=${FAILED}"
exit 1
