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
