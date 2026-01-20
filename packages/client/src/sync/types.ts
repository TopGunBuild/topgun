/**
 * WebSocket Manager Types
 *
 * Types and interfaces for the WebSocketManager module that handles
 * all WebSocket/connection operations for SyncEngine.
 */

import type { IConnectionProvider, ConnectionProviderEvent, ConnectionEventHandler } from '../types';
import type { SyncStateMachine } from '../SyncStateMachine';
import type { BackoffConfig, HeartbeatConfig } from '../SyncEngine';

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
