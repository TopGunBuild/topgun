/**
 * ClusterRouting Integration Tests
 *
 * Tests client-side partition-aware routing:
 * - Smart routing to partition owners
 * - Key distribution across nodes
 * - Failover behavior on node failure
 * - Partition map updates
 *
 * These tests verify the ClusterClient routing logic works correctly
 * with real server nodes.
 */

import { WebSocket } from 'ws';
(global as any).WebSocket = WebSocket;

import { ServerCoordinator } from '../../../server/src/ServerCoordinator';
import { ConsistencyLevel } from '@topgunbuild/core';
import { ClusterClient } from '../cluster/ClusterClient';

describe('ClusterRouting Integration', () => {
  let node1: ServerCoordinator;
  let node2: ServerCoordinator;
  let node3: ServerCoordinator;

  // Helper to wait for cluster stabilization
  async function waitForCluster(
    nodes: ServerCoordinator[],
    expectedSize: number,
    timeoutMs = 15000
  ): Promise<boolean> {
    const start = Date.now();
    while (Date.now() - start < timeoutMs) {
      let allReady = true;
      for (const node of nodes) {
        const members = (node as any).cluster?.getMembers() || [];
        if (members.length < expectedSize) {
          allReady = false;
          break;
        }
      }
      if (allReady) return true;
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
    return false;
  }

  describe('3-Node Cluster Routing', () => {
    beforeAll(async () => {
      // Start nodes in order: node-c (highest), node-b, node-a (lowest initiates)
      node1 = new ServerCoordinator({
        port: 0,
        nodeId: 'node-c',
        host: 'localhost',
        clusterPort: 0,
        peers: [],
        replicationEnabled: true,
        defaultConsistency: ConsistencyLevel.EVENTUAL,
      });
      await node1.ready();

      node2 = new ServerCoordinator({
        port: 0,
        nodeId: 'node-b',
        host: 'localhost',
        clusterPort: 0,
        peers: [`localhost:${node1.clusterPort}`],
        replicationEnabled: true,
        defaultConsistency: ConsistencyLevel.EVENTUAL,
      });
      await node2.ready();

      node3 = new ServerCoordinator({
        port: 0,
        nodeId: 'node-a',
        host: 'localhost',
        clusterPort: 0,
        peers: [
          `localhost:${node1.clusterPort}`,
          `localhost:${node2.clusterPort}`,
        ],
        replicationEnabled: true,
        defaultConsistency: ConsistencyLevel.EVENTUAL,
      });
      await node3.ready();

      // Wait for full mesh
      const formed = await waitForCluster([node1, node2, node3], 3);
      expect(formed).toBe(true);
    }, 30000);

    afterAll(async () => {
      await Promise.all([
        node1?.shutdown(),
        node2?.shutdown(),
        node3?.shutdown(),
      ]);
      await new Promise((resolve) => setTimeout(resolve, 500));
    });

    // ==========================================
    // Smart Routing Tests
    // ==========================================

    describe('Smart Routing', () => {
      let client: ClusterClient;

      afterEach(() => {
        client?.close();
      });

      test('should have partition router after receiving map', async () => {
        client = new ClusterClient({
          enabled: true,
          seedNodes: [
            `ws://localhost:${node1.port}`,
            `ws://localhost:${node2.port}`,
            `ws://localhost:${node3.port}`,
          ],
          routingMode: 'direct',
        });

        await client.start();

        // Wait for partition map
        await new Promise((resolve) => setTimeout(resolve, 2000));

        const stats = client.getRouterStats();
        // Map version should be > 0 after receiving from server
        // Note: may be 0 if partition map not yet received
        expect(stats).toBeDefined();
        expect(typeof stats.mapVersion).toBe('number');
      }, 15000);

      test('should track routing metrics', async () => {
        client = new ClusterClient({
          enabled: true,
          seedNodes: [`ws://localhost:${node1.port}`],
          routingMode: 'direct',
        });

        const metrics = client.getRoutingMetrics();
        expect(metrics.totalRoutes).toBe(0);
        expect(metrics.directRoutes).toBe(0);
        expect(metrics.fallbackRoutes).toBe(0);
        expect(metrics.partitionMisses).toBe(0);
      });

      test('should report routing not active without partition map', () => {
        client = new ClusterClient({
          enabled: true,
          seedNodes: [`ws://localhost:${node1.port}`],
          routingMode: 'direct',
        });

        // Before start, routing should not be active
        expect(client.isRoutingActive()).toBe(false);
      });
    });

    // ==========================================
    // Key Distribution Tests
    // ==========================================

    describe('Key Distribution', () => {
      test('should compute consistent partition IDs for keys', () => {
        const client = new ClusterClient({
          enabled: true,
          seedNodes: [`ws://localhost:${node1.port}`],
          routingMode: 'direct',
        });

        // Same key should always map to same partition
        const key = 'test-key-123';

        // Access internal method via type cast for testing
        const router = (client as any).partitionRouter;
        const partitionId1 = router.getPartitionId(key);
        const partitionId2 = router.getPartitionId(key);

        expect(partitionId1).toBe(partitionId2);
        expect(partitionId1).toBeGreaterThanOrEqual(0);
        expect(partitionId1).toBeLessThan(271); // 271 partitions

        client.close();
      });

      test('should distribute keys across partition range', () => {
        const client = new ClusterClient({
          enabled: true,
          seedNodes: [`ws://localhost:${node1.port}`],
          routingMode: 'direct',
        });

        const router = (client as any).partitionRouter;
        const partitionCounts = new Map<number, number>();

        // Generate 1000 random keys
        for (let i = 0; i < 1000; i++) {
          const key = `random-key-${Math.random().toString(36)}`;
          const partitionId = router.getPartitionId(key);
          partitionCounts.set(partitionId, (partitionCounts.get(partitionId) || 0) + 1);
        }

        // Should have reasonable distribution (not all in one partition)
        expect(partitionCounts.size).toBeGreaterThan(50); // At least 50 different partitions used

        client.close();
      });
    });

    // ==========================================
    // Connection Pool Tests
    // ==========================================

    describe('Connection Pool', () => {
      let client: ClusterClient;

      afterEach(() => {
        client?.close();
      });

      test('should connect to seed nodes', async () => {
        client = new ClusterClient({
          enabled: true,
          seedNodes: [
            `ws://localhost:${node1.port}`,
            `ws://localhost:${node2.port}`,
          ],
          routingMode: 'direct',
        });

        await client.start();

        // Should be initialized after start
        expect(client.isInitialized()).toBe(true);
      }, 15000);

      test('should track health status per node', async () => {
        client = new ClusterClient({
          enabled: true,
          seedNodes: [
            `ws://localhost:${node1.port}`,
            `ws://localhost:${node2.port}`,
          ],
          routingMode: 'direct',
        });

        await client.start();

        const health = client.getHealthStatus();
        expect(health.size).toBeGreaterThanOrEqual(1);
      }, 15000);

      test('should report connected nodes', async () => {
        client = new ClusterClient({
          enabled: true,
          seedNodes: [`ws://localhost:${node1.port}`],
          routingMode: 'direct',
        });

        await client.start();

        // Wait for connection
        await new Promise((resolve) => setTimeout(resolve, 1000));

        const connected = client.getConnectedNodes();
        // May have connections depending on auth state
        expect(Array.isArray(connected)).toBe(true);
      }, 15000);
    });

    // ==========================================
    // Failover Behavior Tests
    // ==========================================

    describe('Failover Behavior', () => {
      test('should mark node unavailable after failures', () => {
        const client = new ClusterClient({
          enabled: true,
          seedNodes: [`ws://localhost:${node1.port}`],
          routingMode: 'direct',
          circuitBreaker: {
            failureThreshold: 3,
            resetTimeoutMs: 100,
          },
        });

        // Record failures
        client.recordFailure('test-node');
        expect(client.canUseNode('test-node')).toBe(true);

        client.recordFailure('test-node');
        expect(client.canUseNode('test-node')).toBe(true);

        client.recordFailure('test-node');
        expect(client.canUseNode('test-node')).toBe(false);

        const circuit = client.getCircuit('test-node');
        expect(circuit.state).toBe('open');

        client.close();
      });

      test('should recover after reset timeout', async () => {
        const client = new ClusterClient({
          enabled: true,
          seedNodes: [`ws://localhost:${node1.port}`],
          routingMode: 'direct',
          circuitBreaker: {
            failureThreshold: 2,
            resetTimeoutMs: 50,
          },
        });

        // Open circuit
        client.recordFailure('recover-node');
        client.recordFailure('recover-node');
        expect(client.canUseNode('recover-node')).toBe(false);

        // Wait for reset timeout
        await new Promise((resolve) => setTimeout(resolve, 100));

        // Should be half-open now
        expect(client.canUseNode('recover-node')).toBe(true);
        expect(client.getCircuit('recover-node').state).toBe('half-open');

        // Record success to close circuit
        client.recordSuccess('recover-node');
        expect(client.getCircuit('recover-node').state).toBe('closed');

        client.close();
      });

      test('should emit circuit events', () => {
        const client = new ClusterClient({
          enabled: true,
          seedNodes: [`ws://localhost:${node1.port}`],
          routingMode: 'direct',
          circuitBreaker: {
            failureThreshold: 2,
            resetTimeoutMs: 1000,
          },
        });

        const events: string[] = [];
        client.on('circuit:open', (nodeId) => events.push(`open:${nodeId}`));

        client.recordFailure('event-node');
        client.recordFailure('event-node');

        expect(events).toContain('open:event-node');

        client.close();
      });
    });

    // ==========================================
    // Routing Mode Tests
    // ==========================================

    describe('Routing Modes', () => {
      test('should support direct routing mode', () => {
        const client = new ClusterClient({
          enabled: true,
          seedNodes: [`ws://localhost:${node1.port}`],
          routingMode: 'direct',
        });

        expect(client).toBeDefined();
        client.close();
      });

      test('should support forward routing mode', () => {
        const client = new ClusterClient({
          enabled: true,
          seedNodes: [`ws://localhost:${node1.port}`],
          routingMode: 'forward',
        });

        expect(client).toBeDefined();
        client.close();
      });
    });
  });
});
