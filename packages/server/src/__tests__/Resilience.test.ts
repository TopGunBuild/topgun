import { ServerCoordinator, ServerFactory } from '../';
import { ChaosProxy } from './utils/ChaosProxy';
import { SyncEngine, SingleServerProvider } from '@topgunbuild/client';
import { MemoryStorageAdapter } from './utils/MemoryStorageAdapter';
import { waitForAuthReady, waitForConvergence, waitForConnection } from './utils/test-helpers';
import { LWWMap } from '@topgunbuild/core';
import * as jwt from 'jsonwebtoken';

const JWT_SECRET = 'topgun-secret-dev';

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
    // 1. Start Server with dynamic ports
    server = ServerFactory.create({
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

  afterEach(() => {
      // Close the current client references (the recreated ones from Phase 4,
      // since the originals were already closed in Phase 2)
      if (clientA) {
          clientA.close();
      }
      if (clientB) {
          clientB.close();
      }
  });

  test('Split-Brain Recovery: Eventual Consistency after Network Isolation', async () => {
    // Phase 1: Connect both clients, authenticate, wait for CONNECTED state, register maps
    storageA = new MemoryStorageAdapter();
    storageB = new MemoryStorageAdapter();

    clientA = new SyncEngine({
        nodeId: 'client-A',
        connectionProvider: new SingleServerProvider({ url: `ws://localhost:${proxyPort}` }),
        storageAdapter: storageA,
        reconnectInterval: 100
    });

    clientB = new SyncEngine({
        nodeId: 'client-B',
        connectionProvider: new SingleServerProvider({ url: `ws://localhost:${proxyPort}` }),
        storageAdapter: storageB,
        reconnectInterval: 100
    });

    await waitForAuthReady(clientA);
    clientA.setAuthToken(tokenA);

    await waitForAuthReady(clientB);
    clientB.setAuthToken(tokenB);

    const mapA = new LWWMap(clientA.getHLC());
    const mapB = new LWWMap(clientB.getHLC());

    clientA.registerMap('shared-data', mapA);
    clientB.registerMap('shared-data', mapB);

    await waitForConnection(clientA, 'CONNECTED', 5000);
    await waitForConnection(clientB, 'CONNECTED', 5000);

    // Phase 2: Close both clients to simulate going offline
    clientA.close();
    clientB.close();

    // Phase 3: Perform offline writes on the SAME LWWMap instances
    // Since SyncEngine is closed, we must populate both the map (via map.set())
    // AND the storage adapter oplog (via storageAdapter.appendOpLog()) so the
    // new SyncEngine can send the operations to the server via OP_BATCH on reconnect.

    // Client A writes key1 = 'ValueA'
    const recordA = mapA.set('key1', 'ValueA');
    await storageA.appendOpLog({
      mapName: 'shared-data',
      opType: 'PUT',
      key: 'key1',
      record: recordA,
      timestamp: recordA.timestamp,
      synced: 0,
    } as any);

    // 20ms delay so Client B's write has a later HLC timestamp (deterministic LWW winner)
    await new Promise(r => setTimeout(r, 20));

    // Client B writes key1 = 'ValueB' (this should win due to later timestamp)
    const recordB = mapB.set('key1', 'ValueB');
    await storageB.appendOpLog({
      mapName: 'shared-data',
      opType: 'PUT',
      key: 'key1',
      record: recordB,
      timestamp: recordB.timestamp,
      synced: 0,
    } as any);

    // Independent writes
    const recordA2 = mapA.set('keyA', 'OnlyA');
    await storageA.appendOpLog({
      mapName: 'shared-data',
      opType: 'PUT',
      key: 'keyA',
      record: recordA2,
      timestamp: recordA2.timestamp,
      synced: 0,
    } as any);

    const recordB2 = mapB.set('keyB', 'OnlyB');
    await storageB.appendOpLog({
      mapName: 'shared-data',
      opType: 'PUT',
      key: 'keyB',
      record: recordB2,
      timestamp: recordB2.timestamp,
      synced: 0,
    } as any);

    // Verify local state is divergent
    expect(mapA.get('key1')).toBe('ValueA');
    expect(mapB.get('key1')).toBe('ValueB');
    expect(mapA.get('keyB')).toBeUndefined();
    expect(mapB.get('keyA')).toBeUndefined();

    // Phase 4: Create new SyncEngine instances with the same MemoryStorageAdapter
    // instances to preserve the oplog entries from Phase 3
    clientA = new SyncEngine({
        nodeId: 'client-A',
        connectionProvider: new SingleServerProvider({ url: `ws://localhost:${proxyPort}` }),
        storageAdapter: storageA,
        reconnectInterval: 100
    });

    clientB = new SyncEngine({
        nodeId: 'client-B',
        connectionProvider: new SingleServerProvider({ url: `ws://localhost:${proxyPort}` }),
        storageAdapter: storageB,
        reconnectInterval: 100
    });

    // Phase 5: Register the same LWWMap instances on the new clients, authenticate,
    // and wait for CONNECTED state
    clientA.registerMap('shared-data', mapA);
    clientB.registerMap('shared-data', mapB);

    await waitForAuthReady(clientA);
    clientA.setAuthToken(tokenA);

    await waitForAuthReady(clientB);
    clientB.setAuthToken(tokenB);

    await waitForConnection(clientA, 'CONNECTED', 10000);
    await waitForConnection(clientB, 'CONNECTED', 10000);

    // Phase 6: Wait for convergence, assert LWW resolution and bidirectional propagation
    await waitForConvergence(mapA, mapB, 'key1', 'ValueB', 10000);
    await waitForConvergence(mapA, mapB, 'keyA', 'OnlyA', 10000);
    await waitForConvergence(mapA, mapB, 'keyB', 'OnlyB', 10000);

    // Assertions
    expect(mapA.get('key1')).toBe('ValueB'); // B was later, wins via LWW
    expect(mapB.get('key1')).toBe('ValueB');

    expect(mapA.get('keyA')).toBe('OnlyA');
    expect(mapB.get('keyA')).toBe('OnlyA'); // Synced to B

    expect(mapA.get('keyB')).toBe('OnlyB'); // Synced to A
    expect(mapB.get('keyB')).toBe('OnlyB');

    // Check Server State
    const serverMap = server.getMap('shared-data') as LWWMap<string, any>;
    expect(serverMap.get('key1')).toBe('ValueB');
    expect(serverMap.get('keyA')).toBe('OnlyA');
  }, 30000);
});
