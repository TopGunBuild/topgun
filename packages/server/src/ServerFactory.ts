import { createServer as createHttpServer, Server as HttpServer } from 'http';
import { WebSocket } from 'ws';
import { ConsistencyLevel, PARTITION_COUNT } from '@topgunbuild/core';
import { ServerCoordinatorConfig, ServerCoordinator } from './ServerCoordinator';
import { ClusterManager } from './cluster/ClusterManager';
import { PartitionService } from './cluster/PartitionService';
import { TopicManager } from './topic/TopicManager';
import { logger } from './utils/logger';
import { validateJwtSecret } from './utils/validateConfig';
import { coalescingPresets } from './utils/coalescingPresets';
import { createCoreModule, createWorkersModule, createClusterModule, createStorageModule, createNetworkModule } from './modules';
import type { MetricsService } from './monitoring/MetricsService';
import { CounterHandler } from './handlers/CounterHandler';
import { EntryProcessorHandler } from './handlers/EntryProcessorHandler';
import { ConflictResolverHandler } from './handlers/ConflictResolverHandler';
import { EventJournalService } from './EventJournalService';
import { SearchCoordinator, ClusterSearchCoordinator } from './search';
import { DistributedSubscriptionCoordinator } from './subscriptions/DistributedSubscriptionCoordinator';
import { createDebugEndpoints } from './debug';
import { createBootstrapController, BootstrapController } from './bootstrap';
import { createSettingsController, SettingsController } from './settings';
import { DebugEndpoints } from './debug';

import {
    AuthHandler,
    ConnectionManager,
    OperationHandler,
    WebSocketHandler,
    BroadcastHandler,
    GCHandler,
    ClusterEventHandler,
    HeartbeatHandler,
    ClientMessageHandler,
    PersistenceHandler,
    OperationContextHandler,
    QueryConversionHandler,
    BatchProcessingHandler,
    WriteConcernHandler,
    createMessageRegistry,
    PartitionHandler,
    TopicHandler,
    LockHandler,
    CounterHandlerAdapter,
    ResolverHandler,
    JournalHandler,
    LwwSyncHandler,
    ORMapSyncHandler,
    EntryProcessorAdapter,
    SearchHandler,
    QueryHandler,
    LifecycleManager,
    DEFAULT_GC_AGE_MS,
} from './coordinator';

export class ServerFactory {
    static create(config: ServerCoordinatorConfig): ServerCoordinator {
        // Validate and handle JWT_SECRET
        const rawSecret = validateJwtSecret(config.jwtSecret, process.env.JWT_SECRET);
        const jwtSecret = rawSecret.replace(/\\n/g, '\n');

        // Create core module
        const core = createCoreModule({
            nodeId: config.nodeId,
            eventStripeCount: config.eventStripeCount,
            eventQueueCapacity: config.eventQueueCapacity,
            backpressureEnabled: config.backpressureEnabled,
            backpressureSyncFrequency: config.backpressureSyncFrequency,
            backpressureMaxPending: config.backpressureMaxPending,
            backpressureBackoffMs: config.backpressureBackoffMs,
            securityPolicies: config.securityPolicies,
        });

        const { hlc, metricsService, securityManager, eventExecutor, backpressure } = core;

        // Initialize write coalescing options
        const writeCoalescingEnabled = config.writeCoalescingEnabled ?? true;
        const preset = coalescingPresets[config.writeCoalescingPreset ?? 'highThroughput'];
        const writeCoalescingOptions = {
            maxBatchSize: config.writeCoalescingMaxBatch ?? preset.maxBatchSize,
            maxDelayMs: config.writeCoalescingMaxDelayMs ?? preset.maxDelayMs,
            maxBatchBytes: config.writeCoalescingMaxBytes ?? preset.maxBatchBytes,
        };

        // Initialize ConnectionManager
        const connectionManager = new ConnectionManager({
            hlc,
            writeCoalescingEnabled,
            writeCoalescingOptions,
        });

        // Create cluster module
        const clusterMod = createClusterModule(
            {
                nodeId: config.nodeId,
                host: config.host,
                clusterPort: config.clusterPort,
                peers: config.peers,
                resolvePeers: config.resolvePeers,
                discovery: config.discovery,
                serviceName: config.serviceName,
                discoveryInterval: config.discoveryInterval,
                clusterTls: config.clusterTls,
                replicationEnabled: config.replicationEnabled,
                defaultConsistency: config.defaultConsistency,
                replicationConfig: config.replicationConfig,
            },
            { hlc: core.hlc }
        );

        const { cluster, partitionService, replicationPipeline, lockManager, merkleTreeManager, partitionReassigner, readReplicaHandler, repairScheduler } = clusterMod;

        // Create storage module
        const storageMod = createStorageModule(
            {
                nodeId: config.nodeId,
                storage: config.storage,
                fullTextSearch: config.fullTextSearch,
                writeAckTimeout: config.writeAckTimeout,
            },
            {
                hlc: core.hlc,
                metricsService: core.metricsService,
                partitionService: clusterMod.partitionService,
            }
        );

        const { storageManager, queryRegistry, eventPayloadPool, taskletScheduler, writeAckManager } = storageMod;

        // Create workers module
        const workers = createWorkersModule({
            workerPoolEnabled: config.workerPoolEnabled,
            workerPoolConfig: config.workerPoolConfig,
        });

        const { workerPool, merkleWorker, crdtMergeWorker, serializationWorker } = workers;

        // Create network module (does NOT start listening)
        const network = createNetworkModule(
            {
                port: config.port,
                tls: config.tls,
                wsBacklog: config.wsBacklog,
                wsCompression: config.wsCompression,
                wsMaxPayload: config.wsMaxPayload ?? 64 * 1024 * 1024,
                maxConnections: config.maxConnections,
                serverTimeout: config.serverTimeout,
                keepAliveTimeout: config.keepAliveTimeout,
                headersTimeout: config.headersTimeout,
                maxConnectionsPerSecond: config.maxConnectionsPerSecond,
                maxPendingConnections: config.maxPendingConnections,
                socketNoDelay: true,
                socketKeepAlive: true,
                socketKeepAliveMs: 60000,
            },
            {} // No dependencies currently required
        );

        const { httpServer, wss, rateLimiter, rateLimitedLogger } = network;

        const authHandler = new AuthHandler({
            jwtSecret,
            onAuthSuccess: (_clientId, _principal) => {
                if (config.rateLimitingEnabled ?? true) {
                    rateLimiter.onConnectionEstablished();
                }
            },
        });

        // Reuse existing endpoint creation logic
        const debugEndpoints = createDebugEndpoints({
            enabled: config.debugEnabled ?? process.env.TOPGUN_DEBUG === 'true',
            getMaps: () => storageManager.getMaps(),
        });

        const bootstrapController = createBootstrapController({ jwtSecret });
        bootstrapController.setDataAccessors({
            getMaps: () => storageManager.getMaps(),
            getClusterStatus: () => ServerFactory.getClusterStatus(cluster, partitionService, connectionManager),
        });

        const settingsController = createSettingsController({ jwtSecret });
        settingsController.setOnSettingsChange((settings) => {
            if (settings.logLevel) {
                logger.level = settings.logLevel;
            }
            if (settings.rateLimits) {
                rateLimiter.updateConfig({
                    maxConnectionsPerSecond: settings.rateLimits.connections,
                });
            }
        });

        // Create metrics server (does NOT start listening yet)
        const metricsServer = config.metricsPort
            ? ServerFactory.createMetricsServer(
                bootstrapController,
                settingsController,
                debugEndpoints,
                metricsService
            )
            : undefined;

        const topicManager = new TopicManager({
            cluster,
            sendToClient: (clientId, message) => {
                const client = connectionManager.getClient(clientId);
                if (client && client.socket.readyState === WebSocket.OPEN) {
                    client.writer.write(message);
                }
            }
        });

        const counterHandler = new CounterHandler(config.nodeId);
        const entryProcessorHandler = new EntryProcessorHandler({ hlc });
        const conflictResolverHandler = new ConflictResolverHandler({ nodeId: config.nodeId });

        let eventJournalService: EventJournalService | undefined;
        if (config.eventJournalEnabled && config.storage && 'pool' in (config.storage as any)) {
            eventJournalService = new EventJournalService({
                capacity: 10000,
                ttlMs: 0,
                persistent: true,
                pool: (config.storage as any).pool,
                ...config.eventJournalConfig,
            });
        }

        const searchCoordinator = new SearchCoordinator();
        if (config.fullTextSearch) {
            for (const [mapName, ftsConfig] of Object.entries(config.fullTextSearch)) {
                searchCoordinator.enableSearch(mapName, ftsConfig);
            }
        }

        searchCoordinator.setNodeId(config.nodeId);

        const clusterSearchCoordinator = new ClusterSearchCoordinator(
            cluster,
            partitionService,
            searchCoordinator,
            config.distributedSearch,
            metricsService
        );

        const distributedSubCoordinator = new DistributedSubscriptionCoordinator(
            cluster,
            queryRegistry,
            searchCoordinator,
            undefined,
            metricsService
        );

        queryRegistry.setClusterManager(cluster, config.nodeId);
        queryRegistry.setMapGetter((name) => storageManager.getMap(name));

        const pendingBatchOperations = new Set<Promise<void>>();

        // Initialize Independent Handlers
        const broadcastHandler = new BroadcastHandler({
            connectionManager,
            securityManager: {
                filterObject: securityManager.filterObject.bind(securityManager),
            },
            queryRegistry: {
                getSubscribedClientIds: queryRegistry.getSubscribedClientIds.bind(queryRegistry),
            },
            metricsService: {
                incEventsRouted: metricsService.incEventsRouted.bind(metricsService),
                incEventsFilteredBySubscription: metricsService.incEventsFilteredBySubscription.bind(metricsService),
                recordSubscribersPerEvent: metricsService.recordSubscribersPerEvent.bind(metricsService),
            },
            hlc,
        });

        const heartbeatHandler = new HeartbeatHandler({
            connectionManager,
        });

        const clientMessageHandler = new ClientMessageHandler({
            connectionManager,
            queryRegistry,
            hlc,
        });

        const persistenceHandler = new PersistenceHandler({
            storage: config.storage || null,
            getMap: (name) => storageManager.getMap(name),
        });

        const operationContextHandler = new OperationContextHandler({
            connectionManager,
            interceptors: config.interceptors || [],
            cluster: {
                config: { nodeId: config.nodeId },
                send: (nodeId, type, payload) => cluster.send(nodeId, type, payload),
            },
        });

        // Initialize OperationHandler (Full Logic moved here)
        const operationHandler = new OperationHandler({
            nodeId: config.nodeId,
            hlc,
            metricsService,
            securityManager: {
                checkPermission: securityManager.checkPermission.bind(securityManager),
            },
            storageManager,
            conflictResolverHandler: {
                hasResolvers: (mapName) => conflictResolverHandler.hasResolvers(mapName),
                mergeWithResolver: (map, mapName, key, record, nodeId) => conflictResolverHandler.mergeWithResolver(map, mapName, key, record, nodeId),
            },
            queryRegistry: {
                processChange: queryRegistry.processChange.bind(queryRegistry),
            },
            eventJournalService: eventJournalService ? {
                append: eventJournalService.append.bind(eventJournalService),
            } : undefined,
            merkleTreeManager: merkleTreeManager ? {
                updateRecord: (pid, key, rec) => merkleTreeManager.updateRecord(pid, key, rec),
            } : undefined,
            partitionService: {
                getPartitionId: (key) => partitionService.getPartitionId(key),
                getOwner: (key) => partitionService.getOwner(key),
                isLocalOwner: (key) => partitionService.isLocalOwner(key),
            },
            searchCoordinator: {
                isSearchEnabled: (mapName) => searchCoordinator.isSearchEnabled(mapName),
                onDataChange: (mapName, key, value, changeType) => searchCoordinator.onDataChange(mapName, key, value, changeType as any),
            },
            storage: config.storage || null,
            replicationPipeline: replicationPipeline ? {
                replicate: async (op, opId, key) => {
                    await replicationPipeline!.replicate(op, opId, key);
                },
            } : undefined,
            broadcastHandler,
            operationContextHandler,
            backpressure: {
                registerPending: () => backpressure.registerPending(),
                waitForCapacity: () => backpressure.waitForCapacity(),
                shouldForceSync: () => backpressure.shouldForceSync(),
            },
        });

        const lockHandler = new LockHandler({
            lockManager: {
                acquire: (name, clientId, requestId, ttl) => lockManager.acquire(name, clientId, requestId, ttl),
                release: (name, clientId, fencingToken) => lockManager.release(name, clientId, fencingToken),
            },
            partitionService: {
                isLocalOwner: (key) => partitionService.isLocalOwner(key),
                getOwner: (key) => partitionService.getOwner(key),
            },
            cluster: {
                getMembers: () => cluster.getMembers(),
                send: (nodeId, type, payload) => cluster.send(nodeId, type, payload),
                config: { nodeId: config.nodeId },
            },
            securityManager: {
                checkPermission: securityManager.checkPermission.bind(securityManager),
            },
        });

        const topicHandler = new TopicHandler({
            topicManager: {
                subscribe: (clientId, topic) => topicManager.subscribe(clientId, topic),
                unsubscribe: (clientId, topic) => topicManager.unsubscribe(clientId, topic),
                publish: (topic, data, senderId) => topicManager.publish(topic, data, senderId),
            },
            securityManager: {
                checkPermission: securityManager.checkPermission.bind(securityManager),
            },
        });

        const partitionHandler = new PartitionHandler({
            partitionService: {
                getPartitionMap: () => partitionService.getPartitionMap(),
            },
        });

        const searchHandler = new SearchHandler({
            searchCoordinator: {
                isSearchEnabled: (mapName) => searchCoordinator.isSearchEnabled(mapName),
                search: (mapName, query, options) => searchCoordinator.search(mapName, query, options),
                subscribe: (clientId, subscriptionId, mapName, query, options) => searchCoordinator.subscribe(clientId, subscriptionId, mapName, query, options),
                unsubscribe: (subscriptionId) => searchCoordinator.unsubscribe(subscriptionId),
            },
            clusterSearchCoordinator,
            distributedSubCoordinator,
            cluster: {
                getMembers: () => cluster.getMembers(),
            },
            securityManager: {
                checkPermission: securityManager.checkPermission.bind(securityManager),
            },
        });

        const journalSubscriptions = new Map();
        const journalHandler = new JournalHandler({
            eventJournalService: eventJournalService,
            journalSubscriptions,
            getClient: (clientId) => connectionManager.getClient(clientId),
        });

        // Initialize GCHandler (late binding for broadcast callback)
        const gcHandler = new GCHandler({
            storageManager,
            connectionManager,
            cluster: {
                getMembers: () => cluster.getMembers(),
                send: (nodeId: string, type: any, payload: any) => cluster.send(nodeId, type, payload),
                isLocal: (id: string) => cluster.isLocal(id),
                config: { nodeId: config.nodeId },
            },
            partitionService: {
                isRelated: (key) => partitionService.isRelated(key),
                getPartitionId: (key) => partitionService.getPartitionId(key),
            },
            replicationPipeline: replicationPipeline ? {
                replicate: async (op, opId, key) => {
                    await replicationPipeline!.replicate(op, opId, key);
                },
            } : undefined,
            merkleTreeManager: merkleTreeManager ? {
                updateRecord: (pid, key, rec) => merkleTreeManager.updateRecord(pid, key, rec),
            } : undefined,
            queryRegistry: {
                processChange: queryRegistry.processChange.bind(queryRegistry),
            },
            hlc,
            storage: config.storage || undefined,
            // broadcast callback will be set via late binding in ServerCoordinator
            metricsService: {
                incOp: (op: any, mapName: string) => metricsService.incOp(op, mapName),
            },
        });

        // Shared pending cluster queries map (used by QueryHandler and QueryConversionHandler)
        const pendingClusterQueries = new Map();

        // Initialize QueryConversionHandler (no late binding needed)
        const queryConversionHandler = new QueryConversionHandler({
            getMapAsync: (mapName: string, typeHint?: 'LWW' | 'OR') => storageManager.getMapAsync(mapName, typeHint),
            pendingClusterQueries,
            queryRegistry,
            securityManager: {
                filterObject: (value: any, principal: any, mapName: string) => securityManager.filterObject(value, principal, mapName),
            },
        });

        // Initialize BatchProcessingHandler (late binding for broadcast callbacks)
        const batchProcessingHandler = new BatchProcessingHandler({
            backpressure: {
                shouldForceSync: () => backpressure.shouldForceSync(),
                registerPending: () => backpressure.registerPending(),
                waitForCapacity: () => backpressure.waitForCapacity(),
                completePending: () => backpressure.completePending(),
                getPendingOps: () => backpressure.getPendingOps(),
            },
            partitionService: {
                isLocalOwner: (key) => partitionService.isLocalOwner(key),
                getOwner: (key) => partitionService.getOwner(key),
            },
            cluster: {
                sendToNode: (nodeId, message) => cluster.send(nodeId, message.type, message.payload),
            },
            metricsService: {
                incBackpressureSyncForced: () => metricsService.incBackpressureSyncForced(),
                incBackpressureWaits: () => metricsService.incBackpressureWaits(),
                incBackpressureTimeouts: () => metricsService.incBackpressureTimeouts(),
                setBackpressurePendingOps: (count) => metricsService.setBackpressurePendingOps(count),
            },
            replicationPipeline: replicationPipeline ? {
                replicate: async (op, opId, key) => {
                    await replicationPipeline!.replicate(op, opId, key);
                },
            } : undefined,
            // broadcast callbacks will be set via late binding in ServerCoordinator
            buildOpContext: operationContextHandler.buildOpContext.bind(operationContextHandler),
            runBeforeInterceptors: operationContextHandler.runBeforeInterceptors.bind(operationContextHandler),
            runAfterInterceptors: operationContextHandler.runAfterInterceptors.bind(operationContextHandler),
            applyOpToMap: operationHandler.applyOpToMap.bind(operationHandler),
        });

        // Initialize WriteConcernHandler (no late binding needed)
        const writeConcernHandler = new WriteConcernHandler({
            backpressure: {
                shouldForceSync: () => backpressure.shouldForceSync(),
                registerPending: () => backpressure.registerPending(),
                waitForCapacity: () => backpressure.waitForCapacity(),
                completePending: () => backpressure.completePending(),
                getPendingOps: () => backpressure.getPendingOps(),
            },
            partitionService: {
                isLocalOwner: (key: string) => partitionService.isLocalOwner(key),
                getOwner: (key: string) => partitionService.getOwner(key),
            },
            cluster: {
                sendToNode: (nodeId: string, message: any) => cluster.send(nodeId, message.type, message.payload),
            },
            metricsService: {
                incBackpressureSyncForced: () => metricsService.incBackpressureSyncForced(),
                incBackpressureWaits: () => metricsService.incBackpressureWaits(),
                incBackpressureTimeouts: () => metricsService.incBackpressureTimeouts(),
                setBackpressurePendingOps: (count: number) => metricsService.setBackpressurePendingOps(count),
            },
            writeAckManager: {
                notifyLevel: (opId: string, level: any) => writeAckManager.notifyLevel(opId, level),
                failPending: (opId: string, error: string) => writeAckManager.failPending(opId, error),
            },
            storage: config.storage || null,
            // broadcast callbacks will be set via late binding in ServerCoordinator
            broadcastBatch: () => {},
            broadcastBatchSync: async () => {},
            buildOpContext: operationContextHandler.buildOpContext.bind(operationContextHandler),
            runBeforeInterceptors: operationContextHandler.runBeforeInterceptors.bind(operationContextHandler),
            runAfterInterceptors: operationContextHandler.runAfterInterceptors.bind(operationContextHandler),
            applyOpToMap: operationHandler.applyOpToMap.bind(operationHandler),
            persistOpSync: persistenceHandler.persistOpSync.bind(persistenceHandler),
            persistOpAsync: persistenceHandler.persistOpAsync.bind(persistenceHandler),
        });

        // Create local handlers for MessageRegistry
        const queryHandler = new QueryHandler({
            securityManager: {
                checkPermission: (principal: any, resource: string, action: any) => securityManager.checkPermission(principal, resource, action),
                filterObject: (value: any, principal: any, mapName: string) => securityManager.filterObject(value, principal, mapName),
            },
            metricsService: {
                incOp: (op: any, mapName: string) => metricsService.incOp(op, mapName),
            },
            queryRegistry: {
                unregister: (queryId: string) => queryRegistry.unregister(queryId),
            },
            distributedSubCoordinator: distributedSubCoordinator,
            cluster: {
                getMembers: () => cluster.getMembers(),
                isLocal: (id: string) => cluster.isLocal(id),
                send: (nodeId: string, type: any, payload: any) => cluster.send(nodeId, type, payload),
                config: { nodeId: config.nodeId },
            },
            executeLocalQuery: (mapName: string, query: any) => queryConversionHandler.executeLocalQuery(mapName, query),
            finalizeClusterQuery: (reqId: string, timeout?: boolean) => queryConversionHandler.finalizeClusterQuery(reqId, timeout),
            pendingClusterQueries, // Shared with QueryConversionHandler
            readReplicaHandler: readReplicaHandler,
            ConsistencyLevel: { EVENTUAL: ConsistencyLevel.EVENTUAL },
        });

        const counterHandlerAdapter = new CounterHandlerAdapter({
            counterHandler: {
                handleCounterRequest: (clientId: string, name: string) => counterHandler.handleCounterRequest(clientId, name),
                handleCounterSync: (clientId: string, name: string, state: any) => counterHandler.handleCounterSync(clientId, name, state),
            },
            getClient: (clientId: string) => connectionManager.getClient(clientId),
        });

        const resolverHandler = new ResolverHandler({
            conflictResolverHandler: {
                registerResolver: (mapName: string, resolver: any, clientId: string) => conflictResolverHandler.registerResolver(mapName, resolver, clientId),
                unregisterResolver: (mapName: string, resolverName: string, clientId: string) => conflictResolverHandler.unregisterResolver(mapName, resolverName, clientId),
                listResolvers: (mapName?: string) => conflictResolverHandler.listResolvers(mapName),
            },
            securityManager: {
                checkPermission: (principal: any, resource: string, action: any) => securityManager.checkPermission(principal, resource, action),
            },
        });

        const lwwSyncHandler = new LwwSyncHandler({
            getMapAsync: (name: string, typeHint?: 'LWW' | 'OR') => storageManager.getMapAsync(name, typeHint),
            hlc,
            securityManager: {
                checkPermission: (principal: any, resource: string, action: any) => securityManager.checkPermission(principal, resource, action),
            },
            metricsService: {
                incOp: (op: any, mapName: string) => metricsService.incOp(op, mapName),
            },
            gcAgeMs: DEFAULT_GC_AGE_MS,
        });

        const orMapSyncHandler = new ORMapSyncHandler({
            getMapAsync: (name: string, typeHint?: 'LWW' | 'OR') => storageManager.getMapAsync(name, typeHint),
            hlc,
            securityManager: {
                checkPermission: (principal: any, resource: string, action: any) => securityManager.checkPermission(principal, resource, action),
            },
            metricsService: {
                incOp: (op: any, mapName: string) => metricsService.incOp(op, mapName),
            },
            storage: config.storage,
            broadcast: (message: any, excludeClientId?: string) => broadcastHandler.broadcast(message, excludeClientId),
            gcAgeMs: DEFAULT_GC_AGE_MS,
        });

        const entryProcessorAdapter = new EntryProcessorAdapter({
            entryProcessorHandler: {
                executeOnKey: (map: any, key: string, processor: any) => entryProcessorHandler.executeOnKey(map, key, processor),
                executeOnKeys: (map: any, keys: string[], processor: any) => entryProcessorHandler.executeOnKeys(map, keys, processor),
            },
            getMap: (name: string) => storageManager.getMap(name),
            securityManager: {
                checkPermission: (principal: any, resource: string, action: any) => securityManager.checkPermission(principal, resource, action),
            },
            queryRegistry: {
                processChange: (mapName: string, map: any, key: string, record: any, oldValue: any) => queryRegistry.processChange(mapName, map, key, record, oldValue),
            },
        });

        // Create MessageRegistry with all handlers
        const messageRegistry = createMessageRegistry({
            // CRDT operations
            onClientOp: (client, msg) => operationHandler.processClientOp(client, msg.payload),
            onOpBatch: async (client, msg) => {
                const ops = msg.payload.ops;
                await batchProcessingHandler.processBatchAsync(ops, client.id);

                // Send OP_ACK with lastId from the batch
                if (ops.length > 0) {
                    const lastId = ops[ops.length - 1].id;
                    client.writer.write({
                        type: 'OP_ACK',
                        payload: { lastId }
                    });
                }
            },
            // Query operations
            onQuerySub: (client, msg) => queryHandler.handleQuerySub(client, msg),
            onQueryUnsub: (client, msg) => queryHandler.handleQueryUnsub(client, msg),
            // LWW Sync protocol
            onSyncInit: (client, msg) => lwwSyncHandler.handleSyncInit(client, msg),
            onMerkleReqBucket: (client, msg) => lwwSyncHandler.handleMerkleReqBucket(client, msg),
            // ORMap Sync protocol
            onORMapSyncInit: (client, msg) => orMapSyncHandler.handleORMapSyncInit(client, msg),
            onORMapMerkleReqBucket: (client, msg) => orMapSyncHandler.handleORMapMerkleReqBucket(client, msg),
            onORMapDiffRequest: (client, msg) => orMapSyncHandler.handleORMapDiffRequest(client, msg),
            onORMapPushDiff: (client, msg) => orMapSyncHandler.handleORMapPushDiff(client, msg),
            // Lock operations
            onLockRequest: (client, msg) => lockHandler.handleLockRequest(client, msg),
            onLockRelease: (client, msg) => lockHandler.handleLockRelease(client, msg),
            // Topic operations
            onTopicSub: (client, msg) => topicHandler.handleTopicSub(client, msg),
            onTopicUnsub: (client, msg) => topicHandler.handleTopicUnsub(client, msg),
            onTopicPub: (client, msg) => topicHandler.handleTopicPub(client, msg),
            // PN Counter operations
            onCounterRequest: (client, msg) => counterHandlerAdapter.handleCounterRequest(client, msg),
            onCounterSync: (client, msg) => counterHandlerAdapter.handleCounterSync(client, msg),
            // Entry processor operations
            onEntryProcess: (client, msg) => entryProcessorAdapter.handleEntryProcess(client, msg),
            onEntryProcessBatch: (client, msg) => entryProcessorAdapter.handleEntryProcessBatch(client, msg),
            // Conflict resolver operations
            onRegisterResolver: (client, msg) => resolverHandler.handleRegisterResolver(client, msg),
            onUnregisterResolver: (client, msg) => resolverHandler.handleUnregisterResolver(client, msg),
            onListResolvers: (client, msg) => resolverHandler.handleListResolvers(client, msg),
            // Partition operations
            onPartitionMapRequest: (client, msg) => partitionHandler.handlePartitionMapRequest(client, msg),
            // Full-text search operations
            onSearch: (client, msg) => searchHandler.handleSearch(client, msg),
            onSearchSub: (client, msg) => searchHandler.handleSearchSub(client, msg),
            onSearchUnsub: (client, msg) => searchHandler.handleSearchUnsub(client, msg),
            // Event journal operations
            onJournalSubscribe: (client, msg) => journalHandler.handleJournalSubscribe(client, msg),
            onJournalUnsubscribe: (client, msg) => journalHandler.handleJournalUnsubscribe(client, msg),
            onJournalRead: (client, msg) => journalHandler.handleJournalRead(client, msg),
        });

        // Create LifecycleManager with direct dependencies
        const lifecycleManager = new LifecycleManager({
            nodeId: config.nodeId,
            httpServer,
            metricsServer,
            wss,
            metricsService: {
                destroy: () => metricsService.destroy(),
            },
            eventExecutor: {
                shutdown: (wait) => eventExecutor.shutdown(wait),
            },
            connectionManager: {
                getClientCount: () => connectionManager.getClientCount(),
                getClients: () => connectionManager.getClients(),
            },
            cluster: {
                getMembers: () => cluster.getMembers(),
                send: (nodeId, type, payload) => cluster.send(nodeId, type, payload),
                stop: () => cluster.stop(),
            },
            partitionService: {
                getPartitionMap: () => partitionService.getPartitionMap(),
            },
            replicationPipeline: replicationPipeline ? {
                getTotalPending: () => replicationPipeline!.getTotalPending(),
                close: () => replicationPipeline!.close(),
            } : undefined,
            workerPool: workerPool ? {
                shutdown: (timeout) => workerPool!.shutdown(timeout),
            } : undefined,
            storage: config.storage ? {
                close: () => config.storage!.close(),
            } : undefined,
            gcHandler: {
                stop: () => gcHandler.stop(),
            },
            heartbeatHandler: {
                stop: () => heartbeatHandler.stop(),
            },
            lockManager: {
                stop: () => lockManager.stop(),
            },
            repairScheduler: repairScheduler ? {
                stop: () => repairScheduler!.stop(),
            } : undefined,
            partitionReassigner: partitionReassigner ? {
                stop: () => partitionReassigner!.stop(),
            } : undefined,
            taskletScheduler: {
                shutdown: () => taskletScheduler.shutdown(),
            },
            writeAckManager: {
                shutdown: () => writeAckManager.shutdown(),
            },
            entryProcessorHandler: {
                dispose: () => entryProcessorHandler.dispose(),
            },
            eventJournalService: eventJournalService ? {
                dispose: () => eventJournalService!.dispose(),
            } : undefined,
            eventPayloadPool: {
                clear: () => eventPayloadPool.clear(),
            },
            clusterSearchCoordinator: clusterSearchCoordinator ? {
                destroy: () => clusterSearchCoordinator!.destroy(),
            } : undefined,
            distributedSubCoordinator: distributedSubCoordinator ? {
                destroy: () => distributedSubCoordinator!.destroy(),
            } : undefined,
            queryConversionHandler: {
                stop: () => queryConversionHandler.stop(),
            },
            searchCoordinator: {
                getEnabledMaps: () => searchCoordinator.getEnabledMaps(),
                buildIndexFromEntries: (mapName, entries) => searchCoordinator.buildIndexFromEntries(mapName, entries),
            },
            getMapAsync: (name) => storageManager.getMapAsync(name),
        });

        // Initialize WebSocket Handler (Phase 1)
        const webSocketHandler = new WebSocketHandler({
            nodeId: config.nodeId,
            rateLimitingEnabled: config.rateLimitingEnabled ?? true,
            writeCoalescingEnabled: writeCoalescingEnabled,
            writeCoalescingOptions,
            interceptors: config.interceptors || [],
            rateLimiter: {
                shouldAccept: () => rateLimiter.shouldAccept(),
                onConnectionAttempt: () => rateLimiter.onConnectionAttempt(),
                onConnectionRejected: () => rateLimiter.onConnectionRejected(),
                onPendingConnectionFailed: () => rateLimiter.onPendingConnectionFailed(),
            },
            metricsService: {
                incConnectionsRejected: () => metricsService.incConnectionsRejected(),
                incConnectionsAccepted: () => metricsService.incConnectionsAccepted(),
                setConnectedClients: (count) => metricsService.setConnectedClients(count),
            },
            connectionManager,
            authHandler,
            rateLimitedLogger,
            queryRegistry: {
                unregister: (subId) => queryRegistry.unregister(subId),
            },
            lockManager: {
                handleClientDisconnect: (clientId) => lockManager.handleClientDisconnect(clientId),
            },
            topicManager: {
                unsubscribeAll: (clientId) => topicManager.unsubscribeAll(clientId),
            },
            counterHandler: {
                unsubscribeAll: (clientId) => counterHandler.unsubscribeAll(clientId),
            },
            searchCoordinator: {
                unsubscribeClient: (clientId) => searchCoordinator.unsubscribeClient(clientId),
            },
            distributedSubCoordinator: distributedSubCoordinator ? {
                unsubscribeClient: (socket) => distributedSubCoordinator.unsubscribeClient(socket),
            } : undefined,
            cluster: {
                getMembers: () => cluster.getMembers(),
                isLocal: (id) => cluster.isLocal(id),
                send: (nodeId, type, payload) => cluster.send(nodeId, type, payload),
                config: { nodeId: config.nodeId },
            },
            heartbeatHandler,
            clientMessageHandler,
        });

        // Assemble ServerCoordinator
        const coordinator = new ServerCoordinator(config, {
            hlc,
            metricsService,
            securityManager,
            eventExecutor,
            backpressure,
            writeCoalescingOptions,
            connectionManager,
            cluster,
            partitionService,
            storageManager,
            queryRegistry,
            eventPayloadPool,
            taskletScheduler,
            writeAckManager,
            rateLimiter,
            authHandler,
            rateLimitedLogger,
            workerPool,
            merkleWorker,
            crdtMergeWorker,
            serializationWorker,
            httpServer,
            debugEndpoints,
            bootstrapController,
            settingsController,
            metricsServer,
            wss,
            replicationPipeline,
            lockManager,
            topicManager,
            counterHandler,
            entryProcessorHandler,
            conflictResolverHandler,
            eventJournalService,
            partitionReassigner,
            readReplicaHandler,
            merkleTreeManager,
            repairScheduler,
            searchCoordinator,
            clusterSearchCoordinator,
            distributedSubCoordinator,
            pendingBatchOperations,
            jwtSecret,
            broadcastHandler,
            heartbeatHandler,
            persistenceHandler,
            operationContextHandler,
            lockHandler,
            topicHandler,
            partitionHandler,
            searchHandler,
            journalHandler,
            operationHandler,
            webSocketHandler,
            clientMessageHandler,
            gcHandler,
            queryConversionHandler,
            batchProcessingHandler,
            writeConcernHandler,
            messageRegistry,
            lifecycleManager,
        });

        // DEFERRED STARTUP - now safe to listen
        network.start();
        if (metricsServer && config.metricsPort) {
            metricsServer.listen(config.metricsPort, () => {
                logger.info({ port: config.metricsPort }, 'Metrics server listening');
            });
        }

        return coordinator;
    }

    private static createMetricsServer(
        bootstrap: BootstrapController,
        settings: SettingsController,
        debug: DebugEndpoints,
        metrics: MetricsService
    ): HttpServer {
        // Create server but do NOT start listening (deferred startup)
        const server = createHttpServer(async (req, res) => {
            const bootstrapHandled = await bootstrap.handle(req, res);
            if (bootstrapHandled) return;

            const url = req.url || '';
            if (url.startsWith('/api/admin/settings')) {
                const settingsHandled = await settings.handle(req, res);
                if (settingsHandled) return;
            }

            const debugHandled = await debug.handle(req, res);
            if (debugHandled) return;

            if (req.url === '/metrics') {
                try {
                    res.setHeader('Content-Type', metrics.getContentType());
                    res.end(await metrics.getMetrics());
                } catch (err) {
                    res.statusCode = 500;
                    res.end('Internal Server Error');
                }
            } else {
                res.statusCode = 404;
                res.end();
            }
        });

        return server;
    }

    private static getClusterStatus(cluster: ClusterManager, partitionService: PartitionService, connectionManager: ConnectionManager) {
        // Logic extracted from ServerCoordinator constructor
        const memberIds = cluster.getMembers();
        const nodes = memberIds.map(nodeId => {
            let partitionCount = 0;
            for (let i = 0; i < PARTITION_COUNT; i++) {
                if (partitionService.getPartitionOwner(i) === nodeId) partitionCount++;
            }

            return {
                id: nodeId,
                address: nodeId,
                status: 'healthy' as const,
                partitions: Array.from({ length: partitionCount }, (_, i) => i),
                connections: connectionManager.getClientCount(),
                memory: { used: process.memoryUsage().heapUsed, total: process.memoryUsage().heapTotal },
                uptime: process.uptime(),
            };
        });

        const partitions: { id: number; owner: string; replicas: string[] }[] = [];
        for (let i = 0; i < PARTITION_COUNT; i++) {
            const owner = partitionService.getPartitionOwner(i);
            const backups = partitionService.getBackups(i);
            partitions.push({
                id: i,
                owner: owner || 'unknown',
                replicas: backups,
            });
        }

        return {
            nodes,
            partitions,
            isRebalancing: partitionService.getMigrationStatus() !== null,
        };
    }
}
