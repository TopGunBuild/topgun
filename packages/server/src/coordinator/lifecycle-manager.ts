import type { WebSocket } from 'ws';
import type { Server as HttpServer } from 'http';
import { serialize } from '@topgunbuild/core';
import type { LWWMap } from '@topgunbuild/core';
import { logger } from '../utils/logger';

/**
 * Configuration for LifecycleManager.
 */
export interface LifecycleManagerConfig {
    nodeId: string;

    // HTTP/WebSocket servers
    httpServer: HttpServer;
    metricsServer: HttpServer;
    wss: { close: () => void };

    // Core services
    metricsService: {
        destroy: () => void;
    };
    eventExecutor: {
        shutdown: (waitForPending: boolean) => Promise<void>;
    };
    connectionManager: {
        getClientCount: () => number;
        getClients: () => Map<string, {
            id: string;
            socket: WebSocket;
            writer?: { close: () => void };
        }>;
    };

    // Cluster components
    cluster?: {
        getMembers: () => string[];
        send: (nodeId: string, type: any, payload: any) => void;
        stop: () => void;
    };
    partitionService?: {
        getPartitionMap: () => { partitions: Array<{ partitionId: number; ownerNodeId: string }> };
    };
    replicationPipeline?: {
        getTotalPending: () => number;
        close: () => void;
    };

    // Optional components
    workerPool?: {
        shutdown: (timeoutMs: number) => Promise<void>;
    };
    storage?: {
        close: () => Promise<void>;
    };
    gcHandler?: {
        stop: () => void;
    };
    heartbeatHandler?: {
        stop: () => void;
    };
    lockManager?: {
        stop: () => void;
    };
    systemManager?: {
        stop: () => void;
    };
    repairScheduler?: {
        stop: () => void;
    };
    partitionReassigner?: {
        stop: () => void;
    };
    taskletScheduler: {
        shutdown: () => void;
    };
    writeAckManager: {
        shutdown: () => void;
    };
    entryProcessorHandler: {
        dispose: () => void;
    };
    eventJournalService?: {
        dispose: () => void;
    };
    eventPayloadPool: {
        clear: () => void;
    };
    clusterSearchCoordinator?: {
        destroy: () => void;
    };
    distributedSubCoordinator?: {
        destroy: () => void;
    };

    // Query conversion handler for cleanup
    queryConversionHandler?: {
        stop: () => void;
    };

    // Search backfill dependencies
    searchCoordinator: {
        getEnabledMaps: () => string[];
        buildIndexFromEntries: (mapName: string, entries: Iterable<[string, Record<string, unknown> | null]>) => void;
    };
    getMapAsync: (name: string) => Promise<LWWMap<string, any> | any>;
}

/**
 * Manages server lifecycle: startup, shutdown, and graceful cluster departure.
 * Extracted from ServerCoordinator to reduce file size.
 */
export class LifecycleManager {
    private readonly config: LifecycleManagerConfig;

    constructor(config: LifecycleManagerConfig) {
        this.config = config;
    }

    /**
     * Populate FTS indexes from existing map data.
     * Called after storage initialization.
     */
    async backfillSearchIndexes(): Promise<void> {
        const enabledMaps = this.config.searchCoordinator.getEnabledMaps();

        const promises = enabledMaps.map(async (mapName) => {
            try {
                const map = await this.config.getMapAsync(mapName);
                if (!map) return;

                const entries = Array.from(map.entries());
                if (entries.length > 0) {
                    logger.info({ mapName, count: entries.length }, 'Backfilling FTS index');
                    this.config.searchCoordinator.buildIndexFromEntries(
                        mapName,
                        map.entries() as Iterable<[string, Record<string, unknown> | null]>
                    );
                }
            } catch (err) {
                logger.error({ mapName, err }, 'Failed to backfill FTS index');
            }
        });

        await Promise.all(promises);
        logger.info('FTS backfill completed');
    }

    /**
     * Perform graceful shutdown of all server components.
     */
    async shutdown(): Promise<void> {
        logger.info('Shutting down Server Coordinator...');

        // 0. Clear pending cluster queries (before closing connections)
        if (this.config.queryConversionHandler) {
            this.config.queryConversionHandler.stop();
        }

        // Graceful cluster departure with partition notification
        await this.gracefulClusterDeparture();

        // 1. Stop accepting new connections
        this.config.httpServer.close();
        if (this.config.metricsServer) {
            this.config.metricsServer.close();
        }
        this.config.metricsService.destroy();
        this.config.wss.close();

        // 2. Notify and Close Clients
        logger.info(`Closing ${this.config.connectionManager.getClientCount()} client connections...`);
        const shutdownMsg = serialize({ type: 'SHUTDOWN_PENDING', retryAfter: 5000 });

        for (const client of this.config.connectionManager.getClients().values()) {
            try {
                if (client.socket.readyState === 1) { // WebSocket.OPEN
                    client.socket.send(shutdownMsg);
                    if (client.writer) {
                        client.writer.close();
                    }
                    client.socket.close(1001, 'Server Shutdown');
                }
            } catch (e) {
                logger.error({ err: e, clientId: client.id }, 'Error closing client connection');
            }
        }
        this.config.connectionManager.getClients().clear();

        // 3. Shutdown event executor (wait for pending tasks)
        logger.info('Shutting down event executor...');
        await this.config.eventExecutor.shutdown(true);

        // 3.5. Shutdown worker pool
        if (this.config.workerPool) {
            logger.info('Shutting down worker pool...');
            await this.config.workerPool.shutdown(5000);
            logger.info('Worker pool shutdown complete.');
        }

        // 4. Close ReplicationPipeline
        if (this.config.replicationPipeline) {
            this.config.replicationPipeline.close();
        }

        // 4.5. Stop Phase 10 components
        if (this.config.repairScheduler) {
            this.config.repairScheduler.stop();
            logger.info('RepairScheduler stopped');
        }
        if (this.config.partitionReassigner) {
            this.config.partitionReassigner.stop();
            logger.info('PartitionReassigner stopped');
        }

        // 5. Stop Cluster
        if (this.config.cluster) {
            this.config.cluster.stop();
        }

        // 6. Close Storage
        if (this.config.storage) {
            logger.info('Closing storage connection...');
            try {
                await this.config.storage.close();
                logger.info('Storage closed successfully.');
            } catch (err) {
                logger.error({ err }, 'Error closing storage');
            }
        }

        // 7. Cleanup handlers
        if (this.config.gcHandler) {
            this.config.gcHandler.stop();
        }
        if (this.config.heartbeatHandler) {
            this.config.heartbeatHandler.stop();
        }
        if (this.config.lockManager) {
            this.config.lockManager.stop();
        }
        if (this.config.systemManager) {
            this.config.systemManager.stop();
        }

        // 8. Clear memory pools and shutdown managers
        this.config.eventPayloadPool.clear();
        this.config.taskletScheduler.shutdown();
        this.config.writeAckManager.shutdown();
        this.config.entryProcessorHandler.dispose();

        if (this.config.eventJournalService) {
            this.config.eventJournalService.dispose();
        }
        if (this.config.clusterSearchCoordinator) {
            this.config.clusterSearchCoordinator.destroy();
        }
        if (this.config.distributedSubCoordinator) {
            this.config.distributedSubCoordinator.destroy();
        }

        logger.info('Server Coordinator shutdown complete.');
    }

    /**
     * Graceful cluster departure allowing time for:
     * 1. Pending replication to complete
     * 2. Other nodes to detect departure
     * 3. Partition reassignment to begin
     */
    private async gracefulClusterDeparture(): Promise<void> {
        if (!this.config.cluster || this.config.cluster.getMembers().length <= 1) {
            return;
        }

        const nodeId = this.config.nodeId;
        const ownedPartitions = this.getOwnedPartitions();

        logger.info({
            nodeId,
            ownedPartitions: ownedPartitions.length,
            clusterMembers: this.config.cluster.getMembers().length
        }, 'Initiating graceful cluster departure');

        // Notify cluster peers that we're leaving
        const departureMessage = {
            type: 'NODE_LEAVING',
            nodeId,
            partitions: ownedPartitions,
            timestamp: Date.now()
        };

        for (const memberId of this.config.cluster.getMembers()) {
            if (memberId !== nodeId) {
                try {
                    this.config.cluster.send(memberId, 'CLUSTER_EVENT', departureMessage);
                } catch (e) {
                    logger.warn({ memberId, err: e }, 'Failed to notify peer of departure');
                }
            }
        }

        // Wait for pending replication to flush
        if (this.config.replicationPipeline) {
            logger.info('Waiting for pending replication to complete...');
            try {
                await this.waitForReplicationFlush(3000);
                logger.info('Replication flush complete');
            } catch (e) {
                logger.warn({ err: e }, 'Replication flush timeout - some data may not be replicated');
            }
        }

        // Brief delay to allow cluster to process departure
        await new Promise(resolve => setTimeout(resolve, 500));

        logger.info({ nodeId }, 'Graceful cluster departure complete');
    }

    /**
     * Get list of partition IDs owned by this node.
     */
    private getOwnedPartitions(): number[] {
        if (!this.config.partitionService) return [];

        const partitionMap = this.config.partitionService.getPartitionMap();
        const owned: number[] = [];

        for (const partition of partitionMap.partitions) {
            if (partition.ownerNodeId === this.config.nodeId) {
                owned.push(partition.partitionId);
            }
        }

        return owned;
    }

    /**
     * Wait for replication pipeline to flush pending operations.
     */
    private async waitForReplicationFlush(timeoutMs: number): Promise<void> {
        if (!this.config.replicationPipeline) return;

        const startTime = Date.now();

        while (Date.now() - startTime < timeoutMs) {
            const pendingOps = this.config.replicationPipeline.getTotalPending();
            if (pendingOps === 0) {
                return;
            }
            await new Promise(resolve => setTimeout(resolve, 100));
        }

        throw new Error('Replication flush timeout');
    }
}
