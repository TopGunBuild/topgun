# TopGun Roadmap

**Last updated:** 2026-03-25 — TODO-187 converted to SPEC-152
**Strategy:** Feature-complete open-source core first, then cloud. All differentiators built before launch.
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

### TODO-070: Partial Replication / Shapes *(superseded)*
- **Priority:** P1 (table stakes for competitive parity)
- **Complexity:** Medium-Large
- **Summary:** Client subscribes to data subsets; server syncs only matching entries. SyncShape struct with filter + field projection + limit. MerkleTree per shape for efficient delta sync.
- **Context:** [PRODUCT_POSITIONING_RESEARCH.md](../reference/PRODUCT_POSITIONING_RESEARCH.md) Section 7.5
- **Depends on:** TODO-069
- **Effort:** 2-3 weeks
- **Status:** **Superseded** — Shapes merged into Queries via TODO-181/182/183/184 (SPEC-142/143/144/145). All Shape functionality (field projection, limit, Merkle delta sync) now lives under the Query path. Shape-specific code deleted in SPEC-145.

### TODO-091: DataFusion SQL Integration *(converted to SPEC-135)*
- **Priority:** P1 (distributed SQL — Hazelcast-level queries)
- **Complexity:** Large
- **Summary:** Apache DataFusion as SQL query engine. `DataFusionBackend` implements `QueryBackend` trait (feature-gated). `TopGunTableProvider` wraps RecordStore. Arrow cache layer (lazy MsgPack → Arrow, invalidated on mutation). Distributed execution via partition owners + shuffle edges (Arroyo pattern). Partial→Final aggregation.
- **Ref:** Arroyo (`arroyo-planner/builder.rs`), ArkFlow (SessionContext, MsgPack→Arrow codec)
- **Depends on:** TODO-069 (Schema provides Arrow column types for TopGunTableProvider)
- **Effort:** 2-3 weeks
- **Status:** ~~SPEC-135a~~ ✓ · ~~SPEC-135b~~ ✓ · ~~SPEC-135c~~ ✓ — **done** (DistributedPlanner deferred)

### TODO-167: ~~Server-Side Shape Max Limit~~ → Absorbed into TODO-182 ✓
- **Status:** Done — max_query_records implemented in unified QueryService (SPEC-143)

### TODO-168: ~~ShapeHandle Generic Typing~~ → Absorbed into TODO-183 ✓
- **Status:** Done — unified QueryHandle with fields/merkleRootHash (SPEC-144)

### TODO-183: ~~Unified Query Client — Client-Side Merge~~ → Converted to SPEC-144
- **Status:** Converted to SPEC-144

### TODO-171: RBAC — Role-Based Access Control Implementation
- **Priority:** P2 (documented but not implemented — expectation gap for adopters)
- **Complexity:** Medium
- **Summary:** Docs describe a full RBAC system (role-based policies, map pattern matching, field-level security) that was implemented in the old TS server but not ported to Rust. Current Rust server has only basic map-level `read`/`write` booleans with exact-match lookup. Roles are extracted from JWT into `Principal.roles` but never evaluated for data access permissions.
- **Depends on:** SPEC-137 ✓ (auth hardening)
- **Effort:** 1-2 weeks
- **TS Reference:** Old TS server had working RBAC — check git history (commit `926e856` removed TS server). Key files to recover via `git show`:
  - `packages/server/src/coordinator/auth-handler.ts` — role extraction + permission assignment
  - `packages/server/src/coordinator/` — permission evaluation logic
  - Search for `PermissionPolicy`, `mapNamePattern`, `allowedFields` in old TS code
- **HC Reference:** `hazelcast/security/` — enterprise security model, permission policies
- **Scope:**
  - **Phase A (core RBAC):** Role-to-policy mapping, wildcard/glob pattern matching on map names (`users:*`, `public:*`), policy evaluation in `WriteValidator`. Store policies in `SecurityConfig`, evaluate `Principal.roles` against policies during `validate_write()` and add `validate_read()`.
  - **Phase B (field-level):** `allowedFields` per policy, field filtering on read responses. More invasive — touches CRDT response serialization.
  - **Phase C (dynamic):** Admin API for policy CRUD, hot-reload without restart.
- **Current code touchpoints:**
  - `core-rust/src/types.rs` — `Principal { id, roles }` (exists, roles unused)
  - `server-rust/src/service/security.rs` — `WriteValidator`, `SecurityConfig` (extend)
  - `server-rust/src/network/connection.rs` — `MapPermissions { read, write }` (extend with policy evaluation)
  - `server-rust/src/network/handlers/auth.rs` — role extraction (exists, works)

### TODO-174: Adaptive Indexing — Rust Port
- **Priority:** P3
- **Complexity:** Medium
- **Summary:** Feature described in `adaptive-indexing.mdx` — automatic index suggestions and creation based on query patterns. Implemented in old TS server but not ported to Rust. Currently presented as available in docs but does not exist in `packages/server-rust/src/`. No grep evidence of HashIndex, NavigableIndex, AdaptiveIndex, or IndexRegistry in server-rust.
- **Documented in:** `guides/adaptive-indexing.mdx` — presented as available but not yet ported to Rust
- **TS Reference:** Old TS server had working implementation — recover via git:
  - `git show 926e856^:packages/server/src/` — search for adaptive-index, index-registry, query-pattern files
  - Search: `git show 926e856^:packages/server/src/` for `AdaptiveIndex`, `IndexSuggestion`, `QueryPattern`
- **HC Reference:** `hazelcast/query/` — Hazelcast query indexing patterns
- **Effort:** 2-3 weeks

### TODO-175: Distributed Locks — Rust Port
- **Priority:** P2 (documented as available, commonly needed for coordination)
- **Complexity:** Medium
- **Summary:** Feature described in `distributed-locks.mdx` — distributed locking with fencing tokens. Wire messages (`LockRequest`, `LockRelease`) exist in core-rust and are routed to `CoordinationService`, but `coordination.rs` returns `NotImplemented` for both (confirmed by AC6 test). Feature is a stub.
- **Documented in:** `guides/distributed-locks.mdx` — presented as available but currently returns NotImplemented at runtime
- **TS Reference:** Old TS server had working distributed locks — recover via git:
  - `git show 926e856^:packages/server/src/coordinator/` — look for lock-handler, distributed-lock files
  - Search: `git show 926e856^:packages/server/src/` for `LockRequest`, `fencing_token`, `acquireLock`
- **HC Reference:** `hazelcast/cp/` — CP subsystem, FencedLock, ILock patterns
- **Effort:** 1-2 weeks

### TODO-176: Entry Processor — Rust Port (WASM Sandbox)
- **Priority:** P2 (documented as available, important for atomic read-modify-write)
- **Complexity:** Large
- **Summary:** Feature described in `entry-processor.mdx` — atomic read-modify-write operations executed server-side. Wire messages (`EntryProcess`, `EntryProcessBatch`) exist in core-rust and are routed to `PersistenceService`, but `persistence.rs` comment says "stub — WASM sandbox required" and returns `NotImplemented` for all calls.
- **Documented in:** `guides/entry-processor.mdx` — presented as available but currently returns NotImplemented at runtime
- **TS Reference:** Old TS server had working entry processor with sandbox execution — recover via git:
  - `git show 926e856^:packages/server/src/` — search for entry-processor, sandbox, execute files
  - Search: `git show 926e856^:packages/server/src/` for `EntryProcessor`, `sandbox`, `vm.runInContext`
- **HC Reference:** `hazelcast/map/impl/operation/` — EntryProcessor execution patterns
- **Effort:** 3-4 weeks (requires WASM sandbox or Deno-based execution environment)

### TODO-177: Indexing (Hash/Navigable/Inverted) — Rust Port
- **Priority:** P2 (documented as available, required for O(1) queries on large maps)
- **Complexity:** Large
- **Summary:** Feature described in `indexing.mdx` — HashIndex (equality), NavigableIndex (range queries), InvertedIndex (tokenized text search). None of these index types exist in `packages/server-rust/src/` (confirmed by grep). This is separate from tantivy full-text search (SearchService) which is already implemented.
- **Documented in:** `guides/indexing.mdx` — presented as available but no index types exist in server-rust
- **TS Reference:** Old TS server had working index types — recover via git:
  - `git show 926e856^:packages/server/src/` — search for hash-index, navigable-index, inverted-index files
  - Search: `git show 926e856^:packages/server/src/` for `HashIndex`, `NavigableIndex`, `IndexRegistry`
- **HC Reference:** `hazelcast/query/impl/` — CompositeIndex, QueryContext, IndexRegistry
- **Effort:** 3-4 weeks

### TODO-178: Interceptors / User-Extensible Middleware — Rust Port
- **Priority:** P3
- **Complexity:** Medium
- **Summary:** Feature described in `interceptors.mdx` — user-facing middleware/interceptor API for validation, enrichment, ML inference, and external service integration. No `Interceptor` or user-extensible API exists in `packages/server-rust/src/` (confirmed by grep). The Tower middleware pipeline is internal only.
- **Documented in:** `guides/interceptors.mdx` — presented as available but no user-facing interceptor API exists
- **TS Reference:** Old TS server had working interceptors — recover via git:
  - `git show 926e856^:packages/server/src/` — search for interceptor files
  - Search: `git show 926e856^:packages/server/src/` for `Interceptor`, `middleware`, `before_write`
- **HC Reference:** `hazelcast/map/impl/MapInterceptor`, `hazelcast/map/interceptor/` — interceptor patterns
- **Effort:** 1-2 weeks

### TODO-179: Conflict Resolvers — Rust Port (WASM Sandbox)
- **Priority:** P2 (documented as available, needed for business-rule conflict resolution)
- **Complexity:** Large
- **Summary:** Feature described in `conflict-resolvers.mdx` — custom server-side JavaScript conflict resolution functions. `ConflictResolver` struct and `RegisterResolver`/`UnregisterResolver`/`ListResolvers` messages exist in core-rust. `PersistenceService` routes these ops, but `handle_register_resolver()` returns `NotImplemented`. Comment: "stub — WASM sandbox required". Feature is functionally unavailable at runtime.
- **Documented in:** `guides/conflict-resolvers.mdx` — presented as available but returns NotImplemented. Code examples will compile but fail at runtime.
- **TS Reference:** Old TS server had working conflict resolvers — recover via git:
  - `git show 926e856^:packages/server/src/` — search for conflict-resolver, resolver-registry, sandbox files
  - Search: `git show 926e856^:packages/server/src/` for `ConflictResolver`, `registerResolver`, `sandbox`
- **Effort:** 3-4 weeks (requires WASM sandbox, same infrastructure as TODO-176)
- **Note:** TODO-176 (Entry Processor) and TODO-179 (Conflict Resolvers) both require WASM sandbox — implement together as a shared infrastructure project

### TODO-180: Write Concern — Server Achievement Reporting
- **Priority:** P2 (documented as reporting achieved_level, but server always returns None)
- **Complexity:** Small-Medium
- **Summary:** `write-concern.mdx` documents that `OpAck` responses include `achieved_level` (the durability level actually achieved). In practice, `crdt.rs` lines 197 and 267 always set `achieved_level: None`. The `WriteConcern` wire protocol exists (FIRE_AND_FORGET, MEMORY, APPLIED, REPLICATED, PERSISTED) but the server never reports back what was achieved. Also: `setWithAck()` and `batchSet()` methods shown in docs do not exist in TS client.
- **Documented in:** `guides/write-concern.mdx` — `achieved_level` claim is false, `setWithAck()` does not exist
- **Scope:**
  - Server: populate `achieved_level` in `OpAckPayload` based on what was actually done (APPLIED after CRDT merge, PERSISTED after PostgreSQL write)
  - TS client: add `setWithAck(key, value, options)` method that returns `WriteResult` with `achievedLevel` and `latencyMs`
  - TS client: add `batchSet(ops, options)` convenience method
- **Effort:** 1-2 weeks

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
- **Priority:** P2 → **P1 for Cloud** (foundation for memory management)
- **Complexity:** Medium
- **Summary:** Hazelcast-style Write-Behind: staging area, write coalescing, batch flush, retry queue.
- **Context:** [topgun-rocksdb.md](../reference/topgun-rocksdb.md)
- **HC Reference:** `hazelcast/map/impl/mapstore/` — WriteBehindStore, StoreWorker
- **Effort:** 2-3 weeks
- **Storage strategy analysis (2026-03-22):**
  - This TODO is the foundation of the entire storage hierarchy: TODO-033 → TODO-043 (S3) → TODO-040 (Tiered)
  - For cloud launch (v2.0), full Write-Behind is not required. What IS required is a **background LRU evictor** that flushes cold records to PostgreSQL via existing `flush_key()` + `on_evict()` hooks
  - Existing building blocks already in codebase: `max_entry_count`, `estimated_cost()`, `random_samples()`, `RecordMetadata` (last_access_time, hits), `ExpiryPolicy` (TTL, max-idle), `MutationObserver::on_evict()`
  - **Recommended split:** extract LRU evictor as a minimal slice (TODO-033a, 3-5 days) for cloud launch, defer full Write-Behind (staging area, coalescing, batch flush) to v3.0
  - LRU evictor converts TopGun from "data must fit in RAM" to "hot data in RAM, cold data in PostgreSQL" — sufficient for v2.0 cloud


### TODO-188: Fix performance.mdx — Replace TS Server Config Knobs
- **Priority:** P1 (tuning instructions reference non-existent config)
- **Complexity:** Small
- **Summary:** `performance.mdx` documents `eventQueueCapacity`, `eventStripeCount`, `backpressureSyncFrequency`, `writeCoalescingMaxDelayMs` etc. — all TS server config. None exist in Rust. Binary name `topgun-server` incorrect (actual: `test-server` from source). Monitoring metrics section has same TS metric names issue.
- **Fix:** Replace with actual Rust config: `ServerConfig` fields (`max_concurrent_operations`, `gc_interval_ms`), `ConnectionConfig` fields (`outbound_channel_capacity`, `send_timeout`, `idle_timeout`). Fix binary name. Replace metric names with actuals.
- **Ref:** DOCS_AUDIT_REPORT.md — performance.mdx section
- **Effort:** 0.5 day

### TODO-189: Fix cluster-replication.mdx — Remove False Env Vars & Consistency Modes
- **Priority:** P1 (cluster setup instructions don't work)
- **Complexity:** Small
- **Summary:** `cluster-replication.mdx` shows `TOPGUN_CLUSTER_PORT`, `TOPGUN_CLUSTER_SEEDS`, `TOPGUN_NODE_ID`, `TOPGUN_CONSISTENCY` env vars — none parsed. Documents QUORUM/STRONG consistency modes — only eventual consistency exists. Docker Compose cluster example will not configure nodes. Replication metrics are aspirational.
- **What IS accurate:** 271 partitions, `backup_count`, Phi Accrual failure detection (`phi_threshold: 8.0`), gossip discovery (HELLO/MEMBER_LIST), partition rebalancing on failure.
- **Fix:** Add banners noting env var cluster config is planned. Mark QUORUM/STRONG as planned. Keep architecture overview sections (gossip, partitions, failure detection) — they describe real behavior. Remove non-existent replication metrics.
- **Ref:** DOCS_AUDIT_REPORT.md — cluster-replication.mdx section
- **Effort:** 0.5 day

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


### TODO-164: Security Hardening — Production Readiness
- **Priority:** P2 (required for cloud launch)
- **Complexity:** Medium
- **Summary:** Second-tier security improvements identified in audit. Required before TopGun Cloud launch but not blocking self-hosted or demo deployments.
- **Depends on:** TODO-163
- **Effort:** 1-2 weeks
- **Tasks:**
  - [x] RS256/ES256 algorithm support — extracted to **TODO-169** (P1, regression)
  - [ ] Rate limiting on WebSocket auth failures — prevent brute-force (5 failures/min per IP)
  - [ ] HSTS header when TLS enabled
  - [ ] Cluster TLS env var parsing — documented config doesn't work
  - [ ] Token revocation mechanism — in-memory deny list for compromised tokens
  - [ ] PostgreSQL connection TLS — document/enforce `sslmode=require` in DATABASE_URL
  - [ ] Client encryption ergonomics — first-class `encryption: { enabled: true }` option in TopGunClient constructor

### TODO-165: Security Hardening — Enterprise
- **Priority:** P3 (v3.0 enterprise)
- **Complexity:** Large
- **Summary:** Enterprise-grade security features. Gated behind enterprise license (BSL 1.1).
- **Depends on:** TODO-041 (multi-tenancy), TODO-164
- **Effort:** 4-6 weeks
- **Tasks:**
  - [ ] Application-level encryption at rest in PostgreSQL (column encryption with KMS integration)
  - [ ] JWKS endpoint support for automatic key rotation (Clerk, Auth0, Okta)
  - [ ] Audit logging — who accessed what data when (append-only audit trail)
  - [ ] Field-level encryption for multi-tenant data isolation
  - [ ] Non-exportable CryptoKey storage for client device keys
  - [ ] SOC 2 compliance checklist and documentation

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
- **Summary:** Managed cloud on Hetzner CCX33. Shared instance with namespace isolation. Tiers: Free ($0, 100 conn) / Pro ($25, 1K conn) / Team ($79, 5K conn) / Enterprise ($299+).
- **Depends on:** TODO-163 (P0 security), TODO-136 (rate limits), TODO-033a (LRU evictor), TODO-141 (Docker), TODO-150, TODO-151
- **Effort:** 3-4 weeks
- **Context:** [CLOUD_ARCHITECTURE.md](../reference/CLOUD_ARCHITECTURE.md)
- **Architecture (2026-03-22):**
  - **Shared instance model.** All tenants (free + paid) on one cluster. Namespace isolation (tenant ID prefix on map names). NOT full multi-tenancy (TODO-041).
  - **No migration free→paid.** Upgrade = change limits in tenant config. Same instance, same endpoint.
  - **Hetzner CCX33** (32GB, €55/мес). 60 tenants use ~27% RAM. Break-even: 4 Pro clients.
  - **Phase B (v3.0):** Full multi-tenancy (TODO-041) at ~100+ customers.
- **Subtasks:**
  - [ ] Namespace isolation: tenant ID from JWT `sub` → prefix for all map names (3-5 days)
  - [ ] Per-tenant config: connection limit, storage quota, ops/sec rate (part of TODO-136)
  - [ ] PostgreSQL: `tenant_id` column in existing tables
  - [ ] Admin API: CRUD tenant configs (limits, status, plan)
  - [ ] Reverse proxy: Caddy with TLS termination + subdomain routing
  - [ ] Self-service signup flow (UI/UX — open question)
  - [ ] Paddle webhook integration for plan changes
  - [ ] Monitoring: UptimeRobot + Grafana Cloud (free tiers)

### TODO-153: Landing Page & Waitlist
- **Priority:** P2 (pre-launch)
- **Complexity:** Low
- **Summary:** topgun.build landing page with clear value prop, feature comparison, pricing preview, email waitlist.
- **Effort:** 2-3 days
- **Audit (2026-03-21):** Landing page already exists in `apps/docs-astro` (Astro 5 + React 19 + Tailwind 4). Hero section with interactive SyncLab demo, feature comparison matrix, architecture visualization — all production-ready. Remaining work:
  - [ ] Add email waitlist form (currently no signup capture)
  - [ ] Add pricing preview section (tiers from BUSINESS_STRATEGY.md)
  - [ ] Add community links (Discord, Telegram, GitHub Discussions)
  - [ ] Add "Customers/Use Cases" placeholder section
  - [ ] Consider static fallback if demo.topgun.build is unavailable

### TODO-155: Show HN Launch
- **Priority:** P1 (primary launch event)
- **Complexity:** Low
- **Summary:** "Show HN: TopGun — Open-source real-time data platform with offline-first CRDTs (Rust)". Prerequisites: working demo, clean README, quick start, Discord, docs, 1-2 blog posts. Prepare FAQ answers via LLM in advance.
- **Depends on:** TODO-153, TODO-154, TODO-156, TODO-159
- **Effort:** 1 day (prep: 1 week)
- **Launch checklist:**
  - [ ] Demo at demo.topgun.build is stable and monitored
  - [ ] README has clear value prop + quick start + badges
  - [ ] At least 2 blog posts published (technical depth)
  - [ ] Discord server with welcome message + FAQ channel
  - [ ] Pre-written answers to expected HN questions (via LLM)
  - [ ] Uptime monitoring on demo.topgun.build (avoid launch-day downtime)

### TODO-159: Sync Lab Demo Improvements
- **Priority:** P2 (Show HN differentiator)
- **Complexity:** Medium
- **Summary:** Enhance the interactive demo (`examples/sync-lab`) embedded on topgun.build landing page. Current state: Conflict Arena + Latency Race, score B+ for Show HN. Improvements to maximize impact.
- **Depends on:** — (independent, can start anytime)
- **Effort:** 3-5 days total
- **Source:** `examples/sync-lab/`, embedded via `apps/docs-astro/src/components/SyncLabDemo.tsx`
- **Quick wins (high impact, low effort):**
  - [ ] Persistence demo — "Reload Page" button, show data survives browser reload (2-3 hours)
  - [ ] Scale badge — "Add 1000 items" button, show sub-ms latency holds at scale (0.5 day)
  - [ ] Error boundary — wrap ConflictArena/LatencyRace in React error boundary for graceful crashes (1 hour)
  - [ ] Empty state onboarding — replace "No to-dos yet" with guided hint (1 hour)
- **Medium effort (high impact):**
  - [ ] Concurrent edits — 3+ virtual devices editing same field simultaneously, HLC resolves (1-2 days)
  - [ ] Network latency slider — artificial 50ms/200ms/1000ms delay, show app stays responsive (0.5 day)
  - [ ] ORMap tab — demonstrate concurrent adds without deletion races (collaborative tags) (1 day)
- **Stretch (nice to have):**
  - [ ] Merkle delta sync visualization — animated tree showing which keys synced (2-3 days)
  - [ ] Partition awareness — show "271 partitions" stat, color-code which partition each item belongs to (1 day)
  - [ ] Auto-show State/Network log on first visit with guiding tooltip (2 hours)

### TODO-160: README Rewrite
- **Priority:** P1 (first thing visitors see on GitHub)
- **Complexity:** Low
- **Summary:** Rewrite `README.md` based on best practices research (Hazelcast, SurrealDB, Quickwit, Arroyo, TiKV, RisingWave, Databend). Current README has broken links, "Alpha" label (v1.0 shipped), no badges, no numbers.
- **Depends on:** TODO-156 (community channels must exist before linking them)
- **Effort:** 0.5-1 day
- **Timing:** Pre-launch, after community channels are live
- **Research (2026-03-21):** Analyzed 7 popular OSS READMEs. Common pattern: Logo → Badges → Nav → Tagline → "What is X?" → Features → Quick Start → Architecture → Community → License.
- **Structure:**
  - [ ] Badges: CI status, License (Apache 2.0), npm version. Add Discord/Twitter only when channels are active
  - [ ] Nav links: `Docs | Getting Started | Live Demo | Discord` (horizontal, linked)
  - [ ] Tagline: one-liner value prop, NOT architecture jargon (e.g., "Zero-latency reads and writes, offline-first, real-time sync — browser to cluster")
  - [ ] "What is TopGun?" — 3-5 sentences, problem-first (not implementation-first)
  - [ ] "When to use TopGun" — explicit use-case bullets instead of naming competitors
  - [ ] Key Features — grouped, with qualitative claims only until benchmarks are finalized (sub-ms writes, high throughput). Don't publish specific ops/sec numbers until all optimizations are complete and fire-and-forget mode is benchmarked — competitors (Hazelcast, Arroyo) claim millions of events/sec
  - [ ] Quick Start — keep current code examples (they're good), add `docker compose up` path
  - [ ] Architecture diagram — ASCII or linked image: Client (CRDT+IDB) → WebSocket → Rust Server → PostgreSQL
  - [ ] Packages table — keep as-is
  - [ ] Community — Discord, GitHub Discussions, Telegram (RU). Add Twitter/X only when active
  - [ ] License — Apache 2.0 one-liner
- **Fix broken links:**
  - [ ] Remove `tests/benchmark/README.md` link (file doesn't exist)
  - [ ] Remove or move `specifications/` links to docs site (internal docs, not marketing)
  - [ ] Remove "Alpha — API may change" (v1.0 released)
- **What NOT to do:**
  - Don't name competitors directly — state advantages only (user preference)
  - Don't link empty social accounts (Twitter is clean, Discord not created yet)
  - Don't add info that isn't verified — only factual, proven claims
  - Don't over-detail Performance Testing section — move to CONTRIBUTING.md

### TODO-161: Multi-Platform Marketing Strategy
- **Priority:** P2 (pre-launch, but after community setup)
- **Complexity:** Medium (ongoing)
- **Summary:** Multi-platform presence strategy. All social accounts are clean (no posts). Plan: what to post, where, when, who to follow, how to build audience as non-native EN speaker. One blog post → cross-post to 3-5 platforms. Text-first approach minimizes language barrier.
- **Depends on:** TODO-156 (community channels), TODO-160 (README ready for traffic)
- **Effort:** 2 days strategy + 30-45 min/day ongoing
- **Принцип:** Один контент → максимальное покрытие. Блог-пост → Twitter thread + Reddit post + Dev.to + Хабр + LinkedIn.
- **Платформы по приоритету:**
  - **P1 — Core (обязательно к launch):**
    - [ ] **Twitter/X** — build-in-public, short posts + screenshots/GIFs/benchmarks. Follow 50-100: Rust leaders, local-first advocates, CRDT researchers, dev tools founders. Cadence: 3-5 posts/week
    - [ ] **Reddit** — r/rust, r/programming, r/selfhosted, r/webdev. Share blog posts, answer questions. Не спамить — 1-2 поста/месяц + активные комментарии в чужих тредах
    - [ ] **Hacker News** — Show HN (TODO-155) + comment on relevant threads about CRDTs, local-first, real-time sync. Organic presence before launch
  - **P2 — Amplification (первые 2 месяца после launch):**
    - [ ] **Dev.to / Hashnode** — кросс-постинг блога с topgun.build. Бесплатный reach, SEO backlinks. Zero extra effort
    - [ ] **LinkedIn** — professional posts for B2B visibility. Repost blog articles with 2-3 sentence summary. Enterprise segment найдёт здесь
    - [ ] **Хабр** — технические статьи на русском (родной язык, нулевой барьер). RU-аудитория для валидации и первых пользователей
    - [ ] **Telegram** — канал на русском (в TODO-156). Кросс-пост из Хабра + progress updates
  - **P3 — Growth (после $1K MRR):**
    - [ ] **YouTube** — записанные видео-туториалы и deep-dives. Можно перезаписывать. 1-2 видео/месяц
    - [ ] **Lobste.rs** — invite-only HN-альтернатива, Rust-дружелюбная. Попросить invite у Rust community
    - [ ] **Rust-specific:** This Week in Rust (newsletter submissions), Rust subreddit, Rust Discord #showcase
- **Контент-микс (Twitter/X):**
  - 40% build-in-public progress (screenshots, metrics, "today I shipped X")
  - 30% technical insights (CRDT tricks, Rust patterns, benchmark results)
  - 20% community engagement (reply, retweet, quote-tweet relevant content)
  - 10% product announcements (releases, blog posts, demo updates)
- **Формат для non-native speaker:**
  - Twitter: 1-2 предложения + визуал (screenshot, GIF, benchmark chart). LLM-редактура
  - Reddit: longer text через LLM, substantive technical content
  - Dev.to/Хабр: полные блог-посты (RU → LLM → EN для Dev.to, оригинал RU для Хабра)
  - LinkedIn: 2-3 sentences + link. Professional tone через LLM
  - YouTube: подготовленный скрипт, можно перезаписать до идеала
- **Последовательность запуска:**
  - [ ] Week 1: Twitter (follow, first 5 posts) + Reddit (lurk, comment)
  - [ ] Week 2-3: First blog post → cross-post Dev.to + Хабр + Twitter thread + LinkedIn
  - [ ] Week 4: Show HN (TODO-155) → все каналы одновременно amplify
  - [ ] Month 2+: YouTube first video, Lobste.rs invite, consistent cadence
- **What NOT to do:**
  - Don't post until README and docs are launch-ready (visitors will click through)
  - Don't create accounts you can't maintain (better 3 active than 7 dead)
  - Don't self-promote on Reddit without contributing value first (comment history matters)
  - Don't buy followers or engagement — dev community detects this instantly

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

### TODO-166: Terms of Service + Privacy Policy
- **Priority:** P1 (blocker for first paying customer — Risk R-007)
- **Complexity:** Low
- **Summary:** Create ToS and Privacy Policy for topgun.build and TopGun Cloud. Publish on website before accepting payments.
- **Effort:** 1-2 days
- **Approach:**
  - [ ] Generate draft via LLM based on comparable SaaS ToS (Supabase, Convex, Turso)
  - [ ] Cover: data processing, SLA (best-effort for non-Enterprise), data portability/export, account termination, limitation of liability
  - [ ] Privacy Policy: what data is collected, how stored (Hetzner EU), GDPR compliance, no third-party tracking (Plausible analytics)
  - [ ] DPA template for EU B2B clients (can be simplified, not full enterprise DPA)
  - [ ] Optional: legal review via Fiverr/Upwork ($200-500) before launch
  - [ ] Publish on topgun.build/terms and topgun.build/privacy
- **Depends on:** — (can start anytime, text-only task)

### TODO-158: Premium License / Open-Core Tier
- **Priority:** P3 (secondary revenue)
- **Complexity:** Low-Medium
- **Summary:** Dual-license: Apache 2.0 core + commercial license for premium features. Premium candidates: advanced monitoring, priority support, custom connectors, SSO/SAML. Complement to cloud revenue.
- **Effort:** 1 week (license + feature gates)

---

## Execution Order

**Strategy:** Feature-complete open-source core first, then cloud. Cloud launch only after all key differentiators are built and there is a compelling story for marketing and Show HN. No shortcuts to revenue — quality and feature depth come first.

### Milestone 2 — v2.0 (Data Platform)

**Completed waves:** 6a (SPEC-126 Tantivy), 6a¹ (SPEC-131 search fix), 6b (SPEC-127 schema types), 6b² (SPEC-128 write-path, SPEC-129 TS codegen, SPEC-130 Arrow derivation), 6c (SPEC-135a-c DataFusion SQL), 6c² (SPEC-142–145 Query unification), 6f² (SPEC-137 P0 Security, SPEC-138 RS256), 6f²¹ (SPEC-149 auth/security docs), 6f²² (SPEC-141 Shapes docs, SPEC-148 SQL docs).

#### Phase: v2.0-beta — Solid Foundation

*Goal: Make what exists work correctly. Production-ready IMDG with SQL, indexing, RBAC, locks, durability guarantees.*
*Marketing point: "Production-ready IMDG with offline-first CRDTs, SQL queries, indexes, RBAC, distributed locks, and write concern guarantees." — first technical blog posts possible here.*

| # | TODO | Feature | Effort | Blocked by |
|---|------|---------|--------|------------|
| 1 | 186, 187, 188, 189 | Docs fixes (deployment, observability, performance, cluster) | 2 days | — |
| 2 | 177 | Indexing: Hash / Navigable / Inverted | 3-4 weeks | — |
| 3 | 175 | Distributed Locks | 1-2 weeks | — |
| 4 | 171 | RBAC (role-based access control) | 1-2 weeks | SPEC-137 ✓ |
| 5 | 180 | Write Concern (server achievement reporting) | 1-2 weeks | — |
| 6 | 138 | Schema Migrations | 2 weeks | TODO-069 ✓ |

#### Phase: v2.0-rc — Headline Differentiators

*Goal: Build the features that make TopGun unique. Stream processing, connectors, WASM user-defined functions, data > RAM.*
*Marketing point: "Stream processing like Hazelcast Jet, connectors to Kafka/S3/PostgreSQL CDC, user-defined WASM functions — and it all works offline-first. No competitor covers both quadrants." — this is the moment to start content marketing.*

| # | TODO | Feature | Effort | Blocked by |
|---|------|---------|--------|------------|
| 7 | 025 | DAG Executor (stream processing) | 3-4 weeks | TODO-091 ✓ |
| 8 | 092 | Connector Framework (Kafka, S3, PG CDC) | 2 weeks | 025 (DAG integration) |
| 9 | 176 + 179 | WASM Sandbox: Entry Processor + Conflict Resolvers | 4-5 weeks | — (shared infra) |
| 10 | 033 | Write-Behind / LRU (data > RAM) | 2-3 weeks | — |

#### Phase: v2.0-release — DX Polish

*Goal: Best developer experience in the category. Same SQL offline and online, browser DevTools, visual pipeline dashboard.*

| # | TODO | Feature | Effort | Blocked by |
|---|------|---------|--------|------------|
| 11 | 072 | WASM client modules (SQL + search in browser) | 2-3 weeks | TODO-091 ✓ |
| 12 | 101 | Client DevTools | 4-6 weeks | — |
| 13 | 093 v2.0 | Admin Dashboard (pipelines, SQL playground) | 2-3 weeks | 025 + 091 + 092 |
| 14 | 048 | SSE Push (serverless environments) | 2-3 days | — |

#### Phase: v2.0-cloud — Cloud Preparation

*Goal: All infrastructure needed for cloud launch in one sprint. After this block — deploy, Show HN, start accepting payments.*

| # | TODO | Feature | Effort | Blocked by |
|---|------|---------|--------|------------|
| 15 | 136 | Rate Limiting & Quotas | 1-2 weeks | — |
| 16 | 033a | LRU Evictor slice (if not covered by 033) | 3-5 days | — |
| 17 | 137 | Prometheus / OpenTelemetry Metrics | 1 week | — |
| 18 | 164 | Security Hardening (auth rate limit, HSTS, token revocation) | 1-2 weeks | TODO-163 ✓ |
| 19 | 139 | Backup / Restore API | 1-2 weeks | — |
| 20 | 141 | Docker deployment configs | 3-5 days | — |
| 21 | 140 | Webhooks (Zapier/n8n/Make integration) | 1 week | — |
| — | — | Namespace isolation (tenant prefix, per-tenant config, PG tenant_id) | 3-5 days | 136 |

### Milestone 3 — v3.0+ (Enterprise)

| Wave | Items | Blocked by |
|------|-------|------------|
| **7a** | TODO-041 (Multi-Tenancy) · TODO-043 (S3 Bottomless) · TODO-165 (Enterprise Security) | — · 033 · 164 |
| **7b** | TODO-040 (Tiered Storage) · TODO-039 (Vector Search) | 043 · — |
| **7c** | TODO-044 (Bi-Temporal) | 043 |
| **7d** | TODO-095 (Enterprise dir) · TODO-093 v3.0 (Dashboard) | — · 041+040 |
| **7e** | TODO-036 (Extensions) · TODO-102 (Rust CLI) · TODO-142 (Multi-language SDKs) | — |
| **7f** | TODO-174 (Adaptive Indexing) · TODO-178 (Interceptors) · TODO-049 (Cluster HTTP) · TODO-076 (Merkle hash opt) | — |

### Milestone 4 — GTM (Go-to-Market)

*Starts after v2.0-release is complete. Pre-launch marketing (community, landing page, content) can begin during v2.0-rc phase. Cloud launch and Show HN only after v2.0-cloud is done.*

| Wave | Items | Blocked by | Timing |
|------|-------|------------|--------|
| **8a** (pre-launch) | TODO-156 (Community) · TODO-153 (Landing page) · TODO-157 (Content) · TODO-159 (Demo improvements) | — | Start during v2.0-rc |
| **8a²** (pre-launch) | TODO-160 (README rewrite) · TODO-161 (Social strategy) | 156 | After community channels live |
| **8b** (pre-launch) | TODO-150 (Company reg) · TODO-154 (Docs) · TODO-166 (ToS/PP) | — | Start 4-6 weeks before cloud launch |
| **8c** (launch) | TODO-151 (Paddle) · TODO-152 (Cloud: shared instance + Clerk portal) | 150+166 · v2.0-cloud done | After v2.0-cloud phase complete |
| **8d** (launch) | TODO-155 (Show HN) | 153+154+156+159+160 | 1-2 weeks after cloud beta |
| **8e** (post-launch) | TODO-158 (Premium license) · TODO-161 execution | Revenue validation | After first paying customers |

## Dependency Graph

```
MILESTONE 2: Data Platform (v2.0)

  v2.0-beta (Solid Foundation):
  ┌─ TODO-186-189 (Docs fixes) ← no deps, hygiene first
  ├─ TODO-177 (Indexing) ← no deps, O(1) queries
  ├─ TODO-175 (Distributed Locks) ← no deps, coordination
  ├─ TODO-171 (RBAC) ← depends on SPEC-137 ✓
  ├─ TODO-180 (Write Concern) ← no deps
  └─ TODO-138 (Schema Migrations) ← depends on TODO-069 ✓

  v2.0-rc (Headline Differentiators):
  ┌─ TODO-025 (DAG Executor) ← depends on TODO-091 ✓
  ├─ TODO-092 (Connectors) ← depends on TODO-025 (DAG integration)
  ├─ TODO-176 + 179 (WASM Sandbox: Entry Processor + Conflict Resolvers) ← shared infra
  └─ TODO-033 (Write-Behind / LRU) ← no deps, unblocks v3.0 S3

  v2.0-release (DX Polish):
  ┌─ TODO-072 (WASM client: SQL+search in browser) ← depends on TODO-091 ✓
  ├─ TODO-101 (Client DevTools) ← no deps
  ├─ TODO-093 v2.0 (Admin Dashboard) ← depends on 025+091+092
  └─ TODO-048 (SSE Push) ← no deps, 2-3 days

  v2.0-cloud (Cloud Preparation):
  ┌─ TODO-136 (Rate Limits) ← no deps
  ├─ TODO-033a (LRU Evictor slice) ← if not covered by TODO-033
  ├─ TODO-137 (Prometheus/OTel) ← no deps
  ├─ TODO-164 (P2 Security) ← depends on TODO-163 ✓
  ├─ TODO-139 (Backup/Restore) ← no deps
  ├─ TODO-141 (Docker configs) ← no deps
  ├─ TODO-140 (Webhooks) ← no deps
  └─ Namespace isolation ← depends on TODO-136

  Completed:
  ✓ SPEC-126 (Tantivy) · ✓ SPEC-131 (Search fix)
  ✓ SPEC-127-130 (Schema system) · ✓ SPEC-135a-c (DataFusion SQL)
  ✓ SPEC-142-145 (Query unification, Shapes absorbed)
  ✓ SPEC-137 (P0 Security) · ✓ SPEC-138 (RS256) · ✓ SPEC-149 (Auth docs)
  ✓ TODO-027 (DST via SPEC-132a-d)

MILESTONE 3: Enterprise (v3.0+)

  TODO-041 (Multi-Tenancy) ← triggers Cloud Phase B (~100+ customers)
  TODO-043 (S3 Bottomless) ──→ TODO-040 (Tiered) ──→ TODO-044 (Time-Travel)
       ↑
  TODO-033 (Write-Behind, from v2.0)
  TODO-039 (Vector Search)
  TODO-095 (Enterprise dir)
  TODO-165 (Enterprise Security) ← KMS, JWKS, audit logging
  TODO-093 v3.0 (Dashboard) ← depends on 041+040
  TODO-036 (Extensions) · TODO-102 (Rust CLI) · TODO-142 (SDKs)
  TODO-174 (Adaptive Indexing) · TODO-178 (Interceptors) · TODO-049 (HTTP) · TODO-076 (Hash opt)

MILESTONE 4: GTM (pre-launch starts during v2.0-rc)

  TODO-156 (Community) ──→ TODO-160 (README) ──┐
  TODO-153 (Landing)  ────────────────────────┤
  TODO-157 (Content)  ────────────────────────┼→ TODO-155 (Show HN)
  TODO-154 (Docs)     ────────────────────────┤
  TODO-159 (Demo)     ────────────────────────┘
  TODO-156 (Community) ──→ TODO-161 (Social strategy) ← execution after Show HN
  TODO-150 (Company) → TODO-151 (Paddle) → TODO-152 (Cloud) ← needs v2.0-cloud done
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
