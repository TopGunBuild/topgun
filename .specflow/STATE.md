# SpecFlow State

## Current Position

- **Active Specification:** none
- **Status:** idle
- **TODO Items:** 16
- **Next Step:** /sf:new or /sf:next

## Queue

| Spec | Title | Priority | Complexity |
|------|-------|----------|------------|

## Decisions

| Date | Specification | Decision |
|------|---------------|----------|
| 2026-02-05 | SPEC-001 | COMPLETED: Deterministic Simulation Testing (DST). Created 11 files (VirtualClock, SeededRNG, VirtualNetwork, InvariantChecker, ScenarioRunner + 5 test files + index), modified 4 files (HLC, LWWMap, ORMap, index). 8 commits, 1 audit cycle, 2 review cycles. Archived to .specflow/archive/SPEC-001.md |
| 2026-02-05 | SPEC-001 | REVIEWED v2: Implementation APPROVED. ClockSource consolidation verified - single definition in HLC.ts, properly imported by VirtualClock.ts. All 11 files created, 4 files modified. All 1928 tests pass. Build succeeds. All 9 acceptance criteria met. No issues found. Ready for finalization. |
| 2026-02-05 | SPEC-001 | FIXED v1: Consolidated duplicate ClockSource interface. Removed from VirtualClock.ts, now imports from HLC.ts with re-export for compatibility. Build and all tests pass. Ready for re-review. |
| 2026-02-05 | SPEC-001 | REVIEWED v1: Implementation APPROVED WITH MINOR ISSUE. All 11 files created, 4 files modified. All 9 acceptance criteria met. 1928 tests pass. Build succeeds. 1 minor issue: Duplicate ClockSource interface in HLC.ts and VirtualClock.ts (structurally compatible, no runtime impact). Ready for finalization as-is or optional fix. |
| 2026-02-05 | SPEC-001 | EXECUTED: Deterministic Simulation Testing (DST). Created testing infrastructure in packages/core/src/testing/ (VirtualClock, SeededRNG, VirtualNetwork, InvariantChecker, ScenarioRunner + 5 test files). Modified HLC, LWWMap, ORMap to use injectable clockSource. Exported DST utilities from core index. 7 commits. 114 new tests. Ready for review. |
| 2026-02-05 | SPEC-001 | AUDITED v1: Approved. All 10 dimensions pass. Context estimate ~36% (GOOD range). All 6 assumptions verified against source code. Minor clarification added: HLC needs getClockSource() getter for LWWMap/ORMap to access clock source for TTL checks. Ready for implementation. |
| 2026-02-05 | SPEC-001 | CREATED: Deterministic Simulation Testing (DST). Creates testing infrastructure in packages/core/src/testing/ with VirtualClock, SeededRNG, VirtualNetwork, InvariantChecker, ScenarioRunner. Modifies HLC to accept injectable clockSource. Source: TODO-027. |
| 2026-02-05 | MIGRATION | COMPLETED: Migrated 17 TODO items from PROMPTS directory with full context preservation. Created .specflow/reference/ with 10 key spec files (5,800+ lines of context). All TODO items in TODO.md now have links to detailed specifications. |
| 2026-02-02 | SPEC-032 | COMPLETED: Add Test Coverage for CLI Commands. Created 9 files (8 test files + 1 utility), modified package.json. 11 commits, 4 audit cycles, 1 review cycle. Archived to .specflow/archive/SPEC-032.md |
| 2026-02-02 | SPEC-032 | REVIEWED v1: Implementation APPROVED. All 9 files created and working. 28 tests pass, 4 skipped (Docker unavailable). Each command has 2+ test cases (success/error paths). Tests complete in 37s. No external service dependencies. No jest.retryTimes usage. All acceptance criteria fully met. No issues found. Ready for finalization. |
| 2026-02-02 | SPEC-032 | EXECUTED: Add Test Coverage for CLI Commands. Created 9 files (8 test files + 1 utility), modified package.json. 14 commits. 28 tests pass, 4 skipped (Docker unavailable). 6 minor deviations auto-fixed (command names, output expectations). Ready for review. |
| 2026-02-02 | SPEC-032 | AUDITED v4: Approved. Verified v3 response correctly updated debug-search test case to match actual search.js behavior. All 10 dimensions pass. All 6 assumptions verified against source. Context estimate ~25% (GOOD range). Ready for implementation. |
| 2026-02-02 | SPEC-032 | REVISED v3: Applied Audit v3 recommendation 1. Updated debug-search.test.ts test case from "shows required argument error" to "attempts HTTP call and fails with connection error" to accurately reflect search.js behavior (does not validate --map requirement). Ready for re-audit. |
| 2026-02-02 | SPEC-032 | AUDITED v3: Approved. All v2 critical issues verified as resolved. All 10 dimensions pass. Context estimate ~25% (GOOD range). 1 recommendation: debug-search test case may need adjustment since search.js does not validate --map requirement. Ready for implementation. |
| 2026-02-02 | SPEC-032 | REVISED v2: Applied all 2 critical issues and 3 recommendations. Fixed Assumption 6 (use temp dir stubs instead of CI detection), fixed cluster:start test (--help instead of invalid args), added debug-search second test, added Docker availability check, fixed test-cmd to avoid execution. Ready for re-audit. |
| 2026-02-02 | SPEC-032 | AUDITED v2: Needs revision. 2 critical issues: (1) Assumption 6 is factually incorrect - setup.js has NO CI detection, will run pnpm install/build unconditionally, (2) cluster:start test case is untestable - command has no argument validation/error path. 3 recommendations. Context estimate ~25%. |
| 2026-02-02 | SPEC-032 | REVISED v1: Applied all 5 critical issues and 4 recommendations. Added cluster:start tests, removed HTTP mocking (subprocess limitation), resolved dependency contradiction, moved test-utils.ts to Files to Create, clarified test scope for long-running processes, added CI=true assumption for setup, made AC6 measurable, corrected file count. Ready for re-audit. |
| 2026-02-02 | SPEC-032 | AUDITED v1: Needs revision. 5 critical issues: (1) Missing cluster:start test coverage, (2) HTTP mocking assumption invalid for subprocess-based tests, (3) Constraint conflicts with HTTP mocking needs, (4) test-utils.ts not in Files to Create list, (5) Key Links contradicts constraints. Context estimate ~30%. 4 recommendations. |
| 2026-02-02 | SPEC-032 | CREATED: Add Test Coverage for CLI Commands. Creates 8 test files for untested CLI command handlers (config, cluster, test, debug/crdt, debug/search, setup, dev, docker). Adds test-utils.ts for shared helpers. Source: TODO-020. |
| 2026-02-02 | SPEC-031 | COMPLETED: Split DistributedSubscriptionCoordinator into Focused Coordinators. Created base class (559 lines), SearchCoordinator (381 lines), QueryCoordinator (291 lines), transformed facade (382 lines). 4 files created, 1 file modified, 6 commits, 2 audit cycles, 1 review cycle. Archived to .specflow/archive/SPEC-031.md |
| 2026-02-02 | SPEC-031 | REVIEWED v1: Implementation APPROVED. All 7 acceptance criteria fully met. Base class (559 lines) extracts shared ACK/timeout/WebSocket logic. SearchCoordinator (381 lines) handles FTS with RRF. QueryCoordinator (291 lines) handles Query with dedupe. Facade (382 lines) preserves API. All 26 tests pass. No issues. Ready for finalization. |
| 2026-02-02 | SPEC-031 | EXECUTED: Split DistributedSubscriptionCoordinator into focused coordinators. Created DistributedSubscriptionBase (559 lines), DistributedSearchCoordinator (381 lines), DistributedQueryCoordinator (291 lines), index.ts barrel exports. Transformed original 1,065-line class into 382-line facade. All 34 tests pass. 6 commits. Ready for review. |

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

## Warnings

(none)

---
*Last updated: 2026-02-05*
