#!/usr/bin/env bash
#
# awk parity of the frozen verdict program on Linux: runs the Linux copy of the
# 19 synthetic verdict cases (spec376-synth373b-linux.sh, whose only hunk is
# the GNU `sed -i` form) and diffs its transcript against the transcript the
# frozen spec373b-synth.sh printed on macOS under BWK awk
# (spec376-synth-ref-darwin.txt). Empty diff = the Linux interpreter reads
# every frozen awk program exactly as the macOS one did.
#
# usage: spec376-parity.sh <SCRATCH_DIR>
#        spec376-parity.sh --darwin-ref <SCRATCH_DIR>
#
#   default mode (the bench host; the first program run there, before any
#   build or cell): prints exactly one SYNTH_PARITY=PASS|FAIL line, then the
#   recorded-only SYNTH_PARITY_MAWK= / SYNTH_PARITY_GAWK= lines. Exit 0 on
#   PASS, 1 on FAIL, 2 on usage.
#   --darwin-ref (macOS only): runs the FROZEN spec373b-synth.sh and prints its
#   @OUT@-substituted transcript on stdout, validated as a reference (exit 1 if
#   it is not one). Both sides of the comparison therefore go through the
#   same substitution code.
#
# WHY ITS OWN SHIM: this runs before the cell chain has created its awk shim,
# and Debian's `awk` is mawk. The frozen programs call `awk` by name, so the
# copy runs with a private directory holding awk -> original-awk first on
# PATH, and the BWK version banner is asserted before anything is compared.
# The executor never switches interpreter silently: no original-awk, or a
# banner that is not BWK's, is a FAIL, not a fallback.
#
# WHY THE REFERENCE IS VALIDATED FIRST: a reference and a transcript that both
# stop at the same early FATAL would diff empty. The reference must carry
# exactly the 19 case sections, "--- synthetic S1" .. "--- synthetic S19" in
# that order, one "rc=<n>" line per section, and no column-0 "FATAL:" line
# (the synth's own setup failures print there; the verdict program's FATALs
# for cases S11/S12/S18/S19 are expected output and are indented by the
# synth). Otherwise SYNTH_PARITY=FAIL reason=ref_invalid.
set -uo pipefail
export LC_ALL=C

MODE=parity
if [ "${1:-}" = "--darwin-ref" ]; then MODE=darwin_ref; shift; fi
SCR="${1:-}"
if [ -z "$SCR" ] || [ "$#" -ne 1 ]; then
  echo "usage: spec376-parity.sh [--darwin-ref] <SCRATCH_DIR>" >&2
  [ "$MODE" = parity ] && echo "SYNTH_PARITY=FAIL reason=usage"
  exit 2
fi
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
REF="$SCRIPT_DIR/spec376-synth-ref-darwin.txt"
N_CASES=19

fail() {   # $1 = reason; the one SYNTH_PARITY= line of a failed run
  echo "SYNTH_PARITY=FAIL reason=$1"
  exit 1
}
refuse() {   # $1 = message, $2 = reason
  echo "FATAL: $1" >&2
  if [ "$MODE" = parity ]; then fail "$2"; fi
  exit 1
}

mkdir -p "$SCR" 2>/dev/null || refuse "cannot create scratch dir ${SCR}" scratch
SCR="$(cd "$SCR" && pwd -P)"
# The synth refuses the evidence dir itself; refusing here too keeps parity's
# own files (shims, transcripts, diffs) out of every path M commits.
case "$SCR/" in "$SCRIPT_DIR"/*) refuse "the scratch dir must not be the evidence dir or under it" scratch ;; esac

# Replace every spelling of a run's scratch path by @OUT@, as a literal, so a
# path in a message can never make two checkouts or two hosts differ.
subst_out() {   # $1 = scratch path as passed, $2 = physical path; stdin -> stdout
  local a b
  a="$(printf '%s' "$1" | sed 's/[][\/.*^$&|]/\\&/g')"
  b="$(printf '%s' "$2" | sed 's/[][\/.*^$&|]/\\&/g')"
  sed -e "s|${b}|@OUT@|g" -e "s|${a}|@OUT@|g"
}
# Prints nothing if $1 is a valid reference transcript, else the reason.
ref_problem() {
  local f="$1" want got
  want="$(i=1; while [ "$i" -le "$N_CASES" ]; do echo "--- synthetic S$i"; i=$((i + 1)); done)"
  got="$(grep '^--- synthetic ' "$f")"
  [ "$got" = "$want" ] || { echo "sections($(printf '%s\n' "$got" | grep -c '^--- synthetic '))"; return; }
  [ "$(grep -cE '^rc=[0-9]+$' "$f")" -eq "$N_CASES" ] || { echo "rc_lines($(grep -cE '^rc=[0-9]+$' "$f"))"; return; }
  grep -q '^FATAL:' "$f" && { echo "fatal_line"; return; }
  return 0
}
# $1 = synth program, $2 = run name: the substituted transcript goes to
# $SCR/<name>.txt and the synth's exit status to $SYNTH_RC (no command
# substitution, so the status is not lost in a subshell).
run_synth() {
  local out="$SCR/$2" raw="$SCR/$2.raw"
  rm -rf "$out"
  bash "$1" "$out" > "$raw" 2>&1
  SYNTH_RC=$?
  subst_out "$out" "$(mkdir -p "$out" && cd "$out" && pwd -P)" < "$raw" > "$SCR/$2.txt"
}
# A private awk shim dir: $1 = dir, $2 = interpreter path or wrapper body.
make_shim() {
  rm -rf "$1" && mkdir -p "$1" || return 1
  case "$2" in
    /*) ln -s "$2" "$1/awk" ;;
    *)  printf '%s\n' '#!/bin/sh' "$2" > "$1/awk" && chmod +x "$1/awk" ;;
  esac
}

if [ "$MODE" = darwin_ref ]; then
  [ "$(uname -s 2>/dev/null || true)" = "Darwin" ] || refuse "--darwin-ref captures the macOS reference and runs on macOS only" os
  BANNER="$(awk -version 2>&1 | head -1)"
  printf '%s\n' "$BANNER" | grep -Eq '^awk version [0-9]{8}' || refuse "awk banner '${BANNER}' is not BWK's" awk_banner
  run_synth "$SCRIPT_DIR/spec373b-synth.sh" darwin-ref
  [ "$SYNTH_RC" -eq 0 ] || refuse "the frozen synth exited ${SYNTH_RC}" synth_rc
  [ -s "$SCR/darwin-ref.txt" ] || refuse "the frozen synth printed nothing" transcript_empty
  P="$(ref_problem "$SCR/darwin-ref.txt")"
  [ -z "$P" ] || refuse "the transcript is not a valid reference: ${P}" ref_invalid
  cat "$SCR/darwin-ref.txt"
  echo "darwin-ref: awk='${BANNER}' sections=${N_CASES} OK" >&2
  exit 0
fi

# ------------------------------------------------------------------ parity
OA="$(command -v original-awk 2>/dev/null || true)"
[ -n "$OA" ] || refuse "original-awk is not installed; no other interpreter is substituted" no_original_awk
SHIM="$SCR/awkbin-original"
make_shim "$SHIM" "$OA" || refuse "cannot create the awk shim in ${SHIM}" awk_shim
BASE_PATH="$PATH"
PATH="${SHIM}:${BASE_PATH}"; export PATH
[ "$(command -v awk)" = "$SHIM/awk" ] || refuse "awk resolves to '$(command -v awk)', not the shim ${SHIM}/awk" awk_shim
BANNER="$(awk -version 2>&1 | head -1)"
printf '%s\n' "$BANNER" | grep -Eq '^awk version [0-9]{8}' || refuse "awk banner '${BANNER}' does not match ^awk version [0-9]{8}" awk_banner
echo "parity awk: ${SHIM}/awk -> ${OA} banner='${BANNER}'"

[ -e "$REF" ] || fail ref_missing
[ -s "$REF" ] || fail ref_empty
P="$(ref_problem "$REF")"
[ -z "$P" ] || { echo "  | reference problem: ${P}"; fail ref_invalid; }

run_synth "$SCRIPT_DIR/spec376-synth373b-linux.sh" linux
[ -s "$SCR/linux.txt" ] || fail transcript_empty
diff "$REF" "$SCR/linux.txt" > "$SCR/linux.diff" 2>&1
DRC=$?
if [ "$DRC" -ne 0 ]; then
  sed 's/^/  | /' "$SCR/linux.diff"
  fail "diff lines=$(wc -l < "$SCR/linux.diff" | tr -d ' ')"
fi
# The reference was captured from a run that exited 0; an identical transcript
# from a run that did not is still not the same run.
[ "$SYNTH_RC" -eq 0 ] || fail "synth_rc=${SYNTH_RC}"
echo "SYNTH_PARITY=PASS cases=${N_CASES} awk='${BANNER}'"

# ------------------------------------------------- recorded only, not gates
other() {   # $1 = KEY, $2 = shim dir, $3 = interpreter path or wrapper body
  local name d
  make_shim "$2" "$3" || { echo "$1=n/a reason=shim"; return; }
  name="$(basename "$2").run"
  PATH="$2:${BASE_PATH}"
  run_synth "$SCRIPT_DIR/spec376-synth373b-linux.sh" "$name"
  PATH="${SHIM}:${BASE_PATH}"
  d="$(diff "$REF" "$SCR/$name.txt" | wc -l | tr -d ' ')"
  if [ -s "$SCR/$name.txt" ] && [ "$d" -eq 0 ]; then echo "$1=PASS rc=${SYNTH_RC}"; else echo "$1=FAIL diff_lines=${d} rc=${SYNTH_RC}"; fi
}
if command -v mawk >/dev/null 2>&1; then
  other SYNTH_PARITY_MAWK "$SCR/awkbin-mawk" "$(command -v mawk)"
else
  echo "SYNTH_PARITY_MAWK=n/a reason=absent"
fi
if command -v gawk >/dev/null 2>&1; then
  other SYNTH_PARITY_GAWK "$SCR/awkbin-gawk" "exec $(command -v gawk) --posix \"\$@\""
else
  echo "SYNTH_PARITY_GAWK=n/a reason=absent"
fi
exit 0
