import { ServerCoordinator } from '../ServerCoordinator';
import { SyncEngine } from '@topgunbuild/client';
import { MemoryStorageAdapter } from './utils/MemoryStorageAdapter';
import { waitForAuthReady } from './utils/waitForAuthReady';
import { LWWMap } from '@topgunbuild/core';
import * as jwt from 'jsonwebtoken';

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
    server = new ServerCoordinator({
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
        serverUrl: `ws://localhost:${serverPort}`,
        storageAdapter: storageA
    });

    // Wait for WebSocket to be ready before setting auth token
    await waitForAuthReady(clientA);
    clientA.setAuthToken(tokenA);
    const mapA = new LWWMap(clientA.getHLC());
    clientA.registerMap('gc-test-map', mapA);

    // Wait for connect
    await new Promise(r => setTimeout(r, 200));

    // Create and sync data
    const rec = mapA.set('key1', 'val1');
    await clientA.recordOperation('gc-test-map', 'PUT', 'key1', { record: rec, timestamp: rec.timestamp });

    // Wait for sync
    await new Promise(r => setTimeout(r, 200));

    // 2. Delete data (create tombstone)
    const tombstone = mapA.remove('key1');
    await clientA.recordOperation('gc-test-map', 'REMOVE', 'key1', { record: tombstone, timestamp: tombstone.timestamp });

    // Wait for sync of tombstone
    await new Promise(r => setTimeout(r, 200));

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
    (server as any).performGarbageCollection(safeTimestamp);

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
        serverUrl: `ws://localhost:${serverPort}`,
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

    // Wait for connection and handshake
    await new Promise(r => setTimeout(r, 1000));

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
    (server as any).performGarbageCollection(olderThan);

    // 4. Verify it became a tombstone
    const updatedRecord = map.getRecord('tempKey');
    expect(updatedRecord).toBeDefined();
    expect(updatedRecord?.value).toBeNull();
    // Timestamp of tombstone should be expiration time
    expect(updatedRecord?.timestamp.millis).toBe(record.timestamp.millis + 1000);
  });
});
