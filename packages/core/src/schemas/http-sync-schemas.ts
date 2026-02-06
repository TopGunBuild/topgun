// packages/core/src/schemas/http-sync-schemas.ts
// HTTP sync request/response schemas for stateless serverless environments
import { z } from 'zod';
import {
  TimestampSchema,
  LWWRecordSchema,
  ClientOpSchema,
} from './base-schemas';
import { OpResultSchema } from './sync-schemas';

// --- HTTP Sync Request ---

/**
 * Schema for individual sync map entries, specifying which maps the client
 * wants deltas for and the last known sync HLC timestamp for each.
 */
export const SyncMapEntrySchema = z.object({
  mapName: z.string(),
  lastSyncTimestamp: TimestampSchema,
});

/**
 * Schema for one-shot query requests over HTTP.
 */
export const HttpQueryRequestSchema = z.object({
  queryId: z.string(),
  mapName: z.string(),
  filter: z.any(),
  limit: z.number().optional(),
  offset: z.number().optional(),
});

/**
 * Schema for one-shot search requests over HTTP.
 */
export const HttpSearchRequestSchema = z.object({
  searchId: z.string(),
  mapName: z.string(),
  query: z.string(),
  options: z.any().optional(),
});

/**
 * HTTP sync request body sent by the client as POST /sync.
 * Contains all context needed for a stateless request: client identity,
 * HLC state, operations to push, maps to pull deltas for, and queries/searches.
 */
export const HttpSyncRequestSchema = z.object({
  // Client identification
  clientId: z.string(),
  // Client's current HLC for causality tracking
  clientHlc: TimestampSchema,
  // Batch of operations to push (optional)
  operations: z.array(ClientOpSchema).optional(),
  // Maps the client wants deltas for, with their last known sync HLC timestamp
  syncMaps: z.array(SyncMapEntrySchema).optional(),
  // One-shot queries to execute (optional)
  queries: z.array(HttpQueryRequestSchema).optional(),
  // One-shot search requests (optional)
  searches: z.array(HttpSearchRequestSchema).optional(),
});

export type HttpSyncRequest = z.infer<typeof HttpSyncRequestSchema>;

// --- HTTP Sync Response ---

/**
 * Delta record for a single key within a map.
 */
export const DeltaRecordSchema = z.object({
  key: z.string(),
  record: LWWRecordSchema,
  eventType: z.enum(['PUT', 'REMOVE']),
});

/**
 * Delta records for a specific map, containing all new/changed records
 * since the client's lastSyncTimestamp.
 */
export const MapDeltaSchema = z.object({
  mapName: z.string(),
  records: z.array(DeltaRecordSchema),
  serverSyncTimestamp: TimestampSchema,
});

/**
 * Query result for a one-shot HTTP query.
 */
export const HttpQueryResultSchema = z.object({
  queryId: z.string(),
  results: z.array(z.any()),
  hasMore: z.boolean().optional(),
  nextCursor: z.string().optional(),
});

/**
 * Search result for a one-shot HTTP search.
 */
export const HttpSearchResultSchema = z.object({
  searchId: z.string(),
  results: z.array(z.any()),
  totalCount: z.number().optional(),
});

/**
 * Error entry for individual operation failures.
 */
export const HttpSyncErrorSchema = z.object({
  code: z.number(),
  message: z.string(),
  context: z.string().optional(),
});

/**
 * HTTP sync response returned by the server for POST /sync.
 * Contains operation acknowledgments, delta records, query/search results,
 * and the server's current HLC for the client to use in subsequent requests.
 */
export const HttpSyncResponseSchema = z.object({
  // Server's current HLC
  serverHlc: TimestampSchema,
  // Acknowledgment of received operations
  ack: z.object({
    lastId: z.string(),
    results: z.array(OpResultSchema).optional(),
  }).optional(),
  // Delta records for requested maps (new/changed since lastSyncTimestamp)
  deltas: z.array(MapDeltaSchema).optional(),
  // Query results
  queryResults: z.array(HttpQueryResultSchema).optional(),
  // Search results
  searchResults: z.array(HttpSearchResultSchema).optional(),
  // Errors for individual operations
  errors: z.array(HttpSyncErrorSchema).optional(),
});

export type HttpSyncResponse = z.infer<typeof HttpSyncResponseSchema>;
