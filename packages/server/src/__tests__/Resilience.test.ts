import { ServerCoordinator } from '../ServerCoordinator';
import { ChaosProxy } from './utils/ChaosProxy';
import { SyncEngine } from '@topgunbuild/client';
import { MemoryStorageAdapter } from './utils/MemoryStorageAdapter';
import { LWWMap } from '@topgunbuild/core';
import * as jwt from 'jsonwebtoken';

const JWT_SECRET = 'topgun-secret-dev';

// Retry flaky resilience tests up to 3 times
jest.retryTimes(3);

describe('Resilience & Chaos Testing', () => {
  let server: ServerCoordinator;
  let proxy: ChaosProxy;
  let clientA: SyncEngine;
  let clientB: SyncEngine;
  let storageA: MemoryStorageAdapter;
  let storageB: MemoryStorageAdapter;
  let tokenA: string;
  let tokenB: string;
  let proxyPort: number;

  beforeAll(async () => {
    jest.setTimeout(30000);

    // 1. Start Server with dynamic ports
    server = new ServerCoordinator({
      port: 0,
      nodeId: 'server-1',
      host: 'localhost',
      clusterPort: 0,
      peers: []
    });

    // Wait for server to be ready
    await server.ready();

    // 2. Start Chaos Proxy with dynamic port
    proxy = new ChaosProxy(0, `ws://localhost:${server.port}`);
    proxyPort = await proxy.ready();

    // 3. Generate Tokens
    tokenA = jwt.sign({ sub: 'client-A', roles: ['USER', 'ADMIN'] }, JWT_SECRET);
    tokenB = jwt.sign({ sub: 'client-B', roles: ['USER', 'ADMIN'] }, JWT_SECRET);
  });

  afterAll(async () => {
    proxy.stop();
    await server.shutdown();
  });

  beforeEach(() => {
      // Reset configs
      proxy.updateConfig({
          latencyMs: 0,
          jitterMs: 0,
          flakeRate: 0,
          isSilent: false
      });
  });

  test('Split-Brain Recovery: Eventual Consistency after Network Isolation', async () => {
    // Setup Clients
    storageA = new MemoryStorageAdapter();
    storageB = new MemoryStorageAdapter();

    clientA = new SyncEngine({
        nodeId: 'client-A',
        serverUrl: `ws://localhost:${proxyPort}`,
        storageAdapter: storageA,
        reconnectInterval: 100
    });
    clientA.setAuthToken(tokenA);

    clientB = new SyncEngine({
        nodeId: 'client-B',
        serverUrl: `ws://localhost:${proxyPort}`,
        storageAdapter: storageB,
        reconnectInterval: 100
    });
    clientB.setAuthToken(tokenB);

    // Setup Maps
    const mapA = new LWWMap(clientA.getHLC());
    const mapB = new LWWMap(clientB.getHLC());

    clientA.registerMap('shared-data', mapA);
    clientB.registerMap('shared-data', mapB);

    // Wait for initial auth/connection
    await new Promise(r => setTimeout(r, 500));

    // 1. Simulate Network Partition (Disconnect both)
    console.log('--- SIMULATING NETWORK PARTITION ---');
    proxy.disconnectAll();
    // Enable silent mode so if they reconnect immediately, they get ignored
    proxy.updateConfig({ isSilent: true });

    // 2. Conflicting Writes while offline
    console.log('--- PERFORMING OFFLINE WRITES ---');

    // Client A writes Key1 = 'ValueA'
    // We must use map.set AND recordOperation to ensure it goes to sync engine
    const recordA = mapA.set('key1', 'ValueA');
    await clientA.recordOperation('shared-data', 'PUT', 'key1', { record: recordA, timestamp: recordA.timestamp });

    // Client B writes Key1 = 'ValueB'
    // Wait a tiny bit to ensure timestamp difference if we want deterministic winner,
    // though HLC handles unique node IDs so tie-breaking works.
    await new Promise(r => setTimeout(r, 20));
    const recordB = mapB.set('key1', 'ValueB');
    await clientB.recordOperation('shared-data', 'PUT', 'key1', { record: recordB, timestamp: recordB.timestamp });

    // Independent writes
    const recordA2 = mapA.set('keyA', 'OnlyA');
    await clientA.recordOperation('shared-data', 'PUT', 'keyA', { record: recordA2, timestamp: recordA2.timestamp });

    const recordB2 = mapB.set('keyB', 'OnlyB');
    await clientB.recordOperation('shared-data', 'PUT', 'keyB', { record: recordB2, timestamp: recordB2.timestamp });

    // Verify local state is divergent
    expect(mapA.get('key1')).toBe('ValueA');
    expect(mapB.get('key1')).toBe('ValueB');
    expect(mapA.get('keyB')).toBeUndefined();
    expect(mapB.get('keyA')).toBeUndefined();

    // 3. Restore Network
    console.log('--- RESTORING NETWORK ---');
    proxy.updateConfig({ isSilent: false });
    // Force reconnect - clients may be connected but stuck waiting for AUTH_ACK
    // that was silently dropped. Disconnecting forces a fresh handshake.
    proxy.disconnectAll();
    // Give clients time to reconnect and sync
    await new Promise(r => setTimeout(r, 500));

    // 4. Wait for Convergence
    console.log('--- WAITING FOR CONVERGENCE ---');
    await waitForConvergence(mapA, mapB, 'key1', 'ValueB', 10000);

    // 5. Assertions
    expect(mapA.get('key1')).toBe('ValueB'); // B was later
    expect(mapB.get('key1')).toBe('ValueB');

    expect(mapA.get('keyA')).toBe('OnlyA');
    expect(mapB.get('keyA')).toBe('OnlyA'); // Synced to B

    expect(mapA.get('keyB')).toBe('OnlyB'); // Synced to A
    expect(mapB.get('keyB')).toBe('OnlyB');

    // Check Server State
    const serverMap = server.getMap('shared-data') as LWWMap<string, any>;
    // Wait for server to have data too (implicit if clients synced via server)
    // Note: serverMap.get returns the value directly, getRecord returns the wrapper
    expect(serverMap.get('key1')).toBe('ValueB');
    expect(serverMap.get('keyA')).toBe('OnlyA');
  }, 30000);
});

async function waitForConvergence(mapA: LWWMap<any, any>, mapB: LWWMap<any, any>, key: string, expectedValue: any, timeout: number) {
    const start = Date.now();
    while (Date.now() - start < timeout) {
        const valA = mapA.get(key);
        const valB = mapB.get(key);
        if (valA === expectedValue && valB === expectedValue) {
            return;
        }
        await new Promise(r => setTimeout(r, 100));
    }
    // If failed, throw with details
    throw new Error(`Convergence failed after ${timeout}ms.
      A: ${JSON.stringify(mapA.get(key))},
      B: ${JSON.stringify(mapB.get(key))},
      Expected: ${expectedValue}`);
}
