# SPEC-011d: Handlers Module + MessageRegistry

---
id: SPEC-011d
parent: SPEC-011
type: refactor
status: draft
priority: high
complexity: medium
depends_on: [SPEC-011a, SPEC-011b, SPEC-011c]
created: 2026-01-30
---

> Part 4 of 5 from SPEC-011 (ServerFactory Modularization for Rust-Portability)

## Context

This is the **largest sub-specification** in the SPEC-011 series. It extracts 25+ handlers from ServerFactory and groups them by domain for Rust Actor Model portability.

### Current State (Lines to Extract)

```
Lines 374-708: Handler instantiation (~30 handlers, 335 lines)
  - AuthHandler, OperationHandler, BatchProcessingHandler, GCHandler
  - LwwSyncHandler, ORMapSyncHandler
  - QueryHandler, QueryConversionHandler
  - TopicHandler, LockHandler, PartitionHandler
  - SearchHandler, JournalHandler, CounterHandlerAdapter
  - ResolverHandler, EntryProcessorAdapter
  - BroadcastHandler, HeartbeatHandler
  - WebSocketHandler, ClientMessageHandler
  - PersistenceHandler, OperationContextHandler, WriteConcernHandler
  - ClusterEventHandler (requires late binding)

Lines 710-779: MessageRegistry creation
  - Routes all 35 message types to handlers
```

## Task

Extract all handler creation into a domain-grouped module factory:
1. Group handlers by domain: CRDT, Sync, Query, Topic, Lock, Search, Journal, etc.
2. Create factory functions for each group
3. Create `modules/handlers-module.ts` that assembles all groups
4. Create MessageRegistry with routes to grouped handlers
5. Prepare enum-like message routes for Rust translation

## Requirements

### R1: Handler Group Interfaces (update `modules/types.ts`)

```typescript
// CRDT handlers - conflict resolution and operations
export interface CRDTHandlers {
  operationHandler: OperationHandler;
  batchProcessingHandler: BatchProcessingHandler;
  gcHandler: GCHandler;
}

// Sync handlers - merkle tree and OR-Map sync
export interface SyncHandlers {
  lwwSyncHandler: LwwSyncHandler;
  orMapSyncHandler: ORMapSyncHandler;
}

// Query handlers - subscriptions and conversions
export interface QueryHandlers {
  queryHandler: QueryHandler;
  queryConversionHandler: QueryConversionHandler;
}

// Messaging handlers - topics, broadcast
export interface MessagingHandlers {
  topicHandler: TopicHandler;
  broadcastHandler: BroadcastHandler;
}

// Coordination handlers - locks, partitions
export interface CoordinationHandlers {
  lockHandler: LockHandler;
  partitionHandler: PartitionHandler;
}

// Search handlers
export interface SearchHandlers {
  searchHandler: SearchHandler;
}

// Persistence handlers - journals, counters, entry processors
export interface PersistenceHandlers {
  journalHandler: JournalHandler;
  counterHandler: CounterHandlerAdapter;
  entryProcessorHandler: EntryProcessorAdapter;
  resolverHandler: ResolverHandler;
}

// Client handlers - auth, websocket, client messages
export interface ClientHandlers {
  authHandler: AuthHandler;
  webSocketHandler: WebSocketHandler;
  clientMessageHandler: ClientMessageHandler;
}

// Server handlers - heartbeat, persistence, write concern
export interface ServerHandlers {
  heartbeatHandler: HeartbeatHandler;
  persistenceHandler: PersistenceHandler;
  operationContextHandler: OperationContextHandler;
  writeConcernHandler: WriteConcernHandler;
}

// Cluster handlers - cluster events (late binding)
export interface ClusterHandlers {
  clusterEventHandler: ClusterEventHandler;
}

// All handlers combined
export interface HandlersModule {
  crdt: CRDTHandlers;
  sync: SyncHandlers;
  query: QueryHandlers;
  messaging: MessagingHandlers;
  coordination: CoordinationHandlers;
  search: SearchHandlers;
  persistence: PersistenceHandlers;
  client: ClientHandlers;
  server: ServerHandlers;
  cluster: ClusterHandlers;
  messageRegistry: MessageRegistry;
}

export interface HandlersModuleConfig {
  nodeId: string;
  jwtSecret: string;
  rateLimitingEnabled?: boolean;
  writeCoalescingEnabled?: boolean;
  writeCoalescingOptions?: Partial<CoalescingWriterOptions>;
  interceptors?: IInterceptor[];
  storage?: IServerStorage;
  eventJournalEnabled?: boolean;
  eventJournalConfig?: Partial<EventJournalServiceConfig>;
}

export interface HandlersModuleDeps {
  core: CoreModule;
  network: NetworkModule;
  cluster: ClusterModule;
  storage: StorageModule;
  workers: WorkerModule;
}
```

### R2: Handler Group Factories (`modules/handlers-module.ts`)

Create factory functions for each handler group:

```typescript
// CRDT handlers
function createCRDTHandlers(
  config: HandlersModuleConfig,
  deps: HandlersModuleDeps
): CRDTHandlers {
  const operationHandler = new OperationHandler({
    hlc: deps.core.hlc,
    storageManager: deps.storage.storageManager,
    metricsService: deps.core.metricsService,
    eventExecutor: deps.core.eventExecutor,
    // ...
  });

  const batchProcessingHandler = new BatchProcessingHandler({
    hlc: deps.core.hlc,
    storageManager: deps.storage.storageManager,
    // ...
  });

  const gcHandler = new GCHandler({
    storageManager: deps.storage.storageManager,
    cluster: deps.cluster.cluster,
    // ...
  });

  return { operationHandler, batchProcessingHandler, gcHandler };
}

// Sync handlers
function createSyncHandlers(
  config: HandlersModuleConfig,
  deps: HandlersModuleDeps
): SyncHandlers {
  const lwwSyncHandler = new LwwSyncHandler({
    storageManager: deps.storage.storageManager,
    hlc: deps.core.hlc,
    // ...
  });

  const orMapSyncHandler = new ORMapSyncHandler({
    storageManager: deps.storage.storageManager,
    hlc: deps.core.hlc,
    // ...
  });

  return { lwwSyncHandler, orMapSyncHandler };
}

// Query handlers
function createQueryHandlers(
  config: HandlersModuleConfig,
  deps: HandlersModuleDeps
): QueryHandlers {
  const queryHandler = new QueryHandler({
    queryRegistry: deps.storage.queryRegistry,
    storageManager: deps.storage.storageManager,
    // ...
  });

  const queryConversionHandler = new QueryConversionHandler({
    cluster: deps.cluster.cluster,
    partitionService: deps.cluster.partitionService,
    // ...
  });

  return { queryHandler, queryConversionHandler };
}

// ... similar factories for other groups
```

### R3: Main Handlers Module Factory

```typescript
export function createHandlersModule(
  config: HandlersModuleConfig,
  deps: HandlersModuleDeps
): HandlersModule {
  // Create all handler groups
  const crdt = createCRDTHandlers(config, deps);
  const sync = createSyncHandlers(config, deps);
  const query = createQueryHandlers(config, deps);
  const messaging = createMessagingHandlers(config, deps);
  const coordination = createCoordinationHandlers(config, deps);
  const search = createSearchHandlers(config, deps);
  const persistence = createPersistenceHandlers(config, deps);
  const client = createClientHandlers(config, deps);
  const server = createServerHandlers(config, deps);
  const cluster = createClusterHandlers(config, deps);

  // Create MessageRegistry with all routes
  const messageRegistry = createMessageRegistry({
    onClientOp: (client, msg) => crdt.operationHandler.processClientOp(client, msg.payload),
    onOpBatch: (client, msg) => crdt.batchProcessingHandler.processBatch(client, msg.payload),
    onQuerySub: (client, msg) => query.queryHandler.handleQuerySub(client, msg.payload),
    // ... all 35 message type routes
  });

  return {
    crdt,
    sync,
    query,
    messaging,
    coordination,
    search,
    persistence,
    client,
    server,
    cluster,
    messageRegistry,
  };
}
```

### R4: Message Route Enum Structure

Prepare for Rust `enum` + `match` translation:

```typescript
// Message types as const enum for Rust-like pattern
export const MessageType = {
  CLIENT_OP: 'CLIENT_OP',
  OP_BATCH: 'OP_BATCH',
  QUERY_SUB: 'QUERY_SUB',
  QUERY_UNSUB: 'QUERY_UNSUB',
  SYNC_INIT: 'SYNC_INIT',
  MERKLE_REQ_BUCKET: 'MERKLE_REQ_BUCKET',
  // ... all 35 message types
} as const;

export type MessageType = typeof MessageType[keyof typeof MessageType];

// Handler reference for documentation (maps to Rust Actor addresses)
interface HandlerRef {
  group: keyof HandlersModule;
  handler: string;
  method: string;
}

// Route table (for documentation, not runtime)
export const MESSAGE_ROUTES: Record<MessageType, HandlerRef> = {
  [MessageType.CLIENT_OP]: { group: 'crdt', handler: 'operationHandler', method: 'processClientOp' },
  [MessageType.OP_BATCH]: { group: 'crdt', handler: 'batchProcessingHandler', method: 'processBatch' },
  [MessageType.QUERY_SUB]: { group: 'query', handler: 'queryHandler', method: 'handleQuerySub' },
  // ... all routes
};
```

### R5: Late Binding for ClusterEventHandler

ClusterEventHandler requires ServerCoordinator callbacks. Use late binding:

```typescript
function createClusterHandlers(
  config: HandlersModuleConfig,
  deps: HandlersModuleDeps
): ClusterHandlers {
  // ClusterEventHandler needs callbacks set later
  const clusterEventHandler = new ClusterEventHandler({
    cluster: deps.cluster.cluster,
    partitionService: deps.cluster.partitionService,
    // callbacks set via setCallbacks() after coordinator creation
  });

  return { clusterEventHandler };
}

// In ServerFactory, after coordinator creation:
handlers.cluster.clusterEventHandler.setCallbacks({
  onNodeJoin: coordinator.handleNodeJoin.bind(coordinator),
  onNodeLeave: coordinator.handleNodeLeave.bind(coordinator),
  // ...
});
```

## Files

### Files to Create

| File | Purpose |
|------|---------|
| `packages/server/src/modules/handlers-module.ts` | All handlers grouped by domain, MessageRegistry |

### Files to Modify

| File | Change |
|------|--------|
| `packages/server/src/modules/types.ts` | Add all handler group interfaces |
| `packages/server/src/modules/index.ts` | Export handlers-module |
| `packages/server/src/ServerFactory.ts` | Use createHandlersModule, set late binding callbacks |

## Acceptance Criteria

### Handler Grouping
1. [ ] Handlers are grouped into: CRDT, Sync, Query, Messaging, Coordination, Search, Persistence, Client, Server, Cluster
2. [ ] Each group has its own factory function in handlers-module.ts
3. [ ] All 25+ handlers are created through group factories

### MessageRegistry
4. [ ] MessageRegistry is created inside createHandlersModule
5. [ ] All 35 message types are routed to appropriate handlers
6. [ ] Routes reference grouped handlers (e.g., `crdt.operationHandler`)

### Message Routes
7. [ ] MessageType const enum exports all message types
8. [ ] MESSAGE_ROUTES documents handler references (group, handler, method)

### Late Binding
9. [ ] ClusterEventHandler uses late binding pattern
10. [ ] Callbacks are set after ServerCoordinator creation

### Compatibility
11. [ ] All 203+ existing tests pass
12. [ ] Build passes (`pnpm build`)
13. [ ] No circular dependencies
14. [ ] TypeScript strict mode passes

## Constraints

- **No Breaking Changes**: All handler behavior must be identical
- **Config Mapping**: Handler configs must map correctly from ServerCoordinatorConfig
- **Late Binding**: ClusterEventHandler callbacks must be set after coordinator creation

## Assumptions

1. All handler config interfaces already exist in codebase
2. MessageRegistry interface is stable
3. Late binding pattern from SPEC-004 is used for ClusterEventHandler
4. WebSocketHandler is created inside createClientHandlers
5. Event journal creation is conditional based on config

---
*Created by SpecFlow split from SPEC-011 on 2026-01-30*
