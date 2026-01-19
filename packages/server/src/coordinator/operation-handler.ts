/**
 * OperationHandler - Handles CRDT operations (CLIENT_OP, OP_BATCH)
 *
 * This module extracts operation handling logic from ServerCoordinator:
 * - Permission checking for write operations
 * - Write Concern categorization and acknowledgment
 * - Delegation to processLocalOp for actual CRDT processing
 *
 * The actual CRDT merge logic remains in ServerCoordinator for now due to
 * the complexity of interceptors, replication, journal, and search indexing.
 * This extraction focuses on the entry point and coordination logic.
 *
 * Extracted from ServerCoordinator as part of Phase 4 refactoring.
 */

import { PermissionType } from '@topgunbuild/core';
import { logger } from '../utils/logger';
import type { IOperationHandler, ClientConnection, OperationHandlerConfig } from './types';

export class OperationHandler implements IOperationHandler {
    private readonly config: OperationHandlerConfig;

    constructor(config: OperationHandlerConfig) {
        this.config = config;
    }

    /**
     * Process a single client operation (CLIENT_OP message).
     *
     * Flow:
     * 1. Determine action type (PUT or REMOVE)
     * 2. Check permission
     * 3. If local owner: process locally
     * 4. If not local owner: forward to partition owner
     */
    async processClientOp(client: ClientConnection, op: any): Promise<void> {
        // Determine action type
        // LWW: op.record.value === null -> REMOVE
        // OR: OR_REMOVE or OR_ADD -> PUT (effectively)
        const isRemove = op.opType === 'REMOVE' || (op.record && op.record.value === null);
        const action: PermissionType = isRemove ? 'REMOVE' : 'PUT';
        this.config.incOp(isRemove ? 'DELETE' : 'PUT', op.mapName);

        // Check Permission
        if (!this.config.checkPermission(client.principal!, op.mapName, action)) {
            logger.warn({ clientId: client.id, action, mapName: op.mapName }, 'Access Denied: Client OP');
            client.writer.write({
                type: 'OP_REJECTED',
                payload: { opId: op.id, reason: 'Access Denied' }
            });
            return;
        }

        logger.info({ clientId: client.id, opType: op.opType, key: op.key, mapName: op.mapName }, 'Received op');

        if (this.config.isLocalOwner(op.key)) {
            try {
                await this.config.processLocalOp(op, false, client.id);
            } catch (err: any) {
                logger.error({ clientId: client.id, err }, 'Op failed');
                client.writer.write({
                    type: 'OP_REJECTED',
                    payload: { opId: op.id, reason: err.message || 'Internal Error' }
                });
            }
        } else {
            this.config.forwardToOwner(op);
        }
    }

    /**
     * Process a batch of operations (OP_BATCH message).
     *
     * Flow:
     * 1. Early validation pass - check permissions
     * 2. Categorize by Write Concern (MEMORY vs deferred)
     * 3. Send early ACK for MEMORY/FIRE_AND_FORGET ops
     * 4. Register deferred ops with WriteAckManager
     * 5. Process valid ops asynchronously
     */
    async processOpBatch(
        client: ClientConnection,
        ops: any[],
        batchWriteConcern?: string,
        batchTimeout?: number
    ): Promise<void> {
        logger.info({ clientId: client.id, count: ops.length, writeConcern: batchWriteConcern }, 'Received batch');

        // === OPTIMIZATION 1: Early ACK ===
        // Fast validation pass - check permissions without processing
        const validOps: typeof ops = [];
        let rejectedCount = 0;

        // Categorize ops by Write Concern for different ACK handling
        const memoryOps: typeof ops = []; // Ops that need immediate ACK (MEMORY or FIRE_AND_FORGET)
        const deferredOps: typeof ops = []; // Ops that need deferred ACK (APPLIED, REPLICATED, PERSISTED)

        for (const op of ops) {
            const isRemove = op.opType === 'REMOVE' || (op.record && op.record.value === null);
            const action: PermissionType = isRemove ? 'REMOVE' : 'PUT';

            if (!this.config.checkPermission(client.principal!, op.mapName, action)) {
                rejectedCount++;
                logger.warn({ clientId: client.id, action, mapName: op.mapName }, 'Access Denied (Batch)');
                continue;
            }

            validOps.push(op);

            // Determine effective Write Concern for this operation
            const effectiveWriteConcern = this.config.getEffectiveWriteConcern(op.writeConcern, batchWriteConcern);

            // Categorize by Write Concern level
            if (effectiveWriteConcern === 'FIRE_AND_FORGET' || effectiveWriteConcern === 'MEMORY' || !effectiveWriteConcern) {
                memoryOps.push(op);
            } else {
                deferredOps.push(op);
            }
        }

        // Send Early ACK for MEMORY/FIRE_AND_FORGET ops (backwards compatible)
        if (memoryOps.length > 0) {
            const lastMemoryId = memoryOps[memoryOps.length - 1].id;
            if (lastMemoryId) {
                client.writer.write({
                    type: 'OP_ACK',
                    payload: {
                        lastId: lastMemoryId,
                        achievedLevel: 'MEMORY'
                    }
                });
            }
        }

        // Send rejection error if any ops were denied
        if (rejectedCount > 0) {
            client.writer.write({
                type: 'ERROR',
                payload: { code: 403, message: `Partial batch failure: ${rejectedCount} ops denied` }
            }, true);
        }

        // Register deferred ops with WriteAckManager for tracking
        for (const op of deferredOps) {
            if (op.id) {
                const effectiveWriteConcern = this.config.getEffectiveWriteConcern(op.writeConcern, batchWriteConcern);
                const effectiveTimeout = op.timeout ?? batchTimeout;
                const wcLevel = this.config.stringToWriteConcern(effectiveWriteConcern);

                // Register and handle the promise
                this.config.writeAckManager.registerPending(op.id, wcLevel, effectiveTimeout)
                    .then((result) => {
                        // Send ACK when Write Concern is achieved
                        client.writer.write({
                            type: 'OP_ACK',
                            payload: {
                                lastId: op.id!,
                                achievedLevel: result.achievedLevel,
                                results: [{
                                    opId: op.id!,
                                    success: result.success,
                                    achievedLevel: result.achievedLevel,
                                    error: result.error
                                }]
                            }
                        });
                    })
                    .catch((err) => {
                        logger.error({ opId: op.id, err }, 'Write concern tracking failed');
                    });
            }
        }

        // Process valid ops asynchronously (non-blocking)
        if (validOps.length > 0) {
            const batchPromise = new Promise<void>((resolve) => {
                setImmediate(() => {
                    this.config.processBatchAsync(validOps, client.id, batchWriteConcern, batchTimeout)
                        .catch(err => {
                            logger.error({ clientId: client.id, err }, 'Batch processing failed');
                        })
                        .finally(() => {
                            this.config.pendingBatchOperations.delete(batchPromise);
                            resolve();
                        });
                });
            });
            this.config.pendingBatchOperations.add(batchPromise);
        }
    }
}
