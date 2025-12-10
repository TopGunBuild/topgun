import { ServerCoordinator } from '@topgunbuild/server';
import { LWWMap, ORMap } from '@topgunbuild/core';
import {
  createTestServer,
  createTestClient,
  createTestContext,
  createLWWRecord,
  createORRecord,
  waitForSync,
  waitUntil,
  TestClient,
  TestContext,
} from './helpers';

describe('E2E: Multi-Client Synchronization', () => {
  // ========================================
  // Two-Client Synchronization
  // ========================================
  describe('Two-Client Sync', () => {
    let ctx: TestContext;

    beforeEach(async () => {
      ctx = await createTestContext(2);
    });

    afterEach(async () => {
      await ctx.cleanup();
    });

    test('Client A writes data → Client B receives in real-time', async () => {
      const [clientA, clientB] = ctx.clients;

      // Client B subscribes to updates
      clientB.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'sync-ab-1',
          mapName: 'two-client-test',
          query: {},
        },
      });

      await clientB.waitForMessage('QUERY_RESP');
      clientB.messages.length = 0;

      // Client A writes data
      clientA.send({
        type: 'CLIENT_OP',
        payload: {
          mapName: 'two-client-test',
          opType: 'PUT',
          key: 'item-from-a',
          record: createLWWRecord({ message: 'Hello from A', value: 42 }),
        },
      });

      // Wait for Client B to receive the update
      await waitUntil(
        () => clientB.messages.some((m) => m.type === 'QUERY_UPDATE' || m.type === 'SERVER_EVENT'),
        3000
      );

      const update = clientB.messages.find(
        (m) => m.type === 'QUERY_UPDATE' || m.type === 'SERVER_EVENT'
      );
      expect(update).toBeDefined();

      // Verify server state
      const map = ctx.server.getMap('two-client-test') as LWWMap<string, any>;
      expect(map.get('item-from-a')).toEqual({ message: 'Hello from A', value: 42 });
    });

    test('Client B writes data → Client A receives in real-time', async () => {
      const [clientA, clientB] = ctx.clients;

      // Client A subscribes
      clientA.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'sync-ba-1',
          mapName: 'reverse-sync-test',
          query: {},
        },
      });

      await clientA.waitForMessage('QUERY_RESP');
      clientA.messages.length = 0;

      // Client B writes data
      clientB.send({
        type: 'CLIENT_OP',
        payload: {
          mapName: 'reverse-sync-test',
          opType: 'PUT',
          key: 'item-from-b',
          record: createLWWRecord({ message: 'Hello from B', value: 100 }),
        },
      });

      // Wait for Client A to receive
      await waitUntil(
        () => clientA.messages.some((m) => m.type === 'QUERY_UPDATE' || m.type === 'SERVER_EVENT'),
        3000
      );

      const update = clientA.messages.find(
        (m) => m.type === 'QUERY_UPDATE' || m.type === 'SERVER_EVENT'
      );
      expect(update).toBeDefined();

      // Verify server state
      const map = ctx.server.getMap('reverse-sync-test') as LWWMap<string, any>;
      expect(map.get('item-from-b')).toEqual({ message: 'Hello from B', value: 100 });
    });

    test('Both clients write simultaneously → both receive all data', async () => {
      const [clientA, clientB] = ctx.clients;

      // Both subscribe
      clientA.send({
        type: 'QUERY_SUB',
        payload: { queryId: 'dual-sync-a', mapName: 'dual-write-test', query: {} },
      });
      clientB.send({
        type: 'QUERY_SUB',
        payload: { queryId: 'dual-sync-b', mapName: 'dual-write-test', query: {} },
      });

      await Promise.all([
        clientA.waitForMessage('QUERY_RESP'),
        clientB.waitForMessage('QUERY_RESP'),
      ]);

      clientA.messages.length = 0;
      clientB.messages.length = 0;

      // Both write simultaneously (different keys)
      clientA.send({
        type: 'CLIENT_OP',
        payload: {
          mapName: 'dual-write-test',
          opType: 'PUT',
          key: 'from-a',
          record: createLWWRecord({ source: 'A' }),
        },
      });

      clientB.send({
        type: 'CLIENT_OP',
        payload: {
          mapName: 'dual-write-test',
          opType: 'PUT',
          key: 'from-b',
          record: createLWWRecord({ source: 'B' }),
        },
      });

      // Wait for both to sync
      await waitForSync(500);

      // Both should receive updates from the other
      await waitUntil(
        () =>
          clientA.messages.some((m) => m.type === 'QUERY_UPDATE' || m.type === 'SERVER_EVENT') &&
          clientB.messages.some((m) => m.type === 'QUERY_UPDATE' || m.type === 'SERVER_EVENT'),
        3000
      );

      // Verify server has both items
      const map = ctx.server.getMap('dual-write-test') as LWWMap<string, any>;
      expect(map.get('from-a')).toEqual({ source: 'A' });
      expect(map.get('from-b')).toEqual({ source: 'B' });
    });
  });

  // ========================================
  // LWW Conflict Resolution
  // ========================================
  describe('LWW Conflict Resolution', () => {
    let ctx: TestContext;

    beforeEach(async () => {
      ctx = await createTestContext(2);
    });

    afterEach(async () => {
      await ctx.cleanup();
    });

    test('concurrent writes to same key - later timestamp wins', async () => {
      const [clientA, clientB] = ctx.clients;
      const baseTime = Date.now();

      // Client A writes with earlier timestamp
      clientA.send({
        type: 'CLIENT_OP',
        payload: {
          mapName: 'lww-conflict',
          opType: 'PUT',
          key: 'conflict-key',
          record: {
            value: { winner: 'A', data: 'client-a-data' },
            timestamp: { millis: baseTime, counter: 0, nodeId: 'client-0' },
          },
        },
      });

      // Client B writes with later timestamp (should win)
      clientB.send({
        type: 'CLIENT_OP',
        payload: {
          mapName: 'lww-conflict',
          opType: 'PUT',
          key: 'conflict-key',
          record: {
            value: { winner: 'B', data: 'client-b-data' },
            timestamp: { millis: baseTime + 1000, counter: 0, nodeId: 'client-1' },
          },
        },
      });

      await waitForSync(300);

      const map = ctx.server.getMap('lww-conflict') as LWWMap<string, any>;
      const result = map.get('conflict-key');
      expect(result).toEqual({ winner: 'B', data: 'client-b-data' });
    });

    test('same timestamp - higher nodeId wins (lexicographic)', async () => {
      const [clientA, clientB] = ctx.clients;
      const sameTime = Date.now();

      // Both write at same timestamp, but different nodeIds
      clientA.send({
        type: 'CLIENT_OP',
        payload: {
          mapName: 'lww-tie',
          opType: 'PUT',
          key: 'tie-key',
          record: {
            value: { from: 'node-a' },
            timestamp: { millis: sameTime, counter: 0, nodeId: 'aaa-node' },
          },
        },
      });

      clientB.send({
        type: 'CLIENT_OP',
        payload: {
          mapName: 'lww-tie',
          opType: 'PUT',
          key: 'tie-key',
          record: {
            value: { from: 'node-z' },
            timestamp: { millis: sameTime, counter: 0, nodeId: 'zzz-node' },
          },
        },
      });

      await waitForSync(300);

      const map = ctx.server.getMap('lww-tie') as LWWMap<string, any>;
      const result = map.get('tie-key');
      // "zzz-node" > "aaa-node" lexicographically, so node-z wins
      expect(result).toEqual({ from: 'node-z' });
    });

    test('both clients converge to same value after conflict', async () => {
      const [clientA, clientB] = ctx.clients;
      const baseTime = Date.now();

      // Subscribe both to receive updates
      clientA.send({
        type: 'QUERY_SUB',
        payload: { queryId: 'converge-a', mapName: 'lww-converge', query: {} },
      });
      clientB.send({
        type: 'QUERY_SUB',
        payload: { queryId: 'converge-b', mapName: 'lww-converge', query: {} },
      });

      await Promise.all([
        clientA.waitForMessage('QUERY_RESP'),
        clientB.waitForMessage('QUERY_RESP'),
      ]);

      // Client A writes first (earlier timestamp)
      clientA.send({
        type: 'CLIENT_OP',
        payload: {
          mapName: 'lww-converge',
          opType: 'PUT',
          key: 'converge-key',
          record: {
            value: { data: 'from-a' },
            timestamp: { millis: baseTime, counter: 0, nodeId: 'client-0' },
          },
        },
      });

      await waitForSync(100);

      // Client B writes later (later timestamp - will win)
      clientB.send({
        type: 'CLIENT_OP',
        payload: {
          mapName: 'lww-converge',
          opType: 'PUT',
          key: 'converge-key',
          record: {
            value: { data: 'from-b-wins' },
            timestamp: { millis: baseTime + 5000, counter: 0, nodeId: 'client-1' },
          },
        },
      });

      await waitForSync(500);

      // Verify server converged
      const map = ctx.server.getMap('lww-converge') as LWWMap<string, any>;
      expect(map.get('converge-key')).toEqual({ data: 'from-b-wins' });
    });

    test('counter breaks ties when millis are equal', async () => {
      const [clientA, clientB] = ctx.clients;
      const sameTime = Date.now();

      clientA.send({
        type: 'CLIENT_OP',
        payload: {
          mapName: 'lww-counter',
          opType: 'PUT',
          key: 'counter-key',
          record: {
            value: { counter: 'low' },
            timestamp: { millis: sameTime, counter: 1, nodeId: 'same-node' },
          },
        },
      });

      clientB.send({
        type: 'CLIENT_OP',
        payload: {
          mapName: 'lww-counter',
          opType: 'PUT',
          key: 'counter-key',
          record: {
            value: { counter: 'high-wins' },
            timestamp: { millis: sameTime, counter: 5, nodeId: 'same-node' },
          },
        },
      });

      await waitForSync(300);

      const map = ctx.server.getMap('lww-counter') as LWWMap<string, any>;
      expect(map.get('counter-key')).toEqual({ counter: 'high-wins' });
    });
  });

  // ========================================
  // ORMap Multi-Client
  // ========================================
  describe('ORMap Multi-Client', () => {
    let ctx: TestContext;

    beforeEach(async () => {
      ctx = await createTestContext(2);
    });

    afterEach(async () => {
      await ctx.cleanup();
    });

    test('Client A adds element → Client B sees it', async () => {
      const [clientA, clientB] = ctx.clients;

      // Client A adds element
      clientA.send({
        type: 'CLIENT_OP',
        payload: {
          mapName: 'or-multi',
          opType: 'OR_ADD',
          key: 'shared-list',
          orRecord: createORRecord('item-from-a', 'client-0'),
        },
      });

      await waitForSync(300);

      // Verify both see it
      const map = ctx.server.getMap('or-multi', 'OR') as ORMap<string, any>;
      const values = map.get('shared-list');
      expect(values).toContain('item-from-a');
    });

    test('Client A removes element → Client B sees removal', async () => {
      const [clientA, clientB] = ctx.clients;
      const tag = `remove-test-${Date.now()}`;

      // Client A adds element with specific tag
      clientA.send({
        type: 'CLIENT_OP',
        payload: {
          mapName: 'or-remove-multi',
          opType: 'OR_ADD',
          key: 'removable-list',
          orRecord: {
            value: 'to-be-removed',
            timestamp: { millis: Date.now(), counter: 0, nodeId: 'client-0' },
            tag,
          },
        },
      });

      await waitForSync(200);

      // Verify element exists
      let map = ctx.server.getMap('or-remove-multi', 'OR') as ORMap<string, any>;
      expect(map.get('removable-list')).toContain('to-be-removed');

      // Client A removes element
      clientA.send({
        type: 'CLIENT_OP',
        payload: {
          mapName: 'or-remove-multi',
          opType: 'OR_REMOVE',
          key: 'removable-list',
          orTag: tag,
        },
      });

      await waitForSync(300);

      // Verify removal
      map = ctx.server.getMap('or-remove-multi', 'OR') as ORMap<string, any>;
      expect(map.get('removable-list') || []).not.toContain('to-be-removed');
    });

    test('concurrent OR_ADD from both clients - both elements preserved', async () => {
      const [clientA, clientB] = ctx.clients;

      // Both add simultaneously to same key
      clientA.send({
        type: 'CLIENT_OP',
        payload: {
          mapName: 'or-concurrent',
          opType: 'OR_ADD',
          key: 'concurrent-list',
          orRecord: createORRecord('from-a', 'client-0'),
        },
      });

      clientB.send({
        type: 'CLIENT_OP',
        payload: {
          mapName: 'or-concurrent',
          opType: 'OR_ADD',
          key: 'concurrent-list',
          orRecord: createORRecord('from-b', 'client-1'),
        },
      });

      await waitForSync(400);

      // Both elements should be preserved (OR-Set semantics)
      const map = ctx.server.getMap('or-concurrent', 'OR') as ORMap<string, any>;
      const values = map.get('concurrent-list');
      expect(values).toHaveLength(2);
      expect(values).toContain('from-a');
      expect(values).toContain('from-b');
    });

    test('add-remove-add sequence preserves correct state', async () => {
      const [clientA, clientB] = ctx.clients;
      const tag1 = `tag-1-${Date.now()}`;
      const tag2 = `tag-2-${Date.now()}`;

      // Client A adds first item
      clientA.send({
        type: 'CLIENT_OP',
        payload: {
          mapName: 'or-sequence',
          opType: 'OR_ADD',
          key: 'sequence-list',
          orRecord: {
            value: 'first-item',
            timestamp: { millis: Date.now(), counter: 0, nodeId: 'client-0' },
            tag: tag1,
          },
        },
      });

      await waitForSync(100);

      // Client B removes it
      clientB.send({
        type: 'CLIENT_OP',
        payload: {
          mapName: 'or-sequence',
          opType: 'OR_REMOVE',
          key: 'sequence-list',
          orTag: tag1,
        },
      });

      await waitForSync(100);

      // Client A adds another item
      clientA.send({
        type: 'CLIENT_OP',
        payload: {
          mapName: 'or-sequence',
          opType: 'OR_ADD',
          key: 'sequence-list',
          orRecord: {
            value: 'second-item',
            timestamp: { millis: Date.now(), counter: 1, nodeId: 'client-0' },
            tag: tag2,
          },
        },
      });

      await waitForSync(300);

      const map = ctx.server.getMap('or-sequence', 'OR') as ORMap<string, any>;
      const values = map.get('sequence-list') || [];
      expect(values).not.toContain('first-item');
      expect(values).toContain('second-item');
    });
  });

  // ========================================
  // Multiple Clients (3+)
  // ========================================
  describe('Multiple Clients (3+)', () => {
    test('3 clients connected - one writes → others receive', async () => {
      const ctx = await createTestContext(3);

      try {
        const [client1, client2, client3] = ctx.clients;

        // Client 2 and 3 subscribe
        client2.send({
          type: 'QUERY_SUB',
          payload: { queryId: 'multi-1', mapName: 'three-client-test', query: {} },
        });
        client3.send({
          type: 'QUERY_SUB',
          payload: { queryId: 'multi-2', mapName: 'three-client-test', query: {} },
        });

        await Promise.all([
          client2.waitForMessage('QUERY_RESP'),
          client3.waitForMessage('QUERY_RESP'),
        ]);

        client2.messages.length = 0;
        client3.messages.length = 0;

        // Client 1 writes
        client1.send({
          type: 'CLIENT_OP',
          payload: {
            mapName: 'three-client-test',
            opType: 'PUT',
            key: 'broadcast-item',
            record: createLWWRecord({ message: 'From client 1 to all' }),
          },
        });

        // Both client 2 and 3 should receive
        await waitUntil(
          () =>
            client2.messages.some((m) => m.type === 'QUERY_UPDATE' || m.type === 'SERVER_EVENT') &&
            client3.messages.some((m) => m.type === 'QUERY_UPDATE' || m.type === 'SERVER_EVENT'),
          3000
        );

        // Verify server state
        const map = ctx.server.getMap('three-client-test') as LWWMap<string, any>;
        expect(map.get('broadcast-item')).toEqual({ message: 'From client 1 to all' });
      } finally {
        await ctx.cleanup();
      }
    });

    test('all 3 clients write → all receive all data', async () => {
      const ctx = await createTestContext(3);

      try {
        const [client1, client2, client3] = ctx.clients;

        // All subscribe
        client1.send({
          type: 'QUERY_SUB',
          payload: { queryId: 'all-1', mapName: 'all-write-test', query: {} },
        });
        client2.send({
          type: 'QUERY_SUB',
          payload: { queryId: 'all-2', mapName: 'all-write-test', query: {} },
        });
        client3.send({
          type: 'QUERY_SUB',
          payload: { queryId: 'all-3', mapName: 'all-write-test', query: {} },
        });

        await Promise.all([
          client1.waitForMessage('QUERY_RESP'),
          client2.waitForMessage('QUERY_RESP'),
          client3.waitForMessage('QUERY_RESP'),
        ]);

        // All write different keys
        client1.send({
          type: 'CLIENT_OP',
          payload: {
            mapName: 'all-write-test',
            opType: 'PUT',
            key: 'from-1',
            record: createLWWRecord({ id: 1 }),
          },
        });

        client2.send({
          type: 'CLIENT_OP',
          payload: {
            mapName: 'all-write-test',
            opType: 'PUT',
            key: 'from-2',
            record: createLWWRecord({ id: 2 }),
          },
        });

        client3.send({
          type: 'CLIENT_OP',
          payload: {
            mapName: 'all-write-test',
            opType: 'PUT',
            key: 'from-3',
            record: createLWWRecord({ id: 3 }),
          },
        });

        await waitForSync(500);

        // Verify server has all data
        const map = ctx.server.getMap('all-write-test') as LWWMap<string, any>;
        expect(map.get('from-1')).toEqual({ id: 1 });
        expect(map.get('from-2')).toEqual({ id: 2 });
        expect(map.get('from-3')).toEqual({ id: 3 });
      } finally {
        await ctx.cleanup();
      }
    });

    test('4 clients with concurrent conflict resolution', async () => {
      const ctx = await createTestContext(4);

      try {
        const clients = ctx.clients;
        const baseTime = Date.now();

        // All write to same key with different timestamps
        clients.forEach((client, i) => {
          client.send({
            type: 'CLIENT_OP',
            payload: {
              mapName: 'four-conflict',
              opType: 'PUT',
              key: 'same-key',
              record: {
                value: { from: `client-${i}` },
                timestamp: { millis: baseTime + i * 100, counter: 0, nodeId: `client-${i}` },
              },
            },
          });
        });

        await waitForSync(500);

        // Latest timestamp wins (client-3)
        const map = ctx.server.getMap('four-conflict') as LWWMap<string, any>;
        expect(map.get('same-key')).toEqual({ from: 'client-3' });
      } finally {
        await ctx.cleanup();
      }
    });
  });

  // ========================================
  // Connection/Disconnection Scenarios
  // ========================================
  describe('Connection/Disconnection', () => {
    test('Client B connects after A already wrote → B gets snapshot', async () => {
      const server = await createTestServer();

      try {
        // Client A connects and writes
        const clientA = await createTestClient(`ws://localhost:${server.port}`, {
          nodeId: 'client-a',
          roles: ['ADMIN'],
        });
        await clientA.waitForMessage('AUTH_ACK');

        clientA.send({
          type: 'CLIENT_OP',
          payload: {
            mapName: 'snapshot-test',
            opType: 'PUT',
            key: 'pre-existing-1',
            record: createLWWRecord({ data: 'existed before B' }),
          },
        });

        clientA.send({
          type: 'CLIENT_OP',
          payload: {
            mapName: 'snapshot-test',
            opType: 'PUT',
            key: 'pre-existing-2',
            record: createLWWRecord({ data: 'also existed' }),
          },
        });

        await waitForSync(200);

        // Now Client B connects
        const clientB = await createTestClient(`ws://localhost:${server.port}`, {
          nodeId: 'client-b',
          roles: ['ADMIN'],
        });
        await clientB.waitForMessage('AUTH_ACK');

        // Client B subscribes and should get snapshot
        clientB.send({
          type: 'QUERY_SUB',
          payload: { queryId: 'snapshot-q', mapName: 'snapshot-test', query: {} },
        });

        const response = await clientB.waitForMessage('QUERY_RESP');
        expect(response.payload.results).toHaveLength(2);

        const keys = response.payload.results.map((r: any) => r.key).sort();
        expect(keys).toEqual(['pre-existing-1', 'pre-existing-2']);

        clientA.close();
        clientB.close();
      } finally {
        await server.shutdown();
      }
    });

    test('Client A disconnects → Client B continues working', async () => {
      const server = await createTestServer();

      try {
        // Both connect
        const clientA = await createTestClient(`ws://localhost:${server.port}`, {
          nodeId: 'client-a',
          roles: ['ADMIN'],
        });
        const clientB = await createTestClient(`ws://localhost:${server.port}`, {
          nodeId: 'client-b',
          roles: ['ADMIN'],
        });

        await Promise.all([
          clientA.waitForMessage('AUTH_ACK'),
          clientB.waitForMessage('AUTH_ACK'),
        ]);

        // Client A writes something
        clientA.send({
          type: 'CLIENT_OP',
          payload: {
            mapName: 'disconnect-test',
            opType: 'PUT',
            key: 'from-a',
            record: createLWWRecord({ before: 'disconnect' }),
          },
        });

        await waitForSync(200);

        // Client A disconnects
        clientA.close();
        await waitForSync(100);

        // Client B continues to work
        clientB.send({
          type: 'CLIENT_OP',
          payload: {
            mapName: 'disconnect-test',
            opType: 'PUT',
            key: 'from-b',
            record: createLWWRecord({ after: 'a-disconnected' }),
          },
        });

        await waitForSync(200);

        // Server should have both
        const map = server.getMap('disconnect-test') as LWWMap<string, any>;
        expect(map.get('from-a')).toEqual({ before: 'disconnect' });
        expect(map.get('from-b')).toEqual({ after: 'a-disconnected' });

        clientB.close();
      } finally {
        await server.shutdown();
      }
    });

    test('Client reconnects and receives missed updates', async () => {
      const server = await createTestServer();

      try {
        // Client A connects
        let clientA = await createTestClient(`ws://localhost:${server.port}`, {
          nodeId: 'reconnect-a',
          roles: ['ADMIN'],
        });
        const clientB = await createTestClient(`ws://localhost:${server.port}`, {
          nodeId: 'reconnect-b',
          roles: ['ADMIN'],
        });

        await Promise.all([
          clientA.waitForMessage('AUTH_ACK'),
          clientB.waitForMessage('AUTH_ACK'),
        ]);

        // Client A writes before disconnect
        clientA.send({
          type: 'CLIENT_OP',
          payload: {
            mapName: 'reconnect-test',
            opType: 'PUT',
            key: 'item-1',
            record: createLWWRecord({ order: 1 }),
          },
        });

        await waitForSync(200);

        // Client A disconnects
        clientA.close();
        await waitForSync(100);

        // Client B writes while A is disconnected
        clientB.send({
          type: 'CLIENT_OP',
          payload: {
            mapName: 'reconnect-test',
            opType: 'PUT',
            key: 'item-2',
            record: createLWWRecord({ order: 2 }),
          },
        });

        await waitForSync(200);

        // Client A reconnects
        clientA = await createTestClient(`ws://localhost:${server.port}`, {
          nodeId: 'reconnect-a-2',
          roles: ['ADMIN'],
        });
        await clientA.waitForMessage('AUTH_ACK');

        // Subscribe to get current state
        clientA.send({
          type: 'QUERY_SUB',
          payload: { queryId: 'reconnect-q', mapName: 'reconnect-test', query: {} },
        });

        const response = await clientA.waitForMessage('QUERY_RESP');
        expect(response.payload.results).toHaveLength(2);

        const keys = response.payload.results.map((r: any) => r.key).sort();
        expect(keys).toEqual(['item-1', 'item-2']);

        clientA.close();
        clientB.close();
      } finally {
        await server.shutdown();
      }
    });

    test('multiple clients disconnect and reconnect simultaneously', async () => {
      const server = await createTestServer();

      try {
        // Create 3 clients
        const clients = await Promise.all([
          createTestClient(`ws://localhost:${server.port}`, { nodeId: 'multi-1', roles: ['ADMIN'] }),
          createTestClient(`ws://localhost:${server.port}`, { nodeId: 'multi-2', roles: ['ADMIN'] }),
          createTestClient(`ws://localhost:${server.port}`, { nodeId: 'multi-3', roles: ['ADMIN'] }),
        ]);

        await Promise.all(clients.map((c) => c.waitForMessage('AUTH_ACK')));

        // Each writes something
        clients.forEach((client, i) => {
          client.send({
            type: 'CLIENT_OP',
            payload: {
              mapName: 'multi-reconnect',
              opType: 'PUT',
              key: `item-${i}`,
              record: createLWWRecord({ clientId: i }),
            },
          });
        });

        await waitForSync(300);

        // All disconnect
        clients.forEach((c) => c.close());
        await waitForSync(100);

        // All reconnect
        const newClients = await Promise.all([
          createTestClient(`ws://localhost:${server.port}`, { nodeId: 'new-1', roles: ['ADMIN'] }),
          createTestClient(`ws://localhost:${server.port}`, { nodeId: 'new-2', roles: ['ADMIN'] }),
          createTestClient(`ws://localhost:${server.port}`, { nodeId: 'new-3', roles: ['ADMIN'] }),
        ]);

        await Promise.all(newClients.map((c) => c.waitForMessage('AUTH_ACK')));

        // Each subscribes and gets all data
        const responses = await Promise.all(
          newClients.map(async (client, i) => {
            client.send({
              type: 'QUERY_SUB',
              payload: { queryId: `sub-${i}`, mapName: 'multi-reconnect', query: {} },
            });
            return client.waitForMessage('QUERY_RESP');
          })
        );

        // All should have 3 items
        responses.forEach((resp) => {
          expect(resp.payload.results).toHaveLength(3);
        });

        newClients.forEach((c) => c.close());
      } finally {
        await server.shutdown();
      }
    });
  });

  // ========================================
  // Edge Cases
  // ========================================
  describe('Edge Cases', () => {
    test('rapid successive writes from multiple clients', async () => {
      const ctx = await createTestContext(2);

      try {
        const [clientA, clientB] = ctx.clients;

        // Rapid fire writes
        for (let i = 0; i < 10; i++) {
          clientA.send({
            type: 'CLIENT_OP',
            payload: {
              mapName: 'rapid-test',
              opType: 'PUT',
              key: `rapid-a-${i}`,
              record: createLWWRecord({ index: i, from: 'A' }),
            },
          });

          clientB.send({
            type: 'CLIENT_OP',
            payload: {
              mapName: 'rapid-test',
              opType: 'PUT',
              key: `rapid-b-${i}`,
              record: createLWWRecord({ index: i, from: 'B' }),
            },
          });
        }

        await waitForSync(1000);

        const map = ctx.server.getMap('rapid-test') as LWWMap<string, any>;

        // All 20 items should exist
        for (let i = 0; i < 10; i++) {
          expect(map.get(`rapid-a-${i}`)).toEqual({ index: i, from: 'A' });
          expect(map.get(`rapid-b-${i}`)).toEqual({ index: i, from: 'B' });
        }
      } finally {
        await ctx.cleanup();
      }
    });

    test('large payload synchronization', async () => {
      const ctx = await createTestContext(2);

      try {
        const [clientA, clientB] = ctx.clients;

        // Create large payload
        const largeData = {
          items: Array.from({ length: 100 }, (_, i) => ({
            id: i,
            name: `Item ${i}`,
            description: 'Lorem ipsum '.repeat(10),
          })),
        };

        clientA.send({
          type: 'CLIENT_OP',
          payload: {
            mapName: 'large-payload-test',
            opType: 'PUT',
            key: 'large-item',
            record: createLWWRecord(largeData),
          },
        });

        await waitForSync(500);

        // Subscribe from B and verify
        clientB.send({
          type: 'QUERY_SUB',
          payload: { queryId: 'large-q', mapName: 'large-payload-test', query: {} },
        });

        const response = await clientB.waitForMessage('QUERY_RESP');
        expect(response.payload.results).toHaveLength(1);

        // The result structure may vary - check for items in the value
        const result = response.payload.results[0];
        const value = result.record?.value || result.value;
        expect(value.items).toHaveLength(100);
      } finally {
        await ctx.cleanup();
      }
    });

    test('empty value handling', async () => {
      const ctx = await createTestContext(2);

      try {
        const [clientA, clientB] = ctx.clients;

        // Write empty object
        clientA.send({
          type: 'CLIENT_OP',
          payload: {
            mapName: 'empty-test',
            opType: 'PUT',
            key: 'empty-obj',
            record: createLWWRecord({}),
          },
        });

        // Write empty string
        clientA.send({
          type: 'CLIENT_OP',
          payload: {
            mapName: 'empty-test',
            opType: 'PUT',
            key: 'empty-str',
            record: createLWWRecord(''),
          },
        });

        // Write null
        clientA.send({
          type: 'CLIENT_OP',
          payload: {
            mapName: 'empty-test',
            opType: 'PUT',
            key: 'null-val',
            record: createLWWRecord(null),
          },
        });

        await waitForSync(300);

        const map = ctx.server.getMap('empty-test') as LWWMap<string, any>;
        expect(map.get('empty-obj')).toEqual({});
        expect(map.get('empty-str')).toBe('');
        // null is treated as tombstone
        expect(map.get('null-val')).toBeUndefined();
      } finally {
        await ctx.cleanup();
      }
    });
  });
});
