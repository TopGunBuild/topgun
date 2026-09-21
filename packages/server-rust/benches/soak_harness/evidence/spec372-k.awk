# spec372 bounds and Stage-1 readings. Pre-registered in spec372-manifest.md §1;
# this program's bytes are bound at M, so a value it computes after the data
# exists is still pre-registered.
#
# usage (from an environment that exports LC_ALL=C):
#   awk -v mode=stage1 -f spec372-k.awk \
#       spec372-{s1a,j1a,m1a,k1,j1b,m1b,s1b}.predicates.txt spec372-k1.csv
#   awk -v mode=stage2 -f spec372-k.awk \
#       spec372.stage1.txt spec372-{s2,j2,m2}.predicates.txt
#
# stage1 (run at M2 over D1) prints the Stage-1 frozen READINGS: S1_SURVIVORS,
#   S1_RANK, CHURN_RATIO, CHURN_RATIO_DRIFT, K_PROVISIONAL, T_DECAY_UPPER_<arm>,
#   CV_<arm>, EST_AGREE_900_<cell>.
# stage2 (run at D2 over D2) prints the per-arm bounds and the level: K_lo_<arm>,
#   K_hi_<arm>, R_redb_<cell>, K_HI_VACUOUS_<arm>, LEVEL_<arm>.
#
# Inputs are files named by cell; a missing file or reading prints the key as
# n/a with a reason and is never defaulted. Every line of a predicates file is
# NAME=VALUE; the value is the first space-delimited token after the "=".

function num(s) { return (s ~ /^-?[0-9]+(\.[0-9]+)?([eE][-+]?[0-9]+)?$/) }
function r4(x) { return sprintf("%.4f", x) }
function has(c, k) { return ((c, k) in P) && num(P[c, k]) }
function absd(x) { return (x < 0) ? -x : x }

FNR == 1 {
  f = FILENAME; sub(/.*\//, "", f); src = ""
  if (f ~ /^spec372-[a-z0-9]+\.predicates\.txt$/) { src = f; sub(/^spec372-/, "", src); sub(/\.predicates\.txt$/, "", src) }
  else if (f == "spec372-k1.csv") src = "K1CSV"
  else if (f == "spec372.stage1.txt") src = "STAGE1"
  seen[src] = 1
}
src == "K1CSV" && FNR == 1 { nc = split($0, hd, ","); for (i = 1; i <= nc; i++) col[hd[i]] = i; next }
src == "K1CSV" {
  # One point per DISTINCT probe line: a CSV row repeats the last probe line
  # seen, and the post-mortem row repeats the final one.
  if (!(("bytes_alloc" in col) && ("alloc_live_bytes" in col) && ("alloc_probe_elapsed_s" in col))) next
  split($0, cv, ",")
  b = cv[col["bytes_alloc"]]; l = cv[col["alloc_live_bytes"]]; e = cv[col["alloc_probe_elapsed_s"]]
  if (!num(b) || !num(l) || !num(e) || (np > 0 && e + 0 == ke[np])) next
  np++; kb[np] = b + 0; kl[np] = l + 0; ke[np] = e + 0
  next
}
src != "" {
  p = index($0, "="); if (p == 0) next
  k = substr($0, 1, p - 1); v = substr($0, p + 1); sub(/ .*/, "", v)
  if (!((src, k) in P)) P[src, k] = v
}

END {
  if (mode == "stage1") stage1(); else if (mode == "stage2") stage2()
  else { print "spec372-k.awk: -v mode= must be stage1 or stage2" > "/dev/stderr"; exit 2 }
}

# ------------------------------------------------------------------ Stage 1
function survivors(   sfp, ss, ok, arm, a, c1, c2, out) {
  ok = has("s1a", "FP_end") && has("s1b", "FP_end") && has("s1a", "S_end") && has("s1b", "S_end")
  if (!ok) return "n/a reason=missing_reading"
  sfp = (P["s1a", "FP_end"] + 0 > P["s1b", "FP_end"] + 0) ? P["s1a", "FP_end"] + 0 : P["s1b", "FP_end"] + 0
  ss  = (P["s1a", "S_end"] + 0  > P["s1b", "S_end"] + 0)  ? P["s1a", "S_end"] + 0  : P["s1b", "S_end"] + 0
  out = ""
  for (a = 1; a <= 2; a++) {
    arm = (a == 1) ? "JE" : "MI"; c1 = (a == 1) ? "j1a" : "m1a"; c2 = (a == 1) ? "j1b" : "m1b"
    if (!(has(c1, "FP_end") && has(c2, "FP_end") && has(c1, "S_end") && has(c2, "S_end"))) return "n/a reason=missing_reading"
    # DROPPED iff strictly worse than the WORST SYS cell on both readings, in both replicates.
    dropped[arm] = (P[c1, "FP_end"] + 0 > sfp) && (P[c1, "S_end"] + 0 > ss) && (P[c2, "FP_end"] + 0 > sfp) && (P[c2, "S_end"] + 0 > ss)
    if (!dropped[arm]) out = (out == "") ? arm : out "+" arm
  }
  return (out == "") ? "NONE" : out
}
function mean2(c1, c2, k) { return (P[c1, k] + P[c2, k]) / 2 }
function stage1(   sv, aj, am, sj, sm, big, n, i, h, cf, cl, drift, churn, arm, c1, c2, m, sd) {
  sv = survivors()
  print "S1_SURVIVORS=" sv

  # The a2j selector: lower mean TERMINAL AMP_FP; within 10 % of the larger ->
  # lower mean S slope; equal slopes -> JE.
  if (sv == "JE" || sv == "MI") print "S1_RANK=" sv
  else if (sv == "JE+MI") {
    if (!(has("j1a", "TERM_AMP_FP") && has("j1b", "TERM_AMP_FP") && has("m1a", "TERM_AMP_FP") && has("m1b", "TERM_AMP_FP"))) print "S1_RANK=n/a reason=missing_reading"
    else {
      aj = mean2("j1a", "j1b", "TERM_AMP_FP"); am = mean2("m1a", "m1b", "TERM_AMP_FP"); big = (aj > am) ? aj : am
      if (absd(aj - am) > 0.10 * big) print "S1_RANK=" ((aj < am) ? "JE" : "MI")
      else if (!(has("j1a", "S_slope") && has("j1b", "S_slope") && has("m1a", "S_slope") && has("m1b", "S_slope"))) print "S1_RANK=n/a reason=missing_reading"
      else {
        sj = mean2("j1a", "j1b", "S_slope"); sm = mean2("m1a", "m1b", "S_slope")
        print "S1_RANK=" ((sm < sj) ? "MI" : "JE")
      }
    }
  } else print "S1_RANK=n/a reason=" ((sv == "NONE") ? "no_survivor" : "missing_reading")

  # Churn relative to the live heap, on the count-alloc cell's own row clock:
  # (bytes allocated over the window / window seconds) / live bytes at the
  # window's last point. Last half = points floor(n/2)..n-1; first half =
  # points 0..floor(n/2), sharing the boundary so the two windows abut.
  n = np
  if (!("K1CSV" in seen) || n < 3) {
    print "CHURN_RATIO=n/a reason=too_few_probe_points points=" n + 0
    print "CHURN_RATIO_DRIFT=n/a reason=too_few_probe_points"
    print "K_PROVISIONAL=n/a reason=too_few_probe_points"
    print "T_DECAY_UPPER_JE=n/a reason=too_few_probe_points"
    print "T_DECAY_UPPER_MI=n/a reason=too_few_probe_points"
  } else {
    h = int((n - 1) / 2) + 1          # 1-based index of the boundary point
    cl = ((kb[n] - kb[h]) / (ke[n] - ke[h])) / kl[n]
    cf = ((kb[h] - kb[1]) / (ke[h] - ke[1])) / kl[h]
    drift = cf / cl
    print "CHURN_RATIO=" r4(cl) " points=" n " window_s=" ke[h] "-" ke[n]
    print "CHURN_RATIO_DRIFT=" r4(drift)
    print "K_PROVISIONAL=" ((drift < 0.67 || drift > 1.5) ? "TRUE" : "FALSE")
    print "T_DECAY_UPPER_JE=" r4(10 * cl)
    print "T_DECAY_UPPER_MI=" r4(1 * cl)
  }

  # Sample (n - 1) coefficient of variation of the two replicates' TERMINAL AMP_FP.
  for (i = 1; i <= 2; i++) {
    arm = (i == 1) ? "JE" : "MI"; c1 = (i == 1) ? "j1a" : "m1a"; c2 = (i == 1) ? "j1b" : "m1b"
    if (has(c1, "TERM_AMP_FP") && has(c2, "TERM_AMP_FP")) {
      m = mean2(c1, c2, "TERM_AMP_FP"); sd = absd(P[c1, "TERM_AMP_FP"] - P[c2, "TERM_AMP_FP"]) / sqrt(2)
      print "CV_" arm "=" ((m > 0) ? r4(sd / m) : "n/a reason=zero_mean")
    } else print "CV_" arm "=n/a reason=missing_reading"
  }

  for (i = 1; i <= 2; i++) {
    c1 = (i == 1) ? "j1a" : "j1b"
    print "EST_AGREE_900_" c1 "=" (has(c1, "TERM_EST_AGREE") ? r4(P[c1, "TERM_EST_AGREE"]) : "n/a reason=missing_reading")
  }
}

# ------------------------------------------------------------------ Stage 2
function stage2(   sv, s2amp, i, arm, c, rmeta, klo, khi, fcls) {
  sv = ((("STAGE1", "S1_SURVIVORS") in P) ? P["STAGE1", "S1_SURVIVORS"] : "missing")
  s2amp = has("s2", "DECIDE_AMP_FP") ? P["s2", "DECIDE_AMP_FP"] + 0 : ""
  for (i = 1; i <= 3; i++) {
    c = (i == 1) ? "s2" : ((i == 2) ? "j2" : "m2")
    if (!(c in seen)) print "R_redb_" c "=n/a reason=cell_did_not_run"
    else print "R_redb_" c "=" (has(c, "DECIDE_R_redb") ? r4(P[c, "DECIDE_R_redb"]) : "n/a reason=missing_reading")
  }
  for (i = 1; i <= 2; i++) {
    arm = (i == 1) ? "JE" : "MI"; c = (i == 1) ? "j2" : "m2"
    # F_class: the worst-case size-class rounding (jemalloc classes 1/4 of the
    # preceding power of two apart, mimalloc bins 1/8 apart).
    fcls = (arm == "JE") ? 0.25 : 0.125
    why = ""
    if (sv != arm && sv != "JE+MI") why = (sv == "missing") ? "missing_reading" : "dropped_stage1"
    else if (!(c in seen)) why = "cell_did_not_run"
    else if (!(has(c, "DECIDE_A0_share") && has(c, "DECIDE_R_redb") && has(c, "DECIDE_AMP_FP"))) why = "missing_reading"
    else if (arm == "JE" && !has(c, "DECIDE_R_meta")) why = "missing_reading"
    if (why != "") {
      print "K_lo_" arm "=n/a reason=" why; print "K_hi_" arm "=n/a reason=" why
      print "K_HI_VACUOUS_" arm "=n/a reason=" why; print "LEVEL_" arm "=n/a reason=" why
      continue
    }
    # R_meta: jemalloc's own stats.metadata on JE; mimalloc exposes no safe
    # metadata reading, so MI carries the frozen 0.05 literal.
    rmeta = (arm == "JE") ? P[c, "DECIDE_R_meta"] + 0 : 0.05
    klo = 1 + P[c, "DECIDE_A0_share"] + rmeta
    khi = klo + fcls + P[c, "DECIDE_R_redb"]
    print "K_lo_" arm "=" r4(klo)
    print "K_hi_" arm "=" r4(khi)
    print "K_HI_VACUOUS_" arm "=" ((s2amp == "") ? "n/a reason=missing_reading" : ((khi >= 0.50 * s2amp) ? "TRUE" : "FALSE"))
    print "LEVEL_" arm "=" ((P[c, "DECIDE_AMP_FP"] + 0 <= khi) ? "WITHIN_K_HI" : "ABOVE_K_HI")
  }
}
