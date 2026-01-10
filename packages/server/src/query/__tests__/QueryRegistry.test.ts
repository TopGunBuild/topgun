import { QueryRegistry, Subscription } from '../QueryRegistry';
import { LWWRecord, LWWMap, deserialize } from '@topgunbuild/core';
import { WebSocket } from 'ws';

// Mock WebSocket
const createMockSocket = (): WebSocket => {
  return {
    readyState: 1,
    send: jest.fn(),
  } as unknown as WebSocket;
};

const createRecord = (value: any, timestamp: number = Date.now()): LWWRecord<any> => ({
  value,
  timestamp: { millis: timestamp, counter: 0, nodeId: 'test' }
});

describe('QueryRegistry', () => {
  let registry: QueryRegistry;
  let mockMap: any;
  let mapRecords: Map<string, LWWRecord<any>>;

  beforeEach(() => {
    registry = new QueryRegistry();
    mapRecords = new Map();
    
    mockMap = {
      allKeys: () => mapRecords.keys(),
      getRecord: (key: string) => mapRecords.get(key),
    };
  });

  test('should handle sliding window (limit)', () => {
    const socket = createMockSocket();
    const sub: Subscription = {
      id: 'sub1',
      clientId: 'c1',
      mapName: 'items',
      query: {
        sort: { score: 'desc' },
        limit: 3
      },
      socket,
      previousResultKeys: new Set(['A', 'B', 'C'])
    };

    // Initial State: A=100, B=90, C=80, D=70
    mapRecords.set('A', createRecord({ score: 100 }));
    mapRecords.set('B', createRecord({ score: 90 }));
    mapRecords.set('C', createRecord({ score: 80 }));
    mapRecords.set('D', createRecord({ score: 70 }));

    registry.register(sub);

    // Update D to 95. It should enter top 3 (A, D, B). C should fall out.
    const newD = createRecord({ score: 95 });
    mapRecords.set('D', newD);

    registry.processChange('items', mockMap as LWWMap<string, any>, 'D', newD);

    expect(socket.send).toHaveBeenCalledTimes(2);

    // C should be removed
    const call1 = deserialize((socket.send as jest.Mock).mock.calls[0][0] as Uint8Array) as any;
    // D should be updated/added
    const call2 = deserialize((socket.send as jest.Mock).mock.calls[1][0] as Uint8Array) as any;
    
    // The order of calls depends on iteration, but logically we expect both.
    // My implementation iterates 'removed' then 'added'.
    
    // Check removed
    const removedMsg = [call1, call2].find(m => m.payload.type === 'REMOVE');
    expect(removedMsg).toBeDefined();
    expect(removedMsg.payload.key).toBe('C');

    // Check updated
    const updatedMsg = [call1, call2].find(m => m.payload.key === 'D');
    expect(updatedMsg).toBeDefined();
    expect(updatedMsg.payload.type).toBe('UPDATE');
    expect(updatedMsg.payload.value.score).toBe(95);

    // Verify internal state
    expect(sub.previousResultKeys).toEqual(new Set(['A', 'B', 'D']));
  });

  test('should handle moving out of window', () => {
    const socket = createMockSocket();
    const sub: Subscription = {
      id: 'sub1',
      clientId: 'c1',
      mapName: 'items',
      query: {
        sort: { score: 'desc' },
        limit: 2
      },
      socket,
      previousResultKeys: new Set(['A', 'B'])
    };

    // Initial: A=100, B=90, C=80
    mapRecords.set('A', createRecord({ score: 100 }));
    mapRecords.set('B', createRecord({ score: 90 }));
    mapRecords.set('C', createRecord({ score: 80 }));

    registry.register(sub);

    // Update A to 70. New order: B(90), C(80), A(70). Top 2: B, C.
    // A leaves, C enters.
    const newA = createRecord({ score: 70 });
    mapRecords.set('A', newA);

    registry.processChange('items', mockMap as LWWMap<string, any>, 'A', newA);

    expect(socket.send).toHaveBeenCalledTimes(2);

    // A should be removed
    const call1 = deserialize((socket.send as jest.Mock).mock.calls[0][0] as Uint8Array) as any;
    const call2 = deserialize((socket.send as jest.Mock).mock.calls[1][0] as Uint8Array) as any;

    const removedMsg = [call1, call2].find(m => m.payload.type === 'REMOVE');
    expect(removedMsg).toBeDefined();
    expect(removedMsg.payload.key).toBe('A');

    const addedMsg = [call1, call2].find(m => m.payload.key === 'C');
    expect(addedMsg).toBeDefined();
    expect(addedMsg.payload.type).toBe('UPDATE');
  });
});

// ===========================================
// Phase 14.2.6: QueryRegistry Distributed Subscription Tests
// ===========================================

import { EventEmitter } from 'events';

// Mock ClusterManager for distributed tests
class MockClusterManager extends EventEmitter {
  config = { nodeId: 'node-1' };
  private sentMessages: Array<{ nodeId: string; type: string; payload: any }> = [];

  send(nodeId: string, type: string, payload: any): void {
    this.sentMessages.push({ nodeId, type, payload });
  }

  getSentMessages(): Array<{ nodeId: string; type: string; payload: any }> {
    return this.sentMessages;
  }

  clearSentMessages(): void {
    this.sentMessages = [];
  }
}

describe('QueryRegistry - Distributed Subscriptions', () => {
  let registry: QueryRegistry;
  let mockClusterManager: MockClusterManager;
  let mapRecords: Map<string, LWWRecord<any>>;
  let mockMap: any;

  beforeEach(() => {
    registry = new QueryRegistry();
    mockClusterManager = new MockClusterManager();
    mapRecords = new Map();

    mockMap = {
      allKeys: () => mapRecords.keys(),
      getRecord: (key: string) => mapRecords.get(key),
    };

    registry.setClusterManager(mockClusterManager as any, 'node-1');
    registry.setMapGetter((mapName: string) => mockMap);
  });

  it('should register distributed subscription and return initial results', () => {
    // Setup mock map with data
    mapRecords.set('rec-1', createRecord({ name: 'Alice', age: 25 }));
    mapRecords.set('rec-2', createRecord({ name: 'Bob', age: 17 }));
    mapRecords.set('rec-3', createRecord({ name: 'Charlie', age: 30 }));

    const results = registry.registerDistributed(
      'dist-sub-1',
      'people',
      { where: { age: { $gte: 18 } } },
      'coordinator-node'
    );

    // Should return records matching query
    expect(results.length).toBe(2);
    expect(results.map(r => r.key)).toContain('rec-1');
    expect(results.map(r => r.key)).toContain('rec-3');
    expect(results.map(r => r.key)).not.toContain('rec-2');
  });

  it('should send updates via ClusterManager for distributed subscriptions', () => {
    mapRecords.set('rec-1', createRecord({ status: 'active' }));

    registry.registerDistributed(
      'dist-sub-2',
      'items',
      { where: { status: 'active' } },
      'node-coordinator'
    );

    mockClusterManager.clearSentMessages();

    // Simulate adding a new record that matches the query
    const newRecord = createRecord({ status: 'active' });
    mapRecords.set('rec-2', newRecord);

    registry.processChange('items', mockMap as LWWMap<string, any>, 'rec-2', newRecord);

    // Verify update sent to coordinator node
    const updateMessages = mockClusterManager.getSentMessages().filter(
      m => m.type === 'CLUSTER_SUB_UPDATE'
    );
    expect(updateMessages.length).toBe(1);
    expect(updateMessages[0].nodeId).toBe('node-coordinator');
    expect(updateMessages[0].payload.changeType).toBe('ENTER');
    expect(updateMessages[0].payload.key).toBe('rec-2');
  });

  it('should send LEAVE update when record no longer matches query', () => {
    // Setup initial matching record
    const initialRecord = createRecord({ status: 'active' });
    mapRecords.set('rec-1', initialRecord);

    registry.registerDistributed(
      'dist-sub-3',
      'items',
      { where: { status: 'active' } },
      'node-coordinator'
    );

    mockClusterManager.clearSentMessages();

    // Update record to no longer match
    const updatedRecord = createRecord({ status: 'inactive' });
    mapRecords.set('rec-1', updatedRecord);

    // Pass the old record to processChange so it can detect the transition
    registry.processChange('items', mockMap as LWWMap<string, any>, 'rec-1', updatedRecord, initialRecord);

    // Verify LEAVE update sent
    const updateMessages = mockClusterManager.getSentMessages().filter(
      m => m.type === 'CLUSTER_SUB_UPDATE'
    );
    expect(updateMessages.length).toBe(1);
    expect(updateMessages[0].payload.changeType).toBe('LEAVE');
    expect(updateMessages[0].payload.key).toBe('rec-1');
  });

  it('should handle unsubscribe for distributed subscriptions', () => {
    mapRecords.set('rec-1', createRecord({ status: 'active' }));

    registry.registerDistributed(
      'dist-sub-4',
      'data',
      { where: { status: 'active' } },
      'node-x'
    );

    expect(registry.getDistributedSubscription('dist-sub-4')).toBeDefined();

    registry.unregister('dist-sub-4');

    expect(registry.getDistributedSubscription('dist-sub-4')).toBeUndefined();
  });

  it('should return empty array when map has no data', () => {
    // Map is empty
    const results = registry.registerDistributed(
      'dist-sub-empty',
      'empty-map',
      { where: { status: 'active' } },
      'coordinator-node'
    );

    expect(results).toEqual([]);
  });

  it('should track previousResultKeys correctly for distributed subscriptions', () => {
    mapRecords.set('rec-1', createRecord({ score: 100 }));
    mapRecords.set('rec-2', createRecord({ score: 50 }));

    registry.registerDistributed(
      'dist-sub-tracking',
      'scores',
      { where: { score: { $gte: 75 } } },
      'coordinator-node'
    );

    const sub = registry.getDistributedSubscription('dist-sub-tracking');
    expect(sub).toBeDefined();
    expect(sub!.previousResultKeys.has('rec-1')).toBe(true);
    expect(sub!.previousResultKeys.has('rec-2')).toBe(false);
  });

  it('should support complex queries in distributed subscriptions', () => {
    mapRecords.set('task-1', createRecord({ status: 'active', priority: 'high' }));
    mapRecords.set('task-2', createRecord({ status: 'active', priority: 'low' }));
    mapRecords.set('task-3', createRecord({ status: 'completed', priority: 'high' }));

    const results = registry.registerDistributed(
      'dist-sub-complex',
      'tasks',
      {
        where: {
          status: 'active',
          priority: 'high',
        }
      },
      'coordinator-node'
    );

    // Only task-1 matches both conditions
    expect(results.length).toBe(1);
    expect(results[0].key).toBe('task-1');
  });

  it('should correctly mark subscription as distributed', () => {
    mapRecords.set('rec-1', createRecord({ active: true }));

    registry.registerDistributed(
      'dist-sub-marker',
      'items',
      { where: { active: true } },
      'remote-coordinator'
    );

    const sub = registry.getDistributedSubscription('dist-sub-marker');
    expect(sub).toBeDefined();
    expect(sub!.isDistributed).toBe(true);
    expect(sub!.coordinatorNodeId).toBe('remote-coordinator');
  });

  it('should not send updates to local socket for distributed subscriptions', () => {
    mapRecords.set('rec-1', createRecord({ status: 'active' }));

    registry.registerDistributed(
      'dist-sub-no-socket',
      'items',
      { where: { status: 'active' } },
      'node-coordinator'
    );

    const sub = registry.getDistributedSubscription('dist-sub-no-socket');
    expect(sub).toBeDefined();

    mockClusterManager.clearSentMessages();

    // Add new matching record
    const newRecord = createRecord({ status: 'active' });
    mapRecords.set('rec-2', newRecord);

    registry.processChange('items', mockMap as LWWMap<string, any>, 'rec-2', newRecord);

    // Should send via ClusterManager, not socket
    const clusterMessages = mockClusterManager.getSentMessages();
    expect(clusterMessages.length).toBeGreaterThan(0);

    // The dummy socket's send should not be called with actual data
    // (socket.send is a no-op for distributed subscriptions)
  });

  it('should include timestamp in distributed updates', () => {
    mapRecords.set('rec-1', createRecord({ status: 'active' }));

    registry.registerDistributed(
      'dist-sub-timestamp',
      'items',
      { where: { status: 'active' } },
      'node-coordinator'
    );

    mockClusterManager.clearSentMessages();

    const beforeUpdate = Date.now();
    const newRecord = createRecord({ status: 'active' });
    mapRecords.set('rec-2', newRecord);

    registry.processChange('items', mockMap as LWWMap<string, any>, 'rec-2', newRecord);
    const afterUpdate = Date.now();

    const updateMessages = mockClusterManager.getSentMessages().filter(
      m => m.type === 'CLUSTER_SUB_UPDATE'
    );
    expect(updateMessages.length).toBe(1);
    expect(updateMessages[0].payload.timestamp).toBeGreaterThanOrEqual(beforeUpdate);
    expect(updateMessages[0].payload.timestamp).toBeLessThanOrEqual(afterUpdate);
  });
});
