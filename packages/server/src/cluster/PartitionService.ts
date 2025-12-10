import { ClusterManager } from './ClusterManager';
import { hashString } from '@topgunbuild/core';
import { logger } from '../utils/logger';

export interface PartitionDistribution {
  owner: string;
  backups: string[];
}

export class PartitionService {
  private cluster: ClusterManager;
  // partitionId -> { owner, backups }
  private partitions: Map<number, PartitionDistribution> = new Map();
  private readonly PARTITION_COUNT = 271;
  private readonly BACKUP_COUNT = 1; // Standard Hazelcast default

  constructor(cluster: ClusterManager) {
    this.cluster = cluster;
    this.cluster.on('memberJoined', () => this.rebalance());
    this.cluster.on('memberLeft', () => this.rebalance());
    
    // Initial rebalance
    this.rebalance();
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

  private rebalance() {
    // this.cluster.getMembers() includes self (added in ClusterManager.start)
    let allMembers = this.cluster.getMembers().sort();

    // If no other members, include self
    if (allMembers.length === 0) {
      allMembers = [this.cluster.config.nodeId];
    }

    logger.info({ memberCount: allMembers.length, members: allMembers }, 'Rebalancing partitions');

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

      this.partitions.set(i, { owner, backups });
    }
  }
}
