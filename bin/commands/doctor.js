const chalk = require('chalk');
const { execSync } = require('child_process');
const fs = require('fs');
const path = require('path');

// Load .env so PORT can be read
const envPath = path.join(process.cwd(), '.env');
if (fs.existsSync(envPath)) {
  require('dotenv').config({ path: envPath });
}

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
    name: 'Rust toolchain (cargo)',
    check: () => {
      try {
        const version = execSync('cargo --version', { encoding: 'utf8' }).trim();
        return {
          pass: true,
          message: `${version} (OK)`,
        };
      } catch {
        return {
          pass: false,
          message: 'Not installed — required to build the server',
          fix: 'Install Rust from https://rustup.rs',
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
          message: 'Not installed (optional — required for PostgreSQL and cluster)',
          optional: true,
        };
      }
    },
  },
  {
    name: `Port ${process.env.PORT || '8080'}`,
    check: () => {
      const port = process.env.PORT || '8080';
      try {
        // Cross-platform port check
        if (process.platform === 'win32') {
          execSync(`netstat -ano | findstr :${port}`, { encoding: 'utf8' });
        } else {
          execSync(`lsof -i :${port} 2>/dev/null || netstat -tln 2>/dev/null | grep :${port}`, { encoding: 'utf8' });
        }
        return {
          pass: false,
          message: 'In use',
          fix: `Stop the process using port ${port} or set PORT= in .env`,
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
    name: 'Rust Server Binary',
    check: () => {
      const binaryPath = path.join(process.cwd(), 'target/release/test-server');
      if (fs.existsSync(binaryPath)) {
        return { pass: true, message: 'Built' };
      }
      return {
        pass: false,
        message: 'Not built',
        fix: 'cargo build --release -p topgun-server --bin test-server',
      };
    },
  },
  {
    name: '.env file',
    check: () => {
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
