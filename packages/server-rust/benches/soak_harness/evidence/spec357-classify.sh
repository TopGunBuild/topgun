#!/bin/sh
# spec357-classify.sh -- THE SINGLE CLASSIFIER BUILDER.
# usage: spec357-classify.sh <entry.jsonl> <residency.jsonl> <prune.csv> <output-prefix>
#   writes <output-prefix>-class.csv         one row per entry-ledger epoch, R3/R4's six-class walk
#          <output-prefix>-conservation.csv  one row per prune.csv scrape, O-0's identity + tear class
#
# <entry.jsonl> / <residency.jsonl> may be /dev/null or an empty file (no per-epoch ledger available
# yet -- e.g. the pre-instrument dry-run leg 1 subject). <prune.csv> may likewise be absent the new
# O-0 counters (again, leg 1's subject). This builder APPLIES the frozen classification walk and the
# frozen O-0 identity; it DECIDES NOTHING BEYOND THEM -- every threshold, window and reading below is
# a literal transcription of the manifest's frozen §11.0 authoring, not a choice made at this
# keyboard. Column names are the camelCase serde rendering the two record types carry
# (`#[serde(rename_all = "camelCase")]`), because that is what a JSON-serialized record emits.
#
# EXIT: 0 on a clean build (including the honest-absence branches below, which are not failures).
# Non-zero only on bad usage, an unreadable/missing REQUIRED input, or a jq/awk internal failure.

if [ $# -ne 4 ]; then
    echo "usage: $0 <entry.jsonl> <residency.jsonl> <prune.csv> <output-prefix>" >&2
    exit 2
fi

ENTRY=$1
RESIDENCY=$2
PRUNE=$3
PREFIX=$4
CLASS_OUT="${PREFIX}-class.csv"
CONS_OUT="${PREFIX}-conservation.csv"

command -v jq >/dev/null 2>&1 || { echo "REFUSED: jq is required and was not found on PATH" >&2; exit 2; }
[ -f "$PRUNE" ] || { echo "REFUSED: $PRUNE not readable" >&2; exit 2; }

is_nonempty() { [ -f "$1" ] && [ -s "$1" ]; }

# ==================================================================================================
# PART 1 -- THE CLASSIFICATION WALK (R4.2's Steps 1-3.5, over the ENTRY x RESIDENCY join).
# ==================================================================================================
# Absence is honest, not an error: an empty (or absent) entry ledger means the deciding population P
# is empty by construction, and R4.4's INDETERMINATE-EMPTY limb is what an empty P endorses -- not a
# builder refusal. The CSV is still produced, header-complete, zero data rows.
if ! is_nonempty "$ENTRY"; then
    echo "epoch,entered_index,stamped_bytes,refs_at_entry,has_exit_row,exit_kind_variant,unclassified_exit,class_a,class_b,class_c,class_d,class_e,class_f" > "$CLASS_OUT"
    echo "spec357-classify.sh: entry ledger absent or empty -- P is empty; wrote header-only $CLASS_OUT (endorses INDETERMINATE-EMPTY, R4.4)"
else
    RES_ARG=/dev/null
    is_nonempty "$RESIDENCY" && RES_ARG=$RESIDENCY
    # jq does the join and the walk in one pass: entry rows are the population P; residency rows are
    # keyed by epoch onto them (an epoch appears at most once in each ledger, per R2.3a / R2.3b's
    # "once per epoch" emission rule). The classification itself is §11.0.7's P0/P1/P2 split, plus, on
    # P2, the T(e)/D(e) predicate from §11.0.3 -- reproduced here as literal jq, not re-derived.
    jq -n -r --slurpfile entries "$ENTRY" \
        --argjson has_res "$( [ "$RES_ARG" != /dev/null ] && echo true || echo false )" \
        --slurpfile residency_arr "$RES_ARG" '
      # --slurpfile already yields a FLAT array of the top-level JSON values in each file (one per
      # JSONL line), so $entries / $residency_arr are arrays of records, never wrapped a level deeper.
      def residency_map:
        if $has_res then (reduce $residency_arr[] as $r ({}; .[$r.epoch|tostring] = $r))
        else {} end;
      residency_map as $rmap
      | $entries[] as $entry
      | ($rmap[($entry.epoch|tostring)]) as $r
      | {
          epoch: $entry.epoch,
          entered_index: $entry.enteredIndex,
          stamped_bytes: ($entry.stampedBytes // 0),
          refs_at_entry: ($entry.refsAtEntry // 0),
          has_exit_row: ($r != null),
          exit_kind_variant: (
            if $r == null then null
            elif ($r.exitKind | type) == "string" then $r.exitKind
            else ($r.exitKind | keys[0]) end
          ),
          lwm_passed_at_op_seq: (if $r == null then null else $r.lwmPassedAtOpSeq end),
          fence_passed_at_op_seq: (if $r == null then null else $r.fencePassedAtOpSeq end),
          entered_at_op_seq: (if $r == null then null else $r.enteredAtOpSeq end),
          exited_at_op_seq: (if $r == null then null else $r.exitedAtOpSeq end)
        }
      | . + {
          # T(e): the triple-overlap window is non-empty. Both Option terms must be present (an
          # absent term contributes the empty window, per the totality argument in section 11.0.3)
          # and the max of the three lower bounds must still precede the exit -- i.e. some instant
          # was simultaneously resident, licensed and fenced.
          triple_overlap: (
            (.has_exit_row) and (.lwm_passed_at_op_seq != null) and (.fence_passed_at_op_seq != null) and
            ([.entered_at_op_seq, .lwm_passed_at_op_seq, .fence_passed_at_op_seq] | max) < .exited_at_op_seq
          ),
          drained_healthy_kind: (.exit_kind_variant == "DrainedByPrune")
        }
      | . + {
          class_a: (if (.entered_index == false) and (.stamped_bytes > 0) then 1 else 0 end),
          class_f: (if (.entered_index == false) and (.stamped_bytes <= 0) then 1 else 0 end),
          class_e: (if (.entered_index == true) and (.has_exit_row == false) then 1 else 0 end),
          class_c: (if (.entered_index == true) and (.has_exit_row == true) and (.triple_overlap == false) then 1 else 0 end),
          class_b: (if (.entered_index == true) and (.has_exit_row == true) and (.triple_overlap == true) and (.drained_healthy_kind == false) then 1 else 0 end),
          class_d: (if (.entered_index == true) and (.has_exit_row == true) and (.triple_overlap == true) and (.drained_healthy_kind == true) then 1 else 0 end),
          unclassified_exit: (if .exit_kind_variant == "Unclassified" then 1 else 0 end)
        }
      | [.epoch, .entered_index, .stamped_bytes, .refs_at_entry, .has_exit_row,
         (.exit_kind_variant // ""), .unclassified_exit,
         .class_a, .class_b, .class_c, .class_d, .class_e, .class_f]
      | @csv
    ' > /tmp/spec357_class_rows.$$.csv

    {
      echo "epoch,entered_index,stamped_bytes,refs_at_entry,has_exit_row,exit_kind_variant,unclassified_exit,class_a,class_b,class_c,class_d,class_e,class_f"
      cat /tmp/spec357_class_rows.$$.csv
    } > "$CLASS_OUT"
    rm -f /tmp/spec357_class_rows.$$.csv

    rows=$(($(wc -l < "$CLASS_OUT" | tr -d ' ') - 1))
    p0=$(awk -F, 'NR>1 && $2=="false"' "$CLASS_OUT" | wc -l | tr -d ' ')
    p1=$(awk -F, 'NR>1{print}' "$CLASS_OUT" | awk -F, '$2=="true" && $5=="false"' | wc -l | tr -d ' ')
    p2=$(awk -F, 'NR>1{print}' "$CLASS_OUT" | awk -F, '$2=="true" && $5=="true"' | wc -l | tr -d ' ')
    echo "spec357-classify.sh: wrote $CLASS_OUT -- |P|=$rows |P0|=$p0 |P1|=$p1 |P2|=$p2 (must sum to |P|)"
fi

# ==================================================================================================
# PART 2 -- O-0'S CONSERVATION LEDGER (§11.0.4), over prune.csv's scrapes, WITH THE DOUBLE-RENDER READ.
# ==================================================================================================
# The identity's eight quantities (the seven new counters plus `indexed_refs`). The runner writes
# ONLY the paired `*_a` / `*_b` families (R3.0's double render); the bare, undoubled name is not
# expected to appear in an armed cell's prune.csv at all, so PRESENCE is decided on the doubled forms
# and the bare form is never required. Three states, checked per quantity and then combined:
#   FULL      -- both `<name>_a` and `<name>_b` present, for every one of the eight quantities.
#   ABSENT    -- NEITHER the bare name NOR either doubled form is present, for at least one quantity
#                -- this subject predates the instrument entirely (R1.4's NEW_EMISSION set does not
#                exist here yet), and the header carries R6.3's exact marker, verbatim, so a reader
#                (or a mechanical grep) can tell "the builder ran and found nothing to build" from
#                "the builder crashed".
#   PARTIAL   -- at least one quantity has SOME form (bare or one side of the pair) but not a full
#                `_a`/`_b` pair -- O-0's double-read sampling rule (R3.0) cannot be exercised on a
#                single render per scrape tick, so the second marker fires on its own.
ALL_QUANTITIES="topgun_or_prune_stamped_refs_total topgun_or_prune_stamped_bytes_total topgun_or_prune_drained_refs_total topgun_or_prune_restored_refs_total topgun_or_prune_rebuild_cleared_refs_total topgun_or_prune_epochs_entered_total topgun_or_prune_epochs_exited_total topgun_or_prune_indexed_refs"

HEADER_LINE=$(head -1 "$PRUNE" | tr -d '\r')
has_col() {
    case ",$HEADER_LINE," in
        *",$1,"*) return 0 ;;
        *) return 1 ;;
    esac
}

wholly_absent=""
partial_only=""
for c in $ALL_QUANTITIES; do
    a_present=1; b_present=1; bare_present=1
    has_col "${c}_a" || a_present=0
    has_col "${c}_b" || b_present=0
    has_col "$c"     || bare_present=0
    if [ "$a_present" -eq 1 ] && [ "$b_present" -eq 1 ]; then
        : # FULL for this quantity
    elif [ "$a_present" -eq 0 ] && [ "$b_present" -eq 0 ] && [ "$bare_present" -eq 0 ]; then
        wholly_absent="$wholly_absent $c"
    else
        partial_only="$partial_only $c"
    fi
done

if [ -n "$wholly_absent" ]; then
    {
        echo "# RESIDENCY_COLUMNS_ABSENT__PRE_INSTRUMENT_SUBJECT"
        echo "#   quantities present in NEITHER bare NOR *_a/*_b form:$wholly_absent"
        echo "# SINGLE_RENDER_SUBJECT__DOUBLE_READ_NOT_EXERCISABLE"
        echo "#   this subject carries at most one /metrics render per scrape tick, so O-0's double-read"
        echo "#   sampling rule (R3.0) has no pair to evaluate even where a single-render column exists"
        echo "elapsed_secs,note"
    } > "$CONS_OUT"
    echo "spec357-classify.sh: wrote $CONS_OUT -- required O-0 columns absent from $PRUNE; header-only, both absence markers present verbatim"
elif [ -n "$partial_only" ]; then
    {
        echo "# SINGLE_RENDER_SUBJECT__DOUBLE_READ_NOT_EXERCISABLE"
        echo "#   quantities without a full *_a/*_b pair:$partial_only"
        echo "elapsed_secs,note"
    } > "$CONS_OUT"
    echo "spec357-classify.sh: wrote $CONS_OUT -- some O-0 quantities lack a full *_a/*_b render pair; header-only, absence marker present verbatim"
else
    # The full double-render identity, evaluated per scrape row. CONSISTENT iff all eight quantities
    # agree between the _a and _b renders of that row; otherwise TORN. TORN-PERSISTENT is a RUNNER
    # fact (a retried scrape that tore twice), not something this builder can see from one row, so it
    # is read from an optional `render_retry_of_torn` column when the runner's own convention supplies
    # one; its absence degrades this builder to the two-value CONSISTENT/TORN reading, reported as such
    # rather than silently fabricating a TORN-PERSISTENT count of zero.
    has_retry_col=0
    case ",$HEADER_LINE," in *",render_retry_of_torn,"*) has_retry_col=1 ;; esac
    awk -F, -v has_retry="$has_retry_col" '
      NR==1 {
        for (i=1;i<=NF;i++) h[$i]=i
        n=split("topgun_or_prune_stamped_refs_total topgun_or_prune_stamped_bytes_total topgun_or_prune_drained_refs_total topgun_or_prune_restored_refs_total topgun_or_prune_rebuild_cleared_refs_total topgun_or_prune_epochs_entered_total topgun_or_prune_epochs_exited_total topgun_or_prune_indexed_refs", q, " ")
        print "elapsed_secs,render_class,identity_lhs,identity_rhs,identity_holds"
        next
      }
      {
        consistent = 1
        for (k=1;k<=n;k++) {
          va = $(h[q[k] "_a"]) + 0
          vb = $(h[q[k] "_b"]) + 0
          if (va != vb) consistent = 0
        }
        render_class = consistent ? "CONSISTENT" : "TORN"
        if (!consistent && has_retry == 1 && $(h["render_retry_of_torn"]) == "1") render_class = "TORN-PERSISTENT"
        lhs = $(h["topgun_or_prune_stamped_refs_total_a"]) + $(h["topgun_or_prune_restored_refs_total_a"]) \
            - $(h["topgun_or_prune_drained_refs_total_a"]) - $(h["topgun_or_prune_rebuild_cleared_refs_total_a"])
        rhs = $(h["topgun_or_prune_indexed_refs_a"]) + 0
        holds = ""
        if (render_class == "CONSISTENT") holds = (lhs == rhs) ? "1" : "0"
        printf "%s,%s,%s,%s,%s\n", $(h["elapsed_secs"]), render_class, lhs, rhs, holds
      }
    ' "$PRUNE" > "$CONS_OUT"
    total=$(($(wc -l < "$CONS_OUT" | tr -d ' ') - 1))
    consistent_n=$(awk -F, 'NR>1 && $2=="CONSISTENT"' "$CONS_OUT" | wc -l | tr -d ' ')
    torn_n=$(awk -F, 'NR>1 && $2=="TORN"' "$CONS_OUT" | wc -l | tr -d ' ')
    tornp_n=$(awk -F, 'NR>1 && $2=="TORN-PERSISTENT"' "$CONS_OUT" | wc -l | tr -d ' ')
    viol_n=$(awk -F, 'NR>1 && $2=="CONSISTENT" && $5=="0"' "$CONS_OUT" | wc -l | tr -d ' ')
    echo "spec357-classify.sh: wrote $CONS_OUT -- $total scrapes: CONSISTENT=$consistent_n TORN=$torn_n TORN-PERSISTENT=$tornp_n; violations over CONSISTENT scrapes=$viol_n"
fi

exit 0
