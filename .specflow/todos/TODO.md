# To-Do List

**Last updated:** 2026-02-07 (added TODO-050; marked TODO-047 done; reordered by technical dependencies)
**Source:** Migrated from PROMPTS directory, reordered by technical dependencies

---

## Wave 0: Foundation Refactoring

*Goal: Fix abstraction leaks that block transport evolution*

### TODO-050: IConnection Abstraction
- **Priority:** 🔴 High
- **Complexity:** Low
- **Summary:** Replace `WebSocket` return type in `IConnectionProvider` with abstract `IConnection` interface
- **Why:** `HttpSyncProvider` throws runtime errors on `getConnection()`/`getAnyConnection()` because the interface forces `WebSocket` return type. `AutoConnectionProvider` inherits the same type-safety hole. Any new transport (SSE, QUIC) will hit the same problem. This is technical debt blocking TODO-048 and TODO-049.
- **Current Problem:**
  - `IConnectionProvider.getConnection()` returns `WebSocket` (types.ts:46)
  - `HttpSyncProvider` throws on these methods (cannot return WebSocket)
  - `AutoConnectionProvider` can throw at runtime in HTTP mode
  - 90% of callers only need `send()` — not raw WebSocket access
- **Proposed Interface:**
  ```
  IConnection { send(data): void; isOpen(): boolean; close(): void }
  IConnectionProvider { getConnection(key): IConnection; getAnyConnection(): IConnection; ... }
  ```
- **Blast Radius:**
  - `types.ts` — define `IConnection`, update `IConnectionProvider`
  - `SingleServerProvider.ts` — wrap WebSocket in IConnection adapter
  - `ConnectionPool.ts` — wrap WebSocket in IConnection adapter
  - `ClusterClient.ts` — update 3 call sites (all just call `.send()`)
  - `PartitionRouter.ts` — update 2 call sites
  - `HttpSyncProvider.ts` — return null-transport or no-op IConnection instead of throwing
  - Tests — update mock types
- **Effort:** 4-6 hours (~100-150 lines changed)
- **Dependencies:** None (pure refactoring)
- **Unlocks:** TODO-048 (SSE), TODO-049 (Cluster HTTP)

---

## Wave 1: Cluster Infrastructure

*Goal: Efficient distributed queries, partition-aware routing*

### TODO-029: Partition Pruning
- **Priority:** 🟡 Medium
- **Complexity:** Medium
- **Context:** [reference/HAZELCAST_QUICK_WINS.md](../reference/HAZELCAST_QUICK_WINS.md)
- **Summary:** Skip partitions that can't contain matching records
- **Why:** Required for efficient distributed queries at scale; prerequisite for TODO-025 (DAG Executor) and recommended for TODO-049 (Cluster HTTP Routing)
- **Current:** Distributed queries scan all partitions
- **Solution:** Use partition key to determine relevant partitions
- **Example:** Query `tenantId = 'abc'` → only scan partitions where hash('abc') maps
- **Effort:** 1 week

---

### TODO-023: Client Cluster Smart Routing
- **Priority:** 🟡 Medium
- **Complexity:** Large
- **Context:** [reference/PHASE_4.5_CLIENT_CLUSTER_SPEC.md](../reference/PHASE_4.5_CLIENT_CLUSTER_SPEC.md)
- **Summary:** Integrate ClusterClient with TopGunClient for transparent partition routing
- **Why:** Full cluster utilization, reduces coordinator bottleneck
- **Key Features:**
  - Smart client routing to partition owners
  - Client-side failover on node failure
  - Partition map synchronization
  - ConnectionPool with health checks
- **Target:** 50,000+ ops/sec in cluster mode
- **Effort:** ~16 hours (7 tasks)
- **Files to modify:** TopGunClient.ts, SyncEngine.ts, ClusterClient.ts, ConnectionPool.ts

---

## Wave 2: Transport Evolution

*Goal: Close the real-time gap for serverless, enable cluster HTTP*

### TODO-048: SSE Push for HTTP Sync
- **Priority:** 🟡 Medium
- **Complexity:** Medium
- **Context:** Extends SPEC-036 (HTTP Sync Protocol)
- **Summary:** Add Server-Sent Events transport for real-time push in serverless environments
- **Why:** HTTP polling introduces latency proportional to `pollIntervalMs`. SSE enables server-initiated push without WebSocket, closing the real-time gap for serverless deployments.
- **Architecture:**
  - Client POSTs writes to `POST /sync` (existing)
  - Client receives real-time updates via `GET /events` (SSE stream)
  - New `SsePushProvider` implements `IConnectionProvider`
  - `AutoConnectionProvider` gains a third tier: WS → SSE → HTTP polling
- **Platform Support:** Vercel Edge (streaming), Cloudflare Workers (with Durable Objects), AWS Lambda (response streaming)
- **Effort:** 2-3 weeks
- **Dependencies:** TODO-050 (IConnection abstraction)

---

### TODO-049: Cluster-Aware HTTP Routing
- **Priority:** 🟡 Medium
- **Complexity:** Medium
- **Context:** Extends SPEC-036 (HTTP Sync Protocol), relates to TODO-023 (Client Cluster Smart Routing)
- **Summary:** Enable `HttpSyncHandler` to route sync requests to partition owners in a cluster
- **Why:** Currently HTTP sync runs standalone against a single node's data. In cluster mode without shared PostgreSQL, a client sees only data from the node it hits. This makes HTTP sync unusable for in-memory-only clusters.
- **Architecture:**
  - `HttpSyncHandler` queries `PartitionService` to find partition owner per map key
  - Forwards delta computation to owner node via internal cluster protocol
  - Merges responses from multiple partition owners into single HTTP response
- **Effort:** 2-3 weeks
- **Dependencies:** TODO-050 (IConnection abstraction), TODO-029 (Partition Pruning — recommended)

---

## Wave 3: Storage Infrastructure

*Goal: Enable slow backends, unlock distributed query processing*

### TODO-033: AsyncStorageWrapper (Write-Behind)
- **Priority:** 🟡 Medium
- **Complexity:** Medium
- **Context:** [reference/topgun-rocksdb.md](../reference/topgun-rocksdb.md)
- **Summary:** Implement Hazelcast-style Write-Behind pattern for slow storage backends
- **Why:** Enables S3/slow storage backends without latency impact. Current IServerStorage is synchronous — slow backends block the write path.
- **Key Features:**
  - Staging Area: In-memory buffer for Read-Your-Writes consistency
  - Write Coalescing: Merge multiple updates to same key
  - Batch Flush: Periodic flush to storage (5s intervals)
  - Retry Queue: Handle storage failures gracefully
- **Note:** Server storage architecture is already clean — IServerStorage is pluggable with PostgreSQL, SQLite, and Memory implementations. This wraps any IServerStorage, not a rewrite.
- **Effort:** 2-3 weeks

---

### TODO-025: DAG Executor for Distributed Queries
- **Priority:** 🟡 Medium
- **Complexity:** Large
- **Context:** [reference/HAZELCAST_DAG_EXECUTOR_SPEC.md](../reference/HAZELCAST_DAG_EXECUTOR_SPEC.md)
- **Additional:** [reference/HAZELCAST_ARCHITECTURE_COMPARISON.md](../reference/HAZELCAST_ARCHITECTURE_COMPARISON.md)
- **Summary:** Implement Hazelcast-style DAG executor for distributed query processing
- **Key Features:**
  - DAG structure with Vertex/Edge graph
  - 3-tier processor model: Source → Transform → Sink
  - Partition-aware execution
  - Backpressure handling
- **Architecture Pattern:** Processors exchange data via Outbox/Inbox queues
- **Effort:** 4-6 weeks
- **Dependencies:** TODO-029 (Partition Pruning)

---

## Wave 4: Advanced Features

*Goal: AI capabilities, performance optimization, extensibility*

### TODO-039: Vector Search
- **Priority:** 🟡 Medium
- **Complexity:** Large
- **Context:** [reference/PHASE_15_VECTOR_SEARCH_SPEC.md](../reference/PHASE_15_VECTOR_SEARCH_SPEC.md)
- **Summary:** Semantic vector search with local embeddings (transformers.js)
- **Key Features:**
  - Local embedding generation (no API keys)
  - Vector storage as CRDT (synced)
  - HNSW index (usearch/voy)
  - Tri-hybrid search: Exact + BM25 + Semantic
- **Package:** `@topgunbuild/vector` (optional)
- **Effort:** 4 weeks
- **Dependencies:** Phase 12 (Hybrid Search), Phase 14 (Distributed Search) — complete

---

### TODO-034: Rust/WASM Hot Path Migration
- **Priority:** 🟡 Medium
- **Complexity:** Large
- **Context:** [reference/RUST_WASM_ANALYSIS.md](../reference/RUST_WASM_ANALYSIS.md)
- **Summary:** Migrate CPU-intensive hot paths to Rust/WASM
- **Why:** Benefits from having DAG Executor (TODO-025) as a prime WASM candidate
- **Candidates (by priority):**
  1. MerkleTree Hash/Diff → 50-60% speedup
  2. CRDT Batch Merge → 30-40% speedup
  3. DAG Executor → 2-5x speedup
  4. SQL Parser (sqlparser-rs) → new feature
- **Package Structure:**
  ```
  packages/core-rust/   # Rust crate
  packages/core-wasm/   # TS wrapper with fallback
  ```
- **Strategy:** Conditional loading (browser=JS, server=WASM)
- **Effort:** 4-6 weeks total

---

### TODO-036: Pluggable Extension System
- **Priority:** 🟢 Low
- **Complexity:** Medium
- **Context:** [reference/TURSO_INSIGHTS.md](../reference/TURSO_INSIGHTS.md) (Section 5)
- **Summary:** Modular extension system for optional features
- **Why:** Enables community contributions, smaller core bundle
- **Example Extensions:**
  ```
  @topgunbuild/ext-crypto      # Encryption at rest
  @topgunbuild/ext-compress    # Compression (zstd, brotli)
  @topgunbuild/ext-audit       # Audit logging
  @topgunbuild/ext-geo         # Geospatial queries
  ```
- **Effort:** 2-3 weeks for infrastructure

---

## Wave 5: Documentation

*Goal: Document public APIs when convenient*

### ~~TODO-047: Blog Post — "TopGun Goes Serverless"~~ DONE
- **Completed:** 2026-02-07 (quick mode, commit `8861f63`)
- **Location:** `apps/docs-astro/src/content/blog/serverless-http-sync.mdx`

---

### TODO-045: DST Documentation
- **Priority:** 🟢 Low
- **Complexity:** Low
- **Context:** Implements SPEC-001 (completed 2026-02-05)
- **Summary:** Document Deterministic Simulation Testing utilities in official docs
- **Why:** New public API (VirtualClock, SeededRNG, ScenarioRunner) exported from @topgunbuild/core
- **Location:** `apps/docs-astro/src/content/docs/reference/testing.mdx`
- **Contents:**
  - VirtualClock: injectable time source for deterministic tests
  - SeededRNG: reproducible randomness (same seed = same sequence)
  - VirtualNetwork: simulated packet loss, latency, partitions
  - InvariantChecker: CRDT convergence property assertions
  - ScenarioRunner: orchestrates reproducible multi-node simulations
- **Example:** Show ScenarioRunner usage for chaos testing with seeds
- **Effort:** 0.5-1 day
- **Note:** Can be done as a breather between heavy implementation tasks

---

## Wave 6: Enterprise (Deferred)

*Goal: Enterprise features, major architectural changes*
*Defer until Waves 0-4 complete*

### TODO-041: Multi-Tenancy
- **Priority:** 🔵 Deferred
- **Complexity:** Large
- **Context:** [reference/PHASE_5_MULTI_TENANCY_SPEC.md](../reference/PHASE_5_MULTI_TENANCY_SPEC.md)
- **Summary:** Per-tenant isolation, quotas, billing
- **Key Features:**
  - Tenant context in all operations
  - Resource quotas (storage, connections, ops/sec)
  - Tenant-aware partitioning

---

### TODO-043: S3 Bottomless Storage
- **Priority:** 🔵 Deferred
- **Complexity:** Very Large
- **Context:** [reference/TURSO_INSIGHTS.md](../reference/TURSO_INSIGHTS.md) (Section 7)
- **Summary:** Append-only log in object storage (S3, R2, GCS)
- **Features:**
  - Operations written to S3 as immutable log segments
  - Nodes replay log on startup
  - Merkle tree checkpoints for fast recovery
  - 10x cheaper storage than managed PostgreSQL
- **Challenges:** Major architectural change, S3 latency for writes
- **Effort:** 6-8 weeks
- **Dependencies:** TODO-033 (AsyncStorageWrapper)

---

### TODO-044: Bi-Temporal Queries (Time-Travel)
- **Priority:** 🔵 Deferred
- **Complexity:** Large
- **Context:** [reference/TURSO_INSIGHTS.md](../reference/TURSO_INSIGHTS.md) (Section 8)
- **Summary:** Query historical state with valid time + transaction time
- **Example:** `client.query('tasks', filter, { asOf: '2025-01-01T00:00:00Z' })`
- **Benefits:** Point-in-time debugging, audit trails, undo/redo
- **Dependencies:** TODO-043 (S3 Bottomless Storage)
- **Effort:** 4-6 weeks

---

### TODO-040: Tiered Storage (Hot/Cold)
- **Priority:** 🔵 Deferred
- **Complexity:** Large
- **Context:** [reference/topgun-rocksdb.md](../reference/topgun-rocksdb.md)
- **Summary:** Hot data in memory/Redis, cold data in S3/cheap storage
- **Features:** Transparent migration based on access patterns
- **Use Case:** Cost reduction for large datasets
- **Dependencies:** TODO-033 (AsyncStorageWrapper)

---

### TODO-042: DBSP Incremental Views
- **Priority:** ⚠️ High Risk — Deferred
- **Complexity:** Very Large
- **Context:** [reference/TURSO_INSIGHTS.md](../reference/TURSO_INSIGHTS.md) (Section 4)
- **Summary:** Implement DBSP (Database Stream Processing) for delta-based query updates
- **Problem:** LiveQueryManager recomputes queries on every change
- **Solution:** Compile queries to streaming operators, maintain incremental state
- **Warning:** High risk — could become 6-month compiler project
- **Alternative:** Start with "React Signals" style fine-grained reactivity
- **Reference:** Turso `/core/incremental/`, Materialize, differential-dataflow

---

## Quick Reference

### Dependency Graph

```
TODO-050 (IConnection)          TODO-029 (Partition Pruning)
    │                               │
    ├──→ TODO-048 (SSE)             ├──→ TODO-025 (DAG Executor)
    │                               │        │
    └──→ TODO-049 (Cluster HTTP) ←──┘        └──→ TODO-034 (Rust/WASM)
                                    │
TODO-023 (Client Cluster)          TODO-033 (AsyncStorage)
    (independent)                   │
                                    ├──→ TODO-043 (S3 Bottomless)
                                    │        │
                                    │        └──→ TODO-044 (Bi-Temporal)
                                    │
                                    └──→ TODO-040 (Tiered Storage)
```

### By Wave

| Wave | Items | Total Effort | Focus |
|------|-------|--------------|-------|
| 0. Foundation | 1 | 4-6 hours | Fix IConnection abstraction |
| 1. Cluster | 2 | ~3 weeks | Partition pruning, client routing |
| 2. Transport | 2 | ~4-6 weeks | SSE, cluster HTTP |
| 3. Storage | 2 | ~7 weeks | Write-behind, DAG |
| 4. Advanced | 3 | ~10 weeks | Vector, WASM, extensions |
| 5. Documentation | 1 | 0.5-1 day | DST docs (TODO-047 done) |
| 6. Enterprise | 5 | ~20+ weeks | Tenancy, S3, time-travel |

### Execution Order (by technical dependency)

| # | TODO | Wave | Effort | Unlocks | Priority |
|---|------|------|--------|---------|----------|
| 1 | TODO-050 | 0 | 4-6 hours | TODO-048, TODO-049 | 🔴 High |
| 2 | TODO-029 | 1 | 1 week | TODO-025, TODO-049 | 🟡 Medium |
| 3 | TODO-023 | 1 | ~16 hours | — (independent) | 🟡 Medium |
| 4 | TODO-048 | 2 | 2-3 weeks | — | 🟡 Medium |
| 5 | TODO-049 | 2 | 2-3 weeks | — | 🟡 Medium |
| 6 | TODO-033 | 3 | 2-3 weeks | TODO-043, TODO-040 | 🟡 Medium |
| 7 | TODO-025 | 3 | 4-6 weeks | TODO-034 | 🟡 Medium |
| 8 | TODO-039 | 4 | 4 weeks | — | 🟡 Medium |
| 9 | TODO-034 | 4 | 4-6 weeks | — | 🟡 Medium |
| 10 | TODO-036 | 4 | 2-3 weeks | — | 🟢 Low |
| 11 | TODO-045 | 5 | 0.5-1 day | — | 🟢 Low |
| 12 | TODO-041 | 6 | Large | — | 🔵 Deferred |
| 13 | TODO-043 | 6 | 6-8 weeks | TODO-044 | 🔵 Deferred |
| 14 | TODO-044 | 6 | 4-6 weeks | — | 🔵 Deferred |
| 15 | TODO-040 | 6 | Large | — | 🔵 Deferred |
| 16 | TODO-042 | 6 | Very Large | — | ⚠️ Risk |

### Context Files

| TODO | Context File | Lines |
|------|--------------|-------|
| TODO-029 | HAZELCAST_QUICK_WINS.md | 400+ |
| TODO-023 | PHASE_4.5_CLIENT_CLUSTER_SPEC.md | 336 |
| TODO-033 | topgun-rocksdb.md | 650+ |
| TODO-025 | HAZELCAST_DAG_EXECUTOR_SPEC.md | 700+ |
| TODO-039 | PHASE_15_VECTOR_SEARCH_SPEC.md | 1696 |
| TODO-034 | RUST_WASM_ANALYSIS.md | 1127 |
| TODO-036 | TURSO_INSIGHTS.md (Section 5) | 482 |
| TODO-041 | PHASE_5_MULTI_TENANCY_SPEC.md | 700+ |
| TODO-043 | TURSO_INSIGHTS.md (Section 7) | 482 |
| TODO-044 | TURSO_INSIGHTS.md (Section 8) | 482 |
| TODO-040 | topgun-rocksdb.md | 650+ |
| TODO-042 | TURSO_INSIGHTS.md (Section 4) | 482 |

---

*Reordered by technical dependencies on 2026-02-07. Marketing considerations removed from prioritization.*
