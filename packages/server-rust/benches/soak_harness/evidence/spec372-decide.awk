# spec372 allocator decision. Pre-registered in spec372-manifest.md §1; this
# program's bytes are bound at M.
#
# usage (from an environment that exports LC_ALL=C):
#   stage1 (at D1) -> spec372.stage1.txt
#     awk -v mode=stage1 -v disk_free=<GB> -v missing="<names>" -f spec372-decide.awk \
#         spec372-{s1a,j1a,m1a,k1,j1b,m1b,s1b}.predicates.txt spec372.k-stage1.txt
#   stage2 (at D2) -> spec372.decision.txt
#     awk -v mode=stage2 -v disk_free=<GB> -v a2j_state=<ran|budget> -v a2j_flavour=<JE|MI> \
#         -v missing="<names>" -f spec372-decide.awk spec372.stage1.txt \
#         spec372-{s2,j2,m2,a2j}.predicates.txt spec372.k-stage2.txt spec372-perf.txt spec372-buildstory.txt
#   list (smoke only): one input per line of KEY=VALUE tokens (an n/a reason is
#     written n/a:<reason>); out, per input, "DEFAULT_CANDIDATE=<..> NEXT=<..>"
#     followed by the input line itself.
#
# Every input line is NAME=VALUE; the value is the first space-delimited token
# after the "=" (JE_CONFIG keeps its whole line). A file the caller could not
# find is passed in `missing` and is a named STOP; it is never defaulted.
# The STOP block is evaluated first; every flag is then printed by the one
# emitter at the end of END, which prints STOP in place of the value whenever
# STOP=TRUE, so no flag can carry anything but STOP under a STOP.

function num(s) { return (s ~ /^-?[0-9]+(\.[0-9]+)?([eE][-+]?[0-9]+)?$/) }
function r4(x) { return sprintf("%.4f", x) }
function absd(x) { return (x < 0) ? -x : x }
function has(c, k) { return ((c, k) in P) && num(P[c, k]) }
function get(c, k, dflt) { return ((c, k) in P) ? P[c, k] : dflt }
function getfull(c, k, dflt) { return ((c, k) in PF) ? PF[c, k] : dflt }
function isna(v) { return (v ~ /^n\/a/) }
# Stage-1-only keys print first, in insertion order; every key of R8's flag
# block then prints in R8's order, from VAL[] or as n/a with the mode's reason.
function pre(k, v) { npre++; PK_[npre] = k; PV_[npre] = v }
function out(k, v) { VAL[k] = v }
function R8_KEYS() { return "WRITE_ERRORS DISK_FREE_AT_START OPS_RATIO_JE_S1 OPS_RATIO_MI_S1 OPS_RATIO_JE_S2 OPS_RATIO_MI_S2 S1_SURVIVORS S1_RANK CHURN_RATIO CHURN_RATIO_DRIFT K_PROVISIONAL T_DECAY_UPPER_JE T_DECAY_UPPER_MI CV_JE CV_MI K_lo_JE K_hi_JE K_lo_MI K_hi_MI R_redb_s2 R_redb_j2 R_redb_m2 K_HI_VACUOUS_JE K_HI_VACUOUS_MI DIRTY_SHARE_j1a DIRTY_SHARE_j1b DIRTY_SHARE_j2 FRAG_SHARE_j1a FRAG_SHARE_j1b FRAG_SHARE_j2 EST_AGREE_900_j1a EST_AGREE_900_j1b UNMODELLED_REACHABLE_j1a UNMODELLED_REACHABLE_j1b UNMODELLED_REACHABLE_j2 AMP_FP_S2 AMP_S_S2 AMP_FP_JE AMP_S_JE AMP_JE AMP_FP_MI AMP_S_MI TREND_JE TREND_MI TREND_JE_NATIVE TREND_JE_LAST_THIRD TREND_MI_LAST_THIRD RECLAIM_SEMANTICS_SYS JE_CONFIG LEVEL_JE LEVEL_MI VS_SYS_JE VS_SYS_MI VS_SYS_S_JE VS_SYS_S_MI EST_PROVISIONAL OPS_a2j OPS_RATIO_a2j_vs_4h VERDICT_JE VERDICT_MI RECLAIM_SEMANTICS_JE RECLAIM_SEMANTICS_MI JE_ESTIMATOR_AGREE JOURNAL PERF_JE PERF_MI SIZE_JE SIZE_HEADROOM_JE SIZE_MI SIZE_HEADROOM_MI BUILD_JE BUILD_MI DEFAULT_CANDIDATE NEXT" }
function stopr(msg) { stop = 1; nsr++; SR[nsr] = msg }

# The total order of R5.4, best first; 0 = not a verdict value.
function rank(v) {
  if (v == "PLATEAU") return 1; if (v == "BOUNDED_ABOVE_K") return 2; if (v == "MARGINAL") return 3
  if (v == "BOUNDED_NO_GAIN") return 4; if (v == "NOT_BOUNDED") return 5; if (isna(v)) return 6
  return 0
}
function is_survivor(arm, sv) { return (sv == arm || sv == "JE+MI") }

# The decision LIST of R8 plus DEFAULT_CANDIDATE's three steps. Inputs are the
# globals D_STOP, D_SURV and, per arm, D_V[], D_B[] (BETTER|NO_GAIN), D_F[]
# (AMP_FP), D_S[] (AMP_S), D_KV[], D_P[], D_BD[], plus D_EST. Sets CAND, NEXTL.
function decide_next(   ns, s, i, a, allna, allops, best, r, nh, h, differ, c, reason, big, fj, fm, sj, sm) {
  CAND = "NONE"; NEXTL = "CONDUCTOR_RULING;UNMATCHED"
  # Values read by substr() are strings; compare them as numbers, never lexically.
  fj = D_F["JE"] + 0; fm = D_F["MI"] + 0; sj = D_S["JE"] + 0; sm = D_S["MI"] + 0
  if (D_STOP == "TRUE") { CAND = "STOP"; NEXTL = "STOP"; return }                               # 1
  if (D_SURV == "NONE") { NEXTL = "CONDUCTOR_RULING;ALLOCATORS_WORSE_AT_900S"; return }          # 2
  ns = 0
  for (i = 1; i <= 2; i++) { a = (i == 1) ? "JE" : "MI"; if (is_survivor(a, D_SURV)) s[++ns] = a }
  if (ns == 0) return                                                                            # 17
  # A verdict token outside the printed domain is a state nobody anticipated:
  # report it through the catch-all rather than routing on the other arm.
  for (i = 1; i <= ns; i++) if (rank(D_V[s[i]]) == 0) return                                     # 17
  allna = 1; allops = 1
  for (i = 1; i <= ns; i++) { if (!isna(D_V[s[i]])) allna = 0; if (D_V[s[i]] != "n/a reason=ops") allops = 0 }
  if (allna && allops) { NEXTL = "CONDUCTOR_RULING;OPS"; return }                                # 3
  if (allna) {                                                                                   # 3b
    for (i = 1; i <= ns; i++) {
      if (D_V[s[i]] == "n/a reason=ops") continue
      reason = D_V[s[i]]; sub(/^n\/a reason=/, "", reason)
      if (reason == "cell_did_not_run") NEXTL = "CONDUCTOR_RULING;CELL_DID_NOT_RUN_NA"
      else if (reason == "few_points") NEXTL = "CONDUCTOR_RULING;FEW_POINTS_NA"
      else if (reason == "missing_reading") NEXTL = "CONDUCTOR_RULING;MISSING_READING_NA"
      return
    }
    return
  }
  best = 6
  for (i = 1; i <= ns; i++) { r = rank(D_V[s[i]]); if (r < best) best = r }
  nh = 0
  for (i = 1; i <= ns; i++) if (rank(D_V[s[i]]) == best) h[++nh] = s[i]
  differ = 0
  if (nh == 2 && num(D_F["JE"]) && num(D_F["MI"]) && num(D_S["JE"]) && num(D_S["MI"]))
    differ = (fj < fm && sj > sm) || (fj > fm && sj < sm)
  # DEFAULT_CANDIDATE, the three steps of R8.
  c = "NONE"
  if (best == 1 || best == 2) {
    if (nh == 1) c = h[1]
    else if (!differ && num(D_F["JE"]) && num(D_F["MI"])) {
      big = (fj > fm) ? fj : fm
      if (absd(fj - fm) <= 0.10 * big) c = "JE"
      else c = (fj < fm) ? "JE" : "MI"
    }
  }
  if (best == 5) {
    for (i = 1; i <= ns; i++) if (D_V[s[i]] == "NOT_BOUNDED" && D_B[s[i]] == "BETTER") { NEXTL = "CONDUCTOR_RULING;BETTER_NOT_BOUNDED"; return }   # 4
    NEXTL = "TODO-591;ALLOCATOR_INSUFFICIENT"; return                                            # 5
  }
  if (best == 4) { NEXTL = "CONDUCTOR_RULING;BOUNDED_NO_GAIN"; return }                          # 6
  if (best == 3) { NEXTL = "CONDUCTOR_RULING;TREND_MARGINAL"; return }                           # 7
  if ((best == 1 || best == 2) && nh == 2 && differ) { NEXTL = "CONDUCTOR_RULING;ARM_ORDER_ACCOUNTING"; return }   # 8
  if (c == "NONE") return                                                                        # 17
  if (best == 2) { CAND = c; NEXTL = "CONDUCTOR_RULING;BOUNDED_ABOVE_K"; return }                # 9
  if (best != 1) return                                                                          # 17
  if (D_KV[c] == "TRUE") { CAND = c; NEXTL = "CONDUCTOR_RULING;K_VACUOUS"; return }              # 10
  if (D_EST == "TRUE") { CAND = c; NEXTL = "CONDUCTOR_RULING;ESTIMATOR_UNVERIFIED"; return }     # 11
  if (isna(D_P[c])) { CAND = c; NEXTL = "CONDUCTOR_RULING;PERF_NA"; return }                     # 12
  if (D_P[c] == "FAIL") { CAND = c; NEXTL = "CONDUCTOR_RULING;PERF_VS_MEMORY"; return }          # 13
  if (D_P[c] == "PASS") {
    if (isna(D_BD[c])) { CAND = c; NEXTL = "CONDUCTOR_RULING;BUILD_NA"; return }                 # 14
    if (D_BD[c] == "PARTIAL" || D_BD[c] == "FAIL") { CAND = c; NEXTL = "CONDUCTOR_RULING;BUILD_STORY"; return }   # 15
    if (D_BD[c] == "OK") { CAND = c; NEXTL = "TODO-589;THEN;DEFAULT_FLIP;THEN;TODO-591"; return }  # 16
  }
  return                                                                                         # 17
}

# ------------------------------------------------------------------ input
mode == "list" {
  delete L
  for (i = 1; i <= NF; i++) { p = index($i, "="); if (p) { k = substr($i, 1, p - 1); v = substr($i, p + 1); sub(/^n\/a:/, "n/a reason=", v); L[k] = v } }
  D_STOP = L["STOP"]; D_SURV = L["S1_SURVIVORS"]; D_EST = L["EST_PROVISIONAL"]
  for (j = 1; j <= 2; j++) {
    a = (j == 1) ? "JE" : "MI"
    D_V[a] = L["VERDICT_" a]; D_B[a] = L["VS_SYS_" a]; D_F[a] = L["AMP_FP_" a]; D_S[a] = L["AMP_S_" a]
    D_KV[a] = L["K_HI_VACUOUS_" a]; D_P[a] = L["PERF_" a]; D_BD[a] = L["BUILD_" a]
  }
  decide_next()
  # The input rides along so a checker can read both from one stream.
  print "DEFAULT_CANDIDATE=" CAND " NEXT=" NEXTL " " $0
  next
}
FNR == 1 {
  f = FILENAME; sub(/.*\//, "", f); src = ""
  if (f ~ /^spec372-[a-z0-9]+\.predicates\.txt$/) { src = f; sub(/^spec372-/, "", src); sub(/\.predicates\.txt$/, "", src) }
  else if (f == "spec372.k-stage1.txt") src = "K1"
  else if (f == "spec372.k-stage2.txt") src = "K2"
  else if (f == "spec372.stage1.txt") src = "STAGE1"
  else if (f == "spec372-perf.txt") src = "PERF"
  else if (f == "spec372-buildstory.txt") src = "BUILD"
  seenfile[src] = 1
}
src != "" {
  p = index($0, "="); if (p == 0) next
  k = substr($0, 1, p - 1); if (k ~ / /) next
  v = substr($0, p + 1)
  if (!((src, k) in PF)) PF[src, k] = v
  sub(/ .*/, "", v)
  if (v == "n/a") { v = PF[src, k]; if (v !~ /^n\/a reason=[^ ]+/) v = "n/a"; else { sub(/^n\/a reason=/, "", v); sub(/ .*/, "", v); v = "n/a reason=" v } }
  if (!((src, k) in P)) P[src, k] = v
}

END {
  if (mode == "list") exit
  if (mode != "stage1" && mode != "stage2") { print "spec372-decide.awk: -v mode= must be stage1, stage2 or list" > "/dev/stderr"; exit 2 }
  stop = 0
  if (missing != "") stopr("missing inputs: " missing)
  if (!num(disk_free)) stopr("DISK_FREE_AT_START=" disk_free " (not a number)")
  else if (disk_free + 0 < 30) stopr("DISK_FREE_AT_START=" disk_free " < 30 GB")
  if (mode == "stage1") stage1(); else stage2()
  print "STOP=" (stop ? "TRUE" : "FALSE")
  for (i = 1; i <= nsr; i++) print "STOP-reason: " SR[i]
  for (i = 1; i <= npre; i++) print PK_[i] "=" (stop ? "STOP" : PV_[i])
  nk = split(R8_KEYS(), keys, " ")
  for (i = 1; i <= nk; i++) print keys[i] "=" (stop ? "STOP" : ((keys[i] in VAL) ? VAL[keys[i]] : ((mode == "stage1") ? "n/a reason=stage1" : "n/a reason=missing_reading")))
}

# The per-cell STOP predicates of R6. PA may read n/a only on SYS and MI cells.
function cell_stops(c, fl,   n, ks, i, x, pass) {
  if (!(c in seenfile)) { stopr("no predicates file for " c); return }
  n = split("PV PR-crashes PR-class P5 P6 P7 PE PA PJ PC PM1", ks, " ")
  for (i = 1; i <= n; i++) {
    x = get(c, ks[i], "<missing>")
    pass = (x == "TRUE") || (ks[i] == "PA" && isna(x) && (fl == "SYS" || fl == "MI"))
    if (!pass) stopr(c " " ks[i] "=" x)
  }
  if (!has(c, "WRITE_ERRORS")) stopr(c " WRITE_ERRORS=" get(c, "WRITE_ERRORS", "<missing>") " (not a number)")
  else { werr += P[c, "WRITE_ERRORS"]; if (P[c, "WRITE_ERRORS"] + 0 > 0) stopr(c " WRITE_ERRORS=" P[c, "WRITE_ERRORS"]) }
}
function ratio(a, b) { return (num(a) && num(b) && b + 0 > 0) ? r4(a / b) : "n/a reason=missing_reading" }
function min2(c1, c2, k) { return (has(c1, k) && has(c2, k)) ? ((P[c1, k] + 0 < P[c2, k] + 0) ? P[c1, k] : P[c2, k]) : "" }
function ktext(src, k) { return getfull(src, k, "n/a reason=missing_reading") }

# ------------------------------------------------------------------ Stage 1
function stage1(   cells, n, i, c, fl, sfp, ss, arm, c1, c2, dr, sv, okr) {
  n = split("s1a j1a m1a k1 j1b m1b s1b", cells, " ")
  for (i = 1; i <= n; i++) {
    c = cells[i]; fl = (c ~ /^s/) ? "SYS" : ((c ~ /^j/) ? "JE" : ((c ~ /^m/) ? "MI" : "CA"))
    cell_stops(c, fl)
  }
  if (!("K1" in seenfile)) stopr("no spec372.k-stage1.txt")
  okr = has("s1a", "FP_end") && has("s1b", "FP_end") && has("s1a", "S_end") && has("s1b", "S_end") && has("j1a", "FP_end") && has("j1b", "FP_end") && has("j1a", "S_end") && has("j1b", "S_end") && has("m1a", "FP_end") && has("m1b", "FP_end") && has("m1a", "S_end") && has("m1b", "S_end")
  if (!okr) stopr("an FP_end or S_end reading the DROP rule needs is missing")

  out("WRITE_ERRORS", werr + 0)
  out("DISK_FREE_AT_START", disk_free)
  for (i = 1; i <= n; i++) {
    c = cells[i]
    pre("CELL_" c, "FP_end=" get(c, "FP_end", "n/a") " S_end=" get(c, "S_end", "n/a") " RECLAIM_end=" get(c, "RECLAIM_end", "n/a") \
        " FP_slope=" get(c, "FP_slope", "n/a") " S_slope=" get(c, "S_slope", "n/a") " AMP_FP_TERM=" get(c, "TERM_AMP_FP", "n/a") \
        " AMP_S_TERM=" get(c, "TERM_AMP_S", "n/a") " OPS_PER_S=" get(c, "OPS_PER_S", "n/a") " join_lag_s=" get(c, "TERM_join_lag_s", "n/a"))
    pre("HOST_" c, getfull(c, "HOST", "n/a reason=no_host_record"))
  }
  out("OPS_RATIO_JE_S1", ratio(min2("j1a", "j1b", "OPS_PER_S"), min2("s1a", "s1b", "OPS_PER_S")))
  out("OPS_RATIO_MI_S1", ratio(min2("m1a", "m1b", "OPS_PER_S"), min2("s1a", "s1b", "OPS_PER_S")))

  # R4.3, authoritative here: DROPPED iff strictly worse than the WORST SYS
  # cell on FP_end AND S_end, in BOTH replicates.
  sv = "n/a reason=missing_reading"
  if (okr) {
    sfp = (P["s1a", "FP_end"] + 0 > P["s1b", "FP_end"] + 0) ? P["s1a", "FP_end"] + 0 : P["s1b", "FP_end"] + 0
    ss  = (P["s1a", "S_end"] + 0  > P["s1b", "S_end"] + 0)  ? P["s1a", "S_end"] + 0  : P["s1b", "S_end"] + 0
    sv = ""
    for (i = 1; i <= 2; i++) {
      arm = (i == 1) ? "JE" : "MI"; c1 = (i == 1) ? "j1a" : "m1a"; c2 = (i == 1) ? "j1b" : "m1b"
      dr = (P[c1, "FP_end"] + 0 > sfp) && (P[c1, "S_end"] + 0 > ss) && (P[c2, "FP_end"] + 0 > sfp) && (P[c2, "S_end"] + 0 > ss)
      pre("S1_VERDICT_" arm, (dr ? "DROP" : "SURVIVE") " worst_SYS_FP_end=" sfp " worst_SYS_S_end=" ss)
      if (!dr) sv = (sv == "") ? arm : sv "+" arm
    }
    if (sv == "") sv = "NONE"
  }
  out("S1_SURVIVORS", sv)
  out("S1_RANK", ktext("K1", "S1_RANK"))
  out("CHURN_RATIO", ktext("K1", "CHURN_RATIO")); out("CHURN_RATIO_DRIFT", ktext("K1", "CHURN_RATIO_DRIFT"))
  out("K_PROVISIONAL", ktext("K1", "K_PROVISIONAL"))
  out("T_DECAY_UPPER_JE", ktext("K1", "T_DECAY_UPPER_JE")); out("T_DECAY_UPPER_MI", ktext("K1", "T_DECAY_UPPER_MI"))
  out("CV_JE", ktext("K1", "CV_JE")); out("CV_MI", ktext("K1", "CV_MI"))
  for (i = 1; i <= 2; i++) {
    c = (i == 1) ? "j1a" : "j1b"
    out("DIRTY_SHARE_" c, get(c, "TERM_DIRTY_SHARE", "n/a reason=missing_reading"))
    out("FRAG_SHARE_" c, get(c, "TERM_FRAG_SHARE", "n/a reason=missing_reading"))
    out("EST_AGREE_900_" c, ktext("K1", "EST_AGREE_900_" c))
    out("UNMODELLED_REACHABLE_" c, has(c, "TERM_UNMODELLED_MB") ? P[c, "TERM_UNMODELLED_MB"] " " P[c, "TERM_UNMODELLED_SHARE"] : "n/a reason=missing_reading")
    pre("JE_CONFIG_" c, getfull(c, "JE_CONFIG", "n/a reason=missing_reading"))
  }
}

# ------------------------------------------------------------------ Stage 2
function trend(c, pre,   lh, used, sl, se) {
  used = get(c, "TREND_points_used", "")
  lh = has(c, pre "_n") ? P[c, pre "_n"] + 0 : (num(used) ? used - int(used / 2) : -1)
  if (pre == "TREND" && lh >= 0 && lh < 12) return "n/a reason=few_points n=" lh
  if (!(has(c, pre) && has(c, pre "_se"))) return (lh >= 0 && lh < 12) ? "n/a reason=few_points n=" lh : "n/a reason=missing_reading"
  sl = P[c, pre] + 0; se = P[c, pre "_se"] + 0
  return ((sl <= se) ? "NON_INCREASING" : ((sl <= 2 * se) ? "MARGINAL" : "INCREASING")) \
         " slope_amp_per_hour=" P[c, pre] " se=" P[c, pre "_se"] " n=" get(c, pre "_n", "n/a") " r2=" get(c, pre "_r2", "n/a") \
         ((pre == "TREND") ? " points_dropped=" get(c, "TREND_dropped", "n/a") : "")
}
function reclaim(c) { return has(c, "RECLAIM_RATIO_end") ? ((P[c, "RECLAIM_RATIO_end"] + 0 <= 0.02) ? "MADV_FREE" : "REUSABLE") " ratio=" P[c, "RECLAIM_RATIO_end"] : "UNKNOWN" }
function stage2(   sv, i, n, ks, arm, c, cells, t, tv, amp, vs, vss, opsr, v, agree, pj, rr) {
  sv = get("STAGE1", "S1_SURVIVORS", "missing")
  if (!("STAGE1" in seenfile)) stopr("no spec372.stage1.txt")
  else if (get("STAGE1", "STOP", "missing") != "FALSE") stopr("spec372.stage1.txt STOP=" get("STAGE1", "STOP", "missing"))
  if (sv != "NONE" && sv != "JE" && sv != "MI" && sv != "JE+MI") stopr("S1_SURVIVORS=" sv " (not a survivor set)")
  if (has("STAGE1", "WRITE_ERRORS")) werr += P["STAGE1", "WRITE_ERRORS"]

  if (sv == "NONE") {
    out("WRITE_ERRORS", werr + 0); out("DISK_FREE_AT_START", disk_free)
    n = split("OPS_RATIO_JE_S1 OPS_RATIO_MI_S1", ks, " ")
    for (i = 1; i <= n; i++) out(ks[i], get("STAGE1", ks[i], "n/a reason=missing_reading"))
    n = split("S1_SURVIVORS S1_RANK CHURN_RATIO CHURN_RATIO_DRIFT K_PROVISIONAL T_DECAY_UPPER_JE T_DECAY_UPPER_MI CV_JE CV_MI", ks, " ")
    for (i = 1; i <= n; i++) out(ks[i], getfull("STAGE1", ks[i], "n/a reason=missing_reading"))
    s1_je_block()
    n = split(R8_KEYS(), ks, " ")
    for (i = 1; i <= n; i++) if (!(ks[i] in VAL)) out(ks[i], "n/a reason=stage1")
    D_STOP = stop ? "TRUE" : "FALSE"; D_SURV = sv; decide_next()
    out("DEFAULT_CANDIDATE", CAND); out("NEXT", NEXTL)
    return
  }

  # The same-chain SYS reference is not optional; a survivor's own cell that
  # did not run is a verdict n/a, not a STOP.
  cell_stops("s2", "SYS")
  for (i = 1; i <= 2; i++) {
    arm = (i == 1) ? "JE" : "MI"; c = (i == 1) ? "j2" : "m2"
    if (is_survivor(arm, sv) && (c in seenfile)) cell_stops(c, arm)
  }
  if (a2j_state == "ran") cell_stops("a2j", a2j_flavour)
  if (!("K2" in seenfile)) stopr("no spec372.k-stage2.txt")

  out("WRITE_ERRORS", werr + 0)
  out("DISK_FREE_AT_START", disk_free)
  out("OPS_RATIO_JE_S1", get("STAGE1", "OPS_RATIO_JE_S1", "n/a reason=missing_reading"))
  out("OPS_RATIO_MI_S1", get("STAGE1", "OPS_RATIO_MI_S1", "n/a reason=missing_reading"))
  for (i = 1; i <= 2; i++) {
    arm = (i == 1) ? "JE" : "MI"; c = (i == 1) ? "j2" : "m2"
    if (!is_survivor(arm, sv)) opsr[arm] = "n/a reason=dropped_stage1"
    else if (!(c in seenfile)) opsr[arm] = "n/a reason=cell_did_not_run"
    else opsr[arm] = ratio(get(c, "OPS_PER_S", ""), get("s2", "OPS_PER_S", ""))
    out("OPS_RATIO_" arm "_S2", opsr[arm])
  }
  n = split("S1_SURVIVORS S1_RANK CHURN_RATIO CHURN_RATIO_DRIFT K_PROVISIONAL T_DECAY_UPPER_JE T_DECAY_UPPER_MI CV_JE CV_MI", ks, " ")
  for (i = 1; i <= n; i++) out(ks[i], getfull("STAGE1", ks[i], "n/a reason=missing_reading"))
  n = split("K_lo_JE K_hi_JE K_lo_MI K_hi_MI R_redb_s2 R_redb_j2 R_redb_m2 K_HI_VACUOUS_JE K_HI_VACUOUS_MI", ks, " ")
  for (i = 1; i <= n; i++) out(ks[i], getfull("K2", ks[i], "n/a reason=missing_reading"))
  s1_je_block()
  out("DIRTY_SHARE_j2", get("j2", "DECIDE_DIRTY_SHARE", "n/a reason=missing_reading"))
  out("FRAG_SHARE_j2", get("j2", "DECIDE_FRAG_SHARE", "n/a reason=missing_reading"))
  out("UNMODELLED_REACHABLE_j2", has("j2", "DECIDE_UNMODELLED_MB") ? P["j2", "DECIDE_UNMODELLED_MB"] " " P["j2", "DECIDE_UNMODELLED_SHARE"] : "n/a reason=missing_reading")
  amp["S2F"] = get("s2", "DECIDE_AMP_FP", "n/a reason=missing_reading"); amp["S2S"] = get("s2", "DECIDE_AMP_S", "n/a reason=missing_reading")
  out("AMP_FP_S2", amp["S2F"]); out("AMP_S_S2", amp["S2S"])
  for (i = 1; i <= 2; i++) {
    arm = (i == 1) ? "JE" : "MI"; c = (i == 1) ? "j2" : "m2"
    amp[arm "F"] = (is_survivor(arm, sv) && (c in seenfile)) ? get(c, "DECIDE_AMP_FP", "n/a reason=missing_reading") : "n/a reason=cell_did_not_run"
    amp[arm "S"] = (is_survivor(arm, sv) && (c in seenfile)) ? get(c, "DECIDE_AMP_S", "n/a reason=missing_reading") : "n/a reason=cell_did_not_run"
    out("AMP_FP_" arm, amp[arm "F"]); out("AMP_S_" arm, amp[arm "S"])
    if (arm == "JE") out("AMP_JE", get("j2", "DECIDE_AMP_JE", "n/a reason=missing_reading"))
  }
  for (i = 1; i <= 2; i++) {
    arm = (i == 1) ? "JE" : "MI"; c = (i == 1) ? "j2" : "m2"
    t[arm] = (is_survivor(arm, sv) && (c in seenfile)) ? trend(c, "TREND") : "n/a reason=cell_did_not_run"
    out("TREND_" arm, t[arm])
  }
  out("TREND_JE_NATIVE", ("j2" in seenfile) ? trend("j2", "TREND_NATIVE") : "n/a reason=cell_did_not_run")
  out("TREND_JE_LAST_THIRD", ("j2" in seenfile) ? trend("j2", "TREND3") : "n/a reason=cell_did_not_run")
  out("TREND_MI_LAST_THIRD", ("m2" in seenfile) ? trend("m2", "TREND3") : "n/a reason=cell_did_not_run")
  out("RECLAIM_SEMANTICS_SYS", reclaim("s2"))
  out("JE_CONFIG", getfull("j2", "JE_CONFIG", "n/a reason=cell_did_not_run"))
  for (i = 1; i <= 2; i++) { arm = (i == 1) ? "JE" : "MI"; out("LEVEL_" arm, get("K2", "LEVEL_" arm, "n/a reason=missing_reading")) }
  for (i = 1; i <= 2; i++) {
    arm = (i == 1) ? "JE" : "MI"
    if (num(amp[arm "F"]) && num(amp["S2F"]) && amp["S2F"] + 0 > 0) {
      vs[arm] = amp[arm "F"] / amp["S2F"]; D_B[arm] = (vs[arm] <= 0.50) ? "BETTER" : "NO_GAIN"
      out("VS_SYS_" arm, r4(vs[arm]) " " D_B[arm])
    } else { D_B[arm] = ""; out("VS_SYS_" arm, "n/a reason=missing_reading") }
  }
  for (i = 1; i <= 2; i++) { arm = (i == 1) ? "JE" : "MI"; out("VS_SYS_S_" arm, ratio(amp[arm "S"], amp["S2S"])) }
  agree = has("j2", "DECIDE_EST_AGREE") ? ((absd(P["j2", "DECIDE_EST_AGREE"] - 1) <= 0.25) ? "TRUE" : "FALSE") : "n/a"
  out("EST_PROVISIONAL", (agree == "TRUE") ? "FALSE" : "TRUE")
  if (a2j_state == "ran") {
    out("OPS_a2j", get("a2j", "OPS_PER_S", "n/a reason=missing_reading"))
    out("OPS_RATIO_a2j_vs_4h", ratio(get("a2j", "OPS_PER_S", ""), get((a2j_flavour == "JE") ? "j2" : "m2", "OPS_AT_900", "")))
  } else { out("OPS_a2j", "n/a reason=" ((a2j_state == "budget") ? "budget" : "cell_did_not_run")); out("OPS_RATIO_a2j_vs_4h", "n/a reason=" ((a2j_state == "budget") ? "budget" : "cell_did_not_run")) }

  # VERDICT per arm, over the total order of R5.4. n/a reasons in precedence:
  # dropped_stage1, cell_did_not_run, ops, missing_reading, few_points.
  for (i = 1; i <= 2; i++) {
    arm = (i == 1) ? "JE" : "MI"; c = (i == 1) ? "j2" : "m2"
    lv = get("K2", "LEVEL_" arm, "")
    if (!is_survivor(arm, sv)) v = "n/a reason=dropped_stage1"
    else if (!(c in seenfile)) v = "n/a reason=cell_did_not_run"
    else if (!num(opsr[arm])) v = "n/a reason=missing_reading"
    else if (opsr[arm] + 0 < 0.95) v = "n/a reason=ops"
    else if (!num(amp[arm "F"]) || !num(amp[arm "S"]) || !num(amp["S2F"]) || !num(amp["S2S"]) || D_B[arm] == "" || (lv != "WITHIN_K_HI" && lv != "ABOVE_K_HI")) v = "n/a reason=missing_reading"
    else if (t[arm] ~ /^n\/a reason=few_points/) v = "n/a reason=few_points"
    else if (t[arm] ~ /^n\/a/) v = "n/a reason=missing_reading"
    else {
      tv = t[arm]; sub(/ .*/, "", tv)
      if (tv == "NON_INCREASING" && D_B[arm] == "BETTER") v = (lv == "WITHIN_K_HI") ? "PLATEAU" : "BOUNDED_ABOVE_K"
      else if (tv == "MARGINAL" && D_B[arm] == "BETTER") v = "MARGINAL"
      else if (tv == "NON_INCREASING") v = "BOUNDED_NO_GAIN"
      else v = "NOT_BOUNDED"
    }
    D_V[arm] = v; out("VERDICT_" arm, v)
  }
  out("RECLAIM_SEMANTICS_JE", ("j2" in seenfile) ? reclaim("j2") : "n/a reason=cell_did_not_run")
  out("RECLAIM_SEMANTICS_MI", ("m2" in seenfile) ? reclaim("m2") : "n/a reason=cell_did_not_run")
  out("JE_ESTIMATOR_AGREE", (agree == "n/a") ? "n/a" : agree " ratio=" P["j2", "DECIDE_EST_AGREE"])
  out("JOURNAL", (a2j_state == "ran") ? "recorded" : ((a2j_state == "budget") ? "n/a reason=budget" : "n/a reason=cell_did_not_run"))
  for (i = 1; i <= 2; i++) { arm = (i == 1) ? "JE" : "MI"; D_P[arm] = get("PERF", "PERF_" arm, "n/a"); out("PERF_" arm, D_P[arm]) }
  for (i = 1; i <= 2; i++) {
    arm = (i == 1) ? "JE" : "MI"
    out("SIZE_" arm, getfull("BUILD", "SIZE_" arm, "n/a")); out("SIZE_HEADROOM_" arm, getfull("BUILD", "SIZE_HEADROOM_" arm, "n/a"))
  }
  for (i = 1; i <= 2; i++) { arm = (i == 1) ? "JE" : "MI"; D_BD[arm] = get("BUILD", "BUILD_" arm, "n/a"); out("BUILD_" arm, D_BD[arm]) }
  D_STOP = stop ? "TRUE" : "FALSE"; D_SURV = sv; D_EST = (agree == "TRUE") ? "FALSE" : "TRUE"
  for (i = 1; i <= 2; i++) { arm = (i == 1) ? "JE" : "MI"; D_F[arm] = amp[arm "F"]; D_S[arm] = amp[arm "S"]; D_KV[arm] = get("K2", "K_HI_VACUOUS_" arm, "n/a") }
  decide_next()
  out("DEFAULT_CANDIDATE", CAND); out("NEXT", NEXTL)
}
# The Stage-1 JE readings, carried from spec372.stage1.txt into the decision.
function s1_je_block(   i, c, n, ks) {
  n = split("DIRTY_SHARE_j1a FRAG_SHARE_j1a EST_AGREE_900_j1a UNMODELLED_REACHABLE_j1a DIRTY_SHARE_j1b FRAG_SHARE_j1b EST_AGREE_900_j1b UNMODELLED_REACHABLE_j1b", ks, " ")
  for (i = 1; i <= n; i++) out(ks[i], getfull("STAGE1", ks[i], "n/a reason=missing_reading"))
}
