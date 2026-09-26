#!/usr/bin/env bash
#
# spec373b ORDER=OK: the freeze check that the cell chain runs at its start and
# the verdict program re-runs at its own start.
#
# usage: spec373b-order.sh <M> [<MANIFEST_FILE>]
#   M              the manifest commit (required, a revision)
#   MANIFEST_FILE  the manifest the caller will READ (default: the working-tree
#                  spec373b-manifest.md); its section-1 prefix must hash equal
#                  to M's
#
# Checks, in order; the first failure prints "ORDER=FAIL <reason>" and exits 3:
#   1. M is an ancestor of HEAD.
#   2. The section-1 prefix sha256 of MANIFEST_FILE equals M's, by the verbatim
#      command of the manifest:
#        git show <M>:<manifest> | sed '/^## APPEND-ONLY BELOW/q' | shasum -a 256
#   3. Every program listed in M's section 1 (lines "- `<sha256>` `<path>`")
#      hashes to its listed sha256, and M lists at least MIN_PROGS of them.
#   4. The freeze gate over every build input: no diff between the freeze
#      literal of spec373b-cells.sh and HEAD, and an empty `git status
#      --porcelain`, over the pathspec below.
# On success prints one line:
#   ORDER=OK manifest_commit=<M> prefix_sha256=<sha> programs=<n> freeze=<F>
set -uo pipefail
export LC_ALL=C

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
SERVER_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd -P)"
REPO_ROOT="$(cd "$SERVER_ROOT/../.." && pwd -P)"
MREL="packages/server-rust/benches/soak_harness/evidence/spec373b-manifest.md"
BUILD_PATHS=(packages/server-rust/src packages/server-rust/Cargo.toml packages/server-rust/build.rs packages/core-rust Cargo.toml Cargo.lock)
MIN_PROGS=10

fail() { echo "ORDER=FAIL $*"; exit 3; }

MC="${1:-}"
MFILE="${2:-${REPO_ROOT}/${MREL}}"
[ -n "$MC" ] || fail "no manifest commit given"
git -C "$REPO_ROOT" rev-parse --verify "${MC}^{commit}" >/dev/null 2>&1 || fail "${MC} is not a commit"
git -C "$REPO_ROOT" merge-base --is-ancestor "$MC" HEAD 2>/dev/null || fail "${MC} is not an ancestor of HEAD"
[ -f "$MFILE" ] || fail "manifest file ${MFILE} missing"

PFX_M="$(git -C "$REPO_ROOT" show "${MC}:${MREL}" 2>/dev/null | sed '/^## APPEND-ONLY BELOW/q' | shasum -a 256 | awk '{print $1}')"
PFX_W="$(sed '/^## APPEND-ONLY BELOW/q' "$MFILE" | shasum -a 256 | awk '{print $1}')"
git -C "$REPO_ROOT" show "${MC}:${MREL}" 2>/dev/null | grep -q '^## APPEND-ONLY BELOW' || fail "M carries no APPEND-ONLY marker"
[ "$PFX_M" = "$PFX_W" ] || fail "manifest prefix sha256 ${PFX_W} != M ${PFX_M}"

PROGS="$(git -C "$REPO_ROOT" show "${MC}:${MREL}" | sed '/^## APPEND-ONLY BELOW/q' | sed -nE 's/^- `([0-9a-f]+)` `([^`]+)`.*/\1 \2/p')"
NPROG=0
while read -r want path; do
  [ -n "$want" ] || continue
  NPROG=$((NPROG + 1))
  got="$(shasum -a 256 "${REPO_ROOT}/${path}" 2>/dev/null | awk '{print $1}')"
  [ "${#want}" -eq 64 ] && [ "$got" = "$want" ] || fail "${path} hashes to '${got}', M lists ${want}"
done <<< "$PROGS"
[ "$NPROG" -ge "$MIN_PROGS" ] || fail "M lists ${NPROG} frozen programs, expected at least ${MIN_PROGS}"

FREEZE="$(awk -F= '/^SPEC373B_CODE_FREEZE=/ { print $2; exit }' "$SCRIPT_DIR/spec373b-cells.sh")"
[ -n "$FREEZE" ] || fail "no SPEC373B_CODE_FREEZE literal in spec373b-cells.sh"
git -C "$REPO_ROOT" rev-parse --verify "${FREEZE}^{commit}" >/dev/null 2>&1 || fail "freeze ${FREEZE} is not a commit"
git -C "$REPO_ROOT" diff --quiet "$FREEZE"..HEAD -- "${BUILD_PATHS[@]}" || fail "build inputs at HEAD differ from the freeze ${FREEZE}"
[ -z "$(git -C "$REPO_ROOT" status --porcelain -- "${BUILD_PATHS[@]}")" ] || fail "the build-input working tree is dirty"

echo "ORDER=OK manifest_commit=${MC} prefix_sha256=${PFX_M} programs=${NPROG} freeze=${FREEZE}"
