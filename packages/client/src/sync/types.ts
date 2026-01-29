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
   * Direct WebSocket URL for single-server mode.
   * @deprecated Use connectionProvider instead
   */
  serverUrl?: string;

  /**
   * Connection provider (preferred over serverUrl).
   * Supports both single-server and cluster modes.
   */
  connectionProvider?: IConnectionProvider;

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
   * @param name - Lock name
   * @param requestId - Unique request ID
   * @param ttl - Time-to-live in milliseconds
   * @returns Promise that resolves with fencing token
   */
  requestLock(name: string, requestId: string, ttl: number): Promise<{ fencingToken: number }>;

  /**
   * Release a distributed lock.
   * @param name - Lock name
   * @param requestId - Unique request ID
   * @param fencingToken - Fencing token from lock grant
   * @returns Promise that resolves with success status
   */
  releaseLock(name: string, requestId: string, fencingToken: number): Promise<boolean>;

  /**
   * Handle lock granted message from server.
   * @param requestId - Request ID
   * @param fencingToken - Fencing token
   */
  handleLockGranted(requestId: string, fencingToken: number): void;

  /**
   * Handle lock released message from server.
   * @param requestId - Request ID
   * @param success - Release success status
   */
  handleLockReleased(requestId: string, success: boolean): void;
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
