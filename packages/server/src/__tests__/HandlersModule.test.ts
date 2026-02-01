/**
 * HandlersModule test - verify handler count and structure
 *
 * This test validates that the HandlersModule contains exactly 23 handlers
 * across 9 public groups per specification. Note: The spec counts
 * 26 total handlers which includes 3 base handlers in _internal (counterHandler,
 * entryProcessorHandler, conflictResolverHandler) that are not part of the
 * 9 public groups.
 */

// Jest globals are available automatically
import { createHandlersModule, MessageType, MESSAGE_ROUTES } from '../modules/handlers-module';
import { createCoreModule, createClusterModule, createStorageModule, createNetworkModule } from '../modules';
import type { HandlersModule } from '../modules/types';

describe('HandlersModule', () => {
  let handlers: HandlersModule;
  let core: ReturnType<typeof createCoreModule>;
  let network: ReturnType<typeof createNetworkModule>;
  let cluster: ReturnType<typeof createClusterModule>;
  let storage: ReturnType<typeof createStorageModule>;

  beforeAll(() => {
    // Create minimal dependencies for handlers module
    core = createCoreModule({
      nodeId: 'test-node',
      eventStripeCount: 1,
      eventQueueCapacity: 100,
      backpressureEnabled: false,
    });

    network = createNetworkModule(
      {
        port: 10301,
        maxConnectionsPerSecond: 100,
        maxPendingConnections: 100,
      },
      {}
    );

    cluster = createClusterModule(
      {
        nodeId: 'test-node',
        clusterPort: 11301,
      },
      { hlc: core.hlc }
    );

    storage = createStorageModule(
      {
        nodeId: 'test-node',
      },
      {
        hlc: core.hlc,
        metricsService: core.metricsService,
        partitionService: cluster.partitionService,
      }
    );

    // Create handlers module
    handlers = createHandlersModule(
      {
        nodeId: 'test-node',
        jwtSecret: 'test-secret',
      },
      {
        core: {
          hlc: core.hlc,
          metricsService: core.metricsService,
          securityManager: core.securityManager,
          eventExecutor: core.eventExecutor,
          backpressure: core.backpressure,
        },
        network: {
          rateLimiter: network.rateLimiter,
          rateLimitedLogger: network.rateLimitedLogger,
        },
        cluster: {
          cluster: cluster.cluster,
          partitionService: cluster.partitionService,
          replicationPipeline: cluster.replicationPipeline,
          lockManager: cluster.lockManager,
          merkleTreeManager: cluster.merkleTreeManager,
          readReplicaHandler: cluster.readReplicaHandler,
        },
        storage: {
          storageManager: storage.storageManager,
          queryRegistry: storage.queryRegistry,
          writeAckManager: storage.writeAckManager,
        },
      }
    );
  });

  afterAll(async () => {
    // Clean up handlers
    if (handlers._internal.eventJournalService) {
      handlers._internal.eventJournalService.dispose();
    }
    if (handlers._internal.clusterSearchCoordinator) {
      handlers._internal.clusterSearchCoordinator.destroy();
    }
    if (handlers._internal.distributedSubCoordinator) {
      handlers._internal.distributedSubCoordinator.destroy();
    }

    // Clean up cluster (has timers)
    if (cluster.replicationPipeline) cluster.replicationPipeline.close();
    if (cluster.lockManager) cluster.lockManager.stop();
    if (cluster.repairScheduler) cluster.repairScheduler.stop();
    if (cluster.partitionReassigner) cluster.partitionReassigner.stop();
    if (cluster.cluster) cluster.cluster.stop();

    // Clean up network (has open ports)
    if (network.wss) network.wss.close();
    if (network.httpServer) network.httpServer.close();

    // Clean up core
    if (core.eventExecutor) await core.eventExecutor.shutdown(false);
    if (core.metricsService) core.metricsService.destroy();

    // Clean up storage
    if (storage.taskletScheduler) storage.taskletScheduler.shutdown();
    if (storage.writeAckManager) storage.writeAckManager.shutdown();
  });

  it('should contain exactly 23 handlers across 9 public groups', () => {
    // Verify 9 handler groups exist
    expect(handlers).toHaveProperty('crdt');
    expect(handlers).toHaveProperty('sync');
    expect(handlers).toHaveProperty('query');
    expect(handlers).toHaveProperty('messaging');
    expect(handlers).toHaveProperty('coordination');
    expect(handlers).toHaveProperty('search');
    expect(handlers).toHaveProperty('persistence');
    expect(handlers).toHaveProperty('client');
    expect(handlers).toHaveProperty('server');

    // Count handlers in each group
    const crdtCount = Object.keys(handlers.crdt).length;
    const syncCount = Object.keys(handlers.sync).length;
    const queryCount = Object.keys(handlers.query).length;
    const messagingCount = Object.keys(handlers.messaging).length;
    const coordinationCount = Object.keys(handlers.coordination).length;
    const searchCount = Object.keys(handlers.search).length;
    const persistenceCount = Object.keys(handlers.persistence).length;
    const clientCount = Object.keys(handlers.client).length;
    const serverCount = Object.keys(handlers.server).length;

    // Verify handler counts per group
    expect(crdtCount).toBe(3); // operationHandler, batchProcessingHandler, gcHandler
    expect(syncCount).toBe(2); // lwwSyncHandler, orMapSyncHandler
    expect(queryCount).toBe(2); // queryHandler, queryConversionHandler
    expect(messagingCount).toBe(2); // topicHandler, broadcastHandler
    expect(coordinationCount).toBe(2); // lockHandler, partitionHandler
    expect(searchCount).toBe(1); // searchHandler
    expect(persistenceCount).toBe(4); // journalHandler, counterHandler, entryProcessorHandler, resolverHandler
    expect(clientCount).toBe(3); // authHandler, webSocketHandler, clientMessageHandler
    expect(serverCount).toBe(4); // heartbeatHandler, persistenceHandler, operationContextHandler, writeConcernHandler

    // Verify total handler count is 23 (public groups only, excludes 3 base handlers in _internal)
    const totalHandlers = crdtCount + syncCount + queryCount + messagingCount +
                         coordinationCount + searchCount + persistenceCount +
                         clientCount + serverCount;
    expect(totalHandlers).toBe(23);
  });

  it('should contain messageRegistry with 29 message type routes', () => {
    expect(handlers).toHaveProperty('messageRegistry');

    // Count message type routes in registry
    const messageTypes = Object.keys(handlers.messageRegistry);
    expect(messageTypes.length).toBe(29);

    // Verify some key message types exist
    expect(messageTypes).toContain('CLIENT_OP');
    expect(messageTypes).toContain('OP_BATCH');
    expect(messageTypes).toContain('QUERY_SUB');
    expect(messageTypes).toContain('QUERY_UNSUB');
    expect(messageTypes).toContain('SYNC_INIT');
    expect(messageTypes).toContain('MERKLE_REQ_BUCKET');
    expect(messageTypes).toContain('ORMAP_SYNC_INIT');
    expect(messageTypes).toContain('LOCK_REQUEST');
    expect(messageTypes).toContain('TOPIC_SUB');
    expect(messageTypes).toContain('COUNTER_REQUEST');
    expect(messageTypes).toContain('ENTRY_PROCESS');
    expect(messageTypes).toContain('REGISTER_RESOLVER');
    expect(messageTypes).toContain('PARTITION_MAP_REQUEST');
    expect(messageTypes).toContain('SEARCH');
    expect(messageTypes).toContain('JOURNAL_SUBSCRIBE');
  });

  it('should have _internal with all required managers', () => {
    expect(handlers).toHaveProperty('_internal');

    const internal = handlers._internal;

    // Verify all internal managers exist
    expect(internal).toHaveProperty('topicManager');
    expect(internal).toHaveProperty('searchCoordinator');
    expect(internal).toHaveProperty('clusterSearchCoordinator');
    expect(internal).toHaveProperty('distributedSubCoordinator');
    expect(internal).toHaveProperty('connectionManager');
    expect(internal).toHaveProperty('counterHandler');
    expect(internal).toHaveProperty('entryProcessorHandler');
    expect(internal).toHaveProperty('conflictResolverHandler');
    expect(internal).toHaveProperty('pendingClusterQueries');
    expect(internal).toHaveProperty('pendingBatchOperations');
    expect(internal).toHaveProperty('journalSubscriptions');

    // eventJournalService is optional based on config
  });

  it('should export MessageType enum with 29 message types', () => {
    const messageTypeKeys = Object.keys(MessageType);
    expect(messageTypeKeys.length).toBe(29);

    // Verify some key message types
    expect(MessageType.CLIENT_OP).toBe('CLIENT_OP');
    expect(MessageType.OP_BATCH).toBe('OP_BATCH');
    expect(MessageType.QUERY_SUB).toBe('QUERY_SUB');
    expect(MessageType.LOCK_REQUEST).toBe('LOCK_REQUEST');
    expect(MessageType.TOPIC_PUB).toBe('TOPIC_PUB');
  });

  it('should export MESSAGE_ROUTES with 29 route mappings', () => {
    const routeKeys = Object.keys(MESSAGE_ROUTES);
    expect(routeKeys.length).toBe(29);

    // Verify route structure for a few message types
    expect(MESSAGE_ROUTES[MessageType.CLIENT_OP]).toEqual({
      group: 'crdt',
      handler: 'operationHandler',
      method: 'processClientOp'
    });

    expect(MESSAGE_ROUTES[MessageType.QUERY_SUB]).toEqual({
      group: 'query',
      handler: 'queryHandler',
      method: 'handleQuerySub'
    });

    expect(MESSAGE_ROUTES[MessageType.TOPIC_PUB]).toEqual({
      group: 'messaging',
      handler: 'topicHandler',
      method: 'handleTopicPub'
    });
  });
});
