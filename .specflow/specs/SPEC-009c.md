---
id: SPEC-009c
parent: SPEC-009
type: refactor
status: draft
priority: high
complexity: small
created: 2026-01-29
depends_on: [SPEC-009a]
---

# Extract Sync Protocol Handlers from SyncEngine

## Context

This is Part 3 of the SyncEngine refactoring (SPEC-009). This phase extracts two sync protocol handlers that manage Merkle tree synchronization for LWWMap and ORMap.

### Prior Work

Reference: SPEC-009 (parent specification), SPEC-009a (core handlers)

After SPEC-009a, the handler pattern will be established. SPEC-009c can be implemented in parallel with SPEC-009b as they have no interdependencies.

### Why Sync Handlers Third

- ORMapSyncHandler and MerkleSyncHandler handle specific sync protocol messages
- They work with the existing map registry in SyncEngine
- They need access to HLC for timestamp updates
- Logically separate from feature handlers (topic, lock, counter, search, etc.)

## Goal Statement

Extract two sync protocol handlers from SyncEngine.ts, reducing the main file by approximately 200 lines while maintaining all existing Merkle tree synchronization functionality.

## Task

### 1. MerkleSyncHandler (~70 lines)

Create `packages/client/src/sync/MerkleSyncHandler.ts`:

**Message types handled:**
- `SYNC_RESP_ROOT` (lines 887-909)
- `SYNC_RESP_BUCKETS` (lines 911-930)
- `SYNC_RESP_LEAF` (lines 932-951)
- `SYNC_RESET_REQUIRED` (lines 874-885)

**Dependencies:**
- Access to `maps: Map<string, LWWMap | ORMap>` (via callback)
- `sendMessage` callback
- `storageAdapter` for persistence
- `hlc` for timestamp updates
- `saveOpLog` callback

**Config interface:**
```typescript
export interface MerkleSyncHandlerConfig {
  getMap: (mapName: string) => LWWMap<any, any> | ORMap<any, any> | undefined;
  sendMessage: (message: any, key?: string) => boolean;
  storageAdapter: IStorageAdapter;
  hlc: HLC;
  onTimestampUpdate: (timestamp: Timestamp) => Promise<void>;
  resetMap: (mapName: string) => Promise<void>;
}
```

**Methods:**
- `handleSyncRespRoot(payload)` - public
- `handleSyncRespBuckets(payload)` - public
- `handleSyncRespLeaf(payload)` - public
- `handleSyncResetRequired(payload)` - public
- `sendSyncInit(mapName: string, lastSyncTimestamp: number)` - public, constructs and sends SYNC_INIT message

### 2. ORMapSyncHandler (~130 lines)

Create `packages/client/src/sync/ORMapSyncHandler.ts`:

**Message types handled:**
- `ORMAP_SYNC_RESP_ROOT` (lines 955-979)
- `ORMAP_SYNC_RESP_BUCKETS` (lines 981-1012)
- `ORMAP_SYNC_RESP_LEAF` (lines 1014-1037)
- `ORMAP_DIFF_RESPONSE` (lines 1039-1058)

**Methods to extract:**
- `pushORMapDiff(mapName, keys, map)` (lines 1355-1400) - becomes public

**Dependencies:**
- Access to `maps: Map<string, LWWMap | ORMap>` (via callback)
- `sendMessage` callback
- `hlc` for timestamp updates

**Config interface:**
```typescript
export interface ORMapSyncHandlerConfig {
  getMap: (mapName: string) => LWWMap<any, any> | ORMap<any, any> | undefined;
  sendMessage: (message: any, key?: string) => boolean;
  hlc: HLC;
  onTimestampUpdate: (timestamp: Timestamp) => Promise<void>;
}
```

**Methods:**
- `handleORMapSyncRespRoot(payload)` - public
- `handleORMapSyncRespBuckets(payload)` - public
- `handleORMapSyncRespLeaf(payload)` - public
- `handleORMapDiffResponse(payload)` - public
- `pushORMapDiff(mapName, keys, map)` - public (called from handleORMapSyncRespLeaf and handleORMapSyncRespBuckets)
- `sendSyncInit(mapName: string, lastSyncTimestamp: number)` - public, constructs and sends ORMAP_SYNC_INIT message

### Implementation Steps

1. **Add interfaces to `sync/types.ts`:**
   - `IMerkleSyncHandler`, `MerkleSyncHandlerConfig`
   - `IORMapSyncHandler`, `ORMapSyncHandlerConfig`

2. **Create handler files:**
   - `sync/MerkleSyncHandler.ts`
   - `sync/ORMapSyncHandler.ts`

3. **Update `sync/index.ts`:**
   - Export new handlers and their types

4. **Update SyncEngine.ts:**
   - Import new handlers
   - Initialize handlers in constructor with appropriate callbacks:
     - `getMap: (name) => this.maps.get(name)`
     - `onTimestampUpdate: async (ts) => { this.hlc.update(ts); this.lastSyncTimestamp = ts.millis; await this.saveOpLog(); }`
     - `resetMap: (name) => this.resetMap(name)`
   - Update `handleServerMessage()` to delegate:
     - SYNC_RESP_ROOT, SYNC_RESP_BUCKETS, SYNC_RESP_LEAF, SYNC_RESET_REQUIRED -> MerkleSyncHandler
     - ORMAP_SYNC_RESP_ROOT, ORMAP_SYNC_RESP_BUCKETS, ORMAP_SYNC_RESP_LEAF, ORMAP_DIFF_RESPONSE -> ORMapSyncHandler
   - Remove `pushORMapDiff` from SyncEngine (moved to ORMapSyncHandler)
   - Update `startMerkleSync()` to delegate sync init message construction:
     - For LWWMap: call `merkleSyncHandler.sendSyncInit(mapName, lastSyncTimestamp)`
     - For ORMap: call `orMapSyncHandler.sendSyncInit(mapName, lastSyncTimestamp)`

## Acceptance Criteria

1. [ ] New file `packages/client/src/sync/MerkleSyncHandler.ts` exists
2. [ ] New file `packages/client/src/sync/ORMapSyncHandler.ts` exists
3. [ ] All handlers implement their respective interfaces
4. [ ] Config interfaces added to `sync/types.ts`
5. [ ] All handlers exported from `sync/index.ts`
6. [ ] SyncEngine delegates to MerkleSyncHandler for LWWMap sync messages
7. [ ] SyncEngine delegates to ORMapSyncHandler for ORMap sync messages
8. [ ] Message routing updated for all 8 message types
9. [ ] `pushORMapDiff` removed from SyncEngine
10. [ ] MerkleSyncHandler has `sendSyncInit()` method
11. [ ] ORMapSyncHandler has `sendSyncInit()` method
12. [ ] `startMerkleSync()` delegates to handlers for sync init messages
13. [ ] All existing tests pass: `pnpm --filter @topgunbuild/client test`
11. [ ] TypeScript compiles without errors: `pnpm --filter @topgunbuild/client build`
12. [ ] No changes to public SyncEngine API

## Constraints

1. **DO NOT** change the WebSocket message protocol
2. **DO NOT** modify test files (tests must pass as-is)
3. **DO NOT** change public method signatures on SyncEngine
4. **DO NOT** introduce circular dependencies between handlers
5. **DO** follow the existing handler pattern
6. **DO** use callbacks for map access and timestamp updates
7. **DO** preserve exact sync behavior including MerkleTree operations
8. **DO** import LWWMap, ORMap from @topgunbuild/core

## Assumptions

1. SPEC-009a has been completed and the handler pattern is validated
2. The `maps` Map in SyncEngine contains both LWWMap and ORMap instances
3. The HLC instance is passed by reference (not copied)
4. The `resetMap` method remains in SyncEngine (called via callback)
5. Timestamp type is exported from @topgunbuild/core

## Estimation

**Complexity: small**

- 2 new handler files (~200 lines total)
- Sync protocol message handling
- Well-defined callbacks for map access
- Estimated token budget: 30-50k tokens

## Files Summary

| File | Action | Lines |
|------|--------|-------|
| `sync/types.ts` | Modify | +50 (interfaces) |
| `sync/MerkleSyncHandler.ts` | Create | ~70 |
| `sync/ORMapSyncHandler.ts` | Create | ~130 |
| `sync/index.ts` | Modify | +8 (exports) |
| `SyncEngine.ts` | Modify | -170 (delegations, remove pushORMapDiff) |
