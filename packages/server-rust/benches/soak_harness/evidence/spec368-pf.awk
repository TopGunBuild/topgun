# P-F -- footprint reconstruction (two identities) and peak/last, recorded only.
BEGIN { FS = "," }
NR == 1 {
    if ($1 != "elapsed_secs" || $2 != "rss_mb" || $7 != "phys_footprint_mb" || $8 != "phys_footprint_peak_mb" || $9 != "reclaimable_mb" || $41 != "clean_mb") { hdr = 1; exit 0 }
    next
}
/^[ \t\r]*$/ { next }
{
    if ($7 != "") { if (!pf_seen || $7 + 0 > pf_peak) { pf_peak = $7 + 0; pf_seen = 1 }; pf_last = $7 }
    if ($9 != "") { if (!rc_seen || $9 + 0 > rc_peak) { rc_peak = $9 + 0; rc_seen = 1 }; rc_last = $9 }
    if ($8 != "") pfp_last = $8
    if ($1 + 0 <= 0) next
    total++
    if ($2 == "" || $7 == "" || $9 == "" || $2 + 0 == 0) next
    d2 = $2 - ($7 + $9); if (d2 < 0) d2 = -d2
    if (d2 / $2 <= 0.02) w2++
    if ($41 != "") { d3 = $2 - ($7 + $41 + $9); if (d3 < 0) d3 = -d3; if (d3 / $2 <= 0.02) w3++ }
}
END {
    if (hdr) { print "PF=INSTRUMENT reason=header_mismatch"; exit 0 }
    printf("PF-recon2 rows_within_2pct=%d/%d %s\n", w2 + 0, total + 0, (total > 0 && (w2 + 0) >= 0.95 * total) ? "MET" : "NOT_MET")
    printf("PF-recon3 rows_within_2pct=%d/%d %s\n", w3 + 0, total + 0, (total > 0 && (w3 + 0) >= 0.95 * total) ? "MET" : "NOT_MET")
    printf("PF-phys_footprint_mb peak=%s last=%s peak_eq_last=%s\n", pf_seen ? sprintf("%.3f", pf_peak) : "NA", pf_last == "" ? "NA" : pf_last, pf_seen ? ((pf_peak == pf_last + 0) ? "TRUE" : "FALSE") : "NA")
    printf("PF-reclaimable_mb peak=%s last=%s peak_eq_last=%s\n", rc_seen ? sprintf("%.3f", rc_peak) : "NA", rc_last == "" ? "NA" : rc_last, rc_seen ? ((rc_peak == rc_last + 0) ? "TRUE" : "FALSE") : "NA")
    printf("PF-phys_footprint_peak_mb last=%s\n", pfp_last == "" ? "NA" : pfp_last)
}
