# SpecFlow State

## Current Position

- **Active Specification:** none
- **Status:** idle
- **Next Step:** /sf:new or /sf:next

## Queue

(empty)

## Decisions

| Date | Specification | Decision |
|------|---------------|----------|
| 2026-02-01 | SPEC-022 | EXECUTED: Harden Debug Endpoint Protection implemented. Created README.md with security documentation. Modified env-schema.ts (added TOPGUN_DEBUG_ENDPOINTS), DebugEndpoints.ts (warning log), ServerFactory.ts (use new env var). 4 commits. Build succeeds. 27 env schema tests pass. Ready for review. |
| 2026-02-01 | SPEC-022 | REVIEWED v1: Implementation CHANGES_REQUESTED. 1 critical issue: Outdated comment in ServerCoordinator.ts:137 references TOPGUN_DEBUG instead of TOPGUN_DEBUG_ENDPOINTS. 1 minor issue: Missing test coverage for TOPGUN_DEBUG_ENDPOINTS. All core acceptance criteria met. 95% complete. |
| 2026-02-01 | SPEC-022 | FIXED v1: Applied all issues. Updated comment in ServerCoordinator.ts:137. Added TOPGUN_DEBUG_ENDPOINTS test assertion in env-schema.test.ts. 2 commits. 27/27 tests pass. Ready for re-review. |
| 2026-02-01 | SPEC-022 | REVIEWED v2: Implementation APPROVED. All Review v1 issues resolved. All 8 acceptance criteria verified. Separation of debug endpoints from logging correctly implemented. Warning logs, documentation, and test coverage complete. Implementation follows all project patterns. No remaining issues. Ready for finalization. |
| 2026-02-01 | SPEC-022 | COMPLETED: Harden Debug Endpoint Protection. Separated TOPGUN_DEBUG_ENDPOINTS from TOPGUN_DEBUG, added startup warning, created security documentation. 1 file created (README.md), 5 files modified. 6 commits, 1 audit cycle, 2 review cycles. Archived to .specflow/archive/SPEC-022.md |

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
*Last updated: 2026-02-01*
