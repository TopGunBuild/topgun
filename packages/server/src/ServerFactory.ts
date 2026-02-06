import { createServer as createHttpServer, Server as HttpServer, IncomingMessage, ServerResponse } from 'http';
import { WebSocket } from 'ws';
import { ConsistencyLevel, PARTITION_COUNT, serialize, deserialize, HttpSyncRequestSchema } from '@topgunbuild/core';
import type { HttpSyncResponse } from '@topgunbuild/core';
import { ServerCoordinatorConfig, ServerCoordinator } from './ServerCoordinator';
import { ClusterManager } from './cluster/ClusterManager';
import { PartitionService } from './cluster/PartitionService';
import { TopicManager } from './topic/TopicManager';
import { logger } from './utils/logger';
import { validateJwtSecret } from './utils/validateConfig';
import { coalescingPresets } from './utils/coalescingPresets';
import { createCoreModule, createWorkersModule, createClusterModule, createStorageModule, createNetworkModule, createHandlersModule, createLifecycleModule } from './modules';
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
    HttpSyncHandler,
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

        // Reuse existing endpoint creation logic
        const debugEndpoints = createDebugEndpoints({
            enabled: config.debugEnabled ?? process.env.TOPGUN_DEBUG_ENDPOINTS === 'true',
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

        // Create all handlers using handlers-module
        const handlers = createHandlersModule(
            {
                nodeId: config.nodeId,
                jwtSecret,
                rateLimitingEnabled: config.rateLimitingEnabled,
                writeCoalescingEnabled,
                writeCoalescingOptions,
                interceptors: config.interceptors,
                storage: config.storage,
                eventJournalEnabled: config.eventJournalEnabled,
                eventJournalConfig: config.eventJournalConfig,
                fullTextSearch: config.fullTextSearch,
                distributedSearch: config.distributedSearch,
                defaultConsistency: config.defaultConsistency,
            },
            {
                core: {
                    hlc,
                    metricsService,
                    securityManager,
                    eventExecutor,
                    backpressure,
                },
                network: {
                    rateLimiter,
                    rateLimitedLogger,
                },
                cluster: {
                    cluster,
                    partitionService,
                    replicationPipeline,
                    lockManager,
                    merkleTreeManager,
                    readReplicaHandler,
                },
                storage: {
                    storageManager,
                    queryRegistry,
                    writeAckManager,
                },
            }
        );

        // Extract handlers and internal components from handlers module
        const {
            crdt: { operationHandler, batchProcessingHandler, gcHandler },
            sync: { lwwSyncHandler, orMapSyncHandler },
            query: { queryHandler, queryConversionHandler },
            messaging: { topicHandler, broadcastHandler },
            coordination: { lockHandler, partitionHandler },
            search: { searchHandler },
            persistence: { journalHandler, counterHandler: counterHandlerAdapter, entryProcessorHandler: entryProcessorAdapter, resolverHandler },
            client: { authHandler, webSocketHandler, clientMessageHandler },
            server: { heartbeatHandler, persistenceHandler, operationContextHandler, writeConcernHandler },
            messageRegistry,
            _internal: {
                topicManager,
                searchCoordinator,
                clusterSearchCoordinator,
                distributedSubCoordinator,
                connectionManager,
                counterHandler,
                entryProcessorHandler,
                conflictResolverHandler,
                eventJournalService,
                pendingClusterQueries,
                pendingBatchOperations,
                journalSubscriptions,
            }
        } = handlers;

        // Create HTTP sync handler and wire into network module
        const httpSyncHandler = new HttpSyncHandler({
            authHandler,
            operationHandler,
            storageManager,
            queryConversionHandler,
            searchCoordinator,
            hlc,
            securityManager,
        });

        // Wire /sync route into the network module's HTTP handler
        if (network.setHttpRequestHandler) {
            network.setHttpRequestHandler((req: IncomingMessage, res: ServerResponse) => {
                if (req.method === 'POST' && req.url === '/sync') {
                    ServerFactory.handleHttpSync(req, res, httpSyncHandler);
                    return;
                }
                // Default response for non-sync routes
                res.writeHead(200);
                res.end('TopGun Server Running');
            });
        }

        // Create lifecycle module
        const lifecycle = createLifecycleModule(
            { nodeId: config.nodeId },
            {
                httpServer,
                metricsServer,
                wss,
                metricsService,
                eventExecutor,
                connectionManager,
                cluster,
                partitionService,
                replicationPipeline,
                workerPool,
                storage: config.storage,
                taskletScheduler,
                writeAckManager,
                eventPayloadPool,
                gcHandler,
                heartbeatHandler,
                lockManager,
                repairScheduler,
                partitionReassigner,
                queryConversionHandler,
                entryProcessorHandler,
                eventJournalService,
                clusterSearchCoordinator,
                distributedSubCoordinator,
                searchCoordinator,
                getMapAsync: (name) => storageManager.getMapAsync(name),
            }
        );

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
            lifecycleManager: lifecycle.lifecycleManager,
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

    /**
     * Handle an HTTP sync request by parsing the body, validating auth,
     * delegating to HttpSyncHandler, and serializing the response.
     */
    private static handleHttpSync(
        req: IncomingMessage,
        res: ServerResponse,
        handler: HttpSyncHandler,
    ): void {
        // Extract auth token
        const authHeader = req.headers['authorization'] || '';
        const token = authHeader.startsWith('Bearer ') ? authHeader.slice(7) : '';

        if (!token) {
            res.writeHead(401, { 'Content-Type': 'application/json' });
            res.end(JSON.stringify({ error: 'Missing Authorization header' }));
            return;
        }

        // Collect request body
        const chunks: Buffer[] = [];
        req.on('data', (chunk: Buffer) => chunks.push(chunk));
        req.on('end', async () => {
            try {
                const body = Buffer.concat(chunks);
                const contentType = req.headers['content-type'] || '';
                const isJson = contentType.includes('application/json');

                // Parse request body
                let parsed: any;
                if (isJson) {
                    parsed = JSON.parse(body.toString('utf-8'));
                } else {
                    parsed = deserialize(body);
                }

                // Validate against schema
                const validation = HttpSyncRequestSchema.safeParse(parsed);
                if (!validation.success) {
                    res.writeHead(400, { 'Content-Type': 'application/json' });
                    res.end(JSON.stringify({
                        error: 'Invalid request body',
                        details: validation.error.issues,
                    }));
                    return;
                }

                // Process the sync request
                const response = await handler.handleSyncRequest(validation.data, token);

                // Serialize response matching request content type
                if (isJson) {
                    res.writeHead(200, { 'Content-Type': 'application/json' });
                    res.end(JSON.stringify(response));
                } else {
                    const responseBytes = serialize(response);
                    res.writeHead(200, { 'Content-Type': 'application/x-msgpack' });
                    res.end(Buffer.from(responseBytes));
                }
            } catch (err: any) {
                const message = err.message || 'Internal server error';

                if (message.startsWith('401:')) {
                    res.writeHead(401, { 'Content-Type': 'application/json' });
                    res.end(JSON.stringify({ error: message.slice(5).trim() }));
                } else if (message.startsWith('403:')) {
                    res.writeHead(403, { 'Content-Type': 'application/json' });
                    res.end(JSON.stringify({ error: message.slice(5).trim() }));
                } else {
                    logger.error({ err }, 'HTTP sync request failed');
                    res.writeHead(500, { 'Content-Type': 'application/json' });
                    res.end(JSON.stringify({ error: 'Internal server error' }));
                }
            }
        });

        req.on('error', (err) => {
            logger.error({ err }, 'HTTP sync request stream error');
            res.writeHead(400, { 'Content-Type': 'application/json' });
            res.end(JSON.stringify({ error: 'Request stream error' }));
        });
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
