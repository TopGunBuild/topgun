/**
 * Sync module - Handles WebSocket/connection, backpressure, and query operations for SyncEngine
 *
 * This module extracts connection management, backpressure, and query management from SyncEngine:
 * - WebSocketManager: Manages WebSocket lifecycle, heartbeat, reconnection
 * - BackpressureController: Manages flow control for pending operations
 * - QueryManager: Manages query subscriptions and local query execution
 */

// Types
export type {
  IWebSocketManager,
  WebSocketManagerConfig,
  IBackpressureController,
  BackpressureControllerConfig,
  IQueryManager,
  QueryManagerConfig,
} from './types';

// Implementation
export { WebSocketManager } from './WebSocketManager';
export { BackpressureController } from './BackpressureController';
export { QueryManager } from './QueryManager';
