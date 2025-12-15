/**
 * BroadcastService - Tick-Based Event Broadcast Buffer
 *
 * Implements Nagle's Algorithm for WebSocket broadcasts:
 * - Instead of broadcasting after each batch (200 times/sec at 1000 ops/sec),
 *   events are buffered and flushed periodically (20 times/sec at 50ms interval).
 *
 * Expected impact: 10x reduction in ws.send() overhead
 * - Before: 200 batches/sec × 100 clients = 20,000 frames/sec
 * - After:  20 flushes/sec × 100 clients = 2,000 frames/sec
 *
 * Design inspired by Hazelcast's BatchInvalidator:
 * - Queue + size/timeout trigger
 * - Serialize once, send to many (already in broadcastBatch)
 *
 * @see /Users/koristuvac/.gemini/antigravity/brain/e966fdb5-80cd-44ae-afe4-018e15b32abd/PERFORMANCE_AUDIT_REPORT.md.resolved
 */

import { logger } from '../utils/logger';

export interface BroadcastEvent {
    mapName: string;
    eventType: string;
    key: string;
    record?: any;
    orRecord?: any;
    orTag?: string;
}

export interface BroadcastServiceConfig {
    /** Flush interval in milliseconds (default: 50ms = 20 flushes/sec) */
    flushIntervalMs?: number;
    /** Maximum events in buffer before immediate flush (default: 1000) */
    maxBufferSize?: number;
    /** Minimum events required to trigger a flush (adaptive logic) */
    minBatchSize?: number;
    /** Enable adaptive flushing - flush immediately if buffer is small and idle (default: true) */
    adaptiveFlush?: boolean;
}

export type BroadcastCallback = (events: BroadcastEvent[], excludeClientId?: string) => void;

interface BufferedBatch {
    events: BroadcastEvent[];
    excludeClientId?: string;
}

export class BroadcastService {
    private eventBuffer: BufferedBatch[] = [];
    private flushTimer: NodeJS.Timeout | null = null;
    private broadcastCallback: BroadcastCallback;
    private isRunning = false;

    // Configuration
    private readonly flushIntervalMs: number;
    private readonly maxBufferSize: number;
    private readonly minBatchSize: number;
    private readonly adaptiveFlush: boolean;

    // Metrics
    private totalEventsBuffered = 0;
    private totalFlushes = 0;
    private totalEventsDelivered = 0;
    private lastFlushTime = Date.now();

    constructor(broadcastCallback: BroadcastCallback, config: BroadcastServiceConfig = {}) {
        this.broadcastCallback = broadcastCallback;
        this.flushIntervalMs = config.flushIntervalMs ?? 50; // 20 flushes/sec
        this.maxBufferSize = config.maxBufferSize ?? 1000;
        this.minBatchSize = config.minBatchSize ?? 1;
        this.adaptiveFlush = config.adaptiveFlush ?? true;

        logger.info({
            flushIntervalMs: this.flushIntervalMs,
            maxBufferSize: this.maxBufferSize,
            adaptiveFlush: this.adaptiveFlush
        }, 'BroadcastService initialized');
    }

    /**
     * Start the broadcast service timer
     */
    start(): void {
        if (this.isRunning) return;

        this.isRunning = true;
        this.scheduleFlush();
        logger.info('BroadcastService started');
    }

    /**
     * Stop the broadcast service and flush remaining events
     */
    stop(): void {
        if (!this.isRunning) return;

        this.isRunning = false;
        if (this.flushTimer) {
            clearTimeout(this.flushTimer);
            this.flushTimer = null;
        }

        // Final flush
        if (this.eventBuffer.length > 0) {
            this.flush();
        }

        logger.info({
            totalEventsBuffered: this.totalEventsBuffered,
            totalFlushes: this.totalFlushes,
            totalEventsDelivered: this.totalEventsDelivered
        }, 'BroadcastService stopped');
    }

    /**
     * Buffer events for batched broadcast
     *
     * @param events - Array of events to broadcast
     * @param excludeClientId - Optional client ID to exclude from broadcast
     */
    buffer(events: BroadcastEvent[], excludeClientId?: string): void {
        if (events.length === 0) return;

        this.eventBuffer.push({ events, excludeClientId });
        this.totalEventsBuffered += events.length;

        // Check for immediate flush conditions
        const totalEvents = this.getBufferEventCount();

        // Condition 1: Buffer size exceeded - flush immediately
        if (totalEvents >= this.maxBufferSize) {
            logger.debug({ totalEvents, maxBufferSize: this.maxBufferSize }, 'Buffer size exceeded, flushing immediately');
            this.flush();
            return;
        }

        // Condition 2: Adaptive flush - if we're under load and have accumulated enough
        // This prevents unnecessary batching delay for low-traffic scenarios
        if (this.adaptiveFlush && this.shouldAdaptiveFlush(totalEvents)) {
            this.flush();
        }
    }

    /**
     * Flush all buffered events immediately
     */
    flush(): void {
        if (this.eventBuffer.length === 0) return;

        const batches = this.eventBuffer;
        this.eventBuffer = [];

        // Group events by excludeClientId for optimal broadcasting
        const groupedByClient = this.groupByExcludeClient(batches);

        for (const [excludeClientId, events] of groupedByClient) {
            if (events.length > 0) {
                try {
                    this.broadcastCallback(events, excludeClientId || undefined);
                    this.totalEventsDelivered += events.length;
                } catch (err) {
                    logger.error({ err, eventCount: events.length }, 'Failed to broadcast events');
                }
            }
        }

        this.totalFlushes++;
        this.lastFlushTime = Date.now();

        // Log stats every 100 flushes for debugging high-load scenarios
        if (this.totalFlushes % 100 === 0) {
            const stats = this.getStats();
            logger.info({
                flushNumber: this.totalFlushes,
                avgEventsPerFlush: stats.avgEventsPerFlush,
                totalEventsDelivered: stats.totalEventsDelivered
            }, 'BroadcastService flush stats');
        }

        // Reschedule if running
        if (this.isRunning) {
            this.scheduleFlush();
        }
    }

    /**
     * Get current buffer statistics
     */
    getStats(): {
        bufferSize: number;
        totalEventsBuffered: number;
        totalFlushes: number;
        totalEventsDelivered: number;
        avgEventsPerFlush: number;
        timeSinceLastFlush: number;
    } {
        return {
            bufferSize: this.getBufferEventCount(),
            totalEventsBuffered: this.totalEventsBuffered,
            totalFlushes: this.totalFlushes,
            totalEventsDelivered: this.totalEventsDelivered,
            avgEventsPerFlush: this.totalFlushes > 0
                ? Math.round(this.totalEventsDelivered / this.totalFlushes)
                : 0,
            timeSinceLastFlush: Date.now() - this.lastFlushTime
        };
    }

    /**
     * Check if service is running
     */
    isActive(): boolean {
        return this.isRunning;
    }

    // Private methods

    private scheduleFlush(): void {
        if (this.flushTimer) {
            clearTimeout(this.flushTimer);
        }

        this.flushTimer = setTimeout(() => {
            this.flush();
        }, this.flushIntervalMs);
    }

    private getBufferEventCount(): number {
        return this.eventBuffer.reduce((sum, batch) => sum + batch.events.length, 0);
    }

    /**
     * Adaptive flush logic - flush immediately if:
     * 1. Under low load (few events) and timer hasn't fired recently
     * 2. This prevents unnecessary latency for sporadic writes
     */
    private shouldAdaptiveFlush(totalEvents: number): boolean {
        // If we have very few events and it's been a while since last flush,
        // don't wait for the timer - this keeps latency low for low-traffic scenarios
        const timeSinceLastFlush = Date.now() - this.lastFlushTime;

        // If buffer has few events and we've waited at least half the interval,
        // flush immediately to reduce latency
        if (totalEvents <= this.minBatchSize && timeSinceLastFlush >= this.flushIntervalMs / 2) {
            return true;
        }

        return false;
    }

    /**
     * Group batched events by excludeClientId for efficient broadcasting
     * This ensures clients are excluded properly while maximizing batch sizes
     */
    private groupByExcludeClient(batches: BufferedBatch[]): Map<string | null, BroadcastEvent[]> {
        const grouped = new Map<string | null, BroadcastEvent[]>();

        for (const batch of batches) {
            const key = batch.excludeClientId ?? null;
            const existing = grouped.get(key) ?? [];
            existing.push(...batch.events);
            grouped.set(key, existing);
        }

        return grouped;
    }
}
