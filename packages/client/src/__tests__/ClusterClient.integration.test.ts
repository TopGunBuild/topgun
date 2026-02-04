/**
 * ClusterClient Integration Tests
 *
 * Tests client-cluster interactions:
 * - Circuit breaker functionality
 * - Basic initialization
 * - Connection configuration
 *
 * Note: Full end-to-end connection tests require authentication setup
 * which is covered in packages/server/src/__tests__/ClusterE2E.test.ts.
 * These tests focus on client-side cluster functionality.
 */

// Use ws package for Node.js WebSocket implementation
import { WebSocket } from 'ws';
(global as any).WebSocket = WebSocket;

// Use relative import path for server module (cross-package dependency)
import { ServerCoordinator, ServerFactory } from '../../../server/src';
import { ConsistencyLevel } from '@topgunbuild/core';
import { ClusterClient } from '../cluster/ClusterClient';

describe('ClusterClient Integration', () => {
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

  describe('3-Node Cluster', () => {
    beforeAll(async () => {
      // Start nodes in order: node-c (highest), node-b, node-a (lowest initiates)
      node1 = ServerFactory.create({
        port: 0,
        nodeId: 'node-c',
        host: 'localhost',
        clusterPort: 0,
        peers: [],
        replicationEnabled: true,
        defaultConsistency: ConsistencyLevel.EVENTUAL,
      });
      await node1.ready();

      node2 = ServerFactory.create({
        port: 0,
        nodeId: 'node-b',
        host: 'localhost',
        clusterPort: 0,
        peers: [`localhost:${node1.clusterPort}`],
        replicationEnabled: true,
        defaultConsistency: ConsistencyLevel.EVENTUAL,
      });
      await node2.ready();

      node3 = ServerFactory.create({
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
    // Initialization Tests
    // ==========================================

    describe('Initialization', () => {
      let client: ClusterClient;

      afterEach(() => {
        client?.close();
      });

      test('should create client with seed nodes', () => {
        client = new ClusterClient({
          enabled: true,
          seedNodes: [
            `ws://localhost:${node1.port}`,
            `ws://localhost:${node2.port}`,
            `ws://localhost:${node3.port}`,
          ],
          routingMode: 'direct',
        });

        expect(client).toBeDefined();
        expect(client.isConnected()).toBe(false); // Not connected until start()
      });

      test('should start and initiate connections', async () => {
        client = new ClusterClient({
          enabled: true,
          seedNodes: [`ws://localhost:${node1.port}`],
          routingMode: 'direct',
        });

        // Start the client (this initiates connections but doesn't wait for auth)
        await client.start();

        // Client should be initialized
        expect(client.isInitialized()).toBe(true);
      });

      test('should report health status for seed nodes after start', async () => {
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
        // Health status should include entries for seed nodes
        expect(health.size).toBeGreaterThanOrEqual(1);
      });

      test('should use default circuit breaker config', () => {
        client = new ClusterClient({
          enabled: true,
          seedNodes: [`ws://localhost:${node1.port}`],
          routingMode: 'direct',
        });

        // Default threshold is 5
        for (let i = 0; i < 4; i++) {
          client.recordFailure('test-node');
          expect(client.canUseNode('test-node')).toBe(true);
        }

        // 5th failure should open circuit
        client.recordFailure('test-node');
        expect(client.canUseNode('test-node')).toBe(false);
      });

      test('should use custom circuit breaker config', () => {
        client = new ClusterClient({
          enabled: true,
          seedNodes: [`ws://localhost:${node1.port}`],
          routingMode: 'direct',
          circuitBreaker: {
            failureThreshold: 2,
            resetTimeoutMs: 50,
          },
        });

        // Custom threshold is 2
        client.recordFailure('test-node');
        expect(client.canUseNode('test-node')).toBe(true);

        client.recordFailure('test-node');
        expect(client.canUseNode('test-node')).toBe(false);
      });
    });

    // ==========================================
    // Circuit Breaker Tests
    // ==========================================

    describe('Circuit Breaker', () => {
      let client: ClusterClient;

      beforeEach(() => {
        client = new ClusterClient({
          enabled: true,
          seedNodes: [`ws://localhost:${node1.port}`],
          routingMode: 'direct',
          circuitBreaker: {
            failureThreshold: 3,
            resetTimeoutMs: 100,
          },
        });
      });

      afterEach(() => {
        client?.close();
      });

      test('should start with closed circuit', () => {
        const circuit = client.getCircuit('test-node');
        expect(circuit.state).toBe('closed');
        expect(circuit.failures).toBe(0);
      });

      test('should count failures', () => {
        client.recordFailure('node-1');
        expect(client.getCircuit('node-1').failures).toBe(1);

        client.recordFailure('node-1');
        expect(client.getCircuit('node-1').failures).toBe(2);
      });

      test('should open circuit after threshold failures', () => {
        const openHandler = jest.fn();
        client.on('circuit:open', openHandler);

        client.recordFailure('node-x');
        expect(client.canUseNode('node-x')).toBe(true);

        client.recordFailure('node-x');
        expect(client.canUseNode('node-x')).toBe(true);

        client.recordFailure('node-x');
        expect(client.canUseNode('node-x')).toBe(false);
        expect(client.getCircuit('node-x').state).toBe('open');
        expect(openHandler).toHaveBeenCalledWith('node-x');
      });

      test('should not allow requests when circuit is open', () => {
        // Open the circuit
        client.recordFailure('node-blocked');
        client.recordFailure('node-blocked');
        client.recordFailure('node-blocked');

        expect(client.canUseNode('node-blocked')).toBe(false);
        expect(client.getCircuit('node-blocked').state).toBe('open');
      });

      test('should transition to half-open after reset timeout', async () => {
        const halfOpenHandler = jest.fn();
        client.on('circuit:half-open', halfOpenHandler);

        // Open the circuit
        client.recordFailure('node-y');
        client.recordFailure('node-y');
        client.recordFailure('node-y');
        expect(client.canUseNode('node-y')).toBe(false);

        // Wait for reset timeout
        await new Promise((resolve) => setTimeout(resolve, 150));

        // Should now be half-open
        expect(client.canUseNode('node-y')).toBe(true);
        expect(client.getCircuit('node-y').state).toBe('half-open');
        expect(halfOpenHandler).toHaveBeenCalledWith('node-y');
      });

      test('should close circuit on success in half-open state', async () => {
        const closedHandler = jest.fn();
        client.on('circuit:closed', closedHandler);

        // Open the circuit
        client.recordFailure('node-z');
        client.recordFailure('node-z');
        client.recordFailure('node-z');

        // Wait for half-open
        await new Promise((resolve) => setTimeout(resolve, 150));
        client.canUseNode('node-z'); // Trigger half-open transition

        // Record success
        client.recordSuccess('node-z');

        expect(client.getCircuit('node-z').state).toBe('closed');
        expect(client.getCircuit('node-z').failures).toBe(0);
        expect(closedHandler).toHaveBeenCalledWith('node-z');
      });

      test('should reset circuit manually', () => {
        // Add some failures
        client.recordFailure('node-reset');
        client.recordFailure('node-reset');

        // Reset the circuit
        client.resetCircuit('node-reset');

        // Should be back to initial state
        const circuit = client.getCircuit('node-reset');
        expect(circuit.state).toBe('closed');
        expect(circuit.failures).toBe(0);
      });

      test('should reset all circuits', () => {
        // Add failures to multiple nodes
        client.recordFailure('node-1');
        client.recordFailure('node-2');
        client.recordFailure('node-3');

        // Get circuit states (creates entries)
        client.getCircuit('node-1');
        client.getCircuit('node-2');
        client.getCircuit('node-3');

        // Reset all
        client.resetAllCircuits();

        // All should be fresh
        expect(client.getCircuitStates().size).toBe(0);
      });

      test('should track separate circuits per node', () => {
        // Fail node-1 multiple times
        client.recordFailure('node-1');
        client.recordFailure('node-1');
        client.recordFailure('node-1');

        // Fail node-2 once
        client.recordFailure('node-2');

        // node-1 should be open, node-2 should be closed
        expect(client.canUseNode('node-1')).toBe(false);
        expect(client.canUseNode('node-2')).toBe(true);
      });

      test('should get all circuit states', () => {
        client.recordFailure('node-1');
        client.recordFailure('node-2');

        const states = client.getCircuitStates();
        expect(states.size).toBe(2);
        expect(states.get('node-1')?.failures).toBe(1);
        expect(states.get('node-2')?.failures).toBe(1);
      });
    });

    // ==========================================
    // Routing Configuration Tests
    // ==========================================

    describe('Routing Configuration', () => {
      let client: ClusterClient;

      afterEach(() => {
        client?.close();
      });

      test('should support direct routing mode', () => {
        client = new ClusterClient({
          enabled: true,
          seedNodes: [`ws://localhost:${node1.port}`],
          routingMode: 'direct',
        });

        expect(client).toBeDefined();
      });

      test('should support forward routing mode', () => {
        client = new ClusterClient({
          enabled: true,
          seedNodes: [`ws://localhost:${node1.port}`],
          routingMode: 'forward',
        });

        expect(client).toBeDefined();
      });

      test('should report routing not active before partition map', () => {
        client = new ClusterClient({
          enabled: true,
          seedNodes: [`ws://localhost:${node1.port}`],
          routingMode: 'direct',
        });

        // Without starting, routing is not active
        expect(client.isRoutingActive()).toBe(false);
      });

      test('should report initial routing metrics as zero', () => {
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

      test('should report router stats with zero version before map', () => {
        client = new ClusterClient({
          enabled: true,
          seedNodes: [`ws://localhost:${node1.port}`],
          routingMode: 'direct',
        });

        const stats = client.getRouterStats();
        expect(stats.mapVersion).toBe(0);
      });
    });

    // ==========================================
    // Error Handling Tests
    // ==========================================

    describe('Error Handling', () => {
      test('should not throw when closing unstarted client', () => {
        const client = new ClusterClient({
          enabled: true,
          seedNodes: ['ws://localhost:99999'],
          routingMode: 'direct',
        });

        expect(() => client.close()).not.toThrow();
      });

      test('should handle multiple close calls', () => {
        const client = new ClusterClient({
          enabled: true,
          seedNodes: [`ws://localhost:${node1.port}`],
          routingMode: 'direct',
        });

        expect(() => {
          client.close();
          client.close();
          client.close();
        }).not.toThrow();
      });

      test('should report not connected for unavailable server', async () => {
        const client = new ClusterClient({
          enabled: true,
          seedNodes: ['ws://localhost:99999'],
          routingMode: 'direct',
        });

        // Start will timeout waiting for partition map
        // but shouldn't throw
        try {
          await client.start();
        } catch {
          // Expected to timeout or fail gracefully
        }

        expect(client.isConnected()).toBe(false);
        expect(client.getConnectedNodes().length).toBe(0);

        client.close();
      }, 15000);
    });

    // ==========================================
    // Event Emitter Tests
    // ==========================================

    describe('Event Emitter', () => {
      let client: ClusterClient;

      beforeEach(() => {
        client = new ClusterClient({
          enabled: true,
          seedNodes: [`ws://localhost:${node1.port}`],
          routingMode: 'direct',
        });
      });

      afterEach(() => {
        client?.close();
      });

      test('should register event listeners', () => {
        const handler = jest.fn();
        client.on('circuit:open', handler);

        // Trigger event
        client.recordFailure('test');
        client.recordFailure('test');
        client.recordFailure('test');
        client.recordFailure('test');
        client.recordFailure('test');

        expect(handler).toHaveBeenCalled();
      });

      test('should remove event listeners', () => {
        const handler = jest.fn();
        client.on('circuit:open', handler);
        client.off('circuit:open', handler);

        // Trigger event (default threshold is 5)
        for (let i = 0; i < 5; i++) {
          client.recordFailure('test');
        }

        expect(handler).not.toHaveBeenCalled();
      });

      test('should remove all listeners for event', () => {
        const handler1 = jest.fn();
        const handler2 = jest.fn();
        client.on('circuit:open', handler1);
        client.on('circuit:open', handler2);
        client.removeAllListeners('circuit:open');

        // Trigger event
        for (let i = 0; i < 5; i++) {
          client.recordFailure('test');
        }

        expect(handler1).not.toHaveBeenCalled();
        expect(handler2).not.toHaveBeenCalled();
      });

      test('should emit returns false when no listeners', () => {
        const result = client.emit('nonexistent', 'data');
        expect(result).toBe(false);
      });
    });
  });
});
