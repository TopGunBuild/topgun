# Getting Started with TopGun

Get from zero to real-time data syncing in under 2 minutes.

## Prerequisites

- **Docker Desktop** 4.x or later ([download](https://www.docker.com/products/docker-desktop/))
- **Node.js** 18 or later ([download](https://nodejs.org/)) — required for the SDK path
- **pnpm** (optional) — only needed if you are working inside the TopGun monorepo

---

## Docker Path (60 seconds)

The fastest way to see TopGun in action. One command starts the server, seeds it with 170 demo records, and opens the admin panel.

### 1. Start the stack

```bash
docker compose --profile getting-started up
```

Expected output (abridged):

```
[+] Running 3/3
 ✔ server        Started
 ✔ seed          Started
 ✔ admin-ui      Started

seed-1      | Connecting to ws://server:8080/ws...
seed-1      | Seeding tasks... 50/50 done
seed-1      | Seeding users... 20/20 done
seed-1      | Seeding messages... 100/100 done
seed-1      | Seeding complete: 170 records across 3 maps
seed-1 exited with code 0
```

The seed container exits automatically once the data is loaded. The server and admin panel stay running.

### 2. Verify the server is healthy

```bash
curl http://localhost:8080/health
```

Expected output:

```json
{"state":"ready","connections":0,"in_flight":0,"uptime_secs":12}
```

### 3. Open the admin panel

Navigate to [http://localhost:3001](http://localhost:3001) in your browser.

You should see the **Maps** section listing `tasks`, `users`, and `messages` with record counts.

> **Note on persistence:** The Docker server binary uses an in-memory data store (`NullDataStore`). Demo data lives only in memory and is lost when the containers restart. Restart the containers to re-seed. Production deployments use PostgreSQL persistence — see [Production Deployment](../docs/guides/) for details.

### 4. Shut down and clean up

```bash
docker compose --profile getting-started down -v
```

This removes all containers and volumes cleanly. No orphan resources remain.

---

## SDK Path (5 minutes)

Connect your own Node.js script to the running server and write + read data in real time.

**Prerequisite:** Complete the Docker Path above so you have a server running on `localhost:8080`.

### 1. Install dependencies

```bash
npm install @topgunbuild/client @topgunbuild/core
```

### 2. Create `quickstart.ts`

Copy this file in full — the `MemoryAdapter` is required because `TopGunClient` needs a storage adapter, and IndexedDB is unavailable in Node.js. In a browser app you would use `IDBAdapter` from `@topgunbuild/adapters` instead.

```typescript
import { TopGunClient } from '@topgunbuild/client';
import type { IStorageAdapter, OpLogEntry } from '@topgunbuild/client';

// Minimal in-memory adapter for Node.js (no IndexedDB)
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
  async markOpsSynced(lastId: number): Promise<void> { this.opLog.forEach(e => { if (e.id <= lastId) e.synced = 1; }); }
  async getAllKeys(): Promise<string[]> { return Array.from(this.data.keys()); }
}

async function main(): Promise<void> {
  const client = new TopGunClient({
    serverUrl: 'ws://localhost:8080/ws',
    storage: new MemoryAdapter(),
  });

  await client.connect();
  console.log('Connected');

  const tasks = client.getMap<{ title: string; status: string }>('tasks');

  // Subscribe to live changes before writing
  const query = client.query('tasks', {});
  query.subscribe((results) => {
    const mine = results.find(r => r.key === 'quickstart-task-1');
    if (mine) console.log('Subscription update:', mine);
  });

  // Write a record
  tasks.put('quickstart-task-1', { title: 'My first TopGun task', status: 'todo' });
  console.log('Written: quickstart-task-1');

  // Read it back
  await new Promise(resolve => setTimeout(resolve, 500));
  const result = tasks.get('quickstart-task-1');
  console.log('Read back:', result);

  await new Promise(resolve => setTimeout(resolve, 2000));
  query.unsubscribe();
  client.disconnect();
}

main().catch(err => { console.error(err); process.exit(1); });
```

### 3. Run it

```bash
npx tsx quickstart.ts
```

Expected output:

```
Connected
Written: quickstart-task-1
Read back: { title: 'My first TopGun task', status: 'todo' }
Subscription update: { key: 'quickstart-task-1', record: { value: { title: 'My first TopGun task', status: 'todo' }, timestamp: { millis: ..., counter: 0, nodeId: '...' } } }
```

> **Tip:** If you started the Docker stack with demo data, running this quickstart will add your record alongside the 50 seeded task records. You can verify all 51 records in the admin panel at [http://localhost:3001](http://localhost:3001).

---

## CLI Path — Coming Soon

A one-command CLI experience is planned but depends on upcoming tooling (TODO-200). Check back soon.

---

## What Just Happened

When you called `tasks.put(...)`, TopGun:

1. **Wrote locally first.** The record was stored in the in-memory `LWWMap` immediately — zero latency, no network round-trip.
2. **Assigned an HLC timestamp.** A Hybrid Logical Clock (HLC) timestamp was attached: `{ millis, counter, nodeId }`. HLC timestamps combine wall-clock time with a monotonic counter to guarantee causality across distributed nodes without requiring clock synchronization.
3. **Synced to the server.** The `SyncEngine` serialized the operation as MsgPack, sent it over WebSocket, and received an `OP_ACK` confirmation.
4. **Broadcasted to subscribers.** The server broadcast the write to all clients subscribed to the `tasks` map — including your own `query.subscribe()` callback.

This is the **local-first** model: reads and writes never wait for the network. Conflicts are resolved automatically using Last-Write-Wins (LWW) merge: the record with the highest HLC timestamp wins.

---

## Next Steps

| Topic | Link |
|-------|------|
| React hooks | [packages/react/README.md](../packages/react/README.md) |
| Storage adapters | [packages/adapters/README.md](../packages/adapters/README.md) |
| CRDT concepts | [docs/guides/](./guides/) |
| Production deployment | [docs/guides/](./guides/) |
| API reference | Source types in `packages/client/src/TopGunClient.ts` |
