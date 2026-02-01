// packages/core/src/schemas/messaging-schemas.ts
import { z } from 'zod';
import { TimestampSchema } from './base-schemas';

// --- Topic Messages ---
export const TopicSubSchema = z.object({
  type: z.literal('TOPIC_SUB'),
  payload: z.object({ topic: z.string() }),
});

export const TopicUnsubSchema = z.object({
  type: z.literal('TOPIC_UNSUB'),
  payload: z.object({ topic: z.string() }),
});

export const TopicPubSchema = z.object({
  type: z.literal('TOPIC_PUB'),
  payload: z.object({
    topic: z.string(),
    data: z.any(),
  }),
});

export const TopicMessageEventSchema = z.object({
  type: z.literal('TOPIC_MESSAGE'),
  payload: z.object({
    topic: z.string(),
    data: z.any(),
    publisherId: z.string().optional(),
    timestamp: z.number(),
  }),
});

// --- Lock Messages ---
export const LockRequestSchema = z.object({
  type: z.literal('LOCK_REQUEST'),
  payload: z.object({
    requestId: z.string(),
    name: z.string(),
    ttl: z.number().optional(),
  }),
});

export const LockReleaseSchema = z.object({
  type: z.literal('LOCK_RELEASE'),
  payload: z.object({
    requestId: z.string().optional(),
    name: z.string(),
    fencingToken: z.number(),
  }),
});

// --- PN Counter Messages ---
export const PNCounterStateObjectSchema = z.object({
  p: z.record(z.string(), z.number()),
  n: z.record(z.string(), z.number()),
});

export const CounterRequestSchema = z.object({
  type: z.literal('COUNTER_REQUEST'),
  payload: z.object({ name: z.string() }),
});

export const CounterSyncSchema = z.object({
  type: z.literal('COUNTER_SYNC'),
  payload: z.object({
    name: z.string(),
    state: PNCounterStateObjectSchema,
  }),
});

export const CounterResponseSchema = z.object({
  type: z.literal('COUNTER_RESPONSE'),
  payload: z.object({
    name: z.string(),
    state: PNCounterStateObjectSchema,
  }),
});

export const CounterUpdateSchema = z.object({
  type: z.literal('COUNTER_UPDATE'),
  payload: z.object({
    name: z.string(),
    state: PNCounterStateObjectSchema,
  }),
});

// --- Heartbeat Messages ---
export const PingMessageSchema = z.object({
  type: z.literal('PING'),
  timestamp: z.number(),
});
export type PingMessage = z.infer<typeof PingMessageSchema>;

export const PongMessageSchema = z.object({
  type: z.literal('PONG'),
  timestamp: z.number(),
  serverTime: z.number(),
});
export type PongMessage = z.infer<typeof PongMessageSchema>;

// --- Entry Processor Messages ---
export const EntryProcessorSchema = z.object({
  name: z.string().min(1).max(100),
  code: z.string().min(1).max(10000),
  args: z.unknown().optional(),
});

export const EntryProcessRequestSchema = z.object({
  type: z.literal('ENTRY_PROCESS'),
  requestId: z.string(),
  mapName: z.string(),
  key: z.string(),
  processor: EntryProcessorSchema,
});
export type EntryProcessRequest = z.infer<typeof EntryProcessRequestSchema>;

export const EntryProcessBatchRequestSchema = z.object({
  type: z.literal('ENTRY_PROCESS_BATCH'),
  requestId: z.string(),
  mapName: z.string(),
  keys: z.array(z.string()),
  processor: EntryProcessorSchema,
});
export type EntryProcessBatchRequest = z.infer<typeof EntryProcessBatchRequestSchema>;

export const EntryProcessResponseSchema = z.object({
  type: z.literal('ENTRY_PROCESS_RESPONSE'),
  requestId: z.string(),
  success: z.boolean(),
  result: z.unknown().optional(),
  newValue: z.unknown().optional(),
  error: z.string().optional(),
});
export type EntryProcessResponse = z.infer<typeof EntryProcessResponseSchema>;

export const EntryProcessKeyResultSchema = z.object({
  success: z.boolean(),
  result: z.unknown().optional(),
  newValue: z.unknown().optional(),
  error: z.string().optional(),
});
export type EntryProcessKeyResult = z.infer<typeof EntryProcessKeyResultSchema>;

export const EntryProcessBatchResponseSchema = z.object({
  type: z.literal('ENTRY_PROCESS_BATCH_RESPONSE'),
  requestId: z.string(),
  results: z.record(z.string(), EntryProcessKeyResultSchema),
});
export type EntryProcessBatchResponse = z.infer<typeof EntryProcessBatchResponseSchema>;

// --- Event Journal Messages ---
export const JournalEventTypeSchema = z.enum(['PUT', 'UPDATE', 'DELETE']);
export type JournalEventType = z.infer<typeof JournalEventTypeSchema>;

export const JournalEventDataSchema = z.object({
  sequence: z.string(),
  type: JournalEventTypeSchema,
  mapName: z.string(),
  key: z.string(),
  value: z.unknown().optional(),
  previousValue: z.unknown().optional(),
  timestamp: TimestampSchema,
  nodeId: z.string(),
  metadata: z.record(z.string(), z.unknown()).optional(),
});
export type JournalEventData = z.infer<typeof JournalEventDataSchema>;

export const JournalSubscribeRequestSchema = z.object({
  type: z.literal('JOURNAL_SUBSCRIBE'),
  requestId: z.string(),
  fromSequence: z.string().optional(),
  mapName: z.string().optional(),
  types: z.array(JournalEventTypeSchema).optional(),
});
export type JournalSubscribeRequest = z.infer<typeof JournalSubscribeRequestSchema>;

export const JournalUnsubscribeRequestSchema = z.object({
  type: z.literal('JOURNAL_UNSUBSCRIBE'),
  subscriptionId: z.string(),
});
export type JournalUnsubscribeRequest = z.infer<typeof JournalUnsubscribeRequestSchema>;

export const JournalEventMessageSchema = z.object({
  type: z.literal('JOURNAL_EVENT'),
  event: JournalEventDataSchema,
});
export type JournalEventMessage = z.infer<typeof JournalEventMessageSchema>;

export const JournalReadRequestSchema = z.object({
  type: z.literal('JOURNAL_READ'),
  requestId: z.string(),
  fromSequence: z.string(),
  limit: z.number().optional(),
  mapName: z.string().optional(),
});
export type JournalReadRequest = z.infer<typeof JournalReadRequestSchema>;

export const JournalReadResponseSchema = z.object({
  type: z.literal('JOURNAL_READ_RESPONSE'),
  requestId: z.string(),
  events: z.array(JournalEventDataSchema),
  hasMore: z.boolean(),
});
export type JournalReadResponse = z.infer<typeof JournalReadResponseSchema>;

// --- Conflict Resolver Messages ---
export const ConflictResolverSchema = z.object({
  name: z.string().min(1).max(100),
  code: z.string().max(50000),
  priority: z.number().int().min(0).max(100).optional(),
  keyPattern: z.string().optional(),
});
export type ConflictResolver = z.infer<typeof ConflictResolverSchema>;

export const RegisterResolverRequestSchema = z.object({
  type: z.literal('REGISTER_RESOLVER'),
  requestId: z.string(),
  mapName: z.string(),
  resolver: ConflictResolverSchema,
});
export type RegisterResolverRequest = z.infer<typeof RegisterResolverRequestSchema>;

export const RegisterResolverResponseSchema = z.object({
  type: z.literal('REGISTER_RESOLVER_RESPONSE'),
  requestId: z.string(),
  success: z.boolean(),
  error: z.string().optional(),
});
export type RegisterResolverResponse = z.infer<typeof RegisterResolverResponseSchema>;

export const UnregisterResolverRequestSchema = z.object({
  type: z.literal('UNREGISTER_RESOLVER'),
  requestId: z.string(),
  mapName: z.string(),
  resolverName: z.string(),
});
export type UnregisterResolverRequest = z.infer<typeof UnregisterResolverRequestSchema>;

export const UnregisterResolverResponseSchema = z.object({
  type: z.literal('UNREGISTER_RESOLVER_RESPONSE'),
  requestId: z.string(),
  success: z.boolean(),
  error: z.string().optional(),
});
export type UnregisterResolverResponse = z.infer<typeof UnregisterResolverResponseSchema>;

export const MergeRejectedMessageSchema = z.object({
  type: z.literal('MERGE_REJECTED'),
  mapName: z.string(),
  key: z.string(),
  attemptedValue: z.unknown(),
  reason: z.string(),
  timestamp: TimestampSchema,
});
export type MergeRejectedMessage = z.infer<typeof MergeRejectedMessageSchema>;

export const ListResolversRequestSchema = z.object({
  type: z.literal('LIST_RESOLVERS'),
  requestId: z.string(),
  mapName: z.string().optional(),
});
export type ListResolversRequest = z.infer<typeof ListResolversRequestSchema>;

export const ListResolversResponseSchema = z.object({
  type: z.literal('LIST_RESOLVERS_RESPONSE'),
  requestId: z.string(),
  resolvers: z.array(z.object({
    mapName: z.string(),
    name: z.string(),
    priority: z.number().optional(),
    keyPattern: z.string().optional(),
  })),
});
export type ListResolversResponse = z.infer<typeof ListResolversResponseSchema>;
