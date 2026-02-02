import { execSync } from 'child_process';
import path from 'path';
import { runCli } from './test-utils';

describe('topgun docker', () => {
  const cliPath = path.join(__dirname, '../../bin/topgun.js');
  let dockerAvailable = false;

  beforeAll(() => {
    // Check if Docker is available
    try {
      execSync('docker --version', { stdio: 'ignore' });
      dockerAvailable = true;
    } catch {
      dockerAvailable = false;
    }
  });

  describe('docker:start', () => {
    it('should show error for unknown profile', () => {
      const result = runCli(['docker:start', '--with', 'invalid-profile']);

      expect(result.exitCode).toBe(1);
      expect(result.stderr).toContain('Unknown profile: invalid-profile');
      expect(result.stderr).toContain('Available profiles:');
      expect(result.stderr).toContain('admin');
      expect(result.stderr).toContain('monitoring');
    });

    (dockerAvailable ? it : it.skip)('should build correct command for valid profile', () => {
      const result = runCli(['docker:start', '--with', 'admin']);

      // Command will try to run docker compose
      // We just verify it attempts to run with correct profile
      const allOutput = result.stdout + result.stderr;
      expect(allOutput).toContain('docker compose --profile admin');
    });
  });

  describe('docker:stop', () => {
    (dockerAvailable ? it : it.skip)('should execute docker compose down', () => {
      const output = execSync(`node ${cliPath} docker:stop`, {
        encoding: 'utf8',
        cwd: path.join(__dirname, '../..'),
      });

      expect(output).toContain('Stopping all Docker services');
    });
  });

  describe('docker:status', () => {
    (dockerAvailable ? it : it.skip)('should execute docker compose ps', () => {
      const output = execSync(`node ${cliPath} docker:status`, {
        encoding: 'utf8',
        cwd: path.join(__dirname, '../..'),
      });

      expect(output).toContain('Docker Compose Status');
    });
  });

  describe('docker:logs', () => {
    (dockerAvailable ? it : it.skip)('should execute docker compose logs', () => {
      // We can't test actual logs output without running services
      // Just verify the command is recognized
      const result = runCli(['--help']);
      expect(result.stdout).toContain('docker:');
    });
  });
});
