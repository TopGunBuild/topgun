/**
 * Integration: an engine migration killed with `kill -9` completes at the next
 * start, with the same content as one that was never interrupted.
 *
 * The older binary (built at 7692c380, the last binary that stores in
 * `engine.redb`) writes a store large enough that the copy takes several
 * batches, plus a small LWW map and an OR-Map key read back as sentinels, and
 * stops drained. Each case runs on a clone of that store. A control run
 * migrates a clone without interruption; its `store engine: migration
 * complete` line (rows, value bytes) and the sentinels are the reference.
 *
 * Five points:
 *
 * - before the first batch — staged, not aimed: a kill there falls in a
 *   sub-second window inside the create, which nothing outside the process can
 *   hit. The test leaves the state such a kill leaves (an `engine.rocksdb/`
 *   without `CURRENT` beside `engine.redb`) and starts the newer binary, which
 *   must log that it removes the empty directory, not refuse two stores.
 * - inside the first batch, during the copy, after the `verified` mark, after
 *   the rename — aimed from outside: while the newer binary migrates, the test
 *   polls the data directory in a tight synchronous loop and sends SIGKILL the
 *   moment its trigger is seen. Inside the first batch: the engine WAL under
 *   `engine.rocksdb/` (which has `CURRENT`) holds more bytes than a freshly
 *   created engine's WAL, so the first batch's record is being appended; the
 *   test measures that fresh size itself beforehand, by killing starts as soon
 *   as `CURRENT` and a WAL file appear and taking the smallest WAL seen. A kill
 *   that lands while the record is still torn leaves no row and no mark, and
 *   the next start must remove `engine.rocksdb/` (one `WARN`) and migrate anew;
 *   one that lands after the record is whole resumes from its mark and is a
 *   miss. The copy: the engine WAL holds more than one copy batch, so at least
 *   the first batch and its mark are whole. The `verified` mark: the engine WAL grows by less than 1 KiB after at least
 *   50 ms without growth (a copy batch is megabytes; the verification between
 *   the last batch and the mark only reads). The rename:
 *   `engine.redb.pre-rocksdb` exists. Which point a kill hit is then established from what the next
 *   start logs and finds, not from the trigger: `store engine: resuming after`
 *   is the copy; `store engine: resuming at the verification` with
 *   `engine.redb` still in place is the `verified` mark, with
 *   `engine.redb.pre-rocksdb` in its place the rename. A kill that hit another
 *   point is retried on a fresh clone, a bounded number of times; a point
 *   never hit fails the test and lists every attempt — it never passes on a
 *   miss.
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
  MIGRATING_LINE,
  drainedStop,
  kindOf,
  layout,
  linesWith,
  putLww,
  storeEnv,
  Layout,
} from './helpers/engine-migration';
import {
  requireEngineEnv,
  spawnBinary,
  connectWithin,
  firstFrame,
  queryKey,
  orMapValues,
  logField,
  observe,
  SpawnedBinary,
  TwoBinaryEnv,
  ENGINE_OLD_COMMIT_PREFIX,
} from './helpers/two-binary';

const BULK_MAPS = ['engine_resume_bulk_a', 'engine_resume_bulk_b'];
const BULK_ROWS_PER_MAP = 100_000;
const BULK_VALUE_BYTES = 512;
const OPS_PER_BATCH = 500;
const SENTINEL = 'engine_resume_sentinel';
const SENTINEL_OR = 'engine_resume_sentinel_or';
const SENTINEL_KEYS = ['s0', 's1', 's2'];
/** Verification of this fixture takes longer (about 125 ms measured on a Mac). */
const QUIET_MS = 50;
/** The `verified` mark's write is a few hundred bytes at most. */
const MARK_MAX_BYTES = 1024;
/**
 * More than one copy batch (at most 10 000 rows, under 7 MB here) on disk, so
 * the first batch and its mark are written whole when the kill lands.
 */
const COPY_TRIGGER_BYTES = 16 * 1024 * 1024;
const AIM_DEADLINE_MS = 60_000;
const MAX_ATTEMPTS = 6;
/** The first batch's record is appended in a few milliseconds, so this point misses more often. */
const FIRST_BATCH_MAX_ATTEMPTS = 20;
/** Kills of fresh starts that measure a freshly created engine's WAL size. */
const FRESH_LOG_SAMPLES = 3;
const FIXTURE_TIMEOUT_MS = 600_000;
const CASE_TIMEOUT_MS = 600_000;

type Aimed = 'first-batch' | 'copy' | 'verified' | 'renamed';

const filler = 'x'.repeat(BULK_VALUE_BYTES);

/** Writes the bulk maps in OP_BATCHes, each answered by one OP_ACK naming its last op. */
async function writeBulk(port: number): Promise<number> {
  const client = await connectWithin(port, 'engine-resume-bulk', 10_000);
  let acked = 0;
  try {
    for (const map of BULK_MAPS) {
      for (let start = 0; start < BULK_ROWS_PER_MAP; start += OPS_PER_BATCH) {
        const ops = [];
        for (let i = start; i < Math.min(start + OPS_PER_BATCH, BULK_ROWS_PER_MAP); i++) {
          ops.push({
            id: `${map}-${i}`,
            mapName: map,
            opType: 'PUT',
            key: `k${String(i).padStart(7, '0')}`,
            record: {
              value: { i, filler },
              timestamp: { millis: Date.now(), counter: 0, nodeId: 'engine-resume' },
            },
          });
        }
        client.messages.length = 0;
        client.send({ type: 'OP_BATCH', payload: { ops } });
        const frame = await firstFrame(client, ['OP_ACK', 'OP_REJECTED', 'ERROR'], 30_000);
        const lastId = ops[ops.length - 1].id;
        if (frame?.type !== 'OP_ACK' || frame.payload?.lastId !== lastId) {
          throw new Error(`bulk batch ending ${lastId}: ${JSON.stringify(frame)?.slice(0, 400)}`);
        }
        acked += ops.length;
      }
    }
  } finally {
    client.close();
  }
  return acked;
}

async function orAdd(port: number): Promise<string> {
  const client = await connectWithin(port, 'engine-resume-or');
  try {
    client.send({
      type: 'CLIENT_OP',
      payload: {
        id: 'engine-resume-or',
        mapName: SENTINEL_OR,
        opType: 'OR_ADD',
        key: 'k',
        orRecord: {
          value: 'or-sentinel',
          timestamp: { millis: Date.now(), counter: 0, nodeId: 'engine-resume' },
          tag: 'engine-resume-or',
        },
      },
    });
    return (await firstFrame(client, ['OP_ACK', 'OP_REJECTED', 'ERROR'], 10_000))?.type ?? 'none';
  } finally {
    client.close();
  }
}

/** What the store must hold after any completed migration: its own count, and the sentinels as a client reads them. */
interface Content {
  rows?: string;
  bytes?: string;
  sentinels: Record<string, unknown>;
  or: unknown;
}

async function content(server: SpawnedBinary & { port: number }, tag: string): Promise<Content> {
  const complete = linesWith(server, COMPLETE_LINE)[0] ?? '';
  const sentinels: Record<string, unknown> = {};
  for (const key of SENTINEL_KEYS) {
    sentinels[key] = await observe(() => queryKey(server.port, SENTINEL, key, `${tag}-${key}`));
  }
  return {
    rows: logField(complete, 'rows'),
    bytes: logField(complete, 'bytes'),
    sentinels,
    or: await observe(() => orMapValues(server.port, SENTINEL_OR, 'k', `${tag}-or`)),
  };
}

function clone(pristine: string, root: string, name: string): string {
  const dir = path.join(root, name);
  // A copy-on-write clone on APFS, so a fresh store per attempt costs nothing.
  fs.cpSync(pristine, dir, { recursive: true, mode: fs.constants.COPYFILE_FICLONE });
  return dir;
}

async function startNew(
  env: TwoBinaryEnv,
  dir: string,
  label: string,
): Promise<SpawnedBinary & { port: number }> {
  const server = await spawnBinary({
    binaryPath: env.newBinary,
    commit: env.newCommit,
    label,
    env: storeEnv(dir, 'rocksdb'),
    portTimeoutMs: 180_000,
  });
  if (server.port === null) {
    await server.stop('SIGKILL', 5_000);
    throw new Error(`${label} did not start:\n${server.stdout().slice(-4000)}\n${server.stderr()}`);
  }
  return server as SpawnedBinary & { port: number };
}

/** The size of every engine WAL file under `engine.rocksdb/`, by name; empty while it is absent. */
function engineLogSizes(rocks: string): Map<string, number> {
  const sizes = new Map<string, number>();
  let names: string[];
  try {
    names = fs.readdirSync(rocks);
  } catch {
    return sizes;
  }
  for (const name of names) {
    if (!name.endsWith('.log')) continue;
    try {
      sizes.set(name, fs.statSync(path.join(rocks, name)).size);
    } catch {
      // A WAL file retired between the listing and the stat holds nothing the trigger needs.
    }
  }
  return sizes;
}

/** By how many bytes the WAL files in `now` grew since `before` (a new file counts from 0). */
function growth(before: Map<string, number>, now: Map<string, number>): number {
  let bytes = 0;
  for (const [name, size] of now) bytes += Math.max(0, size - (before.get(name) ?? 0));
  return bytes;
}

/**
 * Polls in a synchronous loop — no event-loop turn between seeing the trigger
 * and the SIGKILL — and returns what it saw. The newer binary logs a few lines
 * during this time, far less than a pipe holds, so it never blocks on output.
 */
function aimAndKill(point: Aimed, l: Layout, pid: number, freshLog: number): string {
  const end = Date.now() + AIM_DEADLINE_MS;
  let seen = 'deadline passed';
  if (point === 'first-batch') {
    while (Date.now() < end) {
      const total = [...engineLogSizes(l.rocks).values()].reduce((a, b) => a + b, 0);
      if (total > freshLog && fs.existsSync(path.join(l.rocks, 'CURRENT'))) {
        seen = `engine WAL holds ${total} bytes, more than a fresh engine's ${freshLog}`;
        break;
      }
    }
  } else if (point === 'copy') {
    while (Date.now() < end) {
      const total = [...engineLogSizes(l.rocks).values()].reduce((a, b) => a + b, 0);
      if (total > COPY_TRIGGER_BYTES) {
        seen = `engine WAL holds more than ${COPY_TRIGGER_BYTES} bytes`;
        break;
      }
    }
  } else if (point === 'renamed') {
    while (Date.now() < end) {
      if (fs.existsSync(l.kept)) {
        seen = 'kept file present';
        break;
      }
    }
  } else {
    // The mark is a one-row write, a copy batch megabytes, and the
    // verification between them only reads: the trigger is a growth under
    // MARK_MAX_BYTES after QUIET_MS without growth, once the copy is under
    // way (the pause before the first batch is quiet too). Growth is summed
    // per file: a memtable flush retires a WAL file during the verification,
    // and a total would then hide the small write after it.
    let sizes = engineLogSizes(l.rocks);
    let grewAt = Date.now();
    let copying = false;
    // Reported with a miss, so a verification too short for QUIET_MS shows as such.
    let longestQuiet = 0;
    while (Date.now() < end) {
      const now = engineLogSizes(l.rocks);
      const at = Date.now();
      const bytes = growth(sizes, now);
      if (bytes > 0) {
        if (copying && bytes < MARK_MAX_BYTES && at - grewAt >= QUIET_MS) {
          seen = `engine WAL grew by ${bytes} bytes after ${at - grewAt} ms without growth`;
          break;
        }
        if (copying) longestQuiet = Math.max(longestQuiet, at - grewAt);
        grewAt = at;
      }
      sizes = now;
      copying ||= [...sizes.values()].reduce((a, b) => a + b, 0) > COPY_TRIGGER_BYTES;
      if (fs.existsSync(l.kept)) {
        seen =
          `kept file present before the trigger (copying: ${copying}, longest quiet ` +
          `while copying: ${longestQuiet} ms, quiet before the rename: ${at - grewAt} ms)`;
        break;
      }
    }
  }
  try {
    process.kill(pid, 'SIGKILL');
  } catch (err) {
    // A victim that exited on its own before the trigger is a missed attempt,
    // recorded as such so the attempts dump still reaches the report.
    if ((err as NodeJS.ErrnoException).code !== 'ESRCH') throw err;
    return `${seen}; the process had already exited, no SIGKILL sent`;
  }
  return seen;
}

/** What a killed start left in `engine.rocksdb/`, read before the next start. */
interface Engine {
  current: boolean;
  logBytes: number;
  kinds: string[];
}

function engineState(rocks: string): Engine {
  let names: string[] = [];
  try {
    names = fs.readdirSync(rocks).sort();
  } catch {
    // An absent directory holds nothing.
  }
  return {
    current: names.includes('CURRENT'),
    logBytes: [...engineLogSizes(rocks).values()].reduce((a, b) => a + b, 0),
    kinds: [...new Set(names.map((n) => n.replace(/^[0-9]+/, '')))],
  };
}

interface Attempt {
  trigger: string;
  killed: unknown;
  files: { redb: string; kept: string; rocks: string };
  engine: Engine;
  resumedAfter: boolean;
  resumedAtVerification: boolean;
  removing: number;
  migrating: number;
  twoStores: number;
  hit: string;
}

function classify(a: Omit<Attempt, 'hit' | 'trigger' | 'killed'>, freshLog: number): string {
  const { files, resumedAfter: after, resumedAtVerification: atVerification } = a;
  if (
    a.engine.current &&
    a.engine.logBytes > freshLog &&
    a.removing === 1 &&
    a.migrating === 1 &&
    !after &&
    a.twoStores === 0 &&
    files.redb === 'file' &&
    files.kept === 'absent'
  ) {
    return 'first-batch';
  }
  if (after && files.redb === 'file' && files.kept === 'absent') return 'copy';
  if (atVerification && files.redb === 'file' && files.kept === 'absent') return 'verified';
  if (atVerification && files.redb === 'absent' && files.kept === 'file') return 'renamed';
  return `none of the four (files ${JSON.stringify(files)}, resume after: ${after}, at the verification: ${atVerification})`;
}

/**
 * A freshly created engine's WAL size, measured by this test: starts on fresh
 * clones are killed as soon as `CURRENT` and a WAL file both exist, before
 * the first batch can be appended, and the smallest WAL seen is the fresh
 * size (a kill that came late can only see more). Every sample is reported.
 */
async function measureFreshLog(
  env: TwoBinaryEnv,
  pristine: string,
  root: string,
  running: SpawnedBinary[],
): Promise<{ freshLog: number; samples: number[] }> {
  const samples: number[] = [];
  for (let n = 0; n < FRESH_LOG_SAMPLES; n++) {
    const dir = clone(pristine, root, `fresh-log-${n}`);
    const l = layout(dir);
    const victim = await spawnBinary({
      binaryPath: env.newBinary,
      commit: env.newCommit,
      label: `NEW to be killed (fresh engine, sample ${n + 1})`,
      env: storeEnv(dir, 'rocksdb'),
      portTimeoutMs: 0,
    });
    running.push(victim);
    const end = Date.now() + AIM_DEADLINE_MS;
    let sizes = new Map<string, number>();
    while (Date.now() < end) {
      sizes = engineLogSizes(l.rocks);
      if (sizes.size > 0 && fs.existsSync(path.join(l.rocks, 'CURRENT'))) break;
    }
    try {
      process.kill(victim.process.pid as number, 'SIGKILL');
    } catch (err) {
      if ((err as NodeJS.ErrnoException).code !== 'ESRCH') throw err;
    }
    await victim.waitForExit(10_000);
    if (sizes.size === 0) throw new Error(`fresh engine sample ${n + 1}: no WAL file seen`);
    samples.push([...sizes.values()].reduce((a, b) => a + b, 0));
    fs.rmSync(dir, { recursive: true, force: true });
  }
  return { freshLog: Math.min(...samples), samples };
}

test(`the older binary is the build from ${ENGINE_OLD_COMMIT_PREFIX}`, () => {
  expect(requireEngineEnv().oldCommit.startsWith(ENGINE_OLD_COMMIT_PREFIX)).toBe(true);
});

describe('Integration: a killed engine migration completes at the next start', () => {
  let root: string | null = null;
  let pristine = '';
  let control: Content | null = null;
  let freshLog = 0;
  const fixture: Record<string, unknown> = {};
  const running: SpawnedBinary[] = [];

  beforeAll(async () => {
    const env = requireEngineEnv();
    root = fs.mkdtempSync(path.join(os.tmpdir(), 'topgun-engine-resume-'));
    pristine = path.join(root, 'pristine');
    fs.mkdirSync(pristine);

    const old = await spawnBinary({
      binaryPath: env.oldBinary,
      commit: env.oldCommit,
      label: 'OLD (fixture)',
      env: storeEnv(pristine, 'redb', {
        // Every bulk key stays buffered until stored; the default bound would refuse new keys.
        TOPGUN_WRITEBEHIND_CAPACITY: '0',
        TOPGUN_WRITEBEHIND_SHUTDOWN_TIMEOUT_MS: '300000',
      }),
    });
    running.push(old);
    if (old.port === null) throw new Error(`OLD did not start:\n${old.stderr()}`);
    fixture.bulkAcked = await writeBulk(old.port);
    fixture.sentinels = await putLww(
      old.port,
      SENTINEL,
      SENTINEL_KEYS.map((k) => [k, { k }]),
      'sentinel',
    );
    fixture.or = await orAdd(old.port);
    fixture.stop = await drainedStop(old);
    fixture.redbBytes = fs.statSync(layout(pristine).redb).size;
    if (
      fixture.bulkAcked !== BULK_MAPS.length * BULK_ROWS_PER_MAP ||
      fixture.or !== 'OP_ACK' ||
      JSON.stringify(fixture.stop) !== JSON.stringify({ exit: CLEAN_EXIT, drained: true })
    ) {
      throw new Error(`fixture not written: ${JSON.stringify(fixture)}`);
    }

    const dir = clone(pristine, root, 'control');
    const started = Date.now();
    const server = await startNew(env, dir, 'NEW control');
    running.push(server);
    fixture.controlStartMs = Date.now() - started;
    control = await content(server, 'control');
    await drainedStop(server);
    const measured = await measureFreshLog(env, pristine, root, running);
    freshLog = measured.freshLog;
    fixture.freshLogSamples = measured.samples;
    process.stderr.write(
      `[engine-migration-resume] fixture:\n${JSON.stringify({ fixture, control }, null, 2)}\n`,
    );
  }, FIXTURE_TIMEOUT_MS);

  afterAll(async () => {
    for (const server of running) await server.stop('SIGKILL', 5_000);
    if (root) fs.rmSync(root, { recursive: true, force: true });
  });

  test('control: the uninterrupted migration counts every row and serves the sentinels', () => {
    expect(control?.rows).toBeDefined();
    expect(Number(control?.rows)).toBeGreaterThan(BULK_MAPS.length * BULK_ROWS_PER_MAP);
    expect(control?.sentinels).toEqual(Object.fromEntries(SENTINEL_KEYS.map((k) => [k, { k }])));
    expect(control?.or).toEqual(['or-sentinel']);
  });

  test(
    'before the first batch (staged): an engine.rocksdb/ without CURRENT is removed, not refused, and the migration completes',
    async () => {
      const env = requireEngineEnv();
      const dir = clone(pristine, root as string, 'staged');
      const l = layout(dir);
      // What a kill inside the create leaves: the directory, its lock file, no CURRENT.
      fs.mkdirSync(l.rocks);
      fs.writeFileSync(path.join(l.rocks, 'LOCK'), '');
      const server = await startNew(env, dir, 'NEW on the staged directory');
      running.push(server);
      const observed = {
        removing: linesWith(server, 'store engine: removing').filter((x) => x.includes('WARN'))
          .length,
        twoStores: linesWith(server, 'two stores found').length,
        migrating: linesWith(server, MIGRATING_LINE).length,
        content: await content(server, 'staged'),
        stop: await drainedStop(server),
        files: { redb: kindOf(l.redb), kept: kindOf(l.kept), rocks: kindOf(l.rocks) },
      };
      process.stderr.write(
        `[engine-migration-resume] staged:\n${JSON.stringify(observed, null, 2)}\n`,
      );
      expect(observed).toEqual({
        removing: 1,
        twoStores: 0,
        migrating: 1,
        content: control,
        stop: { exit: CLEAN_EXIT, drained: true },
        files: { redb: 'absent', kept: 'file', rocks: 'directory' },
      });
    },
    CASE_TIMEOUT_MS,
  );

  const titles: Record<Aimed, string> = {
    'first-batch': 'inside the first batch',
    copy: 'during the copy',
    verified: 'after the verified mark',
    renamed: 'after the rename',
  };
  for (const point of ['first-batch', 'copy', 'verified', 'renamed'] as Aimed[]) {
    test(
      `kill -9 ${titles[point]}: the next start completes with the same content`,
      async () => {
        const env = requireEngineEnv();
        const attempts: Attempt[] = [];
        let resumed: Record<string, unknown> | null = null;
        const maxAttempts = point === 'first-batch' ? FIRST_BATCH_MAX_ATTEMPTS : MAX_ATTEMPTS;
        for (let n = 0; n < maxAttempts && !resumed; n++) {
          const dir = clone(pristine, root as string, `${point}-${n}`);
          const l = layout(dir);
          // Not awaited for a port: the kill lands before the server listens.
          const victim = await spawnBinary({
            binaryPath: env.newBinary,
            commit: env.newCommit,
            label: `NEW to be killed (${point}, attempt ${n + 1})`,
            env: storeEnv(dir, 'rocksdb'),
            portTimeoutMs: 0,
          });
          running.push(victim);
          const trigger = aimAndKill(point, l, victim.process.pid as number, freshLog);
          const killed = await victim.waitForExit(10_000);
          const files = { redb: kindOf(l.redb), kept: kindOf(l.kept), rocks: kindOf(l.rocks) };
          const engine = engineState(l.rocks);

          const next = await startNew(env, dir, `NEW after the kill (${point}, attempt ${n + 1})`);
          running.push(next);
          const seen = {
            files,
            engine,
            resumedAfter: linesWith(next, 'store engine: resuming after').length > 0,
            resumedAtVerification:
              linesWith(next, 'store engine: resuming at the verification').length > 0,
            removing: linesWith(next, 'store engine: removing').filter((x) => x.includes('WARN'))
              .length,
            migrating: linesWith(next, MIGRATING_LINE).length,
            twoStores: linesWith(next, 'two stores found').length,
          };
          const hit = classify(seen, freshLog);
          attempts.push({ trigger, killed, ...seen, hit });
          if (hit === point) {
            resumed = {
              killed,
              complete: linesWith(next, COMPLETE_LINE).length,
              content: await content(next, `${point}-resumed`),
              stop: await drainedStop(next),
              files: { redb: kindOf(l.redb), kept: kindOf(l.kept), rocks: kindOf(l.rocks) },
            };
          } else {
            await next.stop('SIGKILL', 5_000);
          }
          fs.rmSync(dir, { recursive: true, force: true });
        }
        process.stderr.write(
          `[engine-migration-resume] ${point}:\n${JSON.stringify({ freshLog, attempts, resumed }, null, 2)}\n`,
        );
        // A point never hit is a failure with every attempt listed, never a pass.
        expect({ hit: resumed !== null, attempts: resumed ? undefined : attempts }).toEqual({
          hit: true,
          attempts: undefined,
        });
        expect(resumed).toEqual({
          killed: { code: null, signal: 'SIGKILL' },
          complete: 1,
          content: control,
          stop: { exit: CLEAN_EXIT, drained: true },
          files: { redb: 'absent', kept: 'file', rocks: 'directory' },
        });
      },
      CASE_TIMEOUT_MS,
    );
  }
});
