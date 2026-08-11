#!/bin/sh
# slottruth-v2.sh -- RE-DERIVE every re-derivable slot from the COMMITTED artifacts.
# usage: spec356c-slottruth-v2.sh <evidence-dir> [cell-basename] [duration-s] > truth.tsv
#   defaults: cell-basename = spec356-long, duration-s = 14400 -- with NO override this driver reads
#   EXACTLY the files v1 reads, which is what makes the run-1 / run-2 comparison a regression test.
# v2 differs from spec356-slottruth.sh (v1, UNEDITED, digest 280f7a34..f060) in three admitted ways and
# the published `diff -U0` classifies every hunk: the B fix (ADJ-12 -- the rendered p50 column is
# medianed over the NON-SENTINEL rows only, and the excluded fraction is emitted unconditionally beside
# it), the subject parameter above, and this comment block. NOTHING ELSE MOVED.
# It reads ONLY basenames this spec's Delta already commits and runs ONLY the pinned unforked fitter.
# EVERY WINDOW AND SPLIT BELOW IS ON A COORDINATE, NEVER ON A ROW INDEX, AND THE COORDINATE IS THE ONE
# PROVENANCE BINDS (ADJ-17 as adjudicated by ADJ-18, both governing):
#   last-half window = rows whose `elapsed_secs` EXCEEDS the FULL ledger's elapsed midpoint;
#   early/late split = THAT WINDOW's own `elapsed_secs` SPAN midpoint  (ADJ-18 clause 1);
#   the FIT axis is UNCHANGED -- every slope is still fitted on `current_epoch`.
# `elapsed_secs` is bound by limb (d)(ii) (cadence, span, matrix.txt identity); `current_epoch` is a
# MEASURED column with no independent binding, which is why a split defined on it was movable by a
# monotone forward stretch of ONE value (600/200 -> 436.8/32.7) and why ADJ-18 moved membership off it.
# DENSITY is guarded in BOTH directions at [0.8x,1.2x] of span/cadence, PER LEDGER *and* PER DERIVED
# HALF (ADJ-18 clause 3), and the EPOCH axis carries its own integrity bound -- non-decreasing, and
# every single-row jump <= 10 x the median positive jump (ADJ-18 clause 2).
# WHAT THESE LIMBS DO NOT CLAIM is stated ONCE, at R5.7(f) (ADJ-18 clause 4). Do not restate it here.
# A NONZERO exit means the truth file is INCOMPLETE BY CONSTRUCTION; the grader READS that status and
# REDs with it PRINTED, so a driver that REFUSED TO RUN is never reported as a mere coverage hole.
set -e
E=$1
FIT=$E/spec349c2-fit.awk
PR=$E/${2:-spec356-long}.prune.csv               # the cell's ledger -- THE ONLY PASS-RATE INPUT
PP=$E/${2:-spec356-long}-passrate.csv            # committed CONVENIENCE COPY of the parent series
LO=$E/${2:-spec356-long}-passrate-early.csv      # committed CONVENIENCE COPY of the early slice
HI=$E/${2:-spec356-long}-passrate-late.csv       # committed CONVENIENCE COPY of the late slice
# R5.1's arm B is CROSS-LINEAGE, and its two levels come from SPEC-355's OWN COMMITTED CSVs -- the
# width-1000 / 1800 s HEAD pair manifest §1.1 cites as `38,715 . 36,624`. Both are REAL basenames in
# this evidence dir (Delta, NOT MODIFIED). The phantom `spec355-head-r{1,2}.csv` this driver used to
# read EXISTED NOWHERE and is DELETED (Audit v12 C3).
H1=$E/spec355-sweep1000.csv                     # SPEC-355 HEAD w1000 replicate 1 -> §10.4.2's 38,715
H2=$E/spec355-sweep1000b.csv                    # SPEC-355 HEAD w1000 replicate 2 -> §10.4.2's 36,624
W=${TMPDIR:-/tmp}/slottruth.$$ ; mkdir -p "$W" ; trap 'rm -rf "$W"' EXIT
die() { echo "SLOTTRUTH FAIL: $*" >&2 ; exit 3 ; }

# ---- (d) PROVENANCE: a measurement cell's committed bytes must be ITS OWN ----
# Two independent bindings, because either alone is defeated by copying the other file too:
#  (i) the cell's `matrix.txt` self-description must name THIS cell, THIS width and THIS duration --
#      the runner's own emitted literals (`spec356-prune.sh:864`, `:882`, `:891`);
#  (ii) the DRIVER-CONSUMED csv's OWN `elapsed_secs` column must be STRICTLY INCREASING, must SPAN this
#      cell's duration to +-10 %, and must carry a row count inside `[0.8x, 1.2x]` of `span / cadence`
#      -- a TWO-SIDED density band (ADJ-17 as TIGHTENED by ADJ-18 clause 3). The upper side catches
#      PADDING (invented scrapes); the LOWER side catches THINNING, the channel Audit v12 C1 exploited.
#      [0.5x,1.5x] was too loose to be a bound on a HALF: it left a 50 % deletion budget inside one
#      half, so value-selective thinning still tilted that half's fit. The observed legitimate skip
#      rate is ~1 % (one gated tick per smoke), so 20 % headroom is coarse-generous.
#      THE SAME BAND IS ENFORCED ON EACH DERIVED HALF, below, where the halves exist.
prov() { # prov <basename> <cell> <width-literal> <duration-s> <driver-consumed csv> <cadence-s>
  MX=$E/$1.matrix.txt
  [ -f "$MX" ] || die "$1.matrix.txt is ABSENT: the cell cannot state whose bytes these are"
  grep -qxF "=== spec356 prune-record run: cell $2 ===" "$MX" || die "$1.matrix.txt does not name cell $2"
  grep -qxF "  TOPGUN_EPOCH_WIDTH:  $3" "$MX"                 || die "$1.matrix.txt width is not [$3]"
  grep -qxF "  duration:            $4s" "$MX"                || die "$1.matrix.txt duration is not $4 s"
  awk -F, -v d="$4" -v cad="$6" '
    NR==1 { for (i=1;i<=NF;i++) if ($i=="elapsed_secs") c=i; if (!c) exit 1; next }
    { v = $c + 0; if (n && v <= last) exit 1; if (!n) first = v; last = v; n++ }
    END { if (n < 2) exit 1
          span = last - first
          if (span < 0.9*d || span > 1.1*d) exit 1
          slots = span / cad
          if (n < 0.8*slots || n > 1.2*slots) exit 1 }' "$5" \
    || die "$5 is not cell $2's own ledger: its elapsed_secs must be strictly increasing, span $4 s to +-10%, and carry a row count within [0.8x,1.2x] of span/${6}s"
  # (iv) THE EPOCH AXIS IS GUARDED TOO, coarsely and on its own terms. `current_epoch` is the OLS
  # x-axis, and until this limb NOTHING constrained it. `topgun_or_prune_current_epoch` is the LWM
  # epoch, a MONOTONE COUNTER: it cannot decrease. That is a fact about the instrument, not a bound
  # fitted to data, which is why it is checkable PRE-DATA. The JUMP bound (ADJ-18 clause 2) is applied
  # over the PARENT WINDOW, below, which is the scope ADJ-18 gives it.
  awk -F, '
    NR==1 { for (i=1;i<=NF;i++) if ($i=="topgun_or_prune_current_epoch") c=i; if (!c) exit 0; next }
    { if (!c) next; v = $c + 0; if (n && v < last) exit 1; last = v; n++ }' "$5" \
    || die "$5's topgun_or_prune_current_epoch DECREASES: the LWM epoch is a monotone counter, so this column has been edited"
}
prov spec356-ctl-r1    ctl    "<unset: production default 1000>"  1800  "$E/spec356-ctl-r1.csv"    60
prov spec356-ctl-r2    ctl    "<unset: production default 1000>"  1800  "$E/spec356-ctl-r2.csv"    60
prov spec356-ctloff-r1 ctloff "<unset: production default 1000>"  1800  "$E/spec356-ctloff-r1.csv" 60
prov spec356-ctloff-r2 ctloff "<unset: production default 1000>"  1800  "$E/spec356-ctloff-r2.csv" 60
prov spec356-w100      w100   "100"                               1800  "$E/spec356-w100.csv"      60
prov "${2:-spec356-long}" long   "<unset: production default 1000>" "${3:-14400}" "$PR"     10
[ -f "$E/spec356-cellE.matrix.txt" ] && \
prov spec356-cellE     cellE  "<unset: production default 1000>"  1800  "$E/spec356-cellE.prune.csv" 10 || true

# ---- (d) PROVENANCE FOR ARM B, which is a FROZEN artifact of ANOTHER spec and has no spec356 matrix ----
# A SPEC-355 CSV cannot carry this runner's identity triple, so the binding is CONTENT: the same
# coordinate window, the same +-10 %/density bands, AND the derived level must equal the literal
# manifest §10.4.2 published for that replicate. A substituted file fails the last check by construction.
prov355() { # prov355 <csv> <duration-s> <cadence-s> <published-level-rounded>
  awk -F, -v d="$2" -v cad="$3" '
    NR==1 { for (i=1;i<=NF;i++) if ($i=="elapsed_secs") c=i; if (!c) exit 1; next }
    { v = $c + 0; if (n && v <= last) exit 1; if (!n) first = v; last = v; n++ }
    END { if (n < 2) exit 1
          span = last - first
          if (span < 0.9*d || span > 1.1*d) exit 1
          slots = span / cad
          if (n < 0.8*slots || n > 1.2*slots) exit 1 }' "$1" \
    || die "$1 is not a SPEC-355 1800 s control replicate (coordinates/density)"
  got=$(level "$1")
  awk -v g="$got" -v w="$4" 'BEGIN{ d=g-w; if (d<0) d=-d; exit (d<=1 ? 0 : 1) }' \
    || die "$1's coordinate-window level $got does not reproduce manifest §10.4.2's published $4"
}

# ---- THE COORDINATE WINDOWS, DERIVED FROM THE LEDGER. THE SLICES ARE NOT INPUTS. ----
# parent = rows of the FULL ledger with `elapsed_secs > (t_first + t_last)/2`, emitted as
#          (`current_epoch`, `passes_total`), zero exclusion (Delta / ADJ-13's series leg)
# early  = parent rows with `elapsed_secs <= (t_lo + t_hi)/2`, where t_lo/t_hi are the PARENT WINDOW's
#          own first and last elapsed_secs -- the midpoint of the WINDOW'S ELAPSED SPAN, not the median
#          row and NOT the epoch span (ADJ-18 clause 1). late = the rest.
# The emitted x column is STILL `current_epoch`: only MEMBERSHIP moved to elapsed, the FIT did not.
# Four further limbs run here, where the parent window and its halves exist. THE GUARDED AXIS IS
# CHECKED FIRST and the MEASURED axis last, which is the same precedence prov() already uses:
#   PER-HALF DENSITY (ADJ-18 clause 3) -- each derived half's row count within [0.8x,1.2x] of its OWN
#           span/cadence. This is what bounds value-selective thinning INSIDE one half, which the
#           ledger-global band cannot see. It bounds VOLUME.
#   PER-HALF MAX GAP (ADJ-19 clause 2) -- within each derived half the largest gap between consecutive
#           `elapsed_secs` values must be <= 5 x cadence. This bounds DISTRIBUTION, which is the half of
#           the problem a band cannot reach: any band leaves a budget, and a budget CONCENTRATED at the
#           split boundary tilts that half's fit while numerator and denominator shrink together (100
#           contiguous rows deleted below the boundary measured 1.004 x -- inside the band). A legitimate
#           skipped scrape is 2x and two consecutive skips 3x, against a measured ~1 % skip rate.
#   (d)(iv) EPOCH JUMP BOUND (ADJ-18 clause 2, GATED BY ADJ-19 clause 1) -- over the parent window,
#           every single-row jump in `current_epoch` must be <= 10 x the MEDIAN POSITIVE JUMP, and the
#           rule applies ONLY at >= 5 POSITIVE JUMPS. Below that the reference statistic is dominated by
#           the attacking row itself ({1, 1398} -> median 699.5, bound 6995, the 1398 jump passes its own
#           guard), so the epoch axis cannot carry a pass-rate fit at all and the window routes to
#           INDETERMINATE through R8.1 Step 0(c) -- the EXISTING hatch, no new verdict and no default to
#           either branch. A guard whose scale the attacker sets is not a guard.
#   TWO-VALUE FLOOR -- a slice that cannot support a fit is FAIL-CLOSED, not a thin fit. It counts
#           DISTINCT x-VALUES, not ROWS, and the difference is the whole point: the fit axis is
#           `current_epoch`, so a half with 360 ROWS and ONE distinct epoch has NO x-variance, the
#           fitter refuses it (`sxx <= 0`, exit 2) and NO slope exists. Counting rows cleared exactly
#           that half. A refused fit then reached the grader as an EMPTY slot, which the grader read as
#           0 -- fail-OPEN in the worst possible direction, and one-directional: an empty S_EARLY is
#           caught by the `S_EARLY = 0` arithmetic guard, an empty S_LATE was caught by nothing, and
#           the unguarded side is the one that yields SCHEDULING. BOTH are guarded now, on three
#           independent limbs: this floor, `slope()`'s status propagation, and the grader's own
#           empty-value RED. The regime is NOT adversarial -- `topgun_or_prune_current_epoch` is the
#           LWM epoch and R1.3a says LWM movement STOPS in the Step-3 (scheduling / LWM-stall) regime,
#           so a genuine late-half stall produces this ledger HONESTLY and the leaf it would publish is
#           the very leaf the stall is evidence for, read off a slope nobody computed. Such a window
#           routes to INDETERMINATE via R8.1 Step 0(c) -- the EXISTING hatch -- and the RAW OBSERVATION
#           (rows and distinct epoch counts per half) is printed with the refusal.
# The three committed CSVs are CONVENIENCE COPIES for a reader, never inputs: each is `cmp`ed against
# the derivation and any drift is RED. Every boundary is a pure function of a GUARDED COORDINATE, so no
# row count enters the verdict and there is no post-data choice left to make.
src=0
awk -F, -v P="$W/parent.csv" -v A="$W/early.csv" -v B="$W/late.csv" -v cad=10 '
  NR==1 { for (i=1;i<=NF;i++) h[$i]=i
          if (!(("elapsed_secs" in h) && ("topgun_or_prune_current_epoch" in h) \
                && ("topgun_or_prune_passes_total" in h))) exit 1
          next }
  { n++; t[n]=$(h["elapsed_secs"])+0
         e[n]=$(h["topgun_or_prune_current_epoch"]); p[n]=$(h["topgun_or_prune_passes_total"]) }
  END { if (n < 2) exit 1
        tmid = (t[1] + t[n]) / 2
        for (i = 1; i <= n; i++) if (t[i] > tmid) { m++; pt[m] = t[i]; pe[m] = e[i]; pp[m] = p[i] }
        if (m < 2) exit 2
        tlo = pt[1]; thi = pt[m]; tsplit = (tlo + thi) / 2
        for (i = 1; i <= m; i++) {
          if (pt[i] <= tsplit) { na++; if (na == 1) alo = pt[i]; ahi = pt[i] }
          else                 { nb++; if (nb == 1) blo = pt[i]; bhi = pt[i] }
        }
        if (na < 2 || nb < 2) exit 2
        # THE FLOOR IS ON DISTINCT x-VALUES, NOT ON ROWS -- and that is the whole content of the
        # repair. The OLS x-axis is `current_epoch`; a half carrying 360 rows and ONE distinct epoch
        # has NO x-variance, so the fitter refuses it (`sxx <= 0`) and no slope exists to publish. A
        # ROW-counting floor cleared such a half, which is exactly how a declined fit used to reach the
        # grader as an empty field. This is also the regime the predicate most expects rather than an
        # adversarial one: `topgun_or_prune_current_epoch` is the LWM epoch, and R1.3a states that LWM
        # movement STOPS in the R8.1 Step-3 (scheduling / LWM-stall) regime -- so a genuine stall
        # confined to the late half produces this ledger HONESTLY, and the leaf it would otherwise
        # publish is the very leaf the stall is evidence for, read off a slope nobody computed.
        for (i = 1; i <= m; i++) {
          if (pt[i] <= tsplit) { if (!(pe[i] in DA)) { DA[pe[i]] = 1; da++ } }
          else                 { if (!(pe[i] in DB)) { DB[pe[i]] = 1; db++ } }
        }
        if (da < 2 || db < 2) {
          printf "  OBSERVED: parent rows=%d; early rows=%d distinct current_epoch=%d; late rows=%d distinct current_epoch=%d\n", \
            m, na, da, nb, db > "/dev/stderr"
          exit 7
        }
        # Each consecutive-row gap is attributed to the half its EARLIER endpoint sits in, so the
        # BOUNDARY-CROSSING gap belongs to the early half. That is not a detail: a run deleted at the
        # EDGE of a half leaves a hole with no interior row on one side of it, and a gap sequence read
        # inside [alo, ahi] alone is blind to exactly the deletion ADJ-19 clause 2 names.
        for (i = 2; i <= m; i++) { g = pt[i] - pt[i-1]
          if (pt[i-1] <= tsplit) { if (g > agap) agap = g } else { if (g > bgap) bgap = g } }
        # PER-HALF DENSITY, each half against its OWN span (ADJ-18 clause 3) -- VOLUME
        sa = (ahi - alo) / cad; sb = (bhi - blo) / cad
        if (na < 0.8*sa || na > 1.2*sa) exit 3
        if (nb < 0.8*sb || nb > 1.2*sb) exit 3
        # PER-HALF MAX CONSECUTIVE GAP (ADJ-19 clause 2) -- DISTRIBUTION, which the band cannot see
        if (agap > 5 * cad || bgap > 5 * cad) exit 5
        # (d)(iv) EPOCH JUMP BOUND over the parent window (ADJ-18 clause 2), and its MINIMUM EVIDENCE
        # gate (ADJ-19 clause 1). The gate is evaluated FIRST, because a bound whose own scale comes
        # from too few samples is not a weaker bound -- it is no bound.
        np = 0
        for (i = 2; i <= m; i++) { j = pe[i] - pe[i-1]; if (j > 0) J[++np] = j }
        if (np < 5) exit 6
        for (a = 1; a < np; a++) for (b = 1; b < np; b++) \
          if (J[b] > J[b+1]) { z = J[b]; J[b] = J[b+1]; J[b+1] = z }
        medj = (np % 2) ? J[(np+1)/2] : (J[np/2] + J[np/2+1]) / 2
        for (i = 2; i <= m; i++) if (pe[i] - pe[i-1] > 10 * medj) exit 4
        hdr = "current_epoch,passes_total"
        print hdr > P; print hdr > A; print hdr > B
        for (i = 1; i <= m; i++) {
          print pe[i] "," pp[i] > P
          if (pt[i] <= tsplit) print pe[i] "," pp[i] > A
          else                 print pe[i] "," pp[i] > B
        } }' "$PR" || src=$?
case $src in
  0) ;;
  1) die "the ledger does not carry elapsed_secs + the two pass-rate columns, or has fewer than two rows" ;;
  2) die "the ADJ-18 elapsed-span midpoint left a slice with fewer than 2 rows: the split is unusable and no slope may be read from it" ;;
  3) die "a DERIVED HALF is outside the ADJ-18 density band [0.8x,1.2x] of its OWN span/${cad:-10}s: value-selective thinning inside one half is refused, and the ledger-global band cannot see it" ;;
  4) die "$PR's topgun_or_prune_current_epoch carries a single-row JUMP greater than 10x the window's median positive jump (ADJ-18 clause 2): the epoch axis has been stretched" ;;
  5) die "a DERIVED HALF carries a gap between consecutive elapsed_secs values greater than 5x the ${cad:-10}s cadence (ADJ-19 clause 2): deletion CONCENTRATED at one point is refused, and the per-half density band -- which bounds VOLUME, not DISTRIBUTION -- reads such a half as in-band" ;;
  6) die "$PR's topgun_or_prune_current_epoch carries FEWER THAN FIVE positive jumps over the parent window (ADJ-19 clause 1): the 10x-median bound's own reference statistic would be set by the attacking row, so the epoch axis cannot carry a pass-rate fit and this window routes to INDETERMINATE via R8.1 Step 0(c) -- the EXISTING hatch, no new verdict" ;;
  7) die "a DERIVED HALF's topgun_or_prune_current_epoch carries FEWER THAN TWO DISTINCT VALUES -- the epochs are STATIC across that half (the raw counts are on the OBSERVED line above, which is the LWM-STALL reading R1.3a names). The OLS x-axis has no variance there, so NO slope exists and none may be fabricated: this window routes to INDETERMINATE via R8.1 Step 0(c) -- the EXISTING hatch, no new verdict and no default to either branch" ;;
  *) die "the coordinate split derivation failed with status $src" ;;
esac
cmp -s "$W/parent.csv" "$PP" || die "committed passrate parent != the ledger's coordinate last half"
cmp -s "$W/early.csv"  "$LO" || die "committed early slice != the ADJ-18 ELAPSED-SPAN MIDPOINT split"
cmp -s "$W/late.csv"   "$HI" || die "committed late slice  != the ADJ-18 ELAPSED-SPAN MIDPOINT split"

# ---- s_early / s_late: ADJ-15's PINNED INVOCATION, VERBATIM, run TWICE (R6a) on the DERIVED slices ----
# THE FITTER'S EXIT STATUS IS PROPAGATED, NEVER SWALLOWED, AND THE FUNCTION EMITS ITS OWN SLOT LINE.
# It used to run the fitter inside `$( ... | tr | awk )` and hand the result to `printf`: a command
# substitution's failure is invisible to `set -e` in that position, so a fitter REFUSAL (exit 2 -- e.g.
# `zero variance in elapsed_secs over the window`, which is what a half with ONE distinct x value
# produces) printed to stderr, returned an EMPTY field, and the driver emitted `S_LATE<TAB>` with
# exit 0. A statistic the fitter DECLINED to compute is not zero. So the fit runs in THIS shell, ANY
# nonzero status on a REQUIRED fit is a refusal, and so is an empty field.
slope() { # slope <csv> <half> <slot-name>
  awk -v col=passes_total -v xaxis=current_epoch -v window=full -f "$FIT" "$1" > "$W/fit.out" \
    || die "FIT REFUSED: $2"
  _v=$(tr ' ' '\n' < "$W/fit.out" | awk -F= '$1=="slope_per_x_unit"{print $2}')
  [ -n "$_v" ] || die "FIT REFUSED: $2"
  printf '%s\t%s\n' "$3" "$_v"
}
slope "$W/early.csv" early S_EARLY
slope "$W/late.csv"  late  S_LATE
# ---- exit share and median(L)/B, over the SAME ADJ-17 COORDINATE last-half window ----
# Exit share numerator = the five non-`Dropped` exit counters (manifest §5.3); denominator = considered.
# Both are CUMULATIVE totals, so the last-half share is the DELTA across the window.
# B is the rendered p50 column (col 43, ADJ-12); median(L) is the median of the eligible_refs gauge.
awk -F, '
NR==1 { for (i=1;i<=NF;i++) h[$i]=i; next }
{ n++; ts[n]=$(h["elapsed_secs"])+0; for (i=1;i<=NF;i++) v[n,i]=$i }
END {
  tmid = (ts[1] + ts[n]) / 2                     # ADJ-17: the window is a COORDINATE, not a row index
  for (i = 1; i <= n; i++) if (ts[i] > tmid) { if (!lo) lo = i; hi = i }
  if (!lo || lo == hi) exit 0
  C="topgun_or_prune_considered_total"; MN="topgun_or_prune_matched_nothing_total"
  AB="topgun_or_prune_absent_total"; RR="topgun_or_prune_restored_read_error_total"
  RE="topgun_or_prune_restored_evicted_total"; RW="topgun_or_prune_restored_write_error_total"
  L ="topgun_or_prune_eligible_refs";  B="topgun_or_prune_drain_refs_p50"
  dC = v[hi,h[C]] - v[lo,h[C]]
  dX = (v[hi,h[MN]]-v[lo,h[MN]]) + (v[hi,h[AB]]-v[lo,h[AB]]) + (v[hi,h[RR]]-v[lo,h[RR]]) \
     + (v[hi,h[RE]]-v[lo,h[RE]]) + (v[hi,h[RW]]-v[lo,h[RW]])
  if (dC > 0) printf "EXIT_SHARE_PCT\t%.6f\n", 100*dX/dC
  m=0; k=0                                     # m = ALL rows (median(L)); k = NON-SENTINEL rows (B)
  for (i=lo;i<=hi;i++) { m++; ls[m]=v[i,h[L]]+0
                         if (v[i,h[B]]+0 > 0) { k++; bs[k]=v[i,h[B]]+0 } }
  # The two populations differ in size now, so the ONE shared sort loop v1 used cannot bound both.
  for (i=1;i<m;i++) for (j=1;j<m;j++) if (ls[j]>ls[j+1]) { t=ls[j];ls[j]=ls[j+1];ls[j+1]=t }
  for (i=1;i<k;i++) for (j=1;j<k;j++) if (bs[j]>bs[j+1]) { t=bs[j];bs[j]=bs[j+1];bs[j+1]=t }
  med = (m%2) ? ls[(m+1)/2] : (ls[m/2]+ls[m/2+1])/2  # ADJ-2 aggregator over ALL rows -- UNCHANGED
  # ADJ-12: B is the median of the rendered p50 column with the 0-SENTINEL rows EXCLUDED, and the
  # excluded fraction is published WITH it, unconditionally -- a B without it is incomplete even at 0 %.
  # The >50 % hatch is NOT implemented here: this driver REPORTS, the predicate DECIDES.
  if (k == 0) {
    # 100 % sentinel: B is a median over an EMPTY set. 0 is the sentinel value itself, so emitting 0
    # would publish the sentinel as the estimate; and a bare suppression is indistinguishable from a
    # miss, which is the defect this lineage has already paid for once.
    printf "B\tUNDEFINED (100 %% sentinel, 0 non-sentinel rows)\n"
    printf "B_SENTINEL_EXCLUDED_PCT\t%.6f\n", 100
    printf "MEDIAN_L_OVER_B\twithheld: 0 non-sentinel rows; B undefined\n"
  } else {
    bb = (k%2) ? bs[(k+1)/2] : (bs[k/2]+bs[k/2+1])/2
    printf "B\t%.6f\n", bb
    printf "B_SENTINEL_EXCLUDED_PCT\t%.6f\n", 100*(m-k)/m
    printf "MEDIAN_L_OVER_B\t%.6f\n", med/bb
  }
}' "$PR"

# ---- the control arms: per-replicate coordinate-window level, then §1.1's pooled sd and grand mean ----
# STATED REDUCTION (cite it, do not re-derive it per run): each replicate contributes ONE level -- the
# mean of its `tombstone_bytes` column over the ADJ-17 coordinate last-half window, EMPTY CELLS SKIPPED
# exactly as the pinned fitter skips them. That is SPEC-355 §10.4.2's own statistic, and running it on
# SPEC-355's two committed HEAD CSVs returns 38715.466667 and 36624.133333 -- i.e. it REPRODUCES the
# `38,715 . 36,624` pair manifest §1.1 publishes, from the real bytes, without being told the answer.
# `s_pooled` is the two-sample pooled sd over the comparison's two arms at n1 = n2 = 2, so
# s_pooled = sqrt((s1^2 + s2^2)/2); the percentage denominator is the grand mean of the four levels.
level() { awk -F, '
  NR==1 { for (i=1;i<=NF;i++) h[$i]=i; next }
  { n++; ts[n]=$(h["elapsed_secs"])+0; vs[n]=$(h["tombstone_bytes"]) }
  END { tmid=(ts[1]+ts[n])/2
        for (i=1;i<=n;i++) if (ts[i] > tmid && vs[i] !~ /^[ \t]*$/) { s+=vs[i]+0; m++ }
        if (m) printf "%.6f\n", s/m }' "$1"; }
# THE CONTROL'S VERDICT IS RE-DERIVED TOO, and it is one line of arithmetic away from the numbers this
# function already computes. It was the LAST un-re-derived byte in the block AND the byte that decides
# whether a classification may be published at all: R5.4's 2x2 turns an R5.2 rejection into
# "No classification number may be read off any run". manifest §1.1 pre-registers the comparison as an
# EXECUTABLE formula -- two-sample t, df = 2, two-sided alpha = 0.05, CRITICAL VALUE 4.303 -- so
# emitting the verdict here is EXECUTING the frozen pre-registration, not editing it. At n1 = n2 = 2
# the standard error is sp * sqrt(1/n1 + 1/n2) = sp, which is why the statistic is |ma - mb| / sp.
# `_T` is a DIAGNOSTIC, not a slot: the template publishes no t, so the grader records it and prints
# it in the refusal rather than comparing it to anything.
arms() { # arms <tag> <armA-r1> <armA-r2> <armB-r1> <armB-r2>
  tag=$1; a1=$(level "$2"); a2=$(level "$3"); b1=$(level "$4"); b2=$(level "$5")
  awk -v t="$tag" -v a1="$a1" -v a2="$a2" -v b1="$b1" -v b2="$b2" 'BEGIN{
    ma=(a1+a2)/2; mb=(b1+b2)/2
    va=(a1-ma)^2+(a2-ma)^2; vb=(b1-mb)^2+(b2-mb)^2      # each arm: (n-1)s^2 with n = 2
    sp=sqrt((va+vb)/2)                                   # pooled over df = n1+n2-2 = 2
    d=ma-mb; if (d<0) d=-d
    tt = (sp > 0) ? d/sp : (d > 0 ? 1e9 : 0)             # sp = 0 with a real gap is INFINITE evidence
    printf "%s_SD\t%.6f\n%s_MEAN\t%.6f\n", t, sp, t, (a1+a2+b1+b2)/4
    printf "%s_T\t%.6f\n%s_RESULT\t%s\n", t, tt, t, (tt > 4.303 ? "REJECTED" : "NOT REJECTED") }'; }
prov355 "$H1" 1800 60 38715
prov355 "$H2" 1800 60 36624
# R5.1 is CROSS-LINEAGE: this build's `ctl` pair against SPEC-355's committed HEAD pair.
arms R51 "$E/spec356-ctl-r1.csv" "$E/spec356-ctl-r2.csv" "$H1" "$H2"
# R5.2 is WITHIN-LINEAGE: this build's `ctl` pair against its own `ctloff` pair.
arms R52 "$E/spec356-ctl-r1.csv" "$E/spec356-ctl-r2.csv" "$E/spec356-ctloff-r1.csv" "$E/spec356-ctloff-r2.csv"

# ---- the declined n = 6 extension's cell count (ADJ-10: 8 further control cells at 1800 s) ----
printf 'N6_COST_CELLS\t8\n'
