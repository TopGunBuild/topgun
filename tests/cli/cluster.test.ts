import { execSync } from 'child_process';
import path from 'path';
import fs from 'fs';
import { withTempFile } from './test-utils';

describe('topgun cluster:start', () => {
  const cliPath = path.join(__dirname, '../../bin/topgun.js');

  it('should show help with --help flag', () => {
    const output = execSync(`node ${cliPath} cluster:start --help`, {
      encoding: 'utf8',
      cwd: path.join(__dirname, '../..'),
    });

    expect(output).toContain('Usage: topgun cluster:start');
    expect(output).toContain('Start local cluster');
  });
});

describe('topgun cluster:status', () => {
  const cliPath = path.join(__dirname, '../../bin/topgun.js');

  it('should show warning when no .cluster-pids file exists', () => {
    const pidsPath = path.join(process.cwd(), '.cluster-pids');
    const backupPath = pidsPath + '.backup';

    // Backup and remove .cluster-pids temporarily
    if (fs.existsSync(pidsPath)) {
      fs.copyFileSync(pidsPath, backupPath);
      fs.unlinkSync(pidsPath);
    }

    try {
      const output = execSync(`node ${cliPath} cluster:status`, {
        encoding: 'utf8',
        cwd: path.join(__dirname, '../..'),
      });

      expect(output).toContain('TopGun Cluster Status');
      expect(output).toContain('No cluster PID file found');
      expect(output).toContain('npx topgun cluster:start');
    } finally {
      // Restore .cluster-pids
      if (fs.existsSync(backupPath)) {
        fs.copyFileSync(backupPath, pidsPath);
        fs.unlinkSync(backupPath);
      }
    }
  });

  it('should show status with mock .cluster-pids file', () => {
    const mockPids = `${process.pid},${process.pid + 1000},${process.pid + 2000}`;

    withTempFile('.cluster-pids', mockPids, () => {
      const output = execSync(`node ${cliPath} cluster:status`, {
        encoding: 'utf8',
        cwd: path.join(__dirname, '../..'),
      });

      expect(output).toContain('TopGun Cluster Status');
      expect(output).toContain('Cluster nodes: 3');
      expect(output).toContain('node-1');
      expect(output).toContain('PID');
    });
  });
});

describe('topgun cluster:stop', () => {
  const cliPath = path.join(__dirname, '../../bin/topgun.js');

  it('should show warning when no .cluster-pids file exists', () => {
    const pidsPath = path.join(process.cwd(), '.cluster-pids');
    const backupPath = pidsPath + '.backup';

    // Backup and remove .cluster-pids temporarily
    if (fs.existsSync(pidsPath)) {
      fs.copyFileSync(pidsPath, backupPath);
      fs.unlinkSync(pidsPath);
    }

    try {
      const output = execSync(`node ${cliPath} cluster:stop`, {
        encoding: 'utf8',
        cwd: path.join(__dirname, '../..'),
      });

      expect(output).toContain('TopGun Cluster Stop');
      expect(output).toContain('No running cluster found');
    } finally {
      // Restore .cluster-pids
      if (fs.existsSync(backupPath)) {
        fs.copyFileSync(backupPath, pidsPath);
        fs.unlinkSync(backupPath);
      }
    }
  });

  it('should attempt cleanup with mock .cluster-pids file', () => {
    const mockPids = `99999,99998`;

    withTempFile('.cluster-pids', mockPids, () => {
      const output = execSync(`node ${cliPath} cluster:stop`, {
        encoding: 'utf8',
        cwd: path.join(__dirname, '../..'),
      });

      expect(output).toContain('TopGun Cluster Stop');
      expect(output).toContain('Stopping');
    });
  });
});
