# SpecFlow State

## Current Position

- **Active Specification:** SPEC-001
- **Status:** review
- **Next Step:** /sf:review

## Queue

| # | ID | Title | Priority | Status |
|---|-------|----------|--------|--------|
| 1 | SPEC-001 | Remove Dead Code from ServerCoordinator | high | review |

## Decisions

| Date | Specification | Decision |
|------|---------------|----------|
| 2026-01-23 | SPEC-001 | MessageRegistry pattern for routing CLIENT_OP and OP_BATCH to handlers |

## Project Patterns

- Monorepo with package hierarchy: core -> client/server -> adapters/react
- TypeScript with strict mode
- Commit format: `type(scope): description`
- CRDTs use Hybrid Logical Clocks for causality tracking

## Warnings

_No active warnings_

---
*Last updated: 2026-01-24*
