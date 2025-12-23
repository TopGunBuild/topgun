/**
 * Cluster types for Phase 4: Clustering Improvements
 *
 * These types are shared between client and server packages
 * for partition-aware routing and cluster communication.
 */

// ============================================
// Node and Cluster Status
// ============================================

export type NodeStatus = 'ACTIVE' | 'JOINING' | 'LEAVING' | 'SUSPECTED' | 'FAILED';

export interface NodeInfo {
  nodeId: string;
  endpoints: {
    websocket: string; // ws://host:port or wss://host:port
    http?: string; // Optional REST endpoint
  };
  status: NodeStatus;
}

// ============================================
// Partition Map
// ============================================

export interface PartitionInfo {
  partitionId: number;
  ownerNodeId: string;
  backupNodeIds: string[];
}

export interface PartitionMap {
  version: number; // Incremented on topology change
  partitionCount: number; // 271 by default
  nodes: NodeInfo[];
  partitions: PartitionInfo[];
  generatedAt: number; // Timestamp when map was generated
}

// ============================================
// Partition Map Protocol Messages
// ============================================

export interface PartitionMapMessage {
  type: 'PARTITION_MAP';
  payload: PartitionMap;
}

export interface PartitionChange {
  partitionId: number;
  previousOwner: string;
  newOwner: string;
  reason: 'REBALANCE' | 'FAILOVER' | 'JOIN' | 'LEAVE';
}

export interface PartitionMapDeltaMessage {
  type: 'PARTITION_MAP_DELTA';
  payload: {
    version: number;
    previousVersion: number;
    changes: PartitionChange[];
    timestamp: number;
  };
}

// ============================================
// Routing Errors
// ============================================

export interface NotOwnerError {
  code: 'NOT_OWNER';
  message: string;
  hint: {
    partitionId: number;
    currentOwner: string;
    mapVersion: number;
  };
}

export interface StaleMapError {
  code: 'STALE_MAP';
  message: string;
  hint: {
    clientVersion: number;
    serverVersion: number;
  };
}

export type RoutingError = NotOwnerError | StaleMapError;

// ============================================
// Connection Pool Configuration
// ============================================

export interface ConnectionPoolConfig {
  /** Maximum connections per node (default: 1) */
  maxConnectionsPerNode: number;
  /** Connection timeout in ms (default: 5000) */
  connectionTimeoutMs: number;
  /** Health check interval in ms (default: 10000) */
  healthCheckIntervalMs: number;
  /** Reconnect delay base in ms (default: 1000) */
  reconnectDelayMs: number;
  /** Maximum reconnect delay in ms (default: 30000) */
  maxReconnectDelayMs: number;
  /** Maximum reconnect attempts before marking unhealthy (default: 5) */
  maxReconnectAttempts: number;
}

export const DEFAULT_CONNECTION_POOL_CONFIG: ConnectionPoolConfig = {
  maxConnectionsPerNode: 1,
  connectionTimeoutMs: 5000,
  healthCheckIntervalMs: 10000,
  reconnectDelayMs: 1000,
  maxReconnectDelayMs: 30000,
  maxReconnectAttempts: 5,
};

// ============================================
// Partition Router Configuration
// ============================================

export interface PartitionRouterConfig {
  /** Fallback mode when routing fails: 'forward' uses primary, 'error' throws */
  fallbackMode: 'forward' | 'error';
  /** How often to refresh stale partition map in ms (default: 30000) */
  mapRefreshIntervalMs: number;
  /** Max staleness before forcing refresh in ms (default: 60000) */
  maxMapStalenessMs: number;
}

export const DEFAULT_PARTITION_ROUTER_CONFIG: PartitionRouterConfig = {
  fallbackMode: 'forward',
  mapRefreshIntervalMs: 30000,
  maxMapStalenessMs: 60000,
};

// ============================================
// Cluster Client Configuration
// ============================================

export interface ClusterClientConfig {
  /** Enable cluster mode */
  enabled: boolean;
  /** Initial seed nodes to connect to */
  seedNodes: string[];
  /** Routing mode: 'direct' routes to owner, 'forward' uses server forwarding */
  routingMode: 'direct' | 'forward';
  /** Connection pool configuration */
  connectionPool?: Partial<ConnectionPoolConfig>;
  /** Partition router configuration */
  routing?: Partial<PartitionRouterConfig>;
}

// ============================================
// Node Health
// ============================================

export type ConnectionState =
  | 'DISCONNECTED'
  | 'CONNECTING'
  | 'CONNECTED'
  | 'AUTHENTICATED'
  | 'RECONNECTING'
  | 'FAILED';

export interface NodeHealth {
  nodeId: string;
  state: ConnectionState;
  lastSeen: number;
  latencyMs: number;
  reconnectAttempts: number;
}

// ============================================
// Cluster Events (for EventEmitter)
// ============================================

export interface ClusterEvents {
  'node:connected': { nodeId: string };
  'node:disconnected': { nodeId: string; reason: string };
  'node:healthy': { nodeId: string };
  'node:unhealthy': { nodeId: string; reason: string };
  'partitionMap:updated': { version: number; changesCount: number };
  'partitionMap:stale': { currentVersion: number; lastRefresh: number };
  'routing:miss': { key: string; expectedOwner: string; actualOwner: string };
}

// ============================================
// Constants
// ============================================

export const PARTITION_COUNT = 271;
export const DEFAULT_BACKUP_COUNT = 1;
