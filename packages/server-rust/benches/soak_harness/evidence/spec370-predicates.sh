#!/usr/bin/env bash
# Runs the pre-registered slicing, fits, STOP predicates, deciding predicates, recorded readings and,
# LAST, the decision over the finished file, for one cell prefix.
# No `set -e`: a FALSE predicate must not stop the remaining blocks from running.
set -uo pipefail
EV="${1:-}"; BASE="${2:-}"
if [ "$#" -ne 2 ] || [ ! -d "$EV" ] || [ -z "$BASE" ]; then echo "usage: spec370-predicates.sh <EV_DIR> <BASE>" >&2; exit 2; fi
: > "$EV/$BASE.fits.txt"
: > "$EV/$BASE.predicates.txt"
export LC_ALL=C
case "$BASE" in *-r2) REPL=1 ;; *) REPL=0 ;; esac

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
  echo "== STOP: P-M =="
  awk '/^reconciliation=/ { seen=1; print ($0 ~ /^reconciliation=RECONCILED$/) ? "PM-reconciled=TRUE" : "PM-reconciled=FALSE reason=" $0 } END { if (!seen) print "PM-reconciled=FALSE reason=no_reconciliation_line" }' "$EV/$BASE.readout.txt"
  awk '/^reconciliation=/ { seen=1; print (index($0,"split_epochs=")==0) ? "PM-split=TRUE" : "PM-split=FALSE reason=" substr($0, index($0,"split_epochs=")) } END { if (!seen) print "PM-split=FALSE reason=no_reconciliation_line" }' "$EV/$BASE.readout.txt"
  awk -f "$EV/spec366-p5.awk" "$EV/$BASE.harness-console.log"
  awk -f "$EV/spec366-p67.awk" -v scrapes_dir="$EV/$BASE.scrapes" "$EV/$BASE.harness-console.log"

  echo "== STOP: P-S =="
  awk -f "$EV/spec368-ps.awk" "$EV/$BASE.csv"
  awk '/^READOUT: / { seen=1; v=$2; sub(/;$/, "", v); print (v == "O2" || v == "O3") ? "PS-verdict=TRUE verdict=" v : "PS-verdict=FALSE reason=verdict_" v } END { if (!seen) print "PS-verdict=FALSE reason=no_readout_line" }' "$EV/$BASE.readout.txt"

  echo "== STOP: PV =="
  awk 'NR==1 { ok = ($0 ~ /^provenance: server sha256=[0-9a-f]+ built=[^ ]+ run_start=[^ ]+ topgun_or_prune_restored_cancelled_total=present harness sha256=[0-9a-f]+ harness_built=[^ ]+ tombstone_level_ceiling_gate=present$/); s=$0; sub(/^provenance: server sha256=/, "", s); sub(/ .*/, "", s); h="NA"; if (index($0, " harness sha256=") > 0) { h=$0; sub(/.* harness sha256=/, "", h); sub(/ .*/, "", h) }; print "PV-line1 server_sha256=" s " harness_sha256=" h; line1 = ok } END { if (NR == 0) { print "PV=FALSE reason=empty_console_log"; exit 0 } if (!line1) { print "PV=FALSE reason=line1_shape"; exit 0 } print "PV-shape=TRUE" }' "$EV/$BASE.harness-console.log" > "$EV/$BASE.pv.tmp"
  cat "$EV/$BASE.pv.tmp"
  if grep -q '^PV-shape=TRUE$' "$EV/$BASE.pv.tmp"; then
    s1="$(sed -n 's/^PV-line1 server_sha256=\([^ ]*\) .*/\1/p' "$EV/$BASE.pv.tmp")"
    h1="$(sed -n 's/^PV-line1 .* harness_sha256=//p' "$EV/$BASE.pv.tmp")"
    sm="$(grep -E '^    sha256: +[0-9a-f]{64}$' "$EV/$BASE.matrix.txt" | head -1 | awk '{print $2}')"
    hm="$(grep -E '^  harness sha256: +[0-9a-f]{64}$' "$EV/$BASE.matrix.txt" | head -1 | awk '{print $3}')"
    if printf '%s\n%s\n' "$s1" "$h1" | grep -qvE '^[0-9a-f]{64}$'; then echo "PV=FALSE reason=sha_not_64_hex server=$s1 harness=$h1"
    elif [ "$s1" = "$sm" ] && [ "$h1" = "$hm" ]; then echo "PV=TRUE matrix_server_sha256=$sm matrix_harness_sha256=$hm"
    else echo "PV=FALSE reason=sha_mismatch server_line1=$s1 server_matrix=$sm harness_line1=$h1 harness_matrix=$hm"; fi
  fi
  rm -f "$EV/$BASE.pv.tmp"

  echo "== STOP: PR =="
  rows="$(( $(wc -l < "$EV/$BASE.csv") - 1 ))"
  scr="$(ls -1 "$EV/$BASE.scrapes" | grep -cE '^[0-9-]+T[0-9:]+Z\.txt$')"
  if [ "$rows" -ge 229 ] && [ "$rows" -eq "$scr" ]; then echo "PR-rows=TRUE rows=$rows scrapes=$scr"; else echo "PR-rows=FALSE rows=$rows scrapes=$scr"; fi
  cr="$(jq -r '.crashes // "null"' "$EV/$BASE.soak.json")"
  if [ "$cr" = "0" ]; then echo "PR-crashes=TRUE crashes=0 boot_gap_set=empty"; else echo "PR-crashes=FALSE crashes=$cr"; fi
  awk '/^RESULT: instrument sound; harness exit code [0-9]+\.$/ { c = 1 } /^RESULT: INSTRUMENT DEFECT/ { d = 1 } END { print (c && !d) ? "PR-class=TRUE" : "PR-class=FALSE reason=" (d ? "instrument_defect" : "no_sound_result_line") }' "$EV/$BASE.runner-console.log"

  echo "== DECIDING: P-K =="
  W="$(jq -r '.epochWidth // "null"' "$EV/$BASE.soak.json")"
  B="$(jq -r '.tombstones.tagBytesMax // "null"' "$EV/$BASE.soak.json")"
  S="$(jq -r '.tombstones.stampsInWindowMax // "null"' "$EV/$BASE.soak.json")"
  awk -v stamps_window_max="$S" -v width="$W" -f "$EV/spec370-pk.awk" "$EV/$BASE.csv" > "$EV/$BASE.pk.tmp"
  cat "$EV/$BASE.pk.tmp"
  K="$(sed -n 's/^PK=\([0-9][0-9]*\)$/\1/p' "$EV/$BASE.pk.tmp")"
  rm -f "$EV/$BASE.pk.tmp"
  jq -r '[.tombstones.orRemoveAckLatencyP99Ms, .tombstones.orRemoveAckLatencyMaxMs, .tombstones.sampleIntervalMs, .tombstones.orRemoveUnackedCount] | map(if . == null then "null" else tostring end) | @tsv' "$EV/$BASE.soak.json" \
    | awk -F '\t' 'NR == 1 { seen = 1
        if ($1 !~ /^[0-9]+$/ || $3 !~ /^[0-9]+$/) { printf("PK-latency=UNKNOWN p99_ms=%s max_ms=%s delta_ms=%s unacked=%s recorded_not_gated\n", $1, $2, $3, $4); next }
        printf("PK-latency p99_ms=%s max_ms=%s delta_ms=%s unacked=%s p99<=delta=%s max<=delta=%s recorded_not_gated\n", $1, $2, $3, $4, ($1 + 0 <= $3 + 0) ? "TRUE" : "FALSE", ($2 !~ /^[0-9]+$/) ? "UNKNOWN" : (($2 + 0 <= $3 + 0) ? "TRUE" : "FALSE")) }
      END { if (!seen) print "PK-latency=UNKNOWN reason=no_input recorded_not_gated" }'
  echo "== DECIDING: P-C / P-L =="
  awk -v epochs="${K:-x}" -v width="$W" -v tagmax="$B" -f "$EV/spec370-pc.awk" "$EV/$BASE.csv" > "$EV/$BASE.pc.tmp"
  cat "$EV/$BASE.pc.tmp"
  PCC="$(sed -n 's/^PC-inputs .* ceiling_bytes=\([0-9]*\).*/\1/p' "$EV/$BASE.pc.tmp")"
  PCV="$(grep -q '^PC=TRUE$' "$EV/$BASE.pc.tmp" && echo TRUE || echo FALSE)"
  PLV="$(grep -q '^PL=TRUE$' "$EV/$BASE.pc.tmp" && echo TRUE || echo FALSE)"
  rm -f "$EV/$BASE.pc.tmp"

  echo "== DECIDING: P-H =="
  jq -r '[.tombstones.ceilingBreached, .tombstones.levelBreached, .tombstones.ceilingDisposition, .tombstones.levelDisposition, .tombstones.ceilingEpochs, .epochWidth, .tombstones.tagBytesMax, .tombstones.stampsInWindowMax, .tombstones.ceilingBytes, .tombstones.peakBytes] | map(if . == null then "null" else tostring end) | @tsv' "$EV/$BASE.soak.json" \
    | awk -v pc="$PCV" -v pl="$PLV" -v pk="${K:-x}" -v pb="$B" -v pc_ceiling="${PCC:-0}" -f "$EV/spec370-ph.awk"
  jq -c '.tombstones | {kEffExpectedUnderO2, heldEpochsMaxObserved, neitherEpochsMaxObserved, durableWatermarkLagMaxObserved, fenceAgeBoundMs, stampsInWindowMax, slopeBytesPerHour}' "$EV/$BASE.soak.json"

  echo "== recorded: A7 =="
  awk -v t0="$(ls -1 "$EV/$BASE.scrapes" | grep -E '^[0-9-]+T[0-9:]+Z\.txt$' | sort | head -1 | cut -c1-19)" -f "$EV/spec368-a7.awk" "$EV/$BASE.harness-console.log"
  echo "== recorded: Theil-Sen =="
  awk -f "$EV/spec370-ts.awk" "$EV/$BASE.csv"
  echo "== recorded: WAL watermark alarms =="
  n="$(grep -c 'topgun_server::storage::wal_watermark' "$EV/$BASE.harness-console.log")"
  echo "PA-wal_watermark_alarm_lines=${n:-0} recorded_not_gated"
  echo "== recorded: P-F =="
  awk -f "$EV/spec368-pf.awk" "$EV/$BASE.csv"
  for c in phys_footprint_mb reclaimable_mb; do
    awk -v c="$c" '$1 == c && $2 == "LH" { for (i = 3; i <= NF; i++) if ($i ~ /^slope_mb_per_hour=/) { s = substr($i, 19); print "PF-shape " c " last_half_slope_sign=" ((s + 0 > 0) ? "+" : ((s + 0 < 0) ? "-" : "0")) " slope_mb_per_hour=" s; f = 1 } } END { if (!f) print "PF-shape " c " last_half_slope_sign=NA reason=no_LH_line" }' "$EV/$BASE.fits.txt"
  done
  jq -c '.reading, .reason, (.decidingSeries[] | {name, shape, firingEnvelope, lastHalfMean})' "$EV/$BASE.soak.durable.json"

  echo "== recorded: census =="
  jq -c '.censusTerminal | {source, elapsedSecs, keysScanned, keysUndecodable, orMapKeys, tombstoneEntries, tombstoneBytes, tombstoneDupEntries, keysWithTombstones, keysAllDead, maxTombstonesPerKey}' "$EV/$BASE.soak.durable.json"
  jq '.censuses | length' "$EV/$BASE.soak.durable.json"
  jq -c '.tombstoneCorpus' "$EV/$BASE.soak.json"

  echo "== recorded: harness attribution =="
  grep -E '^(RESULT:|RUNNER_EXIT=|csv rows:|harness exited with code)' "$EV/$BASE.runner-console.log"
  jq -r '.passed, .finishedReason, .durationSecsActual, (.pendingGates | length)' "$EV/$BASE.soak.json"
  head -1 "$EV/$BASE.harness-console.log"
  grep -E '^  +(sha256|code freeze|code freeze diff|harness sha256)' "$EV/$BASE.matrix.txt"
} >> "$EV/$BASE.predicates.txt"

# The deciding flags are computed only now, over the finished file, so no flag can be printed before a
# STOP predicate it depends on.
DEC="$(awk -v replicate="$REPL" -f "$EV/spec370-decide.awk" "$EV/$BASE.predicates.txt")"
{ echo "== DECISION =="; printf '%s\n' "$DEC"; } >> "$EV/$BASE.predicates.txt"
