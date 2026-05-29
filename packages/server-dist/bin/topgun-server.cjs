#!/usr/bin/env node
'use strict';

// Platform → per-platform package name map.
// Only darwin-arm64 and linux-x64 are shipped in the initial release.
// Additional platforms (darwin-x64, linux-arm64, win32-x64) are planned for a future release.
const PLATFORM_PACKAGES = {
  'darwin-arm64': '@topgunbuild/server-darwin-arm64',
  'linux-x64':    '@topgunbuild/server-linux-x64',
};

const SUPPORTED = Object.keys(PLATFORM_PACKAGES).join(', ');

function getPlatformKey() {
  return `${process.platform}-${process.arch}`;
}

function resolveBinaryPath() {
  const key = getPlatformKey();
  const pkgName = PLATFORM_PACKAGES[key];

  if (!pkgName) {
    process.stderr.write(
      `\n  @topgunbuild/server: unsupported platform "${key}".\n` +
      `  Supported platforms: ${SUPPORTED}\n` +
      `  For ${key}, build from source: cargo build --release --bin topgun-server\n\n`
    );
    process.exit(1);
  }

  // Resolve the per-platform package's binary using require.resolve so the shim
  // works regardless of cwd, pnpm symlink layout, or npm flat layout.
  // We resolve the package.json first (always present) then derive the bin path,
  // avoiding any assumption about the internal directory structure of the per-platform
  // package relative to the caller.
  let pkgJsonPath;
  try {
    pkgJsonPath = require.resolve(`${pkgName}/package.json`);
  } catch (_) {
    process.stderr.write(
      `\n  @topgunbuild/server: the optional per-platform package "${pkgName}" is not installed.\n` +
      `  Run: npm install @topgunbuild/server\n` +
      `  Or build from source: cargo build --release --bin topgun-server\n\n`
    );
    process.exit(1);
  }

  const path = require('path');
  // The binary lives at bin/topgun-server relative to the per-platform package root.
  const pkgRoot = path.dirname(pkgJsonPath);
  return path.join(pkgRoot, 'bin', 'topgun-server');
}

const binaryPath = resolveBinaryPath();
const { spawnSync } = require('child_process');

const result = spawnSync(binaryPath, process.argv.slice(2), {
  stdio: 'inherit',
  env: process.env,
});

if (result.error) {
  process.stderr.write(`\n  @topgunbuild/server: failed to exec binary: ${result.error.message}\n\n`);
  process.exit(1);
}

process.exit(result.status !== null ? result.status : 1);
