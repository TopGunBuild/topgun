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
});
