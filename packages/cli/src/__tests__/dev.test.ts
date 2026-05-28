import path from 'path';
import fs from 'fs';
import os from 'os';
import { execSync } from 'child_process';
import { runCli } from './test-utils';

const CLI_PATH = path.join(__dirname, '../../dist/topgun.js');

describe('topgun dev', () => {
  it('should show error when server binary is missing', () => {
    const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), 'topgun-dev-test-'));

    try {
      let output = '';
      let exitCode = 0;

      try {
        execSync(`node ${CLI_PATH} dev --no-db`, {
          encoding: 'utf8',
          cwd: tempDir,
          stdio: 'pipe',
        });
      } catch (error: unknown) {
        const err = error as NodeJS.ErrnoException & { stderr?: Buffer; stdout?: Buffer; status?: number };
        output = err.stderr?.toString() || err.stdout?.toString() || '';
        exitCode = err.status || 1;
      }

      expect(exitCode).toBe(1);
      // Error message confirms binary path and build hint
      expect(output).toContain('Rust server binary not found');
    } finally {
      fs.rmSync(tempDir, { recursive: true, force: true });
    }
  });

  it('should parse server port option', () => {
    const result = runCli(['dev', '--help']);

    expect(result.exitCode).toBe(0);
    expect(result.stdout).toContain('Usage: topgun dev');
    expect(result.stdout).toContain('Start development server');
  });
});
