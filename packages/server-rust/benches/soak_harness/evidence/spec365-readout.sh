#!/usr/bin/env bash
# spec365-readout.sh <basename> -- mechanical readout over a conjunct cell's
# own committed artifacts (CSV, console log, durable-census JSON).
#
# usage: bash spec365-readout.sh <basename>
#
# Reads, from the same OUT_DIR convention spec365-conjuncts.sh uses
# (SPEC365_OUT_DIR, defaulting to this script's own directory):
#   <basename>.csv
#   <basename>.harness-console.log
#   <basename>.soak.durable.json
# Writes <basename>.readout.txt.
#
# This script computes no threshold and reads no environment beyond
# SPEC365_OUT_DIR: every decision below is a function of the three input
# files only, so the same inputs always produce the same output bytes --
# there is no wall-clock, no PID, and no listing order anywhere in it.
#
# WHY a single JSON dependency is allowed while everything else stays
# POSIX+awk: soak.durable.json is written by serde_json::to_writer_pretty
# from a #[serde(rename_all = "camelCase")] struct, so the reliable way to
# pull four scalars out of it without re-deriving a JSON parser in awk is
# the same tool the runner this reads already depends on for its own JSON
# artifacts.
set -euo pipefail

BASE="${1:?usage: spec365-readout.sh <basename>}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
EVIDENCE_DIR="$SCRIPT_DIR"
OUT_DIR="${SPEC365_OUT_DIR:-$EVIDENCE_DIR}"

CSV="${OUT_DIR}/${BASE}.csv"
CONSOLE="${OUT_DIR}/${BASE}.harness-console.log"
DURABLE_JSON="${OUT_DIR}/${BASE}.soak.durable.json"
READOUT="${OUT_DIR}/${BASE}.readout.txt"

for f in "$CSV" "$CONSOLE" "$DURABLE_JSON"; do
  if [ ! -r "$f" ]; then
    echo "FATAL: cannot read required input $f" >&2
    exit 2
  fi
done

# The D5 header literal, byte for byte -- a drift here means the runner and
# this readout have gone out of sync, which is a FATAL condition rather
# than something to parse around: every column offset below is positional.
EXPECTED_HEADER='elapsed_secs,rss_mb,wal_mb,redb_mb,disk_total_mb,tombstone_bytes,phys_footprint_mb,phys_footprint_peak_mb,reclaimable_mb,compressed_mb,conj_snapshots_total,conj_current_epoch,conj_ceiling,conj_durable_watermark,durable_watermark_lag,claims,claim_lag_p50,claim_lag_p99,claim_lag_max,ret_epochs_claim_only,ret_epochs_durability_only,ret_epochs_both,ret_epochs_neither,ret_refs_claim_only,ret_refs_durability_only,ret_refs_both,ret_refs_neither,ret_stamped_bytes,ret_epochs_unslotted,ret_refs_open_epoch,ret_stamped_bytes_open_epoch,indexed_refs,considered_total,dropped_total,matched_nothing_total,absent_total,bytes_freed_total,removed_refs_observed_total,removed_bytes_observed_total,stamped_bytes_total,clean_mb'
ACTUAL_HEADER="$(head -n 1 "$CSV")"
if [ "$ACTUAL_HEADER" != "$EXPECTED_HEADER" ]; then
  echo "FATAL: $CSV header does not match the D5 literal" >&2
  echo "  expected: $ACTUAL_HEADER" >&2
  exit 2
fi

DATA_ROWS="$(($(wc -l < "$CSV") - 1))"
if [ "$DATA_ROWS" -lt 1 ]; then
  echo "FATAL: $CSV carries no data row beneath its header" >&2
  exit 2
fi

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

LAST_ROW="$(tail -n +2 "$CSV" | tail -n 1)"
LAST_ROW_COLS="$(awk -F',' '{print NF}' <<<"$LAST_ROW")"
if [ "$LAST_ROW_COLS" != "41" ]; then
  echo "FATAL: last row of $CSV has $LAST_ROW_COLS columns, expected 41" >&2
  exit 2
fi

# Positional accessor -- the D5 header is a fixed 41-column literal, so a
# column's meaning is its position, not a name looked up at run time.
csv_col() {
  awk -F',' -v n="$1" '{print $n}' <<<"$LAST_ROW"
}

B_STORE="$(csv_col 6)"
CONJ_CURRENT_EPOCH="$(csv_col 12)"
RET_REFS_CLAIM_ONLY="$(csv_col 24)"
RET_REFS_DURABILITY_ONLY="$(csv_col 25)"
RET_REFS_BOTH="$(csv_col 26)"
RET_STAMPED_BYTES="$(csv_col 28)"
RET_EPOCHS_UNSLOTTED="$(csv_col 29)"
RET_STAMPED_BYTES_OPEN_EPOCH="$(csv_col 31)"
REMOVED_REFS_OBSERVED_TOTAL_LAST="$(csv_col 38)"
CONSIDERED_TOTAL_LAST="$(csv_col 33)"

# O4 guard: any empty cell in column 6 or columns 11-40 of the last row.
# Columns 32-40 additionally feed the sectionC last-row metric check below.
ANY_EMPTY_CELL="$(awk -F',' '
  {
    empty = 0
    if ($6 == "") empty = 1
    for (i = 11; i <= 40; i++) if ($i == "") empty = 1
    print empty
  }
' <<<"$LAST_ROW")"

# ---------------------------------------------------------------------------
# 1. Single-pass extraction of the three target line families out of the
#    console log. The harness prefixes every passthrough line "[server] "
#    (process.rs's `eprintln!("[server] {line}")`) and the underlying
#    tracing fmt layer can carry ANSI SGR codes; both are stripped before
#    matching, because the target/field boundaries only line up once they
#    are gone.
# ---------------------------------------------------------------------------
REMOVAL_TSV="$TMP_DIR/removal.tsv"
SETTLEMENT_TSV="$TMP_DIR/settlement.tsv"
CONJUNCT_TSV="$TMP_DIR/conjunct.tsv"
: > "$REMOVAL_TSV"
: > "$SETTLEMENT_TSV"
: > "$CONJUNCT_TSV"

cat >"$TMP_DIR/extract.awk" <<'AWKEOF'
# Read one field's rendered token out of a matched line. Every field this
# script reads is a plain tracing scalar (u64, bool, or the unquoted
# `retained` list) -- none of them contain spaces -- so "look for the
# leading-space-delimited key=, take the token up to the next space" is
# exact, not an approximation, for every field this extractor is asked for.
function field(line, name,    needle, start, rest, sp) {
    needle = " " name "="
    start = index(line, needle)
    if (start == 0) {
        printf("FATAL: field %s missing from matched line: %s\n", name, line) > "/dev/stderr"
        exit_bad = 1
        exit 2
    }
    start = start + length(needle)
    rest = substr(line, start)
    sp = index(rest, " ")
    if (sp == 0) return rest
    return substr(rest, 1, sp - 1)
}
index($0, "topgun_server::tombstone_frontier::removal:") > 0 {
    printf("%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n", field($0, "ts"), field($0, "op_seq"), field($0, "epoch"), field($0, "refs_returned"), field($0, "refs_at_entry"), field($0, "bytes_returned"), field($0, "watermark"), field($0, "ceiling")) >> removal_out
    next
}
index($0, "topgun_server::tombstone_frontier::settlement:") > 0 {
    printf("%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n", field($0, "epoch"), field($0, "considered"), field($0, "dropped"), field($0, "matched_nothing"), field($0, "absent"), field($0, "restored_read_error"), field($0, "restored_evicted"), field($0, "restored_write_error"), field($0, "bytes_freed")) >> settlement_out
    next
}
index($0, "topgun_server::tombstone_frontier::conjunct:") > 0 {
    printf("%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n", field($0, "seq"), field($0, "ts"), field($0, "current_epoch"), field($0, "ceiling"), field($0, "durable_watermark"), field($0, "durable_watermark_lag"), field($0, "claims"), field($0, "claim_lag_p50"), field($0, "claim_lag_p99"), field($0, "claim_lag_max"), field($0, "retained_epochs_claim_only"), field($0, "retained_epochs_durability_only"), field($0, "retained_epochs_both"), field($0, "retained_epochs_neither"), field($0, "retained_refs_claim_only"), field($0, "retained_refs_durability_only"), field($0, "retained_refs_both"), field($0, "retained_refs_neither"), field($0, "retained_stamped_bytes"), field($0, "retained_epochs_unslotted"), field($0, "retained_refs_open_epoch"), field($0, "retained_stamped_bytes_open_epoch"), field($0, "retained"), field($0, "retained_truncated")) >> conjunct_out
    next
}
END {
    if (exit_bad) exit 2
}
AWKEOF

sed -e 's/^\[server\] //' -e 's/\x1b\[[0-9;]*m//g' "$CONSOLE" | \
  LC_ALL=C awk -f "$TMP_DIR/extract.awk" \
    -v removal_out="$REMOVAL_TSV" \
    -v settlement_out="$SETTLEMENT_TSV" \
    -v conjunct_out="$CONJUNCT_TSV"

# ---------------------------------------------------------------------------
# 2. sectionA -- the conjunct line selected per the seq/conj_snapshots_total
#    pin, falling back to the last conjunct line on a miss.
# ---------------------------------------------------------------------------
SECTIONA_TXT="$TMP_DIR/sectionA.txt"
SECTIONA_VARS="$TMP_DIR/sectionA.vars"

cat >"$TMP_DIR/sectionA.awk" <<'AWKEOF'
function split_retained(list, out,    n, i) {
    n = split(list, out, ",")
    return n
}
BEGIN {
    FS = "\t"
    picked = 0
}
{
    seqs[NR] = $1
    lines[NR] = $0
    last_nr = NR
    if ($1 == target_seq) {
        picked_nr = NR
        picked = 1
    }
}
END {
    if (last_nr == 0) {
        print "no_conjunct=1" > vars_out
        print "SectionA: no conjunct line found in the console log." > text_out
        close(vars_out)
        close(text_out)
        exit 0
    }
    if (picked) {
        sel_nr = picked_nr
        source_label = "matched"
    } else {
        sel_nr = last_nr
        source_label = "fallback"
    }
    split(lines[sel_nr], f, "\t")
    seq              = f[1]
    cts               = f[2]
    current_epoch     = f[3]
    ceiling           = f[4]
    durable_watermark = f[5]
    dwl               = f[6]
    claims            = f[7]
    p50               = f[8]
    p99               = f[9]
    lmax              = f[10]
    eco               = f[11]  # retained_epochs_claim_only
    edo               = f[12]  # retained_epochs_durability_only
    ebo               = f[13]  # retained_epochs_both
    eno               = f[14]  # retained_epochs_neither
    rsb               = f[19]  # retained_stamped_bytes
    reu               = f[20]  # retained_epochs_unslotted
    rro               = f[21]  # retained_refs_open_epoch
    rsbo              = f[22]  # retained_stamped_bytes_open_epoch
    retained          = f[23]
    truncated         = f[24]

    n = split_retained(retained, tok)
    printf("SectionA: source=%s seq=%s conj_snapshots_total=%s\n", source_label, seq, target_seq) > text_out
    printf("epoch | class | open\n") > text_out
    c_claim = 0; c_dur = 0; c_both = 0; c_neither = 0
    for (i = 1; i <= n; i++) {
        colon = index(tok[i], ":")
        ep = substr(tok[i], 1, colon - 1)
        cl = substr(tok[i], colon + 1)
        printf("%s | %s | false\n", ep, cl) > text_out
        if (cl == "claim_only") c_claim++
        else if (cl == "durability_only") c_dur++
        else if (cl == "both") c_both++
        else if (cl == "neither") c_neither++
    }
    printf("%s |  | true\n", current_epoch) > text_out

    mismatch = 0
    if (c_claim != eco || c_dur != edo || c_both != ebo || c_neither != eno) mismatch = 1

    printf("consistency: counted(claim_only=%d durability_only=%d both=%d neither=%d) line(claim_only=%s durability_only=%s both=%s neither=%s) retained_truncated=%s %s\n", c_claim, c_dur, c_both, c_neither, eco, edo, ebo, eno, truncated, (mismatch ? "MISMATCH" : "OK")) > text_out

    print "no_conjunct=0" > vars_out
    printf("retained_closed_epochs=%d\n", n) > vars_out
    printf("retained_truncated=%s\n", truncated) > vars_out
    printf("sectionA_mismatch=%d\n", mismatch) > vars_out
    printf("ret_stamped_bytes_open_epoch_line=%s\n", rsbo) > vars_out

    close(text_out)
    close(vars_out)
}
AWKEOF

LC_ALL=C awk -f "$TMP_DIR/sectionA.awk" \
  -v target_seq="$(csv_col 11)" \
  -v text_out="$SECTIONA_TXT" \
  -v vars_out="$SECTIONA_VARS" \
  "$CONJUNCT_TSV"

# shellcheck source=/dev/null
. "$SECTIONA_VARS"

# ---------------------------------------------------------------------------
# 3. sectionC -- per-epoch removal/settlement reconciliation.
# ---------------------------------------------------------------------------
REMOVAL_AGG="$TMP_DIR/removal_agg.tsv"
REMOVAL_SCALARS="$TMP_DIR/removal_scalars.tsv"
# Pre-created: an awk `>` redirect only creates its target on the first
# actual write, and an empty removal/settlement log (e.g. every O4 fixture
# that never reaches a removal line) legitimately writes zero rows.
: > "$REMOVAL_AGG"
: > "$REMOVAL_SCALARS"

cat >"$TMP_DIR/agg_removal.awk" <<'AWKEOF'
BEGIN { FS = "\t"; global_max_op_seq = -1; max_bytes_returned = 0 }
{
    ep = $3
    if (!(ep in seen)) { seen[ep] = 1; order[++norder] = ep; first_entry[ep] = $5 }
    passes[ep]++
    refs_sum[ep] += $4
    bytes_sum[ep] += $6
    if ($2 + 0 > max_op_seq[ep] + 0 || !(ep in max_op_seq)) max_op_seq[ep] = $2
    if ($2 + 0 > global_max_op_seq) global_max_op_seq = $2 + 0
    if ($6 + 0 > max_bytes_returned) max_bytes_returned = $6 + 0
}
END {
    for (i = 1; i <= norder; i++) {
        ep = order[i]
        printf("%s\t%d\t%s\t%d\t%d\t%s\n", ep, passes[ep], first_entry[ep], refs_sum[ep], bytes_sum[ep], max_op_seq[ep]) > agg_out
    }
    printf("global_max_op_seq\t%d\n", global_max_op_seq) > scalars_out
    printf("max_bytes_returned\t%d\n", max_bytes_returned) > scalars_out
    close(agg_out)
    close(scalars_out)
}
AWKEOF
LC_ALL=C awk -f "$TMP_DIR/agg_removal.awk" -v agg_out="$REMOVAL_AGG" -v scalars_out="$REMOVAL_SCALARS" "$REMOVAL_TSV"

SETTLEMENT_AGG="$TMP_DIR/settlement_agg.tsv"
: > "$SETTLEMENT_AGG"
cat >"$TMP_DIR/agg_settlement.awk" <<'AWKEOF'
BEGIN { FS = "\t" }
{
    ep = $1
    if (!(ep in seen)) { seen[ep] = 1; order[++norder] = ep }
    cnt[ep]++
    considered[ep] += $2
    dropped[ep] += $3
    matched_nothing[ep] += $4
    absent[ep] += $5
    restored_row = $6 + $7 + $8
    restored_sum[ep] += restored_row
    bytes_freed[ep] += $9
}
END {
    for (i = 1; i <= norder; i++) {
        ep = order[i]
        printf("%s\t%d\t%d\t%d\t%d\t%d\t%d\t%d\n", ep, cnt[ep], considered[ep], dropped[ep], matched_nothing[ep], absent[ep], restored_sum[ep], bytes_freed[ep]) > agg_out
    }
    close(agg_out)
}
AWKEOF
LC_ALL=C awk -f "$TMP_DIR/agg_settlement.awk" -v agg_out="$SETTLEMENT_AGG" "$SETTLEMENT_TSV"

GLOBAL_MAX_OP_SEQ="$(awk -F'\t' '$1=="global_max_op_seq"{print $2}' "$REMOVAL_SCALARS")"
MAX_BYTES_RETURNED="$(awk -F'\t' '$1=="max_bytes_returned"{print $2}' "$REMOVAL_SCALARS")"
: "${GLOBAL_MAX_OP_SEQ:=-1}"
: "${MAX_BYTES_RETURNED:=0}"

SECTIONC_TXT="$TMP_DIR/sectionC.txt"
SECTIONC_VARS="$TMP_DIR/sectionC.vars"

cat >"$TMP_DIR/join_c.awk" <<'AWKEOF'
BEGIN { FS = "\t" }
FNR == NR {
    ep = $1
    r_passes[ep] = $2
    r_entry[ep] = $3
    r_refs[ep] = $4
    r_bytes[ep] = $5
    r_maxop[ep] = $6
    if (!(ep in seen)) { seen[ep] = 1; order[++norder] = ep }
    next
}
{
    ep = $1
    s_cnt[ep] = $2
    s_considered[ep] = $3
    s_dropped[ep] = $4
    s_matched_nothing[ep] = $5
    s_absent[ep] = $6
    s_restored[ep] = $7
    s_bytes_freed[ep] = $8
    if (!(ep in seen)) { seen[ep] = 1; order[++norder] = ep }
}
END {
    # Deterministic ascending-epoch order: epochs are small non-negative
    # integers, so a straight insertion sort over the (small) order list is
    # both exact and immune to any locale-dependent tool.
    for (i = 2; i <= norder; i++) {
        key = order[i]; j = i - 1
        while (j >= 1 && order[j] + 0 > key + 0) { order[j+1] = order[j]; j-- }
        order[j+1] = key
    }

    t_passes = 0; t_refs_at_entry = 0; t_refs_returned = 0; t_bytes_returned = 0
    t_considered = 0; t_dropped = 0; t_matched_nothing = 0; t_absent = 0
    t_restored = 0; t_bytes_freed = 0
    restored_sum_total = 0
    split_count = 0
    split_list = ""

    printf("SectionC: per exited epoch reconciliation\n") > text_out
    printf("epoch | passes | refs_at_entry | refs_returned | bytes_returned | considered | dropped | matched_nothing | absent | restored_sum | bytes_freed | status\n") > text_out

    for (i = 1; i <= norder; i++) {
        ep = order[i]
        passes = (ep in r_passes) ? r_passes[ep] : 0
        entry = (ep in r_entry) ? r_entry[ep] : 0
        refs_ret = (ep in r_refs) ? r_refs[ep] : 0
        bytes_ret = (ep in r_bytes) ? r_bytes[ep] : 0
        maxop = (ep in r_maxop) ? r_maxop[ep] : -1

        s_count = (ep in s_cnt) ? s_cnt[ep] : 0
        considered = (ep in s_considered) ? s_considered[ep] : 0
        dropped = (ep in s_dropped) ? s_dropped[ep] : 0
        matched_nothing = (ep in s_matched_nothing) ? s_matched_nothing[ep] : 0
        absent = (ep in s_absent) ? s_absent[ep] : 0
        restored = (ep in s_restored) ? s_restored[ep] : 0
        bytes_freed = (ep in s_bytes_freed) ? s_bytes_freed[ep] : 0

        lacks_settlement = (s_count < passes)
        if (!lacks_settlement) {
            considered_match = (considered == refs_ret)
            six_exit_ok = (dropped + matched_nothing + absent + restored == considered)
            status = (considered_match && six_exit_ok) ? "RECONCILED" : "MISMATCH"
        } else if (maxop + 0 == global_max_op_seq + 0) {
            status = "IN_FLIGHT"
        } else {
            status = "NO_SETTLEMENT"
        }

        if (status == "MISMATCH" || status == "NO_SETTLEMENT") {
            split_count++
            split_list = split_list (split_list == "" ? "" : ",") ep
        }

        printf("%s | %d | %s | %d | %d | %d | %d | %d | %d | %d | %d | %s\n", ep, passes, entry, refs_ret, bytes_ret, considered, dropped, matched_nothing, absent, restored, bytes_freed, status) > text_out

        t_passes += passes; t_refs_at_entry += entry; t_refs_returned += refs_ret; t_bytes_returned += bytes_ret
        t_considered += considered; t_dropped += dropped; t_matched_nothing += matched_nothing; t_absent += absent
        t_restored += restored; t_bytes_freed += bytes_freed
        restored_sum_total += restored
    }

    printf("totals: passes=%d refs_at_entry=%d refs_returned=%d bytes_returned=%d considered=%d dropped=%d matched_nothing=%d absent=%d restored_sum=%d bytes_freed=%d\n", t_passes, t_refs_at_entry, t_refs_returned, t_bytes_returned, t_considered, t_dropped, t_matched_nothing, t_absent, t_restored, t_bytes_freed) > text_out

    metric_match = (removed_refs_observed_total_last == considered_total_last) ? "MATCH" : "MISMATCH"
    printf("last-row metric check: removed_refs_observed_total=%s considered_total=%s %s\n", removed_refs_observed_total_last, considered_total_last, metric_match) > text_out

    reconciliation = (split_count == 0) ? "RECONCILED" : "SPLIT"
    printf("reconciliation=%s", reconciliation) > text_out
    if (split_count > 0) printf(" split_epochs=%s", split_list) > text_out
    printf("\n") > text_out

    printf("restored_sum_total=%d\n", restored_sum_total) > vars_out
    printf("reconciliation=%s\n", reconciliation) > vars_out
    printf("split_epochs=%s\n", split_list) > vars_out

    close(text_out)
    close(vars_out)
}
AWKEOF
LC_ALL=C awk -f "$TMP_DIR/join_c.awk" \
  -v text_out="$SECTIONC_TXT" \
  -v vars_out="$SECTIONC_VARS" \
  -v global_max_op_seq="$GLOBAL_MAX_OP_SEQ" \
  -v removed_refs_observed_total_last="$REMOVED_REFS_OBSERVED_TOTAL_LAST" \
  -v considered_total_last="$CONSIDERED_TOTAL_LAST" \
  "$REMOVAL_AGG" "$SETTLEMENT_AGG"

# shellcheck source=/dev/null
. "$SECTIONC_VARS"

# ---------------------------------------------------------------------------
# 4. sectionB -- where the retained tag tombstones live, read off the last
#    CSV row only (the same instant sectionA's seq pin targets).
# ---------------------------------------------------------------------------
SECTIONB_TXT="$TMP_DIR/sectionB.txt"

DURABLE_TOMBSTONE_ENTRIES="$(jq -r '.censusTerminal.tombstoneEntries // "absent"' "$DURABLE_JSON")"
DURABLE_OR_MAP_KEYS="$(jq -r '.censusTerminal.orMapKeys // "absent"' "$DURABLE_JSON")"
DURABLE_KEYS_WITH_TOMBSTONES="$(jq -r '.censusTerminal.keysWithTombstones // "absent"' "$DURABLE_JSON")"
DURABLE_MAX_TOMBSTONES_PER_KEY="$(jq -r '.censusTerminal.maxTombstonesPerKey // "absent"' "$DURABLE_JSON")"

X_SERIES="$TMP_DIR/x_series.tsv"
LC_ALL=C awk -F',' 'NR > 1 {
    if ($6 == "" || $28 == "" || $31 == "") { printf("%s\tNA\n", $1); next }
    x = $6 - ($28 + $31)
    printf("%s\t%d\n", $1, x)
}' "$CSV" > "$X_SERIES"

RESTORE_WINDOW=0
if [ "${RET_EPOCHS_UNSLOTTED:-0}" -gt 0 ] 2>/dev/null && [ "${restored_sum_total:-0}" -gt 0 ] 2>/dev/null; then
  RESTORE_WINDOW=1
fi

LC_ALL=C awk -v b_store="$B_STORE" -v rsb="$RET_STAMPED_BYTES" -v rsbo="$RET_STAMPED_BYTES_OPEN_EPOCH" \
  -v n="$RET_EPOCHS_UNSLOTTED" -v maxbytes="$MAX_BYTES_RETURNED" -v restore_window="$RESTORE_WINDOW" \
  -v tomb_e="$DURABLE_TOMBSTONE_ENTRIES" -v or_map_k="$DURABLE_OR_MAP_KEYS" \
  -v keys_tomb="$DURABLE_KEYS_WITH_TOMBSTONES" -v max_tomb="$DURABLE_MAX_TOMBSTONES_PER_KEY" \
  -v xfile="$X_SERIES" -v text_out="$SECTIONB_TXT" '
BEGIN {
    b_index = rsb + rsbo
    x = b_store - b_index
    printf("SectionB: where the retained tag tombstones live\n") > text_out
    printf("B_store=%d\n", b_store) > text_out
    printf("ret_stamped_bytes=%d\n", rsb) > text_out
    printf("ret_stamped_bytes_open_epoch=%d\n", rsbo) > text_out
    printf("B_index=%d\n", b_index) > text_out
    printf("X=%d\n", x) > text_out
    if (b_store + 0 == 0) {
        printf("X/B_store=NA\n") > text_out
    } else {
        printf("X/B_store=%.6f\n", x / b_store) > text_out
    }
    tau = (0.10 * b_store > 2048) ? 0.10 * b_store : 2048
    printf("tau=%.3f\n", tau) > text_out
    if (restore_window == "1") {
        bound = n * maxbytes
        taup = tau + bound
        printf("restore_window: n=%d max_bytes_returned=%d bound=%d tau_prime=%.3f\n", n, maxbytes, bound, taup) > text_out
    } else {
        taup = tau
    }
    printf("X series (elapsed_secs,X):\n") > text_out
    while ((getline line < xfile) > 0) {
        print line > text_out
    }
    close(xfile)
    printf("durable census (post-kill redb, flushed state only): tombstone_entries=%s or_map_keys=%s keys_with_tombstones=%s max_tombstones_per_key=%s\n", tomb_e, or_map_k, keys_tomb, max_tomb) > text_out
    close(text_out)

    # Publish the scalars the final verdict step needs, on stdout, one
    # "name=value" per line -- picked up by the caller via command
    # substitution into individual variables.
    printf("X=%d\n", x)
    printf("TAU=%.10f\n", tau)
    printf("TAU_PRIME=%.10f\n", taup)
}
' | tee "$TMP_DIR/verdict_b.vars" >/dev/null

# shellcheck source=/dev/null
. "$TMP_DIR/verdict_b.vars"

# ---------------------------------------------------------------------------
# 5. sectionD -- claim-lag distribution over every conjunct line.
# ---------------------------------------------------------------------------
SECTIOND_TXT="$TMP_DIR/sectionD.txt"
cat >"$TMP_DIR/sectionD.awk" <<'AWKEOF'
# Nearest-rank over a 1-indexed sorted array, the identical formula the
# server itself uses for claim_lag percentiles -- reused here so a "median
# across snapshots" is computed the same mechanical way the snapshots
# compute their own percentiles, rather than inventing a second convention.
function nearest_rank(arr, n, k,    idx) {
    if (n == 0) return 0
    idx = int((k * n + 99) / 100)
    if (idx < 1) idx = 1
    if (idx > n) idx = n
    return arr[idx]
}
function isort(arr, n,    i, j, key) {
    for (i = 2; i <= n; i++) {
        key = arr[i]; j = i - 1
        while (j >= 1 && arr[j] + 0 > key + 0) { arr[j+1] = arr[j]; j-- }
        arr[j+1] = key
    }
}
BEGIN { FS = "\t"; lines = 0 }
{
    lines++
    p50[lines] = $8
    p99[lines] = $9
    lmax = $10 + 0
    if (lines == 1 || lmax > max_claim_lag_max) max_claim_lag_max = lmax
    cl = $7 + 0
    if (lines == 1 || cl > max_claims) max_claims = cl
    dwl = $6 + 0
    if (lines == 1 || dwl > max_dwl) max_dwl = dwl
    last_dwl = $6
}
END {
    if (lines == 0) {
        printf("SectionD: no conjunct lines observed.\n") > text_out
        close(text_out)
        exit 0
    }
    isort(p50, lines)
    isort(p99, lines)
    printf("SectionD: claim-lag distribution over %d conjunct line(s)\n", lines) > text_out
    printf("max_claim_lag_max=%d\n", max_claim_lag_max) > text_out
    printf("claim_lag_p50: min=%s median=%s max=%s\n", p50[1], nearest_rank(p50, lines, 50), p50[lines]) > text_out
    printf("claim_lag_p99: min=%s median=%s max=%s\n", p99[1], nearest_rank(p99, lines, 50), p99[lines]) > text_out
    printf("max_claims=%d\n", max_claims) > text_out
    printf("durable_watermark_lag: max=%d last=%s\n", max_dwl, last_dwl) > text_out
    close(text_out)
}
AWKEOF
LC_ALL=C awk -f "$TMP_DIR/sectionD.awk" -v text_out="$SECTIOND_TXT" "$CONJUNCT_TSV"

# ---------------------------------------------------------------------------
# 6. sectionE -- footprint reconstruction over every CSV data row after t=0.
#    rss counts resident clean pages that phys_footprint does not, so the
#    reconstruction is phys_footprint + clean + reclaimable. The t=0 row is
#    taken at server-ready, before the working set exists, and is excluded.
# ---------------------------------------------------------------------------
SECTIONE_TXT="$TMP_DIR/sectionE.txt"
LC_ALL=C awk -F',' -v text_out="$SECTIONE_TXT" '
NR > 1 && $1 + 0 > 0 {
    total++
    if ($2 == "" || $7 == "" || $9 == "" || $41 == "" || $2 + 0 == 0) next
    diff = $2 - ($7 + $41 + $9)
    if (diff < 0) diff = -diff
    ratio = diff / $2
    if (ratio <= 0.02) within++
}
END {
    printf("SectionE: footprint reconstruction\n") > text_out
    printf("rows_within_2pct=%d/%d\n", within + 0, total + 0) > text_out
    close(text_out)
}
' "$CSV"

# ---------------------------------------------------------------------------
# 7. The verdict -- first-match-wins over O4, O1, O5, O2, O3.
# ---------------------------------------------------------------------------
O4=0
[ "${no_conjunct:-0}" = "1" ] && O4=1
[ "${retained_truncated:-false}" = "true" ] && O4=1
[ "${sectionA_mismatch:-0}" = "1" ] && O4=1
[ "$ANY_EMPTY_CELL" = "1" ] && O4=1
[ "$B_STORE" = "0" ] && O4=1
if [ "${RET_EPOCHS_UNSLOTTED:-0}" -gt 0 ] 2>/dev/null && [ "${restored_sum_total:-0}" -eq 0 ] 2>/dev/null; then
  O4=1
fi
if [ "${CONJ_CURRENT_EPOCH:-0}" -lt 2 ] 2>/dev/null; then
  O4=1
fi

VERDICT="$(LC_ALL=C awk -v o4="$O4" -v x="$X" -v tau="$TAU" -v taup="$TAU_PRIME" \
  -v claim="$RET_REFS_CLAIM_ONLY" -v dur="$RET_REFS_DURABILITY_ONLY" -v both="$RET_REFS_BOTH" '
BEGIN {
    if (o4 == 1) { print "O4"; exit }
    if (x + 0 > taup + 0) { print "O1"; exit }
    if (x + 0 < -(tau + 0)) { print "O5"; exit }
    if ((dur + both) > claim) { print "O2"; exit }
    print "O3"
}
')"

RESTORE_CLAUSE=""
if [ "$RESTORE_WINDOW" = "1" ]; then
  RESTORE_CLAUSE="; unslotted_restore_window=${RET_EPOCHS_UNSLOTTED}"
fi

FLINE="READOUT: ${VERDICT}; retained_closed_epochs=${retained_closed_epochs:-0}; reconciliation=${reconciliation:-SPLIT}${RESTORE_CLAUSE}"

# ---------------------------------------------------------------------------
# 8. Assemble the readout file.
# ---------------------------------------------------------------------------
{
  echo "=== spec365 conjunct readout: ${BASE} ==="
  echo
  cat "$SECTIONA_TXT"
  echo
  cat "$SECTIONB_TXT"
  echo
  cat "$SECTIONC_TXT"
  echo
  cat "$SECTIOND_TXT"
  echo
  cat "$SECTIONE_TXT"
  echo
  echo "$FLINE"
} > "$READOUT"

echo "wrote readout to $READOUT" >&2
echo "$FLINE" >&2
