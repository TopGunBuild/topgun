# P-C / P-L -- mechanism ceiling on the whole-run maximum, level stability on the last half.
BEGIN { FS = ","
    if (epochs !~ /^[0-9]+$/ || width !~ /^[0-9]+$/ || tagmax !~ /^[0-9]+$/ || epochs + 0 < 1 || width + 0 < 1 || tagmax + 0 < 1) { badin = 1; exit 0 }
}
NR == 1 { for (i = 1; i <= NF; i++) if ($i == "tombstone_bytes") yc = i; if (!yc) { hdr = 1; exit 0 }; next }
/^[ \t\r]*$/ { next }
{
    v = $yc; gsub(/[ \t\r]/, "", v)
    if (v == "") { skipped++; next }
    if (v !~ /^[0-9]+$/) { nonint++; next }
    t[n] = $1 + 0; y[n++] = v + 0
}
END {
    if (badin) { print "PC=INDETERMINATE reason=bad_inputs(epochs=" epochs ",width=" width ",tagmax=" tagmax ")"; print "PL=INDETERMINATE reason=bad_inputs"; exit 0 }
    if (hdr) { print "PC=INDETERMINATE reason=no_tombstone_bytes_column"; print "PL=INDETERMINATE reason=no_tombstone_bytes_column"; exit 0 }
    if (nonint > 0) { print "PC=INDETERMINATE reason=non_integer_cells(" nonint ")"; print "PL=INDETERMINATE reason=non_integer_cells(" nonint ")"; exit 0 }
    if (n < 8) { print "PC=INDETERMINATE reason=too_few_rows(" n + 0 ")"; print "PL=INDETERMINATE reason=too_few_rows(" n + 0 ")"; exit 0 }
    E = width * tagmax; C = epochs * E
    h = int(n / 2); q = int(3 * n / 4)
    mx = -1; for (i = 0; i < n; i++) if (y[i] > mx) { mx = y[i]; mxt = t[i] }
    lmx = -1; for (i = h; i < n; i++) { sh += y[i]; if (y[i] > lmx) lmx = y[i] }
    for (i = q; i < n; i++) sq += y[i]
    mh = sh / (n - h); mq = sq / (n - q)
    dev = mq - mh; if (dev < 0) dev = -dev
    tol = 0.10 * ((mh > E) ? mh : E)
    pc = (mx <= C); pl = (dev <= tol)
    printf("PC-inputs ceiling_epochs=%d epoch_width=%d tag_bytes_max=%d epoch_bytes=%d ceiling_bytes=%d%s\n", epochs, width, tagmax, E, C, (width + 0 == 1000) ? "" : " width_not_production")
    printf("PC-max n=%d skipped_empty=%d run_max=%d run_max_elapsed=%s last_half_max=%d ceiling=%d run_max_over_ceiling=%.3f <=ceiling=%s\n", n, skipped + 0, mx, mxt, lmx, C, mx / C, pc ? "TRUE" : "FALSE")
    printf("PC-mean last_half_mean=%.3f over_ceiling=%.3f recorded_implied_by_PC-max\n", mh, mh / C)
    dir = (mq > mh) ? "up" : ((mq < mh) ? "down" : "none")
    printf("PL half_start=%d quarter_start=%d last_half_mean=%.3f last_quarter_mean=%.3f deviation_bytes=%.3f tolerance_bytes=%.3f deviation_pct=%s direction=%s <=tolerance=%s\n", h, q, mh, mq, dev, tol, (mh > 0) ? sprintf("%.3f", 100 * dev / mh) : "NA", dir, pl ? "TRUE" : "FALSE")
    if (width + 0 != 1000) { print "PC=FALSE reason=width_not_production(" width ")"; print (pl ? "PL=TRUE" : "PL=FALSE reason=level_deviation_above_tolerance"); exit 0 }
    print (pc ? "PC=TRUE" : "PC=FALSE reason=run_max_above_ceiling")
    print (pl ? "PL=TRUE" : "PL=FALSE reason=level_deviation_above_tolerance")
}
