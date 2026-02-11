# SpecFlow State

## Current Position

- **Active Specification:** none
- **Status:** idle
- **TODO Items:** 15
- **Next Step:** `/sf:new` or `/sf:next`

## Queue

| Position | Spec | Title | Status |
|----------|------|-------|--------|

## Decisions

| Date | Specification | Decision |
|------|---------------|----------|
| 2026-02-11 | SPEC-047 | COMPLETED: Partition Pruning for Distributed Queries. Created 1 file, modified 7 files. 10 commits, 3 audit cycles, 3 review cycles. Archived to .specflow/archive/SPEC-047.md |
| 2026-02-11 | SPEC-047 | Review v3: APPROVED. Critical fix from Review v2 (restoring targetNodes.add(myNodeId)) verified correct. All 12 acceptance criteria met. All 24 new tests pass. All existing tests pass (1 pre-existing flaky EntryProcessor test unrelated). No critical or major issues. |
| 2026-02-10 | SPEC-047 | Audit v3: APPROVED with 2 recommendations. All 10 dimensions passed. All line numbers and code references verified against source. Assumptions are low-risk and conservative. Recommend /sf:run --parallel due to ~56% total context with 3-wave execution plan. |
| 2026-02-10 | SPEC-046 | COMPLETED: Replace WebSocket Return Type in IConnectionProvider with IConnection Interface. Created 1 file, modified 10 files. 9 commits, 2 audit cycles, 1 review cycle. Archived to .specflow/archive/SPEC-046.md |
| 2026-02-10 | SPEC-045 | COMPLETED: Fix ProcessorSandbox Test Hang and Update Documentation Server Instantiation. Modified 13 files, 3 commits, 4 audit cycles, 1 review cycle. Archived to .specflow/archive/SPEC-045.md |

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

## Warnings

(none)

---
*Last updated: 2026-02-11 (SPEC-047 completed and archived)*
