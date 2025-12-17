/**
 * TaskletScheduler — Cooperative multitasking for long-running operations.
 *
 * Inspired by Hazelcast's Tasklet pattern, this scheduler allows long operations
 * to yield control back to the event loop periodically, preventing event loop
 * blocking and maintaining responsiveness.
 *
 * Key concepts:
 * - Tasklet: A unit of work that can be paused and resumed
 * - Time budget: Maximum time a tasklet can run before yielding
 * - Cooperative scheduling: Tasklets voluntarily yield when time budget is exhausted
 */

/**
 * Progress state returned by a tasklet after each execution step.
 */
export type ProgressState =
    | 'DONE'           // Tasklet completed all work
    | 'MADE_PROGRESS'  // Tasklet made progress but has more work
    | 'NO_PROGRESS';   // Tasklet couldn't make progress (e.g., waiting for I/O)

/**
 * Interface for a tasklet that can be scheduled.
 */
export interface Tasklet<T = void> {
    /** Unique name for logging/metrics */
    readonly name: string;

    /**
     * Execute one chunk of work.
     * Should check time budget and return appropriate state.
     */
    call(): ProgressState;

    /**
     * Get the result after tasklet is DONE.
     * Only valid when call() returns 'DONE'.
     */
    getResult(): T;

    /**
     * Called when tasklet is cancelled before completion.
     */
    onCancel?(): void;
}

/**
 * Configuration for TaskletScheduler.
 */
export interface TaskletSchedulerConfig {
    /** Default time budget per tasklet execution in ms (default: 5) */
    defaultTimeBudgetMs?: number;

    /** Maximum concurrent tasklets (default: 10) */
    maxConcurrent?: number;

    /** Interval between scheduler ticks in ms (default: 1) */
    tickIntervalMs?: number;

    /** Enable metrics collection (default: true) */
    metricsEnabled?: boolean;
}

/**
 * Metrics for monitoring scheduler performance.
 */
export interface TaskletSchedulerStats {
    /** Total tasklets scheduled */
    totalScheduled: number;

    /** Tasklets currently running */
    activeTasklets: number;

    /** Tasklets completed successfully */
    completedTasklets: number;

    /** Tasklets cancelled */
    cancelledTasklets: number;

    /** Total iterations across all tasklets */
    totalIterations: number;

    /** Average iterations per tasklet */
    avgIterationsPerTasklet: number;

    /** Tasklets that completed in a single iteration */
    singleIterationCompletions: number;

    /** Time spent in tasklet execution (ms) */
    totalExecutionTimeMs: number;
}

/**
 * Internal state for a running tasklet.
 */
interface TaskletState<T> {
    tasklet: Tasklet<T>;
    resolve: (result: T) => void;
    reject: (error: Error) => void;
    iterations: number;
    startTime: number;
    lastProgressTime: number;
}

const DEFAULT_CONFIG: Required<TaskletSchedulerConfig> = {
    defaultTimeBudgetMs: 5,
    maxConcurrent: 10,
    tickIntervalMs: 1,
    metricsEnabled: true,
};

/**
 * TaskletScheduler manages cooperative multitasking for long-running operations.
 *
 * Usage:
 * ```typescript
 * const scheduler = new TaskletScheduler();
 *
 * // Schedule a tasklet
 * const result = await scheduler.schedule(new QueryExecutionTasklet(records, query));
 *
 * // Cancel all running tasklets
 * scheduler.cancelAll();
 *
 * // Shutdown scheduler
 * scheduler.shutdown();
 * ```
 */
export class TaskletScheduler {
    private readonly config: Required<TaskletSchedulerConfig>;
    private readonly activeTasklets: Map<string, TaskletState<any>> = new Map();
    private tickTimer: NodeJS.Immediate | null = null;
    private isRunning = false;
    private isShuttingDown = false;

    // Metrics
    private totalScheduled = 0;
    private completedTasklets = 0;
    private cancelledTasklets = 0;
    private totalIterations = 0;
    private singleIterationCompletions = 0;
    private totalExecutionTimeMs = 0;

    constructor(config?: TaskletSchedulerConfig) {
        this.config = { ...DEFAULT_CONFIG, ...config };
    }

    /**
     * Schedule a tasklet for execution.
     * Returns a promise that resolves when the tasklet completes.
     */
    schedule<T>(tasklet: Tasklet<T>): Promise<T> {
        if (this.isShuttingDown) {
            return Promise.reject(new Error('Scheduler is shutting down'));
        }

        return new Promise<T>((resolve, reject) => {
            const taskletId = `${tasklet.name}-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;

            // Check concurrent limit
            if (this.activeTasklets.size >= this.config.maxConcurrent) {
                reject(new Error(`Max concurrent tasklets (${this.config.maxConcurrent}) reached`));
                return;
            }

            const state: TaskletState<T> = {
                tasklet,
                resolve,
                reject,
                iterations: 0,
                startTime: Date.now(),
                lastProgressTime: Date.now(),
            };

            this.activeTasklets.set(taskletId, state);
            this.totalScheduled++;

            // Start scheduler if not running
            if (!this.isRunning) {
                this.startScheduler();
            }
        });
    }

    /**
     * Run a tasklet synchronously (blocking).
     * Useful for small operations or when cooperative scheduling isn't needed.
     */
    runSync<T>(tasklet: Tasklet<T>): T {
        let state: ProgressState;
        let iterations = 0;
        const startTime = Date.now();

        do {
            state = tasklet.call();
            iterations++;
        } while (state === 'MADE_PROGRESS');

        if (state === 'NO_PROGRESS') {
            throw new Error(`Tasklet ${tasklet.name} made no progress`);
        }

        if (this.config.metricsEnabled) {
            this.totalIterations += iterations;
            this.totalExecutionTimeMs += Date.now() - startTime;
            if (iterations === 1) {
                this.singleIterationCompletions++;
            }
        }

        return tasklet.getResult();
    }

    /**
     * Cancel a specific tasklet by name pattern.
     * Returns the number of tasklets cancelled.
     */
    cancel(namePattern: string | RegExp): number {
        let cancelled = 0;
        const pattern = typeof namePattern === 'string'
            ? new RegExp(`^${namePattern}`)
            : namePattern;

        for (const [id, state] of this.activeTasklets) {
            if (pattern.test(state.tasklet.name)) {
                this.cancelTasklet(id, state);
                cancelled++;
            }
        }

        return cancelled;
    }

    /**
     * Cancel all running tasklets.
     */
    cancelAll(): number {
        let cancelled = 0;

        for (const [id, state] of this.activeTasklets) {
            this.cancelTasklet(id, state);
            cancelled++;
        }

        return cancelled;
    }

    /**
     * Get scheduler statistics.
     */
    getStats(): TaskletSchedulerStats {
        return {
            totalScheduled: this.totalScheduled,
            activeTasklets: this.activeTasklets.size,
            completedTasklets: this.completedTasklets,
            cancelledTasklets: this.cancelledTasklets,
            totalIterations: this.totalIterations,
            avgIterationsPerTasklet: this.completedTasklets > 0
                ? this.totalIterations / this.completedTasklets
                : 0,
            singleIterationCompletions: this.singleIterationCompletions,
            totalExecutionTimeMs: this.totalExecutionTimeMs,
        };
    }

    /**
     * Reset statistics.
     */
    resetStats(): void {
        this.totalScheduled = 0;
        this.completedTasklets = 0;
        this.cancelledTasklets = 0;
        this.totalIterations = 0;
        this.singleIterationCompletions = 0;
        this.totalExecutionTimeMs = 0;
    }

    /**
     * Shutdown the scheduler.
     * Cancels all running tasklets and stops the tick timer.
     */
    shutdown(): void {
        this.isShuttingDown = true;
        this.cancelAll();
        this.stopScheduler();
    }

    /**
     * Check if scheduler is running.
     */
    get running(): boolean {
        return this.isRunning;
    }

    /**
     * Get number of active tasklets.
     */
    get activeCount(): number {
        return this.activeTasklets.size;
    }

    private startScheduler(): void {
        if (this.isRunning) return;
        this.isRunning = true;
        this.scheduleTick();
    }

    private stopScheduler(): void {
        this.isRunning = false;
        if (this.tickTimer) {
            clearImmediate(this.tickTimer);
            this.tickTimer = null;
        }
    }

    private scheduleTick(): void {
        if (!this.isRunning) return;

        // Use setImmediate for minimal delay while allowing I/O
        this.tickTimer = setImmediate(() => {
            this.tick();
        });
    }

    private tick(): void {
        if (!this.isRunning || this.activeTasklets.size === 0) {
            this.stopScheduler();
            return;
        }

        const tickStart = Date.now();
        const taskletIds = Array.from(this.activeTasklets.keys());

        for (const id of taskletIds) {
            const state = this.activeTasklets.get(id);
            if (!state) continue;

            try {
                const iterationStart = Date.now();
                const result = state.tasklet.call();
                const iterationTime = Date.now() - iterationStart;

                state.iterations++;
                state.lastProgressTime = Date.now();

                if (this.config.metricsEnabled) {
                    this.totalIterations++;
                    this.totalExecutionTimeMs += iterationTime;
                }

                if (result === 'DONE') {
                    this.completeTasklet(id, state);
                } else if (result === 'NO_PROGRESS') {
                    // Tasklet couldn't make progress, will retry next tick
                }
                // MADE_PROGRESS: continue in next tick
            } catch (error) {
                this.failTasklet(id, state, error as Error);
            }

            // Check if we've exceeded our time budget for this tick
            if (Date.now() - tickStart > this.config.defaultTimeBudgetMs * 2) {
                break; // Process remaining tasklets in next tick
            }
        }

        // Schedule next tick if there are still active tasklets
        if (this.activeTasklets.size > 0) {
            this.scheduleTick();
        } else {
            this.stopScheduler();
        }
    }

    private completeTasklet<T>(id: string, state: TaskletState<T>): void {
        this.activeTasklets.delete(id);
        this.completedTasklets++;

        if (state.iterations === 1) {
            this.singleIterationCompletions++;
        }

        try {
            const result = state.tasklet.getResult();
            state.resolve(result);
        } catch (error) {
            state.reject(error as Error);
        }
    }

    private failTasklet<T>(id: string, state: TaskletState<T>, error: Error): void {
        this.activeTasklets.delete(id);
        state.reject(error);
    }

    private cancelTasklet<T>(id: string, state: TaskletState<T>): void {
        this.activeTasklets.delete(id);
        this.cancelledTasklets++;

        if (state.tasklet.onCancel) {
            try {
                state.tasklet.onCancel();
            } catch {
                // Ignore cancel errors
            }
        }

        state.reject(new Error(`Tasklet ${state.tasklet.name} was cancelled`));
    }
}
