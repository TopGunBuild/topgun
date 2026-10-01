#!/usr/bin/env bash
#
# Linux allocator-series predicates (topgun-bench) -- a copy of
# spec376-predicates.sh, which is NOT edited.
#
# A COPY EXISTS BECAUSE THE PARENT CANNOT READ THESE CELLS. Its cell and
# label tables name the calibration cells only, it has no allocator proof
# (a treatment is only what the running process proves), and it has no
# per-live-entry series: an absolute resident slope grows by design under a
# growing live set, so the flagged reading here is resident bytes PER LIVE
# ENTRY and its trend, judged by an equivalence test against a bar read from
# the pre-registered manifest. The difference list against
# spec376-predicates.sh is CLOSED at exactly eight items:
#
#   1. CELLS AND LABELS. Smoke ssy sje smi3 smi2, series s1 je mi3 mi2 s2
#      (base name spec377-<cell>); labels SYS-ser | JE-ser | MI3-ser |
#      MI2-ser (harness H); flavours SYS | JE | MI (both mimalloc majors are
#      MI on console line 1). PA is a STOP predicate on JE cells only and is
#      pre-declared PA=n/a reason=no_probe_arm elsewhere.
#   2. SECTION-1 LITERALS. A0_L_MIB, FLAT_BAR_PER_H, LEVEL_WINDOW_S and
#      STAGE2_MAX_H are read from spec377-manifest.md above its APPEND-ONLY
#      marker, each exactly once at column 0 and numeric; never a literal
#      here. A missing or malformed one makes every reading that needs it n/a
#      (named) and the exit status 3.
#   3. PV CAPTURED. PV's line is kept in a variable so PALLOC can require it;
#      its text is unchanged.
#   4. PALLOC AND JE_CONFIRM_CONF. The allocator proof per label, every item a
#      presence check under the absence rule: the runner's ALLOC_ENV_,
#      JE_BG_THREADS_ and BIN_MARKERS_ keys, the builds file's controls and
#      TREE_PROOF_ records, the harness console's je_config line, jemalloc's
#      confirm_conf printout and mimalloc's startup block; a series MI cell
#      also matches the smoke-captured literal file whose sha256 section 1
#      lists. Each unmet item is named by its id.
#   5. THE PER-ENTRY SERIES. Census points (every LIVE_COPY plus TERMINAL,
#      each once) join the nearest live CSV row within 30 s, else they are
#      dropped and named; live(t) is interpolated between retained census
#      points only; pe = (fp_equiv_mb - A0) * 2^20 / live(t) and its rss_mb
#      twin are written as <BASE>.pe.csv / .rsspe.csv, the census series as
#      <BASE>.pe-census.csv; PE_END, PE_LEVEL (mean over the last
#      LEVEL_WINDOW_S before D), RSS_PE_LEVEL, LIVE_END.
#   6. THE TREND. The frozen spec349c2-fit.awk fits pe over its own last-half
#      window (rows int(n/2)..n-1 of pe.csv); the same rows are refitted here
#      for residuals and the two slopes must agree to 5e-7 (the fitter prints
#      six decimals), else TREND=n/a reason=fit_mismatch. The AR(1)-adjusted
#      relative se e classifies TREND two-sidedly (RISING, FALLING, FLAT,
#      UNDERPOWERED, MARGINAL); TREND3 (last third) and the Stage-2 length
#      follow, and FIXED_EST / FIXED_NOTE / TREND_CORR are recorded only.
#   7. RECORDED READINGS. JE-native (AMP_JE, FRAG_SHAREL, DIRTY_SHAREL,
#      REACH_PE) at the last probe row, LAZY_MAX with its anon ratio,
#      HWM_END, ANON_HUGE_END, DISK_SLOPE (wal + redb) and the runner's
#      MI_POSTINIT_LINES count.
#   8. This header, the usage text and the messages that name this program.
#
# A DIFF HUNK THAT MAPS TO NONE OF THE EIGHT ITEMS IS A DEFECT; the manifest
# carries the hunk-to-item map (diff spec376-predicates.sh spec377-predicates.sh).
#
# usage: spec377-predicates.sh <EV_DIR> <BASE> <BUILDS_FILE>
#   as the parent below, plus <BASE>.{pe.csv,rsspe.csv,pe-census.csv,
#   pe-join.csv} written into EV_DIR. SPEC377_SYNTHETIC=1 lets a synthetic
#   run name its own manifest (SPEC377_MANIFEST) and override the fitter's
#   printed last-half slope (SPEC377_TEST_FIT_SLOPE), only when EV_DIR is not
#   the evidence dir; nothing else is overridable.
#
# Everything from here on is spec376-predicates.sh's own header, kept
# verbatim so the lineage stays readable.
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
  echo "usage: spec377-predicates.sh <EV_DIR> <BASE> <BUILDS_FILE>" >&2; exit 2
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
FIT="$SCRIPT_DIR/spec349c2-fit.awk"
P5AWK="$SCRIPT_DIR/spec366-p5.awk"
P67AWK="$SCRIPT_DIR/spec366-p67.awk"
PRED371="$SCRIPT_DIR/spec371-predicates.sh"
FIT_SHA=840813461e3b1bd5c3a79291044d8ac515e09b94333ee530cd6a10de8fa0436f
PRED371_SHA=7d2ca6214beff1c4c0042879823172a45452ef99c06ca49521d5c96889a61d1b
CELL="${BASE#spec377-}"
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
  ssy|s1|s2) FLAVOUR=SYS; LABEL=SYS-ser ;;
  sje|je)    FLAVOUR=JE;  LABEL=JE-ser ;;
  smi3|mi3)  FLAVOUR=MI;  LABEL=MI3-ser ;;
  smi2|mi2)  FLAVOUR=MI;  LABEL=MI2-ser ;;
  *) echo "unknown cell '$CELL'" >&2; exit 2 ;;
esac
case "$CELL" in
  s1|je|mi3|mi2|s2) PHASE=series ;;
  *) PHASE=smoke ;;
esac

# Item 2: the pre-registered literals, from the manifest's frozen section
# only. A synthetic run may name its own manifest, never a run whose inputs
# lie in the evidence dir.
under() { case "$1/" in "$2"/*) return 0 ;; esac; return 1; }
EV_ABS="$(cd "$EV" && pwd -P)"
SYNTH=0
if [ "${SPEC377_SYNTHETIC:-0}" = "1" ] && ! under "$EV_ABS" "$(cd "$SCRIPT_DIR" && pwd -P)"; then SYNTH=1; fi
MANIFEST="$SCRIPT_DIR/spec377-manifest.md"
[ "$SYNTH" = "1" ] && [ -n "${SPEC377_MANIFEST:-}" ] && MANIFEST="$SPEC377_MANIFEST"
SEC1="$( [ -f "$MANIFEST" ] && grep -q '^## APPEND-ONLY BELOW' "$MANIFEST" && sed '/^## APPEND-ONLY BELOW/q' "$MANIFEST")"
LIT_BAD=""
# The first token of the one column-0 KEY= line of section 1, numeric; a
# missing, duplicated or non-numeric literal is named and never defaulted.
lit() {   # $1 = key; sets LIT_<key> or appends to LIT_BAD
  local n v
  n="$(printf '%s\n' "$SEC1" | grep -c "^$1=")"
  case "$n" in
    1) v="$(printf '%s\n' "$SEC1" | sed -n "s/^$1=//p")"; v="${v%% *}"
       if printf '%s' "$v" | grep -Eq '^[0-9]+(\.[0-9]+)?$'; then eval "LIT_$1=\$v"; return; fi
       LIT_BAD="${LIT_BAD:+${LIT_BAD},}$1=${v:-empty}" ;;
    0) LIT_BAD="${LIT_BAD:+${LIT_BAD},}$1=absent" ;;
    *) LIT_BAD="${LIT_BAD:+${LIT_BAD},}$1=dup" ;;
  esac
  eval "LIT_$1="
}
for k in A0_L_MIB FLAT_BAR_PER_H LEVEL_WINDOW_S STAGE2_MAX_H; do lit "$k"; done
if [ -n "$LIT_BAD" ]; then
  echo "FATAL: manifest section-1 literal(s) not usable: ${LIT_BAD} (${MANIFEST})" >&2
fi
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
    PV_LINE="PV=FALSE reason=missing_console_or_builds"
  else
    PV_LINE="$(awk -v lab="$LABEL" -v fl="$FLAVOUR" -v builds="$BUILDS" '
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
      }' "$CONSOLE")" || step_failed PV
  fi
  printf '%s\n' "$PV_LINE"

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
  if [ "$FLAVOUR" = "JE" ]; then
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
    echo "PA=n/a reason=no_probe_arm"
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

  # ---------------------------------------------------------- item 4: PALLOC
  # A treatment is what the running process proves. Every item is a presence
  # check: an absent, duplicated or unparseable input fails the item and is
  # named, never read as a pass.
  echo "== STOP: PALLOC =="
  # A value that happens to read "absent" fails every item exactly as the
  # absence does, so the two need no separate marker.
  kv_once() {   # $1 = file, $2 = key: the value, or absent / dup
    local n
    [ -f "$1" ] || { echo "absent"; return; }
    n="$(grep -c "^$2=" "$1" 2>/dev/null)"
    case "$n" in 1) sed -n "s/^$2=//p" "$1" ;; 0) echo "absent" ;; *) echo "dup" ;; esac
  }
  # Lines of the harness console matching an extended regex (0 when absent;
  # the console's own absence fails PV and every item that needs a line).
  ccount() { local n; n="$(grep -Ec "$1" "$CONSOLE" 2>/dev/null)"; echo "${n:-0}"; }
  isint() { printf '%s' "$1" | grep -Eq '^[0-9]+$'; }
  PALLOC_FAILED=""
  item() {   # $1 = id, $2 = 1 when the item holds, $3 = detail (no spaces)
    if [ "$2" = "1" ]; then echo "PALLOC_ITEM_$1=PASS $3"
    else echo "PALLOC_ITEM_$1=FAIL $3"; PALLOC_FAILED="${PALLOC_FAILED:+${PALLOC_FAILED},}$1"; fi
  }
  # "je=<n> mi=<n>" -> the two counts, or "bad" when the value is not that shape.
  markers() {   # $1 = value; sets MK_JE MK_MI
    MK_JE=bad; MK_MI=bad
    case "$1" in
      "je="*" mi="*) MK_JE="${1#je=}"; MK_JE="${MK_JE%% *}"; MK_MI="${1##* mi=}" ;;
    esac
    isint "$MK_JE" || MK_JE=bad; isint "$MK_MI" || MK_MI=bad
  }
  tree_pass() {   # $1 = label: 1 iff exactly one ^TREE_PROOF_<label>=PASS( |$) line and no other
    local n
    n="$(grep -c "^TREE_PROOF_$1=" "$BUILDS" 2>/dev/null)"
    [ "${n:-0}" = "1" ] && grep -Eq "^TREE_PROOF_$1=PASS( |\$)" "$BUILDS" && echo 1 || echo 0
  }
  tree_seen() { local v; v="$(kv_once "$BUILDS" "TREE_PROOF_$1")"; v="${v%% *}"; echo "${v:-empty}"; }
  AENV="$(kv_once "$RUNNER" "ALLOC_ENV_${CELL}")"
  ABG="$(kv_once "$RUNNER" "JE_BG_THREADS_${CELL}")"
  ABM="$(kv_once "$RUNNER" "BIN_MARKERS_${CELL}")"
  markers "$ABM"
  case "$PV_LINE" in PV=TRUE|"PV=TRUE "*) PV_OK=1 ;; *) PV_OK=0 ;; esac
  # PV is required on every label; its own reason stays on the PV line.
  item pv "$PV_OK" "PV=$(printf '%s' "${PV_LINE#PV=}" | awk '{ print $1 }')"
  case "$FLAVOUR" in
    SYS)
      [ "$AENV" = "none" ] && ok=1 || ok=0; item s1 "$ok" "ALLOC_ENV=${AENV}"
      [ "$MK_JE" = "0" ] && [ "$MK_MI" = "0" ] && ok=1 || ok=0; item s2 "$ok" "BIN_MARKERS=${ABM// /,}"
      [ "$ABG" = "0" ] && ok=1 || ok=0; item s3 "$ok" "JE_BG_THREADS=${ABG}"
      n1="$(ccount '^\[server\] je_(config|probe) ')"; n2="$(ccount 'mimalloc')"
      [ "$n1" = "0" ] && [ "$n2" = "0" ] && ok=1 || ok=0; item s4 "$ok" "je_lines=${n1},mimalloc_lines=${n2}"
      item s5 "$(tree_pass SYS-ser)" "TREE_PROOF_SYS-ser=$(tree_seen SYS-ser)"
      # Past launch: the runner printed its closing lines, and the inherited
      # flavour-marker assertion (SYS = no other flavour's literal) never fired.
      nst="$(grep -c '^steal_pct=' "$RUNNER" 2>/dev/null)"; nfl="$(grep -c 'FATAL: SOAK_SERVER_BINARY is not a' "$RUNNER" 2>/dev/null)"
      [ "${nst:-0}" = "1" ] && [ "${nfl:-0}" = "0" ] && ok=1 || ok=0; item s6 "$ok" "steal_pct_lines=${nst:-0},flavour_fatal=${nfl:-0}"
      # The absence of both literals in the SYS binary means something only if
      # the same count finds them in the binaries that must carry them.
      ctl=""; ok=1
      for cl in JE-ser:je MI3-ser:mi MI2-ser:mi; do
        v="$(kv_once "$BUILDS" "BIN_MARKERS_${cl%%:*}")"; markers "$v"
        if [ "${cl#*:}" = "je" ]; then c="$MK_JE"; else c="$MK_MI"; fi
        isint "$c" && [ "$c" -ge 1 ] || { ok=0; [ "$c" = "bad" ] && c="$v"; }
        ctl="${ctl:+${ctl},}${cl%%:*}.${cl#*:}=${c// /_}"
      done
      item marker_control "$ok" "$ctl" ;;
    JE)
      want='_RJEM_MALLOC_CONF=background_thread:true,confirm_conf:true'
      [ "$AENV" = "$want" ] && ok=1 || ok=0; item j1 "$ok" "ALLOC_ENV=${AENV}"
      jc="$(grep -E '^\[server\] je_config ' "$CONSOLE" 2>/dev/null)"
      njc="$(printf '%s' "$jc" | grep -c .)"
      ok=0
      if [ "$njc" = "1" ]; then
        printf '%s\n' "$jc" | awk '{ for (i = 3; i <= NF; i++) t[$i] = 1 } END { exit !(("opt_background_thread=true" in t) && ("background_thread=true" in t)) }' && ok=1
      fi
      item j2 "$ok" "je_config_lines=${njc}"
      isint "$ABG" && [ "$ABG" -ge 1 ] && ok=1 || ok=0; item j3 "$ok" "JE_BG_THREADS=${ABG}"
      # The confirm_conf printout: sources #1..#5 in order, only #4 (the
      # environment source, which this prefixed build reads from
      # _RJEM_MALLOC_CONF) non-empty and equal to the cell's env, and exactly
      # the two "Set conf value" lines that env implies.
      cr=""
      nsrc="$(ccount '^\[server\] <jemalloc>: malloc_conf #[1-5] \(')"
      seq="$(sed -nE 's/^\[server\] <jemalloc>: malloc_conf #([1-5]) \(.*/\1/p' "$CONSOLE" 2>/dev/null | tr -d '\n')"
      { [ "$nsrc" = "5" ] && [ "$seq" = "12345" ]; } || cr="${cr},sources=n${nsrc}:seq${seq:-none}"
      for k in 1 2 3 4 5; do
        a="$(ccount "^\\[server\\] <jemalloc>: malloc_conf #${k} \\(")"
        if [ "$k" = "4" ]; then w='"background_thread:true,confirm_conf:true"'; why=not_cell_env; else w='""'; why=nonempty; fi
        n="$(ccount "^\\[server\\] <jemalloc>: malloc_conf #${k} \\(.*: ${w} *\$")"
        if [ "$a" = "0" ]; then cr="${cr},source${k}=absent"
        elif [ "$n" != "1" ] || [ "$a" != "1" ]; then cr="${cr},source${k}_${why}"; fi
      done
      n="$(ccount '^\[server\] <jemalloc>: -- Set conf value: background_thread:true *$')"
      [ "$n" = "1" ] || cr="${cr},set_background_thread=${n}"
      n="$(ccount '^\[server\] <jemalloc>: -- Set conf value: confirm_conf:true *$')"
      [ "$n" = "1" ] || cr="${cr},set_confirm_conf=${n}"
      n="$(ccount 'Set conf value')"
      [ "$n" = "2" ] || cr="${cr},set_lines=${n}"
      if [ -z "$cr" ]; then echo "JE_CONFIRM_CONF=PASS"; ok=1; else echo "JE_CONFIRM_CONF=FAIL reason=${cr#,}"; ok=0; fi
      item j4 "$ok" "JE_CONFIRM_CONF=$( [ "$ok" = 1 ] && echo PASS || echo FAIL)"
      [ "$MK_JE" != "bad" ] && [ "$MK_JE" -ge 1 ] && [ "$MK_MI" = "0" ] && ok=1 || ok=0; item j5 "$ok" "BIN_MARKERS=${ABM// /,}"
      item j6 "$(tree_pass JE-ser)" "TREE_PROOF_JE-ser=$(tree_seen JE-ser)" ;;
    MI)
      if [ "$LABEL" = "MI3-ser" ]; then P=''; VER='3\.3\.2'; OTHER=2; PD=1000; PM=1
      else P='mimalloc: '; VER='2\.3\.2'; OTHER=3; PD=10; PM=10; fi
      [ "$AENV" = "MIMALLOC_VERBOSE=1" ] && ok=1 || ok=0; item m1 "$ok" "ALLOC_ENV=${AENV}"
      [ "$ABG" = "0" ] && ok=1 || ok=0; item m2 "$ok" "JE_BG_THREADS=${ABG}"
      [ "$MK_JE" = "0" ] && [ "$MK_MI" != "bad" ] && [ "$MK_MI" -ge 1 ] && ok=1 || ok=0; item m3 "$ok" "BIN_MARKERS=${ABM// /,}"
      item m4 "$(tree_pass "$LABEL")" "TREE_PROOF_${LABEL}=$(tree_seen "$LABEL")"
      # The startup block by the source-derived anchored regexes: v3 prints it
      # unprefixed, v2 through _mi_message with "mimalloc: ".
      mr=""
      n="$(ccount "^\\[server\\] ${P}v${VER}[ ,(]")"; [ "$n" = "1" ] || mr="${mr},version=${n}"
      for opt in "purge_delay:${PD}" "arena_purge_mult:${PM}" "purge_decommits:1"; do
        o="${opt%%:*}"; w="${opt#*:}"
        n="$(ccount "^\\[server\\] ${P}option '${o}': ${w} *\$")"
        a="$(ccount "^\\[server\\] (mimalloc: )?(thread 0x[0-9a-f]+: )?option '${o}': ")"
        { [ "$n" = "1" ] && [ "$a" = "1" ]; } || mr="${mr},${o}=exact${n}/any${a}"
      done
      # A series cell must also reproduce, byte for byte, every line the smoke
      # captured for its major, from the committed file section 1 hashes.
      if [ "$PHASE" = "series" ]; then
        LIT="$(dirname "$MANIFEST")/spec377-mi-literals.txt"
        LREL="packages/server-rust/benches/soak_harness/evidence/spec377-mi-literals.txt"
        lsha="$(printf '%s\n' "$SEC1" | sed -nE "s|^- \`([0-9a-f]{64})\` \`${LREL}\`.*|\\1|p")"
        tag="s${CELL}"
        if [ "$(printf '%s' "$lsha" | grep -c .)" != "1" ]; then mr="${mr},literals_sha=$( [ -z "$lsha" ] && echo absent || echo dup)"
        elif [ ! -f "$LIT" ]; then mr="${mr},literals=absent"
        elif [ "$(shasum -a 256 "$LIT" | awk '{ print $1 }')" != "$lsha" ]; then mr="${mr},literals_sha=mismatch"
        else
          nl="$(grep -c "^${tag} " "$LIT")"
          if [ "${nl:-0}" -lt 4 ]; then mr="${mr},literals_${tag}=${nl:-0}"
          else
            miss="$(sed -n "s/^${tag} //p" "$LIT" | awk -v c="$CONSOLE" '
              BEGIN { while ((getline l < c) > 0) seen[l]++ }
              { if (seen[$0] != 1) bad++ } END { print bad + 0 }')"
            [ "$miss" = "0" ] || mr="${mr},literal_lines_not_once=${miss}"
          fi
        fi
      fi
      [ -z "$mr" ] && ok=1 || ok=0; item m5 "$ok" "${mr:+${mr#,}}"
      n="$(ccount "^\\[server\\] (mimalloc: )?(thread 0x[0-9a-f]+: )?v${OTHER}\\.")"
      [ "$n" = "0" ] && ok=1 || ok=0; item mi_version "$ok" "other_major_version_lines=${n}"
      n="$(ccount '^\[server\] mimalloc: thread 0x[0-9a-f]+: ')"
      [ "$n" = "0" ] && ok=1 || ok=0; item mi_thread_prefix "$ok" "thread_prefixed_lines=${n}" ;;
  esac
  if [ -z "$PALLOC_FAILED" ]; then echo "PALLOC=TRUE label=${LABEL}"
  else echo "PALLOC=FALSE reason=${PALLOC_FAILED} label=${LABEL}"; fi
  if [ "$FLAVOUR" = "MI" ]; then
    v="$(kv_once "$RUNNER" "MI_POSTINIT_LINES_${CELL}")"
    isint "$v" || v="n/a reason=MI_POSTINIT_LINES=${v:-empty}"
    echo "MI_POSTINIT_LINES_${CELL}=${v}"
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
# ---------------------------------------------------- item 5: per-entry series
# live exists only at census instants, so each census point is joined to the
# nearest LIVE row (elapsed < D: the elapsed = D row is written after the run
# ended) with a number in fp_equiv_mb within 30 s, else dropped and named --
# one rule for every point, TERMINAL included. Rows between two retained
# points get live(t) by linear interpolation; rows outside them are not in the
# series (never extrapolated). pe.csv holds numeric rows only, so the frozen
# fitter's row window is the window.
PE="$EV/$BASE.pe.csv"; RSSPE="$EV/$BASE.rsspe.csv"; PEC="$EV/$BASE.pe-census.csv"; PEJ="$EV/$BASE.pe-join.csv"
: > "$PE"; : > "$RSSPE"; : > "$PEC"; : > "$PEJ"
{
  echo "== readings: per live entry =="
  if [ -n "$LIT_BAD" ]; then
    for k in LIVE_END PE_END PE_LEVEL RSS_PE_LEVEL; do echo "${k}_${CELL}=n/a reason=manifest:${LIT_BAD}"; done
    echo "CENSUS_DROPPED_${CELL}=n/a reason=manifest:${LIT_BAD}"; echo "CENSUS_DROPPED_N_${CELL}=n/a reason=manifest:${LIT_BAD}"
  elif [ -z "$DURATION" ] || [ ! -s "$CSV" ] || [ ! -s "$CONSOLE" ]; then
    for k in LIVE_END PE_END PE_LEVEL RSS_PE_LEVEL; do echo "${k}_${CELL}=n/a reason=no_matrix_csv_or_console"; done
    echo "CENSUS_DROPPED_${CELL}=n/a reason=no_matrix_csv_or_console"; echo "CENSUS_DROPPED_N_${CELL}=n/a reason=no_matrix_csv_or_console"
  else
    awk -v csv="$CSV" -v D="$DURATION" -v a0mib="$LIT_A0_L_MIB" -v lw="$LIT_LEVEL_WINDOW_S" -v c="$CELL" \
        -v pe="$PE" -v rsspe="$RSSPE" -v pec="$PEC" -v pej="$PEJ" '
      function isnum(x) { return x ~ /^-?[0-9]+(\.[0-9]+)?$/ }
      BEGIN {
        FS = ","
        while ((getline l < csv) > 0) {
          nf = split(l, f, ",")
          if (!hdr) { for (i = 1; i <= nf; i++) col[f[i]] = i; hdr = 1
                      if (!("elapsed_secs" in col) || !("fp_equiv_mb" in col) || !("rss_mb" in col)) bad = 1
                      continue }
          if (bad) continue
          e = f[col["elapsed_secs"]]
          # Live rows only; a row whose clock is not a number cannot be placed.
          if (!isnum(e) || e + 0 >= D + 0) continue
          r++; el[r] = e + 0; fp[r] = f[col["fp_equiv_mb"]]; rs[r] = f[col["rss_mb"]]
        }
        FS = " "; a0 = a0mib * 1048576
      }
      /^  (LIVE_COPY|TERMINAL) +t=/ {
        delete g
        for (i = 2; i <= NF; i++) { split($i, kv, "="); g[kv[1]] = kv[2] }
        t = g["t"]; sub(/s$/, "", t)
        key = $1 " " t
        if (key in seenpt) next
        seenpt[key] = 1
        np++; ps[np] = $1; pt[np] = t; pl[np] = g["live"]
      }
      END {
        if (bad) { print "LIVE_END_" c "=n/a reason=missing_csv_column"; print "PE_END_" c "=n/a reason=missing_csv_column"
                   print "PE_LEVEL_" c "=n/a reason=missing_csv_column"; print "RSS_PE_LEVEL_" c "=n/a reason=missing_csv_column"
                   print "CENSUS_DROPPED_" c "=n/a reason=missing_csv_column"; print "CENSUS_DROPPED_N_" c "=n/a reason=missing_csv_column"; exit }
        print "t,row_elapsed_secs,live,fp_equiv_mb,pe_c" > pec
        nd = 0; dl = ""; nk = 0
        for (i = 1; i <= np; i++) {
          if (!isnum(pt[i]) || !isnum(pl[i]) || pl[i] + 0 <= 0) { nd++; dl = dl (dl == "" ? "" : ",") pt[i]; continue }
          best = 0; bd = 31
          for (j = 1; j <= r; j++) if (isnum(fp[j])) { d = el[j] - pt[i]; if (d < 0) d = -d; if (d <= 30 && d < bd) { bd = d; best = j } }
          if (best == 0) { nd++; dl = dl (dl == "" ? "" : ",") pt[i]; continue }
          nk++; kt[nk] = pt[i] + 0; kl[nk] = pl[i] + 0; kf[nk] = fp[best] + 0
          printf "%s,%d,%d,%s,%.6f\n", pt[i], el[best], pl[i], fp[best], (fp[best] * 1048576 - a0) / pl[i] >> pec
        }
        # TERMINAL carries the run-end live count even when it joins no row.
        for (i = np; i >= 1; i--) if (ps[i] == "TERMINAL" && isnum(pl[i])) { le = pl[i] " src=TERMINAL t=" pt[i]; break }
        if (le == "" && nk > 0) le = kl[nk] " src=LIVE_COPY t=" kt[nk]
        print "LIVE_END_" c "=" (le == "" ? "n/a reason=no_census_point" : le)
        print "CENSUS_POINTS_" c "=" np " retained=" nk
        print "CENSUS_DROPPED_" c "=" (dl == "" ? "none" : dl)
        print "CENSUS_DROPPED_N_" c "=" nd
        if (nk > 0) printf "PE_END_%s=%.6f t=%s live=%d\n", c, (kf[nk] * 1048576 - a0) / kl[nk], kt[nk], kl[nk]
        else print "PE_END_" c "=n/a reason=no_retained_census_point"
        print "elapsed_secs,pe" > pe; print "elapsed_secs,rss_pe" > rsspe; print "elapsed_secs,live,fp_equiv_mb" > pej
        nr = 0; lsum = 0; ln = 0; rsum = 0; rn = 0
        for (j = 1; j <= r; j++) {
          if (!isnum(fp[j]) || nk < 2 || el[j] < kt[1] || el[j] > kt[nk]) continue
          for (k = 1; k < nk; k++) if (el[j] >= kt[k] && el[j] <= kt[k + 1]) break
          if (kt[k + 1] > kt[k]) lv = kl[k] + (kl[k + 1] - kl[k]) * (el[j] - kt[k]) / (kt[k + 1] - kt[k]); else lv = kl[k]
          v = (fp[j] * 1048576 - a0) / lv
          printf "%d,%.6f\n", el[j], v >> pe
          printf "%d,%.6f,%s\n", el[j], lv, fp[j] >> pej
          nr++
          inw = (el[j] >= D - lw && el[j] < D + 0)
          if (inw) { lsum += v; ln++ }
          if (isnum(rs[j])) {
            w = (rs[j] * 1048576 - a0) / lv
            printf "%d,%.6f\n", el[j], w >> rsspe
            if (inw) { rsum += w; rn++ }
          }
        }
        print "PE_ROWS_" c "=" nr
        if (ln > 0) printf "PE_LEVEL_%s=%.6f rows=%d\n", c, lsum / ln, ln; else print "PE_LEVEL_" c "=n/a reason=no_rows_in_level_window"
        if (rn > 0) printf "RSS_PE_LEVEL_%s=%.6f rows=%d\n", c, rsum / rn, rn; else print "RSS_PE_LEVEL_" c "=n/a reason=no_rows_in_level_window"
      }' "$CONSOLE" || step_failed PE_SERIES
  fi
} > "$EV/.$BASE.pe-readings.tmp"
cat "$EV/.$BASE.pe-readings.tmp"
PE_LEVEL_V="$(sed -n "s/^PE_LEVEL_${CELL}=//p" "$EV/.$BASE.pe-readings.tmp" | awk '{ print $1 }')"
DROPPED_N="$(sed -n "s/^CENSUS_DROPPED_N_${CELL}=//p" "$EV/.$BASE.pe-readings.tmp" | awk '{ print $1 }')"
rm -f "$EV/.$BASE.pe-readings.tmp"

# ---------------------------------------------------------- item 6: the trend
# One window: the frozen fitter's own rows int(n/2)..n-1 of pe.csv. Its
# printed slope (six decimals) must equal this program's refit of the same
# rows to 5e-7, so the residuals the AR(1) correction reads belong to the fit
# that was published. The fitter's se is optimistic under serial correlation;
# k = sqrt((1 + r1)/(1 - r1)) inflates it (never deflates: r1 < 0 gives 1).
trend() {   # $1 = key prefix, $2 = window (last_half | last_third), $3 = fitter record
  awk -v key="$1_${CELL}" -v win="$2" -v rec="$3" -v L="$PE_LEVEL_V" -v B="$LIT_FLAT_BAR_PER_H" \
      -v D="$DURATION" -v cap="$LIT_STAGE2_MAX_H" -v a0mib="$LIT_A0_L_MIB" -v dropped="${DROPPED_N:-n/a}" \
      -v c="$CELL" -v pej="$PEJ" -v testslope="$4" -F, '
    function isnum(x) { return x ~ /^[-+]?[0-9]+(\.[0-9]+)?([eE][-+]?[0-9]+)?$/ }
    function cls(s, e, inf) {
      # An infinite e places no band anywhere: only UNDERPOWERED can hold.
      if (inf) return "UNDERPOWERED"
      if (s - 2 * e > B + 0) return "RISING"
      if (s + 2 * e < -B) return "FALLING"
      if ((s < 0 ? -s : s) + 2 * e <= B + 0) return "FLAT"
      if (2 * e > B + 0) return "UNDERPOWERED"
      return "MARGINAL"
    }
    NR == 1 { next }
    { n++; t[n] = $1 / 3600.0; y[n] = $2 + 0 }
    END {
      tail = " n=%d dropped=" dropped
      if (win == "last_half") start = int(n / 2) + 1; else start = int(2 * n / 3) + 1
      m = n - start + 1
      if (m < 90) { printf "%s=n/a reason=few_rows n=%d dropped=%s\n", key, (m > 0 ? m : 0), dropped; if (win == "last_half") { print "STAGE2_T_" c "=n/a reason=missing:" c ":e"; nofix("few_rows") } exit }
      if (!isnum(L)) { printf "%s=n/a reason=no_level n=%d\n", key, m; if (win == "last_half") { print "STAGE2_T_" c "=n/a reason=missing:" c ":e"; nofix("no_level") } exit }
      if (L + 0 <= 0) { printf "%s=n/a reason=pe_nonpositive n=%d level=%s\n", key, m, L; if (win == "last_half") { print "STAGE2_T_" c "=n/a reason=missing:" c ":e"; nofix("pe_nonpositive") } exit }
      # The fitter record: slope and se as it printed them.
      nr = split(rec, rf, " ")
      for (i = 1; i <= nr; i++) { p = index(rf[i], "="); if (p) R[substr(rf[i], 1, p - 1)] = substr(rf[i], p + 1) }
      if (testslope != "") R["slope_mb_per_hour"] = testslope
      if (!isnum(R["slope_mb_per_hour"]) || !isnum(R["se_mb_per_hour"]) || R["n"] + 0 != m) {
        printf "%s=n/a reason=fit_error n=%d fitter=%s\n", key, m, (rec == "" ? "none" : rf[1]); if (win == "last_half") { print "STAGE2_T_" c "=n/a reason=missing:" c ":e"; nofix("fit_error") } exit }
      sx = 0; sy = 0
      for (i = start; i <= n; i++) { sx += t[i]; sy += y[i] }
      xb = sx / m; yb = sy / m; sxx = 0; sxy = 0
      for (i = start; i <= n; i++) { dx = t[i] - xb; sxx += dx * dx; sxy += dx * (y[i] - yb) }
      b = sxy / sxx; a = yb - b * xb
      d = b - R["slope_mb_per_hour"]; if (d < 0) d = -d
      printf "FIT_CHECK_%s=%s prog=%.9f fitter=%s absdiff=%.3g\n", key, (d <= 0.0000005 ? "PASS" : "FAIL"), b, R["slope_mb_per_hour"], d
      if (d > 0.0000005) { printf "%s=n/a reason=fit_mismatch n=%d\n", key, m; if (win == "last_half") { print "STAGE2_T_" c "=n/a reason=missing:" c ":e"; nofix("fit_mismatch") } exit }
      sse = 0; num1 = 0; den = 0; sst = 0
      for (i = start; i <= n; i++) { res[i] = y[i] - (a + b * t[i]); sse += res[i] * res[i]; sst += (y[i] - yb) * (y[i] - yb) }
      for (i = start + 1; i <= n; i++) num1 += res[i] * res[i - 1]
      r1 = (sse > 0) ? num1 / sse : 0
      inf = 0
      if (r1 >= 0.99) inf = 1
      else if (r1 < 0) kf = 1
      else kf = sqrt((1 + r1) / (1 - r1))
      s = R["slope_mb_per_hour"] / L; sen = R["se_mb_per_hour"] / L
      e = inf ? 0 : sen * kf
      cl = cls(s, e, inf)
      printf "%s=%s slope_rel=%.6f se_rel=%.6f r1=%.4f se_rel_adj=%s n=%d r2=%s dropped=%s slope_pe_per_h=%s level=%s\n", key, cl, s, sen, r1, (inf ? "r1_ge_0.99" : sprintf("%.6f", e)), m, (sst > 0 ? sprintf("%.6f", 1 - sse / sst) : "NA"), dropped, R["slope_mb_per_hour"], L
      if (win != "last_half") exit
      # Stage-2 length: OLS se scales as W^-3/2 at a fixed cadence and AR(1)
      # structure, so a window W2 = W * (2e / (0.8 B))^(2/3) puts a zero-slope
      # arm 20 % inside the bar; the cell is twice its last-half window.
      W = t[n] - t[start]; Dh = D / 3600.0
      if (inf) printf "STAGE2_T_%s=>STAGE2_MAX_H\n", c
      else {
        T = 2 * W * ((2 * e) / (0.8 * B)) ^ (2 / 3); Ti = int(T); if (Ti < T) Ti++
        if (Ti < Dh) Ti = Dh
        if (Ti > cap + 0) printf "STAGE2_T_%s=>STAGE2_MAX_H\n", c; else printf "STAGE2_T_%s=%g\n", c, Ti
      }
      fixed(s, e, inf, kf)
    }
    function nofix(why) {
      print "FIXED_EST_" c "=n/a reason=" why; print "FIXED_NOTE_" c "=n/a reason=" why; print "TREND_CORR_" c "=n/a reason=" why
    }
    # Recorded only (never a decision input): fp = F0 + MC * live over the
    # window rows. The intercept is not identifiable while live grows near
    # linearly, which is why nothing downstream reads these lines.
    function fixed(s, e, inf, kf,   j, nl, lv, fb, sl, sf, xl, yl, sxl, sxyl, dl, bl, al, ssel, seF, F, note, sq, tl, bt, lmid, bias, sc, stt, sxt, cc) {
      nl = 0
      while ((getline l < pej) > 0) { if (++j == 1) continue; split(l, q, ","); nl++; lv[nl] = q[2] + 0; fb[nl] = q[3] * 1048576 }
      if (nl != n) { nofix("join_rows=" nl); return }
      sl = 0; sf = 0
      for (i = start; i <= n; i++) { sl += lv[i]; sf += fb[i] }
      xl = sl / m; yl = sf / m; sxl = 0; sxyl = 0
      for (i = start; i <= n; i++) { dl = lv[i] - xl; sxl += dl * dl; sxyl += dl * (fb[i] - yl) }
      if (sxl <= 0 || m < 3) { nofix("no_live_variance"); return }
      bl = sxyl / sxl; al = yl - bl * xl; ssel = 0
      for (i = start; i <= n; i++) { dl = fb[i] - (al + bl * lv[i]); ssel += dl * dl }
      F = al - a0mib * 1048576
      sq = sqrt(ssel / (m - 2) * (1 / m + xl * xl / sxl))
      if (inf) seF = "inf"; else seF = sq * kf
      if (!inf && F - 2 * seF > 0) note = "positive_fixed_memory"
      else if (F < 0) note = "rising_marginal_cost_suspected"
      else note = "none"
      printf "FIXED_EST_%s=%.3f se_mib=%s\n", c, F / 1048576, (inf ? "inf" : sprintf("%.3f", seF / 1048576))
      print "FIXED_NOTE_" c "=" note
      # dL/dt over the window (per hour) and the mid-window live count.
      stt = 0
      for (i = start; i <= n; i++) stt += t[i]
      tl = stt / m; sxt = 0; bt = 0
      for (i = start; i <= n; i++) { sxt += (t[i] - tl) * (t[i] - tl); bt += (t[i] - tl) * (lv[i] - xl) }
      bt = bt / sxt; lmid = xl
      bias = -F * bt / (lmid * lmid * L)
      sc = s - bias
      cc = cls(sc, e, inf)
      printf "TREND_CORR_%s=%s slope_rel_corr=%.6f fixed_bias=%.6f recorded_only\n", c, cc, sc, bias
    }' "$PE"
}
{
  if [ -n "$LIT_BAD" ]; then
    for k in TREND TREND3; do echo "${k}_${CELL}=n/a reason=manifest:${LIT_BAD}"; done
    for k in STAGE2_T FIXED_EST FIXED_NOTE TREND_CORR; do echo "${k}_${CELL}=n/a reason=manifest:${LIT_BAD}"; done
    RC=3
  elif [ "$FIT_OK" != "1" ]; then
    for k in TREND TREND3 STAGE2_T FIXED_EST FIXED_NOTE TREND_CORR; do echo "${k}_${CELL}=n/a reason=frozen_fit_sha"; done
  else
    echo "== readings: trend (per-entry, relative per hour) =="
    npe="$(awk 'NR > 1' "$PE" | grep -c .)"
    if [ "$npe" -ge 2 ]; then REC_H="$(fit_stream pe last_half < "$PE")"; else REC_H=""; fi
    TSLOPE=""
    [ "$SYNTH" = "1" ] && TSLOPE="${SPEC377_TEST_FIT_SLOPE:-}"
    printf 'pe last_half %s\n' "${REC_H:-none}" >> "$FITS"
    trend TREND last_half "$REC_H" "$TSLOPE" || step_failed TREND
    third="$EV/.$BASE.pe-third.tmp"
    awk 'NR == 1 { h = $0; next } { r[++n] = $0 } END { print h; for (i = int(2 * n / 3) + 1; i <= n; i++) print r[i] }' "$PE" > "$third"
    if [ "$npe" -ge 2 ]; then REC_T="$(fit_stream pe full < "$third")"; else REC_T=""; fi
    rm -f "$third"
    printf 'pe last_third %s\n' "${REC_T:-none}" >> "$FITS"
    trend TREND3 last_third "$REC_T" "" || step_failed TREND3
  fi
}

# ------------------------------------------------- item 7: recorded readings
{
  echo "== readings: recorded (never decision inputs) =="
  if [ -z "$DURATION" ] || [ ! -s "$CSV" ]; then
    for k in LAZY_MAX HWM_END ANON_HUGE_END; do echo "${k}_${CELL}=n/a reason=no_matrix_or_csv"; done
  else
    awk -F, -v D="$DURATION" -v c="$CELL" '
      function isnum(x) { return x ~ /^-?[0-9]+(\.[0-9]+)?$/ }
      NR == 1 { for (i = 1; i <= NF; i++) col[$i] = i; next }
      !isnum($col["elapsed_secs"]) || $col["elapsed_secs"] + 0 >= D + 0 { next }
      {
        lz = $col["lazyfree_mb"]; an = $col["anon_mb"]
        if (isnum(lz) && (!have || lz + 0 > mx + 0)) { mx = lz; mxt = $col["elapsed_secs"]; mxa = an; have = 1 }
        if (isnum($col["hwm_rss_mb"])) hw = $col["hwm_rss_mb"]
        if (isnum($col["anon_huge_mb"])) ah = $col["anon_huge_mb"]
      }
      END {
        if (!have) print "LAZY_MAX_" c "=n/a reason=no_live_row_with_lazyfree"
        else printf "LAZY_MAX_%s=%s ratio=%s row=%s\n", c, mx, (isnum(mxa) && mxa + 0 > 0 ? sprintf("%.6f", mx / mxa) : "n/a"), mxt
        print "HWM_END_" c "=" (hw == "" ? "n/a reason=no_live_row" : hw)
        print "ANON_HUGE_END_" c "=" (ah == "" ? "n/a reason=no_live_row" : ah)
      }' "$CSV" || step_failed RECORDED
  fi
  # wal + redb over the live rows that carry both, last half, MB/h.
  if [ -n "$DURATION" ] && [ -s "$CSV" ] && [ "$FIT_OK" = "1" ]; then
    rec="$(awk -F, -v D="$DURATION" 'NR == 1 { for (i = 1; i <= NF; i++) col[$i] = i; print "elapsed_secs,disk_wr_mb"; next }
      $col["elapsed_secs"] + 0 < D + 0 && $col["wal_mb"] != "" && $col["redb_mb"] != "" { printf "%s,%.3f\n", $col["elapsed_secs"], $col["wal_mb"] + $col["redb_mb"] }' "$CSV" \
      | fit_stream disk_wr_mb last_half)"
    printf 'disk_wr_mb last_half %s\n' "$rec" >> "$FITS"
    printf '%s\n' "$rec" | awk -v c="$CELL" '{ for (i = 1; i <= NF; i++) { p = index($i, "="); if (p) f[substr($i, 1, p - 1)] = substr($i, p + 1) } }
      END { if (f["slope_mb_per_hour"] == "") print "DISK_SLOPE_" c "=n/a reason=fit_error"
            else print "DISK_SLOPE_" c "=" f["slope_mb_per_hour"] " se=" f["se_mb_per_hour"] " n=" f["n"] }'
  else
    echo "DISK_SLOPE_${CELL}=n/a reason=no_matrix_csv_or_fitter"
  fi
  # JE-native at the last probe row; REACH_PE divides the reachable bytes
  # (allocated above the startup level) by the run-end live count.
  if [ "$FLAVOUR" = "JE" ]; then
    LIVE_T="$(awk '/^  TERMINAL +t=/ { for (i = 2; i <= NF; i++) { split($i, kv, "="); if (kv[1] == "live") v = kv[2] } } END { print v }' "$CONSOLE" 2>/dev/null)"
    awk -F, -v D="${DURATION:-0}" -v c="$CELL" -v a0mib="${LIT_A0_L_MIB:-}" -v live="$LIVE_T" '
      function isnum(x) { return x ~ /^[0-9]+(\.[0-9]+)?$/ }
      NR == 1 { for (i = 1; i <= NF; i++) col[$i] = i; next }
      ("je_allocated" in col) && isnum($col["je_allocated"]) && $col["je_allocated"] + 0 > 0 && isnum($col["je_active"]) && isnum($col["je_resident"]) && isnum($col["je_metadata"]) {
        al = $col["je_allocated"]; ac = $col["je_active"]; rs = $col["je_resident"]; me = $col["je_metadata"]; t = $col["elapsed_secs"]; have = 1 }
      END {
        if (!have) { r = "n/a reason=no_probe_row"
          print "AMP_JE_" c "=" r; print "FRAG_SHAREL_" c "=" r; print "DIRTY_SHAREL_" c "=" r; print "REACH_PE_" c "=" r; exit }
        printf "AMP_JE_%s=%.6f row=%s\n", c, rs / al, t
        printf "FRAG_SHAREL_%s=%.6f\n", c, (ac - al) / al
        printf "DIRTY_SHAREL_%s=%.6f\n", c, (rs - ac - me) / al
        if (!isnum(live) || live + 0 <= 0) print "REACH_PE_" c "=n/a reason=no_terminal_live"
        else if (!isnum(a0mib)) print "REACH_PE_" c "=n/a reason=manifest_A0_L_MIB"
        else printf "REACH_PE_%s=%.3f live=%d\n", c, (al - a0mib * 1048576) / live, live
      }' "$CSV" 2>/dev/null || step_failed JE_NATIVE
  fi
}
[ -n "$LIT_BAD" ] && RC=3
exit "$RC"
