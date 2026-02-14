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
export type ClientOpMessage = z.infer<typeof ClientOpMessageSchema>;

export const OpBatchMessageSchema = z.object({
  type: z.literal('OP_BATCH'),
  payload: z.object({
    ops: z.array(ClientOpSchema),
    writeConcern: WriteConcernSchema.optional(),
    timeout: z.number().optional(),
  }),
});
export type OpBatchMessage = z.infer<typeof OpBatchMessageSchema>;

// --- LWWMap Sync Messages ---
export const SyncInitMessageSchema = z.object({
  type: z.literal('SYNC_INIT'),
  mapName: z.string(),
  lastSyncTimestamp: z.number().optional(),
});
export type SyncInitMessage = z.infer<typeof SyncInitMessageSchema>;

export const SyncRespRootMessageSchema = z.object({
  type: z.literal('SYNC_RESP_ROOT'),
  payload: z.object({
    mapName: z.string(),
    rootHash: z.number(),
    timestamp: TimestampSchema,
  }),
});
export type SyncRespRootMessage = z.infer<typeof SyncRespRootMessageSchema>;

export const SyncRespBucketsMessageSchema = z.object({
  type: z.literal('SYNC_RESP_BUCKETS'),
  payload: z.object({
    mapName: z.string(),
    path: z.string(),
    buckets: z.record(z.string(), z.number()),
  }),
});
export type SyncRespBucketsMessage = z.infer<typeof SyncRespBucketsMessageSchema>;

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
export type SyncRespLeafMessage = z.infer<typeof SyncRespLeafMessageSchema>;

export const MerkleReqBucketMessageSchema = z.object({
  type: z.literal('MERKLE_REQ_BUCKET'),
  payload: z.object({
    mapName: z.string(),
    path: z.string(),
  }),
});
export type MerkleReqBucketMessage = z.infer<typeof MerkleReqBucketMessageSchema>;

// --- ORMap Shared Types ---

/**
 * Shared entry shape for ORMap sync messages.
 * Used in ORMap leaf responses, diff responses, and push diffs.
 */
export const ORMapEntrySchema = z.object({
  key: z.string(),
  records: z.array(ORMapRecordSchema),
  tombstones: z.array(z.string()),
});
export type ORMapEntry = z.infer<typeof ORMapEntrySchema>;

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
export type ORMapSyncInit = z.infer<typeof ORMapSyncInitSchema>;

export const ORMapSyncRespRootSchema = z.object({
  type: z.literal('ORMAP_SYNC_RESP_ROOT'),
  payload: z.object({
    mapName: z.string(),
    rootHash: z.number(),
    timestamp: TimestampSchema,
  }),
});
export type ORMapSyncRespRoot = z.infer<typeof ORMapSyncRespRootSchema>;

export const ORMapSyncRespBucketsSchema = z.object({
  type: z.literal('ORMAP_SYNC_RESP_BUCKETS'),
  payload: z.object({
    mapName: z.string(),
    path: z.string(),
    buckets: z.record(z.string(), z.number()),
  }),
});
export type ORMapSyncRespBuckets = z.infer<typeof ORMapSyncRespBucketsSchema>;

export const ORMapMerkleReqBucketSchema = z.object({
  type: z.literal('ORMAP_MERKLE_REQ_BUCKET'),
  payload: z.object({
    mapName: z.string(),
    path: z.string(),
  }),
});
export type ORMapMerkleReqBucket = z.infer<typeof ORMapMerkleReqBucketSchema>;

export const ORMapSyncRespLeafSchema = z.object({
  type: z.literal('ORMAP_SYNC_RESP_LEAF'),
  payload: z.object({
    mapName: z.string(),
    path: z.string(),
    entries: z.array(ORMapEntrySchema),
  }),
});
export type ORMapSyncRespLeaf = z.infer<typeof ORMapSyncRespLeafSchema>;

export const ORMapDiffRequestSchema = z.object({
  type: z.literal('ORMAP_DIFF_REQUEST'),
  payload: z.object({
    mapName: z.string(),
    keys: z.array(z.string()),
  }),
});
export type ORMapDiffRequest = z.infer<typeof ORMapDiffRequestSchema>;

export const ORMapDiffResponseSchema = z.object({
  type: z.literal('ORMAP_DIFF_RESPONSE'),
  payload: z.object({
    mapName: z.string(),
    entries: z.array(ORMapEntrySchema),
  }),
});
export type ORMapDiffResponse = z.infer<typeof ORMapDiffResponseSchema>;

export const ORMapPushDiffSchema = z.object({
  type: z.literal('ORMAP_PUSH_DIFF'),
  payload: z.object({
    mapName: z.string(),
    entries: z.array(ORMapEntrySchema),
  }),
});
export type ORMapPushDiff = z.infer<typeof ORMapPushDiffSchema>;

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
