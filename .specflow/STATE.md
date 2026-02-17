# SpecFlow State

## Current Position

- **Active Specification:** none
- **Status:** idle
- **Project Phase:** Phase 2 (Rust Core)
- **TODO Items:** 29 (1 client bug fix + 9 Rust bridge/core + 5 audit findings + 14 existing deferred)
- **Next Step:** `/sf:new` or `/sf:next`
- **Roadmap:** See [TODO.md](todos/TODO.md) for full phase-based roadmap

## Queue

| Position | Spec | Title | Status | Phase |
|----------|------|-------|--------|-------|
| 1 | SPEC-052b | Message Schema -- Sync and Query Domain Structs | review | Phase 2 |
| 2 | SPEC-052c | Message Schema -- Search and Cluster Domain Structs | ready | Phase 2 |
| 3 | SPEC-052d | Message Schema -- Messaging and Client Events Domain Structs | ready | Phase 2 |
| 4 | SPEC-052e | Message Schema -- HTTP Sync, Message Union, and Cross-Language Tests | blocked (052c, 052d) | Phase 2 |

## Migration Roadmap (high-level)

| Phase | Description | Status |
|-------|-------------|--------|
| **0. TypeScript Completion** | SPEC-048a/b/c (client cluster integration) | Complete |
| **1. Bridge** | Cargo workspace, CI, 6 upfront traits | Complete |
| **2. Rust Core** | CRDTs, message schemas, partitions | In Progress |
| **3. Rust Server** | Network, handlers, cluster, storage, tests | Not Started |
| **4. Rust Features** | Schema system, shapes, SSE, DAG, tantivy | Not Started |
| **5. Post-Migration** | AsyncStorage, S3, tiered, vector, extensions | Not Started |

See [TODO.md](todos/TODO.md) for detailed task breakdown with dependencies.

## Key Strategic Documents

| Document | Purpose |
|----------|---------|
| [TODO.md](todos/TODO.md) | Phase-based roadmap with all tasks and dependencies |
| [RUST_SERVER_MIGRATION_RESEARCH.md](reference/RUST_SERVER_MIGRATION_RESEARCH.md) | Technical migration strategy, 6 upfront traits |
| [PRODUCT_POSITIONING_RESEARCH.md](reference/PRODUCT_POSITIONING_RESEARCH.md) | Competitive analysis, schema strategy, partial replication |

## Decisions

| Date | Specification | Decision |
|------|---------------|----------|
| 2026-02-17 | SPEC-054 | COMPLETED: Message Schema Architecture Fix-on-Port. 4 commits, 4 files modified. 2 audit cycles, 1 review cycle. All 6 AC pass. 246 tests. Unblocks SPEC-052c/d/e. |
| 2026-02-17 | SPEC-054 | REVIEWED v1: APPROVED. All 6 acceptance criteria verified against source code. 246 tests pass, zero clippy warnings. No critical, major, or minor issues found. |
| 2026-02-17 | SPEC-054 | EXECUTED: 4 commits, 246 tests passing, all 6 acceptance criteria met. Removed 23 r#type fields, replaced 14 f64 with integer types, added Default to 5 structs, prototyped tagged enum. |
| 2026-02-17 | SPEC-054 | AUDITED v2: APPROVED with no new issues. Fresh re-audit confirms all Audit v1 recommendations applied. Source code verified: 23 r#type fields, 14 f64 fields, 232 tests. All 10 audit dimensions pass. Ready for /sf:run --parallel. |
| 2026-02-16 | SPEC-054 | REVISED v1: Applied all 3 Audit v1 recommendations. Removed OpRejectedPayload from Default list (only 1 optional field), added ClientOp doc comment note, verified SyncInitMessage already removed. Ready for re-audit. |

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

## Warnings

(none)

---
*Last updated: 2026-02-17 (SPEC-054 completed and archived)*
