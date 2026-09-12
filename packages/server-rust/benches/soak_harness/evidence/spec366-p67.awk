# P6/P7 -- decided on the LAST persisted scrape lying outside every open prune
# window. Fail-closed: every branch that cannot READ a number prints FALSE with
# a named reason. `absent == absent` is never true.
function fld(line, name,   needle, s, rest, sp) {
    needle = " " name "="
    s = index(line, needle)
    if (s == 0) return ""
    rest = substr(line, s + length(needle))
    sp = index(rest, " ")
    return (sp == 0) ? rest : substr(rest, 1, sp - 1)
}
function fail(reason) { print "P6=FALSE reason=" reason; print "P7=FALSE reason=" reason; exit 0 }
BEGIN { ESC = sprintf("%c", 27) }
{
    line = $0
    gsub(ESC "\\[[0-9;]*m", "", line)
    sub(/^\[server\] /, "", line)
    if (line !~ /^[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9]\./) next
    stamp_full = line; sub(/ .*/, "", stamp_full)          # full-precision console stamp
    if (index(line, "topgun_server::tombstone_frontier::removal:") > 0) {
        ep = fld(line, "epoch"); rr = fld(line, "refs_returned")
        if (ep !~ /^[0-9]+$/ || rr !~ /^[0-9]+$/) { bad_rows++; next }
        if (rr + 0 == 0) { zero_rows++; next }            # a zero-return removal NEVER opens a window
        nw++; w_ep[nw] = ep; w_open_full[nw] = stamp_full; w_open19[nw] = substr(line, 1, 19)
        next
    }
    if (index(line, "topgun_server::tombstone_frontier::settlement:") > 0) {
        ep = fld(line, "epoch")
        if (ep !~ /^[0-9]+$/) { bad_rows++; next }
        ns++; s_ep[ns] = ep; s_full[ns] = stamp_full; s_19[ns] = substr(line, 1, 19)
        next
    }
}
END {
    if (bad_rows > 0) fail("unparsable_console_row(" bad_rows ")")
    # Close each window with the FIRST settlement of the same epoch strictly after
    # it. Console-vs-console only, so the full-precision stamps compare directly:
    # both sides carry microseconds in one fixed-width format.
    for (i = 1; i <= nw; i++) {
        w_close19[i] = ""                                  # "" == still OPEN
        best = ""
        for (j = 1; j <= ns; j++) {
            if (s_ep[j] != w_ep[i]) continue
            if (s_full[j] <= w_open_full[i]) continue
            if (best == "" || s_full[j] < best) { best = s_full[j]; w_close19[i] = s_19[j] }
        }
    }
    # The scrape stamps, lexicographic on the first 19 characters of the file name.
    cmd = "ls -1 " scrapes_dir " 2>/dev/null"
    nf = 0
    while ((cmd | getline fn) > 0) {
        if (fn !~ /^[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9]Z\.txt$/) continue
        nf++; files[nf] = fn; stamps[nf] = substr(fn, 1, 19)
    }
    close(cmd)
    if (nf == 0) fail("no_persisted_scrape_in " scrapes_dir)
    # LAST scrape outside EVERY window. Equal-second is INSIDE at both edges.
    ds = ""; ds_file = ""
    for (k = 1; k <= nf; k++) {
        inside = 0
        for (i = 1; i <= nw; i++) {
            if (stamps[k] < w_open19[i]) continue
            if (w_close19[i] == "" || stamps[k] <= w_close19[i]) { inside = 1; break }
        }
        if (inside) continue
        if (ds == "" || stamps[k] > ds) { ds = stamps[k]; ds_file = files[k] }
    }
    if (ds == "") { print "DECISION_SCRAPE=OPEN"; fail("decision_scrape_OPEN(no scrape outside every open window)") }
    print "DECISION_SCRAPE=" ds_file
    path = scrapes_dir "/" ds_file
    if ((getline probe < path) < 0) fail("unreadable_scrape " ds_file)
    close(path)
    nlines = 0
    while ((getline mline < path) > 0) {
        nlines++
        if (mline ~ /^[[:space:]]*#/) continue
        nm = mline; sub(/[[:space:]].*$/, "", nm)
        b = index(nm, "{"); if (b > 0) nm = substr(nm, 1, b - 1)
        rest = mline; sub(/^[^[:space:]]+[[:space:]]+/, "", rest); sub(/[[:space:]].*$/, "", rest)
        if (!(nm in seen)) { seen[nm] = 1; val[nm] = rest }
    }
    close(path)
    if (nlines == 0) fail("empty_scrape " ds_file)
    OBS = "topgun_or_prune_removed_refs_observed_total"
    CON = "topgun_or_prune_considered_total"
    CAN = "topgun_or_prune_restored_cancelled_total"
    # P6
    if (!(OBS in seen)) print "P6=FALSE reason=absent_series " OBS " in " ds_file
    else if (!(CON in seen)) print "P6=FALSE reason=absent_series " CON " in " ds_file
    else if (val[OBS] !~ /^[0-9]+$/) print "P6=FALSE reason=non_integer_sample " OBS "='" val[OBS] "'"
    else if (val[CON] !~ /^[0-9]+$/) print "P6=FALSE reason=non_integer_sample " CON "='" val[CON] "'"
    else if (val[OBS] + 0 != val[CON] + 0) printf("P6=FALSE reason=gap removed_refs_observed_total=%s considered_total=%s gap=%d\n", val[OBS], val[CON], val[OBS] - val[CON])
    else printf("P6=TRUE removed_refs_observed_total=%s considered_total=%s gap=0\n", val[OBS], val[CON])
    # P7
    if (!(CAN in seen)) print "P7=FALSE reason=absent_series " CAN " in " ds_file
    else if (val[CAN] !~ /^[0-9]+$/) print "P7=FALSE reason=non_integer_sample " CAN "='" val[CAN] "'"
    else if (val[CAN] + 0 != 0) print "P7=FALSE reason=restored_cancelled_total=" val[CAN]
    else print "P7=TRUE restored_cancelled_total=0"
    printf("windows=%d settlement_rows=%d zero_return_removal_rows=%d scrapes=%d\n", nw, ns, zero_rows + 0, nf)
}
