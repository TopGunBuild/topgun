#!/usr/bin/env bash
#
# Live-census coverage of one soak cell: decides, from the harness console and
# soak.json, whether every moment the run was due to read the live census was
# read, and prints CENSUS_COVERAGE_<cell>=TRUE|FALSE. FALSE is a pre-registered
# STOP for any series that reads the census: no per-entry reading may be
# interpolated across moments the census did not see.
#
# usage: census-coverage.sh <console> <soak.json> <D> <I> <cell> <crashes>
#   console    the harness console (stdout and stderr of the run)
#   soak.json  the run's --json-output; its liveCensus section is read
#   D, I       the run's --duration and --live-census-interval, in seconds
#   cell       the label the printed keys carry
#   crashes    the crashes the cell injected; a moment is excused only by a
#              boot gap, so a cell without crashes may excuse none
#
# The moments due are the k >= 1 with k*I + 25 <= D (the last read attempt of a
# moment starts 20 s after it and may take 5 s), the same grid the harness uses.
# A moment is present when a LIVE_GAUGE line carries its ordinal; time is not
# looked at, so a reading stamped late still counts for its own moment and two
# readings of one moment are caught. LIVE_GAUGE lines past the grid are a
# healthy run's last tick before the stop and are neither due nor absent.
#
# Output, one line each, in this order:
#   CENSUS_EXPECTED_N_<cell>=<n>
#   CENSUS_ABSENT_N_<cell>=<n>
#   CENSUS_ABSENT_<cell>=<k*I,..|none>
#   CENSUS_EXCUSED_N_<cell>=<n>
#   CENSUS_RECONCILE_<cell>=TRUE | FALSE | n/a reason=<soak.json reason|none>
#   CENSUS_LOOP_END_<cell>=<reason|none|multiple>
#   CENSUS_COVERAGE_<cell>=TRUE | FALSE reason=<r1,r2,..>
# where the reasons, in this order, are duplicate_ordinal,
# json_console_mismatch, unexcused_absent, excused_over_crashes, reconcile,
# loop_end.
#
# Exit status: 0 = coverage TRUE; 1 = coverage FALSE; 2 = the input could not be
# read (only the CENSUS_COVERAGE line is printed, FALSE reason=input:<what>).

set -u
export LC_ALL=C

if [ "$#" -ne 6 ]; then
  echo "usage: census-coverage.sh <console> <soak.json> <D> <I> <cell> <crashes>" >&2
  exit 2
fi
CONSOLE=$1 JSON=$2 D=$3 I=$4 CELL=$5 CRASHES=$6

input_fail() {
  echo "CENSUS_COVERAGE_${CELL}=FALSE reason=input:$1"
  exit 2
}

for n in "$D" "$I" "$CRASHES"; do
  case $n in '' | *[!0-9]*) input_fail "not_a_number:$n" ;; esac
done
[ "$I" -gt 0 ] || input_fail "interval_zero"
[ -r "$CONSOLE" ] || input_fail "console_unreadable"
[ -r "$JSON" ] || input_fail "json_unreadable"

# One line of soak.json's liveCensus, fields joined by "|": expected, absent,
# excused, reconcile status, reconcile reason, loop end -- lists comma-joined,
# null as "none".
J=$(jq -r '.liveCensus // empty
  | [ (.expected | tostring),
      (.absentOrdinals | map(tostring) | join(",")),
      (.excusedOrdinals | map(tostring) | join(",")),
      (.reconcile.status // "none"),
      (.reconcile.reason // "none"),
      (.loopEndReason // "none") ]
  | join("|")' "$JSON" 2>/dev/null) || input_fail "json_unparseable"
[ -n "$J" ] || input_fail "json_no_liveCensus"

awk -v D="$D" -v I="$I" -v cell="$CELL" -v crashes="$CRASHES" -v j="$J" '
function ord(   i) {
  for (i = 2; i <= NF; i++) if ($i ~ /^ordinal=/) return substr($i, 9) + 0
  return -1
}
# Whether the comma list s holds exactly the n keys of set a.
function same(s, a, n,   m, l, k) {
  m = (s == "") ? 0 : split(s, l, ",")
  if (m != n) return 0
  for (k = 1; k <= m; k++) if (!((l[k] + 0) in a)) return 0
  return 1
}
$1 == "LIVE_GAUGE" {
  o = ord()
  if (seen[o]++) dup = 1
}
$1 == "LIVE_GAUGE_EXCUSED" {
  o = ord()
  if (!(o in excused)) nexc++
  excused[o] = 1
}
$1 == "LIVE_CENSUS_LOOP_END" {
  loops++
  loop_end = substr($2, 8)
}
END {
  split(j, f, "|")
  expected = (D >= 25) ? int((D - 25) / I) : 0

  absent_n = 0; unexcused = 0; moments = ""
  for (k = 1; k <= expected; k++) {
    if (k in seen) continue
    absent[k] = 1; absent_n++
    moments = moments (moments == "" ? "" : ",") k * I
    if (!(k in excused)) unexcused++
  }
  if (loops == 0) loop_end = "none"
  else if (loops > 1) loop_end = "multiple"

  mismatch = f[1] != expected || !same(f[2], absent, absent_n) \
    || !same(f[3], excused, nexc + 0) || f[6] != loop_end

  if (f[4] == "match") rec = "TRUE"
  else if (f[4] == "mismatch") rec = "FALSE"
  else rec = "n/a reason=" f[5]

  reasons = ""
  if (dup) reasons = reasons ",duplicate_ordinal"
  if (mismatch) reasons = reasons ",json_console_mismatch"
  if (unexcused) reasons = reasons ",unexcused_absent"
  if (nexc > crashes) reasons = reasons ",excused_over_crashes"
  if (rec != "TRUE") reasons = reasons ",reconcile"
  if (loop_end != "stop") reasons = reasons ",loop_end"

  print "CENSUS_EXPECTED_N_" cell "=" expected
  print "CENSUS_ABSENT_N_" cell "=" absent_n
  print "CENSUS_ABSENT_" cell "=" (moments == "" ? "none" : moments)
  print "CENSUS_EXCUSED_N_" cell "=" nexc + 0
  print "CENSUS_RECONCILE_" cell "=" rec
  print "CENSUS_LOOP_END_" cell "=" loop_end
  if (reasons == "") { print "CENSUS_COVERAGE_" cell "=TRUE"; exit 0 }
  print "CENSUS_COVERAGE_" cell "=FALSE reason=" substr(reasons, 2)
  exit 1
}' "$CONSOLE"
