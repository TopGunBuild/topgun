## Current Position

- **Active Specification:** none
- **Status:** idle
- **Next Step:** /sf:new or /sf:next

## Queue

| ID | Title | Status | Priority | Complexity |
|----|-------|--------|----------|------------|

## Decisions

- SPEC-155a implemented foundational indexing layer: Index trait, IndexableValue/ComparableValue wrappers, AttributeExtractor, HashIndex (DashMap), NavigableIndex (RwLock<BTreeMap>), InvertedIndex (DashMap). 5 files, 26 tests, 3 commits, 3 review cycles.
- Split SPEC-155 (Implement Indexing Subsystem) into 3 parts: SPEC-155a (core types + 3 index implementations), SPEC-155b (registry + mutation observer), SPEC-155c (query optimizer + wiring). Boundaries follow the 5-file Rust language profile limit and natural layer separation (data structures -> management -> integration).
- SPEC-149 fixed misleading auth/security/RBAC documentation: corrected `clientWssCode` in security.mdx (removed non-existent `token` field, added `storage`, used `setAuthToken()`), corrected false "default-deny" claims in rbac.mdac to accurately describe default-allow model. 2 commits.
- SPEC-150 fixed 5 mcp-server bugs: pagination race condition (Promise.race with 500ms timeout), QueryFilter type annotation, fields projection in QueryArgsSchema/toolSchemas.query, removed dead methods from SearchArgsSchema/toolSchemas.search, dynamic version from package.json via createRequire, fixed test mocks (CONNECTED uppercase, subscribe fires callback immediately). 77 tests pass.
- SPEC-155b implemented IndexRegistry and IndexMutationObserver: 2 new files + mod.rs update, 69 index tests, 653 total tests pass. 4 commits, 2 review cycles, all minor issues resolved.
- SPEC-155c completed indexing subsystem: query_optimizer.rs with index_aware_evaluate, get_registry on IndexObserverFactory, IndexObserverFactory wired into RecordStoreFactory observer chain, QueryService integration. 1 new + 5 modified files, 655 tests pass, 6 commits, 4 review cycles.
