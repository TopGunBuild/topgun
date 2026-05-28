import path from 'path';
import { runCli } from './test-utils';

const CLI_PATH = path.join(__dirname, '../../dist/topgun.js');
const CLI_CWD = path.join(__dirname, '../../');

// Suppress unused import warning — CLI_PATH used in test file name display
void CLI_PATH;
void CLI_CWD;

describe('topgun docker', () => {
  describe('docker:start', () => {
    it('should show error for unknown profile', () => {
      const result = runCli(['docker:start', '--with', 'invalid-profile']);

      expect(result.exitCode).toBe(1);
      const allOutput = result.stdout + result.stderr;
      expect(allOutput).toContain('Unknown profile: invalid-profile');
      expect(allOutput).toContain('Available profiles:');
      expect(allOutput).toContain('admin');
      expect(allOutput).toContain('monitoring');
    });

    it('should show help for docker:start', () => {
      const result = runCli(['docker:start', '--help']);

      expect(result.exitCode).toBe(0);
      expect(result.stdout).toContain('Usage: topgun docker:start');
      expect(result.stdout).toContain('Start Docker services');
    });
  });

  describe('docker:stop', () => {
    it('should show help for docker:stop', () => {
      const result = runCli(['docker:stop', '--help']);

      expect(result.exitCode).toBe(0);
      expect(result.stdout).toContain('Usage: topgun docker:stop');
    });
  });

  describe('docker:status', () => {
    it('should show help for docker:status', () => {
      const result = runCli(['docker:status', '--help']);

      expect(result.exitCode).toBe(0);
      expect(result.stdout).toContain('Usage: topgun docker:status');
    });
  });

  describe('docker:logs', () => {
    it('should show all docker commands in --help', () => {
      const result = runCli(['--help']);
      expect(result.stdout).toContain('docker:start');
      expect(result.stdout).toContain('docker:stop');
      expect(result.stdout).toContain('docker:status');
      expect(result.stdout).toContain('docker:logs');
    });
  });
});
