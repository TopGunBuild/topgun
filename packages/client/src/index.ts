import { SyncEngine } from './SyncEngine';
import { TopGunClient } from './TopGunClient';
import { TopGun } from './TopGun';
export * from './adapters/IDBAdapter';
export * from './adapters/EncryptedStorageAdapter';
import { QueryHandle } from './QueryHandle';
import { LWWMap, Predicates } from '@topgunbuild/core';
import { TopicHandle } from './TopicHandle';

// Type imports
import type { IStorageAdapter, OpLogEntry } from './IStorageAdapter';
import type { LWWRecord, PredicateNode } from '@topgunbuild/core';
import type { QueryFilter, QueryResultItem, QueryResultSource } from './QueryHandle';
import type { TopicCallback } from './TopicHandle';
import type { SyncEngineConfig, HeartbeatConfig } from './SyncEngine';

// Value exports
export { SyncEngine, TopGunClient, TopGun, QueryHandle, LWWMap, Predicates, TopicHandle };
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
  SyncEngineConfig,
  HeartbeatConfig,
};
