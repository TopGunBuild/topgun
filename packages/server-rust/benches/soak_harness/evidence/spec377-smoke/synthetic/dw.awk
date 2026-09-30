function reset(   i, c) {
  delete R; delete X; delete MISS; delete MAN
  for (i = 1; i <= 5; i++) {
    c = CELL[i]
    R[c, "PV"] = "TRUE server_sha256=synthetic"; R[c, "PEL"] = "TRUE rows_with_fp_equiv=360"; R[c, "PM1"] = "TRUE post_mortem_rows=0"
    R[c, "PMEM"] = "TRUE mem_invariant_violations=0 sampler_fatal=0"; R[c, "PALLOC"] = "TRUE label=synthetic"
    R[c, "PA"] = (c == "je") ? "TRUE wellformed_lines=720 lines=720 need=718" : "n/a reason=no_probe_arm"
    R[c, "PR-crashes"] = "TRUE crashes=0"; R[c, "WRITE_ERRORS"] = "0"
    R[c, "TREND"] = "FLAT slope_rel=0.001000 se_rel=0.001000 r1=0.1000 se_rel_adj=0.001100 n=176 r2=0.100000 dropped=1"
    R[c, "TREND3"] = "FLAT slope_rel=0.001000 se_rel=0.001000 r1=0.1000 se_rel_adj=0.001100 n=117"
    R[c, "TREND_CORR"] = "FLAT slope_rel_corr=0.001000 fixed_bias=0.000000 recorded_only"
    R[c, "STAGE2_T"] = "6"; R[c, "FIXED_EST"] = "1.000 se_mib=2.000"; R[c, "FIXED_NOTE"] = "none"
    R[c, "LIVE_END"] = "2220000 src=TERMINAL t=21602.0"; R[c, "PE_END"] = "1000.000000 t=21300.0 live=2189166"
    R[c, "LAZY_MAX"] = "0.000 ratio=0.000000 row=60"; R[c, "HWM_END"] = "3000.000"; R[c, "ANON_HUGE_END"] = "0.000"
    R[c, "DISK_SLOPE"] = "3.600000 se=0.100000 n=176"
    X["steal", c] = "0.0000"; X["runner_exit", c] = "0"; X["tw", c] = "5564160"; X["dur", c] = "21600"; X["predrc", c] = "0"
  }
  R["mi3", "TREND"] = "MARGINAL slope_rel=0.004000 se_rel=0.003000 r1=0.3000 se_rel_adj=0.004000 n=176 r2=0.100000 dropped=1"
  R["mi2", "TREND"] = R["mi3", "TREND"]
  lv["s1"] = 1000; lv["s2"] = 1050; lv["je"] = 600; lv["mi3"] = 900; lv["mi2"] = 950
  for (i = 1; i <= 5; i++) { c = CELL[i]; R[c, "PE_LEVEL"] = lv[c] " rows=26"; R[c, "RSS_PE_LEVEL"] = sprintf("%.3f rows=26", lv[c] * 1.02) }
  X["mipost", "mi3"] = "0"; X["mipost", "mi2"] = "0"
  X["chain", "disk"] = "80000000"; X["chain", "preflight"] = "PREFLIGHT=PASS PREFLIGHT_AT=2026-09-30T00:00:00Z"
}
function out(f, s) { if (s != "@absent") print s > f }
function flush(   i, c, b, k, j, v, pf, ch, np, NP, NR_, RD) {
  if (W == "") return
  system("mkdir -p \"" W "/ev\" \"" W "/smoke\"")
  np = split("PV PEL PA PM1 PMEM PALLOC PR-crashes WRITE_ERRORS", NP, " ")
  NR_ = split("LIVE_END PE_END PE_LEVEL RSS_PE_LEVEL TREND TREND3 TREND_CORR STAGE2_T FIXED_EST FIXED_NOTE LAZY_MAX HWM_END ANON_HUGE_END DISK_SLOPE", RD, " ")
  for (i = 1; i <= 5; i++) {
    c = CELL[i]; b = W "/ev/spec377-" c
    if (!(c in MISS)) {
      f = b ".predicates.txt"; printf "" > f
      for (j = 1; j <= np; j++) if (R[c, NP[j]] != "@absent") print NP[j] "=" R[c, NP[j]] > f
      for (j = 1; j <= NR_; j++) if (R[c, RD[j]] != "@absent") print RD[j] "_" c "=" R[c, RD[j]] > f
      if (c == "je") {
        print "JE_CONFIG=je_config version=5.3.1-0-g81034ce1f1373e37dc865038e1bc8eeecf559ce8 opt_background_thread=true background_thread=true max_background_threads=4" > f
        print "JE_CONFIRM_CONF=PASS" > f; print "AMP_JE_je=1.300000 row=21540" > f; print "FRAG_SHAREL_je=0.100000" > f
        print "DIRTY_SHAREL_je=0.150000" > f; print "REACH_PE_je=700.000 live=2220000" > f
      }
      if ((c == "mi3" || c == "mi2") && X["mipost", c] != "@absent") print "MI_POSTINIT_LINES_" c "=" X["mipost", c] > f
      close(f)
    }
    f = b ".matrix.txt"; printf "" > f
    if (X["dur", c] != "@absent") print "  duration:            " X["dur", c] "s" > f
    print "  csv cadence:         60s" > f; close(f)
    f = b ".soak.json"; print "{" > f
    if (X["tw", c] != "@absent") print "  \"totalWrites\": " X["tw", c] "," > f
    print "  \"writeErrors\": 0," > f; print "  \"crashes\": 0" > f; print "}" > f; close(f)
    f = b ".runner-console.log"; print "RESULT: instrument sound; harness exit code 0." > f
    if (X["steal", c] != "@absent") print "steal_pct=" X["steal", c] > f
    print "post_mortem_mem_reads=0" > f; print "mem_invariant_violations=0" > f
    if (X["runner_exit", c] != "@absent") print "RUNNER_EXIT=" X["runner_exit", c] > f
    close(f)
  }
  pf = "spec377-preflight-20260930T000000Z.log"
  f = W "/ev/" pf; print "CHECK alloc_conf=PASS (synthetic)" > f; print X["chain", "preflight"] > f; close(f)
  f = W "/ev/spec377-chain.log"; print "PROC_ROOT=/proc" > f; print "PREFLIGHT_LOG=" pf > f
  if (X["chain", "disk"] != "@absent") print "DISK_FREE_AT_START=" X["chain", "disk"] " KiB (need >= 40 GiB)" > f
  for (i = 1; i <= 5; i++) if (X["predrc", CELL[i]] != "@absent") print "PREDICATES_EXIT_" CELL[i] "=" X["predrc", CELL[i]] > f
  close(f)
  f = W "/smoke/spec377-chain.log"; print "SMOKE_ADMISSION=PASS failed=none" > f; close(f)
  f = W "/manifest.md"; printf "" > f
  while ((getline l < base) > 0) {
    k = l; sub(/=.*/, "", k)
    if (l ~ /^[A-Z0-9_]+=/ && (k in MAN)) { if (MAN[k] != "@absent") print k "=" MAN[k] > f; continue }
    print l > f
  }
  close(base); print "## APPEND-ONLY BELOW" > f; close(f)
  W = ""
}
BEGIN { split("s1 je mi3 mi2 s2", CELL, " "); W = "" }
/^WORLD / { flush(); W = substr($0, 7); reset(); next }
{
  p = index($0, "="); k = substr($0, 1, p - 1); v = substr($0, p + 1)
  if (k == "missing") { MISS[v] = 1; next }
  q = index(k, ":"); sc = substr(k, 1, q - 1); key = substr(k, q + 1)
  if (sc == "man") MAN[key] = v
  else if (sc == "ops") X["tw", key] = sprintf("%.0f", v * 5564160)
  else if (sc == "steal" || sc == "runner_exit" || sc == "tw" || sc == "dur" || sc == "predrc" || sc == "mipost" || sc == "chain") X[sc, key] = v
  else R[sc, key] = v
}
END { flush() }
