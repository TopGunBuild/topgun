const chalk = require('chalk');
const { spawn, execSync } = require('child_process');
const path = require('path');
const fs = require('fs');

// Load .env file if exists
const envPath = path.join(process.cwd(), '.env');
if (fs.existsSync(envPath)) {
  require('dotenv').config({ path: envPath });
}

module.exports = async function dev(options) {
  console.log(chalk.bold('\n TopGun Development Server\n'));

  const storageMode = process.env.STORAGE_MODE || 'sqlite';
  const serverPort = options.port || process.env.SERVER_PORT || '8080';

  // Check if tsx is available, otherwise use ts-node
  let runner = 'tsx';
  let runnerArgs = ['watch'];

  try {
    execSync('npx tsx --version', { stdio: 'ignore' });
  } catch {
    runner = 'ts-node';
    runnerArgs = ['-r', 'tsconfig-paths/register', '--project', './examples/tsconfig.json'];
    console.log(chalk.yellow('  Note: Using ts-node (tsx not found)'));
  }

  // Start PostgreSQL if needed
  if (storageMode === 'postgres' && !options.noDb) {
    console.log(chalk.cyan('[postgres] Starting...'));
    try {
      const docker = spawn('docker', ['compose', 'up', '-d', 'postgres'], {
        stdio: 'inherit',
      });
      await new Promise((resolve) => docker.on('close', resolve));
      console.log(chalk.green('[postgres] Started'));
    } catch (error) {
      console.error(chalk.red('[postgres] Failed to start. Is Docker running?'));
    }
  }

  // Display configuration
  console.log(chalk.gray(`  Storage: ${storageMode}`));
  console.log(chalk.gray(`  Port: ${serverPort}`));
  console.log('');

  // Determine server entry point
  const serverTsPath = path.join(process.cwd(), 'examples/simple-server.ts');
  const serverJsPath = path.join(process.cwd(), 'packages/server/dist/index.js');

  let serverPath;
  let useNode = false;

  if (fs.existsSync(serverTsPath)) {
    serverPath = serverTsPath;
  } else if (fs.existsSync(serverJsPath)) {
    serverPath = serverJsPath;
    useNode = true;
  } else {
    console.error(chalk.red('  Error: No server entry point found.'));
    console.log(chalk.yellow('  Expected: examples/simple-server.ts or packages/server/dist/index.js'));
    console.log(chalk.yellow('  Run: pnpm build'));
    process.exit(1);
  }

  // Start server with watch mode
  console.log(chalk.cyan('[server] Starting with live reload...\n'));

  const env = {
    ...process.env,
    STORAGE_MODE: storageMode,
    SERVER_PORT: serverPort,
    PORT: serverPort,
  };

  let server;
  if (useNode) {
    server = spawn('node', [serverPath], {
      stdio: 'inherit',
      env,
    });
  } else {
    server = spawn('npx', [runner, ...runnerArgs, serverPath], {
      stdio: 'inherit',
      env,
    });
  }

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
