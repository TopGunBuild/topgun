/**
 * PersistenceHandler - Handles operation persistence to storage
 *
 * This handler manages:
 * - Synchronous persistence for PERSISTED write concern
 * - Asynchronous fire-and-forget persistence
 * - Both LWW and OR map storage
 *
 * Extracted from ServerCoordinator.
 */

import { LWWMap, ORMap } from '@topgunbuild/core';
import type { IPersistenceHandler, PersistenceHandlerConfig } from './types';
import type { ORMapValue, ORMapTombstones } from '../storage/IServerStorage';

export class PersistenceHandler implements IPersistenceHandler {
    private readonly config: PersistenceHandlerConfig;

    constructor(config: PersistenceHandlerConfig) {
        this.config = config;
    }

    /**
     * Persist operation synchronously (blocking).
     * Used for PERSISTED Write Concern.
     */
    async persistOpSync(op: any): Promise<void> {
        if (!this.config.storage) return;

        const isORMapOp = op.opType === 'OR_ADD' || op.opType === 'OR_REMOVE' || op.orRecord || op.orTag;

        if (isORMapOp) {
            const orMap = this.config.getMap(op.mapName, 'OR') as ORMap<string, any>;
            const records = orMap.getRecords(op.key);
            const tombstones = orMap.getTombstones();

            if (records.length > 0) {
                await this.config.storage.store(op.mapName, op.key, { type: 'OR', records } as ORMapValue<any>);
            } else {
                await this.config.storage.delete(op.mapName, op.key);
            }

            if (tombstones.length > 0) {
                await this.config.storage.store(op.mapName, '__tombstones__', { type: 'OR_TOMBSTONES', tags: tombstones } as ORMapTombstones);
            }
        } else {
            const lwwMap = this.config.getMap(op.mapName, 'LWW') as LWWMap<string, any>;
            const record = lwwMap.getRecord(op.key);
            if (record) {
                await this.config.storage.store(op.mapName, op.key, record);
            }
        }
    }

    /**
     * Persist operation asynchronously (fire-and-forget).
     * Used for non-PERSISTED Write Concern levels.
     */
    async persistOpAsync(op: any): Promise<void> {
        return this.persistOpSync(op);
    }
}
