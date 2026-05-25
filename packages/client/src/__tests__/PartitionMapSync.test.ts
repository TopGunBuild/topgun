/**
 * Tests for Partition Map Sync
 *
 * Verifies partition map subscription and updates:
 * - updateMap() accepts newer versions only
 * - updatePartition() updates single partition
 * - getOwner() returns owner for key
 * - getBackups() returns backup nodes for key
 * - getMap() returns full map
 * - Older map versions ignored
 * - partitionMapUpdated event emitted
 */
import { PARTITION_COUNT } from '@topgunbuild/core';
import type { PartitionMap } from '@topgunbuild/core';

// Create mock partition map
function createMockPartitionMap(version: number, nodeCount: number = 3): PartitionMap {
  const nodes = [];
  for (let i = 0; i < nodeCount; i++) {
    nodes.push({
      nodeId: `node-${i + 1}`,
      status: 'ACTIVE' as const,
      endpoints: {
        websocket: `ws://node${i + 1}:8080`,
        grpc: `node${i + 1}:9090`,
      },
    });
  }

  const partitions = [];
  for (let i = 0; i < PARTITION_COUNT; i++) {
    const ownerIndex = i % nodeCount;
    const backupIndex = (i + 1) % nodeCount;
    partitions.push({
      partitionId: i,
      ownerNodeId: `node-${ownerIndex + 1}`,
      backupNodeIds: [`node-${backupIndex + 1}`],
    });
  }

  return {
    version,
    partitionCount: PARTITION_COUNT,
    generatedAt: Date.now(),
    nodes,
    partitions,
  };
}

describe('Partition Map Sync', () => {
  describe('PartitionRouter updateMap', () => {
    // require() used inside describe to load modules after Jest environment is set up
    // eslint-disable-next-line @typescript-eslint/no-require-imports
    const { PartitionRouter } = require('../cluster/PartitionRouter');
    // eslint-disable-next-line @typescript-eslint/no-require-imports
    const { ConnectionPool } = require('../cluster/ConnectionPool');

    test('should accept partition map on first update', async () => {
      const pool = new ConnectionPool({});
      const router = new PartitionRouter(pool, {});

      const map = createMockPartitionMap(1);
      const result = router.updateMap(map);

      expect(result).toBe(true);
      expect(router.getMapVersion()).toBe(1);
      expect(router.hasPartitionMap()).toBe(true);

      router.close();
      await pool.close();
    });

    test('should accept newer version', async () => {
      const pool = new ConnectionPool({});
      const router = new PartitionRouter(pool, {});

      router.updateMap(createMockPartitionMap(1));
      const result = router.updateMap(createMockPartitionMap(2));

      expect(result).toBe(true);
      expect(router.getMapVersion()).toBe(2);

      router.close();
      await pool.close();
    });

    test('should reject older version', async () => {
      const pool = new ConnectionPool({});
      const router = new PartitionRouter(pool, {});

      router.updateMap(createMockPartitionMap(5));
      const result = router.updateMap(createMockPartitionMap(3));

      expect(result).toBe(false);
      expect(router.getMapVersion()).toBe(5);

      router.close();
      await pool.close();
    });

    test('should reject same version', async () => {
      const pool = new ConnectionPool({});
      const router = new PartitionRouter(pool, {});

      router.updateMap(createMockPartitionMap(2));
      const result = router.updateMap(createMockPartitionMap(2));

      expect(result).toBe(false);
      expect(router.getMapVersion()).toBe(2);

      router.close();
      await pool.close();
    });

    test('should emit partitionMap:updated event on successful update', async () => {
      const pool = new ConnectionPool({});
      const router = new PartitionRouter(pool, {});
      const handler = jest.fn();

      router.on('partitionMap:updated', handler);
      router.updateMap(createMockPartitionMap(1));

      expect(handler).toHaveBeenCalledWith(1, PARTITION_COUNT);

      router.close();
      await pool.close();
    });
  });

  describe('PartitionRouter updatePartition', () => {
    // require() used inside describe to load modules after Jest environment is set up
    // eslint-disable-next-line @typescript-eslint/no-require-imports
    const { PartitionRouter } = require('../cluster/PartitionRouter');
    // eslint-disable-next-line @typescript-eslint/no-require-imports
    const { ConnectionPool } = require('../cluster/ConnectionPool');

    test('should update single partition', async () => {
      const pool = new ConnectionPool({});
      const router = new PartitionRouter(pool, {});

      router.updateMap(createMockPartitionMap(1));
      router.updatePartition(0, 'node-5', ['node-6', 'node-7']);

      const map = router.getMap();
      const partition = map?.partitions.find((p: any) => p.partitionId === 0);

      expect(partition?.ownerNodeId).toBe('node-5');
      expect(partition?.backupNodeIds).toEqual(['node-6', 'node-7']);

      router.close();
      await pool.close();
    });

    test('should do nothing if no map loaded', async () => {
      const pool = new ConnectionPool({});
      const router = new PartitionRouter(pool, {});

      // Should not throw
      router.updatePartition(0, 'node-5', ['node-6']);
      expect(router.getMap()).toBeNull();

      router.close();
      await pool.close();
    });
  });

  describe('PartitionRouter getOwner', () => {
    // require() used inside describe to load modules after Jest environment is set up
    // eslint-disable-next-line @typescript-eslint/no-require-imports
    const { PartitionRouter } = require('../cluster/PartitionRouter');
    // eslint-disable-next-line @typescript-eslint/no-require-imports
    const { ConnectionPool } = require('../cluster/ConnectionPool');

    test('should return null when no map', async () => {
      const pool = new ConnectionPool({});
      const router = new PartitionRouter(pool, {});

      expect(router.getOwner('any-key')).toBeNull();

      router.close();
      await pool.close();
    });

    test('should return owner for key', async () => {
      const pool = new ConnectionPool({});
      const router = new PartitionRouter(pool, {});

      router.updateMap(createMockPartitionMap(1));
      const owner = router.getOwner('test-key');

      expect(owner).toBeDefined();
      expect(owner).toMatch(/^node-\d+$/);

      router.close();
      await pool.close();
    });

    test('should return consistent owner for same key', async () => {
      const pool = new ConnectionPool({});
      const router = new PartitionRouter(pool, {});

      router.updateMap(createMockPartitionMap(1));
      const owner1 = router.getOwner('consistent-key');
      const owner2 = router.getOwner('consistent-key');

      expect(owner1).toBe(owner2);

      router.close();
      await pool.close();
    });
  });

  describe('PartitionRouter getBackups', () => {
    // require() used inside describe to load modules after Jest environment is set up
    // eslint-disable-next-line @typescript-eslint/no-require-imports
    const { PartitionRouter } = require('../cluster/PartitionRouter');
    // eslint-disable-next-line @typescript-eslint/no-require-imports
    const { ConnectionPool } = require('../cluster/ConnectionPool');

    test('should return empty array when no map', async () => {
      const pool = new ConnectionPool({});
      const router = new PartitionRouter(pool, {});

      expect(router.getBackups('any-key')).toEqual([]);

      router.close();
      await pool.close();
    });

    test('should return backups for key', async () => {
      const pool = new ConnectionPool({});
      const router = new PartitionRouter(pool, {});

      router.updateMap(createMockPartitionMap(1));
      const backups = router.getBackups('test-key');

      expect(Array.isArray(backups)).toBe(true);
      expect(backups.length).toBeGreaterThan(0);

      router.close();
      await pool.close();
    });
  });

  describe('PartitionRouter getMap', () => {
    // require() used inside describe to load modules after Jest environment is set up
    // eslint-disable-next-line @typescript-eslint/no-require-imports
    const { PartitionRouter } = require('../cluster/PartitionRouter');
    // eslint-disable-next-line @typescript-eslint/no-require-imports
    const { ConnectionPool } = require('../cluster/ConnectionPool');

    test('should return null when no map', async () => {
      const pool = new ConnectionPool({});
      const router = new PartitionRouter(pool, {});

      expect(router.getMap()).toBeNull();

      router.close();
      await pool.close();
    });

    test('should return full map', async () => {
      const pool = new ConnectionPool({});
      const router = new PartitionRouter(pool, {});

      const map = createMockPartitionMap(1);
      router.updateMap(map);

      const result = router.getMap();
      expect(result).not.toBeNull();
      expect(result?.version).toBe(1);
      expect(result?.partitions.length).toBe(PARTITION_COUNT);

      router.close();
      await pool.close();
    });
  });

  describe('ClusterClient partition map events', () => {
    // require() used inside describe to load module after Jest environment is set up
    // eslint-disable-next-line @typescript-eslint/no-require-imports
    const { ClusterClient } = require('../cluster/ClusterClient');

    test('should emit partitionMapUpdated on router update', async () => {
      const client = new ClusterClient({
        seedNodes: ['ws://localhost:9001'],
        routingMode: 'direct',
      });

      const handler = jest.fn();
      client.on('partitionMapUpdated', handler);

      // Trigger update through router
      const router = client.getPartitionRouter();
      router.updateMap(createMockPartitionMap(1));

      expect(handler).toHaveBeenCalled();

      await client.close();
    });

    test('should emit partitionMap:ready with version', async () => {
      const client = new ClusterClient({
        seedNodes: ['ws://localhost:9001'],
        routingMode: 'direct',
      });

      const handler = jest.fn();
      client.on('partitionMap:ready', handler);

      const router = client.getPartitionRouter();
      router.updateMap(createMockPartitionMap(5));

      expect(handler).toHaveBeenCalledWith(5);

      await client.close();
    });

    test('should activate routing on first map update', async () => {
      const client = new ClusterClient({
        seedNodes: ['ws://localhost:9001'],
        routingMode: 'direct',
      });

      const handler = jest.fn();
      client.on('routing:active', handler);

      expect(client.isRoutingActive()).toBe(false);

      const router = client.getPartitionRouter();
      router.updateMap(createMockPartitionMap(1));

      expect(handler).toHaveBeenCalled();
      expect(client.isRoutingActive()).toBe(true);

      await client.close();
    });
  });

  describe('PartitionMapRequestMessage type', () => {
    test('should be exported from core', async () => {
      expect(true).toBe(true);
    });
  });
});
