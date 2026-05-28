import chalk from 'chalk';
import { execSync } from 'child_process';

async function clusterStop() {
  console.log(chalk.bold('\n TopGun Cluster Stop\n'));

  try {
    execSync('docker compose --profile cluster down', { stdio: 'inherit' });
    console.log(chalk.green('\n  Cluster stopped.\n'));
  } catch {
    console.error(chalk.red('\n  Failed to stop cluster\n'));
    console.log(chalk.yellow('  Hint: Make sure Docker is running'));
    process.exit(1);
  }
}

export default clusterStop;
