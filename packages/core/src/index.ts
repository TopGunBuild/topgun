import { HLC, Timestamp } from './HLC';
import { LWWMap, LWWRecord } from './LWWMap';
import { ORMap, ORMapRecord, MergeKeyResult, ORMapSnapshot } from './ORMap';
import { MerkleTree } from './MerkleTree';
import { ORMapMerkleTree, ORMapMerkleNode } from './ORMapMerkleTree';
import { hashORMapEntry, hashORMapRecord, timestampToString, compareTimestamps } from './ORMapMerkle';

export { HLC, LWWMap, ORMap, MerkleTree, ORMapMerkleTree };
export { hashORMapEntry, hashORMapRecord, timestampToString, compareTimestamps };
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
  PartitionChange,
  PartitionMapDeltaMessage,
  NotOwnerError,
  StaleMapError,
  RoutingError,
  ConnectionPoolConfig,
  PartitionRouterConfig,
  ClusterClientConfig,
  ConnectionState,
  NodeHealth,
  ClusterEvents,
} from './types/cluster';
export {
  DEFAULT_CONNECTION_POOL_CONFIG,
  DEFAULT_PARTITION_ROUTER_CONFIG,
  PARTITION_COUNT,
  DEFAULT_BACKUP_COUNT,
} from './types/cluster';
