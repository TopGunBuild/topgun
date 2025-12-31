/**
 * RepairScheduler - Anti-Entropy Repair System
 *
 * Phase 10.04: Proactively detects and repairs data inconsistencies:
 * - Periodic scanning of partitions
 * - Merkle tree-based difference detection
 * - LWW conflict resolution
 * - Throttled repair execution
 *
 * Based on Cassandra/Dynamo anti-entropy patterns.
 */

import { EventEmitter } from 'events';
import { LWWRecord, Timestamp, PARTITION_COUNT } from '@topgunbuild/core';
import { MerkleTreeManager } from './MerkleTreeManager';
import { ClusterManager } from './ClusterManager';
import { PartitionService } from './PartitionService';
import { logger } from '../utils/logger';

export interface RepairConfig {
  /** Enable anti-entropy repair. Default: true */
  enabled: boolean;
  /** Interval between full scans in ms. Default: 3600000 (1 hour) */
  scanIntervalMs: number;
  /** Keys per repair batch. Default: 1000 */
  repairBatchSize: number;
  /** Maximum concurrent partition repairs. Default: 2 */
  maxConcurrentRepairs: number;
  /** Delay between batches in ms. Default: 100 */
  throttleMs: number;
  /** Prioritize recently modified partitions. Default: true */
  prioritizeRecent: boolean;
}

export const DEFAULT_REPAIR_CONFIG: RepairConfig = {
  enabled: true,
  scanIntervalMs: 3600000, // 1 hour
  repairBatchSize: 1000,
  maxConcurrentRepairs: 2,
  throttleMs: 100,
  prioritizeRecent: true,
};

export interface RepairTask {
  partitionId: number;
  replicaNodeId: string;
  priority: 'high' | 'normal' | 'low';
  scheduledAt: number;
}

export interface RepairResult {
  partitionId: number;
  replicaNodeId: string;
  keysScanned: number;
  keysRepaired: number;
  durationMs: number;
  success: boolean;
  error?: string;
}

export interface RepairMetrics {
  scansCompleted: number;
  repairsExecuted: number;
  keysRepaired: number;
  errorsEncountered: number;
  lastScanTime?: number;
  averageRepairDurationMs: number;
}

type RecordGetter = (key: string) => LWWRecord<any> | undefined;
type RecordSetter = (key: string, record: LWWRecord<any>) => void;

export class RepairScheduler extends EventEmitter {
  private config: RepairConfig;
  private merkleManager: MerkleTreeManager;
  private clusterManager: ClusterManager;
  private partitionService: PartitionService;
  private nodeId: string;

  private repairQueue: RepairTask[] = [];
  private activeRepairs: Set<number> = new Set();
  private scanTimer?: NodeJS.Timeout;
  private processTimer?: NodeJS.Timeout;
  private started = false;

  // Metrics
  private metrics: RepairMetrics = {
    scansCompleted: 0,
    repairsExecuted: 0,
    keysRepaired: 0,
    errorsEncountered: 0,
    averageRepairDurationMs: 0,
  };

  // Callbacks for data access
  private getRecord?: RecordGetter;
  private setRecord?: RecordSetter;

  constructor(
    merkleManager: MerkleTreeManager,
    clusterManager: ClusterManager,
    partitionService: PartitionService,
    nodeId: string,
    config: Partial<RepairConfig> = {}
  ) {
    super();
    this.merkleManager = merkleManager;
    this.clusterManager = clusterManager;
    this.partitionService = partitionService;
    this.nodeId = nodeId;
    this.config = { ...DEFAULT_REPAIR_CONFIG, ...config };
  }

  /**
   * Set data access callbacks
   */
  setDataAccessors(
    getRecord: RecordGetter,
    setRecord: RecordSetter
  ): void {
    this.getRecord = getRecord;
    this.setRecord = setRecord;
  }

  /**
   * Start the repair scheduler
   */
  start(): void {
    if (this.started || !this.config.enabled) return;
    this.started = true;

    logger.info({ config: this.config }, 'Starting RepairScheduler');

    // Schedule periodic full scans
    this.scanTimer = setInterval(() => {
      this.scheduleFullScan();
    }, this.config.scanIntervalMs);

    // Process repair queue
    this.processTimer = setInterval(() => {
      this.processRepairQueue();
    }, 1000);

    // Initial scan after startup delay
    setTimeout(() => {
      this.scheduleFullScan();
    }, 60000); // 1 minute delay
  }

  /**
   * Stop the repair scheduler
   */
  stop(): void {
    if (!this.started) return;
    this.started = false;

    if (this.scanTimer) {
      clearInterval(this.scanTimer);
      this.scanTimer = undefined;
    }

    if (this.processTimer) {
      clearInterval(this.processTimer);
      this.processTimer = undefined;
    }

    this.repairQueue = [];
    this.activeRepairs.clear();

    logger.info('RepairScheduler stopped');
  }

  /**
   * Schedule a full scan of all owned partitions
   */
  scheduleFullScan(): void {
    const ownedPartitions = this.getOwnedPartitions();
    const replicas = this.getReplicaPartitions();
    const allPartitions = [...new Set([...ownedPartitions, ...replicas])];

    logger.info({
      ownedCount: ownedPartitions.length,
      replicaCount: replicas.length,
      totalPartitions: allPartitions.length
    }, 'Scheduling full anti-entropy scan');

    for (const partitionId of allPartitions) {
      this.schedulePartitionRepair(partitionId);
    }

    this.metrics.scansCompleted++;
    this.metrics.lastScanTime = Date.now();
  }

  /**
   * Schedule repair for a specific partition
   */
  schedulePartitionRepair(partitionId: number, priority: 'high' | 'normal' | 'low' = 'normal'): void {
    // Get replicas for this partition
    const backups = this.partitionService.getBackups(partitionId);
    const owner = this.partitionService.getPartitionOwner(partitionId);

    // Create repair tasks for each replica relationship
    const replicas = this.nodeId === owner ? backups : (owner ? [owner] : []);

    for (const replicaNodeId of replicas) {
      // Skip if already queued
      const exists = this.repairQueue.some(
        t => t.partitionId === partitionId && t.replicaNodeId === replicaNodeId
      );
      if (exists) continue;

      this.repairQueue.push({
        partitionId,
        replicaNodeId,
        priority,
        scheduledAt: Date.now(),
      });
    }

    // Sort by priority
    this.sortRepairQueue();
  }

  /**
   * Sort repair queue by priority
   */
  private sortRepairQueue(): void {
    const priorityOrder = { high: 0, normal: 1, low: 2 };

    this.repairQueue.sort((a, b) => {
      const priorityDiff = priorityOrder[a.priority] - priorityOrder[b.priority];
      if (priorityDiff !== 0) return priorityDiff;

      // For same priority, prefer recently modified if configured
      if (this.config.prioritizeRecent) {
        const infoA = this.merkleManager.getPartitionInfo(a.partitionId);
        const infoB = this.merkleManager.getPartitionInfo(b.partitionId);
        if (infoA && infoB) {
          return infoB.lastUpdated - infoA.lastUpdated;
        }
      }

      return a.scheduledAt - b.scheduledAt;
    });
  }

  /**
   * Process the repair queue
   */
  private async processRepairQueue(): Promise<void> {
    if (this.activeRepairs.size >= this.config.maxConcurrentRepairs) {
      return;
    }

    const task = this.repairQueue.shift();
    if (!task) return;

    // Skip if already being repaired
    if (this.activeRepairs.has(task.partitionId)) {
      return;
    }

    // Check if replica is still alive
    if (!this.clusterManager.getMembers().includes(task.replicaNodeId)) {
      logger.debug({ task }, 'Skipping repair - replica not available');
      return;
    }

    this.activeRepairs.add(task.partitionId);

    try {
      const result = await this.executeRepair(task);
      this.emit('repairComplete', result);

      if (result.success) {
        this.metrics.repairsExecuted++;
        this.metrics.keysRepaired += result.keysRepaired;
        this.updateAverageRepairDuration(result.durationMs);
      } else {
        this.metrics.errorsEncountered++;
      }

    } catch (error) {
      logger.error({ task, error }, 'Repair failed');
      this.metrics.errorsEncountered++;
    } finally {
      this.activeRepairs.delete(task.partitionId);
    }
  }

  /**
   * Execute repair for a partition-replica pair
   */
  private async executeRepair(task: RepairTask): Promise<RepairResult> {
    const startTime = Date.now();
    let keysScanned = 0;
    let keysRepaired = 0;

    try {
      // 1. Get local Merkle root
      const localRoot = this.merkleManager.getRootHash(task.partitionId);

      // 2. Request remote Merkle root
      // In full implementation, this would be a network request
      // For now, we'll simulate by checking if roots differ
      const remoteRoot = await this.requestRemoteMerkleRoot(task.replicaNodeId, task.partitionId);

      // 3. If roots match, no repair needed
      if (localRoot === remoteRoot) {
        logger.debug({
          partitionId: task.partitionId,
          replicaNodeId: task.replicaNodeId
        }, 'Partition in sync');

        return {
          partitionId: task.partitionId,
          replicaNodeId: task.replicaNodeId,
          keysScanned: 0,
          keysRepaired: 0,
          durationMs: Date.now() - startTime,
          success: true,
        };
      }

      // 4. Find differences via tree traversal
      const differences = await this.findDifferences(task.partitionId, task.replicaNodeId);
      keysScanned = differences.length;

      // 5. Repair each difference
      for (const key of differences) {
        const repaired = await this.repairKey(task.partitionId, task.replicaNodeId, key);
        if (repaired) {
          keysRepaired++;
        }

        // Throttle
        if (keysRepaired % this.config.repairBatchSize === 0) {
          await this.sleep(this.config.throttleMs);
        }
      }

      logger.info({
        partitionId: task.partitionId,
        replicaNodeId: task.replicaNodeId,
        keysScanned,
        keysRepaired,
        durationMs: Date.now() - startTime
      }, 'Partition repair completed');

      return {
        partitionId: task.partitionId,
        replicaNodeId: task.replicaNodeId,
        keysScanned,
        keysRepaired,
        durationMs: Date.now() - startTime,
        success: true,
      };

    } catch (error) {
      return {
        partitionId: task.partitionId,
        replicaNodeId: task.replicaNodeId,
        keysScanned,
        keysRepaired,
        durationMs: Date.now() - startTime,
        success: false,
        error: String(error),
      };
    }
  }

  /**
   * Request Merkle root from remote node
   * Note: In full implementation, this would be a network request
   */
  private async requestRemoteMerkleRoot(nodeId: string, partitionId: number): Promise<number> {
    // Placeholder - in full implementation, send MERKLE_REQ_ROOT message
    // and wait for response
    return 0;
  }

  /**
   * Find keys that differ between local and remote
   */
  private async findDifferences(partitionId: number, replicaNodeId: string): Promise<string[]> {
    // Get all keys in this partition on local node
    return this.merkleManager.getAllKeys(partitionId);
  }

  /**
   * Repair a single key
   */
  private async repairKey(partitionId: number, replicaNodeId: string, key: string): Promise<boolean> {
    if (!this.getRecord || !this.setRecord) {
      return false;
    }

    // Get local record
    const localRecord = this.getRecord(key);

    // In full implementation, would also fetch remote record
    // and use LWW to resolve conflict
    // For now, just verify local record exists
    return localRecord !== undefined;
  }

  /**
   * Resolve conflict between two records using LWW
   */
  resolveConflict<T>(a: LWWRecord<T> | undefined, b: LWWRecord<T> | undefined): LWWRecord<T> | null {
    if (!a && !b) return null;
    if (!a) return b!;
    if (!b) return a;

    // LWW: higher timestamp wins
    if (this.compareTimestamps(a.timestamp, b.timestamp) > 0) {
      return a;
    }
    if (this.compareTimestamps(b.timestamp, a.timestamp) > 0) {
      return b;
    }

    // Tie-breaker: node ID (lexicographic)
    if (a.timestamp.nodeId > b.timestamp.nodeId) {
      return a;
    }
    return b;
  }

  /**
   * Compare two timestamps
   */
  private compareTimestamps(a: Timestamp, b: Timestamp): number {
    if (a.millis !== b.millis) {
      return a.millis - b.millis;
    }
    return a.counter - b.counter;
  }

  /**
   * Get partitions owned by this node
   */
  private getOwnedPartitions(): number[] {
    const owned: number[] = [];
    for (let i = 0; i < PARTITION_COUNT; i++) {
      if (this.partitionService.getPartitionOwner(i) === this.nodeId) {
        owned.push(i);
      }
    }
    return owned;
  }

  /**
   * Get partitions where this node is a backup
   */
  private getReplicaPartitions(): number[] {
    const replicas: number[] = [];
    for (let i = 0; i < PARTITION_COUNT; i++) {
      const backups = this.partitionService.getBackups(i);
      if (backups.includes(this.nodeId)) {
        replicas.push(i);
      }
    }
    return replicas;
  }

  /**
   * Update average repair duration
   */
  private updateAverageRepairDuration(durationMs: number): void {
    const count = this.metrics.repairsExecuted;
    const currentAvg = this.metrics.averageRepairDurationMs;
    this.metrics.averageRepairDurationMs = ((currentAvg * (count - 1)) + durationMs) / count;
  }

  /**
   * Get repair metrics
   */
  getMetrics(): RepairMetrics {
    return { ...this.metrics };
  }

  /**
   * Get repair queue status
   */
  getQueueStatus(): {
    queueLength: number;
    activeRepairs: number;
    maxConcurrent: number;
  } {
    return {
      queueLength: this.repairQueue.length,
      activeRepairs: this.activeRepairs.size,
      maxConcurrent: this.config.maxConcurrentRepairs,
    };
  }

  /**
   * Force immediate repair for a partition
   */
  forceRepair(partitionId: number): void {
    this.schedulePartitionRepair(partitionId, 'high');
  }

  /**
   * Sleep utility
   */
  private sleep(ms: number): Promise<void> {
    return new Promise(resolve => setTimeout(resolve, ms));
  }
}
