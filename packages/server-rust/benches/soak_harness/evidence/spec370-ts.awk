# Theil-Sen slope over the last half of tombstone_bytes (B/h), RECORDED only.
BEGIN { FS = "," }
NR == 1 { for (i = 1; i <= NF; i++) if ($i == "tombstone_bytes") yc = i; if (!yc) { hdr = 1; exit 0 }; next }
/^[ \t\r]*$/ { next }
{ v = $yc; gsub(/[ \t\r]/, "", v); if (v !~ /^[0-9]+$/) next; t[n] = $1 + 0; y[n++] = v + 0 }
END {
    if (hdr || n < 4) { print "PR-theil_sen=NA reason=" (hdr ? "no_column" : "too_few_rows"); exit 0 }
    h = int(n / 2); m = 0
    for (i = h; i < n; i++) for (j = i + 1; j < n; j++) if (t[j] > t[i]) s[m++] = (y[j] - y[i]) / (t[j] - t[i]) * 3600
    for (k = int(m / 2) - 1; k >= 0; k--) sift(k, m)
    for (e = m - 1; e > 0; e--) { x = s[0]; s[0] = s[e]; s[e] = x; sift(0, e) }
    med = (m % 2) ? s[int(m / 2)] : (s[m / 2 - 1] + s[m / 2]) / 2
    printf("PR-theil_sen last_half_rows=%d pairs=%d slope_bytes_per_hour=%.3f recorded_not_gated\n", n - h, m, med)
}
function sift(r, end,   c, x) { while (2 * r + 1 < end) { c = 2 * r + 1; if (c + 1 < end && s[c + 1] > s[c]) c++; if (s[r] >= s[c]) return; x = s[r]; s[r] = s[c]; s[c] = x; r = c } }
