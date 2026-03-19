# TopGun Roadmap

**Last updated:** 2026-03-19 — SPEC-127 completed (schema types/validation/SchemaService); TODO-069 slice 1 done, 3 remaining
**Strategy:** Rust-first IMDG design informed by Hazelcast architecture
**Product vision:** "The unified real-time data platform — from browser to cluster to cloud storage"

---

## v1.0 — RELEASED

v1.0 complete. 84 specs archived (SPEC-038–084, 114–122). 540+ Rust tests, 55 integration tests, clippy-clean. Legacy TS server removed. Post-release performance work: PartitionDispatcher (116), async tantivy (117), OP_BATCH splitting (118), scatter-gather Merkle (119), bounded channels (120), Rust load harness (121a-c), WebSocket pipelining (122). Result: 100 → 200,000 ops/sec (2000x). Full v1.0 history in git: `git show b0ab167^:.specflow/todos/TODO.md`.

---

## Milestone 2: Data Platform (v2.0)

*Goal: SQL queries, stream processing, schema validation, connectors — competitive with Hazelcast feature set.*

### TODO-069: Schema System *(split into 4 slices)*
- **Priority:** P1 (product differentiator)
- **Complexity:** Medium
- **Summary:** TypeScript-first schema definition with server-side validation. Developer writes `topgun.schema.ts`, build step generates Rust validation code + TS client types. Phased rollout: optional → strict.
- **Context:** [PRODUCT_POSITIONING_RESEARCH.md](../reference/PRODUCT_POSITIONING_RESEARCH.md) Section 7.4
- **Effort:** 2-3 weeks
- **Slice 1:** ~~SPEC-127 (Schema types, validation engine, SchemaService)~~ — **done**
- **Slice 2:** TODO-128 (Write-path wiring)
- **Slice 3:** TODO-129 (TS schema DSL & codegen)
- **Slice 4:** TODO-130 (Schema → Arrow type derivation)

### TODO-128: Schema Write-Path Wiring
- **Priority:** P1 (required for schema enforcement)
- **Complexity:** Small-Medium
- **Summary:** Wire SchemaService into CrdtService write path. `CrdtService::handle_put` calls `SchemaProvider::validate()` before CRDT merge. Invalid data returns error to client (no merge, no broadcast). Optional mode: no schema registered = no validation (passthrough).
- **Context:** SPEC-127 defines SchemaService + validate_value. CrdtService already has WriteValidator for permission/size checks — schema validation slots in alongside it.
- **Depends on:** SPEC-127
- **Effort:** 2-3 days

### TODO-129: TS Schema DSL & Codegen Toolchain
- **Priority:** P1 (developer-facing schema definition)
- **Complexity:** Medium
- **Summary:** Developer defines schemas in `topgun.schema.ts` using a TypeScript DSL (builder or decorator pattern). Build step (`topgun codegen`) generates: (1) Rust `MapSchema` registration code (serialized schemas), (2) TypeScript client types with full autocompletion. DSL must express all FieldType variants and FieldConstraint options from SPEC-127.
- **Context:** SPEC-127 defines FieldType/FieldConstraint/MapSchema types. Codegen outputs must match these Rust types exactly. Consider: ts-morph, JSON intermediate format, or direct serde serialization.
- **Depends on:** SPEC-127
- **Effort:** 1-2 weeks

### TODO-130: Schema → Arrow Type Derivation
- **Priority:** P1 (bridge to DataFusion SQL)
- **Complexity:** Small
- **Summary:** Derive Apache Arrow `Schema` from `MapSchema`. Maps `FieldType` → Arrow `DataType` (String→Utf8, Int→Int64, Float→Float64, Bool→Boolean, Binary→Binary, Timestamp→TimestampMillisecond, Array→List, Map→Struct). Used by `TopGunTableProvider` in TODO-091 to register tables with DataFusion. Implement as `impl MapSchema { pub fn to_arrow_schema(&self) -> arrow::datatypes::Schema }`.
- **Context:** SPEC-127 defines FieldType enum. TODO-091 (DataFusion SQL) needs Arrow schemas from TopGunTableProvider. This is the bridge.
- **Depends on:** SPEC-127
- **Effort:** 1-2 days

### TODO-070: Partial Replication / Shapes
- **Priority:** P1 (table stakes for competitive parity)
- **Complexity:** Medium-Large
- **Summary:** Client subscribes to data subsets; server syncs only matching entries. SyncShape struct with filter + field projection + limit. MerkleTree per shape for efficient delta sync.
- **Context:** [PRODUCT_POSITIONING_RESEARCH.md](../reference/PRODUCT_POSITIONING_RESEARCH.md) Section 7.5
- **Depends on:** TODO-069
- **Effort:** 2-3 weeks

### TODO-091: DataFusion SQL Integration
- **Priority:** P1 (distributed SQL — Hazelcast-level queries)
- **Complexity:** Large
- **Summary:** Apache DataFusion as SQL query engine. `DataFusionBackend` implements `QueryBackend` trait (feature-gated). `TopGunTableProvider` wraps RecordStore. Arrow cache layer (lazy MsgPack → Arrow, invalidated on mutation). Distributed execution via partition owners + shuffle edges (Arroyo pattern). Partial→Final aggregation.
- **Ref:** Arroyo (`arroyo-planner/builder.rs`), ArkFlow (SessionContext, MsgPack→Arrow codec)
- **Depends on:** TODO-069 (Schema provides Arrow column types for TopGunTableProvider)
- **Effort:** 2-3 weeks

### TODO-025: DAG Executor for Stream Processing
- **Priority:** P1 (Hazelcast Jet equivalent)
- **Complexity:** Large
- **Summary:** Distributed stream processing DAG. SQL-defined pipelines compiled to operator graphs with windowed aggregation, stateful processing, and checkpointing. petgraph DiGraph, operator chaining, barrier-based checkpointing, shuffle edges.
- **Context:** [HAZELCAST_DAG_EXECUTOR_SPEC.md](../reference/HAZELCAST_DAG_EXECUTOR_SPEC.md)
- **HC Reference:** `hazelcast/jet/core/`, `jet/impl/execution/`
- **Rust Reference:** Arroyo (petgraph DAG, barrier checkpoints, shuffle edges)
- **Depends on:** TODO-091
- **Effort:** 3-4 weeks

### TODO-092: Connector Framework
- **Priority:** P2 (extensible data ingestion/egress)
- **Complexity:** Medium
- **Summary:** Trait-based connector system: `ConnectorSource`, `ConnectorSink`, `Codec` traits. Connector registry. Initial connectors: Kafka source/sink, S3 sink, PostgreSQL CDC source.
- **Ref:** Arroyo connector traits, ArkFlow Input/Output/Codec, RisingWave (`/Users/koristuvac/Projects/rust/risingwave/src/connector/`)
- **Depends on:** TODO-025
- **Effort:** 2 weeks

### TODO-033: AsyncStorageWrapper (Write-Behind)
- **Priority:** P2
- **Complexity:** Medium
- **Summary:** Hazelcast-style Write-Behind: staging area, write coalescing, batch flush, retry queue.
- **Context:** [topgun-rocksdb.md](../reference/topgun-rocksdb.md)
- **HC Reference:** `hazelcast/map/impl/mapstore/` — WriteBehindStore, StoreWorker
- **Effort:** 2-3 weeks

### TODO-036: Pluggable Extension System
- **Priority:** P2
- **Complexity:** Medium
- **Summary:** Modular extension system for community contributions (crypto, compression, audit, geo).
- **Context:** [TURSO_INSIGHTS.md](../reference/TURSO_INSIGHTS.md) Section 5
- **Effort:** 2-3 weeks

### TODO-072: Selective WASM Modules for Client
- **Priority:** P2
- **Complexity:** Medium
- **Summary:** Compile DataFusion SQL + tantivy search to WASM for browser. Same SQL dialect offline and online. NOT for basic CRDT ops (sync JS is faster due to WASM boundary cost).
- **Context:** [PRODUCT_POSITIONING_RESEARCH.md](../reference/PRODUCT_POSITIONING_RESEARCH.md) Section 7.6
- **Depends on:** TODO-091
- **Effort:** 2-3 weeks

### TODO-048: SSE Push for HTTP Sync
- **Priority:** P3
- **Complexity:** Low
- **Summary:** Server-Sent Events transport for real-time push in serverless. `GET /events` SSE endpoint via axum.
- **Effort:** 2-3 days

### TODO-049: Cluster-Aware HTTP Routing
- **Priority:** P3
- **Complexity:** Medium
- **Summary:** HttpSyncHandler routes to partition owners in cluster.
- **Effort:** 1-2 weeks

### TODO-076: Evaluate MsgPack-Based Merkle Hashing
- **Priority:** P3 (deferred optimization)
- **Complexity:** Medium
- **Summary:** Hash MsgPack bytes directly instead of JSON string → FNV-1a.
- **Effort:** 1 day (evaluation) + 2-3 days (implementation)

### TODO-101: Client DevTools
- **Priority:** P2 (DX differentiator)
- **Complexity:** Medium-Large
- **Summary:** Browser DevTools panel or in-app debug overlay: local replica state, pending sync queue, HLC timeline, connection status, CRDT merge history.
- **Effort:** 4-6 weeks

### TODO-102: Rust CLI (clap)
- **Priority:** P3
- **Complexity:** Medium
- **Summary:** Rewrite CLI from JavaScript (commander) to Rust (clap). Single `topgun` binary: `topgun serve`, `topgun status`, `topgun debug crdt`, `topgun sql`.
- **Effort:** 1-2 weeks

### TODO-093 v2.0: Admin Dashboard — Data Platform Features
- **Priority:** P2
- **Complexity:** Medium
- **Summary:** Pipeline visualization (ReactFlow + Dagre), live metric coloring by backpressure, DataFusion SQL playground, connector wizard, schema browser.
- **Ref:** Arroyo WebUI (`/Users/koristuvac/Projects/rust/arroyo/webui/`)
- **Depends on:** TODO-025, TODO-091, TODO-092
- **Effort:** 2-3 weeks

### TODO-027: Deterministic Simulation Testing (DST)
- **Priority:** P2 (testing infrastructure)
- **Complexity:** Medium
- **Summary:** Deterministic simulation testing via `madsim` crate. Seeded RNG, virtual time, simulated network. Property-based invariant checking: CRDT convergence, Merkle sync, cluster rebalancing.
- **Ref:** RisingWave (`/Users/koristuvac/Projects/rust/risingwave`, `ci-sim` profile); [TURSO_INSIGHTS.md](../reference/TURSO_INSIGHTS.md) Section 2
- **Effort:** ~1 week

---

## Milestone 3: Enterprise (v3.0+)

*Goal: Enterprise-grade features for large-scale deployments.*

### TODO-041: Multi-Tenancy
- **Priority:** P1 (enterprise requirement)
- **Complexity:** Large
- **Summary:** Per-tenant isolation, quotas, billing, tenant-aware partitioning.
- **Context:** [PHASE_5_MULTI_TENANCY_SPEC.md](../reference/PHASE_5_MULTI_TENANCY_SPEC.md)
- **Effort:** 4-6 weeks

### TODO-043: S3 Bottomless Storage
- **Priority:** P2
- **Complexity:** Very Large
- **Summary:** Append-only log in S3/R2/GCS. Immutable log segments, replay on startup, Merkle checkpoints.
- **Context:** [TURSO_INSIGHTS.md](../reference/TURSO_INSIGHTS.md) Section 7
- **Crates:** aws-sdk-s3, opendal
- **Depends on:** TODO-033
- **Effort:** 6-8 weeks

### TODO-040: Tiered Storage (Hot/Cold)
- **Priority:** P2
- **Complexity:** Large
- **Summary:** Hot data in memory, cold data in S3/cheap storage. RecordStore metadata (hit_count, last_access_time) enables eviction policies.
- **Context:** [topgun-rocksdb.md](../reference/topgun-rocksdb.md)
- **Depends on:** TODO-033, TODO-043
- **Effort:** 4-6 weeks

### TODO-039: Vector Search
- **Priority:** P3
- **Complexity:** Large
- **Summary:** Semantic vector search with local embeddings, HNSW index, tri-hybrid search (Exact + BM25 + Semantic).
- **Context:** [PHASE_15_VECTOR_SEARCH_SPEC.md](../reference/PHASE_15_VECTOR_SEARCH_SPEC.md)
- **Crate:** usearch
- **Effort:** 4 weeks

### TODO-044: Bi-Temporal Queries (Time-Travel)
- **Priority:** P3
- **Complexity:** Large
- **Summary:** Query historical state with valid time + transaction time.
- **Context:** [TURSO_INSIGHTS.md](../reference/TURSO_INSIGHTS.md) Section 8
- **Depends on:** TODO-043
- **Effort:** 4-6 weeks

### TODO-095: Enterprise Directory Structure
- **Priority:** P3
- **Complexity:** Low
- **Summary:** Create `enterprise/` directory with BSL 1.1 LICENSE. Move v3.0 crates there when implemented. Feature-gated: `[features] enterprise = [...]`.
- **Depends on:** First enterprise crate
- **Effort:** 1 day

### TODO-093 v3.0: Admin Dashboard — Enterprise Features
- **Priority:** P3
- **Complexity:** Medium
- **Summary:** Multi-tenant admin, per-tenant quotas/usage, tiered storage monitor (hot/warm/cold visualization), vector search admin.
- **Depends on:** TODO-041, TODO-040
- **Effort:** 1-2 weeks

---

## Execution Order

### Milestone 2 — v2.0 (Data Platform)

| Wave | Items | Blocked by | Rationale |
|------|-------|------------|-----------|
| **6a** | SPEC-126 (Tantivy optimization) · TODO-027 (DST) | — | Tantivy eats 60-80% CPU; DST = test infra for complex features |
| **6b** | SPEC-127 (Schema types + validation + SchemaService) | — | Data model foundation: FieldType, FieldConstraint, validate_value, SchemaService |
| **6b²** | TODO-128 (Write-path wiring) · TODO-129 (TS codegen) · TODO-130 (Arrow derivation) | SPEC-127 | All three parallel after types exist: enforcement, DX toolchain, DataFusion bridge |
| **6c** | TODO-091 (DataFusion SQL) · TODO-070 (Shapes) · TODO-033 (Write-Behind) | 130 · 128 · — | SQL needs Arrow schemas (130); Shapes needs validation in write path (128); Write-Behind independent |
| **6d** | TODO-025 (DAG Executor) · TODO-092 (Connectors) | 091 · — (traits) / 025 (DAG integration) | DAG needs SQL for pipeline definitions; Connector traits independent, DAG integration after |
| **6e** | TODO-072 (WASM) · TODO-036 (Extensions) | 091 · — | WASM compiles SQL to browser; Extensions knows all extension points |
| **6f** | TODO-048 (SSE) · TODO-049 (Cluster HTTP) · TODO-076 (Hash opt) · TODO-102 (Rust CLI) | — | Independent network/tooling, no blockers |
| **6g** | TODO-101 (DevTools) · TODO-093 v2.0 (Dashboard) | — · 025+091+092 | UI layer last: needs features to visualize |

### Milestone 3 — v3.0+ (Enterprise)

| Wave | Items | Blocked by |
|------|-------|------------|
| **7a** | TODO-041 (Multi-Tenancy) · TODO-043 (S3 Bottomless) | — · 033 |
| **7b** | TODO-040 (Tiered Storage) · TODO-039 (Vector Search) | 043 · — |
| **7c** | TODO-044 (Bi-Temporal) | 043 |
| **7d** | TODO-095 (Enterprise dir) · TODO-093 v3.0 (Dashboard) | — · 041+040 |

## Dependency Graph

```
MILESTONE 2: Data Platform (v2.0)

  SPEC-126 (Tantivy optimization) ← 60-80% CPU, highest impact
  TODO-027 (DST) ← foundational test infra

  SPEC-127 (Schema types + validation + SchemaService)
       ├──→ TODO-128 (Write-path wiring) ──→ TODO-070 (Shapes)
       ├──→ TODO-129 (TS codegen)
       └──→ TODO-130 (Arrow derivation) ──→ TODO-091 (DataFusion SQL) ──→ TODO-025 (DAG Stream Processing)
                                                    │                              │
                                                    └──→ TODO-072 (WASM)          └──→ TODO-092 (Connectors, DAG integration)

  TODO-092 (Connector traits) ← independent of DAG
  TODO-033 (Write-Behind) ← independent, unblocks v3.0 S3
  TODO-036 (Extensions)
  TODO-048 (SSE) · TODO-049 (Cluster HTTP) · TODO-076 (Hash opt) · TODO-102 (Rust CLI)
  TODO-101 (Client DevTools) · TODO-093 v2.0 (Dashboard) ← depends on 025+091+092

MILESTONE 3: Enterprise (v3.0+)

  TODO-041 (Multi-Tenancy)
  TODO-043 (S3 Bottomless) ──→ TODO-040 (Tiered) ──→ TODO-044 (Time-Travel)
       ↑
  TODO-033 (Write-Behind, from v2.0)
  TODO-039 (Vector Search)
  TODO-095 (Enterprise dir)
  TODO-093 v3.0 (Dashboard) ← depends on 041+040
```

---

## Eliminated Items

| TODO | Reason | Date |
|------|--------|------|
| TODO-042 (DBSP) | Not needed; StandingQueryRegistry + ReverseQueryIndex sufficient | 2026-02-10 |
| TODO-034 (Rust/WASM hot paths) | Superseded by full Rust migration | 2026-02-10 |
| TODO-045 (DST Documentation) | TS testing infrastructure documentation for deprecated system | 2026-02-18 |
| TODO-117 (1M ops/sec) | 5 proposed optimizations invalid: per-partition HLC breaks LWW causality, batch drain breaks per-key ordering, batch Merkle/broadcast not bottlenecks (<1% CPU), dynamic workers already implemented. Replaced by TODO-118 (flamegraph profiling) | 2026-03-18 |

## Context Files

| TODO | Context File |
|------|-------------|
| TODO-025 | [HAZELCAST_DAG_EXECUTOR_SPEC.md](../reference/HAZELCAST_DAG_EXECUTOR_SPEC.md) |
| TODO-033 | [topgun-rocksdb.md](../reference/topgun-rocksdb.md) |
| TODO-036 | [TURSO_INSIGHTS.md](../reference/TURSO_INSIGHTS.md) Section 5 |
| TODO-039 | [PHASE_15_VECTOR_SEARCH_SPEC.md](../reference/PHASE_15_VECTOR_SEARCH_SPEC.md) |
| TODO-040 | [topgun-rocksdb.md](../reference/topgun-rocksdb.md) |
| TODO-041 | [PHASE_5_MULTI_TENANCY_SPEC.md](../reference/PHASE_5_MULTI_TENANCY_SPEC.md) |
| TODO-043 | [TURSO_INSIGHTS.md](../reference/TURSO_INSIGHTS.md) Section 7 |
| TODO-044 | [TURSO_INSIGHTS.md](../reference/TURSO_INSIGHTS.md) Section 8 |
| TODO-091 | Arroyo: `arroyo-planner/builder.rs`, `arroyo-datastream/logical.rs`; ArkFlow: `arkflow-plugin/src/processor/sql.rs` |
| TODO-092 | Arroyo: `arroyo-connector/src/`; ArkFlow: `arkflow-core/src/input/`, `arkflow-core/src/codec/`; RisingWave: `src/connector/` |
| TODO-093 | Existing: `apps/admin-dashboard/`; Arroyo WebUI: `/Users/koristuvac/Projects/rust/arroyo/webui/` |
| TODO-027 | [TURSO_INSIGHTS.md](../reference/TURSO_INSIGHTS.md) Section 2; RisingWave `ci-sim` profile |
| SPEC-126 | [FLAMEGRAPH_ANALYSIS.md](../../packages/server-rust/docs/profiling/FLAMEGRAPH_ANALYSIS.md); Quickwit `quickwit-indexing/src/actors/indexer.rs`; SurrealDB `core/src/kvs/index.rs`; Databend `inverted_index_writer.rs` |

## Reference Implementations

| Source | Purpose |
|--------|---------|
| **Hazelcast** (`/Users/koristuvac/Projects/hazelcast`) | Conceptual architecture (WHAT to build) |
| **Arroyo** (`/Users/koristuvac/Projects/rust/arroyo`) | DAG, DataFusion SQL, Admin UI patterns |
| **ArkFlow** (`/Users/koristuvac/Projects/rust/arkflow`) | DataFusion + MsgPack→Arrow codec |
| **TiKV** (`/Users/koristuvac/Projects/rust/tikv`) | Storage traits, per-partition FSM |
| **Quickwit** (`/Users/koristuvac/Projects/rust/quickwit`) | Chitchat gossip, Tower layers, **tantivy indexing pipeline** (actor-based, time+memory+doccount commit triggers) |
| **Databend** (`/Users/koristuvac/Projects/rust/databend`) | OpenDAL, GlobalServices |
| **RisingWave** (`/Users/koristuvac/Projects/rust/risingwave`) | DST (madsim), connectors |
| **SurrealDB** (`/Users/koristuvac/Projects/rust/surrealdb`) | WS lifecycle, multi-tenancy, **batch indexing** (250-doc threshold, custom FT index) |
