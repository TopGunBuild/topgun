import type { HLC, PermissionPolicy } from '@topgunbuild/core';
import type { MetricsService } from '../monitoring/MetricsService';
import type { SecurityManager } from '../security/SecurityManager';
import type { StripedEventExecutor } from '../utils/StripedEventExecutor';
import type { BackpressureRegulator } from '../utils/BackpressureRegulator';
import type { WorkerPool, MerkleWorker, CRDTMergeWorker, SerializationWorker, WorkerPoolConfig } from '../workers';

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

// Placeholder interfaces for later sub-specs
export interface NetworkModule { /* defined in SPEC-011c */ }
export interface ClusterModule { /* defined in SPEC-011b */ }
export interface StorageModule { /* defined in SPEC-011b */ }
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
