# To-Do List

## TODO-019 — 2026-02-01
**Description:** Tech Debt: Legacy/deprecated code patterns
**Priority:** medium
**Notes:**
- Source: SCAN.md (2026-02-01)
- Files: `packages/client/src/cluster/ClusterClient.ts:480-481`, `packages/core/src/debug/CRDTDebugger.ts:414`, `packages/core/src/query/QueryOptimizer.ts:79-91`
- Problem: Deprecated APIs still present for backwards compatibility
- Suggested fix: Create migration guide, remove in next major version
- Reorder reason: Benefits from type safety improvements in TODO-015

---

## TODO-020 — 2026-02-01
**Description:** Test Coverage: CLI commands lack test coverage
**Priority:** low
**Notes:**
- Source: SCAN.md (2026-02-01)
- Files: `bin/commands/debug/search.js`, `bin/commands/debug/crdt.js`, `bin/commands/setup.js`, `bin/commands/config.js`, `bin/commands/cluster/*`
- Problem: No test files found for CLI command handlers
- Suggested fix: Add integration tests in `tests/cli/` directory
- Reorder reason: Independent work that can be parallelized

---

## TODO-016 — 2026-02-01
**Description:** Tech Debt: DistributedSubscriptionCoordinator complexity
**Priority:** low
**Notes:**
- Source: SCAN.md (2026-02-01)
- Files: `packages/server/src/subscriptions/__tests__/DistributedSubscriptionCoordinator.test.ts`
- Problem: 1,282 line test file indicates component is too complex
- Suggested fix: Split distributed subscription logic into separate coordinators for FTS vs Query subscriptions
- Reorder reason: Largest architectural change, benefits from all prior type safety and cleanup work

---

*Last updated: 2026-02-01 (TODO-015 converted to SPEC-029)*
