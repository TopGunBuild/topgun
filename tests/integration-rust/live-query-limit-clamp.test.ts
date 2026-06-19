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
 * Determinism: the writer's `set()` calls are optimistic and batched, so the seed
 * is gated behind waitForSynced(writer) — the QUERY_SUB is sent only once all seed
 * writes have drained and been APPLIED on the server. Without this gate the
 * subscriber's QUERY_SUB can race ahead of the seed reaching the server, and on a
 * cold first spawn a seed write can land in the gap between the server taking the
 * snapshot and activating the live subscription — surfacing in neither the snapshot
 * nor a delta. That race (not a client replay gap — the server replays pre-seeded
 * data on subscribe, verified independently) was the intermittent PR #65 red.
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
import type { IStorageAdapter, OpLogEntry } from '@topgunbuild/client';

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
    // three seed rows are APPLIED server-side BEFORE the QUERY_SUB. This removes
    // the snapshot-vs-write race that otherwise lets the subscriber observe an
    // empty initial window on a cold spawn (the original flake on PR #65).
    await waitForSynced(writer);

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
