import path from 'path';
import fs from 'fs';
import os from 'os';
import { execSync } from 'child_process';

const CLI_PATH = path.join(__dirname, '../../dist/topgun.js');

describe('topgun setup', () => {
  it('should create .env file with --yes mode (default redb storage)', () => {
    const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), 'topgun-setup-test-'));

    // Create stub paths to skip pnpm install and cargo build steps
    const nodeModulesPath = path.join(tempDir, 'node_modules');
    const binaryPath = path.join(tempDir, 'target/release/topgun-server');
    fs.mkdirSync(nodeModulesPath, { recursive: true });
    // Create the binary path as a directory stub (fs.existsSync returns true)
    fs.mkdirSync(binaryPath, { recursive: true });

    try {
      const output = execSync(`node ${CLI_PATH} setup --yes`, {
        encoding: 'utf8',
        cwd: tempDir,
        stdio: 'pipe',
      });

      expect(output).toContain('TopGun Setup Wizard');
      expect(output).toContain('Creating .env file');
      expect(output).toContain('.env created');
      expect(output).toContain('Setup complete');

      // Verify .env was created with the correct keys (AC6 regression guard)
      const envPath = path.join(tempDir, '.env');
      expect(fs.existsSync(envPath)).toBe(true);

      const envContent = fs.readFileSync(envPath, 'utf8');

      // Auth-gap fix: must have TOPGUN_NO_AUTH=1
      expect(envContent).toContain('TOPGUN_NO_AUTH=1');

      // Storage env-var fix: must use STORAGE_BACKEND, not STORAGE_MODE
      expect(envContent).toContain('STORAGE_BACKEND=redb');
      expect(envContent).not.toContain('STORAGE_MODE');

      expect(envContent).toContain('PORT=8080');
      expect(envContent).toContain('METRICS_PORT=9091');
    } finally {
      fs.rmSync(tempDir, { recursive: true, force: true });
    }
  });

  it('should write TOPGUN_NO_AUTH=1 and STORAGE_BACKEND for all non-postgres choices', () => {
    const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), 'topgun-setup-test-'));

    const nodeModulesPath = path.join(tempDir, 'node_modules');
    const binaryPath = path.join(tempDir, 'target/release/topgun-server');
    fs.mkdirSync(nodeModulesPath, { recursive: true });
    fs.mkdirSync(binaryPath, { recursive: true });

    try {
      execSync(`node ${CLI_PATH} setup --yes`, {
        encoding: 'utf8',
        cwd: tempDir,
        stdio: 'pipe',
      });

      const envPath = path.join(tempDir, '.env');
      const envContent = fs.readFileSync(envPath, 'utf8');

      // Must include the WHY comment explaining auth mode
      expect(envContent).toContain('# Auth');
      expect(envContent).toContain('TOPGUN_NO_AUTH=1');

      // Must include STORAGE_BACKEND with valid value
      expect(envContent).toContain('STORAGE_BACKEND=');

      // Must NOT include the dead STORAGE_MODE key
      expect(envContent).not.toContain('STORAGE_MODE');
    } finally {
      fs.rmSync(tempDir, { recursive: true, force: true });
    }
  });
});
