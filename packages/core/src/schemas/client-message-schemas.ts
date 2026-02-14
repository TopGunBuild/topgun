// packages/core/src/schemas/client-message-schemas.ts
import { z } from 'zod';
import {
  TimestampSchema,
  LWWRecordSchema,
  Timestamp,
} from './base-schemas';
import {
  ORMapRecordSchema,
} from './base-schemas';
import {
  SyncRespRootMessageSchema,
  SyncRespBucketsMessageSchema,
  SyncRespLeafMessageSchema,
  ORMapSyncRespRootSchema,
  ORMapSyncRespBucketsSchema,
  ORMapSyncRespLeafSchema,
  ORMapDiffResponseSchema,
} from './sync-schemas';
import { PNCounterStateObjectSchema } from './messaging-schemas';

// --- Server Event Messages ---

export const ServerEventPayloadSchema = z.object({
  mapName: z.string(),
  eventType: z.enum(['PUT', 'REMOVE', 'OR_ADD', 'OR_REMOVE']),
  key: z.string(),
  record: LWWRecordSchema.optional(),
  orRecord: ORMapRecordSchema.optional(),
  orTag: z.string().optional(),
});
export type ServerEventPayload = z.infer<typeof ServerEventPayloadSchema>;

export const ServerEventMessageSchema = z.object({
  type: z.literal('SERVER_EVENT'),
  payload: ServerEventPayloadSchema,
});
export type ServerEventMessage = z.infer<typeof ServerEventMessageSchema>;

export const ServerBatchEventMessageSchema = z.object({
  type: z.literal('SERVER_BATCH_EVENT'),
  payload: z.object({
    events: z.array(ServerEventPayloadSchema),
  }),
});
export type ServerBatchEventMessage = z.infer<typeof ServerBatchEventMessageSchema>;

// --- Query Update Message ---

export const QueryUpdatePayloadSchema = z.object({
  queryId: z.string(),
  key: z.string(),
  value: z.unknown(),
  type: z.enum(['ENTER', 'UPDATE', 'REMOVE']),
});
export type QueryUpdatePayload = z.infer<typeof QueryUpdatePayloadSchema>;

export const QueryUpdateMessageSchema = z.object({
  type: z.literal('QUERY_UPDATE'),
  payload: QueryUpdatePayloadSchema,
});
export type QueryUpdateMessage = z.infer<typeof QueryUpdateMessageSchema>;

// --- GC Prune Message ---

export const GcPrunePayloadSchema = z.object({
  olderThan: TimestampSchema,
});
export type GcPrunePayload = z.infer<typeof GcPrunePayloadSchema>;

export const GcPruneMessageSchema = z.object({
  type: z.literal('GC_PRUNE'),
  payload: GcPrunePayloadSchema,
});
export type GcPruneMessage = z.infer<typeof GcPruneMessageSchema>;

// --- Auth Messages ---

export const AuthAckMessageSchema = z.object({
  type: z.literal('AUTH_ACK'),
  protocolVersion: z.number().optional(),
});
export type AuthAckMessage = z.infer<typeof AuthAckMessageSchema>;

export const AuthFailMessageSchema = z.object({
  type: z.literal('AUTH_FAIL'),
  error: z.string().optional(),
  code: z.number().optional(),
});
export type AuthFailMessage = z.infer<typeof AuthFailMessageSchema>;

// --- Error Message ---

export const ErrorMessageSchema = z.object({
  type: z.literal('ERROR'),
  payload: z.object({
    code: z.number(),
    message: z.string(),
    details: z.unknown().optional(),
  }),
});
export type ErrorMessage = z.infer<typeof ErrorMessageSchema>;

// --- Lock Messages ---

export const LockGrantedPayloadSchema = z.object({
  requestId: z.string(),
  name: z.string(),
  fencingToken: z.number(),
});
export type LockGrantedPayload = z.infer<typeof LockGrantedPayloadSchema>;

export const LockReleasedPayloadSchema = z.object({
  requestId: z.string(),
  name: z.string(),
  success: z.boolean(),
});
export type LockReleasedPayload = z.infer<typeof LockReleasedPayloadSchema>;

// --- Sync Reset Message ---

export const SyncResetRequiredPayloadSchema = z.object({
  mapName: z.string(),
  reason: z.string(),
});
export type SyncResetRequiredPayload = z.infer<typeof SyncResetRequiredPayloadSchema>;

// --- Payload Types from Existing Sync Schemas ---

export type SyncRespRootPayload = z.infer<typeof SyncRespRootMessageSchema>['payload'];
export type SyncRespBucketsPayload = z.infer<typeof SyncRespBucketsMessageSchema>['payload'];
export type SyncRespLeafPayload = z.infer<typeof SyncRespLeafMessageSchema>['payload'];

export type ORMapSyncRespRootPayload = z.infer<typeof ORMapSyncRespRootSchema>['payload'];
export type ORMapSyncRespBucketsPayload = z.infer<typeof ORMapSyncRespBucketsSchema>['payload'];
export type ORMapSyncRespLeafPayload = z.infer<typeof ORMapSyncRespLeafSchema>['payload'];
export type ORMapDiffResponsePayload = z.infer<typeof ORMapDiffResponseSchema>['payload'];

// --- Export PNCounterStateObject Type ---

export type PNCounterStateObject = z.infer<typeof PNCounterStateObjectSchema>;
