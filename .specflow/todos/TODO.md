# To-Do List

**Last updated:** 2026-02-06
**Source:** Migrated from PROMPTS directory, reordered by technical dependencies and business impact

---

## Wave 1: Market Expansion

*Goal: Unlock serverless deployments, improve cluster utilization*

### TODO-026: HTTP Sync Fallback (Serverless)
- **Priority:** 🔴 High (new market)
- **Complexity:** Medium
- **Context:** [reference/TURSO_INSIGHTS.md](../reference/TURSO_INSIGHTS.md) (Section 1)
- **Summary:** Add HTTP request-response sync protocol as fallback for serverless environments
- **Why Now:** Opens Vercel Edge, AWS Lambda, Cloudflare Workers market
- **Problem:** WebSocket-only limits deployment to long-lived connections
- **Solution:**
  - `POST /sync` endpoint for push operations + receive deltas
  - Stateless design for AWS Lambda, Vercel Edge, Cloudflare Workers
  - Automatic protocol negotiation (WebSocket → HTTP)
- **Target Market:** Frontend Cloud (Vercel Edge Functions without dedicated VPS)
- **Effort:** 2-3 weeks

---

### TODO-029: Partition Pruning
- **Priority:** 🟡 Medium
- **Complexity:** Medium
- **Context:** [reference/HAZELCAST_QUICK_WINS.md](../reference/HAZELCAST_QUICK_WINS.md)
- **Summary:** Skip partitions that can't contain matching records
- **Why Now:** Required for efficient distributed queries at scale; prerequisite for TODO-025 (DAG Executor)
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
- **Why Now:** Full cluster utilization, reduces coordinator bottleneck
- **Key Features:**
  - Smart client routing to partition owners
  - Client-side failover on node failure
  - Partition map synchronization
  - ConnectionPool with health checks
- **Target:** 50,000+ ops/sec in cluster mode
- **Effort:** ~16 hours (7 tasks)
- **Files to modify:** TopGunClient.ts, SyncEngine.ts, ClusterClient.ts, ConnectionPool.ts

---

## Wave 2: Core Infrastructure

*Goal: Enable slow backends, unlock distributed query processing*

### TODO-033: AsyncStorageWrapper (Write-Behind)
- **Priority:** 🟡 Medium
- **Complexity:** Medium
- **Context:** [reference/topgun-rocksdb.md](../reference/topgun-rocksdb.md)
- **Summary:** Implement Hazelcast-style Write-Behind pattern for slow storage backends
- **Why Now:** Enables S3/slow storage backends without latency impact
- **Key Features:**
  - Staging Area: In-memory buffer for Read-Your-Writes consistency
  - Write Coalescing: Merge multiple updates to same key
  - Batch Flush: Periodic flush to storage (5s intervals)
  - Retry Queue: Handle storage failures gracefully
- **Effort:** 2-3 weeks

---

### TODO-025: DAG Executor for Distributed Queries
- **Priority:** 🟡 Medium
- **Complexity:** Large
- **Context:** [reference/HAZELCAST_DAG_EXECUTOR_SPEC.md](../reference/HAZELCAST_DAG_EXECUTOR_SPEC.md)
- **Additional:** [reference/HAZELCAST_ARCHITECTURE_COMPARISON.md](../reference/HAZELCAST_ARCHITECTURE_COMPARISON.md)
- **Summary:** Implement Hazelcast-style DAG executor for distributed query processing
- **Why Here:** Partition Pruning (TODO-029) must be completed first
- **Key Features:**
  - DAG structure with Vertex/Edge graph
  - 3-tier processor model: Source → Transform → Sink
  - Partition-aware execution
  - Backpressure handling
- **Architecture Pattern:** Processors exchange data via Outbox/Inbox queues
- **Effort:** 4-6 weeks
- **Dependencies:** TODO-029 (Partition Pruning)

---

## Wave 3: Advanced Features

*Goal: AI capabilities, performance optimization, extensibility*

### TODO-039: Vector Search
- **Priority:** 🟡 Medium
- **Complexity:** Large
- **Context:** [reference/PHASE_15_VECTOR_SEARCH_SPEC.md](../reference/PHASE_15_VECTOR_SEARCH_SPEC.md)
- **Summary:** Semantic vector search with local embeddings (transformers.js)
- **Why Here:** AI/semantic capability, differentiator
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
- **Why Here:** Benefits from having DAG Executor (TODO-025) as a prime WASM candidate
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
- **Why Here:** Enables community contributions, smaller core bundle
- **Example Extensions:**
  ```
  @topgunbuild/ext-crypto      # Encryption at rest
  @topgunbuild/ext-compress    # Compression (zstd, brotli)
  @topgunbuild/ext-audit       # Audit logging
  @topgunbuild/ext-geo         # Geospatial queries
  ```
- **Effort:** 2-3 weeks for infrastructure

---

## Wave 4: Documentation

*Goal: Document public APIs when convenient*

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

## Wave 5: Enterprise (Deferred)

*Goal: Enterprise features, major architectural changes*
*Defer until Waves 1-4 complete*

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

### By Wave

| Wave | Items | Total Effort | Focus |
|------|-------|--------------|-------|
| 1. Market Expansion | 3 | ~4 weeks | Serverless + cluster |
| 2. Core Infrastructure | 2 | ~7 weeks | Storage + DAG |
| 3. Advanced Features | 3 | ~10 weeks | Vector + WASM + Extensions |
| 4. Documentation | 1 | ~1 day | DST docs |
| 5. Enterprise | 5 | ~20+ weeks | Tenancy + S3 + Time-travel |

### Execution Order

| # | TODO | Wave | Effort | ROI |
|---|------|------|--------|-----|
| 1 | TODO-026 | 1 | 2-3 weeks | 🔴 High |
| 2 | TODO-029 | 1 | 1 week | 🟡 Medium |
| 3 | TODO-023 | 1 | ~16 hours | 🟡 Medium |
| 4 | TODO-033 | 2 | 2-3 weeks | 🟡 Medium |
| 5 | TODO-025 | 2 | 4-6 weeks | 🟡 Medium |
| 6 | TODO-039 | 3 | 4 weeks | 🟡 Medium |
| 7 | TODO-034 | 3 | 4-6 weeks | 🟡 Medium |
| 8 | TODO-036 | 3 | 2-3 weeks | 🟢 Low |
| 9 | TODO-045 | 4 | 0.5-1 day | 🟢 Low |
| 10 | TODO-041 | 5 | Large | 🔵 Deferred |
| 11 | TODO-043 | 5 | 6-8 weeks | 🔵 Deferred |
| 12 | TODO-044 | 5 | 4-6 weeks | 🔵 Deferred |
| 13 | TODO-040 | 5 | Large | 🔵 Deferred |
| 14 | TODO-042 | 5 | Very Large | ⚠️ Risk |

### Context Files

| TODO | Context File | Lines |
|------|--------------|-------|
| TODO-026 | TURSO_INSIGHTS.md (Section 1) | 482 |
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

*Reordered by technical dependencies and business impact on 2026-02-06.*
