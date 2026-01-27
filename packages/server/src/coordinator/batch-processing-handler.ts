/**
 * BatchProcessingHandler - Handles batch operation processing with backpressure
 *
 * This handler manages batch processing including:
 * - Async batch processing with backpressure regulation
 * - Sync batch processing when system is overloaded
 * - Operation forwarding to partition owners
 * - Batched broadcast optimization
 *
 * Extracted from ServerCoordinator as part of SPEC-003d refactoring.
 */

import { logger } from '../utils/logger';
import type { IBatchProcessingHandler, BatchProcessingHandlerConfig } from './types';

export class BatchProcessingHandler implements IBatchProcessingHandler {
    private readonly config: BatchProcessingHandlerConfig;

    constructor(config: BatchProcessingHandlerConfig) {
        this.config = config;
    }

    /**
     * === OPTIMIZATION 1: Async Batch Processing with Backpressure ===
     * Processes validated operations asynchronously after ACK has been sent.
     * Uses BackpressureRegulator to periodically force sync processing and
     * prevent unbounded accumulation of async work.
     */
    async processBatchAsync(ops: any[], clientId: string): Promise<void> {
        // === BACKPRESSURE: Check if we should force sync processing ===
        if (this.config.backpressure.shouldForceSync()) {
            this.config.metricsService.incBackpressureSyncForced();
            await this.processBatchSync(ops, clientId);
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
                        // Process without immediate broadcast (we'll batch them)
                        await this.processLocalOpForBatch(op, clientId, batchedEvents);
                    } catch (err) {
                        logger.warn({ clientId, mapName: op.mapName, key: op.key, err }, 'Op failed in async batch');
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
                            opType: op.opType
                        }
                    });
                }
            }

            // Send batched broadcast if we have events
            if (batchedEvents.length > 0) {
                this.config.broadcastBatch(batchedEvents, clientId);
            }
        } finally {
            this.config.backpressure.completePending();
            this.config.metricsService.setBackpressurePendingOps(this.config.backpressure.getPendingOps());
        }
    }

    /**
     * === BACKPRESSURE: Synchronous Batch Processing ===
     * Processes operations synchronously, waiting for broadcast completion.
     * Used when backpressure forces sync to drain the pipeline.
     */
    async processBatchSync(ops: any[], clientId: string): Promise<void> {
        const batchedEvents: any[] = [];

        for (const op of ops) {
            if (this.config.partitionService.isLocalOwner(op.key)) {
                try {
                    await this.processLocalOpForBatch(op, clientId, batchedEvents);
                } catch (err) {
                    logger.warn({ clientId, mapName: op.mapName, key: op.key, err }, 'Op failed in sync batch');
                }
            } else {
                // Forward to owner and wait for acknowledgment
                const owner = this.config.partitionService.getOwner(op.key);
                await this.forwardOpAndWait(op, owner);
            }
        }

        // Send batched broadcast SYNCHRONOUSLY - wait for all sends to complete
        if (batchedEvents.length > 0) {
            await this.config.broadcastBatchSync(batchedEvents, clientId);
        }
    }

    /**
     * Forward operation to owner node and wait for completion.
     * Used in sync processing mode.
     */
    async forwardOpAndWait(op: any, owner: string): Promise<void> {
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

    /**
     * Process a single operation for batch processing.
     * Uses shared applyOpToMap but collects events instead of broadcasting immediately.
     */
    async processLocalOpForBatch(op: any, clientId: string, batchedEvents: any[]): Promise<void> {
        // 1. Build context for interceptors
        const context = this.config.buildOpContext(clientId, false);

        // 2. Run onBeforeOp interceptors
        try {
            const processedOp = await this.config.runBeforeInterceptors(op, context);
            if (!processedOp) return; // Silently dropped by interceptor
            op = processedOp;
        } catch (err) {
            logger.warn({ err, opId: op.id }, 'Interceptor rejected op in batch');
            throw err;
        }

        // 3. Apply operation to map (shared logic)
        const { eventPayload, rejected } = await this.config.applyOpToMap(op, clientId);

        // Skip further processing if operation was rejected by conflict resolver
        if (rejected || !eventPayload) {
            return;
        }

        // 4. Replicate to backup nodes (Hazelcast pattern: after local merge)
        if (this.config.replicationPipeline) {
            const opId = op.id || `${op.mapName}:${op.key}:${Date.now()}`;
            // Fire-and-forget for batch operations (EVENTUAL by default)
            this.config.replicationPipeline.replicate(op, opId, op.key).catch(err => {
                logger.warn({ opId, key: op.key, err }, 'Batch replication failed (non-fatal)');
            });
        }

        // 5. Collect event for batched broadcast (instead of immediate broadcast)
        batchedEvents.push(eventPayload);

        // 6. Distributed subscriptions are now handled via CLUSTER_SUB_UPDATE (Phase 14.2)
        // ReplicationPipeline handles data replication to backup nodes
        // No need for broadcastToCluster here - it was O(N) broadcast to all nodes

        // 7. Run onAfterOp interceptors
        this.config.runAfterInterceptors(op, context);
    }
}
