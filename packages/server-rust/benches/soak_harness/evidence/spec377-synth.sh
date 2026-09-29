#!/usr/bin/env bash
#
# Synthetic cases for the allocator series' own programs: the per-cell
# predicates (spec377-predicates.sh: the allocator proof PALLOC, the per-entry
# series, the fit cross-check and TREND), the chain's feature-graph proof (its
# tree_cmd/tree_proof text extracted from spec377-chain.sh and run against a
# stub cargo), and the decision reading (spec377-decide.sh +
# spec377-decide.awk). Every input is built here from pinned literals or from
# a fixed-seed generator written here; every expectation is written here or
# recomputed here by code that shares nothing with the program under test;
# every comparison is mechanical.
#
# usage: spec377-synth.sh <SCRATCH_DIR>
#   SCRATCH_DIR must not be the evidence dir or lie under it, and must be
#   absent or empty (a leftover from an earlier run could otherwise be read as
#   this run's output).
#
# Output: one "CASE <id> PASS|FAIL" line per case, then exactly one column-0
#   SYNTH377=PASS cases=<n>   or   SYNTH377=FAIL failed=<ids>
# SYNTH377=PASS iff every case below printed exactly one "CASE <id> PASS" line.
# Exit 0 on PASS, 1 on FAIL, 2 on usage or a refused scratch dir.
#
# WHY EVERY ECHOED LINE IS PREFIXED "  | ": this stdout is tee'd into the
# smoke chain log, where the smoke admission counts column-0 KEY= lines
# (SMOKE_ADMISSION=, SYNTH_PARITY=, SYNTH377=, ...). The decision program under
# test prints column-0 keys of its own, so nothing this file echoes from a
# fixture or a program may start at column 0; the only column-0 KEY= line is
# the final SYNTH377= verdict.
#
# WHY THE GENERATOR IS ITS OWN: the noise of the 6 h cells comes from a
# Park-Miller generator (16807 x seed mod 2^31 - 1, exact in a double) and an
# Irwin-Hall normal (twelve uniforms minus six), so no libm call and no awk
# rand() enters a fixture: macOS and the bench host build byte-identical
# cells from the same seeds.
#
# Cases (the spec's R10 table, plus named extras):
#   A1-A12    PALLOC per label on smoke cells (SYS, JE, MI3 v3 unprefixed, MI2
#             v2 prefixed); A13/A14 a series MI cell against the committed
#             literal file (sha listed / sha changed).
#   N_<id>    one negative per PALLOC item id: exactly that id is named.
#   B1-B5     the chain's feature-graph proof over stub cargo outputs.
#   P1-P18    synthetic 6 h cells (60 s rows, census every 300 s): the census
#             join, PE_LEVEL, the fit window and its cross-check at the
#             fitter's printed precision, TREND classes, STAGE2_T, FIXED_EST.
#   D1-D18,   the decision reading over synthetic predicates files and chain
#   CR1-CR10, logs; STOP-H/V clauses, precedence and order; K1 the flags
#   H*/V*/R*  block's key order.
#   E1        an enumeration over INPUT factors (fixed seed, every SYS
#             combination plus every pair of factor levels), checked against an
#             independent recomputation of R7/R8/R8.1 written below; E2 an
#             out-of-domain SYS token.
set -uo pipefail
export LC_ALL=C

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
SCR="${1:-}"
if [ "$#" -ne 1 ] || [ -z "$SCR" ]; then
  echo "usage: spec377-synth.sh <SCRATCH_DIR>" >&2; exit 2
fi
# The physical path of $1 without creating anything: the nearest existing
# ancestor resolved with pwd -P, plus the not-yet-existing tail.
canon() {
  local p="$1" tail=""
  case "$p" in /*) ;; *) p="$PWD/$p" ;; esac
  while [ ! -d "$p" ]; do tail="/$(basename "$p")${tail}"; p="$(dirname "$p")"; done
  printf '%s%s\n' "$(cd "$p" && pwd -P)" "$tail"
}
# Paths are spliced into sh -c strings and awk system() calls below, so the
# scratch path is limited to characters no shell treats specially.
case "$SCR" in *[!A-Za-z0-9_./-]*) echo "FATAL: the scratch dir may use only [A-Za-z0-9_./-]" >&2; exit 2 ;; esac
case "$(canon "$SCR")/" in
  "$SCRIPT_DIR"/*) echo "FATAL: the scratch dir must not be the evidence dir or under it" >&2; exit 2 ;;
esac
if [ -d "$SCR" ] && [ -n "$(ls -A "$SCR" 2>/dev/null)" ]; then
  echo "FATAL: the scratch dir ${SCR} is not empty" >&2; exit 2
fi
mkdir -p "$SCR" || { echo "FATAL: cannot create ${SCR}" >&2; exit 2; }
SCR="$(cd "$SCR" && pwd -P)"
mkdir -p "$SCR/tmp"
# decide keeps its intermediate under TMPDIR and refuses one inside the
# evidence dir; the scratch dir is outside it by the check above.
export TMPDIR="$SCR/tmp"
# The environment must not steer a case: the synthetic switch, the manifest
# names and the fitter override are set per call, never inherited.
unset SPEC377_SYNTHETIC SPEC377_MANIFEST SPEC377_MANIFEST_COMMIT SPEC377_TEST_FIT_SLOPE

PRED="$SCRIPT_DIR/spec377-predicates.sh"
DECIDE="$SCRIPT_DIR/spec377-decide.sh"
CHAIN="$SCRIPT_DIR/spec377-chain.sh"
MANIFEST_REAL="$SCRIPT_DIR/spec377-manifest.md"
CASES="F0 A1 A2 A3 A4 A5 A6 A7 A8 A9 A10 A11 A12 A13 A14
N_pv N_s1 N_s2 N_s3 N_s4 N_s5 N_s6 N_marker_control N_j1 N_j2 N_j3 N_j4 N_j5 N_j6
N_m1 N_m2 N_m3 N_m4 N_m5 N_mi_version N_mi_thread_prefix
B1 B2 B3 B4 B5
P1 P2 P3 P4 P5 P6 P7 P8 P9 P10 P11 P12 P13 P14 P15 P16 P17 P18 P18b P18c
D1 D2 D3 D4 D5 D6 D7 D8 D9 D10 D10b D11 D11b D12 D13 D14 D14b D15 D15b D16 D18
CR1 CR2 CR3 CR4 CR5 CR6 CR6b CR6c CR6d CR7 CR8 CR9 CR10
H1 H2 H3 H4 H5 V1 V2 V3 V4 V5 V6 V7 V8 V9 R11 R12 R13a R13b OOD1 LZ1 K1
E1 E2"
# Jobs for the E1 enumeration: one decide run per input, in parallel.
NJ="$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 2)"
case "$NJ" in ''|*[!0-9]*) NJ=2 ;; esac
[ "$NJ" -gt 8 ] && NJ=8
[ "$NJ" -lt 1 ] && NJ=1

show() { sed 's/^/  | /'; }
RESULTS="$SCR/results.txt"; : > "$RESULTS"
verdict() {   # $1 = id, $2 = 0 for PASS, $3 = the reason on FAIL
  if [ "$2" -eq 0 ]; then echo "CASE $1 PASS"; echo "CASE $1 PASS" >> "$RESULTS"
  else echo "  | $1 failed: $3"; echo "CASE $1 FAIL"; echo "CASE $1 FAIL" >> "$RESULTS"; fi
}
# Exactly one line of $1 equals $2 (a whole-line literal comparison).
has_line() { [ "$(printf '%s\n' "$1" | awk -v w="$2" '$0 == w { n++ } END { print n + 0 }')" -eq 1 ]; }
# Exactly one line of $1 starts with $2 (a literal prefix).
has_prefix() { [ "$(printf '%s\n' "$1" | awk -v w="$2" 'index($0, w) == 1 { n++ } END { print n + 0 }')" -eq 1 ]; }
# The value of the one column-0 KEY= line of $1 (empty when absent or dup).
val() { printf '%s\n' "$1" | awk -v k="$2=" 'index($0, k) == 1 { n++; v = substr($0, length(k) + 1) } END { if (n == 1) print v }'; }
# One field of a "k=v k=v" reading.
fld() { printf '%s\n' "$1" | awk -v k="$2" '{ for (i = 1; i <= NF; i++) if (index($i, k "=") == 1) { print substr($i, length(k) + 2); exit } }'; }
# $1 within [$2, $3] as numbers; false on a non-number.
within() { awk -v x="$1" -v lo="$2" -v hi="$3" 'BEGIN { exit !(x ~ /^[-+]?[0-9]+(\.[0-9]+)?([eE][-+]?[0-9]+)?$/ && x + 0 >= lo + 0 && x + 0 <= hi + 0) }'; }

# ============================================================ shared fixtures
# The CSV header is read from the runner itself, so a renamed column fails
# these cases instead of passing against a stale copy.
N_HDR="$(grep -c "^CSV_HEADER='" "$SCRIPT_DIR/spec377-cells.sh")"
HEADER="$(sed -n "s/^CSV_HEADER='\(.*\)'$/\1/p" "$SCRIPT_DIR/spec377-cells.sh")"
F0_WHY=""
[ "$N_HDR" = "1" ] && [ -n "$HEADER" ] || F0_WHY="${F0_WHY} CSV_HEADER=${N_HDR}"

# The synthetic manifest carries section 1's literal lines copied from the
# real manifest (never retyped), so every threshold a case meets is the
# pre-registered one.
LITERALS="SERIES_PIN A0_L_MIB FLAT_BAR_PER_H BETTER_BAR SYS_AGREE_BAR OPS_PARITY_MIN TIE_BAND STAGE2_MAX_H LEVEL_WINDOW_S MI_POSTINIT_BOUND LAZY_DRIFT_BAR PRICE_EUR_PER_H MIN_DISK_GIB"
SYN_MAN="$SCR/manifest-literals.md"
{
  echo "# synthetic manifest (section-1 literals copied from spec377-manifest.md)"; echo
  for k in $LITERALS; do sed '/^## APPEND-ONLY BELOW/q' "$MANIFEST_REAL" | grep "^${k}="; done
  echo
} > "$SYN_MAN"
N_LIT="$(grep -c '^[A-Z0-9_]*=' "$SYN_MAN")"
[ "$N_LIT" = "13" ] || F0_WHY="${F0_WHY} literals_copied=${N_LIT}"
lit() { sed -n "s/^$1=//p" "$SYN_MAN" | awk '{ print $1 }'; }
A0="$(lit A0_L_MIB)"; LW="$(lit LEVEL_WINDOW_S)"; BAR="$(lit FLAT_BAR_PER_H)"; CAP="$(lit STAGE2_MAX_H)"
L_BETTER="$(lit BETTER_BAR)"; L_TIE="$(lit TIE_BAND)"; L_OPS="$(lit OPS_PARITY_MIN)"
for v in "$A0" "$LW" "$BAR" "$CAP" "$L_BETTER" "$L_TIE" "$L_OPS"; do
  printf '%s' "$v" | grep -Eq '^[0-9]+(\.[0-9]+)?$' || F0_WHY="${F0_WHY} literal_value=${v:-empty}"
done
# F0: every fixture contract above held; a case below never runs on a
# silently broken fixture (its own FAIL would follow anyway, but the cause is
# named here).
[ -z "$F0_WHY" ]; verdict F0 $? "fixture contract:${F0_WHY}"
mkman() {   # $1 = file, then extra lines for section 1
  local f="$1"; shift
  { cat "$SYN_MAN"; for l in "$@"; do printf '%s\n' "$l"; done; echo "## APPEND-ONLY BELOW"; } > "$f"
}

sha64() { printf '%064d' 0 | tr 0 "$1"; }
SHA_SYS="$(sha64 a)"; SHA_JE="$(sha64 b)"; SHA_MI3="$(sha64 c)"; SHA_MI2="$(sha64 e)"; SHA_H="$(sha64 d)"
label_of() { case "$1" in ssy|s1|s2) echo SYS-ser ;; sje|je) echo JE-ser ;; smi3|mi3) echo MI3-ser ;; smi2|mi2) echo MI2-ser ;; esac; }

# The builds file: five flavour lines, the four marker counts (the SYS
# controls) and the feature-graph records, each from a B_* variable.
builds_defaults() {
  B_MARK_SYS="je=0 mi=0"; B_MARK_JE="je=3 mi=0"; B_MARK_MI3="je=0 mi=5"; B_MARK_MI2="je=0 mi=5"
  B_TREE_SYS="PASS reason=none"; B_TREE_JE="PASS reason=none"; B_TREE_MI3="PASS reason=none"; B_TREE_MI2="PASS reason=none"
}
write_builds() {   # $1 = file
  {
    echo "build_start_epoch=1790000000"
    echo "flavour=SYS-ser code=$(printf '%040d' 0) path=/synthetic/SYS-ser sha256=${SHA_SYS} mtime=1790000100 recompiled=yes marker=ok"
    echo "flavour=JE-ser code=$(printf '%040d' 0) path=/synthetic/JE-ser sha256=${SHA_JE} mtime=1790000200 recompiled=yes marker=ok"
    echo "flavour=MI3-ser code=$(printf '%040d' 0) path=/synthetic/MI3-ser sha256=${SHA_MI3} mtime=1790000300 recompiled=yes marker=ok"
    echo "flavour=MI2-ser code=$(printf '%040d' 0) path=/synthetic/MI2-ser sha256=${SHA_MI2} mtime=1790000400 recompiled=yes marker=ok"
    echo "flavour=H code=$(printf '%040d' 0) path=/synthetic/H sha256=${SHA_H} mtime=1790000500 recompiled=yes marker=ok"
    echo "BIN_MARKERS_SYS-ser=${B_MARK_SYS}"; echo "BIN_MARKERS_JE-ser=${B_MARK_JE}"
    echo "BIN_MARKERS_MI3-ser=${B_MARK_MI3}"; echo "BIN_MARKERS_MI2-ser=${B_MARK_MI2}"
    echo "TREE_RC_SYS-ser=tikv-jemalloc-sys:101 libmimalloc-sys:101"; echo "TREE_PROOF_SYS-ser=${B_TREE_SYS}"
    echo "TREE_RC_JE-ser=0"; echo "TREE_PROOF_JE-ser=${B_TREE_JE}"
    echo "TREE_RC_MI3-ser=0"; echo "TREE_PROOF_MI3-ser=${B_TREE_MI3}"
    echo "TREE_RC_MI2-ser=0"; echo "TREE_PROOF_MI2-ser=${B_TREE_MI2}"
  } > "$1"
}

# ============================================================ A / N: PALLOC
# The allocator lines as the harness mirrors them ("[server] " + stderr),
# in the shapes the vendored sources print (R3.3). Trailing spaces on the
# mimalloc option lines are part of the literal.
JE_CONF_LINES='[server] <jemalloc>: malloc_conf #1 (string specified via --with-malloc-conf): ""
[server] <jemalloc>: malloc_conf #2 (string pointed to by the global variable malloc_conf): ""
[server] <jemalloc>: malloc_conf #3 ("name" of the file referenced by the symbolic link named /etc/malloc.conf): ""
[server] <jemalloc>: malloc_conf #4 (value of the environment variable MALLOC_CONF): "background_thread:true,confirm_conf:true"
[server] <jemalloc>: malloc_conf #5 (string pointed to by the global variable malloc_conf_2_conf_harder): ""
[server] <jemalloc>: -- Set conf value: background_thread:true
[server] <jemalloc>: -- Set conf value: confirm_conf:true'
JE_CONFIG_LINE='[server] je_config version=5.3.1-0-g81034ce1f1373e37dc865038e1bc8eeecf559ce8 arenas_narenas=17 opt_narenas=16 opt_background_thread=true background_thread=true max_background_threads=4 opt_tcache=true'
# The option lines end in a space (the empty unit %s of "option '%s': %ld %s"),
# written through printf so no editor can strip it.
MI3_BLOCK="$(echo '[server] v3.3.2 (built on Sep 29 2026, 09:00:00)'
  printf "[server] option '%s': %s \n" show_errors 0 purge_delay 1000 arena_purge_mult 1 purge_decommits 1)"
MI2_BLOCK="$(echo '[server] mimalloc: v2.3.2 (built on Sep 29 2026, 09:00:00)'
  printf "[server] mimalloc: option '%s': %s \n" show_errors 0 purge_delay 10 arena_purge_mult 10 purge_decommits 1)"

a_defaults() {   # $1 = cell
  A_CELL="$1"; A_LABEL="$(label_of "$1")"; A_EXTRA=""; A_STEAL=1; A_POSTINIT=0; A_FATAL=0; A_LINE1_SHA=""
  case "$A_LABEL" in
    SYS-ser) A_FL=SYS; A_SHA="$SHA_SYS"; A_ENV=none; A_BG=0; A_MARK="je=0 mi=0"; A_ALLOC="" ;;
    JE-ser)  A_FL=JE; A_SHA="$SHA_JE"; A_ENV='_RJEM_MALLOC_CONF=background_thread:true,confirm_conf:true'; A_BG=4; A_MARK="je=3 mi=0"
             A_ALLOC="${JE_CONF_LINES}
${JE_CONFIG_LINE}" ;;
    MI3-ser) A_FL=MI; A_SHA="$SHA_MI3"; A_ENV=MIMALLOC_VERBOSE=1; A_BG=0; A_MARK="je=0 mi=5"; A_ALLOC="$MI3_BLOCK" ;;
    MI2-ser) A_FL=MI; A_SHA="$SHA_MI2"; A_ENV=MIMALLOC_VERBOSE=1; A_BG=0; A_MARK="je=0 mi=5"; A_ALLOC="$MI2_BLOCK" ;;
  esac
  builds_defaults
}
# A 120 s smoke-shaped cell (20 s cadence): every file the predicates read.
a_write() {   # $1 = dir
  local d="$1" b="$1/spec377-${A_CELL}"
  mkdir -p "$d" "$b.scrapes"
  awk -v hdr="$HEADER" 'BEGIN {
    n = split(hdr, h, ","); print hdr
    for (e = 0; e <= 120; e += 20) {
      delete v; fp = 40 + e / 20
      v["elapsed_secs"] = e; v["rss_mb"] = sprintf("%.3f", fp * 1.05); v["wal_mb"] = "1.000"; v["redb_mb"] = "2.000"
      v["disk_total_mb"] = "3.000"; v["tombstone_bytes"] = 100; v["fp_equiv_mb"] = sprintf("%.3f", fp)
      v["hwm_rss_mb"] = sprintf("%.3f", fp * 1.1); v["lazyfree_mb"] = "0.000"; v["swap_mb"] = "0.000"; v["file_mb"] = "1.000"
      v["anon_mb"] = sprintf("%.3f", fp); v["private_dirty_mb"] = sprintf("%.3f", fp); v["pss_mb"] = sprintf("%.3f", fp)
      v["anon_huge_mb"] = "0.000"; v["smaps_rss_mb"] = sprintf("%.3f", fp); v["smaps_read_ms"] = 2
      line = ""; for (i = 1; i <= n; i++) line = line (i > 1 ? "," : "") ((h[i] in v) ? v[h[i]] : ""); print line
    } }' > "$b.csv"
  {
    echo "provenance: server sha256=${A_LINE1_SHA:-$A_SHA} flavour=${A_FL} built=2026-09-29T10:00:00Z run_start=2026-09-29T12:00:00Z topgun_or_prune_restored_cancelled_total=present harness sha256=${SHA_H} harness_built=2026-09-29T10:30:00Z tombstone_level_ceiling_gate=present"
    echo "soak: child TOPGUN_JOURNAL_ENABLED=true"
    [ -n "$A_ALLOC" ] && printf '%s\n' "$A_ALLOC"
    if [ "$A_FL" = "JE" ]; then
      for e in 0 30 60 90 120; do echo "[server] je_probe elapsed_s=${e} allocated=1000000 active=1100000 resident=1300000 retained=10 mapped=2000000 metadata=50000 seq=$((e / 30 + 1))"; done
    fi
    [ -n "$A_EXTRA" ] && printf '%s\n' "$A_EXTRA"
    echo "  LIVE_COPY  t=60.0s copy_done=60.1s keys=96 live=1000 live_tag_bytes=50000"
    echo "  TERMINAL   t=122.0s copy_done=n/a keys=96 live=2000 live_tag_bytes=100000"
  } > "$b.harness-console.log"
  {
    echo "=== spec377 Linux run: cell ${A_CELL} (synthetic) ==="
    [ "$A_ENV" = "@absent" ] || echo "ALLOC_ENV_${A_CELL}=${A_ENV}"
    echo "JE_BG_THREADS_${A_CELL}=${A_BG}"
    echo "THP_${A_CELL}=madvise/madvise"
    echo "ALLOC_PROOF_AT_${A_CELL}=60"
    echo "BIN_MARKERS_${A_CELL}=${A_MARK}"
    echo "MI_POSTINIT_LINES_${A_CELL}=${A_POSTINIT}"
    [ "$A_FATAL" = "1" ] && echo "FATAL: SOAK_SERVER_BINARY is not a JE build (synthetic)"
    echo "post_mortem_rows=0"
    echo "RESULT: instrument sound; harness exit code 0."
    [ "$A_STEAL" = "1" ] && echo "steal_pct=0.0000"
    echo "post_mortem_mem_reads=0"
    echo "mem_invariant_violations=0"
    echo "RUNNER_EXIT=0"
  } > "$b.runner-console.log"
  printf '  duration:            120s\n  csv cadence:         20s\n  /proc/loadavg:       0.05 0.10 0.12 1/200 1234\n' > "$b.matrix.txt"
  echo '{"durationSecsActual": 120, "totalWrites": 31000, "writeErrors": 0, "crashes": 0}' > "$b.soak.json"
  echo '{"elapsedSecs": 120, "totalWrites": 31000}' > "$b.progress.jsonl"
  write_builds "$d/builds.txt"
}
# Runs the predicates over the A cell in $1; A_OUT = stdout+stderr.
a_run() {   # $1 = dir, $2 = manifest (default: the literals-only one)
  local man="${2:-}"
  [ -n "$man" ] || { man="$1/manifest.md"; mkman "$man"; }
  A_OUT="$(SPEC377_SYNTHETIC=1 SPEC377_MANIFEST="$man" bash "$PRED" "$1" "spec377-${A_CELL}" "$1/builds.txt" 2>&1)"
  printf '%s\n' "$A_OUT" | grep -E '^(PV|PALLOC|PALLOC_ITEM_[a-z0-9_]+|JE_CONFIRM_CONF|MI_POSTINIT_LINES_[a-z0-9]+)=' | show
}
a_case() {   # $1 = id, $2 = expected PALLOC line, then extra exact lines
  local id="$1" want="$2" miss="" l; shift 2
  a_write "$SCR/a/$id"; a_run "$SCR/a/$id"
  has_line "$A_OUT" "$want" || miss="${miss} [${want}]"
  for l in "$@"; do has_line "$A_OUT" "$l" || miss="${miss} [${l}]"; done
  [ -z "$miss" ]; verdict "$id" $? "missing:${miss}"
}

a_defaults ssy;  a_case A1 "PALLOC=TRUE label=SYS-ser"
a_defaults ssy;  A_ENV="MALLOC_ARENA_MAX=2"; a_case A2 "PALLOC=FALSE reason=s1 label=SYS-ser"
a_defaults ssy;  a_write "$SCR/a/A3"; B_MARK_JE="je=0 mi=0"; write_builds "$SCR/a/A3/builds.txt"; a_run "$SCR/a/A3"
has_line "$A_OUT" "PALLOC=FALSE reason=marker_control label=SYS-ser"; verdict A3 $? "want reason=marker_control"
a_defaults sje;  a_case A4 "PALLOC=TRUE label=JE-ser" "JE_CONFIRM_CONF=PASS"
a_defaults sje;  A_ENV="MALLOC_CONF=background_thread:true"; A_BG=0
A_ALLOC="$(printf '%s\n' "$JE_CONFIG_LINE" | sed 's/opt_background_thread=true background_thread=true/opt_background_thread=false background_thread=false/')"
a_case A5 "PALLOC=FALSE reason=j1,j2,j3,j4 label=JE-ser"
a_defaults sje;  A_ALLOC="$(printf '%s\n' "$A_ALLOC" | sed 's/^\(\[server\] <jemalloc>: malloc_conf #1 .*\): ""$/\1: "background_thread:true"/')"
a_case A6 "PALLOC=FALSE reason=j4 label=JE-ser" "JE_CONFIRM_CONF=FAIL reason=source1_nonempty"
a_defaults smi3; a_case A7 "PALLOC=TRUE label=MI3-ser"
a_defaults smi2; a_case A8 "PALLOC=TRUE label=MI2-ser"
a_defaults smi2; A_ALLOC="$MI3_BLOCK"; a_case A9 "PALLOC=FALSE reason=m5,mi_version label=MI2-ser"
a_defaults smi2; A_ALLOC="$(printf '%s\n' "$MI2_BLOCK" | sed "s/^\[server\] mimalloc: option '/[server] mimalloc: thread 0x7f3a: option '/")"
a_case A10 "PALLOC=FALSE reason=m5,mi_thread_prefix label=MI2-ser"
a_defaults smi3; A_ENV="@absent"; a_case A11 "PALLOC=FALSE reason=m1 label=MI3-ser" "PALLOC_ITEM_m1=FAIL ALLOC_ENV=absent"
a_defaults smi3; A_POSTINIT=9
A_EXTRA="$(for i in 1 2 3 4 5 6 7 8 9; do echo "[server] mimalloc: warning: synthetic post-init message ${i}"; done)"
a_write "$SCR/a/A12"; a_run "$SCR/a/A12"
A12_PRED=0; { has_line "$A_OUT" "PALLOC=TRUE label=MI3-ser" && has_line "$A_OUT" "MI_POSTINIT_LINES_smi3=9"; } || A12_PRED=1
# (the decide half of A12 runs with the D cases below)

# A13/A14: a series MI cell keys on the committed literal file, whose sha256
# section 1 lists; the file here is the A7 console's own block, tagged.
a_defaults mi3; a_write "$SCR/a/A13"
printf '%s\n' "$MI3_BLOCK" | grep -E "^\[server\] (v3|option '(purge_delay|arena_purge_mult|purge_decommits)')" | sed 's/^/smi3 /' > "$SCR/a/A13/spec377-mi-literals.txt"
LSHA="$(shasum -a 256 "$SCR/a/A13/spec377-mi-literals.txt" | awk '{ print $1 }')"
mkman "$SCR/a/A13/manifest.md" "- \`${LSHA}\` \`packages/server-rust/benches/soak_harness/evidence/spec377-mi-literals.txt\`"
a_run "$SCR/a/A13" "$SCR/a/A13/manifest.md"
has_line "$A_OUT" "PALLOC=TRUE label=MI3-ser"; verdict A13 $? "want PALLOC=TRUE with the literal file bound (lines=$(wc -l < "$SCR/a/A13/spec377-mi-literals.txt"))"
a_defaults mi3; a_write "$SCR/a/A14"
cp "$SCR/a/A13/spec377-mi-literals.txt" "$SCR/a/A14/"; mkman "$SCR/a/A14/manifest.md" "- \`$(sha64 f)\` \`packages/server-rust/benches/soak_harness/evidence/spec377-mi-literals.txt\`"
a_run "$SCR/a/A14" "$SCR/a/A14/manifest.md"
{ has_line "$A_OUT" "PALLOC=FALSE reason=m5 label=MI3-ser" && has_line "$A_OUT" "PALLOC_ITEM_m5=FAIL literals_sha=mismatch"; }
verdict A14 $? "want m5 literals_sha=mismatch"

# One negative per item id: exactly that id is named, nothing else.
n_case() {   # $1 = id; the A_* state was prepared by the caller
  a_write "$SCR/n/$1"; a_run "$SCR/n/$1"
  has_line "$A_OUT" "PALLOC=FALSE reason=$1 label=${A_LABEL}"; verdict "N_$1" $? "want PALLOC=FALSE reason=$1 label=${A_LABEL}"
}
n_builds() {   # $1 = id: rewrite the builds file after a_write (B_* set by the caller)
  a_write "$SCR/n/$1"; write_builds "$SCR/n/$1/builds.txt"; a_run "$SCR/n/$1"
  has_line "$A_OUT" "PALLOC=FALSE reason=$1 label=${A_LABEL}"; verdict "N_$1" $? "want PALLOC=FALSE reason=$1 label=${A_LABEL}"
}
a_defaults ssy;  A_LINE1_SHA="$(sha64 9)"; n_case pv
a_defaults ssy;  A_ENV="GLIBC_TUNABLES=glibc.malloc.arena_max=2"; n_case s1
a_defaults ssy;  A_MARK="je=1 mi=0"; n_case s2
a_defaults ssy;  A_BG=1; n_case s3
a_defaults ssy;  A_EXTRA="$JE_CONFIG_LINE"; n_case s4
a_defaults ssy;  B_TREE_SYS="FAIL reason=tikv-jemalloc-sys_rc=0"; n_builds s5
a_defaults ssy;  A_STEAL=0; n_case s6
a_defaults ssy;  B_MARK_MI2="je=0 mi=0"; n_builds marker_control
a_defaults sje;  A_ENV="_RJEM_MALLOC_CONF=background_thread:true"; n_case j1
a_defaults sje;  A_ALLOC="$(printf '%s\n' "$A_ALLOC" | sed 's/opt_background_thread=true/opt_background_thread=false/')"; n_case j2
a_defaults sje;  A_BG=0; n_case j3
a_defaults sje;  A_ALLOC="$(printf '%s\n' "$A_ALLOC" | grep -v 'Set conf value: confirm_conf')"; n_case j4
a_defaults sje;  A_MARK="je=3 mi=1"; n_case j5
a_defaults sje;  B_TREE_JE="FAIL reason=forbidden_present=x"; n_builds j6
a_defaults smi3; A_ENV="MIMALLOC_VERBOSE=1;MIMALLOC_PURGE_DELAY=0"; n_case m1
a_defaults smi3; A_BG=2; n_case m2
a_defaults smi3; A_MARK="je=0 mi=0"; n_case m3
a_defaults smi3; B_TREE_MI3="FAIL reason=forbidden_present=x"; n_builds m4
a_defaults smi3; A_ALLOC="$(printf '%s\n' "$MI3_BLOCK" | sed "s/'purge_delay': 1000 /'purge_delay': 999 /")"; n_case m5
a_defaults smi3; A_EXTRA="[server] mimalloc: v2.3.2 (built on Sep 29 2026, 09:00:00)"; n_case mi_version
a_defaults smi2; A_EXTRA="[server] mimalloc: thread 0x7f3a: warning: synthetic"; n_case mi_thread_prefix

# ============================================================ B: feature graph
# The chain's own tree_cmd/tree_proof text, extracted, run against a stub
# cargo whose stdout/stderr/rc come from per-crate fixture files.
TREE_SRC="$(sed -n '/^  tree_cmd() {/,/^  guarded_rm() {/p' "$CHAIN" | sed '$d')"
TREE_OK=1
[ "$(printf '%s\n' "$TREE_SRC" | grep -c '^  tree_cmd() {')" = "1" ] && [ "$(printf '%s\n' "$TREE_SRC" | grep -c '^  tree_proof() {')" = "1" ] || TREE_OK=0
[ "$TREE_OK" = "1" ] || echo "  | tree_cmd/tree_proof could not be extracted from spec377-chain.sh"
mkdir -p "$SCR/b/bin"
cat > "$SCR/b/bin/cargo" <<'STUB'
#!/bin/sh
# Stub: "-i <crate>" selects the fixture; everything else is ignored.
crate=""; prev=""
for a in "$@"; do [ "$prev" = "-i" ] && crate="$a"; prev="$a"; done
d="${STUB_TREE_DIR:?}"
[ -f "$d/$crate.out" ] && cat "$d/$crate.out"
[ -f "$d/$crate.err" ] && cat "$d/$crate.err" >&2
exit "$(cat "$d/$crate.rc" 2>/dev/null || echo 99)"
STUB
chmod +x "$SCR/b/bin/cargo"
JE_V_LINE='tikv-jemalloc-sys v0.7.1+5.3.1-0-g81034ce1f1373e37dc865038e1bc8eeecf559ce8'
b_fix() {   # $1 = case, $2 = crate, $3 = rc, $4 = stdout text, $5 = stderr text
  local f="$SCR/b/$1/fix"; mkdir -p "$f" "$SCR/b/$1/src/packages/server-rust" "$SCR/b/$1/run"
  printf '%s' "$4" > "$f/$2.out"; printf '%s' "$5" > "$f/$2.err"; echo "$3" > "$f/$2.rc"
}
b_run() {   # $1 = case, $2 = label; B_OUT
  local d="$SCR/b/$1"
  if [ "$TREE_OK" != "1" ]; then B_OUT="extraction failed"; return; fi
  B_OUT="$(cd "$d" && PATH="$SCR/b/bin:$PATH" STUB_TREE_DIR="$d/fix" SRC="$d/src" RUN_DIR="$d/run" LOG="$d/log" \
    bash -c 'say() { printf "%s\n" "$*" | tee -a "$LOG"; }; eval "$1"; tree_proof "$2"' _ "$TREE_SRC" "$2" 2>&1)"
  printf '%s\n' "$B_OUT" | show
}
NOMATCH_JE='error: package ID specification `tikv-jemalloc-sys` did not match any packages'
NOMATCH_MI='error: package ID specification `libmimalloc-sys` did not match any packages'
b_fix B1 tikv-jemalloc-sys 0 "${JE_V_LINE}
tikv-jemalloc-sys feature \"background_threads_runtime_support\"
tikv-jemalloc-sys feature \"default\"
" ""
b_run B1 JE-ser; has_line "$B_OUT" "TREE_PROOF_JE-ser=PASS reason=none"; verdict B1 $? "want PASS"
b_fix B2 tikv-jemalloc-sys 0 "${JE_V_LINE}
tikv-jemalloc-sys feature \"background_threads_runtime_support\"
tikv-jemalloc-sys feature \"background_threads\"
" ""
b_run B2 JE-ser; has_prefix "$B_OUT" "TREE_PROOF_JE-ser=FAIL reason=forbidden_present="; verdict B2 $? "want FAIL forbidden_present"
b_fix B3 libmimalloc-sys 101 "" ""
b_run B3 MI3-ser; has_prefix "$B_OUT" "TREE_PROOF_MI3-ser=FAIL reason=rc=101"; verdict B3 $? "want FAIL rc=101"
b_fix B4 tikv-jemalloc-sys 101 "" "${NOMATCH_JE}
"
b_fix B4 libmimalloc-sys 101 "" "${NOMATCH_MI}
"
b_run B4 SYS-ser; has_line "$B_OUT" "TREE_PROOF_SYS-ser=PASS reason=none"; verdict B4 $? "want PASS on rc=101 + the no-match stderr"
b_fix B5 tikv-jemalloc-sys 0 "${JE_V_LINE}
" ""
b_fix B5 libmimalloc-sys 101 "" "${NOMATCH_MI}
"
b_run B5 SYS-ser
{ has_prefix "$B_OUT" "TREE_PROOF_SYS-ser=FAIL reason=" && printf '%s\n' "$B_OUT" | grep -q '^TREE_PROOF_SYS-ser=FAIL reason=.*tikv-jemalloc-sys_rc=0'; }
verdict B5 $? "want FAIL naming tikv-jemalloc-sys_rc=0 (crate present)"

# ============================================================ P: 6 h cells
# The reference model of the spec's P10-P15 (SPEC-377 synth expectations):
# live grows 370 000 entries/h from 0, MC = 1400 B/entry, pe = (MC(t) + F/live)
# x (1 + sd x eps), fp = A0 + pe x live / 2^20; eps is AR(1) with coefficient
# phi over Irwin-Hall normals. Rows every 60 s from 0 to D = 21600 (the
# elapsed = D row included); LIVE_COPY every 300 s to D - 300 and TERMINAL at
# D + 2, as a real 6 h cell prints them.
# Params: mc rise (fraction/h) slope (B/h) sd phi fmib seed blank_before
# blank_at shift_at shift_by neg.
gen6h() {   # $1 = ev dir, $2 = cell, then k=v params
  local ev="$1" c="$2" b; shift 2; b="$ev/spec377-$c"
  mkdir -p "$ev" "$b.scrapes"
  awk -v hdr="$HEADER" -v a0="$A0" -v csv="$b.csv" -v con="$b.harness-console.log" -v sha="$SHA_SYS" -v shah="$SHA_H" \
      -v mc=1400 -v rise=0 -v slope=0 -v sd=0.01 -v phi=0 -v fmib=0 -v seed=1 -v blank_before=0 -v blank_at=-1 \
      -v shift_at=-1 -v shift_by=0 -v neg=0 $(for p in "$@"; do printf -- '-v %s ' "$p"; done) '
    function u() { seed = (16807 * seed) % 2147483647; return seed / 2147483647 }
    function z(   i, s) { s = 0; for (i = 0; i < 12; i++) s += u(); return s - 6 }
    function f3(x) { return sprintf("%.3f", x) }
    BEGIN {
      D = 21600; n = split(hdr, h, ","); print hdr > csv; eps = 0
      for (t = 0; t <= D; t += 60) {
        L = 370000 * t / 3600
        m = mc * (1 + rise * t / 3600) + slope * t / 3600
        eps = phi * eps + sqrt(1 - phi * phi) * z()
        fp = a0 + (m * L + fmib * 1048576) * (1 + sd * eps) / 1048576
        if (neg) fp = a0 / 2
        delete v
        v["elapsed_secs"] = t
        v["fp_equiv_mb"] = (t < blank_before || t == blank_at) ? "" : f3(fp)
        v["rss_mb"] = f3(fp * 1.05); v["hwm_rss_mb"] = f3(fp * 1.1); v["lazyfree_mb"] = "0.000"; v["swap_mb"] = "0.000"
        v["anon_mb"] = f3(fp); v["private_dirty_mb"] = f3(fp); v["pss_mb"] = f3(fp); v["smaps_rss_mb"] = f3(fp)
        v["file_mb"] = "1.000"; v["anon_huge_mb"] = "0.000"; v["wal_mb"] = "2.000"; v["redb_mb"] = f3(1 + t / 1000)
        v["disk_total_mb"] = f3(3 + t / 1000); v["tombstone_bytes"] = 100; v["smaps_read_ms"] = 3
        line = ""; for (i = 1; i <= n; i++) line = line (i > 1 ? "," : "") ((h[i] in v) ? v[h[i]] : ""); print line > csv
      }
      print "provenance: server sha256=" sha " flavour=SYS built=2026-09-29T10:00:00Z run_start=2026-09-29T12:00:00Z topgun_or_prune_restored_cancelled_total=present harness sha256=" shah " harness_built=2026-09-29T10:30:00Z tombstone_level_ceiling_gate=present" > con
      print "soak: child TOPGUN_JOURNAL_ENABLED=true" > con
      for (t = 300; t <= D - 300; t += 300) {
        tt = (t == shift_at) ? t + shift_by : t
        printf "  LIVE_COPY  t=%.1fs copy_done=%.1fs keys=96 live=%d live_tag_bytes=1000\n", tt, tt + 0.2, int(370000 * tt / 3600) > con
      }
      printf "  TERMINAL   t=%.1fs copy_done=n/a keys=96 live=%d live_tag_bytes=1000\n", D + 2, int(370000 * (D + 2) / 3600) > con
    }'
  {
    echo "ALLOC_ENV_${c}=none"; echo "JE_BG_THREADS_${c}=0"; echo "BIN_MARKERS_${c}=je=0 mi=0"; echo "MI_POSTINIT_LINES_${c}=0"
    echo "post_mortem_rows=0"; echo "RESULT: instrument sound; harness exit code 0."
    echo "steal_pct=0.0000"; echo "post_mortem_mem_reads=0"; echo "mem_invariant_violations=0"; echo "RUNNER_EXIT=0"
  } > "$b.runner-console.log"
  printf '  duration:            21600s\n  csv cadence:         60s\n  /proc/loadavg:       0.05 0.10 0.12 1/200 1234\n' > "$b.matrix.txt"
  echo '{"durationSecsActual": 21600, "totalWrites": 5564160, "writeErrors": 0, "crashes": 0}' > "$b.soak.json"
  echo '{"elapsedSecs": 21600, "totalWrites": 5564160}' > "$b.progress.jsonl"
  builds_defaults; write_builds "$ev/builds.txt"; mkman "$ev/manifest.md"
}
# The independent reading of a 6 h cell, from its CSV and console only: the
# census join (nearest live row with a number within 30 s), live(t)
# interpolated between retained points, pe, PE_LEVEL, the last-half OLS
# (rows int(n/2)..n-1), lag-1 r1 of the residuals, e, the class and STAGE2_T.
# Written without the predicates' code; it prints K=V lines.
ref6h() {   # $1 = ev dir, $2 = cell
  local b="$1/spec377-$2"
  awk -v csv="$b.csv" -v a0="$A0" -v lw="$LW" -v B="$BAR" -v cap="$CAP" '
    function isn(x) { return x ~ /^-?[0-9]+(\.[0-9]+)?$/ }
    BEGIN {
      D = 21600; FS = ","
      while ((getline l < csv) > 0) {
        k = split(l, f, ",")
        if (!hdr) { for (i = 1; i <= k; i++) c[f[i]] = i; hdr = 1; continue }
        tt = f[c["elapsed_secs"]] + 0; v = f[c["fp_equiv_mb"]]
        if (tt < D && isn(v)) { nr++; rt[nr] = tt; rf[nr] = v + 0 }
      }
      FS = " "
    }
    $1 == "LIVE_COPY" || $1 == "TERMINAL" {
      t = ""; lv = ""
      for (i = 2; i <= NF; i++) { if ($i ~ /^t=/) { t = substr($i, 3); sub(/s$/, "", t) } if ($i ~ /^live=/) lv = substr($i, 6) }
      best = 0; bd = 1e9
      for (j = 1; j <= nr; j++) { d = rt[j] - t; if (d < 0) d = -d; if (d < bd) { bd = d; best = j } }
      if (bd > 30) { drop++; dl = dl (dl == "" ? "" : ",") t; next }
      nk++; kt[nk] = t + 0; kl[nk] = lv + 0
    }
    END {
      print "REF_DROPPED_N=" drop + 0; print "REF_DROPPED=" dl
      n = 0; ls = 0; ln = 0
      for (j = 1; j <= nr; j++) {
        if (nk < 2 || rt[j] < kt[1] || rt[j] > kt[nk]) continue
        for (q = 1; q < nk && !(rt[j] >= kt[q] && rt[j] <= kt[q + 1]); q++) ;
        live = kl[q] + (kl[q + 1] - kl[q]) * (rt[j] - kt[q]) / (kt[q + 1] - kt[q])
        n++; x[n] = rt[j] / 3600; y[n] = (rf[j] - a0) * 1048576 / live; lvv[n] = live
        if (rt[j] >= D - lw) { ls += y[n]; ln++ }
      }
      print "REF_ROWS=" n
      if (ln == 0) { print "REF_CLASS=n/a"; exit }
      L = ls / ln; printf "REF_PE_LEVEL=%.6f\nREF_LEVEL_ROWS=%d\n", L, ln
      s0 = int(n / 2) + 1; m = n - s0 + 1; print "REF_N=" m
      if (m < 90) { print "REF_CLASS=n/a reason=few_rows"; exit }
      if (L <= 0) { print "REF_CLASS=n/a reason=pe_nonpositive"; exit }
      mx = 0; my = 0; for (i = s0; i <= n; i++) { mx += x[i]; my += y[i] } mx /= m; my /= m
      sxx = 0; sxy = 0; for (i = s0; i <= n; i++) { sxx += (x[i] - mx) ^ 2; sxy += (x[i] - mx) * (y[i] - my) }
      bb = sxy / sxx; aa = my - bb * mx; sse = 0; num = 0
      for (i = s0; i <= n; i++) { r[i] = y[i] - aa - bb * x[i]; sse += r[i] ^ 2 }
      for (i = s0 + 1; i <= n; i++) num += r[i] * r[i - 1]
      r1 = num / sse; se = sqrt(sse / (m - 2) / sxx)
      kk = (r1 < 0) ? 1 : sqrt((1 + r1) / (1 - r1)); s = bb / L; e = se / L * kk
      if (r1 >= 0.99) cl = "UNDERPOWERED"
      else if (s - 2 * e > B) cl = "RISING"; else if (s + 2 * e < -B) cl = "FALLING"
      else if ((s < 0 ? -s : s) + 2 * e <= B) cl = "FLAT"; else if (2 * e > B) cl = "UNDERPOWERED"; else cl = "MARGINAL"
      printf "REF_CLASS=%s\nREF_S=%.6f\nREF_E=%.6f\nREF_R1=%.4f\nREF_SLOPE=%.6f\n", cl, s, e, r1, bb
      W = x[n] - x[s0]; T = 2 * W * (2 * e / (0.8 * B)) ^ (2 / 3); Ti = int(T); if (Ti < T) Ti++; if (Ti < D / 3600) Ti = D / 3600
      print "REF_STAGE2_T=" ((r1 >= 0.99 || Ti > cap + 0) ? ">STAGE2_MAX_H" : Ti)
    }' "$b.harness-console.log"
}
# p_case <id> <params...>: builds the cell, runs the predicates and the
# reference; P_OUT (predicates), P_REF (reference).
p_run() {   # $1 = id, then gen6h params
  local id="$1"; shift
  P_EV="$SCR/p/$id"; gen6h "$P_EV" s1 "$@"
  P_OUT="$(SPEC377_SYNTHETIC=1 SPEC377_MANIFEST="$P_EV/manifest.md" bash "$PRED" "$P_EV" spec377-s1 "$P_EV/builds.txt" 2>&1)"
  P_REF="$(ref6h "$P_EV" s1)"
  printf '%s\n' "$P_OUT" | grep -E '^(CENSUS_DROPPED(_N)?_s1|PE_LEVEL_s1|FIT_CHECK_TREND_s1|TREND_s1|STAGE2_T_s1|FIXED_EST_s1|FIXED_NOTE_s1)=' | show
  printf '%s\n' "$P_REF" | tr '\n' ' ' | sed 's/ $/\n/' | show
}
P_TREND() { val "$P_OUT" TREND_s1; }
p_class() { P_TREND | awk '{ print $1 }'; }
# The program and the reference agree on the class and on the level, and the
# class is the one the case names.
p_agree() {   # $1 = expected class; sets P_WHY
  local cl rc lvl rl
  cl="$(p_class)"; rc="$(val "$P_REF" REF_CLASS)"
  lvl="$(val "$P_OUT" PE_LEVEL_s1 | awk '{ print $1 }')"; rl="$(val "$P_REF" REF_PE_LEVEL)"
  P_WHY="class=${cl} ref=${rc} want=$1 level=${lvl} ref_level=${rl}"
  [ "$cl" = "$1" ] && [ "$rc" = "$1" ] && awk -v a="$lvl" -v b="$rl" 'BEGIN { re = "^-?[0-9]+([.][0-9]+)?$"; d = a - b
    exit !(a ~ re && b ~ re && (d < 0 ? -d : d) <= 0.0001) }'
}
# slope_rel and se_rel_adj (fractions/h) against a nominal %/h of the spec's
# reference run (different noise draw and window, so within a band).
p_near() {   # $1 = nominal s %/h, $2 = nominal e %/h
  local s e
  s="$(fld "$(P_TREND)" slope_rel)"; e="$(fld "$(P_TREND)" se_rel_adj)"
  P_WHY="${P_WHY} s%=$(awk -v s="$s" 'BEGIN { printf "%.3f", s * 100 }') e%=$(awk -v e="$e" 'BEGIN { printf "%.3f", e * 100 }') nominal=$1/$2"
  awk -v s="$s" -v e="$e" -v ns="$1" -v ne="$2" 'BEGIN { d = s * 100 - ns; tol = 0.25 + 2 * ne
    exit !((d < 0 ? -d : d) <= tol && e * 100 >= ne / 2 && e * 100 <= ne * 2) }'
}

p_run P1 seed=101; p_agree FLAT; r=$?
P1_DROPPED_N="$(val "$P_OUT" CENSUS_DROPPED_N_s1)"
# TERMINAL (t = D + 2) is the one point dropped by construction.
[ "$r" -eq 0 ] && [ "$(fld "$(val "$P_OUT" PE_LEVEL_s1)" rows)" = "26" ] && [ "$(val "$P_REF" REF_LEVEL_ROWS)" = "26" ] \
  && [ "$P1_DROPPED_N" = "1" ] && [ "$(val "$P_REF" REF_DROPPED_N)" = "1" ] && [ "$(val "$P_OUT" CENSUS_DROPPED_s1)" = "21602.0" ] || r=1
verdict P1 "$r" "${P_WHY} rows=$(fld "$(val "$P_OUT" PE_LEVEL_s1)" rows) dropped=$(val "$P_OUT" CENSUS_DROPPED_s1) (want FLAT, the reference level, 26 rows, only TERMINAL dropped)"
p_run P2 seed=102 rise=0.03; p_agree RISING; verdict P2 $? "$P_WHY"
p_run P3 seed=103 rise=0.003 sd=0.005; p_agree FLAT; verdict P3 $? "$P_WHY"
# Rows blanked before 11 760 s: the first retained census is at 12 000 s, so
# pe.csv holds 156 rows and the last half 78 (< 90).
p_run P4 seed=104 blank_before=11760
t4="$(P_TREND)"; has_prefix "$P_OUT" "TREND_s1=n/a reason=few_rows n=" && [ "$(fld "$t4" n)" -lt 90 ] \
  && [ "$(val "$P_REF" REF_CLASS)" = "n/a reason=few_rows" ]
verdict P4 $? "TREND=${t4} ref=$(val "$P_REF" REF_CLASS) (want few_rows, n < 90)"
# The census at 10 800 s is taken at 10 815 s and the 10 800 s row carries no
# fp_equiv_mb: its nearest numeric row is 45 s away, so it is dropped, and the
# rows around it take live from its neighbours at 10 500 s and 11 100 s.
p_run P5 seed=105 shift_at=10800 shift_by=15 blank_at=10800
jl="$(awk -F, '$1 == 10740 { print $2 }' "$P_EV/spec377-s1.pe-join.csv")"
want_jl="$(awk 'BEGIN { a = int(370000 * 10500 / 3600); b = int(370000 * 11100 / 3600); printf "%.6f", a + (b - a) * 240 / 600 }')"
r=0
[ "$(val "$P_OUT" CENSUS_DROPPED_N_s1)" = "$((P1_DROPPED_N + 1))" ] || r=1
printf '%s' "$(val "$P_OUT" CENSUS_DROPPED_s1)" | tr ',' '\n' | grep -qx '10815.0' || r=1
[ "$jl" = "$want_jl" ] || r=1
[ "$(val "$P_REF" REF_DROPPED_N)" = "$(val "$P_OUT" CENSUS_DROPPED_N_s1)" ] || r=1
verdict P5 "$r" "dropped_n=$(val "$P_OUT" CENSUS_DROPPED_N_s1) (P1 ${P1_DROPPED_N}) dropped=$(val "$P_OUT" CENSUS_DROPPED_s1) live@10740=${jl} want ${want_jl}"
p_run P6 seed=106 sd=0.05 phi=0.9; p_agree UNDERPOWERED; r=$?
[ "$(val "$P_OUT" STAGE2_T_s1)" = "$(val "$P_REF" REF_STAGE2_T)" ] || r=1
verdict P6 "$r" "${P_WHY} STAGE2_T=$(val "$P_OUT" STAGE2_T_s1) ref=$(val "$P_REF" REF_STAGE2_T)"
p_run P7 seed=107 phi=-0.3; p_agree FLAT; r=$?
t7="$(P_TREND)"
awk -v r1="$(fld "$t7" r1)" 'BEGIN { exit !(r1 + 0 < 0) }' && [ "$(fld "$t7" se_rel_adj)" = "$(fld "$t7" se_rel)" ] || r=1
verdict P7 "$r" "${P_WHY} r1=$(fld "$t7" r1) se_rel=$(fld "$t7" se_rel) se_rel_adj=$(fld "$t7" se_rel_adj) (want r1 < 0, no deflation)"
p_run P12 seed=112 fmib=60 sd=0.005; p_agree FLAT; r=$?; p_near -0.69 0.042 || r=1; P12_CLASS="$(p_class)"
verdict P12 "$r" "$P_WHY"
p_run P8 seed=108 fmib=60 sd=0.001
fe="$(val "$P_OUT" FIXED_EST_s1 | awk '{ print $1 }')"
{ within "$fe" 55 65 && has_line "$P_OUT" "FIXED_NOTE_s1=positive_fixed_memory" && [ "$(p_class)" = "$P12_CLASS" ] && [ "$(val "$P_REF" REF_CLASS)" = "$P12_CLASS" ]; }
verdict P8 $? "FIXED_EST=${fe} (want 55..65 MiB) FIXED_NOTE=$(val "$P_OUT" FIXED_NOTE_s1) class=$(p_class) (want P12's ${P12_CLASS})"
p_run P9 seed=109 neg=1
has_prefix "$P_OUT" "TREND_s1=n/a reason=pe_nonpositive"; verdict P9 $? "TREND=$(P_TREND)"
p_run P10 seed=110 rise=0.02 sd=0.005; p_agree RISING; r=$?; p_near 1.74 0.04 || r=1
has_line "$P_OUT" "FIXED_NOTE_s1=rising_marginal_cost_suspected" || r=1
verdict P10 "$r" "${P_WHY} FIXED_NOTE=$(val "$P_OUT" FIXED_NOTE_s1)"
p_run P11 seed=111 fmib=200 sd=0.005; p_agree FALLING; r=$?; p_near -2.07 0.064 || r=1; verdict P11 "$r" "$P_WHY"
p_run P13 seed=113 rise=0.02 sd=0.05; p_agree MARGINAL; r=$?; p_near 1.26 0.41 || r=1; verdict P13 "$r" "$P_WHY"
# P14/P15 assert the pre-registered masking limitation: a genuine rise hidden
# by an amortising fixed term reads FLAT. A change that "fixes" this into
# another class changes the rule and must fail here.
p_run P14 seed=114 rise=0.02 fmib=200 sd=0.005; p_agree FLAT; r=$?; p_near -0.19 0.059 || r=1; verdict P14 "$r" "$P_WHY"
p_run P15 seed=115 rise=0.015 fmib=60 sd=0.005; p_agree FLAT; r=$?; p_near 0.72 0.041 || r=1; verdict P15 "$r" "$P_WHY"
# P16: +0.4 B/h on ~1 500 B (below 1 B/h): the cross-check reads the
# fitter's six printed decimals and still passes.
p_run P16 seed=116 mc=1500 slope=0.4 sd=0.001; p_agree FLAT; r=$?
fc="$(val "$P_OUT" FIT_CHECK_TREND_s1)"; sp="$(fld "$(P_TREND)" slope_pe_per_h)"
[ "${fc%% *}" = "PASS" ] && within "$sp" -1 1 || r=1
verdict P16 "$r" "${P_WHY} FIT_CHECK=${fc} slope_pe_per_h=${sp} (want PASS and |slope| < 1 B/h)"
p_run P17 seed=117 mc=1500 slope=15
fc="$(val "$P_OUT" FIT_CHECK_TREND_s1)"; P17_FITTER="$(fld "$fc" fitter)"; P17_PROG="$(fld "$fc" prog)"; cl="$(p_class)"
case "$cl" in RISING|FALLING|FLAT|UNDERPOWERED|MARGINAL) r=0 ;; *) r=1 ;; esac
# The fitter's printed slope is the six-decimal number the TREND line reports,
# and it lies within 0.05 B/h of the reference refit (the cell's +15 B/h slope).
[ "${fc%% *}" = "PASS" ] && [ "$cl" = "$(val "$P_REF" REF_CLASS)" ] || r=1
printf '%s' "$P17_FITTER" | grep -Eq '^-?[0-9]+\.[0-9]{6}$' && [ "$P17_FITTER" = "$(fld "$(P_TREND)" slope_pe_per_h)" ] || r=1
within "$P17_FITTER" "$(awk -v s="$(val "$P_REF" REF_SLOPE)" 'BEGIN { print s - 0.05 }')" "$(awk -v s="$(val "$P_REF" REF_SLOPE)" 'BEGIN { print s + 0.05 }')" || r=1
verdict P17 "$r" "FIT_CHECK=${fc} class=${cl} ref=$(val "$P_REF" REF_CLASS)"
# P18: P17's cell with the fitter's printed slope moved by 9e-4.
mkdir -p "$SCR/p/P18"; cp -R "$SCR/p/P17/." "$SCR/p/P18/"
P18_SLOPE="$(awk -v s="$P17_FITTER" 'BEGIN { printf "%.6f", s + 0.0009 }')"
P_OUT="$(SPEC377_SYNTHETIC=1 SPEC377_MANIFEST="$SCR/p/P18/manifest.md" SPEC377_TEST_FIT_SLOPE="$P18_SLOPE" bash "$PRED" "$SCR/p/P18" spec377-s1 "$SCR/p/P18/builds.txt" 2>&1)"
printf '%s\n' "$P_OUT" | grep -E '^(FIT_CHECK_TREND_s1|TREND_s1)=' | show
{ has_prefix "$P_OUT" "TREND_s1=n/a reason=fit_mismatch" && has_prefix "$P_OUT" "FIT_CHECK_TREND_s1=FAIL "; }
verdict P18 $? "fitter slope ${P17_FITTER} -> ${P18_SLOPE}: TREND=$(P_TREND)"
# P18b/P18c pin the tolerance itself at 5e-7, not merely "below 9e-4": the
# printed slope is replaced by the program's own slope + 4e-7 (must pass) and
# + 6e-7 (must fail).
p18x() {   # $1 = id, $2 = offset
  mkdir -p "$SCR/p/$1"; cp -R "$SCR/p/P17/." "$SCR/p/$1/"
  P_OUT="$(SPEC377_SYNTHETIC=1 SPEC377_MANIFEST="$SCR/p/$1/manifest.md" SPEC377_TEST_FIT_SLOPE="$(awk -v s="$P17_PROG" -v o="$2" 'BEGIN { printf "%.9f", s + o }')" \
    bash "$PRED" "$SCR/p/$1" spec377-s1 "$SCR/p/$1/builds.txt" 2>&1)"
  printf '%s\n' "$P_OUT" | grep -E '^(FIT_CHECK_TREND_s1|TREND_s1)=' | show
}
printf '%s' "$P17_PROG" | grep -Eq '^-?[0-9]+\.[0-9]{9}$'; P17_OK=$?
p18x P18b 0.0000004
[ "$P17_OK" -eq 0 ] && has_prefix "$P_OUT" "FIT_CHECK_TREND_s1=PASS " && case "$(p_class)" in RISING|FALLING|FLAT|UNDERPOWERED|MARGINAL) true ;; *) false ;; esac
verdict P18b $? "prog=${P17_PROG} + 4e-7: want PASS and a numeric TREND, got $(val "$P_OUT" FIT_CHECK_TREND_s1)"
p18x P18c 0.0000006
[ "$P17_OK" -eq 0 ] && has_prefix "$P_OUT" "FIT_CHECK_TREND_s1=FAIL " && has_prefix "$P_OUT" "TREND_s1=n/a reason=fit_mismatch"
verdict P18c $? "prog=${P17_PROG} + 6e-7: want fit_mismatch, got $(P_TREND)"

# ============================================================ D: the decision
# A decide world = five series cells' predicates files, matrices, soak.json
# and runner consoles, the series chain log, a preflight log, a smoke log and
# a manifest. dw.awk writes one or more worlds from a spec file:
#   WORLD <dir>            starts a world at the D1 defaults
#   <cell>:<KEY>=<value>   a predicates key (PV PEL PM1 PMEM PALLOC PA
#                          PR-crashes WRITE_ERRORS) or a reading (TREND,
#                          PE_LEVEL, ...; suffixed _<cell> in the file)
#   steal:<cell>=  runner_exit:<cell>=  tw:<cell>=  ops:<cell>=<ratio>
#   dur:<cell>=  predrc:<cell>=  mipost:<cell>=  missing=<cell>
#   chain:disk=  chain:preflight=<last line>  man:<KEY>=
# "@absent" as a value leaves the line out. D1: SYS s1/s2 FLAT at 1000/1050,
# JE FLAT at 600, MI3/MI2 MARGINAL at 900/950, every OPS_RATIO 1.
DW_AWK="$SCR/dw.awk"
cat > "$DW_AWK" <<'DWAWK'
function reset(   i, c) {
  delete R; delete X; delete MISS; delete MAN
  for (i = 1; i <= 5; i++) {
    c = CELL[i]
    R[c, "PV"] = "TRUE server_sha256=synthetic"; R[c, "PEL"] = "TRUE rows_with_fp_equiv=360"; R[c, "PM1"] = "TRUE post_mortem_rows=0"
    R[c, "PMEM"] = "TRUE mem_invariant_violations=0 sampler_fatal=0"; R[c, "PALLOC"] = "TRUE label=synthetic"
    R[c, "PA"] = (c == "je") ? "TRUE wellformed_lines=720 lines=720 need=718" : "n/a reason=no_probe_arm"
    R[c, "PR-crashes"] = "TRUE crashes=0"; R[c, "WRITE_ERRORS"] = "0"
    R[c, "TREND"] = "FLAT slope_rel=0.001000 se_rel=0.001000 r1=0.1000 se_rel_adj=0.001100 n=176 r2=0.100000 dropped=1"
    R[c, "TREND3"] = "FLAT slope_rel=0.001000 se_rel=0.001000 r1=0.1000 se_rel_adj=0.001100 n=117"
    R[c, "TREND_CORR"] = "FLAT slope_rel_corr=0.001000 fixed_bias=0.000000 recorded_only"
    R[c, "STAGE2_T"] = "6"; R[c, "FIXED_EST"] = "1.000 se_mib=2.000"; R[c, "FIXED_NOTE"] = "none"
    R[c, "LIVE_END"] = "2220000 src=TERMINAL t=21602.0"; R[c, "PE_END"] = "1000.000000 t=21300.0 live=2189166"
    R[c, "LAZY_MAX"] = "0.000 ratio=0.000000 row=60"; R[c, "HWM_END"] = "3000.000"; R[c, "ANON_HUGE_END"] = "0.000"
    R[c, "DISK_SLOPE"] = "3.600000 se=0.100000 n=176"
    X["steal", c] = "0.0000"; X["runner_exit", c] = "0"; X["tw", c] = "5564160"; X["dur", c] = "21600"; X["predrc", c] = "0"
  }
  R["mi3", "TREND"] = "MARGINAL slope_rel=0.004000 se_rel=0.003000 r1=0.3000 se_rel_adj=0.004000 n=176 r2=0.100000 dropped=1"
  R["mi2", "TREND"] = R["mi3", "TREND"]
  lv["s1"] = 1000; lv["s2"] = 1050; lv["je"] = 600; lv["mi3"] = 900; lv["mi2"] = 950
  for (i = 1; i <= 5; i++) { c = CELL[i]; R[c, "PE_LEVEL"] = lv[c] " rows=26"; R[c, "RSS_PE_LEVEL"] = sprintf("%.3f rows=26", lv[c] * 1.02) }
  X["mipost", "mi3"] = "0"; X["mipost", "mi2"] = "0"
  X["chain", "disk"] = "80000000"; X["chain", "preflight"] = "PREFLIGHT=PASS PREFLIGHT_AT=2026-09-30T00:00:00Z"
}
function out(f, s) { if (s != "@absent") print s > f }
function flush(   i, c, b, k, j, v, pf, ch, np, NP, NR_, RD) {
  if (W == "") return
  system("mkdir -p \"" W "/ev\" \"" W "/smoke\"")
  np = split("PV PEL PA PM1 PMEM PALLOC PR-crashes WRITE_ERRORS", NP, " ")
  NR_ = split("LIVE_END PE_END PE_LEVEL RSS_PE_LEVEL TREND TREND3 TREND_CORR STAGE2_T FIXED_EST FIXED_NOTE LAZY_MAX HWM_END ANON_HUGE_END DISK_SLOPE", RD, " ")
  for (i = 1; i <= 5; i++) {
    c = CELL[i]; b = W "/ev/spec377-" c
    if (!(c in MISS)) {
      f = b ".predicates.txt"; printf "" > f
      for (j = 1; j <= np; j++) if (R[c, NP[j]] != "@absent") print NP[j] "=" R[c, NP[j]] > f
      for (j = 1; j <= NR_; j++) if (R[c, RD[j]] != "@absent") print RD[j] "_" c "=" R[c, RD[j]] > f
      if (c == "je") {
        print "JE_CONFIG=je_config version=5.3.1-0-g81034ce1f1373e37dc865038e1bc8eeecf559ce8 opt_background_thread=true background_thread=true max_background_threads=4" > f
        print "JE_CONFIRM_CONF=PASS" > f; print "AMP_JE_je=1.300000 row=21540" > f; print "FRAG_SHAREL_je=0.100000" > f
        print "DIRTY_SHAREL_je=0.150000" > f; print "REACH_PE_je=700.000 live=2220000" > f
      }
      if ((c == "mi3" || c == "mi2") && X["mipost", c] != "@absent") print "MI_POSTINIT_LINES_" c "=" X["mipost", c] > f
      close(f)
    }
    f = b ".matrix.txt"; printf "" > f
    if (X["dur", c] != "@absent") print "  duration:            " X["dur", c] "s" > f
    print "  csv cadence:         60s" > f; close(f)
    f = b ".soak.json"; print "{" > f
    if (X["tw", c] != "@absent") print "  \"totalWrites\": " X["tw", c] "," > f
    print "  \"writeErrors\": 0," > f; print "  \"crashes\": 0" > f; print "}" > f; close(f)
    f = b ".runner-console.log"; print "RESULT: instrument sound; harness exit code 0." > f
    if (X["steal", c] != "@absent") print "steal_pct=" X["steal", c] > f
    print "post_mortem_mem_reads=0" > f; print "mem_invariant_violations=0" > f
    if (X["runner_exit", c] != "@absent") print "RUNNER_EXIT=" X["runner_exit", c] > f
    close(f)
  }
  pf = "spec377-preflight-20260930T000000Z.log"
  f = W "/ev/" pf; print "CHECK alloc_conf=PASS (synthetic)" > f; print X["chain", "preflight"] > f; close(f)
  f = W "/ev/spec377-chain.log"; print "PROC_ROOT=/proc" > f; print "PREFLIGHT_LOG=" pf > f
  if (X["chain", "disk"] != "@absent") print "DISK_FREE_AT_START=" X["chain", "disk"] " KiB (need >= 40 GiB)" > f
  for (i = 1; i <= 5; i++) if (X["predrc", CELL[i]] != "@absent") print "PREDICATES_EXIT_" CELL[i] "=" X["predrc", CELL[i]] > f
  close(f)
  f = W "/smoke/spec377-chain.log"; print "SMOKE_ADMISSION=PASS failed=none" > f; close(f)
  f = W "/manifest.md"; printf "" > f
  while ((getline l < base) > 0) {
    k = l; sub(/=.*/, "", k)
    if (l ~ /^[A-Z0-9_]+=/ && (k in MAN)) { if (MAN[k] != "@absent") print k "=" MAN[k] > f; continue }
    print l > f
  }
  close(base); print "## APPEND-ONLY BELOW" > f; close(f)
  W = ""
}
BEGIN { split("s1 je mi3 mi2 s2", CELL, " "); W = "" }
/^WORLD / { flush(); W = substr($0, 7); reset(); next }
{
  p = index($0, "="); k = substr($0, 1, p - 1); v = substr($0, p + 1)
  if (k == "missing") { MISS[v] = 1; next }
  q = index(k, ":"); sc = substr(k, 1, q - 1); key = substr(k, q + 1)
  if (sc == "man") MAN[key] = v
  else if (sc == "ops") X["tw", key] = sprintf("%.0f", v * 5564160)
  else if (sc == "steal" || sc == "runner_exit" || sc == "tw" || sc == "dur" || sc == "predrc" || sc == "mipost" || sc == "chain") X[sc, key] = v
  else R[sc, key] = v
}
END { flush() }
DWAWK
d_world() {   # $1 = dir, then override lines
  local w="$1"; shift
  { echo "WORLD $w"; for o in "$@"; do printf '%s\n' "$o"; done; } | awk -v base="$SYN_MAN" -f "$DW_AWK"
}
d_run() {   # $1 = dir; D_OUT, D_RC
  D_OUT="$(SPEC377_SYNTHETIC=1 bash "$DECIDE" "$1/ev" "$1/manifest.md" "$1/smoke" 2>&1)"; D_RC=$?
  printf '%s\n' "$D_OUT" | sed -n '/^== stop predicates ==/,$p' | grep -E '^(STOP|STOP_[HV]_CLAUSES|SYS_AGREE|PLATEAU_SYS|STAGE2_FEASIBLE|STAGE2_COST_EUR|VS_SYS_[A-Z0-9]+|ORDER_AGREE|VERDICT_[A-Z0-9]+|DEFAULT_CANDIDATE|NEXT|CONDUCTOR_RULE)=' | show
  echo "decide rc=${D_RC}" | show
}
# d_case <id> <overrides...> -- <exact lines expected once each>
d_case() {
  local id="$1" ov=() ex=() miss="" l; shift
  while [ "$#" -gt 0 ]; do if [ "$1" = "--" ]; then shift; ex=("$@"); break; fi; ov+=("$1"); shift; done
  d_world "$SCR/d/$id" ${ov[@]+"${ov[@]}"}; d_run "$SCR/d/$id"
  eval "D_OUT_${id}=\$D_OUT"
  [ "$D_RC" -eq 0 ] || miss="${miss} [rc=${D_RC}]"
  for l in ${ex[@]+"${ex[@]}"}; do has_line "$D_OUT" "$l" || miss="${miss} [${l}]"; done
  D_MISS="$miss"
  # A case whose verdict needs a second comparison records it itself.
  [ "${D_DEFER:-0}" = "1" ] && return 0
  [ -z "$miss" ]; verdict "$id" $? "missing:${miss}"
}
# The decision keys (R0.4 items 19-27) of an output, one per line.
dkeys() { printf '%s\n' "$1" | grep -E '^(SYS_AGREE|PLATEAU_SYS|STAGE2_FEASIBLE|STAGE2_COST_EUR|VS_SYS_[A-Z0-9]+|ORDER_AGREE|VERDICT_[A-Z0-9]+|DEFAULT_CANDIDATE|NEXT|CONDUCTOR_RULE)='; }

UP='UNDERPOWERED slope_rel=0.000000 se_rel=0.008000 r1=0.9000 se_rel_adj=0.020000 n=176'
MG='MARGINAL slope_rel=0.004000 se_rel=0.002000 r1=0.2000 se_rel_adj=0.002500 n=176'
FL='FLAT slope_rel=0.000000 se_rel=0.001000 r1=0.1000 se_rel_adj=0.001100 n=176'
RI='RISING slope_rel=0.030000 se_rel=0.002000 r1=0.2000 se_rel_adj=0.002500 n=176'
FA='FALLING slope_rel=-0.030000 se_rel=0.001000 r1=0.2000 se_rel_adj=0.003000 n=176'
D2=("je:PE_LEVEL=950 rows=26" "mi3:PE_LEVEL=950 rows=26" "mi2:PE_LEVEL=960 rows=26")
D7=("je:PE_LEVEL=105 rows=26" "mi3:PE_LEVEL=100 rows=26" "mi2:PE_LEVEL=108 rows=26" "mi3:TREND=$FL" "mi2:TREND=$FL"
    "je:RSS_PE_LEVEL=107 rows=26" "mi3:RSS_PE_LEVEL=102 rows=26" "mi2:RSS_PE_LEVEL=110 rows=26")
D8=("${D7[@]}" "je:RSS_PE_LEVEL=130 rows=26" "mi3:RSS_PE_LEVEL=100 rows=26" "mi2:RSS_PE_LEVEL=100 rows=26")
D13=("s1:TREND=$UP" "s2:TREND=$UP" "s1:STAGE2_T=12" "s2:STAGE2_T=10" "${D2[@]}")
CR1=("je:TREND=$MG" "s1:TREND=$UP" "s2:TREND=$UP" "s1:STAGE2_T=>STAGE2_MAX_H" "s2:STAGE2_T=>STAGE2_MAX_H" "je:STAGE2_T=6")
PROV='THEN;TODO-696+REPLICATE_6H;THEN;STAGE2_PLATEAU;PLATEAU=OPEN'

d_case D1 -- 'VERDICT_JE=CANDIDATE' 'DEFAULT_CANDIDATE=JE' 'NEXT=TODO-696+TODO-695;THEN;DEFAULT_FLIP=JE' 'VS_SYS_JE=0.6000 BETTER' 'SYS_AGREE=TRUE value=0.0488'
d_case D2 "${D2[@]}" -- 'DEFAULT_CANDIDATE=SYS' 'NEXT=KEEP_SYSTEM;SYS_FLAT_WITHIN_BAR' 'PLATEAU_SYS=PLATEAU'
d_case D3 "s2:PE_LEVEL=1500 rows=26" -- 'SYS_AGREE=FALSE value=0.4000' 'PLATEAU_SYS=INDETERMINATE reason=disagree' 'NEXT=CONDUCTOR_RULING;SYS_BIMODAL;SEE=TODO-719'
d_case D4 "mi3:PALLOC=FALSE reason=m1 label=MI3-ser" -- 'STOP=V (mi3:PALLOC=FALSE)' 'SYS_AGREE=STOP' 'PLATEAU_SYS=STOP' 'STAGE2_FEASIBLE=STOP' 'STAGE2_COST_EUR=STOP' \
  'VS_SYS_JE=STOP' 'VS_SYS_MI3=STOP' 'VS_SYS_MI2=STOP' 'ORDER_AGREE=STOP' 'VERDICT_JE=STOP' 'VERDICT_MI3=STOP' 'VERDICT_MI2=STOP' 'DEFAULT_CANDIDATE=STOP' 'NEXT=STOP' 'CONDUCTOR_RULE=STOP'
d_case D5 "missing=je" -- 'STOP=V (je:predicates_missing je:write_errors=absent je:PR-crashes=absent)' 'NEXT=STOP'
d_case D6 "steal:s2=n/a" -- 'STOP=H (s2:steal=missing)' 'NEXT=STOP' 'CONDUCTOR_RULE=STOP'
d_case D7 "${D7[@]}" -- 'VERDICT_JE=CANDIDATE' 'VERDICT_MI3=CANDIDATE' 'VERDICT_MI2=CANDIDATE' 'ORDER_AGREE=TRUE n=3' 'DEFAULT_CANDIDATE=JE'
d_case D8 "${D8[@]}" -- 'ORDER_AGREE=FALSE pe_choice=JE rss_choice=MI3' 'DEFAULT_CANDIDATE=NONE' 'NEXT=CONDUCTOR_RULING;ARM_ORDER_ACCOUNTING'
d_case D9 "ops:je=0.90" -- 'OPS_RATIO_JE=0.9000' 'VERDICT_JE=n/a reason=ops'
d_world "$SCR/d/D10" "man:BETTER_BAR=@absent"; d_run "$SCR/d/D10"
[ "$D_RC" -eq 3 ] && ! printf '%s\n' "$D_OUT" | grep -Eq '^(== flags ==|NEXT=|DEFAULT_CANDIDATE=)'; verdict D10 $? "rc=${D_RC} (want 3, no flags)"
d_world "$SCR/d/D10b" "man:MIN_DISK_GIB=@absent"; d_run "$SCR/d/D10b"
[ "$D_RC" -eq 3 ] && ! printf '%s\n' "$D_OUT" | grep -Eq '^(== flags ==|NEXT=)' && printf '%s\n' "$D_OUT" | grep -q 'MIN_DISK_GIB=absent'
verdict D10b $? "rc=${D_RC} (want 3 naming MIN_DISK_GIB=absent, no flags)"
d_case D11 "s1:TREND=$RI" "s2:TREND=n/a reason=few_rows n=80 dropped=1" -- 'PLATEAU_SYS=INDETERMINATE reason=missing:s2:TREND' 'VERDICT_JE=CANDIDATE' \
  'NEXT=CONDUCTOR_RULING;SYS_MISSING' 'CONDUCTOR_RULE=n/a reason=excluded:CONDUCTOR_RULING;SYS_MISSING'
D_DEFER=1 d_case D11b "s1:TREND=$RI" "s2:TREND=@absent" -- 'TREND_s2=absent'
# The same PLATEAU_SYS, VERDICT_*, DEFAULT_CANDIDATE, NEXT and CONDUCTOR_RULE
# as D11: an absent TREND and an n/a TREND take one path.
routes() { dkeys "$1" | grep -E '^(PLATEAU_SYS|VERDICT_[A-Z0-9]+|DEFAULT_CANDIDATE|NEXT|CONDUCTOR_RULE)='; }
[ -z "$D_MISS" ] && [ "$(routes "$D_OUT_D11")" = "$(routes "$D_OUT_D11b")" ] && [ "$(routes "$D_OUT_D11" | grep -c .)" = "7" ]
verdict D11b $? "missing:${D_MISS} or the routing keys differ from D11"
d_case D12 "s2:PE_LEVEL=@absent" -- 'SYS_AGREE=FALSE reason=missing:s2:PE_LEVEL' 'VS_SYS_JE=n/a reason=missing:s2:PE_LEVEL' 'VS_SYS_MI3=n/a reason=missing:s2:PE_LEVEL' \
  'VS_SYS_MI2=n/a reason=missing:s2:PE_LEVEL' 'PLATEAU_SYS=INDETERMINATE reason=missing:s2:PE_LEVEL' 'NEXT=CONDUCTOR_RULING;SYS_MISSING'
d_case D13 "${D13[@]}" -- 'PLATEAU_SYS=UNDERPOWERED' 'STAGE2_FEASIBLE=TRUE' 'STAGE2_COST_EUR=3.312' 'NEXT=STAGE2;SYS_LONG_CELLS'
d_case D14 "${D13[@]}" "s1:STAGE2_T=>STAGE2_MAX_H" -- 'STAGE2_FEASIBLE=FALSE reason=>STAGE2_MAX_H' 'STAGE2_COST_EUR=n/a reason=>STAGE2_MAX_H' 'NEXT=CONDUCTOR_RULING;SYS_UNRESOLVABLE_BY_SLOPE'
d_case D14b "${D13[@]}" "s1:STAGE2_T=>STAGE2_MAX_H" "s2:STAGE2_T=@absent" -- 'STAGE2_FEASIBLE=FALSE reason=missing:s2:STAGE2_T' 'NEXT=CONDUCTOR_RULING;SYS_UNRESOLVABLE_BY_SLOPE'
# Recorded readings never route: D15 is D2 and D15b is D1 in every decision key.
D_DEFER=1 d_case D15 "${D2[@]}" "s1:FIXED_EST=+500.000 se_mib=1.000" "s1:FIXED_NOTE=positive_fixed_memory" -- 'NEXT=KEEP_SYSTEM;SYS_FLAT_WITHIN_BAR' 'FIXED_EST_s1=+500.000 se_mib=1.000'
[ -z "$D_MISS" ] && [ "$(dkeys "$D_OUT_D15")" = "$(dkeys "$D_OUT_D2")" ]; verdict D15 $? "missing:${D_MISS} or a decision key differs from D2"
D_DEFER=1 d_case D15b "je:FIXED_NOTE=rising_marginal_cost_suspected" "je:TREND_CORR=RISING slope_rel_corr=0.030000 fixed_bias=-0.029000 recorded_only" -- 'NEXT=TODO-696+TODO-695;THEN;DEFAULT_FLIP=JE' \
  'TREND_CORR_je=RISING slope_rel_corr=0.030000 fixed_bias=-0.029000 recorded_only'
[ -z "$D_MISS" ] && [ "$(dkeys "$D_OUT_D15b")" = "$(dkeys "$D_OUT_D1")" ]; verdict D15b $? "missing:${D_MISS} or a decision key differs from D1"
d_case D16 "dur:s2=@absent" -- 'OPS_s2=n/a reason=missing:s2:DURATION' 'OPS_RATIO_JE=n/a reason=missing:s2:OPS' 'OPS_RATIO_MI3=n/a reason=missing:s2:OPS' \
  'OPS_RATIO_MI2=n/a reason=missing:s2:OPS' 'VERDICT_JE=n/a reason=missing:s2:OPS' 'VERDICT_MI3=n/a reason=missing:s2:OPS' 'VERDICT_MI2=n/a reason=missing:s2:OPS' 'NEXT=CONDUCTOR_RULING;OPS'
d_case D18 "je:TREND=$FA" -- 'VERDICT_JE=UNRESOLVED(FALLING)' 'DEFAULT_CANDIDATE=SYS' 'NEXT=KEEP_SYSTEM;SYS_FLAT_WITHIN_BAR' 'CONDUCTOR_RULE=n/a reason=not_conductor_ruling'
d_case CR1 "${CR1[@]}" -- 'STAGE2_FEASIBLE=FALSE reason=>STAGE2_MAX_H' 'NEXT=CONDUCTOR_RULING;SYS_UNRESOLVABLE_BY_SLOPE' "CONDUCTOR_RULE=PROVISIONAL_DEFAULT=JE;STAGE2_T=6;${PROV}"
d_case CR2 "${CR1[@]}" "je:PE_LEVEL=950 rows=26" -- 'VS_SYS_JE=0.9500 NO_GAIN' 'VS_SYS_MI3=0.9000 NO_GAIN' 'CONDUCTOR_RULE=KEEP_SYSTEM;PLATEAU=OPEN;NEXT=TODO-719'
d_case CR3 "${CR1[@]}" "ops:je=0.90" "mi3:PE_LEVEL=750 rows=26" "mi2:PE_LEVEL=780 rows=26" "mi3:STAGE2_T=12" -- 'VERDICT_JE=n/a reason=ops' \
  "CONDUCTOR_RULE=PROVISIONAL_DEFAULT=MI3;STAGE2_T=12;${PROV}"
d_case CR4 "${CR1[@]}" "s2:PE_LEVEL=@absent" -- 'NEXT=CONDUCTOR_RULING;SYS_MISSING' 'CONDUCTOR_RULE=n/a reason=excluded:CONDUCTOR_RULING;SYS_MISSING'
d_case CR5 "s1:TREND=$UP" "s2:TREND=$UP" "s1:STAGE2_T=>STAGE2_MAX_H" "s2:STAGE2_T=>STAGE2_MAX_H" -- 'STAGE2_FEASIBLE=FALSE reason=>STAGE2_MAX_H' 'VERDICT_JE=CANDIDATE' \
  'NEXT=TODO-696+TODO-695;THEN;DEFAULT_FLIP=JE' 'CONDUCTOR_RULE=n/a reason=not_conductor_ruling'
d_case CR6 "${CR1[@]}" "mi3:TREND=@absent" -- 'NEXT=CONDUCTOR_RULING;SYS_UNRESOLVABLE_BY_SLOPE' 'CONDUCTOR_RULE=n/a reason=missing:mi3:TREND'
d_case CR6b "${CR1[@]}" "mi3:TREND=n/a reason=few_rows n=80 dropped=1" -- 'NEXT=CONDUCTOR_RULING;SYS_UNRESOLVABLE_BY_SLOPE' 'CONDUCTOR_RULE=n/a reason=missing:mi3:TREND'
# Step 3a precedes the empty-Lset rule: no arm is better here, yet a missing
# arm TREND still names itself instead of KEEP_SYSTEM.
d_case CR6c "${CR1[@]}" "je:PE_LEVEL=950 rows=26" "mi3:TREND=@absent" -- 'VS_SYS_JE=0.9500 NO_GAIN' 'CONDUCTOR_RULE=n/a reason=missing:mi3:TREND'
d_case CR6d "${CR1[@]}" "mi2:TREND=n/a reason=pe_nonpositive n=176 level=-3.0" -- 'CONDUCTOR_RULE=n/a reason=missing:mi2:TREND'
d_case CR7 "${CR1[@]}" "mi2:TREND=$FL" -- 'VS_SYS_MI2=0.9500 NO_GAIN' 'VERDICT_MI2=FLAT_NO_GAIN' 'CONDUCTOR_RULE=n/a reason=judgement:CONDUCTOR_RULING;SYS_UNRESOLVABLE_BY_SLOPE'
d_case CR8 "${CR1[@]}" "je:TREND=$RI" -- 'VERDICT_JE=RISING' 'CONDUCTOR_RULE=KEEP_SYSTEM;PLATEAU=OPEN;NEXT=TODO-719'
d_case CR9 "${CR1[@]}" "je:STAGE2_T=n/a reason=missing:je:e" -- "CONDUCTOR_RULE=PROVISIONAL_DEFAULT=JE;STAGE2_T=n/a reason=missing:je:e;${PROV}"
d_case CR10 "${D8[@]}" -- 'NEXT=CONDUCTOR_RULING;ARM_ORDER_ACCOUNTING' 'CONDUCTOR_RULE=n/a reason=judgement:CONDUCTOR_RULING;ARM_ORDER_ACCOUNTING'
# STOP-H / STOP-V clauses, their precedence and order.
d_case H1 "chain:preflight=PREFLIGHT=FAIL failed=steal PREFLIGHT_AT=2026-09-30T00:00:00Z" -- 'STOP=H (preflight=FAIL)' 'NEXT=STOP'
d_case H2 "chain:disk=1000" -- 'STOP=H (disk_free=1000KiB<40GiB)' 'NEXT=STOP'
d_case H3 "chain:disk=@absent" -- 'STOP=H (disk_free=absent)' 'NEXT=STOP'
d_case H4 "steal:je=1.5" -- 'STOP=H (je:steal=1.5)' 'NEXT=STOP'
d_case H5 "steal:je=1.5" "runner_exit:s1=1" -- 'STOP=H (je:steal=1.5)' 'STOP_V_CLAUSES=s1:runner_exit=1' 'NEXT=STOP'
d_case V1 "predrc:mi2=4" -- 'STOP=V (mi2:predicates_rc=4)'
d_case V2 "predrc:mi2=@absent" -- 'STOP=V (mi2:predicates_rc=absent)'
d_case V3 "runner_exit:s2=@absent" -- 'STOP=V (s2:runner_exit=absent)'
d_case V4 "s1:WRITE_ERRORS=3" -- 'STOP=V (s1:write_errors=3)' 'WRITE_ERRORS=3'
d_case V5 "s1:PR-crashes=FALSE crashes=1" -- 'STOP=V (s1:PR-crashes=FALSE)'
d_case V6 "je:PA=FALSE wellformed_lines=1 lines=1 need=718" -- 'STOP=V (je:PA=FALSE)'
d_case V7 "s1:PALLOC=@absent" -- 'STOP=V (s1:PALLOC=absent)'
d_case V8 "steal:je=n/a" "runner_exit:s1=1" "missing=mi3" -- 'STOP_H_CLAUSES=je:steal=missing' \
  'STOP_V_CLAUSES=s1:runner_exit=1 mi3:predicates_missing mi3:write_errors=absent mi3:PR-crashes=absent'
d_case V9 "s2:PV=FALSE reason=server_sha" -- 'STOP=V (s2:PV=FALSE)' 'NEXT=STOP'
d_case R11 "s1:TREND=$RI" "je:PE_LEVEL=950 rows=26" -- 'PLATEAU_SYS=RISING' 'VERDICT_JE=FLAT_NO_GAIN' 'NEXT=CONDUCTOR_RULING;FLAT_ARM_VS_RISING_SYS' \
  'CONDUCTOR_RULE=KEEP_SYSTEM;PLATEAU=OPEN;NEXT=TODO-719'
d_case R12 "s1:TREND=$RI" "je:TREND=$MG" -- 'NEXT=CONDUCTOR_RULING;NOTHING_PLATEAUS;SEE=TODO-719' "CONDUCTOR_RULE=PROVISIONAL_DEFAULT=JE;STAGE2_T=6;${PROV}"
d_case R13a "s1:TREND=$FA" "je:TREND=$MG" -- 'PLATEAU_SYS=INDETERMINATE reason=falling' 'NEXT=CONDUCTOR_RULING;SYS_INDETERMINATE'
d_case R13b "s1:TREND=$MG" "je:TREND=$MG" -- 'PLATEAU_SYS=INDETERMINATE reason=marginal' 'NEXT=CONDUCTOR_RULING;SYS_INDETERMINATE'
d_case OOD1 "s1:PE_LEVEL=-5 rows=26" -- 'SYS_AGREE=FALSE reason=out_of_domain:s1:PE_LEVEL' 'PLATEAU_SYS=INDETERMINATE reason=out_of_domain:s1:PE_LEVEL' 'NEXT=CONDUCTOR_RULING;UNMATCHED'
d_case LZ1 "s2:LAZY_MAX=5.000 ratio=0.020000 row=60" -- 'LAZY_DRIFT=TRUE cells=s2'
# A12's decide half: the runner's post-init count above MI_POSTINIT_BOUND is
# a recorded note, never a STOP.
d_world "$SCR/d/A12" "mipost:mi3=9"; d_run "$SCR/d/A12"
[ "$A12_PRED" = "0" ] && has_line "$D_OUT" "MI_POSTINIT_NOTE=mi3:9" && has_line "$D_OUT" "STOP=none"
verdict A12 $? "predicates half rc=${A12_PRED}; want MI_POSTINIT_NOTE=mi3:9 and STOP=none"
# K1: the flags block carries the R0.4 keys in order, each exactly once, with
# and without a STOP (D1, D4).
R04_KEYS="STOP WRITE_ERRORS"
for c in s1 je mi3 mi2 s2; do R04_KEYS="$R04_KEYS OPS_$c"; done
for a in JE MI3 MI2; do R04_KEYS="$R04_KEYS OPS_RATIO_$a"; done
for k in LIVE_END PE_END PE_LEVEL TREND TREND3 TREND_CORR STAGE2_T FIXED_EST FIXED_NOTE RSS_PE_LEVEL LAZY_MAX; do
  for c in s1 je mi3 mi2 s2; do R04_KEYS="$R04_KEYS ${k}_$c"; done
done
R04_KEYS="$R04_KEYS LAZY_DRIFT"
for k in HWM_END ANON_HUGE_END; do for c in s1 je mi3 mi2 s2; do R04_KEYS="$R04_KEYS ${k}_$c"; done; done
R04_KEYS="$R04_KEYS JE_CONFIG JE_CONFIRM_CONF AMP_JE FRAG_SHAREL_je DIRTY_SHAREL_je REACH_PE_je MI_POSTINIT_LINES_mi3 MI_POSTINIT_LINES_mi2 MI_POSTINIT_NOTE"
for c in s1 je mi3 mi2 s2; do R04_KEYS="$R04_KEYS DISK_SLOPE_$c"; done
R04_KEYS="$R04_KEYS SYS_AGREE PLATEAU_SYS STAGE2_FEASIBLE STAGE2_COST_EUR VS_SYS_JE VS_SYS_MI3 VS_SYS_MI2 ORDER_AGREE VERDICT_JE VERDICT_MI3 VERDICT_MI2 DEFAULT_CANDIDATE NEXT CONDUCTOR_RULE"
flag_keys() { printf '%s\n' "$1" | sed -n '/^== flags ==/,$p' | sed -n 's/^\([A-Za-z_0-9-][A-Za-z_0-9-]*\)=.*/\1/p' | tr '\n' ' ' | sed 's/ $//'; }
N_R04="$(printf '%s\n' $R04_KEYS | grep -c .)"
[ "$(flag_keys "$D_OUT_D1")" = "$R04_KEYS" ] && [ "$(flag_keys "$D_OUT_D4")" = "$R04_KEYS" ] && [ "$N_R04" = "104" ]
verdict K1 $? "flag keys differ from R0.4 (want ${N_R04} = 104 keys)"
d_case E2 "s1:TREND=FLATX slope_rel=0.000000" -- 'PLATEAU_SYS=INDETERMINATE reason=out_of_domain:s1:TREND' 'VERDICT_JE=CANDIDATE' 'NEXT=CONDUCTOR_RULING;UNMATCHED' \
  'CONDUCTOR_RULE=n/a reason=excluded:CONDUCTOR_RULING;UNMATCHED'

# ============================================================ E1: enumeration
# Inputs, never derived flags. Factors (levels):
#   STOP (none H V) | TREND_<cell> x5 (RISING FALLING FLAT UNDERPOWERED
#   MARGINAL NA ABS) | SYSLEV (agree: s2 1050; disagree: s2 1500; s2abs) |
#   LEV_<arm> x3 (best 600, tie 640, far 750, nogain 950, abs; min SYS = 1000)
#   | RSS (same = PE x 1.02; reversed = 2000 - PE) | FIX_<cell> x5 (pos neg abs)
#   | CORR_<cell> x5 (the six TREND_CORR values) | ST2_s1, ST2_s2 (le gt abs)
#   | ST2_<arm> x3 (6 gt na abs) | OPS_<cell> x5 (ok short missing).
# Rows: every combination of (TREND_s1, TREND_s2, SYSLEV), and every
# (ST2_s1, ST2_s2) pair on each SYS combination that can read UNDERPOWERED,
# all with STOP=none; then rows are added greedily until every pair of levels
# of every two factors occurs. The PRNG is the Park-Miller generator above.
E1_SEED=20260929
E1="$SCR/e1"; mkdir -p "$E1"
awk -v seed="$E1_SEED" -v spec="$E1/worlds.txt" -v rows="$E1/rows.txt" -v root="$E1" '
  function u() { seed = (16807 * seed) % 2147483647; return seed / 2147483647 }
  function ri(n) { return int(u() * n) }
  function covered_new(r, f, lv,   g, n) { n = 0; for (g = 1; g <= NF_; g++) if (g != f && (r, g) in ROW && !((f, lv, g, ROW[r, g]) in COV)) n++; return n }
  function addrow(   f, g) {
    NROW++
    for (f = 1; f <= NF_; f++) for (g = f + 1; g <= NF_; g++) { COV[f, ROW[NROW, f], g, ROW[NROW, g]] = 1; COV[g, ROW[NROW, g], f, ROW[NROW, f]] = 1 }
  }
  # nv = how many of the leading levels are present, valid inputs; the base
  # rows draw from those only, so the SYS product meets well-formed arms, and
  # the pairwise completion reaches the absent/missing levels.
  function factor(name, levels, nv,   k) { NF_++; FN[NF_] = name; NL[NF_] = split(levels, tmp, " "); NV[NF_] = nv; for (k = 1; k <= NL[NF_]; k++) LV[NF_, k] = tmp[k]; FI[name] = NF_ }
  function valid_fill(r,   f) { for (f = 1; f <= NF_; f++) ROW[r, f] = 1 + ri(NV[f]) }
  function set(r, spec,   n, i, kv, t, k) {   # "FACTOR=level,FACTOR=level"
    n = split(spec, t, ",")
    for (i = 1; i <= n; i++) { split(t[i], kv, "="); for (k = 1; k <= NL[FI[kv[1]]]; k++) if (LV[FI[kv[1]], k] == kv[2]) ROW[r, FI[kv[1]]] = k }
  }
  BEGIN {
    factor("STOP", "none H V", 1)
    split("s1 je mi3 mi2 s2", C, " ")
    for (i = 1; i <= 5; i++) factor("TREND_" C[i], "RISING FALLING FLAT UNDERPOWERED MARGINAL NA ABS", 5)
    factor("SYSLEV", "agree disagree s2abs", 2)
    split("je mi3 mi2", A, " ")
    for (i = 1; i <= 3; i++) factor("LEV_" A[i], "best tie far nogain abs", 4)
    factor("RSS", "same reversed", 2)
    for (i = 1; i <= 5; i++) factor("FIX_" C[i], "pos neg abs", 3)
    for (i = 1; i <= 5; i++) factor("CORR_" C[i], "RISING FALLING FLAT UNDERPOWERED MARGINAL NA", 6)
    factor("ST2_s1", "le gt abs", 2); factor("ST2_s2", "le gt abs", 2)
    for (i = 1; i <= 3; i++) factor("ST2_" A[i], "6 gt na abs", 3)
    for (i = 1; i <= 5; i++) factor("OPS_" C[i], "ok short missing", 1)
    # SYS product rows.
    for (a = 1; a <= 7; a++) for (b = 1; b <= 7; b++) for (s = 1; s <= 3; s++) {
      valid_fill(NROW + 1)
      ROW[NROW + 1, FI["TREND_s1"]] = a; ROW[NROW + 1, FI["TREND_s2"]] = b; ROW[NROW + 1, FI["SYSLEV"]] = s
      addrow()
    }
    # UNDERPOWERED-reaching SYS pairs (agree) x every ST2 pair.
    np = split("4:3 4:4 4:5 3:4 5:4", UPP, " ")
    for (k = 1; k <= np; k++) for (p = 1; p <= 3; p++) for (q = 1; q <= 3; q++) {
      split(UPP[k], ab, ":")
      valid_fill(NROW + 1)
      ROW[NROW + 1, FI["TREND_s1"]] = ab[1]; ROW[NROW + 1, FI["TREND_s2"]] = ab[2]; ROW[NROW + 1, FI["SYSLEV"]] = 1
      ROW[NROW + 1, FI["ST2_s1"]] = p; ROW[NROW + 1, FI["ST2_s2"]] = q; addrow()
    }
    # Route rows: one input per NEXT route the product may not meet by
    # chance (the flips per arm, the arm-order disagreement, SYS plateau with
    # no candidate, a FLAT arm beside a RISING SYS, every arm without ops). They are inputs like any
    # other row; the model below derives what each must print.
    nrt = split("TREND_s1=FLAT,TREND_s2=FLAT,SYSLEV=agree,TREND_je=FLAT,LEV_je=best,TREND_mi3=MARGINAL,TREND_mi2=MARGINAL" \
      ";TREND_s1=UNDERPOWERED,TREND_s2=FLAT,SYSLEV=agree,TREND_je=MARGINAL,TREND_mi3=FLAT,LEV_mi3=best,TREND_mi2=RISING" \
      ";TREND_s1=MARGINAL,TREND_s2=RISING,SYSLEV=agree,TREND_je=FALLING,TREND_mi3=UNDERPOWERED,TREND_mi2=FLAT,LEV_mi2=tie" \
      ";TREND_s1=FLAT,TREND_s2=FLAT,SYSLEV=agree,TREND_je=FLAT,LEV_je=far,TREND_mi3=FLAT,LEV_mi3=best,TREND_mi2=MARGINAL,RSS=reversed" \
      ";TREND_s1=FLAT,TREND_s2=FLAT,SYSLEV=agree,TREND_je=MARGINAL,TREND_mi3=FALLING,TREND_mi2=UNDERPOWERED" \
      ";TREND_s1=RISING,TREND_s2=FLAT,SYSLEV=agree,TREND_je=FLAT,LEV_je=nogain,TREND_mi3=MARGINAL,TREND_mi2=MARGINAL" \
      ";TREND_s1=FLAT,TREND_s2=UNDERPOWERED,SYSLEV=agree,TREND_je=FLAT,LEV_je=best,OPS_je=short,OPS_mi3=short,OPS_mi2=missing", RT, ";")
    for (k = 1; k <= nrt; k++) { valid_fill(NROW + 1); set(NROW + 1, RT[k]); addrow() }
    NBASE = NROW
    # Greedy pairwise completion: fix the first uncovered pair, then give
    # every other factor the level that covers the most new pairs (ties by PRNG).
    do {
      found = 0
      for (f = 1; f <= NF_ && !found; f++) for (g = f + 1; g <= NF_ && !found; g++)
        for (x = 1; x <= NL[f] && !found; x++) for (y = 1; y <= NL[g] && !found; y++)
          if (!((f, x, g, y) in COV)) { found = 1; ff = f; fx = x; gg = g; gy = y }
      if (found) {
        r = NROW + 1; delete tmpr
        ROW[r, ff] = fx; ROW[r, gg] = gy
        for (f = 1; f <= NF_; f++) {
          if (f == ff || f == gg) continue
          best = -1; bl = 1; off = ri(NL[f])
          for (k = 0; k < NL[f]; k++) { lv = 1 + (k + off) % NL[f]; c = covered_new(r, f, lv); if (c > best) { best = c; bl = lv } }
          ROW[r, f] = bl
        }
        addrow()
      }
    } while (found)
    # Mechanical coverage proof.
    miss = 0
    for (f = 1; f <= NF_; f++) for (g = f + 1; g <= NF_; g++) for (x = 1; x <= NL[f]; x++) for (y = 1; y <= NL[g]; y++) if (!((f, x, g, y) in COV)) miss++
    print "E1_FACTORS=" NF_ " E1_BASE_ROWS=" NBASE " E1_ROWS=" NROW " E1_UNCOVERED_PAIRS=" miss > (root "/design.txt")
    # Emit: the rows (factor=level) and the decide worlds (base + twin).
    for (r = 1; r <= NROW; r++) {
      line = "ROW " r; for (f = 1; f <= NF_; f++) line = line " " FN[f] "=" LV[f, ROW[r, f]]; print line > rows
      for (tw = 0; tw <= 1; tw++) emit(r, tw)
    }
  }
  function tr(cl, c) {
    if (cl == "NA") return "n/a reason=few_rows n=80 dropped=1"
    if (cl == "ABS") return "@absent"
    return cl " slope_rel=0.000000 se_rel=0.001000 r1=0.1000 se_rel_adj=0.001100 n=176"
  }
  function emit(r, tw,   i, c, a, lv, pe, v, k, sh, cl, CL) {
    print "WORLD " root "/w" r (tw ? "t" : "") > spec
    v = LV[FI["STOP"], ROW[r, FI["STOP"]]]; k = C[1 + (r % 5)]
    if (v == "H") print "steal:" k "=1.5" > spec
    if (v == "V") print "runner_exit:" k "=1" > spec
    for (i = 1; i <= 5; i++) { c = C[i]; print c ":TREND=" tr(LV[FI["TREND_" c], ROW[r, FI["TREND_" c]]]) > spec }
    v = LV[FI["SYSLEV"], ROW[r, FI["SYSLEV"]]]
    print "s1:PE_LEVEL=1000 rows=26" > spec
    print "s2:PE_LEVEL=" (v == "agree" ? "1050 rows=26" : v == "disagree" ? "1500 rows=26" : "@absent") > spec
    PEV["best"] = 600; PEV["tie"] = 640; PEV["far"] = 750; PEV["nogain"] = 950
    for (i = 1; i <= 3; i++) {
      a = A[i]; lv = LV[FI["LEV_" a], ROW[r, FI["LEV_" a]]]
      if (lv == "abs") { print a ":PE_LEVEL=@absent" > spec; print a ":RSS_PE_LEVEL=@absent" > spec; continue }
      pe = PEV[lv]; print a ":PE_LEVEL=" pe " rows=26" > spec
      print a ":RSS_PE_LEVEL=" (LV[FI["RSS"], ROW[r, FI["RSS"]]] == "same" ? sprintf("%.3f", pe * 1.02) : sprintf("%.3f", 2000 - pe)) " rows=26" > spec
    }
    split("RISING FALLING FLAT UNDERPOWERED MARGINAL NA", CL, " ")
    for (i = 1; i <= 5; i++) {
      c = C[i]
      # The twin moves only recorded readings: FIXED_EST/FIXED_NOTE, TREND_CORR, TREND3.
      k = ROW[r, FI["FIX_" c]]; if (tw) k = 1 + k % 3
      v = LV[FI["FIX_" c], k]
      if (v == "pos") { print c ":FIXED_EST=+120.000 se_mib=5.000" > spec; print c ":FIXED_NOTE=positive_fixed_memory" > spec }
      else if (v == "neg") { print c ":FIXED_EST=-80.000 se_mib=5.000" > spec; print c ":FIXED_NOTE=rising_marginal_cost_suspected" > spec }
      else { print c ":FIXED_EST=@absent" > spec; print c ":FIXED_NOTE=@absent" > spec }
      k = ROW[r, FI["CORR_" c]]; if (tw) k = 1 + k % 6
      cl = CL[k]
      print c ":TREND_CORR=" (cl == "NA" ? "n/a reason=few_rows" : cl " slope_rel_corr=0.000000 fixed_bias=0.000000 recorded_only") > spec
      if (tw) print c ":TREND3=RISING slope_rel=0.050000 se_rel=0.001000 r1=0.1000 se_rel_adj=0.001100 n=117" > spec
      v = LV[FI["OPS_" c], ROW[r, FI["OPS_" c]]]
      if (v == "short") print "ops:" c "=0.90" > spec
      if (v == "missing") print "tw:" c "=@absent" > spec
    }
    v = LV[FI["ST2_s1"], ROW[r, FI["ST2_s1"]]]; print "s1:STAGE2_T=" (v == "le" ? "12" : v == "gt" ? ">STAGE2_MAX_H" : "@absent") > spec
    v = LV[FI["ST2_s2"], ROW[r, FI["ST2_s2"]]]; print "s2:STAGE2_T=" (v == "le" ? "10" : v == "gt" ? ">STAGE2_MAX_H" : "@absent") > spec
    for (i = 1; i <= 3; i++) {
      a = A[i]; v = LV[FI["ST2_" a], ROW[r, FI["ST2_" a]]]
      print a ":STAGE2_T=" (v == "6" ? "6" : v == "gt" ? ">STAGE2_MAX_H" : v == "na" ? "n/a reason=missing:" a ":e" : "@absent") > spec
    }
  }' || echo "E1_GENERATOR_FAILED" >> "$E1/design.txt"
awk -v base="$SYN_MAN" -f "$DW_AWK" "$E1/worlds.txt" || echo "E1_WRITER_FAILED" >> "$E1/design.txt"
cat "$E1/design.txt" | show
# One decide run per world, NJ at a time; each writes <world>/out.txt.
( cd "$E1" && ls -d w[0-9]* ) | xargs -P "$NJ" -I{} sh -c 'SPEC377_SYNTHETIC=1 bash "$1" "$2/{}/ev" "$2/{}/manifest.md" "$2/{}/smoke" > "$2/{}/out.txt" 2>&1; echo "rc=$?" >> "$2/{}/out.txt"' _ "$DECIDE" "$E1"
# The independent model of R7.5-R7.7, R8 and R8.1 over the INPUT levels (not
# over anything decide printed), compared with decide's NEXT,
# DEFAULT_CANDIDATE and CONDUCTOR_RULE; plus the closed-list, one-NEXT,
# zero-UNMATCHED and recorded-readings-never-route assertions.
awk -v root="$E1" -v BETTER="$L_BETTER" -v TIE="$L_TIE" -v OPSMIN="$L_OPS" '
  function pick(S, M,   i, a, m, have) {   # set-based tie over arms in S, levels M
    have = 0; for (i = 1; i <= 3; i++) { a = AR[i]; if ((a in S) && (!have || M[a] < m)) { m = M[a]; have = 1 } }
    for (i = 1; i <= 3; i++) { a = AR[i]; if ((a in S) && M[a] <= m * (1 + TIE)) return a }
    return ""
  }
  function key(file, k,   l, n, v) { n = 0; while ((getline l < file) > 0) if (index(l, k "=") == 1) { n++; v = substr(l, length(k) + 2) } close(file); return n == 1 ? v : "<n=" n ">" }
  BEGIN {
    split("s1 je mi3 mi2 s2", C, " "); split("JE MI3 MI2", AR, " "); AC["JE"] = "je"; AC["MI3"] = "mi3"; AC["MI2"] = "mi2"
    CLOSED = "|STOP|CONDUCTOR_RULING;UNMATCHED|CONDUCTOR_RULING;SYS_MISSING|CONDUCTOR_RULING;SYS_BIMODAL;SEE=TODO-719|CONDUCTOR_RULING;OPS|TODO-696+TODO-695;THEN;DEFAULT_FLIP=JE|TODO-696+TODO-695;THEN;DEFAULT_FLIP=MI3|TODO-696+TODO-695;THEN;DEFAULT_FLIP=MI2|CONDUCTOR_RULING;ARM_ORDER_ACCOUNTING|KEEP_SYSTEM;SYS_FLAT_WITHIN_BAR|STAGE2;SYS_LONG_CELLS|CONDUCTOR_RULING;SYS_UNRESOLVABLE_BY_SLOPE|CONDUCTOR_RULING;FLAT_ARM_VS_RISING_SYS|CONDUCTOR_RULING;NOTHING_PLATEAUS;SEE=TODO-719|CONDUCTOR_RULING;SYS_INDETERMINATE|"
    LEVV["best"] = 600; LEVV["tie"] = 640; LEVV["far"] = 750; LEVV["nogain"] = 950
    PROV = "THEN;TODO-696+REPLICATE_6H;THEN;STAGE2_PLATEAU;PLATEAU=OPEN"
    rows = root "/rows.txt"
    while ((getline l < rows) > 0) {
      n = split(l, t, " "); r = t[2]; delete F
      for (i = 3; i <= n; i++) { p = index(t[i], "="); F[substr(t[i], 1, p - 1)] = substr(t[i], p + 1) }
      N++
      # ---- the model
      stop = F["STOP"]
      for (i = 1; i <= 5; i++) { c = C[i]; T[c] = F["TREND_" c]; if (T[c] == "NA" || T[c] == "ABS") T[c] = "MISSING"
        O[c] = (F["OPS_" c] == "ok") ? 1 : (F["OPS_" c] == "short") ? 0.9 : "" }
      PE["s1"] = 1000; PE["s2"] = (F["SYSLEV"] == "agree") ? 1050 : (F["SYSLEV"] == "disagree") ? 1500 : ""
      for (i = 1; i <= 3; i++) { a = AR[i]; c = AC[a]; lv = F["LEV_" c]; PE[c] = (lv == "abs") ? "" : LEVV[lv]
        RSV[a] = (lv == "abs") ? "" : ((F["RSS"] == "same") ? PE[c] * 1.02 : 2000 - PE[c]) }
      # plateau
      ps = ""
      for (i = 1; i <= 2; i++) { c = (i == 1) ? "s1" : "s2"
        if (T[c] == "MISSING") { ps = "missing"; break } if (PE[c] == "") { ps = "missing"; break } }
      if (ps == "") {
        if (F["SYSLEV"] != "agree") ps = "disagree"
        else if (T["s1"] == "RISING" || T["s2"] == "RISING") ps = "RISING"
        else if (T["s1"] == "FALLING" || T["s2"] == "FALLING") ps = "falling"
        else if (T["s1"] == "FLAT" && T["s2"] == "FLAT") ps = "PLATEAU"
        else if (T["s1"] == "UNDERPOWERED" || T["s2"] == "UNDERPOWERED") ps = "UP"
        else ps = "marginal"
      }
      s2f = 0
      if (ps == "UP") { s2f = 1; bad = 0; gt = 0
        for (i = 1; i <= 2; i++) { c = (i == 1) ? "s1" : "s2"; if (T[c] != "UNDERPOWERED") continue
          if (F["ST2_" c] == "abs") bad = 1; if (F["ST2_" c] == "gt") gt = 1 }
        if (bad || gt) s2f = 0 }
      # arms
      nC = 0; delete CS; delete LS; nL = 0; anyflat = 0; anyfng = 0; allops = 1; firstmiss = ""
      for (i = 1; i <= 3; i++) {
        a = AR[i]; c = AC[a]
        opsmiss = (O["s1"] == "" || O["s2"] == "" || O[c] == "")
        ratio = opsmiss ? "" : O[c] / ((O["s1"] + O["s2"]) / 2)
        vsok = (PE["s2"] != "" && PE[c] != ""); better = vsok && (PE[c] / 1000 <= BETTER + 0)
        if (opsmiss) vd = "OPSMISS"
        else if (ratio < OPSMIN + 0) vd = "OPS"
        else if (T[c] == "MISSING" || PE[c] == "" || !vsok) vd = "NA"
        else if (T[c] == "FLAT") vd = better ? "CANDIDATE" : "FLAT_NO_GAIN"
        else if (T[c] == "RISING") vd = "RISING"
        else vd = "UNRESOLVED"
        if (vd != "OPSMISS" && vd != "OPS") allops = 0
        if (vd == "CANDIDATE") { CS[a] = 1; nC++; LEVA[a] = PE[c]; RSA[a] = RSV[a] }
        if (vd == "FLAT_NO_GAIN") anyfng = 1
        if (T[c] == "FLAT") anyflat = 1
        if (T[c] == "MISSING" && firstmiss == "") firstmiss = c
        if (better && ratio != "" && ratio >= OPSMIN + 0 && T[c] != "RISING") { LS[a] = 1; nL++; LEVL[a] = PE[c] }
      }
      if (nC == 0) { dc = (ps == "PLATEAU") ? "SYS" : "NONE"; oa = 1 }
      else { ch = pick(CS, LEVA); oa = (nC <= 1) ? 1 : (pick(CS, RSA) == ch); dc = oa ? ch : "NONE" }
      if (stop != "none") nx = "STOP"
      else if (ps == "missing") nx = "CONDUCTOR_RULING;SYS_MISSING"
      else if (ps == "disagree") nx = "CONDUCTOR_RULING;SYS_BIMODAL;SEE=TODO-719"
      else if (allops) nx = "CONDUCTOR_RULING;OPS"
      else if (dc == "JE" || dc == "MI3" || dc == "MI2") nx = "TODO-696+TODO-695;THEN;DEFAULT_FLIP=" dc
      else if (nC > 0 && !oa) nx = "CONDUCTOR_RULING;ARM_ORDER_ACCOUNTING"
      else if (dc == "SYS") nx = "KEEP_SYSTEM;SYS_FLAT_WITHIN_BAR"
      else if (ps == "UP" && s2f) nx = "STAGE2;SYS_LONG_CELLS"
      else if (ps == "UP") nx = "CONDUCTOR_RULING;SYS_UNRESOLVABLE_BY_SLOPE"
      else if (ps == "RISING" && anyfng) nx = "CONDUCTOR_RULING;FLAT_ARM_VS_RISING_SYS"
      else if (ps == "RISING") nx = "CONDUCTOR_RULING;NOTHING_PLATEAUS;SEE=TODO-719"
      else nx = "CONDUCTOR_RULING;SYS_INDETERMINATE"
      if (stop != "none") { cr = "STOP"; dc = "STOP" }
      else if (index(nx, "CONDUCTOR_RULING;") != 1) cr = "n/a reason=not_conductor_ruling"
      else if (nx == "CONDUCTOR_RULING;SYS_MISSING" || nx == "CONDUCTOR_RULING;OPS") cr = "n/a reason=excluded:" nx
      else if (firstmiss != "") cr = "n/a reason=missing:" firstmiss ":TREND"
      else if (nL == 0) cr = "KEEP_SYSTEM;PLATEAU=OPEN;NEXT=TODO-719"
      else if (anyflat) cr = "n/a reason=judgement:" nx
      else { pc = pick(LS, LEVL); c = AC[pc]; s = F["ST2_" c]
        st = (s == "6") ? "6" : (s == "gt") ? ">STAGE2_MAX_H" : (s == "na") ? "n/a reason=missing:" c ":e" : "n/a reason=missing:" c ":STAGE2_T"
        cr = "PROVISIONAL_DEFAULT=" pc ";STAGE2_T=" st ";" PROV }
      # ---- decide, as printed
      fo = root "/w" r "/out.txt"; ft = root "/w" r "t/out.txt"
      gnx = key(fo, "NEXT"); gdc = key(fo, "DEFAULT_CANDIDATE"); gcr = key(fo, "CONDUCTOR_RULE"); grc = key(fo, "rc")
      tnx = key(ft, "NEXT"); tdc = key(ft, "DEFAULT_CANDIDATE"); tcr = key(ft, "CONDUCTOR_RULE")
      COUNT[gnx]++; CRF[gcr ~ /^PROVISIONAL_DEFAULT=/ ? "PROVISIONAL_DEFAULT" : gcr ~ /^n\/a reason=judgement:/ ? "judgement" : gcr ~ /^n\/a reason=excluded:/ ? "excluded" : gcr ~ /^n\/a reason=missing:/ ? "missing_TREND" : gcr]++
      if (grc != "0") fail("a1_rc", r, grc)
      if (index(CLOSED, "|" gnx "|") == 0) fail("a4_closed_list", r, gnx)
      if (gnx == "CONDUCTOR_RULING;UNMATCHED") fail("a2_unmatched", r, gnx)
      if (gdc != dc) fail("a3_default_candidate", r, gdc " model=" dc)
      if (gnx != nx) fail("next_model", r, gnx " model=" nx)
      if (gnx != tnx || gcr != tcr || gdc != tdc) fail("a5_recorded_route", r, gnx "/" gcr " twin=" tnx "/" tcr)
      if (gcr != cr) fail("a6_conductor_rule", r, gcr " model=" cr)
    }
    print "E1_INPUTS=" N " E1_DECIDE_RUNS=" 2 * N " E1_FAILS=" NFAIL + 0
    for (k in COUNT) print "E1_NEXT_COUNT " COUNT[k] " " k
    for (k in CRF) print "E1_CR_COUNT " CRF[k] " " k
    for (k in FAILS) print "E1_FAIL_ASSERT " FAILS[k] " " k
    exit (NFAIL > 0)
  }
  function fail(a, r, d) { NFAIL++; FAILS[a]++; if (FAILS[a] <= 3) print "E1_FAIL " a " row=" r " " d }' > "$E1/check.txt"
E1_RC=$?
sort "$E1/check.txt" | show
# Every NEXT value of the closed list except the catch-all is reached, so the
# enumeration exercised every route rather than a corner of the domain.
E1_REACH=0
for v in STOP 'CONDUCTOR_RULING;SYS_MISSING' 'CONDUCTOR_RULING;SYS_BIMODAL;SEE=TODO-719' 'CONDUCTOR_RULING;OPS' \
  'TODO-696+TODO-695;THEN;DEFAULT_FLIP=JE' 'TODO-696+TODO-695;THEN;DEFAULT_FLIP=MI3' 'TODO-696+TODO-695;THEN;DEFAULT_FLIP=MI2' \
  'CONDUCTOR_RULING;ARM_ORDER_ACCOUNTING' 'KEEP_SYSTEM;SYS_FLAT_WITHIN_BAR' 'STAGE2;SYS_LONG_CELLS' 'CONDUCTOR_RULING;SYS_UNRESOLVABLE_BY_SLOPE' \
  'CONDUCTOR_RULING;FLAT_ARM_VS_RISING_SYS' 'CONDUCTOR_RULING;NOTHING_PLATEAUS;SEE=TODO-719' 'CONDUCTOR_RULING;SYS_INDETERMINATE'; do
  awk -v v="$v" '$1 == "E1_NEXT_COUNT" { s = $0; sub(/^E1_NEXT_COUNT [0-9]+ /, "", s); if (s == v) f = 1 } END { exit !f }' "$E1/check.txt" \
    || { echo "  | E1: NEXT=${v} never reached"; E1_REACH=1; }
done
grep -q '^E1_FACTORS=.* E1_UNCOVERED_PAIRS=0$' "$E1/design.txt" && ! grep -q 'FAILED' "$E1/design.txt" && [ "$E1_RC" -eq 0 ] && [ "$E1_REACH" -eq 0 ]
verdict E1 $? "rc=${E1_RC} reach=${E1_REACH} design=$(cat "$E1/design.txt")"

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
# A record for an id outside the list (a failed side check) fails the suite too.
ALL_IDS=" $(printf '%s ' $CASES)"
EXTRA="$(awk -v all="$ALL_IDS" '{ if (index(all, " " $2 " ") == 0 && !seen[$2]++) printf "%s%s", (n++ ? "," : ""), $2 }' "$RESULTS")"
[ -z "$EXTRA" ] || FAILED="${FAILED:+${FAILED},}${EXTRA}"
if [ -z "$FAILED" ]; then
  echo "SYNTH377=PASS cases=${N}"
  exit 0
fi
echo "SYNTH377=FAIL failed=${FAILED}"
exit 1
