#!/bin/sh
# spec356c-targets.sh -- THE ONE BUILDER for the five observation-target tables (R3.4).
#
# usage: spec356c-targets.sh <evidence-dir> <cell-basename> <out-prefix> <out-suffix> [prior-basename]
#
#   replicate 1 : spec356c-targets.sh EVID spec356c-r1  spec356c-        -r1
#   replicate 2 : spec356c-targets.sh EVID spec356c-r2  spec356c-        -r2
#   PRE-DATA dry run (R3.5) :
#                 spec356c-targets.sh EVID spec356-long spec356c-dryrun- -long
#
#   Outputs, into <evidence-dir>:
#     <out-prefix>t1-lwmpass<out-suffix>.csv
#     <out-prefix>t2-drains<out-suffix>.csv
#     <out-prefix>t3-windows<out-suffix>.csv
#     <out-prefix>t4-rate<out-suffix>.csv
#     <out-prefix>t5-fate<out-suffix>.csv
#
#   THE NAMING SCHEME IS THE ONLY THING THE PREFIX/SUFFIX PAIR CHANGES. Every derivation below runs on
#   ONE code path for the replicates and for the dry run -- a dry run that exercised different code
#   would buy nothing, which is the whole reason R3.5 makes it a gate rather than a rehearsal.
#
# WHAT THIS BUILDER IS, AND IS NOT (R3.4, R1.3):
#   It is an OBSERVATION builder. It computes NO predicate term, reads NO threshold, decides nothing
#   and emits NO verdict. It alters no Step, no threshold, no ordering and no conditional, and nothing
#   it emits is read by any classification limb, step or slot. If a column cannot be computed it is
#   emitted EMPTY and NEVER a substitute.
#   NO FIT ANYWHERE: no OLS fit, no slope, no standard error, no r2, no alpha, no t-statistic and no
#   significance claim is computed on any target series (R3, R2.4).
#
# THE THREE RESOLUTION CAVEATS THAT BOUND EVERY TABLE (R3.2), restated in every output header:
#   (C1) SAMPLED, NOT EVENT-DRIVEN. LWM advances were measured ~31 s apart while the ledger scrapes
#        every 10 s, so every "at the moment LWM passes" reading here is THE FIRST SCRAPE AT OR AFTER
#        the advance -- a BUCKET, not an instant, with up to one cadence of lag.
#   (C2) THE ADJ-7 BOUNDARY IS NOT CROSSED. eligible_refs (L) and ineligible_refs (P) are PRE-drain;
#        indexed_refs is POST-drain. No ratio is taken across that boundary in any table here, and the
#        two prohibited ratios P/indexed_refs and L/indexed_refs appear NOWHERE in this builder.
#   (C3) MULTI-EVENT BUCKETS ARE REPORTED, NEVER SMOOTHED. A 10 s bucket carrying more than one event
#        does not separate into its events; it is published with an explicit per-bucket event count and
#        is NEVER averaged into a per-event figure.
#
# WINDOWS, STATED HERE SO NO KEYBOARD CHOOSES THEM WITH THE DATA VISIBLE:
#   COORDINATE LAST HALF = rows whose `elapsed_secs` EXCEEDS the full ledger's elapsed midpoint
#        (t_first + t_last)/2 -- a COORDINATE, never a row index (ADJ-17 as adjudicated by ADJ-18).
#        T1(b) and T5 read it. It is the same window the R8.1 walk reads.
#   FULL LEDGER = every row from the first `elapsed_secs` to the last. T2 reads it, by pre-declaration
#        (R3: T2 enumerates EVERY non-empty drain), and T3/T4 partition it.
#   1,000 s COORDINATE WINDOWS = ORIGIN t0 := the ledger's FIRST `elapsed_secs` value, which is 10 and
#        NOT 0 on every committed cell. Window k covers [t0+1000k, t0+1000(k+1)), HALF-OPEN. A window
#        is FULL when its whole nominal width is covered by the ledger (t0+1000(k+1) <= t_last) and
#        PARTIAL otherwise; the PARTIAL window is at most one and always last. It is EMITTED and
#        MARKED, and NO RATE IS EVER COMPUTED FROM IT.
#
# THE PRE-DECLARED DRY-RUN SHAPES AND THEIR GRADING BOUNDS (R3.5, and Audit v2 recommendation 5).
#   On the 14,400 s / 43-column / 1,440-row subject `spec356-long.prune.csv`:
#     t3-windows  14 FULL + 1 PARTIAL   graded EXACT, FULL and PARTIAL graded SEPARATELY
#     t4-rate     53 whole-run / 2 coordinate-last-half   graded EXACT
#     t1-lwmpass  ~232 rows             graded >= 200 rows
#     t5-fate     ~233 rows             graded >= 200 rows
#     t2-drains   ~53 rows              graded >= 40 rows
#   The last three carried only a "publish the row count" obligation, and publishing a number beside a
#   different number is not an assertion that they agree -- a builder reading the wrong window and
#   emitting 5 rows would have satisfied that in full. The range bounds above close it. They sit far
#   below the measured 232 / 233 / 53, so they cannot be tuned to the answer, and A COUNT OUTSIDE ITS
#   BAND IS RED IN G1, which is where R3.5 already says a builder defect is fixed.
#
# READ-ONLY: this builder opens the cell's committed `prune.csv` and the prior arm's committed
# `prune.csv` for reading and writes only its own five outputs. It regenerates no ledger.

set -e

E=${1:?evidence dir}
CELL=${2:?cell basename}
PFX=${3:?output prefix}
SFX=${4:?output suffix}
PRIOR=${5:-spec356-long}

PR=$E/$CELL.prune.csv
PRIOR_PR=$E/$PRIOR.prune.csv

die() { echo "TARGETS FAIL: $*" >&2 ; exit 3 ; }

[ -f "$PR" ]       || die "$PR is ABSENT: there is no ledger to build targets from"
[ -f "$PRIOR_PR" ] || die "$PRIOR_PR is ABSENT: T4's prior-duration arm row cannot be read"

# Every column any table below reads is resolved BY NAME from the ledger's own header, and the whole
# set is checked ONCE, here. A column index silently resolving to 0 would read field $0 -- the entire
# row -- and the table would still be perfectly reproducible from the committed builder, which is the
# one failure mode reproducibility alone cannot catch.
REQ='elapsed_secs
topgun_or_prune_passes_total
topgun_or_prune_bytes_freed_total
topgun_or_prune_empty_drains_total
topgun_or_prune_nonempty_drains_total
topgun_or_prune_lwm_advances_total
topgun_or_prune_indexed_refs
topgun_or_prune_indexed_epochs
topgun_or_prune_eligible_refs
topgun_or_prune_ineligible_refs
topgun_or_prune_current_epoch
topgun_or_prune_low_water_mark
topgun_or_prune_last_drained_epoch
topgun_or_prune_drain_refs_sum
topgun_or_prune_drain_refs_count
topgun_or_prune_drain_epochs_sum
topgun_or_prune_drain_epochs_count
topgun_or_prune_epoch_considered_sum
topgun_or_prune_epoch_dropped_sum
topgun_or_prune_epoch_bytes_freed_sum
topgun_or_prune_epoch_bytes_freed_count
topgun_ormap_tombstone_bytes_total'
W=${TMPDIR:-/tmp}/spec356c-targets.$$ ; mkdir -p "$W" ; trap 'rm -rf "$W"' EXIT
head -1 "$PR"       | tr ',' '\n' | tr -d '\r' > "$W/hdr"
head -1 "$PRIOR_PR" | tr ',' '\n' | tr -d '\r' > "$W/phdr"
for col in $REQ ; do
  grep -qxF "$col" "$W/hdr" || die "$PR carries no column [$col]"
done
for col in elapsed_secs topgun_or_prune_nonempty_drains_total ; do
  grep -qxF "$col" "$W/phdr" || die "$PRIOR_PR carries no column [$col]: T4's prior-arm row is unreadable"
done

T1=$E/${PFX}t1-lwmpass${SFX}.csv
T2=$E/${PFX}t2-drains${SFX}.csv
T3=$E/${PFX}t3-windows${SFX}.csv
T4=$E/${PFX}t4-rate${SFX}.csv
T5=$E/${PFX}t5-fate${SFX}.csv

# The classification-input status is DERIVED from the output prefix, on the one code path: a set whose
# names carry `dryrun` is the PRE-DATA reference set and is fenced off from every classification limb.
case "$PFX" in
  *dryrun*) STATUS="REFERENCE ONLY, NEVER EVIDENCE. This file is the R3.5 PRE-DATA dry run: it is built
#   from an ALREADY-COMMITTED prior-arm ledger, it carries that prior arm's basename in its own
#   filename, and NO classification limb, step, slot or target table may ever read it. It exists to
#   prove the builder was exercised over real bytes before the 16 h clock started." ;;
  *)        STATUS="replicate target table (R3.3). It is an OBSERVATION accompanying the frozen
#   classification and is read by no limb, step or slot." ;;
esac

# ---- the caveat block every output carries, verbatim, so no table can be read without it ----
hdr() { # hdr <title>
  echo "# $1"
  echo "# BUILDER: spec356c-targets.sh -- SPEC-356c R3.4's ONE target builder, committed before the data."
  echo "# SUBJECT: $CELL.prune.csv    PRIOR-DURATION ARM: $PRIOR.prune.csv"
  echo "# STATUS:  $STATUS"
  echo "# NOT A PREDICATE: this builder computes no predicate term, reads no threshold, decides nothing"
  echo "#   and emits no verdict; it alters no Step, threshold, ordering or conditional. A column it"
  echo "#   cannot compute is emitted EMPTY and never a substitute."
  echo "# NO FIT: no OLS fit, no slope, no standard error, no r2, no alpha, no t-statistic and no"
  echo "#   significance claim is computed on any series in this file."
  echo "# (C1) SAMPLED, NOT EVENT-DRIVEN: the ledger scrapes every 10 s while LWM advances ~31 s apart,"
  echo "#   so every 'at the moment LWM passes' reading here is THE FIRST SCRAPE AT OR AFTER the"
  echo "#   advance -- a BUCKET, not an instant, carrying up to one cadence of lag."
  echo "# (C2) THE ADJ-7 BOUNDARY IS NOT CROSSED: eligible_refs (L) and ineligible_refs (P) are"
  echo "#   PRE-drain; indexed_refs is POST-drain. No ratio is taken across that boundary anywhere in"
  echo "#   this file, and the prohibited ratios P/indexed_refs and L/indexed_refs appear NOWHERE."
  echo "# (C3) MULTI-EVENT BUCKETS ARE REPORTED, NEVER SMOOTHED: a 10 s bucket carrying more than one"
  echo "#   event does not separate into its events; the per-bucket event count is published"
  echo "#   explicitly and is NEVER averaged into a per-event figure."
  echo "# UNITS DIFFER ON PURPOSE AND ARE NOT RECONCILED: indexed_refs is a REF COUNT, tombstone_bytes"
  echo "#   is BYTES. No bytes-per-ref constant is assumed, fitted or implied anywhere in this file."
  echo "# The first non-'#' line below is the column header; every line after it is a data row."
}

# ------------------------------------------------------------------------------------------------
# T1(b) -- the LWM-pass ledger over the AGGREGATE index gauges, on the COORDINATE LAST HALF.
# ------------------------------------------------------------------------------------------------
{
hdr "T1(b) -- LWM-PASS LEDGER (coordinate last half). DERIVED (R3.3)."
cat <<'EOT'
# WINDOW: the COORDINATE LAST HALF -- rows whose elapsed_secs EXCEEDS the full ledger's elapsed
#   midpoint. One record per row i of that window at which topgun_or_prune_low_water_mark STRICTLY
#   INCREASES relative to row i-1. Row i-1 is the LEDGER's immediately preceding row, which for the
#   window's first row lies just outside the window; that is the literal reading of R3's derivation
#   and it is what makes the predicate defined for every row of the window.
# WHAT THIS LEDGER ANSWERS: what the AGGREGATE index looked like as the LWM walked past. WHAT IT
#   CANNOT DISTINGUISH: "epoch e's refs were never indexed" from "they were indexed and left the index
#   earlier". T1(a) -- per-epoch index membership -- is OUT-OF-SCOPE (a per-epoch labelled emission on
#   the prune path is a .rs change R0.4 forbids categorically); owner TODO-634. THIS TABLE IS NOT ITS
#   ANSWER and hypothesis (a) is addressed in DIRECTION ONLY.
# refs_per_indexed_epoch IS A MEAN OVER THE INDEX, NOT A PER-EPOCH VALUE. It is indexed_refs divided
#   by indexed_epochs, and it is EMPTY when indexed_epochs = 0.
# EVENTS_IN_BUCKET (C3) is the number of LWM ADVANCE EVENTS attributable to this bucket, read as the
#   delta of topgun_or_prune_lwm_advances_total across (i-1, i]. It is published, never averaged.
#   epochs_passed (lwm_after - lwm_before) is published beside it and is not assumed to be 1.
# The d_*_since_prev_pass columns are deltas since the PREVIOUS ROW OF THIS TABLE (the previous
#   LWM-pass bucket), exact between the two SAMPLED instants. On the first row of the table there is
#   no previous pass, so all three are EMPTY -- never zero, never a substitute.
# T1(b) AND T5 ARE ONE OBSERVATION REACHED BY TWO ROUTES wherever current_epoch == low_water_mark:
#   this table's rows are a PROJECTION of T5's epoch rows, and agreement between the two is a
#   RESTATEMENT, NOT CORROBORATION. The measured equality rate for this replicate is published in the
#   header of the t5-fate table.
EOT
awk -F, -v OFS=, '
NR==1 { for (i=1;i<=NF;i++) c[$i]=i
        need="elapsed_secs topgun_or_prune_low_water_mark topgun_or_prune_current_epoch topgun_or_prune_indexed_refs topgun_or_prune_indexed_epochs topgun_or_prune_eligible_refs topgun_or_prune_ineligible_refs topgun_ormap_tombstone_bytes_total topgun_or_prune_bytes_freed_total topgun_or_prune_nonempty_drains_total topgun_or_prune_lwm_advances_total"
        n=split(need,w," "); for (i=1;i<=n;i++) if (!(w[i] in c)) { print "MISSING COLUMN " w[i] > "/dev/stderr"; exit 1 }
        next }
{ n++
  t[n]=$(c["elapsed_secs"])+0 ; lwm[n]=$(c["topgun_or_prune_low_water_mark"])+0
  ce[n]=$(c["topgun_or_prune_current_epoch"])+0
  ir[n]=$(c["topgun_or_prune_indexed_refs"])+0 ; ie[n]=$(c["topgun_or_prune_indexed_epochs"])+0
  el[n]=$(c["topgun_or_prune_eligible_refs"])+0 ; il[n]=$(c["topgun_or_prune_ineligible_refs"])+0
  tb[n]=$(c["topgun_ormap_tombstone_bytes_total"])+0 ; bf[n]=$(c["topgun_or_prune_bytes_freed_total"])+0
  nd[n]=$(c["topgun_or_prune_nonempty_drains_total"])+0 ; la[n]=$(c["topgun_or_prune_lwm_advances_total"])+0 }
END {
  if (n < 2) exit 0
  tmid = (t[1] + t[n]) / 2
  print "t_secs","lwm_before","lwm_after","epochs_passed","current_epoch","indexed_refs","indexed_epochs","refs_per_indexed_epoch","eligible_refs","ineligible_refs","d_tombstone_bytes_since_prev_pass","d_bytes_freed_since_prev_pass","d_nonempty_drains_since_prev_pass","EVENTS_IN_BUCKET"
  prev = 0
  for (i = 2; i <= n; i++) {
    if (t[i] <= tmid) continue
    if (lwm[i] <= lwm[i-1]) continue
    rpe = (ie[i] > 0) ? sprintf("%.6f", ir[i] / ie[i]) : ""
    if (prev) { dtb = tb[i] - tb[prev] ; dbf = bf[i] - bf[prev] ; dnd = nd[i] - nd[prev] }
    else      { dtb = "" ; dbf = "" ; dnd = "" }
    print t[i], lwm[i-1], lwm[i], lwm[i]-lwm[i-1], ce[i], ir[i], ie[i], rpe, el[i], il[i], dtb, dbf, dnd, la[i]-la[i-1]
    prev = i
  }
}' "$PR"
} > "$T1"

# ------------------------------------------------------------------------------------------------
# T2 -- FULL content enumeration of every non-empty drain. THE WINDOW IS THE FULL LEDGER.
# ------------------------------------------------------------------------------------------------
{
hdr "T2 -- NON-EMPTY DRAIN ENUMERATION (FULL LEDGER). DERIVED (R3.3)."
cat <<'EOT'
# WINDOW: THE FULL LEDGER -- every row from the first elapsed_secs to the last. NOT the coordinate
#   last half and NOT any other slice. T2's own words are "FULL content enumeration of every non-empty
#   drain", and the two readings do not differ by a detail: on the committed prior arm they differ by
#   more than 25x in row count (53 whole-run against 2 in the coordinate last half).
# One record per 10 s bucket in which topgun_or_prune_nonempty_drains_total INCREASES.
# (C3) drains_in_bucket IS THIS TABLE'S EVENTS_IN_BUCKET COUNT. A bucket carrying more than one drain
#   does not separate into its drains; the count is published explicitly and the bucket is NEVER
#   averaged into a per-drain figure. d_drain_refs_sum_over_count is a RATIO OF SUMS across the bucket
#   (the throughput reading), not a mean of per-drain ratios, and it is EMPTY when the count is 0.
# bytes_freed_matches_attribution := (d_bytes_freed_total == d_epoch_bytes_freed_sum). IT IS EMITTED
#   ON EVERY ROW, INCLUDING WHEN EVERY DRAIN MATCHES -- a column that appears only on mismatch cannot
#   be told apart from a column nobody computed.
# WHAT THAT IDENTITY BOUNDS. It is an INTERNAL CONSISTENCY CHECK BETWEEN TWO COMMITTED COUNTERS. It
#   CANNOT compare either against an independent ground truth of the epoch's true tombstone byte size,
#   which no committed column carries. A mismatch is evidence of an accounting gap; a MATCH EXCLUDES
#   ONLY THE GAP BETWEEN THOSE TWO COUNTERS, not every reclaim path.
# T2(exactness) -- "was bytes_freed incremented by the EXACT tombstone size", against an INDEPENDENT
#   ground truth -- is OUT-OF-SCOPE: topgun_ormap_tombstone_bytes_total is a whole-store monotone
#   total and epoch_bytes_freed_sum is the prune path's OWN attribution, i.e. the very quantity under
#   suspicion; producing the independent figure is a .rs change R0.4 forbids categorically. Owner
#   TODO-634. THE bytes_freed_matches_attribution COLUMN IS NOT PRESENTED AS ITS ANSWER.
# d_drain_epochs_sum is published so the reader can see whether T5's contiguity inference was ever
#   exercised at all (SPEC-356b measured 1.000 epoch per drain, under which T5's drained set is EXACT).
EOT
awk -F, -v OFS=, '
NR==1 { for (i=1;i<=NF;i++) c[$i]=i; next }
{ n++
  t[n]=$(c["elapsed_secs"])+0
  nd[n]=$(c["topgun_or_prune_nonempty_drains_total"])+0
  drs[n]=$(c["topgun_or_prune_drain_refs_sum"])+0 ; drc[n]=$(c["topgun_or_prune_drain_refs_count"])+0
  des[n]=$(c["topgun_or_prune_drain_epochs_sum"])+0 ; dec[n]=$(c["topgun_or_prune_drain_epochs_count"])+0
  lde[n]=$(c["topgun_or_prune_last_drained_epoch"])+0
  ecs[n]=$(c["topgun_or_prune_epoch_considered_sum"])+0 ; eds[n]=$(c["topgun_or_prune_epoch_dropped_sum"])+0
  ebs[n]=$(c["topgun_or_prune_epoch_bytes_freed_sum"])+0 ; ebc[n]=$(c["topgun_or_prune_epoch_bytes_freed_count"])+0
  bf[n]=$(c["topgun_or_prune_bytes_freed_total"])+0 }
END {
  print "t_secs","drains_in_bucket","d_drain_refs_sum","d_drain_refs_count","d_drain_refs_sum_over_count","d_drain_epochs_sum","d_drain_epochs_count","last_drained_epoch_before","last_drained_epoch_after","d_epoch_considered_sum","d_epoch_dropped_sum","d_epoch_bytes_freed_sum","d_epoch_bytes_freed_count","d_bytes_freed_total","bytes_freed_matches_attribution"
  for (i = 2; i <= n; i++) {
    if (nd[i] <= nd[i-1]) continue
    ddrs = drs[i]-drs[i-1] ; ddrc = drc[i]-drc[i-1]
    q = (ddrc > 0) ? sprintf("%.6f", ddrs / ddrc) : ""
    dbf = bf[i]-bf[i-1] ; debs = ebs[i]-ebs[i-1]
    print t[i], nd[i]-nd[i-1], ddrs, ddrc, q, des[i]-des[i-1], dec[i]-dec[i-1], lde[i-1], lde[i], \
          ecs[i]-ecs[i-1], eds[i]-eds[i-1], debs, ebc[i]-ebc[i-1], dbf, (dbf == debs ? "true" : "false")
  }
}' "$PR"
} > "$T2"

# ------------------------------------------------------------------------------------------------
# T3 -- delta(LWM) vs delta(bytes_freed), on consecutive 1,000 s COORDINATE windows.
# ------------------------------------------------------------------------------------------------
{
hdr "T3 -- 1,000 s COORDINATE WINDOWS over the FULL LEDGER. DERIVED (R3.3)."
cat <<'EOT'
# ORIGIN AND TAIL ARE LITERALS, NOT CHOICES.
#   ORIGIN t0 := the ledger's FIRST elapsed_secs value -- which is 10, NOT 0, on every committed cell
#     (the runner's first tick is gated). Window k covers [t0+1000k, t0+1000(k+1)), HALF-OPEN, so no
#     row is in two windows and no row is in none. THE PARTITION IS ON THE COORDINATE, NEVER ON A ROW
#     INDEX, so this table cannot be attacked by a row-count edit.
#   TAIL RULE: window_kind reads FULL when the window's whole nominal width is covered by the ledger
#     (t0+1000(k+1) <= t_last) and PARTIAL otherwise. The PARTIAL window is at most one and always
#     last. It is EMITTED and MARKED, carries its OWN rows and t_hi, and NO RATE IS EVER COMPUTED FROM
#     IT -- dropping the tail silently, or letting it enter a rate, is the inflation this forecloses.
#     FULL rows and PARTIAL rows are graded SEPARATELY, so ">= 28" cannot be satisfied by a short
#     window standing in for a full one.
#   t_lo is the window's NOMINAL half-open lower bound; t_hi is the LAST OBSERVED elapsed_secs inside
#     the window (for the PARTIAL window that is the ledger's own last coordinate).
# DELTAS TELESCOPE ACROSS THE BOUNDARIES: d_X for window k is X at the window's last row minus X at
#   the PREVIOUS window's last row; for window 0 the base is the ledger's own first row. Summed over
#   every window this reproduces X[last] - X[first] exactly, with no row double-counted or dropped.
# d_backlog_bytes IS ADJ-14's NAMED SERIES: d_tombstone_bytes_total - d_bytes_freed_total.
# bytes_freed_per_lwm_epoch is emitted only when d_low_water_mark > 0, and is EMPTY otherwise.
# NO FIT IS TAKEN ON THIS SERIES. Whether the divergence accelerates, is linear, or saturates is a
#   DESCRIPTION OF THE PLOTTED SERIES and explicitly not a fitted claim: no slope, no SE, no r2, no
#   alpha is computed here or downstream of here.
EOT
awk -F, -v OFS=, '
NR==1 { for (i=1;i<=NF;i++) c[$i]=i; next }
{ n++
  t[n]=$(c["elapsed_secs"])+0
  lwm[n]=$(c["topgun_or_prune_low_water_mark"])+0
  bf[n]=$(c["topgun_or_prune_bytes_freed_total"])+0
  tb[n]=$(c["topgun_ormap_tombstone_bytes_total"])+0
  ndr[n]=$(c["topgun_or_prune_nonempty_drains_total"])+0
  edr[n]=$(c["topgun_or_prune_empty_drains_total"])+0
  ps[n]=$(c["topgun_or_prune_passes_total"])+0 }
END {
  if (n < 2) exit 0
  t0 = t[1] ; tlast = t[n] ; kmax = int((tlast - t0) / 1000)
  print "w_index","window_kind","t_lo","t_hi","rows","d_low_water_mark","d_bytes_freed_total","d_tombstone_bytes_total","d_backlog_bytes","d_nonempty_drains","d_empty_drains","d_passes","bytes_freed_per_lwm_epoch"
  base = 1                                   # window 0 measures from the ledger'\''s own first row
  for (k = 0; k <= kmax; k++) {
    lo = t0 + 1000*k ; hinom = t0 + 1000*(k+1)
    rows = 0 ; last = 0
    for (i = 1; i <= n; i++) if (t[i] >= lo && t[i] < hinom) { rows++ ; last = i }
    if (rows == 0) continue
    kind = (hinom <= tlast) ? "FULL" : "PARTIAL"
    dl = lwm[last]-lwm[base] ; db = bf[last]-bf[base] ; dt = tb[last]-tb[base]
    bpe = (dl > 0) ? sprintf("%.6f", db / dl) : ""
    print k, kind, lo, t[last], rows, dl, db, dt, dt-db, ndr[last]-ndr[base], edr[last]-edr[base], ps[last]-ps[base], bpe
    base = last
  }
}' "$PR"
} > "$T3"

# ------------------------------------------------------------------------------------------------
# T4 -- non-empty drain RATE vs DURATION, with the discriminant BOUND to exactly one of two counts.
# ------------------------------------------------------------------------------------------------
{
hdr "T4 -- NON-EMPTY DRAIN RATE vs DURATION. DERIVED (R3.3)."
cat <<'EOT'
# THE BINDING, MARKED IN THE TABLE ITSELF BY THE `discriminant_role` COLUMN AND RESTATED HERE:
#   THE DISCRIMINANT IS THE COORDINATE-LAST-HALF COUNT, AND ONLY IT. The whole-run count is PUBLISHED
#   FOR COMPLETENESS AND IS NEVER THE DISCRIMINANT.
#   The reason the whole-run count is disqualified is MEASURED, not stylistic: the process is
#   NON-STATIONARY AND FRONT-LOADED. On the committed prior arm, 39 of 53 drains occur before
#   t = 1,860 s and 51 of 53 before t = 5,090 s, leaving 2 in the last 9,310 s. A STARTUP BURST DOES
#   NOT SCALE WITH DURATION AT ALL, so a whole-run reading answers T4's question with an artefact of
#   when the run began. Only the last-half regime is quasi-stationary.
#   The discriminant reads: a fixed-rate periodic trigger predicts a count roughly PROPORTIONAL to
#   duration; a one-time transient predicts a count that DOES NOT GROW.
# ROWS. `row_kind` takes exactly these values:
#   WINDOW                      -- one per T3 window, REUSING T3's windows unchanged; carries the
#                                  CUMULATIVE nonempty_drains_total at the 1,000 s boundary.
#   SUBJECT_WHOLE_RUN           -- the subject's whole-run count. NO RATE: its span crosses the
#                                  PARTIAL tail, and R3 excludes PARTIAL from EVERY rate.
#   SUBJECT_FULL_WINDOWS        -- the subject's rate over its FULL windows only, which is the
#                                  whole-run rate with the PARTIAL tail excluded as R3 requires.
#   SUBJECT_COORD_LAST_HALF     -- the subject's coordinate-last-half count. THE DISCRIMINANT.
#   PRIOR_ARM_WHOLE_RUN         -- the prior-duration arm, read from the committed, unmodified
#                                  prior ledger. PUBLISHED, NEVER THE DISCRIMINANT.
#   PRIOR_ARM_COORD_LAST_HALF   -- the prior-duration arm's coordinate-last-half count. THE
#                                  DISCRIMINANT's comparison term.
# NO RATE IS COMPUTED FROM A PARTIAL WINDOW: every PARTIAL row carries an EMPTY drains_per_hour and
#   names the exclusion in rate_excluded_reason. An empty rate here is a REFUSAL, not a zero.
# CAVEATS, PRE-DECLARED: the prior arm is n = 1 and each new arm is n = 1. THIS IS A COMPARISON OF
#   OBSERVATIONS, NOT AN INFERENCE. No test statistic is computed, no significance is claimed.
EOT
awk -F, -v OFS=, -v PRIORPR="$PRIOR_PR" -v PRIORNAME="$PRIOR" -v CELLNAME="$CELL" '
function load(file, arr_t, arr_nd,   line, i, nf, f, hdrline, idx_t, idx_nd, m) {
  m = 0
  while ((getline line < file) > 0) {
    nf = split(line, f, ",")
    if (m == 0 && idx_t == 0) { for (i=1;i<=nf;i++) { if (f[i]=="elapsed_secs") idx_t=i; if (f[i]=="topgun_or_prune_nonempty_drains_total") idx_nd=i } ; continue }
    m++ ; arr_t[m] = f[idx_t]+0 ; arr_nd[m] = f[idx_nd]+0
  }
  close(file)
  return m
}
BEGIN {
  OFS=","
  print "row_kind","source_ledger","w_index","window_kind","t_lo","t_hi","span_secs","cum_nonempty_drains_at_t_lo_boundary","cum_nonempty_drains_at_t_hi","d_nonempty_drains","drains_per_hour","discriminant_role","rate_excluded_reason"
}
NR==1 { for (i=1;i<=NF;i++) c[$i]=i; next }
{ n++ ; t[n]=$(c["elapsed_secs"])+0 ; nd[n]=$(c["topgun_or_prune_nonempty_drains_total"])+0 }
END {
  if (n < 2) exit 0
  t0 = t[1] ; tlast = t[n] ; kmax = int((tlast - t0) / 1000)
  base = 1 ; full_spans = 0 ; full_drains = 0 ; last_full = 0
  for (k = 0; k <= kmax; k++) {
    lo = t0 + 1000*k ; hinom = t0 + 1000*(k+1)
    rows = 0 ; last = 0
    for (i = 1; i <= n; i++) if (t[i] >= lo && t[i] < hinom) { rows++ ; last = i }
    if (rows == 0) continue
    kind = (hinom <= tlast) ? "FULL" : "PARTIAL"
    d = nd[last] - nd[base]
    if (kind == "FULL") {
      rate = sprintf("%.6f", d / 1000 * 3600) ; why = ""
      full_spans += 1000 ; full_drains += d ; last_full = last
    } else {
      rate = "" ; why = "PARTIAL window: R3 excludes it from every rate"
    }
    print "WINDOW", CELLNAME, k, kind, lo, t[last], (kind=="FULL" ? 1000 : t[last]-lo), nd[base], nd[last], d, rate, "n/a", why
    base = last
  }
  # coordinate last half of the subject
  tmid = (t[1] + t[n]) / 2 ; lo2 = 0
  for (i = 1; i <= n; i++) if (t[i] > tmid) { if (!lo2) lo2 = i ; hi2 = i }
  print "SUBJECT_WHOLE_RUN", CELLNAME, "", "", t[1], t[n], t[n]-t[1], nd[1], nd[n], nd[n]-nd[1], "", \
        "PUBLISHED_NEVER_THE_DISCRIMINANT", "span crosses the PARTIAL tail; R3 excludes PARTIAL from every rate"
  print "SUBJECT_FULL_WINDOWS", CELLNAME, "", "FULL", t0, (last_full ? t[last_full] : ""), full_spans, nd[1], (last_full ? nd[last_full] : ""), full_drains, (full_spans > 0 ? sprintf("%.6f", full_drains / full_spans * 3600) : ""), "PUBLISHED_NEVER_THE_DISCRIMINANT", ""
  if (lo2 > 1) print "SUBJECT_COORD_LAST_HALF", CELLNAME, "", "", t[lo2], t[hi2], t[hi2]-t[lo2-1], nd[lo2-1], nd[hi2], nd[hi2]-nd[lo2-1], \
        sprintf("%.6f", (nd[hi2]-nd[lo2-1]) / (t[hi2]-t[lo2-1]) * 3600), "THE_DISCRIMINANT", ""
  # prior-duration arm, read from its own committed ledger
  m = load(PRIORPR, pt, pnd)
  if (m >= 2) {
    ptmid = (pt[1] + pt[m]) / 2 ; plo = 0
    for (i = 1; i <= m; i++) if (pt[i] > ptmid) { if (!plo) plo = i ; phi = i }
    print "PRIOR_ARM_WHOLE_RUN", PRIORNAME, "", "", pt[1], pt[m], pt[m]-pt[1], pnd[1], pnd[m], pnd[m]-pnd[1], "", \
          "PUBLISHED_NEVER_THE_DISCRIMINANT", "startup-burst confound: the count is front-loaded and does not scale with duration"
    if (plo > 1) print "PRIOR_ARM_COORD_LAST_HALF", PRIORNAME, "", "", pt[plo], pt[phi], pt[phi]-pt[plo-1], pnd[plo-1], pnd[phi], pnd[phi]-pnd[plo-1], \
          sprintf("%.6f", (pnd[phi]-pnd[plo-1]) / (pt[phi]-pt[plo-1]) * 3600), "THE_DISCRIMINANT", ""
  }
}' "$PR"
} > "$T4"

# ------------------------------------------------------------------------------------------------
# T5 -- the EPOCH CONTENT FATE LEDGER, on the COORDINATE LAST HALF. The target ranked first.
# ------------------------------------------------------------------------------------------------
{
hdr "T5 -- EPOCH CONTENT FATE LEDGER (coordinate last half). DERIVED, ONE COLUMN OUT-OF-SCOPE (R3.3)."
cat <<'EOT'
# WINDOW: the COORDINATE LAST HALF -- the SAME window the R8.1 walk reads, so this ledger and the walk
#   describe the same rows. One row per epoch observed there as a topgun_or_prune_current_epoch value.
# WHAT THIS LEDGER ANSWERS: which content arrived, when the watermark passed it, whether anything ever
#   drained it, and what the prune attributed. WHAT IT DOES NOT ANSWER: whether that content was ever
#   in the drainable index -- that is T1(a), OUT-OF-SCOPE, owner TODO-634.
# THE COLUMN NAME CARRIES ITS OWN MARKER: indexed_refs_at_lwm_pass__AGGREGATE_NOT_PER_EPOCH is the
#   AGGREGATE index gauge at the first scrape at or after the LWM passed this epoch. It is OUT-OF-SCOPE
#   IN ITS PER-EPOCH FORM and is NOT presented as a per-epoch residency reading. No committed column,
#   and no derivation from committed columns, can attribute a ref to an epoch; producing that is a
#   per-epoch labelled emission on the prune path, a .rs change R0.4 forbids categorically.
# added_bytes IS SAMPLED. It is the delta of topgun_ormap_tombstone_bytes_total across the epoch's
#   currency window, measured from the row immediately PRECEDING the epoch's first scrape. At width
#   1000 an epoch is current for ~31 s ~ 3 scrapes, so boundary attribution carries up to ONE SCRAPE AT
#   EACH END => ~1/3 relative uncertainty. scrapes_in_epoch is published BESIDE it so the reader SEES
#   that uncertainty rather than having to infer it. When no preceding row exists the cell is EMPTY.
# was_drained_ever FOLLOWS A STATED INFERENCE RULE, AND THE ASSUMPTION IS VISIBLE HERE.
#   Drains are read over the FULL LEDGER (the question is "ever", not "in this window"). For a drain
#   bucket whose drain_epochs delta is k: when k == 1 the drained epoch is EXACTLY the bucket's
#   last_drained_epoch and the set is EXACT; when k > 1 THE DRAINED SET IS INFERRED AS THE k EPOCHS
#   ENDING AT last_drained_epoch, WHICH ASSUMES CONTIGUITY. Nothing in the committed surface proves
#   contiguity. Whether the assumption was exercised at all is readable from the d_drain_epochs_sum
#   column of the t2-drains table (SPEC-356b measured 1.000 epoch per drain, i.e. never exercised).
# bytes_freed_attributed IS NOT SPLIT WHEN k > 1: the drain's whole epoch_bytes_freed_sum delta is
#   published against EACH epoch of the inferred k-epoch set, because NO COMMITTED COLUMN LICENSES A
#   SPLIT. It is EMPTY for an epoch nothing ever drained.
# lwm_passed_at_t IS SAMPLED (C1): it is the bucket at which low_water_mark FIRST EXCEEDS this epoch,
#   i.e. the first scrape at or after the advance. EMPTY when the LWM never passed the epoch inside
#   this ledger -- an EMPTY here is "not observed", never "did not happen".
# T1(b) AND T5 ARE ONE OBSERVATION REACHED BY TWO ROUTES wherever current_epoch == low_water_mark: the
#   t1-lwmpass rows are a PROJECTION of these epoch rows, and agreement between the two tables is a
#   RESTATEMENT, NOT CORROBORATION. Whether that identity survives is published, not assumed, on the
#   EQUALITY_RATE line below -- if it breaks, the two tables become independent and the reader must be
#   told which regime is being read. THE EQUALITY RATE FOR THIS REPLICATE IS CARRIED HERE, IN THIS
#   HEADER COMMENT, AND NOWHERE ELSE.
EOT
awk -F, '
NR==1 { for (i=1;i<=NF;i++) c[$i]=i; next }
{ n++
  t[n]=$(c["elapsed_secs"])+0 ; ce[n]=$(c["topgun_or_prune_current_epoch"])+0
  lwm[n]=$(c["topgun_or_prune_low_water_mark"])+0 }
END {
  if (n < 1) exit 0
  tmid = (t[1] + t[n]) / 2
  ef = 0 ; eh = 0 ; nh = 0
  for (i = 1; i <= n; i++) {
    if (ce[i] == lwm[i]) ef++
    if (t[i] > tmid) { nh++ ; if (ce[i] == lwm[i]) eh++ }
  }
  printf "# EQUALITY_RATE current_epoch == low_water_mark: full_ledger=%d/%d=%.6f coordinate_last_half=%d/%d=%.6f\n", \
         ef, n, ef/n, eh, nh, (nh ? eh/nh : 0)
}' "$PR"
awk -F, -v OFS=, '
NR==1 { for (i=1;i<=NF;i++) c[$i]=i; next }
{ n++
  t[n]=$(c["elapsed_secs"])+0 ; ce[n]=$(c["topgun_or_prune_current_epoch"])+0
  lwm[n]=$(c["topgun_or_prune_low_water_mark"])+0 ; ir[n]=$(c["topgun_or_prune_indexed_refs"])+0
  tb[n]=$(c["topgun_ormap_tombstone_bytes_total"])+0
  nd[n]=$(c["topgun_or_prune_nonempty_drains_total"])+0
  des[n]=$(c["topgun_or_prune_drain_epochs_sum"])+0
  lde[n]=$(c["topgun_or_prune_last_drained_epoch"])+0
  ebs[n]=$(c["topgun_or_prune_epoch_bytes_freed_sum"])+0 }
END {
  if (n < 1) exit 0
  tmid = (t[1] + t[n]) / 2
  # drains over the FULL ledger -> the set of epochs some drain covered, and what it attributed
  for (i = 2; i <= n; i++) {
    if (nd[i] <= nd[i-1]) continue
    k = des[i] - des[i-1] ; if (k < 1) k = 1
    top = lde[i] ; att = ebs[i] - ebs[i-1]
    for (j = top - k + 1; j <= top; j++) { drained[j] = 1 ; attributed[j] = att }
  }
  print "epoch","t_first_seen","t_last_seen","scrapes_in_epoch","added_bytes","indexed_refs_at_lwm_pass__AGGREGATE_NOT_PER_EPOCH","was_drained_ever","bytes_freed_attributed","lwm_passed_at_t"
  order = 0
  for (i = 1; i <= n; i++) {
    if (t[i] <= tmid) continue
    e = ce[i]
    if (!(e in seen)) { seen[e] = 1 ; order++ ; seq[order] = e ; firstrow[e] = i }
    lastrow[e] = i ; scrapes[e]++
  }
  for (o = 1; o <= order; o++) {
    e = seq[o] ; fr = firstrow[e] ; lr = lastrow[e]
    ab = (fr > 1) ? tb[lr] - tb[fr-1] : ""
    pass_t = "" ; pass_ir = ""
    for (i = 1; i <= n; i++) if (lwm[i] > e) { pass_t = t[i] ; pass_ir = ir[i] ; break }
    drained_s = (e in drained) ? "true" : "false"
    att_s = (e in drained) ? attributed[e] : ""
    print e, t[fr], t[lr], scrapes[e], ab, pass_ir, drained_s, att_s, pass_t
  }
}' "$PR"
} > "$T5"

echo "targets built: $T1 $T2 $T3 $T4 $T5" >&2
