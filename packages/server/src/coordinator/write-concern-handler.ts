/**
 * WriteConcernHandler - Handles Write Concern tracking and acknowledgments
 *
 * This handler manages Write Concern levels including:
 * - Write Concern level resolution (op-level vs batch-level)
 * - String to enum conversion
 * - Async/sync batch processing with Write Concern tracking
 * - ACK notifications at each stage (APPLIED, REPLICATED, PERSISTED)
 *
 * Extracted from ServerCoordinator.
 */

import { WriteConcern, WriteConcernValue } from '@topgunbuild/core';
import { logger } from '../utils/logger';
import type { IWriteConcernHandler, WriteConcernHandlerConfig } from './types';

export class WriteConcernHandler implements IWriteConcernHandler {
    private readonly config: WriteConcernHandlerConfig;

    constructor(config: WriteConcernHandlerConfig) {
        this.config = config;
    }

    /**
     * Get effective Write Concern level for an operation.
     * Per-op writeConcern overrides batch-level.
     */
    getEffectiveWriteConcern(
        opWriteConcern: WriteConcernValue | undefined,
        batchWriteConcern: WriteConcernValue | undefined
    ): WriteConcernValue | undefined {
        return opWriteConcern ?? batchWriteConcern;
    }

    /**
     * Convert string WriteConcern value to enum.
     */
    stringToWriteConcern(value: WriteConcernValue | undefined): WriteConcern {
        switch (value) {
            case 'FIRE_AND_FORGET':
                return WriteConcern.FIRE_AND_FORGET;
            case 'MEMORY':
                return WriteConcern.MEMORY;
            case 'APPLIED':
                return WriteConcern.APPLIED;
            case 'REPLICATED':
                return WriteConcern.REPLICATED;
            case 'PERSISTED':
                return WriteConcern.PERSISTED;
            default:
                return WriteConcern.MEMORY;
        }
    }

    /**
     * Process batch with Write Concern tracking.
     * Notifies WriteAckManager at each stage of processing.
     */
    async processBatchAsyncWithWriteConcern(
        ops: any[],
        clientId: string,
        batchWriteConcern?: WriteConcernValue,
        batchTimeout?: number
    ): Promise<void> {
        // === BACKPRESSURE: Check if we should force sync processing ===
        if (this.config.backpressure.shouldForceSync()) {
            this.config.metricsService.incBackpressureSyncForced();
            await this.processBatchSyncWithWriteConcern(ops, clientId, batchWriteConcern, batchTimeout);
            return;
        }

        // === BACKPRESSURE: Check and wait for capacity ===
        if (!this.config.backpressure.registerPending()) {
            this.config.metricsService.incBackpressureWaits();
            try {
                await this.config.backpressure.waitForCapacity();
                this.config.backpressure.registerPending();
            } catch (err) {
                this.config.metricsService.incBackpressureTimeouts();
                logger.warn({ clientId, pendingOps: ops.length }, 'Backpressure timeout - rejecting batch');
                // Fail all pending operations
                for (const op of ops) {
                    if (op.id) {
                        this.config.writeAckManager.failPending(op.id, 'Server overloaded');
                    }
                }
                throw new Error('Server overloaded');
            }
        }

        // Update pending ops metric
        this.config.metricsService.setBackpressurePendingOps(this.config.backpressure.getPendingOps());

        try {
            // === OPTIMIZATION 3: Batch Broadcast ===
            // Collect all events for a single batched broadcast at the end
            const batchedEvents: any[] = [];

            for (const op of ops) {
                if (this.config.partitionService.isLocalOwner(op.key)) {
                    try {
                        // Process operation with Write Concern tracking
                        await this.processLocalOpWithWriteConcern(op, clientId, batchedEvents, batchWriteConcern);
                    } catch (err) {
                        logger.warn({ clientId, mapName: op.mapName, key: op.key, err }, 'Op failed in async batch');
                        // Fail the pending write
                        if (op.id) {
                            this.config.writeAckManager.failPending(op.id, String(err));
                        }
                    }
                } else {
                    // Forward to owner
                    const owner = this.config.partitionService.getOwner(op.key);
                    this.config.cluster.sendToNode(owner, {
                        type: 'CLIENT_OP',
                        payload: {
                            mapName: op.mapName,
                            key: op.key,
                            record: op.record,
                            orRecord: op.orRecord,
                            orTag: op.orTag,
                            opType: op.opType,
                            writeConcern: op.writeConcern ?? batchWriteConcern,
                        }
                    });
                    // For forwarded ops, we mark REPLICATED immediately since it's sent to cluster
                    if (op.id) {
                        this.config.writeAckManager.notifyLevel(op.id, WriteConcern.REPLICATED);
                    }
                }
            }

            // Send batched broadcast if we have events
            if (batchedEvents.length > 0) {
                this.config.broadcastBatch(batchedEvents, clientId);
                // Notify REPLICATED for all ops that were broadcast
                for (const op of ops) {
                    if (op.id && this.config.partitionService.isLocalOwner(op.key)) {
                        this.config.writeAckManager.notifyLevel(op.id, WriteConcern.REPLICATED);
                    }
                }
            }
        } finally {
            this.config.backpressure.completePending();
            this.config.metricsService.setBackpressurePendingOps(this.config.backpressure.getPendingOps());
        }
    }

    /**
     * Synchronous batch processing with Write Concern.
     */
    async processBatchSyncWithWriteConcern(
        ops: any[],
        clientId: string,
        batchWriteConcern?: WriteConcernValue,
        batchTimeout?: number
    ): Promise<void> {
        const batchedEvents: any[] = [];

        for (const op of ops) {
            if (this.config.partitionService.isLocalOwner(op.key)) {
                try {
                    await this.processLocalOpWithWriteConcern(op, clientId, batchedEvents, batchWriteConcern);
                } catch (err) {
                    logger.warn({ clientId, mapName: op.mapName, key: op.key, err }, 'Op failed in sync batch');
                    if (op.id) {
                        this.config.writeAckManager.failPending(op.id, String(err));
                    }
                }
            } else {
                // Forward to owner and wait for acknowledgment
                const owner = this.config.partitionService.getOwner(op.key);
                await this.forwardOpAndWait(op, owner);
                // Mark REPLICATED after forwarding
                if (op.id) {
                    this.config.writeAckManager.notifyLevel(op.id, WriteConcern.REPLICATED);
                }
            }
        }

        // Send batched broadcast SYNCHRONOUSLY - wait for all sends to complete
        if (batchedEvents.length > 0) {
            await this.config.broadcastBatchSync(batchedEvents, clientId);
            // Notify REPLICATED for all local ops
            for (const op of ops) {
                if (op.id && this.config.partitionService.isLocalOwner(op.key)) {
                    this.config.writeAckManager.notifyLevel(op.id, WriteConcern.REPLICATED);
                }
            }
        }
    }

    /**
     * Process a single operation with Write Concern level notifications.
     */
    async processLocalOpWithWriteConcern(
        op: any,
        clientId: string,
        batchedEvents: any[],
        batchWriteConcern?: WriteConcernValue
    ): Promise<void> {
        // 1. Build context for interceptors
        const context = this.config.buildOpContext(clientId, false);

        // 2. Run onBeforeOp interceptors
        try {
            const processedOp = await this.config.runBeforeInterceptors(op, context);
            if (!processedOp) {
                // Silently dropped by interceptor - fail the pending write
                if (op.id) {
                    this.config.writeAckManager.failPending(op.id, 'Dropped by interceptor');
                }
                return;
            }
            op = processedOp;
        } catch (err) {
            logger.warn({ opId: op.id, err }, 'Interceptor rejected op');
            if (op.id) {
                this.config.writeAckManager.failPending(op.id, String(err));
            }
            return;
        }

        // 3. Apply operation to map
        const { eventPayload, rejected } = await this.config.applyOpToMap(op, clientId);

        // If rejected by conflict resolver, fail the pending write
        if (rejected) {
            if (op.id) {
                this.config.writeAckManager.failPending(op.id, 'Rejected by conflict resolver');
            }
            return;
        }

        // 4. Notify APPLIED level (CRDT merged)
        if (op.id) {
            this.config.writeAckManager.notifyLevel(op.id, WriteConcern.APPLIED);
        }

        // 5. Collect event for batched broadcast
        if (eventPayload) {
            batchedEvents.push({
                mapName: op.mapName,
                key: op.key,
                ...eventPayload
            });
        }

        // 6. Handle PERSISTED Write Concern
        const effectiveWriteConcern = this.getEffectiveWriteConcern(op.writeConcern, batchWriteConcern);
        if (effectiveWriteConcern === 'PERSISTED' && this.config.storage) {
            try {
                // Wait for storage write to complete
                await this.config.persistOpSync(op);
                if (op.id) {
                    this.config.writeAckManager.notifyLevel(op.id, WriteConcern.PERSISTED);
                }
            } catch (err) {
                logger.error({ opId: op.id, err }, 'Persistence failed');
                if (op.id) {
                    this.config.writeAckManager.failPending(op.id, `Persistence failed: ${err}`);
                }
            }
        } else if (this.config.storage && op.id) {
            // Fire-and-forget persistence for non-PERSISTED writes
            this.config.persistOpAsync(op).catch(err => {
                logger.error({ opId: op.id, err }, 'Async persistence failed');
            });
        }

        // 7. Run onAfterOp interceptors
        try {
            const serverOp: any = {
                mapName: op.mapName,
                key: op.key,
                opType: op.opType || (op.record?.value === null ? 'REMOVE' : 'PUT'),
                record: op.record,
                orRecord: op.orRecord,
                orTag: op.orTag,
            };
            await this.config.runAfterInterceptors(serverOp, context);
        } catch (err) {
            logger.warn({ opId: op.id, err }, 'onAfterOp interceptor failed');
        }
    }

    /**
     * Forward operation to owner node and wait for completion.
     * Used in sync processing mode.
     */
    private async forwardOpAndWait(op: any, owner: string): Promise<void> {
        return new Promise<void>((resolve) => {
            // Fire and forget for now - cluster forwarding doesn't have ack mechanism
            // In a full implementation, this would wait for cluster ACK
            this.config.cluster.sendToNode(owner, {
                type: 'CLIENT_OP',
                payload: {
                    mapName: op.mapName,
                    key: op.key,
                    record: op.record,
                    orRecord: op.orRecord,
                    orTag: op.orTag,
                    opType: op.opType
                }
            });
            // Resolve immediately since cluster doesn't support sync ACK yet
            resolve();
        });
    }
}
