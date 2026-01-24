/**
 * LwwSyncHandler - Handles SYNC_INIT, MERKLE_REQ_BUCKET messages for LWWMap
 *
 * This handler manages Merkle tree-based synchronization for LWW maps.
 *
 * Extracted from ServerCoordinator as part of Phase 4 refactoring.
 */

import { LWWMap } from '@topgunbuild/core';
import { logger } from '../utils/logger';
import type { ILwwSyncHandler, ClientConnection, LwwSyncHandlerConfig } from './types';

export class LwwSyncHandler implements ILwwSyncHandler {
    private readonly config: LwwSyncHandlerConfig;

    constructor(config: LwwSyncHandlerConfig) {
        this.config = config;
    }

    /**
     * Handle SYNC_INIT message.
     * Returns the root hash of the LWWMap's Merkle tree.
     */
    async handleSyncInit(client: ClientConnection, message: any): Promise<void> {
        // Check READ permission
        if (!this.config.securityManager.checkPermission(client.principal!, message.mapName, 'READ')) {
            logger.warn({ clientId: client.id, mapName: message.mapName }, 'Access Denied: SYNC_INIT');
            client.writer.write({
                type: 'ERROR',
                payload: { code: 403, message: `Access Denied for map ${message.mapName}` }
            }, true);
            return;
        }

        const lastSync = message.lastSyncTimestamp || 0;
        const now = Date.now();
        if (lastSync > 0 && (now - lastSync) > this.config.gcAgeMs) {
            logger.warn({ clientId: client.id, lastSync, age: now - lastSync }, 'Client too old, sending SYNC_RESET_REQUIRED');
            client.writer.write({
                type: 'SYNC_RESET_REQUIRED',
                payload: { mapName: message.mapName }
            });
            return;
        }

        logger.info({ clientId: client.id, mapName: message.mapName }, 'Client requested sync');
        this.config.metricsService.incOp('GET', message.mapName);

        // [FIX] Wait for map to be fully loaded from storage before sending rootHash
        // This prevents sending rootHash=0 for maps that are still loading from PostgreSQL
        try {
            const mapForSync = await this.config.getMapAsync(message.mapName);
            if (mapForSync instanceof LWWMap) {
                // Use the incremental Merkle Tree from LWWMap
                const tree = mapForSync.getMerkleTree();
                const rootHash = tree.getRootHash();

                client.writer.write({
                    type: 'SYNC_RESP_ROOT',
                    payload: {
                        mapName: message.mapName,
                        rootHash,
                        timestamp: this.config.hlc.now()
                    }
                });
            } else {
                // ORMap sync not implemented via Merkle Tree yet
                logger.warn({ mapName: message.mapName }, 'SYNC_INIT requested for ORMap - Not Implemented');
                client.writer.write({
                    type: 'ERROR',
                    payload: { code: 501, message: `Merkle Sync not supported for ORMap ${message.mapName}` }
                }, true);
            }
        } catch (err) {
            logger.error({ err, mapName: message.mapName }, 'Failed to load map for SYNC_INIT');
            client.writer.write({
                type: 'ERROR',
                payload: { code: 500, message: `Failed to load map ${message.mapName}` }
            }, true);
        }
    }

    /**
     * Handle MERKLE_REQ_BUCKET message.
     * Returns bucket hashes or leaf records for a specific Merkle tree path.
     */
    async handleMerkleReqBucket(client: ClientConnection, message: any): Promise<void> {
        // Check READ permission
        if (!this.config.securityManager.checkPermission(client.principal!, message.payload.mapName, 'READ')) {
            client.writer.write({
                type: 'ERROR',
                payload: { code: 403, message: `Access Denied for map ${message.payload.mapName}` }
            }, true);
            return;
        }

        const { mapName, path } = message.payload;

        // [FIX] Wait for map to be fully loaded before accessing Merkle tree
        try {
            const mapForBucket = await this.config.getMapAsync(mapName);
            if (mapForBucket instanceof LWWMap) {
                const treeForBucket = mapForBucket.getMerkleTree();
                const buckets = treeForBucket.getBuckets(path);
                const node = treeForBucket.getNode(path);
                if (node && node.entries && node.entries.size > 0) {
                    const diffRecords = [];
                    for (const key of node.entries.keys()) {
                        diffRecords.push({ key, record: mapForBucket.getRecord(key) });
                    }
                    client.writer.write({
                        type: 'SYNC_RESP_LEAF',
                        payload: { mapName, path, records: diffRecords }
                    });
                } else {
                    client.writer.write({
                        type: 'SYNC_RESP_BUCKETS',
                        payload: { mapName, path, buckets }
                    });
                }
            }
        } catch (err) {
            logger.error({ err, mapName }, 'Failed to load map for MERKLE_REQ_BUCKET');
        }
    }
}
