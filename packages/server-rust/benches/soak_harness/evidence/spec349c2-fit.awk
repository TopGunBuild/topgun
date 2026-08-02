# Post-hoc ordinary-least-squares fit over a plateau-run CSV.
#
# Every slope recorded for these runs must be re-derivable from the committed
# CSV by anyone, with no toolchain beyond awk -- a slope quoted in prose is a
# number nobody can check, and that is exactly how the previous generation of
# characterization figures evaporated.
#
# Usage:
#   awk -v col=rss_mb -v window=last_half -f spec349c2-fit.awk run.csv
#
#   col     one of the CSV's value columns (rss_mb | wal_mb | redb_mb |
#           disk_total_mb | tombstone_bytes). Required. Resolved by HEADER NAME,
#           so a CSV written before a column was added still fits every column
#           it does carry.
#
#           NOTE on units: the printed field is named `slope_mb_per_hour`
#           because every column it was built for is a megabyte column. For
#           `tombstone_bytes` the fit is arithmetically identical but the unit
#           is BYTES per hour -- read the name as "value units per hour" there.
#   window  last_half (default) | full
#             last_half = rows [floor(n/2) .. n-1] over the FULL row count n,
#             the same floor-biased split the harness's own last-half statistic
#             uses, so this statistic and the harness's mean the same thing.
#             full     = every row; this is the only window the harness also
#             computes for RSS/disk, so it is the cross-instrument check.
#
# The fit is of the value against elapsed time expressed in HOURS
# (x = elapsed_secs / 3600), so the slope comes out directly in MB/h -- the
# same convention the harness's own least-squares helper uses.
#
# Reported with the textbook OLS standard error of the slope coefficient
#   SE(b) = sqrt( SSE / ((m - 2) * Sxx) )
# where SSE = sum (y_i - yhat_i)^2, Sxx = sum (x_i - xbar)^2 and m is the
# window's sample count.
#
# CAVEAT, and it must travel with every number this prints: RSS and disk are
# cumulative, strongly autocorrelated series. Consecutive samples are not
# independent draws around the trend line, so this SE is OPTIMISTIC -- it
# understates the true uncertainty of the slope, often by a large factor. It
# therefore makes "the intervals do not overlap" EASY to satisfy, so SE
# separation alone must never carry a discrimination claim; the minimum
# effect-size floor does. A HAC (Newey-West) estimator is deliberately not
# implemented here: it would put an econometric estimator into a script whose
# whole value is that it stays auditable by inspection.
#
# Exit codes: 0 ok, 2 usage/data error (also printed to stderr).

function die(msg) {
    print "spec349c2-fit: " msg > "/dev/stderr"
    failed = 1
    exit 2
}

# Numeric guard. An empty or non-numeric cell is the blind-sampler failure
# these runs exist to catch, so it is a hard error rather than a skipped row.
function isnum(s) {
    return s ~ /^[+-]?([0-9]+\.?[0-9]*|\.[0-9]+)([eE][+-]?[0-9]+)?$/
}

function trim(s) {
    sub(/^[ \t\r]+/, "", s)
    sub(/[ \t\r]+$/, "", s)
    return s
}

BEGIN {
    FS = ","
    failed = 0
    if (col == "") die("-v col=<column> is required")
    if (window == "") window = "last_half"
    if (window != "last_half" && window != "full") die("-v window= must be last_half or full")
    n = 0
    xcol = -1
    ycol = -1
}

NR == 1 {
    for (i = 1; i <= NF; i++) {
        h = trim($i)
        if (h == "elapsed_secs") xcol = i
        if (h == col) ycol = i
    }
    if (xcol < 0) die("no elapsed_secs column in header")
    if (ycol < 0) die("no column named '" col "' in header")
    next
}

# Skip wholly blank lines (a trailing newline is not a data error); everything
# else must parse.
/^[ \t\r]*$/ { next }

{
    xs = trim($xcol)
    ys = trim($ycol)
    # An EMPTY value cell is a scrape that did not answer, and is dropped from
    # the fit rather than fatal -- a gauge column is allowed to have holes where
    # a du/ps column is not. It is counted so the caller can see how much of the
    # window was actually measured. A cell that is present but non-numeric is
    # still fatal: that is a malformed row, not a missing reading.
    if (ys == "") {
        skipped++
        next
    }
    if (!isnum(xs) || !isnum(ys)) {
        die("non-numeric cell at line " NR " (elapsed_secs='" xs "', " col "='" ys "')")
    }
    t[n] = xs + 0
    y[n] = ys + 0
    n++
}

END {
    # An `exit` from a rule still runs END; do not print a result over an error
    # that already fired.
    if (failed) exit 2
    if (n == 0) die("no data rows")

    start = (window == "last_half") ? int(n / 2) : 0
    m = n - start
    if (m < 2) die("window has " m " row(s); need at least 2")

    sx = 0; sy = 0
    for (i = start; i < n; i++) {
        x[i] = t[i] / 3600.0          # hours, so the slope is MB/h
        sx += x[i]
        sy += y[i]
    }
    xbar = sx / m
    ybar = sy / m

    sxx = 0; sxy = 0
    for (i = start; i < n; i++) {
        dx = x[i] - xbar
        sxx += dx * dx
        sxy += dx * (y[i] - ybar)
    }

    # Every sample at the same instant: the slope is not identified.
    if (sxx <= 0) die("zero variance in elapsed_secs over the window")

    b = sxy / sxx
    a = ybar - b * xbar

    sse = 0; sst = 0
    for (i = start; i < n; i++) {
        r = y[i] - (a + b * x[i])
        sse += r * r
        dy = y[i] - ybar
        sst += dy * dy
    }

    if (m > 2) {
        se_s = sprintf("%.6f", sqrt(sse / ((m - 2) * sxx)))
    } else {
        # m == 2 fits the two points exactly; the SE is undefined, not zero.
        se_s = "NA"
    }
    r2_s = (sst > 0) ? sprintf("%.6f", 1 - sse / sst) : "NA"

    # `rows_used` counts rows that CARRIED A VALUE -- it is what the window and
    # the fit are both computed over, and empty cells are absent from it rather
    # than folded in as zeros. Reported alongside `skipped_empty` so a reader can
    # see how much of the series was actually measured.
    printf "col=%s window=%s rows_used=%d n=%d skipped_empty=%d", col, window, n, m, skipped + 0
    printf " t_start_secs=%.1f t_end_secs=%.1f span_secs=%.1f", t[start], t[n-1], t[n-1] - t[start]
    printf " y_first=%.3f y_last=%.3f", y[start], y[n-1]
    printf " slope_mb_per_hour=%.6f se_mb_per_hour=%s", b, se_s
    printf " intercept_mb=%.6f r2=%s sxx_hours2=%.9f sse=%.9f\n", a, r2_s, sxx, sse
}
