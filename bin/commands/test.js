const chalk = require('chalk');
const { execSync } = require('child_process');

const TEST_SCOPES = {
  core: '@topgunbuild/core',
  client: '@topgunbuild/client',
  server: '@topgunbuild/server',
  react: '@topgunbuild/react',
  adapters: '@topgunbuild/adapters',
  native: '@topgunbuild/native',
  'adapter-better-auth': '@topgunbuild/adapter-better-auth',
  e2e: 'e2e',
  'k6:smoke': 'k6:smoke',
  'k6:throughput': 'k6:throughput',
  'k6:write': 'k6:write',
  'k6:connections': 'k6:connections',
  'k6:stress': 'k6:stress',
};

module.exports = async function test(scope, options) {
  console.log(chalk.bold('\n TopGun Test Runner\n'));

  try {
    if (!scope) {
      // Run all tests
      console.log(chalk.cyan('Running all tests...\n'));
      execSync('pnpm test', { stdio: 'inherit' });
    } else if (scope.startsWith('k6:')) {
      // k6 tests
      const testType = scope.split(':')[1];
      console.log(chalk.cyan(`Running k6 ${testType} test...\n`));
      console.log(chalk.gray('  Note: k6 tests require the server to be running\n'));
      execSync(`pnpm test:k6:${testType}`, { stdio: 'inherit' });
    } else if (scope === 'e2e') {
      // E2E tests
      console.log(chalk.cyan('Running E2E tests...\n'));
      execSync('pnpm test:e2e', { stdio: 'inherit' });
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
      Object.keys(TEST_SCOPES).forEach((s) => {
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
