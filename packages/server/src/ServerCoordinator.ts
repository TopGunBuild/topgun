import { createServer as createHttpServer, Server as HttpServer } from 'http';
import { createServer as createHttpsServer, Server as HttpsServer, ServerOptions as HttpsServerOptions } from 'https';
import { readFileSync } from 'fs';
import * as net from 'net';
import { WebSocketServer, WebSocket } from 'ws';
import { HLC, LWWMap, ORMap, MerkleTree, serialize, deserialize, PermissionPolicy, Principal, PermissionType, Timestamp, LWWRecord, MessageSchema, WriteConcern, WriteConcernValue, ConsistencyLevel, ReplicationConfig, DEFAULT_REPLICATION_CONFIG, IndexedLWWMap, IndexedORMap, QueryCursor, DEFAULT_QUERY_CURSOR_MAX_AGE_MS, PARTITION_COUNT, type QueryExpression as CoreQuery } from '@topgunbuild/core';
import { IServerStorage, StorageValue, ORMapValue, ORMapTombstones } from './storage/IServerStorage';
import { IInterceptor, ServerOp, OpContext, ConnectionContext } from './interceptor/IInterceptor';
import * as jwt from 'jsonwebtoken';
import * as crypto from 'crypto';
import { QueryRegistry, Subscription } from './query/QueryRegistry';

const CLIENT_HEARTBEAT_TIMEOUT_MS = 20000; // 20 seconds - evict clients that haven't pinged
const CLIENT_HEARTBEAT_CHECK_INTERVAL_MS = 5000; // Check for dead clients every 5 seconds
const GC_AGE_MS = 30 * 24 * 60 * 60 * 1000; // 30 days - used by sync handlers for tombstone filtering
import { TopicManager } from './topic/TopicManager';
import { ClusterManager } from './cluster/ClusterManager';
import { PartitionService } from './cluster/PartitionService';
import { LockManager } from './cluster/LockManager';
import { executeQuery, Query } from './query/Matcher';
import { SecurityManager } from './security/SecurityManager';
import { logger } from './utils/logger';
import { validateJwtSecret } from './utils/validateConfig';
import { MetricsService } from './monitoring/MetricsService';
import { SystemManager } from './system/SystemManager';
import { TLSConfig, ClusterTLSConfig } from './types/TLSConfig';
import { StripedEventExecutor } from './utils/StripedEventExecutor';
import { BackpressureRegulator } from './utils/BackpressureRegulator';
import { CoalescingWriter, CoalescingWriterOptions } from './utils/CoalescingWriter';
import { coalescingPresets, CoalescingPreset } from './utils/coalescingPresets';
import { ConnectionRateLimiter } from './utils/ConnectionRateLimiter';
import { RateLimitedLogger } from './utils/RateLimitedLogger';
import { WorkerPool, MerkleWorker, CRDTMergeWorker, SerializationWorker, WorkerPoolConfig } from './workers';
import {
    ObjectPool,
    createEventPayloadPool,
    PooledEventPayload,
} from './memory';
import { TaskletScheduler } from './tasklet';
import { WriteAckManager } from './ack/WriteAckManager';
import { ReplicationPipeline } from './cluster/ReplicationPipeline';
import { PartitionReassigner } from './cluster/PartitionReassigner';
import { ReadReplicaHandler } from './cluster/ReadReplicaHandler';
import { MerkleTreeManager } from './cluster/MerkleTreeManager';
import { RepairScheduler } from './cluster/RepairScheduler';
import { CounterHandler } from './handlers/CounterHandler';
import { EntryProcessorHandler } from './handlers/EntryProcessorHandler';
import { ConflictResolverHandler } from './handlers/ConflictResolverHandler';
import { EventJournalService, EventJournalServiceConfig } from './EventJournalService';
import { SearchCoordinator, SearchConfig, ClusterSearchCoordinator, type ClusterSearchConfig } from './search';
import { DistributedSubscriptionCoordinator } from './subscriptions/DistributedSubscriptionCoordinator';
import { createDebugEndpoints, DebugEndpoints } from './debug';
import { BootstrapController, createBootstrapController } from './bootstrap';
import { SettingsController, createSettingsController } from './settings';
import type { JournalEvent, JournalEventType, MergeRejection, MergeContext, FullTextIndexConfig } from '@topgunbuild/core';
import {
    AuthHandler,
    ConnectionManager,
    StorageManager,
    OperationHandler,
    BroadcastHandler,
    GCHandler,
    ClusterEventHandler,
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
    type MessageRegistry,
    type ClientConnection,
} from './coordinator';

interface PendingClusterQuery {
    requestId: string;
    client: ClientConnection;
    queryId: string; // Client's Query ID
    mapName: string;
    query: Query;
    results: { key: string; value: any }[];
    expectedNodes: Set<string>;
    respondedNodes: Set<string>;
    timer: NodeJS.Timeout;
}

export interface ServerCoordinatorConfig {
    port: number;
    nodeId: string;
    storage?: IServerStorage;
    jwtSecret?: string;
    host?: string;
    clusterPort?: number;
    peers?: string[];
    securityPolicies?: PermissionPolicy[];
    /** Callback to resolve dynamic peer addresses after ports are known */
    resolvePeers?: () => string[];
    interceptors?: IInterceptor[];
    metricsPort?: number;
    discovery?: 'manual' | 'kubernetes';
    serviceName?: string;
    discoveryInterval?: number;
    tls?: TLSConfig;
    clusterTls?: ClusterTLSConfig;
    /** Total event queue capacity for bounded queue (default: 10000) */
    eventQueueCapacity?: number;
    /** Number of event queue stripes for parallel processing (default: 4) */
    eventStripeCount?: number;
    /** Enable/disable backpressure (default: true) */
    backpressureEnabled?: boolean;
    /** How often to force sync processing (default: 100 operations) */
    backpressureSyncFrequency?: number;
    /** Maximum pending async operations before blocking (default: 1000) */
    backpressureMaxPending?: number;
    /** Backoff timeout in ms when at capacity (default: 5000) */
    backpressureBackoffMs?: number;
    /** Enable/disable write coalescing (default: true) */
    writeCoalescingEnabled?: boolean;
    /** Coalescing preset: 'conservative', 'balanced', 'highThroughput', 'aggressive' (default: 'highThroughput') */
    writeCoalescingPreset?: CoalescingPreset;
    /** Maximum messages to batch before forcing flush (default: 500 for highThroughput) */
    writeCoalescingMaxBatch?: number;
    /** Maximum delay before flushing in ms (default: 10 for highThroughput) */
    writeCoalescingMaxDelayMs?: number;
    /** Maximum batch size in bytes (default: 262144/256KB for highThroughput) */
    writeCoalescingMaxBytes?: number;

    // === Connection Scaling Options ===
    /** WebSocket backlog for pending connections (default: 511) */
    wsBacklog?: number;
    /** Enable WebSocket per-message compression (default: false for CPU savings) */
    wsCompression?: boolean;
    /** Maximum WebSocket payload size in bytes (default: 64MB) */
    wsMaxPayload?: number;
    /** Maximum server connections (default: 10000) */
    maxConnections?: number;
    /** Server timeout in ms (default: 120000 = 2 min) */
    serverTimeout?: number;
    /** Keep-alive timeout in ms (default: 5000) */
    keepAliveTimeout?: number;
    /** Headers timeout in ms (default: 60000) */
    headersTimeout?: number;

    // === Rate Limiting Options ===
    /** Enable connection rate limiting (default: true) */
    rateLimitingEnabled?: boolean;
    /** Maximum new connections per second (default: 100) */
    maxConnectionsPerSecond?: number;
    /** Maximum pending connections (default: 1000) */
    maxPendingConnections?: number;

    // === Worker Pool Options ===
    /** Enable worker pool for CPU-bound operations (default: false) */
    workerPoolEnabled?: boolean;
    /** Worker pool configuration */
    workerPoolConfig?: Partial<WorkerPoolConfig>;

    // === Write Concern Options (Phase 5.01) ===
    /** Default timeout for Write Concern acknowledgments in ms (default: 5000) */
    writeAckTimeout?: number;

    // === Replication Options (Phase 4) ===
    /** Enable replication to backup nodes (default: true when cluster has peers) */
    replicationEnabled?: boolean;
    /** Default consistency level for replication (default: EVENTUAL) */
    defaultConsistency?: ConsistencyLevel;
    /** Replication configuration */
    replicationConfig?: Partial<ReplicationConfig>;

    // === Event Journal Options (Phase 5.04) ===
    /** Enable event journal for audit/CDC (default: false) */
    eventJournalEnabled?: boolean;
    /** Event journal configuration */
    eventJournalConfig?: Partial<Omit<EventJournalServiceConfig, 'pool'>>;

    // === Full-Text Search Options (Phase 11.1) ===
    /** Enable full-text search for specific maps */
    fullTextSearch?: Record<string, FullTextIndexConfig>;

    // === Distributed Search Options (Phase 14) ===
    /** Configuration for distributed search across cluster nodes */
    distributedSearch?: ClusterSearchConfig;

    // === Debug Options (Phase 14C) ===
    /** Enable debug endpoints (/debug/crdt/*, /debug/search/*) (default: false, or TOPGUN_DEBUG=true) */
    debugEnabled?: boolean;
}

export class ServerCoordinator {
    private httpServer: HttpServer | HttpsServer;
    private metricsServer?: HttpServer;
    private metricsService: MetricsService;
    private wss: WebSocketServer;
    private connectionManager!: ConnectionManager;

    // Interceptors
    private interceptors: IInterceptor[] = [];

    // In-memory storage - delegated to StorageManager
    private storageManager!: StorageManager;
    private hlc: HLC;
    private storage?: IServerStorage;
    private jwtSecret: string;
    private queryRegistry: QueryRegistry;

    private cluster!: ClusterManager;
    private partitionService!: PartitionService;
    private replicationPipeline?: ReplicationPipeline;
    private lockManager!: LockManager;
    private topicManager!: TopicManager;
    private securityManager: SecurityManager;
    private authHandler!: AuthHandler;
    private operationHandler!: OperationHandler;
    private broadcastHandler!: BroadcastHandler;
    private gcHandler!: GCHandler;
    private clusterEventHandler!: ClusterEventHandler;
    private messageRegistry!: MessageRegistry;
    private systemManager!: SystemManager;

    private pendingClusterQueries: Map<string, PendingClusterQuery> = new Map();
    private heartbeatCheckInterval?: NodeJS.Timeout;

    // Track pending batch operations for testing purposes
    private pendingBatchOperations: Set<Promise<void>> = new Set();

    // Bounded event queue executor for backpressure control
    private eventExecutor: StripedEventExecutor;

    // Backpressure regulator for periodic sync processing
    private backpressure: BackpressureRegulator;

    // Write coalescing options
    private writeCoalescingEnabled: boolean;
    private writeCoalescingOptions: Partial<CoalescingWriterOptions>;

    // Connection rate limiter
    private rateLimiter: ConnectionRateLimiter;
    private rateLimitingEnabled: boolean;

    // Rate-limited logger for invalid message errors (SEC-04)
    private rateLimitedLogger: RateLimitedLogger;

    // Worker pool for CPU-bound operations
    private workerPool?: WorkerPool;
    private merkleWorker?: MerkleWorker;
    private crdtMergeWorker?: CRDTMergeWorker;
    private serializationWorker?: SerializationWorker;

    // Memory pools for GC pressure reduction
    private eventPayloadPool: ObjectPool<PooledEventPayload>;

    // Tasklet scheduler for cooperative multitasking
    private taskletScheduler: TaskletScheduler;

    // Write Concern acknowledgment manager (Phase 5.01)
    private writeAckManager: WriteAckManager;

    // PN Counter handler (Phase 5.2)
    private counterHandler!: CounterHandler;

    // Entry Processor handler (Phase 5.03)
    private entryProcessorHandler!: EntryProcessorHandler;

    // Conflict Resolver handler (Phase 5.05)
    private conflictResolverHandler!: ConflictResolverHandler;

    // Event Journal (Phase 5.04)
    private eventJournalService?: EventJournalService;
    private journalSubscriptions: Map<string, { clientId: string; mapName?: string; types?: JournalEventType[] }> = new Map();

    // Phase 10 - Cluster Enhancements
    private partitionReassigner?: PartitionReassigner;
    private readReplicaHandler?: ReadReplicaHandler;
    private merkleTreeManager?: MerkleTreeManager;
    private repairScheduler?: RepairScheduler;

    // Phase 11.1 - Full-Text Search
    private searchCoordinator!: SearchCoordinator;

    // Phase 14 - Distributed Search
    private clusterSearchCoordinator?: ClusterSearchCoordinator;

    // Phase 14.2 - Distributed Live Subscriptions
    private distributedSubCoordinator?: DistributedSubscriptionCoordinator;

    // Phase 14C - Debug Endpoints
    private debugEndpoints?: DebugEndpoints;

    // Phase 14D - Bootstrap Controller
    private bootstrapController: BootstrapController;

    // Phase 14D-3 - Settings Controller
    private settingsController: SettingsController;

    private readonly _nodeId: string;

    private _actualPort: number = 0;
    private _actualClusterPort: number = 0;
    private _readyPromise: Promise<void>;
    private _readyResolve!: () => void;

    constructor(config: ServerCoordinatorConfig) {
        this._readyPromise = new Promise((resolve) => {
            this._readyResolve = resolve;
        });

        this._nodeId = config.nodeId;
        this.hlc = new HLC(config.nodeId);
        this.storage = config.storage;
        // Validate and handle JWT_SECRET with escaped newlines (e.g., from Docker/Dokploy env vars)
        const rawSecret = validateJwtSecret(config.jwtSecret, process.env.JWT_SECRET);
        this.jwtSecret = rawSecret.replace(/\\n/g, '\n');
        this.queryRegistry = new QueryRegistry();
        this.securityManager = new SecurityManager(config.securityPolicies || []);
        this.interceptors = config.interceptors || [];
        this.metricsService = new MetricsService();

        // Initialize bounded event queue executor
        this.eventExecutor = new StripedEventExecutor({
            stripeCount: config.eventStripeCount ?? 4,
            queueCapacity: config.eventQueueCapacity ?? 10000,
            name: `${config.nodeId}-event-executor`,
            onReject: (task) => {
                logger.warn({ nodeId: config.nodeId, key: task.key }, 'Event task rejected due to queue capacity');
                this.metricsService.incEventQueueRejected();
            }
        });

        // Initialize backpressure regulator for periodic sync processing
        this.backpressure = new BackpressureRegulator({
            syncFrequency: config.backpressureSyncFrequency ?? 100,
            maxPendingOps: config.backpressureMaxPending ?? 1000,
            backoffTimeoutMs: config.backpressureBackoffMs ?? 5000,
            enabled: config.backpressureEnabled ?? true
        });

        // Initialize write coalescing options with preset support
        // Default preset changed from 'conservative' to 'highThroughput' for better performance
        this.writeCoalescingEnabled = config.writeCoalescingEnabled ?? true;
        const preset = coalescingPresets[config.writeCoalescingPreset ?? 'highThroughput'];
        this.writeCoalescingOptions = {
            maxBatchSize: config.writeCoalescingMaxBatch ?? preset.maxBatchSize,
            maxDelayMs: config.writeCoalescingMaxDelayMs ?? preset.maxDelayMs,
            maxBatchBytes: config.writeCoalescingMaxBytes ?? preset.maxBatchBytes,
        };

        // Initialize ConnectionManager (single owner of clients Map)
        this.connectionManager = new ConnectionManager({
            hlc: this.hlc,
            writeCoalescingEnabled: this.writeCoalescingEnabled,
            writeCoalescingOptions: this.writeCoalescingOptions,
        });

        // Initialize StorageManager (single owner of maps Map)
        // Note: isRelatedKey is deferred to partitionService which is initialized in start()
        this.storageManager = new StorageManager({
            nodeId: config.nodeId,
            hlc: this.hlc,
            storage: this.storage,
            fullTextSearch: config.fullTextSearch,
            // Partition filter: defer to partitionService (initialized later in start())
            isRelatedKey: (key: string) => this.partitionService?.isRelated(key) ?? true,
            onMapLoaded: (mapName, recordCount) => {
                // Refresh query subscriptions and update metrics after map load
                const map = this.storageManager.getMaps().get(mapName);
                if (map) {
                    this.queryRegistry.refreshSubscriptions(mapName, map);
                    const mapSize = (map instanceof ORMap) ? map.totalRecords : map.size;
                    this.metricsService.setMapSize(mapName, mapSize);
                }
            },
        });

        // Initialize memory pools for GC pressure reduction
        this.eventPayloadPool = createEventPayloadPool({
            maxSize: 4096,
            initialSize: 128,
        });

        // Initialize tasklet scheduler for cooperative multitasking
        this.taskletScheduler = new TaskletScheduler({
            defaultTimeBudgetMs: 5,
            maxConcurrent: 20,
        });

        // Initialize Write Concern acknowledgment manager (Phase 5.01)
        this.writeAckManager = new WriteAckManager({
            defaultTimeout: config.writeAckTimeout ?? 5000,
        });

        // Initialize connection rate limiter
        this.rateLimitingEnabled = config.rateLimitingEnabled ?? true;
        this.rateLimiter = new ConnectionRateLimiter({
            maxConnectionsPerSecond: config.maxConnectionsPerSecond ?? 100,
            maxPendingConnections: config.maxPendingConnections ?? 1000,
            cooldownMs: 1000,
        });

        // Initialize AuthHandler for JWT authentication (Phase 4 - extracted module)
        this.authHandler = new AuthHandler({
            jwtSecret: this.jwtSecret,
            onAuthSuccess: (_clientId, _principal) => {
                // Mark connection as established (handshake complete)
                if (this.rateLimitingEnabled) {
                    this.rateLimiter.onConnectionEstablished();
                }
            },
        });

        // Initialize rate-limited logger for invalid message errors (SEC-04)
        this.rateLimitedLogger = new RateLimitedLogger({
            windowMs: 10000,  // 10 second window
            maxPerWindow: 5   // 5 errors per client per window
        });

        // Initialize worker pool for CPU-bound operations
        if (config.workerPoolEnabled) {
            this.workerPool = new WorkerPool({
                minWorkers: config.workerPoolConfig?.minWorkers ?? 2,
                maxWorkers: config.workerPoolConfig?.maxWorkers,
                taskTimeout: config.workerPoolConfig?.taskTimeout ?? 5000,
                idleTimeout: config.workerPoolConfig?.idleTimeout ?? 30000,
                autoRestart: config.workerPoolConfig?.autoRestart ?? true,
            });
            this.merkleWorker = new MerkleWorker(this.workerPool);
            this.crdtMergeWorker = new CRDTMergeWorker(this.workerPool);
            this.serializationWorker = new SerializationWorker(this.workerPool);
            logger.info({
                minWorkers: config.workerPoolConfig?.minWorkers ?? 2,
                maxWorkers: config.workerPoolConfig?.maxWorkers ?? 'auto'
            }, 'Worker pool initialized for CPU-bound operations');
        }

        // HTTP Server Setup first (to get actual port if port=0)
        if (config.tls?.enabled) {
            const tlsOptions = this.buildTLSOptions(config.tls);
            this.httpServer = createHttpsServer(tlsOptions, (_req, res) => {
                res.writeHead(200);
                res.end('TopGun Server Running (Secure)');
            });
            logger.info('TLS enabled for client connections');
        } else {
            this.httpServer = createHttpServer((_req, res) => {
                res.writeHead(200);
                res.end('TopGun Server Running');
            });

            if (process.env.NODE_ENV === 'production') {
                logger.warn('⚠️  TLS is disabled! Client connections are NOT encrypted.');
            }
        }

        // Phase 14C: Create debug endpoints
        const debugEnabled = config.debugEnabled ?? process.env.TOPGUN_DEBUG === 'true';
        this.debugEndpoints = createDebugEndpoints({
            enabled: debugEnabled,
            getMaps: () => this.storageManager.getMaps(),
        });
        if (debugEnabled) {
            logger.info('Debug endpoints enabled');
        }

        // Phase 14D: Create bootstrap controller for setup wizard
        this.bootstrapController = createBootstrapController({
            jwtSecret: this.jwtSecret,
        });
        // Provide data accessors for admin API endpoints
        this.bootstrapController.setDataAccessors({
            getMaps: () => this.storageManager.getMaps(),
            getClusterStatus: () => {
                // getMembers returns string[] of node IDs
                const memberIds = this.cluster?.getMembers() || [];
                const nodes = memberIds.map(nodeId => {
                    // Count partitions owned by this node
                    let partitionCount = 0;
                    if (this.partitionService) {
                        for (let i = 0; i < PARTITION_COUNT; i++) {
                            if (this.partitionService.getPartitionOwner(i) === nodeId) {
                                partitionCount++;
                            }
                        }
                    }

                    return {
                        id: nodeId,
                        address: nodeId, // Node ID is used as address identifier
                        status: 'healthy' as const,
                        partitions: Array.from({ length: partitionCount }, (_, i) => i),
                        connections: this.connectionManager.getClientCount(),
                        memory: { used: process.memoryUsage().heapUsed, total: process.memoryUsage().heapTotal },
                        uptime: process.uptime(),
                    };
                });

                // Generate partition info
                const partitions: { id: number; owner: string; replicas: string[] }[] = [];
                if (this.partitionService) {
                    for (let i = 0; i < PARTITION_COUNT; i++) {
                        const owner = this.partitionService.getPartitionOwner(i);
                        const backups = this.partitionService.getBackups(i);
                        partitions.push({
                            id: i,
                            owner: owner || 'unknown',
                            replicas: backups,
                        });
                    }
                }

                return {
                    nodes,
                    partitions,
                    isRebalancing: this.partitionService?.getMigrationStatus() !== null,
                };
            },
        });
        if (this.bootstrapController.isBootstrapMode) {
            logger.info('Server running in BOOTSTRAP MODE - start Admin UI and visit /setup to configure');
            logger.info('  Run: cd apps/admin-dashboard && pnpm dev');
            logger.info('  Then open: http://localhost:5173/setup');
        }

        // Phase 14D-3: Create settings controller for runtime configuration
        this.settingsController = createSettingsController({
            jwtSecret: this.jwtSecret,
        });
        // React to settings changes
        this.settingsController.setOnSettingsChange((settings) => {
            if (settings.logLevel) {
                logger.level = settings.logLevel;
                logger.info({ level: settings.logLevel }, '[Settings] Log level changed');
            }
            if (settings.rateLimits) {
                this.rateLimiter.updateConfig({
                    maxConnectionsPerSecond: settings.rateLimits.connections,
                });
                logger.info({ rateLimits: settings.rateLimits }, '[Settings] Rate limits changed');
            }
        });

        const metricsPort = config.metricsPort !== undefined ? config.metricsPort : 9090;
        this.metricsServer = createHttpServer(async (req, res) => {
            // Try bootstrap controller first (handles /api/status, /api/setup, /api/auth/login, /api/admin/*)
            const bootstrapHandled = await this.bootstrapController.handle(req, res);
            if (bootstrapHandled) return;

            // Try settings controller (handles /api/admin/settings)
            const url = req.url || '';
            if (url.startsWith('/api/admin/settings')) {
                const settingsHandled = await this.settingsController.handle(req, res);
                if (settingsHandled) return;
            }

            // Try debug endpoints (includes /health, /ready)
            if (this.debugEndpoints) {
                const handled = await this.debugEndpoints.handle(req, res);
                if (handled) return;
            }

            // Metrics endpoint
            if (req.url === '/metrics') {
                try {
                    res.setHeader('Content-Type', this.metricsService.getContentType());
                    res.end(await this.metricsService.getMetrics());
                } catch (err) {
                    res.statusCode = 500;
                    res.end('Internal Server Error');
                }
            } else {
                res.statusCode = 404;
                res.end();
            }
        });
        this.metricsServer.listen(metricsPort, () => {
            logger.info({ port: metricsPort }, 'Metrics server listening');
        });
        this.metricsServer.on('error', (err) => {
            logger.error({ err, port: metricsPort }, 'Metrics server failed to start');
        });

        // Configure WebSocketServer with optimal options for connection scaling
        this.wss = new WebSocketServer({
            server: this.httpServer,
            // Increase backlog for pending connections (default Linux is 128)
            backlog: config.wsBacklog ?? 511,
            // Disable per-message deflate by default (CPU overhead)
            perMessageDeflate: config.wsCompression ?? false,
            // Max payload size (64MB default)
            maxPayload: config.wsMaxPayload ?? 64 * 1024 * 1024,
            // Skip UTF-8 validation for binary messages (performance)
            skipUTF8Validation: true,
        });
        this.wss.on('connection', (ws) => this.handleConnection(ws));

        // Configure HTTP server limits for connection scaling
        this.httpServer.maxConnections = config.maxConnections ?? 10000;
        this.httpServer.timeout = config.serverTimeout ?? 120000; // 2 min
        this.httpServer.keepAliveTimeout = config.keepAliveTimeout ?? 5000;
        this.httpServer.headersTimeout = config.headersTimeout ?? 60000;

        // Configure socket options for all incoming connections
        this.httpServer.on('connection', (socket: net.Socket) => {
            // Disable Nagle's algorithm for lower latency
            socket.setNoDelay(true);
            // Enable keep-alive with 60s interval
            socket.setKeepAlive(true, 60000);
        });

        // Use port 0 to let OS assign a free port
        this.httpServer.listen(config.port, () => {
            const addr = this.httpServer.address();
            this._actualPort = typeof addr === 'object' && addr ? addr.port : config.port;
            logger.info({ port: this._actualPort }, 'Server Coordinator listening');

            // Now setup cluster with actual/configured cluster port
            const clusterPort = config.clusterPort ?? 0;

            // Resolve peers dynamically if callback provided
            const peers = config.resolvePeers ? config.resolvePeers() : (config.peers || []);

            this.cluster = new ClusterManager({
                nodeId: config.nodeId,
                host: config.host || 'localhost',
                port: clusterPort,
                peers,
                discovery: config.discovery,
                serviceName: config.serviceName,
                discoveryInterval: config.discoveryInterval,
                tls: config.clusterTls
            });
            this.partitionService = new PartitionService(this.cluster);

            // Phase 4: Create ReplicationPipeline (Hazelcast pattern: always create, runtime check)
            // ReplicationPipeline checks cluster size at runtime - no replication for single node
            if (config.replicationEnabled !== false) {
                this.replicationPipeline = new ReplicationPipeline(
                    this.cluster,
                    this.partitionService,
                    {
                        ...DEFAULT_REPLICATION_CONFIG,
                        defaultConsistency: config.defaultConsistency ?? ConsistencyLevel.EVENTUAL,
                        ...config.replicationConfig,
                    }
                );
                // Setup operation applier for incoming replications
                this.replicationPipeline.setOperationApplier(this.applyReplicatedOperation.bind(this));
                logger.info({ nodeId: config.nodeId }, 'ReplicationPipeline initialized');
            }

            // Phase 4: Listen for partition map changes and broadcast to clients
            this.partitionService.on('rebalanced', (partitionMap, changes) => {
                this.broadcastPartitionMap(partitionMap);
            });

            this.lockManager = new LockManager();
            this.lockManager.on('lockGranted', (evt) => this.handleLockGranted(evt));

            this.topicManager = new TopicManager({
                cluster: this.cluster,
                sendToClient: (clientId, message) => {
                    const client = this.connectionManager.getClient(clientId);
                    if (client && client.socket.readyState === WebSocket.OPEN) {
                        client.writer.write(message);
                    }
                }
            });

            // PN Counter handler (Phase 5.2)
            this.counterHandler = new CounterHandler(this._nodeId);

            // Entry Processor handler (Phase 5.03)
            this.entryProcessorHandler = new EntryProcessorHandler({ hlc: this.hlc });

            // Conflict Resolver handler (Phase 5.05)
            this.conflictResolverHandler = new ConflictResolverHandler({ nodeId: this._nodeId });
            // Wire up rejection notifications to clients
            this.conflictResolverHandler.onRejection((rejection: MergeRejection) => {
                this.notifyMergeRejection(rejection);
            });

            // Event Journal (Phase 5.04) - requires PostgresAdapter with pool
            if (config.eventJournalEnabled && this.storage && 'pool' in (this.storage as any)) {
                const pool = (this.storage as any).pool;
                this.eventJournalService = new EventJournalService({
                    capacity: 10000,
                    ttlMs: 0,
                    persistent: true,
                    pool,
                    ...config.eventJournalConfig,
                });
                this.eventJournalService.initialize().then(() => {
                    logger.info('EventJournalService initialized');
                }).catch(err => {
                    logger.error({ err }, 'Failed to initialize EventJournalService');
                });
            }

            // Phase 10.02: Automatic partition failover
            this.partitionReassigner = new PartitionReassigner(
                this.cluster,
                this.partitionService,
                { reassignmentDelayMs: 1000 }
            );
            this.partitionReassigner.on('failoverComplete', (event) => {
                logger.info({
                    failedNodeId: event.failedNodeId,
                    partitionsReassigned: event.partitionsReassigned,
                    durationMs: event.durationMs
                }, 'Partition failover completed');
                // Broadcast updated partition map to clients
                this.broadcastPartitionMap(this.partitionService.getPartitionMap());
            });
            logger.info('PartitionReassigner initialized');

            // Phase 10.03: Read replica handler for read scaling
            this.readReplicaHandler = new ReadReplicaHandler(
                this.partitionService,
                this.cluster,
                this._nodeId,
                undefined, // LagTracker - can be added later
                {
                    defaultConsistency: config.defaultConsistency ?? ConsistencyLevel.STRONG,
                    preferLocalReplica: true,
                    loadBalancing: 'latency-based'
                }
            );
            logger.info('ReadReplicaHandler initialized');

            // Phase 10.04: Anti-entropy repair
            this.merkleTreeManager = new MerkleTreeManager(this._nodeId);
            this.repairScheduler = new RepairScheduler(
                this.merkleTreeManager,
                this.cluster,
                this.partitionService,
                this._nodeId,
                {
                    enabled: true,
                    scanIntervalMs: 300000, // 5 minutes
                    maxConcurrentRepairs: 2
                }
            );
            // Wire up data accessors for repair
            this.repairScheduler.setDataAccessors(
                (key: string) => this.getLocalRecord(key) ?? undefined,
                (key: string, record: any) => this.applyRepairRecord(key, record)
            );
            this.repairScheduler.start();
            logger.info('MerkleTreeManager and RepairScheduler initialized');

            // Phase 11.1: Full-Text Search
            this.searchCoordinator = new SearchCoordinator();
            // Set up document value getter
            this.searchCoordinator.setDocumentValueGetter((mapName, key) => {
                const map = this.storageManager.getMaps().get(mapName);
                if (!map) return undefined;
                return map.get(key);
            });
            // Phase 11.1b: Set up search update callback for live subscriptions
            this.searchCoordinator.setSendUpdateCallback((clientId, subscriptionId, key, value, score, matchedTerms, type) => {
                const client = this.connectionManager.getClient(clientId);
                if (client) {
                    client.writer.write({
                        type: 'SEARCH_UPDATE',
                        payload: {
                            subscriptionId,
                            key,
                            value,
                            score,
                            matchedTerms,
                            type,
                        }
                    });
                }
            });
            // Enable FTS for configured maps
            if (config.fullTextSearch) {
                for (const [mapName, ftsConfig] of Object.entries(config.fullTextSearch)) {
                    this.searchCoordinator.enableSearch(mapName, ftsConfig);
                    logger.info({ mapName, fields: ftsConfig.fields }, 'FTS enabled for map');
                }
            }

            // Phase 14: Initialize ClusterSearchCoordinator for distributed search
            this.clusterSearchCoordinator = new ClusterSearchCoordinator(
                this.cluster,
                this.partitionService,
                this.searchCoordinator,
                config.distributedSearch,
                this.metricsService
            );
            logger.info('ClusterSearchCoordinator initialized for distributed search');

            // Phase 14.2: Initialize DistributedSubscriptionCoordinator for live subscriptions
            this.distributedSubCoordinator = new DistributedSubscriptionCoordinator(
                this.cluster,
                this.queryRegistry,
                this.searchCoordinator,
                undefined, // Use default config
                this.metricsService
            );
            logger.info('DistributedSubscriptionCoordinator initialized for distributed live subscriptions');

            // Set node ID for SearchCoordinator (needed for distributed update routing)
            this.searchCoordinator.setNodeId(config.nodeId);

            // Set ClusterManager on QueryRegistry for distributed updates
            this.queryRegistry.setClusterManager(this.cluster, config.nodeId);

            // Set map getter for QueryRegistry (needed for distributed query initial results)
            this.queryRegistry.setMapGetter((name) => this.getMap(name));

            // Phase 4: Initialize OperationHandler for CRDT operations (CLIENT_OP, OP_BATCH)
            this.operationHandler = new OperationHandler({
                processLocalOp: this.processLocalOp.bind(this),
                processBatchAsync: this.processBatchAsyncWithWriteConcern.bind(this),
                getEffectiveWriteConcern: this.getEffectiveWriteConcern.bind(this),
                stringToWriteConcern: this.stringToWriteConcern.bind(this),
                forwardToOwner: (op) => {
                    const owner = this.partitionService.getOwner(op.key);
                    logger.info({ key: op.key, owner }, 'Forwarding op');
                    this.cluster.sendToNode(owner, op);
                },
                isLocalOwner: (key) => this.partitionService.isLocalOwner(key),
                checkPermission: this.securityManager.checkPermission.bind(this.securityManager),
                incOp: this.metricsService.incOp.bind(this.metricsService),
                writeAckManager: this.writeAckManager,
                pendingBatchOperations: this.pendingBatchOperations,
            });

            // SPEC-003a: Initialize BroadcastHandler for broadcast operations
            this.broadcastHandler = new BroadcastHandler({
                connectionManager: this.connectionManager,
                securityManager: {
                    filterObject: this.securityManager.filterObject.bind(this.securityManager),
                },
                queryRegistry: {
                    getSubscribedClientIds: this.queryRegistry.getSubscribedClientIds.bind(this.queryRegistry),
                },
                metricsService: {
                    incEventsRouted: this.metricsService.incEventsRouted.bind(this.metricsService),
                    incEventsFilteredBySubscription: this.metricsService.incEventsFilteredBySubscription.bind(this.metricsService),
                    recordSubscribersPerEvent: this.metricsService.recordSubscribersPerEvent.bind(this.metricsService),
                },
                hlc: this.hlc,
            });

            // SPEC-003b: Initialize GCHandler for garbage collection
            this.gcHandler = new GCHandler({
                storageManager: this.storageManager,
                connectionManager: this.connectionManager,
                cluster: {
                    getMembers: () => this.cluster.getMembers(),
                    send: (nodeId, type, payload) => this.cluster.send(nodeId, type, payload),
                    isLocal: (id) => this.cluster.isLocal(id),
                    config: { nodeId: config.nodeId },
                },
                partitionService: {
                    isRelated: (key) => this.partitionService.isRelated(key),
                    getPartitionId: (key) => this.partitionService.getPartitionId(key),
                },
                replicationPipeline: this.replicationPipeline ? {
                    replicate: async (op, opId, key) => {
                        await this.replicationPipeline!.replicate(op, opId, key);
                    }
                } : undefined,
                merkleTreeManager: this.merkleTreeManager ? {
                    updateRecord: (partitionId, key, record) => this.merkleTreeManager!.updateRecord(partitionId, key, record),
                } : undefined,
                queryRegistry: {
                    processChange: this.queryRegistry.processChange.bind(this.queryRegistry),
                },
                hlc: this.hlc,
                storage: this.storage,
                broadcast: (message) => this.broadcast(message),
                metricsService: {
                    incOp: this.metricsService.incOp.bind(this.metricsService),
                },
            });

            // SPEC-003c: Initialize ClusterEventHandler for cluster message routing
            this.clusterEventHandler = new ClusterEventHandler({
                cluster: {
                    on: (event, handler) => this.cluster.on(event, handler),
                    off: (event, handler) => this.cluster.off(event, handler),
                    send: (nodeId, type, payload) => this.cluster.send(nodeId, type, payload),
                    config: { nodeId: config.nodeId },
                },
                partitionService: {
                    isLocalOwner: (key) => this.partitionService.isLocalOwner(key),
                    getOwner: (key) => this.partitionService.getOwner(key),
                    isRelated: (key) => this.partitionService.isRelated(key),
                },
                lockManager: {
                    acquire: (name, clientId, requestId, ttl) => this.lockManager.acquire(name, clientId, requestId, ttl),
                    release: (name, clientId, fencingToken) => this.lockManager.release(name, clientId, fencingToken),
                    handleClientDisconnect: (clientId) => this.lockManager.handleClientDisconnect(clientId),
                },
                topicManager: {
                    publish: (topic, data, senderId, fromCluster) => this.topicManager.publish(topic, data, senderId, fromCluster),
                },
                repairScheduler: this.repairScheduler ? {
                    emit: (event, data) => this.repairScheduler!.emit(event, data),
                } : undefined,
                connectionManager: this.connectionManager,
                storageManager: this.storageManager,
                queryRegistry: {
                    processChange: this.queryRegistry.processChange.bind(this.queryRegistry),
                },
                metricsService: {
                    incOp: this.metricsService.incOp.bind(this.metricsService),
                    setClusterMembers: this.metricsService.setClusterMembers.bind(this.metricsService),
                },
                gcHandler: this.gcHandler,
                hlc: this.hlc,
                merkleTreeManager: this.merkleTreeManager ? {
                    getRootHash: (partitionId) => this.merkleTreeManager!.getRootHash(partitionId),
                } : undefined,
                processLocalOp: this.processLocalOp.bind(this),
                executeLocalQuery: this.executeLocalQuery.bind(this),
                finalizeClusterQuery: this.finalizeClusterQuery.bind(this),
                getLocalRecord: this.getLocalRecord.bind(this),
                broadcast: this.broadcast.bind(this),
                getMap: this.getMap.bind(this),
                pendingClusterQueries: this.pendingClusterQueries,
            });

            // Phase 4: Initialize all message handlers
            const partitionHandler = new PartitionHandler({
                partitionService: {
                    getPartitionMap: () => this.partitionService.getPartitionMap(),
                },
            });

            const topicHandler = new TopicHandler({
                topicManager: {
                    subscribe: (clientId, topic) => this.topicManager.subscribe(clientId, topic),
                    unsubscribe: (clientId, topic) => this.topicManager.unsubscribe(clientId, topic),
                    publish: (topic, data, senderId) => this.topicManager.publish(topic, data, senderId),
                },
                securityManager: {
                    checkPermission: this.securityManager.checkPermission.bind(this.securityManager),
                },
            });

            const lockHandler = new LockHandler({
                lockManager: {
                    acquire: (name, clientId, requestId, ttl) => this.lockManager.acquire(name, clientId, requestId, ttl),
                    release: (name, clientId, fencingToken) => this.lockManager.release(name, clientId, fencingToken),
                },
                partitionService: {
                    isLocalOwner: (key) => this.partitionService.isLocalOwner(key),
                    getOwner: (key) => this.partitionService.getOwner(key),
                },
                cluster: {
                    getMembers: () => this.cluster.getMembers(),
                    send: (nodeId, type, payload) => this.cluster.send(nodeId, type, payload),
                    config: { nodeId: config.nodeId },
                },
                securityManager: {
                    checkPermission: this.securityManager.checkPermission.bind(this.securityManager),
                },
            });

            const counterHandlerAdapter = new CounterHandlerAdapter({
                counterHandler: {
                    handleCounterRequest: (clientId, name) => this.counterHandler.handleCounterRequest(clientId, name),
                    handleCounterSync: (clientId, name, state) => this.counterHandler.handleCounterSync(clientId, name, state),
                },
                getClient: (clientId) => this.connectionManager.getClient(clientId),
            });

            const resolverHandler = new ResolverHandler({
                conflictResolverHandler: {
                    registerResolver: (mapName, resolver, clientId) => this.conflictResolverHandler.registerResolver(mapName, resolver, clientId),
                    unregisterResolver: (mapName, resolverName, clientId) => this.conflictResolverHandler.unregisterResolver(mapName, resolverName, clientId),
                    listResolvers: (mapName) => this.conflictResolverHandler.listResolvers(mapName),
                },
                securityManager: {
                    checkPermission: this.securityManager.checkPermission.bind(this.securityManager),
                },
            });

            const journalHandler = new JournalHandler({
                eventJournalService: this.eventJournalService,
                journalSubscriptions: this.journalSubscriptions,
                getClient: (clientId) => this.connectionManager.getClient(clientId),
            });

            const lwwSyncHandler = new LwwSyncHandler({
                getMapAsync: this.getMapAsync.bind(this),
                hlc: this.hlc,
                securityManager: {
                    checkPermission: this.securityManager.checkPermission.bind(this.securityManager),
                },
                metricsService: {
                    incOp: this.metricsService.incOp.bind(this.metricsService),
                },
                gcAgeMs: GC_AGE_MS,
            });

            const ormapSyncHandler = new ORMapSyncHandler({
                getMapAsync: this.getMapAsync.bind(this),
                hlc: this.hlc,
                securityManager: {
                    checkPermission: this.securityManager.checkPermission.bind(this.securityManager),
                },
                metricsService: {
                    incOp: this.metricsService.incOp.bind(this.metricsService),
                },
                storage: this.storage,
                broadcast: (message, excludeClientId) => this.connectionManager.broadcast(message, excludeClientId),
                gcAgeMs: GC_AGE_MS,
            });

            const entryProcessorAdapter = new EntryProcessorAdapter({
                entryProcessorHandler: {
                    executeOnKey: (map, key, processor) => this.entryProcessorHandler.executeOnKey(map, key, processor),
                    executeOnKeys: (map, keys, processor) => this.entryProcessorHandler.executeOnKeys(map, keys, processor),
                },
                getMap: (name) => this.getMap(name),
                securityManager: {
                    checkPermission: this.securityManager.checkPermission.bind(this.securityManager),
                },
                queryRegistry: {
                    processChange: (mapName, map, key, record, oldValue) => this.queryRegistry.processChange(mapName, map, key, record, oldValue),
                },
            });

            const searchHandler = new SearchHandler({
                searchCoordinator: {
                    isSearchEnabled: (mapName) => this.searchCoordinator.isSearchEnabled(mapName),
                    search: (mapName, query, options) => this.searchCoordinator.search(mapName, query, options),
                    subscribe: (clientId, subscriptionId, mapName, query, options) => this.searchCoordinator.subscribe(clientId, subscriptionId, mapName, query, options),
                    unsubscribe: (subscriptionId) => this.searchCoordinator.unsubscribe(subscriptionId),
                },
                clusterSearchCoordinator: this.clusterSearchCoordinator,
                distributedSubCoordinator: this.distributedSubCoordinator,
                cluster: {
                    getMembers: () => this.cluster.getMembers(),
                },
                securityManager: {
                    checkPermission: this.securityManager.checkPermission.bind(this.securityManager),
                },
            });

            const queryHandler = new QueryHandler({
                securityManager: {
                    checkPermission: this.securityManager.checkPermission.bind(this.securityManager),
                    filterObject: (value, principal, mapName) => this.securityManager.filterObject(value, principal, mapName),
                },
                metricsService: {
                    incOp: this.metricsService.incOp.bind(this.metricsService),
                },
                queryRegistry: {
                    unregister: (queryId) => this.queryRegistry.unregister(queryId),
                },
                distributedSubCoordinator: this.distributedSubCoordinator,
                cluster: {
                    getMembers: () => this.cluster.getMembers(),
                    isLocal: (id) => id === config.nodeId,
                    send: (nodeId, type, payload) => this.cluster.send(nodeId, type, payload),
                    config: { nodeId: config.nodeId },
                },
                executeLocalQuery: this.executeLocalQuery.bind(this),
                finalizeClusterQuery: this.finalizeClusterQuery.bind(this),
                pendingClusterQueries: this.pendingClusterQueries,
                readReplicaHandler: this.readReplicaHandler,
                ConsistencyLevel: { EVENTUAL: ConsistencyLevel.EVENTUAL },
            });

            // Phase 4: Initialize MessageRegistry with all handlers
            this.messageRegistry = createMessageRegistry({
                // CRDT operations
                onClientOp: (client, msg) => this.operationHandler.processClientOp(client, msg.payload),
                onOpBatch: (client, msg) => this.operationHandler.processOpBatch(
                    client, msg.payload.ops, msg.payload.writeConcern, msg.payload.timeout
                ),
                // Query operations
                onQuerySub: (client, msg) => queryHandler.handleQuerySub(client, msg),
                onQueryUnsub: (client, msg) => queryHandler.handleQueryUnsub(client, msg),
                // LWW Sync protocol
                onSyncInit: (client, msg) => lwwSyncHandler.handleSyncInit(client, msg),
                onMerkleReqBucket: (client, msg) => lwwSyncHandler.handleMerkleReqBucket(client, msg),
                // ORMap Sync protocol
                onORMapSyncInit: (client, msg) => ormapSyncHandler.handleORMapSyncInit(client, msg),
                onORMapMerkleReqBucket: (client, msg) => ormapSyncHandler.handleORMapMerkleReqBucket(client, msg),
                onORMapDiffRequest: (client, msg) => ormapSyncHandler.handleORMapDiffRequest(client, msg),
                onORMapPushDiff: (client, msg) => ormapSyncHandler.handleORMapPushDiff(client, msg),
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

            this.systemManager = new SystemManager(
                this.cluster,
                this.metricsService,
                (name) => this.getMap(name) as LWWMap<string, any>
            );

            this.clusterEventHandler.setupListeners();
            this.cluster.start().then((actualClusterPort) => {
                this._actualClusterPort = actualClusterPort;
                this.metricsService.setClusterMembers(this.cluster.getMembers().length);
                logger.info({ clusterPort: this._actualClusterPort }, 'Cluster started');
                this.systemManager.start();
                this.gcHandler.start();
                this._readyResolve();
            }).catch((err) => {
                // Fallback for ClusterManager that doesn't return port
                this._actualClusterPort = clusterPort;
                this.metricsService.setClusterMembers(this.cluster.getMembers().length);
                logger.info({ clusterPort: this._actualClusterPort }, 'Cluster started (sync)');
                this.systemManager.start();
                this.gcHandler.start();
                this._readyResolve();
            });
        });

        if (this.storage) {
            // Wait for server to be ready (searchCoordinator initialized) before backfilling
            this.storage.initialize().then(async () => {
                logger.info('Storage adapter initialized');
                // Wait for ready signal to ensure searchCoordinator is initialized
                await this.ready();
                this.backfillSearchIndexes();
            }).catch(err => {
                logger.error({ err }, 'Failed to initialize storage');
            });
        }

        this.startHeartbeatCheck();
    }

    /**
     * Populate FTS indexes from existing map data.
     * Called after storage initialization.
     */
    private async backfillSearchIndexes(): Promise<void> {
        const enabledMaps = this.searchCoordinator.getEnabledMaps();

        const promises = enabledMaps.map(async (mapName) => {
            try {
                // Ensure map is loaded from storage
                const map = await this.getMapAsync(mapName);
                if (!map) return;

                if (map instanceof LWWMap) {
                    const entries = Array.from(map.entries());
                    if (entries.length > 0) {
                        logger.info({ mapName, count: entries.length }, 'Backfilling FTS index');
                        this.searchCoordinator.buildIndexFromEntries(
                            mapName,
                            map.entries() as Iterable<[string, Record<string, unknown> | null]>
                        );
                    }
                } else {
                    logger.warn({ mapName }, 'FTS backfill skipped: Map type not supported (only LWWMap)');
                }
            } catch (err) {
                logger.error({ mapName, err }, 'Failed to backfill FTS index');
            }
        });

        await Promise.all(promises);
        logger.info('FTS backfill completed');
    }

    /** Wait for server to be fully ready (ports assigned) */
    public ready(): Promise<void> {
        return this._readyPromise;
    }

    /**
     * Wait for all pending batch operations to complete.
     * Useful for tests that need to verify state after OP_BATCH.
     */
    public async waitForPendingBatches(): Promise<void> {
        if (this.pendingBatchOperations.size === 0) return;
        await Promise.all(this.pendingBatchOperations);
    }

    /** Get the actual port the server is listening on */
    public get port(): number {
        return this._actualPort;
    }

    /** Get the actual cluster port */
    public get clusterPort(): number {
        return this._actualClusterPort;
    }

    /** Get event executor metrics for monitoring */
    public getEventExecutorMetrics() {
        return this.eventExecutor.getMetrics();
    }

    /** Get total event executor metrics across all stripes */
    public getEventExecutorTotalMetrics() {
        return this.eventExecutor.getTotalMetrics();
    }

    /** Get connection rate limiter stats for monitoring */
    public getRateLimiterStats() {
        return this.rateLimiter.getStats();
    }

    /** Get worker pool stats for monitoring */
    public getWorkerPoolStats() {
        return this.workerPool?.getStats() ?? null;
    }

    /** Check if worker pool is enabled */
    public get workerPoolEnabled(): boolean {
        return !!this.workerPool;
    }

    /** Get MerkleWorker for external use (null if worker pool disabled) */
    public getMerkleWorker(): MerkleWorker | null {
        return this.merkleWorker ?? null;
    }

    /** Get CRDTMergeWorker for external use (null if worker pool disabled) */
    public getCRDTMergeWorker(): CRDTMergeWorker | null {
        return this.crdtMergeWorker ?? null;
    }

    /** Get SerializationWorker for external use (null if worker pool disabled) */
    public getSerializationWorker(): SerializationWorker | null {
        return this.serializationWorker ?? null;
    }

    /** Get memory pool stats for monitoring GC pressure reduction */
    public getMemoryPoolStats() {
        return {
            eventPayloadPool: this.eventPayloadPool.getStats(),
        };
    }

    /** Get tasklet scheduler stats for monitoring cooperative multitasking */
    public getTaskletSchedulerStats() {
        return this.taskletScheduler.getStats();
    }

    /** Get tasklet scheduler for scheduling long-running operations */
    public getTaskletScheduler(): TaskletScheduler {
        return this.taskletScheduler;
    }

    // === Phase 11.1: Full-Text Search Public API ===

    /**
     * Enable full-text search for a map.
     * Can be called at runtime to enable FTS dynamically.
     *
     * @param mapName - Name of the map to enable FTS for
     * @param config - FTS configuration (fields, tokenizer, bm25 options)
     */
    public enableFullTextSearch(mapName: string, config: FullTextIndexConfig): void {
        this.searchCoordinator.enableSearch(mapName, config);

        // Build index from existing data
        const map = this.storageManager.getMaps().get(mapName);
        if (map) {
            const entries: Array<[string, Record<string, unknown> | null]> = [];
            if (map instanceof LWWMap) {
                for (const [key, value] of map.entries()) {
                    entries.push([key, value as Record<string, unknown> | null]);
                }
            } else if (map instanceof ORMap) {
                for (const key of map.allKeys()) {
                    const values = map.get(key);
                    // ORMap can have multiple values per key, take first one for FTS
                    const value = values.length > 0 ? values[0] : null;
                    entries.push([key, value as Record<string, unknown> | null]);
                }
            }
            this.searchCoordinator.buildIndexFromEntries(mapName, entries);
        }
    }

    /**
     * Disable full-text search for a map.
     *
     * @param mapName - Name of the map to disable FTS for
     */
    public disableFullTextSearch(mapName: string): void {
        this.searchCoordinator.disableSearch(mapName);
    }

    /**
     * Check if full-text search is enabled for a map.
     *
     * @param mapName - Name of the map to check
     * @returns True if FTS is enabled
     */
    public isFullTextSearchEnabled(mapName: string): boolean {
        return this.searchCoordinator.isSearchEnabled(mapName);
    }

    /**
     * Get FTS index statistics for a map.
     *
     * @param mapName - Name of the map
     * @returns Index stats or null if FTS not enabled
     */
    public getFullTextSearchStats(mapName: string): { documentCount: number; fields: string[] } | null {
        return this.searchCoordinator.getIndexStats(mapName);
    }

    /**
     * Phase 10.02: Graceful cluster departure
     *
     * Notifies the cluster that this node is leaving and allows time for:
     * 1. Pending replication to complete
     * 2. Other nodes to detect departure
     * 3. Partition reassignment to begin
     */
    private async gracefulClusterDeparture(): Promise<void> {
        if (!this.cluster || this.cluster.getMembers().length <= 1) {
            // Single node or no cluster - nothing to coordinate
            return;
        }

        const nodeId = this._nodeId;
        const ownedPartitions = this.partitionService
            ? this.getOwnedPartitions()
            : [];

        logger.info({
            nodeId,
            ownedPartitions: ownedPartitions.length,
            clusterMembers: this.cluster.getMembers().length
        }, 'Initiating graceful cluster departure');

        // Notify cluster peers that we're leaving
        const departureMessage = {
            type: 'NODE_LEAVING',
            nodeId,
            partitions: ownedPartitions,
            timestamp: Date.now()
        };

        // Broadcast to all cluster members
        for (const memberId of this.cluster.getMembers()) {
            if (memberId !== nodeId) {
                try {
                    this.cluster.send(memberId, 'CLUSTER_EVENT', departureMessage);
                } catch (e) {
                    logger.warn({ memberId, err: e }, 'Failed to notify peer of departure');
                }
            }
        }

        // Wait for pending replication to flush
        if (this.replicationPipeline) {
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
     * Get list of partition IDs owned by this node
     */
    private getOwnedPartitions(): number[] {
        if (!this.partitionService) return [];

        const partitionMap = this.partitionService.getPartitionMap();
        const owned: number[] = [];

        for (const partition of partitionMap.partitions) {
            if (partition.ownerNodeId === this._nodeId) {
                owned.push(partition.partitionId);
            }
        }

        return owned;
    }

    /**
     * Wait for replication pipeline to flush pending operations
     */
    private async waitForReplicationFlush(timeoutMs: number): Promise<void> {
        if (!this.replicationPipeline) return;

        const startTime = Date.now();

        while (Date.now() - startTime < timeoutMs) {
            const pendingOps = this.replicationPipeline.getTotalPending();
            if (pendingOps === 0) {
                return;
            }
            await new Promise(resolve => setTimeout(resolve, 100));
        }

        throw new Error('Replication flush timeout');
    }

    public async shutdown() {
        logger.info('Shutting down Server Coordinator...');

        // Phase 10.02: Graceful cluster departure with partition notification
        await this.gracefulClusterDeparture();

        // 1. Stop accepting new connections
        this.httpServer.close();
        if (this.metricsServer) {
            this.metricsServer.close();
        }
        this.metricsService.destroy();
        this.wss.close();

        // 2. Notify and Close Clients
        logger.info(`Closing ${this.connectionManager.getClientCount()} client connections...`);
        const shutdownMsg = serialize({ type: 'SHUTDOWN_PENDING', retryAfter: 5000 });

        for (const client of this.connectionManager.getClients().values()) {
            try {
                if (client.socket.readyState === WebSocket.OPEN) {
                    // Send shutdown message directly to socket (bypass batching)
                    // This ensures message is sent before socket.close()
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
        this.connectionManager.getClients().clear();

        // 3. Shutdown event executor (wait for pending tasks)
        logger.info('Shutting down event executor...');
        await this.eventExecutor.shutdown(true);

        // 3.5. Shutdown worker pool
        if (this.workerPool) {
            logger.info('Shutting down worker pool...');
            await this.workerPool.shutdown(5000);
            logger.info('Worker pool shutdown complete.');
        }

        // 4. Close ReplicationPipeline
        if (this.replicationPipeline) {
            this.replicationPipeline.close();
        }

        // 4.5. Stop Phase 10 components
        if (this.repairScheduler) {
            this.repairScheduler.stop();
            logger.info('RepairScheduler stopped');
        }
        if (this.partitionReassigner) {
            this.partitionReassigner.stop();
            logger.info('PartitionReassigner stopped');
        }

        // 5. Stop Cluster
        if (this.cluster) {
            this.cluster.stop();
        }

        // 6. Close Storage
        if (this.storage) {
            logger.info('Closing storage connection...');
            try {
                await this.storage.close();
                logger.info('Storage closed successfully.');
            } catch (err) {
                logger.error({ err }, 'Error closing storage');
            }
        }

        // 6. Cleanup
        if (this.gcHandler) {
            this.gcHandler.stop();
        }

        if (this.heartbeatCheckInterval) {
            clearInterval(this.heartbeatCheckInterval);
            this.heartbeatCheckInterval = undefined;
        }

        // Stop LockManager
        if (this.lockManager) {
            this.lockManager.stop();
        }

        // Stop SystemManager
        if (this.systemManager) {
            this.systemManager.stop();
        }

        // Clear memory pools
        this.eventPayloadPool.clear();

        // Shutdown tasklet scheduler
        this.taskletScheduler.shutdown();

        // Shutdown Write Concern manager (Phase 5.01)
        this.writeAckManager.shutdown();

        // Dispose Entry Processor handler (Phase 5.03)
        this.entryProcessorHandler.dispose();

        // Dispose Event Journal (Phase 5.04)
        if (this.eventJournalService) {
            this.eventJournalService.dispose();
        }

        // Destroy ClusterSearchCoordinator (Phase 14)
        if (this.clusterSearchCoordinator) {
            this.clusterSearchCoordinator.destroy();
        }

        // Phase 14.2: Cleanup distributed subscription coordinator
        if (this.distributedSubCoordinator) {
            this.distributedSubCoordinator.destroy();
        }

        logger.info('Server Coordinator shutdown complete.');
    }

    private async handleConnection(ws: WebSocket) {
        // Check rate limit before accepting connection
        if (this.rateLimitingEnabled && !this.rateLimiter.shouldAccept()) {
            logger.warn('Connection rate limit exceeded, rejecting');
            this.rateLimiter.onConnectionRejected();
            this.metricsService.incConnectionsRejected();
            ws.close(1013, 'Server overloaded'); // 1013 = Try Again Later
            return;
        }

        // Register connection attempt
        if (this.rateLimitingEnabled) {
            this.rateLimiter.onConnectionAttempt();
        }
        this.metricsService.incConnectionsAccepted();

        // Client ID is temporary until auth
        const clientId = crypto.randomUUID();
        logger.info({ clientId }, 'Client connected (pending auth)');

        // Create CoalescingWriter if enabled, otherwise create a pass-through writer
        const writer = new CoalescingWriter(ws, this.writeCoalescingEnabled ? this.writeCoalescingOptions : {
            maxBatchSize: 1, // Disable batching by flushing immediately
            maxDelayMs: 0,
            maxBatchBytes: 0,
        });

        // Register client connection via ConnectionManager
        const connection = this.connectionManager.registerClient(clientId, ws, writer);
        this.metricsService.setConnectedClients(this.connectionManager.getClientCount());

        // Run onConnection interceptors
        try {
            const context: ConnectionContext = {
                clientId: connection.id,
                socket: connection.socket,
                isAuthenticated: connection.isAuthenticated,
                principal: connection.principal
            };
            for (const interceptor of this.interceptors) {
                if (interceptor.onConnection) {
                    await interceptor.onConnection(context);
                }
            }
        } catch (err) {
            logger.error({ clientId, err }, 'Interceptor rejected connection');
            ws.close(4000, 'Connection Rejected');
            this.connectionManager.removeClient(clientId);
            return;
        }

        ws.on('message', (message) => {
            try {
                let data: any;
                let buf: Uint8Array;

                if (Buffer.isBuffer(message)) {
                    buf = message;
                } else if (message instanceof ArrayBuffer) {
                    buf = new Uint8Array(message);
                } else if (Array.isArray(message)) {
                    buf = Buffer.concat(message);
                } else {
                    // Fallback or unexpected type
                    buf = Buffer.from(message as any);
                }

                try {
                    data = deserialize(buf);
                } catch (e) {
                    // If msgpack fails, try JSON (legacy support)
                    try {
                        // Use Buffer.toString() or TextDecoder
                        const text = Buffer.isBuffer(buf) ? buf.toString() : new TextDecoder().decode(buf);
                        data = JSON.parse(text);
                    } catch (jsonErr) {
                        // Original error likely relevant
                        throw e;
                    }
                }

                this.handleMessage(connection, data);
            } catch (err) {
                logger.error({ err }, 'Invalid message format');
                ws.close(1002, 'Protocol Error');
            }
        });

        ws.on('close', () => {
            logger.info({ clientId }, 'Client disconnected');

            // If connection was still pending (not authenticated), mark as failed
            if (this.rateLimitingEnabled && !connection.isAuthenticated) {
                this.rateLimiter.onPendingConnectionFailed();
            }

            // Close the CoalescingWriter to flush any pending messages
            connection.writer.close();

            // Run onDisconnect interceptors
            const context: ConnectionContext = {
                clientId: connection.id,
                socket: connection.socket,
                isAuthenticated: connection.isAuthenticated,
                principal: connection.principal
            };
            for (const interceptor of this.interceptors) {
                if (interceptor.onDisconnect) {
                    interceptor.onDisconnect(context).catch(err => {
                        logger.error({ clientId, err }, 'Error in onDisconnect interceptor');
                    });
                }
            }

            // Cleanup subscriptions
            for (const subId of connection.subscriptions) {
                this.queryRegistry.unregister(subId);
            }

            // Cleanup Locks (Local)
            this.lockManager.handleClientDisconnect(clientId);

            // Cleanup Topics (Local)
            this.topicManager.unsubscribeAll(clientId);

            // Cleanup Counters (Local)
            this.counterHandler.unsubscribeAll(clientId);

            // Phase 11.1b: Cleanup Search Subscriptions
            this.searchCoordinator.unsubscribeClient(clientId);

            // Phase 14.2: Cleanup distributed subscriptions for this client
            if (this.distributedSubCoordinator && connection) {
                this.distributedSubCoordinator.unsubscribeClient(connection.socket);
            }

            // Notify Cluster to Cleanup Locks (Remote)
            const members = this.cluster.getMembers();
            for (const memberId of members) {
                if (!this.cluster.isLocal(memberId)) {
                    this.cluster.send(memberId, 'CLUSTER_CLIENT_DISCONNECTED', {
                        originNodeId: this.cluster.config.nodeId,
                        clientId
                    });
                }
            }

            this.connectionManager.removeClient(clientId);
            this.metricsService.setConnectedClients(this.connectionManager.getClientCount());
        });

        // Send Auth Challenge immediately
        ws.send(serialize({ type: 'AUTH_REQUIRED' }));
    }

    private async handleMessage(client: ClientConnection, rawMessage: any) {
        // Validation with Zod
        const parseResult = MessageSchema.safeParse(rawMessage);
        if (!parseResult.success) {
            this.rateLimitedLogger.error(
                `invalid-message:${client.id}`,
                { clientId: client.id, errorCode: parseResult.error.issues[0]?.code },
                'Invalid message format from client'
            );
            client.writer.write({
                type: 'ERROR',
                payload: { code: 400, message: 'Invalid message format', details: (parseResult.error as any).errors }
            }, true); // urgent
            return;
        }
        const message = parseResult.data;

        // Handle PING immediately (even before auth check for authenticated clients)
        if (message.type === 'PING') {
            this.handlePing(client, message.timestamp);
            return;
        }

        // Update client's last active HLC
        // Try to extract from payload if present, otherwise assume near current time but logically before next op
        this.updateClientHlc(client, message);

        // Handshake / Auth handling (delegated to AuthHandler - Phase 4)
        if (!client.isAuthenticated) {
            if (message.type === 'AUTH') {
                const token = message.token;
                const result = await this.authHandler.handleAuth(client, token);
                if (result.success) {
                    client.writer.write({ type: 'AUTH_ACK' }, true); // urgent: bypass batching
                } else {
                    client.writer.write({ type: 'AUTH_FAIL', error: result.error || 'Invalid token' }, true); // urgent
                    client.socket.close(4001, 'Unauthorized');
                }
                return;
            } else {
                // Reject any other message before auth
                client.socket.close(4001, 'Auth required');
            }
            return;
        }

        // Standard Protocol Handling (Authenticated)
        // Phase 4: All message types are now routed through MessageRegistry
        const registryHandler = this.messageRegistry?.[message.type];
        if (registryHandler) {
            await registryHandler(client, message);
            return;
        }

        // Only AUTH for already-authenticated clients remains (duplicate AUTH handling)
        if (message.type === 'AUTH') {
            // Client already authenticated, ignore duplicate AUTH messages
            // This can happen if client sends AUTH before receiving AUTH_ACK
            logger.debug({ clientId: client.id }, 'Ignoring duplicate AUTH from already authenticated client');
            return;
        }

        logger.warn({ type: message.type }, 'Unknown message type');
    }

    /* NOTE: All message handlers have been extracted to coordinator/ modules:
     * - QueryHandler: QUERY_SUB, QUERY_UNSUB
     * - LwwSyncHandler: SYNC_INIT, MERKLE_REQ_BUCKET
     * - ORMapSyncHandler: ORMAP_SYNC_INIT, ORMAP_MERKLE_REQ_BUCKET, ORMAP_DIFF_REQUEST, ORMAP_PUSH_DIFF
     * - LockHandler: LOCK_REQUEST, LOCK_RELEASE
     * - TopicHandler: TOPIC_SUB, TOPIC_UNSUB, TOPIC_PUB
     * - CounterHandlerAdapter: COUNTER_REQUEST, COUNTER_SYNC
     * - EntryProcessorAdapter: ENTRY_PROCESS, ENTRY_PROCESS_BATCH
     * - ResolverHandler: REGISTER_RESOLVER, UNREGISTER_RESOLVER, LIST_RESOLVERS
     * - PartitionHandler: PARTITION_MAP_REQUEST
     * - SearchHandler: SEARCH, SEARCH_SUB, SEARCH_UNSUB
     * - JournalHandler: JOURNAL_SUBSCRIBE, JOURNAL_UNSUBSCRIBE, JOURNAL_READ
     * The switch statement (1234 LOC) has been removed and replaced with MessageRegistry dispatch.
     */


    private updateClientHlc(client: ClientConnection, message: any) {
        // Try to extract timestamp from message if available
        // This is heuristic based on typical message structure
        let ts: Timestamp | undefined;

        if (message.type === 'CLIENT_OP') {
            const op = message.payload;
            if (op.record && op.record.timestamp) {
                ts = op.record.timestamp;
            } else if (op.orRecord && op.orRecord.timestamp) {
                // orRecord usually has entries which have timestamps, or value itself is decorated?
                // Depends on implementation.
            } else if (op.orTag) {
                try {
                    ts = HLC.parse(op.orTag);
                } catch (e) { }
            }
        }

        if (ts) {
            // Client sent an explicit timestamp, update their HLC
            this.hlc.update(ts); // Also update server clock
            // Client HLC is at least this
            client.lastActiveHlc = ts;
        } else {
            // Just bump to current server time if no explicit TS
            // This assumes client is "alive" at this moment.
            client.lastActiveHlc = this.hlc.now();
        }
    }

    // ============ Phase 4: Partition Map Broadcast ============

    /**
     * Broadcast partition map to all connected and authenticated clients.
     * Called when partition topology changes (node join/leave/failover).
     */
    private broadcastPartitionMap(partitionMap: any): void {
        const message = {
            type: 'PARTITION_MAP',
            payload: partitionMap
        };

        let broadcastCount = 0;
        for (const client of this.connectionManager.getClients().values()) {
            if (client.isAuthenticated && client.socket.readyState === WebSocket.OPEN && client.writer) {
                client.writer.write(message);
                broadcastCount++;
            }
        }

        logger.info({
            version: partitionMap.version,
            clientCount: broadcastCount
        }, 'Broadcast partition map to clients');
    }

    /**
     * Notify a client about a merge rejection (Phase 5.05).
     * Finds the client by node ID and sends MERGE_REJECTED message.
     */
    private notifyMergeRejection(rejection: MergeRejection): void {
        // Find client by node ID
        // Node ID format: "client-{uuid}" - we need to find matching client
        for (const [clientId, client] of this.connectionManager.getClients()) {
            // Check if this client sent the rejected operation
            // The nodeId in rejection matches the remoteNodeId from the operation
            if (clientId === rejection.nodeId || rejection.nodeId.includes(clientId)) {
                client.writer.write({
                    type: 'MERGE_REJECTED',
                    mapName: rejection.mapName,
                    key: rejection.key,
                    attemptedValue: rejection.attemptedValue,
                    reason: rejection.reason,
                    timestamp: rejection.timestamp,
                }, true); // urgent - bypass batching
                return;
            }
        }

        // If no matching client found, broadcast to all clients subscribed to this map
        const subscribedClientIds = this.queryRegistry.getSubscribedClientIds(rejection.mapName);
        for (const clientId of subscribedClientIds) {
            const client = this.connectionManager.getClient(clientId);
            if (client) {
                client.writer.write({
                    type: 'MERGE_REJECTED',
                    mapName: rejection.mapName,
                    key: rejection.key,
                    attemptedValue: rejection.attemptedValue,
                    reason: rejection.reason,
                    timestamp: rejection.timestamp,
                });
            }
        }
    }

    private broadcast(message: any, excludeClientId?: string) {
        this.broadcastHandler.broadcast(message, excludeClientId);
    }

    /**
     * === OPTIMIZATION 2 & 3: Batched Broadcast with Serialization Caching ===
     * Groups clients by their permission roles and serializes once per group.
     * Also batches multiple events into a single SERVER_BATCH_EVENT message.
     * === OPTIMIZATION 4: Subscription-based Routing ===
     * Only sends events to clients with active subscriptions for affected maps.
     */
    private broadcastBatch(events: any[], excludeClientId?: string): void {
        this.broadcastHandler.broadcastBatch(events, excludeClientId);
    }

    /**
     * === BACKPRESSURE: Synchronous Broadcast ===
     * Same as broadcastBatch but waits for all sends to complete.
     * Used when backpressure forces sync processing to drain the pipeline.
     */
    private async broadcastBatchSync(events: any[], excludeClientId?: string): Promise<void> {
        await this.broadcastHandler.broadcastBatchSync(events, excludeClientId);
    }

    private async executeLocalQuery(mapName: string, query: Query) {
        // Wait for map to be fully loaded from storage before querying
        const map = await this.getMapAsync(mapName);

        // Fix: Do not apply cursor/limit locally for cluster queries.
        // They will be applied in finalizeClusterQuery after aggregation.
        const localQuery = { ...query };
        delete localQuery.cursor; // Phase 14.1: replaces offset
        delete localQuery.limit;

        // Use indexed query execution if available (O(1) to O(log N))
        if (map instanceof IndexedLWWMap) {
            // Convert Query to core query format for indexed execution
            const coreQuery = this.convertToCoreQuery(localQuery);
            if (coreQuery) {
                const entries = map.queryEntries(coreQuery);
                return entries.map(([key, value]) => {
                    const record = map.getRecord(key);
                    return { key, value, timestamp: record?.timestamp };
                });
            }
        }

        if (map instanceof IndexedORMap) {
            const coreQuery = this.convertToCoreQuery(localQuery);
            if (coreQuery) {
                const results = map.query(coreQuery);
                return results.map(({ key, value }) => ({ key, value }));
            }
        }

        // Fallback to full scan for non-indexed maps
        const records = new Map<string, any>();

        if (map instanceof LWWMap) {
            for (const key of map.allKeys()) {
                const rec = map.getRecord(key);
                if (rec && rec.value !== null) {
                    records.set(key, rec);
                }
            }
        } else if (map instanceof ORMap) {
            // For ORMap, we flatten values. A key matches if ANY of its values match?
            // Or we expose the array of values?
            // For now, we expose { key, value: [v1, v2, ...] }
            const items = (map as any).items as Map<string, any>;
            for (const key of items.keys()) {
                const values = map.get(key);
                if (values.length > 0) {
                    records.set(key, { value: values });
                }
            }
        }

        return executeQuery(records, localQuery);
    }

    /**
     * Convert server Query format to core Query format for indexed execution.
     * Returns null if conversion is not possible (complex queries).
     */
    private convertToCoreQuery(query: Query): CoreQuery | null {
        // Handle predicate-based queries (core format)
        if (query.predicate) {
            return this.predicateToCoreQuery(query.predicate);
        }

        // Handle where-based queries (server format)
        if (query.where) {
            const conditions: CoreQuery[] = [];

            for (const [attribute, condition] of Object.entries(query.where)) {
                if (typeof condition !== 'object' || condition === null) {
                    // Simple equality: { status: 'active' }
                    conditions.push({ type: 'eq', attribute, value: condition });
                } else {
                    // Operator-based: { age: { $gte: 18 } }
                    for (const [op, value] of Object.entries(condition)) {
                        const coreOp = this.convertOperator(op);
                        if (coreOp) {
                            conditions.push({ type: coreOp, attribute, value } as CoreQuery);
                        }
                    }
                }
            }

            if (conditions.length === 0) return null;
            if (conditions.length === 1) return conditions[0];
            return { type: 'and', children: conditions };
        }

        return null;
    }

    /**
     * Convert predicate node to core Query format.
     */
    private predicateToCoreQuery(predicate: any): CoreQuery | null {
        if (!predicate || !predicate.op) return null;

        switch (predicate.op) {
            case 'eq':
            case 'neq':
            case 'gt':
            case 'gte':
            case 'lt':
            case 'lte':
                return {
                    type: predicate.op,
                    attribute: predicate.attribute,
                    value: predicate.value,
                } as CoreQuery;

            case 'and':
            case 'or':
                if (predicate.children && Array.isArray(predicate.children)) {
                    const children = predicate.children
                        .map((c: any) => this.predicateToCoreQuery(c))
                        .filter((c: any): c is CoreQuery => c !== null);
                    if (children.length === 0) return null;
                    if (children.length === 1) return children[0];
                    return { type: predicate.op, children };
                }
                return null;

            case 'not':
                if (predicate.children && predicate.children[0]) {
                    const child = this.predicateToCoreQuery(predicate.children[0]);
                    if (child) {
                        return { type: 'not', child } as CoreQuery;
                    }
                }
                return null;

            default:
                return null;
        }
    }

    /**
     * Convert server operator to core query type.
     */
    private convertOperator(op: string): 'eq' | 'neq' | 'gt' | 'gte' | 'lt' | 'lte' | null {
        const mapping: Record<string, 'eq' | 'neq' | 'gt' | 'gte' | 'lt' | 'lte'> = {
            '$eq': 'eq',
            '$ne': 'neq',
            '$neq': 'neq',
            '$gt': 'gt',
            '$gte': 'gte',
            '$lt': 'lt',
            '$lte': 'lte',
        };
        return mapping[op] || null;
    }

    private async finalizeClusterQuery(requestId: string, timeout = false) {
        const pending = this.pendingClusterQueries.get(requestId);
        if (!pending) return;

        if (timeout) {
            logger.warn({ requestId, responded: pending.respondedNodes.size, expected: pending.expectedNodes.size }, 'Query timed out. Returning partial results.');
        }

        clearTimeout(pending.timer);
        this.pendingClusterQueries.delete(requestId);

        const { client, queryId, mapName, query, results } = pending;

        // Deduplicate results (if backups responded or multiple nodes have same key)
        const uniqueResults = new Map<string, any>();
        for (const res of results) {
            uniqueResults.set(res.key, res);
        }
        const finalResults = Array.from(uniqueResults.values());

        // Re-Apply Sort (Global)
        if (query.sort) {
            finalResults.sort((a, b) => {
                for (const [field, direction] of Object.entries(query.sort!)) {
                    // Handle ORMap array values vs LWW single values?
                    // Assuming LWW for sort logic or array comparison.
                    const valA = a.value[field];
                    const valB = b.value[field];
                    if (valA < valB) return direction === 'asc' ? -1 : 1;
                    if (valA > valB) return direction === 'asc' ? 1 : -1;
                }
                return 0;
            });
        }

        // Phase 14.1: Apply cursor-based pagination
        let slicedResults = finalResults;
        let nextCursor: string | undefined;
        let hasMore = false;
        let cursorStatus: 'valid' | 'expired' | 'invalid' | 'none' = 'none';

        if (query.cursor || query.limit) {
            const sort = query.sort || {};
            const sortEntries = Object.entries(sort);
            const sortField = sortEntries.length > 0 ? sortEntries[0][0] : '_key';

            // Apply cursor filtering and track status
            if (query.cursor) {
                const cursorData = QueryCursor.decode(query.cursor);
                if (!cursorData) {
                    cursorStatus = 'invalid';
                } else if (!QueryCursor.isValid(cursorData, query.predicate ?? query.where, sort)) {
                    // Check if it's specifically expired vs hash mismatch
                    if (Date.now() - cursorData.timestamp > DEFAULT_QUERY_CURSOR_MAX_AGE_MS) {
                        cursorStatus = 'expired';
                    } else {
                        cursorStatus = 'invalid';
                    }
                } else {
                    cursorStatus = 'valid';
                    slicedResults = finalResults.filter((r: any) => {
                        const sortValue = r.value[sortField];
                        return QueryCursor.isAfterCursor(
                            { key: r.key, sortValue },
                            cursorData
                        );
                    });
                }
            }

            // Apply limit and generate next cursor
            if (query.limit) {
                hasMore = slicedResults.length > query.limit;
                slicedResults = slicedResults.slice(0, query.limit);

                if (hasMore && slicedResults.length > 0) {
                    const lastResult = slicedResults[slicedResults.length - 1];
                    const sortValue = lastResult.value[sortField];
                    nextCursor = QueryCursor.fromLastResult(
                        { key: lastResult.key, sortValue },
                        sort,
                        query.predicate ?? query.where
                    );
                }
            }
        }

        // Register Subscription
        const resultKeys = new Set(slicedResults.map((r: any) => r.key));
        const sub: Subscription = {
            id: queryId,
            clientId: client.id,
            mapName,
            query,
            socket: client.socket,
            previousResultKeys: resultKeys,
            interestedFields: 'ALL'
        };

        this.queryRegistry.register(sub);
        client.subscriptions.add(queryId);

        // Apply Field Level Security
        const filteredResults = slicedResults.map((res: any) => {
            const filteredValue = this.securityManager.filterObject(res.value, client.principal!, mapName);
            return { ...res, value: filteredValue };
        });

        client.writer.write({
            type: 'QUERY_RESP',
            payload: { queryId, results: filteredResults, nextCursor, hasMore, cursorStatus }
        });
    }

    /**
     * Core operation application logic shared between processLocalOp and processLocalOpForBatch.
     * Handles map merge, storage persistence, query evaluation, and event generation.
     *
     * @returns Event payload for broadcasting (or null if operation failed)
     */
    private async applyOpToMap(op: any, remoteNodeId?: string): Promise<{ eventPayload: any; oldRecord: any; rejected?: boolean }> {
        // Determine type hint from op
        const typeHint = (op.opType === 'OR_ADD' || op.opType === 'OR_REMOVE') ? 'OR' : 'LWW';
        const map = this.getMap(op.mapName, typeHint);

        // Check compatibility
        if (typeHint === 'OR' && map instanceof LWWMap) {
            logger.error({ mapName: op.mapName }, 'Map type mismatch: LWWMap but received OR op');
            throw new Error('Map type mismatch: LWWMap but received OR op');
        }
        if (typeHint === 'LWW' && map instanceof ORMap) {
            logger.error({ mapName: op.mapName }, 'Map type mismatch: ORMap but received LWW op');
            throw new Error('Map type mismatch: ORMap but received LWW op');
        }

        let oldRecord: any;
        let recordToStore: StorageValue<any> | undefined;
        let tombstonesToStore: StorageValue<any> | undefined;

        const eventPayload: any = {
            mapName: op.mapName,
            key: op.key,
        };

        if (map instanceof LWWMap) {
            oldRecord = map.getRecord(op.key);

            // Use conflict resolver if registered (Phase 5.05)
            if (this.conflictResolverHandler.hasResolvers(op.mapName)) {
                const mergeResult = await this.conflictResolverHandler.mergeWithResolver(
                    map,
                    op.mapName,
                    op.key,
                    op.record,
                    remoteNodeId || this._nodeId,
                );

                if (!mergeResult.applied) {
                    // Operation was rejected or local value kept
                    if (mergeResult.rejection) {
                        logger.debug(
                            { mapName: op.mapName, key: op.key, reason: mergeResult.rejection.reason },
                            'Merge rejected by resolver'
                        );
                    }
                    return { eventPayload: null, oldRecord, rejected: true };
                }

                // Use the resolved record
                recordToStore = mergeResult.record;
                eventPayload.eventType = 'UPDATED';
                eventPayload.record = mergeResult.record;
            } else {
                // Standard merge without resolver
                map.merge(op.key, op.record);
                recordToStore = op.record;
                eventPayload.eventType = 'UPDATED';
                eventPayload.record = op.record;
            }
        } else if (map instanceof ORMap) {
            oldRecord = map.getRecords(op.key);

            if (op.opType === 'OR_ADD') {
                map.apply(op.key, op.orRecord);
                eventPayload.eventType = 'OR_ADD';
                eventPayload.orRecord = op.orRecord;
                recordToStore = { type: 'OR', records: map.getRecords(op.key) };
            } else if (op.opType === 'OR_REMOVE') {
                map.applyTombstone(op.orTag);
                eventPayload.eventType = 'OR_REMOVE';
                eventPayload.orTag = op.orTag;
                recordToStore = { type: 'OR', records: map.getRecords(op.key) };
                tombstonesToStore = { type: 'OR_TOMBSTONES', tags: map.getTombstones() };
            }
        }

        // Live Query Evaluation
        this.queryRegistry.processChange(op.mapName, map, op.key, op.record || op.orRecord, oldRecord);

        // Update metrics
        const mapSize = (map instanceof ORMap) ? map.totalRecords : map.size;
        this.metricsService.setMapSize(op.mapName, mapSize);

        // Persist to storage (async, don't wait)
        if (this.storage) {
            if (recordToStore) {
                this.storage.store(op.mapName, op.key, recordToStore).catch(err => {
                    logger.error({ mapName: op.mapName, key: op.key, err }, 'Failed to persist op');
                });
            }
            if (tombstonesToStore) {
                this.storage.store(op.mapName, '__tombstones__', tombstonesToStore).catch(err => {
                    logger.error({ mapName: op.mapName, err }, 'Failed to persist tombstones');
                });
            }
        }

        // Append to Event Journal (Phase 5.04)
        if (this.eventJournalService) {
            const isDelete = op.opType === 'REMOVE' || op.opType === 'OR_REMOVE' ||
                (op.record && op.record.value === null);
            const isNew = !oldRecord || (Array.isArray(oldRecord) && oldRecord.length === 0);
            const journalEventType: JournalEventType = isDelete ? 'DELETE' : (isNew ? 'PUT' : 'UPDATE');

            const timestamp = op.record?.timestamp || op.orRecord?.timestamp || this.hlc.now();

            this.eventJournalService.append({
                type: journalEventType,
                mapName: op.mapName,
                key: op.key,
                value: op.record?.value ?? op.orRecord?.value,
                previousValue: oldRecord?.value ?? (Array.isArray(oldRecord) ? oldRecord[0]?.value : undefined),
                timestamp,
                nodeId: this._nodeId,
            });
        }

        // Phase 10.04: Update Merkle tree for anti-entropy
        if (this.merkleTreeManager && recordToStore && op.key) {
            const partitionId = this.partitionService.getPartitionId(op.key);
            this.merkleTreeManager.updateRecord(partitionId, op.key, recordToStore as LWWRecord<any>);
        }

        // Phase 11.1: Update FTS index
        if (this.searchCoordinator.isSearchEnabled(op.mapName)) {
            const isRemove = op.opType === 'REMOVE' || (op.record && op.record.value === null);
            const value = isRemove ? null : (op.record?.value ?? op.orRecord?.value);
            const changeType = isRemove ? 'remove' : (oldRecord ? 'update' : 'add');
            this.searchCoordinator.onDataChange(op.mapName, op.key, value, changeType);
        }

        return { eventPayload, oldRecord };
    }

    /**
     * Apply replicated operation from another node (callback for ReplicationPipeline)
     * This is called when we receive a replicated operation as a backup node
     */
    private async applyReplicatedOperation(
        operation: unknown,
        opId: string,
        sourceNode: string
    ): Promise<boolean> {
        try {
            const op = operation as any;
            logger.debug({ sourceNode, opId, mapName: op.mapName, key: op.key }, 'Applying replicated operation');

            // Apply operation to local map (as backup)
            const { eventPayload, rejected } = await this.applyOpToMap(op, sourceNode);

            // Skip broadcast if operation was rejected by resolver
            if (rejected || !eventPayload) {
                return true; // Still return true - rejection is not an error
            }

            // Broadcast event to local clients subscribed to this data
            this.broadcast({
                type: 'SERVER_EVENT',
                payload: eventPayload,
                timestamp: this.hlc.now()
            });

            return true;
        } catch (error) {
            logger.error({ sourceNode, opId, error }, 'Failed to apply replicated operation');
            return false;
        }
    }

    /**
     * Build OpContext for interceptors.
     */
    private buildOpContext(clientId: string, fromCluster: boolean): OpContext {
        let context: OpContext = {
            clientId,
            isAuthenticated: false,
            fromCluster,
            originalSenderId: clientId
        };

        if (!fromCluster) {
            const client = this.connectionManager.getClient(clientId);
            if (client) {
                context = {
                    clientId: client.id,
                    socket: client.socket,
                    isAuthenticated: client.isAuthenticated,
                    principal: client.principal,
                    fromCluster,
                    originalSenderId: clientId
                };
            }
        }

        return context;
    }

    /**
     * Run onBeforeOp interceptors. Returns modified op or null if dropped.
     */
    private async runBeforeInterceptors(op: any, context: OpContext): Promise<any | null> {
        let currentOp: ServerOp | null = op;

        for (const interceptor of this.interceptors) {
            if (interceptor.onBeforeOp && currentOp) {
                currentOp = await interceptor.onBeforeOp(currentOp, context);
                if (!currentOp) {
                    logger.debug({ interceptor: interceptor.name, opId: op.id }, 'Interceptor silently dropped op');
                    return null;
                }
            }
        }

        return currentOp;
    }

    /**
     * Run onAfterOp interceptors (fire-and-forget).
     */
    private runAfterInterceptors(op: any, context: OpContext): void {
        for (const interceptor of this.interceptors) {
            if (interceptor.onAfterOp) {
                interceptor.onAfterOp(op, context).catch(err => {
                    logger.error({ err }, 'Error in onAfterOp');
                });
            }
        }
    }

    private handleLockGranted({ clientId, requestId, name, fencingToken }: { clientId: string, requestId: string, name: string, fencingToken: number }) {
        // Check if local client
        const client = this.connectionManager.getClient(clientId);
        if (client) {
            client.writer.write({
                type: 'LOCK_GRANTED',
                payload: { requestId, name, fencingToken }
            });
            return;
        }

        // Check if remote client (composite ID: "nodeId:realClientId")
        const parts = clientId.split(':');
        if (parts.length === 2) {
            const [nodeId, realClientId] = parts;
            // Verify nodeId is not self (loopback check, though split should handle it)
            if (nodeId !== this.cluster.config.nodeId) {
                this.cluster.send(nodeId, 'CLUSTER_LOCK_GRANTED', {
                    clientId: realClientId,
                    requestId,
                    name,
                    fencingToken
                });
                return;
            }
        }

        logger.warn({ clientId, name }, 'Lock granted to unknown client');
    }

    private async processLocalOp(op: any, fromCluster: boolean, originalSenderId?: string) {
        // 1. Build context for interceptors
        const context = this.buildOpContext(originalSenderId || 'unknown', fromCluster);

        // 2. Run onBeforeOp interceptors
        try {
            const processedOp = await this.runBeforeInterceptors(op, context);
            if (!processedOp) return; // Silently dropped by interceptor
            op = processedOp;
        } catch (err) {
            logger.warn({ err, opId: op.id }, 'Interceptor rejected op');
            throw err;
        }

        // 3. Apply operation to map (shared logic)
        const { eventPayload, rejected } = await this.applyOpToMap(op, originalSenderId);

        // Skip further processing if operation was rejected by conflict resolver
        if (rejected || !eventPayload) {
            return;
        }

        // 4. Replicate to backup nodes (Hazelcast pattern: after local merge)
        // Note: Replicate if we are the owner. This includes:
        // - Direct client operations (fromCluster=false)
        // - Operations forwarded from other nodes (fromCluster=true) where we are the owner
        // The key insight: the owner is responsible for replicating to backups,
        // regardless of whether the op originated locally or was forwarded.
        if (this.replicationPipeline) {
            const opId = op.id || `${op.mapName}:${op.key}:${Date.now()}`;
            // Fire-and-forget for EVENTUAL, or await for STRONG/QUORUM
            this.replicationPipeline.replicate(op, opId, op.key).catch(err => {
                logger.warn({ opId, key: op.key, err }, 'Replication failed (non-fatal)');
            });
        }

        // 5. Broadcast EVENT to other clients
        this.broadcast({
            type: 'SERVER_EVENT',
            payload: eventPayload,
            timestamp: this.hlc.now()
        }, originalSenderId);

        // 6. Distributed subscriptions are now handled via CLUSTER_SUB_UPDATE (Phase 14.2)
        // ReplicationPipeline handles data replication to backup nodes
        // No need for broadcastToCluster here - it was O(N) broadcast to all nodes

        // 7. Run onAfterOp interceptors
        this.runAfterInterceptors(op, context);
    }

    /**
     * === OPTIMIZATION 1: Async Batch Processing with Backpressure ===
     * Processes validated operations asynchronously after ACK has been sent.
     * Uses BackpressureRegulator to periodically force sync processing and
     * prevent unbounded accumulation of async work.
     */
    private async processBatchAsync(ops: any[], clientId: string): Promise<void> {
        // === BACKPRESSURE: Check if we should force sync processing ===
        if (this.backpressure.shouldForceSync()) {
            this.metricsService.incBackpressureSyncForced();
            await this.processBatchSync(ops, clientId);
            return;
        }

        // === BACKPRESSURE: Check and wait for capacity ===
        if (!this.backpressure.registerPending()) {
            this.metricsService.incBackpressureWaits();
            try {
                await this.backpressure.waitForCapacity();
                this.backpressure.registerPending();
            } catch (err) {
                this.metricsService.incBackpressureTimeouts();
                logger.warn({ clientId, pendingOps: ops.length }, 'Backpressure timeout - rejecting batch');
                throw new Error('Server overloaded');
            }
        }

        // Update pending ops metric
        this.metricsService.setBackpressurePendingOps(this.backpressure.getPendingOps());

        try {
            // === OPTIMIZATION 3: Batch Broadcast ===
            // Collect all events for a single batched broadcast at the end
            const batchedEvents: any[] = [];

            for (const op of ops) {
                if (this.partitionService.isLocalOwner(op.key)) {
                    try {
                        // Process without immediate broadcast (we'll batch them)
                        await this.processLocalOpForBatch(op, clientId, batchedEvents);
                    } catch (err) {
                        logger.warn({ clientId, mapName: op.mapName, key: op.key, err }, 'Op failed in async batch');
                    }
                } else {
                    // Forward to owner
                    const owner = this.partitionService.getOwner(op.key);
                    this.cluster.sendToNode(owner, {
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
                this.broadcastBatch(batchedEvents, clientId);
            }
        } finally {
            this.backpressure.completePending();
            this.metricsService.setBackpressurePendingOps(this.backpressure.getPendingOps());
        }
    }

    /**
     * === BACKPRESSURE: Synchronous Batch Processing ===
     * Processes operations synchronously, waiting for broadcast completion.
     * Used when backpressure forces sync to drain the pipeline.
     */
    private async processBatchSync(ops: any[], clientId: string): Promise<void> {
        const batchedEvents: any[] = [];

        for (const op of ops) {
            if (this.partitionService.isLocalOwner(op.key)) {
                try {
                    await this.processLocalOpForBatch(op, clientId, batchedEvents);
                } catch (err) {
                    logger.warn({ clientId, mapName: op.mapName, key: op.key, err }, 'Op failed in sync batch');
                }
            } else {
                // Forward to owner and wait for acknowledgment
                const owner = this.partitionService.getOwner(op.key);
                await this.forwardOpAndWait(op, owner);
            }
        }

        // Send batched broadcast SYNCHRONOUSLY - wait for all sends to complete
        if (batchedEvents.length > 0) {
            await this.broadcastBatchSync(batchedEvents, clientId);
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
            this.cluster.sendToNode(owner, {
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
    private async processLocalOpForBatch(op: any, clientId: string, batchedEvents: any[]): Promise<void> {
        // 1. Build context for interceptors
        const context = this.buildOpContext(clientId, false);

        // 2. Run onBeforeOp interceptors
        try {
            const processedOp = await this.runBeforeInterceptors(op, context);
            if (!processedOp) return; // Silently dropped by interceptor
            op = processedOp;
        } catch (err) {
            logger.warn({ err, opId: op.id }, 'Interceptor rejected op in batch');
            throw err;
        }

        // 3. Apply operation to map (shared logic)
        const { eventPayload, rejected } = await this.applyOpToMap(op, clientId);

        // Skip further processing if operation was rejected by conflict resolver
        if (rejected || !eventPayload) {
            return;
        }

        // 4. Replicate to backup nodes (Hazelcast pattern: after local merge)
        if (this.replicationPipeline) {
            const opId = op.id || `${op.mapName}:${op.key}:${Date.now()}`;
            // Fire-and-forget for batch operations (EVENTUAL by default)
            this.replicationPipeline.replicate(op, opId, op.key).catch(err => {
                logger.warn({ opId, key: op.key, err }, 'Batch replication failed (non-fatal)');
            });
        }

        // 5. Collect event for batched broadcast (instead of immediate broadcast)
        batchedEvents.push(eventPayload);

        // 6. Distributed subscriptions are now handled via CLUSTER_SUB_UPDATE (Phase 14.2)
        // ReplicationPipeline handles data replication to backup nodes
        // No need for broadcastToCluster here - it was O(N) broadcast to all nodes

        // 7. Run onAfterOp interceptors
        this.runAfterInterceptors(op, context);
    }

    /**
     * Get or create a map by name.
     * Delegates to StorageManager.
     */
    public getMap(name: string, typeHint: 'LWW' | 'OR' = 'LWW'): LWWMap<string, any> | ORMap<string, any> {
        return this.storageManager.getMap(name, typeHint);
    }

    /**
     * Returns map after ensuring it's fully loaded from storage.
     * Use this for queries to avoid returning empty results during initial load.
     * Delegates to StorageManager.
     */
    public async getMapAsync(name: string, typeHint: 'LWW' | 'OR' = 'LWW'): Promise<LWWMap<string, any> | ORMap<string, any>> {
        return this.storageManager.getMapAsync(name, typeHint);
    }

    /**
     * Performs garbage collection (for testing/manual invocation).
     * Delegates to GCHandler.
     */
    public performGarbageCollection(olderThan: Timestamp): void {
        this.gcHandler.performGarbageCollection(olderThan);
    }

    /**
     * Phase 10.04: Get local record for anti-entropy repair
     * Returns the LWWRecord for a key, used by RepairScheduler
     */
    private getLocalRecord(key: string): LWWRecord<any> | null {
        // Parse key format: "mapName:key"
        const separatorIndex = key.indexOf(':');
        if (separatorIndex === -1) {
            return null;
        }
        const mapName = key.substring(0, separatorIndex);
        const actualKey = key.substring(separatorIndex + 1);

        const map = this.storageManager.getMaps().get(mapName);
        if (!map || !(map instanceof LWWMap)) {
            return null;
        }

        return map.getRecord(actualKey) ?? null;
    }

    /**
     * Phase 10.04: Apply repaired record from anti-entropy repair
     * Used by RepairScheduler to apply resolved conflicts
     */
    private applyRepairRecord(key: string, record: LWWRecord<any>): void {
        // Parse key format: "mapName:key"
        const separatorIndex = key.indexOf(':');
        if (separatorIndex === -1) {
            logger.warn({ key }, 'Invalid key format for repair');
            return;
        }
        const mapName = key.substring(0, separatorIndex);
        const actualKey = key.substring(separatorIndex + 1);

        const map = this.getMap(mapName, 'LWW') as LWWMap<string, any>;
        const existingRecord = map.getRecord(actualKey);

        // Only apply if the repaired record is newer (LWW semantics)
        if (!existingRecord || record.timestamp.millis > existingRecord.timestamp.millis ||
            (record.timestamp.millis === existingRecord.timestamp.millis &&
             record.timestamp.counter > existingRecord.timestamp.counter)) {
            map.merge(actualKey, record);
            logger.debug({ mapName, key: actualKey }, 'Applied repair record');

            // Persist to storage
            if (this.storage) {
                this.storage.store(mapName, actualKey, record).catch((err: Error) => {
                    logger.error({ err, mapName, key: actualKey }, 'Failed to persist repair record');
                });
            }

            // Update Merkle tree
            if (this.merkleTreeManager) {
                const partitionId = this.partitionService.getPartitionId(actualKey);
                this.merkleTreeManager.updateRecord(partitionId, actualKey, record);
            }
        }
    }


    // ============ Heartbeat Methods ============

    /**
     * Starts the periodic check for dead clients (those that haven't sent PING).
     */
    private startHeartbeatCheck() {
        this.heartbeatCheckInterval = setInterval(() => {
            this.evictDeadClients();
        }, CLIENT_HEARTBEAT_CHECK_INTERVAL_MS);
    }

    /**
     * Handles incoming PING message from client.
     * Responds with PONG immediately.
     */
    private handlePing(client: ClientConnection, clientTimestamp: number): void {
        client.lastPingReceived = Date.now();

        const pongMessage = {
            type: 'PONG',
            timestamp: clientTimestamp,
            serverTime: Date.now(),
        };

        // PONG is urgent - bypass batching for accurate RTT measurement
        client.writer.write(pongMessage, true);
    }

    /**
     * Checks if a client is still alive based on heartbeat.
     * Delegates to ConnectionManager.
     */
    public isClientAlive(clientId: string): boolean {
        return this.connectionManager.isClientAlive(clientId);
    }

    /**
     * Returns how long the client has been idle (no PING received).
     * Delegates to ConnectionManager.
     */
    public getClientIdleTime(clientId: string): number {
        return this.connectionManager.getClientIdleTime(clientId);
    }

    /**
     * Evicts clients that haven't sent a PING within the timeout period.
     */
    private evictDeadClients(): void {
        const now = Date.now();
        const deadClients: string[] = [];

        for (const [clientId, client] of this.connectionManager.getClients()) {
            // Only check authenticated clients (unauthenticated ones will timeout via auth mechanism)
            if (client.isAuthenticated) {
                const idleTime = now - client.lastPingReceived;
                if (idleTime > CLIENT_HEARTBEAT_TIMEOUT_MS) {
                    deadClients.push(clientId);
                }
            }
        }

        for (const clientId of deadClients) {
            const client = this.connectionManager.getClient(clientId);
            if (client) {
                logger.warn({
                    clientId,
                    idleTime: now - client.lastPingReceived,
                    timeoutMs: CLIENT_HEARTBEAT_TIMEOUT_MS,
                }, 'Evicting dead client (heartbeat timeout)');

                // Close the connection
                if (client.socket.readyState === WebSocket.OPEN) {
                    client.socket.close(4002, 'Heartbeat timeout');
                }
            }
        }
    }

    private buildTLSOptions(config: TLSConfig): HttpsServerOptions {
        const options: HttpsServerOptions = {
            cert: readFileSync(config.certPath),
            key: readFileSync(config.keyPath),
            minVersion: config.minVersion || 'TLSv1.2',
        };

        if (config.caCertPath) {
            options.ca = readFileSync(config.caCertPath);
        }

        if (config.ciphers) {
            options.ciphers = config.ciphers;
        }

        if (config.passphrase) {
            options.passphrase = config.passphrase;
        }

        return options;
    }

    // ============ Write Concern Methods (Phase 5.01) ============

    /**
     * Get effective Write Concern level for an operation.
     * Per-op writeConcern overrides batch-level.
     */
    private getEffectiveWriteConcern(
        opWriteConcern: WriteConcernValue | undefined,
        batchWriteConcern: WriteConcernValue | undefined
    ): WriteConcernValue | undefined {
        return opWriteConcern ?? batchWriteConcern;
    }

    /**
     * Convert string WriteConcern value to enum.
     */
    private stringToWriteConcern(value: WriteConcernValue | undefined): WriteConcern {
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
    private async processBatchAsyncWithWriteConcern(
        ops: any[],
        clientId: string,
        batchWriteConcern?: WriteConcernValue,
        batchTimeout?: number
    ): Promise<void> {
        // === BACKPRESSURE: Check if we should force sync processing ===
        if (this.backpressure.shouldForceSync()) {
            this.metricsService.incBackpressureSyncForced();
            await this.processBatchSyncWithWriteConcern(ops, clientId, batchWriteConcern, batchTimeout);
            return;
        }

        // === BACKPRESSURE: Check and wait for capacity ===
        if (!this.backpressure.registerPending()) {
            this.metricsService.incBackpressureWaits();
            try {
                await this.backpressure.waitForCapacity();
                this.backpressure.registerPending();
            } catch (err) {
                this.metricsService.incBackpressureTimeouts();
                logger.warn({ clientId, pendingOps: ops.length }, 'Backpressure timeout - rejecting batch');
                // Fail all pending operations
                for (const op of ops) {
                    if (op.id) {
                        this.writeAckManager.failPending(op.id, 'Server overloaded');
                    }
                }
                throw new Error('Server overloaded');
            }
        }

        // Update pending ops metric
        this.metricsService.setBackpressurePendingOps(this.backpressure.getPendingOps());

        try {
            // === OPTIMIZATION 3: Batch Broadcast ===
            // Collect all events for a single batched broadcast at the end
            const batchedEvents: any[] = [];

            for (const op of ops) {
                if (this.partitionService.isLocalOwner(op.key)) {
                    try {
                        // Process operation with Write Concern tracking
                        await this.processLocalOpWithWriteConcern(op, clientId, batchedEvents, batchWriteConcern);
                    } catch (err) {
                        logger.warn({ clientId, mapName: op.mapName, key: op.key, err }, 'Op failed in async batch');
                        // Fail the pending write
                        if (op.id) {
                            this.writeAckManager.failPending(op.id, String(err));
                        }
                    }
                } else {
                    // Forward to owner
                    const owner = this.partitionService.getOwner(op.key);
                    this.cluster.sendToNode(owner, {
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
                        this.writeAckManager.notifyLevel(op.id, WriteConcern.REPLICATED);
                    }
                }
            }

            // Send batched broadcast if we have events
            if (batchedEvents.length > 0) {
                this.broadcastBatch(batchedEvents, clientId);
                // Notify REPLICATED for all ops that were broadcast
                for (const op of ops) {
                    if (op.id && this.partitionService.isLocalOwner(op.key)) {
                        this.writeAckManager.notifyLevel(op.id, WriteConcern.REPLICATED);
                    }
                }
            }
        } finally {
            this.backpressure.completePending();
            this.metricsService.setBackpressurePendingOps(this.backpressure.getPendingOps());
        }
    }

    /**
     * Synchronous batch processing with Write Concern.
     */
    private async processBatchSyncWithWriteConcern(
        ops: any[],
        clientId: string,
        batchWriteConcern?: WriteConcernValue,
        batchTimeout?: number
    ): Promise<void> {
        const batchedEvents: any[] = [];

        for (const op of ops) {
            if (this.partitionService.isLocalOwner(op.key)) {
                try {
                    await this.processLocalOpWithWriteConcern(op, clientId, batchedEvents, batchWriteConcern);
                } catch (err) {
                    logger.warn({ clientId, mapName: op.mapName, key: op.key, err }, 'Op failed in sync batch');
                    if (op.id) {
                        this.writeAckManager.failPending(op.id, String(err));
                    }
                }
            } else {
                // Forward to owner and wait for acknowledgment
                const owner = this.partitionService.getOwner(op.key);
                await this.forwardOpAndWait(op, owner);
                // Mark REPLICATED after forwarding
                if (op.id) {
                    this.writeAckManager.notifyLevel(op.id, WriteConcern.REPLICATED);
                }
            }
        }

        // Send batched broadcast SYNCHRONOUSLY - wait for all sends to complete
        if (batchedEvents.length > 0) {
            await this.broadcastBatchSync(batchedEvents, clientId);
            // Notify REPLICATED for all local ops
            for (const op of ops) {
                if (op.id && this.partitionService.isLocalOwner(op.key)) {
                    this.writeAckManager.notifyLevel(op.id, WriteConcern.REPLICATED);
                }
            }
        }
    }

    /**
     * Process a single operation with Write Concern level notifications.
     */
    private async processLocalOpWithWriteConcern(
        op: any,
        clientId: string,
        batchedEvents: any[],
        batchWriteConcern?: WriteConcernValue
    ): Promise<void> {
        // 1. Build context for interceptors
        const context = this.buildOpContext(clientId, false);

        // 2. Run onBeforeOp interceptors
        try {
            const processedOp = await this.runBeforeInterceptors(op, context);
            if (!processedOp) {
                // Silently dropped by interceptor - fail the pending write
                if (op.id) {
                    this.writeAckManager.failPending(op.id, 'Dropped by interceptor');
                }
                return;
            }
            op = processedOp;
        } catch (err) {
            logger.warn({ opId: op.id, err }, 'Interceptor rejected op');
            if (op.id) {
                this.writeAckManager.failPending(op.id, String(err));
            }
            return;
        }

        // 3. Apply operation to map
        const { eventPayload, rejected } = await this.applyOpToMap(op, clientId);

        // If rejected by conflict resolver, fail the pending write
        if (rejected) {
            if (op.id) {
                this.writeAckManager.failPending(op.id, 'Rejected by conflict resolver');
            }
            return;
        }

        // 4. Notify APPLIED level (CRDT merged)
        if (op.id) {
            this.writeAckManager.notifyLevel(op.id, WriteConcern.APPLIED);
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
        if (effectiveWriteConcern === 'PERSISTED' && this.storage) {
            try {
                // Wait for storage write to complete
                await this.persistOpSync(op);
                if (op.id) {
                    this.writeAckManager.notifyLevel(op.id, WriteConcern.PERSISTED);
                }
            } catch (err) {
                logger.error({ opId: op.id, err }, 'Persistence failed');
                if (op.id) {
                    this.writeAckManager.failPending(op.id, `Persistence failed: ${err}`);
                }
            }
        } else if (this.storage && op.id) {
            // Fire-and-forget persistence for non-PERSISTED writes
            this.persistOpAsync(op).catch(err => {
                logger.error({ opId: op.id, err }, 'Async persistence failed');
            });
        }

        // 7. Run onAfterOp interceptors
        try {
            const serverOp: ServerOp = {
                mapName: op.mapName,
                key: op.key,
                opType: op.opType || (op.record?.value === null ? 'REMOVE' : 'PUT'),
                record: op.record,
                orRecord: op.orRecord,
                orTag: op.orTag,
            };
            await this.runAfterInterceptors(serverOp, context);
        } catch (err) {
            logger.warn({ opId: op.id, err }, 'onAfterOp interceptor failed');
        }
    }

    /**
     * Persist operation synchronously (blocking).
     * Used for PERSISTED Write Concern.
     */
    private async persistOpSync(op: any): Promise<void> {
        if (!this.storage) return;

        const isORMapOp = op.opType === 'OR_ADD' || op.opType === 'OR_REMOVE' || op.orRecord || op.orTag;

        if (isORMapOp) {
            const orMap = this.getMap(op.mapName, 'OR') as ORMap<string, any>;
            const records = orMap.getRecords(op.key);
            const tombstones = orMap.getTombstones();

            if (records.length > 0) {
                await this.storage.store(op.mapName, op.key, { type: 'OR', records } as ORMapValue<any>);
            } else {
                await this.storage.delete(op.mapName, op.key);
            }

            if (tombstones.length > 0) {
                await this.storage.store(op.mapName, '__tombstones__', { type: 'OR_TOMBSTONES', tags: tombstones } as ORMapTombstones);
            }
        } else {
            const lwwMap = this.getMap(op.mapName, 'LWW') as LWWMap<string, any>;
            const record = lwwMap.getRecord(op.key);
            if (record) {
                await this.storage.store(op.mapName, op.key, record);
            }
        }
    }

    /**
     * Persist operation asynchronously (fire-and-forget).
     * Used for non-PERSISTED Write Concern levels.
     */
    private async persistOpAsync(op: any): Promise<void> {
        return this.persistOpSync(op);
    }
}
