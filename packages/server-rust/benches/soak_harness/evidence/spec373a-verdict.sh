#!/usr/bin/env bash
# spec373a verdict: carve 9c part a, count-alloc cells b1 b2 (pin 61f84658) and
# a1 a2 (branch head). Frozen at M.
#
# usage: spec373a-verdict.sh <EV_DIR> <MANIFEST>
#   reads spec373a-{b1,b2,a1,a2}.{csv,matrix.txt,runner-console.log} from EV_DIR
#   and the pre-M flags SHARES_STOP / E_FROZEN from MANIFEST (never recomputed)
#
# Order (flags LAST, after every STOP predicate):
#   0. STOP-D/O/S: the manifest's SHARES_STOP line (decided before M by shares_61f.py)
#   1. STOP-V per cell: PA and PM1, verbatim spec371-predicates.sh definitions
#   2. BYTES_ALLOC_RATE / ALLOC_LIVE per cell, verbatim spec372-k.awk CHURN_RATIO
#      factors: one point per DISTINCT alloc_probe_elapsed_s; last half = points
#      floor(n/2)..n-1; rate = d bytes_alloc / d probe seconds over it;
#      live = alloc_live_bytes at its last point
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

SHARES_STOP="$(awk -F= '/^SHARES_STOP=/ { print $2; exit }' "$MANIFEST")"
E="$(awk -F= '/^E_FROZEN=/ { print $2; exit }' "$MANIFEST")"
echo "== pre-M (read from the manifest, not recomputed) =="
echo "SHARES_STOP=${SHARES_STOP:-missing}"
echo "E_FROZEN=${E:-missing}"

matrix_int() { awk -v k="$1" 'index($0, k) { s = substr($0, index($0, k) + length(k)); if (match(s, /[0-9]+/)) { print substr(s, RSTART, RLENGTH); exit } }' "$2" 2>/dev/null; }

for c in b1 b2 a1 a2; do
  BASE="spec373a-${c}"
  CSV="$EV/$BASE.csv"; MATRIX="$EV/$BASE.matrix.txt"; RUNNER="$EV/$BASE.runner-console.log"
  DURATION="$(matrix_int 'duration:' "$MATRIX")"
  CADENCE="$(matrix_int 'csv cadence:' "$MATRIX")"
  echo "== cell ${c} =="
  # PA (spec371-predicates.sh, CA branch, verbatim predicate)
  if [ -z "$DURATION" ] || [ -z "$CADENCE" ] || [ ! -s "$CSV" ]; then
    PA_LINE="PA=FALSE reason=no_matrix_or_csv"
  else
    PA_LINE="$(awk -F, -v dur="$DURATION" '
      NR == 1 {
        for (i = 1; i <= NF; i++) col[$i] = i
        if (!("phys_footprint_mb" in col) || !("elapsed_secs" in col) || !("alloc_probe_seq" in col)) { print "PA=FALSE reason=missing_csv_column"; bad = 1; exit }
        next
      }
      {
        ok = ($col["phys_footprint_mb"] != "") && $col["alloc_live_bytes"] != "" && $col["alloc_live_mb"] != "" && $col["alloc_probe_elapsed_s"] != "" && $col["alloc_probe_seq"] != ""
        if (ok) {
          if ($col["alloc_probe_elapsed_s"] + 0 < $col["elapsed_secs"] - 35) stale++
          if (seen && $col["alloc_probe_seq"] + 0 < lastseq) back++
          lastseq = $col["alloc_probe_seq"] + 0; seen = 1
        }
      }
      END {
        if (bad) exit
        minseq = int(dur / 30) - 1
        if (stale + 0 == 0 && back + 0 == 0 && lastseq >= minseq) print "PA=TRUE last_seq=" lastseq " min_seq=" minseq
        else print "PA=FALSE stale=" stale + 0 " backwards=" back + 0 " last_seq=" lastseq + 0 " min_seq=" minseq
      }' "$CSV")"
  fi
  echo "PA_${c}=${PA_LINE#PA=}"
  # PM1 (spec371-predicates.sh, verbatim predicate)
  PM_ROWS="$(awk -F= '/^post_mortem_rows=/ { v = $2 } END { print (v == "" ? "NA" : v) }' "$RUNNER" 2>/dev/null || echo NA)"
  if [ "$PM_ROWS" = "NA" ] || [ -z "$DURATION" ] || [ -z "$CADENCE" ] || [ ! -s "$CSV" ]; then
    PM_LINE="PM1=FALSE reason=no_counter_or_csv post_mortem_rows=${PM_ROWS}"
  else
    PM_LINE="$(awk -F, -v pm="$PM_ROWS" -v dur="$DURATION" -v cad="$CADENCE" '
      NR == 1 { for (i = 1; i <= NF; i++) col[$i] = i; if (!("tombstone_bytes" in col)) { print "PM1=FALSE reason=missing_csv_column"; bad = 1; exit } next }
      {
        e = $col["elapsed_secs"] + 0
        if ($col["tombstone_bytes"] == "") { empt++; if (!havemin || e < minempty) { minempty = e; havemin = 1 } }
        else lastlive = e
      }
      END {
        if (bad) exit
        need = dur - 2 * cad
        ok = (pm + 0 <= 1) && (pm + 0 == 0 || (havemin && minempty >= dur)) && (lastlive + 0 >= need)
        printf "PM1=%s post_mortem_rows=%s empty_scrape_rows=%d first_empty_elapsed=%s last_live_scrape_elapsed=%d need_ge=%d\n",
               (ok ? "TRUE" : "FALSE"), pm, empt + 0, (havemin ? minempty "" : "none"), lastlive + 0, need
      }' "$CSV")"
  fi
  echo "PM1_${c}=${PM_LINE#PM1=}"
  # Recorded, not STOP: the runner's own exit and the server identity line.
  echo "RUNNER_EXIT_${c}=$(awk -F= '/^RUNNER_EXIT=/ { v = $2 } END { print (v == "" ? "missing" : v) }' "$RUNNER" 2>/dev/null)"
  # BYTES_ALLOC_RATE / ALLOC_LIVE (spec372-k.awk CHURN_RATIO factors)
  if [ -s "$CSV" ]; then
    awk -F, -v c="$c" '
      function num(x) { return x ~ /^[0-9]+(\.[0-9]+)?$/ }
      NR == 1 { for (i = 1; i <= NF; i++) col[$i] = i; next }
      {
        if (!(("bytes_alloc" in col) && ("alloc_live_bytes" in col) && ("alloc_probe_elapsed_s" in col))) next
        b = $col["bytes_alloc"]; l = $col["alloc_live_bytes"]; e = $col["alloc_probe_elapsed_s"]
        if (!num(b) || !num(l) || !num(e) || (np > 0 && e + 0 == ke[np])) next
        np++; kb[np] = b + 0; kl[np] = l + 0; ke[np] = e + 0
      }
      END {
        n = np
        if (n < 3) { print "BYTES_ALLOC_RATE_" c "=n/a reason=too_few_probe_points points=" n + 0; print "ALLOC_LIVE_" c "=n/a reason=too_few_probe_points"; exit }
        h = int((n - 1) / 2) + 1
        if (ke[n] <= ke[h] || kl[n] <= 0) { print "BYTES_ALLOC_RATE_" c "=n/a reason=degenerate_window points=" n; print "ALLOC_LIVE_" c "=n/a reason=degenerate_window"; exit }
        printf "BYTES_ALLOC_RATE_%s=%.3f points=%d window_s=%s-%s\n", c, (kb[n] - kb[h]) / (ke[n] - ke[h]), n, ke[h], ke[n]
        printf "ALLOC_LIVE_%s=%d\n", c, kl[n]
      }' "$CSV"
  else
    echo "BYTES_ALLOC_RATE_${c}=n/a reason=no_csv"
    echo "ALLOC_LIVE_${c}=n/a reason=no_csv"
  fi
done | tee "$EV/.spec373a-verdict.cells"

READ="$EV/.spec373a-verdict.cells"
V_FAIL="$(awk -F= '/^(PA|PM1)_/ && $2 !~ /^TRUE/ { split($1, k, "_"); printf " %s:%s", k[2], k[1] }' "$READ")"
awk -F= -v e="$E" -v shares="$SHARES_STOP" -v vfail="$V_FAIL" '
  function num(x) { return x ~ /^[0-9]+(\.[0-9]+)?$/ }
  function ab(x) { return x < 0 ? -x : x }
  function mx(a, b) { return a > b ? a : b }
  /^BYTES_ALLOC_RATE_/ { c = substr($1, 18); v = $2; sub(/ .*/, "", v); R[c] = v }
  /^ALLOC_LIVE_/ { c = substr($1, 12); v = $2; sub(/ .*/, "", v); L[c] = v }
  END {
    have = num(e) && num(R["b1"]) && num(R["b2"]) && num(R["a1"]) && num(R["a2"]) && num(L["b1"]) && num(L["b2"]) && num(L["a1"]) && num(L["a2"]) && R["b1"] > 0 && R["b2"] > 0 && L["b1"] > 0 && L["b2"] > 0
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
    } else print "READINGS=n/a reason=missing_or_nonpositive_input"
    # STOP precedence D > O > S > V > R; D/O/S come from the manifest.
    stop = "none"
    if (shares != "none") stop = (shares == "" ? "D" : shares)
    else if (vfail != "") stop = "V"
    else if (!have) stop = "V"
    else if (stopR) stop = "R"
    print "== flags =="
    print "STOP=" stop (stop == "V" ? " (" (vfail != "" ? substr(vfail, 2) : "missing_readings") ")" : "")
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
  }' "$READ"
rm -f "$READ"
