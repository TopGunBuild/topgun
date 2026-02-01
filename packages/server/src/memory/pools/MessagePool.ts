/**
 * MessagePool - Pre-configured ObjectPool for message objects.
 * Object Pool Implementation
 *
 * Reduces GC pressure when processing incoming messages.
 */

import { ObjectPool } from '../ObjectPool';

/**
 * Pooled message structure matching common message format.
 */
export interface PooledMessage {
    type: string;
    payload: unknown;
    timestamp: number | null;
    clientId: string | null;
    mapName: string | null;
    key: string | null;
}

const DEFAULT_MAX_SIZE = 1024;

/**
 * Create a new MessagePool instance.
 *
 * @param config - Pool configuration
 * @returns Configured ObjectPool for messages
 */
export function createMessagePool(config?: { maxSize?: number; initialSize?: number }): ObjectPool<PooledMessage> {
    return new ObjectPool<PooledMessage>({
        name: 'message',
        maxSize: config?.maxSize ?? DEFAULT_MAX_SIZE,
        initialSize: config?.initialSize ?? 64,
        factory: () => ({
            type: '',
            payload: null,
            timestamp: null,
            clientId: null,
            mapName: null,
            key: null,
        }),
        reset: (msg) => {
            msg.type = '';
            msg.payload = null;
            msg.timestamp = null;
            msg.clientId = null;
            msg.mapName = null;
            msg.key = null;
        },
    });
}

// Global singleton instance
let globalMessagePool: ObjectPool<PooledMessage> | null = null;

/**
 * Get or create the global message pool.
 */
export function getGlobalMessagePool(): ObjectPool<PooledMessage> {
    if (!globalMessagePool) {
        globalMessagePool = createMessagePool();
    }
    return globalMessagePool;
}

/**
 * Replace the global message pool (for testing).
 */
export function setGlobalMessagePool(pool: ObjectPool<PooledMessage> | null): void {
    globalMessagePool = pool;
}
