/**
 * Integration: vector search on the server binary, from the TS client, across
 * a restart, a delete of a key that is not in memory, and expiry (TG-VEC-001).
 *
 * One server on its own directories, restartable on the same ones. "After a
 * restart" means a graceful stop that exits 0 and a start on the same
 * directories: nothing is resident then, so every hit is read from the store.
 *
 * - `vecs`, `evict`, `ttl`: a vector index on `embedding` (dimension 4,
 *   cosine) created through the admin API before any write. The record field
 *   holds the msgpack encoding of the server's `Vector` type, the map
 *   `{ type: 'f32', data: <little-endian f32 bytes> }`, as a `Uint8Array`.
 * - `docs`: auto-embedded by the deterministic provider (`TOPGUN_VECTOR_MAPS`),
 *   searched with hybrid `methods: ['semantic']` only.
 *
 * The expiry sweep is set to run far later than the test, so an expired record
 * is still stored and only the read paths keep it out (TG-TTL-001). Waits poll
 * with a ceiling; the only fixed waits are for a TTL to pass.
 */

import { vectorToBytes } from '@topgunbuild/core';
import { TopGunClient, SyncState } from '@topgunbuild/client';
import * as fs from 'fs';
import * as jwt from 'jsonwebtoken';
import { pack } from 'msgpackr';
import * as os from 'os';
import * as path from 'path';

import {
  createRustTestClient,
  createTestToken,
  MemoryStorageAdapter,
  spawnRustServer,
  SpawnedServer,
  TestClient,
  waitUntil,
} from './helpers';

const JWT_SECRET = 'test-e2e-secret';
const STOP_TIMEOUT_MS = 20_000;
const SIGNAL_MS = 15_000;
const DIM = 4;
const ADMIN_TOKEN = jwt.sign({ sub: 'operator', roles: ['admin'] }, JWT_SECRET, {
  expiresIn: '1h',
});
const VECTOR_MAPS = JSON.stringify({ docs: { fields: ['text'], dimension: 32 } });

type Vec4 = [number, number, number, number];

/** The record field the server indexes: `Vector::F32(values)` in msgpack. */
function vectorField(values: Vec4): Uint8Array {
  // `pack` returns a view into a buffer the next `pack` overwrites.
  return Uint8Array.from(pack({ type: 'f32', data: vectorToBytes(values) }));
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
  return { status: resp.status, json: text ? JSON.parse(text) : null };
}

async function createVectorIndex(port: number, mapName: string): Promise<void> {
  const created = await admin(port, 'POST', '/api/admin/indexes/vector', {
    mapName,
    attribute: 'embedding',
    indexName: `${mapName}_idx`,
    dimension: DIM,
    distanceMetric: 'cosine',
  });
  expect(created.status).toBe(201);
}

/** The committed vector count of the index on `mapName` (admin listing). */
async function vectorCount(port: number, mapName: string): Promise<number> {
  const listed = await admin(port, 'GET', '/api/admin/indexes/vector');
  expect(listed.status).toBe(200);
  const index = listed.json.indexes.find((i: any) => i.indexName === `${mapName}_idx`);
  expect(index).toBeDefined();
  return index.vectorCount;
}

/** Records of `mapName` held in memory (admin map listing). */
async function residentCount(port: number, mapName: string): Promise<number> {
  const maps = await admin(port, 'GET', '/api/admin/maps');
  expect(maps.status).toBe(200);
  return maps.json.maps.find((m: any) => m.name === mapName)?.entryCount ?? 0;
}

describe('Integration: vector search on the server binary (TG-VEC-001)', () => {
  let dir = '';
  let server: SpawnedServer | null = null;
  const clients: TopGunClient[] = [];
  const raws: TestClient[] = [];

  const port = (): number => server!.port;

  const start = async (): Promise<void> => {
    server = await spawnRustServer({
      env: {
        STORAGE_BACKEND: 'redb',
        TOPGUN_DATA_DIR: path.join(dir, 'topgun-data'),
        TOPGUN_REDB_PATH: path.join(dir, 'topgun.redb'),
        TOPGUN_WAL_DIR: path.join(dir, 'wal'),
        TOPGUN_WAL_FSYNC_POLICY: 'per_op',
        TOPGUN_INDEX_PATH: path.join(dir, 'scalar_indexes.json'),
        TOPGUN_VECTOR_INDEX_PATH: path.join(dir, 'vector_indexes.json'),
        TOPGUN_ADMIN_SUBJECTS: 'operator',
        TOPGUN_EXPIRY_SWEEP_INTERVAL_MS: '3600000',
        TOPGUN_EMBEDDING_PROVIDER: 'deterministic',
        TOPGUN_EMBEDDING_DIMENSION: '32',
        TOPGUN_VECTOR_MAPS: VECTOR_MAPS,
      },
    });
  };

  /** Graceful stop (exit 0) and a start on the same directories. */
  const restart = async (): Promise<void> => {
    for (const client of clients.splice(0)) await client.close();
    for (const raw of raws.splice(0)) raw.close();
    const stopping = server!;
    server = null;
    await stopping.cleanup();
    await waitUntil(
      () => stopping.process.exitCode !== null || stopping.process.signalCode !== null,
      STOP_TIMEOUT_MS,
    );
    expect(stopping.process.exitCode).toBe(0);
    await start();
  };

  /** A real TopGunClient on fresh in-memory storage. */
  const open = async (): Promise<TopGunClient> => {
    const token = createTestToken('vector-client', ['ADMIN']);
    const client = new TopGunClient({
      serverUrl: `ws://localhost:${port()}/ws`,
      storage: new MemoryStorageAdapter(),
      auth: { getToken: () => Promise.resolve(token) },
    });
    clients.push(client);
    await client.start();
    await waitUntil(() => client.getConnectionState() === SyncState.CONNECTED, SIGNAL_MS);
    return client;
  };

  const openRaw = async (): Promise<TestClient> => {
    const raw = await createRustTestClient(port(), { userId: 'vector-raw', roles: ['ADMIN'] });
    await raw.waitForMessage('AUTH_ACK', 10_000);
    raw.stopHeartbeat();
    raws.push(raw);
    return raw;
  };

  /** Sends `message` and resolves with the first later frame of one of `types`. */
  const ask = async (raw: TestClient, message: object, types: string[]): Promise<any> => {
    const from = raw.messages.length;
    raw.send(message);
    let reply: any;
    await waitUntil(() => {
      reply = raw.messages.slice(from).find((m) => types.includes(m.type));
      return reply !== undefined;
    }, SIGNAL_MS);
    return reply;
  };

  let seq = 0;
  const rawSearch = async (raw: TestClient, mapName: string, query: Vec4, k: number) => {
    const reply = await ask(
      raw,
      {
        type: 'VECTOR_SEARCH',
        payload: { id: `vs-${++seq}`, mapName, queryVector: vectorToBytes(query), k },
      },
      ['VECTOR_SEARCH_RESP'],
    );
    return reply.payload;
  };

  const rawRemove = async (raw: TestClient, mapName: string, key: string): Promise<void> => {
    const reply = await ask(
      raw,
      {
        type: 'OP_BATCH',
        payload: {
          ops: [
            {
              id: `rm-${++seq}`,
              mapName,
              opType: 'REMOVE',
              key,
              record: {
                value: null,
                timestamp: { millis: Date.now(), counter: 0, nodeId: 'vector-raw' },
              },
            },
          ],
        },
      },
      ['OP_ACK', 'OP_REJECTED', 'ERROR'],
    );
    expect(reply.type).toBe('OP_ACK');
  };

  const keysOf = (results: Array<{ key: string }>): string[] => results.map((r) => r.key);

  /** Polls `client.vectorSearch` until `ready` holds for its keys. */
  const searchUntil = async (
    client: TopGunClient,
    mapName: string,
    query: Vec4,
    k: number,
    ready: (keys: string[]) => boolean,
  ) => {
    let results: Awaited<ReturnType<TopGunClient['vectorSearch']>> = [];
    await waitUntil(async () => {
      results = await client.vectorSearch(mapName, query, { k });
      return ready(keysOf(results));
    }, SIGNAL_MS);
    return results;
  };

  const semantic = (client: TopGunClient, text: string) =>
    client.hybridSearch('docs', text, { methods: ['semantic'], k: 10, includeValue: true });

  beforeAll(async () => {
    dir = fs.mkdtempSync(path.join(os.tmpdir(), 'topgun-vector-restart-'));
    await start();
    for (const mapName of ['vecs', 'evict', 'ttl']) await createVectorIndex(port(), mapName);
  });

  afterAll(async () => {
    for (const client of clients.splice(0)) await client.close();
    for (const raw of raws.splice(0)) raw.close();
    if (server) await server.cleanup();
    if (dir) fs.rmSync(dir, { recursive: true, force: true });
  });

  const V1: Vec4 = [1, 0, 0, 0];

  test('AC10: vector search returns a written record', async () => {
    const client = await open();
    const map = client.getMap<string, unknown>('vecs');
    map.set('v1', { id: 'v1', embedding: vectorField(V1) });
    map.set('d1', { id: 'd1', embedding: vectorField([0, 1, 0, 0]) });
    map.set('d2', { id: 'd2', embedding: vectorField([0, 0, 1, 0]) });

    const results = await searchUntil(client, 'vecs', V1, 3, (keys) => keys.length === 3);
    expect(keysOf(results)[0]).toBe('v1');
    expect((results[0].value as any).id).toBe('v1');
  });

  test('AC11: after a restart vector search still returns it', async () => {
    await restart();
    const client = await open();
    const results = await client.vectorSearch('vecs', V1, { k: 3 });
    expect(keysOf(results)[0]).toBe('v1');
    expect(keysOf(results).sort()).toEqual(['d1', 'd2', 'v1']);
  });

  test('AC12: evict + delete of a non-resident key is not returned', async () => {
    const N = 6;
    const distinct: Array<[string, Vec4]> = Array.from({ length: N }, (_, i) => {
      const angle = (i * Math.PI) / (2 * N);
      return [`e${i}`, [Math.cos(angle), Math.sin(angle), 0, 0]];
    });
    const SHARED: Vec4 = [0, 0, 1, 1];
    const writer = await open();
    const map = writer.getMap<string, unknown>('evict');
    for (const [key, vector] of distinct) map.set(key, { id: key, embedding: vectorField(vector) });
    for (const key of ['s1', 's2']) map.set(key, { id: key, embedding: vectorField(SHARED) });
    const docs = writer.getMap<string, unknown>('docs');
    docs.set('doc1', { text: 'quiet harbor lighthouse at dawn' });
    docs.set('doc2', { text: 'spicy noodle soup recipe' });
    await searchUntil(writer, 'evict', [1, 0, 0, 0], N + 2, (keys) => keys.length === N + 2);
    // The client sends an operation again with every sync pass until it is
    // acknowledged, and each repeat replaces the stored value, `_embedding`
    // included, and queues another embedding. Only once nothing is pending
    // does a stored `_embedding` stay, and only a stored one survives the stop.
    await waitUntil(() => writer.getPendingOpsCount() === 0, SIGNAL_MS);
    await waitUntil(async () => {
      const stored = await semantic(writer, 'quiet harbor lighthouse at dawn');
      return (
        stored.length === 2 &&
        stored.every((hit) => (hit.value as Record<string, unknown>)?._embedding !== undefined)
      );
    }, SIGNAL_MS);

    await restart();
    // Nothing of either map is in memory, and the boot rebuild committed
    // every stored key, the two that share a vector included.
    expect(await residentCount(port(), 'evict')).toBe(0);
    expect(await residentCount(port(), 'docs')).toBe(0);
    expect(await vectorCount(port(), 'evict')).toBe(N + 2);

    // The delete comes before any read of the key, so it reaches the index
    // as the delete of a key that is not resident.
    const raw = await openRaw();
    const [deleted, deletedVector] = distinct[2];
    await rawRemove(raw, 'evict', deleted);

    const resp = await rawSearch(raw, 'evict', deletedVector, N + 2);
    expect(resp.error ?? null).toBeNull();
    expect(keysOf(resp.results)).not.toContain(deleted);
    expect(resp.totalCandidates).toBe(N + 1);
    // Every other key was not resident before this search: its value was read from the store.
    expect(resp.results).toHaveLength(N + 1);
    for (const r of resp.results) expect(r.value).toEqual(expect.objectContaining({ id: r.key }));

    const client = await open();
    const shared = await client.vectorSearch('evict', SHARED, { k: 2 });
    expect(keysOf(shared).sort()).toEqual(['s1', 's2']);
    expect(shared[0].score).toBeCloseTo(1, 5);
    expect(shared[1].score).toBeCloseTo(1, 5);

    const hits = await semantic(client, 'quiet harbor lighthouse at dawn');
    expect(keysOf(hits)[0]).toBe('doc1');
    // The stored value: the written text, plus the `_embedding` the server wrote back.
    expect(hits[0].value).toEqual(
      expect.objectContaining({ text: 'quiet harbor lighthouse at dawn' }),
    );
  });

  test('AC13: an expired record is not returned', async () => {
    const TTL_MS = 3_000;
    const V_TTL: Vec4 = [0, 0, 0, 1];
    const client = await open();
    const map = client.getMap<string, unknown>('ttl');
    const writtenAt = Date.now();
    map.set('v_ttl', { id: 'v_ttl', embedding: vectorField(V_TTL) }, TTL_MS);
    map.set('live', { id: 'live', embedding: vectorField([0, 0, 1, 1]) });
    const docs = client.getMap<string, unknown>('docs');
    const TTL_TEXT = 'violet glacier telescope';
    docs.set('doc_ttl', { text: TTL_TEXT }, TTL_MS);

    // Both are found before they expire, so the checks below are not vacuous.
    await searchUntil(client, 'ttl', V_TTL, 2, (keys) => keys[0] === 'v_ttl' && keys.length === 2);
    await waitUntil(
      async () => keysOf(await semantic(client, TTL_TEXT)).includes('doc_ttl'),
      SIGNAL_MS,
    );

    const waitMs = writtenAt + TTL_MS + 500 - Date.now();
    if (waitMs > 0) await new Promise((resolve) => setTimeout(resolve, waitMs));

    const expectGone = async (reader: TopGunClient) => {
      const results = await reader.vectorSearch('ttl', V_TTL, { k: 2 });
      expect(keysOf(results)).toEqual(['live']);
      expect(keysOf(await semantic(reader, TTL_TEXT))).not.toContain('doc_ttl');
    };
    await expectGone(client);

    // Expired at boot: the rebuild does not index it, and it is not resident.
    await restart();
    expect(await residentCount(port(), 'ttl')).toBe(0);
    await expectGone(await open());
  });

  test('AC14: the server answers instead of timing out', async () => {
    const client = await open();
    const began = Date.now();
    await expect(client.vectorSearch('vecs', V1, { k: 1 })).resolves.toHaveLength(1);
    // A map without a vector index is answered with an error, not left unanswered.
    await expect(client.vectorSearch('no-index', V1, { k: 1 })).rejects.toThrow(/index/);
    expect(Date.now() - began).toBeLessThan(5_000);
  });
});
