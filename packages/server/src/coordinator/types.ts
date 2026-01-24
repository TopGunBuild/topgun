import type { WebSocket } from 'ws';
import type { HLC, Principal, Timestamp, LWWMap, ORMap, FullTextIndexConfig, PermissionType } from '@topgunbuild/core';
import type { CoalescingWriter, CoalescingWriterOptions } from '../utils/CoalescingWriter';
import type { IServerStorage } from '../storage/IServerStorage';

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
}

/**
 * Configuration for OperationHandler.
 * This handler requires many dependencies due to the complexity of
 * CRDT operations (interceptors, Write Concern, replication, journal, search).
 */
export interface OperationHandlerConfig {
    /** Process a local operation (delegates to ServerCoordinator) */
    processLocalOp: (op: any, isForwarded: boolean, originClientId?: string) => Promise<void>;

    /** Process a batch with Write Concern (delegates to ServerCoordinator) */
    processBatchAsync: (
        ops: any[],
        clientId: string,
        writeConcern?: any,
        timeout?: number
    ) => Promise<void>;

    /** Get effective Write Concern from op-level and batch-level settings */
    getEffectiveWriteConcern: (opLevel?: any, batchLevel?: any) => any;

    /** Convert Write Concern string to enum value */
    stringToWriteConcern: (wc?: any) => any;

    /** Forward operation to partition owner */
    forwardToOwner: (op: any) => void;

    /** Check if key is owned by local node */
    isLocalOwner: (key: string) => boolean;

    /** Security manager for permission checks */
    checkPermission: (principal: Principal, mapName: string, action: PermissionType) => boolean;

    /** Metrics service for operation tracking */
    incOp: (action: any, mapName: string) => void;

    /** Write ACK manager for deferred acknowledgments */
    writeAckManager: {
        registerPending: (
            opId: string,
            writeConcern: any,
            timeout?: number
        ) => Promise<{ success: boolean; achievedLevel: string; error?: string }>;
    };

    /** Track pending batch operations (for testing) */
    pendingBatchOperations: Set<Promise<void>>;
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
    finalizeClusterQuery: (requestId: string, timeout?: boolean) => void;
    pendingClusterQueries: Map<string, any>;
    readReplicaHandler?: {
        selectReadNode: (req: any) => string | null;
    };
    ConsistencyLevel: { EVENTUAL: any };
}
