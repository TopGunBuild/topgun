// packages/core/src/schemas/base-schemas.ts
import { z } from 'zod';

// --- Write Concern Types ---

/**
 * Write Concern schema - defines when an operation is considered acknowledged.
 */
export const WriteConcernSchema = z.enum([
  'FIRE_AND_FORGET',
  'MEMORY',
  'APPLIED',
  'REPLICATED',
  'PERSISTED',
]);
export type WriteConcernValue = z.infer<typeof WriteConcernSchema>;

// --- Basic Types ---
export const TimestampSchema = z.object({
  millis: z.union([z.number(), z.bigint()]).transform(Number),
  counter: z.union([z.number(), z.bigint()]).transform(Number),
  nodeId: z.string(),
});
export type Timestamp = z.infer<typeof TimestampSchema>;

export const LWWRecordSchema = z.object({
  value: z.any().nullable(),
  timestamp: TimestampSchema,
  ttlMs: z.number().optional(),
});
export type LWWRecord<V = any> = z.infer<typeof LWWRecordSchema>;

export const ORMapRecordSchema = z.object({
  value: z.any(),
  timestamp: TimestampSchema,
  tag: z.string(),
  ttlMs: z.number().optional(),
});
export type ORMapRecord<V = any> = z.infer<typeof ORMapRecordSchema>;

// --- Change Event Types ---

/**
 * Unified change event type used across query updates, search updates,
 * and cluster subscription updates.
 */
export const ChangeEventTypeSchema = z.enum(['ENTER', 'UPDATE', 'LEAVE']);
export type ChangeEventType = z.infer<typeof ChangeEventTypeSchema>;

// --- Predicate Types ---
export const PredicateOpSchema = z.enum([
  'eq', 'neq', 'gt', 'gte', 'lt', 'lte', 'like', 'regex', 'and', 'or', 'not'
]);
export type PredicateOp = z.infer<typeof PredicateOpSchema>;

export const PredicateNodeSchema: z.ZodType<any> = z.lazy(() => z.object({
  op: PredicateOpSchema,
  attribute: z.string().optional(),
  value: z.any().optional(),
  children: z.array(PredicateNodeSchema).optional(),
}));
export type PredicateNode = z.infer<typeof PredicateNodeSchema>;

// --- Query Types ---
export const QuerySchema = z.object({
  where: z.record(z.string(), z.any()).optional(),
  predicate: PredicateNodeSchema.optional(),
  sort: z.record(z.string(), z.enum(['asc', 'desc'])).optional(),
  limit: z.number().optional(),
  cursor: z.string().optional(), // Replaces offset for pagination
});
export type Query = z.infer<typeof QuerySchema>;

// --- Client Operation Types ---
export const ClientOpSchema = z.object({
  id: z.string().optional(),
  mapName: z.string(),
  key: z.string(),
  opType: z.string().optional(),
  record: LWWRecordSchema.nullable().optional(),
  orRecord: ORMapRecordSchema.nullable().optional(),
  orTag: z.string().nullable().optional(),
  writeConcern: WriteConcernSchema.optional(),
  timeout: z.number().optional(),
});
export type ClientOp = z.infer<typeof ClientOpSchema>;

// --- Auth Message ---
export const AuthMessageSchema = z.object({
  type: z.literal('AUTH'),
  token: z.string(),
  protocolVersion: z.number().optional(),
});
export type AuthMessage = z.infer<typeof AuthMessageSchema>;

/**
 * AUTH_REQUIRED: Server tells client that authentication is needed.
 */
export const AuthRequiredMessageSchema = z.object({
  type: z.literal('AUTH_REQUIRED'),
});
export type AuthRequiredMessage = z.infer<typeof AuthRequiredMessageSchema>;
