import { execSync } from 'child_process';
import path from 'path';
import fs from 'fs';
import os from 'os';

describe('topgun setup', () => {
  const cliPath = path.join(__dirname, '../../bin/topgun.js');

  it('should create .env file with --yes mode', () => {
    const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), 'topgun-setup-test-'));

    // Create stub directories to skip pnpm install/build
    const nodeModulesPath = path.join(tempDir, 'node_modules');
    const distPath = path.join(tempDir, 'packages/server/dist');
    fs.mkdirSync(nodeModulesPath, { recursive: true });
    fs.mkdirSync(distPath, { recursive: true });

    try {
      const output = execSync(`node ${cliPath} setup --yes --storage sqlite`, {
        encoding: 'utf8',
        cwd: tempDir,
      });

      expect(output).toContain('TopGun Setup Wizard');
      expect(output).toContain('Creating .env file');
      expect(output).toContain('.env created');
      expect(output).toContain('Setup complete');

      // Verify .env was created
      const envPath = path.join(tempDir, '.env');
      expect(fs.existsSync(envPath)).toBe(true);

      const envContent = fs.readFileSync(envPath, 'utf8');
      expect(envContent).toContain('STORAGE_MODE=sqlite');
      expect(envContent).toContain('DB_PATH=./topgun.db');
      expect(envContent).toContain('SERVER_PORT=8080');
      expect(envContent).toContain('METRICS_PORT=9091');
    } finally {
      fs.rmSync(tempDir, { recursive: true, force: true });
    }
  });

  it('should verify generated .env content structure', () => {
    const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), 'topgun-setup-test-'));

    // Create stub directories to skip pnpm install/build
    const nodeModulesPath = path.join(tempDir, 'node_modules');
    const distPath = path.join(tempDir, 'packages/server/dist');
    fs.mkdirSync(nodeModulesPath, { recursive: true });
    fs.mkdirSync(distPath, { recursive: true });

    try {
      execSync(`node ${cliPath} setup --yes --storage memory`, {
        encoding: 'utf8',
        cwd: tempDir,
        stdio: 'pipe',
      });

      const envPath = path.join(tempDir, '.env');
      const envContent = fs.readFileSync(envPath, 'utf8');

      expect(envContent).toContain('# TopGun Configuration');
      expect(envContent).toContain('# Storage');
      expect(envContent).toContain('STORAGE_MODE=memory');
      expect(envContent).toContain('# Server');
      expect(envContent).toContain('SERVER_PORT=');
      expect(envContent).toContain('METRICS_PORT=');
      expect(envContent).toContain('# Debug');
    } finally {
      fs.rmSync(tempDir, { recursive: true, force: true });
    }
  });
});
