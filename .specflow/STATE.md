# SpecFlow State

## Current Position

- **Active Specification:** none
- **Status:** idle
- **TODO Items:** 16
- **Next Step:** /sf:new or /sf:next

## Queue

| Position | Spec | Title | Status |
|----------|------|-------|--------|

## Decisions

| Date | Specification | Decision |
|------|---------------|----------|
| 2026-02-08 | SPEC-040 | COMPLETED: Fix Interceptor Integration and TLS Test Failures After Modular Refactoring. Modified 3 files, 3 commits, 1 audit cycle, 1 review cycle. Archived to .specflow/archive/SPEC-040.md |
| 2026-02-08 | SPEC-040 | REVIEWED v1: APPROVED. All 9 acceptance criteria verified. No critical, major, or minor issues. TLS warning restoration is justified blocking deviation. Clean test-only bugfix. |
| 2026-02-08 | SPEC-040 | EXECUTED: Fix Interceptor Integration and TLS test failures. Modified 3 files, 3 commits. All 9 acceptance criteria met. 1 deviation: restored TLS-disabled warning in network-module.ts (Rule 3 - Blocking). |
| 2026-02-08 | SPEC-040 | AUDITED v1: APPROVED. All 10 dimensions passed. All line references and assumption chains verified against source. ~11% context estimate (PEAK range). No critical issues, no recommendations. |
| 2026-02-08 | SPEC-040 | DRAFTED: Fix Interceptor Integration and TLS Test Failures. Test-only fixes for 5 failures across 2 files (TODO-052). Small complexity, no production code changes. |
| 2026-02-08 | SPEC-039 | COMPLETED: Add Reject Path to network.start() Promise. Modified 3 files, 1 commit, 1 audit cycle, 1 review cycle. Archived to .specflow/archive/SPEC-039.md |
| 2026-02-08 | SPEC-039 | REVIEWED v1: APPROVED. All 7 acceptance criteria verified against source. All 4 constraints respected. No critical, major, or minor issues. Clean minimal bugfix. |
| 2026-02-08 | SPEC-039 | EXECUTED: Add reject path to network.start() Promise. Modified 3 files, 1 commit. All 7 acceptance criteria met. No deviations. |
| 2026-02-08 | SPEC-039 | AUDITED v1: APPROVED. All 10 dimensions passed. All line references verified against source. ~16% context estimate (PEAK range). No critical issues, no recommendations. |
| 2026-02-08 | SPEC-039 | DRAFTED: Add reject path to network.start() Promise. Bugfix for pre-existing pattern limitation (TODO-056). 3 files, small complexity. |
| 2026-02-08 | SPEC-038 | COMPLETED: Fix WebSocket Client Auth Handshake After ServerFactory Modular Refactoring. Modified 4 files, 1 commit, 2 audit cycles, 1 review cycle. Archived to .specflow/archive/SPEC-038.md |
| 2026-02-08 | SPEC-038 | REVIEWED v1: APPROVED. All 9 acceptance criteria verified against source. No critical or major issues. 1 minor pre-existing pattern limitation (network.start() Promise never rejects). Clean minimal fix. |
| 2026-02-08 | SPEC-038 | EXECUTED: Fix WebSocket Client Auth Handshake. Modified 4 files, 1 commit. All 9 acceptance criteria met. No deviations. |
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
*Last updated: 2026-02-08 (SPEC-040 completed)*
