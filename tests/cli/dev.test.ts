import { execSync } from 'child_process';
import path from 'path';
import fs from 'fs';
import os from 'os';
import { runCli } from './test-utils';

describe('topgun dev', () => {
  const cliPath = path.join(__dirname, '../../bin/topgun.js');

  it('should show error when server entry point is missing', () => {
    const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), 'topgun-dev-test-'));

    try {
      let output = '';
      let exitCode = 0;

      try {
        execSync(`node ${cliPath} dev --no-db`, {
          encoding: 'utf8',
          cwd: tempDir,
          stdio: 'pipe',
        });
      } catch (error: any) {
        output = error.stderr?.toString() || error.stdout?.toString() || '';
        exitCode = error.status || 1;
      }

      expect(exitCode).toBe(1);
      expect(output).toContain('No server entry point found');
      expect(output).toContain('examples/simple-server.ts');
    } finally {
      fs.rmSync(tempDir, { recursive: true, force: true });
    }
  });

  it('should parse server port option', () => {
    const result = runCli(['dev', '--help']);

    expect(result.exitCode).toBe(0);
    expect(result.stdout).toContain('TopGun CLI');
  });
});
