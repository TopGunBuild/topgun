/**
 * ReadReplicaHandler Unit Tests
 *
 * Tests read scaling via replicas:
 * - Consistency level handling (STRONG, EVENTUAL)
 * - Local read preference
 * - Load balancing strategies
 * - Staleness constraints
 */

import { EventEmitter } from 'events';
import { ReadReplicaHandler, DEFAULT_READ_REPLICA_CONFIG } from '../ReadReplicaHandler';
import { ConsistencyLevel } from '@topgunbuild/core';

// Mock PartitionService
class MockPartitionService {
  private ownerMap: Map<string, string> = new Map();
  private backupMap: Map<string, string[]> = new Map();

  constructor(config: { owners: Record<string, string>; backups: Record<string, string[]> }) {
    for (const [key, owner] of Object.entries(config.owners)) {
      this.ownerMap.set(key, owner);
    }
    for (const [key, backups] of Object.entries(config.backups)) {
      this.backupMap.set(key, backups);
    }
  }

  getPartitionId(key: string): number {
    // Simple hash for testing
    let hash = 0;
    for (let i = 0; i < key.length; i++) {
      hash = (hash * 31 + key.charCodeAt(i)) % 271;
    }
    return hash;
  }

  getOwner(key: string): string {
    return this.ownerMap.get(key) ?? 'default-owner';
  }

  isLocalOwner(key: string): boolean {
    return this.ownerMap.get(key) === 'local-node';
  }

  isRelated(key: string): boolean {
    const owner = this.ownerMap.get(key);
    const backups = this.backupMap.get(key) ?? [];
    return owner === 'local-node' || backups.includes('local-node');
  }

  getDistribution(key: string) {
    return {
      owner: this.ownerMap.get(key) ?? 'default-owner',
      backups: this.backupMap.get(key) ?? [],
    };
  }
}

// Mock ClusterManager
class MockClusterManager extends EventEmitter {
  private _members: string[] = [];

  constructor(members: string[] = []) {
    super();
    this._members = members;
  }

  getMembers(): string[] {
    return this._members;
  }

  setMembers(members: string[]) {
    this._members = members;
  }
}

// Mock LagTracker
class MockLagTracker {
  private lags: Map<string, { current: number }> = new Map();

  setLag(nodeId: string, lag: number) {
    this.lags.set(nodeId, { current: lag });
  }

  getLag(nodeId: string) {
    return this.lags.get(nodeId);
  }
}

describe('ReadReplicaHandler', () => {
  let partitionService: MockPartitionService;
  let clusterManager: MockClusterManager;
  let lagTracker: MockLagTracker;
  let handler: ReadReplicaHandler;

  beforeEach(() => {
    partitionService = new MockPartitionService({
      owners: {
        'key-local': 'local-node',
        'key-remote': 'remote-node',
        'key-backup': 'remote-node',
      },
      backups: {
        'key-local': ['node-2', 'node-3'],
        'key-remote': ['local-node', 'node-3'],
        'key-backup': ['local-node', 'node-2'],
      },
    });
    clusterManager = new MockClusterManager(['local-node', 'remote-node', 'node-2', 'node-3']);
    lagTracker = new MockLagTracker();
    handler = new ReadReplicaHandler(
      partitionService as any,
      clusterManager as any,
      'local-node',
      lagTracker as any
    );
  });

  describe('canServeLocally', () => {
    test('should return true for STRONG when local is owner', () => {
      const result = handler.canServeLocally({
        mapName: 'test',
        key: 'key-local',
        options: { consistency: ConsistencyLevel.STRONG },
      });
      expect(result).toBe(true);
    });

    test('should return false for STRONG when local is not owner', () => {
      const result = handler.canServeLocally({
        mapName: 'test',
        key: 'key-remote',
        options: { consistency: ConsistencyLevel.STRONG },
      });
      expect(result).toBe(false);
    });

    test('should return true for EVENTUAL when local is backup', () => {
      const result = handler.canServeLocally({
        mapName: 'test',
        key: 'key-remote',
        options: { consistency: ConsistencyLevel.EVENTUAL },
      });
      expect(result).toBe(true);
    });

    test('should return true for EVENTUAL when local is owner', () => {
      const result = handler.canServeLocally({
        mapName: 'test',
        key: 'key-local',
        options: { consistency: ConsistencyLevel.EVENTUAL },
      });
      expect(result).toBe(true);
    });
  });

  describe('selectReadNode', () => {
    test('should select owner for STRONG consistency', () => {
      const node = handler.selectReadNode({
        mapName: 'test',
        key: 'key-local',
        options: { consistency: ConsistencyLevel.STRONG },
      });
      expect(node).toBe('local-node');
    });

    test('should return null for STRONG when owner is down', () => {
      clusterManager.setMembers(['local-node', 'node-2', 'node-3']);
      const node = handler.selectReadNode({
        mapName: 'test',
        key: 'key-remote',
        options: { consistency: ConsistencyLevel.STRONG },
      });
      expect(node).toBeNull();
    });

    test('should fallback to backup for STRONG when allowStale is true', () => {
      clusterManager.setMembers(['local-node', 'node-2', 'node-3']);
      const node = handler.selectReadNode({
        mapName: 'test',
        key: 'key-remote',
        options: { consistency: ConsistencyLevel.STRONG, allowStale: true },
      });
      expect(node).toBe('local-node'); // First alive backup
    });

    test('should prefer local node for EVENTUAL when preferLocalReplica is true', () => {
      const localHandler = new ReadReplicaHandler(
        partitionService as any,
        clusterManager as any,
        'local-node',
        lagTracker as any,
        { preferLocalReplica: true }
      );

      const node = localHandler.selectReadNode({
        mapName: 'test',
        key: 'key-backup',
        options: { consistency: ConsistencyLevel.EVENTUAL },
      });
      expect(node).toBe('local-node');
    });
  });

  describe('shouldForwardRead', () => {
    test('should forward STRONG read when not owner', () => {
      const result = handler.shouldForwardRead({
        mapName: 'test',
        key: 'key-remote',
        options: { consistency: ConsistencyLevel.STRONG },
      });
      expect(result).toBe(true);
    });

    test('should not forward STRONG read when owner', () => {
      const result = handler.shouldForwardRead({
        mapName: 'test',
        key: 'key-local',
        options: { consistency: ConsistencyLevel.STRONG },
      });
      expect(result).toBe(false);
    });

    test('should not forward EVENTUAL read when backup', () => {
      const result = handler.shouldForwardRead({
        mapName: 'test',
        key: 'key-backup',
        options: { consistency: ConsistencyLevel.EVENTUAL },
      });
      expect(result).toBe(false);
    });
  });

  describe('load balancing', () => {
    test('round-robin should cycle through replicas', () => {
      const rrHandler = new ReadReplicaHandler(
        partitionService as any,
        clusterManager as any,
        'local-node',
        lagTracker as any,
        { loadBalancing: 'round-robin', preferLocalReplica: false }
      );

      const selections: string[] = [];
      for (let i = 0; i < 4; i++) {
        const node = rrHandler.selectReadNode({
          mapName: 'test',
          key: 'key-backup',
          options: { consistency: ConsistencyLevel.EVENTUAL },
        });
        if (node) selections.push(node);
      }

      // Should have cycled through available replicas
      expect(selections.length).toBe(4);
      // Should not always be the same
      const unique = new Set(selections);
      expect(unique.size).toBeGreaterThan(1);
    });

    test('latency-based should prefer lowest latency node', () => {
      lagTracker.setLag('remote-node', 100);
      lagTracker.setLag('local-node', 10);
      lagTracker.setLag('node-2', 50);

      const latencyHandler = new ReadReplicaHandler(
        partitionService as any,
        clusterManager as any,
        'other-node', // Not local-node to avoid preferLocalReplica
        lagTracker as any,
        { loadBalancing: 'latency-based', preferLocalReplica: false }
      );

      const node = latencyHandler.selectReadNode({
        mapName: 'test',
        key: 'key-backup',
        options: { consistency: ConsistencyLevel.EVENTUAL },
      });

      expect(node).toBe('local-node'); // Lowest latency
    });
  });

  describe('staleness constraints', () => {
    test('should filter replicas by maxStaleness', () => {
      lagTracker.setLag('remote-node', 1000);
      lagTracker.setLag('local-node', 100);
      lagTracker.setLag('node-2', 500);

      const node = handler.selectReadNode({
        mapName: 'test',
        key: 'key-backup',
        options: {
          consistency: ConsistencyLevel.EVENTUAL,
          maxStaleness: 200,
        },
      });

      // Should select local-node as it's within staleness
      expect(node).toBe('local-node');
    });
  });

  describe('createReadMetadata', () => {
    test('should return correct metadata for owner', () => {
      const metadata = handler.createReadMetadata('key-local', {
        consistency: ConsistencyLevel.STRONG,
      });

      expect(metadata).toEqual({
        source: 'local-node',
        isOwner: true,
        consistency: ConsistencyLevel.STRONG,
      });
    });

    test('should return correct metadata for backup', () => {
      const metadata = handler.createReadMetadata('key-remote', {
        consistency: ConsistencyLevel.EVENTUAL,
      });

      expect(metadata).toEqual({
        source: 'local-node',
        isOwner: false,
        consistency: ConsistencyLevel.EVENTUAL,
      });
    });
  });

  describe('getMetrics', () => {
    test('should return current metrics', () => {
      const metrics = handler.getMetrics();

      expect(metrics).toEqual({
        defaultConsistency: ConsistencyLevel.STRONG,
        preferLocalReplica: true,
        loadBalancing: 'latency-based',
        roundRobinPartitions: 0,
      });
    });
  });

  describe('node availability', () => {
    test('should handle all replicas being down', () => {
      clusterManager.setMembers([]); // All nodes down

      const node = handler.selectReadNode({
        mapName: 'test',
        key: 'key-backup',
        options: { consistency: ConsistencyLevel.EVENTUAL },
      });

      expect(node).toBeNull();
    });

    test('should skip dead nodes in selection', () => {
      clusterManager.setMembers(['local-node', 'node-3']); // remote-node and node-2 are down

      const node = handler.selectReadNode({
        mapName: 'test',
        key: 'key-backup',
        options: { consistency: ConsistencyLevel.EVENTUAL },
      });

      expect(['local-node', 'node-3']).toContain(node);
    });
  });
});
