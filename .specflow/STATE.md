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
| 2026-02-28 | SPEC-068 | COMPLETED: SearchService (Tantivy Full-Text Search) — 7th and final domain service replacing domain_stub! macro, completing Phase 3 domain service layer. 1 file created, 4 modified, 4 commits, 3 audit cycles, 2 review cycles + 1 fix cycle. All 21 ACs implemented. TantivyMapIndex (RAM directory, fixed schema), SearchRegistry (DashMap), SearchMutationObserver (UnboundedSender + 16ms batching), SearchService (tower::Service + ManagedService). 467 total tests (0 failures), clippy clean. APPROVED by impl-reviewer v2. |
| 2026-02-27 | SPEC-067 | COMPLETED: PostgresDataStore (MapDataStore adapter) — first real persistence backend for Rust server. 1 file created, 2 modified, 3 commits, 1 audit cycle, 1 review cycle. All 16 ACs implemented. Write-through persistence via sqlx PgPool, BYTEA+MsgPack storage, feature-gated behind `postgres`. 468 total tests (22 new, 0 failures), clippy clean. APPROVED by impl-reviewer v1. |
| 2026-02-27 | SPEC-066 | COMPLETED: PersistenceService (Counters, Journal, Resolver/EntryProcess Stubs) — sixth real domain service replacing domain_stub! macro. 3 files created, 2 modified, 2 commits, 3 audit cycles, 1 review cycle. All 22 ACs implemented. CounterRegistry (counter.rs) + JournalStore (journal.rs) + PersistenceService (persistence.rs). 446 total tests (27 new, 0 failures), clippy clean. APPROVED by impl-reviewer v1. |
| 2026-02-26 | SPEC-065 | COMPLETED: QueryService (Live Query Subscriptions) — fifth real domain service replacing domain_stub! macro. 2 files created, 2 modified, 2 commits, 3 audit cycles, 2 review cycles + 1 fix cycle. All 16 ACs implemented. PredicateEngine (predicate.rs) + QueryRegistry/QueryMutationObserver/QueryService (query.rs). 60 new tests (419 total, 0 failures), clippy clean for spec files. APPROVED by impl-reviewer v2. |
| 2026-02-25 | SPEC-064 | COMPLETED: MessagingService (Topic Pub/Sub) — fourth real domain service replacing domain_stub! macro. 1 file created, 2 modified, 1 commit, 3 audit cycles, 1 review cycle. All 12 ACs implemented. 360 total tests (0 failures), clippy clean. APPROVED by impl-reviewer v1. |
| 2026-02-25 | SPEC-063 | COMPLETED: SyncService (Merkle Delta Sync Protocol) — third real domain service replacing domain_stub! macro. 2 files created, 4 modified, 5 commits, 3 audit cycles, 2 review cycles + 1 fix cycle. All 14 ACs implemented. 341 total tests (0 failures), clippy clean. APPROVED by impl-reviewer v2. |
| 2026-02-24 | SPEC-062 | COMPLETED: CrdtService (LWW-Map and OR-Map Operations) — second real domain service replacing domain_stub! macro. 1 file created, 3 modified, 3 commits, 2 audit cycles, 1 review cycle. All 9 ACs implemented (LWW PUT/REMOVE, OR_ADD/OR_REMOVE, OpBatch, WrongService, ManagedService name, integration test, empty batch). rmpv added as direct dependency for wire value conversion. Clippy clean. |
| 2026-02-24 | SPEC-061 | COMPLETED: CoordinationService (Ping/PartitionMap/Heartbeat) — first real domain service replacing domain_stub! macro. 1 file created, 4 modified, 4 commits, 4 audit cycles, 1 review cycle. 296 total tests (9 new), clippy clean for spec files. |

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

## Warnings

(none)

---
*Last updated: 2026-02-28 (SPEC-068 completed and archived)*
