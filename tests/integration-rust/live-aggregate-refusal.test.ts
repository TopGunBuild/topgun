/**
 * A live query cannot carry groupBy / aggregations: the server answers a live
 * grouped QUERY_SUB with QUERY_RESP code QUERY_LIVE_AGGREGATE_UNSUPPORTED and
 * registers nothing, while a one-shot (`snapshotOnly: true`) grouped QUERY_SUB is
 * answered with the grouped rows and is not registered either. The client
 * refuses a live grouped handle before sending anything, and `queryOnce` keeps
 * serving grouped results.
 *
 * The "no QUERY_UPDATE" checks write from a second connection: the server does
 * not echo a writer's own changes back to it, so a self-write would prove nothing.
 */

import { TopGunClient, QueryError, SyncState } from '@topgunbuild/client';

import {
  createLWWRecord,
  createRustTestClient,
  createTestToken,
  MemoryStorageAdapter,
  spawnRustServer,
  TestClient,
  waitForSync,
} from './helpers';

const REFUSAL = 'QUERY_LIVE_AGGREGATE_UNSUPPORTED';

const SEED = [
  { key: 'a1', value: { category: 'a', price: 10 } },
  { key: 'a2', value: { category: 'a', price: 25 } },
  { key: 'b1', value: { category: 'b', price: 50 } },
];

function waitMs(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function put(
  writer: TestClient,
  mapName: string,
  key: string,
  value: Record<string, unknown>,
): Promise<void> {
  writer.messages.length = 0;
  writer.send({
    type: 'CLIENT_OP',
    payload: {
      id: `put-${mapName}-${key}`,
      mapName,
      opType: 'PUT',
      key,
      record: createLWWRecord(value),
    },
  });
  await writer.waitForMessage('OP_ACK');
}

async function seed(writer: TestClient, mapName: string): Promise<void> {
  for (const rec of SEED) {
    await put(writer, mapName, rec.key, rec.value);
  }
  await waitForSync(200);
}

/** Rows of a grouped answer, indexed by their `category` column. */
function byCategory(results: Array<{ value: Record<string, number> & { category: string } }>) {
  const groups: Record<string, Record<string, number>> = {};
  for (const row of results) groups[row.value.category] = row.value;
  return groups;
}

describe('Integration: live aggregate refusal (Rust server)', () => {
  let cleanup: () => Promise<void>;
  let port: number;
  let subscriber: TestClient;
  let writer: TestClient;

  beforeAll(async () => {
    const server = await spawnRustServer();
    port = server.port;
    cleanup = server.cleanup;
  });

  afterAll(async () => {
    await cleanup();
  });

  beforeEach(async () => {
    subscriber = await createRustTestClient(port, {
      nodeId: 'agg-refusal-sub',
      userId: 'agg-refusal-sub',
      roles: ['ADMIN'],
    });
    writer = await createRustTestClient(port, {
      nodeId: 'agg-refusal-writer',
      userId: 'agg-refusal-writer',
      roles: ['ADMIN'],
    });
    await subscriber.waitForMessage('AUTH_ACK');
    await writer.waitForMessage('AUTH_ACK');
  });

  afterEach(() => {
    subscriber.close();
    writer.close();
  });

  /** Sends a QUERY_SUB and returns its QUERY_RESP. */
  async function subscribe(payload: Record<string, unknown>): Promise<any> {
    subscriber.messages.length = 0;
    subscriber.send({ type: 'QUERY_SUB', payload });
    return subscriber.waitForMessage('QUERY_RESP');
  }

  /** A matching write from the other connection reaches no QUERY_UPDATE for `queryId`. */
  async function expectNoUpdateAfterWrite(mapName: string, queryId: string): Promise<void> {
    subscriber.messages.length = 0;
    await put(writer, mapName, 'a3', { category: 'a', price: 5 });
    await waitMs(1_000);
    const updates = subscriber.messages.filter(
      (m) => m.type === 'QUERY_UPDATE' && m.payload?.queryId === queryId,
    );
    expect(updates).toEqual([]);
  }

  test('(a) a live QUERY_SUB with groupBy is refused and stays silent', async () => {
    const mapName = `live-agg-a-${Date.now()}`;
    await seed(writer, mapName);
    const resp = await subscribe({
      queryId: 'live-agg-a',
      mapName,
      query: { groupBy: ['category'] },
    });
    expect(resp.payload.code).toBe(REFUSAL);
    expect(resp.payload.results).toEqual([]);
    await expectNoUpdateAfterWrite(mapName, 'live-agg-a');
  });

  test('(b) a live QUERY_SUB with aggregations only is refused and stays silent', async () => {
    const mapName = `live-agg-b-${Date.now()}`;
    await seed(writer, mapName);
    const resp = await subscribe({
      queryId: 'live-agg-b',
      mapName,
      query: { aggregations: [{ func: 'sum', field: 'price' }] },
    });
    expect(resp.payload.code).toBe(REFUSAL);
    expect(resp.payload.results).toEqual([]);
    await expectNoUpdateAfterWrite(mapName, 'live-agg-b');
  });

  test('(c) a snapshotOnly grouped QUERY_SUB gets the groups and no later update', async () => {
    const mapName = `live-agg-c-${Date.now()}`;
    await seed(writer, mapName);
    const resp = await subscribe({
      queryId: 'live-agg-c',
      mapName,
      query: { groupBy: ['category'] },
      snapshotOnly: true,
    });
    expect(resp.payload.code).toBeUndefined();
    const groups = byCategory(resp.payload.results);
    expect(Object.keys(groups).sort()).toEqual(['a', 'b']);
    expect(groups.a.__count).toBe(2);
    expect(groups.b.__count).toBe(1);
    await expectNoUpdateAfterWrite(mapName, 'live-agg-c');
  });

  describe('with the SDK client', () => {
    let client: TopGunClient;

    beforeEach(async () => {
      const token = createTestToken('agg-refusal-sdk', ['ADMIN']);
      client = new TopGunClient({
        serverUrl: `ws://localhost:${port}/ws`,
        storage: new MemoryStorageAdapter(),
        auth: { getToken: async () => token },
      });
      await client.start();
      const deadline = Date.now() + 20_000;
      while (client.getConnectionState() !== SyncState.CONNECTED) {
        if (Date.now() > deadline) throw new Error('client did not connect');
        await waitMs(50);
      }
    });

    afterEach(async () => {
      await client.close().catch(() => {});
    });

    test('(d) queryOnce with groupBy + sum resolves with the group rows', async () => {
      const mapName = `live-agg-d-${Date.now()}`;
      await seed(writer, mapName);
      const rows = (await client.queryOnce(mapName, {
        groupBy: ['category'],
        aggregations: [{ func: 'sum', field: 'price' }],
      })) as unknown as Array<Record<string, number> & { category: string }>;
      const groups = byCategory(rows.map((value) => ({ value })));
      expect(Object.keys(groups).sort()).toEqual(['a', 'b']);
      expect(groups.a.__count).toBe(2);
      expect(groups.a.__sum_price).toBe(35);
      expect(groups.b.__count).toBe(1);
      expect(groups.b.__sum_price).toBe(50);
    });

    test('(e) client.query with groupBy throws QueryError', () => {
      expect(() => client.query('live-agg-e', { groupBy: ['category'] })).toThrow(QueryError);
      try {
        client.query('live-agg-e', { groupBy: ['category'] });
      } catch (err) {
        expect((err as QueryError).code).toBe(REFUSAL);
      }
    });
  });
});
