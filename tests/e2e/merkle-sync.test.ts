import { ServerCoordinator } from '@topgunbuild/server';
import { LWWMap } from '@topgunbuild/core';
import { TopGunClient } from '@topgunbuild/client';
import {
    createTestServer,
    createTestToken,
    waitUntil,
    MemoryStorageAdapter,
    createLWWRecord
} from './helpers';

describe('E2E: Merkle Sync', () => {
    let server: ServerCoordinator;
    let client: TopGunClient;

    beforeEach(async () => {
        server = await createTestServer();
    });

    afterEach(async () => {
        // Close client connection before shutting down server
        if (client) {
            client.close();
        }
        await server.shutdown();
    });

    test('Basic Sync: Client receives all data from server on connect', async () => {
        // 1. Populate Server with data
        const serverMap = server.getMap('todos') as LWWMap<string, any>;
        serverMap.merge('todo-1', createLWWRecord({ title: 'Task 1', done: false }));
        serverMap.merge('todo-2', createLWWRecord({ title: 'Task 2', done: true }));
        serverMap.merge('todo-3', createLWWRecord({ title: 'Task 3', done: false }));

        // 2. Create Client
        const storage = new MemoryStorageAdapter();
        await storage.initialize('test-db');

        client = new TopGunClient({
            serverUrl: `ws://localhost:${server.port}`,
            storage
        });
        await client.start();

        // 3. Authenticate
        const token = createTestToken('test-user', ['ADMIN']);
        client.setAuthToken(token);

        // 4. Register map (triggers sync)
        const clientMap = client.getMap<string, any>('todos');

        // 5. Wait for sync
        await waitUntil(() => clientMap.get('todo-1') !== undefined, 2000);
        await waitUntil(() => clientMap.get('todo-2') !== undefined, 2000);
        await waitUntil(() => clientMap.get('todo-3') !== undefined, 2000);

        // 6. Verify data
        expect(clientMap.get('todo-1')).toEqual({ title: 'Task 1', done: false });
        expect(clientMap.get('todo-2')).toEqual({ title: 'Task 2', done: true });
        expect(clientMap.get('todo-3')).toEqual({ title: 'Task 3', done: false });
    });

    test('Delta Sync: Client receives only updates', async () => {
        // 1. Populate Server
        const serverMap = server.getMap('delta-test') as LWWMap<string, any>;
        serverMap.merge('item-1', createLWWRecord({ val: 1 }));
        serverMap.merge('item-2', createLWWRecord({ val: 2 }));

        // 2. Client 1 connects and syncs
        const storage = new MemoryStorageAdapter();
        await storage.initialize('delta-db');

        client = new TopGunClient({
            serverUrl: `ws://localhost:${server.port}`,
            storage
        });
        await client.start();
        client.setAuthToken(createTestToken('user-1', ['ADMIN']));

        const clientMap = client.getMap<string, any>('delta-test');
        await waitUntil(() => clientMap.get('item-2') !== undefined, 2000);

        // 3. Close first client before creating second one
        client.close();

        // Update server data while client is "offline"
        serverMap.merge('item-3', createLWWRecord({ val: 3 })); // New item
        serverMap.merge('item-1', createLWWRecord({ val: 100 })); // Updated item

        // 4. New Client Instance (Same Storage) -> Simulates Reconnect
        const client2 = new TopGunClient({
            serverUrl: `ws://localhost:${server.port}`,
            storage // Reusing storage
        });
        await client2.start();
        client2.setAuthToken(createTestToken('user-1', ['ADMIN']));

        const clientMap2 = client2.getMap<string, any>('delta-test');

        // Wait for storage restoration (async process in getMap)
        await waitUntil(() => clientMap2.get('item-2') !== undefined, 2000);
        expect(clientMap2.get('item-2')).toEqual({ val: 2 });

        // Wait for sync of new/updated data from server
        await waitUntil(() => clientMap2.get('item-3') !== undefined, 2000);
        await waitUntil(() => clientMap2.get('item-1')?.val === 100, 2000);

        expect(clientMap2.get('item-3')).toEqual({ val: 3 });
        expect(clientMap2.get('item-1')).toEqual({ val: 100 });

        // Clean up client2
        client2.close();
    });
});
