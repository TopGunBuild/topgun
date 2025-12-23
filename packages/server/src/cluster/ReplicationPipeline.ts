/**
 * ReplicationPipeline - Manages async replication with configurable consistency levels
 *
 * Phase 4 Task 04: Async Replication Pipeline
 *
 * Features:
 * - Three consistency levels: STRONG, QUORUM, EVENTUAL
 * - Async replication queue for high throughput
 * - Backpressure handling with queue limits
 * - Retry logic for failed replications
 * - Integration with LagTracker for monitoring
 */

import { EventEmitter } from 'events';
import {
  ConsistencyLevel,
  ReplicationConfig,
  ReplicationTask,
  ReplicationResult,
  ReplicationHealth,
  ReplicationLag,
  DEFAULT_REPLICATION_CONFIG,
} from '@topgunbuild/core';
import { ClusterManager, ClusterMessage } from './ClusterManager';
import { PartitionService } from './PartitionService';
import { LagTracker } from './LagTracker';
import { logger } from '../utils/logger';

export interface PendingAck {
  opId: string;
  consistency: ConsistencyLevel;
  targetNodes: string[];
  ackedNodes: Set<string>;
  resolve: () => void;
  reject: (error: Error) => void;
  timeout: ReturnType<typeof setTimeout>;
  startTime: number;
}

export interface ReplicationPipelineEvents {
  'replicationComplete': (opId: string, ackedBy: string[]) => void;
  'replicationFailed': (opId: string, error: Error) => void;
  'queueOverflow': (nodeId: string) => void;
  'error': (error: Error) => void;
}

export class ReplicationTimeoutError extends Error {
  constructor(
    public readonly opId: string,
    public readonly targetNodes: string[],
    public readonly ackedNodes: string[]
  ) {
    super(
      `Replication timeout for operation ${opId}. Expected: ${targetNodes.join(', ')}, Acked: ${ackedNodes.join(', ')}`
    );
    this.name = 'ReplicationTimeoutError';
  }
}

export class ReplicationPipeline extends EventEmitter {
  private readonly config: ReplicationConfig;
  private readonly clusterManager: ClusterManager;
  private readonly partitionService: PartitionService;
  private readonly lagTracker: LagTracker;
  private readonly nodeId: string;

  // Replication queues per node (for EVENTUAL mode)
  private replicationQueue: Map<string, ReplicationTask[]> = new Map();
  // Pending acknowledgments (for STRONG/QUORUM mode)
  private pendingAcks: Map<string, PendingAck> = new Map();
  // Queue processor timer
  private queueProcessorTimer: ReturnType<typeof setInterval> | null = null;

  constructor(
    clusterManager: ClusterManager,
    partitionService: PartitionService,
    config: Partial<ReplicationConfig> = {}
  ) {
    super();
    this.clusterManager = clusterManager;
    this.partitionService = partitionService;
    this.nodeId = clusterManager.config.nodeId;
    this.config = {
      ...DEFAULT_REPLICATION_CONFIG,
      ...config,
    };
    this.lagTracker = new LagTracker();

    this.setupMessageHandlers();
    this.startQueueProcessor();
  }

  // ============================================
  // Replication API
  // ============================================

  /**
   * Replicate operation to backup nodes
   */
  public async replicate(
    operation: unknown,
    opId: string,
    key: string,
    options: { consistency?: ConsistencyLevel; timeout?: number } = {}
  ): Promise<ReplicationResult> {
    const consistency = options.consistency ?? this.config.defaultConsistency;
    const partitionId = this.partitionService.getPartitionId(key);
    const backups = this.partitionService.getBackups(partitionId);

    if (backups.length === 0) {
      // No replicas, always succeeds
      return { success: true, ackedBy: [this.nodeId] };
    }

    switch (consistency) {
      case ConsistencyLevel.STRONG:
        return this.replicateStrong(operation, opId, backups, options.timeout);

      case ConsistencyLevel.QUORUM:
        return this.replicateQuorum(operation, opId, backups, options.timeout);

      case ConsistencyLevel.EVENTUAL:
        return this.replicateEventual(operation, opId, backups);
    }
  }

  /**
   * STRONG: Wait for all replicas to acknowledge
   */
  private async replicateStrong(
    operation: unknown,
    opId: string,
    backups: string[],
    timeout?: number
  ): Promise<ReplicationResult> {
    const targetNodes = backups;

    return new Promise((resolve, reject) => {
      const pending: PendingAck = {
        opId,
        consistency: ConsistencyLevel.STRONG,
        targetNodes,
        ackedNodes: new Set(),
        resolve: () =>
          resolve({
            success: true,
            ackedBy: [this.nodeId, ...targetNodes],
          }),
        reject: (error) => reject(error),
        timeout: setTimeout(() => {
          this.pendingAcks.delete(opId);
          const ackedList = Array.from(pending.ackedNodes);
          reject(new ReplicationTimeoutError(opId, targetNodes, ackedList));
        }, timeout ?? this.config.ackTimeoutMs),
        startTime: Date.now(),
      };

      this.pendingAcks.set(opId, pending);

      // Track pending ops
      for (const nodeId of targetNodes) {
        this.lagTracker.incrementPending(nodeId);
      }

      // Send to all backups
      for (const nodeId of targetNodes) {
        this.sendReplication(nodeId, operation, opId, ConsistencyLevel.STRONG);
      }
    });
  }

  /**
   * QUORUM: Wait for majority of replicas
   */
  private async replicateQuorum(
    operation: unknown,
    opId: string,
    backups: string[],
    timeout?: number
  ): Promise<ReplicationResult> {
    const targetNodes = backups;
    const quorumSize = Math.floor(targetNodes.length / 2) + 1;

    return new Promise((resolve, reject) => {
      const pending: PendingAck = {
        opId,
        consistency: ConsistencyLevel.QUORUM,
        targetNodes,
        ackedNodes: new Set(),
        resolve: () => {
          const ackedBy = [this.nodeId, ...pending.ackedNodes];
          resolve({ success: true, ackedBy });
        },
        reject: (error) => reject(error),
        timeout: setTimeout(() => {
          this.pendingAcks.delete(opId);
          const ackedList = Array.from(pending.ackedNodes);
          reject(new ReplicationTimeoutError(opId, targetNodes, ackedList));
        }, timeout ?? this.config.ackTimeoutMs),
        startTime: Date.now(),
      };

      this.pendingAcks.set(opId, pending);

      // Track pending ops
      for (const nodeId of targetNodes) {
        this.lagTracker.incrementPending(nodeId);
      }

      // Send to all backups
      for (const nodeId of targetNodes) {
        this.sendReplication(nodeId, operation, opId, ConsistencyLevel.QUORUM);
      }
    });
  }

  /**
   * EVENTUAL: Fire-and-forget with queue
   */
  private async replicateEventual(
    operation: unknown,
    opId: string,
    backups: string[]
  ): Promise<ReplicationResult> {
    // Add to replication queue for each backup
    for (const nodeId of backups) {
      this.enqueue(nodeId, {
        opId,
        operation,
        consistency: ConsistencyLevel.EVENTUAL,
        timestamp: Date.now(),
        retryCount: 0,
      });
    }

    // Return immediately
    return { success: true, ackedBy: [this.nodeId] };
  }

  // ============================================
  // Queue Management
  // ============================================

  /**
   * Add task to replication queue
   */
  private enqueue(nodeId: string, task: ReplicationTask): void {
    let queue = this.replicationQueue.get(nodeId);
    if (!queue) {
      queue = [];
      this.replicationQueue.set(nodeId, queue);
    }

    if (queue.length >= this.config.queueSizeLimit) {
      // Queue overflow - emit event and drop oldest
      this.emit('queueOverflow', nodeId);
      logger.warn({ nodeId, queueSize: queue.length }, 'Replication queue overflow, dropping oldest');
      queue.shift();
    }

    queue.push(task);
    this.lagTracker.incrementPending(nodeId);
  }

  /**
   * Start queue processor
   */
  private startQueueProcessor(): void {
    if (this.queueProcessorTimer) return;

    this.queueProcessorTimer = setInterval(() => {
      for (const nodeId of this.replicationQueue.keys()) {
        this.processQueue(nodeId).catch((err) => {
          logger.error({ nodeId, error: err }, 'Error processing replication queue');
          this.emit('error', err);
        });
      }
    }, this.config.batchIntervalMs);
  }

  /**
   * Stop queue processor
   */
  private stopQueueProcessor(): void {
    if (this.queueProcessorTimer) {
      clearInterval(this.queueProcessorTimer);
      this.queueProcessorTimer = null;
    }
  }

  /**
   * Process replication queue for a node
   */
  private async processQueue(nodeId: string): Promise<void> {
    const queue = this.replicationQueue.get(nodeId);
    if (!queue || queue.length === 0) return;

    // Batch up to config.batchSize operations
    const batch = queue.splice(0, this.config.batchSize);

    try {
      // Send batch to node
      this.clusterManager.send(nodeId, 'OP_FORWARD', {
        _replication: {
          type: 'REPLICATION_BATCH',
          payload: {
            operations: batch.map((t) => t.operation),
            opIds: batch.map((t) => t.opId),
          },
        },
      });

      // Update lag tracker with oldest timestamp in batch
      const oldestTimestamp = Math.min(...batch.map((t) => t.timestamp));
      this.lagTracker.update(nodeId, Date.now() - oldestTimestamp);

      logger.debug({ nodeId, batchSize: batch.length }, 'Sent replication batch');
    } catch (error) {
      // Requeue failed batch with retry increment
      for (const task of batch) {
        task.retryCount++;
        if (task.retryCount <= this.config.maxRetries) {
          queue.unshift(task); // Requeue at front
        } else {
          logger.warn({ nodeId, opId: task.opId, retries: task.retryCount }, 'Replication task exceeded max retries');
          this.emit('replicationFailed', task.opId, new Error('Max retries exceeded'));
        }
      }
    }
  }

  // ============================================
  // Message Handling
  // ============================================

  /**
   * Send replication message to a node
   */
  private sendReplication(
    nodeId: string,
    operation: unknown,
    opId: string,
    consistency: ConsistencyLevel
  ): void {
    this.clusterManager.send(nodeId, 'OP_FORWARD', {
      _replication: {
        type: 'REPLICATION',
        payload: {
          opId,
          operation,
          consistency,
        },
      },
    });
  }

  /**
   * Setup cluster message handlers
   */
  private setupMessageHandlers(): void {
    this.clusterManager.on('message', (msg: ClusterMessage) => {
      if (msg.payload?._replication) {
        const replication = msg.payload._replication;

        switch (replication.type) {
          case 'REPLICATION':
            this.handleReplication(msg.senderId, replication.payload);
            break;
          case 'REPLICATION_BATCH':
            this.handleReplicationBatch(msg.senderId, replication.payload);
            break;
          case 'REPLICATION_ACK':
            this.handleReplicationAck(msg.senderId, replication.payload);
            break;
          case 'REPLICATION_BATCH_ACK':
            this.handleReplicationBatchAck(msg.senderId, replication.payload);
            break;
        }
      }
    });
  }

  /**
   * Handle incoming replication request (on backup node)
   */
  private handleReplication(
    sourceNode: string,
    payload: { opId: string; operation: unknown; consistency: ConsistencyLevel }
  ): void {
    const { opId, operation, consistency } = payload;

    // TODO: Apply operation to local storage
    // This would be injected as a callback similar to MigrationManager
    logger.debug({ sourceNode, opId, consistency }, 'Received replication');

    // For STRONG/QUORUM, send acknowledgment
    if (consistency === ConsistencyLevel.STRONG || consistency === ConsistencyLevel.QUORUM) {
      this.clusterManager.send(sourceNode, 'OP_FORWARD', {
        _replication: {
          type: 'REPLICATION_ACK',
          payload: {
            opId,
            success: true,
            timestamp: Date.now(),
          },
        },
      });
    }
  }

  /**
   * Handle incoming batch replication (on backup node)
   */
  private handleReplicationBatch(
    sourceNode: string,
    payload: { operations: unknown[]; opIds: string[] }
  ): void {
    const { operations, opIds } = payload;

    // TODO: Apply operations to local storage
    logger.debug({ sourceNode, count: operations.length }, 'Received replication batch');

    // Send batch acknowledgment
    this.clusterManager.send(sourceNode, 'OP_FORWARD', {
      _replication: {
        type: 'REPLICATION_BATCH_ACK',
        payload: {
          opIds,
          success: true,
          timestamp: Date.now(),
        },
      },
    });
  }

  /**
   * Handle replication acknowledgment (on owner node)
   */
  private handleReplicationAck(
    sourceNode: string,
    payload: { opId: string; success: boolean; timestamp: number }
  ): void {
    const { opId, success } = payload;

    // Update lag tracker
    this.lagTracker.recordAck(sourceNode);

    const pending = this.pendingAcks.get(opId);
    if (!pending) return; // No pending ack or already resolved

    if (!success) {
      logger.warn({ sourceNode, opId }, 'Replication rejected by backup');
      return;
    }

    pending.ackedNodes.add(sourceNode);

    // Update lag with round-trip time
    const lag = Date.now() - pending.startTime;
    this.lagTracker.update(sourceNode, lag);

    // Check if we've met the consistency requirement
    const ackedCount = pending.ackedNodes.size;
    const targetCount = pending.targetNodes.length;

    switch (pending.consistency) {
      case ConsistencyLevel.STRONG:
        if (ackedCount === targetCount) {
          clearTimeout(pending.timeout);
          this.pendingAcks.delete(opId);
          pending.resolve();
          this.emit('replicationComplete', opId, [this.nodeId, ...pending.ackedNodes]);
        }
        break;

      case ConsistencyLevel.QUORUM:
        const quorumSize = Math.floor(targetCount / 2) + 1;
        if (ackedCount >= quorumSize) {
          clearTimeout(pending.timeout);
          this.pendingAcks.delete(opId);
          pending.resolve();
          this.emit('replicationComplete', opId, [this.nodeId, ...pending.ackedNodes]);
        }
        break;
    }
  }

  /**
   * Handle batch acknowledgment (on owner node)
   */
  private handleReplicationBatchAck(
    sourceNode: string,
    payload: { opIds: string[]; success: boolean; timestamp: number }
  ): void {
    const { success } = payload;

    // Update lag tracker
    this.lagTracker.recordAck(sourceNode);

    if (!success) {
      logger.warn({ sourceNode, count: payload.opIds.length }, 'Batch replication rejected');
    }
  }

  // ============================================
  // Status and Metrics
  // ============================================

  /**
   * Get replication lag for a specific node
   */
  public getLag(nodeId: string): ReplicationLag {
    return this.lagTracker.getLag(nodeId);
  }

  /**
   * Get overall replication health
   */
  public getHealth(): ReplicationHealth {
    return this.lagTracker.getHealth();
  }

  /**
   * Get queue size for a specific node
   */
  public getQueueSize(nodeId: string): number {
    return this.replicationQueue.get(nodeId)?.length ?? 0;
  }

  /**
   * Get total pending operations across all nodes
   */
  public getTotalPending(): number {
    let total = 0;
    for (const queue of this.replicationQueue.values()) {
      total += queue.length;
    }
    return total + this.pendingAcks.size;
  }

  /**
   * Check if a node is considered synced (low lag)
   */
  public isSynced(nodeId: string, maxLagMs: number = 1000): boolean {
    const lag = this.lagTracker.getLag(nodeId);
    return lag.current < maxLagMs;
  }

  /**
   * Get LagTracker for advanced monitoring
   */
  public getLagTracker(): LagTracker {
    return this.lagTracker;
  }

  /**
   * Export metrics in Prometheus format
   */
  public toPrometheusMetrics(): string {
    const lines: string[] = [];

    // Queue sizes
    lines.push('# HELP topgun_replication_queue_size Pending operations in replication queue');
    lines.push('# TYPE topgun_replication_queue_size gauge');
    for (const [nodeId, queue] of this.replicationQueue) {
      lines.push(`topgun_replication_queue_size{node="${nodeId}"} ${queue.length}`);
    }

    // Pending acks
    lines.push('');
    lines.push('# HELP topgun_replication_pending_acks Pending synchronous acknowledgments');
    lines.push('# TYPE topgun_replication_pending_acks gauge');
    lines.push(`topgun_replication_pending_acks ${this.pendingAcks.size}`);

    // Lag tracker metrics
    lines.push('');
    lines.push(this.lagTracker.toPrometheusMetrics());

    return lines.join('\n');
  }

  /**
   * Cleanup resources
   */
  public close(): void {
    this.stopQueueProcessor();

    // Reject all pending acks
    for (const [opId, pending] of this.pendingAcks) {
      clearTimeout(pending.timeout);
      pending.reject(new Error('ReplicationPipeline closed'));
    }
    this.pendingAcks.clear();

    // Clear queues
    this.replicationQueue.clear();

    // Clear lag tracker
    this.lagTracker.clear();
  }
}
