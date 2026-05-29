'use strict';

// Postinstall: verify the host's per-platform package installed and make
// the binary executable. Warns on mismatch but does NOT hard-fail — per-platform
// packages are optionalDependencies by design (best-effort install).
// MUST NOT download anything: binaries ship inside the per-platform packages.

const PLATFORM_PACKAGES = {
  'darwin-arm64': '@topgunbuild/server-darwin-arm64',
  'linux-x64':    '@topgunbuild/server-linux-x64',
};

const key = `${process.platform}-${process.arch}`;
const pkgName = PLATFORM_PACKAGES[key];

if (!pkgName) {
  // Unsupported platform — warn, but don't block the install.
  process.stdout.write(
    `  @topgunbuild/server: platform "${key}" has no prebuilt binary.\n` +
    `  Build from source: cargo build --release --bin topgun-server\n`
  );
  process.exit(0);
}

let pkgJsonPath;
try {
  pkgJsonPath = require.resolve(`${pkgName}/package.json`);
} catch (_) {
  // optionalDependency not installed — this is expected on unsupported platforms
  // or when the registry copy is not yet available.
  process.stdout.write(
    `  @topgunbuild/server: optional package "${pkgName}" was not installed (platform "${key}").\n` +
    `  If you need the prebuilt binary, ensure your registry has "${pkgName}@2.0.0".\n`
  );
  process.exit(0);
}

const path = require('path');
const fs = require('fs');
const pkgRoot = path.dirname(pkgJsonPath);
const binaryPath = path.join(pkgRoot, 'bin', 'topgun-server');

if (!fs.existsSync(binaryPath)) {
  process.stdout.write(
    `  @topgunbuild/server: binary not found at ${binaryPath}.\n` +
    `  The binary is a build artifact — run: bash scripts/build-server-binaries.sh\n`
  );
  process.exit(0);
}

// Make the binary executable on POSIX (chmod 0755).
// On Windows, skip — .exe permissions are handled by the OS.
if (process.platform !== 'win32') {
  try {
    fs.chmodSync(binaryPath, 0o755);
    process.stdout.write(`  @topgunbuild/server: binary ready at ${binaryPath}\n`);
  } catch (err) {
    process.stdout.write(
      `  @topgunbuild/server: warning — could not chmod binary: ${err.message}\n`
    );
  }
}
