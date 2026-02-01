/**
 * ClusterCoordinator - Unified cluster integration layer
 *
 * System Integration
 *
 * Coordinates all cluster components:
 * - ClusterManager: P2P WebSocket mesh
 * - PartitionService: Consistent hashing & routing
 * - MigrationManager: Gradual rebalancing
 * - ReplicationPipeline: Async replication with consistency levels
 * - LagTracker: Replication health monitoring
 */

import { EventEmitter } from 'events';
import { ClusterManager, ClusterConfig } from './ClusterManager';
import { PartitionService, PartitionServiceConfig, PartitionDistribution } from './PartitionService';
import { MigrationManager } from './MigrationManager';
import { ReplicationPipeline } from './ReplicationPipeline';
import { LagTracker } from './LagTracker';
import {
  MigrationConfig,
  MigrationStatus,
  MigrationMetrics,
  ReplicationConfig,
  ReplicationHealth,
  ReplicationLag,
  ReplicationResult,
  ConsistencyLevel,
  PartitionMap,
  PartitionChange,
  DEFAULT_MIGRATION_CONFIG,
  DEFAULT_REPLICATION_CONFIG,
} from '@topgunbuild/core';
import { logger } from '../utils/logger';

// ============================================
// Unified Cluster Configuration
// ============================================

export interface ClusterCoordinatorConfig {
  /** Cluster node configuration */
  cluster: ClusterConfig;

  /** Enable gradual partition rebalancing (default: true) */
  gradualRebalancing: boolean;

  /** Migration configuration for gradual rebalancing */
  migration: Partial<MigrationConfig>;

  /** Replication configuration */
  replication: Partial<ReplicationConfig>;

  /** Enable async replication pipeline (default: true) */
  replicationEnabled: boolean;

  /** Data collector callback for migrations */
  dataCollector?: (partitionId: number) => Promise<Uint8Array[]>;

  /** Data storer callback for incoming migrations */
  dataStorer?: (partitionId: number, data: Uint8Array[]) => Promise<void>;
}

export const DEFAULT_CLUSTER_COORDINATOR_CONFIG: Omit<ClusterCoordinatorConfig, 'cluster'> = {
  gradualRebalancing: true,
  migration: DEFAULT_MIGRATION_CONFIG,
  replication: DEFAULT_REPLICATION_CONFIG,
  replicationEnabled: true,
};

// ============================================
// Cluster Coordinator Events
// ============================================

export interface ClusterCoordinatorEvents {
  'started': () => void;
  'stopped': () => void;
  'member:joined': (nodeId: string) => void;
  'member:left': (nodeId: string) => void;
  'partition:rebalanced': (map: PartitionMap, changes: PartitionChange[]) => void;
  'partition:moved': (info: { partitionId: number; previousOwner: string; newOwner: string; version: number }) => void;
  'migration:started': (partitionId: number, targetNode: string) => void;
  'migration:completed': (partitionId: number) => void;
  'migration:failed': (partitionId: number, error: Error) => void;
  'replication:unhealthy': (nodeId: string) => void;
  'replication:healthy': (nodeId: string) => void;
  'error': (error: Error) => void;
}

// ============================================
// Cluster Coordinator
// ============================================

export class ClusterCoordinator extends EventEmitter {
  private readonly config: ClusterCoordinatorConfig;

  // Core components
  private clusterManager: ClusterManager;
  private partitionService: PartitionService;
  private replicationPipeline: ReplicationPipeline | null = null;
  private lagTracker: LagTracker;

  // State
  private started: boolean = false;
  private actualPort: number = 0;

  constructor(config: ClusterCoordinatorConfig) {
    super();
    this.config = {
      ...DEFAULT_CLUSTER_COORDINATOR_CONFIG,
      ...config,
    };

    // Initialize core components
    this.clusterManager = new ClusterManager(this.config.cluster);
    this.lagTracker = new LagTracker();

    // Initialize partition service with or without gradual rebalancing
    const partitionServiceConfig: Partial<PartitionServiceConfig> = {
      gradualRebalancing: this.config.gradualRebalancing,
      migration: this.config.migration,
    };
    this.partitionService = new PartitionService(this.clusterManager, partitionServiceConfig);

    // Initialize replication pipeline if enabled
    if (this.config.replicationEnabled) {
      this.replicationPipeline = new ReplicationPipeline(
        this.clusterManager,
        this.partitionService,
        this.config.replication
      );
    }

    this.setupEventHandlers();
  }

  // ============================================
  // Lifecycle Methods
  // ============================================

  /**
   * Start the cluster coordinator
   */
  public async start(): Promise<number> {
    if (this.started) {
      return this.actualPort;
    }

    logger.info({ nodeId: this.config.cluster.nodeId }, 'Starting ClusterCoordinator');

    // Start cluster manager
    this.actualPort = await this.clusterManager.start();

    // Configure migration data handlers if provided
    const migrationManager = this.partitionService.getMigrationManager();
    if (migrationManager && this.config.dataCollector) {
      migrationManager.setDataCollector(this.config.dataCollector);
    }
    if (migrationManager && this.config.dataStorer) {
      migrationManager.setDataStorer(this.config.dataStorer);
    }

    this.started = true;
    this.emit('started');

    logger.info({ nodeId: this.config.cluster.nodeId, port: this.actualPort }, 'ClusterCoordinator started');
    return this.actualPort;
  }

  /**
   * Stop the cluster coordinator
   */
  public async stop(): Promise<void> {
    if (!this.started) return;

    logger.info({ nodeId: this.config.cluster.nodeId }, 'Stopping ClusterCoordinator');

    // Cancel any active migrations
    await this.partitionService.cancelMigrations();

    // Close replication pipeline
    this.replicationPipeline?.close();

    // Stop cluster manager
    this.clusterManager.stop();

    this.started = false;
    this.emit('stopped');

    logger.info({ nodeId: this.config.cluster.nodeId }, 'ClusterCoordinator stopped');
  }

  // ============================================
  // Cluster Information
  // ============================================

  /**
   * Get local node ID
   */
  public getNodeId(): string {
    return this.config.cluster.nodeId;
  }

  /**
   * Get cluster port
   */
  public getPort(): number {
    return this.actualPort;
  }

  /**
   * Get all cluster members
   */
  public getMembers(): string[] {
    return this.clusterManager.getMembers();
  }

  /**
   * Check if this is the local node
   */
  public isLocal(nodeId: string): boolean {
    return this.clusterManager.isLocal(nodeId);
  }

  /**
   * Check if coordinator is started
   */
  public isStarted(): boolean {
    return this.started;
  }

  // ============================================
  // Partition Operations
  // ============================================

  /**
   * Get current partition map
   */
  public getPartitionMap(): PartitionMap {
    return this.partitionService.getPartitionMap();
  }

  /**
   * Get partition map version
   */
  public getPartitionMapVersion(): number {
    return this.partitionService.getMapVersion();
  }

  /**
   * Get partition ID for a key
   */
  public getPartitionId(key: string): number {
    return this.partitionService.getPartitionId(key);
  }

  /**
   * Get owner node for a key
   */
  public getOwner(key: string): string {
    return this.partitionService.getOwner(key);
  }

  /**
   * Check if this node owns the key
   */
  public isLocalOwner(key: string): boolean {
    return this.partitionService.isLocalOwner(key);
  }

  /**
   * Check if this node is a backup for the key
   */
  public isLocalBackup(key: string): boolean {
    return this.partitionService.isLocalBackup(key);
  }

  /**
   * Get backup nodes for a partition
   */
  public getBackups(partitionId: number): string[] {
    return this.partitionService.getBackups(partitionId);
  }

  /**
   * Check if partition is currently migrating
   */
  public isMigrating(partitionId: number): boolean {
    return this.partitionService.isMigrating(partitionId);
  }

  /**
   * Check if any rebalancing is in progress
   */
  public isRebalancing(): boolean {
    return this.partitionService.isRebalancing();
  }

  // ============================================
  // Migration Operations
  // ============================================

  /**
   * Get migration status
   */
  public getMigrationStatus(): MigrationStatus | null {
    return this.partitionService.getMigrationStatus();
  }

  /**
   * Get migration metrics
   */
  public getMigrationMetrics(): MigrationMetrics | null {
    return this.partitionService.getMigrationManager()?.getMetrics() ?? null;
  }

  /**
   * Cancel all active migrations
   */
  public async cancelMigrations(): Promise<void> {
    await this.partitionService.cancelMigrations();
  }

  /**
   * Set data collector for migrations
   */
  public setDataCollector(collector: (partitionId: number) => Promise<Uint8Array[]>): void {
    const migrationManager = this.partitionService.getMigrationManager();
    if (migrationManager) {
      migrationManager.setDataCollector(collector);
    }
  }

  /**
   * Set data storer for incoming migrations
   */
  public setDataStorer(storer: (partitionId: number, data: Uint8Array[]) => Promise<void>): void {
    const migrationManager = this.partitionService.getMigrationManager();
    if (migrationManager) {
      migrationManager.setDataStorer(storer);
    }
  }

  // ============================================
  // Replication Operations
  // ============================================

  /**
   * Replicate an operation to backup nodes
   */
  public async replicate(
    operation: unknown,
    opId: string,
    key: string,
    options: { consistency?: ConsistencyLevel; timeout?: number } = {}
  ): Promise<ReplicationResult> {
    if (!this.replicationPipeline) {
      return { success: true, ackedBy: [] };
    }
    return this.replicationPipeline.replicate(operation, opId, key, options);
  }

  /**
   * Get replication health status
   */
  public getReplicationHealth(): ReplicationHealth {
    return this.lagTracker.getHealth();
  }

  /**
   * Get replication lag for a specific node
   */
  public getReplicationLag(nodeId: string): ReplicationLag {
    return this.lagTracker.getLag(nodeId);
  }

  /**
   * Check if a node is healthy for replication
   */
  public isNodeHealthy(nodeId: string): boolean {
    return this.lagTracker.isNodeHealthy(nodeId);
  }

  /**
   * Check if a node is laggy
   */
  public isNodeLaggy(nodeId: string): boolean {
    return this.lagTracker.isNodeLaggy(nodeId);
  }

  // ============================================
  // Cluster Communication
  // ============================================

  /**
   * Send message to a specific node
   */
  public send(nodeId: string, message: unknown): void {
    this.clusterManager.sendToNode(nodeId, message);
  }

  /**
   * Broadcast message to all nodes
   */
  public broadcast(message: unknown): void {
    for (const nodeId of this.clusterManager.getMembers()) {
      if (!this.clusterManager.isLocal(nodeId)) {
        this.clusterManager.sendToNode(nodeId, message);
      }
    }
  }

  // ============================================
  // Component Access
  // ============================================

  /**
   * Get underlying ClusterManager
   */
  public getClusterManager(): ClusterManager {
    return this.clusterManager;
  }

  /**
   * Get underlying PartitionService
   */
  public getPartitionService(): PartitionService {
    return this.partitionService;
  }

  /**
   * Get underlying ReplicationPipeline
   */
  public getReplicationPipeline(): ReplicationPipeline | null {
    return this.replicationPipeline;
  }

  /**
   * Get underlying LagTracker
   */
  public getLagTracker(): LagTracker {
    return this.lagTracker;
  }

  // ============================================
  // Metrics Export
  // ============================================

  /**
   * Get all metrics in Prometheus format
   */
  public getPrometheusMetrics(): string {
    const lines: string[] = [];

    // Cluster info
    lines.push('# HELP topgun_cluster_members Number of cluster members');
    lines.push('# TYPE topgun_cluster_members gauge');
    lines.push(`topgun_cluster_members ${this.clusterManager.getMembers().length}`);

    lines.push('');
    lines.push('# HELP topgun_cluster_started Cluster started status (1=started, 0=stopped)');
    lines.push('# TYPE topgun_cluster_started gauge');
    lines.push(`topgun_cluster_started ${this.started ? 1 : 0}`);

    // Partition map info
    lines.push('');
    lines.push('# HELP topgun_partition_map_version Current partition map version');
    lines.push('# TYPE topgun_partition_map_version gauge');
    lines.push(`topgun_partition_map_version ${this.partitionService.getMapVersion()}`);

    // Migration metrics
    const migrationMetrics = this.getMigrationMetrics();
    if (migrationMetrics) {
      lines.push('');
      lines.push('# HELP topgun_migrations_started Total migrations started');
      lines.push('# TYPE topgun_migrations_started counter');
      lines.push(`topgun_migrations_started ${migrationMetrics.migrationsStarted}`);

      lines.push('');
      lines.push('# HELP topgun_migrations_completed Total migrations completed');
      lines.push('# TYPE topgun_migrations_completed counter');
      lines.push(`topgun_migrations_completed ${migrationMetrics.migrationsCompleted}`);

      lines.push('');
      lines.push('# HELP topgun_migrations_failed Total migrations failed');
      lines.push('# TYPE topgun_migrations_failed counter');
      lines.push(`topgun_migrations_failed ${migrationMetrics.migrationsFailed}`);

      lines.push('');
      lines.push('# HELP topgun_migrations_active Currently active migrations');
      lines.push('# TYPE topgun_migrations_active gauge');
      lines.push(`topgun_migrations_active ${migrationMetrics.activeMigrations}`);

      lines.push('');
      lines.push('# HELP topgun_migrations_queued Queued migrations');
      lines.push('# TYPE topgun_migrations_queued gauge');
      lines.push(`topgun_migrations_queued ${migrationMetrics.queuedMigrations}`);
    }

    // Replication metrics from LagTracker
    lines.push('');
    lines.push(this.lagTracker.toPrometheusMetrics());

    return lines.join('\n');
  }

  // ============================================
  // Private Methods
  // ============================================

  private setupEventHandlers(): void {
    // ClusterManager events
    this.clusterManager.on('memberJoined', (nodeId: string) => {
      logger.info({ nodeId }, 'Cluster member joined');
      this.emit('member:joined', nodeId);
    });

    this.clusterManager.on('memberLeft', (nodeId: string) => {
      logger.info({ nodeId }, 'Cluster member left');
      this.lagTracker.removeNode(nodeId);
      this.emit('member:left', nodeId);
    });

    // PartitionService events
    this.partitionService.on('rebalanced', (map: PartitionMap, changes: PartitionChange[]) => {
      logger.info({ version: map.version, changesCount: changes.length }, 'Partition map rebalanced');
      this.emit('partition:rebalanced', map, changes);
    });

    this.partitionService.on('partitionMoved', (info: { partitionId: number; previousOwner: string; newOwner: string; version: number }) => {
      this.emit('partition:moved', info);
    });

    // MigrationManager events (if gradual rebalancing enabled)
    const migrationManager = this.partitionService.getMigrationManager();
    if (migrationManager) {
      migrationManager.on('migrationStarted', (partitionId: number, targetNode: string) => {
        this.emit('migration:started', partitionId, targetNode);
      });

      migrationManager.on('migrationComplete', (partitionId: number) => {
        this.emit('migration:completed', partitionId);
      });

      migrationManager.on('migrationFailed', (partitionId: number, error: Error) => {
        this.emit('migration:failed', partitionId, error);
      });
    }

    // ReplicationPipeline events
    if (this.replicationPipeline) {
      this.replicationPipeline.on('ackReceived', (nodeId: string) => {
        this.lagTracker.recordAck(nodeId);
      });

      this.replicationPipeline.on('replicationSent', (nodeId: string) => {
        this.lagTracker.incrementPending(nodeId);
      });
    }
  }
}
