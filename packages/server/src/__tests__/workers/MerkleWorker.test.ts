/**
 * MerkleWorker Tests
 * Phase 1.03: MerkleWorker Implementation
 */

import { MerkleTree, LWWMap, HLC } from '@topgunbuild/core';
import {
  WorkerPool,
  MerkleWorker,
  MerkleHashPayload,
  MerkleDiffPayload,
  MerkleRebuildPayload,
} from '../../workers';

// Helper to wait
const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

// Helper to create timestamp
function createTimestamp(millis: number, counter: number, nodeId: string) {
  return { millis, counter, nodeId };
}

describe('MerkleWorker', () => {
  let pool: WorkerPool;
  let merkleWorker: MerkleWorker;

  beforeAll(async () => {
    pool = new WorkerPool({
      minWorkers: 1,
      maxWorkers: 2,
      taskTimeout: 10000,
    });
    merkleWorker = new MerkleWorker(pool);
    // Wait for workers to initialize
    await wait(200);
  });

  afterAll(async () => {
    await pool.shutdown(5000);
  });

  describe('computeHashes', () => {
    it('should compute hashes for small batches (inline)', async () => {
      const payload: MerkleHashPayload = {
        entries: [
          { key: 'key1', timestamp: createTimestamp(1000, 0, 'node1') },
          { key: 'key2', timestamp: createTimestamp(1001, 0, 'node1') },
          { key: 'key3', timestamp: createTimestamp(1002, 0, 'node1') },
        ],
      };

      const result = await merkleWorker.computeHashes(payload);

      expect(result.hashes).toHaveLength(3);
      expect(result.rootHash).toBeGreaterThan(0);
      expect(result.buckets.length).toBeGreaterThan(0);
    });

    it('should compute same hashes as core MerkleTree', async () => {
      const hlc = new HLC('test-node');
      const lwwMap = new LWWMap(hlc);

      // Set some values
      lwwMap.set('user:1', { name: 'Alice' });
      lwwMap.set('user:2', { name: 'Bob' });
      lwwMap.set('user:3', { name: 'Charlie' });

      const coreTree = lwwMap.getMerkleTree();
      const coreRootHash = coreTree.getRootHash();

      // Get records and create payload
      const entries: MerkleHashPayload['entries'] = [];
      for (const key of lwwMap.allKeys()) {
        const record = lwwMap.getRecord(key as string);
        if (record) {
          entries.push({ key: key as string, timestamp: record.timestamp });
        }
      }

      const result = await merkleWorker.computeHashes({ entries });

      // Root hashes should match
      expect(result.rootHash).toBe(coreRootHash);
    });

    it('should handle empty input', async () => {
      const payload: MerkleHashPayload = {
        entries: [],
      };

      const result = await merkleWorker.computeHashes(payload);

      expect(result.hashes).toHaveLength(0);
      expect(result.rootHash).toBe(0);
      expect(result.buckets).toHaveLength(0);
    });

    // Skip worker thread tests in Jest (ts-node doesn't support workers with .ts files)
    // These tests pass after compilation to .js
    it.skip('should handle large batches (worker thread)', async () => {
      // Create 100 entries to exceed threshold
      const entries: MerkleHashPayload['entries'] = [];
      for (let i = 0; i < 100; i++) {
        entries.push({
          key: `key-${i}`,
          timestamp: createTimestamp(1000 + i, 0, 'node1'),
        });
      }

      const result = await merkleWorker.computeHashes({ entries });

      expect(result.hashes).toHaveLength(100);
      expect(result.rootHash).toBeGreaterThan(0);
    }, 15000);
  });

  describe('diff', () => {
    it('should find missing keys', async () => {
      const payload: MerkleDiffPayload = {
        localBuckets: [
          ['abc', { hash: 12345, keys: ['key1', 'key2'] }],
        ],
        remoteBuckets: [
          // Different hash triggers key comparison
          ['abc', { hash: 99999, keys: ['key1', 'key2', 'key3'] }],
        ],
      };

      const result = await merkleWorker.diff(payload);

      expect(result.missingLocal).toContain('key3');
      expect(result.differingPaths).toContain('abc');
    });

    it('should find keys missing on remote', async () => {
      const payload: MerkleDiffPayload = {
        localBuckets: [
          ['abc', { hash: 12345, keys: ['key1', 'key2', 'key3'] }],
        ],
        remoteBuckets: [
          ['abc', { hash: 99999, keys: ['key1'] }],
        ],
      };

      const result = await merkleWorker.diff(payload);

      expect(result.missingRemote).toContain('key2');
      expect(result.missingRemote).toContain('key3');
    });

    it('should detect differing paths', async () => {
      const payload: MerkleDiffPayload = {
        localBuckets: [
          ['abc', { hash: 11111, keys: ['key1'] }],
        ],
        remoteBuckets: [
          ['abc', { hash: 22222, keys: ['key1'] }],
        ],
      };

      const result = await merkleWorker.diff(payload);

      expect(result.differingPaths).toContain('abc');
    });

    it('should handle entirely new buckets', async () => {
      const payload: MerkleDiffPayload = {
        localBuckets: [],
        remoteBuckets: [
          ['xyz', { hash: 12345, keys: ['new-key'] }],
        ],
      };

      const result = await merkleWorker.diff(payload);

      expect(result.missingLocal).toContain('new-key');
    });

    it('should handle buckets missing on remote', async () => {
      const payload: MerkleDiffPayload = {
        localBuckets: [
          ['xyz', { hash: 12345, keys: ['local-key'] }],
        ],
        remoteBuckets: [],
      };

      const result = await merkleWorker.diff(payload);

      expect(result.missingRemote).toContain('local-key');
    });
  });

  describe('rebuild', () => {
    it('should rebuild tree from records', async () => {
      const payload: MerkleRebuildPayload = {
        records: [
          { key: 'a', timestamp: createTimestamp(1000, 0, 'node1') },
          { key: 'b', timestamp: createTimestamp(1001, 0, 'node1') },
          { key: 'c', timestamp: createTimestamp(1002, 0, 'node1') },
        ],
      };

      const result = await merkleWorker.rebuild(payload);

      expect(result.rootHash).toBeGreaterThan(0);
      expect(result.buckets.length).toBeGreaterThan(0);
    });

    it('should produce same tree as computeHashes', async () => {
      const records = [
        { key: 'x', timestamp: createTimestamp(2000, 0, 'node2') },
        { key: 'y', timestamp: createTimestamp(2001, 0, 'node2') },
        { key: 'z', timestamp: createTimestamp(2002, 0, 'node2') },
      ];

      const hashResult = await merkleWorker.computeHashes({ entries: records });
      const rebuildResult = await merkleWorker.rebuild({ records });

      expect(rebuildResult.rootHash).toBe(hashResult.rootHash);
    });

    // Skip worker thread tests in Jest
    it.skip('should handle large rebuilds (worker thread)', async () => {
      const records: MerkleRebuildPayload['records'] = [];
      for (let i = 0; i < 100; i++) {
        records.push({
          key: `record-${i}`,
          timestamp: createTimestamp(3000 + i, 0, 'node3'),
        });
      }

      const result = await merkleWorker.rebuild({ records });

      expect(result.rootHash).toBeGreaterThan(0);
    }, 15000);
  });

  describe('ORMap hashes', () => {
    it('should compute ORMap entry hashes', async () => {
      const result = await merkleWorker.computeORMapHashes({
        entries: [
          {
            key: 'set:members',
            records: [
              { tag: 'tag1', timestamp: createTimestamp(1000, 0, 'node1') },
              { tag: 'tag2', timestamp: createTimestamp(1001, 0, 'node1') },
            ],
          },
        ],
      });

      expect(result.hashes).toHaveLength(1);
      expect(result.rootHash).toBeGreaterThan(0);
    });

    it('should produce deterministic hash regardless of record order', async () => {
      const result1 = await merkleWorker.computeORMapHashes({
        entries: [
          {
            key: 'key',
            records: [
              { tag: 'a', timestamp: createTimestamp(1000, 0, 'node1') },
              { tag: 'b', timestamp: createTimestamp(1001, 0, 'node1') },
            ],
          },
        ],
      });

      const result2 = await merkleWorker.computeORMapHashes({
        entries: [
          {
            key: 'key',
            records: [
              { tag: 'b', timestamp: createTimestamp(1001, 0, 'node1') },
              { tag: 'a', timestamp: createTimestamp(1000, 0, 'node1') },
            ],
          },
        ],
      });

      // Should be same hash regardless of order
      expect(result1.hashes[0][1]).toBe(result2.hashes[0][1]);
    });
  });

  describe('performance', () => {
    // Skip worker thread tests in Jest
    it.skip('should handle 10,000+ entries', async () => {
      const entries: MerkleHashPayload['entries'] = [];
      for (let i = 0; i < 10000; i++) {
        entries.push({
          key: `perf-key-${i}`,
          timestamp: createTimestamp(Date.now() + i, i % 100, `node-${i % 10}`),
        });
      }

      const start = Date.now();
      const result = await merkleWorker.computeHashes({ entries });
      const duration = Date.now() - start;

      expect(result.hashes).toHaveLength(10000);
      expect(result.rootHash).toBeGreaterThan(0);

      // Should complete in reasonable time (< 5 seconds)
      expect(duration).toBeLessThan(5000);

      console.log(`10,000 entries hashed in ${duration}ms`);
    }, 30000);
  });
});
