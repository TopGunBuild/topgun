import type { HLC, PermissionPolicy, ConsistencyLevel, ReplicationConfig, FullTextIndexConfig } from '@topgunbuild/core';
import type { MetricsService } from '../monitoring/MetricsService';
import type { SecurityManager } from '../security/SecurityManager';
import type { StripedEventExecutor } from '../utils/StripedEventExecutor';
import type { BackpressureRegulator } from '../utils/BackpressureRegulator';
import type { WorkerPool, MerkleWorker, CRDTMergeWorker, SerializationWorker, WorkerPoolConfig } from '../workers';
import type { ClusterManager } from '../cluster/ClusterManager';
import type { PartitionService } from '../cluster/PartitionService';
import type { ReplicationPipeline } from '../cluster/ReplicationPipeline';
import type { LockManager } from '../cluster/LockManager';
import type { MerkleTreeManager } from '../cluster/MerkleTreeManager';
import type { PartitionReassigner } from '../cluster/PartitionReassigner';
import type { ReadReplicaHandler } from '../cluster/ReadReplicaHandler';
import type { RepairScheduler } from '../cluster/RepairScheduler';
import type { ClusterTLSConfig } from '../types/TLSConfig';
import type { StorageManager } from '../coordinator/storage-manager';
import type { QueryRegistry } from '../query/QueryRegistry';
import type { ObjectPool, PooledEventPayload } from '../memory';
import type { TaskletScheduler } from '../tasklet';
import type { WriteAckManager } from '../ack/WriteAckManager';
import type { IServerStorage } from '../storage';

// Core module - no dependencies
export interface CoreModule {
  hlc: HLC;
  metricsService: MetricsService;
  securityManager: SecurityManager;
  eventExecutor: StripedEventExecutor;
  backpressure: BackpressureRegulator;
}

export interface CoreModuleConfig {
  nodeId: string;
  eventStripeCount?: number;
  eventQueueCapacity?: number;
  backpressureEnabled?: boolean;
  backpressureSyncFrequency?: number;
  backpressureMaxPending?: number;
  backpressureBackoffMs?: number;
  securityPolicies?: PermissionPolicy[];
}

// Worker module - optional, depends on config
export interface WorkerModule {
  workerPool?: WorkerPool;
  merkleWorker?: MerkleWorker;
  crdtMergeWorker?: CRDTMergeWorker;
  serializationWorker?: SerializationWorker;
}

export interface WorkerModuleConfig {
  workerPoolEnabled?: boolean;
  workerPoolConfig?: Partial<WorkerPoolConfig>;
}

// Cluster module - depends on core
export interface ClusterModuleConfig {
  nodeId: string;
  host?: string;
  clusterPort?: number;
  peers?: string[];
  resolvePeers?: () => string[];
  discovery?: 'manual' | 'kubernetes';
  serviceName?: string;
  discoveryInterval?: number;
  clusterTls?: ClusterTLSConfig;
  replicationEnabled?: boolean;
  defaultConsistency?: ConsistencyLevel;
  replicationConfig?: Partial<ReplicationConfig>;
}

export interface ClusterModuleDeps {
  hlc: HLC;
}

export interface ClusterModule {
  cluster: ClusterManager;
  partitionService: PartitionService;
  replicationPipeline?: ReplicationPipeline;
  lockManager: LockManager;
  merkleTreeManager: MerkleTreeManager;
  partitionReassigner: PartitionReassigner;
  readReplicaHandler: ReadReplicaHandler;
  repairScheduler: RepairScheduler;
}

// Storage module - depends on core and cluster
export interface StorageModuleConfig {
  nodeId: string;
  storage?: IServerStorage;
  fullTextSearch?: Record<string, FullTextIndexConfig>;
  writeAckTimeout?: number;
}

export interface StorageModuleDeps {
  hlc: HLC;
  metricsService: MetricsService;
  partitionService: PartitionService;
}

export interface StorageModule {
  storageManager: StorageManager;
  queryRegistry: QueryRegistry;
  eventPayloadPool: ObjectPool<PooledEventPayload>;
  taskletScheduler: TaskletScheduler;
  writeAckManager: WriteAckManager;
}

// Placeholder interfaces for later sub-specs
export interface NetworkModule { /* defined in SPEC-011c */ }
export interface HandlersModule { /* defined in SPEC-011d */ }
export interface SearchModule { /* defined in SPEC-011e */ }
export interface LifecycleModule { /* defined in SPEC-011e */ }

// All modules combined
export interface ServerModules {
  core: CoreModule;
  network: NetworkModule;
  cluster: ClusterModule;
  storage: StorageModule;
  workers: WorkerModule;
  handlers: HandlersModule;
  search: SearchModule;
  lifecycle: LifecycleModule;
}
