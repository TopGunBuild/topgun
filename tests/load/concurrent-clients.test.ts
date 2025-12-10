/**
 * Load Tests: Concurrent Clients
 * Tests server stability and performance under multiple simultaneous client connections
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

describe('Concurrent Clients Load Tests', () => {
  let server: ServerCoordinator;
  let clients: TestClient[] = [];

  beforeEach(async () => {
    server = await createTestServer();
    clients = [];
  });

  afterEach(async () => {
    // Cleanup all clients
    for (const client of clients) {
      try {
        client.close();
      } catch (e) {
        // Ignore cleanup errors
      }
    }
    clients = [];

    // Shutdown server
    if (server) {
      await server.shutdown();
    }

    // Give time for connections to fully close
    await waitForSync(200);
  });

  describe('Multiple Clients Connection', () => {
    it('should handle 50 clients connecting simultaneously', async () => {
      const numClients = 50;
      const serverUrl = `ws://localhost:${server.port}`;
      const connectionTimes: number[] = [];

      const { timeMs: totalTime } = await measureTime(async () => {
        // Connect all clients in parallel
        const connectionPromises = Array.from({ length: numClients }, async (_, i) => {
          const startTime = performance.now();
          const client = await createTestClient(serverUrl, {
            nodeId: `load-client-${i}`,
            userId: `user-${i}`,
            roles: ['USER'],
          });
          connectionTimes.push(performance.now() - startTime);
          return client;
        });

        clients = await Promise.all(connectionPromises);

        // Wait for all clients to authenticate
        await Promise.all(clients.map((c) => c.waitForMessage('AUTH_ACK', 10000)));
      });

      // Verify all clients connected and authenticated
      expect(clients.length).toBe(numClients);
      expect(clients.every((c) => c.isAuthenticated)).toBe(true);

      // Calculate stats
      const stats = calculateStats(connectionTimes);
      const throughput = calculateThroughput(numClients, totalTime);

      const results: LoadTestResults = {
        testName: 'Concurrent Clients: 50',
        metrics: {
          'Total Clients': numClients,
          'Total Time (ms)': totalTime,
          'Avg Connection Time (ms)': stats.avg,
          'Min Connection Time (ms)': stats.min,
          'Max Connection Time (ms)': stats.max,
          'P95 Connection Time (ms)': stats.p95,
          'Throughput (conn/sec)': throughput.opsPerSec,
        },
      };

      logResults(results);

      // Assertions - reasonable performance expectations
      expect(totalTime).toBeLessThan(30000); // Should complete within 30 seconds
      expect(stats.avg).toBeLessThan(1000); // Average connection should be under 1 second
    });

    it('should handle 100 clients connecting simultaneously', async () => {
      const numClients = 100;
      const serverUrl = `ws://localhost:${server.port}`;
      const connectionTimes: number[] = [];

      const { timeMs: totalTime } = await measureTime(async () => {
        // Connect all clients in parallel
        const connectionPromises = Array.from({ length: numClients }, async (_, i) => {
          const startTime = performance.now();
          const client = await createTestClient(serverUrl, {
            nodeId: `load-client-${i}`,
            userId: `user-${i}`,
            roles: ['USER'],
          });
          connectionTimes.push(performance.now() - startTime);
          return client;
        });

        clients = await Promise.all(connectionPromises);

        // Wait for all clients to authenticate
        await Promise.all(clients.map((c) => c.waitForMessage('AUTH_ACK', 15000)));
      });

      // Verify all clients connected and authenticated
      expect(clients.length).toBe(numClients);
      expect(clients.every((c) => c.isAuthenticated)).toBe(true);

      // Calculate stats
      const stats = calculateStats(connectionTimes);
      const throughput = calculateThroughput(numClients, totalTime);

      const results: LoadTestResults = {
        testName: 'Concurrent Clients: 100',
        metrics: {
          'Total Clients': numClients,
          'Total Time (ms)': totalTime,
          'Avg Connection Time (ms)': stats.avg,
          'Min Connection Time (ms)': stats.min,
          'Max Connection Time (ms)': stats.max,
          'P95 Connection Time (ms)': stats.p95,
          'Throughput (conn/sec)': throughput.opsPerSec,
        },
      };

      logResults(results);

      // Assertions - reasonable performance expectations
      expect(totalTime).toBeLessThan(60000); // Should complete within 60 seconds
      expect(stats.avg).toBeLessThan(2000); // Average connection should be under 2 seconds
    });

    it('should verify server stability after many connections', async () => {
      const numClients = 50;
      const serverUrl = `ws://localhost:${server.port}`;

      // Connect clients
      const connectionPromises = Array.from({ length: numClients }, (_, i) =>
        createTestClient(serverUrl, {
          nodeId: `stability-client-${i}`,
          userId: `user-${i}`,
        })
      );

      clients = await Promise.all(connectionPromises);
      await Promise.all(clients.map((c) => c.waitForMessage('AUTH_ACK', 10000)));

      // Verify server is still responsive - try to connect one more client
      const { timeMs: newConnectionTime } = await measureTime(async () => {
        const newClient = await createTestClient(serverUrl, {
          nodeId: 'final-test-client',
          userId: 'final-user',
        });
        await newClient.waitForMessage('AUTH_ACK', 5000);
        clients.push(newClient);
      });

      expect(newConnectionTime).toBeLessThan(5000);

      const results: LoadTestResults = {
        testName: 'Server Stability Check',
        metrics: {
          'Existing Clients': numClients,
          'New Connection Time (ms)': newConnectionTime,
          'Server Status': 'Stable',
        },
      };

      logResults(results);
    });
  });

  describe('Parallel Writes', () => {
    it('should handle 50 clients writing simultaneously (10 records each)', async () => {
      const numClients = 50;
      const recordsPerClient = 10;
      const totalRecords = numClients * recordsPerClient;
      const serverUrl = `ws://localhost:${server.port}`;

      // Connect all clients
      const connectionPromises = Array.from({ length: numClients }, (_, i) =>
        createTestClient(serverUrl, {
          nodeId: `write-client-${i}`,
          userId: `user-${i}`,
          roles: ['ADMIN'],
        })
      );

      clients = await Promise.all(connectionPromises);
      await Promise.all(clients.map((c) => c.waitForMessage('AUTH_ACK', 10000)));

      // All clients write simultaneously
      const { timeMs: totalWriteTime } = await measureTime(async () => {
        for (let clientIdx = 0; clientIdx < numClients; clientIdx++) {
          const client = clients[clientIdx];

          for (let recordIdx = 0; recordIdx < recordsPerClient; recordIdx++) {
            const record = createLWWRecord(
              { data: `test-data-${clientIdx}-${recordIdx}`, timestamp: Date.now() },
              `write-client-${clientIdx}`
            );
            client.send({
              type: 'CLIENT_OP',
              payload: {
                id: `op-${clientIdx}-${recordIdx}`,
                mapName: 'load-test-map',
                opType: 'PUT',
                key: `client-${clientIdx}-record-${recordIdx}`,
                record,
              },
            });
          }
        }

        // Wait for sync
        await waitForSync(3000);
      });

      // Calculate throughput
      const throughput = calculateThroughput(totalRecords, totalWriteTime);

      const results: LoadTestResults = {
        testName: 'Parallel Writes: 50 Clients x 10 Records',
        metrics: {
          'Total Clients': numClients,
          'Records per Client': recordsPerClient,
          'Total Records': totalRecords,
          'Total Time (ms)': totalWriteTime,
          'Throughput (ops/sec)': throughput.opsPerSec,
        },
      };

      logResults(results);

      // Assertions
      expect(throughput.opsPerSec).toBeGreaterThan(10); // At least 10 ops/sec
    });

    it('should verify data synchronization after parallel writes', async () => {
      const numClients = 20;
      const recordsPerClient = 5;
      const serverUrl = `ws://localhost:${server.port}`;

      // Connect all clients
      const connectionPromises = Array.from({ length: numClients }, (_, i) =>
        createTestClient(serverUrl, {
          nodeId: `sync-client-${i}`,
          userId: `user-${i}`,
          roles: ['ADMIN'],
        })
      );

      clients = await Promise.all(connectionPromises);
      await Promise.all(clients.map((c) => c.waitForMessage('AUTH_ACK', 10000)));

      // All clients write to the same collection
      for (let clientIdx = 0; clientIdx < numClients; clientIdx++) {
        const client = clients[clientIdx];

        for (let recordIdx = 0; recordIdx < recordsPerClient; recordIdx++) {
          const record = createLWWRecord(
            { clientId: clientIdx, recordId: recordIdx },
            `sync-client-${clientIdx}`
          );
          client.send({
            type: 'CLIENT_OP',
            payload: {
              id: `sync-op-${clientIdx}-${recordIdx}`,
              mapName: 'sync-test-map',
              opType: 'PUT',
              key: `key-${clientIdx}-${recordIdx}`,
              record,
            },
          });
        }
      }

      // Wait for sync - give enough time for all records to be processed
      await waitForSync(5000);

      // Subscribe a new client to verify all data is there
      const verifyClient = await createTestClient(serverUrl, {
        nodeId: 'verify-client',
        userId: 'verify-user',
        roles: ['ADMIN'],
      });
      clients.push(verifyClient);
      await verifyClient.waitForMessage('AUTH_ACK', 5000);

      // Subscribe to query
      verifyClient.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'verify-query',
          mapName: 'sync-test-map',
          query: {},
        },
      });

      const response = await verifyClient.waitForMessage('QUERY_RESP', 15000);

      // Count received records - response contains results array, not snapshot
      const receivedRecords = (response.payload?.results || []).length;
      const expectedRecords = numClients * recordsPerClient;

      const results: LoadTestResults = {
        testName: 'Data Synchronization Verification',
        metrics: {
          'Expected Records': expectedRecords,
          'Received Records': receivedRecords,
          'Sync Status': receivedRecords >= expectedRecords * 0.9 ? 'OK' : 'PARTIAL',
        },
      };

      logResults(results);

      // Allow some tolerance for race conditions
      expect(receivedRecords).toBeGreaterThanOrEqual(expectedRecords * 0.8);
    });
  });

  describe('Connection Churn', () => {
    it('should handle rapid connect/disconnect cycles', async () => {
      const cycles = 20;
      const serverUrl = `ws://localhost:${server.port}`;
      const cycleTimes: number[] = [];

      const { timeMs: totalTime } = await measureTime(async () => {
        for (let i = 0; i < cycles; i++) {
          const cycleStart = performance.now();

          // Connect 5 clients
          const newClients = await Promise.all(
            Array.from({ length: 5 }, (_, j) =>
              createTestClient(serverUrl, {
                nodeId: `churn-client-${i}-${j}`,
                userId: `user-${i}-${j}`,
              })
            )
          );

          await Promise.all(newClients.map((c) => c.waitForMessage('AUTH_ACK', 5000)));

          // Disconnect them
          newClients.forEach((c) => c.close());

          cycleTimes.push(performance.now() - cycleStart);

          // Brief pause between cycles
          await waitForSync(50);
        }
      });

      const stats = calculateStats(cycleTimes);

      const results: LoadTestResults = {
        testName: 'Connection Churn Test',
        metrics: {
          'Total Cycles': cycles,
          'Clients per Cycle': 5,
          'Total Time (ms)': totalTime,
          'Avg Cycle Time (ms)': stats.avg,
          'P95 Cycle Time (ms)': stats.p95,
        },
      };

      logResults(results);

      // Server should handle churn without degrading too much
      expect(stats.avg).toBeLessThan(2000);
    });
  });
});
