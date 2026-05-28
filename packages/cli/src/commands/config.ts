import chalk from 'chalk';
import fs from 'fs';
import path from 'path';

interface ConfigOptions {
  show?: boolean;
  storage?: string;
}

interface EnvConfig {
  STORAGE_BACKEND?: string;
  DB_HOST?: string;
  DB_PORT?: string;
  DB_NAME?: string;
  PORT?: string;
  METRICS_PORT?: string;
  TOPGUN_LOG_FORMAT?: string;
  LOG_LEVEL?: string;
  TOPGUN_ADMIN_USERNAME?: string;
  TOPGUN_ADMIN_PASSWORD?: string;
  TOPGUN_NO_AUTH?: string;
  [key: string]: string | undefined;
}

// Valid storage values the Rust server recognises (STORAGE_BACKEND env var)
const VALID_STORAGES = ['redb', 'postgres', 'null'];

async function config(options: ConfigOptions) {
  const envPath = path.join(process.cwd(), '.env');

  if (options.show) {
    console.log(chalk.bold('\n TopGun Configuration\n'));

    if (!fs.existsSync(envPath)) {
      console.log(chalk.yellow('  No .env file found. Run: npx @topgunbuild/cli setup\n'));
      return;
    }

    const envContent = fs.readFileSync(envPath, 'utf8');
    const cfg = parseEnvFile(envContent);

    console.log(chalk.cyan('  Storage'));
    console.log(`    Backend: ${cfg.STORAGE_BACKEND || 'not set (server default: redb)'}`);
    if (cfg.STORAGE_BACKEND === 'postgres') {
      console.log(`    Host: ${cfg.DB_HOST || 'localhost'}`);
      console.log(`    Port: ${cfg.DB_PORT || '5432'}`);
      console.log(`    Database: ${cfg.DB_NAME || 'topgun'}`);
    }

    console.log('');
    console.log(chalk.cyan('  Auth'));
    console.log(
      `    Mode: ${cfg.TOPGUN_NO_AUTH === '1' || cfg.TOPGUN_NO_AUTH === 'true' ? 'no-auth (local dev)' : cfg.TOPGUN_NO_AUTH !== undefined ? `TOPGUN_NO_AUTH=${cfg.TOPGUN_NO_AUTH}` : 'JWT (JWT_SECRET required)'}`,
    );

    console.log('');
    console.log(chalk.cyan('  Server'));
    console.log(`    Port: ${cfg.PORT || '8080'}`);
    console.log(`    Metrics Port: ${cfg.METRICS_PORT || '9091'}`);

    console.log('');
    console.log(chalk.cyan('  Logging'));
    console.log(`    Log Format: ${cfg.TOPGUN_LOG_FORMAT || 'human-readable'}`);
    console.log(`    Log Level: ${cfg.LOG_LEVEL || 'info'}`);

    console.log('');
    console.log(chalk.cyan('  Admin'));
    console.log(`    Username: ${cfg.TOPGUN_ADMIN_USERNAME || 'not set'}`);
    console.log(`    Password: ${cfg.TOPGUN_ADMIN_PASSWORD ? '********' : 'not set'}`);

    console.log('');
    return;
  }

  // No actionable option — show help and exit without requiring a .env
  if (!options.storage) {
    console.log(chalk.bold('\n TopGun Config\n'));
    console.log(chalk.gray('  Usage:'));
    console.log(chalk.gray('    topgun config --show              Show current configuration'));
    console.log(
      chalk.gray('    topgun config --storage redb      Set storage to embedded redb (default)'),
    );
    console.log(chalk.gray('    topgun config --storage postgres  Set storage to PostgreSQL'));
    console.log(
      chalk.gray('    topgun config --storage null      Set storage to ephemeral (no persistence)'),
    );
    console.log('');
    return;
  }

  // Update configuration
  if (!fs.existsSync(envPath)) {
    console.log(chalk.yellow('No .env file found. Run: npx @topgunbuild/cli setup'));
    process.exit(1);
  }

  let envContent = fs.readFileSync(envPath, 'utf8');

  if (!VALID_STORAGES.includes(options.storage)) {
    console.error(chalk.red(`Invalid storage type: ${options.storage}`));
    console.log(chalk.gray(`Valid options: ${VALID_STORAGES.join(', ')}`));
    process.exit(1);
  }

  envContent = updateEnvValue(envContent, 'STORAGE_BACKEND', options.storage);
  console.log(chalk.green(`  ✓ Storage backend set to: ${options.storage}`));

  fs.writeFileSync(envPath, envContent);
  console.log(chalk.green('\n Configuration updated.\n'));
}

function parseEnvFile(content: string): EnvConfig {
  const cfg: EnvConfig = {};
  const lines = content.split('\n');

  for (const line of lines) {
    const trimmed = line.trim();
    if (trimmed && !trimmed.startsWith('#')) {
      const [key, ...valueParts] = trimmed.split('=');
      if (key) {
        cfg[key] = valueParts.join('=');
      }
    }
  }

  return cfg;
}

function updateEnvValue(content: string, key: string, value: string): string {
  const regex = new RegExp(`^${key}=.*$`, 'm');
  if (regex.test(content)) {
    return content.replace(regex, `${key}=${value}`);
  } else {
    // Add the key if it doesn't exist
    return content + `\n${key}=${value}`;
  }
}

export default config;
