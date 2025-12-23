/**
 * ClusterClient - Cluster-aware client wrapper
 *
 * Phase 4: Partition-Aware Client Routing
 *
 * Wraps the standard TopGunClient with cluster-aware routing capabilities.
 * Coordinates between ConnectionPool and PartitionRouter for optimal
 * request routing in a clustered environment.
 */

import {
  ClusterClientConfig,
  ConnectionPoolConfig,
  PartitionRouterConfig,
  DEFAULT_CONNECTION_POOL_CONFIG,
  DEFAULT_PARTITION_ROUTER_CONFIG,
  NodeHealth,
  serialize,
} from '@topgunbuild/core';
import { ConnectionPool } from './ConnectionPool';
import { PartitionRouter } from './PartitionRouter';
import { logger } from '../utils/logger';

export interface ClusterClientEvents {
  'connected': () => void;
  'disconnected': (reason: string) => void;
  'partitionMap:ready': (version: number) => void;
  'routing:active': () => void;
  'error': (error: Error) => void;
}

export type ClusterRoutingMode = 'direct' | 'forward';

export class ClusterClient {
  private readonly listeners: Map<string, Set<(...args: any[]) => void>> = new Map();
  private readonly connectionPool: ConnectionPool;
  private readonly partitionRouter: PartitionRouter;
  private readonly config: ClusterClientConfig;
  private initialized: boolean = false;
  private routingActive: boolean = false;

  constructor(config: ClusterClientConfig) {
    this.config = config;

    // Initialize connection pool
    const poolConfig: ConnectionPoolConfig = {
      ...DEFAULT_CONNECTION_POOL_CONFIG,
      ...config.connectionPool,
    };
    this.connectionPool = new ConnectionPool(poolConfig);

    // Initialize partition router
    const routerConfig: PartitionRouterConfig = {
      ...DEFAULT_PARTITION_ROUTER_CONFIG,
      fallbackMode: config.routingMode === 'direct' ? 'error' : 'forward',
      ...config.routing,
    };
    this.partitionRouter = new PartitionRouter(this.connectionPool, routerConfig);

    this.setupEventHandlers();
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

  public removeAllListeners(event?: string): this {
    if (event) {
      this.listeners.delete(event);
    } else {
      this.listeners.clear();
    }
    return this;
  }

  /**
   * Initialize cluster connections
   */
  public async start(): Promise<void> {
    if (this.initialized) return;

    logger.info({ seedNodes: this.config.seedNodes }, 'Starting cluster client');

    // Connect to seed nodes
    for (let i = 0; i < this.config.seedNodes.length; i++) {
      const endpoint = this.config.seedNodes[i];
      const nodeId = `seed-${i}`; // Temporary ID until we get real ID from handshake
      await this.connectionPool.addNode(nodeId, endpoint);
    }

    // Start health monitoring
    this.connectionPool.startHealthCheck();

    // Start periodic partition map refresh
    this.partitionRouter.startPeriodicRefresh();

    this.initialized = true;

    // Wait for first partition map
    await this.waitForPartitionMap();
  }

  /**
   * Set authentication token
   */
  public setAuthToken(token: string): void {
    this.connectionPool.setAuthToken(token);
  }

  /**
   * Send operation with automatic routing
   */
  public send(key: string, message: any): boolean {
    if (this.config.routingMode === 'direct' && this.routingActive) {
      return this.sendDirect(key, message);
    }
    return this.sendForward(message);
  }

  /**
   * Send directly to partition owner
   */
  public sendDirect(key: string, message: any): boolean {
    const connection = this.partitionRouter.routeToConnection(key);
    if (!connection) {
      logger.warn({ key }, 'No route available for key');
      return false;
    }

    // Add routing metadata
    const routedMessage = {
      ...message,
      _routing: {
        partitionId: this.partitionRouter.getPartitionId(key),
        mapVersion: this.partitionRouter.getMapVersion(),
      },
    };

    connection.socket.send(serialize(routedMessage));
    return true;
  }

  /**
   * Send to primary node for server-side forwarding
   */
  public sendForward(message: any): boolean {
    return this.connectionPool.sendToPrimary(message);
  }

  /**
   * Send batch of operations with routing
   */
  public sendBatch(operations: Array<{ key: string; message: any }>): Map<string, boolean> {
    const results = new Map<string, boolean>();

    if (this.config.routingMode === 'direct' && this.routingActive) {
      // Group by target node
      const nodeMessages = new Map<string, any[]>();

      for (const { key, message } of operations) {
        const routing = this.partitionRouter.route(key);
        const nodeId = routing?.nodeId ?? 'primary';

        if (!nodeMessages.has(nodeId)) {
          nodeMessages.set(nodeId, []);
        }
        nodeMessages.get(nodeId)!.push({ key, message });
      }

      // Send to each node
      for (const [nodeId, messages] of nodeMessages) {
        let success: boolean;
        if (nodeId === 'primary') {
          success = this.connectionPool.sendToPrimary({
            type: 'OP_BATCH',
            payload: { ops: messages.map(m => m.message) },
          });
        } else {
          success = this.connectionPool.send(nodeId, {
            type: 'OP_BATCH',
            payload: { ops: messages.map(m => m.message) },
          });
        }

        for (const { key } of messages) {
          results.set(key, success);
        }
      }
    } else {
      // Forward all to primary
      const success = this.connectionPool.sendToPrimary({
        type: 'OP_BATCH',
        payload: { ops: operations.map(o => o.message) },
      });

      for (const { key } of operations) {
        results.set(key, success);
      }
    }

    return results;
  }

  /**
   * Get connection pool health status
   */
  public getHealthStatus(): Map<string, NodeHealth> {
    return this.connectionPool.getHealthStatus();
  }

  /**
   * Get partition router stats
   */
  public getRouterStats(): ReturnType<PartitionRouter['getStats']> {
    return this.partitionRouter.getStats();
  }

  /**
   * Check if cluster routing is active
   */
  public isRoutingActive(): boolean {
    return this.routingActive;
  }

  /**
   * Get list of connected nodes
   */
  public getConnectedNodes(): string[] {
    return this.connectionPool.getConnectedNodes();
  }

  /**
   * Check if cluster client is initialized
   */
  public isInitialized(): boolean {
    return this.initialized;
  }

  /**
   * Force refresh of partition map
   */
  public async refreshPartitionMap(): Promise<void> {
    await this.partitionRouter.refreshPartitionMap();
  }

  /**
   * Shutdown cluster client
   */
  public close(): void {
    this.partitionRouter.close();
    this.connectionPool.close();
    this.initialized = false;
    this.routingActive = false;
    logger.info('Cluster client closed');
  }

  // ============================================
  // Internal Access for TopGunClient
  // ============================================

  /**
   * Get the connection pool (for internal use)
   */
  public getConnectionPool(): ConnectionPool {
    return this.connectionPool;
  }

  /**
   * Get the partition router (for internal use)
   */
  public getPartitionRouter(): PartitionRouter {
    return this.partitionRouter;
  }

  /**
   * Get any healthy WebSocket connection
   */
  public getAnyConnection(): WebSocket | null {
    const conn = this.connectionPool.getAnyHealthyConnection();
    return conn?.socket ?? null;
  }

  // ============================================
  // Private Methods
  // ============================================

  private setupEventHandlers(): void {
    // Connection pool events
    this.connectionPool.on('node:connected', (nodeId: string) => {
      logger.debug({ nodeId }, 'Node connected');
      if (this.connectionPool.getConnectedNodes().length === 1) {
        this.emit('connected');
      }
    });

    this.connectionPool.on('node:disconnected', (nodeId: string, reason: string) => {
      logger.debug({ nodeId, reason }, 'Node disconnected');
      if (this.connectionPool.getConnectedNodes().length === 0) {
        this.routingActive = false;
        this.emit('disconnected', reason);
      }
    });

    this.connectionPool.on('node:unhealthy', (nodeId: string, reason: string) => {
      logger.warn({ nodeId, reason }, 'Node unhealthy');
    });

    this.connectionPool.on('error', (nodeId: string, error: Error) => {
      this.emit('error', error);
    });

    // Partition router events
    this.partitionRouter.on('partitionMap:updated', (version: number, changesCount: number) => {
      if (!this.routingActive && this.partitionRouter.hasPartitionMap()) {
        this.routingActive = true;
        logger.info({ version }, 'Direct routing activated');
        this.emit('routing:active');
      }
      this.emit('partitionMap:ready', version);
    });

    this.partitionRouter.on('routing:miss', (key: string, expected: string, actual: string) => {
      logger.debug({ key, expected, actual }, 'Routing miss detected');
    });
  }

  private async waitForPartitionMap(timeoutMs: number = 10000): Promise<void> {
    if (this.partitionRouter.hasPartitionMap()) {
      this.routingActive = true;
      return;
    }

    return new Promise((resolve) => {
      const timeout = setTimeout(() => {
        this.partitionRouter.off('partitionMap:updated', onUpdate);
        // Don't reject - fallback mode will be used
        logger.warn('Partition map not received, using fallback routing');
        resolve();
      }, timeoutMs);

      const onUpdate = () => {
        clearTimeout(timeout);
        this.partitionRouter.off('partitionMap:updated', onUpdate);
        this.routingActive = true;
        resolve();
      };

      this.partitionRouter.once('partitionMap:updated', onUpdate);
    });
  }
}
