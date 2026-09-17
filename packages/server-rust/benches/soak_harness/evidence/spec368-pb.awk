# P-B -- decay / bound / level decision over tombstone_bytes.
function isslope(s) { return s ~ /^[+-]?[0-9]+(\.[0-9]+)?$/ }
BEGIN { FS = "," }
NR == 1 { for (i = 1; i <= NF; i++) if ($i == "tombstone_bytes") yc = i; if (!yc) { hdr = 1; exit 0 }; next }
/^[ \t\r]*$/ { next }
{
    v = $yc; gsub(/[ \t\r]/, "", v)
    if (v == "") { skipped++; next }
    if (v !~ /^[0-9]+$/) { nonint++; next }
    y[n++] = v + 0
}
END {
    if (hdr) { print "PB=INDETERMINATE reason=no_tombstone_bytes_column"; exit 0 }
    if (nonint > 0) { print "PB=INDETERMINATE reason=non_integer_cells(" nonint ")"; exit 0 }
    if (!isslope(s1) || !isslope(s8) || !isslope(slh)) { print "PB=INDETERMINATE reason=missing_slope(s1=" s1 ",s8=" s8 ",slh=" slh ")"; exit 0 }
    if (n < 4) { print "PB=INDETERMINATE reason=too_few_rows(" n + 0 ")"; exit 0 }
    h = int(n / 2); q = int(3 * n / 4)
    for (i = h; i < n; i++) sh += y[i]
    for (i = q; i < n; i++) sq += y[i]
    mh = sh / (n - h); mq = sq / (n - q)
    c1 = (s8 + 0 < s1 + 0); c2 = (slh + 0 <= 512)
    if (mh <= 0) { c3 = 0; dpct = "NA"; near = 0; miss = 0; c3why = " reason=nonpositive_last_half_mean" }
    else { dv = (mq - mh) / mh; if (dv < 0) dv = -dv; c3 = (dv <= 0.10); dpct = sprintf("%.3f", dv * 100); near = (dv * 100 >= 7.0 && dv * 100 <= 13.0); miss = (!c3 && dv * 100 <= 13.0); c3why = "" }
    verdict = (c2 && c3) ? "PLATEAU" : (c1 ? "DECAYING_NOT_BOUND" : "NOT_MET")
    printf("PB-C1 slope_W8=%s slope_W1=%s W8<W1=%s recorded_decay_observation\n", s8, s1, c1 ? "TRUE" : "FALSE")
    printf("PB-C2 last_half_slope=%s <=512=%s\n", slh, c2 ? "TRUE" : "FALSE")
    printf("PB-C3 n=%d skipped_empty=%d half_start=%d quarter_start=%d last_half_mean=%.3f last_quarter_mean=%.3f deviation_pct=%s <=10=%s%s\n", n, skipped + 0, h, q, mh, mq, dpct, c3 ? "TRUE" : "FALSE", c3why)
    printf("PB-ratio W8/W1=%s\n", (s1 + 0 != 0) ? sprintf("%.2f", (s8 + 0) / (s1 + 0)) : "NA")
    printf("PB-near_threshold=%s\n", near ? "YES" : "NO")
    printf("PB-level_near_miss=%s\n", miss ? "YES" : "NO")
    printf("PB=%s\n", verdict)
    printf("REPLICATE=%s\n", (verdict == "DECAYING_NOT_BOUND" || (verdict == "NOT_MET" && miss)) ? "AUTHORIZED" : "NOT_AUTHORIZED")
}
