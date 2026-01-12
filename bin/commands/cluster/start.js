const chalk = require('chalk');
const { spawn, execSync } = require('child_process');
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

  // Build seed list (all nodes except self)
  const buildSeeds = (excludeNode) => {
    const seeds = [];
    for (let i = 0; i < nodeCount; i++) {
      if (i !== excludeNode) {
        seeds.push(`ws://localhost:${clusterPort + i}`);
      }
    }
    return seeds.join(',');
  };

  // Start each node
  for (let i = 0; i < nodeCount; i++) {
    const serverPort = basePort + i;
    const nodeClusterPort = clusterPort + i;
    const nodeId = `node-${i + 1}`;
    const seeds = buildSeeds(i);

    console.log(chalk.cyan(`  [${nodeId}] Starting on port ${serverPort} (cluster: ${nodeClusterPort})...`));

    const serverPath = path.join(process.cwd(), 'examples/simple-server.ts');

    const env = {
      ...process.env,
      SERVER_PORT: serverPort.toString(),
      PORT: serverPort.toString(),
      NODE_ID: nodeId,
      CLUSTER_ENABLED: 'true',
      CLUSTER_PORT: nodeClusterPort.toString(),
      CLUSTER_SEEDS: seeds,
    };

    const proc = spawn('npx', ['tsx', serverPath], {
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

  console.log(chalk.green(`\n  ✓ Cluster started with ${nodeCount} nodes`));
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
