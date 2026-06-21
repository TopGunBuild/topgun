/**
 * Cross-boundary live top-N clamp test against the real Rust server.
 *
 * Exercises the full @topgunbuild/client SDK (TopGunClient -> SyncEngine ->
 * QueryHandle) over real WebSockets to a real Rust server, proving the
 * end-to-end behavior the unit tests cannot: that a live `limit:N` subscription's
 * EMITTED result set stays clamped to exactly N rows BOTH on the initial snapshot
 * of pre-seeded data AND after the server starts sending top-N displacement deltas
 * for a write that ranks a new row into the window and pushes an existing row out.
 *
 * The flow: seed THREE rows (one more than the limit) and wait for them to be
 * APPLIED server-side, then subscribe with limit:2 ascending. The initial rendered
 * window is the top-2 [a, b] — the third seed row d(3) is below the window. Then a
 * separate writer adds a new lowest n=0 that displaces b out, so the window nets to
 * [c, a]. The Rust server emits correct displacement (a LEAVE for the displaced row
 * + an ENTER for the new in-window row); the client's render-time, non-destructive
 * window slice composes with those deltas so the FINAL rendered set nets to exactly
 * N in sort order. This mirrors the AC1/AC2/AC3 unit scenarios at integration scale.
 *
 * Determinism (startup-speed independent): the writer's `set()` calls are
 * optimistic and batched, so the seed is first gated behind waitForSynced(writer)
 * — every seed op has drained and been OP_ACKed. But OP_ACK only proves the op was
 * accepted, not that it is already VISIBLE to a query scan: on a cold/slow start
 * the per-record apply into the partition RecordStore can lag the ack, and
 * `handle_query_subscribe` scans the snapshot BEFORE it registers the subscription
 * — so a not-yet-applied seed can land in the gap and surface in neither the
 * initial snapshot nor a delta, freezing a live handle at an incomplete window
 * forever (the intermittent PR #65 red: got `[]`, or `["a"]` after the first fix).
 *
 * The fix gates the real subscribe on the seed being CONFIRMED QUERY-VISIBLE via
 * waitForServerQuery(): a poll loop over `queryOnce`, which opens a FRESH server
 * subscription each call, waits for the authoritative QUERY_RESP, reads it, and
 * auto-unsubscribes. Because every poll is a new server-side scan (not a frozen
 * snapshot), it converges to the full seed set as soon as the writes are stably
 * applied — independent of how slow the server started. Once the top-2 is
 * server-visible the store is stable, so the subsequent live subscribe's initial
 * snapshot is deterministically `[a, b]`. (This is not a client replay gap — the
 * server replays pre-seeded data on subscribe, verified independently.)
 *
 * Scope note (honesty): against the real server, every observed path is already
 * net-N correct on its own — the initial QUERY_RESP is itself limit-clamped to N,
 * an ENTER that ranks outside the window is suppressed entirely, and an in-window
 * displacement arrives as LEAVE-then-ENTER (never a transient N+1). So this test
 * proves the literal end-to-end claim (the client's emitted set is exactly N,
 * correctly ordered, on both the initial snapshot and after a server-driven
 * displacement); the client clamp's extra safety against an unpaired/out-of-order
 * ENTER and against an over-full snapshot is exercised load-bearingly by the unit
 * suite, which can inject those directly. We do NOT assert the client clamp is the
 * load-bearing mechanism here, because the real server never forces it to be.
 *
 * Two clients are used deliberately: a separate writer drives the mutation while
 * the subscriber observes the live query. The server suppresses self-echo of
 * QUERY_UPDATE deltas back to the originating writer, so a single self-writing
 * client would never receive the displacement stream this test must verify.
 */

import { TopGunClient, SyncState } from '@topgunbuild/client';
import type { IStorageAdapter, OpLogEntry, QueryFilter } from '@topgunbuild/client';

import { spawnRustServer, createTestToken, SpawnedServer } from './helpers';

// In-memory storage adapter so the SDK runs headless in Node (no IndexedDB).
class MemoryStorageAdapter implements IStorageAdapter {
  private kv = new Map<string, unknown>();
  private meta = new Map<string, unknown>();
  private opLog: OpLogEntry[] = [];
  private pending: OpLogEntry[] = [];

  async initialize(): Promise<void> {}
  async close(): Promise<void> {}
  async get(key: string): Promise<any> {
    return this.kv.get(key);
  }
  async put(key: string, value: unknown): Promise<void> {
    this.kv.set(key, value);
  }
  async remove(key: string): Promise<void> {
    this.kv.delete(key);
  }
  async getMeta(key: string): Promise<any> {
    return this.meta.get(key);
  }
  async setMeta(key: string, value: unknown): Promise<void> {
    this.meta.set(key, value);
  }
  async batchPut(entries: Map<string, unknown>): Promise<void> {
    for (const [k, v] of entries) this.kv.set(k, v);
  }
  async appendOpLog(entry: Omit<OpLogEntry, 'id'>): Promise<number> {
    const id = this.opLog.length + 1;
    const e = { ...entry, id, synced: 0 } as OpLogEntry;
    this.opLog.push(e);
    this.pending.push(e);
    return id;
  }
  async getPendingOps(): Promise<OpLogEntry[]> {
    return this.pending;
  }
  async markOpsSynced(lastId: number): Promise<void> {
    this.pending = this.pending.filter((op) => (op.id ?? 0) > lastId);
    this.opLog.forEach((op) => {
      if ((op.id ?? 0) <= lastId) op.synced = 1;
    });
  }
  async deleteOp(id: number): Promise<void> {
    this.opLog = this.opLog.filter((op) => op.id !== id);
  }

  async commitWrite(
    mutations: Array<{ store: 'kv' | 'meta'; type: 'put' | 'remove'; key: string; value?: any }>,
    op: Omit<OpLogEntry, 'id'>,
  ): Promise<number> {
    for (const m of mutations) {
      const target = m.store === 'meta' ? this.meta : this.kv;
      if (m.type === 'remove') target.delete(m.key);
      else target.set(m.key, m.value as never);
    }
    return this.appendOpLog(op);
  }

  async getAllKeys(): Promise<string[]> {
    return Array.from(this.kv.keys());
  }
}

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

/** Polls a condition rather than sleeping a fixed interval for live emissions. */
async function waitUntil(
  predicate: () => boolean,
  timeoutMs: number,
  describe: () => string,
): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (predicate()) return;
    await waitMs(50);
  }
  throw new Error(`Timed out waiting for condition: ${describe()}`);
}

/**
 * Resolves once the client has no unacknowledged pending operations — i.e. every
 * optimistic local write has round-tripped to the server and been APPLIED. The
 * writer's `set()` calls are optimistic and batched, so without this wait the
 * subscriber's QUERY_SUB can race AHEAD of the seed writes reaching the server;
 * the server then either snapshots an empty map OR (on a cold first spawn) lands
 * a write in the gap between taking the snapshot and activating the live
 * subscription, so the seed surfaces in neither the snapshot nor a delta. Gating
 * the subscribe on a fully-drained writer makes the seed deterministically
 * present in the initial QUERY_RESP snapshot.
 */
async function waitForSynced(client: TopGunClient, timeoutMs = 20_000): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (client.getPendingOpsCount() === 0) return;
    await waitMs(25);
  }
  throw new Error(
    `Timed out waiting for writer to drain; pending = ${client.getPendingOpsCount()}`,
  );
}

/**
 * Deterministically waits until the SERVER's query result for (mapName, filter)
 * satisfies `predicate`, independent of server startup speed.
 *
 * Each poll issues a fresh `queryOnce` — which opens a new server subscription,
 * waits for the AUTHORITATIVE QUERY_RESP, reads the snapshot, then
 * auto-unsubscribes. A live QueryHandle freezes its first snapshot and can never
 * recover if a seed write lands in the server's scan-vs-subscription-activation
 * gap on a cold start; re-scanning with a fresh subscription each poll sidesteps
 * that entirely, converging as soon as the seed writes are stably applied. Once
 * this resolves, the store is stable, so a subsequent live subscribe sees a
 * complete, deterministic initial snapshot.
 */
async function waitForServerQuery<T>(
  client: TopGunClient,
  mapName: string,
  filter: QueryFilter,
  predicate: (rows: Array<T & { _key: string }>) => boolean,
  describe: () => string,
  timeoutMs = 20_000,
): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  let lastSeen = '<none>';
  while (Date.now() < deadline) {
    try {
      const rows = (await client.queryOnce<T>(mapName, filter, {
        timeoutMs: 5_000,
      })) as Array<T & { _key: string }>;
      if (predicate(rows)) return;
      lastSeen = JSON.stringify(rows.map((r) => r._key));
    } catch {
      // Offline / settle timeout on this poll — retry until the outer deadline.
    }
    await waitMs(100);
  }
  throw new Error(`Timed out waiting for server query: ${describe()}; last = ${lastSeen}`);
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

describe('Integration: live top-N clamp end-to-end (client <-> Rust server)', () => {
  let server: SpawnedServer | null = null;
  let subscriber: TopGunClient | null = null;
  let writer: TopGunClient | null = null;

  afterEach(async () => {
    if (subscriber) {
      await subscriber.close().catch(() => {});
      subscriber = null;
    }
    if (writer) {
      await writer.close().catch(() => {});
      writer = null;
    }
    if (server) {
      await server.cleanup().catch(() => {});
      server = null;
    }
  });

  test('limit:2 live query stays clamped to the top-2 by sort order after a displacing write', async () => {
    server = await spawnRustServer();

    subscriber = makeClient(server.port, 'clamp-subscriber');
    writer = makeClient(server.port, 'clamp-writer');
    await subscriber.start();
    await writer.start();
    await waitForState(subscriber, SyncState.CONNECTED, 20_000);
    await waitForState(writer, SyncState.CONNECTED, 20_000);

    // Unique map name so every row in the map matches the (filterless) query and
    // the displacement is unambiguous — no cross-test pollution under the null
    // in-memory backend.
    const mapName = `clamp-map-${Date.now()}`;
    const writerMap = writer.getMap<string, { n: number }>(mapName);

    // Seed THREE rows — one MORE than the limit — so the initial window is
    // genuinely over-full and the rendered initial snapshot must be clamped down
    // to exactly `limit`. Ascending sort on `n` ranks a(1), b(2), then d(3); with
    // limit:2 the authoritative top-2 window is [a, b] and d(3) is below the
    // window. (Seeding only `limit` rows — as the original test did — never
    // exercises an initial-snapshot clamp, since `limit` rows trivially fit.)
    writerMap.set('a', { n: 1 });
    writerMap.set('b', { n: 2 });
    writerMap.set('d', { n: 3 });

    // Gate the subscribe on the writer fully draining its pending ops, so all
    // three seed rows are OP_ACKed server-side BEFORE the QUERY_SUB.
    await waitForSynced(writer);

    // OP_ACK proves acceptance, not query-visibility: on a cold/slow start the
    // per-record apply can lag the ack, and a seed could land in the server's
    // scan-vs-activation gap, freezing a live handle at an incomplete window.
    // Confirm the over-full seed is CONFIRMED QUERY-VISIBLE (top-2 clamped to
    // [a, b], with d below the window) by re-scanning the server until it
    // converges. After this the store is stable, so the live subscribe below is
    // deterministic at any startup speed.
    await waitForServerQuery<{ n: number }>(
      subscriber,
      mapName,
      { limit: 2, sort: { n: 'asc' } },
      (rows) => rows.length === 2 && rows[0]?._key === 'a' && rows[1]?._key === 'b',
      () => 'seed top-2 [a, b] server-visible before live subscribe',
    );

    // Capture the latest emitted (rendered) result set from the live subscription.
    let last: Array<{ n: number; _key: string }> = [];
    const emittedLengths: number[] = [];
    const handle = subscriber.query<{ n: number }>(mapName, {
      limit: 2,
      sort: { n: 'asc' },
    });
    const unsubscribe = handle.subscribe((results) => {
      last = results as Array<{ n: number; _key: string }>;
      emittedLengths.push(results.length);
    });

    try {
      // The initial rendered window is exactly the top-2 [a, b] — d(3) is clamped
      // out of the window end-to-end. (Against the real server the QUERY_RESP
      // snapshot is itself limit-clamped to 2 rows; the client's render-time slice
      // is the defense-in-depth half, exercised load-bearingly by the unit suite.)
      await waitUntil(
        () => last.length === 2 && last[0]?._key === 'a' && last[1]?._key === 'b',
        20_000,
        () => `initial clamped top-2 window; got ${JSON.stringify(last.map((r) => r._key))}`,
      );

      // The below-window seed row d(3) is never in the rendered window.
      expect(last.find((r) => r._key === 'd')).toBeUndefined();

      // Write a NEW LOWEST row from the separate writer. The server ranks n=0 ahead
      // of a(1)/b(2)/d(3), so the top-2 window becomes [c, a] and b is displaced out.
      // The server emits the LEAVE(b) + ENTER(c) displacement deltas; the client's
      // render-time clamp must net these to exactly 2 rendered rows in the end — not
      // 3 (no clamp) and not a permanent 1 (double-drop of b on top of the clamp).
      writer.getMap<string, { n: number }>(mapName).set('c', { n: 0 });

      await waitUntil(
        () => last.length === 2 && last[0]?._key === 'c' && last[1]?._key === 'a',
        20_000,
        () => `clamped top-2 after displacement; got ${JSON.stringify(last.map((r) => r._key))}`,
      );

      // End-to-end assertions: exactly N rows, the correct two by ascending sort.
      expect(last).toHaveLength(2);
      expect(last.map((r) => r._key)).toEqual(['c', 'a']);
      expect(last.map((r) => r.n)).toEqual([0, 1]);

      // Neither the displaced row b nor the always-below-window row d is in the
      // rendered window.
      expect(last.find((r) => r._key === 'b')).toBeUndefined();
      expect(last.find((r) => r._key === 'd')).toBeUndefined();

      // No emission ever EXCEEDED the limit end-to-end. (On the real server this
      // holds because deltas are net-N correct per the scope note; the clamp would
      // additionally guarantee it under an out-of-order ENTER.)
      expect(Math.max(...emittedLengths)).toBeLessThanOrEqual(2);
    } finally {
      unsubscribe();
    }
  }, 60_000);
});
