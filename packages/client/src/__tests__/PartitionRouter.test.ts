/**
 * Tests for PartitionRouter
 */

import { PartitionRouter } from '../cluster/PartitionRouter';
import { ConnectionPool } from '../cluster/ConnectionPool';
import { PartitionMap, PARTITION_COUNT, hashString } from '@topgunbuild/core';

// Mock ConnectionPool
jest.mock('../cluster/ConnectionPool');

describe('PartitionRouter', () => {
  let connectionPool: jest.Mocked<ConnectionPool>;
  let router: PartitionRouter;

  const createMockPartitionMap = (nodes: string[], version = 1): PartitionMap => {
    const partitions = [];
    for (let i = 0; i < PARTITION_COUNT; i++) {
      const ownerIndex = i % nodes.length;
      const backupIndex = nodes.length > 1 ? (ownerIndex + 1) % nodes.length : -1;

      partitions.push({
        partitionId: i,
        ownerNodeId: nodes[ownerIndex],
        backupNodeIds: backupIndex >= 0 ? [nodes[backupIndex]] : [],
      });
    }

    return {
      version,
      partitionCount: PARTITION_COUNT,
      nodes: nodes.map(nodeId => ({
        nodeId,
        endpoints: { websocket: `ws://node-${nodeId}:8080` },
        status: 'ACTIVE' as const,
      })),
      partitions,
      generatedAt: Date.now(),
    };
  };

  beforeEach(() => {
    connectionPool = new ConnectionPool() as jest.Mocked<ConnectionPool>;
    connectionPool.on = jest.fn().mockReturnThis();
    connectionPool.getConnection = jest.fn().mockReturnValue(null);
    connectionPool.getAnyHealthyConnection = jest.fn().mockReturnValue(null);
    connectionPool.sendToPrimary = jest.fn().mockReturnValue(true);
    connectionPool.addNode = jest.fn();
    connectionPool.removeNode = jest.fn();
    connectionPool.getAllNodes = jest.fn().mockReturnValue([]);

    router = new PartitionRouter(connectionPool);
  });

  describe('getPartitionId', () => {
    it('should return consistent partition ID for same key', () => {
      const key = 'test-key-123';
      const partitionId1 = router.getPartitionId(key);
      const partitionId2 = router.getPartitionId(key);

      expect(partitionId1).toBe(partitionId2);
      expect(partitionId1).toBeGreaterThanOrEqual(0);
      expect(partitionId1).toBeLessThan(PARTITION_COUNT);
    });

    it('should distribute keys across partitions', () => {
      const partitionCounts = new Map<number, number>();

      // Generate 1000 random keys
      for (let i = 0; i < 1000; i++) {
        const key = `key-${i}-${Math.random()}`;
        const partitionId = router.getPartitionId(key);
        partitionCounts.set(partitionId, (partitionCounts.get(partitionId) || 0) + 1);
      }

      // Should use multiple partitions (not all in one)
      expect(partitionCounts.size).toBeGreaterThan(50);
    });

    it('should match server-side hashing', () => {
      const key = 'users:abc123';
      const expected = Math.abs(hashString(key)) % PARTITION_COUNT;
      expect(router.getPartitionId(key)).toBe(expected);
    });
  });

  describe('route', () => {
    it('should return null when no partition map available', () => {
      const result = router.route('any-key');
      expect(result).toBeNull();
    });

    it('should route to correct owner after receiving partition map', () => {
      const nodes = ['node-1', 'node-2', 'node-3'];
      const partitionMap = createMockPartitionMap(nodes);

      // Simulate receiving partition map
      const messageHandler = (connectionPool.on as jest.Mock).mock.calls.find(
        call => call[0] === 'message'
      )?.[1];

      if (messageHandler) {
        messageHandler('node-1', { type: 'PARTITION_MAP', payload: partitionMap });
      }

      const key = 'test-key';
      const result = router.route(key);

      expect(result).not.toBeNull();
      expect(result!.partitionId).toBe(router.getPartitionId(key));
      expect(nodes).toContain(result!.nodeId);
      expect(result!.isOwner).toBe(true);
    });
  });

  describe('hasPartitionMap / isMapStale', () => {
    it('should report no partition map initially', () => {
      expect(router.hasPartitionMap()).toBe(false);
      expect(router.isMapStale()).toBe(true);
    });

    it('should report partition map after receiving one', () => {
      const messageHandler = (connectionPool.on as jest.Mock).mock.calls.find(
        call => call[0] === 'message'
      )?.[1];

      if (messageHandler) {
        messageHandler('node-1', {
          type: 'PARTITION_MAP',
          payload: createMockPartitionMap(['node-1']),
        });
      }

      expect(router.hasPartitionMap()).toBe(true);
      expect(router.isMapStale()).toBe(false);
    });
  });

  describe('getMapVersion', () => {
    it('should return 0 when no map', () => {
      expect(router.getMapVersion()).toBe(0);
    });

    it('should return version from partition map', () => {
      const messageHandler = (connectionPool.on as jest.Mock).mock.calls.find(
        call => call[0] === 'message'
      )?.[1];

      const map = createMockPartitionMap(['node-1'], 42);

      if (messageHandler) {
        messageHandler('node-1', { type: 'PARTITION_MAP', payload: map });
      }

      expect(router.getMapVersion()).toBe(42);
    });
  });

  describe('routeToConnection', () => {
    it('should fall back to any healthy connection when no partition map', () => {
      const mockConnection = { send: jest.fn(), close: jest.fn(), readyState: 1 } as any;
      connectionPool.getAnyHealthyConnection = jest.fn().mockReturnValue({
        nodeId: 'fallback-node',
        connection: mockConnection,
      });

      const result = router.routeToConnection('any-key');

      expect(result).not.toBeNull();
      expect(result!.nodeId).toBe('fallback-node');
      expect(connectionPool.getAnyHealthyConnection).toHaveBeenCalled();
    });

    it('should return null in error mode when no partition map', () => {
      const errorRouter = new PartitionRouter(connectionPool, { fallbackMode: 'error' });
      const result = errorRouter.routeToConnection('any-key');
      expect(result).toBeNull();
    });
  });

  describe('getStats', () => {
    it('should return correct stats', () => {
      const stats = router.getStats();

      expect(stats).toEqual({
        mapVersion: 0,
        partitionCount: 0,
        nodeCount: 0,
        lastRefresh: 0,
        isStale: true,
      });
    });

    it('should update stats after receiving partition map', () => {
      const messageHandler = (connectionPool.on as jest.Mock).mock.calls.find(
        call => call[0] === 'message'
      )?.[1];

      const map = createMockPartitionMap(['node-1', 'node-2'], 5);

      if (messageHandler) {
        messageHandler('node-1', { type: 'PARTITION_MAP', payload: map });
      }

      const stats = router.getStats();

      expect(stats.mapVersion).toBe(5);
      expect(stats.partitionCount).toBe(PARTITION_COUNT);
      expect(stats.nodeCount).toBe(2);
      expect(stats.isStale).toBe(false);
      expect(stats.lastRefresh).toBeGreaterThan(0);
    });
  });

  describe('getPartitionsForNode', () => {
    it('should return empty array when no partition map', () => {
      expect(router.getPartitionsForNode('node-1')).toEqual([]);
    });

    it('should return correct partitions for node', () => {
      const messageHandler = (connectionPool.on as jest.Mock).mock.calls.find(
        call => call[0] === 'message'
      )?.[1];

      // With 3 nodes, each should own ~90 partitions (271/3)
      const map = createMockPartitionMap(['node-1', 'node-2', 'node-3']);

      if (messageHandler) {
        messageHandler('node-1', { type: 'PARTITION_MAP', payload: map });
      }

      const partitions = router.getPartitionsForNode('node-1');

      expect(partitions.length).toBeGreaterThan(80);
      expect(partitions.length).toBeLessThan(100);

      // Verify all returned partitions are owned by node-1
      for (const p of partitions) {
        const partitionInfo = map.partitions.find(pi => pi.partitionId === p);
        expect(partitionInfo?.ownerNodeId).toBe('node-1');
      }
    });
  });

  describe('events', () => {
    it('should emit partitionMap:updated event', (done) => {
      router.on('partitionMap:updated', (version, changesCount) => {
        expect(version).toBe(1);
        expect(changesCount).toBeGreaterThan(0);
        done();
      });

      const messageHandler = (connectionPool.on as jest.Mock).mock.calls.find(
        call => call[0] === 'message'
      )?.[1];

      if (messageHandler) {
        messageHandler('node-1', {
          type: 'PARTITION_MAP',
          payload: createMockPartitionMap(['node-1']),
        });
      }
    });
  });

  describe('close', () => {
    it('should cleanup resources', () => {
      router.close();
      expect(router.hasPartitionMap()).toBe(false);
    });
  });
});
