#!/usr/bin/env bash
#
# Runs census-coverage.sh over every fixture here and compares its output,
# plus a last line exit=<status>, with the fixture's `expected` byte for byte.
#
# usage: run.sh [script]   (default: the census-coverage.sh beside this dir)
# Each fixture dir holds console.log, soak.json, args ("D I cell crashes") and
# expected. Prints PASS|FAIL <fixture> per fixture, then FIXTURES passed= failed=;
# exit status 0 only when every fixture passed.

set -u
export LC_ALL=C
HERE=$(cd "$(dirname "$0")" && pwd)
SCRIPT=${1:-$HERE/../census-coverage.sh}
pass=0 fail=0
for dir in "$HERE"/*/; do
  name=$(basename "$dir")
  read -r d i cell crashes < "$dir/args"
  actual=$(bash "$SCRIPT" "$dir/console.log" "$dir/soak.json" "$d" "$i" "$cell" "$crashes"; echo "exit=$?")
  if printf '%s\n' "$actual" | cmp -s - "$dir/expected"; then
    echo "PASS $name"; pass=$((pass + 1))
  else
    echo "FAIL $name"; printf '%s\n' "$actual" | diff "$dir/expected" - | sed 's/^/  /'
    fail=$((fail + 1))
  fi
done
echo "FIXTURES passed=$pass failed=$fail"
[ "$fail" -eq 0 ] && [ "$pass" -gt 0 ]
