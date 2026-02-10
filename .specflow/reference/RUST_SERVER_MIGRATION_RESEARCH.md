# Rust Server Migration Research

> **Date:** 2026-02-10
> **Status:** Research Complete
> **Context:** Analysis of migrating `@topgunbuild/server` from TypeScript/Node.js to Rust
> **Supersedes:** Extends RUST_WASM_ANALYSIS.md (which covers hot-path WASM only)

---

## Executive Summary

| Question | Answer |
|----------|--------|
| **Was TypeScript a mistake?** | No — enabled rapid prototyping (45 specs in ~5 days). Correct for current stage. |
| **Can Node.js be world-class?** | No — GC pauses, single-threaded event loop, V8 heap limits cap at ~50K ops/sec/node |
| **Is current architecture portable to Rust?** | Yes — modular DI, clean interfaces, 70% direct mapping |
| **What's the recommended approach?** | Hybrid: finish TS Wave 0-2, then rewrite server in Rust using TS as spec |
| **Estimated timeline with AI agents?** | 12-18 weeks for Rust server |
| **Risk of double-rework?** | Manageable — 5 trait abstractions prevent 95% of rework |

---

## Table of Contents

1. [Why Migrate to Rust](#1-why-migrate-to-rust)
2. [Current Architecture Assessment](#2-current-architecture-assessment)
3. [Rust Ecosystem Readiness](#3-rust-ecosystem-readiness)
4. [Architecture Mapping: TypeScript to Rust](#4-architecture-mapping-typescript-to-rust)
5. [TODO Impact Analysis](#5-todo-impact-analysis)
6. [Migration Strategy](#6-migration-strategy)
7. [Trait Abstractions to Design Upfront](#7-trait-abstractions-to-design-upfront)
8. [Monorepo Structure](#8-monorepo-structure)
9. [Package Impact Matrix](#9-package-impact-matrix)
10. [Timeline Estimates](#10-timeline-estimates)
11. [Risk Assessment](#11-risk-assessment)
12. [Key Decisions and Rationale](#12-key-decisions-and-rationale)
13. [Eliminated TODOs](#13-eliminated-todos)

---

## 1. Why Migrate to Rust

### Node.js Fundamental Limitations

| Aspect | Node.js | Rust | Hazelcast (Java) |
|--------|---------|------|-------------------|
| **Parallelism** | 1 event loop + Worker threads (data copying) | True threads with zero-copy shared memory | JVM threads + off-heap |
| **GC** | V8 stop-the-world pauses on large heap (>4GB) | No GC, deterministic latency | G1/ZGC (still has pauses) |
| **Memory** | V8 heap ~4GB practical limit | Byte-level control, arena allocators | Off-heap (sun.misc.Unsafe) |
| **Serialization** | msgpackr (good, but JS overhead) | serde + zero-copy (rkyv) | Compact serialization |
| **SIMD** | No direct access | Full support (std::simd) | JVM auto-vectorization |
| **CPU throughput** | ~100-300K ops/sec per core | ~1-5M ops/sec per core | ~500K-2M ops/sec per core |
| **Cold start** | ~500ms | ~10ms | ~2-5s |
| **Binary size** | node_modules (~200MB) | Single binary (~10-20MB) | JVM + JAR (~100MB+) |

### Target Performance

- Current Node.js: 10-50K ops/sec per node
- Target Rust: 100K-1M+ ops/sec per node
- Required for "best-in-class in-memory data grid for small/medium workloads"

---

## 2. Current Architecture Assessment

### Server Package Statistics

- **Production source**: 176 files, ~45,539 LOC TypeScript
- **Test files**: 60 files, ~19,108 LOC
- **Modules**: 9 factory modules with dependency injection
- **Message handlers**: 26, organized in 8 domains
- **Cluster messages**: 26 types
- **Partitions**: 271 (consistent hashing)

### Abstraction Boundary Quality

| Boundary | Quality | Notes |
|----------|---------|-------|
| StorageManager -> IServerStorage | Excellent | Clean 8-method interface, decorator-friendly |
| WorkerPool -> Tasks | Excellent | Generic task interface, no CRDT coupling |
| ReplicationPipeline -> OperationApplier | Good | Callback-based inversion of control |
| QueryRegistry -> Matcher | Good | But coupled to LWWMap/ORMap types |
| Handlers -> Each other | Medium | Deep dependency chains (5+ levels) |
| Modules -> ServerFactory | Good | Strict acyclic dependency order |

### Key Architecture Patterns (Rust-Friendly)

1. **Module factory pattern**: Each domain has factory function with explicit DI
2. **Deferred startup**: Create resources first, bind ports later
3. **Late binding**: Callbacks wired after all modules created
4. **Striped execution**: Partitioned event queues for parallel processing
5. **Domain grouping**: Handlers grouped for Actor Model portability (noted in STATE.md)
6. **DTO-based workers**: Plain objects, no class instances — ideal for trait boundaries

---

## 3. Rust Ecosystem Readiness

| TopGun Need | Rust Crate | Maturity | Notes |
|-------------|-----------|----------|-------|
| Async runtime | tokio | Production (AWS, Discord, Cloudflare) | Foundation |
| WebSocket | tokio-tungstenite, axum | Production | Better than ws |
| HTTP server | axum | Production | Built on hyper + tokio |
| PostgreSQL | sqlx, tokio-postgres | Production | Async, compile-time checked |
| Serialization | serde + rmp-serde (msgpack) | Production | Binary compatible with current |
| JWT | jsonwebtoken | Production | Same crate name as npm |
| Metrics | prometheus | Production | Same exposition format |
| Logging | tracing | Production | Superior to pino |
| Full-text search | tantivy | Production | Lucene-equivalent, better than custom BM25 |
| Consistent hashing | hashring | Stable | Direct replacement |
| CRDT | crdts, yrs | Stable | Or custom (recommended) |
| Sandboxing | wasmtime | Production (Bytecode Alliance) | Better than isolated-vm |
| Object storage | aws-sdk-s3, opendal | Production | For S3 Bottomless |
| Vector search | usearch | Growing | For TODO-039 |

**Verdict**: Complete coverage. Several components (tantivy, wasmtime, tracing) are upgrades over current JS equivalents.

### Serverless Support

| Platform | Rust Support | Cold Start |
|----------|-------------|------------|
| AWS Lambda | First-class (cargo-lambda) | ~10ms |
| Cloudflare Workers | WASM target | ~5ms |
| Fly.io | Native binary | N/A (always running) |
| Google Cloud Run | Docker container | ~50ms |
| Vercel Edge | WASM target | ~10ms |

---

## 4. Architecture Mapping: TypeScript to Rust

### Direct Mapping (Low Risk)

| TypeScript Component | Rust Equivalent | Complexity |
|---------------------|-----------------|-----------|
| Modules (core, cluster, storage...) | Rust modules / crates | Low |
| 26 Message Handlers | Trait-based handlers or Actor (Actix) | Medium |
| ServerCoordinator (routing) | Actor supervisor or tokio task router | Medium |
| WebSocket (ws) | tokio-tungstenite / axum | Low |
| ClusterManager (WS mesh) | tokio + custom protocol | Medium |
| PartitionService (271 partitions) | Identical, pure logic | Low |
| LWWMap / ORMap CRDTs | Direct port (deterministic logic) | Low |
| MerkleTree | Direct port + SIMD acceleration | Low |
| WorkerPool | tokio::spawn / rayon thread pool | Simpler than Node |
| ProcessorSandbox (isolated-vm) | wasmtime (sandboxed WASM) | Medium |
| PostgreSQL (pg) | sqlx / tokio-postgres | Low |
| JWT auth | jsonwebtoken crate | Low |
| BM25 search | tantivy (upgrade) | Improvement |
| Backpressure / striped executor | tokio channels + bounded queues | Improvement |
| Object pools (memory/) | Arena allocators, zero-alloc patterns | Improvement |

### Requires Rethinking

| Component | Why | Approach |
|-----------|-----|----------|
| Async model | Event loop -> tokio runtime | Different mental model |
| Error handling | Exceptions -> Result<T, E> | Better but different |
| Late binding callbacks | JS closures -> trait objects or channels | Channels preferred |
| Worker threads | V8 isolates with data copy -> shared memory threads | Major simplification |

### Not Needed in Rust

| Component | Why |
|-----------|-----|
| Worker thread pool for CPU ops | tokio + rayon handle this natively |
| Object pools for GC pressure | No GC = no pressure |
| Write coalescing for WS | Rust's zero-copy writes are fast enough |
| BackpressureRegulator | tokio bounded channels provide this inherently |

---

## 5. TODO Impact Analysis

### Risk Matrix

| TODO | Type | Breaks Interfaces? | Requires Rust Rework? | Risk | Recommendation |
|------|------|--------------------|-----------------------|------|----------------|
| TODO-033 AsyncStorage | Wrapper/decorator | No | No | **Low** | Implement in Rust directly |
| TODO-025 DAG Executor | New subsystem | Partially (new messages) | Partially | **Medium** | Prototype in TS first |
| TODO-043 S3 Bottomless | New IServerStorage impl | No | No | **Low** | Implement in Rust directly |
| TODO-040 Tiered Storage | StorageManager internals | Partially | Partially | **Medium** | Design trait upfront |
| TODO-042 DBSP | Write path change | Yes | Yes | **Eliminated** | Not needed (see below) |

### TODO-033: AsyncStorageWrapper (Write-Behind) — LOW RISK

Pure decorator around IServerStorage. Adds staging area, write coalescing, batch flush.

```
Handlers -> StorageManager -> AsyncStorageWrapper -> IServerStorage (Postgres/S3)
                                ↑ new layer (same interface)
```

In Rust: `AsyncStorageWrapper<S: ServerStorage>` decorator. Hazelcast Write-Behind pattern is language-agnostic.

**Impact on Rust migration**: Zero. Implement storage trait first, decorator wraps any impl.

### TODO-025: DAG Executor — MEDIUM RISK

Adds Hazelcast-style DAG engine for distributed queries. New subsystem with:
- DAG, Vertex, Edge structures
- Processor trait (cooperative/non-cooperative)
- New cluster messages (DAG_DATA, DAG_EXECUTE, DAG_COMPLETE)
- ClusterQueryCoordinator for distributed execution

**Why medium, not high**:
1. **Additive** — doesn't replace existing query path, adds parallel path
2. **Interfaces don't break** — StorageManager, IServerStorage, ReplicationPipeline unchanged
3. **New cluster messages** — extending protocol, not changing existing types
4. **Hazelcast audit confirms**: "Rust Future::poll() perfectly matches Cooperative Tasklet model"

**What to do**: Prototype DAG in TypeScript (4-6 weeks) to stabilize Processor trait and routing policies. Then port to Rust with high confidence.

**Beyond current plans**: Future extensions (window functions, checkpointing, streaming mode) won't break base DAG architecture if Processor trait is designed with async + cooperative flag.

### TODO-043: S3 Bottomless Storage — LOW RISK

Another IServerStorage implementation. Internally complex (WAL + snapshots + compaction) but externally clean.

```
S3BottomlessAdapter implements IServerStorage {
    // Internal: WAL -> batch flush -> S3 PUT
    // Startup: download snapshot -> replay WAL
    // Background: compaction
}
```

**Impact on Rust migration**: Zero. Implement ServerStorage trait, then add S3 impl. Rust libraries: aws-sdk-s3, opendal.

### TODO-040: Tiered Storage (Hot/Cold) — MEDIUM RISK

Changes StorageManager internals. Hot data in memory, cold data in S3/cheap storage with transparent migration.

**What changes**:
- StorageManager gets eviction policy (LRU/LFU/TTL)
- `get_map()` may trigger async promotion from cold tier
- Access tracker for promotion/demotion decisions

**External API unchanged**: `get_map()` and `get_map_async()` remain the same.

**For Rust**: Design MapProvider as async trait from day one:
```rust
#[async_trait]
trait MapProvider {
    async fn get_map(&self, name: &str) -> Option<Arc<CrdtMap>>;
    async fn get_or_load_map(&self, name: &str) -> Arc<CrdtMap>;
}
```
Simple in-memory impl first, tiered impl later — same trait.

### TODO-042: DBSP Incremental Views — ELIMINATED

**Origin**: Turso's `/core/incremental/` module (NOT from Hazelcast).

**Problem it solves**: LiveQueryManager recomputes queries on every change.

**Why not needed for TopGun**:
1. QueryRegistry already has **StandingQueryRegistry** — O(1) affected query detection for IndexedLWWMap
2. QueryRegistry already has **ReverseQueryIndex** — field-based candidate filtering
3. Both are already incremental (check only affected queries, not all)
4. DBSP adds value for complex aggregations (GROUP BY, SUM, AVG, JOIN) — TopGun explicitly doesn't support these
5. The TODO itself warns: "Risk of 6-month compiler project"
6. If aggregations are ever needed, DAG Executor (TODO-025) handles them via streaming processors

**Decision**: Remove from roadmap. Existing notification mechanisms are sufficient.

---

## 6. Migration Strategy

### Phase 1: Complete TypeScript Wave 0-2 (~6-8 weeks)

Stabilize protocols and client API before starting Rust work.

| TODO | Wave | Effort | Value for Rust Migration |
|------|------|--------|--------------------------|
| TODO-050 IConnection | 0 | 4-6 hours | Stabilizes transport abstraction |
| TODO-029 Partition Pruning | 1 | 1 week | Clarifies partition query protocol |
| TODO-023 Client Cluster | 1 | ~16 hours | Stabilizes client-server contract |
| TODO-048 SSE Push | 2 | 2-3 weeks | Defines HTTP push protocol |
| TODO-049 Cluster HTTP | 2 | 2-3 weeks | Defines cluster HTTP routing |

### Phase 2: DAG Executor Prototype in TypeScript (~4-6 weeks, can overlap)

- Implement TODO-025 in TypeScript
- Stabilize: Processor interface, routing policies, cluster messages
- This becomes the behavioral spec for Rust DAG

### Phase 3: Rust Server Rewrite (~12-18 weeks)

Rewrite (not port) using TypeScript server as executable specification:

| Component | Effort | Source |
|-----------|--------|--------|
| Core CRDTs + HLC | 1-2 weeks | packages/core/ |
| Cluster protocol + partitions | 2-3 weeks | packages/server/src/cluster/ |
| Message handlers (26) | 2-3 weeks | packages/server/src/coordinator/ |
| Storage + PostgreSQL | 1 week | packages/server/src/storage/ |
| Networking (WS + HTTP + SSE) | 1-2 weeks | packages/server/src/modules/network-module.ts |
| Search (tantivy integration) | 1-2 weeks | packages/server/src/search/ |
| DAG Executor | 2-3 weeks | From Phase 2 TS prototype |
| Testing + integration | 3-4 weeks | packages/server/src/__tests__/ |

### Phase 4: Rust-native Features (post-migration)

Implement directly in Rust (no TS prototype needed):
- TODO-033 AsyncStorageWrapper
- TODO-043 S3 Bottomless Storage
- TODO-040 Tiered Storage
- TODO-039 Vector Search (with usearch)

---

## 7. Trait Abstractions to Design Upfront

These 5 traits prevent 95% of future rework. ~100 lines of Rust that gate architectural flexibility:

### 1. ServerStorage Trait (for TODO-033, TODO-043)

```rust
#[async_trait]
pub trait ServerStorage: Send + Sync {
    async fn load(&self, map: &str, key: &str) -> Result<Option<StorageValue>>;
    async fn load_all(&self, map: &str, keys: &[String]) -> Result<Vec<(String, StorageValue)>>;
    async fn load_all_keys(&self, map: &str) -> Result<Vec<String>>;
    async fn store(&self, map: &str, key: &str, value: &StorageValue) -> Result<()>;
    async fn store_all(&self, map: &str, records: &[(String, StorageValue)]) -> Result<()>;
    async fn delete(&self, map: &str, key: &str) -> Result<()>;
    async fn delete_all(&self, map: &str, keys: &[String]) -> Result<()>;
    async fn initialize(&self) -> Result<()>;
    async fn close(&self) -> Result<()>;
}
```

### 2. MapProvider Trait (for TODO-040 Tiered Storage)

```rust
#[async_trait]
pub trait MapProvider: Send + Sync {
    async fn get_map(&self, name: &str) -> Option<Arc<CrdtMap>>;
    async fn get_or_load_map(&self, name: &str, type_hint: MapType) -> Result<Arc<CrdtMap>>;
    fn has_map(&self, name: &str) -> bool;
}
```

### 3. QueryNotifier Trait (future-proofing write path)

```rust
pub trait QueryNotifier: Send + Sync {
    fn notify_change(
        &self,
        map_name: &str,
        key: &str,
        old_value: Option<&Value>,
        new_value: &Value,
    );
}
```

Passing `old_value` costs nothing now but enables delta-based optimizations later.

### 4. Processor Trait (for TODO-025 DAG Executor)

```rust
#[async_trait]
pub trait Processor: Send {
    async fn init(&mut self, ctx: ProcessorContext) -> Result<()>;
    /// Returns true when processing is complete
    async fn process(&mut self, ordinal: usize, inbox: &mut Inbox) -> Result<bool>;
    async fn complete(&mut self) -> Result<bool>;
    fn is_cooperative(&self) -> bool;
    async fn close(&mut self) -> Result<()>;
}
```

### 5. RequestContext with TenantId (for TODO-041 Multi-tenancy)

```rust
pub struct RequestContext {
    pub node_id: String,
    pub tenant_id: Option<String>,
    pub principal: Option<Principal>,
    pub trace_id: String,
}
```

Adding `tenant_id: Option<String>` costs nothing now, prevents pervasive refactoring later.

---

## 8. Monorepo Structure

```
topgun/
├── packages/
│   ├── core/              # TS — shared types, schemas (stays)
│   ├── client/            # TS — browser SDK (stays)
│   ├── server/            # TS — reference impl (stays as spec)
│   ├── react/             # TS (stays)
│   ├── adapters/          # TS (stays)
│   ├── native/            # N-API — absorbed into Rust crates
│   ├── server-rust/       # NEW: Rust server
│   │   ├── Cargo.toml
│   │   ├── src/
│   │   │   ├── main.rs
│   │   │   ├── coordinator/   # Message handlers
│   │   │   ├── cluster/       # ClusterManager, PartitionService
│   │   │   ├── storage/       # Storage trait + impls
│   │   │   ├── query/         # QueryRegistry, DAG executor
│   │   │   ├── search/        # tantivy integration
│   │   │   ├── network/       # axum + WebSocket
│   │   │   └── config/        # Configuration
│   │   └── tests/
│   └── core-rust/         # NEW: shared Rust types (CRDT, HLC, MerkleTree)
│       ├── Cargo.toml
│       └── src/
├── Cargo.toml             # Workspace root
├── pnpm-workspace.yaml    # Existing
└── package.json           # Existing
```

Cargo workspace + pnpm workspace in same repo. Standard practice (Turso, Deno, SurrealDB).

---

## 9. Package Impact Matrix

| Package | Impact | Reason |
|---------|--------|--------|
| `server` | **Full replacement** -> `server-rust` | Primary migration target |
| `core` | **Partial** — Rust version of CRDT/HLC | Shared algorithms in Rust crate |
| `client` | **Not affected** | Stays TypeScript for browser |
| `react` | **Not affected** | Depends on client, not server |
| `adapters` | **Not affected** | Client-side adapters (IndexedDB) |
| `native` | **Absorbed** | xxHash goes into core-rust |

**Client-server contract**: Message schemas (currently Zod in `@topgunbuild/core`) are the only coupling point. Binary compatibility maintained through msgpack format.

---

## 10. Timeline Estimates

### With AI Agents (Claude, etc.)

| Phase | Duration | Prerequisites |
|-------|----------|---------------|
| TS Wave 0-2 completion | 6-8 weeks | Current codebase |
| DAG Executor TS prototype | 4-6 weeks | Can overlap with above |
| Rust server rewrite | 12-18 weeks | TS spec complete |
| Rust-native features | 8-12 weeks | Post-migration |
| **Total to Rust server launch** | **~22-32 weeks** | |

### Without AI

Multiply by 2.5-3x (~55-90 weeks).

### Caveats

- AI generates Rust less confidently than TypeScript (more borrow checker iterations)
- First 2-3 weeks will be slow (toolchain setup, pattern establishment)
- Tests are the most labor-intensive part (behavioral equivalence, not just LOC)
- Rust compiler errors often require architectural decisions, not just code fixes

---

## 11. Risk Assessment

### Migration Risks

| Risk | Impact | Probability | Mitigation |
|------|--------|-------------|------------|
| Architectural incompatibility discovered mid-migration | High | Low | TS server as executable spec; 5 upfront traits |
| AI generates poor Rust code | Medium | Medium | Iterative review; CI with clippy + miri |
| Feature freeze during migration | Medium | High | TS server remains operational; migrate incrementally |
| Client-server protocol mismatch | High | Low | Shared msgpack format; integration tests from day 1 |
| Losing team velocity on TS features | Medium | Medium | Parallel work: TS client features + Rust server |

### "Double Rework" Risk per TODO

| TODO | Risk of Rework | Why |
|------|---------------|-----|
| TODO-033 AsyncStorage | None | Pure decorator, interface stable |
| TODO-025 DAG Executor | Low (with TS prototype) | TS prototype validates design |
| TODO-043 S3 Bottomless | None | Behind ServerStorage trait |
| TODO-040 Tiered Storage | Low (with async MapProvider trait) | Internal change only |
| TODO-042 DBSP | Eliminated | Not needed for TopGun's model |
| TODO-041 Multi-tenancy | Low (with RequestContext.tenant_id) | Pervasive but mechanical |

### Combinatorial Risk (Multiple TODOs Intersecting)

**Scenario**: DAG Executor + Tiered Storage + S3 Bottomless all active.

DAG ScanProcessor reads data -> data may be in cold tier -> async promotion needed.

**Mitigation**: MapProvider as async trait handles this transparently. ScanProcessor calls `get_or_load_map()` which may await promotion. No special DAG logic needed.

---

## 12. Key Decisions and Rationale

### Decision 1: Rewrite, Not Port

**Rationale**: Rust idioms (ownership, lifetimes, trait objects) are fundamentally different from TypeScript patterns (closures, late binding, prototypes). Line-by-line porting produces non-idiomatic Rust. Rewriting with TS as spec produces better architecture.

### Decision 2: Finish Wave 0-2 in TypeScript First

**Rationale**: Wave 0-2 stabilizes protocols (IConnection, partition pruning, SSE, cluster HTTP). These protocol decisions are language-agnostic. Cheaper to iterate in TS, then implement final version in Rust.

### Decision 3: DAG Executor Gets TS Prototype

**Rationale**: Only architecturally risky TODO. New subsystem with new abstractions (Processor, routing policies). TS prototype at ~1/3 the cost validates design before committing to Rust.

### Decision 4: S3, AsyncStorage, Tiered Storage Go Straight to Rust

**Rationale**: These are implementations behind stable interfaces (ServerStorage trait, MapProvider trait). TS prototype adds no value — they're engineering problems, not design problems.

### Decision 5: Eliminate DBSP (TODO-042)

**Rationale**: Originated from Turso analysis, not Hazelcast. Solves a problem TopGun doesn't have — existing StandingQueryRegistry and ReverseQueryIndex already provide incremental query notification for supported query types (filter + sort + limit). Full DBSP is a 6-month compiler project with high risk and low ROI for this product.

### Decision 6: TypeScript Server Remains as Reference

**Rationale**: TS server becomes test oracle. Run same integration tests against both servers to verify behavioral equivalence. TS server also useful for client-side development (faster iteration cycle).

---

## 13. Eliminated TODOs

### TODO-042: DBSP Incremental Views — REMOVED FROM ROADMAP

**Origin**: Turso's `/core/incremental/` module (TURSO_INSIGHTS.md, Section 4).

**Why not needed**:
1. TopGun already has incremental query notification:
   - `StandingQueryRegistry`: O(1) affected query detection for IndexedLWWMap
   - `ReverseQueryIndex`: Field-based candidate filtering for regular maps
2. DBSP adds value only for complex aggregations (GROUP BY, SUM, AVG, JOIN)
3. TopGun explicitly doesn't support aggregations (out of scope)
4. If aggregations needed in future, DAG Executor (TODO-025) handles them via streaming processors
5. The TODO itself warned: "Risk of 6-month compiler project"

**Alternative**: If finer-grained reactivity needed, "React Signals" style approach (simple, 1-2 weeks) vs full DBSP compiler (6+ months).

---

## Appendix A: Updated Roadmap Summary

### Pre-Migration (TypeScript)

| # | TODO | Wave | Effort | Purpose |
|---|------|------|--------|---------|
| 1 | TODO-050 IConnection | 0 | 4-6 hours | Transport abstraction |
| 2 | TODO-029 Partition Pruning | 1 | 1 week | Query optimization |
| 3 | TODO-023 Client Cluster | 1 | ~16 hours | Client smart routing |
| 4 | TODO-048 SSE Push | 2 | 2-3 weeks | Serverless push |
| 5 | TODO-049 Cluster HTTP | 2 | 2-3 weeks | Cluster HTTP routing |
| 6 | TODO-025 DAG Executor | 3 | 4-6 weeks | Distributed queries (TS prototype) |

### Post-Migration (Rust)

| # | TODO | Effort | Notes |
|---|------|--------|-------|
| 7 | TODO-033 AsyncStorage | 2-3 weeks | Implement directly in Rust |
| 8 | TODO-043 S3 Bottomless | 6-8 weeks | Implement directly in Rust |
| 9 | TODO-040 Tiered Storage | 4-6 weeks | Implement directly in Rust |
| 10 | TODO-039 Vector Search | 4 weeks | Implement directly in Rust (usearch) |
| 11 | TODO-036 Extensions | 2-3 weeks | Implement directly in Rust |

### Eliminated

| TODO | Reason |
|------|--------|
| TODO-042 DBSP | Not needed — existing incremental notification is sufficient |
| TODO-034 Rust/WASM Hot Paths | Superseded by full Rust migration |

---

## Appendix B: References

### Internal Documents

| Document | Relevance |
|----------|-----------|
| `RUST_WASM_ANALYSIS.md` | Hot-path WASM strategy (superseded by full migration) |
| `HAZELCAST_ARCHITECTURE_COMPARISON.md` | Architecture gap analysis |
| `HAZELCAST_DAG_EXECUTOR_SPEC.md` | DAG Executor specification |
| `topgun-rocksdb.md` | Storage architecture patterns |
| `TURSO_INSIGHTS.md` | Turso ideas (DBSP originated here) |

### External References

- Hazelcast source: `/Users/koristuvac/Projects/hazelcast/` (Java reference implementation)
- tokio ecosystem: https://tokio.rs
- axum web framework: https://docs.rs/axum
- tantivy search: https://github.com/quickwit-oss/tantivy
- sqlx database: https://github.com/launchbadge/sqlx

---

*This document captures the complete migration research as of 2026-02-10. Use as foundation for future migration planning.*
