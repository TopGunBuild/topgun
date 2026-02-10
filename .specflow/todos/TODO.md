# To-Do List

**Last updated:** 2026-02-10 (TODO-050 converted to SPEC-046)
**Source:** Migrated from PROMPTS directory, reordered by technical dependencies

---

## Wave -1: Post-Release Test Stability (v0.11.0 regression fixes)

*Goal: All server test suites pass — zero ignored failures*

### ~~TODO-058: Rewrite or remove Resilience.test.ts split-brain recovery test~~ → SPEC-044
- **Status:** Converted to [SPEC-044](.specflow/specs/SPEC-044.md)

---

### ~~TODO-057: Fix SearchCoordinator batched LEAVE notification bug~~ → SPEC-043
- **Status:** Converted to [SPEC-043](.specflow/specs/SPEC-043.md)

---

### ~~TODO-051: Fix WebSocket client auth handshake after ServerFactory modular refactoring~~ → SPEC-038
- **Status:** Converted to [SPEC-038](.specflow/specs/SPEC-038.md)

---

### ~~TODO-052: Verify interceptor pipeline and TLS setup work in production after modular refactoring~~ → SPEC-040
- **Status:** Converted to [SPEC-040](.specflow/specs/SPEC-040.md)

---

### ~~TODO-053: Fix DistributedSearch cluster event routing and GC broadcast gap~~ → SPEC-041 ✅
- **Status:** Completed via [SPEC-041](.specflow/archive/SPEC-041.md)

---

### ~~TODO-054: Fix ProcessorSandbox test hang + update 12 docs files with ServerFactory.create()~~ → SPEC-045
- **Status:** Converted to [SPEC-045](.specflow/specs/SPEC-045.md)

---

### ~~TODO-055: Harden timing-sensitive server tests — replace setTimeout with polling~~ → SPEC-042
- **Status:** Converted to [SPEC-042](.specflow/specs/SPEC-042.md)

---

### ~~TODO-056: Add reject path to network.start() Promise for listen failure handling~~ → SPEC-039
- **Status:** Converted to [SPEC-039](.specflow/specs/SPEC-039.md)

---

## ~~Wave 0: Foundation Refactoring~~ → SPEC-046

*Goal: Fix abstraction leaks that block transport evolution*

### ~~TODO-050: IConnection Abstraction~~ → SPEC-046
- **Status:** Converted to [SPEC-046](.specflow/specs/SPEC-046.md)

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

## Wave 2: Transport Evolution → DEFERRED TO RUST

*Goal: Close the real-time gap for serverless, enable cluster HTTP*
*Decision (2026-02-10): Entire wave deferred to Rust server rewrite. SSE is ~50 lines in axum; Cluster HTTP routing benefits from Rust's async model. See [RUST_SERVER_MIGRATION_RESEARCH.md](../reference/RUST_SERVER_MIGRATION_RESEARCH.md)*

### TODO-048: SSE Push for HTTP Sync → DEFERRED TO RUST
- **Priority:** 🔵 Deferred to Rust
- **Complexity:** Medium (trivial in Rust: ~50 lines in axum)
- **Context:** Extends SPEC-036 (HTTP Sync Protocol)
- **Summary:** Add Server-Sent Events transport for real-time push in serverless environments
- **Why:** HTTP polling introduces latency proportional to `pollIntervalMs`. SSE enables server-initiated push without WebSocket, closing the real-time gap for serverless deployments.
- **Architecture:**
  - Client POSTs writes to `POST /sync` (existing)
  - Client receives real-time updates via `GET /events` (SSE stream)
  - New `SsePushProvider` implements `IConnectionProvider`
  - `AutoConnectionProvider` gains a third tier: WS → SSE → HTTP polling
- **Platform Support:** Vercel Edge (streaming), Cloudflare Workers (with Durable Objects), AWS Lambda (response streaming)
- **Effort:** 2-3 days in Rust (vs 2-3 weeks in TS)
- **Dependencies:** TODO-050 (IConnection abstraction)

---

### TODO-049: Cluster-Aware HTTP Routing → DEFERRED TO RUST
- **Priority:** 🔵 Deferred to Rust
- **Complexity:** Medium
- **Context:** Extends SPEC-036 (HTTP Sync Protocol), relates to TODO-023 (Client Cluster Smart Routing)
- **Summary:** Enable `HttpSyncHandler` to route sync requests to partition owners in a cluster
- **Why:** Currently HTTP sync runs standalone against a single node's data. In cluster mode without shared PostgreSQL, a client sees only data from the node it hits. This makes HTTP sync unusable for in-memory-only clusters.
- **Architecture:**
  - `HttpSyncHandler` queries `PartitionService` to find partition owner per map key
  - Forwards delta computation to owner node via internal cluster protocol
  - Merges responses from multiple partition owners into single HTTP response
- **Effort:** 1-2 weeks in Rust
- **Dependencies:** TODO-050 (IConnection abstraction), TODO-029 (Partition Pruning — recommended)

---

## Wave 3: Storage Infrastructure → DEFERRED TO RUST

*Goal: Enable slow backends, unlock distributed query processing*
*Decision (2026-02-10): Both items deferred to Rust server rewrite. See [RUST_SERVER_MIGRATION_RESEARCH.md](../reference/RUST_SERVER_MIGRATION_RESEARCH.md)*

### TODO-033: AsyncStorageWrapper (Write-Behind) → DEFERRED TO RUST
- **Priority:** 🔵 Deferred to Rust
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

### TODO-025: DAG Executor for Distributed Queries → DEFERRED TO RUST
- **Priority:** 🔵 Deferred to Rust
- **Complexity:** Large
- **Context:** [reference/HAZELCAST_DAG_EXECUTOR_SPEC.md](../reference/HAZELCAST_DAG_EXECUTOR_SPEC.md)
- **Additional:** [reference/HAZELCAST_ARCHITECTURE_COMPARISON.md](../reference/HAZELCAST_ARCHITECTURE_COMPARISON.md)
- **Summary:** Implement Hazelcast-style DAG executor for distributed query processing
- **Why deferred:** HAZELCAST_DAG_EXECUTOR_SPEC.md (700+ lines) provides complete interfaces. Hazelcast Java source available as reference. Rust `Future::poll()` maps naturally to Cooperative Tasklet model. TS prototype would be discarded after Rust rewrite.
- **Key Features:**
  - DAG structure with Vertex/Edge graph
  - 3-tier processor model: Source → Transform → Sink
  - Partition-aware execution
  - Backpressure handling
- **Architecture Pattern:** Processors exchange data via Outbox/Inbox queues
- **Effort:** 2-3 weeks in Rust (vs 4-6 weeks in TS)
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

### ~~TODO-034: Rust/WASM Hot Path Migration~~ SUPERSEDED
- **Status:** Superseded by full Rust server migration (2026-02-10)
- **Reason:** Full Rust server rewrite makes partial WASM hot-path approach obsolete. All CPU-intensive operations will be native Rust. See [RUST_SERVER_MIGRATION_RESEARCH.md](../reference/RUST_SERVER_MIGRATION_RESEARCH.md).

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

### ~~TODO-042: DBSP Incremental Views~~ ELIMINATED
- **Status:** Removed from roadmap (2026-02-10)
- **Reason:** Not needed for TopGun's model. Existing StandingQueryRegistry (O(1) affected query detection) and ReverseQueryIndex (field-based candidate filtering) already provide incremental query notification for supported query types (filter + sort + limit). DBSP adds value only for complex aggregations (GROUP BY, SUM, AVG, JOIN) which TopGun explicitly doesn't support. If aggregations needed, DAG Executor (TODO-025) handles them via streaming processors. Origin was Turso, not Hazelcast.
- **Reference:** [RUST_SERVER_MIGRATION_RESEARCH.md](../reference/RUST_SERVER_MIGRATION_RESEARCH.md) (Section 13)

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
| 2. Transport | 2 | Deferred to Rust | SSE, cluster HTTP |
| 3. Storage + DAG | 2 | Deferred to Rust | Write-behind, DAG |
| 4. Advanced | 3 | Deferred to Rust | Vector, extensions (WASM superseded) |
| 5. Documentation | 1 | 0.5-1 day | DST docs (TODO-047 done) |
| 6. Enterprise | 4 | Deferred to Rust | Tenancy, S3, time-travel (DBSP eliminated) |

### Execution Order (by technical dependency)

| # | TODO | Wave | Effort | Unlocks | Priority |
|---|------|------|--------|---------|----------|
| ★ | ~~TODO-051~~ → SPEC-038 | -1 | 1-2 days | TODO-052, TODO-053 | 🔴 P0 |
| ★ | ~~TODO-052~~ → SPEC-040 | -1 | 0.5 day | — | 🔴 P1 |
| ★ | ~~TODO-053~~ → SPEC-041 | -1 | 1 day | — | 🟡 P1 |
| ★ | ~~TODO-055~~ → SPEC-042 | -1 | 1 day | — | 🟡 P1 |
| ★ | ~~TODO-056~~ → SPEC-039 | -1 | 2 hours | — | 🔴 P1 |
| ★ | ~~TODO-057~~ → SPEC-043 | -1 | 0.5 day | — | 🔴 P1 |
| ★ | ~~TODO-058~~ → SPEC-044 | -1 | 0.5 day | — | 🟡 P2 |
| ★ | ~~TODO-054~~ → SPEC-045 | -1 | 1 day | — | 🟡 P2 |
| 1 | ~~TODO-050~~ → SPEC-046 | 0 | 4-6 hours | TODO-048, TODO-049 | 🔴 High |
| 2 | TODO-029 | 1 | 1 week | TODO-025, TODO-049 | 🟡 Medium |
| 3 | TODO-023 | 1 | ~16 hours | — (independent) | 🟡 Medium |
| 4 | TODO-048 | 2 | Rust phase | — | 🔵 Deferred to Rust |
| 5 | TODO-049 | 2 | Rust phase | — | 🔵 Deferred to Rust |
| 6 | TODO-033 | 3 | Rust phase | TODO-043, TODO-040 | 🔵 Deferred to Rust |
| 7 | TODO-025 | 3 | Rust phase | — | 🔵 Deferred to Rust |
| 8 | TODO-039 | 4 | Rust phase | — | 🔵 Deferred to Rust |
| ~~9~~ | ~~TODO-034~~ | ~~4~~ | ~~Superseded~~ | — | Superseded (2026-02-10) |
| 10 | TODO-036 | 4 | Rust phase | — | 🔵 Deferred to Rust |
| 11 | TODO-045 | 5 | 0.5-1 day | — | 🟢 Low |
| 12 | TODO-041 | 6 | Large | — | 🔵 Deferred |
| 13 | TODO-043 | 6 | 6-8 weeks | TODO-044 | 🔵 Deferred |
| 14 | TODO-044 | 6 | 4-6 weeks | — | 🔵 Deferred |
| 15 | TODO-040 | 6 | Large | — | 🔵 Deferred |
| ~~16~~ | ~~TODO-042~~ | ~~6~~ | ~~Eliminated~~ | — | Removed (2026-02-10) |

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
| ~~TODO-042~~ | ~~TURSO_INSIGHTS.md (Section 4)~~ | Eliminated |

---

### Rust Server Migration

Full migration research completed 2026-02-10. See [RUST_SERVER_MIGRATION_RESEARCH.md](../reference/RUST_SERVER_MIGRATION_RESEARCH.md) for:
- Architecture mapping (TypeScript → Rust)
- TODO impact analysis (which TODOs to implement in TS vs Rust)
- 5 key trait abstractions to prevent rework
- Timeline estimates (14-20 weeks Rust rewrite with AI agents)
- Strategy: Complete Wave 0-1 in TS (3-4 weeks) → Rust server rewrite (includes DAG, SSE, Cluster HTTP, all remaining TODOs)

---

*Reordered by technical dependencies on 2026-02-07. Wave -1 added on 2026-02-08 for post-release test stability fixes. TODO-042 eliminated and Rust migration reference added on 2026-02-10. TODO-050 converted to SPEC-046 on 2026-02-10. Waves 2-4 deferred to Rust, TODO-034 superseded, strategy revised to Wave 0-1 only in TS on 2026-02-10.*
