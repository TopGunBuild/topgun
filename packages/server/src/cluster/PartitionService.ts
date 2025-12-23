import { EventEmitter } from 'events';
import { ClusterManager } from './ClusterManager';
import {
  hashString,
  PartitionMap,
  PartitionInfo,
  NodeInfo,
  PartitionChange,
  PARTITION_COUNT,
  DEFAULT_BACKUP_COUNT,
} from '@topgunbuild/core';
import { logger } from '../utils/logger';

export interface PartitionDistribution {
  owner: string;
  backups: string[];
}

export interface PartitionServiceEvents {
  'rebalanced': (map: PartitionMap, changes: PartitionChange[]) => void;
}

export class PartitionService extends EventEmitter {
  private cluster: ClusterManager;
  // partitionId -> { owner, backups }
  private partitions: Map<number, PartitionDistribution> = new Map();
  private readonly PARTITION_COUNT = PARTITION_COUNT;
  private readonly BACKUP_COUNT = DEFAULT_BACKUP_COUNT;

  // Phase 4: Version tracking for partition map
  private mapVersion: number = 0;
  private lastRebalanceTime: number = 0;

  constructor(cluster: ClusterManager) {
    super();
    this.cluster = cluster;
    this.cluster.on('memberJoined', (nodeId: string) => this.rebalance('JOIN', nodeId));
    this.cluster.on('memberLeft', (nodeId: string) => this.rebalance('LEAVE', nodeId));

    // Initial rebalance
    this.rebalance('REBALANCE');
  }

  public getPartitionId(key: string): number {
    // Use Math.abs to ensure positive partition ID
    return Math.abs(hashString(key)) % this.PARTITION_COUNT;
  }

  public getDistribution(key: string): PartitionDistribution {
    const pId = this.getPartitionId(key);
    return this.partitions.get(pId) || { 
      owner: this.cluster.config.nodeId, 
      backups: [] 
    };
  }

  public getOwner(key: string): string {
    return this.getDistribution(key).owner;
  }

  public isLocalOwner(key: string): boolean {
    return this.getOwner(key) === this.cluster.config.nodeId;
  }

  public isLocalBackup(key: string): boolean {
    const dist = this.getDistribution(key);
    return dist.backups.includes(this.cluster.config.nodeId);
  }

  public isRelated(key: string): boolean {
    return this.isLocalOwner(key) || this.isLocalBackup(key);
  }

  // ============================================
  // Phase 4: Partition Map Methods
  // ============================================

  /**
   * Get current partition map version
   */
  public getMapVersion(): number {
    return this.mapVersion;
  }

  /**
   * Generate full PartitionMap for client consumption
   */
  public getPartitionMap(): PartitionMap {
    const nodes: NodeInfo[] = [];
    const partitions: PartitionInfo[] = [];

    // Build node info from cluster members
    for (const nodeId of this.cluster.getMembers()) {
      const isSelf = nodeId === this.cluster.config.nodeId;
      const host = isSelf ? this.cluster.config.host : 'unknown';
      const port = isSelf ? this.cluster.port : 0;

      nodes.push({
        nodeId,
        endpoints: {
          websocket: `ws://${host}:${port}`,
        },
        status: 'ACTIVE',
      });
    }

    // Build partition info
    for (let i = 0; i < this.PARTITION_COUNT; i++) {
      const dist = this.partitions.get(i);
      if (dist) {
        partitions.push({
          partitionId: i,
          ownerNodeId: dist.owner,
          backupNodeIds: dist.backups,
        });
      }
    }

    return {
      version: this.mapVersion,
      partitionCount: this.PARTITION_COUNT,
      nodes,
      partitions,
      generatedAt: Date.now(),
    };
  }

  /**
   * Get partition info by ID
   */
  public getPartitionInfo(partitionId: number): PartitionInfo | null {
    const dist = this.partitions.get(partitionId);
    if (!dist) return null;

    return {
      partitionId,
      ownerNodeId: dist.owner,
      backupNodeIds: dist.backups,
    };
  }

  /**
   * Get owner node for a partition ID
   */
  public getPartitionOwner(partitionId: number): string | null {
    const dist = this.partitions.get(partitionId);
    return dist?.owner ?? null;
  }

  private rebalance(reason: 'REBALANCE' | 'FAILOVER' | 'JOIN' | 'LEAVE' = 'REBALANCE', triggerNodeId?: string) {
    // Store old partitions for change detection
    const oldPartitions = new Map(this.partitions);

    // this.cluster.getMembers() includes self (added in ClusterManager.start)
    let allMembers = this.cluster.getMembers().sort();

    // If no other members, include self
    if (allMembers.length === 0) {
      allMembers = [this.cluster.config.nodeId];
    }

    logger.info({ memberCount: allMembers.length, members: allMembers, reason }, 'Rebalancing partitions');

    const changes: PartitionChange[] = [];

    for (let i = 0; i < this.PARTITION_COUNT; i++) {
      const ownerIndex = i % allMembers.length;
      const owner = allMembers[ownerIndex];

      const backups: string[] = [];
      if (allMembers.length > 1) {
        for (let b = 1; b <= this.BACKUP_COUNT; b++) {
          const backupIndex = (ownerIndex + b) % allMembers.length;
          backups.push(allMembers[backupIndex]);
        }
      }

      // Track changes
      const oldDist = oldPartitions.get(i);
      if (oldDist && oldDist.owner !== owner) {
        changes.push({
          partitionId: i,
          previousOwner: oldDist.owner,
          newOwner: owner,
          reason,
        });
      }

      this.partitions.set(i, { owner, backups });
    }

    // Increment version if there were changes
    if (changes.length > 0 || this.mapVersion === 0) {
      this.mapVersion++;
      this.lastRebalanceTime = Date.now();

      logger.info({
        version: this.mapVersion,
        changesCount: changes.length,
        reason,
      }, 'Partition map updated');

      // Emit event for ServerCoordinator to broadcast to clients
      this.emit('rebalanced', this.getPartitionMap(), changes);
    }
  }
}
