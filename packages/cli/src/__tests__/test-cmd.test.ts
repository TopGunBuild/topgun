import { execSync } from 'child_process';
import path from 'path';
import { runCli } from './test-utils';

const CLI_PATH = path.join(__dirname, '../../dist/topgun.js');
const CLI_CWD = path.join(__dirname, '../../');

describe('topgun test', () => {
  it('should show error for unknown scope', () => {
    const result = runCli(['test', 'invalid-scope']);

    expect(result.exitCode).toBe(1);
    const allOutput = result.stdout + result.stderr;
    expect(allOutput).toContain('Unknown scope: invalid-scope');
    expect(allOutput).toContain('Available scopes:');
    expect(allOutput).toContain('core');
    expect(allOutput).toContain('client');
    expect(allOutput).toContain('server');
  });

  it('should show usage information with --help', () => {
    const output = execSync(`node ${CLI_PATH} --help`, {
      encoding: 'utf8',
      cwd: CLI_CWD,
    });

    expect(output).toContain('TopGun CLI');
    expect(output).toContain('test');
  });

  it('should note k6 test requirement for k6 scope', () => {
    const result = runCli(['test', 'k6:smoke']);

    // k6 tests will attempt to run, but we verify the note appears
    const allOutput = result.stdout + result.stderr;
    expect(allOutput).toContain('k6 tests require the server to be running');
  });
});
