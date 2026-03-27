const chalk = require('chalk');
const fs = require('fs');
const path = require('path');

module.exports = async function config(options) {
  const envPath = path.join(process.cwd(), '.env');

  if (options.show) {
    // Show current configuration
    console.log(chalk.bold('\n TopGun Configuration\n'));

    if (!fs.existsSync(envPath)) {
      console.log(chalk.yellow('  No .env file found. Run: npx topgun setup\n'));
      return;
    }

    const envContent = fs.readFileSync(envPath, 'utf8');
    const config = parseEnvFile(envContent);

    console.log(chalk.cyan('  Storage'));
    console.log(`    Mode: ${config.STORAGE_MODE || 'not set'}`);
    if (config.STORAGE_MODE === 'postgres') {
      console.log(`    Host: ${config.DB_HOST || 'localhost'}`);
      console.log(`    Port: ${config.DB_PORT || '5432'}`);
      console.log(`    Database: ${config.DB_NAME || 'topgun'}`);
    }

    console.log('');
    console.log(chalk.cyan('  Server'));
    console.log(`    Port: ${config.PORT || '8080'}`);
    console.log(`    Metrics Port: ${config.METRICS_PORT || '9091'}`);

    console.log('');
    console.log(chalk.cyan('  Logging'));
    console.log(`    Log Format: ${config.TOPGUN_LOG_FORMAT || 'human-readable'}`);
    console.log(`    Log Level: ${config.LOG_LEVEL || 'info'}`);

    console.log('');
    console.log(chalk.cyan('  Admin'));
    console.log(`    Username: ${config.TOPGUN_ADMIN_USERNAME || 'not set'}`);
    console.log(`    Password: ${config.TOPGUN_ADMIN_PASSWORD ? '********' : 'not set'}`);

    console.log('');
    return;
  }

  // Update configuration
  let updated = false;

  if (!fs.existsSync(envPath)) {
    console.log(chalk.yellow('No .env file found. Run: npx topgun setup'));
    process.exit(1);
  }

  let envContent = fs.readFileSync(envPath, 'utf8');

  if (options.storage) {
    const validStorages = ['postgres', 'memory'];
    if (!validStorages.includes(options.storage)) {
      console.error(chalk.red(`Invalid storage type: ${options.storage}`));
      console.log(chalk.gray(`Valid options: ${validStorages.join(', ')}`));
      process.exit(1);
    }

    envContent = updateEnvValue(envContent, 'STORAGE_MODE', options.storage);
    console.log(chalk.green(`  ✓ Storage mode set to: ${options.storage}`));
    updated = true;
  }

  if (updated) {
    fs.writeFileSync(envPath, envContent);
    console.log(chalk.green('\n Configuration updated.\n'));
  } else {
    console.log(chalk.bold('\n TopGun Config\n'));
    console.log(chalk.gray('  Usage:'));
    console.log(chalk.gray('    topgun config --show              Show current configuration'));
    console.log(chalk.gray('    topgun config --storage postgres  Set storage to PostgreSQL'));
    console.log(chalk.gray('    topgun config --storage memory    Set storage to in-memory'));
    console.log('');
  }
};

function parseEnvFile(content) {
  const config = {};
  const lines = content.split('\n');

  for (const line of lines) {
    const trimmed = line.trim();
    if (trimmed && !trimmed.startsWith('#')) {
      const [key, ...valueParts] = trimmed.split('=');
      if (key) {
        config[key] = valueParts.join('=');
      }
    }
  }

  return config;
}

function updateEnvValue(content, key, value) {
  const regex = new RegExp(`^${key}=.*$`, 'm');
  if (regex.test(content)) {
    return content.replace(regex, `${key}=${value}`);
  } else {
    // Add the key if it doesn't exist
    return content + `\n${key}=${value}`;
  }
}
