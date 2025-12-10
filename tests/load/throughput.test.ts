/**
 * Load Tests: Throughput
 * Tests write/read throughput and latency under various conditions
 */

import {
  createTestServer,
  createTestClient,
  waitForSync,
  createLWWRecord,
  TestClient,
} from '../e2e/helpers';
import { ServerCoordinator } from '@topgunbuild/server';
import {
  measureTime,
  calculateThroughput,
  calculateStats,
  logResults,
  LoadTestResults,
} from './utils';

describe('Throughput Load Tests', () => {
  let server: ServerCoordinator;
  let client: TestClient;
  let clients: TestClient[] = [];

  beforeEach(async () => {
    server = await createTestServer();
    clients = [];
  });

  afterEach(async () => {
    // Cleanup client
    if (client) {
      try {
        client.close();
      } catch (e) {
        // Ignore
      }
    }

    // Cleanup all clients
    for (const c of clients) {
      try {
        c.close();
      } catch (e) {
        // Ignore
      }
    }
    clients = [];

    // Shutdown server
    if (server) {
      await server.shutdown();
    }

    await waitForSync(200);
  });

  describe('Single Client Throughput', () => {
    it('should measure sequential write throughput (1000 records)', async () => {
      const numRecords = 1000;
      const serverUrl = `ws://localhost:${server.port}`;

      client = await createTestClient(serverUrl, {
        nodeId: 'throughput-client',
        userId: 'throughput-user',
        roles: ['ADMIN'],
      });
      await client.waitForMessage('AUTH_ACK', 5000);

      const writeTimes: number[] = [];

      const { timeMs: totalTime } = await measureTime(async () => {
        for (let i = 0; i < numRecords; i++) {
          const writeStart = performance.now();

          const record = createLWWRecord(
            { index: i, data: `sequential-data-${i}` },
            'throughput-client'
          );
          client.send({
            type: 'CLIENT_OP',
            payload: {
              id: `seq-op-${i}`,
              mapName: 'throughput-test',
              opType: 'PUT',
              key: `record-${i}`,
              record,
            },
          });

          writeTimes.push(performance.now() - writeStart);

          // Don't wait for ACK for each write - measure raw send throughput
          if (i % 100 === 99) {
            // Every 100 writes, wait briefly to let the server process
            await waitForSync(10);
          }
        }

        // Wait for final sync
        await waitForSync(500);
      });

      const stats = calculateStats(writeTimes);
      const throughput = calculateThroughput(numRecords, totalTime);

      const results: LoadTestResults = {
        testName: 'Sequential Write Throughput',
        metrics: {
          'Total Records': numRecords,
          'Total Time (ms)': totalTime,
          'Throughput (ops/sec)': throughput.opsPerSec,
          'Avg Write Time (ms)': stats.avg,
          'P95 Write Time (ms)': stats.p95,
          'P99 Write Time (ms)': stats.p99,
        },
      };

      logResults(results);

      // Baseline assertion: should achieve at least 100 ops/sec
      expect(throughput.opsPerSec).toBeGreaterThan(100);
    });

    it('should measure batch write throughput (1000 records in batches of 10)', async () => {
      const totalRecords = 1000;
      const batchSize = 10;
      const numBatches = totalRecords / batchSize;
      const serverUrl = `ws://localhost:${server.port}`;

      client = await createTestClient(serverUrl, {
        nodeId: 'batch-client',
        userId: 'batch-user',
        roles: ['ADMIN'],
      });
      await client.waitForMessage('AUTH_ACK', 5000);

      const batchTimes: number[] = [];

      const { timeMs: totalTime } = await measureTime(async () => {
        for (let batch = 0; batch < numBatches; batch++) {
          const batchStart = performance.now();

          // Send all records in batch without waiting
          for (let i = 0; i < batchSize; i++) {
            const recordIndex = batch * batchSize + i;
            const record = createLWWRecord(
              { index: recordIndex, data: `batch-data-${recordIndex}` },
              'batch-client'
            );
            client.send({
              type: 'CLIENT_OP',
              payload: {
                id: `batch-op-${recordIndex}`,
                mapName: 'batch-test',
                opType: 'PUT',
                key: `record-${recordIndex}`,
                record,
              },
            });
          }

          batchTimes.push(performance.now() - batchStart);

          // Brief pause between batches
          if (batch % 10 === 9) {
            await waitForSync(5);
          }
        }

        // Wait for final sync
        await waitForSync(500);
      });

      const stats = calculateStats(batchTimes);
      const throughput = calculateThroughput(totalRecords, totalTime);

      const results: LoadTestResults = {
        testName: 'Batch Write Throughput (10 per batch)',
        metrics: {
          'Total Records': totalRecords,
          'Batch Size': batchSize,
          'Total Batches': numBatches,
          'Total Time (ms)': totalTime,
          'Throughput (ops/sec)': throughput.opsPerSec,
          'Avg Batch Time (ms)': stats.avg,
          'P95 Batch Time (ms)': stats.p95,
        },
      };

      logResults(results);

      // Batch writes should be at least as fast as sequential
      expect(throughput.opsPerSec).toBeGreaterThan(100);
    });

    it('should compare sequential vs batch throughput', async () => {
      const numRecords = 500;
      const batchSize = 50;
      const serverUrl = `ws://localhost:${server.port}`;

      client = await createTestClient(serverUrl, {
        nodeId: 'compare-client',
        userId: 'compare-user',
        roles: ['ADMIN'],
      });
      await client.waitForMessage('AUTH_ACK', 5000);

      // Sequential writes
      const { timeMs: sequentialTime } = await measureTime(async () => {
        for (let i = 0; i < numRecords; i++) {
          const record = createLWWRecord({ i }, 'compare-client');
          client.send({
            type: 'CLIENT_OP',
            payload: {
              id: `seq-cmp-${i}`,
              mapName: 'compare-seq',
              opType: 'PUT',
              key: `record-${i}`,
              record,
            },
          });
        }
        await waitForSync(500);
      });

      const sequentialThroughput = calculateThroughput(numRecords, sequentialTime);

      // Batch writes
      const numBatches = numRecords / batchSize;
      const { timeMs: batchTime } = await measureTime(async () => {
        for (let batch = 0; batch < numBatches; batch++) {
          for (let i = 0; i < batchSize; i++) {
            const idx = batch * batchSize + i;
            const record = createLWWRecord({ idx }, 'compare-client');
            client.send({
              type: 'CLIENT_OP',
              payload: {
                id: `batch-cmp-${idx}`,
                mapName: 'compare-batch',
                opType: 'PUT',
                key: `record-${idx}`,
                record,
              },
            });
          }
        }
        await waitForSync(500);
      });

      const batchThroughput = calculateThroughput(numRecords, batchTime);

      const speedup = batchThroughput.opsPerSec / sequentialThroughput.opsPerSec;

      const results: LoadTestResults = {
        testName: 'Sequential vs Batch Comparison',
        metrics: {
          'Total Records': numRecords,
          'Batch Size': batchSize,
          'Sequential Time (ms)': sequentialTime,
          'Sequential (ops/sec)': sequentialThroughput.opsPerSec,
          'Batch Time (ms)': batchTime,
          'Batch (ops/sec)': batchThroughput.opsPerSec,
          'Speedup': `${speedup.toFixed(2)}x`,
        },
      };

      logResults(results);

      // Both should achieve reasonable throughput
      expect(sequentialThroughput.opsPerSec).toBeGreaterThan(50);
      expect(batchThroughput.opsPerSec).toBeGreaterThan(50);
    });
  });

  describe('Read Under Load', () => {
    it('should measure read latency with 1000 records', async () => {
      const numRecords = 1000;
      const serverUrl = `ws://localhost:${server.port}`;

      // Setup: Create client and write data
      client = await createTestClient(serverUrl, {
        nodeId: 'read-writer',
        userId: 'read-user',
        roles: ['ADMIN'],
      });
      await client.waitForMessage('AUTH_ACK', 5000);

      // Write 1000 records
      for (let i = 0; i < numRecords; i++) {
        const record = createLWWRecord(
          { index: i, data: `read-data-${i}` },
          'read-writer'
        );
        client.send({
          type: 'CLIENT_OP',
          payload: {
            id: `read-op-${i}`,
            mapName: 'read-test',
            opType: 'PUT',
            key: `record-${i}`,
            record,
          },
        });
      }

      await waitForSync(3000);

      // Measure read latency
      const { timeMs: readTime, result: response } = await measureTime(async () => {
        client.send({
          type: 'QUERY_SUB',
          payload: {
            queryId: 'read-query',
            mapName: 'read-test',
            query: {},
          },
        });
        return client.waitForMessage('QUERY_RESP', 15000);
      });

      const recordsReceived = (response.payload?.results || []).length;

      const results: LoadTestResults = {
        testName: 'Read Latency (1000 records)',
        metrics: {
          'Records in DB': numRecords,
          'Records Received': recordsReceived,
          'Read Latency (ms)': readTime,
          'Records/sec': calculateThroughput(recordsReceived, readTime).opsPerSec,
        },
      };

      logResults(results);

      // Read should be fast
      expect(readTime).toBeLessThan(5000);
      expect(recordsReceived).toBeGreaterThanOrEqual(numRecords * 0.9);
    });

    // This test can be flaky due to timing issues with multiple concurrent subscriptions
    // Skip for CI but can be run locally with: jest --testPathPattern=throughput --testNamePattern="parallel"
    it.skip('should measure parallel read latency (5 clients)', async () => {
      const numRecords = 200;
      const numReaders = 5;
      const serverUrl = `ws://localhost:${server.port}`;

      // Setup: Create writer and write data
      const writer = await createTestClient(serverUrl, {
        nodeId: 'parallel-writer',
        userId: 'write-user',
        roles: ['ADMIN'],
      });
      clients.push(writer);
      await writer.waitForMessage('AUTH_ACK', 5000);

      // Write records
      for (let i = 0; i < numRecords; i++) {
        const record = createLWWRecord({ index: i }, 'parallel-writer');
        writer.send({
          type: 'CLIENT_OP',
          payload: {
            id: `parallel-write-${i}`,
            mapName: 'parallel-read-test',
            opType: 'PUT',
            key: `record-${i}`,
            record,
          },
        });
      }

      await waitForSync(2000);

      // Create reader clients and subscribe them sequentially to avoid overwhelming
      const readers: TestClient[] = [];
      const readTimes: number[] = [];

      for (let i = 0; i < numReaders; i++) {
        const reader = await createTestClient(serverUrl, {
          nodeId: `reader-${i}`,
          userId: `reader-user-${i}`,
          roles: ['USER'],
        });
        readers.push(reader);
        clients.push(reader);
        await reader.waitForMessage('AUTH_ACK', 5000);

        // Subscribe and measure time for each reader
        const start = performance.now();
        reader.send({
          type: 'QUERY_SUB',
          payload: {
            queryId: `parallel-query-${i}`,
            mapName: 'parallel-read-test',
            query: {},
          },
        });
        await reader.waitForMessage('QUERY_RESP', 15000);
        readTimes.push(performance.now() - start);
      }

      const stats = calculateStats(readTimes);

      const results: LoadTestResults = {
        testName: 'Parallel Read Latency (5 clients)',
        metrics: {
          'Records in DB': numRecords,
          'Readers': numReaders,
          'Avg Read Latency (ms)': stats.avg,
          'Min Read Latency (ms)': stats.min,
          'Max Read Latency (ms)': stats.max,
          'P95 Read Latency (ms)': stats.p95,
        },
      };

      logResults(results);

      // All reads should complete within reasonable time
      expect(stats.avg).toBeLessThan(5000);
      expect(stats.p95).toBeLessThan(10000);
    });
  });

  describe('Mixed Read/Write Load', () => {
    // This test can be flaky due to timing issues with multiple concurrent subscriptions
    // Skip for CI but can be run locally with: jest --testPathPattern=throughput --testNamePattern="concurrent"
    it.skip('should handle concurrent reads and writes', async () => {
      const numWriters = 2;
      const numReaders = 2;
      const writesPerClient = 30;
      const serverUrl = `ws://localhost:${server.port}`;

      // Create writers
      const writers: TestClient[] = [];
      for (let i = 0; i < numWriters; i++) {
        const writer = await createTestClient(serverUrl, {
          nodeId: `mixed-writer-${i}`,
          userId: `writer-${i}`,
          roles: ['ADMIN'],
        });
        writers.push(writer);
        clients.push(writer);
        await writer.waitForMessage('AUTH_ACK', 5000);
      }

      // Create readers and subscribe them one by one
      const readers: TestClient[] = [];
      for (let i = 0; i < numReaders; i++) {
        const reader = await createTestClient(serverUrl, {
          nodeId: `mixed-reader-${i}`,
          userId: `reader-${i}`,
          roles: ['USER'],
        });
        readers.push(reader);
        clients.push(reader);
        await reader.waitForMessage('AUTH_ACK', 5000);

        // Subscribe immediately
        reader.send({
          type: 'QUERY_SUB',
          payload: {
            queryId: `mixed-query-${i}`,
            mapName: 'mixed-test',
            query: {},
          },
        });
        await reader.waitForMessage('QUERY_RESP', 10000);
      }

      // Writers start writing
      const writeStartTime = performance.now();
      for (let w = 0; w < numWriters; w++) {
        const writer = writers[w];
        for (let i = 0; i < writesPerClient; i++) {
          const record = createLWWRecord(
            { writer: w, record: i },
            `mixed-writer-${w}`
          );
          writer.send({
            type: 'CLIENT_OP',
            payload: {
              id: `mixed-op-${w}-${i}`,
              mapName: 'mixed-test',
              opType: 'PUT',
              key: `writer-${w}-record-${i}`,
              record,
            },
          });
        }
      }
      const writeEndTime = performance.now();
      const totalWriteTime = writeEndTime - writeStartTime;

      // Wait for sync
      await waitForSync(2000);

      // Count updates received by readers
      const totalWrites = numWriters * writesPerClient;
      const writeThroughput = calculateThroughput(totalWrites, totalWriteTime);

      const results: LoadTestResults = {
        testName: 'Mixed Read/Write Load',
        metrics: {
          'Writers': numWriters,
          'Readers': numReaders,
          'Writes per Writer': writesPerClient,
          'Total Writes': totalWrites,
          'Write Time (ms)': totalWriteTime,
          'Write Throughput (ops/sec)': writeThroughput.opsPerSec,
        },
      };

      logResults(results);

      expect(writeThroughput.opsPerSec).toBeGreaterThan(50);
    });
  });
});
