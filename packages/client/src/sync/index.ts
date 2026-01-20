/**
 * Sync module - Handles WebSocket/connection operations for SyncEngine
 *
 * This module extracts connection management from SyncEngine:
 * - WebSocketManager: Manages WebSocket lifecycle, heartbeat, reconnection
 */

// Types
export type { IWebSocketManager, WebSocketManagerConfig } from './types';

// Implementation
export { WebSocketManager } from './WebSocketManager';
