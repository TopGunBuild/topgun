/**
 * BufferPool - High-performance buffer reuse for serialization operations.
 * Phase 2.03: Memory Pooling
 *
 * Reduces GC pressure by reusing pre-allocated Uint8Array buffers instead of
 * creating new ones for each serialization operation.
 *
 * Reference: Hazelcast MemoryAllocator pattern
 */

export interface BufferPoolConfig {
    /**
     * Size of each buffer chunk in bytes.
     * Larger chunks = fewer allocations, but more memory waste.
     * Default: 64KB (65536)
     */
    chunkSize?: number;

    /**
     * Initial number of buffers to pre-allocate.
     * Default: 16
     */
    initialSize?: number;

    /**
     * Maximum buffers to keep in pool.
     * Excess buffers are released to GC.
     * Default: 256
     */
    maxSize?: number;

    /**
     * Auto-shrink pool when idle buffers exceed this ratio.
     * Default: true
     */
    autoShrink?: boolean;

    /**
     * Shrink threshold - shrink if (available / maxSize) > threshold.
     * Default: 0.75 (75% idle)
     */
    shrinkThreshold?: number;

    /**
     * Enable metrics collection.
     * Default: true
     */
    metricsEnabled?: boolean;
}

export interface BufferPoolStats {
    /** Buffers currently available in pool */
    available: number;
    /** Buffers currently in use */
    inUse: number;
    /** Total buffers ever created */
    created: number;
    /** Total times a buffer was reused */
    reused: number;
    /** Reuse ratio: reused / (created + reused) */
    reuseRatio: number;
    /** Total bytes in pool (available buffers only) */
    poolSizeBytes: number;
    /** Peak concurrent usage */
    peakUsage: number;
    /** Times pool was exhausted (had to create new buffer) */
    misses: number;
    /** Chunk size configuration */
    chunkSize: number;
    /** Max pool size configuration */
    maxSize: number;
}

const DEFAULT_CONFIG: Required<BufferPoolConfig> = {
    chunkSize: 65536,        // 64KB
    initialSize: 16,
    maxSize: 256,
    autoShrink: true,
    shrinkThreshold: 0.75,
    metricsEnabled: true,
};

/**
 * High-performance buffer pool for serialization operations.
 *
 * Usage:
 * ```typescript
 * const pool = new BufferPool({ chunkSize: 64 * 1024 });
 * const buffer = pool.acquire();
 * // ... use buffer for serialization ...
 * pool.release(buffer);
 * ```
 *
 * Thread Safety:
 * This pool is designed for single-threaded use (Node.js main thread).
 * For worker threads, create separate pools or copy buffers.
 */
export class BufferPool {
    private readonly pool: Uint8Array[] = [];
    private readonly chunkSize: number;
    private readonly maxSize: number;
    private readonly autoShrink: boolean;
    private readonly shrinkThreshold: number;
    private readonly metricsEnabled: boolean;

    // Metrics
    private inUseCount = 0;
    private createdCount = 0;
    private reusedCount = 0;
    private peakUsage = 0;
    private missCount = 0;

    // Track buffers from this pool (for debugging/validation)
    private readonly pooledBuffers = new WeakSet<Uint8Array>();

    // Track released buffers to detect double-release
    private readonly releasedBuffers = new WeakSet<Uint8Array>();

    constructor(config?: BufferPoolConfig) {
        const cfg = { ...DEFAULT_CONFIG, ...config };

        this.chunkSize = cfg.chunkSize;
        this.maxSize = cfg.maxSize;
        this.autoShrink = cfg.autoShrink;
        this.shrinkThreshold = cfg.shrinkThreshold;
        this.metricsEnabled = cfg.metricsEnabled;

        // Pre-warm pool
        this.prewarm(cfg.initialSize);
    }

    /**
     * Acquire a buffer from the pool.
     * Creates new buffer if pool is empty.
     *
     * @returns A Uint8Array of exactly chunkSize bytes
     */
    acquire(): Uint8Array {
        let buffer: Uint8Array;

        if (this.pool.length > 0) {
            buffer = this.pool.pop()!;
            this.releasedBuffers.delete(buffer);
            if (this.metricsEnabled) {
                this.reusedCount++;
            }
        } else {
            buffer = this.createBuffer(this.chunkSize);
            if (this.metricsEnabled) {
                this.missCount++;
            }
        }

        if (this.metricsEnabled) {
            this.inUseCount++;
            this.peakUsage = Math.max(this.peakUsage, this.inUseCount);
        }

        return buffer;
    }

    /**
     * Release a buffer back to the pool.
     * Buffer contents are cleared before returning to pool.
     *
     * @param buffer - The buffer to release
     */
    release(buffer: Uint8Array): void {
        // Skip if already released (idempotent)
        if (this.releasedBuffers.has(buffer)) {
            return;
        }

        if (this.metricsEnabled) {
            this.inUseCount = Math.max(0, this.inUseCount - 1);
        }

        // Don't return oversized buffers to pool - let GC handle
        if (buffer.byteLength > this.chunkSize) {
            return;
        }

        // Don't return undersized buffers (shouldn't happen, but be safe)
        if (buffer.byteLength < this.chunkSize) {
            return;
        }

        // Don't exceed max pool size - let GC handle excess
        if (this.pool.length >= this.maxSize) {
            return;
        }

        // Clear buffer before returning (security + helps with debugging)
        // Note: fill(0) is optimized in V8 for typed arrays
        buffer.fill(0);

        this.pool.push(buffer);
        this.releasedBuffers.add(buffer);

        // Auto-shrink if needed
        if (this.autoShrink && this.shouldShrink()) {
            this.shrink();
        }
    }

    /**
     * Acquire a buffer of specific minimum size.
     * May return a buffer larger than requested.
     * For sizes > chunkSize, creates new buffer (not pooled).
     *
     * @param minSize - Minimum required size in bytes
     * @returns A Uint8Array of at least minSize bytes
     */
    acquireSize(minSize: number): Uint8Array {
        // For zero or negative, return standard chunk
        if (minSize <= 0) {
            return this.acquire();
        }

        // If fits in chunk, use pool
        if (minSize <= this.chunkSize) {
            return this.acquire();
        }

        // Large buffer - create directly, not pooled
        // These will be GC'd after use
        return this.createBuffer(minSize);
    }

    /**
     * Get current pool statistics.
     */
    getStats(): BufferPoolStats {
        const total = this.createdCount + this.reusedCount;
        return {
            available: this.pool.length,
            inUse: this.inUseCount,
            created: this.createdCount,
            reused: this.reusedCount,
            reuseRatio: total > 0 ? this.reusedCount / total : 0,
            poolSizeBytes: this.pool.length * this.chunkSize,
            peakUsage: this.peakUsage,
            misses: this.missCount,
            chunkSize: this.chunkSize,
            maxSize: this.maxSize,
        };
    }

    /**
     * Clear all buffers from pool.
     * Use for shutdown or memory pressure situations.
     */
    clear(): void {
        this.pool.length = 0;
    }

    /**
     * Manually trigger shrink operation.
     * Removes excess idle buffers to reduce memory footprint.
     */
    shrink(): void {
        // Target 50% of threshold to avoid immediate re-shrink
        const targetSize = Math.floor(this.maxSize * (this.shrinkThreshold - 0.25));
        const safeTarget = Math.max(0, targetSize);

        while (this.pool.length > safeTarget) {
            this.pool.pop(); // Let GC collect
        }
    }

    /**
     * Pre-warm pool by creating buffers up to specified count.
     * Called automatically in constructor.
     *
     * @param count - Number of buffers to create
     */
    prewarm(count?: number): void {
        const targetCount = count ?? DEFAULT_CONFIG.initialSize;
        const toCreate = Math.min(targetCount, this.maxSize) - this.pool.length;

        for (let i = 0; i < toCreate; i++) {
            const buffer = this.createBuffer(this.chunkSize);
            this.pool.push(buffer);
            this.releasedBuffers.add(buffer);
        }
    }

    /**
     * Reset all metrics to zero.
     * Useful for testing or periodic metric collection.
     */
    resetStats(): void {
        this.createdCount = 0;
        this.reusedCount = 0;
        this.peakUsage = this.inUseCount;
        this.missCount = 0;
    }

    /**
     * Get configuration values.
     */
    getConfig(): Readonly<Required<BufferPoolConfig>> {
        return {
            chunkSize: this.chunkSize,
            initialSize: DEFAULT_CONFIG.initialSize,
            maxSize: this.maxSize,
            autoShrink: this.autoShrink,
            shrinkThreshold: this.shrinkThreshold,
            metricsEnabled: this.metricsEnabled,
        };
    }

    // ============ Private Methods ============

    private createBuffer(size: number): Uint8Array {
        const buffer = new Uint8Array(size);
        this.pooledBuffers.add(buffer);
        if (this.metricsEnabled) {
            this.createdCount++;
        }
        return buffer;
    }

    private shouldShrink(): boolean {
        if (this.maxSize === 0) return false;
        const ratio = this.pool.length / this.maxSize;
        return ratio > this.shrinkThreshold;
    }
}

/**
 * Global shared buffer pool instance.
 * Use this for most serialization operations.
 *
 * For custom configurations or isolated pools, create new BufferPool instances.
 */
let globalPool: BufferPool | null = null;

/**
 * Get or create the global buffer pool.
 * Uses default configuration (64KB chunks, 256 max).
 */
export function getGlobalBufferPool(): BufferPool {
    if (!globalPool) {
        globalPool = new BufferPool();
    }
    return globalPool;
}

/**
 * Replace the global buffer pool with a custom instance.
 * Useful for testing or custom configurations.
 *
 * @param pool - New pool instance, or null to reset to default
 */
export function setGlobalBufferPool(pool: BufferPool | null): void {
    globalPool = pool;
}
