/**
 * Integration: rolling the server binary back is safe after the newer binary
 * has stored maps the older one cannot name.
 *
 * The newer binary keeps maps whose names are not identifier-shaped
 * (`user-profiles`, `users/profiles`) in tables the older binary does not
 * look at. This test starts the older binary on a directory the newer one
 * wrote, and checks what an operator rolling back would see: the older binary
 * starts, reports no fatal error, serves the identifier-shaped map and stays
 * up; and when the newer binary returns, the other maps are there again.
 *
 * Two cases, each on a directory of its own:
 *
 * - clean stop — the newer binary flushed everything to the store before it
 *   was stopped;
 * - `kill -9` — the newer binary was killed with every write still only in the
 *   WAL (its flush interval is set far beyond the test's duration), so the
 *   older binary boots over frames it cannot apply. It must report them, keep
 *   serving, and leave them for the newer binary to replay.
 *
 * Two binaries are needed, each given by path together with the commit it was
 * built from (OLD_SERVER_BINARY / OLD_SERVER_COMMIT for the older one,
 * RUST_SERVER_BINARY / NEW_SERVER_COMMIT for the newer). With any of them
 * missing the test fails; it never runs one binary in both roles on its own
 * initiative.
 */

import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

import { createORRecord } from './helpers';
import {
  requireTwoBinaryEnv,
  spawnBinary,
  connectWithin,
  firstFrame,
  queryKey,
  merkleKey,
  orMapValues,
  observe,
  sleep,
  walPartitions,
  grownPartitions,
  plainLines,
  SpawnedBinary,
  BinaryExit,
  TwoBinaryEnv,
} from './helpers/two-binary';

const OLD_CLASS_MAP = 'user_profiles';
const DASH_MAP = 'user-profiles';
const SLASH_MAP = 'users/profiles';
const KEY = 'alice';

const VALUE_CONTROL = { name: 'Alice', map: 'control' };
const VALUE_DASH = { name: 'Alice', map: 'dash' };
const VALUE_SLASH = { name: 'Alice', map: 'slash' };
const OR_VALUE = 'blue';

const ACK_WAIT_MS = 5_000;
const FLUSH_SETTLE_MS = 10_000;
const STAY_UP_MS = 10_000;
/** A graceful stop may spend up to 30 s draining buffered writes; past this the process is killed and the exit status says so. */
const STOP_TIMEOUT_MS = 40_000;

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

/** What the older binary did on the directory, common to both cases. */
interface OldRun {
  port: number | null;
  fatal: string[];
  unreplayed: string[];
  controlValue?: unknown;
  /** Exit status at the end of the stay-up window; `null` means still running. */
  exitDuringWindow: BinaryExit | null;
  /**
   * The control map read again at the end of the window. A hung process has not
   * exited either, so only an answered read shows the binary is still serving.
   */
  controlValueAtEndOfWindow?: unknown;
  stop?: BinaryExit | null;
}

async function runOld(
  env: TwoBinaryEnv,
  dir: string,
  label: string,
): Promise<[SpawnedBinary, OldRun]> {
  const old = await spawnBinary({
    binaryPath: env.oldBinary,
    commit: env.oldCommit,
    label,
    env: storeEnv(dir),
  });
  const run: OldRun = { port: old.port, fatal: [], unreplayed: [], exitDuringWindow: old.exit() };
  if (old.port !== null) {
    const port = old.port;
    run.controlValue = await observe(() => queryKey(port, OLD_CLASS_MAP, KEY, `${label}-control`));
    await sleep(STAY_UP_MS);
    run.exitDuringWindow = old.exit();
    run.controlValueAtEndOfWindow = await observe(() =>
      queryKey(port, OLD_CLASS_MAP, KEY, `${label}-control-end`),
    );
  }
  run.unreplayed = unreplayedLines(old);
  run.stop = await old.stop('SIGTERM', STOP_TIMEOUT_MS);
  run.fatal = fatalLines(old);
  return [old, run];
}

describe('Integration: rollback to the older server binary after a clean stop', () => {
  const o: {
    writes: Record<string, string[] | string>;
    newAnsweredAfterSettle?: unknown;
    newStop?: BinaryExit | null;
    old?: OldRun;
    newAgainPort?: number | null;
    query: Record<string, unknown>;
    merkle: Record<string, unknown>;
  } = { writes: {}, query: {}, merkle: {} };
  let dataDir: string | null = null;
  const running: SpawnedBinary[] = [];

  beforeAll(async () => {
    const env = requireTwoBinaryEnv();
    dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'topgun-map-name-rollback-clean-'));
    const dir = dataDir;

    const first = await spawnBinary({
      binaryPath: env.newBinary,
      commit: env.newCommit,
      label: 'clean: NEW first run',
      env: storeEnv(dir),
    });
    running.push(first);
    if (first.port === null) throw new Error(`NEW did not print PORT=; stderr:\n${first.stderr()}`);
    const port = first.port;
    const values: Array<[string, unknown]> = [
      [OLD_CLASS_MAP, VALUE_CONTROL],
      [DASH_MAP, VALUE_DASH],
      [SLASH_MAP, VALUE_SLASH],
    ];
    for (const [i, [mapName, value]] of values.entries()) {
      o.writes[mapName] = await observe(() =>
        writeAndCollect(port, `clean-writer-${i}`, lwwPut(String(i + 1), mapName, value)),
      );
    }
    await sleep(FLUSH_SETTLE_MS);
    // Recorded, not asserted: whether the writing binary still answers after its flush attempts.
    o.newAnsweredAfterSettle = await observe(() =>
      queryKey(port, OLD_CLASS_MAP, KEY, 'clean-liveness'),
    );
    o.newStop = await first.stop('SIGTERM', STOP_TIMEOUT_MS);

    const [old, oldRun] = await runOld(env, dir, 'clean: OLD');
    running.push(old);
    o.old = oldRun;

    const again = await spawnBinary({
      binaryPath: env.newBinary,
      commit: env.newCommit,
      label: 'clean: NEW again',
      env: storeEnv(dir),
    });
    running.push(again);
    o.newAgainPort = again.port;
    if (again.port !== null) {
      const againPort = again.port;
      for (const [i, mapName] of [DASH_MAP, SLASH_MAP].entries()) {
        o.query[mapName] = await observe(() => queryKey(againPort, mapName, KEY, `clean-q-${i}`));
        o.merkle[mapName] = await observe(() => merkleKey(againPort, mapName, KEY, `clean-m-${i}`));
      }
    }
    await again.stop('SIGTERM', STOP_TIMEOUT_MS);

    // Straight to stderr: the suite silences console.log.
    process.stderr.write(
      `[map-name-rollback] clean stop, observed:\n${JSON.stringify(o, null, 2)}\n${streams(old)}`,
    );
  }, 420_000);

  afterAll(async () => {
    for (const server of running) await server.stop('SIGKILL', 5_000);
    if (dataDir) fs.rmSync(dataDir, { recursive: true, force: true });
  });

  test('setup: the newer binary acknowledged the three writes and stopped cleanly', () => {
    expect(o.writes).toEqual({
      [OLD_CLASS_MAP]: ['OP_ACK'],
      [DASH_MAP]: ['OP_ACK'],
      [SLASH_MAP]: ['OP_ACK'],
    });
    expect(o.newStop).toEqual({ code: 0, signal: null });
  });

  test('R8-1: the older binary prints PORT=', () => {
    expect(typeof o.old?.port).toBe('number');
  });

  test('R8-2: neither stream of the older binary contains FATAL', () => {
    expect(o.old?.fatal).toEqual([]);
  });

  test('R8-3: the older binary returns the identifier-named map the newer one wrote', () => {
    expect(o.old?.controlValue).toEqual(VALUE_CONTROL);
  });

  test('R8-4: the older binary is still serving at the end of the 10 s of the check', () => {
    expect(typeof o.old?.port).toBe('number');
    expect(o.old?.exitDuringWindow).toBeNull();
    expect(o.old?.controlValueAtEndOfWindow).toEqual(VALUE_CONTROL);
  });

  test.each([
    [DASH_MAP, VALUE_DASH],
    [SLASH_MAP, VALUE_SLASH],
  ])('R8-5: back on the newer binary, a query for %p returns the value', (mapName, value) => {
    expect(typeof o.newAgainPort).toBe('number');
    expect(o.query[mapName]).toEqual(value);
  });

  test.each([
    [DASH_MAP, VALUE_DASH],
    [SLASH_MAP, VALUE_SLASH],
  ])('R8-6: back on the newer binary, a Merkle sync for %p returns the value', (mapName, value) => {
    expect(o.merkle[mapName]).toEqual(value);
  });
});

describe('Integration: rollback to the older server binary after kill -9', () => {
  const o: {
    writes: { control?: string[] | string; lww?: string[] | string; orAdd?: string[] | string };
    /** Partitions whose WAL segments grew between the newer binary's PORT= line and its kill. */
    grown: number[];
    grownWithSidecar: number[];
    grownEmpty: number[];
    kill?: BinaryExit | null;
    old?: OldRun;
    newAgainPort?: number | null;
    lwwQuery?: unknown;
    lwwMerkle?: unknown;
    orValues?: unknown;
    newAgainUnreplayed?: string[];
  } = { writes: {}, grown: [], grownWithSidecar: [], grownEmpty: [] };
  let dataDir: string | null = null;
  const running: SpawnedBinary[] = [];

  beforeAll(async () => {
    const env = requireTwoBinaryEnv();
    dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'topgun-map-name-rollback-kill-'));
    const dir = dataDir;
    const walDir = path.join(dir, 'wal');

    // A flush interval far beyond the test's duration: nothing reaches the
    // store, so every acknowledged write exists only as a WAL frame at the kill.
    const first = await spawnBinary({
      binaryPath: env.newBinary,
      commit: env.newCommit,
      label: 'kill: NEW first run',
      env: storeEnv(dir, { TOPGUN_WRITEBEHIND_FLUSH_INTERVAL_MS: '600000' }),
    });
    running.push(first);
    if (first.port === null) throw new Error(`NEW did not print PORT=; stderr:\n${first.stderr()}`);
    const port = first.port;
    const atPort = walPartitions(walDir);

    // Each wait for an acknowledgement is bounded and its result recorded, so
    // an unacknowledged write fails its own clause and the run goes on.
    o.writes.control = await observe(() =>
      writeAndCollect(port, 'kill-writer-0', lwwPut('1', OLD_CLASS_MAP, VALUE_CONTROL)),
    );
    o.writes.lww = await observe(() =>
      writeAndCollect(port, 'kill-writer-1', lwwPut('2', DASH_MAP, VALUE_DASH)),
    );
    o.writes.orAdd = await observe(() =>
      writeAndCollect(port, 'kill-writer-2', {
        id: '3',
        mapName: SLASH_MAP,
        opType: 'OR_ADD',
        key: KEY,
        orRecord: createORRecord(OR_VALUE, 'rollback'),
      }),
    );

    o.kill = await first.stop('SIGKILL', 5_000);
    const atKill = walPartitions(walDir);
    o.grown = grownPartitions(atPort, atKill);
    o.grownWithSidecar = o.grown.filter((id) => atKill.get(id)?.hasSidecar);
    o.grownEmpty = o.grown.filter((id) => (atKill.get(id)?.segmentBytes ?? 0) === 0);

    const [old, oldRun] = await runOld(env, dir, 'kill: OLD');
    running.push(old);
    o.old = oldRun;

    const again = await spawnBinary({
      binaryPath: env.newBinary,
      commit: env.newCommit,
      label: 'kill: NEW again',
      env: storeEnv(dir),
    });
    running.push(again);
    o.newAgainPort = again.port;
    if (again.port !== null) {
      const againPort = again.port;
      o.lwwQuery = await observe(() => queryKey(againPort, DASH_MAP, KEY, 'kill-q'));
      o.lwwMerkle = await observe(() => merkleKey(againPort, DASH_MAP, KEY, 'kill-m'));
      o.orValues = await observe(() => orMapValues(againPort, SLASH_MAP, KEY, 'kill-or'));
    }
    o.newAgainUnreplayed = unreplayedLines(again);
    await again.stop('SIGTERM', STOP_TIMEOUT_MS);

    process.stderr.write(
      `[map-name-rollback] kill -9, observed:\n${JSON.stringify(o, null, 2)}\n${streams(old)}`,
    );
  }, 420_000);

  afterAll(async () => {
    for (const server of running) await server.stop('SIGKILL', 5_000);
    if (dataDir) fs.rmSync(dataDir, { recursive: true, force: true });
  });

  test('R8b-1: both LWW writes were acknowledged before the kill, and no frame written since PORT= was applied', () => {
    expect(o.writes.control).toEqual(['OP_ACK']);
    expect(o.writes.lww).toEqual(['OP_ACK']);
    expect(o.kill).toEqual({ code: null, signal: 'SIGKILL' });
    // No upper bound on the count: besides the three writes, every client that
    // authenticates gets a tombstone-cursor record of its own, which is a WAL
    // frame in whatever partition its id hashes to.
    expect(o.grown.length).toBeGreaterThanOrEqual(1);
    expect(o.grownEmpty).toEqual([]);
    expect(o.grownWithSidecar).toEqual([]);
  });

  test('R8b-2: the older binary prints PORT= and neither of its streams contains FATAL', () => {
    expect(typeof o.old?.port).toBe('number');
    expect(o.old?.fatal).toEqual([]);
  });

  test('R8b-3: the older binary reports the frames it could not replay', () => {
    expect(o.old?.unreplayed.length).toBeGreaterThanOrEqual(1);
  });

  test('R8b-4: the older binary returns the identifier-named map and is still serving at the end of the 10 s of the check', () => {
    expect(o.old?.controlValue).toEqual(VALUE_CONTROL);
    expect(o.old?.exitDuringWindow).toBeNull();
    expect(o.old?.controlValueAtEndOfWindow).toEqual(VALUE_CONTROL);
  });

  test('R8b-5: back on the newer binary, a query and a Merkle sync return the LWW value', () => {
    expect(typeof o.newAgainPort).toBe('number');
    expect(o.lwwQuery).toEqual(VALUE_DASH);
    expect(o.lwwMerkle).toEqual(VALUE_DASH);
  });

  test('R8b-6: the OR_ADD was acknowledged before the kill, and an OR-Map sync returns the entry after the replay', () => {
    expect(o.writes.orAdd).toEqual(['OP_ACK']);
    expect(o.orValues).toEqual([OR_VALUE]);
  });
});
