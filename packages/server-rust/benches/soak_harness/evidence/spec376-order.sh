#!/usr/bin/env bash
#
# Linux calibration series ORDER=OK -- a copy of spec373b-order.sh, which is
# NOT edited. The cal chain (spec376-chain.sh) runs it at its start and the
# calibration reading (spec376-calib.sh) re-runs it at its own start.
#
# A COPY EXISTS BECAUSE THE PARENT IS BOUND TO ITS OWN SERIES: it reads
# spec373b-manifest.md and the SPEC373B_CODE_FREEZE literal of
# spec373b-cells.sh. The difference list against spec373b-order.sh is CLOSED
# at exactly four items:
#
#   1. THE MANIFEST is spec376-manifest.md.
#   2. THE FREEZE LITERAL is CAL_PIN= in spec376-cells.sh, and it must occur
#      there EXACTLY ONCE: a second definition would let the runner and this
#      check read different commits.
#   3. PROGRAM COVERAGE, not a count. The parent asked for at least N listed
#      programs; a count cannot tell a complete list from a list that dropped
#      the one program that changed. Here M's section 1 must list (a) every
#      spec376-*.sh / .awk / .py file present in the evidence dir and (b) each
#      frozen parent the Linux programs execute, and every listed file must
#      hash as listed. An empty list is a failure, never a vacuous pass.
#   4. This header, the usage text and the success line (cal_pin= instead of
#      freeze=).
#
# A DIFF HUNK THAT MAPS TO NONE OF THE FOUR ITEMS IS A DEFECT; the manifest
# carries the hunk-to-item map (diff spec373b-order.sh spec376-order.sh).
#
# usage: spec376-order.sh <M> [<MANIFEST_FILE>]
#   M              the manifest commit (required, a revision)
#   MANIFEST_FILE  the manifest the caller will READ (default: the working-tree
#                  spec376-manifest.md); its section-1 prefix must hash equal
#                  to M's
#
# Checks, in order; the first failure prints "ORDER=FAIL <reason>" and exits 3:
#   1. M is an ancestor of HEAD.
#   2. The section-1 prefix sha256 of MANIFEST_FILE equals M's, by the verbatim
#      command of the manifest:
#        git show <M>:<manifest> | sed '/^## APPEND-ONLY BELOW/q' | shasum -a 256
#   3. Every program listed in M's section 1 (lines "- `<sha256>` `<path>`")
#      hashes to its listed sha256, and the list covers every spec376 program
#      in the evidence dir and every frozen parent named in REQUIRED_PARENTS.
#   4. The freeze gate over every build input: the CAL_PIN literal of
#      spec376-cells.sh is a hex commit id (never a movable name such as HEAD),
#      and there is no diff between it and HEAD, and an empty `git status
#      --porcelain`, over the pathspec below -- the spec's six paths plus the
#      root rust-toolchain.toml, which also selects what a build compiles.
# This file cannot vouch for itself: every caller checks its sha256 against
# M's listing BEFORE running it (spec376-calib.sh, spec376-chain.sh).
# On success prints one line:
#   ORDER=OK manifest_commit=<M> prefix_sha256=<sha> programs=<n> cal_pin=<F>
set -uo pipefail
export LC_ALL=C

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
SERVER_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd -P)"
REPO_ROOT="$(cd "$SERVER_ROOT/../.." && pwd -P)"
EVREL="packages/server-rust/benches/soak_harness/evidence"
MREL="${EVREL}/spec376-manifest.md"
BUILD_PATHS=(packages/server-rust/src packages/server-rust/Cargo.toml packages/server-rust/build.rs packages/core-rust Cargo.toml Cargo.lock rust-toolchain.toml)
# The frozen parents the Linux programs execute byte-for-byte (the rate
# program, the shares self-check, PM1, the fit and the two P5/P6/P7 awk
# programs); a manifest that forgets one would let it change unseen.
REQUIRED_PARENTS=(spec373b-verdict.sh spec373b-shares.py spec371-predicates.sh spec349c2-fit.awk spec366-p5.awk spec366-p67.awk)

fail() { echo "ORDER=FAIL $*"; exit 3; }

MC="${1:-}"
MFILE="${2:-${REPO_ROOT}/${MREL}}"
[ -n "$MC" ] || fail "no manifest commit given"
git -C "$REPO_ROOT" rev-parse --verify "${MC}^{commit}" >/dev/null 2>&1 || fail "${MC} is not a commit"
git -C "$REPO_ROOT" merge-base --is-ancestor "$MC" HEAD 2>/dev/null || fail "${MC} is not an ancestor of HEAD"
[ -f "$MFILE" ] || fail "manifest file ${MFILE} missing"

MTEXT="$(git -C "$REPO_ROOT" show "${MC}:${MREL}" 2>/dev/null)" || fail "M carries no ${MREL}"
printf '%s\n' "$MTEXT" | grep -q '^## APPEND-ONLY BELOW' || fail "M carries no APPEND-ONLY marker"
grep -q '^## APPEND-ONLY BELOW' "$MFILE" || fail "manifest file ${MFILE} carries no APPEND-ONLY marker"
PFX_M="$(git -C "$REPO_ROOT" show "${MC}:${MREL}" | sed '/^## APPEND-ONLY BELOW/q' | shasum -a 256 | awk '{print $1}')"
PFX_W="$(sed '/^## APPEND-ONLY BELOW/q' "$MFILE" | shasum -a 256 | awk '{print $1}')"
[ "${#PFX_M}" -eq 64 ] && [ "${#PFX_W}" -eq 64 ] || fail "a prefix sha256 could not be computed"
[ "$PFX_M" = "$PFX_W" ] || fail "manifest prefix sha256 ${PFX_W} != M ${PFX_M}"

PROGS="$(git -C "$REPO_ROOT" show "${MC}:${MREL}" | sed '/^## APPEND-ONLY BELOW/q' | sed -nE 's/^- `([0-9a-f]+)` `([^`]+)`.*/\1 \2/p')"
NPROG=0
while read -r want path; do
  [ -n "$want" ] || continue
  NPROG=$((NPROG + 1))
  got="$(shasum -a 256 "${REPO_ROOT}/${path}" 2>/dev/null | awk '{print $1}')"
  [ "${#want}" -eq 64 ] && [ "$got" = "$want" ] || fail "${path} hashes to '${got}', M lists ${want}"
done <<< "$PROGS"
[ "$NPROG" -gt 0 ] || fail "M lists no frozen programs"
# Each covered file must appear exactly once: a duplicate row could pin one
# copy while a reader trusts the other.
listed_once() {   # $1 = basename; 0 iff M lists evidence/<basename> exactly once
  [ "$(printf '%s\n' "$PROGS" | awk -v p="${EVREL}/$1" '$2 == p { n++ } END { print n + 0 }')" -eq 1 ]
}
for f in "$SCRIPT_DIR"/spec376-*.sh "$SCRIPT_DIR"/spec376-*.awk "$SCRIPT_DIR"/spec376-*.py; do
  [ -f "$f" ] || continue
  listed_once "$(basename "$f")" || fail "$(basename "$f") is not listed exactly once in M's section 1"
done
for p in "${REQUIRED_PARENTS[@]}"; do
  listed_once "$p" || fail "frozen parent ${p} is not listed exactly once in M's section 1"
done

[ "$(grep -c '^CAL_PIN=' "$SCRIPT_DIR/spec376-cells.sh" 2>/dev/null)" = "1" ] || fail "spec376-cells.sh must define CAL_PIN= exactly once"
FREEZE="$(sed -n 's/^CAL_PIN=//p' "$SCRIPT_DIR/spec376-cells.sh")"
[ -n "$FREEZE" ] || fail "no CAL_PIN literal in spec376-cells.sh"
printf '%s' "$FREEZE" | grep -Eq '^[0-9a-f]{8,40}$' || fail "CAL_PIN literal '${FREEZE}' is not a hex commit id"
git -C "$REPO_ROOT" rev-parse --verify "${FREEZE}^{commit}" >/dev/null 2>&1 || fail "CAL_PIN ${FREEZE} is not a commit"
git -C "$REPO_ROOT" diff --quiet "$FREEZE"..HEAD -- "${BUILD_PATHS[@]}" || fail "build inputs at HEAD differ from CAL_PIN ${FREEZE}"
[ -z "$(git -C "$REPO_ROOT" status --porcelain -- "${BUILD_PATHS[@]}")" ] || fail "the build-input working tree is dirty"

echo "ORDER=OK manifest_commit=${MC} prefix_sha256=${PFX_M} programs=${NPROG} cal_pin=${FREEZE}"
