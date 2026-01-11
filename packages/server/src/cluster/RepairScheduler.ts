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
import { ClusterManager, ClusterMessage } from './ClusterManager';
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
  /** Timeout for network requests in ms. Default: 5000 */
  requestTimeoutMs: number;
}

export const DEFAULT_REPAIR_CONFIG: RepairConfig = {
  enabled: true,
  scanIntervalMs: 3600000, // 1 hour
  repairBatchSize: 1000,
  maxConcurrentRepairs: 2,
  throttleMs: 100,
  prioritizeRecent: true,
  requestTimeoutMs: 5000
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

interface PendingRequest {
  resolve: (value: any) => void;
  reject: (reason?: any) => void;
  timer: NodeJS.Timeout;
}

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
  private initialScanTimer?: NodeJS.Timeout;
  private started = false;

  // Pending network requests
  private pendingRequests: Map<string, PendingRequest> = new Map();

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

    this.setupNetworkHandlers();
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
   * Setup network message handlers
   */
  private setupNetworkHandlers(): void {
    this.clusterManager.on('message', (msg: ClusterMessage) => {
      this.handleClusterMessage(msg);
    });
  }

  /**
   * Handle incoming cluster messages
   */
  private handleClusterMessage(msg: ClusterMessage): void {
    switch (msg.type) {
      case 'CLUSTER_MERKLE_ROOT_REQ':
        this.handleMerkleRootReq(msg);
        break;
      case 'CLUSTER_MERKLE_ROOT_RESP':
        this.handleResponse(msg);
        break;
      case 'CLUSTER_MERKLE_BUCKETS_REQ':
        this.handleMerkleBucketsReq(msg);
        break;
      case 'CLUSTER_MERKLE_BUCKETS_RESP':
        this.handleResponse(msg);
        break;
      case 'CLUSTER_MERKLE_KEYS_REQ':
        this.handleMerkleKeysReq(msg);
        break;
      case 'CLUSTER_MERKLE_KEYS_RESP':
        this.handleResponse(msg);
        break;
      case 'CLUSTER_REPAIR_DATA_REQ':
        this.handleRepairDataReq(msg);
        break;
      case 'CLUSTER_REPAIR_DATA_RESP':
        this.handleResponse(msg);
        break;
    }
  }

  // === Request Handlers (Passive) ===

  private handleMerkleRootReq(msg: ClusterMessage): void {
    const { requestId, partitionId } = msg.payload;
    const rootHash = this.merkleManager.getRootHash(partitionId);
    
    this.clusterManager.send(msg.senderId, 'CLUSTER_MERKLE_ROOT_RESP', {
      requestId,
      partitionId,
      rootHash
    });
  }

  private handleMerkleBucketsReq(msg: ClusterMessage): void {
    const { requestId, partitionId } = msg.payload;
    const tree = this.merkleManager.serializeTree(partitionId);
    
    this.clusterManager.send(msg.senderId, 'CLUSTER_MERKLE_BUCKETS_RESP', {
      requestId,
      partitionId,
      buckets: tree?.buckets || {}
    });
  }

  private handleMerkleKeysReq(msg: ClusterMessage): void {
    const { requestId, partitionId, path } = msg.payload;
    const keys = this.merkleManager.getKeysInBucket(partitionId, path);
    
    this.clusterManager.send(msg.senderId, 'CLUSTER_MERKLE_KEYS_RESP', {
      requestId,
      partitionId,
      path,
      keys
    });
  }

  private handleRepairDataReq(msg: ClusterMessage): void {
    const { requestId, key } = msg.payload;
    if (!this.getRecord) return;

    const record = this.getRecord(key);
    this.clusterManager.send(msg.senderId, 'CLUSTER_REPAIR_DATA_RESP', {
      requestId,
      key,
      record
    });
  }

  private handleResponse(msg: ClusterMessage): void {
    const { requestId } = msg.payload;
    const pending = this.pendingRequests.get(requestId);
    if (pending) {
      clearTimeout(pending.timer);
      this.pendingRequests.delete(requestId);
      pending.resolve(msg.payload);
    }
  }

  // === Lifecycle Methods ===

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
    this.initialScanTimer = setTimeout(() => {
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

    if (this.initialScanTimer) {
      clearTimeout(this.initialScanTimer);
      this.initialScanTimer = undefined;
    }

    this.repairQueue = [];
    this.activeRepairs.clear();
    
    // Clear pending requests
    for (const [, req] of this.pendingRequests) {
      clearTimeout(req.timer);
      req.reject(new Error('Scheduler stopped'));
    }
    this.pendingRequests.clear();

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

      // 2. Request remote Merkle root via network
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

      // 4. Find differences via bucket exchange
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
   * Send a request and wait for response
   */
  private sendRequest<T>(nodeId: string, type: ClusterMessage['type'], payload: any): Promise<T> {
    return new Promise((resolve, reject) => {
      const requestId = Math.random().toString(36).substring(7);
      
      const timer = setTimeout(() => {
        this.pendingRequests.delete(requestId);
        reject(new Error(`Request timeout: ${type} to ${nodeId}`));
      }, this.config.requestTimeoutMs);

      this.pendingRequests.set(requestId, { resolve, reject, timer });

      this.clusterManager.send(nodeId, type, { ...payload, requestId });
    });
  }

  /**
   * Request Merkle root from remote node
   */
  private async requestRemoteMerkleRoot(nodeId: string, partitionId: number): Promise<number> {
    const response = await this.sendRequest<{ rootHash: number }>(
      nodeId,
      'CLUSTER_MERKLE_ROOT_REQ',
      { partitionId }
    );
    return response.rootHash;
  }

  /**
   * Find keys that differ between local and remote using bucket exchange
   */
  private async findDifferences(partitionId: number, replicaNodeId: string): Promise<string[]> {
    // 1. Request remote buckets
    const response = await this.sendRequest<{ buckets: Record<string, Record<string, number>> }>(
      replicaNodeId,
      'CLUSTER_MERKLE_BUCKETS_REQ',
      { partitionId }
    );
    const remoteBuckets = response.buckets;
    const localTree = this.merkleManager.getTree(partitionId);
    if (!localTree) return [];
    
    // 2. Traverse and compare
    const differingKeys: Set<string> = new Set();
    const queue: string[] = ['']; // Start at root
    const maxDepth = 3; // Should match config

    while(queue.length > 0) {
      const path = queue.shift()!;
      
      // Get local buckets at this path
      const localChildren = localTree.getBuckets(path);
      // Get remote buckets at this path
      const remoteChildren = remoteBuckets[path] || {};
      
      const allChars = new Set([...Object.keys(localChildren), ...Object.keys(remoteChildren)]);
      
      for (const char of allChars) {
        const localHash = localChildren[char] || 0;
        const remoteHash = remoteChildren[char] || 0;
        
        if (localHash !== remoteHash) {
           const nextPath = path + char;
           if (nextPath.length >= maxDepth) {
              // Leaf bucket differs - we need to reconcile keys
              // Request keys for this bucket from remote
              const bucketKeysResp = await this.sendRequest<{ keys: string[] }>(
                 replicaNodeId,
                 'CLUSTER_MERKLE_KEYS_REQ',
                 { partitionId, path: nextPath }
              );
              
              const localBucketKeys = localTree.getKeysInBucket(nextPath);
              const remoteBucketKeys = bucketKeysResp.keys;
              
              for(const k of localBucketKeys) differingKeys.add(k);
              for(const k of remoteBucketKeys) differingKeys.add(k);
           } else {
              // Intermediate differs - recurse
              queue.push(nextPath);
           }
        }
      }
    }
    
    return Array.from(differingKeys);
  }

  /**
   * Repair a single key
   */
  private async repairKey(partitionId: number, replicaNodeId: string, key: string): Promise<boolean> {
    if (!this.getRecord || !this.setRecord) {
      return false;
    }

    // 1. Get local record
    const localRecord = this.getRecord(key);

    // 2. Request remote record
    let remoteRecord: LWWRecord<any> | undefined;
    try {
      const response = await this.sendRequest<{ record: LWWRecord<any> }>(
        replicaNodeId,
        'CLUSTER_REPAIR_DATA_REQ',
        { key }
      );
      remoteRecord = response.record;
    } catch (e) {
      logger.warn({ key, replicaNodeId, err: e }, 'Failed to fetch remote record for repair');
      return false;
    }

    // 3. Resolve conflict
    const resolved = this.resolveConflict(localRecord, remoteRecord);

    if (!resolved) return false;

    // 4. Update if needed
    // If resolved is different from local, update local
    if (JSON.stringify(resolved) !== JSON.stringify(localRecord)) {
      this.setRecord(key, resolved);
      
      // If resolved is different from remote, send repair to remote (read repair)
      if (JSON.stringify(resolved) !== JSON.stringify(remoteRecord)) {
        this.clusterManager.send(replicaNodeId, 'CLUSTER_REPAIR_DATA_RESP', { 
           // In future: Use dedicated WRITE/REPAIR message
           // For now we rely on the fact that repair will eventually run on other node too
        });
      }
      return true;
    }

    return false;
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
