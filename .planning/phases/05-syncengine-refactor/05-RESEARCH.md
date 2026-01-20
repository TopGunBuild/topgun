# Phase 5: SyncEngine Refactor - Research

**Researched:** 2026-01-20
**Domain:** Client-side state management, WebSocket connection handling, query management, backpressure control
**Confidence:** HIGH

## Summary

This research analyzes the 2612-line SyncEngine class and identifies the standard approach for extracting it into focused modules: WebSocketManager, QueryManager, and BackpressureController. The decisions from CONTEXT.md constrain this to a specific extraction pattern with constructor injection, shared state references, and following the Phase 4 ServerCoordinator patterns.

The codebase already demonstrates the target extraction patterns through the Phase 4 refactoring (ConnectionManager, StorageManager, OperationHandler) and existing client-side abstractions (IConnectionProvider, SingleServerProvider, SyncStateMachine). These provide proven templates for the extraction. The refactoring is internal restructuring with no public API changes - all existing tests should continue to pass.

**Primary recommendation:** Extract modules one at a time in dependency order (WebSocketManager first as it has the most isolated logic, then QueryManager which depends on message sending, then BackpressureController which coordinates with opLog). Use constructor injection with config objects and follow the Phase 4 patterns established in `packages/server/src/coordinator/`.

## Standard Stack

The extraction uses existing codebase patterns - no new libraries required.

### Core (Already in Codebase)
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| TypeScript | 5.3+ | Type safety, interfaces | Already used throughout |
| ws | 8.x | WebSocket in Node.js tests | Already used for testing |
| @topgunbuild/core | local | HLC, serialize/deserialize | Already the sync foundation |

### Supporting Patterns
| Pattern | Purpose | When to Use |
|---------|---------|-------------|
| Constructor injection | Pass dependencies to modules | All new manager classes |
| Config objects | Group related configuration | Each manager's config interface |
| Callback functions | Cross-module events | When one module needs to notify another |
| Barrel files (index.ts) | Clean imports | For the sync/ folder |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| Constructor injection | Passing SyncEngine reference | Creates tight coupling, harder to test |
| Shared state references | EventEmitter for all state | More indirection, harder to trace state |
| Separate types file | Inline types | Types file is cleaner for interfaces used across modules |

**Installation:**
No new dependencies required - this is pure internal refactoring.

## Architecture Patterns

### Recommended Project Structure
```
packages/client/src/
├── SyncEngine.ts           # Orchestrator (reduced from 2612 lines)
├── sync/                   # NEW: Extracted modules
│   ├── index.ts            # Barrel exports
│   ├── types.ts            # Shared interfaces
│   ├── WebSocketManager.ts # IWebSocketManager implementation
│   ├── QueryManager.ts     # IQueryManager implementation
│   └── BackpressureController.ts # IBackpressureController implementation
├── connection/             # Existing: SingleServerProvider
├── SyncStateMachine.ts     # Existing: connection state machine
├── BackpressureConfig.ts   # Existing: backpressure config types
└── ...
```

### Pattern 1: Handler Interface Contract (from Phase 4)
**What:** Define explicit TypeScript interfaces for each module
**When to use:** All 3 extracted modules
**Example:**
```typescript
// Source: Derived from Phase 4 patterns and CONTEXT.md decisions
// packages/client/src/sync/types.ts

import type { IConnectionProvider, ConnectionProviderEvent, ConnectionEventHandler } from '../types';
import type { SyncState } from '../SyncState';
import type { QueryHandle, QueryFilter } from '../QueryHandle';
import type { HybridQueryHandle, HybridQueryFilter } from '../HybridQueryHandle';
import type { SearchHandle } from '../SearchHandle';
import type { BackpressureStatus, BackpressureThresholdEvent, OperationDroppedEvent } from '../BackpressureConfig';

/**
 * WebSocketManager owns the connection and message routing.
 * It creates/manages WebSocket or IConnectionProvider.
 */
export interface IWebSocketManager {
  /** Initialize connection (called from SyncEngine constructor) */
  connect(): void;

  /** Send a message through the current connection */
  sendMessage(message: any, key?: string): boolean;

  /** Check if we can send messages (connection is ready) */
  canSend(): boolean;

  /** Check if connection is online (may not be authenticated) */
  isOnline(): boolean;

  /** Get the connection provider (for cluster mode access) */
  getConnectionProvider(): IConnectionProvider;

  /** Close connection and cleanup */
  close(): void;

  /** Reset connection state */
  reset(): void;

  /** Subscribe to connection events */
  on(event: ConnectionProviderEvent, handler: ConnectionEventHandler): void;
  off(event: ConnectionProviderEvent, handler: ConnectionEventHandler): void;
}

/**
 * QueryManager owns the queries Map and handles all query types.
 * Single source of truth for query subscriptions.
 */
export interface IQueryManager {
  /** Get all queries (read-only access) */
  getQueries(): Map<string, QueryHandle<any>>;

  /** Subscribe to a standard query */
  subscribeToQuery(query: QueryHandle<any>): void;

  /** Unsubscribe from a query */
  unsubscribeFromQuery(queryId: string): void;

  /** Subscribe to a hybrid query (FTS + filter) */
  subscribeToHybridQuery(query: HybridQueryHandle<any>): void;

  /** Unsubscribe from a hybrid query */
  unsubscribeFromHybridQuery(queryId: string): void;

  /** Get a hybrid query by ID */
  getHybridQuery(queryId: string): HybridQueryHandle<any> | undefined;

  /** Run a local query against storage */
  runLocalQuery(mapName: string, filter: QueryFilter): Promise<{ key: string; value: any }[]>;

  /** Run a local hybrid query */
  runLocalHybridQuery<T>(mapName: string, filter: HybridQueryFilter): Promise<Array<{ key: string; value: T; score?: number; matchedTerms?: string[] }>>;

  /** Re-subscribe all queries (called after auth) */
  resubscribeAll(): void;
}

/**
 * BackpressureController manages operation flow control.
 * May own opLog depending on analysis (Claude's discretion from CONTEXT.md).
 */
export interface IBackpressureController {
  /** Get current pending ops count */
  getPendingOpsCount(): number;

  /** Get backpressure status */
  getBackpressureStatus(): BackpressureStatus;

  /** Check if writes are paused */
  isBackpressurePaused(): boolean;

  /** Check backpressure before adding operation (may pause/throw/drop) */
  checkBackpressure(): Promise<void>;

  /** Check high water mark after adding operation */
  checkHighWaterMark(): void;

  /** Check low water mark after ACKs */
  checkLowWaterMark(): void;

  /** Subscribe to backpressure events */
  onBackpressure(
    event: 'backpressure:high' | 'backpressure:low' | 'backpressure:paused' | 'backpressure:resumed' | 'operation:dropped',
    listener: (data?: BackpressureThresholdEvent | OperationDroppedEvent) => void
  ): () => void;
}
```

### Pattern 2: Constructor Injection (from Phase 4)
**What:** Dependencies passed via constructor, stored as private readonly
**When to use:** All new modules
**Example:**
```typescript
// Source: Derived from Phase 4 ConnectionManager pattern
// packages/client/src/sync/WebSocketManager.ts

import type { IConnectionProvider } from '../types';
import type { SyncStateMachine } from '../SyncStateMachine';
import { serialize, deserialize } from '@topgunbuild/core';
import { logger } from '../utils/logger';

export interface WebSocketManagerConfig {
  /** Server URL for direct WebSocket connection */
  serverUrl?: string;
  /** Connection provider (preferred over serverUrl) */
  connectionProvider?: IConnectionProvider;
  /** State machine for connection state management */
  stateMachine: SyncStateMachine;
  /** Callback when a message is received */
  onMessage: (message: any) => void;
  /** Callback when connection is established */
  onConnected?: () => void;
  /** Callback when connection is lost */
  onDisconnected?: () => void;
  /** Callback when reconnected */
  onReconnected?: () => void;
}

export class WebSocketManager implements IWebSocketManager {
  private readonly serverUrl: string;
  private readonly connectionProvider: IConnectionProvider;
  private readonly useConnectionProvider: boolean;
  private readonly stateMachine: SyncStateMachine;
  private readonly onMessage: (message: any) => void;
  private readonly onConnected?: () => void;
  private readonly onDisconnected?: () => void;

  private websocket: WebSocket | null = null;
  private reconnectTimer: any = null;

  constructor(config: WebSocketManagerConfig) {
    this.serverUrl = config.serverUrl || '';
    this.stateMachine = config.stateMachine;
    this.onMessage = config.onMessage;
    this.onConnected = config.onConnected;
    this.onDisconnected = config.onDisconnected;

    if (config.connectionProvider) {
      this.connectionProvider = config.connectionProvider;
      this.useConnectionProvider = true;
    } else {
      this.connectionProvider = new SingleServerProvider({ url: config.serverUrl! });
      this.useConnectionProvider = false;
    }
  }

  // ... implementation
}
```

### Pattern 3: Shared State References (from CONTEXT.md)
**What:** Modules receive shared state via constructor, not callbacks
**When to use:** When modules need access to shared state like opLog
**Example:**
```typescript
// Source: Derived from CONTEXT.md decision
// packages/client/src/sync/BackpressureController.ts

import type { BackpressureConfig } from '../BackpressureConfig';
import type { OpLogEntry } from '../SyncEngine';
import { logger } from '../utils/logger';

export interface BackpressureControllerConfig {
  /** Backpressure configuration */
  config: BackpressureConfig;
  /** Reference to opLog array (shared state) */
  opLog: OpLogEntry[];
}

export class BackpressureController implements IBackpressureController {
  private readonly config: BackpressureConfig;
  private readonly opLog: OpLogEntry[];

  private backpressurePaused: boolean = false;
  private waitingForCapacity: Array<() => void> = [];
  private highWaterMarkEmitted: boolean = false;
  private listeners: Map<string, Set<(...args: any[]) => void>> = new Map();

  constructor(config: BackpressureControllerConfig) {
    this.config = config.config;
    this.opLog = config.opLog;  // Shared reference, not copy
  }

  getPendingOpsCount(): number {
    return this.opLog.filter(op => !op.synced).length;
  }

  // ... implementation
}
```

### Pattern 4: Message Handler Dispatch (similar to Phase 4 MessageRegistry)
**What:** WebSocketManager routes messages to SyncEngine's handleServerMessage
**When to use:** When WebSocketManager receives messages
**Example:**
```typescript
// In WebSocketManager:
private setupConnectionProviderListeners(): void {
  this.connectionProvider.on('message', (_nodeId: string, data: any) => {
    let message: any;
    if (data instanceof ArrayBuffer) {
      message = deserialize(new Uint8Array(data));
    } else if (data instanceof Uint8Array) {
      message = deserialize(data);
    } else {
      try {
        message = typeof data === 'string' ? JSON.parse(data) : data;
      } catch (e) {
        logger.error({ err: e }, 'Failed to parse message');
        return;
      }
    }
    // Dispatch to SyncEngine via callback
    this.onMessage(message);
  });
}
```

### Anti-Patterns to Avoid
- **Circular dependencies between modules:** QueryManager should not import WebSocketManager directly. Use callbacks.
- **Passing SyncEngine instance to modules:** Creates tight coupling. Pass specific dependencies only.
- **Duplicating state:** CONTEXT.md specifies QueryManager owns queries Map. Don't have SyncEngine also track them.
- **Mixed ownership of opLog:** Decide once whether SyncEngine or BackpressureController owns it (recommendation: SyncEngine owns, BackpressureController receives reference).

## opLog Ownership Analysis (Claude's Discretion from CONTEXT.md)

Based on usage analysis of SyncEngine:

**opLog is used by:**
1. `recordOperation()` - adds entries (line 530)
2. `syncPendingOperations()` - filters unsynced, sends batch (line 545)
3. `loadOpLog()` - loads from storage (line 484)
4. Backpressure methods - counts pending, drops oldest (lines 1714, 1885)
5. OP_ACK handler - marks as synced (line 951)

**Recommendation: SyncEngine keeps opLog ownership**

Rationale:
- opLog is tightly coupled to CRDT operations and storage
- SyncEngine already handles recordOperation, loadOpLog, syncPendingOperations
- BackpressureController only reads counts and potentially drops entries
- Passing opLog reference to BackpressureController allows read/modify access without ownership

Implementation:
```typescript
// In SyncEngine constructor:
this.backpressureController = new BackpressureController({
  config: this.backpressureConfig,
  opLog: this.opLog,  // Pass reference
});

// BackpressureController can read/modify the array
// but SyncEngine remains the "owner" for storage/sync operations
```

## Don't Hand-Roll

Problems that look simple but should use existing solutions:

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Connection state | Custom flags | SyncStateMachine (exists) | Already handles state transitions, history |
| WebSocket reconnection | Custom timers | IConnectionProvider.scheduleReconnect (exists) | Handles backoff, jitter |
| Message serialization | Custom JSON | serialize/deserialize from @topgunbuild/core | msgpackr handles binary efficiently |
| Query ID generation | Custom counter | crypto.randomUUID() | Already used throughout |
| Predicate evaluation | Custom matcher | evaluatePredicate from @topgunbuild/core | Handles all predicate types |

**Key insight:** All 3 modules should reuse existing utilities. The extraction is about reorganizing responsibility, not reimplementing functionality.

## Common Pitfalls

### Pitfall 1: Breaking Private Method Tests
**What goes wrong:** Tests that access private methods via `(engine as any).methodName` break when methods move
**Why it happens:** Tests in `backpressure.test.ts` access `(engine as any).opLog`, `(engine as any).checkLowWaterMark()`
**How to avoid:**
1. Before extraction, identify all tests that access internals (backpressure.test.ts lines 133, 134, 194, etc.)
2. Consider exposing test-only methods or keeping methods accessible via SyncEngine delegation
3. Update tests incrementally with each module extraction
**Warning signs:** Test failures that mention "undefined is not a function"

### Pitfall 2: Query Subscription Race Conditions
**What goes wrong:** Queries don't receive initial data if subscribed before auth completes
**Why it happens:** QueryManager needs to queue subscriptions until authenticated
**How to avoid:**
1. QueryManager should track pending subscriptions
2. SyncEngine should call `queryManager.resubscribeAll()` after AUTH_ACK
3. Keep the existing `hasReceivedServerData` pattern in QueryHandle
**Warning signs:** Empty query results after reconnection

### Pitfall 3: WebSocket Legacy Mode vs Provider Mode
**What goes wrong:** WebSocketManager needs to support both direct WebSocket and IConnectionProvider
**Why it happens:** SyncEngine supports `serverUrl` (legacy) and `connectionProvider` (cluster) modes
**How to avoid:**
1. WebSocketManager constructor should accept both modes (like current SyncEngine lines 196-205)
2. Create SingleServerProvider internally if only serverUrl provided
3. Use `useConnectionProvider` flag to choose code path
**Warning signs:** Connection failures in single-server mode

### Pitfall 4: Backpressure State Synchronization
**What goes wrong:** BackpressureController state doesn't match opLog state after ACKs
**Why it happens:** OP_ACK handler marks ops synced, BackpressureController counts unsynced
**How to avoid:**
1. SyncEngine should call `backpressureController.checkLowWaterMark()` after ACKs (already done)
2. BackpressureController reads from shared opLog reference, not cached counts
3. Never store pendingCount separately - always compute from opLog
**Warning signs:** Paused writes not resuming after ACKs

### Pitfall 5: Hybrid Query vs Standard Query Handling
**What goes wrong:** QueryManager conflates standard queries and hybrid queries
**Why it happens:** Both have similar patterns but different message types
**How to avoid:**
1. Keep separate Maps: `queries` and `hybridQueries` (like current SyncEngine)
2. Different subscription methods: `subscribeToQuery` vs `subscribeToHybridQuery`
3. Different message types: `QUERY_SUB`/`QUERY_RESP` vs `HYBRID_QUERY_SUBSCRIBE`
**Warning signs:** FTS queries not working, scores missing

## Code Examples

Verified patterns from the existing codebase:

### Existing Phase 4 ConnectionManager Pattern
```typescript
// Source: packages/server/src/coordinator/connection-manager.ts
export class ConnectionManager implements IConnectionManager {
    private clients: Map<string, ClientConnection> = new Map();
    private readonly hlc: HLC;
    private readonly onClientRegistered?: (client: ClientConnection) => void;
    private readonly onClientRemoved?: (clientId: string) => void;

    constructor(config: ConnectionManagerConfig) {
        this.hlc = config.hlc;
        this.onClientRegistered = config.onClientRegistered;
        this.onClientRemoved = config.onClientRemoved;
    }

    getClients(): Map<string, ClientConnection> {
        return this.clients;
    }

    // ... methods operate on owned Map
}
```

### Existing IConnectionProvider Pattern
```typescript
// Source: packages/client/src/types.ts
export interface IConnectionProvider {
  connect(): Promise<void>;
  getConnection(key: string): WebSocket;
  isConnected(): boolean;
  on(event: ConnectionProviderEvent, handler: ConnectionEventHandler): void;
  off(event: ConnectionProviderEvent, handler: ConnectionEventHandler): void;
  send(data: ArrayBuffer | Uint8Array, key?: string): void;
  close(): Promise<void>;
}
```

### Existing SingleServerProvider Implementation
```typescript
// Source: packages/client/src/connection/SingleServerProvider.ts
export class SingleServerProvider implements IConnectionProvider {
  private ws: WebSocket | null = null;
  private listeners: Map<ConnectionProviderEvent, Set<ConnectionEventHandler>> = new Map();

  constructor(config: SingleServerProviderConfig) {
    this.url = config.url;
    // ... config handling
  }

  async connect(): Promise<void> {
    // WebSocket setup with reconnection
  }

  send(data: ArrayBuffer | Uint8Array, _key?: string): void {
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN) {
      throw new Error('Not connected');
    }
    this.ws.send(data);
  }
}
```

### Query Subscription Pattern in SyncEngine
```typescript
// Source: packages/client/src/SyncEngine.ts lines 638-650
public subscribeToQuery(query: QueryHandle<any>) {
  this.queries.set(query.id, query);
  if (this.isAuthenticated()) {
    this.sendQuerySubscription(query);
  }
}

public unsubscribeFromQuery(queryId: string) {
  this.queries.delete(queryId);
  if (this.isAuthenticated()) {
    this.sendMessage({
      type: 'QUERY_UNSUB',
      payload: { queryId }
    });
  }
}
```

### Backpressure Event Pattern
```typescript
// Source: packages/client/src/SyncEngine.ts lines 1744-1776
public onBackpressure(
  event: 'backpressure:high' | 'backpressure:low' | 'backpressure:paused' | 'backpressure:resumed' | 'operation:dropped',
  listener: (data?: BackpressureThresholdEvent | OperationDroppedEvent) => void
): () => void {
  if (!this.backpressureListeners.has(event)) {
    this.backpressureListeners.set(event, new Set());
  }
  this.backpressureListeners.get(event)!.add(listener);

  return () => {
    this.backpressureListeners.get(event)?.delete(listener);
  };
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| God classes | Extract class refactoring | Established pattern | Better maintainability |
| Global state | Constructor injection | Standard since ~2015 | Testability |
| Callback hell | Promise/async-await | ES2017+ | Readability |
| Direct WebSocket | IConnectionProvider abstraction | Phase 4.5 | Cluster support |

**Deprecated/outdated:**
- Direct WebSocket in SyncEngine: Replaced by IConnectionProvider for cluster support (though still supported as legacy)
- Boolean flags for connection state: Replaced by SyncStateMachine FSM

## Module Extraction Boundaries

Based on SyncEngine analysis (2612 lines):

### WebSocketManager (lines ~275-475, ~1380-1628)
**Owns:**
- WebSocket/IConnectionProvider instance
- Message serialization/deserialization
- Connection lifecycle (init, close, reset)
- Heartbeat mechanism (ping/pong, timeout)

**Message routing:**
- Receives raw messages
- Deserializes to objects
- Dispatches to SyncEngine.handleServerMessage via callback

**Line counts:**
- initConnection/initConnectionProvider: ~100 lines
- sendMessage/canSend: ~30 lines
- scheduleReconnect/calculateBackoffDelay/resetBackoff: ~50 lines
- Heartbeat methods: ~100 lines
- close/resetConnection: ~50 lines
- **Total: ~330 lines**

### QueryManager (lines ~638-785, ~2410-2611)
**Owns:**
- queries Map (standard QueryHandle instances)
- hybridQueries Map (HybridQueryHandle instances)

**Operations:**
- subscribeToQuery/unsubscribeFromQuery
- subscribeToHybridQuery/unsubscribeFromHybridQuery
- sendQuerySubscription/sendHybridQuerySubscription
- runLocalQuery/runLocalHybridQuery

**Line counts:**
- Query subscription methods: ~60 lines
- Local query execution: ~80 lines
- Hybrid query methods: ~150 lines
- **Total: ~290 lines**

### BackpressureController (lines ~1708-1905)
**Owns:**
- backpressurePaused state
- waitingForCapacity queue
- highWaterMarkEmitted flag
- backpressureListeners Map

**Receives (shared state):**
- opLog reference (for counting pending ops)
- backpressureConfig

**Operations:**
- getPendingOpsCount/getBackpressureStatus/isBackpressurePaused
- checkBackpressure/checkHighWaterMark/checkLowWaterMark
- waitForCapacity/dropOldestOp
- onBackpressure/emitBackpressureEvent

**Line counts:**
- Status methods: ~30 lines
- Check methods: ~80 lines
- Event handling: ~40 lines
- Drop/wait logic: ~50 lines
- **Total: ~200 lines**

### Stays in SyncEngine
- HLC management
- opLog operations (recordOperation, loadOpLog, saveOpLog, syncPendingOperations)
- Map registration (registerMap)
- Auth handling (setAuthToken, setTokenProvider, sendAuth)
- Topics (subscribeToTopic, publishTopic, topic queue)
- Counters (counter sync methods)
- Locks (requestLock, releaseLock)
- Entry processors
- Conflict resolvers
- Message handling (handleServerMessage switch statement)
- Failover support methods
- Write Concern methods

## Open Questions

Things that couldn't be fully resolved:

1. **Search Handle Integration**
   - What we know: SearchHandle uses `syncEngine.on('message')` for message listening
   - What's unclear: Should QueryManager handle SearchHandle or keep separate?
   - Recommendation: Keep SearchHandle separate (it's not a query, it's a subscription). It can continue using `syncEngine.on('message')`.

2. **Test Update Scope**
   - What we know: `backpressure.test.ts` accesses `(engine as any).opLog` and `(engine as any).checkLowWaterMark()`
   - What's unclear: Full extent of tests accessing internals
   - Recommendation: Audit all client test files before starting, document breaking changes per module

3. **Heartbeat Ownership**
   - What we know: Heartbeat is currently in SyncEngine (lines 1547-1653)
   - What's unclear: Should it stay in SyncEngine or move to WebSocketManager?
   - Recommendation: Move to WebSocketManager since it's tightly coupled to connection health

## Sources

### Primary (HIGH confidence)
- `/Users/koristuvac/Projects/topgun/topgun/packages/client/src/SyncEngine.ts` - Source file being refactored (2612 lines)
- `/Users/koristuvac/Projects/topgun/topgun/packages/server/src/coordinator/connection-manager.ts` - Phase 4 extraction pattern
- `/Users/koristuvac/Projects/topgun/topgun/packages/server/src/coordinator/storage-manager.ts` - Phase 4 state ownership pattern
- `/Users/koristuvac/Projects/topgun/topgun/packages/server/src/coordinator/types.ts` - Phase 4 interface pattern
- `/Users/koristuvac/Projects/topgun/topgun/.planning/phases/04-servercoordinator-refactor/04-RESEARCH.md` - Phase 4 research
- `/Users/koristuvac/Projects/topgun/topgun/.planning/phases/05-syncengine-refactor/05-CONTEXT.md` - User decisions

### Secondary (MEDIUM confidence)
- `/Users/koristuvac/Projects/topgun/topgun/packages/client/src/connection/SingleServerProvider.ts` - IConnectionProvider implementation
- `/Users/koristuvac/Projects/topgun/topgun/packages/client/src/types.ts` - IConnectionProvider interface
- `/Users/koristuvac/Projects/topgun/topgun/packages/client/src/SyncStateMachine.ts` - State machine pattern

### Tertiary (LOW confidence)
- General refactoring patterns from Phase 4 research sources

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH - Uses existing codebase patterns, no new deps
- Architecture: HIGH - Based on Phase 4 patterns and CONTEXT.md decisions
- Module boundaries: HIGH - Derived from analyzing actual SyncEngine code
- Pitfalls: HIGH - Derived from test file analysis and code patterns

**Research date:** 2026-01-20
**Valid until:** 60 days (internal refactoring, stable codebase patterns)

## Extraction Order Recommendation

Based on dependency analysis:

1. **WebSocketManager (05-01)**
   - No dependencies on other extracted modules
   - Self-contained connection logic
   - Clear interface boundary with SyncEngine via callbacks

2. **QueryManager (05-02)**
   - Depends on message sending (via WebSocketManager/SyncEngine callback)
   - Depends on storage adapter (for local queries)
   - Clear ownership of queries Map

3. **BackpressureController (05-03)**
   - Depends on opLog (shared reference from SyncEngine)
   - Most coupled to SyncEngine state
   - Should be extracted last

**Rationale:** This order minimizes refactoring churn. Each extraction builds on the previous without requiring rework.
