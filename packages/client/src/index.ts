import { SyncEngine } from './SyncEngine';
import { TopGunClient, DEFAULT_CLUSTER_CONFIG } from './TopGunClient';
import { TopGun } from './TopGun';
export * from './adapters/IDBAdapter';
export * from './adapters/EncryptedStorageAdapter';
import { QueryHandle } from './QueryHandle';
import { LWWMap, Predicates } from '@topgunbuild/core';
import { TopicHandle } from './TopicHandle';
import { SyncState, VALID_TRANSITIONS, isValidTransition } from './SyncState';
import { SyncStateMachine } from './SyncStateMachine';
import { BackpressureError } from './errors/BackpressureError';
import { DEFAULT_BACKPRESSURE_CONFIG } from './BackpressureConfig';

// Cluster imports (Phase 4)
import { ConnectionPool, PartitionRouter, ClusterClient } from './cluster';

// Connection provider imports (Phase 4.5)
import { SingleServerProvider } from './connection';

// Type imports
import type { IStorageAdapter, OpLogEntry } from './IStorageAdapter';
import type { LWWRecord, PredicateNode } from '@topgunbuild/core';
import type { QueryFilter, QueryResultItem, QueryResultSource } from './QueryHandle';
import type { TopicCallback } from './TopicHandle';
import type { BackoffConfig, HeartbeatConfig, SyncEngineConfig } from './SyncEngine';
import type { StateChangeEvent, StateChangeListener, SyncStateMachineConfig } from './SyncStateMachine';
import type {
  BackpressureConfig,
  BackpressureStrategy,
  BackpressureStatus,
  BackpressureThresholdEvent,
  OperationDroppedEvent,
} from './BackpressureConfig';

// Value exports
export { SyncEngine, TopGunClient, TopGun, QueryHandle, LWWMap, Predicates, TopicHandle };
export { SyncState, VALID_TRANSITIONS, isValidTransition, SyncStateMachine };
export { BackpressureError, DEFAULT_BACKPRESSURE_CONFIG, DEFAULT_CLUSTER_CONFIG };
export { logger } from './utils/logger';

// Cluster exports (Phase 4)
export { ConnectionPool, PartitionRouter, ClusterClient };
export type {
  ConnectionPoolEvents,
  RoutingResult,
  PartitionRouterEvents,
  ClusterClientEvents,
  ClusterRoutingMode,
  RoutingMetrics,
  CircuitState,
} from './cluster';

// Connection provider exports (Phase 4.5)
export { SingleServerProvider };
export type {
  IConnectionProvider,
  ConnectionProviderEvent,
  ConnectionEventHandler,
  SingleServerProviderConfig,
} from './types';

// TopGunClient cluster config types (Phase 4.5)
export type { TopGunClusterConfig, TopGunClientConfig } from './TopGunClient';

// Type exports
export type {
  IStorageAdapter,
  OpLogEntry,
  LWWRecord,
  PredicateNode,
  QueryFilter,
  QueryResultItem,
  QueryResultSource,
  TopicCallback,
  BackoffConfig,
  HeartbeatConfig,
  SyncEngineConfig,
  StateChangeEvent,
  StateChangeListener,
  SyncStateMachineConfig,
  BackpressureConfig,
  BackpressureStrategy,
  BackpressureStatus,
  BackpressureThresholdEvent,
  OperationDroppedEvent,
};
