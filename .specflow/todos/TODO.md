# To-Do List

**Last updated:** 2026-02-08 (TODO-056 added — network.start() Promise reject path)
**Source:** Migrated from PROMPTS directory, reordered by technical dependencies

---

## Wave -1: Post-Release Test Stability (v0.11.0 regression fixes)

*Goal: All server test suites pass — zero ignored failures*

### ~~TODO-051: Fix WebSocket client auth handshake after ServerFactory modular refactoring~~ → SPEC-038
- **Status:** Converted to [SPEC-038](.specflow/specs/SPEC-038.md)

---

### TODO-052: Verify interceptor pipeline and TLS setup work in production after modular refactoring
- **Priority:** 🔴 P1
- **Complexity:** Medium
- **Summary:** 5 tests expose potential production regressions introduced during sf-011b. Ключевой вопрос для каждого: **работает ли тестируемая функциональность в production?** Если нет — чинить production code, если да — адаптировать тест под новый API.
- **Affected Tests (5 failures):**
  - InterceptorIntegration.test.ts (3/5): `TypeError: serverWithInterceptor.processLocalOp is not a function`
  - tls.test.ts (1/2): `server.port` returns 0
  - tls.test.ts (1/2): `validateJwtSecret` throws — missing JWT_SECRET
- **Investigation questions (production-first):**
  1. **Interceptor pipeline**: `processLocalOp` убран из публичного API `ServerCoordinator`. Но interceptors задекларированы в `ServerCoordinatorConfig.interceptors` и должны вызываться при каждой операции. **Вопрос: interceptor pipeline вообще ещё подключён к потоку операций в `ServerFactory`?** Если `createHandlersModule()` не прокидывает interceptors в `OperationHandler` — это production баг, а не проблема теста. Нужно проверить: (a) передаются ли `config.interceptors` в `OperationHandler`, (b) вызывается ли `onBeforeOp`/`onAfterOp` при реальных операциях через WS.
  2. **TLS server port**: `server.port` возвращал 0 потому что порт назначался async, а getter читал config. **Уже исправлено** — `network.start()` теперь возвращает `Promise<number>`, `completeStartup(actualPort, actualClusterPort)` сохраняет оба. Тест должен вызвать `await server.ready()` перед проверкой порта.
  3. **TLS without JWT**: Тест не передаёт `jwtSecret` в config → `validateJwtSecret` бросает ошибку. Это тестовый конфиг, не production баг.
- **Approach:**
  - **Сначала** проверить production code: interceptors wiring в `ServerFactory` → `createHandlersModule()` → `OperationHandler`
  - **Если interceptors подключены**: тест нужно адаптировать (вызывать через ServerTestHarness или реальный WS клиент, а не через удалённый метод)
  - **Если interceptors НЕ подключены**: починить production wiring, затем тест пройдёт как есть
  - tls.test.ts: добавить `await server.ready()` и `jwtSecret` — это действительно тестовые правки
- **Key Files:**
  - `packages/server/src/ServerFactory.ts:~240` — как `config.interceptors` передаются в handlers module
  - `packages/server/src/modules/handlers-module.ts` — создание OperationHandler, проверить наличие interceptors
  - `packages/server/src/coordinator/operation-handler.ts` — вызов interceptor pipeline
  - `packages/server/src/__tests__/InterceptorIntegration.test.ts` — тест, проверяющий функциональность
  - `packages/server/src/__tests__/tls.test.ts` — TLS + port тесты
- **Verification:** `cd packages/server && npx jest --forceExit --testPathPattern="(InterceptorIntegration|tls\.test)" --verbose`
- **Dependencies:** Частично пересекается с TODO-051 (2 из 5 InterceptorIntegration тестов падают на auth, 3 — на API)

---

### TODO-053: Fix DistributedSearch cluster event routing and GC broadcast gap
- **Priority:** 🟡 P1
- **Complexity:** Medium
- **Summary:** 7 tests fail because ClusterEventHandler drops events without `key` field, but search and GC events use different routing. All DistributedSearch.e2e tests fail with empty AggregateError; 1 GC test fails because TTL expiration doesn't emit SERVER_EVENT broadcast.
- **Affected Tests (7 failures):**
  - DistributedSearch.e2e.test.ts (6/6): All tests fail with `AggregateError` — cluster forms correctly, nodes see each other, but search queries fail. Log shows WARNING: "Received cluster event with undefined key, ignoring"
  - GC.test.ts (1/6): "TTL expiration notifies query subscriptions via processChange" — `expect(serverEvents.length).toBeGreaterThan(0)` fails, actual: 0
- **Root Cause:** The `ClusterEventHandler` (packages/server/src/cluster/ClusterEventHandler.ts) validates incoming cluster events and drops messages where `key` is undefined. However, distributed search events (CLUSTER_SEARCH_REQUEST, CLUSTER_SEARCH_RESPONSE) and GC broadcast events don't use per-key routing — they are aggregate operations. The handler's key validation is too strict for these event types.
- **Why this is NOT trivial:** The fix isn't just "remove the key check". Search events need their own routing path that bypasses key-based partition validation. The `ClusterEventHandler.setupListeners()` method registers handlers for various cluster message types — search events may need separate registration or the key check needs to be type-aware.
- **Key Files:**
  - `packages/server/src/cluster/ClusterEventHandler.ts` — the `setupListeners()` method and key validation
  - `packages/server/src/__tests__/DistributedSearch.e2e.test.ts` — all 6 tests
  - `packages/server/src/__tests__/GC.test.ts:~320` — "TTL expiration notifies query subscriptions"
  - `packages/server/src/search/ClusterSearchCoordinator.ts` — how search requests are sent/received
  - `packages/server/src/coordinator/broadcast-handler.ts` — SERVER_EVENT broadcast for GC
- **Verification:** `cd packages/server && npx jest --forceExit --testPathPattern="(DistributedSearch.e2e|GC\.test)" --verbose`

---

### TODO-054: Fix ProcessorSandbox test hang + update 12 docs files with ServerFactory.create()
- **Priority:** 🟡 P2
- **Complexity:** Low (docs) + Unknown (sandbox)
- **Summary:** Two unrelated issues grouped as P2: (1) ProcessorSandbox.test.ts hangs indefinitely, even with `--forceExit`; (2) 12 documentation files in `apps/docs-astro` show `new ServerCoordinator(config)` which no longer works — constructor changed to `(config, dependencies)`, only `ServerFactory.create(config)` is the public API.
- **ProcessorSandbox hang:**
  - `packages/server/src/__tests__/ProcessorSandbox.test.ts` — hangs entire Jest process
  - Likely cause: `isolated-vm` sandbox creation fails silently or creates an unresolved Promise
  - Log shows `WARN: isolated-vm not available, falling back to less secure VM` — the fallback VM may have infinite loop or deadlock
  - **Investigation needed:** Run with `--detectOpenHandles`, check if the fallback `vm` module's `runInNewContext` hangs
- **Documentation (12 files with wrong server instantiation):**
  - reference/server.mdx, reference/adapter.mdx, guides/cluster-replication.mdx (8 examples), guides/full-text-search.mdx (2), guides/performance.mdx (3), guides/deployment.mdx, guides/security.mdx, guides/authentication.mdx, guides/event-journal.mdx, guides/rbac.mdx, guides/interceptors.mdx (3), blog/full-text-search-offline-first.mdx
  - All show `new ServerCoordinator({...})` → must be `ServerFactory.create({...})`
  - Note: `serverUrl` in client examples is CORRECT — `TopGunClient` still accepts `serverUrl` and wraps with `SingleServerProvider` internally (confirmed in TopGunClient.ts:155-156)
- **Verification:**
  - Sandbox: `cd packages/server && npx jest --forceExit --testPathPattern="ProcessorSandbox" --detectOpenHandles`
  - Docs: `grep -r "new ServerCoordinator" apps/docs-astro/` should return 0 results after fix

---

### TODO-055: Harden timing-sensitive server tests — replace setTimeout with polling
- **Priority:** 🟡 P1
- **Complexity:** Medium
- **Summary:** Multiple server tests use fixed `setTimeout` delays for synchronization, causing intermittent failures. Replace with bounded polling from existing `test-helpers.ts` utilities (`pollUntil`, `waitForCluster`, etc.). Goal: eliminate flaky timing-dependent failures without masking real bugs.
- **Affected Tests:**
  - Resilience.test.ts: "Split-Brain Recovery" — `await new Promise(r => setTimeout(r, 500))` before convergence check is too short; `mapB.get('keyA')` returns `undefined` after 10s. Fix: replace fixed 500ms delay with polling or increase convergence timeout. The `waitForConvergence()` helper already uses polling but the pre-wait is the bottleneck.
  - Chaos.test.ts: Lines 86, 145, 155, 282 use fixed `setTimeout` delays for cluster sync, packet loss recovery, and backpressure. Also accesses private cluster internals via `(nodeA as any).cluster` — fragile but lower priority to fix.
  - Other tests across server package that use raw `setTimeout` for synchronization waits (audit needed — many test files still use this pattern instead of centralized polling helpers)
- **Approach:**
  1. Audit all `setTimeout` usage in server test files — identify which are sync waits vs intentional delays
  2. Replace sync waits with `pollUntil()` or `waitForCluster()` from `test-helpers.ts`
  3. For convergence tests: ensure `waitForConvergence()` is used consistently with adequate timeouts
  4. Keep intentional delays (e.g., TTL expiration waits) but make them explicit with comments
- **Key Files:**
  - `packages/server/src/__tests__/Resilience.test.ts` — split-brain recovery timing
  - `packages/server/src/__tests__/Chaos.test.ts` — cluster chaos simulation timing
  - `packages/server/src/__tests__/utils/test-helpers.ts` — existing hardened polling utilities
- **Verification:** `cd packages/server && npx jest --forceExit --testPathPattern="(Resilience|Chaos)" --verbose` — should pass consistently across 3+ consecutive runs
- **Dependencies:** TODO-051/SPEC-038 (port capture fix — now complete), TODO-052 (interceptor pipeline fix — independent but same wave)

---

### TODO-056: Add reject path to network.start() Promise for listen failure handling
- **Priority:** 🔴 P1
- **Complexity:** Low
- **Summary:** `network.start()` in `packages/server/src/modules/network-module.ts:95` creates a `new Promise<number>((resolve) => {...})` with no `reject` path. If `httpServer.listen()` encounters an error (e.g., EADDRINUSE), the listen callback never fires, the Promise never resolves, and `server.ready()` hangs indefinitely. This is a pre-existing pattern limitation discovered during SPEC-038 review.
- **Root Cause:** The Promise only has a `resolve` callback. Listen errors go to `httpServer.on('error')` in ServerCoordinator, but that handler doesn't unblock the startup Promise chain.
- **Fix:** Add `httpServer.on('error', reject)` inside the Promise constructor in `network.start()`, so listen failures propagate through the `Promise.all` startup chain in `ServerFactory.create()`. The ServerCoordinator should then catch the rejection from `ready()` and log/handle gracefully.
- **Key Files:**
  - `packages/server/src/modules/network-module.ts:95` — Promise needs reject path
  - `packages/server/src/ServerFactory.ts:438` — `Promise.all([networkReady, clusterReady])` will propagate rejection
  - `packages/server/src/ServerCoordinator.ts` — `ready()` caller should handle rejection
- **Verification:** Start server with a port already in use → should reject `ready()` with EADDRINUSE error instead of hanging
- **Dependencies:** SPEC-038 (port capture fix — now complete)

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
| -1. Stability | 6 | ~4-5 days | Post-v0.11.0 test regression fixes |
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
| ★ | ~~TODO-051~~ → SPEC-038 | -1 | 1-2 days | TODO-052, TODO-053 | 🔴 P0 |
| ★ | TODO-052 | -1 | 0.5 day | — | 🔴 P1 |
| ★ | TODO-053 | -1 | 1 day | — | 🟡 P1 |
| ★ | TODO-055 | -1 | 1 day | — | 🟡 P1 |
| ★ | TODO-056 | -1 | 2 hours | — | 🔴 P1 |
| ★ | TODO-054 | -1 | 1 day | — | 🟡 P2 |
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

*Reordered by technical dependencies on 2026-02-07. Wave -1 added on 2026-02-08 for post-release test stability fixes.*
