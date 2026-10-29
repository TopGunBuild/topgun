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
# Every WAL this tree writes is format v2: each segment starts with a "TGWS"
# header. A binary built before that format reads the header as a frame whose
# magic is wrong at offset 0 and refuses to start, so no older binary reads a
# WAL from this tree, whatever the frames inside it hold. The delta frame is
# still placed in BOTH positions -- mid-segment and trailing -- because a
# trailing frame is where an older decoder tolerates a torn tail; the refusal
# must hold there too, before the decoder reaches any frame.
#
# Asserted in both positions: the old binary exits non-zero, logs wrong magic at
# offset 0, and leaves the WAL directory byte for byte as it was.
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

# Every file of the WAL directory with its content digest, in name order.
wal_digest() {
  (cd "$1" && find . -type f | LC_ALL=C sort | while read -r f; do
    printf '%s %s\n' "$(shasum -a 256 <"$f" | cut -d' ' -f1)" "$f"
  done)
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

  before="$(wal_digest "$wal_dir")"

  echo "== 4.${position}: run the OLD binary over it =="
  outcome="$(probe_old_binary "$wal_dir" "$SCRATCH/run-$position" "$SCRATCH/old-$position.log")"
  echo "   outcome: $outcome"

  after="$(wal_digest "$wal_dir")"

  if [ "${outcome#refused:}" = "$outcome" ] || [ "$outcome" = "refused:0" ]; then
    echo "   FAIL: an older binary must refuse a v2 WAL, got $outcome"
    tail -20 "$SCRATCH/old-$position.log"
    fail=1
  elif ! grep -q 'has wrong magic at offset 0' "$SCRATCH/old-$position.log"; then
    echo "   FAIL: refused, but not on the v2 header (no wrong magic at offset 0)"
    tail -20 "$SCRATCH/old-$position.log"
    fail=1
  elif [ "$before" != "$after" ]; then
    echo "   FAIL: the refusal changed the WAL directory"
    fail=1
  else
    echo "   OK: refused with wrong magic at offset 0; the WAL directory is unchanged"
  fi
done

echo
if [ "$fail" -ne 0 ]; then
  echo "WAL cross-binary compatibility: FAILED"
  exit 1
fi
echo "WAL cross-binary compatibility: an older binary refuses this tree's WAL in both frame positions"
