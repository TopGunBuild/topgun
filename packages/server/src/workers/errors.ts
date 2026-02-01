/**
 * WorkerPool Custom Errors
 * Worker Threads Implementation
 */

/**
 * Base error class for worker-related errors
 */
export class WorkerError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'WorkerError';
    // Maintain proper stack trace in V8
    Error.captureStackTrace?.(this, this.constructor);
  }
}

/**
 * Thrown when a task exceeds its timeout
 */
export class WorkerTimeoutError extends WorkerError {
  public readonly taskId: string;
  public readonly timeout: number;

  constructor(taskId: string, timeout: number) {
    super(`Task ${taskId} timed out after ${timeout}ms`);
    this.name = 'WorkerTimeoutError';
    this.taskId = taskId;
    this.timeout = timeout;
  }
}

/**
 * Thrown when a task fails during execution
 */
export class WorkerTaskError extends WorkerError {
  public readonly taskId: string;
  public readonly originalError: string;

  constructor(taskId: string, originalError: string) {
    super(`Task ${taskId} failed: ${originalError}`);
    this.name = 'WorkerTaskError';
    this.taskId = taskId;
    this.originalError = originalError;
  }
}

/**
 * Thrown when trying to submit a task to a shutdown pool
 */
export class WorkerPoolShutdownError extends WorkerError {
  constructor() {
    super('Cannot submit task: WorkerPool is shutting down or already shut down');
    this.name = 'WorkerPoolShutdownError';
  }
}

/**
 * Thrown when a worker crashes unexpectedly
 */
export class WorkerCrashError extends WorkerError {
  public readonly workerId: number;
  public readonly exitCode: number | null;

  constructor(workerId: number, exitCode: number | null) {
    super(`Worker ${workerId} crashed with exit code ${exitCode}`);
    this.name = 'WorkerCrashError';
    this.workerId = workerId;
    this.exitCode = exitCode;
  }
}
