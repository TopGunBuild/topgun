import { ObjectPool, ObjectPoolConfig } from '../ObjectPool';
import {
    createMessagePool,
    getGlobalMessagePool,
    setGlobalMessagePool,
    PooledMessage,
} from '../pools/MessagePool';
import {
    createTimestampPool,
    getGlobalTimestampPool,
    setGlobalTimestampPool,
    PooledTimestamp,
} from '../pools/TimestampPool';
import {
    createRecordPool,
    createEventPayloadPool,
} from '../pools/RecordPool';

interface TestObject {
    name: string;
    value: number;
    data: unknown;
}

describe('ObjectPool', () => {
    const createTestPool = (config?: Partial<ObjectPoolConfig<TestObject>>) =>
        new ObjectPool<TestObject>({
            factory: () => ({ name: '', value: 0, data: null }),
            reset: (obj) => {
                obj.name = '';
                obj.value = 0;
                obj.data = null;
            },
            name: 'test',
            initialSize: 4,
            maxSize: 20,
            ...config,
        });

    describe('Basic Operations', () => {
        it('should create objects via factory', () => {
            const pool = createTestPool({ initialSize: 0 });
            const obj = pool.acquire();

            expect(obj).toEqual({ name: '', value: 0, data: null });
            expect(pool.getStats().created).toBe(1);
        });

        it('should reuse released objects', () => {
            const pool = createTestPool({ initialSize: 0 });
            const obj1 = pool.acquire();
            obj1.name = 'test';
            pool.release(obj1);

            const obj2 = pool.acquire();
            expect(pool.getStats().reused).toBe(1);
            // Object should be reset
            expect(obj2.name).toBe('');
        });

        it('should reset objects on release', () => {
            const pool = createTestPool();
            const obj = pool.acquire();
            obj.name = 'modified';
            obj.value = 999;
            obj.data = { complex: 'data' };

            pool.release(obj);

            const obj2 = pool.acquire();
            expect(obj2.name).toBe('');
            expect(obj2.value).toBe(0);
            expect(obj2.data).toBeNull();
        });

        it('should validate objects if validator provided', () => {
            let validatorCalled = 0;
            const pool = new ObjectPool<TestObject>({
                factory: () => ({ name: '', value: 0, data: null }),
                reset: (obj) => {
                    obj.name = '';
                    obj.value = 0;
                    obj.data = null;
                },
                validate: (obj) => {
                    validatorCalled++;
                    return typeof obj.name === 'string';
                },
                initialSize: 2,
            });

            const obj = pool.acquire();
            expect(validatorCalled).toBe(1); // Called on acquire from pool

            pool.release(obj);
            expect(validatorCalled).toBe(2); // Called on release
        });

        it('should discard invalid objects on acquire', () => {
            const pool = new ObjectPool<TestObject>({
                factory: () => ({ name: '', value: 0, data: null }),
                reset: (obj) => {
                    obj.name = '';
                    obj.value = 0;
                    obj.data = null;
                },
                validate: (obj) => obj.value < 100, // Fail if value >= 100
                initialSize: 1,
            });

            const obj = pool.acquire();
            obj.value = 200; // Will fail validation

            // Manually put corrupted object back
            (pool as any).pool.push(obj);

            // Next acquire should discard and create new
            const obj2 = pool.acquire();
            expect(obj2.value).toBe(0);
            expect(pool.getStats().discarded).toBe(1);
        });
    });

    describe('Batch Operations', () => {
        it('should acquire batch efficiently', () => {
            const pool = createTestPool({ initialSize: 10 });
            const batch = pool.acquireBatch(5);

            expect(batch.length).toBe(5);
            expect(pool.getStats().inUse).toBe(5);
            expect(pool.getStats().available).toBe(5);
        });

        it('should release batch efficiently', () => {
            const pool = createTestPool({ initialSize: 10 });
            const batch = pool.acquireBatch(5);

            batch.forEach((obj, i) => {
                obj.name = `item-${i}`;
            });

            pool.releaseBatch(batch);

            expect(pool.getStats().inUse).toBe(0);
            expect(pool.getStats().available).toBe(10);

            // All should be reset
            const newBatch = pool.acquireBatch(5);
            newBatch.forEach((obj) => {
                expect(obj.name).toBe('');
            });
        });
    });

    describe('Pool Limits', () => {
        it('should respect maxSize', () => {
            const pool = createTestPool({ initialSize: 0, maxSize: 3 });

            const objects = [];
            for (let i = 0; i < 5; i++) {
                objects.push(pool.acquire());
            }

            for (const obj of objects) {
                pool.release(obj);
            }

            expect(pool.getStats().available).toBeLessThanOrEqual(3);
        });

        it('should discard invalid objects on release', () => {
            const pool = new ObjectPool<TestObject>({
                factory: () => ({ name: '', value: 0, data: null }),
                reset: (obj) => {
                    obj.name = '';
                    obj.value = 0;
                    obj.data = null;
                },
                validate: (obj) => typeof obj.name === 'string',
                initialSize: 0,
            });

            const obj = pool.acquire();
            (obj as any).name = 123; // Corrupt the object

            pool.release(obj);

            expect(pool.getStats().discarded).toBe(1);
            expect(pool.getStats().available).toBe(0); // Not added to pool
        });
    });

    describe('Statistics', () => {
        it('should track created/reused counts', () => {
            const pool = createTestPool({ initialSize: 2 });

            // First 2 acquires from pre-warmed pool
            pool.acquire();
            pool.acquire();

            // 3rd acquire creates new
            pool.acquire();

            const stats = pool.getStats();
            expect(stats.created).toBe(3); // 2 initial + 1 new
            expect(stats.reused).toBe(2); // 2 from pool
        });

        it('should calculate reuse ratio', () => {
            const pool = createTestPool({ initialSize: 0 });

            // Create 2, reuse 2
            const obj1 = pool.acquire(); // create
            const obj2 = pool.acquire(); // create
            pool.release(obj1);
            pool.release(obj2);
            pool.acquire(); // reuse
            pool.acquire(); // reuse

            const stats = pool.getStats();
            expect(stats.reuseRatio).toBe(0.5); // 2 reused / (2 created + 2 reused)
        });

        it('should track discarded count', () => {
            const pool = new ObjectPool<TestObject>({
                factory: () => ({ name: '', value: 0, data: null }),
                reset: (obj) => {
                    obj.name = '';
                    obj.value = 0;
                    obj.data = null;
                },
                validate: () => false, // Always invalid
                initialSize: 0,
            });

            const obj = pool.acquire(); // Creates new (no validation on empty pool)
            pool.release(obj); // Discards (invalid)

            expect(pool.getStats().discarded).toBe(1);
        });

        it('should track peak usage', () => {
            const pool = createTestPool({ initialSize: 10 });

            const objs = [];
            for (let i = 0; i < 5; i++) {
                objs.push(pool.acquire());
            }

            expect(pool.getStats().peakUsage).toBe(5);

            for (const obj of objs) {
                pool.release(obj);
            }

            // Peak remains at 5
            expect(pool.getStats().peakUsage).toBe(5);
        });

        it('should reset stats correctly', () => {
            const pool = createTestPool({ initialSize: 4 });

            pool.acquire();
            pool.acquire();

            pool.resetStats();

            const stats = pool.getStats();
            expect(stats.created).toBe(0);
            expect(stats.reused).toBe(0);
            expect(stats.peakUsage).toBe(2); // Current inUse
        });
    });

    describe('Edge Cases', () => {
        it('should handle double release (idempotent)', () => {
            const pool = createTestPool({ initialSize: 2 });
            const obj = pool.acquire();

            pool.release(obj);
            pool.release(obj); // Should be no-op

            expect(pool.getStats().available).toBe(2);
            expect(pool.getStats().inUse).toBe(0);
        });

        it('should handle empty pool', () => {
            const pool = createTestPool({ initialSize: 0 });

            const obj = pool.acquire();
            expect(obj).toBeDefined();
            expect(pool.getStats().created).toBe(1);
        });

        it('should clear all objects', () => {
            const pool = createTestPool({ initialSize: 10 });
            pool.clear();

            expect(pool.getStats().available).toBe(0);
        });

        it('should prewarm pool', () => {
            const pool = createTestPool({ initialSize: 0 });
            expect(pool.getStats().available).toBe(0);

            pool.prewarm(5);
            expect(pool.getStats().available).toBe(5);
        });
    });

    describe('Performance', () => {
        it('should achieve high reuse ratio under sustained load', () => {
            const pool = createTestPool({ initialSize: 10, maxSize: 50 });

            for (let i = 0; i < 1000; i++) {
                const obj = pool.acquire();
                obj.name = `test-${i}`;
                pool.release(obj);
            }

            expect(pool.getStats().reuseRatio).toBeGreaterThan(0.8);
        });
    });
});

describe('MessagePool', () => {
    afterEach(() => {
        setGlobalMessagePool(null);
    });

    it('should create message objects', () => {
        const pool = createMessagePool();
        const msg = pool.acquire();

        expect(msg).toEqual({
            type: '',
            payload: null,
            timestamp: null,
            clientId: null,
            mapName: null,
            key: null,
        });
    });

    it('should reset all fields on release', () => {
        const pool = createMessagePool();
        const msg = pool.acquire();

        msg.type = 'SET';
        msg.payload = { key: 'value' };
        msg.timestamp = Date.now();
        msg.clientId = 'client-1';
        msg.mapName = 'myMap';
        msg.key = 'key1';

        pool.release(msg);

        const msg2 = pool.acquire();
        expect(msg2.type).toBe('');
        expect(msg2.payload).toBeNull();
        expect(msg2.timestamp).toBeNull();
        expect(msg2.clientId).toBeNull();
        expect(msg2.mapName).toBeNull();
        expect(msg2.key).toBeNull();
    });

    it('should handle high throughput', () => {
        const pool = createMessagePool({ maxSize: 100 });

        for (let i = 0; i < 1000; i++) {
            const msg = pool.acquire();
            msg.type = 'SET';
            msg.payload = { i };
            pool.release(msg);
        }

        expect(pool.getStats().reuseRatio).toBeGreaterThan(0.8);
    });

    it('should use global singleton', () => {
        const pool1 = getGlobalMessagePool();
        const pool2 = getGlobalMessagePool();
        expect(pool1).toBe(pool2);
    });
});

describe('TimestampPool', () => {
    afterEach(() => {
        setGlobalTimestampPool(null);
    });

    it('should create timestamp objects', () => {
        const pool = createTimestampPool();
        const ts = pool.acquire();

        expect(ts).toEqual({
            millis: 0,
            counter: 0,
            nodeId: '',
        });
    });

    it('should validate timestamp structure', () => {
        const pool = createTimestampPool();
        const ts = pool.acquire();

        // Corrupt the timestamp
        (ts as any).millis = 'not a number';

        pool.release(ts);

        // Should be discarded
        expect(pool.getStats().discarded).toBe(1);
    });

    it('should reset timestamp fields', () => {
        const pool = createTimestampPool();
        const ts = pool.acquire();

        ts.millis = Date.now();
        ts.counter = 42;
        ts.nodeId = 'node-1';

        pool.release(ts);

        const ts2 = pool.acquire();
        expect(ts2.millis).toBe(0);
        expect(ts2.counter).toBe(0);
        expect(ts2.nodeId).toBe('');
    });

    it('should use global singleton', () => {
        const pool1 = getGlobalTimestampPool();
        const pool2 = getGlobalTimestampPool();
        expect(pool1).toBe(pool2);
    });
});

describe('RecordPool', () => {
    it('should create record objects', () => {
        const pool = createRecordPool();
        const rec = pool.acquire();

        expect(rec).toEqual({
            value: null,
            timestamp: null,
            ttlMs: undefined,
        });
    });

    it('should reset record fields', () => {
        const pool = createRecordPool<string>();
        const rec = pool.acquire();

        rec.value = 'test value';
        rec.timestamp = { millis: 1000, counter: 1, nodeId: 'node' };
        rec.ttlMs = 5000;

        pool.release(rec);

        const rec2 = pool.acquire();
        expect(rec2.value).toBeNull();
        expect(rec2.timestamp).toBeNull();
        expect(rec2.ttlMs).toBeUndefined();
    });
});

describe('EventPayloadPool', () => {
    it('should create event payload objects', () => {
        const pool = createEventPayloadPool();
        const payload = pool.acquire();

        expect(payload).toEqual({
            mapName: '',
            key: '',
            eventType: '',
            record: null,
            orRecord: null,
            orTag: null,
        });
    });

    it('should reset event payload fields', () => {
        const pool = createEventPayloadPool();
        const payload = pool.acquire();

        payload.mapName = 'users';
        payload.key = 'user-1';
        payload.eventType = 'PUT';
        payload.record = { value: 'data', timestamp: null };

        pool.release(payload);

        const payload2 = pool.acquire();
        expect(payload2.mapName).toBe('');
        expect(payload2.key).toBe('');
        expect(payload2.eventType).toBe('');
        expect(payload2.record).toBeNull();
    });
});
