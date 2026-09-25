#!/usr/bin/env bash
# spec373a verdict: carve 9c part a, count-alloc cells b1 b2 (pin 61f84658) and
# a1 a2 (branch head). Frozen at M' (supersedes M).
#
# usage: spec373a-verdict.sh <EV_DIR> <MANIFEST>
#   reads spec373a-{b1,b2,a1,a2}.{csv,matrix.txt,runner-console.log} from EV_DIR
#   and the pre-M' lines SHARES_STOP / E_FROZEN from MANIFEST (never recomputed)
#
# Exit status: 0 = the flags block was printed; 3 = a pre-M' line or the frozen
# predicate source is invalid (NO flags printed); 4 = a program step failed
# (NO flags printed; the intermediate file is kept for inspection).
#
# Order (flags LAST, after every STOP predicate):
#   0. SHARES_STOP / E_FROZEN: exactly one line each, SHARES_STOP in {none,D,O,S},
#      E_FROZEN numeric -- else exit 3. STOP-D/O/S = SHARES_STOP.
#   1. STOP-V per cell: PA and PM1, evaluated by the awk programs TAKEN FROM the
#      frozen spec371-predicates.sh (sha256 asserted; lines 101-131 = the PE/PA
#      program, lines 154-169 = the PM1 program; only the invocation line and the
#      closing `' "$CSV"` are shell plumbing and are not part of the program).
#      Also STOP-V: SKIPPED_<cell> > 0 (rows whose three probe fields are neither
#      all empty nor all numeric), or any BYTES_ALLOC_RATE / ALLOC_LIVE that is
#      n/a or not > 0.
#   2. BYTES_ALLOC_RATE / ALLOC_LIVE per cell, the spec372-k.awk CHURN_RATIO
#      factors, same code: one point per DISTINCT alloc_probe_elapsed_s (adjacent
#      repeats dropped); n points; the last half starts at the 1-based point
#      h = int((n-1)/2)+1 (= 0-based ceil(n/2)-1: floor(n/2) for odd n, one point
#      earlier for even n) and ends at point n; rate = d bytes_alloc / d probe
#      seconds over it; live = alloc_live_bytes at point n. Printed %.6f.
#   3. cross ratios R_ij = a_i / b_j -> I = [min, max]; same for live -> LIVE_I
#   4. spreads s = |x1 - x2| / mean(x1, x2), per side and metric
#   5. STOP-R: min I > 1 + max(s_b, s_a) (rate) or min LIVE_I > 1 + max(s_b_live, s_a_live)
#   6. class (rate): INDETERMINATE iff s_b >= E/2; else CONFIRMED iff max I <= 1 - E/2;
#      else NOT_MET
#   7. STOP=<first of D > O > S > V > R | none>, VERDICT_BYTES=<class | WITHHELD>,
#      I=[..], S_B=.., E=.., LIVE_I=[..]
set -uo pipefail
export LC_ALL=C
EV="${1:-}"; MANIFEST="${2:-}"
if [ "$#" -ne 2 ] || [ ! -d "$EV" ] || [ ! -f "$MANIFEST" ]; then
  echo "usage: spec373a-verdict.sh <EV_DIR> <MANIFEST>" >&2; exit 2
fi
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"

# ---- 0. pre-M' lines: validated, never defaulted
n_stop="$(grep -c '^SHARES_STOP=' "$MANIFEST" || true)"
n_e="$(grep -c '^E_FROZEN=' "$MANIFEST" || true)"
SHARES_STOP="$(sed -n 's/^SHARES_STOP=//p' "$MANIFEST")"
E="$(sed -n 's/^E_FROZEN=//p' "$MANIFEST")"
if [ "$n_stop" != "1" ] || [ "$n_e" != "1" ]; then
  echo "FATAL: the manifest must carry exactly one SHARES_STOP= and one E_FROZEN= line (found ${n_stop} / ${n_e})" >&2; exit 3
fi
case "$SHARES_STOP" in none|D|O|S) ;; *) echo "FATAL: SHARES_STOP='${SHARES_STOP}' is not one of none|D|O|S" >&2; exit 3 ;; esac
if ! printf '%s' "$E" | grep -Eq '^[0-9]+(\.[0-9]+)?$'; then
  echo "FATAL: E_FROZEN='${E}' is not a number" >&2; exit 3
fi

# ---- 1. the frozen predicate programs, taken from spec371-predicates.sh
PRED371="$SCRIPT_DIR/spec371-predicates.sh"
PRED371_SHA=7d2ca6214beff1c4c0042879823172a45452ef99c06ca49521d5c96889a61d1b
if [ "$(shasum -a 256 "$PRED371" 2>/dev/null | awk '{print $1}')" != "$PRED371_SHA" ]; then
  echo "FATAL: ${PRED371} does not hash to the frozen ${PRED371_SHA}" >&2; exit 3
fi
PEPA_PROG="$(sed -n '101,131p' "$PRED371" | sed '$ s/'\'' "\$CSV"$//')"
PM1_PROG="$(sed -n '154,169p' "$PRED371" | sed '$ s/'\'' "\$CSV"$//')"
case "$PEPA_PROG" in *'print "PA=TRUE last_seq="'*) ;; *) echo "FATAL: PE/PA program extraction failed" >&2; exit 3 ;; esac
case "$PM1_PROG" in *'printf "PM1=%s post_mortem_rows='*) ;; *) echo "FATAL: PM1 program extraction failed" >&2; exit 3 ;; esac
for prog in "$PEPA_PROG" "$PM1_PROG"; do
  [ "$(printf '%s\n' "$prog" | tail -1)" = "      }" ] || { echo "FATAL: an extracted predicate program does not end at its closing brace" >&2; exit 3; }
done

echo "== pre-M' (read from the manifest, not recomputed) =="
echo "SHARES_STOP=${SHARES_STOP}"
echo "E_FROZEN=${E}"

matrix_int() { awk -v k="$1" 'index($0, k) { s = substr($0, index($0, k) + length(k)); if (match(s, /[0-9]+/)) { print substr(s, RSTART, RLENGTH); exit } }' "$2" 2>/dev/null; }

READ="$EV/.spec373a-verdict.cells"
FAILED=""
for c in b1 b2 a1 a2; do
  BASE="spec373a-${c}"
  CSV="$EV/$BASE.csv"; MATRIX="$EV/$BASE.matrix.txt"; RUNNER="$EV/$BASE.runner-console.log"
  DURATION="$(matrix_int 'duration:' "$MATRIX")"
  CADENCE="$(matrix_int 'csv cadence:' "$MATRIX")"
  echo "== cell ${c} =="
  # PA: the frozen PE/PA program with fl=CA; the wrapper's missing-input branch is
  # spec371-predicates.sh's own (plumbing).
  if [ -z "$DURATION" ] || [ -z "$CADENCE" ] || [ ! -s "$CSV" ]; then
    PA_LINE="PA=FALSE reason=no_matrix_or_csv"
  else
    PA_OUT="$(awk -F, -v fl="CA" -v dur="$DURATION" -v cad="$CADENCE" "$PEPA_PROG" "$CSV")" || FAILED="${FAILED} PA_${c}"
    PA_LINE="$(printf '%s\n' "$PA_OUT" | grep '^PA=' || echo 'PA=FALSE reason=no_PA_line')"
  fi
  echo "PA_${c}=${PA_LINE#PA=}"
  # PM1: the frozen PM1 program.
  PM_ROWS="$(awk -F= '/^post_mortem_rows=/ { v = $2 } END { print (v == "" ? "NA" : v) }' "$RUNNER" 2>/dev/null || echo NA)"
  if [ "$PM_ROWS" = "NA" ] || [ -z "$DURATION" ] || [ -z "$CADENCE" ] || [ ! -s "$CSV" ]; then
    PM_LINE="PM1=FALSE reason=no_counter_or_csv post_mortem_rows=${PM_ROWS}"
  else
    PM_LINE="$(awk -F, -v pm="$PM_ROWS" -v dur="$DURATION" -v cad="$CADENCE" "$PM1_PROG" "$CSV")" || FAILED="${FAILED} PM1_${c}"
  fi
  echo "PM1_${c}=${PM_LINE#PM1=}"
  echo "RUNNER_EXIT_${c}=$(awk -F= '/^RUNNER_EXIT=/ { v = $2 } END { print (v == "" ? "missing" : v) }' "$RUNNER" 2>/dev/null)"
  # BYTES_ALLOC_RATE / ALLOC_LIVE (spec372-k.awk CHURN_RATIO factors) + skipped rows.
  if [ -s "$CSV" ]; then
    awk -F, -v c="$c" '
      function num(x) { return x ~ /^[0-9]+(\.[0-9]+)?$/ }
      NR == 1 { for (i = 1; i <= NF; i++) col[$i] = i; next }
      {
        if (!(("bytes_alloc" in col) && ("alloc_live_bytes" in col) && ("alloc_probe_elapsed_s" in col))) { nocol = 1; next }
        b = $col["bytes_alloc"]; l = $col["alloc_live_bytes"]; e = $col["alloc_probe_elapsed_s"]
        if (b == "" && l == "" && e == "") next
        if (!num(b) || !num(l) || !num(e)) { skipped++; next }
        if (np > 0 && e + 0 == ke[np]) next
        np++; kb[np] = b + 0; kl[np] = l + 0; ke[np] = e + 0
      }
      END {
        print "SKIPPED_" c "=" skipped + 0
        n = np
        if (nocol) { print "BYTES_ALLOC_RATE_" c "=n/a reason=missing_csv_column"; print "ALLOC_LIVE_" c "=n/a reason=missing_csv_column"; exit }
        if (n < 3) { print "BYTES_ALLOC_RATE_" c "=n/a reason=too_few_probe_points points=" n + 0; print "ALLOC_LIVE_" c "=n/a reason=too_few_probe_points"; exit }
        h = int((n - 1) / 2) + 1
        if (ke[n] <= ke[h]) { print "BYTES_ALLOC_RATE_" c "=n/a reason=degenerate_window points=" n; print "ALLOC_LIVE_" c "=n/a reason=degenerate_window"; exit }
        printf "BYTES_ALLOC_RATE_%s=%.6f points=%d h=%d window_s=%s-%s\n", c, (kb[n] - kb[h]) / (ke[n] - ke[h]), n, h, ke[h], ke[n]
        printf "ALLOC_LIVE_%s=%d\n", c, kl[n]
      }' "$CSV" || FAILED="${FAILED} RATE_${c}"
  else
    echo "SKIPPED_${c}=0"
    echo "BYTES_ALLOC_RATE_${c}=n/a reason=no_csv"
    echo "ALLOC_LIVE_${c}=n/a reason=no_csv"
  fi
done > "$READ"
if [ -n "$FAILED" ]; then
  cat "$READ"; echo "FATAL: a program step failed:${FAILED}; no flags printed (kept ${READ})" >&2; exit 4
fi
cat "$READ"

OUT_FLAGS="$(awk -F= -v e="$E" -v shares="$SHARES_STOP" '
  function num(x) { return x ~ /^[0-9]+(\.[0-9]+)?$/ }
  function ab(x) { return x < 0 ? -x : x }
  function mx(a, b) { return a > b ? a : b }
  /^(PA|PM1)_/ { if ($2 !~ /^TRUE/) { split($1, k, "_"); vfail = vfail " " k[2] ":" k[1] } }
  /^SKIPPED_/ { c = substr($1, 9); if ($2 + 0 > 0) vfail = vfail " " c ":skipped=" $2 }
  /^BYTES_ALLOC_RATE_/ { c = substr($1, 18); v = $2; sub(/ .*/, "", v); R[c] = v }
  /^ALLOC_LIVE_/ { c = substr($1, 12); v = $2; sub(/ .*/, "", v); L[c] = v }
  END {
    have = 1
    split("b1 b2 a1 a2", C, " ")
    for (i = 1; i <= 4; i++) {
      if (!num(R[C[i]]) || R[C[i]] + 0 <= 0) { have = 0; vfail = vfail " " C[i] ":rate=" (R[C[i]] == "" ? "missing" : R[C[i]]) }
      if (!num(L[C[i]]) || L[C[i]] + 0 <= 0) { have = 0; vfail = vfail " " C[i] ":live=" (L[C[i]] == "" ? "missing" : L[C[i]]) }
    }
    print "== readings =="
    if (have) {
      lo = 1e18; hi = -1e18; llo = 1e18; lhi = -1e18
      split("a1 a2", A, " "); split("b1 b2", B, " ")
      for (i = 1; i <= 2; i++) for (j = 1; j <= 2; j++) {
        r = R[A[i]] / R[B[j]]; l = L[A[i]] / L[B[j]]
        printf "R_%s_%s=%.4f LIVE_R_%s_%s=%.4f\n", A[i], B[j], r, A[i], B[j], l
        if (r < lo) lo = r; if (r > hi) hi = r; if (l < llo) llo = l; if (l > lhi) lhi = l
      }
      sb = ab(R["b1"] - R["b2"]) / ((R["b1"] + R["b2"]) / 2); sa = ab(R["a1"] - R["a2"]) / ((R["a1"] + R["a2"]) / 2)
      lsb = ab(L["b1"] - L["b2"]) / ((L["b1"] + L["b2"]) / 2); lsa = ab(L["a1"] - L["a2"]) / ((L["a1"] + L["a2"]) / 2)
      printf "S_A=%.4f S_B_LIVE=%.4f S_A_LIVE=%.4f\n", sa, lsb, lsa
      stopR = (lo > 1 + mx(sb, sa)) || (llo > 1 + mx(lsb, lsa))
      printf "STOP_R_PREDICATE=%s (rate min I %.4f vs %.4f; live min I %.4f vs %.4f)\n", (stopR ? "TRUE" : "FALSE"), lo, 1 + mx(sb, sa), llo, 1 + mx(lsb, lsa)
    } else print "READINGS=n/a"
    # STOP precedence D > O > S > V > R; D/O/S come from the manifest.
    stop = "none"
    if (shares != "none") stop = shares
    else if (vfail != "") stop = "V"
    else if (stopR) stop = "R"
    print "== flags =="
    print "STOP=" stop ((stop == "V") ? " (" substr(vfail, 2) ")" : "")
    if (stop != "none") cls = "WITHHELD"
    else if (sb >= e / 2) cls = "INDETERMINATE"
    else if (hi <= 1 - e / 2) cls = "CONFIRMED"
    else cls = "NOT_MET"
    print "VERDICT_BYTES=" cls
    if (have) {
      printf "I=[%.4f,%.4f]\n", lo, hi
      printf "S_B=%.4f\n", sb
      print "E=" e
      printf "LIVE_I=[%.4f,%.4f]\n", llo, lhi
    } else { print "I=n/a"; print "S_B=n/a"; print "E=" e; print "LIVE_I=n/a" }
  }' "$READ")" || { echo "FATAL: the verdict step failed; no flags printed (kept ${READ})" >&2; exit 4; }
printf '%s\n' "$OUT_FLAGS"
rm -f "$READ"
