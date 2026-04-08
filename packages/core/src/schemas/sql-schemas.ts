// packages/core/src/schemas/sql-schemas.ts
import { z } from 'zod';

// --- SQL_QUERY (client -> server) ---
export const SqlQueryPayloadSchema = z.object({
  sql: z.string(),
  queryId: z.string(),
});
export type SqlQueryPayload = z.infer<typeof SqlQueryPayloadSchema>;

export const SqlQueryMessageSchema = z.object({
  type: z.literal('SQL_QUERY'),
  payload: SqlQueryPayloadSchema,
});
export type SqlQueryMessage = z.infer<typeof SqlQueryMessageSchema>;

// --- SQL_QUERY_RESP (server -> client) ---
export const SqlQueryRespPayloadSchema = z.object({
  queryId: z.string(),
  columns: z.array(z.string()),
  rows: z.array(z.array(z.unknown())),
  error: z.string().optional(),
});
export type SqlQueryRespPayload = z.infer<typeof SqlQueryRespPayloadSchema>;

export const SqlQueryRespMessageSchema = z.object({
  type: z.literal('SQL_QUERY_RESP'),
  payload: SqlQueryRespPayloadSchema,
});
export type SqlQueryRespMessage = z.infer<typeof SqlQueryRespMessageSchema>;
