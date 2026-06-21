/**
 * Cross-boundary SQL honesty test against the real Rust server.
 *
 * SQL (DataFusion) is a compile-time opt-in (`--features datafusion`) and the
 * default server build does NOT include it. Even a datafusion build does not yet
 * wire a SQL backend, so `client.sql()` is not functional on any current server.
 *
 * This suite proves the HONEST GATE on the DEFAULT binary (the one the rest of
 * the integration suite already builds — no extra datafusion build needed):
 *
 *   1. `client.sql()` rejects with a typed `SqlError` whose `code` is
 *      `FEATURE_DISABLED` — a machine-distinguishable "feature off" signal, not
 *      an opaque "internal error".
 *   2. It rejects PROMPTLY (server replies with a correlated SQL_QUERY_RESP)
 *      rather than hanging until the client's request timeout. A single-message
 *      dispatch error used to be logged-and-dropped server-side, leaving the
 *      client to stall — indistinguishable from a network failure. The fast
 *      rejection here is the regression guard for that.
 *
 * The positive path (real SELECT/WHERE/GROUP BY over data on a datafusion
 * binary) is intentionally NOT covered here: wiring the SQL engine end-to-end —
 * and giving SQL per-table authorization — is deferred (see TODO-444). When that
 * lands, a positive suite built against a `--features datafusion` binary joins
 * this gate.
 */

import { TopGunClient, SqlError, SyncState } from '@topgunbuild/client';

import { spawnRustServer, createTestToken, MemoryStorageAdapter, SpawnedServer } from './helpers';

function waitMs(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function waitForState(
  client: TopGunClient,
  state: SyncState,
  timeoutMs = 20_000,
): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (client.getConnectionState() === state) return;
    await waitMs(50);
  }
  throw new Error(`Timed out waiting for ${state}; last state = ${client.getConnectionState()}`);
}

function makeClient(port: number, userId: string): TopGunClient {
  const token = createTestToken(userId, ['ADMIN']);
  return new TopGunClient({
    serverUrl: `ws://localhost:${port}/ws`,
    storage: new MemoryStorageAdapter(),
    auth: { getToken: async () => token },
    backoff: { initialDelayMs: 200, maxDelayMs: 400, jitter: true },
  });
}

describe('Integration: SQL honesty (client <-> Rust server)', () => {
  let server: SpawnedServer | null = null;
  let client: TopGunClient | null = null;

  beforeAll(async () => {
    server = await spawnRustServer();
  });

  afterEach(async () => {
    if (client) {
      await client.close().catch(() => {});
      client = null;
    }
  });

  afterAll(async () => {
    if (server) {
      await server.cleanup();
      server = null;
    }
  });

  test('client.sql() rejects with a typed FEATURE_DISABLED SqlError on a server without datafusion', async () => {
    client = makeClient(server!.port, 'sql-user-1');
    await client.start();
    await waitForState(client, SyncState.CONNECTED);

    const err = await client.sql('SELECT 1').then(
      () => {
        throw new Error('expected client.sql() to reject on a server without SQL enabled');
      },
      (e) => e,
    );

    expect(err).toBeInstanceOf(SqlError);
    expect((err as SqlError).code).toBe('FEATURE_DISABLED');
    // Honest, human-readable message — not "internal error".
    expect((err as SqlError).message.toLowerCase()).toContain('sql is not available');
    expect((err as SqlError).message.toLowerCase()).not.toContain('internal error');
  });

  test('client.sql() rejects promptly rather than hanging until the request timeout', async () => {
    client = makeClient(server!.port, 'sql-user-2');
    await client.start();
    await waitForState(client, SyncState.CONNECTED);

    const start = Date.now();
    await client.sql('SELECT * FROM whatever').catch(() => {});
    const elapsedMs = Date.now() - start;

    // The default SQL request timeout is 30s; a correlated server reply lands in
    // well under a second. Use a generous ceiling so the assertion is about
    // "did NOT time out", not raw latency.
    expect(elapsedMs).toBeLessThan(5_000);
  });
});
