---
id: SPEC-048a
parent: SPEC-048
type: feature
status: done
priority: medium
complexity: medium
depends_on: []
created: 2026-02-11
---

# SPEC-048a: ConnectionPool Foundation Fixes

> Part 1 of 3 from SPEC-048 (Complete Client Cluster Integration for Transparent Partition Routing)

## Context

TopGun's client cluster infrastructure includes `ConnectionPool` for multi-node WebSocket management, `ClusterClient` as the `IConnectionProvider`, and `PartitionRouter` for key-to-partition routing. The `SyncEngine` uses the `IConnectionProvider` abstraction (SPEC-046), and the server has partition pruning (SPEC-047).

However, `ConnectionPool` has three foundational issues that must be fixed before routing logic can work correctly:

1. **WebSocketConnection allocation waste** -- `getConnection()` and `getAnyHealthyConnection()` create a new `WebSocketConnection` wrapper on every call, rather than caching and reusing the instance per node.

2. **Node ID reconciliation gap** -- The client assigns temporary IDs (`seed-0`, `seed-1`) to seed nodes, but the server assigns real node IDs. When the partition map arrives with real node IDs, the `PartitionRouter` tries to route to "new" nodes and cannot find existing connections. This is the critical wiring gap.

3. **Auth flow duplication** -- `ConnectionPool` has its own auth mechanism (`sendAuth`, `AUTHENTICATED` state) independent of `SyncEngine`'s auth flow. `ConnectionPool` gates `getConnection()` on `AUTHENTICATED` state and swallows auth-related messages instead of forwarding them. This prevents `SyncEngine` from managing auth properly in cluster mode.

### Key Links

- `ConnectionPool.addNode()` -> `PartitionRouter.updateConnectionPool()`: The router adds nodes by server-assigned IDs, but pool has them under seed IDs. **This is the critical wiring gap.**
- `ConnectionPool.handleMessage()`: Currently handles `AUTH_ACK`, `AUTH_REQUIRED`, `AUTH_FAIL` internally instead of forwarding to ClusterClient/SyncEngine.

### Reference

Original design document: `.specflow/reference/PHASE_4.5_CLIENT_CLUSTER_SPEC.md`

## Task

Fix the three foundational ConnectionPool issues (WebSocket caching, node ID reconciliation, auth message forwarding) to enable correct routing in subsequent specifications.

## Requirements

### R1: Fix WebSocketConnection Allocation (ConnectionPool)

**File:** `packages/client/src/cluster/ConnectionPool.ts`

- Currently `getConnection()` and `getAnyHealthyConnection()` create a new `WebSocketConnection` wrapper on every call. Store the `WebSocketConnection` instance alongside the raw socket in `NodeConnection` and reuse it.
- Invalidate the cached `WebSocketConnection` when the socket changes (reconnect). Eagerly nullify the cached instance in `socket.onclose` handler to ensure immediate invalidation when the socket closes.

### R2: Node ID Reconciliation (ConnectionPool)

**File:** `packages/client/src/cluster/ConnectionPool.ts`

**Primary Reconciliation Mechanism: PARTITION_MAP Endpoint Matching**

- Add a `remapNodeId(oldId: string, newId: string)` method that transfers a `NodeConnection` entry from `oldId` key to `newId` key in the connections map, preserving the existing WebSocket, state, and pending messages.
- When `PartitionRouter.updateConnectionPool()` calls `ConnectionPool.addNode()` with server-assigned node IDs from the `PARTITION_MAP` message, match the new node's endpoint against existing connection endpoints. If a matching connection exists with a different node ID (e.g., `seed-0`), call `remapNodeId` to replace the temporary ID with the server-assigned ID. Use `NodeInfo.endpoints.websocket` from the partition map for endpoint comparison.
- Update `primaryNodeId` if the remapped node was the primary.
- Emit `node:remapped` event with `(oldId, newId)` for the ClusterClient and PartitionRouter to react.
- **Future-proof optimization:** If the server includes a `nodeId` field in `AUTH_ACK` or adds a `WELCOME` message type in the future, the `handleMessage` method may extract the node ID and trigger `remapNodeId` earlier. This is an optional enhancement and not required for initial implementation.
- Update `ConnectionPoolEventType` and `ConnectionPoolEvents` type definitions to include `node:remapped` event.

### R3: Auth Flow Unification (ConnectionPool + ClusterClient)

**File:** `packages/client/src/cluster/ConnectionPool.ts`

**Auth State Tracking with Message Forwarding:**

- `handleMessage` retains all existing auth-related side effects: transitioning `connection.state` to `AUTHENTICATED` on `AUTH_ACK`, calling `flushPendingMessages()`, and calling `sendAuth()` on `AUTH_REQUIRED`.
- `handleMessage` ALSO forwards ALL messages (including `AUTH_ACK`, `AUTH_REQUIRED`, `AUTH_FAIL`) to the ClusterClient via `emit('message', nodeId, message)`. No messages are swallowed.
- ClusterClient should forward messages to SyncEngine via the `IConnectionProvider.on('message')` event. SyncEngine's existing auth flow then handles authentication.

**Method Gating Changes:**

- `getConnection()` and `getAnyHealthyConnection()` return connections that are `CONNECTED` (WebSocket open), not gated on `AUTHENTICATED` state.
- `send()` retains `AUTHENTICATED` state gating (queues messages to pending list if not authenticated).
- `getConnectedNodes()` changes to filter on `CONNECTED` state instead of `AUTHENTICATED`. **Implementer note:** This change intentionally fixes a latent bug in `ClusterClient.setupEventHandlers()` where the `node:connected` event handler calls `getConnectedNodes().length === 1` to emit the ClusterClient `connected` event. Under the old `AUTHENTICATED` gating, this check always returned 0 because `node:connected` fires when state is `CONNECTED` (before authentication completes). The new `CONNECTED` gating makes this check work correctly.
- `isNodeConnected()` changes to check `CONNECTED` state instead of `AUTHENTICATED`.
- `isConnected()` (delegates to `isNodeConnected()`) automatically inherits `CONNECTED` gating.

## Acceptance Criteria

1. **AC-1**: `WebSocketConnection` instances are cached per node connection, not recreated on every `getConnection()` call.
2. **AC-2**: When PARTITION_MAP arrives, ConnectionPool matches endpoints and remaps `seed-0` to the real node ID, and PartitionRouter can subsequently route to that node.
3. **AC-3**: `node:remapped` event is emitted with `(oldId, newId)` when node ID reconciliation occurs.
4. **AC-4**: `primaryNodeId` is updated if the remapped node was the primary.
5. **AC-5**: ConnectionPool forwards ALL messages (including AUTH_ACK, AUTH_REQUIRED, AUTH_FAIL) to ClusterClient — no message swallowing.
6. **AC-6**: `getConnection()` returns connections that are `CONNECTED` (WebSocket open), not gated on `AUTHENTICATED`.
7. **AC-7**: All existing single-server tests pass without modification (backward compatibility).
8. **AC-8**: All existing cluster tests (`ClusterClient.integration.test.ts`, `ClusterRouting.integration.test.ts`, `ClientFailover.test.ts`, `PartitionMapSync.test.ts`, `PartitionRouter.test.ts`, `PartitionRouting.test.ts`) pass without modification.

## Constraints

- **Do not modify** `packages/client/src/connection/SingleServerProvider.ts` -- single-server mode must remain unchanged.
- **Do not modify** server-side code -- the fixes are client-side only.
- **Do not break** the `IConnectionProvider` interface contract -- all changes must be backward compatible with existing implementations.
- **Do not add** new package dependencies.
- **Do not modify** `packages/core/src/types/cluster.ts` -- the type definitions are already sufficient.

## Assumptions

1. **Node ID reconciliation relies on PARTITION_MAP endpoint matching.** If the server later adds `nodeId` to `AUTH_ACK`, earlier reconciliation becomes possible as a future optimization. (Low risk -- the PartitionRouter already has this endpoint-matching logic in `updateConnectionPool`.)

2. **Auth unification will use the less invasive approach** (ConnectionPool tracks state for health but does not block message delivery) to minimize regression risk. (Low risk.)

## Implementation Tasks

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | R1: Cache WebSocketConnection in ConnectionPool (eliminates per-call allocation) | — | ~15% |
| G2 | 2 | R2: Node ID reconciliation (remapNodeId, PARTITION_MAP endpoint matching, node:remapped event) | G1 | ~25% |
| G3 | 3 | R3: Auth flow unification (message forwarding, retain auth state transitions, change method gating) | G2 | ~25% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1 | No | 1 |
| 2 | G2 | No | 1 |
| 3 | G3 | No | 1 |

**Total workers needed:** 1 (sequential execution)

## Deletions

None. All changes are modifications to existing files.

## Audit History

### Audit v1 (2026-02-11 16:00)
**Status:** NEEDS_REVISION

**Context Estimate:** ~55% total

**Critical:**

1. **R2 primary mechanism is inoperable -- server sends no nodeId in AUTH_ACK or WELCOME.** Verified in `packages/server/src/coordinator/websocket-handler.ts` line 165: server sends `{ type: 'AUTH_ACK' }` with no `nodeId` field. No `WELCOME` message type exists anywhere in the codebase. Assumption 1 acknowledges this fallback, but R2's requirements and AC-2 are written as if AUTH_ACK/WELCOME extraction is the primary mechanism ("On receiving a WELCOME or AUTH_ACK message containing a nodeId field, call remapNodeId"). An implementer would build this extraction, then discover at runtime it never triggers. R2 must be rewritten with PARTITION_MAP endpoint-matching as the primary reconciliation path. The AUTH_ACK/WELCOME path can remain as a future-proof optimization but must not be the primary design.

2. **R3 is self-contradictory -- "track auth state" vs "remove internal handling".** R3 says "ConnectionPool tracks auth state for connection health" (implying `handleMessage` still transitions `connection.state` to `AUTHENTICATED` on AUTH_ACK and calls `flushPendingMessages`), but also says "Remove internal AUTH_ACK, AUTH_REQUIRED, AUTH_FAIL handling that swallows messages." These instructions conflict. The spec must clarify: (a) Does `handleMessage` still set `connection.state = 'AUTHENTICATED'` on AUTH_ACK? (b) Does it still call `flushPendingMessages`? (c) Does it still call `sendAuth` on AUTH_REQUIRED? The intended behavior is: do all of the above AND ALSO emit the message. But the current wording ("Remove internal handling") can reasonably be interpreted as removing the state transition entirely.

3. **R3 incomplete scope -- 4 other methods also gate on AUTHENTICATED.** AC-6 says `getConnection()` returns `CONNECTED` connections, but these methods remain gated on `AUTHENTICATED` with no guidance: (a) `send()` (line 231) gates on `AUTHENTICATED`; (b) `getConnectedNodes()` (line 279) filters on `AUTHENTICATED`; (c) `isNodeConnected()` (line 294) checks `AUTHENTICATED`; (d) `isConnected()` (line 303) delegates to `isNodeConnected()`. `ClusterClient.isConnected()` calls `getConnectedNodes()`, so it would report "not connected" while `getConnection()` returns a connection. The spec must specify which methods change gating and which retain AUTHENTICATED gating.

4. **G2 and G3 cannot safely run in parallel -- both modify `handleMessage` in the same file.** G2 adds nodeId extraction from AUTH_ACK/WELCOME messages in `handleMessage` (lines 432-454). G3 changes auth message forwarding by modifying the same `handleMessage` code block. Running them as parallel workers in Wave 2 will produce merge conflicts. They should be sequential (Wave 2 then Wave 3) or merged into a single group.

**Recommendations:**

5. [Strategic] R2 should be restructured to use PARTITION_MAP endpoint-matching as the primary reconciliation mechanism. When `PartitionRouter.updateConnectionPool()` calls `ConnectionPool.addNode()` with server-assigned node IDs, it should match against existing connections by endpoint. If a match is found (same endpoint, different node ID), call `remapNodeId`. This is where the existing `NodeInfo.endpoints.websocket` in the partition map naturally provides the mapping.

6. AC-8 references `PartitionRouting.test.ts` but the actual file at `packages/client/src/__tests__/` is `PartitionRouter.test.ts` (both exist as separate files). Clarify which file(s) are meant.

7. R1 invalidation trigger should specify whether to nullify the cached `WebSocketConnection` eagerly in `socket.onclose` (line 391) or lazily on next `getConnection()` call by detecting socket mismatch. Both are valid but have different behaviors during the reconnect window.

8. R2 should note that `ConnectionPoolEventType` (line 22-28) and `ConnectionPoolEvents` (line 30-37) need to be updated to include `node:remapped`. This is in `ConnectionPool.ts` (not the constrained `cluster.ts`), so it is allowed.

### Response v1 (2026-02-11 16:15)
**Applied:** All critical issues (1-4) AND all recommendations (5-8)

**Changes:**

1. [✓] **Critical #1 - R2 primary mechanism** — Rewrote R2 to use PARTITION_MAP endpoint-matching as the primary reconciliation strategy. Added detailed explanation of endpoint comparison logic using `NodeInfo.endpoints.websocket`. Moved AUTH_ACK/WELCOME extraction to "Future-proof optimization" section, clearly marked as optional.

2. [✓] **Critical #2 - R3 self-contradiction** — Clarified R3 with new "Auth State Tracking with Message Forwarding" section. Explicitly states that `handleMessage` retains all existing side effects (state transitions, `flushPendingMessages()`, `sendAuth()`) AND ALSO forwards messages. Removed misleading "Remove internal handling" wording.

3. [✓] **Critical #3 - R3 incomplete scope** — Added "Method Gating Changes" subsection in R3 specifying: `getConnection()` and `getAnyHealthyConnection()` change to `CONNECTED` gating; `send()` retains `AUTHENTICATED` gating; `getConnectedNodes()`, `isNodeConnected()`, and `isConnected()` change to `CONNECTED` gating.

4. [✓] **Critical #4 - G2 and G3 parallel conflict** — Made task groups sequential: G1 Wave 1, G2 Wave 2, G3 Wave 3. Updated execution plan to show 3 waves, 1 worker per wave, no parallelism. Changed G3 dependency from G1 to G2.

5. [✓] **Recommendation #5 - Strategic R2 restructure** — (Same as Critical #1 resolution) R2 now leads with PARTITION_MAP endpoint-matching as the primary mechanism.

6. [✓] **Recommendation #6 - AC-8 filename** — AC-8 now correctly references `PartitionRouter.test.ts` (removed the incorrect `PartitionRouting.test.ts` reference).

7. [✓] **Recommendation #7 - R1 invalidation trigger** — Added explicit guidance in R1: "Eagerly nullify the cached instance in `socket.onclose` handler to ensure immediate invalidation when the socket closes."

8. [✓] **Recommendation #8 - node:remapped event types** — Added to R2: "Update `ConnectionPoolEventType` and `ConnectionPoolEvents` type definitions to include `node:remapped` event."

### Audit v2 (2026-02-11 17:30)
**Status:** APPROVED

**Context Estimate:** ~55% total

**Context Estimation:**

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~55% | ≤50% | ⚠ |
| Largest task group | ~25% | ≤30% | ✓ |
| Worker overhead | ~5% per wave | ≤10% | ✓ |

**Quality Projection:**

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | - |
| 30-50% | GOOD | - |
| 50-70% | DEGRADING | ← Current estimate |
| 70%+ | POOR | - |

**Per-Group Breakdown:**

| Group | Wave | Tasks | Est. Context | Cumulative |
|-------|------|-------|--------------|------------|
| G1 | 1 | Cache WebSocketConnection | ~15% | 15% |
| G2 | 2 | Node ID reconciliation | ~25% | 40% |
| G3 | 3 | Auth flow unification | ~25% | 55% |

Note: Each wave runs as a separate worker invocation (~20-25% per invocation including overhead), well within the 30% PEAK range per worker. The 55% cumulative figure applies only if run as a single worker; the sequential 3-wave plan keeps each invocation in the optimal range.

**Dimensions Evaluated:**

1. Clarity: PASS
2. Completeness: PASS
3. Testability: PASS
4. Scope: PASS
5. Feasibility: PASS
6. Architecture fit: PASS
7. Non-duplication: PASS
8. Cognitive load: PASS
9. Strategic fit: PASS
10. Project compliance: PASS

**Strategic fit:** Aligned with project goals.
**Project compliance:** Honors PROJECT.md decisions and constraints.

**Recommendations:**

1. Assumption 1 wording is slightly misleading -- it opens with "Server sends node ID in AUTH_ACK or a separate WELCOME message" as if this is the expected case, but the requirements correctly use PARTITION_MAP endpoint-matching as primary. Consider rewording to: "Node ID reconciliation relies on PARTITION_MAP endpoint matching. If the server later adds nodeId to AUTH_ACK, earlier reconciliation becomes possible as a future optimization." This is cosmetic and does not block implementation since R2's requirements are clear.

2. AC-8 lists 5 test files but omits `PartitionRouting.test.ts` (a separate file from `PartitionRouter.test.ts` that tests smart routing logic in ClusterClient). Since AC-7 covers "all existing tests pass", this is covered implicitly, but explicitly listing it in AC-8 would be more thorough for cluster test tracking.

3. The `getConnectedNodes()` gating change (CONNECTED instead of AUTHENTICATED) fixes a latent bug: the `node:connected` event handler in `ClusterClient.setupEventHandlers()` calls `getConnectedNodes().length === 1` to emit the ClusterClient `connected` event, but `node:connected` fires when state is `CONNECTED` (not yet `AUTHENTICATED`), so the check always returned 0 under the old gating. The implementer should be aware this is an intentional improvement, not a side effect.

**Comment:** Spec is well-structured after the v1 revision. All four critical issues from Audit v1 were properly addressed.

### Response v2 (2026-02-11 17:45)
**Applied:** All 3 recommendations from Audit v2

**Changes:**

1. [✓] **Recommendation #1 - Assumption 1 wording** — Reworded Assumption 1 from "Server sends node ID in AUTH_ACK or a separate WELCOME message" to "Node ID reconciliation relies on PARTITION_MAP endpoint matching. If the server later adds nodeId to AUTH_ACK, earlier reconciliation becomes possible as a future optimization."

2. [✓] **Recommendation #2 - AC-8 missing test file** — Added `PartitionRouting.test.ts` to AC-8's explicit test file list (now lists 6 files).

3. [✓] **Recommendation #3 - getConnectedNodes() latent bug note** — Added implementer note to the `getConnectedNodes()` bullet in R3's Method Gating Changes section explaining that the CONNECTED gating change intentionally fixes a latent bug in `ClusterClient.setupEventHandlers()` where `getConnectedNodes().length === 1` always returned 0 under the old AUTHENTICATED gating.

### Audit v3 (2026-02-11 18:15)
**Status:** APPROVED

**Context Estimate:** ~55% total (unchanged from v2; sequential 3-wave plan keeps each worker at ~20% PEAK)

**Dimensions Evaluated:**

1. Clarity: PASS -- Requirements use concrete method names, state values, and behavior descriptions. No vague terms.
2. Completeness: PASS -- All three requirements specify exact methods to change, exact state transitions, and exact gating behavior. Event type updates included.
3. Testability: PASS -- All 8 acceptance criteria are measurable and verifiable via unit/integration tests.
4. Scope: PASS -- Constraints clearly delineate boundaries (no server changes, no SingleServerProvider, no core types, no new deps).
5. Feasibility: PASS -- Verified against source code: `NodeConnection.endpoint` field exists for matching, `addNode()` receives both nodeId and endpoint, `handleMessage` return statements are the only barrier to forwarding.
6. Architecture fit: PASS -- Changes follow existing event emitter pattern, NodeConnection struct extension, and ConnectionPool/PartitionRouter separation of concerns.
7. Non-duplication: PASS -- Reuses existing WebSocketConnection class; no new abstractions introduced.
8. Cognitive load: PASS -- Single file modified (ConnectionPool.ts) across all 3 waves. Each wave has a focused concern.
9. Strategic fit: PASS -- Client cluster integration (TODO-023) is an established project goal. Foundation fixes are prerequisite for routing (SPEC-048b) and integration testing (SPEC-048c).
10. Project compliance: PASS -- TypeScript strict mode, no new dependencies, WHY-comments convention, client-side only.

**Assumptions Validated:**

| # | Assumption | If wrong, impact | Risk |
|---|------------|------------------|------|
| A1 | PARTITION_MAP contains node endpoints matching seed connection endpoints | Remap fails silently; routing creates duplicate connections | Low -- verified NodeInfo.endpoints.websocket exists in cluster.ts |
| A2 | Auth state tracking alongside forwarding does not cause duplicate handling | SyncEngine may process AUTH_ACK twice (once from ConnectionPool state transition, once from forwarded message) | Low -- SyncEngine auth is idempotent; ConnectionPool auth is internal state only |

**Project Compliance:**

| Decision | Spec Compliance | Status |
|----------|-----------------|--------|
| TypeScript strict mode | No `any` types added to interfaces | Compliant |
| No new runtime dependencies | No deps added | Compliant |
| Client-side only constraint | Only ConnectionPool.ts modified | Compliant |
| No code comment references | Spec notes WHY-comments convention | Compliant |
| Do not modify cluster.ts | Explicitly constrained | Compliant |

**Goal-Backward Validation:** No Goal Analysis section present. Complexity is medium -- recommended for larger specs but not blocking for a focused 3-requirement fix spec.

**Comment:** Spec is well-structured and implementable. All four critical issues from Audit v1 were properly addressed in Response v1, and all three v2 recommendations were applied in Response v2. The current spec text is internally consistent, technically sound against the source code, and ready for implementation.

---

## Execution Summary

**Executed:** 2026-02-11 20:40
**Commits:** 3

### Files Created
None.

### Files Modified
- `packages/client/src/cluster/ConnectionPool.ts` -- Added cachedConnection to NodeConnection, remapNodeId method with endpoint matching in addNode, node:remapped event type, auth message forwarding (removed return statements), CONNECTED gating for getConnection/getAnyHealthyConnection/getConnectedNodes/isNodeConnected

### Files Deleted
None.

### Acceptance Criteria Status
- [x] AC-1: WebSocketConnection instances cached per node (cachedConnection field in NodeConnection, reused in getConnection/getAnyHealthyConnection)
- [x] AC-2: PARTITION_MAP endpoint matching triggers remapNodeId in addNode(), converting seed-0 to server-assigned ID
- [x] AC-3: node:remapped event emitted with (oldId, newId) in remapNodeId method
- [x] AC-4: primaryNodeId updated when remapped node was primary
- [x] AC-5: All messages forwarded -- removed return statements after AUTH_ACK/AUTH_REQUIRED/AUTH_FAIL, unified emit('message') at end of handleMessage
- [x] AC-6: getConnection() returns connections in CONNECTED or AUTHENTICATED state (not gated on AUTHENTICATED only)
- [x] AC-7: All 27 client test suites pass (497 tests), no modifications to any test
- [x] AC-8: All 6 cluster tests pass: ClusterClient.integration, ClusterRouting.integration, ClientFailover, PartitionMapSync, PartitionRouter, PartitionRouting

### Deviations
None. Implementation followed specification exactly.

### Notes
- The CONNECTED gating in getConnection/getAnyHealthyConnection/getConnectedNodes/isNodeConnected accepts both CONNECTED and AUTHENTICATED states (since AUTHENTICATED is a superset of CONNECTED in the connection lifecycle)
- PONG messages still return early (not forwarded) as they are internal health-check responses with no value to ClusterClient/SyncEngine
- The `send()` method retains AUTHENTICATED gating as specified, ensuring messages are queued until auth completes

---

## Review History

### Review v1 (2026-02-11 20:50)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Passed:**
- [x] **AC-1: WebSocketConnection caching** -- `cachedConnection` field added to `NodeConnection` (line 45). `getConnection()` (lines 235-238) and `getAnyHealthyConnection()` (lines 255-257) check for and reuse cached instance. Invalidated eagerly in `socket.onclose` handler (line 436). Verified no stale reference risk: `WebSocketConnection` holds `readonly ws`, and `cachedConnection` is nullified before reconnect creates a new socket.
- [x] **AC-2: PARTITION_MAP endpoint matching** -- `addNode()` (lines 139-143) iterates existing connections to find endpoint match under a different ID, then calls `remapNodeId()`. End-to-end flow verified: `PartitionRouter.updateConnectionPool()` calls `addNode(node.nodeId, node.endpoints.websocket)`, which triggers the match and remap.
- [x] **AC-3: node:remapped event** -- `remapNodeId()` emits `this.emit('node:remapped', oldId, newId)` at line 220. Event type added to both `ConnectionPoolEventType` (line 27) and `ConnectionPoolEvents` (line 36).
- [x] **AC-4: primaryNodeId update** -- Lines 215-217 in `remapNodeId()` check and update `primaryNodeId` when the remapped node was primary.
- [x] **AC-5: Message forwarding** -- All `return` statements after `AUTH_ACK`, `AUTH_REQUIRED`, and `AUTH_FAIL` handling removed. Unified `this.emit('message', nodeId, message)` at line 505 forwards all messages. Only `PONG` returns early (line 502), which is correct as PONG is internal health-check plumbing.
- [x] **AC-6: CONNECTED gating** -- `getConnection()` (line 230), `getAnyHealthyConnection()` (line 254), `getConnectedNodes()` (line 324), `isNodeConnected()` (line 340) all accept `CONNECTED || AUTHENTICATED`. `send()` (line 276) retains `AUTHENTICATED`-only gating as specified.
- [x] **AC-7: All tests pass** -- Verified: 27 test suites, 497 tests, all pass. No test modifications.
- [x] **AC-8: All 6 cluster tests pass** -- Verified in test output: ClusterClient.integration, ClusterRouting.integration, ClientFailover, PartitionMapSync, PartitionRouter, PartitionRouting all PASS.
- [x] **Constraint: SingleServerProvider unchanged** -- Verified via `git diff HEAD~3..HEAD -- packages/client/src/connection/SingleServerProvider.ts` returns empty.
- [x] **Constraint: No server changes** -- Verified via `git diff HEAD~3..HEAD -- packages/server/` returns empty.
- [x] **Constraint: No new dependencies** -- Verified via `git diff HEAD~3..HEAD -- packages/client/package.json` returns empty.
- [x] **Constraint: cluster.ts unchanged** -- Verified via `git diff HEAD~3..HEAD -- packages/core/src/types/cluster.ts` returns empty.
- [x] **Constraint: IConnectionProvider interface** -- No changes to interface contract. ClusterClient still implements IConnectionProvider correctly.
- [x] **Build check** -- `pnpm --filter @topgunbuild/client build` succeeds cleanly (CJS, ESM, DTS all pass).
- [x] **Code quality** -- Clean, focused changes. WHY-comments used (e.g., line 138: "seed-0 needs to be remapped to the server-assigned node ID"). No spec references in code.
- [x] **Architecture fit** -- Follows existing patterns: event emitter, NodeConnection struct extension, ConnectionPool/PartitionRouter separation of concerns. No new abstractions.
- [x] **Non-duplication** -- Reuses existing `WebSocketConnection` class. No copy-paste or reinvention.
- [x] **Cognitive load** -- Single file modified across all 3 waves. Each change is straightforward and locally understandable. `remapNodeId` is a clean 15-line method.
- [x] **Security** -- No hardcoded secrets, no new input paths, no SQL/XSS vectors. Auth token handling unchanged.
- [x] **Implementation reality check** -- Implementation complexity matches spec expectations. No signals of misdirection. Changes are minimal and surgical as intended.

**Minor:**
1. The `PARTITION_MAP` and `PARTITION_MAP_DELTA` special-casing that was previously in `handleMessage` (original lines that did `this.emit('message', nodeId, message); return;`) has been replaced by the unified `this.emit('message', nodeId, message)` at the end. This is correct but worth noting: the old code had a redundant explicit emit for these types that would have caused a double-emit if the `return` had been missing. The new flow is cleaner and avoids this class of bug.

**Summary:** Implementation is clean, complete, and fully compliant with the specification. All 8 acceptance criteria verified. All 5 constraints honored. Build succeeds. All 497 tests pass. The diff is +55/-21 lines in a single file -- a focused, low-risk change set. No critical or major issues found.

---

## Completion

**Completed:** 2026-02-11
**Total Commits:** 3
**Audit Cycles:** 3
**Review Cycles:** 1
