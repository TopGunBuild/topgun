/**
 * Tests for ReplicationPipeline and LagTracker
 * Phase 4 Task 04: Async Replication Pipeline
 */

import { ReplicationPipeline, ReplicationTimeoutError } from '../cluster/ReplicationPipeline';
import { LagTracker } from '../cluster/LagTracker';
import { PartitionService, PartitionDistribution } from '../cluster/PartitionService';
import { ClusterManager } from '../cluster/ClusterManager';
import { ConsistencyLevel, PARTITION_COUNT } from '@topgunbuild/core';
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
  private members: string[] = ['node-1', 'node-2', 'node-3'];
  public sentMessages: Array<{ nodeId: string; type: string; payload: any }> = [];

  getMembers(): string[] {
    return this.members;
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
    // Initialize partitions with backups
    for (let i = 0; i < PARTITION_COUNT; i++) {
      this.partitions.set(i, {
        owner: 'node-1',
        backups: ['node-2', 'node-3'],
      });
    }
  }

  getPartitionId(key: string): number {
    let hash = 0;
    for (let i = 0; i < key.length; i++) {
      hash = (hash * 31 + key.charCodeAt(i)) % PARTITION_COUNT;
    }
    return Math.abs(hash);
  }

  getBackups(partitionId: number): string[] {
    return this.partitions.get(partitionId)?.backups ?? [];
  }

  setBackups(partitionId: number, backups: string[]): void {
    const dist = this.partitions.get(partitionId);
    if (dist) {
      dist.backups = backups;
    }
  }
}

describe('LagTracker', () => {
  let lagTracker: LagTracker;

  beforeEach(() => {
    lagTracker = new LagTracker({
      historySize: 10,
      laggyThresholdMs: 1000,
      unhealthyThresholdMs: 5000,
    });
  });

  describe('update', () => {
    it('should track lag for a node', () => {
      lagTracker.update('node-1', 100);

      const lag = lagTracker.getLag('node-1');
      expect(lag.current).toBe(100);
      expect(lag.avg).toBe(100);
      expect(lag.max).toBe(100);
    });

    it('should maintain history', () => {
      for (let i = 0; i < 5; i++) {
        lagTracker.update('node-1', i * 100);
      }

      const lag = lagTracker.getLag('node-1');
      expect(lag.max).toBe(400);
      expect(lag.avg).toBe(200); // (0+100+200+300+400)/5
    });

    it('should respect history size limit', () => {
      for (let i = 0; i < 20; i++) {
        lagTracker.update('node-1', i * 10);
      }

      const info = lagTracker.getRawLagInfo('node-1');
      expect(info?.history.length).toBe(10); // Max history size
    });
  });

  describe('recordAck', () => {
    it('should reset current lag to 0', () => {
      lagTracker.update('node-1', 500);
      lagTracker.recordAck('node-1');

      const lag = lagTracker.getLag('node-1');
      expect(lag.current).toBe(0);
    });

    it('should decrement pending ops', () => {
      lagTracker.incrementPending('node-1');
      lagTracker.incrementPending('node-1');
      expect(lagTracker.getPendingOps('node-1')).toBe(2);

      lagTracker.recordAck('node-1');
      expect(lagTracker.getPendingOps('node-1')).toBe(1);
    });
  });

  describe('getHealth', () => {
    it('should report healthy when all nodes are good', () => {
      lagTracker.update('node-1', 100);
      lagTracker.update('node-2', 200);

      const health = lagTracker.getHealth();
      expect(health.healthy).toBe(true);
      expect(health.unhealthyNodes).toEqual([]);
      expect(health.laggyNodes).toEqual([]);
    });

    it('should identify laggy nodes', () => {
      lagTracker.update('node-1', 100);
      lagTracker.update('node-2', 2000); // Exceeds 1000ms threshold

      const health = lagTracker.getHealth();
      expect(health.laggyNodes).toContain('node-2');
    });
  });

  describe('isNodeHealthy / isNodeLaggy', () => {
    it('should correctly identify node status', () => {
      lagTracker.update('node-1', 100);
      lagTracker.update('node-2', 2000);

      expect(lagTracker.isNodeHealthy('node-1')).toBe(true);
      expect(lagTracker.isNodeLaggy('node-1')).toBe(false);
      expect(lagTracker.isNodeLaggy('node-2')).toBe(true);
    });
  });

  describe('percentile calculation', () => {
    it('should calculate 99th percentile correctly', () => {
      // Add 100 samples
      for (let i = 1; i <= 100; i++) {
        lagTracker.update('node-1', i);
      }

      const lag = lagTracker.getLag('node-1');
      // With history limited to 10, we get samples 91-100
      // 99th percentile of [91,92,93,94,95,96,97,98,99,100] should be ~100
      expect(lag.percentile99).toBeGreaterThanOrEqual(99);
    });
  });

  describe('toPrometheusMetrics', () => {
    it('should generate valid Prometheus format', () => {
      lagTracker.update('node-1', 100);
      lagTracker.update('node-2', 200);

      const metrics = lagTracker.toPrometheusMetrics();

      expect(metrics).toContain('topgun_replication_lag_ms');
      expect(metrics).toContain('node="node-1"');
      expect(metrics).toContain('node="node-2"');
      expect(metrics).toContain('topgun_replication_healthy');
    });
  });

  describe('removeNode', () => {
    it('should remove node from tracking', () => {
      lagTracker.update('node-1', 100);
      lagTracker.removeNode('node-1');

      expect(lagTracker.getTrackedNodes()).not.toContain('node-1');
    });
  });
});

describe('ReplicationPipeline', () => {
  let cluster: MockClusterManager;
  let partitionService: MockPartitionService;
  let pipeline: ReplicationPipeline;

  beforeEach(() => {
    cluster = new MockClusterManager();
    partitionService = new MockPartitionService();
    pipeline = new ReplicationPipeline(
      cluster as unknown as ClusterManager,
      partitionService as unknown as PartitionService,
      {
        defaultConsistency: ConsistencyLevel.EVENTUAL,
        queueSizeLimit: 100,
        batchSize: 10,
        batchIntervalMs: 50,
        ackTimeoutMs: 1000,
        maxRetries: 2,
      }
    );
  });

  afterEach(() => {
    pipeline.close();
  });

  describe('replicate with EVENTUAL consistency', () => {
    it('should return immediately', async () => {
      const startTime = Date.now();

      const result = await pipeline.replicate(
        { data: 'test' },
        'op-1',
        'test-key',
        { consistency: ConsistencyLevel.EVENTUAL }
      );

      const elapsed = Date.now() - startTime;

      expect(result.success).toBe(true);
      expect(result.ackedBy).toContain('node-1');
      expect(elapsed).toBeLessThan(100); // Should be very fast
    });

    it('should queue operations for batch processing', async () => {
      await pipeline.replicate({ data: 'test1' }, 'op-1', 'key-1');
      await pipeline.replicate({ data: 'test2' }, 'op-2', 'key-2');

      const total = pipeline.getTotalPending();
      // Each replicate to 2 backups = 4 queued items
      expect(total).toBeGreaterThan(0);
    });
  });

  describe('replicate with STRONG consistency', () => {
    it('should wait for all backups to acknowledge', async () => {
      const replicatePromise = pipeline.replicate(
        { data: 'test' },
        'op-1',
        'test-key',
        { consistency: ConsistencyLevel.STRONG }
      );

      // Wait for messages to be sent
      await new Promise((r) => setTimeout(r, 10));

      // Simulate acks from all backups
      cluster.emit('message', {
        type: 'OP_FORWARD',
        senderId: 'node-2',
        payload: {
          _replication: {
            type: 'REPLICATION_ACK',
            payload: { opId: 'op-1', success: true, timestamp: Date.now() },
          },
        },
      });

      cluster.emit('message', {
        type: 'OP_FORWARD',
        senderId: 'node-3',
        payload: {
          _replication: {
            type: 'REPLICATION_ACK',
            payload: { opId: 'op-1', success: true, timestamp: Date.now() },
          },
        },
      });

      const result = await replicatePromise;

      expect(result.success).toBe(true);
      expect(result.ackedBy).toContain('node-1');
      expect(result.ackedBy).toContain('node-2');
      expect(result.ackedBy).toContain('node-3');
    });

    it('should timeout if backups do not acknowledge', async () => {
      const replicatePromise = pipeline.replicate(
        { data: 'test' },
        'op-timeout',
        'test-key',
        { consistency: ConsistencyLevel.STRONG, timeout: 100 }
      );

      // Don't send any acks

      await expect(replicatePromise).rejects.toThrow(ReplicationTimeoutError);
    });
  });

  describe('replicate with QUORUM consistency', () => {
    it('should succeed when majority acknowledges', async () => {
      const replicatePromise = pipeline.replicate(
        { data: 'test' },
        'op-1',
        'test-key',
        { consistency: ConsistencyLevel.QUORUM }
      );

      // Wait for messages to be sent
      await new Promise((r) => setTimeout(r, 10));

      // Simulate ack from only one backup (majority of 2 backups is 2, but we need quorum)
      // With 2 backups, quorum = floor(2/2) + 1 = 2
      cluster.emit('message', {
        type: 'OP_FORWARD',
        senderId: 'node-2',
        payload: {
          _replication: {
            type: 'REPLICATION_ACK',
            payload: { opId: 'op-1', success: true, timestamp: Date.now() },
          },
        },
      });

      cluster.emit('message', {
        type: 'OP_FORWARD',
        senderId: 'node-3',
        payload: {
          _replication: {
            type: 'REPLICATION_ACK',
            payload: { opId: 'op-1', success: true, timestamp: Date.now() },
          },
        },
      });

      const result = await replicatePromise;
      expect(result.success).toBe(true);
    });
  });

  describe('replicate with no backups', () => {
    it('should succeed immediately', async () => {
      // Create a new partition service where all partitions have no backups
      const noBackupPartitionService = new MockPartitionService();
      for (let i = 0; i < PARTITION_COUNT; i++) {
        noBackupPartitionService.setBackups(i, []);
      }

      const noBackupPipeline = new ReplicationPipeline(
        cluster as unknown as ClusterManager,
        noBackupPartitionService as unknown as PartitionService,
        { ackTimeoutMs: 1000 }
      );

      const result = await noBackupPipeline.replicate(
        { data: 'test' },
        'op-1',
        'test-key',
        { consistency: ConsistencyLevel.STRONG }
      );

      expect(result.success).toBe(true);
      expect(result.ackedBy).toEqual(['node-1']);

      noBackupPipeline.close();
    });
  });

  describe('getLag', () => {
    it('should return lag statistics', async () => {
      // Send some operations and get acks
      await pipeline.replicate({ data: 'test' }, 'op-1', 'key-1');

      const lag = pipeline.getLag('node-2');
      expect(lag).toHaveProperty('current');
      expect(lag).toHaveProperty('avg');
      expect(lag).toHaveProperty('max');
      expect(lag).toHaveProperty('percentile99');
    });
  });

  describe('getHealth', () => {
    it('should return health status', () => {
      const health = pipeline.getHealth();

      expect(health).toHaveProperty('healthy');
      expect(health).toHaveProperty('unhealthyNodes');
      expect(health).toHaveProperty('laggyNodes');
      expect(health).toHaveProperty('avgLagMs');
    });
  });

  describe('isSynced', () => {
    it('should return true for low-lag nodes', () => {
      // Simulate ack to set low lag
      const lagTracker = pipeline.getLagTracker();
      lagTracker.update('node-2', 100);

      expect(pipeline.isSynced('node-2', 1000)).toBe(true);
    });

    it('should return false for high-lag nodes', () => {
      const lagTracker = pipeline.getLagTracker();
      lagTracker.update('node-2', 5000);

      expect(pipeline.isSynced('node-2', 1000)).toBe(false);
    });
  });

  describe('queue management', () => {
    it('should emit queueOverflow when limit exceeded', async () => {
      // Create pipeline with tiny queue
      const tinyPipeline = new ReplicationPipeline(
        cluster as unknown as ClusterManager,
        partitionService as unknown as PartitionService,
        {
          queueSizeLimit: 2,
          batchSize: 100, // Large batch so queue doesn't drain
          batchIntervalMs: 10000, // Long interval so queue doesn't process
        }
      );

      let overflowCount = 0;
      tinyPipeline.on('queueOverflow', () => {
        overflowCount++;
      });

      // Fill queue - each replicate goes to 2 backups, so 5 replicates = 10 items
      // With queueSizeLimit: 2, we should see overflow for each queue
      for (let i = 0; i < 5; i++) {
        await tinyPipeline.replicate({ data: i }, `op-${i}`, `key-${i}`);
      }

      // Should have emitted overflow events
      expect(overflowCount).toBeGreaterThan(0);

      tinyPipeline.close();
    });
  });

  describe('message handling', () => {
    it('should handle incoming REPLICATION message', () => {
      // Simulate receiving replication from another node
      cluster.emit('message', {
        type: 'OP_FORWARD',
        senderId: 'node-2',
        payload: {
          _replication: {
            type: 'REPLICATION',
            payload: {
              opId: 'op-from-node-2',
              operation: { data: 'test' },
              consistency: ConsistencyLevel.STRONG,
            },
          },
        },
      });

      // Should send ack back
      const ackMessage = cluster.sentMessages.find(
        (m) => m.payload?._replication?.type === 'REPLICATION_ACK'
      );
      expect(ackMessage).toBeDefined();
    });

    it('should handle incoming REPLICATION_BATCH message', () => {
      cluster.emit('message', {
        type: 'OP_FORWARD',
        senderId: 'node-2',
        payload: {
          _replication: {
            type: 'REPLICATION_BATCH',
            payload: {
              operations: [{ data: 1 }, { data: 2 }],
              opIds: ['op-1', 'op-2'],
            },
          },
        },
      });

      // Should send batch ack back
      const ackMessage = cluster.sentMessages.find(
        (m) => m.payload?._replication?.type === 'REPLICATION_BATCH_ACK'
      );
      expect(ackMessage).toBeDefined();
    });
  });

  describe('toPrometheusMetrics', () => {
    it('should generate metrics', async () => {
      await pipeline.replicate({ data: 'test' }, 'op-1', 'key-1');

      const metrics = pipeline.toPrometheusMetrics();

      expect(metrics).toContain('topgun_replication_queue_size');
      expect(metrics).toContain('topgun_replication_pending_acks');
    });
  });

  describe('close', () => {
    it('should cleanup resources', () => {
      pipeline.close();

      expect(pipeline.getTotalPending()).toBe(0);
    });

    it('should reject pending acks', async () => {
      // Create a fresh pipeline for this test
      const closePipeline = new ReplicationPipeline(
        cluster as unknown as ClusterManager,
        partitionService as unknown as PartitionService,
        { ackTimeoutMs: 10000 } // Long timeout so we can close before it fires
      );

      const promise = closePipeline.replicate(
        { data: 'test' },
        'op-close-test',
        'test-key',
        { consistency: ConsistencyLevel.STRONG }
      );

      // Don't wait for acks, just close
      await new Promise((r) => setTimeout(r, 10));
      closePipeline.close();

      await expect(promise).rejects.toThrow('ReplicationPipeline closed');
    });
  });
});
