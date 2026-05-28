import { execSync } from 'child_process';
import path from 'path';

const CLI_PATH = path.join(__dirname, '../../dist/topgun.js');
const CLI_CWD = path.join(__dirname, '../../');

describe('topgun cluster:start', () => {
  it('should show help with --help flag', () => {
    const output = execSync(`node ${CLI_PATH} cluster:start --help`, {
      encoding: 'utf8',
      cwd: CLI_CWD,
    });

    expect(output).toContain('Usage: topgun cluster:start');
    expect(output).toContain('Start local cluster');
  });
});

describe('topgun cluster:stop', () => {
  it('should show help with --help flag', () => {
    const output = execSync(`node ${CLI_PATH} cluster:stop --help`, {
      encoding: 'utf8',
      cwd: CLI_CWD,
    });

    expect(output).toContain('Usage: topgun cluster:stop');
    expect(output).toContain('Stop local cluster');
  });
});

describe('topgun cluster:status', () => {
  it('should show help with --help flag', () => {
    const output = execSync(`node ${CLI_PATH} cluster:status --help`, {
      encoding: 'utf8',
      cwd: CLI_CWD,
    });

    expect(output).toContain('Usage: topgun cluster:status');
    expect(output).toContain('Show cluster status');
  });
});
