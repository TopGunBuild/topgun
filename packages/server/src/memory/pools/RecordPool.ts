/**
 * RecordPool - Pre-configured ObjectPool for LWW/OR record objects.
 * Phase 2.04: Object Pool Implementation
 *
 * Reduces GC pressure when processing CRDT operations.
 */

import { ObjectPool } from '../ObjectPool';
import type { PooledTimestamp } from './TimestampPool';

/**
 * Pooled record structure matching LWWRecord format.
 */
export interface PooledRecord<T = unknown> {
    value: T | null;
    timestamp: PooledTimestamp | null;
    ttlMs?: number;
}

/**
 * Pooled event payload structure.
 */
export interface PooledEventPayload {
    mapName: string;
    key: string;
    eventType: string;
    record: PooledRecord | null;
    orRecord: PooledRecord | null;
    orTag: string | null;
}

const DEFAULT_MAX_SIZE = 4096;
const DEFAULT_EVENT_MAX_SIZE = 2048;

/**
 * Create a new RecordPool instance.
 *
 * @param config - Pool configuration
 * @returns Configured ObjectPool for records
 */
export function createRecordPool<T = unknown>(config?: { maxSize?: number; initialSize?: number }): ObjectPool<PooledRecord<T>> {
    return new ObjectPool<PooledRecord<T>>({
        name: 'record',
        maxSize: config?.maxSize ?? DEFAULT_MAX_SIZE,
        initialSize: config?.initialSize ?? 64,
        factory: () => ({
            value: null,
            timestamp: null,
            ttlMs: undefined,
        }),
        reset: (rec) => {
            rec.value = null;
            rec.timestamp = null;
            rec.ttlMs = undefined;
        },
    });
}

/**
 * Create a new EventPayloadPool instance.
 *
 * @param config - Pool configuration
 * @returns Configured ObjectPool for event payloads
 */
export function createEventPayloadPool(config?: { maxSize?: number; initialSize?: number }): ObjectPool<PooledEventPayload> {
    return new ObjectPool<PooledEventPayload>({
        name: 'eventPayload',
        maxSize: config?.maxSize ?? DEFAULT_EVENT_MAX_SIZE,
        initialSize: config?.initialSize ?? 64,
        factory: () => ({
            mapName: '',
            key: '',
            eventType: '',
            record: null,
            orRecord: null,
            orTag: null,
        }),
        reset: (payload) => {
            payload.mapName = '';
            payload.key = '';
            payload.eventType = '';
            payload.record = null;
            payload.orRecord = null;
            payload.orTag = null;
        },
    });
}

// Global singleton instances
let globalRecordPool: ObjectPool<PooledRecord> | null = null;
let globalEventPayloadPool: ObjectPool<PooledEventPayload> | null = null;

/**
 * Get or create the global record pool.
 */
export function getGlobalRecordPool(): ObjectPool<PooledRecord> {
    if (!globalRecordPool) {
        globalRecordPool = createRecordPool();
    }
    return globalRecordPool;
}

/**
 * Replace the global record pool (for testing).
 */
export function setGlobalRecordPool(pool: ObjectPool<PooledRecord> | null): void {
    globalRecordPool = pool;
}

/**
 * Get or create the global event payload pool.
 */
export function getGlobalEventPayloadPool(): ObjectPool<PooledEventPayload> {
    if (!globalEventPayloadPool) {
        globalEventPayloadPool = createEventPayloadPool();
    }
    return globalEventPayloadPool;
}

/**
 * Replace the global event payload pool (for testing).
 */
export function setGlobalEventPayloadPool(pool: ObjectPool<PooledEventPayload> | null): void {
    globalEventPayloadPool = pool;
}
