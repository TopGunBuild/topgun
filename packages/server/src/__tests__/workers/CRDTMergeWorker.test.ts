/**
 * CRDTMergeWorker Tests
 * Phase 1.04: CRDTMergeWorker Implementation
 */

import {
  WorkerPool,
  CRDTMergeWorker,
  LWWMergePayload,
  LWWMergeResult,
  ORMapMergePayload,
  ORMapMergeResult,
} from '../../workers';

// Helper to wait
const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

// Helper to create timestamp
function createTimestamp(millis: number, counter: number, nodeId: string) {
  return { millis, counter, nodeId };
}

describe('CRDTMergeWorker', () => {
  let pool: WorkerPool;
  let crdtWorker: CRDTMergeWorker;

  beforeAll(async () => {
    pool = new WorkerPool({
      minWorkers: 1,
      maxWorkers: 2,
      taskTimeout: 10000,
    });
    crdtWorker = new CRDTMergeWorker(pool);
    // Wait for workers to initialize
    await wait(200);
  });

  afterAll(async () => {
    await pool.shutdown(5000);
  });

  describe('LWW Merge', () => {
    it('should apply records with newer timestamps', async () => {
      const payload: LWWMergePayload = {
        mapName: 'test-map',
        records: [
          {
            key: 'key1',
            value: { name: 'Alice' },
            timestamp: createTimestamp(2000, 0, 'node1'),
          },
          {
            key: 'key2',
            value: { name: 'Bob' },
            timestamp: createTimestamp(2001, 0, 'node1'),
          },
        ],
        existingState: [
          {
            key: 'key1',
            timestamp: createTimestamp(1000, 0, 'node1'),
          },
        ],
      };

      const result = await crdtWorker.mergeLWW(payload);

      expect(result.toApply).toHaveLength(2);
      expect(result.toApply[0].key).toBe('key1');
      expect(result.toApply[1].key).toBe('key2');
      expect(result.skipped).toBe(0);
    });

    it('should skip records with older timestamps', async () => {
      const payload: LWWMergePayload = {
        mapName: 'test-map',
        records: [
          {
            key: 'key1',
            value: { name: 'Old Alice' },
            timestamp: createTimestamp(500, 0, 'node1'),
          },
        ],
        existingState: [
          {
            key: 'key1',
            timestamp: createTimestamp(1000, 0, 'node1'),
          },
        ],
      };

      const result = await crdtWorker.mergeLWW(payload);

      expect(result.toApply).toHaveLength(0);
      expect(result.skipped).toBe(1);
    });

    it('should resolve conflicts by counter when millis are equal', async () => {
      const payload: LWWMergePayload = {
        mapName: 'test-map',
        records: [
          {
            key: 'key1',
            value: { name: 'Counter wins' },
            timestamp: createTimestamp(1000, 5, 'node1'),
          },
        ],
        existingState: [
          {
            key: 'key1',
            timestamp: createTimestamp(1000, 3, 'node1'),
          },
        ],
      };

      const result = await crdtWorker.mergeLWW(payload);

      expect(result.toApply).toHaveLength(1);
      expect(result.conflicts).toContain('key1');
    });

    it('should resolve conflicts by nodeId when millis and counter are equal', async () => {
      const payload: LWWMergePayload = {
        mapName: 'test-map',
        records: [
          {
            key: 'key1',
            value: { name: 'NodeId wins' },
            timestamp: createTimestamp(1000, 0, 'node2'),
          },
        ],
        existingState: [
          {
            key: 'key1',
            timestamp: createTimestamp(1000, 0, 'node1'),
          },
        ],
      };

      const result = await crdtWorker.mergeLWW(payload);

      // 'node2' > 'node1' lexicographically, so new record wins
      expect(result.toApply).toHaveLength(1);
      expect(result.conflicts).toContain('key1');
    });

    it('should handle empty records', async () => {
      const payload: LWWMergePayload = {
        mapName: 'test-map',
        records: [],
        existingState: [],
      };

      const result = await crdtWorker.mergeLWW(payload);

      expect(result.toApply).toHaveLength(0);
      expect(result.skipped).toBe(0);
      expect(result.conflicts).toHaveLength(0);
    });

    it('should preserve TTL in merge results', async () => {
      const payload: LWWMergePayload = {
        mapName: 'test-map',
        records: [
          {
            key: 'key1',
            value: { name: 'Alice' },
            timestamp: createTimestamp(2000, 0, 'node1'),
            ttlMs: 60000,
          },
        ],
        existingState: [],
      };

      const result = await crdtWorker.mergeLWW(payload);

      expect(result.toApply).toHaveLength(1);
      expect(result.toApply[0].ttlMs).toBe(60000);
    });

    // Skip worker thread tests in Jest
    it.skip('should handle large batches (worker thread)', async () => {
      const records: LWWMergePayload['records'] = [];
      for (let i = 0; i < 100; i++) {
        records.push({
          key: `key-${i}`,
          value: { index: i },
          timestamp: createTimestamp(2000 + i, 0, 'node1'),
        });
      }

      const existingState: LWWMergePayload['existingState'] = [];
      for (let i = 0; i < 50; i++) {
        existingState.push({
          key: `key-${i}`,
          timestamp: createTimestamp(1000, 0, 'node1'),
        });
      }

      const result = await crdtWorker.mergeLWW({
        mapName: 'test-map',
        records,
        existingState,
      });

      expect(result.toApply).toHaveLength(100);
    }, 15000);
  });

  describe('ORMap Merge', () => {
    it('should apply new items', async () => {
      const payload: ORMapMergePayload = {
        mapName: 'test-set',
        items: [
          {
            key: 'members',
            value: 'alice',
            tag: 'tag-alice-1',
            timestamp: createTimestamp(1000, 0, 'node1'),
          },
          {
            key: 'members',
            value: 'bob',
            tag: 'tag-bob-1',
            timestamp: createTimestamp(1001, 0, 'node1'),
          },
        ],
        tombstones: [],
        existingTags: [],
        existingTombstones: [],
      };

      const result = await crdtWorker.mergeORMap(payload);

      expect(result.itemsToApply).toHaveLength(2);
      expect(result.itemsSkipped).toBe(0);
    });

    it('should skip items with existing tags', async () => {
      const payload: ORMapMergePayload = {
        mapName: 'test-set',
        items: [
          {
            key: 'members',
            value: 'alice',
            tag: 'existing-tag',
            timestamp: createTimestamp(1000, 0, 'node1'),
          },
        ],
        tombstones: [],
        existingTags: ['existing-tag'],
        existingTombstones: [],
      };

      const result = await crdtWorker.mergeORMap(payload);

      expect(result.itemsToApply).toHaveLength(0);
      expect(result.itemsSkipped).toBe(1);
    });

    it('should skip items with tombstoned tags', async () => {
      const payload: ORMapMergePayload = {
        mapName: 'test-set',
        items: [
          {
            key: 'members',
            value: 'alice',
            tag: 'tombstoned-tag',
            timestamp: createTimestamp(1000, 0, 'node1'),
          },
        ],
        tombstones: [],
        existingTags: [],
        existingTombstones: ['tombstoned-tag'],
      };

      const result = await crdtWorker.mergeORMap(payload);

      expect(result.itemsToApply).toHaveLength(0);
      expect(result.itemsSkipped).toBe(1);
    });

    it('should apply new tombstones', async () => {
      const payload: ORMapMergePayload = {
        mapName: 'test-set',
        items: [],
        tombstones: [
          {
            tag: 'tag-to-remove',
            timestamp: createTimestamp(2000, 0, 'node1'),
          },
        ],
        existingTags: ['tag-to-remove'],
        existingTombstones: [],
      };

      const result = await crdtWorker.mergeORMap(payload);

      expect(result.tombstonesToApply).toContain('tag-to-remove');
      expect(result.tagsToRemove).toContain('tag-to-remove');
    });

    it('should skip existing tombstones', async () => {
      const payload: ORMapMergePayload = {
        mapName: 'test-set',
        items: [],
        tombstones: [
          {
            tag: 'already-tombstoned',
            timestamp: createTimestamp(2000, 0, 'node1'),
          },
        ],
        existingTags: [],
        existingTombstones: ['already-tombstoned'],
      };

      const result = await crdtWorker.mergeORMap(payload);

      expect(result.tombstonesToApply).toHaveLength(0);
      expect(result.tombstonesSkipped).toBe(1);
    });

    it('should handle concurrent add and remove', async () => {
      // Item added and its tombstone received in same batch
      const payload: ORMapMergePayload = {
        mapName: 'test-set',
        items: [
          {
            key: 'members',
            value: 'concurrent',
            tag: 'concurrent-tag',
            timestamp: createTimestamp(1000, 0, 'node1'),
          },
        ],
        tombstones: [
          {
            tag: 'concurrent-tag',
            timestamp: createTimestamp(1500, 0, 'node2'),
          },
        ],
        existingTags: [],
        existingTombstones: [],
      };

      const result = await crdtWorker.mergeORMap(payload);

      // The tombstone for the same tag should win
      // Item should be skipped because its tag is being tombstoned
      expect(result.tombstonesToApply).toContain('concurrent-tag');
      // Item may or may not be applied depending on implementation
      // What matters is the final state is consistent
    });

    it('should handle empty input', async () => {
      const payload: ORMapMergePayload = {
        mapName: 'test-set',
        items: [],
        tombstones: [],
        existingTags: [],
        existingTombstones: [],
      };

      const result = await crdtWorker.mergeORMap(payload);

      expect(result.itemsToApply).toHaveLength(0);
      expect(result.tombstonesToApply).toHaveLength(0);
      expect(result.tagsToRemove).toHaveLength(0);
    });

    // Skip worker thread tests in Jest
    it.skip('should handle large batches (worker thread)', async () => {
      const items: ORMapMergePayload['items'] = [];
      for (let i = 0; i < 100; i++) {
        items.push({
          key: 'members',
          value: `member-${i}`,
          tag: `tag-${i}`,
          timestamp: createTimestamp(1000 + i, 0, 'node1'),
        });
      }

      const result = await crdtWorker.mergeORMap({
        mapName: 'test-set',
        items,
        tombstones: [],
        existingTags: [],
        existingTombstones: [],
      });

      expect(result.itemsToApply).toHaveLength(100);
    }, 15000);
  });

  describe('Edge Cases', () => {
    it('should handle identical timestamps (LWW tie-breaker)', async () => {
      const payload: LWWMergePayload = {
        mapName: 'test-map',
        records: [
          {
            key: 'key1',
            value: { name: 'New' },
            timestamp: createTimestamp(1000, 0, 'node1'),
          },
        ],
        existingState: [
          {
            key: 'key1',
            timestamp: createTimestamp(1000, 0, 'node1'),
          },
        ],
      };

      const result = await crdtWorker.mergeLWW(payload);

      // Same timestamp - should be skipped (not newer)
      expect(result.toApply).toHaveLength(0);
      expect(result.skipped).toBe(1);
    });

    it('should handle complex nested values', async () => {
      const payload: LWWMergePayload = {
        mapName: 'test-map',
        records: [
          {
            key: 'complex',
            value: {
              nested: {
                array: [1, 2, 3],
                object: { a: 1, b: 2 },
              },
              string: 'test',
            },
            timestamp: createTimestamp(2000, 0, 'node1'),
          },
        ],
        existingState: [],
      };

      const result = await crdtWorker.mergeLWW(payload);

      expect(result.toApply).toHaveLength(1);
      expect(result.toApply[0].value).toEqual({
        nested: {
          array: [1, 2, 3],
          object: { a: 1, b: 2 },
        },
        string: 'test',
      });
    });

    it('should handle null and undefined values', async () => {
      const payload: LWWMergePayload = {
        mapName: 'test-map',
        records: [
          {
            key: 'null-key',
            value: null,
            timestamp: createTimestamp(2000, 0, 'node1'),
          },
          {
            key: 'undef-key',
            value: undefined,
            timestamp: createTimestamp(2001, 0, 'node1'),
          },
        ],
        existingState: [],
      };

      const result = await crdtWorker.mergeLWW(payload);

      expect(result.toApply).toHaveLength(2);
    });
  });
});
