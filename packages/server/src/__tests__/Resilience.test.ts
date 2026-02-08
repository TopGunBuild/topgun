import { ServerCoordinator, ServerFactory } from '../';
import { ChaosProxy } from './utils/ChaosProxy';
import { SyncEngine, SingleServerProvider } from '@topgunbuild/client';
import { MemoryStorageAdapter } from './utils/MemoryStorageAdapter';
import { waitForAuthReady, waitForConvergence, pollUntil } from './utils/test-helpers';
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
      // Close client connections to prevent "Jest did not exit" warnings
      if (clientA) {
          clientA.close();
      }
      if (clientB) {
          clientB.close();
      }
  });

  test('Split-Brain Recovery: Eventual Consistency after Network Isolation', async () => {
    // 1. FIRST: Simulate Network Partition BEFORE clients connect
    // This ensures clients connect but cannot communicate through proxy
    console.log('--- SIMULATING NETWORK PARTITION ---');
    proxy.updateConfig({ isSilent: true });

    // Setup Clients
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

    // Wait for WebSocket to be ready before setting auth token
    await waitForAuthReady(clientA);
    clientA.setAuthToken(tokenA);

    await waitForAuthReady(clientB);
    clientB.setAuthToken(tokenB);

    // Setup Maps
    const mapA = new LWWMap(clientA.getHLC());
    const mapB = new LWWMap(clientB.getHLC());

    clientA.registerMap('shared-data', mapA);
    clientB.registerMap('shared-data', mapB);

    // Wait for both clients to reach AUTHENTICATING state (WS open, AUTH sent, but
    // AUTH_ACK silently dropped by proxy so they stay stuck in AUTHENTICATING)
    await pollUntil(
      () => {
        const stateA = clientA.getConnectionState();
        const stateB = clientB.getConnectionState();
        return stateA !== 'INITIAL' && stateA !== 'DISCONNECTED' &&
               stateB !== 'INITIAL' && stateB !== 'DISCONNECTED';
      },
      { timeoutMs: 5000, intervalMs: 50, description: 'clients to reach authenticating state through silent proxy' }
    );

    // 2. Conflicting Writes while "offline" (silent mode)
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

    // Verify local state is divergent (data not synced during silent mode)
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
    // Wait for clients to reconnect and complete auth handshake
    await pollUntil(
      () => clientA.getConnectionState() === 'CONNECTED' && clientB.getConnectionState() === 'CONNECTED',
      { timeoutMs: 10000, intervalMs: 100, description: 'clients to reconnect after proxy restored' }
    );

    // 4. Wait for Convergence on all keys
    console.log('--- WAITING FOR CONVERGENCE ---');
    await waitForConvergence(mapA, mapB, 'key1', 'ValueB', 10000);
    await waitForConvergence(mapA, mapB, 'keyA', 'OnlyA', 10000);
    await waitForConvergence(mapA, mapB, 'keyB', 'OnlyB', 10000);

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
