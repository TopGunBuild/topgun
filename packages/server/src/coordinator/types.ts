import type { WebSocket } from 'ws';
import type { HLC, Principal, Timestamp, LWWMap, ORMap, FullTextIndexConfig, PermissionType } from '@topgunbuild/core';
import type { CoalescingWriter, CoalescingWriterOptions } from '../utils/CoalescingWriter';
import type { IServerStorage } from '../storage/IServerStorage';

/**
 * Default garbage collection age in milliseconds (30 days).
 * Records older than this are eligible for tombstone cleanup.
 */
export const DEFAULT_GC_AGE_MS = 30 * 24 * 60 * 60 * 1000;

/**
 * Represents a connected client with its WebSocket and state.
 */
export interface ClientConnection {
    id: string;
    socket: WebSocket;
    writer: CoalescingWriter;
    principal?: Principal;
    isAuthenticated: boolean;
    subscriptions: Set<string>;
    lastActiveHlc: Timestamp;
    lastPingReceived: number;
}

/**
 * Interface for managing client connections.
 * ConnectionManager is the single owner of the clients Map.
 */
export interface IConnectionManager {
    /** Get all connected clients (read-only access) */
    getClients(): Map<string, ClientConnection>;

    /** Get a specific client by ID */
    getClient(clientId: string): ClientConnection | undefined;

    /** Register a new client connection */
    registerClient(clientId: string, socket: WebSocket, writer: CoalescingWriter): ClientConnection;

    /** Remove client and return the removed connection (for cleanup) */
    removeClient(clientId: string): ClientConnection | undefined;

    /** Update client's authenticated state */
    setClientAuthenticated(clientId: string, principal: Principal): void;

    /** Broadcast message to all clients (optionally excluding one) */
    broadcast(message: any, excludeClientId?: string): void;

    /** Broadcast batch of events */
    broadcastBatch(events: any[], excludeClientId?: string): void;

    /** Check if client is alive based on heartbeat */
    isClientAlive(clientId: string): boolean;

    /** Get client idle time in ms */
    getClientIdleTime(clientId: string): number;

    /** Update client's last ping timestamp */
    updateLastPing(clientId: string): void;

    /** Get total client count */
    getClientCount(): number;
}

/**
 * Configuration for ConnectionManager.
 */
export interface ConnectionManagerConfig {
    hlc: HLC;
    writeCoalescingEnabled: boolean;
    writeCoalescingOptions: Partial<CoalescingWriterOptions>;
    /** Client heartbeat timeout in ms (default: 20000) */
    clientHeartbeatTimeoutMs?: number;
    /** Callback when a client is registered */
    onClientRegistered?: (client: ClientConnection) => void;
    /** Callback when a client is removed */
    onClientRemoved?: (clientId: string) => void;
}

// ============================================================================
// AuthHandler Types
// ============================================================================

/**
 * Result of an authentication attempt.
 */
export interface AuthResult {
    success: boolean;
    principal?: Principal;
    error?: string;
}

/**
 * Configuration for the AuthHandler.
 */
export interface AuthHandlerConfig {
    jwtSecret: string;
    onAuthSuccess?: (clientId: string, principal: Principal) => void;
    onAuthFailure?: (clientId: string, error: string) => void;
}

/**
 * Interface for authentication handling.
 * AuthHandler is stateless - it only processes tokens and updates client state.
 */
export interface IAuthHandler {
    /**
     * Verify a JWT token and return the principal.
     * @param token The JWT token to verify
     * @returns The decoded principal
     * @throws Error if token is invalid
     */
    verifyToken(token: string): Principal;

    /**
     * Handle an AUTH message from a client.
     * Updates client.principal and client.isAuthenticated on success.
     * @param client The client connection to authenticate
     * @param token The JWT token
     * @returns AuthResult with success status and principal or error
     */
    handleAuth(client: ClientConnection, token: string): Promise<AuthResult>;
}

// ============================================================================
// StorageManager Types
// ============================================================================

/**
 * Interface for managing in-memory CRDT maps and their storage persistence.
 * StorageManager is the single owner of the maps Map.
 */
export interface IStorageManager {
    /** Get or create a map by name (synchronous, may return empty map while loading) */
    getMap(name: string, typeHint?: 'LWW' | 'OR'): LWWMap<string, any> | ORMap<string, any>;

    /** Get map with async loading guarantee (waits for storage load) */
    getMapAsync(name: string, typeHint?: 'LWW' | 'OR'): Promise<LWWMap<string, any> | ORMap<string, any>>;

    /** Get all maps (for iteration/debug) */
    getMaps(): Map<string, LWWMap<string, any> | ORMap<string, any>>;

    /** Check if a map exists */
    hasMap(name: string): boolean;

    /** Load a map from storage (triggers async load) */
    loadMapFromStorage(name: string, typeHint: 'LWW' | 'OR'): Promise<void>;

    /** Check if map is currently loading */
    isMapLoading(name: string): boolean;
}

/**
 * Configuration for StorageManager.
 */
export interface StorageManagerConfig {
    nodeId: string;
    hlc: HLC;
    storage?: IServerStorage;
    fullTextSearch?: Record<string, FullTextIndexConfig>;
    /** Function to check if a key is related to this node (owner or backup) */
    isRelatedKey?: (key: string) => boolean;
    /** Callback when a map is loaded from storage */
    onMapLoaded?: (mapName: string, recordCount: number) => void;
}

// ============================================================================
// OperationHandler Types
// ============================================================================

/**
 * Interface for handling CRDT operations (CLIENT_OP, OP_BATCH).
 * OperationHandler processes write operations and manages Write Concern.
 */
export interface IOperationHandler {
    /** Process a single client operation (CLIENT_OP) */
    processClientOp(client: ClientConnection, op: any): Promise<void>;

    /** Process a batch of operations (OP_BATCH) */
    processOpBatch(
        client: ClientConnection,
        ops: any[],
        writeConcern?: string,
        timeout?: number
    ): Promise<void>;

    /** Core operation processing logic */
    processLocalOp(op: any, fromCluster: boolean, originalSenderId?: string): Promise<void>;

    /** Core CRDT merge logic - applies operation to map and returns event payload */
    applyOpToMap(op: any, remoteNodeId?: string): Promise<{ eventPayload: any; oldRecord: any; rejected?: boolean }>;
}

/**
 * Configuration for OperationHandler.
 * This handler requires many dependencies due to the complexity of
 * CRDT operations (interceptors, Write Concern, replication, journal, search).
 */
// Core OperationHandler Configuration
export interface OperationHandlerConfig {
    nodeId: string;
    hlc: HLC;
    metricsService: any; // Using any for now to avoid circular imports, ideally import MetricsService interface
    securityManager: {
        checkPermission: (principal: Principal, mapName: string, action: PermissionType) => boolean;
    };
    storageManager: IStorageManager;
    conflictResolverHandler: {
        hasResolvers: (mapName: string) => boolean;
        mergeWithResolver: (map: any, mapName: string, key: string, record: any, nodeId: string) => Promise<any>;
    };
    queryRegistry: {
        processChange: (mapName: string, map: any, key: string, record: any, oldValue: any) => void;
    };
    eventJournalService?: {
        append: (event: any) => void;
    };
    merkleTreeManager?: {
        updateRecord: (partitionId: number, key: string, record: any) => void;
    };
    partitionService: {
        getPartitionId: (key: string) => number;
        getOwner: (key: string) => string;
        isLocalOwner: (key: string) => boolean;
    };
    searchCoordinator: {
        isSearchEnabled: (mapName: string) => boolean;
        onDataChange: (mapName: string, key: string, value: any, changeType: string) => void;
    };
    storage?: IServerStorage | null;
    replicationPipeline?: {
        replicate: (op: any, opId: string, key: string) => Promise<void>;
    };
    broadcastHandler: IBroadcastHandler;
    operationContextHandler: IOperationContextHandler;

    // Optional for backpressure customization
    backpressure?: {
        registerPending: () => boolean;
        waitForCapacity: () => Promise<void>;
        shouldForceSync: () => boolean;
    };
}

// ============================================================================
// PartitionHandler Types
// ============================================================================

/**
 * Interface for handling PARTITION_MAP_REQUEST messages.
 */
export interface IPartitionHandler {
    handlePartitionMapRequest(client: ClientConnection, message: any): void;
}

/**
 * Configuration for PartitionHandler.
 */
export interface PartitionHandlerConfig {
    partitionService: {
        getPartitionMap: () => any;
    };
}

// ============================================================================
// TopicHandler Types
// ============================================================================

/**
 * Interface for handling topic pub/sub messages.
 */
export interface ITopicHandler {
    handleTopicSub(client: ClientConnection, message: any): void;
    handleTopicUnsub(client: ClientConnection, message: any): void;
    handleTopicPub(client: ClientConnection, message: any): void;
}

/**
 * Configuration for TopicHandler.
 */
export interface TopicHandlerConfig {
    topicManager: {
        subscribe: (clientId: string, topic: string) => void;
        unsubscribe: (clientId: string, topic: string) => void;
        publish: (topic: string, data: any, senderId: string) => void;
    };
    securityManager: {
        checkPermission: (principal: Principal, resource: string, action: PermissionType) => boolean;
    };
}

// ============================================================================
// LockHandler Types
// ============================================================================

/**
 * Interface for handling distributed lock messages.
 */
export interface ILockHandler {
    handleLockRequest(client: ClientConnection, message: any): void;
    handleLockRelease(client: ClientConnection, message: any): void;
}

/**
 * Configuration for LockHandler.
 */
export interface LockHandlerConfig {
    lockManager: {
        acquire: (name: string, clientId: string, requestId: string, ttl: number) => { granted: boolean; fencingToken?: number };
        release: (name: string, clientId: string, fencingToken: number) => boolean;
    };
    partitionService: {
        isLocalOwner: (key: string) => boolean;
        getOwner: (key: string) => string;
    };
    cluster: {
        getMembers: () => string[];
        send: (nodeId: string, type: any, payload: any) => void;
        config: { nodeId: string };
    };
    securityManager: {
        checkPermission: (principal: Principal, resource: string, action: PermissionType) => boolean;
    };
}

// ============================================================================
// CounterHandlerAdapter Types
// ============================================================================

/**
 * Interface for handling PN Counter messages.
 */
export interface ICounterHandlerAdapter {
    handleCounterRequest(client: ClientConnection, message: any): void;
    handleCounterSync(client: ClientConnection, message: any): void;
}

/**
 * Configuration for CounterHandlerAdapter.
 */
export interface CounterHandlerAdapterConfig {
    counterHandler: {
        handleCounterRequest: (clientId: string, name: string) => any;
        handleCounterSync: (clientId: string, name: string, state: any) => {
            response: any;
            broadcastTo: string[];
            broadcastMessage: any;
        };
    };
    getClient: (clientId: string) => ClientConnection | undefined;
}

// ============================================================================
// ResolverHandler Types
// ============================================================================

/**
 * Interface for handling conflict resolver messages.
 */
export interface IResolverHandler {
    handleRegisterResolver(client: ClientConnection, message: any): void;
    handleUnregisterResolver(client: ClientConnection, message: any): void;
    handleListResolvers(client: ClientConnection, message: any): void;
}

/**
 * Configuration for ResolverHandler.
 */
export interface ResolverHandlerConfig {
    conflictResolverHandler: {
        registerResolver: (mapName: string, resolver: any, clientId: string) => void;
        unregisterResolver: (mapName: string, resolverName: string, clientId: string) => boolean;
        listResolvers: (mapName?: string) => any[];
    };
    securityManager: {
        checkPermission: (principal: Principal, resource: string, action: PermissionType) => boolean;
    };
}

// ============================================================================
// JournalHandler Types
// ============================================================================

/**
 * Interface for handling event journal messages.
 */
export interface IJournalHandler {
    handleJournalSubscribe(client: ClientConnection, message: any): void;
    handleJournalUnsubscribe(client: ClientConnection, message: any): void;
    handleJournalRead(client: ClientConnection, message: any): void;
}

/**
 * Configuration for JournalHandler.
 */
export interface JournalHandlerConfig {
    eventJournalService?: {
        subscribe: (callback: (event: any) => void, fromSequence?: bigint) => () => void;
        readFrom: (startSeq: bigint, limit: number) => any[];
    };
    journalSubscriptions: Map<string, { clientId: string; mapName?: string; types?: string[] }>;
    getClient: (clientId: string) => ClientConnection | undefined;
}

// ============================================================================
// LwwSyncHandler Types
// ============================================================================

/**
 * Interface for handling LWW map sync messages.
 */
export interface ILwwSyncHandler {
    handleSyncInit(client: ClientConnection, message: any): Promise<void>;
    handleMerkleReqBucket(client: ClientConnection, message: any): Promise<void>;
}

/**
 * Configuration for LwwSyncHandler.
 */
export interface LwwSyncHandlerConfig {
    getMapAsync: (name: string, typeHint?: 'LWW' | 'OR') => Promise<LWWMap<string, any> | ORMap<string, any>>;
    hlc: HLC;
    securityManager: {
        checkPermission: (principal: Principal, resource: string, action: PermissionType) => boolean;
    };
    metricsService: {
        incOp: (op: any, mapName: string) => void;
    };
    gcAgeMs: number;
}

// ============================================================================
// ORMapSyncHandler Types
// ============================================================================

/**
 * Interface for handling ORMap sync messages.
 */
export interface IORMapSyncHandler {
    handleORMapSyncInit(client: ClientConnection, message: any): Promise<void>;
    handleORMapMerkleReqBucket(client: ClientConnection, message: any): Promise<void>;
    handleORMapDiffRequest(client: ClientConnection, message: any): Promise<void>;
    handleORMapPushDiff(client: ClientConnection, message: any): Promise<void>;
}

/**
 * Configuration for ORMapSyncHandler.
 */
export interface ORMapSyncHandlerConfig {
    getMapAsync: (name: string, typeHint?: 'LWW' | 'OR') => Promise<LWWMap<string, any> | ORMap<string, any>>;
    hlc: HLC;
    securityManager: {
        checkPermission: (principal: Principal, resource: string, action: PermissionType) => boolean;
    };
    metricsService: {
        incOp: (op: any, mapName: string) => void;
    };
    storage?: IServerStorage;
    broadcast: (message: any, excludeClientId?: string) => void;
    gcAgeMs: number;
}

// ============================================================================
// EntryProcessorAdapter Types
// ============================================================================

/**
 * Interface for handling entry processor messages.
 */
export interface IEntryProcessorAdapter {
    handleEntryProcess(client: ClientConnection, message: any): Promise<void>;
    handleEntryProcessBatch(client: ClientConnection, message: any): Promise<void>;
}

/**
 * Configuration for EntryProcessorAdapter.
 */
export interface EntryProcessorAdapterConfig {
    entryProcessorHandler: {
        executeOnKey: (map: LWWMap<string, any>, key: string, processor: any) => Promise<{
            result: { success: boolean; result?: any; newValue?: any; error?: string };
            timestamp?: any;
        }>;
        executeOnKeys: (map: LWWMap<string, any>, keys: string[], processor: any) => Promise<{
            results: Map<string, { success: boolean; result?: any; newValue?: any; error?: string }>;
            timestamps: Map<string, any>;
        }>;
    };
    getMap: (name: string) => LWWMap<string, any> | ORMap<string, any>;
    securityManager: {
        checkPermission: (principal: Principal, resource: string, action: PermissionType) => boolean;
    };
    queryRegistry: {
        processChange: (mapName: string, map: any, key: string, record: any, oldValue: any) => void;
    };
}

// ============================================================================
// SearchHandler Types
// ============================================================================

/**
 * Interface for handling full-text search messages.
 */
export interface ISearchHandler {
    handleSearch(client: ClientConnection, message: any): Promise<void>;
    handleSearchSub(client: ClientConnection, message: any): Promise<void>;
    handleSearchUnsub(client: ClientConnection, message: any): void;
}

/**
 * Configuration for SearchHandler.
 */
export interface SearchHandlerConfig {
    searchCoordinator: {
        isSearchEnabled: (mapName: string) => boolean;
        search: (mapName: string, query: string, options?: any) => any;
        subscribe: (clientId: string, subscriptionId: string, mapName: string, query: string, options?: any) => any[];
        unsubscribe: (subscriptionId: string) => void;
    };
    clusterSearchCoordinator?: {
        search: (mapName: string, query: string, options: any) => Promise<any>;
    };
    distributedSubCoordinator?: {
        subscribeSearch: (subscriptionId: string, socket: WebSocket, mapName: string, query: string, options: any) => Promise<any>;
        unsubscribe: (subscriptionId: string) => Promise<void>;
    };
    cluster: {
        getMembers: () => string[];
    };
    securityManager: {
        checkPermission: (principal: Principal, resource: string, action: PermissionType) => boolean;
    };
}

// ============================================================================
// QueryHandler Types
// ============================================================================

/**
 * Interface for handling query subscription messages.
 */
export interface IQueryHandler {
    handleQuerySub(client: ClientConnection, message: any): Promise<void>;
    handleQueryUnsub(client: ClientConnection, message: any): Promise<void>;
}

/**
 * Configuration for QueryHandler.
 */
export interface QueryHandlerConfig {
    securityManager: {
        checkPermission: (principal: Principal, resource: string, action: PermissionType) => boolean;
        filterObject: (value: any, principal: Principal, mapName: string) => any;
    };
    metricsService: {
        incOp: (op: any, mapName: string) => void;
    };
    queryRegistry: {
        unregister: (queryId: string) => void;
    };
    distributedSubCoordinator?: {
        subscribeQuery: (queryId: string, socket: WebSocket, mapName: string, query: any) => Promise<any>;
        unsubscribe: (id: string) => Promise<void>;
    };
    cluster: {
        getMembers: () => string[];
        isLocal: (id: string) => boolean;
        send: (nodeId: string, type: any, payload: any) => void;
        config: { nodeId: string };
    };
    executeLocalQuery: (mapName: string, query: any) => Promise<any[]>;
    finalizeClusterQuery: (requestId: string, timeout?: boolean) => Promise<void>;
    pendingClusterQueries: Map<string, any>;
    readReplicaHandler?: {
        selectReadNode: (req: any) => string | null;
    };
    ConsistencyLevel: { EVENTUAL: any };
}

// ============================================================================
// BroadcastHandler Types
// ============================================================================

/**
 * Interface for handling broadcast operations.
 */
export interface IBroadcastHandler {
    broadcast(message: any, excludeClientId?: string): void;
    broadcastBatch(events: any[], excludeClientId?: string): void;
    broadcastBatchSync(events: any[], excludeClientId?: string): Promise<void>;
}

/**
 * Configuration for BroadcastHandler.
 */
export interface BroadcastHandlerConfig {
    connectionManager: IConnectionManager;
    securityManager: { filterObject: (value: any, principal: Principal, mapName: string) => any };
    queryRegistry: { getSubscribedClientIds: (mapName: string) => Set<string> };
    metricsService: {
        incEventsRouted: () => void;
        incEventsFilteredBySubscription: () => void;
        recordSubscribersPerEvent: (count: number) => void;
    };
    hlc: HLC;
}

// ============================================================================
// GCHandler Types
// ============================================================================

/**
 * Interface for handling garbage collection operations.
 */
export interface IGCHandler {
    start(): void;
    stop(): void;
    handleGcReport(nodeId: string, minHlc: Timestamp): void;
    performGarbageCollection(olderThan: Timestamp): void;
}

/**
 * Configuration for GCHandler.
 */
export interface GCHandlerConfig {
    storageManager: IStorageManager;
    connectionManager: IConnectionManager;
    cluster: {
        getMembers: () => string[];
        send: (nodeId: string, type: any, payload: any) => void;
        isLocal: (id: string) => boolean;
        config: { nodeId: string };
        getLeaderId?: () => string | null;
    };
    partitionService: {
        isRelated: (key: string) => boolean;
        getPartitionId: (key: string) => number;
    };
    replicationPipeline?: {
        replicate: (op: any, opId: string, key: string) => Promise<void>;
    };
    merkleTreeManager?: {
        updateRecord: (partitionId: number, key: string, record: any) => void;
    };
    queryRegistry: {
        processChange: (mapName: string, map: any, key: string, record: any, oldValue: any) => void;
    };
    hlc: HLC;
    storage?: IServerStorage;
    broadcast?: (message: any) => void;
    metricsService: { incOp: (op: any, mapName: string) => void };
    gcIntervalMs?: number;
    gcAgeMs?: number;
}

// ============================================================================
// ClusterEventHandler Types
// ============================================================================

/**
 * Interface for handling cluster event messages.
 */
export interface IClusterEventHandler {
    setupListeners(): void;
    teardownListeners(): void;
}

// ============================================================================
// HeartbeatHandler Types (SPEC-003d)
// ============================================================================

/**
 * Interface for handling client heartbeat/ping operations.
 */
export interface IHeartbeatHandler {
    /** Start the periodic heartbeat check */
    start(): void;
    /** Stop the periodic heartbeat check */
    stop(): void;
    /** Handle incoming PING from client */
    handlePing(client: ClientConnection, clientTimestamp: number): void;
    /** Check if a client is still alive based on heartbeat */
    isClientAlive(clientId: string): boolean;
    /** Get client idle time in ms */
    getClientIdleTime(clientId: string): number;
}

/**
 * Configuration for HeartbeatHandler.
 */
export interface HeartbeatHandlerConfig {
    connectionManager: IConnectionManager;
    /** Client heartbeat timeout in ms (default: 20000) */
    heartbeatTimeoutMs?: number;
    /** Heartbeat check interval in ms (default: 5000) */
    heartbeatCheckIntervalMs?: number;
}

/**
 * Interface for ClientMessageHandler.
 */
export interface IClientMessageHandler {
    /** Update client's HLC timestamp from incoming message */
    updateClientHlc(client: ClientConnection, message: any): void;
    /** Broadcast partition map to all authenticated clients */
    broadcastPartitionMap(partitionMap: any): void;
    /** Notify client about merge rejection */
    notifyMergeRejection(rejection: any): void;
}

/**
 * Configuration for ClientMessageHandler.
 */
export interface ClientMessageHandlerConfig {
    connectionManager: IConnectionManager;
    queryRegistry: {
        getSubscribedClientIds: (mapName: string) => Set<string>;
    };
    hlc: HLC;
}

/**
 * Interface for PersistenceHandler.
 */
export interface IPersistenceHandler {
    /** Persist operation synchronously (blocking) */
    persistOpSync(op: any): Promise<void>;
    /** Persist operation asynchronously (fire-and-forget) */
    persistOpAsync(op: any): Promise<void>;
}

/**
 * Configuration for PersistenceHandler.
 */
export interface PersistenceHandlerConfig {
    storage: IServerStorage | null;
    getMap: (mapName: string, type: 'LWW' | 'OR') => LWWMap<string, any> | ORMap<string, any>;
}

/**
 * Interface for OperationContextHandler.
 */
export interface IOperationContextHandler {
    /** Build operation context from clientId */
    buildOpContext(clientId: string, fromCluster: boolean): any;
    /** Run before interceptors on operation */
    runBeforeInterceptors(op: any, context: any): Promise<any | null>;
    /** Run after interceptors on operation (fire-and-forget) */
    runAfterInterceptors(op: any, context: any): void;
    /** Handle lock granted notification */
    handleLockGranted(event: { clientId: string, requestId: string, name: string, fencingToken: number }): void;
}

/**
 * Configuration for OperationContextHandler.
 */
export interface OperationContextHandlerConfig {
    connectionManager: IConnectionManager;
    interceptors: any[];
    cluster: {
        config: { nodeId: string };
        send: (nodeId: string, type: any, payload: any) => void;
    };
}

/**
 * Interface for QueryConversionHandler.
 */
export interface IQueryConversionHandler {
    /** Execute query on local map */
    executeLocalQuery(mapName: string, query: any): Promise<any[]>;
    /** Convert server query to core query format */
    convertToCoreQuery(query: any): any | null;
    /** Convert predicate AST to core query */
    predicateToCoreQuery(predicate: any): any | null;
    /** Convert server operator to core operator */
    convertOperator(op: string): 'eq' | 'neq' | 'gt' | 'gte' | 'lt' | 'lte' | null;
    /** Finalize cluster query with aggregation and pagination */
    finalizeClusterQuery(requestId: string, timeout?: boolean): Promise<void>;
}

/**
 * Configuration for QueryConversionHandler.
 */
export interface QueryConversionHandlerConfig {
    getMapAsync: (mapName: string, typeHint?: 'LWW' | 'OR') => Promise<LWWMap<string, any> | ORMap<string, any>>;
    pendingClusterQueries: Map<string, any>;
    queryRegistry: any;
    securityManager: {
        filterObject: (value: any, principal: any, mapName: string) => any;
    };
}

/**
 * Configuration for ClusterEventHandler.
 */
export interface ClusterEventHandlerConfig {
    cluster: {
        on: (event: string, handler: (...args: any[]) => void) => void;
        off?: (event: string, handler: (...args: any[]) => void) => void;
        send: (nodeId: string, type: any, payload: any) => void;
        config: { nodeId: string };
    };
    partitionService: {
        isLocalOwner: (key: string) => boolean;
        getOwner: (key: string) => string;
        isRelated: (key: string) => boolean;
    };
    lockManager: {
        acquire: (name: string, clientId: string, requestId: string, ttl: number) => { granted: boolean; fencingToken?: number };
        release: (name: string, clientId: string, fencingToken: number) => boolean;
        handleClientDisconnect: (clientId: string) => void;
    };
    topicManager: {
        publish: (topic: string, data: any, senderId: string, fromCluster?: boolean) => void;
    };
    repairScheduler?: {
        emit: (event: string, data: any) => void;
    };
    connectionManager: IConnectionManager;
    storageManager: IStorageManager;
    queryRegistry: {
        processChange: (mapName: string, map: any, key: string, record: any, oldValue: any) => void;
    };
    metricsService: {
        incOp: (op: any, mapName: string) => void;
        setClusterMembers: (count: number) => void;
    };
    gcHandler: IGCHandler;
    hlc: HLC;
    merkleTreeManager?: {
        getRootHash: (partitionId: number) => number;
    };

    // Callbacks for operations that remain in ServerCoordinator
    processLocalOp: (op: any, fromCluster: boolean, senderId?: string) => Promise<void>;
    executeLocalQuery: (mapName: string, query: any) => Promise<any[]>;
    finalizeClusterQuery: (requestId: string, timeout?: boolean) => Promise<void>;
    getLocalRecord: (key: string) => any;
    broadcast: (message: any, excludeClientId?: string) => void;
    getMap: (name: string, typeHint: 'LWW' | 'OR') => any;
    pendingClusterQueries: Map<string, any>;
}

// ============================================================================
// BatchProcessingHandler Types (SPEC-003d)
// ============================================================================

/**
 * Interface for handling batch operation processing with backpressure.
 */
export interface IBatchProcessingHandler {
    /** Process batch asynchronously with backpressure regulation */
    processBatchAsync(ops: any[], clientId: string): Promise<void>;
    /** Process batch synchronously (blocking) */
    processBatchSync(ops: any[], clientId: string): Promise<void>;
    /** Process single operation for batch (collects events instead of immediate broadcast) */
    processLocalOpForBatch(op: any, clientId: string, batchedEvents: any[]): Promise<void>;
    /** Forward operation to partition owner and wait for completion */
    forwardOpAndWait(op: any, owner: string): Promise<void>;
}

/**
 * Configuration for BatchProcessingHandler.
 */
export interface BatchProcessingHandlerConfig {
    backpressure: {
        shouldForceSync: () => boolean;
        registerPending: () => boolean;
        waitForCapacity: () => Promise<void>;
        completePending: () => void;
        getPendingOps: () => number;
    };
    partitionService: {
        isLocalOwner: (key: string) => boolean;
        getOwner: (key: string) => string;
    };
    cluster: {
        sendToNode: (nodeId: string, message: any) => void;
    };
    metricsService: {
        incBackpressureSyncForced: () => void;
        incBackpressureWaits: () => void;
        incBackpressureTimeouts: () => void;
        setBackpressurePendingOps: (count: number) => void;
    };
    replicationPipeline?: {
        replicate: (op: any, opId: string, key: string) => Promise<any>;
    };
    broadcastBatch?: (events: any[], excludeClientId?: string) => void;
    broadcastBatchSync?: (events: any[], excludeClientId?: string) => Promise<void>;
    buildOpContext: (clientId: string, fromCluster: boolean) => any;
    runBeforeInterceptors: (op: any, context: any) => Promise<any | null>;
    runAfterInterceptors: (op: any, context: any) => void;
    applyOpToMap: (op: any, clientId?: string) => Promise<{ eventPayload: any; oldRecord?: any; rejected?: boolean }>;
}

// ============================================================================
// WriteConcernHandler Types (SPEC-003d)
// ============================================================================

/**
 * Interface for handling Write Concern tracking and acknowledgments.
 */
export interface IWriteConcernHandler {
    /** Get effective Write Concern level for an operation */
    getEffectiveWriteConcern(opLevel?: any, batchLevel?: any): any;
    /** Convert string WriteConcern value to enum */
    stringToWriteConcern(value?: any): any;
    /** Process batch asynchronously with Write Concern tracking */
    processBatchAsyncWithWriteConcern(
        ops: any[],
        clientId: string,
        batchWriteConcern?: any,
        batchTimeout?: number
    ): Promise<void>;
    /** Process batch synchronously with Write Concern tracking */
    processBatchSyncWithWriteConcern(
        ops: any[],
        clientId: string,
        batchWriteConcern?: any,
        batchTimeout?: number
    ): Promise<void>;
    /** Process single operation with Write Concern level notifications */
    processLocalOpWithWriteConcern(
        op: any,
        clientId: string,
        batchedEvents: any[],
        batchWriteConcern?: any
    ): Promise<void>;
}

/**
 * Configuration for WriteConcernHandler.
 */
export interface WriteConcernHandlerConfig {
    backpressure: {
        shouldForceSync: () => boolean;
        registerPending: () => boolean;
        waitForCapacity: () => Promise<void>;
        completePending: () => void;
        getPendingOps: () => number;
    };
    partitionService: {
        isLocalOwner: (key: string) => boolean;
        getOwner: (key: string) => string;
    };
    cluster: {
        sendToNode: (nodeId: string, message: any) => void;
    };
    metricsService: {
        incBackpressureSyncForced: () => void;
        incBackpressureWaits: () => void;
        incBackpressureTimeouts: () => void;
        setBackpressurePendingOps: (count: number) => void;
    };
    writeAckManager: {
        notifyLevel: (opId: string, level: any) => void;
        failPending: (opId: string, error: string) => void;
    };
    storage: IServerStorage | null;
    broadcastBatch: (events: any[], excludeClientId?: string) => void;
    broadcastBatchSync: (events: any[], excludeClientId?: string) => Promise<void>;
    buildOpContext: (clientId: string, fromCluster: boolean) => any;
    runBeforeInterceptors: (op: any, context: any) => Promise<any | null>;
    runAfterInterceptors: (op: any, context: any) => void;
    applyOpToMap: (op: any, clientId?: string) => Promise<{ eventPayload: any; oldRecord?: any; rejected?: boolean }>;
    persistOpSync: (op: any) => Promise<void>;
    persistOpAsync: (op: any) => Promise<void>;
}

// ============================================================================
// WebSocketHandler Types (Phase 1 Extraction)
// ============================================================================

/**
 * Interface for handling WebSocket connections and messages.
 */
export interface IWebSocketHandler {
    /** Handle new WebSocket connection */
    handleConnection(ws: WebSocket): Promise<void>;
    /** Handle incoming message from client */
    handleMessage(client: ClientConnection, rawMessage: any): Promise<void>;
    /** Set message registry for routing (late binding) */
    setMessageRegistry(registry: Record<string, (client: ClientConnection, message: any) => void | Promise<void>>): void;
}

/**
 * Configuration for WebSocketHandler.
 */
export interface WebSocketHandlerConfig {
    nodeId: string;
    rateLimitingEnabled: boolean;
    writeCoalescingEnabled: boolean;
    writeCoalescingOptions: any;
    interceptors: any[];

    // Dependencies
    rateLimiter: {
        shouldAccept: () => boolean;
        onConnectionAttempt: () => void;
        onConnectionRejected: () => void;
        onPendingConnectionFailed: () => void;
    };
    metricsService: {
        incConnectionsRejected: () => void;
        incConnectionsAccepted: () => void;
        setConnectedClients: (count: number) => void;
    };
    connectionManager: IConnectionManager;
    authHandler: IAuthHandler;
    rateLimitedLogger: {
        error: (key: string, context: any, message: string) => void;
    };

    // Message routing (optional - can be set later via setMessageRegistry)
    messageRegistry?: Record<string, (client: ClientConnection, message: any) => void | Promise<void>>;

    // Cleanup handlers
    queryRegistry: {
        unregister: (subId: string) => void;
    };
    lockManager: {
        handleClientDisconnect: (clientId: string) => void;
    };
    topicManager: {
        unsubscribeAll: (clientId: string) => void;
    };
    counterHandler: {
        unsubscribeAll: (clientId: string) => void;
    };
    searchCoordinator: {
        unsubscribeClient: (clientId: string) => void;
    };
    distributedSubCoordinator?: {
        unsubscribeClient: (socket: WebSocket) => void;
    };
    cluster: {
        getMembers: () => string[];
        isLocal: (id: string) => boolean;
        send: (nodeId: string, type: any, payload: any) => void;
        config: { nodeId: string };
    };

    // Heartbeat
    heartbeatHandler: IHeartbeatHandler;
    clientMessageHandler: IClientMessageHandler;
}
