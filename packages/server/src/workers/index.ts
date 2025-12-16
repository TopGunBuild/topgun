/**
 * WorkerPool Module Exports
 * Phase 1.02: Worker Threads Implementation
 */

// Main class
export { WorkerPool } from './WorkerPool';

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

// Errors
export {
  WorkerError,
  WorkerTimeoutError,
  WorkerTaskError,
  WorkerPoolShutdownError,
  WorkerCrashError,
} from './errors';
