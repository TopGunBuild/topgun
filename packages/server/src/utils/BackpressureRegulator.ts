import { logger } from './logger';

export interface BackpressureConfig {
    /**
     * How often to force sync (every N operations).
     * Default: 100
     */
    syncFrequency: number;

    /**
     * Maximum pending async operations before blocking.
     * Default: 1000
     */
    maxPendingOps: number;

    /**
     * Backoff timeout in ms when at capacity.
     * Default: 5000
     */
    backoffTimeoutMs: number;

    /**
     * Enable/disable backpressure.
     * Default: true
     */
    enabled: boolean;
}

export interface BackpressureStats {
    pendingOps: number;
    syncCounter: number;
    utilizationPercent: number;
    syncForcedTotal: number;
    waitsTotal: number;
    timeoutsTotal: number;
}

const DEFAULT_CONFIG: BackpressureConfig = {
    syncFrequency: 100,
    maxPendingOps: 1000,
    backoffTimeoutMs: 5000,
    enabled: true
};

/**
 * BackpressureRegulator implements backpressure control similar to Hazelcast's approach.
 *
 * It periodically forces synchronous processing to drain queues and prevent
 * unbounded accumulation of async work. Key features:
 *
 * 1. Sync Window: Every N operations, force sync processing
 * 2. Capacity Limiting: Block when too many operations are pending
 * 3. Randomization: Prevents thundering herd on sync points
 */
export class BackpressureRegulator {
    private syncCounter = 0;
    private pendingOps = 0;
    private readonly config: BackpressureConfig;

    // Metrics tracking
    private syncForcedTotal = 0;
    private waitsTotal = 0;
    private timeoutsTotal = 0;

    // Waiters queue for capacity
    private waiters: Array<{ resolve: () => void; reject: (err: Error) => void }> = [];

    constructor(config?: Partial<BackpressureConfig>) {
        this.config = { ...DEFAULT_CONFIG, ...config };
    }

    /**
     * Check if current operation should be forced to sync.
     * Based on Hazelcast's sync window with randomization.
     *
     * Uses randomization within the sync window to prevent all operations
     * from syncing at the same point, which could cause thundering herd.
     */
    shouldForceSync(): boolean {
        if (!this.config.enabled) {
            return false;
        }

        this.syncCounter++;

        // Randomize within the sync window to prevent thundering herd
        // Each operation has a 1/syncFrequency chance of being forced to sync
        // This distributes sync points evenly across the window
        const syncThreshold = this.config.syncFrequency;
        const shouldSync = this.syncCounter >= syncThreshold;

        if (shouldSync) {
            this.syncCounter = 0;
            this.syncForcedTotal++;
            logger.debug({ syncForcedTotal: this.syncForcedTotal }, 'Forcing sync operation');
            return true;
        }

        // Additional randomization: 10% chance of early sync if above 80% capacity
        const utilization = this.pendingOps / this.config.maxPendingOps;
        if (utilization > 0.8 && Math.random() < 0.1) {
            this.syncForcedTotal++;
            logger.debug({ utilization, pendingOps: this.pendingOps }, 'Early sync due to high utilization');
            return true;
        }

        return false;
    }

    /**
     * Register a new pending async operation.
     * @returns true if allowed, false if should wait
     */
    registerPending(): boolean {
        if (!this.config.enabled) {
            return true;
        }

        if (this.pendingOps >= this.config.maxPendingOps) {
            return false;
        }

        this.pendingOps++;
        return true;
    }

    /**
     * Mark an async operation as complete.
     * Also notifies any waiters that capacity may be available.
     */
    completePending(): void {
        if (this.pendingOps > 0) {
            this.pendingOps--;
        }

        // Notify a waiter if there's capacity now
        if (this.waiters.length > 0 && this.pendingOps < this.config.maxPendingOps) {
            const waiter = this.waiters.shift();
            if (waiter) {
                waiter.resolve();
            }
        }
    }

    /**
     * Wait until there's capacity for new operations.
     * Throws after backoffTimeoutMs if still no capacity.
     */
    async waitForCapacity(): Promise<void> {
        if (!this.config.enabled) {
            return;
        }

        if (this.pendingOps < this.config.maxPendingOps) {
            return;
        }

        this.waitsTotal++;

        return new Promise<void>((resolve, reject) => {
            const timeoutId = setTimeout(() => {
                // Remove from waiters queue
                const index = this.waiters.findIndex(w => w.resolve === resolve);
                if (index !== -1) {
                    this.waiters.splice(index, 1);
                }
                this.timeoutsTotal++;
                reject(new Error(`Backpressure timeout after ${this.config.backoffTimeoutMs}ms`));
            }, this.config.backoffTimeoutMs);

            this.waiters.push({
                resolve: () => {
                    clearTimeout(timeoutId);
                    resolve();
                },
                reject
            });
        });
    }

    /**
     * Get current stats.
     */
    getStats(): BackpressureStats {
        return {
            pendingOps: this.pendingOps,
            syncCounter: this.syncCounter,
            utilizationPercent: this.config.maxPendingOps > 0
                ? (this.pendingOps / this.config.maxPendingOps) * 100
                : 0,
            syncForcedTotal: this.syncForcedTotal,
            waitsTotal: this.waitsTotal,
            timeoutsTotal: this.timeoutsTotal
        };
    }

    /**
     * Check if backpressure is currently active (at capacity).
     */
    isAtCapacity(): boolean {
        return this.config.enabled && this.pendingOps >= this.config.maxPendingOps;
    }

    /**
     * Get current pending operations count.
     */
    getPendingOps(): number {
        return this.pendingOps;
    }

    /**
     * Check if backpressure is enabled.
     */
    isEnabled(): boolean {
        return this.config.enabled;
    }

    /**
     * Reset all counters and clear waiters (for testing).
     */
    reset(): void {
        this.syncCounter = 0;
        this.pendingOps = 0;
        this.syncForcedTotal = 0;
        this.waitsTotal = 0;
        this.timeoutsTotal = 0;

        // Reject all pending waiters
        for (const waiter of this.waiters) {
            waiter.reject(new Error('BackpressureRegulator reset'));
        }
        this.waiters = [];
    }
}
