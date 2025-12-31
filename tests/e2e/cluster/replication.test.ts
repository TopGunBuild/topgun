/**
 * E2E Cluster Replication Tests
 *
 * Tests real replication between multiple server nodes:
 * - 3-node cluster formation
 * - Write replication to backup nodes
 * - Consistency level behavior (EVENTUAL, QUORUM, STRONG)
 * - Concurrent writes from multiple nodes
 * - LWW conflict resolution across cluster
 */

import { ConsistencyLevel } from '@topgunbuild/core';
import {
  createCluster,
  ClusterContext,
  createClusterClient,
  writeAndWaitForReplication,
  verifyDataOnAllNodes,
  waitForDataOnNode,
  sleep,
  generateTestData,
  assertClusterConsistency,
  createMockClient,
} from './helpers';

// Increase timeout for cluster tests
jest.setTimeout(60000);

describe('Cluster Replication E2E', () => {
  let cluster: ClusterContext;

  describe('3-Node Cluster Basic Replication', () => {
    beforeAll(async () => {
      cluster = await createCluster({
        nodeCount: 3,
        nodeIds: ['node-c', 'node-b', 'node-a'],
        defaultConsistency: ConsistencyLevel.EVENTUAL,
      });
    });

    afterAll(async () => {
      await cluster?.cleanup();
    });

    test('should form 3-node cluster', () => {
      expect(cluster.nodes).toHaveLength(3);

      for (const node of cluster.nodes) {
        const members = (node.coordinator as any).cluster.getMembers();
        expect(members).toHaveLength(3);
        expect(members).toContain('node-a');
        expect(members).toContain('node-b');
        expect(members).toContain('node-c');
      }
    });

    test('should distribute partitions across all nodes', () => {
      const ps = (cluster.nodes[0].coordinator as any).partitionService;
      const partitionMap = ps.getPartitionMap();

      // Count partitions per node
      const counts: Record<string, number> = {};
      for (const partition of partitionMap.partitions) {
        counts[partition.ownerNodeId] = (counts[partition.ownerNodeId] || 0) + 1;
      }

      // With 271 partitions and 3 nodes, each should have ~90 partitions
      expect(Object.keys(counts)).toHaveLength(3);
      for (const count of Object.values(counts)) {
        expect(count).toBeGreaterThan(80);
        expect(count).toBeLessThan(100);
      }
    });

    test('should replicate write to backup nodes (EVENTUAL)', async () => {
      const node1 = cluster.nodes[0];
      const ps = (node1.coordinator as any).partitionService;

      // Find a key owned by node1
      let testKey = '';
      for (let i = 0; i < 100; i++) {
        const key = `replication-test-${i}`;
        if (ps.isLocalOwner(key)) {
          testKey = key;
          break;
        }
      }
      expect(testKey).not.toBe('');

      // Write via mock client
      const client = createMockClient('writer-replication');
      const timestamp = Date.now();
      const op = {
        opType: 'set',
        mapName: 'replication-map',
        key: testKey,
        record: {
          value: { data: 'replicated-value', timestamp },
          timestamp: { millis: timestamp, counter: 0, nodeId: 'client' }
        }
      };

      (node1.coordinator as any).handleMessage(client, {
        type: 'CLIENT_OP',
        payload: op
      });

      // Wait for replication
      await sleep(1000);

      // Verify on owner
      const map1 = node1.coordinator.getMap('replication-map');
      expect((map1 as any).get(testKey)?.data).toBe('replicated-value');

      // At least one backup should have the data
      let backupHasData = false;
      for (const node of cluster.nodes.slice(1)) {
        const map = node.coordinator.getMap('replication-map');
        if ((map as any)?.get(testKey)?.data === 'replicated-value') {
          backupHasData = true;
          break;
        }
      }
      expect(backupHasData).toBe(true);
    });

    test('should replicate multiple writes across cluster', async () => {
      const testData = generateTestData('multi-write', 20);

      // Write data through different nodes
      for (let i = 0; i < testData.length; i++) {
        const nodeIndex = i % cluster.nodes.length;
        const node = cluster.nodes[nodeIndex];
        const client = createMockClient(`writer-multi-${i}`);

        (node.coordinator as any).handleMessage(client, {
          type: 'CLIENT_OP',
          payload: {
            opType: 'set',
            mapName: 'multi-write-map',
            key: testData[i].key,
            record: {
              value: testData[i].value,
              timestamp: { millis: Date.now(), counter: i, nodeId: 'client' }
            }
          }
        });
      }

      // Wait for replication
      await sleep(2000);

      // Verify all keys exist on at least owner + backup
      for (const item of testData) {
        let foundCount = 0;
        for (const node of cluster.nodes) {
          const map = node.coordinator.getMap('multi-write-map');
          if ((map as any)?.get(item.key)?.index === item.value.index) {
            foundCount++;
          }
        }
        expect(foundCount).toBeGreaterThanOrEqual(2);
      }
    });

    test('should handle concurrent writes to same key with LWW resolution', async () => {
      const node1 = cluster.nodes[0];
      const node2 = cluster.nodes[1];
      const key = 'concurrent-key-1';

      // Write concurrently from two nodes
      const client1 = createMockClient('concurrent-writer-1');
      const client2 = createMockClient('concurrent-writer-2');

      const ts1 = Date.now();
      const ts2 = ts1 + 100; // Second write is later (will win)

      // First write
      (node1.coordinator as any).handleMessage(client1, {
        type: 'CLIENT_OP',
        payload: {
          opType: 'set',
          mapName: 'concurrent-map',
          key,
          record: {
            value: { writer: 'first', ts: ts1 },
            timestamp: { millis: ts1, counter: 0, nodeId: 'node1' }
          }
        }
      });

      // Second write (should win due to higher timestamp)
      (node2.coordinator as any).handleMessage(client2, {
        type: 'CLIENT_OP',
        payload: {
          opType: 'set',
          mapName: 'concurrent-map',
          key,
          record: {
            value: { writer: 'second', ts: ts2 },
            timestamp: { millis: ts2, counter: 0, nodeId: 'node2' }
          }
        }
      });

      // Wait for replication
      await sleep(1500);

      // All nodes should converge to the value with higher timestamp
      for (const node of cluster.nodes) {
        const map = node.coordinator.getMap('concurrent-map');
        const value = (map as any)?.get(key);
        expect(value?.writer).toBe('second');
      }
    });

    test('should maintain consistency after rapid writes', async () => {
      const mapName = 'rapid-writes-map';
      const writeCount = 50;

      // Rapid writes from all nodes
      const promises: Promise<void>[] = [];

      for (let i = 0; i < writeCount; i++) {
        const nodeIndex = i % cluster.nodes.length;
        const node = cluster.nodes[nodeIndex];
        const client = createMockClient(`rapid-${i}`);
        const key = `rapid-key-${i}`;

        promises.push(new Promise(resolve => {
          (node.coordinator as any).handleMessage(client, {
            type: 'CLIENT_OP',
            payload: {
              opType: 'set',
              mapName,
              key,
              record: {
                value: { index: i, writer: node.nodeId },
                timestamp: { millis: Date.now(), counter: i, nodeId: 'client' }
              }
            }
          });
          setTimeout(resolve, 10);
        }));
      }

      await Promise.all(promises);

      // Wait for replication to settle
      await sleep(3000);

      // Verify consistency
      const keys = Array.from({ length: writeCount }, (_, i) => `rapid-key-${i}`);
      const result = await assertClusterConsistency(cluster.nodes, mapName, keys);

      // Most keys should be consistent (allowing some latency)
      const inconsistentCount = result.details.length;
      expect(inconsistentCount).toBeLessThan(writeCount * 0.1); // Max 10% inconsistent
    });
  });

  describe('Cluster with QUORUM Consistency', () => {
    let quorumCluster: ClusterContext;

    beforeAll(async () => {
      quorumCluster = await createCluster({
        nodeCount: 3,
        nodeIds: ['quorum-1', 'quorum-2', 'quorum-3'],
        defaultConsistency: ConsistencyLevel.QUORUM,
      });
    });

    afterAll(async () => {
      await quorumCluster?.cleanup();
    });

    test('should require quorum acknowledgment for writes', async () => {
      const node = quorumCluster.nodes[0];
      const pipeline = (node.coordinator as any).replicationPipeline;

      expect(pipeline).toBeDefined();

      // Write and verify replication
      const client = createMockClient('quorum-writer');
      const key = 'quorum-key-1';

      (node.coordinator as any).handleMessage(client, {
        type: 'CLIENT_OP',
        payload: {
          opType: 'set',
          mapName: 'quorum-map',
          key,
          record: {
            value: { quorum: true },
            timestamp: { millis: Date.now(), counter: 0, nodeId: 'client' }
          }
        }
      });

      // Wait for quorum replication
      await sleep(1000);

      // At least 2 nodes should have the data (quorum for 3 nodes)
      let hasDataCount = 0;
      for (const n of quorumCluster.nodes) {
        const map = n.coordinator.getMap('quorum-map');
        if ((map as any)?.get(key)?.quorum === true) {
          hasDataCount++;
        }
      }

      expect(hasDataCount).toBeGreaterThanOrEqual(2);
    });
  });

  describe('Event Propagation Across Cluster', () => {
    let eventCluster: ClusterContext;

    beforeAll(async () => {
      eventCluster = await createCluster({
        nodeCount: 3,
        nodeIds: ['event-1', 'event-2', 'event-3'],
        defaultConsistency: ConsistencyLevel.EVENTUAL,
      });
    });

    afterAll(async () => {
      await eventCluster?.cleanup();
    });

    test('should propagate query subscription updates across nodes', async () => {
      const node1 = eventCluster.nodes[0];
      const node2 = eventCluster.nodes[1];

      // Setup subscriber on node2
      const subscriber = createMockClient('event-subscriber');
      (node2.coordinator as any).clients.set(subscriber.id, subscriber);

      (node2.coordinator as any).handleMessage(subscriber, {
        type: 'QUERY_SUB',
        payload: { queryId: 'event-sub-1', mapName: 'event-map', query: {} }
      });

      await sleep(200);

      // Write on node1
      const writer = createMockClient('event-writer');
      (node1.coordinator as any).handleMessage(writer, {
        type: 'CLIENT_OP',
        payload: {
          opType: 'set',
          mapName: 'event-map',
          key: 'event-key-1',
          record: {
            value: { eventData: 'propagated' },
            timestamp: { millis: Date.now(), counter: 0, nodeId: 'client' }
          }
        }
      });

      // Wait for propagation
      await sleep(1500);

      // Verify subscriber received update
      const calls = (subscriber.socket.send as jest.Mock).mock.calls;
      let foundUpdate = false;

      for (const call of calls) {
        try {
          const { deserialize } = require('@topgunbuild/core');
          const msg = deserialize(call[0]) as any;
          if (msg.type === 'QUERY_UPDATE' && msg.payload?.key === 'event-key-1') {
            foundUpdate = true;
            expect(msg.payload.value.eventData).toBe('propagated');
            break;
          }
        } catch {
          // Skip non-msgpack
        }
      }

      expect(foundUpdate).toBe(true);
    });
  });

  describe('Replication Pipeline Health', () => {
    let healthCluster: ClusterContext;

    beforeAll(async () => {
      healthCluster = await createCluster({
        nodeCount: 3,
        nodeIds: ['health-1', 'health-2', 'health-3'],
        defaultConsistency: ConsistencyLevel.EVENTUAL,
      });
    });

    afterAll(async () => {
      await healthCluster?.cleanup();
    });

    test('should report healthy replication pipeline', () => {
      for (const node of healthCluster.nodes) {
        const pipeline = (node.coordinator as any).replicationPipeline;
        expect(pipeline).toBeDefined();

        const health = pipeline.getHealth();
        expect(health).toBeDefined();
        expect(typeof health.healthy).toBe('boolean');
      }
    });

    test('should track replication metrics', async () => {
      const node = healthCluster.nodes[0];
      const pipeline = (node.coordinator as any).replicationPipeline;

      // Write some data to generate metrics
      const client = createMockClient('metrics-writer');
      for (let i = 0; i < 10; i++) {
        (node.coordinator as any).handleMessage(client, {
          type: 'CLIENT_OP',
          payload: {
            opType: 'set',
            mapName: 'metrics-map',
            key: `metrics-key-${i}`,
            record: {
              value: { index: i },
              timestamp: { millis: Date.now(), counter: i, nodeId: 'client' }
            }
          }
        });
      }

      await sleep(1000);

      // Check health includes proper fields
      const health = pipeline.getHealth();
      expect(health).toHaveProperty('healthy');
      expect(health).toHaveProperty('unhealthyNodes');
      expect(health).toHaveProperty('laggyNodes');
      expect(health).toHaveProperty('avgLagMs');
    });
  });
});
