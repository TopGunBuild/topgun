import { SyncEngine } from './SyncEngine';
import { TopGunClient } from './TopGunClient';
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
export { BackpressureError, DEFAULT_BACKPRESSURE_CONFIG };
export { logger } from './utils/logger';

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
