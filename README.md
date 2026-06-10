# TopGun

[![License](https://img.shields.io/github/license/TopGunBuild/topgun)](LICENSE) [![CI](https://img.shields.io/github/actions/workflow/status/TopGunBuild/topgun/rust.yml?branch=main&label=CI)](.github/workflows/rust.yml) [![Docker](https://github.com/TopGunBuild/topgun/actions/workflows/docker.yml/badge.svg)](https://github.com/TopGunBuild/topgun/actions/workflows/docker.yml) [![npm](https://img.shields.io/npm/v/@topgunbuild/client)](https://www.npmjs.com/package/@topgunbuild/client) [![Discord](https://img.shields.io/badge/discord-join-5865F2?logo=discord&logoColor=white)](https://discord.gg/NDpMG4dmJu) [![GitHub Stars](https://img.shields.io/github/stars/TopGunBuild/topgun?style=social)](https://github.com/TopGunBuild/topgun)

> **v2.0 — single-node stable; cluster features in progress.** The TypeScript client and Rust server APIs are stable for single-node deployments. Cluster-mode capabilities (Raft-backed distributed locks, cross-node replication) are being actively developed.

Build real-time apps that work offline. Local writes are instant and survive disconnects; reconnecting clients sync seamlessly with automatic conflict resolution. Self-host today with the embedded backend, or wire up Postgres when you need it. Apache-2.0, Rust server, TypeScript client. AI agents talk to your data natively through MCP.

TopGun v2 is a complete rewrite. It's not a port — it's a new architecture designed for production workloads.

**[Live Demo](https://demo.topgun.build/)** — try it in your browser

## Key features

- **Local-first**: Data lives in memory. Reads and writes never wait for network.
- **Offline support**: Changes persist to IndexedDB and sync when reconnected.
- **Conflict-free merges**: Automatic conflict resolution via LWW-Map and OR-Map, with causality tracked by logical clocks.
- **Efficient delta sync**: Only changed data moves over the wire, via Merkle tree comparison.
- **Pluggable storage**: Embedded redb by default; Postgres optional for production; bring your own adapter.
- **Cluster-ready**: Server-side partitioning, pub/sub, and distributed locks (single-node stable; cluster-mode uses partition-routing — Raft-backed cluster locks in progress).
- **Rust server, TypeScript client**: Type-safe SDK with a high-performance Rust backend.
- **AI-native**: AI agents read and mutate your live data natively over MCP — the bundled `@topgunbuild/mcp-server` exposes your maps to Claude Desktop, Cursor, or any MCP client. Query, mutate, search, and subscribe without leaving your agent workflow.
- **Hybrid search**: Real-time hybrid search in one query — exact + full-text (BM25) + semantic vector search, RRF-fused, with live subscriptions. Pluggable embeddings (local Ollama or any OpenAI-compatible endpoint). Built-in HNSW vector database; works with your existing map data. Covers vector search, semantic search, hybrid search, RAG pipelines, and embeddings out of the box.

## Quick start

### Drop-in (5 minutes)

Scaffold a working app with one command:

```bash
npx create-topgun-app my-app
cd my-app && pnpm install && pnpm dev
```

This boots a Vite app with a working LWW-Map todo demo. For the backend, in a separate terminal:

```bash
npx @topgunbuild/server
```

No Rust toolchain required — this downloads a prebuilt binary for your platform and starts the server instantly. The server runs on `ws://localhost:8080` with embedded storage at `./topgun.redb` — writes survive restart, no Postgres required. Stop with `Ctrl-C`, restart, and your data is still there. Backup is a single-file copy.

**Contributor / monorepo path:**

```bash
pnpm start:server
```

`pnpm start:server` uses a prebuilt binary when present and falls back to `cargo run --bin topgun-server --release` for contributors with the Rust toolchain.

For users who prefer containers, `docker compose up server` works too — it spins up Postgres alongside the server in one command.

### Pull pre-built image (Docker)

If you have Docker installed, skip the build entirely:

```bash
docker pull ghcr.io/topgunbuild/topgun-server:latest
docker run -p 8080:8080 -e TOPGUN_NO_AUTH=1 ghcr.io/topgunbuild/topgun-server:latest
```

The image is multi-arch (linux/amd64 + linux/arm64) and is rebuilt on every push to `main`. For tagged releases use `:v2.0.0` (or `:2`, `:2.0`) instead of `:latest`.

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
# Or start the server directly (no Rust toolchain required)
npx @topgunbuild/server
# Contributor / monorepo path (has cargo):
# cargo run --bin topgun-server --release
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

const client = new TopGunClient({
  serverUrl: 'ws://localhost:8080',  // optional — omit for local-only
  storage: new IDBAdapter('my-app'),
});
client.start();

// Reactive read — fires immediately and on every change
const todos = client.getMap('todos');
todos.subscribe((entries) => render(entries));

// Optimistic write — applies locally, syncs in background
todos.set('todo-1', { text: 'Buy milk', done: false });
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

### Hybrid search (vector + full-text + exact)

TopGun supports tri-hybrid search — exact key lookup, BM25 full-text, and semantic vector search — fused with RRF into a single ranked result. Subscriptions update in real time as data changes.

```tsx
import { useHybridSearch } from '@topgunbuild/react';

function DocSearch({ query }: { query: string }) {
  const { results, loading } = useHybridSearch('docs', query, {
    methods: ['fullText', 'semantic'],  // HybridSearchMethod: 'exact' | 'fullText' | 'semantic'
    k: 10,
    minScore: 0.3,
  });

  if (loading) return <p>Searching…</p>;

  return (
    <ul>
      {results.map((r) => (
        <li key={r.key}>
          {r.key} — score {r.score.toFixed(3)}
          <small>
            {' '}(BM25: {r.methodScores?.fullText?.toFixed(3)},
            semantic: {r.methodScores?.semantic?.toFixed(3)})
          </small>
        </li>
      ))}
    </ul>
  );
}
```

> **Semantic search** requires server-side embedding config (`VectorConfig` on the server — set `TOPGUN_VECTOR_INDEX_PATH` and configure your embedding provider). `fullText` and `exact` work fully offline. See the [Vector & hybrid search guide](https://topgun.build/docs/guides/vector-and-hybrid-search).

## Documentation

Full docs: [topgun.build/docs](https://topgun.build/docs)

## Packages

| Package | Description |
|---------|-------------|
| `@topgunbuild/core` | CRDTs, Hybrid Logical Clock, Merkle trees, message schemas |
| `@topgunbuild/client` | Browser/Node.js SDK with IndexedDB persistence |
| `server-rust` | Rust WebSocket server (axum), clustering, PostgreSQL |
| `core-rust` | Rust port of CRDT primitives, depended on by `server-rust` (internal) |
| `@topgunbuild/react` | React hooks: `useQuery`, `useMap`, `useMutation`, `useTopic` |
| `@topgunbuild/adapters` | Storage adapters: IndexedDB |
| `@topgunbuild/adapter-better-auth` | Better Auth integration |
| `@topgunbuild/mcp-server` | MCP server for AI agents (Claude Desktop, Cursor) |
| `@topgunbuild/schema` | Shared Zod schemas + types |
| `create-topgun-app` | Scaffold a TopGun app in one command (`npx create-topgun-app my-app`) |

## Running locally

```bash
# Start the server — no Rust toolchain required
npx @topgunbuild/server

# Or start with Docker (includes Postgres)
docker compose up --build

# Contributor / monorepo path (has cargo):
# cargo run --bin topgun-server --release

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

## Coming from X

Migrating from another sync stack? Each guide names what TopGun does not replace, up front.

| Coming from | Why migrate | Migration guide | Status |
|-------------|-------------|-----------------|--------|
| Firebase Realtime | SQL, FTS, no vendor lock-in | [Guide](https://topgun.build/docs/guides/migrating-from-firebase) | Live |
| Y.js / Automerge | Server backend + SQL queries | [Guide](https://topgun.build/docs/guides/migrating-from-yjs) | Live |
| Replicache | Open source, no SaaS invoice | [Guide](https://topgun.build/docs/guides/migrating-from-replicache) | Live |
| Supabase Realtime | Offline-first, CRDT auto-merge | [Guide](https://topgun.build/docs/guides/migrating-from-supabase-realtime) | Live |
| Liveblocks | Self-hosted option | (planned) | Q3 2026 |

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

See [packages/server-rust/benches/load_harness/](packages/server-rust/benches/load_harness/) and the [Performance tuning guide](https://topgun.build/docs/deploy/performance) for details.

## AI Agents

TopGun ships with a built-in MCP (Model Context Protocol) server, enabling AI assistants like Claude Desktop and Cursor to query and mutate your live data through natural language.

Add to `~/Library/Application Support/Claude/claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "topgun": {
      "command": "npx",
      "args": ["@topgunbuild/mcp-server"],
      "env": {
        "TOPGUN_URL": "ws://localhost:8080"
      }
    }
  }
}
```

Full guide: [topgun.build/docs/guides/mcp-server](https://topgun.build/docs/guides/mcp-server)

## Contributing

PRs welcome. See [`CONTRIBUTING.md`](CONTRIBUTING.md) for the dev workflow (fork, branch, test, PR).

All contributions require signing the [Contributor License Agreement](.github/CLA.md) — the [cla-assistant.io](https://cla-assistant.io) bot prompts automatically on PR open. Takes ~30 seconds; you sign once.

## Community

- **Discord**: [discord.gg/NDpMG4dmJu](https://discord.gg/NDpMG4dmJu) — ask questions, share what you're building
- **GitHub Discussions**: [github.com/topgunbuild/topgun/discussions](https://github.com/topgunbuild/topgun/discussions) — longer-form Q&A and proposals
- **GitHub Issues**: [github.com/topgunbuild/topgun/issues](https://github.com/topgunbuild/topgun/issues) — bug reports and feature requests

## TopGun v1

Looking for the original gun.js TypeScript port? See the [`legacy-v1`](https://github.com/TopGunBuild/topgun/tree/legacy-v1) branch (unmaintained).

---

Built by [Ivan Kalashnik](https://github.com/ivkan)
