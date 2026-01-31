# SpecFlow State

## Current Position

- **Active Specification:** none
- **Status:** idle
- **Next Step:** /sf:new or /sf:next

## Queue

| # | ID | Title | Priority | Status | Depends On |
|---|-------|----------|--------|--------|------------|

## Decisions

| Date | Specification | Decision |
|------|---------------|----------|
| 2026-01-31 | SPEC-012 | COMPLETED: React Hooks Test Suite. 3 test files created (useConflictResolver, useEntryProcessor, useMergeRejections). 42 test cases total. All 182 tests pass. Archived to .specflow/archive/SPEC-012.md |
| 2026-01-31 | SPEC-013 | COMPLETED: Silent Error Handling Audit. Added debug logging to client-message-handler.ts:43. 1 commit. Archived to .specflow/archive/SPEC-013.md |
| 2026-01-31 | SPEC-014 | COMPLETED: Skipped test removed from ClientFailover.test.ts. Codebase now has zero test.skip() or test.only() patterns. Archived to .specflow/archive/SPEC-014.md |
| 2026-01-31 | SPEC-015 | COMPLETED: Schema File Splitting. Split schemas.ts (1160 lines) into 6 domain modules + barrel. All 53 message types preserved in MessageSchema union. Build passes. Archived to .specflow/archive/SPEC-015.md |

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

## Warnings

(none)

---
*Last updated: 2026-01-31 23:50*
