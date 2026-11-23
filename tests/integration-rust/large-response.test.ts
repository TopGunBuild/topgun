/**
 * Responses larger than the server's per-connection WebSocket write buffer
 * reach a TopGunClient whole, on the connection that asked for them.
 *
 * The server's write buffer is 512 KiB. A message larger than that is sent as
 * RFC 6455 continuation frames (TG-WS-001); before that, the writer refused
 * such a message, closed the connection, and the client reconnected into the
 * same failure. Each case uses a fresh client, counts every transition into a
 * disconnected state, and checks that what arrived really is above the case's
 * size boundary, so a test that silently shrank below it would fail.
 */

import { pack } from 'msgpackr';

import { TopGunClient, SyncState } from '@topgunbuild/client';

import {
  spawnRustServer,
  createRustTestClient,
  createTestToken,
  waitUntil,
  MemoryStorageAdapter,
  TestClient,
} from './helpers';

const RUNS = 5;
const PAD = 'x'.repeat(1000);
const MAP_A = 'large-resp-700';
const MAP_B = 'large-resp-2300';
const MAP_C = 'large-resp-4000';
const DISCONNECTED_STATES = new Set<string>([
  SyncState.DISCONNECTED,
  SyncState.BACKOFF,
  SyncState.ERROR,
]);

function rowKey(i: number): string {
  return `k${i}`;
}

function expectedKeys(rows: number): string[] {
  return Array.from({ length: rows }, (_, i) => rowKey(i)).sort();
}

/** Writes `rows` rows with OP_BATCHes of 100 ops, waiting for each batch's OP_ACK. */
async function writeRows(writer: TestClient, mapName: string, rows: number): Promise<void> {
  for (let at = 0; at < rows; at += 100) {
    const ops = [];
    for (let i = at; i < Math.min(at + 100, rows); i++) {
      ops.push({
        id: `${mapName}-${i}`,
        mapName,
        opType: 'PUT',
        key: rowKey(i),
        record: {
          value: { i, pad: PAD, word: 'needle' },
          timestamp: { millis: Date.now(), counter: i, nodeId: 'large-resp-writer' },
        },
      });
    }
    writer.messages.length = 0;
    writer.send({ type: 'OP_BATCH', payload: { ops } });
    const ack = await writer.waitForMessage('OP_ACK', 10_000);
    expect(ack.type).toBe('OP_ACK');
  }
}

/** A fresh client that counts every transition into a disconnected state until `close()`. */
class CountingClient {
  readonly client: TopGunClient;
  disconnects = 0;
  private closing = false;

  constructor(port: number, run: string) {
    const token = createTestToken(`large-resp-${run}`, ['ADMIN']);
    this.client = new TopGunClient({
      serverUrl: `ws://127.0.0.1:${port}/ws`,
      storage: new MemoryStorageAdapter(),
      auth: { getToken: async () => token },
    });
    this.client.onConnectionStateChange((event) => {
      if (!this.closing && DISCONNECTED_STATES.has(event.to)) {
        this.disconnects++;
      }
    });
  }

  async close(): Promise<void> {
    this.closing = true;
    await this.client.close();
  }
}

function keysOf(rows: Array<Record<string, unknown>>): string[] {
  return rows.map((row) => String(row._key)).sort();
}

/** Bounds a request whose answer may never come, so a lost answer fails fast. */
async function withTimeout<T>(promise: Promise<T>, ms: number, what: string): Promise<T> {
  let timer: NodeJS.Timeout | undefined;
  const timeout = new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(new Error(`${what}: no answer within ${ms} ms`)), ms);
  });
  try {
    return await Promise.race([promise, timeout]);
  } finally {
    clearTimeout(timer);
  }
}

function encodedSize(value: unknown): number {
  return pack(value).length;
}

describe('Integration: responses above the WebSocket write buffer (Rust Server)', () => {
  let port: number;
  let cleanup: () => Promise<void>;
  let writer: TestClient;

  beforeAll(async () => {
    const server = await spawnRustServer();
    port = server.port;
    cleanup = server.cleanup;
    writer = await createRustTestClient(port, {
      nodeId: 'large-resp-writer',
      userId: 'large-resp-writer',
      roles: ['ADMIN'],
    });
    await writer.waitForMessage('AUTH_ACK', 10_000);
    await writeRows(writer, MAP_A, 700);
    await writeRows(writer, MAP_B, 2300);
    await writeRows(writer, MAP_C, 4000);
  }, 120_000);

  afterAll(async () => {
    writer?.close();
    await cleanup?.();
  });

  async function oneShotCase(mapName: string, rows: number, boundary: number): Promise<void> {
    for (let run = 0; run < RUNS; run++) {
      const c = new CountingClient(port, `${mapName}-${run}`);
      try {
        const result = (await c.client.queryOnce(mapName, {}, { timeoutMs: 15_000 })) as Array<
          Record<string, unknown>
        >;
        expect(result).toHaveLength(rows);
        expect(keysOf(result)).toEqual(expectedKeys(rows));
        expect(encodedSize(result)).toBeGreaterThan(boundary);
        expect(c.disconnects).toBe(0);
      } finally {
        await c.close();
      }
    }
  }

  test('A: a queryOnce answer above 512 KB arrives whole, 5 of 5, with no disconnect', async () => {
    await oneShotCase(MAP_A, 700, 524_288);
  }, 120_000);

  test('B: a queryOnce answer above 2 MB arrives whole, 5 of 5, with no disconnect', async () => {
    await oneShotCase(MAP_B, 2300, 2_097_152);
  }, 120_000);

  test('C: a ~4.4 MB live snapshot arrives whole and a 4 000-hit search follows, 5 of 5, with no disconnect', async () => {
    for (let run = 0; run < RUNS; run++) {
      const c = new CountingClient(port, `${MAP_C}-${run}`);
      try {
        let rows: Array<Record<string, unknown>> = [];
        const handle = c.client.query(MAP_C, {});
        const unsubscribe = handle.subscribe((r) => {
          rows = r as Array<Record<string, unknown>>;
        });
        try {
          await waitUntil(() => rows.length === 4000, 30_000, 50);
        } finally {
          unsubscribe();
        }
        expect(keysOf(rows)).toEqual(expectedKeys(4000));
        expect(encodedSize(rows)).toBeGreaterThan(4_000_000);

        // Indexing is asynchronous, so the search is polled until every row is
        // indexed: a red here then means a lost answer, not index lag.
        let hits: Array<{ key: string; value: unknown }> = [];
        await waitUntil(
          async () => {
            hits = await withTimeout(
              c.client.search(MAP_C, 'needle', { limit: 4000 }),
              10_000,
              'search',
            );
            return hits.length === 4000;
          },
          30_000,
          200,
        );
        expect(hits.map((h) => h.key).sort()).toEqual(expectedKeys(4000));
        // A one-shot search hit carries no value (`null`), so its answer stays
        // below the boundary; only a hit list with values is held to it.
        if (hits.every((h) => h.value != null)) {
          expect(encodedSize(hits)).toBeGreaterThan(4_000_000);
        }
        expect(c.disconnects).toBe(0);
      } finally {
        await c.close();
      }
    }
  }, 240_000);
});
