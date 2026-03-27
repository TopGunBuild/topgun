const chalk = require('chalk');
const { execSync } = require('child_process');

module.exports = async function clusterStop() {
  console.log(chalk.bold('\n TopGun Cluster Stop\n'));

  try {
    execSync('docker compose --profile cluster down', { stdio: 'inherit' });
    console.log(chalk.green('\n  Cluster stopped.\n'));
  } catch (error) {
    console.error(chalk.red('\n  Failed to stop cluster\n'));
    console.log(chalk.yellow('  Hint: Make sure Docker is running'));
    process.exit(1);
  }
};
