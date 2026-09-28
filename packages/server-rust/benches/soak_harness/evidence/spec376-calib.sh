#!/usr/bin/env bash
#
# Calibration reading of the Linux soak instrument (topgun-bench): the four
# cal cells c1 pb c2 pa (c1/c2 the same CAL_PIN binary, pb/pa the 373b pin and
# freeze as a portability control). It decides whether the instrument is sound
# and records the Linux reference rates; it decides nothing about the server.
#
# usage: spec376-calib.sh <EV_DIR> <MANIFEST> <SMOKE_DIR>
#   EV_DIR     the cal cells' artifacts: spec376-<cell>.{csv,matrix.txt,
#              runner-console.log,soak.json,predicates.txt} and the cal chain
#              log spec376-chain.log (PROC_ROOT=, PREFLIGHT_LOG=,
#              PREDICATES_EXIT_<cell>=, LOAD_AT_START_<cell>=) plus the
#              preflight log that line names
#   MANIFEST   spec376-manifest.md; section 1 (above the APPEND-ONLY marker)
#              must carry exactly one CAL_PIN= and one PORT_EXPECT= line at
#              column 0
#   SMOKE_DIR  the admitting smoke's outputs; its spec376-chain.log carries the
#              SMOKE_ADMISSION= line
#   SPEC376_MANIFEST_COMMIT (= M) is required, except under the synthetic rule.
#
# Exit status: 0 = the flags block was printed; 3 = ORDER is not OK, or the
# manifest's section-1 literals or a frozen source are invalid (NO flags);
# 4 = a program step failed (NO flags; the intermediate file is kept).
#
# Order (flags LAST, after every STOP predicate):
#   0. ORDER=OK re-checked here (spec376-order.sh <M> <MANIFEST>, whose own
#      bytes are first checked against M's listing), not only at the chain's
#      start -- else exit 3. SPEC376_SYNTHETIC=1 skips it ONLY when none of
#      EV_DIR, MANIFEST and SMOKE_DIR lies in the evidence dir, and prints
#      ORDER=SKIPPED (synthetic). Then the section-1 literals, then the rate
#      program: taken byte-for-byte from the frozen spec373b-verdict.sh
#      (lines 151-170, file sha256 and extracted-text sha256 asserted, must end
#      at its closing brace), so the Linux rate is the M1 rate's own definition.
#   1. STOP-H (host): the chain log's PREFLIGHT_LOG= line absent or duplicate;
#      the log it names absent, empty, or whose last line is not
#      ^PREFLIGHT=PASS( |$); a cal cell's steal_pct > 1.0; a steal_pct= line
#      absent, duplicate or non-numeric (n/a included) -> <cell>:steal=missing.
#   2. STOP-V: the chain log's PROC_ROOT= not exactly one line equal to /proc
#      (chain:proc_root=<value|absent|dup>); per cell, in cal order c1 pb c2 pa:
#      PV PEL PA PM1 PMEM each TRUE only if the cell's predicates file carries
#      EXACTLY ONE ^<P>=TRUE( |$) line (else <cell>:<P>=absent|dup|<value>; a
#      missing file is named once as <cell>:predicates_missing), then
#      PREDICATES_EXIT_<cell> absent or non-zero, RUNNER_EXIT absent or non-zero,
#      SKIPPED_<cell> absent, non-numeric or > 0, rate / live / totalWrites
#      absent, n/a or <= 0; last the pair clause c1c2:write_parity=<v> when
#      |w_c1 - w_c2| / mean > 0.05 or either totalWrites is missing (n/a).
#   3. Precedence H > V. INSTRUMENT=SOUND iff STOP=none and the smoke chain log
#      carries exactly one ^SMOKE_ADMISSION= line, matching
#      ^SMOKE_ADMISSION=PASS( |$); else NOT_SOUND reason=stop|smoke, stop
#      winning when both apply.
#   4. Recorded, never a STOP and never INSTRUMENT: the reference rates, the
#      c1/c2 spreads, bytes per write, the four-cell write parity, A0_L_MIB,
#      the portability ratios and the manifest's PORT_EXPECT band (not a
#      gate), steal and load at start per cell.
# Every read gate follows the absence rule: an absent, duplicate, empty or
# non-numeric input is never a pass and is named.
set -uo pipefail
export LC_ALL=C
EV="${1:-}"; MANIFEST="${2:-}"; SMOKE="${3:-}"
if [ "$#" -ne 3 ] || [ ! -d "$EV" ] || [ ! -f "$MANIFEST" ] || [ -z "$SMOKE" ]; then
  echo "usage: spec376-calib.sh <EV_DIR> <MANIFEST> <SMOKE_DIR>" >&2; exit 2
fi
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
SERVER_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd -P)"
REPO_ROOT="$(cd "$SERVER_ROOT/../.." && pwd -P)"
EVREL="packages/server-rust/benches/soak_harness/evidence"
EV_ABS="$(cd "$EV" && pwd -P)"
MAN_ABS="$(cd "$(dirname "$MANIFEST")" && pwd -P)/$(basename "$MANIFEST")"
if [ -d "$SMOKE" ]; then SMOKE_ABS="$(cd "$SMOKE" && pwd -P)"; else SMOKE_ABS="$SMOKE"; fi
CELLS="c1 pb c2 pa"
CHAIN_LOG="$EV/spec376-chain.log"

under() {   # $1 path, $2 dir: 0 iff $1 is $2 or lies below it
  case "$1/" in "$2"/*) return 0 ;; esac; return 1
}
# One value for a column-0 KEY= line of a file, or the absence named:
# absent (no file or no line), dup (two or more lines).
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
# spec376-order.sh cannot vouch for itself: check its bytes against M's
# section-1 listing before it is trusted.
order_sha_ok() {   # $1 = M; prints nothing, returns 0 iff order.sh hashes as M lists it
  local want got
  [ -n "$1" ] || return 1
  want="$(git -C "$REPO_ROOT" show "${1}:${EVREL}/spec376-manifest.md" 2>/dev/null \
    | sed '/^## APPEND-ONLY BELOW/q' | sed -nE 's/^- `([0-9a-f]{64})` `packages\/server-rust\/benches\/soak_harness\/evidence\/spec376-order\.sh`.*/\1/p')"
  got="$(shasum -a 256 "$SCRIPT_DIR/spec376-order.sh" 2>/dev/null | awk '{print $1}')"
  [ "$(printf '%s\n' "$want" | grep -c .)" -eq 1 ] && [ "$want" = "$got" ]
}

# ---- 0. ORDER=OK at the reading's own start
if [ "${SPEC376_SYNTHETIC:-0}" = "1" ] && ! under "$EV_ABS" "$SCRIPT_DIR" \
   && ! under "$(dirname "$MAN_ABS")" "$SCRIPT_DIR" && ! under "$SMOKE_ABS" "$SCRIPT_DIR"; then
  ORDER_LINE="ORDER=SKIPPED (synthetic)"
else
  MC="${SPEC376_MANIFEST_COMMIT:-}"
  if [ -z "$MC" ]; then
    ORDER_LINE="ORDER=FAIL no manifest commit (SPEC376_MANIFEST_COMMIT unset)"; orc=3
  elif order_sha_ok "$MC"; then
    ORDER_LINE="$(bash "$SCRIPT_DIR/spec376-order.sh" "$MC" "$MAN_ABS")"
    orc=$?
  else
    ORDER_LINE="ORDER=FAIL spec376-order.sh does not hash as M '${MC}' lists it"; orc=3
  fi
  if [ "$orc" -ne 0 ] || ! printf '%s\n' "$ORDER_LINE" | grep -Eq '^ORDER=OK( |$)'; then
    echo "$ORDER_LINE"; echo "FATAL: ORDER is not OK (rc=${orc}); no flags printed" >&2; exit 3
  fi
fi
echo "$ORDER_LINE"

# ---- 0b. section-1 literals: validated, never defaulted
if ! grep -q '^## APPEND-ONLY BELOW' "$MANIFEST"; then
  echo "FATAL: the manifest has no '## APPEND-ONLY BELOW' marker, so its section 1 is undefined" >&2; exit 3
fi
SEC1="$(sed '/^## APPEND-ONLY BELOW/q' "$MANIFEST")"
n_pin="$(printf '%s\n' "$SEC1" | grep -c '^CAL_PIN=')"
n_exp="$(printf '%s\n' "$SEC1" | grep -c '^PORT_EXPECT=')"
if [ "$n_pin" != "1" ] || [ "$n_exp" != "1" ]; then
  echo "FATAL: section 1 must carry exactly one CAL_PIN= and one PORT_EXPECT= line (found ${n_pin} / ${n_exp})" >&2; exit 3
fi
CAL_PIN="$(printf '%s\n' "$SEC1" | sed -n 's/^CAL_PIN=//p')"
PORT_EXPECT="$(printf '%s\n' "$SEC1" | sed -n 's/^PORT_EXPECT=//p')"
if ! printf '%s' "$CAL_PIN" | grep -Eq '^[0-9a-f]{7,40}$'; then
  echo "FATAL: CAL_PIN='${CAL_PIN}' is not a hex commit id" >&2; exit 3
fi
if ! printf '%s' "$PORT_EXPECT" | grep -Eq '^\[[0-9]+(\.[0-9]+)?,[0-9]+(\.[0-9]+)?\] src=[^ ]+ not_a_gate$'; then
  echo "FATAL: PORT_EXPECT='${PORT_EXPECT}' is not '[lo,hi] src=<src> not_a_gate'" >&2; exit 3
fi
EXP_LO="$(printf '%s' "$PORT_EXPECT" | sed -E 's/^\[([^,]+),.*/\1/')"
EXP_HI="$(printf '%s' "$PORT_EXPECT" | sed -E 's/^\[[^,]+,([^]]+)\].*/\1/')"

# ---- 0c. the rate program, taken from the frozen spec373b-verdict.sh
VERDICT="$SCRIPT_DIR/spec373b-verdict.sh"
VERDICT_SHA=d5cef0dfe2033f13dc8c5446a49233046a38beeaa43f5ea24817aab1d836dffa
RATE_SHA=35fb23008728703b9b2159061ed513d0d040f1554d1f3f47917ea401d2cc01fd
if [ "$(shasum -a 256 "$VERDICT" 2>/dev/null | awk '{print $1}')" != "$VERDICT_SHA" ]; then
  echo "FATAL: ${VERDICT} does not hash to the frozen ${VERDICT_SHA}" >&2; exit 3
fi
RATE_PROG="$(sed -n '151,170p' "$VERDICT" | sed '$ s/'\'' "\$CSV" || FAILED="\${FAILED} RATE_\${c}"$//')"
[ "$(printf '%s\n' "$RATE_PROG" | tail -1)" = "      }" ] \
  || { echo "FATAL: the extracted rate program does not end at its closing brace" >&2; exit 3; }
[ "$(printf '%s\n' "$RATE_PROG" | shasum -a 256 | awk '{print $1}')" = "$RATE_SHA" ] \
  || { echo "FATAL: the extracted rate program does not hash to the frozen ${RATE_SHA}" >&2; exit 3; }

echo "== section 1 (read from the manifest, not recomputed) =="
echo "MANIFEST_CAL_PIN=${CAL_PIN}"
echo "MANIFEST_PORT_EXPECT=${PORT_EXPECT}"

# The intermediate lives outside the evidence dir, in a fresh file this run
# created, so a stale file can never be read as this run's readings.
READ="$(mktemp "${TMPDIR:-/tmp}/spec376-calib.XXXXXX")" || { echo "FATAL: cannot create the intermediate file" >&2; exit 4; }
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
    if ! printf '%s' "$PL" | grep -Eq '^spec376-preflight-[A-Za-z0-9._:-]+\.log$'; then
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

{
  echo "== chain log =="
  echo "PROC_ROOT_SEEN=$(key_once "$CHAIN_LOG" PROC_ROOT)"
  echo "PREFLIGHT_CLAUSE=${H_PREFLIGHT:-none}"
  for c in $CELLS; do
    BASE="spec376-${c}"
    CSV="$EV/$BASE.csv"; MATRIX="$EV/$BASE.matrix.txt"; RUNNER="$EV/$BASE.runner-console.log"
    SOAK="$EV/$BASE.soak.json"; PRED="$EV/$BASE.predicates.txt"
    echo "== cell ${c} =="
    if [ -f "$PRED" ]; then
      echo "PRED_${c}=present"
      for p in PV PEL PA PM1 PMEM; do
        n="$(grep -c "^${p}=" "$PRED")"
        case "$n" in
          0) v=absent ;;
          1) v="$(sed -n "s/^${p}=//p" "$PRED" | awk '{ print ($1 == "" ? "empty" : $1) }')" ;;
          *) v=dup ;;
        esac
        echo "PRED_${c}_${p}=${v}"
      done
    else
      echo "PRED_${c}=missing"
    fi
    echo "PREDRC_${c}=$(key_once "$CHAIN_LOG" "PREDICATES_EXIT_${c}")"
    echo "RUNNER_EXIT_${c}=$(key_once "$RUNNER" RUNNER_EXIT)"
    echo "STEAL_${c}=$(key_once "$RUNNER" steal_pct)"
    echo "LOAD_${c}=$(key_once "$CHAIN_LOG" "LOAD_AT_START_${c}")"
    echo "FPL_END_${c}=$(key_once "$PRED" FPL_end | awk '{ print $1 }')"
    n_tw="$( [ -f "$SOAK" ] && grep -c '"totalWrites":' "$SOAK")"
    case "${n_tw:-0}" in
      0) echo "TOTAL_WRITES_${c}=absent" ;;
      1) TW="$(sed -nE 's/.*"totalWrites": *([0-9]+).*/\1/p' "$SOAK")"; echo "TOTAL_WRITES_${c}=${TW:-empty}" ;;
      *) echo "TOTAL_WRITES_${c}=dup" ;;
    esac
    n_du="$( [ -f "$MATRIX" ] && grep -c '^  duration: ' "$MATRIX")"
    case "${n_du:-0}" in
      0) echo "DURATION_${c}=absent" ;;
      1) DU="$(sed -nE 's/^  duration: +([0-9]+)s.*/\1/p' "$MATRIX")"; echo "DURATION_${c}=${DU:-absent}" ;;
      *) echo "DURATION_${c}=dup" ;;
    esac
    if [ -s "$CSV" ]; then
      # alloc_live_mb at t = 60: the row nearest 60 s (within 30 s) carrying a number.
      A0="$(awk -F, '
        function num(x) { return x ~ /^[0-9]+(\.[0-9]+)?$/ }
        NR == 1 { for (i = 1; i <= NF; i++) col[$i] = i
                  if (!("alloc_live_mb" in col) || !("elapsed_secs" in col)) { bad = 1; exit } next }
        { e = $col["elapsed_secs"]; v = $col["alloc_live_mb"]
          if (!num(e) || !num(v)) next
          d = e - 60; if (d < 0) d = -d
          if (d <= 30 && (!have || d < bd)) { bd = d; best = v; have = 1 } }
        END { if (bad) print "n/a reason=missing_csv_column"; else if (!have) print "n/a reason=no_row_near_60s"; else print best }' "$CSV")" \
        || FAILED="${FAILED} A0_${c}"
      echo "A0_${c}=${A0}"
      awk -F, -v c="$c" "$RATE_PROG" "$CSV" || FAILED="${FAILED} RATE_${c}"
    else
      echo "A0_${c}=n/a reason=no_csv"
      echo "BYTES_ALLOC_RATE_${c}=n/a reason=no_csv"
      echo "ALLOC_LIVE_${c}=n/a reason=no_csv"
    fi
  done
} > "$READ" || FAILED="${FAILED} WRITE_INTERMEDIATE"
[ -s "$READ" ] || FAILED="${FAILED} EMPTY_INTERMEDIATE"
if [ -n "$FAILED" ]; then
  cat "$READ"; echo "FATAL: a program step failed:${FAILED}; no flags printed (kept ${READ})" >&2; exit 4
fi
cat "$READ"

# ---- smoke admission, exactly once by key
SMOKE_LOG="$SMOKE/spec376-chain.log"
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

OUT_FLAGS="$(awk -F= -v smoke="$SMOKE_SEEN" -v pexp="$PORT_EXPECT" -v elo="$EXP_LO" -v ehi="$EXP_HI" '
  function num(x) { return x ~ /^[0-9]+(\.[0-9]+)?$/ }
  function ab(x) { return x < 0 ? -x : x }
  function first(x) { sub(/ .*/, "", x); return x }
  function add(s, cl) { return s (s == "" ? "" : " ") cl }
  { k = $1; v = substr($0, length($1) + 2); V[k] = v }
  END {
    split("c1 pb c2 pa", C, " ")
    # STOP-H
    h = ""
    if (V["PREFLIGHT_CLAUSE"] != "none") h = add(h, V["PREFLIGHT_CLAUSE"])
    for (i = 1; i <= 4; i++) {
      c = C[i]; s = V["STEAL_" c]
      if (!num(s)) h = add(h, c ":steal=missing")
      else if (s + 0 > 1.0) h = add(h, c ":steal=" s)
    }
    # STOP-V
    vf = ""
    if (V["PROC_ROOT_SEEN"] != "/proc") vf = add(vf, "chain:proc_root=" (V["PROC_ROOT_SEEN"] == "" ? "empty" : V["PROC_ROOT_SEEN"]))
    np = split("PV PEL PA PM1 PMEM", P, " ")
    for (i = 1; i <= 4; i++) {
      c = C[i]
      if (V["PRED_" c] != "present") vf = add(vf, c ":predicates_missing")
      else for (j = 1; j <= np; j++) if (V["PRED_" c "_" P[j]] != "TRUE") vf = add(vf, c ":" P[j] "=" V["PRED_" c "_" P[j]])
      if (V["PREDRC_" c] != "0") vf = add(vf, c ":predicates_rc=" V["PREDRC_" c])
      if (V["RUNNER_EXIT_" c] != "0") vf = add(vf, c ":runner_exit=" V["RUNNER_EXIT_" c])
      if (!(("SKIPPED_" c) in V)) vf = add(vf, c ":skipped=absent")
      else if (!num(V["SKIPPED_" c])) vf = add(vf, c ":skipped=" V["SKIPPED_" c])
      else if (V["SKIPPED_" c] + 0 > 0) vf = add(vf, c ":skipped=" V["SKIPPED_" c])
      R[c] = (("BYTES_ALLOC_RATE_" c) in V) ? first(V["BYTES_ALLOC_RATE_" c]) : "absent"
      L[c] = (("ALLOC_LIVE_" c) in V) ? first(V["ALLOC_LIVE_" c]) : "absent"
      W[c] = V["TOTAL_WRITES_" c]; D[c] = V["DURATION_" c]
      okR[c] = num(R[c]) && R[c] + 0 > 0; okL[c] = num(L[c]) && L[c] + 0 > 0; okW[c] = num(W[c]) && W[c] + 0 > 0
      okD[c] = num(D[c]) && D[c] + 0 > 0
      if (!okR[c]) vf = add(vf, c ":rate=" R[c])
      if (!okL[c]) vf = add(vf, c ":live=" L[c])
      if (!okW[c]) vf = add(vf, c ":totalWrites=" W[c])
    }
    if (okW["c1"] && okW["c2"]) {
      wp = ab(W["c1"] - W["c2"]) / ((W["c1"] + W["c2"]) / 2); parity = sprintf("%.4f", wp)
      if (wp > 0.05) vf = add(vf, "c1c2:write_parity=" parity)
    } else { parity = "n/a"; vf = add(vf, "c1c2:write_parity=n/a") }

    print "== stop predicates =="
    print "STOP_H_CLAUSES=" (h == "" ? "none" : h)
    print "STOP_V_CLAUSES=" (vf == "" ? "none" : vf)

    # recorded readings
    if (!okD["c1"] || !okD["c2"]) wps = "n/a reason=DURATION_" (okD["c1"] ? "c2" : "c1") "=absent"
    else if (!okW["c1"] || !okW["c2"]) wps = "n/a reason=totalWrites_" (okW["c1"] ? "c2" : "c1") "=" (okW["c1"] ? W["c2"] : W["c1"])
    else wps = sprintf("%.3f", (W["c1"] / D["c1"] + W["c2"] / D["c2"]) / 2)
    sr = (okR["c1"] && okR["c2"]) ? sprintf("%.6f", ab(R["c1"] - R["c2"]) / ((R["c1"] + R["c2"]) / 2)) : "n/a reason=rate_missing"
    sl = (okL["c1"] && okL["c2"]) ? sprintf("%.6f", ab(L["c1"] - L["c2"]) / ((L["c1"] + L["c2"]) / 2)) : "n/a reason=live_missing"
    f1 = V["FPL_END_c1"]; f2 = V["FPL_END_c2"]
    sf = (num(f1) && num(f2) && f1 + f2 > 0) ? sprintf("%.6f", ab(f1 - f2) / ((f1 + f2) / 2)) : "n/a reason=FPL_end_missing"
    rates = ""; lives = ""; bpws = ""; steals = ""; loads = ""
    wmin = 1e18; wmax = -1; wsum = 0; wall = 1
    for (i = 1; i <= 4; i++) {
      c = C[i]
      rates = add(rates, c "=" (okR[c] ? R[c] : "n/a"))
      lives = add(lives, c "=" (okL[c] ? L[c] : "n/a"))
      if (!okD[c]) bpw[c] = "n/a reason=DURATION_" c "=absent"
      else if (!okR[c] || !okW[c]) bpw[c] = "n/a"
      else bpw[c] = sprintf("%.1f", R[c] / (W[c] / D[c]))
      bpws = add(bpws, c "=" bpw[c])
      steals = add(steals, c "=" (num(V["STEAL_" c]) ? V["STEAL_" c] : "n/a"))
      ld = V["LOAD_" c]; loads = add(loads, c "=" (num(ld) ? ld : (ld == "dup" ? "dup" : "absent")))
      if (okW[c]) { wsum += W[c]; if (W[c] + 0 < wmin) wmin = W[c] + 0; if (W[c] + 0 > wmax) wmax = W[c] + 0 } else wall = 0
    }
    pall = wall ? sprintf("%.4f", (wmax - wmin) / (wsum / 4)) : "n/a"
    a1 = V["A0_c1"]; a2 = V["A0_c2"]
    a0 = (num(a1) && num(a2)) ? sprintf("%.3f", (a1 + a2) / 2) : "n/a reason=alloc_live_mb_at_60s_missing"
    if (okR["pa"] && okR["pb"]) { pr = R["pa"] / R["pb"]; prs = sprintf("%.4f", pr) } else prs = "n/a"
    pbr = (bpw["pa"] ~ /^[0-9]/ && bpw["pb"] ~ /^[0-9]/ && bpw["pb"] + 0 > 0) ? sprintf("%.4f", bpw["pa"] / bpw["pb"]) : "n/a"
    inexp = (prs != "n/a" && pr >= elo + 0 && pr <= ehi + 0) ? "TRUE" : "FALSE"

    stop = "none"
    if (h != "") stop = "H (" h ")"
    else if (vf != "") stop = "V (" vf ")"
    if (stop != "none") inst = "NOT_SOUND reason=stop"
    else if (smoke != "PASS") inst = "NOT_SOUND reason=smoke"
    else inst = "SOUND"

    print "== flags =="
    print "STOP=" stop
    print "INSTRUMENT=" inst
    print "WRITES_PER_S_REF=" wps
    print "S_CA_RATE=" sr
    print "S_CA_LIVE=" sl
    print "S_CA_FPL=" sf
    print "BYTES_ALLOC_RATE=" rates
    print "ALLOC_LIVE=" lives
    print "BYTES_PER_WRITE=" bpws
    print "WRITE_PARITY=" parity
    print "WRITE_PARITY_ALL=" pall
    print "A0_L_MIB=" a0
    print "PORT_RATIO=" prs
    print "PORT_BPW_RATIO=" pbr
    print "PORT_EXPECT=" pexp
    print "PORT_IN_EXPECT=" inexp
    print "STEAL=" steals
    print "LOAD_AT_START=" loads
  }' "$READ")" || { echo "FATAL: the reading step failed; no flags printed (kept ${READ})" >&2; exit 4; }
printf '%s\n' "$OUT_FLAGS"
rm -f "$READ"
