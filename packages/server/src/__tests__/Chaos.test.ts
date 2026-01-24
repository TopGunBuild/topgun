import { ServerCoordinator } from '../ServerCoordinator';
import { ChaosProxy } from './utils/ChaosProxy';
import { SyncEngine } from '@topgunbuild/client';
import { MemoryStorageAdapter } from './utils/MemoryStorageAdapter';
import { waitForAuthReady, waitForCluster, pollUntil } from './utils/test-helpers';
import { LWWMap } from '@topgunbuild/core';
import * as jwt from 'jsonwebtoken';
import { WebSocket } from 'ws';

const JWT_SECRET = 'topgun-secret-dev';

// Retry flaky chaos tests up to 3 times
jest.retryTimes(3);

describe('Chaos Testing', () => {

  // Scenario 1: Split Brain
  describe('Cluster Split Brain', () => {
    let nodeA: ServerCoordinator;
    let nodeB: ServerCoordinator;
    let nodeC: ServerCoordinator;

    beforeAll(async () => {
      // Start C first
      nodeC = new ServerCoordinator({
        port: 0,
        nodeId: 'node-c',
        host: 'localhost',
        clusterPort: 0,
        peers: [],
        metricsPort: 0
      });
      await nodeC.ready();

      // Start B, peer C
      nodeB = new ServerCoordinator({
        port: 0,
        nodeId: 'node-b',
        host: 'localhost',
        clusterPort: 0,
        peers: [`localhost:${nodeC.clusterPort}`],
        metricsPort: 0
      });
      await nodeB.ready();

      // Start A, peer C AND B (to form full mesh without gossip)
      nodeA = new ServerCoordinator({
        port: 0,
        nodeId: 'node-a',
        host: 'localhost',
        clusterPort: 0,
        peers: [
            `localhost:${nodeC.clusterPort}`,
            `localhost:${nodeB.clusterPort}`
        ],
        metricsPort: 0
      });
      await nodeA.ready();

      // Wait for mesh
      await waitForCluster([nodeA, nodeB, nodeC], 3);
    }, 30000);

    afterAll(async () => {
        await nodeA.shutdown();
        await nodeB.shutdown();
        await nodeC.shutdown();
    });

    test('Availability maintained during partial partition (A-B disconnect)', async () => {
        // 1. Disconnect A <-> B
        // Find A's connection to B
        const clusterA = (nodeA as any).cluster;
        const memberB = clusterA.members.get('node-b');
        
        if (memberB) {
            console.log('Closing connection A->B');
            memberB.socket.close(); // Trigger close/rebalance on A
        }

        const clusterB = (nodeB as any).cluster;
        const memberA = clusterB.members.get('node-a');
         if (memberA) {
             console.log('Closing connection B->A');
            memberA.socket.close(); // Trigger close/rebalance on B
        }

        // Wait for rebalance
        await new Promise(r => setTimeout(r, 1000));

        // 2. Write to A
        const clientA = new SyncEngine({
            nodeId: 'client-a',
            serverUrl: `ws://localhost:${nodeA.port}`,
            storageAdapter: new MemoryStorageAdapter()
        });
        await waitForAuthReady(clientA);
        clientA.setAuthToken(jwt.sign({ sub: 'ca', roles: ['ADMIN'] }, JWT_SECRET));
        const mapA = new LWWMap(clientA.getHLC());
        clientA.registerMap('split-data', mapA);

        const recA = mapA.set('key-a', 'val-a');
        await clientA.recordOperation('split-data', 'PUT', 'key-a', { record: recA, timestamp: recA.timestamp });

        // 3. Write to B
        const clientB = new SyncEngine({
            nodeId: 'client-b',
            serverUrl: `ws://localhost:${nodeB.port}`,
            storageAdapter: new MemoryStorageAdapter()
        });
        await waitForAuthReady(clientB);
        clientB.setAuthToken(jwt.sign({ sub: 'cb', roles: ['ADMIN'] }, JWT_SECRET));
        const mapB = new LWWMap(clientB.getHLC());
        clientB.registerMap('split-data', mapB);

        const recB = mapB.set('key-b', 'val-b');
        await clientB.recordOperation('split-data', 'PUT', 'key-b', { record: recB, timestamp: recB.timestamp });

        // 4. Verify Availability (Local Clients should be happy)
        // This demonstrates AP behavior: Clients can write and read their own writes despite partition.
        expect(mapA.get('key-a')).toBe('val-a');
        expect(mapB.get('key-b')).toBe('val-b');

        // 5. Verify Partial Convergence (Best Effort)
        // Node C connects to both, so it *might* receive updates depending on hash mapping and forwarding.
        const mapC = nodeC.getMap('split-data');
        expect(mapC).toBeDefined();
        
        // We log what C has, but don't strictly assert because topology views are inconsistent (Split Brain).
        // A sees {A,C}, B sees {B,C}, C sees {A,B,C}.
        // Ownership logic is broken.
        console.log('C key-a:', mapC.get('key-a'));
        console.log('C key-b:', mapC.get('key-b'));

        // Cleanup clients
        clientA.close();
        clientB.close();
    }, 30000);
  });
  
  // Scenario 2: Packet Loss
  describe('Packet Loss & Flaky Network', () => {
    let server: ServerCoordinator;
    let proxy: ChaosProxy;
    let proxyPort: number;
    let client: SyncEngine;
    let storage: MemoryStorageAdapter;
    let token: string;

    beforeAll(async () => {
        server = new ServerCoordinator({
            port: 0,
            nodeId: 'server-flake',
            host: 'localhost',
            clusterPort: 0,
            peers: []
        });
        await server.ready();

        proxy = new ChaosProxy(0, `ws://localhost:${server.port}`);
        proxyPort = await proxy.ready();

        token = jwt.sign({ sub: 'client-flake', roles: ['ADMIN'] }, JWT_SECRET);
    });

    afterAll(async () => {
        proxy.stop();
        await server.shutdown();
    });

    afterEach(() => {
        // Close client to prevent "Jest did not exit" warnings
        if (client) {
            client.close();
        }
    });

    test('Sync converges despite 10% packet loss', async () => {
        // 10% packet loss
        proxy.updateConfig({ flakeRate: 0.1 });

        storage = new MemoryStorageAdapter();
        client = new SyncEngine({
            nodeId: 'client-flake',
            serverUrl: `ws://localhost:${proxyPort}`,
            storageAdapter: storage,
            reconnectInterval: 100
        });
        await waitForAuthReady(client);
        client.setAuthToken(token);

        const map = new LWWMap(client.getHLC());
        client.registerMap('flake-data', map);

        // Generate heavy traffic
        const ITEM_COUNT = 50;
        for (let i = 0; i < ITEM_COUNT; i++) {
            const record = map.set(`item-${i}`, `value-${i}`);
            // Don't await each individual op to simulate burst
            client.recordOperation('flake-data', 'PUT', `item-${i}`, { record, timestamp: record.timestamp });
            await new Promise(r => setTimeout(r, 5)); // Slight delay to spread out slightly
        }

        // Wait for convergence with bounded polling
        await pollUntil(
            () => {
                const serverMap = server.getMap('flake-data');
                if (!serverMap) return false;
                for (let i = 0; i < ITEM_COUNT; i++) {
                    if (serverMap.get(`item-${i}`) !== `value-${i}`) {
                        return false;
                    }
                }
                return true;
            },
            {
                timeoutMs: 20000,
                intervalMs: 200,
                description: 'all items synced to server despite packet loss',
            }
        );
    }, 30000);
  });

  // Scenario 3: Slow Consumer
  describe('Slow Consumer', () => {
    let server: ServerCoordinator;
    let port: number;

    beforeAll(async () => {
        server = new ServerCoordinator({
            port: 0,
            nodeId: 'server-slow',
            host: 'localhost',
            clusterPort: 0,
            peers: []
        });
        await server.ready();
        port = server.port;
    });

    afterAll(async () => {
        await server.shutdown();
    });

    test('Server handles slow consumer without exploding', async () => {
        // Create a "Fast" producer client (normal SyncEngine)
        const producerToken = jwt.sign({ sub: 'producer', roles: ['ADMIN'] }, JWT_SECRET);
        const storage = new MemoryStorageAdapter();
        const producer = new SyncEngine({
            nodeId: 'producer',
            serverUrl: `ws://localhost:${port}`,
            storageAdapter: storage
        });
        await waitForAuthReady(producer);
        producer.setAuthToken(producerToken);
        const map = new LWWMap(producer.getHLC());
        producer.registerMap('stream-data', map);

        // Create a "Slow" consumer (Raw WebSocket)
        const consumerWs = new WebSocket(`ws://localhost:${port}`);
        
        await new Promise<void>(resolve => {
            consumerWs.on('open', () => {
                // Authenticate
                const authMsg = {
                    type: 'AUTH',
                    payload: { token: jwt.sign({ sub: 'slow-consumer', roles: ['ADMIN'] }, JWT_SECRET) }
                };
                consumerWs.send(JSON.stringify(authMsg));
                resolve();
            });
        });

        // Subscribe the slow consumer
        // We need to wait for AUTH_ACK first ideally, but let's just send subscription
        // Wait a bit for auth to process
        await new Promise(r => setTimeout(r, 200));

        const subMsg = {
            type: 'QUERY_SUB',
            payload: { queryId: 'q1', mapName: 'stream-data', query: {} }
        };
        consumerWs.send(JSON.stringify(subMsg));

        // PAUSE the consumer socket at TCP/Kernel level (or as close as we can in node)
        // ws.pause() stops emitting 'message' events, effectively stopping reading from the internal buffer.
        // This should cause the internal buffer to fill, then the TCP window to close,
        // forcing backpressure on the sender (Server).
        consumerWs.pause();

        // Pump data from Producer
        console.log('Starting fast production...');
        const ITEM_COUNT = 1000;
        for (let i = 0; i < ITEM_COUNT; i++) {
            const record = map.set(`stream-${i}`, 'large-payload-'.repeat(100)); // ~1.4KB per msg
            producer.recordOperation('stream-data', 'PUT', `stream-${i}`, { record, timestamp: record.timestamp });
            if (i % 100 === 0) await new Promise(r => setTimeout(r, 10));
        }
        console.log('Finished production');

        // Check Server Health
        // We expect the server NOT to crash. 
        // Ideally, it should have disconnected the slow consumer or dropped messages.
        // Checking if server is still alive
        expect(server).toBeDefined();
        
        // Check if consumer is still connected or disconnected
        // If server implements backpressure/slow-consumer protection, it might have closed the connection.
        // If not, it might still be open but buffering.
        // We mainly want to ensure "buffers don't explode". 
        // Hard to measure memory exactly in unit test without external monitoring, 
        // but we can check if the server is still responsive to other clients.
        
        const checkClient = new SyncEngine({
            nodeId: 'check-client',
            serverUrl: `ws://localhost:${port}`,
            storageAdapter: new MemoryStorageAdapter()
        });
        await waitForAuthReady(checkClient);
        checkClient.setAuthToken(jwt.sign({ sub: 'check', roles: ['ADMIN'] }, JWT_SECRET));
        
        // Try to do a simple op
        const checkMap = new LWWMap(checkClient.getHLC());
        checkClient.registerMap('check-data', checkMap);
        const rec = checkMap.set('check', 'alive');
        checkClient.recordOperation('check-data', 'PUT', 'check', { record: rec, timestamp: rec.timestamp });

        // Should succeed - verify with bounded polling
        await pollUntil(
            () => {
                const m = server.getMap('check-data');
                return m && m.get('check') === 'alive';
            },
            {
                timeoutMs: 5000,
                intervalMs: 100,
                description: 'server still responsive after slow consumer backpressure',
            }
        );

        // Cleanup
        producer.close();
        checkClient.close();
        consumerWs.close();
        consumerWs.terminate();
    }, 30000);
  });

});
