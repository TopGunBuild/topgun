# SpecFlow State

## Current Position

- **Active Specification:** none
- **Status:** idle
- **TODO Items:** 15
- **Next Step:** `/sf:new` or `/sf:next`

## Queue

| Position | Spec | Title | Status |
|----------|------|-------|--------|
| (empty) | | | |

## Decisions

| Date | Specification | Decision |
|------|---------------|----------|
| 2026-02-10 | SPEC-045 | COMPLETED: Fix ProcessorSandbox Test Hang and Update Documentation Server Instantiation. Modified 13 files, 3 commits, 4 audit cycles, 1 review cycle. Archived to .specflow/archive/SPEC-045.md |
| 2026-02-09 | SPEC-044 | COMPLETED: Fix Flaky Split-Brain Recovery Test in Resilience.test.ts. Modified 1 file, 1 commit, 2 audit cycles, 1 review cycle. Archived to .specflow/archive/SPEC-044.md |
| 2026-02-09 | SPEC-042 | COMPLETED: Replace Fixed setTimeout Delays with Bounded Polling in Server Tests. Modified 21 files, 8 commits, 2 audit cycles, 2 review cycles. Archived to .specflow/archive/SPEC-042.md |
| 2026-02-09 | SPEC-043 | COMPLETED: Fix SearchCoordinator Batched LEAVE Notification Bug. Modified 1 file, 1 commit, 1 audit cycle, 1 review cycle. Archived to .specflow/archive/SPEC-043.md |
| 2026-02-08 | SPEC-041 | COMPLETED: Fix DistributedSearch and GC Broadcast Test Failures. Modified 1 file, 1 commit, 2 audit cycles, 1 review cycle. Archived to .specflow/archive/SPEC-041.md |
| 2026-02-08 | SPEC-040 | COMPLETED: Fix Interceptor Integration and TLS Test Failures After Modular Refactoring. Modified 3 files, 3 commits, 1 audit cycle, 1 review cycle. Archived to .specflow/archive/SPEC-040.md |
| 2026-02-08 | SPEC-039 | COMPLETED: Add Reject Path to network.start() Promise. Modified 3 files, 1 commit, 1 audit cycle, 1 review cycle. Archived to .specflow/archive/SPEC-039.md |
| 2026-02-08 | SPEC-038 | COMPLETED: Fix WebSocket Client Auth Handshake After ServerFactory Modular Refactoring. Modified 4 files, 1 commit, 2 audit cycles, 1 review cycle. Archived to .specflow/archive/SPEC-038.md |
| 2026-02-07 | SPEC-037 | COMPLETED: Document HTTP Sync Protocol and Serverless Deployment. Modified 4 files (sync-protocol.mdx, server.mdx, client.mdx, deployment.mdx). 6 commits, 4 audit cycles, 2 review cycles. Archived to .specflow/archive/SPEC-037.md |
| 2026-02-06 | SPEC-036 | COMPLETED: HTTP Sync Protocol for Serverless Environments. Created 9 files, modified 7 files. 9 commits, 3 audit cycles, 2 review cycles. Archived to .specflow/archive/SPEC-036.md |

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

## Warnings

(none)

---
*Last updated: 2026-02-10 (SPEC-045 completed and archived)*
