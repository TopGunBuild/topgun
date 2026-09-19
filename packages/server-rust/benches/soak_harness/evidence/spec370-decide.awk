# Deciding flags -- computed only after every STOP predicate line is already in the file.
BEGIN { ns = split("PM-reconciled PM-split P5 P6 P7 PS-rows PS-verdict PV PR-rows PR-crashes PR-class", stopk, " ") }
{
    for (i = 1; i <= ns; i++) { k = stopk[i]
        if (index($0, k "=") == 1) { seen[k]++; if (index($0, k "=TRUE") == 1) tru[k]++; else fal[k]++ } }
    if ($0 ~ /^PC=/) { pcs++; pc = $0 }
    if ($0 ~ /^PL=/) { pls++; pl = $0 }
    if ($0 ~ /^PH=/) { phs++; ph = $0 }
    if ($0 ~ /^PK=/) { pks++; pk = $0 }
    if ($0 ~ /^PK-premise=/) { prem = $0; sub(/^PK-premise=/, "", prem); sub(/ .*/, "", prem) }
    if ($0 ~ /^PA-wal_watermark_alarm_lines=[0-9]+ /) pa++
    if ($0 ~ /^PH-harness ceiling_ok=/) { hc = $0; sub(/^PH-harness ceiling_ok=/, "", hc); sub(/ .*/, "", hc) }
    if ($0 ~ /^PL half_start=/) { d = $0; sub(/.* deviation_pct=/, "", d); sub(/ .*/, "", d); devpct = d
                                  r = $0; sub(/.* direction=/, "", r); sub(/ .*/, "", r); dir = r }
}
END {
    bad = ""
    for (i = 1; i <= ns; i++) { k = stopk[i]; if (seen[k] != 1 || fal[k] > 0 || tru[k] != 1) bad = bad "," k }
    if (pcs != 1 || pls != 1 || phs != 1 || pks != 1) bad = bad ",deciding_lines"
    if (pc ~ /INDETERMINATE/ || pl ~ /INDETERMINATE/ || ph ~ /INDETERMINATE/ || pk ~ /INDETERMINATE/) bad = bad ",indeterminate"
    if (prem == "") prem = "UNKNOWN"
    if (bad != "") {
        print "DECISION stops=FAILED(" substr(bad, 2) ")"
        print "PREMISE=" prem
        print "PLATEAU=STOP reason=" substr(bad, 2)
        print "REPLICATE=NOT_AUTHORIZED"
        print "TG-OR-005=STAYS_OPEN"
        exit 0
    }
    print "DECISION stops=OK"
    print "PREMISE=" prem
    PC = (pc == "PC=TRUE"); PL = (pl == "PL=TRUE"); PH = (ph == "PH=TRUE")
    if (PC && PL && PH) {
        print "PLATEAU=TRUE"; print "READING=BOUNDED_STEADY"; print "REPLICATE=NOT_NEEDED"
        if (prem == "TRUE" && pa == 1) print "TG-OR-005=FLIP_TO_EVIDENCED"
        else print "TG-OR-005=STAYS_OPEN reason=premise_unverified"
        exit 0
    }
    if (!PC) { print "PLATEAU=FALSE reason=" ((prem == "FALSE") ? "ceiling_premise_violated" : "ceiling"); print "READING=NOT_BOUNDED"; print "REPLICATE=NOT_AUTHORIZED"; print "TG-OR-005=STAYS_OPEN"; exit 0 }
    if (hc == "FALSE") { print "PLATEAU=FALSE reason=ceiling_5s"; print "READING=NOT_BOUNDED"; print "REPLICATE=NOT_AUTHORIZED"; print "TG-OR-005=STAYS_OPEN"; exit 0 }
    if (!PH) { print "PLATEAU=FALSE reason=instrument_disagreement"; print "READING=INSTRUMENT_DISAGREEMENT"; print "REPLICATE=NOT_AUTHORIZED"; print "TG-OR-005=STAYS_OPEN"; exit 0 }
    if (dir == "down") {
        print "PLATEAU=FALSE reason=level_decaying deviation_pct=" devpct; print "READING=BOUNDED_DECAYING"
        print (!replicate ? "REPLICATE=AUTHORIZED" : "REPLICATE=NOT_AUTHORIZED"); print "TG-OR-005=STAYS_OPEN"; exit 0
    }
    near = (devpct != "NA" && devpct + 0 <= 13.0)
    print "PLATEAU=FALSE reason=level_up deviation_pct=" devpct; print "READING=BOUNDED_NOT_STEADY"
    print ((near && !replicate) ? "REPLICATE=AUTHORIZED" : "REPLICATE=NOT_AUTHORIZED"); print "TG-OR-005=STAYS_OPEN"
}
