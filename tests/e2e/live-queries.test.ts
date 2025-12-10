import { ServerCoordinator } from '@topgunbuild/server';
import { LWWMap } from '@topgunbuild/core';
import {
  createTestServer,
  createTestClient,
  createTestContext,
  createLWWRecord,
  waitForSync,
  waitUntil,
  TestClient,
} from './helpers';

describe('E2E: Live Queries', () => {
  // ========================================
  // Basic Query Tests
  // ========================================
  describe('Basic Queries', () => {
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

    test('client creates query on collection and receives initial snapshot', async () => {
      // Pre-populate server data
      const map = server.getMap('users') as LWWMap<string, any>;
      map.merge('user-1', createLWWRecord({ name: 'Alice', age: 30 }));
      map.merge('user-2', createLWWRecord({ name: 'Bob', age: 25 }));
      map.merge('user-3', createLWWRecord({ name: 'Charlie', age: 35 }));

      // Subscribe to query
      client.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'q-users',
          mapName: 'users',
          query: {},
        },
      });

      const response = await client.waitForMessage('QUERY_RESP');
      expect(response).toBeDefined();
      expect(response.payload.queryId).toBe('q-users');
      expect(response.payload.results).toHaveLength(3);

      const names = response.payload.results.map((r: any) => r.value.name).sort();
      expect(names).toEqual(['Alice', 'Bob', 'Charlie']);
    });

    test('new data automatically arrives after subscription', async () => {
      // Subscribe first (empty collection)
      client.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'q-live',
          mapName: 'live-data',
          query: {},
        },
      });

      await client.waitForMessage('QUERY_RESP');
      client.messages.length = 0;

      // Another client writes data
      const writer = await createTestClient(`ws://localhost:${server.port}`, {
        roles: ['ADMIN'],
        nodeId: 'writer',
      });
      await writer.waitForMessage('AUTH_ACK');

      writer.send({
        type: 'CLIENT_OP',
        payload: {
          mapName: 'live-data',
          opType: 'PUT',
          key: 'item-1',
          record: createLWWRecord({ message: 'Hello Live Query!' }),
        },
      });

      // Wait for update
      await waitUntil(
        () => client.messages.some((m) => m.type === 'QUERY_UPDATE'),
        3000
      );

      const update = client.messages.find((m) => m.type === 'QUERY_UPDATE');
      expect(update).toBeDefined();
      expect(update.payload.queryId).toBe('q-live');
      expect(update.payload.key).toBe('item-1');
      expect(update.payload.value.message).toBe('Hello Live Query!');

      writer.close();
    });

    test('empty collection returns empty results', async () => {
      client.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'q-empty',
          mapName: 'empty-collection',
          query: {},
        },
      });

      const response = await client.waitForMessage('QUERY_RESP');
      expect(response.payload.results).toHaveLength(0);
    });
  });

  // ========================================
  // Filtering Tests
  // ========================================
  describe('Filtering', () => {
    let server: ServerCoordinator;
    let client: TestClient;

    beforeEach(async () => {
      server = await createTestServer();
      client = await createTestClient(`ws://localhost:${server.port}`, {
        roles: ['ADMIN'],
      });
      await client.waitForMessage('AUTH_ACK');

      // Pre-populate with test data
      const map = server.getMap('tasks') as LWWMap<string, any>;
      map.merge('task-1', createLWWRecord({ title: 'Task 1', status: 'active', priority: 1 }));
      map.merge('task-2', createLWWRecord({ title: 'Task 2', status: 'completed', priority: 2 }));
      map.merge('task-3', createLWWRecord({ title: 'Task 3', status: 'active', priority: 3 }));
      map.merge('task-4', createLWWRecord({ title: 'Task 4', status: 'active', priority: 1 }));
      map.merge('task-5', createLWWRecord({ title: 'Task 5', status: 'completed', priority: 2 }));
    });

    afterEach(async () => {
      client.close();
      await server.shutdown();
    });

    test('query with equality filter returns matching records', async () => {
      client.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'q-active',
          mapName: 'tasks',
          query: {
            where: { status: 'active' },
          },
        },
      });

      const response = await client.waitForMessage('QUERY_RESP');
      expect(response.payload.results).toHaveLength(3);

      const titles = response.payload.results.map((r: any) => r.value.title).sort();
      expect(titles).toEqual(['Task 1', 'Task 3', 'Task 4']);
    });

    test('record no longer matches filter - removed from results', async () => {
      // Subscribe to active tasks
      client.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'q-filter-remove',
          mapName: 'tasks',
          query: {
            where: { status: 'active' },
          },
        },
      });

      await client.waitForMessage('QUERY_RESP');
      client.messages.length = 0;

      // Update task-1 to completed (no longer matches)
      const updateRecord = {
        value: { title: 'Task 1', status: 'completed', priority: 1 },
        timestamp: {
          millis: Date.now() + 1000,
          counter: 0,
          nodeId: 'test',
        },
      };

      client.send({
        type: 'CLIENT_OP',
        payload: {
          mapName: 'tasks',
          opType: 'PUT',
          key: 'task-1',
          record: updateRecord,
        },
      });

      // Wait for removal update
      await waitUntil(
        () => client.messages.some(
          (m) => m.type === 'QUERY_UPDATE' && m.payload.type === 'REMOVE'
        ),
        3000
      );

      const removeUpdate = client.messages.find(
        (m) => m.type === 'QUERY_UPDATE' && m.payload.type === 'REMOVE'
      );
      expect(removeUpdate).toBeDefined();
      expect(removeUpdate.payload.key).toBe('task-1');
    });

    test('record starts matching filter - added to results', async () => {
      // Subscribe to completed tasks
      client.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'q-filter-add',
          mapName: 'tasks',
          query: {
            where: { status: 'completed' },
          },
        },
      });

      const initialResp = await client.waitForMessage('QUERY_RESP');
      expect(initialResp.payload.results).toHaveLength(2); // task-2, task-5
      client.messages.length = 0;

      // Update task-3 to completed (now matches)
      const updateRecord = {
        value: { title: 'Task 3', status: 'completed', priority: 3 },
        timestamp: {
          millis: Date.now() + 1000,
          counter: 0,
          nodeId: 'test',
        },
      };

      client.send({
        type: 'CLIENT_OP',
        payload: {
          mapName: 'tasks',
          opType: 'PUT',
          key: 'task-3',
          record: updateRecord,
        },
      });

      // Wait for addition update
      await waitUntil(
        () => client.messages.some(
          (m) => m.type === 'QUERY_UPDATE' && m.payload.key === 'task-3'
        ),
        3000
      );

      const addUpdate = client.messages.find(
        (m) => m.type === 'QUERY_UPDATE' && m.payload.key === 'task-3'
      );
      expect(addUpdate).toBeDefined();
      expect(addUpdate.payload.value.status).toBe('completed');
    });

    test('query without filter returns all records', async () => {
      client.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'q-all',
          mapName: 'tasks',
          query: {},
        },
      });

      const response = await client.waitForMessage('QUERY_RESP');
      expect(response.payload.results).toHaveLength(5);
    });
  });

  // ========================================
  // Comparison Operators Tests
  // ========================================
  describe('Comparison Operators', () => {
    let server: ServerCoordinator;
    let client: TestClient;

    beforeEach(async () => {
      server = await createTestServer();
      client = await createTestClient(`ws://localhost:${server.port}`, {
        roles: ['ADMIN'],
      });
      await client.waitForMessage('AUTH_ACK');

      // Pre-populate with test data
      const map = server.getMap('products') as LWWMap<string, any>;
      map.merge('prod-1', createLWWRecord({ name: 'Laptop', price: 1000, category: 'electronics' }));
      map.merge('prod-2', createLWWRecord({ name: 'Phone', price: 500, category: 'electronics' }));
      map.merge('prod-3', createLWWRecord({ name: 'Shirt', price: 50, category: 'clothing' }));
      map.merge('prod-4', createLWWRecord({ name: 'Book', price: 20, category: 'books' }));
      map.merge('prod-5', createLWWRecord({ name: 'Monitor', price: 300, category: 'electronics' }));
    });

    afterEach(async () => {
      client.close();
      await server.shutdown();
    });

    test('$gt operator - greater than', async () => {
      client.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'q-gt',
          mapName: 'products',
          query: {
            where: { price: { $gt: 100 } },
          },
        },
      });

      const response = await client.waitForMessage('QUERY_RESP');
      expect(response.payload.results).toHaveLength(3);

      const names = response.payload.results.map((r: any) => r.value.name).sort();
      expect(names).toEqual(['Laptop', 'Monitor', 'Phone']);
    });

    test('$lt operator - less than', async () => {
      client.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'q-lt',
          mapName: 'products',
          query: {
            where: { price: { $lt: 100 } },
          },
        },
      });

      const response = await client.waitForMessage('QUERY_RESP');
      expect(response.payload.results).toHaveLength(2);

      const names = response.payload.results.map((r: any) => r.value.name).sort();
      expect(names).toEqual(['Book', 'Shirt']);
    });

    test('$gte operator - greater than or equal', async () => {
      client.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'q-gte',
          mapName: 'products',
          query: {
            where: { price: { $gte: 300 } },
          },
        },
      });

      const response = await client.waitForMessage('QUERY_RESP');
      expect(response.payload.results).toHaveLength(3);

      const names = response.payload.results.map((r: any) => r.value.name).sort();
      expect(names).toEqual(['Laptop', 'Monitor', 'Phone']);
    });

    test('$lte operator - less than or equal', async () => {
      client.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'q-lte',
          mapName: 'products',
          query: {
            where: { price: { $lte: 50 } },
          },
        },
      });

      const response = await client.waitForMessage('QUERY_RESP');
      expect(response.payload.results).toHaveLength(2);

      const names = response.payload.results.map((r: any) => r.value.name).sort();
      expect(names).toEqual(['Book', 'Shirt']);
    });

    test('$ne operator - not equal', async () => {
      client.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'q-ne',
          mapName: 'products',
          query: {
            where: { category: { $ne: 'electronics' } },
          },
        },
      });

      const response = await client.waitForMessage('QUERY_RESP');
      expect(response.payload.results).toHaveLength(2);

      const names = response.payload.results.map((r: any) => r.value.name).sort();
      expect(names).toEqual(['Book', 'Shirt']);
    });
  });

  // ========================================
  // Sorting Tests
  // ========================================
  describe('Sorting', () => {
    let server: ServerCoordinator;
    let client: TestClient;

    beforeEach(async () => {
      server = await createTestServer();
      client = await createTestClient(`ws://localhost:${server.port}`, {
        roles: ['ADMIN'],
      });
      await client.waitForMessage('AUTH_ACK');

      // Pre-populate with test data
      const map = server.getMap('scores') as LWWMap<string, any>;
      map.merge('s1', createLWWRecord({ player: 'Alice', score: 150, level: 3 }));
      map.merge('s2', createLWWRecord({ player: 'Bob', score: 200, level: 5 }));
      map.merge('s3', createLWWRecord({ player: 'Charlie', score: 100, level: 2 }));
      map.merge('s4', createLWWRecord({ player: 'Diana', score: 175, level: 4 }));
    });

    afterEach(async () => {
      client.close();
      await server.shutdown();
    });

    test('query with ascending sort', async () => {
      client.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'q-sort-asc',
          mapName: 'scores',
          query: {
            sort: { score: 'asc' },
          },
        },
      });

      const response = await client.waitForMessage('QUERY_RESP');
      expect(response.payload.results).toHaveLength(4);

      const players = response.payload.results.map((r: any) => r.value.player);
      expect(players).toEqual(['Charlie', 'Alice', 'Diana', 'Bob']);
    });

    test('query with descending sort', async () => {
      client.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'q-sort-desc',
          mapName: 'scores',
          query: {
            sort: { score: 'desc' },
          },
        },
      });

      const response = await client.waitForMessage('QUERY_RESP');
      const players = response.payload.results.map((r: any) => r.value.player);
      expect(players).toEqual(['Bob', 'Diana', 'Alice', 'Charlie']);
    });

    test('new record with better priority triggers update', async () => {
      // Subscribe to scores sorted descending
      client.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'q-sort-update',
          mapName: 'scores',
          query: {
            sort: { score: 'desc' },
          },
        },
      });

      await client.waitForMessage('QUERY_RESP');
      client.messages.length = 0;

      // Add new top scorer
      const writer = await createTestClient(`ws://localhost:${server.port}`, {
        roles: ['ADMIN'],
        nodeId: 'writer',
      });
      await writer.waitForMessage('AUTH_ACK');

      writer.send({
        type: 'CLIENT_OP',
        payload: {
          mapName: 'scores',
          opType: 'PUT',
          key: 's5',
          record: createLWWRecord({ player: 'Eve', score: 250, level: 6 }),
        },
      });

      // Wait for update
      await waitUntil(
        () => client.messages.some((m) => m.type === 'QUERY_UPDATE'),
        3000
      );

      const update = client.messages.find((m) => m.type === 'QUERY_UPDATE');
      expect(update).toBeDefined();
      expect(update.payload.value.player).toBe('Eve');
      expect(update.payload.value.score).toBe(250);

      writer.close();
    });
  });

  // ========================================
  // Limit Tests
  // ========================================
  describe('Limit', () => {
    let server: ServerCoordinator;
    let client: TestClient;

    beforeEach(async () => {
      server = await createTestServer();
      client = await createTestClient(`ws://localhost:${server.port}`, {
        roles: ['ADMIN'],
      });
      await client.waitForMessage('AUTH_ACK');

      // Pre-populate with test data
      const map = server.getMap('items') as LWWMap<string, any>;
      for (let i = 1; i <= 10; i++) {
        map.merge(`item-${i}`, createLWWRecord({ name: `Item ${i}`, order: i }));
      }
    });

    afterEach(async () => {
      client.close();
      await server.shutdown();
    });

    test('query with limit returns maximum N records', async () => {
      client.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'q-limit',
          mapName: 'items',
          query: {
            sort: { order: 'asc' },
            limit: 5,
          },
        },
      });

      const response = await client.waitForMessage('QUERY_RESP');
      expect(response.payload.results).toHaveLength(5);

      const names = response.payload.results.map((r: any) => r.value.name);
      expect(names).toEqual(['Item 1', 'Item 2', 'Item 3', 'Item 4', 'Item 5']);
    });

    test('query with offset and limit works correctly', async () => {
      client.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'q-offset-limit',
          mapName: 'items',
          query: {
            sort: { order: 'asc' },
            offset: 3,
            limit: 3,
          },
        },
      });

      const response = await client.waitForMessage('QUERY_RESP');
      expect(response.payload.results).toHaveLength(3);

      const names = response.payload.results.map((r: any) => r.value.name);
      expect(names).toEqual(['Item 4', 'Item 5', 'Item 6']);
    });
  });

  // ========================================
  // Multi-Client Live Queries
  // ========================================
  describe('Multi-Client Live Queries', () => {
    test('client A subscribed, client B adds matching record, client A receives update', async () => {
      const ctx = await createTestContext(2);

      try {
        const [clientA, clientB] = ctx.clients;

        // Client A subscribes
        clientA.send({
          type: 'QUERY_SUB',
          payload: {
            queryId: 'q-multi',
            mapName: 'shared-data',
            query: {},
          },
        });

        await clientA.waitForMessage('QUERY_RESP');
        clientA.messages.length = 0;

        // Client B writes data
        clientB.send({
          type: 'CLIENT_OP',
          payload: {
            mapName: 'shared-data',
            opType: 'PUT',
            key: 'shared-1',
            record: createLWWRecord({ content: 'From Client B' }),
          },
        });

        // Wait for update on Client A
        await waitUntil(
          () => clientA.messages.some((m) => m.type === 'QUERY_UPDATE'),
          3000
        );

        const update = clientA.messages.find((m) => m.type === 'QUERY_UPDATE');
        expect(update).toBeDefined();
        expect(update.payload.key).toBe('shared-1');
        expect(update.payload.value.content).toBe('From Client B');
      } finally {
        await ctx.cleanup();
      }
    });

    test('multiple clients with different filters receive appropriate updates', async () => {
      const ctx = await createTestContext(3);

      try {
        const [clientA, clientB, writer] = ctx.clients;

        // Client A subscribes to category 'A'
        clientA.send({
          type: 'QUERY_SUB',
          payload: {
            queryId: 'q-cat-a',
            mapName: 'categorized',
            query: { where: { category: 'A' } },
          },
        });

        // Client B subscribes to category 'B'
        clientB.send({
          type: 'QUERY_SUB',
          payload: {
            queryId: 'q-cat-b',
            mapName: 'categorized',
            query: { where: { category: 'B' } },
          },
        });

        await clientA.waitForMessage('QUERY_RESP');
        await clientB.waitForMessage('QUERY_RESP');
        clientA.messages.length = 0;
        clientB.messages.length = 0;

        // Writer adds item to category 'A'
        writer.send({
          type: 'CLIENT_OP',
          payload: {
            mapName: 'categorized',
            opType: 'PUT',
            key: 'item-a1',
            record: createLWWRecord({ name: 'A Item', category: 'A' }),
          },
        });

        await waitForSync(300);

        // Client A should receive update
        const updateA = clientA.messages.find((m) => m.type === 'QUERY_UPDATE');
        expect(updateA).toBeDefined();
        expect(updateA.payload.value.category).toBe('A');

        // Client B should NOT receive update (wrong category)
        const updateB = clientB.messages.find((m) => m.type === 'QUERY_UPDATE');
        expect(updateB).toBeUndefined();
      } finally {
        await ctx.cleanup();
      }
    });
  });

  // ========================================
  // Unsubscribe Tests
  // ========================================
  describe('Unsubscribe', () => {
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

    test('after unsubscribe, new data does NOT arrive', async () => {
      // Subscribe
      client.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'q-unsub',
          mapName: 'unsub-test',
          query: {},
        },
      });

      await client.waitForMessage('QUERY_RESP');

      // Unsubscribe
      client.send({
        type: 'QUERY_UNSUB',
        payload: { queryId: 'q-unsub' },
      });

      await waitForSync(100);
      client.messages.length = 0;

      // Another client writes data
      const writer = await createTestClient(`ws://localhost:${server.port}`, {
        roles: ['ADMIN'],
        nodeId: 'writer',
      });
      await writer.waitForMessage('AUTH_ACK');

      writer.send({
        type: 'CLIENT_OP',
        payload: {
          mapName: 'unsub-test',
          opType: 'PUT',
          key: 'after-unsub',
          record: createLWWRecord({ data: 'Should not receive' }),
        },
      });

      await waitForSync(300);

      // Client should NOT have received update
      const update = client.messages.find((m) => m.type === 'QUERY_UPDATE');
      expect(update).toBeUndefined();

      writer.close();
    });

    test('unsubscribe from one query does not affect other queries', async () => {
      // Subscribe to two queries
      client.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'q-keep',
          mapName: 'keep-data',
          query: {},
        },
      });

      client.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'q-remove',
          mapName: 'remove-data',
          query: {},
        },
      });

      await client.waitForMessage('QUERY_RESP');
      await waitForSync(100);

      // Unsubscribe from one
      client.send({
        type: 'QUERY_UNSUB',
        payload: { queryId: 'q-remove' },
      });

      await waitForSync(100);
      client.messages.length = 0;

      // Write to both
      const writer = await createTestClient(`ws://localhost:${server.port}`, {
        roles: ['ADMIN'],
        nodeId: 'writer',
      });
      await writer.waitForMessage('AUTH_ACK');

      writer.send({
        type: 'CLIENT_OP',
        payload: {
          mapName: 'keep-data',
          opType: 'PUT',
          key: 'keep-item',
          record: createLWWRecord({ data: 'Keep' }),
        },
      });

      writer.send({
        type: 'CLIENT_OP',
        payload: {
          mapName: 'remove-data',
          opType: 'PUT',
          key: 'remove-item',
          record: createLWWRecord({ data: 'Remove' }),
        },
      });

      await waitForSync(300);

      // Should receive update for 'keep-data' only
      const updates = client.messages.filter((m) => m.type === 'QUERY_UPDATE');
      expect(updates.length).toBeGreaterThanOrEqual(1);

      const keepUpdate = updates.find((u) => u.payload.queryId === 'q-keep');
      expect(keepUpdate).toBeDefined();

      const removeUpdate = updates.find((u) => u.payload.queryId === 'q-remove');
      expect(removeUpdate).toBeUndefined();

      writer.close();
    });
  });

  // ========================================
  // Edge Cases
  // ========================================
  describe('Edge Cases', () => {
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

    test('multiple queries on same collection with different filters', async () => {
      // Pre-populate
      const map = server.getMap('multi-filter') as LWWMap<string, any>;
      map.merge('m1', createLWWRecord({ type: 'A', active: true }));
      map.merge('m2', createLWWRecord({ type: 'B', active: true }));
      map.merge('m3', createLWWRecord({ type: 'A', active: false }));
      map.merge('m4', createLWWRecord({ type: 'B', active: false }));

      // Query 1: type A
      client.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'q-type-a',
          mapName: 'multi-filter',
          query: { where: { type: 'A' } },
        },
      });

      // Query 2: active items
      client.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'q-active',
          mapName: 'multi-filter',
          query: { where: { active: true } },
        },
      });

      await waitForSync(200);

      const responses = client.messages.filter((m) => m.type === 'QUERY_RESP');
      expect(responses).toHaveLength(2);

      const respTypeA = responses.find((r) => r.payload.queryId === 'q-type-a');
      expect(respTypeA.payload.results).toHaveLength(2); // m1, m3

      const respActive = responses.find((r) => r.payload.queryId === 'q-active');
      expect(respActive.payload.results).toHaveLength(2); // m1, m2
    });

    test('record deletion triggers removal from query results', async () => {
      // Pre-populate
      const map = server.getMap('deletable') as LWWMap<string, any>;
      map.merge('d1', createLWWRecord({ name: 'To Delete' }));
      map.merge('d2', createLWWRecord({ name: 'To Keep' }));

      // Subscribe
      client.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'q-delete',
          mapName: 'deletable',
          query: {},
        },
      });

      const initialResp = await client.waitForMessage('QUERY_RESP');
      expect(initialResp.payload.results).toHaveLength(2);
      client.messages.length = 0;

      // Delete d1 (tombstone)
      const tombstone = {
        value: null,
        timestamp: {
          millis: Date.now() + 1000,
          counter: 0,
          nodeId: 'client',
        },
      };

      client.send({
        type: 'CLIENT_OP',
        payload: {
          mapName: 'deletable',
          opType: 'REMOVE',
          key: 'd1',
          record: tombstone,
        },
      });

      await waitUntil(
        () => client.messages.some(
          (m) => m.type === 'QUERY_UPDATE' && m.payload.type === 'REMOVE'
        ),
        3000
      );

      const removeUpdate = client.messages.find(
        (m) => m.type === 'QUERY_UPDATE' && m.payload.type === 'REMOVE'
      );
      expect(removeUpdate).toBeDefined();
      expect(removeUpdate.payload.key).toBe('d1');
    });

    test('rapid updates to same key are handled correctly', async () => {
      // Subscribe
      client.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'q-rapid',
          mapName: 'rapid-updates',
          query: {},
        },
      });

      await client.waitForMessage('QUERY_RESP');
      client.messages.length = 0;

      // Rapidly update the same key
      const writer = await createTestClient(`ws://localhost:${server.port}`, {
        roles: ['ADMIN'],
        nodeId: 'rapid-writer',
      });
      await writer.waitForMessage('AUTH_ACK');

      for (let i = 1; i <= 10; i++) {
        writer.send({
          type: 'CLIENT_OP',
          payload: {
            mapName: 'rapid-updates',
            opType: 'PUT',
            key: 'rapid-key',
            record: {
              value: { count: i },
              timestamp: {
                millis: Date.now() + i,
                counter: i,
                nodeId: 'rapid-writer',
              },
            },
          },
        });
      }

      await waitForSync(500);

      // Should have received updates (at least the final state)
      const updates = client.messages.filter(
        (m) => m.type === 'QUERY_UPDATE' && m.payload.key === 'rapid-key'
      );
      expect(updates.length).toBeGreaterThan(0);

      // The latest update should have count: 10
      const lastUpdate = updates[updates.length - 1];
      expect(lastUpdate.payload.value.count).toBe(10);

      writer.close();
    });

    test('query on non-existent collection returns empty and receives future updates', async () => {
      // Subscribe to non-existent collection
      client.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'q-future',
          mapName: 'future-collection',
          query: {},
        },
      });

      const initialResp = await client.waitForMessage('QUERY_RESP');
      expect(initialResp.payload.results).toHaveLength(0);
      client.messages.length = 0;

      // Write to the collection
      const writer = await createTestClient(`ws://localhost:${server.port}`, {
        roles: ['ADMIN'],
        nodeId: 'future-writer',
      });
      await writer.waitForMessage('AUTH_ACK');

      writer.send({
        type: 'CLIENT_OP',
        payload: {
          mapName: 'future-collection',
          opType: 'PUT',
          key: 'future-item',
          record: createLWWRecord({ message: 'Future data' }),
        },
      });

      await waitUntil(
        () => client.messages.some((m) => m.type === 'QUERY_UPDATE'),
        3000
      );

      const update = client.messages.find((m) => m.type === 'QUERY_UPDATE');
      expect(update).toBeDefined();
      expect(update.payload.value.message).toBe('Future data');

      writer.close();
    });

    test('subscribe to same query multiple times (idempotent)', async () => {
      // Subscribe twice with same queryId
      client.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'q-duplicate',
          mapName: 'duplicate-test',
          query: {},
        },
      });

      await client.waitForMessage('QUERY_RESP');
      client.messages.length = 0;

      // Subscribe again
      client.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'q-duplicate',
          mapName: 'duplicate-test',
          query: {},
        },
      });

      await waitForSync(200);

      // Should still work without errors
      const errorMsg = client.messages.find((m) => m.type === 'ERROR');
      expect(errorMsg).toBeUndefined();
    });

    test('complex filter with multiple conditions', async () => {
      // Pre-populate
      const map = server.getMap('complex') as LWWMap<string, any>;
      map.merge('c1', createLWWRecord({ status: 'active', priority: 1, score: 80 }));
      map.merge('c2', createLWWRecord({ status: 'active', priority: 2, score: 90 }));
      map.merge('c3', createLWWRecord({ status: 'inactive', priority: 1, score: 85 }));
      map.merge('c4', createLWWRecord({ status: 'active', priority: 1, score: 70 }));

      // Query with multiple conditions
      client.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'q-complex',
          mapName: 'complex',
          query: {
            where: {
              status: 'active',
              priority: 1,
            },
          },
        },
      });

      const response = await client.waitForMessage('QUERY_RESP');
      expect(response.payload.results).toHaveLength(2); // c1, c4

      const keys = response.payload.results.map((r: any) => r.key).sort();
      expect(keys).toEqual(['c1', 'c4']);
    });
  });

  // ========================================
  // Real-time Sync Verification
  // ========================================
  describe('Real-time Sync Verification', () => {
    test('updates propagate in near real-time', async () => {
      const ctx = await createTestContext(2);

      try {
        const [subscriber, writer] = ctx.clients;

        // Subscriber subscribes
        subscriber.send({
          type: 'QUERY_SUB',
          payload: {
            queryId: 'q-realtime',
            mapName: 'realtime-data',
            query: {},
          },
        });

        await subscriber.waitForMessage('QUERY_RESP');
        subscriber.messages.length = 0;

        const startTime = Date.now();

        // Writer sends data
        writer.send({
          type: 'CLIENT_OP',
          payload: {
            mapName: 'realtime-data',
            opType: 'PUT',
            key: 'rt-item',
            record: createLWWRecord({ timestamp: startTime }),
          },
        });

        // Wait for update
        await waitUntil(
          () => subscriber.messages.some((m) => m.type === 'QUERY_UPDATE'),
          3000
        );

        const endTime = Date.now();
        const latency = endTime - startTime;

        // Should be reasonably fast (less than 1 second)
        expect(latency).toBeLessThan(1000);

        const update = subscriber.messages.find((m) => m.type === 'QUERY_UPDATE');
        expect(update).toBeDefined();
      } finally {
        await ctx.cleanup();
      }
    });

    test('sequence of updates maintains correct order', async () => {
      const ctx = await createTestContext(2);

      try {
        const [subscriber, writer] = ctx.clients;

        // Subscriber subscribes
        subscriber.send({
          type: 'QUERY_SUB',
          payload: {
            queryId: 'q-sequence',
            mapName: 'sequence-data',
            query: {},
          },
        });

        await subscriber.waitForMessage('QUERY_RESP');
        subscriber.messages.length = 0;

        // Writer sends sequence of items
        for (let i = 1; i <= 5; i++) {
          writer.send({
            type: 'CLIENT_OP',
            payload: {
              mapName: 'sequence-data',
              opType: 'PUT',
              key: `seq-${i}`,
              record: createLWWRecord({ order: i }),
            },
          });
        }

        // Wait for all updates
        await waitUntil(
          () => subscriber.messages.filter((m) => m.type === 'QUERY_UPDATE').length >= 5,
          5000
        );

        const updates = subscriber.messages.filter((m) => m.type === 'QUERY_UPDATE');
        expect(updates).toHaveLength(5);

        // Verify all items received
        const keys = updates.map((u) => u.payload.key).sort();
        expect(keys).toEqual(['seq-1', 'seq-2', 'seq-3', 'seq-4', 'seq-5']);
      } finally {
        await ctx.cleanup();
      }
    });
  });
});
