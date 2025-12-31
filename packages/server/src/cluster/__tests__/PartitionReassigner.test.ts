/**
 * PartitionReassigner Unit Tests
 *
 * Tests automatic partition failover:
 * - Backup promotion on node failure
 * - New backup assignment
 * - Failover status tracking
 * - Event emissions
 */

import { EventEmitter } from 'events';
import { PartitionReassigner, DEFAULT_REASSIGNER_CONFIG } from '../PartitionReassigner';
import { PartitionService } from '../PartitionService';
import { ClusterManager } from '../ClusterManager';
import { PARTITION_COUNT } from '@topgunbuild/core';

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

  get config() {
    return {
      nodeId: 'node-1',
      host: 'localhost',
      port: 10001,
    };
  }
}

// Mock PartitionService
class MockPartitionService {
  private owners: Map<number, string> = new Map();
  private backups: Map<number, string[]> = new Map();
  private members: string[];

  constructor(members: string[]) {
    this.members = members;
    this.initializePartitions();
  }

  private initializePartitions() {
    const sortedMembers = [...this.members].sort();
    for (let i = 0; i < PARTITION_COUNT; i++) {
      const ownerIndex = i % sortedMembers.length;
      this.owners.set(i, sortedMembers[ownerIndex]);

      // Assign backups
      const backupList: string[] = [];
      if (sortedMembers.length > 1) {
        for (let b = 1; b <= 2 && b < sortedMembers.length; b++) {
          const backupIndex = (ownerIndex + b) % sortedMembers.length;
          backupList.push(sortedMembers[backupIndex]);
        }
      }
      this.backups.set(i, backupList);
    }
  }

  getPartitionMap() {
    const partitions = [];
    for (let i = 0; i < PARTITION_COUNT; i++) {
      partitions.push({
        partitionId: i,
        ownerNodeId: this.owners.get(i) || 'unknown',
        backupNodeIds: this.backups.get(i) || [],
      });
    }
    return {
      version: 1,
      partitionCount: PARTITION_COUNT,
      nodes: this.members.map(m => ({ nodeId: m, endpoints: { websocket: '' }, status: 'ACTIVE' })),
      partitions,
      generatedAt: Date.now(),
    };
  }

  getBackups(partitionId: number): string[] {
    return this.backups.get(partitionId) || [];
  }

  setOwner(partitionId: number, nodeId: string) {
    this.owners.set(partitionId, nodeId);
  }

  getPartitionOwner(partitionId: number): string | null {
    return this.owners.get(partitionId) || null;
  }

  getPartitionsForNode(nodeId: string): number[] {
    const partitions: number[] = [];
    for (const [pid, owner] of this.owners) {
      if (owner === nodeId) {
        partitions.push(pid);
      }
    }
    return partitions;
  }
}

// Helper to flush all pending promises
const flushPromises = () => new Promise(resolve => setTimeout(resolve, 0));

describe('PartitionReassigner', () => {
  let clusterManager: MockClusterManager;
  let partitionService: MockPartitionService;
  let reassigner: PartitionReassigner;

  beforeEach(() => {
    clusterManager = new MockClusterManager(['node-1', 'node-2', 'node-3']);
    partitionService = new MockPartitionService(['node-1', 'node-2', 'node-3']);
    reassigner = new PartitionReassigner(
      clusterManager as unknown as ClusterManager,
      partitionService as unknown as PartitionService,
      { reassignmentDelayMs: 0 } // Use 0 delay for faster tests
    );
  });

  afterEach(() => {
    reassigner.stop();
  });

  describe('initialization', () => {
    test('should initialize with default config', () => {
      const defaultReassigner = new PartitionReassigner(
        clusterManager as unknown as ClusterManager,
        partitionService as unknown as PartitionService
      );

      const status = defaultReassigner.getStatus();
      expect(status.inProgress).toBe(false);
      expect(status.partitionsReassigned).toBe(0);
      defaultReassigner.stop();
    });

    test('should not be in failover initially', () => {
      expect(reassigner.isFailoverInProgress()).toBe(false);
    });
  });

  describe('node failure handling', () => {
    test('should trigger failover on nodeConfirmedFailed event', async () => {
      const completeHandler = jest.fn();
      reassigner.on('failoverComplete', completeHandler);

      // Simulate node-2 failure
      clusterManager.setMembers(['node-1', 'node-3']);
      clusterManager.emit('nodeConfirmedFailed', 'node-2');

      // Wait for async operations
      await flushPromises();

      expect(completeHandler).toHaveBeenCalled();
    });

    test('should promote backup to owner', async () => {
      // Find a partition owned by node-2
      const node2Partitions = partitionService.getPartitionsForNode('node-2');
      expect(node2Partitions.length).toBeGreaterThan(0);

      const testPartition = node2Partitions[0];

      // Simulate failure
      clusterManager.setMembers(['node-1', 'node-3']);
      clusterManager.emit('nodeConfirmedFailed', 'node-2');

      await flushPromises();

      // Check that a backup was promoted
      const newOwner = partitionService.getPartitionOwner(testPartition);
      expect(newOwner).not.toBe('node-2');
      expect(['node-1', 'node-3']).toContain(newOwner);
    });
  });

  describe('failover status', () => {
    test('should track failover progress', () => {
      const status = reassigner.getStatus();
      expect(status).toEqual({
        inProgress: false,
        failedNodeId: undefined,
        partitionsReassigned: 0,
        partitionsPending: 0,
        startedAt: undefined,
        completedAt: expect.any(Number),
      });
    });

    test('should update status during failover', async () => {
      const completeHandler = jest.fn();
      reassigner.on('failoverComplete', completeHandler);

      clusterManager.setMembers(['node-1', 'node-3']);
      clusterManager.emit('nodeConfirmedFailed', 'node-2');

      await flushPromises();

      // Verify failover completed
      expect(completeHandler).toHaveBeenCalled();
      const status = reassigner.getStatus();
      expect(status.partitionsReassigned).toBeGreaterThan(0);
    });
  });

  describe('event emissions', () => {
    test('should emit reassignment events', async () => {
      const reassignmentHandler = jest.fn();
      reassigner.on('reassignment', reassignmentHandler);

      clusterManager.setMembers(['node-1', 'node-3']);
      clusterManager.emit('nodeConfirmedFailed', 'node-2');

      await flushPromises();

      expect(reassignmentHandler).toHaveBeenCalled();
      const event = reassignmentHandler.mock.calls[0][0];
      expect(event.type).toBe('backup-promoted');
      expect(event.previousOwner).toBe('node-2');
    });

    test('should emit failoverComplete event', async () => {
      const completeHandler = jest.fn();
      reassigner.on('failoverComplete', completeHandler);

      clusterManager.setMembers(['node-1', 'node-3']);
      clusterManager.emit('nodeConfirmedFailed', 'node-2');

      await flushPromises();

      expect(completeHandler).toHaveBeenCalled();
      const event = completeHandler.mock.calls[0][0];
      expect(event.failedNodeId).toBe('node-2');
      expect(event.partitionsReassigned).toBeGreaterThan(0);
    });

    test('should emit partitionsReassigned event with changes', async () => {
      const partitionsHandler = jest.fn();
      reassigner.on('partitionsReassigned', partitionsHandler);

      clusterManager.setMembers(['node-1', 'node-3']);
      clusterManager.emit('nodeConfirmedFailed', 'node-2');

      await flushPromises();

      expect(partitionsHandler).toHaveBeenCalled();
      const event = partitionsHandler.mock.calls[0][0];
      expect(event.failedNodeId).toBe('node-2');
      expect(event.changes.length).toBeGreaterThan(0);
      expect(event.partitionMap).toBeDefined();
    });
  });

  describe('force reassignment', () => {
    test('should immediately execute failover on forceReassignment', async () => {
      const completeHandler = jest.fn();
      reassigner.on('failoverComplete', completeHandler);

      clusterManager.setMembers(['node-1', 'node-3']);
      reassigner.forceReassignment('node-2');

      await flushPromises();

      expect(completeHandler).toHaveBeenCalled();
    });
  });

  describe('stop', () => {
    test('should prevent pending reassignment after stop', async () => {
      // Create a reassigner with a delay
      const delayedReassigner = new PartitionReassigner(
        clusterManager as unknown as ClusterManager,
        partitionService as unknown as PartitionService,
        { reassignmentDelayMs: 100 }
      );

      const completeHandler = jest.fn();
      delayedReassigner.on('failoverComplete', completeHandler);

      clusterManager.setMembers(['node-1', 'node-3']);
      clusterManager.emit('nodeConfirmedFailed', 'node-2');

      // Stop immediately before the delay expires
      delayedReassigner.stop();

      // Wait longer than the delay
      await new Promise(resolve => setTimeout(resolve, 150));

      // Should not have called complete handler since we stopped
      expect(completeHandler).not.toHaveBeenCalled();
    });
  });

  describe('no orphaned partitions', () => {
    test('should complete quickly when no partitions to reassign', async () => {
      const completeHandler = jest.fn();
      reassigner.on('failoverComplete', completeHandler);

      // node-4 doesn't own any partitions
      clusterManager.emit('nodeConfirmedFailed', 'node-4');

      await flushPromises();

      expect(completeHandler).toHaveBeenCalled();
      expect(completeHandler.mock.calls[0][0].partitionsReassigned).toBe(0);
    });
  });

  describe('single remaining node', () => {
    test('should reassign to last remaining node', async () => {
      clusterManager.setMembers(['node-1']);
      partitionService = new MockPartitionService(['node-1', 'node-2']);
      reassigner = new PartitionReassigner(
        clusterManager as unknown as ClusterManager,
        partitionService as unknown as PartitionService,
        { reassignmentDelayMs: 0 }
      );

      const completeHandler = jest.fn();
      reassigner.on('failoverComplete', completeHandler);

      clusterManager.emit('nodeConfirmedFailed', 'node-2');

      await flushPromises();

      expect(completeHandler).toHaveBeenCalled();

      // All partitions should now be owned by node-1
      const node1Partitions = partitionService.getPartitionsForNode('node-1');
      expect(node1Partitions.length).toBe(PARTITION_COUNT);
    });
  });
});
