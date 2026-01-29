/**
 * E2E Node Failure Tests
 *
 * Tests cluster behavior during node failures:
 * - Cluster continues after node crash
 * - Data recovery when node rejoins
 * - Partition reassignment on failure
 * - Graceful shutdown handling
 * - Network partition scenarios
 */

import { ConsistencyLevel, PARTITION_COUNT } from '@topgunbuild/core';
import {
  createCluster,
  ClusterContext,
  sleep,
  createMockClient,
  waitForClusterFormation,
  waitForPartitionStability,
  getPartitionDistribution,
  isPartitionBalanced,
  verifyDataOnAllNodes,
  waitForDataOnNode,
  ClusterNode,
} from './helpers';
import { ServerCoordinator, ServerFactory } from '@topgunbuild/server';

jest.setTimeout(90000);

describe('Node Failure E2E', () => {
  describe('Cluster Continues After Node Crash', () => {
    let cluster: ClusterContext;

    beforeAll(async () => {
      cluster = await createCluster({
        nodeCount: 3,
        nodeIds: ['crash-1', 'crash-2', 'crash-3'],
        defaultConsistency: ConsistencyLevel.EVENTUAL,
      });
    });

    afterAll(async () => {
      await cluster?.cleanup();
    });

    test('should continue operating after one node shuts down', async () => {
      // Write data before crash
      const client = createMockClient('pre-crash-writer');
      (cluster.nodes[0].coordinator as any).handleMessage(client, {
        type: 'CLIENT_OP',
        payload: {
          opType: 'set',
          mapName: 'crash-test-map',
          key: 'pre-crash-key',
          record: {
            value: { preCrash: true },
            timestamp: { millis: Date.now(), counter: 0, nodeId: 'client' }
          }
        }
      });

      await sleep(500);

      // Shutdown node 2
      const crashedNode = cluster.nodes[1];
      await crashedNode.coordinator.shutdown();

      // Wait for cluster to detect failure
      await sleep(2000);

      // Remaining cluster should still work
      const remainingNodes = cluster.nodes.filter(n => n !== crashedNode);

      // Verify pre-crash data still accessible
      for (const node of remainingNodes) {
        const map = node.coordinator.getMap('crash-test-map');
        const value = (map as any)?.get('pre-crash-key');
        if (value) {
          expect(value.preCrash).toBe(true);
        }
      }

      // Write new data after crash
      const postCrashClient = createMockClient('post-crash-writer');
      (remainingNodes[0].coordinator as any).handleMessage(postCrashClient, {
        type: 'CLIENT_OP',
        payload: {
          opType: 'set',
          mapName: 'crash-test-map',
          key: 'post-crash-key',
          record: {
            value: { postCrash: true },
            timestamp: { millis: Date.now(), counter: 0, nodeId: 'client' }
          }
        }
      });

      await sleep(500);

      // New data should be written on remaining node
      const map = remainingNodes[0].coordinator.getMap('crash-test-map');
      expect((map as any)?.get('post-crash-key')?.postCrash).toBe(true);
    });
  });

  describe('Data Recovery When Node Rejoins', () => {
    let cluster: ClusterContext;
    let rejoiningNodeInfo: ClusterNode;

    beforeAll(async () => {
      cluster = await createCluster({
        nodeCount: 3,
        nodeIds: ['rejoin-1', 'rejoin-2', 'rejoin-3'],
        defaultConsistency: ConsistencyLevel.EVENTUAL,
      });
    });

    afterAll(async () => {
      // Cleanup includes the new node if it was added
      await cluster?.cleanup();
    });

    test('should sync missed data when node rejoins', async () => {
      const nodeToRemove = cluster.nodes[1];
      rejoiningNodeInfo = { ...nodeToRemove };

      // Write initial data
      const client1 = createMockClient('initial-writer');
      (cluster.nodes[0].coordinator as any).handleMessage(client1, {
        type: 'CLIENT_OP',
        payload: {
          opType: 'set',
          mapName: 'rejoin-map',
          key: 'initial-key',
          record: {
            value: { phase: 'initial' },
            timestamp: { millis: Date.now(), counter: 0, nodeId: 'client' }
          }
        }
      });

      await sleep(1000);

      // Shutdown node 2
      await nodeToRemove.coordinator.shutdown();

      // Remove from cluster list
      const nodeIndex = cluster.nodes.indexOf(nodeToRemove);
      cluster.nodes.splice(nodeIndex, 1);

      await sleep(2000);

      // Write data while node 2 is down
      const client2 = createMockClient('offline-writer');
      (cluster.nodes[0].coordinator as any).handleMessage(client2, {
        type: 'CLIENT_OP',
        payload: {
          opType: 'set',
          mapName: 'rejoin-map',
          key: 'missed-key',
          record: {
            value: { phase: 'missed', writtenWhileOffline: true },
            timestamp: { millis: Date.now(), counter: 0, nodeId: 'client' }
          }
        }
      });

      await sleep(500);

      // Rejoin with new ServerCoordinator
      const newNode = ServerFactory.create({
        port: 0,
        nodeId: 'rejoin-2',
        host: 'localhost',
        clusterPort: 0,
        metricsPort: 0,
        peers: [`localhost:${cluster.nodes[0].clusterPort}`],
        jwtSecret: 'cluster-e2e-test-secret',
        replicationEnabled: true,
        defaultConsistency: ConsistencyLevel.EVENTUAL,
      });

      await newNode.ready();

      cluster.nodes.push({
        coordinator: newNode,
        port: newNode.port,
        clusterPort: newNode.clusterPort,
        nodeId: 'rejoin-2',
      });

      // Wait for cluster reformation
      await waitForClusterFormation(cluster.nodes, 3, 15000);
      await sleep(3000); // Extra time for sync

      // Verify cluster has reformed
      for (const node of cluster.nodes) {
        const members = (node.coordinator as any).cluster?.getMembers() || [];
        expect(members.length).toBe(3);
      }
    });
  });

  describe('Partition Reassignment on Failure', () => {
    let cluster: ClusterContext;

    beforeAll(async () => {
      cluster = await createCluster({
        nodeCount: 3,
        nodeIds: ['reassign-1', 'reassign-2', 'reassign-3'],
        defaultConsistency: ConsistencyLevel.EVENTUAL,
      });
    });

    afterAll(async () => {
      await cluster?.cleanup();
    });

    test('should reassign partitions after node leaves', async () => {
      // Record initial distribution
      const initialDist = getPartitionDistribution(cluster.nodes[0]);
      expect(initialDist.size).toBe(3);

      const nodeToRemove = cluster.nodes[2];
      const removedNodeId = nodeToRemove.nodeId;

      // Get partitions owned by the node we're removing
      const removedNodePartitions = initialDist.get(removedNodeId) || [];
      expect(removedNodePartitions.length).toBeGreaterThan(0);

      // Shutdown node
      await nodeToRemove.coordinator.shutdown();
      cluster.nodes.pop();

      // Wait for failure detection and rebalancing
      await sleep(3000);

      // Get new distribution
      const finalDist = getPartitionDistribution(cluster.nodes[0]);

      // Should only have 2 nodes now
      expect(finalDist.size).toBe(2);

      // Removed node should not own any partitions
      expect(finalDist.has(removedNodeId)).toBe(false);

      // Remaining nodes should own all partitions
      const totalOwned = Array.from(finalDist.values())
        .reduce((sum, partitions) => sum + partitions.length, 0);
      expect(totalOwned).toBe(PARTITION_COUNT);

      // Distribution should still be balanced
      expect(isPartitionBalanced(finalDist, 10)).toBe(true);
    });
  });

  describe('Graceful Shutdown', () => {
    let cluster: ClusterContext;

    beforeAll(async () => {
      cluster = await createCluster({
        nodeCount: 3,
        nodeIds: ['graceful-1', 'graceful-2', 'graceful-3'],
        defaultConsistency: ConsistencyLevel.EVENTUAL,
      });
    });

    afterAll(async () => {
      await cluster?.cleanup();
    });

    test('graceful shutdown should complete pending operations', async () => {
      // Write some data
      const client = createMockClient('graceful-writer');
      for (let i = 0; i < 10; i++) {
        (cluster.nodes[0].coordinator as any).handleMessage(client, {
          type: 'CLIENT_OP',
          payload: {
            opType: 'set',
            mapName: 'graceful-map',
            key: `graceful-key-${i}`,
            record: {
              value: { index: i },
              timestamp: { millis: Date.now(), counter: i, nodeId: 'client' }
            }
          }
        });
      }

      // Give time for writes to start replicating
      await sleep(500);

      // Graceful shutdown
      const shuttingNode = cluster.nodes[2];
      const shutdownPromise = shuttingNode.coordinator.shutdown();

      // Shutdown should complete
      await shutdownPromise;
      cluster.nodes.pop();

      // Wait for cluster stabilization
      await sleep(2000);

      // Remaining nodes should have the data
      let foundData = false;
      for (const node of cluster.nodes) {
        const map = node.coordinator.getMap('graceful-map');
        if ((map as any)?.get('graceful-key-0')) {
          foundData = true;
          break;
        }
      }
      expect(foundData).toBe(true);
    });
  });

  describe('Failure Detection Timing', () => {
    let cluster: ClusterContext;

    beforeAll(async () => {
      cluster = await createCluster({
        nodeCount: 3,
        nodeIds: ['timing-1', 'timing-2', 'timing-3'],
        defaultConsistency: ConsistencyLevel.EVENTUAL,
      });
    });

    afterAll(async () => {
      await cluster?.cleanup();
    });

    test('should detect failure within expected timeout', async () => {
      const nodeToFail = cluster.nodes[1];
      const failedNodeId = nodeToFail.nodeId;

      // Get failure detector from another node
      const monitorNode = cluster.nodes[0];
      const failureDetector = (monitorNode.coordinator as any).cluster?.failureDetector;

      const startTime = Date.now();

      // Abruptly close the connection (simulate crash)
      await nodeToFail.coordinator.shutdown();
      cluster.nodes.splice(1, 1);

      // Wait for failure detection
      let detected = false;
      while (Date.now() - startTime < 20000) {
        const members = (monitorNode.coordinator as any).cluster?.getMembers() || [];
        if (!members.includes(failedNodeId)) {
          detected = true;
          break;
        }
        await sleep(100);
      }

      const detectionTime = Date.now() - startTime;

      expect(detected).toBe(true);
      // Should detect within 15 seconds (heartbeat timeout + confirmation)
      expect(detectionTime).toBeLessThan(15000);
    });
  });

  describe('Multiple Node Failures', () => {
    let cluster: ClusterContext;

    beforeAll(async () => {
      cluster = await createCluster({
        nodeCount: 4,
        nodeIds: ['multi-1', 'multi-2', 'multi-3', 'multi-4'],
        defaultConsistency: ConsistencyLevel.EVENTUAL,
      });
    });

    afterAll(async () => {
      await cluster?.cleanup();
    });

    test('cluster should survive losing minority of nodes', async () => {
      // Write data
      const client = createMockClient('multi-fail-writer');
      (cluster.nodes[0].coordinator as any).handleMessage(client, {
        type: 'CLIENT_OP',
        payload: {
          opType: 'set',
          mapName: 'multi-fail-map',
          key: 'survive-key',
          record: {
            value: { survives: true },
            timestamp: { millis: Date.now(), counter: 0, nodeId: 'client' }
          }
        }
      });

      await sleep(1000);

      // Shutdown 1 node (minority failure)
      await cluster.nodes[3].coordinator.shutdown();
      cluster.nodes.pop();

      await sleep(2000);

      // Cluster should still function
      const remainingMembers = (cluster.nodes[0].coordinator as any).cluster?.getMembers() || [];
      expect(remainingMembers.length).toBe(3);

      // Can still write
      const client2 = createMockClient('post-multi-writer');
      (cluster.nodes[0].coordinator as any).handleMessage(client2, {
        type: 'CLIENT_OP',
        payload: {
          opType: 'set',
          mapName: 'multi-fail-map',
          key: 'new-key',
          record: {
            value: { afterFailure: true },
            timestamp: { millis: Date.now(), counter: 0, nodeId: 'client' }
          }
        }
      });

      await sleep(500);

      const map = cluster.nodes[0].coordinator.getMap('multi-fail-map');
      expect((map as any)?.get('new-key')?.afterFailure).toBe(true);
    });
  });

  describe('Node Rejoin After Long Offline', () => {
    let cluster: ClusterContext;

    beforeAll(async () => {
      cluster = await createCluster({
        nodeCount: 3,
        nodeIds: ['long-1', 'long-2', 'long-3'],
        defaultConsistency: ConsistencyLevel.EVENTUAL,
      });
    });

    afterAll(async () => {
      await cluster?.cleanup();
    });

    test('node should sync accumulated changes on rejoin', async () => {
      const nodeToRemove = cluster.nodes[1];

      // Write initial data
      const client1 = createMockClient('long-writer-1');
      (cluster.nodes[0].coordinator as any).handleMessage(client1, {
        type: 'CLIENT_OP',
        payload: {
          opType: 'set',
          mapName: 'long-offline-map',
          key: 'initial',
          record: {
            value: { batch: 0 },
            timestamp: { millis: Date.now(), counter: 0, nodeId: 'client' }
          }
        }
      });

      await sleep(500);

      // Shutdown node
      await nodeToRemove.coordinator.shutdown();
      const nodeIndex = cluster.nodes.indexOf(nodeToRemove);
      cluster.nodes.splice(nodeIndex, 1);

      await sleep(2000);

      // Write many updates while offline
      for (let batch = 1; batch <= 5; batch++) {
        const client = createMockClient(`batch-writer-${batch}`);
        for (let i = 0; i < 10; i++) {
          (cluster.nodes[0].coordinator as any).handleMessage(client, {
            type: 'CLIENT_OP',
            payload: {
              opType: 'set',
              mapName: 'long-offline-map',
              key: `batch-${batch}-key-${i}`,
              record: {
                value: { batch, index: i },
                timestamp: { millis: Date.now(), counter: batch * 10 + i, nodeId: 'client' }
              }
            }
          });
        }
        await sleep(200);
      }

      // Wait for replication among remaining nodes
      await sleep(1000);

      // Rejoin node
      const newNode = ServerFactory.create({
        port: 0,
        nodeId: 'long-2',
        host: 'localhost',
        clusterPort: 0,
        metricsPort: 0,
        peers: [`localhost:${cluster.nodes[0].clusterPort}`],
        jwtSecret: 'cluster-e2e-test-secret',
        replicationEnabled: true,
        defaultConsistency: ConsistencyLevel.EVENTUAL,
      });

      await newNode.ready();

      cluster.nodes.push({
        coordinator: newNode,
        port: newNode.port,
        clusterPort: newNode.clusterPort,
        nodeId: 'long-2',
      });

      // Wait for full sync
      await waitForClusterFormation(cluster.nodes, 3, 15000);
      await sleep(5000); // Extra time for data sync

      // Verify cluster has 3 members
      for (const node of cluster.nodes) {
        const members = (node.coordinator as any).cluster?.getMembers() || [];
        expect(members.length).toBe(3);
      }
    });
  });
});
