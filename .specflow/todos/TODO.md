# To-Do List

**Last updated:** 2026-02-05
**Source:** Migrated from PROMPTS directory via IDEA_REGISTRY verification

---

## High Priority (4)

### TODO-023: Client Cluster Smart Routing
- **Complexity:** Large
- **Context:** [reference/PHASE_4.5_CLIENT_CLUSTER_SPEC.md](../reference/PHASE_4.5_CLIENT_CLUSTER_SPEC.md)
- **Summary:** Integrate ClusterClient with TopGunClient for transparent partition routing
- **Key Features:**
  - Smart client routing to partition owners
  - Client-side failover on node failure
  - Partition map synchronization
  - ConnectionPool with health checks
- **Target:** 50,000+ ops/sec in cluster mode
- **Effort:** ~16 hours (7 tasks)
- **Files to modify:** TopGunClient.ts, SyncEngine.ts, ClusterClient.ts, ConnectionPool.ts

---

### TODO-025: DAG Executor for Distributed Queries
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
- **Effort:** High (4-6 weeks)
- **Dependencies:** Phase 4 clustering

---

### TODO-026: HTTP Sync Fallback (Serverless)
- **Complexity:** Medium
- **Context:** [reference/TURSO_INSIGHTS.md](../reference/TURSO_INSIGHTS.md) (Section 1)
- **Summary:** Add HTTP request-response sync protocol as fallback for serverless environments
- **Problem:** WebSocket-only limits deployment to long-lived connections
- **Solution:**
  - `POST /sync` endpoint for push operations + receive deltas
  - Stateless design for AWS Lambda, Vercel Edge, Cloudflare Workers
  - Automatic protocol negotiation (WebSocket → HTTP)
- **Target Market:** Frontend Cloud (Vercel Edge Functions without dedicated VPS)
- **Effort:** 2-3 weeks

---

### TODO-027: Deterministic Simulation Testing (DST)
- **Complexity:** Medium
- **Context:** [reference/TURSO_INSIGHTS.md](../reference/TURSO_INSIGHTS.md) (Section 2)
- **Summary:** Implement deterministic simulation testing for distributed bug reproduction
- **Problem:** Race conditions, network partitions, clock drift are hard to reproduce
- **Solution:**
  - Seeded randomness for reproducible test runs
  - Virtual clock (no `Date.now()` in core)
  - Simulated network (packet loss, latency, partitions)
  - Property-based invariant checking
- **Key Components:**
  ```
  packages/core/src/testing/
  ├── DeterministicSimulator.ts  # Seeded RNG, virtual clock
  ├── VirtualNetwork.ts          # Simulated network layer
  ├── InvariantChecker.ts        # Property-based assertions
  └── ScenarioRunner.ts          # Reproducible test scenarios
  ```
- **Reference:** Turso's `/simulator/` (12K LOC), Antithesis DST
- **Effort:** 2-3 weeks

---

## Medium Priority (5)

### TODO-028: Point Lookup Optimization
- **Complexity:** Low
- **Context:** [reference/HAZELCAST_QUICK_WINS.md](../reference/HAZELCAST_QUICK_WINS.md)
- **Summary:** Optimize single-key lookups bypassing query planning
- **Current:** All queries go through QueryPlanner even for `equal('id', 'foo')`
- **Solution:** Detect point lookups and use direct HashIndex access
- **Expected:** 2-3x speedup for single-key queries
- **Effort:** Low (1-2 days)

---

### TODO-029: Partition Pruning
- **Complexity:** Medium
- **Context:** [reference/HAZELCAST_QUICK_WINS.md](../reference/HAZELCAST_QUICK_WINS.md)
- **Summary:** Skip partitions that can't contain matching records
- **Current:** Distributed queries scan all partitions
- **Solution:** Use partition key to determine relevant partitions
- **Example:** Query `tenantId = 'abc'` → only scan partitions where hash('abc') maps
- **Effort:** Medium (1 week)

---

### TODO-033: AsyncStorageWrapper (Write-Behind)
- **Complexity:** Medium
- **Context:** [reference/topgun-rocksdb.md](../reference/topgun-rocksdb.md)
- **Summary:** Implement Hazelcast-style Write-Behind pattern for slow storage backends
- **Key Features:**
  - Staging Area: In-memory buffer for Read-Your-Writes consistency
  - Write Coalescing: Merge multiple updates to same key
  - Batch Flush: Periodic flush to storage (5s intervals)
  - Retry Queue: Handle storage failures gracefully
- **Use Case:** Enables S3/slow storage backends without latency impact
- **Effort:** 2-3 weeks

---

### TODO-034: Rust/WASM Hot Path Migration
- **Complexity:** Large
- **Context:** [reference/RUST_WASM_ANALYSIS.md](../reference/RUST_WASM_ANALYSIS.md)
- **Summary:** Migrate CPU-intensive hot paths to Rust/WASM
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
- **Expected:** 2-3x speedup on CPU-intensive operations
- **Effort:** 4-6 weeks total

---

### TODO-036: Pluggable Extension System
- **Complexity:** Medium
- **Context:** [reference/TURSO_INSIGHTS.md](../reference/TURSO_INSIGHTS.md) (Section 5)
- **Summary:** Modular extension system for optional features
- **Example Extensions:**
  ```
  @topgunbuild/ext-crypto      # Encryption at rest
  @topgunbuild/ext-compress    # Compression (zstd, brotli)
  @topgunbuild/ext-audit       # Audit logging
  @topgunbuild/ext-geo         # Geospatial queries
  ```
- **Benefits:** Smaller core bundle, community contributions
- **Effort:** 2-3 weeks for infrastructure

---

## Low Priority (8)

### TODO-037: Network-aware Cost Model
- **Complexity:** Low
- **Context:** [reference/HAZELCAST_QUICK_WINS.md](../reference/HAZELCAST_QUICK_WINS.md)
- **Summary:** Factor network latency into query cost estimation
- **Current:** Cost model considers only CPU/memory
- **Solution:** Add network hop cost to distributed query planning

---

### TODO-038: Index Hints
- **Complexity:** Low
- **Context:** [reference/HAZELCAST_QUICK_WINS.md](../reference/HAZELCAST_QUICK_WINS.md)
- **Summary:** Allow explicit index hints in queries
- **Example:** `query.hint({ useIndex: 'status_idx' })`
- **Use Case:** Override query planner when developer knows better

---

### TODO-039: Vector Search
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
- **Dependencies:** Phase 12 (Hybrid Search), Phase 14 (Distributed Search)

---

### TODO-040: Tiered Storage (Hot/Cold)
- **Complexity:** Large
- **Context:** [reference/topgun-rocksdb.md](../reference/topgun-rocksdb.md)
- **Summary:** Hot data in memory/Redis, cold data in S3/cheap storage
- **Features:** Transparent migration based on access patterns
- **Use Case:** Cost reduction for large datasets
- **Priority:** Enterprise feature

---

### TODO-041: Multi-Tenancy
- **Complexity:** Large
- **Context:** [reference/PHASE_5_MULTI_TENANCY_SPEC.md](../reference/PHASE_5_MULTI_TENANCY_SPEC.md)
- **Summary:** Per-tenant isolation, quotas, billing
- **Key Features:**
  - Tenant context in all operations
  - Resource quotas (storage, connections, ops/sec)
  - Tenant-aware partitioning
- **Priority:** Enterprise feature

---

### TODO-042: DBSP Incremental Views
- **Complexity:** Very Large
- **Context:** [reference/TURSO_INSIGHTS.md](../reference/TURSO_INSIGHTS.md) (Section 4)
- **Summary:** Implement DBSP (Database Stream Processing) for delta-based query updates
- **Problem:** LiveQueryManager recomputes queries on every change
- **Solution:** Compile queries to streaming operators, maintain incremental state
- **Warning:** High risk - could become 6-month compiler project
- **Alternative:** Start with "React Signals" style fine-grained reactivity
- **Reference:** Turso `/core/incremental/`, Materialize, differential-dataflow

---

### TODO-043: S3 Bottomless Storage
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
- **Complexity:** Large
- **Context:** [reference/TURSO_INSIGHTS.md](../reference/TURSO_INSIGHTS.md) (Section 8)
- **Summary:** Query historical state with valid time + transaction time
- **Example:** `client.query('tasks', filter, { asOf: '2025-01-01T00:00:00Z' })`
- **Benefits:** Point-in-time debugging, audit trails, undo/redo
- **Dependencies:** TODO-043 (S3 Bottomless Storage)
- **Effort:** 4-6 weeks

---

## Quick Reference: Context Files

| TODO | Context File | Lines |
|------|--------------|-------|
| TODO-023 | PHASE_4.5_CLIENT_CLUSTER_SPEC.md | 336 |
| TODO-025 | HAZELCAST_DAG_EXECUTOR_SPEC.md | 700+ |
| TODO-026 | TURSO_INSIGHTS.md (Section 1) | 482 |
| TODO-027 | TURSO_INSIGHTS.md (Section 2) | 482 |
| TODO-028 | HAZELCAST_QUICK_WINS.md | 400+ |
| TODO-029 | HAZELCAST_QUICK_WINS.md | 400+ |
| TODO-033 | topgun-rocksdb.md | 650+ |
| TODO-034 | RUST_WASM_ANALYSIS.md | 1127 |
| TODO-036 | TURSO_INSIGHTS.md (Section 5) | 482 |
| TODO-039 | PHASE_15_VECTOR_SEARCH_SPEC.md | 1696 |
| TODO-040 | topgun-rocksdb.md | 650+ |
| TODO-041 | PHASE_5_MULTI_TENANCY_SPEC.md | 700+ |
| TODO-042 | TURSO_INSIGHTS.md (Section 4) | 482 |
| TODO-043 | TURSO_INSIGHTS.md (Section 7) | 482 |
| TODO-044 | TURSO_INSIGHTS.md (Section 8) | 482 |

---

## Recommended Implementation Order

1. **Immediate (High ROI):**
   - TODO-027: DST Testing (quality improvement, safety net)
   - TODO-026: HTTP Sync Fallback (new market: serverless)

2. **Short-term (Performance):**
   - TODO-028: Point Lookup Optimization (quick win)
   - TODO-023: Client Cluster Smart Routing (full cluster utilization)

3. **Medium-term (Features):**
   - TODO-025: DAG Executor (distributed queries)
   - TODO-039: Vector Search (AI/semantic capability)

4. **Long-term (Enterprise):**
   - TODO-041: Multi-Tenancy
   - TODO-043: S3 Bottomless Storage

---

*Migrated from PROMPTS directory on 2026-02-05. Context files preserved in `.specflow/reference/`.*
