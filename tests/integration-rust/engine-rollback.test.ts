/**
 * Integration: rolling a node back across the engine change.
 *
 * - Documented rollback. The older binary (built at 7692c380, the last binary
 *   that stores in `engine.redb`) writes and stops drained; the newer binary
 *   migrates the store to `engine.rocksdb/`, takes more writes and stops with
 *   exit 0 and `drained=true`. Started on that directory as it is, the older
 *   binary exits 1 with its row L3 refusal and changes no file of the data
 *   directory or the WAL directory. After the documented steps — move
 *   `engine.rocksdb/` away, rename `engine.redb.pre-rocksdb` to `engine.redb`,
 *   move the WAL directory away and create an empty one — the older binary
 *   starts and serves the kept store: the writes from before the migration,
 *   none from after it.
 * - `undrained-old-stop`. The older binary acks a `per_op` write `w` with no
 *   flush pass due and is killed with `kill -9`, so `w` is a WAL frame above
 *   its redb watermark. Before the newer binary starts, the test checks that
 *   this is so (else it throws `ScenarioPreconditionError` naming the failed
 *   check): a copy of the data directory started by the older binary with an
 *   empty WAL directory does not serve `w`, and a WAL segment grew with `w`.
 *   The newer binary migrates, recovers the frame and serves `w`; after the
 *   documented rollback the older binary does not serve `w`. A write the older
 *   binary acked above its watermark before the upgrade is lost by the
 *   rollback, because the step that discards the WAL directory discards it.
 *
 * Two binaries are needed, each given by path together with the commit it was
 * built from (ENGINE_OLD_SERVER_BINARY / ENGINE_OLD_SERVER_COMMIT for the older
 * one, RUST_SERVER_BINARY / NEW_SERVER_COMMIT for the newer). With any of them
 * missing the test fails; it never runs one binary in both roles.
 */

import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

import {
  CLEAN_EXIT,
  COMPLETE_LINE,
  drainedStop,
  kindOf,
  layout,
  linesWith,
  putLww,
  storeEnv,
} from './helpers/engine-migration';
import {
  requireEngineEnv,
  spawnBinary,
  queryKey,
  observe,
  sleep,
  treeSnapshot,
  walPartitions,
  grownPartitions,
  SpawnedBinary,
  BinaryExit,
  TreeSnapshot,
  TwoBinaryEnv,
  ENGINE_OLD_COMMIT_PREFIX,
} from './helpers/two-binary';

/**
 * The older binary's row L3 refusal, as `git show 7692c380:packages/server-rust/src/storage/datastores/redb.rs`
 * words it: `"store location: {} is not a store directory, an empty directory or absent"`, the
 * placeholder being the data directory.
 */
function l3Line(dataDir: string): string {
  return `store location: ${dataDir} is not a store directory, an empty directory or absent`;
}

/** A scenario that did not set up what it claims to test; never a verdict on the binaries. */
class ScenarioPreconditionError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'ScenarioPreconditionError';
  }
}

const MAP = 'engine_rollback_items';
const FLUSH_SETTLE_MS = 3_000;
const REFUSAL_DEADLINE_MS = 30_000;
const CASE_TIMEOUT_MS = 300_000;
const PER_OP = { TOPGUN_WAL_FSYNC_POLICY: 'per_op' };

function entries(keys: string[], run: string): [string, unknown][] {
  return keys.map((key) => [key, { key, run }]);
}

async function readKeys(
  port: number,
  keys: string[],
  tag: string,
): Promise<Record<string, unknown>> {
  const values: Record<string, unknown> = {};
  for (const key of keys)
    values[key] = await observe(() => queryKey(port, MAP, key, `${tag}-${key}`));
  return values;
}

/**
 * The documented rollback steps after the newer binary's drained stop: move
 * `engine.rocksdb/` away, rename the kept file to `engine.redb`, move the WAL
 * directory away and create an empty one.
 */
function rollBack(dir: string): void {
  const l = layout(dir);
  fs.renameSync(l.rocks, path.join(dir, 'engine.rocksdb.moved-away'));
  fs.renameSync(l.kept, l.redb);
  fs.renameSync(l.walDir, path.join(dir, 'wal.moved-away'));
  fs.mkdirSync(l.walDir);
}

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

interface Refusal {
  exit: BinaryExit | null;
  printedPort: boolean;
  l3: boolean;
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
    label: 'OLD on the migrated directory',
    env: storeEnv(dir, 'redb'),
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
  return { exit, printedPort, l3: linesWith(server, l3Line(layout(dir).dataDir)).length > 0 };
}

test(`the older binary is the build from ${ENGINE_OLD_COMMIT_PREFIX}`, () => {
  expect(requireEngineEnv().oldCommit.startsWith(ENGINE_OLD_COMMIT_PREFIX)).toBe(true);
});

describe('Integration: the documented rollback brings the older binary up on the kept store', () => {
  const BEFORE = ['b0', 'b1', 'b2'];
  const AFTER = ['a0', 'a1'];
  const o: Record<string, any> = {};
  let dir: string | null = null;
  const running: SpawnedBinary[] = [];

  beforeAll(async () => {
    const env = requireEngineEnv();
    dir = fs.mkdtempSync(path.join(os.tmpdir(), 'topgun-engine-rollback-'));

    const old = await startOrThrow(
      running,
      env.oldBinary,
      env.oldCommit,
      'OLD',
      storeEnv(dir, 'redb', PER_OP),
    );
    o.oldWrites = await putLww(old.port, MAP, entries(BEFORE, 'old'), 'old');
    await sleep(FLUSH_SETTLE_MS);
    o.oldStop = await drainedStop(old);

    const fresh = await startOrThrow(
      running,
      env.newBinary,
      env.newCommit,
      'NEW (migrates)',
      storeEnv(dir, 'rocksdb', PER_OP),
    );
    o.complete = linesWith(fresh, COMPLETE_LINE).length;
    o.newBefore = await readKeys(fresh.port, BEFORE, 'new-before');
    o.newWrites = await putLww(fresh.port, MAP, entries(AFTER, 'new'), 'new');
    await sleep(FLUSH_SETTLE_MS);
    o.newStop = await drainedStop(fresh);

    // Every file is hashed: the older binary must change none of them.
    o.snapshotBefore = treeSnapshot(dir);
    o.refusal = await refusedStart(running, env, dir);
    o.snapshotAfter = treeSnapshot(dir);

    rollBack(dir);
    const back = await startOrThrow(
      running,
      env.oldBinary,
      env.oldCommit,
      'OLD after rollback',
      storeEnv(dir, 'redb'),
    );
    o.rolledBack = await readKeys(back.port, [...BEFORE, ...AFTER], 'rolled-back');
    o.backStop = await drainedStop(back);

    process.stderr.write(
      `[engine-rollback] documented, observed:\n${JSON.stringify(o, null, 2)}\n`,
    );
  }, CASE_TIMEOUT_MS);

  afterAll(async () => {
    for (const server of running) await server.stop('SIGKILL', 5_000);
    if (dir) fs.rmSync(dir, { recursive: true, force: true });
  });

  test('setup: the older binary stopped drained; the newer one migrated, took writes and stopped drained', () => {
    expect(o.oldWrites).toEqual({ b0: 'OP_ACK', b1: 'OP_ACK', b2: 'OP_ACK' });
    expect(o.oldStop).toEqual({ exit: CLEAN_EXIT, drained: true });
    expect(o.complete).toBe(1);
    expect(o.newBefore).toEqual(Object.fromEntries(entries(BEFORE, 'old')));
    expect(o.newWrites).toEqual({ a0: 'OP_ACK', a1: 'OP_ACK' });
    expect(o.newStop).toEqual({ exit: CLEAN_EXIT, drained: true });
  });

  test('without the rollback the older binary exits 1 with its L3 line and never prints PORT=', () => {
    expect(o.refusal).toEqual({ exit: { code: 1, signal: null }, printedPort: false, l3: true });
  });

  test('the refused start changed no file of the data directory or the WAL directory', () => {
    const snap = o.snapshotBefore as TreeSnapshot;
    expect(snap.paths).toEqual(expect.arrayContaining(['topgun-data/engine.rocksdb/', 'wal/']));
    expect(o.snapshotAfter).toEqual(o.snapshotBefore);
  });

  test('after the documented rollback the older binary serves the kept store, without the later writes', () => {
    expect(o.rolledBack).toEqual({
      ...Object.fromEntries(entries(BEFORE, 'old')),
      a0: undefined,
      a1: undefined,
    });
    expect(o.backStop).toEqual({ exit: CLEAN_EXIT, drained: true });
  });
});

describe('Integration: undrained-old-stop, a write the older binary left above its watermark is lost by the rollback', () => {
  const o: Record<string, any> = {};
  let dir: string | null = null;
  const running: SpawnedBinary[] = [];

  /**
   * Both checks that `w` is a WAL frame above the redb watermark when the
   * newer binary starts; throws naming the one that fails.
   */
  async function checkPrecondition(env: TwoBinaryEnv, root: string): Promise<void> {
    if (o.grown.length === 0 || o.grown.some((p: number) => !(o.walAfter[p] > 0))) {
      throw new ScenarioPreconditionError(
        `(ii) no WAL segment grew with w: partition bytes before ${JSON.stringify(o.walBefore)}, after ${JSON.stringify(o.walAfter)}`,
      );
    }
    // A copy, so nothing the older binary does here reaches the directory the
    // newer binary migrates; an empty WAL directory, so nothing is replayed.
    const copy = path.join(root, 'precondition');
    fs.mkdirSync(copy);
    fs.cpSync(layout(root).dataDir, layout(copy).dataDir, { recursive: true });
    fs.mkdirSync(layout(copy).walDir);
    const probe = await startOrThrow(
      running,
      env.oldBinary,
      env.oldCommit,
      'OLD on a copy, empty WAL',
      storeEnv(copy, 'redb'),
    );
    o.copyServes = await readKeys(probe.port, ['base', 'w'], 'copy');
    await probe.stop('SIGTERM');
    fs.rmSync(copy, { recursive: true, force: true });
    // A copy that serves nothing (or fails the read) would also serve no w, so
    // (i) would pass on a broken probe; base must be read back first.
    if (o.copyServes.base?.key !== 'base') {
      throw new ScenarioPreconditionError(
        `(i) the copy serves no base, so it cannot tell whether w is in engine.redb: ${JSON.stringify(o.copyServes)}`,
      );
    }
    if (o.copyServes.w !== undefined) {
      throw new ScenarioPreconditionError(
        `(i) w is in engine.redb before the upgrade: a copy started with an empty WAL directory serves ${JSON.stringify(o.copyServes.w)}`,
      );
    }
  }

  beforeAll(async () => {
    const env = requireEngineEnv();
    dir = fs.mkdtempSync(path.join(os.tmpdir(), 'topgun-engine-rollback-undrained-'));
    const l = layout(dir);

    const first = await startOrThrow(
      running,
      env.oldBinary,
      env.oldCommit,
      'OLD',
      storeEnv(dir, 'redb', PER_OP),
    );
    o.baseWrite = await putLww(first.port, MAP, [['base', { key: 'base', run: 'old' }]], 'base');
    await sleep(FLUSH_SETTLE_MS);
    o.firstStop = await drainedStop(first);

    // No flush pass is due before the kill, so w stays above the watermark.
    const undrained = await startOrThrow(
      running,
      env.oldBinary,
      env.oldCommit,
      'OLD undrained',
      storeEnv(dir, 'redb', { ...PER_OP, TOPGUN_WRITEBEHIND_FLUSH_INTERVAL_MS: '3600000' }),
    );
    const before = walPartitions(l.walDir);
    o.wWrite = await putLww(undrained.port, MAP, [['w', { key: 'w', run: 'old-undrained' }]], 'w');
    const after = walPartitions(l.walDir);
    o.walBefore = Object.fromEntries([...before].map(([p, s]) => [p, s.segmentBytes]));
    o.walAfter = Object.fromEntries([...after].map(([p, s]) => [p, s.segmentBytes]));
    o.grown = grownPartitions(before, after);
    o.killed = await undrained.stop('SIGKILL', 5_000);
    await checkPrecondition(env, dir);

    const fresh = await startOrThrow(
      running,
      env.newBinary,
      env.newCommit,
      'NEW (migrates, recovers w)',
      storeEnv(dir, 'rocksdb', PER_OP),
    );
    o.complete = linesWith(fresh, COMPLETE_LINE).length;
    o.newServes = await readKeys(fresh.port, ['base', 'w'], 'new');
    o.newStop = await drainedStop(fresh);
    o.kept = kindOf(l.kept);

    rollBack(dir);
    const back = await startOrThrow(
      running,
      env.oldBinary,
      env.oldCommit,
      'OLD after rollback',
      storeEnv(dir, 'redb'),
    );
    o.rolledBack = await readKeys(back.port, ['base', 'w'], 'rolled-back');
    o.backStop = await drainedStop(back);

    process.stderr.write(
      `[engine-rollback] undrained-old-stop, observed:\n${JSON.stringify(o, null, 2)}\n`,
    );
  }, CASE_TIMEOUT_MS);

  afterAll(async () => {
    for (const server of running) await server.stop('SIGKILL', 5_000);
    if (dir) fs.rmSync(dir, { recursive: true, force: true });
  });

  test('setup: base stored by a drained stop; w acked by a per_op binary killed with kill -9', () => {
    expect(o.baseWrite).toEqual({ base: 'OP_ACK' });
    expect(o.firstStop).toEqual({ exit: CLEAN_EXIT, drained: true });
    expect(o.wWrite).toEqual({ w: 'OP_ACK' });
    expect(o.killed).toEqual({ code: null, signal: 'SIGKILL' });
    expect(o.copyServes).toEqual({ base: { key: 'base', run: 'old' }, w: undefined });
  });

  test('the newer binary migrates, recovers w from the WAL and serves it', () => {
    expect(o.complete).toBe(1);
    expect(o.newServes).toEqual({
      base: { key: 'base', run: 'old' },
      w: { key: 'w', run: 'old-undrained' },
    });
    expect(o.newStop).toEqual({ exit: CLEAN_EXIT, drained: true });
    expect(o.kept).toBe('file');
  });

  test('after the documented rollback the older binary serves the kept store, which lacks w', () => {
    expect(o.rolledBack).toEqual({ base: { key: 'base', run: 'old' }, w: undefined });
    expect(o.backStop).toEqual({ exit: CLEAN_EXIT, drained: true });
  });
});
