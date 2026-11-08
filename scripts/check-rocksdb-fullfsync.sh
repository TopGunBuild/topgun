#!/usr/bin/env bash
# Checks that every darwin build of RocksDB defines HAVE_FULLFSYNC.
#
# RocksDB's build script does not define it on macOS, and without it RocksDB's
# Sync() is a plain fsync, which macOS does not make durable on the device.
# Two places set it, and both must:
#
#   1. .cargo/config.toml, [env] CXXFLAGS_aarch64_apple_darwin = "-DHAVE_FULLFSYNC"
#      (every cargo build in the repository, native and cargo-zigbuild alike);
#   2. scripts/build-server-binaries.sh, export CXXFLAGS_aarch64_apple_darwin=
#      "-DHAVE_FULLFSYNC" (the script that builds the published binaries).
#
# Exits 1 naming each missing one.
set -euo pipefail

cd "$(dirname "$0")/.."

missing=0

# The entry must sit in the [env] table: the lines after `[env]` up to the
# next table header.
if ! awk '
  /^[[:space:]]*\[/ { in_env = ($0 ~ /^[[:space:]]*\[env\][[:space:]]*$/); next }
  in_env && /^[[:space:]]*CXXFLAGS_aarch64_apple_darwin[[:space:]]*=[[:space:]]*"-DHAVE_FULLFSYNC"[[:space:]]*$/ { found = 1 }
  END { exit !found }
' .cargo/config.toml 2>/dev/null; then
  echo "ERROR: .cargo/config.toml has no [env] entry CXXFLAGS_aarch64_apple_darwin = \"-DHAVE_FULLFSYNC\"" >&2
  missing=1
fi

if ! grep -Eq '^[[:space:]]*export[[:space:]]+CXXFLAGS_aarch64_apple_darwin="-DHAVE_FULLFSYNC"[[:space:]]*$' \
  scripts/build-server-binaries.sh; then
  echo "ERROR: scripts/build-server-binaries.sh does not export CXXFLAGS_aarch64_apple_darwin=\"-DHAVE_FULLFSYNC\"" >&2
  missing=1
fi

if [ "$missing" -ne 0 ]; then
  exit 1
fi
echo "RocksDB HAVE_FULLFSYNC: set in .cargo/config.toml [env] and scripts/build-server-binaries.sh"
