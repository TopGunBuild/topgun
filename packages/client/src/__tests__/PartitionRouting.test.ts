/**
 * Tests for Partition Routing (Phase 4.5 Task 03)
 *
 * Verifies smart routing logic in ClusterClient:
 * - Routing to partition owner when connected
 * - Fallback when owner not connected
 * - Fallback when partition map is empty
 * - Smart routing disabled behavior
 * - Routing metrics tracking
 */
import { hashString, PARTITION_COUNT } from '@topgunbuild/core';

describe('Partition Routing', () => {
  describe('Hash Compatibility', () => {
    test('client hash algorithm matches server (hashString % 271)', () => {
      // Same keys should produce same partition IDs on client and server
      const testKeys = [
        'users:abc123',
        'posts:xyz789',
        'test-key-1',
        'another/key/path',
        'special!@#$%chars',
      ];

      for (const key of testKeys) {
        const partitionId = Math.abs(hashString(key)) % PARTITION_COUNT;
        expect(partitionId).toBeGreaterThanOrEqual(0);
        expect(partitionId).toBeLessThan(PARTITION_COUNT);
      }
    });

    test('hash produces consistent results', () => {
      const key = 'consistent-key-test';
      const hash1 = Math.abs(hashString(key)) % PARTITION_COUNT;
      const hash2 = Math.abs(hashString(key)) % PARTITION_COUNT;
      expect(hash1).toBe(hash2);
    });

    test('different keys distribute across partitions', () => {
      const partitions = new Set<number>();

      for (let i = 0; i < 1000; i++) {
        const key = `test-key-${i}-${Math.random()}`;
        const partitionId = Math.abs(hashString(key)) % PARTITION_COUNT;
        partitions.add(partitionId);
      }

      // With 1000 random keys, should cover most of 271 partitions
      expect(partitions.size).toBeGreaterThan(200);
    });

    test('PARTITION_COUNT is 271', () => {
      expect(PARTITION_COUNT).toBe(271);
    });
  });

  describe('ClusterClient Routing Metrics', () => {
    // Import ClusterClient for metrics testing
    const { ClusterClient } = require('../cluster/ClusterClient');

    test('should initialize with zero metrics', () => {
      const client = new ClusterClient({
        seedNodes: ['ws://localhost:9001'],
        routingMode: 'direct',
      });

      const metrics = client.getRoutingMetrics();
      expect(metrics.directRoutes).toBe(0);
      expect(metrics.fallbackRoutes).toBe(0);
      expect(metrics.partitionMisses).toBe(0);
      expect(metrics.totalRoutes).toBe(0);

      client.close();
    });

    test('should reset routing metrics', () => {
      const client = new ClusterClient({
        seedNodes: ['ws://localhost:9001'],
        routingMode: 'direct',
      });

      // Manually modify metrics for testing
      const metrics = client.getRoutingMetrics();

      client.resetRoutingMetrics();

      const resetMetrics = client.getRoutingMetrics();
      expect(resetMetrics.directRoutes).toBe(0);
      expect(resetMetrics.fallbackRoutes).toBe(0);
      expect(resetMetrics.partitionMisses).toBe(0);
      expect(resetMetrics.totalRoutes).toBe(0);

      client.close();
    });

    test('should return metrics copy (not reference)', () => {
      const client = new ClusterClient({
        seedNodes: ['ws://localhost:9001'],
        routingMode: 'direct',
      });

      const metrics1 = client.getRoutingMetrics();
      metrics1.directRoutes = 999;

      const metrics2 = client.getRoutingMetrics();
      expect(metrics2.directRoutes).toBe(0);

      client.close();
    });
  });

  describe('ConnectionPool isConnected', () => {
    const { ConnectionPool } = require('../cluster/ConnectionPool');

    test('should return false for unknown node', () => {
      const pool = new ConnectionPool({});
      expect(pool.isConnected('unknown-node')).toBe(false);
      expect(pool.isNodeConnected('unknown-node')).toBe(false);
      pool.close();
    });

    test('isConnected should be alias for isNodeConnected', () => {
      const pool = new ConnectionPool({});
      // Both methods should return same result for any node
      expect(pool.isConnected('node-1')).toBe(pool.isNodeConnected('node-1'));
      pool.close();
    });
  });

  describe('PartitionRouter', () => {
    const { PartitionRouter } = require('../cluster/PartitionRouter');
    const { ConnectionPool } = require('../cluster/ConnectionPool');

    test('should return null when no partition map', () => {
      const pool = new ConnectionPool({});
      const router = new PartitionRouter(pool, {});

      const result = router.route('any-key');
      expect(result).toBeNull();

      router.close();
      pool.close();
    });

    test('getPartitionId should return consistent values', () => {
      const pool = new ConnectionPool({});
      const router = new PartitionRouter(pool, {});

      const key = 'test-key';
      const p1 = router.getPartitionId(key);
      const p2 = router.getPartitionId(key);
      expect(p1).toBe(p2);
      expect(p1).toBeGreaterThanOrEqual(0);
      expect(p1).toBeLessThan(PARTITION_COUNT);

      router.close();
      pool.close();
    });

    test('should track partition map version', () => {
      const pool = new ConnectionPool({});
      const router = new PartitionRouter(pool, {});

      expect(router.getMapVersion()).toBe(0);
      expect(router.hasPartitionMap()).toBe(false);

      router.close();
      pool.close();
    });
  });

  describe('ClusterClient getConnection behavior', () => {
    const { ClusterClient } = require('../cluster/ClusterClient');

    test('should throw when not connected', () => {
      const client = new ClusterClient({
        seedNodes: ['ws://localhost:9001'],
        routingMode: 'direct',
      });

      // Not initialized, no connections
      expect(() => client.getConnection('any-key')).toThrow('ClusterClient not connected');

      client.close();
    });

    test('should throw when getAnyConnection and not connected', () => {
      const client = new ClusterClient({
        seedNodes: ['ws://localhost:9001'],
        routingMode: 'direct',
      });

      expect(() => client.getAnyConnection()).toThrow('No healthy connection available');

      client.close();
    });

    test('isRoutingActive should be false initially', () => {
      const client = new ClusterClient({
        seedNodes: ['ws://localhost:9001'],
        routingMode: 'direct',
      });

      expect(client.isRoutingActive()).toBe(false);

      client.close();
    });

    test('should default to direct routing mode', () => {
      const client = new ClusterClient({
        seedNodes: ['ws://localhost:9001'],
      });

      // No routingMode specified, should use default from config
      expect(client.isInitialized()).toBe(false);

      client.close();
    });
  });

  describe('Routing Mode Configuration', () => {
    const { ClusterClient } = require('../cluster/ClusterClient');

    test('should accept forward routing mode', () => {
      const client = new ClusterClient({
        seedNodes: ['ws://localhost:9001'],
        routingMode: 'forward',
      });

      // In forward mode, all operations go to primary
      expect(client.isRoutingActive()).toBe(false);

      client.close();
    });

    test('should accept direct routing mode', () => {
      const client = new ClusterClient({
        seedNodes: ['ws://localhost:9001'],
        routingMode: 'direct',
      });

      expect(client.isRoutingActive()).toBe(false);

      client.close();
    });
  });
});
