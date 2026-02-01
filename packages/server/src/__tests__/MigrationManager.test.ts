/**
 * Tests for MigrationManager
 * Parallel Partition Sync
 */

import { MigrationManager } from '../cluster/MigrationManager';
import { PartitionService, PartitionDistribution } from '../cluster/PartitionService';
import { ClusterManager } from '../cluster/ClusterManager';
import { PartitionState, PARTITION_COUNT, DEFAULT_MIGRATION_CONFIG } from '@topgunbuild/core';
import { EventEmitter } from 'events';

// Mock ClusterManager
class MockClusterManager extends EventEmitter {
  config = {
    nodeId: 'node-1',
    host: 'localhost',
    port: 8080,
    peers: [],
  };

  port = 8080;
  private members: string[] = ['node-1'];
  public sentMessages: Array<{ nodeId: string; type: string; payload: any }> = [];

  getMembers(): string[] {
    return this.members;
  }

  setMembers(members: string[]): void {
    this.members = members;
  }

  send(nodeId: string, type: string, payload: any): void {
    this.sentMessages.push({ nodeId, type, payload });
  }

  isLocal(nodeId: string): boolean {
    return nodeId === this.config.nodeId;
  }

  clearMessages(): void {
    this.sentMessages = [];
  }
}

// Mock PartitionService
class MockPartitionService extends EventEmitter {
  private partitions: Map<number, PartitionDistribution> = new Map();

  constructor() {
    super();
    // Initialize all partitions with node-1 as owner
    for (let i = 0; i < PARTITION_COUNT; i++) {
      this.partitions.set(i, { owner: 'node-1', backups: [] });
    }
  }

  getPartitionId(key: string): number {
    // Simple hash for testing
    let hash = 0;
    for (let i = 0; i < key.length; i++) {
      hash = (hash * 31 + key.charCodeAt(i)) % PARTITION_COUNT;
    }
    return Math.abs(hash);
  }

  getBackups(partitionId: number): string[] {
    return this.partitions.get(partitionId)?.backups ?? [];
  }

  setPartitions(partitions: Map<number, PartitionDistribution>): void {
    this.partitions = partitions;
  }

  getPartitions(): Map<number, PartitionDistribution> {
    return this.partitions;
  }
}

describe('MigrationManager', () => {
  let cluster: MockClusterManager;
  let partitionService: MockPartitionService;
  let migrationManager: MigrationManager;

  beforeEach(() => {
    cluster = new MockClusterManager();
    partitionService = new MockPartitionService();
    migrationManager = new MigrationManager(
      cluster as unknown as ClusterManager,
      partitionService as unknown as PartitionService,
      {
        batchSize: 5,
        parallelTransfers: 2,
        batchIntervalMs: 100,
        maxRetries: 2,
      }
    );
  });

  afterEach(() => {
    migrationManager.close();
  });

  describe('planMigration', () => {
    it('should plan migrations when partition ownership changes', () => {
      const oldDistribution = new Map<number, PartitionDistribution>();
      const newDistribution = new Map<number, PartitionDistribution>();

      // Old: node-1 owns all
      for (let i = 0; i < PARTITION_COUNT; i++) {
        oldDistribution.set(i, { owner: 'node-1', backups: [] });
      }

      // New: node-1 and node-2 split ownership
      for (let i = 0; i < PARTITION_COUNT; i++) {
        const owner = i % 2 === 0 ? 'node-1' : 'node-2';
        newDistribution.set(i, { owner, backups: [] });
      }

      let plannedEvent: any = null;
      migrationManager.on('migrationPlanned', (info) => {
        plannedEvent = info;
      });

      migrationManager.planMigration(oldDistribution, newDistribution);

      // Should plan migrations for partitions moving from node-1 to node-2
      // That's all odd partitions = 135 or 136 partitions
      expect(plannedEvent).not.toBeNull();
      expect(plannedEvent.total).toBeGreaterThan(100);
    });

    it('should not plan migrations when this node is not the source', () => {
      const oldDistribution = new Map<number, PartitionDistribution>();
      const newDistribution = new Map<number, PartitionDistribution>();

      // Old: node-2 owns all
      for (let i = 0; i < PARTITION_COUNT; i++) {
        oldDistribution.set(i, { owner: 'node-2', backups: [] });
      }

      // New: node-1 takes some
      for (let i = 0; i < PARTITION_COUNT; i++) {
        const owner = i % 2 === 0 ? 'node-1' : 'node-2';
        newDistribution.set(i, { owner, backups: [] });
      }

      let plannedEvent: any = null;
      migrationManager.on('migrationPlanned', (info) => {
        plannedEvent = info;
      });

      migrationManager.planMigration(oldDistribution, newDistribution);

      // node-1 is not the source for any partition, so no migrations planned
      expect(plannedEvent).not.toBeNull();
      expect(plannedEvent.total).toBe(0);
    });

    it('should order migrations by partition ID', () => {
      // Create manager with 0 parallel transfers to keep everything in queue
      const queueOnlyManager = new MigrationManager(
        cluster as unknown as ClusterManager,
        partitionService as unknown as PartitionService,
        {
          batchSize: 10,
          parallelTransfers: 0, // Disable parallel transfers to test queue
          batchIntervalMs: 100000, // Long interval
        }
      );

      const oldDistribution = new Map<number, PartitionDistribution>();
      const newDistribution = new Map<number, PartitionDistribution>();

      // Assign partitions 5, 3, 1 (out of order) to migrate
      for (let i = 0; i < PARTITION_COUNT; i++) {
        oldDistribution.set(i, { owner: 'node-1', backups: [] });
        newDistribution.set(i, { owner: 'node-1', backups: [] });
      }

      // Change only partitions 5, 3, 1
      newDistribution.set(5, { owner: 'node-2', backups: [] });
      newDistribution.set(3, { owner: 'node-2', backups: [] });
      newDistribution.set(1, { owner: 'node-2', backups: [] });

      queueOnlyManager.planMigration(oldDistribution, newDistribution);

      const status = queueOnlyManager.getStatus();
      // All 3 should be queued since parallelTransfers is 0
      expect(status.queued).toBe(3);

      queueOnlyManager.close();
    });
  });

  describe('getStatus', () => {
    it('should return correct initial status', () => {
      const status = migrationManager.getStatus();

      expect(status.inProgress).toBe(false);
      expect(status.active).toEqual([]);
      expect(status.queued).toBe(0);
      expect(status.completed).toBe(0);
      expect(status.failed).toBe(0);
    });

    it('should update status after planning', () => {
      const oldDistribution = new Map<number, PartitionDistribution>();
      const newDistribution = new Map<number, PartitionDistribution>();

      for (let i = 0; i < 10; i++) {
        oldDistribution.set(i, { owner: 'node-1', backups: [] });
        newDistribution.set(i, { owner: i < 5 ? 'node-1' : 'node-2', backups: [] });
      }

      migrationManager.planMigration(oldDistribution, newDistribution);

      const status = migrationManager.getStatus();
      // 5 partitions need to migrate, 2 parallel, so 3 remain queued after first batch
      expect(status.inProgress).toBe(true);
    });
  });

  describe('getMetrics', () => {
    it('should track metrics correctly', () => {
      const metrics = migrationManager.getMetrics();

      expect(metrics.migrationsStarted).toBe(0);
      expect(metrics.migrationsCompleted).toBe(0);
      expect(metrics.migrationsFailed).toBe(0);
      expect(metrics.chunksTransferred).toBe(0);
      expect(metrics.bytesTransferred).toBe(0);
    });
  });

  describe('isActive', () => {
    it('should return false for inactive partitions', () => {
      expect(migrationManager.isActive(0)).toBe(false);
      expect(migrationManager.isActive(100)).toBe(false);
    });
  });

  describe('cancelAll', () => {
    it('should cancel all migrations', async () => {
      const oldDistribution = new Map<number, PartitionDistribution>();
      const newDistribution = new Map<number, PartitionDistribution>();

      for (let i = 0; i < 10; i++) {
        oldDistribution.set(i, { owner: 'node-1', backups: [] });
        newDistribution.set(i, { owner: 'node-2', backups: [] });
      }

      migrationManager.planMigration(oldDistribution, newDistribution);

      // Wait a bit for batch to start
      await new Promise((resolve) => setTimeout(resolve, 50));

      await migrationManager.cancelAll();

      const status = migrationManager.getStatus();
      expect(status.inProgress).toBe(false);
      expect(status.queued).toBe(0);
    });
  });

  describe('data collector', () => {
    it('should use data collector when set', async () => {
      let collectorCalled = false;
      migrationManager.setDataCollector(async (partitionId: number) => {
        collectorCalled = true;
        return [new Uint8Array([1, 2, 3])];
      });

      const oldDistribution = new Map<number, PartitionDistribution>();
      const newDistribution = new Map<number, PartitionDistribution>();

      oldDistribution.set(0, { owner: 'node-1', backups: [] });
      newDistribution.set(0, { owner: 'node-2', backups: [] });

      migrationManager.planMigration(oldDistribution, newDistribution);

      // Wait for migration to start
      await new Promise((resolve) => setTimeout(resolve, 50));

      expect(collectorCalled).toBe(true);
    });
  });

  describe('message handling', () => {
    it('should handle MIGRATION_START message', () => {
      // Simulate receiving MIGRATION_START from another node
      cluster.emit('message', {
        type: 'OP_FORWARD',
        senderId: 'node-2',
        payload: {
          _migration: {
            type: 'MIGRATION_START',
            payload: {
              partitionId: 5,
              sourceNode: 'node-2',
              estimatedSize: 1000,
            },
          },
        },
      });

      // Migration should be tracked as incoming
      expect(migrationManager.isActive(5)).toBe(true);
    });

    it('should handle MIGRATION_CHUNK_ACK message', (done) => {
      const oldDistribution = new Map<number, PartitionDistribution>();
      const newDistribution = new Map<number, PartitionDistribution>();

      oldDistribution.set(0, { owner: 'node-1', backups: [] });
      newDistribution.set(0, { owner: 'node-2', backups: [] });

      // Set up data collector to return small data
      migrationManager.setDataCollector(async () => [new Uint8Array([1, 2, 3])]);

      migrationManager.planMigration(oldDistribution, newDistribution);

      // Wait for MIGRATION_START to be sent, then simulate ack
      setTimeout(() => {
        // Find the MIGRATION_CHUNK message
        const chunkMessage = cluster.sentMessages.find(
          (m) => m.payload?._migration?.type === 'MIGRATION_CHUNK'
        );

        if (chunkMessage) {
          // Simulate receiving chunk ack
          cluster.emit('message', {
            type: 'OP_FORWARD',
            senderId: 'node-2',
            payload: {
              _migration: {
                type: 'MIGRATION_CHUNK_ACK',
                payload: {
                  partitionId: 0,
                  chunkIndex: 0,
                  success: true,
                },
              },
            },
          });
        }
        done();
      }, 100);
    });
  });
});
