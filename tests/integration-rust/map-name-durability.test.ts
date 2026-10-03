/**
 * Integration: a write the server acknowledged is still there after a restart,
 * whatever characters its map name uses.
 *
 * The client accepts any map name without a ":" and its own error text
 * recommends "-" and "/" as separators. The embedded store (the default
 * backend) accepts only identifier-shaped names, so a write to `user-profiles`
 * is applied in memory and acknowledged while the store refuses every attempt
 * to persist it. This test pins what a restart then does to that write.
 *
 * `per_op` makes every acknowledged write durable in the WAL at once, so a
 * missing record after the restart cannot be an fsync race. The server is given
 * time to run its write-behind flush (delay, then every retry) before it is
 * stopped, and is stopped gracefully, so a missing record cannot be an
 * interrupted flush either.
 *
 * The identifier-shaped name is the control: it must survive, which proves the
 * restart-and-read procedure itself is sound.
 */

import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

import {
  spawnRustServer,
  createRustTestClient,
  createLWWRecord,
  completeMerkleSync,
  waitForSync,
  waitUntil,
  SpawnedServer,
  TestClient,
} from './helpers';

const CONTROL_MAP = 'user_profiles';
const DASH_MAP = 'user-profiles';
const SLASH_MAP = 'users/profiles';
const MAPS = [CONTROL_MAP, DASH_MAP, SLASH_MAP];

const KEY = 'alice';
const VALUE = { name: 'Alice', plan: 'pro' };

const SIGNAL_TIMEOUT_MS = 10_000;
/**
 * Write-behind holds a write for 1 s, then tries it up to three times with
 * 1 s and 2 s of backoff in between. Entries are retried one after another, so
 * two refused maps take about twice as long.
 */
const FLUSH_SETTLE_MS = 10_000;

interface Observed {
  acked: boolean;
  queryBeforeRestart: unknown;
  queryAfterRestart: unknown;
  merkleAfterRestart: unknown;
}

describe('Integration: an acknowledged write survives a restart for any map name the client accepts', () => {
  let server: SpawnedServer | null = null;
  let dataDir: string | null = null;
  const observed = new Map<string, Observed>();
  /** WARN/ERROR lines the server logged, per incarnation, kept as evidence of what it did. */
  const serverLog: string[] = [];

  function captureLog(spawned: SpawnedServer, incarnation: string): SpawnedServer {
    spawned.process.stdout?.on('data', (chunk: Buffer) => {
      for (const line of chunk.toString('utf8').split('\n')) {
        if (/WARN|ERROR/.test(line)) serverLog.push(`[${incarnation}] ${line}`);
      }
    });
    return spawned;
  }

  /** A read that fails is itself an observation, so it is recorded instead of aborting the run. */
  async function attempt(read: () => Promise<unknown>): Promise<unknown> {
    try {
      return await read();
    } catch (err) {
      return `READ FAILED: ${(err as Error).message}`;
    }
  }

  function startServer(dir: string): Promise<SpawnedServer> {
    return spawnRustServer({
      env: {
        STORAGE_BACKEND: 'redb',
        TOPGUN_REDB_PATH: path.join(dir, 'topgun.redb'),
        TOPGUN_WAL_DIR: path.join(dir, 'wal'),
        TOPGUN_WAL_FSYNC_POLICY: 'per_op',
      },
    });
  }

  async function stopServer(): Promise<void> {
    if (!server) return;
    const stopping = server;
    server = null;
    await stopping.cleanup();
    await waitUntil(
      () => stopping.process.exitCode !== null || stopping.process.signalCode !== null,
      SIGNAL_TIMEOUT_MS,
    );
  }

  async function connect(port: number, nodeId: string): Promise<TestClient> {
    const client = await createRustTestClient(port, {
      nodeId,
      userId: 'map-name-user',
      roles: ['ADMIN'],
    });
    await client.waitForMessage('AUTH_ACK', SIGNAL_TIMEOUT_MS);
    return client;
  }

  /** Reads one key of a map through a snapshot query, over a connection of its own. */
  async function queryValue(port: number, mapName: string, nodeId: string): Promise<unknown> {
    const client = await connect(port, nodeId);
    try {
      client.send({
        type: 'QUERY_SUB',
        payload: { queryId: `q-${nodeId}`, mapName, query: {} },
      });
      const response = await client.waitForMessage('QUERY_RESP', SIGNAL_TIMEOUT_MS);
      const row = (response.payload.results as any[]).find((r) => r.key === KEY);
      return row ? (row.record?.value ?? row.value) : undefined;
    } finally {
      client.close();
    }
  }

  /** Reads one key of a map through a full Merkle walk from an empty tree. */
  async function merkleValue(port: number, mapName: string, nodeId: string): Promise<unknown> {
    const client = await connect(port, nodeId);
    try {
      const records = await completeMerkleSync(client, mapName, SIGNAL_TIMEOUT_MS);
      return records.get(KEY)?.value;
    } finally {
      client.close();
    }
  }

  beforeAll(async () => {
    dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'topgun-map-name-'));
    server = captureLog(await startServer(dataDir), 'first run');

    const writer = await connect(server.port, 'map-name-writer');
    for (const [i, mapName] of MAPS.entries()) {
      writer.messages.length = 0;
      writer.send({
        type: 'CLIENT_OP',
        payload: {
          id: `op-map-name-${i}`,
          mapName,
          opType: 'PUT',
          key: KEY,
          record: createLWWRecord(VALUE),
        },
      });
      let acked = true;
      try {
        await writer.waitForMessage('OP_ACK', SIGNAL_TIMEOUT_MS);
      } catch {
        acked = false;
      }
      observed.set(mapName, {
        acked,
        queryBeforeRestart: undefined,
        queryAfterRestart: undefined,
        merkleAfterRestart: undefined,
      });
    }
    writer.close();

    for (const [i, mapName] of MAPS.entries()) {
      const port = server.port;
      observed.get(mapName)!.queryBeforeRestart = await attempt(() =>
        queryValue(port, mapName, `map-name-before-${i}`),
      );
    }

    await waitForSync(FLUSH_SETTLE_MS);
    await stopServer();
    server = captureLog(await startServer(dataDir), 'after restart');
    const port = server.port;

    for (const [i, mapName] of MAPS.entries()) {
      const seen = observed.get(mapName)!;
      seen.queryAfterRestart = await attempt(() =>
        queryValue(port, mapName, `map-name-after-q-${i}`),
      );
      seen.merkleAfterRestart = await attempt(() =>
        merkleValue(port, mapName, `map-name-after-m-${i}`),
      );
    }

    // Written straight to stderr (the suite silences console.log) so a failing
    // run records exactly what the server did.
    process.stderr.write(
      `map-name durability observed:\n${JSON.stringify([...observed.entries()], null, 2)}\n` +
        `server WARN/ERROR lines:\n${serverLog.join('\n')}\n`,
    );
  }, 240_000);

  afterAll(async () => {
    await stopServer();
    if (dataDir) fs.rmSync(dataDir, { recursive: true, force: true });
  });

  describe.each(MAPS)('map %p', (mapName) => {
    test('the write is acknowledged and readable before the restart', () => {
      const seen = observed.get(mapName)!;
      expect(seen.acked).toBe(true);
      expect(seen.queryBeforeRestart).toEqual(VALUE);
    });

    test('a query after the restart returns the acknowledged write', () => {
      expect(observed.get(mapName)!.queryAfterRestart).toEqual(VALUE);
    });

    test('a Merkle sync after the restart returns the acknowledged write', () => {
      expect(observed.get(mapName)!.merkleAfterRestart).toEqual(VALUE);
    });
  });
});
