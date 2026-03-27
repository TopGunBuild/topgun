/**
 * TopGun Quickstart Example
 *
 * Demonstrates connecting to a running TopGun server, writing a record,
 * reading it back via a query, and subscribing to live changes.
 *
 * Prerequisites:
 *   1. Start the server: docker compose --profile getting-started up
 *   2. Install dependencies: npm install @topgunbuild/client @topgunbuild/core
 *   3. Run this file: npx tsx examples/quickstart.ts
 */

import { TopGunClient } from '@topgunbuild/client';
import type { IStorageAdapter, OpLogEntry } from '@topgunbuild/client';

// ---------------------------------------------------------------------------
// Minimal in-memory storage adapter — required by TopGunClient.
// Stores everything in Maps; no persistence. Copy this into your project if
// you need a Node.js-compatible adapter without IndexedDB.
// ---------------------------------------------------------------------------
class MemoryAdapter implements IStorageAdapter {
  private data = new Map<string, unknown>();
  private meta = new Map<string, unknown>();
  private opLog: (OpLogEntry & { id: number })[] = [];
  private nextId = 1;

  async initialize(_dbName: string): Promise<void> {}
  async close(): Promise<void> { this.data.clear(); this.meta.clear(); this.opLog = []; }
  async get<V>(key: string): Promise<V | undefined> { return this.data.get(key) as V; }
  async put(key: string, value: unknown): Promise<void> { this.data.set(key, value); }
  async remove(key: string): Promise<void> { this.data.delete(key); }
  async getMeta(key: string): Promise<unknown> { return this.meta.get(key); }
  async setMeta(key: string, value: unknown): Promise<void> { this.meta.set(key, value); }
  async batchPut(entries: Map<string, unknown>): Promise<void> { entries.forEach((v, k) => this.data.set(k, v)); }
  async appendOpLog(entry: Omit<OpLogEntry, 'id'>): Promise<number> {
    const id = this.nextId++;
    this.opLog.push({ ...entry, id } as OpLogEntry & { id: number });
    return id;
  }
  async getPendingOps(): Promise<OpLogEntry[]> { return this.opLog.filter(e => e.synced === 0); }
  async markOpsSynced(lastId: number): Promise<void> { this.opLog.forEach(e => { if ((e as any).id <= lastId) e.synced = 1; }); }
  async getAllKeys(): Promise<string[]> { return Array.from(this.data.keys()); }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------
async function main(): Promise<void> {
  const client = new TopGunClient({
    serverUrl: 'ws://localhost:8080/ws',
    storage: new MemoryAdapter(),
  });

  await client.connect();
  console.log('Connected to TopGun server at ws://localhost:8080/ws');

  // Get a reference to the tasks map
  const tasks = client.getMap<{ title: string; status: string }>('tasks');

  // Subscribe to changes before writing so we see the update arrive
  const query = client.query<{ title: string; status: string }>('tasks', {});
  query.subscribe((results) => {
    const mine = results.find(r => r.key === 'quickstart-task-1');
    if (mine) {
      console.log('Subscription update received:', mine);
    }
  });

  // Write a record to the tasks map
  tasks.put('quickstart-task-1', { title: 'My first TopGun task', status: 'todo' });
  console.log('Written: quickstart-task-1 = { title: "My first TopGun task", status: "todo" }');

  // Read it back via a point query
  await new Promise(resolve => setTimeout(resolve, 500));
  const result = tasks.get('quickstart-task-1');
  console.log('Read back:', result);

  // Keep the process alive for 2 seconds to receive the subscription update
  await new Promise(resolve => setTimeout(resolve, 2000));

  query.unsubscribe();
  client.disconnect();
}

main().catch((err) => {
  console.error('Quickstart failed:', err);
  process.exit(1);
});
