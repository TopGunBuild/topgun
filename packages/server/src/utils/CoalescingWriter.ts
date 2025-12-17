import { WebSocket } from 'ws';
import { serialize } from '@topgunbuild/core';

export interface CoalescingWriterOptions {
    /**
     * Maximum messages to batch before forcing flush.
     * Default: 100
     */
    maxBatchSize: number;

    /**
     * Maximum time to wait before flushing (ms).
     * Default: 5 (similar to Nagle's algorithm)
     */
    maxDelayMs: number;

    /**
     * Maximum batch size in bytes.
     * Default: 65536 (64KB)
     */
    maxBatchBytes: number;
}

interface QueuedMessage {
    data: Uint8Array;
    urgent: boolean;
}

/**
 * Extended metrics for CoalescingWriter performance analysis.
 */
export interface CoalescingWriterMetrics {
    /** Total messages sent */
    messagesSent: number;
    /** Total batches sent */
    batchesSent: number;
    /** Total bytes sent */
    bytesSent: number;
    /** Average messages per batch */
    avgMessagesPerBatch: number;
    /** Average bytes per batch */
    avgBytesPerBatch: number;
    /** Messages currently in queue */
    pendingMessages: number;
    /** Bytes currently pending */
    pendingBytes: number;
    /** Count of flushes triggered by size limits (batch full or bytes exceeded) */
    immediateFlushes: number;
    /** Count of flushes triggered by timer expiration */
    timedFlushes: number;
    /** Ratio of actual batch size to maxBatchSize (0-1, higher = better utilization) */
    batchUtilization: number;
    /** Ratio of immediate flushes to total flushes (high = batches filling up quickly) */
    immediateFlushRatio: number;
}

/**
 * State machine for flush scheduling.
 * Similar to Hazelcast's NioOutboundPipeline.State
 */
enum WriterState {
    IDLE,       // No pending writes
    PENDING,    // Has pending writes, flush scheduled
    FLUSHING,   // Currently flushing
}

const DEFAULT_OPTIONS: CoalescingWriterOptions = {
    maxBatchSize: 100,
    maxDelayMs: 5,
    maxBatchBytes: 65536,
};

/**
 * Per-connection write coalescing that batches multiple messages into single WebSocket frames.
 * Inspired by Hazelcast's NioOutboundPipeline batching strategy.
 */
export class CoalescingWriter {
    private socket: WebSocket;
    private queue: QueuedMessage[] = [];
    private urgentQueue: QueuedMessage[] = [];  // Priority queue for urgent messages
    private pendingBytes = 0;
    private flushTimer: NodeJS.Immediate | null = null;
    private delayTimer: NodeJS.Timeout | null = null;
    private state: WriterState = WriterState.IDLE;
    private readonly options: CoalescingWriterOptions;
    private closed = false;

    // Metrics
    private messagesSent = 0;
    private batchesSent = 0;
    private bytesSent = 0;
    private immediateFlushCount = 0;  // Size-triggered flushes
    private timedFlushCount = 0;      // Timer-triggered flushes

    constructor(socket: WebSocket, options?: Partial<CoalescingWriterOptions>) {
        this.socket = socket;
        this.options = { ...DEFAULT_OPTIONS, ...options };
    }

    /**
     * Queue a message for sending.
     * @param message - The message object to serialize and send
     * @param urgent - If true, bypass batching and send immediately
     */
    write(message: any, urgent?: boolean): void {
        if (this.closed) {
            return;
        }

        const data = serialize(message);
        this.writeRaw(data, urgent);
    }

    /**
     * Queue pre-serialized data.
     */
    writeRaw(data: Uint8Array, urgent?: boolean): void {
        if (this.closed) {
            return;
        }

        if (urgent) {
            // Urgent messages bypass batching entirely
            this.sendImmediate(data);
            return;
        }

        const msg: QueuedMessage = { data, urgent: false };
        this.queue.push(msg);
        this.pendingBytes += data.length;

        // Check if we should flush immediately due to size limits
        if (this.queue.length >= this.options.maxBatchSize ||
            this.pendingBytes >= this.options.maxBatchBytes) {
            this.immediateFlushCount++;
            this.flush();
            return;
        }

        // Schedule flush if not already scheduled
        this.scheduleFlush();
    }

    /**
     * Force flush all pending messages immediately.
     */
    flush(): void {
        if (this.closed) {
            return;
        }

        // Cancel any pending timers
        this.cancelTimers();

        // Nothing to flush
        if (this.queue.length === 0) {
            this.state = WriterState.IDLE;
            return;
        }

        this.state = WriterState.FLUSHING;

        try {
            if (this.socket.readyState !== WebSocket.OPEN) {
                // Socket not ready, discard messages
                this.queue = [];
                this.pendingBytes = 0;
                this.state = WriterState.IDLE;
                return;
            }

            if (this.queue.length === 1) {
                // Single message - send directly without batching overhead
                const msg = this.queue[0];
                this.socket.send(msg.data);
                this.messagesSent++;
                this.batchesSent++;
                this.bytesSent += msg.data.length;
            } else {
                // Multiple messages - create a batch
                const batch = this.createBatch(this.queue);
                this.socket.send(batch);
                this.messagesSent += this.queue.length;
                this.batchesSent++;
                this.bytesSent += batch.length;
            }
        } catch (error) {
            // Socket error - discard messages silently
            // The connection close handler will clean up
        }

        // Clear queue
        this.queue = [];
        this.pendingBytes = 0;
        this.state = WriterState.IDLE;
    }

    /**
     * Get writer metrics.
     */
    getMetrics(): CoalescingWriterMetrics {
        const totalFlushes = this.immediateFlushCount + this.timedFlushCount;
        return {
            messagesSent: this.messagesSent,
            batchesSent: this.batchesSent,
            bytesSent: this.bytesSent,
            avgMessagesPerBatch: this.batchesSent > 0 ? this.messagesSent / this.batchesSent : 0,
            avgBytesPerBatch: this.batchesSent > 0 ? this.bytesSent / this.batchesSent : 0,
            pendingMessages: this.queue.length,
            pendingBytes: this.pendingBytes,
            // Extended metrics for tuning analysis
            immediateFlushes: this.immediateFlushCount,
            timedFlushes: this.timedFlushCount,
            batchUtilization: this.batchesSent > 0
                ? (this.messagesSent / this.batchesSent) / this.options.maxBatchSize
                : 0,
            immediateFlushRatio: totalFlushes > 0
                ? this.immediateFlushCount / totalFlushes
                : 0,
        };
    }

    /**
     * Get current configuration options.
     */
    getOptions(): Readonly<CoalescingWriterOptions> {
        return { ...this.options };
    }

    /**
     * Close and cleanup.
     */
    close(): void {
        if (this.closed) {
            return;
        }

        this.cancelTimers();

        // Flush any remaining messages before marking as closed
        if (this.queue.length > 0) {
            this.flush();
        }

        this.closed = true;
        this.queue = [];
        this.urgentQueue = [];
        this.pendingBytes = 0;
    }

    /**
     * Send a message immediately without batching.
     */
    private sendImmediate(data: Uint8Array): void {
        if (this.socket.readyState !== WebSocket.OPEN) {
            return;
        }

        try {
            this.socket.send(data);
            this.messagesSent++;
            this.batchesSent++;
            this.bytesSent += data.length;
        } catch (error) {
            // Socket error - ignore
        }
    }

    /**
     * Schedule a flush using setImmediate + setTimeout for the delay.
     */
    private scheduleFlush(): void {
        if (this.state === WriterState.PENDING) {
            // Already scheduled
            return;
        }

        this.state = WriterState.PENDING;

        // Use setTimeout for the delay (similar to Nagle's algorithm)
        this.delayTimer = setTimeout(() => {
            this.delayTimer = null;
            // Use setImmediate to batch with other I/O in the same tick
            this.flushTimer = setImmediate(() => {
                this.flushTimer = null;
                this.timedFlushCount++;
                this.flush();
            });
        }, this.options.maxDelayMs);
    }

    /**
     * Cancel any pending flush timers.
     */
    private cancelTimers(): void {
        if (this.flushTimer) {
            clearImmediate(this.flushTimer);
            this.flushTimer = null;
        }
        if (this.delayTimer) {
            clearTimeout(this.delayTimer);
            this.delayTimer = null;
        }
    }

    /**
     * Create a batch message from multiple queued messages.
     * Uses length-prefixed format for efficiency:
     * [4 bytes: message count][4 bytes: msg1 length][msg1 bytes][4 bytes: msg2 length][msg2 bytes]...
     */
    private createBatch(messages: QueuedMessage[]): Uint8Array {
        // Calculate total size needed
        // 4 bytes for count + (4 bytes length + data) for each message
        let totalSize = 4; // message count
        for (const msg of messages) {
            totalSize += 4 + msg.data.length; // length prefix + data
        }

        const batch = new Uint8Array(totalSize);
        const view = new DataView(batch.buffer);
        let offset = 0;

        // Write message count
        view.setUint32(offset, messages.length, true); // little-endian
        offset += 4;

        // Write each message with length prefix
        for (const msg of messages) {
            view.setUint32(offset, msg.data.length, true);
            offset += 4;
            batch.set(msg.data, offset);
            offset += msg.data.length;
        }

        // Wrap in a BATCH message envelope
        const batchEnvelope = serialize({
            type: 'BATCH',
            count: messages.length,
            data: batch,
        });

        return batchEnvelope;
    }
}
