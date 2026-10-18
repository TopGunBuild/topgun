/**
 * Integration: what rolling the server binary back does once the newer binary
 * has moved the store into the data directory.
 *
 * The older binary (built at b346e277) keeps the store as one redb file at
 * `TOPGUN_REDB_PATH`, one table per map. The newer binary keeps it in the data
 * directory `topgun-data/` beside that path, migrates an old file into it once,
 * keeps the old file as `topgun.redb.pre-migration`, and leaves a small marker
 * file at the old path so that the older binary refuses to start instead of
 * serving an empty store. The older binary cannot read the new store; the only
 * supported rollback is a return to the kept pre-migration file. Five cases:
 *
 * - R-A clean stop and R-B `kill -9` — the newer binary writes on a fresh
 *   store and stops; the older binary, started on the marker file and then on
 *   the data directory itself, exits non-zero, never prints `PORT=` and
 *   changes no file; the newer binary started again serves every value.
 * - R-C documented rollback — the older binary writes `v0`, the newer one
 *   migrates and writes `v1`, the operator's steps restore the pre-migration
 *   file, and the older binary serves `v0` (the write acknowledged after the
 *   migration is gone, as documented) and writes `v2`.
 * - R-D roll forward again — after R-C the newer binary migrates once more and
 *   serves `v2`; the older binary then meets the new marker file and refuses as
 *   in R-A.
 * - R-E two stores — a legacy file put back at the old path beside an existing
 *   data directory is refused by the newer binary without touching either; with
 *   the legacy file removed the newer binary serves what it served before.
 *
 * Two binaries are needed, each given by path together with the commit it was
 * built from (FORMAT_OLD_SERVER_BINARY / FORMAT_OLD_SERVER_COMMIT for the older
 * one, RUST_SERVER_BINARY / NEW_SERVER_COMMIT for the newer). With any of them
 * missing the test fails; it never runs one binary in both roles on its own
 * initiative.
 */

import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

import { createORRecord } from './helpers';
import {
  requireFormatRollbackEnv,
  spawnBinary,
  connectWithin,
  firstFrame,
  queryKey,
  merkleKey,
  orMapValues,
  observe,
  sleep,
  plainLines,
  movedMarkerState,
  treeSnapshot,
  SpawnedBinary,
  BinaryExit,
  TwoBinaryEnv,
  MovedMarkerState,
  TreeSnapshot,
} from './helpers/two-binary';

const CONTROL_MAP = 'user_profiles';
const DASH_MAP = 'user-profiles';
const SLASH_MAP = 'users/profiles';
const OR_MAP = 'users/colours';
const KEY = 'alice';

const VALUE_CONTROL = { name: 'Alice', map: 'control' };
const VALUE_DASH = { name: 'Alice', map: 'dash' };
const VALUE_SLASH = { name: 'Alice', map: 'slash' };
const OR_VALUE = 'blue';

const V0 = { version: 'v0' };
const V1 = { version: 'v1' };
const V2 = { version: 'v2' };

const ACK_WAIT_MS = 5_000;
const FLUSH_SETTLE_MS = 10_000;
const STAY_UP_MS = 10_000;
/** A binary that must refuse to start has this long, from its spawn, to exit. */
const REFUSAL_DEADLINE_MS = 30_000;
/** A graceful stop may spend up to 30 s draining buffered writes; past this the process is killed and the exit status says so. */
const STOP_TIMEOUT_MS = 40_000;
const CASE_TIMEOUT_MS = 600_000;

const CLEAN_EXIT: BinaryExit = { code: 0, signal: null };

function storeEnv(dir: string, extra: Record<string, string> = {}): Record<string, string> {
  return {
    STORAGE_BACKEND: 'redb',
    TOPGUN_REDB_PATH: path.join(dir, 'topgun.redb'),
    TOPGUN_WAL_DIR: path.join(dir, 'wal'),
    TOPGUN_WAL_FSYNC_POLICY: 'per_op',
    ...extra,
  };
}

function fatalLines(server: SpawnedBinary): string[] {
  return [...plainLines(server.stdout()), ...plainLines(server.stderr())].filter((line) =>
    line.includes('FATAL'),
  );
}

function unreplayedLines(server: SpawnedBinary): string[] {
  return plainLines(server.stdout())
    .filter((l) => l.includes('BootUnreplayed') || l.includes('replay failed for entry'))
    .map((l) => l.slice(0, 400));
}

function countLines(server: SpawnedBinary, needle: string): number {
  return plainLines(server.stdout()).filter((l) => l.includes(needle)).length;
}

/** Both streams of a server, for the record: the stdout without the per-index startup lines. */
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

function lwwPut(id: string, mapName: string, value: unknown): Record<string, unknown> {
  return {
    id,
    mapName,
    opType: 'PUT',
    key: KEY,
    record: { value, timestamp: { millis: Date.now(), counter: 0, nodeId: 'rollback' } },
  };
}

function orAdd(id: string): Record<string, unknown> {
  return {
    id,
    mapName: OR_MAP,
    opType: 'OR_ADD',
    key: KEY,
    orRecord: createORRecord(OR_VALUE, 'rollback'),
  };
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

function exitedNonZero(start: RefusedStart | undefined): boolean {
  return typeof start?.exit?.code === 'number' && start.exit.code !== 0;
}

function freshDir(tag: string): string {
  return fs.mkdtempSync(path.join(os.tmpdir(), `topgun-format-rollback-${tag}-`));
}

test('the older binary is the build from b346e277', () => {
  const env = requireFormatRollbackEnv();
  expect(env.oldCommit.startsWith('b346e277')).toBe(true);
});

/** What R-A and R-B observe. */
interface RefusalCase {
  writes: Record<string, string[] | string>;
  newStop?: BinaryExit | null;
  marker?: MovedMarkerState;
  before?: TreeSnapshot;
  onMarker?: RefusedStart;
  afterOnMarker?: TreeSnapshot;
  onDataDir?: RefusedStart;
  afterOnDataDir?: TreeSnapshot;
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
      const env: TwoBinaryEnv = requireFormatRollbackEnv();
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
      const writes: Array<[string, Record<string, unknown>]> = [
        [CONTROL_MAP, lwwPut('1', CONTROL_MAP, VALUE_CONTROL)],
        [DASH_MAP, lwwPut('2', DASH_MAP, VALUE_DASH)],
        [SLASH_MAP, lwwPut('3', SLASH_MAP, VALUE_SLASH)],
        [OR_MAP, orAdd('4')],
      ];
      for (const [i, [mapName, payload]] of writes.entries()) {
        o.writes[mapName] = await observe(() =>
          writeAndCollect(port, `${tag}-writer-${i}`, payload),
        );
      }
      if (stop === 'SIGTERM') await sleep(FLUSH_SETTLE_MS);
      o.newStop = await first.stop(stop, stop === 'SIGKILL' ? 5_000 : STOP_TIMEOUT_MS);

      o.marker = movedMarkerState(path.join(dir, 'topgun.redb'));
      o.before = treeSnapshot(dir);
      o.onMarker = await startExpectingRefusal(
        running,
        env.oldBinary,
        env.oldCommit,
        `${tag}: OLD on the marker file`,
        storeEnv(dir),
      );
      o.afterOnMarker = treeSnapshot(dir);
      o.onDataDir = await startExpectingRefusal(
        running,
        env.oldBinary,
        env.oldCommit,
        `${tag}: OLD on the data directory`,
        storeEnv(dir, { TOPGUN_REDB_PATH: path.join(dir, 'topgun-data') }),
      );
      o.afterOnDataDir = treeSnapshot(dir);

      const again = await spawnBinary({
        binaryPath: env.newBinary,
        commit: env.newCommit,
        label: `${tag}: NEW again`,
        env: storeEnv(dir),
      });
      running.push(again);
      o.againPort = again.port;
      if (again.port !== null) {
        const p = again.port;
        o.values[`query ${CONTROL_MAP}`] = await observe(() =>
          queryKey(p, CONTROL_MAP, KEY, `${tag}-q-0`),
        );
        for (const [i, mapName] of [DASH_MAP, SLASH_MAP].entries()) {
          o.values[`query ${mapName}`] = await observe(() =>
            queryKey(p, mapName, KEY, `${tag}-q-${i + 1}`),
          );
          o.values[`merkle ${mapName}`] = await observe(() =>
            merkleKey(p, mapName, KEY, `${tag}-m-${i + 1}`),
          );
        }
        o.values[`or ${OR_MAP}`] = await observe(() => orMapValues(p, OR_MAP, KEY, `${tag}-or`));
      }
      o.againUnreplayed = unreplayedLines(again);
      o.againStop = await again.stop('SIGTERM', STOP_TIMEOUT_MS);

      // Straight to stderr: the suite silences console.log.
      process.stderr.write(
        `[map-name-rollback] ${tag}, observed:\n${JSON.stringify(o, null, 2)}\n` +
          running.map(streams).join(''),
      );
    }, CASE_TIMEOUT_MS);

    afterAll(async () => {
      for (const server of running) await server.stop('SIGKILL', 5_000);
      if (dataDir) fs.rmSync(dataDir, { recursive: true, force: true });
    });

    test('setup: the newer binary acknowledged the four writes and was stopped as the case says', () => {
      expect(o.writes).toEqual({
        [CONTROL_MAP]: ['OP_ACK'],
        [DASH_MAP]: ['OP_ACK'],
        [SLASH_MAP]: ['OP_ACK'],
        [OR_MAP]: ['OP_ACK'],
      });
      expect(o.newStop).toEqual(
        stop === 'SIGKILL' ? { code: null, signal: 'SIGKILL' } : CLEAN_EXIT,
      );
    });

    test('the old store path holds the marker file', () => {
      expect(o.marker?.kind).toBe('file');
      expect(o.marker?.firstLine).toBe('TOPGUN-STORE-MOVED 1');
      expect(o.marker?.size).toBeLessThanOrEqual(16_384);
      expect(o.marker?.isMarker).toBe(true);
    });

    test('the older binary on the marker file exits non-zero within 30 s and never prints PORT=', () => {
      expect(o.onMarker?.printedPort).toBe(false);
      expect(o.onMarker?.exit).not.toBeNull();
      expect(exitedNonZero(o.onMarker)).toBe(true);
    });

    test('the older binary on the data directory exits non-zero within 30 s and never prints PORT=', () => {
      expect(o.onDataDir?.printedPort).toBe(false);
      expect(o.onDataDir?.exit).not.toBeNull();
      expect(exitedNonZero(o.onDataDir)).toBe(true);
    });

    test('neither refused start changed, created or removed any file under the directory', () => {
      expect(o.before?.paths.length).toBeGreaterThan(0);
      expect(o.afterOnMarker).toEqual(o.before);
      expect(o.afterOnDataDir).toEqual(o.before);
    });

    test('the newer binary started again serves every value and reports nothing unreplayed', () => {
      expect(typeof o.againPort).toBe('number');
      expect(o.values).toEqual({
        [`query ${CONTROL_MAP}`]: VALUE_CONTROL,
        [`query ${DASH_MAP}`]: VALUE_DASH,
        [`merkle ${DASH_MAP}`]: VALUE_DASH,
        [`query ${SLASH_MAP}`]: VALUE_SLASH,
        [`merkle ${SLASH_MAP}`]: VALUE_SLASH,
        [`or ${OR_MAP}`]: [OR_VALUE],
      });
      expect(o.againUnreplayed).toEqual([]);
      expect(o.againStop).toEqual(CLEAN_EXIT);
    });
  });
}

refusalCase(
  'Integration: R-A, the older binary refuses the moved store after a clean stop',
  'ra',
  'SIGTERM',
);
refusalCase(
  'Integration: R-B, the older binary refuses the moved store after kill -9',
  'rb',
  'SIGKILL',
);

describe('Integration: R-C, the documented rollback to the pre-migration file, and R-D, rolling forward again', () => {
  const o: {
    oldV0Write?: string[] | string;
    oldV0Stop?: BinaryExit | null;
    newMigrations?: number;
    newV1Write?: string[] | string;
    newStop?: BinaryExit | null;
    restoredFile?: boolean;
    rollbackPort?: number | null;
    rollbackValue?: unknown;
    rollbackExitDuringWindow?: BinaryExit | null;
    rollbackValueAtEndOfWindow?: unknown;
    rollbackV2Write?: string[] | string;
    rollbackStop?: BinaryExit | null;
    rollbackFatal?: string[];
    forwardPort?: number | null;
    forwardMigrations?: number;
    forwardValue?: unknown;
    forwardStop?: BinaryExit | null;
    forwardMarker?: MovedMarkerState;
    before?: TreeSnapshot;
    refused?: RefusedStart;
    after?: TreeSnapshot;
  } = {};
  let dataDir: string | null = null;
  let movedAway: string | null = null;
  const running: SpawnedBinary[] = [];

  beforeAll(async () => {
    const env = requireFormatRollbackEnv();
    dataDir = freshDir('rc');
    movedAway = freshDir('rc-moved-away');
    const dir = dataDir;
    const legacy = path.join(dir, 'topgun.redb');

    // --- The older binary writes v0 on a fresh file.
    const [old, oldPort] = await startServing(
      running,
      env.oldBinary,
      env.oldCommit,
      'rc: OLD first run',
      storeEnv(dir),
    );
    o.oldV0Write = await observe(() =>
      writeAndCollect(oldPort, 'rc-writer-v0', lwwPut('1', CONTROL_MAP, V0)),
    );
    o.oldV0Stop = await old.stop('SIGTERM', STOP_TIMEOUT_MS);

    // --- The newer binary migrates the file and acknowledges v1.
    const [migrating, newPort] = await startServing(
      running,
      env.newBinary,
      env.newCommit,
      'rc: NEW (migrates)',
      storeEnv(dir),
    );
    o.newV1Write = await observe(() =>
      writeAndCollect(newPort, 'rc-writer-v1', lwwPut('2', CONTROL_MAP, V1)),
    );
    o.newStop = await migrating.stop('SIGTERM', STOP_TIMEOUT_MS);
    o.newMigrations = countLines(migrating, 'migration complete');

    // --- The operator's rollback: move the data directory away, delete the
    // marker file, put the pre-migration file back at the old path.
    fs.renameSync(path.join(dir, 'topgun-data'), path.join(movedAway, 'topgun-data'));
    fs.unlinkSync(legacy);
    fs.renameSync(path.join(dir, 'topgun.redb.pre-migration'), legacy);
    o.restoredFile = fs.lstatSync(legacy).isFile();

    // --- R-C: the older binary serves the state at migration time and writes v2.
    const rollback = await spawnBinary({
      binaryPath: env.oldBinary,
      commit: env.oldCommit,
      label: 'rc: OLD after the rollback',
      env: storeEnv(dir),
    });
    running.push(rollback);
    o.rollbackPort = rollback.port;
    o.rollbackExitDuringWindow = rollback.exit();
    if (rollback.port !== null) {
      const p = rollback.port;
      o.rollbackValue = await observe(() => queryKey(p, CONTROL_MAP, KEY, 'rc-old-read'));
      await sleep(STAY_UP_MS);
      o.rollbackExitDuringWindow = rollback.exit();
      o.rollbackValueAtEndOfWindow = await observe(() =>
        queryKey(p, CONTROL_MAP, KEY, 'rc-old-read-end'),
      );
      o.rollbackV2Write = await observe(() =>
        writeAndCollect(p, 'rc-writer-v2', lwwPut('3', CONTROL_MAP, V2)),
      );
    }
    o.rollbackStop = await rollback.stop('SIGTERM', STOP_TIMEOUT_MS);
    o.rollbackFatal = fatalLines(rollback);

    // --- R-D: the newer binary migrates again and serves v2.
    const forward = await spawnBinary({
      binaryPath: env.newBinary,
      commit: env.newCommit,
      label: 'rd: NEW (migrates again)',
      env: storeEnv(dir),
    });
    running.push(forward);
    o.forwardPort = forward.port;
    if (forward.port !== null) {
      const p = forward.port;
      o.forwardValue = await observe(() => queryKey(p, CONTROL_MAP, KEY, 'rd-new-read'));
    }
    o.forwardStop = await forward.stop('SIGTERM', STOP_TIMEOUT_MS);
    o.forwardMigrations = countLines(forward, 'migration complete');

    // --- R-D: the older binary meets the marker file that migration wrote.
    o.forwardMarker = movedMarkerState(legacy);
    o.before = treeSnapshot(dir);
    o.refused = await startExpectingRefusal(
      running,
      env.oldBinary,
      env.oldCommit,
      'rd: OLD on the marker file',
      storeEnv(dir),
    );
    o.after = treeSnapshot(dir);

    process.stderr.write(
      `[map-name-rollback] rc/rd, observed:\n${JSON.stringify(o, null, 2)}\n` +
        running.map(streams).join(''),
    );
  }, CASE_TIMEOUT_MS);

  afterAll(async () => {
    for (const server of running) await server.stop('SIGKILL', 5_000);
    if (dataDir) fs.rmSync(dataDir, { recursive: true, force: true });
    if (movedAway) fs.rmSync(movedAway, { recursive: true, force: true });
  });

  test('setup: the older binary acknowledged v0 and stopped cleanly; the newer one migrated, acknowledged v1 and stopped cleanly', () => {
    expect(o.oldV0Write).toEqual(['OP_ACK']);
    expect(o.oldV0Stop).toEqual(CLEAN_EXIT);
    expect(o.newMigrations).toBe(1);
    expect(o.newV1Write).toEqual(['OP_ACK']);
    expect(o.newStop).toEqual(CLEAN_EXIT);
    expect(o.restoredFile).toBe(true);
  });

  test('R-C: after the rollback the older binary serves v0 (the write acknowledged after the migration is absent) and stays up for 10 s', () => {
    expect(typeof o.rollbackPort).toBe('number');
    expect(o.rollbackValue).toEqual(V0);
    expect(o.rollbackExitDuringWindow).toBeNull();
    expect(o.rollbackValueAtEndOfWindow).toEqual(V0);
  });

  test('R-C: the older binary acknowledges v2, stops cleanly and reports no fatal error', () => {
    expect(o.rollbackV2Write).toEqual(['OP_ACK']);
    expect(o.rollbackStop).toEqual(CLEAN_EXIT);
    expect(o.rollbackFatal).toEqual([]);
  });

  test('R-D: the newer binary migrates exactly once more, serves v2 and stops cleanly', () => {
    expect(typeof o.forwardPort).toBe('number');
    expect(o.forwardMigrations).toBe(1);
    expect(o.forwardValue).toEqual(V2);
    expect(o.forwardStop).toEqual(CLEAN_EXIT);
  });

  test('R-D: the migration left the marker file at the old path', () => {
    expect(o.forwardMarker?.firstLine).toBe('TOPGUN-STORE-MOVED 1');
    expect(o.forwardMarker?.isMarker).toBe(true);
  });

  test('R-D: the older binary on that marker file exits non-zero within 30 s, never prints PORT= and changes no file', () => {
    expect(o.refused?.printedPort).toBe(false);
    expect(o.refused?.exit).not.toBeNull();
    expect(exitedNonZero(o.refused)).toBe(true);
    expect(o.before?.paths.length).toBeGreaterThan(0);
    expect(o.after).toEqual(o.before);
  });
});

describe('Integration: R-E, a legacy file beside an existing data directory is refused', () => {
  const o: {
    oldV0Write?: string[] | string;
    oldV0Stop?: BinaryExit | null;
    migratedValue?: unknown;
    migrations?: number;
    newStop?: BinaryExit | null;
    before?: TreeSnapshot;
    refused?: RefusedStart;
    refusalNamesBoth?: { twoStoresFound: boolean; dataDir: boolean; legacyPath: boolean };
    after?: TreeSnapshot;
    againPort?: number | null;
    againValue?: unknown;
    againMarker?: MovedMarkerState;
    againStop?: BinaryExit | null;
  } = {};
  let dataDir: string | null = null;
  let side: string | null = null;
  const running: SpawnedBinary[] = [];

  beforeAll(async () => {
    const env = requireFormatRollbackEnv();
    dataDir = freshDir('re');
    side = freshDir('re-side');
    const dir = dataDir;
    const legacy = path.join(dir, 'topgun.redb');
    const legacyCopy = path.join(side, 'topgun.redb');
    const engineDir = path.join(dir, 'topgun-data');

    const [old, oldPort] = await startServing(
      running,
      env.oldBinary,
      env.oldCommit,
      're: OLD',
      storeEnv(dir),
    );
    o.oldV0Write = await observe(() =>
      writeAndCollect(oldPort, 're-writer-v0', lwwPut('1', CONTROL_MAP, V0)),
    );
    o.oldV0Stop = await old.stop('SIGTERM', STOP_TIMEOUT_MS);
    fs.copyFileSync(legacy, legacyCopy);

    const [migrating, newPort] = await startServing(
      running,
      env.newBinary,
      env.newCommit,
      're: NEW (migrates)',
      storeEnv(dir),
    );
    o.migratedValue = await observe(() => queryKey(newPort, CONTROL_MAP, KEY, 're-new-read'));
    o.newStop = await migrating.stop('SIGTERM', STOP_TIMEOUT_MS);
    o.migrations = countLines(migrating, 'migration complete');

    // The legacy file goes back over the marker file; the data directory stays.
    fs.copyFileSync(legacyCopy, legacy);

    // The engine file's bytes are not compared: the refused start opens the
    // engine to read its migration mark, and that open commits. Its content is
    // checked through the server below instead.
    const outsideEngine = (rel: string) => rel.split(path.sep)[0] !== 'topgun-data';
    o.before = treeSnapshot(dir, outsideEngine);
    o.refused = await startExpectingRefusal(
      running,
      env.newBinary,
      env.newCommit,
      're: NEW with two stores',
      storeEnv(dir),
    );
    o.after = treeSnapshot(dir, outsideEngine);
    o.refusalNamesBoth = {
      twoStoresFound: o.refused.output.includes('two stores found'),
      dataDir: o.refused.output.includes(engineDir),
      legacyPath: o.refused.output.includes(legacy),
    };

    fs.unlinkSync(legacy);
    const again = await spawnBinary({
      binaryPath: env.newBinary,
      commit: env.newCommit,
      label: 're: NEW without the legacy file',
      env: storeEnv(dir),
    });
    running.push(again);
    o.againPort = again.port;
    if (again.port !== null) {
      const p = again.port;
      o.againValue = await observe(() => queryKey(p, CONTROL_MAP, KEY, 're-new-read-again'));
    }
    o.againStop = await again.stop('SIGTERM', STOP_TIMEOUT_MS);
    o.againMarker = movedMarkerState(legacy);

    process.stderr.write(
      `[map-name-rollback] re, observed:\n${JSON.stringify(o, null, 2)}\n` +
        running.map(streams).join(''),
    );
  }, CASE_TIMEOUT_MS);

  afterAll(async () => {
    for (const server of running) await server.stop('SIGKILL', 5_000);
    if (dataDir) fs.rmSync(dataDir, { recursive: true, force: true });
    if (side) fs.rmSync(side, { recursive: true, force: true });
  });

  test('setup: the older binary acknowledged v0 and stopped cleanly; the newer one migrated it and stopped cleanly', () => {
    expect(o.oldV0Write).toEqual(['OP_ACK']);
    expect(o.oldV0Stop).toEqual(CLEAN_EXIT);
    expect(o.migrations).toBe(1);
    expect(o.migratedValue).toEqual(V0);
    expect(o.newStop).toEqual(CLEAN_EXIT);
  });

  test('the newer binary with two stores exits non-zero within 30 s and never prints PORT=', () => {
    expect(o.refused?.printedPort).toBe(false);
    expect(o.refused?.exit).not.toBeNull();
    expect(exitedNonZero(o.refused)).toBe(true);
  });

  test('its output says two stores were found and names both absolute paths', () => {
    expect(o.refusalNamesBoth).toEqual({ twoStoresFound: true, dataDir: true, legacyPath: true });
  });

  test('the refused start changed no file outside the data directory and created or removed no path', () => {
    expect(Object.keys(o.before?.sha256 ?? {})).toEqual(
      expect.arrayContaining(['topgun.redb', 'topgun.redb.pre-migration']),
    );
    expect(o.after).toEqual(o.before);
  });

  test('with the legacy file removed the newer binary starts, writes the marker file back and serves v0 as before', () => {
    expect(typeof o.againPort).toBe('number');
    expect(o.againValue).toEqual(o.migratedValue);
    expect(o.againValue).toEqual(V0);
    expect(o.againMarker?.isMarker).toBe(true);
    expect(o.againStop).toEqual(CLEAN_EXIT);
  });
});
