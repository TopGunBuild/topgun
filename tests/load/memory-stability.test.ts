/**
 * Load Tests: Memory Stability
 * Tests long-running operations and large object handling
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
  logResults,
  LoadTestResults,
  generateLargeObject,
} from './utils';

describe('Memory Stability Load Tests', () => {
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

  describe('Long Running Operations', () => {
    it('should maintain stability during 30 seconds of continuous operation', async () => {
      const durationMs = 30000; // 30 seconds
      const serverUrl = `ws://localhost:${server.port}`;

      client = await createTestClient(serverUrl, {
        nodeId: 'long-running-client',
        userId: 'stability-user',
        roles: ['ADMIN'],
      });
      await client.waitForMessage('AUTH_ACK', 5000);

      let writeCount = 0;
      let readCount = 0;
      let errorCount = 0;
      const startTime = Date.now();
      const operationTimes: number[] = [];

      // Subscribe to updates
      client.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'stability-query',
          mapName: 'stability-test',
          query: {},
        },
      });
      await client.waitForMessage('QUERY_RESP', 5000);
      readCount++;

      // Continuous write/read loop
      while (Date.now() - startTime < durationMs) {
        try {
          const opStart = performance.now();

          // Write
          const record = createLWWRecord(
            {
              iteration: writeCount,
              timestamp: Date.now(),
              data: `stability-data-${writeCount}`,
            },
            'long-running-client'
          );
          client.send({
            type: 'CLIENT_OP',
            payload: {
              id: `stability-op-${writeCount}`,
              mapName: 'stability-test',
              opType: 'PUT',
              key: `record-${writeCount % 100}`, // Reuse keys to test overwrites
              record,
            },
          });
          writeCount++;

          operationTimes.push(performance.now() - opStart);

          // Brief pause to not overwhelm
          if (writeCount % 50 === 0) {
            await waitForSync(10);
          }
        } catch (e) {
          errorCount++;
        }
      }

      const totalTime = Date.now() - startTime;
      const throughput = calculateThroughput(writeCount, totalTime);

      // Check connection is still alive
      const isConnected = client.ws.readyState === 1; // WebSocket.OPEN

      const results: LoadTestResults = {
        testName: 'Long Running Stability (30s)',
        metrics: {
          'Duration (ms)': totalTime,
          'Total Writes': writeCount,
          'Error Count': errorCount,
          'Throughput (ops/sec)': throughput.opsPerSec,
          'Connection Status': isConnected ? 'Connected' : 'Disconnected',
          'Stability': errorCount === 0 && isConnected ? 'STABLE' : 'ISSUES',
        },
      };

      logResults(results);

      // Assertions
      expect(errorCount).toBe(0);
      expect(isConnected).toBe(true);
      expect(writeCount).toBeGreaterThan(100); // Should complete many operations
    });

    it('should handle connection stability over time', async () => {
      const checkIntervalMs = 5000; // Check every 5 seconds
      const totalDurationMs = 20000; // 20 seconds total
      const serverUrl = `ws://localhost:${server.port}`;

      client = await createTestClient(serverUrl, {
        nodeId: 'stability-check-client',
        userId: 'stability-user',
        roles: ['ADMIN'],
      });
      await client.waitForMessage('AUTH_ACK', 5000);

      const connectionChecks: { time: number; connected: boolean }[] = [];
      const startTime = Date.now();

      // Periodic connection checks with activity
      while (Date.now() - startTime < totalDurationMs) {
        // Do some activity
        for (let i = 0; i < 10; i++) {
          const record = createLWWRecord({ t: Date.now() }, 'stability-check-client');
          client.send({
            type: 'CLIENT_OP',
            payload: {
              id: `check-op-${Date.now()}-${i}`,
              mapName: 'check-test',
              opType: 'PUT',
              key: `key-${i}`,
              record,
            },
          });
        }

        // Check connection
        connectionChecks.push({
          time: Date.now() - startTime,
          connected: client.ws.readyState === 1,
        });

        await waitForSync(checkIntervalMs);
      }

      const allConnected = connectionChecks.every((c) => c.connected);

      const results: LoadTestResults = {
        testName: 'Connection Stability Check',
        metrics: {
          'Duration (ms)': totalDurationMs,
          'Check Intervals': connectionChecks.length,
          'All Checks Passed': allConnected ? 'Yes' : 'No',
          'Final Status': client.ws.readyState === 1 ? 'Connected' : 'Disconnected',
        },
      };

      logResults(results);

      expect(allConnected).toBe(true);
    });
  });

  describe('Large Object Handling', () => {
    it('should handle 100KB objects', async () => {
      const objectSize = 100 * 1024; // 100KB
      const serverUrl = `ws://localhost:${server.port}`;

      client = await createTestClient(serverUrl, {
        nodeId: 'large-obj-client',
        userId: 'large-user',
        roles: ['ADMIN'],
      });
      await client.waitForMessage('AUTH_ACK', 5000);

      const largeObject = generateLargeObject(objectSize);
      const record = createLWWRecord(largeObject, 'large-obj-client');

      const { timeMs: writeTime } = await measureTime(async () => {
        client.send({
          type: 'CLIENT_OP',
          payload: {
            id: 'large-op-100kb',
            mapName: 'large-objects',
            opType: 'PUT',
            key: 'large-100kb',
            record,
          },
        });
        await waitForSync(500);
      });

      // Verify by reading
      client.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'large-query',
          mapName: 'large-objects',
          query: {},
        },
      });

      const { timeMs: readTime, result: response } = await measureTime(async () => {
        return client.waitForMessage('QUERY_RESP', 10000);
      });

      const queryResults = response.payload?.results || [];
      const receivedData = queryResults.find((r: any) => r.key === 'large-100kb');

      const results: LoadTestResults = {
        testName: 'Large Object: 100KB',
        metrics: {
          'Object Size': '100KB',
          'Write Time (ms)': writeTime,
          'Read Time (ms)': readTime,
          'Data Received': receivedData ? 'Yes' : 'No',
          'Data Integrity': receivedData?.value?.id === largeObject.id ? 'OK' : 'FAILED',
        },
      };

      logResults(results);

      expect(receivedData).toBeTruthy();
      expect(receivedData?.value?.id).toBe(largeObject.id);
    });

    it('should handle 1MB objects', async () => {
      const objectSize = 1024 * 1024; // 1MB
      const serverUrl = `ws://localhost:${server.port}`;

      client = await createTestClient(serverUrl, {
        nodeId: 'mb-obj-client',
        userId: 'mb-user',
        roles: ['ADMIN'],
      });
      await client.waitForMessage('AUTH_ACK', 5000);

      const largeObject = generateLargeObject(objectSize);
      const record = createLWWRecord(largeObject, 'mb-obj-client');

      const { timeMs: writeTime } = await measureTime(async () => {
        client.send({
          type: 'CLIENT_OP',
          payload: {
            id: 'large-op-1mb',
            mapName: 'mb-objects',
            opType: 'PUT',
            key: 'large-1mb',
            record,
          },
        });
        await waitForSync(2000);
      });

      // Verify by reading
      client.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'mb-query',
          mapName: 'mb-objects',
          query: {},
        },
      });

      const { timeMs: readTime, result: response } = await measureTime(async () => {
        return client.waitForMessage('QUERY_RESP', 15000);
      });

      const queryResults = response.payload?.results || [];
      const receivedData = queryResults.find((r: any) => r.key === 'large-1mb');

      const results: LoadTestResults = {
        testName: 'Large Object: 1MB',
        metrics: {
          'Object Size': '1MB',
          'Write Time (ms)': writeTime,
          'Read Time (ms)': readTime,
          'Data Received': receivedData ? 'Yes' : 'No',
          'Data Integrity': receivedData?.value?.id === largeObject.id ? 'OK' : 'FAILED',
        },
      };

      logResults(results);

      expect(receivedData).toBeTruthy();
      expect(receivedData?.value?.id).toBe(largeObject.id);
    });

    it('should handle many medium-sized objects', async () => {
      const numObjects = 100;
      const objectSize = 10 * 1024; // 10KB each = 1MB total
      const serverUrl = `ws://localhost:${server.port}`;

      client = await createTestClient(serverUrl, {
        nodeId: 'many-obj-client',
        userId: 'many-user',
        roles: ['ADMIN'],
      });
      await client.waitForMessage('AUTH_ACK', 5000);

      const objects = Array.from({ length: numObjects }, (_, i) => ({
        key: `obj-${i}`,
        value: generateLargeObject(objectSize),
      }));

      const { timeMs: writeTime } = await measureTime(async () => {
        for (const obj of objects) {
          const record = createLWWRecord(obj.value, 'many-obj-client');
          client.send({
            type: 'CLIENT_OP',
            payload: {
              id: `many-op-${obj.key}`,
              mapName: 'many-objects',
              opType: 'PUT',
              key: obj.key,
              record,
            },
          });
        }
        await waitForSync(5000);
      });

      // Verify by reading
      client.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'many-query',
          mapName: 'many-objects',
          query: {},
        },
      });

      const { timeMs: readTime, result: response } = await measureTime(async () => {
        return client.waitForMessage('QUERY_RESP', 15000);
      });

      const receivedCount = (response.payload?.results || []).length;
      const totalDataSize = numObjects * objectSize;
      const throughput = calculateThroughput(numObjects, writeTime);

      const results: LoadTestResults = {
        testName: 'Many Medium Objects (100 x 10KB)',
        metrics: {
          'Object Count': numObjects,
          'Object Size': '10KB',
          'Total Data': `${(totalDataSize / 1024 / 1024).toFixed(2)}MB`,
          'Write Time (ms)': writeTime,
          'Read Time (ms)': readTime,
          'Objects Received': receivedCount,
          'Throughput (obj/sec)': throughput.opsPerSec,
        },
      };

      logResults(results);

      expect(receivedCount).toBeGreaterThanOrEqual(numObjects * 0.9);
    });
  });

  describe('Stress Testing', () => {
    it('should handle burst traffic', async () => {
      const burstSize = 200;
      const serverUrl = `ws://localhost:${server.port}`;

      client = await createTestClient(serverUrl, {
        nodeId: 'burst-client',
        userId: 'burst-user',
        roles: ['ADMIN'],
      });
      await client.waitForMessage('AUTH_ACK', 5000);

      // Send burst of messages as fast as possible
      const { timeMs: burstTime } = await measureTime(async () => {
        for (let i = 0; i < burstSize; i++) {
          const record = createLWWRecord(
            { i, data: `burst-${i}` },
            'burst-client'
          );
          client.send({
            type: 'CLIENT_OP',
            payload: {
              id: `burst-op-${i}`,
              mapName: 'burst-test',
              opType: 'PUT',
              key: `burst-${i}`,
              record,
            },
          });
        }
      });

      // Wait for processing
      await waitForSync(2000);

      // Check connection is still alive
      const isConnected = client.ws.readyState === 1;

      // Verify data
      client.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'burst-verify',
          mapName: 'burst-test',
          query: {},
        },
      });

      const response = await client.waitForMessage('QUERY_RESP', 10000);
      const receivedCount = (response.payload?.results || []).length;

      const throughput = calculateThroughput(burstSize, burstTime);

      const results: LoadTestResults = {
        testName: 'Burst Traffic Test',
        metrics: {
          'Burst Size': burstSize,
          'Burst Time (ms)': burstTime,
          'Throughput (ops/sec)': throughput.opsPerSec,
          'Records Synced': receivedCount,
          'Connection Status': isConnected ? 'Connected' : 'Disconnected',
        },
      };

      logResults(results);

      expect(isConnected).toBe(true);
      expect(receivedCount).toBeGreaterThanOrEqual(burstSize * 0.9);
    });

    it('should recover from high load', async () => {
      const highLoadOps = 500;
      const serverUrl = `ws://localhost:${server.port}`;

      client = await createTestClient(serverUrl, {
        nodeId: 'recovery-client',
        userId: 'recovery-user',
        roles: ['ADMIN'],
      });
      await client.waitForMessage('AUTH_ACK', 5000);

      // High load phase
      const { timeMs: loadTime } = await measureTime(async () => {
        for (let i = 0; i < highLoadOps; i++) {
          const record = createLWWRecord({ i }, 'recovery-client');
          client.send({
            type: 'CLIENT_OP',
            payload: {
              id: `load-op-${i}`,
              mapName: 'recovery-test',
              opType: 'PUT',
              key: `key-${i}`,
              record,
            },
          });
        }
      });

      // Recovery phase - wait for system to stabilize
      await waitForSync(3000);

      // Test normal operations after high load
      const { timeMs: normalOpTime } = await measureTime(async () => {
        const record = createLWWRecord({ recovered: true }, 'recovery-client');
        client.send({
          type: 'CLIENT_OP',
          payload: {
            id: 'post-load-op',
            mapName: 'recovery-test',
            opType: 'PUT',
            key: 'recovery-key',
            record,
          },
        });
        await waitForSync(100);
      });

      const isResponsive = normalOpTime < 1000;
      const isConnected = client.ws.readyState === 1;

      const results: LoadTestResults = {
        testName: 'High Load Recovery',
        metrics: {
          'High Load Ops': highLoadOps,
          'Load Phase Time (ms)': loadTime,
          'Post-Load Op Time (ms)': normalOpTime,
          'System Responsive': isResponsive ? 'Yes' : 'No',
          'Connection Status': isConnected ? 'Connected' : 'Disconnected',
        },
      };

      logResults(results);

      expect(isConnected).toBe(true);
      expect(isResponsive).toBe(true);
    });
  });

  describe('Memory Patterns', () => {
    it('should handle repeated overwrites without accumulating data', async () => {
      const overwrites = 1000;
      const serverUrl = `ws://localhost:${server.port}`;

      client = await createTestClient(serverUrl, {
        nodeId: 'overwrite-client',
        userId: 'overwrite-user',
        roles: ['ADMIN'],
      });
      await client.waitForMessage('AUTH_ACK', 5000);

      // Repeatedly overwrite the same key
      const { timeMs: overwriteTime } = await measureTime(async () => {
        for (let i = 0; i < overwrites; i++) {
          const record = createLWWRecord(
            { iteration: i, data: `value-${i}` },
            'overwrite-client'
          );
          client.send({
            type: 'CLIENT_OP',
            payload: {
              id: `overwrite-op-${i}`,
              mapName: 'overwrite-test',
              opType: 'PUT',
              key: 'same-key', // Always the same key
              record,
            },
          });

          if (i % 100 === 99) {
            await waitForSync(10);
          }
        }
        await waitForSync(500);
      });

      // Verify only latest value exists
      client.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'overwrite-query',
          mapName: 'overwrite-test',
          query: {},
        },
      });

      const response = await client.waitForMessage('QUERY_RESP', 5000);
      const queryResults = response.payload?.results || [];
      const recordCount = queryResults.length;
      const sameKeyRecord = queryResults.find((r: any) => r.key === 'same-key');
      const latestValue = sameKeyRecord?.value;

      const throughput = calculateThroughput(overwrites, overwriteTime);

      const results: LoadTestResults = {
        testName: 'Repeated Overwrites',
        metrics: {
          'Total Overwrites': overwrites,
          'Time (ms)': overwriteTime,
          'Throughput (ops/sec)': throughput.opsPerSec,
          'Final Record Count': recordCount,
          'Latest Iteration': latestValue?.iteration ?? 'N/A',
          'LWW Behavior': recordCount === 1 ? 'Correct' : 'ISSUE',
        },
      };

      logResults(results);

      // Should only have one record (LWW semantics)
      expect(recordCount).toBe(1);
      // Latest value should be close to the last iteration (within 10% tolerance due to async processing)
      expect(latestValue?.iteration).toBeGreaterThan(overwrites * 0.9);
    });
  });
});
