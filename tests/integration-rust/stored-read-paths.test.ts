/**
 * Whole-map read paths see stored records, not only resident ones (TG-READ-001).
 *
 * A server on the embedded store with its own data, WAL and index-descriptor
 * paths. "After a restart" means a graceful stop that exits 0, a start on the
 * same directories, and no read of the map by any path between the start and
 * the read under test: nothing is resident then, so every record a read path
 * returns comes from the store.
 *
 * Covered: HTTP `/sync`, full-text `SEARCH`, hybrid `exact` (with a scalar
 * index on the predicate attribute, which must not be trusted as complete),
 * hybrid `fullText`, hybrid `includeValue`, deletes before and after a
 * restart, and the same reads after eviction has thinned the resident set
 * (`TOPGUN_MAX_RAM_MB=1`, witnessed by the admin map listing's resident count).
 * Under the same eviction, the scalar index keeps one entry per stored record,
 * also after evicted keys are rewritten; after a restart, a scalar index
 * restored at start and one created with `create_index` hold every stored
 * record (TG-IDX-001).
 */

import { serialize, deserialize } from '@topgunbuild/core';
import * as fs from 'fs';
import * as jwt from 'jsonwebtoken';
import * as os from 'os';
import * as path from 'path';

import {
  spawnRustServer,
  createRustTestClient,
  createLWWRecord,
  waitUntil,
  SpawnedServer,
  TestClient,
} from './helpers';

const JWT_SECRET = 'test-e2e-secret';
const STOP_TIMEOUT_MS = 20_000;
const MAP = 'm';
const ADMIN_TOKEN = jwt.sign({ sub: 'operator', roles: ['admin'] }, JWT_SECRET, {
  expiresIn: '1h',
});
const READER_TOKEN = jwt.sign({ sub: 'http-reader', roles: ['ADMIN'] }, JWT_SECRET, {
  expiresIn: '1h',
});

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
async function ask(
  client: TestClient,
  message: object,
  types: string[],
  timeoutMs = 10_000,
): Promise<any> {
  const from = client.messages.length;
  client.send(message);
  let reply: any;
  await waitUntil(() => {
    reply = client.messages.slice(from).find((m) => types.includes(m.type));
    return reply !== undefined;
  }, timeoutMs);
  return reply;
}

let opSeq = 0;

async function writeMany(
  client: TestClient,
  records: Array<{ key: string; value: unknown }>,
): Promise<void> {
  const ops = records.map(({ key, value }) => ({
    id: `op-${++opSeq}`,
    mapName: MAP,
    opType: 'PUT',
    key,
    record: createLWWRecord(value),
  }));
  const reply = await ask(client, { type: 'OP_BATCH', payload: { ops } }, [
    'OP_ACK',
    'OP_REJECTED',
    'ERROR',
  ]);
  expect(reply.type).toBe('OP_ACK');
}

async function remove(client: TestClient, key: string): Promise<void> {
  const reply = await ask(
    client,
    {
      type: 'OP_BATCH',
      payload: {
        ops: [
          {
            id: `op-${++opSeq}`,
            mapName: MAP,
            opType: 'REMOVE',
            key,
            record: {
              value: null,
              timestamp: { millis: Date.now(), counter: 0, nodeId: 'stored-read-paths' },
            },
          },
        ],
      },
    },
    ['OP_ACK', 'OP_REJECTED', 'ERROR'],
  );
  expect(reply.type).toBe('OP_ACK');
}

/**
 * Every value `/sync` returns for `MAP` with `filter`, following `nextCursor`.
 * `/sync` returns values without their keys, so each written value carries its
 * key as `id`.
 */
async function syncRows(port: number, filter: unknown = null): Promise<any[]> {
  const rows: any[] = [];
  let cursor: string | undefined;
  for (let page = 0; page < 100; page++) {
    const resp = await fetch(`http://127.0.0.1:${port}/sync`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/msgpack', Authorization: `Bearer ${READER_TOKEN}` },
      body: Buffer.from(
        serialize({
          clientId: 'stored-read-paths-http',
          clientHlc: { millis: Date.now(), counter: 0, nodeId: 'stored-read-paths-http' },
          queries: [{ queryId: 'q', mapName: MAP, filter, ...(cursor ? { cursor } : {}) }],
        }),
      ),
    });
    expect(resp.status).toBe(200);
    const body: any = deserialize(new Uint8Array(await resp.arrayBuffer()));
    expect(body.errors ?? []).toEqual([]);
    const result = (body.queryResults ?? []).find((r: any) => r.queryId === 'q');
    rows.push(...(result?.results ?? []));
    cursor = result?.nextCursor;
    if (!result?.hasMore || !cursor) break;
  }
  return rows;
}

async function syncKeys(port: number, filter: unknown = null): Promise<string[]> {
  return (await syncRows(port, filter)).map((r: any) => r.id).sort();
}

async function search(client: TestClient, query: string, limit = 100): Promise<string[]> {
  const reply = await ask(
    client,
    {
      type: 'SEARCH',
      payload: { requestId: `s-${++opSeq}`, mapName: MAP, query, options: { limit } },
    },
    ['SEARCH_RESP'],
  );
  expect(reply.payload.error ?? null).toBeNull();
  return (reply.payload.results ?? []).map((r: any) => r.key).sort();
}

async function hybrid(client: TestClient, payload: Record<string, unknown>): Promise<any[]> {
  const reply = await ask(
    client,
    {
      type: 'HYBRID_SEARCH',
      payload: { requestId: `h-${++opSeq}`, mapName: MAP, queryText: '', k: 100, ...payload },
    },
    ['HYBRID_SEARCH_RESP'],
  );
  expect(reply.payload.error ?? null).toBeNull();
  return reply.payload.results ?? [];
}

const STATUS_OPEN = { op: 'eq', attribute: 'status', value: 'open' };

/**
 * Keys only: values are checked once, in AC3. With values, an answer above the
 * server's 512 KiB WebSocket write buffer closes the connection.
 */
async function exactKeys(client: TestClient, k = 100): Promise<string[]> {
  const results = await hybrid(client, {
    methods: ['exact'],
    predicate: STATUS_OPEN,
    k,
    includeValue: false,
  });
  return results.map((r: any) => r.key).sort();
}

async function residentCount(port: number): Promise<number> {
  const maps = await admin(port, 'GET', '/api/admin/maps');
  expect(maps.status).toBe(200);
  return maps.json.maps.find((m: any) => m.name === MAP)?.entryCount ?? 0;
}

/** One server on its own directories, restartable on the same ones. */
function storedServer(extraEnv: Record<string, string> = {}) {
  const ctx: { dir: string; server: SpawnedServer | null; clients: TestClient[] } = {
    dir: '',
    server: null,
    clients: [],
  };

  const start = async (): Promise<void> => {
    ctx.server = await spawnRustServer({
      env: {
        STORAGE_BACKEND: 'redb',
        TOPGUN_DATA_DIR: path.join(ctx.dir, 'topgun-data'),
        TOPGUN_REDB_PATH: path.join(ctx.dir, 'topgun.redb'),
        TOPGUN_WAL_DIR: path.join(ctx.dir, 'wal'),
        TOPGUN_WAL_FSYNC_POLICY: 'per_op',
        TOPGUN_INDEX_PATH: path.join(ctx.dir, 'scalar_indexes.json'),
        TOPGUN_VECTOR_INDEX_PATH: path.join(ctx.dir, 'vector_indexes.json'),
        TOPGUN_ADMIN_SUBJECTS: 'operator',
        ...extraEnv,
      },
    });
  };

  const stop = async (): Promise<void> => {
    for (const client of ctx.clients.splice(0)) client.close();
    if (!ctx.server) return;
    const stopping = ctx.server;
    ctx.server = null;
    await stopping.cleanup();
    await waitUntil(
      () => stopping.process.exitCode !== null || stopping.process.signalCode !== null,
      STOP_TIMEOUT_MS,
    );
    expect(stopping.process.exitCode).toBe(0);
  };

  const restart = async (): Promise<number> => {
    await stop();
    await start();
    return ctx.server!.port;
  };

  const connect = async (userId: string): Promise<TestClient> => {
    const client = await createRustTestClient(ctx.server!.port, {
      userId,
      roles: ['ADMIN'],
    });
    await client.waitForMessage('AUTH_ACK', 10_000);
    client.stopHeartbeat();
    ctx.clients.push(client);
    return client;
  };

  const createStatusIndex = async (): Promise<void> => {
    const created = await admin(ctx.server!.port, 'POST', '/api/admin/indexes', {
      mapName: MAP,
      attribute: 'status',
      indexType: 'hash',
    });
    expect(created.status).toBe(201);
  };

  return { ctx, start, stop, restart, connect, createStatusIndex };
}

describe('Integration: whole-map reads see stored records after a restart (TG-READ-001)', () => {
  const s = storedServer();
  const WRITTEN: Record<string, Record<string, string>> = {
    a: { id: 'a', status: 'open', text: 'alpha zebra common' },
    b: { id: 'b', status: 'open', text: 'beta common' },
    c: { id: 'c', status: 'done', text: 'gamma common' },
    d: { id: 'd', status: 'later', text: 'epsilon common' },
  };

  beforeAll(async () => {
    s.ctx.dir = fs.mkdtempSync(path.join(os.tmpdir(), 'topgun-stored-read-'));
    await s.start();
    // Before any write: the index exists on the predicate attribute.
    await s.createStatusIndex();
    const writer = await s.connect('writer');
    await writeMany(
      writer,
      Object.entries(WRITTEN).map(([key, value]) => ({ key, value })),
    );
    await writeMany(writer, [
      { key: 'gone', value: { id: 'gone', status: 'open', text: 'delta common' } },
    ]);
    await remove(writer, 'gone');
  });

  afterAll(async () => {
    await s.stop();
    if (s.ctx.dir) fs.rmSync(s.ctx.dir, { recursive: true, force: true });
  });

  test('AC1: /sync returns every stored record, and the filtered ones', async () => {
    const port = await s.restart();
    const rows = await syncRows(port);
    expect(rows.map((r: any) => r.id).sort()).toEqual(['a', 'b', 'c', 'd']);
    for (const row of rows) expect(row).toEqual(WRITTEN[row.id]);
    expect(await syncKeys(port, { status: 'open' })).toEqual(['a', 'b']);
  });

  test('AC2: SEARCH finds stored records', async () => {
    await s.restart();
    const client = await s.connect('ac2');
    expect(await search(client, 'zebra')).toEqual(['a']);
    expect(await search(client, 'common', 10)).toEqual(['a', 'b', 'c', 'd']);
  });

  test('AC3: hybrid exact, fullText and includeValue see stored records despite the index', async () => {
    await s.restart();
    const client = await s.connect('ac3');
    expect(await exactKeys(client)).toEqual(['a', 'b']);
    const fullText = await hybrid(client, { methods: ['fullText'], queryText: 'zebra' });
    expect(fullText.map((r: any) => r.key)).toEqual(['a']);
    const withValues = await hybrid(client, {
      methods: ['exact'],
      predicate: STATUS_OPEN,
      includeValue: true,
    });
    expect(withValues.map((r: any) => r.key).sort()).toEqual(['a', 'b']);
    for (const r of withValues) expect(r.value).toEqual(WRITTEN[r.key]);
  });

  test('AC4(a): a key removed before the restart appears in no read path', async () => {
    const port = await s.restart();
    const client = await s.connect('ac4a');
    expect(await syncKeys(port)).not.toContain('gone');
    expect(await search(client, 'delta')).toEqual([]);
    expect(await exactKeys(client)).not.toContain('gone');
    const fullText = await hybrid(client, { methods: ['fullText'], queryText: 'delta' });
    expect(fullText).toEqual([]);
  });

  test('AC4(b): deleting a non-resident key after the restart removes it from search and /sync', async () => {
    const port = await s.restart();
    const client = await s.connect('ac4b');
    // Populates the index from the store.
    expect(await search(client, 'epsilon')).toEqual(['d']);
    // `d` is not resident right before the delete.
    expect(await residentCount(port)).toBe(0);
    await remove(client, 'd');
    expect(await search(client, 'epsilon')).toEqual([]);
    expect(await syncKeys(port)).not.toContain('d');
  });
});

describe('Integration: whole-map reads see evicted records (TG-READ-001)', () => {
  const s = storedServer({ TOPGUN_MAX_RAM_MB: '1' });
  const N = 600;
  const PAD = 'x'.repeat(4000);

  beforeAll(async () => {
    s.ctx.dir = fs.mkdtempSync(path.join(os.tmpdir(), 'topgun-stored-evict-'));
    await s.start();
    await s.createStatusIndex();
    const writer = await s.connect('writer');
    for (let from = 0; from < N; from += 50) {
      const batch = [];
      for (let i = from; i < Math.min(N, from + 50); i++) {
        const key = `k${String(i).padStart(4, '0')}`;
        batch.push({ key, value: { id: key, status: 'open', text: 'sharedterm', pad: PAD } });
      }
      await writeMany(writer, batch);
    }
  }, 120_000);

  afterAll(async () => {
    await s.stop();
    if (s.ctx.dir) fs.rmSync(s.ctx.dir, { recursive: true, force: true });
  });

  test('AC5: after eviction /sync, SEARCH and hybrid exact still return every record', async () => {
    const port = s.ctx.server!.port;
    let resident = N;
    await waitUntil(async () => {
      resident = await residentCount(port);
      return resident < N;
    }, 30_000);
    process.stderr.write(`[stored-read-paths] eviction witness: entry_count ${resident} < ${N}\n`);

    expect(new Set(await syncKeys(port)).size).toBe(N);
    const client = await s.connect('ac5');
    expect(await search(client, 'sharedterm', N + 10)).toHaveLength(N);
    expect(await exactKeys(client, N + 10)).toHaveLength(N);
  }, 120_000);
});

/** The admin index listing's `entryCount` of the index on `MAP`.`status`. */
async function statusIndexEntries(port: number): Promise<number> {
  const listed = await admin(port, 'GET', '/api/admin/indexes');
  expect(listed.status).toBe(200);
  const index = listed.json.indexes.find((i: any) => i.mapName === MAP && i.attribute === 'status');
  expect(index).toBeDefined();
  return index.entryCount;
}

/** Writes keys `k0000…` with ~4 KiB values, `status` chosen per key. */
async function writePadded(
  client: TestClient,
  n: number,
  status: (i: number) => string,
): Promise<void> {
  const pad = 'x'.repeat(4000);
  for (let from = 0; from < n; from += 50) {
    const batch = [];
    for (let i = from; i < Math.min(n, from + 50); i++) {
      const key = `k${String(i).padStart(4, '0')}`;
      batch.push({ key, value: { id: key, status: status(i), pad } });
    }
    await writeMany(client, batch);
  }
}

/** Waits until the resident count of `MAP` drops below `n`, so eviction has run. */
async function evictionWitness(port: number, n: number): Promise<void> {
  let resident = n;
  await waitUntil(async () => {
    resident = await residentCount(port);
    return resident < n;
  }, 30_000);
  process.stderr.write(`[stored-read-paths] eviction witness: entry_count ${resident} < ${n}\n`);
}

describe('Integration: a scalar index under eviction (TG-IDX-001)', () => {
  const N = 600;
  let s: ReturnType<typeof storedServer>;

  beforeEach(async () => {
    s = storedServer({ TOPGUN_MAX_RAM_MB: '1' });
    s.ctx.dir = fs.mkdtempSync(path.join(os.tmpdir(), 'topgun-stored-index-'));
    await s.start();
    await s.createStatusIndex();
  });

  afterEach(async () => {
    await s.stop();
    if (s.ctx.dir) fs.rmSync(s.ctx.dir, { recursive: true, force: true });
  });

  test('AC14: the scalar index keeps every evicted key', async () => {
    const port = s.ctx.server!.port;
    const writer = await s.connect('writer');
    await writePadded(writer, N, (i) => `s${i}`);
    await evictionWitness(port, N);
    expect(await statusIndexEntries(port)).toBe(N);
  }, 120_000);

  test('AC22: rewriting evicted keys replaces their index entries', async () => {
    const port = s.ctx.server!.port;
    const writer = await s.connect('writer');
    await writePadded(writer, N, () => 'old');
    await evictionWitness(port, N);
    await writePadded(writer, N, () => 'new');
    expect(await statusIndexEntries(port)).toBe(N);
  }, 120_000);
});

/** The backfill status of the index on `MAP`.`status`. */
async function statusIndexProgress(port: number): Promise<any> {
  const status = await admin(port, 'GET', `/api/admin/indexes/${MAP}/status/status`);
  expect(status.status).toBe(200);
  return status.json;
}

describe('Integration: a scalar index after a restart covers the stored records (TG-IDX-001)', () => {
  const N = 50;
  let s: ReturnType<typeof storedServer>;

  const writeDistinct = async (): Promise<void> => {
    const writer = await s.connect('writer');
    const records = [];
    for (let i = 0; i < N; i++) records.push({ key: `k${i}`, value: { id: `k${i}`, status: `s${i}` } });
    await writeMany(writer, records);
  };

  beforeEach(async () => {
    s = storedServer();
    s.ctx.dir = fs.mkdtempSync(path.join(os.tmpdir(), 'topgun-stored-rebuild-'));
    await s.start();
  });

  afterEach(async () => {
    await s.stop();
    if (s.ctx.dir) fs.rmSync(s.ctx.dir, { recursive: true, force: true });
  });

  test('AC12: the index restored at start holds every stored record', async () => {
    await s.createStatusIndex();
    await writeDistinct();
    const port = await s.restart();
    expect(await statusIndexEntries(port)).toBe(N);
    expect(await statusIndexProgress(port)).toMatchObject({
      done: true,
      total: N,
      processed: N,
      rebuildType: 'startupRebuild',
    });
  }, 60_000);

  test('AC13: create_index after a restart backfills every stored record', async () => {
    await writeDistinct();
    const port = await s.restart();
    // No read of the map before the index is created.
    await s.createStatusIndex();
    let progress: any;
    await waitUntil(async () => {
      progress = await statusIndexProgress(port);
      return progress.done === true;
    }, 30_000);
    expect(progress).toMatchObject({ done: true, total: N, processed: N });
    expect(await statusIndexEntries(port)).toBe(N);
  }, 60_000);
});
