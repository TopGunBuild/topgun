import { execSync } from 'child_process';
import path from 'path';
import { runCli } from './test-utils';

describe('topgun test', () => {
  const cliPath = path.join(__dirname, '../../bin/topgun.js');

  it('should show error for unknown scope', () => {
    const result = runCli(['test', 'invalid-scope']);

    expect(result.exitCode).toBe(1);
    expect(result.stderr).toContain('Unknown scope: invalid-scope');
    expect(result.stderr).toContain('Available scopes:');
    expect(result.stderr).toContain('core');
    expect(result.stderr).toContain('client');
    expect(result.stderr).toContain('server');
  });

  it('should show usage information with --help', () => {
    const output = execSync(`node ${cliPath} --help`, {
      encoding: 'utf8',
      cwd: path.join(__dirname, '../..'),
    });

    expect(output).toContain('TopGun CLI');
    expect(output).toContain('test');
  });

  it('should show k6 test note for k6 scope', () => {
    const result = runCli(['test', 'k6:smoke']);

    // k6 tests will try to run, so we just verify the message appears
    const allOutput = result.stdout + result.stderr;
    expect(allOutput).toContain('k6 tests require the server to be running');
  });
});
