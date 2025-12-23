/**
 * Tests for PartitionService
 * Phase 4: Partition-Aware Server Routing
 */

import { PartitionService } from '../cluster/PartitionService';
import { ClusterManager } from '../cluster/ClusterManager';
import { PARTITION_COUNT, hashString } from '@topgunbuild/core';
import { EventEmitter } from 'events';

// Mock ClusterManager
class MockClusterManager extends EventEmitter {
  config = {
    nodeId: 'node-1',
    host: 'localhost',
    port: 8080,
  };

  port = 8080;

  private members: string[] = ['node-1'];

  getMembers(): string[] {
    return this.members;
  }

  setMembers(members: string[]): void {
    this.members = members;
  }

  isLocal(nodeId: string): boolean {
    return nodeId === this.config.nodeId;
  }
}

describe('PartitionService', () => {
  let cluster: MockClusterManager;
  let partitionService: PartitionService;

  beforeEach(() => {
    cluster = new MockClusterManager();
    partitionService = new PartitionService(cluster as unknown as ClusterManager);
  });

  describe('initialization', () => {
    it('should initialize with version 1', () => {
      expect(partitionService.getMapVersion()).toBe(1);
    });

    it('should assign all partitions to single node', () => {
      for (let i = 0; i < PARTITION_COUNT; i++) {
        const owner = partitionService.getPartitionOwner(i);
        expect(owner).toBe('node-1');
      }
    });
  });

  describe('getPartitionId', () => {
    it('should return consistent partition ID', () => {
      const key = 'test-key-123';
      const p1 = partitionService.getPartitionId(key);
      const p2 = partitionService.getPartitionId(key);
      expect(p1).toBe(p2);
    });

    it('should match client-side hash', () => {
      const key = 'users:abc123';
      const expected = Math.abs(hashString(key)) % PARTITION_COUNT;
      expect(partitionService.getPartitionId(key)).toBe(expected);
    });

    it('should return valid partition range', () => {
      for (let i = 0; i < 100; i++) {
        const key = `key-${i}-${Math.random()}`;
        const partitionId = partitionService.getPartitionId(key);
        expect(partitionId).toBeGreaterThanOrEqual(0);
        expect(partitionId).toBeLessThan(PARTITION_COUNT);
      }
    });
  });

  describe('getOwner / isLocalOwner', () => {
    it('should identify local owner correctly', () => {
      const key = 'test-key';
      expect(partitionService.isLocalOwner(key)).toBe(true);
    });

    it('should return owner from distribution', () => {
      const key = 'test-key';
      expect(partitionService.getOwner(key)).toBe('node-1');
    });
  });

  describe('rebalance on memberJoined', () => {
    it('should increment version on rebalance', () => {
      const v1 = partitionService.getMapVersion();

      cluster.setMembers(['node-1', 'node-2']);
      cluster.emit('memberJoined', 'node-2');

      const v2 = partitionService.getMapVersion();
      expect(v2).toBe(v1 + 1);
    });

    it('should distribute partitions across nodes', () => {
      cluster.setMembers(['node-1', 'node-2', 'node-3']);
      cluster.emit('memberJoined', 'node-2');

      const node1Partitions = new Set<number>();
      const node2Partitions = new Set<number>();
      const node3Partitions = new Set<number>();

      for (let i = 0; i < PARTITION_COUNT; i++) {
        const owner = partitionService.getPartitionOwner(i);
        if (owner === 'node-1') node1Partitions.add(i);
        else if (owner === 'node-2') node2Partitions.add(i);
        else if (owner === 'node-3') node3Partitions.add(i);
      }

      // Each node should own ~90 partitions (271/3)
      expect(node1Partitions.size).toBeGreaterThan(85);
      expect(node2Partitions.size).toBeGreaterThan(85);
      expect(node3Partitions.size).toBeGreaterThan(85);

      // Total should equal PARTITION_COUNT
      expect(node1Partitions.size + node2Partitions.size + node3Partitions.size).toBe(PARTITION_COUNT);
    });

    it('should emit rebalanced event', (done) => {
      partitionService.on('rebalanced', (map, changes) => {
        expect(map.version).toBeGreaterThan(1);
        expect(Array.isArray(changes)).toBe(true);
        done();
      });

      cluster.setMembers(['node-1', 'node-2']);
      cluster.emit('memberJoined', 'node-2');
    });
  });

  describe('rebalance on memberLeft', () => {
    beforeEach(() => {
      // Start with 3 nodes
      cluster.setMembers(['node-1', 'node-2', 'node-3']);
      cluster.emit('memberJoined', 'node-2');
    });

    it('should reassign partitions when node leaves', () => {
      const v1 = partitionService.getMapVersion();

      // Node-3 leaves
      cluster.setMembers(['node-1', 'node-2']);
      cluster.emit('memberLeft', 'node-3');

      const v2 = partitionService.getMapVersion();
      expect(v2).toBe(v1 + 1);

      // All partitions should now be owned by node-1 or node-2
      for (let i = 0; i < PARTITION_COUNT; i++) {
        const owner = partitionService.getPartitionOwner(i);
        expect(['node-1', 'node-2']).toContain(owner);
      }
    });
  });

  describe('getPartitionMap', () => {
    it('should return complete partition map', () => {
      cluster.setMembers(['node-1', 'node-2']);
      cluster.emit('memberJoined', 'node-2');

      const map = partitionService.getPartitionMap();

      expect(map.version).toBeGreaterThan(0);
      expect(map.partitionCount).toBe(PARTITION_COUNT);
      expect(map.nodes.length).toBe(2);
      expect(map.partitions.length).toBe(PARTITION_COUNT);
      expect(map.generatedAt).toBeGreaterThan(0);
    });

    it('should include node endpoints', () => {
      const map = partitionService.getPartitionMap();

      expect(map.nodes[0].nodeId).toBe('node-1');
      expect(map.nodes[0].endpoints.websocket).toContain('ws://');
      expect(map.nodes[0].status).toBe('ACTIVE');
    });

    it('should include backup nodes', () => {
      cluster.setMembers(['node-1', 'node-2']);
      cluster.emit('memberJoined', 'node-2');

      const map = partitionService.getPartitionMap();

      // Each partition should have 1 backup
      for (const partition of map.partitions) {
        expect(partition.backupNodeIds.length).toBe(1);
        expect(partition.backupNodeIds[0]).not.toBe(partition.ownerNodeId);
      }
    });
  });

  describe('getPartitionInfo', () => {
    it('should return null for invalid partition', () => {
      expect(partitionService.getPartitionInfo(-1)).toBeNull();
      expect(partitionService.getPartitionInfo(PARTITION_COUNT + 1)).toBeNull();
    });

    it('should return correct info for valid partition', () => {
      const info = partitionService.getPartitionInfo(0);

      expect(info).not.toBeNull();
      expect(info!.partitionId).toBe(0);
      expect(info!.ownerNodeId).toBe('node-1');
      expect(Array.isArray(info!.backupNodeIds)).toBe(true);
    });
  });

  describe('isRelated / isLocalBackup', () => {
    beforeEach(() => {
      cluster.setMembers(['node-1', 'node-2']);
      cluster.emit('memberJoined', 'node-2');
    });

    it('should identify related partitions', () => {
      // For node-1, check all keys
      let relatedCount = 0;
      let ownerCount = 0;
      let backupCount = 0;

      for (let i = 0; i < PARTITION_COUNT; i++) {
        const key = `partition-${i}-key`;
        const partitionId = partitionService.getPartitionId(key);

        if (partitionService.isRelated(key)) {
          relatedCount++;
        }
        if (partitionService.isLocalOwner(key)) {
          ownerCount++;
        }
        if (partitionService.isLocalBackup(key)) {
          backupCount++;
        }
      }

      // With 2 nodes and 1 backup, node-1 should be related to all partitions
      // (owner of ~135 + backup of ~136)
      expect(ownerCount).toBeGreaterThan(100);
      expect(backupCount).toBeGreaterThan(100);
    });
  });
});
