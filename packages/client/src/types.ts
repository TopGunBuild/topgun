/**
 * Connection Provider Types
 *
 * IConnectionProvider abstracts connection handling to support
 * both single-server and cluster modes across WebSocket and HTTP transports.
 */

/**
 * Minimal connection interface capturing the actual usage pattern.
 * Allows transport-agnostic code without depending on the WebSocket global.
 */
export interface IConnection {
  send(data: ArrayBuffer | Uint8Array | string): void;
  close(): void;
  readonly readyState: number;
}

/**
 * Events emitted by IConnectionProvider.
 */
export type ConnectionProviderEvent =
  | 'connected'
  | 'disconnected'
  | 'reconnected'
  | 'message'
  | 'partitionMapUpdated'
  | 'error';

/**
 * Connection event handler type.
 */
export type ConnectionEventHandler = (...args: any[]) => void;

/**
 * Abstract interface for connection providers.
 *
 * Implementations:
 * - SingleServerProvider: Direct connection to a single server
 * - ClusterClient: Multi-node connection pool with partition routing
 * - HttpSyncProvider: HTTP polling for serverless environments
 * - AutoConnectionProvider: WebSocket-to-HTTP fallback
 */
export interface IConnectionProvider {
  /**
   * Connect to the server(s).
   * In cluster mode, connects to all seed nodes.
   */
  connect(): Promise<void>;

  /**
   * Get connection for a specific key.
   * In cluster mode: routes to partition owner based on key hash.
   * In single-server mode: returns the only connection.
   *
   * @param key - The key to route (used for partition-aware routing)
   * @throws Error if not connected
   */
  getConnection(key: string): IConnection;

  /**
   * Get any available connection.
   * Used for subscriptions, metadata requests, and non-key-specific operations.
   *
   * @throws Error if not connected
   */
  getAnyConnection(): IConnection;

  /**
   * Check if at least one connection is active and ready.
   */
  isConnected(): boolean;

  /**
   * Get all connected node IDs.
   * Single-server mode returns ['default'].
   * Cluster mode returns actual node IDs.
   */
  getConnectedNodes(): string[];

  /**
   * Subscribe to connection events.
   *
   * Events:
   * - 'connected': A connection was established (nodeId?: string)
   * - 'disconnected': A connection was lost (nodeId?: string)
   * - 'reconnected': A connection was re-established after disconnect (nodeId?: string)
   * - 'message': A message was received (nodeId: string, data: any)
   * - 'partitionMapUpdated': Partition map was updated (cluster mode only)
   * - 'error': An error occurred (error: Error)
   */
  on(event: ConnectionProviderEvent, handler: ConnectionEventHandler): void;

  /**
   * Unsubscribe from connection events.
   */
  off(event: ConnectionProviderEvent, handler: ConnectionEventHandler): void;

  /**
   * Send a message via the appropriate connection.
   * In cluster mode, routes based on key if provided.
   *
   * @param data - Serialized message data
   * @param key - Optional key for routing (cluster mode)
   */
  send(data: ArrayBuffer | Uint8Array, key?: string): void;

  /**
   * Close all connections gracefully.
   */
  close(): Promise<void>;
}

/**
 * Configuration for SingleServerProvider.
 */
export interface SingleServerProviderConfig {
  /** WebSocket URL to connect to */
  url: string;

  /** Maximum reconnection attempts (default: 10) */
  maxReconnectAttempts?: number;

  /** Initial reconnect delay in ms (default: 1000) */
  reconnectDelayMs?: number;

  /** Backoff multiplier for reconnect delay (default: 2) */
  backoffMultiplier?: number;

  /** Maximum reconnect delay in ms (default: 30000) */
  maxReconnectDelayMs?: number;
}
