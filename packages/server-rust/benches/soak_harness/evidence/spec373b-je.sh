#!/usr/bin/env bash
#
# spec373b JE reading (RECORDED only, n = 1 per side, no class, no STOP): runs
# the frozen spec372-predicates.sh -- unedited, sha256 asserted -- over one
# spec373b jemalloc cell (jb or ja).
#
# usage: spec373b-je.sh <EV_DIR> <jb|ja> <BUILDS_FILE>
#
# spec372-predicates.sh derives the flavour from its own cell names, so the
# cell's artifacts are COPIED into a scratch dir under the name of its JE cell
# spec372-j2 (the 4 h JE cell), with a two-line builds stub carrying the JE
# server and harness sha256 of THIS chain's builds file (JE-pin for jb, JE-head
# for ja; flavour renamed to the JE the frozen program expects). Its outputs
# (predicates, fits, amp, ampfp) are copied back as spec373b-<cell>.*. The
# recorded values are TERM_/DECIDE_ AMP_FP, AMP_JE, DIRTY_SHARE, FRAG_SHARE.
set -uo pipefail
export LC_ALL=C
EV="${1:-}"; CELL="${2:-}"; BUILDS="${3:-}"
if [ "$#" -ne 3 ] || [ ! -d "$EV" ] || [ ! -s "$BUILDS" ]; then
  echo "usage: spec373b-je.sh <EV_DIR> <jb|ja> <BUILDS_FILE>" >&2; exit 2
fi
case "$CELL" in jb) SRCFL=JE-pin ;; ja) SRCFL=JE-head ;; *) echo "FATAL: cell must be jb or ja" >&2; exit 2 ;; esac
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
PRED372="$SCRIPT_DIR/spec372-predicates.sh"
PRED372_SHA=baf0bce7dd6c29751fb62f525153b631ec9eeb37f0ba858aed7174fd978b1c91
[ "$(shasum -a 256 "$PRED372" | awk '{print $1}')" = "$PRED372_SHA" ] || { echo "FATAL: ${PRED372} does not hash to ${PRED372_SHA}" >&2; exit 3; }

sha_of() { awk -v f="$1" '{ fv = ""; sh = ""; for (i = 1; i <= NF; i++) { split($i, kv, "="); if (kv[1] == "flavour") fv = kv[2]; if (kv[1] == "sha256") sh = kv[2] } if (fv == f) { print sh; exit } }' "$BUILDS"; }
SRV="$(sha_of "$SRCFL")"; HAR="$(sha_of H)"
[ "${#SRV}" -eq 64 ] && [ "${#HAR}" -eq 64 ] || { echo "FATAL: ${BUILDS} lacks a ${SRCFL} or H sha256" >&2; exit 3; }

TMP="$(mktemp -d "${TMPDIR:-/tmp}/spec373b-je.XXXXXX")" || exit 4
SRC="$EV/spec373b-${CELL}"; DST="$TMP/spec372-j2"
for ext in csv harness-console.log matrix.txt runner-console.log soak.json soak.durable.json progress.jsonl mechanism.json; do
  [ -e "${SRC}.${ext}" ] && cp "${SRC}.${ext}" "${DST}.${ext}"
done
[ -d "${SRC}.scrapes" ] && cp -R "${SRC}.scrapes" "${DST}.scrapes"
printf 'flavour=JE sha256=%s\nflavour=H sha256=%s\n' "$SRV" "$HAR" > "$TMP/builds.txt"
echo "je reading: cell=${CELL} as spec372-j2, server ${SRCFL} sha256=${SRV}, harness sha256=${HAR}, program sha256=${PRED372_SHA}"
bash "$PRED372" "$TMP" spec372-j2 "$TMP/builds.txt"
rc=$?
for ext in predicates.txt fits.txt amp.txt ampfp.csv; do
  [ -e "${DST}.${ext}" ] && cp "${DST}.${ext}" "${SRC}.${ext}"
done
rm -rf "$TMP"
[ "$rc" -eq 0 ] || { echo "FATAL: spec372-predicates.sh rc=${rc}" >&2; exit 4; }
grep -E '^(TERM|DECIDE)_(AMP_FP|AMP_JE|DIRTY_SHARE|FRAG_SHARE)=' "${SRC}.predicates.txt"
