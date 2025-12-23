/**
 * PartitionRouter - Routes operations to the correct cluster node
 *
 * Phase 4: Partition-Aware Client Routing
 *
 * Features:
 * - Maintains local copy of partition map
 * - Routes keys to owner nodes using consistent hashing
 * - Handles stale routing with automatic refresh
 * - Supports fallback to server-side forwarding
 */

import {
  PartitionMap,
  PartitionRouterConfig,
  DEFAULT_PARTITION_ROUTER_CONFIG,
  PartitionMapMessage,
  PartitionMapDeltaMessage,
  PartitionChange,
  PARTITION_COUNT,
  hashString,
} from '@topgunbuild/core';
import { ConnectionPool } from './ConnectionPool';
import { logger } from '../utils/logger';

export interface RoutingResult {
  nodeId: string;
  partitionId: number;
  isOwner: boolean;
  isBackup: boolean;
}

export interface PartitionRouterEvents {
  'partitionMap:updated': (version: number, changesCount: number) => void;
  'partitionMap:stale': (currentVersion: number, lastRefresh: number) => void;
  'routing:miss': (key: string, expectedOwner: string, actualOwner: string) => void;
}

export class PartitionRouter {
  private readonly listeners: Map<string, Set<(...args: any[]) => void>> = new Map();
  private readonly config: PartitionRouterConfig;
  private readonly connectionPool: ConnectionPool;
  private partitionMap: PartitionMap | null = null;
  private lastRefreshTime: number = 0;
  private refreshTimer: ReturnType<typeof setInterval> | null = null;
  private pendingRefresh: Promise<void> | null = null;

  constructor(
    connectionPool: ConnectionPool,
    config: Partial<PartitionRouterConfig> = {}
  ) {
    this.connectionPool = connectionPool;
    this.config = {
      ...DEFAULT_PARTITION_ROUTER_CONFIG,
      ...config,
    };

    // Listen for partition map updates from any connection
    this.connectionPool.on('message', (nodeId: string, message: any) => {
      if (message.type === 'PARTITION_MAP') {
        this.handlePartitionMap(message as PartitionMapMessage);
      } else if (message.type === 'PARTITION_MAP_DELTA') {
        this.handlePartitionMapDelta(message as PartitionMapDeltaMessage);
      }
    });
  }

  // ============================================
  // Event Emitter Methods (browser-compatible)
  // ============================================

  public on(event: string, listener: (...args: any[]) => void): this {
    if (!this.listeners.has(event)) {
      this.listeners.set(event, new Set());
    }
    this.listeners.get(event)!.add(listener);
    return this;
  }

  public off(event: string, listener: (...args: any[]) => void): this {
    this.listeners.get(event)?.delete(listener);
    return this;
  }

  public once(event: string, listener: (...args: any[]) => void): this {
    const wrapper = (...args: any[]) => {
      this.off(event, wrapper);
      listener(...args);
    };
    return this.on(event, wrapper);
  }

  public emit(event: string, ...args: any[]): boolean {
    const eventListeners = this.listeners.get(event);
    if (!eventListeners || eventListeners.size === 0) {
      return false;
    }
    for (const listener of eventListeners) {
      try {
        listener(...args);
      } catch (err) {
        logger.error({ event, err }, 'Error in event listener');
      }
    }
    return true;
  }

  public removeListener(event: string, listener: (...args: any[]) => void): this {
    return this.off(event, listener);
  }

  public removeAllListeners(event?: string): this {
    if (event) {
      this.listeners.delete(event);
    } else {
      this.listeners.clear();
    }
    return this;
  }

  /**
   * Get the partition ID for a given key
   */
  public getPartitionId(key: string): number {
    return Math.abs(hashString(key)) % PARTITION_COUNT;
  }

  /**
   * Route a key to the owner node
   */
  public route(key: string): RoutingResult | null {
    if (!this.partitionMap) {
      return null;
    }

    const partitionId = this.getPartitionId(key);
    const partition = this.partitionMap.partitions.find(p => p.partitionId === partitionId);

    if (!partition) {
      logger.warn({ key, partitionId }, 'Partition not found in map');
      return null;
    }

    return {
      nodeId: partition.ownerNodeId,
      partitionId,
      isOwner: true,
      isBackup: false,
    };
  }

  /**
   * Route a key and get the WebSocket connection to use
   */
  public routeToConnection(key: string): { nodeId: string; socket: WebSocket } | null {
    const routing = this.route(key);

    if (!routing) {
      // No partition map, use fallback
      if (this.config.fallbackMode === 'forward') {
        const primary = this.connectionPool.getAnyHealthyConnection();
        if (primary) {
          return primary;
        }
      }
      return null;
    }

    // Try to get connection to owner
    const socket = this.connectionPool.getConnection(routing.nodeId);
    if (socket) {
      return { nodeId: routing.nodeId, socket };
    }

    // Owner not available, try backup
    const partition = this.partitionMap!.partitions.find(p => p.partitionId === routing.partitionId);
    if (partition) {
      for (const backupId of partition.backupNodeIds) {
        const backupSocket = this.connectionPool.getConnection(backupId);
        if (backupSocket) {
          logger.debug({ key, owner: routing.nodeId, backup: backupId }, 'Using backup node');
          return { nodeId: backupId, socket: backupSocket };
        }
      }
    }

    // Fallback to any connection
    if (this.config.fallbackMode === 'forward') {
      return this.connectionPool.getAnyHealthyConnection();
    }

    return null;
  }

  /**
   * Get routing info for multiple keys (batch routing)
   */
  public routeBatch(keys: string[]): Map<string, RoutingResult[]> {
    const result = new Map<string, RoutingResult[]>();

    for (const key of keys) {
      const routing = this.route(key);
      if (routing) {
        const nodeId = routing.nodeId;
        if (!result.has(nodeId)) {
          result.set(nodeId, []);
        }
        result.get(nodeId)!.push({ ...routing, key } as any);
      }
    }

    return result;
  }

  /**
   * Get all partitions owned by a specific node
   */
  public getPartitionsForNode(nodeId: string): number[] {
    if (!this.partitionMap) return [];

    return this.partitionMap.partitions
      .filter(p => p.ownerNodeId === nodeId)
      .map(p => p.partitionId);
  }

  /**
   * Get current partition map version
   */
  public getMapVersion(): number {
    return this.partitionMap?.version ?? 0;
  }

  /**
   * Check if partition map is available
   */
  public hasPartitionMap(): boolean {
    return this.partitionMap !== null;
  }

  /**
   * Check if partition map is stale
   */
  public isMapStale(): boolean {
    if (!this.partitionMap) return true;

    const now = Date.now();
    return (now - this.lastRefreshTime) > this.config.maxMapStalenessMs;
  }

  /**
   * Request fresh partition map from server
   */
  public async refreshPartitionMap(): Promise<void> {
    if (this.pendingRefresh) {
      return this.pendingRefresh;
    }

    this.pendingRefresh = this.doRefreshPartitionMap();

    try {
      await this.pendingRefresh;
    } finally {
      this.pendingRefresh = null;
    }
  }

  /**
   * Start periodic partition map refresh
   */
  public startPeriodicRefresh(): void {
    if (this.refreshTimer) return;

    this.refreshTimer = setInterval(() => {
      if (this.isMapStale()) {
        this.emit('partitionMap:stale', this.getMapVersion(), this.lastRefreshTime);
        this.refreshPartitionMap().catch(err => {
          logger.error({ error: err }, 'Failed to refresh partition map');
        });
      }
    }, this.config.mapRefreshIntervalMs);
  }

  /**
   * Stop periodic refresh
   */
  public stopPeriodicRefresh(): void {
    if (this.refreshTimer) {
      clearInterval(this.refreshTimer);
      this.refreshTimer = null;
    }
  }

  /**
   * Handle NOT_OWNER error from server
   */
  public handleNotOwnerError(key: string, actualOwner: string, newMapVersion: number): void {
    const routing = this.route(key);
    const expectedOwner = routing?.nodeId ?? 'unknown';

    this.emit('routing:miss', key, expectedOwner, actualOwner);

    // If server has newer map, request it
    if (newMapVersion > this.getMapVersion()) {
      this.refreshPartitionMap().catch(err => {
        logger.error({ error: err }, 'Failed to refresh partition map after NOT_OWNER');
      });
    }
  }

  /**
   * Get statistics about routing
   */
  public getStats(): {
    mapVersion: number;
    partitionCount: number;
    nodeCount: number;
    lastRefresh: number;
    isStale: boolean;
  } {
    return {
      mapVersion: this.getMapVersion(),
      partitionCount: this.partitionMap?.partitionCount ?? 0,
      nodeCount: this.partitionMap?.nodes.length ?? 0,
      lastRefresh: this.lastRefreshTime,
      isStale: this.isMapStale(),
    };
  }

  /**
   * Cleanup resources
   */
  public close(): void {
    this.stopPeriodicRefresh();
    this.partitionMap = null;
  }

  // ============================================
  // Private Methods
  // ============================================

  private handlePartitionMap(message: PartitionMapMessage): void {
    const newMap = message.payload;

    // Only accept newer versions
    if (this.partitionMap && newMap.version <= this.partitionMap.version) {
      logger.debug({
        current: this.partitionMap.version,
        received: newMap.version
      }, 'Ignoring older partition map');
      return;
    }

    this.partitionMap = newMap;
    this.lastRefreshTime = Date.now();

    // Update connection pool with node endpoints
    this.updateConnectionPool(newMap);

    const changesCount = newMap.partitions.length;
    logger.info({
      version: newMap.version,
      partitions: newMap.partitionCount,
      nodes: newMap.nodes.length
    }, 'Partition map updated');

    this.emit('partitionMap:updated', newMap.version, changesCount);
  }

  private handlePartitionMapDelta(message: PartitionMapDeltaMessage): void {
    const delta = message.payload;

    // Must have base map and correct previous version
    if (!this.partitionMap) {
      logger.warn('Received delta but no base map, requesting full map');
      this.refreshPartitionMap();
      return;
    }

    if (delta.previousVersion !== this.partitionMap.version) {
      logger.warn({
        expected: this.partitionMap.version,
        received: delta.previousVersion
      }, 'Delta version mismatch, requesting full map');
      this.refreshPartitionMap();
      return;
    }

    // Apply changes
    for (const change of delta.changes) {
      this.applyPartitionChange(change);
    }

    this.partitionMap.version = delta.version;
    this.lastRefreshTime = Date.now();

    logger.info({
      version: delta.version,
      changes: delta.changes.length
    }, 'Applied partition map delta');

    this.emit('partitionMap:updated', delta.version, delta.changes.length);
  }

  private applyPartitionChange(change: PartitionChange): void {
    if (!this.partitionMap) return;

    const partition = this.partitionMap.partitions.find(p => p.partitionId === change.partitionId);
    if (partition) {
      partition.ownerNodeId = change.newOwner;
      // Backups would also be updated but simplified here
    }
  }

  private updateConnectionPool(map: PartitionMap): void {
    // Add new nodes
    for (const node of map.nodes) {
      if (node.status === 'ACTIVE' || node.status === 'JOINING') {
        this.connectionPool.addNode(node.nodeId, node.endpoints.websocket);
      }
    }

    // Remove nodes that are no longer in the map
    const currentNodeIds = new Set(map.nodes.map(n => n.nodeId));
    for (const nodeId of this.connectionPool.getAllNodes()) {
      if (!currentNodeIds.has(nodeId)) {
        this.connectionPool.removeNode(nodeId);
      }
    }
  }

  private async doRefreshPartitionMap(): Promise<void> {
    logger.debug('Requesting partition map refresh');

    // Send request to any connected node
    const sent = this.connectionPool.sendToPrimary({
      type: 'PARTITION_MAP_REQUEST',
      payload: {
        currentVersion: this.getMapVersion(),
      },
    });

    if (!sent) {
      throw new Error('No connection available to request partition map');
    }

    // Wait for response (handled via message event)
    return new Promise((resolve, reject) => {
      const timeout = setTimeout(() => {
        this.removeListener('partitionMap:updated', onUpdate);
        reject(new Error('Partition map refresh timeout'));
      }, 5000);

      const onUpdate = () => {
        clearTimeout(timeout);
        this.removeListener('partitionMap:updated', onUpdate);
        resolve();
      };

      this.once('partitionMap:updated', onUpdate);
    });
  }
}
