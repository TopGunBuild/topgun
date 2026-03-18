# TopGun — Product Capabilities

> **Vision:** "The unified real-time data platform — from browser to cluster to cloud storage."

---

## What Is TopGun

TopGun is a **unified real-time data platform** that combines:

- **In-memory data grid** (Hazelcast-class): distributed maps, clustering, partitioning, server-side compute
- **SQL query engine** (DataFusion): distributed SQL, stream processing, windowed aggregation
- **Offline-first client SDK**: zero-latency reads, CRDT conflict resolution, browser-native replicas
- **Tiered storage**: hot data in memory, warm in PostgreSQL, cold in S3/R2/GCS
- **Admin UI**: cluster topology, data explorer, query playground, settings management

### The "Client as Replica" Concept

This is the core architectural shift that defines TopGun's category:

- The client **is a cluster participant** — it holds CRDT replicas, not cached copies
- Writes happen locally first — the client doesn't ask the server for permission
- The server is the "Super Node" — holds all partitions, manages durability, coordinates replication
- Hybrid Logical Clocks (HLC) track causality across all nodes (browser + server) in a unified timeline

TopGun is neither a sync layer (PowerSync) nor a server cache (Hazelcast). It is a **distributed data platform where browsers are first-class nodes**.

### Platform Capabilities (Hazelcast Parity + Offline-First)

| Capability | TopGun | Hazelcast |
|---|---|---|
| Stateful data processing over streaming data or data at rest | DAG stream executor + entry processors | Jet engine |
| SQL over streaming and batch data sources | DataFusion (cost-based optimizer, distributed) | Calcite |
| Connector library for ingestion and serving | Connector Framework (Kafka, S3, CDC, webhooks) | 50+ connectors |
| Push updates to applications on events | Live queries + topic pub/sub | Event listeners + topics |
| Low-latency pub-sub messaging | Topic pub/sub with ordered delivery | ITopic, ReliableTopic |
| Caching patterns (read/write-through, write-behind) | Write-through (PostgreSQL) + write-behind | MapStore, MapLoader |
| **Browser-native offline-first client** | **First-class (CRDT replicas)** | **Not supported** |
| **Automatic conflict resolution** | **LWW-Map, OR-Map (CRDTs)** | **Manual (distributed locks)** |
| **Same SQL dialect client + server** | **DataFusion WASM in browser** | **Not applicable** |
| **Tiered storage (memory → DB → S3)** | **Hot/warm/cold with transparent access** | **Hot restart only** |
| **Vector + semantic search** | **Tantivy + HNSW (tri-hybrid)** | **Not built-in** |
| **Admin dashboard** | **Built-in web UI** | **Management Center (commercial)** |

---

## Who It's For

### Target Audience

Developers and teams building high-performance, interactive, data-intensive applications where **real-time state** is critical.

### Use Cases

| Category | Examples |
|----------|----------|
| **Collaborative apps** | Design tools, project management, whiteboards, document editors |
| **Real-time dashboards** | Analytics, monitoring, trading, IoT |
| **AI/ML data backbone** | Real-time feature stores, agent state, model serving cache, streaming inference pipelines |
| **Automation platforms** | Event-driven workflows (n8n, Zapier-style), CDC pipelines, webhook processing |
| **Multiplayer experiences** | Games, live polls, shared workspaces |
| **Offline-critical apps** | Field service, mobile CRM, logistics, healthcare |
| **High-frequency updates** | Chat, notifications, presence, live feeds |
| **Stream processing** | Windowed aggregations, stateful transforms, real-time ETL |

### When NOT to Use TopGun

| If you need... | Use instead | Why |
|---|---|---|
| Offline CRUD forms over Postgres | PowerSync, ElectricSQL | TopGun IS the database, not a Postgres sync layer |
| Server-side Java distributed cache | Hazelcast, Redis | TopGun targets TS/JS developers (with Rust server) |
| Collaborative rich text editing | Liveblocks, Yjs | Sequence CRDTs are a different domain |
| Backend-as-a-service | Convex, Supabase | TopGun is self-hosted infrastructure |
| P2P mesh without internet | Ditto | TopGun is server-mediated |
| Edge-native serverless state | Cloudflare Durable Objects + D1 | Different trust model (vendor-hosted edge) |
| Server-authoritative mutations (no CRDTs) | Replicache / Zero | Different trade-off: server authority (simpler security) vs CRDTs (automatic merge) |

---

## Adoption Path

TopGun doesn't require replacing your existing stack. Three tiers of adoption, from lightweight to full platform:

### Tier 1: Real-Time Layer (Recommended starting point)

Add collaborative/real-time features to an existing application. Keep your current database and API.

**Example:** A project management app keeps all data in Postgres. But the Kanban board state (card positions, assignments, comments) flows through TopGun for instant collaborative updates. The rest of the app (user management, billing, reports) stays on the existing Postgres + REST API stack.

```typescript
// Your existing Express app stays unchanged
// TopGun handles ONLY the real-time collaborative features
const topgun = new TopGunClient({ serverUrl: 'ws://localhost:8080' });
const board = topgun.getMap('kanban-board-42');

// Instant local writes + automatic sync to all connected clients
board.set('card-7', { column: 'done', assignee: 'alice', updatedAt: Date.now() });
```

### Tier 2: Cache + Sync

Use TopGun as an in-memory cache in front of an existing database. This is the Hazelcast use case.

- `PostgresDataStore` loads data from your existing Postgres tables on startup
- Reads come from in-memory (0ms)
- Writes go through to Postgres (write-through)
- TopGun adds CRDT sync, live queries, and offline support on top
- **No schema migration required** — point at existing tables

### Tier 3: Full Platform

Greenfield applications or full platform replacement. TopGun is the primary data store, compute layer, and sync engine.

| Tier | What you keep | What TopGun adds | Effort |
|------|--------------|-----------------|--------|
| **Tier 1** | Existing DB, API, auth | Real-time sync + offline for specific features | Hours |
| **Tier 2** | Existing DB (Postgres) | In-memory cache, CRDT sync, live queries | Days |
| **Tier 3** | Nothing | Everything: storage, compute, sync, search, streaming | Weeks |

---

## Security Model

### Trust Boundary

Clients are **untrusted**. The server is **authoritative**. "Client as Replica" is an architectural concept for data availability — it does not mean clients have unrestricted write access.

### Security Pipeline

Every write passes through the security pipeline before reaching CRDT merge:

```
Client write → Auth check → Map ACL check → HLC sanitization → CRDT merge → Persist
```

| Layer | What it does |
|-------|-------------|
| **Authentication** | JWT-based. Clients must authenticate before any operations |
| **Map-level ACL** | Per-connection, per-map read/write permissions. Simple allow/deny |
| **HLC sanitization** | Server replaces client-provided HLC timestamps with server-generated ones. Prevents future-timestamp attacks that would "win" all LWW conflicts |
| **Value size limits** | Server enforces maximum value size per write |

### What This Means for CRDTs

The CRDT merge semantics remain unchanged. The security layer sits BEFORE the merge — it's the same approach used by Ditto and Firebase. CRDTs handle conflict resolution; the security layer handles authorization.

---

## Core Differentiators

### 1. Synchronous In-Memory Reads

`map.get('key')` returns instantly — no `await`, no async. This enables 60fps UI rendering without Suspense boundaries. Competitors (PowerSync, ElectricSQL) require async SQLite reads (1-5ms). This isn't "0ms vs 5ms" — it's a **sync vs async API**, which fundamentally changes how you write UI components.

### 2. Automatic CRDT Conflict Resolution

No developer code needed. PowerSync requires custom conflict handlers. Convex avoids the problem by not supporting offline. Only TopGun and Ditto have automatic merge, and Ditto has no web SDK.

### 3. Self-Contained Architecture

No external database required. TopGun IS the database + sync engine + SQL engine + compute layer + search engine. One deployment replaces the combination of PowerSync + Postgres + Hazelcast + Elasticsearch.

### 4. Unified Client + Server

Same CRDT types, same HLC clock, same SQL dialect (DataFusion WASM in browser, DataFusion native on server). No impedance mismatch between client and server.

### 5. Full Data Platform

Unlike sync-only solutions, TopGun provides server-side distributed computing: SQL queries, stream processing DAGs, entry processors, counters, connectors — the full Hazelcast feature set, plus offline-first.

### 6. Built-In Admin UI

First-class admin dashboard with cluster topology visualization, data explorer, query playground, and settings management. No separate commercial product required (unlike Hazelcast Management Center).

---

## Capabilities by Version

### v1.0 — Working IMDG

*The foundational release: a production-ready distributed data grid with offline-first clients.*

| Capability | Description |
|------------|-------------|
| **Distributed Maps** | Partitioned key-value store (LWW-Map, OR-Map, PNCounter) with automatic CRDT conflict resolution |
| **Real-Time Sync** | WebSocket-based push architecture with automatic reconnection |
| **Merkle Tree Delta Sync** | Only changed data moves over the wire — efficient bandwidth on reconnect |
| **Offline Support** | Client persists to IndexedDB, queues operations, syncs when back online |
| **Live Queries** | Client subscribes to a query; server pushes incremental updates as data changes |
| **Predicate Queries** | Server-side filtering: Eq, Gt, Lt, In, AND/OR/NOT, ORDER BY, LIMIT, aggregations (count/min/max/sum) |
| **Topic Pub/Sub** | Cluster-wide publish/subscribe with ordered delivery |
| **Server Clustering** | Multi-node cluster with automatic partitioning (271 partitions), failure detection, rebalancing |
| **Counters** | Atomic server-side increment/decrement operations |
| **Entry Processors** | Server-side in-place mutations (read-modify-write without serialization round-trip) |
| **PostgreSQL Persistence** | Durable storage via write-through to PostgreSQL |
| **Full-Text Search** | Tantivy-powered: tokenization, fuzzy matching, phrase queries |
| **Hybrid Logical Clock** | Global causality tracking across all nodes (browsers + servers) |
| **React SDK** | Hooks: `useQuery`, `useMap`, `useORMap`, `useMutation`, `useTopic` |
| **Framework Agnostic** | Core SDK works with React, Vue, Svelte, Solid, vanilla JS |
| **Admin Dashboard** | Web UI: cluster topology, data explorer, query playground, settings |
| **Self-Hosted** | Run on your own infrastructure — no vendor lock-in |

**Developer experience:**

```typescript
// Instant writes (works offline)
const todos = client.getMap('todos');
todos.set('todo-1', { text: 'Buy milk', done: false });

// Synchronous reads — no await
const todo = todos.get('todo-1');

// Live queries with React
const { data } = useQuery('todos', { where: { done: false }, orderBy: 'createdAt' });

// Topic pub/sub
client.topic('notifications').publish({ type: 'alert', message: 'Hello!' });
```

---

### v2.0 — Data Platform

*SQL queries, stream processing, schema validation, connectors — a unified data platform.*

| Capability | Description |
|------------|-------------|
| **SQL Queries (DataFusion)** | Full SQL: SELECT, WHERE, JOIN, GROUP BY, aggregations, window functions. Cost-based optimizer |
| **Distributed SQL Execution** | Queries distributed across partition owners with partial-to-final aggregation on coordinator |
| **Client-Side SQL (WASM)** | Same SQL dialect offline via DataFusion compiled to WASM — query local data with SQL in the browser |
| **Schema System** | TypeScript-first schema definition (`topgun.schema.ts`), server-side validation, auto-generated types |
| **Partial Replication / Shapes** | Client subscribes to data subsets; server syncs only matching entries. Row + field filtering |
| **Stream Processing (DAG)** | Distributed streaming pipelines: windowed aggregation, stateful operators, fault-tolerant checkpointing |
| **Connector Framework** | Pluggable sources and sinks: Kafka, S3, PostgreSQL CDC, webhooks |
| **Write-Behind Persistence** | Async write coalescing and batch flush to storage backends |
| **Extension System** | Pluggable modules for community contributions (crypto, compression, audit, geo) |
| **SSE Transport** | Server-Sent Events for real-time push in serverless/edge environments |
| **Custom Conflict Resolvers** | User-defined CRDT merge strategies via WASM sandbox: priority-based composition, key pattern matching, 11+ built-in resolvers (first-write-wins, numeric min/max, array union, deep merge, owner-only, immutable, version increment) |
| **WASM Modules** | DataFusion SQL + Tantivy search compiled to WASM for browser use |

**Developer experience:**

```typescript
// SQL queries — same syntax client and server
const results = await client.sql(
  'SELECT userId, COUNT(*) as total FROM orders GROUP BY userId ORDER BY total DESC LIMIT 10'
);

// Schema definition
const Todo = tg.map('todos', {
  id: tg.string(),
  text: tg.string(),
  done: tg.boolean(),
  createdAt: tg.timestamp(),
});

// Partial replication — client only gets their data
const myTodos = client.shape('todos', {
  where: { userId: currentUser.id },
  fields: ['id', 'text', 'done'],
});

// Stream processing pipeline
client.pipeline('daily-stats')
  .from('orders')
  .window({ type: 'tumbling', size: '1 day' })
  .aggregate({ total: sql`SUM(amount)`, count: sql`COUNT(*)` })
  .into('order_stats');

// Connectors — ingest from external sources
server.connector('kafka-orders', {
  source: { type: 'kafka', brokers: ['localhost:9092'], topic: 'raw-orders' },
  sink: { type: 'topgun-map', map: 'orders' },
});
```

---

### v3.0 — Enterprise

*Enterprise-grade: multi-tenancy, tiered storage, advanced analytics, time-travel.*

| Capability | Description |
|------------|-------------|
| **Multi-Tenancy** | Per-tenant isolation, quotas, billing hooks, tenant-aware partitioning |
| **S3 Bottomless Storage** | Append-only immutable log segments in S3/R2/GCS with Merkle checkpoints |
| **Tiered Storage (Hot/Cold)** | Hot data in memory, warm in PostgreSQL, cold in S3 — transparent access, automatic migration |
| **Vector Search** | Semantic search with local embeddings, HNSW index. Tri-hybrid: exact + BM25 + semantic |
| **Bi-Temporal Queries** | Time-travel queries with valid time + transaction time dimensions |

**Developer experience:**

```typescript
// Multi-tenant — transparent isolation
const server = new TopGunServer({
  tenancy: { mode: 'isolated', quotas: { maxMaps: 100, maxStorageGB: 10 } }
});

// Tiered storage — transparent access across tiers
const server = new TopGunServer({
  storage: {
    hot: { type: 'memory', maxSizeGB: 8 },
    warm: { type: 'postgresql', connectionString: '...' },
    cold: { type: 's3', bucket: 'topgun-archive', region: 'us-east-1' },
  }
});

// Vector search — semantic + full-text hybrid
const results = client.search('products', {
  query: 'comfortable running shoes for trail',
  mode: 'hybrid', // exact + BM25 + semantic
  limit: 10,
});

// Time-travel queries
const snapshot = client.sql('SELECT * FROM orders AS OF TIMESTAMP "2026-01-15T00:00:00Z"');
```

---

## Admin Dashboard

TopGun ships with a built-in web admin UI (React 19 + Vite). No separate commercial product required.

| Feature | Description |
|---------|-------------|
| **Cluster Topology** | Partition ring SVG visualization, node health cards (CPU, memory, uptime), rebalancing status |
| **Data Explorer** | Map browser with filtering, CRUD operations, inline JSON editor (Monaco) |
| **Query Playground** | Code editor with JavaScript execution sandbox, results in table/JSON/stats views |
| **Settings Management** | General, storage, integrations, cluster, rate limits. Hot-reloadable vs restart-required distinction |
| **Setup Wizard** | Initial server bootstrap: standalone/cluster mode, storage configuration, admin user creation |
| **Auth & Security** | JWT-based authentication, protected routes |
| **Command Palette** | Cmd+K navigation, search, theme toggle |
| **Dark Mode** | Full light/dark theme support |

---

## Competitive Comparison

| Feature | TopGun (v3.0) | Hazelcast | PowerSync | ElectricSQL | Firebase | Convex | RxDB |
|---------|--------|-----------|-----------|-------------|----------|--------|------|
| **Primary model** | Real-Time Data Platform | Server-Side IMDG | Postgres Sync | Postgres Sync | Cloud DB | Reactive BaaS | Local-First DB |
| **Browser client** | First-class (CRDT replica) | None | Yes | Yes | Yes | Yes | Yes |
| **Offline support** | First-class (CRDT) | None | Good (SQLite) | Good (SQLite) | Limited | None | Excellent |
| **Read latency** | ~0ms (in-memory) | <1ms (server) | ~5ms (SQLite) | ~5ms (SQLite) | Network | Network | ~5ms (IDB) |
| **Read API** | Synchronous | Async (network) | Async (SQLite) | Async (SQLite) | Async | Async | Async |
| **Conflict resolution** | Automatic (CRDT) | Manual (locks) | Manual | Rich CRDTs | LWW (server) | Serializable | Revision trees |
| **SQL engine** | DataFusion (distributed) | Calcite | SQLite (local) | SQLite (local) | None | Custom | None |
| **Client-side SQL** | DataFusion WASM | None | SQLite | SQLite | None | None | None |
| **Stream processing** | DAG with checkpoints | Jet engine | None | None | None | None | None |
| **Server compute** | Entry processors, DAG, SQL | Entry processors, Jet | None | None | Cloud Functions | Mutations | None |
| **Connectors** | Kafka, S3, CDC, webhooks | 50+ connectors | None | None | Extensions | Integrations | None |
| **Clustering** | Multi-node, 271 partitions | Multi-node | Single server | Single service | Managed | Managed | None |
| **Tiered storage** | Memory → PostgreSQL → S3 | Hot restart only | N/A | N/A | Managed | Managed | N/A |
| **Multi-tenancy** | Built-in | Enterprise license | Per-project | Per-project | Per-project | Per-project | N/A |
| **Vector search** | Tri-hybrid (exact+BM25+semantic) | None | None | None | Extensions | None | None |
| **Works alongside existing DB** | Yes (Tier 1-2 adoption) | Yes (cache layer) | Yes (Postgres sync) | Yes (Postgres sync) | No (proprietary) | No (own DB) | Yes (CouchDB) |
| **Admin UI** | Built-in (open source) | Management Center (commercial) | Cloud dashboard | None | Console | Dashboard | None |
| **Self-hosted** | Yes | Yes (open-core) | Yes | Yes | No | Yes (OSS) | Yes |
| **License** | Open Source | Apache + Commercial | Apache 2.0 | Apache 2.0 | Proprietary | BSL | Apache 2.0 |

### Positioning Matrix

```
                  Server-side compute power →
                  Low                              High
              ┌─────────────────────────────────────────┐
        High  │  PowerSync    ElectricSQL               │
              │  RxDB                                    │
  Offline     │                                          │
  capability  │                       ★ TopGun ★         │
              │                                          │
        Low   │  Firebase     Convex                     │
              │  Liveblocks              Hazelcast       │
              └─────────────────────────────────────────┘
```

TopGun is the only product in the **upper-right quadrant**: strong offline capability AND strong server-side compute. No other product bridges both worlds.

---

## Architecture (Full Platform — v3.0)

```
┌──────────────────────────────────────────────────────────────────┐
│                     Client (Browser / Node / WASM)                │
│                                                                   │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌────────────────┐   │
│  │ LWW-Map  │  │  OR-Map  │  │  HLC     │  │ DataFusion SQL │   │
│  │ (replica)│  │ (replica)│  │ (local)  │  │ (WASM)         │   │
│  └──────────┘  └──────────┘  └──────────┘  └────────────────┘   │
│  ┌────────────────────────────────────────────────────────────┐  │
│  │  SyncEngine (WebSocket · SSE · HTTP) + offline queue       │  │
│  └────────────────────────────────────────────────────────────┘  │
│  ┌────────────────────────────────────────────────────────────┐  │
│  │  IndexedDB / SQLite (local persistence + Tantivy WASM)     │  │
│  └────────────────────────────────────────────────────────────┘  │
└──────────────────────────┬───────────────────────────────────────┘
                           │ WebSocket / SSE / HTTP (MsgPack)
┌──────────────────────────▼───────────────────────────────────────┐
│                      Server Cluster (Rust)                        │
│                                                                   │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────────┐     │
│  │ Node 1   │  │ Node 2   │  │ Node 3   │  │ ...          │     │
│  │ P[0-90]  │  │ P[91-180]│  │P[181-270]│  │              │     │
│  └──────────┘  └──────────┘  └──────────┘  └──────────────┘     │
│                                                                   │
│  ┌────────────────────────────────────────────────────────────┐  │
│  │  Domain Services                                            │  │
│  │  CRDT · Sync · Query · Messaging · Persistence · Search     │  │
│  └────────────────────────────────────────────────────────────┘  │
│  ┌────────────────────────────────────────────────────────────┐  │
│  │  Data Platform                                              │  │
│  │  DataFusion SQL · DAG Stream Executor · Connector Framework │  │
│  └────────────────────────────────────────────────────────────┘  │
│  ┌────────────────────────────────────────────────────────────┐  │
│  │  Enterprise Layer                                           │  │
│  │  Multi-Tenancy · Schema Validation · Extension System       │  │
│  └────────────────────────────────────────────────────────────┘  │
│  ┌────────────────────────────────────────────────────────────┐  │
│  │  Tiered Storage                                             │  │
│  │  ┌─────────────┐  ┌──────────────┐  ┌──────────────────┐  │  │
│  │  │ Hot (Memory) │→ │ Warm (PgSQL) │→ │ Cold (S3/R2/GCS) │  │  │
│  │  └─────────────┘  └──────────────┘  └──────────────────┘  │  │
│  └────────────────────────────────────────────────────────────┘  │
│                                                                   │
│  ┌────────────────────────────────────────────────────────────┐  │
│  │  Admin Dashboard (React)                                    │  │
│  │  Cluster Topology · Data Explorer · Query Playground        │  │
│  └────────────────────────────────────────────────────────────┘  │
│                                                                   │
│  ┌────────────────────────────────────────────────────────────┐  │
│  │  Connectors                                                 │  │
│  │  Kafka · S3 · PostgreSQL CDC · Webhooks · Custom            │  │
│  └────────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────────┘
```

---

## Technical Foundation

| Layer | Technology | Purpose |
|-------|-----------|---------|
| **Server runtime** | Rust (tokio, axum) | Performance, safety, low memory footprint |
| **Client SDK** | TypeScript | Developer experience, ecosystem compatibility |
| **Wire protocol** | MsgPack over WebSocket/SSE/HTTP | Cross-language, compact, JS-native decoding |
| **Conflict resolution** | LWW-Map, OR-Map (CRDTs) | Automatic convergence without coordination |
| **Causality** | Hybrid Logical Clock (HLC) | Unified time across browsers and servers |
| **Delta sync** | Merkle Trees | Only changed data transfers on reconnect |
| **SQL engine** | Apache DataFusion | Cost-based optimizer, distributed execution, WASM-compatible |
| **Stream processing** | DAG executor (Arroyo-informed) | Windowed aggregation, stateful operators, checkpointing |
| **Full-text search** | Tantivy | Tokenization, fuzzy matching, phrase queries |
| **Vector search** | usearch (HNSW) | Semantic similarity, local embeddings |
| **Hot storage** | In-memory (DashMap) | Zero-latency reads/writes |
| **Warm storage** | PostgreSQL (sqlx) | Durable persistence, SQL-queryable |
| **Cold storage** | S3/R2/GCS (opendal) | Cost-effective archival, immutable log segments |
| **Client persistence** | IndexedDB / SQLite | Offline data survival |
| **Admin UI** | React 19 + Vite + Radix UI | Cluster management, data exploration |

---

## Summary

**TopGun** is a unified real-time data platform that replaces the combination of:

| Traditional stack | TopGun equivalent |
|---|---|
| Sync engine (PowerSync, ElectricSQL) | Built-in CRDT sync + offline-first client |
| Server database (PostgreSQL) | In-memory data grid + tiered storage |
| Compute layer (Hazelcast, Redis) | Entry processors, DAG stream processing |
| SQL engine (separate query service) | DataFusion (server + client WASM) |
| Search engine (Elasticsearch) | Tantivy full-text + usearch vector |
| Admin panel (separate build) | Built-in admin dashboard |

**One platform.** Zero-latency reads. Automatic conflict resolution. Same SQL dialect online and offline. From browser to cluster to cloud storage.

---

## Version Roadmap

| Version | Theme | Key Deliverables |
|---------|-------|-------------------|
| **v1.0** | Working IMDG | Distributed maps (LWW-Map, OR-Map, PNCounter), clustering, live queries, full-text search, PostgreSQL persistence, admin UI, React SDK |
| **v2.0** | Data Platform | DataFusion SQL (server + WASM client), DAG stream processing, schema system, partial replication, connectors, custom conflict resolvers, write-behind persistence |
| **v3.0** | Enterprise | Multi-tenancy, S3 tiered storage, vector search, bi-temporal queries |
