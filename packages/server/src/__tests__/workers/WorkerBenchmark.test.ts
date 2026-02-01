/**
 * Worker Benchmark Tests
 * Performance comparison inline vs worker threads
 *
 * These tests measure the performance characteristics of worker operations.
 * NOTE: Worker thread tests are skipped in Jest due to ts-node limitations.
 * Only inline operations (batch size < 10) are tested here.
 * For full worker thread benchmarks, use compiled .js files.
 */

import { WorkerPool, MerkleWorker, CRDTMergeWorker } from '../../workers';
import type {
  MerkleHashPayload,
  MerkleRebuildPayload,
  LWWMergePayload,
  ORMapMergePayload,
} from '../../workers';

// Helper to wait
const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

// Threshold for inline execution (same as in workers)
const INLINE_THRESHOLD = 10;

// Helper to create timestamp
function createTimestamp(millis: number, counter: number, nodeId: string) {
  return { millis, counter, nodeId };
}

// Benchmark helper
interface BenchmarkResult {
  name: string;
  iterations: number;
  totalMs: number;
  avgMs: number;
  opsPerSec: number;
  itemsProcessed: number;
  itemsPerSec: number;
}

async function benchmark(
  name: string,
  iterations: number,
  itemsPerIteration: number,
  fn: () => Promise<void>
): Promise<BenchmarkResult> {
  // Warmup
  for (let i = 0; i < Math.min(3, iterations); i++) {
    await fn();
  }

  const start = performance.now();
  for (let i = 0; i < iterations; i++) {
    await fn();
  }
  const totalMs = performance.now() - start;

  const avgMs = totalMs / iterations;
  const opsPerSec = (iterations / totalMs) * 1000;
  const itemsProcessed = iterations * itemsPerIteration;
  const itemsPerSec = (itemsProcessed / totalMs) * 1000;

  return {
    name,
    iterations,
    totalMs,
    avgMs,
    opsPerSec,
    itemsProcessed,
    itemsPerSec,
  };
}

function printResults(results: BenchmarkResult[]) {
  console.log('\n=== Benchmark Results ===\n');
  console.log('| Test | Iterations | Total (ms) | Avg (ms) | Ops/sec | Items/sec |');
  console.log('|------|------------|------------|----------|---------|-----------|');
  for (const r of results) {
    console.log(
      `| ${r.name.padEnd(30)} | ${r.iterations.toString().padStart(10)} | ${r.totalMs.toFixed(2).padStart(10)} | ${r.avgMs.toFixed(3).padStart(8)} | ${r.opsPerSec.toFixed(0).padStart(7)} | ${r.itemsPerSec.toFixed(0).padStart(9)} |`
    );
  }
  console.log('\n');
}

describe('Worker Performance Benchmarks', () => {
  let pool: WorkerPool;
  let merkleWorker: MerkleWorker;
  let crdtWorker: CRDTMergeWorker;

  beforeAll(async () => {
    pool = new WorkerPool({
      minWorkers: 2,
      maxWorkers: 4,
      taskTimeout: 30000,
    });
    merkleWorker = new MerkleWorker(pool);
    crdtWorker = new CRDTMergeWorker(pool);
    await wait(300); // Wait for workers to initialize
  });

  afterAll(async () => {
    await pool.shutdown(5000);
  });

  describe('MerkleWorker Benchmarks (Inline)', () => {
    // Test only batch sizes below threshold (inline execution)
    it('should benchmark computeHashes at inline batch sizes', async () => {
      const results: BenchmarkResult[] = [];
      const batchSizes = [1, 3, 5, 7, 9]; // All below INLINE_THRESHOLD

      for (const size of batchSizes) {
        const entries: MerkleHashPayload['entries'] = [];
        for (let i = 0; i < size; i++) {
          entries.push({
            key: `key-${i}`,
            timestamp: createTimestamp(1000 + i, i % 10, `node-${i % 3}`),
          });
        }

        const result = await benchmark(
          `MerkleHash (${size} entries)`,
          500, // Many iterations for accurate timing
          size,
          async () => {
            await merkleWorker.computeHashes({ entries });
          }
        );

        results.push(result);
      }

      printResults(results);

      // Basic sanity checks
      expect(results.length).toBe(batchSizes.length);
      for (const r of results) {
        expect(r.avgMs).toBeGreaterThan(0);
        expect(r.opsPerSec).toBeGreaterThan(0);
      }
    }, 60000);

    it('should benchmark diff operation (inline)', async () => {
      const results: BenchmarkResult[] = [];
      // Keep total keys below threshold * 2 = 20 (inline path)
      // buckets * keys_per_bucket * 2 (local + remote) < 20
      const bucketCounts = [1, 2, 3, 4];

      for (const count of bucketCounts) {
        const localBuckets: Array<[string, { hash: number; keys: string[] }]> = [];
        const remoteBuckets: Array<[string, { hash: number; keys: string[] }]> = [];

        for (let i = 0; i < count; i++) {
          const path = i.toString(16).padStart(3, '0');
          const keys = [`key-${i}`]; // 1 key per bucket to stay under threshold

          localBuckets.push([path, { hash: i * 1000, keys }]);
          // Remote has slightly different data
          if (i % 2 === 0) {
            remoteBuckets.push([path, { hash: i * 1000 + 1, keys: [...keys, `key-${i}-new`] }]);
          } else {
            remoteBuckets.push([path, { hash: i * 1000, keys }]);
          }
        }

        const result = await benchmark(
          `MerkleDiff (${count} buckets)`,
          500,
          count,
          async () => {
            await merkleWorker.diff({ localBuckets, remoteBuckets });
          }
        );

        results.push(result);
      }

      printResults(results);

      expect(results.length).toBe(bucketCounts.length);
    }, 60000);

    it('should benchmark rebuild operation (inline)', async () => {
      const results: BenchmarkResult[] = [];
      const recordCounts = [1, 3, 5, 7, 9]; // Below threshold

      for (const count of recordCounts) {
        const records: MerkleRebuildPayload['records'] = [];
        for (let i = 0; i < count; i++) {
          records.push({
            key: `record-${i}`,
            timestamp: createTimestamp(2000 + i, i % 5, `node-${i % 2}`),
          });
        }

        const result = await benchmark(
          `MerkleRebuild (${count} records)`,
          500,
          count,
          async () => {
            await merkleWorker.rebuild({ records });
          }
        );

        results.push(result);
      }

      printResults(results);

      expect(results.length).toBe(recordCounts.length);
    }, 60000);
  });

  describe('CRDTMergeWorker Benchmarks (Inline)', () => {
    it('should benchmark LWW merge at inline batch sizes', async () => {
      const results: BenchmarkResult[] = [];
      const batchSizes = [1, 3, 5, 7, 9]; // Below threshold

      for (const size of batchSizes) {
        const records: LWWMergePayload['records'] = [];
        const existingState: LWWMergePayload['existingState'] = [];

        for (let i = 0; i < size; i++) {
          records.push({
            key: `key-${i}`,
            value: { data: `value-${i}`, index: i },
            timestamp: createTimestamp(2000 + i, i % 10, 'incoming-node'),
          });

          // 50% have existing state
          if (i % 2 === 0) {
            existingState.push({
              key: `key-${i}`,
              timestamp: createTimestamp(1000 + i, 0, 'local-node'),
            });
          }
        }

        const result = await benchmark(
          `LWWMerge (${size} records)`,
          500,
          size,
          async () => {
            await crdtWorker.mergeLWW({
              mapName: 'test-map',
              records,
              existingState,
            });
          }
        );

        results.push(result);
      }

      printResults(results);

      expect(results.length).toBe(batchSizes.length);
    }, 60000);

    it('should benchmark ORMap merge at inline batch sizes', async () => {
      const results: BenchmarkResult[] = [];
      const batchSizes = [1, 3, 5, 7, 9]; // Below threshold

      for (const size of batchSizes) {
        const items: ORMapMergePayload['items'] = [];
        const tombstones: ORMapMergePayload['tombstones'] = [];
        const existingTags: string[] = [];
        const existingTombstones: string[] = [];

        for (let i = 0; i < size; i++) {
          items.push({
            key: 'members',
            value: `member-${i}`,
            tag: `tag-${i}`,
            timestamp: createTimestamp(1000 + i, 0, 'node1'),
          });

          // 20% have existing tags
          if (i % 5 === 0) {
            existingTags.push(`existing-tag-${i}`);
          }
        }

        const result = await benchmark(
          `ORMapMerge (${size} items)`,
          500,
          size,
          async () => {
            await crdtWorker.mergeORMap({
              mapName: 'test-set',
              items,
              tombstones,
              existingTags,
              existingTombstones,
            });
          }
        );

        results.push(result);
      }

      printResults(results);

      expect(results.length).toBe(batchSizes.length);
    }, 60000);
  });

  describe('Scaling Analysis (Inline)', () => {
    it('should show linear scaling for inline operations', async () => {
      // Test scaling behavior within inline threshold
      const testSizes = [1, 2, 3, 5, 7, 9];
      const results: BenchmarkResult[] = [];

      for (const size of testSizes) {
        const entries: MerkleHashPayload['entries'] = [];
        for (let i = 0; i < size; i++) {
          entries.push({
            key: `key-${i}`,
            timestamp: createTimestamp(1000 + i, 0, 'node1'),
          });
        }

        const result = await benchmark(
          `MerkleHash@${size}`,
          500,
          size,
          async () => {
            await merkleWorker.computeHashes({ entries });
          }
        );

        results.push(result);
      }

      console.log('\n=== Inline Scaling Analysis: MerkleHash ===');
      console.log('Size | Avg (ms) | Items/sec | Time per item (μs)');
      console.log('-----|----------|-----------|-------------------');
      for (let i = 0; i < results.length; i++) {
        const r = results[i];
        const timePerItem = (r.avgMs * 1000) / testSizes[i]; // microseconds
        console.log(
          `${testSizes[i].toString().padStart(4)} | ${r.avgMs.toFixed(3).padStart(8)} | ${r.itemsPerSec.toFixed(0).padStart(9)} | ${timePerItem.toFixed(2).padStart(17)}`
        );
      }

      expect(results.length).toBe(testSizes.length);
    }, 60000);

    it('should show LWW merge scaling', async () => {
      const testSizes = [1, 2, 3, 5, 7, 9];
      const results: BenchmarkResult[] = [];

      for (const size of testSizes) {
        const records: LWWMergePayload['records'] = [];
        const existingState: LWWMergePayload['existingState'] = [];

        for (let i = 0; i < size; i++) {
          records.push({
            key: `key-${i}`,
            value: { index: i },
            timestamp: createTimestamp(2000 + i, 0, 'node1'),
          });
          if (i % 2 === 0) {
            existingState.push({
              key: `key-${i}`,
              timestamp: createTimestamp(1000, 0, 'node2'),
            });
          }
        }

        const result = await benchmark(
          `LWWMerge@${size}`,
          500,
          size,
          async () => {
            await crdtWorker.mergeLWW({
              mapName: 'test',
              records,
              existingState,
            });
          }
        );

        results.push(result);
      }

      console.log('\n=== Inline Scaling Analysis: LWWMerge ===');
      console.log('Size | Avg (ms) | Items/sec | Time per item (μs)');
      console.log('-----|----------|-----------|-------------------');
      for (let i = 0; i < results.length; i++) {
        const r = results[i];
        const timePerItem = (r.avgMs * 1000) / testSizes[i];
        console.log(
          `${testSizes[i].toString().padStart(4)} | ${r.avgMs.toFixed(3).padStart(8)} | ${r.itemsPerSec.toFixed(0).padStart(9)} | ${timePerItem.toFixed(2).padStart(17)}`
        );
      }

      expect(results.length).toBe(testSizes.length);
    }, 60000);
  });

  describe('Throughput Summary', () => {
    it('should summarize overall throughput metrics', async () => {
      console.log('\n=== THROUGHPUT SUMMARY ===\n');

      // MerkleHash throughput at typical batch size
      const merkleEntries: MerkleHashPayload['entries'] = [];
      for (let i = 0; i < 5; i++) {
        merkleEntries.push({
          key: `key-${i}`,
          timestamp: createTimestamp(1000 + i, 0, 'node1'),
        });
      }

      const merkleResult = await benchmark('MerkleHash@5', 1000, 5, async () => {
        await merkleWorker.computeHashes({ entries: merkleEntries });
      });

      // LWW Merge throughput
      const lwwRecords: LWWMergePayload['records'] = [];
      const lwwExisting: LWWMergePayload['existingState'] = [];
      for (let i = 0; i < 5; i++) {
        lwwRecords.push({
          key: `key-${i}`,
          value: { data: i },
          timestamp: createTimestamp(2000, 0, 'node1'),
        });
        if (i % 2 === 0) {
          lwwExisting.push({ key: `key-${i}`, timestamp: createTimestamp(1000, 0, 'node2') });
        }
      }

      const lwwResult = await benchmark('LWWMerge@5', 1000, 5, async () => {
        await crdtWorker.mergeLWW({ mapName: 'test', records: lwwRecords, existingState: lwwExisting });
      });

      // ORMap Merge throughput
      const ormapItems: ORMapMergePayload['items'] = [];
      for (let i = 0; i < 5; i++) {
        ormapItems.push({
          key: 'set',
          value: `item-${i}`,
          tag: `tag-${i}`,
          timestamp: createTimestamp(1000, 0, 'node1'),
        });
      }

      const ormapResult = await benchmark('ORMapMerge@5', 1000, 5, async () => {
        await crdtWorker.mergeORMap({
          mapName: 'test',
          items: ormapItems,
          tombstones: [],
          existingTags: [],
          existingTombstones: [],
        });
      });

      console.log('| Operation     | Ops/sec   | Items/sec   | Avg latency |');
      console.log('|---------------|-----------|-------------|-------------|');
      console.log(`| MerkleHash    | ${merkleResult.opsPerSec.toFixed(0).padStart(9)} | ${merkleResult.itemsPerSec.toFixed(0).padStart(11)} | ${(merkleResult.avgMs * 1000).toFixed(0).padStart(8)} μs |`);
      console.log(`| LWWMerge      | ${lwwResult.opsPerSec.toFixed(0).padStart(9)} | ${lwwResult.itemsPerSec.toFixed(0).padStart(11)} | ${(lwwResult.avgMs * 1000).toFixed(0).padStart(8)} μs |`);
      console.log(`| ORMapMerge    | ${ormapResult.opsPerSec.toFixed(0).padStart(9)} | ${ormapResult.itemsPerSec.toFixed(0).padStart(11)} | ${(ormapResult.avgMs * 1000).toFixed(0).padStart(8)} μs |`);
      console.log('\n');

      expect(merkleResult.opsPerSec).toBeGreaterThan(1000);
      expect(lwwResult.opsPerSec).toBeGreaterThan(1000);
      expect(ormapResult.opsPerSec).toBeGreaterThan(1000);
    }, 60000);
  });
});
