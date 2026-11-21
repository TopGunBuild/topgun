/**
 * Integration: a live query's rows are in the server's order, on a real server.
 *
 * The client orders a live query by the server's sort-key order (booleans,
 * numbers, strings, then null and missing values; `desc` reverses a field; ties
 * by key). This file checks that order against what the server itself sends, for
 * the value kinds where the server's page order and its live window agree with
 * it: finite numbers, strings, `null` and absent values under `asc`, finite
 * numbers and strings under `desc` — each with and without a `limit`.
 *
 * Two clients: W writes through the TopGun client; S subscribes and its
 * inbound frames are recorded through the engine's generic message listener.
 * For each case:
 *
 * - the page: the keys of S's QUERY_RESP, in frame order, equal the rows the
 *   handle shows (set and order);
 * - the live window: after each of W's writes, the keys the server's
 *   QUERY_RESP and QUERY_UPDATEs leave in the window equal the rows the handle
 *   holds, and the rows the handle shows are those keys in the order of a fresh
 *   server page without a limit (a QUERY_RESP of a new one-shot query), the
 *   first `limit` of them. A fresh limited page is not the oracle: the window
 *   promotes only rows it has seen, so a row that falls out of it with no
 *   retained candidate below stays in it (TODO-1005).
 */

import { TopGunClient, SyncState } from '@topgunbuild/client';
import type { QueryFilter, QueryHandle } from '@topgunbuild/client';

import { spawnRustServer, createTestToken, MemoryStorageAdapter, SpawnedServer } from './helpers';

type Value = { v?: unknown };
type Write = [string, Value | null];

const sleep = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

async function waitFor(predicate: () => boolean, describe: () => string, timeoutMs = 20_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (predicate()) return;
    await sleep(25);
  }
  throw new Error(`timed out: ${describe()}`);
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

// eslint-disable-next-line @typescript-eslint/no-explicit-any -- the engine is private to the client
const engineOf = (client: TopGunClient): any => (client as any).syncEngine;

/** Seed values under `asc`: finite numbers (ties included), strings (an astral one), null, absent. */
const ASC_SEED: Write[] = [
  ['n1', { v: 5 }],
  ['n2', { v: -1.5 }],
  ['n3', { v: 5 }],
  ['n4', { v: 0 }],
  ['s1', { v: 'apple' }],
  ['s2', { v: '\u{1F600}' }],
  ['s3', { v: '�' }],
  ['s4', { v: '10' }],
  ['z1', { v: null }],
  ['z0', {}],
  ['z2', { v: null }],
];

/** Writes that admit a row into a window of 4, move one out of it, enter below it and leave. */
const ASC_WRITES: Write[] = [
  ['n5', { v: -10 }],
  ['n2', { v: null }],
  ['s5', { v: 'b' }],
  ['n4', { v: 'a' }],
  ['n5', null],
  ['z3', {}],
  ['n1', { v: -2 }],
];

/** Under `desc`: finite numbers and strings only. */
const DESC_SEED: Write[] = [
  ['n1', { v: 5 }],
  ['n2', { v: -1.5 }],
  ['n3', { v: 5 }],
  ['n4', { v: 100 }],
  ['s1', { v: 'apple' }],
  ['s2', { v: '\u{1F600}' }],
  ['s3', { v: '�' }],
];

const DESC_WRITES: Write[] = [
  ['s4', { v: 'zzz' }],
  ['s1', { v: 50 }],
  ['n5', { v: 7 }],
  ['s4', null],
  ['n2', { v: '\u{10000}' }],
];

const CASES: { name: string; filter: QueryFilter; seed: Write[]; writes: Write[] }[] = [
  { name: 'asc', filter: { sort: { v: 'asc' } }, seed: ASC_SEED, writes: ASC_WRITES },
  {
    name: 'asc, limit 4',
    filter: { sort: { v: 'asc' }, limit: 4 },
    seed: ASC_SEED,
    writes: ASC_WRITES,
  },
  { name: 'desc', filter: { sort: { v: 'desc' } }, seed: DESC_SEED, writes: DESC_WRITES },
  {
    name: 'desc, limit 3',
    filter: { sort: { v: 'desc' }, limit: 3 },
    seed: DESC_SEED,
    writes: DESC_WRITES,
  },
];

describe('Integration: live query order matches the server (client <-> Rust server)', () => {
  let server: SpawnedServer | null = null;
  let subscriber: TopGunClient | null = null;
  let writer: TopGunClient | null = null;

  beforeAll(async () => {
    server = await spawnRustServer();
    subscriber = makeClient(server.port, 'order-subscriber');
    writer = makeClient(server.port, 'order-writer');
    await subscriber.start();
    await writer.start();
    await waitFor(
      () =>
        subscriber!.getConnectionState() === SyncState.CONNECTED &&
        writer!.getConnectionState() === SyncState.CONNECTED,
      () => 'both clients connected',
    );
  }, 60_000);

  afterAll(async () => {
    await subscriber?.close().catch(() => {});
    await writer?.close().catch(() => {});
    await server?.cleanup().catch(() => {});
  });

  test.each(CASES)(
    '$name',
    async ({ name, filter, seed, writes }) => {
      const s = subscriber!;
      const w = writer!;
      const mapName = `order-${name.replace(/[^a-z0-9]+/g, '-')}-${Date.now()}`;
      const writerMap = w.getMap<string, Value>(mapName);
      const apply = ([key, value]: Write) => {
        if (value === null) writerMap.remove(key);
        else writerMap.set(key, value);
      };
      const drained = () => w.getPendingOpsCount() === 0;

      for (const write of seed) apply(write);
      await waitFor(drained, () => 'seed acknowledged');
      // An acknowledged write can still be missing from a scan; wait until a fresh
      // server page holds the whole seed before the live subscribe.
      const all = await (async () => {
        const deadline = Date.now() + 20_000;
        while (Date.now() < deadline) {
          const rows = await s.queryOnce<Value>(mapName, {}, { timeoutMs: 5_000 }).catch(() => []);
          if (rows.length === seed.length) return rows;
          await sleep(100);
        }
        throw new Error('seed never became visible to a query');
      })();
      expect(all).toHaveLength(seed.length);

      // Every QUERY_RESP and QUERY_UPDATE S receives, by query id, in arrival order.
      const responses = new Map<string, string[][]>();
      const windows = new Map<string, Set<string>>();
      engineOf(s).on('message', (raw: unknown) => {
        // eslint-disable-next-line @typescript-eslint/no-explicit-any -- inbound frames have no single static shape
        const msg = raw as any;
        const queryId: string | undefined = msg?.payload?.queryId;
        if (!queryId) return;
        if (msg.type === 'QUERY_RESP' && !msg.payload.code) {
          const keys = (msg.payload.results ?? []).map((r: { key: string }) => r.key);
          responses.set(queryId, [...(responses.get(queryId) ?? []), keys]);
          windows.set(queryId, new Set(keys));
        } else if (msg.type === 'QUERY_UPDATE') {
          const window = windows.get(queryId) ?? new Set<string>();
          if (msg.payload.changeType === 'LEAVE') window.delete(msg.payload.key);
          else window.add(msg.payload.key);
          windows.set(queryId, window);
        }
      });

      const handle: QueryHandle<Value> = s.query<Value>(mapName, filter);
      let shown: string[] = [];
      const unsubscribe = handle.subscribe((rows) => (shown = rows.map((r) => r._key)));
      const held = () => [...(handle as unknown as { rows: Map<string, unknown> }).rows.keys()];
      const sorted = (keys: Iterable<string>) => [...keys].sort();

      try {
        // The page: QUERY_RESP's keys, in frame order, are the rows the handle shows.
        await waitFor(
          () => handle.getSnapshotMeta().settled && (responses.get(handle.id)?.length ?? 0) > 0,
          () => 'live query settled',
        );
        const page = responses.get(handle.id)![0];
        expect(page.length).toBe(filter.limit ?? seed.length);
        expect(shown).toEqual(page);

        // The live window, after each write: the server's window set is the set
        // the handle holds, and a fresh server page is the order it shows.
        // A fresh one-shot page of the same query without its limit, read from its
        // raw QUERY_RESP, cut to the keys the server's live window holds.
        const unlimited: QueryFilter = { sort: filter.sort };
        const freshPage = async (): Promise<string[]> => {
          const before = new Set(responses.keys());
          await s.queryOnce<Value>(mapName, unlimited, { timeoutMs: 5_000 });
          const freshId = [...responses.keys()].find((id) => !before.has(id) && id !== handle.id);
          const order = freshId ? responses.get(freshId)![0] : [];
          const window = windows.get(handle.id) ?? new Set<string>();
          return order.filter((key) => window.has(key)).slice(0, filter.limit ?? Infinity);
        };
        const windowKeys = () => sorted(windows.get(handle.id) ?? []);

        for (const write of writes) {
          apply(write);
          await waitFor(drained, () => `write ${JSON.stringify(write)} acknowledged`);
          // The write's QUERY_UPDATEs may trail its acknowledgement; poll until
          // the handle shows what a fresh page holds.
          const deadline = Date.now() + 20_000;
          let fresh = await freshPage();
          while (
            Date.now() < deadline &&
            (JSON.stringify(fresh) !== JSON.stringify(shown) ||
              JSON.stringify(windowKeys()) !== JSON.stringify(sorted(held())))
          ) {
            await sleep(50);
            fresh = await freshPage();
          }
          expect({ write, shown }).toEqual({ write, shown: fresh });
          expect({ write, held: sorted(held()) }).toEqual({ write, held: windowKeys() });
        }
      } finally {
        unsubscribe();
      }
    },
    90_000,
  );
});
