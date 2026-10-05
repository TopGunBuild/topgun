/**
 * Child-process body for AbandonedClientExit.test.ts.
 *
 * Builds a client, brings it to the state named by the scenario, prints
 * RELEASED and returns — in most scenarios WITHOUT closing it. Whether the
 * process then exits is decided only by what the client left on the event
 * loop, so the parent can assert which of the client's timers hold a Node
 * process open and which do not.
 *
 * Nothing here opens a real socket: a real open socket is a handle of its own
 * and would keep the process alive whatever the timers do. The question this
 * fixture isolates is the timers.
 */
import { HLC, serialize } from '@topgunbuild/core';
import { TopGunClient } from '../../TopGunClient';
import { HttpSyncProvider } from '../../connection/HttpSyncProvider';
import { SyncState } from '../../SyncState';
import type { IStorageAdapter, OpLogEntry, StorageMutation } from '../../IStorageAdapter';

/** A server that accepts the connection and then never says anything. */
class SilentWebSocket {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSED = 3;

  readyState = SilentWebSocket.CONNECTING;
  binaryType = 'blob';
  onopen: (() => void) | null = null;
  onclose: ((event: { code: number }) => void) | null = null;
  onerror: ((error: unknown) => void) | null = null;
  onmessage: ((event: { data: unknown }) => void) | null = null;

  constructor(public readonly url: string) {
    // Deliberately a ref'd timer: the open must be delivered even when the
    // client under test holds nothing else on the loop, as it would be by a
    // real socket. It fires once, so it cannot hold the process by itself.
    setTimeout(() => {
      // Delivered whatever close() did in between — the late "open" a closed
      // client must not come back to life on.
      this.readyState = SilentWebSocket.OPEN;
      this.onopen?.();
    }, 0);
  }

  send(_data: unknown): void {}

  close(): void {
    this.readyState = SilentWebSocket.CLOSED;
    this.onclose?.({ code: 1000 });
  }
}

class MemoryStorage implements IStorageAdapter {
  private readonly kv = new Map<string, unknown>();
  private readonly meta = new Map<string, unknown>();
  private ops: OpLogEntry[] = [];
  private nextId = 1;

  async initialize(_dbName: string): Promise<void> {}
  async close(): Promise<void> {}
  async get(key: string): Promise<unknown> {
    return this.kv.get(key);
  }
  async put(key: string, value: unknown): Promise<void> {
    this.kv.set(key, value);
  }
  async remove(key: string): Promise<void> {
    this.kv.delete(key);
  }
  async getMeta(key: string): Promise<unknown> {
    return this.meta.get(key);
  }
  async setMeta(key: string, value: unknown): Promise<void> {
    this.meta.set(key, value);
  }
  async batchPut(entries: Map<string, unknown>): Promise<void> {
    for (const [key, value] of entries) this.kv.set(key, value);
  }
  async appendOpLog(entry: Omit<OpLogEntry, 'id'>): Promise<number> {
    const id = this.nextId++;
    this.ops.push({ ...entry, id, synced: 0 });
    return id;
  }
  async getPendingOps(): Promise<OpLogEntry[]> {
    return this.ops.filter((op) => op.synced === 0);
  }
  async markOpsSynced(lastId: number): Promise<void> {
    for (const op of this.ops) if (op.id! <= lastId) op.synced = 1;
  }
  async deleteOp(id: number): Promise<void> {
    this.ops = this.ops.filter((op) => op.id !== id);
  }
  async commitWrite(mutations: StorageMutation[], op: Omit<OpLogEntry, 'id'>): Promise<number> {
    for (const m of mutations) {
      const target = m.store === 'meta' ? this.meta : this.kv;
      if (m.type === 'remove') target.delete(m.key);
      else target.set(m.key, m.value);
    }
    return this.appendOpLog(op);
  }
  async getAllKeys(): Promise<string[]> {
    return Array.from(this.kv.keys());
  }
  async getAllMetaKeys(): Promise<string[]> {
    return Array.from(this.meta.keys());
  }
}

/**
 * Holds the process open until `ready()` is true, then lets go. The hold is the
 * fixture's own ref'd timer, so reaching the scenario's state never depends on
 * the client's timers being ref'd.
 */
function holdUntil(ready: () => boolean): Promise<void> {
  return new Promise((resolve) => {
    const hold = setInterval(() => {
      if (ready()) {
        clearInterval(hold);
        resolve();
      }
    }, 10);
  });
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function main(): Promise<void> {
  const scenario = process.env.ABANDONED_CLIENT_SCENARIO;
  (globalThis as unknown as { WebSocket: unknown }).WebSocket = SilentWebSocket;

  switch (scenario) {
    // Connected and idle: the heartbeat interval is armed and nothing answers it.
    case 'heartbeat': {
      const client = new TopGunClient({
        serverUrl: 'ws://mock.invalid',
        storage: new MemoryStorage(),
      });
      await holdUntil(() => client.getConnectionState() === SyncState.CONNECTED);
      break;
    }

    // Abandoned inside the window in which the client waits for the device credential.
    case 'device-credential-grace': {
      const client = new TopGunClient({
        serverUrl: 'ws://mock.invalid',
        storage: new MemoryStorage(),
      });
      await holdUntil(() => client.getConnectionState() === SyncState.AUTHENTICATING);
      break;
    }

    // close() returns before the socket reports "open"; the open arrives afterwards.
    case 'closed-before-open': {
      const client = new TopGunClient({
        serverUrl: 'ws://mock.invalid',
        storage: new MemoryStorage(),
      });
      await client.close();
      break;
    }

    // Cluster mode: health check, periodic partition-map refresh, per-node reconnect.
    case 'cluster': {
      const client = new TopGunClient({
        cluster: { seeds: ['ws://mock-a.invalid', 'ws://mock-b.invalid'] },
        storage: new MemoryStorage(),
      });
      void client;
      await sleep(1500);
      break;
    }

    // HTTP transport. Between polls there is no handle, so the polling interval
    // is what keeps an open client's process alive ('http-open'), and close()
    // has to be all it takes to let the process go ('http-closed').
    case 'http-open':
    case 'http-closed': {
      const body = serialize({});
      const fetchImpl = (async () => ({
        ok: true,
        status: 200,
        statusText: 'OK',
        arrayBuffer: async () =>
          body.buffer.slice(body.byteOffset, body.byteOffset + body.byteLength),
      })) as unknown as typeof fetch;
      const provider = new HttpSyncProvider({
        url: 'http://mock.invalid',
        clientId: 'abandoned',
        hlc: new HLC('abandoned'),
        pollIntervalMs: 50,
        fetchImpl,
      });
      await provider.connect();
      // Long enough for the interval to have ticked, so it is armed for certain.
      await sleep(200);
      if (scenario === 'http-closed') await provider.close();
      break;
    }

    // Negative control: a handle that IS ref'd must keep the process alive,
    // otherwise an exit in the scenarios above would prove nothing.
    case 'control-held': {
      setInterval(() => {}, 1000);
      break;
    }

    default:
      throw new Error(`unknown scenario: ${scenario}`);
  }

  process.stdout.write('RELEASED\n');
}

main().catch((err) => {
  process.stderr.write(`fixture failed: ${String(err instanceof Error ? err.stack : err)}\n`);
  process.exit(2);
});
