import { execSync } from 'child_process';
import fs from 'fs';
import path from 'path';
import os from 'os';

export interface CliResult {
  stdout: string;
  stderr: string;
  exitCode: number;
}

// Points at the BUILT artifact — validates the actual published bin, not the TS source
const CLI_PATH = path.join(__dirname, '../../dist/topgun.js');
// Run from packages/cli/ by default; individual tests may override cwd
const DEFAULT_CWD = path.join(__dirname, '../..');

/**
 * Runs the built TopGun CLI with given arguments and returns the result.
 * Captures both stdout and stderr, and returns the exit code.
 */
export function runCli(args: string[], cwd?: string): CliResult {
  const command = `node ${CLI_PATH} ${args.join(' ')}`;

  try {
    const stdout = execSync(command, {
      encoding: 'utf8',
      cwd: cwd ?? DEFAULT_CWD,
      stdio: ['pipe', 'pipe', 'pipe'],
    });

    return {
      stdout,
      stderr: '',
      exitCode: 0,
    };
  } catch (error: unknown) {
    const err = error as NodeJS.ErrnoException & {
      stdout?: Buffer | string;
      stderr?: Buffer | string;
      status?: number;
    };
    return {
      stdout: err.stdout?.toString() || '',
      stderr: err.stderr?.toString() || '',
      exitCode: err.status || 1,
    };
  }
}

/**
 * Creates a temporary .env file, runs the callback, then cleans up.
 */
export function withTempEnv(content: string, fn: () => void): void {
  const envPath = path.join(process.cwd(), '.env');
  const backupPath = envPath + '.backup';

  if (fs.existsSync(envPath)) {
    fs.copyFileSync(envPath, backupPath);
  }

  try {
    fs.writeFileSync(envPath, content);
    fn();
  } finally {
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
export function withTempDir(
  structure: Record<string, string | null>,
  fn: (tempDir: string) => void,
): void {
  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), 'topgun-test-'));

  try {
    for (const [filePath, content] of Object.entries(structure)) {
      const fullPath = path.join(tempDir, filePath);
      const dir = path.dirname(fullPath);

      if (!fs.existsSync(dir)) {
        fs.mkdirSync(dir, { recursive: true });
      }

      if (content !== null) {
        fs.writeFileSync(fullPath, content);
      }
    }

    fn(tempDir);
  } finally {
    if (fs.existsSync(tempDir)) {
      fs.rmSync(tempDir, { recursive: true, force: true });
    }
  }
}
