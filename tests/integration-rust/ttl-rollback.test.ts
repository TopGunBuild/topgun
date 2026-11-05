/**
 * Integration: an older binary reads a store holding records with a TTL, and
 * the newer binary still expires them afterwards (TG-TTL-001).
 *
 * The newer binary writes an LWW record and an OR-Map record, each with a TTL,
 * and stops cleanly (exit 0, `drained=true`). The older binary (built at
 * 241e65bb, the last binary before records carried a TTL) then starts on the
 * same data and WAL directories, serves both records, and stops with exit 0.
 * The TTL is a field of the stored value that the older binary's decoder skips,
 * and no metadata name, key space or WAL frame kind was added, so the older
 * binary has nothing to refuse; it only reads here, so it rewrites nothing.
 * Started again, the newer binary still holds both records with the stamp and
 * TTL they were first stored with, and deletes them once that TTL has run out.
 *
 * Two binaries are needed, each given by path together with the commit it was
 * built from (TTL_OLD_SERVER_BINARY / TTL_OLD_SERVER_COMMIT for the older one,
 * RUST_SERVER_BINARY / NEW_SERVER_COMMIT for the newer). With any of them
 * missing the test fails; it never runs one binary in both roles.
 */

import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

import {
  spawnBinary,
  connectWithin,
  firstFrame,
  queryKey,
  orMapValues,
  merkleLeaves,
  observe,
  plainLines,
  sleep,
  SpawnedBinary,
  BinaryExit,
} from './helpers/two-binary';

const OLD_COMMIT_PREFIX = '241e65bb';

const BUILD_HINT =
  `TTL rollback: build the server at ${OLD_COMMIT_PREFIX} (the last binary before records carried ` +
  'a TTL) in its own worktree and target directory (`git worktree add <scratch>/topgun-ttl-old ' +
  `${OLD_COMMIT_PREFIX}\`, then \`cargo build --release -p topgun-server --manifest-path ` +
  '<scratch>/topgun-ttl-old/Cargo.toml --target-dir <scratch>/topgun-ttl-old-target`), build the ' +
  'newer one in this checkout, and export TTL_OLD_SERVER_BINARY, TTL_OLD_SERVER_COMMIT (the full ' +
  'commit the older binary was built from), RUST_SERVER_BINARY and NEW_SERVER_COMMIT.';

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
    'TTL_OLD_SERVER_BINARY',
    'TTL_OLD_SERVER_COMMIT',
    'RUST_SERVER_BINARY',
    'NEW_SERVER_COMMIT',
  ];
  const missing = names.filter((name) => !process.env[name]);
  if (missing.length > 0) {
    throw new Error(`${missing.join(', ')} not set. ${BUILD_HINT}`);
  }
  const oldCommit = process.env.TTL_OLD_SERVER_COMMIT as string;
  if (!oldCommit.startsWith(OLD_COMMIT_PREFIX)) {
    throw new Error(
      `TTL_OLD_SERVER_COMMIT is ${oldCommit}, not a commit starting ${OLD_COMMIT_PREFIX}. ${BUILD_HINT}`,
    );
  }
  return {
    oldBinary: executable('TTL_OLD_SERVER_BINARY'),
    oldCommit,
    newBinary: executable('RUST_SERVER_BINARY'),
    newCommit: process.env.NEW_SERVER_COMMIT as string,
  };
}

const LWW = 'ttl_rollback_lww';
const OR = 'ttl_rollback_or';
/** Outlives the three runs before the last one starts, and expires in it. */
const TTL_MS = 20_000;
const SWEEP_INTERVAL_MS = 1_000;
/** One sweep interval plus one cycle past expiry, doubled. */
const SWEEP_CEILING_MS = 2 * (SWEEP_INTERVAL_MS + SWEEP_INTERVAL_MS);
const ACK_WAIT_MS = 5_000;
const STOP_TIMEOUT_MS = 40_000;
const CASE_TIMEOUT_MS = 180_000;
const CLEAN_EXIT: BinaryExit = { code: 0, signal: null };

function storeEnv(dir: string): Record<string, string> {
  return {
    STORAGE_BACKEND: 'redb',
    TOPGUN_DATA_DIR: path.join(dir, 'topgun-data'),
    TOPGUN_REDB_PATH: path.join(dir, 'topgun.redb'),
    TOPGUN_WAL_DIR: path.join(dir, 'wal'),
    TOPGUN_WAL_FSYNC_POLICY: 'per_op',
    TOPGUN_INDEX_PATH: path.join(dir, 'scalar_indexes.json'),
    TOPGUN_VECTOR_INDEX_PATH: path.join(dir, 'vector_indexes.json'),
    TOPGUN_EXPIRY_SWEEP_INTERVAL_MS: String(SWEEP_INTERVAL_MS),
  };
}

/** The stamp and TTL the newer binary holds for each record, read from its Merkle leaves. */
interface Held {
  lww: { value: unknown; timestamp: unknown; ttlMs: unknown } | null;
  or: Array<{ value: unknown; tag: unknown; timestamp: unknown; ttlMs: unknown }>;
}

async function held(port: number): Promise<Held> {
  const lww = (await merkleLeaves(port, LWW, false)).get('k');
  const or = (await merkleLeaves(port, OR, true)).get('k');
  return {
    lww: lww ? { value: lww.value, timestamp: lww.timestamp, ttlMs: lww.ttlMs } : null,
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- leaf records are untyped msgpack
    or: (or?.records ?? []).map((r: any) => ({
      value: r.value,
      tag: r.tag,
      timestamp: r.timestamp,
      ttlMs: r.ttlMs,
    })),
  };
}

async function metric(port: number, series: string): Promise<number> {
  const text = await (await fetch(`http://127.0.0.1:${port}/metrics`)).text();
  const line = text.split('\n').find((l) => l.startsWith(`${series} `));
  return line ? Number(line.slice(series.length + 1)) : 0;
}

async function write(port: number): Promise<string[]> {
  const client = await connectWithin(port, 'ttl-rollback-writer');
  const answers: string[] = [];
  try {
    const now = Date.now();
    for (const payload of [
      {
        id: 'ttl-rollback-lww',
        mapName: LWW,
        opType: 'PUT',
        key: 'k',
        record: {
          value: 'lww-value',
          timestamp: { millis: now, counter: 0, nodeId: 'ttl-rollback' },
          ttlMs: TTL_MS,
        },
      },
      {
        id: 'ttl-rollback-or',
        mapName: OR,
        opType: 'OR_ADD',
        key: 'k',
        orRecord: {
          value: 'or-value',
          timestamp: { millis: now, counter: 0, nodeId: 'ttl-rollback' },
          tag: 'ttl-rollback-or',
          ttlMs: TTL_MS,
        },
      },
    ]) {
      client.messages.length = 0;
      client.send({ type: 'CLIENT_OP', payload });
      const frame = await firstFrame(client, ['OP_ACK', 'OP_REJECTED', 'ERROR'], ACK_WAIT_MS);
      answers.push(frame?.type ?? 'none');
    }
  } finally {
    client.close();
  }
  return answers;
}

interface Observed {
  writes: string[];
  writtenAt: number;
  first?: Held;
  newStop?: BinaryExit | null;
  drained?: boolean;
  oldPort?: number | null;
  oldRefusal?: string[];
  oldValues?: { lww: unknown; or: unknown };
  oldStop?: BinaryExit | null;
  again?: Held;
  againAt?: number;
  afterExpiry?: Held;
  removed?: { lww: number; or: number };
}

describe('Integration: an older binary serves records with a TTL, the newer one still expires them', () => {
  const o: Observed = { writes: [], writtenAt: 0 };
  let dir: string | null = null;
  const running: SpawnedBinary[] = [];

  beforeAll(async () => {
    const binaries = requireBinaries();
    dir = fs.mkdtempSync(path.join(os.tmpdir(), 'topgun-ttl-rollback-'));

    const fresh = await spawnBinary({
      binaryPath: binaries.newBinary,
      commit: binaries.newCommit,
      label: 'NEW first run',
      env: storeEnv(dir),
    });
    running.push(fresh);
    if (fresh.port === null) throw new Error(`NEW did not start:\n${fresh.stderr()}`);
    o.writtenAt = Date.now();
    o.writes = await write(fresh.port);
    o.first = await held(fresh.port);
    o.newStop = await fresh.stop('SIGTERM', STOP_TIMEOUT_MS);
    o.drained = plainLines(fresh.stdout()).some(
      (l) => l.includes('rollback hints written') && l.includes('drained=true'),
    );

    const old = await spawnBinary({
      binaryPath: binaries.oldBinary,
      commit: binaries.oldCommit,
      label: 'OLD',
      env: storeEnv(dir),
    });
    running.push(old);
    o.oldPort = old.port;
    if (old.port !== null) {
      o.oldValues = {
        lww: await observe(() => queryKey(old.port as number, LWW, 'k', 'ttl-rollback-old-q')),
        or: await observe(() => orMapValues(old.port as number, OR, 'k', 'ttl-rollback-old-or')),
      };
      o.oldStop = await old.stop('SIGTERM', STOP_TIMEOUT_MS);
    } else {
      o.oldRefusal = plainLines(`${old.stdout()}\n${old.stderr()}`)
        .filter((l) => /ERROR|refus|Error/.test(l))
        .map((l) => l.slice(0, 400));
    }

    const again = await spawnBinary({
      binaryPath: binaries.newBinary,
      commit: binaries.newCommit,
      label: 'NEW second run',
      env: storeEnv(dir),
    });
    running.push(again);
    if (again.port === null) throw new Error(`NEW did not start again:\n${again.stderr()}`);
    o.again = await held(again.port);
    o.againAt = Date.now();

    // Polled, never slept through: the newer binary deletes both records once
    // the TTL counted from the first write has run out.
    const deadline = o.writtenAt + TTL_MS + SWEEP_CEILING_MS;
    for (;;) {
      o.removed = {
        lww: await metric(again.port, 'topgun_expiry_removed_total{kind="lww_record"}'),
        or: await metric(again.port, 'topgun_expiry_removed_total{kind="or_tag"}'),
      };
      if ((o.removed.lww >= 1 && o.removed.or >= 1) || Date.now() >= deadline) break;
      await sleep(250);
    }
    o.afterExpiry = await held(again.port);
    o.oldValues ??= { lww: undefined, or: undefined };

    process.stderr.write(`[ttl-rollback] observed:\n${JSON.stringify(o, null, 2)}\n`);
  }, CASE_TIMEOUT_MS);

  afterAll(async () => {
    for (const server of running) await server.stop('SIGKILL', 5_000);
    if (dir) fs.rmSync(dir, { recursive: true, force: true });
  });

  test(`the older binary is the build from ${OLD_COMMIT_PREFIX}`, () => {
    expect(requireBinaries().oldCommit.startsWith(OLD_COMMIT_PREFIX)).toBe(true);
  });

  test('setup: both writes acknowledged and stored with the TTL, then a drained clean stop', () => {
    expect(o.writes).toEqual(['OP_ACK', 'OP_ACK']);
    expect(o.first?.lww).toMatchObject({ value: 'lww-value', ttlMs: TTL_MS });
    expect(o.first?.or).toEqual([expect.objectContaining({ value: 'or-value', ttlMs: TTL_MS })]);
    expect(o.newStop).toEqual(CLEAN_EXIT);
    expect(o.drained).toBe(true);
  });

  test('the older binary starts on the same directories, serves both records and stops cleanly', () => {
    expect(o.oldRefusal).toBeUndefined();
    expect(typeof o.oldPort).toBe('number');
    expect(o.oldValues).toEqual({ lww: 'lww-value', or: ['or-value'] });
    expect(o.oldStop).toEqual(CLEAN_EXIT);
  });

  test('started again, the newer binary holds both records with their first stamp and TTL', () => {
    // Read before expiry, or the comparison would be against what remains after it.
    expect(o.againAt).toBeLessThan(o.writtenAt + TTL_MS);
    expect(o.again).toEqual(o.first);
  });

  test('the newer binary deletes both records once their TTL has run out', () => {
    expect(o.removed).toEqual({ lww: 1, or: 1 });
    expect(o.afterExpiry).toEqual({ lww: null, or: [] });
  });
});
