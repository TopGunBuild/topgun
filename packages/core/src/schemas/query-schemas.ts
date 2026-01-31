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
