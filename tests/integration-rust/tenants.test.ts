/**
 * Tenant boundary over the wire (TG-TEN-001).
 *
 * One server with `TOPGUN_TENANT_CLAIM=org_id`: every client token carries the
 * claim, and a client of tenant `a` reaches only maps named `a.…`. Tenant `b`
 * owns `b.orders` (LWW) and `b.docs` (OR). Checked here, end to end:
 *
 * - a write by A to `b.orders` is refused per operation, on WebSocket and on
 *   HTTP `/sync`, while its write to `a.orders` is acknowledged;
 * - a read by A of `b.orders` over HTTP `/sync` is a 403 entry with no rows,
 *   while B reads its own rows;
 * - a query subscription by A to `b.orders` is not served and delivers none of
 *   B's later writes;
 * - an OR push-diff by B to `b.docs` reaches no connection of A.
 *
 * Read refusals on WebSocket send no frame, and the server dispatches a
 * connection's messages concurrently, so "nothing arrived" is decided at a
 * barrier: B first reads its own record back, then A sends `PING`; any frame
 * the server had queued for A about B's write is ahead of A's `PONG` on the
 * same outbound channel.
 */

import { serialize, deserialize } from '@topgunbuild/core';
import * as jwt from 'jsonwebtoken';
import {
  spawnRustServer,
  createRustTestClient,
  createLWWRecord,
  createORRecord,
  waitForSync,
  waitUntil,
  SpawnedServer,
  TestClient,
} from './helpers';

const JWT_SECRET = 'test-e2e-secret';
const TENANT_CLAIM = 'org_id';

function tenantToken(userId: string, tenant: string): string {
  return jwt.sign({ sub: userId, roles: ['USER'], [TENANT_CLAIM]: tenant }, JWT_SECRET, {
    expiresIn: '1h',
  });
}

/** A WebSocket client that authenticates with a token of `tenant`. */
async function tenantClient(port: number, userId: string, tenant: string): Promise<TestClient> {
  const client = await createRustTestClient(port, { userId, autoAuth: false });
  await client.waitForMessage('AUTH_REQUIRED', 8000);
  client.send({ type: 'AUTH', token: tenantToken(userId, tenant) });
  await client.waitForMessage('AUTH_ACK', 8000);
  return client;
}

/** Sends one OP_BATCH write and resolves with OP_ACK, OP_REJECTED or ERROR. */
async function write(
  client: TestClient,
  mapName: string,
  key: string,
  value: unknown,
): Promise<{ type: string; payload?: any }> {
  const opId = `op-${mapName}-${key}-${Math.random().toString(36).slice(2)}`;
  const from = client.messages.length;
  client.send({
    type: 'OP_BATCH',
    payload: {
      ops: [{ id: opId, mapName, opType: 'PUT', key, record: createLWWRecord(value) }],
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
  expect(reply.payload?.permanent).toBe(true);
}

/** POST /sync with `token`; returns the decoded response body. */
async function postSync(port: number, token: string | null, body: object): Promise<any> {
  const headers: Record<string, string> = { 'Content-Type': 'application/msgpack' };
  if (token) headers.Authorization = `Bearer ${token}`;
  const resp = await fetch(`http://127.0.0.1:${port}/sync`, {
    method: 'POST',
    headers,
    body: Buffer.from(
      serialize({
        clientId: 'tenants-http',
        clientHlc: { millis: Date.now(), counter: 0, nodeId: 'tenants-http' },
        ...body,
      }),
    ),
  });
  return {
    status: resp.status,
    body: resp.ok ? deserialize(new Uint8Array(await resp.arrayBuffer())) : null,
  };
}

/** Whether any decoded frame mentions `name` anywhere in its body. */
function names(message: unknown, name: string): boolean {
  return JSON.stringify(message).includes(name);
}

/** Sends `PING` and waits for its `PONG`; returns the frames that came before it. */
async function framesBeforePong(client: TestClient, from: number): Promise<any[]> {
  const pingFrom = client.messages.length;
  client.send({ type: 'PING', timestamp: Date.now() });
  let pong = -1;
  await waitUntil(() => {
    pong = client.messages.findIndex((m, i) => i >= pingFrom && m.type === 'PONG');
    return pong >= 0;
  }, 5000);
  return client.messages.slice(from, pong);
}

describe('Integration: tenant boundary (TOPGUN_TENANT_CLAIM)', () => {
  let server: SpawnedServer;
  let a: TestClient;
  let b: TestClient;

  beforeAll(async () => {
    server = await spawnRustServer({ env: { TOPGUN_TENANT_CLAIM: TENANT_CLAIM } });
    a = await tenantClient(server.port, 'user-a', 'a');
    b = await tenantClient(server.port, 'user-b', 'b');
    expect((await write(b, 'b.orders', 'o1', { item: 'theirs' })).type).toBe('OP_ACK');
  });

  afterAll(async () => {
    a?.close();
    b?.close();
    await server.cleanup();
  });

  test('a write to another tenant map is refused on WebSocket; one to its own is acknowledged', async () => {
    expectRefused(await write(a, 'b.orders', 'x1', { item: 'intrusion' }));
    expect((await write(a, 'a.orders', 'x1', { item: 'mine' })).type).toBe('OP_ACK');
  });

  test('HTTP /sync refuses another tenant map per operation and per query', async () => {
    const token = tenantToken('user-a', 'a');
    const ops = await postSync(server.port, token, {
      operations: [
        { id: 'h1', mapName: 'b.orders', opType: 'PUT', key: 'h1', record: createLWWRecord('x') },
        { id: 'h2', mapName: 'a.orders', opType: 'PUT', key: 'h2', record: createLWWRecord('y') },
      ],
    });
    expect(ops.status).toBe(200);
    const refused = (ops.body.errors ?? []).filter((e: any) => e.code === 403);
    expect(refused.map((e: any) => e.context)).toEqual(['h1']);

    const reads = await postSync(server.port, token, {
      queries: [{ queryId: 'qb', mapName: 'b.orders', filter: null }],
    });
    expect(reads.status).toBe(200);
    expect((reads.body.errors ?? []).map((e: any) => [e.code, e.context])).toEqual([[403, 'qb']]);
    expect((reads.body.queryResults ?? []).filter((r: any) => r.queryId === 'qb')).toEqual([]);

    // The rows exist: their tenant reads them.
    const own = await postSync(server.port, tenantToken('user-b', 'b'), {
      queries: [{ queryId: 'qb', mapName: 'b.orders', filter: null }],
    });
    expect(own.body.errors ?? []).toEqual([]);
    expect(own.body.queryResults.find((r: any) => r.queryId === 'qb').results).toHaveLength(1);
  });

  test("a subscription to another tenant map and B's OR push-diff reach none of A's frames", async () => {
    const from = a.messages.length;
    a.send({
      type: 'QUERY_SUB',
      payload: { queryId: 'a-peeks-orders', mapName: 'b.orders', query: {} },
    });
    a.send({
      type: 'QUERY_SUB',
      payload: { queryId: 'a-peeks-docs', mapName: 'b.docs', query: {} },
    });
    // Its own subscription is served, so A's subscriptions were dispatched.
    a.send({ type: 'QUERY_SUB', payload: { queryId: 'a-own', mapName: 'a.orders', query: {} } });
    await waitUntil(
      () => a.messages.some((m) => m.type === 'QUERY_RESP' && m.payload?.queryId === 'a-own'),
      5000,
    );
    await waitForSync(200);

    expect((await write(b, 'b.orders', 'o2', { item: 'later' })).type).toBe('OP_ACK');
    const orRecord = createORRecord('pushed', 'tenant-b');
    b.send({
      type: 'ORMAP_PUSH_DIFF',
      payload: {
        mapName: 'b.docs',
        entries: [{ key: 'd1', records: [orRecord], tombstones: [] }],
      },
    });

    // Barrier: B reads its pushed record back ...
    await waitUntil(async () => {
      const readFrom = b.messages.length;
      b.send({ type: 'ORMAP_DIFF_REQUEST', payload: { mapName: 'b.docs', keys: ['d1'] } });
      await waitUntil(
        () => b.messages.slice(readFrom).some((m) => m.type === 'ORMAP_DIFF_RESPONSE'),
        5000,
      );
      const response = b.messages.slice(readFrom).find((m) => m.type === 'ORMAP_DIFF_RESPONSE');
      return (response.payload?.entries ?? []).some((e: any) =>
        e.records?.some((r: any) => r.tag === orRecord.tag),
      );
    }, 5000);

    // ... then A pings: nothing about tenant b's maps came before the PONG.
    const frames = await framesBeforePong(a, from);
    expect(frames.filter((m) => names(m, 'b.orders') || names(m, 'b.docs'))).toEqual([]);
    expect(frames.some((m) => m.type === 'QUERY_RESP' && m.payload?.queryId === 'a-own')).toBe(
      true,
    );
  });
});
