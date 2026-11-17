/**
 * Integration: rolling a node back across record version rows.
 *
 * - `wrote`. The newer binary stores a record under its WAL and stops with
 *   exit 0 and `drained=true`. The commit that stored the record left the
 *   `max_version` row in the reserved keyspace `0x12`, which is never
 *   reclaimed. Started on that directory, the older binary (built at
 *   67ff9b67, the last binary that keeps no version rows) exits 1 with
 *   `store format: refusing to open: a key in the reserved keyspace 0x12` on
 *   stderr and never prints `PORT=`. It changes no file of the WAL directory
 *   and no file of the data directory outside the engine; the engine's own
 *   files are not compared, because its open writes RocksDB's housekeeping
 *   files before it reads the metadata, so the records are checked through
 *   the newer binary instead, which serves the record afterwards.
 * - `opened`. The newer binary only starts and stops, with no write: the
 *   version rows are lazy, so the directory holds no `0x12` row and the older
 *   binary starts on it, takes a write and serves it.
 *
 * Two binaries are needed, each given by path together with the commit it was
 * built from (VERSION_OLD_SERVER_BINARY / VERSION_OLD_SERVER_COMMIT for the
 * older one, RUST_SERVER_BINARY / NEW_SERVER_COMMIT for the newer). With any of
 * them missing the test fails; it never runs one binary in both roles.
 */

import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

import { CLEAN_EXIT, drainedStop, layout, putLww, storeEnv } from './helpers/engine-migration';
import {
  requireVersionRollbackEnv,
  spawnBinary,
  queryKey,
  observe,
  sleep,
  treeSnapshot,
  SpawnedBinary,
  BinaryExit,
  TreeSnapshot,
  TwoBinaryEnv,
  VERSION_OLD_COMMIT_PREFIX,
} from './helpers/two-binary';

/** The older binary's refusal of a `0x12` key, as `git show 67ff9b67:packages/server-rust/src/storage/datastores/engine_store.rs` words it. */
const REFUSAL_LINE = 'store format: refusing to open: a key in the reserved keyspace 0x12';

const MAP = 'version_rollback_items';
const FLUSH_SETTLE_MS = 3_000;
const REFUSAL_DEADLINE_MS = 30_000;
const CASE_TIMEOUT_MS = 300_000;
const PER_OP = { TOPGUN_WAL_FSYNC_POLICY: 'per_op' };
const ENGINE_PREFIX = `topgun-data${path.sep}engine.rocksdb${path.sep}`;

async function startOrThrow(
  running: SpawnedBinary[],
  binary: string,
  commit: string,
  label: string,
  env: Record<string, string>,
): Promise<SpawnedBinary & { port: number }> {
  const server = await spawnBinary({
    binaryPath: binary,
    commit,
    label,
    env,
    portTimeoutMs: 120_000,
  });
  running.push(server);
  if (server.port === null) {
    throw new Error(`${label} did not start:\n${server.stdout().slice(-4000)}\n${server.stderr()}`);
  }
  return server as SpawnedBinary & { port: number };
}

/**
 * Every path and file hash under `dir` except inside the engine directory,
 * whose files RocksDB's open rewrites; the WAL directory is wholly included.
 */
function outsideEngineSnapshot(dir: string): TreeSnapshot {
  const outside = (rel: string) => !rel.startsWith(ENGINE_PREFIX);
  const snap = treeSnapshot(dir, outside);
  return { paths: snap.paths.filter(outside), sha256: snap.sha256 };
}

interface Refusal {
  exit: BinaryExit | null;
  printedPort: boolean;
  refusalOnStderr: boolean;
}

/** Starts the older binary where it must refuse; kills it if it does not. */
async function refusedStart(
  running: SpawnedBinary[],
  env: TwoBinaryEnv,
  dir: string,
): Promise<Refusal> {
  const server = await spawnBinary({
    binaryPath: env.oldBinary,
    commit: env.oldCommit,
    label: 'OLD on the written directory',
    env: storeEnv(dir, 'rocksdb'),
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
  return { exit, printedPort, refusalOnStderr: server.stderr().includes(REFUSAL_LINE) };
}

test(`the older binary is the build from ${VERSION_OLD_COMMIT_PREFIX}`, () => {
  expect(requireVersionRollbackEnv().oldCommit.startsWith(VERSION_OLD_COMMIT_PREFIX)).toBe(true);
});

describe('Integration: wrote, the older binary refuses a directory the newer one stored a record in', () => {
  const o: Record<string, any> = {};
  let dir: string | null = null;
  const running: SpawnedBinary[] = [];

  beforeAll(async () => {
    const env = requireVersionRollbackEnv();
    dir = fs.mkdtempSync(path.join(os.tmpdir(), 'topgun-version-rollback-wrote-'));

    const fresh = await startOrThrow(
      running,
      env.newBinary,
      env.newCommit,
      'NEW (writes)',
      storeEnv(dir, 'rocksdb', PER_OP),
    );
    o.write = await putLww(fresh.port, MAP, [['k', { key: 'k', run: 'new' }]], 'new');
    await sleep(FLUSH_SETTLE_MS);
    o.newStop = await drainedStop(fresh);

    o.snapshotBefore = outsideEngineSnapshot(dir);
    o.refusal = await refusedStart(running, env, dir);
    o.snapshotAfter = outsideEngineSnapshot(dir);

    const again = await startOrThrow(
      running,
      env.newBinary,
      env.newCommit,
      'NEW after the refusal',
      storeEnv(dir, 'rocksdb'),
    );
    o.served = await observe(() => queryKey(again.port, MAP, 'k', 'again-k'));
    o.againStop = await drainedStop(again);

    process.stderr.write(`[version-rollback] wrote, observed:\n${JSON.stringify(o, null, 2)}\n`);
  }, CASE_TIMEOUT_MS);

  afterAll(async () => {
    for (const server of running) await server.stop('SIGKILL', 5_000);
    if (dir) fs.rmSync(dir, { recursive: true, force: true });
  });

  test('setup: the newer binary acked the write and stopped drained', () => {
    expect(o.write).toEqual({ k: 'OP_ACK' });
    expect(o.newStop).toEqual({ exit: CLEAN_EXIT, drained: true });
  });

  test('the older binary exits 1 with the 0x12 refusal on stderr and never prints PORT=', () => {
    expect(o.refusal).toEqual({
      exit: { code: 1, signal: null },
      printedPort: false,
      refusalOnStderr: true,
    });
  });

  test('the refused start changed no file of the WAL directory or outside the engine', () => {
    const snap = o.snapshotBefore as TreeSnapshot;
    expect(snap.paths).toEqual(expect.arrayContaining(['topgun-data/engine.rocksdb/', 'wal/']));
    expect(Object.keys(snap.sha256).some((rel) => rel.startsWith(`wal${path.sep}`))).toBe(true);
    expect(o.snapshotAfter).toEqual(o.snapshotBefore);
  });

  test('the newer binary still serves the record after the refused start', () => {
    expect(o.served).toEqual({ key: 'k', run: 'new' });
    expect(o.againStop).toEqual({ exit: CLEAN_EXIT, drained: true });
  });
});

describe('Integration: opened, the older binary serves a directory the newer one only opened', () => {
  const o: Record<string, any> = {};
  let dir: string | null = null;
  const running: SpawnedBinary[] = [];

  beforeAll(async () => {
    const env = requireVersionRollbackEnv();
    dir = fs.mkdtempSync(path.join(os.tmpdir(), 'topgun-version-rollback-opened-'));

    const fresh = await startOrThrow(
      running,
      env.newBinary,
      env.newCommit,
      'NEW (opens only)',
      storeEnv(dir, 'rocksdb', PER_OP),
    );
    o.newStop = await drainedStop(fresh);
    o.engineCreated = fs.existsSync(layout(dir).rocks);

    const old = await startOrThrow(
      running,
      env.oldBinary,
      env.oldCommit,
      'OLD on the opened directory',
      storeEnv(dir, 'rocksdb', PER_OP),
    );
    o.oldWrite = await putLww(old.port, MAP, [['k', { key: 'k', run: 'old' }]], 'old');
    o.oldServes = await observe(() => queryKey(old.port, MAP, 'k', 'old-k'));
    await sleep(FLUSH_SETTLE_MS);
    o.oldStop = await drainedStop(old);

    process.stderr.write(`[version-rollback] opened, observed:\n${JSON.stringify(o, null, 2)}\n`);
  }, CASE_TIMEOUT_MS);

  afterAll(async () => {
    for (const server of running) await server.stop('SIGKILL', 5_000);
    if (dir) fs.rmSync(dir, { recursive: true, force: true });
  });

  test('setup: the newer binary created the store and stopped drained without a write', () => {
    expect(o.newStop).toEqual({ exit: CLEAN_EXIT, drained: true });
    expect(o.engineCreated).toBe(true);
  });

  test('the older binary starts on it, takes a write, serves it and stops drained', () => {
    expect(o.oldWrite).toEqual({ k: 'OP_ACK' });
    expect(o.oldServes).toEqual({ key: 'k', run: 'old' });
    expect(o.oldStop).toEqual({ exit: CLEAN_EXIT, drained: true });
  });
});
