# SpecFlow State

## Current Position

- **Active Specification:** none
- **Status:** idle
- **Project Phase:** Phase 2.5 complete -> Phase 3 (Rust Server)
- **TODO Items:** 28 (1 client bug fix + 8 Rust bridge/core + 5 audit findings + 14 existing deferred)
- **Next Step:** /sf:new or /sf:next
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
| 2026-02-21 | SPEC-057c | COMPLETED: Networking Middleware, NetworkModule, and Integration. 2 files created, 2 modified, 59 tests, 3 commits. 2 audit cycles, 2 review cycles. |
| 2026-02-21 | SPEC-057c | REVIEW v2: APPROVED. All 3 Review v1 fixes verified. No new issues. 59/59 tests pass. Build and clippy clean. |
| 2026-02-21 | SPEC-057c | FIX v1: Applied all 3 minor review findings (router dedup, clone removal, serve completion test). 59/59 tests pass. |
| 2026-02-21 | SPEC-057c | REVIEW v1: APPROVED. All 6 acceptance criteria met. 59/59 tests pass. Build and clippy clean. 3 minor findings (router duplication, avoidable clone, shutdown test completeness). |
| 2026-02-21 | SPEC-057b | COMPLETED: Networking HTTP and WebSocket Handlers. 4 files created, 3 modified, 12 tests, 4 commits. |

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
- VM sandbox pattern: ProcessorSandbox fallback uses vm.Script + runInNewContext({ timeout }) for synchronous code interruption; isolated-vm primary path unchanged
- IConnection adapter pattern: IConnectionProvider returns IConnection interface (send/close/readyState) instead of concrete WebSocket; WebSocketConnection wraps WS, HttpConnection wraps HTTP queue; ConnectionReadyState constants avoid WebSocket global dependency
- Partition pruning pattern: PartitionService.getRelevantPartitions extracts key predicates (_key, key, id, _id) from queries to prune distributed fan-out; targetedNodes on DistributedSubscription keeps checkAcksComplete consistent with pruned node sets
- Node ID reconciliation pattern: ConnectionPool.addNode() matches incoming server-assigned nodeId against existing seed connections by endpoint; remapNodeId() transfers NodeConnection entry preserving WebSocket/state/pending; node:remapped event notifies ClusterClient and PartitionRouter
- Batch delegation pattern: IConnectionProvider.sendBatch is optional; SyncEngine delegates to it when available (cluster mode) for per-key partition routing; falls back to single OP_BATCH in single-server mode
- Double-option deserialization pattern: For Rust fields matching TS `.nullable().optional()`, use `Option<Option<T>>` with custom `deserialize_double_option` to distinguish absent (None) from explicitly-null (Some(None))
- Float64 numeric interop pattern: JavaScript's msgpackr encodes numbers > 2^32 as float64; use `serde_number::deserialize_u64` (from `hlc::serde_number`) on Rust u64/i64 fields to accept both integer and float64 MsgPack wire values

## Warnings

(none)

---
*Last updated: 2026-02-21 (SPEC-057c COMPLETED)*
