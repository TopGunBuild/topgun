/**
 * ShapeHandle - Client-side Shape Subscription Handle
 *
 * Manages a live shape subscription with initial snapshot and delta updates.
 */

import type { ChangeEventType } from '@topgunbuild/core';
import { logger } from './utils/logger';

/**
 * A shape update event describing a single record change.
 */
export interface ShapeUpdate {
  key: string;
  /** Present for ENTER/UPDATE events; absent (undefined) for LEAVE events. */
  value: any | undefined;
  changeType: ChangeEventType;
}

/**
 * Callback type for shape update notifications.
 */
export type ShapeUpdateCallback = (update: ShapeUpdate) => void;

/**
 * Dependency injected by ShapeManager to allow ShapeHandle to send
 * SHAPE_UNSUBSCRIBE without a direct reference to SyncEngine.
 */
export interface ShapeHandleOptions {
  shapeId: string;
  mapName: string;
  filter?: any;
  fields?: string[];
  limit?: number;
  sendMessage: (message: any) => boolean;
}

/**
 * ShapeHandle manages a live shape subscription.
 *
 * Provides:
 * - Current matching records as a Map (populated from SHAPE_RESP)
 * - Delta updates via onUpdate callbacks (ENTER/UPDATE/LEAVE)
 * - Unsubscribe to stop updates and clean up server-side subscription
 * - merkleRootHash for efficient delta sync on reconnect
 */
export class ShapeHandle {
  /** Unique identifier for this shape subscription. */
  readonly shapeId: string;

  /** Original map name used to create this subscription (needed for reconnect). */
  readonly mapName: string;

  /** Original filter used to create this subscription (needed for reconnect). */
  readonly filter: any | undefined;

  /** Original field projection used to create this subscription (needed for reconnect). */
  readonly fields: string[] | undefined;

  /** Original limit used to create this subscription (needed for reconnect). */
  readonly limit: number | undefined;

  /**
   * Current matching records.
   * Populated from SHAPE_RESP (converted from array of { key, value } objects).
   * Updated by SHAPE_UPDATE (ENTER/UPDATE/LEAVE).
   */
  records: Map<string, any> = new Map();

  /**
   * Merkle root hash from the last SHAPE_RESP.
   * Stored as a u32 integer. Used as rootHash in SHAPE_SYNC_INIT on reconnect.
   */
  merkleRootHash: number = 0;

  private readonly sendMessage: (message: any) => boolean;
  private listeners: Set<ShapeUpdateCallback> = new Set();
  private unsubscribed = false;

  constructor(options: ShapeHandleOptions) {
    this.shapeId = options.shapeId;
    this.mapName = options.mapName;
    this.filter = options.filter;
    this.fields = options.fields;
    this.limit = options.limit;
    this.sendMessage = options.sendMessage;
  }

  /**
   * Register a callback for shape updates.
   * Called for ENTER, UPDATE, and LEAVE events.
   *
   * @returns Unsubscribe function that removes this callback.
   */
  onUpdate(callback: ShapeUpdateCallback): () => void {
    this.listeners.add(callback);
    return () => {
      this.listeners.delete(callback);
    };
  }

  /**
   * Unsubscribe from this shape.
   * Sends SHAPE_UNSUBSCRIBE to the server and clears local state.
   */
  unsubscribe(): void {
    if (this.unsubscribed) return;
    this.unsubscribed = true;

    this.sendMessage({
      type: 'SHAPE_UNSUBSCRIBE',
      payload: { shapeId: this.shapeId },
    });

    this.records.clear();
    this.listeners.clear();
  }

  /**
   * Notify all update listeners.
   * Called by ShapeManager when a SHAPE_UPDATE message arrives for this shape.
   */
  notifyUpdate(update: ShapeUpdate): void {
    for (const listener of this.listeners) {
      try {
        listener(update);
      } catch (err) {
        logger.error({ err, shapeId: this.shapeId }, 'ShapeHandle listener error');
      }
    }
  }

  /**
   * Whether unsubscribe() has been called on this handle.
   */
  isUnsubscribed(): boolean {
    return this.unsubscribed;
  }
}
