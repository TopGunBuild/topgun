# Phase 3 Research: Bug Fixes

**Researched:** 2026-01-18
**Domain:** Client/Server bug fixes, adapter patterns, debug logging
**Confidence:** HIGH

## Summary

Phase 3 addresses three known bugs affecting production reliability:
1. **BUG-05:** BetterAuth adapter race condition on cold start (requests arrive before storage loads)
2. **BUG-06:** Topic messages dropped when client is offline (no queueing)
3. **BUG-07:** Verbose debug logging in `getMapAsync` pollutes production logs

All three bugs have clear code locations, existing patterns to follow, and straightforward fixes. The codebase already has established patterns for ready-state gating (IDBAdapter), queue management (BackpressureConfig), and environment-gated debug logging (TOPGUN_DEBUG).

**Primary recommendation:** Follow existing codebase patterns - add ready Promise to BetterAuth adapter, add topic message queue to SyncEngine, gate debug logs behind TOPGUN_DEBUG.

---

## BUG-05: BetterAuth Cold Start Race

### Current State

**File:** `packages/adapter-better-auth/src/TopGunAdapter.ts`

The BetterAuth adapter creates a `topGunAdapter` factory that wraps `TopGunClient` for use with the BetterAuth authentication library. The adapter provides CRUD operations (`create`, `findOne`, `findMany`, `update`, `delete`) that delegate to TopGun's LWWMap.

**Current Code Flow:**
```typescript
// TopGunAdapter.ts lines 110-118
async create({ model, data }) {
  const mapName = getMapName(model);
  const id = (data as any).id || crypto.randomUUID();
  const record = { ...data, id };

  // Uses LWWMap for standard records
  const map = client.getMap<string, any>(mapName);
  map.set(id, record);

  return record as any;
}
```

The `client.getMap()` call (in `TopGunClient.ts:240-287`) returns an LWWMap immediately but triggers async storage restoration in the background:

```typescript
// TopGunClient.ts lines 253-267
this.storageAdapter.getAllKeys().then(async (keys) => {
  // ... restore state from storage asynchronously
}).catch(err => logger.error({ err }, 'Failed to restore keys from storage'));
```

### Problem Analysis

**The Race Condition:**
1. Application starts, creates `TopGunClient` with storage adapter
2. `client.start()` is called, begins storage initialization
3. BetterAuth adapter is created with the client
4. First HTTP request arrives (e.g., user login)
5. Adapter calls `client.getMap('auth_user')` - returns empty map
6. `map.get('user-1')` returns `undefined` - user not found
7. Authentication fails even though user exists in storage

**Root Cause:** `TopGunClient.getMap()` is synchronous and doesn't wait for storage restoration. The comment in the adapter explicitly acknowledges this:

```typescript
// TopGunAdapter.ts lines 126-134
// LWWMap.get is synchronous from memory (loaded from storage).
// If we haven't loaded yet, we might miss it.
// Ideally we should ensure map is loaded.
// TopGunClient.getMap returns immediately but starts restoring in background.
// This creates a race condition for cold start.
```

**Scope of Impact:**
- All adapter methods that call `client.getMap()` are affected
- `create`, `findOne`, `findMany`, `update`, `delete`, `updateMany`, `deleteMany`, `count`
- Most critical: `findOne` for user lookups during authentication

### Implementation Approach

**Pattern to Follow:** IDBAdapter's ready-state pattern

The `IDBAdapter` in `packages/client/src/adapters/IDBAdapter.ts` has the exact pattern needed:

```typescript
// IDBAdapter.ts lines 26-28, 75-80
private isReady = false;
private initPromise?: Promise<void>;

async waitForReady(): Promise<void> {
  if (this.isReady) return;
  if (this.initPromise) {
    await this.initPromise;
  }
}
```

**Recommended Fix:**

1. Add `TopGunAdapterOptions.waitForReady` option:
```typescript
export interface TopGunAdapterOptions {
  client: TopGunClient;
  modelMap?: Record<string, string>;
  /** Wait for client storage to be ready before accepting requests (default: true) */
  waitForReady?: boolean;
}
```

2. Add ready state tracking and wait method:
```typescript
let isReady = false;
let readyPromise: Promise<void> | null = null;

const ensureReady = async () => {
  if (isReady) return;
  if (!readyPromise) {
    // Wait for storage adapter to be ready
    // TopGunClient.start() initializes storage
    readyPromise = client.start().then(() => {
      isReady = true;
    });
  }
  await readyPromise;
};
```

3. Call `ensureReady()` at the start of each adapter method:
```typescript
async create({ model, data }) {
  await ensureReady();  // <-- Add this
  const mapName = getMapName(model);
  // ... rest of method
}
```

**Alternative Approaches Considered:**
- **Option B:** Add `getMapAsync()` to TopGunClient - Rejected: Would require changing all adapter methods and doesn't solve the general problem
- **Option C:** Lazy initialization per-map - Rejected: Complex, doesn't guarantee cross-map consistency

### Testing Strategy

**Unit Tests (TopGunAdapter.test.ts):**
1. Test that adapter waits for storage before first operation
2. Test that concurrent requests all wait for same ready promise
3. Test that subsequent requests don't wait if already ready
4. Test error handling if storage initialization fails

**Integration Test:**
1. Create adapter with slow storage initialization
2. Fire request immediately after creation
3. Verify request succeeds with correct data

**Files to Modify:**
- `packages/adapter-better-auth/src/TopGunAdapter.ts` - Add ready gating
- `packages/adapter-better-auth/src/__tests__/TopGunAdapter.test.ts` - Add tests

---

## BUG-06: Topic Offline Queue

### Current State

**File:** `packages/client/src/SyncEngine.ts`

Topics provide pub/sub messaging via `TopicHandle`. When online, publishing works:

```typescript
// SyncEngine.ts lines 632-645
public publishTopic(topic: string, data: any) {
  if (this.isAuthenticated()) {
    this.sendMessage({
      type: 'TOPIC_PUB',
      payload: { topic, data }
    });
  } else {
    // TODO: Queue topic messages or drop?
    // Spec says Fire-and-Forget, so dropping is acceptable if offline,
    // but queueing is better UX.
    // For now, log warning.
    logger.warn({ topic }, 'Dropped topic publish (offline)');
  }
}
```

**Current Behavior:** Messages are silently dropped when offline (only a warning is logged).

### Problem Analysis

**User Impact:**
- User publishes message while offline
- Message is lost - no indication to user
- When reconnecting, no queued messages are sent
- Data loss for any offline topic publishes

**Why This Matters:**
- Topics are used for ephemeral messaging (chat, notifications, live updates)
- Brief disconnections are common (network switching, server restart)
- Users expect messages sent during brief offline periods to be delivered

**Constraints from Requirements:**
- "Topic messages queued when offline with configurable max size"
- Prior decision: "LRU over hard limits: Graceful degradation preferred"

### Implementation Approach

**Pattern to Follow:** Backpressure queue in SyncEngine

The codebase has established queue patterns:

```typescript
// BackpressureConfig.ts - Queue configuration pattern
export interface BackpressureConfig {
  maxPendingOps: number;        // Maximum pending operations
  strategy: 'pause' | 'throw' | 'drop-oldest';
  highWaterMark: number;        // 0.8 = 80%
  lowWaterMark: number;         // 0.5 = 50%
}

// SyncEngine.ts - Queue management
private opLog: OpLogEntry[] = [];
private waitingForCapacity: Array<() => void> = [];
```

**Recommended Fix:**

1. Add topic queue configuration to `SyncEngineConfig`:
```typescript
export interface TopicQueueConfig {
  /** Maximum queued topic messages (default: 100) */
  maxSize: number;
  /** Strategy when queue is full: 'drop-oldest' | 'drop-newest' (default: 'drop-oldest') */
  strategy: 'drop-oldest' | 'drop-newest';
}

export interface SyncEngineConfig {
  // ... existing fields
  topicQueue?: Partial<TopicQueueConfig>;
}
```

2. Add topic message queue to SyncEngine:
```typescript
interface QueuedTopicMessage {
  topic: string;
  data: any;
  timestamp: number;
}

private topicQueue: QueuedTopicMessage[] = [];
private readonly topicQueueConfig: TopicQueueConfig;
```

3. Modify `publishTopic` to queue when offline:
```typescript
public publishTopic(topic: string, data: any) {
  if (this.isAuthenticated()) {
    this.sendMessage({
      type: 'TOPIC_PUB',
      payload: { topic, data }
    });
  } else {
    this.queueTopicMessage(topic, data);
  }
}

private queueTopicMessage(topic: string, data: any): void {
  const message: QueuedTopicMessage = {
    topic,
    data,
    timestamp: Date.now()
  };

  if (this.topicQueue.length >= this.topicQueueConfig.maxSize) {
    if (this.topicQueueConfig.strategy === 'drop-oldest') {
      const dropped = this.topicQueue.shift();
      logger.warn({ topic: dropped?.topic }, 'Dropped oldest queued topic message');
    } else {
      logger.warn({ topic }, 'Dropped newest topic message (queue full)');
      return;
    }
  }

  this.topicQueue.push(message);
  logger.debug({ topic, queueSize: this.topicQueue.length }, 'Queued topic message for offline');
}
```

4. Flush queue on reconnection (in `AUTH_ACK` handler):
```typescript
case 'AUTH_ACK': {
  // ... existing code
  this.flushTopicQueue();  // <-- Add this
  break;
}

private flushTopicQueue(): void {
  if (this.topicQueue.length === 0) return;

  logger.info({ count: this.topicQueue.length }, 'Flushing queued topic messages');

  for (const msg of this.topicQueue) {
    this.sendMessage({
      type: 'TOPIC_PUB',
      payload: { topic: msg.topic, data: msg.data }
    });
  }

  this.topicQueue = [];
}
```

5. Expose queue status for client monitoring:
```typescript
public getTopicQueueStatus(): { size: number; maxSize: number } {
  return {
    size: this.topicQueue.length,
    maxSize: this.topicQueueConfig.maxSize
  };
}
```

**Alternative Approaches Considered:**
- **Persist queue to storage:** Rejected - Topics are ephemeral by design, persistence adds complexity
- **No limit queue:** Rejected - Memory safety concern, goes against "LRU over hard limits" decision

### Testing Strategy

**Unit Tests (SyncEngine.test.ts or new TopicQueue.test.ts):**
1. Test message queuing when offline
2. Test queue flush on reconnection
3. Test drop-oldest strategy when queue is full
4. Test drop-newest strategy when queue is full
5. Test queue size limits
6. Test multiple topics in queue

**Integration Test:**
1. Connect client, subscribe to topic
2. Disconnect client
3. Publish messages while offline
4. Reconnect
5. Verify messages delivered in order

**Files to Modify:**
- `packages/client/src/SyncEngine.ts` - Add topic queue
- `packages/client/src/BackpressureConfig.ts` - Add TopicQueueConfig export (optional, could be inline)
- `packages/client/src/__tests__/SyncEngine.test.ts` or new test file

---

## BUG-07: Debug Logging Gating

### Current State

**File:** `packages/server/src/ServerCoordinator.ts`

The `getMapAsync` function contains verbose debug logging that runs on every map access:

```typescript
// ServerCoordinator.ts lines 4251-4267
// [DEBUG] Log state for troubleshooting sync issues
const map = this.maps.get(name);
const mapSize = map instanceof LWWMap ? Array.from(map.entries()).length :
               map instanceof ORMap ? map.size : 0;
logger.info({
    mapName: name,
    mapExisted,
    hasLoadingPromise: !!loadingPromise,
    currentMapSize: mapSize
}, '[getMapAsync] State check');

if (loadingPromise) {
    logger.info({ mapName: name }, '[getMapAsync] Waiting for loadMapFromStorage...');
    await loadingPromise;
    // ... more logging
    logger.info({ mapName: name, mapSizeAfterLoad: newMapSize }, '[getMapAsync] Load completed');
}
```

**Current Problem:**
- `logger.info` is always called, regardless of debug mode
- High-frequency code path (called for every query, every operation)
- Performance impact from string formatting and object creation
- Log pollution in production

### Problem Analysis

**Why This Exists:**
- Added during development/debugging of sync issues
- Comment says "[DEBUG]" but uses `logger.info` instead of `logger.debug`
- Never gated behind environment check

**Existing Pattern:**
The codebase already has the `TOPGUN_DEBUG` environment variable pattern:

```typescript
// ServerCoordinator.ts lines 179, 411
/** Enable debug endpoints (default: false, or TOPGUN_DEBUG=true) */
debugEnabled?: boolean;

const debugEnabled = config.debugEnabled ?? process.env.TOPGUN_DEBUG === 'true';
```

### Implementation Approach

**Recommended Fix:**

1. Check `TOPGUN_DEBUG` before debug logging:
```typescript
public async getMapAsync(name: string, typeHint: 'LWW' | 'OR' = 'LWW'): Promise<LWWMap<string, any> | ORMap<string, any>> {
    const mapExisted = this.maps.has(name);

    // First ensure map exists (this triggers loading if needed)
    this.getMap(name, typeHint);

    // Wait for loading to complete if in progress
    const loadingPromise = this.mapLoadingPromises.get(name);

    // Debug logging gated behind TOPGUN_DEBUG
    if (process.env.TOPGUN_DEBUG === 'true') {
        const map = this.maps.get(name);
        const mapSize = map instanceof LWWMap ? Array.from(map.entries()).length :
                       map instanceof ORMap ? map.size : 0;
        logger.info({
            mapName: name,
            mapExisted,
            hasLoadingPromise: !!loadingPromise,
            currentMapSize: mapSize
        }, '[getMapAsync] State check');
    }

    if (loadingPromise) {
        if (process.env.TOPGUN_DEBUG === 'true') {
            logger.info({ mapName: name }, '[getMapAsync] Waiting for loadMapFromStorage...');
        }
        await loadingPromise;
        if (process.env.TOPGUN_DEBUG === 'true') {
            const map = this.maps.get(name);
            const newMapSize = map instanceof LWWMap ? Array.from(map.entries()).length :
                              map instanceof ORMap ? map.size : 0;
            logger.info({ mapName: name, mapSizeAfterLoad: newMapSize }, '[getMapAsync] Load completed');
        }
    }

    return this.maps.get(name)!;
}
```

**Alternative: Cache the debug flag:**
```typescript
// In constructor or as class field
private readonly debugEnabled = process.env.TOPGUN_DEBUG === 'true';

// Then use:
if (this.debugEnabled) {
  logger.info(...);
}
```

**Alternative Approaches Considered:**
- **Use logger.debug instead of logger.info:** Rejected - Still formats strings even if level filtered, less explicit control
- **Remove logging entirely:** Rejected - Useful for debugging sync issues when enabled
- **Add dedicated debug logger:** Overkill for this single use case

### Testing Strategy

**Unit Tests:**
1. Test that logs appear when `TOPGUN_DEBUG=true`
2. Test that logs do NOT appear when `TOPGUN_DEBUG` is not set
3. Test that logs do NOT appear when `TOPGUN_DEBUG=false`

**Manual Verification:**
1. Start server without `TOPGUN_DEBUG`
2. Make requests that trigger `getMapAsync`
3. Verify no `[getMapAsync]` logs appear
4. Restart with `TOPGUN_DEBUG=true`
5. Verify logs now appear

**Files to Modify:**
- `packages/server/src/ServerCoordinator.ts` - Gate debug logs

---

## Cross-Cutting Concerns

### Shared Patterns

All three bugs follow established codebase patterns:
- **Ready gating:** IDBAdapter.waitForReady()
- **Queue management:** BackpressureConfig, opLog
- **Environment flags:** TOPGUN_DEBUG, LOG_LEVEL

### Dependencies Between Bugs

- **BUG-05 and BUG-06 are independent** - Can be fixed in parallel
- **BUG-07 is independent** - Pure logging change, no behavioral dependencies

### Testing Environment

- All tests can use existing test infrastructure
- No new test dependencies required
- Tests should clean up environment variables after modifying them

### Rollout Considerations

- **BUG-05:** Breaking change if adapter previously worked with race (unlikely). Safe to deploy.
- **BUG-06:** New behavior (queuing vs dropping). Default should match current behavior for safe rollout, or document the change.
- **BUG-07:** No behavior change, only log output change. Safe to deploy.

---

## Sources

### Primary (HIGH confidence)
- `packages/adapter-better-auth/src/TopGunAdapter.ts` - Direct code inspection
- `packages/client/src/SyncEngine.ts` - Direct code inspection
- `packages/server/src/ServerCoordinator.ts` - Direct code inspection
- `packages/client/src/adapters/IDBAdapter.ts` - Pattern reference
- `packages/client/src/BackpressureConfig.ts` - Pattern reference

### Secondary (HIGH confidence)
- `.planning/codebase/CONCERNS.md` - Bug documentation
- Existing tests in `packages/adapter-better-auth/src/__tests__/`
- Existing tests in `packages/client/src/__tests__/`

---

## Metadata

**Confidence breakdown:**
- BUG-05 Analysis: HIGH - Clear code path, acknowledged in comments
- BUG-05 Fix: HIGH - Follows established IDBAdapter pattern
- BUG-06 Analysis: HIGH - Clear TODO with context
- BUG-06 Fix: HIGH - Follows established BackpressureConfig pattern
- BUG-07 Analysis: HIGH - Clear code location, existing env var pattern
- BUG-07 Fix: HIGH - Simple conditional gating

**Research date:** 2026-01-18
**Valid until:** 2026-02-18 (30 days - stable patterns, no external dependencies)

---

## RESEARCH COMPLETE

**Phase:** 3 - Bug Fixes
**Confidence:** HIGH

### Key Findings

1. **BUG-05 (BetterAuth race):** Race condition is acknowledged in code comments. Fix is to add `ensureReady()` call using IDBAdapter's pattern.

2. **BUG-06 (Topic offline queue):** TODO exists at line 639 with clear context. Fix adds queue with configurable max size and drop-oldest strategy.

3. **BUG-07 (Debug logging):** Debug logs use `logger.info` but should be gated behind `TOPGUN_DEBUG` env var, matching existing pattern.

4. **All fixes follow existing patterns** - No new patterns or dependencies needed.

5. **Independent bugs** - Can be planned and implemented in parallel.

### File Created

`.planning/phases/03-bug-fixes/03-RESEARCH.md`

### Confidence Assessment

| Area | Level | Reason |
|------|-------|--------|
| BUG-05 Fix | HIGH | Pattern exists in IDBAdapter |
| BUG-06 Fix | HIGH | Pattern exists in BackpressureConfig |
| BUG-07 Fix | HIGH | Pattern exists in ServerCoordinator |
| Testing | HIGH | Existing test infrastructure |

### Open Questions

None - all three bugs have clear fixes with established patterns.

### Ready for Planning

Research complete. Planner can now create PLAN.md files.
