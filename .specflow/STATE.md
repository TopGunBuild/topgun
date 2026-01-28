# SpecFlow State

## Current Position

- **Active Specification:** SPEC-004
- **Status:** review
- **Next Step:** `/sf:fix` - fix TypeScript compilation errors in ServerFactory.ts

## Queue

| # | ID | Title | Priority | Status | Depends On |
|---|-------|----------|--------|--------|------------|
| 1 | SPEC-004 | Simplify ServerCoordinator Constructor | high | review | - |

## Decisions

| Date | Specification | Decision |
|------|---------------|----------|
| 2026-01-23 | SPEC-001 | MessageRegistry pattern for routing CLIENT_OP and OP_BATCH to handlers |
| 2026-01-24 | SPEC-002 | PollOptions pattern for bounded test polling (timeoutMs, intervalMs, maxIterations, description) |
| 2026-01-25 | PRE-001 | Handler extraction pattern for ServerCoordinator reduction (BroadcastHandler, GCHandler, ClusterEventHandler) |
| 2026-01-25 | SPEC-003 | Split SPEC-003 into 4 parts: SPEC-003a (BroadcastHandler), SPEC-003b (GCHandler), SPEC-003c (ClusterEventHandler), SPEC-003d (Additional Handlers) |
| 2026-01-25 | SPEC-003a | BroadcastHandler extraction pattern with Config-based DI and delegation from ServerCoordinator |
| 2026-01-25 | SPEC-003b | GCHandler extraction approved - follows established handler pattern with distributed consensus |
| 2026-01-25 | SPEC-003c | ClusterEventHandler extraction approved - routes 16 cluster message types with callback pattern |
| 2026-01-26 | SPEC-003c | ClusterEventHandler implementation approved - all 16 message types correctly routed, 187 lines removed from ServerCoordinator |
| 2026-01-27 | SPEC-003d | Extract 7 additional handlers (HeartbeatHandler, QueryConversionHandler, BatchProcessingHandler, WriteConcernHandler, ClientMessageHandler, PersistenceHandler, OperationContextHandler) - all methods verified, ~948 lines total |
| 2026-01-27 | SPEC-003d | APPROVED: All 7 handlers extracted successfully (720 lines removed, 22.8% reduction). ServerCoordinator reduced from 3163 to 2443 lines. Target was <2300 (6.2% over), but acceptable due to delegation overhead and core coordination logic. |
| 2026-01-28 | SPEC-004 | Imported external feedback: 2 critical (QueryHandler/LifecycleManager circular deps), 3 major (target size, init order, interface fields), 1 minor (architecture docs) |
| 2026-01-28 | SPEC-004 | Response v1: Applied items 1,2,4,5 (QueryHandler wiring, LifecycleManager getMapAsync, init order, ServerDependencies clarification). Added note for item 3 (target size). Skipped item 6 (architecture docs out of scope). |
| 2026-01-28 | SPEC-004 | APPROVED: Audit v2 passed all 9 dimensions. ~35% context estimate (GOOD range). Late binding pattern for GCHandler/BatchProcessingHandler broadcast callbacks. QueryHandler wired through QueryConversionHandler. |
| 2026-01-28 | SPEC-004 | CHANGES_REQUESTED (Review v1): 58+ TypeScript compilation errors in ServerFactory.ts. Handler configurations don't match actual handler interfaces. Late binding pattern correctly implemented, but handler instantiation code needs fixes. |

## Project Patterns

- Monorepo with package hierarchy: core -> client/server -> adapters/react
- TypeScript with strict mode
- Commit format: `type(scope): description`
- CRDTs use Hybrid Logical Clocks for causality tracking
- Handler extraction pattern: separate message handlers into focused modules with config injection
- Test polling pattern: use centralized test-helpers.ts with PollOptions for bounded iterations
- Late binding pattern: handlers can receive callbacks after construction via setXxxCallbacks methods

## Warnings

**SPEC-004 Implementation Issues:**
- TypeScript compilation fails due to handler interface mismatches in ServerFactory.ts
- Build cannot complete until 58+ type errors are resolved
- Tests cannot run until build succeeds

---
*Last updated: 2026-01-28 19:30*
