/**
 * Shared pieces of the engine upgrade, resume and rollback tests: one
 * directory layout for both binaries, the drained stop, LWW writes, and the
 * migration's own log lines.
 */

import * as fs from 'fs';
import * as path from 'path';

import { connectWithin, firstFrame, plainLines, BinaryExit, SpawnedBinary } from './two-binary';

export const CLEAN_EXIT: BinaryExit = { code: 0, signal: null };
export const STOP_TIMEOUT_MS = 60_000;
const ACK_WAIT_MS = 10_000;

/** The kept source of a finished engine migration, inside the data directory. */
export const KEPT_FILE = 'engine.redb.pre-rocksdb';
export const MIGRATING_LINE = 'store engine: migrating ';
export const COMPLETE_LINE = 'store engine: migration complete';

export interface Layout {
  dataDir: string;
  walDir: string;
  redb: string;
  rocks: string;
  kept: string;
}

export function layout(dir: string): Layout {
  const dataDir = path.join(dir, 'topgun-data');
  return {
    dataDir,
    walDir: path.join(dir, 'wal'),
    redb: path.join(dataDir, 'engine.redb'),
    rocks: path.join(dataDir, 'engine.rocksdb'),
    kept: path.join(dataDir, KEPT_FILE),
  };
}

/**
 * The environment of either binary on `dir`. `engine` is the older binary's
 * `redb` or the newer one's `rocksdb`; the index descriptor paths are kept in
 * `dir` so no run writes into the checkout.
 */
export function storeEnv(
  dir: string,
  engine: 'redb' | 'rocksdb',
  extra: Record<string, string> = {},
): Record<string, string> {
  return {
    STORAGE_BACKEND: engine,
    TOPGUN_DATA_DIR: path.join(dir, 'topgun-data'),
    TOPGUN_REDB_PATH: path.join(dir, 'topgun.redb'),
    TOPGUN_WAL_DIR: path.join(dir, 'wal'),
    TOPGUN_INDEX_PATH: path.join(dir, 'scalar_indexes.json'),
    TOPGUN_VECTOR_INDEX_PATH: path.join(dir, 'vector_indexes.json'),
    ...extra,
  };
}

export interface Stopped {
  exit: BinaryExit | null;
  /** The stop logged `rollback hints written` with `drained=true`. */
  drained: boolean;
}

/** SIGTERM, then whether the drain finished, which a rollback or an upgrade needs. */
export async function drainedStop(server: SpawnedBinary): Promise<Stopped> {
  const exit = await server.stop('SIGTERM', STOP_TIMEOUT_MS);
  const drained = plainLines(server.stdout()).some(
    (l) => l.includes('rollback hints written') && l.includes('drained=true'),
  );
  return { exit, drained };
}

/** Writes each `[key, value]` of `map` as an LWW PUT on its own connection; the answer per key. */
export async function putLww(
  port: number,
  map: string,
  entries: [string, unknown][],
  tag: string,
  extra: Record<string, unknown> = {},
): Promise<Record<string, string>> {
  const answers: Record<string, string> = {};
  const client = await connectWithin(port, `${tag}-writer`);
  try {
    for (const [i, [key, value]] of entries.entries()) {
      client.messages.length = 0;
      client.send({
        type: 'CLIENT_OP',
        payload: {
          id: `${tag}-${i}`,
          mapName: map,
          opType: 'PUT',
          key,
          record: { value, timestamp: { millis: Date.now(), counter: 0, nodeId: tag }, ...extra },
        },
      });
      const frame = await firstFrame(client, ['OP_ACK', 'OP_REJECTED', 'ERROR'], ACK_WAIT_MS);
      answers[key] = frame?.type ?? 'none';
    }
  } finally {
    client.close();
  }
  return answers;
}

/** The lines of a capture (stdout and stderr) that contain `needle`. */
export function linesWith(server: SpawnedBinary, needle: string): string[] {
  return plainLines(`${server.stdout()}\n${server.stderr()}`).filter((l) => l.includes(needle));
}

/** `file`, `directory` or `absent`. */
export function kindOf(p: string): string {
  try {
    return fs.lstatSync(p).isDirectory() ? 'directory' : 'file';
  } catch {
    return 'absent';
  }
}
