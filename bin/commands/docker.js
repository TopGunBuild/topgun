const chalk = require('chalk');
const { execSync } = require('child_process');

const PROFILES = {
  admin: 'Admin UI at :3001',
  monitoring: 'Prometheus (:9090) + Grafana (:3000)',
  dbtools: 'DbGate database browser at :3002',
  k6: 'k6 load testing',
  cluster: '3-node cluster for testing',
  all: 'All services',
};

async function start(options) {
  const profiles = options.with ? options.with.split(',') : [];

  console.log(chalk.bold('\n Docker Compose Start\n'));

  let cmd = 'docker compose';

  for (const profile of profiles) {
    if (!PROFILES[profile]) {
      console.error(chalk.red(`Unknown profile: ${profile}`));
      console.log(chalk.gray('\nAvailable profiles:'));
      Object.entries(PROFILES).forEach(([name, desc]) => {
        console.log(chalk.gray(`  - ${name}: ${desc}`));
      });
      process.exit(1);
    }
    cmd += ` --profile ${profile}`;
  }

  cmd += ' up -d';

  console.log(chalk.gray(`Running: ${cmd}\n`));

  try {
    execSync(cmd, { stdio: 'inherit' });

    console.log(chalk.green('\n Services started!\n'));

    // Show what's running
    console.log(chalk.white('  Base services:'));
    console.log(chalk.cyan('    - Server: http://localhost:8080'));
    console.log(chalk.cyan('    - PostgreSQL: localhost:5432'));

    for (const profile of profiles) {
      if (profile === 'admin' || profile === 'all') {
        console.log(chalk.white('\n  Admin:'));
        console.log(chalk.cyan('    - Admin UI: http://localhost:3001'));
      }
      if (profile === 'monitoring' || profile === 'all') {
        console.log(chalk.white('\n  Monitoring:'));
        console.log(chalk.cyan('    - Prometheus: http://localhost:9090'));
        console.log(chalk.cyan('    - Grafana: http://localhost:3000 (admin/admin)'));
      }
      if (profile === 'dbtools' || profile === 'all') {
        console.log(chalk.white('\n  Database Tools:'));
        console.log(chalk.cyan('    - DbGate: http://localhost:3002'));
      }
      if (profile === 'cluster') {
        console.log(chalk.white('\n  Cluster:'));
        console.log(chalk.cyan('    - Node 1: http://localhost:10001'));
        console.log(chalk.cyan('    - Node 2: http://localhost:10002'));
        console.log(chalk.cyan('    - Node 3: http://localhost:10003'));
      }
    }

    console.log('');
  } catch (error) {
    console.error(chalk.red('\n Failed to start services\n'));
    process.exit(1);
  }
}

async function stop() {
  console.log(chalk.bold('\n Stopping all Docker services...\n'));
  execSync('docker compose --profile all down', { stdio: 'inherit' });
  console.log(chalk.green('\n All services stopped.\n'));
}

async function status() {
  console.log(chalk.bold('\n Docker Compose Status\n'));
  execSync('docker compose ps -a', { stdio: 'inherit' });
}

async function logs(options) {
  const service = options.service || '';
  const follow = options.follow ? '-f' : '';
  execSync(`docker compose logs ${follow} ${service}`, { stdio: 'inherit' });
}

module.exports = {
  start,
  stop,
  status,
  logs,
};
