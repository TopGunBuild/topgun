import { LifecycleManager } from '../coordinator';
import type { LifecycleModuleConfig, LifecycleModuleDeps, LifecycleModule } from './types';

/**
 * Creates the lifecycle management module.
 *
 * This module creates a LifecycleManager that orchestrates graceful shutdown
 * of all server components in the correct order.
 */
export function createLifecycleModule(
    config: LifecycleModuleConfig,
    deps: LifecycleModuleDeps
): LifecycleModule {
    const lifecycleManager = new LifecycleManager({
        nodeId: config.nodeId,

        // Network shutdown
        httpServer: deps.httpServer,
        metricsServer: deps.metricsServer,
        wss: deps.wss,

        // Core shutdown
        metricsService: {
            destroy: () => deps.metricsService.destroy(),
        },
        eventExecutor: {
            shutdown: (wait) => deps.eventExecutor.shutdown(wait),
        },
        connectionManager: {
            getClientCount: () => deps.connectionManager.getClientCount(),
            getClients: () => deps.connectionManager.getClients(),
        },

        // Cluster shutdown
        cluster: deps.cluster ? {
            getMembers: () => deps.cluster!.getMembers(),
            send: (nodeId, type, payload) => deps.cluster!.send(nodeId, type, payload),
            stop: () => deps.cluster!.stop(),
        } : undefined,
        partitionService: deps.partitionService ? {
            getPartitionMap: () => deps.partitionService!.getPartitionMap(),
        } : undefined,
        replicationPipeline: deps.replicationPipeline ? {
            getTotalPending: () => deps.replicationPipeline!.getTotalPending(),
            close: () => deps.replicationPipeline!.close(),
        } : undefined,

        // Worker shutdown
        workerPool: deps.workerPool ? {
            shutdown: (timeout) => deps.workerPool!.shutdown(timeout),
        } : undefined,

        // Storage shutdown
        storage: deps.storage ? {
            close: () => deps.storage!.close(),
        } : undefined,
        taskletScheduler: {
            shutdown: () => deps.taskletScheduler.shutdown(),
        },
        writeAckManager: {
            shutdown: () => deps.writeAckManager.shutdown(),
        },
        eventPayloadPool: {
            clear: () => deps.eventPayloadPool.clear(),
        },

        // Handler shutdown
        gcHandler: deps.gcHandler ? {
            stop: () => deps.gcHandler!.stop(),
        } : undefined,
        heartbeatHandler: deps.heartbeatHandler ? {
            stop: () => deps.heartbeatHandler!.stop(),
        } : undefined,
        lockManager: deps.lockManager ? {
            stop: () => deps.lockManager!.stop(),
        } : undefined,
        systemManager: deps.systemManager ? {
            stop: () => deps.systemManager!.stop(),
        } : undefined,
        repairScheduler: deps.repairScheduler ? {
            stop: () => deps.repairScheduler!.stop(),
        } : undefined,
        partitionReassigner: deps.partitionReassigner ? {
            stop: () => deps.partitionReassigner!.stop(),
        } : undefined,
        queryConversionHandler: deps.queryConversionHandler ? {
            stop: () => deps.queryConversionHandler!.stop(),
        } : undefined,
        entryProcessorHandler: {
            dispose: () => deps.entryProcessorHandler.dispose(),
        },
        eventJournalService: deps.eventJournalService ? {
            dispose: () => deps.eventJournalService!.dispose(),
        } : undefined,

        // Search shutdown
        clusterSearchCoordinator: deps.clusterSearchCoordinator ? {
            destroy: () => deps.clusterSearchCoordinator!.destroy(),
        } : undefined,
        distributedSubCoordinator: deps.distributedSubCoordinator ? {
            destroy: () => deps.distributedSubCoordinator!.destroy(),
        } : undefined,
        searchCoordinator: {
            getEnabledMaps: () => deps.searchCoordinator.getEnabledMaps(),
            buildIndexFromEntries: (mapName, entries) => deps.searchCoordinator.buildIndexFromEntries(mapName, entries),
        },

        // Map access for backfill
        getMapAsync: (name) => deps.getMapAsync(name),
    });

    return { lifecycleManager };
}
