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
// eslint-disable-next-line @typescript-eslint/no-explicit-any -- connection event handlers receive heterogeneous args depending on event type (nodeId, data, error); a discriminated union would require one handler type per event
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
   * Send a batch of keyed operations with per-key routing.
   * Optional -- when implemented (e.g. by ClusterClient), SyncEngine delegates
   * batch sending here so each operation is routed to the correct partition owner.
   * When absent, SyncEngine falls back to sending all ops in a single OP_BATCH.
   *
   * A provider that implements this MUST call `onBatchSent` synchronously (before
   * `sendBatch` returns), once per `OP_BATCH` frame it hands to a connection, with
   * the ids of that frame's operations in frame order. A frame whose hand-off
   * failed is not reported. The engine retires operations on an `OP_ACK` without
   * `results` only by matching it to a reported batch (TG-SYNC-004), so a batch
   * that is not reported cannot be acknowledged by such an `OP_ACK`: its
   * operations stay pending and are re-sent on every flush.
   *
   * @param operations - Array of { key, message } pairs where key is the routing key
   * @param onBatchSent - Reports the operation ids of one `OP_BATCH` frame handed to a connection
   * @returns Map of key -> success boolean
   */
  sendBatch?(
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- batch message shape varies by operation type; msgpack serialization happens after routing, so the message is still an untyped object here
    operations: Array<{ key: string; message: any }>,
    onBatchSent?: (opIds: string[]) => void,
  ): Map<string, boolean>;

  /**
   * The transport this provider's frames travel over.
   *
   * It decides how the engine reads an `OP_ACK` that carries no `results`
   * (TG-SYNC-004). Over a WebSocket the acknowledgement of a real batch always
   * carries the level it reached, so a frame without one is not applied even
   * when its `lastId` matches a sent batch; over HTTP the acknowledgement never
   * carries a level, so the match alone decides.
   *
   * Optional. A provider that omits it gets the match-a-sent-batch rule alone:
   * it loses only the extra WebSocket protection, never the retirement of its
   * operations.
   *
   * The engine reads it after `connect()`, on each acknowledgement, and never
   * keeps a copy. A provider that chooses its transport while connecting may
   * therefore return `undefined` before that.
   */
  transport?: 'websocket' | 'http';

  /**
   * Force-close the current connection to trigger reconnection.
   * Unlike close(), this preserves reconnect behavior so the provider
   * will automatically attempt to re-establish the connection.
   */
  forceReconnect(): void;

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

  /**
   * Maximum reconnection attempts before giving up (default: Infinity — retry
   * indefinitely with capped backoff). Set a finite number for a bounded policy;
   * on exhaustion the provider emits a terminal ReconnectExhaustedError 'error'
   * event (SyncEngine maps it to SyncState.ERROR).
   */
  maxReconnectAttempts?: number;

  /** Initial reconnect delay in ms (default: 1000) */
  reconnectDelayMs?: number;

  /** Backoff multiplier for reconnect delay (default: 2) */
  backoffMultiplier?: number;

  /** Maximum reconnect delay in ms (default: 30000) */
  maxReconnectDelayMs?: number;

  /** Listen for browser online/offline events to trigger instant reconnect (default: true) */
  listenNetworkEvents?: boolean;
}
