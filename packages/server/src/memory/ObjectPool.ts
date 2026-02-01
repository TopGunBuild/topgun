/**
 * ObjectPool - Generic object pooling for reducing GC pressure.
 * Object Pool Implementation
 *
 * Reuses objects instead of creating new ones in hot paths.
 * Works with any plain data structure type.
 */

export interface ObjectPoolConfig<T> {
    /**
     * Factory function to create new objects.
     * Called when pool is empty.
     */
    factory: () => T;

    /**
     * Reset function to clean object before reuse.
     * Should clear all fields to initial state.
     */
    reset: (obj: T) => void;

    /**
     * Optional validator to check if object is reusable.
     * Return false to discard corrupted objects.
     */
    validate?: (obj: T) => boolean;

    /**
     * Initial pool size.
     * Default: 32
     */
    initialSize?: number;

    /**
     * Maximum pool size.
     * Excess objects are discarded.
     * Default: 512
     */
    maxSize?: number;

    /**
     * Name for debugging/metrics.
     */
    name?: string;
}

export interface ObjectPoolStats {
    /** Pool name */
    name: string;
    /** Objects currently available */
    available: number;
    /** Objects currently in use */
    inUse: number;
    /** Total objects created */
    created: number;
    /** Total times an object was reused */
    reused: number;
    /** Reuse ratio */
    reuseRatio: number;
    /** Objects discarded (failed validation) */
    discarded: number;
    /** Max pool size */
    maxSize: number;
    /** Peak concurrent usage */
    peakUsage: number;
}

const DEFAULT_INITIAL_SIZE = 32;
const DEFAULT_MAX_SIZE = 512;

/**
 * Generic object pool for reusing plain data structures.
 *
 * Usage:
 * ```typescript
 * const pool = new ObjectPool({
 *   factory: () => ({ name: '', value: 0 }),
 *   reset: (obj) => { obj.name = ''; obj.value = 0; },
 * });
 *
 * const obj = pool.acquire();
 * obj.name = 'test';
 * obj.value = 42;
 * // ... use obj ...
 * pool.release(obj);
 * ```
 *
 * Important:
 * - Only use for plain data objects
 * - Don't pool objects with closures or event listeners
 * - Don't keep references after release
 */
export class ObjectPool<T> {
    private readonly pool: T[] = [];
    private readonly factory: () => T;
    private readonly resetFn: (obj: T) => void;
    private readonly validate?: (obj: T) => boolean;
    private readonly maxSize: number;
    private readonly name: string;

    // Metrics
    private inUseCount = 0;
    private createdCount = 0;
    private reusedCount = 0;
    private discardedCount = 0;
    private peakUsage = 0;

    // Track released objects for idempotent release
    private readonly releasedObjects = new WeakSet<object>();

    constructor(config: ObjectPoolConfig<T>) {
        this.factory = config.factory;
        this.resetFn = config.reset;
        this.validate = config.validate;
        this.maxSize = config.maxSize ?? DEFAULT_MAX_SIZE;
        this.name = config.name ?? 'unnamed';

        // Pre-warm
        const initialSize = config.initialSize ?? DEFAULT_INITIAL_SIZE;
        this.prewarm(initialSize);
    }

    /**
     * Acquire an object from the pool.
     * Creates new object via factory if pool is empty.
     *
     * @returns A clean, ready-to-use object
     */
    acquire(): T {
        let obj: T;

        if (this.pool.length > 0) {
            obj = this.pool.pop()!;

            // Remove from released set
            if (typeof obj === 'object' && obj !== null) {
                this.releasedObjects.delete(obj as object);
            }

            // Validate if validator provided
            if (this.validate && !this.validate(obj)) {
                this.discardedCount++;
                obj = this.createObject();
            } else {
                this.reusedCount++;
            }
        } else {
            obj = this.createObject();
        }

        this.inUseCount++;
        this.peakUsage = Math.max(this.peakUsage, this.inUseCount);
        return obj;
    }

    /**
     * Release an object back to the pool.
     * Object is reset before returning to pool.
     *
     * @param obj - The object to release
     */
    release(obj: T): void {
        // Skip if already released (idempotent)
        if (typeof obj === 'object' && obj !== null) {
            if (this.releasedObjects.has(obj as object)) {
                return;
            }
        }

        this.inUseCount = Math.max(0, this.inUseCount - 1);

        // Don't exceed max pool size
        if (this.pool.length >= this.maxSize) {
            return; // Let GC handle it
        }

        // Validate before returning
        if (this.validate && !this.validate(obj)) {
            this.discardedCount++;
            return;
        }

        // Reset object
        this.resetFn(obj);

        // Track as released
        if (typeof obj === 'object' && obj !== null) {
            this.releasedObjects.add(obj as object);
        }

        this.pool.push(obj);
    }

    /**
     * Acquire multiple objects at once.
     * More efficient than multiple acquire() calls.
     *
     * @param count - Number of objects to acquire
     * @returns Array of objects
     */
    acquireBatch(count: number): T[] {
        const result: T[] = new Array(count);
        for (let i = 0; i < count; i++) {
            result[i] = this.acquire();
        }
        return result;
    }

    /**
     * Release multiple objects at once.
     *
     * @param objects - Objects to release
     */
    releaseBatch(objects: T[]): void {
        for (const obj of objects) {
            this.release(obj);
        }
    }

    /**
     * Get pool statistics.
     */
    getStats(): ObjectPoolStats {
        const total = this.createdCount + this.reusedCount;
        return {
            name: this.name,
            available: this.pool.length,
            inUse: this.inUseCount,
            created: this.createdCount,
            reused: this.reusedCount,
            reuseRatio: total > 0 ? this.reusedCount / total : 0,
            discarded: this.discardedCount,
            maxSize: this.maxSize,
            peakUsage: this.peakUsage,
        };
    }

    /**
     * Clear all objects from pool.
     */
    clear(): void {
        this.pool.length = 0;
    }

    /**
     * Pre-warm pool with objects.
     *
     * @param count - Number of objects to create
     */
    prewarm(count: number = DEFAULT_INITIAL_SIZE): void {
        const toCreate = Math.min(count, this.maxSize) - this.pool.length;
        for (let i = 0; i < toCreate; i++) {
            const obj = this.createObject();
            if (typeof obj === 'object' && obj !== null) {
                this.releasedObjects.add(obj as object);
            }
            this.pool.push(obj);
        }
    }

    /**
     * Reset statistics.
     */
    resetStats(): void {
        this.createdCount = 0;
        this.reusedCount = 0;
        this.discardedCount = 0;
        this.peakUsage = this.inUseCount;
    }

    // ============ Private Methods ============

    private createObject(): T {
        this.createdCount++;
        return this.factory();
    }
}
