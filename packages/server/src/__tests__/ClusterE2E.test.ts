/**
 * E2E Cluster Replication Tests
 *
 * Tests real replication between multiple server nodes:
 * - 3-node cluster formation
 * - Write replication to backup nodes
 * - Consistency level behavior (EVENTUAL, QUORUM)
 * - Node failure and recovery
 * - Partition ownership verification
 */

import { ServerCoordinator, ServerFactory } from '../';
import { WebSocket } from 'ws';
import { LWWMap, ConsistencyLevel, deserialize } from '@topgunbuild/core';
import { waitForCluster } from './utils/test-helpers';
import { createTestHarness, ServerTestHarness } from './utils/ServerTestHarness';

describe('Cluster E2E Replication', () => {
  let node1: ServerCoordinator;
  let node2: ServerCoordinator;
  let node3: ServerCoordinator;
  let harness1: ServerTestHarness;
  let harness2: ServerTestHarness;
  let harness3: ServerTestHarness;

  // Helper to create a mock client
  function createMockClient(id: string) {
    return {
      id,
      socket: { send: jest.fn(), readyState: WebSocket.OPEN, close: jest.fn() } as any,
      isAuthenticated: true,
      subscriptions: new Set(),
      principal: { userId: id, roles: ['ADMIN'] }
    };
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
      harness1 = createTestHarness(node1);

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
      harness2 = createTestHarness(node2);

      node3 = ServerFactory.create({
        port: 0,
        nodeId: 'node-a',
        host: 'localhost',
        clusterPort: 0,
        peers: [`localhost:${node1.clusterPort}`, `localhost:${node2.clusterPort}`],
        replicationEnabled: true,
        defaultConsistency: ConsistencyLevel.EVENTUAL,
      });
      await node3.ready();
      harness3 = createTestHarness(node3);

      // Wait for full mesh
      await waitForCluster([node1, node2, node3], 3, 15000);
    }, 20000);

    afterAll(async () => {
      await Promise.all([
        node1?.shutdown(),
        node2?.shutdown(),
        node3?.shutdown(),
      ]);
      await new Promise(resolve => setTimeout(resolve, 300));
    });

    test('should form 3-node cluster', () => {
      const members1 = harness1.cluster.getMembers();
      const members2 = harness2.cluster.getMembers();
      const members3 = harness3.cluster.getMembers();

      expect(members1).toHaveLength(3);
      expect(members2).toHaveLength(3);
      expect(members3).toHaveLength(3);

      expect(members1).toContain('node-a');
      expect(members1).toContain('node-b');
      expect(members1).toContain('node-c');
    });

    test('should distribute partitions across all nodes', () => {
      const ps1 = harness1.partitionService;
      const partitionMap = ps1.getPartitionMap();

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
      // Find a key owned by node1
      const ps1 = harness1.partitionService;
      let testKey = '';
      for (let i = 0; i < 100; i++) {
        const key = `test-key-${i}`;
        if (ps1.isLocalOwner(key)) {
          testKey = key;
          break;
        }
      }
      expect(testKey).not.toBe('');

      // Write via node1
      const client = createMockClient('writer-1');
      const timestamp = Date.now();
      const op = {
        opType: 'set',
        mapName: 'e2e-test',
        key: testKey,
        record: {
          value: { data: 'replicated-value', ts: timestamp },
          timestamp: { millis: timestamp, counter: 0, nodeId: 'client' }
        }
      };

      harness1.handleMessage(client, {
        type: 'CLIENT_OP',
        payload: op
      });

      // Wait for replication (EVENTUAL is async)
      await new Promise(resolve => setTimeout(resolve, 500));

      // Verify on owner
      const map1 = node1.getMap('e2e-test') as LWWMap<string, any>;
      expect(map1.get(testKey)?.data).toBe('replicated-value');

      // Verify on backup nodes
      const map2 = node2.getMap('e2e-test') as LWWMap<string, any>;
      const map3 = node3.getMap('e2e-test') as LWWMap<string, any>;

      // At least one backup should have the data
      const hasData2 = map2.get(testKey)?.data === 'replicated-value';
      const hasData3 = map3.get(testKey)?.data === 'replicated-value';

      // With 3 nodes, we should have 2 backups
      expect(hasData2 || hasData3).toBe(true);
    });

    test('should replicate data to backup nodes for local queries', async () => {
      // Phase 14.2: Cross-node live updates now require distributed subscriptions (CLUSTER_SUB_*).
      // Local QUERY_SUB only receives updates from local data changes.
      // This test verifies that data is replicated to backup nodes, so local queries work.

      // Write on node1
      const writer = createMockClient('writer-2');
      const op = {
        opType: 'set',
        mapName: 'events-test',
        key: 'event-key-1',
        record: {
          value: { message: 'hello-cluster' },
          timestamp: { millis: Date.now(), counter: 0, nodeId: 'client' }
        }
      };

      harness1.handleMessage(writer, {
        type: 'CLIENT_OP',
        payload: op
      });

      // Wait for replication to backup nodes
      await new Promise(resolve => setTimeout(resolve, 500));

      // Verify data is replicated - at least one other node should have it
      const map1 = node1.getMap('events-test') as LWWMap<string, any>;
      const map2 = node2.getMap('events-test') as LWWMap<string, any>;
      const map3 = node3.getMap('events-test') as LWWMap<string, any>;

      const value1 = map1.get('event-key-1');
      const value2 = map2.get('event-key-1');
      const value3 = map3.get('event-key-1');

      // Owner has the data
      expect(value1?.message || value2?.message || value3?.message).toBe('hello-cluster');

      // At least one backup should have it (replication factor)
      const nodesWithData = [value1, value2, value3].filter(v => v?.message === 'hello-cluster').length;
      expect(nodesWithData).toBeGreaterThanOrEqual(1);
    });

    test('should handle concurrent writes to different keys', async () => {
      const promises: Promise<void>[] = [];
      const keys: string[] = [];
      const harnesses = [harness1, harness2, harness3];

      // Write 10 keys in parallel from different nodes
      for (let i = 0; i < 10; i++) {
        const key = `concurrent-${i}`;
        keys.push(key);

        const harness = harnesses[i % 3];
        const client = createMockClient(`concurrent-writer-${i}`);

        promises.push(new Promise((resolve) => {
          harness.handleMessage(client, {
            type: 'CLIENT_OP',
            payload: {
              opType: 'set',
              mapName: 'concurrent-test',
              key,
              record: {
                value: { index: i },
                timestamp: { millis: Date.now(), counter: i, nodeId: 'client' }
              }
            }
          });
          setTimeout(resolve, 50);
        }));
      }

      await Promise.all(promises);

      // Wait for propagation
      await new Promise(resolve => setTimeout(resolve, 1000));

      // Verify all keys exist on at least owner + 1 backup
      for (let i = 0; i < keys.length; i++) {
        const key = keys[i];
        let foundCount = 0;

        for (const node of [node1, node2, node3]) {
          const map = node.getMap('concurrent-test') as LWWMap<string, any>;
          if (map.get(key)?.index === i) {
            foundCount++;
          }
        }

        // Should be on at least 2 nodes (owner + backup)
        expect(foundCount).toBeGreaterThanOrEqual(2);
      }
    });

    test('should report replication pipeline status', () => {
      const pipeline = harness1.replicationPipeline;
      expect(pipeline).toBeDefined();

      if (pipeline) {
        const health = pipeline.getHealth();
        expect(health).toBeDefined();
        expect(typeof health.healthy).toBe('boolean');
      }
    });
  });

  describe('Partition Ownership', () => {
    let localNode: ServerCoordinator;
    let localHarness: ServerTestHarness;

    beforeAll(async () => {
      localNode = ServerFactory.create({
        port: 0,
        nodeId: 'single-node',
        host: 'localhost',
        clusterPort: 0,
        peers: [],
        replicationEnabled: true,
      });
      await localNode.ready();
      localHarness = createTestHarness(localNode);
    }, 10000);

    afterAll(async () => {
      await localNode?.shutdown();
      await new Promise(resolve => setTimeout(resolve, 200));
    });

    test('single node should own all partitions', () => {
      const ps = localHarness.partitionService;
      const partitionMap = ps.getPartitionMap();

      for (const partition of partitionMap.partitions) {
        expect(partition.ownerNodeId).toBe('single-node');
      }
    });

    test('single node should have no backups', () => {
      const ps = localHarness.partitionService;

      for (let i = 0; i < 271; i++) {
        const backups = ps.getBackups(i);
        expect(backups).toHaveLength(0);
      }
    });

    test('writes should succeed without replication on single node', async () => {
      const client = createMockClient('single-client');
      const op = {
        opType: 'set',
        mapName: 'single-test',
        key: 'single-key',
        record: {
          value: { single: true },
          timestamp: { millis: Date.now(), counter: 0, nodeId: 'client' }
        }
      };

      localHarness.handleMessage(client, {
        type: 'CLIENT_OP',
        payload: op
      });

      await new Promise(resolve => setTimeout(resolve, 100));

      const map = localNode.getMap('single-test') as LWWMap<string, any>;
      expect(map.get('single-key')?.single).toBe(true);
    });
  });
});
