import path from 'path';
import { execSync } from 'child_process';
import { runCli } from './test-utils';

const CLI_PATH = path.join(__dirname, '../../dist/topgun.js');
const CLI_CWD = path.join(__dirname, '../../');

describe('topgun search:explain', () => {
  it('should show the "Not available" stub message (preserved verbatim)', () => {
    const output = execSync(`node ${CLI_PATH} search:explain`, {
      encoding: 'utf8',
      cwd: CLI_CWD,
    });

    expect(output).toContain('TopGun Search Explainer');
    expect(output).toContain('Not available');
  });

  it('should show "Not available" even when query is provided', () => {
    const result = runCli(['search:explain', '--query', 'test']);

    const allOutput = result.stdout + result.stderr;
    expect(allOutput).toContain('Not available');
  });
});
