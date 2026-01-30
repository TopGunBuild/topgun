import { StorageManager } from '../coordinator/storage-manager';
import { QueryRegistry } from '../query/QueryRegistry';
import { createEventPayloadPool } from '../memory';
import { TaskletScheduler } from '../tasklet';
import { WriteAckManager } from '../ack/WriteAckManager';
import type { StorageModule, StorageModuleConfig, StorageModuleDeps } from './types';

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
