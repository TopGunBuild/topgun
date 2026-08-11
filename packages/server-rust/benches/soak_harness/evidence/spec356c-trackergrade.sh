#!/bin/sh
# spec356c-trackergrade.sh -- THE TRACKER-DISCIPLINE CONTENT GRADER (R0.5, AC15, checklist item 16).
#
# usage: spec356c-trackergrade.sh <pre-edit-fixture> <todos-dir> [reference-file]
#
#   real run (wave 5) : spec356c-trackergrade.sh <scratch>/TODO-634.pre.md .specflow/todos
#   proof runs        : spec356c-trackergrade.sh <scratch>/TODO-634.pre.md <scratch>/case-N
#
#   <pre-edit-fixture>  a copy of .specflow/todos/TODO-634.md at its PRE-DATA bytes. Its own
#                       digest is checked against the reference BEFORE anything is graded --
#                       an unprovenanced fixture grounds nothing, so a mismatch REFUSES the
#                       grade (exit 2) rather than producing a verdict.
#   <todos-dir>         the directory holding the four tracker files to grade. The real run
#                       points at .specflow/todos; a proof run points at a scratch copy. This
#                       argument exists so limb (a) can be shown RED on a genuinely broken
#                       input WITHOUT ever touching a file under .specflow/.
#   [reference-file]    defaults to spec356c-trackergrade.ref beside this script.
#
# WHY THIS EXISTS AT ALL. `.specflow/` is gitignored at .gitignore:111 and no path under it is in
# the index, so `git diff <PIN>..HEAD -- '.specflow/todos/'` is EMPTY FOR EVERY POSSIBLE EXECUTION
# and a checklist item resting on it is vacuously GREEN -- PD-8's own defect ("a checklist that
# cannot fail on an ABSENT table") reproduced inside the checklist authored to replace it. Tracking
# `.specflow/` in git is barred by standing project policy and is NOT the remedy. Reading the files
# and asserting their CONTENT is. That is this script.
#
# THIS SCRIPT IS READ-ONLY. It opens every input for reading, writes nothing anywhere, and creates
# no temporary file outside $TMPDIR. It decides nothing about the round: it emits no predicate term,
# reads no threshold and touches no measurement. It grades tracker discipline and nothing else.
#
# WHY THE REFERENCE VALUES LIVE IN A COMMITTED FILE RATHER THAN IN ARGUMENTS. The values being
# compared against are the whole substance of the check, so if they were typed at the keyboard for
# each run the grade would be exactly as strong as the operator's memory -- an assertion, not a
# measurement, which is the failure mode R0.5 exists to refuse. A committed .ref file is a single
# artifact with its own digest: it is pinnable verbatim in the manifest's §10.0 ledger, it is the
# same bytes for the proof runs and for the real run, and a later edit to it shows up as a repo
# diff instead of vanishing into an invocation line. The file path is still overridable (third
# argument) so the grader itself can be exercised, but the default is the committed one.
#
# THE FIVE LIMBS.
#   (a) UNRELATED FILES UNMOVED. The three digests equal the recorded triple byte for byte. Any
#       change => RED, NAMING the file. A missing file is also RED, named.
#   (b) THE REQUIRED EDIT HAPPENED. TODO-634.md's digest DIFFERS from the recorded pre-value. An
#       UNCHANGED digest is RED, not a pass -- a checklist that greens on "nothing happened" is
#       precisely the defect this grader replaces.
#   (c) NOTHING WAS TICKED AND NO BOX MOVED. The tick counts are still 2 and 5, the strict box
#       anchor still matches as a line PREFIX, and the box census (7 top-level, 0 indented) still
#       holds so that those two counts remain exhaustive over the file's box set.
#   (d) THE EDIT IS THE RIGHT EDIT. The §8.1 box's after-text carries an outcome token and a
#       literal `spec356-manifest.md §10` pointer. Both before-text and after-text are emitted so
#       §10.x can publish them side by side.
#   (e) THE DELTA IS CONSTRAINED. Every line the diff changes -- deleted from the fixture or added
#       to the candidate -- falls INSIDE the §8.1 box. This is the limb that expresses AC15's
#       "every other box in that file is unchanged", which (a)-(d) cannot: (a) reads other files,
#       (b) is satisfied by ANY edit, (c) catches a tick elsewhere but not a text change, and (d)
#       inspects the §8.1 box only.
#
# TWO HONEST LIMITS OF THIS INSTRUMENT, STATED HERE RATHER THAN DISCOVERED LATER.
#   1. Limb (d)'s outcome-token sub-check is satisfied by the PRE-text as well: the unedited box
#      already contains the word INDETERMINATE (in "If the repeat is STILL INDETERMINATE"). So on
#      an unedited input, (d)'s discrimination rests entirely on the missing `spec356-manifest.md
#      §10` pointer. The grader therefore ALSO reports, as evidence and not as a verdict term,
#      whether each of the two literals first appears on a line ADDED relative to the fixture.
#      That report is what a reader should look at before believing a green (d).
#   2. Limb (e) is VACUOUSLY green when the diff is empty. That is deliberate and not a hole: the
#      "nothing happened" state is limb (b)'s job, and (b) REDs on it. (e) constrains a delta that
#      exists; it does not assert that one does.
#
# BOX LOCATION vs. BOX ANCHOR -- deliberately two different tests. Limb (c) tests the STRICT anchor
# (`- [ ] **NEW, ...REPEAT:` as a line prefix), which is what breaks the moment the box is ticked.
# Locating the box for limbs (d) and (e) uses the LOOSE anchor (the same text without the checkbox
# marker, matched anywhere in the line), so that a ticked box still REDs at (c) while (d) and (e)
# stay computable. A limb that fails must not blind the other limbs; that is how one defect ends up
# reported as one finding instead of three.
#
# EXIT: 0 = all five limbs GREEN. 1 = at least one limb RED. 2 = REFUSED (bad usage, unreadable
# reference file, or an unprovenanced fixture) -- a refusal is NOT a verdict and must not be read
# as one.

if [ $# -lt 2 ] || [ $# -gt 3 ]; then
    echo "usage: $0 <pre-edit-fixture> <todos-dir> [reference-file]" >&2
    exit 2
fi

FIXTURE=$1
TODOS=$2
REF=${3:-$(dirname "$0")/spec356c-trackergrade.ref}

[ -f "$FIXTURE" ] || { echo "REFUSED: fixture not readable: $FIXTURE" >&2; exit 2; }
[ -d "$TODOS" ]   || { echo "REFUSED: todos dir not a directory: $TODOS" >&2; exit 2; }
[ -f "$REF" ]     || { echo "REFUSED: reference file not readable: $REF" >&2; exit 2; }

REF_634=; REF_637=; REF_638=; REF_648=
REF_TICKED=; REF_UNTICKED=; REF_TOTAL=; REF_INDENTED=
REF_ANCHOR=; REF_LOOSE=; REF_POINTER=; REF_OUTCOMES=

while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in '' | '#'*) continue ;; esac
    key=${line%% *}
    val=${line#* }
    case "$key" in
        todo634_pre_sha256) REF_634=$val ;;
        todo637_sha256)     REF_637=$val ;;
        todo638_sha256)     REF_638=$val ;;
        todo648_sha256)     REF_648=$val ;;
        ticked_count)       REF_TICKED=$val ;;
        unticked_count)     REF_UNTICKED=$val ;;
        total_boxes)        REF_TOTAL=$val ;;
        indented_boxes)     REF_INDENTED=$val ;;
        box_anchor)         REF_ANCHOR=$val ;;
        box_anchor_loose)   REF_LOOSE=$val ;;
        pointer_literal)    REF_POINTER=$val ;;
        outcome_tokens)     REF_OUTCOMES=$val ;;
        *) echo "REFUSED: unknown key in reference file: $key" >&2; exit 2 ;;
    esac
done < "$REF"

for v in REF_634 REF_637 REF_638 REF_648 REF_TICKED REF_UNTICKED REF_TOTAL \
         REF_INDENTED REF_ANCHOR REF_LOOSE REF_POINTER REF_OUTCOMES; do
    eval "set -- \"\$$v\""
    [ -n "$1" ] || { echo "REFUSED: reference file is missing $v" >&2; exit 2; }
done

sha() { shasum -a 256 "$1" | awk '{print $1}'; }

# Range of the §8.1 box in FILE: the loose-anchor line, plus every following line that is indented
# (a list-item continuation), tolerating blank lines INSIDE the item and trimming trailing blanks.
# Prints "<start> <end>", or "0 0" when the loose anchor is absent.
box_range() {
    awk -v anchor="$2" '
        start == 0 && index($0, anchor) > 0 { start = NR; next }
        start > 0 && stop == 0 {
            if ($0 ~ /^[ \t]*$/) { blanks++; next }
            if ($0 ~ /^[ \t]/)   { blanks = 0; next }
            stop = NR - 1 - blanks
        }
        END {
            if (start > 0 && stop == 0) stop = NR - blanks
            print start " " stop
        }
    ' "$1"
}

RED_LIMBS=

echo "================================================================================"
echo "spec356c-trackergrade.sh -- tracker-discipline CONTENT grade (R0.5 / AC15 / item 16)"
echo "================================================================================"
echo "fixture   : $FIXTURE"
echo "todos dir : $TODOS"
echo "reference : $REF"
echo "date      : $(date -u '+%Y-%m-%dT%H:%M:%SZ')"
echo

# ---- fixture provenance (a REFUSAL, not a limb) -------------------------------------------------
FIX_SHA=$(sha "$FIXTURE")
echo "FIXTURE PROVENANCE (checked before any limb; a mismatch REFUSES the grade)"
echo "  recorded TODO-634.md pre-digest : $REF_634"
echo "  fixture digest                  : $FIX_SHA"
if [ "$FIX_SHA" != "$REF_634" ]; then
    echo "  RESULT : REFUSED -- the fixture is not TODO-634.md at its recorded PRE-DATA bytes."
    echo
    echo "VERDICT: REFUSED (no limb was graded; this is NOT a RED and NOT a GREEN)"
    exit 2
fi
echo "  RESULT : OK -- the fixture is provenanced."
echo

CAND=$TODOS/TODO-634.md
[ -f "$CAND" ] || { echo "REFUSED: $CAND not readable" >&2; exit 2; }

# ---- limb (a) -----------------------------------------------------------------------------------
echo "--------------------------------------------------------------------------------"
echo "LIMB (a) UNRELATED FILES UNMOVED"
echo "--------------------------------------------------------------------------------"
a_red=0
for pair in "TODO-637.md $REF_637" "TODO-638.md $REF_638" "TODO-648.md $REF_648"; do
    f=${pair%% *}
    want=${pair#* }
    if [ ! -f "$TODOS/$f" ]; then
        echo "  $f : MISSING  expected $want  => RED (file $f is absent)"
        a_red=1
        continue
    fi
    got=$(sha "$TODOS/$f")
    if [ "$got" = "$want" ]; then
        echo "  $f : MATCH    $got"
    else
        echo "  $f : MOVED    expected $want"
        echo "  $f :          observed $got  => RED (file $f changed)"
        a_red=1
    fi
done
if [ "$a_red" -eq 0 ]; then echo "  (a) GREEN"; else echo "  (a) RED"; RED_LIMBS="$RED_LIMBS a"; fi
echo

# ---- limb (b) -----------------------------------------------------------------------------------
echo "--------------------------------------------------------------------------------"
echo "LIMB (b) THE REQUIRED EDIT HAPPENED"
echo "--------------------------------------------------------------------------------"
CAND_SHA=$(sha "$CAND")
echo "  recorded pre-digest : $REF_634"
echo "  observed digest     : $CAND_SHA"
if [ "$CAND_SHA" = "$REF_634" ]; then
    echo "  the digest has NOT moved -- the required edit never happened."
    echo "  (b) RED"
    RED_LIMBS="$RED_LIMBS b"
else
    echo "  the digest HAS moved."
    echo "  (b) GREEN"
fi
echo

# ---- limb (c) -----------------------------------------------------------------------------------
echo "--------------------------------------------------------------------------------"
echo "LIMB (c) NOTHING WAS TICKED AND NO BOX MOVED"
echo "--------------------------------------------------------------------------------"
n_ticked=$(grep -c '^- \[x\] ' "$CAND" || true)
n_unticked=$(grep -c '^- \[ \] ' "$CAND" || true)
n_total=$(grep -c '^- \[[ x]\] ' "$CAND" || true)
n_indent=$(grep -c '^[ 	][ 	]*[-*+] \[' "$CAND" || true)
c_red=0
echo "  ticked   '^- \[x\] ' : expected $REF_TICKED   observed $n_ticked"
[ "$n_ticked" = "$REF_TICKED" ] || { echo "    => RED (a box was ticked or un-ticked)"; c_red=1; }
echo "  unticked '^- \[ \] ' : expected $REF_UNTICKED   observed $n_unticked"
[ "$n_unticked" = "$REF_UNTICKED" ] || { echo "    => RED (the unticked box count moved)"; c_red=1; }
echo "  census, top-level boxes  : expected $REF_TOTAL   observed $n_total"
[ "$n_total" = "$REF_TOTAL" ] || { echo "    => RED (a box was added or removed)"; c_red=1; }
echo "  census, indented boxes   : expected $REF_INDENTED   observed $n_indent"
[ "$n_indent" = "$REF_INDENTED" ] || { echo "    => RED (an indented box appeared; the two counts above are no longer exhaustive)"; c_red=1; }
echo "  (the census is what makes the two counts exhaustive over the file's box set)"
# -e is mandatory: the anchor begins with "- " and grep would otherwise parse it as an option.
if grep -qF -e "$REF_ANCHOR" "$CAND"; then
    echo "  strict box anchor        : MATCHES"
else
    echo "  strict box anchor        : ABSENT  => RED (the §8.1 box was ticked, moved or rewritten)"
    echo "    anchor: $REF_ANCHOR"
    c_red=1
fi
if [ "$c_red" -eq 0 ]; then echo "  (c) GREEN"; else echo "  (c) RED"; RED_LIMBS="$RED_LIMBS c"; fi
echo

# ---- box extraction (shared by (d) and (e)) -----------------------------------------------------
set -- $(box_range "$FIXTURE" "$REF_LOOSE"); fb_start=$1; fb_end=$2
set -- $(box_range "$CAND" "$REF_LOOSE");    cb_start=$1; cb_end=$2

echo "--------------------------------------------------------------------------------"
echo "LIMB (d) THE EDIT IS THE RIGHT EDIT"
echo "--------------------------------------------------------------------------------"
echo "  §8.1 box located by the LOOSE anchor (survives a tick, so a broken (c) does not blind (d)/(e))"
echo "  fixture box lines   : $fb_start-$fb_end"
echo "  candidate box lines : $cb_start-$cb_end"
d_red=0
if [ "$fb_start" -eq 0 ]; then
    echo "  => RED (the §8.1 box cannot be located in the FIXTURE; the fixture is unusable)"
    d_red=1
fi
if [ "$cb_start" -eq 0 ]; then
    echo "  => RED (the §8.1 box cannot be located in the CANDIDATE; it was removed or rewritten)"
    d_red=1
fi

if [ "$fb_start" -gt 0 ]; then
    echo
    echo "  ---- BEGIN §8.1 BOX BEFORE-TEXT (fixture, lines $fb_start-$fb_end) ----"
    sed -n "${fb_start},${fb_end}p" "$FIXTURE" | sed 's/^/  | /'
    echo "  ---- END §8.1 BOX BEFORE-TEXT ----"
fi
if [ "$cb_start" -gt 0 ]; then
    echo
    echo "  ---- BEGIN §8.1 BOX AFTER-TEXT (candidate, lines $cb_start-$cb_end) ----"
    sed -n "${cb_start},${cb_end}p" "$CAND" | sed 's/^/  | /'
    echo "  ---- END §8.1 BOX AFTER-TEXT ----"
fi
echo

if [ "$cb_start" -gt 0 ]; then
    # Backticks are stripped before matching. The pointer is a POINTER, not a rendering: an editor
    # who writes the natural markdown `spec356-manifest.md` §10 means exactly what an editor who
    # writes spec356-manifest.md §10 means, and a grader that REDs on the difference is REDing on
    # code-span formatting rather than on tracker discipline. Same normalisation on both sides.
    after=$(sed -n "${cb_start},${cb_end}p" "$CAND" | tr -d '`')
    before=
    [ "$fb_start" -gt 0 ] && before=$(sed -n "${fb_start},${fb_end}p" "$FIXTURE" | tr -d '`')

    found_tok=
    for tok in $REF_OUTCOMES; do
        case "$after" in *"$tok"*) found_tok="${found_tok:+$found_tok }$tok" ;; esac
    done
    if [ -n "$found_tok" ]; then
        echo "  outcome token in after-text   : PRESENT ($found_tok)"
    else
        echo "  outcome token in after-text   : ABSENT  => RED (no outcome recorded)"
        echo "    accepted tokens: $REF_OUTCOMES"
        d_red=1
    fi
    ptr_present=0
    case "$after" in
        *"$REF_POINTER"*) echo "  pointer literal in after-text : PRESENT ('$REF_POINTER', backticks ignored)"; ptr_present=1 ;;
        *) echo "  pointer literal in after-text : ABSENT  => RED (no '$REF_POINTER' pointer)"; d_red=1 ;;
    esac

    # EVIDENCE, NOT A VERDICT TERM. The outcome-token check above is satisfied by the PRE-text too
    # (the unedited box already says "If the repeat is STILL INDETERMINATE"), so a reader who wants
    # to know whether the edit actually recorded something new needs these lines, not that one.
    if [ "$fb_start" -gt 0 ]; then
        for tok in $found_tok; do
            case "$before" in
                *"$tok"*) echo "  [evidence] outcome token '$tok' was ALREADY in the before-text" ;;
                *) echo "  [evidence] outcome token '$tok' is NEW in the after-text" ;;
            esac
        done
        if [ "$ptr_present" -eq 1 ]; then
            case "$before" in
                *"$REF_POINTER"*) echo "  [evidence] pointer literal was ALREADY in the before-text" ;;
                *) echo "  [evidence] pointer literal is NEW in the after-text" ;;
            esac
        fi
    fi
fi
if [ "$d_red" -eq 0 ]; then echo "  (d) GREEN"; else echo "  (d) RED"; RED_LIMBS="$RED_LIMBS d"; fi
echo

# ---- limb (e) -----------------------------------------------------------------------------------
echo "--------------------------------------------------------------------------------"
echo "LIMB (e) THE DELTA IS CONSTRAINED -- EVERY CHANGED LINE IS INSIDE THE §8.1 BOX"
echo "--------------------------------------------------------------------------------"
echo "  invocation: diff -U0 <fixture> <candidate>"
echo "  fixture box $fb_start-$fb_end   candidate box $cb_start-$cb_end"
e_red=0
if [ "$fb_start" -eq 0 ] || [ "$cb_start" -eq 0 ]; then
    echo "  => RED (a box range is unknown, so the delta cannot be constrained)"
    e_red=1
else
    hunks=$(diff -U0 "$FIXTURE" "$CAND" | grep '^@@' || true)
    if [ -z "$hunks" ]; then
        echo "  0 hunks -- the candidate is byte-identical to the fixture."
        echo "  (e) is VACUOUSLY green here; 'nothing happened' is limb (b)'s finding, not (e)'s."
    else
        echo "  hunks:"
        printf '%s\n' "$hunks" | awk -v fs="$fb_start" -v fe="$fb_end" -v cs="$cb_start" -v ce="$cb_end" '
            {
                hdr = $0
                sub(/^@@ -/, "", hdr); sub(/ @@.*$/, "", hdr)
                split(hdr, parts, " +\\+")
                split(parts[1], o, ",")
                split(parts[2], n, ",")
                ol = o[1] + 0; os = (2 in o) ? o[2] + 0 : 1
                nl = n[1] + 0; ns = (2 in n) ? n[2] + 0 : 1
                bad = ""
                if (os > 0 && (ol < fs || ol + os - 1 > fe))
                    bad = bad sprintf("      deleted lines %d-%d are OUTSIDE the fixture box %d-%d\n", ol, ol + os - 1, fs, fe)
                if (ns > 0 && (nl < cs || nl + ns - 1 > ce))
                    bad = bad sprintf("      added lines %d-%d are OUTSIDE the candidate box %d-%d\n", nl, nl + ns - 1, cs, ce)
                printf "    %s  =>  %s\n", $0, (bad == "" ? "inside the §8.1 box" : "OUTSIDE THE §8.1 BOX")
                if (bad != "") printf "%s", bad
            }'
        outside=$(printf '%s\n' "$hunks" | awk -v fs="$fb_start" -v fe="$fb_end" -v cs="$cb_start" -v ce="$cb_end" '
            {
                hdr = $0
                sub(/^@@ -/, "", hdr); sub(/ @@.*$/, "", hdr)
                split(hdr, parts, " +\\+")
                split(parts[1], o, ",")
                split(parts[2], n, ",")
                ol = o[1] + 0; os = (2 in o) ? o[2] + 0 : 1
                nl = n[1] + 0; ns = (2 in n) ? n[2] + 0 : 1
                if (os > 0 && (ol < fs || ol + os - 1 > fe)) c++
                else if (ns > 0 && (nl < cs || nl + ns - 1 > ce)) c++
            }
            END { print c + 0 }')
        echo "  hunks outside the §8.1 box: $outside"
        [ "$outside" -eq 0 ] || e_red=1
    fi
fi
if [ "$e_red" -eq 0 ]; then echo "  (e) GREEN"; else echo "  (e) RED"; RED_LIMBS="$RED_LIMBS e"; fi
echo

echo "================================================================================"
if [ -z "$RED_LIMBS" ]; then
    echo "VERDICT: GREEN (a, b, c, d, e)"
    echo "================================================================================"
    exit 0
fi
echo "VERDICT: RED (red limbs:$RED_LIMBS)"
echo "================================================================================"
exit 1
