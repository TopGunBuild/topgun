/**
 * Tenant deletion over the wire (TG-TEN-002, `DELETE /api/admin/tenants/{tenant}`).
 *
 * One server with `TOPGUN_TENANT_CLAIM=org_id`, the embedded store and its own
 * data, WAL and index-descriptor paths, kept across a restart. RBAC is
 * configured with one `**` allow policy, as a tenant delete requires. Tenant
 * `b` owns `b.x` (one record) and `b.never` (an index, never a record); tenant
 * `c` owns `c.keep`, the control. Each of the three maps has a scalar and a
 * vector index.
 *
 * After the delete, in the same process: a client of `b` has a write, a read
 * (HTTP `/sync`) and a subscription on `b.x` refused, and the admin map listing
 * omits `b.x`. Then the `tenant-deleted:b` policy is deleted — the documented
 * re-onboarding path — and the server restarts: a client of `b` gets no record
 * of `b.x` from query, search, hybrid, vector and HTTP query, and the index
 * descriptors of `b.x` and `b.never` are gone (the latter was never in a drop's
 * `dropped`), while `c.keep` keeps its record and its indexes. Of the five read
 * paths only the query serves a record stored before a restart on this binary
 * (the control on `c.keep` shows it), so it is the one that tells a dropped map
 * from a kept one.
 */

import { serialize, deserialize } from '@topgunbuild/core';
import * as fs from 'fs';
import * as jwt from 'jsonwebtoken';
import { pack } from 'msgpackr';
import * as os from 'os';
import * as path from 'path';

import {
  spawnRustServer,
  createRustTestClient,
  createLWWRecord,
  waitForSync,
  waitUntil,
  SpawnedServer,
  TestClient,
} from './helpers';

const JWT_SECRET = 'test-e2e-secret';
const TENANT_CLAIM = 'org_id';
const STOP_TIMEOUT_MS = 20_000;
const ADMIN_TOKEN = jwt.sign({ sub: 'operator', roles: ['admin'] }, JWT_SECRET, {
  expiresIn: '1h',
});

const RECORD = { title: 'quarterly report', body: 'revenue grew in every region' };

function tenantToken(userId: string, tenant: string): string {
  return jwt.sign({ sub: userId, roles: ['USER'], [TENANT_CLAIM]: tenant }, JWT_SECRET, {
    expiresIn: '1h',
  });
}

async function tenantClient(port: number, userId: string, tenant: string): Promise<TestClient> {
  const client = await createRustTestClient(port, { userId, autoAuth: false });
  await client.waitForMessage('AUTH_REQUIRED', 8000);
  client.send({ type: 'AUTH', token: tenantToken(userId, tenant) });
  await client.waitForMessage('AUTH_ACK', 8000);
  client.stopHeartbeat();
  return client;
}

async function admin(
  port: number,
  method: string,
  route: string,
  body?: unknown,
): Promise<{ status: number; json: any }> {
  const resp = await fetch(`http://127.0.0.1:${port}${route}`, {
    method,
    headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${ADMIN_TOKEN}` },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await resp.text();
  let json: any = null;
  try {
    json = text ? JSON.parse(text) : null;
  } catch {
    json = text;
  }
  return { status: resp.status, json };
}

/** Sends `message` and resolves with the first later frame of one of `types`. */
async function ask(client: TestClient, message: object, types: string[]): Promise<any> {
  const from = client.messages.length;
  client.send(message);
  let reply: any;
  await waitUntil(() => {
    reply = client.messages.slice(from).find((m) => types.includes(m.type));
    return reply !== undefined;
  }, 5000);
  return reply;
}

async function write(client: TestClient, mapName: string, key: string): Promise<any> {
  return ask(
    client,
    {
      type: 'OP_BATCH',
      payload: {
        ops: [
          {
            id: `op-${mapName}-${key}-${Math.random().toString(36).slice(2)}`,
            mapName,
            opType: 'PUT',
            key,
            record: createLWWRecord(RECORD),
          },
        ],
      },
    },
    ['OP_ACK', 'OP_REJECTED', 'ERROR'],
  );
}

async function httpQuery(port: number, tenant: string, mapName: string): Promise<any> {
  const resp = await fetch(`http://127.0.0.1:${port}/sync`, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/msgpack',
      Authorization: `Bearer ${tenantToken(`http-${tenant}`, tenant)}`,
    },
    body: Buffer.from(
      serialize({
        clientId: 'tenant-delete-http',
        clientHlc: { millis: Date.now(), counter: 0, nodeId: 'tenant-delete-http' },
        queries: [{ queryId: 'q', mapName, filter: null }],
      }),
    ),
  });
  expect(resp.status).toBe(200);
  return deserialize(new Uint8Array(await resp.arrayBuffer()));
}

function httpRows(body: any): unknown[] {
  return (body.queryResults ?? []).find((r: any) => r.queryId === 'q')?.results ?? [];
}

/** Sends `PING` and returns the frames that arrived from `from` up to its `PONG`. */
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

/**
 * A query vector's bytes. Sent with `pack` directly: the shared `serialize`
 * turns a `Uint8Array` into a map, which the server cannot decode.
 */
function vectorBytes(): Uint8Array {
  return new Uint8Array(new Float32Array([0.1, 0.2, 0.3, 0.4]).buffer);
}

/** Every key any read path returns for `mapName` to a client of `tenant`. */
async function keysFromEveryReadPath(
  port: number,
  client: TestClient,
  tenant: string,
  mapName: string,
): Promise<Record<string, unknown>> {
  const query = await ask(
    client,
    { type: 'QUERY_SUB', payload: { queryId: `q-${mapName}`, mapName, query: {} } },
    ['QUERY_RESP'],
  );
  const search = await ask(
    client,
    { type: 'SEARCH', payload: { requestId: `s-${mapName}`, mapName, query: 'quarterly report' } },
    ['SEARCH_RESP'],
  );
  const hybrid = await ask(
    client,
    {
      type: 'HYBRID_SEARCH',
      payload: {
        requestId: `h-${mapName}`,
        mapName,
        queryText: 'quarterly report',
        methods: ['fullText'],
        k: 10,
      },
    },
    ['HYBRID_SEARCH_RESP'],
  );
  // The binary answers no VECTOR_SEARCH today (its query service has no index
  // registry), so an unanswered search counts as no record; a frame that does
  // come must hold none of the map's keys.
  const from = client.messages.length;
  client.ws.send(
    pack({
      type: 'VECTOR_SEARCH',
      payload: { id: `v-${mapName}`, mapName, queryVector: vectorBytes(), k: 10 },
    }),
  );
  await waitForSync(1000);
  const vector = client.messages.slice(from).find((m) => m.type === 'VECTOR_SEARCH_RESP');
  const http = await httpQuery(port, tenant, mapName);
  const keys = (rows: any[] | undefined) => (rows ?? []).map((r: any) => r.key);
  return {
    query: keys(query.payload?.results),
    search: keys(search.payload?.results),
    hybrid: keys(hybrid.payload?.results),
    vector: keys(vector?.payload?.results),
    http: keys(httpRows(http) as any[]),
  };
}

const NO_KEYS = { query: [], search: [], hybrid: [], vector: [], http: [] };

/** The maps the scalar and vector descriptor files name, and the scalar indexes served. */
async function indexes(
  port: number,
  dir: string,
): Promise<{ scalarFile: string[]; vectorFile: string[]; scalarServed: string[] }> {
  const mapsIn = (file: string) =>
    (JSON.parse(fs.readFileSync(path.join(dir, file), 'utf8')) as any[])
      .map((d) => d.mapName)
      .sort();
  const served = await admin(port, 'GET', '/api/admin/indexes');
  expect(served.status).toBe(200);
  return {
    scalarFile: mapsIn('scalar_indexes.json'),
    vectorFile: mapsIn('vector_indexes.json'),
    scalarServed: served.json.indexes
      .filter((i: any) => i.indexType !== 'vector')
      .map((i: any) => i.mapName)
      .sort(),
  };
}

describe('Integration: tenant deletion (DELETE /api/admin/tenants/{tenant})', () => {
  let dir: string;
  let server: SpawnedServer | null = null;
  const clients: TestClient[] = [];

  function start(): Promise<SpawnedServer> {
    return spawnRustServer({
      env: {
        STORAGE_BACKEND: 'redb',
        TOPGUN_TENANT_CLAIM: TENANT_CLAIM,
        TOPGUN_DATA_DIR: path.join(dir, 'topgun-data'),
        TOPGUN_REDB_PATH: path.join(dir, 'topgun.redb'),
        TOPGUN_WAL_DIR: path.join(dir, 'wal'),
        TOPGUN_WAL_FSYNC_POLICY: 'per_op',
        TOPGUN_INDEX_PATH: path.join(dir, 'scalar_indexes.json'),
        TOPGUN_VECTOR_INDEX_PATH: path.join(dir, 'vector_indexes.json'),
      },
    });
  }

  async function stop(): Promise<void> {
    for (const client of clients.splice(0)) client.close();
    if (!server) return;
    const stopping = server;
    server = null;
    await stopping.cleanup();
    await waitUntil(
      () => stopping.process.exitCode !== null || stopping.process.signalCode !== null,
      STOP_TIMEOUT_MS,
    );
  }

  async function connect(userId: string, tenant: string): Promise<TestClient> {
    const client = await tenantClient(server!.port, userId, tenant);
    clients.push(client);
    return client;
  }

  beforeAll(async () => {
    dir = fs.mkdtempSync(path.join(os.tmpdir(), 'topgun-tenant-delete-'));
    server = await start();
    const port = server.port;
    const allow = await admin(port, 'POST', '/api/admin/policies', {
      id: 'allow-all',
      mapPattern: '**',
      action: 'all',
      effect: 'allow',
    });
    expect(allow.status).toBe(201);
    for (const mapName of ['b.x', 'b.never', 'c.keep']) {
      const scalar = await admin(port, 'POST', '/api/admin/indexes', {
        mapName,
        attribute: 'title',
        indexType: 'hash',
      });
      expect(scalar.status).toBe(201);
      const vector = await admin(port, 'POST', '/api/admin/indexes/vector', {
        mapName,
        attribute: '_embedding',
        indexName: `${mapName.replace('.', '_')}_emb`,
        dimension: 4,
        distanceMetric: 'cosine',
      });
      expect(vector.status).toBe(201);
    }
    const b = await connect('user-b', 'b');
    const c = await connect('user-c', 'c');
    expect((await write(b, 'b.x', 'k1')).type).toBe('OP_ACK');
    expect((await write(c, 'c.keep', 'k1')).type).toBe('OP_ACK');
    // The record of b.x is served before the delete.
    expect(httpRows(await httpQuery(port, 'b', 'b.x'))).toHaveLength(1);
  });

  afterAll(async () => {
    await stop();
    if (dir) fs.rmSync(dir, { recursive: true, force: true });
  });

  test('the route refuses an encoded slash before writing anything; a raw slash is not routed', async () => {
    const port = server!.port;
    expect((await admin(port, 'DELETE', '/api/admin/tenants/a%2Fb')).status).toBe(400);
    expect((await admin(port, 'DELETE', '/api/admin/tenants/a/b')).status).toBe(404);
    const policies = await admin(port, 'GET', '/api/admin/policies');
    expect(policies.json.policies.map((p: any) => p.id)).toEqual(['allow-all']);
  });

  test('after the delete, a client of b is refused a write, a read and a subscription on b.x', async () => {
    const port = server!.port;
    const deleted = await admin(port, 'DELETE', '/api/admin/tenants/b');
    expect(deleted.status).toBe(200);
    expect(deleted.json.dropped).toEqual(['b.x']);
    expect(deleted.json.failed).toEqual([]);

    const b = await connect('user-b2', 'b');
    const refused = await write(b, 'b.x', 'k2');
    expect(refused.type).toBe('OP_REJECTED');
    expect(refused.payload?.code).toBe(403);

    const read = await httpQuery(port, 'b', 'b.x');
    expect((read.errors ?? []).map((e: any) => [e.code, e.context])).toEqual([[403, 'q']]);
    expect(httpRows(read)).toEqual([]);

    const from = b.messages.length;
    b.send({ type: 'QUERY_SUB', payload: { queryId: 'b-sub', mapName: 'b.x', query: {} } });
    await waitForSync(200);
    const frames = await framesBeforePong(b, from);
    expect(frames.filter((m) => JSON.stringify(m).includes('b-sub'))).toEqual([]);

    const maps = await admin(port, 'GET', '/api/admin/maps');
    expect(maps.status).toBe(200);
    const names = maps.json.maps.map((m: any) => m.name);
    expect(names).toContain('c.keep');
    expect(names.filter((n: string) => n.startsWith('b.'))).toEqual([]);
  });

  test('after the policy is deleted and a restart, nothing of b.x is served and its indexes are gone', async () => {
    const policy = await admin(server!.port, 'DELETE', '/api/admin/policies/tenant-deleted:b');
    expect(policy.status).toBe(204);
    await stop();
    server = await start();
    const port = server.port;

    const b = await connect('user-b3', 'b');
    expect(await keysFromEveryReadPath(port, b, 'b', 'b.x')).toEqual(NO_KEYS);

    // The control: the restart serves the other tenant's record through a
    // query, and its indexes. Search, hybrid, HTTP query and vector search
    // serve no record stored before a restart for any map on this binary, so
    // for them the empty answers above hold with or without the delete; only
    // the query discriminates.
    const c = await connect('user-c2', 'c');
    const control = await keysFromEveryReadPath(port, c, 'c', 'c.keep');
    process.stderr.write(`[tenant-delete] c.keep after the restart: ${JSON.stringify(control)}\n`);
    expect(control.query).toEqual(['k1']);
    expect(await indexes(port, dir)).toEqual({
      scalarFile: ['c.keep'],
      vectorFile: ['c.keep'],
      scalarServed: ['c.keep'],
    });

    // b can write b.x again: re-onboarded after the restart.
    expect((await write(b, 'b.x', 'k3')).type).toBe('OP_ACK');
  });
});
