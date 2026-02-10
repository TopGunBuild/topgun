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
