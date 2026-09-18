#!/usr/bin/env bash
# Calibration of the level-ceiling predicates on committed 4 h cells: one expected PASS, two expected FAIL.
set -uo pipefail
EV="${1:-}"
if [ "$#" -ne 1 ] || [ ! -d "$EV" ]; then echo "usage: spec370-calibrate.sh <EV_DIR>" >&2; exit 2; fi
OUT="$EV/spec370-calibration.txt"
: > "$OUT"
export LC_ALL=C
# S_60 from the only calibration CSV that carries the stamp columns; applied to all three (same matrix).
S="$(awk -v stamps_window_max=0 -v width=1000 -f "$EV/spec370-pk.awk" "$EV/spec368-plateau4h.csv" | sed -n 's/^PK-crosscheck csv_stamps_per_row_max=\([0-9]*\) .*/\1/p')"
{ echo "## K derivation (spec368-plateau4h CSV)"; awk -v stamps_window_max="${S:-x}" -v width=1000 -f "$EV/spec370-pk.awk" "$EV/spec368-plateau4h.csv"; } >> "$OUT"
K="$(sed -n 's/^PK=\([0-9][0-9]*\)$/\1/p' "$OUT")"
C=$(( ${K:-0} * 1000 * 23 ))
for b in spec368-plateau4h spec355-w1000 spec362b-long4h; do
  {
    echo "## $b"
    awk -v epochs="${K:-x}" -v width=1000 -v tagmax=23 -f "$EV/spec370-pc.awk" "$EV/$b.csv"
    awk -f "$EV/spec370-ts.awk" "$EV/$b.csv"
    p="$(jq -r '.tombstones.peakBytes' "$EV/$b.soak.json")"
    if [ "$p" -le "$C" ] 2>/dev/null; then r=TRUE; else r=FALSE; fi
    echo "CAL-harness-peak $b peak_bytes=$p ceiling=$C <=ceiling=$r"
  } >> "$OUT"
done
