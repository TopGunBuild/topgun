#!/usr/bin/env bash
# Post-publish live-registry smoke test for @topgunbuild/mcp-server.
#
# Polls npm until the specified version is visible in the registry, then runs
# a clean-dir --help check to confirm the package installs and starts without
# module-resolution errors. No running TopGun server is required — the --help
# path exits 0 from the CLI layer before any backend connection is attempted.
#
# Usage (locally or from CI):
#   bash scripts/smoke-npx-mcp.sh [version]
#   version defaults to "latest"
#
# Tunable env vars (for slow registry propagation):
#   SMOKE_NPM_POLL_TIMEOUT   total seconds to wait for npm view (default: 180)
#   SMOKE_NPM_POLL_INTERVAL  retry interval in seconds (default: 5)

set -euo pipefail

VERSION="${1:-latest}"
SMOKE_NPM_POLL_TIMEOUT="${SMOKE_NPM_POLL_TIMEOUT:-180}"
SMOKE_NPM_POLL_INTERVAL="${SMOKE_NPM_POLL_INTERVAL:-5}"

PASS_COUNT=0
FAIL_COUNT=0
TMPDIR_BASE="/tmp/smoke-mcp-$$"

# ── Helpers ───────────────────────────────────────────────────────────────────

pass() {
  echo "PASS: $1"
  PASS_COUNT=$((PASS_COUNT + 1))
}

fail() {
  echo "FAIL: $1"
  FAIL_COUNT=$((FAIL_COUNT + 1))
}

fail_exit() {
  echo "FAIL: $1"
  exit 1
}

cleanup() {
  rm -rf "$TMPDIR_BASE"
}
trap cleanup EXIT

echo ""
echo "=== smoke-npx-mcp @topgunbuild/mcp-server@${VERSION} ==="
echo ""

# ── Check 1: npm view poll ────────────────────────────────────────────────────

echo "[1/2] Polling npm view @topgunbuild/mcp-server@${VERSION}..."
ELAPSED=0
RESOLVED_VERSION=""
while [ "$ELAPSED" -lt "$SMOKE_NPM_POLL_TIMEOUT" ]; do
  RESULT=$(npm view "@topgunbuild/mcp-server@${VERSION}" version 2>/dev/null || true)
  if [ -n "$RESULT" ]; then
    RESOLVED_VERSION="$RESULT"
    pass "npm-view: resolved @topgunbuild/mcp-server@${VERSION} → ${RESOLVED_VERSION}"
    break
  fi
  echo "  registry not yet reflecting ${VERSION}, retrying in ${SMOKE_NPM_POLL_INTERVAL}s (${ELAPSED}/${SMOKE_NPM_POLL_TIMEOUT}s)..."
  sleep "$SMOKE_NPM_POLL_INTERVAL"
  ELAPSED=$((ELAPSED + SMOKE_NPM_POLL_INTERVAL))
done

if [ -z "$RESOLVED_VERSION" ]; then
  fail_exit "npm-view: @topgunbuild/mcp-server@${VERSION} did not resolve within ${SMOKE_NPM_POLL_TIMEOUT}s (registry propagation timeout)"
fi

# Use the resolved exact version so "latest" is pinned for the help check.
EXACT_VERSION="$RESOLVED_VERSION"

# ── Check 2: clean-dir --help ─────────────────────────────────────────────────

echo "[2/2] Running npx --help in clean dir (no backend required)..."
HELP_DIR="$TMPDIR_BASE/help"
mkdir -p "$HELP_DIR"
cd "$HELP_DIR"

HELP_OUT=$(npx -y "@topgunbuild/mcp-server@${EXACT_VERSION}" --help 2>&1) || {
  fail "help: npx exited non-zero for @topgunbuild/mcp-server@${EXACT_VERSION}"
  HELP_OUT=""
}

if [ -n "$HELP_OUT" ]; then
  if echo "$HELP_OUT" | grep -qiE "Cannot find module|MODULE_NOT_FOUND"; then
    fail "help: output contains module resolution error — package likely has a missing production dependency"
    echo "  Output snippet:"
    echo "$HELP_OUT" | head -20
  else
    pass "help: npx @topgunbuild/mcp-server@${EXACT_VERSION} --help exited 0, no module errors"
  fi
fi

# ── Final summary ─────────────────────────────────────────────────────────────

TOTAL=$((PASS_COUNT + FAIL_COUNT))
echo ""
if [ "$FAIL_COUNT" -eq 0 ]; then
  echo "=== smoke-npx-mcp: PASS (${PASS_COUNT}/${TOTAL} checks) ==="
  exit 0
else
  echo "=== smoke-npx-mcp: FAIL (${PASS_COUNT} passed, ${FAIL_COUNT} failed of ${TOTAL} checks) ==="
  exit 1
fi
