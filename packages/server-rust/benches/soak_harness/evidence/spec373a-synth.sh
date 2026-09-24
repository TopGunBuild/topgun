#!/usr/bin/env bash
# spec373a synthetic verdict cases (smoke). Frozen at M' (supersedes M).
#
# usage: spec373a-synth.sh <SCRATCH_DIR>
# Writes S1..S12 under SCRATCH_DIR, each a minimal b1/b2/a1/a2 artifact set plus a
# manifest stub, runs spec373a-verdict.sh over each, and prints its exit status
# and flags. The inputs are pinned here and the expected output is derived by
# hand below; the reader compares them, this script does not.
#
# Default cell: duration 900 at cadence 60; rows t = 0..900; row 0 carries no
# probe; the probe at row t has elapsed_s = t, seq = t/30; alloc_live_bytes = LIVE
# on every row; bytes_alloc(t) = R1*t up to the kink K, then R1*K + R2*(t - K)
# (no kink by default, so BYTES_ALLOC_RATE = R1 exactly). Manifest stub:
# SHARES_STOP=none, E_FROZEN=0.1334 (E/2 = 0.0667) unless stated.
#
#   case  b1   b2   a1   a2   live(a)  extra                 expected
#   S1   1000 1010  860  870  1e8      -                     rc=0 STOP=none VERDICT_BYTES=CONFIRMED I=[0.8515,0.8700]
#   S2   1000 1010  960  965  1e8      -                     rc=0 STOP=none VERDICT_BYTES=NOT_MET (I max 0.9650 > 0.9333)
#   S3   1000 1100  860  870  1e8      -                     rc=0 STOP=none VERDICT_BYTES=INDETERMINATE (s_b 0.0952 >= 0.0667)
#   S4   1000 1010 1300 1310  1e8      -                     rc=0 STOP=R WITHHELD (min I 1.2871 > 1.00995)
#   S5   = S1, b2 post_mortem_rows=2                         rc=0 STOP=V (b2:PM1) WITHHELD
#   S6   = S1, manifest SHARES_STOP=S                        rc=0 STOP=S WITHHELD
#   S7   = S1, a1/a2 live 1.3e8                              rc=0 STOP=R WITHHELD (live min I 1.3 > 1)
#   S8   EVEN n: every cell 840 s (rows 0..840, n = 14 probe points), kink K = 480,
#        b: R1 2000 then R2 1000; a: R1 2000 then R2 800.
#        h = int(13/2)+1 = 7 -> window 420..840:
#        b rate = (1320000 - 840000)/420 = 1142.857143; a rate = (1248000 - 840000)/420 = 971.428571
#                                                            rc=0 STOP=none VERDICT_BYTES=CONFIRMED I=[0.8500,0.8500]
#        (the rejected start int(n/2)+1 = 8 would give 1000 / 800 and I = 0.8000)
#   S9   = S1 with a1 = a2 = 0 (zero after-rates)            rc=0 STOP=V (a1:rate=0.000000 a2:rate=0.000000) WITHHELD
#   S10  = S1, b1 row t=480 bytes_alloc "x1"                 rc=0 STOP=V (b1:skipped=1) WITHHELD
#   S11  = S1, manifest without SHARES_STOP=                 rc=3, no flags
#   S12  = S1, manifest E_FROZEN=abc                         rc=3, no flags
set -uo pipefail
export LC_ALL=C
OUT="${1:-}"
[ -n "$OUT" ] || { echo "usage: spec373a-synth.sh <SCRATCH_DIR>" >&2; exit 2; }
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
mkdir -p "$OUT"; OUT="$(cd "$OUT" && pwd -P)"
[ "$OUT" != "$SCRIPT_DIR" ] || { echo "FATAL: synthetic cases must not write into the evidence dir" >&2; exit 2; }

cell() {   # $1 dir, $2 cell, $3 R1, $4 live, $5 post_mortem_rows, $6 duration, $7 kink, $8 R2
  local d="$1" c="$2" r1="$3" live="$4" pm="$5" dur="$6" kink="$7" r2="$8" b="$1/spec373a-$2"
  {
    echo "elapsed_secs,phys_footprint_mb,tombstone_bytes,alloc_live_bytes,alloc_live_mb,alloc_probe_elapsed_s,alloc_probe_seq,bytes_alloc"
    awk -v r1="$r1" -v r2="$r2" -v k="$kink" -v live="$live" -v dur="$dur" 'BEGIN {
      print "0,1.0,,,,,,"
      for (t = 60; t <= dur; t += 60) {
        ba = (t <= k) ? r1 * t : r1 * k + r2 * (t - k)
        printf "%d,1.0,100,%d,%.3f,%d,%d,%d\n", t, live, live / 1048576, t, t / 30, ba
      }
    }'
  } > "$b.csv"
  printf '  duration:            %ss\n  csv cadence:         60s\n' "$dur" > "$b.matrix.txt"
  printf 'post_mortem_rows=%s\nRUNNER_EXIT=0\n' "$pm" > "$b.runner-console.log"
}
run_case() {   # $1 name
  local rc
  echo "--- synthetic $1"
  bash "$SCRIPT_DIR/spec373a-verdict.sh" "$OUT/$1" "$OUT/$1/manifest.md" > "$OUT/$1/verdict.txt" 2>&1
  rc=$?
  echo "rc=${rc}"
  grep -E '^BYTES_ALLOC_RATE_|^SKIPPED_[ab][12]=[1-9]|^FATAL' "$OUT/$1/verdict.txt" | sed 's/^/  /'
  sed -n '/^== flags ==/,$p' "$OUT/$1/verdict.txt"
}
# $1 name, $2 manifest body, $3..$6 b1 b2 a1 a2 R1, $7 a-live, $8 b2 pm,
# $9 duration, $10 kink, $11 b R2, $12 a R2
case_dir() {
  local d="$OUT/$1"
  rm -rf "$d"; mkdir -p "$d"
  printf '%b' "$2" > "$d/manifest.md"
  cell "$d" b1 "$3" 100000000 0    "$9" "${10}" "${11}"
  cell "$d" b2 "$4" 100000000 "$8" "$9" "${10}" "${11}"
  cell "$d" a1 "$5" "$7"      0    "$9" "${10}" "${12}"
  cell "$d" a2 "$6" "$7"      0    "$9" "${10}" "${12}"
}
OK='SHARES_STOP=none\nE_FROZEN=0.1334\n'
NOK=1000000   # no kink inside any run
case_dir S1  "$OK"                               1000 1010  860  870 100000000 0 900 $NOK 0 0;    run_case S1
case_dir S2  "$OK"                               1000 1010  960  965 100000000 0 900 $NOK 0 0;    run_case S2
case_dir S3  "$OK"                               1000 1100  860  870 100000000 0 900 $NOK 0 0;    run_case S3
case_dir S4  "$OK"                               1000 1010 1300 1310 100000000 0 900 $NOK 0 0;    run_case S4
case_dir S5  "$OK"                               1000 1010  860  870 100000000 2 900 $NOK 0 0;    run_case S5
case_dir S6  'SHARES_STOP=S\nE_FROZEN=0.1334\n'  1000 1010  860  870 100000000 0 900 $NOK 0 0;    run_case S6
case_dir S7  "$OK"                               1000 1010  860  870 130000000 0 900 $NOK 0 0;    run_case S7
case_dir S8  "$OK"                               2000 2000 2000 2000 100000000 0 840 480 1000 800; run_case S8
case_dir S9  "$OK"                               1000 1010    0    0 100000000 0 900 $NOK 0 0;    run_case S9
case_dir S10 "$OK"                               1000 1010  860  870 100000000 0 900 $NOK 0 0
sed -i '' 's/^480,1.0,100,\([0-9]*\),\([0-9.]*\),480,16,[0-9]*$/480,1.0,100,\1,\2,480,16,x1/' "$OUT/S10/spec373a-b1.csv"
run_case S10
case_dir S11 'E_FROZEN=0.1334\n'                 1000 1010  860  870 100000000 0 900 $NOK 0 0;    run_case S11
case_dir S12 'SHARES_STOP=none\nE_FROZEN=abc\n'  1000 1010  860  870 100000000 0 900 $NOK 0 0;    run_case S12
