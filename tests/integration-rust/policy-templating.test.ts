/**
 * Policy templates over the wire (TG-AUTHZ-003, TG-AUTHZ-004).
 *
 * One server with tenancy on (`TOPGUN_TENANT_CLAIM=org_id`) and one policy set
 * for every tenant and visitor:
 *
 * - `{auth.tenant}.visitor.{auth.id}.**` (all): each visitor reaches only its
 *   own maps, also when the same `sub` exists in two tenants;
 * - `acme.cart` write with `keyPattern {auth.id}`: a visitor writes only its
 *   own key, whatever owner it puts in the value; beside it, the documented
 *   `data.ownerId == auth.id` condition on `acme.legacy-cart`, which a forged
 *   owner satisfies (the contrast the docs state);
 * - `{auth.tenant}.signal.{auth.id}` (read, write): a visitor publishes and
 *   subscribes only to its own topic. A monitor principal reads and writes
 *   every `acme.signal.*` topic through a condition on its id.
 *
 * Refused reads and topic operations answer with no frame, and the server
 * dispatches a connection's messages concurrently, so "nothing arrived" is
 * decided after a positive barrier on the same connection plus a settle.
 */

import * as jwt from 'jsonwebtoken';
import {
  spawnRustServer,
  createRustTestClient,
  createTestToken,
  createLWWRecord,
  waitForSync,
  waitUntil,
  SpawnedServer,
  TestClient,
} from './helpers';

const JWT_SECRET = 'test-e2e-secret';
const TENANT_CLAIM = 'org_id';
const SETTLE_MS = 500;
const adminToken = createTestToken('admin-setup', ['admin']);

async function visitor(port: number, sub: string, tenant: string): Promise<TestClient> {
  const client = await createRustTestClient(port, { userId: sub, autoAuth: false });
  await client.waitForMessage('AUTH_REQUIRED', 8000);
  const token = jwt.sign({ sub, roles: ['USER'], [TENANT_CLAIM]: tenant }, JWT_SECRET, {
    expiresIn: '1h',
  });
  client.send({ type: 'AUTH', token });
  await client.waitForMessage('AUTH_ACK', 8000);
  return client;
}

async function createPolicy(port: number, policy: Record<string, string>): Promise<void> {
  const resp = await fetch(`http://localhost:${port}/api/admin/policies`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${adminToken}` },
    body: JSON.stringify(policy),
  });
  if (resp.status !== 201) {
    throw new Error(`policy ${policy.id}: HTTP ${resp.status} ${await resp.text()}`);
  }
}

/** One OP_BATCH write; resolves with OP_ACK, OP_REJECTED or ERROR. */
async function write(
  client: TestClient,
  mapName: string,
  key: string,
  value: unknown,
): Promise<{ type: string; payload?: any }> {
  const from = client.messages.length;
  client.send({
    type: 'OP_BATCH',
    payload: {
      ops: [
        {
          id: `op-${mapName}-${key}-${Math.random().toString(36).slice(2)}`,
          mapName,
          opType: 'PUT',
          key,
          record: createLWWRecord(value),
        },
      ],
    },
  });
  let reply: { type: string; payload?: any } | undefined;
  await waitUntil(() => {
    reply = client.messages
      .slice(from)
      .find((m) => ['OP_ACK', 'OP_REJECTED', 'ERROR'].includes(m.type));
    return reply !== undefined;
  }, 5000);
  return reply!;
}

function expectRefused(reply: { type: string; payload?: any }): void {
  expect(reply.type).toBe('OP_REJECTED');
  expect(reply.payload?.code).toBe(403);
}

/** A snapshot query; the rows it returns, or `null` when no answer came before a PONG. */
async function query(client: TestClient, mapName: string): Promise<any[] | null> {
  const queryId = `q-${mapName}-${Math.random().toString(36).slice(2)}`;
  client.send({ type: 'QUERY_SUB', payload: { queryId, mapName, query: {} } });
  const pingFrom = client.messages.length;
  await waitForSync(SETTLE_MS);
  client.send({ type: 'PING', timestamp: Date.now() });
  await waitUntil(() => client.messages.some((m, i) => i >= pingFrom && m.type === 'PONG'), 5000);
  const resp = client.messages.find(
    (m) => m.type === 'QUERY_RESP' && m.payload?.queryId === queryId,
  );
  return resp ? (resp.payload.results as any[]) : null;
}

function topicMessages(client: TestClient, topic: string): any[] {
  return client.messages.filter((m) => m.type === 'TOPIC_MESSAGE' && m.payload?.topic === topic);
}

describe('Integration: policy templates (placeholders, keyPattern)', () => {
  let server: SpawnedServer;
  const open: TestClient[] = [];

  beforeAll(async () => {
    server = await spawnRustServer({ env: { TOPGUN_TENANT_CLAIM: TENANT_CLAIM } });
    const { port } = server;
    await createPolicy(port, {
      id: 'visitor-own',
      mapPattern: '{auth.tenant}.visitor.{auth.id}.**',
      action: 'all',
      effect: 'allow',
    });
    await createPolicy(port, {
      id: 'cart-own-key',
      mapPattern: 'acme.cart',
      keyPattern: '{auth.id}',
      action: 'write',
      effect: 'allow',
    });
    await createPolicy(port, {
      id: 'legacy-cart-owner',
      mapPattern: 'acme.legacy-cart',
      action: 'write',
      effect: 'allow',
      conditionExpr: 'data.ownerId == auth.id',
    });
    for (const action of ['read', 'write']) {
      await createPolicy(port, {
        id: `signal-own-${action}`,
        mapPattern: '{auth.tenant}.signal.{auth.id}',
        action,
        effect: 'allow',
      });
      await createPolicy(port, {
        id: `signal-monitor-${action}`,
        mapPattern: 'acme.signal.*',
        action,
        effect: 'allow',
        conditionExpr: "auth.id == 'monitor'",
      });
    }
  });

  afterAll(async () => {
    for (const client of open) client.close();
    await server.cleanup();
  });

  async function connect(sub: string, tenant: string): Promise<TestClient> {
    const client = await visitor(server.port, sub, tenant);
    open.push(client);
    return client;
  }

  test("a visitor cannot overwrite another visitor's key by forging ownerId", async () => {
    const v1 = await connect('v1', 'acme');
    const v2 = await connect('v2', 'acme');

    expect((await write(v1, 'acme.cart', 'v1', { ownerId: 'v1' })).type).toBe('OP_ACK');
    expectRefused(await write(v2, 'acme.cart', 'v1', { ownerId: 'v2' }));
    expect((await write(v2, 'acme.cart', 'v2', { ownerId: 'v2' })).type).toBe('OP_ACK');

    // The documented contrast: a condition on the value alone is satisfied by
    // the forged owner, so the same write is admitted there.
    expect((await write(v2, 'acme.legacy-cart', 'v1', { ownerId: 'v2' })).type).toBe('OP_ACK');
  });

  test('templated policies hold across tenants', async () => {
    const acmeV1 = await connect('v1', 'acme');
    const globexV1 = await connect('v1', 'globex');

    expect((await write(acmeV1, 'acme.visitor.v1.cart', 'item', { n: 1 })).type).toBe('OP_ACK');
    expect((await write(globexV1, 'globex.visitor.v1.cart', 'item', { n: 2 })).type).toBe('OP_ACK');
    expectRefused(await write(acmeV1, 'globex.visitor.v1.cart', 'item', { n: 3 }));
    expectRefused(await write(acmeV1, 'acme.visitor.v2.cart', 'item', { n: 4 }));

    const own = await query(acmeV1, 'acme.visitor.v1.cart');
    expect(own?.map((r) => r.key)).toEqual(['item']);
    expect(await query(acmeV1, 'acme.visitor.v2.cart')).toBeNull();
    expect(await query(acmeV1, 'globex.visitor.v1.cart')).toBeNull();
    const theirs = await query(globexV1, 'globex.visitor.v1.cart');
    expect(theirs?.map((r) => r.key)).toEqual(['item']);
  });

  test("a templated topic admits only the principal's own topic", async () => {
    const own = 'acme.signal.v1';
    const other = 'acme.signal.v2';
    const v1 = await connect('v1', 'acme');
    const v2 = await connect('v2', 'acme');
    const monitor = await connect('monitor', 'acme');

    for (const [client, topic] of [
      [monitor, own],
      [monitor, other],
      [v1, own],
      [v1, other],
      [v2, other],
    ] as [TestClient, string][]) {
      client.send({ type: 'TOPIC_SUB', payload: { topic } });
    }
    await waitForSync(SETTLE_MS);

    // v1 publishes to its own topic: the monitor receives it.
    v1.send({ type: 'TOPIC_PUB', payload: { topic: own, data: { from: 'v1', n: 1 } } });
    await waitUntil(() => topicMessages(monitor, own).length === 1, 5000);
    // v1 publishes to v2's topic: refused, nobody receives it.
    v1.send({ type: 'TOPIC_PUB', payload: { topic: other, data: { from: 'v1', n: 2 } } });
    // The monitor publishes to both topics: v1 receives its own topic only.
    monitor.send({ type: 'TOPIC_PUB', payload: { topic: own, data: { from: 'monitor' } } });
    monitor.send({ type: 'TOPIC_PUB', payload: { topic: other, data: { from: 'monitor' } } });
    await waitUntil(() => topicMessages(v1, own).length === 1, 5000);
    await waitUntil(() => topicMessages(v2, other).length === 1, 5000);
    await waitForSync(SETTLE_MS);

    expect(topicMessages(v1, own).map((m) => m.payload.data)).toEqual([{ from: 'monitor' }]);
    expect(topicMessages(v1, other)).toEqual([]);
    expect(topicMessages(v2, other).map((m) => m.payload.data)).toEqual([{ from: 'monitor' }]);
    expect(topicMessages(monitor, other)).toEqual([]);
    expect(topicMessages(monitor, own).map((m) => m.payload.data)).toEqual([{ from: 'v1', n: 1 }]);
  });
});
