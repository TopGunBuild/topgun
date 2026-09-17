# A7 -- per 30-minute window: max pass latency / min interval between consecutive removals.
function fld(line, name,   needle, s, rest, sp) {
    needle = " " name "="
    s = index(line, needle)
    if (s == 0) return ""
    rest = substr(line, s + length(needle))
    sp = index(rest, " ")
    return (sp == 0) ? rest : substr(rest, 1, sp - 1)
}
function daynum(t,   y, mo, d, a, yy, mm) {
    y = substr(t, 1, 4) + 0; mo = substr(t, 6, 2) + 0; d = substr(t, 9, 2) + 0
    a = int((14 - mo) / 12); yy = y + 4800 - a; mm = mo + 12 * a - 3
    return d + int((153 * mm + 2) / 5) + 365 * yy + int(yy / 4) - int(yy / 100) + int(yy / 400) - 32045
}
function secs(tok,   z, frac) {
    frac = 0
    if (substr(tok, 20, 1) == ".") { z = index(tok, "Z"); if (z > 21) frac = ("0" substr(tok, 20, z - 20)) + 0 }
    return (daynum(tok) - d0) * 86400 + substr(tok, 12, 2) * 3600 + substr(tok, 15, 2) * 60 + substr(tok, 18, 2) + frac
}
BEGIN {
    ESC = sprintf("%c", 27)
    if (t0 !~ /^[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9]$/) { badt0 = 1; exit 0 }
    d0 = daynum(t0); T0S = secs(t0)
}
{
    line = $0
    gsub(ESC "\\[[0-9;]*m", "", line)
    sub(/^\[server\] /, "", line)
    if (line !~ /^[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9]/) next
    tok = line; sub(/ .*/, "", tok)
    if (index(line, "topgun_server::tombstone_frontier::removal:") > 0) {
        ep = fld(line, "epoch"); rr = fld(line, "refs_returned")
        if (ep !~ /^[0-9]+$/ || rr !~ /^[0-9]+$/) { bad++; next }
        if (rr + 0 == 0) next
        nr++; r_ep[nr] = ep; r_t[nr] = secs(tok)
        next
    }
    if (index(line, "topgun_server::tombstone_frontier::settlement:") > 0) {
        ep = fld(line, "epoch")
        if (ep !~ /^[0-9]+$/) { bad++; next }
        ns++; s_ep[ns] = ep; s_t[ns] = secs(tok)
        next
    }
}
END {
    if (badt0) { print "A7=FALSE reason=bad_t0(" t0 ")"; exit 0 }
    if (bad > 0) { print "A7=FALSE reason=unparsable_console_row(" bad ")"; exit 0 }
    unsettled = ""
    for (i = 1; i <= nr; i++) {
        w = int((r_t[i] - T0S) / 1800) + 1; if (w < 1) w = 1; if (w > 8) w = 8
        rw[i] = w; lat_ok[i] = 0
        best = -1
        for (j = 1; j <= ns; j++) if (s_ep[j] == r_ep[i] && s_t[j] > r_t[i] && (best < 0 || s_t[j] < best)) best = s_t[j]
        if (best < 0) unsettled = unsettled (unsettled == "" ? "" : ",") r_ep[i]
        else { L = best - r_t[i]; lat[i] = L; lat_ok[i] = 1; nl[w]++; if (nl[w] == 1 || L > maxl[w]) maxl[w] = L }
        if (i > 1) { I = r_t[i] - r_t[i - 1]; ni[w]++; if (ni[w] == 1 || I < mini[w]) mini[w] = I }
    }
    ok = 1; why = ""
    print "window | span_s | latencies | max_latency_s | intervals | min_interval_s | ratio"
    for (w = 1; w <= 8; w++) {
        if (nl[w] + 0 == 0 || ni[w] + 0 == 0) { ratio = "NA"; ok = 0; why = why " empty_window_W" w }
        else if (mini[w] <= 0) { ratio = "NA"; ok = 0; why = why " nonpositive_interval_W" w }
        else { r = maxl[w] / mini[w]; ratio = sprintf("%.3f", r); if (r >= 1) { ok = 0; why = why " ratio_ge_1_W" w } }
        printf("W%d | %d-%d | %d | %s | %d | %s | %s\n", w, (w - 1) * 1800, w * 1800, nl[w] + 0,
               (nl[w] + 0 ? sprintf("%.3f", maxl[w]) : "NA"), ni[w] + 0, (ni[w] + 0 ? sprintf("%.3f", mini[w]) : "NA"), ratio)
    }
    print "A7-removals=" nr + 0 " settlements=" ns + 0 " unsettled=" (unsettled == "" ? "none" : unsettled)
    for (i = 1; i <= nr; i++) {
        nx_ok = (i < nr); if (nx_ok) nx = r_t[i + 1] - r_t[i]
        er = (lat_ok[i] && nx_ok && nx > 0) ? sprintf("%.3f", lat[i] / nx) : "NA"
        printf("A7-epoch epoch=%s window=%d latency_s=%s next_interval_s=%s ratio=%s\n", r_ep[i], rw[i],
               (lat_ok[i] ? sprintf("%.3f", lat[i]) : "NA"), (nx_ok ? sprintf("%.3f", nx) : "NA"), er)
    }
    print (ok ? "A7=TRUE" : "A7=FALSE reason=" substr(why, 2))
}
