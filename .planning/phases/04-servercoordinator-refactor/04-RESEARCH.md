# Phase 4: ServerCoordinator Refactor - Research

**Researched:** 2026-01-18
**Domain:** Large class refactoring, module extraction, dependency injection
**Confidence:** HIGH

## Summary

This research analyzes the 5086-line ServerCoordinator god object and identifies the standard approach for extracting it into focused modules: AuthHandler, ConnectionManager, OperationHandler, and StorageManager. The decisions from CONTEXT.md constrain this to a specific extraction pattern with constructor injection, explicit TypeScript interfaces, and a message registry pattern.

The codebase already demonstrates the target pattern through existing handlers (CounterHandler, EntryProcessorHandler, ConflictResolverHandler) and managers (TopicManager, SecurityManager, LockManager). These provide a proven template for the extraction. The refactoring is internal restructuring with no API changes - all existing tests should continue to pass.

**Primary recommendation:** Extract modules one at a time in dependency order (AuthHandler first, then ConnectionManager, then StorageManager, then OperationHandler), using the existing handler patterns as templates. Preserve method signatures during extraction and update tests incrementally.

## Standard Stack

The extraction uses existing codebase patterns - no new libraries required.

### Core (Already in Codebase)
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| TypeScript | 5.3+ | Type safety, interfaces | Already used throughout |
| pino | 8.x | Structured logging | Already in packages/server |
| ws | 8.x | WebSocket connections | Already the connection layer |
| jsonwebtoken | 9.x | JWT auth | Already used for auth |

### Supporting Patterns
| Pattern | Purpose | When to Use |
|---------|---------|-------------|
| Constructor injection | Pass dependencies to modules | All new handler/manager classes |
| Factory functions | Create handler instances with config | If initialization is complex |
| Barrel files (index.ts) | Clean imports | For the coordinator/ folder |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| Constructor injection | DI framework (InversifyJS) | Overkill for 4 modules, adds complexity |
| Manual routing | Decorator-based routing | Would require new patterns, not in codebase |
| Inline type definitions | Separate types file | Types file is cleaner for interfaces used across modules |

**Installation:**
No new dependencies required - this is pure internal refactoring.

## Architecture Patterns

### Recommended Project Structure
```
packages/server/src/
├── ServerCoordinator.ts     # Orchestrator (reduced from 5086 lines)
├── coordinator/             # NEW: Extracted modules
│   ├── index.ts             # Barrel exports
│   ├── types.ts             # Shared interfaces
│   ├── auth-handler.ts      # IAuthHandler implementation
│   ├── connection-manager.ts # IConnectionManager implementation
│   ├── operation-handler.ts  # IOperationHandler implementation
│   └── storage-manager.ts    # IStorageManager implementation
├── handlers/                # Existing: CounterHandler, etc.
├── cluster/                 # Existing: ClusterManager, etc.
└── ...
```

### Pattern 1: Handler Interface Contract (from CONTEXT.md)
**What:** Define explicit TypeScript interfaces for each module
**When to use:** All 4 extracted modules
**Example:**
```typescript
// Source: Derived from existing codebase patterns
// packages/server/src/coordinator/types.ts

import { WebSocket } from 'ws';
import { Principal } from '@topgunbuild/core';

export interface ClientConnection {
  id: string;
  socket: WebSocket;
  writer: CoalescingWriter;
  principal?: Principal;
  isAuthenticated: boolean;
  subscriptions: Set<string>;
  lastActiveHlc: Timestamp;
  lastPingReceived: number;
}

export interface IAuthHandler {
  /**
   * Verify JWT token and return principal, or throw on failure
   */
  verifyToken(token: string): Principal;

  /**
   * Handle AUTH message from client
   * @returns true if auth succeeded, false if failed
   */
  handleAuth(client: ClientConnection, token: string): Promise<boolean>;
}

export interface IConnectionManager {
  /**
   * Get all connected clients
   */
  getClients(): Map<string, ClientConnection>;

  /**
   * Register a new client connection
   */
  registerClient(socket: WebSocket): ClientConnection;

  /**
   * Remove client and cleanup subscriptions
   */
  removeClient(clientId: string): void;

  /**
   * Broadcast message to clients (with optional exclusion)
   */
  broadcast(message: any, excludeClientId?: string): void;

  /**
   * Check if client is alive based on heartbeat
   */
  isClientAlive(clientId: string): boolean;
}

export interface IOperationHandler {
  /**
   * Process a single client operation
   */
  processOp(op: any, clientId: string): Promise<void>;

  /**
   * Process a batch of operations
   */
  processBatch(ops: any[], clientId: string): Promise<void>;

  /**
   * Handle query subscription
   */
  handleQuerySub(client: ClientConnection, queryId: string, mapName: string, query: any): Promise<void>;
}

export interface IStorageManager {
  /**
   * Get or create a map by name
   */
  getMap(name: string, typeHint?: 'LWW' | 'OR'): LWWMap<string, any> | ORMap<string, any>;

  /**
   * Get map with async loading guarantee
   */
  getMapAsync(name: string, typeHint?: 'LWW' | 'OR'): Promise<LWWMap<string, any> | ORMap<string, any>>;

  /**
   * Persist a record to storage
   */
  store(mapName: string, key: string, record: any): Promise<void>;

  /**
   * Load all records for a map
   */
  loadMapFromStorage(mapName: string, typeHint: 'LWW' | 'OR'): Promise<void>;
}
```

### Pattern 2: Constructor Injection (from CONTEXT.md)
**What:** Dependencies passed via constructor, stored as private readonly
**When to use:** All new modules
**Example:**
```typescript
// Source: Derived from existing TopicManager pattern
// packages/server/src/coordinator/auth-handler.ts

import * as jwt from 'jsonwebtoken';
import { Principal } from '@topgunbuild/core';
import { logger } from '../utils/logger';
import type { IAuthHandler, ClientConnection, AuthHandlerConfig } from './types';

export interface AuthHandlerConfig {
  jwtSecret: string;
  onAuthSuccess?: (clientId: string, principal: Principal) => void;
  onAuthFailure?: (clientId: string, error: string) => void;
}

export class AuthHandler implements IAuthHandler {
  private readonly jwtSecret: string;
  private readonly onAuthSuccess?: (clientId: string, principal: Principal) => void;
  private readonly onAuthFailure?: (clientId: string, error: string) => void;

  constructor(config: AuthHandlerConfig) {
    this.jwtSecret = config.jwtSecret;
    this.onAuthSuccess = config.onAuthSuccess;
    this.onAuthFailure = config.onAuthFailure;
  }

  verifyToken(token: string): Principal {
    const isRSAKey = this.jwtSecret.includes('-----BEGIN');
    const verifyOptions: jwt.VerifyOptions = isRSAKey
      ? { algorithms: ['RS256'] }
      : { algorithms: ['HS256'] };

    const decoded = jwt.verify(token, this.jwtSecret, verifyOptions) as any;

    // Normalize principal
    if (!decoded.roles) {
      decoded.roles = ['USER'];
    }
    if (!decoded.userId && decoded.sub) {
      decoded.userId = decoded.sub;
    }

    return decoded as Principal;
  }

  async handleAuth(client: ClientConnection, token: string): Promise<boolean> {
    try {
      const principal = this.verifyToken(token);
      client.principal = principal;
      client.isAuthenticated = true;

      logger.info({ clientId: client.id, user: principal.userId || 'anon' }, 'Client authenticated');
      this.onAuthSuccess?.(client.id, principal);

      return true;
    } catch (e) {
      logger.error({ clientId: client.id, err: e }, 'Auth failed');
      this.onAuthFailure?.(client.id, String(e));
      return false;
    }
  }
}
```

### Pattern 3: Message Registry (from CONTEXT.md)
**What:** Routing table mapping message types to handler modules
**When to use:** To replace the 30+ case switch statement in handleMessage
**Example:**
```typescript
// Source: Standard pattern for large switch replacement
// packages/server/src/coordinator/message-registry.ts

export type MessageHandler = (client: ClientConnection, message: any) => Promise<void>;

export interface MessageRegistry {
  // Auth-related
  AUTH: MessageHandler;

  // Query-related
  QUERY_SUB: MessageHandler;
  QUERY_UNSUB: MessageHandler;

  // Operation-related
  CLIENT_OP: MessageHandler;
  OP_BATCH: MessageHandler;

  // Sync-related
  SYNC_INIT: MessageHandler;
  MERKLE_REQ_BUCKET: MessageHandler;

  // ... other message types
}

// In ServerCoordinator:
private createMessageRegistry(): MessageRegistry {
  return {
    AUTH: (client, msg) => this.authHandler.handleAuth(client, msg.token),
    QUERY_SUB: (client, msg) => this.operationHandler.handleQuerySub(client, msg.payload),
    CLIENT_OP: (client, msg) => this.operationHandler.processOp(msg.payload, client.id),
    // ... map all 30+ message types
  };
}

private async handleMessage(client: ClientConnection, message: any) {
  const handler = this.messageRegistry[message.type as keyof MessageRegistry];
  if (handler) {
    await handler(client, message);
  } else {
    logger.warn({ type: message.type }, 'Unknown message type');
  }
}
```

### Pattern 4: Callback Functions for Cross-Module Events (from CONTEXT.md)
**What:** Modules communicate via callbacks passed in constructor, not direct calls
**When to use:** When one module needs to notify another of events
**Example:**
```typescript
// Source: Derived from existing TopicManager.sendToClient pattern
export interface ConnectionManagerConfig {
  // Callbacks for cross-module events
  onClientConnected?: (client: ClientConnection) => void;
  onClientDisconnected?: (clientId: string) => void;
  onClientAuthenticated?: (clientId: string, principal: Principal) => void;

  // Dependencies
  writeCoalescingEnabled: boolean;
  writeCoalescingOptions: Partial<CoalescingWriterOptions>;
  rateLimiter: ConnectionRateLimiter;
  metricsService: MetricsService;
}
```

### Anti-Patterns to Avoid
- **Circular dependencies between modules:** AuthHandler should not import ConnectionManager and vice versa. Use callbacks instead.
- **Passing ServerCoordinator instance to modules:** Creates tight coupling. Pass specific dependencies only.
- **Mixed state management:** CONTEXT.md specifies ConnectionManager is stateful (holds clients Map), AuthHandler is stateless. Don't mix.
- **Duplicating logic:** Don't copy methods - move them. Delete old code immediately after extraction.

## Don't Hand-Roll

Problems that look simple but should use existing solutions:

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| JWT verification | Custom token parsing | `jsonwebtoken` library (already in use) | Edge cases, algorithm support |
| Rate limiting | Simple counter | `ConnectionRateLimiter` (already exists) | Window management, cooldown |
| Write batching | Manual buffering | `CoalescingWriter` (already exists) | Timing, backpressure |
| WebSocket handling | Raw socket management | Keep using `ws` library patterns | Binary handling, close codes |

**Key insight:** All 4 modules should reuse existing utilities. The extraction is about reorganizing responsibility, not reimplementing functionality.

## Common Pitfalls

### Pitfall 1: Breaking Existing Tests During Extraction
**What goes wrong:** Tests that access private methods via `(server as any).methodName` break when methods move
**Why it happens:** Tests were written against implementation details
**How to avoid:**
1. Before extraction, identify all tests that access internals
2. Add public methods or test-only accessors if needed
3. Update tests incrementally with each module extraction
**Warning signs:** Test failures that mention "undefined is not a function"

### Pitfall 2: Losing Timing-Dependent Behavior
**What goes wrong:** The original handleMessage has careful ordering (auth before processing, interceptors, backpressure)
**Why it happens:** Order of operations is implicit in the 5086-line file
**How to avoid:**
1. Document the current flow before extraction
2. Keep orchestration in ServerCoordinator
3. Modules should be pure handlers, orchestrator controls sequence
**Warning signs:** Auth bypasses, missing interceptor calls, backpressure not working

### Pitfall 3: State Synchronization Issues
**What goes wrong:** ConnectionManager owns clients Map but other modules need to access it
**Why it happens:** Multiple modules need client state
**How to avoid:**
1. ConnectionManager provides read-only access via getClients()
2. Modifications only through ConnectionManager methods
3. Never store references to client objects outside ConnectionManager
**Warning signs:** Stale client references, "client not found" after disconnect

### Pitfall 4: Incomplete Message Type Routing
**What goes wrong:** Missing handler for one of the 30+ message types
**Why it happens:** Large switch statement makes it easy to miss during extraction
**How to avoid:**
1. Create exhaustive type for MessageRegistry
2. Add TypeScript exhaustiveness check
3. Log and fail on unknown message types (don't silently ignore)
**Warning signs:** "Unknown message type" warnings in logs

### Pitfall 5: Constructor Bloat
**What goes wrong:** ServerCoordinator constructor grows with all module initializations
**Why it happens:** Each module needs configuration
**How to avoid:**
1. Group related config into sub-objects
2. Create modules in `start()` or after server listen (when ports known)
3. Consider builder pattern if constructor exceeds 10 parameters
**Warning signs:** Constructor with 15+ parameters

## Code Examples

Verified patterns from the existing codebase:

### Existing Handler Pattern (CounterHandler)
```typescript
// Source: packages/server/src/handlers/CounterHandler.ts
export class CounterHandler {
  private counters: Map<string, PNCounterImpl> = new Map();
  private subscriptions: Map<string, Set<string>> = new Map();

  constructor(private readonly nodeId: string = 'server') {}

  // Returns response message, doesn't send directly
  handleCounterRequest(clientId: string, name: string): {
    type: string;
    payload: { name: string; state: PNCounterStateObject };
  } {
    const counter = this.getOrCreateCounter(name);
    this.subscribe(clientId, name);
    return {
      type: 'COUNTER_RESPONSE',
      payload: { name, state: this.stateToObject(counter.getState()) },
    };
  }
}
```

### Existing Manager Pattern (TopicManager)
```typescript
// Source: packages/server/src/topic/TopicManager.ts
export interface TopicManagerConfig {
  cluster: ClusterManager;
  sendToClient: (clientId: string, message: any) => void;
}

export class TopicManager {
  private subscribers: Map<string, Set<string>> = new Map();
  private cluster: ClusterManager;
  private sendToClient: (clientId: string, message: any) => void;

  constructor(config: TopicManagerConfig) {
    this.cluster = config.cluster;
    this.sendToClient = config.sendToClient;
  }

  public subscribe(clientId: string, topic: string) { /* ... */ }
  public publish(topic: string, data: any, senderId?: string) { /* ... */ }
}
```

### SecurityManager Pattern (Stateless Handler)
```typescript
// Source: packages/server/src/security/SecurityManager.ts
export class SecurityManager {
  private policies: PermissionPolicy[] = [];

  constructor(policies: PermissionPolicy[] = []) {
    this.policies = policies;
  }

  public checkPermission(principal: Principal, mapName: string, action: PermissionType): boolean {
    // Pure function - no side effects, no dependencies on external state
  }

  public filterObject(object: any, principal: Principal, mapName: string): any {
    // Pure function - filters based on policies
  }
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| God objects | Extract class refactoring | Established pattern | Better maintainability |
| Global dependencies | Constructor injection | Standard since ~2015 | Testability |
| Switch statements | Registry/map patterns | Common in Node.js | Extensibility |

**Deprecated/outdated:**
- Service locator pattern: Replaced by constructor injection for better testability
- Singleton handlers: Use instance injection instead for testing

## Open Questions

Things that couldn't be fully resolved:

1. **Extraction Order**
   - What we know: CONTEXT.md says "One plan per module"
   - What's unclear: Optimal order based on dependency analysis
   - Recommendation: AuthHandler (no deps on others) -> ConnectionManager (needs auth callback) -> StorageManager (needs connection for broadcast) -> OperationHandler (needs all three)

2. **Test Update Strategy**
   - What we know: Tests exist that use `(server as any).handleMessage` patterns
   - What's unclear: How many tests need updating, effort level
   - Recommendation: Survey tests before starting, document breaking changes per module

3. **Metrics Integration**
   - What we know: MetricsService is passed throughout
   - What's unclear: Which module should own which metrics
   - Recommendation: Keep metrics in orchestrator, modules call callbacks for events

## Sources

### Primary (HIGH confidence)
- `/Users/koristuvac/Projects/topgun/topgun/packages/server/src/ServerCoordinator.ts` - Source file being refactored (5086 lines)
- `/Users/koristuvac/Projects/topgun/topgun/packages/server/src/handlers/CounterHandler.ts` - Existing handler pattern
- `/Users/koristuvac/Projects/topgun/topgun/packages/server/src/topic/TopicManager.ts` - Existing manager pattern with DI
- `/Users/koristuvac/Projects/topgun/topgun/packages/server/src/security/SecurityManager.ts` - Existing stateless handler pattern
- `/Users/koristuvac/Projects/topgun/topgun/.planning/phases/04-servercoordinator-refactor/04-CONTEXT.md` - User decisions

### Secondary (MEDIUM confidence)
- [Refactoring Guru: Large Class](https://refactoring.guru/smells/large-class) - Extract Class pattern
- [CodeSignal: Extract Class](https://codesignal.com/learn/courses/refactoring-by-leveraging-your-tests-with-typescript-jest/lessons/large-class-extract-class) - Large class refactoring with tests
- [RisingStack: Dependency Injection in Node.js](https://blog.risingstack.com/dependency-injection-in-node-js/) - DI patterns

### Tertiary (LOW confidence)
- [DEV: TypeScript Best Practices 2025](https://dev.to/mitu_mariam/typescript-best-practices-in-2025-57hb) - General TypeScript patterns
- [Snyk: Dependency Injection in JavaScript](https://snyk.io/blog/dependency-injection-in-javascript/) - DI overview

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH - Uses existing codebase patterns, no new deps
- Architecture: HIGH - Based on existing handler/manager patterns in codebase
- Pitfalls: HIGH - Derived from analyzing actual ServerCoordinator code
- Module boundaries: MEDIUM - Depends on implementation details to confirm

**Research date:** 2026-01-18
**Valid until:** 60 days (internal refactoring, stable codebase patterns)

## Message Type Grouping Reference

Based on analysis of ServerCoordinator.handleMessage switch statement (30+ message types):

### AuthHandler Messages
- `AUTH` - JWT authentication

### ConnectionManager Messages
- `PING` - Heartbeat (already handled separately)
- No other direct messages (connection lifecycle is socket events)

### OperationHandler Messages
- `CLIENT_OP` - Single operation
- `OP_BATCH` - Batch operations
- `QUERY_SUB` - Query subscription
- `QUERY_UNSUB` - Query unsubscription
- `LOCK_REQUEST` - Distributed lock
- `LOCK_RELEASE` - Lock release
- `TOPIC_SUB` - Topic subscription
- `TOPIC_UNSUB` - Topic unsubscription
- `TOPIC_PUB` - Topic publish
- `COUNTER_REQUEST` - PN Counter
- `COUNTER_SYNC` - Counter sync
- `ENTRY_PROCESS` - Entry processor
- `ENTRY_PROCESS_BATCH` - Batch processor
- `REGISTER_RESOLVER` - Conflict resolver
- `UNREGISTER_RESOLVER` - Remove resolver
- `LIST_RESOLVERS` - List resolvers
- `PARTITION_MAP_REQUEST` - Partition topology
- `SEARCH` - Full-text search
- `SEARCH_SUB` - Search subscription
- `SEARCH_UNSUB` - Search unsubscription

### StorageManager Messages (Sync Protocol)
- `SYNC_INIT` - Begin sync
- `MERKLE_REQ_BUCKET` - Merkle tree bucket request
- `ORMAP_SYNC_INIT` - ORMap sync
- `ORMAP_MERKLE_REQ_BUCKET` - ORMap bucket request
- `ORMAP_DIFF_REQUEST` - ORMap diff
- `ORMAP_PUSH_DIFF` - ORMap push

### Event Journal Messages (keep in ServerCoordinator or separate JournalHandler)
- `JOURNAL_SUBSCRIBE`
- `JOURNAL_UNSUBSCRIBE`
- `JOURNAL_READ`

**Grouping rationale:**
- AuthHandler: Only auth message, stateless
- ConnectionManager: No messages, handles socket events
- OperationHandler: All data mutation and query messages
- StorageManager: All sync/Merkle protocol messages
