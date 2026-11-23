/**
 * A TopGunClient that holds a live query of one map, run in a process of its
 * own by helpers/client-subscriber-process.js (see there for how it loads).
 *
 * Started with `fork(…/client-subscriber-process.js, [port, mapName])`. Every
 * 100 ms it tells the parent `{ type: 'status', … }`:
 *
 * - `keys` — the number of distinct keys its live view holds;
 * - `state` — its connection state;
 * - `changes` — every connection-state transition since it first reached
 *   CONNECTED, each with `at` (ms since the epoch);
 * - `serverResults` / `lastServerResultAt` — server results applied to the live
 *   query handle (its `onResult(…, 'server')`, one per clean `QUERY_RESP`);
 * - `overflowResults` / `lastOverflowAt` — `QUERY_RESP`s of the live query that
 *   carry an error code (a snapshot overflow), seen on the engine's message bus;
 * - `searchResults` / `lastSearchResultAt` — `SEARCH_RESP`s of the live search
 *   (`search` command) for its current subscription id.
 *
 * Commands from the parent:
 *
 * - `{ type: 'search', query, limit }` — opens a live search of the map and
 *   keeps its latest results;
 * - `{ type: 'block', sentinel, maxMs }` — busy-waits without yielding to the
 *   event loop, so nothing is read from the socket, until the file `sentinel`
 *   exists or `maxMs` (default 30 000) passes; then answers
 *   `{ type: 'unblocked', found }`;
 * - `{ type: 'hog', busyMs, periodMs }` / `{ type: 'hog', off: true }` — steals
 *   the event loop for `busyMs` every `periodMs`, or stops doing so;
 * - `{ type: 'dump' }` — answers `{ type: 'rows', rows, searchRows }`: the live
 *   query's `[key, value]` pairs and the live search's sorted key set;
 * - `{ type: 'exit' }` — closes the client and ends the process.
 */
import * as fs from 'fs';

import { TopGunClient, SyncState } from '@topgunbuild/client';
import type { QueryHandle, SearchHandle, StateChangeEvent } from '@topgunbuild/client';

import { MemoryStorageAdapter } from './memory-storage';
import { createTestToken } from './test-client';

const [port, mapName] = process.argv.slice(2);
const token = createTestToken('client-subscriber', ['ADMIN']);
const client = new TopGunClient({
  serverUrl: `ws://localhost:${port}/ws`,
  storage: new MemoryStorageAdapter(),
  auth: { getToken: async () => token },
});

const changes: Array<StateChangeEvent & { at: number }> = [];
let keys = 0;
let watching = false;
let rows: Array<[string, unknown]> = [];
let serverResults = 0;
let lastServerResultAt: number | null = null;
let overflowResults = 0;
let lastOverflowAt: number | null = null;
let searchResults = 0;
let lastSearchResultAt: number | null = null;
// eslint-disable-next-line @typescript-eslint/no-explicit-any -- the live query's value type is whatever the test writes
let handle: QueryHandle<any> | null = null;
let search: SearchHandle<unknown> | null = null;
let hog: ReturnType<typeof setInterval> | null = null;

// eslint-disable-next-line @typescript-eslint/no-explicit-any -- the helper reaches into the client's engine
const engine = (client as any).syncEngine;

const busyFor = (ms: number) => {
  const until = Date.now() + ms;
  while (Date.now() < until) {
    // Deliberately spinning: the event loop must not run.
  }
};

const report = () =>
  process.send?.({
    type: 'status',
    keys,
    state: client.getConnectionState(),
    changes: changes.map((c) => ({ from: c.from, to: c.to, at: c.at })),
    serverResults,
    lastServerResultAt,
    overflowResults,
    lastOverflowAt,
    searchResults,
    lastSearchResultAt,
  });

function watch(): void {
  watching = true;
  client.onConnectionStateChange((event) => changes.push({ ...event, at: Date.now() }));
  handle = client.query(mapName, {});
  const live = handle;
  const onResult = live.onResult.bind(live);
  live.onResult = (items, source = 'server', merkleRootHash) => {
    if (source === 'server') {
      serverResults += 1;
      lastServerResultAt = Date.now();
    }
    return onResult(items, source, merkleRootHash);
  };
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- frames on the engine's bus are untyped
  engine.on('message', (message: any) => {
    if (
      message?.type === 'QUERY_RESP' &&
      message.payload?.queryId === live.id &&
      message.payload?.code !== undefined
    ) {
      overflowResults += 1;
      lastOverflowAt = Date.now();
    }
  });
  live.subscribe((results) => {
    keys = new Set(results.map((row) => row._key)).size;
    rows = results.map((row) => {
      const { _key, ...value } = row;
      return [String(_key), value];
    });
  });
}

function openSearch(query: string, limit: number): void {
  search = client.searchSubscribe(mapName, query, { limit });
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- counts responses through the handle's private response handler
  const live = search as any;
  const handleSearchResponse = live.handleSearchResponse.bind(live);
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- a deserialized frame
  live.handleSearchResponse = (message: any) => {
    // The handler is also called for responses of other request ids.
    if (message?.payload?.requestId === live.subscriptionId) {
      searchResults += 1;
      lastSearchResultAt = Date.now();
    }
    return handleSearchResponse(message);
  };
  search.subscribe(() => undefined);
}

const timer = setInterval(() => {
  if (!watching && client.getConnectionState() === SyncState.CONNECTED) watch();
  report();
}, 100);

process.on(
  'message',
  (message: {
    type?: string;
    query?: string;
    limit?: number;
    sentinel?: string;
    maxMs?: number;
    busyMs?: number;
    periodMs?: number;
    off?: boolean;
  }) => {
    switch (message?.type) {
      case 'search':
        openSearch(message.query ?? '', message.limit ?? 1_000);
        break;
      case 'block': {
        const deadline = Date.now() + (message.maxMs ?? 30_000);
        let found = false;
        while (Date.now() < deadline) {
          if (fs.existsSync(message.sentinel!)) {
            found = true;
            break;
          }
          busyFor(5);
        }
        process.send?.({ type: 'unblocked', found });
        break;
      }
      case 'hog':
        if (hog) clearInterval(hog);
        hog = null;
        if (!message.off) {
          const busyMs = message.busyMs ?? 8;
          hog = setInterval(() => busyFor(busyMs), message.periodMs ?? 10);
        }
        break;
      case 'dump':
        process.send?.({
          type: 'rows',
          rows,
          searchRows: (search?.getResults() ?? []).map((r) => r.key).sort(),
        });
        break;
      case 'exit':
        clearInterval(timer);
        if (hog) clearInterval(hog);
        void client.close().finally(() => process.exit(0));
        break;
      default:
        break;
    }
  },
);

void client.start();
