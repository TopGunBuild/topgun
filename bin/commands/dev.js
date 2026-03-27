const chalk = require('chalk');
const { spawn } = require('child_process');
const path = require('path');
const fs = require('fs');

// Load .env file if exists
const envPath = path.join(process.cwd(), '.env');
if (fs.existsSync(envPath)) {
  require('dotenv').config({ path: envPath });
}

module.exports = async function dev(options) {
  console.log(chalk.bold('\n TopGun Development Server\n'));

  const serverPort = options.port || process.env.PORT || '8080';

  // Display configuration
  console.log(chalk.gray(`  Port: ${serverPort}`));
  if (options.debug) {
    console.log(chalk.gray('  Debug: enabled (RUST_LOG=debug, TOPGUN_LOG_FORMAT=json)'));
  }
  console.log('');

  // Determine Rust server binary path
  const rustBinaryPath = path.join(process.cwd(), 'target/release/test-server');

  if (!fs.existsSync(rustBinaryPath)) {
    console.error(chalk.red('  Error: Rust server binary not found.'));
    console.log(chalk.yellow(`  Expected: ${rustBinaryPath}`));
    console.log(chalk.yellow('  Run: cargo build --release -p topgun-server --bin test-server'));
    process.exit(1);
  }

  // Start Rust server
  console.log(chalk.cyan('[server] Starting Rust server...\n'));

  const env = {
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

  // Handle graceful shutdown
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
};
