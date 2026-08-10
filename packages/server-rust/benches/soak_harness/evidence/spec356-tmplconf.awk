# tmplconf.awk -- §9 controls-and-dynamics TEMPLATE CONFORMANCE + RE-DERIVATION (checklists 15/16/18/19).
# usage: awk -v ev=<evidence-dir> -v tol=0.005 -f tmplconf.awk skeleton.txt submitted.md   <- REAL RUNS
#        awk -v truth=truth.tsv  -v tol=0.005 -f tmplconf.awk skeleton.txt submitted.md   <- FIXTURE MODE ONLY
#   (a) [item 15] the block between the markers IS the pinned skeleton, byte-exact modulo slots
#   (b) [item 16] every slot value is well-formed against its declared grammar
#   (c) [item 18] every RE-DERIVABLE slot equals the value re-derived from the COMMITTED artifacts,
#                 and the slots that are arithmetic functions of other slots agree with them
#   (d) [item 19] the PUBLISHED leaf equals the leaf the frozen R8.1 predicate implies
# NO ENGLISH IS SCANNED. Prose outside the block is NON-NORMATIVE and is never read.
# `truth` is REQUIRED whenever the cell has run. Item 18 REDs on a MISSING or INCOMPLETE truth file
# and NAMES the ungraded slots; item 19 has NO fallback to the published slots and REDs "cannot
# reconcile" instead. The ONLY vacuous path is `-v nosubject=1` WITH no truth file -- the operator
# explicitly asserting the cell has not run -- and that assertion is itself FAIL-CLOSED: a block that
# publishes any leaf but INDETERMINATE contradicts it and REDs.
# `ev=` IS THE MANDATED MODE FOR A REAL RUN, and it exists because `truth=` is an OPAQUE PATH: a
# hand-written nine-name truth file whose values are copied from the block certifies that block
# against ITSELF -- the identical property the deleted item-19 fallback had, mediated by a file the
# executor authors. Under `ev=` the grader RUNS the driver itself, so `TR` is this run's own
# re-derivation from the committed artifacts and nothing else. The mode is PRINTED, so a fixture-mode
# run can never be read as a re-derived one. (Cross-vendor round, Response v11.)
# BUT `ev=` WAS ITSELF AN OPAQUE PATH UNTIL Audit v12 C2 MEASURED IT, and two limbs close that:
#   (1) THE DRIVER IS PINNED BY DIGEST. `slottruth.sh` in $PWD was whatever file happened to be there:
#       `printf '#!/bin/sh\ncat ../forged.tsv\n' > slottruth.sh` bought PASS on the a1 transposition.
#       The grader now VERIFIES sha256(slottruth.sh) AGAINST DRIVER_SHA256 BEFORE EXECUTING IT, and a
#       mismatch REDs with both digests printed. The digest is also pinned in checklist 18's prose, so
#       an executor extracting the two programs has a check that survives every future line-range move.
#   (2) `ev` IS QUOTED, AND CHARSET-GUARDED. It used to be interpolated RAW into a shell command, so
#       `-v ev='/dev/null; cat forged.tsv #'` was a command, not a path, and printed PASS. It is now
#       single-quoted in the command string AND rejected up front if it carries anything outside
#       [A-Za-z0-9._/-] (which excludes the quote character itself, so the quoting cannot be escaped).
# THE DRIVER'S EXIT STATUS IS READ, NOT DISCARDED (Audit v12 rec 5). A driver that REFUSED TO RUN
# (`SLOTTRUTH FAIL`, exit 3) used to be indistinguishable in this transcript from a truth file with a
# coverage hole -- both printed "INCOMPLETE". A nonzero status now prints its own FAIL 18 line.
# The status is carried by an APPENDED `echo SLOTTRUTH_EXIT $?` rather than by `close()`, because
# `close()` on a command pipe returns 0 on this platform's awk (`awk version 20200816`, measured) --
# reading it would have been a check that silently never fires. The sentinel is portable and is
# EXEMPT from the slot-name check, and its ABSENCE is itself RED (the pipe died before the echo).
# (The former header said a missing truth file made 18/19 report NO SUBJECT "rather than passing
# vacuously". Audit v11 C2 MEASURED that FALSE for item 19 and half-false for 18. It is gone.)
function grammar(name) {
  if (name ~ /_RESULT$/)             return "ENUM_RESULT"
  if (name ~ /_FIELD$/)              return "ENUM_FIELD"
  if (name == "CELLE_DISPOSITION")   return "ENUM_CELLE"
  if (name == "STEP_LEAF")           return "ENUM_LEAF"
  return "REAL"
}
function wellformed(name, v,   g) {
  g = grammar(name)
  if (v ~ /\{\{/) return 0
  if (g == "REAL")        return (v ~ /^-?[0-9]+(\.[0-9]+)?$/)
  if (g == "ENUM_RESULT") return (v == "NOT REJECTED" || v == "REJECTED")
  # The wrong-axis BELT (Audit v10 C2): the ONLY admissible source field for a per-epoch slope is the
  # renamed one an explicit -v xaxis= produces. A time-axis fit reports `slope_mb_per_hour`, which this
  # rejects -- restoring structurally the field-name evidence the template's bare numeral had stripped.
  if (g == "ENUM_FIELD")  return (v == "slope_per_x_unit")
  if (g == "ENUM_CELLE")  return (v == "CLOSED-NOT-NEEDED" || v == "RUN-NON-REPRODUCTION-RECORDED-AND-SPUN-OFF" \
                               || v == "RUN-REPRODUCTION-WIDENS-CLAIM" || v == "RUN-INDETERMINATE")
  if (g == "ENUM_LEAF")   return (v == "SELECTION / FRONTIER (exit limb)" \
                               || v == "SELECTION / FRONTIER (licensing limb)" \
                               || v == "SCHEDULING / LICENSING" || v == "THROUGHPUT" \
                               || v == "Steps 3–4, NOT SEPARABLE (degenerate pass rate)" \
                               || v == "INDETERMINATE")
  return 0
}
# PRESENTATION-ONLY normalization, applied to BOTH sides.
function norm(x) {
  sub(/^([[:space:]]*>)*[[:space:]]*/, "", x)
  sub(/[[:space:]]+$/, "", x)
  return x
}
function conform(s, t,   rest, lit, name, nextlit, p, val, st, sl) {
  rest = t
  while (1) {
    st = match(s, /\{\{[A-Z_0-9]+\}\}/)
    if (st == 0) break
    sl   = RLENGTH
    lit  = substr(s, 1, st - 1)
    name = substr(s, st + 2, sl - 4)
    s    = substr(s, st + sl)
    if (substr(rest, 1, length(lit)) != lit) return 0
    rest = substr(rest, length(lit) + 1)
    if (match(s, /\{\{[A-Z_0-9]+\}\}/)) nextlit = substr(s, 1, RSTART - 1)
    else                                nextlit = s
    if (nextlit == "") { val = rest; rest = "" }
    else {
      p = index(rest, nextlit)
      if (p == 0) return 0
      val  = substr(rest, 1, p - 1)
      rest = substr(rest, p)
    }
    nslot++; sname[nslot] = name; sval[nslot] = val; sidx[name] = nslot
  }
  return (rest == s)
}
function has(n) { return (n in sidx) }
function V(n)   { return sval[sidx[n]] + 0 }
function SV(n)  { return sval[sidx[n]] }
# relative agreement; an exact-zero expectation falls back to absolute
function agree(got, want,   d) {
  d = got - want; if (d < 0) d = -d
  if (want < 0 ? -want : want) return (d <= tol * (want < 0 ? -want : want))
  return (d <= tol)
}
function red18(msg) { printf "FAIL 18 -- %s\n", msg; bad18 = 1; rc = 1 }
function red19(msg) { printf "FAIL 19 -- %s\n", msg; bad19 = 1; rc = 1 }
# ONE reader for BOTH modes, so the `ev=` and `truth=` paths cannot drift apart -- they were duplicated
# blocks, and a belt added to one of them would have been a belt on one mode only.
# THREE things it does that the duplicated inline loops did not:
#  (i)  AN EMPTY VALUE IS RED, NOT `0`. `tv = a[2] + 0` coerced a missing field to zero, `agree(0,0)`
#       took the absolute branch and passed, and `leafof(...,0)` returned SCHEDULING / LICENSING off a
#       slope the fitter had DECLINED to compute. This is the BELT for that: the driver now refuses
#       first (`FIT REFUSED`), and if it ever did not, an empty cell REDs here on its own limb.
#  (ii) ENUM slots are compared AS STRINGS. The re-derivation now carries the R5.1/R5.2 verdicts, and
#       `"NOT REJECTED" + 0` is `0` for every enum value -- a numeric comparison would grade nothing.
#  (iii) `_T` names are DIAGNOSTICS with no published counterpart and are recorded, not graded.
function take(ln,   a, tn, tvs, tv, pfx) {
  if (ln ~ /^#/ || ln ~ /^[[:space:]]*$/) return
  split(ln, a, "\t")
  tn = a[1]; tvs = a[2]
  if (tn ~ /_T$/) { TT[tn] = tvs; return }
  if (!has(tn)) { red18(sprintf("truth names slot %s, which the template does not carry", tn)); return }
  if (tvs ~ /^[ \t]*$/) {
    red18(sprintf("the re-derivation returned an EMPTY VALUE for slot %s: a statistic the derivation DECLINED to compute is NOT zero, and the `+0` coercion that used to make it one is the worst available fail-OPEN. The slot stays UNGRADED, and that is RED.", tn))
    return
  }
  if (grammar(tn) == "ENUM_RESULT") {
    TR[tn] = tvs; nt18++
    pfx = tn; sub(/_RESULT$/, "", pfx); pfx = pfx "_T"
    if (SV(tn) != tvs)
      red18(sprintf("slot %s = '%s' CONTRADICTS the verdict RE-DERIVED from the committed control CSVs = '%s' (two-sample t = %s against manifest §1.1's pre-registered critical value 4.303, df = 2)", \
            tn, SV(tn), tvs, (pfx in TT ? TT[pfx] : "<not emitted>")))
    return
  }
  tv = tvs + 0; TR[tn] = tv; nt18++
  if (!agree(V(tn), tv))
    red18(sprintf("slot %s = %s CONTRADICTS the value re-derived from the committed artifacts = %.6f", \
          tn, SV(tn), tv))
}
# R5.4's 2x2 as a pure function, so the PUBLISHED licence and the RE-DERIVED licence are computed by
# the SAME code and a difference between them is a difference in the inputs, never in the reading.
function licof(r51, r52) {
  if (r52 != "REJECTED") return "OK"
  return (r51 == "REJECTED") ? "INDETERMINATE - both suspected" : "INSTRUMENT PERTURBATION"
}
# The FROZEN R8.1 ordered predicate, as a pure function. FIRST match wins, exactly as the table states.
function leafof(share, lb, se, sl) {
  if (share > 10)      return "SELECTION / FRONTIER (exit limb)"
  if (lb <= 1)         return "SELECTION / FRONTIER (licensing limb)"
  if (se < 5)          return "Steps 3–4, NOT SEPARABLE (degenerate pass rate)"
  if (sl <= 0.5 * se)  return "SCHEDULING / LICENSING"
  return "THROUGHPUT"
}
BEGIN { if (tol + 0 <= 0) tol = 0.005; NSLOT = 19
        # THE DRIVER, PINNED BY CONTENT. This is sha256 of `slottruth.sh` as extracted from THIS spec's
        # own bytes by checklist 18's stated extraction command. A digest survives an edit that moves a
        # line range; a line range does not. Re-pinning the driver means re-pinning this constant and
        # the two digests checklist 18 prints -- there is no third place to forget.
        DRIVER_SHA256 = "280f7a3466a0bffc89059815bb3862bb9581cb95d7efa3fe58b1152749b8f060" }
FNR == NR && FILENAME == ARGV[1] { S[++ns] = norm($0); next }
/TG356B-CTRL BEGIN v2/ { nbeg++; inb = 1; next }
/TG356B-CTRL END v2/   { nend++; inb = 0; next }
inb { T[++nt] = norm($0) }
END {
  if (nbeg != 1 || nend != 1) {
    printf "FAIL 15 -- template markers: expected exactly one BEGIN and one END, got %d/%d%s\n", \
      nbeg + 0, nend + 0, (nbeg + 0 == 0 ? "  (TEMPLATE ABSENT -- the silent-drop failure mode)" : "")
    print "FAIL 16 -- slots ungradeable: no single template block"
    print "FAIL 18 -- slots ungradeable: no single template block"
    print "FAIL 19 -- leaf ungradeable: no single template block"
    exit 1
  }
  bad = 0
  if (nt != ns)
    printf "NOTE   -- block has %d lines, skeleton has %d: a REFLOWED or re-wrapped template cascades.\n         Re-paste the skeleton and fill slots only.\n", nt, ns
  m = (ns > nt ? ns : nt)
  for (i = 1; i <= m; i++) {
    if (i > nt) { printf "FAIL 15 -- template TRUNCATED at line %d (skeleton %d lines, block %d)\n", i, ns, nt; bad = 1; break }
    if (i > ns) { printf "FAIL 15 -- block carries EXTRA line %d beyond the skeleton: %s\n", i, T[i]; bad = 1; break }
    if (!conform(S[i], T[i])) {
      ndiff++
      if (ndiff <= 5) {
        printf "FAIL 15 -- line %d differs from the pinned skeleton\n", i
        printf "           skeleton: %s\n", S[i]
        printf "           block:    %s\n", T[i]
      }
      bad = 1
    }
  }
  if (ndiff > 5) printf "FAIL 15 -- (+%d further differing lines suppressed; %d differ in total)\n", ndiff - 5, ndiff
  if (bad) rc = 1
  else print "PASS 15 -- block is the pinned skeleton, byte-exact modulo slots"

  nb = 0
  for (k = 1; k <= nslot; k++)
    if (!wellformed(sname[k], sval[k])) {
      printf "FAIL 16 -- slot %s = %s is not well-formed (grammar %s)\n", \
        sname[k], (sval[k] == "" ? "<empty>" : "'" sval[k] "'"), grammar(sname[k])
      nb = 1; rc = 1
    }
  if (nslot != NSLOT && !bad) { printf "FAIL 16 -- expected %d slots, matched %d\n", NSLOT, nslot; nb = 1; rc = 1 }
  if (!nb && !bad) printf "PASS 16 -- all %d slots well-formed\n", nslot

  # ---- items 18 + 19 need well-formed slots to mean anything ----
  if (bad || nb) { print "SKIP 18 -- not attempted: the block is not a well-formed filled skeleton"
                   print "SKIP 19 -- not attempted: the block is not a well-formed filled skeleton"; exit rc }

  # ---- item 18(a): RE-DERIVATION against the committed artifacts ----
  # THE PINNED REQUIRED SET. These ELEVEN slots are re-derivable from committed artifacts, and the set
  # is enumerated HERE, BY NAME, so coverage is a MEASUREMENT and not a count. A truth file omitting any
  # of them leaves that slot UNGRADED -- the hole Audit v11 C2 measured, where ONE irrelevant slot
  # bought `PASS 18 -- all 1 re-derivable slots reproduce` on a transposition forgery.
  # `R51_RESULT` / `R52_RESULT` JOINED THE SET at the round that closed Audit v13 C2: the two enum
  # slots that DECIDE THE LICENCE were the last pair outside it, on artifacts that need not be forged
  # at all for the block to contradict them.
  nreq = split("S_EARLY S_LATE EXIT_SHARE_PCT MEDIAN_L_OVER_B R51_SD R51_MEAN R51_RESULT R52_SD R52_MEAN R52_RESULT N6_COST_CELLS", REQ, " ")
  nt18 = 0
  if (ev != "" && truth != "") {
    red18("both `ev=` and `truth=` were supplied: there is exactly ONE source of truth, and choosing between two is the degree of freedom this item exists to remove")
  } else if (ev != "") {
    # (2) CHARSET GUARD, evaluated BEFORE anything is executed: an `ev` carrying a shell metacharacter
    # is a command, not an evidence directory, and Audit v12 C2 executed one.
    if (ev ~ /[^A-Za-z0-9._\/-]/) {
      red18(sprintf("`ev=%s` is not a plain path: it carries a character outside [A-Za-z0-9._/-] and would be interpolated into a shell command. REFUSED before execution.", ev))
    } else {
      # (1) DIGEST GATE, also BEFORE execution: verify the driver is THE driver this spec pins.
      dcmd = "shasum -a 256 slottruth.sh 2>/dev/null | awk '{print $1}'"
      dsha = ""; dcmd | getline dsha; close(dcmd)
      if (dsha != DRIVER_SHA256) {
        red18(sprintf("slottruth.sh in $PWD is NOT the pinned driver: sha256 %s != DRIVER_SHA256 %s. The re-derivation was NOT run. Extract the driver from this spec's own bytes (checklist 18) before grading.", \
              (dsha == "" ? "<absent/unreadable>" : dsha), DRIVER_SHA256))
      } else {
        print "MODE   -- item 18/19 truth: RE-DERIVED BY THIS RUN from " ev " (sh slottruth.sh '" ev "')"
        print "MODE   -- driver sha256 " dsha " == DRIVER_SHA256 (verified BEFORE execution)"
        cmd = "sh slottruth.sh '" ev "'; echo SLOTTRUTH_EXIT $?"
        dst = -1
        while ((cmd | getline ln) > 0) {
          if (ln ~ /^SLOTTRUTH_EXIT /) { split(ln, z, " "); dst = z[2] + 0; continue }
          take(ln)
        }
        # THE STATUS IS READ. A REFUSAL AND A COVERAGE HOLE ARE DIFFERENT FACTS (Audit v12 rec 5).
        close(cmd)
        if (dst != 0)
          red18(sprintf("the re-derivation driver EXITED %d -- it REFUSED to derive (its `SLOTTRUTH FAIL:` line is on stderr). This is a REFUSAL, not a coverage hole: nothing was re-derived, and the MISSING list below is a CONSEQUENCE of the refusal, not an independent finding.", dst))
      }
    }
  } else if (truth != "") {
    print "MODE   -- item 18/19 truth: FIXTURE (`truth=" truth "`) -- PROVENANCE NOT ESTABLISHED, not admissible for a real §9"
    while ((getline ln < truth) > 0) take(ln)
    close(truth)
  }
  nmiss = 0; miss = ""
  for (q = 1; q <= nreq; q++)
    if (!(REQ[q] in TR)) { miss = miss (nmiss++ ? ", " : "") REQ[q] }
  # `nosubject=1` is the ONE vacuous path, and it is an ASSERTION: the cell has not run, so no committed
  # artifact exists to re-derive FROM. It requires the truth file to be absent ENTIRELY -- a partially
  # derived cell is an INCOMPLETE derivation, never a missing subject.
  nosub = (truth == "" && ev == "" && nosubject + 0 == 1 && nmiss == nreq)
  # THE ASSERTION IS FAIL-CLOSED. A cell that has not run cannot have published a classification, so
  # `nosubject=1` beside any leaf but INDETERMINATE is a self-contradiction and REDs (cross-vendor round:
  # an unchecked assertion is an escape hatch an executor can take AFTER seeing a disfavourable leaf).
  if (nosub && has("STEP_LEAF") && SV("STEP_LEAF") != "INDETERMINATE") {
    red18(sprintf("`-v nosubject=1` asserts the cell has NOT RUN, but the block publishes STEP_LEAF = '%s': a cell that has not run cannot have read a classification off any run", SV("STEP_LEAF")))
    nosub = 0; contra = 1
  }
  if (truth == "" && ev == "" && !nosub && !contra)
    red18("NO source of truth supplied (`ev=` absent, `truth=` absent) and `-v nosubject=1` was not asserted: all " nreq " re-derivable slots are UNGRADED. There is no vacuous pass here -- re-derive, or assert the cell has not run.")
  else if (!nosub && nmiss > 0)
    red18(sprintf("truth file is INCOMPLETE -- %d of %d re-derivable slots are MISSING and therefore UNGRADED: %s", \
          nmiss, nreq, miss))

  # ---- item 18(b): cross-slot arithmetic (independent of the truth file) ----
  if (has("S_RATIO") && has("S_EARLY") && has("S_LATE")) {
    if (V("S_EARLY") == 0) red18("S_EARLY = 0: S_RATIO is undefined and the Steps 3/4 floor is unreadable")
    else if (!agree(V("S_RATIO"), V("S_LATE") / V("S_EARLY")))
      red18(sprintf("S_RATIO = %s CONTRADICTS its own published slopes: S_LATE/S_EARLY = %.6f", \
            SV("S_RATIO"), V("S_LATE") / V("S_EARLY")))
  }
  for (r = 1; r <= 2; r++) {
    p = "R5" r
    if (has(p "_MDE_PCT") && has(p "_SD") && has(p "_MEAN")) {
      if (V(p "_MEAN") == 0) red18(sprintf("%s_MEAN = 0: the MDE percentage is undefined", p))
      else {
        want = 4.303 * V(p "_SD") / V(p "_MEAN") * 100
        if (!agree(V(p "_MDE_PCT"), want))
          red18(sprintf("%s_MDE_PCT = %s CONTRADICTS manifest §1.1's formula 4.303*sd/mean*100 = %.6f", \
                p, SV(p "_MDE_PCT"), want))
      }
    }
  }
  if (has("N6_COST_H") && has("N6_COST_CELLS")) {
    want = V("N6_COST_CELLS") * 1800 / 3600
    if (!agree(V("N6_COST_H"), want))
      red18(sprintf("N6_COST_H = %s CONTRADICTS its own cell count: %s x 1800 s = %.6f h (ADJ-10)", \
            SV("N6_COST_H"), SV("N6_COST_CELLS"), want))
  }

  # ---- item 18(c): ranges ----
  split("EXIT_SHARE_PCT R51_MDE_PCT R52_MDE_PCT", pc, " ")
  for (i = 1; i <= 3; i++)
    if (has(pc[i]) && (V(pc[i]) < 0 || V(pc[i]) > 100))
      red18(sprintf("%s = %s is not a percentage in [0,100]", pc[i], SV(pc[i])))
  split("S_EARLY S_LATE S_RATIO MEDIAN_L_OVER_B R51_SD R52_SD R51_MEAN R52_MEAN N6_COST_CELLS N6_COST_H", nn, " ")
  for (i = 1; i <= 10; i++)
    if (has(nn[i]) && V(nn[i]) < 0)
      red18(sprintf("%s = %s is negative, which none of these quantities can be", nn[i], SV(nn[i])))

  if (nosub && !bad18) print "NO SUBJECT 18 -- the cell has NOT RUN (`-v nosubject=1`; no committed artifact exists to re-derive from): re-derivation has no subject (arithmetic and range limbs PASSED)"
  else if (!bad18)     printf "PASS 18 -- all %d re-derivable slots reproduce the committed artifacts; arithmetic and ranges agree\n", nt18

  # ---- item 19(a): the R5.4 2x2 LICENCE, a pure function of two published enum slots ----
  lic = "OK"
  if (has("R51_RESULT") && has("R52_RESULT")) lic = licof(SV("R51_RESULT"), SV("R52_RESULT"))
  if (lic != "OK" && has("STEP_LEAF") && SV("STEP_LEAF") != "INDETERMINATE")
    red19(sprintf("R5.4 cell is %s -- no classification number may be read off any run -- but STEP_LEAF publishes '%s'", \
          lic, SV("STEP_LEAF")))

  # ---- item 19(a-ii): THE LICENCE ITSELF IS RE-DERIVED, WHICH CLOSES THE OTHER DIRECTION ----
  # (a) above grades only the DECLINING direction: a PUBLISHED rejection forces INDETERMINATE. The
  # direction that LAUNDERS -- a control that REJECTED, published as `NOT REJECTED`, buying the right
  # to publish a classification AT ALL -- was graded by nothing, because these two slots sat outside
  # the re-derivation set while R5.7(g) stated the re-derivation principle as universal. Nothing here
  # is forged in the fixture that found it: every other slot agrees, the matrix triples, the coordinate
  # limbs, the epoch limbs and the slice `cmp` are all intact, and the ONE un-re-derived byte was the
  # control's verdict. The two licences are now compared AS LICENCES, by the same pure function.
  if (("R51_RESULT" in TR) && ("R52_RESULT" in TR)) {
    licd = licof(TR["R51_RESULT"], TR["R52_RESULT"])
    if (licd != lic)
      red19(sprintf("the PUBLISHED R5.4 licence is %s (from R51_RESULT '%s' / R52_RESULT '%s') but the licence RE-DERIVED from the committed control CSVs is %s (from '%s' / '%s'). The licence decides whether ANY classification number may be read off ANY run, so it is graded in BOTH directions: publishing a rejection the controls do not show DECLINES a classification the evidence permits, and publishing NOT REJECTED where the controls REJECT LAUNDERS the right to classify at all.", \
            lic, SV("R51_RESULT"), SV("R52_RESULT"), licd, TR["R51_RESULT"], TR["R52_RESULT"]))
  }

  # ---- item 19(b): the published leaf vs the FROZEN predicate on the RE-DERIVED values ----
  # THERE IS NO FALLBACK TO THE PUBLISHED SLOTS. Deleted at Audit v11 C2: grading the block against
  # itself certifies any internally-consistent forgery, and `src` printed "re-derived" while doing
  # exactly that. Either ALL FOUR predicate inputs are re-derived, or this limb REDs.
  na19 = 0
  if (has("STEP_LEAF") && SV("STEP_LEAF") == "INDETERMINATE") {
    print "NOTE   -- STEP_LEAF is INDETERMINATE: item 19(b) is NOT APPLICABLE (Step 0/5 admissibility is decided"
    print "          upstream of these slots). This is the SAFE direction: INDETERMINATE reads no classification"
    print "          number, so it cannot launder one. Recorded as this limb's measured residual."
    na19 = 1
  } else if (nosub) {
    na19 = 2
  } else {
    nmp = 0; mp = ""
    split("EXIT_SHARE_PCT MEDIAN_L_OVER_B S_EARLY S_LATE", PRD, " ")
    for (q = 1; q <= 4; q++) if (!(PRD[q] in TR)) mp = mp (nmp++ ? ", " : "") PRD[q]
    if (nmp > 0)
      red19(sprintf("cannot reconcile -- truth incomplete: the frozen predicate's input(s) %s were NOT re-derived, and there is NO fallback to the published slots (grading them against themselves certifies any internally-consistent forgery)", mp))
    else {
      src = "re-derived"                        # all four inputs are in TR, or we did not reach here
      e = TR["EXIT_SHARE_PCT"]; b = TR["MEDIAN_L_OVER_B"]; se = TR["S_EARLY"]; sl = TR["S_LATE"]
      want = leafof(e, b, se, sl)
      if (SV("STEP_LEAF") != want)
        red19(sprintf("PUBLISHED leaf '%s' != the leaf the frozen R8.1 predicate implies from the %s values: '%s'\n           (exit share %s %%, median(L)/B %s, s_early %s, s_late %s)", \
              SV("STEP_LEAF"), src, want, e "", b "", se "", sl ""))
    }
  }
  # A GREEN LINE MUST MEAN WHAT IT SAYS. `PASS 19` asserts a comparison; where none was performed the
  # verdict is NOT APPLICABLE or NO SUBJECT, never PASS (Audit v11 rec 3).
  # `FAIL 18` AND `PASS 19` MAY NOT CO-PRINT (Audit v12 rec 6). Item 19 reads item 18's own inputs, so
  # a green 19 beside a red 18 is a verdict issued over a re-derivation that did not stand. It is not
  # downgraded to a NOTE and it is not promoted to a second FAIL -- it is WITHHELD, which is the honest
  # third thing: no comparison is being certified, and item 18's RED is the reason.
  if (bad19) { }
  else if (bad18) print "WITHHELD 19 -- item 18 is RED, so no PASS is issued here: the leaf comparison rests on the same re-derivation item 18 just rejected. Fix 18, then re-run."
  else if (na19 == 1) print "NOT APPLICABLE 19 -- STEP_LEAF is INDETERMINATE: the frozen predicate has nothing to reconcile, and NO comparison was performed"
  else if (na19 == 2) print "NO SUBJECT 19 -- the cell has NOT RUN: there are no re-derived values to reconcile the published leaf against"
  else print "PASS 19 -- the published leaf is the leaf the frozen predicate implies, and the R5.4 licence permits it"
  exit rc
}
