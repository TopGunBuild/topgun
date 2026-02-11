/**
 * ConnectionPool - Manages WebSocket connections to multiple cluster nodes
 *
 * Features:
 * - Maintains connections to all known cluster nodes
 * - Automatic reconnection with exponential backoff
 * - Health monitoring and status tracking
 * - Connection lifecycle management
 */

import {
  ConnectionPoolConfig,
  DEFAULT_CONNECTION_POOL_CONFIG,
  ConnectionState,
  NodeHealth,
} from '@topgunbuild/core';
import { serialize, deserialize } from '@topgunbuild/core';
import type { IConnection } from '../types';
import { WebSocketConnection } from '../connection/WebSocketConnection';
import { logger } from '../utils/logger';

export type ConnectionPoolEventType =
  | 'node:connected'
  | 'node:disconnected'
  | 'node:healthy'
  | 'node:unhealthy'
  | 'node:remapped'
  | 'message'
  | 'error';

export interface ConnectionPoolEvents {
  'node:connected': (nodeId: string) => void;
  'node:disconnected': (nodeId: string, reason: string) => void;
  'node:healthy': (nodeId: string) => void;
  'node:unhealthy': (nodeId: string, reason: string) => void;
  'node:remapped': (oldId: string, newId: string) => void;
  'message': (nodeId: string, message: any) => void;
  'error': (nodeId: string, error: Error) => void;
}

interface NodeConnection {
  nodeId: string;
  endpoint: string;
  socket: WebSocket | null;
  cachedConnection: WebSocketConnection | null;
  state: ConnectionState;
  lastSeen: number;
  latencyMs: number;
  reconnectAttempts: number;
  reconnectTimer: ReturnType<typeof setTimeout> | null;
  pendingMessages: Uint8Array[];
}

export class ConnectionPool {
  private readonly listeners: Map<string, Set<(...args: any[]) => void>> = new Map();
  private readonly config: ConnectionPoolConfig;
  private readonly connections: Map<string, NodeConnection> = new Map();
  private primaryNodeId: string | null = null;
  private healthCheckTimer: ReturnType<typeof setInterval> | null = null;
  private authToken: string | null = null;

  constructor(config: Partial<ConnectionPoolConfig> = {}) {
    this.config = {
      ...DEFAULT_CONNECTION_POOL_CONFIG,
      ...config,
    };
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
   * Set authentication token for all connections
   */
  public setAuthToken(token: string): void {
    this.authToken = token;
    // Re-authenticate existing connections
    for (const conn of this.connections.values()) {
      if (conn.state === 'CONNECTED') {
        this.sendAuth(conn);
      }
    }
  }

  /**
   * Add a node to the connection pool
   */
  public async addNode(nodeId: string, endpoint: string): Promise<void> {
    if (this.connections.has(nodeId)) {
      const existing = this.connections.get(nodeId)!;
      if (existing.endpoint !== endpoint) {
        // Endpoint changed, reconnect
        await this.removeNode(nodeId);
      } else {
        return; // Already connected
      }
    }

    // Check if an existing connection has the same endpoint under a different ID
    // (e.g., seed-0 needs to be remapped to the server-assigned node ID)
    for (const [existingId, existingConn] of this.connections) {
      if (existingConn.endpoint === endpoint && existingId !== nodeId) {
        this.remapNodeId(existingId, nodeId);
        return;
      }
    }

    const connection: NodeConnection = {
      nodeId,
      endpoint,
      socket: null,
      cachedConnection: null,
      state: 'DISCONNECTED',
      lastSeen: 0,
      latencyMs: 0,
      reconnectAttempts: 0,
      reconnectTimer: null,
      pendingMessages: [],
    };

    this.connections.set(nodeId, connection);

    // Set first node as primary
    if (!this.primaryNodeId) {
      this.primaryNodeId = nodeId;
    }

    await this.connect(nodeId);
  }

  /**
   * Remove a node from the connection pool
   */
  public async removeNode(nodeId: string): Promise<void> {
    const connection = this.connections.get(nodeId);
    if (!connection) return;

    // Clear reconnect timer
    if (connection.reconnectTimer) {
      clearTimeout(connection.reconnectTimer);
      connection.reconnectTimer = null;
    }

    // Close socket
    if (connection.socket) {
      connection.socket.onclose = null; // Prevent reconnect
      connection.socket.close();
      connection.socket = null;
    }

    this.connections.delete(nodeId);

    // Update primary if needed
    if (this.primaryNodeId === nodeId) {
      this.primaryNodeId = this.connections.size > 0
        ? this.connections.keys().next().value ?? null
        : null;
    }

    logger.info({ nodeId }, 'Node removed from connection pool');
  }

  /**
   * Remap a node from one ID to another, preserving the existing connection.
   * Used when the server-assigned node ID differs from the temporary seed ID.
   */
  public remapNodeId(oldId: string, newId: string): void {
    const connection = this.connections.get(oldId);
    if (!connection) return;

    // Transfer the entry under the new key
    connection.nodeId = newId;
    this.connections.delete(oldId);
    this.connections.set(newId, connection);

    // Update primary if the remapped node was primary
    if (this.primaryNodeId === oldId) {
      this.primaryNodeId = newId;
    }

    logger.info({ oldId, newId }, 'Node ID remapped');
    this.emit('node:remapped', oldId, newId);
  }

  /**
   * Get connection for a specific node
   */
  public getConnection(nodeId: string): IConnection | null {
    const connection = this.connections.get(nodeId);
    if (
      !connection ||
      (connection.state !== 'CONNECTED' && connection.state !== 'AUTHENTICATED') ||
      !connection.socket
    ) {
      return null;
    }
    if (!connection.cachedConnection) {
      connection.cachedConnection = new WebSocketConnection(connection.socket);
    }
    return connection.cachedConnection;
  }

  /**
   * Get primary connection (first/seed node)
   */
  public getPrimaryConnection(): IConnection | null {
    if (!this.primaryNodeId) return null;
    return this.getConnection(this.primaryNodeId);
  }

  /**
   * Get any healthy connection
   */
  public getAnyHealthyConnection(): { nodeId: string; connection: IConnection } | null {
    for (const [nodeId, conn] of this.connections) {
      if ((conn.state === 'CONNECTED' || conn.state === 'AUTHENTICATED') && conn.socket) {
        if (!conn.cachedConnection) {
          conn.cachedConnection = new WebSocketConnection(conn.socket);
        }
        return { nodeId, connection: conn.cachedConnection };
      }
    }
    return null;
  }

  /**
   * Send message to a specific node
   */
  public send(nodeId: string, message: any): boolean {
    const connection = this.connections.get(nodeId);
    if (!connection) {
      logger.warn({ nodeId }, 'Cannot send: node not in pool');
      return false;
    }

    const data = serialize(message);

    if (connection.state === 'AUTHENTICATED' && connection.socket?.readyState === WebSocket.OPEN) {
      connection.socket.send(data);
      return true;
    }

    // Queue message for later
    if (connection.pendingMessages.length < 1000) {
      connection.pendingMessages.push(data);
      return true;
    }

    logger.warn({ nodeId }, 'Message queue full, dropping message');
    return false;
  }

  /**
   * Send message to primary node
   */
  public sendToPrimary(message: any): boolean {
    if (!this.primaryNodeId) {
      logger.warn('No primary node available');
      return false;
    }
    return this.send(this.primaryNodeId, message);
  }

  /**
   * Get health status for all nodes
   */
  public getHealthStatus(): Map<string, NodeHealth> {
    const status = new Map<string, NodeHealth>();
    for (const [nodeId, conn] of this.connections) {
      status.set(nodeId, {
        nodeId,
        state: conn.state,
        lastSeen: conn.lastSeen,
        latencyMs: conn.latencyMs,
        reconnectAttempts: conn.reconnectAttempts,
      });
    }
    return status;
  }

  /**
   * Get list of connected node IDs
   */
  public getConnectedNodes(): string[] {
    return Array.from(this.connections.entries())
      .filter(([_, conn]) => conn.state === 'CONNECTED' || conn.state === 'AUTHENTICATED')
      .map(([nodeId]) => nodeId);
  }

  /**
   * Get all node IDs
   */
  public getAllNodes(): string[] {
    return Array.from(this.connections.keys());
  }

  /**
   * Check if node has an open WebSocket connection
   */
  public isNodeConnected(nodeId: string): boolean {
    const conn = this.connections.get(nodeId);
    return conn?.state === 'CONNECTED' || conn?.state === 'AUTHENTICATED';
  }

  /**
   * Check if connected to a specific node.
   * Alias for isNodeConnected() for IConnectionProvider compatibility.
   */
  public isConnected(nodeId: string): boolean {
    return this.isNodeConnected(nodeId);
  }

  /**
   * Start health monitoring
   */
  public startHealthCheck(): void {
    if (this.healthCheckTimer) return;

    this.healthCheckTimer = setInterval(() => {
      this.performHealthCheck();
    }, this.config.healthCheckIntervalMs);
  }

  /**
   * Stop health monitoring
   */
  public stopHealthCheck(): void {
    if (this.healthCheckTimer) {
      clearInterval(this.healthCheckTimer);
      this.healthCheckTimer = null;
    }
  }

  /**
   * Close all connections and cleanup
   */
  public close(): void {
    this.stopHealthCheck();

    for (const nodeId of this.connections.keys()) {
      this.removeNode(nodeId);
    }

    this.connections.clear();
    this.primaryNodeId = null;
  }

  // ============================================
  // Private Methods
  // ============================================

  private async connect(nodeId: string): Promise<void> {
    const connection = this.connections.get(nodeId);
    if (!connection) return;

    if (connection.state === 'CONNECTING' || connection.state === 'CONNECTED') {
      return;
    }

    connection.state = 'CONNECTING';
    logger.info({ nodeId, endpoint: connection.endpoint }, 'Connecting to node');

    try {
      const socket = new WebSocket(connection.endpoint);
      socket.binaryType = 'arraybuffer';
      connection.socket = socket;

      socket.onopen = () => {
        connection.state = 'CONNECTED';
        connection.reconnectAttempts = 0;
        connection.lastSeen = Date.now();
        logger.info({ nodeId }, 'Connected to node');
        this.emit('node:connected', nodeId);

        // Send auth if we have token
        if (this.authToken) {
          this.sendAuth(connection);
        }

        // Flush pending messages after auth
        // Note: Messages will be sent after AUTH_ACK
      };

      socket.onmessage = (event) => {
        connection.lastSeen = Date.now();
        this.handleMessage(nodeId, event);
      };

      socket.onerror = (error) => {
        logger.error({ nodeId, error }, 'WebSocket error');
        this.emit('error', nodeId, error instanceof Error ? error : new Error('WebSocket error'));
      };

      socket.onclose = () => {
        const wasConnected = connection.state === 'AUTHENTICATED';
        connection.state = 'DISCONNECTED';
        connection.socket = null;
        connection.cachedConnection = null;

        if (wasConnected) {
          this.emit('node:disconnected', nodeId, 'Connection closed');
        }

        // Schedule reconnect
        this.scheduleReconnect(nodeId);
      };

    } catch (error) {
      connection.state = 'FAILED';
      logger.error({ nodeId, error }, 'Failed to connect');
      this.scheduleReconnect(nodeId);
    }
  }

  private sendAuth(connection: NodeConnection): void {
    if (!this.authToken || !connection.socket) return;

    connection.socket.send(serialize({
      type: 'AUTH',
      token: this.authToken,
    }));
  }

  private handleMessage(nodeId: string, event: MessageEvent): void {
    const connection = this.connections.get(nodeId);
    if (!connection) return;

    let message: any;
    try {
      if (event.data instanceof ArrayBuffer) {
        message = deserialize(new Uint8Array(event.data));
      } else {
        message = JSON.parse(event.data);
      }
    } catch (e) {
      logger.error({ nodeId, error: e }, 'Failed to parse message');
      return;
    }

    // Handle auth-related side effects (state tracking and pending message flush)
    if (message.type === 'AUTH_ACK') {
      connection.state = 'AUTHENTICATED';
      logger.info({ nodeId }, 'Authenticated with node');
      this.emit('node:healthy', nodeId);
      this.flushPendingMessages(connection);
    }

    if (message.type === 'AUTH_REQUIRED') {
      if (this.authToken) {
        this.sendAuth(connection);
      }
    }

    if (message.type === 'AUTH_FAIL') {
      logger.error({ nodeId, error: message.error }, 'Authentication failed');
      connection.state = 'FAILED';
    }

    if (message.type === 'PONG') {
      if (message.timestamp) {
        connection.latencyMs = Date.now() - message.timestamp;
      }
      return;
    }

    // Forward all messages (including auth messages) to listeners
    this.emit('message', nodeId, message);
  }

  private flushPendingMessages(connection: NodeConnection): void {
    if (!connection.socket || connection.state !== 'AUTHENTICATED') return;

    const pending = connection.pendingMessages;
    connection.pendingMessages = [];

    for (const data of pending) {
      if (connection.socket.readyState === WebSocket.OPEN) {
        connection.socket.send(data);
      }
    }

    if (pending.length > 0) {
      logger.debug({ nodeId: connection.nodeId, count: pending.length }, 'Flushed pending messages');
    }
  }

  private scheduleReconnect(nodeId: string): void {
    const connection = this.connections.get(nodeId);
    if (!connection) return;

    // Clear existing timer
    if (connection.reconnectTimer) {
      clearTimeout(connection.reconnectTimer);
      connection.reconnectTimer = null;
    }

    // Check max attempts
    if (connection.reconnectAttempts >= this.config.maxReconnectAttempts) {
      connection.state = 'FAILED';
      logger.error({ nodeId, attempts: connection.reconnectAttempts }, 'Max reconnect attempts reached');
      this.emit('node:unhealthy', nodeId, 'Max reconnect attempts reached');
      return;
    }

    // Calculate backoff delay
    const delay = Math.min(
      this.config.reconnectDelayMs * Math.pow(2, connection.reconnectAttempts),
      this.config.maxReconnectDelayMs
    );

    connection.state = 'RECONNECTING';
    connection.reconnectAttempts++;

    logger.info({ nodeId, delay, attempt: connection.reconnectAttempts }, 'Scheduling reconnect');

    connection.reconnectTimer = setTimeout(() => {
      connection.reconnectTimer = null;
      this.connect(nodeId);
    }, delay);
  }

  private performHealthCheck(): void {
    const now = Date.now();

    for (const [nodeId, connection] of this.connections) {
      // Skip nodes that are not authenticated
      if (connection.state !== 'AUTHENTICATED') continue;

      // Check staleness
      const timeSinceLastSeen = now - connection.lastSeen;
      if (timeSinceLastSeen > this.config.healthCheckIntervalMs * 3) {
        logger.warn({ nodeId, timeSinceLastSeen }, 'Node appears stale, sending ping');
      }

      // Send ping
      if (connection.socket?.readyState === WebSocket.OPEN) {
        connection.socket.send(serialize({
          type: 'PING',
          timestamp: now,
        }));
      }
    }
  }
}
