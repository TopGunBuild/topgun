import { createServer as createHttpServer, Server as HttpServer } from 'http';
import { createServer as createHttpsServer, Server as HttpsServer, ServerOptions as HttpsServerOptions } from 'https';
import { readFileSync } from 'fs';
import * as net from 'net';
import { WebSocketServer, WebSocket } from 'ws';
import { HLC, LWWMap, ORMap, MerkleTree, serialize, deserialize, PermissionPolicy, Principal, PermissionType, Timestamp, LWWRecord, ORMapRecord, MessageSchema } from '@topgunbuild/core';
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

    private _actualPort: number = 0;
    private _actualClusterPort: number = 0;
    private _readyPromise: Promise<void>;
    private _readyResolve!: () => void;

    constructor(config: ServerCoordinatorConfig) {
        this._readyPromise = new Promise((resolve) => {
            this._readyResolve = resolve;
        });

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

        const metricsPort = config.metricsPort !== undefined ? config.metricsPort : 9090;
        this.metricsServer = createHttpServer(async (req, res) => {
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
            this.storage.initialize().then(() => {
                logger.info('Storage adapter initialized');
            }).catch(err => {
                logger.error({ err }, 'Failed to initialize storage');
            });
        }

        this.startGarbageCollection();
        this.startHeartbeatCheck();
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

    public async shutdown() {
        logger.info('Shutting down Server Coordinator...');

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

        // 4. Stop Cluster
        if (this.cluster) {
            this.cluster.stop();
        }

        // 5. Close Storage
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

                // Identify all relevant nodes
                const allMembers = this.cluster.getMembers();
                const remoteMembers = allMembers.filter(id => !this.cluster.isLocal(id));

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
                break;
            }

            case 'QUERY_UNSUB': {
                const { queryId: unsubId } = message.payload;
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
                logger.info({ clientId: client.id, count: ops.length }, 'Received batch');

                // === OPTIMIZATION 1: Early ACK ===
                // Fast validation pass - check permissions without processing
                const validOps: typeof ops = [];
                let rejectedCount = 0;
                let lastValidId: string | null = null;

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
                }

                // Send ACK IMMEDIATELY after validation (before processing)
                if (lastValidId !== null) {
                    client.writer.write({
                        type: 'OP_ACK',
                        payload: { lastId: lastValidId }
                    });
                }

                // Send rejection error if any ops were denied
                if (rejectedCount > 0) {
                    client.writer.write({
                        type: 'ERROR',
                        payload: { code: 403, message: `Partial batch failure: ${rejectedCount} ops denied` }
                    }, true);
                }

                // Process valid ops asynchronously (non-blocking)
                if (validOps.length > 0) {
                    const batchPromise = new Promise<void>((resolve) => {
                        setImmediate(() => {
                            this.processBatchAsync(validOps, client.id)
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
                    logger.info({ senderId: msg.senderId }, 'Received forwarded op');
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
            }
        });
    }

    private async executeLocalQuery(mapName: string, query: Query) {
        // Wait for map to be fully loaded from storage before querying
        const map = await this.getMapAsync(mapName);
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
            // Accessing properties on array might fail depending on query.
            // Assuming user knows what they are querying.
            // Since ORMap doesn't have allKeys, we iterate internal structure?
            // ORMap doesn't expose keys iterator publicly in the class I read?
            // Wait, checking ORMap.ts...
            // It doesn't export keys()! It exports items: Map.
            // But items is private.
            // I need to add keys() to ORMap or use 'any' cast.
            // I will cast to any for now.
            const items = (map as any).items as Map<string, any>;
            for (const key of items.keys()) {
                const values = map.get(key);
                if (values.length > 0) {
                    // We wrap in object matching LWWRecord structure roughly?
                    // { value: values, timestamp: ... }
                    // But timestamp differs per record.
                    records.set(key, { value: values });
                }
            }
        }

        // Fix: Do not apply offset/limit locally for cluster queries.
        // They will be applied in finalizeClusterQuery after aggregation.
        const localQuery = { ...query };
        delete localQuery.offset;
        delete localQuery.limit;

        return executeQuery(records, localQuery);
    }

    private finalizeClusterQuery(requestId: string, timeout = false) {
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

        const slicedResults = (query.offset || query.limit)
            ? finalResults.slice(query.offset || 0, (query.offset || 0) + (query.limit || finalResults.length))
            : finalResults;

        // Register Subscription
        const resultKeys = new Set(slicedResults.map(r => r.key));
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
        const filteredResults = slicedResults.map(res => {
            const filteredValue = this.securityManager.filterObject(res.value, client.principal!, mapName);
            return { ...res, value: filteredValue };
        });

        client.writer.write({
            type: 'QUERY_RESP',
            payload: { queryId, results: filteredResults }
        });
    }

    /**
     * Core operation application logic shared between processLocalOp and processLocalOpForBatch.
     * Handles map merge, storage persistence, query evaluation, and event generation.
     *
     * @returns Event payload for broadcasting (or null if operation failed)
     */
    private applyOpToMap(op: any): { eventPayload: any; oldRecord: any } {
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
            map.merge(op.key, op.record);
            recordToStore = op.record;
            eventPayload.eventType = 'UPDATED';
            eventPayload.record = op.record;
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

        return { eventPayload, oldRecord };
    }

    /**
     * Broadcast event to cluster members (excluding self).
     */
    private broadcastToCluster(eventPayload: any): void {
        const members = this.cluster.getMembers();
        for (const memberId of members) {
            if (!this.cluster.isLocal(memberId)) {
                this.cluster.send(memberId, 'CLUSTER_EVENT', eventPayload);
            }
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
        const { eventPayload } = this.applyOpToMap(op);

        // 4. Broadcast EVENT to other clients
        this.broadcast({
            type: 'SERVER_EVENT',
            payload: eventPayload,
            timestamp: this.hlc.now()
        }, originalSenderId);

        // 5. Broadcast to cluster
        this.broadcastToCluster(eventPayload);

        // 6. Run onAfterOp interceptors
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
        const { eventPayload } = this.applyOpToMap(op);

        // 4. Collect event for batched broadcast (instead of immediate broadcast)
        batchedEvents.push(eventPayload);

        // 5. Broadcast to cluster
        this.broadcastToCluster(eventPayload);

        // 6. Run onAfterOp interceptors
        this.runAfterInterceptors(op, context);
    }

    private handleClusterEvent(payload: any) {
        // 1. Replication Logic: Am I a Backup?
        const { mapName, key, eventType } = payload;
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
                                // Construct an artificial op to reuse pipeline logic or do manual steps
                                // Manual steps are safer here as we don't have a client op context

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

                                this.broadcast({
                                    type: 'SERVER_EVENT',
                                    payload: eventPayload,
                                    timestamp: this.hlc.now()
                                });

                                const members = this.cluster.getMembers();
                                for (const memberId of members) {
                                    if (!this.cluster.isLocal(memberId)) {
                                        this.cluster.send(memberId, 'CLUSTER_EVENT', eventPayload);
                                    }
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

                    this.broadcast({
                        type: 'SERVER_EVENT',
                        payload: eventPayload,
                        timestamp: this.hlc.now()
                    });

                    const members = this.cluster.getMembers();
                    for (const memberId of members) {
                        if (!this.cluster.isLocal(memberId)) {
                            this.cluster.send(memberId, 'CLUSTER_EVENT', eventPayload);
                        }
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
}
