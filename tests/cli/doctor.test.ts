import { execSync } from 'child_process';
import path from 'path';

describe('topgun doctor', () => {
  const cliPath = path.join(__dirname, '../../bin/topgun.js');

  it('should run doctor command', () => {
    const output = execSync(`node ${cliPath} doctor`, {
      encoding: 'utf8',
      cwd: path.join(__dirname, '../..'),
    });

    expect(output).toContain('TopGun Environment Check');
    expect(output).toContain('Node.js');
    expect(output).toContain('pnpm');
  });

  it('should detect Node.js version', () => {
    const output = execSync(`node ${cliPath} doctor`, {
      encoding: 'utf8',
      cwd: path.join(__dirname, '../..'),
    });

    expect(output).toMatch(/Node\.js.*v\d+/);
    expect(output).toContain('✓');
  });

  it('should check for dependencies', () => {
    const output = execSync(`node ${cliPath} doctor`, {
      encoding: 'utf8',
      cwd: path.join(__dirname, '../..'),
    });

    expect(output).toContain('Dependencies');
  });
});

describe('topgun --version', () => {
  const cliPath = path.join(__dirname, '../../bin/topgun.js');

  it('should display version', () => {
    const output = execSync(`node ${cliPath} --version`, {
      encoding: 'utf8',
    });

    expect(output.trim()).toMatch(/^\d+\.\d+\.\d+$/);
  });
});

describe('topgun --help', () => {
  const cliPath = path.join(__dirname, '../../bin/topgun.js');

  it('should display help', () => {
    const output = execSync(`node ${cliPath} --help`, {
      encoding: 'utf8',
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
  });
});
