#!/usr/bin/env bash
# CI gate for INVARIANTS.md — the machine-checkable layer the omnigraph precedent lacks.
#
# Checks:
#   1. Every `### TG-<DOMAIN>-<NNN>:` entry has an `**Enforcing test:**` field.
#   2. Every enforcing-test field either cites a test that still exists in the tree
#      (function name greppable under packages/server-rust/src) or carries an explicit
#      `NAKED` marker with a tracking reference (TODO-nnn / SPEC-nnn).
#   3. The NAKED count does not grow silently: it is compared against the committed
#      baseline in this script. Lowering it is celebrated (update the baseline);
#      raising it fails CI unless the baseline is consciously raised in the same PR.
#
# The gate is NOT "zero NAKED" — that would incentivize deleting hard invariants
# instead of writing hard tests. The failure mode being blocked is an invariant with
# neither a test nor an honest marker.
set -euo pipefail

cd "$(dirname "$0")/.."
DOC="INVARIANTS.md"
NAKED_BASELINE=6

[ -f "$DOC" ] || { echo "FAIL: $DOC missing"; exit 1; }

fail=0
naked=0
entries=0

# Split the doc into entries and inspect each.
ids=$(grep -E '^### TG-[A-Z]+-[0-9]{3}' "$DOC" | sed -E 's/^### (TG-[A-Z]+-[0-9]{3}).*/\1/')
for id in $ids; do
  entries=$((entries + 1))
  # The entry body = lines from this heading to the next heading/EOF.
  body=$(awk -v id="### $id" '
    $0 ~ "^" id { grab=1 }
    grab && $0 ~ /^### TG-/ && $0 !~ "^" id { exit }
    grab { print }' "$DOC")

  enforcing=$(printf '%s\n' "$body" | grep -A3 '\*\*Enforcing test:\*\*' || true)
  if [ -z "$enforcing" ]; then
    echo "FAIL: $id has no 'Enforcing test:' field"
    fail=1
    continue
  fi

  if printf '%s' "$enforcing" | grep -q 'NAKED'; then
    naked=$((naked + 1))
    if ! printf '%s' "$enforcing" | grep -qE '(TODO|SPEC)-[0-9]+'; then
      echo "FAIL: $id is NAKED without a tracking TODO/SPEC reference"
      fail=1
    fi
    continue
  fi

  # Extract candidate test function names (snake_case identifiers of length >= 12
  # that look like test fns) and verify at least one still exists in the tree.
  fns=$(printf '%s' "$enforcing" | grep -oE '[a-z][a-z0-9_]{11,}' | sort -u || true)
  found=""
  for fn in $fns; do
    if grep -rq "fn $fn" packages/server-rust/src packages/server-rust/benches 2>/dev/null; then
      found="$fn"
      break
    fi
  done
  # Fall back: a cited FILE that exists also counts (whole-file oracles like proptests).
  if [ -z "$found" ]; then
    files=$(printf '%s' "$enforcing" | grep -oE '[a-zA-Z0-9_/.-]+\.rs' | sort -u || true)
    for f in $files; do
      base=$(basename "$f")
      if find packages/server-rust -name "$base" | grep -q .; then
        found="$base"
        break
      fi
    done
  fi
  if [ -z "$found" ]; then
    echo "FAIL: $id cites an enforcing test, but no cited test fn/file exists in the tree"
    fail=1
  fi
done

echo "invariants: $entries entries, $naked NAKED (baseline $NAKED_BASELINE)"
# The baseline is an exact-match ratchet, BOTH directions. Growth is the obvious failure;
# an un-lowered baseline after a NAKED closure is the subtle one — it leaves silent headroom
# a later PR could spend on a new naked entry without tripping the gate.
if [ "$naked" -ne "$NAKED_BASELINE" ]; then
  if [ "$naked" -gt "$NAKED_BASELINE" ]; then
    echo "FAIL: NAKED count grew ($naked > $NAKED_BASELINE). Add an enforcing test or"
    echo "      consciously raise NAKED_BASELINE in scripts/check-invariants.sh in this PR."
  else
    echo "FAIL: NAKED count dropped to $naked but NAKED_BASELINE still says $NAKED_BASELINE."
    echo "      Lower NAKED_BASELINE in this PR to lock in the progress (ratchet, not a cap)."
  fi
  fail=1
fi

exit $fail
