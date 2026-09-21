#!/usr/bin/env bash
# Per-cell STOP predicates and recorded readings for one spec372 allocator
# cell. Derived from spec371-predicates.sh, which is NOT edited. Writes, each
# truncated first:
#   <BASE>.predicates.txt -- every NAME=VALUE line spec372-k.awk and
#                            spec372-decide.awk consume
#   <BASE>.fits.txt       -- the raw frozen-fitter records (provenance only)
#   <BASE>.amp.txt        -- the census-joined amplification table
#   <BASE>.ampfp.csv      -- elapsed_secs,amp_fp over the USED census points,
#                            the series the TREND fit reads
#
# Departures from the parent, all forced by this carve's cells:
#   - flavours SYS|JE|MI|CA (a2j's is JE|MI, supplied as SPEC372_A2J_FLAVOUR);
#     P5/P6/P7 are STOP predicates on EVERY cell (no dhat cells here);
#   - PE reads N = D/cadence + 1 rows from matrix.txt and needs >= N - 1 of
#     them to carry a footprint, so the same rule holds at 900 s and at 4 h;
#   - PA counts probe lines: >= ceil(D/30) - 2 well-formed je_probe lines on a
#     JE cell (every field numeric), alloc_probe lines carrying bytes_alloc on
#     the CA cell; n/a on SYS and MI by declaration;
#   - the AMP reading divides by the live-OR-set estimator
#     reachable_est = A0 + B_live x live (constants below) instead of by
#     live_tag_bytes, joins census instants to CSV rows (+-30 s; TERMINAL to
#     the last row with a footprint) and DROPS a point whose join lag exceeds
#     90 s, printing it as dropped;
#   - S = phys_footprint + reclaimable is fitted beside the footprint, and the
#     TREND fits (last half, last third, and on JE the native FP/allocated
#     series) run the frozen spec349c2-fit.awk over derived two-column streams.
# No `set -e`: a FALSE predicate must not stop the remaining blocks from running.
set -uo pipefail
# Numeric parsing below must not follow a comma-decimal shell locale: under
# one, awk reads 144.110 as 144 and the fits silently move.
export LC_ALL=C

EV="${1:-}"; BASE="${2:-}"; BUILDS="${3:-}"
if [ "$#" -ne 3 ] || [ ! -d "$EV" ] || [ -z "$BASE" ]; then
  echo "usage: spec372-predicates.sh <EV_DIR> <BASE> <BUILDS_FILE>" >&2; exit 2
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
FIT="$SCRIPT_DIR/spec349c2-fit.awk"
P5AWK="$SCRIPT_DIR/spec366-p5.awk"
P67AWK="$SCRIPT_DIR/spec366-p67.awk"
CELL="${BASE#spec372-}"
CSV="$EV/$BASE.csv"
CONSOLE="$EV/$BASE.harness-console.log"
MATRIX="$EV/$BASE.matrix.txt"
RUNNER="$EV/$BASE.runner-console.log"
PRED="$EV/$BASE.predicates.txt"
FITS="$EV/$BASE.fits.txt"
AMP="$EV/$BASE.amp.txt"
AMPFP="$EV/$BASE.ampfp.csv"
: > "$PRED"; : > "$FITS"; : > "$AMP"; : > "$AMPFP"

# The estimator's two constants, frozen at M from SPEC-371's committed
# count-alloc cells: A0 = mean alloc_live_mb at t = 60 over c0/c1/c2 (MiB),
# B_live = mean bytes per live OR entry over the same cells.
A0_MIB=16.64
B_LIVE=886

case "$CELL" in
  s1a|s1b|s2)  FLAVOUR=SYS ;;
  j1a|j1b|j2)  FLAVOUR=JE  ;;
  m1a|m1b|m2)  FLAVOUR=MI  ;;
  k1)          FLAVOUR=CA  ;;
  a2j)
    case "${SPEC372_A2J_FLAVOUR:-}" in
      JE|MI) FLAVOUR="$SPEC372_A2J_FLAVOUR" ;;
      *) FLAVOUR="UNKNOWN" ;;
    esac ;;
  *) echo "unknown cell '$CELL'" >&2; exit 2 ;;
esac
EXPECT_JOURNAL=true; [ "$CELL" = "a2j" ] && EXPECT_JOURNAL=false

# Leading integer of a matrix line ("duration: 120s  <-- SMOKE OVERRIDE" -> 120).
matrix_int() { awk -v k="$1" 'index($0, k) { s = substr($0, index($0, k) + length(k)); if (match(s, /[0-9]+/)) { print substr(s, RSTART, RLENGTH); exit } }' "$MATRIX" 2>/dev/null; }
DURATION="$(matrix_int 'duration:')"
CADENCE="$(matrix_int 'csv cadence:')"

{
  echo "== STOP: PV =="
  if [ "$FLAVOUR" = "UNKNOWN" ]; then
    echo "PV=FALSE reason=a2j_flavour_not_supplied"
  elif [ ! -s "$CONSOLE" ] || [ ! -s "$BUILDS" ]; then
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
        # awk has no portable {64} interval, so the 64-hex width is checked with
        # length() on the extracted fields instead.
        re = "^provenance: server sha256=[0-9a-f]+ flavour=(SYS|JE|MI|CA) built=[^ ]+ run_start=[^ ]+ topgun_or_prune_restored_cancelled_total=present harness sha256=[0-9a-f]+ harness_built=[^ ]+ tombstone_level_ceiling_gate=present$"
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

  echo "== STOP: P5 / P6 / P7 =="
  awk -f "$P5AWK" "$CONSOLE" 2>&1
  awk -f "$P67AWK" -v scrapes_dir="$EV/$BASE.scrapes" "$CONSOLE" 2>&1

  echo "== STOP: PE / PA =="
  if [ -z "$DURATION" ] || [ -z "$CADENCE" ] || [ ! -s "$CSV" ]; then
    echo "PE=FALSE reason=no_matrix_or_csv"
  else
    awk -F, -v dur="$DURATION" -v cad="$CADENCE" '
      NR == 1 { for (i = 1; i <= NF; i++) col[$i] = i
                if (!("phys_footprint_mb" in col)) { print "PE=FALSE reason=missing_csv_column"; bad = 1; exit } next }
      { rows++; if ($col["phys_footprint_mb"] != "") ev++ }
      END {
        if (bad) exit
        N = int(dur / cad) + 1; need = N - 1
        printf "PE=%s rows_with_footprint=%d rows=%d N=%d need=%d\n", (ev >= need ? "TRUE" : "FALSE"), ev + 0, rows + 0, N, need
      }' "$CSV"
  fi
  if [ "$FLAVOUR" = "JE" ] || [ "$FLAVOUR" = "CA" ]; then
    if [ -z "$DURATION" ] || [ ! -s "$CONSOLE" ]; then
      echo "PA=FALSE reason=no_matrix_or_console"
    else
      awk -v fl="$FLAVOUR" -v dur="$DURATION" '
        function numeric(k) { return (k in f) && f[k] ~ /^[0-9]+$/ }
        (fl == "JE" && /^\[server\] je_probe /) || (fl == "CA" && /^\[server\] alloc_probe /) {
          delete f
          for (i = 3; i <= NF; i++) { split($i, kv, "="); f[kv[1]] = kv[2] }
          seen++
          if (fl == "JE") ok = numeric("elapsed_s") && numeric("allocated") && numeric("active") && numeric("resident") && numeric("retained") && numeric("mapped") && numeric("metadata") && numeric("seq")
          else ok = numeric("elapsed_s") && numeric("bytes_alloc") && numeric("bytes_dealloc")
          if (ok) good++
        }
        END {
          need = int(dur / 30); if (need * 30 < dur) need++; need -= 2
          printf "PA=%s wellformed_lines=%d lines=%d need=%d\n", (good >= need ? "TRUE" : "FALSE"), good + 0, seen + 0, need
        }' "$CONSOLE"
    fi
  else
    echo "PA=n/a reason=flavour_${FLAVOUR}"
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

  echo "== recorded: ops =="
  awk -v f="$EV/$BASE.soak.json" -v p="$EV/$BASE.progress.jsonl" '
    function field(l, k,   v) { if (match(l, "\"" k "\"[ ]*:[ ]*[0-9.]+")) { v = substr(l, RSTART, RLENGTH); sub(/.*: */, "", v); return v } return "" }
    BEGIN {
      while ((getline l < f) > 0) {
        v = field(l, "totalWrites");        if (v != "") tw = v + 0
        v = field(l, "writeErrors");        if (v != "") { we = v + 0; hwe = 1 }
        v = field(l, "durationSecsActual"); if (v != "") ds = v + 0
      }
      if (ds > 0 && tw > 0) printf "OPS_PER_S=%.3f\n", tw / ds; else print "OPS_PER_S=n/a reason=no_totalWrites_or_duration"
      print "WRITE_ERRORS=" (hwe ? we "" : "n/a")
      print "TOTAL_WRITES=" (tw ? tw "" : "n/a") " DURATION_ACTUAL=" (ds ? ds "" : "n/a")
      # The ops rate at the checkpoint nearest t = 900 (within 60 s), so a 4 h
      # cell can be compared with the 900 s journal-off cell at a matched window.
      bd = 61
      while ((getline l < p) > 0) {
        e = field(l, "elapsedSecs"); w = field(l, "totalWrites")
        if (e == "" || w == "") continue
        d = e - 900; if (d < 0) d = -d
        if (d < bd) { bd = d; be = e + 0; bw = w + 0 }
      }
      if (bd <= 60 && be > 0) printf "OPS_AT_900=%.3f checkpoint_elapsed=%d\n", bw / be, be
      else print "OPS_AT_900=n/a reason=no_checkpoint_near_900"
    }' /dev/null

  echo "== recorded: host state before the cell (the chain writes it at the head of the runner console) =="
  awk '/^HOST uptime:/ { l = $0; sub(/.*load averages: /, "", l); split(l, a, " ") }
       /^HOST memory_pressure:/ { m = $NF }
       /^HOST therm:/ && !/No (thermal|performance) warning level has been recorded|No CPU power status has been recorded/ { t = t (t == "" ? "" : ";") substr($0, 14) }
       END { if (a[1] == "") print "HOST=n/a reason=no_host_record"
             else printf "HOST=load_1m=%s load_5m=%s load_15m=%s memory_free=%s therm=%s\n", a[1], a[2], a[3], m, (t == "" ? "no_warning_recorded" : t) }' "$RUNNER" 2>/dev/null || echo "HOST=n/a reason=no_runner_console"

  if [ "$FLAVOUR" = "JE" ]; then
    echo "== recorded: je_config =="
    awk '/^\[server\] je_config / { l = $0; sub(/^\[server\] /, "", l) } END { print "JE_CONFIG=" (l == "" ? "n/a reason=no_je_config_line" : l) }' "$CONSOLE"
  fi
} >> "$PRED"

# ---------------------------------------------------------------- fits
fit_stream() {   # $1 = column, $2 = window; reads a CSV on stdin
  local out rc
  out="$(awk -f "$FIT" -v col="$1" -v window="$2" - 2>&1)"; rc=$?
  if [ "$rc" -ne 0 ]; then printf 'FIT_ERROR rc=%s\n' "$rc"; else printf '%s\n' "$out"; fi
}
# S = phys_footprint + reclaimable, as a derived stream over the rows that carry both.
s_stream() {
  awk -F, 'NR == 1 { for (i = 1; i <= NF; i++) col[$i] = i; print "elapsed_secs,s_mb"; next }
           $col["phys_footprint_mb"] != "" && $col["reclaimable_mb"] != "" {
             printf "%s,%.3f\n", $col["elapsed_secs"], $col["phys_footprint_mb"] + $col["reclaimable_mb"] }' "$CSV"
}
# JE's arm-native amplification on the 60 s row clock: no census join needed.
native_stream() {
  awk -F, 'NR == 1 { for (i = 1; i <= NF; i++) col[$i] = i; print "elapsed_secs,amp_native"; next }
           ("je_allocated" in col) && $col["phys_footprint_mb"] != "" && $col["je_allocated"] + 0 > 0 {
             printf "%s,%.6f\n", $col["elapsed_secs"], $col["phys_footprint_mb"] * 1048576 / $col["je_allocated"] }' "$CSV"
}
{
  for w in last_half full; do printf 'phys_footprint_mb %s ' "$w"; fit_stream phys_footprint_mb "$w" < "$CSV"; done
  for w in last_half full; do printf 'reclaimable_mb %s ' "$w"; fit_stream reclaimable_mb "$w" < "$CSV"; done
  for w in last_half full; do printf 's_mb %s ' "$w"; s_stream | fit_stream s_mb "$w"; done
  if [ "$FLAVOUR" = "CA" ]; then
    for w in last_half full; do printf 'alloc_live_mb %s ' "$w"; fit_stream alloc_live_mb "$w" < "$CSV"; done
  fi
  if [ "$FLAVOUR" = "JE" ]; then
    printf 'amp_native last_half '; native_stream | fit_stream amp_native last_half
  fi
} >> "$FITS"

# ---------------------------------------------------------------- AMP
# Census rows are printed in the harness console as, e.g.,
#   "  LIVE_COPY  t=300.0s copy_done=300.2183s ... live=20245 live_tag_bytes=688331 ..."
# t= / copy_done= carry a trailing "s", stripped before any comparison. The
# census clock starts at the harness sampler, the CSV clock at server-ready.
# Series points are LIVE_COPY and TERMINAL instants only; CHECKPOINT rows are
# printed but never enter the series.
awk -v csv="$CSV" -v a0mib="$A0_MIB" -v blive="$B_LIVE" -v ampfp="$AMPFP" '
  BEGIN {
    FS = ","
    while ((getline l < csv) > 0) {
      nf = split(l, f, ",")
      if (!hdr) { for (i = 1; i <= nf; i++) col[f[i]] = i; hdr = 1; continue }
      r++; el[r] = f[col["elapsed_secs"]] + 0; fp[r] = f[col["phys_footprint_mb"]]
      rc[r] = f[col["reclaimable_mb"]]; rd[r] = f[col["redb_mb"]]
      if ("je_allocated" in col) {
        ja[r] = f[col["je_allocated"]]; jc[r] = f[col["je_active"]]
        jr[r] = f[col["je_resident"]]; jm[r] = f[col["je_metadata"]]
      }
    }
    FS = " "
    a0 = a0mib * 1048576
    print "elapsed_secs,amp_fp" > ampfp
  }
  /^  (LIVE_COPY|TERMINAL|CHECKPOINT) +t=/ {
    src = $1; delete g
    for (i = 2; i <= NF; i++) { split($i, kv, "="); g[kv[1]] = kv[2] }
    t = g["t"]; sub(/s$/, "", t); live = g["live"] + 0
    best = 0
    if (src == "TERMINAL") { for (j = r; j >= 1; j--) if (fp[j] != "") { best = j; break } }
    else { bd = 31; for (j = 1; j <= r; j++) if (fp[j] != "") { d = el[j] - t; if (d < 0) d = -d; if (d <= 30 && d < bd) { bd = d; best = j } } }
    line = sprintf("AMP source=%s t=%s live=%d", src, t, live)
    if (best == 0) { print line " status=dropped reason=no_row_within_30s"; next }
    lag = t - el[best]; alag = (lag < 0) ? -lag : lag
    reach = a0 + blive * live
    s_mb = (rc[best] != "") ? fp[best] + rc[best] : ""
    line = line sprintf(" row=%d join_lag_s=%.1f fp_mb=%s s_mb=%s reclaim_mb=%s redb_mb=%s reach_mb=%.3f", el[best], lag, fp[best], (s_mb == "" ? "" : sprintf("%.3f", s_mb)), rc[best], rd[best], reach / 1048576)
    line = line sprintf(" AMP_FP=%.6f", fp[best] * 1048576 / reach)
    line = line ((s_mb != "") ? sprintf(" AMP_S=%.6f", s_mb * 1048576 / reach) : " AMP_S=n/a")
    if (best in ja) line = line sprintf(" je_allocated=%s je_active=%s je_resident=%s je_metadata=%s", ja[best], jc[best], jr[best], jm[best])
    if (src == "TERMINAL" && alag > 90) { print line " status=dropped reason=join_lag_gt_90s"; next }
    if (src == "CHECKPOINT") { print line " status=not_a_series_point"; next }
    print line " status=used"
    printf "%s,%.6f\n", t, fp[best] * 1048576 / reach >> ampfp
  }' "$CONSOLE" > "$AMP"
{
  printf 'amp_fp last_half '; fit_stream amp_fp last_half < "$AMPFP"
  # Last third: rows floor(2n/3)..n-1 of the used series, fitted over their full window.
  printf 'amp_fp last_third '
  awk 'NR == 1 { h = $0; next } { r[++n] = $0 } END { print h; for (i = int(2 * n / 3) + 1; i <= n; i++) print r[i] }' "$AMPFP" | fit_stream amp_fp full
} >> "$FITS"

# ---------------------------------------------------------------- readings
reading() {   # $1 = key prefix, $2 = column, $3 = window label
  awk -v k="$1" -v c="$2" -v w="$3" '$1 == c && $2 == w {
      if ($3 == "FIT_ERROR") { print k "=FIT_ERROR"; exit }
      for (i = 3; i <= NF; i++) { split($i, kv, "="); f[kv[1]] = kv[2] }
      print k "=" f["slope_mb_per_hour"]; print k "_se=" f["se_mb_per_hour"]; print k "_n=" f["n"]
      print k "_r2=" f["r2"]; exit }' "$FITS"
}
{
  echo "== readings: fits (slopes per hour; on amp series the unit is AMP per hour) =="
  reading FP_slope phys_footprint_mb last_half
  reading S_slope s_mb last_half
  reading TREND amp_fp last_half
  reading TREND3 amp_fp last_third
  [ "$FLAVOUR" = "JE" ] && reading TREND_NATIVE amp_native last_half
  awk '/status=dropped/ && /source=(LIVE_COPY|TERMINAL)/ { d++ } /status=used/ { u++ }
       END { print "TREND_dropped=" d + 0; print "TREND_points_used=" u + 0 }' "$AMP"

  echo "== readings: end levels (last row carrying a footprint) =="
  awk -F, '
    NR == 1 { for (i = 1; i <= NF; i++) col[$i] = i; next }
    $col["phys_footprint_mb"] != "" { e = $col["elapsed_secs"]; fp = $col["phys_footprint_mb"]; rc = $col["reclaimable_mb"]; have = 1 }
    END {
      if (!have) { print "FP_end=n/a reason=no_footprint_row"; exit }
      print "END_elapsed=" e
      print "FP_end=" fp
      if (rc != "") { printf "S_end=%.3f\n", fp + rc; print "RECLAIM_end=" rc; if (fp + 0 > 0) printf "RECLAIM_RATIO_end=%.6f\n", rc / fp }
      else { print "S_end=n/a reason=no_reclaimable"; print "RECLAIM_end=n/a" }
    }' "$CSV"

  echo "== readings: census points (TERMINAL, and the deciding point = the last used one) =="
  awk -v a0mib="$A0_MIB" '
    function emit(p, line,   i, n, kv, f, alloc, reach, act, res, meta) {
      n = split(line, tok, " ")
      for (i = 2; i <= n; i++) { split(tok[i], kv, "="); f[kv[1]] = kv[2] }
      reach = f["reach_mb"] * 1048576
      print p "_src=" f["source"]; print p "_t=" f["t"]; print p "_join_lag_s=" f["join_lag_s"]
      print p "_live=" f["live"]; printf "%s_reach_bytes=%.0f\n", p, reach
      print p "_fp_mb=" f["fp_mb"]; print p "_s_mb=" f["s_mb"]; print p "_redb_mb=" f["redb_mb"]
      print p "_AMP_FP=" f["AMP_FP"]; print p "_AMP_S=" f["AMP_S"]
      printf "%s_A0_share=%.6f\n", p, a0mib * 1048576 / reach
      printf "%s_R_redb=%.6f\n", p, f["redb_mb"] * 1048576 / reach
      if (("je_allocated" in f) && f["je_allocated"] + 0 > 0) {
        alloc = f["je_allocated"]; act = f["je_active"]; res = f["je_resident"]; meta = f["je_metadata"]
        print p "_je_allocated=" alloc; print p "_je_metadata=" meta
        printf "%s_AMP_JE=%.6f\n", p, res / alloc
        printf "%s_DIRTY_SHARE=%.6f\n", p, (res - act - meta) / alloc
        printf "%s_FRAG_SHARE=%.6f\n", p, (act - alloc) / alloc
        printf "%s_R_meta=%.6f\n", p, meta / reach
        printf "%s_EST_AGREE=%.6f\n", p, alloc / reach
        printf "%s_UNMODELLED_MB=%.3f\n", p, (alloc - reach) / 1048576
        printf "%s_UNMODELLED_SHARE=%.6f\n", p, (alloc - reach) / reach
      }
    }
    /status=used/ { last = $0; if ($2 == "source=TERMINAL") term = $0 }
    END {
      if (term != "") emit("TERM", term); else print "TERM_src=n/a reason=terminal_dropped_or_absent"
      if (last != "") emit("DECIDE", last); else print "DECIDE_src=n/a reason=no_used_census_point"
    }' "$AMP"
} >> "$PRED"
echo "wrote $PRED $FITS $AMP $AMPFP"
