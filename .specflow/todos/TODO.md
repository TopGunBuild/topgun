# TopGun Roadmap

**Last updated:** 2026-03-20 — Added TODO-136–142 (v2.0 gaps), Milestone 4 GTM, language-adapted strategy
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
- **Slice 2:** ~~SPEC-128 (Write-path wiring)~~ — **done**
- **Slice 3:** ~~SPEC-129 (TS schema DSL & codegen)~~ — **done**
- **Slice 4:** ~~SPEC-130 (Schema → Arrow type derivation)~~ — **done**

### TODO-070: Partial Replication / Shapes
- **Priority:** P1 (table stakes for competitive parity)
- **Complexity:** Medium-Large
- **Summary:** Client subscribes to data subsets; server syncs only matching entries. SyncShape struct with filter + field projection + limit. MerkleTree per shape for efficient delta sync.
- **Context:** [PRODUCT_POSITIONING_RESEARCH.md](../reference/PRODUCT_POSITIONING_RESEARCH.md) Section 7.5
- **Depends on:** TODO-069
- **Effort:** 2-3 weeks

### TODO-091: DataFusion SQL Integration *(converted to SPEC-135)*
- **Priority:** P1 (distributed SQL — Hazelcast-level queries)
- **Complexity:** Large
- **Summary:** Apache DataFusion as SQL query engine. `DataFusionBackend` implements `QueryBackend` trait (feature-gated). `TopGunTableProvider` wraps RecordStore. Arrow cache layer (lazy MsgPack → Arrow, invalidated on mutation). Distributed execution via partition owners + shuffle edges (Arroyo pattern). Partial→Final aggregation.
- **Ref:** Arroyo (`arroyo-planner/builder.rs`), ArkFlow (SessionContext, MsgPack→Arrow codec)
- **Depends on:** TODO-069 (Schema provides Arrow column types for TopGunTableProvider)
- **Effort:** 2-3 weeks
- **Status:** ~~SPEC-135~~ — needs `/sf:split` (large, 7 groups across 5 waves)

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
- **Depends on:** TODO-025 (DAG integration only; connector traits are independent)
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

### TODO-136: Rate Limiting & Quotas
- **Priority:** P1 (required for cloud free tier)
- **Complexity:** Medium
- **Summary:** Per-connection and per-tenant rate limiting via Tower middleware. Configurable limits: ops/sec, concurrent connections, storage bytes. Needed to prevent abuse on free tier and enforce pricing tiers.
- **Crates:** governor (token-bucket), tower::limit
- **Effort:** 1-2 weeks

### TODO-137: Prometheus/OpenTelemetry Metrics Export
- **Priority:** P2 (cloud observability)
- **Complexity:** Medium
- **Summary:** Export server metrics (ops/sec, latency histograms, connection count, partition status, storage usage) via Prometheus endpoint (`GET /metrics`) and optional OTel gRPC. Complements TODO-099 (tracing).
- **Crates:** metrics, metrics-exporter-prometheus, opentelemetry-otlp
- **Effort:** 1 week

### TODO-138: Schema Migrations
- **Priority:** P2 (production readiness)
- **Complexity:** Medium
- **Summary:** Versioned schema evolution: add/remove fields, change types with coercion rules. Migration defined in `topgun.schema.ts`, applied via CLI or API. Backward-compatible reads during migration window.
- **Depends on:** TODO-069
- **Effort:** 2 weeks

### TODO-139: Backup/Restore API
- **Priority:** P2 (cloud self-serve)
- **Complexity:** Medium
- **Summary:** `POST /admin/backup` triggers consistent snapshot (pause writes, snapshot RecordStore + OpLog + MerkleTree, resume). `POST /admin/restore` from snapshot. JSON + MsgPack format. Needed for cloud self-serve and enterprise compliance.
- **Effort:** 1-2 weeks

### TODO-140: Webhooks
- **Priority:** P3 (integration)
- **Complexity:** Low-Medium
- **Summary:** User-configurable HTTP callbacks on data events (insert, update, delete per map). Webhook registry, retry with exponential backoff, delivery log. Enables Zapier/n8n/Make integration.
- **Effort:** 1 week

### TODO-141: Cloud Deployment Configs
- **Priority:** P2 (GTM prerequisite)
- **Complexity:** Low
- **Summary:** Production-ready Docker Compose (single-node), Docker Compose (3-node cluster), Dockerfile (multi-stage, minimal image). Hetzner Cloud deployment guide. Health check endpoints.
- **Effort:** 3-5 days

### TODO-142: Multi-Language SDKs (Python, Go)
- **Priority:** P3 (market expansion)
- **Complexity:** Large
- **Summary:** Python and Go client SDKs. MsgPack wire protocol makes this feasible (language-agnostic). Start with Python (larger market). Core: connect, authenticate, CRUD, subscribe, offline queue.
- **Effort:** 3-4 weeks per SDK

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

## Milestone 4: Go-to-Market (GTM)

*Goal: Transform open-source project into revenue-generating cloud service. Solo-founder, non-native EN speaker — async-first, text-heavy strategy.*
*Context: [BUSINESS_STRATEGY.md](../reference/BUSINESS_STRATEGY.md)*

### TODO-150: Company Registration
- **Priority:** P1 (blocker for payments)
- **Complexity:** Low
- **Summary:** Register legal entity. Options: Stripe Atlas (Delaware LLC, $500) or Estonia e-Residency OÜ. Needed before accepting any payment. Trigger: 2-4 weeks before cloud launch.
- **Effort:** 1 hour setup + 2-4 weeks processing

### TODO-151: Payment Integration (Paddle)
- **Priority:** P1 (blocker for revenue)
- **Complexity:** Low-Medium
- **Summary:** Integrate Paddle as Merchant of Record. Handles VAT/sales tax globally. Subscription tiers: Free / Starter ($99) / Pro ($299) / Enterprise ($999). Paddle approval takes 1-2 weeks — start early.
- **Depends on:** TODO-150
- **Effort:** 3-5 days

### TODO-152: TopGun Cloud — Managed Hosting
- **Priority:** P1 (primary revenue stream)
- **Complexity:** Large
- **Summary:** Multi-tenant managed service on Hetzner. Provisioning API (create/delete/scale instances), tenant isolation, usage metering, Paddle billing webhooks. Base infra: 2x Hetzner CCX13 ($30-60/mo).
- **Depends on:** TODO-136 (rate limits), TODO-141 (Docker), TODO-150, TODO-151
- **Effort:** 3-4 weeks

### TODO-153: Landing Page & Waitlist
- **Priority:** P2 (pre-launch)
- **Complexity:** Low
- **Summary:** topgun.build landing page with clear value prop, feature comparison, pricing preview, email waitlist. Static site (Astro/Next) on Vercel.
- **Effort:** 2-3 days

### TODO-154: Documentation Site
- **Priority:** P1 (adoption prerequisite)
- **Complexity:** Medium
- **Summary:** Comprehensive docs on topgun.build: getting started, API reference, guides (offline-first, real-time sync, SQL queries), deployment. Content written RU → LLM → EN.
- **Effort:** 2 weeks (ongoing)

### TODO-155: Show HN Launch
- **Priority:** P1 (primary launch event)
- **Complexity:** Low
- **Summary:** "Show HN: TopGun — Open-source real-time data platform with offline-first CRDTs (Rust)". Prerequisites: working demo, clean README, quick start, Discord, docs, 1-2 blog posts. Prepare FAQ answers via LLM in advance.
- **Depends on:** TODO-153, TODO-154, TODO-156
- **Effort:** 1 day (prep: 1 week)

### TODO-156: Community Setup
- **Priority:** P2 (retention)
- **Complexity:** Low
- **Summary:** Discord server (text-only, no voice channels — language constraint). Telegram channel (RU) for Russian-speaking early adopters. GitHub Discussions enabled. Template responses + FAQ bot for common questions.
- **Effort:** 2-3 hours setup

### TODO-157: Content Marketing Pipeline
- **Priority:** P2 (long-term growth)
- **Complexity:** Medium (ongoing)
- **Summary:** 2 blog posts/month. Process: draft RU → LLM → EN → publish. Topics: "How we built X" (Rust perf), comparison posts (vs Firebase/Supabase), CRDT explainers, benchmark posts. Cross-post to dev.to. Twitter/X build-in-public (short posts + screenshots).
- **Effort:** 4-8 hours/post (ongoing)

### TODO-158: Premium License / Open-Core Tier
- **Priority:** P3 (secondary revenue)
- **Complexity:** Low-Medium
- **Summary:** Dual-license: Apache 2.0 core + commercial license for premium features. Premium candidates: advanced monitoring, priority support, custom connectors, SSO/SAML. Complement to cloud revenue.
- **Effort:** 1 week (license + feature gates)

---

## Execution Order

### Milestone 2 — v2.0 (Data Platform)

**Completed waves:** 6a (SPEC-126 Tantivy), 6a¹ (SPEC-131 search fix), 6b (SPEC-127 schema types), 6b² (SPEC-128 write-path, SPEC-129 TS codegen, SPEC-130 Arrow derivation).

| Wave | Items | Blocked by | Rationale |
|------|-------|------------|-----------|
| **6c** | TODO-091 (DataFusion SQL) · TODO-070 (Shapes) · TODO-033 (Write-Behind) | 130 ✓ · 128 ✓ · — | SQL needs Arrow schemas (done); Shapes needs write-path wiring (done); Write-Behind independent |
| **6d** | TODO-025 (DAG Executor) · TODO-092 (Connector traits) | 091 · — | DAG needs SQL for pipeline definitions; Connector traits independent, DAG integration after |
| **6e** | TODO-072 (WASM) · TODO-036 (Extensions) | 091 · soft: 025+091+092 | WASM compiles SQL to browser; Extensions benefits from knowing all extension points first |
| **6f** | TODO-048 (SSE) · TODO-049 (Cluster HTTP) · TODO-076 (Hash opt) · TODO-102 (Rust CLI) | — | Independent network/tooling, low priority (P3), no blockers |
| **6f²** | TODO-136 (Rate Limits) · TODO-137 (Metrics) · TODO-138 (Schema Migrations) | 069 ✓ | Cloud prerequisites; Rate limits needed for free tier |
| **6f³** | TODO-139 (Backup/Restore) · TODO-140 (Webhooks) · TODO-141 (Docker) | — | Cloud-readiness; Docker needed for deployment |
| **6g** | TODO-101 (DevTools) · TODO-093 v2.0 (Dashboard) | — · 025+091+092 | UI layer last: needs features to visualize |
| **6h** | TODO-142 (Python SDK) | — | Market expansion; after core stabilizes |

### Milestone 3 — v3.0+ (Enterprise)

| Wave | Items | Blocked by |
|------|-------|------------|
| **7a** | TODO-041 (Multi-Tenancy) · TODO-043 (S3 Bottomless) | — · 033 |
| **7b** | TODO-040 (Tiered Storage) · TODO-039 (Vector Search) | 043 · — |
| **7c** | TODO-044 (Bi-Temporal) | 043 |
| **7d** | TODO-095 (Enterprise dir) · TODO-093 v3.0 (Dashboard) | — · 041+040 |

### Milestone 4 — GTM (Go-to-Market)

*Runs in parallel with late v2.0 waves (6f²+). Business tasks don't block technical work.*

| Wave | Items | Blocked by | Timing |
|------|-------|------------|--------|
| **8a** (pre-launch) | TODO-156 (Community) · TODO-153 (Landing page) · TODO-157 (Content) | — | Start during wave 6d-6e |
| **8b** (pre-launch) | TODO-150 (Company reg) · TODO-154 (Docs) | — | Start 4-6 weeks before cloud launch |
| **8c** (launch) | TODO-151 (Paddle) · TODO-152 (Cloud) | 150 · 136+141 | After v2.0 feature-complete |
| **8d** (launch) | TODO-155 (Show HN) | 153+154+156 | 1-2 weeks after cloud beta |
| **8e** (post-launch) | TODO-158 (Premium license) | Revenue validation | After first paying customers |

## Dependency Graph

```
MILESTONE 2: Data Platform (v2.0)

  ✓ SPEC-126 (Tantivy optimization)
  ✓ SPEC-131 (Search partition fix)
  ✓ SPEC-127 → SPEC-128 (Write-path) → TODO-070 (Shapes)
  ✓ SPEC-127 → SPEC-129 (TS codegen)
  ✓ SPEC-127 → SPEC-130 (Arrow)     → TODO-091 (DataFusion SQL) → TODO-025 (DAG Stream Processing)
                                              │                            │
                                              └→ TODO-072 (WASM)          └→ TODO-092 (DAG integration)

  TODO-092 (Connector traits) ← independent of DAG, can start in 6d
  TODO-033 (Write-Behind) ← independent, unblocks v3.0 S3
  ✓ TODO-027 (DST) ← completed via SPEC-132a-d (madsim, SimCluster, fault injection, proptest)
  TODO-036 (Extensions) ← soft dep on 025+091+092 (needs extension points)
  TODO-048 (SSE) · TODO-049 (Cluster HTTP) · TODO-076 (Hash opt) · TODO-102 (Rust CLI) ← P3, no blockers
  TODO-101 (Client DevTools) · TODO-093 v2.0 (Dashboard) ← depends on 025+091+092

  Cloud prerequisites (wave 6f²-6f³):
  TODO-136 (Rate Limits) ← Tower middleware, independent
  TODO-137 (Metrics) ← complements TODO-099 (tracing)
  TODO-138 (Schema Migrations) ← depends on TODO-069 ✓
  TODO-139 (Backup/Restore) · TODO-140 (Webhooks) · TODO-141 (Docker) ← independent
  TODO-142 (Python SDK) ← after core API stabilizes

MILESTONE 3: Enterprise (v3.0+)

  TODO-041 (Multi-Tenancy)
  TODO-043 (S3 Bottomless) ──→ TODO-040 (Tiered) ──→ TODO-044 (Time-Travel)
       ↑
  TODO-033 (Write-Behind, from v2.0)
  TODO-039 (Vector Search)
  TODO-095 (Enterprise dir)
  TODO-093 v3.0 (Dashboard) ← depends on 041+040

MILESTONE 4: GTM (parallel with late v2.0)

  TODO-156 (Community) ──┐
  TODO-153 (Landing)  ───┤
  TODO-157 (Content)  ───┼→ TODO-155 (Show HN)
  TODO-154 (Docs)     ───┘
  TODO-150 (Company) → TODO-151 (Paddle) → TODO-152 (Cloud) ← also needs 136+141
  TODO-158 (Premium license) ← after revenue validation
```

---

## Eliminated Items

| TODO | Reason | Date |
|------|--------|------|
| TODO-042 (DBSP) | Not needed; StandingQueryRegistry + ReverseQueryIndex sufficient | 2026-02-10 |
| TODO-034 (Rust/WASM hot paths) | Superseded by full Rust migration | 2026-02-10 |
| TODO-045 (DST Documentation) | TS testing infrastructure documentation for deprecated system | 2026-02-18 |
| TODO-117 (1M ops/sec) | 5 proposed optimizations invalid: per-partition HLC breaks LWW causality, batch drain breaks per-key ordering, batch Merkle/broadcast not bottlenecks (<1% CPU), dynamic workers already implemented. Replaced by TODO-118 (flamegraph profiling) | 2026-03-18 |
| TODO-027 (DST) | Completed via SPEC-132a-d: madsim integration, SimCluster harness, fault injection, 11 sim tests, proptest property-based testing | 2026-03-20 |

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
| SPEC-126 | [FLAMEGRAPH_ANALYSIS.md](../../packages/server-rust/docs/profiling/FLAMEGRAPH_ANALYSIS.md); Quickwit `quickwit-indexing/src/actors/indexer.rs`; SurrealDB `core/src/kvs/index.rs`; Databend `inverted_index_writer.rs` |
| TODO-150–158 | [BUSINESS_STRATEGY.md](../reference/BUSINESS_STRATEGY.md) |

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
