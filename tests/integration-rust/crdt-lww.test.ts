import {
  createRustTestContext,
  createRustTestClient,
  spawnRustServer,
  createLWWRecord,
  waitForSync,
  waitUntil,
  TestClient,
  RustTestContext,
} from './helpers';

describe('Integration: LWW CRDT (Rust Server)', () => {
  // ========================================
  // Basic Write/Read Tests (G2a)
  // ========================================
  describe('Basic Write and Read', () => {
    let ctx: RustTestContext;

    beforeAll(async () => {
      ctx = await createRustTestContext(1);
    });

    afterAll(async () => {
      await ctx.cleanup();
    });

    test('CLIENT_OP with PUT writes data, OP_ACK received', async () => {
      const [client] = ctx.clients;
      const record = createLWWRecord({ title: 'Test Todo', done: false });

      // Clear messages to isolate this test's OP_ACK
      client.messages.length = 0;

      client.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'op-put-1',
          mapName: 'todos',
          opType: 'PUT',
          key: 'todo-1',
          record,
        },
      });

      const ack = await client.waitForMessage('OP_ACK');
      expect(ack).toBeDefined();
      expect(ack.type).toBe('OP_ACK');
      // The OP_ACK payload should reference the operation id
      expect(ack.payload).toBeDefined();
    });

    test('read back written data via QUERY_SUB snapshot', async () => {
      const [client] = ctx.clients;

      // Write data first
      const record = createLWWRecord({ name: 'Product A', price: 42 });
      client.messages.length = 0;

      client.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'op-read-1',
          mapName: 'products',
          opType: 'PUT',
          key: 'prod-1',
          record,
        },
      });

      // Wait for the write to be processed
      await client.waitForMessage('OP_ACK');
      await waitForSync(200);

      // Clear messages before subscribing so we get a fresh QUERY_RESP
      client.messages.length = 0;

      // Subscribe to read back the data
      client.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'q-read-1',
          mapName: 'products',
          query: {},
        },
      });

      const response = await client.waitForMessage('QUERY_RESP');
      expect(response).toBeDefined();
      expect(response.payload.queryId).toBe('q-read-1');
      expect(response.payload.results).toBeDefined();
      expect(Array.isArray(response.payload.results)).toBe(true);
      expect(response.payload.results.length).toBeGreaterThanOrEqual(1);

      // Find the product we wrote
      const prod = response.payload.results.find(
        (r: any) => r.key === 'prod-1'
      );
      expect(prod).toBeDefined();

      // The value should match what we wrote (the server may wrap it differently)
      const value = prod.record?.value ?? prod.value;
      expect(value).toEqual({ name: 'Product A', price: 42 });
    });

    test('OP_BATCH with multiple ops, OP_ACK.lastId equals last op id', async () => {
      const [client] = ctx.clients;
      client.messages.length = 0;

      const ops = [
        {
          id: 'batch-1',
          mapName: 'items',
          opType: 'PUT',
          key: 'item-1',
          record: createLWWRecord({ name: 'Item 1' }),
        },
        {
          id: 'batch-2',
          mapName: 'items',
          opType: 'PUT',
          key: 'item-2',
          record: createLWWRecord({ name: 'Item 2' }),
        },
        {
          id: 'batch-3',
          mapName: 'items',
          opType: 'PUT',
          key: 'item-3',
          record: createLWWRecord({ name: 'Item 3' }),
        },
      ];

      client.send({
        type: 'OP_BATCH',
        payload: { ops },
      });

      const ack = await client.waitForMessage('OP_ACK');
      expect(ack).toBeDefined();
      expect(ack.type).toBe('OP_ACK');
      expect(ack.payload.lastId).toBe('batch-3');
    });
  });

  // ========================================
  // Conflict Resolution Tests (G2b)
  // ========================================
  describe('LWW Conflict Resolution', () => {
    let cleanup: () => Promise<void>;
    let port: number;

    beforeAll(async () => {
      const server = await spawnRustServer();
      port = server.port;
      cleanup = server.cleanup;
    });

    afterAll(async () => {
      await cleanup();
    });

    test('LWW conflict: later write wins (two clients write sequentially)', async () => {
      // Client 1 writes first
      const client1 = await createRustTestClient(port, {
        nodeId: 'conflict-client-1',
        userId: 'user-1',
        roles: ['ADMIN'],
      });
      await client1.waitForMessage('AUTH_ACK');

      client1.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'conflict-op-1',
          mapName: 'conflict-test',
          opType: 'PUT',
          key: 'contested-key',
          record: createLWWRecord({ winner: 'client1-value' }, 'node-1'),
        },
      });

      await client1.waitForMessage('OP_ACK');
      await waitForSync(200);

      // Client 2 writes later -- the server assigns a later HLC timestamp
      const client2 = await createRustTestClient(port, {
        nodeId: 'conflict-client-2',
        userId: 'user-2',
        roles: ['ADMIN'],
      });
      await client2.waitForMessage('AUTH_ACK');
      client2.messages.length = 0;

      client2.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'conflict-op-2',
          mapName: 'conflict-test',
          opType: 'PUT',
          key: 'contested-key',
          record: createLWWRecord({ winner: 'client2-value' }, 'node-2'),
        },
      });

      await client2.waitForMessage('OP_ACK');
      await waitForSync(200);

      // Verify via QUERY_SUB that the later write (client2) won
      const reader = await createRustTestClient(port, {
        nodeId: 'conflict-reader',
        userId: 'reader',
        roles: ['ADMIN'],
      });
      await reader.waitForMessage('AUTH_ACK');
      reader.messages.length = 0;

      reader.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'conflict-q',
          mapName: 'conflict-test',
          query: {},
        },
      });

      const response = await reader.waitForMessage('QUERY_RESP');
      const result = response.payload.results.find(
        (r: any) => r.key === 'contested-key'
      );
      expect(result).toBeDefined();

      const value = result.record?.value ?? result.value;
      expect(value).toEqual({ winner: 'client2-value' });

      client1.close();
      client2.close();
      reader.close();
    });

    test('tombstone (value: null) via REMOVE -- key absent from QUERY_SUB snapshot', async () => {
      const client = await createRustTestClient(port, {
        nodeId: 'tombstone-client',
        userId: 'tombstoner',
        roles: ['ADMIN'],
      });
      await client.waitForMessage('AUTH_ACK');

      // Write a value first
      client.messages.length = 0;
      client.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'tomb-write-1',
          mapName: 'tombstone-test',
          opType: 'PUT',
          key: 'doomed-key',
          record: createLWWRecord({ alive: true }),
        },
      });

      await client.waitForMessage('OP_ACK');
      await waitForSync(200);

      // Delete via REMOVE with null value (tombstone)
      client.messages.length = 0;
      client.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'tomb-remove-1',
          mapName: 'tombstone-test',
          opType: 'REMOVE',
          key: 'doomed-key',
          record: {
            value: null,
            timestamp: {
              millis: Date.now(),
              counter: 0,
              nodeId: 'tombstone-client',
            },
          },
        },
      });

      await client.waitForMessage('OP_ACK');
      await waitForSync(200);

      // Verify via QUERY_SUB that the key is absent
      client.messages.length = 0;
      client.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'tomb-q',
          mapName: 'tombstone-test',
          query: {},
        },
      });

      const response = await client.waitForMessage('QUERY_RESP');
      const result = response.payload.results.find(
        (r: any) => r.key === 'doomed-key'
      );
      // Tombstoned keys should not appear in the snapshot, or if they do,
      // the value should be null/undefined
      if (result) {
        const value = result.record?.value ?? result.value;
        expect(value).toBeNull();
      }

      client.close();
    });

    test('HLC ordering: later write wins (server sanitizes to monotonic timestamps)', async () => {
      // Write two values to the same key in sequence from a single client.
      // The first write has value "first", the second has value "second".
      // Even though both come from the same client, the server assigns
      // monotonically increasing HLC timestamps, so "second" should win.
      const client = await createRustTestClient(port, {
        nodeId: 'hlc-order-client',
        userId: 'hlc-tester',
        roles: ['ADMIN'],
      });
      await client.waitForMessage('AUTH_ACK');

      // First write
      client.messages.length = 0;
      client.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'hlc-op-1',
          mapName: 'hlc-order-test',
          opType: 'PUT',
          key: 'order-key',
          record: createLWWRecord({ order: 'first' }),
        },
      });

      await client.waitForMessage('OP_ACK');
      await waitForSync(100);

      // Second write (later server timestamp due to monotonic HLC)
      client.messages.length = 0;
      client.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'hlc-op-2',
          mapName: 'hlc-order-test',
          opType: 'PUT',
          key: 'order-key',
          record: createLWWRecord({ order: 'second' }),
        },
      });

      await client.waitForMessage('OP_ACK');
      await waitForSync(200);

      // Verify that "second" won
      client.messages.length = 0;
      client.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'hlc-q',
          mapName: 'hlc-order-test',
          query: {},
        },
      });

      const response = await client.waitForMessage('QUERY_RESP');
      const result = response.payload.results.find(
        (r: any) => r.key === 'order-key'
      );
      expect(result).toBeDefined();

      const value = result.record?.value ?? result.value;
      expect(value).toEqual({ order: 'second' });

      client.close();
    });

    test('deterministic winner: later-processed write wins (verified by value)', async () => {
      // Two clients write the same key in controlled sequence.
      // We verify that the value from the later-processed write is returned,
      // without comparing timestamps directly (server sanitizes them).
      const client1 = await createRustTestClient(port, {
        nodeId: 'det-client-1',
        userId: 'det-user-1',
        roles: ['ADMIN'],
      });
      await client1.waitForMessage('AUTH_ACK');

      const client2 = await createRustTestClient(port, {
        nodeId: 'det-client-2',
        userId: 'det-user-2',
        roles: ['ADMIN'],
      });
      await client2.waitForMessage('AUTH_ACK');

      // Client 1 writes first
      client1.messages.length = 0;
      client1.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'det-op-1',
          mapName: 'deterministic-test',
          opType: 'PUT',
          key: 'det-key',
          record: createLWWRecord({ author: 'client1' }, 'det-node-1'),
        },
      });

      await client1.waitForMessage('OP_ACK');
      await waitForSync(200);

      // Client 2 writes later
      client2.messages.length = 0;
      client2.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'det-op-2',
          mapName: 'deterministic-test',
          opType: 'PUT',
          key: 'det-key',
          record: createLWWRecord({ author: 'client2' }, 'det-node-2'),
        },
      });

      await client2.waitForMessage('OP_ACK');
      await waitForSync(200);

      // Read back via a third client
      const reader = await createRustTestClient(port, {
        nodeId: 'det-reader',
        userId: 'det-reader',
        roles: ['ADMIN'],
      });
      await reader.waitForMessage('AUTH_ACK');
      reader.messages.length = 0;

      reader.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'det-q',
          mapName: 'deterministic-test',
          query: {},
        },
      });

      const response = await reader.waitForMessage('QUERY_RESP');
      const result = response.payload.results.find(
        (r: any) => r.key === 'det-key'
      );
      expect(result).toBeDefined();

      const value = result.record?.value ?? result.value;
      // Client 2 wrote later, so its value should win
      expect(value).toEqual({ author: 'client2' });

      client1.close();
      client2.close();
      reader.close();
    });
  });
});
