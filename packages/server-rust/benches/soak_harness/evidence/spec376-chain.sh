#!/usr/bin/env bash
#
# Linux cell chain (topgun-bench) -- a copy of spec373b-chain.sh,
# which is NOT edited. One detached launch per phase.
#
# A COPY EXISTS BECAUSE THE PARENT CHAIN CANNOT RUN THIS SERIES. It builds two
# flavours at two commits into one evidence dir, reads host state with
# vm_stat/uptime, resolves `awk` to whatever the host ships, and knows none of
# the cells, programs or gates this series pre-registers. The difference list
# against spec373b-chain.sh is CLOSED at exactly nine items:
#
#   1. LINUX ONLY. `uname -s` must print Linux, checked before any file is
#      created; anything else is FATAL, exit 2. No macOS code path remains
#      (no xcrun/SDKROOT, no vm_stat, no uptime parsing).
#   2. THE AWK SHIM. Every frozen program invokes `awk` by name and was
#      validated under the BWK line; Debian's `awk` is mawk. The chain links
#      target/spec376-awkbin/awk to `original-awk`, prepends it to PATH,
#      asserts that `awk` resolves to it and that `awk -version` matches
#      ^awk version [0-9]{8}, and logs the banner. Absent or mismatched is
#      FATAL.
#   3. PHASES build | smoke | cal (SPEC376_PHASE, required, no default)
#      replace the parent's ca | je phases and SPEC373B_SMOKE switch. Each
#      phase is one detached launch with its own log, and the inherited
#      per-run knobs (SPEC365_*, the smoke duration/cadence/census overrides,
#      base-name suffixes, SOAK_SERVER_BINARY) are unset first, so nothing
#      from the caller's shell changes what a cell measures.
#   4. BUILDS (phase build): clean detached checkouts at CAL_PIN (read from
#      the one ^CAL_PIN= line of spec376-cells.sh), the 373b pin b166719d and
#      the 373b freeze e85adb1f; eight builds, each into its own chain-owned
#      target dir keyed by BUILD LABEL (target/spec376-<label>), so no build
#      overwrites another label's binary; recompiled=yes, mtime >= build
#      start and the flavour marker asserted per label. The builds file is
#      written OUTSIDE the evidence dir, to target/spec376-run/, and carries
#      the build start epoch (the freshness bound the cells compare binary
#      mtimes against, since smoke and cal are later launches), the eight
#      flavour= lines, the rustc -vV and glibc lines and the awk banner.
#   5. PRE-LAUNCH SHA ASSERTION BY BUILD LABEL. Before each cell the server
#      binary of the cell's mapped label and the harness (label H) must hash
#      to the sha256= of exactly one builds-file line; a missing line, a
#      duplicate or an empty sha256= is a mismatch and the cell is NOT
#      launched.
#   6. SMOKE (phase smoke, SPEC376_OUT_DIR pinned to target/spec376-run/smoke,
#      handed to the cells as SPEC365_OUT_DIR; the evidence dir and anything
#      under it are refused): one SMOKE_PROG_SHA= line per spec376 program,
#      PROC_ROOT=/proc, the six admission cells, the Linux predicates after
#      each cell (PREDICATES_EXIT_<cell>=), a smaps_rollup sample from sc's
#      server at ~60 s, a calibration-reading self-run (rc logged; no M
#      exists, so it exercises the refusal path), the dhat frame check, the
#      shares self-check, the awk parity run, the synthetic cases, and one
#      SMOKE_ADMISSION= line.
#   7. CAL (phase cal, evidence dir): ORDER=OK (spec376-order.sh, its sha
#      checked against M first), the smoke->M program binding, the preflight
#      gate (PREFLIGHT_LOG=), PROC_ROOT=/proc, c1 -> pb -> c2 -> pa with the
#      Linux predicates after each cell (PREDICATES_EXIT_<cell>=), then the
#      calibration reading -> spec376-calib.txt, its exit status logged.
#   8. HOST SIDECAR FROM /proc: per cell /proc/loadavg, top-5 by CPU and the
#      first 8 lines of /proc/meminfo into spec376-host.log, and
#      LOAD_AT_START_<cell>= (the 1-min field of /proc/loadavg) in the chain
#      log immediately before the launch.
#   9. This header, the messages and the log names.
#
# A DIFF HUNK THAT MAPS TO NONE OF THE NINE ITEMS IS A DEFECT; the manifest
# carries the hunk-to-item map (diff spec373b-chain.sh spec376-chain.sh).
#
# Every line a later reading keys on is printed at column 0, exactly once per
# log: PROC_ROOT=, PREFLIGHT_LOG=, LOAD_AT_START_<cell>=,
# PREDICATES_EXIT_<cell>=, SMOKE_PROG_SHA=<file> (once per program),
# SMOKE_ADMISSION=. Anything echoed from another program's output that is not
# such a key is indented, so no reader can count it twice.
set -uo pipefail
export LC_ALL=C

# Item 1: /proc, ss, GNU tools and the Linux cells behind this chain; refuse
# before anything is created.
if [ "$(uname -s 2>/dev/null || true)" != "Linux" ]; then
  echo "FATAL: spec376-chain.sh runs on Linux only (uname -s = '$(uname -s 2>/dev/null || true)')" >&2
  exit 2
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
SERVER_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd -P)"          # packages/server-rust
REPO_ROOT="$(cd "$SERVER_ROOT/../.." && pwd -P)"
PIN=b166719d
FRZ=e85adb1f
PORT=47376
T_ROOT="${REPO_ROOT}/target"
RUN_DIR="${T_ROOT}/spec376-run"
BUILDS="${RUN_DIR}/spec376-builds.txt"
SMOKE_OUT_PIN="${RUN_DIR}/smoke"
MANIFEST="$SCRIPT_DIR/spec376-manifest.md"
EVREL="packages/server-rust/benches/soak_harness/evidence"
LABELS="CA-cal CA-pin CA-frz DH-pin JE-cal SYS-cal MI-cal H"

# The freeze commit lives in one place, the runner; a second definition would
# let the chain and the runner build and check different commits.
[ "$(grep -c '^CAL_PIN=' "$SCRIPT_DIR/spec376-cells.sh" 2>/dev/null)" = "1" ] \
  || { echo "FATAL: spec376-cells.sh must define CAL_PIN= exactly once" >&2; exit 1; }
CAL_PIN="$(sed -n 's/^CAL_PIN=//p' "$SCRIPT_DIR/spec376-cells.sh")"
printf '%s' "$CAL_PIN" | grep -Eq '^[0-9a-f]{8,40}$' \
  || { echo "FATAL: CAL_PIN literal '${CAL_PIN}' is not a hex commit id" >&2; exit 1; }

PHASE="${SPEC376_PHASE:-}"
case "$PHASE" in build|smoke|cal) ;; *) echo "FATAL: SPEC376_PHASE must be build, smoke or cal (got '${PHASE}')" >&2; exit 2 ;; esac

# The inherited per-run knobs change what a cell measures; this chain sets the
# ones it needs and nothing else may leak in from the caller's environment.
unset SPEC365_OUT_DIR SPEC365_DATA_DIR SPEC365_FORCE SPEC362B_SMOKE_DURATION \
      SPEC365_SMOKE_SAMPLE_INTERVAL SPEC371_SMOKE_LIVE_CENSUS SPEC370_BASE_SUFFIX \
      SPEC371_BASE_SUFFIX SOAK_SERVER_BINARY 2>/dev/null || true

if [ "$PHASE" = "smoke" ]; then
  # The smoke OUT is pinned (gitignored target/, outside every path M commits);
  # the textual check runs before any directory is created.
  OUT="${SPEC376_OUT_DIR:-}"
  OUT="${OUT%/}"
  [ -n "$OUT" ] || { echo "FATAL: the smoke needs SPEC376_OUT_DIR=${SMOKE_OUT_PIN}" >&2; exit 2; }
  case "$OUT/" in "$SCRIPT_DIR"/*) echo "FATAL: the smoke must not write into or under the evidence dir (${OUT})" >&2; exit 2 ;; esac
  [ "$OUT" = "$SMOKE_OUT_PIN" ] || { echo "FATAL: SPEC376_OUT_DIR must be ${SMOKE_OUT_PIN} (got '${OUT}')" >&2; exit 2; }
  mkdir -p "$OUT"; OUT="$(cd "$OUT" && pwd -P)"
  case "$OUT/" in "$SCRIPT_DIR"/*) echo "FATAL: the smoke OUT resolves into the evidence dir (${OUT})" >&2; exit 2 ;; esac
  [ "$OUT" = "$SMOKE_OUT_PIN" ] || { echo "FATAL: the smoke OUT resolves to ${OUT}, not ${SMOKE_OUT_PIN}" >&2; exit 2; }
  export SPEC365_OUT_DIR="$OUT"
  CELLS="sc spb sdh sje ssy smi"
  LOG="$OUT/spec376-chain.log"
elif [ "$PHASE" = "cal" ]; then
  OUT="$SCRIPT_DIR"; CELLS="c1 pb c2 pa"
  LOG="$OUT/spec376-chain.log"
else
  mkdir -p "$RUN_DIR"
  OUT="$RUN_DIR"; CELLS=""
  LOG="$OUT/spec376-chain-build.log"
fi
: > "$LOG"
say() { echo "$*" | tee -a "$LOG"; }

# ----------------------------------------------------------------- 1. start
CHAIN_START_EPOCH="$(date +%s)"
say "chain start: $(date -u +%Y-%m-%dT%H:%M:%SZ) epoch=${CHAIN_START_EPOCH} phase=${PHASE} HEAD=$(git -C "$REPO_ROOT" rev-parse HEAD) cal_pin=${CAL_PIN}"

# Item 2: every frozen program was validated under BWK awk; if `awk` resolved
# to mawk here, the parity those programs rely on would be meaningless.
OA="$(command -v original-awk 2>/dev/null || true)"
[ -n "$OA" ] || { say "FATAL: original-awk is not installed; refusing to run any awk program under another interpreter"; exit 1; }
AWKBIN="${T_ROOT}/spec376-awkbin"
mkdir -p "$AWKBIN" && ln -sfn "$OA" "$AWKBIN/awk" || { say "FATAL: cannot create the awk shim in ${AWKBIN}"; exit 1; }
export PATH="${AWKBIN}:${PATH}"
[ "$(command -v awk)" = "$AWKBIN/awk" ] || { say "FATAL: awk resolves to '$(command -v awk)', not the shim ${AWKBIN}/awk"; exit 1; }
AWK_BANNER="$(awk -version 2>&1 | head -1)"
printf '%s\n' "$AWK_BANNER" | grep -Eq '^awk version [0-9]{8}' \
  || { say "FATAL: awk banner '${AWK_BANNER}' does not match ^awk version [0-9]{8}"; exit 1; }
say "awk shim: ${AWKBIN}/awk -> ${OA} banner='${AWK_BANNER}'"

# ----------------------------------------------------------------- helpers
# The build label a cell's server must come from (the PV map of manifest R0.3).
cell_label() {
  case "$1" in
    sc|c1|c2) echo CA-cal ;;
    spb|pb)   echo CA-pin ;;
    pa)       echo CA-frz ;;
    sdh)      echo DH-pin ;;
    sje)      echo JE-cal ;;
    ssy)      echo SYS-cal ;;
    smi)      echo MI-cal ;;
    *)        echo "" ;;
  esac
}
label_commit() {
  case "$1" in
    CA-cal|JE-cal|SYS-cal|MI-cal|H) echo "$CAL_PIN" ;;
    CA-pin|DH-pin)                  echo "$PIN" ;;
    CA-frz)                         echo "$FRZ" ;;
  esac
}
# One field of the builds-file line of a label; empty unless EXACTLY one line
# carries that label, so a duplicate can never pick "the first one".
builds_field() {   # $1 = label, $2 = field
  [ -f "$BUILDS" ] || return 0
  awk -v l="flavour=$1" -v f="$2" '
    $1 == l { n++; for (i = 2; i <= NF; i++) if (index($i, f "=") == 1) v = substr($i, length(f) + 2) }
    END { if (n == 1) print v }' "$BUILDS"
}
builds_count() { [ -f "$BUILDS" ] && awk -v l="flavour=$1" '$1 == l { n++ } END { print n + 0 }' "$BUILDS" || echo 0; }
# Item 5: the launched binary must be the built one. Prints nothing on success;
# on failure prints the named reason and returns 1.
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
# The flavour markers, the same rule the runner asserts before its clock.
marker_ok() {   # $1 = label, $2 = binary
  local p d j m
  p="$(hits "$2" 'alloc_probe elapsed_s=')"; d="$(hits "$2" 'DHAT_OUT')"
  j="$(hits "$2" 'je_probe elapsed_s=')";    m="$(hits "$2" 'mimalloc: warning: ')"
  case "$1" in
    CA-*)  [ "$p" -gt 0 ] && [ "$d" -eq 0 ] && [ "$j" -eq 0 ] && [ "$m" -eq 0 ] ;;
    DH-*)  [ "$d" -gt 0 ] && [ "$p" -eq 0 ] && [ "$j" -eq 0 ] && [ "$m" -eq 0 ] ;;
    JE-*)  [ "$j" -gt 0 ] && [ "$p" -eq 0 ] && [ "$d" -eq 0 ] && [ "$m" -eq 0 ] ;;
    MI-*)  [ "$m" -gt 0 ] && [ "$p" -eq 0 ] && [ "$d" -eq 0 ] && [ "$j" -eq 0 ] ;;
    SYS-*) [ "$p" -eq 0 ] && [ "$d" -eq 0 ] && [ "$j" -eq 0 ] && [ "$m" -eq 0 ] ;;
    H)     [ "$(hits "$2" 'tombstone-byte level ceiling breached')" -gt 0 ] && [ "$(hits "$2" 'soak: child TOPGUN_JOURNAL_ENABLED=')" -gt 0 ] ;;
    *)     false ;;
  esac
}
# The spec376 programs whose bytes a smoke admits and M freezes; the smoke's
# SMOKE_PROG_SHA= lines and M's section-1 list are both drawn by this rule.
is_program() { case "$1" in spec376-*.sh|spec376-*.awk|spec376-*.py) return 0 ;; *) return 1 ;; esac; }
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
  SRC_CAL="${T_ROOT}/spec376-src-${CAL_PIN}"
  SRC_PIN="${T_ROOT}/spec376-src-${PIN}"
  SRC_FRZ="${T_ROOT}/spec376-src-${FRZ}"
  checkout "$SRC_CAL" "$CAL_PIN"
  checkout "$SRC_PIN" "$PIN"
  checkout "$SRC_FRZ" "$FRZ"

  guarded_rm() {
    local cand="$1" parent resolved l
    parent="$(dirname "$cand")"; mkdir -p "$parent"
    resolved="$(cd "$parent" && pwd -P)/$(basename "$cand")"
    for l in $LABELS; do
      [ "$resolved" = "${T_ROOT}/spec376-${l}" ] && { rm -rf "$resolved"; return 0; }
    done
    say "FATAL: refusing to remove '$resolved': not a chain-owned target dir"; exit 1
  }
  build() {   # $1 = label, $2 = source crate dir, $3 = target dir, $4.. = cargo args
    local fl="$1" src="$2" td="$3"; shift 3
    guarded_rm "$td"
    say "build ${fl}: (cd ${src} && CARGO_TARGET_DIR=${td} cargo build $*)"
    ( cd "$src" && CARGO_TARGET_DIR="$td" cargo build "$@" ) > "$OUT/spec376-build-${fl}.log" 2>&1
    local rc=$?
    tail -3 "$OUT/spec376-build-${fl}.log" | sed 's/^/  | /' >> "$LOG"
    [ "$rc" -eq 0 ] || { say "FATAL: build ${fl} failed rc=${rc}"; exit 1; }
    if grep -q 'Compiling topgun-server v' "$OUT/spec376-build-${fl}.log"; then echo yes; else echo no; fi > "$OUT/.recompiled-${fl}"
  }
  for fl in $LABELS; do
    td="${T_ROOT}/spec376-${fl}"
    case "$fl" in
      CA-cal)  build "$fl" "${SRC_CAL}/packages/server-rust" "$td" --release --features count-alloc --bin topgun-server ;;
      CA-pin)  build "$fl" "${SRC_PIN}/packages/server-rust" "$td" --release --features count-alloc --bin topgun-server ;;
      CA-frz)  build "$fl" "${SRC_FRZ}/packages/server-rust" "$td" --release --features count-alloc --bin topgun-server ;;
      DH-pin)  build "$fl" "${SRC_PIN}/packages/server-rust" "$td" --profile release-with-debug --features dhat-heap --bin topgun-server ;;
      JE-cal)  build "$fl" "${SRC_CAL}/packages/server-rust" "$td" --release --features alloc-jemalloc --bin topgun-server ;;
      SYS-cal) build "$fl" "${SRC_CAL}/packages/server-rust" "$td" --release --bin topgun-server ;;
      MI-cal)  build "$fl" "${SRC_CAL}/packages/server-rust" "$td" --release --features alloc-mimalloc --bin topgun-server ;;
      H)       build "$fl" "${SRC_CAL}/packages/server-rust" "$td" --release --bench soak_harness ;;
    esac
  done

  H_CANDS="$(ls "${T_ROOT}"/spec376-H/release/deps/soak_harness-* 2>/dev/null | grep -vE '\.(d|o|rcgu)' || true)"
  [ "$(printf '%s\n' "$H_CANDS" | grep -c .)" -eq 1 ] || { say "FATAL: expected exactly one soak_harness binary, got: ${H_CANDS}"; exit 1; }
  : > "$BUILDS"
  # The cells compare every binary's mtime with this epoch; smoke and cal are
  # later launches, so the bound they need is the build start, not their own.
  echo "build_start_epoch=${CHAIN_START_EPOCH}" >> "$BUILDS"
  for fl in $LABELS; do
    case "$fl" in
      DH-pin) b="${T_ROOT}/spec376-${fl}/release-with-debug/topgun-server" ;;
      H)      b="$H_CANDS" ;;
      *)      b="${T_ROOT}/spec376-${fl}/release/topgun-server" ;;
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
  ( cd "$SRC_CAL" && rustc -vV ) 2>&1 | sed 's/^/rustc: /' >> "$BUILDS"
  echo "glibc: $(ldd --version 2>&1 | head -1)" >> "$BUILDS"
  echo "awk: ${AWK_BANNER}" >> "$BUILDS"
  sed 's/^/  | /' "$BUILDS" >> "$LOG"
  say "builds file: ${BUILDS}"
  say "chain end: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  exit 0
fi

# ----------------------------------------------------------------- 3. cal gates
# Only the cal phase is pre-registered: ORDER, the smoke->M binding and the
# preflight gate run before any cell, and any of them refuses the phase.
if [ "$PHASE" = "cal" ]; then
  # spec376-order.sh cannot vouch for itself: check its bytes against M's
  # section-1 listing before it is trusted.
  order_sha_ok() {   # $1 = M; prints nothing, returns 0 iff order.sh hashes as M lists it
    local want got
    want="$(git -C "$REPO_ROOT" show "${1}:${EVREL}/spec376-manifest.md" 2>/dev/null \
      | sed '/^## APPEND-ONLY BELOW/q' | sed -nE 's/^- `([0-9a-f]{64})` `packages\/server-rust\/benches\/soak_harness\/evidence\/spec376-order\.sh`.*/\1/p')"
    got="$(shasum -a 256 "$SCRIPT_DIR/spec376-order.sh" 2>/dev/null | awk '{print $1}')"
    [ "$(printf '%s\n' "$want" | grep -c .)" -eq 1 ] && [ "$want" = "$got" ]
  }
  MC="${SPEC376_MANIFEST_COMMIT:-}"
  [ -n "$MC" ] || { say "FATAL: SPEC376_MANIFEST_COMMIT is required for the cal phase"; exit 1; }
  if order_sha_ok "$MC"; then
    ORDER_LINE="$(bash "$SCRIPT_DIR/spec376-order.sh" "$MC")"
    orc=$?
  else
    ORDER_LINE="ORDER=FAIL spec376-order.sh does not hash as M '${MC}' lists it"; orc=3
  fi
  say "$ORDER_LINE"
  { [ "$orc" -eq 0 ] && printf '%s\n' "$ORDER_LINE" | grep -Eq '^ORDER=OK( |$)'; } \
    || { say "FATAL: ORDER is not OK; refusing to run"; exit 1; }

  # Smoke -> M binding: the admitting smoke must have run exactly the bytes M
  # freezes. Both sides are drawn by is_program(); any difference is named.
  SMOKE_LOG="$SCRIPT_DIR/spec376-smoke/spec376-chain.log"
  M_PROGS="$(git -C "$REPO_ROOT" show "${MC}:${EVREL}/spec376-manifest.md" 2>/dev/null | sed '/^## APPEND-ONLY BELOW/q' \
    | sed -nE 's/^- `([0-9a-f]{64})` `packages\/server-rust\/benches\/soak_harness\/evidence\/(spec376-[^`\/]+)`.*/\2 \1/p' \
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
  PF="$(ls "$SCRIPT_DIR"/spec376-preflight-*.log 2>/dev/null | sort | tail -1)"
  [ -n "$PF" ] || { say "FATAL: no spec376-preflight-*.log in the evidence dir; run spec376-preflight.sh first"; exit 1; }
  [ -s "$PF" ] || { say "FATAL: preflight log $(basename "$PF") is empty"; exit 1; }
  PF_LAST="$(tail -n 1 "$PF")"
  printf '%s\n' "$PF_LAST" | grep -Eq '^PREFLIGHT=PASS( |$)' \
    || { say "FATAL: preflight log $(basename "$PF") does not end with PREFLIGHT=PASS (last line: '${PF_LAST}')"; exit 1; }
  PF_AT_N="$(printf '%s\n' "$PF_LAST" | tr ' ' '\n' | grep -c '^PREFLIGHT_AT=')"
  [ "$PF_AT_N" -eq 1 ] || { say "FATAL: preflight last line carries ${PF_AT_N} PREFLIGHT_AT= fields, need exactly 1"; exit 1; }
  PF_AT="$(printf '%s\n' "$PF_LAST" | tr ' ' '\n' | sed -n 's/^PREFLIGHT_AT=//p')"
  PF_EPOCH="$(iso_epoch "$PF_AT")"
  [ -n "$PF_EPOCH" ] || { say "FATAL: PREFLIGHT_AT='${PF_AT}' is not YYYY-MM-DDTHH:MM:SSZ"; exit 1; }
  PF_AGE=$((CHAIN_START_EPOCH - PF_EPOCH))
  { [ "$PF_AGE" -ge 0 ] && [ "$PF_AGE" -le 3600 ]; } \
    || { say "FATAL: preflight PASS is ${PF_AGE}s before the chain start; it must be 0..3600s"; exit 1; }
  say "PREFLIGHT_LOG=$(basename "$PF")"
  say "preflight: PREFLIGHT_AT=${PF_AT} age=${PF_AGE}s"
fi

# ----------------------------------------------------------------- 4. cells
# smoke and cal: the binaries are the build phase's, asserted by label.
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
  # The bytes this smoke admits, one line per program (item 6).
  for f in $(cd "$SCRIPT_DIR" && ls 2>/dev/null | sort); do
    is_program "$f" && [ -f "$SCRIPT_DIR/$f" ] || continue
    say "SMOKE_PROG_SHA=${f} sha256=$(shasum -a 256 "$SCRIPT_DIR/$f" | awk '{print $1}')"
  done
fi
# The runner reads memory from /proc and has no knob that changes it; this
# line is what the calibration reading checks to know no fixture root ran.
say "PROC_ROOT=/proc"

HOSTLOG="$OUT/spec376-host.log"
: > "$HOSTLOG"
export SPEC376_CHAIN_START_EPOCH="$BUILD_START_EPOCH"
export SPEC376_HARNESS_BIN="$BIN_H"

# The smaps_rollup sample (smoke admission item 2): the live file from sc's
# server ~60 s after its listener appears, plus status and `ps -o rss=`, so
# the sampler can be replayed over it in fixture mode. Written only whole.
capture_smaps() {   # $1 = runner pid, $2 = sample file
  local rpid="$1" dst="$2" pid="" t=0 tmp
  while kill -0 "$rpid" 2>/dev/null; do
    pid="$(ss -Hltnp "sport = :${PORT}" 2>/dev/null | sed -nE 's/.*pid=([0-9]+).*/\1/p' | head -1)"
    [ -n "$pid" ] && break
    sleep 1
  done
  [ -n "$pid" ] || return 0
  while [ "$t" -lt 60 ] && kill -0 "$rpid" 2>/dev/null; do sleep 1; t=$((t + 1)); done
  [ "$t" -ge 60 ] || return 0
  tmp="${dst}.part"
  {
    echo "== pid=${pid} captured_at=$(date -u +%Y-%m-%dT%H:%M:%SZ) after_listener_s=${t}"
    echo "== smaps_rollup" && cat "/proc/${pid}/smaps_rollup" &&
    echo "== status" && cat "/proc/${pid}/status" &&
    echo "== ps_rss" && ps -o rss= -p "$pid"
  } > "$tmp" 2>/dev/null && mv "$tmp" "$dst" || rm -f "$tmp"
}

run_cell() {   # $1 = cell
  local c="$1" rc l commit r rpid cap=""
  l="$(cell_label "$c")"; commit="$(label_commit "$l")"
  SOAK_SERVER_BINARY="$(builds_field "$l" path)"; SPEC376_SERVER_COMMIT="$commit"
  export SOAK_SERVER_BINARY SPEC376_SERVER_COMMIT
  {
    echo "--- cell ${c} at $(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "loadavg: $(cat /proc/loadavg 2>&1)"
    ps -eo pcpu,comm --sort=-pcpu 2>&1 | head -6
    head -8 /proc/meminfo 2>&1
  } >> "$HOSTLOG"
  r="$(awk '{ print $1 }' /proc/loadavg 2>/dev/null)"
  say "LOAD_AT_START_${c}=${r:-n/a}"
  # Item 5: the launched binaries must be the built ones, or nothing launches.
  if ! r="$(label_sha_reason "$l")" || ! r="$(label_sha_reason H)"; then
    echo "FATAL: pre-launch sha256 assertion failed for cell ${c}: ${r}; not launched" > "$OUT/spec376-${c}.runner-console.log"
    echo "RUNNER_EXIT=98" >> "$OUT/spec376-${c}.runner-console.log"
    say "cell ${c}: RUNNER_EXIT=98 (not launched: ${r})"
    return 0
  fi
  say "cell ${c}: label=${l} server=${SOAK_SERVER_BINARY} commit=${commit}"
  bash "$SCRIPT_DIR/spec376-cells.sh" "$c" > "$OUT/spec376-${c}.runner-console.log" 2>&1 &
  rpid=$!
  if [ "$c" = "sc" ]; then
    capture_smaps "$rpid" "$OUT/spec376-smaps-sample.txt" &
    cap=$!
  fi
  wait "$rpid"; rc=$?
  [ -z "$cap" ] || wait "$cap"
  echo "RUNNER_EXIT=${rc}" >> "$OUT/spec376-${c}.runner-console.log"
  say "cell ${c}: RUNNER_EXIT=${rc}"
}
# The Linux predicates right after each cell, so the calibration reading
# finds one predicates file and one exit status per cell (manifest R3.4).
run_predicates() {   # $1 = cell
  local c="$1" rc
  bash "$SCRIPT_DIR/spec376-predicates.sh" "$OUT" "spec376-${c}" "$BUILDS" > "$OUT/spec376-${c}.predicates.txt" 2>&1
  rc=$?
  say "PREDICATES_EXIT_${c}=${rc}"
}
for c in $CELLS; do
  run_cell "$c"
  run_predicates "$c"
done

# ----------------------------------------------------------------- 5. readings
if [ "$PHASE" = "cal" ]; then
  SPEC376_MANIFEST_COMMIT="$MC" bash "$SCRIPT_DIR/spec376-calib.sh" "$OUT" "$MANIFEST" "$SCRIPT_DIR/spec376-smoke" > "$OUT/spec376-calib.txt" 2>&1
  CRC=$?
  say "calib rc=${CRC}$( [ "$CRC" -ne 0 ] && echo ' (NO flags: see spec376-calib.txt)')"
  # Indented: this log is itself an input of the reading's re-run.
  sed -n '/^== flags ==/,$p' "$OUT/spec376-calib.txt" | sed 's/^/  | /' | tee -a "$LOG"
  say "chain end: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  exit 0
fi

# smoke: the admission programs, then the one admission line.
# No M exists at smoke time: the reading must refuse (exit 3, no flags). Its
# status is recorded, never an admission input.
env -u SPEC376_MANIFEST_COMMIT bash "$SCRIPT_DIR/spec376-calib.sh" "$OUT" "$MANIFEST" "$OUT" > "$OUT/spec376-calib-selfrun.txt" 2>&1
say "calib self-run rc=$? (no M: the refusal path; not an admission input)"

# Replays procmem_row over the captured sample in fixture mode (item 2 of the
# admission). Prints PASS or a named reason.
smaps_sample_check() {   # $1 = sample file
  local s="$1" root pid row prc
  [ -s "$s" ] || { echo "absent"; return; }
  pid="$(sed -nE '1s/^== pid=([0-9]+) .*/\1/p' "$s")"
  [ -n "$pid" ] || { echo "no_pid_header"; return; }
  root="$(mktemp -d "${TMPDIR:-/tmp}/spec376-smaps.XXXXXX")"
  mkdir -p "$root/$pid"
  awk -v d="$root/$pid" -v r="$root/$pid.ps_rss" '
    /^== smaps_rollup$/ { f = d "/smaps_rollup"; next }
    /^== status$/       { f = d "/status"; next }
    /^== ps_rss$/       { f = r; next }
    /^== /              { f = ""; next }
    f != ""             { print > f }' "$s"
  row="$( PROCMEM_INV_FILE="$root/inv" PROCMEM_PM_FILE="$root/pm" bash -c '. "$1" && procmem_row "$2" "$3"' _ "$SCRIPT_DIR/spec376-procmem.sh" "$pid" "$root")"
  prc=$?
  rm -rf "$root"
  [ "$prc" -eq 0 ] || { echo "procmem_rc=${prc}(${row})"; return; }
  printf '%s\n' "$row" | awk -F, 'NF == 12 { for (i = 1; i <= NF; i++) if ($i !~ /^-?[0-9]+(\.[0-9]+)?$/) exit 1; exit 0 } { exit 1 }' \
    || { echo "empty_or_non_numeric_field(${row})"; return; }
  echo "PASS"
}
SMAPS_RESULT="$(smaps_sample_check "$OUT/spec376-smaps-sample.txt")"
say "smaps sample: ${SMAPS_RESULT}"

# The dhat frame check: each b166719d site must name at least one frame of
# sdh's profile, by the shares program's own anchoring rule.
DHAT_GZ="$OUT/spec376-sdh.dhat.json.gz"
if [ -s "$DHAT_GZ" ]; then
  python3 - "$DHAT_GZ" <<'PY' 2>&1 | tee -a "$LOG"
import gzip, json, re, sys
SITES = ("storage/engines/hashmap.rs:136",
         "storage/datastores/write_behind.rs:2463",
         "storage/datastores/write_behind.rs:2518")
try:
    ftbl = json.load(gzip.open(sys.argv[1]))["ftbl"]
except Exception as e:
    print("DH_FRAMES=FAIL reason=profile_unreadable(%s)" % type(e).__name__)
    sys.exit(0)
ok = True
for s in SITES:
    rx = re.compile(r"\(" + re.escape(s) + r":\d+\)$")
    n = sum(1 for f in ftbl if rx.search(f))
    print("DH_FRAME_%s=%d" % (s, n))
    ok = ok and n >= 1
print("DH_FRAMES=%s" % ("PASS" if ok else "FAIL"))
PY
else
  say "DH_FRAMES=FAIL reason=profile_missing"
fi

( cd "$SCRIPT_DIR" && python3 spec373b-shares.py --pin 46dcc12a spec371-c3e.dhat.json.gz spec371-c3l.dhat.json.gz ) > "$OUT/spec376-selfcheck.txt" 2>&1
SRC_RC=$?
say "shares self-check rc=${SRC_RC}"
grep '^SELF_CHECK=' "$OUT/spec376-selfcheck.txt" | tee -a "$LOG"

if [ -f "$SCRIPT_DIR/spec376-parity.sh" ]; then
  bash "$SCRIPT_DIR/spec376-parity.sh" "$OUT/parity" 2>&1 | tee -a "$LOG"
  say "parity rc=${PIPESTATUS[0]}"
else
  say "parity: spec376-parity.sh missing"
fi
if [ -f "$SCRIPT_DIR/spec376-synth.sh" ]; then
  bash "$SCRIPT_DIR/spec376-synth.sh" "$OUT/synthetic" 2>&1 | tee -a "$LOG"
  say "synth rc=${PIPESTATUS[0]}"
else
  say "synth: spec376-synth.sh missing"
fi

# ----------------------------------------------------------------- 6. admission
# Every item under the absence rule: a missing line, file or column fails the
# item and is named in failed=.
FAILED=""
fail_item() { FAILED="${FAILED:+${FAILED},}$1"; }
pred_true() {   # $1 = cell, $2 = predicate; exactly one ^P=TRUE( |$) line
  local f="$OUT/spec376-$1.predicates.txt" n v
  n="$(grep -c "^$2=" "$f" 2>/dev/null)"
  case "$n" in
    0) fail_item "$1:$2=absent" ;;
    # The text after = up to the first space: only ^P=TRUE( |$) reads TRUE.
    1) v="$(sed -n "s/^$2=//p" "$f")"; v="${v%% *}"; [ -n "$v" ] || v=empty
       [ "$v" = "TRUE" ] || fail_item "$1:$2=$v" ;;
    *) fail_item "$1:$2=dup" ;;
  esac
}
for c in $CELLS; do
  v="$(key_once "$OUT/spec376-${c}.runner-console.log" RUNNER_EXIT)"
  [ "$v" = "0" ] || fail_item "${c}:runner_exit=${v}"
  # A predicates run that crashed after printing its TRUE lines left a
  # partial file; its exit status is what names that, as in the cal reading.
  v="$(key_once "$LOG" "PREDICATES_EXIT_${c}")"
  [ "$v" = "0" ] || fail_item "${c}:predicates_rc=${v}"
  if [ -f "$OUT/spec376-${c}.predicates.txt" ]; then
    pred_true "$c" PEL; pred_true "$c" PMEM
    case "$c" in sc|spb) pred_true "$c" PA ;; esac
  else
    fail_item "${c}:predicates_missing"
  fi
  # Every row before the end of the run carries all eleven memory columns,
  # located by header name, each a number (fp_equiv_mb may be negative on a
  # row that breaks the invariants), and there is at least one such row; a
  # row whose elapsed_secs is not a number cannot be placed and fails too.
  dur="$( [ -f "$OUT/spec376-${c}.matrix.txt" ] && awk '/^  duration: / { n++; v = $2 } END { if (n == 1) { sub(/s$/, "", v); print v } }' "$OUT/spec376-${c}.matrix.txt")"
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
            else if (empty > 0) print "empty_rows=" empty; else if (nonnum > 0) print "non_numeric_rows=" nonnum; else print "ok" }' "$OUT/spec376-${c}.csv" 2>/dev/null)"
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
              else if (nonnum > 0) print "non_numeric_rows=" nonnum; else print "ok" }' "$OUT/spec376-sje.csv" 2>/dev/null)"
      [ "$r" = "ok" ] || fail_item "sje:je_rows=${r:-csv_missing}"
      n="$(grep -c '^\[server\] je_config ' "$OUT/spec376-sje.harness-console.log" 2>/dev/null)"
      [ "$n" = "1" ] || fail_item "sje:je_config=$( [ "${n:-0}" = "0" ] && echo absent || echo dup)" ;;
    ssy|smi)
      # The runner writes console line 1 only after its flavour-marker
      # assertion passed; the builds line carries the chain's own marker=ok.
      fl="$( [ "$c" = "ssy" ] && echo SYS || echo MI)"
      head -1 "$OUT/spec376-${c}.harness-console.log" 2>/dev/null | grep -Eq "^provenance: server sha256=[0-9a-f]{64} flavour=${fl}( |$)" \
        && [ "$(builds_field "$(cell_label "$c")" marker)" = "ok" ] || fail_item "${c}:marker=absent" ;;
  esac
done
[ "$SMAPS_RESULT" = "PASS" ] || fail_item "smaps_sample=${SMAPS_RESULT}"
v="$(key_once "$LOG" DH_FRAMES)"
[ "$v" = "PASS" ] || fail_item "DH_FRAMES=${v}"
v="$(key_once "$LOG" SELF_CHECK)"
{ [ "$v" = "PASS" ] && [ "$SRC_RC" -eq 0 ]; } || fail_item "SELF_CHECK=${v}(rc=${SRC_RC})"
for k in SYNTH_PARITY SYNTH376; do
  v="$(key_once "$LOG" "$k")"
  printf '%s\n' "$v" | grep -Eq '^PASS( |$)' || fail_item "${k}=${v%% *}"
done
# One SMOKE_PROG_SHA= line per program, each with a 64-hex sha.
for f in $(cd "$SCRIPT_DIR" && ls 2>/dev/null | sort); do
  is_program "$f" && [ -f "$SCRIPT_DIR/$f" ] || continue
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
