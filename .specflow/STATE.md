# SpecFlow State

## Current Position

- **Active Specification:** none
- **Status:** idle
- **Next Step:** `/sf:next` or `/sf:new`

## Queue

| # | ID | Title | Priority | Status | Depends On |
|---|-------|----------|--------|--------|------------|
| 1 | SPEC-011c | Network Module (Deferred Startup) | high | draft | - |
| 2 | SPEC-011d | Handlers Module + MessageRegistry | high | draft | SPEC-011c |
| 3 | SPEC-011e | Search + Lifecycle + Final Assembly | high | draft | SPEC-011d |
| 4 | SPEC-010 | Extract SyncEngine Message Handlers | medium | draft | - |

## Decisions

| Date | Specification | Decision |
|------|---------------|----------|
| 2026-01-30 | SPEC-011a | COMPLETED: Module infrastructure created (types.ts, core-module.ts, workers-module.ts). ServerFactory.create() refactored. 22 lines removed. Archived to .specflow/archive/SPEC-011a.md |
| 2026-01-30 | SPEC-011b | Audit v1: APPROVED. ~20% context (PEAK range). All constructors verified. Line numbers corrected. ClusterModule interface updated (required vs optional fields). Code examples completed with all instantiation details. |
| 2026-01-30 | SPEC-011b | EXECUTED: Created cluster-module.ts (78 lines) and storage-module.ts (47 lines). Updated types.ts with 6 interfaces. ServerFactory.ts refactored (-62 lines net). Build passes. 46+ tests pass. 5 commits total. |
| 2026-01-30 | SPEC-011b | APPROVED (Review v1): All 14 acceptance criteria met. Module factory pattern cleanly applied to cluster and storage domains. Proper dependency injection. QueryRegistry closure handled correctly. Build passes, tests pass, no circular dependencies. Zero behavior change. Code quality excellent. |
| 2026-01-30 | SPEC-011b | COMPLETED: Cluster + Storage modules extracted. ServerFactory.ts reduced 62 lines. Archived to .specflow/archive/SPEC-011b.md |

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

## Warnings

(none)

---
*Last updated: 2026-01-30 17:35*
