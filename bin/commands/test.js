const chalk = require('chalk');
const { execSync } = require('child_process');

const TEST_SCOPES = {
  core: '@topgunbuild/core',
  client: '@topgunbuild/client',
  react: '@topgunbuild/react',
  adapters: '@topgunbuild/adapters',
  'adapter-better-auth': '@topgunbuild/adapter-better-auth',
  'mcp-server': '@topgunbuild/mcp-server',
  schema: '@topgunbuild/schema',
  'k6:smoke': 'k6:smoke',
  'k6:throughput': 'k6:throughput',
  'k6:write': 'k6:write',
  'k6:connections': 'k6:connections',
  'k6:stress': 'k6:stress',
  'k6:cluster': 'k6:cluster',
  'k6:cluster:throughput': 'k6:cluster:throughput',
  'k6:cluster:failover': 'k6:cluster:failover',
  'k6:cluster:rebalance': 'k6:cluster:rebalance',
};

module.exports = async function test(scope, options) {
  console.log(chalk.bold('\n TopGun Test Runner\n'));

  try {
    if (!scope) {
      // Run all tests
      console.log(chalk.cyan('Running all tests...\n'));
      execSync('pnpm test', { stdio: 'inherit' });
    } else if (scope === 'sim') {
      // Simulation tests (cargo, deterministic distributed behavior)
      console.log(chalk.cyan('Running simulation tests...\n'));
      execSync('pnpm test:sim', { stdio: 'inherit' });
    } else if (scope === 'server') {
      // Rust cargo tests for the server package
      console.log(chalk.cyan('Running Rust server tests...\n'));
      if (process.platform === 'darwin') {
        const sdkRoot = execSync('/usr/bin/xcrun --sdk macosx --show-sdk-path', { encoding: 'utf8' }).trim();
        execSync(`SDKROOT=${sdkRoot} cargo test --release -p topgun-server`, { stdio: 'inherit' });
      } else {
        execSync('cargo test --release -p topgun-server', { stdio: 'inherit' });
      }
    } else if (scope === 'e2e' || scope === 'integration-rust') {
      // Integration tests (TS client -> Rust server)
      console.log(chalk.cyan('Running integration tests (TS -> Rust)...\n'));
      execSync('pnpm test:integration-rust', { stdio: 'inherit' });
    } else if (scope.startsWith('k6:')) {
      // k6 tests
      const testType = scope.replace('k6:', '');
      console.log(chalk.cyan(`Running k6 ${testType} test...\n`));
      console.log(chalk.gray('  Note: k6 tests require the server to be running\n'));
      execSync(`pnpm test:k6:${testType}`, { stdio: 'inherit' });
    } else if (TEST_SCOPES[scope]) {
      // Package-specific tests
      const packageName = TEST_SCOPES[scope];
      console.log(chalk.cyan(`Running tests for ${packageName}...\n`));

      const coverageFlag = options.coverage ? ' -- --coverage' : '';
      execSync(`pnpm --filter ${packageName} test${coverageFlag}`, {
        stdio: 'inherit',
      });
    } else {
      console.error(chalk.red(`Unknown scope: ${scope}`));
      console.log(chalk.gray('\nAvailable scopes:'));
      const allScopes = ['sim', 'server', 'integration-rust', 'e2e', ...Object.keys(TEST_SCOPES)];
      allScopes.forEach((s) => {
        console.log(chalk.gray(`  - ${s}`));
      });
      process.exit(1);
    }

    console.log(chalk.green('\n Tests passed!\n'));
  } catch (error) {
    console.error(chalk.red('\n Tests failed!\n'));
    process.exit(1);
  }
};
