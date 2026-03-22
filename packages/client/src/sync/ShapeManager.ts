/**
 * ShapeManager - Handles shape subscription operations for SyncEngine.
 *
 * Responsibilities:
 * - Sending SHAPE_SUBSCRIBE and SHAPE_UNSUBSCRIBE messages
 * - Maintaining active ShapeHandle instances (single source of truth)
 * - Applying SHAPE_RESP (initial snapshot) to handles
 * - Applying SHAPE_UPDATE (delta) to handles and notifying listeners
 * - Resubscribing all active shapes on reconnect with Merkle sync init
 */

import type { PredicateNode } from '@topgunbuild/core';
import type { ShapeRespPayload, ShapeUpdatePayload } from '@topgunbuild/core';
import { ShapeHandle } from '../ShapeHandle';
import { logger } from '../utils/logger';

/**
 * Options for subscribing to a shape.
 */
export interface ShapeSubscribeOptions {
  filter?: PredicateNode;
  fields?: string[];
  limit?: number;
}

/**
 * Configuration injected by SyncEngine.
 */
export interface ShapeManagerConfig {
  sendMessage: (message: any) => boolean;
}

/**
 * ShapeManager follows the QueryManager/TopicManager manager pattern.
 * Shape logic lives here, not in SyncEngine.
 */
export class ShapeManager {
  private readonly config: ShapeManagerConfig;

  /** Active shapes, keyed by shapeId. Single source of truth. */
  private shapes: Map<string, ShapeHandle> = new Map();

  constructor(config: ShapeManagerConfig) {
    this.config = config;
  }

  // ============================================
  // Public API
  // ============================================

  /**
   * Subscribe to a shape.
   * Generates a UUID, sends SHAPE_SUBSCRIBE, stores and returns the handle.
   */
  public subscribeShape(
    mapName: string,
    options?: ShapeSubscribeOptions
  ): ShapeHandle {
    const shapeId = crypto.randomUUID();

    const handle = new ShapeHandle({
      shapeId,
      mapName,
      filter: options?.filter,
      fields: options?.fields,
      limit: options?.limit,
      sendMessage: this.config.sendMessage,
    });

    this.shapes.set(shapeId, handle);

    // SyncShape is nested inside payload.shape (not flat on payload)
    this.config.sendMessage({
      type: 'SHAPE_SUBSCRIBE',
      payload: {
        shape: {
          shapeId,
          mapName,
          filter: options?.filter,
          fields: options?.fields,
          limit: options?.limit,
        },
      },
    });

    logger.debug({ shapeId, mapName }, 'Shape subscribed');

    return handle;
  }

  /**
   * Handle SHAPE_RESP from server.
   * Converts the records array to a Map and stores the merkle root hash.
   */
  public handleShapeResp(payload: ShapeRespPayload): void {
    const handle = this.shapes.get(payload.shapeId);
    if (!handle) {
      logger.warn({ shapeId: payload.shapeId }, 'SHAPE_RESP for unknown shape — ignoring');
      return;
    }

    // Convert Array<{ key, value }> to Map<string, any>
    const recordsMap = new Map<string, any>();
    for (const record of payload.records) {
      recordsMap.set(record.key, record.value);
    }
    handle.records = recordsMap;

    // Store merkle root hash for reconnect delta sync
    handle.merkleRootHash = payload.merkleRootHash;

    logger.debug(
      { shapeId: payload.shapeId, count: recordsMap.size, merkleRootHash: payload.merkleRootHash },
      'SHAPE_RESP applied'
    );
  }

  /**
   * Handle SHAPE_UPDATE from server.
   * Applies ENTER/UPDATE/LEAVE to the handle's records and notifies listeners.
   */
  public handleShapeUpdate(payload: ShapeUpdatePayload): void {
    const handle = this.shapes.get(payload.shapeId);
    if (!handle) {
      logger.warn({ shapeId: payload.shapeId }, 'SHAPE_UPDATE for unknown shape — ignoring');
      return;
    }

    const { key, value, changeType } = payload;

    switch (changeType) {
      case 'ENTER':
      case 'UPDATE':
        handle.records.set(key, value);
        break;
      case 'LEAVE':
        handle.records.delete(key);
        break;
    }

    handle.notifyUpdate({ key, value, changeType });

    logger.debug({ shapeId: payload.shapeId, key, changeType }, 'SHAPE_UPDATE applied');
  }

  /**
   * Re-subscribe all active shapes on reconnect.
   * Sends SHAPE_SUBSCRIBE followed by SHAPE_SYNC_INIT for each active shape.
   * SHAPE_SYNC_INIT carries the stored merkleRootHash as rootHash so the server
   * can send only the delta since the last snapshot.
   */
  public resubscribeAll(): void {
    // Collect stale handles first to avoid mutating the Map during iteration
    const staleIds: string[] = [];
    for (const [shapeId, handle] of this.shapes) {
      if (handle.isUnsubscribed()) {
        staleIds.push(shapeId);
      }
    }
    for (const id of staleIds) {
      this.shapes.delete(id);
    }

    // Re-subscribe active shapes: SHAPE_SUBSCRIBE first (server cleans up
    // registrations on disconnect), then SHAPE_SYNC_INIT for delta sync
    for (const [shapeId, handle] of this.shapes) {
      this.config.sendMessage({
        type: 'SHAPE_SUBSCRIBE',
        payload: {
          shape: {
            shapeId,
            mapName: handle.mapName,
            filter: handle.filter,
            fields: handle.fields,
            limit: handle.limit,
          },
        },
      });

      this.config.sendMessage({
        type: 'SHAPE_SYNC_INIT',
        payload: {
          shapeId,
          rootHash: handle.merkleRootHash,
        },
      });

      logger.debug(
        { shapeId, rootHash: handle.merkleRootHash },
        'Shape resubscribed with SHAPE_SUBSCRIBE + SHAPE_SYNC_INIT'
      );
    }
  }

  /**
   * Unsubscribe a shape by ID.
   * Sends SHAPE_UNSUBSCRIBE and removes from active shapes.
   */
  public unsubscribeShape(shapeId: string): void {
    const handle = this.shapes.get(shapeId);
    if (!handle) return;

    this.shapes.delete(shapeId);

    // ShapeHandle.unsubscribe() sends SHAPE_UNSUBSCRIBE and clears internal state
    if (!handle.isUnsubscribed()) {
      handle.unsubscribe();
    }

    logger.debug({ shapeId }, 'Shape unsubscribed');
  }
}
