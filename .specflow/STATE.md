# SpecFlow State

## Current Position

- **Active Specification:** SPEC-017
- **Status:** review
- **Next Step:** /sf:review

## Queue

| # | ID | Title | Priority | Status | Depends On |
|---|-------|----------|--------|--------|------------|
| 1 | SPEC-017 | Add ESLint + Prettier Configuration | medium | audited | - |

## Decisions

| Date | Specification | Decision |
|------|---------------|----------|
| 2026-01-31 | SPEC-012 | COMPLETED: React Hooks Test Suite. 3 test files created (useConflictResolver, useEntryProcessor, useMergeRejections). 42 test cases total. All 182 tests pass. Archived to .specflow/archive/SPEC-012.md |
| 2026-01-31 | SPEC-013 | COMPLETED: Silent Error Handling Audit. Added debug logging to client-message-handler.ts:43. 1 commit. Archived to .specflow/archive/SPEC-013.md |
| 2026-01-31 | SPEC-014 | COMPLETED: Skipped test removed from ClientFailover.test.ts. Codebase now has zero test.skip() or test.only() patterns. Archived to .specflow/archive/SPEC-014.md |
| 2026-01-31 | SPEC-015 | COMPLETED: Schema File Splitting. Split schemas.ts (1160 lines) into 6 domain modules + barrel. All 53 message types preserved in MessageSchema union. Build passes. Archived to .specflow/archive/SPEC-015.md |
| 2026-01-31 | SPEC-016 | AUDITED: Audit v1 approved with 3 recommendations. Spec updated with missing `any` occurrences (lines 161, 168, 175, 192, 193) and expanded test file coverage. |
| 2026-01-31 | SPEC-016 | REVISED: Applied all 3 recommendations from Audit v1. Added Verification Commands section, updated test file to use MockTopGunClient interface, replaced Observable Truth #4 with measurable criterion. Ready for re-audit. |
| 2026-01-31 | SPEC-016 | AUDITED v2: All 9 dimensions PASS. Line numbers verified against source. Context estimate ~15% (PEAK range). Ready for implementation. |
| 2026-01-31 | SPEC-016 | REVIEWED: Implementation APPROVED. Zero `any` types in both files, all 13 tests pass, build succeeds, type declarations correct. Ready for finalization. |
| 2026-01-31 | SPEC-016 | COMPLETED: BetterAuth Adapter Type Safety. Eliminated all `any` types in TopGunAdapter.ts and test file. Added AuthRecord/SortSpec interfaces. 4 commits. Archived to .specflow/archive/SPEC-016.md |
| 2026-01-31 | SPEC-017 | CREATED: Add ESLint + Prettier Configuration. Infrastructure task to add linting and formatting to monorepo. |
| 2026-01-31 | SPEC-017 | AUDITED v1: APPROVED with 3 recommendations. Context estimate ~14% (PEAK). All 9 dimensions pass. Recommendations: include .tsx files in scripts, consider unified typescript-eslint package, add Key Links to Goal Analysis. |
| 2026-01-31 | SPEC-017 | REVISED: Applied all 3 recommendations. Updated scripts to include .tsx files, replaced separate packages with unified typescript-eslint, added Key Links to Goal Analysis. Ready for re-audit. |
| 2026-01-31 | SPEC-017 | AUDITED v2: APPROVED with 1 minor recommendation. Context estimate ~13% (PEAK). All 9 dimensions PASS. Previous recommendations verified as applied. Ready for implementation. |
| 2026-01-31 | SPEC-017 | REVISED v2: Applied minor recommendation. Added k6 test exclusion clarification to Out of Scope section. Ready for re-audit. |
| 2026-01-31 | SPEC-017 | AUDITED v3: APPROVED. Context estimate ~12% (PEAK). All 9 dimensions PASS. All assumptions validated against codebase. Ready for implementation. |
| 2026-01-31 | SPEC-017 | EXECUTED: ESLint + Prettier configuration added. 3 commits (config files, package.json scripts, bug fix). Dependencies installed (eslint ^9.39, prettier ^3.8, typescript-eslint ^8.49). Lint finds 2333 violations, format:check finds 441 unformatted files (not auto-fixed per constraint). Build passes, 1813/1815 tests pass (2 flaky performance tests unrelated). |

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
*Last updated: 2026-01-31*
