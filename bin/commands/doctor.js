const chalk = require('chalk');
const { execSync } = require('child_process');
const fs = require('fs');
const path = require('path');

const checks = [
  {
    name: 'Node.js',
    check: () => {
      const version = process.version;
      const major = parseInt(version.slice(1).split('.')[0]);
      return {
        pass: major >= 18,
        message: major >= 18
          ? `${version} (OK)`
          : `${version} (requires >= 18)`,
        fix: 'Install Node.js 18+ from https://nodejs.org',
      };
    },
  },
  {
    name: 'pnpm',
    check: () => {
      try {
        const version = execSync('pnpm --version', { encoding: 'utf8' }).trim();
        const [major, minor] = version.split('.').map(Number);
        const ok = major > 10 || (major === 10 && minor >= 13);
        return {
          pass: ok,
          message: ok ? `${version} (OK)` : `${version} (requires >= 10.13.1)`,
          fix: 'npm install -g pnpm@latest',
        };
      } catch {
        return {
          pass: false,
          message: 'Not installed',
          fix: 'npm install -g pnpm',
        };
      }
    },
  },
  {
    name: 'Docker',
    check: () => {
      try {
        const output = execSync('docker --version', { encoding: 'utf8' }).trim();
        const version = output.match(/Docker version ([^\s,]+)/)?.[1] || output;
        return {
          pass: true,
          message: `${version} (optional)`,
          optional: true,
        };
      } catch {
        return {
          pass: true,
          message: 'Not installed (optional for SQLite mode)',
          optional: true,
        };
      }
    },
  },
  {
    name: 'Port 8080',
    check: () => {
      try {
        // Cross-platform port check
        if (process.platform === 'win32') {
          execSync('netstat -ano | findstr :8080', { encoding: 'utf8' });
        } else {
          // Use lsof on macOS/Linux; netstat as fallback for minimal Linux systems
          execSync('lsof -i :8080 2>/dev/null || netstat -tln 2>/dev/null | grep :8080', { encoding: 'utf8' });
        }
        return {
          pass: false,
          message: 'In use',
          fix: 'Stop the process using port 8080 or use --port flag',
        };
      } catch {
        return {
          pass: true,
          message: 'Available',
        };
      }
    },
  },
  {
    name: 'Dependencies',
    check: () => {
      const nodeModules = path.join(process.cwd(), 'node_modules');
      if (fs.existsSync(nodeModules)) {
        return { pass: true, message: 'Installed' };
      }
      return {
        pass: false,
        message: 'Not installed',
        fix: 'pnpm install',
      };
    },
  },
  {
    name: 'Build',
    check: () => {
      const distPath = path.join(process.cwd(), 'packages/server/dist');
      if (fs.existsSync(distPath)) {
        return { pass: true, message: 'Built' };
      }
      return {
        pass: false,
        message: 'Not built',
        fix: 'pnpm build',
      };
    },
  },
  {
    name: '.env file',
    check: () => {
      const envPath = path.join(process.cwd(), '.env');
      if (fs.existsSync(envPath)) {
        return { pass: true, message: 'Present' };
      }
      return {
        pass: false,
        message: 'Not found',
        fix: 'npx topgun setup',
      };
    },
  },
];

module.exports = async function doctor() {
  console.log(chalk.bold('\n TopGun Environment Check\n'));

  let hasErrors = false;
  const fixes = [];

  for (const { name, check } of checks) {
    const result = check();
    const icon = result.pass
      ? chalk.green('✓')
      : result.optional
        ? chalk.yellow('○')
        : chalk.red('✗');

    console.log(`  ${icon} ${name}: ${result.message}`);

    if (!result.pass && !result.optional) {
      hasErrors = true;
      if (result.fix) {
        fixes.push({ name, fix: result.fix });
      }
    }
  }

  if (fixes.length > 0) {
    console.log(chalk.bold('\n To fix issues:\n'));
    for (const { name, fix } of fixes) {
      console.log(`  ${chalk.cyan(name)}: ${fix}`);
    }
  }

  if (hasErrors) {
    console.log(chalk.red('\n Some checks failed. Fix the issues above.\n'));
    process.exit(1);
  } else {
    console.log(chalk.green('\n All checks passed! Ready to run.\n'));
  }
};
