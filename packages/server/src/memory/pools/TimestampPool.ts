/**
 * TimestampPool - Pre-configured ObjectPool for HLC timestamp objects.
 * Phase 2.04: Object Pool Implementation
 *
 * Reduces GC pressure for frequent timestamp operations.
 * Timestamps are created on every operation (set, merge, etc.).
 */

import { ObjectPool } from '../ObjectPool';

/**
 * Pooled timestamp structure matching HLC format.
 */
export interface PooledTimestamp {
    millis: number;
    counter: number;
    nodeId: string;
}

const DEFAULT_MAX_SIZE = 2048;

/**
 * Create a new TimestampPool instance.
 *
 * @param config - Pool configuration
 * @returns Configured ObjectPool for timestamps
 */
export function createTimestampPool(config?: { maxSize?: number; initialSize?: number }): ObjectPool<PooledTimestamp> {
    return new ObjectPool<PooledTimestamp>({
        name: 'timestamp',
        maxSize: config?.maxSize ?? DEFAULT_MAX_SIZE,
        initialSize: config?.initialSize ?? 128,
        factory: () => ({
            millis: 0,
            counter: 0,
            nodeId: '',
        }),
        reset: (ts) => {
            ts.millis = 0;
            ts.counter = 0;
            ts.nodeId = '';
        },
        validate: (ts) => {
            // Ensure fields are correct type (for corruption detection)
            return (
                typeof ts.millis === 'number' &&
                typeof ts.counter === 'number' &&
                typeof ts.nodeId === 'string'
            );
        },
    });
}

// Global singleton instance
let globalTimestampPool: ObjectPool<PooledTimestamp> | null = null;

/**
 * Get or create the global timestamp pool.
 */
export function getGlobalTimestampPool(): ObjectPool<PooledTimestamp> {
    if (!globalTimestampPool) {
        globalTimestampPool = createTimestampPool();
    }
    return globalTimestampPool;
}

/**
 * Replace the global timestamp pool (for testing).
 */
export function setGlobalTimestampPool(pool: ObjectPool<PooledTimestamp> | null): void {
    globalTimestampPool = pool;
}
