/**
 * A TopGunClient that holds a live query of one map, run in a process of its
 * own by helpers/client-subscriber-process.js (see there for how it loads).
 *
 * Started with `fork(…/client-subscriber-process.js, [port, mapName])`. Every
 * 100 ms it tells the parent `{ type: 'status', keys, state, changes }`: the
 * number of distinct keys its live view holds, its connection state, and every
 * connection-state transition since it first reached CONNECTED.
 * `{ type: 'exit' }` closes the client and ends the process.
 */
import { TopGunClient, SyncState } from '@topgunbuild/client';
import type { StateChangeEvent } from '@topgunbuild/client';

import { MemoryStorageAdapter } from './memory-storage';
import { createTestToken } from './test-client';

const [port, mapName] = process.argv.slice(2);
const token = createTestToken('client-subscriber', ['ADMIN']);
const client = new TopGunClient({
  serverUrl: `ws://localhost:${port}/ws`,
  storage: new MemoryStorageAdapter(),
  auth: { getToken: async () => token },
});

const changes: StateChangeEvent[] = [];
let keys = 0;
let watching = false;

const report = () =>
  process.send?.({
    type: 'status',
    keys,
    state: client.getConnectionState(),
    changes: changes.map((c) => ({ from: c.from, to: c.to })),
  });
const timer = setInterval(() => {
  if (!watching && client.getConnectionState() === SyncState.CONNECTED) {
    watching = true;
    client.onConnectionStateChange((event) => changes.push(event));
    client.query<{ v: number }>(mapName, {}).subscribe((rows) => {
      keys = new Set(rows.map((row) => row._key)).size;
    });
  }
  report();
}, 100);

process.on('message', (message: { type?: string }) => {
  if (message?.type !== 'exit') return;
  clearInterval(timer);
  void client.close().finally(() => process.exit(0));
});

void client.start();
