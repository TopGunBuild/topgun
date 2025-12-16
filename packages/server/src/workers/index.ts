/**
 * WorkerPool Module Exports
 * Phase 1.02-03: Worker Threads Implementation
 */

// Main classes
export { WorkerPool } from './WorkerPool';
export { MerkleWorker } from './MerkleWorker';

// Types
export type {
  WorkerPoolConfig,
  WorkerTask,
  WorkerTaskType,
  TaskPriority,
  WorkerPoolStats,
  WorkerMessage,
  WorkerResponse,
} from './types';

// Merkle Types
export type {
  MerkleHashPayload,
  MerkleHashResult,
  MerkleHashEntry,
  ORMapMerkleHashPayload,
  ORMapMerkleHashResult,
  ORMapMerkleHashEntry,
  MerkleDiffPayload,
  MerkleDiffResult,
  MerkleRebuildPayload,
  MerkleRebuildResult,
  ORMapMerkleRebuildPayload,
  BucketInfo,
} from './merkle-types';

// Errors
export {
  WorkerError,
  WorkerTimeoutError,
  WorkerTaskError,
  WorkerPoolShutdownError,
  WorkerCrashError,
} from './errors';
