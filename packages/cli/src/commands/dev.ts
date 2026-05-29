import chalk from 'chalk';
import { spawn } from 'child_process';
import path from 'path';
import fs from 'fs';

// Load .env file if present — this applies the stored TOPGUN_NO_AUTH and STORAGE_BACKEND
const envPath = path.join(process.cwd(), '.env');
if (fs.existsSync(envPath)) {
  // eslint-disable-next-line @typescript-eslint/no-require-imports
  require('dotenv').config({ path: envPath });
}

interface DevOptions {
  port?: string;
  debug?: boolean;
  db?: boolean;
}

async function dev(options: DevOptions) {
  console.log(chalk.bold('\n TopGun Development Server\n'));

  const serverPort = options.port || process.env.PORT || '8080';

  console.log(chalk.gray(`  Port: ${serverPort}`));
  if (options.debug) {
    console.log(chalk.gray('  Debug: enabled (RUST_LOG=debug, TOPGUN_LOG_FORMAT=json)'));
  }
  console.log('');

  // Binary resolution (Key Link L3 — monorepo path is always first):
  //   1. <cwd>/target/release/topgun-server — local cargo build (monorepo / contributors)
  //   2. @topgunbuild/server bin shim        — installed npm package (out-of-monorepo)
  // If neither is found, the error message names both remedies.
  const cwdBinaryPath = path.join(process.cwd(), 'target/release/topgun-server');

  let serverBinary: string | null = null;
  let isNodeShim = false;

  if (fs.existsSync(cwdBinaryPath)) {
    serverBinary = cwdBinaryPath;
  } else {
    // Fallback: resolve the bin shim from an installed @topgunbuild/server package.
    // Wrapped in try/catch — package resolution is best-effort (out-of-monorepo).
    try {
      // eslint-disable-next-line @typescript-eslint/no-require-imports
      const pkgJsonPath = require.resolve('@topgunbuild/server/package.json');
      const pkgRoot = path.dirname(pkgJsonPath);
      const shimPath = path.join(pkgRoot, 'bin', 'topgun-server.cjs');
      if (fs.existsSync(shimPath)) {
        serverBinary = shimPath;
        isNodeShim = true;
      }
    } catch (_) {
      // @topgunbuild/server not installed — fall through to error below
    }
  }

  if (!serverBinary) {
    console.error(chalk.red('  Error: Rust server binary not found.'));
    console.log(chalk.yellow(`  Expected: ${cwdBinaryPath}`));
    console.log(chalk.yellow('  Options:'));
    console.log(chalk.yellow('    Build from source: cargo build --release -p topgun-server --bin topgun-server'));
    console.log(chalk.yellow('    Or install prebuilt: npm install @topgunbuild/server'));
    process.exit(1);
  }

  console.log(chalk.cyan('[server] Starting Rust server...\n'));

  const env: NodeJS.ProcessEnv = {
    ...process.env,
    PORT: serverPort,
  };

  // Remove SERVER_PORT — the Rust server only reads PORT
  delete env.SERVER_PORT;

  if (options.debug) {
    env.RUST_LOG = 'debug';
    env.TOPGUN_LOG_FORMAT = 'json';
  }

  // When using the @topgunbuild/server bin shim, invoke it via Node.js.
  // When using the direct binary, invoke it directly.
  const [execCmd, execArgs]: [string, string[]] = isNodeShim
    ? [process.execPath, [serverBinary]]
    : [serverBinary, []];

  const server = spawn(execCmd, execArgs, {
    stdio: 'inherit',
    env,
  });

  const shutdown = () => {
    console.log(chalk.yellow('\n\nShutting down...'));
    server.kill('SIGINT');
    process.exit(0);
  };

  process.on('SIGINT', shutdown);
  process.on('SIGTERM', shutdown);

  server.on('close', (code) => {
    process.exit(code || 0);
  });

  server.on('error', (err) => {
    console.error(chalk.red(`[server] Error: ${err.message}`));
    process.exit(1);
  });
}

export default dev;
