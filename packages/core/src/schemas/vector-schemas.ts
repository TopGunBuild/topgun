import { z } from 'zod';
import { PredicateNodeSchema } from './base-schemas';

export const VectorSearchOptionsSchema = z.object({
  includeValue: z.boolean().optional(),
  includeVectors: z.boolean().optional(),
  minScore: z.number().optional(),
  filter: PredicateNodeSchema.optional(),
});
export type VectorSearchOptions = z.infer<typeof VectorSearchOptionsSchema>;

export const VectorSearchPayloadSchema = z.object({
  id: z.string(),
  mapName: z.string(),
  indexName: z.string().optional(),
  // Uint8Array over the wire; MsgPack bin format. Zod's `instanceof(Uint8Array)`
  // matches what msgpackr decodes.
  queryVector: z.instanceof(Uint8Array),
  k: z.number().int().nonnegative(),
  efSearch: z.number().int().nonnegative().optional(),
  options: VectorSearchOptionsSchema.optional(),
});
export type VectorSearchPayload = z.infer<typeof VectorSearchPayloadSchema>;

export const VectorSearchMessageSchema = z.object({
  type: z.literal('VECTOR_SEARCH'),
  payload: VectorSearchPayloadSchema,
});
export type VectorSearchMessage = z.infer<typeof VectorSearchMessageSchema>;

export const VectorSearchResultSchema = z.object({
  key: z.string(),
  score: z.number(),
  value: z.unknown().optional(),
  vector: z.instanceof(Uint8Array).optional(),
});
export type VectorSearchResult = z.infer<typeof VectorSearchResultSchema>;

export const VectorSearchRespPayloadSchema = z.object({
  id: z.string(),
  results: z.array(VectorSearchResultSchema),
  totalCandidates: z.number().int().nonnegative(),
  searchTimeMs: z.number().int().nonnegative(),
  error: z.string().optional(),
});
export type VectorSearchRespPayload = z.infer<typeof VectorSearchRespPayloadSchema>;

export const VectorSearchRespMessageSchema = z.object({
  type: z.literal('VECTOR_SEARCH_RESP'),
  payload: VectorSearchRespPayloadSchema,
});
export type VectorSearchRespMessage = z.infer<typeof VectorSearchRespMessageSchema>;
