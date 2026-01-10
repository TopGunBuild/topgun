/**
 * DistributedSubscriptionCoordinator Tests
 *
 * Tests for distributed live subscriptions across cluster nodes.
 * Phase 14.2 implementation.
 */

import { EventEmitter } from 'events';
import { DistributedSubscriptionCoordinator } from '../DistributedSubscriptionCoordinator';
import { SearchCoordinator } from '../../search/SearchCoordinator';
import { QueryRegistry } from '../../query/QueryRegistry';
import type { WebSocket } from 'ws';

// Mock ClusterManager
class MockClusterManager extends EventEmitter {
  config = { nodeId: 'node-1' };
  private members: string[] = ['node-1', 'node-2', 'node-3'];
  private sentMessages: Array<{ nodeId: string; type: string; payload: any }> = [];

  getMembers(): string[] {
    return this.members;
  }

  setMembers(members: string[]): void {
    this.members = members;
  }

  send(nodeId: string, type: string, payload: any): void {
    this.sentMessages.push({ nodeId, type, payload });
  }

  getSentMessages(): Array<{ nodeId: string; type: string; payload: any }> {
    return this.sentMessages;
  }

  clearSentMessages(): void {
    this.sentMessages = [];
  }

  // Simulate receiving a message from another node
  receiveMessage(senderId: string, type: string, payload: any): void {
    this.emit('message', { type, senderId, payload });
  }
}

// Mock WebSocket
function createMockSocket(id: string): WebSocket {
  const sent: any[] = [];
  return {
    readyState: 1, // WebSocket.OPEN
    send: jest.fn((data: string) => sent.push(JSON.parse(data))),
    _sent: sent,
    _id: id,
  } as any;
}

describe('DistributedSubscriptionCoordinator', () => {
  let coordinator: DistributedSubscriptionCoordinator;
  let clusterManager: MockClusterManager;
  let searchCoordinator: SearchCoordinator;
  let queryRegistry: QueryRegistry;

  beforeEach(() => {
    clusterManager = new MockClusterManager();
    searchCoordinator = new SearchCoordinator();
    queryRegistry = new QueryRegistry();

    // Enable search on a test map
    searchCoordinator.enableSearch('articles', { fields: ['title', 'body'] });

    // Add some test documents
    searchCoordinator.onDataChange('articles', 'doc1', {
      title: 'Machine Learning',
      body: 'Introduction to ML algorithms',
    }, 'add');
    searchCoordinator.onDataChange('articles', 'doc2', {
      title: 'Deep Learning',
      body: 'Neural networks and deep learning concepts',
    }, 'add');

    // Set node ID for SearchCoordinator
    searchCoordinator.setNodeId('node-1');

    coordinator = new DistributedSubscriptionCoordinator(
      clusterManager as any,
      queryRegistry,
      searchCoordinator
    );
  });

  afterEach(() => {
    coordinator.destroy();
    searchCoordinator.clear();
  });

  describe('subscribeSearch', () => {
    it('should register subscription and send CLUSTER_SUB_REGISTER to all nodes', async () => {
      const socket = createMockSocket('client-1');

      // Start subscription (will wait for ACKs)
      const subscribePromise = coordinator.subscribeSearch(
        'sub-1',
        socket,
        'articles',
        'machine learning',
        { limit: 10 }
      );

      // Simulate ACKs from remote nodes
      clusterManager.receiveMessage('node-2', 'CLUSTER_SUB_ACK', {
        subscriptionId: 'sub-1',
        nodeId: 'node-2',
        success: true,
        initialResults: [
          { key: 'remote-doc1', value: { title: 'Remote ML' }, score: 0.8 },
        ],
        totalHits: 1,
      });

      clusterManager.receiveMessage('node-3', 'CLUSTER_SUB_ACK', {
        subscriptionId: 'sub-1',
        nodeId: 'node-3',
        success: true,
        initialResults: [],
        totalHits: 0,
      });

      const result = await subscribePromise;

      // Verify CLUSTER_SUB_REGISTER was sent to remote nodes
      const registerMessages = clusterManager.getSentMessages().filter(
        m => m.type === 'CLUSTER_SUB_REGISTER'
      );
      expect(registerMessages.length).toBe(2); // node-2 and node-3

      // Verify subscription result
      expect(result.subscriptionId).toBe('sub-1');
      expect(result.registeredNodes).toContain('node-1');
      expect(result.registeredNodes).toContain('node-2');
      expect(result.registeredNodes).toContain('node-3');
    });

    it('should merge results from all nodes using RRF', async () => {
      const socket = createMockSocket('client-1');

      // Start subscription
      const subscribePromise = coordinator.subscribeSearch(
        'sub-2',
        socket,
        'articles',
        'learning',
        { limit: 10 }
      );

      // Simulate ACKs with results from different nodes
      clusterManager.receiveMessage('node-2', 'CLUSTER_SUB_ACK', {
        subscriptionId: 'sub-2',
        nodeId: 'node-2',
        success: true,
        initialResults: [
          { key: 'doc-a', value: { title: 'Learning A' }, score: 0.9 },
          { key: 'doc-b', value: { title: 'Learning B' }, score: 0.8 },
        ],
        totalHits: 2,
      });

      clusterManager.receiveMessage('node-3', 'CLUSTER_SUB_ACK', {
        subscriptionId: 'sub-2',
        nodeId: 'node-3',
        success: true,
        initialResults: [
          { key: 'doc-c', value: { title: 'Learning C' }, score: 0.7 },
        ],
        totalHits: 1,
      });

      const result = await subscribePromise;

      // Results should be merged
      expect(result.results.length).toBeGreaterThan(0);
      expect(result.totalHits).toBeGreaterThan(0);
    });

    it('should handle partial ACKs on timeout', async () => {
      // Use a short timeout config
      const fastCoordinator = new DistributedSubscriptionCoordinator(
        clusterManager as any,
        queryRegistry,
        searchCoordinator,
        { ackTimeoutMs: 100 } // Short timeout
      );

      const socket = createMockSocket('client-1');

      // Start subscription - only node-2 will respond
      const subscribePromise = fastCoordinator.subscribeSearch(
        'sub-3',
        socket,
        'articles',
        'test',
        { limit: 10 }
      );

      // Only one remote node responds
      clusterManager.receiveMessage('node-2', 'CLUSTER_SUB_ACK', {
        subscriptionId: 'sub-3',
        nodeId: 'node-2',
        success: true,
        initialResults: [],
        totalHits: 0,
      });

      // node-3 doesn't respond, wait for timeout
      const result = await subscribePromise;

      // Should still succeed with partial results
      expect(result.subscriptionId).toBe('sub-3');
      expect(result.registeredNodes).toContain('node-1');
      expect(result.registeredNodes).toContain('node-2');
      // node-3 should be in failed nodes
      expect(result.failedNodes).toContain('node-3');

      fastCoordinator.destroy();
    });
  });

  describe('unsubscribe', () => {
    it('should send CLUSTER_SUB_UNREGISTER to all registered nodes', async () => {
      const socket = createMockSocket('client-1');

      // First, subscribe
      const subscribePromise = coordinator.subscribeSearch(
        'sub-4',
        socket,
        'articles',
        'test',
        {}
      );

      // Simulate ACKs
      clusterManager.receiveMessage('node-2', 'CLUSTER_SUB_ACK', {
        subscriptionId: 'sub-4',
        nodeId: 'node-2',
        success: true,
        initialResults: [],
        totalHits: 0,
      });
      clusterManager.receiveMessage('node-3', 'CLUSTER_SUB_ACK', {
        subscriptionId: 'sub-4',
        nodeId: 'node-3',
        success: true,
        initialResults: [],
        totalHits: 0,
      });

      await subscribePromise;

      // Clear messages from subscribe
      clusterManager.clearSentMessages();

      // Now unsubscribe
      await coordinator.unsubscribe('sub-4');

      // Verify CLUSTER_SUB_UNREGISTER was sent
      const unregisterMessages = clusterManager.getSentMessages().filter(
        m => m.type === 'CLUSTER_SUB_UNREGISTER'
      );
      expect(unregisterMessages.length).toBe(2); // node-2 and node-3
    });
  });

  describe('handleSubUpdate', () => {
    it('should forward updates to client socket', async () => {
      const socket = createMockSocket('client-1');

      // Subscribe first
      const subscribePromise = coordinator.subscribeSearch(
        'sub-5',
        socket,
        'articles',
        'machine',
        {}
      );

      clusterManager.receiveMessage('node-2', 'CLUSTER_SUB_ACK', {
        subscriptionId: 'sub-5',
        nodeId: 'node-2',
        success: true,
        initialResults: [],
        totalHits: 0,
      });
      clusterManager.receiveMessage('node-3', 'CLUSTER_SUB_ACK', {
        subscriptionId: 'sub-5',
        nodeId: 'node-3',
        success: true,
        initialResults: [],
        totalHits: 0,
      });

      await subscribePromise;

      // Simulate update from remote node
      clusterManager.receiveMessage('node-2', 'CLUSTER_SUB_UPDATE', {
        subscriptionId: 'sub-5',
        sourceNodeId: 'node-2',
        key: 'new-doc',
        value: { title: 'New Machine Learning Doc' },
        score: 0.95,
        matchedTerms: ['machine'],
        changeType: 'ENTER',
        timestamp: Date.now(),
      });

      // Verify socket received the update
      const sent = (socket as any)._sent;
      expect(sent.length).toBeGreaterThan(0);
      const updateMsg = sent.find((m: any) => m.type === 'SEARCH_UPDATE');
      expect(updateMsg).toBeDefined();
      expect(updateMsg.payload.key).toBe('new-doc');
      expect(updateMsg.payload.type).toBe('ENTER');
    });
  });

  describe('handleSubRegister (as data node)', () => {
    it('should register local subscription when receiving CLUSTER_SUB_REGISTER', () => {
      // Simulate receiving registration request from another coordinator
      clusterManager.receiveMessage('node-2', 'CLUSTER_SUB_REGISTER', {
        subscriptionId: 'remote-sub-1',
        coordinatorNodeId: 'node-2',
        mapName: 'articles',
        type: 'SEARCH',
        searchQuery: 'learning',
        searchOptions: { limit: 10 },
      });

      // Verify ACK was sent back
      const ackMessages = clusterManager.getSentMessages().filter(
        m => m.type === 'CLUSTER_SUB_ACK'
      );
      expect(ackMessages.length).toBe(1);
      expect(ackMessages[0].nodeId).toBe('node-2');
      expect(ackMessages[0].payload.subscriptionId).toBe('remote-sub-1');
      expect(ackMessages[0].payload.success).toBe(true);
    });
  });

  describe('unsubscribeClient', () => {
    it('should unsubscribe all subscriptions for a client', async () => {
      const socket = createMockSocket('client-1');

      // Create multiple subscriptions
      const sub1Promise = coordinator.subscribeSearch('sub-a', socket, 'articles', 'test1', {});
      const sub2Promise = coordinator.subscribeSearch('sub-b', socket, 'articles', 'test2', {});

      // ACK both
      for (const subId of ['sub-a', 'sub-b']) {
        clusterManager.receiveMessage('node-2', 'CLUSTER_SUB_ACK', {
          subscriptionId: subId,
          nodeId: 'node-2',
          success: true,
          initialResults: [],
          totalHits: 0,
        });
        clusterManager.receiveMessage('node-3', 'CLUSTER_SUB_ACK', {
          subscriptionId: subId,
          nodeId: 'node-3',
          success: true,
          initialResults: [],
          totalHits: 0,
        });
      }

      await Promise.all([sub1Promise, sub2Promise]);

      expect(coordinator.getActiveSubscriptionCount()).toBe(2);

      // Unsubscribe client
      coordinator.unsubscribeClient(socket);

      expect(coordinator.getActiveSubscriptionCount()).toBe(0);
    });
  });

  describe('metrics integration', () => {
    it('should record metrics on successful subscription', async () => {
      // Create a mock metrics service
      const mockMetrics = {
        incDistributedSub: jest.fn(),
        recordDistributedSubRegistration: jest.fn(),
        recordDistributedSubInitialResultsCount: jest.fn(),
        setDistributedSubPendingAcks: jest.fn(),
        incDistributedSubAck: jest.fn(),
        incDistributedSubUnsubscribe: jest.fn(),
        decDistributedSubActive: jest.fn(),
        incDistributedSubUpdates: jest.fn(),
        recordDistributedSubUpdateLatency: jest.fn(),
      };

      const coordinatorWithMetrics = new DistributedSubscriptionCoordinator(
        clusterManager as any,
        queryRegistry,
        searchCoordinator,
        undefined,
        mockMetrics as any
      );

      const socket = createMockSocket('client-1');

      // Start subscription
      const subscribePromise = coordinatorWithMetrics.subscribeSearch(
        'sub-metrics-1',
        socket,
        'articles',
        'test',
        {}
      );

      // Simulate ACKs
      clusterManager.receiveMessage('node-2', 'CLUSTER_SUB_ACK', {
        subscriptionId: 'sub-metrics-1',
        nodeId: 'node-2',
        success: true,
        initialResults: [{ key: 'doc1', value: { title: 'Test' }, score: 0.9 }],
        totalHits: 1,
      });
      clusterManager.receiveMessage('node-3', 'CLUSTER_SUB_ACK', {
        subscriptionId: 'sub-metrics-1',
        nodeId: 'node-3',
        success: true,
        initialResults: [],
        totalHits: 0,
      });

      await subscribePromise;

      // Verify metrics were recorded
      expect(mockMetrics.incDistributedSub).toHaveBeenCalledWith('SEARCH', 'success');
      expect(mockMetrics.recordDistributedSubRegistration).toHaveBeenCalledWith('SEARCH', expect.any(Number));
      expect(mockMetrics.recordDistributedSubInitialResultsCount).toHaveBeenCalledWith('SEARCH', expect.any(Number));
      expect(mockMetrics.setDistributedSubPendingAcks).toHaveBeenCalled();
      expect(mockMetrics.incDistributedSubAck).toHaveBeenCalledWith('success', expect.any(Number));

      coordinatorWithMetrics.destroy();
    });

    it('should record metrics on unsubscribe', async () => {
      const mockMetrics = {
        incDistributedSub: jest.fn(),
        recordDistributedSubRegistration: jest.fn(),
        recordDistributedSubInitialResultsCount: jest.fn(),
        setDistributedSubPendingAcks: jest.fn(),
        incDistributedSubAck: jest.fn(),
        incDistributedSubUnsubscribe: jest.fn(),
        decDistributedSubActive: jest.fn(),
        incDistributedSubUpdates: jest.fn(),
        recordDistributedSubUpdateLatency: jest.fn(),
      };

      const coordinatorWithMetrics = new DistributedSubscriptionCoordinator(
        clusterManager as any,
        queryRegistry,
        searchCoordinator,
        undefined,
        mockMetrics as any
      );

      const socket = createMockSocket('client-1');

      // First subscribe
      const subscribePromise = coordinatorWithMetrics.subscribeSearch(
        'sub-metrics-2',
        socket,
        'articles',
        'test',
        {}
      );

      // Simulate ACKs
      clusterManager.receiveMessage('node-2', 'CLUSTER_SUB_ACK', {
        subscriptionId: 'sub-metrics-2',
        nodeId: 'node-2',
        success: true,
        initialResults: [],
        totalHits: 0,
      });
      clusterManager.receiveMessage('node-3', 'CLUSTER_SUB_ACK', {
        subscriptionId: 'sub-metrics-2',
        nodeId: 'node-3',
        success: true,
        initialResults: [],
        totalHits: 0,
      });

      await subscribePromise;

      // Clear mocks to verify unsubscribe metrics
      mockMetrics.incDistributedSubUnsubscribe.mockClear();
      mockMetrics.decDistributedSubActive.mockClear();

      // Unsubscribe
      await coordinatorWithMetrics.unsubscribe('sub-metrics-2');

      expect(mockMetrics.incDistributedSubUnsubscribe).toHaveBeenCalledWith('SEARCH');
      expect(mockMetrics.decDistributedSubActive).toHaveBeenCalledWith('SEARCH');

      coordinatorWithMetrics.destroy();
    });

    it('should record update metrics when receiving updates', async () => {
      const mockMetrics = {
        incDistributedSub: jest.fn(),
        recordDistributedSubRegistration: jest.fn(),
        recordDistributedSubInitialResultsCount: jest.fn(),
        setDistributedSubPendingAcks: jest.fn(),
        incDistributedSubAck: jest.fn(),
        incDistributedSubUnsubscribe: jest.fn(),
        decDistributedSubActive: jest.fn(),
        incDistributedSubUpdates: jest.fn(),
        recordDistributedSubUpdateLatency: jest.fn(),
      };

      const coordinatorWithMetrics = new DistributedSubscriptionCoordinator(
        clusterManager as any,
        queryRegistry,
        searchCoordinator,
        undefined,
        mockMetrics as any
      );

      const socket = createMockSocket('client-1');

      // Subscribe first
      const subscribePromise = coordinatorWithMetrics.subscribeSearch(
        'sub-metrics-3',
        socket,
        'articles',
        'test',
        {}
      );

      clusterManager.receiveMessage('node-2', 'CLUSTER_SUB_ACK', {
        subscriptionId: 'sub-metrics-3',
        nodeId: 'node-2',
        success: true,
        initialResults: [],
        totalHits: 0,
      });
      clusterManager.receiveMessage('node-3', 'CLUSTER_SUB_ACK', {
        subscriptionId: 'sub-metrics-3',
        nodeId: 'node-3',
        success: true,
        initialResults: [],
        totalHits: 0,
      });

      await subscribePromise;

      // Clear mocks
      mockMetrics.incDistributedSubUpdates.mockClear();
      mockMetrics.recordDistributedSubUpdateLatency.mockClear();

      // Simulate update from remote node
      const timestamp = Date.now() - 10; // 10ms ago
      clusterManager.receiveMessage('node-2', 'CLUSTER_SUB_UPDATE', {
        subscriptionId: 'sub-metrics-3',
        sourceNodeId: 'node-2',
        key: 'new-doc',
        value: { title: 'New Doc' },
        score: 0.95,
        changeType: 'ENTER',
        timestamp,
      });

      expect(mockMetrics.incDistributedSubUpdates).toHaveBeenCalledWith('received', 'ENTER');
      expect(mockMetrics.recordDistributedSubUpdateLatency).toHaveBeenCalledWith('SEARCH', expect.any(Number));

      coordinatorWithMetrics.destroy();
    });

    it('should record timeout metrics when ACKs timeout', async () => {
      const mockMetrics = {
        incDistributedSub: jest.fn(),
        recordDistributedSubRegistration: jest.fn(),
        recordDistributedSubInitialResultsCount: jest.fn(),
        setDistributedSubPendingAcks: jest.fn(),
        incDistributedSubAck: jest.fn(),
        incDistributedSubUnsubscribe: jest.fn(),
        decDistributedSubActive: jest.fn(),
        incDistributedSubUpdates: jest.fn(),
        recordDistributedSubUpdateLatency: jest.fn(),
      };

      const fastCoordinator = new DistributedSubscriptionCoordinator(
        clusterManager as any,
        queryRegistry,
        searchCoordinator,
        { ackTimeoutMs: 100 },
        mockMetrics as any
      );

      const socket = createMockSocket('client-1');

      // Start subscription - only one node responds
      const subscribePromise = fastCoordinator.subscribeSearch(
        'sub-timeout',
        socket,
        'articles',
        'test',
        {}
      );

      // Only node-2 responds
      clusterManager.receiveMessage('node-2', 'CLUSTER_SUB_ACK', {
        subscriptionId: 'sub-timeout',
        nodeId: 'node-2',
        success: true,
        initialResults: [],
        totalHits: 0,
      });

      // Wait for timeout
      await subscribePromise;

      // Should record timeout status
      expect(mockMetrics.incDistributedSub).toHaveBeenCalledWith('SEARCH', 'timeout');
      // Should record timeout ACKs for failed nodes
      expect(mockMetrics.incDistributedSubAck).toHaveBeenCalledWith('timeout', expect.any(Number));

      fastCoordinator.destroy();
    });
  });

  // ===========================================
  // Phase 14.2.6: Query Distributed Subscription Tests
  // ===========================================

  describe('subscribeQuery', () => {
    it('should register query subscription and send CLUSTER_SUB_REGISTER to all nodes', async () => {
      const socket = createMockSocket('client-1');

      const subscribePromise = coordinator.subscribeQuery(
        'query-sub-1',
        socket,
        'users',
        { where: { age: { $gte: 18 } } }
      );

      // Simulate ACKs from remote nodes
      clusterManager.receiveMessage('node-2', 'CLUSTER_SUB_ACK', {
        subscriptionId: 'query-sub-1',
        nodeId: 'node-2',
        success: true,
        initialResults: [
          { key: 'user-1', value: { name: 'Alice', age: 25 } },
        ],
        totalHits: 1,
      });

      clusterManager.receiveMessage('node-3', 'CLUSTER_SUB_ACK', {
        subscriptionId: 'query-sub-1',
        nodeId: 'node-3',
        success: true,
        initialResults: [
          { key: 'user-2', value: { name: 'Bob', age: 30 } },
        ],
        totalHits: 1,
      });

      const result = await subscribePromise;

      // Verify CLUSTER_SUB_REGISTER was sent with type: 'QUERY'
      const registerMessages = clusterManager.getSentMessages().filter(
        m => m.type === 'CLUSTER_SUB_REGISTER'
      );
      expect(registerMessages.length).toBe(2);
      expect(registerMessages[0].payload.type).toBe('QUERY');
      expect(registerMessages[0].payload.queryPredicate).toBeDefined();

      // Verify results merged from all nodes
      expect(result.subscriptionId).toBe('query-sub-1');
      expect(result.results.length).toBeGreaterThanOrEqual(2);
    });

    it('should merge query results from all nodes without RRF scoring', async () => {
      const socket = createMockSocket('client-1');

      const subscribePromise = coordinator.subscribeQuery(
        'query-sub-2',
        socket,
        'products',
        { where: { inStock: true }, sort: { price: 'asc' } }
      );

      // Node-2 results
      clusterManager.receiveMessage('node-2', 'CLUSTER_SUB_ACK', {
        subscriptionId: 'query-sub-2',
        nodeId: 'node-2',
        success: true,
        initialResults: [
          { key: 'prod-a', value: { name: 'Widget', price: 10, inStock: true } },
          { key: 'prod-b', value: { name: 'Gadget', price: 20, inStock: true } },
        ],
        totalHits: 2,
      });

      // Node-3 results
      clusterManager.receiveMessage('node-3', 'CLUSTER_SUB_ACK', {
        subscriptionId: 'query-sub-2',
        nodeId: 'node-3',
        success: true,
        initialResults: [
          { key: 'prod-c', value: { name: 'Doodad', price: 15, inStock: true } },
        ],
        totalHits: 1,
      });

      const result = await subscribePromise;

      // Results should be merged (all 3 products)
      expect(result.results.length).toBe(3);
      expect(result.totalHits).toBe(3);
    });

    it('should handle query with complex predicates', async () => {
      const socket = createMockSocket('client-1');

      const complexQuery = {
        where: {
          $and: [
            { status: 'active' },
            { $or: [{ priority: 'high' }, { dueDate: { $lt: '2024-01-01' } }] }
          ]
        },
        sort: { createdAt: 'desc' as const },
        limit: 50
      };

      const subscribePromise = coordinator.subscribeQuery(
        'query-sub-3',
        socket,
        'tasks',
        complexQuery
      );

      clusterManager.receiveMessage('node-2', 'CLUSTER_SUB_ACK', {
        subscriptionId: 'query-sub-3',
        nodeId: 'node-2',
        success: true,
        initialResults: [],
        totalHits: 0,
      });

      clusterManager.receiveMessage('node-3', 'CLUSTER_SUB_ACK', {
        subscriptionId: 'query-sub-3',
        nodeId: 'node-3',
        success: true,
        initialResults: [],
        totalHits: 0,
      });

      const result = await subscribePromise;
      expect(result.subscriptionId).toBe('query-sub-3');
    });
  });

  describe('handleSubUpdate for Query subscriptions', () => {
    it('should forward ENTER updates to client when new record matches query', async () => {
      const socket = createMockSocket('client-1');

      // Setup subscription
      const subscribePromise = coordinator.subscribeQuery(
        'query-sub-4',
        socket,
        'orders',
        { where: { status: 'pending' } }
      );

      // ACK subscription
      clusterManager.receiveMessage('node-2', 'CLUSTER_SUB_ACK', {
        subscriptionId: 'query-sub-4',
        nodeId: 'node-2',
        success: true,
        initialResults: [],
        totalHits: 0,
      });
      clusterManager.receiveMessage('node-3', 'CLUSTER_SUB_ACK', {
        subscriptionId: 'query-sub-4',
        nodeId: 'node-3',
        success: true,
        initialResults: [],
        totalHits: 0,
      });

      await subscribePromise;

      // Clear previous socket sends
      (socket as any)._sent.length = 0;

      // Simulate ENTER update from remote node
      clusterManager.receiveMessage('node-2', 'CLUSTER_SUB_UPDATE', {
        subscriptionId: 'query-sub-4',
        sourceNodeId: 'node-2',
        key: 'order-123',
        value: { customerId: 'cust-1', status: 'pending', total: 99.99 },
        changeType: 'ENTER',
        timestamp: Date.now(),
      });

      // Verify client received update
      const sent = (socket as any)._sent;
      const updateMsg = sent.find((m: any) => m.type === 'QUERY_UPDATE');
      expect(updateMsg).toBeDefined();
      expect(updateMsg.payload.key).toBe('order-123');
      expect(updateMsg.payload.type).toBe('ENTER');
    });

    it('should forward LEAVE updates when record no longer matches query', async () => {
      const socket = createMockSocket('client-1');

      const subscribePromise = coordinator.subscribeQuery(
        'query-sub-5',
        socket,
        'orders',
        { where: { status: 'pending' } }
      );

      // ACK with initial result
      clusterManager.receiveMessage('node-2', 'CLUSTER_SUB_ACK', {
        subscriptionId: 'query-sub-5',
        nodeId: 'node-2',
        success: true,
        initialResults: [
          { key: 'order-456', value: { status: 'pending' } }
        ],
        totalHits: 1,
      });
      clusterManager.receiveMessage('node-3', 'CLUSTER_SUB_ACK', {
        subscriptionId: 'query-sub-5',
        nodeId: 'node-3',
        success: true,
        initialResults: [],
        totalHits: 0,
      });

      await subscribePromise;
      (socket as any)._sent.length = 0;

      // Record status changed to 'completed' - should trigger LEAVE
      clusterManager.receiveMessage('node-2', 'CLUSTER_SUB_UPDATE', {
        subscriptionId: 'query-sub-5',
        sourceNodeId: 'node-2',
        key: 'order-456',
        value: { status: 'completed' },
        changeType: 'LEAVE',
        timestamp: Date.now(),
      });

      const sent = (socket as any)._sent;
      const leaveMsg = sent.find((m: any) =>
        m.type === 'QUERY_UPDATE' && m.payload.type === 'LEAVE'
      );
      expect(leaveMsg).toBeDefined();
      expect(leaveMsg.payload.key).toBe('order-456');
    });

    it('should forward UPDATE when matched record is modified', async () => {
      const socket = createMockSocket('client-1');

      const subscribePromise = coordinator.subscribeQuery(
        'query-sub-6',
        socket,
        'users',
        { where: { role: 'admin' } }
      );

      clusterManager.receiveMessage('node-2', 'CLUSTER_SUB_ACK', {
        subscriptionId: 'query-sub-6',
        nodeId: 'node-2',
        success: true,
        initialResults: [
          { key: 'user-admin', value: { name: 'Admin', role: 'admin', email: 'old@test.com' } }
        ],
        totalHits: 1,
      });
      clusterManager.receiveMessage('node-3', 'CLUSTER_SUB_ACK', {
        subscriptionId: 'query-sub-6',
        nodeId: 'node-3',
        success: true,
        initialResults: [],
        totalHits: 0,
      });

      await subscribePromise;
      (socket as any)._sent.length = 0;

      // Admin email updated (still matches query)
      clusterManager.receiveMessage('node-2', 'CLUSTER_SUB_UPDATE', {
        subscriptionId: 'query-sub-6',
        sourceNodeId: 'node-2',
        key: 'user-admin',
        value: { name: 'Admin', role: 'admin', email: 'new@test.com' },
        changeType: 'UPDATE',
        timestamp: Date.now(),
      });

      const sent = (socket as any)._sent;
      const updateMsg = sent.find((m: any) =>
        m.type === 'QUERY_UPDATE' && m.payload.type === 'UPDATE'
      );
      expect(updateMsg).toBeDefined();
      expect(updateMsg.payload.value.email).toBe('new@test.com');
    });
  });

  describe('handleSubRegister for Query subscriptions (as data node)', () => {
    it('should register local query subscription when receiving CLUSTER_SUB_REGISTER', () => {
      // Simulate receiving registration request from another coordinator
      clusterManager.receiveMessage('node-2', 'CLUSTER_SUB_REGISTER', {
        subscriptionId: 'remote-query-sub-1',
        coordinatorNodeId: 'node-2',
        mapName: 'items',
        type: 'QUERY',
        queryPredicate: { where: { status: 'active' } },
      });

      // Verify ACK was sent back
      const ackMessages = clusterManager.getSentMessages().filter(
        m => m.type === 'CLUSTER_SUB_ACK'
      );
      expect(ackMessages.length).toBe(1);
      expect(ackMessages[0].nodeId).toBe('node-2');
      expect(ackMessages[0].payload.subscriptionId).toBe('remote-query-sub-1');
      expect(ackMessages[0].payload.success).toBe(true);
    });
  });

  describe('Query subscription edge cases', () => {
    it('should handle empty result sets from all nodes', async () => {
      const socket = createMockSocket('client-1');

      const subscribePromise = coordinator.subscribeQuery(
        'query-empty',
        socket,
        'nonexistent',
        { where: { impossible: true } }
      );

      clusterManager.receiveMessage('node-2', 'CLUSTER_SUB_ACK', {
        subscriptionId: 'query-empty',
        nodeId: 'node-2',
        success: true,
        initialResults: [],
        totalHits: 0,
      });
      clusterManager.receiveMessage('node-3', 'CLUSTER_SUB_ACK', {
        subscriptionId: 'query-empty',
        nodeId: 'node-3',
        success: true,
        initialResults: [],
        totalHits: 0,
      });

      const result = await subscribePromise;
      expect(result.results).toEqual([]);
      expect(result.totalHits).toBe(0);
    });

    it('should deduplicate results with same key from different nodes', async () => {
      const socket = createMockSocket('client-1');

      const subscribePromise = coordinator.subscribeQuery(
        'query-dedup',
        socket,
        'shared',
        { where: { replicated: true } }
      );

      // Same record exists on both nodes (replicated data)
      clusterManager.receiveMessage('node-2', 'CLUSTER_SUB_ACK', {
        subscriptionId: 'query-dedup',
        nodeId: 'node-2',
        success: true,
        initialResults: [
          { key: 'shared-rec', value: { data: 'from-node-2' } }
        ],
        totalHits: 1,
      });
      clusterManager.receiveMessage('node-3', 'CLUSTER_SUB_ACK', {
        subscriptionId: 'query-dedup',
        nodeId: 'node-3',
        success: true,
        initialResults: [
          { key: 'shared-rec', value: { data: 'from-node-3' } }
        ],
        totalHits: 1,
      });

      const result = await subscribePromise;

      // Should only have one result with that key
      const sharedRecs = result.results.filter(r => r.key === 'shared-rec');
      expect(sharedRecs.length).toBe(1);
    });

    it('should handle node failure during query subscription', async () => {
      const fastCoordinator = new DistributedSubscriptionCoordinator(
        clusterManager as any,
        queryRegistry,
        searchCoordinator,
        { ackTimeoutMs: 100 }
      );

      const socket = createMockSocket('client-1');

      const subscribePromise = fastCoordinator.subscribeQuery(
        'query-partial',
        socket,
        'data',
        { where: { active: true } }
      );

      // Only node-2 responds
      clusterManager.receiveMessage('node-2', 'CLUSTER_SUB_ACK', {
        subscriptionId: 'query-partial',
        nodeId: 'node-2',
        success: true,
        initialResults: [{ key: 'rec-1', value: { active: true } }],
        totalHits: 1,
      });

      const result = await subscribePromise;

      expect(result.registeredNodes).toContain('node-2');
      expect(result.failedNodes).toContain('node-3');
      expect(result.results.length).toBeGreaterThan(0);

      fastCoordinator.destroy();
    });

    it('should handle unsubscribe for query subscriptions', async () => {
      const socket = createMockSocket('client-1');

      // Subscribe to a query
      const subscribePromise = coordinator.subscribeQuery(
        'query-unsub-test',
        socket,
        'items',
        { where: { active: true } }
      );

      // ACK from remote nodes
      clusterManager.receiveMessage('node-2', 'CLUSTER_SUB_ACK', {
        subscriptionId: 'query-unsub-test',
        nodeId: 'node-2',
        success: true,
        initialResults: [],
        totalHits: 0,
      });
      clusterManager.receiveMessage('node-3', 'CLUSTER_SUB_ACK', {
        subscriptionId: 'query-unsub-test',
        nodeId: 'node-3',
        success: true,
        initialResults: [],
        totalHits: 0,
      });

      await subscribePromise;

      expect(coordinator.getActiveSubscriptionCount()).toBe(1);

      // Clear previous messages
      clusterManager.clearSentMessages();

      // Unsubscribe
      await coordinator.unsubscribe('query-unsub-test');

      // Verify CLUSTER_SUB_UNREGISTER was sent
      const unregisterMessages = clusterManager.getSentMessages().filter(
        m => m.type === 'CLUSTER_SUB_UNREGISTER'
      );
      expect(unregisterMessages.length).toBe(2); // node-2 and node-3

      expect(coordinator.getActiveSubscriptionCount()).toBe(0);
    });

    it('should record metrics for query subscriptions', async () => {
      const mockMetrics = {
        incDistributedSub: jest.fn(),
        recordDistributedSubRegistration: jest.fn(),
        recordDistributedSubInitialResultsCount: jest.fn(),
        setDistributedSubPendingAcks: jest.fn(),
        incDistributedSubAck: jest.fn(),
        incDistributedSubUnsubscribe: jest.fn(),
        decDistributedSubActive: jest.fn(),
        incDistributedSubUpdates: jest.fn(),
        recordDistributedSubUpdateLatency: jest.fn(),
      };

      const coordinatorWithMetrics = new DistributedSubscriptionCoordinator(
        clusterManager as any,
        queryRegistry,
        searchCoordinator,
        undefined,
        mockMetrics as any
      );

      const socket = createMockSocket('client-1');

      const subscribePromise = coordinatorWithMetrics.subscribeQuery(
        'query-metrics',
        socket,
        'items',
        { where: { active: true } }
      );

      clusterManager.receiveMessage('node-2', 'CLUSTER_SUB_ACK', {
        subscriptionId: 'query-metrics',
        nodeId: 'node-2',
        success: true,
        initialResults: [{ key: 'item-1', value: { active: true } }],
        totalHits: 1,
      });
      clusterManager.receiveMessage('node-3', 'CLUSTER_SUB_ACK', {
        subscriptionId: 'query-metrics',
        nodeId: 'node-3',
        success: true,
        initialResults: [],
        totalHits: 0,
      });

      await subscribePromise;

      // Verify metrics were recorded with QUERY type
      expect(mockMetrics.incDistributedSub).toHaveBeenCalledWith('QUERY', 'success');
      expect(mockMetrics.recordDistributedSubRegistration).toHaveBeenCalledWith('QUERY', expect.any(Number));
      expect(mockMetrics.recordDistributedSubInitialResultsCount).toHaveBeenCalledWith('QUERY', expect.any(Number));

      coordinatorWithMetrics.destroy();
    });

    it('should record update latency metrics for query updates', async () => {
      const mockMetrics = {
        incDistributedSub: jest.fn(),
        recordDistributedSubRegistration: jest.fn(),
        recordDistributedSubInitialResultsCount: jest.fn(),
        setDistributedSubPendingAcks: jest.fn(),
        incDistributedSubAck: jest.fn(),
        incDistributedSubUnsubscribe: jest.fn(),
        decDistributedSubActive: jest.fn(),
        incDistributedSubUpdates: jest.fn(),
        recordDistributedSubUpdateLatency: jest.fn(),
      };

      const coordinatorWithMetrics = new DistributedSubscriptionCoordinator(
        clusterManager as any,
        queryRegistry,
        searchCoordinator,
        undefined,
        mockMetrics as any
      );

      const socket = createMockSocket('client-1');

      const subscribePromise = coordinatorWithMetrics.subscribeQuery(
        'query-latency-metrics',
        socket,
        'items',
        { where: { active: true } }
      );

      clusterManager.receiveMessage('node-2', 'CLUSTER_SUB_ACK', {
        subscriptionId: 'query-latency-metrics',
        nodeId: 'node-2',
        success: true,
        initialResults: [],
        totalHits: 0,
      });
      clusterManager.receiveMessage('node-3', 'CLUSTER_SUB_ACK', {
        subscriptionId: 'query-latency-metrics',
        nodeId: 'node-3',
        success: true,
        initialResults: [],
        totalHits: 0,
      });

      await subscribePromise;

      // Clear mocks
      mockMetrics.incDistributedSubUpdates.mockClear();
      mockMetrics.recordDistributedSubUpdateLatency.mockClear();

      // Simulate update
      const timestamp = Date.now() - 15; // 15ms ago
      clusterManager.receiveMessage('node-2', 'CLUSTER_SUB_UPDATE', {
        subscriptionId: 'query-latency-metrics',
        sourceNodeId: 'node-2',
        key: 'item-1',
        value: { active: true },
        changeType: 'ENTER',
        timestamp,
      });

      expect(mockMetrics.incDistributedSubUpdates).toHaveBeenCalledWith('received', 'ENTER');
      expect(mockMetrics.recordDistributedSubUpdateLatency).toHaveBeenCalledWith('QUERY', expect.any(Number));

      coordinatorWithMetrics.destroy();
    });
  });
});
