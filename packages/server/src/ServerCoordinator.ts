import { Server as HttpServer } from 'http';
import { Server as HttpsServer } from 'https';
import { HLC, LWWMap, ORMap, PermissionPolicy, Timestamp, LWWRecord, ConsistencyLevel, ReplicationConfig } from '@topgunbuild/core';
import { IServerStorage } from './storage/IServerStorage';
import { IInterceptor } from './interceptor/IInterceptor';
import { ServerDependencies } from './ServerDependencies';
import { ClusterManager } from './cluster/ClusterManager';
import { PartitionService } from './cluster/PartitionService';
import { ReplicationPipeline } from './cluster/ReplicationPipeline';
import { Query } from './query/Matcher';
import { logger } from './utils/logger';
import { TLSConfig, ClusterTLSConfig } from './types/TLSConfig';
import { StripedEventExecutor } from './utils/StripedEventExecutor';
import { CoalescingPreset } from './utils/coalescingPresets';
import { ConnectionRateLimiter } from './utils/ConnectionRateLimiter';
import { WorkerPool, MerkleWorker, CRDTMergeWorker, SerializationWorker, WorkerPoolConfig } from './workers';
import { ObjectPool, PooledEventPayload } from './memory';
import { TaskletScheduler } from './tasklet';
import { PartitionReassigner } from './cluster/PartitionReassigner';
import { MerkleTreeManager } from './cluster/MerkleTreeManager';
import { RepairScheduler } from './cluster/RepairScheduler';
import { EventJournalServiceConfig } from './EventJournalService';
import { SearchCoordinator, type ClusterSearchConfig } from './search';
import type { MergeRejection, FullTextIndexConfig } from '@topgunbuild/core';
import {
    ConnectionManager,
    StorageManager,
    OperationHandler,
    BroadcastHandler,
    GCHandler,
    HeartbeatHandler,
    ClientMessageHandler,
    OperationContextHandler,
    QueryConversionHandler,
    BatchProcessingHandler,
    WebSocketHandler,
    LifecycleManager,
} from './coordinator';

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

    // === Write Concern Options ===
    /** Default timeout for Write Concern acknowledgments in ms (default: 5000) */
    writeAckTimeout?: number;

    // === Replication Options ===
    /** Enable replication to backup nodes (default: true when cluster has peers) */
    replicationEnabled?: boolean;
    /** Default consistency level for replication (default: EVENTUAL) */
    defaultConsistency?: ConsistencyLevel;
    /** Replication configuration */
    replicationConfig?: Partial<ReplicationConfig>;

    // === Event Journal Options ===
    /** Enable event journal for audit/CDC (default: false) */
    eventJournalEnabled?: boolean;
    /** Event journal configuration */
    eventJournalConfig?: Partial<Omit<EventJournalServiceConfig, 'pool'>>;

    // === Full-Text Search Options ===
    /** Enable full-text search for specific maps */
    fullTextSearch?: Record<string, FullTextIndexConfig>;

    // === Distributed Search Options ===
    /** Configuration for distributed search across cluster nodes */
    distributedSearch?: ClusterSearchConfig;

    // === Debug Options  ===
    /** Enable debug endpoints (/debug/crdt/*, /debug/search/*) (default: false, or TOPGUN_DEBUG_ENDPOINTS=true) */
    debugEnabled?: boolean;
}

export class ServerCoordinator {
    private httpServer: HttpServer | HttpsServer;
    private metricsServer?: HttpServer;
    private connectionManager!: ConnectionManager;

    // In-memory storage - delegated to StorageManager
    private storageManager!: StorageManager;
    private hlc: HLC;
    private storage?: IServerStorage;

    private cluster!: ClusterManager;
    private partitionService!: PartitionService;
    private operationHandler!: OperationHandler;
    private webSocketHandler!: WebSocketHandler;
    private lifecycleManager!: LifecycleManager;
    private broadcastHandler!: BroadcastHandler;
    private gcHandler!: GCHandler;
    private heartbeatHandler!: HeartbeatHandler;
    private clientMessageHandler!: ClientMessageHandler;
    private operationContextHandler!: OperationContextHandler;
    private queryConversionHandler!: QueryConversionHandler;
    private batchProcessingHandler!: BatchProcessingHandler;

    // Track pending batch operations for testing purposes
    private pendingBatchOperations: Set<Promise<void>> = new Set();

    // Bounded event queue executor for backpressure control
    private eventExecutor: StripedEventExecutor;

    // Connection rate limiter
    private rateLimiter: ConnectionRateLimiter;

    // Worker pool for CPU-bound operations
    private workerPool?: WorkerPool;
    private merkleWorker?: MerkleWorker;
    private crdtMergeWorker?: CRDTMergeWorker;
    private serializationWorker?: SerializationWorker;

    // Memory pools for GC pressure reduction
    private eventPayloadPool: ObjectPool<PooledEventPayload>;

    // Tasklet scheduler for cooperative multitasking
    private taskletScheduler: TaskletScheduler;

    // Cluster Enhancements
    private partitionReassigner?: PartitionReassigner;
    private merkleTreeManager?: MerkleTreeManager;
    private repairScheduler?: RepairScheduler;

    // Replication
    private replicationPipeline?: ReplicationPipeline;

    // Full-Text Search
    private searchCoordinator!: SearchCoordinator;

    private readonly _nodeId: string;

    private _actualPort: number = 0;
    private _actualClusterPort: number = 0;
    private _readyPromise: Promise<void>;
    private _readyResolve!: () => void;
    private _readyReject!: (err: Error) => void;

    constructor(
        config: ServerCoordinatorConfig,
        dependencies: ServerDependencies
    ) {
        this._readyPromise = new Promise((resolve, reject) => {
            this._readyResolve = resolve;
            this._readyReject = reject;
        });

        this._nodeId = config.nodeId;

        // Inject Dependencies
        this.hlc = dependencies.hlc;
        this.eventExecutor = dependencies.eventExecutor;
        this.connectionManager = dependencies.connectionManager;
        this.cluster = dependencies.cluster;
        this.partitionService = dependencies.partitionService;
        this.storageManager = dependencies.storageManager;
        this.eventPayloadPool = dependencies.eventPayloadPool;
        this.taskletScheduler = dependencies.taskletScheduler;
        this.rateLimiter = dependencies.rateLimiter;
        this.workerPool = dependencies.workerPool;
        this.merkleWorker = dependencies.merkleWorker;
        this.crdtMergeWorker = dependencies.crdtMergeWorker;
        this.serializationWorker = dependencies.serializationWorker;
        this.httpServer = dependencies.httpServer;
        this.metricsServer = dependencies.metricsServer;
        this.partitionReassigner = dependencies.partitionReassigner;
        this.merkleTreeManager = dependencies.merkleTreeManager;
        this.repairScheduler = dependencies.repairScheduler;
        this.searchCoordinator = dependencies.searchCoordinator;
        this.pendingBatchOperations = dependencies.pendingBatchOperations;
        this.storage = config.storage;

        // Constructor-only locals for wiring (Part 3)
        const wss = dependencies.wss;
        this.replicationPipeline = dependencies.replicationPipeline;
        const replicationPipeline = this.replicationPipeline;
        const lockManager = dependencies.lockManager;
        const conflictResolverHandler = dependencies.conflictResolverHandler;
        const messageRegistry = dependencies.messageRegistry;

        // Initialize Listeners & Wiring (Minimal logic required to bind handlers to THIS instance)

        // Setup operation applier for incoming replications
        if (replicationPipeline) {
            replicationPipeline.setOperationApplier(this.applyReplicatedOperation.bind(this));
        }

        // Listen for partition map changes
        this.partitionService.on('rebalanced', (partitionMap) => {
            this.broadcastPartitionMap(partitionMap);
        });

        // Wire up LockManager
        lockManager.on('lockGranted', (evt) => this.handleLockGranted(evt));

        // Wire up ConflictResolver
        conflictResolverHandler.onRejection((rejection) => {
            this.notifyMergeRejection(rejection);
        });

        // Wire up PartitionFailover
        if (this.partitionReassigner) {
            this.partitionReassigner.on('failoverComplete', (event) => {
                logger.info({
                    failedNodeId: event.failedNodeId,
                    partitionsReassigned: event.partitionsReassigned,
                    durationMs: event.durationMs
                }, 'Partition failover completed');
                this.broadcastPartitionMap(this.partitionService.getPartitionMap());
            });
        }

        // Wire up RepairScheduler
        if (this.repairScheduler) {
            this.repairScheduler.setDataAccessors(
                (key: string) => this.getLocalRecord(key) ?? undefined,
                (key: string, record: any) => this.applyRepairRecord(key, record)
            );
            this.repairScheduler.start();
        }

        // Wire up FTS updates
        this.searchCoordinator.setDocumentValueGetter((mapName, key) => {
            const map = this.storageManager.getMaps().get(mapName);
            if (!map) return undefined;
            return map.get(key);
        });

        this.searchCoordinator.setSendUpdateCallback((clientId, subscriptionId, key, value, score, matchedTerms, changeType) => {
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
                        changeType,
                    }
                });
            }
        });

        // Initialize Handlers
        // Independent handlers injected from dependencies
        this.broadcastHandler = dependencies.broadcastHandler;
        this.heartbeatHandler = dependencies.heartbeatHandler;
        this.operationContextHandler = dependencies.operationContextHandler;
        this.operationHandler = dependencies.operationHandler;
        this.webSocketHandler = dependencies.webSocketHandler;
        this.clientMessageHandler = dependencies.clientMessageHandler;
        this.gcHandler = dependencies.gcHandler;
        this.queryConversionHandler = dependencies.queryConversionHandler;
        this.batchProcessingHandler = dependencies.batchProcessingHandler;
        this.lifecycleManager = dependencies.lifecycleManager;

        // Set coordinator callbacks via late binding pattern
        this.gcHandler.setCoordinatorCallbacks({
            broadcast: this.broadcast.bind(this),
        });

        this.batchProcessingHandler.setCoordinatorCallbacks({
            broadcastBatch: this.broadcastBatch.bind(this),
            broadcastBatchSync: this.broadcastBatchSync.bind(this),
        });

        // Set message registry on WebSocketHandler (late binding)
        this.webSocketHandler.setMessageRegistry(messageRegistry);

        this.heartbeatHandler.start();
        this.gcHandler.start();

        // Listen for connections and errors
        wss.on('connection', (ws) => this.webSocketHandler.handleConnection(ws));
        this.httpServer.on('error', (err) => {
            logger.error({ err }, 'HTTP Server error');
        });

        // Port is set later by completeStartup() once the HTTP server is listening

        // Initialize storage and backfill FTS if needed
        if (this.storage) {
            this.storage.initialize().then(async () => {
                logger.info('Storage adapter initialized');
                await this.ready();
                this.lifecycleManager.backfillSearchIndexes();
            }).catch(err => {
                logger.error({ err }, 'Failed to initialize storage');
            });
        }

    }

    // NOTE: backfillSearchIndexes moved to LifecycleManager

    /** Wait for server to be fully ready (ports assigned, cluster started) */
    public ready(): Promise<void> {
        return this._readyPromise;
    }

    /**
     * Signal that server startup failed so that ready() rejects with the error.
     * Called by ServerFactory when network.start() or cluster.start() fails.
     */
    public failStartup(err: Error): void {
        logger.error({ err }, 'Server failed to start');
        this._readyReject(err);
    }

    /**
     * Complete server startup after both HTTP and cluster servers are listening.
     * Called by ServerFactory once network.start() and cluster.start() resolve.
     */
    public completeStartup(actualPort: number, actualClusterPort: number): void {
        this._actualPort = actualPort;
        this._actualClusterPort = actualClusterPort;
        logger.info({
            port: actualPort,
            nodeId: this._nodeId,
            mode: 'CLUSTERED'
        }, 'Server Coordinator Initialized via Factory');
        this._readyResolve();
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

    // === Full-Text Search Public API ===

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

    // NOTE: gracefulClusterDeparture, getOwnedPartitions, waitForReplicationFlush moved to LifecycleManager

    /**
     * Shutdown the server coordinator.
     * Delegates to LifecycleManager for graceful shutdown.
     */
    public async shutdown(): Promise<void> {
        return this.lifecycleManager.shutdown();
    }

    // NOTE: handleConnection and handleMessage have been extracted to WebSocketHandler

    // ============ Partition Map Broadcast ============

    /**
     * Broadcast partition map to all connected and authenticated clients.
     * Called when partition topology changes (node join/leave/failover).
     */
    private broadcastPartitionMap(partitionMap: any): void {
        this.clientMessageHandler.broadcastPartitionMap(partitionMap);
    }

    /**
     * Notify a client about a merge rejection.
     * Finds the client by node ID and sends MERGE_REJECTED message.
     */
    private notifyMergeRejection(rejection: MergeRejection): void {
        this.clientMessageHandler.notifyMergeRejection(rejection);
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
        return this.queryConversionHandler.executeLocalQuery(mapName, query);
    }

    private async finalizeClusterQuery(requestId: string, timeout = false) {
        return this.queryConversionHandler.finalizeClusterQuery(requestId, timeout);
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
            const { eventPayload, rejected } = await this.operationHandler.applyOpToMap(op, sourceNode);

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

    private handleLockGranted({ clientId, requestId, name, fencingToken }: { clientId: string, requestId: string, name: string, fencingToken: number }) {
        this.operationContextHandler.handleLockGranted({ clientId, requestId, name, fencingToken });
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
     * Get local record for anti-entropy repair
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
     * Apply repaired record from anti-entropy repair
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


    // ============ Heartbeat Methods (delegated to handler) ============

    /**
     * Checks if a client is still alive based on heartbeat.
     * Delegates to HeartbeatHandler.
     */
    public isClientAlive(clientId: string): boolean {
        return this.heartbeatHandler.isClientAlive(clientId);
    }

    /**
     * Returns how long the client has been idle (no PING received).
     * Delegates to HeartbeatHandler.
     */
    public getClientIdleTime(clientId: string): number {
        return this.heartbeatHandler.getClientIdleTime(clientId);
    }
}
