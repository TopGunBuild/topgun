# P-S rows -- retained closed epochs never exceed the durable watermark lag plus the one open epoch.
BEGIN { FS = "," }
NR == 1 {
    if ($11 != "conj_snapshots_total" || $15 != "durable_watermark_lag" || $20 != "ret_epochs_claim_only" || $21 != "ret_epochs_durability_only" || $22 != "ret_epochs_both") { hdr = 1; exit 0 }
    next
}
/^[ \t\r]*$/ { next }
{
    rows++
    if (!started) {
        if ($11 ~ /^[0-9]+$/ && $11 + 0 >= 1) { started = 1; first = $1 }
        else { exempt++; next }
    }
    evaluable++
    if ($15 !~ /^[0-9]+$/ || $20 !~ /^[0-9]+$/ || $21 !~ /^[0-9]+$/ || $22 !~ /^[0-9]+$/) { nbad++; if (nbad <= 5) bl = bl " elapsed=" $1; next }
    s = $20 + $21 + $22
    ex = s - $15
    if (!seen || ex > maxex) { maxex = ex; seen = 1 }
    if (s > $15 + 1) { nv++; if (nv <= 5) vl = vl " elapsed=" $1 "(sum=" s ",lag=" $15 ")" }
}
END {
    if (hdr) { print "PS-rows=FALSE reason=header_mismatch"; exit 0 }
    if (rows + 0 == 0) { print "PS-rows=FALSE reason=no_data_rows"; exit 0 }
    if (!started) { print "PS-rows=FALSE reason=no_conjunct_snapshot rows=" rows; exit 0 }
    if (evaluable < 0.95 * rows) { print "PS-rows=FALSE reason=evaluable_below_95pct evaluable=" evaluable "/" rows " first_snapshot_elapsed=" first; exit 0 }
    if (nbad > 0) { print "PS-rows=FALSE reason=empty_or_non_integer_cell_after_first_snapshot rows=" nbad bl; exit 0 }
    if (nv > 0) { print "PS-rows=FALSE reason=bound_violated rows=" nv vl; exit 0 }
    print "PS-rows=TRUE rows=" rows " evaluable=" evaluable " exempt=" (exempt + 0) " first_snapshot_elapsed=" first " max(sum-lag)=" maxex
}
