> **SPLIT:** This specification was decomposed into:
> - SPEC-003a: Extract BroadcastHandler from ServerCoordinator
> - SPEC-003b: Extract GCHandler from ServerCoordinator
> - SPEC-003c: Extract ClusterEventHandler from ServerCoordinator
> - SPEC-003d: Extract Additional Handlers to Reach 1500 Line Target
>
> See child specifications for implementation.

---
id: SPEC-003
type: refactor
status: split
priority: high
complexity: large
created: 2026-01-25
split_date: 2026-01-25
children: [SPEC-003a, SPEC-003b, SPEC-003c, SPEC-003d]
---

# Extract Handlers from ServerCoordinator to Under 1500 Lines

## Context

ServerCoordinator.ts is currently 3841 lines, making it difficult to navigate, test, and maintain. The file has already had 17+ handlers extracted to `packages/server/src/coordinator/` using a dependency injection pattern, but significant responsibilities remain embedded in the main class.

### Prior Discussion

Reference: PRE-001 (ServerCoordinator.ts Extraction)

Key decisions from discussion:
- **Target:** Reduce to under 1500 lines (from 3841)
- **Strategy:** Handler extraction pattern with dependency injection
- **Compatibility:** Strict - all public methods remain on ServerCoordinator
- **Priority:** BroadcastHandler, GCHandler, ClusterEventHandler first
- **Location:** `packages/server/src/coordinator/` folder
- **Testing:** All existing tests must pass unchanged

### Why This Matters

- Large files are harder to review, test, and reason about
- The existing extraction pattern is proven and consistent
- Better separation enables focused unit testing of each handler

## Goal Statement

Reduce ServerCoordinator.ts to under 1500 lines by extracting BroadcastHandler, GCHandler, ClusterEventHandler, and additional handlers as needed while maintaining all public API contracts.

## Observable Truths (when complete)

1. ServerCoordinator.ts is under 1500 lines (measured by `wc -l`)
2. All existing tests pass (`pnpm test` in server package)
3. New handler files exist in `packages/server/src/coordinator/`
4. Public methods on ServerCoordinator delegate to extracted handlers
5. Handler pattern matches existing extractors (Config interface, dependency injection)
6. No changes to external API (client-facing WebSocket protocol unchanged)

## Task

### Phase 1: Extract BroadcastHandler (~180 lines extracted)

Extract broadcast-related methods to `packages/server/src/coordinator/broadcast-handler.ts`:

**Methods to extract:**
- `broadcast(message, excludeClientId?)` - Single event broadcast
- `broadcastBatch(events, excludeClientId?)` - Batched broadcast with role-based serialization caching
- `broadcastBatchSync(events, excludeClientId?)` - Synchronous batched broadcast for backpressure
- `getClientRoleSignature(client)` - Helper for role-based grouping (can be private in handler)

**Dependencies required:**
- ConnectionManager (for getClients)
- SecurityManager (for filterObject)
- QueryRegistry (for getSubscribedClientIds)
- MetricsService (for tracking)
- HLC (for timestamps)
- serialize function from @topgunbuild/core

### Phase 2: Extract GCHandler (~360 lines extracted)

Extract garbage collection methods to `packages/server/src/coordinator/gc-handler.ts`:

**Methods to extract:**
- `startGarbageCollection()` - Starts the GC interval
- `reportLocalHlc()` - Reports local HLC to leader
- `handleGcReport(nodeId, minHlc)` - Leader processes GC reports
- `performGarbageCollection(olderThan)` - Executes the actual GC

**State to manage:**
- `gcInterval: NodeJS.Timeout`
- `gcReports: Map<string, Timestamp>`
- `GC_INTERVAL_MS` and `GC_AGE_MS` constants

**Dependencies required:**
- StorageManager (for getMaps)
- ConnectionManager (for getClients)
- ClusterManager (for getMembers, send, isLocal, config)
- PartitionService (for isRelated, getPartitionId)
- ReplicationPipeline (for replicate)
- MerkleTreeManager (for updateRecord)
- QueryRegistry (for processChange)
- HLC (for now, compare)
- Storage (for store, delete, deleteAll)
- Broadcast callback (to broadcast GC_PRUNE)
- MetricsService

### Phase 3: Extract ClusterEventHandler (~200 lines extracted)

Extract cluster message handling to `packages/server/src/coordinator/cluster-event-handler.ts`:

**Methods to extract:**
- `setupClusterListeners()` - Sets up all cluster.on('message') handlers
- `handleClusterEvent(payload)` - Processes CLUSTER_EVENT messages

**Message types handled:**
- OP_FORWARD
- CLUSTER_EVENT
- CLUSTER_QUERY_EXEC / CLUSTER_QUERY_RESP
- CLUSTER_GC_REPORT / CLUSTER_GC_COMMIT
- CLUSTER_LOCK_REQ / CLUSTER_LOCK_RELEASE / CLUSTER_LOCK_GRANTED / CLUSTER_LOCK_RELEASED
- CLUSTER_CLIENT_DISCONNECTED
- CLUSTER_TOPIC_PUB
- CLUSTER_MERKLE_ROOT_REQ / CLUSTER_MERKLE_ROOT_RESP
- CLUSTER_REPAIR_DATA_REQ / CLUSTER_REPAIR_DATA_RESP

**Dependencies required:**
- ClusterManager
- PartitionService
- LockManager
- TopicManager
- RepairScheduler
- ConnectionManager
- StorageManager
- QueryRegistry
- MetricsService
- GCHandler (for handleGcReport, performGarbageCollection)
- Various callbacks for processLocalOp, executeLocalQuery, finalizeClusterQuery, getLocalRecord

### Phase 4: Extract Additional Handlers (as needed to reach target)

If Phases 1-3 don't achieve the 1500-line target, extract additional handlers:

**HeartbeatHandler (~80 lines):**
- `startHeartbeatCheck()`
- `handlePing(client, timestamp)`
- `evictDeadClients()`
- Constants: `CLIENT_HEARTBEAT_TIMEOUT_MS`, `CLIENT_HEARTBEAT_CHECK_INTERVAL_MS`

**BatchProcessingHandler (~170 lines):**
- `processBatchAsync(ops, clientId)`
- `processBatchSync(ops, clientId)`
- `forwardOpAndWait(op, owner)`
- `processLocalOpForBatch(op, clientId, batchedEvents)`

**WriteConcernHandler (~250 lines):**
- `getEffectiveWriteConcern(opLevel, batchLevel)`
- `stringToWriteConcern(value)`
- `processBatchAsyncWithWriteConcern(...)`
- `processBatchSyncWithWriteConcern(...)`
- `processLocalOpWithWriteConcern(...)`
- `persistOpSync(op)`
- `persistOpAsync(op)`

**QueryConversionHandler (~100 lines):**
- `convertToCoreQuery(query)`
- `predicateToCoreQuery(predicate)`
- `convertOperator(op)`

## Acceptance Criteria

1. [ ] `ServerCoordinator.ts` is under 1500 lines (verified by `wc -l packages/server/src/ServerCoordinator.ts`)
2. [ ] All existing tests pass: `pnpm --filter @topgunbuild/server test`
3. [ ] No public API changes on ServerCoordinator (all existing public methods still work)
4. [ ] New handlers follow existing pattern in coordinator/ folder
5. [ ] Each new handler has a Config interface in types.ts
6. [ ] New handlers are exported from coordinator/index.ts
7. [ ] TypeScript compiles without errors: `pnpm --filter @topgunbuild/server build`

## Constraints

1. **DO NOT** change the WebSocket message protocol
2. **DO NOT** modify test files (tests must pass as-is)
3. **DO NOT** change public method signatures on ServerCoordinator
4. **DO NOT** introduce new dependencies - only use existing imports
5. **DO** follow the existing handler pattern (see query-handler.ts, operation-handler.ts)
6. **DO** use dependency injection via Config objects
7. **DO** keep handlers focused on a single responsibility

## Assumptions

1. The existing handler extraction pattern (Config interface + dependency injection) is the correct approach
2. Extracting ~1000 lines through BroadcastHandler, GCHandler, and ClusterEventHandler will be the primary reduction; additional handlers may be needed
3. Integration tests cover the critical paths and will catch regressions
4. Line count includes all code, comments, and blank lines (measured by `wc -l`)

## Estimation

**Complexity: large**

- Multiple new files to create (3-6 handlers)
- Significant code movement with careful dependency wiring
- Must preserve existing behavior for all cluster, GC, and broadcast operations
- Estimated token budget: 150-200k tokens

**Risk areas:**
- GC consensus logic has complex state management
- Cluster event routing has many message types
- Broadcast has FLS filtering that must be preserved
