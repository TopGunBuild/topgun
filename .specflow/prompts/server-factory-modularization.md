# Specification Prompt: ServerFactory Modularization for Rust-Portability

## Context

The `packages/server/src/ServerFactory.ts` (1,066 lines) contains a single `create()` method spanning 893 lines that:
- Creates ~45 objects with complex interdependencies
- Establishes ~25 callback bindings via `.bind()`
- Starts HTTP/WebSocket servers before ServerCoordinator is created
- Has implicit initialization order dependencies
- Is untestable in isolation

This architecture is problematic for:
1. **Maintenance**: Changes require understanding the entire 893-line method
2. **Testing**: Cannot test individual modules in isolation
3. **Error Recovery**: Resources start before coordinator exists (leak on failure)
4. **Rust Portability**: Mutable state + callbacks don't translate to Rust ownership model

## Objective

Refactor `ServerFactory` into a **modular, Rust-portable architecture** using:
- **Module Factory Pattern**: Each domain gets its own factory function
- **Explicit Dependency Graph**: Dependencies passed as parameters, not created inline
- **Deferred Resource Startup**: Servers start only after full assembly
- **Message-Passing Ready**: Prepare for Actor Model / Channel-based communication

## Target Architecture

```
ServerFactory.create(config)
  │
  ├── createCoreModule(config)           → { hlc, metricsService, securityManager }
  │
  ├── createNetworkModule(config, core)  → { httpServer, wss, rateLimiter }
  │
  ├── createClusterModule(config, core)  → { cluster, partitionService, replication }
  │
  ├── createStorageModule(config, core, cluster) → { storageManager, queryRegistry }
  │
  ├── createHandlersModule(config, ...)  → { all 25+ handlers }
  │
  ├── createSearchModule(config, ...)    → { searchCoordinator, clusterSearch }
  │
  └── assembleServerCoordinator(modules) → ServerCoordinator
      └── startListening()               → void (deferred startup)
```

## Requirements

### R1: Module Factory Functions
Create separate factory functions in `packages/server/src/modules/`:
- `core-module.ts` - HLC, MetricsService, SecurityManager, EventExecutor
- `network-module.ts` - HTTP server, WebSocket server, RateLimiter (NO listening yet)
- `cluster-module.ts` - ClusterManager, PartitionService, ReplicationPipeline
- `storage-module.ts` - StorageManager, QueryRegistry
- `handlers-module.ts` - All message handlers (grouped by domain)
- `search-module.ts` - SearchCoordinator, ClusterSearchCoordinator
- `lifecycle-module.ts` - LifecycleManager assembly

### R2: Explicit Dependency Types
Define TypeScript interfaces for each module's inputs and outputs:
```typescript
// modules/types.ts
export interface CoreModule {
  hlc: HLC;
  metricsService: MetricsService;
  securityManager: SecurityManager;
  eventExecutor: StripedEventExecutor;
  backpressure: BackpressureRegulator;
}

export interface NetworkModule {
  httpServer: HttpServer;
  wss: WebSocketServer;
  rateLimiter: ConnectionRateLimiter;
  start: () => void;  // Deferred startup
}
// ... etc
```

### R3: Deferred Server Startup
Servers must NOT call `.listen()` during creation:
```typescript
// BAD (current):
httpServer.listen(config.port);  // Line 278

// GOOD (target):
return {
  httpServer,
  start: () => httpServer.listen(config.port),
};
```

### R4: Handler Grouping (Rust-Portable)
Group handlers by domain for future Actor mapping:
```typescript
// handlers/crdt-handlers.ts
export function createCRDTHandlers(deps): CRDTHandlers {
  return {
    operation: new OperationHandler(deps),
    batch: new BatchProcessingHandler(deps),
    gc: new GCHandler(deps),
  };
}

// handlers/sync-handlers.ts
export function createSyncHandlers(deps): SyncHandlers {
  return {
    lww: new LwwSyncHandler(deps),
    ormap: new ORMapSyncHandler(deps),
    merkle: /* ... */,
  };
}
```

### R5: Message Registry as Enum-like Structure
Prepare MessageRegistry for Rust `enum` + `match` translation:
```typescript
// Current: callback-based
const messageRegistry = createMessageRegistry({
  onClientOp: (client, msg) => operationHandler.processClientOp(client, msg.payload),
  // ... 25+ callbacks
});

// Target: declarative routing table
type MessageType = 'CLIENT_OP' | 'OP_BATCH' | 'QUERY_SUB' | /* ... */;

const MESSAGE_ROUTES: Record<MessageType, HandlerRef> = {
  'CLIENT_OP': { handler: 'crdt', method: 'processClientOp' },
  'OP_BATCH': { handler: 'crdt', method: 'processBatch' },
  // ...
};
```

### R6: Dependency Injection Preparation
Each handler should receive dependencies via constructor interface, not inline callbacks:
```typescript
// BAD (current - line 416-459):
const operationHandler = new OperationHandler({
  securityManager: {
    checkPermission: securityManager.checkPermission.bind(securityManager),
  },
  // ... 15 more inline objects
});

// GOOD (target):
interface OperationHandlerDeps {
  securityManager: ISecurityManager;
  storageManager: IStorageManager;
  // ... typed interfaces
}
const operationHandler = new OperationHandler(deps);
```

### R7: ServerFactory Becomes Assembly Only
Final `ServerFactory.create()` should be ~50-100 lines:
```typescript
static create(config: ServerCoordinatorConfig): ServerCoordinator {
  const core = createCoreModule(config);
  const network = createNetworkModule(config, core);
  const cluster = createClusterModule(config, core);
  const storage = createStorageModule(config, core, cluster);
  const handlers = createHandlersModule(config, core, cluster, storage);
  const search = createSearchModule(config, core, cluster, storage);

  const coordinator = new ServerCoordinator(config, {
    ...core, ...network, ...cluster, ...storage, ...handlers, ...search,
  });

  // Deferred startup
  network.start();
  cluster.start();

  return coordinator;
}
```

## Constraints

- **No Breaking Changes**: Public API of `ServerCoordinator` must remain unchanged
- **Incremental Refactoring**: Each module can be extracted in a separate PR
- **Test Coverage**: Each module factory must have unit tests
- **No New Dependencies**: Use only existing packages

## Success Criteria

1. `ServerFactory.create()` reduced to <100 lines
2. Each module factory testable in isolation
3. All 203 existing tests pass
4. No resource leaks on initialization failure
5. TypeScript interfaces ready for Rust FFI generation

## Files to Modify

- `packages/server/src/ServerFactory.ts` - Refactor to assembly-only
- `packages/server/src/ServerCoordinator.ts` - Accept module interfaces
- `packages/server/src/ServerDependencies.ts` - Update types

## Files to Create

- `packages/server/src/modules/index.ts`
- `packages/server/src/modules/types.ts`
- `packages/server/src/modules/core-module.ts`
- `packages/server/src/modules/network-module.ts`
- `packages/server/src/modules/cluster-module.ts`
- `packages/server/src/modules/storage-module.ts`
- `packages/server/src/modules/handlers-module.ts`
- `packages/server/src/modules/search-module.ts`
- `packages/server/src/modules/lifecycle-module.ts`

## Rust Portability Notes

This refactoring prepares the codebase for eventual Rust port by:
1. **Eliminating callback soup**: Rust doesn't have `.bind()`, prefers channels
2. **Explicit ownership**: Module factories return owned objects, no shared mutable state
3. **Enum-based routing**: `MessageType` enum maps directly to Rust `enum` + `match`
4. **Actor boundaries**: Each handler group becomes a potential Rust Actor
5. **Typed interfaces**: Can generate Rust traits from TypeScript interfaces

## Execution Notes

- Start with `core-module.ts` as it has no dependencies
- Extract `network-module.ts` second (enables deferred startup fix)
- `handlers-module.ts` is largest, can be split into sub-PRs
- Keep `ServerFactory.create()` working at each step (incremental)
