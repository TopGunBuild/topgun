import { z } from 'zod';
import { ChangeEventTypeSchema } from './base-schemas';

export const SearchMethodSchema = z.enum(['exact', 'fullText', 'semantic']);
export type SearchMethodType = z.infer<typeof SearchMethodSchema>;

export const HybridSearchPayloadSchema = z.object({
  requestId: z.string(),
  mapName: z.string(),
  queryText: z.string(),
  methods: z.array(SearchMethodSchema),
  k: z.number(),
  queryVector: z.instanceof(Uint8Array).optional(),
  predicate: z.unknown().optional(),
  includeValue: z.boolean().optional(),
  minScore: z.number().optional(),
});
export type HybridSearchPayload = z.infer<typeof HybridSearchPayloadSchema>;

export const HybridSearchMessageSchema = z.object({
  type: z.literal('HYBRID_SEARCH'),
  payload: HybridSearchPayloadSchema,
});
export type HybridSearchMessage = z.infer<typeof HybridSearchMessageSchema>;

export const HybridSearchResultEntrySchema = z.object({
  key: z.string(),
  score: z.number(),
  methodScores: z.record(SearchMethodSchema, z.number().optional()),
  value: z.unknown().optional(),
});
export type HybridSearchResultEntry = z.infer<typeof HybridSearchResultEntrySchema>;

export const HybridSearchRespPayloadSchema = z.object({
  requestId: z.string(),
  results: z.array(HybridSearchResultEntrySchema),
  searchTimeMs: z.number(),
  error: z.string().optional(),
});
export type HybridSearchRespPayload = z.infer<typeof HybridSearchRespPayloadSchema>;

export const HybridSearchRespMessageSchema = z.object({
  type: z.literal('HYBRID_SEARCH_RESP'),
  payload: HybridSearchRespPayloadSchema,
});
export type HybridSearchRespMessage = z.infer<typeof HybridSearchRespMessageSchema>;

export const HybridSearchSubPayloadSchema = z.object({
  subscriptionId: z.string(),
  mapName: z.string(),
  queryText: z.string(),
  methods: z.array(SearchMethodSchema),
  k: z.number(),
  queryVector: z.instanceof(Uint8Array).optional(),
  predicate: z.unknown().optional(),
  includeValue: z.boolean().optional(),
  minScore: z.number().optional(),
});
export type HybridSearchSubPayload = z.infer<typeof HybridSearchSubPayloadSchema>;

export const HybridSearchSubMessageSchema = z.object({
  type: z.literal('HYBRID_SEARCH_SUB'),
  payload: HybridSearchSubPayloadSchema,
});

export const HybridSearchUpdatePayloadSchema = z.object({
  subscriptionId: z.string(),
  key: z.string(),
  score: z.number(),
  methodScores: z.record(SearchMethodSchema, z.number().optional()),
  value: z.unknown().optional(),
  changeType: ChangeEventTypeSchema,
});
export type HybridSearchUpdatePayload = z.infer<typeof HybridSearchUpdatePayloadSchema>;

export const HybridSearchUpdateMessageSchema = z.object({
  type: z.literal('HYBRID_SEARCH_UPDATE'),
  payload: HybridSearchUpdatePayloadSchema,
});

export const HybridSearchUnsubPayloadSchema = z.object({
  subscriptionId: z.string(),
});
export type HybridSearchUnsubPayload = z.infer<typeof HybridSearchUnsubPayloadSchema>;

export const HybridSearchUnsubMessageSchema = z.object({
  type: z.literal('HYBRID_SEARCH_UNSUB'),
  payload: HybridSearchUnsubPayloadSchema,
});
