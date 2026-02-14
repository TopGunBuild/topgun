# SpecFlow State

## Current Position

- **Active Specification:** SPEC-052
- **Status:** drafting
- **Project Phase:** Phase 2 (Rust Core)
- **TODO Items:** 23 (1 client bug fix + 8 Rust bridge/core + 14 existing deferred)
- **Next Step:** /sf:audit
- **Roadmap:** See [TODO.md](todos/TODO.md) for full phase-based roadmap

## Queue

| Position | Spec | Title | Status | Phase |
|----------|------|-------|--------|-------|
| 1 | SPEC-052 | Message Schema Compatibility -- Rust Serde Structs for MsgPack Wire Protocol | draft | Phase 2 |

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
| 2026-02-14 | SPEC-052 | DRAFTED: Message Schema Compatibility -- Rust serde structs for all 59+ message types. Complexity: large. Needs /sf:split (13 files, max 5 per sub-spec). |
| 2026-02-14 | SPEC-051c | COMPLETED: ORMap Implementation and CrdtMap Wrapper. 3 commits, 1 file created, 4 modified. 173 tests. 3 audit cycles, 2 review cycles. All 7 AC pass. |
| 2026-02-14 | SPEC-051c | REVIEW v2: APPROVED. Post-fix verification confirms issue #2 fix applied correctly. All 173 tests pass. No regressions. |
| 2026-02-14 | SPEC-051c | FIX v1: Applied issue #2 (unwrap_or_default → .expect()). Skipped #1, #3 as positive deviations. |
| 2026-02-14 | SPEC-051c | REVIEW v1: APPROVED. All 7 ACs verified independently. 3 minor issues (positive signature deviation, silent unwrap_or_default, HLC update scope divergence). None block approval. |
| 2026-02-14 | SPEC-051c | EXECUTED: Implementation complete. 2 commits, 173 tests pass, all 7 ACs met. |

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

## Warnings

(none)

---
*Last updated: 2026-02-14 (SPEC-052 drafted)*
