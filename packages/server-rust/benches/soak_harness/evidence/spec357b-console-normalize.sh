#!/usr/bin/env bash
# spec357b-console-normalize.sh -- THE TRANSPORT ADAPTER (computes no predicate term).
#
# usage: spec357b-console-normalize.sh <raw server-console.log> <normalized output>
#
# WHY THIS EXISTS. `spec357-residency.sh` (pinned, NOT edited by this half) scans each
# residency-target line for the first balanced `{...}` and treats it as the record payload.
# The emitter it was written against renders ONE TRACING FIELD PER STRUCT FIELD instead --
# `kind="epoch_entry" epoch=1 entered_index=false ...` -- with no JSON blob anywhere on the
# line, and the captured stream additionally carries ANSI colour codes that split even the
# module target from its trailing colon. The pinned extractor therefore matches ZERO lines and
# reports every one as `unrecognized`, which is CORRECT behaviour on its part: its own header
# states it will report such a payload "rather than guessed at", because inventing field values
# from a non-JSON rendering would be exactly the predicate-term computation it forbids itself.
#
# This adapter closes the TRANSPORT gap and nothing else. It performs two mechanical
# transformations -- strip ANSI, render the already-present key=value fields as a JSON object --
# and then the PINNED extractor runs UNCHANGED over the result.
#
# WHAT IT DOES NOT DO, and these are load-bearing:
#   * It computes no predicate term, no class, no threshold and no share.
#   * It invents no field. Every key and every value on an output line is present on its input
#     line; nothing is defaulted, inferred from a neighbour, or carried across lines.
#   * It drops no record. Every target line produces exactly one output line, or the run FAILS.
#   * It chooses no key name. `snake_case -> camelCase` is applied uniformly and mechanically,
#     which is what `#[serde(rename_all = "camelCase")]` on the record structs already specifies,
#     and it reproduces the committed reference rendering the Tier-1 harness itself emitted
#     (see `spec357-tier1-transcript.txt`, "TIER-1 HARNESS: entry.jsonl / residency.jsonl").
#   * It guesses no type. The value grammar below is CLOSED; an unrecognised value shape is a
#     FATAL error naming the line, never a coerced string.
#
# VALUE GRAMMAR (closed -- anything else is fatal):
#   key="…"        JSON string   (tracing renders string fields quoted; escapes honoured)
#   key=123        JSON number   (integer-typed struct fields)
#   key=-123       JSON number   (i64 fields, e.g. observed_refs_delta)
#   key=true|false JSON boolean  (bool fields, e.g. entered_index)
#   key=Some(123)  JSON number   (Option<u64> rendered by `?` Debug)
#   key=None       KEY OMITTED   (matches `#[serde(skip_serializing_if = "Option::is_none")]`
#                                 on those fields -- the committed Tier-1 reference likewise
#                                 omits the key rather than emitting null)
#   key=Ident      JSON string, camelCased  (a unit enum variant under the enum's own
#                                 `#[serde(rename_all = "camelCase")]`, e.g.
#                                 DrainedByPrune -> "drainedByPrune")
#
# DELIBERATELY NOT SUPPORTED -- a struct-shaped enum variant, i.e. `exit_kind=Unclassified { … }`
# in Rust Debug form. Rendering that into the nested JSON its serde form produces would be a
# genuine semantic translation, not a transport one, and this adapter refuses it: the run FAILS
# and names the line. No such value occurs in either committed cell (every exit in both is
# `DrainedByPrune`), so the refusal costs no record here -- but it must stay a refusal, because
# the day it does occur is exactly the day `INDETERMINATE-RESIDUAL` turns on it.
#
# EXIT: 0 on a clean pass. Non-zero, with the offending line named, on ANY unparseable field.
# Non-zero ALSO when no residency-target line is seen at all, or when fewer lines are emitted
# than were seen -- the "drops no record" guarantee above is an assertion, not a comment, so a
# silently-empty or silently-short ledger is a FAILURE rather than a clean pass. Without that
# check a transport regression upstream (an ANSI strip that stops matching, a renamed target)
# would hand the pinned extractor an empty file and report success.
# A partial output file is never left behind on failure.
set -euo pipefail

RAW="${1:?usage: $0 <raw server-console.log> <normalized output>}"
OUT="${2:?usage: $0 <raw server-console.log> <normalized output>}"
TARGET='topgun_server::tombstone_frontier::residency'

[ -r "$RAW" ] || { echo "FATAL: cannot read '$RAW'" >&2; exit 2; }

TMP="$(mktemp)"
trap 'rm -f "$TMP"' EXIT

# Strip ANSI SGR sequences, then rebuild each residency line as
#   <prefix> <target>: <epoch_entry|epoch_exit> record={…}
# which is the exact shape the pinned extractor's own header documents.
sed 's/\x1b\[[0-9;]*m//g' "$RAW" | awk -v target="$TARGET" '
    function fail(msg, line) {
        printf("FATAL: %s\n  line: %s\n", msg, line) > "/dev/stderr"
        exit_code = 1
        exit 1
    }
    # snake_case -> camelCase, applied uniformly: no per-field table, so no per-field decision.
    function camel(s,   out, i, c, up) {
        out = ""; up = 0
        for (i = 1; i <= length(s); i++) {
            c = substr(s, i, 1)
            if (c == "_") { up = 1; continue }
            out = out (up ? toupper(c) : c)
            up = 0
        }
        return out
    }
    # Lowercase the first character only: DrainedByPrune -> drainedByPrune, matching the
    # enum'"'"'s own #[serde(rename_all = "camelCase")].
    function lower_first(s) { return tolower(substr(s, 1, 1)) substr(s, 2) }

    index($0, target ":") == 0 { next }
    {
        seen++
        line = $0
        # Field list begins at the first `kind=` token; everything before it is tracing'"'"'s own
        # timestamp/level/target/message prefix and carries no record field.
        fpos = index(line, "kind=\"epoch_")
        if (fpos == 0) fail("residency-target line carries no kind=\"epoch_*\" field", line)

        fields = substr(line, fpos)
        n = length(fields)
        json = ""; nkeys = 0; kind = ""
        i = 1
        while (i <= n) {
            while (i <= n && substr(fields, i, 1) == " ") i++
            if (i > n) break
            eq = index(substr(fields, i), "=")
            if (eq == 0) fail("trailing token with no '"'"'='"'"'", line)
            key = substr(fields, i, eq - 1)
            i = i + eq                      # first char of the value

            c = substr(fields, i, 1)
            if (c == "\"") {                # quoted string: honour backslash escapes
                val = ""; i++
                while (i <= n) {
                    c = substr(fields, i, 1)
                    if (c == "\\") { val = val c substr(fields, i + 1, 1); i += 2; continue }
                    if (c == "\"") { i++; break }
                    val = val c; i++
                }
                out_val = "\"" val "\""
            } else {                        # bare token, ends at the next space
                sp = index(substr(fields, i), " ")
                raw = (sp == 0) ? substr(fields, i) : substr(fields, i, sp - 1)
                i = (sp == 0) ? n + 1 : i + sp

                if (raw ~ /^-?[0-9]+$/)            out_val = raw
                else if (raw == "true" || raw == "false") out_val = raw
                else if (raw ~ /^Some\([-0-9]+\)$/) { out_val = raw; sub(/^Some\(/, "", out_val); sub(/\)$/, "", out_val) }
                else if (raw == "None")            { continue }   # serde omits the key entirely
                else if (raw ~ /^[A-Z][A-Za-z0-9]*$/) out_val = "\"" lower_first(raw) "\""
                else fail("unparseable value for key '"'"'" key "'"'"': <" raw ">", line)
            }

            if (key == "kind") { kind = out_val; gsub(/"/, "", kind); continue }
            json = json (nkeys++ ? "," : "") "\"" camel(key) "\":" out_val
        }
        if (kind != "epoch_entry" && kind != "epoch_exit") fail("kind is neither epoch_entry nor epoch_exit: <" kind ">", line)
        if (nkeys == 0) fail("no record fields recovered", line)

        printf("%s: %s record={%s}\n", target, kind, json)
        emitted++
    }
    END {
        if (exit_code) exit 1
        if (seen == 0)
            fail("no residency-target line was seen at all -- ANSI strip or target match failed", target ":")
        if (emitted != seen)
            fail(sprintf("dropped %d of %d residency-target lines", seen - emitted, seen), target ":")
        printf("spec357b-console-normalize.sh: %d residency records normalized\n", emitted) > "/dev/stderr"
    }
' > "$TMP"

mv "$TMP" "$OUT"
trap - EXIT
