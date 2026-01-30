/**
 * Handlers Module - Domain-grouped handler creation for Rust Actor Model portability
 *
 * This module extracts all handler instantiation from ServerFactory and groups them
 * by domain for eventual translation to Rust Actor Model.
 *
 * Handler Groups:
 * - CRDT: Operation, BatchProcessing, GC handlers
 * - Sync: LWW and ORMap sync handlers
 * - Query: Query and QueryConversion handlers
 * - Messaging: Topic and Broadcast handlers
 * - Coordination: Lock and Partition handlers
 * - Search: Search handler
 * - Persistence: Journal, Counter, EntryProcessor, Resolver handlers
 * - Client: Auth, WebSocket, ClientMessage handlers
 * - Server: Heartbeat, Persistence, OperationContext, WriteConcern handlers
 *
 * Created as part of SPEC-011d (ServerFactory Modularization for Rust-Portability).
 */

import { WebSocket } from 'ws';
import { ConsistencyLevel } from '@topgunbuild/core';
import {
  AuthHandler,
  OperationHandler,
  BatchProcessingHandler,
  GCHandler,
  LwwSyncHandler,
  ORMapSyncHandler,
  QueryHandler,
  QueryConversionHandler,
  TopicHandler,
  BroadcastHandler,
  LockHandler,
  PartitionHandler,
  SearchHandler,
  JournalHandler,
  CounterHandlerAdapter,
  EntryProcessorAdapter,
  ResolverHandler,
  WebSocketHandler,
  ClientMessageHandler,
  HeartbeatHandler,
  PersistenceHandler,
  OperationContextHandler,
  WriteConcernHandler,
  createMessageRegistry,
  DEFAULT_GC_AGE_MS,
} from '../coordinator';
import { CounterHandler } from '../handlers/CounterHandler';
import { EntryProcessorHandler } from '../handlers/EntryProcessorHandler';
import { ConflictResolverHandler } from '../handlers/ConflictResolverHandler';
import { TopicManager } from '../topic/TopicManager';
import { SearchCoordinator, ClusterSearchCoordinator } from '../search';
import { DistributedSubscriptionCoordinator } from '../subscriptions/DistributedSubscriptionCoordinator';
import { ConnectionManager } from '../coordinator/ConnectionManager';
import { EventJournalService } from '../EventJournalService';

import type {
  HandlersModule,
  HandlersModuleConfig,
  HandlersModuleDeps,
  CRDTHandlers,
  SyncHandlers,
  QueryHandlers,
  MessagingHandlers,
  CoordinationHandlers,
  SearchHandlers,
  PersistenceHandlers,
  ClientHandlers,
  ServerHandlers,
} from './types';

/**
 * Step 1: Create internal managers (no handler dependencies)
 *
 * These managers are created inside handlers-module to keep the external interface clean
 * while allowing internal complexity. They include:
 * - ConnectionManager: Manages WebSocket client connections
 * - TopicManager: Pub/sub topic management
 * - CounterHandler, EntryProcessorHandler, ConflictResolverHandler: Base handlers
 * - EventJournalService: Optional event journal
 * - SearchCoordinator, ClusterSearchCoordinator, DistributedSubscriptionCoordinator: Search coordination
 * - Shared state maps: pendingClusterQueries, pendingBatchOperations, journalSubscriptions
 */
function createInternalManagers(
  config: HandlersModuleConfig,
  deps: HandlersModuleDeps
): HandlersModule['_internal'] {
  // ConnectionManager needs hlc and write coalescing config
  const connectionManager = new ConnectionManager({
    hlc: deps.core.hlc,
    writeCoalescingEnabled: config.writeCoalescingEnabled ?? true,
    writeCoalescingOptions: config.writeCoalescingOptions,
  });

  // TopicManager needs cluster and a sendToClient callback
  const topicManager = new TopicManager({
    cluster: deps.cluster.cluster,
    sendToClient: (clientId, message) => {
      const client = connectionManager.getClient(clientId);
      if (client && client.socket.readyState === WebSocket.OPEN) {
        client.writer.write(message);
      }
    }
  });

  // CounterHandler and EntryProcessorHandler (base classes)
  const counterHandler = new CounterHandler(config.nodeId);
  const entryProcessorHandler = new EntryProcessorHandler({ hlc: deps.core.hlc });
  const conflictResolverHandler = new ConflictResolverHandler({ nodeId: config.nodeId });

  // EventJournalService (optional, conditional on config)
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

  // SearchCoordinator setup
  const searchCoordinator = new SearchCoordinator();
  if (config.fullTextSearch) {
    for (const [mapName, ftsConfig] of Object.entries(config.fullTextSearch)) {
      searchCoordinator.enableSearch(mapName, ftsConfig);
    }
  }
  searchCoordinator.setNodeId(config.nodeId);

  const clusterSearchCoordinator = new ClusterSearchCoordinator(
    deps.cluster.cluster,
    deps.cluster.partitionService,
    searchCoordinator,
    config.distributedSearch,
    deps.core.metricsService
  );

  const distributedSubCoordinator = new DistributedSubscriptionCoordinator(
    deps.cluster.cluster,
    deps.storage.queryRegistry,
    searchCoordinator,
    undefined,
    deps.core.metricsService
  );

  // Shared state maps
  const pendingClusterQueries = new Map<string, any>();
  const pendingBatchOperations = new Set<Promise<void>>();
  const journalSubscriptions = new Map<string, any>();

  return {
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
  };
}

/**
 * Step 2: Create server handlers (independent handlers, no cross-handler deps)
 */
function createServerHandlers(
  config: HandlersModuleConfig,
  deps: HandlersModuleDeps,
  internal: HandlersModule['_internal']
): ServerHandlers {
  const heartbeatHandler = new HeartbeatHandler({
    connectionManager: internal.connectionManager,
  });

  const persistenceHandler = new PersistenceHandler({
    storage: config.storage || null,
    getMap: (name) => deps.storage.storageManager.getMap(name),
  });

  const operationContextHandler = new OperationContextHandler({
    connectionManager: internal.connectionManager,
    interceptors: config.interceptors || [],
    cluster: {
      config: { nodeId: config.nodeId },
      send: (nodeId, type, payload) => deps.cluster.cluster.send(nodeId, type, payload),
    },
  });

  // WriteConcernHandler depends on operationContextHandler and persistenceHandler
  // Created here since it's part of server handlers group
  const writeConcernHandler = new WriteConcernHandler({
    backpressure: {
      shouldForceSync: () => deps.core.backpressure.shouldForceSync(),
      registerPending: () => deps.core.backpressure.registerPending(),
      waitForCapacity: () => deps.core.backpressure.waitForCapacity(),
      completePending: () => deps.core.backpressure.completePending(),
      getPendingOps: () => deps.core.backpressure.getPendingOps(),
    },
    partitionService: {
      isLocalOwner: (key: string) => deps.cluster.partitionService.isLocalOwner(key),
      getOwner: (key: string) => deps.cluster.partitionService.getOwner(key),
    },
    cluster: {
      sendToNode: (nodeId: string, message: any) => deps.cluster.cluster.send(nodeId, message.type, message.payload),
    },
    metricsService: {
      incBackpressureSyncForced: () => deps.core.metricsService.incBackpressureSyncForced(),
      incBackpressureWaits: () => deps.core.metricsService.incBackpressureWaits(),
      incBackpressureTimeouts: () => deps.core.metricsService.incBackpressureTimeouts(),
      setBackpressurePendingOps: (count: number) => deps.core.metricsService.setBackpressurePendingOps(count),
    },
    writeAckManager: {
      notifyLevel: (opId: string, level: any) => deps.storage.writeAckManager.notifyLevel(opId, level),
      failPending: (opId: string, error: string) => deps.storage.writeAckManager.failPending(opId, error),
    },
    storage: config.storage || null,
    // broadcast callbacks will be set via late binding in ServerCoordinator
    broadcastBatch: () => {},
    broadcastBatchSync: async () => {},
    buildOpContext: operationContextHandler.buildOpContext.bind(operationContextHandler),
    runBeforeInterceptors: operationContextHandler.runBeforeInterceptors.bind(operationContextHandler),
    runAfterInterceptors: operationContextHandler.runAfterInterceptors.bind(operationContextHandler),
    applyOpToMap: undefined as any, // Set later after operationHandler is created
    persistOpSync: persistenceHandler.persistOpSync.bind(persistenceHandler),
    persistOpAsync: persistenceHandler.persistOpAsync.bind(persistenceHandler),
  });

  return {
    heartbeatHandler,
    persistenceHandler,
    operationContextHandler,
    writeConcernHandler,
  };
}

/**
 * Step 2b: Create messaging handlers (independent, need connectionManager)
 */
function createMessagingHandlers(
  config: HandlersModuleConfig,
  deps: HandlersModuleDeps,
  internal: HandlersModule['_internal']
): MessagingHandlers {
  const broadcastHandler = new BroadcastHandler({
    connectionManager: internal.connectionManager,
    securityManager: {
      filterObject: deps.core.securityManager.filterObject.bind(deps.core.securityManager),
    },
    queryRegistry: {
      getSubscribedClientIds: deps.storage.queryRegistry.getSubscribedClientIds.bind(deps.storage.queryRegistry),
    },
    metricsService: {
      incEventsRouted: deps.core.metricsService.incEventsRouted.bind(deps.core.metricsService),
      incEventsFilteredBySubscription: deps.core.metricsService.incEventsFilteredBySubscription.bind(deps.core.metricsService),
      recordSubscribersPerEvent: deps.core.metricsService.recordSubscribersPerEvent.bind(deps.core.metricsService),
    },
    hlc: deps.core.hlc,
  });

  const topicHandler = new TopicHandler({
    topicManager: {
      subscribe: (clientId, topic) => internal.topicManager.subscribe(clientId, topic),
      unsubscribe: (clientId, topic) => internal.topicManager.unsubscribe(clientId, topic),
      publish: (topic, data, senderId) => internal.topicManager.publish(topic, data, senderId),
    },
    securityManager: {
      checkPermission: deps.core.securityManager.checkPermission.bind(deps.core.securityManager),
    },
  });

  return { topicHandler, broadcastHandler };
}

/**
 * Step 3: Create CRDT handlers (depend on messaging and server handlers)
 */
function createCRDTHandlers(
  config: HandlersModuleConfig,
  deps: HandlersModuleDeps,
  internal: HandlersModule['_internal'],
  serverHandlers: ServerHandlers,
  messagingHandlers: MessagingHandlers
): CRDTHandlers {
  const operationHandler = new OperationHandler({
    nodeId: config.nodeId,
    hlc: deps.core.hlc,
    metricsService: deps.core.metricsService,
    securityManager: {
      checkPermission: deps.core.securityManager.checkPermission.bind(deps.core.securityManager),
    },
    storageManager: deps.storage.storageManager,
    conflictResolverHandler: {
      hasResolvers: (mapName) => internal.conflictResolverHandler.hasResolvers(mapName),
      mergeWithResolver: (map, mapName, key, record, nodeId) => internal.conflictResolverHandler.mergeWithResolver(map, mapName, key, record, nodeId),
    },
    queryRegistry: {
      processChange: deps.storage.queryRegistry.processChange.bind(deps.storage.queryRegistry),
    },
    eventJournalService: internal.eventJournalService ? {
      append: internal.eventJournalService.append.bind(internal.eventJournalService),
    } : undefined,
    merkleTreeManager: deps.cluster.merkleTreeManager ? {
      updateRecord: (pid, key, rec) => deps.cluster.merkleTreeManager!.updateRecord(pid, key, rec),
    } : undefined,
    partitionService: {
      getPartitionId: (key) => deps.cluster.partitionService.getPartitionId(key),
      getOwner: (key) => deps.cluster.partitionService.getOwner(key),
      isLocalOwner: (key) => deps.cluster.partitionService.isLocalOwner(key),
    },
    searchCoordinator: {
      isSearchEnabled: (mapName) => internal.searchCoordinator.isSearchEnabled(mapName),
      onDataChange: (mapName, key, value, changeType) => internal.searchCoordinator.onDataChange(mapName, key, value, changeType as any),
    },
    storage: config.storage || null,
    replicationPipeline: deps.cluster.replicationPipeline ? {
      replicate: async (op, opId, key) => {
        await deps.cluster.replicationPipeline!.replicate(op, opId, key);
      },
    } : undefined,
    broadcastHandler: messagingHandlers.broadcastHandler,
    operationContextHandler: serverHandlers.operationContextHandler,
    backpressure: {
      registerPending: () => deps.core.backpressure.registerPending(),
      waitForCapacity: () => deps.core.backpressure.waitForCapacity(),
      shouldForceSync: () => deps.core.backpressure.shouldForceSync(),
    },
  });

  // Now we can set applyOpToMap on writeConcernHandler
  (serverHandlers.writeConcernHandler as any).applyOpToMap = operationHandler.applyOpToMap.bind(operationHandler);

  const batchProcessingHandler = new BatchProcessingHandler({
    backpressure: {
      shouldForceSync: () => deps.core.backpressure.shouldForceSync(),
      registerPending: () => deps.core.backpressure.registerPending(),
      waitForCapacity: () => deps.core.backpressure.waitForCapacity(),
      completePending: () => deps.core.backpressure.completePending(),
      getPendingOps: () => deps.core.backpressure.getPendingOps(),
    },
    partitionService: {
      isLocalOwner: (key) => deps.cluster.partitionService.isLocalOwner(key),
      getOwner: (key) => deps.cluster.partitionService.getOwner(key),
    },
    cluster: {
      sendToNode: (nodeId, message) => deps.cluster.cluster.send(nodeId, message.type, message.payload),
    },
    metricsService: {
      incBackpressureSyncForced: () => deps.core.metricsService.incBackpressureSyncForced(),
      incBackpressureWaits: () => deps.core.metricsService.incBackpressureWaits(),
      incBackpressureTimeouts: () => deps.core.metricsService.incBackpressureTimeouts(),
      setBackpressurePendingOps: (count) => deps.core.metricsService.setBackpressurePendingOps(count),
    },
    replicationPipeline: deps.cluster.replicationPipeline ? {
      replicate: async (op, opId, key) => {
        await deps.cluster.replicationPipeline!.replicate(op, opId, key);
      },
    } : undefined,
    buildOpContext: serverHandlers.operationContextHandler.buildOpContext.bind(serverHandlers.operationContextHandler),
    runBeforeInterceptors: serverHandlers.operationContextHandler.runBeforeInterceptors.bind(serverHandlers.operationContextHandler),
    runAfterInterceptors: serverHandlers.operationContextHandler.runAfterInterceptors.bind(serverHandlers.operationContextHandler),
    applyOpToMap: operationHandler.applyOpToMap.bind(operationHandler),
  });

  const gcHandler = new GCHandler({
    storageManager: deps.storage.storageManager,
    connectionManager: internal.connectionManager,
    cluster: {
      getMembers: () => deps.cluster.cluster.getMembers(),
      send: (nodeId: string, type: any, payload: any) => deps.cluster.cluster.send(nodeId, type, payload),
      isLocal: (id: string) => deps.cluster.cluster.isLocal(id),
      config: { nodeId: config.nodeId },
    },
    partitionService: {
      isRelated: (key) => deps.cluster.partitionService.isRelated(key),
      getPartitionId: (key) => deps.cluster.partitionService.getPartitionId(key),
    },
    replicationPipeline: deps.cluster.replicationPipeline ? {
      replicate: async (op, opId, key) => {
        await deps.cluster.replicationPipeline!.replicate(op, opId, key);
      },
    } : undefined,
    merkleTreeManager: deps.cluster.merkleTreeManager ? {
      updateRecord: (pid, key, rec) => deps.cluster.merkleTreeManager!.updateRecord(pid, key, rec),
    } : undefined,
    queryRegistry: {
      processChange: deps.storage.queryRegistry.processChange.bind(deps.storage.queryRegistry),
    },
    hlc: deps.core.hlc,
    storage: config.storage || undefined,
    // broadcast callback will be set via late binding in ServerCoordinator
    metricsService: {
      incOp: (op: any, mapName: string) => deps.core.metricsService.incOp(op, mapName),
    },
  });

  return { operationHandler, batchProcessingHandler, gcHandler };
}

/**
 * Step 3b: Create sync handlers (depend on messaging handlers)
 */
function createSyncHandlers(
  config: HandlersModuleConfig,
  deps: HandlersModuleDeps,
  messagingHandlers: MessagingHandlers
): SyncHandlers {
  const lwwSyncHandler = new LwwSyncHandler({
    getMapAsync: (name: string, typeHint?: 'LWW' | 'OR') => deps.storage.storageManager.getMapAsync(name, typeHint),
    hlc: deps.core.hlc,
    securityManager: {
      checkPermission: (principal: any, resource: string, action: any) => deps.core.securityManager.checkPermission(principal, resource, action),
    },
    metricsService: {
      incOp: (op: any, mapName: string) => deps.core.metricsService.incOp(op, mapName),
    },
    gcAgeMs: DEFAULT_GC_AGE_MS,
  });

  const orMapSyncHandler = new ORMapSyncHandler({
    getMapAsync: (name: string, typeHint?: 'LWW' | 'OR') => deps.storage.storageManager.getMapAsync(name, typeHint),
    hlc: deps.core.hlc,
    securityManager: {
      checkPermission: (principal: any, resource: string, action: any) => deps.core.securityManager.checkPermission(principal, resource, action),
    },
    metricsService: {
      incOp: (op: any, mapName: string) => deps.core.metricsService.incOp(op, mapName),
    },
    storage: config.storage,
    broadcast: (message: any, excludeClientId?: string) => messagingHandlers.broadcastHandler.broadcast(message, excludeClientId),
    gcAgeMs: DEFAULT_GC_AGE_MS,
  });

  return { lwwSyncHandler, orMapSyncHandler };
}

/**
 * Step 3c: Create query handlers (depend on internal state)
 */
function createQueryHandlers(
  config: HandlersModuleConfig,
  deps: HandlersModuleDeps,
  internal: HandlersModule['_internal']
): QueryHandlers {
  const queryConversionHandler = new QueryConversionHandler({
    getMapAsync: (mapName: string, typeHint?: 'LWW' | 'OR') => deps.storage.storageManager.getMapAsync(mapName, typeHint),
    pendingClusterQueries: internal.pendingClusterQueries,
    queryRegistry: deps.storage.queryRegistry,
    securityManager: {
      filterObject: (value: any, principal: any, mapName: string) => deps.core.securityManager.filterObject(value, principal, mapName),
    },
  });

  const queryHandler = new QueryHandler({
    securityManager: {
      checkPermission: (principal: any, resource: string, action: any) => deps.core.securityManager.checkPermission(principal, resource, action),
      filterObject: (value: any, principal: any, mapName: string) => deps.core.securityManager.filterObject(value, principal, mapName),
    },
    metricsService: {
      incOp: (op: any, mapName: string) => deps.core.metricsService.incOp(op, mapName),
    },
    queryRegistry: {
      unregister: (queryId: string) => deps.storage.queryRegistry.unregister(queryId),
    },
    distributedSubCoordinator: internal.distributedSubCoordinator,
    cluster: {
      getMembers: () => deps.cluster.cluster.getMembers(),
      isLocal: (id: string) => deps.cluster.cluster.isLocal(id),
      send: (nodeId: string, type: any, payload: any) => deps.cluster.cluster.send(nodeId, type, payload),
      config: { nodeId: config.nodeId },
    },
    executeLocalQuery: (mapName: string, query: any) => queryConversionHandler.executeLocalQuery(mapName, query),
    finalizeClusterQuery: (reqId: string, timeout?: boolean) => queryConversionHandler.finalizeClusterQuery(reqId, timeout),
    pendingClusterQueries: internal.pendingClusterQueries,
    readReplicaHandler: deps.cluster.readReplicaHandler,
    ConsistencyLevel: { EVENTUAL: ConsistencyLevel.EVENTUAL },
  });

  return { queryHandler, queryConversionHandler };
}

/**
 * Step 3d: Create coordination handlers (independent)
 */
function createCoordinationHandlers(
  config: HandlersModuleConfig,
  deps: HandlersModuleDeps,
  internal: HandlersModule['_internal']
): CoordinationHandlers {
  const lockHandler = new LockHandler({
    lockManager: {
      acquire: (name, clientId, requestId, ttl) => deps.cluster.lockManager.acquire(name, clientId, requestId, ttl),
      release: (name, clientId, fencingToken) => deps.cluster.lockManager.release(name, clientId, fencingToken),
    },
    partitionService: {
      isLocalOwner: (key) => deps.cluster.partitionService.isLocalOwner(key),
      getOwner: (key) => deps.cluster.partitionService.getOwner(key),
    },
    cluster: {
      getMembers: () => deps.cluster.cluster.getMembers(),
      send: (nodeId, type, payload) => deps.cluster.cluster.send(nodeId, type, payload),
      config: { nodeId: config.nodeId },
    },
    securityManager: {
      checkPermission: deps.core.securityManager.checkPermission.bind(deps.core.securityManager),
    },
  });

  const partitionHandler = new PartitionHandler({
    partitionService: {
      getPartitionMap: () => deps.cluster.partitionService.getPartitionMap(),
    },
  });

  return { lockHandler, partitionHandler };
}

/**
 * Step 3e: Create search handlers (depend on search coordinators)
 */
function createSearchHandlers(
  config: HandlersModuleConfig,
  deps: HandlersModuleDeps,
  internal: HandlersModule['_internal']
): SearchHandlers {
  const searchHandler = new SearchHandler({
    searchCoordinator: {
      isSearchEnabled: (mapName) => internal.searchCoordinator.isSearchEnabled(mapName),
      search: (mapName, query, options) => internal.searchCoordinator.search(mapName, query, options),
      subscribe: (clientId, subscriptionId, mapName, query, options) => internal.searchCoordinator.subscribe(clientId, subscriptionId, mapName, query, options),
      unsubscribe: (subscriptionId) => internal.searchCoordinator.unsubscribe(subscriptionId),
    },
    clusterSearchCoordinator: internal.clusterSearchCoordinator,
    distributedSubCoordinator: internal.distributedSubCoordinator,
    cluster: {
      getMembers: () => deps.cluster.cluster.getMembers(),
    },
    securityManager: {
      checkPermission: deps.core.securityManager.checkPermission.bind(deps.core.securityManager),
    },
  });

  return { searchHandler };
}

/**
 * Step 3f: Create persistence handlers (depend on internal managers)
 */
function createPersistenceHandlers(
  config: HandlersModuleConfig,
  deps: HandlersModuleDeps,
  internal: HandlersModule['_internal']
): PersistenceHandlers {
  const journalHandler = new JournalHandler({
    eventJournalService: internal.eventJournalService,
    journalSubscriptions: internal.journalSubscriptions,
    getClient: (clientId) => internal.connectionManager.getClient(clientId),
  });

  const counterHandlerAdapter = new CounterHandlerAdapter({
    counterHandler: {
      handleCounterRequest: (clientId: string, name: string) => internal.counterHandler.handleCounterRequest(clientId, name),
      handleCounterSync: (clientId: string, name: string, state: any) => internal.counterHandler.handleCounterSync(clientId, name, state),
    },
    getClient: (clientId: string) => internal.connectionManager.getClient(clientId),
  });

  const resolverHandler = new ResolverHandler({
    conflictResolverHandler: {
      registerResolver: (mapName: string, resolver: any, clientId: string) => internal.conflictResolverHandler.registerResolver(mapName, resolver, clientId),
      unregisterResolver: (mapName: string, resolverName: string, clientId: string) => internal.conflictResolverHandler.unregisterResolver(mapName, resolverName, clientId),
      listResolvers: (mapName?: string) => internal.conflictResolverHandler.listResolvers(mapName),
    },
    securityManager: {
      checkPermission: (principal: any, resource: string, action: any) => deps.core.securityManager.checkPermission(principal, resource, action),
    },
  });

  const entryProcessorAdapter = new EntryProcessorAdapter({
    entryProcessorHandler: {
      executeOnKey: (map: any, key: string, processor: any) => internal.entryProcessorHandler.executeOnKey(map, key, processor),
      executeOnKeys: (map: any, keys: string[], processor: any) => internal.entryProcessorHandler.executeOnKeys(map, keys, processor),
    },
    getMap: (name: string) => deps.storage.storageManager.getMap(name),
    securityManager: {
      checkPermission: (principal: any, resource: string, action: any) => deps.core.securityManager.checkPermission(principal, resource, action),
    },
    queryRegistry: {
      processChange: (mapName: string, map: any, key: string, record: any, oldValue: any) => deps.storage.queryRegistry.processChange(mapName, map, key, record, oldValue),
    },
  });

  return {
    journalHandler,
    counterHandler: counterHandlerAdapter,
    entryProcessorHandler: entryProcessorAdapter,
    resolverHandler,
  };
}

/**
 * Step 4: Create client handlers (depend on many other handlers)
 */
function createClientHandlers(
  config: HandlersModuleConfig,
  deps: HandlersModuleDeps,
  internal: HandlersModule['_internal'],
  serverHandlers: ServerHandlers
): ClientHandlers {
  const authHandler = new AuthHandler({
    jwtSecret: config.jwtSecret,
    onAuthSuccess: (_clientId, _principal) => {
      if (config.rateLimitingEnabled ?? true) {
        deps.network.rateLimiter.onConnectionEstablished();
      }
    },
  });

  const clientMessageHandler = new ClientMessageHandler({
    connectionManager: internal.connectionManager,
    queryRegistry: deps.storage.queryRegistry,
    hlc: deps.core.hlc,
  });

  const webSocketHandler = new WebSocketHandler({
    nodeId: config.nodeId,
    rateLimitingEnabled: config.rateLimitingEnabled ?? true,
    writeCoalescingEnabled: config.writeCoalescingEnabled ?? true,
    writeCoalescingOptions: config.writeCoalescingOptions,
    interceptors: config.interceptors || [],
    rateLimiter: {
      shouldAccept: () => deps.network.rateLimiter.shouldAccept(),
      onConnectionAttempt: () => deps.network.rateLimiter.onConnectionAttempt(),
      onConnectionRejected: () => deps.network.rateLimiter.onConnectionRejected(),
      onPendingConnectionFailed: () => deps.network.rateLimiter.onPendingConnectionFailed(),
    },
    metricsService: {
      incConnectionsRejected: () => deps.core.metricsService.incConnectionsRejected(),
      incConnectionsAccepted: () => deps.core.metricsService.incConnectionsAccepted(),
      setConnectedClients: (count) => deps.core.metricsService.setConnectedClients(count),
    },
    connectionManager: internal.connectionManager,
    authHandler,
    rateLimitedLogger: deps.network.rateLimitedLogger,
    queryRegistry: {
      unregister: (subId) => deps.storage.queryRegistry.unregister(subId),
    },
    lockManager: {
      handleClientDisconnect: (clientId) => deps.cluster.lockManager.handleClientDisconnect(clientId),
    },
    topicManager: {
      unsubscribeAll: (clientId) => internal.topicManager.unsubscribeAll(clientId),
    },
    counterHandler: {
      unsubscribeAll: (clientId) => internal.counterHandler.unsubscribeAll(clientId),
    },
    searchCoordinator: {
      unsubscribeClient: (clientId) => internal.searchCoordinator.unsubscribeClient(clientId),
    },
    distributedSubCoordinator: internal.distributedSubCoordinator ? {
      unsubscribeClient: (socket) => internal.distributedSubCoordinator.unsubscribeClient(socket),
    } : undefined,
    cluster: {
      getMembers: () => deps.cluster.cluster.getMembers(),
      isLocal: (id) => deps.cluster.cluster.isLocal(id),
      send: (nodeId, type, payload) => deps.cluster.cluster.send(nodeId, type, payload),
      config: { nodeId: config.nodeId },
    },
    heartbeatHandler: serverHandlers.heartbeatHandler,
    clientMessageHandler,
  });

  return {
    authHandler,
    webSocketHandler,
    clientMessageHandler,
  };
}

/**
 * Main factory function - creates all handler groups and message registry
 */
export function createHandlersModule(
  config: HandlersModuleConfig,
  deps: HandlersModuleDeps
): HandlersModule {
  // Step 1: Create internal managers first (no handler dependencies)
  const internal = createInternalManagers(config, deps);

  // Step 2: Create independent handlers (server, messaging base)
  const server = createServerHandlers(config, deps, internal);
  const messaging = createMessagingHandlers(config, deps, internal);

  // Step 3: Create dependent handlers (need handlers from step 2)
  const crdt = createCRDTHandlers(config, deps, internal, server, messaging);
  const sync = createSyncHandlers(config, deps, messaging);
  const query = createQueryHandlers(config, deps, internal);
  const coordination = createCoordinationHandlers(config, deps, internal);
  const search = createSearchHandlers(config, deps, internal);
  const persistence = createPersistenceHandlers(config, deps, internal);

  // Step 4: Create client handlers (need many other handlers)
  const client = createClientHandlers(config, deps, internal, server);

  // Set queryRegistry cluster manager (required for distributed queries)
  deps.storage.queryRegistry.setClusterManager(deps.cluster.cluster, config.nodeId);
  deps.storage.queryRegistry.setMapGetter((name) => deps.storage.storageManager.getMap(name));

  // Create MessageRegistry with all routes (29 message types)
  const messageRegistry = createMessageRegistry({
    // CRDT operations
    onClientOp: (client, msg) => crdt.operationHandler.processClientOp(client, msg.payload),
    onOpBatch: async (client, msg) => {
      const ops = msg.payload.ops;
      await crdt.batchProcessingHandler.processBatchAsync(ops, client.id);

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
    onQuerySub: (client, msg) => query.queryHandler.handleQuerySub(client, msg),
    onQueryUnsub: (client, msg) => query.queryHandler.handleQueryUnsub(client, msg),
    // LWW Sync protocol
    onSyncInit: (client, msg) => sync.lwwSyncHandler.handleSyncInit(client, msg),
    onMerkleReqBucket: (client, msg) => sync.lwwSyncHandler.handleMerkleReqBucket(client, msg),
    // ORMap Sync protocol
    onORMapSyncInit: (client, msg) => sync.orMapSyncHandler.handleORMapSyncInit(client, msg),
    onORMapMerkleReqBucket: (client, msg) => sync.orMapSyncHandler.handleORMapMerkleReqBucket(client, msg),
    onORMapDiffRequest: (client, msg) => sync.orMapSyncHandler.handleORMapDiffRequest(client, msg),
    onORMapPushDiff: (client, msg) => sync.orMapSyncHandler.handleORMapPushDiff(client, msg),
    // Lock operations
    onLockRequest: (client, msg) => coordination.lockHandler.handleLockRequest(client, msg),
    onLockRelease: (client, msg) => coordination.lockHandler.handleLockRelease(client, msg),
    // Topic operations
    onTopicSub: (client, msg) => messaging.topicHandler.handleTopicSub(client, msg),
    onTopicUnsub: (client, msg) => messaging.topicHandler.handleTopicUnsub(client, msg),
    onTopicPub: (client, msg) => messaging.topicHandler.handleTopicPub(client, msg),
    // PN Counter operations
    onCounterRequest: (client, msg) => persistence.counterHandler.handleCounterRequest(client, msg),
    onCounterSync: (client, msg) => persistence.counterHandler.handleCounterSync(client, msg),
    // Entry processor operations
    onEntryProcess: (client, msg) => persistence.entryProcessorHandler.handleEntryProcess(client, msg),
    onEntryProcessBatch: (client, msg) => persistence.entryProcessorHandler.handleEntryProcessBatch(client, msg),
    // Conflict resolver operations
    onRegisterResolver: (client, msg) => persistence.resolverHandler.handleRegisterResolver(client, msg),
    onUnregisterResolver: (client, msg) => persistence.resolverHandler.handleUnregisterResolver(client, msg),
    onListResolvers: (client, msg) => persistence.resolverHandler.handleListResolvers(client, msg),
    // Partition operations
    onPartitionMapRequest: (client, msg) => coordination.partitionHandler.handlePartitionMapRequest(client, msg),
    // Full-text search operations
    onSearch: (client, msg) => search.searchHandler.handleSearch(client, msg),
    onSearchSub: (client, msg) => search.searchHandler.handleSearchSub(client, msg),
    onSearchUnsub: (client, msg) => search.searchHandler.handleSearchUnsub(client, msg),
    // Event journal operations
    onJournalSubscribe: (client, msg) => persistence.journalHandler.handleJournalSubscribe(client, msg),
    onJournalUnsubscribe: (client, msg) => persistence.journalHandler.handleJournalUnsubscribe(client, msg),
    onJournalRead: (client, msg) => persistence.journalHandler.handleJournalRead(client, msg),
  });

  return {
    crdt,
    sync,
    query,
    messaging,
    coordination,
    search,
    persistence,
    client,
    server,
    messageRegistry,
    _internal: internal,
  };
}

// Message types as const enum for Rust-like pattern (29 message types)
export const MessageType = {
  CLIENT_OP: 'CLIENT_OP',
  OP_BATCH: 'OP_BATCH',
  QUERY_SUB: 'QUERY_SUB',
  QUERY_UNSUB: 'QUERY_UNSUB',
  SYNC_INIT: 'SYNC_INIT',
  MERKLE_REQ_BUCKET: 'MERKLE_REQ_BUCKET',
  ORMAP_SYNC_INIT: 'ORMAP_SYNC_INIT',
  ORMAP_MERKLE_REQ_BUCKET: 'ORMAP_MERKLE_REQ_BUCKET',
  ORMAP_DIFF_REQUEST: 'ORMAP_DIFF_REQUEST',
  ORMAP_PUSH_DIFF: 'ORMAP_PUSH_DIFF',
  LOCK_REQUEST: 'LOCK_REQUEST',
  LOCK_RELEASE: 'LOCK_RELEASE',
  TOPIC_SUB: 'TOPIC_SUB',
  TOPIC_UNSUB: 'TOPIC_UNSUB',
  TOPIC_PUB: 'TOPIC_PUB',
  COUNTER_REQUEST: 'COUNTER_REQUEST',
  COUNTER_SYNC: 'COUNTER_SYNC',
  ENTRY_PROCESS: 'ENTRY_PROCESS',
  ENTRY_PROCESS_BATCH: 'ENTRY_PROCESS_BATCH',
  REGISTER_RESOLVER: 'REGISTER_RESOLVER',
  UNREGISTER_RESOLVER: 'UNREGISTER_RESOLVER',
  LIST_RESOLVERS: 'LIST_RESOLVERS',
  PARTITION_MAP_REQUEST: 'PARTITION_MAP_REQUEST',
  SEARCH: 'SEARCH',
  SEARCH_SUB: 'SEARCH_SUB',
  SEARCH_UNSUB: 'SEARCH_UNSUB',
  JOURNAL_SUBSCRIBE: 'JOURNAL_SUBSCRIBE',
  JOURNAL_UNSUBSCRIBE: 'JOURNAL_UNSUBSCRIBE',
  JOURNAL_READ: 'JOURNAL_READ',
} as const;

export type MessageTypeEnum = typeof MessageType[keyof typeof MessageType];

// Handler reference for documentation (maps to Rust Actor addresses)
interface HandlerRef {
  group: keyof Omit<HandlersModule, 'messageRegistry' | '_internal'>;
  handler: string;
  method: string;
}

// Route table (for documentation, not runtime) - 29 routes
export const MESSAGE_ROUTES: Record<MessageTypeEnum, HandlerRef> = {
  [MessageType.CLIENT_OP]: { group: 'crdt', handler: 'operationHandler', method: 'processClientOp' },
  [MessageType.OP_BATCH]: { group: 'crdt', handler: 'batchProcessingHandler', method: 'processBatchAsync' },
  [MessageType.QUERY_SUB]: { group: 'query', handler: 'queryHandler', method: 'handleQuerySub' },
  [MessageType.QUERY_UNSUB]: { group: 'query', handler: 'queryHandler', method: 'handleQueryUnsub' },
  [MessageType.SYNC_INIT]: { group: 'sync', handler: 'lwwSyncHandler', method: 'handleSyncInit' },
  [MessageType.MERKLE_REQ_BUCKET]: { group: 'sync', handler: 'lwwSyncHandler', method: 'handleMerkleReqBucket' },
  [MessageType.ORMAP_SYNC_INIT]: { group: 'sync', handler: 'orMapSyncHandler', method: 'handleORMapSyncInit' },
  [MessageType.ORMAP_MERKLE_REQ_BUCKET]: { group: 'sync', handler: 'orMapSyncHandler', method: 'handleORMapMerkleReqBucket' },
  [MessageType.ORMAP_DIFF_REQUEST]: { group: 'sync', handler: 'orMapSyncHandler', method: 'handleORMapDiffRequest' },
  [MessageType.ORMAP_PUSH_DIFF]: { group: 'sync', handler: 'orMapSyncHandler', method: 'handleORMapPushDiff' },
  [MessageType.LOCK_REQUEST]: { group: 'coordination', handler: 'lockHandler', method: 'handleLockRequest' },
  [MessageType.LOCK_RELEASE]: { group: 'coordination', handler: 'lockHandler', method: 'handleLockRelease' },
  [MessageType.TOPIC_SUB]: { group: 'messaging', handler: 'topicHandler', method: 'handleTopicSub' },
  [MessageType.TOPIC_UNSUB]: { group: 'messaging', handler: 'topicHandler', method: 'handleTopicUnsub' },
  [MessageType.TOPIC_PUB]: { group: 'messaging', handler: 'topicHandler', method: 'handleTopicPub' },
  [MessageType.COUNTER_REQUEST]: { group: 'persistence', handler: 'counterHandler', method: 'handleCounterRequest' },
  [MessageType.COUNTER_SYNC]: { group: 'persistence', handler: 'counterHandler', method: 'handleCounterSync' },
  [MessageType.ENTRY_PROCESS]: { group: 'persistence', handler: 'entryProcessorHandler', method: 'handleEntryProcess' },
  [MessageType.ENTRY_PROCESS_BATCH]: { group: 'persistence', handler: 'entryProcessorHandler', method: 'handleEntryProcessBatch' },
  [MessageType.REGISTER_RESOLVER]: { group: 'persistence', handler: 'resolverHandler', method: 'handleRegisterResolver' },
  [MessageType.UNREGISTER_RESOLVER]: { group: 'persistence', handler: 'resolverHandler', method: 'handleUnregisterResolver' },
  [MessageType.LIST_RESOLVERS]: { group: 'persistence', handler: 'resolverHandler', method: 'handleListResolvers' },
  [MessageType.PARTITION_MAP_REQUEST]: { group: 'coordination', handler: 'partitionHandler', method: 'handlePartitionMapRequest' },
  [MessageType.SEARCH]: { group: 'search', handler: 'searchHandler', method: 'handleSearch' },
  [MessageType.SEARCH_SUB]: { group: 'search', handler: 'searchHandler', method: 'handleSearchSub' },
  [MessageType.SEARCH_UNSUB]: { group: 'search', handler: 'searchHandler', method: 'handleSearchUnsub' },
  [MessageType.JOURNAL_SUBSCRIBE]: { group: 'persistence', handler: 'journalHandler', method: 'handleJournalSubscribe' },
  [MessageType.JOURNAL_UNSUBSCRIBE]: { group: 'persistence', handler: 'journalHandler', method: 'handleJournalUnsubscribe' },
  [MessageType.JOURNAL_READ]: { group: 'persistence', handler: 'journalHandler', method: 'handleJournalRead' },
};
