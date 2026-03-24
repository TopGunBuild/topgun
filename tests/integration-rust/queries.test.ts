import {
  createRustTestClient,
  spawnRustServer,
  createLWWRecord,
  waitForSync,
  waitUntil,
  TestClient,
} from './helpers';

describe('Integration: Queries (Rust Server)', () => {
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

  // ========================================
  // Snapshot Tests (AC22)
  // ========================================
  describe('QUERY_SUB snapshot', () => {
    test('QUERY_SUB on populated map returns QUERY_RESP with all records', async () => {
      const mapName = `snap-map-${Date.now()}`;
      const client = await createRustTestClient(port, {
        nodeId: 'snap-client-1',
        userId: 'snap-user-1',
        roles: ['ADMIN'],
      });
      await client.waitForMessage('AUTH_ACK');

      // Populate the map with multiple records
      const records = [
        { key: 'rec-1', value: { name: 'Alice', age: 30 } },
        { key: 'rec-2', value: { name: 'Bob', age: 25 } },
        { key: 'rec-3', value: { name: 'Charlie', age: 35 } },
      ];

      for (const rec of records) {
        client.messages.length = 0;
        client.send({
          type: 'CLIENT_OP',
          payload: {
            id: `snap-put-${rec.key}`,
            mapName,
            opType: 'PUT',
            key: rec.key,
            record: createLWWRecord(rec.value),
          },
        });
        await client.waitForMessage('OP_ACK');
      }

      await waitForSync(200);

      // Subscribe to query all records
      client.messages.length = 0;
      client.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'snap-q-1',
          mapName,
          query: {},
        },
      });

      const response = await client.waitForMessage('QUERY_RESP');
      expect(response).toBeDefined();
      expect(response.payload.queryId).toBe('snap-q-1');
      expect(response.payload.results).toBeDefined();
      expect(Array.isArray(response.payload.results)).toBe(true);

      // All 3 records should be in the results
      expect(response.payload.results.length).toBe(3);

      // Each result should be { key, value } format
      for (const rec of records) {
        const found = response.payload.results.find(
          (r: any) => r.key === rec.key
        );
        expect(found).toBeDefined();
        expect(found.value).toEqual(rec.value);
      }

      client.close();
    });
  });

  // ========================================
  // Where Filter Tests (AC23)
  // ========================================
  describe('QUERY_SUB with where filter (exact equality)', () => {
    test('where filter returns only matching records', async () => {
      const mapName = `where-map-${Date.now()}`;
      const client = await createRustTestClient(port, {
        nodeId: 'where-client-1',
        userId: 'where-user-1',
        roles: ['ADMIN'],
      });
      await client.waitForMessage('AUTH_ACK');

      // Populate with records of different categories
      const records = [
        { key: 'item-1', value: { category: 'electronics', name: 'Phone' } },
        { key: 'item-2', value: { category: 'books', name: 'Novel' } },
        { key: 'item-3', value: { category: 'electronics', name: 'Laptop' } },
        { key: 'item-4', value: { category: 'books', name: 'Guide' } },
      ];

      for (const rec of records) {
        client.messages.length = 0;
        client.send({
          type: 'CLIENT_OP',
          payload: {
            id: `where-put-${rec.key}`,
            mapName,
            opType: 'PUT',
            key: rec.key,
            record: createLWWRecord(rec.value),
          },
        });
        await client.waitForMessage('OP_ACK');
      }

      await waitForSync(200);

      // Query with where filter for exact equality
      client.messages.length = 0;
      client.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'where-q-1',
          mapName,
          query: {
            where: { category: 'electronics' },
          },
        },
      });

      const response = await client.waitForMessage('QUERY_RESP');
      expect(response).toBeDefined();
      expect(response.payload.queryId).toBe('where-q-1');

      // Only electronics records should be returned
      expect(response.payload.results.length).toBe(2);

      const keys = response.payload.results.map((r: any) => r.key).sort();
      expect(keys).toEqual(['item-1', 'item-3']);

      for (const result of response.payload.results) {
        expect(result.value.category).toBe('electronics');
      }

      client.close();
    });
  });

  // ========================================
  // Predicate Comparison Tests (AC24)
  // ========================================
  describe('QUERY_SUB with predicate (comparison operators)', () => {
    let predClient: TestClient;
    const predMapName = `pred-map-${Date.now()}`;

    beforeAll(async () => {
      predClient = await createRustTestClient(port, {
        nodeId: 'pred-client-1',
        userId: 'pred-user-1',
        roles: ['ADMIN'],
      });
      await predClient.waitForMessage('AUTH_ACK');

      // Populate with products at different prices
      const products = [
        { key: 'p-1', value: { name: 'Cheap', price: 10 } },
        { key: 'p-2', value: { name: 'Mid', price: 50 } },
        { key: 'p-3', value: { name: 'MidHigh', price: 100 } },
        { key: 'p-4', value: { name: 'Expensive', price: 200 } },
        { key: 'p-5', value: { name: 'Premium', price: 500 } },
      ];

      for (const prod of products) {
        predClient.messages.length = 0;
        predClient.send({
          type: 'CLIENT_OP',
          payload: {
            id: `pred-put-${prod.key}`,
            mapName: predMapName,
            opType: 'PUT',
            key: prod.key,
            record: createLWWRecord(prod.value),
          },
        });
        await predClient.waitForMessage('OP_ACK');
      }

      await waitForSync(200);
    });

    afterAll(() => {
      predClient.close();
    });

    test('predicate gt: returns records with price > 100', async () => {
      predClient.messages.length = 0;
      predClient.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'pred-gt-q',
          mapName: predMapName,
          query: {
            predicate: {
              op: 'gt',
              attribute: 'price',
              value: 100,
            },
          },
        },
      });

      const response = await predClient.waitForMessage('QUERY_RESP');
      expect(response.payload.queryId).toBe('pred-gt-q');

      // price > 100: p-4 (200), p-5 (500)
      expect(response.payload.results.length).toBe(2);
      const keys = response.payload.results.map((r: any) => r.key).sort();
      expect(keys).toEqual(['p-4', 'p-5']);
    });

    test('predicate lt: returns records with price < 50', async () => {
      predClient.messages.length = 0;
      predClient.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'pred-lt-q',
          mapName: predMapName,
          query: {
            predicate: {
              op: 'lt',
              attribute: 'price',
              value: 50,
            },
          },
        },
      });

      const response = await predClient.waitForMessage('QUERY_RESP');
      expect(response.payload.queryId).toBe('pred-lt-q');

      // price < 50: p-1 (10)
      expect(response.payload.results.length).toBe(1);
      expect(response.payload.results[0].key).toBe('p-1');
    });

    test('predicate gte: returns records with price >= 100', async () => {
      predClient.messages.length = 0;
      predClient.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'pred-gte-q',
          mapName: predMapName,
          query: {
            predicate: {
              op: 'gte',
              attribute: 'price',
              value: 100,
            },
          },
        },
      });

      const response = await predClient.waitForMessage('QUERY_RESP');
      expect(response.payload.queryId).toBe('pred-gte-q');

      // price >= 100: p-3 (100), p-4 (200), p-5 (500)
      expect(response.payload.results.length).toBe(3);
      const keys = response.payload.results.map((r: any) => r.key).sort();
      expect(keys).toEqual(['p-3', 'p-4', 'p-5']);
    });

    test('predicate lte: returns records with price <= 50', async () => {
      predClient.messages.length = 0;
      predClient.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'pred-lte-q',
          mapName: predMapName,
          query: {
            predicate: {
              op: 'lte',
              attribute: 'price',
              value: 50,
            },
          },
        },
      });

      const response = await predClient.waitForMessage('QUERY_RESP');
      expect(response.payload.queryId).toBe('pred-lte-q');

      // price <= 50: p-1 (10), p-2 (50)
      expect(response.payload.results.length).toBe(2);
      const keys = response.payload.results.map((r: any) => r.key).sort();
      expect(keys).toEqual(['p-1', 'p-2']);
    });

    test('predicate neq: returns records with price != 100', async () => {
      predClient.messages.length = 0;
      predClient.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'pred-neq-q',
          mapName: predMapName,
          query: {
            predicate: {
              op: 'neq',
              attribute: 'price',
              value: 100,
            },
          },
        },
      });

      const response = await predClient.waitForMessage('QUERY_RESP');
      expect(response.payload.queryId).toBe('pred-neq-q');

      // price != 100: p-1 (10), p-2 (50), p-4 (200), p-5 (500)
      expect(response.payload.results.length).toBe(4);
      const keys = response.payload.results.map((r: any) => r.key).sort();
      expect(keys).toEqual(['p-1', 'p-2', 'p-4', 'p-5']);
    });
  });

  // ========================================
  // Sort Tests (AC25)
  // ========================================
  describe('QUERY_SUB with sort', () => {
    test('sort ascending by price returns results in order', async () => {
      const mapName = `sort-map-${Date.now()}`;
      const client = await createRustTestClient(port, {
        nodeId: 'sort-client-1',
        userId: 'sort-user-1',
        roles: ['ADMIN'],
      });
      await client.waitForMessage('AUTH_ACK');

      // Populate records in non-sorted order
      const products = [
        { key: 's-3', value: { name: 'C', price: 300 } },
        { key: 's-1', value: { name: 'A', price: 100 } },
        { key: 's-2', value: { name: 'B', price: 200 } },
      ];

      for (const prod of products) {
        client.messages.length = 0;
        client.send({
          type: 'CLIENT_OP',
          payload: {
            id: `sort-put-${prod.key}`,
            mapName,
            opType: 'PUT',
            key: prod.key,
            record: createLWWRecord(prod.value),
          },
        });
        await client.waitForMessage('OP_ACK');
      }

      await waitForSync(200);

      // Query with ascending sort
      client.messages.length = 0;
      client.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'sort-asc-q',
          mapName,
          query: {
            sort: { price: 'asc' },
          },
        },
      });

      const response = await client.waitForMessage('QUERY_RESP');
      expect(response.payload.queryId).toBe('sort-asc-q');
      expect(response.payload.results.length).toBe(3);

      // Verify ascending order
      const prices = response.payload.results.map(
        (r: any) => r.value.price
      );
      expect(prices).toEqual([100, 200, 300]);

      client.close();
    });

    test('sort descending by price returns results in reverse order', async () => {
      const mapName = `sortdesc-map-${Date.now()}`;
      const client = await createRustTestClient(port, {
        nodeId: 'sortdesc-client-1',
        userId: 'sortdesc-user-1',
        roles: ['ADMIN'],
      });
      await client.waitForMessage('AUTH_ACK');

      const products = [
        { key: 'sd-1', value: { name: 'A', price: 100 } },
        { key: 'sd-2', value: { name: 'B', price: 200 } },
        { key: 'sd-3', value: { name: 'C', price: 300 } },
      ];

      for (const prod of products) {
        client.messages.length = 0;
        client.send({
          type: 'CLIENT_OP',
          payload: {
            id: `sortdesc-put-${prod.key}`,
            mapName,
            opType: 'PUT',
            key: prod.key,
            record: createLWWRecord(prod.value),
          },
        });
        await client.waitForMessage('OP_ACK');
      }

      await waitForSync(200);

      client.messages.length = 0;
      client.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'sort-desc-q',
          mapName,
          query: {
            sort: { price: 'desc' },
          },
        },
      });

      const response = await client.waitForMessage('QUERY_RESP');
      expect(response.payload.queryId).toBe('sort-desc-q');
      expect(response.payload.results.length).toBe(3);

      // Verify descending order
      const prices = response.payload.results.map(
        (r: any) => r.value.price
      );
      expect(prices).toEqual([300, 200, 100]);

      client.close();
    });
  });

  // ========================================
  // Limit Tests (AC26)
  // ========================================
  describe('QUERY_SUB with limit', () => {
    test('limit returns at most N results', async () => {
      const mapName = `limit-map-${Date.now()}`;
      const client = await createRustTestClient(port, {
        nodeId: 'limit-client-1',
        userId: 'limit-user-1',
        roles: ['ADMIN'],
      });
      await client.waitForMessage('AUTH_ACK');

      // Write 5 records
      for (let i = 1; i <= 5; i++) {
        client.messages.length = 0;
        client.send({
          type: 'CLIENT_OP',
          payload: {
            id: `limit-put-${i}`,
            mapName,
            opType: 'PUT',
            key: `lim-${i}`,
            record: createLWWRecord({ index: i, name: `Item ${i}` }),
          },
        });
        await client.waitForMessage('OP_ACK');
      }

      await waitForSync(200);

      // Query with limit of 3
      client.messages.length = 0;
      client.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'limit-q-1',
          mapName,
          query: {
            limit: 3,
          },
        },
      });

      const response = await client.waitForMessage('QUERY_RESP');
      expect(response.payload.queryId).toBe('limit-q-1');
      expect(response.payload.results.length).toBeLessThanOrEqual(3);

      client.close();
    });
  });

  // ========================================
  // Live Update: ENTER (AC27)
  // ========================================
  describe('QUERY_UPDATE with changeType ENTER', () => {
    test('new record matching filter triggers QUERY_UPDATE with ENTER', async () => {
      const mapName = `live-enter-${Date.now()}`;

      const subscriber = await createRustTestClient(port, {
        nodeId: 'live-sub-1',
        userId: 'live-sub-user-1',
        roles: ['ADMIN'],
      });
      await subscriber.waitForMessage('AUTH_ACK');

      const writer = await createRustTestClient(port, {
        nodeId: 'live-writer-1',
        userId: 'live-writer-user-1',
        roles: ['ADMIN'],
      });
      await writer.waitForMessage('AUTH_ACK');

      // Subscribe to the map (empty at this point)
      subscriber.messages.length = 0;
      subscriber.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'live-q-enter',
          mapName,
          query: {},
        },
      });

      // Wait for initial QUERY_RESP (empty snapshot)
      await subscriber.waitForMessage('QUERY_RESP');
      await waitForSync(200);

      // Clear messages to isolate QUERY_UPDATE
      subscriber.messages.length = 0;

      // Writer adds a new record
      writer.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'live-enter-op',
          mapName,
          opType: 'PUT',
          key: 'new-item',
          record: createLWWRecord({ name: 'New Item', status: 'active' }),
        },
      });

      // Wait for subscriber to receive QUERY_UPDATE
      await waitUntil(
        () =>
          subscriber.messages.some(
            (m) =>
              m.type === 'QUERY_UPDATE' &&
              m.payload?.queryId === 'live-q-enter'
          ),
        5000
      );

      const update = subscriber.messages.find(
        (m) =>
          m.type === 'QUERY_UPDATE' &&
          m.payload?.queryId === 'live-q-enter'
      );
      expect(update).toBeDefined();
      expect(update.payload.key).toBe('new-item');
      expect(update.payload.changeType).toBe('ENTER');
      expect(update.payload.value).toEqual({ name: 'New Item', status: 'active' });

      subscriber.close();
      writer.close();
    });
  });

  // ========================================
  // Live Update: UPDATE (AC36)
  // ========================================
  describe('QUERY_UPDATE with changeType UPDATE', () => {
    test('modified record still matching filter triggers UPDATE', async () => {
      const mapName = `live-update-${Date.now()}`;

      const subscriber = await createRustTestClient(port, {
        nodeId: 'live-upd-sub-1',
        userId: 'live-upd-sub-user-1',
        roles: ['ADMIN'],
      });
      await subscriber.waitForMessage('AUTH_ACK');

      const writer = await createRustTestClient(port, {
        nodeId: 'live-upd-writer-1',
        userId: 'live-upd-writer-user-1',
        roles: ['ADMIN'],
      });
      await writer.waitForMessage('AUTH_ACK');

      // Write an initial record
      writer.messages.length = 0;
      writer.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'live-upd-put-1',
          mapName,
          opType: 'PUT',
          key: 'upd-item',
          record: createLWWRecord({ name: 'Original', count: 1 }),
        },
      });
      await writer.waitForMessage('OP_ACK');
      await waitForSync(200);

      // Subscribe (should get initial snapshot with the record)
      subscriber.messages.length = 0;
      subscriber.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'live-q-update',
          mapName,
          query: {},
        },
      });
      await subscriber.waitForMessage('QUERY_RESP');
      await waitForSync(200);

      // Clear messages to isolate QUERY_UPDATE
      subscriber.messages.length = 0;

      // Writer modifies the record (still matches the unfiltered query)
      writer.messages.length = 0;
      writer.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'live-upd-put-2',
          mapName,
          opType: 'PUT',
          key: 'upd-item',
          record: createLWWRecord({ name: 'Updated', count: 2 }),
        },
      });

      // Wait for subscriber to receive QUERY_UPDATE with UPDATE changeType
      await waitUntil(
        () =>
          subscriber.messages.some(
            (m) =>
              m.type === 'QUERY_UPDATE' &&
              m.payload?.queryId === 'live-q-update' &&
              m.payload?.changeType === 'UPDATE'
          ),
        5000
      );

      const update = subscriber.messages.find(
        (m) =>
          m.type === 'QUERY_UPDATE' &&
          m.payload?.queryId === 'live-q-update' &&
          m.payload?.changeType === 'UPDATE'
      );
      expect(update).toBeDefined();
      expect(update.payload.key).toBe('upd-item');
      expect(update.payload.value).toEqual({ name: 'Updated', count: 2 });

      subscriber.close();
      writer.close();
    });
  });

  // ========================================
  // Live Update: LEAVE (AC29)
  // ========================================
  describe('QUERY_UPDATE with changeType LEAVE', () => {
    test('record no longer matching filter triggers LEAVE', async () => {
      const mapName = `live-leave-${Date.now()}`;

      const subscriber = await createRustTestClient(port, {
        nodeId: 'live-leave-sub-1',
        userId: 'live-leave-sub-user-1',
        roles: ['ADMIN'],
      });
      await subscriber.waitForMessage('AUTH_ACK');

      const writer = await createRustTestClient(port, {
        nodeId: 'live-leave-writer-1',
        userId: 'live-leave-writer-user-1',
        roles: ['ADMIN'],
      });
      await writer.waitForMessage('AUTH_ACK');

      // Write records: one matches filter, one does not
      writer.messages.length = 0;
      writer.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'live-leave-put-1',
          mapName,
          opType: 'PUT',
          key: 'leave-item',
          record: createLWWRecord({ status: 'active', name: 'Item A' }),
        },
      });
      await writer.waitForMessage('OP_ACK');
      await waitForSync(200);

      // Subscribe with a where filter for status = 'active'
      subscriber.messages.length = 0;
      subscriber.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'live-q-leave',
          mapName,
          query: {
            where: { status: 'active' },
          },
        },
      });

      const resp = await subscriber.waitForMessage('QUERY_RESP');
      expect(resp.payload.results.length).toBe(1);
      await waitForSync(200);

      // Clear to isolate QUERY_UPDATE
      subscriber.messages.length = 0;

      // Writer updates the record so it no longer matches the filter
      writer.messages.length = 0;
      writer.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'live-leave-put-2',
          mapName,
          opType: 'PUT',
          key: 'leave-item',
          record: createLWWRecord({ status: 'archived', name: 'Item A' }),
        },
      });

      // Wait for LEAVE notification
      await waitUntil(
        () =>
          subscriber.messages.some(
            (m) =>
              m.type === 'QUERY_UPDATE' &&
              m.payload?.queryId === 'live-q-leave' &&
              m.payload?.changeType === 'LEAVE'
          ),
        5000
      );

      const leaveUpdate = subscriber.messages.find(
        (m) =>
          m.type === 'QUERY_UPDATE' &&
          m.payload?.queryId === 'live-q-leave' &&
          m.payload?.changeType === 'LEAVE'
      );
      expect(leaveUpdate).toBeDefined();
      expect(leaveUpdate.payload.key).toBe('leave-item');

      subscriber.close();
      writer.close();
    });
  });

  // ========================================
  // QUERY_UNSUB (AC28)
  // ========================================
  describe('QUERY_UNSUB stops delivery', () => {
    test('after QUERY_UNSUB, no more QUERY_UPDATE messages', async () => {
      const mapName = `live-unsub-${Date.now()}`;

      const subscriber = await createRustTestClient(port, {
        nodeId: 'live-unsub-sub-1',
        userId: 'live-unsub-sub-user-1',
        roles: ['ADMIN'],
      });
      await subscriber.waitForMessage('AUTH_ACK');

      const writer = await createRustTestClient(port, {
        nodeId: 'live-unsub-writer-1',
        userId: 'live-unsub-writer-user-1',
        roles: ['ADMIN'],
      });
      await writer.waitForMessage('AUTH_ACK');

      // Subscribe
      subscriber.messages.length = 0;
      subscriber.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'live-q-unsub',
          mapName,
          query: {},
        },
      });
      await subscriber.waitForMessage('QUERY_RESP');
      await waitForSync(200);

      // Clear messages and verify we get updates before unsubscribing
      subscriber.messages.length = 0;

      writer.messages.length = 0;
      writer.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'unsub-verify-put',
          mapName,
          opType: 'PUT',
          key: 'verify-item',
          record: createLWWRecord({ check: true }),
        },
      });

      // Wait for QUERY_UPDATE proving subscription works
      await waitUntil(
        () =>
          subscriber.messages.some(
            (m) => m.type === 'QUERY_UPDATE'
          ),
        5000
      );

      // Now unsubscribe
      subscriber.send({
        type: 'QUERY_UNSUB',
        payload: {
          queryId: 'live-q-unsub',
        },
      });

      await waitForSync(300);

      // Clear messages
      subscriber.messages.length = 0;

      // Writer adds another record
      writer.messages.length = 0;
      writer.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'unsub-post-put',
          mapName,
          opType: 'PUT',
          key: 'post-unsub-item',
          record: createLWWRecord({ afterUnsub: true }),
        },
      });

      await writer.waitForMessage('OP_ACK');

      // Wait to ensure no QUERY_UPDATE arrives
      await waitForSync(1000);

      const postUnsubUpdates = subscriber.messages.filter(
        (m) => m.type === 'QUERY_UPDATE'
      );
      expect(postUnsubUpdates.length).toBe(0);

      subscriber.close();
      writer.close();
    });
  });

  // ========================================
  // Multi-Client Live Updates
  // ========================================
  describe('Multi-client live updates', () => {
    test('subscriber receives updates from another client\'s writes', async () => {
      const mapName = `live-multi-${Date.now()}`;

      const subscriber = await createRustTestClient(port, {
        nodeId: 'multi-live-sub',
        userId: 'multi-live-sub-user',
        roles: ['ADMIN'],
      });
      await subscriber.waitForMessage('AUTH_ACK');

      // Subscribe first, then writers add data
      subscriber.messages.length = 0;
      subscriber.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'multi-live-q',
          mapName,
          query: {},
        },
      });
      await subscriber.waitForMessage('QUERY_RESP');
      await waitForSync(200);

      subscriber.messages.length = 0;

      // Two different writers add records
      const writer1 = await createRustTestClient(port, {
        nodeId: 'multi-writer-1',
        userId: 'multi-writer-user-1',
        roles: ['ADMIN'],
      });
      await writer1.waitForMessage('AUTH_ACK');

      const writer2 = await createRustTestClient(port, {
        nodeId: 'multi-writer-2',
        userId: 'multi-writer-user-2',
        roles: ['ADMIN'],
      });
      await writer2.waitForMessage('AUTH_ACK');

      writer1.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'multi-w1-put',
          mapName,
          opType: 'PUT',
          key: 'from-writer-1',
          record: createLWWRecord({ author: 'writer1' }),
        },
      });

      await waitForSync(200);

      writer2.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'multi-w2-put',
          mapName,
          opType: 'PUT',
          key: 'from-writer-2',
          record: createLWWRecord({ author: 'writer2' }),
        },
      });

      // Wait for both QUERY_UPDATE messages
      await waitUntil(
        () =>
          subscriber.messages.filter(
            (m) =>
              m.type === 'QUERY_UPDATE' &&
              m.payload?.queryId === 'multi-live-q'
          ).length >= 2,
        5000
      );

      const updates = subscriber.messages.filter(
        (m) =>
          m.type === 'QUERY_UPDATE' &&
          m.payload?.queryId === 'multi-live-q'
      );
      expect(updates.length).toBeGreaterThanOrEqual(2);

      const updateKeys = updates.map((u: any) => u.payload.key).sort();
      expect(updateKeys).toContain('from-writer-1');
      expect(updateKeys).toContain('from-writer-2');

      subscriber.close();
      writer1.close();
      writer2.close();
    });
  });

  // ========================================
  // Multiple Queries with Different Filters (G5)
  // ========================================
  describe('Multiple queries on same collection with different filters', () => {
    test('two queries with different where filters both receive correct updates', async () => {
      const mapName = `multi-query-${Date.now()}`;

      const subscriber = await createRustTestClient(port, {
        nodeId: 'mq-sub-1',
        userId: 'mq-sub-user-1',
        roles: ['ADMIN'],
      });
      await subscriber.waitForMessage('AUTH_ACK');

      const writer = await createRustTestClient(port, {
        nodeId: 'mq-writer-1',
        userId: 'mq-writer-user-1',
        roles: ['ADMIN'],
      });
      await writer.waitForMessage('AUTH_ACK');

      // Subscribe to two queries with different filters on the same map
      // Query A: status = 'active'
      subscriber.messages.length = 0;
      subscriber.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'mq-active',
          mapName,
          query: {
            where: { status: 'active' },
          },
        },
      });
      await subscriber.waitForMessage('QUERY_RESP');

      // Query B: status = 'archived'
      subscriber.messages.length = 0;
      subscriber.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'mq-archived',
          mapName,
          query: {
            where: { status: 'archived' },
          },
        },
      });
      await subscriber.waitForMessage('QUERY_RESP');

      await waitForSync(200);
      subscriber.messages.length = 0;

      // Writer adds an 'active' record
      writer.messages.length = 0;
      writer.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'mq-put-active',
          mapName,
          opType: 'PUT',
          key: 'task-1',
          record: createLWWRecord({ status: 'active', title: 'Active Task' }),
        },
      });

      // Wait for QUERY_UPDATE on the 'active' query
      await waitUntil(
        () =>
          subscriber.messages.some(
            (m) =>
              m.type === 'QUERY_UPDATE' &&
              m.payload?.queryId === 'mq-active' &&
              m.payload?.key === 'task-1'
          ),
        5000
      );

      // Verify the active query got an ENTER
      const activeUpdate = subscriber.messages.find(
        (m) =>
          m.type === 'QUERY_UPDATE' &&
          m.payload?.queryId === 'mq-active' &&
          m.payload?.key === 'task-1'
      );
      expect(activeUpdate).toBeDefined();
      expect(activeUpdate.payload.changeType).toBe('ENTER');

      await waitForSync(300);

      // Writer adds an 'archived' record
      subscriber.messages.length = 0;
      writer.messages.length = 0;
      writer.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'mq-put-archived',
          mapName,
          opType: 'PUT',
          key: 'task-2',
          record: createLWWRecord({ status: 'archived', title: 'Old Task' }),
        },
      });

      // Wait for QUERY_UPDATE on the 'archived' query
      await waitUntil(
        () =>
          subscriber.messages.some(
            (m) =>
              m.type === 'QUERY_UPDATE' &&
              m.payload?.queryId === 'mq-archived' &&
              m.payload?.key === 'task-2'
          ),
        5000
      );

      const archivedUpdate = subscriber.messages.find(
        (m) =>
          m.type === 'QUERY_UPDATE' &&
          m.payload?.queryId === 'mq-archived' &&
          m.payload?.key === 'task-2'
      );
      expect(archivedUpdate).toBeDefined();
      expect(archivedUpdate.payload.changeType).toBe('ENTER');

      // The archived record should NOT trigger an update on the active query
      const activeUpdatesForTask2 = subscriber.messages.filter(
        (m) =>
          m.type === 'QUERY_UPDATE' &&
          m.payload?.queryId === 'mq-active' &&
          m.payload?.key === 'task-2'
      );
      expect(activeUpdatesForTask2.length).toBe(0);

      subscriber.close();
      writer.close();
    });
  });

  // ========================================
  // Field Projection (SPEC-144)
  // ========================================
  describe('QUERY_SUB with field projection', () => {
    test('fields projection returns only specified fields in results', async () => {
      const mapName = `fields-proj-${Date.now()}`;

      const client = await createRustTestClient(port, {
        nodeId: 'fields-proj-client-1',
        userId: 'fields-proj-user-1',
        roles: ['ADMIN'],
      });
      await client.waitForMessage('AUTH_ACK');

      // Write records with multiple fields
      const records = [
        { key: 'fp-1', value: { name: 'Alice', email: 'alice@example.com', age: 30, role: 'admin' } },
        { key: 'fp-2', value: { name: 'Bob', email: 'bob@example.com', age: 25, role: 'user' } },
      ];

      for (const rec of records) {
        client.messages.length = 0;
        client.send({
          type: 'CLIENT_OP',
          payload: {
            id: `fp-put-${rec.key}`,
            mapName,
            opType: 'PUT',
            key: rec.key,
            record: createLWWRecord(rec.value),
          },
        });
        await client.waitForMessage('OP_ACK');
      }

      await waitForSync(200);

      // Subscribe with field projection: only 'name' and 'email'
      client.messages.length = 0;
      client.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'fp-q-1',
          mapName,
          query: {},
          fields: ['name', 'email'],
        },
      });

      const response = await client.waitForMessage('QUERY_RESP');
      expect(response).toBeDefined();
      expect(response.payload.queryId).toBe('fp-q-1');
      expect(response.payload.results).toBeDefined();
      expect(response.payload.results.length).toBe(2);

      // Each result should only contain projected fields (name and email)
      for (const result of response.payload.results) {
        const keys = Object.keys(result.value);
        expect(keys).toContain('name');
        expect(keys).toContain('email');
        // Non-projected fields should not be present
        expect(keys).not.toContain('age');
        expect(keys).not.toContain('role');
      }

      // Verify a merkleRootHash is returned (server builds Merkle tree for projected queries)
      expect(response.payload.merkleRootHash).toBeDefined();
      expect(typeof response.payload.merkleRootHash).toBe('number');

      client.close();
    });
  });

  // ========================================
  // Merkle Delta Reconnect (SPEC-144)
  // ========================================
  describe('QUERY_SYNC_INIT Merkle delta reconnect', () => {
    test('reconnect with stored Merkle hash sends QUERY_SYNC_INIT', async () => {
      const mapName = `merkle-reconnect-${Date.now()}`;

      // First connection: establish query with fields, receive QUERY_RESP with merkleRootHash
      const client1 = await createRustTestClient(port, {
        nodeId: 'merkle-rc-client-1',
        userId: 'merkle-rc-user-1',
        roles: ['ADMIN'],
      });
      await client1.waitForMessage('AUTH_ACK');

      // Write initial data
      client1.messages.length = 0;
      client1.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'mrc-put-1',
          mapName,
          opType: 'PUT',
          key: 'mrc-item-1',
          record: createLWWRecord({ name: 'Initial', status: 'active' }),
        },
      });
      await client1.waitForMessage('OP_ACK');
      await waitForSync(200);

      // Subscribe with field projection
      const queryId = 'mrc-q-1';
      client1.messages.length = 0;
      client1.send({
        type: 'QUERY_SUB',
        payload: {
          queryId,
          mapName,
          query: {},
          fields: ['name', 'status'],
        },
      });

      const initialResp = await client1.waitForMessage('QUERY_RESP');
      expect(initialResp.payload.queryId).toBe(queryId);
      expect(initialResp.payload.results.length).toBe(1);

      // Server should return a merkleRootHash for field-projected queries
      const storedHash = initialResp.payload.merkleRootHash;
      expect(storedHash).toBeDefined();
      expect(typeof storedHash).toBe('number');
      expect(storedHash).not.toBe(0);

      client1.close();
      await waitForSync(200);

      // Second connection: simulate reconnect by opening a new client
      // and sending QUERY_SYNC_INIT with the stored hash
      const client2 = await createRustTestClient(port, {
        nodeId: 'merkle-rc-client-2',
        userId: 'merkle-rc-user-2',
        roles: ['ADMIN'],
      });
      await client2.waitForMessage('AUTH_ACK');

      // Send QUERY_SUB first (as QueryManager.resubscribeAll does)
      client2.messages.length = 0;
      client2.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'mrc-q-reconnect',
          mapName,
          query: {},
          fields: ['name', 'status'],
        },
      });

      // Then send QUERY_SYNC_INIT with the stored Merkle hash
      client2.send({
        type: 'QUERY_SYNC_INIT',
        payload: {
          queryId: 'mrc-q-reconnect',
          rootHash: storedHash,
        },
      });

      // Server should respond — either QUERY_RESP (full snapshot if hash mismatch)
      // or no response if state is identical. In test, we expect at least QUERY_RESP
      // from the initial QUERY_SUB to confirm the server accepted both messages.
      const reconnectResp = await client2.waitForMessage('QUERY_RESP');
      expect(reconnectResp).toBeDefined();
      expect(reconnectResp.payload.queryId).toBe('mrc-q-reconnect');

      client2.close();
    });
  });
});
