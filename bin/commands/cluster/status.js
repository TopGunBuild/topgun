const chalk = require('chalk');
const fs = require('fs');
const path = require('path');
const http = require('http');

module.exports = async function clusterStatus() {
  console.log(chalk.bold('\n TopGun Cluster Status\n'));

  const pidsPath = path.join(process.cwd(), '.cluster-pids');

  // Check for PIDs file
  if (!fs.existsSync(pidsPath)) {
    console.log(chalk.yellow('  No cluster PID file found.'));
    console.log(chalk.gray('  Start a cluster with: npx topgun cluster:start\n'));
    return;
  }

  const pidsContent = fs.readFileSync(pidsPath, 'utf8').trim();
  const pids = pidsContent.split(',').filter(Boolean);

  if (pids.length === 0) {
    console.log(chalk.yellow('  No PIDs in cluster file.\n'));
    return;
  }

  console.log(chalk.cyan(`  Cluster nodes: ${pids.length}\n`));

  // Check each process
  const basePort = 8080;
  let running = 0;
  let stopped = 0;

  for (let i = 0; i < pids.length; i++) {
    const pid = parseInt(pids[i], 10);
    const port = basePort + i;
    const nodeId = `node-${i + 1}`;

    let isRunning = false;
    try {
      process.kill(pid, 0); // Signal 0 checks if process exists
      isRunning = true;
      running++;
    } catch {
      stopped++;
    }

    const status = isRunning
      ? chalk.green('running')
      : chalk.red('stopped');

    console.log(`  ${nodeId}: PID ${pid} - ${status} (port ${port})`);
  }

  console.log('');
  console.log(chalk.gray(`  Summary: ${running} running, ${stopped} stopped`));

  if (stopped > 0 && running > 0) {
    console.log(chalk.yellow('  Warning: Some nodes are down. Consider restarting the cluster.'));
  }

  console.log('');
};
