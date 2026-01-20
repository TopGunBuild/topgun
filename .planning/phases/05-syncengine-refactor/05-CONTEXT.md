# Phase 5: SyncEngine Refactor - Context

**Gathered:** 2026-01-19
**Status:** Ready for planning

<domain>
## Phase Boundary

Split the 2612-line SyncEngine into focused, single-responsibility classes. The refactor extracts WebSocketManager, QueryManager, and BackpressureController as separate modules while SyncEngine remains the orchestrator. Topics, Counters, Locks, and Entry Processors stay in SyncEngine.

</domain>

<decisions>
## Implementation Decisions

### Module Boundaries
- Extract exactly 3 modules as specified in roadmap: WebSocketManager, QueryManager, BackpressureController
- WebSocketManager owns connection AND message routing (dispatch to handlers, not the handlers themselves)
- QueryManager owns ALL query types: standard queries, hybrid queries, and search operations
- Topics, Counters, Locks, Entry Processors remain in SyncEngine (not extracted)

### State Ownership
- Modules receive shared state references via constructor injection (not callbacks)
- WebSocketManager owns the WebSocket/connectionProvider instance (creates and manages it)
- QueryManager owns the queries Map (single source of truth for all query subscriptions)
- BackpressureController relationship with opLog: Claude's discretion based on access patterns

### Claude's Discretion
- opLog ownership: Determine whether SyncEngine or BackpressureController should own it based on usage analysis
- Message handler organization within WebSocketManager's routing
- Exact interface contracts between modules (method signatures, event types)
- How modules emit events back to SyncEngine (EventEmitter, callbacks, or direct calls)

</decisions>

<specifics>
## Specific Ideas

- Follow Phase 4 patterns where applicable (ServerCoordinator refactor used similar extraction approach)
- Keep backward compatibility: SyncEngine public API should not change for consumers

</specifics>

<deferred>
## Deferred Ideas

None - discussion stayed within phase scope

</deferred>

---

*Phase: 05-syncengine-refactor*
*Context gathered: 2026-01-19*
