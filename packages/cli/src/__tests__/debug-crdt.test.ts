import { execSync } from 'child_process';
import path from 'path';
import fs from 'fs';
import os from 'os';
import { runCli } from './test-utils';

const CLI_PATH = path.join(__dirname, '../../dist/topgun.js');
const CLI_CWD = path.join(__dirname, '../../');

describe('topgun debug:crdt', () => {
  it('should show error for unknown action', () => {
    const result = runCli(['debug:crdt', 'invalid-action']);

    expect(result.exitCode).toBe(1);
    const allOutput = result.stdout + result.stderr;
    expect(allOutput).toContain('Unknown action: invalid-action');
    expect(allOutput).toContain('Valid actions:');
    expect(allOutput).toContain('export');
    expect(allOutput).toContain('stats');
    expect(allOutput).toContain('conflicts');
    expect(allOutput).toContain('timeline');
    expect(allOutput).toContain('replay');
    expect(allOutput).toContain('tail');
  });

  it('should show "Not available" for network actions (preserved stub)', () => {
    const result = runCli(['debug:crdt', 'export', '--map', 'users']);

    // Network actions return the "Not available" stub — server does not expose debug endpoints yet
    const allOutput = result.stdout + result.stderr;
    expect(allOutput).toContain('Not available');
  });

  describe('replay', () => {
    it('should show error when --input is missing', () => {
      const result = runCli(['debug:crdt', 'replay']);

      expect(result.exitCode).toBe(1);
      const allOutput = result.stdout + result.stderr;
      expect(allOutput).toContain('--input <file> is required');
    });

    it('should parse operations from mock input file', () => {
      const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), 'topgun-crdt-test-'));
      const inputFile = path.join(tempDir, 'ops.json');

      const mockOps = {
        operations: [
          {
            operation: 'set',
            key: 'user:123',
            nodeId: 'node-1',
            timestamp: { millis: Date.now() },
          },
          {
            operation: 'delete',
            key: 'user:456',
            nodeId: 'node-2',
            timestamp: { millis: Date.now() + 1000 },
          },
        ],
      };

      try {
        fs.writeFileSync(inputFile, JSON.stringify(mockOps));

        const output = execSync(`node ${CLI_PATH} debug:crdt replay --input ${inputFile}`, {
          encoding: 'utf8',
          cwd: CLI_CWD,
        });

        expect(output).toContain('Replaying operations');
        expect(output).toContain('Loaded 2 operations');
        expect(output).toContain('SET');
        expect(output).toContain('DELETE');
        expect(output).toContain('user:123');
        expect(output).toContain('user:456');
        expect(output).toContain('Replay complete');
      } finally {
        fs.unlinkSync(inputFile);
        fs.rmdirSync(tempDir);
      }
    });
  });
});
