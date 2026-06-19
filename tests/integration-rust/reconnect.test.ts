/**
 * Live reconnect-honesty tests (F2 / TODO-414 + TODO-429) against the Rust server.
 *
 * These exercise the real @topgunbuild/client SDK (TopGunClient → SyncEngine →
 * SingleServerProvider) over real WebSockets, proving the end-to-end behavior that
 * the unit tests cannot: recovery against an actual server socket.
 *
 *  1. server-after-client: a client constructed while NOTHING is listening keeps
 *     retrying and connects + syncs once the server comes up later (the exact
 *     TODO-414 MCP-smoke repro). With the old default (give up after 10 attempts)
 *     the client would be permanently stuck and this test would time out.
 *  2. prolonged outage beyond the old 10-attempt budget: the client stays down for
 *     several seconds of fast retries, far past where a capped-at-10 client would
 *     have terminally given up, then still recovers when the server appears.
 *  3. server bounce under a live client: an established client survives the server
 *     being killed and restarted on the same port (the deploy/crash scenario).
 */

import * as net from 'net';

import { TopGunClient, SyncState } from '@topgunbuild/client';
import type { IStorageAdapter, OpLogEntry } from '@topgunbuild/client';

import {
  spawnRustServer,
  createRustTestClient,
  createTestToken,
  completeMerkleSync,
  waitUntil,
  SpawnedServer,
} from './helpers';

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

/** Reserve an ephemeral port, then release it so the server can bind it later. */
function findFreePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const srv = net.createServer();
    srv.unref();
    srv.on('error', reject);
    srv.listen(0, '127.0.0.1', () => {
      const addr = srv.address();
      if (addr && typeof addr === 'object') {
        const { port } = addr;
        srv.close(() => resolve(port));
      } else {
        srv.close(() => reject(new Error('Could not determine free port')));
      }
    });
  });
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
    await waitMs(100);
  }
  throw new Error(`Timed out waiting for ${state}; last state = ${client.getConnectionState()}`);
}

function makeClient(port: number): TopGunClient {
  const token = createTestToken('reconnect-user', ['ADMIN']);
  return new TopGunClient({
    serverUrl: `ws://localhost:${port}/ws`,
    storage: new MemoryStorageAdapter(),
    auth: { getToken: async () => token },
    // initialDelayMs also derives the per-attempt connection timeout
    // (reconnectDelayMs * 5). 200ms → a 1s handshake timeout (ample for a local WS
    // upgrade) and, since a failed connect only closes via that timeout, a retry
    // roughly every ~1.2s — brisk enough to exceed 10 attempts within the outage
    // window without hammering.
    backoff: { initialDelayMs: 200, maxDelayMs: 400, jitter: true },
  });
}

describe('Integration: resilient reconnect (F2 / TODO-414 + TODO-429)', () => {
  let server: SpawnedServer | null = null;
  let client: TopGunClient | null = null;

  afterEach(async () => {
    if (client) {
      await client.close().catch(() => {});
      client = null;
    }
    if (server) {
      await server.cleanup().catch(() => {});
      server = null;
    }
  });

  test('client started BEFORE the server connects + syncs once the server comes up', async () => {
    const port = await findFreePort();

    // Construct the client while nothing is listening — it begins retrying immediately.
    client = makeClient(port);
    await client.start();

    // Confirm it is NOT connected yet (server does not exist).
    await waitMs(500);
    expect(client.getConnectionState()).not.toBe(SyncState.CONNECTED);

    // Bring the server up on the same port; the client must reconnect on its own.
    server = await spawnRustServer({ port });
    await waitForState(client, SyncState.CONNECTED, 20_000);
    expect(client.getConnectionState()).toBe(SyncState.CONNECTED);

    // And it actually SYNCS: a local write must reach the server. Verify end-to-end
    // with an independent raw client that Merkle-syncs the map and finds the key.
    client.getMap<string, { name: string }>('reconnect-map').set('k1', { name: 'after-server' });

    const verifier = await createRustTestClient(server.port, {
      userId: 'verifier',
      roles: ['ADMIN'],
    });
    await verifier.waitForMessage('AUTH_ACK', 10_000);
    try {
      await waitUntil(async () => {
        const records = await completeMerkleSync(verifier, 'reconnect-map');
        return records.has('k1');
      }, 10_000);
    } finally {
      verifier.close();
    }
  }, 45_000);

  test('survives a prolonged outage well past the old 10-attempt budget, then recovers', async () => {
    const port = await findFreePort();

    client = makeClient(port);
    await client.start();

    // Stay down for ~14s of ~0.8s retries — that is well over 10 attempts, far
    // past where the historical maxReconnectAttempts=10 would have terminally
    // given up (it would emit ERROR and never recover).
    await waitMs(14000);
    expect(client.getConnectionState()).not.toBe(SyncState.CONNECTED);
    // It must NOT have transitioned to the terminal ERROR state (no silent give-up).
    expect(client.getConnectionState()).not.toBe(SyncState.ERROR);

    server = await spawnRustServer({ port });
    await waitForState(client, SyncState.CONNECTED, 20_000);
    expect(client.getConnectionState()).toBe(SyncState.CONNECTED);
  }, 60_000);

  test('recovers after the server is bounced under a live client', async () => {
    const port = await findFreePort();

    // Start server first, connect the client.
    server = await spawnRustServer({ port });
    client = makeClient(port);
    await client.start();
    await waitForState(client, SyncState.CONNECTED, 20_000);

    // Kill the server (simulating a deploy / crash) and restart it on the same port.
    await server.cleanup();
    server = null;
    await waitForState(client, SyncState.DISCONNECTED, 15_000).catch(() => {
      // Either DISCONNECTED or a brief CONNECTING churn is acceptable here; the
      // hard requirement is that it is NOT CONNECTED while the server is down.
      expect(client!.getConnectionState()).not.toBe(SyncState.CONNECTED);
    });
    expect(client.getConnectionState()).not.toBe(SyncState.CONNECTED);

    server = await spawnRustServer({ port });
    await waitForState(client, SyncState.CONNECTED, 20_000);
    expect(client.getConnectionState()).toBe(SyncState.CONNECTED);
  }, 60_000);
});
