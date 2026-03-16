const chalk = require('chalk');
const { spawn } = require('child_process');
const path = require('path');
const fs = require('fs');

module.exports = async function clusterStart(options) {
  const nodeCount = parseInt(options.nodes, 10) || 3;

  console.log(chalk.bold('\n TopGun Cluster Start\n'));
  console.log(chalk.gray(`  Starting ${nodeCount} nodes...\n`));

  const basePort = 8080;
  const clusterPort = 8180;
  const processes = [];

  // Load environment
  const envPath = path.join(process.cwd(), '.env');
  if (fs.existsSync(envPath)) {
    require('dotenv').config({ path: envPath });
  }

  // Check for Rust server binary
  const rustBinaryPath = path.join(process.cwd(), 'target/release/test-server');
  if (!fs.existsSync(rustBinaryPath)) {
    console.error(chalk.red('  Error: Rust server binary not found.'));
    console.log(chalk.yellow(`  Expected: ${rustBinaryPath}`));
    console.log(chalk.yellow('  Run: cargo build --release --bin test-server'));
    process.exit(1);
  }

  // Build peer list (all nodes except self)
  const buildPeers = (excludeNode) => {
    const peers = [];
    for (let i = 0; i < nodeCount; i++) {
      if (i !== excludeNode) {
        peers.push(`ws://localhost:${clusterPort + i}`);
      }
    }
    return peers.join(',');
  };

  // Start each node
  for (let i = 0; i < nodeCount; i++) {
    const serverPort = basePort + i;
    const nodeClusterPort = clusterPort + i;
    const nodeId = `node-${i + 1}`;
    const peers = buildPeers(i);

    console.log(chalk.cyan(`  [${nodeId}] Starting on port ${serverPort} (cluster: ${nodeClusterPort})...`));

    const env = {
      ...process.env,
      PORT: serverPort.toString(),
      NODE_ID: nodeId,
      TOPGUN_CLUSTER_PORT: nodeClusterPort.toString(),
      TOPGUN_PEERS: peers,
    };

    const proc = spawn(rustBinaryPath, [], {
      env,
      stdio: ['ignore', 'pipe', 'pipe'],
      detached: false,
    });

    proc.stdout.on('data', (data) => {
      const lines = data.toString().trim().split('\n');
      lines.forEach((line) => {
        if (line.trim()) {
          console.log(chalk.gray(`  [${nodeId}] ${line}`));
        }
      });
    });

    proc.stderr.on('data', (data) => {
      const lines = data.toString().trim().split('\n');
      lines.forEach((line) => {
        if (line.trim()) {
          console.log(chalk.red(`  [${nodeId}] ${line}`));
        }
      });
    });

    proc.on('close', (code) => {
      console.log(chalk.yellow(`  [${nodeId}] Exited with code ${code}`));
    });

    processes.push({ proc, nodeId });

    // Stagger starts to allow discovery
    await new Promise((r) => setTimeout(r, 500));
  }

  // Store PIDs for later cleanup
  const pidsPath = path.join(process.cwd(), '.cluster-pids');
  const pids = processes.map((p) => p.proc.pid).join(',');
  fs.writeFileSync(pidsPath, pids);

  console.log(chalk.green(`\n  Cluster started with ${nodeCount} nodes`));
  console.log(chalk.gray('  PIDs stored in .cluster-pids'));
  console.log(chalk.gray('  To stop: npx topgun cluster:stop\n'));

  // Keep process alive
  const shutdown = () => {
    console.log(chalk.yellow('\n  Shutting down cluster...'));
    processes.forEach(({ proc, nodeId }) => {
      console.log(chalk.gray(`  Stopping ${nodeId}...`));
      proc.kill('SIGTERM');
    });
    if (fs.existsSync(pidsPath)) {
      fs.unlinkSync(pidsPath);
    }
    process.exit(0);
  };

  process.on('SIGINT', shutdown);
  process.on('SIGTERM', shutdown);

  // Wait forever (until killed)
  await new Promise(() => {});
};
