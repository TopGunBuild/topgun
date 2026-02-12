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
| 2026-02-12 | SPEC-048c | Audit v1: NEEDS_REVISION. 1 critical issue (getRoutingMetrics() not exposed on TopGunClient), 5 recommendations (auth details, storage adapter, WS polyfill, port convention, read-back ambiguity). |
| 2026-02-12 | SPEC-048b | COMPLETED: Routing Logic — Batch Delegation and Reconnect Map Refresh. Modified 3 files, 3 commits, 3 audit cycles, 2 review cycles. Archived to .specflow/archive/SPEC-048b.md |
| 2026-02-12 | SPEC-048b | Review v2: APPROVED with no issues. Fix Response v1 verified (sendBatch return value handling + 3 new unit tests). 500 tests pass. |
| 2026-02-12 | SPEC-048b | Fix Response v1: Applied 2 minor fixes (sendBatch return value logging, 3 new unit tests). 500 tests pass. |
| 2026-02-12 | SPEC-048b | Review v1: APPROVED with 2 minor issues. All 9 AC satisfied, all 5 constraints honored. Build succeeds, 497 tests pass. |
| 2026-02-12 | SPEC-048b | EXECUTED: 2 commits, 3 files modified, all 497 tests pass. G1 (reconnect map refresh) and G2 (sendBatch delegation) both complete. |
| 2026-02-12 | SPEC-048b | Audit v3: APPROVED with no issues. Fresh-eyes source code verification confirmed all claims. Spec is implementation-ready. |
| 2026-02-12 | SPEC-048b | Revision v2: Applied 3 audit v2 recommendations. Clarified sendBatch message field contents, annotated AC-1/AC-2 as verified by SPEC-048c, added connection provider access path to R1. |
| 2026-02-12 | SPEC-048b | Audit v2: APPROVED with 3 recommendations. Source code verified. Delegation approach via optional IConnectionProvider.sendBatch is clean and correct. Reconnect map refresh fix well-scoped. |
| 2026-02-12 | SPEC-048b | Revision v1: Removed R2 (NOT_OWNER handling) — server doesn't send these. Restructured R1 to delegate to ClusterClient.sendBatch() via optional IConnectionProvider.sendBatch. Reduced complexity to small. Ready for re-audit. |
| 2026-02-12 | SPEC-048b | Audit v1: NEEDS_REVISION. 3 critical issues: (1) Server does not actually send NOT_OWNER messages — R2 based on false premise, (2) NotOwnerError uses `code` not `type` field, (3) Significant duplication with existing ClusterClient.sendWithRetry/sendBatch/PartitionRouter.handleNotOwnerError. |
| 2026-02-12 | Roadmap | RESTRUCTURED: Replaced wave-based TODO.md with phase-based Rust migration roadmap. Added TODO-059 through TODO-072 for Rust-specific work. Product positioning decisions integrated (schema, shapes, WASM). |
| 2026-02-12 | Architecture | 6th upfront trait added: SchemaProvider (schema validation + partial replication shapes). TypeScript-first schema strategy decided. Selective WASM strategy decided. |
| 2026-02-11 | SPEC-048a | COMPLETED: ConnectionPool Foundation Fixes. Modified 1 file, 3 commits, 3 audit cycles, 1 review cycle. Archived to .specflow/archive/SPEC-048a.md |
| 2026-02-11 | SPEC-048a | Review v1: APPROVED. All 8 acceptance criteria verified. All 5 constraints honored. Build succeeds. 497 tests pass. +55/-21 lines in single file. No critical or major issues. |
| 2026-02-11 | SPEC-048a | Audit v3: APPROVED. All 10 dimensions passed. Source code verified: NodeConnection.endpoint exists for matching, addNode() receives both nodeId and endpoint, handleMessage return statements are only barrier to forwarding. No critical issues. No recommendations. |
| 2026-02-11 | SPEC-048 | SPLIT into 3 parts: SPEC-048a (ConnectionPool Foundation Fixes), SPEC-048b (Routing Logic and Error Recovery), SPEC-048c (End-to-End Cluster Integration Test). Parent archived to .specflow/archive/SPEC-048.md |
| 2026-02-11 | SPEC-047 | COMPLETED: Partition Pruning for Distributed Queries. Created 1 file, modified 7 files. 10 commits, 3 audit cycles, 3 review cycles. Archived to .specflow/archive/SPEC-047.md |

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
*Last updated: 2026-02-12 (SPEC-048c executed)*
