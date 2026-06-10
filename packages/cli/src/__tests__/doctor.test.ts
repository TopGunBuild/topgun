import { execSync } from 'child_process';
import path from 'path';
import { runCli } from './test-utils';

const CLI_PATH = path.join(__dirname, '../../dist/topgun.js');
// Run from the repo root so doctor finds the server binary + .env
const CLI_CWD = path.join(__dirname, '../../../..');

describe('topgun doctor', () => {
  // `doctor` intentionally exits non-zero when an optional dependency is absent
  // (e.g. the Rust toolchain, which the Node CI runner does not install), so the
  // raw execSync would throw before any assertion. runCli captures stdout/stderr
  // regardless of exit code; the environment-check report still prints in full.
  it('should run doctor command', () => {
    const { stdout, stderr } = runCli(['doctor'], CLI_CWD);
    const out = stdout + stderr;

    expect(out).toContain('TopGun Environment Check');
    expect(out).toContain('Node.js');
    expect(out).toContain('pnpm');
  });

  it('should detect Node.js version', () => {
    const { stdout, stderr } = runCli(['doctor'], CLI_CWD);
    const out = stdout + stderr;

    expect(out).toMatch(/Node\.js.*v\d+/);
    expect(out).toContain('✓');
  });

  it('should check for dependencies', () => {
    const { stdout, stderr } = runCli(['doctor'], CLI_CWD);
    const out = stdout + stderr;

    expect(out).toContain('Dependencies');
  });
});

describe('topgun --version', () => {
  it('should display version from @topgunbuild/cli package.json', () => {
    const output = execSync(`node ${CLI_PATH} --version`, {
      encoding: 'utf8',
      cwd: CLI_CWD,
    });

    expect(output.trim()).toMatch(/^\d+\.\d+\.\d+$/);
  });
});

describe('topgun --help', () => {
  it('should display help with all 15 commands', () => {
    const output = execSync(`node ${CLI_PATH} --help`, {
      encoding: 'utf8',
      cwd: CLI_CWD,
    });

    expect(output).toContain('TopGun CLI');
    expect(output).toContain('doctor');
    expect(output).toContain('setup');
    expect(output).toContain('dev');
    expect(output).toContain('test');
    expect(output).toContain('config');
    expect(output).toContain('cluster:start');
    expect(output).toContain('cluster:stop');
    expect(output).toContain('cluster:status');
    expect(output).toContain('docker:start');
    expect(output).toContain('docker:stop');
    expect(output).toContain('docker:status');
    expect(output).toContain('docker:logs');
    expect(output).toContain('debug:crdt');
    expect(output).toContain('search:explain');
    expect(output).toContain('codegen');
  });
});
