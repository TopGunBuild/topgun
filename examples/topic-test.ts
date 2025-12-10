import { ServerCoordinator } from '../packages/server/src';
import { deserialize } from '../packages/core/src';
import { WebSocket } from 'ws';

// Helper to wait
const wait = (ms: number) => new Promise(resolve => setTimeout(resolve, ms));

async function runTest() {
    // Node 1
    const node1 = new ServerCoordinator({
        port: 8083,
        nodeId: 'node-1',
        host: 'localhost',
        clusterPort: 9003,
        peers: ['localhost:9004']
    });

    // Node 2
    const node2 = new ServerCoordinator({
        port: 8084,
        nodeId: 'node-2',
        host: 'localhost',
        clusterPort: 9004,
        peers: ['localhost:9003']
    });

    await node1.ready();
    await node2.ready();

    console.log('Cluster nodes started on 8083 and 8084');

    await wait(1000); // Wait for cluster connection

    console.log('\n--- Starting Topic Pub/Sub Test ---');

    let client2Received = false;

    // Mock Client 2 on Node 2
    const client2Mock = {
        id: 'mock-client-2',
        socket: {
            send: (msgData: any) => {
                let msg;
                if (Buffer.isBuffer(msgData) || msgData instanceof Uint8Array) {
                    msg = deserialize(msgData);
                } else {
                    msg = JSON.parse(msgData);
                }
                
                console.log(`[Client 2 Received]`, msg);
                if (msg.type === 'TOPIC_MESSAGE' && msg.payload.topic === 'chat' && msg.payload.data.text === 'Hello World') {
                    client2Received = true;
                }
            },
            readyState: WebSocket.OPEN,
            close: () => {}
        } as any,
        isAuthenticated: true,
        principal: { userId: 'mock-user-2', roles: ['ADMIN'] },
        subscriptions: new Set(),
        lastActiveHlc: { millis: 0, counter: 0, nodeId: 'mock' }
    };
    // Hack to inject client into private map
    (node2 as any).clients.set(client2Mock.id, client2Mock);

    // Client 2 Subscribes to 'chat'
    console.log('Client 2 subscribing to topic "chat"...');
    (node2 as any).handleMessage(client2Mock, {
        type: 'TOPIC_SUB',
        payload: {
            topic: 'chat'
        }
    });

    await wait(100);

    // Mock Client 1 on Node 1
    const client1Mock = {
        id: 'mock-client-1',
        socket: {
            send: (msg: string) => console.log(`[Client 1 Received] ${msg}`),
            readyState: WebSocket.OPEN,
            close: () => {}
        } as any,
        isAuthenticated: true,
        principal: { userId: 'mock-user-1', roles: ['ADMIN'] },
        subscriptions: new Set(),
        lastActiveHlc: { millis: 0, counter: 0, nodeId: 'mock' }
    };
    (node1 as any).clients.set(client1Mock.id, client1Mock);

    // Client 1 Publishes to 'chat'
    console.log('Client 1 publishing to topic "chat"...');
    (node1 as any).handleMessage(client1Mock, {
        type: 'TOPIC_PUB',
        payload: {
            topic: 'chat',
            data: { text: 'Hello World' }
        }
    });

    // Check result
    await wait(1000);
    
    if (client2Received) {
        console.log('SUCCESS: Client 2 received TOPIC_MESSAGE via Cluster');
    } else {
        console.log('FAILURE: Client 2 did not receive TOPIC_MESSAGE');
    }

    await node1.shutdown();
    await node2.shutdown();
    process.exit(client2Received ? 0 : 1);
}

runTest();

