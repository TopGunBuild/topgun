import { HLC, Timestamp } from './HLC';
import { LWWMap, LWWRecord } from './LWWMap';
import { ORMap, ORMapRecord, MergeKeyResult, ORMapSnapshot } from './ORMap';
import { MerkleTree } from './MerkleTree';
import { ORMapMerkleTree, ORMapMerkleNode } from './ORMapMerkleTree';
import { hashORMapEntry, hashORMapRecord, timestampToString, compareTimestamps } from './ORMapMerkle';
import { PNCounterImpl } from './PNCounter';
import type { PNCounter, PNCounterState, PNCounterStateObject, PNCounterConfig } from './PNCounter';

export { HLC, LWWMap, ORMap, MerkleTree, ORMapMerkleTree, PNCounterImpl };
export { hashORMapEntry, hashORMapRecord, timestampToString, compareTimestamps };
export type { PNCounter, PNCounterState, PNCounterStateObject, PNCounterConfig };
export * from './utils/hash';
export * from './serializer';
export * from './predicate';
export * from './security';
export * from './schemas';
export type { Timestamp, LWWRecord, ORMapRecord, MergeKeyResult, ORMapSnapshot, ORMapMerkleNode };

// Re-export heartbeat types for convenience
export type { PingMessage, PongMessage } from './schemas';

// Write Concern exports (Phase 5.01)
export {
  WriteConcern,
  WriteOptions,
  WriteResult,
  PendingWrite,
  WRITE_CONCERN_ORDER,
  DEFAULT_WRITE_CONCERN_TIMEOUT,
  isWriteConcernAchieved,
  getHighestWriteConcernLevel,
} from './types/WriteConcern';
export type { WriteConcernValue } from './schemas';

// Cluster types exports (Phase 4)
export type {
  NodeStatus,
  NodeInfo,
  PartitionInfo,
  PartitionMap,
  PartitionMapMessage,
  PartitionMapRequestMessage,
  PartitionChange,
  PartitionMapDeltaMessage,
  NotOwnerError,
  StaleMapError,
  RoutingError,
  ConnectionPoolConfig,
  PartitionRouterConfig,
  ClusterClientConfig,
  CircuitBreakerConfig,
  ConnectionState,
  NodeHealth,
  ClusterEvents,
  // Migration types (Task 03)
  PartitionMigration,
  MigrationConfig,
  MigrationStatus,
  MigrationMetrics,
  MigrationStartMessage,
  MigrationChunkMessage,
  MigrationChunkAckMessage,
  MigrationCompleteMessage,
  MigrationVerifyMessage,
  MigrationMessage,
  // Replication types (Task 04)
  WriteOptions as ClusterWriteOptions,
  ReadOptions as ClusterReadOptions,
  ReplicationConfig,
  ReplicationTask,
  ReplicationLag,
  ReplicationHealth,
  ReplicationResult,
  ReplicationMessage,
  ReplicationBatchMessage,
  ReplicationAckMessage,
  ReplicationBatchAckMessage,
  ReplicationProtocolMessage,
} from './types/cluster';
export {
  DEFAULT_CONNECTION_POOL_CONFIG,
  DEFAULT_PARTITION_ROUTER_CONFIG,
  DEFAULT_CIRCUIT_BREAKER_CONFIG,
  PARTITION_COUNT,
  DEFAULT_BACKUP_COUNT,
  // Migration exports (Task 03)
  PartitionState,
  DEFAULT_MIGRATION_CONFIG,
  // Replication exports (Task 04)
  ConsistencyLevel,
  DEFAULT_REPLICATION_CONFIG,
} from './types/cluster';
