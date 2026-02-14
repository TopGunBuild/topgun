// packages/core/src/schemas/index.ts

// Base schemas (foundational types used by other modules)
export * from './base-schemas';

// Sync schemas (LWWMap/ORMap sync, operations)
export * from './sync-schemas';

// Query schemas (query subscriptions and responses)
export * from './query-schemas';

// Search schemas (full-text search)
export * from './search-schemas';

// Cluster schemas (distributed coordination)
export * from './cluster-schemas';

// Messaging schemas (topics, counters, locks, journal, processors, resolvers)
export * from './messaging-schemas';

// Client message schemas (server events, query updates, GC, auth, errors, locks)
export * from './client-message-schemas';

// HTTP sync schemas (stateless request/response for serverless environments)
export * from './http-sync-schemas';

// Union MessageSchema (combines all message types)
import { z } from 'zod';
import { AuthMessageSchema, AuthRequiredMessageSchema } from './base-schemas';
import {
  ClientOpMessageSchema,
  OpBatchMessageSchema,
  SyncInitMessageSchema,
  SyncRespRootMessageSchema,
  SyncRespBucketsMessageSchema,
  SyncRespLeafMessageSchema,
  MerkleReqBucketMessageSchema,
  ORMapSyncInitSchema,
  ORMapSyncRespRootSchema,
  ORMapSyncRespBucketsSchema,
  ORMapMerkleReqBucketSchema,
  ORMapSyncRespLeafSchema,
  ORMapDiffRequestSchema,
  ORMapDiffResponseSchema,
  ORMapPushDiffSchema,
  OpAckMessageSchema,
  OpRejectedMessageSchema,
  BatchMessageSchema,
} from './sync-schemas';
import {
  QuerySubMessageSchema,
  QueryUnsubMessageSchema,
  QueryRespMessageSchema,
} from './query-schemas';
import {
  SearchMessageSchema,
  SearchRespMessageSchema,
  SearchSubMessageSchema,
  SearchUpdateMessageSchema,
  SearchUnsubMessageSchema,
} from './search-schemas';
import {
  PartitionMapRequestSchema,
  PartitionMapMessageSchema,
  ClusterSubRegisterMessageSchema,
  ClusterSubAckMessageSchema,
  ClusterSubUpdateMessageSchema,
  ClusterSubUnregisterMessageSchema,
  ClusterSearchReqMessageSchema,
  ClusterSearchRespMessageSchema,
  ClusterSearchSubscribeMessageSchema,
  ClusterSearchUnsubscribeMessageSchema,
  ClusterSearchUpdateMessageSchema,
} from './cluster-schemas';
import {
  TopicSubSchema,
  TopicUnsubSchema,
  TopicPubSchema,
  TopicMessageEventSchema,
  LockRequestSchema,
  LockReleaseSchema,
  CounterRequestSchema,
  CounterSyncSchema,
  CounterResponseSchema,
  CounterUpdateSchema,
  PingMessageSchema,
  PongMessageSchema,
  EntryProcessRequestSchema,
  EntryProcessBatchRequestSchema,
  EntryProcessResponseSchema,
  EntryProcessBatchResponseSchema,
  JournalSubscribeRequestSchema,
  JournalUnsubscribeRequestSchema,
  JournalEventMessageSchema,
  JournalReadRequestSchema,
  JournalReadResponseSchema,
  RegisterResolverRequestSchema,
  RegisterResolverResponseSchema,
  UnregisterResolverRequestSchema,
  UnregisterResolverResponseSchema,
  MergeRejectedMessageSchema,
  ListResolversRequestSchema,
  ListResolversResponseSchema,
} from './messaging-schemas';
import {
  ServerEventMessageSchema,
  ServerBatchEventMessageSchema,
  QueryUpdateMessageSchema,
  GcPruneMessageSchema,
  AuthAckMessageSchema,
  AuthFailMessageSchema,
  ErrorMessageSchema,
  LockGrantedMessageSchema,
  LockReleasedMessageSchema,
  SyncResetRequiredMessageSchema,
} from './client-message-schemas';

export const MessageSchema = z.discriminatedUnion('type', [
  // --- Base ---
  AuthMessageSchema,
  AuthRequiredMessageSchema,
  // --- Sync ---
  ClientOpMessageSchema,
  OpBatchMessageSchema,
  SyncInitMessageSchema,
  SyncRespRootMessageSchema,
  SyncRespBucketsMessageSchema,
  SyncRespLeafMessageSchema,
  MerkleReqBucketMessageSchema,
  OpAckMessageSchema,
  OpRejectedMessageSchema,
  BatchMessageSchema,
  // --- ORMap Sync ---
  ORMapSyncInitSchema,
  ORMapSyncRespRootSchema,
  ORMapSyncRespBucketsSchema,
  ORMapMerkleReqBucketSchema,
  ORMapSyncRespLeafSchema,
  ORMapDiffRequestSchema,
  ORMapDiffResponseSchema,
  ORMapPushDiffSchema,
  // --- Query ---
  QuerySubMessageSchema,
  QueryUnsubMessageSchema,
  QueryRespMessageSchema,
  QueryUpdateMessageSchema,
  // --- Search ---
  SearchMessageSchema,
  SearchRespMessageSchema,
  SearchSubMessageSchema,
  SearchUpdateMessageSchema,
  SearchUnsubMessageSchema,
  // --- Cluster ---
  PartitionMapRequestSchema,
  PartitionMapMessageSchema,
  ClusterSubRegisterMessageSchema,
  ClusterSubAckMessageSchema,
  ClusterSubUpdateMessageSchema,
  ClusterSubUnregisterMessageSchema,
  ClusterSearchReqMessageSchema,
  ClusterSearchRespMessageSchema,
  ClusterSearchSubscribeMessageSchema,
  ClusterSearchUnsubscribeMessageSchema,
  ClusterSearchUpdateMessageSchema,
  // --- Messaging ---
  TopicSubSchema,
  TopicUnsubSchema,
  TopicPubSchema,
  TopicMessageEventSchema,
  LockRequestSchema,
  LockReleaseSchema,
  CounterRequestSchema,
  CounterSyncSchema,
  CounterResponseSchema,
  CounterUpdateSchema,
  PingMessageSchema,
  PongMessageSchema,
  // --- Entry Processor ---
  EntryProcessRequestSchema,
  EntryProcessBatchRequestSchema,
  EntryProcessResponseSchema,
  EntryProcessBatchResponseSchema,
  // --- Journal ---
  JournalSubscribeRequestSchema,
  JournalUnsubscribeRequestSchema,
  JournalEventMessageSchema,
  JournalReadRequestSchema,
  JournalReadResponseSchema,
  // --- Conflict Resolver ---
  RegisterResolverRequestSchema,
  RegisterResolverResponseSchema,
  UnregisterResolverRequestSchema,
  UnregisterResolverResponseSchema,
  MergeRejectedMessageSchema,
  ListResolversRequestSchema,
  ListResolversResponseSchema,
  // --- Server-to-Client ---
  ServerEventMessageSchema,
  ServerBatchEventMessageSchema,
  GcPruneMessageSchema,
  AuthAckMessageSchema,
  AuthFailMessageSchema,
  ErrorMessageSchema,
  LockGrantedMessageSchema,
  LockReleasedMessageSchema,
  SyncResetRequiredMessageSchema,
]);

export type Message = z.infer<typeof MessageSchema>;
