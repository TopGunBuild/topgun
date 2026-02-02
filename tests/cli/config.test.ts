import { execSync } from 'child_process';
import path from 'path';
import fs from 'fs';
import { runCli, withTempEnv } from './test-utils';

describe('topgun config', () => {
  const cliPath = path.join(__dirname, '../../bin/topgun.js');

  describe('--show', () => {
    it('should show warning when no .env file exists', () => {
      const envPath = path.join(process.cwd(), '.env');
      const backupPath = envPath + '.backup-config-test';

      // Backup and remove .env temporarily
      if (fs.existsSync(envPath)) {
        fs.copyFileSync(envPath, backupPath);
        fs.unlinkSync(envPath);
      }

      try {
        const output = execSync(`node ${cliPath} config --show`, {
          encoding: 'utf8',
          cwd: path.join(__dirname, '../..'),
        });

        expect(output).toContain('TopGun Configuration');
        expect(output).toContain('No .env file found');
        expect(output).toContain('npx topgun setup');
      } finally {
        // Restore .env
        if (fs.existsSync(backupPath)) {
          fs.copyFileSync(backupPath, envPath);
          fs.unlinkSync(backupPath);
        }
      }
    });

    it('should display configuration from .env file', () => {
      const envContent = `
STORAGE_MODE=sqlite
DB_PATH=./topgun.db
SERVER_PORT=8080
METRICS_PORT=9091
`;

      withTempEnv(envContent, () => {
        const output = execSync(`node ${cliPath} config --show`, {
          encoding: 'utf8',
          cwd: path.join(__dirname, '../..'),
        });

        expect(output).toContain('TopGun Configuration');
        expect(output).toContain('Storage');
        expect(output).toContain('Mode: sqlite');
        expect(output).toContain('Server');
        expect(output).toContain('Port: 8080');
      });
    });
  });

  describe('--storage', () => {
    it('should update .env with valid storage mode', () => {
      const envContent = `
STORAGE_MODE=memory
SERVER_PORT=8080
`;

      withTempEnv(envContent, () => {
        const output = execSync(`node ${cliPath} config --storage sqlite`, {
          encoding: 'utf8',
          cwd: path.join(__dirname, '../..'),
        });

        expect(output).toContain('Storage mode set to: sqlite');
        expect(output).toContain('Configuration updated');

        // Verify .env was updated
        const envPath = path.join(process.cwd(), '.env');
        const updatedContent = fs.readFileSync(envPath, 'utf8');
        expect(updatedContent).toContain('STORAGE_MODE=sqlite');
      });
    });

    it('should show error for invalid storage mode', () => {
      const envContent = `STORAGE_MODE=memory`;

      withTempEnv(envContent, () => {
        const result = runCli(['config', '--storage', 'invalid']);

        expect(result.exitCode).toBe(1);
        expect(result.stderr).toContain('Invalid storage type: invalid');
        expect(result.stderr).toContain('Valid options: sqlite, postgres, memory');
      });
    });
  });

  describe('help', () => {
    it('should show help when no options provided', () => {
      const output = execSync(`node ${cliPath} config`, {
        encoding: 'utf8',
        cwd: path.join(__dirname, '../..'),
      });

      expect(output).toContain('TopGun Config');
      expect(output).toContain('Usage:');
      expect(output).toContain('--show');
      expect(output).toContain('--storage');
    });
  });
});
