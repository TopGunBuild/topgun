/**
 * Sync module - Handles WebSocket/connection and backpressure operations for SyncEngine
 *
 * This module extracts connection management and backpressure from SyncEngine:
 * - WebSocketManager: Manages WebSocket lifecycle, heartbeat, reconnection
 * - BackpressureController: Manages flow control for pending operations
 */

// Types
export type {
  IWebSocketManager,
  WebSocketManagerConfig,
  IBackpressureController,
  BackpressureControllerConfig,
} from './types';

// Implementation
export { WebSocketManager } from './WebSocketManager';
export { BackpressureController } from './BackpressureController';
