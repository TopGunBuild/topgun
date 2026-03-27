const chalk = require('chalk');
const { execSync } = require('child_process');

module.exports = async function clusterStatus() {
  console.log(chalk.bold('\n TopGun Cluster Status\n'));

  try {
    execSync('docker compose --profile cluster ps', { stdio: 'inherit' });
    console.log('');
  } catch (error) {
    console.error(chalk.red('\n  Failed to get cluster status\n'));
    console.log(chalk.yellow('  Hint: Make sure Docker is running'));
    console.log(chalk.gray('  Start the cluster with: npx topgun cluster:start\n'));
    process.exit(1);
  }
};
