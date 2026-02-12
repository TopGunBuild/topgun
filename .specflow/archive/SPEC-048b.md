---
id: SPEC-048b
parent: SPEC-048
type: feature
status: done
priority: medium
complexity: small
depends_on: [SPEC-048a]
created: 2026-02-11
---

# SPEC-048b: Routing Logic — Batch Delegation and Reconnect Map Refresh

> Part 2 of 3 from SPEC-048 (Complete Client Cluster Integration for Transparent Partition Routing)

## Context

With SPEC-048a completed, `ConnectionPool` correctly caches `WebSocketConnection` instances, reconciles seed IDs with server-assigned node IDs, and forwards all messages (including auth types) to ClusterClient. The PartitionRouter can now find connections by real node IDs.

However, two routing-level gaps remain:

1. **Per-key batch routing** -- `SyncEngine.syncPendingOperations()` sends all pending ops as a single `OP_BATCH` without per-key routing. In cluster mode, operations for different keys may need to go to different nodes (different partition owners). The batch path does not group by target node. However, `ClusterClient.sendBatch()` already implements this grouping logic — it groups operations by target node via `PartitionRouter.route(key)` and sends per-node `OP_BATCH` messages. SyncEngine should delegate to this existing infrastructure rather than reimplementing the grouping.

2. **Partition map re-request on reconnect** -- The server only sends partition maps on explicit `PARTITION_MAP_REQUEST`. The client requests it during `ClusterClient.start()`, but only if `partitionRouter.getMapVersion() === 0`. After a reconnection, the partition map may be stale, but the guard prevents re-requesting it.

### Key Links

- `SyncEngine.syncPendingOperations()` -> `WebSocketManager.sendMessage()` -> `IConnectionProvider.send()`: The batch send path does not pass keys, so cluster mode cannot route per-operation.
- `ClusterClient.sendBatch()` (lines 512-561): Already groups operations by target node via `PartitionRouter.route(key)` and sends per-node `OP_BATCH` messages. This is the correct delegation target.
- `ClusterClient.setupEventHandlers()` -> `node:connected` handler: Only requests partition map when `mapVersion === 0`.

### Reference

Original design document: `.specflow/reference/PHASE_4.5_CLIENT_CLUSTER_SPEC.md`

## Task

Wire SyncEngine batch sending to delegate to `ClusterClient.sendBatch()` in cluster mode, and fix partition map re-request on reconnect to always fetch the latest map.

## Requirements

### R1: Batch Routing via ClusterClient Delegation (SyncEngine + IConnectionProvider)

SyncEngine should remain transport-agnostic. Rather than adding cluster-aware grouping logic to SyncEngine, the batch send path should delegate to the connection provider.

**File:** `packages/client/src/types.ts`

- Add an optional `sendBatch?(operations: Array<{ key: string; message: any }>): Map<string, boolean>` method to the `IConnectionProvider` interface. This keeps the interface backward compatible — providers that do not support batch routing simply omit the method.

**File:** `packages/client/src/SyncEngine.ts`

- Modify `syncPendingOperations()` to check if the connection provider implements `sendBatch`. The connection provider is accessed via `this.webSocketManager.getConnectionProvider()`. If the provider implements `sendBatch`, call `connectionProvider.sendBatch(ops)` where each op is `{ key: op.key, message: op }` (`op` being the `OpLogEntry`). If `sendBatch` is not available, keep existing behavior: send all ops in one `OP_BATCH` via `sendMessage()`.
- The `key` field from `OpLogEntry` is the routing key used for partition assignment. This is the same field that `PartitionRouter.route(key)` accepts.

**File:** `packages/client/src/cluster/ClusterClient.ts`

- `ClusterClient` already implements `sendBatch()` with the correct signature. No changes needed to ClusterClient for batch routing — the existing method groups by target node via `PartitionRouter.route(key)` and sends per-node `OP_BATCH` messages.

**File:** `packages/client/src/connection/SingleServerProvider.ts`

- Do NOT add `sendBatch` to SingleServerProvider. It does not implement the optional method, so SyncEngine falls back to existing single-batch behavior. No changes to this file.

### R2: Partition Map Re-Request on Reconnect (ClusterClient)

**File:** `packages/client/src/cluster/ClusterClient.ts`

- In `setupEventHandlers()`, on `node:connected` event, always request the partition map (remove the `if (this.partitionRouter.getMapVersion() === 0)` guard). This ensures that after a reconnection, the client gets the latest partition map even if it had one before (it may be stale post-reconnect).
- Add a debounce (e.g., 500ms) to prevent flooding with requests when multiple nodes reconnect simultaneously.

### Deletions

None. All changes are modifications to existing files.

## Acceptance Criteria

1. **AC-1**: `TopGunClient({ cluster: { seeds: [...] } })` connects and receives a partition map (version > 0) within 10 seconds after authentication. *(Integration-level — verified by SPEC-048c.)*
2. **AC-2**: After partition map is received, `client.getMap('test').set('key-1', { v: 1 })` sends the operation message to the node that owns partition `hash('key-1') % 271`. *(Integration-level — verified by SPEC-048c.)*
3. **AC-3**: `syncPendingOperations()` delegates to `connectionProvider.sendBatch()` when available (cluster mode), passing each operation's `key` field as the routing key. `ClusterClient.sendBatch()` groups these by target node and sends separate `OP_BATCH` messages per node.
4. **AC-4**: In single-server mode, `syncPendingOperations()` continues to send all ops in one batch (no behavior change) because `SingleServerProvider` does not implement `sendBatch`.
5. **AC-5**: Routing metrics (`getRoutingMetrics()`) on ClusterClient accurately track direct routes, fallback routes, and partition misses. (Already implemented in `ClusterClient.getRoutingMetrics()` — this AC verifies no regression.)
6. **AC-6**: After reconnection, the client re-requests the partition map regardless of current map version.
7. **AC-7**: Multiple simultaneous node reconnections do not flood the server with partition map requests (debounce).
8. **AC-8**: All existing single-server tests pass without modification (backward compatibility).
9. **AC-9**: All existing cluster tests (`ClusterClient.integration.test.ts`, `ClusterRouting.integration.test.ts`, `ClientFailover.test.ts`, `PartitionMapSync.test.ts`, `PartitionRouting.test.ts`) pass without modification.

## Constraints

- **Do not modify** `packages/client/src/connection/SingleServerProvider.ts` -- single-server mode must remain unchanged.
- **Do not modify** server-side code -- the server already handles `PARTITION_MAP_REQUEST`; the fixes are client-side only.
- **Do not break** the `IConnectionProvider` interface contract -- the `sendBatch` addition is optional (method may be undefined), so all existing implementations remain valid without changes.
- **Do not add** new package dependencies.
- **Do not modify** `packages/core/src/types/cluster.ts` -- the type definitions are already sufficient.

## Assumptions

1. **SyncEngine can detect cluster mode by checking if `connectionProvider.sendBatch` is defined.** This avoids needing a new `isClusterMode()` method and keeps SyncEngine transport-agnostic. (Low risk — standard TypeScript optional method check.)

2. **`OpLogEntry.key` is the correct routing key for partition assignment.** This is the same key that `PartitionRouter.route(key)` uses to compute `hash(key) % 271`. (Low risk — consistent with existing ClusterClient.sendBatch usage.)

## Implementation Tasks

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | R2: Partition map re-request on reconnect (remove version-0 guard, add debounce) | — | ~12% |
| G2 | 1 | R1: Add optional `sendBatch` to IConnectionProvider, modify SyncEngine.syncPendingOperations to delegate | — | ~20% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1, G2 | Yes | 2 |

**Total workers needed:** 2 (max in any wave)

## Audit History

### Audit v1 (2026-02-12 14:30)
**Status:** NEEDS_REVISION

**Context Estimate:** ~60% total

**Critical:**

1. **Server does NOT send NOT_OWNER messages to clients.** The spec's constraint states "the server already handles PARTITION_MAP_REQUEST and sends NOT_OWNER errors" but source code verification shows this is FALSE. In `packages/server/src/coordinator/cluster-event-handler.ts` line 186, when the server receives an OP_FORWARD for a partition it doesn't own, it logs a warning ("Received OP_FORWARD but not owner. Dropping.") and silently drops the message. There are zero instances in the server package of constructing or sending a `NOT_OWNER` response to clients. The `NotOwnerError` type in `packages/core/src/types/cluster.ts` exists only as a type definition -- it is never instantiated or sent. This means R2 (NOT_OWNER error handling) and AC-5 are based on a false premise: the server-side NOT_OWNER response does not exist yet. Either the server must be modified to send NOT_OWNER (which violates the spec's own constraint), or R2 must be redesigned.

2. **NOT_OWNER message format mismatch.** Assumption 1 states the message format is `{ type: 'NOT_OWNER', hint: {...} }`, but the `NotOwnerError` interface uses `code: 'NOT_OWNER'` (not `type`). The spec conflates two different shapes. If the server were to send this message, the handler would need to match the actual message shape.

3. **Significant duplication with existing ClusterClient infrastructure.** `ClusterClient.sendWithRetry()` (lines 284-372) already implements NOT_OWNER retry logic with partition map refresh and exponential backoff. `ClusterClient.sendBatch()` (lines 512-561) already groups operations by target node and sends per-node OP_BATCH messages. `PartitionRouter.handleNotOwnerError()` (line 368) already handles NOT_OWNER errors with routing miss events and map refresh. The spec proposes reimplementing this logic inside SyncEngine rather than leveraging the existing ClusterClient methods, creating dual code paths for the same concerns.

**Recommendations:**

4. **[Strategic] Reconsider the approach for R1 and R2.** Rather than adding cluster-aware logic to SyncEngine (which should remain transport-agnostic), consider having SyncEngine delegate batch sending to the connection provider. `ClusterClient.sendBatch()` already groups by partition and routes per-node. SyncEngine could call `connectionProvider.sendBatch()` if available, keeping SyncEngine clean and avoiding cluster-specific logic leaking into it.

5. **[Strategic] R2 may need server-side work first.** If NOT_OWNER handling is needed, the server must be modified to actually send NOT_OWNER responses before the client can handle them. This should either be split into a separate server-side spec or acknowledged as out of scope for the client-only constraint.

6. **Clarify grouping key for batch routing.** R1 says to group by "routing key" but OpLogEntry has both `mapName` and `key`. The routing key for partition lookup should be `key` (or potentially `mapName:key`). The spec should be explicit about which field(s) are hashed for partition assignment.

7. **AC-7 (routing metrics) has no implementation task.** The acceptance criterion requires routing metrics to be accurately tracked, but no requirement or task addresses implementing or modifying metrics tracking. The existing `getRoutingMetrics()` in ClusterClient may already satisfy this, but this should be explicitly stated.

8. **G3 dependency on G1 is questionable.** Per-key batch routing (G3/R1) does not technically depend on NOT_OWNER handling (G1/R2). Batch routing is about correct partition-aware sends; NOT_OWNER handling is about error recovery. These are independent concerns. G3 should have no dependencies (wave 1) or depend only on G2 if the partition map refresh logic is needed.

### Response v1 (2026-02-12 16:45)
**Applied:** All 3 critical issues and all 5 recommendations (items 1-8).

**Changes:**
1. [✓] Server does NOT send NOT_OWNER messages — Removed R2 (NOT_OWNER error handling) entirely. Removed the false constraint claiming "the server already sends NOT_OWNER errors." Removed AC-5 (NOT_OWNER client handling) and AC-6 (fallback re-route on NOT_OWNER). Updated title, Context, and Task sections to reflect the reduced scope.
2. [✓] NOT_OWNER message format mismatch — Removed Assumption 1 (NOT_OWNER format) and Assumption 3 (OP_BATCH routing key hint) since R2 is removed and the OP_BATCH concern is moot when delegating to ClusterClient.
3. [✓] Significant duplication with ClusterClient — Restructured R1 to delegate to `ClusterClient.sendBatch()` via an optional `sendBatch` method on `IConnectionProvider`, rather than reimplementing grouping logic in SyncEngine. SyncEngine remains transport-agnostic.
4. [✓] Reconsider approach for R1 — R1 now adds optional `sendBatch` to IConnectionProvider and has SyncEngine check for it. Cluster-aware grouping stays in ClusterClient where it belongs.
5. [✓] R2 needs server-side work first — R2 removed entirely as out of scope. NOT_OWNER server-side implementation can be a future spec if needed.
6. [✓] Clarify grouping key — R1 now explicitly states that `OpLogEntry.key` is the routing key, and that this is the same key `PartitionRouter.route(key)` uses to compute `hash(key) % 271`. Added as Assumption 2.
7. [✓] AC-7 routing metrics has no implementation task — Reworded AC-5 (renumbered from AC-7) to clarify that `ClusterClient.getRoutingMetrics()` already satisfies this and the AC verifies no regression rather than new implementation.
8. [✓] G3 dependency on G1 is questionable — Removed G1 (was R2/NOT_OWNER). G1 and G2 are now the two remaining groups (R2 reconnect + R1 batch delegation), both in wave 1 with no dependencies between them. Reduced complexity from medium to small.

### Audit v2 (2026-02-12 18:00)
**Status:** APPROVED

**Context Estimate:** ~32% total

**Execution Scope:**

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~32% | <=50% | OK |
| Largest task group (G2) | ~20% | <=30% | OK |
| Worker overhead | ~10% | <=10% | OK |

**Quality Projection:**

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | - |
| 30-50% | GOOD | <-- Current estimate |
| 50-70% | DEGRADING | - |
| 70%+ | POOR | - |

**Dimensions:**

All 10 audit dimensions pass. Source code verified against:
- `IConnectionProvider` in `packages/client/src/types.ts` -- confirmed no `sendBatch` method exists; adding optional method is backward compatible.
- `SyncEngine.syncPendingOperations()` at lines 498-510 -- confirmed sends single `OP_BATCH` without per-key routing; connection provider accessible via `this.webSocketManager.getConnectionProvider()`.
- `ClusterClient.sendBatch()` at lines 512-561 -- confirmed groups by target node via `PartitionRouter.route(key)` and sends per-node `OP_BATCH` messages. Signature `sendBatch(operations: Array<{ key: string; message: any }>): Map<string, boolean>` matches proposed interface addition.
- `ClusterClient.setupEventHandlers()` at line 782 -- confirmed `if (this.partitionRouter.getMapVersion() === 0)` guard exists and limits partition map requests to first connection only.
- `SingleServerProvider` -- confirmed does not implement `sendBatch`.

**Assumptions validated:**
- Assumption 1: `this.webSocketManager.getConnectionProvider()` provides access. Optional method check (`if (provider.sendBatch)`) is standard TypeScript.
- Assumption 2: `OpLogEntry.key` (line 43 of SyncEngine.ts) is the routing key. Confirmed consistent with `PartitionRouter.route(key)` usage in `ClusterClient.sendBatch()`.

**Strategic fit:** Aligned with project goals. Phase 0 (TypeScript Completion) work, necessary for SPEC-048c (E2E test).

**Project compliance:** Honors PROJECT.md decisions. No new dependencies, TypeScript strict mode compatible, client-side only changes.

**Recommendations:**

1. **Clarify the `message` field contents in `sendBatch` delegation.** R1 states SyncEngine should call `connectionProvider.sendBatch(ops)` where each op includes the `key` field, but does not explicitly state what the `message` field should contain. From source inspection, `ClusterClient.sendBatch()` wraps `messages.map(m => m.message)` into `OP_BATCH` payloads (lines 535, 539), so each `message` should be the raw `OpLogEntry` (or the relevant operation data). Consider adding a note like: "Each operation in the array is `{ key: op.key, message: op }` where `op` is the `OpLogEntry`."

2. **AC-1 and AC-2 are integration-level criteria.** These describe end-to-end behavior requiring a running cluster, which seems more appropriate for SPEC-048c (E2E test). They are not harmful here but may cause confusion about whether this spec needs to create new integration tests. Consider marking them as "verified by SPEC-048c" to set clear expectations.

3. **R1 should note that SyncEngine accesses the connection provider via `this.webSocketManager.getConnectionProvider()`.** The spec says to "check if the connection provider implements sendBatch" but `syncPendingOperations()` currently has no direct reference to the connection provider. The access path is `this.webSocketManager.getConnectionProvider()`. This is a minor implementation detail the developer can figure out, but stating it avoids any ambiguity.

### Response v2 (2026-02-12 19:15)
**Applied:** All 3 recommendations from audit v2 (items 1-3).

**Changes:**
1. [✓] Clarify `message` field contents in `sendBatch` delegation — Updated R1's SyncEngine section to explicitly state that each op passed to `sendBatch` is `{ key: op.key, message: op }` where `op` is the `OpLogEntry`.
2. [✓] AC-1 and AC-2 are integration-level criteria — Annotated AC-1 and AC-2 with "*(Integration-level — verified by SPEC-048c.)*" to set clear expectations that these do not require new tests in this spec.
3. [✓] R1 should note connection provider access path — Added to R1's SyncEngine section that the connection provider is accessed via `this.webSocketManager.getConnectionProvider()`.

### Audit v3 (2026-02-12 20:30)
**Status:** APPROVED

**Context Estimate:** ~32% total

**Execution Scope:**

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~32% | <=50% | OK |
| Largest task group (G2) | ~20% | <=30% | OK |
| Worker overhead | ~10% | <=10% | OK |

**Quality Projection:**

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | - |
| 30-50% | GOOD | <-- Current estimate |
| 50-70% | DEGRADING | - |
| 70%+ | POOR | - |

**Dimensions:**

All 10 audit dimensions pass with fresh-eyes verification:

1. **Clarity:** Requirements are precise with exact file paths, method names, and signatures. No vague terms.
2. **Completeness:** All files to modify listed. Files NOT to modify explicitly constrained. Deletions section present (none needed).
3. **Testability:** 9 acceptance criteria, all measurable. AC-1/AC-2 correctly deferred to SPEC-048c. AC-3 through AC-9 verifiable at unit/integration level.
4. **Scope:** Two focused requirements (R1, R2) with 5 explicit constraints. Complexity "small" is accurate for 2-3 file modifications.
5. **Feasibility:** All source code claims verified against current codebase (see below).
6. **Architecture fit:** Delegation via optional interface method is consistent with existing IConnectionProvider adapter pattern.
7. **Non-duplication:** Deliberately reuses ClusterClient.sendBatch() rather than reimplementing grouping logic.
8. **Cognitive load:** Minimal -- one optional method addition, one conditional branch, one guard removal with debounce.
9. **Strategic fit:** Phase 0 (TypeScript Completion) work, directly enables SPEC-048c E2E test.
10. **Project compliance:** No new dependencies, TypeScript strict mode compatible, client-side only, honors all PROJECT.md constraints.

**Source code verification (fresh):**
- `IConnectionProvider` in `packages/client/src/types.ts` (lines 43-111): Confirmed no `sendBatch` method. Interface has `send(data, key?)`, `getConnection(key)`, `getAnyConnection()`, `on/off`, `connect`, `close`, `isConnected`, `getConnectedNodes`. Adding optional `sendBatch?` is backward compatible.
- `SyncEngine.syncPendingOperations()` at lines 498-510: Confirmed sends `{ type: 'OP_BATCH', payload: { ops: pending } }` via `this.sendMessage()` without per-key routing. WebSocketManager accessible at `this.webSocketManager`, and `getConnectionProvider()` confirmed at line 170 of WebSocketManager.ts.
- `ClusterClient.sendBatch()` at lines 512-561: Signature is `sendBatch(operations: Array<{ key: string; message: any }>): Map<string, boolean>`. Groups by target node via `this.partitionRouter.route(key)`, sends per-node `OP_BATCH`. Matches proposed interface addition exactly.
- `ClusterClient.setupEventHandlers()` at line 782: Guard `if (this.partitionRouter.getMapVersion() === 0)` confirmed. Only requests partition map on first connection.
- `SingleServerProvider` (lines 27-287): Implements `IConnectionProvider` without `sendBatch`. No changes needed.

**Assumptions validated:**
- Assumption 1: `connectionProvider.sendBatch` check is standard TypeScript optional method detection. WebSocketManager.getConnectionProvider() returns IConnectionProvider.
- Assumption 2: `OpLogEntry.key` (line 43 of SyncEngine.ts) is the routing key used by `PartitionRouter.route(key)` in ClusterClient.sendBatch() (line 520).

**Strategic fit:** Aligned with project goals. Phase 0 completion work, prerequisite for SPEC-048c.

**Project compliance:** Honors all PROJECT.md decisions and constraints. No violations or deviations.

**Comment:** Spec is well-structured and implementation-ready after two revision cycles. All previous audit findings have been properly addressed. The delegation approach via optional `IConnectionProvider.sendBatch` is clean, maintains SyncEngine's transport-agnostic design, and avoids duplication. The reconnect map refresh fix (R2) is a straightforward one-line guard removal with sensible debounce. No critical issues or recommendations remain.

## Execution Summary

**Executed:** 2026-02-12
**Mode:** orchestrated (sequential fallback -- no subagent tool available)
**Commits:** 2

### Execution Waves

| Wave | Groups | Status |
|------|--------|--------|
| 1 | G1, G2 | complete |

### Files Modified
- `packages/client/src/types.ts` -- Added optional `sendBatch` method to `IConnectionProvider` interface
- `packages/client/src/SyncEngine.ts` -- Modified `syncPendingOperations()` to delegate to `sendBatch` when available
- `packages/client/src/cluster/ClusterClient.ts` -- Removed version-0 guard on partition map request, added debounced request with 500ms coalescing, cleanup on close

### Acceptance Criteria Status
- [x] AC-1: Integration-level (verified by SPEC-048c)
- [x] AC-2: Integration-level (verified by SPEC-048c)
- [x] AC-3: `syncPendingOperations()` delegates to `connectionProvider.sendBatch()` when available, passing `{ key: op.key, message: op }` for each operation
- [x] AC-4: Single-server mode unchanged -- `SingleServerProvider` does not implement `sendBatch`, so fallback path used
- [x] AC-5: Routing metrics unchanged -- no regression (verified: no modifications to `getRoutingMetrics()`)
- [x] AC-6: After reconnection, partition map re-requested regardless of current map version (version-0 guard removed)
- [x] AC-7: Multiple simultaneous reconnections debounced via 500ms timer coalescing
- [x] AC-8: All 497 existing tests pass without modification (27 test suites)
- [x] AC-9: All cluster tests pass without modification

### Deviations
None.

---

## Review History

### Review v1 (2026-02-12 21:30)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Minor:**
1. **sendBatch return value ignored in SyncEngine**
   - File: `/Users/koristuvac/Projects/topgun/topgun/packages/client/src/SyncEngine.ts:509`
   - The `sendBatch` method returns `Map<string, boolean>` indicating per-key success/failure, but SyncEngine discards the return value. This is consistent with the existing fallback path (which also ignores `sendMessage()`'s return value), and ops are only marked synced when `OP_ACK` arrives from the server. However, future implementations could benefit from logging failed keys. Low priority.

2. **No new unit tests for the sendBatch delegation or debounce behavior**
   - The existing 497 tests all pass (AC-8, AC-9), confirming backward compatibility. However, there are no new tests that specifically exercise: (a) the `sendBatch` delegation path when a provider implements it, or (b) the debounce timer coalescing behavior. The spec does not require new unit tests (AC-1/AC-2 are deferred to SPEC-048c), so this is not a compliance gap, but adding targeted unit tests would improve confidence in the new code paths.

**Passed:**
- [x] AC-3: `syncPendingOperations()` at line 508 checks `connectionProvider.sendBatch`, delegates with `{ key: op.key, message: op }` mapping -- exactly matches spec
- [x] AC-4: `SingleServerProvider` does not implement `sendBatch` (verified: no `sendBatch` in file), fallback path at line 514 sends single `OP_BATCH` -- no behavior change
- [x] AC-5: `getRoutingMetrics()` at line 599 unchanged -- no regression
- [x] AC-6: Version-0 guard fully removed (grep confirms zero matches for `getMapVersion() === 0` in client package); replaced with unconditional `debouncedPartitionMapRequest(nodeId)` at line 806
- [x] AC-7: Debounce implemented via `partitionMapRequestTimer` with 500ms constant; `clearTimeout` + `setTimeout` pattern correctly coalesces rapid calls, only the last nodeId is used
- [x] AC-8: All 497 existing tests pass across 27 test suites (verified by running `pnpm --filter @topgunbuild/client test`)
- [x] AC-9: All cluster tests pass without modification (verified in same test run)
- [x] Constraint: `SingleServerProvider` not modified (git diff confirms no changes)
- [x] Constraint: No server-side code modified (git diff confirms no changes in `packages/server/`)
- [x] Constraint: `IConnectionProvider` backward compatible -- `sendBatch?` is optional with `?` syntax
- [x] Constraint: No new package dependencies (package.json unchanged)
- [x] Constraint: `packages/core/src/types/cluster.ts` not modified (git diff confirms)
- [x] Build succeeds (`pnpm build` completes without errors)
- [x] Timer cleanup: `partitionMapRequestTimer` properly cleared in `close()` method at line 646
- [x] Code comments follow WHY-comment convention (no spec/phase references)
- [x] Only 3 files modified, matching execution summary exactly
- [x] Architecture: SyncEngine remains transport-agnostic; cluster logic stays in ClusterClient
- [x] Non-duplication: Reuses existing `ClusterClient.sendBatch()` rather than reimplementing grouping
- [x] Cognitive load: Minimal changes -- one optional interface method, one conditional branch, one guard removal with debounce

**Summary:** Implementation precisely matches the specification with no critical or major issues. The three modified files (`types.ts`, `SyncEngine.ts`, `ClusterClient.ts`) implement exactly what was specified: an optional `sendBatch` interface method enabling SyncEngine to delegate per-key routing to ClusterClient in cluster mode, and unconditional debounced partition map requests on reconnect. All 5 constraints are honored, all 9 acceptance criteria are satisfied (with AC-1/AC-2 deferred to SPEC-048c as designed). The code is clean, minimal, and well-commented. Two minor observations noted but neither blocks approval.

### Fix Response v1 (2026-02-12 22:30)
**Applied:** All 2 minor issues from Review v1.

**Fixes:**
1. [✓] sendBatch return value ignored — Captured return `Map<string, boolean>`, filter for failed keys, log warning via `logger.warn()` when any keys fail.
   - Commit: 921fd81
2. [✓] No new unit tests — Added 3 tests to `SyncEngine.test.ts`: (a) sendBatch delegation when provider implements it, (b) fallback to single OP_BATCH when provider lacks sendBatch, (c) logger.warn emitted for failed keys.
   - Commit: 921fd81

**Verification:** All 500 tests pass (27 test suites, 3 new tests added).

### Review v2 (2026-02-12 23:00)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Focus:** Verification of Fix Response v1 changes + full re-review with fresh eyes.

**Findings:**

**Passed:**
- [x] **Fix #1 verified: sendBatch return value handling** -- `SyncEngine.ts` line 509 now captures the `Map<string, boolean>` result from `connectionProvider.sendBatch()`. Line 510 filters entries where `success` is `false` to extract failed keys. Lines 511-512 log a structured warning via `logger.warn({ failedKeys, count }, ...)` when any keys fail. The early `return` at line 514 correctly prevents the fallback path from also executing.
- [x] **Fix #2 verified: New unit tests** -- 3 new tests added to `/Users/koristuvac/Projects/topgun/topgun/packages/client/src/__tests__/SyncEngine.test.ts` in the "Batch delegation (sendBatch)" describe block (lines 978-1121):
  - (a) "should delegate to sendBatch when provider implements it" (line 1028) -- creates mock cluster provider with `sendBatch`, triggers AUTH_ACK, verifies `sendBatch` called once with correct `{ key, message }` pairs
  - (b) "should fall back to single OP_BATCH when provider lacks sendBatch" (line 1059) -- uses default config without `sendBatch`, verifies OP_BATCH sent via WebSocket
  - (c) "should log warning when sendBatch reports failed keys" (line 1081) -- overrides `sendBatch` to return `false` for 'user2', verifies `logger.warn` called with `{ failedKeys: ['user2'], count: 1 }`
- [x] **AC-3:** `syncPendingOperations()` at line 508 checks `connectionProvider.sendBatch`, delegates with `pending.map(op => ({ key: op.key, message: op }))` -- matches spec exactly. Now also handles return value for observability.
- [x] **AC-4:** `SingleServerProvider` does not implement `sendBatch` (confirmed: no `sendBatch` in file at `/Users/koristuvac/Projects/topgun/topgun/packages/client/src/connection/SingleServerProvider.ts`). SyncEngine falls back to single `OP_BATCH` via `sendMessage()` at line 518.
- [x] **AC-5:** `getRoutingMetrics()` at line 599 of ClusterClient.ts unchanged -- returns a spread copy of `routingMetrics`. No regression.
- [x] **AC-6:** Version-0 guard fully removed -- `grep` for `getMapVersion() === 0` returns zero matches in client package. Replaced with unconditional `this.debouncedPartitionMapRequest(nodeId)` at line 806 of ClusterClient.ts.
- [x] **AC-7:** Debounce via `partitionMapRequestTimer` (line 88) with static 500ms constant (line 89). `debouncedPartitionMapRequest()` (lines 249-257) uses `clearTimeout` + `setTimeout` pattern -- coalesces rapid calls, uses the most recently connected nodeId.
- [x] **AC-8:** All 500 tests pass (27 test suites) -- confirmed by running `pnpm --filter @topgunbuild/client test`. The 3 new tests bring the total from 497 to 500.
- [x] **AC-9:** All cluster tests pass within the same test run -- no modifications needed.
- [x] **Constraint: SingleServerProvider not modified** -- `git diff HEAD~3 --name-only -- packages/client/src/connection/SingleServerProvider.ts` returns empty.
- [x] **Constraint: No server-side code modified** -- `git diff HEAD~3 --name-only -- packages/server/` returns empty.
- [x] **Constraint: IConnectionProvider backward compatible** -- `sendBatch?` uses TypeScript optional method syntax (`?` after name) at line 116 of types.ts.
- [x] **Constraint: No new package dependencies** -- no changes to any package.json.
- [x] **Constraint: cluster.ts not modified** -- `git diff HEAD~3 --name-only -- packages/core/src/types/cluster.ts` returns empty.
- [x] **Build succeeds** -- `pnpm build` completes without errors.
- [x] **Timer cleanup** -- `partitionMapRequestTimer` cleared in `close()` method at lines 646-649 of ClusterClient.ts. Sets to `null` after clearing.
- [x] **Code comments** -- WHY-comments used throughout. No spec/phase/bug references in code. Comment at line 504-506 of SyncEngine.ts explains the delegation pattern clearly.
- [x] **Files modified** -- 3 source files (`types.ts`, `SyncEngine.ts`, `ClusterClient.ts`) + 1 test file (`SyncEngine.test.ts`). Matches execution summary + fix response.
- [x] **Architecture** -- SyncEngine remains transport-agnostic; the `sendBatch` check is a simple capability detection, not cluster-specific logic. All routing logic stays in ClusterClient.
- [x] **Non-duplication** -- Reuses existing `ClusterClient.sendBatch()` for per-node grouping. No reimplementation.
- [x] **Cognitive load** -- Minimal additions: one optional interface method (6 lines with JSDoc), one conditional branch (7 lines), one guard removal with debounce helper method (8 lines). New tests are well-structured with clear mock setup and assertion.
- [x] **Security** -- No hardcoded secrets, no new input vectors, no external dependencies added.
- [x] **Implementation reality check** -- Implementation is appropriately scoped for what the spec describes. No signs of strategic mismatch.

**Summary:** All Fix Response v1 changes are correctly implemented and verified. The sendBatch return value is now captured and logged for failed keys with structured logging. Three well-targeted unit tests cover the delegation path, the fallback path, and the failure logging path. All 9 acceptance criteria remain satisfied (500 tests passing, up from 497). All 5 constraints continue to be honored. Build succeeds. No critical, major, or minor issues found.

---

## Completion

**Completed:** 2026-02-12
**Total Commits:** 3
**Audit Cycles:** 3
**Review Cycles:** 2
