# SpecFlow State

## Current Position

- **Active Specification:** none
- **Status:** idle
- **TODO Items:** 13
- **Next Step:** /sf:new or /sf:next

## Queue

| Position | Spec | Title | Status |
|----------|------|-------|--------|

## Decisions

| Date | Specification | Decision |
|------|---------------|----------|
| 2026-02-06 | SPEC-036 | COMPLETED: HTTP Sync Protocol for Serverless Environments. Created 9 files (http-sync-schemas, HttpSyncHandler, HttpSyncProvider, AutoConnectionProvider, 5 test files with 56 tests), modified 7 files (schema/index, coordinator/index, network-module, types, ServerFactory, connection/index, client/index). 9 commits, 3 audit cycles, 2 review cycles. Archived to .specflow/archive/SPEC-036.md |
| 2026-02-06 | SPEC-036 | REVIEWED v2: Implementation APPROVED. All 3 fixes from Review v1 verified as correctly applied (requestTimeoutMs default 30s, TLS status message preserved, WS success test added). All 13 acceptance criteria fully satisfied. All 11 constraints respected. 56 tests pass across 5 test files (12+12+8+17+7). Build succeeds. No new issues found. Ready for finalization. |
| 2026-02-06 | SPEC-036 | FIXED v1: Applied all 3 minor review issues. (1) Changed requestTimeoutMs default from 10s to 30s to match spec. (2) Preserved TLS status message in ServerFactory HTTP handler. (3) Added WebSocket success path test to AutoConnectionProvider (7 tests, up from 6). 3 commits. Ready for re-review. |
| 2026-02-06 | SPEC-036 | REVIEWED v1: Implementation APPROVED. All 13 acceptance criteria fully satisfied. All 10 constraints respected. 55 tests pass across 5 test files (12+12+8+17+6). Build succeeds. 3 minor issues found (requestTimeoutMs default, TLS status message, missing WS success test). No critical or major issues. Ready for finalization. |
| 2026-02-06 | SPEC-036 | EXECUTED: HTTP Sync Protocol for Serverless Environments. Created 9 files (http-sync-schemas, HttpSyncHandler, HttpSyncProvider, AutoConnectionProvider, 5 test files with 55 tests), modified 7 files (schema/index, coordinator/index, network-module, types, ServerFactory, connection/index, client/index). 6 commits across 4 waves. All 13 acceptance criteria met. No deviations. Ready for review. |

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
*Last updated: 2026-02-06 (SPEC-036 completed, archived)*
