#!/bin/sh
# spec357-residency.sh -- THE RESIDENCY EXTRACTOR (Delta: "computes no predicate term").
# usage: spec357-residency.sh <server-console.log> <output-prefix>
#   writes <output-prefix>.entry.jsonl      one line per PruneEpochEntryRecord emission
#          <output-prefix>.residency.jsonl  one line per PruneEpochResidencyRecord emission
#
# WHAT THIS SCRIPT DOES AND DOES NOT DO. It reads `server-console.log` -- the runner's captured
# stdout/stderr of the server process, filtered by the targeted
# `SOAK_SERVER_LOG=warn,topgun_server::tombstone_frontier::residency=info` env var so the residency
# target's INFO lines are the only INFO lines present -- and ROUTES each matching line to one of the
# two output files by MESSAGE NAME (`epoch_entry` vs `epoch_exit`), passing the emitted record's own
# JSON payload through UNCHANGED. It evaluates NO field of either record: not `entered_index`, not
# `exit_kind`, not any of R3/R4's predicates. Routing on the message name a `tracing::info!` call
# chose is not evaluating a hypothesis; it is reading which of the two fixed emission SITES (R2.3a's
# rollover site, R2.3b's detection site) produced the line, which is a property of the LOG LINE, not
# of the record's content.
#
# THE EXPECTED WIRE SHAPE, AND WHY IT IS ROBUST TO BEING WRONG IN DETAIL. `tombstone_frontier.rs`
# already states the per-epoch join is not representable over the metrics transport, and the two new
# record types carry `#[serde(rename_all = "camelCase")]` (the Rust type-mapping rule every new
# struct here follows). The emitting code (a G2 deliverable, not this script's) is expected to log
# each record as a single JSON-serialized field on the `topgun_server::tombstone_frontier::residency`
# target, e.g. (default `tracing_subscriber::fmt()` rendering, the one this binary's own boot-summary
# test exercises):
#   <ts>  INFO topgun_server::tombstone_frontier::residency: epoch_entry record={"epoch":461,...}
# This extractor does NOT assume a field name for the JSON value (it does not require `record=`):
# it scans the whole matched line for the FIRST balanced, string-aware `{...}` substring and treats
# that as the payload. A balanced-brace scanner survives a renamed field, a reordered field list, or
# an extra field tracing's own default formatter appends -- the properties actually within this
# script's control -- without needing a second commit once G2 lands. What it cannot survive is a
# payload that is not JSON at all (e.g. one tracing field per struct field, Debug-formatted, with no
# single JSON blob on the line); that case is reported below as a SKIPPED / UNPARSEABLE line count
# rather than guessed at, per this script's own "no predicate term" rule -- inventing field values
# from a non-JSON rendering would be exactly that.
#
# EXIT: 0 on a clean scan (even if zero matching lines were found -- an honest empty run is not a
# script failure; Q3/Q4's PRESENCE bands are what judge the COUNT). Non-zero only on bad usage or an
# unreadable input file.

if [ $# -ne 2 ]; then
    echo "usage: $0 <server-console.log> <output-prefix>" >&2
    exit 2
fi

LOG=$1
PREFIX=$2
TARGET='topgun_server::tombstone_frontier::residency:'
ENTRY_OUT="${PREFIX}.entry.jsonl"
RESIDENCY_OUT="${PREFIX}.residency.jsonl"

[ -f "$LOG" ] || { echo "REFUSED: $LOG not readable" >&2; exit 2; }
command -v jq >/dev/null 2>&1 || { echo "REFUSED: jq is required and was not found on PATH" >&2; exit 2; }

: > "$ENTRY_OUT"
: > "$RESIDENCY_OUT"

# Balanced-brace, string-aware JSON extractor. Scans from the FIRST unquoted `{` to the `}` that
# closes it, tracking string state (and backslash-escapes inside strings) so a brace character
# appearing inside a quoted field value (e.g. `Unclassified`'s free-form `note` field) never
# miscounts depth. Prints the extracted substring, or nothing if no balanced object is found.
extract_json() {
    awk '
        {
            line = $0
            start = 0; depth = 0; instr = 0; esc = 0
            n = length(line)
            for (i = 1; i <= n; i++) {
                c = substr(line, i, 1)
                if (start == 0) {
                    if (c == "{") { start = i; depth = 1; instr = 0; esc = 0 }
                    continue
                }
                if (instr) {
                    if (esc) { esc = 0 }
                    else if (c == "\\") { esc = 1 }
                    else if (c == "\"") { instr = 0 }
                    continue
                }
                if (c == "\"") { instr = 1; continue }
                if (c == "{") { depth++ }
                else if (c == "}") {
                    depth--
                    if (depth == 0) { print substr(line, start, i - start + 1); exit }
                }
            }
        }
    '
}

total=0
entry_matched=0
exit_matched=0
skipped_no_json=0
skipped_invalid_json=0
unrecognized=0

while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in
        *"$TARGET"*) : ;;
        *) continue ;;
    esac
    total=$((total + 1))

    kind=""
    case "$line" in
        *"$TARGET epoch_entry"*) kind=entry ;;
        *"$TARGET epoch_exit"*)  kind=exit ;;
        *) unrecognized=$((unrecognized + 1)); continue ;;
    esac

    payload=$(printf '%s' "$line" | extract_json)
    if [ -z "$payload" ]; then
        skipped_no_json=$((skipped_no_json + 1))
        continue
    fi

    compact=$(printf '%s' "$payload" | jq -c . 2>/dev/null)
    if [ -z "$compact" ]; then
        skipped_invalid_json=$((skipped_invalid_json + 1))
        continue
    fi

    if [ "$kind" = entry ]; then
        printf '%s\n' "$compact" >> "$ENTRY_OUT"
        entry_matched=$((entry_matched + 1))
    else
        printf '%s\n' "$compact" >> "$RESIDENCY_OUT"
        exit_matched=$((exit_matched + 1))
    fi
done < "$LOG"

echo "spec357-residency.sh -- extraction summary"
echo "  input                    : $LOG"
echo "  target                   : $TARGET"
echo "  residency-target lines   : $total"
echo "  epoch_entry  -> $ENTRY_OUT     : $entry_matched"
echo "  epoch_exit   -> $RESIDENCY_OUT : $exit_matched"
echo "  skipped, no JSON payload found on the line : $skipped_no_json"
echo "  skipped, payload present but not valid JSON: $skipped_invalid_json"
echo "  unrecognized message on a residency-target line (neither epoch_entry nor epoch_exit): $unrecognized"

exit 0
