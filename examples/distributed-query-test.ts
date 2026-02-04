import { ServerFactory } from '../packages/server/src';
import { WebSocket } from 'ws';
import { deserialize } from '../packages/core/src/serializer';

// Helper to delay
const delay = (ms: number) => new Promise(resolve => setTimeout(resolve, ms));

async function runTest() {
    console.log('Starting Distributed Query Test...');

    // 1. Start 3 Nodes
    const node1 = ServerFactory.create({
        port: 8081,
        nodeId: 'node-1',
        host: 'localhost',
        clusterPort: 9001,
        peers: ['localhost:9002', 'localhost:9003']
    });

    const node2 = ServerFactory.create({
        port: 8082,
        nodeId: 'node-2',
        host: 'localhost',
        clusterPort: 9002,
        peers: ['localhost:9001', 'localhost:9003']
    });

    const node3 = ServerFactory.create({
        port: 8083,
        nodeId: 'node-3',
        host: 'localhost',
        clusterPort: 9003,
        peers: ['localhost:9001', 'localhost:9002']
    });

    console.log('Waiting for cluster to form...');
    await delay(2000);

    // 2. Mock Client Connection on Node 1
    const clientMock = {
        id: 'test-client',
        socket: {
            send: (msgData: Uint8Array) => {
                try {
                    const msg = deserialize(msgData) as any;
                    if (msg.type === 'QUERY_RESP') {
                        console.log('\n[TEST RESULT] Received QUERY_RESP:');
                        console.log(`Query ID: ${msg.payload.queryId}`);
                        console.log(`Record Count: ${msg.payload.results.length}`);
                        const names = msg.payload.results.map((r: any) => r.value.name).sort();
                        console.log('Names:', names);

                        // We expect multiple records.
                        // If we injected 10 records, we expect 10 (assuming all age > 20).
                        if (names.length === 10) {
                            console.log('SUCCESS: Received data from all distributed nodes!');
                            process.exit(0);
                        } else {
                            console.error(`FAILURE: Incomplete results. Expected 10, got ${names.length}`);
                            // Don't exit yet, maybe more updates coming? 
                            // Actually QUERY_RESP is one-shot for initial results.
                            process.exit(1);
                        }
                    }
                } catch (e) {
                    console.error('Failed to deserialize message', e);
                }
            },
            readyState: WebSocket.OPEN,
            close: () => {}
        } as any,
        isAuthenticated: true,
        subscriptions: new Set()
    };
    // Force inject client into Node 1
    (node1 as any).clients.set(clientMock.id, clientMock);

    // 3. Inject Data (Distributed)
    // Inject 10 users to ensure distribution across 3 nodes.
    const data = [];
    for (let i = 0; i < 10; i++) {
        data.push({ 
            key: `user:${i}`, 
            value: { name: `User${i}`, age: 25 + i } 
        });
    }

    console.log(`\nInjecting ${data.length} records (should be forwarded to owners)...`);
    for (const item of data) {
        (node1 as any).handleMessage(clientMock, {
            type: 'CLIENT_OP',
            payload: {
                opType: 'set',
                mapName: 'users',
                key: item.key,
                record: {
                    value: item.value,
                    timestamp: Date.now() // Simple timestamp
                }
            }
        });
    }

    await delay(2000); // Wait for forwarding and storage

    // 4. Execute Distributed Query
    console.log('\nExecuting Query on Node 1 (Expect Scatter-Gather)...');
    (node1 as any).handleMessage(clientMock, {
        type: 'QUERY_SUB',
        payload: {
            queryId: 'q1',
            mapName: 'users',
            query: {
                where: { age: { $gt: 20 } } // Should match all
            }
        }
    });

    // Wait for result
    setTimeout(() => {
        console.log('Timeout waiting for results.');
        process.exit(1);
    }, 5000);
}

runTest().catch(console.error);
