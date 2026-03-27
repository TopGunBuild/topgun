# TopGun Roadmap

**Last updated:** 2026-03-27 — CEO review v2: TODO-201 (Client SDK `sql()` method) added to Phase 1 to support product concept "SQL из SDK" claim. Prior: Product concept audit: TODO-200 (CLI Audit & Fix) added to Phase 0; TODO-194 updated for 3 onboarding paths; TODO-191 marked verified (560K/37K); TODO-151 pricing corrected ($25/$79/$299); TODO-187/199 dependencies updated for CLI.
**Strategy:** ~~Feature-complete open-source core first~~ → **Firebase Killer (compressed):** UX-first, Phase 0 validation, simplified RBAC, enterprise features deferred. See CEO plan: `~/.gstack/projects/TopGunBuild-topgun/ceo-plans/2026-03-26-firebase-killer-compressed.md`
**Eng review:** `~/.claude/plans/sequential-frolicking-wave.md` — 14 decisions, all resolved. See review for phase structure.
**Product vision:** "The unified real-time data platform — from browser to cluster to cloud storage"

---

## v1.0 — RELEASED

v1.0 complete. 84 specs archived (SPEC-038–084, 114–122). 540+ Rust tests, 55 integration tests, clippy-clean. Legacy TS server removed. Post-release performance work: PartitionDispatcher (116), async tantivy (117), OP_BATCH splitting (118), scatter-gather Merkle (119), bounded channels (120), Rust load harness (121a-c), WebSocket pipelining (122). Result: 100 → 200,000 ops/sec (2000x). Full v1.0 history in git: `git show b0ab167^:.specflow/todos/TODO.md`.

---

## Milestone 2: Data Platform (v2.0) — Firebase Killer

*Goal: ~~SQL queries, stream processing, schema validation, connectors — competitive with Hazelcast feature set.~~ → **Firebase Killer:** UX-first, production-ready core, admin panel, Docker one-click, template apps, Show HN. Enterprise features deferred until user demand.*

*Execution order: Phase 0 (validation) → Phase 1 (production core) → **SOFT LAUNCH** → Phase 2 (UX) → Phase 3 (cloud) → Phase 4 (Show HN). See Execution Order section below.*

### TODO-069: Schema System *(split into 4 slices)* — ✅ DONE
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

### TODO-091: DataFusion SQL Integration *(converted to SPEC-135)* — ✅ DONE
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
- **Priority:** P2 → **P1** (documented but not implemented — expectation gap for adopters)
- **Complexity:** Medium
- **Summary:** Docs describe a full RBAC system (role-based policies, map pattern matching, field-level security) that was implemented in the old TS server but not ported to Rust. Current Rust server has only basic map-level `read`/`write` booleans with exact-match lookup. Roles are extracted from JWT into `Principal.roles` but never evaluated for data access permissions.
- **Depends on:** SPEC-137 ✓ (auth hardening)
- **Effort:** Phase A: 3-5 days. Phase A-ext: 1 week.
- **TS Reference:** Old TS server had working RBAC — check git history (commit `926e856` removed TS server). Key files to recover via `git show`:
  - `packages/server/src/coordinator/auth-handler.ts` — role extraction + permission assignment
  - `packages/server/src/coordinator/` — permission evaluation logic
  - Search for `PermissionPolicy`, `mapNamePattern`, `allowedFields` in old TS code
- **HC Reference:** `hazelcast/security/` — enterprise security model, permission policies
- **Scope (eng review 2026-03-27 — split into phases):**
  - **Phase A (minimal RBAC — v2.0 Phase 1):** Role→map permission matrix (boolean read/write per role per map). Policies stored in `topgun-rbac.json` file, loaded at startup. Exact map name matching only. Default-allow when no policies configured. Precomputed into per-connection `MapPermissions` cache on auth (O(1) lookup on write path). **PREREQUISITE:** Write regression tests for existing `WriteValidator` behavior before modifying.
  - **Phase A-ext (extended RBAC — v2.0 Phase 3):** Admin API hot-reload via `PUT /api/admin/rbac/policies`. Wildcard `"*"` map matching. Precomputed cache invalidation on reload (broadcast to all connections). TODO-190 simulation test (hot-reload under write load).
  - **Phase B (field-level):** `allowedFields` per policy, field filtering on read responses. More invasive — touches CRDT response serialization. Deferred until users ask.
  - **Phase C (dynamic):** Full policy CRUD via admin API, UI in admin dashboard. Deferred until users ask.
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

### TODO-188: Index Management via Admin API *(revised by eng review 2026-03-27)*
- **Priority:** P1 (bridges server indexing to user-controllable API)
- **Complexity:** Medium
- **Summary:** Admin API endpoints for index create/drop/list. Admin panel UI for index management. `remove_index()` on Rust `IndexRegistry`. Async backfill with progress reporting for existing records. **No client wire protocol changes** — indexes managed via admin panel, not client SDK. (Revised from original wire protocol scope per CEO review outside voice.)
- **Depends on:** SPEC-155 ✓ (server indexing), TODO-177 ✓ (indexing core)
- **Blocks:** Removing "Planned Feature" banner from `indexing.mdx` docs
- **Files (estimated):** `server-rust/src/network/handlers/admin.rs` (add index endpoints), `server-rust/src/service/domain/index/registry.rs` (add `remove_index()`), `apps/admin-dashboard/src/features/indexes/` (new UI page)
- **Phase:** v2.0 Phase 1 (Production Core)
- **Effort:** 1-1.5 weeks
- **Eng review details (2026-03-27):**
  - REST endpoints: `POST /api/admin/indexes` (create), `DELETE /api/admin/indexes/:attr` (remove), `GET /api/admin/indexes` (list + stats), `GET /api/admin/indexes/:id/status` (backfill progress)
  - Backfill: async background task via `store.snapshot_iter()`. Returns 202 Accepted with status `"building"`. Admin panel polls for progress.
  - `remove_index()`: add to `IndexRegistry` (currently only `add_*_index` exists). DashMap removal.
  - Admin panel: index CRUD page with create form (attribute, type), status indicators, remove button

### TODO-189: Webhook DNS Rebinding Protection
- **Priority:** P2 (security hardening)
- **Complexity:** Small
- **Summary:** Add DNS resolution check before webhook HTTP dispatch — resolve the target URL's DNS, verify the resolved IP is not in the private range blocklist, then connect. Prevents DNS rebinding attacks where a domain initially resolves to a public IP (passing the blocklist) but later resolves to an internal IP during the actual HTTP request. Phase 1 ships URL blocklist (blocks private IPs directly); this TODO adds the DNS-level bypass prevention.
- **Depends on:** TODO-140 (Webhooks), TODO-164 (Security Hardening)
- **Effort:** 2-3 hours
- **Phase:** v2.0 Phase 3 (Cloud Readiness) — post-webhook security hardening

### TODO-190: RBAC Policy Hot-Reload Chaos Test
- **Priority:** P2 (test hardening)
- **Complexity:** Small
- **Summary:** Simulation test that hot-reloads RBAC policies during high write load and verifies no writes slip through during the transition from old policies to new ones. Uses existing SimCluster framework. Validates that policy update is atomic — no window where writes are evaluated against partially-updated policy set.
- **Depends on:** TODO-171 Phase A-ext (RBAC hot-reload)
- **Effort:** 3 hours
- **Phase:** v2.0 Phase 3 (bundled with RBAC extended — hot-reload + wildcard)

### TODO-175: Distributed Locks — Rust Port
- **Priority:** ~~P2~~ → **P3 DEFERRED** (documented as available — deferred per Firebase Killer pivot)
- **Complexity:** Medium
- **Summary:** Feature described in `distributed-locks.mdx` — distributed locking with fencing tokens. Wire messages (`LockRequest`, `LockRelease`) exist in core-rust and are routed to `CoordinationService`, but `coordination.rs` returns `NotImplemented` for both (confirmed by AC6 test). Feature is a stub.
- **Documented in:** `guides/distributed-locks.mdx` — presented as available but currently returns NotImplemented at runtime
- **TS Reference:** Old TS server had working distributed locks — recover via git:
  - `git show 926e856^:packages/server/src/coordinator/` — look for lock-handler, distributed-lock files
  - Search: `git show 926e856^:packages/server/src/` for `LockRequest`, `fencing_token`, `acquireLock`
- **HC Reference:** `hazelcast/cp/` — CP subsystem, FencedLock, ILock patterns
- **Effort:** 1-2 weeks

### TODO-176: Entry Processor — Rust Port (WASM Sandbox)
- **Priority:** ~~P2~~ → **P3 DEFERRED** (documented as available — deferred per Firebase Killer pivot)
- **Complexity:** Large
- **Summary:** Feature described in `entry-processor.mdx` — atomic read-modify-write operations executed server-side. Wire messages (`EntryProcess`, `EntryProcessBatch`) exist in core-rust and are routed to `PersistenceService`, but `persistence.rs` comment says "stub — WASM sandbox required" and returns `NotImplemented` for all calls.
- **Documented in:** `guides/entry-processor.mdx` — presented as available but currently returns NotImplemented at runtime
- **TS Reference:** Old TS server had working entry processor with sandbox execution — recover via git:
  - `git show 926e856^:packages/server/src/` — search for entry-processor, sandbox, execute files
  - Search: `git show 926e856^:packages/server/src/` for `EntryProcessor`, `sandbox`, `vm.runInContext`
- **HC Reference:** `hazelcast/map/impl/operation/` — EntryProcessor execution patterns
- **Effort:** 3-4 weeks (requires WASM sandbox or Deno-based execution environment)


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
- **Priority:** ~~P2~~ → **P3 DEFERRED** (documented as available — deferred per Firebase Killer pivot)
- **Complexity:** Large
- **Summary:** Feature described in `conflict-resolvers.mdx` — custom server-side JavaScript conflict resolution functions. `ConflictResolver` struct and `RegisterResolver`/`UnregisterResolver`/`ListResolvers` messages exist in core-rust. `PersistenceService` routes these ops, but `handle_register_resolver()` returns `NotImplemented`. Comment: "stub — WASM sandbox required". Feature is functionally unavailable at runtime.
- **Documented in:** `guides/conflict-resolvers.mdx` — presented as available but returns NotImplemented. Code examples will compile but fail at runtime.
- **TS Reference:** Old TS server had working conflict resolvers — recover via git:
  - `git show 926e856^:packages/server/src/` — search for conflict-resolver, resolver-registry, sandbox files
  - Search: `git show 926e856^:packages/server/src/` for `ConflictResolver`, `registerResolver`, `sandbox`
- **Effort:** 3-4 weeks (requires WASM sandbox, same infrastructure as TODO-176)
- **Note:** TODO-176 (Entry Processor) and TODO-179 (Conflict Resolvers) both require WASM sandbox — implement together as a shared infrastructure project

### TODO-180: Write Concern — Server Achievement Reporting
- **Priority:** P2 → **P1** (documented as reporting achieved_level, but server always returns None)
- **Complexity:** Small-Medium
- **Summary:** `write-concern.mdx` documents that `OpAck` responses include `achieved_level` (the durability level actually achieved). In practice, `crdt.rs` lines 197 and 267 always set `achieved_level: None`. The `WriteConcern` wire protocol exists (FIRE_AND_FORGET, MEMORY, APPLIED, REPLICATED, PERSISTED) but the server never reports back what was achieved. Also: `setWithAck()` and `batchSet()` methods shown in docs do not exist in TS client.
- **Documented in:** `guides/write-concern.mdx` — `achieved_level` claim is false, `setWithAck()` does not exist
- **Phase:** v2.0 Phase 1 (APPLIED only). PERSISTED level ships in Phase 3 alongside LRU Evictor (TODO-033a).
- **Scope (eng review 2026-03-27 — server + client in Phase 1):**
  - Server: populate `achieved_level: Some("APPLIED".into())` in `OpAckPayload` after CRDT merge succeeds. Levels: FIRE_AND_FORGET, MEMORY, APPLIED (Phase 1), REPLICATED, PERSISTED (Phase 3 via TODO-033a flush path).
  - TS client: add `setWithAck(key, value, options?)` method returning `{ achievedLevel: string, latencyMs: number }`
  - TS client: add `batchSet(ops, options?)` method returning `{ results: [...], achievedLevel }`
  - Without client methods, the feature is invisible to users — both must ship together.
- **Effort:** 3-5 days (server: 1 day, client: 2-3 days, tests: 1 day)

### TODO-025: DAG Executor for Stream Processing
- **Priority:** ~~P1~~ → **P3 DEFERRED** (Hazelcast Jet equivalent — deferred per Firebase Killer pivot 2026-03-26)
- **Complexity:** Large
- **Summary:** Distributed stream processing DAG. SQL-defined pipelines compiled to operator graphs with windowed aggregation, stateful processing, and checkpointing. petgraph DiGraph, operator chaining, barrier-based checkpointing, shuffle edges.
- **Context:** [HAZELCAST_DAG_EXECUTOR_SPEC.md](../reference/HAZELCAST_DAG_EXECUTOR_SPEC.md) (restored from git `b0ab167^`)
- **HC Reference:** `hazelcast/jet/core/`, `jet/impl/execution/` — Jet DAG model, cooperative multithreading, snapshot barriers
- **Rust Reference:** Arroyo `crates/arroyo-planner/` — DataFusion LogicalPlan → streaming operators (ArroyoRewriter, PlanToGraphVisitor). Key: DataFusion as parser/optimizer only, physical execution replaced with streaming operators (UpdatingAggregateOperator, WindowAggregate, WatermarkNode). This is the pattern for making SQL live in TopGun.
- **Relevance to Live SQL:** TODO-025 is the prerequisite for live SQL with aggregations/JOINs/GROUP BY. Simple SELECT...WHERE can be made live earlier via SQL→predicate bridge (see TODO-025 notes). Full incremental view maintenance requires this DAG.
- **Depends on:** TODO-091
- **Effort:** 3-4 weeks

### TODO-092: Connector Framework
- **Priority:** ~~P2~~ → **P3 DEFERRED** (extensible data ingestion/egress — deferred per Firebase Killer pivot)
- **Complexity:** Medium
- **Summary:** Trait-based connector system: `ConnectorSource`, `ConnectorSink`, `Codec` traits. Connector registry. Initial connectors: Kafka source/sink, S3 sink, PostgreSQL CDC source.
- **Ref:** Arroyo connector traits, ArkFlow Input/Output/Codec, RisingWave (`/Users/koristuvac/Projects/rust/risingwave/src/connector/`)
- **Depends on:** TODO-025 (DAG integration only; connector traits are independent)
- **Effort:** 2 weeks

### TODO-033: AsyncStorageWrapper (Write-Behind)
- **Priority:** ~~P1 for Cloud~~ → **P2** (foundation for memory management — Phase 3)
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
  - **Phase 3 sequencing (eng review 2026-03-26):** LRU evictor's flush-to-PostgreSQL path IS the Write Concern PERSISTED acknowledgment path. Implement PERSISTED level as part of evictor work, not as a separate item — avoids building the async persistence pipeline twice. Phase 1 delivers APPLIED-only; Phase 3 delivers PERSISTED via evictor flush.
  - **Evictor implementation spec (eng review 2026-03-27):** Dedicated tokio task, configurable interval (default 10s). Reservoir sampling: `random_samples(100)` via lock-free DashMap shard iteration. Sort samples by `last_access_time`, evict coldest 25%. Each eviction calls `flush_key()` to PostgreSQL before removal. Write Concern PERSISTED acknowledged after flush completes. Failure handling: if PG is down during `flush_key()`, skip eviction for that record (do not evict without persistence — data loss risk).




### TODO-036: Pluggable Extension System
- **Priority:** ~~P2~~ → **P3 DEFERRED** (deferred per Firebase Killer pivot)
- **Complexity:** Medium
- **Summary:** Modular extension system for community contributions (crypto, compression, audit, geo).
- **Context:** [TURSO_INSIGHTS.md](../reference/TURSO_INSIGHTS.md) Section 5
- **Effort:** 2-3 weeks

### TODO-072: Selective WASM Modules for Client
- **Priority:** ~~P2~~ → **P3 DEFERRED** (deferred per Firebase Killer pivot)
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
- **Priority:** ~~P2~~ → **P3 DEFERRED** (DX differentiator — deferred per Firebase Killer pivot)
- **Complexity:** Medium-Large
- **Summary:** Browser DevTools panel or in-app debug overlay: local replica state, pending sync queue, HLC timeline, connection status, CRDT merge history.
- **Effort:** 4-6 weeks

### TODO-102: Rust CLI (clap)
- **Priority:** P3
- **Complexity:** Medium
- **Summary:** Rewrite CLI from JavaScript (commander) to Rust (clap). Single `topgun` binary: `topgun serve`, `topgun status`, `topgun debug crdt`, `topgun sql`.
- **Effort:** 1-2 weeks

### TODO-093 v2.0: Admin Dashboard — Data Platform Features
- **Priority:** ~~P2~~ → **P3 DEFERRED** (deferred per Firebase Killer pivot — depends on DAG/Connectors)
- **Complexity:** Medium
- **Summary:** Pipeline visualization (ReactFlow + Dagre), live metric coloring by backpressure, DataFusion SQL playground, connector wizard, schema browser.
- **Ref:** Arroyo WebUI (`/Users/koristuvac/Projects/rust/arroyo/webui/`)
- **Depends on:** TODO-025, TODO-091, TODO-092
- **Effort:** 2-3 weeks

### TODO-187: SetupWizard — Audit & Backend Implementation
- **Priority:** P2 → **P1** (DX, first-run experience — de-risks Phase 2)
- **Complexity:** Medium
- **Summary:** Audit the full SetupWizard implementation and implement missing backend. Three parts: (1) **Audit existing frontend** — review `apps/admin-dashboard/src/features/setup/SetupWizard.tsx` for correctness, security (credential handling, CSRF), UX completeness, and alignment with current server capabilities. (2) **Implement Rust backend** — add `POST /api/setup` (apply config, create admin user, restart) and `POST /api/setup/test-connection` (validate DB connectivity) endpoints in `packages/server-rust/src/network/handlers/`. Add bootstrap mode to `GET /api/status` (`configured: false` when unconfigured, currently hardcoded `true`). (3) **Audit CLI setup** — review `bin/commands/setup.js` and `.env.auto-setup.example` for consistency with UI wizard and server config model. Ensure zero-touch (`TOPGUN_AUTO_SETUP=true`) and interactive paths both work end-to-end.
- **Files:** `SetupWizard.tsx`, `admin.rs`, `bin/commands/setup.js`, `.env.auto-setup.example`, `tests/cli/setup.test.ts`
- **Depends on:** — (no blockers; admin API already exists)
- **Phase:** v2.0 Phase 1 (late) — de-risks Phase 2 by removing the Phase 2 blocker. Eng review 2026-03-27.
- **Effort:** 3-5 days

### TODO-136: Rate Limiting & Quotas
- **Priority:** ~~P1~~ → **P2** (required for cloud free tier — Phase 3)
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
- **Priority:** ~~P2~~ → **P3 DEFERRED** (production readiness — deferred per Firebase Killer pivot)
- **Complexity:** Medium
- **Summary:** Versioned schema evolution: add/remove fields, change types with coercion rules. Migration defined in `topgun.schema.ts`, applied via CLI or API. Backward-compatible reads during migration window.
- **Depends on:** TODO-069
- **Effort:** 2 weeks

### TODO-139: Backup/Restore API
- **Priority:** P2 (cloud self-serve)
- **Complexity:** Medium
- **Summary:** `POST /admin/backup` triggers consistent snapshot (pause writes, snapshot RecordStore + OpLog + MerkleTree, resume). `POST /admin/restore` from snapshot (atomic: restore to temp state, verify, then swap). Pre-flight disk space check. Exclude security config/secrets from snapshots. JSON + MsgPack format. Needed for cloud self-serve and enterprise compliance.
- **Phase:** v2.0 Phase 3 (Cloud Readiness)
- **Effort:** 1-2 weeks

### TODO-140: Webhooks
- **Priority:** P3 → **P2** (integration — accepted in CEO review cherry-pick)
- **Complexity:** Medium
- **Summary:** User-configurable HTTP callbacks on data events (insert, update, delete per map). WebhookService domain service with event registry per map, HTTP POST dispatch via reqwest, retry (3x exponential backoff), delivery log, SSRF protection (block private IPs + cloud metadata endpoints), dedicated connection pool with circuit breaker. Admin API for webhook CRUD. Enables Zapier/n8n/Make integration.
- **Phase:** v2.0 Phase 3 (Cloud Readiness) — moved from Phase 2 per eng review 2026-03-27. Not needed for onboarding; aligns with security hardening.
- **Effort:** 1-2 weeks
- **Eng review details (2026-03-27):**
  - Bounded delivery queue: `tokio::sync::mpsc` bounded channel (10K capacity). If full, drop oldest undelivered. Log dropped count via metrics counter.
  - SSRF protection: resolve target URL DNS, verify resolved IP is not in private range blocklist. See also TODO-189 (DNS rebinding).
  - Circuit breaker: per-webhook-URL. Open after 5 consecutive failures. Half-open after 60s. Close after 1 success.


### TODO-164: Security Hardening — Production Readiness
- **Priority:** P2 (required for cloud launch)
- **Complexity:** Medium
- **Summary:** Second-tier security improvements identified in audit. Required before TopGun Cloud launch but not blocking self-hosted or demo deployments.
- **Depends on:** SPEC-137 ✓ (auth hardening — was TODO-163)
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
- **Depends on:** TODO-041 (multi-tenancy), TODO-164 (production security hardening)
- **Effort:** 4-6 weeks
- **Tasks:**
  - [ ] Application-level encryption at rest in PostgreSQL (column encryption with KMS integration)
  - [ ] JWKS endpoint support for automatic key rotation (Clerk, Auth0, Okta)
  - [ ] Audit logging — who accessed what data when (append-only audit trail)
  - [ ] Field-level encryption for multi-tenant data isolation
  - [ ] Non-exportable CryptoKey storage for client device keys
  - [ ] SOC 2 compliance checklist and documentation

### TODO-141: Cloud Deployment Configs (Production)
- **Priority:** P2 (GTM prerequisite)
- **Complexity:** Low
- **Summary:** **Production-only** Docker configs — distinct from Phase 2 dev setup. Multi-stage minimal Dockerfile, production Docker Compose (single-node), Docker Compose (3-node cluster), Hetzner Cloud deployment guide, health check endpoints. Phase 2 delivers the *development* docker-compose with demo data and admin profile; this TODO delivers *production hardening*.
- **Effort:** 3-5 days

### TODO-142: Multi-Language SDKs (Python, Go)
- **Priority:** P3 (market expansion)
- **Complexity:** Large
- **Summary:** Python and Go client SDKs. MsgPack wire protocol makes this feasible (language-agnostic). Start with Python (larger market). Core: connect, authenticate, CRUD, subscribe, offline queue.
- **Effort:** 3-4 weeks per SDK

### TODO-194: Phase 0 — Getting Started Validation
- **Priority:** P1 (validates entire plan)
- **Complexity:** Small-Medium
- **Summary:** Write "Getting Started in 60 seconds" tutorial, follow it yourself, fix every blocker. Validate **all three onboarding paths:**
  1. **Docker path:** `docker compose up` → working server + admin panel + demo data in <2 min
  2. **CLI path:** `topgun doctor && topgun setup && topgun dev` → server running in <2 min (depends on TODO-200)
  3. **SDK path:** `npm install @topgunbuild/client` + 10 lines of code → data syncing with running server
  Create headless client seed script for demo data (valid HLC + Merkle state, <500 records, <5s seed time, runs as init container in docker-compose).
- **Depends on:** TODO-200 (CLI must work before CLI path can be validated)
- **Phase:** v2.0 Phase 0 (Validation) — first thing to execute
- **Effort:** 2-3 weeks
- **Origin:** CEO plan Phase 0, eng review 2026-03-27, updated 2026-03-27 (three onboarding paths per product concept audit)

### TODO-195: SQL Tab in Admin QueryPlayground
- **Priority:** P1 (key demo feature)
- **Complexity:** Small
- **Summary:** Add SQL query tab to existing QueryPlayground (Monaco editor). Queries execute via admin-authenticated WebSocket connection through existing QueryService → DataFusion backend. Technical path: Admin Panel → WS connect (admin JWT) → QueryService → DataFusionBackend → results. Needs: query timeout (30s), result limit (10K rows), cancel button. Decoupled from SetupWizard — can ship independently.
- **Depends on:** SPEC-135 ✓ (DataFusion), existing QueryService + admin WS connection
- **Phase:** v2.0 Phase 1 (Production Core)
- **Effort:** 2-3 days
- **Origin:** eng review 2026-03-27 (decoupled from SetupWizard, moved to Phase 1)

### TODO-196: "Planned Feature" Cleanup
- **Priority:** P2 (trust-building)
- **Complexity:** Small
- **Summary:** Replace docs pages for Distributed Locks, Entry Processor, Conflict Resolvers, Interceptors with honest "Coming Soon" pages + GitHub tracking issues. Audit client SDK surface for methods that call NotImplemented endpoints — either remove or add clear deprecation warnings. 77 wire protocol message types exist, many are stubs — identify which are real vs stub.
- **Phase:** v2.0 Phase 2 (User Experience)
- **Effort:** 1-2 days
- **Origin:** CEO plan Phase 1 → moved to Phase 2 by eng review 2026-03-27

### TODO-197: Template Apps (todo, chat, e-commerce)
- **Priority:** P2 (onboarding)
- **Complexity:** Medium
- **Summary:** 3 template apps: (1) Collaborative todo — offline sync + conflict resolution. (2) Real-time chat — multi-user messaging. (3) E-commerce catalog — product search + filtering via predicate queries. All use React + TopGun client SDK. Zero external dependencies — only TopGun server from docker-compose. Shared `packages/template-base` with TopGunProvider setup, auth helpers, docker-compose fragment, shared CSS. Runnable via `docker compose up`.
- **Depends on:** TODO-194 (Docker compose ready)
- **Phase:** v2.0 Phase 2 (User Experience)
- **Effort:** 2-3 weeks
- **Origin:** CEO plan Phase 2, eng review 2026-03-27 (shared base for DRY, zero ext deps)

### TODO-198: "Migrating from Firebase" Guide
- **Priority:** P2 (content marketing)
- **Complexity:** Small
- **Summary:** Map Firebase concepts → TopGun equivalents: collections→maps, onSnapshot→subscribe, security rules→RBAC, Firestore queries→predicate queries. Note what TopGun does NOT replace (auth, hosting, Cloud Functions). Content marketing piece for Show HN.
- **Depends on:** TODO-171 (RBAC)
- **Phase:** v2.0 Phase 2 (User Experience) — moved from Phase 4 per eng review 2026-03-27
- **Effort:** 2-3 days
- **Origin:** CEO plan cherry-pick #3

### TODO-199: Docs Audit & Accuracy Pass
- **Priority:** P1 (trust — soft launch sends people to docs)
- **Complexity:** Small-Medium
- **Summary:** Audit all 51 pages in `apps/docs-astro/` against current Rust server codebase. For each page, verify that API descriptions, code examples, and behavioral claims match reality. Fix or flag every discrepancy. **Two categories:**
  - **NotImplemented features** (entry-processor, conflict-resolvers, interceptors, distributed-locks, adaptive-indexing) — handled by TODO-196 (Coming Soon pages). This audit verifies those 5 pages are covered AND checks that no other pages reference NotImplemented APIs as available.
  - **Implemented features with stale docs** — RBAC (docs may describe glob patterns / field-level access that don't exist yet), Write Concern (`setWithAck()` described but not in client SDK until TODO-180), Observability (references TODO-137 metrics that don't exist yet), Indexing (must reflect SPEC-155 reality), community.mdx (Discord/Telegram links point to `#`). Reference pages (client.mdx, server.mdx, cli.mdx) may describe APIs that changed during Rust migration.
- **Depends on:** TODO-171 Phase A, TODO-180, TODO-188 (audit must run after Phase 1 features land — they change what's "true")
- **Phase:** v2.0 Phase 1 (late, before soft launch)
- **Effort:** 2-3 days
- **Note:** All NotImplemented features (entry processors, conflict resolvers, interceptors, distributed locks, adaptive indexing) were previously implemented in the old TypeScript server, removed at commit `926e856`. The TS implementations are recoverable via `git show 926e856^:packages/server/src/` and serve as reference for future Rust ports. This context is already captured in individual TODO entries (171, 174, 175, 176, 178, 179) but is noted here as the single source of truth for the docs audit scope.

### TODO-200: CLI Audit & Fix — Verify All Commands Against Rust Server
- **Priority:** P1 (onboarding — second entry path alongside Docker)
- **Complexity:** Small-Medium
- **Summary:** The existing CLI (`bin/topgun.js`, Commander.js) has 15 commands built for the old TypeScript server. Many likely broken or misaligned with the current Rust server. Audit and fix every command end-to-end:
  - **Must work:** `topgun doctor` (verify prerequisites), `topgun setup` (interactive setup → .env → dependencies), `topgun dev` (start Rust server — verify binary name `target/release/test-server` is correct), `topgun config --show` (display current config)
  - **Must work:** `topgun docker:start/stop/status/logs` (Docker Compose profiles: admin, monitoring, dbtools, k6, cluster)
  - **Must work:** `topgun codegen` (schema → types generation via `@topgunbuild/schema`)
  - **Verify:** `topgun cluster:start/stop/status` (multi-node Rust server spawn with correct env vars)
  - **Verify:** `topgun debug:crdt` and `topgun search:explain` (require `TOPGUN_DEBUG=true` and metrics port 9091 endpoints — do these Rust endpoints exist?)
  - **Verify:** `topgun test` (all 12 scopes — correct commands?)
  - **Sync with TODO-187:** If SetupWizard backend changes the config flow (`POST /api/setup`), CLI `setup.js` must be consistent. Ensure both zero-touch (`.env.auto-setup.example`) and interactive paths work.
  - **Docs:** Verify `cli.mdx` docs page describes commands that actually work (part of TODO-199 audit scope but blocked by this fix).
- **Files:** `bin/topgun.js`, `bin/commands/*.js`, `bin/commands/cluster/`, `bin/commands/debug/`, `.env.example`
- **Depends on:** — (no blockers; audit against current Rust server codebase)
- **Phase:** v2.0 Phase 0 (Validation) — prerequisite for TODO-194 CLI path validation
- **Effort:** 2-3 days
- **Origin:** Product concept audit 2026-03-27 — CLI is the developer's daily interface but was completely absent from roadmap

### TODO-201: Client SDK SQL Method (`client.sql()`)
- **Priority:** P1 (product concept differentiator — "SQL из SDK")
- **Complexity:** Small
- **Summary:** Add `client.sql(query, params?)` method to the TypeScript client SDK. Sends SQL string over existing WebSocket connection to QueryService → DataFusion backend → returns results. This is a thin wrapper over the same server path that TODO-195 (SQL tab in admin panel) uses. NOT client-side SQL execution — queries are executed on the server. Enables the product concept claim "SQL-запросы из SDK."
- **API:**
  ```typescript
  const results = await client.sql("SELECT * FROM products WHERE price > ?", [10]);
  // Returns: { columns: string[], rows: any[][], rowCount: number }
  ```
- **Depends on:** SPEC-135 ✓ (DataFusion SQL on server)
- **Phase:** v2.0 Phase 1 (Production Core) — ships alongside TODO-195 (SQL tab)
- **Effort:** 2-3 days
- **Origin:** CEO review 2026-03-27 — product concept claims "SQL из SDK" but no SDK method exists

### TODO-191: ~~Verify 200K ops/sec Benchmark Claim~~ → Document Verified Benchmark Numbers
- **Priority:** P2 (positioning integrity)
- **Complexity:** Small
- **Summary:** ~~Re-run the load harness under fire-and-wait conditions.~~ **Benchmark verified (2026-03-27):** fire-and-wait: ~37K confirmed writes/sec (sub-2ms median latency); fire-and-forget: ~560K ops/sec (OS-bottlenecked — macOS socket buffers, server ceiling higher). 200 WebSocket connections, in-process benchmark, Apple M1 Max. Remaining work: document these numbers in README benchmarks section as part of TODO-160 (README Rewrite). Use marketing-safe formulation: "500K+ ops/sec throughput, sub-2ms median write latency."
- **Depends on:** —
- **Phase:** Absorbed into TODO-160 (README Rewrite, Phase 4)
- **Effort:** 30 min (write benchmark section for README)
- **Origin:** eng review outside voice (2026-03-27), updated 2026-03-27 after benchmark run

### TODO-192: Docker Image CI/CD — GitHub Actions + GHCR
- **Priority:** P2 (distribution)
- **Complexity:** Small
- **Summary:** GitHub Actions workflow for automated Docker image build and push to GitHub Container Registry (GHCR) on git tag. The `deploy/Dockerfile.server` exists but no CI/CD publishes it. Enables `docker pull ghcr.io/topgunbuild/server:latest` convenience for users and is a prerequisite for cloud launch. Also build and push admin dashboard image.
- **Depends on:** TODO-141 (Docker production configs)
- **Phase:** v2.0 Phase 3 (Cloud Readiness)
- **Effort:** 3-5 hours
- **Origin:** eng review distribution check (2026-03-27)

### TODO-193: Phase 1 Lightweight Metrics Counters
- **Priority:** P1 (observability)
- **Complexity:** Small
- **Summary:** Add basic atomic counters to the Rust server: ops/sec, active connections, RBAC denials, write concern acks. Expose via existing `GET /api/status` endpoint (extend `ServerStatusResponse`). This is NOT full Prometheus (TODO-137) — just enough to debug Phase 1 features without flying blind. Uses `std::sync::atomic::AtomicU64` counters, no external crates.
- **Depends on:** —
- **Phase:** v2.0 Phase 1 (Production Core)
- **Effort:** 2-3 hours
- **Origin:** eng review performance section + outside voice (2026-03-27)

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
- **Summary:** Integrate Paddle as Merchant of Record. Handles VAT/sales tax globally. Subscription tiers: Free ($0) / Pro ($25/мес) / Team ($79/мес) / Enterprise ($299+/мес). ~~Old pricing: $99/$299/$999 — superseded by Decision Log 2026-03-22 in STRATEGIC_REVIEW.md.~~ Paddle approval takes 1-2 weeks — start early.
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
- **Positioning (eng review 2026-03-26):** Use **"Firestore alternative"** not "Firebase Killer" in Show HN title and README. Firebase includes Auth, Hosting, Cloud Functions — claiming to "kill" it sets expectations TopGun can't meet. "Firestore alternative with offline-first, SQL, and search" is honest and compelling. Avoids HN comments like "this doesn't replace Firebase."
- **Depends on:** TODO-153, TODO-156, TODO-159, TODO-160
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

**Strategy:** ~~Feature-complete open-source core first~~ → **Firebase Killer (compressed).** UX-first, soft launch after Phase 1, Show HN after Phase 4. Enterprise features deferred until user demand. See eng review: `~/.claude/plans/sequential-frolicking-wave.md`

### Milestone 2 — v2.0 (Firebase Killer)

**Completed:** SPEC-126 (Tantivy), SPEC-127–130 (Schema), SPEC-131 (Search fix), SPEC-135a-c (DataFusion SQL), SPEC-142–145 (Query unification), SPEC-137 (P0 Security), SPEC-138 (RS256), SPEC-149 (Auth docs), SPEC-150–155 (Docs fixes + Indexing). TODO-069 ✓, TODO-091 ✓, TODO-177 ✓.

#### Phase 0: Validation (3-4 weeks)

*Goal: Validate the plan. Fix CLI, write Getting Started guide, follow it via all three paths (Docker, CLI, SDK), fix every blocker it reveals.*

| # | TODO | Feature | Effort | Blocked by |
|---|------|---------|--------|------------|
| 1 | 200 | CLI Audit & Fix (verify all 15 commands against Rust server) | 2-3 days | — |
| 2 | 194 | Getting Started guide + Docker demo data + compose polish + validate 3 onboarding paths | 2-3 weeks | 200 |

#### Phase 1: Production Core (7-9 weeks)

*Goal: Make TopGun credible for real apps. RBAC, write acknowledgments, index management, SQL queries, first-run wizard.*

| # | TODO | Feature | Effort | Blocked by |
|---|------|---------|--------|------------|
| 3 | 193 | Lightweight metrics counters (ops/sec, connections, RBAC denials) | 2-3 hours | — |
| 4 | 171 Phase A | Simplified RBAC (role→map boolean, JSON file, startup load) | 3-5 days | SPEC-137 ✓ |
| 5 | 180 | Write Concern APPLIED (server + client setWithAck/batchSet) | 3-5 days | — |
| 6 | 188 | Index Admin API (REST CRUD, async backfill, remove_index) | 1-1.5 weeks | SPEC-155 ✓ |
| 7 | 195 | SQL tab in Admin QueryPlayground (Monaco → WS → DataFusion) | 2-3 days | SPEC-135 ✓ |
| 8 | 201 | Client SDK `sql()` method (WS → QueryService → DataFusion) | 2-3 days | SPEC-135 ✓ |
| 9 | 187 | SetupWizard backend (POST /api/setup, test-connection) + sync with CLI setup.js | 3-5 days | 200 |
| 10 | 199 | Docs audit & accuracy pass (51 pages + cli.mdx vs current codebase) | 2-3 days | 171+180+188+200 |

#### ── SOFT LAUNCH ── (r/rust, r/selfhosted, GUN.js Discord)

*Note: TODO-191 (benchmark verification) completed 2026-03-27. Numbers documented: 500K+ ops/sec throughput, sub-2ms median write latency. Remaining: write benchmark section for README (part of TODO-160, Phase 4).*

#### Phase 2: User Experience (6-8 weeks)

*Goal: Make TopGun accessible to non-technical users. Templates, docs cleanup, migration guide.*

| # | TODO | Feature | Effort | Blocked by |
|---|------|---------|--------|------------|
| 11 | 196 | "Planned Feature" cleanup (Coming Soon pages, SDK audit) | 1-2 days | — |
| 12 | 197 | Template apps: todo + chat + e-commerce (shared base, zero ext deps) | 2-3 weeks | 194 |
| 13 | 198 | "Migrating from Firebase" guide | 2-3 days | 171 |
| — | — | Admin Dashboard polish (based on soft launch feedback, component tests for key interactions) | 1-2 weeks | Phase 1 |
| — | — | Getting Started guide finalization — all 3 paths (based on soft launch feedback) | 2-3 days | 194+200 |

#### Phase 3: Cloud Readiness (4-5 weeks)

*Goal: Production infrastructure. Rate limits, persistence, security, observability, webhooks.*

| # | TODO | Feature | Effort | Blocked by |
|---|------|---------|--------|------------|
| 14 | 136 | Rate Limiting & Quotas (governor + Tower) | 1-2 weeks | — |
| 15 | 033a | LRU Evictor (reservoir sampling, flush to PG, PERSISTED) | 3-5 days | — |
| 16 | 171 Phase A-ext | RBAC extended (hot-reload, wildcard, precomputed cache invalidation) | 1 week | 171 Phase A |
| 17 | 190 | RBAC hot-reload simulation test | 3 hours | 171 Phase A-ext |
| 18 | 137 | Prometheus / OpenTelemetry Metrics | 1 week | — |
| 19 | 164 | Security Hardening (auth rate limit, HSTS, token revocation) | 1-2 weeks | SPEC-137 ✓ |
| 20 | 141 | Docker production configs | 3-5 days | — |
| 21 | 139 | Backup / Restore API | 1-2 weeks | — |
| 22 | 140 | Webhooks (bounded queue 10K, SSRF, circuit breaker) | 1-2 weeks | — |
| 23 | 189 | Webhook DNS Rebinding Protection | 2-3 hours | 140+164 |
| 24 | 192 | Docker Image CI/CD (GitHub Actions + GHCR) | 3-5 hours | 141 |
| — | — | Namespace isolation (tenant prefix, per-tenant config) | 3-5 days | 136 |

#### Phase 4: Show HN Launch (2-3 weeks)

*Goal: Maximum launch impact. README, community, landing page, demo polish.*

| # | TODO | Feature | Effort | Blocked by |
|---|------|---------|--------|------------|
| 25 | 156 | Community setup (Discord, Telegram) | 2-3 hours | — |
| 26 | 160 | README rewrite ("Firestore alternative" positioning) | 0.5-1 day | 156 |
| 27 | 153 | Landing page polish (waitlist, pricing preview) | 2-3 days | — |
| 28 | 159 | Sync Lab demo improvements (quick wins only) | 3-5 days | — |
| 29 | 155 | Show HN | 1 day | 153+156+159+160 |

#### DEFERRED (build when users ask)

*Enterprise features deferred per Firebase Killer pivot 2026-03-26. Trigger for re-evaluation: first 50 users request the feature, or single-server ceiling hit.*

| TODO | Feature | When to Build |
|------|---------|---------------|
| 025 | DAG Executor | When user hits single-server ceiling |
| 092 | Connector Framework | After DAG, when users request connectors |
| 176+179 | WASM Sandbox (Entry Processor + Conflict Resolvers) | When users need custom server-side logic |
| 072 | WASM Client Modules | When offline SQL/search demand validated |
| 101 | Client DevTools (full version) | After adoption reveals debugging pain |
| 048 | SSE Push | When serverless deployment requested |
| 175 | Distributed Locks | When multi-user coordination demand appears |
| 138 | Schema Migrations | When production users need schema evolution |
| 174 | Adaptive Indexing | When query pattern optimization needed |
| 178 | Interceptors / Middleware | When users need custom server-side hooks |
| 093 v2.0 | Admin Dashboard — Data Platform Features | After DAG+Connectors built |
| 036 | Pluggable Extension System | Post-adoption |
| 049 | Cluster-Aware HTTP Routing | → Milestone 3 (depends on cluster maturity) |
| 076 | MsgPack-Based Merkle Hashing | → Milestone 3 (deferred optimization) |
| 102 | Rust CLI (clap) | → Milestone 3 (post-adoption tooling) |
| 142 | Multi-Language SDKs (Python, Go) | → Milestone 3 (market expansion) |
| 165 | Security Hardening — Enterprise | → Milestone 3 (enterprise, BSL 1.1) |

### Milestone 3 — v3.0+ (Enterprise)

*Triggers: first 50 users request feature, or single-server ceiling hit, or $5K MRR.*

| Wave | Items | Blocked by |
|------|-------|------------|
| **7a** | TODO-041 (Multi-Tenancy) · TODO-043 (S3 Bottomless) · TODO-165 (Enterprise Security) | — · 033 · 164 |
| **7b** | TODO-040 (Tiered Storage) · TODO-039 (Vector Search) | 043 · — |
| **7c** | TODO-044 (Bi-Temporal) | 043 |
| **7d** | TODO-095 (Enterprise dir) · TODO-093 v3.0 (Dashboard) | — · 041+040 |
| **7e** | TODO-036 (Extensions) · TODO-102 (Rust CLI) · TODO-142 (Multi-language SDKs) | — |
| **7f** | TODO-174 (Adaptive Indexing) · TODO-178 (Interceptors) · TODO-049 (Cluster HTTP) · TODO-076 (Merkle hash opt) | — |

### Milestone 4 — GTM (Go-to-Market)

*Cloud launch after Phase 3 complete. Show HN is Phase 4. GTM business tasks (company reg, payments) start 4-6 weeks before cloud launch.*

| Wave | Items | Blocked by | Timing |
|------|-------|------------|--------|
| **8a** (pre-launch) | TODO-150 (Company reg) · TODO-166 (ToS/PP) | — | Start during Phase 3 |
| **8b** (launch) | TODO-151 (Paddle) · TODO-152 (Cloud: shared instance + Clerk) | 150+166 · Phase 3 done | After Phase 3 |
| **8c** (post-launch) | TODO-161 (Social strategy execution) · TODO-157 (Content pipeline) · TODO-158 (Premium license) | Revenue validation | After Show HN |

## Dependency Graph

```
MILESTONE 2: Firebase Killer (v2.0)

  Phase 0 (Validation):
  ┌─ TODO-200 (CLI Audit & Fix — verify 15 commands against Rust server)
  └─ TODO-194 (Getting Started + Docker demo + compose polish + 3 onboarding paths) ← TODO-200

  Phase 1 (Production Core):
  ┌─ TODO-193 (Lightweight metrics) ← no deps, 2-3 hours
  ├─ TODO-171 Phase A (Minimal RBAC) ← SPEC-137 ✓
  ├─ TODO-180 (Write Concern APPLIED + client SDK) ← no deps
  ├─ TODO-188 (Index Admin API) ← SPEC-155 ✓
  ├─ TODO-195 (SQL tab in QueryPlayground) ← SPEC-135 ✓
  ├─ TODO-201 (Client SDK sql() method) ← SPEC-135 ✓
  ├─ TODO-187 (SetupWizard backend + sync CLI setup.js) ← TODO-200
  └─ TODO-199 (Docs audit & accuracy pass + cli.mdx) ← TODO-171+180+188+200

  ── SOFT LAUNCH → r/rust, r/selfhosted, GUN.js Discord ──
  └─ TODO-191 (Verify 200K ops/sec benchmark)

  Phase 2 (User Experience):
  ┌─ TODO-196 (Planned Feature cleanup) ← no deps
  ├─ TODO-197 (Template apps: todo, chat, e-commerce) ← TODO-194
  ├─ TODO-198 (Firebase migration guide) ← TODO-171
  └─ Admin Dashboard polish (+ component tests for key interactions) ← soft launch feedback

  Phase 3 (Cloud Readiness):
  ┌─ TODO-136 (Rate Limiting) ← no deps
  ├─ TODO-033a (LRU Evictor + PERSISTED) ← no deps
  ├─ TODO-171 Phase A-ext (RBAC hot-reload + wildcard) ← TODO-171 Phase A
  │  └─ TODO-190 (RBAC sim test) ← TODO-171 Phase A-ext
  ├─ TODO-137 (Prometheus/OTel) ← no deps
  ├─ TODO-164 (Security Hardening) ← SPEC-137 ✓
  ├─ TODO-141 (Docker prod configs) ← no deps
  │  └─ TODO-192 (Docker CI/CD) ← TODO-141
  ├─ TODO-139 (Backup/Restore) ← no deps
  ├─ TODO-140 (Webhooks) ← no deps
  │  └─ TODO-189 (DNS rebinding) ← TODO-140 + TODO-164
  └─ Namespace isolation ← TODO-136

  Phase 4 (Show HN Launch):
  ┌─ TODO-156 (Community) ──→ TODO-160 (README) ──┐
  ├─ TODO-153 (Landing page)  ─────────────────────┼→ TODO-155 (Show HN)
  └─ TODO-159 (Demo improvements) ─────────────────┘

  Completed:
  ✓ TODO-069 (Schema) · ✓ TODO-091 (DataFusion SQL) · ✓ TODO-177 (Indexing)
  ✓ SPEC-126 (Tantivy) · ✓ SPEC-131 (Search fix) · ✓ SPEC-127-130 (Schema)
  ✓ SPEC-135a-c (DataFusion) · ✓ SPEC-142-145 (Query unification)
  ✓ SPEC-137 (P0 Security) · ✓ SPEC-138 (RS256) · ✓ SPEC-149-155 (Docs+Indexing)
  ✓ TODO-027 (DST via SPEC-132a-d)

  Deferred (build when users ask → Milestone 3):
  TODO-025 (DAG) · TODO-092 (Connectors) · TODO-176+179 (WASM Sandbox)
  TODO-072 (WASM Client) · TODO-101 (DevTools) · TODO-048 (SSE Push)
  TODO-175 (Dist. Locks) · TODO-138 (Schema Migrations)
  TODO-174 (Adaptive Index) · TODO-178 (Interceptors) · TODO-093 v2.0 (Dashboard++)
  TODO-036 (Extensions) · TODO-049 (Cluster HTTP) · TODO-076 (Merkle hash opt)
  TODO-102 (Rust CLI) · TODO-142 (Multi-lang SDKs) · TODO-165 (Enterprise Security)

MILESTONE 3: Enterprise (v3.0+)

  TODO-041 (Multi-Tenancy) ← triggers Cloud Phase B (~100+ customers)
  TODO-043 (S3 Bottomless) ──→ TODO-040 (Tiered) ──→ TODO-044 (Time-Travel)
       ↑
  TODO-033 (Full Write-Behind, deferred from v2.0)
  TODO-039 (Vector Search)
  TODO-095 (Enterprise dir)
  TODO-165 (Enterprise Security) ← KMS, JWKS, audit logging
  TODO-093 v3.0 (Dashboard) ← depends on 041+040
  TODO-036 (Extensions) · TODO-102 (Rust CLI) · TODO-142 (SDKs)
  TODO-174 (Adaptive Indexing) · TODO-178 (Interceptors) · TODO-049 (HTTP) · TODO-076 (Hash opt)

MILESTONE 4: GTM

  TODO-150 (Company) → TODO-151 (Paddle) → TODO-152 (Cloud) ← needs Phase 3 done
  TODO-166 (ToS/PP) ← blocker for payments
  TODO-161 (Social) · TODO-157 (Content) · TODO-158 (Premium) ← post Show HN
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
