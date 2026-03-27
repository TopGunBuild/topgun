const chalk = require('chalk');
const { execSync } = require('child_process');

module.exports = async function clusterStart() {
  console.log(chalk.bold('\n TopGun Cluster Start\n'));

  console.log(chalk.gray('  Starting 3-node cluster via Docker Compose...\n'));

  try {
    execSync('docker compose --profile cluster up -d', { stdio: 'inherit' });

    console.log(chalk.green('\n  Cluster started (3 nodes)\n'));
    console.log(chalk.white('  Nodes (mapped via Docker):'));
    console.log(chalk.cyan('    - node-1: http://localhost:10001'));
    console.log(chalk.cyan('    - node-2: http://localhost:10002'));
    console.log(chalk.cyan('    - node-3: http://localhost:10003'));
    console.log('');
    console.log(chalk.gray('  To stop: npx topgun cluster:stop'));
    console.log(chalk.gray('  To view status: npx topgun cluster:status\n'));
  } catch (error) {
    console.error(chalk.red('\n  Failed to start cluster\n'));
    console.log(chalk.yellow('  Hint: Make sure Docker is running and docker-compose.yml has the cluster profile'));
    process.exit(1);
  }
};
