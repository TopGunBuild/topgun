import { createServer as createHttpServer, Server as HttpServer } from 'http';
import { createServer as createHttpsServer, Server as HttpsServer } from 'https';
import { readFileSync } from 'fs';
import * as net from 'net';
import { WebSocketServer, WebSocket } from 'ws';
import * as crypto from 'crypto';
import { HLC, ConsistencyLevel, DEFAULT_REPLICATION_CONFIG, PARTITION_COUNT } from '@topgunbuild/core';
import { ServerCoordinatorConfig, ServerCoordinator } from './ServerCoordinator';
import { ClusterManager } from './cluster/ClusterManager';
import { PartitionService } from './cluster/PartitionService';
import { ReplicationPipeline } from './cluster/ReplicationPipeline';
import { PartitionReassigner } from './cluster/PartitionReassigner';
import { ReadReplicaHandler } from './cluster/ReadReplicaHandler';
import { MerkleTreeManager } from './cluster/MerkleTreeManager';
import { RepairScheduler } from './cluster/RepairScheduler';
import { LockManager } from './cluster/LockManager';
import { TopicManager } from './topic/TopicManager';
import { SecurityManager } from './security/SecurityManager';
import { logger } from './utils/logger';
import { validateJwtSecret } from './utils/validateConfig';
import { MetricsService } from './monitoring/MetricsService';
import { SystemManager } from './system/SystemManager';
import { StripedEventExecutor } from './utils/StripedEventExecutor';
import { BackpressureRegulator } from './utils/BackpressureRegulator';
import { CoalescingWriter } from './utils/CoalescingWriter';
import { coalescingPresets } from './utils/coalescingPresets';
import { ConnectionRateLimiter } from './utils/ConnectionRateLimiter';
import { RateLimitedLogger } from './utils/RateLimitedLogger';
import { WorkerPool, MerkleWorker, CRDTMergeWorker, SerializationWorker } from './workers';
import { createEventPayloadPool } from './memory';
import { TaskletScheduler } from './tasklet';
import { WriteAckManager } from './ack/WriteAckManager';
import { CounterHandler } from './handlers/CounterHandler';
import { EntryProcessorHandler } from './handlers/EntryProcessorHandler';
import { ConflictResolverHandler } from './handlers/ConflictResolverHandler';
import { EventJournalService } from './EventJournalService';
import { SearchCoordinator, ClusterSearchCoordinator } from './search';
import { DistributedSubscriptionCoordinator } from './subscriptions/DistributedSubscriptionCoordinator';
import { createDebugEndpoints } from './debug';
import { createBootstrapController, BootstrapController } from './bootstrap';
import { createSettingsController, SettingsController } from './settings';
import { QueryRegistry } from './query/QueryRegistry';
import { DebugEndpoints } from './debug';
import { ServerOptions as HttpsServerOptions } from 'https';
import type { MergeRejection } from '@topgunbuild/core';

import {
    AuthHandler,
    ConnectionManager,
    StorageManager,
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
} from './coordinator';

export class ServerFactory {
    static create(config: ServerCoordinatorConfig): ServerCoordinator {
        // Validate and handle JWT_SECRET
        const rawSecret = validateJwtSecret(config.jwtSecret, process.env.JWT_SECRET);
        const jwtSecret = rawSecret.replace(/\\n/g, '\n');

        const hlc = new HLC(config.nodeId);
        const metricsService = new MetricsService();
        const securityManager = new SecurityManager(config.securityPolicies || []);

        // Initialize bounded event queue executor
        const eventExecutor = new StripedEventExecutor({
            stripeCount: config.eventStripeCount ?? 4,
            queueCapacity: config.eventQueueCapacity ?? 10000,
            name: `${config.nodeId}-event-executor`,
            onReject: (task) => {
                logger.warn({ nodeId: config.nodeId, key: task.key }, 'Event task rejected due to queue capacity');
                metricsService.incEventQueueRejected();
            }
        });

        // Initialize backpressure regulator
        const backpressure = new BackpressureRegulator({
            syncFrequency: config.backpressureSyncFrequency ?? 100,
            maxPendingOps: config.backpressureMaxPending ?? 1000,
            backoffTimeoutMs: config.backpressureBackoffMs ?? 5000,
            enabled: config.backpressureEnabled ?? true
        });

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

        // Initialize Cluster Logic
        const clusterPort = config.clusterPort ?? 0;
        const peers = config.resolvePeers ? config.resolvePeers() : (config.peers || []);

        const cluster = new ClusterManager({
            nodeId: config.nodeId,
            host: config.host || 'localhost',
            port: clusterPort,
            peers,
            discovery: config.discovery,
            serviceName: config.serviceName,
            discoveryInterval: config.discoveryInterval,
            tls: config.clusterTls
        });

        const partitionService = new PartitionService(cluster);

        // Initialize Query Registry (Moved up)
        const queryRegistry = new QueryRegistry();

        // Initialize StorageManager
        const storageManager = new StorageManager({
            nodeId: config.nodeId,
            hlc,
            storage: config.storage,
            fullTextSearch: config.fullTextSearch,
            isRelatedKey: (key: string) => partitionService.isRelated(key) ?? true,
            onMapLoaded: (mapName: string, _recordCount: number) => {
                const map = storageManager.getMaps().get(mapName);
                if (map) {
                    queryRegistry.refreshSubscriptions(mapName, map);
                    const mapSize = (map as any).totalRecords ?? map.size;
                    metricsService.setMapSize(mapName, mapSize);
                }
            },
        });

        const eventPayloadPool = createEventPayloadPool({ maxSize: 4096, initialSize: 128 });

        const taskletScheduler = new TaskletScheduler({
            defaultTimeBudgetMs: 5,
            maxConcurrent: 20,
        });

        const writeAckManager = new WriteAckManager({
            defaultTimeout: config.writeAckTimeout ?? 5000,
        });

        const rateLimits = {
            maxConnectionsPerSecond: config.maxConnectionsPerSecond ?? 100,
            maxPendingConnections: config.maxPendingConnections ?? 1000,
        };

        const rateLimiter = new ConnectionRateLimiter({
            ...rateLimits,
            cooldownMs: 1000,
        });

        const authHandler = new AuthHandler({
            jwtSecret,
            onAuthSuccess: (_clientId, _principal) => {
                if (config.rateLimitingEnabled ?? true) {
                    rateLimiter.onConnectionEstablished();
                }
            },
        });

        const rateLimitedLogger = new RateLimitedLogger({ windowMs: 10000, maxPerWindow: 5 });

        let workerPool: WorkerPool | undefined;
        let merkleWorker: MerkleWorker | undefined;
        let crdtMergeWorker: CRDTMergeWorker | undefined;
        let serializationWorker: SerializationWorker | undefined;

        if (config.workerPoolEnabled) {
            workerPool = new WorkerPool({
                minWorkers: config.workerPoolConfig?.minWorkers ?? 2,
                maxWorkers: config.workerPoolConfig?.maxWorkers,
                taskTimeout: config.workerPoolConfig?.taskTimeout ?? 5000,
                idleTimeout: config.workerPoolConfig?.idleTimeout ?? 30000,
                autoRestart: config.workerPoolConfig?.autoRestart ?? true,
            });
            merkleWorker = new MerkleWorker(workerPool);
            crdtMergeWorker = new CRDTMergeWorker(workerPool);
            serializationWorker = new SerializationWorker(workerPool);
        }

        // HTTP Server Setup
        let httpServer: HttpServer | HttpsServer;
        if (config.tls?.enabled) {
            const tlsOptions = ServerFactory.buildTLSOptions(config.tls);
            httpServer = createHttpsServer(tlsOptions, (_req, res) => {
                res.writeHead(200);
                res.end('TopGun Server Running (Secure)');
            });
        } else {
            httpServer = createHttpServer((_req, res) => {
                res.writeHead(200);
                res.end('TopGun Server Running');
            });
        }

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

        const metricsServer = ServerFactory.createMetricsServer(
            config.metricsPort ?? 9090,
            bootstrapController,
            settingsController,
            debugEndpoints,
            metricsService
        );

        const wss = new WebSocketServer({
            server: httpServer,
            backlog: config.wsBacklog ?? 511,
            perMessageDeflate: config.wsCompression ?? false,
            maxPayload: config.wsMaxPayload ?? 64 * 1024 * 1024,
            skipUTF8Validation: true,
        });

        // HTTP Server Limits
        httpServer.maxConnections = config.maxConnections ?? 10000;
        httpServer.timeout = config.serverTimeout ?? 120000;
        httpServer.keepAliveTimeout = config.keepAliveTimeout ?? 5000;
        httpServer.headersTimeout = config.headersTimeout ?? 60000;

        httpServer.on('connection', (socket: net.Socket) => {
            socket.setNoDelay(true);
            socket.setKeepAlive(true, 60000);
        });

        // Replication Pipeline
        let replicationPipeline: ReplicationPipeline | undefined;
        if (config.replicationEnabled !== false) {
            replicationPipeline = new ReplicationPipeline(
                cluster,
                partitionService,
                {
                    ...DEFAULT_REPLICATION_CONFIG,
                    defaultConsistency: config.defaultConsistency ?? ConsistencyLevel.EVENTUAL,
                    ...config.replicationConfig,
                }
            );
        }

        const lockManager = new LockManager();
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

        const partitionReassigner = new PartitionReassigner(cluster, partitionService, { reassignmentDelayMs: 1000 });
        const readReplicaHandler = new ReadReplicaHandler(
            partitionService,
            cluster,
            config.nodeId,
            undefined,
            {
                defaultConsistency: config.defaultConsistency ?? ConsistencyLevel.STRONG,
                preferLocalReplica: true,
                loadBalancing: 'latency-based'
            }
        );

        const merkleTreeManager = new MerkleTreeManager(config.nodeId);
        const repairScheduler = new RepairScheduler(
            merkleTreeManager,
            cluster,
            partitionService,
            config.nodeId,
            { enabled: true, scanIntervalMs: 300000, maxConcurrentRepairs: 2 }
        );

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
                send: (nodeId, type, payload) => cluster.send(nodeId, type, payload),
                isLocal: (id) => cluster.isLocal(id),
                config: { nodeId: config.nodeId },
                getLeaderId: () => cluster.getLeaderId ? cluster.getLeaderId() : null,
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
            storage: config.storage || null,
            // broadcast callback will be set via late binding in ServerCoordinator
            metricsService: {
                incOp: (op, mapName) => metricsService.incOp(op, mapName),
            },
            gcIntervalMs: config.gcIntervalMs,
            gcAgeMs: config.gcAgeMs,
        });

        // Initialize QueryConversionHandler (no late binding needed)
        const queryConversionHandler = new QueryConversionHandler({
            storageManager,
            cluster: {
                getMembers: () => cluster.getMembers(),
                send: (nodeId, type, payload) => cluster.send(nodeId, type, payload),
                config: { nodeId: config.nodeId },
            },
            partitionService: {
                getPartitionMap: () => partitionService.getPartitionMap(),
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
            writeAckManager: {
                registerWrite: (reqId, ackConfig, onComplete) => writeAckManager.registerWrite(reqId, ackConfig, onComplete),
            },
        });

        // Initialize ClusterEventHandler (no late binding needed)
        const clusterEventHandler = new ClusterEventHandler({
            storageManager,
            cluster: {
                getMembers: () => cluster.getMembers(),
            },
            partitionService: {
                getOwner: (key) => partitionService.getOwner(key),
            },
            gcHandler: {
                handleGcReport: (nodeId, minHlc) => gcHandler.handleGcReport(nodeId, minHlc),
                handleGcCommit: (globalSafe) => gcHandler.handleGcCommit(globalSafe),
            },
            queryConversionHandler: {
                handleQueryFinalRequest: (reqId, originNode) => queryConversionHandler.handleQueryFinalRequest(reqId, originNode),
                handleQueryFinalResponse: (reqId, partialResults) => queryConversionHandler.handleQueryFinalResponse(reqId, partialResults),
            },
            writeConcernHandler: {
                handleWriteAck: (reqId, nodeId) => writeConcernHandler.handleWriteAck(reqId, nodeId),
            },
            replicationPipeline: replicationPipeline ? {
                handleReplicationAck: (opId, nodeId) => replicationPipeline!.handleReplicationAck(opId, nodeId),
            } : undefined,
        });

        // Create local handlers for MessageRegistry
        const queryHandler = new QueryHandler({
            executeLocalQuery: (map, query) => queryConversionHandler.executeLocalQuery(map, query),
            finalizeClusterQuery: (reqId, timeout) => queryConversionHandler.finalizeClusterQuery(reqId, timeout),
        });

        const counterHandlerAdapter = new CounterHandlerAdapter({
            counterHandler: {
                handleIncrement: (clientId, mapName, key, delta) => counterHandler.handleIncrement(clientId, mapName, key, delta),
                handleGet: (clientId, mapName, key) => counterHandler.handleGet(clientId, mapName, key),
                handleSubscribe: (clientId, mapName, key, subscriptionId) => counterHandler.handleSubscribe(clientId, mapName, key, subscriptionId),
                handleUnsubscribe: (subscriptionId) => counterHandler.handleUnsubscribe(subscriptionId),
            },
        });

        const resolverHandler = new ResolverHandler({
            conflictResolverHandler: {
                registerResolver: (mapName, resolver) => conflictResolverHandler.registerResolver(mapName, resolver),
                unregisterResolver: (mapName) => conflictResolverHandler.unregisterResolver(mapName),
            },
        });

        const lwwSyncHandler = new LwwSyncHandler({
            storageManager,
        });

        const orMapSyncHandler = new ORMapSyncHandler({
            storageManager,
        });

        const entryProcessorAdapter = new EntryProcessorAdapter({
            entryProcessorHandler: {
                execute: (mapName, key, processorName, args, principal) => entryProcessorHandler.execute(mapName, key, processorName, args, principal),
            },
        });

        // Create MessageRegistry with all handlers
        const messageRegistry = createMessageRegistry({
            queryHandler,
            lockHandler,
            topicHandler,
            counterHandlerAdapter,
            resolverHandler,
            partitionHandler,
            searchHandler,
            journalHandler,
            lwwSyncHandler,
            orMapSyncHandler,
            entryProcessorAdapter,
        });

        // Create LifecycleManager with direct dependencies
        const lifecycleManager = new LifecycleManager({
            storageManager,
            metricsService,
            hlc,
            queryRegistry,
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

        return new ServerCoordinator(config, {
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
            clusterEventHandler,
            messageRegistry,
            lifecycleManager,
        });
    }

    // Helper to build TLS options
    private static buildTLSOptions(config: any): HttpsServerOptions {
        const options: HttpsServerOptions = {
            cert: readFileSync(config.certPath),
            key: readFileSync(config.keyPath),
            minVersion: config.minVersion || 'TLSv1.2',
        };
        if (config.caCertPath) options.ca = readFileSync(config.caCertPath);
        if (config.ciphers) options.ciphers = config.ciphers;
        if (config.passphrase) options.passphrase = config.passphrase;
        return options;
    }

    private static createMetricsServer(
        port: number,
        bootstrap: BootstrapController,
        settings: SettingsController,
        debug: DebugEndpoints,
        metrics: MetricsService
    ): HttpServer {
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

        server.listen(port, () => {
            logger.info({ port }, 'Metrics server listening');
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
