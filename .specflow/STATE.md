# SpecFlow State

## Current Position

- **Active Specification:** SPEC-048c — End-to-End Cluster Integration Test
- **Status:** review
- **Project Phase:** Phase 0 (TypeScript Completion) — 1 spec remaining
- **TODO Items:** 23 (1 TS spec + 8 new Rust bridge/core + 14 existing deferred)
- **Next Step:** /sf:review
- **Roadmap:** See [TODO.md](todos/TODO.md) for full phase-based roadmap

## Queue

| Position | Spec | Title | Status | Phase |
|----------|------|-------|--------|-------|
| 1 | SPEC-048c | End-to-End Cluster Integration Test | review | Phase 0 (TS) |

## Migration Roadmap (high-level)

| Phase | Description | Status |
|-------|-------------|--------|
| **0. TypeScript Completion** | SPEC-048c (client cluster E2E test) | In Progress |
| **1. Bridge** | Cargo workspace, CI, 6 upfront traits | Not Started |
| **2. Rust Core** | CRDTs, message schemas, partitions | Not Started |
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
| 2026-02-12 | SPEC-048c | EXECUTED: 1 commit, 1 file created. Test passes (4.5s). All 501 client tests + 1211 server tests pass. 4 Rule 2 deviations (inlined pollUntil, patched updateConnectionPool, key ownership selection, dual auth). 3 pre-existing bugs documented. |
| 2026-02-12 | SPEC-048c | Audit v3: APPROVED with no issues. Fresh-eyes source verification confirmed all API references, patterns, and assumptions. Spec is implementation-ready. |
| 2026-02-12 | SPEC-048c | Response v2: Applied all 3 Audit v2 recommendations (verified no duplicate prerequisites, added pollUntil import path, clarified LWWMap.get() assertion pattern). |
| 2026-02-12 | SPEC-048c | Audit v2: APPROVED with 3 recommendations (duplicate prerequisites heading, pollUntil import path, LWWMap.get() return shape). All v1 critical issues resolved. |
| 2026-02-12 | SPEC-048c | Response v1: Applied all 6 items (1 critical + 5 recommendations). Rewrote AC-5 to use public TopGunClient API, added auth/storage/WS polyfill prerequisites, changed to port:0, specified server-side inspection. |

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
*Last updated: 2026-02-12 (SPEC-048c executed, decisions rotated)*
