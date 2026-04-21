/**
 * Sync Module Types
 *
 * Types and interfaces for the sync module that handles
 * WebSocket/connection operations and query management for SyncEngine.
 */

import type { IConnectionProvider, ConnectionProviderEvent, ConnectionEventHandler } from '../types';
import type { SyncStateMachine } from '../SyncStateMachine';
import type { BackoffConfig, HeartbeatConfig, OpLogEntry, TopicQueueConfig } from '../SyncEngine';
import type {
  BackpressureConfig,
  BackpressureStatus,
  BackpressureThresholdEvent,
  OperationDroppedEvent,
} from '../BackpressureConfig';
import type { QueryHandle, QueryFilter } from '../QueryHandle';
import type { HybridQueryHandle, HybridQueryFilter } from '../HybridQueryHandle';
import type { IStorageAdapter } from '../IStorageAdapter';
import type { TopicHandle } from '../TopicHandle';
import type { EntryProcessorDef, EntryProcessorResult } from '@topgunbuild/core';

/**
 * Interface for WebSocket connection management.
 * Handles WebSocket lifecycle, message serialization/deserialization,
 * heartbeat mechanism, and reconnection logic.
 */
export interface IWebSocketManager {
  /**
   * Initialize the WebSocket connection.
   * Sets up event handlers and starts the connection process.
   */
  connect(): void;

  /**
   * Send a message through the current connection.
   * Serializes the message before sending.
   *
   * @param message - Message object to serialize and send
   * @param key - Optional key for routing (cluster mode only)
   * @returns true if message was sent successfully, false otherwise
   */
  sendMessage(message: any, key?: string): boolean;

  /**
   * Check if the connection is ready to send messages.
   * Connection must be open and ready.
   */
  canSend(): boolean;

  /**
   * Check if connected to the server (may not be authenticated yet).
   */
  isOnline(): boolean;

  /**
   * Get the connection provider (for external access).
   */
  getConnectionProvider(): IConnectionProvider;

  /**
   * Close the connection and clean up resources.
   * Stops heartbeat, clears timers.
   */
  close(): void;

  /**
   * Reset connection state for a fresh reconnection.
   * Clears backoff, stops heartbeat, prepares for new connection.
   */
  reset(): void;

  /**
   * Subscribe to connection events.
   * @param event - Event type to subscribe to
   * @param handler - Event handler callback
   */
  on(event: ConnectionProviderEvent, handler: ConnectionEventHandler): void;

  /**
   * Unsubscribe from connection events.
   * @param event - Event type to unsubscribe from
   * @param handler - Event handler to remove
   */
  off(event: ConnectionProviderEvent, handler: ConnectionEventHandler): void;

  /**
   * Start the heartbeat mechanism.
   * Called after successful authentication.
   */
  startHeartbeat(): void;

  /**
   * Stop the heartbeat mechanism.
   * Called on disconnect or close.
   */
  stopHeartbeat(): void;

  /**
   * Returns the last measured round-trip time in milliseconds.
   * Returns null if no PONG has been received yet.
   */
  getLastRoundTripTime(): number | null;

  /**
   * Returns true if the connection is considered healthy based on heartbeat.
   */
  isConnectionHealthy(): boolean;

  /**
   * Reset backoff counter (called on successful connection/auth).
   */
  resetBackoff(): void;
}

/**
 * Configuration for WebSocketManager.
 */
export interface WebSocketManagerConfig {
  /**
   * Connection provider for WebSocket connections.
   * Supports both single-server and cluster modes.
   */
  connectionProvider: IConnectionProvider;

  /**
   * State machine reference for managing connection states.
   */
  stateMachine: SyncStateMachine;

  /**
   * Configuration for exponential backoff reconnection.
   */
  backoffConfig: BackoffConfig;

  /**
   * Configuration for heartbeat mechanism.
   */
  heartbeatConfig: HeartbeatConfig;

  /**
   * Callback invoked when a message is received from the server.
   * Message is already deserialized.
   */
  onMessage: (message: any) => void;

  /**
   * Callback invoked when connection is established.
   * Called for both initial connection and reconnection.
   */
  onConnected?: () => void;

  /**
   * Callback invoked when connection is lost.
   */
  onDisconnected?: () => void;

  /**
   * Callback invoked when reconnection succeeds.
   */
  onReconnected?: () => void;
}

// ============================================
// BackpressureController Types
// ============================================

/**
 * Interface for backpressure control.
 * Handles flow control for pending operations, including pause/resume/throw/drop
 * strategies and high/low water mark events.
 */
export interface IBackpressureController {
  /**
   * Get current pending ops count.
   */
  getPendingOpsCount(): number;

  /**
   * Get backpressure status.
   */
  getBackpressureStatus(): BackpressureStatus;

  /**
   * Check if writes are paused.
   */
  isBackpressurePaused(): boolean;

  /**
   * Check backpressure before adding operation (may pause/throw/drop).
   */
  checkBackpressure(): Promise<void>;

  /**
   * Check high water mark after adding operation.
   */
  checkHighWaterMark(): void;

  /**
   * Check low water mark after ACKs.
   */
  checkLowWaterMark(): void;

  /**
   * Subscribe to backpressure events.
   * @param event Event name: 'backpressure:high', 'backpressure:low', 'backpressure:paused', 'backpressure:resumed', 'operation:dropped'
   * @param listener Callback function
   * @returns Unsubscribe function
   */
  onBackpressure(
    event: 'backpressure:high' | 'backpressure:low' | 'backpressure:paused' | 'backpressure:resumed' | 'operation:dropped',
    listener: (data?: BackpressureThresholdEvent | OperationDroppedEvent) => void
  ): () => void;
}

/**
 * Configuration for BackpressureController.
 */
export interface BackpressureControllerConfig {
  /**
   * Backpressure configuration.
   */
  config: BackpressureConfig;

  /**
   * Reference to opLog array (shared state from SyncEngine).
   */
  opLog: OpLogEntry[];
}

// ============================================
// Query Manager Types
// ============================================

/**
 * Interface for query management.
 * Handles query subscriptions, unsubscriptions, and local query execution.
 */
export interface IQueryManager {
  /**
   * Get all queries (read-only access).
   */
  getQueries(): Map<string, QueryHandle<any>>;

  /**
   * Get all hybrid queries.
   */
  getHybridQueries(): Map<string, HybridQueryHandle<any>>;

  /**
   * Subscribe to a standard query.
   * @param query - Query handle to subscribe
   */
  subscribeToQuery(query: QueryHandle<any>): void;

  /**
   * Unsubscribe from a query.
   * @param queryId - ID of the query to unsubscribe
   */
  unsubscribeFromQuery(queryId: string): void;

  /**
   * Subscribe to a hybrid query (FTS + filter).
   * @param query - Hybrid query handle to subscribe
   */
  subscribeToHybridQuery<T>(query: HybridQueryHandle<T>): void;

  /**
   * Unsubscribe from a hybrid query.
   * @param queryId - ID of the hybrid query to unsubscribe
   */
  unsubscribeFromHybridQuery(queryId: string): void;

  /**
   * Get a hybrid query by ID.
   * @param queryId - ID of the hybrid query
   */
  getHybridQuery(queryId: string): HybridQueryHandle<any> | undefined;

  /**
   * Run a local query against storage.
   * @param mapName - Name of the map to query
   * @param filter - Query filter
   */
  runLocalQuery(mapName: string, filter: QueryFilter): Promise<{ key: string; value: any }[]>;

  /**
   * Run a local hybrid query.
   * @param mapName - Name of the map to query
   * @param filter - Hybrid query filter
   */
  runLocalHybridQuery<T>(
    mapName: string,
    filter: HybridQueryFilter
  ): Promise<Array<{ key: string; value: T; score?: number; matchedTerms?: string[] }>>;

  /**
   * Re-subscribe all queries (called after auth).
   */
  resubscribeAll(): void;
}

/**
 * Configuration for QueryManager.
 */
export interface QueryManagerConfig {
  /**
   * Storage adapter for local queries.
   */
  storageAdapter: IStorageAdapter;

  /**
   * Callback to send messages via SyncEngine/WebSocketManager.
   * @param message - Message to send
   * @param key - Optional key for routing
   * @returns true if sent successfully
   */
  sendMessage: (message: any, key?: string) => boolean;

  /**
   * Callback to check if authenticated.
   */
  isAuthenticated: () => boolean;
}

// ============================================
// TopicManager Types
// ============================================

/**
 * Interface for topic (pub/sub) management.
 * Handles topic subscriptions, publications, and offline message queueing.
 */
export interface ITopicManager {
  /**
   * Subscribe to a topic.
   * @param topic - Topic name
   * @param handle - Topic handle for message delivery
   */
  subscribeToTopic(topic: string, handle: TopicHandle): void;

  /**
   * Unsubscribe from a topic.
   * @param topic - Topic name
   */
  unsubscribeFromTopic(topic: string): void;

  /**
   * Publish a message to a topic.
   * @param topic - Topic name
   * @param data - Message data
   */
  publishTopic(topic: string, data: any): void;

  /**
   * Flush queued topic messages (called after authentication).
   */
  flushTopicQueue(): void;

  /**
   * Get topic queue status.
   * @returns Queue size and max size
   */
  getTopicQueueStatus(): { size: number; maxSize: number };

  /**
   * Get all subscribed topics (for resubscription).
   * @returns Iterator of topic names
   */
  getTopics(): IterableIterator<string>;

  /**
   * Re-subscribe all topics after authentication.
   * Called by SyncEngine after AUTH_ACK.
   */
  resubscribeAll(): void;

  /**
   * Handle incoming topic message from server.
   * @param topic - Topic name
   * @param data - Message data
   * @param publisherId - Publisher node ID
   * @param timestamp - Message timestamp
   */
  handleTopicMessage(topic: string, data: any, publisherId: string, timestamp: number): void;
}

/**
 * Configuration for TopicManager.
 */
export interface TopicManagerConfig {
  /**
   * Topic queue configuration for offline messages.
   */
  topicQueueConfig: TopicQueueConfig;

  /**
   * Callback to send messages via SyncEngine/WebSocketManager.
   * @param message - Message to send
   * @param key - Optional key for routing
   * @returns true if sent successfully
   */
  sendMessage: (message: any, key?: string) => boolean;

  /**
   * Callback to check if authenticated.
   */
  isAuthenticated: () => boolean;
}

// ============================================
// LockManager Types
// ============================================

/**
 * Interface for distributed lock management.
 * Handles lock acquisition, release, and timeout tracking.
 */
export interface ILockManager {
  /**
   * Request a distributed lock.
   *
   * Response timeout is derived from `ttl` as `max(ttl + 5000ms, 5000ms)`.
   * This ensures the client waits long enough for the server to respond within
   * the TTL window without leaking pending requests indefinitely.
   *
   * @param name - Lock name
   * @param requestId - Unique request ID
   * @param ttl - Lock lease duration in milliseconds (server-side)
   * @returns Promise that resolves with fencing token on grant
   */
  requestLock(name: string, requestId: string, ttl: number): Promise<{ fencingToken: number }>;

  /**
   * Release a distributed lock.
   *
   * Returns `true` only on server `success: true` ACK. `false` may mean ACK timeout,
   * offline, send failure (`send_failed`), send exception (`send_threw`), or server
   * `success: false` (`server_rejected`) — check debug logs for disambiguation.
   *
   * @param name - Lock name
   * @param requestId - Unique request ID
   * @param fencingToken - Fencing token from lock grant
   * @returns Promise that resolves true only on server success: true ACK
   */
  releaseLock(name: string, requestId: string, fencingToken: number): Promise<boolean>;

  /**
   * Handle lock granted message from server.
   * @param requestId - Request ID
   * @param fencingToken - Fencing token
   */
  handleLockGranted(requestId: string, name: string, fencingToken: number): void;

  /**
   * Handle lock released message from server.
   * @param requestId - Request ID
   * @param name - Lock name
   * @param success - Release success status
   */
  handleLockReleased(requestId: string, name: string, success: boolean): void;
}

/**
 * Configuration for LockManager.
 */
export interface LockManagerConfig {
  /**
   * Callback to send messages via SyncEngine/WebSocketManager.
   * @param message - Message to send
   * @param key - Optional key for routing
   * @returns true if sent successfully
   */
  sendMessage: (message: any, key?: string) => boolean;

  /**
   * Callback to check if authenticated.
   */
  isAuthenticated: () => boolean;

  /**
   * Callback to check if online.
   */
  isOnline: () => boolean;
}

// ============================================
// WriteConcernManager Types
// ============================================

/**
 * Interface for write concern management.
 * Tracks pending write concern promises and resolves them on server ACK.
 */
export interface IWriteConcernManager {
  /**
   * Register a pending write concern promise for an operation.
   * @param opId - Operation ID
   * @param timeout - Timeout in milliseconds
   * @returns Promise that resolves with the write concern result
   */
  registerWriteConcernPromise(opId: string, timeout?: number): Promise<any>;

  /**
   * Resolve a pending write concern promise with the server result.
   * @param opId - Operation ID
   * @param result - Result from server ACK
   */
  resolveWriteConcernPromise(opId: string, result: any): void;

  /**
   * Cancel all pending write concern promises (e.g., on disconnect).
   * @param error - Error to reject promises with
   */
  cancelAllWriteConcernPromises(error: Error): void;
}

/**
 * Configuration for WriteConcernManager.
 */
export interface WriteConcernManagerConfig {
  /**
   * Default timeout for write concern promises in milliseconds.
   * Default: 5000 (5 seconds)
   */
  defaultTimeout?: number;
}

// ============================================
// CounterManager Types
// ============================================

/**
 * Interface for PN counter management.
 * Handles counter subscriptions, requests, and sync operations.
 */
export interface ICounterManager {
  /**
   * Subscribe to counter updates from server.
   * @param name - Counter name
   * @param listener - Callback when counter state is updated
   * @returns Unsubscribe function
   */
  onCounterUpdate(name: string, listener: (state: any) => void): () => void;

  /**
   * Request initial counter state from server.
   * @param name - Counter name
   */
  requestCounter(name: string): void;

  /**
   * Sync local counter state to server.
   * @param name - Counter name
   * @param state - Counter state to sync
   */
  syncCounter(name: string, state: any): void;

  /**
   * Handle incoming counter update from server.
   * @param name - Counter name
   * @param stateObj - Counter state object
   */
  handleCounterUpdate(name: string, stateObj: { positive: Record<string, number>; negative: Record<string, number> }): void;

  /**
   * Clean up resources (clear listeners).
   */
  close(): void;
}

/**
 * Configuration for CounterManager.
 */
export interface CounterManagerConfig {
  /**
   * Callback to send messages via SyncEngine/WebSocketManager.
   * @param message - Message to send
   * @param key - Optional key for routing
   * @returns true if sent successfully
   */
  sendMessage: (message: any, key?: string) => boolean;

  /**
   * Callback to check if authenticated.
   */
  isAuthenticated: () => boolean;
}

// ============================================
// EntryProcessorClient Types
// ============================================

/**
 * Interface for entry processor operations.
 * Handles executing entry processors on single keys and batches.
 */
export interface IEntryProcessorClient {
  /**
   * Execute an entry processor on a single key atomically.
   * @param mapName - Name of the map
   * @param key - Key to process
   * @param processor - Processor definition
   * @returns Promise resolving to the processor result
   */
  executeOnKey<V, R = V>(
    mapName: string,
    key: string,
    processor: EntryProcessorDef<V, R>
  ): Promise<EntryProcessorResult<R>>;

  /**
   * Execute an entry processor on multiple keys.
   * @param mapName - Name of the map
   * @param keys - Keys to process
   * @param processor - Processor definition
   * @returns Promise resolving to a map of key -> result
   */
  executeOnKeys<V, R = V>(
    mapName: string,
    keys: string[],
    processor: EntryProcessorDef<V, R>
  ): Promise<Map<string, EntryProcessorResult<R>>>;

  /**
   * Handle entry processor response from server.
   * @param message - Response message
   */
  handleEntryProcessResponse(message: {
    requestId: string;
    success: boolean;
    result?: unknown;
    newValue?: unknown;
    error?: string;
  }): void;

  /**
   * Handle entry processor batch response from server.
   * @param message - Batch response message
   */
  handleEntryProcessBatchResponse(message: {
    requestId: string;
    results: Record<string, { success: boolean; result?: unknown; newValue?: unknown; error?: string }>;
  }): void;

  /**
   * Clean up resources (cancel pending requests).
   * @param error - Error to reject pending promises with
   */
  close(error?: Error): void;
}

/**
 * Configuration for EntryProcessorClient.
 */
export interface EntryProcessorClientConfig {
  /**
   * Callback to send messages via SyncEngine/WebSocketManager.
   * @param message - Message to send
   * @param key - Optional key for routing
   * @returns true if sent successfully
   */
  sendMessage: (message: any, key?: string) => boolean;

  /**
   * Callback to check if authenticated.
   */
  isAuthenticated: () => boolean;

  /**
   * Timeout for entry processor requests in milliseconds.
   * Default: 30000 (30 seconds)
   */
  timeoutMs?: number;
}

// ============================================
// SearchClient Types
// ============================================

/**
 * Search result item from server.
 */
export interface SearchResult<T> {
  key: string;
  value: T;
  score: number;
  matchedTerms: string[];
}

/**
 * Interface for full-text search operations.
 * Handles one-shot BM25 search requests.
 */
export interface ISearchClient {
  /**
   * Perform a one-shot BM25 search on the server.
   * @param mapName - Name of the map to search
   * @param query - Search query text
   * @param options - Search options (limit, minScore, boost)
   * @returns Promise resolving to search results
   */
  search<T>(
    mapName: string,
    query: string,
    options?: { limit?: number; minScore?: number; boost?: Record<string, number> }
  ): Promise<SearchResult<T>[]>;

  /**
   * Handle search response from server.
   * @param payload - Response payload
   */
  handleSearchResponse(payload: {
    requestId: string;
    results: SearchResult<unknown>[];
    totalCount: number;
    error?: string;
  }): void;

  /**
   * Clean up resources (cancel pending requests).
   * @param error - Error to reject pending promises with
   */
  close(error?: Error): void;
}

// ============================================
// SqlClient Types
// ============================================

/**
 * Result of a server-side SQL query execution.
 */
export interface SqlQueryResult {
  columns: string[];
  rows: unknown[][];
}

/**
 * Interface for server-side SQL query execution via DataFusion.
 * Handles one-shot request/response queries over the existing WebSocket connection.
 */
export interface ISqlClient {
  /**
   * Execute a SQL query on the server.
   * @param query SQL query string
   * @returns Promise resolving to { columns, rows }
   */
  sql(query: string): Promise<SqlQueryResult>;

  /**
   * Handle SQL query response from server.
   * Called by SyncEngine for SQL_QUERY_RESP messages.
   * @param payload Response payload
   */
  handleSqlQueryResponse(payload: {
    queryId: string;
    columns: string[];
    rows: unknown[][];
    error?: string;
  }): void;

  /**
   * Clean up resources (cancel pending requests).
   * @param error - Error to reject pending promises with
   */
  close(error?: Error): void;
}

/**
 * Configuration for SqlClient.
 */
export interface SqlClientConfig {
  /**
   * Callback to send messages via SyncEngine/WebSocketManager.
   * @param message - Message to send
   * @returns true if sent successfully
   */
  sendMessage: (message: any) => boolean;

  /**
   * Callback to check if authenticated.
   */
  isAuthenticated: () => boolean;

  /**
   * Timeout for SQL query requests in milliseconds.
   * Default: 30000 (30 seconds)
   */
  timeoutMs?: number;
}

/**
 * Configuration for SearchClient.
 */
export interface SearchClientConfig {
  /**
   * Callback to send messages via SyncEngine/WebSocketManager.
   * @param message - Message to send
   * @param key - Optional key for routing
   * @returns true if sent successfully
   */
  sendMessage: (message: any, key?: string) => boolean;

  /**
   * Callback to check if authenticated.
   */
  isAuthenticated: () => boolean;

  /**
   * Timeout for search requests in milliseconds.
   * Default: 30000 (30 seconds)
   */
  timeoutMs?: number;
}

// ============================================
// MerkleSyncHandler Types
// ============================================

/**
 * Interface for Merkle tree synchronization handler (LWWMap).
 * Handles sync protocol messages for LWWMap synchronization.
 */
export interface IMerkleSyncHandler {
  /**
   * Handle SYNC_RESP_ROOT message from server.
   * @param payload - Root hash and timestamp from server
   */
  handleSyncRespRoot(payload: { mapName: string; rootHash: number; timestamp?: any }): Promise<void>;

  /**
   * Handle SYNC_RESP_BUCKETS message from server.
   * @param payload - Bucket hashes for a path
   */
  handleSyncRespBuckets(payload: { mapName: string; path: string; buckets: Record<string, number> }): void;

  /**
   * Handle SYNC_RESP_LEAF message from server.
   * @param payload - Leaf records to merge
   */
  handleSyncRespLeaf(payload: { mapName: string; records: Array<{ key: string; record: any }> }): Promise<void>;

  /**
   * Handle SYNC_RESET_REQUIRED message from server.
   * @param payload - Map name that requires reset
   */
  handleSyncResetRequired(payload: { mapName: string }): Promise<void>;

  /**
   * Send SYNC_INIT message to server.
   * @param mapName - Map name to sync
   * @param lastSyncTimestamp - Last sync timestamp
   */
  sendSyncInit(mapName: string, lastSyncTimestamp: number): void;

  /**
   * Get the last sync timestamp for debugging/testing.
   * @returns Last sync timestamp
   */
  getLastSyncTimestamp(): number;
}

/**
 * Configuration for MerkleSyncHandler.
 */
export interface MerkleSyncHandlerConfig {
  /**
   * Callback to get a map by name.
   * @param mapName - Map name
   * @returns LWWMap or ORMap instance, or undefined if not found
   */
  getMap: (mapName: string) => any; // LWWMap<any, any> | ORMap<any, any> | undefined

  /**
   * Callback to send messages via SyncEngine/WebSocketManager.
   * @param message - Message to send
   * @param key - Optional key for routing
   * @returns true if sent successfully
   */
  sendMessage: (message: any, key?: string) => boolean;

  /**
   * Storage adapter for persistence.
   */
  storageAdapter: IStorageAdapter;

  /**
   * HLC instance for timestamp updates.
   */
  hlc: any; // HLC from @topgunbuild/core

  /**
   * Callback when timestamp is updated.
   * @param timestamp - New timestamp to update
   */
  onTimestampUpdate: (timestamp: any) => Promise<void>;

  /**
   * Callback to reset a map (clear memory and storage).
   * @param mapName - Map name to reset
   */
  resetMap: (mapName: string) => Promise<void>;
}

// ============================================
// ORMapSyncHandler Types
// ============================================

/**
 * Interface for ORMap Merkle tree synchronization handler.
 * Handles sync protocol messages for ORMap synchronization.
 */
export interface IORMapSyncHandler {
  /**
   * Handle ORMAP_SYNC_RESP_ROOT message from server.
   * @param payload - Root hash and timestamp from server
   */
  handleORMapSyncRespRoot(payload: { mapName: string; rootHash: number; timestamp?: any }): Promise<void>;

  /**
   * Handle ORMAP_SYNC_RESP_BUCKETS message from server.
   * @param payload - Bucket hashes for a path
   */
  handleORMapSyncRespBuckets(payload: { mapName: string; path: string; buckets: Record<string, number> }): Promise<void>;

  /**
   * Handle ORMAP_SYNC_RESP_LEAF message from server.
   * @param payload - Leaf entries to merge
   */
  handleORMapSyncRespLeaf(payload: { mapName: string; entries: Array<{ key: string; records: any[]; tombstones: string[] }> }): Promise<void>;

  /**
   * Handle ORMAP_DIFF_RESPONSE message from server.
   * @param payload - Diff entries to merge
   */
  handleORMapDiffResponse(payload: { mapName: string; entries: Array<{ key: string; records: any[]; tombstones: string[] }> }): Promise<void>;

  /**
   * Push local ORMap diff to server for given keys.
   * @param mapName - Map name
   * @param keys - Keys to push
   * @param map - ORMap instance
   */
  pushORMapDiff(mapName: string, keys: string[], map: any): Promise<void>;

  /**
   * Send ORMAP_SYNC_INIT message to server.
   * @param mapName - Map name to sync
   * @param lastSyncTimestamp - Last sync timestamp
   */
  sendSyncInit(mapName: string, lastSyncTimestamp: number): void;

  /**
   * Get the last sync timestamp for debugging/testing.
   * @returns Last sync timestamp
   */
  getLastSyncTimestamp(): number;
}

/**
 * Configuration for ORMapSyncHandler.
 */
export interface ORMapSyncHandlerConfig {
  /**
   * Callback to get a map by name.
   * @param mapName - Map name
   * @returns LWWMap or ORMap instance, or undefined if not found
   */
  getMap: (mapName: string) => any; // LWWMap<any, any> | ORMap<any, any> | undefined

  /**
   * Callback to send messages via SyncEngine/WebSocketManager.
   * @param message - Message to send
   * @param key - Optional key for routing
   * @returns true if sent successfully
   */
  sendMessage: (message: any, key?: string) => boolean;

  /**
   * HLC instance for timestamp updates.
   */
  hlc: any; // HLC from @topgunbuild/core

  /**
   * Callback when timestamp is updated.
   * @param timestamp - New timestamp to update
   */
  onTimestampUpdate: (timestamp: any) => Promise<void>;
}

// ============================================
// VectorSearchClient Types
// ============================================

/**
 * Configuration for VectorSearchClient.
 */
export interface VectorSearchClientConfig {
  /**
   * Callback to send messages via SyncEngine/WebSocketManager.
   * @param message - Message to send
   * @returns true if sent successfully
   */
  sendMessage: (message: any) => boolean;

  /**
   * Callback to check if authenticated.
   */
  isAuthenticated: () => boolean;

  /**
   * Timeout for vector search requests in milliseconds.
   * Default: 30000 (30 seconds)
   */
  timeoutMs?: number;
}

/**
 * Options for a vector search request.
 */
export interface VectorSearchClientOptions {
  /** Number of nearest neighbours to return. Default: 10 */
  k?: number;
  /** Name of the HNSW index to query (if multiple indexes exist on the map). */
  indexName?: string;
  /** HNSW efSearch parameter — controls recall vs. speed trade-off. */
  efSearch?: number;
  /** Include the stored value in each result. Default: false */
  includeValue?: boolean;
  /** Include the stored vector in each result. Default: false */
  includeVectors?: boolean;
  /** Minimum similarity score threshold (0–1). Results below this are filtered. */
  minScore?: number;
}

/**
 * A single result from a vector search, as seen by the developer.
 * The vector field is Float32Array (developer-facing), not the wire-format Uint8Array.
 */
export interface VectorSearchClientResult {
  key: string;
  score: number;
  value?: unknown;
  /** Decoded Float32Array, present only when includeVectors was true. */
  vector?: Float32Array;
}

/**
 * Interface for approximate nearest-neighbour vector search operations.
 * Handles one-shot VECTOR_SEARCH request/response over the WebSocket connection.
 */
export interface IVectorSearchClient {
  /**
   * Perform an ANN vector search on the server.
   * @param mapName - Name of the map / index to search
   * @param queryVector - Query vector as Float32Array or number[]
   * @param options - Search options (k, efSearch, filters, etc.)
   * @returns Promise resolving to ranked VectorSearchClientResult[]
   */
  vectorSearch(
    mapName: string,
    queryVector: Float32Array | number[],
    options?: VectorSearchClientOptions
  ): Promise<VectorSearchClientResult[]>;

  /**
   * Handle VECTOR_SEARCH_RESP message from server.
   * Called by the message router.
   * @param payload - Response payload (matches VectorSearchRespPayloadSchema)
   */
  handleResponse(payload: {
    id: string;
    results: Array<{ key: string; score: number; value?: unknown; vector?: Uint8Array }>;
    totalCandidates: number;
    searchTimeMs: number;
    error?: string;
  }): void;

  /**
   * Clean up resources (clear pending timeouts).
   * Does NOT reject pending promises — use when shutting down cleanly.
   * @param error - If provided, reject pending promises with this error.
   */
  close(error?: Error): void;
}

// ============================================
// MessageRouter Types
// ============================================

/**
 * Handler function for a message type.
 * Can be sync or async.
 */
export type MessageHandler = (message: any) => Promise<void> | void;

/**
 * Configuration for MessageRouter.
 */
export interface MessageRouterConfig {
  /**
   * Handlers registered during construction.
   */
  handlers?: Map<string, MessageHandler>;

  /**
   * Fallback for unregistered message types.
   */
  onUnhandled?: (message: any) => void;
}

/**
 * Interface for message routing.
 * Routes incoming messages to appropriate handlers based on message type.
 */
export interface IMessageRouter {
  /**
   * Register a handler for a message type.
   * @param type - Message type to handle
   * @param handler - Handler function
   */
  registerHandler(type: string, handler: MessageHandler): void;

  /**
   * Register multiple handlers at once.
   * @param handlers - Record of type -> handler
   */
  registerHandlers(handlers: Record<string, MessageHandler>): void;

  /**
   * Route a message to its registered handler.
   * Returns true if handled, false if no handler found.
   * @param message - Message to route
   * @returns Promise resolving to true if handled
   */
  route(message: any): Promise<boolean>;

  /**
   * Check if a handler is registered for a message type.
   * @param type - Message type to check
   * @returns true if handler exists
   */
  hasHandler(type: string): boolean;
}

// ============================================
// HybridSearchClient Types
// ============================================

/**
 * The three search methods that can be combined via RRF fusion.
 */
export type HybridSearchMethod = 'exact' | 'fullText' | 'semantic';

/**
 * Configuration for HybridSearchClient.
 */
export interface HybridSearchClientConfig {
  /**
   * Callback to send messages via SyncEngine/WebSocketManager.
   * @param message - Message to send
   * @returns true if sent successfully
   */
  sendMessage: (message: any) => boolean;

  /**
   * Callback to check if authenticated.
   */
  isAuthenticated: () => boolean;

  /**
   * Timeout for hybrid search requests in milliseconds.
   * Default: 30000 (30 seconds)
   */
  timeoutMs?: number;
}

/**
 * Options for a hybrid search request.
 */
export interface HybridSearchClientOptions {
  /** Which methods to combine via RRF. Defaults to ['fullText'] if omitted. */
  methods?: HybridSearchMethod[];
  /** Number of fused results to return. Default: 10 */
  k?: number;
  /** Pre-computed embedding for the semantic leg. Required when 'semantic' is in methods and the server does not auto-embed. */
  queryVector?: Float32Array | number[];
  /** Optional predicate tree to restrict the candidate set. */
  predicate?: unknown;
  /** Include the full value in each result entry. Default: false */
  includeValue?: boolean;
  /** Minimum fused score to include in results. */
  minScore?: number;
}

/**
 * A single result from a hybrid search, containing the RRF-fused score and per-method raw scores.
 */
export interface HybridSearchClientResult {
  key: string;
  /** Final RRF-fused score */
  score: number;
  /** Per-method raw scores (only present for methods that matched this key) */
  methodScores: Partial<Record<HybridSearchMethod, number>>;
  /** Document value when includeValue=true */
  value?: unknown;
}

/**
 * Interface for tri-hybrid search (exact + fullText + semantic) with RRF fusion.
 * Handles one-shot HYBRID_SEARCH request/response over the WebSocket connection.
 */
export interface IHybridSearchClient {
  /**
   * Perform a hybrid search on the server combining exact, fullText, and/or semantic methods via RRF.
   * @param mapName - Name of the map to search
   * @param queryText - Search query text
   * @param options - Search options (methods, k, queryVector, predicate, etc.)
   * @returns Promise resolving to ranked HybridSearchClientResult[]
   */
  hybridSearch(
    mapName: string,
    queryText: string,
    options?: HybridSearchClientOptions
  ): Promise<HybridSearchClientResult[]>;

  /**
   * Handle HYBRID_SEARCH_RESP message from server.
   * Called by the message router.
   * @param payload - Response payload (matches HybridSearchRespPayloadSchema)
   */
  handleResponse(payload: {
    requestId: string;
    results: Array<{
      key: string;
      score: number;
      methodScores: Partial<Record<HybridSearchMethod, number>>;
      value?: unknown;
    }>;
    searchTimeMs: number;
    error?: string;
  }): void;

  /**
   * Clean up resources (clear pending timeouts).
   * Does NOT reject pending promises — use when shutting down cleanly.
   * @param error - If provided, reject pending promises with this error.
   */
  close(error?: Error): void;
}
