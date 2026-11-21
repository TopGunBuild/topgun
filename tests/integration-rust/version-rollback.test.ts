/**
 * Integration: rolling a node back across record version rows.
 *
 * - `wrote`. The newer binary stores records in two maps under its WAL and
 *   stops with exit 0 and `drained=true`. The commit that stored them left the
 *   `max_version` row in the reserved keyspace `0x12`, which is never
 *   reclaimed. Started on that directory, the older binary (built at
 *   67ff9b67, the last binary that keeps no version rows) exits 1 with
 *   `store format: refusing to open: a key in the reserved keyspace 0x12` on
 *   stderr and never prints `PORT=`. It changes no record or metadata row and
 *   no WAL file. The engine's files are not compared, because its open
 *   rewrites RocksDB's housekeeping files (LOG, OPTIONS, MANIFEST) before it
 *   reads the metadata, and no tool here reads the engine's rows without the
 *   server. So the rows are compared through the newer binary, started on the
 *   directory before and after the refused start: it serves every record and
 *   lists every map identically both times, and it starts at all, which it
 *   would not on a regressed watermark or version row. The WAL directory is
 *   compared byte for byte.
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
import * as jwt from 'jsonwebtoken';
import * as os from 'os';
import * as path from 'path';

import { CLEAN_EXIT, drainedStop, layout, putLww, storeEnv } from './helpers/engine-migration';
import {
  requireVersionRollbackEnv,
  spawnBinary,
  queryKey,
  connectWithin,
  firstFrame,
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
const MAP2 = 'version_rollback_more';
const FLUSH_SETTLE_MS = 3_000;
const REFUSAL_DEADLINE_MS = 30_000;
const CASE_TIMEOUT_MS = 300_000;
const PER_OP = { TOPGUN_WAL_FSYNC_POLICY: 'per_op' };
const ADMIN_TOKEN = jwt.sign({ sub: 'operator', roles: ['admin'] }, 'test-e2e-secret', {
  expiresIn: '1h',
});
const ADMIN_ENV = { TOPGUN_ADMIN_SUBJECTS: 'operator' };

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

/** Every row of `mapName` the server serves, by key. */
async function mapRows(
  port: number,
  mapName: string,
  tag: string,
): Promise<Record<string, unknown>> {
  const client = await connectWithin(port, tag);
  try {
    client.messages.length = 0;
    client.send({ type: 'QUERY_SUB', payload: { queryId: `q-${tag}`, mapName, query: {} } });
    const response = await firstFrame(client, ['QUERY_RESP'], 10_000);
    if (!response) throw new Error(`no QUERY_RESP for ${mapName}`);
    const rows: Record<string, unknown> = {};
    for (const r of response.payload.results as any[]) rows[r.key] = r.record?.value ?? r.value;
    return rows;
  } finally {
    client.close();
  }
}

/** What the newer binary serves of the directory: the map list and every row of both maps. */
async function served(port: number, tag: string): Promise<unknown> {
  const resp = await fetch(`http://127.0.0.1:${port}/api/admin/maps`, {
    headers: { Authorization: `Bearer ${ADMIN_TOKEN}` },
  });
  const maps = ((await resp.json()).maps ?? []).map((m: any) => m.name).sort();
  return {
    maps,
    [MAP]: await mapRows(port, MAP, `${tag}-1`),
    [MAP2]: await mapRows(port, MAP2, `${tag}-2`),
  };
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
    o.write = {
      ...(await putLww(
        fresh.port,
        MAP,
        [
          ['k1', { key: 'k1', run: 'new' }],
          ['k2', { key: 'k2', run: 'new' }],
        ],
        'new',
      )),
      ...(await putLww(fresh.port, MAP2, [['k3', { key: 'k3', run: 'new' }]], 'new2')),
    };
    await sleep(FLUSH_SETTLE_MS);
    o.newStop = await drainedStop(fresh);

    const before = await startOrThrow(
      running,
      env.newBinary,
      env.newCommit,
      'NEW before the refusal',
      storeEnv(dir, 'rocksdb', ADMIN_ENV),
    );
    o.servedBefore = await observe(() => served(before.port, 'before'));
    o.beforeStop = await drainedStop(before);

    const walDir = path.join(dir, 'wal');
    o.walBefore = treeSnapshot(walDir);
    o.refusal = await refusedStart(running, env, dir);
    o.walAfter = treeSnapshot(walDir);

    const again = await startOrThrow(
      running,
      env.newBinary,
      env.newCommit,
      'NEW after the refusal',
      storeEnv(dir, 'rocksdb', ADMIN_ENV),
    );
    o.servedAfter = await observe(() => served(again.port, 'after'));
    o.againStop = await drainedStop(again);

    process.stderr.write(`[version-rollback] wrote, observed:\n${JSON.stringify(o, null, 2)}\n`);
  }, CASE_TIMEOUT_MS);

  afterAll(async () => {
    for (const server of running) await server.stop('SIGKILL', 5_000);
    if (dir) fs.rmSync(dir, { recursive: true, force: true });
  });

  test('setup: the newer binary acked the writes and stopped drained', () => {
    expect(o.write).toEqual({ k1: 'OP_ACK', k2: 'OP_ACK', k3: 'OP_ACK' });
    expect(o.newStop).toEqual({ exit: CLEAN_EXIT, drained: true });
    expect(o.beforeStop).toEqual({ exit: CLEAN_EXIT, drained: true });
  });

  test('the older binary exits 1 with the 0x12 refusal on stderr and never prints PORT=', () => {
    expect(o.refusal).toEqual({
      exit: { code: 1, signal: null },
      printedPort: false,
      refusalOnStderr: true,
    });
  });

  test('the refused start changed no WAL file', () => {
    const snap = o.walBefore as TreeSnapshot;
    expect(Object.keys(snap.sha256).length).toBeGreaterThan(0);
    expect(o.walAfter).toEqual(o.walBefore);
  });

  test('the refused start changed no record or metadata row: the newer binary serves the same', () => {
    expect(o.servedBefore).toEqual({
      maps: expect.arrayContaining([MAP, MAP2]),
      [MAP]: { k1: { key: 'k1', run: 'new' }, k2: { key: 'k2', run: 'new' } },
      [MAP2]: { k3: { key: 'k3', run: 'new' } },
    });
    expect(o.servedAfter).toEqual(o.servedBefore);
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
