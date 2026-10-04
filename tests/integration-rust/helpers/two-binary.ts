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

import { createRustTestClient, completeMerkleSync } from './index';
import { TestClient } from './test-client';

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

// ---------------------------------------------------------------------------
// Bounded probes
//
// A server under test here may stop answering altogether (it keeps its port
// open, completes no handshake and ignores SIGTERM). Every probe below
// therefore gives up after a stated time and says what it was waiting for, so
// a dead server shows up as a named observation and never as a suite timeout.
// ---------------------------------------------------------------------------

const DEFAULT_PROBE_TIMEOUT_MS = 5_000;

export function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/** Rejects with `what` in the message when `work` has not settled in time. */
export async function within<T>(work: Promise<T>, timeoutMs: number, what: string): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const expired = new Promise<never>((_, reject) => {
    timer = setTimeout(
      () => reject(new Error(`${what}: nothing within ${timeoutMs} ms`)),
      timeoutMs,
    );
  });
  // The loser of the race must not surface later as an unhandled rejection.
  work.catch(() => undefined);
  try {
    return await Promise.race([work, expired]);
  } finally {
    clearTimeout(timer);
  }
}

/**
 * Opens an authenticated admin connection, or rejects when the server has not
 * completed the handshake in time. The client's own heartbeat is stopped so
 * that every frame a test later sees on the socket is an answer to something
 * the test sent.
 */
export async function connectWithin(
  port: number,
  nodeId: string,
  timeoutMs = DEFAULT_PROBE_TIMEOUT_MS,
): Promise<TestClient> {
  let abandoned = false;
  const opening = (async () => {
    const client = await createRustTestClient(port, {
      nodeId,
      userId: 'map-name-user',
      roles: ['ADMIN'],
    });
    try {
      await client.waitForMessage('AUTH_ACK', timeoutMs);
    } catch (err) {
      client.close();
      throw err;
    }
    client.stopHeartbeat();
    if (abandoned) client.close();
    return client;
  })();
  try {
    return await within(opening, timeoutMs, `handshake with port ${port} (${nodeId})`);
  } catch (err) {
    abandoned = true;
    throw err;
  }
}

/** Resolves with the first frame of one of the given types, or `null` when none arrives in time. */
export async function firstFrame(
  client: TestClient,
  types: string[],
  timeoutMs: number,
): Promise<any | null> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const frame = client.messages.find((m) => types.includes(m.type));
    if (frame) return frame;
    if (Date.now() >= deadline) return null;
    await sleep(20);
  }
}

/** Reads one key of a map through a snapshot query, over a connection of its own. */
export async function queryKey(
  port: number,
  mapName: string,
  key: string,
  nodeId: string,
  timeoutMs = DEFAULT_PROBE_TIMEOUT_MS,
): Promise<unknown> {
  const client = await connectWithin(port, nodeId, timeoutMs);
  try {
    client.messages.length = 0;
    client.send({ type: 'QUERY_SUB', payload: { queryId: `q-${nodeId}`, mapName, query: {} } });
    const response = await firstFrame(client, ['QUERY_RESP'], timeoutMs);
    if (!response)
      throw new Error(`no QUERY_RESP for ${JSON.stringify(mapName)} in ${timeoutMs} ms`);
    const row = (response.payload.results as any[]).find((r) => r.key === key);
    return row ? (row.record?.value ?? row.value) : undefined;
  } finally {
    client.close();
  }
}

/** Reads one key of a map through a full Merkle walk from an empty tree, over a connection of its own. */
export async function merkleKey(
  port: number,
  mapName: string,
  key: string,
  nodeId: string,
  timeoutMs = DEFAULT_PROBE_TIMEOUT_MS,
): Promise<unknown> {
  const client = await connectWithin(port, nodeId, timeoutMs);
  try {
    const records = await completeMerkleSync(client, mapName, timeoutMs);
    return records.get(key)?.value;
  } finally {
    client.close();
  }
}

/**
 * Reads the live values of one key of an OR-Map through a full OR-Map Merkle
 * walk from an empty tree, over a connection of its own. An empty array means
 * the server answered and holds nothing for the key.
 */
export async function orMapValues(
  port: number,
  mapName: string,
  key: string,
  nodeId: string,
  timeoutMs = DEFAULT_PROBE_TIMEOUT_MS,
): Promise<unknown[]> {
  const client = await connectWithin(port, nodeId, timeoutMs);
  try {
    client.messages.length = 0;
    client.send({
      type: 'ORMAP_SYNC_INIT',
      mapName,
      rootHash: 0,
      bucketHashes: {},
      lastSyncTimestamp: 0,
    });
    const root = await firstFrame(client, ['ORMAP_SYNC_RESP_ROOT'], timeoutMs);
    if (!root) {
      throw new Error(`no ORMAP_SYNC_RESP_ROOT for ${JSON.stringify(mapName)} in ${timeoutMs} ms`);
    }
    if (Number(root.payload.rootHash) === 0) return [];

    const values: unknown[] = [];
    // A path the server holds nothing under gets no answer, so only children
    // reported with a non-zero hash are followed.
    const pendingPaths = [''];
    while (pendingPaths.length > 0) {
      const requested = pendingPaths.shift() as string;
      const seen = client.messages.length;
      client.send({ type: 'ORMAP_MERKLE_REQ_BUCKET', payload: { mapName, path: requested } });
      const deadline = Date.now() + timeoutMs;
      let answer: any;
      while (!answer && Date.now() < deadline) {
        answer = client.messages
          .slice(seen)
          .find(
            (m) =>
              (m.type === 'ORMAP_SYNC_RESP_BUCKETS' || m.type === 'ORMAP_SYNC_RESP_LEAF') &&
              m.payload?.mapName === mapName &&
              m.payload?.path === requested,
          );
        if (!answer) await sleep(20);
      }
      if (!answer) throw new Error(`no answer for OR-Map Merkle path "${requested}"`);
      if (answer.type === 'ORMAP_SYNC_RESP_BUCKETS') {
        for (const [child, hash] of Object.entries(answer.payload.buckets as object)) {
          if (Number(hash) !== 0) pendingPaths.push(requested + child);
        }
      } else {
        for (const entry of answer.payload.entries as any[]) {
          if (entry.key === key) values.push(...entry.records.map((r: any) => r.value));
        }
      }
    }
    return values;
  } finally {
    client.close();
  }
}

/** Runs a probe and returns its result, or the reason it failed as a string that no real value equals. */
export async function observe<T>(probe: () => Promise<T>): Promise<T | string> {
  try {
    return await probe();
  } catch (err) {
    return `NOT OBSERVED: ${(err as Error).message}`;
  }
}

// ---------------------------------------------------------------------------
// WAL directory, as files
// ---------------------------------------------------------------------------

export interface WalPartition {
  /** Total size of the partition's segment files. */
  segmentBytes: number;
  /** Whether the partition has an applied-watermark sidecar file. */
  hasSidecar: boolean;
}

/**
 * Lists the WAL directory by partition without decoding anything: segment
 * files are `partition-NNN-SSS.log`, the applied watermark of a partition is
 * the sidecar `partition-NNN.applied`.
 */
export function walPartitions(walDir: string): Map<number, WalPartition> {
  const partitions = new Map<number, WalPartition>();
  if (!fs.existsSync(walDir)) return partitions;
  const slot = (id: number): WalPartition => {
    let entry = partitions.get(id);
    if (!entry) {
      entry = { segmentBytes: 0, hasSidecar: false };
      partitions.set(id, entry);
    }
    return entry;
  };
  for (const name of fs.readdirSync(walDir)) {
    const segment = /^partition-(\d+)-\d+\.log$/.exec(name);
    const sidecar = /^partition-(\d+)\.applied$/.exec(name);
    if (segment) {
      slot(parseInt(segment[1], 10)).segmentBytes += fs.statSync(path.join(walDir, name)).size;
    } else if (sidecar) {
      slot(parseInt(sidecar[1], 10)).hasSidecar = true;
    }
  }
  return partitions;
}

/** Ids of the partitions whose segments are larger in `after` than in `before`. */
export function grownPartitions(
  before: Map<number, WalPartition>,
  after: Map<number, WalPartition>,
): number[] {
  return [...after.entries()]
    .filter(([id, state]) => state.segmentBytes > (before.get(id)?.segmentBytes ?? 0))
    .map(([id]) => id)
    .sort((a, b) => a - b);
}

/** How many times `needle` occurs in the files under `dir`, searched as raw bytes. */
export function countBytesUnder(dir: string, needle: string): number {
  if (!fs.existsSync(dir)) return 0;
  const pattern = Buffer.from(needle, 'utf8');
  let count = 0;
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      count += countBytesUnder(full, needle);
    } else if (entry.isFile()) {
      const bytes = fs.readFileSync(full);
      for (let at = bytes.indexOf(pattern); at !== -1; at = bytes.indexOf(pattern, at + 1)) count++;
    }
  }
  return count;
}

/** Log lines of a capture with terminal colour codes removed. */
export function plainLines(capture: string): string[] {
  return capture.replace(/\u001b\[[0-9;]*m/g, '').split('\n');
}
