# SpecFlow State

## Current Position

- **Active Specification:** none
- **Status:** idle
- **Project Phase:** Phase 3 (Rust Server) — Wave 4
- **TODO Items:** 28 (1 client bug fix + 8 Rust bridge/core + 5 audit findings + 14 existing deferred)
- **Next Step:** `/sf:new` or `/sf:next`
- **Roadmap:** See [TODO.md](todos/TODO.md) for full phase-based roadmap

## Queue

| Position | Spec | Title | Status | Phase |
|----------|------|-------|--------|-------|

## Migration Roadmap (high-level)

| Phase | Description | Status |
|-------|-------------|--------|
| **0. TypeScript Completion** | SPEC-048a/b/c (client cluster integration) | Complete |
| **1. Bridge** | Cargo workspace, CI, 6 upfront traits | Complete |
| **2. Rust Core** | CRDTs, message schemas, partitions | Complete |
| **2.5 Research Sprint** | Storage, Cluster, Service, Networking architecture | **Complete** |
| **3. Rust Server** | Network, handlers, cluster, storage, tests | In Progress |
| **4. Rust Features** | Schema system, shapes, SSE, DAG, tantivy | Not Started |
| **5. Post-Migration** | AsyncStorage, S3, tiered, vector, extensions | Not Started |

See [TODO.md](todos/TODO.md) for detailed task breakdown with dependencies.

## Key Strategic Documents

| Document | Purpose |
|----------|---------|
| [TODO.md](todos/TODO.md) | Phase-based roadmap with all tasks and dependencies |
| [RUST_SERVER_MIGRATION_RESEARCH.md](reference/RUST_SERVER_MIGRATION_RESEARCH.md) | Technical migration strategy, 6 upfront traits |
| [PRODUCT_POSITIONING_RESEARCH.md](reference/PRODUCT_POSITIONING_RESEARCH.md) | Competitive analysis, schema strategy, partial replication |
| [RUST_STORAGE_ARCHITECTURE.md](reference/RUST_STORAGE_ARCHITECTURE.md) | Multi-layer storage design (TODO-080) |
| [RUST_CLUSTER_ARCHITECTURE.md](reference/RUST_CLUSTER_ARCHITECTURE.md) | Cluster protocol design (TODO-081) |
| [RUST_SERVICE_ARCHITECTURE.md](reference/RUST_SERVICE_ARCHITECTURE.md) | Service & operation routing design (TODO-082) |
| [RUST_NETWORKING_PATTERNS.md](reference/RUST_NETWORKING_PATTERNS.md) | Networking layer patterns (TODO-083) |

## Decisions

| Date | Specification | Decision |
|------|---------------|----------|
| 2026-03-04 | SPEC-076c | COMPLETED: React admin dashboard adaptation — admin-api-types.ts (13 hand-typed interfaces), swr-config.ts (SWR global config), CrdtDebug.tsx (placeholder UI). SWR replaces manual polling in ClusterTopology/Settings/useServerStatus. Settings restructured from 5-tab nested to 2-tab flat layout. Token key unified to topgun_admin_token. Setup Wizard hidden. 3 files created, 11 modified, 4 commits, 2 audit cycles, 1 review cycle. All 13 ACs met. APPROVED by impl-reviewer v1. |
| 2026-03-04 | SPEC-076b | COMPLETED: Admin handlers, OpenAPI, and router wiring -- admin.rs (6 handlers: status, login, cluster/status, maps, settings GET/PUT), openapi.rs (utoipa OpenApi derive + Swagger UI), module.rs (ServeDir SPA fallback with ServeFile for index.html, NetworkModule setter methods), handlers/mod.rs (AppState extended with cluster_state/store_factory/server_config), network/mod.rs (pub mod openapi), factory.rs (map_names()). 2 files created, 4 modified. All 12 ACs met. 502 tests pass, clippy clean. APPROVED by impl-reviewer v2. |
| 2026-03-04 | SPEC-076a | COMPLETED: Admin types & auth middleware — admin_types.rs (13 types with ToSchema), admin_auth.rs (AdminClaims extractor with 401/403), JwtClaims made pub with roles, handle_auth propagates roles to Principal. 2 files created, 3 modified, 2 commits, 1 audit cycle, 1 fix cycle. All 10 ACs met. 502 tests pass, clippy clean. APPROVED by impl-reviewer v2. |
| 2026-03-04 | SPEC-076 | SPLIT into 3 parts: SPEC-076a (Rust admin types + auth middleware, 4 files), SPEC-076b (Rust admin handlers + wiring, 4 files), SPEC-076c (React dashboard adaptation, 14 TS/TSX files). Parent archived. |
| 2026-03-03 | SPEC-075 | COMPLETED: Wire QueryObserverFactory for Live Query Updates — QueryObserverFactory struct in test_server.rs sharing Arc<QueryRegistry> and Arc<ConnectionRegistry> with QueryService, registered alongside SearchObserverFactory in RecordStoreFactory::with_observer_factories(). 1 file modified, 1 commit, 2 audit cycles. All 12 ACs met. Integration tests: 50/50 passing (up from 44/50). |


## Project Patterns

- Monorepo with package hierarchy: core -> client/server -> adapters/react
- TypeScript with strict mode
- Commit format: `type(scope): description`
- CRDTs use Hybrid Logical Clocks for causality tracking
- Handler extraction pattern: separate message handlers into focused modules with config injection
- Test polling pattern: use centralized test-helpers.ts with PollOptions for bounded iterations
- Late binding pattern: handlers can receive callbacks after construction via setXxxCallbacks methods
- Test harness pattern: ServerTestHarness provides controlled access to internal handlers for tests
- Timer cleanup pattern: handlers with timers implement stop() method, called by LifecycleManager during shutdown
- Message routing pattern: MessageRouter provides declarative type-based routing for server messages
- Module factory pattern: each domain gets its own factory function with explicit dependency injection
- Deferred startup pattern: module factories create resources but do not bind ports; start() method called after assembly
- Domain grouping pattern: handlers grouped by domain (CRDT, Sync, Query, Messaging, etc.) for Actor Model portability
- Client message handler pattern: ClientMessageHandlers module registers all client-side message types via registerClientMessageHandlers()
- React hook testing pattern: use renderHook + act from @testing-library/react with mock client wrapped in TopGunProvider
- Schema domain splitting pattern: organize schemas by domain (base, sync, query, search, cluster, messaging) with barrel re-exports
- DST infrastructure pattern: VirtualClock/SeededRNG/VirtualNetwork for deterministic simulation testing; injectable ClockSource via HLC for reproducible time
- HTTP sync transport pattern: HttpSyncProvider implements IConnectionProvider via message type routing in send(); AutoConnectionProvider provides WS-to-HTTP fallback; server uses setHttpRequestHandler() deferred wiring for POST /sync
- VM sandbox pattern: ProcessorSandbox fallback uses vm.Script + runInNewContext({ timeout }) for synchronous code retrieval; isolated-vm primary path unchanged
- IConnection adapter pattern: IConnectionProvider returns IConnection interface (send/close/readyState) instead of concrete WebSocket; WebSocketConnection wraps WS, HttpConnection wraps HTTP queue; ConnectionReadyState constants avoid WebSocket global dependency
- Partition pruning pattern: PartitionService.getRelevantPartitions extracts key predicates (_key, key, id, _id) from queries to prune distributed fan-out; targetedNodes on DistributedSubscription keeps checkAcksComplete consistent with pruned node sets
- Node ID reconciliation pattern: ConnectionPool.addNode() matches incoming server-assigned nodeId against existing seed connections by endpoint; remapNodeId() transfers NodeConnection entry preserving WebSocket/state/pending; node:remapped event notifies ClusterClient and PartitionRouter
- Batch delegation pattern: IConnectionProvider.sendBatch is optional; SyncEngine delegates to it when available (cluster mode) for per-key partition routing; falls back to single OP_BATCH in single-server mode
- Double-option deserialization pattern: For Rust fields matching TS `.nullable().optional()`, use `Option<Option<T>>` with custom `deserialize_double_option` to distinguish absent (None) from explicitly-null (Some(None))
- Float64 numeric interop pattern: JavaScript's msgpackr encodes numbers > 2^32 as float64; use `serde_number::deserialize_u64` (from `hlc::serde_number`) on Rust u64/i64 fields to accept both integer and float64 MsgPack wire values
- Operation routing pattern: ServiceRegistry with ManagedService trait for lifecycle; Operation enum classifies all client-to-server Messages; OperationRouter dispatches by service_name; Tower middleware pipeline (LoadShed -> Timeout -> Metrics) wraps routing; domain_stub! macro generates stub services
- Migration trait seam pattern: MapProvider trait abstracts storage access for partition migration; NoOpMapProvider (pub(crate)) enables unit testing without full storage layer; concrete wiring deferred to storage module spec
- Domain service replacement pattern: replace `domain_stub!` macro services one-at-a-time with real `tower::Service<Operation>` implementations; each real service takes `Arc` dependencies via constructor; heartbeat side-effects use `OperationContext.connection_id` for `ConnectionRegistry` lookup
- Write validation pattern: `WriteValidator` intercepts client writes BEFORE CRDT merge; metadata snapshot pattern acquires `RwLock` read guard then immediately clones to avoid holding locks across async ops; validate-all-then-apply-all for atomic batch rejection
- Observability pattern: `init_observability()` with `OnceLock` idempotency guard installs `tracing-subscriber` (EnvFilter + optional JSON) and `metrics-exporter-prometheus` recorder; `ObservabilityHandle` threaded via `AppState`; `/metrics` endpoint renders Prometheus text format on scrape; `metrics::counter!/histogram!/gauge!` macros in Tower middleware; manual `info_span!` + `.instrument()` on domain service `call()` methods
- WebSocket dispatch pattern: Two-phase auth (pre-split AUTH_REQUIRED on raw socket, post-split AUTH_ACK/FAIL via mpsc channel); inbound binary -> `rmp_serde::from_slice` -> auth gate -> `OperationService.classify()` -> `set_connection_id()` -> `OperationPipeline` (BoxService) dispatch; BATCH unpacking via 4-byte BE u32 length-prefixed inner messages; `OperationResponse` variant mapping (Message/Messages/Empty/Ack/NotImplemented); `Option<Arc<...>>` AppState fields with None defaults for backward-compatible test compilation
- Rust integration test harness pattern: `spawnRustServer()` spawns test binary (RUST_SERVER_BINARY or cargo run), captures PORT= from stdout via readline, returns cleanup (process-group SIGTERM/SIGKILL); standalone `TestClient` in test-client.ts (no @topgunbuild/server dep) sends individual MsgPack frames (no BATCH)
- Observer factory pattern: `ObserverFactory` trait enables per-map mutation observer creation at `RecordStore` creation time; `RecordStoreFactory.with_observer_factories()` builder wires factories; `get_or_create()` calls each factory on cache miss and merges returned observers into `CompositeMutationObserver`; `get_all_for_map()` returns all cached stores across partitions for cross-partition aggregation

## Execution Status

| Spec | Mode | Progress | Last Updated |
|------|------|----------|--------------|

## Warnings

(none)

---
*Last updated: 2026-03-04 (SPEC-076c archived)*
