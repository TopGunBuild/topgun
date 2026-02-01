/**
 * Cluster types
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

export interface PartitionMapRequestMessage {
  type: 'PARTITION_MAP_REQUEST';
  payload?: {
    currentVersion?: number;
  };
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

/**
 * Circuit breaker configuration.
 */
export interface CircuitBreakerConfig {
  /** Number of failures before opening circuit (default: 5) */
  failureThreshold: number;
  /** Time in ms before attempting to close circuit (default: 30000) */
  resetTimeoutMs: number;
}

export const DEFAULT_CIRCUIT_BREAKER_CONFIG: CircuitBreakerConfig = {
  failureThreshold: 5,
  resetTimeoutMs: 30000,
};

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
  /** Circuit breaker configuration */
  circuitBreaker?: Partial<CircuitBreakerConfig>;
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
// Migration State Machine (Task 03)
// ============================================

export enum PartitionState {
  STABLE = 'STABLE', // Normal operation
  MIGRATING = 'MIGRATING', // Data being transferred
  SYNC = 'SYNC', // Verifying consistency
  FAILED = 'FAILED', // Migration failed, needs retry
}

export interface PartitionMigration {
  partitionId: number;
  state: PartitionState;
  sourceNode: string;
  targetNode: string;
  startTime: number;
  bytesTransferred: number;
  totalBytes: number;
  retryCount: number;
}

export interface MigrationConfig {
  /** Partitions per batch (default: 10) */
  batchSize: number;
  /** Delay between batches in ms (default: 5000) */
  batchIntervalMs: number;
  /** Bytes per chunk (default: 64KB) */
  transferChunkSize: number;
  /** Retries per partition (default: 3) */
  maxRetries: number;
  /** Sync phase timeout in ms (default: 30000) */
  syncTimeoutMs: number;
  /** Concurrent transfers (default: 4) */
  parallelTransfers: number;
}

export const DEFAULT_MIGRATION_CONFIG: MigrationConfig = {
  batchSize: 10,
  batchIntervalMs: 5000,
  transferChunkSize: 65536, // 64KB
  maxRetries: 3,
  syncTimeoutMs: 30000,
  parallelTransfers: 4,
};

export interface MigrationStatus {
  inProgress: boolean;
  active: PartitionMigration[];
  queued: number;
  completed: number;
  failed: number;
  estimatedTimeRemainingMs: number;
}

export interface MigrationMetrics {
  migrationsStarted: number;
  migrationsCompleted: number;
  migrationsFailed: number;
  chunksTransferred: number;
  bytesTransferred: number;
  activeMigrations: number;
  queuedMigrations: number;
}

// ============================================
// Migration Protocol Messages (Task 03)
// ============================================

export interface MigrationStartMessage {
  type: 'MIGRATION_START';
  payload: {
    partitionId: number;
    sourceNode: string;
    estimatedSize: number;
  };
}

export interface MigrationChunkMessage {
  type: 'MIGRATION_CHUNK';
  payload: {
    partitionId: number;
    chunkIndex: number;
    totalChunks: number;
    data: Uint8Array;
    checksum: string;
  };
}

export interface MigrationChunkAckMessage {
  type: 'MIGRATION_CHUNK_ACK';
  payload: {
    partitionId: number;
    chunkIndex: number;
    success: boolean;
  };
}

export interface MigrationCompleteMessage {
  type: 'MIGRATION_COMPLETE';
  payload: {
    partitionId: number;
    totalRecords: number;
    checksum: string;
  };
}

export interface MigrationVerifyMessage {
  type: 'MIGRATION_VERIFY';
  payload: {
    partitionId: number;
    success: boolean;
    checksumMatch: boolean;
  };
}

export type MigrationMessage =
  | MigrationStartMessage
  | MigrationChunkMessage
  | MigrationChunkAckMessage
  | MigrationCompleteMessage
  | MigrationVerifyMessage;

// ============================================
// Consistency Levels (Task 04)
// ============================================

export enum ConsistencyLevel {
  /** Wait for all replicas (owner + all backups) */
  STRONG = 'STRONG',
  /** Wait for majority (owner + N/2 backups) */
  QUORUM = 'QUORUM',
  /** Acknowledge after owner write only, background replication */
  EVENTUAL = 'EVENTUAL',
}

export interface WriteOptions {
  consistency?: ConsistencyLevel;
  /** Replication timeout in ms */
  timeout?: number;
}

export interface ReadOptions {
  consistency?: ConsistencyLevel;
  /** Read from backup if owner unavailable */
  allowStale?: boolean;
  /** Max acceptable lag in ms */
  maxStaleness?: number;
}

// ============================================
// Replication Configuration (Task 04)
// ============================================

export interface ReplicationConfig {
  defaultConsistency: ConsistencyLevel;
  /** Max queued operations (default: 10000) */
  queueSizeLimit: number;
  /** Operations per batch (default: 100) */
  batchSize: number;
  /** Batch flush interval in ms (default: 50) */
  batchIntervalMs: number;
  /** Ack timeout in ms (default: 5000) */
  ackTimeoutMs: number;
  /** Retries before marking node unhealthy (default: 3) */
  maxRetries: number;
}

export const DEFAULT_REPLICATION_CONFIG: ReplicationConfig = {
  defaultConsistency: ConsistencyLevel.EVENTUAL,
  queueSizeLimit: 10000,
  batchSize: 100,
  batchIntervalMs: 50,
  ackTimeoutMs: 5000,
  maxRetries: 3,
};

export interface ReplicationTask {
  opId: string;
  operation: unknown; // Will be typed more specifically in server
  consistency: ConsistencyLevel;
  timestamp: number;
  retryCount: number;
}

export interface ReplicationLag {
  /** Current lag in ms */
  current: number;
  /** Average lag */
  avg: number;
  /** Maximum observed lag */
  max: number;
  /** 99th percentile lag */
  percentile99: number;
}

export interface ReplicationHealth {
  healthy: boolean;
  unhealthyNodes: string[];
  laggyNodes: string[];
  avgLagMs: number;
}

export interface ReplicationResult {
  success: boolean;
  ackedBy: string[];
}

// ============================================
// Replication Protocol Messages (Task 04)
// ============================================

export interface ReplicationMessage {
  type: 'REPLICATION';
  payload: {
    opId: string;
    operation: unknown;
    consistency: ConsistencyLevel;
  };
}

export interface ReplicationBatchMessage {
  type: 'REPLICATION_BATCH';
  payload: {
    operations: unknown[];
    opIds: string[];
  };
}

export interface ReplicationAckMessage {
  type: 'REPLICATION_ACK';
  payload: {
    opId: string;
    success: boolean;
    timestamp: number;
  };
}

export interface ReplicationBatchAckMessage {
  type: 'REPLICATION_BATCH_ACK';
  payload: {
    opIds: string[];
    success: boolean;
    timestamp: number;
  };
}

export type ReplicationProtocolMessage =
  | ReplicationMessage
  | ReplicationBatchMessage
  | ReplicationAckMessage
  | ReplicationBatchAckMessage;

// ============================================
// Constants
// ============================================

export const PARTITION_COUNT = 271;
export const DEFAULT_BACKUP_COUNT = 1;
