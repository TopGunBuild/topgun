import { execSync } from 'child_process';
import path from 'path';
import { runCli } from './test-utils';

describe('topgun debug:search', () => {
  const cliPath = path.join(__dirname, '../../bin/topgun.js');

  it('should show help when no query provided', () => {
    const output = execSync(`node ${cliPath} debug:search`, {
      encoding: 'utf8',
      cwd: path.join(__dirname, '../..'),
    });

    expect(output).toContain('TopGun Search Explainer');
    expect(output).toContain('Usage:');
    expect(output).toContain('--query');
    expect(output).toContain('--map');
    expect(output).toContain('Examples:');
  });

  it('should attempt HTTP call when query provided', () => {
    const result = runCli(['debug:search', '--query', 'test']);

    // Command will attempt HTTP connection and fail
    // We verify it proceeds past argument validation
    const allOutput = result.stdout + result.stderr;
    expect(allOutput).toContain('Connection error');
  });
});
