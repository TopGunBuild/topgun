# Phase 4: ServerCoordinator Refactor - Context

**Gathered:** 2026-01-18
**Status:** Ready for planning

<domain>
## Phase Boundary

Split the 5086-line ServerCoordinator god object into focused modules: AuthHandler, ConnectionManager, OperationHandler, StorageManager. ServerCoordinator becomes an orchestrator that delegates all logic. This is internal refactoring — no new capabilities, no API changes.

</domain>

<decisions>
## Implementation Decisions

### Module boundaries
- Orchestrator handles cross-cutting concerns: ServerCoordinator calls auth first, then storage — modules stay pure
- Modules can share utility/helper functions but never call each other directly
- If a clear 5th boundary emerges during extraction, extract it and document as plan deviation
- Message registry pattern: Create a mapping table that routes the 30+ ClusterMessage types to handler modules

### Extraction approach
- One plan per module: Plan 1: AuthHandler, Plan 2: ConnectionManager, Plan 3: OperationHandler, Plan 4: StorageManager — commit after each
- Delete old code immediately after extraction — clean break, no deprecation period
- Nested folder structure: `server/coordinator/auth-handler.ts`, `server/coordinator/connection-manager.ts`, etc.
- Preserve existing method signatures during extraction — refine later if needed

### Interface contracts
- Constructor injection for dependencies (database, config, logger)
- Explicit TypeScript interfaces: IAuthHandler, IConnectionManager, etc. — enables mocking and future swaps
- Callback functions for cross-module events (e.g., onClientDisconnected passed in constructor)
- Mixed state approach: ConnectionManager holds active connections (stateful), AuthHandler is stateless

### Claude's Discretion
- Exact grouping of the 30+ message types to modules
- Which utility functions to extract as shared
- Testing strategy (unit vs integration, order of test updates)
- Order of the 4 module extractions based on dependency analysis

</decisions>

<specifics>
## Specific Ideas

No specific requirements — open to standard approaches for large file refactoring.

</specifics>

<deferred>
## Deferred Ideas

None — discussion stayed within phase scope

</deferred>

---

*Phase: 04-servercoordinator-refactor*
*Context gathered: 2026-01-18*
