# SPEC-011e: Search + Lifecycle + Final Assembly

---
id: SPEC-011e
parent: SPEC-011
type: refactor
status: draft
priority: high
complexity: small
depends_on: [SPEC-011d]
created: 2026-01-30
---

> Part 5 of 5 from SPEC-011 (ServerFactory Modularization for Rust-Portability)

## Context

This is the **final sub-specification** that completes the ServerFactory modularization. After SPEC-011a-d extract core, cluster, storage, network, and handlers modules, this spec:
1. Extracts search coordinators into `search-module.ts`
2. Extracts lifecycle management into `lifecycle-module.ts`
3. Reduces `ServerFactory.create()` to ~100 lines of pure assembly

### Current State (Lines to Extract)

```
Lines 344-368: Search coordinators
  - SearchCoordinator
  - ClusterSearchCoordinator
  - DistributedSubscriptionCoordinator

Lines 781-859: LifecycleManager creation
  - Shutdown hooks for all modules
  - Graceful shutdown ordering
```

### Goal

After this spec, `ServerFactory.create()` becomes:
1. Create modules in dependency order
2. Assemble ServerCoordinator
3. Set late binding callbacks
4. Start network (deferred)
5. Return coordinator

**Target: ~50-100 lines** (down from 893 lines)

## Task

1. Create `modules/search-module.ts` with search coordinators
2. Create `modules/lifecycle-module.ts` with LifecycleManager
3. Reduce ServerFactory.create() to assembly-only
4. Verify all Observable Truths from SPEC-011 Goal Analysis

## Requirements

### R1: Search Module Types (update `modules/types.ts`)

```typescript
export interface SearchModuleConfig {
  nodeId: string;
  fullTextSearch?: Record<string, FullTextIndexConfig>;
  distributedSearch?: DistributedSearchConfig;
}

export interface SearchModuleDeps {
  cluster: ClusterManager;
  partitionService: PartitionService;
  queryRegistry: QueryRegistry;
  metricsService: MetricsService;
}

export interface SearchModule {
  searchCoordinator: SearchCoordinator;
  clusterSearchCoordinator?: ClusterSearchCoordinator;
  distributedSubCoordinator?: DistributedSubscriptionCoordinator;
}
```

### R2: Search Module Factory (`modules/search-module.ts`)

```typescript
export function createSearchModule(
  config: SearchModuleConfig,
  deps: SearchModuleDeps
): SearchModule {
  const searchCoordinator = new SearchCoordinator();

  // Configure full-text search
  if (config.fullTextSearch) {
    for (const [mapName, ftsConfig] of Object.entries(config.fullTextSearch)) {
      searchCoordinator.enableSearch(mapName, ftsConfig);
    }
  }
  searchCoordinator.setNodeId(config.nodeId);

  // Create cluster search coordinator
  const clusterSearchCoordinator = new ClusterSearchCoordinator(
    deps.cluster,
    deps.partitionService,
    searchCoordinator,
    config.distributedSearch,
    deps.metricsService
  );

  // Create distributed subscription coordinator
  const distributedSubCoordinator = new DistributedSubscriptionCoordinator(
    deps.cluster,
    deps.queryRegistry,
    searchCoordinator,
    undefined,
    deps.metricsService
  );

  return {
    searchCoordinator,
    clusterSearchCoordinator,
    distributedSubCoordinator,
  };
}
```

### R3: Lifecycle Module Types (update `modules/types.ts`)

```typescript
export interface LifecycleModuleConfig {
  nodeId: string;
  gracefulShutdownTimeoutMs?: number;
}

export interface LifecycleModuleDeps {
  core: CoreModule;
  network: NetworkModule;
  cluster: ClusterModule;
  storage: StorageModule;
  workers: WorkerModule;
  handlers: HandlersModule;
  search: SearchModule;
}

export interface LifecycleModule {
  lifecycleManager: LifecycleManager;
}
```

### R4: Lifecycle Module Factory (`modules/lifecycle-module.ts`)

```typescript
export function createLifecycleModule(
  config: LifecycleModuleConfig,
  deps: LifecycleModuleDeps
): LifecycleModule {
  const lifecycleManager = new LifecycleManager({
    nodeId: config.nodeId,
    gracefulShutdownTimeoutMs: config.gracefulShutdownTimeoutMs ?? 30000,

    // Network shutdown
    httpServer: deps.network.httpServer,
    metricsServer: deps.network.metricsServer,
    wss: deps.network.wss,

    // Core shutdown
    metricsService: { destroy: () => deps.core.metricsService.destroy() },
    eventExecutor: { shutdown: (wait) => deps.core.eventExecutor.shutdown(wait) },

    // Cluster shutdown
    cluster: deps.cluster.cluster,
    replicationPipeline: deps.cluster.replicationPipeline,
    repairScheduler: deps.cluster.repairScheduler,
    partitionReassigner: deps.cluster.partitionReassigner,

    // Storage shutdown
    storageManager: deps.storage.storageManager,
    taskletScheduler: deps.storage.taskletScheduler,

    // Worker shutdown
    workerPool: deps.workers.workerPool,

    // Handler shutdown (those with stop methods)
    queryConversionHandler: deps.handlers.query.queryConversionHandler,
    heartbeatHandler: deps.handlers.server.heartbeatHandler,
    gcHandler: deps.handlers.crdt.gcHandler,
    // ... other handlers with cleanup
  });

  return { lifecycleManager };
}
```

### R5: Assembly-Only ServerFactory

Final `ServerFactory.create()` — target ~50-100 lines:

```typescript
export class ServerFactory {
  static create(config: ServerCoordinatorConfig): ServerCoordinator {
    // Step 1: Validate config
    const jwtSecret = validateAndProcessJwtSecret(config.jwtSecret);

    // Step 2: Create modules in dependency order
    const core = createCoreModule({
      nodeId: config.nodeId,
      eventStripeCount: config.eventStripeCount,
      eventQueueCapacity: config.eventQueueCapacity,
      backpressureEnabled: config.backpressureEnabled,
      securityPolicies: config.securityPolicies,
    });

    const workers = createWorkersModule({
      nodeId: config.nodeId,
      useWorkerPool: config.useWorkerPool,
      workerPoolSize: config.workerPoolSize,
    });

    // ConnectionManager (used by both network and handlers)
    const connectionManager = new ConnectionManager({
      hlc: core.hlc,
      writeCoalescingEnabled: config.writeCoalescingEnabled ?? true,
      writeCoalescingOptions: getWriteCoalescingOptions(config),
    });

    const cluster = createClusterModule(
      { nodeId: config.nodeId, ...extractClusterConfig(config) },
      { hlc: core.hlc }
    );

    const storage = createStorageModule(
      { nodeId: config.nodeId, ...extractStorageConfig(config) },
      { hlc: core.hlc, metricsService: core.metricsService, partitionService: cluster.partitionService }
    );

    const search = createSearchModule(
      { nodeId: config.nodeId, ...extractSearchConfig(config) },
      { cluster: cluster.cluster, partitionService: cluster.partitionService, queryRegistry: storage.queryRegistry, metricsService: core.metricsService }
    );

    const network = createNetworkModule(
      { port: config.port, jwtSecret, ...extractNetworkConfig(config) },
      { metricsService: core.metricsService, storageManager: storage.storageManager, cluster: cluster.cluster, partitionService: cluster.partitionService, connectionManager }
    );

    const handlers = createHandlersModule(
      { nodeId: config.nodeId, jwtSecret, ...extractHandlersConfig(config) },
      { core, network, cluster, storage, workers }
    );

    const lifecycle = createLifecycleModule(
      { nodeId: config.nodeId, gracefulShutdownTimeoutMs: config.gracefulShutdownTimeoutMs },
      { core, network, cluster, storage, workers, handlers, search }
    );

    // Step 3: Assemble ServerCoordinator
    const coordinator = new ServerCoordinator(config, {
      ...flattenModules({ core, network, cluster, storage, workers, handlers, search, lifecycle }),
      connectionManager,
      jwtSecret,
    });

    // Step 4: Set late binding callbacks
    handlers.cluster.clusterEventHandler.setCallbacks({
      onNodeJoin: coordinator.handleNodeJoin.bind(coordinator),
      onNodeLeave: coordinator.handleNodeLeave.bind(coordinator),
      // ...
    });

    handlers.crdt.gcHandler.setBroadcastCallback(
      coordinator.broadcast.bind(coordinator)
    );

    // Step 5: DEFERRED startup (after assembly)
    network.start();

    return coordinator;
  }
}

// Helper functions for config extraction
function extractClusterConfig(config: ServerCoordinatorConfig) { /* ... */ }
function extractStorageConfig(config: ServerCoordinatorConfig) { /* ... */ }
function extractSearchConfig(config: ServerCoordinatorConfig) { /* ... */ }
function extractNetworkConfig(config: ServerCoordinatorConfig) { /* ... */ }
function extractHandlersConfig(config: ServerCoordinatorConfig) { /* ... */ }
function flattenModules(modules: ServerModules) { /* ... */ }
```

### R6: Verify Observable Truths

After completion, verify all truths from SPEC-011 Goal Analysis:

| # | Truth | Verification Method |
|---|-------|---------------------|
| T1 | `ServerFactory.create()` is under 100 lines | `wc -l ServerFactory.ts` after removing extracted code |
| T2 | Each module factory is independently testable | Unit tests exist for each module |
| T3 | HTTP/WebSocket servers start only after full assembly | No `.listen()` in factory functions |
| T4 | All 203+ existing tests pass | `pnpm test` |
| T5 | TypeScript interfaces define explicit module inputs/outputs | `modules/types.ts` exports |
| T6 | Handlers are grouped by domain | Handler factory grouping in handlers-module.ts |
| T7 | Message routes are enum-like | MessageType const in handlers-module.ts |

## Files

### Files to Create

| File | Purpose |
|------|---------|
| `packages/server/src/modules/search-module.ts` | SearchCoordinator, ClusterSearch |
| `packages/server/src/modules/lifecycle-module.ts` | LifecycleManager assembly |

### Files to Modify

| File | Change |
|------|--------|
| `packages/server/src/modules/types.ts` | Add SearchModule, LifecycleModule interfaces |
| `packages/server/src/modules/index.ts` | Export search-module, lifecycle-module |
| `packages/server/src/ServerFactory.ts` | Reduce to ~100 lines assembly-only |
| `packages/server/src/ServerDependencies.ts` | Import/use module types |
| `packages/server/src/index.ts` | Export module types for consumers |

## Acceptance Criteria

### Search Module
1. [ ] `modules/search-module.ts` exports `createSearchModule(config, deps)` function
2. [ ] SearchCoordinator, ClusterSearchCoordinator, DistributedSubscriptionCoordinator are created
3. [ ] Full-text search configuration is applied correctly

### Lifecycle Module
4. [ ] `modules/lifecycle-module.ts` exports `createLifecycleModule(config, deps)` function
5. [ ] LifecycleManager receives all shutdown hooks from all modules
6. [ ] Graceful shutdown ordering is preserved

### Assembly
7. [ ] `ServerFactory.create()` is under 100 lines
8. [ ] ServerFactory only calls module factories + assembly logic
9. [ ] Late binding callbacks are set after coordinator creation
10. [ ] `network.start()` is called after assembly

### Observable Truths
11. [ ] T1: ServerFactory.create() < 100 lines ✓
12. [ ] T2: Each module has unit test ✓
13. [ ] T3: No .listen() in factories ✓
14. [ ] T4: All 203+ tests pass ✓
15. [ ] T5: types.ts exports all interfaces ✓
16. [ ] T6: Handlers grouped by domain ✓
17. [ ] T7: MessageType enum exists ✓

### Compatibility
18. [ ] All 203+ existing tests pass
19. [ ] Build passes (`pnpm build`)
20. [ ] Public API of ServerCoordinator unchanged
21. [ ] No circular dependencies
22. [ ] TypeScript strict mode passes

## Constraints

- **No Breaking Changes**: Public API must remain identical
- **Config Helpers**: Use helper functions for config extraction to keep assembly clean
- **Flat Dependencies**: ServerCoordinator constructor signature may need adjustment

## Assumptions

1. ServerCoordinator constructor can accept flattened module dependencies
2. Config extraction helpers are small utility functions, not separate modules
3. Module exports are added to packages/server/src/index.ts for consumers

---
*Created by SpecFlow split from SPEC-011 on 2026-01-30*
