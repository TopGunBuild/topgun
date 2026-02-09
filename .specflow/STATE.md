# SpecFlow State

## Current Position

- **Active Specification:** none
- **Status:** idle
- **TODO Items:** 16
- **Next Step:** /sf:next

## Queue

| Position | Spec | Title | Status |
|----------|------|-------|--------|
| 1 | SPEC-042 | Replace Fixed setTimeout Delays with Bounded Polling in Server Tests | review |

## Decisions

| Date | Specification | Decision |
|------|---------------|----------|
| 2026-02-09 | SPEC-043 | COMPLETED: Fix SearchCoordinator Batched LEAVE Notification Bug. Modified 1 file, 1 commit, 1 audit cycle, 1 review cycle. Archived to .specflow/archive/SPEC-043.md |
| 2026-02-09 | SPEC-043 | REVIEWED v1: APPROVED. All 7 AC verified via test run (67/67 pass). All 5 constraints respected. canDeliver guard correctly mirrors delivery logic. No critical, major, or minor issues. |
| 2026-02-09 | SPEC-043 | EXECUTED: Fix SearchCoordinator Batched LEAVE Notification Bug. Modified 1 file, 1 commit. Added canDeliver guard on 3 currentResults mutation sites in notifySubscribers(). All 7 AC met. 67/67 SearchCoordinator tests pass. No deviations. |
| 2026-02-09 | SPEC-043 | AUDITED v1: APPROVED. All 10 dimensions pass. Root cause analysis verified against source code. ~11% context estimate (PEAK range). No critical issues, no recommendations. Ready for implementation. |
| 2026-02-09 | SPEC-043 | CREATED: Fix SearchCoordinator Batched LEAVE Notification Bug. Small complexity, 1 file (SearchCoordinator.ts). From TODO-057 (P1). |
| 2026-02-08 | SPEC-042 | EXECUTED: Replace Fixed setTimeout Delays with Bounded Polling in Server Tests. Modified 21 files, 7 commits, 5 waves. 82/84 test suites pass (2 pre-existing failures). 63+ Category A delays replaced, 12+ WHY-comments added, Category C kept as-is. G2 partial (Resilience.test.ts pre-existing failure). |
| 2026-02-08 | SPEC-042 | AUDITED v2: APPROVED. All 5 revisions from Audit v1 verified correctly applied. All 10 dimensions pass. Data validated via grep (84 occurrences, 20 files match exactly). ~15% max per worker -- PEAK quality range. Ready for parallel execution. |
| 2026-02-08 | SPEC-042 | REVISED v1: Applied all 5 items from Audit v1 (1 critical + 4 recommendations). AC-7 scope corrected (Category B only). Context counts corrected (84/20). GC.test.ts L228 recategorized. ClusterE2E.test.ts phantom L245 removed. G7 split note added. Ready for re-audit. |
| 2026-02-08 | SPEC-042 | AUDITED v1: NEEDS_REVISION. 1 critical issue (AC-7 contradicts Category C "leave unchanged" directive). 4 recommendations (count/file discrepancies, GC.test.ts L228 miscategorization, phantom L245 reference, G7 size). All 10 dimensions pass. ~100% cumulative context but ~15% max per worker -- decomposition already present. |
| 2026-02-08 | SPEC-042 | CREATED: Replace Fixed setTimeout Delays with Bounded Polling in Server Tests. Medium complexity, 20 test files, 86 setTimeout instances. From TODO-055 (P1). |
| 2026-02-08 | SPEC-041 | COMPLETED: Fix DistributedSearch and GC Broadcast Test Failures. Modified 1 file, 1 commit, 2 audit cycles, 1 review cycle. Archived to .specflow/archive/SPEC-041.md |
| 2026-02-08 | SPEC-041 | REVIEWED v1: APPROVED. All 9 acceptance criteria verified (AC1-AC8 confirmed by test runs, AC9 deferred). All 6 constraints respected. No critical, major, or minor issues. Clean test-only fix: spy moved from harness proxy to broadcastHandler.broadcast. |
| 2026-02-08 | SPEC-041 | EXECUTED: Fix DistributedSearch and GC Broadcast Test Failures. Modified 1 file, 1 commit. DistributedSearch 6 tests already passing (no fix needed). GC broadcast spy fix: spy on broadcastHandler.broadcast instead of harness proxy. All 9 acceptance criteria met. 2 deviations: G2 skipped (not needed), sequential fallback (worker spawning hung). |
| 2026-02-08 | SPEC-041 | AUDITED v2: APPROVED. All 4 revisions from Response v1 verified correctly applied. All 10 dimensions pass. No new issues. ~45% context estimate (GOOD range). Ready for implementation. |
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

## Warnings

(none)

---
*Last updated: 2026-02-09 (SPEC-043 completed)*
