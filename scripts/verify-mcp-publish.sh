#!/usr/bin/env bash
# Pre-publish verification gate for @topgunbuild/mcp-server.
#
# Simulates `npx -y @topgunbuild/mcp-server` end-to-end: packs the three
# user-facing packages exactly as `pnpm publish` would, installs them in a
# clean sandbox WITHOUT devDependencies (matching what an HN reader gets),
# spawns the MCP server, and drives the MCP stdio JSON-RPC handshake.
#
# Boot-only mode (default, used in CI): asserts the server starts, completes
# the MCP `initialize` handshake, and registers all 8 tools. Does NOT require
# a running TopGun server.
#
# Full mode (`--full`): additionally drives the 8 user-facing tool calls plus
# 2 edge cases against a TopGun server reachable via $TOPGUN_URL
# (default ws://localhost:8080). Use locally before `pnpm publish`.
#
# Background:
# @topgunbuild/client@2.0.1 shipped with pino-pretty as a devDependency
# but hard-required it as a pino transport target. `npx -y @topgunbuild/mcp-server`
# crashed synchronously on require. This gate catches that class of bug
# (package boots only when its devDeps are accidentally present).

set -euo pipefail

REPO_ROOT="$(cd "$(/usr/bin/dirname "$0")/.." && pwd)"
SANDBOX_DIR="${SANDBOX_DIR:-/tmp/mcp-publish-check}"
MODE="${1:-boot}"

cd "$REPO_ROOT"

echo "=== verify-mcp-publish — mode=$MODE sandbox=$SANDBOX_DIR ==="

echo "[1/5] Building user-facing packages…"
pnpm --filter @topgunbuild/core --filter @topgunbuild/client --filter @topgunbuild/mcp-server build

echo "[2/5] Resetting sandbox…"
/bin/rm -rf "$SANDBOX_DIR"
/bin/mkdir -p "$SANDBOX_DIR/tarballs"

echo "[3/5] Packing tarballs…"
for pkg in core client mcp-server; do
  pnpm --filter "@topgunbuild/$pkg" exec pnpm pack --pack-destination="$SANDBOX_DIR/tarballs" > /dev/null
done

CLIENT_TGZ=$(/bin/ls "$SANDBOX_DIR/tarballs"/topgunbuild-client-*.tgz)
CORE_TGZ=$(/bin/ls "$SANDBOX_DIR/tarballs"/topgunbuild-core-*.tgz)
MCP_TGZ=$(/bin/ls "$SANDBOX_DIR/tarballs"/topgunbuild-mcp-server-*.tgz)

# Quote-safe filenames in the package.json below.
cat > "$SANDBOX_DIR/package.json" <<EOF
{
  "name": "mcp-publish-check",
  "private": true,
  "version": "0.0.1",
  "description": "Clean-install sandbox that simulates 'npx -y @topgunbuild/mcp-server' from local tarballs.",
  "dependencies": {
    "@topgunbuild/mcp-server": "file:./tarballs/$(/usr/bin/basename "$MCP_TGZ")"
  },
  "overrides": {
    "@topgunbuild/client": "file:./tarballs/$(/usr/bin/basename "$CLIENT_TGZ")",
    "@topgunbuild/core": "file:./tarballs/$(/usr/bin/basename "$CORE_TGZ")"
  }
}
EOF

echo "[4/5] Installing tarballs in sandbox (--omit=dev)…"
cd "$SANDBOX_DIR"
npm install --omit=dev --no-audit --no-fund --silent

# Sanity: pino-pretty MUST NOT be present — that's the regression we're guarding against.
if /usr/bin/find node_modules -maxdepth 4 -name pino-pretty -type d | /usr/bin/grep -q .; then
  echo "FAIL: pino-pretty was installed in the sandbox. The package must not require it at runtime." >&2
  exit 1
fi

cp "$REPO_ROOT/scripts/verify-mcp-publish-driver.mjs" "$SANDBOX_DIR/driver.mjs"

echo "[5/5] Driving stdio JSON-RPC…"
if [ "$MODE" = "--full" ] || [ "$MODE" = "full" ]; then
  TEST_MODE=full node driver.mjs
else
  TEST_MODE=boot node driver.mjs
fi

echo "=== verify-mcp-publish: PASS ==="
