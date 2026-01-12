const chalk = require('chalk');
const fs = require('fs');
const path = require('path');

module.exports = async function clusterStop() {
  console.log(chalk.bold('\n TopGun Cluster Stop\n'));

  const pidsPath = path.join(process.cwd(), '.cluster-pids');

  if (!fs.existsSync(pidsPath)) {
    console.log(chalk.yellow('  No running cluster found.\n'));
    return;
  }

  const pidsContent = fs.readFileSync(pidsPath, 'utf8').trim();
  const pids = pidsContent.split(',').filter(Boolean);

  if (pids.length === 0) {
    console.log(chalk.yellow('  No PIDs found in .cluster-pids\n'));
    fs.unlinkSync(pidsPath);
    return;
  }

  console.log(chalk.gray(`  Stopping ${pids.length} processes...`));

  let stopped = 0;
  for (const pid of pids) {
    try {
      process.kill(parseInt(pid, 10), 'SIGTERM');
      stopped++;
      console.log(chalk.gray(`  Stopped PID ${pid}`));
    } catch (error) {
      // Process may already be dead
      if (error.code !== 'ESRCH') {
        console.log(chalk.yellow(`  Could not stop PID ${pid}: ${error.message}`));
      }
    }
  }

  fs.unlinkSync(pidsPath);

  console.log(chalk.green(`\n  ✓ Stopped ${stopped} cluster node(s)\n`));
};
