import { EventEmitter } from 'events';
import { ClusterManager } from './ClusterManager';
import { MigrationManager } from './MigrationManager';
import {
  hashString,
  PartitionMap,
  PartitionInfo,
  NodeInfo,
  PartitionChange,
  MigrationConfig,
  MigrationStatus,
  PARTITION_COUNT,
  DEFAULT_BACKUP_COUNT,
  DEFAULT_MIGRATION_CONFIG,
} from '@topgunbuild/core';
import { logger } from '../utils/logger';

export interface PartitionDistribution {
  owner: string;
  backups: string[];
}

export interface PartitionServiceEvents {
  'rebalanced': (map: PartitionMap, changes: PartitionChange[]) => void;
  'partitionMoved': (info: { partitionId: number; previousOwner: string; newOwner: string; version: number }) => void;
}

export interface PartitionServiceConfig {
  /** Enable gradual rebalancing (default: false for backward compatibility) */
  gradualRebalancing: boolean;
  /** Migration configuration */
  migration: Partial<MigrationConfig>;
}

export const DEFAULT_PARTITION_SERVICE_CONFIG: PartitionServiceConfig = {
  gradualRebalancing: false,
  migration: DEFAULT_MIGRATION_CONFIG,
};

export class PartitionService extends EventEmitter {
  private cluster: ClusterManager;
  // partitionId -> { owner, backups }
  private partitions: Map<number, PartitionDistribution> = new Map();
  private readonly PARTITION_COUNT = PARTITION_COUNT;
  private readonly BACKUP_COUNT = DEFAULT_BACKUP_COUNT;

  // Version tracking for partition map
  private mapVersion: number = 0;
  private lastRebalanceTime: number = 0;

  // Gradual rebalancing
  private config: PartitionServiceConfig;
  private migrationManager: MigrationManager | null = null;

  constructor(cluster: ClusterManager, config: Partial<PartitionServiceConfig> = {}) {
    super();
    this.cluster = cluster;
    this.config = {
      ...DEFAULT_PARTITION_SERVICE_CONFIG,
      ...config,
    };

    // Initialize migration manager if gradual rebalancing is enabled
    if (this.config.gradualRebalancing) {
      this.migrationManager = new MigrationManager(
        cluster,
        this,
        this.config.migration
      );

      // Forward migration events
      this.migrationManager.on('migrationComplete', (partitionId: number) => {
        logger.info({ partitionId }, 'Migration completed, updating ownership');
      });

      this.migrationManager.on('migrationFailed', (partitionId: number, error: Error) => {
        logger.error({ partitionId, error: error.message }, 'Migration failed');
      });
    }

    this.cluster.on('memberJoined', (nodeId: string) => this.onMembershipChange('JOIN', nodeId));
    this.cluster.on('memberLeft', (nodeId: string) => this.onMembershipChange('LEAVE', nodeId));

    // Initial rebalance (always immediate on startup)
    this.rebalance('REBALANCE');
  }

  /**
   * Handle membership change
   */
  private onMembershipChange(reason: 'JOIN' | 'LEAVE', nodeId: string): void {
    if (this.config.gradualRebalancing && this.migrationManager) {
      // Use gradual rebalancing
      this.rebalanceGradual(reason, nodeId);
    } else {
      // Use immediate rebalancing (original behavior)
      this.rebalance(reason, nodeId);
    }
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
  // Partition Pruning Methods
  // ============================================

  /**
   * Determine which partitions a query touches based on key equality filters.
   * Returns partition IDs when pruning is possible, null when the query
   * cannot be narrowed (e.g. OR/NOT predicates, missing key filter).
   */
  public getRelevantPartitions(query: { where?: Record<string, any>; predicate?: any }): number[] | null {
    const keys = this.extractKeysFromQuery(query);
    if (keys === null || keys.length === 0) return null;

    const seen = new Set<number>();
    for (const key of keys) {
      seen.add(this.getPartitionId(String(key)));
    }
    return Array.from(seen);
  }

  /**
   * Map partition IDs to their owning nodes.
   * Returns a deduplicated array of node IDs, excluding unassigned partitions.
   */
  public getOwnerNodesForPartitions(partitionIds: number[]): string[] {
    const seen = new Set<string>();
    for (const pid of partitionIds) {
      const owner = this.getPartitionOwner(pid);
      if (owner !== null) {
        seen.add(owner);
      }
    }
    return Array.from(seen);
  }

  // ============================================
  // Key extraction helpers (private)
  // ============================================

  private static readonly KEY_ATTRIBUTES = new Set(['_key', 'key', 'id', '_id']);

  /**
   * Extract concrete key values from a query object.
   * Returns an array of key values when deterministic, null otherwise.
   */
  private extractKeysFromQuery(query: { where?: Record<string, any>; predicate?: any }): any[] | null {
    // Try structured `where` clause first
    if (query.where) {
      const result = this.extractKeysFromWhere(query.where);
      if (result !== null) return result;
    }

    // Try predicate-based extraction
    if (query.predicate) {
      return this.extractKeysFromPredicate(query.predicate);
    }

    return null;
  }

  private extractKeysFromWhere(where: Record<string, any>): any[] | null {
    for (const attr of PartitionService.KEY_ATTRIBUTES) {
      const value = where[attr];
      if (value === undefined) continue;

      // Simple equality: { _key: "abc" }
      if (typeof value === 'string' || typeof value === 'number') {
        return [value];
      }

      // Array of keys (implicit IN): { _key: ["a", "b"] }
      if (Array.isArray(value)) {
        return value;
      }

      // Operator form: { _key: { $eq: "abc" } }
      if (value !== null && typeof value === 'object') {
        if ('$eq' in value) {
          return [value.$eq];
        }
        if ('$in' in value && Array.isArray(value.$in)) {
          return value.$in;
        }
      }
    }
    return null;
  }

  private extractKeysFromPredicate(predicate: any): any[] | null {
    if (!predicate || typeof predicate !== 'object') return null;

    const { op } = predicate;

    // Direct equality predicate: { op: "eq", attribute: "_key", value: "x" }
    if (op === 'eq' && PartitionService.KEY_ATTRIBUTES.has(predicate.attribute)) {
      return [predicate.value];
    }

    // AND predicate: check children for a key equality clause
    if (op === 'and' && Array.isArray(predicate.children)) {
      for (const child of predicate.children) {
        if (child.op === 'eq' && PartitionService.KEY_ATTRIBUTES.has(child.attribute)) {
          return [child.value];
        }
      }
    }

    // OR / NOT predicates cannot be pruned
    if (op === 'or' || op === 'not') {
      return null;
    }

    return null;
  }

  // ============================================
  // Partition Map Methods
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

  // ============================================
  // Gradual Rebalancing
  // ============================================

  /**
   * Perform gradual rebalancing using MigrationManager
   */
  private rebalanceGradual(reason: 'JOIN' | 'LEAVE', triggerNodeId: string): void {
    if (!this.migrationManager) {
      // Fall back to immediate rebalancing
      this.rebalance(reason, triggerNodeId);
      return;
    }

    // Store old distribution
    const oldDistribution = new Map(this.partitions);

    // Calculate new distribution
    let allMembers = this.cluster.getMembers().sort();
    if (allMembers.length === 0) {
      allMembers = [this.cluster.config.nodeId];
    }

    const newDistribution = new Map<number, PartitionDistribution>();

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

      newDistribution.set(i, { owner, backups });
    }

    logger.info({ memberCount: allMembers.length, reason, triggerNodeId }, 'Planning gradual rebalance');

    // Plan migrations (MigrationManager will handle gradual transfer)
    this.migrationManager.planMigration(oldDistribution, newDistribution);

    // Update partition map immediately for routing purposes
    // Actual data migration happens in background
    for (const [partitionId, dist] of newDistribution) {
      this.partitions.set(partitionId, dist);
    }

    this.mapVersion++;
    this.lastRebalanceTime = Date.now();

    // Emit event for clients
    const changes: PartitionChange[] = [];
    for (const [partitionId, newDist] of newDistribution) {
      const oldDist = oldDistribution.get(partitionId);
      if (oldDist && oldDist.owner !== newDist.owner) {
        changes.push({
          partitionId,
          previousOwner: oldDist.owner,
          newOwner: newDist.owner,
          reason,
        });
      }
    }

    this.emit('rebalanced', this.getPartitionMap(), changes);
  }

  /**
   * Set partition owner (called after migration completes)
   */
  public setOwner(partitionId: number, nodeId: string): void {
    const partition = this.partitions.get(partitionId);
    if (!partition) return;

    const previousOwner = partition.owner;
    if (previousOwner === nodeId) return; // No change

    partition.owner = nodeId;
    this.mapVersion++;

    logger.info({ partitionId, previousOwner, newOwner: nodeId, version: this.mapVersion }, 'Partition owner updated');

    this.emit('partitionMoved', {
      partitionId,
      previousOwner,
      newOwner: nodeId,
      version: this.mapVersion,
    });
  }

  /**
   * Get backups for a partition
   */
  public getBackups(partitionId: number): string[] {
    const dist = this.partitions.get(partitionId);
    return dist?.backups ?? [];
  }

  /**
   * Get migration status
   */
  public getMigrationStatus(): MigrationStatus | null {
    return this.migrationManager?.getStatus() ?? null;
  }

  /**
   * Check if partition is currently migrating
   */
  public isMigrating(partitionId: number): boolean {
    return this.migrationManager?.isActive(partitionId) ?? false;
  }

  /**
   * Check if any partition is currently migrating
   */
  public isRebalancing(): boolean {
    const status = this.getMigrationStatus();
    return status?.inProgress ?? false;
  }

  /**
   * Get MigrationManager for configuration
   */
  public getMigrationManager(): MigrationManager | null {
    return this.migrationManager;
  }

  /**
   * Cancel all migrations
   */
  public async cancelMigrations(): Promise<void> {
    if (this.migrationManager) {
      await this.migrationManager.cancelAll();
    }
  }
}
