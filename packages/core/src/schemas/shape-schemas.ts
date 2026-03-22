// packages/core/src/schemas/shape-schemas.ts
import { z } from 'zod';
import { PredicateNodeSchema, ChangeEventTypeSchema } from './base-schemas';

// --- Shape Record ---

/**
 * A single record returned in a shape response.
 * Mirrors the Rust ShapeRecord struct: { key: String, value: rmpv::Value }.
 * On the wire, SHAPE_RESP.records is an array of these objects.
 */
export const ShapeRecordSchema = z.object({
  key: z.string(),
  value: z.unknown(),
});
export type ShapeRecord = z.infer<typeof ShapeRecordSchema>;

// --- SyncShape (nested inside ShapeSubscribePayload) ---

/**
 * Describes the shape definition sent to the server.
 * Mirrors the Rust SyncShape struct.
 */
export const SyncShapeSchema = z.object({
  shapeId: z.string(),
  mapName: z.string(),
  filter: PredicateNodeSchema.optional(),
  fields: z.array(z.string()).optional(),
  limit: z.number().int().optional(),
});
export type SyncShape = z.infer<typeof SyncShapeSchema>;

// --- SHAPE_SUBSCRIBE ---

export const ShapeSubscribePayloadSchema = z.object({
  shape: SyncShapeSchema,
});
export type ShapeSubscribePayload = z.infer<typeof ShapeSubscribePayloadSchema>;

export const ShapeSubscribeMessageSchema = z.object({
  type: z.literal('SHAPE_SUBSCRIBE'),
  payload: ShapeSubscribePayloadSchema,
});
export type ShapeSubscribeMessage = z.infer<typeof ShapeSubscribeMessageSchema>;

// --- SHAPE_UNSUBSCRIBE ---

export const ShapeUnsubscribePayloadSchema = z.object({
  shapeId: z.string(),
});
export type ShapeUnsubscribePayload = z.infer<typeof ShapeUnsubscribePayloadSchema>;

export const ShapeUnsubscribeMessageSchema = z.object({
  type: z.literal('SHAPE_UNSUBSCRIBE'),
  payload: ShapeUnsubscribePayloadSchema,
});
export type ShapeUnsubscribeMessage = z.infer<typeof ShapeUnsubscribeMessageSchema>;

// --- SHAPE_RESP ---

/**
 * Matches Rust ShapeRespPayload.
 * records is Vec<ShapeRecord> — serialized as an array of { key, value } objects.
 * merkleRootHash is u32.
 * hasMore maps to Rust has_more: Option<bool> — omitted when None.
 */
export const ShapeRespPayloadSchema = z.object({
  shapeId: z.string(),
  records: z.array(ShapeRecordSchema),
  merkleRootHash: z.number().int(),
  hasMore: z.boolean().optional(),
});
export type ShapeRespPayload = z.infer<typeof ShapeRespPayloadSchema>;

export const ShapeRespMessageSchema = z.object({
  type: z.literal('SHAPE_RESP'),
  payload: ShapeRespPayloadSchema,
});
export type ShapeRespMessage = z.infer<typeof ShapeRespMessageSchema>;

// --- SHAPE_UPDATE ---

/**
 * Matches Rust ShapeUpdatePayload.
 * value is Option — absent on the wire for LEAVE events (skip_serializing_if = "Option::is_none").
 * changeType reuses the existing ChangeEventType enum.
 */
export const ShapeUpdatePayloadSchema = z.object({
  shapeId: z.string(),
  key: z.string(),
  value: z.unknown().optional(),
  changeType: ChangeEventTypeSchema,
});
export type ShapeUpdatePayload = z.infer<typeof ShapeUpdatePayloadSchema>;

export const ShapeUpdateMessageSchema = z.object({
  type: z.literal('SHAPE_UPDATE'),
  payload: ShapeUpdatePayloadSchema,
});
export type ShapeUpdateMessage = z.infer<typeof ShapeUpdateMessageSchema>;

// --- SHAPE_SYNC_INIT ---

/**
 * Matches Rust ShapeSyncInitPayload.
 * rootHash is u32 — note the field name is rootHash here, NOT merkleRootHash.
 * This is intentionally different from ShapeRespPayload.merkleRootHash.
 */
export const ShapeSyncInitPayloadSchema = z.object({
  shapeId: z.string(),
  rootHash: z.number().int(),
});
export type ShapeSyncInitPayload = z.infer<typeof ShapeSyncInitPayloadSchema>;

export const ShapeSyncInitMessageSchema = z.object({
  type: z.literal('SHAPE_SYNC_INIT'),
  payload: ShapeSyncInitPayloadSchema,
});
export type ShapeSyncInitMessage = z.infer<typeof ShapeSyncInitMessageSchema>;
