#!/usr/bin/env bash
# Post-publish live-registry smoke test for @topgunbuild/server.
#
# Pulls the specified version from npm (polling until registry propagation
# completes), runs a clean-dir --help check, asserts the os/cpu gating
# installs only the host platform package, verifies the binary size and
# exec bit, boots the server zero-config and confirms /health + /api/status
# + /admin/ all return 2xx, and asserts the server is loopback-bound (not
# reachable on the host's non-loopback address) when no auth secret is set.
#
# Usage (locally or from CI):
#   bash scripts/smoke-npx-published.sh [version]
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
TMPDIR_BASE="/tmp/smoke-server-$$"
SERVER_PID=""

# ── Helpers ───────────────────────────────────────────────────────────────────

pass() {
  echo "PASS: $1"
  PASS_COUNT=$((PASS_COUNT + 1))
}

fail() {
  echo "FAIL: $1"
  FAIL_COUNT=$((FAIL_COUNT + 1))
  # Do NOT exit immediately — collect all failures before teardown.
}

fail_exit() {
  echo "FAIL: $1"
  exit 1
}

cleanup() {
  if [ -n "$SERVER_PID" ] && kill -0 "$SERVER_PID" 2>/dev/null; then
    kill "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true
  fi
  rm -rf "$TMPDIR_BASE"
}
trap cleanup EXIT

echo ""
echo "=== smoke-npx-published @topgunbuild/server@${VERSION} ==="
echo ""

# ── Check 1: npm view poll ────────────────────────────────────────────────────

echo "[1/6] Polling npm view @topgunbuild/server@${VERSION}..."
ELAPSED=0
RESOLVED_VERSION=""
while [ "$ELAPSED" -lt "$SMOKE_NPM_POLL_TIMEOUT" ]; do
  RESULT=$(npm view "@topgunbuild/server@${VERSION}" version 2>/dev/null || true)
  if [ -n "$RESULT" ]; then
    RESOLVED_VERSION="$RESULT"
    pass "npm-view: resolved @topgunbuild/server@${VERSION} → ${RESOLVED_VERSION}"
    break
  fi
  echo "  registry not yet reflecting ${VERSION}, retrying in ${SMOKE_NPM_POLL_INTERVAL}s (${ELAPSED}/${SMOKE_NPM_POLL_TIMEOUT}s)..."
  sleep "$SMOKE_NPM_POLL_INTERVAL"
  ELAPSED=$((ELAPSED + SMOKE_NPM_POLL_INTERVAL))
done

if [ -z "$RESOLVED_VERSION" ]; then
  fail_exit "npm-view: @topgunbuild/server@${VERSION} did not resolve within ${SMOKE_NPM_POLL_TIMEOUT}s (registry propagation timeout)"
fi

# Use the resolved exact version for all subsequent checks so "latest" is pinned.
EXACT_VERSION="$RESOLVED_VERSION"

# ── Check 2: clean-dir --help ─────────────────────────────────────────────────

echo "[2/6] Running npx --help in clean dir..."
HELP_DIR="$TMPDIR_BASE/help"
mkdir -p "$HELP_DIR"
cd "$HELP_DIR"

HELP_OUT=$(npx -y "@topgunbuild/server@${EXACT_VERSION}" --help 2>&1) || {
  fail "help: npx exited non-zero"
  HELP_OUT=""
}

if [ -n "$HELP_OUT" ]; then
  if echo "$HELP_OUT" | grep -qiE "Cannot find module|MODULE_NOT_FOUND"; then
    fail "help: output contains module resolution error"
  else
    pass "help: npx @topgunbuild/server@${EXACT_VERSION} --help exited 0, no module errors"
  fi
fi

# ── Check 3: single-platform pull assertion ───────────────────────────────────

echo "[3/6] Installing and asserting single-platform package pull..."
INSTALL_DIR="$TMPDIR_BASE/install"
mkdir -p "$INSTALL_DIR"
cd "$INSTALL_DIR"

# Determine expected platform package based on host OS and architecture.
HOST_OS=$(uname -s)
HOST_ARCH=$(uname -m)

if [ "$HOST_OS" = "Darwin" ] && [ "$HOST_ARCH" = "arm64" ]; then
  EXPECTED_PKG="server-darwin-arm64"
  UNEXPECTED_PKG="server-linux-x64"
elif [ "$HOST_OS" = "Linux" ] && [ "$HOST_ARCH" = "x86_64" ]; then
  EXPECTED_PKG="server-linux-x64"
  UNEXPECTED_PKG="server-darwin-arm64"
else
  echo "  WARNING: unrecognized platform ${HOST_OS}/${HOST_ARCH} — skipping single-platform pull assertion"
  EXPECTED_PKG=""
  UNEXPECTED_PKG=""
fi

npm install "@topgunbuild/server@${EXACT_VERSION}" --omit=dev --no-audit --no-fund --silent

if [ -n "$EXPECTED_PKG" ]; then
  if [ -d "node_modules/@topgunbuild/${EXPECTED_PKG}" ]; then
    pass "single-platform-pull: @topgunbuild/${EXPECTED_PKG} is present (os/cpu gate working)"
  else
    fail "single-platform-pull: expected @topgunbuild/${EXPECTED_PKG} is NOT present"
  fi

  if [ -d "node_modules/@topgunbuild/${UNEXPECTED_PKG}" ]; then
    fail "single-platform-pull: unexpected @topgunbuild/${UNEXPECTED_PKG} is present (os/cpu gate broken)"
  else
    pass "single-platform-pull: @topgunbuild/${UNEXPECTED_PKG} correctly absent"
  fi
fi

# ── Check 4: binary invariants ────────────────────────────────────────────────

echo "[4/6] Checking binary size and exec bit..."
BINARY=$(find "node_modules/@topgunbuild" -name 'topgun-server' -type f 2>/dev/null | head -1)

if [ -z "$BINARY" ]; then
  fail "binary-invariants: could not locate topgun-server binary in node_modules"
else
  MAX_SIZE_BYTES=$((20 * 1024 * 1024))
  BINARY_SIZE=$(wc -c < "$BINARY")
  if [ "$BINARY_SIZE" -gt "$MAX_SIZE_BYTES" ]; then
    fail "binary-invariants: binary size ${BINARY_SIZE} bytes exceeds 20 MB ceiling"
  else
    pass "binary-invariants: size ${BINARY_SIZE} bytes (< 20 MB)"
  fi

  if [ -x "$BINARY" ]; then
    pass "binary-invariants: exec bit is set"
  else
    fail "binary-invariants: exec bit NOT set on ${BINARY}"
  fi
fi

# ── Check 5: zero-config boot + endpoint curls ────────────────────────────────

echo "[5/6] Zero-config server boot + endpoint checks..."
BOOT_DIR="$TMPDIR_BASE/boot"
mkdir -p "$BOOT_DIR"
cd "$BOOT_DIR"

npm install "@topgunbuild/server@${EXACT_VERSION}" --omit=dev --no-audit --no-fund --silent

# Use a fixed port unlikely to conflict; if occupied the server will fail to bind and we catch it.
PORT=18765

# Boot the server; shim sets TOPGUN_NO_AUTH=1 + loopback bind automatically.
PORT="$PORT" node node_modules/@topgunbuild/server/bin/topgun-server.cjs > server.log 2>&1 &
SERVER_PID=$!

# Wait for the server to become ready by polling /health on the loopback literal.
READY=0
for i in $(seq 1 30); do
  if curl -sf "http://127.0.0.1:${PORT}/health" > /dev/null 2>&1; then
    READY=1
    break
  fi
  sleep 1
done

if [ "$READY" -eq 0 ]; then
  echo "  Server log:"
  cat server.log || true
  fail "boot: server did not become ready on 127.0.0.1:${PORT} within 30s"
else
  pass "boot: server ready on 127.0.0.1:${PORT}"

  # Assert each endpoint returns 2xx.
  for ENDPOINT in "/health" "/api/status" "/admin/"; do
    HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" "http://127.0.0.1:${PORT}${ENDPOINT}" 2>/dev/null || echo "000")
    if [[ "$HTTP_CODE" =~ ^2 ]]; then
      pass "endpoint ${ENDPOINT}: HTTP ${HTTP_CODE}"
    else
      fail "endpoint ${ENDPOINT}: HTTP ${HTTP_CODE} (expected 2xx)"
    fi
  done
fi

# ── Check 6: loopback-only bind assertion ─────────────────────────────────────

echo "[6/6] Asserting loopback-only bind (non-loopback address must be unreachable)..."

# Discover the host's primary non-loopback IPv4 in an OS-conditional way.
# macOS and Linux use different network utility commands.
NON_LOOPBACK_ADDR=""
if [ "$HOST_OS" = "Darwin" ]; then
  # macOS: try the primary Wi-Fi/Ethernet interface first, then fall back to ifconfig parsing.
  NON_LOOPBACK_ADDR=$(ipconfig getifaddr en0 2>/dev/null || true)
  if [ -z "$NON_LOOPBACK_ADDR" ]; then
    NON_LOOPBACK_ADDR=$(ipconfig getifaddr en1 2>/dev/null || true)
  fi
  if [ -z "$NON_LOOPBACK_ADDR" ]; then
    NON_LOOPBACK_ADDR=$(ifconfig 2>/dev/null | awk '/inet / && !/127\.0\.0\.1/{print $2; exit}' || true)
  fi
elif [ "$HOST_OS" = "Linux" ]; then
  # Linux: use ip route to find the source address for outbound routing, then hostname -I as fallback.
  NON_LOOPBACK_ADDR=$(ip route get 1 2>/dev/null | awk '{for(i=1;i<=NF;i++) if($i=="src") {print $(i+1); exit}}' || true)
  if [ -z "$NON_LOOPBACK_ADDR" ]; then
    NON_LOOPBACK_ADDR=$(hostname -I 2>/dev/null | awk '{print $1}' || true)
  fi
fi

if [ -z "$NON_LOOPBACK_ADDR" ] || [ "$NON_LOOPBACK_ADDR" = "127.0.0.1" ]; then
  echo "  WARNING: could not determine non-loopback address (likely a container/CI with no external interface) — skipping loopback-bind assertion"
  pass "loopback-bind: skipped (no non-loopback interface detected)"
else
  echo "  Non-loopback address: ${NON_LOOPBACK_ADDR}"
  # The no-auth server binds ONLY to 127.0.0.1. A connection attempt to the non-loopback
  # interface must fail (refused or timeout). We use a short timeout to avoid hanging CI.
  if curl -sf --connect-timeout 3 --max-time 3 "http://${NON_LOOPBACK_ADDR}:${PORT}/health" > /dev/null 2>&1; then
    fail "loopback-bind: server is reachable on non-loopback ${NON_LOOPBACK_ADDR}:${PORT} (bind is NOT loopback-only)"
  else
    pass "loopback-bind: server NOT reachable on ${NON_LOOPBACK_ADDR}:${PORT} (loopback-only confirmed)"
  fi
fi

# Also confirm the loopback literal 127.0.0.1 is reachable (not localhost hostname
# which can resolve to ::1 on macOS with IPv4-only bind).
if curl -sf --connect-timeout 3 "http://127.0.0.1:${PORT}/health" > /dev/null 2>&1; then
  pass "loopback-bind: 127.0.0.1:${PORT}/health reachable"
else
  fail "loopback-bind: 127.0.0.1:${PORT}/health NOT reachable (unexpected)"
fi

# ── Final summary ─────────────────────────────────────────────────────────────

TOTAL=$((PASS_COUNT + FAIL_COUNT))
echo ""
if [ "$FAIL_COUNT" -eq 0 ]; then
  echo "=== smoke-npx-published: PASS (${PASS_COUNT}/${TOTAL} checks) ==="
  exit 0
else
  echo "=== smoke-npx-published: FAIL (${PASS_COUNT} passed, ${FAIL_COUNT} failed of ${TOTAL} checks) ==="
  exit 1
fi
