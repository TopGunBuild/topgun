import { EventEmitter } from 'events';
import { logger } from './logger';

export interface QueueMetrics {
    enqueued: number;
    dequeued: number;
    rejected: number;
    currentSize: number;
}

export interface BoundedEventQueueOptions {
    maxSize: number;
    name: string;
    onReject?: (item: any) => void;
    /** High water mark percentage (0-1). Emits 'highWater' when reached. Default: 0.8 */
    highWaterMark?: number;
}

export class BoundedEventQueue<T> extends EventEmitter {
    private queue: T[] = [];
    private readonly maxSize: number;
    private readonly name: string;
    private readonly onReject?: (item: any) => void;
    private readonly highWaterMark: number;
    private highWaterEmitted: boolean = false;
    private metrics: QueueMetrics = {
        enqueued: 0,
        dequeued: 0,
        rejected: 0,
        currentSize: 0
    };

    constructor(options: BoundedEventQueueOptions) {
        super();
        this.maxSize = options.maxSize;
        this.name = options.name;
        this.onReject = options.onReject;
        this.highWaterMark = options.highWaterMark ?? 0.8;
    }

    /**
     * Attempt to enqueue an item.
     * @returns true if enqueued, false if rejected due to capacity
     */
    enqueue(item: T): boolean {
        if (this.queue.length >= this.maxSize) {
            this.metrics.rejected++;
            logger.warn(
                { queue: this.name, currentSize: this.queue.length, maxSize: this.maxSize },
                'Queue full, rejecting item'
            );
            this.onReject?.(item);
            return false;
        }

        this.queue.push(item);
        this.metrics.enqueued++;
        this.metrics.currentSize = this.queue.length;

        // Check for high water mark
        const usage = this.queue.length / this.maxSize;
        if (usage >= this.highWaterMark && !this.highWaterEmitted) {
            this.highWaterEmitted = true;
            this.emit('highWater', { name: this.name, usage, size: this.queue.length });
        }

        // Check if queue just became full
        if (this.queue.length === this.maxSize) {
            this.emit('full', { name: this.name, size: this.queue.length });
        }

        return true;
    }

    /**
     * Dequeue an item. Returns undefined if queue is empty.
     */
    dequeue(): T | undefined {
        const item = this.queue.shift();

        if (item !== undefined) {
            this.metrics.dequeued++;
            this.metrics.currentSize = this.queue.length;

            // Reset high water mark flag when below threshold
            const usage = this.queue.length / this.maxSize;
            if (usage < this.highWaterMark) {
                this.highWaterEmitted = false;
            }

            // Check if queue just became empty
            if (this.queue.length === 0) {
                this.emit('empty', { name: this.name });
            }
        }

        return item;
    }

    /**
     * Peek at front item without removing.
     */
    peek(): T | undefined {
        return this.queue[0];
    }

    /**
     * Current queue size.
     */
    get size(): number {
        return this.queue.length;
    }

    /**
     * Check if queue is full.
     */
    get isFull(): boolean {
        return this.queue.length >= this.maxSize;
    }

    /**
     * Check if queue is empty.
     */
    get isEmpty(): boolean {
        return this.queue.length === 0;
    }

    /**
     * Get queue metrics.
     */
    getMetrics(): QueueMetrics {
        return { ...this.metrics };
    }

    /**
     * Clear the queue.
     */
    clear(): void {
        const wasNotEmpty = this.queue.length > 0;
        this.queue = [];
        this.metrics.currentSize = 0;
        this.highWaterEmitted = false;

        if (wasNotEmpty) {
            this.emit('empty', { name: this.name });
        }
    }
}
