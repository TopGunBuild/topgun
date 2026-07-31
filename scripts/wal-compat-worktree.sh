#!/usr/bin/env bash
#
# Cross-binary WAL compatibility, the REVERSE direction: an OLDER binary reading
# a WAL that THIS tree wrote.
#
# The forward direction (this binary reading an older WAL) is covered in-process
# by the crash harness's legacy-replay case and the checked-in legacy corpus.
# The reverse direction cannot be: the harness's crash model is an in-process
# modelled incarnation loss running ONE binary, so it can never execute a binary
# built from another commit. That is what this script exists for.
#
# BOTH frame positions are exercised, because a decoder that does NOT know the
# delta variant behaves differently between them, and asserting only one would
# report a guarantee the product does not have:
#
#   * MID-SEGMENT delta frame  -> payload deserialization fails with bytes still
#                                 ahead of it, which is corruption: refuse to
#                                 start.
#   * TRAILING delta frame     -> the same failure at the end of the byte stream
#                                 is a torn tail, which recovery TOLERATES on the
#                                 active segment: start cleanly, warn, silently
#                                 drop that mutation, then durably truncate the
#                                 segment to the intact prefix.
#
# MEASURED AGAINST THE PINNED BASE, AND IT IS NOT THAT. `feacb7d4` is the merge
# point of the READER, so the binary this script builds already understands the
# delta variant: it recovers a WAL from this tree CLEANLY IN BOTH POSITIONS, with
# the delta frame replayed rather than dropped or refused. The split above is the
# behaviour of a decoder OLDER than that reader — which is precisely what the
# one-step contract declines to support, so it is described here and asserted
# nowhere. Asserting the split against this base would be a test that passes only
# by describing something else.
#
# The contract is ONE-STEP: a WAL written here needs a binary at or after the
# reader's merge point. A two-step downgrade to a binary older than that is out
# of contract and is not claimed to work.
#
# Usage:  bash scripts/wal-compat-worktree.sh
# Env:    OLD_COMMIT   base commit to build the old binary from (default below)
#         KEEP         set to 1 to keep the worktree and scratch dirs

set -euo pipefail

OLD_COMMIT="${OLD_COMMIT:-feacb7d4}"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORKTREE="${WORKTREE:-/tmp/tg-wal-compat-${OLD_COMMIT}}"
OLD_TARGET="${OLD_TARGET:-/tmp/tg-wal-compat-target-${OLD_COMMIT}}"
SCRATCH="$(mktemp -d "/tmp/tg-wal-compat-run.XXXXXX")"

cleanup() {
  if [ "${KEEP:-0}" != "1" ]; then
    rm -rf "$SCRATCH"
  else
    echo "kept scratch: $SCRATCH"
  fi
}
trap cleanup EXIT

if command -v xcrun >/dev/null 2>&1; then
  export SDKROOT="${SDKROOT:-$(/usr/bin/xcrun --sdk macosx --show-sdk-path)}"
fi

echo "== 1. old tree at ${OLD_COMMIT} =="
if [ ! -d "$WORKTREE" ]; then
  git -C "$REPO_ROOT" worktree add --detach "$WORKTREE" "$OLD_COMMIT"
fi

echo "== 2. build the OLD binary =="
OLD_BIN="$OLD_TARGET/release/topgun-server"
if [ ! -x "$OLD_BIN" ]; then
  (cd "$WORKTREE" && CARGO_TARGET_DIR="$OLD_TARGET" cargo build --release --bin topgun-server)
fi
test -x "$OLD_BIN" || { echo "FAIL: no old binary at $OLD_BIN"; exit 1; }

# Runs the old binary over $1 and echoes "started" or "refused:<exit code>".
#
# "Started" is read as survival past a boot window rather than off a log line:
# recovery runs before the listener opens, so a process still alive after the
# window has completed recovery without refusing. A refusal exits immediately.
probe_old_binary() {
  local wal_dir="$1" run_dir="$2" log="$3"
  mkdir -p "$run_dir"
  (
    cd "$run_dir"
    TOPGUN_WAL_DIR="$wal_dir" \
    TOPGUN_ALLOW_NO_AUTH_PUBLIC_BIND=0 \
    TOPGUN_NO_AUTH=1 \
    HOST=127.0.0.1 \
    PORT=0 \
    RUST_LOG=info \
    "$OLD_BIN"
  ) >"$log" 2>&1 &
  local pid=$!
  local waited=0
  while [ "$waited" -lt 100 ]; do
    if ! kill -0 "$pid" 2>/dev/null; then
      wait "$pid" 2>/dev/null && echo "started" || echo "refused:$?"
      return 0
    fi
    sleep 0.1
    waited=$((waited + 1))
  done
  kill -TERM "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
  echo "started"
}

fail=0
for position in trailing mid; do
  echo
  echo "== 3.${position}: emit a WAL from THIS tree, delta frame in ${position} position =="
  wal_dir="$SCRATCH/wal-$position"
  TG_WAL_COMPAT_OUT="$wal_dir" \
  TG_WAL_COMPAT_POSITION="$position" \
    cargo test --release --manifest-path "$REPO_ROOT/packages/server-rust/Cargo.toml" \
      --lib -- --ignored --nocapture \
      emit_a_wal_this_tree_wrote_for_the_cross_binary_compat_script \
    >"$SCRATCH/emit-$position.log" 2>&1 \
    || { echo "FAIL: could not emit the $position WAL"; cat "$SCRATCH/emit-$position.log"; exit 1; }

  before_bytes=$(find "$wal_dir" -type f -exec stat -f%z {} \; 2>/dev/null | paste -sd+ - | bc \
                 || find "$wal_dir" -type f -printf '%s\n' | paste -sd+ - | bc)

  echo "== 4.${position}: run the OLD binary over it =="
  outcome="$(probe_old_binary "$wal_dir" "$SCRATCH/run-$position" "$SCRATCH/old-$position.log")"
  echo "   outcome: $outcome"

  after_bytes=$(find "$wal_dir" -type f -exec stat -f%z {} \; 2>/dev/null | paste -sd+ - | bc \
                || find "$wal_dir" -type f -printf '%s\n' | paste -sd+ - | bc)

  # The one-step contract, in both positions: start cleanly AND actually replay
  # the window. "Started" alone would be satisfiable by a binary that skipped
  # every frame, so the replay line is the non-vacuity half.
  if [ "$outcome" != "started" ]; then
    echo "   FAIL: the reader-era binary must recover this tree's WAL, got $outcome"
    tail -20 "$SCRATCH/old-$position.log"
    fail=1
  else
    replayed=$(grep -c 'WAL recovery: replaying unapplied entries' "$SCRATCH/old-$position.log" || true)
    if [ "${replayed:-0}" -lt 1 ]; then
      echo "   FAIL: started but replayed nothing -- the delta frame was skipped, not recovered"
      tail -20 "$SCRATCH/old-$position.log"
      fail=1
    else
      echo "   OK: started and replayed the window ($before_bytes -> $after_bytes bytes on disk)"
    fi
  fi
done

echo
if [ "$fail" -ne 0 ]; then
  echo "WAL cross-binary compatibility: FAILED"
  exit 1
fi
echo "WAL cross-binary compatibility: both frame positions satisfy the ONE-STEP contract"
echo "  (the pre-reader refuse-to-start / silent-tail-drop split is described in this"
echo "   script's header and asserted NOWHERE -- see the base-commit caveat above)"
