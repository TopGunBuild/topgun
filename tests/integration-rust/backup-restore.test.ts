/**
 * Integration: the supported backup — stop the server, copy the data directory
 * (`topgun-data/`, recursively) and the WAL directory, start a server on the
 * copy — brings back every acknowledged write. The file at the old store path
 * is not copied: the server writes it again beside the data directory it
 * opens. The control arm copies the data directory alone after an unclean stop
 * and must lose writes: it is what shows the WAL directory is part of the
 * backup.
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

function binary(): string {
  const p = process.env.RUST_SERVER_BINARY;
  if (!p) throw new Error('RUST_SERVER_BINARY must point at a release build of topgun-server');
  return p;
}

function start(dir: string, label: string): Promise<SpawnedBinary> {
  return spawnBinary({
    binaryPath: binary(),
    label,
    env: {
      // spawnBinary defaults the backend to `null`, which would store nothing.
      STORAGE_BACKEND: 'redb',
      TOPGUN_REDB_PATH: path.join(dir, 'topgun.redb'),
      TOPGUN_WAL_DIR: path.join(dir, 'topgun-wal'),
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

async function arm(run: string, stop: 'SIGTERM' | 'SIGKILL', copyWal: boolean): Promise<ArmResult> {
  const dir1 = fs.mkdtempSync(path.join(os.tmpdir(), `topgun-backup-${run}-a-`));
  const dir2 = fs.mkdtempSync(path.join(os.tmpdir(), `topgun-backup-${run}-b-`));
  let a: SpawnedBinary | null = null;
  let b: SpawnedBinary | null = null;
  try {
    a = await start(dir1, `${run} A`);
    const acked = await writeAll(a.port!, run);
    const exit = await a.stop(stop, STOP_WAIT_MS);
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
    const r = await arm('term', 'SIGTERM', true);
    process.stderr.write(`[backup] term: ${JSON.stringify(r)}\n`);
    expect(r).toEqual({ acked: LWW_COUNT + OR_COUNT, stopCode: 0, stopSignal: null, lost: [] });
  }, 240_000);

  test('SIGKILL, data directory + WAL copied: every acknowledged write is back', async () => {
    const r = await arm('killwal', 'SIGKILL', true);
    process.stderr.write(`[backup] killwal: ${JSON.stringify(r)}\n`);
    expect(r.acked).toBe(LWW_COUNT + OR_COUNT);
    expect(r.lost).toEqual([]);
  }, 240_000);

  test('control: SIGKILL, data directory alone: acknowledged writes are missing', async () => {
    const r = await arm('killfile', 'SIGKILL', false);
    process.stderr.write(`[backup] killfile: ${JSON.stringify(r)}\n`);
    expect(r.acked).toBe(LWW_COUNT + OR_COUNT);
    if (r.lost.length === 0) {
      throw new Error(
        'the data-directory-only arm lost nothing: it no longer shows that the WAL directory is part of the backup',
      );
    }
  }, 240_000);
});
