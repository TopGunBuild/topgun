import { ServerCoordinator } from '@topgunbuild/server';
import { LWWMap, ORMap, serialize, deserialize } from '@topgunbuild/core';
import WebSocket from 'ws';
import {
  createTestServer,
  createTestClient,
  createTestContext,
  createTestToken,
  createLWWRecord,
  createORRecord,
  waitForSync,
  waitUntil,
  TestClient,
} from './helpers';

describe('E2E: Basic Sync', () => {
  // ========================================
  // Connection Tests
  // ========================================
  describe('Connection', () => {
    let server: ServerCoordinator;

    beforeEach(async () => {
      server = await createTestServer();
    });

    afterEach(async () => {
      await server.shutdown();
    });

    test('client successfully connects to server', async () => {
      const client = await createTestClient(`ws://localhost:${server.port}`);

      expect(client.ws.readyState).toBe(WebSocket.OPEN);

      client.close();
    });

    test('client receives AUTH_REQUIRED on connect', async () => {
      const client = await createTestClient(`ws://localhost:${server.port}`, {
        autoAuth: false,
      });

      const authRequired = await client.waitForMessage('AUTH_REQUIRED');
      expect(authRequired).toBeDefined();
      expect(authRequired.type).toBe('AUTH_REQUIRED');

      client.close();
    });

    test('client receives AUTH_ACK after successful authentication', async () => {
      const client = await createTestClient(`ws://localhost:${server.port}`);

      const authAck = await client.waitForMessage('AUTH_ACK');
      expect(authAck).toBeDefined();
      expect(authAck.type).toBe('AUTH_ACK');
      expect(client.isAuthenticated).toBe(true);

      client.close();
    });

    test('client receives AUTH_FAIL with invalid token', async () => {
      const client = await createTestClient(`ws://localhost:${server.port}`, {
        autoAuth: false,
      });

      await client.waitForMessage('AUTH_REQUIRED');

      // Send invalid token
      client.send({ type: 'AUTH', token: 'invalid-token' });

      const authFail = await client.waitForMessage('AUTH_FAIL');
      expect(authFail).toBeDefined();
      expect(authFail.type).toBe('AUTH_FAIL');

      // Wait a bit for close
      await waitForSync(100);
    });

    test('client reconnects after connection drop', async () => {
      const client = await createTestClient(`ws://localhost:${server.port}`);
      await client.waitForMessage('AUTH_ACK');

      // Close connection
      client.close();
      await waitForSync(100);

      // Reconnect
      const client2 = await createTestClient(`ws://localhost:${server.port}`);
      const authAck = await client2.waitForMessage('AUTH_ACK');
      expect(authAck).toBeDefined();

      client2.close();
    });
  });

  // ========================================
  // LWWMap Write Tests (Client -> Server)
  // ========================================
  describe('LWWMap Write (Client -> Server)', () => {
    let server: ServerCoordinator;
    let client: TestClient;

    beforeEach(async () => {
      server = await createTestServer();
      client = await createTestClient(`ws://localhost:${server.port}`, {
        roles: ['ADMIN'],
      });
      await client.waitForMessage('AUTH_ACK');
    });

    afterEach(async () => {
      client.close();
      await server.shutdown();
    });

    test('client writes data to LWWMap via CLIENT_OP', async () => {
      const record = createLWWRecord({ title: 'Test Todo', done: false });

      client.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'op-1',
          mapName: 'todos',
          opType: 'PUT',
          key: 'todo-1',
          record,
        },
      });

      await waitForSync(200);

      // Verify server state
      const map = server.getMap('todos') as LWWMap<string, any>;
      expect(map.get('todo-1')).toEqual({ title: 'Test Todo', done: false });
    });

    test('client receives OP_ACK for batch operations', async () => {
      const ops = [
        {
          id: '100',
          mapName: 'items',
          opType: 'PUT',
          key: 'item-1',
          record: createLWWRecord({ name: 'Item 1' }),
        },
        {
          id: '101',
          mapName: 'items',
          opType: 'PUT',
          key: 'item-2',
          record: createLWWRecord({ name: 'Item 2' }),
        },
      ];

      client.send({
        type: 'OP_BATCH',
        payload: { ops },
      });

      const ack = await client.waitForMessage('OP_ACK');
      expect(ack).toBeDefined();
      expect(ack.payload.lastId).toBe('101');

      // Verify server state
      const map = server.getMap('items') as LWWMap<string, any>;
      expect(map.get('item-1')).toEqual({ name: 'Item 1' });
      expect(map.get('item-2')).toEqual({ name: 'Item 2' });
    });

    test('client can update existing LWWMap entry', async () => {
      // First write
      const record1 = createLWWRecord({ title: 'Original' }, 'client-1');
      client.send({
        type: 'CLIENT_OP',
        payload: {
          mapName: 'notes',
          opType: 'PUT',
          key: 'note-1',
          record: record1,
        },
      });

      await waitForSync(100);

      // Update with newer timestamp
      const record2 = {
        value: { title: 'Updated' },
        timestamp: {
          millis: Date.now() + 1000, // Ensure newer
          counter: 0,
          nodeId: 'client-1',
        },
      };

      client.send({
        type: 'CLIENT_OP',
        payload: {
          mapName: 'notes',
          opType: 'PUT',
          key: 'note-1',
          record: record2,
        },
      });

      await waitForSync(100);

      const map = server.getMap('notes') as LWWMap<string, any>;
      expect(map.get('note-1')).toEqual({ title: 'Updated' });
    });

    test('client can delete LWWMap entry', async () => {
      // First write
      client.send({
        type: 'CLIENT_OP',
        payload: {
          mapName: 'deletables',
          opType: 'PUT',
          key: 'item-1',
          record: createLWWRecord({ name: 'To be deleted' }),
        },
      });

      await waitForSync(100);

      // Delete (tombstone)
      const tombstone = {
        value: null,
        timestamp: {
          millis: Date.now() + 1000,
          counter: 0,
          nodeId: 'client-1',
        },
      };

      client.send({
        type: 'CLIENT_OP',
        payload: {
          mapName: 'deletables',
          opType: 'REMOVE',
          key: 'item-1',
          record: tombstone,
        },
      });

      await waitForSync(100);

      const map = server.getMap('deletables') as LWWMap<string, any>;
      // LWWMap.get() returns undefined for tombstones (value: null)
      expect(map.get('item-1')).toBeUndefined();
    });
  });

  // ========================================
  // Read/Subscription Tests (Server -> Client)
  // ========================================
  describe('Read/Subscription (Server -> Client)', () => {
    let server: ServerCoordinator;
    let client: TestClient;

    beforeEach(async () => {
      server = await createTestServer();
      client = await createTestClient(`ws://localhost:${server.port}`, {
        roles: ['ADMIN'],
      });
      await client.waitForMessage('AUTH_ACK');
    });

    afterEach(async () => {
      client.close();
      await server.shutdown();
    });

    test('client subscribes and receives existing data (snapshot)', async () => {
      // Pre-populate server data
      const map = server.getMap('products') as LWWMap<string, any>;
      map.merge('prod-1', createLWWRecord({ name: 'Product 1', price: 100 }));
      map.merge('prod-2', createLWWRecord({ name: 'Product 2', price: 200 }));

      // Subscribe
      client.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'q1',
          mapName: 'products',
          query: {},
        },
      });

      const response = await client.waitForMessage('QUERY_RESP');
      expect(response).toBeDefined();
      expect(response.payload.queryId).toBe('q1');
      expect(response.payload.results).toHaveLength(2);

      const keys = response.payload.results.map((r: any) => r.key).sort();
      expect(keys).toEqual(['prod-1', 'prod-2']);
    });

    test('client receives updates after subscription', async () => {
      // Subscribe first
      client.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'q2',
          mapName: 'updates-test',
          query: {},
        },
      });

      await client.waitForMessage('QUERY_RESP');
      client.messages.length = 0; // Clear messages

      // Another client writes data
      const client2 = await createTestClient(`ws://localhost:${server.port}`, {
        roles: ['ADMIN'],
        nodeId: 'client-2',
      });
      await client2.waitForMessage('AUTH_ACK');

      client2.send({
        type: 'CLIENT_OP',
        payload: {
          mapName: 'updates-test',
          opType: 'PUT',
          key: 'new-item',
          record: createLWWRecord({ name: 'New Item' }),
        },
      });

      // Wait for update notification
      await waitForSync(200);

      // Check for QUERY_UPDATE or SERVER_EVENT
      const updateMsg = client.messages.find(
        (m) => m.type === 'QUERY_UPDATE' || m.type === 'SERVER_EVENT'
      );
      expect(updateMsg).toBeDefined();

      client2.close();
    });

    test('client receives filtered results with where clause', async () => {
      // Pre-populate
      const map = server.getMap('filtered') as LWWMap<string, any>;
      map.merge('item-1', createLWWRecord({ category: 'A', value: 10 }));
      map.merge('item-2', createLWWRecord({ category: 'B', value: 20 }));
      map.merge('item-3', createLWWRecord({ category: 'A', value: 30 }));

      // Subscribe with where filter
      client.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'q3',
          mapName: 'filtered',
          query: {
            where: { category: 'A' },
          },
        },
      });

      const response = await client.waitForMessage('QUERY_RESP');
      expect(response.payload.results).toHaveLength(2);

      const keys = response.payload.results.map((r: any) => r.key).sort();
      expect(keys).toEqual(['item-1', 'item-3']);
    });

    test('client can unsubscribe from query', async () => {
      client.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'q4',
          mapName: 'unsub-test',
          query: {},
        },
      });

      await client.waitForMessage('QUERY_RESP');

      // Unsubscribe
      client.send({
        type: 'QUERY_UNSUB',
        payload: { queryId: 'q4' },
      });

      await waitForSync(100);

      // No error should occur
    });
  });

  // ========================================
  // ORMap Tests
  // ========================================
  describe('ORMap Operations', () => {
    let server: ServerCoordinator;
    let client: TestClient;

    beforeEach(async () => {
      server = await createTestServer();
      client = await createTestClient(`ws://localhost:${server.port}`, {
        roles: ['ADMIN'],
      });
      await client.waitForMessage('AUTH_ACK');
    });

    afterEach(async () => {
      client.close();
      await server.shutdown();
    });

    test('client adds item to ORMap via OR_ADD', async () => {
      const orRecord = createORRecord('Value 1', 'client-1');

      client.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'or-1',
          mapName: 'or-items',
          opType: 'OR_ADD',
          key: 'list-1',
          orRecord,
        },
      });

      await waitForSync(200);

      const map = server.getMap('or-items', 'OR') as ORMap<string, any>;
      const values = map.get('list-1');
      expect(Array.isArray(values)).toBe(true);
      expect(values).toContain('Value 1');
    });

    test('ORMap supports multiple values per key', async () => {
      // Add first value
      client.send({
        type: 'CLIENT_OP',
        payload: {
          mapName: 'multi-values',
          opType: 'OR_ADD',
          key: 'tags',
          orRecord: createORRecord('tag1', 'client-1'),
        },
      });

      await waitForSync(100);

      // Add second value
      client.send({
        type: 'CLIENT_OP',
        payload: {
          mapName: 'multi-values',
          opType: 'OR_ADD',
          key: 'tags',
          orRecord: createORRecord('tag2', 'client-1'),
        },
      });

      await waitForSync(200);

      const map = server.getMap('multi-values', 'OR') as ORMap<string, any>;
      const values = map.get('tags');
      expect(values).toHaveLength(2);
      expect(values.sort()).toEqual(['tag1', 'tag2']);
    });

    test('client removes item from ORMap via OR_REMOVE (tombstone)', async () => {
      const tag = 'test-tag-' + Date.now();
      const orRecord = {
        value: 'To Remove',
        timestamp: { millis: Date.now(), counter: 0, nodeId: 'client-1' },
        tag,
      };

      // Add item
      client.send({
        type: 'CLIENT_OP',
        payload: {
          mapName: 'removable',
          opType: 'OR_ADD',
          key: 'items',
          orRecord,
        },
      });

      await waitForSync(200);

      // Verify added
      let map = server.getMap('removable', 'OR') as ORMap<string, any>;
      expect(map.get('items')).toContain('To Remove');

      // Remove by tag
      client.send({
        type: 'CLIENT_OP',
        payload: {
          mapName: 'removable',
          opType: 'OR_REMOVE',
          key: 'items',
          orTag: tag,
        },
      });

      await waitForSync(200);

      map = server.getMap('removable', 'OR') as ORMap<string, any>;
      expect(map.get('items')).not.toContain('To Remove');
    });

    test('tombstone synchronizes between clients', async () => {
      const tag = `shared-tag-${Date.now()}`;

      // Client 1 adds item
      client.send({
        type: 'CLIENT_OP',
        payload: {
          mapName: 'shared-or',
          opType: 'OR_ADD',
          key: 'shared-list',
          orRecord: {
            value: 'Shared Item',
            timestamp: { millis: Date.now(), counter: 0, nodeId: 'client-1' },
            tag,
          },
        },
      });

      await waitForSync(200);

      // Client 2 connects and subscribes
      const client2 = await createTestClient(`ws://localhost:${server.port}`, {
        roles: ['ADMIN'],
        nodeId: 'client-2',
      });
      await client2.waitForMessage('AUTH_ACK');

      // Client 2 removes the item
      client2.send({
        type: 'CLIENT_OP',
        payload: {
          mapName: 'shared-or',
          opType: 'OR_REMOVE',
          key: 'shared-list',
          orTag: tag,
        },
      });

      await waitForSync(200);

      // Verify removal on server
      const map = server.getMap('shared-or', 'OR') as ORMap<string, any>;
      expect(map.get('shared-list')).not.toContain('Shared Item');

      client2.close();
    });
  });

  // ========================================
  // Multi-Client Synchronization
  // ========================================
  describe('Multi-Client Synchronization', () => {
    test('changes from one client propagate to another', async () => {
      const ctx = await createTestContext(2);

      try {
        const [client1, client2] = ctx.clients;

        // Client 2 subscribes
        client2.send({
          type: 'QUERY_SUB',
          payload: {
            queryId: 'sync-q1',
            mapName: 'sync-test',
            query: {},
          },
        });

        await client2.waitForMessage('QUERY_RESP');
        client2.messages.length = 0;

        // Client 1 writes
        client1.send({
          type: 'CLIENT_OP',
          payload: {
            mapName: 'sync-test',
            opType: 'PUT',
            key: 'shared-item',
            record: createLWWRecord({ message: 'Hello from Client 1' }),
          },
        });

        // Wait for propagation
        await waitUntil(
          () => client2.messages.some((m) => m.type === 'QUERY_UPDATE' || m.type === 'SERVER_EVENT'),
          3000
        );

        const update = client2.messages.find(
          (m) => m.type === 'QUERY_UPDATE' || m.type === 'SERVER_EVENT'
        );
        expect(update).toBeDefined();
      } finally {
        await ctx.cleanup();
      }
    });

    test('concurrent writes from multiple clients are resolved', async () => {
      const ctx = await createTestContext(2);

      try {
        const [client1, client2] = ctx.clients;
        const baseTs = Date.now();

        // Client 1 writes with earlier timestamp
        client1.send({
          type: 'CLIENT_OP',
          payload: {
            mapName: 'conflict-test',
            opType: 'PUT',
            key: 'item',
            record: {
              value: { winner: 'client1' },
              timestamp: { millis: baseTs, counter: 0, nodeId: 'client-0' },
            },
          },
        });

        // Client 2 writes with later timestamp
        client2.send({
          type: 'CLIENT_OP',
          payload: {
            mapName: 'conflict-test',
            opType: 'PUT',
            key: 'item',
            record: {
              value: { winner: 'client2' },
              timestamp: { millis: baseTs + 1000, counter: 0, nodeId: 'client-1' },
            },
          },
        });

        await waitForSync(300);

        // LWW: Later timestamp wins
        const map = ctx.server.getMap('conflict-test') as LWWMap<string, any>;
        expect(map.get('item')).toEqual({ winner: 'client2' });
      } finally {
        await ctx.cleanup();
      }
    });
  });

  // ========================================
  // Topic Pub/Sub Tests
  // ========================================
  describe('Topic Pub/Sub', () => {
    let server: ServerCoordinator;
    let publisher: TestClient;
    let subscriber: TestClient;

    beforeEach(async () => {
      server = await createTestServer();
      publisher = await createTestClient(`ws://localhost:${server.port}`, {
        roles: ['ADMIN'],
        nodeId: 'publisher',
      });
      subscriber = await createTestClient(`ws://localhost:${server.port}`, {
        roles: ['ADMIN'],
        nodeId: 'subscriber',
      });
      await publisher.waitForMessage('AUTH_ACK');
      await subscriber.waitForMessage('AUTH_ACK');
    });

    afterEach(async () => {
      publisher.close();
      subscriber.close();
      await server.shutdown();
    });

    test('subscriber receives messages published to topic', async () => {
      // Subscribe
      subscriber.send({
        type: 'TOPIC_SUB',
        payload: { topic: 'chat' },
      });

      await waitForSync(100);
      subscriber.messages.length = 0;

      // Publish
      publisher.send({
        type: 'TOPIC_PUB',
        payload: {
          topic: 'chat',
          data: { text: 'Hello World!' },
        },
      });

      // Wait for message
      const msg = await subscriber.waitForMessage('TOPIC_MESSAGE');
      expect(msg).toBeDefined();
      expect(msg.payload.topic).toBe('chat');
      expect(msg.payload.data).toEqual({ text: 'Hello World!' });
    });
  });
});
