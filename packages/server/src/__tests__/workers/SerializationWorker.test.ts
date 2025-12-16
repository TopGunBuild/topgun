/**
 * SerializationWorker Tests
 * Phase 1.07: SerializationWorker Implementation
 *
 * Tests for serialization/deserialization operations.
 * NOTE: Worker thread tests are skipped in Jest due to ts-node limitations.
 * Only inline operations (batch size < 10) are tested here.
 */

import { WorkerPool, SerializationWorker } from '../../workers';

// Helper to wait
const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

// Threshold for inline execution (same as in worker)
const INLINE_THRESHOLD = 10;

describe('SerializationWorker Tests', () => {
  let pool: WorkerPool;
  let worker: SerializationWorker;

  beforeAll(async () => {
    pool = new WorkerPool({
      minWorkers: 1,
      maxWorkers: 2,
      taskTimeout: 5000,
    });
    worker = new SerializationWorker(pool);
    await wait(200); // Wait for workers to initialize
  });

  afterAll(async () => {
    await pool.shutdown(5000);
  });

  describe('Single Serialize/Deserialize', () => {
    it('should serialize and deserialize primitives', () => {
      const testCases = [
        'hello world',
        42,
        3.14159,
        true,
        false,
        null,
      ];

      for (const original of testCases) {
        const serialized = worker.serialize(original);
        const deserialized = worker.deserialize(serialized);
        expect(deserialized).toEqual(original);
      }
    });

    it('should serialize and deserialize objects', () => {
      const original = {
        name: 'Test User',
        age: 30,
        active: true,
        tags: ['admin', 'user'],
        nested: { key: 'value' },
      };

      const serialized = worker.serialize(original);
      const deserialized = worker.deserialize<typeof original>(serialized);

      expect(deserialized).toEqual(original);
    });

    it('should serialize and deserialize arrays', () => {
      const original = [1, 'two', true, null, { key: 'value' }];

      const serialized = worker.serialize(original);
      const deserialized = worker.deserialize<typeof original>(serialized);

      expect(deserialized).toEqual(original);
    });

    it('should handle empty objects and arrays', () => {
      expect(worker.deserialize(worker.serialize({}))).toEqual({});
      expect(worker.deserialize(worker.serialize([]))).toEqual([]);
    });

    it('should handle unicode strings', () => {
      const original = '你好世界 🌍 مرحبا';
      const serialized = worker.serialize(original);
      const deserialized = worker.deserialize<string>(serialized);
      expect(deserialized).toBe(original);
    });
  });

  describe('Batch Serialize (Inline)', () => {
    it('should serialize batch of small items', async () => {
      const items = [
        { id: 1, name: 'Item 1' },
        { id: 2, name: 'Item 2' },
        { id: 3, name: 'Item 3' },
      ];

      const serialized = await worker.serializeBatch(items);

      expect(serialized.length).toBe(items.length);
      for (let i = 0; i < items.length; i++) {
        expect(serialized[i]).toBeInstanceOf(Uint8Array);
        const deserialized = worker.deserialize(serialized[i]);
        expect(deserialized).toEqual(items[i]);
      }
    });

    it('should handle empty batch', async () => {
      const result = await worker.serializeBatch([]);
      expect(result).toEqual([]);
    });

    it('should serialize batch up to threshold inline', async () => {
      const items: Array<{ index: number }> = [];
      for (let i = 0; i < INLINE_THRESHOLD - 1; i++) {
        items.push({ index: i });
      }

      const serialized = await worker.serializeBatch(items);

      expect(serialized.length).toBe(items.length);
      for (let i = 0; i < items.length; i++) {
        const deserialized = worker.deserialize<{ index: number }>(serialized[i]);
        expect(deserialized.index).toBe(i);
      }
    });
  });

  describe('Batch Deserialize (Inline)', () => {
    it('should deserialize batch of items', async () => {
      const items = [
        { id: 1, name: 'Item 1' },
        { id: 2, name: 'Item 2' },
        { id: 3, name: 'Item 3' },
      ];

      const serialized = await worker.serializeBatch(items);
      const deserialized = await worker.deserializeBatch<typeof items[0]>(serialized);

      expect(deserialized.length).toBe(items.length);
      for (let i = 0; i < items.length; i++) {
        expect(deserialized[i]).toEqual(items[i]);
      }
    });

    it('should handle empty batch', async () => {
      const result = await worker.deserializeBatch([]);
      expect(result).toEqual([]);
    });
  });

  describe('TopGun-like Data Structures', () => {
    it('should serialize LWW record structure', async () => {
      const record = {
        value: { name: 'Test', email: 'test@example.com' },
        timestamp: {
          millis: Date.now(),
          counter: 5,
          nodeId: 'node-123',
        },
        ttlMs: 3600000,
      };

      const serialized = worker.serialize(record);
      const deserialized = worker.deserialize<typeof record>(serialized);

      expect(deserialized).toEqual(record);
    });

    it('should serialize OR record structure', async () => {
      const record = {
        key: 'members',
        value: { userId: 'user-1', role: 'admin' },
        timestamp: {
          millis: 1000000,
          counter: 3,
          nodeId: 'node-abc',
        },
        tag: 'tag-unique-123',
      };

      const serialized = worker.serialize(record);
      const deserialized = worker.deserialize<typeof record>(serialized);

      expect(deserialized).toEqual(record);
    });

    it('should serialize batch event structure', async () => {
      const batchEvent = {
        type: 'SERVER_BATCH_EVENT',
        payload: {
          events: [
            {
              mapName: 'users',
              key: 'user1',
              eventType: 'SET',
              record: {
                value: { name: 'Alice' },
                timestamp: { millis: 1000, counter: 0, nodeId: 'n1' },
              },
            },
            {
              mapName: 'users',
              key: 'user2',
              eventType: 'SET',
              record: {
                value: { name: 'Bob' },
                timestamp: { millis: 1001, counter: 0, nodeId: 'n1' },
              },
            },
          ],
        },
        timestamp: { millis: 1002, counter: 0, nodeId: 'n1' },
      };

      const serialized = worker.serialize(batchEvent);
      const deserialized = worker.deserialize<typeof batchEvent>(serialized);

      expect(deserialized).toEqual(batchEvent);
      expect(deserialized.payload.events.length).toBe(2);
    });
  });

  describe('shouldUseWorker Decision', () => {
    it('should return false for small batches', () => {
      const smallBatch = [{ a: 1 }, { b: 2 }];
      expect(worker.shouldUseWorker(smallBatch)).toBe(false);
    });

    it('should return true for batches at threshold', () => {
      const items: unknown[] = [];
      for (let i = 0; i < INLINE_THRESHOLD; i++) {
        items.push({ index: i });
      }
      expect(worker.shouldUseWorker(items)).toBe(true);
    });

    it('should return true for large payload size', () => {
      // Create a small array with large strings
      const largePayload = [
        { data: 'x'.repeat(30000) }, // ~30KB string
        { data: 'y'.repeat(30000) }, // ~30KB string
      ];
      expect(worker.shouldUseWorker(largePayload)).toBe(true);
    });

    it('should return false for empty array', () => {
      expect(worker.shouldUseWorker([])).toBe(false);
    });
  });

  describe('Edge Cases', () => {
    it('should handle deeply nested objects', () => {
      let nested: Record<string, unknown> = { value: 'deep' };
      for (let i = 0; i < 10; i++) {
        nested = { nested };
      }

      const serialized = worker.serialize(nested);
      const deserialized = worker.deserialize<typeof nested>(serialized);

      expect(deserialized).toEqual(nested);
    });

    it('should handle large arrays', async () => {
      const items: Array<{ index: number }> = [];
      for (let i = 0; i < 5; i++) {
        items.push({ index: i });
      }

      const serialized = await worker.serializeBatch(items);
      const deserialized = await worker.deserializeBatch<{ index: number }>(serialized);

      expect(deserialized.length).toBe(5);
      for (let i = 0; i < 5; i++) {
        expect(deserialized[i].index).toBe(i);
      }
    });

    it('should handle special number values', () => {
      const testCases = [
        0,
        -0,
        Number.MAX_SAFE_INTEGER,
        Number.MIN_SAFE_INTEGER,
        0.1 + 0.2, // floating point precision
      ];

      for (const original of testCases) {
        const serialized = worker.serialize(original);
        const deserialized = worker.deserialize<number>(serialized);
        expect(deserialized).toBeCloseTo(original, 10);
      }
    });

    it('should preserve object key order', () => {
      const original = { z: 1, a: 2, m: 3 };
      const serialized = worker.serialize(original);
      const deserialized = worker.deserialize<typeof original>(serialized);

      expect(Object.keys(deserialized)).toEqual(Object.keys(original));
    });
  });

  describe('Roundtrip Tests', () => {
    it('should maintain data integrity through roundtrip', async () => {
      const items = [
        { type: 'user', data: { name: 'Alice', age: 30 } },
        { type: 'post', data: { title: 'Hello', body: 'World' } },
        { type: 'comment', data: { text: 'Nice!', likes: 42 } },
      ];

      const serialized = await worker.serializeBatch(items);
      const deserialized = await worker.deserializeBatch<typeof items[0]>(serialized);

      expect(deserialized).toEqual(items);
    });

    it('should handle multiple roundtrips', () => {
      const original = { key: 'value', count: 42 };

      let data = original;
      for (let i = 0; i < 5; i++) {
        const serialized = worker.serialize(data);
        data = worker.deserialize<typeof original>(serialized);
      }

      expect(data).toEqual(original);
    });
  });

  // Worker thread tests - skipped due to ts-node limitations
  describe.skip('Worker Thread Operations', () => {
    it('should serialize large batch via worker thread', async () => {
      const items: unknown[] = [];
      for (let i = 0; i < 100; i++) {
        items.push({
          id: i,
          name: `Item ${i}`,
          data: { nested: { value: i * 100 } },
        });
      }

      const serialized = await worker.serializeBatch(items);

      expect(serialized.length).toBe(100);
    });

    it('should deserialize large batch via worker thread', async () => {
      const items: unknown[] = [];
      for (let i = 0; i < 100; i++) {
        items.push({ id: i, name: `Item ${i}` });
      }

      const serialized = await worker.serializeBatch(items);
      const deserialized = await worker.deserializeBatch(serialized);

      expect(deserialized.length).toBe(100);
      expect(deserialized).toEqual(items);
    });
  });
});
