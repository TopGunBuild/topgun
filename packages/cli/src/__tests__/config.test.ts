import path from 'path';
import fs from 'fs';
import os from 'os';
import { execSync, spawnSync } from 'child_process';
import { runCli, withTempEnv } from './test-utils';

const CLI_PATH = path.join(__dirname, '../../dist/topgun.js');
const CLI_CWD = path.join(__dirname, '../../');

describe('topgun config', () => {
  describe('--show', () => {
    it('should show warning when no .env file exists', () => {
      const envPath = path.join(CLI_CWD, '.env');
      const backupPath = envPath + '.backup-config-test';

      if (fs.existsSync(envPath)) {
        fs.copyFileSync(envPath, backupPath);
        fs.unlinkSync(envPath);
      }

      try {
        const output = execSync(`node ${CLI_PATH} config --show`, {
          encoding: 'utf8',
          cwd: CLI_CWD,
        });

        expect(output).toContain('TopGun Configuration');
        expect(output).toContain('No .env file found');
      } finally {
        if (fs.existsSync(backupPath)) {
          fs.copyFileSync(backupPath, envPath);
          fs.unlinkSync(backupPath);
        }
      }
    });

    it('should display STORAGE_BACKEND from .env file (AC8 regression guard)', () => {
      // Uses the STORAGE_BACKEND key
      const envContent = `
TOPGUN_NO_AUTH=1
STORAGE_BACKEND=redb
PORT=8080
METRICS_PORT=9091
`;

      withTempEnv(envContent, () => {
        const output = execSync(`node ${CLI_PATH} config --show`, {
          encoding: 'utf8',
          cwd: CLI_CWD,
        });

        expect(output).toContain('TopGun Configuration');
        expect(output).toContain('Storage');
        expect(output).toContain('redb');
        expect(output).toContain('Server');
        expect(output).toContain('Port: 8080');
      });
    });
  });

  describe('--storage', () => {
    it('should update .env with valid storage backend (redb)', () => {
      const envContent = `
TOPGUN_NO_AUTH=1
STORAGE_BACKEND=null
PORT=8080
`;

      withTempEnv(envContent, () => {
        const output = execSync(`node ${CLI_PATH} config --storage redb`, {
          encoding: 'utf8',
          cwd: CLI_CWD,
        });

        expect(output).toContain('Storage backend set to: redb');
        expect(output).toContain('Configuration updated');

        const envPath = path.join(CLI_CWD, '.env');
        const updatedContent = fs.readFileSync(envPath, 'utf8');
        // Must write the STORAGE_BACKEND key
        expect(updatedContent).toContain('STORAGE_BACKEND=redb');
      });
    });

    it('should reject the old "memory" value and accept "null" instead', () => {
      const envContent = `STORAGE_BACKEND=redb`;

      withTempEnv(envContent, () => {
        const result = runCli(['config', '--storage', 'memory']);

        // 'memory' is not a valid STORAGE_BACKEND value; null is the ephemeral equivalent
        expect(result.exitCode).toBe(1);
        const allOutput = result.stdout + result.stderr;
        expect(allOutput).toContain('Invalid storage type: memory');
        expect(allOutput).toContain('Valid options:');
        expect(allOutput).toContain('redb');
        expect(allOutput).not.toContain('postgres');
        expect(allOutput).toContain('null');
      });
    });

    it('refuses postgres: the backend has been removed', () => {
      const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), 'topgun-config-test-'));
      try {
        const result = spawnSync('node', [CLI_PATH, 'config', '--storage', 'postgres'], {
          encoding: 'utf8',
          cwd: tempDir,
        });
        const allOutput = (result.stdout || '') + (result.stderr || '');
        expect(result.status).toBe(1);
        expect(allOutput).toContain('The Postgres backend has been removed');
        expect(allOutput).toContain('There is no migration tool');
      } finally {
        fs.rmSync(tempDir, { recursive: true, force: true });
      }
    });

    it('should show error for invalid storage type', () => {
      const envContent = `STORAGE_BACKEND=redb`;

      withTempEnv(envContent, () => {
        const result = runCli(['config', '--storage', 'invalid']);

        expect(result.exitCode).toBe(1);
        const allOutput = result.stdout + result.stderr;
        expect(allOutput).toContain('Invalid storage type: invalid');
        expect(allOutput).toContain('Valid options:');
      });
    });
  });

  describe('help', () => {
    it('should show help when no options provided', () => {
      const output = execSync(`node ${CLI_PATH} config`, {
        encoding: 'utf8',
        cwd: CLI_CWD,
      });

      expect(output).toContain('TopGun Config');
      expect(output).toContain('Usage:');
      expect(output).toContain('--show');
      expect(output).toContain('--storage');
      // Updated help should mention new valid values
      expect(output).toContain('redb');
    });
  });
});
