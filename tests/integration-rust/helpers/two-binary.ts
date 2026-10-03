/**
 * Launcher for tests that run more than one server binary against the same
 * data directory (an older build and a newer one), and for tests that need the
 * server's complete output.
 *
 * `spawnRustServer` cannot serve either purpose: it launches the single binary
 * named by `RUST_SERVER_BINARY`, inherits the child's stderr, and rejects when
 * the server exits before printing its port. A test that asserts "the older
 * binary refuses to start" or "this line was never logged" needs the opposite
 * on every count:
 *
 * - the binary is given by explicit path, one call per binary;
 * - stdout and stderr are piped and accumulated SEPARATELY from the first byte,
 *   so "absent from the log" is a statement about the whole log and about a
 *   known stream (the server writes `tracing` output and the `PORT=` line to
 *   stdout, and its fatal startup errors to stderr);
 * - a server that never prints `PORT=` is reported, not thrown, because that
 *   is an outcome some tests assert;
 * - the sha256 of the launched file is printed, because release builds are not
 *   byte-reproducible and the hash of what actually ran is the only provenance.
 */

import * as child_process from 'child_process';
import * as crypto from 'crypto';
import * as fs from 'fs';
import * as path from 'path';

/** Repository root — three levels up from tests/integration-rust/helpers/. */
const REPO_ROOT = path.resolve(__dirname, '..', '..', '..');

const DEFAULT_PORT_TIMEOUT_MS = 30_000;
const DEFAULT_EXIT_TIMEOUT_MS = 35_000;

/** The commands that produce both binaries, repeated in every "variable is missing" failure. */
const BUILD_HINT =
  'Two-binary mechanism: build the older server in its own worktree and target directory ' +
  '(`git worktree add <scratch>/topgun-old <old commit>`, then `cargo build --release -p topgun-server ' +
  '--manifest-path <scratch>/topgun-old/Cargo.toml --target-dir <scratch>/topgun-old-target`), ' +
  'build the newer one in this checkout (`cargo build --release -p topgun-server`), and export ' +
  'OLD_SERVER_BINARY, RUST_SERVER_BINARY (the newer binary), OLD_SERVER_COMMIT and NEW_SERVER_COMMIT ' +
  '(the commit each binary was built from).';

export interface BinaryExit {
  code: number | null;
  signal: NodeJS.Signals | null;
}

export interface SpawnBinaryOptions {
  /** Absolute path of the server executable to launch. */
  binaryPath: string;
  /** Shown in the provenance line and in error messages, e.g. `OLD` or `NEW first run`. */
  label: string;
  /** The commit the binary was built from; printed next to its sha256. */
  commit?: string;
  /** Merged over the parent environment and the launcher's defaults. */
  env?: Record<string, string>;
  /** Port to bind; 0 (the default) lets the OS choose. */
  port?: number;
  /** How long to wait for the `PORT=` line before reporting that it never came. */
  portTimeoutMs?: number;
}

export interface SpawnedBinary {
  label: string;
  binaryPath: string;
  /** sha256 of the file that was launched, hex. */
  sha256: string;
  process: child_process.ChildProcess;
  /**
   * The port the server bound, or `null` when no `PORT=` line appeared before
   * the process exited or the wait timed out.
   */
  port: number | null;
  /** Everything the process has written to stdout since it started. */
  stdout(): string;
  /** Everything the process has written to stderr since it started. */
  stderr(): string;
  /** Exit status once the process has ended, otherwise `null`. */
  exit(): BinaryExit | null;
  /** Sends a signal to the process. Returns false when it has already ended. */
  signal(sig: 'SIGTERM' | 'SIGKILL'): boolean;
  /** Resolves with the exit status, or `null` if the process is still running after the timeout. */
  waitForExit(timeoutMs?: number): Promise<BinaryExit | null>;
  /**
   * Sends the signal and waits for the process to end. If it outlives the
   * timeout it is killed, so a test never leaves a server holding the store's
   * file lock.
   */
  stop(sig?: 'SIGTERM' | 'SIGKILL', timeoutMs?: number): Promise<BinaryExit | null>;
}

export interface TwoBinaryEnv {
  oldBinary: string;
  newBinary: string;
  oldCommit: string;
  newCommit: string;
}

function requireVariable(name: string): string {
  const value = process.env[name];
  if (!value) {
    throw new Error(`${name} is not set. ${BUILD_HINT}`);
  }
  return value;
}

function requireExecutable(name: string): string {
  const value = requireVariable(name);
  try {
    if (!fs.statSync(value).isFile()) throw new Error('not a regular file');
    fs.accessSync(value, fs.constants.X_OK);
  } catch (err) {
    throw new Error(
      `${name} does not name an executable file (${value}: ${(err as Error).message}). ${BUILD_HINT}`,
    );
  }
  return value;
}

/**
 * Reads the four variables a two-binary test needs and throws, naming the
 * first one that is missing or unusable. There is deliberately no fallback: a
 * test that quietly ran one binary in both roles would pass without proving
 * anything about the other.
 */
export function requireTwoBinaryEnv(): TwoBinaryEnv {
  return {
    oldBinary: requireExecutable('OLD_SERVER_BINARY'),
    newBinary: requireExecutable('RUST_SERVER_BINARY'),
    oldCommit: requireVariable('OLD_SERVER_COMMIT'),
    newCommit: requireVariable('NEW_SERVER_COMMIT'),
  };
}

export function sha256OfFile(filePath: string): string {
  return crypto.createHash('sha256').update(fs.readFileSync(filePath)).digest('hex');
}

/**
 * Starts the given server binary and resolves once it has printed `PORT=`, has
 * exited, or the port wait has timed out — whichever comes first. It never
 * rejects for a server that failed to start; inspect `port`, `exit()` and the
 * two captures instead.
 */
export async function spawnBinary(options: SpawnBinaryOptions): Promise<SpawnedBinary> {
  const { binaryPath, label } = options;
  const sha256 = sha256OfFile(binaryPath);

  // Straight to stderr: the suite silences console.log, and this line is the
  // record of which build produced the run.
  process.stderr.write(
    `[two-binary] ${label}: sha256=${sha256} commit=${options.commit ?? 'unstated'} path=${binaryPath}\n`,
  );

  const proc = child_process.spawn(binaryPath, ['--port', String(options.port ?? 0)], {
    cwd: REPO_ROOT,
    stdio: ['ignore', 'pipe', 'pipe'],
    env: {
      ...process.env,
      STORAGE_BACKEND: 'null',
      JWT_SECRET: 'test-e2e-secret',
      ...options.env,
    },
  });

  // Listeners are attached in the same tick as the spawn, before any I/O can be
  // delivered, so nothing the process writes is missed. They stay attached for
  // the life of the process, which also keeps both pipes drained: a server
  // whose stdout pipe fills up blocks on its next log write and stops serving.
  const stdoutChunks: Buffer[] = [];
  const stderrChunks: Buffer[] = [];
  proc.stdout!.on('data', (chunk: Buffer) => stdoutChunks.push(chunk));
  proc.stderr!.on('data', (chunk: Buffer) => stderrChunks.push(chunk));
  const stdout = () => Buffer.concat(stdoutChunks).toString('utf8');
  const stderr = () => Buffer.concat(stderrChunks).toString('utf8');

  let exited: BinaryExit | null = null;
  // 'close' rather than 'exit': it fires after both pipes have ended, so the
  // captures are complete by the time anything awaiting the exit reads them.
  const closed = new Promise<BinaryExit>((resolve) => {
    proc.once('close', (code, signal) => {
      exited = { code, signal };
      resolve(exited);
    });
    proc.once('error', (err) => {
      stderrChunks.push(Buffer.from(`[two-binary] spawn error: ${err.message}\n`));
      exited = { code: null, signal: null };
      resolve(exited);
    });
  });

  const waitForExit = (timeoutMs = DEFAULT_EXIT_TIMEOUT_MS): Promise<BinaryExit | null> => {
    if (exited) return Promise.resolve(exited);
    return new Promise((resolve) => {
      const timer = setTimeout(() => resolve(null), timeoutMs);
      void closed.then((status) => {
        clearTimeout(timer);
        resolve(status);
      });
    });
  };

  const signal = (sig: 'SIGTERM' | 'SIGKILL'): boolean => {
    if (exited || proc.pid == null) return false;
    try {
      return proc.kill(sig);
    } catch {
      return false;
    }
  };

  const stop = async (
    sig: 'SIGTERM' | 'SIGKILL' = 'SIGTERM',
    timeoutMs = DEFAULT_EXIT_TIMEOUT_MS,
  ): Promise<BinaryExit | null> => {
    signal(sig);
    const status = await waitForExit(timeoutMs);
    if (status) return status;
    signal('SIGKILL');
    return waitForExit(5_000);
  };

  const readPort = (): number | null => {
    const match = /^PORT=(\d+)\s*$/m.exec(stdout());
    return match ? parseInt(match[1], 10) : null;
  };

  const portDeadline = Date.now() + (options.portTimeoutMs ?? DEFAULT_PORT_TIMEOUT_MS);
  let port = readPort();
  while (port === null && !exited && Date.now() < portDeadline) {
    await new Promise((resolve) => setTimeout(resolve, 25));
    port = readPort();
  }
  // The line may have arrived in the same turn the process ended.
  if (port === null) port = readPort();

  return {
    label,
    binaryPath,
    sha256,
    process: proc,
    port,
    stdout,
    stderr,
    exit: () => exited,
    signal,
    waitForExit,
    stop,
  };
}
