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
