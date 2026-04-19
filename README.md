# TopGun

> **Alpha** — API may change

Hybrid offline-first in-memory data grid. Zero-latency reads and writes via local CRDTs, real-time sync via WebSockets, durable storage on your own infrastructure.

TopGun v2 is a complete rewrite. It's not a port — it's a new architecture designed for production workloads.

**[Live Demo](https://demo.topgun.build/)** — try real-time CRDT sync in your browser

## Key features

- **Local-first**: Data lives in memory. Reads and writes never wait for network.
- **Offline support**: Changes persist to IndexedDB and sync when reconnected.
- **CRDT conflict resolution**: LWW-Map and OR-Map with Hybrid Logical Clocks.
- **Merkle tree sync**: Efficient delta synchronization — only changed data moves over the wire.
- **Pluggable storage**: PostgreSQL for server, IndexedDB for client, or bring your own adapter.
- **Cluster-ready**: Server-side partitioning, pub/sub, and distributed locks (single-node stable; cluster-mode uses partition-routing — Raft-backed cluster locks in progress).
- **Rust server, TypeScript client**: Type-safe SDK with a high-performance Rust backend.

## Quick start

```bash
npm install @topgunbuild/client @topgunbuild/adapters @topgunbuild/react
```

```typescript
import { TopGunClient } from '@topgunbuild/client';
import { IDBAdapter } from '@topgunbuild/adapters';

const adapter = new IDBAdapter();
const client = new TopGunClient({
  serverUrl: 'ws://localhost:8080',
  storage: adapter,
});

client.start();

// Write data (instant, works offline)
const todos = client.getMap('todos');
todos.set('todo-1', {
  id: 'todo-1',
  text: 'Buy milk',
  done: false,
});

// Read data
const todo = todos.get('todo-1');

// Subscribe to changes via live queries
// See useQuery hook for React integration
```

With React:

```tsx
import { TopGunProvider, useQuery, useClient } from '@topgunbuild/react';

function App() {
  return (
    <TopGunProvider client={client}>
      <TodoList />
    </TopGunProvider>
  );
}

function TodoList() {
  const client = useClient();
  const { data, loading } = useQuery('todos');

  if (loading) return <div>Loading...</div>;

  const toggleTodo = (todo) => {
    const todosMap = client.getMap('todos');
    todosMap.set(todo.id, { ...todo, done: !todo.done });
  };

  return (
    <ul>
      {data.map((todo) => (
        <li key={todo.id} onClick={() => toggleTodo(todo)}>
          {todo.text}
        </li>
      ))}
    </ul>
  );
}
```

## Documentation

Full docs: [topgun.build/docs](https://topgun.build/docs)

Specifications in this repo:
- [Master Architecture](specifications/00_MASTER_ARCHITECTURE.md)
- [System Architecture](specifications/01_SYSTEM_ARCHITECTURE.md)
- [CRDT & Data Structures](specifications/02_DATA_STRUCTURES_CRDT.md)
- [Synchronization Protocol](specifications/03_SYNCHRONIZATION_PROTOCOL.md)

## Packages

| Package | Description |
|---------|-------------|
| `@topgunbuild/core` | CRDTs, Hybrid Logical Clock, Merkle trees, message schemas |
| `@topgunbuild/client` | Browser/Node.js SDK with IndexedDB persistence |
| `server-rust` | Rust WebSocket server (axum), clustering, PostgreSQL |
| `@topgunbuild/react` | React hooks: `useQuery`, `useMap`, `useMutation`, `useTopic` |
| `@topgunbuild/adapters` | Storage adapters: IndexedDB |
| `@topgunbuild/adapter-better-auth` | Better Auth integration |

## Running locally

```bash
# Start server with Postgres
docker compose up --build

# Or run the Rust server directly
cargo run --bin test-server --release

# Or run the example app
cd examples/notes-app
pnpm install
pnpm start
```

## Performance Testing

### Quick Smoke Test
```bash
pnpm test:k6:smoke
```

### Full Throughput Benchmark
```bash
pnpm test:k6:throughput
```

### Micro-Benchmarks (CRDT operations)
```bash
pnpm --filter @topgunbuild/core bench
```

See [tests/benchmark/README.md](tests/benchmark/README.md) for details.

## TopGun v1

Looking for the original gun.js TypeScript port? See the [`legacy-v1`](https://github.com/TopGunBuild/topgun/tree/legacy-v1) branch (unmaintained).

---

Built by [Ivan Kalashnik](https://github.com/ivkan)
