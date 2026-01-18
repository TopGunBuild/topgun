import { ServerCoordinator } from '../ServerCoordinator';
import { WebSocket } from 'ws';
import { LWWMap, HLC, Timestamp } from '@topgunbuild/core';
import * as jwt from 'jsonwebtoken';

const JWT_SECRET = 'topgun-secret-dev';

describe('Distributed Garbage Collection Consensus', () => {
  let node1: ServerCoordinator;
  let node2: ServerCoordinator;
  let node3: ServerCoordinator;
  let originalDateNow: () => number;

  // Helper to create a mock client connection on a node
  function mockClient(node: ServerCoordinator, clientId: string, lastActiveHlc?: Timestamp) {
    const mockConn = {
      id: clientId,
      socket: { 
        send: jest.fn(), 
        readyState: WebSocket.OPEN,
        close: jest.fn()
      } as any,
      isAuthenticated: true,
      subscriptions: new Set(),
      principal: { userId: clientId, roles: ['USER'] },
      lastActiveHlc: lastActiveHlc || (node as any).hlc.now(),
      lastPingReceived: Date.now(),
    };
    (node as any).connectionManager.getClients().set(clientId, mockConn);
    return mockConn;
  }

  // Helper to trigger GC cycle manually
  async function triggerConsensusCycle() {
    // We manually invoke reportLocalHlc on all nodes to simulate the interval firing
    (node1 as any).reportLocalHlc();
    (node2 as any).reportLocalHlc();
    (node3 as any).reportLocalHlc();
    
    // Give time for messages to exchange and commit to happen
    await new Promise(resolve => setTimeout(resolve, 500));
  }

  beforeAll(async () => {
    originalDateNow = Date.now;

    // Setup 3-node cluster
    // Node 1 (Leader - lowest ID)
    node1 = new ServerCoordinator({
      port: 0,
      nodeId: 'node-1',
      host: 'localhost',
      clusterPort: 0,
      peers: []
    });
    await node1.ready();

    // Node 2
    node2 = new ServerCoordinator({
      port: 0,
      nodeId: 'node-2',
      host: 'localhost',
      clusterPort: 0,
      peers: [`localhost:${node1.clusterPort}`]
    });
    await node2.ready();

    // Node 3
    node3 = new ServerCoordinator({
      port: 0,
      nodeId: 'node-3',
      host: 'localhost',
      clusterPort: 0,
      peers: [`localhost:${node1.clusterPort}`]
    });
    await node3.ready();

    // Wait for cluster formation
    const start = Date.now();
    while (Date.now() - start < 5000) {
        const m1 = (node1 as any).cluster.getMembers();
        const m2 = (node2 as any).cluster.getMembers();
        const m3 = (node3 as any).cluster.getMembers();
        if (m1.length === 3 && m2.length === 3 && m3.length === 3) break;
        await new Promise(r => setTimeout(r, 100));
    }
  }, 20000);

  afterAll(async () => {
    await node1.shutdown();
    await node2.shutdown();
    await node3.shutdown();
    Date.now = originalDateNow;
    // Cleanup ports
    await new Promise(r => setTimeout(r, 200));
  });

  beforeEach(() => {
    Date.now = originalDateNow;
  });

  test('GC Consensus: Blocks pruning if a node has lagging clients', async () => {
    // 1. Create Data & Tombstones
    const mapName = 'dist-gc-map';
    const key = 'zombie-key';
    
    // Write LWW Record
    const rec = { value: 'test', timestamp: (node1 as any).hlc.now() };
    (node1 as any).processLocalOp({
        mapName,
        key,
        record: rec,
        opType: 'PUT'
    }, false);

    await new Promise(r => setTimeout(r, 200)); // Propagate

    // Verify write
    expect((node2.getMap(mapName) as LWWMap<any, any>).getRecord(key)?.value).toBe('test');

    // Delete (Create Tombstone)
    const tombstone = { value: null, timestamp: (node1 as any).hlc.now() };
    (node1 as any).processLocalOp({
        mapName,
        key,
        record: tombstone,
        opType: 'REMOVE'
    }, false);

    await new Promise(r => setTimeout(r, 200)); // Propagate

    // Verify tombstone
    expect((node1.getMap(mapName) as LWWMap<any, any>).getRecord(key)?.value).toBeNull();
    expect((node2.getMap(mapName) as LWWMap<any, any>).getRecord(key)?.value).toBeNull();
    expect((node3.getMap(mapName) as LWWMap<any, any>).getRecord(key)?.value).toBeNull();

    // 2. Fast Forward Time > GC_AGE
    const GC_AGE_MS = 30 * 24 * 60 * 60 * 1000;
    const futureTime = Date.now() + GC_AGE_MS + 10000; // +10s buffer
    Date.now = jest.fn(() => futureTime);

    // 3. Simulate Lagging Client on Node 3
    // This client was last seen BEFORE the tombstone creation
    // So it might re-introduce the item if we prune.
    // Thus, GC should be blocked to the time of this client.
    const laggingTime = originalDateNow() - 1000; // 1s before start
    const laggingTs: Timestamp = { millis: laggingTime, counter: 0, nodeId: 'client-lag' };
    
    mockClient(node3, 'client-lag', laggingTs);

    // 4. Trigger Consensus
    await triggerConsensusCycle();

    // 5. Verify Tombstone still exists (Pruning Blocked)
    // Because Node 3 reported a minHlc of `laggingTime`, which is ~30 days ago relative to `futureTime`.
    // The consensus safe time would be `laggingTime - GC_AGE`, which is ridiculously old.
    // The tombstone (created at `originalDateNow`) is NEWER than `laggingTime - GC_AGE`.
    // So it should NOT be pruned.
    
    expect((node1.getMap(mapName) as LWWMap<any, any>).getRecord(key)).not.toBeUndefined();
    expect((node2.getMap(mapName) as LWWMap<any, any>).getRecord(key)).not.toBeUndefined();

    // 6. Update Lagging Client (Client catches up)
    const caughtUpTime = futureTime;
    const clientConn = (node3 as any).connectionManager.getClients().get('client-lag');
    clientConn.lastActiveHlc = { millis: caughtUpTime, counter: 0, nodeId: 'client-lag' };

    // 7. Trigger Consensus Again
    await triggerConsensusCycle();

    // 8. Verify Tombstone is pruned
    // Now all nodes report ~futureTime.
    // Safe GC Time = futureTime - GC_AGE = originalDateNow + 10s.
    // Tombstone was created at originalDateNow.
    // Tombstone is OLDER than Safe GC Time.
    // Should be pruned.

    expect((node1.getMap(mapName) as LWWMap<any, any>).getRecord(key)).toBeUndefined();
    expect((node2.getMap(mapName) as LWWMap<any, any>).getRecord(key)).toBeUndefined();
    expect((node3.getMap(mapName) as LWWMap<any, any>).getRecord(key)).toBeUndefined();
  });
});

