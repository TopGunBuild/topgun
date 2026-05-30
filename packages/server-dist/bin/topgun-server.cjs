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

// Ensure the binary is executable. We cannot rely on the postinstall chmod:
// pnpm v10+ ignores dependency build scripts by default (requires
// `pnpm approve-builds`), and tarball extraction does not reliably preserve the
// executable bit across package managers. The shim runs on every invocation, so
// chmod here is the robust guarantee that `npx @topgunbuild/server` works under
// npm, pnpm, and yarn alike. Best-effort: a failure here is surfaced by the spawn.
if (process.platform !== 'win32') {
  try {
    require('fs').chmodSync(binaryPath, 0o755);
  } catch (_) {
    /* fall through — spawn will report the real error if the binary is unusable */
  }
}

// Zero-config convenience: if the operator has configured neither a JWT secret
// nor an explicit auth posture, default to local-dev mode (auth disabled). The
// server, in turn, binds loopback-only when auth is disabled, so this never
// exposes an unauthenticated server to the network. Setting JWT_SECRET or
// TOPGUN_NO_AUTH explicitly overrides this default.
const env = { ...process.env };
if (!env.JWT_SECRET && env.TOPGUN_NO_AUTH === undefined) {
  env.TOPGUN_NO_AUTH = '1';
  process.stderr.write(
    '\n  @topgunbuild/server: starting in local-dev mode (auth disabled, bound to 127.0.0.1).\n' +
    '  To enforce auth, set JWT_SECRET=<secret>. To silence this notice, set TOPGUN_NO_AUTH explicitly.\n\n'
  );
}

// Point the server at the admin SPA bundled inside this meta package, so an
// `npx @topgunbuild/server` user outside the monorepo gets a working admin UI.
// The SPA ships once here (sibling of this bin/ dir), not in the per-platform
// binary packages. Resolving from __dirname is layout-stable under both npm-flat
// and pnpm-symlinked installs because the SPA lives in the same package as this
// shim — no cross-package require.resolve needed. We only set the var when the
// operator left it unset AND the bundled SPA actually exists, so an explicit
// TOPGUN_ADMIN_DIR always wins and a shim without the bundle falls through to
// the Rust default (./admin-dashboard/dist, the monorepo path).
if (env.TOPGUN_ADMIN_DIR === undefined) {
  const path = require('path');
  const fs = require('fs');
  const bundledAdminDir = path.join(__dirname, '..', 'admin-dist');
  if (fs.existsSync(path.join(bundledAdminDir, 'index.html'))) {
    env.TOPGUN_ADMIN_DIR = bundledAdminDir;
  }
}

const result = spawnSync(binaryPath, process.argv.slice(2), {
  stdio: 'inherit',
  env,
});

if (result.error) {
  process.stderr.write(`\n  @topgunbuild/server: failed to exec binary: ${result.error.message}\n\n`);
  process.exit(1);
}

process.exit(result.status !== null ? result.status : 1);
