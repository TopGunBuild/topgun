import { BoundedEventQueue, QueueMetrics } from './BoundedEventQueue';
import { logger } from './logger';

export interface StripedExecutorOptions {
    /** Number of worker stripes (default: 4) */
    stripeCount: number;
    /** Total capacity across all stripes */
    queueCapacity: number;
    name: string;
    /** Callback when a task is rejected */
    onReject?: (task: StripedTask) => void;
}

export interface StripedTask {
    /** Used for stripe selection (hash % stripeCount) */
    key: string | number;
    /** The task to execute */
    execute: () => Promise<void> | void;
}

export interface StripeMetrics {
    stripe: number;
    metrics: QueueMetrics;
}

/**
 * Hazelcast-style striped executor that distributes tasks across multiple bounded queues.
 * Tasks with the same key are guaranteed to be processed in order on the same stripe.
 */
export class StripedEventExecutor {
    private stripes: BoundedEventQueue<StripedTask>[];
    private processing: boolean[];
    private readonly stripeCount: number;
    private readonly name: string;
    private readonly onReject?: (task: StripedTask) => void;
    private isShuttingDown: boolean = false;
    private pendingPromises: Set<Promise<void>> = new Set();

    constructor(options: StripedExecutorOptions) {
        this.stripeCount = options.stripeCount;
        this.name = options.name;
        this.onReject = options.onReject;

        // Distribute capacity across stripes
        const capacityPerStripe = Math.ceil(options.queueCapacity / options.stripeCount);

        this.stripes = [];
        this.processing = [];

        for (let i = 0; i < options.stripeCount; i++) {
            const queue = new BoundedEventQueue<StripedTask>({
                maxSize: capacityPerStripe,
                name: `${options.name}-stripe-${i}`,
                onReject: (task) => this.handleReject(task)
            });

            // Set up event handlers for monitoring
            queue.on('highWater', (event) => {
                logger.warn(
                    { executor: this.name, stripe: i, ...event },
                    'Stripe approaching capacity'
                );
            });

            this.stripes.push(queue);
            this.processing.push(false);
        }
    }

    /**
     * Submit a task. Tasks with same key go to same stripe (ordering guarantee).
     * @returns true if accepted, false if rejected
     */
    submit(task: StripedTask): boolean {
        if (this.isShuttingDown) {
            logger.warn({ executor: this.name }, 'Executor is shutting down, rejecting task');
            this.handleReject(task);
            return false;
        }

        const stripeIndex = this.getStripeIndex(task.key);
        const stripe = this.stripes[stripeIndex];

        const accepted = stripe.enqueue(task);

        if (accepted) {
            this.processStripe(stripeIndex);
        }

        return accepted;
    }

    /**
     * Get metrics for all stripes.
     */
    getMetrics(): StripeMetrics[] {
        return this.stripes.map((stripe, index) => ({
            stripe: index,
            metrics: stripe.getMetrics()
        }));
    }

    /**
     * Get aggregated metrics across all stripes.
     */
    getTotalMetrics(): QueueMetrics {
        const total: QueueMetrics = {
            enqueued: 0,
            dequeued: 0,
            rejected: 0,
            currentSize: 0
        };

        for (const stripe of this.stripes) {
            const metrics = stripe.getMetrics();
            total.enqueued += metrics.enqueued;
            total.dequeued += metrics.dequeued;
            total.rejected += metrics.rejected;
            total.currentSize += metrics.currentSize;
        }

        return total;
    }

    /**
     * Check if all stripes are full.
     */
    get isFull(): boolean {
        return this.stripes.every(stripe => stripe.isFull);
    }

    /**
     * Get current total size across all stripes.
     */
    get size(): number {
        return this.stripes.reduce((sum, stripe) => sum + stripe.size, 0);
    }

    /**
     * Shutdown executor, optionally waiting for pending tasks.
     */
    async shutdown(waitForPending: boolean = true): Promise<void> {
        this.isShuttingDown = true;

        if (waitForPending) {
            // Wait for all currently processing tasks to complete
            const promises = Array.from(this.pendingPromises);
            if (promises.length > 0) {
                await Promise.all(promises);
            }

            // Process any remaining items in queues
            const drainPromises: Promise<void>[] = [];
            for (let i = 0; i < this.stripeCount; i++) {
                if (!this.stripes[i].isEmpty) {
                    drainPromises.push(this.drainStripe(i));
                }
            }
            await Promise.all(drainPromises);
        }

        // Clear all queues
        for (const stripe of this.stripes) {
            stripe.clear();
        }

        logger.info({ executor: this.name }, 'Executor shutdown complete');
    }

    private getStripeIndex(key: string | number): number {
        const hash = typeof key === 'number' ? key : this.hashString(key);
        return Math.abs(hash) % this.stripeCount;
    }

    private hashString(str: string): number {
        let hash = 0;
        for (let i = 0; i < str.length; i++) {
            const char = str.charCodeAt(i);
            hash = ((hash << 5) - hash) + char;
            hash = hash & hash; // Convert to 32-bit integer
        }
        return hash;
    }

    private handleReject(task: StripedTask): void {
        this.onReject?.(task);
    }

    private async processStripe(stripeIndex: number): Promise<void> {
        // Prevent concurrent processing of the same stripe
        if (this.processing[stripeIndex]) {
            return;
        }

        this.processing[stripeIndex] = true;
        const stripe = this.stripes[stripeIndex];

        try {
            while (!stripe.isEmpty && !this.isShuttingDown) {
                const task = stripe.dequeue();
                if (!task) break;

                try {
                    const result = task.execute();
                    if (result instanceof Promise) {
                        this.pendingPromises.add(result);
                        await result;
                        this.pendingPromises.delete(result);
                    }
                } catch (error) {
                    logger.error(
                        { executor: this.name, stripe: stripeIndex, key: task.key, error },
                        'Task execution failed'
                    );
                }
            }
        } finally {
            this.processing[stripeIndex] = false;
        }
    }

    private async drainStripe(stripeIndex: number): Promise<void> {
        const stripe = this.stripes[stripeIndex];

        while (!stripe.isEmpty) {
            const task = stripe.dequeue();
            if (!task) break;

            try {
                const result = task.execute();
                if (result instanceof Promise) {
                    await result;
                }
            } catch (error) {
                logger.error(
                    { executor: this.name, stripe: stripeIndex, key: task.key, error },
                    'Task execution failed during drain'
                );
            }
        }
    }
}
