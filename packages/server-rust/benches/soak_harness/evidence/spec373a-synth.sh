#!/usr/bin/env bash
# spec373a synthetic verdict cases (smoke). Frozen at M.
#
# usage: spec373a-synth.sh <SCRATCH_DIR>
# Writes S1..S7 under SCRATCH_DIR, each a minimal b1/b2/a1/a2 artifact set plus a
# manifest stub, runs spec373a-verdict.sh over each, and prints the flags. The
# inputs are pinned here and the expected flags are derived by hand below; the
# reader compares them, this script does not.
#
# Every cell: 900 s at cadence 60; rows t = 0..900; row 0 carries no probe; the
# probe line at row t has elapsed_s = t, seq = t/30; bytes_alloc = RATE * t, so
# BYTES_ALLOC_RATE = RATE exactly; alloc_live_bytes = LIVE on every row.
# E_FROZEN = 0.1334 unless stated (E/2 = 0.0667).
#
#   case  b1   b2   a1   a2   live(a)  extra                 expected
#   S1   1000 1010  860  870  1e8      -                     STOP=none VERDICT_BYTES=CONFIRMED
#        (s_b 0.00995; I = [0.8515, 0.8700] <= 0.9333)
#   S2   1000 1010  960  965  1e8      -                     STOP=none VERDICT_BYTES=NOT_MET
#        (I max 0.9650 > 0.9333)
#   S3   1000 1100  860  870  1e8      -                     STOP=none VERDICT_BYTES=INDETERMINATE
#        (s_b 0.0952 >= 0.0667)
#   S4   1000 1010 1300 1310  1e8      -                     STOP=R VERDICT_BYTES=WITHHELD
#        (min I 1.2871 > 1 + 0.00995)
#   S5   = S1, b2 post_mortem_rows=2                         STOP=V VERDICT_BYTES=WITHHELD
#   S6   = S1, manifest SHARES_STOP=S                        STOP=S VERDICT_BYTES=WITHHELD
#   S7   = S1, a1/a2 live 1.3e8                              STOP=R VERDICT_BYTES=WITHHELD
#        (live min I 1.3 > 1 + 0)
set -uo pipefail
export LC_ALL=C
OUT="${1:-}"
[ -n "$OUT" ] || { echo "usage: spec373a-synth.sh <SCRATCH_DIR>" >&2; exit 2; }
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
mkdir -p "$OUT"; OUT="$(cd "$OUT" && pwd -P)"
[ "$OUT" != "$SCRIPT_DIR" ] || { echo "FATAL: synthetic cases must not write into the evidence dir" >&2; exit 2; }

cell() {   # $1 dir, $2 cell, $3 rate, $4 live, $5 post_mortem_rows
  local d="$1" c="$2" rate="$3" live="$4" pm="$5" b="$1/spec373a-$2"
  {
    echo "elapsed_secs,phys_footprint_mb,tombstone_bytes,alloc_live_bytes,alloc_live_mb,alloc_probe_elapsed_s,alloc_probe_seq,bytes_alloc"
    awk -v rate="$rate" -v live="$live" 'BEGIN {
      print "0,1.0,,,,,,"
      for (t = 60; t <= 900; t += 60) printf "%d,1.0,100,%d,%.3f,%d,%d,%d\n", t, live, live / 1048576, t, t / 30, rate * t
    }'
  } > "$b.csv"
  printf '  duration:            900s\n  csv cadence:         60s\n' > "$b.matrix.txt"
  printf 'post_mortem_rows=%s\nRUNNER_EXIT=0\n' "$pm" > "$b.runner-console.log"
}
case_dir() {   # $1 name, $2 shares_stop, then b1 b2 a1 a2 rates, a-live, b2 pm
  local d="$OUT/$1"
  rm -rf "$d"; mkdir -p "$d"
  printf 'SHARES_STOP=%s\nE_FROZEN=0.1334\n' "$2" > "$d/manifest.md"
  cell "$d" b1 "$3" 100000000 0
  cell "$d" b2 "$4" 100000000 "$8"
  cell "$d" a1 "$5" "$7" 0
  cell "$d" a2 "$6" "$7" 0
  echo "--- synthetic $1"
  bash "$SCRIPT_DIR/spec373a-verdict.sh" "$d" "$d/manifest.md" | sed -n '/^== flags ==/,$p'
}
case_dir S1 none 1000 1010  860  870 100000000 0
case_dir S2 none 1000 1010  960  965 100000000 0
case_dir S3 none 1000 1100  860  870 100000000 0
case_dir S4 none 1000 1010 1300 1310 100000000 0
case_dir S5 none 1000 1010  860  870 100000000 2
case_dir S6 S    1000 1010  860  870 100000000 0
case_dir S7 none 1000 1010  860  870 130000000 0
