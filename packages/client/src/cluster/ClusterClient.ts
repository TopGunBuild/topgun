/**
 * ClusterClient - Cluster-aware client wrapper
 *
 * Phase 4: Partition-Aware Client Routing
 * Phase 4.5: Implements IConnectionProvider for SyncEngine abstraction
 *
 * Wraps the standard TopGunClient with cluster-aware routing capabilities.
 * Coordinates between ConnectionPool and PartitionRouter for optimal
 * request routing in a clustered environment.
 */

import {
  ClusterClientConfig,
  ConnectionPoolConfig,
  PartitionRouterConfig,
  CircuitBreakerConfig,
  DEFAULT_CONNECTION_POOL_CONFIG,
  DEFAULT_PARTITION_ROUTER_CONFIG,
  DEFAULT_CIRCUIT_BREAKER_CONFIG,
  NodeHealth,
  serialize,
} from '@topgunbuild/core';
import { ConnectionPool } from './ConnectionPool';
import { PartitionRouter } from './PartitionRouter';
import { logger } from '../utils/logger';
import type { IConnectionProvider, ConnectionProviderEvent, ConnectionEventHandler } from '../types';

export interface ClusterClientEvents {
  'connected': () => void;
  'disconnected': (reason: string) => void;
  'partitionMap:ready': (version: number) => void;
  'routing:active': () => void;
  'error': (error: Error) => void;
  'circuit:open': (nodeId: string) => void;
  'circuit:closed': (nodeId: string) => void;
  'circuit:half-open': (nodeId: string) => void;
}

/**
 * Circuit breaker state for a node.
 */
export interface CircuitState {
  /** Number of consecutive failures */
  failures: number;
  /** Timestamp of last failure */
  lastFailure: number;
  /** Current circuit state */
  state: 'closed' | 'open' | 'half-open';
}

/**
 * Routing metrics for monitoring smart routing effectiveness.
 */
export interface RoutingMetrics {
  /** Operations routed directly to partition owner */
  directRoutes: number;
  /** Operations falling back to any node (owner unavailable) */
  fallbackRoutes: number;
  /** Operations when partition map is missing/stale */
  partitionMisses: number;
  /** Total routing decisions made */
  totalRoutes: number;
}

export type ClusterRoutingMode = 'direct' | 'forward';

/**
 * ClusterClient implements IConnectionProvider for multi-node cluster mode.
 * It provides partition-aware routing and connection management.
 */
export class ClusterClient implements IConnectionProvider {
  private readonly listeners: Map<string, Set<(...args: any[]) => void>> = new Map();
  private readonly connectionPool: ConnectionPool;
  private readonly partitionRouter: PartitionRouter;
  private readonly config: ClusterClientConfig;
  private initialized: boolean = false;
  private routingActive: boolean = false;
  private readonly routingMetrics: RoutingMetrics = {
    directRoutes: 0,
    fallbackRoutes: 0,
    partitionMisses: 0,
    totalRoutes: 0,
  };

  // Circuit breaker state per node
  private readonly circuits: Map<string, CircuitState> = new Map();
  private readonly circuitBreakerConfig: CircuitBreakerConfig;

  constructor(config: ClusterClientConfig) {
    this.config = config;

    // Initialize circuit breaker config
    this.circuitBreakerConfig = {
      ...DEFAULT_CIRCUIT_BREAKER_CONFIG,
      ...config.circuitBreaker,
    };

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

  // ============================================
  // IConnectionProvider Implementation
  // ============================================

  /**
   * Connect to cluster nodes (IConnectionProvider interface).
   * Alias for start() method.
   */
  public async connect(): Promise<void> {
    return this.start();
  }

  /**
   * Get connection for a specific key (IConnectionProvider interface).
   * Routes to partition owner based on key hash when smart routing is enabled.
   * @throws Error if not connected
   */
  public getConnection(key: string): WebSocket {
    if (!this.isConnected()) {
      throw new Error('ClusterClient not connected');
    }

    this.routingMetrics.totalRoutes++;

    // If not in direct routing mode or routing not active, use fallback
    if (this.config.routingMode !== 'direct' || !this.routingActive) {
      this.routingMetrics.fallbackRoutes++;
      return this.getFallbackConnection();
    }

    // Try to route to partition owner
    const routing = this.partitionRouter.route(key);

    // No partition map available
    if (!routing) {
      this.routingMetrics.partitionMisses++;
      logger.debug({ key }, 'No partition map available, using fallback');
      return this.getFallbackConnection();
    }

    const owner = routing.nodeId;

    // Check if owner is connected
    if (!this.connectionPool.isNodeConnected(owner)) {
      this.routingMetrics.fallbackRoutes++;
      logger.debug({ key, owner }, 'Partition owner not connected, using fallback');
      // Request partition map refresh since owner might have changed
      this.requestPartitionMapRefresh();
      return this.getFallbackConnection();
    }

    // Get connection to owner
    const socket = this.connectionPool.getConnection(owner);
    if (!socket) {
      this.routingMetrics.fallbackRoutes++;
      logger.debug({ key, owner }, 'Could not get connection to owner, using fallback');
      return this.getFallbackConnection();
    }

    this.routingMetrics.directRoutes++;
    return socket;
  }

  /**
   * Get fallback connection when owner is unavailable.
   * @throws Error if no connection available
   */
  private getFallbackConnection(): WebSocket {
    const conn = this.connectionPool.getAnyHealthyConnection();
    if (!conn?.socket) {
      throw new Error('No healthy connection available');
    }
    return conn.socket;
  }

  /**
   * Request a partition map refresh in the background.
   * Called when routing to an unknown/disconnected owner.
   */
  private requestPartitionMapRefresh(): void {
    this.partitionRouter.refreshPartitionMap().catch(err => {
      logger.error({ err }, 'Failed to refresh partition map');
    });
  }

  /**
   * Request partition map from a specific node.
   * Called on first node connection.
   */
  private requestPartitionMapFromNode(nodeId: string): void {
    const socket = this.connectionPool.getConnection(nodeId);
    if (socket) {
      logger.debug({ nodeId }, 'Requesting partition map from node');
      socket.send(serialize({
        type: 'PARTITION_MAP_REQUEST',
        payload: {
          currentVersion: this.partitionRouter.getMapVersion(),
        },
      }));
    }
  }

  /**
   * Check if at least one connection is active (IConnectionProvider interface).
   */
  public isConnected(): boolean {
    return this.connectionPool.getConnectedNodes().length > 0;
  }

  /**
   * Send data via the appropriate connection (IConnectionProvider interface).
   * Routes based on key if provided.
   */
  public send(data: ArrayBuffer | Uint8Array, key?: string): void {
    if (!this.isConnected()) {
      throw new Error('ClusterClient not connected');
    }

    const socket = key ? this.getConnection(key) : this.getAnyConnection();
    socket.send(data);
  }

  /**
   * Send data with automatic retry and rerouting on failure.
   * @param data - Data to send
   * @param key - Optional key for routing
   * @param options - Retry options
   * @throws Error after max retries exceeded
   */
  public async sendWithRetry(
    data: ArrayBuffer | Uint8Array,
    key?: string,
    options: {
      maxRetries?: number;
      retryDelayMs?: number;
      retryOnNotOwner?: boolean;
    } = {}
  ): Promise<void> {
    const {
      maxRetries = 3,
      retryDelayMs = 100,
      retryOnNotOwner = true,
    } = options;

    let lastError: Error | null = null;
    let nodeId: string | null = null;

    for (let attempt = 0; attempt < maxRetries; attempt++) {
      try {
        // Get the target node for circuit breaker tracking
        if (key && this.routingActive) {
          const routing = this.partitionRouter.route(key);
          nodeId = routing?.nodeId ?? null;
        }

        // Check circuit breaker
        if (nodeId && !this.canUseNode(nodeId)) {
          logger.debug({ nodeId, attempt }, 'Circuit open, using fallback');
          nodeId = null; // Force fallback
        }

        // Get connection and send
        const socket = key && nodeId
          ? this.connectionPool.getConnection(nodeId)
          : this.getAnyConnection();

        if (!socket) {
          throw new Error('No connection available');
        }

        socket.send(data);

        // Record success if using a specific node
        if (nodeId) {
          this.recordSuccess(nodeId);
        }

        return; // Success
      } catch (error) {
        lastError = error as Error;

        // Record failure if using a specific node
        if (nodeId) {
          this.recordFailure(nodeId);
        }

        const errorCode = (error as any)?.code;

        // Check if error is retryable
        if (this.isRetryableError(error)) {
          logger.debug(
            { attempt, maxRetries, errorCode, nodeId },
            'Retryable error, will retry'
          );

          // Handle specific error types
          if (errorCode === 'NOT_OWNER' && retryOnNotOwner) {
            // Wait for partition map update
            await this.waitForPartitionMapUpdateInternal(2000);
          } else if (errorCode === 'CONNECTION_CLOSED' || !this.isConnected()) {
            // Wait for reconnection
            await this.waitForConnectionInternal(5000);
          }

          // Small delay before retry
          await this.delay(retryDelayMs * (attempt + 1)); // Exponential backoff
          continue;
        }

        // Non-retryable error, fail immediately
        throw error;
      }
    }

    throw new Error(
      `Operation failed after ${maxRetries} retries: ${lastError?.message}`
    );
  }

  /**
   * Check if an error is retryable.
   */
  private isRetryableError(error: any): boolean {
    const code = error?.code;
    const message = error?.message || '';

    return (
      code === 'NOT_OWNER' ||
      code === 'CONNECTION_CLOSED' ||
      code === 'TIMEOUT' ||
      code === 'ECONNRESET' ||
      message.includes('No active connections') ||
      message.includes('No connection available') ||
      message.includes('No healthy connection')
    );
  }

  /**
   * Wait for partition map update.
   */
  private waitForPartitionMapUpdateInternal(timeoutMs: number): Promise<void> {
    return new Promise((resolve) => {
      const timeout = setTimeout(resolve, timeoutMs);

      const handler = () => {
        clearTimeout(timeout);
        this.off('partitionMapUpdated', handler);
        resolve();
      };

      this.on('partitionMapUpdated', handler);
    });
  }

  /**
   * Wait for at least one connection to be available.
   */
  private waitForConnectionInternal(timeoutMs: number): Promise<void> {
    return new Promise((resolve, reject) => {
      if (this.isConnected()) {
        resolve();
        return;
      }

      const timeout = setTimeout(() => {
        this.off('connected', handler);
        reject(new Error('Connection timeout'));
      }, timeoutMs);

      const handler = () => {
        clearTimeout(timeout);
        this.off('connected', handler);
        resolve();
      };

      this.on('connected', handler);
    });
  }

  /**
   * Helper delay function.
   */
  private delay(ms: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, ms));
  }

  // ============================================
  // Cluster-Specific Methods
  // ============================================

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
   * Send operation with automatic routing (legacy API for cluster operations).
   * @deprecated Use send(data, key) for IConnectionProvider interface
   */
  public sendMessage(key: string, message: any): boolean {
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
   * Get routing metrics for monitoring smart routing effectiveness.
   */
  public getRoutingMetrics(): RoutingMetrics {
    return { ...this.routingMetrics };
  }

  /**
   * Reset routing metrics counters.
   * Useful for monitoring intervals.
   */
  public resetRoutingMetrics(): void {
    this.routingMetrics.directRoutes = 0;
    this.routingMetrics.fallbackRoutes = 0;
    this.routingMetrics.partitionMisses = 0;
    this.routingMetrics.totalRoutes = 0;
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
   * Shutdown cluster client (IConnectionProvider interface).
   */
  public async close(): Promise<void> {
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
   * Get any healthy WebSocket connection (IConnectionProvider interface).
   * @throws Error if not connected
   */
  public getAnyConnection(): WebSocket {
    const conn = this.connectionPool.getAnyHealthyConnection();
    if (!conn?.socket) {
      throw new Error('No healthy connection available');
    }
    return conn.socket;
  }

  /**
   * Get any healthy WebSocket connection, or null if none available.
   * Use this for optional connection checks.
   */
  public getAnyConnectionOrNull(): WebSocket | null {
    const conn = this.connectionPool.getAnyHealthyConnection();
    return conn?.socket ?? null;
  }

  // ============================================
  // Circuit Breaker Methods
  // ============================================

  /**
   * Get circuit breaker state for a node.
   */
  public getCircuit(nodeId: string): CircuitState {
    let circuit = this.circuits.get(nodeId);
    if (!circuit) {
      circuit = { failures: 0, lastFailure: 0, state: 'closed' };
      this.circuits.set(nodeId, circuit);
    }
    return circuit;
  }

  /**
   * Check if a node can be used (circuit not open).
   */
  public canUseNode(nodeId: string): boolean {
    const circuit = this.getCircuit(nodeId);

    if (circuit.state === 'closed') {
      return true;
    }

    if (circuit.state === 'open') {
      // Check if reset timeout elapsed
      if (Date.now() - circuit.lastFailure > this.circuitBreakerConfig.resetTimeoutMs) {
        circuit.state = 'half-open';
        logger.debug({ nodeId }, 'Circuit breaker half-open, allowing test request');
        this.emit('circuit:half-open', nodeId);
        return true; // Allow one test request
      }
      return false;
    }

    // half-open: allow requests
    return true;
  }

  /**
   * Record a successful operation to a node.
   * Resets circuit breaker on success.
   */
  public recordSuccess(nodeId: string): void {
    const circuit = this.getCircuit(nodeId);
    const wasOpen = circuit.state !== 'closed';

    circuit.failures = 0;
    circuit.state = 'closed';

    if (wasOpen) {
      logger.info({ nodeId }, 'Circuit breaker closed after success');
      this.emit('circuit:closed', nodeId);
    }
  }

  /**
   * Record a failed operation to a node.
   * Opens circuit breaker after threshold failures.
   */
  public recordFailure(nodeId: string): void {
    const circuit = this.getCircuit(nodeId);
    circuit.failures++;
    circuit.lastFailure = Date.now();

    if (circuit.failures >= this.circuitBreakerConfig.failureThreshold) {
      if (circuit.state !== 'open') {
        circuit.state = 'open';
        logger.warn({ nodeId, failures: circuit.failures }, 'Circuit breaker opened');
        this.emit('circuit:open', nodeId);
      }
    }
  }

  /**
   * Get all circuit breaker states.
   */
  public getCircuitStates(): Map<string, CircuitState> {
    return new Map(this.circuits);
  }

  /**
   * Reset circuit breaker for a specific node.
   */
  public resetCircuit(nodeId: string): void {
    this.circuits.delete(nodeId);
    logger.debug({ nodeId }, 'Circuit breaker reset');
  }

  /**
   * Reset all circuit breakers.
   */
  public resetAllCircuits(): void {
    this.circuits.clear();
    logger.debug('All circuit breakers reset');
  }

  // ============================================
  // Private Methods
  // ============================================

  private setupEventHandlers(): void {
    // Connection pool events
    this.connectionPool.on('node:connected', (nodeId: string) => {
      logger.debug({ nodeId }, 'Node connected');

      // Request partition map on first connection if not already received
      if (this.partitionRouter.getMapVersion() === 0) {
        this.requestPartitionMapFromNode(nodeId);
      }

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

    // Forward messages from connection pool
    this.connectionPool.on('message', (nodeId: string, data: any) => {
      this.emit('message', nodeId, data);
    });

    // Partition router events
    this.partitionRouter.on('partitionMap:updated', (version: number, changesCount: number) => {
      if (!this.routingActive && this.partitionRouter.hasPartitionMap()) {
        this.routingActive = true;
        logger.info({ version }, 'Direct routing activated');
        this.emit('routing:active');
      }
      this.emit('partitionMap:ready', version);
      // Emit IConnectionProvider compatible event
      this.emit('partitionMapUpdated');
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
