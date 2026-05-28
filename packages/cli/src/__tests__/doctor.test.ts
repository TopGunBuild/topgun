import { execSync } from 'child_process';
import path from 'path';

const CLI_PATH = path.join(__dirname, '../../dist/topgun.js');
// Run from the repo root so doctor finds the server binary + .env
const CLI_CWD = path.join(__dirname, '../../../..');

describe('topgun doctor', () => {
  it('should run doctor command', () => {
    // Pipe stderr to avoid propagation of docker-not-found shell errors to the test runner
    const output = execSync(`node ${CLI_PATH} doctor`, {
      encoding: 'utf8',
      cwd: CLI_CWD,
      stdio: ['pipe', 'pipe', 'pipe'],
    });

    expect(output).toContain('TopGun Environment Check');
    expect(output).toContain('Node.js');
    expect(output).toContain('pnpm');
  });

  it('should detect Node.js version', () => {
    const output = execSync(`node ${CLI_PATH} doctor`, {
      encoding: 'utf8',
      cwd: CLI_CWD,
      stdio: ['pipe', 'pipe', 'pipe'],
    });

    expect(output).toMatch(/Node\.js.*v\d+/);
    expect(output).toContain('✓');
  });

  it('should check for dependencies', () => {
    const output = execSync(`node ${CLI_PATH} doctor`, {
      encoding: 'utf8',
      cwd: CLI_CWD,
      stdio: ['pipe', 'pipe', 'pipe'],
    });

    expect(output).toContain('Dependencies');
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
