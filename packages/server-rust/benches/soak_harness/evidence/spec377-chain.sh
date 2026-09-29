#!/usr/bin/env bash
#
# Linux allocator-series chain (topgun-bench) -- a copy of spec376-chain.sh,
# which is NOT edited. One detached launch per phase.
#
# A COPY EXISTS BECAUSE THE PARENT CHAIN CANNOT RUN THIS SERIES. It builds
# eight calibration flavours at three commits, knows none of the allocator
# treatments this series compares, proves nothing about the feature graph a
# build resolved or the jemalloc configuration a build compiled in, and its
# cal phase is a 1 h calibration, not five 6 h cells. The difference list
# against spec376-chain.sh is CLOSED at exactly eight items:
#
#   1. LINUX ONLY, unchanged in substance: `uname -s` must print Linux,
#      checked before any file is created; anything else is FATAL, exit 2.
#   2. PHASES build | smoke | series (SPEC377_PHASE, required, no default),
#      SPEC377_OUT_DIR / SPEC377_MANIFEST_COMMIT, spec377-* paths and log
#      names, ONE commit: every build is at SERIES_PIN, read from the one
#      ^SERIES_PIN= line of spec377-cells.sh. Labels SYS-ser JE-ser MI3-ser
#      MI2-ser H; the cell -> label map of the cell table.
#   3. BUILD ENV DISCIPLINE. Before every build the two env names the
#      jemalloc build script reads a compile-time malloc_conf from are unset,
#      and any exported name that would still configure the jemalloc build
#      (a *JEMALLOC_SYS_WITH_MALLOC_CONF or *JEMALLOC_OVERRIDE) is FATAL;
#      then BUILD_ENV_MALLOC_CONF=unset label=<label> is printed, once per
#      build, into the chain log and the builds file.
#   4. FEATURE-GRAPH PROOFS, positive AND negative. Before each server build
#      `cargo tree ... -e features -i <crate> --prefix none` runs in the
#      SERIES_PIN checkout; stdout goes to target/spec377-run/tree-*.txt,
#      TREE_RC_<label>= is logged, and whole-line regexes decide: rc 0,
#      non-empty output, every REQUIRED line present, no FORBIDDEN line. SYS
#      is the one case whose expected rc is 101 (the crate is not in the
#      graph), asserted exactly, with cargo's own stderr line. Any failure
#      prints TREE_PROOF_<label>=FAIL reason=... and refuses the build.
#   5. MARKER CONTROLS. After the builds, BIN_MARKERS_<label>= (strings
#      counts of the jemalloc confirm_conf literal and the mimalloc message
#      prefix) for the four server labels goes into the builds file, so a
#      SYS cell's "no allocator literal" reading has its positive controls.
#   6. SMOKE: the program list is every spec377-* program plus the frozen
#      parents they execute; cells ssy -> sje -> smi3 -> smi2 with the
#      predicates after each; the frozen parity run; spec377-synth.sh; the
#      mimalloc startup literals and the jemalloc confirm_conf lines are
#      captured into spec377-mi-literals.txt / spec377-je-conf.txt and
#      checked; SMOKE_ADMISSION= per the admission items. The parent's smaps
#      sample, dhat frame check, shares self-check and calibration self-run
#      have no counterpart in this series and are gone.
#   7. SERIES (evidence dir), replacing the parent's cal phase: refuses a
#      terminal (the phase runs ~31 h and must be detached); prints
#      SERIES_START_UTC=, PREDICTED_END_UTC= and CAP_CROSS_UTC= (server
#      creation, SPEC377_SERVER_CREATED_UTC, + 37 h); ORDER=OK, the smoke->M
#      binding and the preflight gate (which must also carry exactly one
#      CHECK alloc_conf=PASS); DISK_FREE_AT_START= >= MIN_DISK_GIB /opt; a
#      settle step before every cell after the first (SETTLE_<cell>=);
#      s1 -> je -> mi3 -> mi2 -> s2; MemAvailable at the start and end of
#      each cell in the host sidecar; the predicates after all five cells;
#      then the decision program, its rc logged. A failed cell never stops
#      the chain: the decision names the STOP.
#   8. This header, the messages and the log names.
#
# A DIFF HUNK THAT MAPS TO NONE OF THE EIGHT ITEMS IS A DEFECT; the manifest
# carries the hunk-to-item map (diff spec376-chain.sh spec377-chain.sh).
#
# Every line a later reading keys on is printed at column 0, exactly once per
# log: PROC_ROOT=, PREFLIGHT_LOG=, LOAD_AT_START_<cell>=, SETTLE_<cell>=,
# PREDICATES_EXIT_<cell>=, SMOKE_PROG_SHA=<file> (once per program),
# TREE_RC_<label>=, TREE_PROOF_<label>=, BUILD_ENV_MALLOC_CONF=unset
# label=<label> (once per label), SMOKE_ADMISSION=, SERIES_START_UTC=,
# PREDICTED_END_UTC=, CAP_CROSS_UTC=, DISK_FREE_AT_START=. Anything echoed
# from another program's output that is not such a key is indented, so no
# reader can count it twice.
set -uo pipefail
export LC_ALL=C

# Item 1: /proc, ss, GNU tools and the Linux cells behind this chain; refuse
# before anything is created.
if [ "$(uname -s 2>/dev/null || true)" != "Linux" ]; then
  echo "FATAL: spec377-chain.sh runs on Linux only (uname -s = '$(uname -s 2>/dev/null || true)')" >&2
  exit 2
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
SERVER_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd -P)"          # packages/server-rust
REPO_ROOT="$(cd "$SERVER_ROOT/../.." && pwd -P)"
PORT=47376
T_ROOT="${REPO_ROOT}/target"
RUN_DIR="${T_ROOT}/spec377-run"
BUILDS="${RUN_DIR}/spec377-builds.txt"
SMOKE_OUT_PIN="${RUN_DIR}/smoke"
MANIFEST="$SCRIPT_DIR/spec377-manifest.md"
EVREL="packages/server-rust/benches/soak_harness/evidence"
LABELS="SYS-ser JE-ser MI3-ser MI2-ser H"
SERVER_LABELS="SYS-ser JE-ser MI3-ser MI2-ser"
# The host files every reading below takes; one name so nothing reads another root.
PROC=/proc
# Item 7: 5 x (21600 s cell + ~180 s overhead + <= 600 s settle), and the
# 37 h cap counted from server creation.
SERIES_PREDICTED_S=111900
CAP_S=133200
MIN_DISK_GIB="$(sed '/^## APPEND-ONLY BELOW/q' "$MANIFEST" 2>/dev/null | awk -F= '$1 == "MIN_DISK_GIB" { n++; v = $2 } END { sub(/ .*/, "", v); if (n == 1 && v ~ /^[1-9][0-9]?[0-9]?[0-9]?[0-9]?$/) print v }')"   # section 1, the floor decide's STOP-H reads too; empty = absent, dup or malformed
SETTLE_POLL_S=15
SETTLE_MAX_POLLS=40      # 40 x 15 s = 10 min

# The freeze commit lives in one place, the runner; a second definition would
# let the chain and the runner build and check different commits.
[ "$(grep -c '^SERIES_PIN=' "$SCRIPT_DIR/spec377-cells.sh" 2>/dev/null)" = "1" ] \
  || { echo "FATAL: spec377-cells.sh must define SERIES_PIN= exactly once" >&2; exit 1; }
SERIES_PIN="$(sed -n 's/^SERIES_PIN=//p' "$SCRIPT_DIR/spec377-cells.sh")"
printf '%s' "$SERIES_PIN" | grep -Eq '^[0-9a-f]{8,40}$' \
  || { echo "FATAL: SERIES_PIN literal '${SERIES_PIN}' is not a hex commit id" >&2; exit 1; }

PHASE="${SPEC377_PHASE:-}"
case "$PHASE" in build|smoke|series) ;; *) echo "FATAL: SPEC377_PHASE must be build, smoke or series (got '${PHASE}')" >&2; exit 2 ;; esac

# Item 7: a ~31 h phase attached to a terminal dies with the session that
# started it; refuse before any file is created.
if [ "$PHASE" = "series" ] && { [ -t 0 ] || [ -t 1 ] || [ -t 2 ]; }; then
  echo "FATAL: the series phase must be launched detached (systemd-run or setsid nohup, no terminal on stdin/stdout/stderr)" >&2
  exit 2
fi

# The inherited per-run knobs change what a cell measures; this chain sets the
# ones it needs and nothing else may leak in from the caller's environment.
unset SPEC365_OUT_DIR SPEC365_DATA_DIR SPEC365_FORCE SPEC362B_SMOKE_DURATION \
      SPEC365_SMOKE_SAMPLE_INTERVAL SPEC371_SMOKE_LIVE_CENSUS SPEC370_BASE_SUFFIX \
      SPEC371_BASE_SUFFIX SOAK_SERVER_BINARY 2>/dev/null || true

if [ "$PHASE" = "smoke" ]; then
  # The smoke OUT is pinned (gitignored target/, outside every path M commits);
  # the textual check runs before any directory is created.
  OUT="${SPEC377_OUT_DIR:-}"
  OUT="${OUT%/}"
  [ -n "$OUT" ] || { echo "FATAL: the smoke needs SPEC377_OUT_DIR=${SMOKE_OUT_PIN}" >&2; exit 2; }
  case "$OUT/" in "$SCRIPT_DIR"/*) echo "FATAL: the smoke must not write into or under the evidence dir (${OUT})" >&2; exit 2 ;; esac
  [ "$OUT" = "$SMOKE_OUT_PIN" ] || { echo "FATAL: SPEC377_OUT_DIR must be ${SMOKE_OUT_PIN} (got '${OUT}')" >&2; exit 2; }
  mkdir -p "$OUT"; OUT="$(cd "$OUT" && pwd -P)"
  case "$OUT/" in "$SCRIPT_DIR"/*) echo "FATAL: the smoke OUT resolves into the evidence dir (${OUT})" >&2; exit 2 ;; esac
  [ "$OUT" = "$SMOKE_OUT_PIN" ] || { echo "FATAL: the smoke OUT resolves to ${OUT}, not ${SMOKE_OUT_PIN}" >&2; exit 2; }
  export SPEC365_OUT_DIR="$OUT"
  CELLS="ssy sje smi3 smi2"
  LOG="$OUT/spec377-chain.log"
elif [ "$PHASE" = "series" ]; then
  OUT="$SCRIPT_DIR"; CELLS="s1 je mi3 mi2 s2"
  LOG="$OUT/spec377-chain.log"
else
  mkdir -p "$RUN_DIR"
  OUT="$RUN_DIR"; CELLS=""
  LOG="$OUT/spec377-chain-build.log"
fi
: > "$LOG"
say() { echo "$*" | tee -a "$LOG"; }

# ----------------------------------------------------------------- 1. start
CHAIN_START_EPOCH="$(date +%s)"
say "chain start: $(date -u +%Y-%m-%dT%H:%M:%SZ) epoch=${CHAIN_START_EPOCH} phase=${PHASE} HEAD=$(git -C "$REPO_ROOT" rev-parse HEAD) series_pin=${SERIES_PIN}"

# Every frozen program was validated under BWK awk; if `awk` resolved to mawk
# here, the parity those programs rely on would be meaningless.
OA="$(command -v original-awk 2>/dev/null || true)"
[ -n "$OA" ] || { say "FATAL: original-awk is not installed; refusing to run any awk program under another interpreter"; exit 1; }
AWKBIN="${T_ROOT}/spec377-awkbin"
mkdir -p "$AWKBIN" && ln -sfn "$OA" "$AWKBIN/awk" || { say "FATAL: cannot create the awk shim in ${AWKBIN}"; exit 1; }
export PATH="${AWKBIN}:${PATH}"
[ "$(command -v awk)" = "$AWKBIN/awk" ] || { say "FATAL: awk resolves to '$(command -v awk)', not the shim ${AWKBIN}/awk"; exit 1; }
AWK_BANNER="$(awk -version 2>&1 | head -1)"
printf '%s\n' "$AWK_BANNER" | grep -Eq '^awk version [0-9]{8}' \
  || { say "FATAL: awk banner '${AWK_BANNER}' does not match ^awk version [0-9]{8}"; exit 1; }
say "awk shim: ${AWKBIN}/awk -> ${OA} banner='${AWK_BANNER}'"

# ----------------------------------------------------------------- helpers
# The build label a cell's server must come from (the cell table's label map).
cell_label() {
  case "$1" in
    ssy|s1|s2) echo SYS-ser ;;
    sje|je)    echo JE-ser ;;
    smi3|mi3)  echo MI3-ser ;;
    smi2|mi2)  echo MI2-ser ;;
    *)         echo "" ;;
  esac
}
label_commit() { echo "$SERIES_PIN"; }
# One field of the builds-file line of a label; empty unless EXACTLY one line
# carries that label, so a duplicate can never pick "the first one".
builds_field() {   # $1 = label, $2 = field
  [ -f "$BUILDS" ] || return 0
  awk -v l="flavour=$1" -v f="$2" '
    $1 == l { n++; for (i = 2; i <= NF; i++) if (index($i, f "=") == 1) v = substr($i, length(f) + 2) }
    END { if (n == 1) print v }' "$BUILDS"
}
builds_count() { [ -f "$BUILDS" ] && awk -v l="flavour=$1" '$1 == l { n++ } END { print n + 0 }' "$BUILDS" || echo 0; }
# The launched binary must be the built one. Prints nothing on success; on
# failure prints the named reason and returns 1.
label_sha_reason() {   # $1 = label
  local n want path got
  n="$(builds_count "$1")"
  [ "$n" = "1" ] || { [ "$n" = "0" ] && echo "builds_line=absent" || echo "builds_line=dup"; return 1; }
  want="$(builds_field "$1" sha256)"; path="$(builds_field "$1" path)"
  printf '%s' "$want" | grep -Eq '^[0-9a-f]{64}$' || { echo "sha256=${want:-empty}"; return 1; }
  [ -n "$path" ] && [ -x "$path" ] || { echo "binary=missing(${path:-empty})"; return 1; }
  got="$(shasum -a 256 "$path" 2>/dev/null | awk '{print $1}')"
  [ "$got" = "$want" ] || { echo "sha256_mismatch(got=${got:-empty},want=${want})"; return 1; }
}
hits() { strings "$1" 2>/dev/null | grep -cF "$2" || true; }
# The flavour markers, the same rule the runner asserts before its clock; both
# mimalloc majors carry the MI literal.
marker_ok() {   # $1 = label, $2 = binary
  local p d j m
  p="$(hits "$2" 'alloc_probe elapsed_s=')"; d="$(hits "$2" 'DHAT_OUT')"
  j="$(hits "$2" 'je_probe elapsed_s=')";    m="$(hits "$2" 'mimalloc: warning: ')"
  case "$1" in
    JE-*)  [ "$j" -gt 0 ] && [ "$p" -eq 0 ] && [ "$d" -eq 0 ] && [ "$m" -eq 0 ] ;;
    MI*-*) [ "$m" -gt 0 ] && [ "$p" -eq 0 ] && [ "$d" -eq 0 ] && [ "$j" -eq 0 ] ;;
    SYS-*) [ "$p" -eq 0 ] && [ "$d" -eq 0 ] && [ "$j" -eq 0 ] && [ "$m" -eq 0 ] ;;
    H)     [ "$(hits "$2" 'tombstone-byte level ceiling breached')" -gt 0 ] && [ "$(hits "$2" 'soak: child TOPGUN_JOURNAL_ENABLED=')" -gt 0 ] ;;
    *)     false ;;
  esac
}
# Item 5: the allocator message literals a binary carries, counted as the
# runner counts them for the launched binary. An unreadable binary prints a
# named non-numeric value, never 0, so a control can never pass on a failed read.
bin_marker_counts() {   # $1 = binary
  local s je mi
  s="$(strings "$1" 2>/dev/null)" || s=""
  if [ -z "$s" ]; then
    printf 'je=unreadable mi=unreadable'
    return 0
  fi
  je="$(printf '%s\n' "$s" | grep -cF '<jemalloc>: malloc_conf #' || true)"
  mi="$(printf '%s\n' "$s" | grep -cF 'mimalloc: ' || true)"
  case "$je" in ''|*[!0-9]*) je=unreadable ;; esac
  case "$mi" in ''|*[!0-9]*) mi=unreadable ;; esac
  printf 'je=%s mi=%s' "$je" "$mi"
}
# The programs whose bytes a smoke admits and M freezes: every spec377
# program, plus the frozen parents they execute byte-for-byte. The smoke's
# SMOKE_PROG_SHA= lines and M's section-1 list are both drawn by this rule.
FROZEN_PARENTS="spec349c2-fit.awk spec366-p5.awk spec366-p67.awk spec371-predicates.sh spec376-parity.sh spec376-procmem.sh spec376-synth373b-linux.sh"
is_program() {
  case "$1" in spec377-*.sh|spec377-*.awk|spec377-*.py) return 0 ;; esac
  case " $FROZEN_PARENTS " in *" $1 "*) return 0 ;; esac
  return 1
}
# The program names a smoke must have hashed: every spec377 program present
# plus every frozen parent whether present or not, so a missing parent is
# named instead of skipped.
program_names() {
  { (cd "$SCRIPT_DIR" && ls 2>/dev/null) | while read -r f; do
      case "$f" in spec377-*.sh|spec377-*.awk|spec377-*.py) [ -f "$SCRIPT_DIR/$f" ] && echo "$f" ;; esac
    done
    for f in $FROZEN_PARENTS; do echo "$f"; done
  } | sort -u
}
# Exactly-once presence of a column-0 key in a file: prints the value, or
# "absent" / "dup" (R0.6).
key_once() {   # $1 = file, $2 = key
  local n
  [ -f "$1" ] || { echo "absent"; return; }
  n="$(grep -c "^$2=" "$1" 2>/dev/null)"
  case "$n" in
    1) sed -n "s/^$2=//p" "$1" ;;
    0) echo "absent" ;;
    *) echo "dup" ;;
  esac
}
# ISO-8601 UTC (YYYY-MM-DDTHH:MM:SSZ) to epoch seconds by arithmetic, so the
# preflight age does not depend on any date(1) dialect; empty if unparseable.
iso_epoch() {
  printf '%s\n' "$1" | awk '
    /^[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9]Z$/ {
      y = substr($0, 1, 4) + 0; m = substr($0, 6, 2) + 0; d = substr($0, 9, 2) + 0
      H = substr($0, 12, 2) + 0; M = substr($0, 15, 2) + 0; S = substr($0, 18, 2) + 0
      if (m < 1 || m > 12 || d < 1 || d > 31 || H > 23 || M > 59 || S > 60) exit
      if (m <= 2) { y -= 1; m += 12 }
      days = 365 * y + int(y / 4) - int(y / 100) + int(y / 400) + int((153 * (m - 3) + 2) / 5) + d - 719469
      printf "%d\n", days * 86400 + H * 3600 + M * 60 + S
    }'
}
# The inverse, for the printed schedule: epoch seconds to YYYY-MM-DDTHH:MM:SSZ
# (civil-from-days), again with no date(1) dialect; empty if not an integer.
epoch_iso() {
  printf '%s\n' "$1" | awk '
    /^[0-9]+$/ {
      t = $0 + 0; days = int(t / 86400); s = t - days * 86400
      z = days + 719468; era = int(z / 146097); doe = z - era * 146097
      yoe = int((doe - int(doe / 1460) + int(doe / 36524) - int(doe / 146096)) / 365)
      y = yoe + era * 400; doy = doe - (365 * yoe + int(yoe / 4) - int(yoe / 100))
      mp = int((5 * doy + 2) / 153); d = doy - int((153 * mp + 2) / 5) + 1
      m = mp < 10 ? mp + 3 : mp - 9; if (m <= 2) y++
      printf "%04d-%02d-%02dT%02d:%02d:%02dZ\n", y, m, d, int(s / 3600), int((s % 3600) / 60), s % 60
    }'
}
load1() { awk 'NR == 1 && $1 ~ /^[0-9]+(\.[0-9]+)?$/ { print $1 }' "$PROC/loadavg" 2>/dev/null; }
below_half() { awk -v v="$1" 'BEGIN { exit !(v + 0 < 0.5) }'; }
mem_available() { awk '$1 == "MemAvailable:" && $2 ~ /^[0-9]+$/ { n++; v = $2 } END { if (n == 1) print v }' "$PROC/meminfo" 2>/dev/null; }

# ----------------------------------------------------------------- 2. builds
if [ "$PHASE" = "build" ]; then
  command -v cargo >/dev/null 2>&1 || { say "FATAL: cargo is not on PATH"; exit 1; }
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
  SRC="${T_ROOT}/spec377-src-${SERIES_PIN}"
  checkout "$SRC" "$SERIES_PIN"
  : > "$RUN_DIR/.build-records"

  # Item 3: a compile-time malloc_conf (or a jemalloc linked from elsewhere)
  # would change the JE treatment without any runtime env showing it.
  build_env_discipline() {   # $1 = label
    local left
    unset JEMALLOC_SYS_WITH_MALLOC_CONF X86_64_UNKNOWN_LINUX_GNU_JEMALLOC_SYS_WITH_MALLOC_CONF
    left="$(compgen -e | grep -E 'JEMALLOC_(SYS_WITH_MALLOC_CONF|OVERRIDE)$' | tr '\n' ',' || true)"
    [ -z "$left" ] || { say "FATAL: build ${1}: jemalloc build env still exported after the unset: ${left%,}"; exit 1; }
    say "BUILD_ENV_MALLOC_CONF=unset label=${1}"
    echo "BUILD_ENV_MALLOC_CONF=unset label=${1}" >> "$RUN_DIR/.build-records"
  }

  # Item 4: what the feature graph resolved, proven from both sides. Each
  # regex is whole-line; a trailing " (*)" (a repeated node) is allowed.
  tree_cmd() {   # $1 = crate, $2 = features ("" = none), $3 = stdout file, $4 = stderr file; prints rc
    ( cd "$SRC/packages/server-rust" && \
      if [ -n "$2" ]; then cargo tree -p topgun-server --features "$2" -e features -i "$1" --prefix none --target x86_64-unknown-linux-gnu
      else cargo tree -p topgun-server -e features -i "$1" --prefix none --target x86_64-unknown-linux-gnu; fi ) > "$3" 2> "$4"
    echo $?
  }
  # A missing file counts no lines; its absence is named by the rc and
  # empty-output checks that run beside these.
  lines_matching() { local n; n="$(grep -Ec "$2" "$1" 2>/dev/null)"; echo "${n:-0}"; }
  need_line() { [ "$(lines_matching "$1" "$2")" -ge 1 ] || reason="${reason},required_absent=${2}"; }
  no_line()   { [ "$(lines_matching "$1" "$2")" -eq 0 ] || reason="${reason},forbidden_present=${2}"; }
  tree_proof() {   # $1 = label; prints TREE_RC_/TREE_PROOF_ and exits on FAIL
    local l="$1" out err rc c rc2 out2 err2
    reason=""
    local JE_V='^tikv-jemalloc-sys v0\.7\.1\+5\.3\.1-0-g81034ce1f1373e37dc865038e1bc8eeecf559ce8( \(\*\))?$'
    local JE_RT='^tikv-jemalloc-sys feature "background_threads_runtime_support"( \(\*\))?$'
    local JE_BG='^tikv-jemalloc-sys feature "background_threads"( \(\*\))?$'
    local MI_V='^libmimalloc-sys v0\.1\.49$'
    local MI_2='^libmimalloc-sys feature "v2"( \(\*\))?$'
    case "$l" in
      SYS-ser)
        out="$RUN_DIR/tree-${l}-tikv-jemalloc-sys.txt"; err="${out%.txt}.err"
        out2="$RUN_DIR/tree-${l}-libmimalloc-sys.txt"; err2="${out2%.txt}.err"
        rc="$(tree_cmd tikv-jemalloc-sys "" "$out" "$err")"
        rc2="$(tree_cmd libmimalloc-sys "" "$out2" "$err2")"
        say "TREE_RC_${l}=tikv-jemalloc-sys:${rc} libmimalloc-sys:${rc2}"
        # The only case whose expected rc is not 0: the crate is absent from
        # the graph, so cargo refuses the -i spec. Exactly 101, never "non-zero".
        [ "$rc" = "101" ] || reason="${reason},tikv-jemalloc-sys_rc=${rc}"
        [ "$rc2" = "101" ] || reason="${reason},libmimalloc-sys_rc=${rc2}"
        [ "$(grep -cFx 'error: package ID specification `tikv-jemalloc-sys` did not match any packages' "$err" 2>/dev/null || true)" -ge 1 ] \
          || reason="${reason},tikv-jemalloc-sys_stderr_line=absent"
        [ "$(grep -cFx 'error: package ID specification `libmimalloc-sys` did not match any packages' "$err2" 2>/dev/null || true)" -ge 1 ] \
          || reason="${reason},libmimalloc-sys_stderr_line=absent"
        for f in "$out" "$out2"; do
          [ -f "$f" ] || { reason="${reason},$(basename "$f")=absent"; continue; }
          c="$(lines_matching "$f" '^(tikv-jemalloc-sys|libmimalloc-sys)')"
          [ "$c" -eq 0 ] || reason="${reason},$(basename "$f")_crate_lines=${c}"
        done ;;
      JE-ser|MI3-ser|MI2-ser)
        out="$RUN_DIR/tree-${l}.txt"; err="${out%.txt}.err"
        case "$l" in
          JE-ser)  rc="$(tree_cmd tikv-jemalloc-sys alloc-jemalloc "$out" "$err")" ;;
          MI3-ser) rc="$(tree_cmd libmimalloc-sys alloc-mimalloc "$out" "$err")" ;;
          MI2-ser) rc="$(tree_cmd libmimalloc-sys alloc-mimalloc,mimalloc/v2 "$out" "$err")" ;;
        esac
        say "TREE_RC_${l}=${rc}"
        [ "$rc" = "0" ] || reason="${reason},rc=${rc}"
        [ -s "$out" ] || reason="${reason},output=empty"
        case "$l" in
          JE-ser)  need_line "$out" "$JE_V"; need_line "$out" "$JE_RT"; no_line "$out" "$JE_BG" ;;
          MI3-ser) need_line "$out" "$MI_V"; no_line "$out" "$MI_2" ;;
          MI2-ser) need_line "$out" "$MI_V"; need_line "$out" "$MI_2" ;;
        esac ;;
    esac
    if [ -n "$reason" ]; then
      say "TREE_PROOF_${l}=FAIL reason=${reason#,}"
      echo "TREE_PROOF_${l}=FAIL reason=${reason#,}" >> "$RUN_DIR/.build-records"
      say "FATAL: build ${l}: the feature-graph proof failed (tree_${l}=${reason#,})"
      exit 1
    fi
    say "TREE_PROOF_${l}=PASS reason=none"
    { grep "^TREE_RC_${l}=" "$LOG" | tail -1; echo "TREE_PROOF_${l}=PASS reason=none"; } >> "$RUN_DIR/.build-records"
  }

  guarded_rm() {
    local cand="$1" parent resolved l
    parent="$(dirname "$cand")"; mkdir -p "$parent"
    resolved="$(cd "$parent" && pwd -P)/$(basename "$cand")"
    for l in $LABELS; do
      [ "$resolved" = "${T_ROOT}/spec377-${l}" ] && { rm -rf "$resolved"; return 0; }
    done
    say "FATAL: refusing to remove '$resolved': not a chain-owned target dir"; exit 1
  }
  build() {   # $1 = label, $2 = source crate dir, $3 = target dir, $4.. = cargo args
    local fl="$1" src="$2" td="$3"; shift 3
    guarded_rm "$td"
    case "$fl" in H) ;; *) tree_proof "$fl" ;; esac
    build_env_discipline "$fl"
    say "build ${fl}: (cd ${src} && CARGO_TARGET_DIR=${td} cargo build $*)"
    ( cd "$src" && CARGO_TARGET_DIR="$td" cargo build "$@" ) > "$OUT/spec377-build-${fl}.log" 2>&1
    local rc=$?
    tail -3 "$OUT/spec377-build-${fl}.log" | sed 's/^/  | /' >> "$LOG"
    [ "$rc" -eq 0 ] || { say "FATAL: build ${fl} failed rc=${rc}"; exit 1; }
    if grep -q 'Compiling topgun-server v' "$OUT/spec377-build-${fl}.log"; then echo yes; else echo no; fi > "$OUT/.recompiled-${fl}"
  }
  for fl in $LABELS; do
    td="${T_ROOT}/spec377-${fl}"
    case "$fl" in
      SYS-ser) build "$fl" "${SRC}/packages/server-rust" "$td" --release --bin topgun-server ;;
      JE-ser)  build "$fl" "${SRC}/packages/server-rust" "$td" --release --features alloc-jemalloc --bin topgun-server ;;
      MI3-ser) build "$fl" "${SRC}/packages/server-rust" "$td" --release --features alloc-mimalloc --bin topgun-server ;;
      MI2-ser) build "$fl" "${SRC}/packages/server-rust" "$td" --release --features alloc-mimalloc,mimalloc/v2 --bin topgun-server ;;
      H)       build "$fl" "${SRC}/packages/server-rust" "$td" --release --bench soak_harness ;;
    esac
  done

  H_CANDS="$(ls "${T_ROOT}"/spec377-H/release/deps/soak_harness-* 2>/dev/null | grep -vE '\.(d|o|rcgu)' || true)"
  [ "$(printf '%s\n' "$H_CANDS" | grep -c .)" -eq 1 ] || { say "FATAL: expected exactly one soak_harness binary, got: ${H_CANDS}"; exit 1; }
  : > "$BUILDS"
  # The cells compare every binary's mtime with this epoch; smoke and series
  # are later launches, so the bound they need is the build start, not their own.
  echo "build_start_epoch=${CHAIN_START_EPOCH}" >> "$BUILDS"
  for fl in $LABELS; do
    case "$fl" in
      H) b="$H_CANDS" ;;
      *) b="${T_ROOT}/spec377-${fl}/release/topgun-server" ;;
    esac
    code="$(git -C "$REPO_ROOT" rev-parse "$(label_commit "$fl")^{commit}")"
    [ -n "$b" ] && [ -x "$b" ] || { say "FATAL: ${fl} binary missing: '${b}'"; exit 1; }
    rec="$(cat "$OUT/.recompiled-${fl}" 2>/dev/null)"; rm -f "$OUT/.recompiled-${fl}"
    mt="$(date -r "$b" '+%s' 2>/dev/null)"
    marker_ok "$fl" "$b"; marker=$?
    [ "$rec" = "yes" ] || { say "FATAL: ${fl} recompiled=${rec:-absent}"; exit 1; }
    case "$mt" in ''|*[!0-9]*) say "FATAL: ${fl} mtime unreadable ('${mt}')"; exit 1 ;; esac
    [ "$mt" -ge "$CHAIN_START_EPOCH" ] || { say "FATAL: ${fl} binary predates the build start"; exit 1; }
    [ "$marker" -eq 0 ] || { say "FATAL: ${fl} marker mismatch"; exit 1; }
    echo "flavour=${fl} code=${code} path=${b} sha256=$(shasum -a 256 "$b" | awk '{print $1}') mtime=${mt} recompiled=${rec} marker=ok" >> "$BUILDS"
  done
  # Item 5: the controls for the SYS cells' absence reading.
  for fl in $SERVER_LABELS; do
    m="BIN_MARKERS_${fl}=$(bin_marker_counts "$(builds_field "$fl" path)")"
    echo "$m" >> "$BUILDS"; say "$m"
  done
  cat "$RUN_DIR/.build-records" >> "$BUILDS"; rm -f "$RUN_DIR/.build-records"
  ( cd "$SRC" && rustc -vV ) 2>&1 | sed 's/^/rustc: /' >> "$BUILDS"
  echo "glibc: $(ldd --version 2>&1 | head -1)" >> "$BUILDS"
  echo "awk: ${AWK_BANNER}" >> "$BUILDS"
  sed 's/^/  | /' "$BUILDS" >> "$LOG"
  say "builds file: ${BUILDS}"
  say "chain end: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  exit 0
fi

# ----------------------------------------------------------------- 3. series gates
# Only the series phase is pre-registered: the schedule, ORDER, the smoke->M
# binding, the preflight gate and the disk check run before any cell, and any
# of them refuses the phase.
if [ "$PHASE" = "series" ]; then
  # Item 7: the schedule the conductor plans the working-hours end and the cap
  # against. The cap is counted from server creation, which only the
  # conductor knows; it is required, never guessed.
  CREATED="${SPEC377_SERVER_CREATED_UTC:-}"
  CREATED_EPOCH="$(iso_epoch "$CREATED")"
  [ -n "$CREATED_EPOCH" ] || { say "FATAL: SPEC377_SERVER_CREATED_UTC='${CREATED}' is not YYYY-MM-DDTHH:MM:SSZ (the server creation time)"; exit 1; }
  [ "$CREATED_EPOCH" -le "$CHAIN_START_EPOCH" ] || { say "FATAL: SPEC377_SERVER_CREATED_UTC=${CREATED} is after the chain start"; exit 1; }
  PRED_END_EPOCH=$((CHAIN_START_EPOCH + SERIES_PREDICTED_S))
  CAP_EPOCH=$((CREATED_EPOCH + CAP_S))
  say "SERIES_START_UTC=$(epoch_iso "$CHAIN_START_EPOCH")"
  say "PREDICTED_END_UTC=$(epoch_iso "$PRED_END_EPOCH") series_s=${SERIES_PREDICTED_S}"
  say "CAP_CROSS_UTC=$(epoch_iso "$CAP_EPOCH") created=${CREATED} cap_s=${CAP_S}"
  # Fetch and delete take about an hour after the chain ends; the cap is the
  # conductor's STOP, so a tight margin is announced, never enforced here.
  say "cap headroom after predicted end + 1 h: $((CAP_EPOCH - PRED_END_EPOCH - 3600))s$( [ $((CAP_EPOCH - PRED_END_EPOCH - 3600)) -lt 0 ] && echo ' (NEGATIVE: the predicted end + 1 h crosses the cap)')"

  # spec377-order.sh cannot vouch for itself: check its bytes against M's
  # section-1 listing before it is trusted.
  order_sha_ok() {   # $1 = M; prints nothing, returns 0 iff order.sh hashes as M lists it
    local want got
    want="$(git -C "$REPO_ROOT" show "${1}:${EVREL}/spec377-manifest.md" 2>/dev/null \
      | sed '/^## APPEND-ONLY BELOW/q' | sed -nE 's/^- `([0-9a-f]{64})` `packages\/server-rust\/benches\/soak_harness\/evidence\/spec377-order\.sh`.*/\1/p')"
    got="$(shasum -a 256 "$SCRIPT_DIR/spec377-order.sh" 2>/dev/null | awk '{print $1}')"
    [ "$(printf '%s\n' "$want" | grep -c .)" -eq 1 ] && [ "$want" = "$got" ]
  }
  MC="${SPEC377_MANIFEST_COMMIT:-}"
  [ -n "$MC" ] || { say "FATAL: SPEC377_MANIFEST_COMMIT is required for the series phase"; exit 1; }
  if order_sha_ok "$MC"; then
    ORDER_LINE="$(bash "$SCRIPT_DIR/spec377-order.sh" "$MC")"
    orc=$?
  else
    ORDER_LINE="ORDER=FAIL spec377-order.sh does not hash as M '${MC}' lists it"; orc=3
  fi
  say "$ORDER_LINE"
  { [ "$orc" -eq 0 ] && printf '%s\n' "$ORDER_LINE" | grep -Eq '^ORDER=OK( |$)'; } \
    || { say "FATAL: ORDER is not OK; refusing to run"; exit 1; }

  # Smoke -> M binding: the admitting smoke must have run exactly the bytes M
  # freezes. Both sides are drawn by is_program(); any difference is named.
  SMOKE_LOG="$SCRIPT_DIR/spec377-smoke/spec377-chain.log"
  M_PROGS="$(git -C "$REPO_ROOT" show "${MC}:${EVREL}/spec377-manifest.md" 2>/dev/null | sed '/^## APPEND-ONLY BELOW/q' \
    | sed -nE 's/^- `([0-9a-f]{64})` `packages\/server-rust\/benches\/soak_harness\/evidence\/([^`\/]+)`.*/\2 \1/p' \
    | while read -r f s; do is_program "$f" && echo "$f $s"; done)"
  S_PROGS="$( [ -f "$SMOKE_LOG" ] && sed -nE 's/^SMOKE_PROG_SHA=([^ ]+) sha256=([0-9a-f]{64})$/\1 \2/p' "$SMOKE_LOG")"
  BIND_FAIL=""
  [ -f "$SMOKE_LOG" ] || BIND_FAIL="${BIND_FAIL} smoke_log=absent"
  [ -n "$M_PROGS" ] || BIND_FAIL="${BIND_FAIL} manifest_programs=absent"
  while read -r f s; do
    [ -n "$f" ] || continue
    n="$(printf '%s\n' "$S_PROGS" | awk -v f="$f" '$1 == f { n++ } END { print n + 0 }')"
    if [ "$n" -eq 0 ]; then BIND_FAIL="${BIND_FAIL} ${f}=absent"
    elif [ "$n" -gt 1 ]; then BIND_FAIL="${BIND_FAIL} ${f}=dup"
    elif [ "$(printf '%s\n' "$S_PROGS" | awk -v f="$f" '$1 == f { print $2 }')" != "$s" ]; then BIND_FAIL="${BIND_FAIL} ${f}=changed"
    fi
  done <<< "$M_PROGS"
  # A malformed SMOKE_PROG_SHA= line (no 64-hex sha) is not silently dropped.
  if [ -f "$SMOKE_LOG" ]; then
    bad="$(grep '^SMOKE_PROG_SHA=' "$SMOKE_LOG" | grep -Evc '^SMOKE_PROG_SHA=[^ ]+ sha256=[0-9a-f]{64}$')"
    [ "$bad" -eq 0 ] || BIND_FAIL="${BIND_FAIL} smoke_prog_sha=malformed(${bad})"
  fi
  while read -r f s; do
    [ -n "$f" ] || continue
    printf '%s\n' "$M_PROGS" | awk -v f="$f" '$1 == f { found = 1 } END { exit !found }' || BIND_FAIL="${BIND_FAIL} ${f}=unlisted"
  done <<< "$(printf '%s\n' "$S_PROGS" | sort -u)"
  if [ -n "$BIND_FAIL" ]; then
    say "SMOKE_BINDING=FAIL${BIND_FAIL}"
    say "FATAL: the admitting smoke did not run the bytes M freezes; re-smoke before M is re-frozen"
    exit 1
  fi
  say "SMOKE_BINDING=PASS programs=$(printf '%s\n' "$M_PROGS" | grep -c .)"

  # The preflight gate: the newest preflight log in the evidence dir, whose
  # LAST line must be PREFLIGHT=PASS with a PREFLIGHT_AT= at most 60 min old.
  PF="$(ls "$SCRIPT_DIR"/spec377-preflight-*.log 2>/dev/null | sort | tail -1)"
  [ -n "$PF" ] || { say "FATAL: no spec377-preflight-*.log in the evidence dir; run spec377-preflight.sh first"; exit 1; }
  [ -s "$PF" ] || { say "FATAL: preflight log $(basename "$PF") is empty"; exit 1; }
  PF_LAST="$(tail -n 1 "$PF")"
  printf '%s\n' "$PF_LAST" | grep -Eq '^PREFLIGHT=PASS( |$)' \
    || { say "FATAL: preflight log $(basename "$PF") does not end with PREFLIGHT=PASS (last line: '${PF_LAST}')"; exit 1; }
  # Item 7: a PASS whose allocator-conf row could not be completed (no JE
  # build yet) is a host-hygiene PASS only; the series needs the full row.
  printf '%s\n' "$PF_LAST" | tr ' ' '\n' | grep -q '^pending=' \
    && { say "FATAL: preflight log $(basename "$PF") carries a pending check (${PF_LAST}); re-run the preflight after the builds"; exit 1; }
  PF_AC_N="$(grep -c '^CHECK alloc_conf=' "$PF" 2>/dev/null)"
  PF_AC_OK="$(grep -Ec '^CHECK alloc_conf=PASS( |$)' "$PF" 2>/dev/null)"
  { [ "$PF_AC_N" = "1" ] && [ "$PF_AC_OK" = "1" ]; } \
    || { say "FATAL: preflight log $(basename "$PF") must carry exactly one CHECK alloc_conf=PASS (alloc_conf lines=${PF_AC_N:-0}, PASS=${PF_AC_OK:-0})"; exit 1; }
  PF_AT_N="$(printf '%s\n' "$PF_LAST" | tr ' ' '\n' | grep -c '^PREFLIGHT_AT=')"
  [ "$PF_AT_N" -eq 1 ] || { say "FATAL: preflight last line carries ${PF_AT_N} PREFLIGHT_AT= fields, need exactly 1"; exit 1; }
  PF_AT="$(printf '%s\n' "$PF_LAST" | tr ' ' '\n' | sed -n 's/^PREFLIGHT_AT=//p')"
  PF_EPOCH="$(iso_epoch "$PF_AT")"
  [ -n "$PF_EPOCH" ] || { say "FATAL: PREFLIGHT_AT='${PF_AT}' is not YYYY-MM-DDTHH:MM:SSZ"; exit 1; }
  PF_AGE=$((CHAIN_START_EPOCH - PF_EPOCH))
  { [ "$PF_AGE" -ge 0 ] && [ "$PF_AGE" -le 3600 ]; } \
    || { say "FATAL: preflight PASS is ${PF_AGE}s before the chain start; it must be 0..3600s"; exit 1; }
  say "PREFLIGHT_LOG=$(basename "$PF")"
  say "preflight: PREFLIGHT_AT=${PF_AT} age=${PF_AGE}s alloc_conf=PASS"

  # Item 7: five 6 h cells write WAL, redb and CSVs under /opt; refuse a disk
  # that cannot hold them rather than lose a cell to ENOSPC hours in.
  DF="$(df -Pk /opt 2>/dev/null | awk 'NR == 2 && $4 ~ /^[0-9]+$/ { print $4 }')"
  say "DISK_FREE_AT_START=${DF:-absent} KiB (need >= ${MIN_DISK_GIB:-<section-1 MIN_DISK_GIB unusable>} GiB)"
  case "$DF" in ''|*[!0-9]*) say "FATAL: /opt free space unreadable; refusing to run"; exit 1 ;; esac
  [ -n "$MIN_DISK_GIB" ] && [ "$DF" -ge $((MIN_DISK_GIB * 1048576)) ] || { say "FATAL: /opt has ${DF} KiB free, below MIN_DISK_GIB=${MIN_DISK_GIB:-unusable} GiB; refusing to run"; exit 1; }
fi

# ----------------------------------------------------------------- 4. cells
# smoke and series: the binaries are the build phase's, asserted by label.
BSE_N="$(grep -c '^build_start_epoch=' "$BUILDS" 2>/dev/null)"; BSE_N="${BSE_N:-0}"
[ "$BSE_N" = "1" ] || { say "FATAL: ${BUILDS} must carry exactly one build_start_epoch= line (found ${BSE_N})"; exit 1; }
BUILD_START_EPOCH="$(sed -n 's/^build_start_epoch=//p' "$BUILDS")"
case "$BUILD_START_EPOCH" in ''|*[!0-9]*) say "FATAL: build_start_epoch='${BUILD_START_EPOCH}' is not an integer"; exit 1 ;; esac
NEED_LABELS="H"
for c in $CELLS; do
  l="$(cell_label "$c")"
  case " $NEED_LABELS " in *" $l "*) ;; *) NEED_LABELS="${NEED_LABELS} ${l}" ;; esac
done
for l in $NEED_LABELS; do
  r="$(label_sha_reason "$l")" || { say "FATAL: label ${l}: ${r}; refusing to start the ${PHASE} phase"; exit 1; }
  [ "$(builds_field "$l" marker)" = "ok" ] || { say "FATAL: label ${l}: builds line lacks marker=ok"; exit 1; }
  say "label ${l}: sha256=$(builds_field "$l" sha256) matches the builds file"
done
BIN_H="$(builds_field H path)"

if [ "$PHASE" = "smoke" ]; then
  # The bytes this smoke admits, one line per program (item 6); a frozen
  # parent that is missing prints no line and fails the admission by name.
  for f in $(program_names); do
    [ -f "$SCRIPT_DIR/$f" ] || { say "smoke program missing: ${f}"; continue; }
    say "SMOKE_PROG_SHA=${f} sha256=$(shasum -a 256 "$SCRIPT_DIR/$f" | awk '{print $1}')"
  done
fi
# The runner reads memory from /proc and has no knob that changes it; this
# line is what the decision reading checks to know no fixture root ran.
say "PROC_ROOT=/proc"

HOSTLOG="$OUT/spec377-host.log"
: > "$HOSTLOG"
export SPEC377_CHAIN_START_EPOCH="$BUILD_START_EPOCH"
export SPEC377_HARNESS_BIN="$BIN_H"

# Item 7: the previous cell's own exit (teardown, fsync, page reclaim) raises
# the 1-min load; a cell started on that tail would read it as its own noise.
# Recorded, never a STOP: steal stays the host gate.
settle() {   # $1 = cell
  local c="$1" t0 l="" polls=0 st=timeout
  t0="$(date +%s)"
  while :; do
    l="$(load1)"
    if [ -n "$l" ] && below_half "$l"; then st=ok; break; fi
    [ "$polls" -ge "$SETTLE_MAX_POLLS" ] && break
    polls=$((polls + 1))
    sleep "$SETTLE_POLL_S"
  done
  say "SETTLE_${c}=${st} waited_s=$(( $(date +%s) - t0 )) load1=${l:-n/a}"
}

run_cell() {   # $1 = cell
  local c="$1" rc l commit r rpid ma
  l="$(cell_label "$c")"; commit="$(label_commit "$l")"
  SOAK_SERVER_BINARY="$(builds_field "$l" path)"; SPEC377_SERVER_COMMIT="$commit"
  export SOAK_SERVER_BINARY SPEC377_SERVER_COMMIT
  ma="$(mem_available)"
  {
    echo "--- cell ${c} at $(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "loadavg: $(cat "$PROC/loadavg" 2>&1)"
    ps -eo pcpu,comm --sort=-pcpu 2>&1 | head -6
    head -8 "$PROC/meminfo" 2>&1
    echo "MEM_AVAILABLE_START_${c}=${ma:-n/a}"
  } >> "$HOSTLOG"
  r="$(load1)"
  say "LOAD_AT_START_${c}=${r:-n/a}"
  # The launched binaries must be the built ones, or nothing launches.
  if ! r="$(label_sha_reason "$l")" || ! r="$(label_sha_reason H)"; then
    echo "FATAL: pre-launch sha256 assertion failed for cell ${c}: ${r}; not launched" > "$OUT/spec377-${c}.runner-console.log"
    echo "RUNNER_EXIT=98" >> "$OUT/spec377-${c}.runner-console.log"
    say "cell ${c}: RUNNER_EXIT=98 (not launched: ${r})"
    echo "MEM_AVAILABLE_END_${c}=$(mem_available || true)" >> "$HOSTLOG"
    return 0
  fi
  say "cell ${c}: label=${l} server=${SOAK_SERVER_BINARY} commit=${commit}"
  bash "$SCRIPT_DIR/spec377-cells.sh" "$c" > "$OUT/spec377-${c}.runner-console.log" 2>&1 &
  rpid=$!
  wait "$rpid"; rc=$?
  echo "RUNNER_EXIT=${rc}" >> "$OUT/spec377-${c}.runner-console.log"
  say "cell ${c}: RUNNER_EXIT=${rc}"
  ma="$(mem_available)"
  echo "MEM_AVAILABLE_END_${c}=${ma:-n/a}" >> "$HOSTLOG"
}
# The Linux predicates per cell, so the decision reading finds one
# predicates file and one exit status per cell.
run_predicates() {   # $1 = cell
  local c="$1" rc
  bash "$SCRIPT_DIR/spec377-predicates.sh" "$OUT" "spec377-${c}" "$BUILDS" > "$OUT/spec377-${c}.predicates.txt" 2>&1
  rc=$?
  say "PREDICATES_EXIT_${c}=${rc}"
}
if [ "$PHASE" = "series" ]; then
  # A failed cell never stops the chain: the remaining cells still run and the
  # decision names the STOP. The predicates run after the last cell, so no
  # reading shares the host with a measured cell.
  FIRST=1
  for c in $CELLS; do
    [ "$FIRST" -eq 1 ] || settle "$c"
    FIRST=0
    run_cell "$c"
  done
  for c in $CELLS; do run_predicates "$c"; done
else
  for c in $CELLS; do
    run_cell "$c"
    run_predicates "$c"
  done
fi

# ----------------------------------------------------------------- 5. readings
if [ "$PHASE" = "series" ]; then
  SPEC377_MANIFEST_COMMIT="$MC" bash "$SCRIPT_DIR/spec377-decide.sh" "$OUT" "$MANIFEST" "$SCRIPT_DIR/spec377-smoke" > "$OUT/spec377.decision.txt" 2>&1
  DRC=$?
  say "decide rc=${DRC}$( [ "$DRC" -ne 0 ] && echo ' (NO flags: see spec377.decision.txt)')"
  # Indented: this log is itself an input of the reading's re-run.
  sed -n '/^== flags ==/,$p' "$OUT/spec377.decision.txt" | sed 's/^/  | /' | tee -a "$LOG"
  say "chain end: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  exit 0
fi

# smoke: the admission programs, then the literal capture, then the one
# admission line.
if [ -f "$SCRIPT_DIR/spec376-parity.sh" ]; then
  bash "$SCRIPT_DIR/spec376-parity.sh" "$OUT/parity" 2>&1 | tee -a "$LOG"
  say "parity rc=${PIPESTATUS[0]}"
else
  say "parity: spec376-parity.sh missing"
fi
if [ -f "$SCRIPT_DIR/spec377-synth.sh" ]; then
  bash "$SCRIPT_DIR/spec377-synth.sh" "$OUT/synthetic" 2>&1 | tee -a "$LOG"
  say "synth rc=${PIPESTATUS[0]}"
else
  say "synth: spec377-synth.sh missing"
fi

# Item 6: the allocator's own startup print is the treatment's proof. Every
# line of either mimalloc form (v3 unprefixed, v2 "mimalloc: ") and every
# jemalloc message is captured byte-for-byte, tagged with its cell, for M to
# freeze; the checks below read the captured files, not the consoles.
MI_CAPTURE_RE="^\\[server\\] (mimalloc: )?(thread 0x[0-9a-f]+: )?(v[0-9]+\\.[0-9]+\\.[0-9]+|option ')"
MI_LIT="$OUT/spec377-mi-literals.txt"
JE_CONF="$OUT/spec377-je-conf.txt"
: > "$MI_LIT"; : > "$JE_CONF"
for c in smi3 smi2; do
  grep -E "$MI_CAPTURE_RE" "$OUT/spec377-${c}.harness-console.log" 2>/dev/null | sed "s/^/${c} /" >> "$MI_LIT"
done
grep -E '^\[server\] <jemalloc>: ' "$OUT/spec377-sje.harness-console.log" 2>/dev/null | sed 's/^/sje /' >> "$JE_CONF"
say "literal capture: $(grep -c . "$MI_LIT") mimalloc line(s) -> $(basename "$MI_LIT"), $(grep -c . "$JE_CONF") jemalloc line(s) -> $(basename "$JE_CONF")"

# ----------------------------------------------------------------- 6. admission
# Every item under the absence rule: a missing line, file or column fails the
# item and is named in failed=.
FAILED=""
fail_item() { FAILED="${FAILED:+${FAILED},}$1"; }
pred_true() {   # $1 = cell, $2 = predicate; exactly one ^P=TRUE( |$) line
  local f="$OUT/spec377-$1.predicates.txt" n v
  n="$(grep -c "^$2=" "$f" 2>/dev/null)"
  case "$n" in
    0) fail_item "$1:$2=absent" ;;
    # The text after = up to the first space: only ^P=TRUE( |$) reads TRUE.
    1) v="$(sed -n "s/^$2=//p" "$f")"; v="${v%% *}"; [ -n "$v" ] || v=empty
       [ "$v" = "TRUE" ] || fail_item "$1:$2=$v" ;;
    *) fail_item "$1:$2=dup" ;;
  esac
}
# The captured lines of one cell, tag removed.
cell_lines() { sed -n "s/^$2 //p" "$1" 2>/dev/null; }
count_re() { printf '%s\n' "$1" | grep -Ec "$2" || true; }
# One mimalloc cell's startup block against the sources: the version line and
# three options exactly once each at their default values; any other value of
# those options, the other major's version, or a per-thread prefix fails.
mi_check() {   # $1 = cell, $2 = prefix ("" or "mimalloc: "), $3 = version, $4 = other major, $5..$7 = purge_delay arena_purge_mult purge_decommits
  local c="$1" P="$2" L n o v opt want
  L="$(cell_lines "$MI_LIT" "$c")"
  [ -n "$L" ] || { fail_item "${c}:mi_literals=absent"; return; }
  n="$(count_re "$L" "^\\[server\\] ${P}v${3//./\\.}[ ,(]")"
  [ "$n" = "1" ] || fail_item "${c}:mi_version_line=${n}"
  o="$(count_re "$L" "^\\[server\\] (mimalloc: )?v${4}\\.")"
  [ "$o" = "0" ] || fail_item "${c}:mi_version=other_major(${o})"
  n="$(count_re "$L" 'thread 0x')"
  [ "$n" = "0" ] || fail_item "${c}:mi_thread_prefix=${n}"
  for opt in "purge_delay:$5" "arena_purge_mult:$6" "purge_decommits:$7"; do
    v="${opt%%:*}"; want="${opt#*:}"
    n="$(count_re "$L" "^\\[server\\] ${P}option '${v}': ${want} *\$")"
    o="$(count_re "$L" "^\\[server\\] (mimalloc: )?option '${v}': ")"
    { [ "$n" = "1" ] && [ "$o" = "1" ]; } || fail_item "${c}:mi_option_${v}=exact:${n}/any:${o}"
  done
}
# sje's confirm_conf printout: sources #1..#5 in order, only #4 (the env
# source) non-empty and equal to the cell's env, and exactly the two
# "Set conf value" lines that env implies.
je_conf_check() {
  local L seq n
  L="$(cell_lines "$JE_CONF" sje)"
  [ -n "$L" ] || { fail_item "sje:je_conf=absent"; return; }
  n="$(count_re "$L" '^\[server\] <jemalloc>: malloc_conf #[1-5] \(')"
  seq="$(printf '%s\n' "$L" | sed -nE 's/^\[server\] <jemalloc>: malloc_conf #([1-5]) \(.*/\1/p' | tr -d '\n')"
  { [ "$n" = "5" ] && [ "$seq" = "12345" ]; } || fail_item "sje:je_conf_sources=n${n}:seq${seq:-none}"
  for k in 1 2 3 5; do
    n="$(count_re "$L" "^\\[server\\] <jemalloc>: malloc_conf #${k} \\(.*: \"\" *\$")"
    [ "$n" = "1" ] || fail_item "sje:je_conf_${k}=not_empty_or_absent"
  done
  n="$(count_re "$L" '^\[server\] <jemalloc>: malloc_conf #4 \(.*: "background_thread:true,confirm_conf:true" *$')"
  [ "$n" = "1" ] || fail_item "sje:je_conf_4=not_the_cell_env"
  n="$(count_re "$L" '^\[server\] <jemalloc>: -- Set conf value: background_thread:true *$')"
  [ "$n" = "1" ] || fail_item "sje:je_set_background_thread=${n}"
  n="$(count_re "$L" '^\[server\] <jemalloc>: -- Set conf value: confirm_conf:true *$')"
  [ "$n" = "1" ] || fail_item "sje:je_set_confirm_conf=${n}"
  n="$(count_re "$L" 'Set conf value')"
  [ "$n" = "2" ] || fail_item "sje:je_set_lines=${n}"
}
for c in $CELLS; do
  v="$(key_once "$OUT/spec377-${c}.runner-console.log" RUNNER_EXIT)"
  [ "$v" = "0" ] || fail_item "${c}:runner_exit=${v}"
  # A predicates run that crashed after printing its TRUE lines left a
  # partial file; its exit status is what names that.
  v="$(key_once "$LOG" "PREDICATES_EXIT_${c}")"
  [ "$v" = "0" ] || fail_item "${c}:predicates_rc=${v}"
  if [ -f "$OUT/spec377-${c}.predicates.txt" ]; then
    pred_true "$c" PEL; pred_true "$c" PMEM; pred_true "$c" PV; pred_true "$c" PALLOC
  else
    fail_item "${c}:predicates_missing"
  fi
  # Every row before the end of the run carries all eleven memory columns,
  # located by header name, each a number (fp_equiv_mb may be negative on a
  # row that breaks the invariants), and there is at least one such row; a
  # row whose elapsed_secs is not a number cannot be placed and fails too.
  dur="$( [ -f "$OUT/spec377-${c}.matrix.txt" ] && awk '/^  duration: / { n++; v = $2 } END { if (n == 1) { sub(/s$/, "", v); print v } }' "$OUT/spec377-${c}.matrix.txt")"
  case "$dur" in ''|*[!0-9]*) fail_item "${c}:duration=absent" ;; *)
    r="$(awk -F, -v D="$dur" '
      NR == 1 {
        split("elapsed_secs rss_mb fp_equiv_mb hwm_rss_mb lazyfree_mb swap_mb file_mb anon_mb private_dirty_mb pss_mb anon_huge_mb smaps_rss_mb", need, " ")
        for (i = 1; i <= NF; i++) h[$i] = i
        for (k in need) if (!(need[k] in h)) { print "column_missing(" need[k] ")"; bad = 1; exit }
        next
      }
      $h["elapsed_secs"] !~ /^[0-9]+(\.[0-9]+)?$/ { badel++; next }
      $h["elapsed_secs"] + 0 < D {
        rows++
        for (k in need) { x = $h[need[k]]; if (x == "") { empty++; break } if (x !~ /^-?[0-9]+(\.[0-9]+)?$/) { nonnum++; break } }
      }
      END { if (bad) exit; if (badel > 0) print "non_numeric_elapsed=" badel; else if (rows == 0) print "no_rows"
            else if (empty > 0) print "empty_rows=" empty; else if (nonnum > 0) print "non_numeric_rows=" nonnum; else print "ok" }' "$OUT/spec377-${c}.csv" 2>/dev/null)"
    [ "$r" = "ok" ] || fail_item "${c}:mem_rows=${r:-csv_missing}" ;;
  esac
  case "$c" in
    sje)
      r="$(awk -F, '
        NR == 1 {
          split("je_allocated je_active je_resident je_retained je_mapped je_metadata je_probe_elapsed_s je_probe_seq", need, " ")
          for (i = 1; i <= NF; i++) h[$i] = i
          for (k in need) if (!(need[k] in h)) { print "column_missing(" need[k] ")"; bad = 1; exit }
          next
        }
        $h["je_probe_seq"] != "" { rows++; for (k in need) { x = $h[need[k]]; if (x == "") { empty++; break } if (x !~ /^[0-9]+(\.[0-9]+)?$/) { nonnum++; break } } }
        END { if (bad) exit; if (rows == 0) print "no_probe_rows"; else if (empty > 0) print "empty_rows=" empty
              else if (nonnum > 0) print "non_numeric_rows=" nonnum; else print "ok" }' "$OUT/spec377-sje.csv" 2>/dev/null)"
      [ "$r" = "ok" ] || fail_item "sje:je_rows=${r:-csv_missing}" ;;
  esac
done
mi_check smi3 "" 3.3.2 2 1000 1 1
mi_check smi2 "mimalloc: " 2.3.2 3 10 10 1
je_conf_check
for k in SYNTH_PARITY SYNTH377; do
  v="$(key_once "$LOG" "$k")"
  printf '%s\n' "$v" | grep -Eq '^PASS( |$)' || fail_item "${k}=${v%% *}"
done
# One SMOKE_PROG_SHA= line per program, each with a 64-hex sha.
for f in $(program_names); do
  # Literal prefix match: a file name's dots must not act as regex wildcards.
  n="$(awk -v p="SMOKE_PROG_SHA=${f} " 'index($0, p) == 1 { n++ } END { print n + 0 }' "$LOG")"
  if [ "$n" -ne 1 ]; then fail_item "SMOKE_PROG_SHA_${f}=$( [ "$n" -eq 0 ] && echo absent || echo dup)"
  elif ! awk -v p="SMOKE_PROG_SHA=${f} sha256=" 'index($0, p) == 1 && substr($0, length(p) + 1) ~ /^[0-9a-f]+$/ && length($0) == length(p) + 64 { ok = 1 } END { exit !ok }' "$LOG"; then
    fail_item "SMOKE_PROG_SHA_${f}=empty"
  fi
done
if [ -z "$FAILED" ]; then say "SMOKE_ADMISSION=PASS failed=none"; else say "SMOKE_ADMISSION=FAIL failed=${FAILED}"; fi
say "### SMOKE COMPLETE $(date -u +%Y-%m-%dT%H:%M:%SZ)"
say "chain end: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
