# The allocator-series decision program. Called by spec377-decide.sh, which
# resolves presence (absent / dup / the value) into one KEY=VALUE intermediate
# and passes every threshold in from the manifest's frozen section:
#   awk -f spec377-decide.awk -v better_bar= -v sys_agree_bar= -v ops_min= \
#       -v tie_band= -v stage2_max= -v mi_bound= -v lazy_bar= -v price= <intermediate>
#
# No threshold lives in this file: every number it compares against arrives
# through -v, and the only numeric constants in code are 0, 1 and 2. Regular
# expressions that need a digit class are built from strings for the same
# reason.
#
# Order: STOP-H and STOP-V are evaluated first; the readings are printed next;
# the decision flags are printed LAST, and under any STOP every one of them
# reads STOP. Every number compared goes through num(), which rejects an
# empty, n/a or non-numeric token, so no string can compare as a number and
# pass a gate. Every SYS-derived quantity needs BOTH SYS cells.

function num(x) { return x ~ NUMRE }
function tok(x) { sub(/ .*/, "", x); return x }
function add(s, c) { return s (s == "" ? "" : " ") c }
function ab(x) { return x < 0 ? -x : x }
# The reason= text of an n/a value ("n/a reason=missing:s2:OPS" -> "missing:s2:OPS").
function why(v) { if (match(v, /reason=[^ ]+/)) return substr(v, RSTART + length("reason="), RLENGTH - length("reason=")); return "unknown" }
# The value of a key=value token inside a reading line.
function field(v, k,   n, i, t, p) {
  n = split(v, t, " ")
  for (i = 1; i <= n; i++) { p = index(t[i], "="); if (p && substr(t[i], 1, p - 1) == k) return substr(t[i], p + 1) }
  return ""
}
function isabsent(v) { return v == "" || v == "absent" || v == "dup" }
# A cell's TREND class: MISSING (absent, dup or n/a of any reason -- one path),
# OOD (a token outside the printed domain), or the class itself.
function trendc(c,   v, t) {
  v = V["TREND_" c]
  if (isabsent(v)) return "MISSING"
  t = tok(v)
  if (t == "n/a") return "MISSING"
  if (t == "RISING" || t == "FALLING" || t == "FLAT" || t == "UNDERPOWERED" || t == "MARGINAL") return t
  return "OOD"
}
function level(c, k) { return tok(V[k "_" c]) }
# The set-based tie of R7.7 step 2: every member within tie_band of the
# minimum, then the fixed preference JE, MI3, MI2 (the order of ARMS).
function tiepick(inset, key,   i, a, v, m, have) {
  have = 0
  for (i = 1; i <= NARM; i++) { a = ARMS[i]; if (!(a in inset)) continue; v = level(AC[a], key) + 0; if (!have || v < m) { m = v; have = 1 } }
  for (i = 1; i <= NARM; i++) { a = ARMS[i]; if ((a in inset) && level(AC[a], key) + 0 <= m * (1 + tie_band)) return a }
  return ""
}

BEGIN { D = "0-9"; NUMRE = "^-?[" D "]+([.][" D "]+)?([eE][-+]?[" D "]+)?$" }
{ p = index($0, "="); if (p == 0) next; V[substr($0, 1, p - 1)] = substr($0, p + 1) }

END {
  NCELL = split("s1 je mi3 mi2 s2", CELLS, " ")
  NARM = split("JE MI3 MI2", ARMS, " ")
  AC["JE"] = "je"; AC["MI3"] = "mi3"; AC["MI2"] = "mi2"
  NSYS = split("s1 s2", SYS, " ")
  NMI = split("mi3 mi2", MIC, " ")

  # ------------------------------------------------------------ STOP-H
  h = ""
  if (V["PREFLIGHT_CLAUSE"] != "none") h = add(h, V["PREFLIGHT_CLAUSE"] == "" ? "preflight_clause=absent" : V["PREFLIGHT_CLAUSE"])
  for (i = 1; i <= NCELL; i++) {
    c = CELLS[i]; s = V["STEAL_" c]
    if (!num(s)) h = add(h, c ":steal=missing")
    else if (s + 0 > 1) h = add(h, c ":steal=" s)
  }
  if (V["DISK_CLAUSE"] != "none") h = add(h, V["DISK_CLAUSE"] == "" ? "disk_free=absent" : V["DISK_CLAUSE"])

  # ------------------------------------------------------------ STOP-V
  vf = ""
  np = split("PV PEL PM1 PMEM PALLOC", PRED, " ")
  for (i = 1; i <= NCELL; i++) {
    c = CELLS[i]
    if (V["PRED_" c] != "present") vf = add(vf, c ":predicates_missing")
    else {
      for (j = 1; j <= np; j++) if (V["PRED_" c "_" PRED[j]] != "TRUE") vf = add(vf, c ":" PRED[j] "=" (V["PRED_" c "_" PRED[j]] == "" ? "absent" : V["PRED_" c "_" PRED[j]]))
      # PA is pre-declared n/a on SYS and MI cells and never read there.
      if (c == "je" && V["PRED_je_PA"] != "TRUE") vf = add(vf, "je:PA=" (V["PRED_je_PA"] == "" ? "absent" : V["PRED_je_PA"]))
    }
    if (V["PREDRC_" c] != "0") vf = add(vf, c ":predicates_rc=" (V["PREDRC_" c] == "" ? "absent" : V["PREDRC_" c]))
    if (V["RUNNER_EXIT_" c] != "0") vf = add(vf, c ":runner_exit=" (V["RUNNER_EXIT_" c] == "" ? "absent" : V["RUNNER_EXIT_" c]))
    we = V["PRED_" c "_WRITE_ERRORS"]
    if (!num(we)) vf = add(vf, c ":write_errors=" (we == "" ? "absent" : we))
    else if (we + 0 != 0) vf = add(vf, c ":write_errors=" we)
    pc = V["PRED_" c "_PR-crashes"]
    if (pc != "TRUE") vf = add(vf, c ":PR-crashes=" (pc == "" ? "absent" : pc))
  }
  stop = "none"
  if (h != "") stop = "H (" h ")"
  else if (vf != "") stop = "V (" vf ")"
  STOPPED = (stop != "none")

  print "== stop predicates =="
  print "STOP_H_CLAUSES=" (h == "" ? "none" : h)
  print "STOP_V_CLAUSES=" (vf == "" ? "none" : vf)

  # ------------------------------------------------------------ readings
  wsum = 0; wmiss = ""
  for (i = 1; i <= NCELL; i++) { c = CELLS[i]; we = V["PRED_" c "_WRITE_ERRORS"]; if (num(we)) wsum += we; else if (wmiss == "") wmiss = c }
  WRITE_ERRORS = (wmiss == "") ? wsum "" : "n/a reason=missing:" wmiss ":WRITE_ERRORS"
  for (i = 1; i <= NCELL; i++) {
    c = CELLS[i]; tw = V["TOTAL_WRITES_" c]; du = V["DURATION_" c]
    if (!num(du)) OPS[c] = "n/a reason=missing:" c ":DURATION"
    else if (du + 0 <= 0) OPS[c] = "n/a reason=out_of_domain:" c ":DURATION"
    else if (!num(tw)) OPS[c] = "n/a reason=missing:" c ":totalWrites"
    else OPS[c] = sprintf("%.3f", tw / du)
  }
  for (i = 1; i <= NARM; i++) {
    a = ARMS[i]; ac = AC[a]; miss = ""
    if (!num(OPS["s1"])) miss = "s1"; else if (!num(OPS["s2"])) miss = "s2"; else if (!num(OPS[ac])) miss = ac
    if (miss != "") OPSR[a] = "n/a reason=missing:" miss ":OPS"
    else if ((OPS["s1"] + OPS["s2"]) / 2 <= 0) OPSR[a] = "n/a reason=out_of_domain:s1:OPS"
    else OPSR[a] = sprintf("%.4f", OPS[ac] / ((OPS["s1"] + OPS["s2"]) / 2))
  }
  # LAZY_DRIFT: a config-drift canary, recorded.
  ld = ""; lmiss = ""
  for (i = 1; i <= NCELL; i++) {
    c = CELLS[i]; r = field(V["LAZY_MAX_" c], "ratio")
    if (num(r)) { if (r + 0 > lazy_bar + 0) ld = ld (ld == "" ? "" : ",") c } else if (lmiss == "") lmiss = c
  }
  LAZY_DRIFT = (ld != "") ? "TRUE cells=" ld : ((lmiss != "") ? "n/a reason=missing:" lmiss ":LAZY_MAX" : "FALSE")
  mn = ""; mmiss = ""
  for (i = 1; i <= NMI; i++) {
    c = MIC[i]; v = tok(V["MI_POSTINIT_LINES_" c])
    if (num(v)) { if (v + 0 > mi_bound + 0) mn = mn (mn == "" ? "" : ",") c ":" v } else if (mmiss == "") mmiss = c
  }
  MI_NOTE = (mn != "") ? mn : ((mmiss != "") ? "n/a reason=missing:" mmiss ":MI_POSTINIT_LINES" : "none")

  # ------------------------------------------------------------ decision
  # SYS_AGREE: both replicates, positive levels, no division by a non-positive mean.
  sa = ""
  for (i = 1; i <= NSYS; i++) {
    c = SYS[i]; t = level(c, "PE_LEVEL")
    if (!num(t)) { sa = "FALSE reason=missing:" c ":PE_LEVEL"; break }
    if (t + 0 <= 0) { sa = "FALSE reason=out_of_domain:" c ":PE_LEVEL"; break }
  }
  if (sa == "") {
    p1 = level("s1", "PE_LEVEL") + 0; p2 = level("s2", "PE_LEVEL") + 0
    dv = sprintf("%.4f", ab(p1 - p2) / ((p1 + p2) / 2))
    sa = (dv + 0 <= sys_agree_bar + 0) ? "TRUE value=" dv : "FALSE value=" dv
  }
  # PLATEAU_SYS, first match; the input checks run cell-major.
  ps = ""
  for (i = 1; i <= NSYS; i++) {
    c = SYS[i]; tc = trendc(c)
    if (tc == "MISSING") { ps = "INDETERMINATE reason=missing:" c ":TREND"; break }
    if (tc == "OOD") { ps = "INDETERMINATE reason=out_of_domain:" c ":TREND"; break }
    t = level(c, "PE_LEVEL")
    if (!num(t)) { ps = "INDETERMINATE reason=missing:" c ":PE_LEVEL"; break }
    if (t + 0 <= 0) { ps = "INDETERMINATE reason=out_of_domain:" c ":PE_LEVEL"; break }
  }
  if (ps == "") {
    t1 = trendc("s1"); t2 = trendc("s2")
    if (tok(sa) != "TRUE") ps = "INDETERMINATE reason=disagree"
    else if (t1 == "RISING" || t2 == "RISING") ps = "RISING"
    else if (t1 == "FALLING" || t2 == "FALLING") ps = "INDETERMINATE reason=falling"
    else if (t1 == "FLAT" && t2 == "FLAT") ps = "PLATEAU"
    else if (t1 == "UNDERPOWERED" || t2 == "UNDERPOWERED") ps = "UNDERPOWERED"
    else ps = "INDETERMINATE reason=marginal"
  }
  # STAGE2_FEASIBLE: absence is named before size.
  if (ps == "UNDERPOWERED") {
    sf = ""; mx = 0
    for (i = 1; i <= NSYS; i++) {
      c = SYS[i]; if (trendc(c) != "UNDERPOWERED") continue
      v = tok(V["STAGE2_T_" c])
      if (isabsent(v) || v == "n/a") { sf = "FALSE reason=missing:" c ":STAGE2_T"; break }
    }
    if (sf == "") for (i = 1; i <= NSYS; i++) {
      c = SYS[i]; if (trendc(c) != "UNDERPOWERED") continue
      v = tok(V["STAGE2_T_" c])
      if (v == ">STAGE2_MAX_H") { sf = "FALSE reason=>STAGE2_MAX_H"; break }
    }
    if (sf == "") for (i = 1; i <= NSYS; i++) {
      c = SYS[i]; if (trendc(c) != "UNDERPOWERED") continue
      v = tok(V["STAGE2_T_" c])
      if (!num(v)) { sf = "FALSE reason=out_of_domain:" c ":STAGE2_T"; break }
      if (v + 0 > stage2_max + 0) { sf = "FALSE reason=>STAGE2_MAX_H"; break }
      if (v + 0 > mx) mx = v + 0
    }
    if (sf == "") { sf = "TRUE"; sc = sprintf("%.3f", 2 * mx * price) }
    else sc = "n/a reason=" why(sf)
  } else { sf = "n/a reason=not_underpowered"; sc = "n/a reason=not_underpowered" }
  # VS_SYS: the arm's level against the BETTER SYS replicate.
  for (i = 1; i <= NARM; i++) {
    a = ARMS[i]; ac = AC[a]; VS[a] = ""
    for (j = 1; j <= NSYS; j++) {
      c = SYS[j]; t = level(c, "PE_LEVEL")
      if (!num(t)) { VS[a] = "n/a reason=missing:" c ":PE_LEVEL"; break }
      if (t + 0 <= 0) { VS[a] = "n/a reason=out_of_domain:" c ":PE_LEVEL"; break }
    }
    if (VS[a] != "") continue
    t = level(ac, "PE_LEVEL")
    if (!num(t)) { VS[a] = "n/a reason=missing:" ac ":PE_LEVEL"; continue }
    if (t + 0 <= 0) { VS[a] = "n/a reason=out_of_domain:" ac ":PE_LEVEL"; continue }
    p1 = level("s1", "PE_LEVEL") + 0; p2 = level("s2", "PE_LEVEL") + 0
    r = sprintf("%.4f", t / (p1 < p2 ? p1 : p2))
    VS[a] = r " " ((r + 0 <= better_bar + 0) ? "BETTER" : "NO_GAIN")
  }
  # VERDICT, first matching clause.
  for (i = 1; i <= NARM; i++) {
    a = ARMS[i]; ac = AC[a]; o = OPSR[a]; tc = trendc(ac)
    if (!num(tok(o))) VD[a] = "n/a reason=" why(o)
    else if (o + 0 < ops_min + 0) VD[a] = "n/a reason=ops"
    else if (tc == "MISSING") VD[a] = "n/a reason=missing:" ac ":TREND"
    else if (tc == "OOD") VD[a] = "n/a reason=out_of_domain:" ac ":TREND"
    else if (!num(level(ac, "PE_LEVEL"))) VD[a] = "n/a reason=missing:" ac ":PE_LEVEL"
    else if (!num(tok(VS[a]))) VD[a] = "n/a reason=" why(VS[a])
    else if (tc == "FLAT" && VS[a] ~ / BETTER$/) VD[a] = "CANDIDATE"
    else if (tc == "FLAT" && VS[a] ~ / NO_GAIN$/) VD[a] = "FLAT_NO_GAIN"
    else if (tc == "FALLING" || tc == "UNDERPOWERED" || tc == "MARGINAL") VD[a] = "UNRESOLVED(" tc ")"
    else if (tc == "RISING") VD[a] = "RISING"
    else VD[a] = "n/a reason=unmatched"
  }
  # DEFAULT_CANDIDATE, set-based; ORDER_AGREE repeats the pick on RSS_PE_LEVEL.
  nC = 0; delete CS
  for (i = 1; i <= NARM; i++) if (VD[ARMS[i]] == "CANDIDATE") { CS[ARMS[i]] = 1; nC++ }
  if (nC == 0) { dc = (ps == "PLATEAU") ? "SYS" : "NONE"; oa = "TRUE n=0" }
  else {
    choice = tiepick(CS, "PE_LEVEL")
    om = ""
    for (i = 1; i <= NARM; i++) { a = ARMS[i]; if ((a in CS) && !num(level(AC[a], "RSS_PE_LEVEL")) && om == "") om = AC[a] }
    if (om != "") oa = "FALSE reason=missing:" om ":RSS_PE_LEVEL"
    else if (nC <= 1) oa = "TRUE n=" nC
    else { rch = tiepick(CS, "RSS_PE_LEVEL"); oa = (rch == choice) ? "TRUE n=" nC : "FALSE pe_choice=" choice " rss_choice=" rch }
    dc = (tok(oa) == "TRUE") ? choice : "NONE"
  }
  # NEXT, first match; clause numbers are the manifest's.
  ood = 0
  for (i = 1; i <= NCELL; i++) if (OPS[CELLS[i]] ~ /out_of_domain/) ood = 1
  for (i = 1; i <= NARM; i++) { a = ARMS[i]; if (OPSR[a] ~ /out_of_domain/ || VS[a] ~ /out_of_domain/ || VD[a] ~ /out_of_domain/ || VD[a] == "n/a reason=unmatched") ood = 1 }
  if (sa ~ /out_of_domain/ || ps ~ /out_of_domain/ || sf ~ /out_of_domain/ || sc ~ /out_of_domain/ || oa ~ /out_of_domain/) ood = 1
  allops = 1; anyfng = 0
  for (i = 1; i <= NARM; i++) {
    a = ARMS[i]
    if (!(VD[a] == "n/a reason=ops" || VD[a] ~ /^n\/a reason=missing:[^:]+:OPS$/)) allops = 0
    if (VD[a] == "FLAT_NO_GAIN") anyfng = 1
  }
  if (STOPPED) nx = "STOP"                                                                          # 1
  else if (ood) nx = "CONDUCTOR_RULING;UNMATCHED"                                                    # 1a
  else if (ps ~ /^INDETERMINATE reason=missing:/) nx = "CONDUCTOR_RULING;SYS_MISSING"               # 2
  else if (ps == "INDETERMINATE reason=disagree") nx = "CONDUCTOR_RULING;SYS_BIMODAL;SEE=TODO-719"  # 3
  else if (allops) nx = "CONDUCTOR_RULING;OPS"                                                       # 4
  else if (dc == "JE" || dc == "MI3" || dc == "MI2") nx = "TODO-696+TODO-695;THEN;DEFAULT_FLIP=" dc  # 5
  else if (nC > 0 && tok(oa) != "TRUE") nx = "CONDUCTOR_RULING;ARM_ORDER_ACCOUNTING"                # 6
  else if (dc == "SYS") nx = "KEEP_SYSTEM;SYS_FLAT_WITHIN_BAR"                                       # 8
  else if (ps == "UNDERPOWERED" && tok(sf) == "TRUE") nx = "STAGE2;SYS_LONG_CELLS"                  # 9
  else if (ps == "UNDERPOWERED") nx = "CONDUCTOR_RULING;SYS_UNRESOLVABLE_BY_SLOPE"                   # 10
  else if (ps == "RISING" && anyfng) nx = "CONDUCTOR_RULING;FLAT_ARM_VS_RISING_SYS"                  # 11
  else if (ps == "RISING") nx = "CONDUCTOR_RULING;NOTHING_PLATEAUS;SEE=TODO-719"                     # 12
  else if (ps == "INDETERMINATE reason=falling" || ps == "INDETERMINATE reason=marginal") nx = "CONDUCTOR_RULING;SYS_INDETERMINATE"   # 13
  else nx = "CONDUCTOR_RULING;UNMATCHED"                                                             # 14
  # CONDUCTOR_RULE: the pre-registered conductor rule in its mechanical form.
  if (STOPPED) cr = "STOP"
  else if (nx !~ /^CONDUCTOR_RULING;/) cr = "n/a reason=not_conductor_ruling"
  else if (nx == "CONDUCTOR_RULING;UNMATCHED" || nx == "CONDUCTOR_RULING;SYS_MISSING" || nx == "CONDUCTOR_RULING;OPS") cr = "n/a reason=excluded:" nx
  else {
    cr = ""
    # Step 3a: "no arm is FLAT" is never inferred from a missing line.
    for (i = 1; i <= NARM; i++) { ac = AC[ARMS[i]]; tc = trendc(ac); if (tc == "MISSING" || tc == "OOD") { cr = "n/a reason=missing:" ac ":TREND"; break } }
    if (cr == "") {
      nL = 0; anyflat = 0; delete LS
      for (i = 1; i <= NARM; i++) {
        a = ARMS[i]; ac = AC[a]; tc = trendc(ac)
        if (tc == "FLAT") anyflat = 1
        if (num(tok(VS[a])) && VS[a] ~ / BETTER$/ && num(tok(OPSR[a])) && OPSR[a] + 0 >= ops_min + 0 && tc != "RISING") { LS[a] = 1; nL++ }
      }
      if (nL == 0) cr = "KEEP_SYSTEM;PLATEAU=OPEN;NEXT=TODO-719"
      else if (!anyflat) {
        pc = tiepick(LS, "PE_LEVEL"); st = V["STAGE2_T_" AC[pc]]
        if (isabsent(st)) st = "n/a reason=missing:" AC[pc] ":STAGE2_T"
        cr = "PROVISIONAL_DEFAULT=" pc ";STAGE2_T=" st ";THEN;TODO-696+REPLICATE_6H;THEN;STAGE2_PLATEAU;PLATEAU=OPEN"
      } else cr = "n/a reason=judgement:" nx
    }
  }

  # ------------------------------------------------------------ the flags block
  print "== flags =="
  print "STOP=" stop
  print "WRITE_ERRORS=" WRITE_ERRORS
  for (i = 1; i <= NCELL; i++) print "OPS_" CELLS[i] "=" OPS[CELLS[i]]
  for (i = 1; i <= NARM; i++) print "OPS_RATIO_" ARMS[i] "=" OPSR[ARMS[i]]
  np = split("LIVE_END PE_END PE_LEVEL TREND TREND3 TREND_CORR STAGE2_T FIXED_EST FIXED_NOTE RSS_PE_LEVEL LAZY_MAX", RK, " ")
  for (j = 1; j <= np; j++) {
    for (i = 1; i <= NCELL; i++) { k = RK[j] "_" CELLS[i]; print k "=" (V[k] == "" ? "absent" : V[k]) }
    if (RK[j] == "LAZY_MAX") print "LAZY_DRIFT=" LAZY_DRIFT
  }
  np = split("HWM_END ANON_HUGE_END", RK, " ")
  for (j = 1; j <= np; j++) for (i = 1; i <= NCELL; i++) { k = RK[j] "_" CELLS[i]; print k "=" (V[k] == "" ? "absent" : V[k]) }
  print "JE_CONFIG=" (V["JE_CONFIG"] == "" ? "absent" : V["JE_CONFIG"])
  print "JE_CONFIRM_CONF=" (V["JE_CONFIRM_CONF"] == "" ? "absent" : V["JE_CONFIRM_CONF"])
  print "AMP_JE=" (V["AMP_JE_je"] == "" ? "absent" : V["AMP_JE_je"])
  np = split("FRAG_SHAREL DIRTY_SHAREL REACH_PE", RK, " ")
  for (j = 1; j <= np; j++) { k = RK[j] "_je"; print k "=" (V[k] == "" ? "absent" : V[k]) }
  for (i = 1; i <= NMI; i++) { k = "MI_POSTINIT_LINES_" MIC[i]; print k "=" (V[k] == "" ? "absent" : V[k]) }
  print "MI_POSTINIT_NOTE=" MI_NOTE
  for (i = 1; i <= NCELL; i++) { k = "DISK_SLOPE_" CELLS[i]; print k "=" (V[k] == "" ? "absent" : V[k]) }
  S = STOPPED
  print "SYS_AGREE=" (S ? "STOP" : sa)
  print "PLATEAU_SYS=" (S ? "STOP" : ps)
  print "STAGE2_FEASIBLE=" (S ? "STOP" : sf)
  print "STAGE2_COST_EUR=" (S ? "STOP" : sc)
  for (i = 1; i <= NARM; i++) print "VS_SYS_" ARMS[i] "=" (S ? "STOP" : VS[ARMS[i]])
  print "ORDER_AGREE=" (S ? "STOP" : oa)
  for (i = 1; i <= NARM; i++) print "VERDICT_" ARMS[i] "=" (S ? "STOP" : VD[ARMS[i]])
  print "DEFAULT_CANDIDATE=" (S ? "STOP" : dc)
  print "NEXT=" nx
  print "CONDUCTOR_RULE=" cr
}
