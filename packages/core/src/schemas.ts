import { z } from 'zod';

// --- Basic Types ---

export const TimestampSchema = z.object({
  millis: z.number(),
  counter: z.number(),
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

