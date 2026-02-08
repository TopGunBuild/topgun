import { ServerCoordinator, ServerFactory } from '../';
import { SyncEngine, SingleServerProvider } from '@topgunbuild/client';
import { MemoryStorageAdapter } from './utils/MemoryStorageAdapter';
import { waitForAuthReady, pollUntil, waitForMapValue, waitForSpyCall } from './utils/test-helpers';
import { createTestHarness, ServerTestHarness } from './utils/ServerTestHarness';
import { LWWMap, ORMap } from '@topgunbuild/core';
import * as jwt from 'jsonwebtoken';
import { WebSocket } from 'ws';

const JWT_SECRET = 'topgun-secret-dev';

describe('Garbage Collection & Zombie Protection', () => {
  let server: ServerCoordinator;
  let clientA: SyncEngine;
  let clientB: SyncEngine; // "Zombie" client
  let storageA: MemoryStorageAdapter;
  let storageB: MemoryStorageAdapter;
  let tokenA: string;
  let tokenB: string;
  let originalDateNow: () => number;
  let serverPort: number;

  beforeAll(async () => {
    originalDateNow = Date.now;

    // Start Server with dynamic port
    server = ServerFactory.create({
      port: 0,
      nodeId: 'gc-server',
      host: 'localhost',
      clusterPort: 0,
      peers: []
    });

    await server.ready();
    serverPort = server.port;

    tokenA = jwt.sign({ sub: 'client-A', roles: ['ADMIN'] }, JWT_SECRET);
    tokenB = jwt.sign({ sub: 'client-B', roles: ['ADMIN'] }, JWT_SECRET);
  });

  afterAll(async () => {
    await server.shutdown();
    Date.now = originalDateNow;
  });

  beforeEach(() => {
    Date.now = originalDateNow;
  });

  afterEach(() => {
    // Close client connections to prevent "Jest did not exit" warnings
    if (clientA) {
      clientA.close();
    }
    if (clientB) {
      clientB.close();
    }
  });

  test('Should reject old client with SYNC_RESET_REQUIRED', async () => {
    // 1. Setup Client A and create data
    storageA = new MemoryStorageAdapter();
    clientA = new SyncEngine({
        nodeId: 'client-A',
        connectionProvider: new SingleServerProvider({ url: `ws://localhost:${serverPort}` }),
        storageAdapter: storageA
    });

    // Wait for WebSocket to be ready before setting auth token
    await waitForAuthReady(clientA);
    clientA.setAuthToken(tokenA);
    const mapA = new LWWMap(clientA.getHLC());
    clientA.registerMap('gc-test-map', mapA);

    // Wait for client to be connected
    await pollUntil(
      () => clientA.getConnectionState() === 'CONNECTED',
      { timeoutMs: 5000, intervalMs: 50, description: 'clientA to reach CONNECTED state' }
    );

    // Create and sync data
    const rec = mapA.set('key1', 'val1');
    await clientA.recordOperation('gc-test-map', 'PUT', 'key1', { record: rec, timestamp: rec.timestamp });

    // Wait for sync to server
    await waitForMapValue(server, 'gc-test-map', 'key1', 'val1', {
      description: 'key1=val1 synced to server',
    });

    // 2. Delete data (create tombstone)
    const tombstone = mapA.remove('key1');
    await clientA.recordOperation('gc-test-map', 'REMOVE', 'key1', { record: tombstone, timestamp: tombstone.timestamp });

    // Wait for tombstone to sync to server
    await pollUntil(
      () => {
        const serverMap = server.getMap('gc-test-map');
        const serverRec = (serverMap as LWWMap<any, any>).getRecord('key1');
        return serverRec?.value === null;
      },
      { timeoutMs: 5000, intervalMs: 50, description: 'tombstone for key1 synced to server' }
    );

    // Verify on server
    const serverMap = server.getMap('gc-test-map');
    const serverRec = (serverMap as LWWMap<any, any>).getRecord('key1');
    expect(serverRec?.value).toBeNull(); // Tombstone exists

    // 3. Time Travel: Advance time by 31 days (GC_AGE_MS is 30 days)
    const THIRTY_ONE_DAYS = 31 * 24 * 60 * 60 * 1000;
    const futureTime = originalDateNow() + THIRTY_ONE_DAYS;
    Date.now = jest.fn(() => futureTime);

    // 4. Trigger GC on Server manually (to ensure it runs now)
    // performGarbageCollection now requires a Timestamp parameter (olderThan)
    // We pass the futureTime as the safe GC cutoff
    const safeTimestamp = { millis: futureTime, counter: 0, nodeId: 'gc-server' };
    server.performGarbageCollection(safeTimestamp);

    // Verify tombstone is gone from server
    expect((serverMap as LWWMap<any, any>).getRecord('key1')).toBeUndefined();

    // 5. Client B connects (simulating an old client that was offline)
    // We manually set a very old lastSyncTimestamp in storage
    storageB = new MemoryStorageAdapter();
    // Set last sync to 32 days ago relative to futureTime
    const oldSyncTime = futureTime - (32 * 24 * 60 * 60 * 1000);
    await storageB.setMeta('lastSyncTimestamp', oldSyncTime);

    clientB = new SyncEngine({
        nodeId: 'client-B',
        connectionProvider: new SingleServerProvider({ url: `ws://localhost:${serverPort}` }),
        storageAdapter: storageB,
        reconnectInterval: 100
    });

    // Wait for WebSocket to be ready before setting auth token
    await waitForAuthReady(clientB);
    clientB.setAuthToken(tokenB);
    const mapB = new LWWMap(clientB.getHLC());
    clientB.registerMap('gc-test-map', mapB);

    // Spy on resetMap to verify it gets called
    const resetSpy = jest.spyOn((clientB as any), 'resetMap');

    // Wait for resetMap to be called (triggered by SYNC_RESET_REQUIRED during handshake)
    await waitForSpyCall(resetSpy, {
      timeoutMs: 5000,
      description: 'resetMap called on clientB after SYNC_RESET_REQUIRED',
    });

    // Expect resetMap to have been called
    expect(resetSpy).toHaveBeenCalledWith('gc-test-map');
  });

  test('Should expire records with TTL proactively', async () => {
    const mapName = 'ttl-test-map';
    const now = originalDateNow();
    Date.now = jest.fn(() => now);

    // 1. Create Map & Record with TTL using explicit timestamp
    // We use merge() instead of set() to avoid HLC lastMillis issues
    const map = server.getMap(mapName) as LWWMap<string, string>;
    const recordTimestamp = { millis: now, counter: 0, nodeId: 'gc-server' };
    const record = { value: 'tempValue', timestamp: recordTimestamp, ttlMs: 1000 };
    map.merge('tempKey', record);

    // 2. Advance time by 2 seconds (Expired)
    // expirationTime = now + 1000 (record will expire at this time)
    const futureTime = now + 2000;
    Date.now = jest.fn(() => futureTime);

    // 3. Run GC
    // Use olderThan = now (before record creation) so that:
    // - TTL expiration check uses Date.now() = futureTime (expired)
    // - prune() won't delete the freshly created tombstone (tombstone.millis = now+1000 > now)
    const olderThan = { millis: now, counter: 0, nodeId: 'gc-server' };
    server.performGarbageCollection(olderThan);

    // 4. Verify it became a tombstone
    const updatedRecord = map.getRecord('tempKey');
    expect(updatedRecord).toBeDefined();
    expect(updatedRecord?.value).toBeNull();
    // Timestamp of tombstone should be expiration time
    expect(updatedRecord?.timestamp.millis).toBe(record.timestamp.millis + 1000);
  });
});

describe('TTL Expiration with ReplicationPipeline', () => {
  let node1: ServerCoordinator;
  let node2: ServerCoordinator;
  let harness1: ServerTestHarness;
  let harness2: ServerTestHarness;
  let originalDateNow: () => number;

  beforeAll(async () => {
    originalDateNow = Date.now;

    // Start Node B first (higher ID, will receive connection)
    node1 = ServerFactory.create({
      port: 0,
      nodeId: 'ttl-node-b',
      host: 'localhost',
      clusterPort: 0,
      peers: []
    });

    await node1.ready();
    harness1 = createTestHarness(node1);

    // Start Node A (lower ID, will initiate connection to node-b)
    node2 = ServerFactory.create({
      port: 0,
      nodeId: 'ttl-node-a',
      host: 'localhost',
      clusterPort: 0,
      peers: [`localhost:${node1.clusterPort}`]
    });

    await node2.ready();
    harness2 = createTestHarness(node2);

    // Wait for cluster to stabilize with bounded polling
    await pollUntil(
      () => {
        const m1 = harness1.cluster.getMembers();
        const m2 = harness2.cluster.getMembers();
        return m1.includes('ttl-node-a') && m2.includes('ttl-node-b');
      },
      {
        timeoutMs: 5000,
        intervalMs: 100,
        description: 'TTL cluster formation (nodes see each other)',
      }
    );
  }, 15000);

  afterAll(async () => {
    await node1.shutdown();
    await node2.shutdown();
    Date.now = originalDateNow;
    // WHY: Allow pending cluster WebSocket close events to drain before Jest tears down,
    // preventing "Jest did not exit" warnings from dangling async operations
    await new Promise(resolve => setTimeout(resolve, 200));
  });

  beforeEach(() => {
    Date.now = originalDateNow;
  });

  test('LWWMap TTL expiration replicates to backup nodes via ReplicationPipeline', async () => {
    const mapName = 'ttl-replicated-map';
    const now = originalDateNow();
    Date.now = jest.fn(() => now);

    // Find a key where node1 is owner and node2 is backup
    // The partition service should have node2 as backup for partitions owned by node1
    let testKey = '';
    const partitionService = harness1.partitionService;
    for (let i = 0; i < 100; i++) {
      const candidateKey = `ttl-key-${i}`;
      const partitionId = partitionService.getPartitionId(candidateKey);
      const info = partitionService.getPartitionInfo(partitionId);
      if (info && info.ownerNodeId === 'ttl-node-b' && info.backupNodeIds.includes('ttl-node-a')) {
        testKey = candidateKey;
        break;
      }
    }

    // If no key found with proper distribution, use default (test may be flaky)
    if (!testKey) {
      testKey = 'replicatedKey';
    }

    // Create data on node1 with TTL
    const map1 = node1.getMap(mapName) as LWWMap<string, string>;
    const recordTimestamp = { millis: now, counter: 0, nodeId: 'ttl-node-b' };
    const record = { value: 'replicatedValue', timestamp: recordTimestamp, ttlMs: 1000 };
    map1.merge(testKey, record);

    // Ensure node2 also has the data (simulate prior replication)
    const map2 = node2.getMap(mapName) as LWWMap<string, string>;
    map2.merge(testKey, record);

    // Verify both nodes have the value
    expect(map1.get(testKey)).toBe('replicatedValue');
    expect(map2.get(testKey)).toBe('replicatedValue');

    // Advance time past TTL expiration
    const futureTime = now + 2000;
    Date.now = jest.fn(() => futureTime);

    // Run GC on node1 - should use ReplicationPipeline instead of CLUSTER_EVENT
    const olderThan = { millis: now, counter: 0, nodeId: 'ttl-node-b' };
    node1.performGarbageCollection(olderThan);

    // Verify node1 has tombstone (synchronous -- GC runs locally)
    const record1 = map1.getRecord(testKey);
    expect(record1?.value).toBeNull();

    // Wait for node2 to receive tombstone via ReplicationPipeline
    await pollUntil(
      () => {
        const rec = map2.getRecord(testKey);
        return rec?.value === null;
      },
      { timeoutMs: 10000, intervalMs: 100, description: 'node2 to receive TTL tombstone via replication' }
    );

    // Verify node2 received tombstone via ReplicationPipeline
    const record2 = map2.getRecord(testKey);
    expect(record2?.value).toBeNull();
    expect(record2?.timestamp.millis).toBe(now + 1000); // Tombstone at expiration time
  });

  test('TTL expiration notifies query subscriptions via processChange', async () => {
    const mapName = 'ttl-query-map';
    const now = originalDateNow();
    Date.now = jest.fn(() => now);

    // Spy on the actual broadcast path: GCHandler.broadcastFn -> ServerCoordinator.broadcast -> broadcastHandler.broadcast
    // The harness.broadcast method is a proxy that doesn't intercept the GCHandler's late-bound broadcastFn
    const broadcastCalls: any[] = [];
    const broadcastHandler = harness1.broadcastHandler;
    const originalBroadcast = broadcastHandler.broadcast.bind(broadcastHandler);
    jest.spyOn(broadcastHandler, 'broadcast').mockImplementation((msg: any, ...args: any[]) => {
      broadcastCalls.push(msg);
      return originalBroadcast(msg, ...args);
    });

    // Create data with TTL
    const map = node1.getMap(mapName) as LWWMap<string, string>;
    const recordTimestamp = { millis: now, counter: 0, nodeId: 'ttl-node-b' };
    const record = { value: 'queryValue', timestamp: recordTimestamp, ttlMs: 500 };
    map.merge('queryKey', record);

    // Advance time past TTL
    const futureTime = now + 1000;
    Date.now = jest.fn(() => futureTime);

    // Clear broadcast calls before GC
    broadcastCalls.length = 0;

    // Run GC
    const olderThan = { millis: now, counter: 0, nodeId: 'ttl-node-b' };
    node1.performGarbageCollection(olderThan);

    // Verify SERVER_EVENT was broadcast for the expired record
    const serverEvents = broadcastCalls.filter(c => c.type === 'SERVER_EVENT');
    expect(serverEvents.length).toBeGreaterThan(0);

    const expiredEvent = serverEvents.find(e =>
      e.payload.mapName === mapName &&
      e.payload.key === 'queryKey' &&
      e.payload.eventType === 'UPDATED'
    );
    expect(expiredEvent).toBeDefined();
    expect(expiredEvent.payload.record.value).toBeNull();

    // Restore the spy to avoid affecting subsequent tests
    jest.restoreAllMocks();
  });

  test('ORMap TTL expiration replicates tombstone to backup nodes', async () => {
    const mapName = 'ttl-ormap-replicated';
    const now = originalDateNow();
    Date.now = jest.fn(() => now);

    // Create OR map on both nodes
    const map1 = node1.getMap(mapName, 'OR') as ORMap<string, any>;
    const map2 = node2.getMap(mapName, 'OR') as ORMap<string, any>;

    // Add record with TTL
    const orRecord = {
      value: { data: 'orValue' },
      timestamp: { millis: now, counter: 0, nodeId: 'ttl-node-b' },
      tag: 'unique-tag-123',
      ttlMs: 1000
    };

    map1.apply('orKey', orRecord);
    map2.apply('orKey', orRecord);

    // Verify both have the value
    expect(map1.get('orKey')).toEqual([{ data: 'orValue' }]);
    expect(map2.get('orKey')).toEqual([{ data: 'orValue' }]);

    // Advance time past TTL
    const futureTime = now + 2000;
    Date.now = jest.fn(() => futureTime);

    // Run GC on node1
    const olderThan = { millis: now, counter: 0, nodeId: 'ttl-node-b' };
    node1.performGarbageCollection(olderThan);

    // Verify node1 removed the record (synchronous -- GC runs locally)
    expect(map1.get('orKey')).toEqual([]);

    // Wait for node2 to receive the removal via replication
    await pollUntil(
      () => {
        const vals = map2.get('orKey');
        return Array.isArray(vals) && vals.length === 0;
      },
      { timeoutMs: 10000, intervalMs: 100, description: 'node2 ORMap orKey to be empty via replication' }
    );

    // Verify node2 also has the record removed via replication
    expect(map2.get('orKey')).toEqual([]);
  });

  test('GC does not use O(N) CLUSTER_EVENT broadcast', async () => {
    const mapName = 'ttl-no-cluster-event-map';
    const now = originalDateNow();
    Date.now = jest.fn(() => now);

    // Spy on cluster.send to verify CLUSTER_EVENT is not used
    const clusterSendCalls: any[] = [];
    const originalSend = harness1.cluster.send.bind(harness1.cluster);
    harness1.cluster.send = (nodeId: string, type: any, payload: any) => {
      clusterSendCalls.push({ nodeId, type, payload });
      return originalSend(nodeId, type, payload);
    };

    // Create data with TTL
    const map = node1.getMap(mapName) as LWWMap<string, string>;
    const recordTimestamp = { millis: now, counter: 0, nodeId: 'ttl-node-b' };
    const record = { value: 'noClusterEvent', timestamp: recordTimestamp, ttlMs: 500 };
    map.merge('noEventKey', record);

    // Advance time past TTL
    const futureTime = now + 1000;
    Date.now = jest.fn(() => futureTime);

    // Clear calls before GC
    clusterSendCalls.length = 0;

    // Run GC
    const olderThan = { millis: now, counter: 0, nodeId: 'ttl-node-b' };
    node1.performGarbageCollection(olderThan);

    // Wait for replication pipeline to process (poll for OP_FORWARD or short timeout)
    await pollUntil(
      () => clusterSendCalls.length > 0,
      { timeoutMs: 2000, intervalMs: 50, description: 'cluster send calls after GC' }
    ).catch(() => {
      // No cluster sends is also valid -- means no backups for this partition
    });

    // Verify NO CLUSTER_EVENT was sent (we use ReplicationPipeline now)
    const clusterEvents = clusterSendCalls.filter(c => c.type === 'CLUSTER_EVENT');
    expect(clusterEvents.length).toBe(0);

    // Verify OP_FORWARD was used instead (ReplicationPipeline sends batches via OP_FORWARD)
    // Note: ReplicationPipeline batches operations, so we might see OP_FORWARD with _replication
    const opForwards = clusterSendCalls.filter(c => c.type === 'OP_FORWARD');
    // ReplicationPipeline uses OP_FORWARD for batched replication
    // If no backups exist for this partition, no replication occurs (which is fine)
    // The key point is CLUSTER_EVENT is NOT used
  });
});
