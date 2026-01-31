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

// Union MessageSchema (combines all message types)
import { z } from 'zod';
import { AuthMessageSchema } from './base-schemas';
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
} from './sync-schemas';
import { QuerySubMessageSchema, QueryUnsubMessageSchema } from './query-schemas';
import {
  SearchMessageSchema,
  SearchRespMessageSchema,
  SearchSubMessageSchema,
  SearchUpdateMessageSchema,
  SearchUnsubMessageSchema,
} from './search-schemas';
import {
  PartitionMapRequestSchema,
  ClusterSubRegisterMessageSchema,
  ClusterSubAckMessageSchema,
  ClusterSubUpdateMessageSchema,
  ClusterSubUnregisterMessageSchema,
} from './cluster-schemas';
import {
  TopicSubSchema,
  TopicUnsubSchema,
  TopicPubSchema,
  LockRequestSchema,
  LockReleaseSchema,
  CounterRequestSchema,
  CounterSyncSchema,
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

export const MessageSchema = z.discriminatedUnion('type', [
  AuthMessageSchema,
  QuerySubMessageSchema,
  QueryUnsubMessageSchema,
  ClientOpMessageSchema,
  OpBatchMessageSchema,
  SyncInitMessageSchema,
  SyncRespRootMessageSchema,
  SyncRespBucketsMessageSchema,
  SyncRespLeafMessageSchema,
  MerkleReqBucketMessageSchema,
  LockRequestSchema,
  LockReleaseSchema,
  TopicSubSchema,
  TopicUnsubSchema,
  TopicPubSchema,
  PingMessageSchema,
  PongMessageSchema,
  ORMapSyncInitSchema,
  ORMapSyncRespRootSchema,
  ORMapSyncRespBucketsSchema,
  ORMapMerkleReqBucketSchema,
  ORMapSyncRespLeafSchema,
  ORMapDiffRequestSchema,
  ORMapDiffResponseSchema,
  ORMapPushDiffSchema,
  PartitionMapRequestSchema,
  CounterRequestSchema,
  CounterSyncSchema,
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
  SearchMessageSchema,
  SearchRespMessageSchema,
  SearchSubMessageSchema,
  SearchUpdateMessageSchema,
  SearchUnsubMessageSchema,
  ClusterSubRegisterMessageSchema,
  ClusterSubAckMessageSchema,
  ClusterSubUpdateMessageSchema,
  ClusterSubUnregisterMessageSchema,
]);

export type Message = z.infer<typeof MessageSchema>;
