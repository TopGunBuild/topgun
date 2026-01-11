/**
 * Tests for DistributedSubscriptionCoordinator
 * Covers Zod validation and node disconnect handling
 */

import { EventEmitter } from 'events';
import { WebSocket } from 'ws';
import { DistributedSubscriptionCoordinator } from '../subscriptions/DistributedSubscriptionCoordinator';
import { ClusterManager } from '../cluster/ClusterManager';
import { SearchCoordinator } from '../search/SearchCoordinator';
import { QueryRegistry } from '../query/QueryRegistry';
import { MetricsService } from '../monitoring/MetricsService';
import { register } from 'prom-client';

// Mock ClusterManager
class MockClusterManager extends EventEmitter {
  config = { nodeId: 'node-1' };
  private members = new Set<string>(['node-1', 'node-2', 'node-3']);

  getMembers(): string[] {
    return Array.from(this.members);
  }

  send(nodeId: string, type: string, payload: any): void {
    // Mock send - emit for testing
    this.emit('messageSent', { nodeId, type, payload });
  }

  removeMember(nodeId: string): void {
    this.members.delete(nodeId);
  }
}

// Mock SearchCoordinator
class MockSearchCoordinator extends EventEmitter {
  private distributedSubs = new Map<string, { coordinatorNodeId: string }>();
  public unsubscribeCalls: string[] = [];
  public unsubscribeByCoordinatorCalls: string[] = [];

  registerDistributedSubscription(
    subscriptionId: string,
    mapName: string,
    query: string,
    options: any,
    coordinatorNodeId: string
  ): { results: any[]; totalHits: number } {
    this.distributedSubs.set(subscriptionId, { coordinatorNodeId });
    return { results: [], totalHits: 0 };
  }

  getDistributedSubscription(subscriptionId: string) {
    return this.distributedSubs.get(subscriptionId);
  }

  unsubscribe(subscriptionId: string): void {
    this.unsubscribeCalls.push(subscriptionId);
    this.distributedSubs.delete(subscriptionId);
  }

  unsubscribeByCoordinator(coordinatorNodeId: string): void {
    this.unsubscribeByCoordinatorCalls.push(coordinatorNodeId);
    for (const [subId, sub] of this.distributedSubs) {
      if (sub.coordinatorNodeId === coordinatorNodeId) {
        this.distributedSubs.delete(subId);
      }
    }
  }
}

// Mock QueryRegistry
class MockQueryRegistry {
  private distributedSubs = new Map<string, { coordinatorNodeId: string }>();
  public unregisterCalls: string[] = [];
  public unregisterByCoordinatorCalls: string[] = [];

  registerDistributed(
    subscriptionId: string,
    mapName: string,
    query: any,
    coordinatorNodeId: string
  ): any[] {
    this.distributedSubs.set(subscriptionId, { coordinatorNodeId });
    return [];
  }

  getDistributedSubscription(subscriptionId: string) {
    return this.distributedSubs.get(subscriptionId);
  }

  unregister(subscriptionId: string): void {
    this.unregisterCalls.push(subscriptionId);
    this.distributedSubs.delete(subscriptionId);
  }

  unregisterByCoordinator(coordinatorNodeId: string): void {
    this.unregisterByCoordinatorCalls.push(coordinatorNodeId);
    for (const [subId, sub] of this.distributedSubs) {
      if (sub.coordinatorNodeId === coordinatorNodeId) {
        this.distributedSubs.delete(subId);
      }
    }
  }
}

// Mock WebSocket
function createMockSocket(): WebSocket {
  return {
    readyState: WebSocket.OPEN,
    send: jest.fn(),
  } as unknown as WebSocket;
}

describe('DistributedSubscriptionCoordinator', () => {
  let clusterManager: MockClusterManager;
  let searchCoordinator: MockSearchCoordinator;
  let queryRegistry: MockQueryRegistry;
  let coordinator: DistributedSubscriptionCoordinator;
  let metricsService: MetricsService;

  beforeEach(() => {
    register.clear();
    clusterManager = new MockClusterManager();
    searchCoordinator = new MockSearchCoordinator();
    queryRegistry = new MockQueryRegistry();
    metricsService = new MetricsService();

    coordinator = new DistributedSubscriptionCoordinator(
      clusterManager as unknown as ClusterManager,
      queryRegistry as unknown as QueryRegistry,
      searchCoordinator as unknown as SearchCoordinator,
      undefined,
      metricsService
    );
  });

  afterEach(() => {
    coordinator.destroy();
    metricsService.destroy();
    register.clear();
  });

  describe('Zod validation in handleClusterMessage', () => {
    test('should reject invalid CLUSTER_SUB_REGISTER payload', () => {
      const invalidPayload = {
        // Missing required fields
        subscriptionId: 'sub-1',
        // coordinatorNodeId is missing
        // mapName is missing
        // type is missing
      };

      // Emit message through cluster manager
      clusterManager.emit('message', {
        type: 'CLUSTER_SUB_REGISTER',
        senderId: 'node-2',
        payload: invalidPayload,
      });

      // Should not crash, should log warning and ignore
      expect(searchCoordinator.unsubscribeCalls).toHaveLength(0);
    });

    test('should reject invalid CLUSTER_SUB_ACK payload', () => {
      const invalidPayload = {
        // Missing required fields
        subscriptionId: 'sub-1',
        // nodeId is missing
        // success is missing
      };

      clusterManager.emit('message', {
        type: 'CLUSTER_SUB_ACK',
        senderId: 'node-2',
        payload: invalidPayload,
      });

      // Should not crash
      expect(coordinator.getActiveSubscriptionCount()).toBe(0);
    });

    test('should reject invalid CLUSTER_SUB_UPDATE payload', () => {
      const invalidPayload = {
        subscriptionId: 'sub-1',
        // key is missing
        // changeType is missing
      };

      clusterManager.emit('message', {
        type: 'CLUSTER_SUB_UPDATE',
        senderId: 'node-2',
        payload: invalidPayload,
      });

      // Should not crash
      expect(coordinator.getActiveSubscriptionCount()).toBe(0);
    });

    test('should reject invalid CLUSTER_SUB_UNREGISTER payload', () => {
      const invalidPayload = {
        // subscriptionId is missing
      };

      clusterManager.emit('message', {
        type: 'CLUSTER_SUB_UNREGISTER',
        senderId: 'node-2',
        payload: invalidPayload,
      });

      // Should not crash
      expect(searchCoordinator.unsubscribeCalls).toHaveLength(0);
    });

    test('should accept valid CLUSTER_SUB_UNREGISTER payload', () => {
      const validPayload = {
        subscriptionId: 'sub-123',
      };

      clusterManager.emit('message', {
        type: 'CLUSTER_SUB_UNREGISTER',
        senderId: 'node-2',
        payload: validPayload,
      });

      // Should call unsubscribe on both registries
      expect(searchCoordinator.unsubscribeCalls).toContain('sub-123');
      expect(queryRegistry.unregisterCalls).toContain('sub-123');
    });
  });

  describe('handleMemberLeft - node disconnect cleanup', () => {
    test('should cleanup subscriptions when coordinator node disconnects', () => {
      // Simulate node-2 disconnecting
      clusterManager.emit('memberLeft', 'node-2');

      // Should call unsubscribeByCoordinator on both registries
      expect(searchCoordinator.unsubscribeByCoordinatorCalls).toContain('node-2');
      expect(queryRegistry.unregisterByCoordinatorCalls).toContain('node-2');
    });

    test('should record node disconnect metric', async () => {
      clusterManager.emit('memberLeft', 'node-2');

      const output = await metricsService.getMetrics();
      expect(output).toContain('topgun_distributed_sub_node_disconnect_total 1');
    });

    test('should cleanup multiple node disconnects', async () => {
      clusterManager.emit('memberLeft', 'node-2');
      clusterManager.emit('memberLeft', 'node-3');

      expect(searchCoordinator.unsubscribeByCoordinatorCalls).toEqual(['node-2', 'node-3']);
      expect(queryRegistry.unregisterByCoordinatorCalls).toEqual(['node-2', 'node-3']);

      const output = await metricsService.getMetrics();
      expect(output).toContain('topgun_distributed_sub_node_disconnect_total 2');
    });
  });
});
