# spec371 memory-diagnosis decision. Pre-registered in spec371-manifest.md §1.
#
# usage (from an environment that exports LC_ALL=C):
#   awk -v missing="<space-separated missing inputs>" -f spec371-decide.awk \
#       spec371-{r0,c0,c1,c2,c3e,c3l}.predicates.txt spec371-gref.txt spec371-dhat-diff.txt
#
# Closed input set: the six predicates files, the gref file and the dhat diff.
# A file the caller could not find is passed in `missing` and is a named STOP;
# it is never defaulted. Every line's first space-delimited token is NAME=VALUE;
# the key is the text before its first "=". The STOP block runs first, and every
# flag is computed after it, so no flag can read anything but STOP under a STOP.

function key(tok,   p) { p = index(tok, "="); return (p ? substr(tok, 1, p - 1) : tok) }
function val(tok,   p) { p = index(tok, "="); return (p ? substr(tok, p + 1) : "") }
function num(s) { return (s ~ /^-?[0-9]+(\.[0-9]+)?$/) }
function band(r) { return (r <= 0.10) ? "RETENTION" : ((r >= 0.50) ? "REACHABLE" : "MIXED") }
function route(l) {
  if (l == "591") return "TODO-591"; if (l == "593") return "TODO-593"
  if (l == "592") return "TODO-592"; if (l == "588") return "TODO-588"
  if (l == "JOURNAL") return "JOURNAL-CAP-SLIM"
  return "CONDUCTOR_RULING"
}
function need(c, k) { nreq++; req_c[nreq] = c; req_k[nreq] = k }

BEGIN {
  ncells = split("r0 c0 c1 c2 c3e c3l", cells, " ")
  for (i = 1; i <= ncells; i++) {
    c = cells[i]
    need(c, "PV"); need(c, "PR-crashes"); need(c, "PR-class"); need(c, "PE"); need(c, "PA")
    need(c, "PJ"); need(c, "PC"); need(c, "PM1")
    if (c == "r0" || c == "c0" || c == "c1" || c == "c2") { need(c, "P5"); need(c, "P6"); need(c, "P7") }
  }
  need("c3l", "PD")
  # PA may read n/a only where R4 pre-declares it.
  na_ok["r0", "PA"] = 1; na_ok["c3e", "PA"] = 1; na_ok["c3l", "PA"] = 1
}

FNR == 1 {
  f = FILENAME; sub(/.*\//, "", f)
  src = ""
  if (f ~ /^spec371-(r0|c0|c1|c2|c3e|c3l)\.predicates\.txt$/) { src = f; sub(/^spec371-/, "", src); sub(/\.predicates\.txt$/, "", src) }
  else if (f == "spec371-gref.txt") src = "GREF"
  else if (f == "spec371-dhat-diff.txt") src = "DIFF"
  seenfile[src] = 1
}
src == "GREF" && $1 == "G_ref" {
  m = ""; sl = ""
  for (i = 2; i <= NF; i++) { if (key($i) == "member") m = val($i); if (key($i) == "slope") sl = val($i) }
  if (m != "") { gref[m] = sl; gorder[++ng] = m }
  next
}
src == "DIFF" {
  if (key($1) == "C3-top1-lever") top1 = val($1)
  if (key($1) == "LEVER_CONTESTED") contested = val($1)
  if (key($1) == "C3-top1-all-std") allstd = val($1)
  next
}
src != "" && src != "GREF" && src != "DIFF" { k = key($1); if (!((src, k) in v)) v[src, k] = val($1) }

END {
  # ------------------------------------------------------------ STOP block
  stop = 0
  if (missing != "") { stop = 1; print "STOP-reason: missing inputs: " missing }
  for (i = 1; i <= ncells; i++) if (!(cells[i] in seenfile)) { stop = 1; print "STOP-reason: no predicates file for " cells[i] }
  if (!("GREF" in seenfile)) { stop = 1; print "STOP-reason: gref_missing (no spec371-gref.txt)" }
  if (!("DIFF" in seenfile)) { stop = 1; print "STOP-reason: no spec371-dhat-diff.txt" }
  for (i = 1; i <= nreq; i++) {
    c = req_c[i]; k = req_k[i]
    x = ((c, k) in v) ? v[c, k] : "<missing>"
    pass = (x == "TRUE") || (x == "n/a" && ((c, k) in na_ok))
    if (!pass) { stop = 1; print "STOP-reason: " c " " k "=" x }
  }
  for (j = 1; j <= 3; j++) { m = (j == 1 ? "r0" : (j == 2 ? "8e" : "8f")); if (!(m in gref)) { stop = 1; print "STOP-reason: gref_missing member=" m } }
  # The fits that CLASS and JOURNAL divide and compare are inputs too: a fit that
  # produced no number (FIT_ERROR, empty column) is a missing input, so it STOPs.
  split("c0:G c2:G c1:G c0:L c2:L", fk, " ")
  for (j = 1; j <= 5; j++) {
    split(fk[j], cc, ":"); x = ((cc[1], cc[2]) in v) ? v[cc[1], cc[2]] : "<missing>"
    if (!num(x)) { stop = 1; print "STOP-reason: " cc[1] " " cc[2] "=" x " (not a number)" }
  }
  print "STOP=" (stop ? "TRUE" : "FALSE")

  if (stop) {
    print "CLASS=STOP"; print "REPLICATE_AGREE=STOP"; print "JOURNAL=STOP"; print "F1_FORECAST_HELD=STOP"
    print "CA_REGIME=STOP"; print "RELEASE_RETENTION=STOP"; print "LEVER=STOP"; print "NEXT=STOP"
    exit 0
  }

  # ------------------------------------------------------------ readings
  # substr() yields STRINGS, and awk compares two strings lexically ("700" >= "1000"
  # is true), so every reading is forced numeric with +0 before any comparison.
  G0 = v["c0", "G"] + 0; G2 = v["c2", "G"] + 0; G1 = v["c1", "G"] + 0
  L0 = v["c0", "L"] + 0; L2 = v["c2", "L"] + 0
  GMIN = (G0 < G2) ? G0 : G2; GMAX = (G0 > G2) ? G0 : G2
  printf "GMIN=%s GMAX=%s G_c0=%s G_c2=%s G_c1=%s L_c0=%s L_c2=%s\n", GMIN, GMAX, G0, G2, G1, L0, L2

  # Ops parity. Every slope is per HOUR, so a count-alloc build that served
  # fewer ops would show a smaller slope for a reason that is not retention.
  # The load is paced, so parity is expected -- but it is read, not assumed.
  ops_r0 = v["r0", "OPS_PER_S"] + 0; ops_c0 = v["c0", "OPS_PER_S"] + 0
  ops_c2 = v["c2", "OPS_PER_S"] + 0; ops_c1 = v["c1", "OPS_PER_S"] + 0
  ops_min = (ops_c0 < ops_c2) ? ops_c0 : ops_c2
  if (num(v["r0", "OPS_PER_S"]) && ops_r0 > 0 && num(v["c0", "OPS_PER_S"]) && num(v["c2", "OPS_PER_S"])) {
    ops_ratio = ops_min / ops_r0
    printf "OPS_PER_S r0=%s c0=%s c1=%s c2=%s\n", ops_r0, ops_c0, ops_c1, ops_c2
    printf "OPS_RATIO=%.3f OPS_RATIO_c1=%.3f\n", ops_ratio, (ops_r0 > 0 ? ops_c1 / ops_r0 : 0)
    ops_ok = (ops_ratio >= 0.80)
  } else { ops_ok = 0; ops_ratio = -1; print "OPS_RATIO=n/a reason=missing_ops_reading" }

  # CA regime over G_ref = {r0, 8e, 8f}
  gbad = 0; ghave = 0
  for (j = 1; j <= ng; j++) {
    x = gref[gorder[j]]
    if (!num(x) || x + 0 <= 0) { gbad = 1; continue }
    x = x + 0
    if (!ghave || x < grmin) grmin = x
    if (!ghave || x > grmax) grmax = x
    ghave = 1
  }
  if (!ops_ok) { CA = "n/a"; RR = "n/a"; print "CA_REGIME-reason=ops (OPS_RATIO " ((ops_ratio < 0) ? "unreadable" : "< 0.80") ")" }
  else if (gbad) { CA = "n/a"; RR = "n/a"; print "CA_REGIME-reason=gref member <= 0 or non-numeric" }
  else {
    CA = (GMIN >= 0.5 * grmin) ? "COMPARABLE" : ((GMAX < 0.5 * grmin) ? "SUPPRESSED" : "PARTIAL")
    RR = (CA == "SUPPRESSED") ? "TRUE" : ((CA == "COMPARABLE") ? "FALSE" : "PARTIAL")
    printf "G_ref min=%s max=%s ratio_GMIN_over_max=%.3f ratio_GMAX_over_min=%.3f\n", grmin, grmax, GMIN / grmax, GMAX / grmin
    rs = 1 - GMAX / grmin; if (rs < 0) rs = 0
    printf "RET_SHARE_REL=%.3f\n", rs
  }

  # CLASS
  if (GMIN < 200) { CLASS = "INDETERMINATE_NO_GROWTH"; AGREE = "n/a" }
  else {
    R0 = L0 / G0; R2 = L2 / G2
    printf "R_c0=%.4f R_c2=%.4f\n", R0, R2
    if (R0 <= 0.10 && R2 <= 0.10) CLASS = "RETENTION"
    else if (R0 >= 0.50 && R2 >= 0.50) CLASS = "REACHABLE"
    else { CLASS = "MIXED"; mid = (R0 + R2) / 2; printf "reachable_share=[%.4f,%.4f] reachable_mid=%.4f\n", (R0 < R2 ? R0 : R2), (R0 > R2 ? R0 : R2), mid }
    AGREE = (band(R0) == band(R2)) ? "TRUE" : "FALSE"
  }

  # JOURNAL (churn share against the baseline RANGE)
  if (CLASS == "INDETERMINATE_NO_GROWTH") J = "n/a"
  else J = (G1 <= 0.5 * GMIN) ? "HOLDER" : ((G1 >= GMIN) ? "NOT_HOLDER" : "UNRESOLVED")
  ll1 = v["c1", "LIVE_LH_mean"]; ll0 = v["c0", "LIVE_LH_mean"]; ll2 = v["c2", "LIVE_LH_mean"]
  if (num(ll1) && num(ll0) && num(ll2)) { ll0 += 0; ll1 += 0; ll2 += 0; printf "JLIVE=%.3f\n", ll1 - (ll0 < ll2 ? ll0 : ll2) }
  else print "JLIVE=n/a"
  F1 = (J == "NOT_HOLDER") ? "TRUE" : ((J == "HOLDER") ? "FALSE" : "UNRESOLVED")

  # AMP_ratio_CA (recorded)
  ar = v["r0", "AMP_fp_terminal"]
  for (j = 1; j <= 2; j++) {
    c = (j == 1 ? "c0" : "c2"); x = v[c, "AMP_fp_terminal"]
    if (num(ar) && ar + 0 > 0 && num(x)) printf "AMP_ratio_CA %s=%.3f\n", c, x / ar
    else print "AMP_ratio_CA " c "=n/a"
  }

  # CLASS_FRAGILE (recorded; changes no flag): would the band of c0 or c2 move
  # if its two slopes were read one standard error apart?
  fragile = "n/a"
  if (CLASS != "INDETERMINATE_NO_GROWTH") {
    fragile = "FALSE"
    for (j = 1; j <= 2; j++) {
      c = (j == 1 ? "c0" : "c2")
      gs = v[c, "G_se"]; ls = v[c, "L_se"]; g = v[c, "G"] + 0; l = v[c, "L"] + 0
      if (!num(gs) || !num(ls)) { fragile = "n/a"; break }
      lo = (g + gs > 0) ? (l - ls) / (g + gs) : 0
      hi = (g - gs > 0) ? (l + ls) / (g - gs) : 1e9
      if (band(lo) != band(hi)) fragile = "TRUE"
    }
  }
  print "CLASS_FRAGILE=" fragile
  print "LEVER_CONTESTED=" ((contested == "") ? "n/a" : contested)
  print "C3-top1-all-std=" ((allstd == "") ? "n/a" : allstd)

  # LEVER and NEXT
  LEVER = (CLASS == "REACHABLE" || CLASS == "MIXED") ? (top1 == "" ? "UNMAPPED" : top1) : "n/a"
  REACH = (J == "HOLDER") ? "JOURNAL-CAP-SLIM" : route(LEVER)
  RET = "TODO-590+TODO-591"
  if (CA == "n/a") base = "CONDUCTOR_RULING"
  else if (CLASS == "INDETERMINATE_NO_GROWTH") base = (RR == "TRUE") ? RET : "CONDUCTOR_RULING"
  else if (CLASS == "RETENTION") base = (J == "HOLDER") ? "JOURNAL-CAP-SLIM;THEN;" RET : RET
  else if (CLASS == "REACHABLE") base = REACH
  else base = (mid >= 0.5) ? REACH ";THEN;" RET : RET ";THEN;" REACH
  NEXT = base
  if (CA != "n/a" && CLASS != "INDETERMINATE_NO_GROWTH" && RR == "TRUE" && index(base, RET) == 0) NEXT = RET ";THEN;" base
  if (J == "UNRESOLVED") NEXT = NEXT ";JOURNAL_UNRESOLVED"
  if (RR == "PARTIAL") NEXT = NEXT ";CA_PARTIAL"

  print "CLASS=" CLASS
  print "REPLICATE_AGREE=" AGREE
  print "JOURNAL=" J
  print "F1_FORECAST_HELD=" F1
  print "CA_REGIME=" CA
  print "RELEASE_RETENTION=" RR
  print "LEVER=" LEVER
  print "NEXT=" NEXT
}
