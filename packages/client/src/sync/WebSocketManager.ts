/**
 * WebSocketManager - Handles all WebSocket/connection operations for SyncEngine
 *
 * Responsibilities:
 * - WebSocket lifecycle management (connect, close, reset)
 * - Message serialization/deserialization
 * - Heartbeat mechanism (PING/PONG)
 * - Reconnection with exponential backoff
 * - Event forwarding from connection provider
 */

import { serialize, deserialize } from '@topgunbuild/core';
import type { IConnectionProvider, ConnectionProviderEvent, ConnectionEventHandler } from '../types';
import { SyncState } from '../SyncState';
import { logger } from '../utils/logger';
import type { IWebSocketManager, WebSocketManagerConfig } from './types';

/**
 * WebSocketManager implements IWebSocketManager.
 *
 * Manages WebSocket connections via IConnectionProvider.
 * Supports both single-server and cluster modes through the provider abstraction.
 */
export class WebSocketManager implements IWebSocketManager {
  private readonly config: WebSocketManagerConfig;
  private readonly connectionProvider: IConnectionProvider;

  // Reconnection state
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private backoffAttempt: number = 0;

  // Heartbeat state
  private heartbeatInterval: ReturnType<typeof setInterval> | null = null;
  private lastPongReceived: number = Date.now();
  private lastRoundTripTime: number | null = null;

  constructor(config: WebSocketManagerConfig) {
    this.config = config;
    this.connectionProvider = config.connectionProvider;
  }

  /**
   * Initialize the connection.
   * Sets up event handlers and starts the connection process.
   */
  connect(): void {
    this.initConnectionProvider();
  }

  /**
   * Initialize connection using IConnectionProvider.
   */
  private initConnectionProvider(): void {
    // Transition to CONNECTING state
    this.config.stateMachine.transition(SyncState.CONNECTING);

    // Set up event handlers
    this.connectionProvider.on('connected', (_nodeId: string) => {
      logger.info('ConnectionProvider connected.');
      this.config.onConnected?.();
    });

    this.connectionProvider.on('disconnected', (_nodeId: string) => {
      logger.info('ConnectionProvider disconnected.');
      this.stopHeartbeat();
      this.config.stateMachine.transition(SyncState.DISCONNECTED);
      this.config.onDisconnected?.();
      // Don't schedule reconnect - provider handles it
    });

    this.connectionProvider.on('reconnected', (_nodeId: string) => {
      logger.info('ConnectionProvider reconnected.');
      this.config.stateMachine.transition(SyncState.CONNECTING);
      this.config.onReconnected?.();
    });

    this.connectionProvider.on('message', (_nodeId: string, data: any) => {
      const message = this.deserializeMessage(data);
      if (message) {
        this.handleMessage(message);
      }
    });

    this.connectionProvider.on('partitionMapUpdated', () => {
      logger.debug('Partition map updated');
    });

    this.connectionProvider.on('error', (error: Error) => {
      logger.error({ err: error }, 'ConnectionProvider error');
    });

    // Start connection
    this.connectionProvider.connect().catch((err) => {
      logger.error({ err }, 'Failed to connect via ConnectionProvider');
      this.config.stateMachine.transition(SyncState.DISCONNECTED);
    });
  }

  /**
   * Deserialize incoming message data.
   */
  private deserializeMessage(data: any): any {
    try {
      if (data instanceof ArrayBuffer) {
        return deserialize(new Uint8Array(data));
      } else if (data instanceof Uint8Array) {
        return deserialize(data);
      } else if (typeof data === 'string') {
        return JSON.parse(data);
      } else {
        return data;
      }
    } catch (e) {
      logger.error({ err: e }, 'Failed to parse message');
      return null;
    }
  }

  /**
   * Handle incoming message.
   * Routes PONG to internal handler, all others to SyncEngine.
   */
  private handleMessage(message: any): void {
    // Handle PONG internally for heartbeat tracking
    if (message.type === 'PONG') {
      this.handlePong(message);
    }
    // Route all messages to SyncEngine (including PONG for HLC sync if needed)
    this.config.onMessage(message);
  }

  /**
   * Send a message through the current connection.
   */
  sendMessage(message: any, key?: string): boolean {
    const data = serialize(message);

    try {
      this.connectionProvider.send(data, key);
      return true;
    } catch (err) {
      logger.warn({ err }, 'Failed to send via ConnectionProvider');
      return false;
    }
  }

  /**
   * Check if we can send messages (connection is ready).
   */
  canSend(): boolean {
    return this.connectionProvider.isConnected();
  }

  /**
   * Check if connected to the server.
   */
  isOnline(): boolean {
    const state = this.config.stateMachine.getState();
    return (
      state === SyncState.CONNECTING ||
      state === SyncState.AUTHENTICATING ||
      state === SyncState.SYNCING ||
      state === SyncState.CONNECTED
    );
  }

  /**
   * Get the connection provider.
   */
  getConnectionProvider(): IConnectionProvider {
    return this.connectionProvider;
  }

  /**
   * Close the connection and clean up resources.
   */
  close(): void {
    this.stopHeartbeat();

    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }

    this.connectionProvider.close().catch((err) => {
      logger.error({ err }, 'Error closing ConnectionProvider');
    });
  }

  /**
   * Reset connection state for a fresh reconnection.
   */
  reset(): void {
    this.close();
    this.resetBackoff();
  }

  /**
   * Subscribe to connection events.
   */
  on(event: ConnectionProviderEvent, handler: ConnectionEventHandler): void {
    this.connectionProvider.on(event, handler);
  }

  /**
   * Unsubscribe from connection events.
   */
  off(event: ConnectionProviderEvent, handler: ConnectionEventHandler): void {
    this.connectionProvider.off(event, handler);
  }

  /**
   * Reset backoff counter.
   */
  resetBackoff(): void {
    this.backoffAttempt = 0;
  }

  /**
   * Get current backoff attempt count.
   */
  getBackoffAttempt(): number {
    return this.backoffAttempt;
  }

  /**
   * Clear reconnect timer (for external control, e.g., when new auth token provided).
   */
  clearReconnectTimer(): void {
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
  }

  // ============================================
  // Heartbeat Mechanism
  // ============================================

  /**
   * Starts the heartbeat mechanism after successful connection.
   */
  startHeartbeat(): void {
    if (!this.config.heartbeatConfig.enabled) {
      return;
    }

    this.stopHeartbeat(); // Clear any existing interval
    this.lastPongReceived = Date.now();

    this.heartbeatInterval = setInterval(() => {
      this.sendPing();
      this.checkHeartbeatTimeout();
    }, this.config.heartbeatConfig.intervalMs);

    logger.info({ intervalMs: this.config.heartbeatConfig.intervalMs }, 'Heartbeat started');
  }

  /**
   * Stops the heartbeat mechanism.
   */
  stopHeartbeat(): void {
    if (this.heartbeatInterval) {
      clearInterval(this.heartbeatInterval);
      this.heartbeatInterval = null;
      logger.info('Heartbeat stopped');
    }
  }

  /**
   * Sends a PING message to the server.
   */
  private sendPing(): void {
    if (this.canSend()) {
      const pingMessage = {
        type: 'PING',
        timestamp: Date.now(),
      };
      this.sendMessage(pingMessage);
    }
  }

  /**
   * Handles incoming PONG message from server.
   */
  private handlePong(msg: { timestamp: number; serverTime: number }): void {
    const now = Date.now();
    this.lastPongReceived = now;
    this.lastRoundTripTime = now - msg.timestamp;

    logger.debug({
      rtt: this.lastRoundTripTime,
      serverTime: msg.serverTime,
      clockSkew: msg.serverTime - (msg.timestamp + this.lastRoundTripTime / 2),
    }, 'Received PONG');
  }

  /**
   * Checks if heartbeat has timed out and triggers reconnection if needed.
   */
  private checkHeartbeatTimeout(): void {
    const now = Date.now();
    const timeSinceLastPong = now - this.lastPongReceived;

    if (timeSinceLastPong > this.config.heartbeatConfig.timeoutMs) {
      logger.warn({
        timeSinceLastPong,
        timeoutMs: this.config.heartbeatConfig.timeoutMs,
      }, 'Heartbeat timeout - triggering reconnection');

      this.stopHeartbeat();

      // Force close and reconnect via connection provider
      this.connectionProvider.close().catch((err) => {
        logger.error({ err }, 'Error closing ConnectionProvider on heartbeat timeout');
      });
    }
  }

  /**
   * Returns the last measured round-trip time in milliseconds.
   */
  getLastRoundTripTime(): number | null {
    return this.lastRoundTripTime;
  }

  /**
   * Returns true if the connection is considered healthy based on heartbeat.
   */
  isConnectionHealthy(): boolean {
    const state = this.config.stateMachine.getState();
    const isOnline = (
      state === SyncState.CONNECTING ||
      state === SyncState.AUTHENTICATING ||
      state === SyncState.SYNCING ||
      state === SyncState.CONNECTED
    );
    const isAuthenticated = state === SyncState.SYNCING || state === SyncState.CONNECTED;

    if (!isOnline || !isAuthenticated) {
      return false;
    }

    if (!this.config.heartbeatConfig.enabled) {
      return true; // If heartbeat disabled, consider healthy if online
    }

    const timeSinceLastPong = Date.now() - this.lastPongReceived;
    return timeSinceLastPong < this.config.heartbeatConfig.timeoutMs;
  }
}
