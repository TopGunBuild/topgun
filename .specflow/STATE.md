# SpecFlow State

## Current Position

- **Active Specification:** none
- **Status:** idle
- **TODO Items:** 14
- **Next Step:** /sf:new or /sf:next

## Queue

| Position | Spec | Title | Status |
|----------|------|-------|--------|

## Decisions

| Date | Specification | Decision |
|------|---------------|----------|
| 2026-02-07 | SPEC-037 | COMPLETED: Document HTTP Sync Protocol and Serverless Deployment. Modified 4 files (sync-protocol.mdx, server.mdx, client.mdx, deployment.mdx). 6 commits, 4 audit cycles, 2 review cycles. Archived to .specflow/archive/SPEC-037.md |
| 2026-02-07 | SPEC-037 | REVIEWED v2: APPROVED. All 3 fixes from Review v1 verified as correctly applied. All 14 acceptance criteria satisfied, all constraints respected, no new issues found. Ready for finalization. |
| 2026-02-07 | SPEC-037 | FIXED v1: Applied all 3 review issues. (1) Major: corrected HttpSyncError field types -- code is number not string, context is string not object. (2) Minor: added ? optionality markers to 5 optional HttpSyncResponse fields. (3) Minor: updated HttpSyncHandler constructor in all 3 serverless examples to use HttpSyncHandlerConfig with 7 dependencies. 2 commits. Ready for re-review. |
| 2026-02-07 | SPEC-037 | REVIEWED v1: CHANGES_REQUESTED. 1 major issue: HttpSyncError field types in server.mdx describe code as (string) and context as (object) but actual Zod schema has code: z.number() and context: z.string().optional() -- AC9 violation. 2 minor issues. 13/14 acceptance criteria pass. |
| 2026-02-07 | SPEC-037 | EXECUTED: Document HTTP Sync Protocol and Serverless Deployment. Modified 4 files. 4 commits across 2 waves. All 14 acceptance criteria met. No deviations. |
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

## Warnings

(none)

---
*Last updated: 2026-02-07 (SPEC-037 completed and archived)*
