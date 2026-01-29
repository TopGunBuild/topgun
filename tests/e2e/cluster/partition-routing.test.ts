/**
 * E2E Partition Routing Tests
 *
 * Tests correct partition ownership and routing:
 * - Partition distribution across nodes
 * - Key routing to correct owner
 * - Partition rebalancing on node join
 * - Backup partition assignment
 */

import { ConsistencyLevel, hashString, PARTITION_COUNT } from '@topgunbuild/core';
import {
  createCluster,
  ClusterContext,
  getPartitionDistribution,
  isPartitionBalanced,
  sleep,
  createMockClient,
  waitForClusterFormation,
  waitForPartitionStability,
  ClusterNode,
} from './helpers';
import { ServerCoordinator, ServerFactory } from '@topgunbuild/server';

jest.setTimeout(60000);

describe('Partition Routing E2E', () => {
  describe('Single Node Partition Ownership', () => {
    let cluster: ClusterContext;

    beforeAll(async () => {
      cluster = await createCluster({
        nodeCount: 1,
        nodeIds: ['single-node'],
      });
    });

    afterAll(async () => {
      await cluster?.cleanup();
    });

    test('single node should own all partitions', () => {
      const ps = (cluster.nodes[0].coordinator as any).partitionService;
      const partitionMap = ps.getPartitionMap();

      for (const partition of partitionMap.partitions) {
        expect(partition.ownerNodeId).toBe('single-node');
      }
    });

    test('single node should have no backups', () => {
      const ps = (cluster.nodes[0].coordinator as any).partitionService;

      for (let i = 0; i < PARTITION_COUNT; i++) {
        const backups = ps.getBackups(i);
        expect(backups).toHaveLength(0);
      }
    });

    test('all keys should be local owner on single node', () => {
      const ps = (cluster.nodes[0].coordinator as any).partitionService;

      for (let i = 0; i < 100; i++) {
        const key = `test-key-${i}`;
        expect(ps.isLocalOwner(key)).toBe(true);
      }
    });
  });

  describe('3-Node Partition Distribution', () => {
    let cluster: ClusterContext;

    beforeAll(async () => {
      cluster = await createCluster({
        nodeCount: 3,
        nodeIds: ['node-a', 'node-b', 'node-c'],
      });
    });

    afterAll(async () => {
      await cluster?.cleanup();
    });

    test('should distribute 271 partitions across 3 nodes', () => {
      const distribution = getPartitionDistribution(cluster.nodes[0]);

      expect(distribution.size).toBe(3);

      // Each node should own roughly 90 partitions (271/3)
      for (const [nodeId, partitions] of distribution) {
        expect(partitions.length).toBeGreaterThan(80);
        expect(partitions.length).toBeLessThan(100);
      }
    });

    test('partition distribution should be balanced', () => {
      const distribution = getPartitionDistribution(cluster.nodes[0]);
      expect(isPartitionBalanced(distribution, 20)).toBe(true);
    });

    test('each partition should have 1 backup (DEFAULT_BACKUP_COUNT=1)', () => {
      const ps = (cluster.nodes[0].coordinator as any).partitionService;

      for (let i = 0; i < PARTITION_COUNT; i++) {
        const backups = ps.getBackups(i);
        // With DEFAULT_BACKUP_COUNT=1 and 3 nodes, each partition has 1 backup
        expect(backups).toHaveLength(1);
      }
    });

    test('backup nodes should be different from owner', () => {
      const ps = (cluster.nodes[0].coordinator as any).partitionService;

      for (let i = 0; i < PARTITION_COUNT; i++) {
        const owner = ps.getPartitionOwner(i);
        const backups = ps.getBackups(i);

        expect(backups).not.toContain(owner);
        expect(new Set(backups).size).toBe(backups.length); // No duplicate backups
      }
    });

    test('keys should route to consistent owners', () => {
      // All nodes should agree on partition ownership
      const testKeys = Array.from({ length: 50 }, (_, i) => `routing-test-${i}`);

      for (const key of testKeys) {
        const owners = new Set<string>();

        for (const node of cluster.nodes) {
          const ps = (node.coordinator as any).partitionService;
          owners.add(ps.getOwner(key));
        }

        // All nodes should report same owner
        expect(owners.size).toBe(1);
      }
    });

    test('isRelated should return true for owner and backups', () => {
      const testKey = 'related-test-key';
      let relatedCount = 0;

      for (const node of cluster.nodes) {
        const ps = (node.coordinator as any).partitionService;
        if (ps.isRelated(testKey)) {
          relatedCount++;
        }
      }

      // Owner + 1 backup (DEFAULT_BACKUP_COUNT=1) = 2 nodes should be related
      expect(relatedCount).toBe(2);
    });
  });

  describe('Partition Rebalancing on Node Join', () => {
    let cluster: ClusterContext;

    beforeAll(async () => {
      // Start with 2 nodes
      cluster = await createCluster({
        nodeCount: 2,
        nodeIds: ['rebalance-1', 'rebalance-2'],
      });
    });

    afterAll(async () => {
      await cluster?.cleanup();
    });

    test('should rebalance when new node joins', async () => {
      // Record initial distribution
      const initialDist = getPartitionDistribution(cluster.nodes[0]);
      expect(initialDist.size).toBe(2);

      // Each node owns ~135 partitions
      for (const [, partitions] of initialDist) {
        expect(partitions.length).toBeGreaterThan(130);
        expect(partitions.length).toBeLessThan(140);
      }

      // Add third node
      const newNodeId = 'rebalance-3';
      const thirdNode = ServerFactory.create({
        port: 0,
        nodeId: newNodeId,
        host: 'localhost',
        clusterPort: 0,
        metricsPort: 0,
        peers: [`localhost:${cluster.nodes[0].clusterPort}`],
        jwtSecret: 'cluster-e2e-test-secret',
        replicationEnabled: true,
        defaultConsistency: ConsistencyLevel.EVENTUAL,
      });

      await thirdNode.ready();

      cluster.nodes.push({
        coordinator: thirdNode,
        port: thirdNode.port,
        clusterPort: thirdNode.clusterPort,
        nodeId: newNodeId,
      });

      // Wait for cluster to reform
      await waitForClusterFormation(cluster.nodes, 3);
      await waitForPartitionStability(cluster.nodes);

      // Verify rebalanced distribution
      const rebalancedDist = getPartitionDistribution(cluster.nodes[0]);
      expect(rebalancedDist.size).toBe(3);

      // Each node now owns ~90 partitions
      for (const [, partitions] of rebalancedDist) {
        expect(partitions.length).toBeGreaterThan(80);
        expect(partitions.length).toBeLessThan(100);
      }
    });

    test('partition map version should increment on rebalance', async () => {
      const ps = (cluster.nodes[0].coordinator as any).partitionService;
      const version = ps.getMapVersion();
      expect(version).toBeGreaterThan(1); // Should have incremented
    });
  });

  describe('Key-to-Partition Mapping', () => {
    let cluster: ClusterContext;

    beforeAll(async () => {
      cluster = await createCluster({
        nodeCount: 3,
        nodeIds: ['map-1', 'map-2', 'map-3'],
      });
    });

    afterAll(async () => {
      await cluster?.cleanup();
    });

    test('consistent hashing should distribute keys evenly', () => {
      const ps = (cluster.nodes[0].coordinator as any).partitionService;
      const distribution = new Map<string, number>();

      // Hash 1000 keys
      for (let i = 0; i < 1000; i++) {
        const key = `hash-test-${i}-${Math.random().toString(36)}`;
        const owner = ps.getOwner(key);
        distribution.set(owner, (distribution.get(owner) || 0) + 1);
      }

      // Each node should get roughly 333 keys (within 30% tolerance)
      const expected = 1000 / 3;
      for (const count of distribution.values()) {
        expect(count).toBeGreaterThan(expected * 0.7);
        expect(count).toBeLessThan(expected * 1.3);
      }
    });

    test('same key should always map to same partition', () => {
      const ps = (cluster.nodes[0].coordinator as any).partitionService;

      for (let i = 0; i < 100; i++) {
        const key = `deterministic-${i}`;
        const partition1 = ps.getPartitionId(key);
        const partition2 = ps.getPartitionId(key);
        expect(partition1).toBe(partition2);
      }
    });

    test('partition ID should be in valid range', () => {
      const ps = (cluster.nodes[0].coordinator as any).partitionService;

      for (let i = 0; i < 1000; i++) {
        const key = `range-test-${i}`;
        const partitionId = ps.getPartitionId(key);
        expect(partitionId).toBeGreaterThanOrEqual(0);
        expect(partitionId).toBeLessThan(PARTITION_COUNT);
      }
    });
  });

  describe('Writes Route to Correct Owner', () => {
    let cluster: ClusterContext;

    beforeAll(async () => {
      cluster = await createCluster({
        nodeCount: 3,
        nodeIds: ['write-1', 'write-2', 'write-3'],
      });
    });

    afterAll(async () => {
      await cluster?.cleanup();
    });

    test('write on non-owner should be forwarded to owner', async () => {
      const ps = (cluster.nodes[0].coordinator as any).partitionService;

      // Find a key NOT owned by node 0
      let testKey = '';
      for (let i = 0; i < 100; i++) {
        const key = `forward-test-${i}`;
        if (!ps.isLocalOwner(key)) {
          testKey = key;
          break;
        }
      }
      expect(testKey).not.toBe('');

      // Write through node 0 (not the owner)
      const client = createMockClient('forward-writer');
      (cluster.nodes[0].coordinator as any).handleMessage(client, {
        type: 'CLIENT_OP',
        payload: {
          opType: 'set',
          mapName: 'forward-map',
          key: testKey,
          record: {
            value: { forwarded: true },
            timestamp: { millis: Date.now(), counter: 0, nodeId: 'client' }
          }
        }
      });

      // Wait for forwarding and replication
      await sleep(1000);

      // Find the actual owner
      const ownerNodeId = ps.getOwner(testKey);
      const ownerNode = cluster.nodes.find(n => n.nodeId === ownerNodeId)!;

      // Owner should have the data
      const map = ownerNode.coordinator.getMap('forward-map');
      expect((map as any)?.get(testKey)?.forwarded).toBe(true);
    });

    test('write on owner should be processed locally', async () => {
      const ps = (cluster.nodes[0].coordinator as any).partitionService;

      // Find a key owned by node 0
      let testKey = '';
      for (let i = 0; i < 100; i++) {
        const key = `local-test-${i}`;
        if (ps.isLocalOwner(key)) {
          testKey = key;
          break;
        }
      }
      expect(testKey).not.toBe('');

      // Write through the owner
      const client = createMockClient('local-writer');
      (cluster.nodes[0].coordinator as any).handleMessage(client, {
        type: 'CLIENT_OP',
        payload: {
          opType: 'set',
          mapName: 'local-map',
          key: testKey,
          record: {
            value: { local: true },
            timestamp: { millis: Date.now(), counter: 0, nodeId: 'client' }
          }
        }
      });

      // Should be immediately available on owner
      await sleep(100);
      const map = cluster.nodes[0].coordinator.getMap('local-map');
      expect((map as any)?.get(testKey)?.local).toBe(true);
    });

    test('writes should reach all backups', async () => {
      const ps = (cluster.nodes[0].coordinator as any).partitionService;

      // Find a key with known owner and backups
      const testKey = 'backup-test-key';
      const partitionId = ps.getPartitionId(testKey);
      const owner = ps.getPartitionOwner(partitionId);
      const backups = ps.getBackups(partitionId);

      // Write through owner
      const ownerNode = cluster.nodes.find(n => n.nodeId === owner)!;
      const client = createMockClient('backup-writer');

      (ownerNode.coordinator as any).handleMessage(client, {
        type: 'CLIENT_OP',
        payload: {
          opType: 'set',
          mapName: 'backup-map',
          key: testKey,
          record: {
            value: { reachedBackups: true },
            timestamp: { millis: Date.now(), counter: 0, nodeId: 'client' }
          }
        }
      });

      // Wait for replication
      await sleep(1500);

      // Verify on all backups
      for (const backupId of backups) {
        const backupNode = cluster.nodes.find(n => n.nodeId === backupId)!;
        const map = backupNode.coordinator.getMap('backup-map');
        expect((map as any)?.get(testKey)?.reachedBackups).toBe(true);
      }
    });
  });

  describe('Partition Map Consistency', () => {
    let cluster: ClusterContext;

    beforeAll(async () => {
      cluster = await createCluster({
        nodeCount: 3,
        nodeIds: ['map-a', 'map-b', 'map-c'],
      });
    });

    afterAll(async () => {
      await cluster?.cleanup();
    });

    test('all nodes should have same partition map version', () => {
      const versions = new Set<number>();

      for (const node of cluster.nodes) {
        const ps = (node.coordinator as any).partitionService;
        versions.add(ps.getMapVersion());
      }

      expect(versions.size).toBe(1);
    });

    test('all nodes should agree on partition ownership', () => {
      for (let i = 0; i < PARTITION_COUNT; i++) {
        const owners = new Set<string>();

        for (const node of cluster.nodes) {
          const ps = (node.coordinator as any).partitionService;
          owners.add(ps.getPartitionOwner(i));
        }

        expect(owners.size).toBe(1);
      }
    });

    test('all nodes should agree on backup assignments', () => {
      for (let i = 0; i < PARTITION_COUNT; i++) {
        const backupSets: string[] = [];

        for (const node of cluster.nodes) {
          const ps = (node.coordinator as any).partitionService;
          backupSets.push(ps.getBackups(i).sort().join(','));
        }

        // All backup sets should be identical
        expect(new Set(backupSets).size).toBe(1);
      }
    });

    test('partition map should include node metadata', () => {
      const ps = (cluster.nodes[0].coordinator as any).partitionService;
      const partitionMap = ps.getPartitionMap();

      expect(partitionMap.version).toBeGreaterThan(0);
      expect(partitionMap.partitionCount).toBe(PARTITION_COUNT);
      expect(partitionMap.nodes).toHaveLength(3);
      expect(partitionMap.partitions).toHaveLength(PARTITION_COUNT);
      expect(partitionMap.generatedAt).toBeDefined();

      for (const node of partitionMap.nodes) {
        expect(node.nodeId).toBeDefined();
        expect(node.endpoints.websocket).toBeDefined();
        expect(node.status).toBe('ACTIVE');
      }
    });
  });
});
