/**
 * WorkerPool Module Exports
 * Phase 1.02-07: Worker Threads Implementation
 * Phase 3.04: SharedArrayBuffer Integration
 */

// Main classes
export { WorkerPool } from './WorkerPool';
export { MerkleWorker } from './MerkleWorker';
export { CRDTMergeWorker } from './CRDTMergeWorker';
export { SerializationWorker } from './SerializationWorker';

// Shared Memory (Phase 3.04)
export {
  SharedMemoryManager,
  SlotStatus,
  type SharedMemoryConfig,
  type SharedMemoryStats,
  type SharedSlot,
  type SharedWorkerConfig,
} from './SharedMemoryManager';
export { SharedMemoryWorkerHelper } from './SharedMemoryWorkerHelper';

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

// CRDT Types
export type {
  LWWMergePayload,
  LWWMergeResult,
  LWWMergeRecord,
  LWWExistingRecord,
  ORMapMergePayload,
  ORMapMergeResult,
  ORMapMergeItem,
  ORMapMergeTombstone,
} from './crdt-types';

// Serialization Types
export type {
  SerializeBatchPayload,
  SerializeBatchResult,
  DeserializeBatchPayload,
  DeserializeBatchResult,
  SerializePayload,
  SerializeResult,
  DeserializePayload,
  DeserializeResult,
} from './serialization-types';

// Errors
export {
  WorkerError,
  WorkerTimeoutError,
  WorkerTaskError,
  WorkerPoolShutdownError,
  WorkerCrashError,
} from './errors';
