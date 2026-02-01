/**
 * ReadReplicaHandler - Read Scaling via Replicas
 *
 * Enables reading from backup nodes to:
 * - Scale read throughput linearly with replicas
 * - Reduce latency by reading from nearest replica
 * - Provide availability during owner unavailability
 *
 * Supports three consistency levels for reads:
 * - STRONG: Read from partition owner (current behavior)
 * - EVENTUAL: Read from any replica (owner or backup)
 * - LOCAL: Read from local node if it's a replica
 */

import { EventEmitter } from 'events';
import { ConsistencyLevel, ClusterReadOptions as ReadOptions, LWWRecord, Timestamp } from '@topgunbuild/core';
import { PartitionService } from './PartitionService';
import { ClusterManager } from './ClusterManager';
import { LagTracker } from './LagTracker';
import { logger } from '../utils/logger';

export interface ReadReplicaConfig {
  /** Default consistency for reads. Default: STRONG */
  defaultConsistency: ConsistencyLevel;
  /** Maximum staleness for eventual reads in ms. Default: 5000 */
  maxStalenessMs: number;
  /** Prefer local replica over remote. Default: true */
  preferLocalReplica: boolean;
  /** Load balancing strategy for replica selection. Default: 'latency-based' */
  loadBalancing: 'round-robin' | 'least-connections' | 'latency-based';
}

export const DEFAULT_READ_REPLICA_CONFIG: ReadReplicaConfig = {
  defaultConsistency: ConsistencyLevel.STRONG,
  maxStalenessMs: 5000,
  preferLocalReplica: true,
  loadBalancing: 'latency-based',
};

export interface ReadResult<T> {
  value: T | null;
  version?: Timestamp;
  source: string; // nodeId that served the read
  isOwner: boolean;
  staleness?: number; // Estimated staleness in ms
}

export interface ReadRequest {
  mapName: string;
  key: string;
  options?: ReadOptions;
}

export class ReadReplicaHandler extends EventEmitter {
  private config: ReadReplicaConfig;
  private partitionService: PartitionService;
  private clusterManager: ClusterManager;
  private lagTracker?: LagTracker;
  private nodeId: string;

  // Round-robin counters for load balancing
  private roundRobinCounters: Map<number, number> = new Map();

  constructor(
    partitionService: PartitionService,
    clusterManager: ClusterManager,
    nodeId: string,
    lagTracker?: LagTracker,
    config: Partial<ReadReplicaConfig> = {}
  ) {
    super();
    this.partitionService = partitionService;
    this.clusterManager = clusterManager;
    this.nodeId = nodeId;
    this.lagTracker = lagTracker;
    this.config = { ...DEFAULT_READ_REPLICA_CONFIG, ...config };
  }

  /**
   * Determine if a read request can be served locally
   */
  canServeLocally(request: ReadRequest): boolean {
    const consistency = request.options?.consistency ?? this.config.defaultConsistency;

    // STRONG consistency requires owner
    if (consistency === ConsistencyLevel.STRONG) {
      return this.partitionService.isLocalOwner(request.key);
    }

    // EVENTUAL or LOCAL - can serve if we're owner or backup
    return this.partitionService.isRelated(request.key);
  }

  /**
   * Determine which node should handle the read
   */
  selectReadNode(request: ReadRequest): string | null {
    const key = request.key;
    const consistency = request.options?.consistency ?? this.config.defaultConsistency;

    // Get partition info
    const partitionId = this.partitionService.getPartitionId(key);
    const distribution = this.partitionService.getDistribution(key);

    // STRONG: Must read from owner
    if (consistency === ConsistencyLevel.STRONG) {
      if (!this.isNodeAlive(distribution.owner)) {
        // If allowStale is true, fall back to backup
        if (request.options?.allowStale) {
          return this.selectAliveBackup(distribution.backups);
        }
        return null;
      }
      return distribution.owner;
    }

    // EVENTUAL: Select best replica based on load balancing
    const allReplicas = [distribution.owner, ...distribution.backups];
    const aliveReplicas = allReplicas.filter(n => this.isNodeAlive(n));

    if (aliveReplicas.length === 0) {
      return null;
    }

    // Check max staleness constraint
    if (request.options?.maxStaleness) {
      const withinStaleness = aliveReplicas.filter(n =>
        this.getNodeStaleness(n) <= (request.options?.maxStaleness ?? Infinity)
      );
      if (withinStaleness.length > 0) {
        return this.selectByStrategy(withinStaleness, partitionId);
      }
      // If no replica meets staleness requirement, try owner
      if (this.isNodeAlive(distribution.owner)) {
        return distribution.owner;
      }
    }

    // Prefer local replica if configured
    if (this.config.preferLocalReplica && aliveReplicas.includes(this.nodeId)) {
      return this.nodeId;
    }

    return this.selectByStrategy(aliveReplicas, partitionId);
  }

  /**
   * Select replica using configured load balancing strategy
   */
  private selectByStrategy(replicas: string[], partitionId: number): string {
    if (replicas.length === 0) {
      throw new Error('No replicas available');
    }

    if (replicas.length === 1) {
      return replicas[0];
    }

    switch (this.config.loadBalancing) {
      case 'round-robin':
        return this.selectRoundRobin(replicas, partitionId);

      case 'latency-based':
        return this.selectByLatency(replicas);

      case 'least-connections':
        // For now, fall back to round-robin
        // Could be enhanced with connection tracking
        return this.selectRoundRobin(replicas, partitionId);

      default:
        return replicas[0];
    }
  }

  /**
   * Round-robin selection
   */
  private selectRoundRobin(replicas: string[], partitionId: number): string {
    const counter = this.roundRobinCounters.get(partitionId) ?? 0;
    const selected = replicas[counter % replicas.length];
    this.roundRobinCounters.set(partitionId, counter + 1);
    return selected;
  }

  /**
   * Latency-based selection using lag tracker
   */
  private selectByLatency(replicas: string[]): string {
    if (!this.lagTracker) {
      return replicas[0];
    }

    let bestNode = replicas[0];
    let bestLatency = Infinity;

    for (const nodeId of replicas) {
      const lag = this.lagTracker.getLag(nodeId);
      if (lag && lag.current < bestLatency) {
        bestLatency = lag.current;
        bestNode = nodeId;
      }
    }

    return bestNode;
  }

  /**
   * Get estimated staleness for a node in ms
   */
  private getNodeStaleness(nodeId: string): number {
    if (nodeId === this.partitionService.getOwner('')) {
      return 0; // Owner is always fresh
    }

    if (this.lagTracker) {
      const lag = this.lagTracker.getLag(nodeId);
      return lag?.current ?? 0;
    }

    return 0;
  }

  /**
   * Check if a node is alive in the cluster
   */
  private isNodeAlive(nodeId: string): boolean {
    const members = this.clusterManager.getMembers();
    return members.includes(nodeId);
  }

  /**
   * Select first alive backup from list
   */
  private selectAliveBackup(backups: string[]): string | null {
    for (const backup of backups) {
      if (this.isNodeAlive(backup)) {
        return backup;
      }
    }
    return null;
  }

  /**
   * Create read response metadata
   */
  createReadMetadata(key: string, options?: ReadOptions): {
    source: string;
    isOwner: boolean;
    consistency: ConsistencyLevel;
  } {
    const consistency = options?.consistency ?? this.config.defaultConsistency;
    const isOwner = this.partitionService.isLocalOwner(key);

    return {
      source: this.nodeId,
      isOwner,
      consistency,
    };
  }

  /**
   * Check if local node should forward read to owner
   */
  shouldForwardRead(request: ReadRequest): boolean {
    const consistency = request.options?.consistency ?? this.config.defaultConsistency;

    // STRONG requires forwarding if not owner
    if (consistency === ConsistencyLevel.STRONG) {
      return !this.partitionService.isLocalOwner(request.key);
    }

    // EVENTUAL/LOCAL can serve if we're a replica
    if (!this.partitionService.isRelated(request.key)) {
      return true; // Not a replica, must forward
    }

    return false;
  }

  /**
   * Get metrics for monitoring
   */
  getMetrics(): {
    defaultConsistency: ConsistencyLevel;
    preferLocalReplica: boolean;
    loadBalancing: string;
    roundRobinPartitions: number;
  } {
    return {
      defaultConsistency: this.config.defaultConsistency,
      preferLocalReplica: this.config.preferLocalReplica,
      loadBalancing: this.config.loadBalancing,
      roundRobinPartitions: this.roundRobinCounters.size,
    };
  }
}
