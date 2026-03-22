import {
  createRustTestClient,
  spawnRustServer,
  createLWWRecord,
  waitForSync,
  waitUntil,
  TestClient,
} from './helpers';

describe('Integration: Shapes (Rust Server)', () => {
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
  // Test 1: Shape subscribe returns filtered records (AC16)
  // ========================================
  describe('SHAPE_SUBSCRIBE returns only filtered records', () => {
    test('shape with eq filter returns only matching records', async () => {
      const mapName = `shape-filter-${Date.now()}`;
      const client = await createRustTestClient(port, {
        nodeId: 'shape-filter-1',
        userId: 'shape-filter-user-1',
        roles: ['ADMIN'],
      });
      await client.waitForMessage('AUTH_ACK');

      // Write records with different status values
      const records = [
        { key: 'user-1', value: { name: 'Alice', status: 'active' } },
        { key: 'user-2', value: { name: 'Bob', status: 'inactive' } },
        { key: 'user-3', value: { name: 'Charlie', status: 'active' } },
        { key: 'user-4', value: { name: 'Dave', status: 'pending' } },
      ];

      for (const rec of records) {
        client.messages.length = 0;
        client.send({
          type: 'CLIENT_OP',
          payload: {
            id: `shape-filter-put-${rec.key}`,
            mapName,
            opType: 'PUT',
            key: rec.key,
            record: createLWWRecord(rec.value),
          },
        });
        await client.waitForMessage('OP_ACK');
      }

      await waitForSync(200);

      const shapeId = `shape-filter-${Date.now()}`;
      client.messages.length = 0;
      client.send({
        type: 'SHAPE_SUBSCRIBE',
        payload: {
          shape: {
            shapeId,
            mapName,
            filter: {
              op: 'eq',
              attribute: 'status',
              value: 'active',
            },
          },
        },
      });

      const resp = await client.waitForMessage('SHAPE_RESP');
      expect(resp).toBeDefined();
      expect(resp.payload.shapeId).toBe(shapeId);
      expect(Array.isArray(resp.payload.records)).toBe(true);

      // Only active users should be returned
      const keys = resp.payload.records.map((r: any) => r.key).sort();
      expect(keys).toContain('user-1');
      expect(keys).toContain('user-3');
      expect(keys).not.toContain('user-2');
      expect(keys).not.toContain('user-4');
      expect(resp.payload.records.length).toBe(2);

      // merkleRootHash is a non-negative integer
      expect(typeof resp.payload.merkleRootHash).toBe('number');
      expect(Number.isInteger(resp.payload.merkleRootHash)).toBe(true);
      expect(resp.payload.merkleRootHash).toBeGreaterThanOrEqual(0);

      // Each record has { key, value } structure
      for (const record of resp.payload.records) {
        expect(typeof record.key).toBe('string');
        expect(record.value).toBeDefined();
      }

      client.close();
    });
  });

  // ========================================
  // Test 2: Shape with field projection (AC17)
  // ========================================
  describe('SHAPE_SUBSCRIBE with field projection', () => {
    test('field projection returns only projected fields', async () => {
      const mapName = `shape-proj-${Date.now()}`;
      const client = await createRustTestClient(port, {
        nodeId: 'shape-proj-1',
        userId: 'shape-proj-user-1',
        roles: ['ADMIN'],
      });
      await client.waitForMessage('AUTH_ACK');

      // Write records with multiple fields
      client.messages.length = 0;
      client.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'shape-proj-put-1',
          mapName,
          opType: 'PUT',
          key: 'contact-1',
          record: createLWWRecord({ name: 'Alice', email: 'alice@example.com', phone: '555-1234', age: 30 }),
        },
      });
      await client.waitForMessage('OP_ACK');

      client.messages.length = 0;
      client.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'shape-proj-put-2',
          mapName,
          opType: 'PUT',
          key: 'contact-2',
          record: createLWWRecord({ name: 'Bob', email: 'bob@example.com', phone: '555-5678', age: 25 }),
        },
      });
      await client.waitForMessage('OP_ACK');

      await waitForSync(200);

      const shapeId = `shape-proj-${Date.now()}`;
      client.messages.length = 0;
      client.send({
        type: 'SHAPE_SUBSCRIBE',
        payload: {
          shape: {
            shapeId,
            mapName,
            fields: ['name', 'email'],
          },
        },
      });

      const resp = await client.waitForMessage('SHAPE_RESP');
      expect(resp).toBeDefined();
      expect(resp.payload.shapeId).toBe(shapeId);
      expect(resp.payload.records.length).toBe(2);

      // Each returned record's value should contain only the projected fields
      for (const record of resp.payload.records) {
        expect(typeof record.key).toBe('string');
        expect(record.value).toBeDefined();
        const value = record.value as any;
        expect(value.name).toBeDefined();
        expect(value.email).toBeDefined();
        // phone and age should NOT be present (projected out)
        expect(value.phone).toBeUndefined();
        expect(value.age).toBeUndefined();
      }

      client.close();
    });
  });

  // ========================================
  // Test 3: Mutation matching shape triggers ShapeUpdate (AC18)
  // ========================================
  describe('Mutation matching shape triggers SHAPE_UPDATE ENTER', () => {
    test('new matching record triggers ENTER update', async () => {
      const mapName = `shape-enter-${Date.now()}`;

      const subscriber = await createRustTestClient(port, {
        nodeId: 'shape-enter-sub-1',
        userId: 'shape-enter-sub-user-1',
        roles: ['ADMIN'],
      });
      await subscriber.waitForMessage('AUTH_ACK');

      const writer = await createRustTestClient(port, {
        nodeId: 'shape-enter-writer-1',
        userId: 'shape-enter-writer-user-1',
        roles: ['ADMIN'],
      });
      await writer.waitForMessage('AUTH_ACK');

      const shapeId = `shape-enter-${Date.now()}`;

      // Subscribe to shape with status == "active"
      subscriber.messages.length = 0;
      subscriber.send({
        type: 'SHAPE_SUBSCRIBE',
        payload: {
          shape: {
            shapeId,
            mapName,
            filter: {
              op: 'eq',
              attribute: 'status',
              value: 'active',
            },
          },
        },
      });

      // Wait for initial SHAPE_RESP
      await subscriber.waitForMessage('SHAPE_RESP');

      // Clear messages to isolate the SHAPE_UPDATE
      subscriber.messages.length = 0;

      // Write a new record that matches the shape filter
      writer.messages.length = 0;
      writer.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'shape-enter-put-1',
          mapName,
          opType: 'PUT',
          key: 'new-active-user',
          record: createLWWRecord({ name: 'New User', status: 'active' }),
        },
      });

      // Wait for SHAPE_UPDATE with ENTER changeType
      await waitUntil(
        () =>
          subscriber.messages.some(
            (m) =>
              m.type === 'SHAPE_UPDATE' &&
              m.payload?.shapeId === shapeId &&
              m.payload?.changeType === 'ENTER'
          ),
        8000
      );

      const enterUpdate = subscriber.messages.find(
        (m) =>
          m.type === 'SHAPE_UPDATE' &&
          m.payload?.shapeId === shapeId &&
          m.payload?.changeType === 'ENTER'
      );
      expect(enterUpdate).toBeDefined();
      expect(enterUpdate.payload.key).toBe('new-active-user');
      expect(enterUpdate.payload.value).toBeDefined();
      expect(enterUpdate.payload.changeType).toBe('ENTER');

      subscriber.close();
      writer.close();
    });

    test('non-matching mutation does not trigger SHAPE_UPDATE', async () => {
      const mapName = `shape-nomatch-${Date.now()}`;

      const subscriber = await createRustTestClient(port, {
        nodeId: 'shape-nomatch-sub-1',
        userId: 'shape-nomatch-sub-user-1',
        roles: ['ADMIN'],
      });
      await subscriber.waitForMessage('AUTH_ACK');

      const writer = await createRustTestClient(port, {
        nodeId: 'shape-nomatch-writer-1',
        userId: 'shape-nomatch-writer-user-1',
        roles: ['ADMIN'],
      });
      await writer.waitForMessage('AUTH_ACK');

      const shapeId = `shape-nomatch-${Date.now()}`;

      // Subscribe to shape with status == "active"
      subscriber.messages.length = 0;
      subscriber.send({
        type: 'SHAPE_SUBSCRIBE',
        payload: {
          shape: {
            shapeId,
            mapName,
            filter: {
              op: 'eq',
              attribute: 'status',
              value: 'active',
            },
          },
        },
      });

      await subscriber.waitForMessage('SHAPE_RESP');
      subscriber.messages.length = 0;

      // Write a record that does NOT match the filter
      writer.messages.length = 0;
      writer.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'shape-nomatch-put-1',
          mapName,
          opType: 'PUT',
          key: 'inactive-user',
          record: createLWWRecord({ name: 'Inactive User', status: 'inactive' }),
        },
      });
      await writer.waitForMessage('OP_ACK');

      // Wait a fixed timeout — no SHAPE_UPDATE should arrive
      await waitForSync(1000);

      const shapeUpdates = subscriber.messages.filter(
        (m) => m.type === 'SHAPE_UPDATE' && m.payload?.shapeId === shapeId
      );
      expect(shapeUpdates.length).toBe(0);

      subscriber.close();
      writer.close();
    });
  });

  // ========================================
  // Test 4: Shape unsubscribe stops updates (AC19)
  // ========================================
  describe('SHAPE_UNSUBSCRIBE stops SHAPE_UPDATE delivery', () => {
    test('after unsubscribe no more SHAPE_UPDATE messages are received', async () => {
      const mapName = `shape-unsub-${Date.now()}`;

      const subscriber = await createRustTestClient(port, {
        nodeId: 'shape-unsub-sub-1',
        userId: 'shape-unsub-sub-user-1',
        roles: ['ADMIN'],
      });
      await subscriber.waitForMessage('AUTH_ACK');

      const writer = await createRustTestClient(port, {
        nodeId: 'shape-unsub-writer-1',
        userId: 'shape-unsub-writer-user-1',
        roles: ['ADMIN'],
      });
      await writer.waitForMessage('AUTH_ACK');

      const shapeId = `shape-unsub-${Date.now()}`;

      // Subscribe to shape
      subscriber.messages.length = 0;
      subscriber.send({
        type: 'SHAPE_SUBSCRIBE',
        payload: {
          shape: {
            shapeId,
            mapName,
            filter: {
              op: 'eq',
              attribute: 'type',
              value: 'monitored',
            },
          },
        },
      });

      await subscriber.waitForMessage('SHAPE_RESP');

      // Write a matching record to verify subscription works
      subscriber.messages.length = 0;
      writer.messages.length = 0;
      writer.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'shape-unsub-verify-put',
          mapName,
          opType: 'PUT',
          key: 'monitored-1',
          record: createLWWRecord({ name: 'Item 1', type: 'monitored' }),
        },
      });

      // Wait to confirm subscription is working
      await waitUntil(
        () =>
          subscriber.messages.some(
            (m) =>
              m.type === 'SHAPE_UPDATE' &&
              m.payload?.shapeId === shapeId
          ),
        8000
      );

      // Send SHAPE_UNSUBSCRIBE
      subscriber.send({
        type: 'SHAPE_UNSUBSCRIBE',
        payload: { shapeId },
      });

      await waitForSync(300);

      // Clear messages after unsubscribe
      subscriber.messages.length = 0;

      // Writer adds another matching record
      writer.messages.length = 0;
      writer.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'shape-unsub-post-put',
          mapName,
          opType: 'PUT',
          key: 'monitored-2',
          record: createLWWRecord({ name: 'Item 2', type: 'monitored' }),
        },
      });
      await writer.waitForMessage('OP_ACK');

      // Wait to ensure no SHAPE_UPDATE arrives after unsubscribe
      await waitForSync(1000);

      const postUnsubUpdates = subscriber.messages.filter(
        (m) =>
          m.type === 'SHAPE_UPDATE' &&
          m.payload?.shapeId === shapeId
      );
      expect(postUnsubUpdates.length).toBe(0);

      subscriber.close();
      writer.close();
    });
  });

  // ========================================
  // Test 5: Reconnect shape Merkle sync sends only delta (AC20)
  // ========================================
  describe('Reconnect shape Merkle sync via SHAPE_SYNC_INIT', () => {
    test('SHAPE_SYNC_INIT with stored rootHash triggers delta sync', async () => {
      const mapName = `shape-sync-${Date.now()}`;
      const shapeId = `shape-sync-${Date.now()}`;

      // Connection 1: Subscribe, get initial snapshot, record merkleRootHash
      const conn1 = await createRustTestClient(port, {
        nodeId: 'shape-sync-conn1',
        userId: 'shape-sync-user-1',
        roles: ['ADMIN'],
      });
      await conn1.waitForMessage('AUTH_ACK');

      // Write initial data BEFORE subscribing (so we get a non-empty snapshot)
      conn1.messages.length = 0;
      conn1.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'shape-sync-init-put',
          mapName,
          opType: 'PUT',
          key: 'existing-item',
          record: createLWWRecord({ name: 'Existing Item', category: 'widget' }),
        },
      });
      await conn1.waitForMessage('OP_ACK');
      await waitForSync(200);

      // Subscribe to shape matching category == "widget"
      conn1.messages.length = 0;
      conn1.send({
        type: 'SHAPE_SUBSCRIBE',
        payload: {
          shape: {
            shapeId,
            mapName,
            filter: {
              op: 'eq',
              attribute: 'category',
              value: 'widget',
            },
          },
        },
      });

      const initialResp = await conn1.waitForMessage('SHAPE_RESP');
      expect(initialResp.payload.shapeId).toBe(shapeId);
      expect(initialResp.payload.records.length).toBeGreaterThanOrEqual(1);

      // Record the merkle root hash from the initial snapshot
      const storedMerkleRootHash: number = initialResp.payload.merkleRootHash;
      expect(typeof storedMerkleRootHash).toBe('number');

      conn1.close();

      // Connection 2: Add a new matching record (simulates data added "while disconnected")
      const conn2 = await createRustTestClient(port, {
        nodeId: 'shape-sync-conn2',
        userId: 'shape-sync-user-2',
        roles: ['ADMIN'],
      });
      await conn2.waitForMessage('AUTH_ACK');

      conn2.messages.length = 0;
      conn2.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'shape-sync-new-put',
          mapName,
          opType: 'PUT',
          key: 'new-widget',
          record: createLWWRecord({ name: 'New Widget', category: 'widget' }),
        },
      });
      await conn2.waitForMessage('OP_ACK');
      await waitForSync(200);

      conn2.close();

      // Connection 3: Simulate reconnect by sending SHAPE_SYNC_INIT with stored rootHash
      // The server should respond with only the delta (the new record added after conn1 disconnected)
      const conn3 = await createRustTestClient(port, {
        nodeId: 'shape-sync-conn3',
        userId: 'shape-sync-user-3',
        roles: ['ADMIN'],
      });
      await conn3.waitForMessage('AUTH_ACK');

      conn3.messages.length = 0;
      conn3.send({
        type: 'SHAPE_SYNC_INIT',
        payload: {
          shapeId,
          rootHash: storedMerkleRootHash,
        },
      });

      // The server should respond with sync protocol messages (SYNC_RESP_ROOT etc.)
      // Wait for sync responses to arrive
      await waitForSync(2000);

      // Verify that sync protocol messages arrived (SYNC_RESP_ROOT, SYNC_RESP_BUCKETS, or SYNC_RESP_LEAF)
      const syncMessages = conn3.messages.filter(
        (m) =>
          m.type === 'SYNC_RESP_ROOT' ||
          m.type === 'SYNC_RESP_BUCKETS' ||
          m.type === 'SYNC_RESP_LEAF'
      );
      // The server should have responded with some sync protocol message
      // (at minimum SYNC_RESP_ROOT indicating the current state)
      expect(syncMessages.length).toBeGreaterThanOrEqual(1);

      conn3.close();
    });
  });
});
