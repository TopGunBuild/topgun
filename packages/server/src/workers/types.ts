/**
 * WorkerPool Types
 * Phase 1.02: Worker Threads Implementation
 */

/**
 * Configuration for WorkerPool
 */
export interface WorkerPoolConfig {
  /** Minimum number of workers (default: 2) */
  minWorkers?: number;
  /** Maximum number of workers (default: os.cpus().length - 1) */
  maxWorkers?: number;
  /** Task execution timeout in ms (default: 5000) */
  taskTimeout?: number;
  /** Worker idle timeout before termination in ms (default: 30000) */
  idleTimeout?: number;
  /** Enable worker restart on crash (default: true) */
  autoRestart?: boolean;
  /** Custom worker script path (for testing or custom workers) */
  workerScript?: string;
}

/**
 * Task types supported by workers
 */
export type WorkerTaskType =
  | 'merkle-hash'
  | 'merkle-diff'
  | 'crdt-merge'
  | 'serialize'
  | 'deserialize';

/**
 * Task priority levels
 */
export type TaskPriority = 'high' | 'normal' | 'low';

/**
 * Task to be executed by a worker
 */
export interface WorkerTask<TPayload = unknown, TResult = unknown> {
  /** Unique task ID */
  id: string;
  /** Task type for routing to correct handler */
  type: WorkerTaskType;
  /** Task payload (must be serializable) */
  payload: TPayload;
  /** Priority (default: 'normal') */
  priority?: TaskPriority;
  /** Internal: Expected result type marker */
  _resultType?: TResult;
}

/**
 * Message sent from main thread to worker
 */
export interface WorkerMessage<TPayload = unknown> {
  id: string;
  type: WorkerTaskType;
  payload: TPayload;
}

/**
 * Response from worker to main thread
 */
export interface WorkerResponse<TResult = unknown> {
  id: string;
  success: boolean;
  result?: TResult;
  error?: string;
}

/**
 * Internal worker state tracking
 */
export interface WorkerState {
  /** Worker instance */
  worker: import('worker_threads').Worker;
  /** Is worker currently executing a task */
  busy: boolean;
  /** Current task ID (if busy) */
  currentTaskId?: string;
  /** Timestamp when worker became idle */
  idleSince?: number;
  /** Number of tasks completed by this worker */
  tasksCompleted: number;
}

/**
 * Internal task tracking with resolve/reject
 */
export interface PendingTask<TResult = unknown> {
  task: WorkerTask<unknown, TResult>;
  resolve: (result: TResult) => void;
  reject: (error: Error) => void;
  submittedAt: number;
  timeoutId?: NodeJS.Timeout;
}

/**
 * Pool statistics
 */
export interface WorkerPoolStats {
  /** Number of workers currently executing tasks */
  activeWorkers: number;
  /** Number of workers waiting for tasks */
  idleWorkers: number;
  /** Number of tasks in queue */
  pendingTasks: number;
  /** Total tasks completed successfully */
  completedTasks: number;
  /** Total tasks that failed */
  failedTasks: number;
  /** Average task execution time in ms */
  avgTaskDuration: number;
}
