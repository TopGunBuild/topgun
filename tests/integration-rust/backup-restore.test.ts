/**
 * Integration: the supported backup — stop the server, copy the data directory
 * (`topgun-data/`, recursively) and the WAL directory, start a server on the
 * copy — brings back every acknowledged write. The file at the old store path
 * is not copied: the server writes it again beside the data directory it
 * opens. The control arm copies the data directory alone after an unclean stop
 * and must lose writes: it is what shows the WAL directory is part of the
 * backup. The control and its twin hold the write-behind flush, so at the kill
 * every acknowledged write is in the WAL directory and none is in the data
 * directory: the control, with the data directory alone, loses every one of
 * them, and the twin, identical but for the copied WAL directory, brings every
 * one back. The unheld SIGKILL arm covers a kill at production defaults, where
 * a pass may have stored some writes and the rest are replayed from the WAL.
 */

import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

import { createLWWRecord, createORRecord } from './helpers';
import {
  connectWithin,
  firstFrame,
  orMapValues,
  queryKey,
  spawnBinary,
  SpawnedBinary,
} from './helpers/two-binary';

const LWW_MAP = 'backup_lww';
const OR_MAP = 'backup_or';
const LWW_COUNT = 50;
const OR_COUNT = 20;
const ACK_WAIT_MS = 10_000;
const STOP_WAIT_MS = 60_000;

// Under kill -9 the flush interval is set far beyond the test's duration, so
// every acknowledged write exists only as a WAL frame at the kill: the flush
// task waits one interval before its first pass and nothing else wakes it, so
// no pass runs while server A lives. The stalled-watermark bound is left at its
// default: server A lives a few seconds, far below it.
const HELD_FLUSH_INTERVAL_MS = '3600000';
// The startup WARN an interval of 500 ms or more logs: it shows the hold was read.
const HELD_FLUSH_WARN = 'the write-behind flush interval is at least 500 ms';

function binary(): string {
  const p = process.env.RUST_SERVER_BINARY;
  if (!p) throw new Error('RUST_SERVER_BINARY must point at a release build of topgun-server');
  return p;
}

function start(
  dir: string,
  label: string,
  extraEnv?: Record<string, string>,
): Promise<SpawnedBinary> {
  return spawnBinary({
    binaryPath: binary(),
    label,
    env: {
      // spawnBinary defaults the backend to `null`, which would store nothing.
      STORAGE_BACKEND: 'redb',
      TOPGUN_REDB_PATH: path.join(dir, 'topgun.redb'),
      TOPGUN_WAL_DIR: path.join(dir, 'topgun-wal'),
      ...extraEnv,
    },
  });
}

/** Sends every write over one connection and counts the acknowledgements. */
async function writeAll(port: number, run: string): Promise<number> {
  const client = await connectWithin(port, `backup-writer-${run}`);
  let acked = 0;
  try {
    for (let i = 0; i < LWW_COUNT + OR_COUNT; i++) {
      client.messages.length = 0;
      const payload =
        i < LWW_COUNT
          ? {
              id: `lww-${i}`,
              mapName: LWW_MAP,
              opType: 'PUT',
              key: `k${i}`,
              record: createLWWRecord({ run, i }),
            }
          : {
              id: `or-${i}`,
              mapName: OR_MAP,
              opType: 'OR_ADD',
              key: `k${i}`,
              orRecord: {
                ...createORRecord({ run, i }, 'backup-writer'),
                tag: `backup-${run}-${i}`,
              },
            };
      client.send({ type: 'CLIENT_OP', payload });
      const answer = await firstFrame(client, ['OP_ACK', 'OP_REJECTED', 'ERROR'], ACK_WAIT_MS);
      if (answer?.type === 'OP_ACK') acked++;
    }
  } finally {
    client.close();
  }
  return acked;
}

/** The written value, compared field by field: the server does not keep the field order. */
function same(v: unknown, run: string, i: number): boolean {
  const o = v as { run?: unknown; i?: unknown } | undefined;
  return !!o && o.run === run && o.i === i && Object.keys(o).length === 2;
}

/** Keys whose value on the restored server is not the one that was written. */
async function missing(port: number, run: string): Promise<string[]> {
  const lost: string[] = [];
  for (let i = 0; i < LWW_COUNT + OR_COUNT; i++) {
    const nodeId = `backup-reader-${run}-${i}`;
    if (i < LWW_COUNT) {
      const v = await queryKey(port, LWW_MAP, `k${i}`, nodeId).catch(() => undefined);
      if (!same(v, run, i)) lost.push(`${LWW_MAP}/k${i}`);
    } else {
      // An OR key must hold exactly the one value that was added.
      const vs = await orMapValues(port, OR_MAP, `k${i}`, nodeId).catch(() => []);
      if (!(Array.isArray(vs) && vs.length === 1 && same(vs[0], run, i)))
        lost.push(`${OR_MAP}/k${i}`);
    }
  }
  return lost;
}

interface ArmResult {
  acked: number;
  stopCode: number | null;
  stopSignal: string | null;
  lost: string[];
}

/** Every key path, in the order `missing()` reports them. */
function allKeys(): string[] {
  return Array.from({ length: LWW_COUNT + OR_COUNT }, (_, i) =>
    i < LWW_COUNT ? `${LWW_MAP}/k${i}` : `${OR_MAP}/k${i}`,
  );
}

async function arm(
  run: string,
  stop: 'SIGTERM' | 'SIGKILL',
  { copyWal, holdFlush }: { copyWal: boolean; holdFlush: boolean },
): Promise<ArmResult> {
  const dir1 = fs.mkdtempSync(path.join(os.tmpdir(), `topgun-backup-${run}-a-`));
  const dir2 = fs.mkdtempSync(path.join(os.tmpdir(), `topgun-backup-${run}-b-`));
  let a: SpawnedBinary | null = null;
  let b: SpawnedBinary | null = null;
  try {
    // RUST_LOG is pinned so a developer shell that filters out WARN cannot hide the hold's proof.
    a = await start(
      dir1,
      `${run} A`,
      holdFlush
        ? { TOPGUN_WRITEBEHIND_FLUSH_INTERVAL_MS: HELD_FLUSH_INTERVAL_MS, RUST_LOG: 'info' }
        : undefined,
    );
    const acked = await writeAll(a.port!, run);
    const exit = await a.stop(stop, STOP_WAIT_MS);
    if (holdFlush && !a.stdout().includes(HELD_FLUSH_WARN)) {
      throw new Error(
        `the hold did not take effect: server A did not log "${HELD_FLUSH_WARN}" ` +
          '(TOPGUN_WRITEBEHIND_FLUSH_INTERVAL_MS renamed or not read)',
      );
    }
    fs.cpSync(path.join(dir1, 'topgun-data'), path.join(dir2, 'topgun-data'), { recursive: true });
    if (copyWal) {
      fs.cpSync(path.join(dir1, 'topgun-wal'), path.join(dir2, 'topgun-wal'), { recursive: true });
    }
    b = await start(dir2, `${run} B`);
    const lost = await missing(b.port!, run);
    return { acked, stopCode: exit?.code ?? null, stopSignal: exit?.signal ?? null, lost };
  } finally {
    if (a && a.exit() === null) await a.stop('SIGKILL', STOP_WAIT_MS);
    if (b && b.exit() === null) await b.stop('SIGTERM', STOP_WAIT_MS);
    fs.rmSync(dir1, { recursive: true, force: true });
    fs.rmSync(dir2, { recursive: true, force: true });
  }
}

describe('Integration: backup by stopping the server and copying the data directory and the WAL directory', () => {
  test('SIGTERM, data directory + WAL copied: every acknowledged write is back', async () => {
    const r = await arm('term', 'SIGTERM', { copyWal: true, holdFlush: false });
    process.stderr.write(`[backup] term: ${JSON.stringify(r)}\n`);
    expect(r).toEqual({ acked: LWW_COUNT + OR_COUNT, stopCode: 0, stopSignal: null, lost: [] });
  }, 240_000);

  test('SIGKILL, data directory + WAL copied: every acknowledged write is back', async () => {
    const r = await arm('killwal', 'SIGKILL', { copyWal: true, holdFlush: false });
    process.stderr.write(`[backup] killwal: ${JSON.stringify(r)}\n`);
    expect(r.acked).toBe(LWW_COUNT + OR_COUNT);
    expect(r.lost).toEqual([]);
  }, 240_000);

  test('SIGKILL, flush held, data directory + WAL copied: every acknowledged write is back', async () => {
    const r = await arm('killwalheld', 'SIGKILL', { copyWal: true, holdFlush: true });
    process.stderr.write(`[backup] killwalheld: ${JSON.stringify(r)}\n`);
    expect(r.acked).toBe(LWW_COUNT + OR_COUNT);
    expect(r.lost).toEqual([]);
  }, 240_000);

  test('control: SIGKILL, data directory alone: every acknowledged write is missing', async () => {
    const r = await arm('killfile', 'SIGKILL', { copyWal: false, holdFlush: true });
    process.stderr.write(`[backup] killfile: ${JSON.stringify(r)}\n`);
    expect(r.acked).toBe(LWW_COUNT + OR_COUNT);
    expect(r.lost).toEqual(allKeys());
  }, 240_000);
});
