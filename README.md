# TopGun

[![License](https://img.shields.io/github/license/TopGunBuild/topgun)](LICENSE) [![CI](https://img.shields.io/github/actions/workflow/status/TopGunBuild/topgun/rust.yml?branch=main&label=CI)](.github/workflows/rust.yml) [![npm](https://img.shields.io/npm/v/@topgunbuild/client)](https://www.npmjs.com/package/@topgunbuild/client) [![Discord](https://img.shields.io/badge/discord-join-5865F2?logo=discord&logoColor=white)](https://discord.gg/NDpMG4dmJu) [![GitHub Stars](https://img.shields.io/github/stars/TopGunBuild/topgun?style=social)](https://github.com/TopGunBuild/topgun)

> **v2.0 — single-node stable; cluster features in progress.** The TypeScript client and Rust server APIs are stable for single-node deployments. Cluster-mode capabilities (Raft-backed distributed locks, cross-node replication) are being actively developed.

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

### Drop-in (5 minutes)

Scaffold a working app with one command:

```bash
npx create-topgun-app my-app
cd my-app && pnpm install && pnpm dev
```

This boots a Vite app with a working LWW-Map todo demo. For the backend, in a separate terminal:

```bash
docker compose up server
```

Single-command bring-up — Postgres + server start together. No env vars, no auth required — meant for local development and exploring the API.

### Production

Production deployments need Postgres for durability and explicit auth configuration. The TopGun v2 server is **single-node stable**; multi-node Raft consensus is on the roadmap (see [`/docs/roadmap`](https://topgun.build/docs/roadmap)).

**1. Postgres setup**

Provision a Postgres database and set the DSN:

```bash
export DATABASE_URL=postgres://user:pass@host:5432/topgun
```

The server applies its schema on first boot. Retention policy (op-log truncation, snapshot cadence) is configured per-map.

**2. Server-side env vars**

```bash
export JWT_SECRET=<random-32-byte-secret>     # Required for signed tokens
export TOPGUN_NO_AUTH=0                       # 0 = enforce auth, 1 = dev-only bypass
# ACL config: see /docs/security for per-map and per-op rules
```

**3. Single-node deployment**

Single-node is fully consistent and production-ready for workloads that fit one server:

```bash
docker compose up --build
# Or run the Rust server directly
cargo run --bin test-server --release
```

**4. Multi-node cluster** *(on roadmap — Raft consensus)*

Multi-node deployments today use partition-routing without Raft consensus. **Cluster-safe distributed locks and split-brain protection require Raft, which is on the roadmap.** See [`/docs/roadmap`](https://topgun.build/docs/roadmap) for status.

**5. Backup and restore** *(coming with TODO-139)*

Operational backup/restore tooling is planned. Today, take Postgres-level backups (`pg_dump`) and restore via standard Postgres workflows.

**6. Monitoring hooks** *(coming)*

Prometheus metrics endpoint and structured-log hooks are planned.

**Client wiring (both paths):**

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

## Performance

Measured on Apple M1 Max, 200 concurrent WebSocket connections, using the in-process load harness (`packages/server-rust/benches/load_harness/`):

| Mode | Throughput | Latency |
|------|-----------|---------|
| Fire-and-forget | **483K ops/sec** | — |
| Fire-and-wait | **~37K ops/sec** | **1.5ms p50** |

> Numbers are from an in-process load harness on Apple M1 Max with 200 concurrent connections. Performance on your hardware will differ. See [`packages/server-rust/benches/load_harness/baseline.json`](packages/server-rust/benches/load_harness/baseline.json) for CI thresholds and [`packages/server-rust/docs/profiling/FLAMEGRAPH_ANALYSIS.md`](packages/server-rust/docs/profiling/FLAMEGRAPH_ANALYSIS.md) for methodology.

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

## Community

- **Discord**: [discord.gg/NDpMG4dmJu](https://discord.gg/NDpMG4dmJu) — ask questions, share what you're building
- **GitHub Discussions**: [github.com/topgunbuild/topgun/discussions](https://github.com/topgunbuild/topgun/discussions) — longer-form Q&A and proposals
- **GitHub Issues**: [github.com/topgunbuild/topgun/issues](https://github.com/topgunbuild/topgun/issues) — bug reports and feature requests

## TopGun v1

Looking for the original gun.js TypeScript port? See the [`legacy-v1`](https://github.com/TopGunBuild/topgun/tree/legacy-v1) branch (unmaintained).

---

Built by [Ivan Kalashnik](https://github.com/ivkan)
