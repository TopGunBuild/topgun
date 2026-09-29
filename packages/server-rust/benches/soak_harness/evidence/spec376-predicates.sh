#!/usr/bin/env bash
#
# Linux per-cell predicates and recorded readings (topgun-bench) -- a copy of
# spec372-predicates.sh, which is NOT edited.
#
# A COPY EXISTS BECAUSE THE PARENT CANNOT READ A LINUX CELL HONESTLY. Its PE
# gate, its S/FP readings, its TREND fits and its census AMP all key on the
# macOS phys_footprint_mb / reclaimable_mb columns, which a Linux CSV does not
# carry (the Linux memory columns are fp_equiv_mb = Anonymous - LazyFree + Swap,
# lazyfree_mb, hwm_rss_mb, ...). Pointed at a Linux CSV the parent fails
# closed with missing_csv_column; renaming the Linux columns to the macOS names
# would let a Linux number be cited as an M1 footprint. The difference list
# against spec372-predicates.sh is CLOSED at exactly nine items:
#
#   1. CELLS AND FLAVOURS. The cells are the Linux series' own (sc spb sdh sje
#      ssy smi c1 c2 pb pa; base name spec376-<cell>), flavours CA|DH|JE|SYS|MI;
#      the journal is expected enabled on every cell.
#   2. PV BY BUILD LABEL. Three CA binaries (cal, pin, freeze) share one
#      flavour, so a flavour-keyed builds lookup would collide. PV maps the cell
#      to its build label (sc/c1/c2 CA-cal, spb/pb CA-pin, pa CA-frz, sdh
#      DH-pin, sje JE-cal, ssy SYS-cal, smi MI-cal; harness H), requires that
#      label's builds line EXACTLY ONCE, accepts flavour=DH on console line 1,
#      and compares the line-1 flavour with the label's prefix (CA of CA-cal).
#   3. PEL = the parent's PE/PA block with exactly three substitutions: column
#      phys_footprint_mb -> fp_equiv_mb, label PE= -> PEL= (in the awk program
#      AND in the no_matrix_or_csv line outside it), detail
#      rows_with_footprint= -> rows_with_fp_equiv=. PA is unchanged.
#   4. PM1 IS EXECUTED FROM THE FROZEN spec371-predicates.sh (lines 154-169,
#      sha256 asserted, the extraction must end at its closing brace), the
#      method spec373b-verdict.sh uses, instead of a second copy of the text.
#      The post_mortem_rows= counter must occur exactly once and be a plain
#      count; anything else is PM1=FALSE with the reason named.
#   5. PMEM (new): TRUE iff the runner console exists, carries exactly one
#      mem_invariant_violations= line whose value is 0, and no SAMPLER FATAL
#      line. Every other case is PMEM=FALSE reason=<...>, never a pass.
#   6. LINUX READINGS UNDER LINUX NAMES. FPL_slope (fp_equiv_mb), SL_slope
#      (SL = fp_equiv_mb + lazyfree_mb), LAZY_slope; FPL_end, SL_end, LAZY_end,
#      LAZY_RATIO_end, HWM_end, ANON_HUGE_end; TRENDL/TRENDL3 over amp_fpl and
#      TRENDL_NATIVE (JE); census <P>_AMP_FPL/_AMP_SL/_AMP_JEL/_DIRTY_SHAREL/
#      _FRAG_SHAREL. No macOS memory key (PE, FP_*, S_*, RECLAIM_*, *_AMP_FP,
#      *_AMP_S, *_AMP_JE, *_fp_mb, *_s_mb, *_DIRTY_SHARE, *_FRAG_SHARE,
#      *_A0_share, TREND*) is emitted. The fitter is the frozen
#      spec349c2-fit.awk, sha256 asserted; on a mismatch every fit reads
#      FIT_ERROR and the exit status is non-zero.
#   7. NO FROZEN ESTIMATOR. A0 and B_live come from SPEC376_A0_MIB /
#      SPEC376_B_LIVE (both numeric, or neither is used): the parent's
#      constants were derived on the M1 and are no threshold on Linux. Without
#      them every AMP_*L reading and every reading that divides by the
#      reachable estimate prints n/a reason=no_linux_estimator.
#   8. PORTABILITY OF THE RECORDED LINES. PR-crashes reads soak.json with sed
#      (jq is not on the bench image); HOST reads the cell matrix's
#      /proc/loadavg line (the parent parsed BSD uptime lines the Linux chain
#      does not write).
#   9. OUTPUT ON STDOUT. The predicates block is printed on stdout and the
#      caller writes it to <BASE>.predicates.txt (the chain redirects stdout
#      and stderr there); the parent wrote that file itself, which would race
#      the caller's redirection of the same path. The fits, the census table
#      and the amp_fpl series are still files in EV_DIR, and the exit status is
#      non-zero when a frozen source fails its sha256 or a program step fails.
#
# A DIFF HUNK THAT MAPS TO NONE OF THE NINE ITEMS IS A DEFECT; the manifest
# carries the hunk-to-item map (diff spec372-predicates.sh spec376-predicates.sh).
#
# usage: spec376-predicates.sh <EV_DIR> <BASE> <BUILDS_FILE>
#   reads <BASE>.{csv,harness-console.log,matrix.txt,runner-console.log,
#   soak.json,progress.jsonl,scrapes/} from EV_DIR; prints the predicates block
#   on stdout; writes <BASE>.{fits.txt,amp.txt,ampfpl.csv} into EV_DIR.
# Exit status: 0 = every block ran; 2 = usage or unknown cell; 3 = a frozen
# source failed its sha256 (the dependent lines read FALSE / FIT_ERROR);
# 4 = a program step failed.
#
# ---------------------------------------------------------------------------
# Parent header, kept verbatim:
#
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
  echo "usage: spec376-predicates.sh <EV_DIR> <BASE> <BUILDS_FILE>" >&2; exit 2
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
FIT="$SCRIPT_DIR/spec349c2-fit.awk"
P5AWK="$SCRIPT_DIR/spec366-p5.awk"
P67AWK="$SCRIPT_DIR/spec366-p67.awk"
PRED371="$SCRIPT_DIR/spec371-predicates.sh"
FIT_SHA=840813461e3b1bd5c3a79291044d8ac515e09b94333ee530cd6a10de8fa0436f
PRED371_SHA=7d2ca6214beff1c4c0042879823172a45452ef99c06ca49521d5c96889a61d1b
CELL="${BASE#spec376-}"
CSV="$EV/$BASE.csv"
CONSOLE="$EV/$BASE.harness-console.log"
MATRIX="$EV/$BASE.matrix.txt"
RUNNER="$EV/$BASE.runner-console.log"
FITS="$EV/$BASE.fits.txt"
AMP="$EV/$BASE.amp.txt"
AMPFPL="$EV/$BASE.ampfpl.csv"
: > "$FITS"; : > "$AMP"; : > "$AMPFPL"
RC=0
step_failed() { RC=4; echo "PROGRAM_STEP_FAILED=$1" >&2; }

# The estimator's two constants are pre-registered per host, never carried
# over from another one; without both, every reading that divides by the
# reachable estimate is n/a rather than a number with a foreign denominator.
isnum() { printf '%s' "$1" | grep -Eq '^[0-9]+(\.[0-9]+)?$'; }
A0_MIB="${SPEC376_A0_MIB:-}"
B_LIVE="${SPEC376_B_LIVE:-}"
if isnum "$A0_MIB" && isnum "$B_LIVE"; then HAVE_EST=1; else HAVE_EST=0; A0_MIB=0; B_LIVE=0; fi

case "$CELL" in
  sc|spb|c1|c2|pb|pa) FLAVOUR=CA ;;
  sdh) FLAVOUR=DH ;;
  sje) FLAVOUR=JE ;;
  ssy) FLAVOUR=SYS ;;
  smi) FLAVOUR=MI ;;
  *) echo "unknown cell '$CELL'" >&2; exit 2 ;;
esac
case "$CELL" in
  sc|c1|c2) LABEL=CA-cal ;;
  spb|pb)   LABEL=CA-pin ;;
  pa)       LABEL=CA-frz ;;
  sdh)      LABEL=DH-pin ;;
  sje)      LABEL=JE-cal ;;
  ssy)      LABEL=SYS-cal ;;
  smi)      LABEL=MI-cal ;;
esac
EXPECT_JOURNAL=true

# Leading integer of a matrix line ("duration: 120s  <-- SMOKE OVERRIDE" -> 120).
matrix_int() { awk -v k="$1" 'index($0, k) { s = substr($0, index($0, k) + length(k)); if (match(s, /[0-9]+/)) { print substr(s, RSTART, RLENGTH); exit } }' "$MATRIX" 2>/dev/null; }
DURATION="$(matrix_int 'duration:')"
CADENCE="$(matrix_int 'csv cadence:')"

# PM1 runs from the frozen spec371 text, not from a copy of it.
PM1_PROG=""
if [ "$(shasum -a 256 "$PRED371" 2>/dev/null | awk '{print $1}')" = "$PRED371_SHA" ]; then
  PM1_PROG="$(sed -n '154,169p' "$PRED371" | sed '$ s/'\'' "\$CSV"$//')"
  case "$PM1_PROG" in *'printf "PM1=%s post_mortem_rows='*) ;; *) PM1_PROG="" ;; esac
  [ "$(printf '%s\n' "$PM1_PROG" | tail -1)" = "      }" ] || PM1_PROG=""
fi
FIT_OK=0
[ "$(shasum -a 256 "$FIT" 2>/dev/null | awk '{print $1}')" = "$FIT_SHA" ] && FIT_OK=1

{
  echo "== STOP: PV =="
  if [ ! -s "$CONSOLE" ] || [ ! -s "$BUILDS" ]; then
    echo "PV=FALSE reason=missing_console_or_builds"
  else
    awk -v lab="$LABEL" -v fl="$FLAVOUR" -v builds="$BUILDS" '
      BEGIN {
        while ((getline l < builds) > 0) {
          n = split(l, f, " "); fv = ""; sh = ""
          for (i = 1; i <= n; i++) { split(f[i], kv, "="); if (kv[1] == "flavour") fv = kv[2]; if (kv[1] == "sha256") sh = kv[2] }
          if (fv != "") { want[fv] = sh; cnt[fv]++ }
        }
      }
      NR == 1 {
        if (cnt[lab] != 1) { print "PV=FALSE reason=builds_label " lab "=" (cnt[lab] + 0 == 0 ? "absent" : "dup"); exit }
        if (cnt["H"] != 1) { print "PV=FALSE reason=builds_label H=" (cnt["H"] + 0 == 0 ? "absent" : "dup"); exit }
        # awk has no portable {64} interval, so the 64-hex width is checked with
        # length() on the extracted fields instead.
        re = "^provenance: server sha256=[0-9a-f]+ flavour=(SYS|JE|MI|CA|DH) built=[^ ]+ run_start=[^ ]+ topgun_or_prune_restored_cancelled_total=present harness sha256=[0-9a-f]+ harness_built=[^ ]+ tombstone_level_ceiling_gate=present$"
        if ($0 !~ re) { print "PV=FALSE reason=line1_shape"; exit }
        s = $0; sub(/^provenance: server sha256=/, "", s); sub(/ .*/, "", s)
        g = $0; sub(/.* flavour=/, "", g); sub(/ .*/, "", g)
        h = $0; sub(/.* harness sha256=/, "", h); sub(/ .*/, "", h)
        if (length(s) != 64 || length(h) != 64) print "PV=FALSE reason=sha_not_64_hex server=" s " harness=" h
        else if (g != fl) print "PV=FALSE reason=flavour line1=" g " cell=" fl
        else if (s != want[lab]) print "PV=FALSE reason=server_sha line1=" s " builds=" want[lab] " label=" lab
        else if (h != want["H"]) print "PV=FALSE reason=harness_sha line1=" h " builds=" want["H"]
        else print "PV=TRUE server_sha256=" s " harness_sha256=" h " flavour=" g " label=" lab
        exit
      }' "$CONSOLE" || step_failed PV
  fi

  echo "== STOP: PR =="
  cr="$(sed -nE 's/.*"crashes": *([0-9]+).*/\1/p' "$EV/$BASE.soak.json" 2>/dev/null | head -1)"
  if [ "$cr" = "0" ]; then echo "PR-crashes=TRUE crashes=0"; else echo "PR-crashes=FALSE crashes=${cr:-null}"; fi
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
    echo "PEL=FALSE reason=no_matrix_or_csv"
  else
    awk -F, -v dur="$DURATION" -v cad="$CADENCE" '
      NR == 1 { for (i = 1; i <= NF; i++) col[$i] = i
                if (!("fp_equiv_mb" in col)) { print "PEL=FALSE reason=missing_csv_column"; bad = 1; exit } next }
      { rows++; if ($col["fp_equiv_mb"] != "") ev++ }
      END {
        if (bad) exit
        N = int(dur / cad) + 1; need = N - 1
        printf "PEL=%s rows_with_fp_equiv=%d rows=%d N=%d need=%d\n", (ev >= need ? "TRUE" : "FALSE"), ev + 0, rows + 0, N, need
      }' "$CSV" || step_failed PEL
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
        }' "$CONSOLE" || step_failed PA
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
  # The counter must be present exactly once and be a plain count before it
  # is forwarded into the frozen program.
  n_pm="$(grep -c '^post_mortem_rows=' "$RUNNER" 2>/dev/null)"
  case "$n_pm" in
    1) PM_ROWS="$(sed -n 's/^post_mortem_rows=//p' "$RUNNER")"
       printf '%s' "$PM_ROWS" | grep -Eq '^[0-9]+$' || PM_ROWS="${PM_ROWS:-empty}" ;;
    ''|0) PM_ROWS=absent ;;
    *) PM_ROWS=dup ;;
  esac
  if [ -z "$PM1_PROG" ]; then
    echo "PM1=FALSE reason=frozen_source spec371-predicates.sh"; RC=3
  elif ! printf '%s' "$PM_ROWS" | grep -Eq '^[0-9]+$' || [ -z "$DURATION" ] || [ -z "$CADENCE" ] || [ ! -s "$CSV" ]; then
    echo "PM1=FALSE reason=no_counter_or_csv post_mortem_rows=${PM_ROWS}"
  else
    awk -F, -v pm="$PM_ROWS" -v dur="$DURATION" -v cad="$CADENCE" "$PM1_PROG" "$CSV" || step_failed PM1
  fi

  echo "== STOP: PMEM =="
  if [ ! -f "$RUNNER" ]; then
    echo "PMEM=FALSE reason=console_missing"
  else
    n_inv="$(grep -c '^mem_invariant_violations=' "$RUNNER" 2>/dev/null)"
    n_fatal="$(grep -c 'SAMPLER FATAL' "$RUNNER" 2>/dev/null)"
    inv="$(sed -n 's/^mem_invariant_violations=//p' "$RUNNER" | head -1)"
    if [ "${n_inv:-0}" -eq 0 ]; then echo "PMEM=FALSE reason=violations=absent sampler_fatal=${n_fatal:-0}"
    elif [ "$n_inv" -gt 1 ]; then echo "PMEM=FALSE reason=violations=dup sampler_fatal=${n_fatal:-0}"
    elif [ "$inv" != "0" ]; then echo "PMEM=FALSE reason=violations=${inv:-empty} sampler_fatal=${n_fatal:-0}"
    elif [ "${n_fatal:-0}" -ne 0 ]; then echo "PMEM=FALSE reason=sampler_fatal=${n_fatal}"
    else echo "PMEM=TRUE mem_invariant_violations=0 sampler_fatal=0"
    fi
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

  echo "== recorded: host state at launch (the cell matrix's /proc/loadavg line) =="
  awk '/^  \/proc\/loadavg:/ { n++; l = $0; sub(/^  \/proc\/loadavg: */, "", l); split(l, a, " ") }
       END { if (n != 1 || a[1] !~ /^[0-9]+(\.[0-9]+)?$/) print "HOST=n/a reason=no_loadavg_line lines=" n + 0
             else printf "HOST=load_1m=%s load_5m=%s load_15m=%s\n", a[1], a[2], a[3] }' "$MATRIX" 2>/dev/null || echo "HOST=n/a reason=no_matrix"

  if [ "$FLAVOUR" = "JE" ]; then
    echo "== recorded: je_config =="
    awk '/^\[server\] je_config / { l = $0; sub(/^\[server\] /, "", l) } END { print "JE_CONFIG=" (l == "" ? "n/a reason=no_je_config_line" : l) }' "$CONSOLE"
  fi
}

# ---------------------------------------------------------------- fits
fit_stream() {   # $1 = column, $2 = window; reads a CSV on stdin
  local out rc
  if [ "$FIT_OK" != "1" ]; then cat > /dev/null; printf 'FIT_ERROR reason=frozen_fit_sha\n'; return; fi
  out="$(awk -f "$FIT" -v col="$1" -v window="$2" - 2>&1)"; rc=$?
  if [ "$rc" -ne 0 ]; then printf 'FIT_ERROR rc=%s\n' "$rc"; else printf '%s\n' "$out"; fi
}
# SL = fp_equiv + lazyfree, as a derived stream over the rows that carry both.
sl_stream() {
  awk -F, 'NR == 1 { for (i = 1; i <= NF; i++) col[$i] = i; print "elapsed_secs,sl_mb"; next }
           $col["fp_equiv_mb"] != "" && $col["lazyfree_mb"] != "" {
             printf "%s,%.3f\n", $col["elapsed_secs"], $col["fp_equiv_mb"] + $col["lazyfree_mb"] }' "$CSV"
}
# JE's arm-native amplification on the 60 s row clock: no census join needed.
native_stream() {
  awk -F, 'NR == 1 { for (i = 1; i <= NF; i++) col[$i] = i; print "elapsed_secs,amp_nativel"; next }
           ("je_allocated" in col) && $col["fp_equiv_mb"] != "" && $col["je_allocated"] + 0 > 0 {
             printf "%s,%.6f\n", $col["elapsed_secs"], $col["fp_equiv_mb"] * 1048576 / $col["je_allocated"] }' "$CSV"
}
[ "$FIT_OK" = "1" ] || RC=3
{
  for w in last_half full; do printf 'fp_equiv_mb %s ' "$w"; fit_stream fp_equiv_mb "$w" < "$CSV"; done
  for w in last_half full; do printf 'lazyfree_mb %s ' "$w"; fit_stream lazyfree_mb "$w" < "$CSV"; done
  for w in last_half full; do printf 'sl_mb %s ' "$w"; sl_stream | fit_stream sl_mb "$w"; done
  if [ "$FLAVOUR" = "CA" ]; then
    for w in last_half full; do printf 'alloc_live_mb %s ' "$w"; fit_stream alloc_live_mb "$w" < "$CSV"; done
  fi
  if [ "$FLAVOUR" = "JE" ]; then
    printf 'amp_nativel last_half '; native_stream | fit_stream amp_nativel last_half
  fi
} >> "$FITS"

# ---------------------------------------------------------------- AMP
# Census rows are printed in the harness console as, e.g.,
#   "  LIVE_COPY  t=300.0s copy_done=300.2183s ... live=20245 live_tag_bytes=688331 ..."
# t= / copy_done= carry a trailing "s", stripped before any comparison. The
# census clock starts at the harness sampler, the CSV clock at server-ready.
# Series points are LIVE_COPY and TERMINAL instants only; CHECKPOINT rows are
# printed but never enter the series. Without an estimator the join is still
# made (the TERM_/DECIDE_ time, live and redb readings need it) but no point
# enters the amp_fpl series.
awk -v csv="$CSV" -v a0mib="$A0_MIB" -v blive="$B_LIVE" -v est="$HAVE_EST" -v ampfpl="$AMPFPL" '
  BEGIN {
    FS = ","
    while ((getline l < csv) > 0) {
      nf = split(l, f, ",")
      if (!hdr) { for (i = 1; i <= nf; i++) col[f[i]] = i; hdr = 1; continue }
      r++; el[r] = f[col["elapsed_secs"]] + 0; fp[r] = f[col["fp_equiv_mb"]]
      lz[r] = f[col["lazyfree_mb"]]; rd[r] = f[col["redb_mb"]]
      if ("je_allocated" in col) {
        ja[r] = f[col["je_allocated"]]; jc[r] = f[col["je_active"]]
        jr[r] = f[col["je_resident"]]; jm[r] = f[col["je_metadata"]]
      }
    }
    FS = " "
    a0 = a0mib * 1048576
    print "elapsed_secs,amp_fpl" > ampfpl
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
    sl_mb = (lz[best] != "") ? fp[best] + lz[best] : ""
    line = line sprintf(" row=%d join_lag_s=%.1f fpl_mb=%s sl_mb=%s lazy_mb=%s redb_mb=%s", el[best], lag, fp[best], (sl_mb == "" ? "" : sprintf("%.3f", sl_mb)), lz[best], rd[best])
    if (est) {
      reach = a0 + blive * live
      line = line sprintf(" reach_mb=%.3f AMP_FPL=%.6f", reach / 1048576, fp[best] * 1048576 / reach)
      line = line ((sl_mb != "") ? sprintf(" AMP_SL=%.6f", sl_mb * 1048576 / reach) : " AMP_SL=n/a")
    } else line = line " reach_mb=n/a AMP_FPL=n/a AMP_SL=n/a"
    if (best in ja) line = line sprintf(" je_allocated=%s je_active=%s je_resident=%s je_metadata=%s", ja[best], jc[best], jr[best], jm[best])
    if (src == "TERMINAL" && alag > 90) { print line " status=dropped reason=join_lag_gt_90s"; next }
    if (src == "CHECKPOINT") { print line " status=not_a_series_point"; next }
    print line " status=used"
    if (est) printf "%s,%.6f\n", t, fp[best] * 1048576 / reach >> ampfpl
  }' "$CONSOLE" > "$AMP" || step_failed AMP
{
  printf 'amp_fpl last_half '; fit_stream amp_fpl last_half < "$AMPFPL"
  # Last third: rows floor(2n/3)..n-1 of the used series, fitted over their full window.
  printf 'amp_fpl last_third '
  awk 'NR == 1 { h = $0; next } { r[++n] = $0 } END { print h; for (i = int(2 * n / 3) + 1; i <= n; i++) print r[i] }' "$AMPFPL" | fit_stream amp_fpl full
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
  reading FPL_slope fp_equiv_mb last_half
  reading SL_slope sl_mb last_half
  reading LAZY_slope lazyfree_mb last_half
  if [ "$HAVE_EST" = "1" ]; then
    reading TRENDL amp_fpl last_half
    reading TRENDL3 amp_fpl last_third
  else
    echo "TRENDL=n/a reason=no_linux_estimator"
    echo "TRENDL3=n/a reason=no_linux_estimator"
  fi
  [ "$FLAVOUR" = "JE" ] && reading TRENDL_NATIVE amp_nativel last_half
  # Points used = rows of the amp_fpl series the TRENDL fits read (none
  # without an estimator), not merely census points that joined a CSV row.
  awk '/status=dropped/ && /source=(LIVE_COPY|TERMINAL)/ { d++ }
       END { print "TRENDL_dropped=" d + 0 }' "$AMP"
  awk 'NR > 1 { u++ } END { print "TRENDL_points_used=" u + 0 }' "$AMPFPL"

  echo "== readings: end levels (last row carrying fp_equiv_mb) =="
  awk -F, '
    NR == 1 { for (i = 1; i <= NF; i++) col[$i] = i; next }
    ("fp_equiv_mb" in col) && $col["fp_equiv_mb"] != "" { e = $col["elapsed_secs"]; fp = $col["fp_equiv_mb"]; lz = $col["lazyfree_mb"]; hw = $col["hwm_rss_mb"]; ah = $col["anon_huge_mb"]; have = 1 }
    END {
      if (!have) { print "FPL_end=n/a reason=no_fp_equiv_row"; exit }
      print "END_elapsed=" e
      print "FPL_end=" fp
      if (lz != "") { printf "SL_end=%.3f\n", fp + lz; print "LAZY_end=" lz; if (fp + 0 > 0) printf "LAZY_RATIO_end=%.6f\n", lz / fp }
      else { print "SL_end=n/a reason=no_lazyfree"; print "LAZY_end=n/a" }
      print "HWM_end=" (hw == "" ? "n/a" : hw)
      print "ANON_HUGE_end=" (ah == "" ? "n/a" : ah)
    }' "$CSV" 2>/dev/null || echo "FPL_end=n/a reason=no_csv"

  echo "== readings: census points (TERMINAL, and the deciding point = the last used one) =="
  awk -v a0mib="$A0_MIB" -v est="$HAVE_EST" '
    function emit(p, line,   i, n, kv, f, alloc, reach, act, res, meta) {
      n = split(line, tok, " ")
      for (i = 2; i <= n; i++) { split(tok[i], kv, "="); f[kv[1]] = kv[2] }
      reach = f["reach_mb"] * 1048576
      print p "_src=" f["source"]; print p "_t=" f["t"]; print p "_join_lag_s=" f["join_lag_s"]
      print p "_live=" f["live"]
      if (est) printf "%s_reach_bytes=%.0f\n", p, reach; else print p "_reach_bytes=n/a reason=no_linux_estimator"
      print p "_redb_mb=" f["redb_mb"]
      if (est) { print p "_AMP_FPL=" f["AMP_FPL"]; print p "_AMP_SL=" f["AMP_SL"]; printf "%s_R_redb=%.6f\n", p, f["redb_mb"] * 1048576 / reach }
      else { print p "_AMP_FPL=n/a reason=no_linux_estimator"; print p "_AMP_SL=n/a reason=no_linux_estimator"; print p "_R_redb=n/a reason=no_linux_estimator" }
      if (("je_allocated" in f) && f["je_allocated"] + 0 > 0) {
        alloc = f["je_allocated"]; act = f["je_active"]; res = f["je_resident"]; meta = f["je_metadata"]
        print p "_je_allocated=" alloc; print p "_je_metadata=" meta
        if (est) printf "%s_AMP_JEL=%.6f\n", p, res / alloc; else print p "_AMP_JEL=n/a reason=no_linux_estimator"
        printf "%s_DIRTY_SHAREL=%.6f\n", p, (res - act - meta) / alloc
        printf "%s_FRAG_SHAREL=%.6f\n", p, (act - alloc) / alloc
        if (est) {
          printf "%s_R_meta=%.6f\n", p, meta / reach
          printf "%s_EST_AGREE=%.6f\n", p, alloc / reach
          printf "%s_UNMODELLED_MB=%.3f\n", p, (alloc - reach) / 1048576
          printf "%s_UNMODELLED_SHARE=%.6f\n", p, (alloc - reach) / reach
        } else {
          print p "_R_meta=n/a reason=no_linux_estimator"; print p "_EST_AGREE=n/a reason=no_linux_estimator"
          print p "_UNMODELLED_MB=n/a reason=no_linux_estimator"; print p "_UNMODELLED_SHARE=n/a reason=no_linux_estimator"
        }
      }
    }
    /status=used/ { last = $0; if ($2 == "source=TERMINAL") term = $0 }
    END {
      if (term != "") emit("TERM", term); else print "TERM_src=n/a reason=terminal_dropped_or_absent"
      if (last != "") emit("DECIDE", last); else print "DECIDE_src=n/a reason=no_used_census_point"
    }' "$AMP" || step_failed CENSUS
}
exit "$RC"
