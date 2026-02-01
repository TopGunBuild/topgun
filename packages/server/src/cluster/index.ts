/**
 * Cluster module exports
 *
 * Clustering Improvements
 */

// Core cluster manager
export { ClusterManager, ClusterConfig, ClusterMember, ClusterMessage } from './ClusterManager';

// Partition service
export { PartitionService, PartitionDistribution, PartitionServiceConfig, PartitionServiceEvents } from './PartitionService';

// Migration manager (Task 03)
export { MigrationManager } from './MigrationManager';

// Replication pipeline (Task 04)
export { ReplicationPipeline } from './ReplicationPipeline';

// Lag tracker (Task 04)
export { LagTracker, LagInfo, LagTrackerConfig, DEFAULT_LAG_TRACKER_CONFIG } from './LagTracker';

// Lock manager
export { LockManager } from './LockManager';

// Cluster coordinator (Task 06 - Integration)
export { ClusterCoordinator, ClusterCoordinatorConfig, ClusterCoordinatorEvents, DEFAULT_CLUSTER_COORDINATOR_CONFIG } from './ClusterCoordinator';

// Failure detector
export { FailureDetector, FailureDetectorConfig, NodeState, FailureDetectorEvents, DEFAULT_FAILURE_DETECTOR_CONFIG } from './FailureDetector';

// Partition reassigner
export { PartitionReassigner, PartitionReassignerConfig, ReassignmentEvent, FailoverStatus, DEFAULT_REASSIGNER_CONFIG } from './PartitionReassigner';

// Read replica handler
export { ReadReplicaHandler, ReadReplicaConfig, ReadResult, ReadRequest, DEFAULT_READ_REPLICA_CONFIG } from './ReadReplicaHandler';

// Merkle tree manager
export { MerkleTreeManager, MerkleTreeManagerConfig, MerkleComparisonResult, PartitionMerkleInfo, DEFAULT_MERKLE_TREE_CONFIG } from './MerkleTreeManager';

// Repair scheduler
export { RepairScheduler, RepairConfig, RepairTask, RepairResult, RepairMetrics, DEFAULT_REPAIR_CONFIG } from './RepairScheduler';
