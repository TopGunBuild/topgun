# SpecFlow State

## Current Position

- **Active Specification:** none
- **Status:** idle
- **TODO Items:** 15
- **Next Step:** /sf:new or /sf:next

## Queue

| Position | Spec | Title | Status |
|----------|------|-------|--------|
| 1 | SPEC-048b | Routing Logic and Error Recovery | draft |
| 2 | SPEC-048c | End-to-End Cluster Integration Test | draft |

## Decisions

| Date | Specification | Decision |
|------|---------------|----------|
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

## Warnings

(none)

---
*Last updated: 2026-02-11 (SPEC-048a completed and archived)*
