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

  // The server binary lives at <cwd>/target/release/topgun-server.
  // This is a monorepo-internal path; out-of-monorepo usage requires a
  // prebuilt binary placed there (see TODO-365 for prebuilt distribution).
  const rustBinaryPath = path.join(process.cwd(), 'target/release/topgun-server');

  if (!fs.existsSync(rustBinaryPath)) {
    console.error(chalk.red('  Error: Rust server binary not found.'));
    console.log(chalk.yellow(`  Expected: ${rustBinaryPath}`));
    console.log(chalk.yellow('  Run: cargo build --release -p topgun-server --bin topgun-server'));
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

  const server = spawn(rustBinaryPath, [], {
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
