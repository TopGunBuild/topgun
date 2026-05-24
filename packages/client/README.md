# @topgunbuild/client

[![npm](https://img.shields.io/npm/v/@topgunbuild/client)](https://www.npmjs.com/package/@topgunbuild/client) [![License](https://img.shields.io/npm/l/@topgunbuild/client)](https://github.com/TopGunBuild/topgun/blob/main/LICENSE)

Browser and Node.js client for [TopGun](https://topgun.build) — build real-time apps that work offline. Local writes are instant and survive disconnects; reconnecting clients sync seamlessly with automatic conflict resolution.

## Install

```bash
npm install @topgunbuild/client @topgunbuild/adapters
```

## Quickstart

```typescript
import { TopGunClient } from '@topgunbuild/client';
import { IDBAdapter } from '@topgunbuild/adapters';

const client = new TopGunClient({
  serverUrl: 'ws://localhost:8080',   // optional — omit for local-only
  storage: new IDBAdapter('my-app'),
});
client.start();

// Reactive map — fires on every change
const todos = client.getMap('todos');
todos.subscribe((entries) => render(entries));

// Optimistic write — applies locally, syncs in background
todos.set('todo-1', { text: 'Buy milk', done: false });
```

## API at a glance

### Construction

```typescript
new TopGunClient({
  storage: new IDBAdapter('my-app'),  // required
  serverUrl: 'ws://...',              // optional — single-server mode
  // or
  cluster: { seeds: ['ws://node1', 'ws://node2'] },  // optional — cluster mode
});

client.start();      // initialize storage (non-blocking)
await client.close(); // graceful shutdown
```

Omit `serverUrl` and `cluster` to run **local-only** (no network, IndexedDB only).

### Reactive primitives

| Primitive | Use for | Subscribe callback |
|-----------|---------|--------------------|
| `client.getMap(name)` | Last-write-wins key/value | `(entries: [K, V][]) => void` |
| `client.getORMap(name)` | Multimap, concurrent adds | `(entries: [K, V[]][]) => void` |
| `client.query(name, filter)` | Filtered live view | `(results: T[]) => void` |
| `client.topic(name)` | Fire-and-forget pub/sub | `(data, ctx) => void` |
| `client.getPNCounter(name)` | Distributed counter | `(value: number) => void` |

### Filtered queries

```typescript
import { Predicates } from '@topgunbuild/client';

const open = client.query('todos', {
  where: { done: false },
  // or use Predicates for richer filters:
  predicate: Predicates.and(
    Predicates.equal('done', false),
    Predicates.greaterThan('priority', 5),
  ),
  sort: { createdAt: 'desc' },
  limit: 20,
});

open.subscribe((results) => render(results));
open.onDelta((changes) => log(changes));   // add / update / remove deltas
```

### Distributed locks

```typescript
const lock = client.getLock('migration');
const acquired = await lock.lock(30_000);   // TTL in ms
if (acquired) {
  try { await runMigration(); }
  finally { await lock.unlock(); }
}
```

### Hybrid search (BM25 + filters)

```typescript
const results = await client.search('articles', 'distributed crdts', {
  limit: 10,
  minScore: 0.5,
});
```

## React users

Use [`@topgunbuild/react`](https://www.npmjs.com/package/@topgunbuild/react) for hooks (`useQuery`, `useMap`, `useMutation`, ...).

## Documentation

- Full docs: [topgun.build/docs](https://topgun.build/docs)
- Live demo: [demo.topgun.build](https://demo.topgun.build)
- GitHub: [TopGunBuild/topgun](https://github.com/TopGunBuild/topgun)

## License

Apache-2.0
