import {
  createRustTestContext,
  createRustTestClient,
  spawnRustServer,
  createORRecord,
  waitForSync,
  waitUntil,
  TestClient,
} from './helpers';

/**
 * Subscribes a client to a map via QUERY_SUB so it receives SERVER_EVENT
 * broadcasts for writes to that map.
 */
function subscribeToMap(client: TestClient, mapName: string, queryId: string) {
  client.send({
    type: 'QUERY_SUB',
    payload: {
      queryId,
      mapName,
      query: {},
    },
  });
}

describe('Integration: ORMap CRDT (Rust Server)', () => {
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
  // OR_ADD Tests
  // ========================================
  describe('OR_ADD via CLIENT_OP', () => {
    test('OR_ADD creates entry, verified via SERVER_EVENT broadcast to second client', async () => {
      // Client 1 is the writer, Client 2 is the observer
      const writer = await createRustTestClient(port, {
        nodeId: 'or-writer-1',
        userId: 'writer-1',
        roles: ['ADMIN'],
      });
      await writer.waitForMessage('AUTH_ACK');

      const observer = await createRustTestClient(port, {
        nodeId: 'or-observer-1',
        userId: 'observer-1',
        roles: ['ADMIN'],
      });
      await observer.waitForMessage('AUTH_ACK');

      // Subscribe observer to the map so it receives SERVER_EVENT broadcasts
      subscribeToMap(observer, 'or-items', 'q-or-items-obs');
      await waitForSync(100);

      // Clear observer messages before the operation
      observer.messages.length = 0;

      const orRecord = createORRecord('Value 1', 'or-writer-1');

      // Send OR_ADD as a CLIENT_OP with opType and orRecord fields
      writer.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'or-add-1',
          mapName: 'or-items',
          opType: 'OR_ADD',
          key: 'list-1',
          orRecord,
        },
      });

      // Wait for the observer to receive a SERVER_EVENT confirming the OR_ADD
      await waitUntil(
        () =>
          observer.messages.some(
            (m) =>
              m.type === 'SERVER_EVENT' &&
              m.payload?.eventType === 'OR_ADD'
          ),
        5000
      );

      const serverEvent = observer.messages.find(
        (m) =>
          m.type === 'SERVER_EVENT' &&
          m.payload?.eventType === 'OR_ADD'
      );
      expect(serverEvent).toBeDefined();
      expect(serverEvent.payload.mapName).toBe('or-items');
      expect(serverEvent.payload.key).toBe('list-1');

      writer.close();
      observer.close();
    });
  });

  // ========================================
  // OR_REMOVE Tests
  // ========================================
  describe('OR_REMOVE via CLIENT_OP', () => {
    test('OR_REMOVE removes entry by tag, verified via SERVER_EVENT broadcast to second client', async () => {
      const writer = await createRustTestClient(port, {
        nodeId: 'or-remover-1',
        userId: 'remover-1',
        roles: ['ADMIN'],
      });
      await writer.waitForMessage('AUTH_ACK');

      const observer = await createRustTestClient(port, {
        nodeId: 'or-remove-observer-1',
        userId: 'remove-observer-1',
        roles: ['ADMIN'],
      });
      await observer.waitForMessage('AUTH_ACK');

      // Subscribe observer to the map so it receives SERVER_EVENT broadcasts
      subscribeToMap(observer, 'removable', 'q-removable-obs');
      await waitForSync(100);

      // First, add an item so we have something to remove
      const orRecord = createORRecord('To Remove', 'or-remover-1');
      const tag = orRecord.tag;

      writer.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'or-add-for-remove',
          mapName: 'removable',
          opType: 'OR_ADD',
          key: 'items',
          orRecord,
        },
      });

      // Wait for the OR_ADD to be processed (observer should see the event)
      await waitUntil(
        () =>
          observer.messages.some(
            (m) =>
              m.type === 'SERVER_EVENT' &&
              m.payload?.eventType === 'OR_ADD'
          ),
        5000
      );

      // Clear observer messages to isolate the OR_REMOVE event
      observer.messages.length = 0;

      // Now remove the entry by tag via OR_REMOVE
      writer.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'or-remove-1',
          mapName: 'removable',
          opType: 'OR_REMOVE',
          key: 'items',
          orTag: tag,
        },
      });

      // Wait for the observer to receive a SERVER_EVENT with OR_REMOVE
      await waitUntil(
        () =>
          observer.messages.some(
            (m) =>
              m.type === 'SERVER_EVENT' &&
              m.payload?.eventType === 'OR_REMOVE'
          ),
        5000
      );

      const removeEvent = observer.messages.find(
        (m) =>
          m.type === 'SERVER_EVENT' &&
          m.payload?.eventType === 'OR_REMOVE'
      );
      expect(removeEvent).toBeDefined();
      expect(removeEvent.payload.mapName).toBe('removable');
      expect(removeEvent.payload.key).toBe('items');

      writer.close();
      observer.close();
    });
  });

  // ========================================
  // Multi-Value Tests
  // ========================================
  describe('Multiple values per key', () => {
    test('concurrent OR_ADD from different clients adds multiple values', async () => {
      const client1 = await createRustTestClient(port, {
        nodeId: 'multi-or-1',
        userId: 'multi-user-1',
        roles: ['ADMIN'],
      });
      await client1.waitForMessage('AUTH_ACK');

      const client2 = await createRustTestClient(port, {
        nodeId: 'multi-or-2',
        userId: 'multi-user-2',
        roles: ['ADMIN'],
      });
      await client2.waitForMessage('AUTH_ACK');

      // Observer to verify both adds
      const observer = await createRustTestClient(port, {
        nodeId: 'multi-or-observer',
        userId: 'multi-observer',
        roles: ['ADMIN'],
      });
      await observer.waitForMessage('AUTH_ACK');

      // Subscribe observer to the map so it receives SERVER_EVENT broadcasts
      subscribeToMap(observer, 'multi-values', 'q-multi-values-obs');
      await waitForSync(100);

      observer.messages.length = 0;

      // Client 1 adds value
      client1.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'multi-or-add-1',
          mapName: 'multi-values',
          opType: 'OR_ADD',
          key: 'tags',
          orRecord: createORRecord('tag1', 'multi-or-1'),
        },
      });

      await waitForSync(200);

      // Client 2 adds different value to the same key
      client2.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'multi-or-add-2',
          mapName: 'multi-values',
          opType: 'OR_ADD',
          key: 'tags',
          orRecord: createORRecord('tag2', 'multi-or-2'),
        },
      });

      // Wait for both OR_ADD events to reach the observer
      await waitUntil(
        () =>
          observer.messages.filter(
            (m) =>
              m.type === 'SERVER_EVENT' &&
              m.payload?.eventType === 'OR_ADD'
          ).length >= 2,
        5000
      );

      const addEvents = observer.messages.filter(
        (m) =>
          m.type === 'SERVER_EVENT' &&
          m.payload?.eventType === 'OR_ADD' &&
          m.payload?.mapName === 'multi-values'
      );
      expect(addEvents.length).toBeGreaterThanOrEqual(2);

      client1.close();
      client2.close();
      observer.close();
    });
  });

  // ========================================
  // Tombstone Synchronization Tests
  // ========================================
  describe('Tombstone synchronization', () => {
    test('tombstone from one client propagates to another via SERVER_EVENT', async () => {
      const client1 = await createRustTestClient(port, {
        nodeId: 'tomb-or-1',
        userId: 'tomb-user-1',
        roles: ['ADMIN'],
      });
      await client1.waitForMessage('AUTH_ACK');

      const client2 = await createRustTestClient(port, {
        nodeId: 'tomb-or-2',
        userId: 'tomb-user-2',
        roles: ['ADMIN'],
      });
      await client2.waitForMessage('AUTH_ACK');

      // Both clients subscribe to the map so they receive each other's events
      subscribeToMap(client1, 'shared-or', 'q-shared-or-c1');
      subscribeToMap(client2, 'shared-or', 'q-shared-or-c2');
      await waitForSync(100);

      // Client 1 adds an item
      const orRecord = createORRecord('Shared Item', 'tomb-or-1');
      const tag = orRecord.tag;

      client1.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'tomb-or-add',
          mapName: 'shared-or',
          opType: 'OR_ADD',
          key: 'shared-list',
          orRecord,
        },
      });

      // Wait for client2 to see the OR_ADD event
      await waitUntil(
        () =>
          client2.messages.some(
            (m) =>
              m.type === 'SERVER_EVENT' &&
              m.payload?.eventType === 'OR_ADD'
          ),
        5000
      );

      // Clear client2 messages to isolate the removal event
      client2.messages.length = 0;

      // Client 2 removes the item by tag
      client2.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'tomb-or-remove',
          mapName: 'shared-or',
          opType: 'OR_REMOVE',
          key: 'shared-list',
          orTag: tag,
        },
      });

      // Wait for client1 to see the OR_REMOVE event (tombstone propagation)
      await waitUntil(
        () =>
          client1.messages.some(
            (m) =>
              m.type === 'SERVER_EVENT' &&
              m.payload?.eventType === 'OR_REMOVE'
          ),
        5000
      );

      const removeEvent = client1.messages.find(
        (m) =>
          m.type === 'SERVER_EVENT' &&
          m.payload?.eventType === 'OR_REMOVE'
      );
      expect(removeEvent).toBeDefined();
      expect(removeEvent.payload.mapName).toBe('shared-or');
      expect(removeEvent.payload.key).toBe('shared-list');

      client1.close();
      client2.close();
    });
  });
});
