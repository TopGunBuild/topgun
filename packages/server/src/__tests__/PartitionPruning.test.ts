/**
 * Tests for partition pruning logic in PartitionService and
 * checkAcksComplete behaviour when targetedNodes is set.
 */

import { PartitionService } from '../cluster/PartitionService';
import { ClusterManager } from '../cluster/ClusterManager';
import { PARTITION_COUNT, hashString } from '@topgunbuild/core';
import { EventEmitter } from 'events';
import {
  DistributedSubscriptionBase,
  DistributedSubscription,
  DistributedSubscriptionResult,
} from '../subscriptions/DistributedSubscriptionBase';
import type {
  ClusterSubAckPayload,
  ClusterSubUpdatePayload,
} from '@topgunbuild/core';
import { WebSocket } from 'ws';

// ---------------------------------------------------------------------------
// Mocks
// ---------------------------------------------------------------------------

class MockClusterManager extends EventEmitter {
  config = {
    nodeId: 'node-1',
    host: 'localhost',
    port: 10600,
  };

  port = 10600;

  private members: string[] = ['node-1'];

  getMembers(): string[] {
    return this.members;
  }

  setMembers(members: string[]): void {
    this.members = members;
  }

  isLocal(nodeId: string): boolean {
    return nodeId === this.config.nodeId;
  }

  send(_nodeId: string, _type: string, _payload: unknown): void {
    // no-op for testing
  }
}

/**
 * Minimal concrete subclass so we can unit-test checkAcksComplete.
 */
class TestSubscriptionCoordinator extends DistributedSubscriptionBase {
  getSubscriptionType(): 'SEARCH' | 'QUERY' {
    return 'QUERY';
  }

  mergeInitialResults(sub: DistributedSubscription): DistributedSubscriptionResult {
    const results: DistributedSubscriptionResult['results'] = [];
    for (const [, payload] of sub.pendingResults) {
      if (payload.initialResults) {
        for (const r of payload.initialResults) {
          results.push({ key: r.key, value: r.value, score: r.score, matchedTerms: r.matchedTerms });
        }
      }
    }

    const expectedNodes = sub.targetedNodes ?? new Set(this.clusterManager.getMembers());
    const failedNodes = Array.from(expectedNodes).filter(n => !sub.registeredNodes.has(n));

    return {
      subscriptionId: sub.id,
      results,
      totalHits: results.length,
      registeredNodes: Array.from(sub.registeredNodes),
      failedNodes,
    };
  }

  buildUpdateMessage(_sub: DistributedSubscription, _payload: ClusterSubUpdatePayload) {
    return { type: 'QUERY_UPDATE', payload: {} };
  }

  cleanupByCoordinator(_nodeId: string): void {
    // no-op
  }

  protected unregisterLocalSubscription(_sub: DistributedSubscription): void {
    // no-op
  }

  // Expose protected internals for testing
  public exposedSubscriptions() {
    return this.subscriptions;
  }

  public exposedNodeAcks() {
    return this.nodeAcks;
  }

  public exposedWaitForAcks(subId: string, expectedNodes: Set<string>) {
    return this.waitForAcks(subId, expectedNodes);
  }
}

// ---------------------------------------------------------------------------
// Partition Pruning – getRelevantPartitions
// ---------------------------------------------------------------------------

describe('PartitionPruning', () => {
  let cluster: MockClusterManager;
  let ps: PartitionService;

  beforeEach(() => {
    cluster = new MockClusterManager();
    ps = new PartitionService(cluster as unknown as ClusterManager);
  });

  // ---- where-clause extraction ----

  describe('getRelevantPartitions – where clause', () => {
    it('should return single partition for _key equality', () => {
      const result = ps.getRelevantPartitions({ where: { _key: 'abc' } });

      expect(result).not.toBeNull();
      expect(result!.length).toBe(1);
      expect(result![0]).toBe(ps.getPartitionId('abc'));
    });

    it('should return single partition for _id equality (aligned with read-replica)', () => {
      const result = ps.getRelevantPartitions({ where: { _id: 'doc-42' } });

      expect(result).not.toBeNull();
      expect(result!.length).toBe(1);
      expect(result![0]).toBe(ps.getPartitionId('doc-42'));
    });

    it('should recognise "key" attribute name', () => {
      const result = ps.getRelevantPartitions({ where: { key: 'k1' } });

      expect(result).not.toBeNull();
      expect(result!.length).toBe(1);
      expect(result![0]).toBe(ps.getPartitionId('k1'));
    });

    it('should recognise "id" attribute name', () => {
      const result = ps.getRelevantPartitions({ where: { id: 'id1' } });

      expect(result).not.toBeNull();
      expect(result!.length).toBe(1);
      expect(result![0]).toBe(ps.getPartitionId('id1'));
    });

    it('should handle $eq operator form', () => {
      const result = ps.getRelevantPartitions({ where: { _key: { $eq: 'x' } } });

      expect(result).not.toBeNull();
      expect(result!.length).toBe(1);
      expect(result![0]).toBe(ps.getPartitionId('x'));
    });

    it('should handle $in operator form', () => {
      const result = ps.getRelevantPartitions({ where: { _key: { $in: ['a', 'b', 'c'] } } });

      expect(result).not.toBeNull();
      // Could be 1-3 depending on hash collisions – just verify correctness
      const expectedPids = new Set(['a', 'b', 'c'].map(k => ps.getPartitionId(k)));
      expect(result!.length).toBe(expectedPids.size);
      for (const pid of result!) {
        expect(expectedPids.has(pid)).toBe(true);
      }
    });

    it('should handle array of keys (implicit IN)', () => {
      const result = ps.getRelevantPartitions({ where: { _key: ['a', 'b'] } });

      expect(result).not.toBeNull();
      const expected = new Set(['a', 'b'].map(k => ps.getPartitionId(k)));
      expect(result!.length).toBe(expected.size);
    });
  });

  // ---- predicate extraction ----

  describe('getRelevantPartitions – predicate', () => {
    it('should return single partition for _key eq predicate', () => {
      const result = ps.getRelevantPartitions({
        predicate: { op: 'eq', attribute: '_key', value: 'hello' },
      });

      expect(result).not.toBeNull();
      expect(result!.length).toBe(1);
      expect(result![0]).toBe(ps.getPartitionId('hello'));
    });

    it('should extract _key from AND compound predicate', () => {
      const result = ps.getRelevantPartitions({
        predicate: {
          op: 'and',
          children: [
            { op: 'eq', attribute: '_key', value: 'k1' },
            { op: 'gt', attribute: 'age', value: 18 },
          ],
        },
      });

      expect(result).not.toBeNull();
      expect(result!.length).toBe(1);
      expect(result![0]).toBe(ps.getPartitionId('k1'));
    });

    it('should recognise key and id attributes in predicate', () => {
      for (const attr of ['_key', 'key', 'id', '_id']) {
        const result = ps.getRelevantPartitions({
          predicate: { op: 'eq', attribute: attr, value: 'val' },
        });
        expect(result).not.toBeNull();
        expect(result!.length).toBe(1);
      }
    });
  });

  // ---- multiple keys / deduplication ----

  describe('getRelevantPartitions – deduplication', () => {
    it('should return deduplicated partitions for multiple keys', () => {
      // Find two keys that hash to the same partition (pigeonhole: with 271 partitions
      // trying 300+ keys guarantees at least one collision)
      const keysByPartition = new Map<number, string[]>();
      let duplicateKeys: string[] | null = null;

      for (let i = 0; i < 500; i++) {
        const k = `dup-key-${i}`;
        const pid = ps.getPartitionId(k);
        const existing = keysByPartition.get(pid) || [];
        existing.push(k);
        keysByPartition.set(pid, existing);

        if (existing.length >= 2 && !duplicateKeys) {
          duplicateKeys = existing.slice(0, 2);
        }
      }

      expect(duplicateKeys).not.toBeNull();

      const result = ps.getRelevantPartitions({ where: { _key: duplicateKeys! } });
      expect(result).not.toBeNull();
      // Both keys hash to same partition so result should have length 1
      expect(result!.length).toBe(1);
    });
  });

  // ---- non-prunable queries ----

  describe('getRelevantPartitions – non-prunable', () => {
    it('should return null when no key predicate present', () => {
      expect(ps.getRelevantPartitions({})).toBeNull();
      expect(ps.getRelevantPartitions({ where: { name: 'foo' } })).toBeNull();
      expect(ps.getRelevantPartitions({
        predicate: { op: 'gt', attribute: 'age', value: 18 },
      })).toBeNull();
    });

    it('should return null for OR query with _key', () => {
      const result = ps.getRelevantPartitions({
        predicate: {
          op: 'or',
          children: [
            { op: 'eq', attribute: '_key', value: 'a' },
            { op: 'eq', attribute: 'name', value: 'b' },
          ],
        },
      });
      expect(result).toBeNull();
    });

    it('should return null for NOT predicate', () => {
      const result = ps.getRelevantPartitions({
        predicate: { op: 'not', children: [{ op: 'eq', attribute: '_key', value: 'x' }] },
      });
      expect(result).toBeNull();
    });
  });

  // ---- getOwnerNodesForPartitions ----

  describe('getOwnerNodesForPartitions', () => {
    it('should return correct deduplicated owner nodes', () => {
      // With a 3-node cluster, partitions are distributed across 3 nodes
      cluster.setMembers(['node-1', 'node-2', 'node-3']);
      cluster.emit('memberJoined', 'node-2');

      // Partition 0 -> node-1, partition 1 -> node-2, partition 2 -> node-3
      // (sorted members: node-1, node-2, node-3; owner = partition % 3)
      const owners = ps.getOwnerNodesForPartitions([0, 1, 2]);
      expect(owners.sort()).toEqual(['node-1', 'node-2', 'node-3']);
    });

    it('should deduplicate when partitions share the same owner', () => {
      // Single-node cluster: all partitions owned by node-1
      const owners = ps.getOwnerNodesForPartitions([0, 1, 2, 3]);
      expect(owners).toEqual(['node-1']);
    });

    it('should exclude unassigned partition IDs', () => {
      const owners = ps.getOwnerNodesForPartitions([99999]);
      expect(owners).toEqual([]);
    });

    it('should return empty array for empty input', () => {
      expect(ps.getOwnerNodesForPartitions([])).toEqual([]);
    });
  });

  // ---- $eq operator on non-key attribute ----

  describe('getRelevantPartitions – non-key $eq operator', () => {
    it('should return null when $eq is on a non-key attribute', () => {
      const result = ps.getRelevantPartitions({ where: { status: { $eq: 'active' } } });
      expect(result).toBeNull();
    });
  });

  // ---- checkAcksComplete with targetedNodes (R4a fix) ----

  describe('checkAcksComplete with targetedNodes', () => {
    let mockCluster: MockClusterManager;
    let coordinator: TestSubscriptionCoordinator;

    beforeEach(() => {
      mockCluster = new MockClusterManager();
      mockCluster.setMembers(['node-1', 'node-2', 'node-3']);

      coordinator = new TestSubscriptionCoordinator(
        mockCluster as unknown as ClusterManager,
        { ackTimeoutMs: 10_000 },
        undefined,
        { registerMemberLeftListener: false }
      );
    });

    it('should complete ACKs without timeout when only targeted nodes respond', async () => {
      const subId = 'sub-prune-1';
      const targetedNodes = new Set(['node-1', 'node-2']); // Only 2 of 3 cluster nodes

      const mockSocket = {
        readyState: WebSocket.OPEN,
        send: jest.fn(),
      } as unknown as WebSocket;

      // Set up subscription state with targetedNodes
      const sub: DistributedSubscription = {
        id: subId,
        type: 'QUERY',
        coordinatorNodeId: 'node-1',
        clientSocket: mockSocket,
        mapName: 'users',
        registeredNodes: new Set(),
        pendingResults: new Map(),
        createdAt: Date.now(),
        currentResults: new Map(),
        targetedNodes,
      };

      coordinator.exposedSubscriptions().set(subId, sub);
      coordinator.exposedNodeAcks().set(subId, new Set());

      // Start waiting for ACKs
      const resultPromise = coordinator.exposedWaitForAcks(subId, targetedNodes);

      // Simulate ACKs from only the 2 targeted nodes (node-3 never responds)
      coordinator.handleSubAck('node-1', {
        subscriptionId: subId,
        nodeId: 'node-1',
        success: true,
        initialResults: [{ key: 'k1', value: { name: 'Alice' } }],
        totalHits: 1,
      });

      coordinator.handleSubAck('node-2', {
        subscriptionId: subId,
        nodeId: 'node-2',
        success: true,
        initialResults: [{ key: 'k2', value: { name: 'Bob' } }],
        totalHits: 1,
      });

      // Should resolve promptly without waiting for node-3
      const result = await resultPromise;

      expect(result.subscriptionId).toBe(subId);
      expect(result.registeredNodes.sort()).toEqual(['node-1', 'node-2']);
      expect(result.failedNodes).toEqual([]);
      expect(result.results.length).toBe(2);
    });

    it('should wait for all cluster members when targetedNodes is absent', async () => {
      const subId = 'sub-no-prune';

      const mockSocket = {
        readyState: WebSocket.OPEN,
        send: jest.fn(),
      } as unknown as WebSocket;

      // Subscription without targetedNodes – should expect all 3 cluster members
      const sub: DistributedSubscription = {
        id: subId,
        type: 'QUERY',
        coordinatorNodeId: 'node-1',
        clientSocket: mockSocket,
        mapName: 'users',
        registeredNodes: new Set(),
        pendingResults: new Map(),
        createdAt: Date.now(),
        currentResults: new Map(),
        // no targetedNodes
      };

      coordinator.exposedSubscriptions().set(subId, sub);
      coordinator.exposedNodeAcks().set(subId, new Set());

      const resultPromise = coordinator.exposedWaitForAcks(subId, new Set(mockCluster.getMembers()));

      // Send ACKs from only 2 of 3 nodes
      coordinator.handleSubAck('node-1', {
        subscriptionId: subId,
        nodeId: 'node-1',
        success: true,
        initialResults: [],
        totalHits: 0,
      });

      coordinator.handleSubAck('node-2', {
        subscriptionId: subId,
        nodeId: 'node-2',
        success: true,
        initialResults: [],
        totalHits: 0,
      });

      // Result should NOT be resolved yet (still waiting for node-3)
      let resolved = false;
      resultPromise.then(() => {
        resolved = true;
      });

      // Give a tick for promise to settle if it were going to
      await new Promise(resolve => setTimeout(resolve, 50));
      expect(resolved).toBe(false);

      // Now send the third ACK
      coordinator.handleSubAck('node-3', {
        subscriptionId: subId,
        nodeId: 'node-3',
        success: true,
        initialResults: [],
        totalHits: 0,
      });

      const result = await resultPromise;
      expect(result.registeredNodes.sort()).toEqual(['node-1', 'node-2', 'node-3']);
    });
  });
});

// ---------------------------------------------------------------------------
// Integration Test: Reduced Fan-out in a 2-node Cluster
// ---------------------------------------------------------------------------

import { ServerCoordinator, ServerFactory } from '../';
import { pollUntil } from './utils/test-helpers';
import { createTestHarness, ServerTestHarness } from './utils/ServerTestHarness';

describe('PartitionPruning Integration – Reduced Fan-out', () => {
  let nodeA: ServerCoordinator;
  let nodeB: ServerCoordinator;
  let harnessA: ServerTestHarness;
  let harnessB: ServerTestHarness;

  beforeAll(async () => {
    // Start Node B first (higher ID, receives connection)
    nodeB = ServerFactory.create({
      port: 12010,
      nodeId: 'prune-node-b',
      host: 'localhost',
      clusterPort: 12011,
      peers: [],
    });
    await nodeB.ready();
    harnessB = createTestHarness(nodeB);

    // Start Node A (lower ID, initiates connection to B)
    nodeA = ServerFactory.create({
      port: 12012,
      nodeId: 'prune-node-a',
      host: 'localhost',
      clusterPort: 12013,
      peers: [`localhost:12011`],
    });
    await nodeA.ready();
    harnessA = createTestHarness(nodeA);

    // Wait for cluster to stabilize
    await pollUntil(
      () => {
        const mA = harnessA.cluster.getMembers();
        const mB = harnessB.cluster.getMembers();
        return mA.includes('prune-node-b') && mB.includes('prune-node-a');
      },
      {
        timeoutMs: 10000,
        intervalMs: 100,
        description: 'partition pruning cluster formation',
      }
    );
  }, 20000);

  afterAll(async () => {
    await nodeA?.shutdown();
    await nodeB?.shutdown();
    // WHY: Allow pending cluster WebSocket close events to drain before Jest tears down
    await new Promise(resolve => setTimeout(resolve, 300));
  });

  function findKeyOwnedBy(ps: PartitionService, targetNodeId: string): string {
    for (let i = 0; i < 1000; i++) {
      const key = `prune-test-key-${i}`;
      const pid = ps.getPartitionId(key);
      const owner = ps.getPartitionOwner(pid);
      if (owner === targetNodeId) {
        return key;
      }
    }
    throw new Error(`Could not find key owned by ${targetNodeId}`);
  }

  test('query with _key targeting local partition does NOT send CLUSTER_SUB_REGISTER to remote node', async () => {
    const psA = harnessA.partitionService;
    const localKey = findKeyOwnedBy(psA, 'prune-node-a');

    // Spy on ClusterManager.send on Node A
    // WHY: In a 2-node cluster with distributedSubCoordinator, the distributed subscription
    // path is used (CLUSTER_SUB_REGISTER), not the legacy scatter-gather path (CLUSTER_QUERY_EXEC)
    const sendSpy = jest.spyOn(harnessA.cluster, 'send');

    // Create a mock client to send query through Node A
    const mockClient = {
      id: 'client-prune-local',
      socket: { send: jest.fn(), readyState: WebSocket.OPEN, close: jest.fn() } as any,
      writer: { write: jest.fn(), close: jest.fn() },
      isAuthenticated: true,
      subscriptions: new Set<string>(),
      principal: { userId: 'test', roles: ['ADMIN'] },
      lastActiveHlc: { millis: Date.now(), counter: 0, nodeId: 'client' },
      lastPingReceived: Date.now(),
    };

    // Send QUERY_SUB via the harness
    await harnessA.handleMessage(mockClient, {
      type: 'QUERY_SUB',
      payload: {
        queryId: 'q-local-prune',
        mapName: 'prune-test',
        query: { where: { _key: localKey } },
      },
    });

    // Verify: CLUSTER_SUB_REGISTER should NOT have been sent to prune-node-b
    // because the key is owned locally, so no remote subscription is needed
    const subRegisterCalls = sendSpy.mock.calls.filter(
      ([_nodeId, type]) => type === 'CLUSTER_SUB_REGISTER'
    );
    expect(subRegisterCalls.length).toBe(0);

    sendSpy.mockRestore();
  });

  test('query with _key targeting remote partition sends CLUSTER_SUB_REGISTER to only the owner node', async () => {
    const psA = harnessA.partitionService;
    const remoteKey = findKeyOwnedBy(psA, 'prune-node-b');

    // Spy on ClusterManager.send on Node A
    const sendSpy = jest.spyOn(harnessA.cluster, 'send');

    const mockClient = {
      id: 'client-prune-remote',
      socket: { send: jest.fn(), readyState: WebSocket.OPEN, close: jest.fn() } as any,
      writer: { write: jest.fn(), close: jest.fn() },
      isAuthenticated: true,
      subscriptions: new Set<string>(),
      principal: { userId: 'test', roles: ['ADMIN'] },
      lastActiveHlc: { millis: Date.now(), counter: 0, nodeId: 'client' },
      lastPingReceived: Date.now(),
    };

    // Send QUERY_SUB with _key owned by remote node
    await harnessA.handleMessage(mockClient, {
      type: 'QUERY_SUB',
      payload: {
        queryId: 'q-remote-prune',
        mapName: 'prune-test',
        query: { where: { _key: remoteKey } },
      },
    });

    // Verify: CLUSTER_SUB_REGISTER should have been sent to ONLY prune-node-b
    const subRegisterCalls = sendSpy.mock.calls.filter(
      ([_nodeId, type]) => type === 'CLUSTER_SUB_REGISTER'
    );
    expect(subRegisterCalls.length).toBe(1);
    expect(subRegisterCalls[0][0]).toBe('prune-node-b');

    sendSpy.mockRestore();
  });

  test('query without _key predicate fans out to all remote nodes (no regression)', async () => {
    // Spy on ClusterManager.send on Node A
    const sendSpy = jest.spyOn(harnessA.cluster, 'send');

    const mockClient = {
      id: 'client-prune-all',
      socket: { send: jest.fn(), readyState: WebSocket.OPEN, close: jest.fn() } as any,
      writer: { write: jest.fn(), close: jest.fn() },
      isAuthenticated: true,
      subscriptions: new Set<string>(),
      principal: { userId: 'test', roles: ['ADMIN'] },
      lastActiveHlc: { millis: Date.now(), counter: 0, nodeId: 'client' },
      lastPingReceived: Date.now(),
    };

    // Send QUERY_SUB without _key predicate
    await harnessA.handleMessage(mockClient, {
      type: 'QUERY_SUB',
      payload: {
        queryId: 'q-all-nodes',
        mapName: 'prune-test',
        query: { where: { status: 'active' } },
      },
    });

    // Verify: CLUSTER_SUB_REGISTER should have been sent to prune-node-b (the only remote)
    // because without a _key predicate, all nodes must be contacted
    const subRegisterCalls = sendSpy.mock.calls.filter(
      ([_nodeId, type]) => type === 'CLUSTER_SUB_REGISTER'
    );
    expect(subRegisterCalls.length).toBe(1);
    expect(subRegisterCalls[0][0]).toBe('prune-node-b');

    sendSpy.mockRestore();
  });
});
