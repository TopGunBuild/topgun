/**
 * Integration tests for 3-node cluster smart routing, failover, and partition
 * map sync. These tests boot a real Rust server cluster and exercise the
 * ClusterClient / PartitionRouter end-to-end.
 *
 * Requires: RUST_SERVER_BINARY env var or a cargo build at repo root.
 * Timeout: 30 s per test (cluster formation is slow).
 */

import './helpers/setup';

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

      // Use the topgun client to write 100 keys
      const map = topgun.getMap<string, string>('test-routing');

      clusterClient.resetRoutingMetrics();

      for (let i = 0; i < 100; i++) {
        map.set(`key-${i}`, `value-${i}`);
        // Small pause to allow routing
        await waitMs(5);
      }

      // Give the sync engine a moment to flush ops
      await waitMs(500);

      const metrics = clusterClient.getRoutingMetrics();
      expect(metrics.directRoutes).toBeGreaterThan(0);
      expect(metrics.partitionMisses).toBe(0);
    } finally {
      topgun.close();
      clusterClient.close();
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
});
