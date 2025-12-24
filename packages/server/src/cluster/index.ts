/**
 * Cluster module exports
 *
 * Phase 4: Clustering Improvements
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
