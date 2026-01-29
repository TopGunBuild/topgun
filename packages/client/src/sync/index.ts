/**
 * Sync module - Handles WebSocket/connection, backpressure, and query operations for SyncEngine
 *
 * This module extracts connection management, backpressure, and query management from SyncEngine:
 * - WebSocketManager: Manages WebSocket lifecycle, heartbeat, reconnection
 * - BackpressureController: Manages flow control for pending operations
 * - QueryManager: Manages query subscriptions and local query execution
 * - TopicManager: Manages topic (pub/sub) subscriptions and offline queueing
 * - LockManager: Manages distributed lock acquisition and release
 * - WriteConcernManager: Manages write concern promise tracking
 */

// Types
export type {
  IWebSocketManager,
  WebSocketManagerConfig,
  IBackpressureController,
  BackpressureControllerConfig,
  IQueryManager,
  QueryManagerConfig,
  ITopicManager,
  TopicManagerConfig,
  ILockManager,
  LockManagerConfig,
  IWriteConcernManager,
  WriteConcernManagerConfig,
} from './types';

// Implementation
export { WebSocketManager } from './WebSocketManager';
export { BackpressureController } from './BackpressureController';
export { QueryManager } from './QueryManager';
export { TopicManager } from './TopicManager';
export { LockManager } from './LockManager';
export { WriteConcernManager } from './WriteConcernManager';
