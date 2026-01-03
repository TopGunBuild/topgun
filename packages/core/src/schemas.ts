import { z } from 'zod';

// --- Write Concern Types ---

/**
 * Write Concern schema - defines when an operation is considered acknowledged.
 */
export const WriteConcernSchema = z.enum([
  'FIRE_AND_FORGET',
  'MEMORY',
  'APPLIED',
  'REPLICATED',
  'PERSISTED',
]);

export type WriteConcernValue = z.infer<typeof WriteConcernSchema>;

// --- Basic Types ---

export const TimestampSchema = z.object({
  millis: z.union([z.number(), z.bigint()]).transform(Number),
  counter: z.union([z.number(), z.bigint()]).transform(Number),
  nodeId: z.string(),
});

export const LWWRecordSchema = z.object({
  value: z.any().nullable(),
  timestamp: TimestampSchema,
  ttlMs: z.number().optional(),
});

export const ORMapRecordSchema = z.object({
  value: z.any(),
  timestamp: TimestampSchema,
  tag: z.string(),
  ttlMs: z.number().optional(),
});

// --- Predicate Types ---

export const PredicateOpSchema = z.enum([
  'eq', 'neq', 'gt', 'gte', 'lt', 'lte', 'like', 'regex', 'and', 'or', 'not'
]);

// Recursive schema for PredicateNode
export const PredicateNodeSchema: z.ZodType<any> = z.lazy(() => z.object({
  op: PredicateOpSchema,
  attribute: z.string().optional(),
  value: z.any().optional(),
  children: z.array(PredicateNodeSchema).optional(),
}));

// --- Query Types ---

export const QuerySchema = z.object({
  where: z.record(z.string(), z.any()).optional(),
  predicate: PredicateNodeSchema.optional(),
  sort: z.record(z.string(), z.enum(['asc', 'desc'])).optional(),
  limit: z.number().optional(),
  offset: z.number().optional(),
});

// --- Client Operation Types ---

export const ClientOpSchema = z.object({
  id: z.string().optional(),
  mapName: z.string(),
  key: z.string(),
  // Permissive opType to match ServerCoordinator behavior logic
  // It can be 'REMOVE', 'OR_ADD', 'OR_REMOVE' or undefined/other (implies PUT/LWW)
  opType: z.string().optional(),
  record: LWWRecordSchema.nullable().optional(),
  orRecord: ORMapRecordSchema.nullable().optional(),
  orTag: z.string().nullable().optional(),
  // Write Concern fields (Phase 5.01)
  writeConcern: WriteConcernSchema.optional(),
  timeout: z.number().optional(),
});

// --- Message Schemas ---

export const AuthMessageSchema = z.object({
  type: z.literal('AUTH'),
  token: z.string(),
});

export const QuerySubMessageSchema = z.object({
  type: z.literal('QUERY_SUB'),
  payload: z.object({
    queryId: z.string(),
    mapName: z.string(),
    query: QuerySchema,
  }),
});

export const QueryUnsubMessageSchema = z.object({
  type: z.literal('QUERY_UNSUB'),
  payload: z.object({
    queryId: z.string(),
  }),
});

export const ClientOpMessageSchema = z.object({
  type: z.literal('CLIENT_OP'),
  payload: ClientOpSchema,
});

export const OpBatchMessageSchema = z.object({
  type: z.literal('OP_BATCH'),
  payload: z.object({
    ops: z.array(ClientOpSchema),
    // Batch-level Write Concern (can be overridden per-op)
    writeConcern: WriteConcernSchema.optional(),
    timeout: z.number().optional(),
  }),
});

export const SyncInitMessageSchema = z.object({
  type: z.literal('SYNC_INIT'),
  mapName: z.string(),
  lastSyncTimestamp: z.number().optional(),
});

export const SyncRespRootMessageSchema = z.object({
  type: z.literal('SYNC_RESP_ROOT'),
  payload: z.object({
    mapName: z.string(),
    rootHash: z.number(),
    timestamp: TimestampSchema,
  }),
});

export const SyncRespBucketsMessageSchema = z.object({
  type: z.literal('SYNC_RESP_BUCKETS'),
  payload: z.object({
    mapName: z.string(),
    path: z.string(),
    buckets: z.record(z.string(), z.number()),
  }),
});

export const SyncRespLeafMessageSchema = z.object({
  type: z.literal('SYNC_RESP_LEAF'),
  payload: z.object({
    mapName: z.string(),
    path: z.string(),
    records: z.array(z.object({
      key: z.string(),
      record: LWWRecordSchema,
    })),
  }),
});

export const MerkleReqBucketMessageSchema = z.object({
  type: z.literal('MERKLE_REQ_BUCKET'),
  payload: z.object({
    mapName: z.string(),
    path: z.string(),
  }),
});

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

// --- Topic Messages ---

export const TopicSubSchema = z.object({
  type: z.literal('TOPIC_SUB'),
  payload: z.object({
    topic: z.string(),
  }),
});

export const TopicUnsubSchema = z.object({
  type: z.literal('TOPIC_UNSUB'),
  payload: z.object({
    topic: z.string(),
  }),
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

// --- PN Counter Messages (Phase 5.2) ---

export const PNCounterStateObjectSchema = z.object({
  p: z.record(z.string(), z.number()), // positive counts per node
  n: z.record(z.string(), z.number()), // negative counts per node
});

export const CounterRequestSchema = z.object({
  type: z.literal('COUNTER_REQUEST'),
  payload: z.object({
    name: z.string(),
  }),
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
  timestamp: z.number(), // Client's Date.now()
});

export const PongMessageSchema = z.object({
  type: z.literal('PONG'),
  timestamp: z.number(),   // Echo back client's timestamp
  serverTime: z.number(),  // Server's Date.now() (for clock skew detection)
});

// --- Batched Messages ---

/**
 * BATCH: Server sends multiple messages batched together.
 * Uses length-prefixed binary format for efficiency.
 * Format: [4 bytes: count][4 bytes: len1][msg1][4 bytes: len2][msg2]...
 */
export const BatchMessageSchema = z.object({
  type: z.literal('BATCH'),
  count: z.number(),
  data: z.instanceof(Uint8Array),
});

// --- ORMap Sync Messages ---

/**
 * ORMAP_SYNC_INIT: Client initiates ORMap sync
 * Sends root hash and bucket hashes to server
 */
export const ORMapSyncInitSchema = z.object({
  type: z.literal('ORMAP_SYNC_INIT'),
  mapName: z.string(),
  rootHash: z.number(),
  bucketHashes: z.record(z.string(), z.number()), // path -> hash
  lastSyncTimestamp: z.number().optional(),
});

/**
 * ORMAP_SYNC_RESP_ROOT: Server responds with its root hash
 */
export const ORMapSyncRespRootSchema = z.object({
  type: z.literal('ORMAP_SYNC_RESP_ROOT'),
  payload: z.object({
    mapName: z.string(),
    rootHash: z.number(),
    timestamp: TimestampSchema,
  }),
});

/**
 * ORMAP_SYNC_RESP_BUCKETS: Server sends bucket hashes for comparison
 */
export const ORMapSyncRespBucketsSchema = z.object({
  type: z.literal('ORMAP_SYNC_RESP_BUCKETS'),
  payload: z.object({
    mapName: z.string(),
    path: z.string(),
    buckets: z.record(z.string(), z.number()),
  }),
});

/**
 * ORMAP_MERKLE_REQ_BUCKET: Client requests bucket details
 */
export const ORMapMerkleReqBucketSchema = z.object({
  type: z.literal('ORMAP_MERKLE_REQ_BUCKET'),
  payload: z.object({
    mapName: z.string(),
    path: z.string(),
  }),
});

/**
 * ORMAP_SYNC_RESP_LEAF: Server sends actual records for differing keys
 */
export const ORMapSyncRespLeafSchema = z.object({
  type: z.literal('ORMAP_SYNC_RESP_LEAF'),
  payload: z.object({
    mapName: z.string(),
    path: z.string(),
    entries: z.array(z.object({
      key: z.string(),
      records: z.array(ORMapRecordSchema),
      tombstones: z.array(z.string()), // Tombstone tags for this key's records
    })),
  }),
});

/**
 * ORMAP_DIFF_REQUEST: Client requests data for specific keys
 */
export const ORMapDiffRequestSchema = z.object({
  type: z.literal('ORMAP_DIFF_REQUEST'),
  payload: z.object({
    mapName: z.string(),
    keys: z.array(z.string()),
  }),
});

/**
 * ORMAP_DIFF_RESPONSE: Server responds with data for requested keys
 */
export const ORMapDiffResponseSchema = z.object({
  type: z.literal('ORMAP_DIFF_RESPONSE'),
  payload: z.object({
    mapName: z.string(),
    entries: z.array(z.object({
      key: z.string(),
      records: z.array(ORMapRecordSchema),
      tombstones: z.array(z.string()),
    })),
  }),
});

/**
 * ORMAP_PUSH_DIFF: Client pushes local diffs to server
 */
export const ORMapPushDiffSchema = z.object({
  type: z.literal('ORMAP_PUSH_DIFF'),
  payload: z.object({
    mapName: z.string(),
    entries: z.array(z.object({
      key: z.string(),
      records: z.array(ORMapRecordSchema),
      tombstones: z.array(z.string()),
    })),
  }),
});

// --- Phase 4: Partition Map Schemas ---

/**
 * PARTITION_MAP_REQUEST: Client requests current partition map
 */
export const PartitionMapRequestSchema = z.object({
  type: z.literal('PARTITION_MAP_REQUEST'),
  payload: z.object({
    currentVersion: z.number().optional(),
  }).optional(),
});

// --- Entry Processor Messages (Phase 5.03) ---

/**
 * Entry processor definition schema.
 */
export const EntryProcessorSchema = z.object({
  name: z.string().min(1).max(100),
  code: z.string().min(1).max(10000),
  args: z.unknown().optional(),
});

/**
 * ENTRY_PROCESS: Client requests atomic operation on single key.
 */
export const EntryProcessRequestSchema = z.object({
  type: z.literal('ENTRY_PROCESS'),
  requestId: z.string(),
  mapName: z.string(),
  key: z.string(),
  processor: EntryProcessorSchema,
});

/**
 * ENTRY_PROCESS_BATCH: Client requests atomic operation on multiple keys.
 */
export const EntryProcessBatchRequestSchema = z.object({
  type: z.literal('ENTRY_PROCESS_BATCH'),
  requestId: z.string(),
  mapName: z.string(),
  keys: z.array(z.string()),
  processor: EntryProcessorSchema,
});

/**
 * ENTRY_PROCESS_RESPONSE: Server responds to single-key processor request.
 */
export const EntryProcessResponseSchema = z.object({
  type: z.literal('ENTRY_PROCESS_RESPONSE'),
  requestId: z.string(),
  success: z.boolean(),
  result: z.unknown().optional(),
  newValue: z.unknown().optional(),
  error: z.string().optional(),
});

/**
 * Individual key result in batch response.
 */
export const EntryProcessKeyResultSchema = z.object({
  success: z.boolean(),
  result: z.unknown().optional(),
  newValue: z.unknown().optional(),
  error: z.string().optional(),
});

/**
 * ENTRY_PROCESS_BATCH_RESPONSE: Server responds to multi-key processor request.
 */
export const EntryProcessBatchResponseSchema = z.object({
  type: z.literal('ENTRY_PROCESS_BATCH_RESPONSE'),
  requestId: z.string(),
  results: z.record(z.string(), EntryProcessKeyResultSchema),
});

// --- Event Journal Messages (Phase 5.04) ---

/**
 * Journal event type schema.
 */
export const JournalEventTypeSchema = z.enum(['PUT', 'UPDATE', 'DELETE']);

/**
 * Journal event data (serialized for network).
 */
export const JournalEventDataSchema = z.object({
  sequence: z.string(), // bigint as string
  type: JournalEventTypeSchema,
  mapName: z.string(),
  key: z.string(),
  value: z.unknown().optional(),
  previousValue: z.unknown().optional(),
  timestamp: TimestampSchema,
  nodeId: z.string(),
  metadata: z.record(z.string(), z.unknown()).optional(),
});

/**
 * JOURNAL_SUBSCRIBE: Client subscribes to journal events.
 */
export const JournalSubscribeRequestSchema = z.object({
  type: z.literal('JOURNAL_SUBSCRIBE'),
  requestId: z.string(),
  fromSequence: z.string().optional(), // bigint as string
  mapName: z.string().optional(),
  types: z.array(JournalEventTypeSchema).optional(),
});

/**
 * JOURNAL_UNSUBSCRIBE: Client unsubscribes from journal events.
 */
export const JournalUnsubscribeRequestSchema = z.object({
  type: z.literal('JOURNAL_UNSUBSCRIBE'),
  subscriptionId: z.string(),
});

/**
 * JOURNAL_EVENT: Server sends journal event to client.
 */
export const JournalEventMessageSchema = z.object({
  type: z.literal('JOURNAL_EVENT'),
  event: JournalEventDataSchema,
});

/**
 * JOURNAL_READ: Client requests events from journal.
 */
export const JournalReadRequestSchema = z.object({
  type: z.literal('JOURNAL_READ'),
  requestId: z.string(),
  fromSequence: z.string(),
  limit: z.number().optional(),
  mapName: z.string().optional(),
});

/**
 * JOURNAL_READ_RESPONSE: Server responds with journal events.
 */
export const JournalReadResponseSchema = z.object({
  type: z.literal('JOURNAL_READ_RESPONSE'),
  requestId: z.string(),
  events: z.array(JournalEventDataSchema),
  hasMore: z.boolean(),
});

// --- Full-Text Search Messages (Phase 11.1) ---

/**
 * Search options schema for FTS queries.
 */
export const SearchOptionsSchema = z.object({
  limit: z.number().optional(),
  minScore: z.number().optional(),
  boost: z.record(z.string(), z.number()).optional(),
});

/**
 * SEARCH: Client requests one-shot BM25 search.
 */
export const SearchPayloadSchema = z.object({
  requestId: z.string(),
  mapName: z.string(),
  query: z.string(),
  options: SearchOptionsSchema.optional(),
});

export const SearchMessageSchema = z.object({
  type: z.literal('SEARCH'),
  payload: SearchPayloadSchema,
});

/**
 * SEARCH_RESP: Server responds with search results.
 */
export const SearchRespPayloadSchema = z.object({
  requestId: z.string(),
  results: z.array(z.object({
    key: z.string(),
    value: z.unknown(),
    score: z.number(),
    matchedTerms: z.array(z.string()),
  })),
  totalCount: z.number(),
  error: z.string().optional(),
});

export const SearchRespMessageSchema = z.object({
  type: z.literal('SEARCH_RESP'),
  payload: SearchRespPayloadSchema,
});

// --- Conflict Resolver Messages (Phase 5.05) ---

/**
 * Conflict resolver definition schema (wire format).
 */
export const ConflictResolverSchema = z.object({
  name: z.string().min(1).max(100),
  code: z.string().max(50000),
  priority: z.number().int().min(0).max(100).optional(),
  keyPattern: z.string().optional(),
});

/**
 * REGISTER_RESOLVER: Client registers a conflict resolver on server.
 */
export const RegisterResolverRequestSchema = z.object({
  type: z.literal('REGISTER_RESOLVER'),
  requestId: z.string(),
  mapName: z.string(),
  resolver: ConflictResolverSchema,
});

/**
 * REGISTER_RESOLVER_RESPONSE: Server acknowledges resolver registration.
 */
export const RegisterResolverResponseSchema = z.object({
  type: z.literal('REGISTER_RESOLVER_RESPONSE'),
  requestId: z.string(),
  success: z.boolean(),
  error: z.string().optional(),
});

/**
 * UNREGISTER_RESOLVER: Client unregisters a conflict resolver.
 */
export const UnregisterResolverRequestSchema = z.object({
  type: z.literal('UNREGISTER_RESOLVER'),
  requestId: z.string(),
  mapName: z.string(),
  resolverName: z.string(),
});

/**
 * UNREGISTER_RESOLVER_RESPONSE: Server acknowledges resolver unregistration.
 */
export const UnregisterResolverResponseSchema = z.object({
  type: z.literal('UNREGISTER_RESOLVER_RESPONSE'),
  requestId: z.string(),
  success: z.boolean(),
  error: z.string().optional(),
});

/**
 * MERGE_REJECTED: Server notifies client that a merge was rejected.
 */
export const MergeRejectedMessageSchema = z.object({
  type: z.literal('MERGE_REJECTED'),
  mapName: z.string(),
  key: z.string(),
  attemptedValue: z.unknown(),
  reason: z.string(),
  timestamp: TimestampSchema,
});

/**
 * LIST_RESOLVERS: Client requests list of registered resolvers.
 */
export const ListResolversRequestSchema = z.object({
  type: z.literal('LIST_RESOLVERS'),
  requestId: z.string(),
  mapName: z.string().optional(),
});

/**
 * LIST_RESOLVERS_RESPONSE: Server responds with registered resolvers.
 */
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

// --- Write Concern Response Schemas (Phase 5.01) ---

/**
 * Individual operation result within a batch ACK
 */
export const OpResultSchema = z.object({
  opId: z.string(),
  success: z.boolean(),
  achievedLevel: WriteConcernSchema,
  error: z.string().optional(),
});

/**
 * OP_ACK: Server acknowledges write operations
 * Extended to support Write Concern levels
 */
export const OpAckMessageSchema = z.object({
  type: z.literal('OP_ACK'),
  payload: z.object({
    /** ID of the last operation in the batch (for backwards compatibility) */
    lastId: z.string(),
    /** Write Concern level achieved (for simple ACKs) */
    achievedLevel: WriteConcernSchema.optional(),
    /** Per-operation results (for batch operations with mixed Write Concern) */
    results: z.array(OpResultSchema).optional(),
  }),
});

/**
 * OP_REJECTED: Server rejects a write operation
 */
export const OpRejectedMessageSchema = z.object({
  type: z.literal('OP_REJECTED'),
  payload: z.object({
    /** Operation ID that was rejected */
    opId: z.string(),
    /** Reason for rejection */
    reason: z.string(),
    /** Error code */
    code: z.number().optional(),
  }),
});

// --- Union Schema ---

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
  // ORMap Sync Messages
  ORMapSyncInitSchema,
  ORMapSyncRespRootSchema,
  ORMapSyncRespBucketsSchema,
  ORMapMerkleReqBucketSchema,
  ORMapSyncRespLeafSchema,
  ORMapDiffRequestSchema,
  ORMapDiffResponseSchema,
  ORMapPushDiffSchema,
  // Phase 4: Partition Map
  PartitionMapRequestSchema,
  // Phase 5.2: PN Counter
  CounterRequestSchema,
  CounterSyncSchema,
  // Phase 5.03: Entry Processor
  EntryProcessRequestSchema,
  EntryProcessBatchRequestSchema,
  EntryProcessResponseSchema,
  EntryProcessBatchResponseSchema,
  // Phase 5.04: Event Journal
  JournalSubscribeRequestSchema,
  JournalUnsubscribeRequestSchema,
  JournalEventMessageSchema,
  JournalReadRequestSchema,
  JournalReadResponseSchema,
  // Phase 5.05: Conflict Resolver
  RegisterResolverRequestSchema,
  RegisterResolverResponseSchema,
  UnregisterResolverRequestSchema,
  UnregisterResolverResponseSchema,
  MergeRejectedMessageSchema,
  ListResolversRequestSchema,
  ListResolversResponseSchema,
  // Phase 11.1: Full-Text Search
  SearchMessageSchema,
  SearchRespMessageSchema,
]);

// --- Type Inference ---

export type Timestamp = z.infer<typeof TimestampSchema>;
export type LWWRecord<V = any> = z.infer<typeof LWWRecordSchema>; // Generic placeholder
export type ORMapRecord<V = any> = z.infer<typeof ORMapRecordSchema>; // Generic placeholder
// export type PredicateNode = z.infer<typeof PredicateNodeSchema>; // Conflict with predicate.ts
export type Query = z.infer<typeof QuerySchema>;
export type ClientOp = z.infer<typeof ClientOpSchema>;
export type Message = z.infer<typeof MessageSchema>;
export type PingMessage = z.infer<typeof PingMessageSchema>;
export type PongMessage = z.infer<typeof PongMessageSchema>;
export type BatchMessage = z.infer<typeof BatchMessageSchema>;

// Write Concern types (Phase 5.01)
export type OpAckMessage = z.infer<typeof OpAckMessageSchema>;
export type OpRejectedMessage = z.infer<typeof OpRejectedMessageSchema>;
export type OpResult = z.infer<typeof OpResultSchema>;

// Entry Processor types (Phase 5.03)
export type EntryProcessRequest = z.infer<typeof EntryProcessRequestSchema>;
export type EntryProcessBatchRequest = z.infer<typeof EntryProcessBatchRequestSchema>;
export type EntryProcessResponse = z.infer<typeof EntryProcessResponseSchema>;
export type EntryProcessBatchResponse = z.infer<typeof EntryProcessBatchResponseSchema>;
export type EntryProcessKeyResult = z.infer<typeof EntryProcessKeyResultSchema>;

// Event Journal types (Phase 5.04)
export type JournalEventType = z.infer<typeof JournalEventTypeSchema>;
export type JournalEventData = z.infer<typeof JournalEventDataSchema>;
export type JournalSubscribeRequest = z.infer<typeof JournalSubscribeRequestSchema>;
export type JournalUnsubscribeRequest = z.infer<typeof JournalUnsubscribeRequestSchema>;
export type JournalEventMessage = z.infer<typeof JournalEventMessageSchema>;
export type JournalReadRequest = z.infer<typeof JournalReadRequestSchema>;
export type JournalReadResponse = z.infer<typeof JournalReadResponseSchema>;

// Conflict Resolver types (Phase 5.05)
export type ConflictResolver = z.infer<typeof ConflictResolverSchema>;
export type RegisterResolverRequest = z.infer<typeof RegisterResolverRequestSchema>;
export type RegisterResolverResponse = z.infer<typeof RegisterResolverResponseSchema>;
export type UnregisterResolverRequest = z.infer<typeof UnregisterResolverRequestSchema>;
export type UnregisterResolverResponse = z.infer<typeof UnregisterResolverResponseSchema>;
export type MergeRejectedMessage = z.infer<typeof MergeRejectedMessageSchema>;
export type ListResolversRequest = z.infer<typeof ListResolversRequestSchema>;
export type ListResolversResponse = z.infer<typeof ListResolversResponseSchema>;

// Full-Text Search types (Phase 11.1)
export type SearchOptions = z.infer<typeof SearchOptionsSchema>;
export type SearchPayload = z.infer<typeof SearchPayloadSchema>;
export type SearchMessage = z.infer<typeof SearchMessageSchema>;
export type SearchRespPayload = z.infer<typeof SearchRespPayloadSchema>;
export type SearchRespMessage = z.infer<typeof SearchRespMessageSchema>;
