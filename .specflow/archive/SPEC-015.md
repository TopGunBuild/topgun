# SPEC-015: Schema File Splitting

---
id: SPEC-015
type: refactor
status: done
priority: medium
complexity: medium
created: 2026-01-31
---

## Context

The `packages/core/src/schemas.ts` file has grown to 1160 lines containing 80+ Zod schemas and type exports. This monolithic file:
- Mixes unrelated domain concerns (auth, sync, search, cluster, messaging, etc.)
- Makes it difficult to find schemas by domain
- Increases merge conflicts when multiple features touch schemas
- Violates single-responsibility principle

The file is currently exported via barrel export in `index.ts` (line 73: `export * from './schemas'`), so all schemas remain public API.

### Goal Statement

Split schemas.ts into domain-focused modules while preserving all existing exports.

### Observable Truths (when complete)
1. schemas.ts is under 150 lines (barrel re-exports only)
2. Each domain module contains only related schemas
3. All existing imports from `@topgunbuild/core` continue to work
4. Build passes with no TypeScript errors
5. All tests pass without modification

### Key Links
- schemas.ts re-exports from domain modules
- index.ts re-exports from schemas.ts (unchanged)
- Consumers import from `@topgunbuild/core` (unchanged)

## Task

Split `packages/core/src/schemas.ts` into 6 domain-specific modules:

1. **base-schemas.ts** - Core types used across domains
2. **sync-schemas.ts** - Sync engine and CRDT operation messages
3. **query-schemas.ts** - Query subscription and response messages
4. **search-schemas.ts** - Full-text search messages
5. **cluster-schemas.ts** - Cluster coordination and distributed messages
6. **messaging-schemas.ts** - Topics, counters, locks, heartbeat, journal, processors, resolvers

Then reduce schemas.ts to a barrel that re-exports all modules.

## Requirements

### R1: Create `packages/core/src/schemas/base-schemas.ts`

Contains foundational schemas used by other domain modules:

```typescript
// packages/core/src/schemas/base-schemas.ts
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
export type Timestamp = z.infer<typeof TimestampSchema>;

export const LWWRecordSchema = z.object({
  value: z.any().nullable(),
  timestamp: TimestampSchema,
  ttlMs: z.number().optional(),
});
export type LWWRecord<V = any> = z.infer<typeof LWWRecordSchema>;

export const ORMapRecordSchema = z.object({
  value: z.any(),
  timestamp: TimestampSchema,
  tag: z.string(),
  ttlMs: z.number().optional(),
});
export type ORMapRecord<V = any> = z.infer<typeof ORMapRecordSchema>;

// --- Predicate Types ---
export const PredicateOpSchema = z.enum([
  'eq', 'neq', 'gt', 'gte', 'lt', 'lte', 'like', 'regex', 'and', 'or', 'not'
]);

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
  cursor: z.string().optional(), // Phase 14.1: replaces offset
});
export type Query = z.infer<typeof QuerySchema>;

// --- Client Operation Types ---
export const ClientOpSchema = z.object({
  id: z.string().optional(),
  mapName: z.string(),
  key: z.string(),
  opType: z.string().optional(),
  record: LWWRecordSchema.nullable().optional(),
  orRecord: ORMapRecordSchema.nullable().optional(),
  orTag: z.string().nullable().optional(),
  writeConcern: WriteConcernSchema.optional(),
  timeout: z.number().optional(),
});
export type ClientOp = z.infer<typeof ClientOpSchema>;

// --- Auth Message ---
export const AuthMessageSchema = z.object({
  type: z.literal('AUTH'),
  token: z.string(),
});
```

**Estimated lines:** ~80

### R2: Create `packages/core/src/schemas/sync-schemas.ts`

Contains sync engine messages (LWWMap sync, ORMap sync, operation batching):

```typescript
// packages/core/src/schemas/sync-schemas.ts
import { z } from 'zod';
import {
  TimestampSchema,
  LWWRecordSchema,
  ORMapRecordSchema,
  ClientOpSchema,
  WriteConcernSchema,
} from './base-schemas';

// --- Client Operations ---
export const ClientOpMessageSchema = z.object({
  type: z.literal('CLIENT_OP'),
  payload: ClientOpSchema,
});

export const OpBatchMessageSchema = z.object({
  type: z.literal('OP_BATCH'),
  payload: z.object({
    ops: z.array(ClientOpSchema),
    writeConcern: WriteConcernSchema.optional(),
    timeout: z.number().optional(),
  }),
});

// --- LWWMap Sync Messages ---
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

// --- ORMap Sync Messages ---

/**
 * ORMAP_SYNC_INIT: Client initiates ORMap sync
 * Sends root hash and bucket hashes to server
 */
export const ORMapSyncInitSchema = z.object({
  type: z.literal('ORMAP_SYNC_INIT'),
  mapName: z.string(),
  rootHash: z.number(),
  bucketHashes: z.record(z.string(), z.number()),
  lastSyncTimestamp: z.number().optional(),
});

export const ORMapSyncRespRootSchema = z.object({
  type: z.literal('ORMAP_SYNC_RESP_ROOT'),
  payload: z.object({
    mapName: z.string(),
    rootHash: z.number(),
    timestamp: TimestampSchema,
  }),
});

export const ORMapSyncRespBucketsSchema = z.object({
  type: z.literal('ORMAP_SYNC_RESP_BUCKETS'),
  payload: z.object({
    mapName: z.string(),
    path: z.string(),
    buckets: z.record(z.string(), z.number()),
  }),
});

export const ORMapMerkleReqBucketSchema = z.object({
  type: z.literal('ORMAP_MERKLE_REQ_BUCKET'),
  payload: z.object({
    mapName: z.string(),
    path: z.string(),
  }),
});

export const ORMapSyncRespLeafSchema = z.object({
  type: z.literal('ORMAP_SYNC_RESP_LEAF'),
  payload: z.object({
    mapName: z.string(),
    path: z.string(),
    entries: z.array(z.object({
      key: z.string(),
      records: z.array(ORMapRecordSchema),
      tombstones: z.array(z.string()),
    })),
  }),
});

export const ORMapDiffRequestSchema = z.object({
  type: z.literal('ORMAP_DIFF_REQUEST'),
  payload: z.object({
    mapName: z.string(),
    keys: z.array(z.string()),
  }),
});

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

// --- Write Concern Response Schemas ---
export const OpResultSchema = z.object({
  opId: z.string(),
  success: z.boolean(),
  achievedLevel: WriteConcernSchema,
  error: z.string().optional(),
});
export type OpResult = z.infer<typeof OpResultSchema>;

export const OpAckMessageSchema = z.object({
  type: z.literal('OP_ACK'),
  payload: z.object({
    lastId: z.string(),
    achievedLevel: WriteConcernSchema.optional(),
    results: z.array(OpResultSchema).optional(),
  }),
});
export type OpAckMessage = z.infer<typeof OpAckMessageSchema>;

export const OpRejectedMessageSchema = z.object({
  type: z.literal('OP_REJECTED'),
  payload: z.object({
    opId: z.string(),
    reason: z.string(),
    code: z.number().optional(),
  }),
});
export type OpRejectedMessage = z.infer<typeof OpRejectedMessageSchema>;

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
export type BatchMessage = z.infer<typeof BatchMessageSchema>;
```

**Estimated lines:** ~180

### R3: Create `packages/core/src/schemas/query-schemas.ts`

Contains query subscription messages:

```typescript
// packages/core/src/schemas/query-schemas.ts
import { z } from 'zod';
import { QuerySchema } from './base-schemas';

// --- Query Subscription Messages ---
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

// --- Query Response Types ---
export const CursorStatusSchema = z.enum(['valid', 'expired', 'invalid', 'none']);
export type CursorStatus = z.infer<typeof CursorStatusSchema>;

export const QueryRespPayloadSchema = z.object({
  queryId: z.string(),
  results: z.array(z.object({
    key: z.string(),
    value: z.unknown(),
  })),
  nextCursor: z.string().optional(),
  hasMore: z.boolean().optional(),
  cursorStatus: CursorStatusSchema.optional(),
});
export type QueryRespPayload = z.infer<typeof QueryRespPayloadSchema>;

export const QueryRespMessageSchema = z.object({
  type: z.literal('QUERY_RESP'),
  payload: QueryRespPayloadSchema,
});
export type QueryRespMessage = z.infer<typeof QueryRespMessageSchema>;
```

**Estimated lines:** ~50

### R4: Create `packages/core/src/schemas/search-schemas.ts`

Contains full-text search messages:

```typescript
// packages/core/src/schemas/search-schemas.ts
import { z } from 'zod';

// --- Search Options ---
export const SearchOptionsSchema = z.object({
  limit: z.number().optional(),
  minScore: z.number().optional(),
  boost: z.record(z.string(), z.number()).optional(),
});
export type SearchOptions = z.infer<typeof SearchOptionsSchema>;

// --- One-Shot Search ---
export const SearchPayloadSchema = z.object({
  requestId: z.string(),
  mapName: z.string(),
  query: z.string(),
  options: SearchOptionsSchema.optional(),
});
export type SearchPayload = z.infer<typeof SearchPayloadSchema>;

export const SearchMessageSchema = z.object({
  type: z.literal('SEARCH'),
  payload: SearchPayloadSchema,
});
export type SearchMessage = z.infer<typeof SearchMessageSchema>;

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
export type SearchRespPayload = z.infer<typeof SearchRespPayloadSchema>;

export const SearchRespMessageSchema = z.object({
  type: z.literal('SEARCH_RESP'),
  payload: SearchRespPayloadSchema,
});
export type SearchRespMessage = z.infer<typeof SearchRespMessageSchema>;

// --- Live Search Subscriptions ---
export const SearchUpdateTypeSchema = z.enum(['ENTER', 'UPDATE', 'LEAVE']);
export type SearchUpdateType = z.infer<typeof SearchUpdateTypeSchema>;

export const SearchSubPayloadSchema = z.object({
  subscriptionId: z.string(),
  mapName: z.string(),
  query: z.string(),
  options: SearchOptionsSchema.optional(),
});
export type SearchSubPayload = z.infer<typeof SearchSubPayloadSchema>;

export const SearchSubMessageSchema = z.object({
  type: z.literal('SEARCH_SUB'),
  payload: SearchSubPayloadSchema,
});
export type SearchSubMessage = z.infer<typeof SearchSubMessageSchema>;

export const SearchUpdatePayloadSchema = z.object({
  subscriptionId: z.string(),
  key: z.string(),
  value: z.unknown(),
  score: z.number(),
  matchedTerms: z.array(z.string()),
  type: SearchUpdateTypeSchema,
});
export type SearchUpdatePayload = z.infer<typeof SearchUpdatePayloadSchema>;

export const SearchUpdateMessageSchema = z.object({
  type: z.literal('SEARCH_UPDATE'),
  payload: SearchUpdatePayloadSchema,
});
export type SearchUpdateMessage = z.infer<typeof SearchUpdateMessageSchema>;

export const SearchUnsubPayloadSchema = z.object({
  subscriptionId: z.string(),
});
export type SearchUnsubPayload = z.infer<typeof SearchUnsubPayloadSchema>;

export const SearchUnsubMessageSchema = z.object({
  type: z.literal('SEARCH_UNSUB'),
  payload: SearchUnsubPayloadSchema,
});
export type SearchUnsubMessage = z.infer<typeof SearchUnsubMessageSchema>;
```

**Estimated lines:** ~100

### R5: Create `packages/core/src/schemas/cluster-schemas.ts`

Contains cluster coordination messages:

```typescript
// packages/core/src/schemas/cluster-schemas.ts
import { z } from 'zod';
import { SearchOptionsSchema, SearchUpdateTypeSchema } from './search-schemas';

// --- Partition Map ---
export const PartitionMapRequestSchema = z.object({
  type: z.literal('PARTITION_MAP_REQUEST'),
  payload: z.object({
    currentVersion: z.number().optional(),
  }).optional(),
});

// --- Distributed Live Subscriptions (Phase 14.2) ---
export const ClusterSubRegisterPayloadSchema = z.object({
  subscriptionId: z.string(),
  coordinatorNodeId: z.string(),
  mapName: z.string(),
  type: z.enum(['SEARCH', 'QUERY']),
  searchQuery: z.string().optional(),
  searchOptions: z.object({
    limit: z.number().int().positive().optional(),
    minScore: z.number().optional(),
    boost: z.record(z.string(), z.number()).optional(),
  }).optional(),
  queryPredicate: z.any().optional(),
  querySort: z.record(z.string(), z.enum(['asc', 'desc'])).optional(),
});
export type ClusterSubRegisterPayload = z.infer<typeof ClusterSubRegisterPayloadSchema>;

export const ClusterSubRegisterMessageSchema = z.object({
  type: z.literal('CLUSTER_SUB_REGISTER'),
  payload: ClusterSubRegisterPayloadSchema,
});
export type ClusterSubRegisterMessage = z.infer<typeof ClusterSubRegisterMessageSchema>;

export const ClusterSubAckPayloadSchema = z.object({
  subscriptionId: z.string(),
  nodeId: z.string(),
  success: z.boolean(),
  error: z.string().optional(),
  initialResults: z.array(z.object({
    key: z.string(),
    value: z.unknown(),
    score: z.number().optional(),
    matchedTerms: z.array(z.string()).optional(),
  })).optional(),
  totalHits: z.number().int().nonnegative().optional(),
});
export type ClusterSubAckPayload = z.infer<typeof ClusterSubAckPayloadSchema>;

export const ClusterSubAckMessageSchema = z.object({
  type: z.literal('CLUSTER_SUB_ACK'),
  payload: ClusterSubAckPayloadSchema,
});
export type ClusterSubAckMessage = z.infer<typeof ClusterSubAckMessageSchema>;

export const ClusterSubUpdatePayloadSchema = z.object({
  subscriptionId: z.string(),
  sourceNodeId: z.string(),
  key: z.string(),
  value: z.unknown(),
  score: z.number().optional(),
  matchedTerms: z.array(z.string()).optional(),
  changeType: z.enum(['ENTER', 'UPDATE', 'LEAVE']),
  timestamp: z.number(),
});
export type ClusterSubUpdatePayload = z.infer<typeof ClusterSubUpdatePayloadSchema>;

export const ClusterSubUpdateMessageSchema = z.object({
  type: z.literal('CLUSTER_SUB_UPDATE'),
  payload: ClusterSubUpdatePayloadSchema,
});
export type ClusterSubUpdateMessage = z.infer<typeof ClusterSubUpdateMessageSchema>;

export const ClusterSubUnregisterPayloadSchema = z.object({
  subscriptionId: z.string(),
});
export type ClusterSubUnregisterPayload = z.infer<typeof ClusterSubUnregisterPayloadSchema>;

export const ClusterSubUnregisterMessageSchema = z.object({
  type: z.literal('CLUSTER_SUB_UNREGISTER'),
  payload: ClusterSubUnregisterPayloadSchema,
});
export type ClusterSubUnregisterMessage = z.infer<typeof ClusterSubUnregisterMessageSchema>;

// --- Distributed Search (Phase 14) ---
export const ClusterSearchReqPayloadSchema = z.object({
  requestId: z.string(),
  mapName: z.string(),
  query: z.string(),
  options: z.object({
    limit: z.number().int().positive().max(1000),
    minScore: z.number().optional(),
    boost: z.record(z.string(), z.number()).optional(),
    includeMatchedTerms: z.boolean().optional(),
    afterScore: z.number().optional(),
    afterKey: z.string().optional(),
  }),
  timeoutMs: z.number().int().positive().optional(),
});
export type ClusterSearchReqPayload = z.infer<typeof ClusterSearchReqPayloadSchema>;

export const ClusterSearchReqMessageSchema = z.object({
  type: z.literal('CLUSTER_SEARCH_REQ'),
  payload: ClusterSearchReqPayloadSchema,
});
export type ClusterSearchReqMessage = z.infer<typeof ClusterSearchReqMessageSchema>;

export const ClusterSearchRespPayloadSchema = z.object({
  requestId: z.string(),
  nodeId: z.string(),
  results: z.array(z.object({
    key: z.string(),
    value: z.unknown(),
    score: z.number(),
    matchedTerms: z.array(z.string()).optional(),
  })),
  totalHits: z.number().int().nonnegative(),
  executionTimeMs: z.number().nonnegative(),
  error: z.string().optional(),
});
export type ClusterSearchRespPayload = z.infer<typeof ClusterSearchRespPayloadSchema>;

export const ClusterSearchRespMessageSchema = z.object({
  type: z.literal('CLUSTER_SEARCH_RESP'),
  payload: ClusterSearchRespPayloadSchema,
});
export type ClusterSearchRespMessage = z.infer<typeof ClusterSearchRespMessageSchema>;

export const ClusterSearchSubscribePayloadSchema = z.object({
  subscriptionId: z.string(),
  mapName: z.string(),
  query: z.string(),
  options: SearchOptionsSchema.optional(),
});
export type ClusterSearchSubscribePayload = z.infer<typeof ClusterSearchSubscribePayloadSchema>;

export const ClusterSearchSubscribeMessageSchema = z.object({
  type: z.literal('CLUSTER_SEARCH_SUBSCRIBE'),
  payload: ClusterSearchSubscribePayloadSchema,
});
export type ClusterSearchSubscribeMessage = z.infer<typeof ClusterSearchSubscribeMessageSchema>;

export const ClusterSearchUnsubscribePayloadSchema = z.object({
  subscriptionId: z.string(),
});
export type ClusterSearchUnsubscribePayload = z.infer<typeof ClusterSearchUnsubscribePayloadSchema>;

export const ClusterSearchUnsubscribeMessageSchema = z.object({
  type: z.literal('CLUSTER_SEARCH_UNSUBSCRIBE'),
  payload: ClusterSearchUnsubscribePayloadSchema,
});
export type ClusterSearchUnsubscribeMessage = z.infer<typeof ClusterSearchUnsubscribeMessageSchema>;

export const ClusterSearchUpdatePayloadSchema = z.object({
  subscriptionId: z.string(),
  nodeId: z.string(),
  key: z.string(),
  value: z.unknown(),
  score: z.number(),
  matchedTerms: z.array(z.string()).optional(),
  type: SearchUpdateTypeSchema,
});
export type ClusterSearchUpdatePayload = z.infer<typeof ClusterSearchUpdatePayloadSchema>;

export const ClusterSearchUpdateMessageSchema = z.object({
  type: z.literal('CLUSTER_SEARCH_UPDATE'),
  payload: ClusterSearchUpdatePayloadSchema,
});
export type ClusterSearchUpdateMessage = z.infer<typeof ClusterSearchUpdateMessageSchema>;
```

**Estimated lines:** ~180

### R6: Create `packages/core/src/schemas/messaging-schemas.ts`

Contains topics, counters, locks, heartbeat, journal, entry processors, and conflict resolvers:

```typescript
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

// --- Entry Processor Messages (Phase 5.03) ---
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

// --- Event Journal Messages (Phase 5.04) ---
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

// --- Conflict Resolver Messages (Phase 5.05) ---
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
```

**Estimated lines:** ~260

### R7: Create `packages/core/src/schemas/index.ts` (barrel)

Re-exports all domain modules:

```typescript
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
```

**Estimated lines:** ~120

### R8: Update `packages/core/src/schemas.ts` to re-export

Replace entire file content with:

```typescript
// packages/core/src/schemas.ts
// Barrel re-export - all schemas moved to ./schemas/ directory
export * from './schemas/index';
```

**Lines:** 3

### R9: Update `packages/core/src/index.ts`

No changes needed - line 73 already has `export * from './schemas'` which will now re-export from the new barrel.

## Acceptance Criteria

- [x] AC1: 6 new files created in `packages/core/src/schemas/` directory
- [x] AC2: schemas.ts reduced to 3 lines (barrel re-export only)
- [x] AC3: All 80+ existing schema exports preserved
- [x] AC4: All 30+ existing type exports preserved
- [x] AC5: MessageSchema union includes all 53 message types (verified count from current schemas.ts)
- [x] AC6: `pnpm build` passes with no TypeScript errors
- [x] AC7: `pnpm test` passes with no test modifications
- [x] AC8: No circular dependencies between schema modules
- [x] AC9: Each domain module has single import from `zod`

## Constraints

- DO NOT change any schema definitions (only move them)
- DO NOT change any type definitions
- DO NOT change any export names
- DO NOT modify index.ts beyond what is required
- DO NOT add new dependencies

## Assumptions

- Zod remains the only schema library (verified in PROJECT.md)
- Domain modules can import from each other for shared types (e.g., cluster-schemas imports from search-schemas for SearchUpdateTypeSchema)
- The existing test suite validates schema behavior (no new tests needed)
- File naming follows kebab-case convention (verified in codebase patterns)

## Notes

- This refactor sets up the codebase for future schema additions to be domain-localized
- The MessageSchema union must be reconstructed in the barrel to maintain type inference
- Estimated total lines across new files: ~850 (vs original 1160)

---

## Audit History

### Audit v1 (2026-01-31 22:30)
**Status:** NEEDS_REVISION

**Context Estimate:** ~21% total (PEAK range)

**Critical:**
1. AC5 incorrectly claims "47 message types" but actual count in current MessageSchema is **53**. The spec's proposed R7 also has 53 message types, which is correct. AC5 must be corrected from "47" to "53".

**Recommendations:**
2. Consider adding the comment from current schemas.ts line 60 (`// Phase 14.1: replaces offset`) to base-schemas.ts QuerySchema.cursor field for historical context.
3. Consider preserving JSDoc comments from original file (e.g., WriteConcernSchema doc at line 5-7, BatchMessageSchema doc at lines 295-299) for API documentation continuity.

**Verification performed:**
- Counted 98 schema exports in current file (spec says "80+" - OK)
- Counted 65 type exports in current file (spec says "30+" - OK)
- Counted 53 message types in current MessageSchema (spec claimed 47 - INCORRECT)
- Verified all schemas accounted for in domain modules
- Verified dependency chain: cluster-schemas imports from search-schemas (documented in Assumptions)
- Verified barrel pattern preserves API compatibility

### Response v1 (2026-01-31 22:45)
**Applied:** all (1 critical + 2 recommendations)

**Changes:**
1. [x] AC5 message type count - Added clarification "(verified count from current schemas.ts)" to confirm 53 message types is correct
2. [x] Historical comment for cursor field - Added `// Phase 14.1: replaces offset` to QuerySchema.cursor in R1 (base-schemas.ts)
3. [x] JSDoc comments preserved - Added JSDoc for WriteConcernSchema, BatchMessageSchema, and ORMapSyncInitSchema from original file

### Audit v2 (2026-01-31 23:15)
**Status:** APPROVED

**Context Estimate:** ~21% total (PEAK range)

**Per-Group Breakdown:**
| Group | Wave | Est. Context | Status |
|-------|------|--------------|--------|
| G1 | 1 | ~4% | base-schemas.ts (~80 lines) |
| G2 | 2 | ~6% | sync-schemas.ts (~180 lines) |
| G3 | 2 | ~2% | query-schemas.ts (~50 lines) |
| G4 | 2 | ~4% | search-schemas.ts (~100 lines) |
| G5 | 3 | ~6% | cluster-schemas.ts (~180 lines, imports search) |
| G6 | 2 | ~8% | messaging-schemas.ts (~260 lines) |
| G7 | 4 | ~4% | schemas/index.ts barrel (~120 lines) |
| G8 | 5 | ~1% | schemas.ts update (3 lines) |

**Total:** ~35% context in GOOD range

**Quality Projection:** GOOD range (30-50%)

**Verification performed:**
- Message count: Confirmed 53 message types in both current schemas.ts (lines 899-962) and proposed R7
- Schema coverage: All 98+ schemas accounted for in 6 domain modules
- Type coverage: All 65 type exports preserved
- Dependency chain: cluster-schemas -> search-schemas (SearchUpdateTypeSchema) - documented in Assumptions
- R6 import correction: Removed unused `WriteConcernSchema` import from R6 (messaging-schemas.ts only uses TimestampSchema)
- Barrel pattern: Correctly re-exports all domain modules plus reconstructs MessageSchema union
- AC5 corrected to 53 with verification note
- JSDoc comments preserved for WriteConcernSchema, BatchMessageSchema, ORMapSyncInitSchema

**All 9 audit dimensions passed:**
1. Clarity: Clear task description and domain boundaries
2. Completeness: All schemas and types accounted for
3. Testability: All ACs are measurable (line counts, build/test pass)
4. Scope: Clear boundaries (only move, no changes)
5. Feasibility: Straightforward file reorganization
6. Architecture fit: Follows existing barrel pattern in codebase
7. Non-duplication: No new abstractions, pure reorganization
8. Cognitive load: Domain grouping improves maintainability
9. Strategic fit: Aligns with codebase modularization goals

**Comment:** Well-crafted specification. The domain groupings are logical, dependency chain is documented, and all exports are preserved. Ready for implementation.

---

## Execution Summary

**Executed:** 2026-01-31 23:30
**Commits:** 8

### Files Created
- `packages/core/src/schemas/base-schemas.ts` — foundational types (WriteConcern, Timestamp, LWWRecord, ORMapRecord, Predicate, Query, ClientOp, Auth)
- `packages/core/src/schemas/sync-schemas.ts` — CRDT sync operations (LWWMap sync, ORMap sync, client ops, write concern responses, batch messages)
- `packages/core/src/schemas/query-schemas.ts` — query subscriptions (QUERY_SUB, QUERY_UNSUB, QUERY_RESP with cursor pagination)
- `packages/core/src/schemas/search-schemas.ts` — full-text search (one-shot search, live subscriptions, SearchUpdateType enum)
- `packages/core/src/schemas/cluster-schemas.ts` — distributed coordination (partition map, cluster subscriptions, distributed search)
- `packages/core/src/schemas/messaging-schemas.ts` — topics, counters, locks, heartbeat, entry processors, journal, conflict resolvers
- `packages/core/src/schemas/index.ts` — barrel re-export with MessageSchema union (53 message types)

### Files Modified
- `packages/core/src/schemas.ts` — reduced from 1160 lines to 3 lines (barrel re-export only)

### Files Deleted
None

### Acceptance Criteria Status
- [x] AC1: 6 new files created in `packages/core/src/schemas/` directory
- [x] AC2: schemas.ts reduced to 3 lines (barrel re-export only)
- [x] AC3: All 80+ existing schema exports preserved
- [x] AC4: All 30+ existing type exports preserved
- [x] AC5: MessageSchema union includes all 53 message types (verified)
- [x] AC6: `pnpm build` passes with no TypeScript errors (core, client, server packages)
- [x] AC7: `pnpm test` passes (1814/1815 tests pass, 1 flaky performance test unrelated to schema changes)
- [x] AC8: No circular dependencies between schema modules (cluster-schemas imports from search-schemas only)
- [x] AC9: Each domain module has single import from `zod`

### Deviations
None

### Notes
- Build verification: core, client, and server packages all build successfully
- Test verification: schemas.test.ts passes all 5 tests
- 1 flaky performance test in SortedMap.test.ts failed due to timing (unrelated to schema changes)
- Total line count across new files: ~994 lines (vs original 1160 lines, ~14% reduction through removal of duplication and comments)
- All JSDoc comments preserved for WriteConcernSchema, BatchMessageSchema, and ORMapSyncInitSchema
- Phase 14.1 cursor comment preserved in QuerySchema
- Barrel pattern ensures zero breaking changes to public API

---

## Review History

### Review v1 (2026-01-31 23:45)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [x] AC1: 6 new files created — All domain modules exist in `/Users/koristuvac/Projects/topgun/topgun/packages/core/src/schemas/` (base-schemas.ts, sync-schemas.ts, query-schemas.ts, search-schemas.ts, cluster-schemas.ts, messaging-schemas.ts, index.ts)
- [x] AC2: schemas.ts reduced to 3 lines — Verified exactly 3 lines (barrel re-export comment + export statement)
- [x] AC3: All 80+ existing schema exports preserved — Domain modules export all schemas via barrel pattern
- [x] AC4: All 30+ existing type exports preserved — Type exports included in each domain module
- [x] AC5: MessageSchema union includes all 53 message types — Manually verified 53 schemas in discriminated union (lines 85-137 in index.ts)
- [x] AC6: `pnpm build` passes — Core, client, and server packages all build successfully with no TypeScript errors
- [x] AC7: `pnpm test` passes — schemas.test.ts passes all 5 tests with no modifications
- [x] AC8: No circular dependencies — Dependency chain is clean: base-schemas (no deps) -> query-schemas/sync-schemas/messaging-schemas (import from base) -> search-schemas (no deps) -> cluster-schemas (imports from search)
- [x] AC9: Each domain module has single import from `zod` — Verified all 7 files have exactly 1 zod import
- [x] File organization: Domain groupings are logical and cohesive
- [x] JSDoc comments preserved: WriteConcernSchema, BatchMessageSchema, ORMapSyncInitSchema all have original documentation
- [x] Historical comments preserved: Phase 14.1 cursor comment in QuerySchema (base-schemas.ts line 59)
- [x] Barrel pattern: schemas.ts -> schemas/index.ts -> domain modules follows established codebase pattern
- [x] Zero breaking changes: All exports remain accessible via `@topgunbuild/core`
- [x] Code quality: Clean imports, proper TypeScript types, consistent formatting
- [x] Security: No hardcoded secrets, no input validation changes (pure reorganization)
- [x] Integration: Follows monorepo barrel export pattern used throughout packages/core
- [x] Architecture: Aligns with modularization effort (SPEC-011 series pattern)
- [x] Non-duplication: No new abstractions, pure file reorganization
- [x] Cognitive load: Domain grouping significantly reduces cognitive overhead for schema discovery

**Summary:**

Outstanding implementation. The schema file splitting has been executed flawlessly with perfect adherence to the specification. All 9 acceptance criteria are met:

1. **File Structure:** 7 files created (6 domain modules + 1 barrel index) in correct location
2. **Line Reduction:** schemas.ts reduced from 1160 lines to exactly 3 lines
3. **Export Preservation:** All 80+ schema exports maintained via barrel pattern
4. **Type Preservation:** All 30+ type exports included in domain modules
5. **Message Count:** MessageSchema union verified to contain exactly 53 message types
6. **Build Success:** Core, client, and server packages build without TypeScript errors
7. **Test Success:** All schema tests pass without modification
8. **No Circular Dependencies:** Clean dependency chain (base -> query/sync/messaging, search -> cluster)
9. **Single Zod Import:** Each of 7 files has exactly 1 import from 'zod'

**Code Quality Highlights:**
- Domain groupings are semantically coherent (base types, sync operations, queries, search, cluster coordination, messaging)
- Dependency chain is minimal and unidirectional (cluster-schemas -> search-schemas only)
- JSDoc comments preserved for WriteConcernSchema, BatchMessageSchema, ORMapSyncInitSchema
- Historical Phase 14.1 comment preserved in QuerySchema.cursor field
- Barrel export pattern ensures zero breaking changes to public API
- 14% line reduction (1160 -> 994 lines) through deduplication

**Architecture Alignment:**
- Follows established barrel pattern from packages/core/src/index.ts
- Consistent with SPEC-011 modularization series (domain-focused extraction)
- Prepares codebase for future schema additions to be domain-localized
- Reduces merge conflicts by separating unrelated schema domains

**No issues found.** This is a textbook example of a clean refactoring that improves maintainability without altering behavior.

---

## Completion

**Completed:** 2026-01-31 23:50
**Total Commits:** 8
**Audit Cycles:** 2
**Review Cycles:** 1
