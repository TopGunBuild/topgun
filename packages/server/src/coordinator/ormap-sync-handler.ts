/**
 * ORMapSyncHandler - Handles ORMAP_SYNC_INIT, ORMAP_MERKLE_REQ_BUCKET,
 * ORMAP_DIFF_REQUEST, ORMAP_PUSH_DIFF messages
 *
 * This handler manages Merkle tree-based synchronization for OR maps.
 *
 * Extracted from ServerCoordinator .
 */

import { ORMap, ORMapRecord } from '@topgunbuild/core';
import { logger } from '../utils/logger';
import type { IORMapSyncHandler, ClientConnection, ORMapSyncHandlerConfig } from './types';

export class ORMapSyncHandler implements IORMapSyncHandler {
    private readonly config: ORMapSyncHandlerConfig;

    constructor(config: ORMapSyncHandlerConfig) {
        this.config = config;
    }

    /**
     * Handle ORMAP_SYNC_INIT message.
     * Returns the root hash of the ORMap's Merkle tree.
     */
    async handleORMapSyncInit(client: ClientConnection, message: any): Promise<void> {
        // Check READ permission
        if (!this.config.securityManager.checkPermission(client.principal!, message.mapName, 'READ')) {
            client.writer.write({
                type: 'ERROR',
                payload: { code: 403, message: `Access Denied for map ${message.mapName}` }
            }, true);
            return;
        }

        const lastSync = message.lastSyncTimestamp || 0;
        const now = Date.now();
        if (lastSync > 0 && (now - lastSync) > this.config.gcAgeMs) {
            logger.warn({ clientId: client.id, lastSync, age: now - lastSync }, 'ORMap client too old, sending SYNC_RESET_REQUIRED');
            client.writer.write({
                type: 'SYNC_RESET_REQUIRED',
                payload: { mapName: message.mapName }
            });
            return;
        }

        logger.info({ clientId: client.id, mapName: message.mapName }, 'Client requested ORMap sync');
        this.config.metricsService.incOp('GET', message.mapName);

        try {
            const mapForSync = await this.config.getMapAsync(message.mapName, 'OR');
            if (mapForSync instanceof ORMap) {
                const tree = mapForSync.getMerkleTree();
                const rootHash = tree.getRootHash();

                client.writer.write({
                    type: 'ORMAP_SYNC_RESP_ROOT',
                    payload: {
                        mapName: message.mapName,
                        rootHash,
                        timestamp: this.config.hlc.now()
                    }
                });
            } else {
                // It's actually an LWWMap, client should use SYNC_INIT
                client.writer.write({
                    type: 'ERROR',
                    payload: { code: 400, message: `Map ${message.mapName} is not an ORMap` }
                }, true);
            }
        } catch (err) {
            logger.error({ err, mapName: message.mapName }, 'Failed to load map for ORMAP_SYNC_INIT');
            client.writer.write({
                type: 'ERROR',
                payload: { code: 500, message: `Failed to load map ${message.mapName}` }
            }, true);
        }
    }

    /**
     * Handle ORMAP_MERKLE_REQ_BUCKET message.
     * Returns bucket hashes or leaf entries for a specific Merkle tree path.
     */
    async handleORMapMerkleReqBucket(client: ClientConnection, message: any): Promise<void> {
        // Check READ permission
        if (!this.config.securityManager.checkPermission(client.principal!, message.payload.mapName, 'READ')) {
            client.writer.write({
                type: 'ERROR',
                payload: { code: 403, message: `Access Denied for map ${message.payload.mapName}` }
            }, true);
            return;
        }

        const { mapName, path } = message.payload;

        try {
            const mapForBucket = await this.config.getMapAsync(mapName, 'OR');
            if (mapForBucket instanceof ORMap) {
                const tree = mapForBucket.getMerkleTree();
                const buckets = tree.getBuckets(path);
                const isLeaf = tree.isLeaf(path);

                if (isLeaf) {
                    // This is a leaf node - send actual records
                    const keys = tree.getKeysInBucket(path);
                    const entries: Array<{ key: string; records: ORMapRecord<any>[]; tombstones: string[] }> = [];

                    for (const key of keys) {
                        const recordsMap = mapForBucket.getRecordsMap(key);
                        if (recordsMap && recordsMap.size > 0) {
                            entries.push({
                                key,
                                records: Array.from(recordsMap.values()),
                                tombstones: mapForBucket.getTombstones()
                            });
                        }
                    }

                    client.writer.write({
                        type: 'ORMAP_SYNC_RESP_LEAF',
                        payload: { mapName, path, entries }
                    });
                } else {
                    // Not a leaf - send bucket hashes
                    client.writer.write({
                        type: 'ORMAP_SYNC_RESP_BUCKETS',
                        payload: { mapName, path, buckets }
                    });
                }
            }
        } catch (err) {
            logger.error({ err, mapName }, 'Failed to load map for ORMAP_MERKLE_REQ_BUCKET');
        }
    }

    /**
     * Handle ORMAP_DIFF_REQUEST message.
     * Returns records for specific keys.
     */
    async handleORMapDiffRequest(client: ClientConnection, message: any): Promise<void> {
        // Check READ permission
        if (!this.config.securityManager.checkPermission(client.principal!, message.payload.mapName, 'READ')) {
            client.writer.write({
                type: 'ERROR',
                payload: { code: 403, message: `Access Denied for map ${message.payload.mapName}` }
            }, true);
            return;
        }

        const { mapName: diffMapName, keys } = message.payload;

        try {
            const mapForDiff = await this.config.getMapAsync(diffMapName, 'OR');
            if (mapForDiff instanceof ORMap) {
                const entries: Array<{ key: string; records: ORMapRecord<any>[]; tombstones: string[] }> = [];
                const allTombstones = mapForDiff.getTombstones();

                for (const key of keys) {
                    const recordsMap = mapForDiff.getRecordsMap(key);
                    entries.push({
                        key,
                        records: recordsMap ? Array.from(recordsMap.values()) : [],
                        tombstones: allTombstones
                    });
                }

                client.writer.write({
                    type: 'ORMAP_DIFF_RESPONSE',
                    payload: { mapName: diffMapName, entries }
                });
            }
        } catch (err) {
            logger.error({ err, mapName: diffMapName }, 'Failed to load map for ORMAP_DIFF_REQUEST');
        }
    }

    /**
     * Handle ORMAP_PUSH_DIFF message.
     * Merges incoming records and broadcasts changes.
     */
    async handleORMapPushDiff(client: ClientConnection, message: any): Promise<void> {
        // Check WRITE permission
        if (!this.config.securityManager.checkPermission(client.principal!, message.payload.mapName, 'PUT')) {
            client.writer.write({
                type: 'ERROR',
                payload: { code: 403, message: `Access Denied for map ${message.payload.mapName}` }
            }, true);
            return;
        }

        const { mapName: pushMapName, entries: pushEntries } = message.payload;

        try {
            const mapForPush = await this.config.getMapAsync(pushMapName, 'OR');
            if (mapForPush instanceof ORMap) {
                let totalAdded = 0;
                let totalUpdated = 0;

                for (const entry of pushEntries) {
                    const { key, records, tombstones } = entry;
                    const result = mapForPush.mergeKey(key, records, tombstones);
                    totalAdded += result.added;
                    totalUpdated += result.updated;
                }

                if (totalAdded > 0 || totalUpdated > 0) {
                    logger.info({ mapName: pushMapName, added: totalAdded, updated: totalUpdated, clientId: client.id }, 'Merged ORMap diff from client');

                    // Broadcast changes to other clients
                    for (const entry of pushEntries) {
                        for (const record of entry.records) {
                            this.config.broadcast({
                                type: 'SERVER_EVENT',
                                payload: {
                                    mapName: pushMapName,
                                    eventType: 'OR_ADD',
                                    key: entry.key,
                                    orRecord: record
                                }
                            }, client.id);
                        }
                    }

                    // Persist to storage
                    if (this.config.storage) {
                        for (const entry of pushEntries) {
                            const recordsMap = mapForPush.getRecordsMap(entry.key);
                            if (recordsMap && recordsMap.size > 0) {
                                await this.config.storage.store(pushMapName, entry.key, {
                                    type: 'OR',
                                    records: Array.from(recordsMap.values())
                                });
                            }
                        }
                    }
                }
            }
        } catch (err) {
            logger.error({ err, mapName: pushMapName }, 'Failed to process ORMAP_PUSH_DIFF');
        }
    }
}
