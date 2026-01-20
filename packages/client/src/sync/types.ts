/**
 * Sync Module Types
 *
 * Types and interfaces for the sync module that handles
 * WebSocket/connection operations and query management for SyncEngine.
 */

import type { IConnectionProvider, ConnectionProviderEvent, ConnectionEventHandler } from '../types';
import type { SyncStateMachine } from '../SyncStateMachine';
import type { BackoffConfig, HeartbeatConfig, OpLogEntry } from '../SyncEngine';
import type {
  BackpressureConfig,
  BackpressureStatus,
  BackpressureThresholdEvent,
  OperationDroppedEvent,
} from '../BackpressureConfig';
import type { QueryHandle, QueryFilter } from '../QueryHandle';
import type { HybridQueryHandle, HybridQueryFilter } from '../HybridQueryHandle';
import type { IStorageAdapter } from '../IStorageAdapter';

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
