import { ServerCoordinator } from '../packages/server/src';
import { WebSocket } from 'ws';

// Node 1
const node1 = new ServerCoordinator({
    port: 8081,
    nodeId: 'node-1',
    host: 'localhost',
    clusterPort: 9001,
    peers: ['localhost:9002']
});

// Node 2
const node2 = new ServerCoordinator({
    port: 8082,
    nodeId: 'node-2',
    host: 'localhost',
    clusterPort: 9002,
    peers: ['localhost:9001']
});

console.log('Cluster nodes started on 8081 and 8082');

setTimeout(() => {
    console.log('\n--- Starting Sync Test (Pub/Sub) ---');

    // Mock Client 2 on Node 2
    let client2Received = false;
    const client2Mock = {
        id: 'mock-client-2',
        socket: {
            send: (msg: string) => {
                console.log(`[Client 2 Received] ${msg}`);
                if (msg.includes('user:1') && msg.includes('Maverick')) {
                    client2Received = true;
                }
            },
            readyState: WebSocket.OPEN
        } as any,
        isAuthenticated: true,
        subscriptions: new Set()
    };
    (node2 as any).clients.set(client2Mock.id, client2Mock);

    // Client 2 Subscribes to 'users'
    console.log('Client 2 subscribing to users...');
    (node2 as any).handleMessage(client2Mock, {
        type: 'QUERY_SUB',
        payload: {
            queryId: 'q1',
            mapName: 'users',
            query: {} // Match all
        }
    });

    // Mock Client 1 on Node 1
    const client1Mock = {
        id: 'mock-client-1',
        socket: {
            send: (msg: string) => console.log(`[Client 1 Received] ${msg}`),
            readyState: WebSocket.OPEN
        } as any,
        isAuthenticated: true,
        subscriptions: new Set()
    };
    (node1 as any).clients.set(client1Mock.id, client1Mock);

    // Create OP
    const op = {
        opType: 'set',
        mapName: 'users',
        key: 'user:1',
        record: {
            value: { name: 'Maverick' },
            timestamp: '0000000000000-0000-node-client'
        }
    };

    console.log('Injecting Client OP into Node 1...');
    (node1 as any).handleMessage(client1Mock, {
        type: 'CLIENT_OP',
        payload: op
    });

    // Check result
    setTimeout(() => {
        if (client2Received) {
            console.log('SUCCESS: Client 2 received update via Cluster Event');
        } else {
            console.log('FAILURE: Client 2 did not receive update');
        }
        process.exit(0);
    }, 1000);

}, 2000);
