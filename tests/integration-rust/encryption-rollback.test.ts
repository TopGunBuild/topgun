/**
 * Integration: an older binary refuses a data directory that has had
 * encryption at rest on (TG-ENC-009).
 *
 * The newer binary keeps the wrapped data keys in a `keyring` metadata row of
 * the data directory, and keeps that row after encryption is turned off again.
 * The older binary (built at 727db547) does not know the row and refuses the
 * store at its open — before it opens the WAL and before it accepts a
 * connection — so it never reads sealed values as if they were plain. Three
 * cases:
 *
 * - E-A clean stop and E-B `kill -9` — the newer binary writes with encryption
 *   on and stops; the older binary started on the directory exits non-zero
 *   naming the unknown metadata, never prints `PORT=`, and changes no file of
 *   the WAL directory and not the marker file at the old store path; the newer
 *   binary started again with the key serves every value. After the clean stop
 *   the WAL directory holds no segment.
 * - E-C — on, then off: the newer binary unseals the store; the older binary is
 *   still refused, and the newer binary serves every value.
 *
 * Two binaries are needed, each given by path together with the commit it was
 * built from (ENC_OLD_SERVER_BINARY / ENC_OLD_SERVER_COMMIT for the older one,
 * RUST_SERVER_BINARY / NEW_SERVER_COMMIT for the newer). With any of them
 * missing the test fails; it never runs one binary in both roles.
 */

import * as crypto from 'crypto';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

import {
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

const ENC_OLD_COMMIT_PREFIX = '727db547';

const BUILD_HINT =
  `Encryption rollback: build the server at ${ENC_OLD_COMMIT_PREFIX} (the last binary without the ` +
  'keyring) in its own worktree and target directory (`git worktree add <scratch>/topgun-enc-old ' +
  `${ENC_OLD_COMMIT_PREFIX}\`, then \`cargo build --release -p topgun-server --manifest-path ` +
  '<scratch>/topgun-enc-old/Cargo.toml --target-dir <scratch>/topgun-enc-old-target`), build the newer ' +
  'one in this checkout, and export ENC_OLD_SERVER_BINARY, ENC_OLD_SERVER_COMMIT (the full commit the ' +
  'older binary was built from), RUST_SERVER_BINARY and NEW_SERVER_COMMIT.';

interface Binaries {
  oldBinary: string;
  oldCommit: string;
  newBinary: string;
  newCommit: string;
}

function executable(name: string): string {
  const value = process.env[name] as string;
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

/** The four variables, or a failure naming every missing one; never a skip. */
function requireBinaries(): Binaries {
  const names = [
    'ENC_OLD_SERVER_BINARY',
    'ENC_OLD_SERVER_COMMIT',
    'RUST_SERVER_BINARY',
    'NEW_SERVER_COMMIT',
  ];
  const missing = names.filter((name) => !process.env[name]);
  if (missing.length > 0) {
    throw new Error(`${missing.join(', ')} not set. ${BUILD_HINT}`);
  }
  const oldCommit = process.env.ENC_OLD_SERVER_COMMIT as string;
  if (!oldCommit.startsWith(ENC_OLD_COMMIT_PREFIX)) {
    throw new Error(
      `ENC_OLD_SERVER_COMMIT is ${oldCommit}, not a commit starting ${ENC_OLD_COMMIT_PREFIX}. ${BUILD_HINT}`,
    );
  }
  return {
    oldBinary: executable('ENC_OLD_SERVER_BINARY'),
    oldCommit,
    newBinary: executable('RUST_SERVER_BINARY'),
    newCommit: process.env.NEW_SERVER_COMMIT as string,
  };
}

const MAP = 'encryption_rollback_items';
const KEYS = ['k0', 'k1', 'k2', 'k3', 'k4', 'k5'];
const ACK_WAIT_MS = 5_000;
const FLUSH_SETTLE_MS = 5_000;
const REFUSAL_DEADLINE_MS = 30_000;
const STOP_TIMEOUT_MS = 40_000;
const CASE_TIMEOUT_MS = 300_000;

const CLEAN_EXIT: BinaryExit = { code: 0, signal: null };

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

function keyEnv(dir: string, mode: 'on' | 'off'): Record<string, string> {
  return {
    TOPGUN_ENCRYPTION_AT_REST: mode,
    TOPGUN_MASTER_KEY_PROVIDER: 'file',
    TOPGUN_MASTER_KEY_FILE: path.join(dir, 'master.key'),
  };
}

function valueOf(key: string, run: string): Record<string, string> {
  return { key, run };
}

function expectedValues(run: string): Record<string, unknown> {
  return Object.fromEntries(KEYS.map((key) => [key, valueOf(key, run)]));
}

async function writeAll(port: number, tag: string): Promise<Record<string, unknown>> {
  const answers: Record<string, unknown> = {};
  for (const [i, key] of KEYS.entries()) {
    answers[key] = await observe(async () => {
      const client = await connectWithin(port, `${tag}-writer-${i}`);
      try {
        client.messages.length = 0;
        client.send({
          type: 'CLIENT_OP',
          payload: {
            id: String(i + 1),
            mapName: MAP,
            opType: 'PUT',
            key,
            record: {
              value: valueOf(key, 'new'),
              timestamp: { millis: Date.now(), counter: 0, nodeId: 'encryption' },
            },
          },
        });
        const frame = await firstFrame(client, ['OP_ACK', 'OP_REJECTED', 'ERROR'], ACK_WAIT_MS);
        return frame?.type ?? 'none';
      } finally {
        client.close();
      }
    });
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

/** The WAL directory's files and the marker file at the old store path are hashed; the engine file is not, as an open commits. */
function guardedSnapshot(dir: string): TreeSnapshot {
  return treeSnapshot(dir, (rel) => rel.startsWith(`wal${path.sep}`) || rel === 'topgun.redb');
}

function walSegments(dir: string): string[] {
  const walDir = path.join(dir, 'wal');
  if (!fs.existsSync(walDir)) return [];
  return fs
    .readdirSync(walDir)
    .filter((name) => name.endsWith('.log'))
    .sort();
}

function unreplayedLines(server: SpawnedBinary): string[] {
  return plainLines(server.stdout())
    .filter((l) => l.includes('BootUnreplayed') || l.includes('replay failed for entry'))
    .map((l) => l.slice(0, 400));
}

interface RefusedStart {
  exit: BinaryExit | null;
  printedPort: boolean;
  unknownMetadata: boolean;
}

async function startExpectingRefusal(
  running: SpawnedBinary[],
  binaries: Binaries,
  label: string,
  env: Record<string, string>,
): Promise<RefusedStart> {
  const server = await spawnBinary({
    binaryPath: binaries.oldBinary,
    commit: binaries.oldCommit,
    label,
    env,
    portTimeoutMs: REFUSAL_DEADLINE_MS,
  });
  running.push(server);
  let exit = server.exit();
  if (exit === null && server.port === null) exit = await server.waitForExit(REFUSAL_DEADLINE_MS);
  const printedPort = server.port !== null || /^PORT=/m.test(server.stdout());
  if (exit === null || printedPort) {
    exit = null;
    await server.stop('SIGKILL', 5_000);
  }
  return {
    exit,
    printedPort,
    unknownMetadata: `${server.stdout()}\n${server.stderr()}`.includes('unknown metadata'),
  };
}

type Run = 'clean' | 'kill' | 'on-then-off';

interface Observed {
  writes: Record<string, unknown>;
  firstStop?: BinaryExit | null;
  offStop?: BinaryExit | null;
  segmentsAfterStop?: string[];
  before?: TreeSnapshot;
  refused?: RefusedStart;
  after?: TreeSnapshot;
  againPort?: number | null;
  values: Record<string, unknown>;
  againUnreplayed?: string[];
  againStop?: BinaryExit | null;
}

function rollbackCase(title: string, tag: string, run: Run): void {
  describe(title, () => {
    const o: Observed = { writes: {}, values: {} };
    let dir: string | null = null;
    const running: SpawnedBinary[] = [];

    beforeAll(async () => {
      const binaries = requireBinaries();
      dir = fs.mkdtempSync(path.join(os.tmpdir(), `topgun-encryption-rollback-${tag}-`));
      const key = path.join(dir, 'master.key');
      fs.writeFileSync(key, `${crypto.randomBytes(32).toString('hex')}\n`, { mode: 0o600 });

      // Under kill -9 every acknowledged write exists only as a WAL frame at the kill.
      const flush: Record<string, string> =
        run === 'kill' ? { TOPGUN_WRITEBEHIND_FLUSH_INTERVAL_MS: '600000' } : {};
      const first = await spawnBinary({
        binaryPath: binaries.newBinary,
        commit: binaries.newCommit,
        label: `${tag}: NEW on`,
        env: storeEnv(dir, { ...keyEnv(dir, 'on'), ...flush }),
      });
      running.push(first);
      if (first.port === null) throw new Error(`NEW did not start:\n${first.stderr()}`);
      o.writes = await writeAll(first.port, tag);
      if (run !== 'kill') await sleep(FLUSH_SETTLE_MS);
      o.firstStop = await first.stop(
        run === 'kill' ? 'SIGKILL' : 'SIGTERM',
        run === 'kill' ? 5_000 : STOP_TIMEOUT_MS,
      );

      const mode = run === 'on-then-off' ? 'off' : 'on';
      if (run === 'on-then-off') {
        const off = await spawnBinary({
          binaryPath: binaries.newBinary,
          commit: binaries.newCommit,
          label: `${tag}: NEW off`,
          env: storeEnv(dir, keyEnv(dir, 'off')),
        });
        running.push(off);
        o.offStop = await off.stop('SIGTERM', STOP_TIMEOUT_MS);
      }
      o.segmentsAfterStop = walSegments(dir);

      o.before = guardedSnapshot(dir);
      o.refused = await startExpectingRefusal(running, binaries, `${tag}: OLD`, storeEnv(dir));
      o.after = guardedSnapshot(dir);

      const again = await spawnBinary({
        binaryPath: binaries.newBinary,
        commit: binaries.newCommit,
        label: `${tag}: NEW again`,
        env: storeEnv(dir, keyEnv(dir, mode)),
      });
      running.push(again);
      o.againPort = again.port;
      if (again.port !== null) o.values = await readAll(again.port, tag);
      o.againUnreplayed = unreplayedLines(again);
      o.againStop = await again.stop('SIGTERM', STOP_TIMEOUT_MS);

      process.stderr.write(
        `[encryption-rollback] ${tag}, observed:\n${JSON.stringify(o, null, 2)}\n`,
      );
    }, CASE_TIMEOUT_MS);

    afterAll(async () => {
      for (const server of running) await server.stop('SIGKILL', 5_000);
      if (dir) fs.rmSync(dir, { recursive: true, force: true });
    });

    test('setup: the newer binary acknowledged every write and was stopped as the case says', () => {
      expect(o.writes).toEqual(Object.fromEntries(KEYS.map((key) => [key, 'OP_ACK'])));
      expect(o.firstStop).toEqual(run === 'kill' ? { code: null, signal: 'SIGKILL' } : CLEAN_EXIT);
      if (run === 'on-then-off') expect(o.offStop).toEqual(CLEAN_EXIT);
    });

    if (run !== 'kill') {
      test('after a drained stop with nothing unapplied the WAL directory holds no segment', () => {
        expect(o.segmentsAfterStop).toEqual([]);
      });
    }

    test('the older binary exits non-zero on unknown metadata and never prints PORT=', () => {
      expect(o.refused?.printedPort).toBe(false);
      expect(o.refused?.exit).not.toBeNull();
      expect(typeof o.refused?.exit?.code).toBe('number');
      expect(o.refused?.exit?.code).not.toBe(0);
      expect(o.refused?.unknownMetadata).toBe(true);
    });

    test('the refused start changed no file of the WAL directory and not the marker file', () => {
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

test(`the older binary is the build from ${ENC_OLD_COMMIT_PREFIX}`, () => {
  expect(requireBinaries().oldCommit.startsWith(ENC_OLD_COMMIT_PREFIX)).toBe(true);
});

rollbackCase(
  'Integration: E-A, the older binary refuses a sealed store after a clean stop',
  'ea',
  'clean',
);
rollbackCase(
  'Integration: E-B, the older binary refuses a sealed store after kill -9',
  'eb',
  'kill',
);
rollbackCase(
  'Integration: E-C, the older binary refuses a store sealed and then unsealed',
  'ec',
  'on-then-off',
);
