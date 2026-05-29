'use strict';

// Resolve and start the topgun-server binary.
// Resolution order (preserves contributor experience — C5 / Key Link L3):
//   1. <cwd>/target/release/topgun-server   — local cargo build (monorepo, contributors)
//   2. @topgunbuild/server bin shim           — installed npm package (out-of-monorepo / prebuilt)
//   3. cargo run --bin topgun-server --release — fallback build-on-demand for contributors

const { spawnSync } = require('child_process');
const path = require('path');
const fs = require('fs');

const cwdBinary = path.join(process.cwd(), 'target', 'release', 'topgun-server');

function tryResolvePackageBinary() {
  try {
    const pkgJsonPath = require.resolve('@topgunbuild/server/package.json');
    const pkgRoot = path.dirname(pkgJsonPath);
    const shimPath = path.join(pkgRoot, 'bin', 'topgun-server.cjs');
    if (fs.existsSync(shimPath)) {
      return shimPath;
    }
  } catch (_) {
    // @topgunbuild/server not installed — not an error
  }
  return null;
}

let cmd, args;

if (fs.existsSync(cwdBinary)) {
  // Path 1: local build artifact
  cmd = cwdBinary;
  args = [];
} else {
  const shimPath = tryResolvePackageBinary();
  if (shimPath) {
    // Path 2: installed @topgunbuild/server package
    cmd = process.execPath;  // node
    args = [shimPath];
  } else {
    // Path 3: build on demand via cargo
    cmd = 'cargo';
    args = ['run', '--bin', 'topgun-server', '--release'];
  }
}

const result = spawnSync(cmd, args.concat(process.argv.slice(2)), {
  stdio: 'inherit',
  env: process.env,
});

if (result.error) {
  process.stderr.write(`start-server: failed to exec: ${result.error.message}\n`);
  process.exit(1);
}

process.exit(result.status !== null ? result.status : 1);
