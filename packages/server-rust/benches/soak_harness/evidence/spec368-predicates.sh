#!/usr/bin/env bash
# Runs the pre-registered slicing, fits and predicate blocks, in manifest order, for one cell prefix.
# No `set -e`: a FALSE predicate must not stop the remaining blocks from running.
set -uo pipefail
EV="${1:-}"; BASE="${2:-}"
if [ "$#" -ne 2 ] || [ ! -d "$EV" ] || [ -z "$BASE" ]; then echo "usage: spec368-predicates.sh <EV_DIR> <BASE>" >&2; exit 2; fi
: > "$EV/$BASE.fits.txt"
: > "$EV/$BASE.predicates.txt"
export LC_ALL=C

# Slicing: 8 equal-row windows; each segment file is overwritten.
( cd "$EV" && awk -F, 'NR==1{h=$0; next} {rows[++n]=$0}
  END{seg=int((n+7)/8);
      for(i=1;i<=8;i++){f=sprintf("'"$BASE"'-seg%d.csv",i); print h > f;
        for(j=(i-1)*seg+1; j<=i*seg && j<=n; j++) print rows[j] > f; close(f)}}' "$BASE.csv" )

# Fits: one newline-terminated record per window, FIT_ERROR on a non-zero fitter exit.
fit_record() {
  out="$(awk -f spec349c2-fit.awk -v col="$1" -v window="$2" "$3")"; rc=$?
  if [ "$rc" -ne 0 ]; then printf 'FIT_ERROR rc=%s\n' "$rc"; else printf '%s\n' "$out"; fi
}
( cd "$EV" && for c in tombstone_bytes phys_footprint_mb reclaimable_mb; do
  for i in 1 2 3 4 5 6 7 8; do printf '%s W%s ' "$c" "$i"; fit_record "$c" full "$BASE-seg$i.csv"; done
  printf '%s LH ' "$c"; fit_record "$c" last_half "$BASE.csv"
done ) >> "$EV/$BASE.fits.txt"

{
  echo "== P-M =="
  awk '/^reconciliation=/ { seen=1; print ($0 ~ /^reconciliation=RECONCILED$/) ? "PM-reconciled=TRUE" : "PM-reconciled=FALSE reason=" $0 } END { if (!seen) print "PM-reconciled=FALSE reason=no_reconciliation_line" }' "$EV/$BASE.readout.txt"
  awk '/^reconciliation=/ { seen=1; print (index($0,"split_epochs=")==0) ? "PM-split=TRUE" : "PM-split=FALSE reason=" substr($0, index($0,"split_epochs=")) } END { if (!seen) print "PM-split=FALSE reason=no_reconciliation_line" }' "$EV/$BASE.readout.txt"
  awk -f "$EV/spec366-p5.awk" "$EV/$BASE.harness-console.log"
  awk -f "$EV/spec366-p67.awk" -v scrapes_dir="$EV/$BASE.scrapes" "$EV/$BASE.harness-console.log"
  awk -v t0="$(ls -1 "$EV/$BASE.scrapes" | grep -E '^[0-9-]+T[0-9:]+Z\.txt$' | sort | head -1 | cut -c1-19)" -f "$EV/spec368-a7.awk" "$EV/$BASE.harness-console.log"

  echo "== P-S =="
  awk -f "$EV/spec368-ps.awk" "$EV/$BASE.csv"
  awk '/^READOUT: / { seen=1; v=$2; sub(/;$/, "", v); print (v == "O2" || v == "O3") ? "PS-verdict=TRUE verdict=" v : "PS-verdict=FALSE reason=verdict_" v } END { if (!seen) print "PS-verdict=FALSE reason=no_readout_line" }' "$EV/$BASE.readout.txt"

  echo "== P-B =="
  slope() { awk -f "$EV/spec349c2-fit.awk" -v col=tombstone_bytes -v window="$2" "$1" | awk '{ for (i = 1; i <= NF; i++) if ($i ~ /^slope_mb_per_hour=/) { sub(/^slope_mb_per_hour=/, "", $i); print $i } }'; }
  awk -v s1="$(slope "$EV/$BASE-seg1.csv" full)" -v s8="$(slope "$EV/$BASE-seg8.csv" full)" \
      -v slh="$(slope "$EV/$BASE.csv" last_half)" -f "$EV/spec368-pb.awk" "$EV/$BASE.csv"

  echo "== P-F =="
  awk -f "$EV/spec368-pf.awk" "$EV/$BASE.csv"
  for c in phys_footprint_mb reclaimable_mb; do
    awk -v c="$c" '$1 == c && $2 == "LH" { for (i = 3; i <= NF; i++) if ($i ~ /^slope_mb_per_hour=/) { s = substr($i, 19); print "PF-shape " c " last_half_slope_sign=" ((s + 0 > 0) ? "+" : ((s + 0 < 0) ? "-" : "0")) " slope_mb_per_hour=" s; f = 1 } } END { if (!f) print "PF-shape " c " last_half_slope_sign=NA reason=no_LH_line" }' "$EV/$BASE.fits.txt"
  done
  jq -c '.reading, .reason, (.decidingSeries[] | {name, shape, firingEnvelope, lastHalfMean})' "$EV/$BASE.soak.durable.json"

  echo "== census =="
  jq -c '.censusTerminal | {source, elapsedSecs, keysScanned, keysUndecodable, orMapKeys, tombstoneEntries, tombstoneBytes, tombstoneDupEntries, keysWithTombstones, keysAllDead, maxTombstonesPerKey}' "$EV/$BASE.soak.durable.json"
  jq '.censuses | length' "$EV/$BASE.soak.durable.json"
  jq -c '.tombstoneCorpus' "$EV/$BASE.soak.json"

  echo "== harness attribution =="
  grep -E '^(RESULT:|RUNNER_EXIT=|csv rows:|harness exited with code)' "$EV/$BASE.runner-console.log"
  jq -r '.passed, .finishedReason, .durationSecsActual' "$EV/$BASE.soak.json"
  head -1 "$EV/$BASE.harness-console.log"
  grep -E '^  +(sha256|code freeze|code freeze diff)' "$EV/$BASE.matrix.txt"
} >> "$EV/$BASE.predicates.txt"
