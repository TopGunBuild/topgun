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

