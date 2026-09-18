# P-H -- the harness (5 s) verdicts agree with the cell's at the same K and b_max.
BEGIN { FS = "\t"; split("ceilingBreached levelBreached ceilingDisposition levelDisposition ceilingEpochs epochWidth tagBytesMax stampsInWindowMax ceilingBytes peakBytes", nm, " ") }
NR == 1 {
    for (i = 1; i <= 10; i++) if ($i == "" || $i == "null") { printf("PH=INDETERMINATE reason=missing_field(%s)\n", nm[i]); done = 1; exit 0 }
    printf("PH-harness ceiling_ok=%s level_ok=%s ceiling_disposition=%s level_disposition=%s ceiling_epochs=%s epoch_width=%s tag_bytes_max=%s stamps_in_window_max=%s ceiling_bytes=%s peak_bytes=%s\n", ($1 == "false") ? "TRUE" : "FALSE", ($2 == "false") ? "TRUE" : "FALSE", $3, $4, $5, $6, $7, $8, $9, $10)
    printf("PH-cell ceiling_ok=%s level_ok=%s ceiling_epochs=%s tag_bytes_max=%s ceiling_bytes=%s\n", pc, pl, pk, pb, pc_ceiling)
    why = ""
    if ($5 + 0 != pk + 0) why = why " ceiling_epochs_mismatch"
    if ($7 + 0 != pb + 0) why = why " tag_bytes_mismatch"
    if ($9 + 0 != $5 * $6 * $7) why = why " ceiling_arithmetic"
    if ($9 + 0 != pc_ceiling + 0) why = why " ceiling_mismatch"
    if ($3 != "EVALUATED") why = why " ceiling_not_evaluated"
    if ($4 != "EVALUATED") why = why " level_not_evaluated"
    if ((($1 == "false") ? "TRUE" : "FALSE") != pc) why = why " ceiling_verdict_disagrees"
    if ((($2 == "false") ? "TRUE" : "FALSE") != pl) why = why " level_verdict_disagrees"
    print (why == "") ? "PH=TRUE" : "PH=FALSE reason=" substr(why, 2)
    done = 1
}
END { if (!done) print "PH=INDETERMINATE reason=no_input" }
