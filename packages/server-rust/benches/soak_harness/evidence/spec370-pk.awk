# P-K -- ceiling epochs derived from the fence-age window, and the recorded premise rows.
# -v stamps_window_max=<S from soak.json> -v width=<W>. Reads the CSV for the RECORDED server-side rows.
BEGIN { FS = ","
    if (stamps_window_max !~ /^[0-9]+$/ || width !~ /^[0-9]+$/ || width + 0 < 1) { badin = 1; exit 0 }
    H = int((stamps_window_max + width - 1) / width); K = 2 + H
}
NR == 1 { for (i = 1; i <= NF; i++) c[$i] = i
    if (!c["durable_watermark_lag"] || !c["ret_epochs_durability_only"] || !c["ret_epochs_both"] || !c["ret_epochs_claim_only"] || !c["ret_epochs_neither"] || !c["considered_total"] || !c["indexed_refs"]) { hdr = 1; exit 0 }
    next }
/^[ \t\r]*$/ { next }
{
    held = $c["ret_epochs_durability_only"] + $c["ret_epochs_both"] + $c["ret_epochs_claim_only"]
    if (held > hm) hm = held
    if ($c["ret_epochs_neither"] + 0 > nm) nm = $c["ret_epochs_neither"] + 0
    if ($c["durable_watermark_lag"] + 0 > lm) lm = $c["durable_watermark_lag"] + 0
    st = $c["considered_total"] + $c["indexed_refs"]; t = $1 + 0
    if (seen && t - pt <= 60 && st - ps > sm) sm = st - ps
    ps = st; pt = t; seen = 1
}
END {
    if (badin) { print "PK=INDETERMINATE reason=bad_inputs(stamps_window_max=" stamps_window_max ",width=" width ")"; exit 0 }
    printf("PK-derived stamps_window_max=%d epoch_width=%d held_max_derived=%d ceiling_epochs=%d\n", stamps_window_max, width, H, K)
    if (hdr) { print "PK-recorded reason=missing_columns"; print "PK-crosscheck=UNKNOWN reason=missing_columns"; print "PK-premise=UNKNOWN reason=missing_columns"; print "PK=" K; exit 0 }
    printf("PK-recorded held_max_observed=%d durable_watermark_lag_max=%d neither_max_observed=%d k_eff_expected_under_O2=4 recorded_not_gated\n", hm, lm, nm)
    printf("PK-crosscheck csv_stamps_per_row_max=%d stamps_window_max=%d <=%s recorded_not_gated\n", sm, stamps_window_max, (sm <= stamps_window_max + 0) ? "TRUE" : "FALSE")
    print ((nm == 0) ? "PK-a7 neither_max_observed=0 TRUE recorded_attributed_not_gated" : "PK-a7 neither_max_observed=" nm " FALSE recorded_attributed_not_gated")
    print ((hm <= H && lm <= H + 1) ? "PK-premise=TRUE held_max_observed=" hm " durable_watermark_lag_max=" lm " held_max_derived=" H : "PK-premise=FALSE held_max_observed=" hm " durable_watermark_lag_max=" lm " held_max_derived=" H)
    print "PK=" K
}
