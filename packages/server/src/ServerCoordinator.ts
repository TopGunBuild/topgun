import { createServer as createHttpServer, Server as HttpServer } from 'http';
import { createServer as createHttpsServer, Server as HttpsServer, ServerOptions as HttpsServerOptions } from 'https';
import { readFileSync } from 'fs';
import * as net from 'net';
import { WebSocketServer, WebSocket } from 'ws';
import { HLC, LWWMap, ORMap, MerkleTree, serialize, deserialize, PermissionPolicy, Principal, PermissionType, Timestamp, LWWRecord, ORMapRecord, MessageSchema, WriteConcern, WriteConcernValue, ConsistencyLevel, ReplicationConfig, DEFAULT_REPLICATION_CONFIG, IndexedLWWMap, IndexedORMap, QueryCursor, DEFAULT_QUERY_CURSOR_MAX_AGE_MS, PARTITION_COUNT, type QueryExpression as CoreQuery } from '@topgunbuild/core';
import { IServerStorage, StorageValue, ORMapValue, ORMapTombstones } from './storage/IServerStorage';
import { IInterceptor, ServerOp, OpContext, ConnectionContext } from './interceptor/IInterceptor';
import * as jwt from 'jsonwebtoken';
import * as crypto from 'crypto';
import { QueryRegistry, Subscription } from './query/QueryRegistry';

const GC_INTERVAL_MS = 60 * 60 * 1000; // 1 hour
const GC_AGE_MS = 30 * 24 * 60 * 60 * 1000; // 30 days
const CLIENT_HEARTBEAT_TIMEOUT_MS = 20000; // 20 seconds - evict clients that haven't pinged
const CLIENT_HEARTBEAT_CHECK_INTERVAL_MS = 5000; // Check for dead clients every 5 seconds
import { TopicManager } from './topic/TopicManager';
import { ClusterManager } from './cluster/ClusterManager';
import { PartitionService } from './cluster/PartitionService';
import { LockManager } from './cluster/LockManager';
import { executeQuery, Query } from './query/Matcher';
import { SecurityManager } from './security/SecurityManager';
import { logger } from './utils/logger';
import { MetricsService } from './monitoring/MetricsService';
import { SystemManager } from './system/SystemManager';
import { TLSConfig, ClusterTLSConfig } from './types/TLSConfig';
import { StripedEventExecutor } from './utils/StripedEventExecutor';
import { BackpressureRegulator } from './utils/BackpressureRegulator';
import { CoalescingWriter, CoalescingWriterOptions } from './utils/CoalescingWriter';
import { coalescingPresets, CoalescingPreset } from './utils/coalescingPresets';
import { ConnectionRateLimiter } from './utils/ConnectionRateLimiter';
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

interface ClientConnection {
    id: string;
    socket: WebSocket;
    writer: CoalescingWriter; // Per-connection write coalescing
    principal?: Principal; // Auth info
    isAuthenticated: boolean;
    subscriptions: Set<string>; // Set of Query IDs
    lastActiveHlc: Timestamp;
    lastPingReceived: number; // Date.now() of last PING received
}

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
    private clients: Map<string, ClientConnection> = new Map();

    // Interceptors
    private interceptors: IInterceptor[] = [];

    // In-memory storage (partitioned later)
    private maps: Map<string, LWWMap<string, any> | ORMap<string, any>> = new Map();
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
    private systemManager!: SystemManager;

    private pendingClusterQueries: Map<string, PendingClusterQuery> = new Map();
    private gcInterval?: NodeJS.Timeout;
    private heartbeatCheckInterval?: NodeJS.Timeout;

    // GC Consensus State
    private gcReports: Map<string, Timestamp> = new Map();

    // Track map loading state to avoid returning empty results during async load
    private mapLoadingPromises: Map<string, Promise<void>> = new Map();

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
        // Handle JWT_SECRET with escaped newlines (e.g., from Docker/Dokploy env vars)
        const rawSecret = config.jwtSecret || process.env.JWT_SECRET || 'topgun-secret-dev';
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
            getMaps: () => this.maps,
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
            getMaps: () => this.maps,
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
                        connections: this.clients.size,
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
                    const client = this.clients.get(clientId);
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
                const map = this.maps.get(mapName);
                if (!map) return undefined;
                return map.get(key);
            });
            // Phase 11.1b: Set up search update callback for live subscriptions
            this.searchCoordinator.setSendUpdateCallback((clientId, subscriptionId, key, value, score, matchedTerms, type) => {
                const client = this.clients.get(clientId);
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

            this.systemManager = new SystemManager(
                this.cluster,
                this.metricsService,
                (name) => this.getMap(name) as LWWMap<string, any>
            );

            this.setupClusterListeners();
            this.cluster.start().then((actualClusterPort) => {
                this._actualClusterPort = actualClusterPort;
                this.metricsService.setClusterMembers(this.cluster.getMembers().length);
                logger.info({ clusterPort: this._actualClusterPort }, 'Cluster started');
                this.systemManager.start();
                this._readyResolve();
            }).catch((err) => {
                // Fallback for ClusterManager that doesn't return port
                this._actualClusterPort = clusterPort;
                this.metricsService.setClusterMembers(this.cluster.getMembers().length);
                logger.info({ clusterPort: this._actualClusterPort }, 'Cluster started (sync)');
                this.systemManager.start();
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

        this.startGarbageCollection();
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
                await this.getMapAsync(mapName);
                
                const map = this.maps.get(mapName);
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
        const map = this.maps.get(mapName);
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
        logger.info(`Closing ${this.clients.size} client connections...`);
        const shutdownMsg = serialize({ type: 'SHUTDOWN_PENDING', retryAfter: 5000 });

        for (const client of this.clients.values()) {
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
        this.clients.clear();

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
        if (this.gcInterval) {
            clearInterval(this.gcInterval);
            this.gcInterval = undefined;
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

        const connection: ClientConnection = {
            id: clientId,
            socket: ws,
            writer,
            isAuthenticated: false,
            subscriptions: new Set(),
            lastActiveHlc: this.hlc.now(), // Initialize with current time
            lastPingReceived: Date.now(), // Initialize heartbeat tracking
        };
        this.clients.set(clientId, connection);
        this.metricsService.setConnectedClients(this.clients.size);

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
            this.clients.delete(clientId);
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

            this.clients.delete(clientId);
            this.metricsService.setConnectedClients(this.clients.size);
        });

        // Send Auth Challenge immediately
        ws.send(serialize({ type: 'AUTH_REQUIRED' }));
    }

    private async handleMessage(client: ClientConnection, rawMessage: any) {
        // Validation with Zod
        const parseResult = MessageSchema.safeParse(rawMessage);
        if (!parseResult.success) {
            logger.error({ clientId: client.id, error: parseResult.error }, 'Invalid message format from client');
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

        // Handshake / Auth handling
        if (!client.isAuthenticated) {
            if (message.type === 'AUTH') {
                const token = message.token;
                try {
                    // Verify JWT - support both HS256 (symmetric) and RS256 (asymmetric/Clerk)
                    const isRSAKey = this.jwtSecret.includes('-----BEGIN');
                    const verifyOptions: jwt.VerifyOptions = isRSAKey
                        ? { algorithms: ['RS256'] }
                        : { algorithms: ['HS256'] };
                    const decoded = jwt.verify(token, this.jwtSecret, verifyOptions) as any;
                    // Ensure roles exist
                    if (!decoded.roles) {
                        decoded.roles = ['USER']; // Default role
                    }
                    // Ensure userId exists (map from sub if needed)
                    if (!decoded.userId && decoded.sub) {
                        decoded.userId = decoded.sub;
                    }

                    client.principal = decoded;
                    client.isAuthenticated = true;
                    logger.info({ clientId: client.id, user: client.principal!.userId || 'anon' }, 'Client authenticated');

                    // Mark connection as established (handshake complete)
                    if (this.rateLimitingEnabled) {
                        this.rateLimiter.onConnectionEstablished();
                    }

                    client.writer.write({ type: 'AUTH_ACK' }, true); // urgent: bypass batching
                    return; // Stop processing this message
                } catch (e) {
                    logger.error({ clientId: client.id, err: e }, 'Auth failed');
                    client.writer.write({ type: 'AUTH_FAIL', error: 'Invalid token' }, true); // urgent
                    client.socket.close(4001, 'Unauthorized');
                }
            } else {
                // Reject any other message before auth
                client.socket.close(4001, 'Auth required');
            }
            return;
        }

        // Standard Protocol Handling (Authenticated)
        switch (message.type) {
            case 'QUERY_SUB': {
                const { queryId, mapName, query } = message.payload;

                // Check READ permission
                if (!this.securityManager.checkPermission(client.principal!, mapName, 'READ')) {
                    logger.warn({ clientId: client.id, mapName }, 'Access Denied: QUERY_SUB');
                    client.writer.write({
                        type: 'ERROR',
                        payload: { code: 403, message: `Access Denied for map ${mapName}` }
                    }, true);
                    return;
                }

                logger.info({ clientId: client.id, mapName, query }, 'Client subscribed');
                this.metricsService.incOp('SUBSCRIBE', mapName);

                // Phase 14.2: Use distributed subscription if cluster mode with multiple nodes
                if (this.distributedSubCoordinator && this.cluster && this.cluster.getMembers().length > 1) {
                    // Distributed query subscription
                    this.distributedSubCoordinator.subscribeQuery(
                        queryId,
                        client.socket,
                        mapName,
                        query
                    ).then((result) => {
                        // Apply Field Level Security to results
                        const filteredResults = result.results.map((res: any) => {
                            const filteredValue = this.securityManager.filterObject(res.value, client.principal!, mapName);
                            return { ...res, value: filteredValue };
                        });

                        client.writer.write({
                            type: 'QUERY_RESP',
                            payload: {
                                queryId,
                                results: filteredResults,
                            },
                        });

                        // Track subscription on client
                        client.subscriptions.add(queryId);

                        logger.debug({
                            clientId: client.id,
                            queryId,
                            mapName,
                            resultCount: result.results.length,
                            totalHits: result.totalHits,
                            nodes: result.registeredNodes,
                        }, 'Distributed query subscription created');
                    }).catch((err) => {
                        logger.error({ err, queryId }, 'Distributed query subscription failed');
                        client.writer.write({
                            type: 'QUERY_RESP',
                            payload: {
                                queryId,
                                results: [],
                                error: 'Failed to create distributed subscription',
                            },
                        });
                    });
                } else {
                    // Single-node fallback: use existing logic
                    // Identify all relevant nodes
                    const allMembers = this.cluster.getMembers();
                    let remoteMembers = allMembers.filter(id => !this.cluster.isLocal(id));

                    // Phase 10.03: Read-from-Replica Optimization
                    // If query targets a specific key, we can optimize by routing to a specific replica
                    // instead of broadcasting to the entire cluster.
                    const queryKey = (query as any)._id || (query as any).where?._id;

                    if (queryKey && typeof queryKey === 'string' && this.readReplicaHandler) {
                        try {
                            const targetNode = this.readReplicaHandler.selectReadNode({
                                mapName,
                                key: queryKey,
                                options: {
                                    // Default to EVENTUAL for read scaling unless specified otherwise
                                    // In future, we could extract consistency from query options if available
                                    consistency: ConsistencyLevel.EVENTUAL
                                }
                            });

                            if (targetNode) {
                                if (this.cluster.isLocal(targetNode)) {
                                    // Serve locally only
                                    remoteMembers = [];
                                    logger.debug({ clientId: client.id, mapName, key: queryKey }, 'Read optimization: Serving locally');
                                } else if (remoteMembers.includes(targetNode)) {
                                    // Serve from specific remote replica
                                    remoteMembers = [targetNode];
                                    logger.debug({ clientId: client.id, mapName, key: queryKey, targetNode }, 'Read optimization: Routing to replica');
                                }
                            }
                        } catch (e) {
                            logger.warn({ err: e }, 'Error in ReadReplicaHandler selection');
                        }
                    }

                    const requestId = crypto.randomUUID();

                    const pending: PendingClusterQuery = {
                        requestId,
                        client,
                        queryId,
                        mapName,
                        query,
                        results: [], // Will populate with local results first
                        expectedNodes: new Set(remoteMembers),
                        respondedNodes: new Set(),
                        timer: setTimeout(() => this.finalizeClusterQuery(requestId, true), 5000) // 5s timeout
                    };

                    this.pendingClusterQueries.set(requestId, pending);

                    // Execute Locally (async - wait for map to load from storage)
                    // [FIX] Using await ensures handleMessage completes only after query execution
                    // This is important for:
                    // 1. Tests that need to verify results immediately after handleMessage
                    // 2. Ensuring storage is loaded before returning results
                    try {
                        const localResults = await this.executeLocalQuery(mapName, query);
                        pending.results.push(...localResults);

                        // Scatter: Send to other nodes
                        if (remoteMembers.length > 0) {
                            for (const nodeId of remoteMembers) {
                                this.cluster.send(nodeId, 'CLUSTER_QUERY_EXEC', {
                                    requestId,
                                    mapName,
                                    query
                                });
                            }
                        } else {
                            // Single node cluster: finalize immediately
                            this.finalizeClusterQuery(requestId);
                        }
                    } catch (err) {
                        logger.error({ err, mapName }, 'Failed to execute local query');
                        // Finalize with empty results on error
                        this.finalizeClusterQuery(requestId);
                    }
                }
                break;
            }

            case 'QUERY_UNSUB': {
                const { queryId: unsubId } = message.payload;

                // Phase 14.2: Unsubscribe from distributed coordinator if in cluster mode
                if (this.distributedSubCoordinator && this.cluster && this.cluster.getMembers().length > 1) {
                    this.distributedSubCoordinator.unsubscribe(unsubId).catch((err) => {
                        logger.warn({ err, queryId: unsubId }, 'Failed to unsubscribe from distributed coordinator');
                    });
                }

                this.queryRegistry.unregister(unsubId);
                client.subscriptions.delete(unsubId);
                break;
            }

            case 'CLIENT_OP': {
                const op = message.payload;

                // Determine action type
                // LWW: op.record.value === null -> REMOVE
                // OR: OR_REMOVE or OR_ADD -> PUT (effectively)
                const isRemove = op.opType === 'REMOVE' || (op.record && op.record.value === null);
                const action: PermissionType = isRemove ? 'REMOVE' : 'PUT';
                this.metricsService.incOp(isRemove ? 'DELETE' : 'PUT', op.mapName);

                // Check Permission
                if (!this.securityManager.checkPermission(client.principal!, op.mapName, action)) {
                    logger.warn({ clientId: client.id, action, mapName: op.mapName }, 'Access Denied: Client OP');
                    client.writer.write({
                        type: 'OP_REJECTED',
                        payload: { opId: op.id, reason: 'Access Denied' }
                    });
                    return;
                }

                logger.info({ clientId: client.id, opType: op.opType, key: op.key, mapName: op.mapName }, 'Received op');

                if (this.partitionService.isLocalOwner(op.key)) {
                    this.processLocalOp(op, false, client.id).catch(err => {
                        logger.error({ clientId: client.id, err }, 'Op failed');
                        client.writer.write({
                            type: 'OP_REJECTED',
                            payload: { opId: op.id, reason: err.message || 'Internal Error' }
                        });
                    });
                } else {
                    const owner = this.partitionService.getOwner(op.key);
                    logger.info({ key: op.key, owner }, 'Forwarding op');
                    this.cluster.sendToNode(owner, op);
                }
                break;
            }

            case 'OP_BATCH': {
                const ops = message.payload.ops;
                // Extract batch-level Write Concern (Phase 5.01)
                const batchWriteConcern = (message.payload as any).writeConcern as WriteConcernValue | undefined;
                const batchTimeout = (message.payload as any).timeout as number | undefined;

                logger.info({ clientId: client.id, count: ops.length, writeConcern: batchWriteConcern }, 'Received batch');

                // === OPTIMIZATION 1: Early ACK ===
                // Fast validation pass - check permissions without processing
                const validOps: typeof ops = [];
                let rejectedCount = 0;
                let lastValidId: string | null = null;

                // Categorize ops by Write Concern for different ACK handling
                const memoryOps: typeof ops = []; // Ops that need immediate ACK (MEMORY or FIRE_AND_FORGET)
                const deferredOps: typeof ops = []; // Ops that need deferred ACK (APPLIED, REPLICATED, PERSISTED)

                for (const op of ops) {
                    const isRemove = op.opType === 'REMOVE' || (op.record && op.record.value === null);
                    const action: PermissionType = isRemove ? 'REMOVE' : 'PUT';

                    if (!this.securityManager.checkPermission(client.principal!, op.mapName, action)) {
                        rejectedCount++;
                        logger.warn({ clientId: client.id, action, mapName: op.mapName }, 'Access Denied (Batch)');
                        continue;
                    }

                    validOps.push(op);
                    if (op.id) {
                        lastValidId = op.id;
                    }

                    // Determine effective Write Concern for this operation
                    const effectiveWriteConcern = this.getEffectiveWriteConcern(op.writeConcern, batchWriteConcern);

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
                        const effectiveWriteConcern = this.getEffectiveWriteConcern(op.writeConcern, batchWriteConcern);
                        const effectiveTimeout = op.timeout ?? batchTimeout;
                        const wcLevel = this.stringToWriteConcern(effectiveWriteConcern);

                        // Register and handle the promise
                        this.writeAckManager.registerPending(op.id, wcLevel, effectiveTimeout)
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
                            this.processBatchAsyncWithWriteConcern(validOps, client.id, batchWriteConcern, batchTimeout)
                                .catch(err => {
                                    logger.error({ clientId: client.id, err }, 'Batch processing failed');
                                })
                                .finally(() => {
                                    this.pendingBatchOperations.delete(batchPromise);
                                    resolve();
                                });
                        });
                    });
                    this.pendingBatchOperations.add(batchPromise);
                }
                break;
            }

            case 'SYNC_INIT': {
                // Check READ permission
                if (!this.securityManager.checkPermission(client.principal!, message.mapName, 'READ')) {
                    logger.warn({ clientId: client.id, mapName: message.mapName }, 'Access Denied: SYNC_INIT');
                    client.writer.write({
                        type: 'ERROR',
                        payload: { code: 403, message: `Access Denied for map ${message.mapName}` }
                    }, true);
                    return;
                }

                const lastSync = message.lastSyncTimestamp || 0;
                const now = Date.now();
                if (lastSync > 0 && (now - lastSync) > GC_AGE_MS) {
                    logger.warn({ clientId: client.id, lastSync, age: now - lastSync }, 'Client too old, sending SYNC_RESET_REQUIRED');
                    client.writer.write({
                        type: 'SYNC_RESET_REQUIRED',
                        payload: { mapName: message.mapName }
                    });
                    return;
                }

                logger.info({ clientId: client.id, mapName: message.mapName }, 'Client requested sync');
                this.metricsService.incOp('GET', message.mapName);

                // [FIX] Wait for map to be fully loaded from storage before sending rootHash
                // This prevents sending rootHash=0 for maps that are still loading from PostgreSQL
                try {
                    const mapForSync = await this.getMapAsync(message.mapName);
                    if (mapForSync instanceof LWWMap) {
                        // Use the incremental Merkle Tree from LWWMap
                        const tree = mapForSync.getMerkleTree();
                        const rootHash = tree.getRootHash();

                        client.writer.write({
                            type: 'SYNC_RESP_ROOT',
                            payload: {
                                mapName: message.mapName,
                                rootHash,
                                timestamp: this.hlc.now()
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
                break;
            }

            case 'MERKLE_REQ_BUCKET': {
                // Check READ permission
                if (!this.securityManager.checkPermission(client.principal!, message.payload.mapName, 'READ')) {
                    client.writer.write({
                        type: 'ERROR',
                        payload: { code: 403, message: `Access Denied for map ${message.payload.mapName}` }
                    }, true);
                    return;
                }

                const { mapName, path } = message.payload;

                // [FIX] Wait for map to be fully loaded before accessing Merkle tree
                try {
                    const mapForBucket = await this.getMapAsync(mapName);
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
                break;
            }

            case 'LOCK_REQUEST': {
                const { requestId, name, ttl } = message.payload;

                // 1. Access Control
                // Define a convention: lock names are resources.
                // Check if user has 'WRITE' permission on "locks" map or specific lock name.
                // Since locks are ephemeral, we might treat them as a special resource "sys:locks".
                // Or just check against the lock name itself.
                // Let's use `sys:lock:${name}` pattern or just `${name}`.
                // If we use just name, it might conflict with map names if policies are strict.
                // Assuming for now that lock name represents the resource being protected.
                if (!this.securityManager.checkPermission(client.principal!, name, 'PUT')) {
                    client.writer.write({
                        // We don't have LOCK_DENIED type in schema yet?
                        // Using LOCK_RELEASED with success=false as a hack or ERROR.
                        // Ideally ERROR.
                        type: 'ERROR',
                        payload: { code: 403, message: `Access Denied for lock ${name}` }
                    }, true);
                    return;
                }

                if (this.partitionService.isLocalOwner(name)) {
                    const result = this.lockManager.acquire(name, client.id, requestId, ttl || 10000);
                    if (result.granted) {
                        client.writer.write({
                            type: 'LOCK_GRANTED',
                            payload: { requestId, name, fencingToken: result.fencingToken }
                        });
                    }
                    // If not granted, it is queued. Response sent later via event.
                } else {
                    const owner = this.partitionService.getOwner(name);
                    // 2. Cluster Reliability Check
                    if (!this.cluster.getMembers().includes(owner)) {
                        client.writer.write({
                            type: 'ERROR',
                            payload: { code: 503, message: `Lock owner ${owner} is unavailable` }
                        }, true);
                        return;
                    }

                    this.cluster.send(owner, 'CLUSTER_LOCK_REQ', {
                        originNodeId: this.cluster.config.nodeId,
                        clientId: client.id,
                        requestId,
                        name,
                        ttl
                    });
                }
                break;
            }

            case 'LOCK_RELEASE': {
                const { requestId, name, fencingToken } = message.payload;

                if (this.partitionService.isLocalOwner(name)) {
                    const success = this.lockManager.release(name, client.id, fencingToken);
                    client.writer.write({
                        type: 'LOCK_RELEASED',
                        payload: { requestId, name, success }
                    });
                } else {
                    const owner = this.partitionService.getOwner(name);
                    this.cluster.send(owner, 'CLUSTER_LOCK_RELEASE', {
                        originNodeId: this.cluster.config.nodeId,
                        clientId: client.id,
                        requestId,
                        name,
                        fencingToken
                    });
                }
                break;
            }

            case 'TOPIC_SUB': {
                const { topic } = message.payload;

                // C1: Access Control
                // We treat topics as resources.
                // Policy check: action 'READ' on resource `topic:${topic}`
                if (!this.securityManager.checkPermission(client.principal!, `topic:${topic}`, 'READ')) {
                    logger.warn({ clientId: client.id, topic }, 'Access Denied: TOPIC_SUB');
                    client.writer.write({
                        type: 'ERROR',
                        payload: { code: 403, message: `Access Denied for topic ${topic}` }
                    }, true);
                    return;
                }

                try {
                    this.topicManager.subscribe(client.id, topic);
                } catch (e: any) {
                    client.writer.write({
                        type: 'ERROR',
                        payload: { code: 400, message: e.message }
                    }, true);
                }
                break;
            }

            case 'TOPIC_UNSUB': {
                const { topic } = message.payload;
                this.topicManager.unsubscribe(client.id, topic);
                break;
            }

            case 'TOPIC_PUB': {
                const { topic, data } = message.payload;

                // C1: Access Control
                // Policy check: action 'PUT' (publish) on resource `topic:${topic}`
                if (!this.securityManager.checkPermission(client.principal!, `topic:${topic}`, 'PUT')) {
                    logger.warn({ clientId: client.id, topic }, 'Access Denied: TOPIC_PUB');
                    // No error sent back? Fire and forget usually implies silent drop or async error.
                    // But for security violations, an error is useful during dev.
                    // Spec says fire-and-forget delivery, but security rejection should ideally notify.
                    // Let's send error.
                    client.writer.write({
                        type: 'ERROR',
                        payload: { code: 403, message: `Access Denied for topic ${topic}` }
                    }, true);
                    return;
                }

                try {
                    this.topicManager.publish(topic, data, client.id);
                } catch (e: any) {
                    // Invalid topic name etc
                    client.writer.write({
                        type: 'ERROR',
                        payload: { code: 400, message: e.message }
                    }, true);
                }
                break;
            }

            // ============ Phase 5.2: PN Counter Handlers ============

            case 'COUNTER_REQUEST': {
                const { name } = message.payload;
                const response = this.counterHandler.handleCounterRequest(client.id, name);
                client.writer.write(response);
                logger.debug({ clientId: client.id, name }, 'Counter request handled');
                break;
            }

            case 'COUNTER_SYNC': {
                const { name, state } = message.payload;
                const result = this.counterHandler.handleCounterSync(client.id, name, state);

                // Send response to the syncing client
                client.writer.write(result.response);

                // Broadcast to other subscribed clients
                for (const targetClientId of result.broadcastTo) {
                    const targetClient = this.clients.get(targetClientId);
                    if (targetClient && targetClient.socket.readyState === WebSocket.OPEN) {
                        targetClient.writer.write(result.broadcastMessage);
                    }
                }
                logger.debug({ clientId: client.id, name, broadcastCount: result.broadcastTo.length }, 'Counter sync handled');
                break;
            }

            // ============ Phase 5.03: Entry Processor Handlers ============

            case 'ENTRY_PROCESS': {
                const { requestId, mapName, key, processor } = message;

                // Check PUT permission (entry processor modifies data)
                if (!this.securityManager.checkPermission(client.principal!, mapName, 'PUT')) {
                    client.writer.write({
                        type: 'ENTRY_PROCESS_RESPONSE',
                        requestId,
                        success: false,
                        error: `Access Denied for map ${mapName}`,
                    }, true);
                    break;
                }

                // Get or create the map
                const entryMap = this.getMap(mapName) as LWWMap<string, any>;

                // Execute the processor
                const { result, timestamp } = await this.entryProcessorHandler.executeOnKey(
                    entryMap,
                    key,
                    processor,
                );

                // Send response to client
                client.writer.write({
                    type: 'ENTRY_PROCESS_RESPONSE',
                    requestId,
                    success: result.success,
                    result: result.result,
                    newValue: result.newValue,
                    error: result.error,
                });

                // If successful and value changed, notify query subscribers
                if (result.success && timestamp) {
                    const record = entryMap.getRecord(key);
                    if (record) {
                        this.queryRegistry.processChange(mapName, entryMap, key, record, undefined);
                    }
                }

                logger.debug({
                    clientId: client.id,
                    mapName,
                    key,
                    processor: processor.name,
                    success: result.success,
                }, 'Entry processor executed');
                break;
            }

            case 'ENTRY_PROCESS_BATCH': {
                const { requestId, mapName, keys, processor } = message;

                // Check PUT permission
                if (!this.securityManager.checkPermission(client.principal!, mapName, 'PUT')) {
                    const errorResults: Record<string, { success: boolean; error: string }> = {};
                    for (const key of keys) {
                        errorResults[key] = {
                            success: false,
                            error: `Access Denied for map ${mapName}`,
                        };
                    }
                    client.writer.write({
                        type: 'ENTRY_PROCESS_BATCH_RESPONSE',
                        requestId,
                        results: errorResults,
                    }, true);
                    break;
                }

                // Get or create the map
                const batchMap = this.getMap(mapName) as LWWMap<string, any>;

                // Execute the processor on all keys
                const { results, timestamps } = await this.entryProcessorHandler.executeOnKeys(
                    batchMap,
                    keys,
                    processor,
                );

                // Convert Map to Record for serialization
                const resultsRecord: Record<string, {
                    success: boolean;
                    result?: unknown;
                    newValue?: unknown;
                    error?: string;
                }> = {};

                for (const [key, keyResult] of results) {
                    resultsRecord[key] = {
                        success: keyResult.success,
                        result: keyResult.result,
                        newValue: keyResult.newValue,
                        error: keyResult.error,
                    };
                }

                // Send batch response to client
                client.writer.write({
                    type: 'ENTRY_PROCESS_BATCH_RESPONSE',
                    requestId,
                    results: resultsRecord,
                });

                // Notify query subscribers about changes
                for (const [key] of timestamps) {
                    const record = batchMap.getRecord(key);
                    if (record) {
                        this.queryRegistry.processChange(mapName, batchMap, key, record, undefined);
                    }
                }

                logger.debug({
                    clientId: client.id,
                    mapName,
                    keyCount: keys.length,
                    processor: processor.name,
                    successCount: Array.from(results.values()).filter(r => r.success).length,
                }, 'Entry processor batch executed');
                break;
            }

            // ============ Phase 5.05: Conflict Resolver Handlers ============

            case 'REGISTER_RESOLVER': {
                const { requestId, mapName, resolver } = message;

                // Check PUT permission (resolver registration is a privileged operation)
                if (!this.securityManager.checkPermission(client.principal!, mapName, 'PUT')) {
                    client.writer.write({
                        type: 'REGISTER_RESOLVER_RESPONSE',
                        requestId,
                        success: false,
                        error: `Access Denied for map ${mapName}`,
                    }, true);
                    break;
                }

                try {
                    this.conflictResolverHandler.registerResolver(
                        mapName,
                        {
                            name: resolver.name,
                            code: resolver.code,
                            priority: resolver.priority,
                            keyPattern: resolver.keyPattern,
                        },
                        client.id,
                    );

                    client.writer.write({
                        type: 'REGISTER_RESOLVER_RESPONSE',
                        requestId,
                        success: true,
                    });

                    logger.info({
                        clientId: client.id,
                        mapName,
                        resolverName: resolver.name,
                        priority: resolver.priority,
                    }, 'Conflict resolver registered');
                } catch (err) {
                    const errorMessage = err instanceof Error ? err.message : String(err);
                    client.writer.write({
                        type: 'REGISTER_RESOLVER_RESPONSE',
                        requestId,
                        success: false,
                        error: errorMessage,
                    }, true);
                    logger.warn({
                        clientId: client.id,
                        mapName,
                        error: errorMessage,
                    }, 'Failed to register conflict resolver');
                }
                break;
            }

            case 'UNREGISTER_RESOLVER': {
                const { requestId, mapName, resolverName } = message;

                // Check PUT permission
                if (!this.securityManager.checkPermission(client.principal!, mapName, 'PUT')) {
                    client.writer.write({
                        type: 'UNREGISTER_RESOLVER_RESPONSE',
                        requestId,
                        success: false,
                        error: `Access Denied for map ${mapName}`,
                    }, true);
                    break;
                }

                const removed = this.conflictResolverHandler.unregisterResolver(
                    mapName,
                    resolverName,
                    client.id,
                );

                client.writer.write({
                    type: 'UNREGISTER_RESOLVER_RESPONSE',
                    requestId,
                    success: removed,
                    error: removed ? undefined : 'Resolver not found or not owned by this client',
                });

                if (removed) {
                    logger.info({
                        clientId: client.id,
                        mapName,
                        resolverName,
                    }, 'Conflict resolver unregistered');
                }
                break;
            }

            case 'LIST_RESOLVERS': {
                const { requestId, mapName } = message;

                // Check READ permission if mapName specified
                if (mapName && !this.securityManager.checkPermission(client.principal!, mapName, 'READ')) {
                    client.writer.write({
                        type: 'LIST_RESOLVERS_RESPONSE',
                        requestId,
                        resolvers: [],
                    });
                    break;
                }

                const resolvers = this.conflictResolverHandler.listResolvers(mapName);

                client.writer.write({
                    type: 'LIST_RESOLVERS_RESPONSE',
                    requestId,
                    resolvers,
                });
                break;
            }

            // ============ Phase 4: Partition Map Request Handler ============

            case 'PARTITION_MAP_REQUEST': {
                // Client is requesting the current partition map
                // This is used for cluster-aware routing
                const clientVersion = message.payload?.currentVersion ?? 0;
                const currentMap = this.partitionService.getPartitionMap();

                // Only send if client has stale version or no version
                if (clientVersion < currentMap.version) {
                    client.writer.write({
                        type: 'PARTITION_MAP',
                        payload: currentMap
                    });
                    logger.debug({
                        clientId: client.id,
                        clientVersion,
                        serverVersion: currentMap.version
                    }, 'Sent partition map to client');
                }
                break;
            }

            // ============ ORMap Sync Message Handlers ============

            case 'ORMAP_SYNC_INIT': {
                // Check READ permission
                if (!this.securityManager.checkPermission(client.principal!, message.mapName, 'READ')) {
                    client.writer.write({
                        type: 'ERROR',
                        payload: { code: 403, message: `Access Denied for map ${message.mapName}` }
                    }, true);
                    return;
                }

                const lastSync = message.lastSyncTimestamp || 0;
                const now = Date.now();
                if (lastSync > 0 && (now - lastSync) > GC_AGE_MS) {
                    logger.warn({ clientId: client.id, lastSync, age: now - lastSync }, 'ORMap client too old, sending SYNC_RESET_REQUIRED');
                    client.writer.write({
                        type: 'SYNC_RESET_REQUIRED',
                        payload: { mapName: message.mapName }
                    });
                    return;
                }

                logger.info({ clientId: client.id, mapName: message.mapName }, 'Client requested ORMap sync');
                this.metricsService.incOp('GET', message.mapName);

                try {
                    const mapForSync = await this.getMapAsync(message.mapName, 'OR');
                    if (mapForSync instanceof ORMap) {
                        const tree = mapForSync.getMerkleTree();
                        const rootHash = tree.getRootHash();

                        client.writer.write({
                            type: 'ORMAP_SYNC_RESP_ROOT',
                            payload: {
                                mapName: message.mapName,
                                rootHash,
                                timestamp: this.hlc.now()
                            }
                        });
                    } else {
                        // It's actually an LWWMap, client should use SYNC_INIT
                        client.writer.write({
                            type: 'ERROR',
                            payload: { code: 400, message: `Map ${message.mapName} is not an ORMap` }
                        }, true);
                    }
                } catch (err) {
                    logger.error({ err, mapName: message.mapName }, 'Failed to load map for ORMAP_SYNC_INIT');
                    client.writer.write({
                        type: 'ERROR',
                        payload: { code: 500, message: `Failed to load map ${message.mapName}` }
                    }, true);
                }
                break;
            }

            case 'ORMAP_MERKLE_REQ_BUCKET': {
                // Check READ permission
                if (!this.securityManager.checkPermission(client.principal!, message.payload.mapName, 'READ')) {
                    client.writer.write({
                        type: 'ERROR',
                        payload: { code: 403, message: `Access Denied for map ${message.payload.mapName}` }
                    }, true);
                    return;
                }

                const { mapName, path } = message.payload;

                try {
                    const mapForBucket = await this.getMapAsync(mapName, 'OR');
                    if (mapForBucket instanceof ORMap) {
                        const tree = mapForBucket.getMerkleTree();
                        const buckets = tree.getBuckets(path);
                        const isLeaf = tree.isLeaf(path);

                        if (isLeaf) {
                            // This is a leaf node - send actual records
                            const keys = tree.getKeysInBucket(path);
                            const entries: Array<{ key: string; records: ORMapRecord<any>[]; tombstones: string[] }> = [];

                            for (const key of keys) {
                                const recordsMap = mapForBucket.getRecordsMap(key);
                                if (recordsMap && recordsMap.size > 0) {
                                    entries.push({
                                        key,
                                        records: Array.from(recordsMap.values()),
                                        tombstones: mapForBucket.getTombstones()
                                    });
                                }
                            }

                            client.writer.write({
                                type: 'ORMAP_SYNC_RESP_LEAF',
                                payload: { mapName, path, entries }
                            });
                        } else {
                            // Not a leaf - send bucket hashes
                            client.writer.write({
                                type: 'ORMAP_SYNC_RESP_BUCKETS',
                                payload: { mapName, path, buckets }
                            });
                        }
                    }
                } catch (err) {
                    logger.error({ err, mapName }, 'Failed to load map for ORMAP_MERKLE_REQ_BUCKET');
                }
                break;
            }

            case 'ORMAP_DIFF_REQUEST': {
                // Check READ permission
                if (!this.securityManager.checkPermission(client.principal!, message.payload.mapName, 'READ')) {
                    client.writer.write({
                        type: 'ERROR',
                        payload: { code: 403, message: `Access Denied for map ${message.payload.mapName}` }
                    }, true);
                    return;
                }

                const { mapName: diffMapName, keys } = message.payload;

                try {
                    const mapForDiff = await this.getMapAsync(diffMapName, 'OR');
                    if (mapForDiff instanceof ORMap) {
                        const entries: Array<{ key: string; records: ORMapRecord<any>[]; tombstones: string[] }> = [];
                        const allTombstones = mapForDiff.getTombstones();

                        for (const key of keys) {
                            const recordsMap = mapForDiff.getRecordsMap(key);
                            entries.push({
                                key,
                                records: recordsMap ? Array.from(recordsMap.values()) : [],
                                tombstones: allTombstones
                            });
                        }

                        client.writer.write({
                            type: 'ORMAP_DIFF_RESPONSE',
                            payload: { mapName: diffMapName, entries }
                        });
                    }
                } catch (err) {
                    logger.error({ err, mapName: diffMapName }, 'Failed to load map for ORMAP_DIFF_REQUEST');
                }
                break;
            }

            case 'ORMAP_PUSH_DIFF': {
                // Check WRITE permission
                if (!this.securityManager.checkPermission(client.principal!, message.payload.mapName, 'PUT')) {
                    client.writer.write({
                        type: 'ERROR',
                        payload: { code: 403, message: `Access Denied for map ${message.payload.mapName}` }
                    }, true);
                    return;
                }

                const { mapName: pushMapName, entries: pushEntries } = message.payload;

                try {
                    const mapForPush = await this.getMapAsync(pushMapName, 'OR');
                    if (mapForPush instanceof ORMap) {
                        let totalAdded = 0;
                        let totalUpdated = 0;

                        for (const entry of pushEntries) {
                            const { key, records, tombstones } = entry;
                            const result = mapForPush.mergeKey(key, records, tombstones);
                            totalAdded += result.added;
                            totalUpdated += result.updated;
                        }

                        if (totalAdded > 0 || totalUpdated > 0) {
                            logger.info({ mapName: pushMapName, added: totalAdded, updated: totalUpdated, clientId: client.id }, 'Merged ORMap diff from client');

                            // Broadcast changes to other clients
                            for (const entry of pushEntries) {
                                for (const record of entry.records) {
                                    this.broadcast({
                                        type: 'SERVER_EVENT',
                                        payload: {
                                            mapName: pushMapName,
                                            eventType: 'OR_ADD',
                                            key: entry.key,
                                            orRecord: record
                                        }
                                    }, client.id);
                                }
                            }

                            // Persist to storage
                            if (this.storage) {
                                for (const entry of pushEntries) {
                                    const recordsMap = mapForPush.getRecordsMap(entry.key);
                                    if (recordsMap && recordsMap.size > 0) {
                                        await this.storage.store(pushMapName, entry.key, {
                                            type: 'OR',
                                            records: Array.from(recordsMap.values())
                                        });
                                    }
                                }
                            }
                        }
                    }
                } catch (err) {
                    logger.error({ err, mapName: pushMapName }, 'Failed to process ORMAP_PUSH_DIFF');
                }
                break;
            }

            // === Event Journal Messages (Phase 5.04) ===

            case 'JOURNAL_SUBSCRIBE': {
                if (!this.eventJournalService) {
                    client.writer.write({
                        type: 'ERROR',
                        payload: { code: 503, message: 'Event journal not enabled' }
                    }, true);
                    break;
                }

                const { requestId, fromSequence, mapName, types } = message;
                const subscriptionId = requestId;

                // Store subscription metadata
                this.journalSubscriptions.set(subscriptionId, {
                    clientId: client.id,
                    mapName,
                    types,
                });

                // Subscribe to journal events
                const unsubscribe = this.eventJournalService.subscribe(
                    (event) => {
                        // Apply filters
                        if (mapName && event.mapName !== mapName) return;
                        if (types && types.length > 0 && !types.includes(event.type)) return;

                        // Check if client still connected
                        const clientConn = this.clients.get(client.id);
                        if (!clientConn) {
                            unsubscribe();
                            this.journalSubscriptions.delete(subscriptionId);
                            return;
                        }

                        // Send event to client
                        clientConn.writer.write({
                            type: 'JOURNAL_EVENT',
                            event: {
                                sequence: event.sequence.toString(),
                                type: event.type,
                                mapName: event.mapName,
                                key: event.key,
                                value: event.value,
                                previousValue: event.previousValue,
                                timestamp: event.timestamp,
                                nodeId: event.nodeId,
                                metadata: event.metadata,
                            },
                        });
                    },
                    fromSequence ? BigInt(fromSequence) : undefined
                );

                logger.info({ clientId: client.id, subscriptionId, mapName }, 'Journal subscription created');
                break;
            }

            case 'JOURNAL_UNSUBSCRIBE': {
                const { subscriptionId } = message;
                this.journalSubscriptions.delete(subscriptionId);
                logger.info({ clientId: client.id, subscriptionId }, 'Journal subscription removed');
                break;
            }

            case 'JOURNAL_READ': {
                if (!this.eventJournalService) {
                    client.writer.write({
                        type: 'ERROR',
                        payload: { code: 503, message: 'Event journal not enabled' }
                    }, true);
                    break;
                }

                const { requestId: readReqId, fromSequence: readFromSeq, limit, mapName: readMapName } = message;
                const startSeq = BigInt(readFromSeq);
                const eventLimit = limit ?? 100;

                let events = this.eventJournalService.readFrom(startSeq, eventLimit);

                // Filter by map name if provided
                if (readMapName) {
                    events = events.filter(e => e.mapName === readMapName);
                }

                // Serialize events
                const serializedEvents = events.map(e => ({
                    sequence: e.sequence.toString(),
                    type: e.type,
                    mapName: e.mapName,
                    key: e.key,
                    value: e.value,
                    previousValue: e.previousValue,
                    timestamp: e.timestamp,
                    nodeId: e.nodeId,
                    metadata: e.metadata,
                }));

                client.writer.write({
                    type: 'JOURNAL_READ_RESPONSE',
                    requestId: readReqId,
                    events: serializedEvents,
                    hasMore: events.length === eventLimit,
                });
                break;
            }

            // Phase 11.1: Full-Text Search (Phase 14: Distributed Search)
            case 'SEARCH': {
                const { requestId: searchReqId, mapName: searchMapName, query: searchQuery, options: searchOptions } = message.payload;

                // Check READ permission
                if (!this.securityManager.checkPermission(client.principal!, searchMapName, 'READ')) {
                    logger.warn({ clientId: client.id, mapName: searchMapName }, 'Access Denied: SEARCH');
                    client.writer.write({
                        type: 'SEARCH_RESP',
                        payload: {
                            requestId: searchReqId,
                            results: [],
                            totalCount: 0,
                            error: `Access denied for map: ${searchMapName}`,
                        }
                    });
                    break;
                }

                // Check if FTS is enabled for this map
                if (!this.searchCoordinator.isSearchEnabled(searchMapName)) {
                    client.writer.write({
                        type: 'SEARCH_RESP',
                        payload: {
                            requestId: searchReqId,
                            results: [],
                            totalCount: 0,
                            error: `Full-text search not enabled for map: ${searchMapName}`,
                        }
                    });
                    break;
                }

                // Phase 14: Use distributed search if ClusterSearchCoordinator is available
                // and we have more than one node in the cluster
                if (this.clusterSearchCoordinator && this.cluster.getMembers().length > 1) {
                    // Execute distributed search across cluster
                    this.clusterSearchCoordinator.search(searchMapName, searchQuery, {
                        limit: searchOptions?.limit ?? 10,
                        minScore: searchOptions?.minScore,
                        boost: searchOptions?.boost,
                    }).then(distributedResult => {
                        logger.debug({
                            clientId: client.id,
                            mapName: searchMapName,
                            query: searchQuery,
                            resultCount: distributedResult.results.length,
                            totalHits: distributedResult.totalHits,
                            respondedNodes: distributedResult.respondedNodes.length,
                            failedNodes: distributedResult.failedNodes.length,
                            executionTimeMs: distributedResult.executionTimeMs,
                        }, 'Distributed search executed');

                        client.writer.write({
                            type: 'SEARCH_RESP',
                            payload: {
                                requestId: searchReqId,
                                results: distributedResult.results,
                                totalCount: distributedResult.totalHits,
                                // Include cursor for pagination if available
                                nextCursor: distributedResult.nextCursor,
                            },
                        });
                    }).catch(err => {
                        logger.error({ err, mapName: searchMapName, query: searchQuery }, 'Distributed search failed');
                        client.writer.write({
                            type: 'SEARCH_RESP',
                            payload: {
                                requestId: searchReqId,
                                results: [],
                                totalCount: 0,
                                error: `Distributed search failed: ${err.message}`,
                            },
                        });
                    });
                } else {
                    // Execute local search (single node or no cluster)
                    const searchResult = this.searchCoordinator.search(searchMapName, searchQuery, searchOptions);
                    searchResult.requestId = searchReqId;

                    logger.debug({
                        clientId: client.id,
                        mapName: searchMapName,
                        query: searchQuery,
                        resultCount: searchResult.results.length
                    }, 'Local search executed');

                    client.writer.write({
                        type: 'SEARCH_RESP',
                        payload: searchResult,
                    });
                }
                break;
            }

            // Phase 11.1b: Live Search Subscriptions
            case 'SEARCH_SUB': {
                const { subscriptionId, mapName: subMapName, query: subQuery, options: subOptions } = message.payload;

                // Check READ permission
                if (!this.securityManager.checkPermission(client.principal!, subMapName, 'READ')) {
                    logger.warn({ clientId: client.id, mapName: subMapName }, 'Access Denied: SEARCH_SUB');
                    client.writer.write({
                        type: 'SEARCH_RESP',
                        payload: {
                            requestId: subscriptionId,
                            results: [],
                            totalCount: 0,
                            error: `Access denied for map: ${subMapName}`,
                        }
                    });
                    break;
                }

                // Check if FTS is enabled for this map
                if (!this.searchCoordinator.isSearchEnabled(subMapName)) {
                    client.writer.write({
                        type: 'SEARCH_RESP',
                        payload: {
                            requestId: subscriptionId,
                            results: [],
                            totalCount: 0,
                            error: `Full-text search not enabled for map: ${subMapName}`,
                        }
                    });
                    break;
                }

                // Phase 14.2: Use distributed subscription if cluster mode with multiple nodes
                if (this.distributedSubCoordinator && this.cluster && this.cluster.getMembers().length > 1) {
                    // Distributed search subscription
                    this.distributedSubCoordinator.subscribeSearch(
                        subscriptionId,
                        client.socket,
                        subMapName,
                        subQuery,
                        subOptions || {}
                    ).then((result) => {
                        client.writer.write({
                            type: 'SEARCH_RESP',
                            payload: {
                                requestId: subscriptionId,
                                results: result.results,
                                totalCount: result.totalHits,
                            },
                        });

                        logger.debug({
                            clientId: client.id,
                            subscriptionId,
                            mapName: subMapName,
                            query: subQuery,
                            resultCount: result.results.length,
                            totalHits: result.totalHits,
                            nodes: result.registeredNodes,
                        }, 'Distributed search subscription created');
                    }).catch((err) => {
                        logger.error({ err, subscriptionId }, 'Distributed search subscription failed');
                        client.writer.write({
                            type: 'SEARCH_RESP',
                            payload: {
                                requestId: subscriptionId,
                                results: [],
                                totalCount: 0,
                                error: 'Failed to create distributed subscription',
                            },
                        });
                    });
                } else {
                    // Single-node fallback: use local SearchCoordinator
                    const initialResults = this.searchCoordinator.subscribe(
                        client.id,
                        subscriptionId,
                        subMapName,
                        subQuery,
                        subOptions
                    );

                    logger.debug({
                        clientId: client.id,
                        subscriptionId,
                        mapName: subMapName,
                        query: subQuery,
                        resultCount: initialResults.length
                    }, 'Search subscription created (local)');

                    // Send initial snapshot as SEARCH_RESP
                    client.writer.write({
                        type: 'SEARCH_RESP',
                        payload: {
                            requestId: subscriptionId,
                            results: initialResults,
                            totalCount: initialResults.length,
                        },
                    });
                }
                break;
            }

            case 'SEARCH_UNSUB': {
                const { subscriptionId: unsubId } = message.payload;
                // Unsubscribe from both local and distributed
                this.searchCoordinator.unsubscribe(unsubId);
                if (this.distributedSubCoordinator) {
                    this.distributedSubCoordinator.unsubscribe(unsubId);
                }
                logger.debug({ clientId: client.id, subscriptionId: unsubId }, 'Search unsubscription');
                break;
            }

            default:
                logger.warn({ type: message.type }, 'Unknown message type');
        }
    }

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
        for (const client of this.clients.values()) {
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
        for (const [clientId, client] of this.clients) {
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
            const client = this.clients.get(clientId);
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
        const isServerEvent = message.type === 'SERVER_EVENT';

        if (isServerEvent) {
            const payload = message.payload;
            const mapName = payload.mapName;

            // === SUBSCRIPTION-BASED ROUTING ===
            // Only send to clients that have active subscriptions for this map
            const subscribedClientIds = this.queryRegistry.getSubscribedClientIds(mapName);

            // Track metrics
            this.metricsService.incEventsRouted();

            if (subscribedClientIds.size === 0) {
                // Early exit - no subscribers for this map!
                this.metricsService.incEventsFilteredBySubscription();
                return;
            }

            // Track average subscribers per event
            this.metricsService.recordSubscribersPerEvent(subscribedClientIds.size);

            // Send only to subscribed clients with FLS filtering
            for (const clientId of subscribedClientIds) {
                if (clientId === excludeClientId) continue;

                const client = this.clients.get(clientId);
                if (!client || client.socket.readyState !== 1 || !client.isAuthenticated || !client.principal) {
                    continue;
                }

                // Shallow clone payload for FLS filtering
                const newPayload = { ...payload };

                if (newPayload.record) { // LWW
                    const newVal = this.securityManager.filterObject(newPayload.record.value, client.principal, mapName);
                    newPayload.record = { ...newPayload.record, value: newVal };
                }

                if (newPayload.orRecord) { // OR_ADD
                    const newVal = this.securityManager.filterObject(newPayload.orRecord.value, client.principal, mapName);
                    newPayload.orRecord = { ...newPayload.orRecord, value: newVal };
                }

                client.writer.write({ ...message, payload: newPayload });
            }
        } else {
            // Non-event messages (GC_PRUNE, SHUTDOWN_PENDING) still go to all clients
            const msgData = serialize(message);
            for (const [id, client] of this.clients) {
                if (id !== excludeClientId && client.socket.readyState === 1) { // 1 = OPEN
                    client.writer.writeRaw(msgData);
                }
            }
        }
    }

    /**
     * === OPTIMIZATION 2 & 3: Batched Broadcast with Serialization Caching ===
     * Groups clients by their permission roles and serializes once per group.
     * Also batches multiple events into a single SERVER_BATCH_EVENT message.
     * === OPTIMIZATION 4: Subscription-based Routing ===
     * Only sends events to clients with active subscriptions for affected maps.
     */
    private broadcastBatch(events: any[], excludeClientId?: string): void {
        if (events.length === 0) return;

        // === SUBSCRIPTION-BASED ROUTING ===
        // Get unique map names from events
        const affectedMaps = new Set<string>();
        for (const event of events) {
            if (event.mapName) {
                affectedMaps.add(event.mapName);
            }
        }

        // Get all subscribed client IDs across all affected maps
        const subscribedClientIds = new Set<string>();
        for (const mapName of affectedMaps) {
            const mapSubscribers = this.queryRegistry.getSubscribedClientIds(mapName);
            for (const clientId of mapSubscribers) {
                subscribedClientIds.add(clientId);
            }
        }

        // Track metrics
        this.metricsService.incEventsRouted();

        if (subscribedClientIds.size === 0) {
            // Early exit - no subscribers for any of the affected maps!
            this.metricsService.incEventsFilteredBySubscription();
            return;
        }

        this.metricsService.recordSubscribersPerEvent(subscribedClientIds.size);

        // Group subscribed clients by their role signature for serialization caching
        const clientsByRoleSignature = new Map<string, ClientConnection[]>();

        for (const clientId of subscribedClientIds) {
            if (clientId === excludeClientId) continue;

            const client = this.clients.get(clientId);
            if (!client || client.socket.readyState !== 1 || !client.isAuthenticated || !client.principal) {
                continue;
            }

            // Create a role signature for grouping (sorted roles joined)
            const roleSignature = (client.principal.roles || ['USER']).sort().join(',');

            if (!clientsByRoleSignature.has(roleSignature)) {
                clientsByRoleSignature.set(roleSignature, []);
            }
            clientsByRoleSignature.get(roleSignature)!.push(client);
        }

        // For each role group, filter events once and serialize once
        for (const [, clients] of clientsByRoleSignature) {
            if (clients.length === 0) continue;

            // Use first client as representative for filtering (same roles = same permissions)
            const representativeClient = clients[0];

            // Filter all events for this role group
            const filteredEvents = events.map(eventPayload => {
                const mapName = eventPayload.mapName;
                const newPayload = { ...eventPayload };

                if (newPayload.record) { // LWW
                    const newVal = this.securityManager.filterObject(
                        newPayload.record.value,
                        representativeClient.principal!,
                        mapName
                    );
                    newPayload.record = { ...newPayload.record, value: newVal };
                }

                if (newPayload.orRecord) { // OR_ADD
                    const newVal = this.securityManager.filterObject(
                        newPayload.orRecord.value,
                        representativeClient.principal!,
                        mapName
                    );
                    newPayload.orRecord = { ...newPayload.orRecord, value: newVal };
                }

                return newPayload;
            });

            // Serialize ONCE for this entire group
            const batchMessage = {
                type: 'SERVER_BATCH_EVENT',
                payload: { events: filteredEvents },
                timestamp: this.hlc.now()
            };
            const serializedBatch = serialize(batchMessage);

            // Send to all clients in this role group
            for (const client of clients) {
                try {
                    client.writer.writeRaw(serializedBatch);
                } catch (err) {
                    logger.error({ clientId: client.id, err }, 'Failed to send batch to client');
                }
            }
        }
    }

    /**
     * Helper method to get role signature for a client (for caching key)
     */
    private getClientRoleSignature(client: ClientConnection): string {
        if (!client.principal || !client.principal.roles) {
            return 'USER';
        }
        return client.principal.roles.sort().join(',');
    }

    /**
     * === BACKPRESSURE: Synchronous Broadcast ===
     * Same as broadcastBatch but waits for all sends to complete.
     * Used when backpressure forces sync processing to drain the pipeline.
     */
    private async broadcastBatchSync(events: any[], excludeClientId?: string): Promise<void> {
        if (events.length === 0) return;

        // Get unique map names from events
        const affectedMaps = new Set<string>();
        for (const event of events) {
            if (event.mapName) {
                affectedMaps.add(event.mapName);
            }
        }

        // Get all subscribed client IDs across all affected maps
        const subscribedClientIds = new Set<string>();
        for (const mapName of affectedMaps) {
            const mapSubscribers = this.queryRegistry.getSubscribedClientIds(mapName);
            for (const clientId of mapSubscribers) {
                subscribedClientIds.add(clientId);
            }
        }

        if (subscribedClientIds.size === 0) {
            return;
        }

        // Group subscribed clients by their role signature
        const clientsByRoleSignature = new Map<string, ClientConnection[]>();

        for (const clientId of subscribedClientIds) {
            if (clientId === excludeClientId) continue;

            const client = this.clients.get(clientId);
            if (!client || client.socket.readyState !== 1 || !client.isAuthenticated || !client.principal) {
                continue;
            }

            const roleSignature = (client.principal.roles || ['USER']).sort().join(',');

            if (!clientsByRoleSignature.has(roleSignature)) {
                clientsByRoleSignature.set(roleSignature, []);
            }
            clientsByRoleSignature.get(roleSignature)!.push(client);
        }

        // Collect all send promises
        const sendPromises: Promise<void>[] = [];

        for (const [, clients] of clientsByRoleSignature) {
            if (clients.length === 0) continue;

            const representativeClient = clients[0];

            // Filter all events for this role group
            const filteredEvents = events.map(eventPayload => {
                const mapName = eventPayload.mapName;
                const newPayload = { ...eventPayload };

                if (newPayload.record) {
                    const newVal = this.securityManager.filterObject(
                        newPayload.record.value,
                        representativeClient.principal!,
                        mapName
                    );
                    newPayload.record = { ...newPayload.record, value: newVal };
                }

                if (newPayload.orRecord) {
                    const newVal = this.securityManager.filterObject(
                        newPayload.orRecord.value,
                        representativeClient.principal!,
                        mapName
                    );
                    newPayload.orRecord = { ...newPayload.orRecord, value: newVal };
                }

                return newPayload;
            });

            const batchMessage = {
                type: 'SERVER_BATCH_EVENT',
                payload: { events: filteredEvents },
                timestamp: this.hlc.now()
            };
            const serializedBatch = serialize(batchMessage);

            // Send to all clients and collect promises
            for (const client of clients) {
                sendPromises.push(new Promise<void>((resolve, reject) => {
                    try {
                        client.socket.send(serializedBatch, (err) => {
                            if (err) {
                                logger.error({ clientId: client.id, err }, 'Failed to send sync batch to client');
                                reject(err);
                            } else {
                                resolve();
                            }
                        });
                    } catch (err) {
                        logger.error({ clientId: client.id, err }, 'Exception sending sync batch to client');
                        reject(err);
                    }
                }));
            }
        }

        // Wait for all sends to complete (ignore individual failures)
        await Promise.allSettled(sendPromises);
    }

    private setupClusterListeners() {
        this.cluster.on('memberJoined', () => {
            this.metricsService.setClusterMembers(this.cluster.getMembers().length);
        });
        this.cluster.on('memberLeft', () => {
            this.metricsService.setClusterMembers(this.cluster.getMembers().length);
        });

        this.cluster.on('message', (msg) => {
            switch (msg.type) {
                case 'OP_FORWARD':
                    // OP_FORWARD is used for multiple purposes:
                    // 1. Actual operation forwards (has key field) - route to partition owner
                    // 2. Replication messages (has _replication field) - handled by ReplicationPipeline
                    // 3. Migration messages (has _migration field) - handled by MigrationManager
                    // Only validate key for actual operation forwards
                    if (msg.payload._replication || msg.payload._migration) {
                        // These are handled by ReplicationPipeline and MigrationManager listeners
                        // No routing check needed
                        break;
                    }

                    // Actual operation forward - validate key and route
                    logger.info({ senderId: msg.senderId }, 'Received forwarded op');
                    if (!msg.payload.key) {
                        logger.warn({ senderId: msg.senderId }, 'OP_FORWARD missing key, dropping');
                        break;
                    }
                    if (this.partitionService.isLocalOwner(msg.payload.key)) {
                        this.processLocalOp(msg.payload, true, msg.senderId).catch(err => {
                            logger.error({ err, senderId: msg.senderId }, 'Forwarded op failed');
                        });
                    } else {
                        logger.warn({ key: msg.payload.key }, 'Received OP_FORWARD but not owner. Dropping.');
                    }
                    break;
                case 'CLUSTER_EVENT':
                    this.handleClusterEvent(msg.payload);
                    break;

                case 'CLUSTER_QUERY_EXEC': {
                    const { requestId, mapName, query } = msg.payload;
                    this.executeLocalQuery(mapName, query).then(results => {
                        this.cluster.send(msg.senderId, 'CLUSTER_QUERY_RESP', {
                            requestId,
                            results
                        });
                    }).catch(err => {
                        logger.error({ err, mapName }, 'Failed to execute cluster query');
                        this.cluster.send(msg.senderId, 'CLUSTER_QUERY_RESP', {
                            requestId,
                            results: []
                        });
                    });
                    break;
                }

                case 'CLUSTER_QUERY_RESP': {
                    const { requestId: reqId, results: remoteResults } = msg.payload;
                    const pendingQuery = this.pendingClusterQueries.get(reqId);
                    if (pendingQuery) {
                        pendingQuery.results.push(...remoteResults);
                        pendingQuery.respondedNodes.add(msg.senderId);

                        if (pendingQuery.respondedNodes.size === pendingQuery.expectedNodes.size) {
                            this.finalizeClusterQuery(reqId);
                        }
                    }
                    break;
                }

                case 'CLUSTER_GC_REPORT': {
                    this.handleGcReport(msg.senderId, msg.payload.minHlc);
                    break;
                }

                case 'CLUSTER_GC_COMMIT': {
                    this.performGarbageCollection(msg.payload.safeTimestamp);
                    break;
                }

                case 'CLUSTER_LOCK_REQ': {
                    const { originNodeId, clientId, requestId, name, ttl } = msg.payload;
                    const compositeId = `${originNodeId}:${clientId}`;
                    const result = this.lockManager.acquire(name, compositeId, requestId, ttl || 10000);
                    if (result.granted) {
                        this.cluster.send(originNodeId, 'CLUSTER_LOCK_GRANTED', {
                            clientId,
                            requestId,
                            name,
                            fencingToken: result.fencingToken
                        });
                    }
                    break;
                }

                case 'CLUSTER_LOCK_RELEASE': {
                    const { originNodeId, clientId, requestId, name, fencingToken } = msg.payload;
                    const compositeId = `${originNodeId}:${clientId}`;
                    const success = this.lockManager.release(name, compositeId, fencingToken);
                    this.cluster.send(originNodeId, 'CLUSTER_LOCK_RELEASED', {
                        clientId, requestId, name, success
                    });
                    break;
                }

                case 'CLUSTER_LOCK_RELEASED': {
                    const { clientId, requestId, name, success } = msg.payload;
                    const client = this.clients.get(clientId);
                    if (client) {
                        client.writer.write({
                            type: 'LOCK_RELEASED',
                            payload: { requestId, name, success }
                        });
                    }
                    break;
                }

                case 'CLUSTER_LOCK_GRANTED': {
                    const { clientId, requestId, name, fencingToken } = msg.payload;
                    const client = this.clients.get(clientId);
                    if (client) {
                        client.writer.write({
                            type: 'LOCK_GRANTED',
                            payload: { requestId, name, fencingToken }
                        });
                    }
                    break;
                }

                case 'CLUSTER_CLIENT_DISCONNECTED': {
                    const { clientId, originNodeId } = msg.payload;
                    const compositeId = `${originNodeId}:${clientId}`;
                    this.lockManager.handleClientDisconnect(compositeId);
                    break;
                }

                case 'CLUSTER_TOPIC_PUB': {
                    const { topic, data, originalSenderId } = msg.payload;
                    this.topicManager.publish(topic, data, originalSenderId, true);
                    break;
                }

                // Phase 10.04: Anti-entropy repair messages
                case 'CLUSTER_MERKLE_ROOT_REQ': {
                    const { partitionId, requestId } = msg.payload;
                    const rootHash = this.merkleTreeManager?.getRootHash(partitionId) ?? 0;
                    this.cluster.send(msg.senderId, 'CLUSTER_MERKLE_ROOT_RESP', {
                        requestId,
                        partitionId,
                        rootHash
                    });
                    break;
                }

                case 'CLUSTER_MERKLE_ROOT_RESP': {
                    // Response handled by RepairScheduler via event or callback
                    // For now, emit as an event that RepairScheduler can listen to
                    if (this.repairScheduler) {
                        this.repairScheduler.emit('merkleRootResponse', {
                            nodeId: msg.senderId,
                            ...msg.payload
                        });
                    }
                    break;
                }

                case 'CLUSTER_REPAIR_DATA_REQ': {
                    // Request for data records from a specific partition
                    const { partitionId, keys, requestId } = msg.payload;
                    const records: Record<string, any> = {};
                    for (const key of keys) {
                        const record = this.getLocalRecord(key);
                        if (record) {
                            records[key] = record;
                        }
                    }
                    this.cluster.send(msg.senderId, 'CLUSTER_REPAIR_DATA_RESP', {
                        requestId,
                        partitionId,
                        records
                    });
                    break;
                }

                case 'CLUSTER_REPAIR_DATA_RESP': {
                    // Response with data records for repair
                    if (this.repairScheduler) {
                        this.repairScheduler.emit('repairDataResponse', {
                            nodeId: msg.senderId,
                            ...msg.payload
                        });
                    }
                    break;
                }
            }
        });
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
            const client = this.clients.get(clientId);
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
        const client = this.clients.get(clientId);
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

    private handleClusterEvent(payload: any) {
        // 1. Replication Logic: Am I a Backup?
        const { mapName, key, eventType } = payload;

        // Guard against undefined key (can happen with malformed cluster messages)
        if (!key) {
            logger.warn({ mapName, eventType }, 'Received cluster event with undefined key, ignoring');
            return;
        }

        const map = this.getMap(mapName, (eventType === 'OR_ADD' || eventType === 'OR_REMOVE') ? 'OR' : 'LWW');
        const oldRecord = (map instanceof LWWMap) ? map.getRecord(key) : null;

        // Only store if we are Owner (shouldn't receive event unless forwarded) or Backup
        if (this.partitionService.isRelated(key)) {
            if (map instanceof LWWMap && payload.record) {
                map.merge(key, payload.record);
            } else if (map instanceof ORMap) {
                if (eventType === 'OR_ADD' && payload.orRecord) {
                    map.apply(key, payload.orRecord);
                } else if (eventType === 'OR_REMOVE' && payload.orTag) {
                    map.applyTombstone(payload.orTag);
                }
            }
        }

        // 2. Notify Query Subscriptions
        this.queryRegistry.processChange(mapName, map, key, payload.record || payload.orRecord, oldRecord);

        // 3. Broadcast to local clients (Notification)
        this.broadcast({
            type: 'SERVER_EVENT',
            payload: payload,
            timestamp: this.hlc.now()
        });
    }

    public getMap(name: string, typeHint: 'LWW' | 'OR' = 'LWW'): LWWMap<string, any> | ORMap<string, any> {
        if (!this.maps.has(name)) {
            let map: LWWMap<string, any> | ORMap<string, any>;

            if (typeHint === 'OR') {
                map = new ORMap(this.hlc);
            } else {
                map = new LWWMap(this.hlc);
            }

            this.maps.set(name, map);

            // Lazy load from storage - track the promise for getMapAsync
            if (this.storage) {
                logger.info({ mapName: name }, 'Loading map from storage...');
                const loadPromise = this.loadMapFromStorage(name, typeHint);
                this.mapLoadingPromises.set(name, loadPromise);
                loadPromise.finally(() => {
                    this.mapLoadingPromises.delete(name);
                });
            }
        }
        return this.maps.get(name)!;
    }

    /**
     * Returns map after ensuring it's fully loaded from storage.
     * Use this for queries to avoid returning empty results during initial load.
     */
    public async getMapAsync(name: string, typeHint: 'LWW' | 'OR' = 'LWW'): Promise<LWWMap<string, any> | ORMap<string, any>> {
        const mapExisted = this.maps.has(name);

        // First ensure map exists (this triggers loading if needed)
        this.getMap(name, typeHint);

        // Wait for loading to complete if in progress
        const loadingPromise = this.mapLoadingPromises.get(name);

        // [DEBUG] Log state for troubleshooting sync issues
        const map = this.maps.get(name);
        const mapSize = map instanceof LWWMap ? Array.from(map.entries()).length :
                       map instanceof ORMap ? map.size : 0;
        logger.info({
            mapName: name,
            mapExisted,
            hasLoadingPromise: !!loadingPromise,
            currentMapSize: mapSize
        }, '[getMapAsync] State check');

        if (loadingPromise) {
            logger.info({ mapName: name }, '[getMapAsync] Waiting for loadMapFromStorage...');
            await loadingPromise;
            const newMapSize = map instanceof LWWMap ? Array.from(map.entries()).length :
                              map instanceof ORMap ? map.size : 0;
            logger.info({ mapName: name, mapSizeAfterLoad: newMapSize }, '[getMapAsync] Load completed');
        }

        return this.maps.get(name)!;
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

        const map = this.maps.get(mapName);
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

    private async loadMapFromStorage(name: string, typeHint: 'LWW' | 'OR'): Promise<void> {
        try {
            const keys = await this.storage!.loadAllKeys(name);
            if (keys.length === 0) return;

            // Check for ORMap markers in keys
            const hasTombstones = keys.includes('__tombstones__');

            const relatedKeys = keys.filter(k => this.partitionService.isRelated(k));
            if (relatedKeys.length === 0) return;

            const records = await this.storage!.loadAll(name, relatedKeys);
            let count = 0;

            // Check for Type Mismatch and Replace Map if needed
            let isOR = hasTombstones;
            if (!isOR) {
                // Check first record
                for (const [k, v] of records) {
                    if (k !== '__tombstones__' && (v as any).type === 'OR') {
                        isOR = true;
                        break;
                    }
                }
            }

            // If we created LWW but it's OR, replace it.
            // If we created OR but it's LWW, replace it? (Less likely if hint was OR, but possible if hint was wrong?)
            const currentMap = this.maps.get(name);
            if (!currentMap) return;
            let targetMap = currentMap;

            if (isOR && currentMap instanceof LWWMap) {
                logger.info({ mapName: name }, 'Map auto-detected as ORMap. Switching type.');
                targetMap = new ORMap(this.hlc);
                this.maps.set(name, targetMap);
            } else if (!isOR && currentMap instanceof ORMap && typeHint !== 'OR') {
                // Only switch back to LWW if hint wasn't explicit OR
                logger.info({ mapName: name }, 'Map auto-detected as LWWMap. Switching type.');
                targetMap = new LWWMap(this.hlc);
                this.maps.set(name, targetMap);
            }

            if (targetMap instanceof ORMap) {
                for (const [key, record] of records) {
                    if (key === '__tombstones__') {
                        const t = record as ORMapTombstones;
                        if (t && t.tags) t.tags.forEach(tag => targetMap.applyTombstone(tag));
                    } else {
                        const orVal = record as ORMapValue<any>;
                        if (orVal && orVal.records) {
                            orVal.records.forEach(r => targetMap.apply(key, r));
                            count++;
                        }
                    }
                }
            } else if (targetMap instanceof LWWMap) {
                for (const [key, record] of records) {
                    // Expect LWWRecord
                    // If record is actually ORMapValue (mismatch), we skip or error?
                    // If !isOR, we assume LWWRecord.
                    if (!(record as any).type) { // LWWRecord doesn't have type property in my impl
                        targetMap.merge(key, record as LWWRecord<any>);
                        count++;
                    }
                }
            }

            if (count > 0) {
                logger.info({ mapName: name, count }, 'Loaded records for map');
                this.queryRegistry.refreshSubscriptions(name, targetMap);
                const mapSize = (targetMap instanceof ORMap) ? targetMap.totalRecords : targetMap.size;
                this.metricsService.setMapSize(name, mapSize);
            }
        } catch (err) {
            logger.error({ mapName: name, err }, 'Failed to load map');
        }
    }

    private startGarbageCollection() {
        this.gcInterval = setInterval(() => {
            this.reportLocalHlc();
        }, GC_INTERVAL_MS);
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
     */
    public isClientAlive(clientId: string): boolean {
        const client = this.clients.get(clientId);
        if (!client) return false;

        const idleTime = Date.now() - client.lastPingReceived;
        return idleTime < CLIENT_HEARTBEAT_TIMEOUT_MS;
    }

    /**
     * Returns how long the client has been idle (no PING received).
     */
    public getClientIdleTime(clientId: string): number {
        const client = this.clients.get(clientId);
        if (!client) return Infinity;

        return Date.now() - client.lastPingReceived;
    }

    /**
     * Evicts clients that haven't sent a PING within the timeout period.
     */
    private evictDeadClients(): void {
        const now = Date.now();
        const deadClients: string[] = [];

        for (const [clientId, client] of this.clients) {
            // Only check authenticated clients (unauthenticated ones will timeout via auth mechanism)
            if (client.isAuthenticated) {
                const idleTime = now - client.lastPingReceived;
                if (idleTime > CLIENT_HEARTBEAT_TIMEOUT_MS) {
                    deadClients.push(clientId);
                }
            }
        }

        for (const clientId of deadClients) {
            const client = this.clients.get(clientId);
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

    private reportLocalHlc() {
        // 1. Calculate Local Min HLC
        let minHlc = this.hlc.now();

        for (const client of this.clients.values()) {
            if (HLC.compare(client.lastActiveHlc, minHlc) < 0) {
                minHlc = client.lastActiveHlc;
            }
        }

        const members = this.cluster.getMembers().sort();
        const leaderId = members[0];
        const myId = this.cluster.config.nodeId;

        if (leaderId === myId) {
            // I am Leader
            this.handleGcReport(myId, minHlc);
        } else {
            // Send to Leader
            this.cluster.send(leaderId, 'CLUSTER_GC_REPORT', { minHlc });
        }
    }

    private handleGcReport(nodeId: string, minHlc: Timestamp) {
        this.gcReports.set(nodeId, minHlc);

        const members = this.cluster.getMembers();

        // Check if we have reports from ALL members
        // (Including self, which is inserted directly)
        const allReported = members.every(m => this.gcReports.has(m));

        if (allReported) {
            // Calculate Global Safe Timestamp
            let globalSafe = this.hlc.now(); // Start high
            let initialized = false;

            for (const ts of this.gcReports.values()) {
                if (!initialized || HLC.compare(ts, globalSafe) < 0) {
                    globalSafe = ts;
                    initialized = true;
                }
            }

            // Add safety buffer (e.g. GC_AGE)
            // prune(timestamp) removes items OLDER than timestamp.
            // We want to remove items OLDER than (GlobalMin - GC_AGE).

            const olderThanMillis = globalSafe.millis - GC_AGE_MS;
            const safeTimestamp: Timestamp = {
                millis: olderThanMillis,
                counter: 0,
                nodeId: globalSafe.nodeId // Doesn't matter much for comparison if millis match, but best effort
            };

            logger.info({
                globalMinHlc: globalSafe.millis,
                safeGcTimestamp: olderThanMillis,
                reportsCount: this.gcReports.size
            }, 'GC Consensus Reached. Broadcasting Commit.');

            // Broadcast Commit
            const commitMsg = {
                type: 'CLUSTER_GC_COMMIT', // Handled by cluster listener
                payload: { safeTimestamp }
            };

            // Send to others
            for (const member of members) {
                if (!this.cluster.isLocal(member)) {
                    this.cluster.send(member, 'CLUSTER_GC_COMMIT', { safeTimestamp });
                }
            }

            // Execute Locally
            this.performGarbageCollection(safeTimestamp);

            // Clear reports for next round?
            // Or keep them and overwrite?
            // Overwriting is better for partial updates, but clearing ensures freshness.
            // Since we run interval based, clearing is safer to ensure active participation next time.
            this.gcReports.clear();
        }
    }

    private performGarbageCollection(olderThan: Timestamp) {
        logger.info({ olderThanMillis: olderThan.millis }, 'Performing Garbage Collection');
        const now = Date.now();

        for (const [name, map] of this.maps) {
            // 1. Check for active expired records (TTL)
            if (map instanceof LWWMap) {
                for (const key of map.allKeys()) {
                    const record = map.getRecord(key);
                    if (record && record.value !== null && record.ttlMs) {
                        const expirationTime = record.timestamp.millis + record.ttlMs;
                        if (expirationTime < now) {
                            logger.info({ mapName: name, key }, 'Record expired (TTL). Converting to tombstone.');

                            // Create Tombstone at expiration time to handle "Resurrection" correctly
                            const tombstoneTimestamp: Timestamp = {
                                millis: expirationTime,
                                counter: 0, // Reset counter for expiration time
                                nodeId: this.hlc.getNodeId // Use our ID
                            };

                            const tombstone: LWWRecord<any> = { value: null, timestamp: tombstoneTimestamp };

                            // Apply locally
                            const changed = map.merge(key, tombstone);

                            if (changed) {
                                // Persist and Broadcast
                                if (this.storage) {
                                    this.storage.store(name, key, tombstone).catch(err =>
                                        logger.error({ mapName: name, key, err }, 'Failed to persist expired tombstone')
                                    );
                                }

                                const eventPayload = {
                                    mapName: name,
                                    key: key,
                                    eventType: 'UPDATED',
                                    record: tombstone
                                };

                                // Broadcast to local clients
                                this.broadcast({
                                    type: 'SERVER_EVENT',
                                    payload: eventPayload,
                                    timestamp: this.hlc.now()
                                });

                                // Notify query subscriptions (handles both local and distributed via CLUSTER_SUB_UPDATE)
                                this.queryRegistry.processChange(name, map, key, tombstone, record);

                                // Replicate to backup nodes via partition-aware ReplicationPipeline
                                // This replaces the O(N) CLUSTER_EVENT broadcast
                                if (this.replicationPipeline) {
                                    const op = {
                                        opType: 'set',
                                        mapName: name,
                                        key: key,
                                        record: tombstone
                                    };
                                    const opId = `ttl:${name}:${key}:${Date.now()}`;
                                    this.replicationPipeline.replicate(op, opId, key).catch(err => {
                                        logger.warn({ opId, key, err }, 'TTL expiration replication failed (non-fatal)');
                                    });
                                }
                            }
                        }
                    }
                }

                // 2. Prune old tombstones
                const removedKeys = map.prune(olderThan);
                if (removedKeys.length > 0) {
                    logger.info({ mapName: name, count: removedKeys.length }, 'Pruned records from LWW map');
                    if (this.storage) {
                        this.storage.deleteAll(name, removedKeys).catch(err => {
                            logger.error({ mapName: name, err }, 'Failed to delete pruned keys from storage');
                        });
                    }
                }
            } else if (map instanceof ORMap) {
                // ORMap Expiration
                // We need to check all active records in the ORMap
                const items = (map as any).items as Map<string, Map<string, ORMapRecord<any>>>;
                const tombstonesSet = (map as any).tombstones as Set<string>;

                const tagsToExpire: { key: string; tag: string }[] = [];

                for (const [key, keyMap] of items) {
                    for (const [tag, record] of keyMap) {
                        if (!tombstonesSet.has(tag)) {
                            if (record.ttlMs) {
                                const expirationTime = record.timestamp.millis + record.ttlMs;
                                if (expirationTime < now) {
                                    tagsToExpire.push({ key, tag });
                                }
                            }
                        }
                    }
                }

                for (const { key, tag } of tagsToExpire) {
                    logger.info({ mapName: name, key, tag }, 'ORMap Record expired (TTL). Removing.');

                    // Get old records for processChange before modification
                    const oldRecords = map.getRecords(key);

                    // Remove by adding tag to tombstones
                    map.applyTombstone(tag);

                    // Persist change
                    if (this.storage) {
                        // We need to update the key's record list and tombstones
                        // Optimally, we should batch these updates
                        const records = map.getRecords(key);
                        if (records.length > 0) {
                            this.storage.store(name, key, { type: 'OR', records });
                        } else {
                            this.storage.delete(name, key);
                        }

                        const currentTombstones = map.getTombstones();
                        this.storage.store(name, '__tombstones__', {
                            type: 'OR_TOMBSTONES',
                            tags: currentTombstones
                        });
                    }

                    // Broadcast
                    const eventPayload = {
                        mapName: name,
                        key: key,
                        eventType: 'OR_REMOVE',
                        orTag: tag
                    };

                    // Broadcast to local clients
                    this.broadcast({
                        type: 'SERVER_EVENT',
                        payload: eventPayload,
                        timestamp: this.hlc.now()
                    });

                    // Notify query subscriptions (handles both local and distributed via CLUSTER_SUB_UPDATE)
                    const newRecords = map.getRecords(key);
                    this.queryRegistry.processChange(name, map, key, newRecords, oldRecords);

                    // Replicate to backup nodes via partition-aware ReplicationPipeline
                    // This replaces the O(N) CLUSTER_EVENT broadcast
                    if (this.replicationPipeline) {
                        const op = {
                            opType: 'OR_REMOVE',
                            mapName: name,
                            key: key,
                            orTag: tag
                        };
                        const opId = `ttl:${name}:${key}:${tag}:${Date.now()}`;
                        this.replicationPipeline.replicate(op, opId, key).catch(err => {
                            logger.warn({ opId, key, err }, 'ORMap TTL expiration replication failed (non-fatal)');
                        });
                    }
                }

                // 2. Prune old tombstones
                const removedTags = map.prune(olderThan);
                if (removedTags.length > 0) {
                    logger.info({ mapName: name, count: removedTags.length }, 'Pruned tombstones from OR map');
                    // We need to update __tombstones__ in storage
                    if (this.storage) {
                        const currentTombstones = map.getTombstones();
                        this.storage.store(name, '__tombstones__', {
                            type: 'OR_TOMBSTONES',
                            tags: currentTombstones
                        }).catch(err => {
                            logger.error({ mapName: name, err }, 'Failed to update tombstones');
                        });
                    }
                }
            }
        }

        // Broadcast to clients
        this.broadcast({
            type: 'GC_PRUNE',
            payload: {
                olderThan
            }
        });
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
