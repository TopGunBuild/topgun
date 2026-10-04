/**
 * Integration: on the Postgres backend, a write the server acknowledged is
 * still there after a restart for map names that use "-", "/" and ":".
 *
 * Postgres keeps the map name as a column value, so none of these characters
 * needs any mapping there. This file pins that against a real database: it is
 * the same write, restart and read as the embedded-store durability test, with
 * the backend switched.
 *
 * It needs two things the default run does not have: `DATABASE_URL` naming a
 * reachable Postgres database, and a server binary built with the `postgres`
 * feature (`RUST_SERVER_BINARY`). Without `DATABASE_URL` the whole file is
 * skipped and says so on stderr; nothing here passes without having run.
 *
 * The database outlives the run, so the key and the value are unique per run:
 * a value read after the restart can only be the one this run wrote.
 */

import * as crypto from 'crypto';
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

const DATABASE_URL = process.env.DATABASE_URL;

const DASH_MAP = 'user-profiles';
const SLASH_MAP = 'users/profiles';
const COLON_MAP = 'notes:abc';
const MAPS = [DASH_MAP, SLASH_MAP, COLON_MAP];

const RUN_ID = crypto.randomUUID();
const KEY = `alice-${RUN_ID}`;
const VALUE = { name: 'Alice', run: RUN_ID };

const SIGNAL_TIMEOUT_MS = 10_000;
/** Write-behind holds a write for 1 s before it flushes; this leaves room for the flush and its retries. */
const FLUSH_SETTLE_MS = 10_000;

interface Observed {
  acked: boolean;
  queryBeforeRestart: unknown;
  queryAfterRestart: unknown;
  merkleAfterRestart: unknown;
}

if (!DATABASE_URL) {
  // Straight to stderr: the suite silences console.log, and a skipped file
  // that does not say why reads as a file that passed.
  process.stderr.write(
    'SKIPPED map-name-postgres: DATABASE_URL is not set. To run it, point DATABASE_URL at a ' +
      'Postgres database and RUST_SERVER_BINARY at a server built with `--features postgres`.\n',
  );
}

const describeWithPostgres = DATABASE_URL ? describe : describe.skip;

describeWithPostgres(
  'Integration: an acknowledged write survives a restart on Postgres for map names with "-", "/" and ":"',
  () => {
    let server: SpawnedServer | null = null;
    let walDir: string | null = null;
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
          STORAGE_BACKEND: 'postgres',
          DATABASE_URL: DATABASE_URL!,
          TOPGUN_WAL_DIR: dir,
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
      walDir = fs.mkdtempSync(path.join(os.tmpdir(), 'topgun-map-name-pg-'));
      server = captureLog(await startServer(walDir), 'first run');

      const writer = await connect(server.port, 'map-name-pg-writer');
      for (const [i, mapName] of MAPS.entries()) {
        writer.messages.length = 0;
        writer.send({
          type: 'CLIENT_OP',
          payload: {
            id: `op-map-name-pg-${i}`,
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
          queryValue(port, mapName, `map-name-pg-before-${i}`),
        );
      }

      await waitForSync(FLUSH_SETTLE_MS);
      await stopServer();
      server = captureLog(await startServer(walDir), 'after restart');
      const port = server.port;

      for (const [i, mapName] of MAPS.entries()) {
        const seen = observed.get(mapName)!;
        seen.queryAfterRestart = await attempt(() =>
          queryValue(port, mapName, `map-name-pg-after-q-${i}`),
        );
        seen.merkleAfterRestart = await attempt(() =>
          merkleValue(port, mapName, `map-name-pg-after-m-${i}`),
        );
      }

      // Written straight to stderr (the suite silences console.log) so a run
      // records exactly what the server did.
      process.stderr.write(
        `map-name postgres observed:\n${JSON.stringify([...observed.entries()], null, 2)}\n` +
          `server WARN/ERROR lines:\n${serverLog.join('\n')}\n`,
      );
    }, 240_000);

    afterAll(async () => {
      await stopServer();
      if (walDir) fs.rmSync(walDir, { recursive: true, force: true });
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
  },
);
