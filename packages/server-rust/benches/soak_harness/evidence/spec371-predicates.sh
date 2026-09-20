#!/usr/bin/env bash
# Per-cell STOP predicates and recorded readings for one spec371 memory-diagnosis
# cell. Writes three files, each truncated first:
#   <BASE>.predicates.txt -- every NAME=VALUE line spec371-decide.awk consumes
#                            (STOP predicates, fit readings, levels, AMP summary)
#   <BASE>.fits.txt       -- the raw frozen-fitter records (provenance only)
#   <BASE>.amp.txt        -- the census-joined amplification table
# No `set -e`: a FALSE predicate must not stop the remaining blocks from running.
set -uo pipefail
EV="${1:-}"; BASE="${2:-}"; BUILDS="${3:-}"
if [ "$#" -ne 3 ] || [ ! -d "$EV" ] || [ -z "$BASE" ]; then
  echo "usage: spec371-predicates.sh <EV_DIR> <BASE> <BUILDS_FILE>" >&2; exit 2
fi
# Numeric parsing below must not follow a comma-decimal shell locale: under
# one, awk reads 144.110 as 144 and the fits silently move.
export LC_ALL=C

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
FIT="$SCRIPT_DIR/spec349c2-fit.awk"
P5AWK="$SCRIPT_DIR/spec366-p5.awk"
P67AWK="$SCRIPT_DIR/spec366-p67.awk"
CELL="${BASE#spec371-}"
CSV="$EV/$BASE.csv"
CONSOLE="$EV/$BASE.harness-console.log"
MATRIX="$EV/$BASE.matrix.txt"
RUNNER="$EV/$BASE.runner-console.log"
PRED="$EV/$BASE.predicates.txt"
FITS="$EV/$BASE.fits.txt"
AMP="$EV/$BASE.amp.txt"
: > "$PRED"; : > "$FITS"; : > "$AMP"

case "$CELL" in
  r0)          FLAVOUR=R  ;;
  c0|c1|c2)    FLAVOUR=CA ;;
  c3e|c3l)     FLAVOUR=DH ;;
  *) echo "unknown cell '$CELL'" >&2; exit 2 ;;
esac
EXPECT_JOURNAL=true; [ "$CELL" = "c1" ] && EXPECT_JOURNAL=false

# Leading integer of a matrix line ("duration: 120s  <-- SMOKE OVERRIDE" -> 120).
matrix_int() { awk -v k="$1" 'index($0, k) { s = substr($0, index($0, k) + length(k)); if (match(s, /[0-9]+/)) { print substr(s, RSTART, RLENGTH); exit } }' "$MATRIX" 2>/dev/null; }
DURATION="$(matrix_int 'duration:')"
CADENCE="$(matrix_int 'csv cadence:')"

{
  echo "== STOP: PV =="
  if [ ! -s "$CONSOLE" ] || [ ! -s "$BUILDS" ]; then
    echo "PV=FALSE reason=missing_console_or_builds"
  else
    awk -v fl="$FLAVOUR" -v builds="$BUILDS" '
      BEGIN {
        while ((getline l < builds) > 0) {
          n = split(l, f, " "); fv = ""; sh = ""
          for (i = 1; i <= n; i++) { split(f[i], kv, "="); if (kv[1] == "flavour") fv = kv[2]; if (kv[1] == "sha256") sh = kv[2] }
          if (fv != "") want[fv] = sh
        }
      }
      NR == 1 {
        # The R3.6 shape. awk has no portable {64} interval, so the 64-hex width is
        # checked with length() on the extracted fields instead.
        re = "^provenance: server sha256=[0-9a-f]+ flavour=(R|CA|DH) built=[^ ]+ run_start=[^ ]+ topgun_or_prune_restored_cancelled_total=present harness sha256=[0-9a-f]+ harness_built=[^ ]+ tombstone_level_ceiling_gate=present$"
        if ($0 !~ re) { print "PV=FALSE reason=line1_shape"; exit }
        s = $0; sub(/^provenance: server sha256=/, "", s); sub(/ .*/, "", s)
        g = $0; sub(/.* flavour=/, "", g); sub(/ .*/, "", g)
        h = $0; sub(/.* harness sha256=/, "", h); sub(/ .*/, "", h)
        if (length(s) != 64 || length(h) != 64) print "PV=FALSE reason=sha_not_64_hex server=" s " harness=" h
        else if (g != fl) print "PV=FALSE reason=flavour line1=" g " cell=" fl
        else if (s != want[fl]) print "PV=FALSE reason=server_sha line1=" s " builds=" want[fl]
        else if (h != want["H"]) print "PV=FALSE reason=harness_sha line1=" h " builds=" want["H"]
        else print "PV=TRUE server_sha256=" s " harness_sha256=" h " flavour=" g
        exit
      }' "$CONSOLE"
  fi

  echo "== STOP: PR =="
  cr="$(jq -r '.crashes // "null"' "$EV/$BASE.soak.json" 2>/dev/null || echo null)"
  if [ "$cr" = "0" ]; then echo "PR-crashes=TRUE crashes=0"; else echo "PR-crashes=FALSE crashes=$cr"; fi
  if [ -f "$RUNNER" ]; then
    awk '/^RESULT: instrument sound; harness exit code [0-9]+\.$/ { c = 1 } /^RESULT: INSTRUMENT DEFECT/ { d = 1 }
         END { print (c && !d) ? "PR-class=TRUE" : "PR-class=FALSE reason=" (d ? "instrument_defect" : "no_sound_result_line") }' "$RUNNER"
  else
    echo "PR-class=FALSE reason=no_runner_console"
  fi

  echo "== mechanism: P5 / P6 / P7 =="
  p5="$(awk -f "$P5AWK" "$CONSOLE" 2>&1)"
  p67="$(awk -f "$P67AWK" -v scrapes_dir="$EV/$BASE.scrapes" "$CONSOLE" 2>&1)"
  if [ "$FLAVOUR" = "DH" ]; then
    # Recorded only on the dhat cells: renamed so they can never enter the STOP set.
    printf '%s\n%s\n' "$p5" "$p67" | sed -E "s/^P([567])=/P\\1_${CELL}=/"
  else
    printf '%s\n%s\n' "$p5" "$p67"
  fi

  echo "== STOP: PE / PA =="
  if [ -z "$DURATION" ] || [ -z "$CADENCE" ] || [ ! -s "$CSV" ]; then
    echo "PE=FALSE reason=no_matrix_or_csv"
    [ "$FLAVOUR" = "CA" ] && echo "PA=FALSE reason=no_matrix_or_csv" || echo "PA=n/a reason=flavour_${FLAVOUR}"
  else
    awk -F, -v fl="$FLAVOUR" -v dur="$DURATION" -v cad="$CADENCE" '
      NR == 1 {
        for (i = 1; i <= NF; i++) col[$i] = i
        # A CSV without the columns this cell needs fails closed, never crashes.
        if (!("phys_footprint_mb" in col) || !("elapsed_secs" in col) || (fl == "CA" && !("alloc_probe_seq" in col))) {
          print "PE=FALSE reason=missing_csv_column"; print (fl == "CA") ? "PA=FALSE reason=missing_csv_column" : "PA=n/a reason=flavour_" fl
          bad = 1; exit
        }
        next
      }
      {
        fp = $col["phys_footprint_mb"]
        ok = (fp != "")
        if (fl == "CA") ok = ok && $col["alloc_live_bytes"] != "" && $col["alloc_live_mb"] != "" && $col["alloc_probe_elapsed_s"] != "" && $col["alloc_probe_seq"] != ""
        if (ok) {
          ev++
          if (fl == "CA") {
            if ($col["alloc_probe_elapsed_s"] + 0 < $col["elapsed_secs"] - 35) stale++
            if (seen && $col["alloc_probe_seq"] + 0 < lastseq) back++
            lastseq = $col["alloc_probe_seq"] + 0; seen = 1
          }
        }
      }
      END {
        if (bad) exit
        E = int(dur / cad); need = int(0.95 * E); if (need < 0.95 * E) need++
        print (ev >= need) ? "PE=TRUE evaluable=" ev " need=" need " E=" E : "PE=FALSE evaluable=" ev " need=" need " E=" E
        if (fl != "CA") { print "PA=n/a reason=flavour_" fl; exit }
        minseq = int(dur / 30) - 1
        if (stale + 0 == 0 && back + 0 == 0 && lastseq >= minseq) print "PA=TRUE last_seq=" lastseq " min_seq=" minseq
        else print "PA=FALSE stale=" stale + 0 " backwards=" back + 0 " last_seq=" lastseq + 0 " min_seq=" minseq
      }' "$CSV"
  fi

  echo "== STOP: PJ =="
  awk -v want="$EXPECT_JOURNAL" '
    /soak: child TOPGUN_JOURNAL_ENABLED=/ { n++; v = $0; sub(/.*soak: child TOPGUN_JOURNAL_ENABLED=/, "", v); sub(/[ \r].*/, "", v); if (v != want) bad++ }
    END { if (n == 0) print "PJ=FALSE reason=no_echo_line"; else if (bad) print "PJ=FALSE echoes=" n " mismatched=" bad " want=" want; else print "PJ=TRUE echoes=" n " value=" want }' "$CONSOLE"
  awk -v want="$EXPECT_JOURNAL" '
    /^\[server\].*event journal initialized/ { n++; v = $0; if (match(v, /journal_enabled=[a-z]+/)) { v = substr(v, RSTART + 16, RLENGTH - 16); if (v != want) bad++ } }
    END { if (n == 0) print "PJ-child=n/a reason=filtered"; else print (bad ? "PJ-child=FALSE" : "PJ-child=TRUE") " lines=" n }' "$CONSOLE"

  echo "== STOP: PC =="
  awk '/^  TERMINAL +t=/ { for (i = 1; i <= NF; i++) { split($i, kv, "="); f[kv[1]] = kv[2] } seen = 1 }
       END { if (!seen) print "PC=FALSE reason=no_terminal_census"
             else if (f["live"] + 0 > 0 && f["live_tag_bytes"] + 0 > 0) print "PC=TRUE live=" f["live"] " live_tag_bytes=" f["live_tag_bytes"]
             else print "PC=FALSE live=" f["live"] " live_tag_bytes=" f["live_tag_bytes"] }' "$CONSOLE"

  echo "== STOP: PM1 =="
  PM_ROWS="$(awk -F= '/^post_mortem_rows=/ { v = $2 } END { print (v == "" ? "NA" : v) }' "$RUNNER" 2>/dev/null || echo NA)"
  if [ "$PM_ROWS" = "NA" ] || [ -z "$DURATION" ] || [ -z "$CADENCE" ] || [ ! -s "$CSV" ]; then
    echo "PM1=FALSE reason=no_counter_or_csv post_mortem_rows=${PM_ROWS}"
  else
    awk -F, -v pm="$PM_ROWS" -v dur="$DURATION" -v cad="$CADENCE" '
      NR == 1 { for (i = 1; i <= NF; i++) col[$i] = i; if (!("tombstone_bytes" in col)) { print "PM1=FALSE reason=missing_csv_column"; bad = 1; exit } next }
      {
        e = $col["elapsed_secs"] + 0
        # A row whose scrape produced nothing has an empty tombstone_bytes; the
        # post-mortem counter says how many of those were taken after the server
        # was gone. Both must be late, and a live scrape must exist near the end.
        if ($col["tombstone_bytes"] == "") { empt++; if (!havemin || e < minempty) { minempty = e; havemin = 1 } }
        else lastlive = e
      }
      END {
        if (bad) exit
        need = dur - 2 * cad
        ok = (pm + 0 <= 1) && (pm + 0 == 0 || (havemin && minempty >= dur)) && (lastlive + 0 >= need)
        printf "PM1=%s post_mortem_rows=%s empty_scrape_rows=%d first_empty_elapsed=%s last_live_scrape_elapsed=%d need_ge=%d\n",
               (ok ? "TRUE" : "FALSE"), pm, empt + 0, (havemin ? minempty "" : "none"), lastlive + 0, need
      }' "$CSV"
  fi

  echo "== recorded: ops parity =="
  awk -v f="$EV/$BASE.soak.json" '
    BEGIN {
      while ((getline l < f) > 0) {
        if (match(l, /"totalWrites"[ ]*:[ ]*[0-9]+/)) { v = substr(l, RSTART, RLENGTH); sub(/.*: */, "", v); tw = v + 0 }
        if (match(l, /"writeErrors"[ ]*:[ ]*[0-9]+/)) { v = substr(l, RSTART, RLENGTH); sub(/.*: */, "", v); we = v + 0; hwe = 1 }
        if (match(l, /"durationSecsActual"[ ]*:[ ]*[0-9.]+/)) { v = substr(l, RSTART, RLENGTH); sub(/.*: */, "", v); ds = v + 0 }
      }
      if (ds > 0 && tw > 0) printf "OPS_PER_S=%.3f\n", tw / ds; else print "OPS_PER_S=n/a reason=no_totalWrites_or_duration"
      print "WRITE_ERRORS=" (hwe ? we "" : "n/a")
      print "TOTAL_WRITES=" (tw ? tw "" : "n/a") " DURATION_ACTUAL=" (ds ? ds "" : "n/a")
    }' /dev/null

  if [ "$CELL" = "c3l" ]; then
    echo "== STOP: PD (c3l, over both dhat cells) =="
    DIFF="$EV/spec371-dhat-diff.txt"
    if [ ! -s "$DIFF" ]; then
      echo "PD=FALSE reason=no_diff_file"; echo "PD-crate=FALSE reason=no_diff_file"
    else
      grep -E '^(PD-format|PD-sym|PD-crate)=' "$DIFF"
      awk '/^PD-format=TRUE/ { a = 1 } /^PD-sym=TRUE/ { b = 1 } /^PD-crate=TRUE/ { c = 1 }
           END { print (a && b && c) ? "PD=TRUE" : "PD=FALSE reason=conjunct format=" a + 0 " sym=" b + 0 " crate=" c + 0 }' "$DIFF"
    fi
  fi
} >> "$PRED"

# ---------------------------------------------------------------- fits
fit_line() {   # $1 = column, $2 = window
  local out rc
  out="$(awk -f "$FIT" -v col="$1" -v window="$2" "$CSV" 2>&1)"; rc=$?
  if [ "$rc" -ne 0 ]; then printf 'FIT_ERROR rc=%s\n' "$rc"; else printf '%s\n' "$out"; fi
}
cols="phys_footprint_mb rss_mb reclaimable_mb"
[ "$FLAVOUR" = "CA" ] && cols="$cols alloc_live_mb"
for c in $cols; do
  for w in last_half full; do printf '%s %s ' "$c" "$w"; fit_line "$c" "$w"; done
done >> "$FITS"
# The two fits decide.awk reads, reduced to NAME=VALUE lines.
reading() {   # $1 = key prefix, $2 = column
  awk -v k="$1" -v c="$2" '$1 == c && $2 == "last_half" {
      if ($3 == "FIT_ERROR") { print k "=FIT_ERROR"; exit }
      for (i = 3; i <= NF; i++) { split($i, kv, "="); f[kv[1]] = kv[2] }
      print k "=" f["slope_mb_per_hour"]; print k "_se=" f["se_mb_per_hour"]; print k "_n=" f["n"]; exit }' "$FITS"
}
{
  echo "== readings =="
  reading G phys_footprint_mb
  [ "$FLAVOUR" = "CA" ] && reading L alloc_live_mb
  awk -F, -v fl="$FLAVOUR" '
    NR == 1 { for (i = 1; i <= NF; i++) col[$i] = i; if (!("phys_footprint_mb" in col)) { print "FP_end=n/a reason=missing_csv_column"; bad = 1; exit } next }
    { n++; fp[n] = $col["phys_footprint_mb"]; lv[n] = ("alloc_live_mb" in col) ? $col["alloc_live_mb"] : "" }
    END {
      if (bad) exit
      print "FP_end=" fp[n]
      if (fl != "CA") exit
      print "LIVE_end=" lv[n]
      if (fp[n] != "" && lv[n] != "") printf "RET_end=%.3f\n", fp[n] - lv[n]
      for (i = int(n / 2) + 1; i <= n; i++) if (lv[i] != "") { s += lv[i]; m++ }   # rows floor(n/2)..n-1, 0-based
      if (m) printf "LIVE_LH_mean=%.3f\n", s / m
    }' "$CSV"
} >> "$PRED"

# ---------------------------------------------------------------- AMP
# Census rows are printed in the harness console as, e.g.,
#   "  LIVE_COPY  t=300.0s copy_done=300.2183s ... live=20245 live_tag_bytes=688331 ..."
# The source field is padded to 10 columns and t= / copy_done= carry a trailing
# "s", which is stripped before any comparison. The census clock starts at the
# harness sampler, the CSV clock at server-ready; the +-30 s join absorbs the gap.
awk -v csv="$CSV" '
  BEGIN {
    FS = ","
    while ((getline l < csv) > 0) {
      nf = split(l, f, ",")
      if (!hdr) { for (i = 1; i <= nf; i++) col[f[i]] = i; hdr = 1; continue }
      r++; el[r] = f[col["elapsed_secs"]] + 0; fp[r] = f[col["phys_footprint_mb"]]
      rd[r] = f[col["redb_mb"]]; al[r] = ("alloc_live_bytes" in col) ? f[col["alloc_live_bytes"]] : ""
    }
    FS = " "
  }
  /^  (LIVE_COPY|TERMINAL|CHECKPOINT) +t=/ {
    src = $1; delete g
    for (i = 2; i <= NF; i++) { split($i, kv, "="); g[kv[1]] = kv[2] }
    t = g["t"]; sub(/s$/, "", t); cd = g["copy_done"]; sub(/s$/, "", cd)
    best = 0
    # TERMINAL joins to the LAST row that actually has a footprint: the final
    # row of a run can be sampled while the server is being torn down, and its
    # footprint fields are then empty.
    if (src == "TERMINAL") { for (j = r; j >= 1; j--) if (fp[j] != "") { best = j; break } }
    else { bd = 31; for (j = 1; j <= r; j++) { d = el[j] - t; if (d < 0) d = -d; if (d <= 30 && d < bd) { bd = d; best = j } } }
    ltb = g["live_tag_bytes"] + 0; live = g["live"] + 0
    line = sprintf("AMP source=%s t=%s copy_done=%s live=%d live_tag_bytes=%d", src, t, cd, live, ltb)
    if (best == 0 || fp[best] == "") { print line " row=none"; next }
    fpb = fp[best] * 1048576
    line = line sprintf(" row=%d join_lag_s=%.1f fp_mb=%s", el[best], t - el[best], fp[best])
    line = line ((ltb > 0) ? sprintf(" AMP_fp=%.1f", fpb / ltb) : " AMP_fp=n/a")
    line = line ((rd[best] + 0 > 0) ? sprintf(" AMP_redb=%.2f", fp[best] / rd[best]) : " AMP_redb=n/a")
    line = line ((al[best] != "" && ltb > 0) ? sprintf(" AMP_live=%.1f", al[best] / ltb) : " AMP_live=n/a")
    line = line ((live > 0) ? sprintf(" B_per_entry=%.0f", fpb / live) : " B_per_entry=n/a")
    line = line ((cd != "n/a" && cd != "") ? sprintf(" copy_smear_s=%.4f", cd - t) : " copy_smear_s=n/a")
    print line
  }' "$CONSOLE" > "$AMP"
{
  echo "== AMP summary =="
  awk '/^AMP source=TERMINAL / { for (i = 2; i <= NF; i++) { split($i, kv, "="); f[kv[1]] = kv[2] } term = 1 }
       /^AMP source=LIVE_COPY / && !first { for (i = 2; i <= NF; i++) { split($i, kv, "="); if (kv[1] == "AMP_fp") fa = kv[2] } first = 1 }
       END {
         if (!term) { print "AMP_fp_terminal=n/a reason=no_terminal_line"; exit }
         print "AMP_fp_terminal=" (("AMP_fp" in f) ? f["AMP_fp"] : "n/a")
         print "AMP_redb_terminal=" (("AMP_redb" in f) ? f["AMP_redb"] : "n/a")
         print "AMP_live_terminal=" (("AMP_live" in f) ? f["AMP_live"] : "n/a")
         print "B_per_entry_terminal=" (("B_per_entry" in f) ? f["B_per_entry"] : "n/a")
         if (fa != "" && fa != "n/a" && ("AMP_fp" in f) && f["AMP_fp"] != "n/a") printf "AMP_trend=%.3f\n", f["AMP_fp"] / fa
         else print "AMP_trend=n/a"
       }' "$AMP"
} >> "$PRED"
echo "wrote $PRED $FITS $AMP"
