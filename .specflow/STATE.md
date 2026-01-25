# SpecFlow State

## Current Position

- **Active Specification:** SPEC-003b
- **Status:** review
- **Next Step:** /sf:review

## Queue

| # | ID | Title | Priority | Status | Depends On |
|---|-------|----------|--------|--------|------------|
| 1 | SPEC-003b | Extract GCHandler from ServerCoordinator | high | review | - |
| 2 | SPEC-003c | Extract ClusterEventHandler from ServerCoordinator | high | draft | SPEC-003b |
| 3 | SPEC-003d | Extract Additional Handlers to Reach 1500 Line Target | high | draft | SPEC-003c |

## Decisions

| Date | Specification | Decision |
|------|---------------|----------|
| 2026-01-23 | SPEC-001 | MessageRegistry pattern for routing CLIENT_OP and OP_BATCH to handlers |
| 2026-01-24 | SPEC-002 | PollOptions pattern for bounded test polling (timeoutMs, intervalMs, maxIterations, description) |
| 2026-01-25 | PRE-001 | Handler extraction pattern for ServerCoordinator reduction (BroadcastHandler, GCHandler, ClusterEventHandler) |
| 2026-01-25 | SPEC-003 | Split SPEC-003 into 4 parts: SPEC-003a (BroadcastHandler), SPEC-003b (GCHandler), SPEC-003c (ClusterEventHandler), SPEC-003d (Additional Handlers) |
| 2026-01-25 | SPEC-003a | BroadcastHandler extraction pattern with Config-based DI and delegation from ServerCoordinator |
| 2026-01-25 | SPEC-003b | GCHandler extraction approved - follows established handler pattern |

## Project Patterns

- Monorepo with package hierarchy: core -> client/server -> adapters/react
- TypeScript with strict mode
- Commit format: `type(scope): description`
- CRDTs use Hybrid Logical Clocks for causality tracking
- Handler extraction pattern: separate message handlers into focused modules with config injection
- Test polling pattern: use centralized test-helpers.ts with PollOptions for bounded iterations

## Warnings

_No active warnings_

---
*Last updated: 2026-01-25*
