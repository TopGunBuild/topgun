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
    fields: z.array(z.string()).optional(),
  }),
});
export type QuerySubMessage = z.infer<typeof QuerySubMessageSchema>;

export const QueryUnsubMessageSchema = z.object({
  type: z.literal('QUERY_UNSUB'),
  payload: z.object({
    queryId: z.string(),
  }),
});
export type QueryUnsubMessage = z.infer<typeof QueryUnsubMessageSchema>;

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
  merkleRootHash: z.number().int().optional(),
});
export type QueryRespPayload = z.infer<typeof QueryRespPayloadSchema>;

export const QueryRespMessageSchema = z.object({
  type: z.literal('QUERY_RESP'),
  payload: QueryRespPayloadSchema,
});
export type QueryRespMessage = z.infer<typeof QueryRespMessageSchema>;

// --- Query Sync Init (Merkle delta reconnect) ---
export const QuerySyncInitPayloadSchema = z.object({
  queryId: z.string(),
  rootHash: z.number().int(),
});
export type QuerySyncInitPayload = z.infer<typeof QuerySyncInitPayloadSchema>;

export const QuerySyncInitMessageSchema = z.object({
  type: z.literal('QUERY_SYNC_INIT'),
  payload: QuerySyncInitPayloadSchema,
});
export type QuerySyncInitMessage = z.infer<typeof QuerySyncInitMessageSchema>;
