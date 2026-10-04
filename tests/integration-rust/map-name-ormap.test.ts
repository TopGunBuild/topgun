/**
 * Integration: on the embedded store, an OR-Map entry added under a map name
 * with a "-" in it is acknowledged and is still there after a restart; and a
 * write under the `$sys/` prefix is acknowledged and readable after a restart.
 *
 * `per_op` makes an acknowledged write durable in the WAL at once and the
 * server is stopped gracefully, so a missing value after the restart is the
 * store's doing and not an fsync race or an interrupted flush.
 *
 * The two probes use a server and a data directory each, so what one name does
 * to the server cannot change what the other observes.
 *
 * Stated boundary of the `$sys/` probe: the server leaves `$sys/` maps out of
 * the Merkle trees it builds at startup, so the Merkle answer for such a map
 * after a restart is printed and not asserted.
 */

import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

import { createLWWRecord, createORRecord } from './helpers';
import {
  spawnBinary,
  connectWithin,
  firstFrame,
  queryKey,
  merkleKey,
  orMapValues,
  observe,
  plainLines,
  SpawnedBinary,
  BinaryExit,
} from './helpers/two-binary';

const OR_MAP = 'user-profiles';
const OR_KEY = 'tags';
const OR_VALUE = 'blue';

const SYS_MAP = '$sys/probe';
const SYS_KEY = 'k';
const SYS_VALUE = { probe: true };

const ACK_WAIT_MS = 5_000;
/** A graceful stop may spend up to 30 s draining buffered writes; past this the process is killed and the exit status says so. */
const STOP_TIMEOUT_MS = 40_000;

function serverBinary(): string {
  const binary =
    process.env.RUST_SERVER_BINARY ??
    path.resolve(__dirname, '..', '..', 'target', 'release', 'topgun-server');
  if (!fs.existsSync(binary)) {
    throw new Error(
      `No server binary at ${binary}. Build it (cargo build --release -p topgun-server) or set RUST_SERVER_BINARY.`,
    );
  }
  return binary;
}

function start(dir: string, label: string): Promise<SpawnedBinary> {
  return spawnBinary({
    binaryPath: serverBinary(),
    label,
    env: {
      STORAGE_BACKEND: 'redb',
      TOPGUN_REDB_PATH: path.join(dir, 'topgun.redb'),
      TOPGUN_WAL_DIR: path.join(dir, 'wal'),
      TOPGUN_WAL_FSYNC_POLICY: 'per_op',
    },
  });
}

function requirePort(server: SpawnedBinary): number {
  if (server.port === null) {
    throw new Error(`${server.label} did not print PORT=; stderr:\n${server.stderr()}`);
  }
  return server.port;
}

function warnings(server: SpawnedBinary): string[] {
  return [...plainLines(server.stdout()), ...plainLines(server.stderr())]
    .filter((line) => /\b(WARN|ERROR)\b|FATAL/.test(line))
    .map((line) => line.slice(0, 400));
}

interface Observed {
  /** Types of the frames the write was answered with inside the ack wait. */
  answer?: string[] | string;
  beforeRestart?: unknown;
  stop?: BinaryExit | null;
  restartPort?: number | null;
  afterRestart?: unknown;
  merkleAfterRestart?: unknown;
  log?: string[];
}

/** Sends one write, waits a bounded time for its acknowledgement, and returns the types of every frame received. */
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

describe('Integration: OR-Map under a map name with "-" on the embedded store', () => {
  const o: Observed = {};
  let dataDir: string | null = null;
  let server: SpawnedBinary | null = null;

  beforeAll(async () => {
    dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'topgun-map-name-ormap-'));
    server = await start(dataDir, 'ormap first run');
    const port = requirePort(server);

    o.answer = await observe(() =>
      writeAndCollect(port, 'ormap-writer', {
        id: '1',
        mapName: OR_MAP,
        opType: 'OR_ADD',
        key: OR_KEY,
        orRecord: createORRecord(OR_VALUE, 'ormap-writer'),
      }),
    );
    o.beforeRestart = await observe(() => orMapValues(port, OR_MAP, OR_KEY, 'ormap-before'));

    o.stop = await server.stop('SIGTERM', STOP_TIMEOUT_MS);
    o.log = warnings(server).map((line) => `[first run] ${line}`);

    server = await start(dataDir, 'ormap after restart');
    o.restartPort = server.port;
    if (server.port !== null) {
      const restarted = server.port;
      o.afterRestart = await observe(() => orMapValues(restarted, OR_MAP, OR_KEY, 'ormap-after'));
    }
    o.log.push(...warnings(server).map((line) => `[after restart] ${line}`));
    await server.stop('SIGTERM', STOP_TIMEOUT_MS);
    server = null;

    // Straight to stderr: the suite silences console.log.
    process.stderr.write(`[map-name-ormap] OR_ADD on ${OR_MAP}:\n${JSON.stringify(o, null, 2)}\n`);
  }, 240_000);

  afterAll(async () => {
    if (server) await server.stop('SIGKILL', 5_000);
    if (dataDir) fs.rmSync(dataDir, { recursive: true, force: true });
  });

  test('the OR_ADD is acknowledged', () => {
    expect(o.answer).toEqual(['OP_ACK']);
  });

  test('the server stops cleanly after the write', () => {
    expect(o.stop).toEqual({ code: 0, signal: null });
  });

  test('after the restart, an OR-Map sync from an empty client returns the added entry', () => {
    expect(typeof o.restartPort).toBe('number');
    expect(o.afterRestart).toEqual([OR_VALUE]);
  });
});

describe('Integration: a write under the $sys/ prefix on the embedded store', () => {
  const o: Observed = {};
  let dataDir: string | null = null;
  let server: SpawnedBinary | null = null;

  beforeAll(async () => {
    dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'topgun-map-name-sys-'));
    server = await start(dataDir, '$sys first run');
    const port = requirePort(server);

    o.answer = await observe(() =>
      writeAndCollect(port, 'sys-writer', {
        id: '1',
        mapName: SYS_MAP,
        opType: 'PUT',
        key: SYS_KEY,
        record: createLWWRecord(SYS_VALUE),
      }),
    );
    o.beforeRestart = await observe(() => queryKey(port, SYS_MAP, SYS_KEY, 'sys-before'));

    o.stop = await server.stop('SIGTERM', STOP_TIMEOUT_MS);
    o.log = warnings(server).map((line) => `[first run] ${line}`);

    server = await start(dataDir, '$sys after restart');
    o.restartPort = server.port;
    if (server.port !== null) {
      const restarted = server.port;
      o.afterRestart = await observe(() => queryKey(restarted, SYS_MAP, SYS_KEY, 'sys-after-q'));
      o.merkleAfterRestart = await observe(() =>
        merkleKey(restarted, SYS_MAP, SYS_KEY, 'sys-after-m'),
      );
    }
    o.log.push(...warnings(server).map((line) => `[after restart] ${line}`));
    await server.stop('SIGTERM', STOP_TIMEOUT_MS);
    server = null;

    process.stderr.write(
      `[map-name-ormap] PUT on ${SYS_MAP} (the Merkle answer is recorded, not asserted):\n` +
        `${JSON.stringify(o, null, 2)}\n`,
    );
  }, 240_000);

  afterAll(async () => {
    if (server) await server.stop('SIGKILL', 5_000);
    if (dataDir) fs.rmSync(dataDir, { recursive: true, force: true });
  });

  test('the PUT is acknowledged', () => {
    expect(o.answer).toEqual(['OP_ACK']);
  });

  test('the server stops cleanly after the write', () => {
    expect(o.stop).toEqual({ code: 0, signal: null });
  });

  test('after the restart, a query returns the value', () => {
    expect(typeof o.restartPort).toBe('number');
    expect(o.afterRestart).toEqual(SYS_VALUE);
  });
});
