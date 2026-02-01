/**
 * WorkerPool Implementation
 * Worker Threads Implementation
 *
 * Manages a pool of worker threads for CPU-bound operations.
 * Features:
 * - Auto-scaling (minWorkers to maxWorkers)
 * - Priority queue (high > normal > low)
 * - Task timeouts
 * - Worker crash recovery
 * - Graceful shutdown
 */

import { Worker } from 'worker_threads';
import { cpus } from 'os';
import { join } from 'path';
import {
  WorkerPoolConfig,
  WorkerTask,
  WorkerPoolStats,
  WorkerState,
  PendingTask,
  WorkerMessage,
  WorkerResponse,
  TaskPriority,
} from './types';
import {
  WorkerTimeoutError,
  WorkerTaskError,
  WorkerPoolShutdownError,
  WorkerCrashError,
} from './errors';

// Priority order for queue sorting
const PRIORITY_ORDER: Record<TaskPriority, number> = {
  high: 0,
  normal: 1,
  low: 2,
};

export class WorkerPool {
  private readonly config: Required<Omit<WorkerPoolConfig, 'workerScript'>> & { workerScript: string };
  private readonly workers: Map<number, WorkerState> = new Map();
  private readonly taskQueue: PendingTask[] = [];
  private readonly pendingTasks: Map<string, PendingTask> = new Map();

  private workerIdCounter = 0;
  private isShuttingDown = false;
  private isShutdown = false;

  // Statistics
  private completedTaskCount = 0;
  private failedTaskCount = 0;
  private totalTaskDuration = 0;

  // Idle check interval
  private idleCheckInterval?: NodeJS.Timeout;

  constructor(config?: WorkerPoolConfig) {
    const cpuCount = cpus().length;

    // Determine worker script path
    // In production: compiled .js file
    // In tests: .ts file needs ts-node registration
    const defaultWorkerScript = this.resolveWorkerScript();

    this.config = {
      minWorkers: config?.minWorkers ?? 2,
      maxWorkers: config?.maxWorkers ?? Math.max(1, cpuCount - 1),
      taskTimeout: config?.taskTimeout ?? 5000,
      idleTimeout: config?.idleTimeout ?? 30000,
      autoRestart: config?.autoRestart ?? true,
      workerScript: config?.workerScript ?? defaultWorkerScript,
    };

    // Validate config
    if (this.config.minWorkers < 1) {
      this.config.minWorkers = 1;
    }
    if (this.config.maxWorkers < this.config.minWorkers) {
      this.config.maxWorkers = this.config.minWorkers;
    }

    // Initialize minimum workers
    this.initializeWorkers();

    // Start idle check interval
    this.startIdleCheck();
  }

  /**
   * Resolve the worker script path based on environment
   */
  private resolveWorkerScript(): string {
    // When running via ts-jest, __dirname is src/workers
    // When running compiled, __dirname is dist/workers
    // We need to check both locations for the compiled .js file

    // Path 1: Direct location (compiled environment - __dirname is dist/workers)
    const directJsPath = join(__dirname, 'worker-scripts', 'base.worker.js');

    // Path 2: dist directory from package root (ts-jest - __dirname is src/workers)
    // Go up from src/workers to package root, then into dist/workers/worker-scripts
    const distJsPath = join(__dirname, '..', '..', 'dist', 'workers', 'worker-scripts', 'base.worker.js');

    // Path 3: Fallback to .ts for development without build
    const tsPath = join(__dirname, 'worker-scripts', 'base.worker.ts');

    // Try paths in order: direct .js, dist .js, then .ts fallback
    try {
      require.resolve(directJsPath);
      return directJsPath;
    } catch {
      // Direct .js not found, try dist path
    }

    try {
      require.resolve(distJsPath);
      return distJsPath;
    } catch {
      // dist .js not found, fall back to .ts (will fail in worker_threads but works inline)
      return tsPath;
    }
  }

  /**
   * Submit a task to the pool
   */
  public submit<TPayload, TResult>(
    task: WorkerTask<TPayload, TResult>
  ): Promise<TResult> {
    return new Promise((resolve, reject) => {
      if (this.isShuttingDown || this.isShutdown) {
        reject(new WorkerPoolShutdownError());
        return;
      }

      const pendingTask: PendingTask<TResult> = {
        task: task as WorkerTask<unknown, TResult>,
        resolve,
        reject,
        submittedAt: Date.now(),
      };

      // Set up timeout
      if (this.config.taskTimeout > 0) {
        pendingTask.timeoutId = setTimeout(() => {
          this.handleTaskTimeout(task.id);
        }, this.config.taskTimeout);
      }

      // Add to pending map
      this.pendingTasks.set(task.id, pendingTask as PendingTask);

      // Try to assign to idle worker
      const idleWorker = this.findIdleWorker();
      if (idleWorker) {
        this.assignTaskToWorker(idleWorker, pendingTask as PendingTask);
      } else {
        // Queue the task with priority
        this.enqueueTask(pendingTask as PendingTask);

        // Try to scale up if possible
        this.tryScaleUp();
      }
    });
  }

  /**
   * Get current pool statistics
   */
  public getStats(): WorkerPoolStats {
    let activeWorkers = 0;
    let idleWorkers = 0;

    for (const state of this.workers.values()) {
      if (state.busy) {
        activeWorkers++;
      } else {
        idleWorkers++;
      }
    }

    return {
      activeWorkers,
      idleWorkers,
      pendingTasks: this.taskQueue.length,
      completedTasks: this.completedTaskCount,
      failedTasks: this.failedTaskCount,
      avgTaskDuration:
        this.completedTaskCount > 0
          ? this.totalTaskDuration / this.completedTaskCount
          : 0,
    };
  }

  /**
   * Gracefully shutdown all workers
   */
  public async shutdown(timeout = 10000): Promise<void> {
    if (this.isShutdown) {
      return;
    }

    this.isShuttingDown = true;

    // Stop idle check
    if (this.idleCheckInterval) {
      clearInterval(this.idleCheckInterval);
      this.idleCheckInterval = undefined;
    }

    // Reject all queued tasks
    for (const pendingTask of this.taskQueue) {
      if (pendingTask.timeoutId) {
        clearTimeout(pendingTask.timeoutId);
      }
      pendingTask.reject(new WorkerPoolShutdownError());
    }
    this.taskQueue.length = 0;

    // Wait for active tasks to complete (with timeout)
    const startTime = Date.now();
    while (this.pendingTasks.size > 0 && Date.now() - startTime < timeout) {
      await new Promise((resolve) => setTimeout(resolve, 100));
    }

    // Force reject remaining pending tasks
    for (const [taskId, pendingTask] of this.pendingTasks) {
      if (pendingTask.timeoutId) {
        clearTimeout(pendingTask.timeoutId);
      }
      pendingTask.reject(new WorkerPoolShutdownError());
      this.pendingTasks.delete(taskId);
    }

    // Terminate all workers
    const terminatePromises: Promise<number>[] = [];
    for (const [workerId, state] of this.workers) {
      terminatePromises.push(
        state.worker.terminate().then(() => workerId)
      );
    }

    await Promise.all(terminatePromises);
    this.workers.clear();

    this.isShutdown = true;
    this.isShuttingDown = false;
  }

  /**
   * Check if pool is accepting tasks
   */
  public isRunning(): boolean {
    return !this.isShuttingDown && !this.isShutdown;
  }

  // ============ Private Methods ============

  private initializeWorkers(): void {
    for (let i = 0; i < this.config.minWorkers; i++) {
      this.createWorker();
    }
  }

  private createWorker(): WorkerState | null {
    if (this.isShuttingDown || this.isShutdown) {
      return null;
    }

    try {
      const workerId = ++this.workerIdCounter;

      // For TypeScript files in test environment, use ts-node
      const workerOptions = this.config.workerScript.endsWith('.ts')
        ? { execArgv: ['--require', 'ts-node/register'] }
        : {};

      const worker = new Worker(this.config.workerScript, workerOptions);

      const state: WorkerState = {
        worker,
        busy: false,
        idleSince: Date.now(),
        tasksCompleted: 0,
      };

      // Handle messages from worker
      worker.on('message', (response: WorkerResponse) => {
        this.handleWorkerResponse(workerId, response);
      });

      // Handle worker errors
      worker.on('error', (error) => {
        console.error(`Worker ${workerId} error:`, error);
        this.handleWorkerError(workerId, error);
      });

      // Handle worker exit
      worker.on('exit', (code) => {
        this.handleWorkerExit(workerId, code);
      });

      this.workers.set(workerId, state);
      return state;
    } catch (error) {
      console.error('Failed to create worker:', error);
      return null;
    }
  }

  private findIdleWorker(): WorkerState | undefined {
    for (const state of this.workers.values()) {
      if (!state.busy) {
        return state;
      }
    }
    return undefined;
  }

  private assignTaskToWorker(
    workerState: WorkerState,
    pendingTask: PendingTask
  ): void {
    workerState.busy = true;
    workerState.currentTaskId = pendingTask.task.id;
    workerState.idleSince = undefined;

    const message: WorkerMessage = {
      id: pendingTask.task.id,
      type: pendingTask.task.type,
      payload: pendingTask.task.payload,
    };

    workerState.worker.postMessage(message);
  }

  private enqueueTask(pendingTask: PendingTask): void {
    const priority = pendingTask.task.priority ?? 'normal';
    const priorityOrder = PRIORITY_ORDER[priority];

    // Find insertion point (maintain priority order, FIFO within same priority)
    let insertIndex = this.taskQueue.length;
    for (let i = 0; i < this.taskQueue.length; i++) {
      const queuedPriority = this.taskQueue[i].task.priority ?? 'normal';
      if (PRIORITY_ORDER[queuedPriority] > priorityOrder) {
        insertIndex = i;
        break;
      }
    }

    this.taskQueue.splice(insertIndex, 0, pendingTask);
  }

  private tryScaleUp(): void {
    if (this.workers.size < this.config.maxWorkers) {
      const newWorker = this.createWorker();
      if (newWorker && this.taskQueue.length > 0) {
        const nextTask = this.taskQueue.shift()!;
        this.assignTaskToWorker(newWorker, nextTask);
      }
    }
  }

  private handleWorkerResponse(
    workerId: number,
    response: WorkerResponse
  ): void {
    const pendingTask = this.pendingTasks.get(response.id);
    if (!pendingTask) {
      // Task already timed out or cancelled
      return;
    }

    // Clear timeout
    if (pendingTask.timeoutId) {
      clearTimeout(pendingTask.timeoutId);
    }

    // Remove from pending
    this.pendingTasks.delete(response.id);

    // Update stats
    const duration = Date.now() - pendingTask.submittedAt;
    this.totalTaskDuration += duration;

    // Resolve or reject
    if (response.success) {
      this.completedTaskCount++;
      pendingTask.resolve(response.result);
    } else {
      this.failedTaskCount++;
      pendingTask.reject(
        new WorkerTaskError(response.id, response.error ?? 'Unknown error')
      );
    }

    // Mark worker as idle and process next task
    const workerState = this.workers.get(workerId);
    if (workerState) {
      workerState.busy = false;
      workerState.currentTaskId = undefined;
      workerState.idleSince = Date.now();
      workerState.tasksCompleted++;

      // Process next task from queue
      if (this.taskQueue.length > 0) {
        const nextTask = this.taskQueue.shift()!;
        this.assignTaskToWorker(workerState, nextTask);
      }
    }
  }

  private handleTaskTimeout(taskId: string): void {
    const pendingTask = this.pendingTasks.get(taskId);
    if (!pendingTask) {
      return;
    }

    // Remove from pending
    this.pendingTasks.delete(taskId);
    this.failedTaskCount++;

    // Reject with timeout error
    pendingTask.reject(
      new WorkerTimeoutError(taskId, this.config.taskTimeout)
    );

    // Note: The worker will continue processing and send a response,
    // but it will be ignored since we removed from pendingTasks.
    // The worker itself is not killed - it will become available for
    // next task when it finishes.
  }

  private handleWorkerError(workerId: number, error: Error): void {
    const workerState = this.workers.get(workerId);
    if (!workerState) {
      return;
    }

    // If worker was processing a task, reject it
    if (workerState.currentTaskId) {
      const pendingTask = this.pendingTasks.get(workerState.currentTaskId);
      if (pendingTask) {
        if (pendingTask.timeoutId) {
          clearTimeout(pendingTask.timeoutId);
        }
        this.pendingTasks.delete(workerState.currentTaskId);
        this.failedTaskCount++;
        pendingTask.reject(
          new WorkerTaskError(workerState.currentTaskId, error.message)
        );
      }
    }
  }

  private handleWorkerExit(workerId: number, exitCode: number | null): void {
    const workerState = this.workers.get(workerId);
    this.workers.delete(workerId);

    if (!workerState) {
      return;
    }

    // If worker was processing a task, reject it
    if (workerState.currentTaskId) {
      const pendingTask = this.pendingTasks.get(workerState.currentTaskId);
      if (pendingTask) {
        if (pendingTask.timeoutId) {
          clearTimeout(pendingTask.timeoutId);
        }
        this.pendingTasks.delete(workerState.currentTaskId);
        this.failedTaskCount++;
        pendingTask.reject(new WorkerCrashError(workerId, exitCode));
      }
    }

    // Auto-restart if needed
    if (
      this.config.autoRestart &&
      !this.isShuttingDown &&
      !this.isShutdown &&
      this.workers.size < this.config.minWorkers
    ) {
      const newWorker = this.createWorker();
      if (newWorker && this.taskQueue.length > 0) {
        const nextTask = this.taskQueue.shift()!;
        this.assignTaskToWorker(newWorker, nextTask);
      }
    }
  }

  private startIdleCheck(): void {
    this.idleCheckInterval = setInterval(() => {
      this.checkIdleWorkers();
    }, 5000); // Check every 5 seconds

    // Don't prevent process from exiting
    this.idleCheckInterval.unref();
  }

  private checkIdleWorkers(): void {
    if (this.isShuttingDown || this.isShutdown) {
      return;
    }

    const now = Date.now();
    const workersToTerminate: number[] = [];

    for (const [workerId, state] of this.workers) {
      // Don't terminate if we're at minimum
      if (this.workers.size <= this.config.minWorkers) {
        break;
      }

      // Check if worker is idle and past timeout
      if (
        !state.busy &&
        state.idleSince &&
        now - state.idleSince > this.config.idleTimeout
      ) {
        workersToTerminate.push(workerId);

        // Don't terminate below minimum
        if (
          this.workers.size - workersToTerminate.length <=
          this.config.minWorkers
        ) {
          break;
        }
      }
    }

    // Terminate idle workers
    for (const workerId of workersToTerminate) {
      const state = this.workers.get(workerId);
      if (state) {
        this.workers.delete(workerId);
        state.worker.terminate().catch(() => {
          // Ignore termination errors
        });
      }
    }
  }
}
