# P5 -- settlement discipline, over the ANSI-stripped console log.
function add(a, b) { return (a == "") ? b : a "; " b }
function fld(line, name,   needle, s, rest, sp) {
    needle = " " name "="
    s = index(line, needle)
    if (s == 0) return ""
    rest = substr(line, s + length(needle))
    sp = index(rest, " ")
    return (sp == 0) ? rest : substr(rest, 1, sp - 1)
}
function sod(t,   h, m, s2) {                       # seconds of day of YYYY-MM-DDTHH:MM:SS
    h = substr(t, 12, 2) + 0; m = substr(t, 15, 2) + 0; s2 = substr(t, 18, 2) + 0
    return h * 3600 + m * 60 + s2
}
function daynum(t,   y, mo, d, a, yy, mm) {         # Gregorian day index; integer only, no mktime
    y = substr(t, 1, 4) + 0; mo = substr(t, 6, 2) + 0; d = substr(t, 9, 2) + 0
    a = int((14 - mo) / 12); yy = y + 4800 - a; mm = mo + 12 * a - 3
    return d + int((153 * mm + 2) / 5) + 365 * yy + int(yy / 4) - int(yy / 100) + int(yy / 400) - 32045
}
BEGIN { ESC = sprintf("%c", 27); gmax = -1 }
{
    line = $0
    gsub(ESC "\\[[0-9;]*m", "", line)
    sub(/^\[server\] /, "", line)
    if (line ~ /^[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9]\./) last_ts = substr(line, 1, 19)
    if (index(line, "topgun_server::tombstone_frontier::removal:") > 0) {
        ep = fld(line, "epoch"); rr = fld(line, "refs_returned"); os = fld(line, "op_seq")
        if (ep !~ /^[0-9]+$/ || rr !~ /^[0-9]+$/ || os !~ /^[0-9]+$/) { unparsable = add(unparsable, "removal row at " substr(line, 1, 19)); next }
        nrem++
        if (rr + 0 == 0) { zn++; zero[zn] = "epoch=" ep " at=" substr(line, 1, 19); next }
        if (!(ep in rem)) { rem[ep] = 1; ord[++nord] = ep }
        rts[ep] = substr(line, 1, 19)
        if (!(ep in maxop) || os + 0 > maxop[ep] + 0) maxop[ep] = os + 0
        if (os + 0 > gmax) gmax = os + 0
        next
    }
    if (index(line, "topgun_server::tombstone_frontier::settlement:") > 0) {
        ep = fld(line, "epoch"); rc = fld(line, "restored_cancelled")
        nset++
        if (ep !~ /^[0-9]+$/) { unparsable = add(unparsable, "settlement row at " substr(line, 1, 19)); next }
        settled[ep]++
        if (rc == "") rcbad = add(rcbad, "epoch " ep ": restored_cancelled ABSENT")
        else if (rc !~ /^[0-9]+$/) rcbad = add(rcbad, "epoch " ep ": restored_cancelled non-integer '" rc "'")
        else if (rc + 0 != 0) rcbad = add(rcbad, "epoch " ep ": restored_cancelled=" rc)
        next
    }
}
END {
    msg = ""
    if (unparsable != "") msg = add(msg, "unparsable rows: " unparsable)
    if (nrem == 0) msg = add(msg, "no removal row in the log")
    un = 0; unlist = ""
    for (i = 1; i <= nord; i++) {
        ep = ord[i]
        if (!(ep in settled)) { un++; unlist = unlist (un > 1 ? "," : "") ep; only = ep }
    }
    if (rcbad != "") msg = add(msg, "restored_cancelled clause: " rcbad)
    carve = ""
    if (un == 1) {
        ci = (maxop[only] + 0 == gmax + 0)
        cii = 0; why = ""
        if (last_ts == "") why = "no server timestamp in the log"
        else {
            dd = daynum(last_ts) - daynum(rts[only])
            if (dd < 0 || dd > 1) why = "date difference above one day (dd=" dd ")"
            else {
                delta = sod(last_ts) - sod(rts[only]) + 86400 * dd
                if (delta >= 0 && delta <= 15) cii = 1
                else why = "removal at " rts[only] " is " delta "s before the last server line " last_ts ", not within 15s"
            }
        }
        carve = sprintf("carve-out: epoch %s in_flight=%s (epoch max op_seq %s vs global %s) terminal=%s%s",
                        only, (ci ? "yes" : "no"), maxop[only], gmax, (cii ? "yes" : "no"), (cii ? "" : " [" why "]"))
        if (!(ci && cii)) msg = add(msg, "unsettled epoch " only " fails the terminal-pass carve-out")
    } else if (un > 1) {
        msg = add(msg, "unsettled epochs " unlist " (the carve-out admits at most one)")
    }
    print (msg == "" ? "P5=TRUE" : "P5=FALSE reason=" msg)
    printf("P5-observed: removal_rows=%d settlement_rows=%d unsettled=%s\n", nrem, nset, (un ? unlist : "none"))
    if (carve != "") print "P5-" carve
    for (i = 1; i <= zn; i++) print "P5-zero-return: " zero[i]
    if (zn + 0 == 0) print "P5-zero-return: none observed"
}
