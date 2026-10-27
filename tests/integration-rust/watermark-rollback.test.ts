/**
 * Integration: the applied WAL watermark lives in the data directory, and an
 * older binary refuses a data directory that holds it.
 *
 * The older binary (built at 6a030bb6) keeps each WAL partition's applied
 * watermark in a `.applied` file of the WAL directory. The newer binary keeps
 * it in the data directory, committed with the records it covers, imports the
 * older binary's `.applied` files once and deletes them, and marks the data
 * directory so that the older binary refuses it at its store open — before it
 * opens the WAL and before it listens — instead of replaying the WAL against a
 * watermark it cannot see. At a clean stop the newer binary writes the
 * watermarks back as `.applied` rollback hints; after `kill -9` there are none.
 * Three cases:
 *
 * - W-A clean stop and W-B `kill -9` — the newer binary writes on a fresh
 *   directory and stops; the WAL directory holds `wal-collected`, and rollback
 *   hints only after the clean stop; the older binary started on the directory
 *   exits non-zero naming the unknown metadata, never prints `PORT=`, and
 *   changes no file of the WAL directory and not the marker file at the old
 *   store path; the newer binary started again serves every value.
 * - W-C — a directory written only by the older binary is opened by the newer
 *   one: its `.applied` files are gone while the newer binary runs, the values
 *   are served, and the older binary started afterwards refuses the directory,
 *   so the import marked it.
 *
 * Two binaries are needed, each given by path together with the commit it was
 * built from (WATERMARK_OLD_SERVER_BINARY / WATERMARK_OLD_SERVER_COMMIT for the
 * older one, RUST_SERVER_BINARY / NEW_SERVER_COMMIT for the newer). With any of
 * them missing the test fails; it never runs one binary in both roles.
 */

import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

import {
  requireWatermarkRollbackEnv,
  WATERMARK_OLD_COMMIT_PREFIX,
  spawnBinary,
  connectWithin,
  firstFrame,
  queryKey,
  observe,
  sleep,
  plainLines,
  treeSnapshot,
  SpawnedBinary,
  BinaryExit,
  TreeSnapshot,
} from './helpers/two-binary';

const MAP = 'watermark_items';
const KEYS = ['k0', 'k1', 'k2', 'k3', 'k4', 'k5'];

const ACK_WAIT_MS = 5_000;
/** The default flush interval is 1 s; this leaves every write stored and its watermark committed. */
const FLUSH_SETTLE_MS = 5_000;
/** A binary that must refuse to start has this long, from its spawn, to exit. */
const REFUSAL_DEADLINE_MS = 30_000;
/** A graceful stop may spend up to 30 s draining buffered writes; past this the process is killed and the exit status says so. */
const STOP_TIMEOUT_MS = 40_000;
const CASE_TIMEOUT_MS = 300_000;

const CLEAN_EXIT: BinaryExit = { code: 0, signal: null };

function valueOf(key: string, run: string): Record<string, string> {
  return { key, run };
}

function storeEnv(dir: string, extra: Record<string, string> = {}): Record<string, string> {
  return {
    STORAGE_BACKEND: 'redb',
    TOPGUN_DATA_DIR: path.join(dir, 'topgun-data'),
    TOPGUN_REDB_PATH: path.join(dir, 'topgun.redb'),
    TOPGUN_WAL_DIR: path.join(dir, 'wal'),
    TOPGUN_WAL_FSYNC_POLICY: 'per_op',
    ...extra,
  };
}

function freshDir(tag: string): string {
  return fs.mkdtempSync(path.join(os.tmpdir(), `topgun-watermark-rollback-${tag}-`));
}

/** The WAL directory as files, without decoding any. */
interface WalFiles {
  collected: boolean;
  /** Partitions with at least one segment file. */
  segmentPartitions: number[];
  /** Size of every `partition-NNN.applied` file, by partition. */
  applied: Record<number, number>;
  /** Names of every `.applied` / `.applied.tmp` file. */
  appliedNames: string[];
}

function walFiles(walDir: string): WalFiles {
  const files: WalFiles = {
    collected: false,
    segmentPartitions: [],
    applied: {},
    appliedNames: [],
  };
  if (!fs.existsSync(walDir)) return files;
  const segments = new Set<number>();
  for (const name of fs.readdirSync(walDir)) {
    const segment = /^partition-(\d+)-\d+\.log$/.exec(name);
    const applied = /^partition-(\d+)\.applied$/.exec(name);
    if (name === 'wal-collected') files.collected = true;
    if (segment) segments.add(parseInt(segment[1], 10));
    if (applied) {
      files.applied[parseInt(applied[1], 10)] = fs.statSync(path.join(walDir, name)).size;
    }
    if (/\.applied(\.tmp)?$/.test(name)) files.appliedNames.push(name);
  }
  files.segmentPartitions = [...segments].sort((a, b) => a - b);
  files.appliedNames.sort();
  return files;
}

/** The WAL directory's files and the marker file at the old store path are hashed; the engine file is not, as an open commits. */
function guardedSnapshot(dir: string): TreeSnapshot {
  return treeSnapshot(dir, (rel) => rel.startsWith(`wal${path.sep}`) || rel === 'topgun.redb');
}

function unreplayedLines(server: SpawnedBinary): string[] {
  return plainLines(server.stdout())
    .filter((l) => l.includes('BootUnreplayed') || l.includes('replay failed for entry'))
    .map((l) => l.slice(0, 400));
}

/** Both streams of a server, for the record. */
function streams(server: SpawnedBinary): string {
  const stdout = plainLines(server.stdout())
    .filter((l) => !l.includes('scalar index rebuild complete'))
    .map((l) => l.slice(0, 500))
    .join('\n');
  return `--- ${server.label} stdout ---\n${stdout}\n--- ${server.label} stderr ---\n${server.stderr()}\n`;
}

/** Sends one write and returns the types of the frames it was answered with inside the ack wait. */
async function writeAndCollect(
  port: number,
  nodeId: string,
  payload: Record<string, unknown>,
): Promise<string[]> {
  const client = await connectWithin(port, nodeId);
  try {
    client.messages.length = 0;
    client.send({ type: 'CLIENT_OP', payload });
    await firstFrame(client, ['OP_ACK', 'OP_REJECTED', 'ERROR'], ACK_WAIT_MS);
    return client.messages.map((m) => m.type);
  } finally {
    client.close();
  }
}

/** Writes every key of MAP with the value of `run`; returns the answers by key. */
async function writeAll(
  port: number,
  tag: string,
  run: string,
): Promise<Record<string, string[] | string>> {
  const answers: Record<string, string[] | string> = {};
  for (const [i, key] of KEYS.entries()) {
    answers[key] = await observe(() =>
      writeAndCollect(port, `${tag}-writer-${i}`, {
        id: String(i + 1),
        mapName: MAP,
        opType: 'PUT',
        key,
        record: {
          value: valueOf(key, run),
          timestamp: { millis: Date.now(), counter: 0, nodeId: 'watermark' },
        },
      }),
    );
  }
  return answers;
}

async function readAll(port: number, tag: string): Promise<Record<string, unknown>> {
  const values: Record<string, unknown> = {};
  for (const [i, key] of KEYS.entries()) {
    values[key] = await observe(() => queryKey(port, MAP, key, `${tag}-q-${i}`));
  }
  return values;
}

function expectedValues(run: string): Record<string, unknown> {
  return Object.fromEntries(KEYS.map((key) => [key, valueOf(key, run)]));
}

function allAcked(): Record<string, string[]> {
  return Object.fromEntries(KEYS.map((key) => [key, ['OP_ACK']]));
}

/** Starts a binary that must serve, or throws with its stderr. */
async function startServing(
  running: SpawnedBinary[],
  binaryPath: string,
  commit: string,
  label: string,
  env: Record<string, string>,
): Promise<[SpawnedBinary, number]> {
  const server = await spawnBinary({ binaryPath, commit, label, env });
  running.push(server);
  if (server.port === null) {
    throw new Error(`${label} did not print PORT=; stderr:\n${server.stderr()}`);
  }
  return [server, server.port];
}

/** What a start that must be refused did. */
interface RefusedStart {
  /** Exit status within the deadline; `null` means it was still running and was killed. */
  exit: BinaryExit | null;
  printedPort: boolean;
  output: string;
}

/**
 * Starts a binary that must refuse to start and waits for it to end, at most
 * REFUSAL_DEADLINE_MS from the spawn. A process still running then — or one
 * that printed `PORT=` — is killed at once, so it cannot go on to write into
 * the directory, and is recorded as not having exited.
 */
async function startExpectingRefusal(
  running: SpawnedBinary[],
  binaryPath: string,
  commit: string,
  label: string,
  env: Record<string, string>,
): Promise<RefusedStart> {
  const spawnedAt = Date.now();
  const server = await spawnBinary({
    binaryPath,
    commit,
    label,
    env,
    portTimeoutMs: REFUSAL_DEADLINE_MS,
  });
  running.push(server);
  let exit = server.exit();
  if (exit === null && server.port === null) {
    exit = await server.waitForExit(Math.max(0, spawnedAt + REFUSAL_DEADLINE_MS - Date.now()));
  }
  const printedPort = server.port !== null || /^PORT=/m.test(server.stdout());
  if (exit === null || printedPort) {
    exit = null;
    await server.stop('SIGKILL', 5_000);
  }
  return { exit, printedPort, output: `${server.stdout()}\n${server.stderr()}` };
}

function refusedAsOlder(start: RefusedStart | undefined): void {
  expect(start?.printedPort).toBe(false);
  expect(start?.exit).not.toBeNull();
  expect(typeof start?.exit?.code).toBe('number');
  expect(start?.exit?.code).not.toBe(0);
  expect(start?.output).toContain('unknown metadata');
}

test(`the older binary is the build from ${WATERMARK_OLD_COMMIT_PREFIX}`, () => {
  const env = requireWatermarkRollbackEnv();
  expect(env.oldCommit.startsWith(WATERMARK_OLD_COMMIT_PREFIX)).toBe(true);
});

/** What W-A and W-B observe. */
interface RefusalCase {
  writes: Record<string, string[] | string>;
  newStop?: BinaryExit | null;
  wal?: WalFiles;
  before?: TreeSnapshot;
  refused?: RefusedStart;
  after?: TreeSnapshot;
  againPort?: number | null;
  values: Record<string, unknown>;
  againUnreplayed?: string[];
  againStop?: BinaryExit | null;
}

function refusalCase(title: string, tag: string, stop: 'SIGTERM' | 'SIGKILL'): void {
  describe(title, () => {
    const o: RefusalCase = { writes: {}, values: {} };
    let dataDir: string | null = null;
    const running: SpawnedBinary[] = [];

    beforeAll(async () => {
      const env = requireWatermarkRollbackEnv();
      dataDir = freshDir(tag);
      const dir = dataDir;

      // Under kill -9 the flush interval is set far beyond the test's duration,
      // so every acknowledged write exists only as a WAL frame at the kill and
      // the restarted newer binary must replay it.
      const firstEnv =
        stop === 'SIGKILL'
          ? storeEnv(dir, { TOPGUN_WRITEBEHIND_FLUSH_INTERVAL_MS: '600000' })
          : storeEnv(dir);
      const [first, port] = await startServing(
        running,
        env.newBinary,
        env.newCommit,
        `${tag}: NEW first run`,
        firstEnv,
      );
      o.writes = await writeAll(port, tag, 'new');
      if (stop === 'SIGTERM') await sleep(FLUSH_SETTLE_MS);
      o.newStop = await first.stop(stop, stop === 'SIGKILL' ? 5_000 : STOP_TIMEOUT_MS);
      o.wal = walFiles(path.join(dir, 'wal'));

      o.before = guardedSnapshot(dir);
      o.refused = await startExpectingRefusal(
        running,
        env.oldBinary,
        env.oldCommit,
        `${tag}: OLD`,
        storeEnv(dir),
      );
      o.after = guardedSnapshot(dir);

      const again = await spawnBinary({
        binaryPath: env.newBinary,
        commit: env.newCommit,
        label: `${tag}: NEW again`,
        env: storeEnv(dir),
      });
      running.push(again);
      o.againPort = again.port;
      if (again.port !== null) o.values = await readAll(again.port, tag);
      o.againUnreplayed = unreplayedLines(again);
      o.againStop = await again.stop('SIGTERM', STOP_TIMEOUT_MS);

      // Straight to stderr: the suite silences console.log.
      process.stderr.write(
        `[watermark-rollback] ${tag}, observed:\n${JSON.stringify(o, null, 2)}\n` +
          running.map(streams).join(''),
      );
    }, CASE_TIMEOUT_MS);

    afterAll(async () => {
      for (const server of running) await server.stop('SIGKILL', 5_000);
      if (dataDir) fs.rmSync(dataDir, { recursive: true, force: true });
    });

    test('setup: the newer binary acknowledged every write and was stopped as the case says', () => {
      expect(o.writes).toEqual(allAcked());
      expect(o.newStop).toEqual(
        stop === 'SIGKILL' ? { code: null, signal: 'SIGKILL' } : CLEAN_EXIT,
      );
    });

    test('the WAL directory holds wal-collected', () => {
      expect(o.wal?.collected).toBe(true);
    });

    if (stop === 'SIGTERM') {
      test('the clean stop wrote 8-byte rollback hints and left no segment', () => {
        // A drained stop with nothing above the watermark unlinks every
        // segment it covers, so the hints are what is left of the WAL.
        const hints = Object.values(o.wal?.applied ?? {});
        expect(hints.length).toBeGreaterThanOrEqual(1);
        expect(hints.filter((size) => size !== 8)).toEqual([]);
        expect(o.wal?.segmentPartitions).toEqual([]);
      });
    } else {
      test('after kill -9 the WAL directory holds no .applied file', () => {
        expect(o.wal?.appliedNames).toEqual([]);
      });
    }

    test('the older binary exits non-zero within 30 s on unknown metadata and never prints PORT=', () => {
      refusedAsOlder(o.refused);
    });

    test('the refused start changed no file of the WAL directory, not the marker file, and no path', () => {
      expect(o.before?.paths.length).toBeGreaterThan(0);
      expect(o.before?.sha256['topgun.redb']).toBeDefined();
      expect(o.after).toEqual(o.before);
    });

    test('the newer binary started again serves every value and reports nothing unreplayed', () => {
      expect(typeof o.againPort).toBe('number');
      expect(o.values).toEqual(expectedValues('new'));
      expect(o.againUnreplayed).toEqual([]);
      expect(o.againStop).toEqual(CLEAN_EXIT);
    });
  });
}

refusalCase(
  'Integration: W-A, the older binary refuses the data directory after a clean stop',
  'wa',
  'SIGTERM',
);
refusalCase(
  'Integration: W-B, the older binary refuses the data directory after kill -9',
  'wb',
  'SIGKILL',
);

describe('Integration: W-C, the newer binary imports a directory the older binary wrote', () => {
  const o: {
    oldWrites: Record<string, string[] | string>;
    oldStop?: BinaryExit | null;
    oldWal?: WalFiles;
    newPort?: number | null;
    walWhileNew?: WalFiles;
    newValues: Record<string, unknown>;
    newUnreplayed?: string[];
    newStop?: BinaryExit | null;
    refused?: RefusedStart;
  } = { oldWrites: {}, newValues: {} };
  let dataDir: string | null = null;
  const running: SpawnedBinary[] = [];

  beforeAll(async () => {
    const env = requireWatermarkRollbackEnv();
    dataDir = freshDir('wc');
    const dir = dataDir;
    const walDir = path.join(dir, 'wal');

    // --- The older binary writes and stops cleanly: its watermarks are .applied files.
    const [old, oldPort] = await startServing(
      running,
      env.oldBinary,
      env.oldCommit,
      'wc: OLD',
      storeEnv(dir),
    );
    o.oldWrites = await writeAll(oldPort, 'wc-old', 'old');
    await sleep(FLUSH_SETTLE_MS);
    o.oldStop = await old.stop('SIGTERM', STOP_TIMEOUT_MS);
    o.oldWal = walFiles(walDir);

    // --- The newer binary opens the directory.
    const fresh = await spawnBinary({
      binaryPath: env.newBinary,
      commit: env.newCommit,
      label: 'wc: NEW',
      env: storeEnv(dir),
    });
    running.push(fresh);
    o.newPort = fresh.port;
    if (fresh.port !== null) {
      o.walWhileNew = walFiles(walDir);
      o.newValues = await readAll(fresh.port, 'wc-new');
    }
    o.newUnreplayed = unreplayedLines(fresh);
    o.newStop = await fresh.stop('SIGTERM', STOP_TIMEOUT_MS);

    // --- The older binary meets the marker the import wrote.
    o.refused = await startExpectingRefusal(
      running,
      env.oldBinary,
      env.oldCommit,
      'wc: OLD again',
      storeEnv(dir),
    );

    // Straight to stderr: the suite silences console.log.
    process.stderr.write(
      `[watermark-rollback] wc, observed:\n${JSON.stringify(o, null, 2)}\n` +
        running.map(streams).join(''),
    );
  }, CASE_TIMEOUT_MS);

  afterAll(async () => {
    for (const server of running) await server.stop('SIGKILL', 5_000);
    if (dataDir) fs.rmSync(dataDir, { recursive: true, force: true });
  });

  test('setup: the older binary acknowledged every write, stopped cleanly and left .applied files', () => {
    expect(o.oldWrites).toEqual(allAcked());
    expect(o.oldStop).toEqual(CLEAN_EXIT);
    expect(Object.keys(o.oldWal?.applied ?? {}).length).toBeGreaterThanOrEqual(1);
    expect(o.oldWal?.collected).toBe(false);
  });

  test('while the newer binary runs, the .applied files are gone and wal-collected exists', () => {
    expect(typeof o.newPort).toBe('number');
    expect(o.walWhileNew?.appliedNames).toEqual([]);
    expect(o.walWhileNew?.collected).toBe(true);
  });

  test('the newer binary serves every value the older one wrote and reports nothing unreplayed', () => {
    expect(o.newValues).toEqual(expectedValues('old'));
    expect(o.newUnreplayed).toEqual([]);
    expect(o.newStop).toEqual(CLEAN_EXIT);
  });

  test('the older binary started afterwards refuses the directory: the import marked it', () => {
    refusedAsOlder(o.refused);
  });
});
