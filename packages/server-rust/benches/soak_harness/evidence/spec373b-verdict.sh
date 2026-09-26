#!/usr/bin/env bash
# spec373b verdict: carve 9c part b, count-alloc cells b1 b2 (pin b166719d) and
# a1 a2 (the branch at the freeze). Frozen at M.
#
# usage: spec373b-verdict.sh <EV_DIR> <MANIFEST>
#   reads spec373b-{b1,b2,a1,a2}.{csv,matrix.txt,runner-console.log,soak.json}
#   from EV_DIR and the pre-M lines SHARES_STOP / E_FROZEN from MANIFEST (never
#   recomputed). SPEC373B_MANIFEST_COMMIT (= M) is required.
#
# Exit status: 0 = the flags block was printed; 3 = ORDER is not OK, or a pre-M
# line or the frozen predicate source is invalid (NO flags printed); 4 = a
# program step failed (NO flags printed; the intermediate file is kept).
#
# Order (flags LAST, after every STOP predicate):
#   0. ORDER=OK re-checked here (spec373b-order.sh <M> <MANIFEST>), not only at
#      the chain's start -- else exit 3. The one exception is a synthetic case:
#      SPEC373B_SYNTHETIC=1 skips it ONLY when neither EV_DIR nor MANIFEST is in
#      the evidence dir, and prints ORDER=SKIPPED (synthetic) instead.
#      Then SHARES_STOP / E_FROZEN: exactly one line each, SHARES_STOP in
#      {none,D,O,S}, E_FROZEN numeric -- else exit 3 (a missing line is never a
#      STOP). STOP-D/O/S = SHARES_STOP.
#   1. STOP-V per cell: PA and PM1, evaluated by the awk programs TAKEN FROM the
#      frozen spec371-predicates.sh (sha256 asserted; lines 101-131 = the PE/PA
#      program, lines 154-169 = the PM1 program; only the invocation line and
#      the closing `' "$CSV"` are shell plumbing); SKIPPED_<cell> > 0 (rows whose
#      three probe fields are neither all empty nor all numeric); any
#      BYTES_ALLOC_RATE, ALLOC_LIVE or totalWrites that is n/a or not > 0; and
#      WRITE_PARITY: |w_c - mean(w)| / mean(w) > 0.05 for any of the four cells'
#      totalWrites. The STOP line names every failing clause.
#   2. BYTES_ALLOC_RATE / ALLOC_LIVE per cell, the spec372-k.awk CHURN_RATIO
#      factors, same code: one point per DISTINCT alloc_probe_elapsed_s
#      (adjacent repeats dropped); n points; the last half starts at the 1-based
#      point h = int((n-1)/2)+1 and ends at point n; rate = d bytes_alloc / d
#      probe seconds over it; live = alloc_live_bytes at point n. Printed %.6f.
#   3. R_ij = a_i / b_j -> I = [min, max]; same for live -> LIVE_I.
#   4. s = |x1 - x2| / mean(x1, x2), per side and metric.
#   5. STOP-R (increase only): min I > 1 + max(s_b, s_a) or
#      min LIVE_I > 1 + max(s_b_live, s_a_live). A drop never fires it.
#   6. VERDICT_BYTES: INDETERMINATE iff s_b >= min(E/2, 0.05); else CONFIRMED
#      iff max I <= 1 - E/2; else NOT_MET. WITHHELD under any STOP.
#   7. VERDICT_LIVE=DESCRIPTIVE with LIVE_SIGN = down iff
#      max LIVE_I < 1 - max(s_b_live, s_a_live), up iff
#      min LIVE_I > 1 + max(s_b_live, s_a_live) (which is also STOP-R), else flat.
#   8. Recorded, not a STOP and not a class: EXCESS=TRUE iff 1 - min I > 1.5 E;
#      BYTES_PER_WRITE_<cell> = rate / (totalWrites / duration), duration = the
#      cell's matrix "duration:" (900 s for every real cell), and its a/b ratio
#      range; the four totalWrites and their max relative deviation.
#   9. Precedence D > O > S > V > R. Flags: STOP=, VERDICT_BYTES=,
#      VERDICT_LIVE=DESCRIPTIVE, LIVE_SIGN=, I=, S_B=, E=, LIVE_I=, S_B_LIVE=,
#      EXCESS=, BYTES_PER_WRITE=, WRITE_PARITY=.
set -uo pipefail
export LC_ALL=C
EV="${1:-}"; MANIFEST="${2:-}"
if [ "$#" -ne 2 ] || [ ! -d "$EV" ] || [ ! -f "$MANIFEST" ]; then
  echo "usage: spec373b-verdict.sh <EV_DIR> <MANIFEST>" >&2; exit 2
fi
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
EV_ABS="$(cd "$EV" && pwd -P)"
MAN_ABS="$(cd "$(dirname "$MANIFEST")" && pwd -P)/$(basename "$MANIFEST")"

# ---- 0. ORDER=OK at the verdict's own start
if [ "${SPEC373B_SYNTHETIC:-0}" = "1" ] && [ "$EV_ABS" != "$SCRIPT_DIR" ] && [ "$(dirname "$MAN_ABS")" != "$SCRIPT_DIR" ]; then
  ORDER_LINE="ORDER=SKIPPED (synthetic)"
else
  ORDER_LINE="$(bash "$SCRIPT_DIR/spec373b-order.sh" "${SPEC373B_MANIFEST_COMMIT:-}" "$MAN_ABS")"
  orc=$?
  if [ "$orc" -ne 0 ]; then echo "$ORDER_LINE"; echo "FATAL: ORDER is not OK (rc=${orc}); no flags printed" >&2; exit 3; fi
fi
echo "$ORDER_LINE"

# ---- 0b. pre-M lines: validated, never defaulted
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

echo "== pre-M (read from the manifest, not recomputed) =="
echo "SHARES_STOP=${SHARES_STOP}"
echo "E_FROZEN=${E}"

matrix_int() { awk -v k="$1" 'index($0, k) { s = substr($0, index($0, k) + length(k)); if (match(s, /[0-9]+/)) { print substr(s, RSTART, RLENGTH); exit } }' "$2" 2>/dev/null; }

READ="$EV/.spec373b-verdict.cells"
FAILED=""
for c in b1 b2 a1 a2; do
  BASE="spec373b-${c}"
  CSV="$EV/$BASE.csv"; MATRIX="$EV/$BASE.matrix.txt"; RUNNER="$EV/$BASE.runner-console.log"; SOAK="$EV/$BASE.soak.json"
  DURATION="$(matrix_int 'duration:' "$MATRIX")"
  CADENCE="$(matrix_int 'csv cadence:' "$MATRIX")"
  echo "== cell ${c} =="
  if [ -z "$DURATION" ] || [ -z "$CADENCE" ] || [ ! -s "$CSV" ]; then
    PA_LINE="PA=FALSE reason=no_matrix_or_csv"
  else
    PA_OUT="$(awk -F, -v fl="CA" -v dur="$DURATION" -v cad="$CADENCE" "$PEPA_PROG" "$CSV")" || FAILED="${FAILED} PA_${c}"
    PA_LINE="$(printf '%s\n' "$PA_OUT" | grep '^PA=' || echo 'PA=FALSE reason=no_PA_line')"
  fi
  echo "PA_${c}=${PA_LINE#PA=}"
  PM_ROWS="$(awk -F= '/^post_mortem_rows=/ { v = $2 } END { print (v == "" ? "NA" : v) }' "$RUNNER" 2>/dev/null || echo NA)"
  if [ "$PM_ROWS" = "NA" ] || [ -z "$DURATION" ] || [ -z "$CADENCE" ] || [ ! -s "$CSV" ]; then
    PM_LINE="PM1=FALSE reason=no_counter_or_csv post_mortem_rows=${PM_ROWS}"
  else
    PM_LINE="$(awk -F, -v pm="$PM_ROWS" -v dur="$DURATION" -v cad="$CADENCE" "$PM1_PROG" "$CSV")" || FAILED="${FAILED} PM1_${c}"
  fi
  echo "PM1_${c}=${PM_LINE#PM1=}"
  echo "RUNNER_EXIT_${c}=$(awk -F= '/^RUNNER_EXIT=/ { v = $2 } END { print (v == "" ? "missing" : v) }' "$RUNNER" 2>/dev/null)"
  TW="$(sed -nE 's/.*"totalWrites": *([0-9]+).*/\1/p' "$SOAK" 2>/dev/null | head -1)"
  echo "TOTAL_WRITES_${c}=${TW:-n/a}"
  echo "DURATION_${c}=${DURATION:-n/a}"
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
  function mn(a, b) { return a < b ? a : b }
  /^(PA|PM1)_/ { if ($2 !~ /^TRUE/) { split($1, k, "_"); vfail = vfail " " k[2] ":" k[1] } }
  /^SKIPPED_/ { c = substr($1, 9); if ($2 + 0 > 0) vfail = vfail " " c ":skipped=" $2 }
  /^BYTES_ALLOC_RATE_/ { c = substr($1, 18); v = $2; sub(/ .*/, "", v); R[c] = v }
  /^ALLOC_LIVE_/ { c = substr($1, 12); v = $2; sub(/ .*/, "", v); L[c] = v }
  /^TOTAL_WRITES_/ { c = substr($1, 14); W[c] = $2 }
  /^DURATION_/ { c = substr($1, 10); D[c] = $2 }
  END {
    have = 1; haveW = 1
    split("b1 b2 a1 a2", C, " ")
    for (i = 1; i <= 4; i++) {
      if (!num(R[C[i]]) || R[C[i]] + 0 <= 0) { have = 0; vfail = vfail " " C[i] ":rate=" (R[C[i]] == "" ? "missing" : R[C[i]]) }
      if (!num(L[C[i]]) || L[C[i]] + 0 <= 0) { have = 0; vfail = vfail " " C[i] ":live=" (L[C[i]] == "" ? "missing" : L[C[i]]) }
      if (!num(W[C[i]]) || W[C[i]] + 0 <= 0) { haveW = 0; vfail = vfail " " C[i] ":totalWrites=" (W[C[i]] == "" ? "missing" : W[C[i]]) }
    }
    print "== readings =="
    parity = "n/a"
    if (haveW) {
      wm = (W["b1"] + W["b2"] + W["a1"] + W["a2"]) / 4; wdev = 0
      for (i = 1; i <= 4; i++) {
        d = ab(W[C[i]] - wm) / wm
        printf "WRITE_DEV_%s=%.4f\n", C[i], d
        if (d > wdev) wdev = d
        if (d > 0.05) vfail = vfail " " C[i] ":write_parity=" sprintf("%.4f", d)
      }
      parity = sprintf("%s max_dev=%.4f totalWrites=%s/%s/%s/%s", (wdev > 0.05 ? "FAIL" : "OK"), wdev, W["b1"], W["b2"], W["a1"], W["a2"])
    }
    stopR = 0
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
      ls = mx(lsb, lsa)
      sign = (lhi < 1 - ls) ? "down" : ((llo > 1 + ls) ? "up" : "flat")
      excess = ((1 - lo) > 1.5 * e) ? "TRUE" : "FALSE"
      printf "EXCESS_PREDICATE=%s (1 - min I %.4f vs 1.5 E %.4f)\n", excess, 1 - lo, 1.5 * e
      printf "CLASS_THRESHOLDS: indeterminate s_b >= %.4f (min(E/2, 0.05)); confirmed max I <= %.4f (1 - E/2)\n", mn(e / 2, 0.05), 1 - e / 2
      if (haveW) {
        bpw_lo = 1e18; bpw_hi = -1e18
        for (i = 1; i <= 4; i++) {
          if (!num(D[C[i]]) || D[C[i]] + 0 <= 0) { bpw[C[i]] = "n/a"; continue }
          bpw[C[i]] = R[C[i]] / (W[C[i]] / D[C[i]])
          printf "BYTES_PER_WRITE_%s=%.1f\n", C[i], bpw[C[i]]
        }
        bpwok = 1
        for (i = 1; i <= 4; i++) if (bpw[C[i]] == "n/a") bpwok = 0
        if (bpwok) {
          for (i = 1; i <= 2; i++) for (j = 1; j <= 2; j++) {
            q = bpw[A[i]] / bpw[B[j]]; if (q < bpw_lo) bpw_lo = q; if (q > bpw_hi) bpw_hi = q
          }
          bpws = sprintf("b1=%.1f b2=%.1f a1=%.1f a2=%.1f a/b=[%.4f,%.4f]", bpw["b1"], bpw["b2"], bpw["a1"], bpw["a2"], bpw_lo, bpw_hi)
        } else bpws = "n/a"
      } else bpws = "n/a"
    } else print "READINGS=n/a"
    # STOP precedence D > O > S > V > R; D/O/S come from the manifest.
    stop = "none"
    if (shares != "none") stop = shares
    else if (vfail != "") stop = "V"
    else if (stopR) stop = "R"
    print "== flags =="
    print "STOP=" stop ((stop == "V") ? " (" substr(vfail, 2) ")" : "")
    if (stop != "none") cls = "WITHHELD"
    else if (sb >= mn(e / 2, 0.05)) cls = "INDETERMINATE"
    else if (hi <= 1 - e / 2) cls = "CONFIRMED"
    else cls = "NOT_MET"
    print "VERDICT_BYTES=" cls
    print "VERDICT_LIVE=DESCRIPTIVE"
    if (have) {
      print "LIVE_SIGN=" sign
      printf "I=[%.4f,%.4f]\n", lo, hi
      printf "S_B=%.4f\n", sb
      print "E=" e
      printf "LIVE_I=[%.4f,%.4f]\n", llo, lhi
      printf "S_B_LIVE=%.4f\n", lsb
      print "EXCESS=" excess
      print "BYTES_PER_WRITE=" bpws
    } else {
      print "LIVE_SIGN=n/a"; print "I=n/a"; print "S_B=n/a"; print "E=" e; print "LIVE_I=n/a"
      print "S_B_LIVE=n/a"; print "EXCESS=n/a"; print "BYTES_PER_WRITE=n/a"
    }
    print "WRITE_PARITY=" parity
  }' "$READ")" || { echo "FATAL: the verdict step failed; no flags printed (kept ${READ})" >&2; exit 4; }
printf '%s\n' "$OUT_FLAGS"
rm -f "$READ"
