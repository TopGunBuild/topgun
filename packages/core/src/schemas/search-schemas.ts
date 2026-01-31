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
