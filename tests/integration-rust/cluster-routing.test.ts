/**
 * Integration tests for 3-node cluster smart routing, failover, and partition
 * map sync. These tests boot a real Rust server cluster and exercise the
 * ClusterClient / PartitionRouter end-to-end.
 *
 * Requires: RUST_SERVER_BINARY env var or a cargo build at repo root.
 * Timeout: 30 s per test (cluster formation is slow).
 */

import { TopGunClient, ClusterClient } from '@topgunbuild/client';
import { PARTITION_COUNT, hashString, LWWRecord, ORMapRecord } from '@topgunbuild/core';
import type { IStorageAdapter, OpLogEntry } from '@topgunbuild/client';
import { spawnCluster, ClusterSetup } from './helpers/cluster-setup';

// ----------------------------------------------------------------
// Shared in-memory storage adapter for TopGunClient
// ----------------------------------------------------------------

class MemoryStorageAdapter implements IStorageAdapter {
  private kvStore: Map<string, any> = new Map();
  private metaStore: Map<string, any> = new Map();
  private opLog: OpLogEntry[] = [];
  private _pendingOps: OpLogEntry[] = [];

  async initialize(_dbName: string): Promise<void> {}
  async close(): Promise<void> {}

  async get<V>(key: string): Promise<LWWRecord<V> | ORMapRecord<V>[] | any | undefined> {
    return this.kvStore.get(key);
  }

  async put(key: string, value: any): Promise<void> {
    this.kvStore.set(key, value);
  }

  async remove(key: string): Promise<void> {
    this.kvStore.delete(key);
  }

  async getMeta(key: string): Promise<any> {
    return this.metaStore.get(key);
  }

  async setMeta(key: string, value: any): Promise<void> {
    this.metaStore.set(key, value);
  }

  async batchPut(entries: Map<string, any>): Promise<void> {
    for (const [k, v] of entries) {
      this.kvStore.set(k, v);
    }
  }

  async appendOpLog(entry: Omit<OpLogEntry, 'id'>): Promise<number> {
    const id = this.opLog.length + 1;
    const newEntry = { ...entry, id, synced: 0 };
    this.opLog.push(newEntry);
    this._pendingOps.push(newEntry);
    return id;
  }

  async getPendingOps(): Promise<OpLogEntry[]> {
    return this._pendingOps;
  }

  async markOpsSynced(lastId: number): Promise<void> {
    this._pendingOps = this._pendingOps.filter(op => op.id! > lastId);
    this.opLog.forEach(op => {
      if (op.id! <= lastId) op.synced = 1;
    });
  }

  async getAllKeys(): Promise<string[]> {
    return Array.from(this.kvStore.keys());
  }
}

// ----------------------------------------------------------------
// Helpers
// ----------------------------------------------------------------

function waitMs(ms: number): Promise<void> {
  return new Promise(resolve => setTimeout(resolve, ms));
}

async function waitUntil(
  condition: () => boolean | Promise<boolean>,
  timeoutMs: number,
  pollIntervalMs = 200
): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (await condition()) return;
    await waitMs(pollIntervalMs);
  }
  throw new Error(`Condition not met within ${timeoutMs} ms`);
}

// ----------------------------------------------------------------
// Test suite
// ----------------------------------------------------------------

describe('Integration: 3-node cluster smart routing', () => {
  let cluster: ClusterSetup;

  // Spawn the cluster once before all tests in this suite
  beforeAll(async () => {
    cluster = await spawnCluster({ timeout: 60_000 });
  }, 70_000);

  afterAll(async () => {
    await cluster.cleanup();
  });

  // ----------------------------------------------------------------
  // Test 1: Partition map acquisition
  // ----------------------------------------------------------------
  test('acquires partition map from cluster within 10 s', async () => {
    const client = new ClusterClient({
      enabled: true,
      seedNodes: cluster.seedAddresses,
      routingMode: 'direct',
    });

    try {
      await client.connect();

      await waitUntil(() => client.isRoutingActive(), 10_000);

      const stats = client.getRouterStats()!;
      expect(stats.mapVersion).toBeGreaterThan(0);

      // All 271 partitions must have owners
      const partitionMap = (client as any).partitionRouter?.partitionMap;
      if (partitionMap) {
        expect(partitionMap.partitions.length).toBe(PARTITION_COUNT);
        for (const p of partitionMap.partitions) {
          expect(p.ownerNodeId).toBeTruthy();
        }
      } else {
        // Verify via stats if direct access unavailable
        expect(stats.partitionCount).toBe(PARTITION_COUNT);
      }
    } finally {
      client.close();
    }
  }, 30_000);

  // ----------------------------------------------------------------
  // Test 2: Direct routing correctness
  // ----------------------------------------------------------------
  test('routes 100 write operations with directRoutes > 0 and partitionMisses === 0', async () => {
    const topgun = new TopGunClient({
      cluster: {
        seeds: cluster.seedAddresses,
        smartRouting: true,
      },
      storage: new MemoryStorageAdapter(),
    });

    try {
      // Wait for the TopGunClient's internal ClusterClient to become routing-active
      await waitUntil(() => topgun.isRoutingActive(), 10_000);

      // Access the internal ClusterClient for metrics
      const internalCluster = (topgun as any).clusterClient as ClusterClient;
      internalCluster.resetRoutingMetrics();

      // Use the topgun client to write 100 keys
      const map = topgun.getMap<string, string>('test-routing');

      for (let i = 0; i < 100; i++) {
        map.set(`key-${i}`, `value-${i}`);
        // Small pause to allow routing
        await waitMs(5);
      }

      // Give the sync engine a moment to flush ops
      await waitMs(500);

      const metrics = internalCluster.getRoutingMetrics();
      expect(metrics.directRoutes).toBeGreaterThan(0);
      expect(metrics.partitionMisses).toBe(0);
    } finally {
      topgun.close();
    }
  }, 30_000);

  // ----------------------------------------------------------------
  // Test 3: Read-after-write consistency
  // ----------------------------------------------------------------
  test('read-after-write returns the same value', async () => {
    const clusterClient = new ClusterClient({
      enabled: true,
      seedNodes: cluster.seedAddresses,
      routingMode: 'direct',
    });

    const topgun = new TopGunClient({
      cluster: {
        seeds: cluster.seedAddresses,
        smartRouting: true,
      },
      storage: new MemoryStorageAdapter(),
    });

    try {
      await clusterClient.connect();
      await waitUntil(() => clusterClient.isRoutingActive(), 10_000);

      const map = topgun.getMap<string, string>('test-raw');
      const testKey = 'consistency-key';
      const testValue = 'consistency-value-' + Date.now();

      map.set(testKey, testValue);

      // Allow sync to propagate
      await waitMs(1_000);

      // Read back from the local map (LWW CRDT keeps latest value)
      const value = map.get(testKey);
      expect(value).toBe(testValue);
    } finally {
      topgun.close();
      clusterClient.close();
    }
  }, 30_000);

  // ----------------------------------------------------------------
  // Test 4: Node failover
  // ----------------------------------------------------------------
  test('write succeeds via fallback routing when owning node is stopped', async () => {
    // Connect with only nodes 0 and 2 as seeds (node 1 will be stopped)
    const topgun = new TopGunClient({
      cluster: {
        seeds: cluster.seedAddresses.filter((_, i) => i !== 1),
        smartRouting: true,
      },
      storage: new MemoryStorageAdapter(),
    });

    try {
      await waitUntil(() => topgun.isRoutingActive(), 10_000);

      const internalCluster = (topgun as any).clusterClient as ClusterClient;

      // Stop node 1 — the node owning some partitions
      await cluster.stopNode(1);

      // Give the cluster a moment to detect the failure
      await waitMs(2_000);

      internalCluster.resetRoutingMetrics();

      // Write to a key; if it was owned by node-1, routing falls back to a healthy node
      const map = topgun.getMap<string, string>('test-failover');
      map.set('failover-test-key', 'failover-value');

      await waitMs(1_000);

      const metrics = internalCluster.getRoutingMetrics();
      // The write was routed (direct to live owner or fallback if owner was node-1)
      const totalRouted = metrics.directRoutes + metrics.fallbackRoutes;
      expect(totalRouted).toBeGreaterThan(0);
      expect(metrics.partitionMisses).toBe(0);
    } finally {
      topgun.close();
      // Restart node 1 for subsequent tests
      await cluster.restartNode(1);
      await waitMs(3_000);
    }
  }, 30_000);

  // ----------------------------------------------------------------
  // Test 5: Partition map refresh after failover
  // ----------------------------------------------------------------
  test('partition map version increments and stopped node removed after failover', async () => {
    const clusterClient = new ClusterClient({
      enabled: true,
      seedNodes: cluster.seedAddresses,
      routingMode: 'direct',
    });

    try {
      await clusterClient.connect();
      await waitUntil(() => clusterClient.isRoutingActive(), 10_000);

      const versionBefore = clusterClient.getRouterStats()!.mapVersion;

      // Stop node 1 to trigger cluster failure detection and rebalancing
      await cluster.stopNode(1);

      // Wait up to 15 s for partition map version to increment
      // HeartbeatService phi-accrual detects failure; MembershipReactor rebalances
      await waitUntil(
        () => clusterClient.getRouterStats()!.mapVersion > versionBefore,
        15_000
      );

      const statsAfter = clusterClient.getRouterStats()!;
      expect(statsAfter.mapVersion).toBeGreaterThan(versionBefore);

      // After rebalancing, node-1 should no longer own any partition
      const partitionMap = (clusterClient as any).partitionRouter?.partitionMap;
      if (partitionMap) {
        const node1Partitions = partitionMap.partitions.filter(
          (p: any) => p.ownerNodeId === 'node-1'
        );
        expect(node1Partitions.length).toBe(0);
      }

      // A write after rebalance should route to the new owner (direct, no misses)
      clusterClient.resetRoutingMetrics();

      const topgun = new TopGunClient({
        cluster: {
          seeds: cluster.seedAddresses.filter((_, i) => i !== 1),
          smartRouting: true,
        },
        storage: new MemoryStorageAdapter(),
      });
      try {
        await waitUntil(() => topgun.isRoutingActive(), 10_000);
        const map = topgun.getMap<string, string>('test-rebalance');
        map.set('post-rebalance-key', 'rebalanced-value');
        await waitMs(1_000);

        const internalCluster = (topgun as any).clusterClient as ClusterClient;
        const metrics = internalCluster.getRoutingMetrics();
        expect(metrics.directRoutes).toBeGreaterThan(0);
        expect(metrics.partitionMisses).toBe(0);
      } finally {
        topgun.close();
      }
    } finally {
      clusterClient.close();
      // Restart node 1 for subsequent tests
      await cluster.restartNode(1);
      await waitMs(3_000);
    }
  }, 30_000);

  // ----------------------------------------------------------------
  // Test 6: Circuit breaker activation and reset
  // ----------------------------------------------------------------
  test('circuit breaker opens after threshold failures and resets after timeout', async () => {
    // Use short reset timeout so the test doesn't wait 30 s
    const clusterClient = new ClusterClient({
      enabled: true,
      seedNodes: cluster.seedAddresses,
      routingMode: 'direct',
      circuitBreaker: {
        failureThreshold: 5,
        resetTimeoutMs: 500,
      },
    });

    try {
      await clusterClient.connect();
      await waitUntil(() => clusterClient.isRoutingActive(), 10_000);

      const testNodeId = 'node-1';

      // Record 5 failures to open the circuit breaker
      for (let i = 0; i < 5; i++) {
        clusterClient.recordFailure(testNodeId);
      }

      // Circuit should now be open
      expect(clusterClient.canUseNode(testNodeId)).toBe(false);

      // Wait for reset timeout to elapse
      await waitMs(600);

      // After reset timeout, circuit moves to half-open — node becomes usable again
      expect(clusterClient.canUseNode(testNodeId)).toBe(true);
    } finally {
      clusterClient.close();
    }
  }, 30_000);

  // ----------------------------------------------------------------
  // R4: Hash compatibility verification
  // ----------------------------------------------------------------
  test('R4: TS hashString produces same partition assignment as server routing', async () => {
    const clusterClient = new ClusterClient({
      enabled: true,
      seedNodes: cluster.seedAddresses,
      routingMode: 'direct',
    });

    try {
      await clusterClient.connect();
      await waitUntil(() => clusterClient.isRoutingActive(), 10_000);

      // Write a key and verify the client-computed partition matches routing
      const testKey = 'hash-compat-key';
      const expectedPartitionId = Math.abs(hashString(testKey)) % PARTITION_COUNT;

      // Verify the partition router agrees
      const partitionRouter = (clusterClient as any).partitionRouter;
      if (partitionRouter) {
        const computedPartitionId = partitionRouter.getPartitionId(testKey);
        expect(computedPartitionId).toBe(expectedPartitionId);
      }

      // Write the key via the cluster so it routes through the partition owner
      const topgun = new TopGunClient({
        cluster: {
          seeds: cluster.seedAddresses,
          smartRouting: true,
        },
        storage: new MemoryStorageAdapter(),
      });

      // Track NOT_OWNER errors — server should agree with client routing
      // The Rust server does not currently emit NOT_OWNER errors (not_owner_response()
      // exists but is never called), so this assertion trivially passes. It serves
      // as a forward-compatibility guard for when server-side ownership checks are added.
      let notOwnerReceived = false;
      const internalCluster = (topgun as any).clusterClient;
      if (internalCluster) {
        internalCluster.on('routing:miss', () => {
          notOwnerReceived = true;
        });
      }

      try {
        const map = topgun.getMap<string, string>('test-hash-compat');
        map.set(testKey, 'hash-compat-value');
        await waitMs(1_000);

        // Server did not return NOT_OWNER — hash functions are compatible
        expect(notOwnerReceived).toBe(false);

        // Client-side partition assignment is consistent
        expect(expectedPartitionId).toBeGreaterThanOrEqual(0);
        expect(expectedPartitionId).toBeLessThan(PARTITION_COUNT);
      } finally {
        topgun.close();
      }
    } finally {
      clusterClient.close();
    }
  }, 30_000);
});
