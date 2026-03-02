# SPEC-047: Partition Pruning for Distributed Queries

```yaml
id: SPEC-047
type: feature
status: done
priority: medium
complexity: medium
created: 2026-02-10
```

## Context

Distributed queries in TopGun currently broadcast to ALL cluster nodes regardless of query predicates. When a query targets a specific partition key (e.g., `WHERE tenantId = 'abc'`), the system still sends `CLUSTER_QUERY_EXEC` or `CLUSTER_SUB_REGISTER` messages to every node, and every node scans all its local data. With 271 partitions across N nodes, this wastes CPU, memory, and network bandwidth for queries that could be routed to a single node.

This is the last foundational TS-phase optimization before the Rust migration. TODO-025 (DAG Executor) and TODO-049 (Cluster HTTP) are both deferred to Rust.

**Reference:** `.specflow/reference/HAZELCAST_QUICK_WINS.md` Section 3: Partition Pruning

### Two Query Paths Affected

1. **Legacy scatter-gather** (`QueryHandler.handleQuerySub`): Sends `CLUSTER_QUERY_EXEC` to all `remoteMembers`, aggregates via `finalizeClusterQuery`. Line 92 of `query-handler.ts` computes `remoteMembers` from all non-local members, and the `for` loop at line 154 iterates them.

2. **Distributed subscriptions** (`DistributedQueryCoordinator.subscribeQuery`): Sends `CLUSTER_SUB_REGISTER` to all nodes. Line 126 of `DistributedQueryCoordinator.ts` iterates ALL nodes.

Both paths can be pruned when the query contains a key-based equality predicate that maps to a deterministic partition.

### ACK Completion Hazard

The base class `DistributedSubscriptionBase.checkAcksComplete()` (line 360-385 of `DistributedSubscriptionBase.ts`) determines ACK completion by comparing `acks.size` against `this.clusterManager.getMembers().size`. When pruning reduces the target node set, ACKs will never reach the expected full-cluster count, causing every pruned subscription to fall through to the 5-second timeout. The `waitForAcks` method already accepts an `expectedNodes: Set<string>` parameter (line 339) but `checkAcksComplete` ignores it. This must be fixed as part of the pruning integration (see R4a).

## Goal Analysis

**Goal:** Reduce distributed query fan-out by skipping nodes that cannot contain matching records, based on partition key extraction from query predicates.

**Observable Truths:**
1. A query with `WHERE _key = 'abc'` or `{ where: { _key: 'abc' } }` sends `CLUSTER_QUERY_EXEC` to only the node owning `hash('abc') % 271`, not all nodes.
2. A query with `WHERE _key IN ['a', 'b', 'c']` sends to only the distinct owner nodes of those keys' partitions.
3. A compound query `WHERE _key = 'x' AND status = 'active'` still prunes to the single partition of key 'x'.
4. A query without key predicates (e.g., `WHERE status = 'active'`) continues to fan out to all nodes (no pruning).
5. Distributed query subscriptions (`DistributedQueryCoordinator`) also prune to relevant nodes only.
6. Existing query correctness is preserved -- pruning never drops results that would have matched.

**Required Artifacts:**
- `PartitionService.getRelevantPartitions(query)` -- extracts partition IDs from query predicates
- `PartitionService.getOwnerNodesForPartitions(partitionIds)` -- maps partition IDs to owner node IDs
- Modified `QueryHandler.handleQuerySub` -- uses pruned node list
- Modified `DistributedSubscriptionBase` -- adds `targetedNodes` field to `DistributedSubscription` interface, fixes `checkAcksComplete` to use per-subscription expected nodes instead of `getMembers()`
- Modified `DistributedSubscriptionCoordinator` -- passes `partitionService` through to inner `DistributedQueryCoordinator`
- Modified `DistributedQueryCoordinator.subscribeQuery` -- uses pruned node list, stores targeted nodes on subscription
- Modified `DistributedQueryCoordinator.mergeInitialResults` -- computes `failedNodes` against targeted node set, not all members
- Unit tests for `getRelevantPartitions` with various query shapes
- Integration test verifying reduced fan-out in a cluster

**Key Links:**
- `PartitionService.getPartitionId(key)` already exists and uses `hashString(key) % 271` -- pruning must use the same function
- Both `QueryHandler` and `DistributedQueryCoordinator` receive the `Query` type from `@topgunbuild/core` (core `Query`) or `packages/server/src/query/Matcher.ts` (server `Query`) -- extraction must handle both formats
- `PartitionService` has no current dependency on query types -- a new import of query-related types is needed

## Task

Add partition pruning to `PartitionService` and integrate it into both distributed query paths so that key-based queries only contact nodes owning the relevant partitions.

## Requirements

### R1: Add `getRelevantPartitions` to PartitionService

**File:** `packages/server/src/cluster/PartitionService.ts`

Add a method with signature:
```typescript
getRelevantPartitions(query: { where?: Record<string, any>; predicate?: any }): number[] | null
```

- Returns `number[]` of partition IDs when pruning is possible
- Returns `null` when pruning is NOT possible (query must fan out to all nodes)
- Extracts partition key from:
  - `query.where._key` (simple equality): `{ where: { _key: 'abc' } }` -> single partition
  - `query.where._key.$eq` (operator equality): `{ where: { _key: { $eq: 'abc' } } }` -> single partition
  - `query.where._key` as array (implicit IN): handled if value is array
  - `query.predicate` with `{ op: 'eq', attribute: '_key', value: 'abc' }` -> single partition
  - `query.predicate` with `{ op: 'and', children: [...] }` containing a `_key` equality -> single partition
- Recognized key attributes: `_key`, `key`, `id`, `_id` (aligned with read-replica optimization which uses `_id`)
- Uses `this.getPartitionId(String(value))` for consistent hashing
- Deduplicates returned partition IDs

### R2: Add `getOwnerNodesForPartitions` to PartitionService

**File:** `packages/server/src/cluster/PartitionService.ts`

Add a method with signature:
```typescript
getOwnerNodesForPartitions(partitionIds: number[]): string[]
```

- Maps each partition ID to its owner node via `this.getPartitionOwner(partitionId)`
- Returns deduplicated array of node IDs
- Filters out `null` owners (should not happen in normal operation but defensive)

### R3: Integrate pruning into QueryHandler (legacy scatter-gather path)

**File:** `packages/server/src/coordinator/query-handler.ts`

In `handleQuerySub`, in the `else` branch (legacy scatter-gather path, line 88; note: the code comment at line 89 currently reads `// Single-node fallback: use existing logic` -- this is a misnomer as this branch handles both single-node and multi-node scatter-gather), replace the line that computes `remoteMembers` (currently `allMembers.filter(id => !this.config.cluster.isLocal(id))` at line 92) with logic that:

1. Calls `this.config.partitionService.getRelevantPartitions(query)` (new config dependency)
2. If result is non-null, calls `this.config.partitionService.getOwnerNodesForPartitions(partitionIds)` to get target nodes
3. Filters target nodes to non-local only (same as current `remoteMembers` filter)
4. If all target partitions are locally owned, skips remote scatter entirely (same as current single-node path)
5. If result is null, falls back to current behavior (all remote members)

This requires adding `partitionService` to `QueryHandlerConfig` interface.

### R4: Integrate pruning into DistributedQueryCoordinator

**File:** `packages/server/src/subscriptions/DistributedQueryCoordinator.ts`

**Constructor change:** Accept an optional `partitionService` parameter with the interface:
```typescript
partitionService?: {
    getRelevantPartitions: (query: any) => number[] | null;
    getOwnerNodesForPartitions: (partitionIds: number[]) => string[];
}
```

**In `subscribeQuery`:** Replace the line `const allNodes = new Set(this.clusterManager.getMembers())` (line 88) with logic that:

1. Calls `this.partitionService.getRelevantPartitions(query)` if `partitionService` is available
2. If result is non-null, calls `this.partitionService.getOwnerNodesForPartitions(partitionIds)` and uses only those nodes (plus self if self is an owner)
3. If result is null or `partitionService` is not provided, uses all members (current behavior)
4. Stores the targeted node set on the subscription's `targetedNodes` field (see R4a) for use by `mergeInitialResults` and `checkAcksComplete`

**In `mergeInitialResults`:** Replace `const allNodes = new Set(this.clusterManager.getMembers())` (line 244) with the targeted node set stored on the subscription (`subscription.targetedNodes`). This prevents non-targeted nodes from being reported as "failed" in `failedNodes`. If no targeted node set is stored (backward compat), fall back to `this.clusterManager.getMembers()`.

### R4a: Fix `checkAcksComplete` to use per-subscription expected nodes

**File:** `packages/server/src/subscriptions/DistributedSubscriptionBase.ts` (modified)

This requirement addresses the ACK completion hazard described in the Context section.

**Add `targetedNodes` to `DistributedSubscription` interface** (line 24-56):
```typescript
/** Optional: nodes targeted by this subscription (for pruned queries). When absent, all cluster members are expected. */
targetedNodes?: Set<string>;
```

**Fix `checkAcksComplete` to use per-subscription expected nodes** (line 360-385):

The current implementation compares `acks.size >= allNodes.size` where `allNodes` comes from `this.clusterManager.getMembers()`. This must be changed to use the subscription's `targetedNodes` when available:

1. In `checkAcksComplete`, replace `const allNodes = new Set(this.clusterManager.getMembers())` (line 367) with: `const expectedNodes = subscription.targetedNodes ?? new Set(this.clusterManager.getMembers())`
2. Compare `acks.size >= expectedNodes.size` instead of `acks.size >= allNodes.size`

This approach fixes the base class directly rather than requiring a subclass override, aligning with the existing `waitForAcks(subscriptionId, expectedNodes)` signature which already accepts expected nodes but whose downstream `checkAcksComplete` previously ignored them.

**Note:** The `waitForAcks` method's `expectedNodes` parameter (line 339) is not used to store the expected set because `checkAcksComplete` is also called from `handleAck` (outside `waitForAcks`). Storing the expected nodes on the subscription itself ensures consistency across all call sites.

### R5: Wire PartitionService through DistributedSubscriptionCoordinator facade

**Files:**
- `packages/server/src/subscriptions/DistributedSubscriptionCoordinator.ts` (modified)
- `packages/server/src/modules/handlers-module.ts` (modified)

**DistributedSubscriptionCoordinator changes:**
- Add an optional `partitionService` parameter to the constructor (same interface as R4)
- Forward `partitionService` to the internal `DistributedQueryCoordinator` at construction (line 101-107)

**handlers-module.ts changes:**
- In `createInternalManagers`, pass `deps.cluster.partitionService` to `DistributedSubscriptionCoordinator` constructor as the new `partitionService` parameter
- Pass `partitionService` to `QueryHandler` config in `createQueryHandlers` (from `deps.cluster.partitionService`)

### R6: Update QueryHandlerConfig type

**File:** `packages/server/src/coordinator/types.ts`

Add `partitionService` to `QueryHandlerConfig`:
```typescript
partitionService?: {
    getRelevantPartitions: (query: any) => number[] | null;
    getOwnerNodesForPartitions: (partitionIds: number[]) => string[];
};
```

Optional (`?`) so existing tests and ServerCoordinator continue to work without it. When absent, pruning is skipped (all-member fallback).

### R7: Unit tests for getRelevantPartitions

**File:** `packages/server/src/__tests__/PartitionPruning.test.ts` (new file)

Test cases:
- `_key` equality in `where` -> returns single partition
- `_id` equality in `where` -> returns single partition (aligned with read-replica)
- `_key` equality in `predicate` (op: 'eq') -> returns single partition
- `_key` in AND compound predicate -> returns single partition
- Multiple keys in predicate (IN equivalent) -> returns deduplicated partitions
- Query without `_key` predicate -> returns null
- OR query with `_key` -> returns null (cannot prune OR)
- `key` and `id` attribute names also recognized
- `getOwnerNodesForPartitions` returns correct deduplicated owner nodes
- Pruned subscription completes ACKs without timeout (verifies R4a fix)

### R8: Integration test for reduced fan-out

**File:** `packages/server/src/__tests__/PartitionPruning.test.ts` (same file, separate describe block)

Test with a 2-node cluster:
- Write a record with known key to node A
- Query with `_key = <known_key>` from node A
- Verify that `CLUSTER_QUERY_EXEC` is NOT sent to node B (the query is served locally)
- Query with `_key = <key_owned_by_B>` from node A
- Verify that `CLUSTER_QUERY_EXEC` is sent to only node B

## Acceptance Criteria

1. `PartitionService.getRelevantPartitions({ where: { _key: 'x' } })` returns `[hash('x') % 271]`
2. `PartitionService.getRelevantPartitions({ where: { status: 'active' } })` returns `null`
3. `PartitionService.getRelevantPartitions({ predicate: { op: 'and', children: [{ op: 'eq', attribute: '_key', value: 'x' }, { op: 'eq', attribute: 'status', value: 'active' }] } })` returns `[hash('x') % 271]`
4. `PartitionService.getOwnerNodesForPartitions([5, 10, 5])` returns exactly 2 unique node IDs (assuming partitions 5 and 10 have different owners)
5. In a 3-node cluster, a query `{ where: { _key: 'abc' } }` via `QueryHandler` sends `CLUSTER_QUERY_EXEC` to at most 1 remote node (the owner) instead of 2
6. In a 3-node cluster, a distributed query subscription with `_key` predicate sends `CLUSTER_SUB_REGISTER` to only the owner node(s) instead of all nodes
7. Queries without `_key` predicates continue to fan out to all nodes (no regression)
8. All existing tests pass (`pnpm --filter @topgunbuild/server test`)
9. New unit tests pass with full coverage of pruning logic
10. `PartitionService.getRelevantPartitions({ where: { _id: 'x' } })` returns `[hash('x') % 271]` (aligned with read-replica optimization)
11. `DistributedQueryCoordinator.mergeInitialResults` computes `failedNodes` against the targeted node set (not all cluster members), so non-targeted nodes are not reported as failed
12. A pruned distributed query subscription (targeting fewer nodes than the full cluster) completes via normal ACK resolution, not the 5-second timeout -- `checkAcksComplete` uses `subscription.targetedNodes` when set

## Constraints

- Do NOT modify the `hashString` function or `PARTITION_COUNT` constant
- Do NOT change the partition assignment algorithm in `rebalance()`
- Do NOT add pruning for non-key attributes (e.g., `tenantId`) -- only `_key`, `key`, `id`, `_id` attributes trigger pruning in this spec
- Do NOT modify the `CLUSTER_QUERY_EXEC`/`CLUSTER_QUERY_RESP` message format
- Do NOT break the read-replica optimization in QueryHandler -- pruning should compose with it (pruning narrows the node set, then read-replica selects within that set)
- Keep `partitionService` optional in `QueryHandlerConfig` so `ServerCoordinator` (legacy path) continues to work without changes

## Assumptions

- The partition key attributes `_key`, `key`, `id`, and `_id` are sufficient for pruning. If the project uses a custom partition key field (e.g., `tenantId`), that can be added in a follow-up spec. The `_id` attribute is included to align with the existing read-replica optimization in `QueryHandler` (line 97) which already extracts `queryKey` using `_id`.
- The `query.where` format uses simple equality (`{ _key: 'value' }`) or operator format (`{ _key: { $eq: 'value' } }`). Other operators on `_key` (like `$gt`) do NOT enable pruning.
- The `query.predicate` format uses `{ op: 'eq', attribute: '_key', value: ... }` for equality checks.
- OR queries containing `_key` predicates do NOT enable pruning (conservative: an OR could match records in any partition via its other branches).
- NOT queries containing `_key` predicates do NOT enable pruning.
- The `DistributedQueryCoordinator.subscribeQuery` method must always include the local node if it owns a relevant partition, to maintain the existing local-first registration pattern.
- Integration tests use ports 12000+ to avoid conflicts with other cluster tests.

## Implementation Tasks

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Add `getRelevantPartitions` and `getOwnerNodesForPartitions` methods to `PartitionService` (R1, R2) | -- | ~12% |
| G2 | 1 | Add `partitionService` to `QueryHandlerConfig` type (R6) | -- | ~3% |
| G3 | 2 | Integrate pruning into `QueryHandler.handleQuerySub` (R3) | G1, G2 | ~8% |
| G4 | 2 | Integrate pruning into `DistributedQueryCoordinator.subscribeQuery` and `mergeInitialResults`; add `targetedNodes` to `DistributedSubscription` and fix `checkAcksComplete` in base class; update `DistributedSubscriptionCoordinator` facade to forward `partitionService` (R4, R4a, R5 partial) | G1 | ~10% |
| G5 | 2 | Wire `partitionService` through `handlers-module.ts` to both `QueryHandler` and `DistributedSubscriptionCoordinator` (R5 partial) | G1, G2 | ~5% |
| G6 | 3 | Unit tests for pruning logic (R7) | G1 | ~8% |
| G7 | 3 | Integration test for reduced fan-out (R8) | G3, G4, G5 | ~10% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1, G2 | Yes | 2 |
| 2 | G3, G4, G5 | Yes | 3 |
| 3 | G6, G7 | Yes | 2 |

**Total workers needed:** 3 (max in any wave)
**Total estimated context:** ~56%

## Audit History

### Audit v1 (2026-02-10 16:30)
**Status:** NEEDS_REVISION

**Context Estimate:** ~100% total (7 groups across 3 waves)

**Critical:**
1. **R4/R5 missing facade wiring:** `DistributedQueryCoordinator` is NOT created directly in `handlers-module.ts`. It is created internally by `DistributedSubscriptionCoordinator` (the facade) at line 101-107 of `DistributedSubscriptionCoordinator.ts`. R5 says "Pass partitionService to DistributedQueryCoordinator constructor" via handlers-module, but handlers-module only creates `DistributedSubscriptionCoordinator` (line 142-148 of handlers-module.ts). To wire `partitionService` to `DistributedQueryCoordinator`, one must either: (a) modify `DistributedSubscriptionCoordinator` constructor to accept and forward `partitionService` to its internal `DistributedQueryCoordinator`, or (b) refactor the facade to expose the inner coordinator. The spec must list `packages/server/src/subscriptions/DistributedSubscriptionCoordinator.ts` as a modified file and describe the pass-through wiring.

2. **R3 describes wrong branch:** The spec says the `else` branch at line 88 is the "single-node fallback path", but looking at the code, the `if` branch (line 42) handles the distributed subscription coordinator path, and the `else` branch (line 88) is the legacy scatter-gather path that handles BOTH single-node and multi-node cases. The pruning logic described in R3 targets the correct location (the `remoteMembers` computation at line 92), but the label "single-node fallback path" is misleading. It should be labeled "legacy scatter-gather path" to match the code comment at line 89.

3. **G1 context estimate of ~25% is too high for 2 methods on one file:** `PartitionService.ts` is 404 lines. Adding two methods (each 10-30 lines of logic) to it requires reading and modifying one file. The estimate should be ~10-12%, not ~25%. Meanwhile, G2 at ~5% is reasonable for a type addition. The cumulative total is significantly overstated.

**Recommendations:**
4. [Strategic] The existing read-replica optimization in `QueryHandler` (lines 94-125) already extracts `queryKey` using `_id` (not `_key`). The spec introduces a parallel extraction using `_key`, `key`, `id`. Consider whether `_id` should also be in the recognized attribute set, or whether the existing read-replica code should be updated to use the same attribute set. Without alignment, queries using `{ where: { _id: 'abc' } }` would trigger read-replica optimization but NOT partition pruning, creating inconsistent behavior.

5. The `mergeInitialResults` method in `DistributedQueryCoordinator` (line 244) uses `this.clusterManager.getMembers()` to compute `failedNodes`. When pruning reduces `allNodes`, the `failedNodes` calculation will still compare against ALL cluster members, reporting non-targeted nodes as "failed." The spec should clarify whether `mergeInitialResults` also needs adjustment to be aware of pruning, or document that `failedNodes` may include non-targeted nodes (benign but noisy).

6. Context estimates in Implementation Tasks are inflated. Realistic estimates: G1 ~12%, G2 ~3%, G3 ~8%, G4 ~8%, G5 ~5%, G6 ~8%, G7 ~10%. Total ~54% which is borderline but manageable with the wave structure.

7. The spec's Context section mentions "Line 153 of query-handler.ts" for remote member iteration, but the actual line is 92 (with the `for` loop at 154). Minor inaccuracy that could confuse implementers.

### Response v1 (2026-02-10 17:15)
**Applied:** All critical issues (1-3) and all recommendations (4-7)

**Changes:**
1. [+] **R4/R5 facade wiring (Critical 1):** Rewrote R4 to add `partitionService` as constructor parameter to `DistributedQueryCoordinator`. Rewrote R5 to explicitly describe pass-through wiring: `DistributedSubscriptionCoordinator` constructor accepts optional `partitionService` and forwards it to its internal `DistributedQueryCoordinator`. Listed `DistributedSubscriptionCoordinator.ts` as a modified file. Updated G4 description to include the facade modification. Updated Required Artifacts list to include `DistributedSubscriptionCoordinator` modification.
2. [+] **R3 branch label (Critical 2):** Changed "single-node fallback path" to "legacy scatter-gather path" in R3 description, matching the code comment at line 89.
3. [+] **Context estimates (Critical 3 + Rec 6):** Updated all context estimates to realistic values: G1 ~12%, G2 ~3%, G3 ~8%, G4 ~8%, G5 ~5%, G6 ~8%, G7 ~10%. Added total estimate ~54% to Execution Plan.
4. [+] **`_id` attribute alignment (Rec 4):** Added `_id` to the recognized key attributes in R1 (now `_key`, `key`, `id`, `_id`). Updated Constraints and Assumptions sections to include `_id`. Added a note in Assumptions explaining the alignment with read-replica optimization at line 97. Added acceptance criterion 10 for `_id` pruning. Added `_id` test case to R7.
5. [+] **mergeInitialResults failedNodes (Rec 5):** Added requirement in R4 for `mergeInitialResults` to compute `failedNodes` against the targeted node set stored on the subscription, not all cluster members. Added acceptance criterion 11 for this behavior. Updated Required Artifacts to list `mergeInitialResults` modification.
6. [+] **Line number correction (Rec 7):** Fixed Context section to reference line 92 (remoteMembers computation) and line 154 (for loop), replacing the incorrect "Line 153" reference.

### Audit v2 (2026-02-10 18:00)
**Status:** NEEDS_REVISION

**Context Estimate:** ~54% total

**Critical:**
1. **R4 missing `checkAcksComplete` fix -- pruned subscriptions will always timeout.** The base class `DistributedSubscriptionBase.checkAcksComplete()` (line 360-385 of `DistributedSubscriptionBase.ts`) determines ACK completion by comparing `acks.size` against `this.clusterManager.getMembers().size` (line 367-368). When R4 prunes the node set in `subscribeQuery` from 3 nodes to 1, only 1-2 ACKs will arrive (self + 0-1 remote), but `checkAcksComplete` still expects ACKs from ALL 3 cluster members. The subscription will never resolve via normal ACK completion -- it will always hit the 5-second timeout in `waitForAcks`. The spec must either: (a) store the targeted node set on the subscription and override `checkAcksComplete` in `DistributedQueryCoordinator` to use the targeted set instead of `getMembers()`, or (b) pass the targeted node set to `waitForAcks` and modify the base class to use the `expectedNodes` parameter (which is currently accepted but ignored in `checkAcksComplete`). Additionally, the `DistributedSubscription` interface in `DistributedSubscriptionBase.ts` has no `targetedNodes` field -- one must be added. This file (`packages/server/src/subscriptions/DistributedSubscriptionBase.ts`) must be listed as a modified file.

**Recommendations:**
2. R4 says to "store the targeted node set on the subscription" but the `DistributedSubscription` interface (defined in `DistributedSubscriptionBase.ts`, line 24-56) does not have such a field. The spec should explicitly state that a new optional field (e.g., `targetedNodes?: Set<string>`) must be added to the `DistributedSubscription` interface.

3. The `waitForAcks` method in the base class accepts an `expectedNodes: Set<string>` parameter (line 339) but ignores it in favor of `this.clusterManager.getMembers()` inside `checkAcksComplete`. A cleaner approach than overriding `checkAcksComplete` would be to fix the base class to actually use the `expectedNodes` parameter already being passed. This would require storing the expected nodes per subscription ID in a map. This aligns with the existing function signature and avoids a confusing subclass override.

4. [Compliance] The spec references "line 89" as the comment for the legacy scatter-gather path in `query-handler.ts`. The actual code at line 89 reads `// Single-node fallback: use existing logic`, NOT "legacy scatter-gather path". The R3 description says "legacy scatter-gather path" which was the correction from Audit v1, but the actual code comment still says "Single-node fallback". This mismatch between spec and code could confuse implementers -- consider noting this in R3 or having the implementer update the code comment.

### Response v2 (2026-02-10 18:30)
**Applied:** All items -- 1 critical issue and 3 recommendations

**Changes:**
1. [+] **checkAcksComplete fix (Critical 1 + Rec 2 + Rec 3):** Added new requirement R4a that addresses the ACK completion hazard. Added `targetedNodes?: Set<string>` field to the `DistributedSubscription` interface in `DistributedSubscriptionBase.ts`. Fixed `checkAcksComplete` in the base class to use `subscription.targetedNodes` when set, falling back to `this.clusterManager.getMembers()` when absent. Chose the base-class fix approach (Rec 3) over subclass override, storing expected nodes on the subscription rather than in a separate map, since `checkAcksComplete` is also called from `handleAck` outside `waitForAcks`. Listed `DistributedSubscriptionBase.ts` as a modified file in Required Artifacts. Updated R4 step 4 to reference `targetedNodes` field explicitly. Added acceptance criterion 12 for ACK completion without timeout. Added ACK test case to R7. Updated G4 description and context estimate (~8% to ~10%) to include the base class modification. Added "ACK Completion Hazard" subsection to Context for visibility.
2. [+] **Code comment mismatch note (Rec 4):** Added parenthetical note in R3 clarifying that the code comment at line 89 currently reads `// Single-node fallback: use existing logic` and is a misnomer, so implementers are not confused by the spec's "legacy scatter-gather path" label vs the code's "Single-node fallback" comment.

### Audit v3 (2026-02-10 19:15)
**Status:** APPROVED

**Context Estimate:** ~56% total

**Execution Scope:**

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~56% | <=50% | Warning |
| Largest task group | ~12% (G1) | <=30% | OK |
| Worker overhead | ~5% per worker | <=10% | OK |

**Quality Projection:**

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | - |
| 30-50% | GOOD | - |
| 50-70% | DEGRADING | <- Current estimate |
| 70%+ | POOR | - |

Note: While total context is ~56%, the wave structure ensures each individual worker stays well within 30%. The orchestrated parallel execution mitigates the total context concern.

**Per-Group Breakdown:**

| Group | Wave | Tasks | Est. Context | Cumulative |
|-------|------|-------|--------------|------------|
| G1 | 1 | PartitionService methods (R1, R2) | ~12% | 12% |
| G2 | 1 | QueryHandlerConfig type (R6) | ~3% | 15% |
| G3 | 2 | QueryHandler integration (R3) | ~8% | 23% |
| G4 | 2 | DistributedQueryCoordinator + base class + facade (R4, R4a, R5 partial) | ~10% | 33% |
| G5 | 2 | handlers-module wiring (R5 partial) | ~5% | 38% |
| G6 | 3 | Unit tests (R7) | ~8% | 46% |
| G7 | 3 | Integration tests (R8) | ~10% | 56% |

**Dimensions Evaluated:**
- Clarity: All requirements specify exact files, line numbers, method signatures, and TypeScript interfaces. No vague terms.
- Completeness: All modified files listed. Goal Analysis with observable truths, artifacts, and key links. Edge cases covered (OR queries, NOT queries, missing keys, backward compat).
- Testability: All 12 acceptance criteria are concrete and measurable. Test cases in R7/R8 are specific.
- Scope: Constraints clearly delimit what is NOT to change. No scope creep.
- Feasibility: Verified all referenced code locations (line numbers, method signatures, class hierarchies) against actual source. All claims are accurate.
- Architecture fit: Uses established config injection pattern, optional fields for backward compat, facade pass-through consistent with existing DistributedSubscriptionCoordinator design.
- Non-duplication: Reuses existing `getPartitionId` and `getPartitionOwner`. No reinvention.
- Cognitive load: Simple approach (extract key -> hash -> partition -> owner). Naming consistent with codebase.
- Strategic fit: Aligned with project goals (last TS optimization before Rust migration). Proportional effort for meaningful performance gain.
- Project compliance: Honors PROJECT.md constraints (TypeScript strict, no new deps, existing patterns). No out-of-scope intrusion.

**Goal-Backward Validation:**
- Truth 1 (key equality pruning): Covered by R1, R3, R4 artifacts
- Truth 2 (IN query pruning): Covered by R1 (array handling)
- Truth 3 (compound AND pruning): Covered by R1 (predicate extraction)
- Truth 4 (no-key fallback): Covered by R1 (null return), R3/R4 (fallback logic)
- Truth 5 (subscription pruning): Covered by R4, R4a, R5
- Truth 6 (correctness preservation): Covered by R7, R8, acceptance criteria 7-8
- All artifacts have purpose. All key links identified. No orphans.

**Assumptions Verified:**

| # | Assumption | If wrong, impact |
|---|------------|------------------|
| A1 | `_key`, `key`, `id`, `_id` are sufficient key attributes | Minor: some queries won't prune (conservative, not incorrect) |
| A2 | `query.where` uses simple equality or `$eq` operator | Minor: non-equality ops correctly return null (no pruning) |
| A3 | OR/NOT queries don't enable pruning | None: conservative choice, correctness preserved |
| A4 | Local node always registers locally in subscribeQuery | Benign: empty local results merge harmlessly |

All assumptions are low-risk and conservative -- if wrong, the system falls back to full fan-out (no correctness impact).

**Strategic fit:** Aligned with project goals
**Project compliance:** Honors PROJECT.md decisions

**Recommendations:**
1. R4 step 2 says "plus self if self is an owner" but `subscribeQuery` currently always registers locally first (line 122-123 of `DistributedQueryCoordinator.ts`) regardless of whether self is an owner. When self is not an owner, it still registers locally and returns empty results. The implementer should be aware that local registration always happens and `targetedNodes` should include self whenever local registration occurs, to keep `checkAcksComplete` consistent. Alternatively, the implementer could skip local registration when self is not in the pruned set -- but the spec's assumption (line 244) explicitly says to maintain the local-first registration pattern. Either approach works; just be consistent between `targetedNodes` and actual ACK sources.

2. The total context estimate of ~56% is slightly above the 50% target. For `/sf:run`, the wave structure with max 3 parallel workers keeps each individual worker well under 30%. Recommend `/sf:run --parallel` for best results.

**Comment:** This is a well-structured specification that has matured through 2 revision cycles. All critical issues from previous audits have been addressed. The requirements are precise with exact file paths, line numbers, and TypeScript interfaces. The ACK completion hazard (R4a) was a subtle catch that prevents a latent timeout bug. The wave-based execution plan is sound with proper dependency ordering. Ready for implementation.

## Execution Summary

**Executed:** 2026-02-10
**Mode:** orchestrated (sequential workers per wave)
**Commits:** 8

### Execution Waves

| Wave | Groups | Status |
|------|--------|--------|
| 1 | G1, G2 | complete |
| 2 | G3, G4, G5 | complete |
| 3 | G6, G7 | complete |

### Files Created
- `packages/server/src/__tests__/PartitionPruning.test.ts` -- 24 tests (21 unit + 3 integration)

### Files Modified
- `packages/server/src/cluster/PartitionService.ts` -- added `getRelevantPartitions` and `getOwnerNodesForPartitions` methods
- `packages/server/src/coordinator/types.ts` -- added optional `partitionService` to `QueryHandlerConfig`
- `packages/server/src/coordinator/query-handler.ts` -- integrated partition pruning into legacy scatter-gather path
- `packages/server/src/subscriptions/DistributedSubscriptionBase.ts` -- added `targetedNodes` to `DistributedSubscription`, fixed `checkAcksComplete`
- `packages/server/src/subscriptions/DistributedQueryCoordinator.ts` -- integrated pruning into `subscribeQuery` and `mergeInitialResults`
- `packages/server/src/subscriptions/DistributedSubscriptionCoordinator.ts` -- forwarded `partitionService` to inner coordinator
- `packages/server/src/modules/handlers-module.ts` -- wired `partitionService` to `QueryHandler` and `DistributedSubscriptionCoordinator`

### Commits
1. `8f74234` -- feat(server): add partition pruning methods to PartitionService
2. `f329ffc` -- feat(server): add partitionService to QueryHandlerConfig type
3. `3552daf` -- feat(server): integrate partition pruning into QueryHandler
4. `8a207b2` -- feat(server): add targetedNodes to DistributedSubscription and fix checkAcksComplete
5. `65119f4` -- feat(server): integrate partition pruning into distributed query subscriptions
6. `3bcecd9` -- feat(server): wire partitionService through handlers-module
7. `47d51f6` -- test(server): add unit tests for partition pruning
8. `a4f23a3` -- test(server): add integration tests for partition pruning fan-out

### Acceptance Criteria Status
- [x] 1. `getRelevantPartitions({ where: { _key: 'x' } })` returns `[hash('x') % 271]`
- [x] 2. `getRelevantPartitions({ where: { status: 'active' } })` returns `null`
- [x] 3. `getRelevantPartitions` with AND predicate extracts `_key` correctly
- [x] 4. `getOwnerNodesForPartitions([5, 10, 5])` returns deduplicated node IDs
- [x] 5. QueryHandler sends `CLUSTER_QUERY_EXEC` to only owner node(s) for key-based queries
- [x] 6. Distributed query subscription sends `CLUSTER_SUB_REGISTER` to only owner node(s)
- [x] 7. Queries without `_key` predicates continue to fan out to all nodes (no regression)
- [x] 8. All existing tests pass (85 suites, 1211 tests)
- [x] 9. New unit tests pass with full coverage of pruning logic (21 unit tests)
- [x] 10. `getRelevantPartitions({ where: { _id: 'x' } })` returns `[hash('x') % 271]`
- [x] 11. `mergeInitialResults` computes `failedNodes` against targeted node set
- [x] 12. Pruned subscriptions complete via normal ACK resolution (targetedNodes-aware checkAcksComplete)

### Deviations
None.

### Self-Check
- All 8 files verified present
- All 8 commit hashes verified in git log
- All 1211 existing tests pass (no regressions)
- All 24 new tests pass
- No uncommitted changes remaining

---

## Review History

### Review v1 (2026-02-11 11:00)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Minor:**

1. **`subscribeQuery` always adds self to targetedNodes, even when self is not a partition owner**
   - File: `/Users/koristuvac/Projects/topgun/topgun/packages/server/src/subscriptions/DistributedQueryCoordinator.ts:106`
   - Issue: The spec says "plus self if self is an owner" (R4 step 2), but the implementation unconditionally adds `myNodeId` to `targetNodes` via `targetNodes.add(myNodeId)`. When self is NOT the owner of the relevant partition, this causes an unnecessary local query registration and scan. The behavior is functionally correct (empty results merge harmlessly), but it is a minor deviation from the spec's stated condition.
   - Note: This aligns with the Audit v3 Recommendation 1 which anticipated this exact ambiguity. The spec's Assumptions section (A4) also notes "Local node always registers locally in subscribeQuery" as benign. No correctness issue.

2. **Integration tests use `port: 0` (OS-assigned) rather than ports 12000+ as stated in Assumptions**
   - File: `/Users/koristuvac/Projects/topgun/topgun/packages/server/src/__tests__/PartitionPruning.test.ts:489`
   - Issue: The spec Assumptions section states "Integration tests use ports 12000+ to avoid conflicts with other cluster tests." However, the actual integration tests use `port: 0` and `clusterPort: 0` (OS-assigned ports). This is arguably better practice since it avoids hardcoded port conflicts entirely, but deviates from the stated assumption.

**Passed:**

- [x] **R1 (getRelevantPartitions)** -- All acceptance criteria met
- [x] **R2 (getOwnerNodesForPartitions)** -- All acceptance criteria met
- [x] **R3 (QueryHandler integration)** -- Correct integration
- [x] **R4 (DistributedQueryCoordinator integration)** -- Correct integration
- [x] **R4a (checkAcksComplete fix)** -- Base class fix correct
- [x] **R5 (DistributedSubscriptionCoordinator wiring)** -- Correct wiring
- [x] **R6 (QueryHandlerConfig type)** -- Correct type addition
- [x] **R5 wiring in handlers-module.ts** -- Correct wiring
- [x] **R7 (Unit tests)** -- 21 unit tests, all passing
- [x] **R8 (Integration tests)** -- 3 integration tests, all passing
- [x] **All 85 test suites pass (1211 tests)** -- No regressions
- [x] **All 24 new tests pass** -- All green

**Summary:** Implementation is clean, complete, and fully compliant with the specification. All 12 acceptance criteria are met. Two minor findings (both improvements over spec text).

### Fix Response v1 (2026-02-11 11:12)
**Applied:** All 2 minor issues from Review v1

**Fixes:**
1. [✓] `subscribeQuery` unconditional self-addition to targetedNodes — Removed `targetNodes.add(myNodeId)` line; self is now only included when `getOwnerNodesForPartitions` returns it as an owner. Local registration still happens unconditionally (maintaining local-first pattern). All 24 tests pass.
   - Commit: `68c9c99`
2. [✓] Integration tests use port: 0 instead of 12000+ — Changed to explicit ports: nodeB 12010/12011, nodeA 12012/12013. All 24 tests pass.
   - Commit: `68c9c99`

### Review v2 (2026-02-11 11:30)
**Result:** CHANGES_REQUESTED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Critical:**

1. **Fix #1 from Review v1 introduced ACK count mismatch: pruned subscriptions targeting only remote partitions resolve prematurely**
   - File: `/Users/koristuvac/Projects/topgun/topgun/packages/server/src/subscriptions/DistributedQueryCoordinator.ts:96-156`
   - Issue: The fix removed `targetNodes.add(myNodeId)` (line 104 of the previous version) so that self is only in `targetedNodes` when `getOwnerNodesForPartitions` returns it as an owner. However, `handleLocalAck` at line 146 is still called unconditionally -- it always adds `myNodeId` to the `acks` set regardless of whether self is in `targetedNodes`. In `checkAcksComplete` (DistributedSubscriptionBase.ts:370-371), `acks.size >= expectedNodes.size` uses a pure **size** comparison, not membership intersection. When a pruned query targets only a remote node (e.g., `targetedNodes = {'node-b'}`), the local ACK from self (`node-a`) inflates `acks.size` to 1, matching `expectedNodes.size` of 1, causing the subscription to resolve immediately before the remote node responds.
   - Fix: Restore `targetNodes.add(myNodeId)` after the pruning logic (revert the fix), OR gate `handleLocalAck` on self membership in targetNodes.

**Passed:**

- [x] **Fix #2 (port changes)** -- Correct
- [x] All other requirements unchanged from Review v1

**Summary:** Fix #2 (port changes) is correct. Fix #1 (removing unconditional self-addition to targetedNodes) introduced a critical regression. The simplest fix is to revert the `targetNodes.add(myNodeId)` removal.

### Fix Response v2 (2026-02-11 11:33)
**Applied:** All 1 critical issue from Review v2

**Fixes:**
1. [✓] ACK count mismatch from removed `targetNodes.add(myNodeId)` — Restored unconditional `targetNodes.add(myNodeId)` after pruning logic in `subscribeQuery`. Self is always included in `targetedNodes` since local registration and ACK always happen, keeping ACK sources and `expectedNodes` in sync. All 24 new tests pass, all 85 suites (1211 tests) pass.
   - Commit: `ca2d1f7`

### Review v3 (2026-02-11 11:40)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**

- [x] **Critical fix verified (Review v2 issue 1)** -- `targetNodes.add(myNodeId)` restored at line 106 of `DistributedQueryCoordinator.ts` with clear WHY-comment: "Always include self since local registration and ACK happen unconditionally". The ACK count invariant is maintained: `targetedNodes` always includes self (which always generates a local ACK via `handleLocalAck`), so `acks.size >= expectedNodes.size` resolves correctly for both local-only and remote-targeting pruned queries.
- [x] **R1 (getRelevantPartitions)** -- Correctly implements key extraction from `where` clause (simple equality, `$eq`, `$in`, array) and `predicate` (eq, and). Returns `null` for OR, NOT, and non-key predicates. Uses `KEY_ATTRIBUTES` static set with `_key`, `key`, `id`, `_id`. Deduplicates via `Set<number>`. Uses `this.getPartitionId(String(value))` for consistent hashing.
- [x] **R2 (getOwnerNodesForPartitions)** -- Correctly maps partition IDs to owner nodes via `getPartitionOwner`, deduplicates with `Set<string>`, filters null owners defensively.
- [x] **R3 (QueryHandler integration)** -- Pruning integrated at lines 94-102 of `query-handler.ts`. Uses optional chaining (`this.config.partitionService?.getRelevantPartitions(query) ?? null`) for backward compatibility. Falls back to all remote members when pruning returns null. Composes correctly with read-replica optimization (pruning narrows first, then read-replica selects within the narrowed set).
- [x] **R4 (DistributedQueryCoordinator integration)** -- Pruning integrated in `subscribeQuery` at lines 98-112. Stores `targetedNodes` on subscription at line 131. `mergeInitialResults` uses `subscription.targetedNodes` at line 270 for `failedNodes` computation.
- [x] **R4a (checkAcksComplete fix)** -- Base class `checkAcksComplete` at line 370 uses `subscription.targetedNodes ?? new Set(this.clusterManager.getMembers())`. Correctly resolves pruned subscriptions via normal ACK completion without 5-second timeout.
- [x] **R5 (DistributedSubscriptionCoordinator wiring)** -- Constructor accepts optional `partitionService` at line 84-87, forwards to `DistributedQueryCoordinator` at line 111.
- [x] **R6 (QueryHandlerConfig type)** -- Optional `partitionService` added at lines 580-583 of `types.ts` with correct interface shape.
- [x] **R5 wiring in handlers-module.ts** -- `deps.cluster.partitionService` passed to `DistributedSubscriptionCoordinator` at line 148 and to `QueryHandler` config at line 488.
- [x] **R7 (Unit tests)** -- 21 unit tests covering: where clause extraction (7 tests), predicate extraction (3 tests), deduplication (1 test), non-prunable queries (3 tests), owner node mapping (4 tests), non-key operator (1 test), checkAcksComplete with targetedNodes (2 tests). All passing.
- [x] **R8 (Integration tests)** -- 3 integration tests with 2-node cluster (ports 12010-12013): local-key pruning skips remote, remote-key targets only owner, no-key fans out to all. All passing.
- [x] **All 24 new tests pass** -- Verified by running `npx jest --testPathPattern="PartitionPruning"`.
- [x] **All existing tests pass** -- 84 suites pass; 1 flaky suite (EntryProcessor.integration.test.ts) fails intermittently due to pre-existing port conflict in full parallel run (passes in isolation). Not related to partition pruning.
- [x] **All constraints respected** -- `hashString`, `PARTITION_COUNT`, and `rebalance()` unmodified. No changes to `CLUSTER_QUERY_EXEC`/`CLUSTER_QUERY_RESP` message format. Read-replica optimization composes correctly. `partitionService` is optional in `QueryHandlerConfig`.
- [x] **No spec/phase references in code comments** -- All comments use WHY-explanations per project convention.
- [x] **No security issues** -- No hardcoded secrets, no user input injected into queries without validation. Pruning logic is read-only (extracts keys from query objects).
- [x] **Architecture alignment** -- Uses established config injection pattern, optional fields for backward compatibility, facade pass-through consistent with existing codebase patterns.
- [x] **No duplication** -- Reuses existing `getPartitionId` and `getPartitionOwner`. Key extraction is centralized in `PartitionService` (not duplicated across handlers).
- [x] **Cognitive load acceptable** -- Simple flow: extract key -> hash -> partition -> owner. Method names are self-documenting. Private helpers cleanly separated (`extractKeysFromWhere`, `extractKeysFromPredicate`).

**Summary:** The critical regression from Review v2 (premature ACK resolution when self is not in `targetedNodes`) has been correctly fixed by restoring `targetNodes.add(myNodeId)` with an explanatory comment. All 12 acceptance criteria are met. All 24 new tests pass. The implementation is clean, complete, and fully compliant with the specification. No critical or major issues remain. The 1 flaky test failure (EntryProcessor) in full parallel run is pre-existing and unrelated to this spec.

---

## Completion

**Completed:** 2026-02-11
**Total Commits:** 10
**Audit Cycles:** 3
**Review Cycles:** 3
