import { execSync } from 'child_process';
import fs from 'fs';
import path from 'path';
import os from 'os';

export interface CliResult {
  stdout: string;
  stderr: string;
  exitCode: number;
}

/**
 * Runs the TopGun CLI with given arguments and returns the result.
 * Captures both stdout and stderr, and returns the exit code.
 */
export function runCli(args: string[]): CliResult {
  const cliPath = path.join(__dirname, '../../bin/topgun.js');
  const command = `node ${cliPath} ${args.join(' ')}`;

  try {
    const stdout = execSync(command, {
      encoding: 'utf8',
      cwd: path.join(__dirname, '../..'),
      stdio: ['pipe', 'pipe', 'pipe'],
    });

    return {
      stdout,
      stderr: '',
      exitCode: 0,
    };
  } catch (error: any) {
    return {
      stdout: error.stdout?.toString() || '',
      stderr: error.stderr?.toString() || '',
      exitCode: error.status || 1,
    };
  }
}

/**
 * Creates a temporary .env file, runs the callback, then cleans up.
 */
export function withTempEnv(content: string, fn: () => void): void {
  const envPath = path.join(process.cwd(), '.env');
  const backupPath = envPath + '.backup';

  // Backup existing .env if present
  if (fs.existsSync(envPath)) {
    fs.copyFileSync(envPath, backupPath);
  }

  try {
    fs.writeFileSync(envPath, content);
    fn();
  } finally {
    // Restore or remove .env
    if (fs.existsSync(backupPath)) {
      fs.copyFileSync(backupPath, envPath);
      fs.unlinkSync(backupPath);
    } else if (fs.existsSync(envPath)) {
      fs.unlinkSync(envPath);
    }
  }
}

/**
 * Creates a temporary file, runs the callback, then cleans up.
 */
export function withTempFile(filePath: string, content: string, fn: () => void): void {
  const fullPath = path.join(process.cwd(), filePath);
  const dir = path.dirname(fullPath);

  // Ensure directory exists
  if (!fs.existsSync(dir)) {
    fs.mkdirSync(dir, { recursive: true });
  }

  try {
    fs.writeFileSync(fullPath, content);
    fn();
  } finally {
    if (fs.existsSync(fullPath)) {
      fs.unlinkSync(fullPath);
    }
  }
}

/**
 * Creates a temporary directory with given structure and content.
 * Returns the temp directory path for use in tests.
 */
export function withTempDir(structure: Record<string, string | null>, fn: (tempDir: string) => void): void {
  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), 'topgun-test-'));

  try {
    // Create directory structure
    for (const [filePath, content] of Object.entries(structure)) {
      const fullPath = path.join(tempDir, filePath);
      const dir = path.dirname(fullPath);

      // Create directory if needed
      if (!fs.existsSync(dir)) {
        fs.mkdirSync(dir, { recursive: true });
      }

      // Create file if content is provided, otherwise just create directory
      if (content !== null) {
        fs.writeFileSync(fullPath, content);
      }
    }

    fn(tempDir);
  } finally {
    // Cleanup
    if (fs.existsSync(tempDir)) {
      fs.rmSync(tempDir, { recursive: true, force: true });
    }
  }
}
