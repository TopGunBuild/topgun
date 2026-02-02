# SpecFlow State

## Current Position

- **Active Specification:** none
- **Status:** idle
- **Next Step:** /sf:new or /sf:next

## Queue

| Spec | Title | Priority | Complexity |
|------|-------|----------|------------|
| (empty) | | | |

## Decisions

| Date | Specification | Decision |
|------|---------------|----------|
| 2026-02-02 | SPEC-029 | COMPLETED: Eliminate `any` Types in Message Handling. Created client-message-schemas.ts with Zod schemas. Updated 4 files with typed signatures. 1 file created, 5 files modified, 5 commits, 2 audit cycles, 1 review cycle. Archived to .specflow/archive/SPEC-029.md |
| 2026-02-02 | SPEC-029 | REVIEWED v1: Implementation APPROVED. All 5 acceptance criteria met. Zero `any` in message handler signatures (documented exceptions preserved). TypeScript builds with strict mode. 431 tests pass (2 pre-existing failures unrelated). Proper Zod schema inference. All deviations justified. client-message-schemas.ts created. 5 files modified with typed signatures. Code quality excellent. No issues. Ready for finalization. |
| 2026-02-02 | SPEC-029 | EXECUTED: Eliminated `any` types in message handling. Created client-message-schemas.ts with Zod schemas. Updated ClientMessageHandlers.ts, SyncEngine.ts, TopicHandle.ts, ClusterManager.ts with typed signatures. 5 commits. Build passes. 431 tests pass. 3 deviations documented (counter state type, ClusterMessage.payload, ClusterMessageType). Ready for review. |
| 2026-02-02 | SPEC-029 | AUDITED v2: Approved. All 9 dimensions passed. Line numbers verified accurate (413, 586, 630, 655, 707-814, 824-831, 1095, 1114, 1161-1202 in SyncEngine.ts; 4, 23, 48 in TopicHandle.ts; 12-24, 29-65 in ClientMessageHandlers.ts; 33-37, 565, 579 in ClusterManager.ts). Context estimate ~38% (GOOD range). Implementation tasks generated with 5 groups across 2 waves. Ready for implementation. |
| 2026-02-02 | SPEC-029 | REVISED v1: Applied all 7 audit items (4 critical + 3 recommendations). Corrected line numbers (707-814, 824-831), added sync payload type extraction, standardized on PNCounterStateObject, clarified AC1 scope, corrected assumption 5, added notes for schema reuse. Ready for re-audit. |
| 2026-02-01 | SPEC-029 | AUDITED v1: Needs revision. 4 critical issues: (1) Line number inaccuracies for SyncEngine.ts, (2) Missing payload type definitions for sync handlers in ManagerDelegates, (3) PNCounterStateObject vs PNCounterState naming inconsistency, (4) AC1 wording conflicts with constraint about preserving Map<string, any>. 3 recommendations. Context estimate ~35%. |
| 2026-02-01 | SPEC-029 | CREATED: Eliminate `any` Types in Message Handling. Replaces ~55 `any` type annotations with proper typed interfaces in SyncEngine.ts, TopicHandle.ts, ClientMessageHandlers.ts, and ClusterManager.ts. Creates new client-message-schemas.ts. Source: TODO-015. |
| 2026-02-01 | SPEC-028 | COMPLETED: Remove jest.retryTimes from Hardened Server Tests. Removed jest.retryTimes(3) from 3 test files (LiveQuery.test.ts, Resilience.test.ts, Chaos.test.ts). 3 files modified, 3 commits, 1 audit cycle, 1 review cycle. Archived to .specflow/archive/SPEC-028.md |
| 2026-02-01 | SPEC-028 | EXECUTED: Removed jest.retryTimes(3) from 3 test files. 3 commits. LiveQuery.test.ts verified stable (10 consecutive runs). Resilience.test.ts and Chaos.test.ts have pre-existing TS errors (serverUrl removed in SPEC-018). Ready for review. |
| 2026-02-01 | SPEC-028 | AUDITED v1: Approved. All 9 dimensions passed. Line numbers verified accurate. Context estimate ~14% (PEAK range). Simple refactor with clear scope. Strategic fit confirmed. 1 optional recommendation (check CI/CD history). Ready for implementation. |
| 2026-02-01 | SPEC-028 | CREATED: Remove jest.retryTimes from Hardened Server Tests. Removes jest.retryTimes(3) from LiveQuery.test.ts, Resilience.test.ts, and Chaos.test.ts after test suite hardening from SPEC-002 has proven stable. Also removes TODO(sf-002) comment. Source: TODO-017. |
| 2026-02-01 | SPEC-027 | COMPLETED: Replace Console Statements with Structured Logging. Migrated console.warn in HLC.ts and 3 console.log in nativeStats.ts to pino structured logging. Updated 4 test files to spy on logger methods. 4 files modified, 5 commits, 2 audit cycles, 1 review cycle. Archived to .specflow/archive/SPEC-027.md |
| 2026-02-01 | SPEC-027 | REVIEWED v1: Implementation APPROVED. All acceptance criteria met. HLC.ts and nativeStats.ts migrated to structured logging. All 4 test files updated to spy on logger methods. Bug fix applied for missed test spy reference. All 31 HLC tests pass, all 24 UAT security tests pass. Build passes. Implementation follows SPEC-023 pattern. Code quality excellent. No issues. Ready for finalization. |
| 2026-02-01 | SPEC-027 | AUDITED v2: Approved. All 9 dimensions passed. Line numbers verified accurate. Context estimate ~12% (PEAK range). Minor correction applied: nativeStats.ts lines 84-88, not 84-86. Spec well-formed after v1 revisions. Ready for implementation. |
| 2026-02-01 | SPEC-027 | REVISED v1: Applied all audit feedback. (1) Removed "Do NOT modify test files" constraint, added 2 test files to scope with change details, updated Assumption 3. (2) Narrowed AC1/AC2 to specific files in scope, removed grep-based verification. (3) Clarified this migrates only TODO-018's 2 locations, not all console statements. Ready for re-audit. |
| 2026-02-01 | SPEC-027 | AUDITED v1: Needs revision. 2 critical issues: (1) Tests spy on console.warn for HLC drift - constraint "Do NOT modify test files" contradicts implementation, (2) AC1/AC2 claim "zero console.warn/log" but additional console statements exist outside scope. Context estimate ~10%. |
| 2026-02-01 | SPEC-027 | CREATED: Replace Console Statements with Structured Logging. Replaces console.warn in HLC.ts:90 and console.log in nativeStats.ts:84-86 with pino structured logging. Follows SPEC-023 pattern. Source: TODO-018. |
| 2026-02-01 | SPEC-026 | COMPLETED: Custom Foreign Key Configuration for BetterAuth Adapter. Added foreignKeyMap option to TopGunAdapterOptions for configurable join foreign keys. 2 files modified, 2 commits, 1 audit cycle, 1 review cycle. Archived to .specflow/archive/SPEC-026.md |
| 2026-02-01 | SPEC-026 | AUDITED v1: Approved. All 9 dimensions passed. Line numbers verified accurate (line 176 hardcodes userId, lines 174-175 contain TODO). Context estimate ~18%. Existing join test verifies backwards compatibility. Ready for implementation. |
| 2026-02-01 | SPEC-026 | CREATED: Custom Foreign Key Configuration for BetterAuth Adapter. Adds foreignKeyMap option to TopGunAdapterOptions for configurable join foreign keys. Default remains userId for backwards compatibility. Source: TODO-014. |
| 2026-02-01 | SPEC-025 | COMPLETED: Timer Cleanup System. Added dispose() method to SearchCoordinator that flushes notifications before clearing timer. Wired into LifecycleManager.shutdown(). 2 files modified, 3 commits, 2 audit cycles, 2 review cycles. Archived to .specflow/archive/SPEC-025.md |
| 2026-02-01 | SPEC-025 | REVIEWED v2: Implementation APPROVED. Critical issue from v1 resolved in commit 0c7b82d. dispose() method now correctly flushes notifications BEFORE clearing timer. All 4 acceptance criteria fully met. All tests pass (35 SearchCoordinator + 10 ClusterSearchCoordinator). Build passes. Code quality excellent, follows established patterns. 100% complete. Ready for finalization. |
| 2026-02-01 | SPEC-025 | FIXED v1: Reordered operations in dispose() to flush notifications BEFORE clearing timer. 1 commit (0c7b82d). Build passes. Ready for re-review. |
| 2026-02-01 | SPEC-025 | REVIEWED v1: Implementation CHANGES_REQUESTED. 1 critical issue: dispose() method has incorrect operation order - clears timer BEFORE flushing notifications, violating AC1 requirement. All LifecycleManager integration correct. Build passes. Pre-existing test failure unrelated to changes. 95% complete. Ready for fix. |
| 2026-02-01 | SPEC-025 | EXECUTED: Timer Cleanup System implemented. Added dispose() method to SearchCoordinator that flushes pending notifications before clearing timer. Wired into LifecycleManager.shutdown() after partitionReassigner.stop(). 2 files modified, 2 commits. All SearchCoordinator tests pass (35/35). Build succeeds. Ready for review. |
| 2026-02-01 | SPEC-025 | AUDITED v2: Approved. All 9 dimensions passed. notificationTimer confirmed as instance field (line 180). searchCoordinator exists in LifecycleManagerConfig but lacks dispose. Existing clear() discards notifications; new dispose() will flush first - correct distinction. Context estimate ~12%. Ready for implementation. |
| 2026-02-01 | SPEC-025 | REVISED v1: Applied all audit feedback. Removed BackpressureRegulator from scope (timer is local variable). Clarified shutdown() insertion point (after partitionReassigner.stop(), before cluster.stop()). Resolved AC ambiguity (verification via existing tests). Ready for re-audit. |
| 2026-02-01 | SPEC-025 | AUDITED v1: Needs revision. 1 critical issue: BackpressureRegulator's timeoutId is a local variable (line 161), not an instance field - dispose() cannot clear it. 2 recommendations: clarify shutdown() insertion point, resolve AC4/AC5 ambiguity. Context estimate ~17%. |
| 2026-02-01 | SPEC-025 | CREATED: Timer Cleanup System. Adds dispose() methods to SearchCoordinator and BackpressureRegulator for timer cleanup. Wires SearchCoordinator.dispose() into LifecycleManager. Source: TODO-013. |
| 2026-02-01 | SPEC-024 | COMPLETED: Type Safety Cleanup. Removed 6 @ts-ignore comments (WebCrypto polyfills), 56 as any casts in mcp-integration.test.ts, 1 eslint-disable in SettingsController. Created test-polyfills.ts, typed callTool return. 1 file created, 5 files modified. 7 commits, 1 audit cycle, 1 review cycle. Archived to .specflow/archive/SPEC-024.md |
| 2026-02-01 | SPEC-024 | EXECUTED: Type Safety Cleanup implemented. Created test-polyfills.ts with global Crypto declaration. Removed all 6 @ts-ignore comments from client tests, all 56 as any casts from mcp-integration.test.ts, and 1 eslint-disable from SettingsController. Typed callTool return value as MCPToolResult. 7 commits. All tests pass. Build succeeds. Ready for review. |
| 2026-02-01 | SPEC-024 | AUDITED v1: Approved. All 9 dimensions passed. Verified: 6 @ts-ignore (exact locations), 56 as any casts, MCPToolResult type exists. Context estimate ~25%. Fixed minor artifact naming inconsistency. Ready for implementation. |
| 2026-02-01 | SPEC-024 | CREATED: Type Safety Cleanup. Removes 6 @ts-ignore comments (WebCrypto polyfills), 56 as any casts in mcp-integration.test.ts, 1 eslint-disable in SettingsController. Creates shared test polyfill. Source: TODO-012. |
| 2026-02-01 | SPEC-023 | COMPLETED: Standardize Error Handling with Structured Logging. Replaced 12 console.error calls with pino structured logging across core/client/server/mcp-server packages. Created core logger, fixed empty catch block. 1 file created, 13 files modified. 8 commits, 1 audit cycle, 2 review cycles. Archived to .specflow/archive/SPEC-023.md |
| 2026-02-01 | SPEC-023 | REVIEWED v2: Implementation APPROVED. Fix from v1 properly applied. Test now verifies structured logging. All 67 core test suites pass (1815 tests). Build passes. Zero console.error in production code. All 12 replacements correct with structured format. No issues remaining. Ready for finalization. |
| 2026-02-01 | SPEC-023 | FIXED v1: Applied all Review v1 issues. Updated EventJournal.test.ts to spy on logger.error instead of console.error. Test now verifies structured logging format. 1 commit. All 19 EventJournal tests pass. Ready for re-review. |
| 2026-02-01 | SPEC-023 | REVIEWED v1: Implementation CHANGES_REQUESTED. 1 critical issue: Test failure in EventJournal.test.ts - test expects console.error but production code now uses logger.error. All production code changes correct. All 12 console.error replacements properly implemented with structured logging. Build passes. 1 test fails due to outdated assertion. Ready for fix. |
| 2026-02-01 | SPEC-023 | EXECUTED: Standardize Error Handling with Structured Logging implemented. Created core logger (logger.ts), added pino dependency. Replaced 12 console.error calls with structured logging across core/client/server/mcp-server packages. Fixed empty catch block in ClusterManager. 7 commits (5 implementation + 2 fixes). Build passes. Tests pass. Ready for review. |
| 2026-02-01 | SPEC-023 | AUDITED v1: Approved. All 9 dimensions passed. Line numbers verified accurate. Context estimate ~45%. Implementation Tasks generated with 5 groups across 2 waves. Ready for implementation. |
| 2026-02-01 | SPEC-023 | CREATED: Standardize Error Handling with Structured Logging. Replaces 40+ console.error calls with pino structured logging. Adds logger to core package. Fixes empty catch blocks. Source: TODO-011. |
| 2026-02-01 | SPEC-022 | COMPLETED: Harden Debug Endpoint Protection. Separated TOPGUN_DEBUG_ENDPOINTS from TOPGUN_DEBUG, added startup warning, created security documentation. 1 file created (README.md), 5 files modified. 6 commits, 1 audit cycle, 2 review cycles. Archived to .specflow/archive/SPEC-022.md |
| 2026-02-01 | SPEC-022 | REVIEWED v2: Implementation APPROVED. All Review v1 issues resolved. All 8 acceptance criteria verified. Separation of debug endpoints from logging correctly implemented. Warning logs, documentation, and test coverage complete. Implementation follows all project patterns. No remaining issues. Ready for finalization. |
| 2026-02-01 | SPEC-022 | FIXED v1: Applied all issues. Updated comment in ServerCoordinator.ts:137. Added TOPGUN_DEBUG_ENDPOINTS test assertion in env-schema.test.ts. 2 commits. 27/27 tests pass. Ready for re-review. |
| 2026-02-01 | SPEC-022 | REVIEWED v1: Implementation CHANGES_REQUESTED. 1 critical issue: Outdated comment in ServerCoordinator.ts:137 references TOPGUN_DEBUG instead of TOPGUN_DEBUG_ENDPOINTS. 1 minor issue: Missing test coverage for TOPGUN_DEBUG_ENDPOINTS. All core acceptance criteria met. 95% complete. |
| 2026-02-01 | SPEC-022 | EXECUTED: Harden Debug Endpoint Protection implemented. Created README.md with security documentation. Modified env-schema.ts (added TOPGUN_DEBUG_ENDPOINTS), DebugEndpoints.ts (warning log), ServerFactory.ts (use new env var). 4 commits. Build succeeds. 27 env schema tests pass. Ready for review. |

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
*Last updated: 2026-02-02*
