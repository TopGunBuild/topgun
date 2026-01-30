import { WorkerPool, MerkleWorker, CRDTMergeWorker, SerializationWorker } from '../workers';
import type { WorkerModule, WorkerModuleConfig } from './types';

export function createWorkersModule(config: WorkerModuleConfig): WorkerModule {
  if (!config.workerPoolEnabled) {
    return {};
  }

  const workerPool = new WorkerPool({
    minWorkers: config.workerPoolConfig?.minWorkers ?? 2,
    maxWorkers: config.workerPoolConfig?.maxWorkers,
    taskTimeout: config.workerPoolConfig?.taskTimeout ?? 5000,
    idleTimeout: config.workerPoolConfig?.idleTimeout ?? 30000,
    autoRestart: config.workerPoolConfig?.autoRestart ?? true,
  });
  const merkleWorker = new MerkleWorker(workerPool);
  const crdtMergeWorker = new CRDTMergeWorker(workerPool);
  const serializationWorker = new SerializationWorker(workerPool);

  return { workerPool, merkleWorker, crdtMergeWorker, serializationWorker };
}
