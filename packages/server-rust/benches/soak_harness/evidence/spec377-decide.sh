#!/usr/bin/env bash
#
# The allocator-series decision reading (topgun-bench): reads the five series
# cells s1 je mi3 mi2 s2 and prints the pre-registered flags block, ending in
# DEFAULT_CANDIDATE=, NEXT= and CONDUCTOR_RULE=. It decides nothing about
# shipping a default: every allocator flip it can name routes through the
# out-of-process performance follow-up first.
#
# usage: spec377-decide.sh <EV_DIR> <MANIFEST> <SMOKE_DIR>
#   EV_DIR     the series artifacts: spec377-<cell>.{predicates.txt,matrix.txt,
#              runner-console.log,soak.json} for s1 je mi3 mi2 s2, and the series
#              chain log spec377-chain.log (PREFLIGHT_LOG=, DISK_FREE_AT_START=,
#              PREDICATES_EXIT_<cell>=) plus the preflight log that line names
#   MANIFEST   spec377-manifest.md; section 1 (above the APPEND-ONLY marker)
#              must carry each pre-registered literal exactly once at column 0
#   SMOKE_DIR  the admitting smoke's outputs; its SMOKE_ADMISSION= line is
#              recorded
#   SPEC377_MANIFEST_COMMIT (= M) is required, except under the synthetic rule.
#
# Exit status: 0 = the flags block was printed; 3 = ORDER is not OK or a
# section-1 literal is absent, duplicated or malformed (NO flags); 4 = a
# program step failed (NO flags; the intermediate file is kept).
#
# Order (flags LAST, after every STOP predicate):
#   0. ORDER=OK re-checked here (spec377-order.sh <M> <MANIFEST>, whose own
#      bytes are first checked against M's listing). SPEC377_SYNTHETIC=1 skips
#      it ONLY when none of EV_DIR, MANIFEST and SMOKE_DIR lies in the evidence
#      dir, and prints ORDER=SKIPPED (synthetic). Then every pre-registered
#      literal, validated, never defaulted.
#   1. The inputs are gathered into one intermediate KEY=VALUE file: presence
#      is resolved HERE (absent / dup / the value), so the awk program never
#      mistakes a missing line for a value.
#   2. spec377-decide.awk evaluates STOP-H, STOP-V, the readings and the
#      flags; every threshold it compares against is passed in from section 1.
# The disk floor of STOP-H (40 GiB, the series chain's own refusal bound) is
# evaluated here, so the awk program carries no threshold section 1 does not
# list.
set -uo pipefail
export LC_ALL=C
EV="${1:-}"; MANIFEST="${2:-}"; SMOKE="${3:-}"
if [ "$#" -ne 3 ] || [ ! -d "$EV" ] || [ ! -f "$MANIFEST" ] || [ -z "$SMOKE" ]; then
  echo "usage: spec377-decide.sh <EV_DIR> <MANIFEST> <SMOKE_DIR>" >&2; exit 2
fi
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
SERVER_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd -P)"
REPO_ROOT="$(cd "$SERVER_ROOT/../.." && pwd -P)"
EVREL="packages/server-rust/benches/soak_harness/evidence"
EV_ABS="$(cd "$EV" && pwd -P)"
MAN_ABS="$(cd "$(dirname "$MANIFEST")" && pwd -P)/$(basename "$MANIFEST")"
if [ -d "$SMOKE" ]; then SMOKE_ABS="$(cd "$SMOKE" && pwd -P)"; else SMOKE_ABS="$SMOKE"; fi
CELLS="s1 je mi3 mi2 s2"
CHAIN_LOG="$EV/spec377-chain.log"
DECIDE_AWK="$SCRIPT_DIR/spec377-decide.awk"
MIN_DISK_KB=41943040

under() {   # $1 path, $2 dir: 0 iff $1 is $2 or lies below it
  case "$1/" in "$2"/*) return 0 ;; esac; return 1
}
# One value for a column-0 KEY= line of a file, or the absence named.
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
# spec377-order.sh cannot vouch for itself: its bytes are checked against M's
# section-1 listing before it is trusted.
order_sha_ok() {   # $1 = M
  local want got
  [ -n "$1" ] || return 1
  want="$(git -C "$REPO_ROOT" show "${1}:${EVREL}/spec377-manifest.md" 2>/dev/null \
    | sed '/^## APPEND-ONLY BELOW/q' | sed -nE 's/^- `([0-9a-f]{64})` `packages\/server-rust\/benches\/soak_harness\/evidence\/spec377-order\.sh`.*/\1/p')"
  got="$(shasum -a 256 "$SCRIPT_DIR/spec377-order.sh" 2>/dev/null | awk '{print $1}')"
  [ "$(printf '%s\n' "$want" | grep -c .)" -eq 1 ] && [ "$want" = "$got" ]
}

# ---- 0. ORDER=OK at the reading's own start
if [ "${SPEC377_SYNTHETIC:-0}" = "1" ] && ! under "$EV_ABS" "$SCRIPT_DIR" \
   && ! under "$(dirname "$MAN_ABS")" "$SCRIPT_DIR" && ! under "$SMOKE_ABS" "$SCRIPT_DIR"; then
  ORDER_LINE="ORDER=SKIPPED (synthetic)"
else
  MC="${SPEC377_MANIFEST_COMMIT:-}"
  if [ -z "$MC" ]; then
    ORDER_LINE="ORDER=FAIL no manifest commit (SPEC377_MANIFEST_COMMIT unset)"; orc=3
  elif order_sha_ok "$MC"; then
    ORDER_LINE="$(bash "$SCRIPT_DIR/spec377-order.sh" "$MC" "$MAN_ABS")"
    orc=$?
  else
    ORDER_LINE="ORDER=FAIL spec377-order.sh does not hash as M '${MC}' lists it"; orc=3
  fi
  if [ "$orc" -ne 0 ] || ! printf '%s\n' "$ORDER_LINE" | grep -Eq '^ORDER=OK( |$)'; then
    echo "$ORDER_LINE"; echo "FATAL: ORDER is not OK (rc=${orc}); no flags printed" >&2; exit 3
  fi
fi
echo "$ORDER_LINE"

# ---- 0b. section-1 literals: every one exactly once at column 0, validated
if ! grep -q '^## APPEND-ONLY BELOW' "$MANIFEST"; then
  echo "FATAL: the manifest has no '## APPEND-ONLY BELOW' marker, so its section 1 is undefined" >&2; exit 3
fi
SEC1="$(sed '/^## APPEND-ONLY BELOW/q' "$MANIFEST")"
LITERALS="SERIES_PIN A0_L_MIB FLAT_BAR_PER_H BETTER_BAR SYS_AGREE_BAR OPS_PARITY_MIN TIE_BAND STAGE2_MAX_H LEVEL_WINDOW_S MI_POSTINIT_BOUND LAZY_DRIFT_BAR PRICE_EUR_PER_H"
BAD=""
echo "== section 1 (read from the manifest, not recomputed) =="
for k in $LITERALS; do
  n="$(printf '%s\n' "$SEC1" | grep -c "^${k}=")"
  if [ "$n" != "1" ]; then BAD="${BAD} ${k}=$( [ "$n" = "0" ] && echo absent || echo dup)"; continue; fi
  v="$(printf '%s\n' "$SEC1" | sed -n "s/^${k}=//p")"; v="${v%% *}"
  case "$k" in
    SERIES_PIN) printf '%s' "$v" | grep -Eq '^[0-9a-f]{7,40}$' || { BAD="${BAD} ${k}=${v:-empty}"; continue; } ;;
    *) printf '%s' "$v" | grep -Eq '^[0-9]+(\.[0-9]+)?$' || { BAD="${BAD} ${k}=${v:-empty}"; continue; } ;;
  esac
  eval "L_${k}=\$v"
  echo "MANIFEST_${k}=${v}"
done
if [ -n "$BAD" ]; then
  echo "FATAL: section-1 literal(s) absent, duplicated or malformed:${BAD}; no flags printed" >&2; exit 3
fi

# The intermediate lives outside the evidence dir, in a fresh file this run
# created, so a stale file can never be read as this run's readings.
READ="$(mktemp "${TMPDIR:-/tmp}/spec377-decide.XXXXXX")" || { echo "FATAL: cannot create the intermediate file" >&2; exit 4; }
READ_DIR="$(cd "$(dirname "$READ")" && pwd -P)"
if under "$READ_DIR" "$SCRIPT_DIR" || under "$READ_DIR" "$EV_ABS"; then
  echo "FATAL: the intermediate ${READ} lies in the evidence dir; set TMPDIR elsewhere" >&2; rm -f "$READ"; exit 4
fi
FAILED=""

# ---- 1. host inputs
H_PREFLIGHT=""
n_pl="$( [ -f "$CHAIN_LOG" ] && grep -c '^PREFLIGHT_LOG=' "$CHAIN_LOG")"
case "${n_pl:-0}" in
  0) H_PREFLIGHT="preflight_log=absent" ;;
  1)
    PL="$(sed -n 's/^PREFLIGHT_LOG=//p' "$CHAIN_LOG")"
    if ! printf '%s' "$PL" | grep -Eq '^spec377-preflight-[A-Za-z0-9._:-]+\.log$'; then
      H_PREFLIGHT="preflight_log=${PL:-empty}"
    elif [ ! -f "$EV/$PL" ]; then
      H_PREFLIGHT="preflight=absent"
    elif [ ! -s "$EV/$PL" ]; then
      H_PREFLIGHT="preflight=empty"
    else
      LAST="$(tail -n 1 "$EV/$PL")"
      if ! printf '%s\n' "$LAST" | grep -Eq '^PREFLIGHT=PASS( |$)'; then
        case "$LAST" in
          PREFLIGHT=*) v="${LAST#PREFLIGHT=}"; H_PREFLIGHT="preflight=${v%% *}" ;;
          '') H_PREFLIGHT="preflight=last_line_empty" ;;
          *) H_PREFLIGHT="preflight=last_line_not_verdict" ;;
        esac
      fi
    fi ;;
  *) H_PREFLIGHT="preflight_log=dup" ;;
esac
DF="$(key_once "$CHAIN_LOG" DISK_FREE_AT_START)"; DF="${DF%% *}"
case "$DF" in
  absent|dup) H_DISK="disk_free=${DF}" ;;
  ''|*[!0-9]*) H_DISK="disk_free=${DF:-empty}" ;;
  *) if [ "$DF" -lt "$MIN_DISK_KB" ]; then H_DISK="disk_free=${DF}KiB<40GiB"; else H_DISK=none; fi ;;
esac

# The readings every cell carries, suffixed with the cell in its predicates
# file; copied verbatim (or named absent / dup), never recomputed.
READINGS="LIVE_END PE_END PE_LEVEL TREND TREND3 TREND_CORR STAGE2_T FIXED_EST FIXED_NOTE RSS_PE_LEVEL LAZY_MAX HWM_END ANON_HUGE_END DISK_SLOPE"
{
  echo "PREFLIGHT_CLAUSE=${H_PREFLIGHT:-none}"
  echo "DISK_CLAUSE=${H_DISK}"
  echo "DISK_FREE_SEEN=${DF}"
  for c in $CELLS; do
    BASE="spec377-${c}"
    MATRIX="$EV/$BASE.matrix.txt"; RUNNER="$EV/$BASE.runner-console.log"
    SOAK="$EV/$BASE.soak.json"; PRED="$EV/$BASE.predicates.txt"
    if [ -f "$PRED" ]; then
      echo "PRED_${c}=present"
      for p in PV PEL PM1 PMEM PALLOC PA PR-crashes WRITE_ERRORS; do
        n="$(grep -c "^${p}=" "$PRED")"
        case "$n" in
          0) v=absent ;;
          # The value is the text after = up to the first space, so only a
          # line matching ^<P>=TRUE( |$) reads TRUE.
          1) v="$(sed -n "s/^${p}=//p" "$PRED")"; v="${v%% *}"; [ -n "$v" ] || v=empty ;;
          *) v=dup ;;
        esac
        echo "PRED_${c}_${p}=${v}"
      done
      for k in $READINGS; do echo "${k}_${c}=$(key_once "$PRED" "${k}_${c}")"; done
    else
      echo "PRED_${c}=missing"
      for k in $READINGS; do echo "${k}_${c}=absent"; done
    fi
    echo "PREDRC_${c}=$(key_once "$CHAIN_LOG" "PREDICATES_EXIT_${c}")"
    echo "RUNNER_EXIT_${c}=$(key_once "$RUNNER" RUNNER_EXIT)"
    echo "STEAL_${c}=$(key_once "$RUNNER" steal_pct)"
    n_tw="$( [ -f "$SOAK" ] && grep -c '"totalWrites":' "$SOAK")"
    case "${n_tw:-0}" in
      0) echo "TOTAL_WRITES_${c}=absent" ;;
      1) TW="$(sed -nE 's/.*"totalWrites": *([0-9]+)([^0-9A-Za-z.]|$).*/\1/p' "$SOAK")"; echo "TOTAL_WRITES_${c}=${TW:-empty}" ;;
      *) echo "TOTAL_WRITES_${c}=dup" ;;
    esac
    n_du="$( [ -f "$MATRIX" ] && grep -c '^  duration: ' "$MATRIX")"
    case "${n_du:-0}" in
      0) echo "DURATION_${c}=absent" ;;
      1) DU="$(sed -nE 's/^  duration: +([0-9]+)s.*/\1/p' "$MATRIX")"; echo "DURATION_${c}=${DU:-absent}" ;;
      *) echo "DURATION_${c}=dup" ;;
    esac
  done
  PJE="$EV/spec377-je.predicates.txt"
  echo "JE_CONFIG=$(key_once "$PJE" JE_CONFIG)"
  echo "JE_CONFIRM_CONF=$(key_once "$PJE" JE_CONFIRM_CONF)"
  for k in AMP_JE FRAG_SHAREL DIRTY_SHAREL REACH_PE; do echo "${k}_je=$(key_once "$PJE" "${k}_je")"; done
  for c in mi3 mi2; do echo "MI_POSTINIT_LINES_${c}=$(key_once "$EV/spec377-${c}.predicates.txt" "MI_POSTINIT_LINES_${c}")"; done
} > "$READ" || FAILED="${FAILED} WRITE_INTERMEDIATE"
[ -s "$READ" ] || FAILED="${FAILED} EMPTY_INTERMEDIATE"
if [ -n "$FAILED" ]; then
  sed 's/^/  | /' "$READ"; echo "FATAL: a program step failed:${FAILED}; no flags printed (kept ${READ})" >&2; exit 4
fi
# Echoed indented: the decision file carries every key at column 0 exactly
# once, and these are inputs, not flags.
echo "== inputs (presence resolved) =="
sed 's/^/  | /' "$READ"

# ---- smoke admission, recorded exactly once by key
SMOKE_LOG="$SMOKE/spec377-chain.log"
if [ ! -f "$SMOKE_LOG" ]; then SMOKE_SEEN=log_absent
else
  n_sa="$(grep -c '^SMOKE_ADMISSION=' "$SMOKE_LOG")"
  case "$n_sa" in
    0) SMOKE_SEEN=absent ;;
    1) if grep -Eq '^SMOKE_ADMISSION=PASS( |$)' "$SMOKE_LOG"; then SMOKE_SEEN=PASS; else SMOKE_SEEN=FAIL; fi ;;
    *) SMOKE_SEEN=dup ;;
  esac
fi
echo "== smoke =="
echo "SMOKE_ADMISSION_SEEN=${SMOKE_SEEN}"

OUT_FLAGS="$(awk -f "$DECIDE_AWK" \
  -v better_bar="$L_BETTER_BAR" -v sys_agree_bar="$L_SYS_AGREE_BAR" -v ops_min="$L_OPS_PARITY_MIN" \
  -v tie_band="$L_TIE_BAND" -v stage2_max="$L_STAGE2_MAX_H" -v mi_bound="$L_MI_POSTINIT_BOUND" \
  -v lazy_bar="$L_LAZY_DRIFT_BAR" -v price="$L_PRICE_EUR_PER_H" "$READ")" \
  || { echo "FATAL: the reading step failed; no flags printed (kept ${READ})" >&2; exit 4; }
printf '%s\n' "$OUT_FLAGS"
rm -f "$READ"
