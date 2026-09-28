#!/usr/bin/env bash
# spec373b synthetic verdict cases. Frozen at M.
#
# usage: spec373b-synth.sh <SCRATCH_DIR>
# Writes S1..S18 under SCRATCH_DIR, each a minimal b1/b2/a1/a2 artifact set plus a
# manifest stub, runs spec373b-verdict.sh over each with SPEC373B_SYNTHETIC=1
# (ORDER is skipped only because neither the cases nor their manifests are in
# the evidence dir), and prints its exit status and flags. The inputs are
# pinned here and the expected output is derived by hand below; the reader
# compares them, this script does not.
#
# Default cell: duration 900 at cadence 60; rows t = 0..900; row 0 carries no
# probe; the probe at row t has elapsed_s = t, seq = t/30; alloc_live_bytes = LIVE
# on every row; bytes_alloc(t) = R1*t up to the kink K, then R1*K + R2*(t - K)
# (no kink by default, so BYTES_ALLOC_RATE = R1 exactly); soak.json
# totalWrites = 150000 (BYTES_PER_WRITE = R1 / (150000/900) = R1 * 0.006).
# Manifest stub: SHARES_STOP=none, E_FROZEN=0.1334 unless stated, so
# E/2 = 0.0667, the INDETERMINATE bar is min(E/2, 0.05) = 0.05, CONFIRMED needs
# max I <= 0.9333, EXCESS needs 1 - min I > 0.2001.
#
#   case  b1   b2   a1   a2   live(a)  extra                 expected
#   S1   1000 1010  860  870  1e8      -                     rc=0 STOP=none CONFIRMED I=[0.8515,0.8700] S_B=0.009950 LIVE_SIGN=flat
#                                                            EXCESS=FALSE WRITE_PARITY=OK max_dev=0.0000
#                                                            BYTES_PER_WRITE=b1=6.0 b2=6.1 a1=5.2 a2=5.2 a/b=[0.8515,0.8700]
#   S2   1000 1010  960  965  1e8      -                     rc=0 STOP=none NOT_MET (max I 0.9650 > 0.9333)
#   S3   1000 1100  860  870  1e8      -                     rc=0 STOP=none INDETERMINATE (s_b 0.0952 >= 0.05)
#                                                            EXCESS=TRUE (1 - min I = 1 - 860/1100 = 0.2182 > 0.2001)
#   S4   1000 1010 1300 1310  1e8      -                     rc=0 STOP=R WITHHELD (min I 1.2871 > 1.00995)
#   S5   = S1, b2 post_mortem_rows=2                         rc=0 STOP=V (b2:PM1) WITHHELD
#   S6   = S1, manifest SHARES_STOP=S                        rc=0 STOP=S WITHHELD
#   S7   = S1, a1/a2 live 1.3e8 (LIVE_SIGN up)               rc=0 STOP=R WITHHELD LIVE_I=[1.3000,1.3000] LIVE_SIGN=up
#   S8   EVEN n: every cell 840 s (rows 0..840, n = 14 probe points), kink K = 480,
#        b: R1 2000 then R2 1000; a: R1 2000 then R2 800.
#        h = int(13/2)+1 = 7 -> window 420..840:
#        b rate = (1320000 - 840000)/420 = 1142.857143; a rate = (1248000 - 840000)/420 = 971.428571
#                                                            rc=0 STOP=none CONFIRMED I=[0.8500,0.8500]
#        (the rejected start int(n/2)+1 = 8 would give 1000 / 800 and I = 0.8000)
#   S9   = S1 with a1 = a2 = 0 (zero after-rates)            rc=0 STOP=V (a1:rate=0.000000 a2:rate=0.000000) WITHHELD, I=n/a
#   S10  = S1, b1 row t=480 bytes_alloc "x1"                 rc=0 STOP=V (b1:skipped=1) WITHHELD
#   S11  = S1, manifest without SHARES_STOP=                 rc=3, no flags
#   S12  = S1, manifest E_FROZEN=abc                         rc=3, no flags
#   S13  0.05 CAP: E_FROZEN=0.3000 (E/2 = 0.15), b 1000/1060 (s_b = 60/1030 = 0.0583),
#        a 600/610: s_b < E/2 but >= 0.05                    rc=0 STOP=none INDETERMINATE
#        (without the cap: max I 0.6100 <= 0.85 -> CONFIRMED)
#   S14  EXCESS: b 1000/1010, a 700/705                      rc=0 STOP=none CONFIRMED I=[0.6931,0.7050] EXCESS=TRUE
#        (1 - min I = 0.3069 > 1.5 E = 0.2001)
#   S15  WRITE_PARITY: = S1, a2 totalWrites 165000 (mean 153750; a2 dev 0.0732, others 0.0244)
#                                                            rc=0 STOP=V (a2:write_parity=0.0732) WITHHELD
#                                                            WRITE_PARITY=FAIL max_dev=0.0732
#   S16  LIVE_SIGN down: = S1, a1/a2 live 0.8e8               rc=0 STOP=none CONFIRMED LIVE_I=[0.8000,0.8000] LIVE_SIGN=down
#   S17  LIVE_SIGN flat with spread: = S1, b2 live 1.02e8, a live 1.01e8
#        (LIVE_I = [0.9902,1.0100], s_b_live = 0.0198)       rc=0 STOP=none CONFIRMED LIVE_SIGN=flat
#   S18  = S1, manifest without E_FROZEN=                    rc=3, no flags
#   S19  = S1 WITHOUT SPEC373B_SYNTHETIC (ORDER is required; no manifest commit)
#                                                            rc=3, ORDER=FAIL (order.sh does not hash as M '' lists it), no flags
set -uo pipefail
export LC_ALL=C
OUT="${1:-}"
[ -n "$OUT" ] || { echo "usage: spec373b-synth.sh <SCRATCH_DIR>" >&2; exit 2; }
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
mkdir -p "$OUT"; OUT="$(cd "$OUT" && pwd -P)"
case "$OUT/" in "$SCRIPT_DIR"/*) echo "FATAL: synthetic cases must not write into (or under) the evidence dir" >&2; exit 2 ;; esac

cell() {   # $1 dir, $2 cell, $3 R1, $4 live, $5 post_mortem_rows, $6 duration, $7 kink, $8 R2, $9 totalWrites
  local d="$1" c="$2" r1="$3" live="$4" pm="$5" dur="$6" kink="$7" r2="$8" tw="$9" b="$1/spec373b-$2"
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
  printf '{\n  "totalWrites": %s,\n  "durationSecsTarget": %s\n}\n' "$tw" "$dur" > "$b.soak.json"
}
run_case() {   # $1 name, $2 SPEC373B_SYNTHETIC (default 1)
  local rc
  echo "--- synthetic $1"
  SPEC373B_SYNTHETIC="${2:-1}" SPEC373B_MANIFEST_COMMIT= bash "$SCRIPT_DIR/spec373b-verdict.sh" "$OUT/$1" "$OUT/$1/manifest.md" > "$OUT/$1/verdict.txt" 2>&1
  rc=$?
  echo "rc=${rc}"
  grep -E '^ORDER=|^BYTES_ALLOC_RATE_|^SKIPPED_[ab][12]=[1-9]|^FATAL' "$OUT/$1/verdict.txt" | sed 's/^/  /'
  sed -n '/^== flags ==/,$p' "$OUT/$1/verdict.txt"
}
# $1 name, $2 manifest body, $3..$6 b1 b2 a1 a2 R1, $7 a-live, $8 b2 pm,
# $9 duration, $10 kink, $11 b R2, $12 a R2, $13 b2-live, $14 a2 totalWrites
case_dir() {
  local d="$OUT/$1"
  rm -rf "$d"; mkdir -p "$d"
  printf '%b' "$2" > "$d/manifest.md"
  cell "$d" b1 "$3" 100000000 0    "$9" "${10}" "${11}" 150000
  cell "$d" b2 "$4" "${13}"   "$8" "$9" "${10}" "${11}" 150000
  cell "$d" a1 "$5" "$7"      0    "$9" "${10}" "${12}" 150000
  cell "$d" a2 "$6" "$7"      0    "$9" "${10}" "${12}" "${14}"
}
OK='SHARES_STOP=none\nE_FROZEN=0.1334\n'
NOK=1000000   # no kink inside any run
L=100000000
T=150000
case_dir S1  "$OK"                               1000 1010  860  870 $L        0 900 $NOK 0 0 $L $T;    run_case S1
case_dir S2  "$OK"                               1000 1010  960  965 $L        0 900 $NOK 0 0 $L $T;    run_case S2
case_dir S3  "$OK"                               1000 1100  860  870 $L        0 900 $NOK 0 0 $L $T;    run_case S3
case_dir S4  "$OK"                               1000 1010 1300 1310 $L        0 900 $NOK 0 0 $L $T;    run_case S4
case_dir S5  "$OK"                               1000 1010  860  870 $L        2 900 $NOK 0 0 $L $T;    run_case S5
case_dir S6  'SHARES_STOP=S\nE_FROZEN=0.1334\n'  1000 1010  860  870 $L        0 900 $NOK 0 0 $L $T;    run_case S6
case_dir S7  "$OK"                               1000 1010  860  870 130000000 0 900 $NOK 0 0 $L $T;    run_case S7
case_dir S8  "$OK"                               2000 2000 2000 2000 $L        0 840 480 1000 800 $L $T; run_case S8
case_dir S9  "$OK"                               1000 1010    0    0 $L        0 900 $NOK 0 0 $L $T;    run_case S9
case_dir S10 "$OK"                               1000 1010  860  870 $L        0 900 $NOK 0 0 $L $T
sed -i '' 's/^480,1.0,100,\([0-9]*\),\([0-9.]*\),480,16,[0-9]*$/480,1.0,100,\1,\2,480,16,x1/' "$OUT/S10/spec373b-b1.csv" \
  || { echo "FATAL: S10 setup (BSD sed -i '') failed" >&2; exit 2; }
grep -q ',480,16,x1$' "$OUT/S10/spec373b-b1.csv" || { echo "FATAL: S10 setup did not corrupt row t=480" >&2; exit 2; }
run_case S10
case_dir S11 'E_FROZEN=0.1334\n'                 1000 1010  860  870 $L        0 900 $NOK 0 0 $L $T;    run_case S11
case_dir S12 'SHARES_STOP=none\nE_FROZEN=abc\n'  1000 1010  860  870 $L        0 900 $NOK 0 0 $L $T;    run_case S12
case_dir S13 'SHARES_STOP=none\nE_FROZEN=0.3000\n' 1000 1060 600 610 $L        0 900 $NOK 0 0 $L $T;    run_case S13
case_dir S14 "$OK"                               1000 1010  700  705 $L        0 900 $NOK 0 0 $L $T;    run_case S14
case_dir S15 "$OK"                               1000 1010  860  870 $L        0 900 $NOK 0 0 $L 165000; run_case S15
case_dir S16 "$OK"                               1000 1010  860  870 80000000  0 900 $NOK 0 0 $L $T;    run_case S16
case_dir S17 "$OK"                               1000 1010  860  870 101000000 0 900 $NOK 0 0 102000000 $T; run_case S17
case_dir S18 'SHARES_STOP=none\n'                1000 1010  860  870 $L        0 900 $NOK 0 0 $L $T;    run_case S18
case_dir S19 "$OK"                               1000 1010  860  870 $L        0 900 $NOK 0 0 $L $T;    run_case S19 0
