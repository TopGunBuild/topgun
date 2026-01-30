# SPEC-011b: Cluster + Storage Modules

---
id: SPEC-011b
parent: SPEC-011
type: refactor
status: draft
priority: high
complexity: small
depends_on: [SPEC-011a]
created: 2026-01-30
---

> Part 2 of 5 from SPEC-011 (ServerFactory Modularization for Rust-Portability)

## Context

After SPEC-011a establishes the module infrastructure and core services, this sub-specification extracts cluster and storage module factories. These modules handle data distribution, replication, and persistence.

### Current State (Lines to Extract)

```
Lines 124-159: Cluster setup (ClusterManager, PartitionService, LockManager)
Lines 141-170: Storage setup (StorageManager, QueryRegistry)
Lines 282-342: Replication, MerkleTreeManager, PartitionReassigner
```

## Task

Extract cluster and storage creation into dedicated module factories:
1. Create `modules/cluster-module.ts` with ClusterManager, PartitionService, replication
2. Create `modules/storage-module.ts` with StorageManager, QueryRegistry, pools
3. Update type definitions in `modules/types.ts`
4. Update ServerFactory to use the new factories

## Requirements

### R1: Cluster Module Types (update `modules/types.ts`)

```typescript
export interface ClusterModuleConfig {
  nodeId: string;
  host?: string;
  clusterPort?: number;
  peers?: string[];
  resolvePeers?: () => string[];
  discovery?: 'manual' | 'kubernetes';
  serviceName?: string;
  discoveryInterval?: number;
  clusterTls?: ClusterTLSConfig;
  replicationEnabled?: boolean;
  defaultConsistency?: ConsistencyLevel;
  replicationConfig?: Partial<ReplicationConfig>;
}

export interface ClusterModuleDeps {
  hlc: HLC;
}

export interface ClusterModule {
  cluster: ClusterManager;
  partitionService: PartitionService;
  replicationPipeline?: ReplicationPipeline;
  lockManager: LockManager;
  merkleTreeManager?: MerkleTreeManager;
  partitionReassigner?: PartitionReassigner;
  readReplicaHandler?: ReadReplicaHandler;
  repairScheduler?: RepairScheduler;
}
```

### R2: Storage Module Types (update `modules/types.ts`)

```typescript
export interface StorageModuleConfig {
  nodeId: string;
  storage?: IServerStorage;
  fullTextSearch?: Record<string, FullTextIndexConfig>;
  writeAckTimeout?: number;
}

export interface StorageModuleDeps {
  hlc: HLC;
  metricsService: MetricsService;
  partitionService: PartitionService;
}

export interface StorageModule {
  storageManager: StorageManager;
  queryRegistry: QueryRegistry;
  eventPayloadPool: ObjectPool<PooledEventPayload>;
  taskletScheduler: TaskletScheduler;
  writeAckManager: WriteAckManager;
}
```

### R3: Cluster Module Factory (`modules/cluster-module.ts`)

```typescript
export function createClusterModule(
  config: ClusterModuleConfig,
  deps: ClusterModuleDeps
): ClusterModule {
  const peers = config.resolvePeers ? config.resolvePeers() : (config.peers || []);

  const cluster = new ClusterManager({
    nodeId: config.nodeId,
    host: config.host || 'localhost',
    port: config.clusterPort ?? 0,
    peers,
    discovery: config.discovery,
    serviceName: config.serviceName,
    discoveryInterval: config.discoveryInterval,
    tls: config.clusterTls
  });

  const partitionService = new PartitionService(cluster);
  const lockManager = new LockManager();

  let replicationPipeline: ReplicationPipeline | undefined;
  if (config.replicationEnabled !== false) {
    replicationPipeline = new ReplicationPipeline(
      cluster,
      partitionService,
      {
        ...DEFAULT_REPLICATION_CONFIG,
        defaultConsistency: config.defaultConsistency ?? ConsistencyLevel.EVENTUAL,
        ...config.replicationConfig,
      }
    );
  }

  const merkleTreeManager = new MerkleTreeManager(config.nodeId);
  const partitionReassigner = new PartitionReassigner(cluster, partitionService, {
    reassignmentDelayMs: 1000
  });
  // ... readReplicaHandler, repairScheduler

  return {
    cluster,
    partitionService,
    replicationPipeline,
    lockManager,
    merkleTreeManager,
    partitionReassigner,
    // ...
  };
}
```

### R4: Storage Module Factory (`modules/storage-module.ts`)

```typescript
export function createStorageModule(
  config: StorageModuleConfig,
  deps: StorageModuleDeps
): StorageModule {
  // QueryRegistry must be created first (used in StorageManager callback)
  const queryRegistry = new QueryRegistry();

  const storageManager = new StorageManager({
    nodeId: config.nodeId,
    hlc: deps.hlc,
    storage: config.storage,
    fullTextSearch: config.fullTextSearch,
    isRelatedKey: (key: string) => deps.partitionService.isRelated(key) ?? true,
    onMapLoaded: (mapName: string, _recordCount: number) => {
      const map = storageManager.getMaps().get(mapName);
      if (map) {
        queryRegistry.refreshSubscriptions(mapName, map);
        const mapSize = (map as any).totalRecords ?? map.size;
        deps.metricsService.setMapSize(mapName, mapSize);
      }
    },
  });

  const eventPayloadPool = createEventPayloadPool({ maxSize: 4096, initialSize: 128 });
  const taskletScheduler = new TaskletScheduler({
    defaultTimeBudgetMs: 5,
    maxConcurrent: 20,
  });
  const writeAckManager = new WriteAckManager({
    defaultTimeout: config.writeAckTimeout ?? 5000,
  });

  return {
    storageManager,
    queryRegistry,
    eventPayloadPool,
    taskletScheduler,
    writeAckManager,
  };
}
```

### R5: Update Module Index

Add exports to `modules/index.ts`:

```typescript
export * from './cluster-module';
export * from './storage-module';
```

### R6: Update ServerFactory

```typescript
import { createClusterModule, createStorageModule } from './modules';

const cluster = createClusterModule(
  {
    nodeId: config.nodeId,
    host: config.host,
    clusterPort: config.clusterPort,
    peers: config.peers,
    // ... map config fields
  },
  { hlc: core.hlc }
);

const storage = createStorageModule(
  {
    nodeId: config.nodeId,
    storage: config.storage,
    fullTextSearch: config.fullTextSearch,
  },
  {
    hlc: core.hlc,
    metricsService: core.metricsService,
    partitionService: cluster.partitionService,
  }
);
```

## Files

### Files to Create

| File | Purpose |
|------|---------|
| `packages/server/src/modules/cluster-module.ts` | ClusterManager, PartitionService, Replication |
| `packages/server/src/modules/storage-module.ts` | StorageManager, QueryRegistry, pools |

### Files to Modify

| File | Change |
|------|--------|
| `packages/server/src/modules/types.ts` | Add ClusterModule, StorageModule interfaces |
| `packages/server/src/modules/index.ts` | Export new modules |
| `packages/server/src/ServerFactory.ts` | Use createClusterModule, createStorageModule |

## Acceptance Criteria

1. [ ] `modules/types.ts` exports ClusterModule, ClusterModuleConfig, ClusterModuleDeps interfaces
2. [ ] `modules/types.ts` exports StorageModule, StorageModuleConfig, StorageModuleDeps interfaces
3. [ ] `modules/cluster-module.ts` exports `createClusterModule(config, deps)` function
4. [ ] `createClusterModule()` returns { cluster, partitionService, lockManager, ... }
5. [ ] `createClusterModule()` conditionally creates replicationPipeline based on config
6. [ ] `modules/storage-module.ts` exports `createStorageModule(config, deps)` function
7. [ ] `createStorageModule()` returns { storageManager, queryRegistry, eventPayloadPool, taskletScheduler, writeAckManager }
8. [ ] StorageManager's onMapLoaded callback correctly references queryRegistry
9. [ ] ServerFactory.create() uses `createClusterModule()` instead of inline creation
10. [ ] ServerFactory.create() uses `createStorageModule()` instead of inline creation
11. [ ] All 203+ existing tests pass
12. [ ] Build passes (`pnpm build`)
13. [ ] No circular dependencies
14. [ ] TypeScript strict mode passes

## Constraints

- **No Breaking Changes**: ServerFactory behavior must be identical
- **Dependency Order**: Storage module depends on cluster.partitionService
- **Callback Closure**: StorageManager's onMapLoaded must capture queryRegistry correctly

## Assumptions

1. ReplicationPipeline is optional (controlled by replicationEnabled config)
2. MerkleTreeManager, PartitionReassigner, ReadReplicaHandler, RepairScheduler are created unconditionally
3. QueryRegistry must be created before StorageManager

---
*Created by SpecFlow split from SPEC-011 on 2026-01-30*
