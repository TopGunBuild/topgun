import { execSync } from 'child_process';
import path from 'path';
import fs from 'fs';
import os from 'os';
import { runCli } from './test-utils';

describe('topgun debug:crdt', () => {
  const cliPath = path.join(__dirname, '../../bin/topgun.js');

  it('should show error for unknown action', () => {
    const result = runCli(['debug:crdt', 'invalid-action']);

    expect(result.exitCode).toBe(1);
    expect(result.stderr).toContain('Unknown action: invalid-action');
    expect(result.stderr).toContain('Valid actions:');
    expect(result.stderr).toContain('export');
    expect(result.stderr).toContain('stats');
    expect(result.stderr).toContain('conflicts');
    expect(result.stderr).toContain('timeline');
    expect(result.stderr).toContain('replay');
    expect(result.stderr).toContain('tail');
  });

  describe('replay', () => {
    it('should show error when --input is missing', () => {
      const result = runCli(['debug:crdt', 'replay']);

      expect(result.exitCode).toBe(1);
      expect(result.stderr).toContain('--input <file> is required');
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

        const output = execSync(`node ${cliPath} debug:crdt replay --input ${inputFile}`, {
          encoding: 'utf8',
          cwd: path.join(__dirname, '../..'),
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
