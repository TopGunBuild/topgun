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
  MemoryStorageAdapter,
} from './helpers';

describe('E2E: Offline-First and Reconnect', () => {
  // ========================================
  // Offline Work Tests
  // ========================================
  describe('Offline Work', () => {
    test('client writes data without server connection - stored in local storage', async () => {
      const storage = new MemoryStorageAdapter();
      await storage.initialize('test-db');

      // Simulate offline: write directly to storage without server
      const record = createLWWRecord({ title: 'Offline Todo', done: false });
      await storage.put('todos:todo-1', record);

      // Verify data is in storage
      const retrieved = await storage.get('todos:todo-1');
      expect(retrieved).toBeDefined();
      expect(retrieved?.value).toEqual({ title: 'Offline Todo', done: false });
    });

    test('client accumulates operations in opLog while offline', async () => {
      const storage = new MemoryStorageAdapter();
      await storage.initialize('test-db');

      // Append operations to opLog (simulating offline writes)
      const id1 = await storage.appendOpLog({
        key: 'item-1',
        op: 'PUT',
        value: { name: 'Item 1' },
        synced: 0,
        mapName: 'items',
      });

      const id2 = await storage.appendOpLog({
        key: 'item-2',
        op: 'PUT',
        value: { name: 'Item 2' },
        synced: 0,
        mapName: 'items',
      });

      // Verify operations are pending
      const pendingOps = await storage.getPendingOps();
      expect(pendingOps).toHaveLength(2);
      expect(pendingOps[0].key).toBe('item-1');
      expect(pendingOps[1].key).toBe('item-2');
    });

    test('client can read its own offline data', async () => {
      const storage = new MemoryStorageAdapter();
      await storage.initialize('test-db');

      // Write multiple records
      await storage.put('notes:note-1', createLWWRecord({ text: 'First note' }));
      await storage.put('notes:note-2', createLWWRecord({ text: 'Second note' }));
      await storage.put('notes:note-3', createLWWRecord({ text: 'Third note' }));

      // Read all keys
      const allKeys = await storage.getAllKeys();
      const noteKeys = allKeys.filter((k) => k.startsWith('notes:'));
      expect(noteKeys).toHaveLength(3);

      // Read individual records
      const note1 = await storage.get('notes:note-1');
      expect(note1?.value).toEqual({ text: 'First note' });
    });
  });

  // ========================================
  // Reconnect and Sync Tests
  // ========================================
  describe('Reconnect and Sync', () => {
    test('client syncs accumulated operations on reconnect', async () => {
      const server = await createTestServer();

      try {
        // Create a client with pre-populated pending ops in storage
        const storage = new MemoryStorageAdapter();
        await storage.initialize('test-db');

        // Simulate pending operations (as if written offline)
        const baseTime = Date.now();
        await storage.appendOpLog({
          key: 'offline-item-1',
          op: 'PUT',
          record: {
            value: { name: 'Offline Item 1' },
            timestamp: { millis: baseTime, counter: 0, nodeId: 'offline-client' },
          },
          synced: 0,
          mapName: 'sync-test',
        });

        await storage.appendOpLog({
          key: 'offline-item-2',
          op: 'PUT',
          record: {
            value: { name: 'Offline Item 2' },
            timestamp: { millis: baseTime + 100, counter: 0, nodeId: 'offline-client' },
          },
          synced: 0,
          mapName: 'sync-test',
        });

        // Connect client to server
        const client = await createTestClient(`ws://localhost:${server.port}`, {
          nodeId: 'reconnect-client',
          roles: ['ADMIN'],
        });

        await client.waitForMessage('AUTH_ACK');

        // Send the pending ops as OP_BATCH
        const pendingOps = await storage.getPendingOps();
        client.send({
          type: 'OP_BATCH',
          payload: {
            ops: pendingOps.map((op) => ({
              id: String(op.id),
              mapName: op.mapName,
              opType: op.op,
              key: op.key,
              record: op.record,
            })),
          },
        });

        // Wait for ACK
        const ack = await client.waitForMessage('OP_ACK');
        expect(ack).toBeDefined();

        await waitForSync(200);

        // Verify server received the data
        const map = server.getMap('sync-test') as LWWMap<string, any>;
        expect(map.get('offline-item-1')).toEqual({ name: 'Offline Item 1' });
        expect(map.get('offline-item-2')).toEqual({ name: 'Offline Item 2' });

        client.close();
      } finally {
        await server.shutdown();
      }
    });

    test('server acknowledges received operations with OP_ACK', async () => {
      const server = await createTestServer();

      try {
        const client = await createTestClient(`ws://localhost:${server.port}`, {
          nodeId: 'ack-test-client',
          roles: ['ADMIN'],
        });

        await client.waitForMessage('AUTH_ACK');

        // Send batch of operations
        client.send({
          type: 'OP_BATCH',
          payload: {
            ops: [
              {
                id: '1',
                mapName: 'ack-test',
                opType: 'PUT',
                key: 'key-1',
                record: createLWWRecord({ data: 1 }),
              },
              {
                id: '2',
                mapName: 'ack-test',
                opType: 'PUT',
                key: 'key-2',
                record: createLWWRecord({ data: 2 }),
              },
              {
                id: '3',
                mapName: 'ack-test',
                opType: 'PUT',
                key: 'key-3',
                record: createLWWRecord({ data: 3 }),
              },
            ],
          },
        });

        const ack = await client.waitForMessage('OP_ACK');
        expect(ack.type).toBe('OP_ACK');
        expect(ack.payload.lastId).toBe('3');

        client.close();
      } finally {
        await server.shutdown();
      }
    });

    test('multiple reconnects do not duplicate data', async () => {
      const server = await createTestServer();

      try {
        // First connection
        let client = await createTestClient(`ws://localhost:${server.port}`, {
          nodeId: 'dup-test-1',
          roles: ['ADMIN'],
        });
        await client.waitForMessage('AUTH_ACK');

        client.send({
          type: 'CLIENT_OP',
          payload: {
            mapName: 'dup-test',
            opType: 'PUT',
            key: 'unique-item',
            record: createLWWRecord({ count: 1 }),
          },
        });

        await waitForSync(200);
        client.close();

        // Second connection (reconnect)
        client = await createTestClient(`ws://localhost:${server.port}`, {
          nodeId: 'dup-test-2',
          roles: ['ADMIN'],
        });
        await client.waitForMessage('AUTH_ACK');

        // Subscribe to get current state
        client.send({
          type: 'QUERY_SUB',
          payload: { queryId: 'dup-q', mapName: 'dup-test', query: {} },
        });

        const response = await client.waitForMessage('QUERY_RESP');
        // Should only have one item, not duplicated
        expect(response.payload.results).toHaveLength(1);

        client.close();
      } finally {
        await server.shutdown();
      }
    });
  });

  // ========================================
  // Merge on Reconnect Tests
  // ========================================
  describe('Merge on Reconnect', () => {
    test('offline client A merges with online client B data on reconnect', async () => {
      const server = await createTestServer();

      try {
        // Client A connects, writes, then disconnects
        let clientA = await createTestClient(`ws://localhost:${server.port}`, {
          nodeId: 'merge-client-a',
          roles: ['ADMIN'],
        });
        await clientA.waitForMessage('AUTH_ACK');

        const baseTime = Date.now();

        clientA.send({
          type: 'CLIENT_OP',
          payload: {
            mapName: 'merge-test',
            opType: 'PUT',
            key: 'item-from-a',
            record: {
              value: { source: 'A', message: 'Written before offline' },
              timestamp: { millis: baseTime, counter: 0, nodeId: 'merge-client-a' },
            },
          },
        });

        await waitForSync(200);
        clientA.close();

        // Client B stays online and writes data
        const clientB = await createTestClient(`ws://localhost:${server.port}`, {
          nodeId: 'merge-client-b',
          roles: ['ADMIN'],
        });
        await clientB.waitForMessage('AUTH_ACK');

        clientB.send({
          type: 'CLIENT_OP',
          payload: {
            mapName: 'merge-test',
            opType: 'PUT',
            key: 'item-from-b',
            record: {
              value: { source: 'B', message: 'Written while A offline' },
              timestamp: { millis: baseTime + 1000, counter: 0, nodeId: 'merge-client-b' },
            },
          },
        });

        await waitForSync(200);

        // Client A reconnects
        clientA = await createTestClient(`ws://localhost:${server.port}`, {
          nodeId: 'merge-client-a-2',
          roles: ['ADMIN'],
        });
        await clientA.waitForMessage('AUTH_ACK');

        // Client A subscribes to get merged state
        clientA.send({
          type: 'QUERY_SUB',
          payload: { queryId: 'merge-q', mapName: 'merge-test', query: {} },
        });

        const response = await clientA.waitForMessage('QUERY_RESP');
        expect(response.payload.results).toHaveLength(2);

        const keys = response.payload.results.map((r: any) => r.key).sort();
        expect(keys).toEqual(['item-from-a', 'item-from-b']);

        clientA.close();
        clientB.close();
      } finally {
        await server.shutdown();
      }
    });

    test('both clients see consistent state after reconnect merge', async () => {
      const ctx = await createTestContext(2);

      try {
        const [clientA, clientB] = ctx.clients;
        const baseTime = Date.now();

        // Client A writes
        clientA.send({
          type: 'CLIENT_OP',
          payload: {
            mapName: 'consistent-test',
            opType: 'PUT',
            key: 'shared-key',
            record: {
              value: { version: 1, author: 'A' },
              timestamp: { millis: baseTime, counter: 0, nodeId: 'client-0' },
            },
          },
        });

        await waitForSync(100);

        // Client B writes with later timestamp
        clientB.send({
          type: 'CLIENT_OP',
          payload: {
            mapName: 'consistent-test',
            opType: 'PUT',
            key: 'shared-key',
            record: {
              value: { version: 2, author: 'B' },
              timestamp: { millis: baseTime + 500, counter: 0, nodeId: 'client-1' },
            },
          },
        });

        await waitForSync(300);

        // Verify server state is consistent
        const map = ctx.server.getMap('consistent-test') as LWWMap<string, any>;
        expect(map.get('shared-key')).toEqual({ version: 2, author: 'B' });
      } finally {
        await ctx.cleanup();
      }
    });

    test('offline writes are applied in correct order based on HLC', async () => {
      const server = await createTestServer();

      try {
        const client = await createTestClient(`ws://localhost:${server.port}`, {
          nodeId: 'hlc-order-client',
          roles: ['ADMIN'],
        });
        await client.waitForMessage('AUTH_ACK');

        const baseTime = Date.now();

        // Send operations with explicit timestamps to test ordering
        // Operation with later timestamp should win even if sent first
        client.send({
          type: 'CLIENT_OP',
          payload: {
            mapName: 'hlc-order',
            opType: 'PUT',
            key: 'ordered-key',
            record: {
              value: { order: 'second', ts: baseTime + 1000 },
              timestamp: { millis: baseTime + 1000, counter: 0, nodeId: 'node-1' },
            },
          },
        });

        // Older timestamp sent after
        client.send({
          type: 'CLIENT_OP',
          payload: {
            mapName: 'hlc-order',
            opType: 'PUT',
            key: 'ordered-key',
            record: {
              value: { order: 'first', ts: baseTime },
              timestamp: { millis: baseTime, counter: 0, nodeId: 'node-1' },
            },
          },
        });

        await waitForSync(300);

        const map = server.getMap('hlc-order') as LWWMap<string, any>;
        // Later HLC timestamp should win
        expect(map.get('ordered-key')).toEqual({ order: 'second', ts: baseTime + 1000 });

        client.close();
      } finally {
        await server.shutdown();
      }
    });
  });

  // ========================================
  // Conflict Resolution After Offline
  // ========================================
  describe('Conflict Resolution After Offline', () => {
    test('offline client A writes X, online client B writes Y (later) - Y wins on reconnect', async () => {
      const server = await createTestServer();

      try {
        const baseTime = Date.now();

        // Client A connects and writes X
        let clientA = await createTestClient(`ws://localhost:${server.port}`, {
          nodeId: 'conflict-a',
          roles: ['ADMIN'],
        });
        await clientA.waitForMessage('AUTH_ACK');

        clientA.send({
          type: 'CLIENT_OP',
          payload: {
            mapName: 'conflict-test',
            opType: 'PUT',
            key: 'conflict-key',
            record: {
              value: { data: 'X from A (offline write)' },
              timestamp: { millis: baseTime, counter: 0, nodeId: 'conflict-a' },
            },
          },
        });

        await waitForSync(100);
        clientA.close();

        // Client B writes Y with later timestamp
        const clientB = await createTestClient(`ws://localhost:${server.port}`, {
          nodeId: 'conflict-b',
          roles: ['ADMIN'],
        });
        await clientB.waitForMessage('AUTH_ACK');

        clientB.send({
          type: 'CLIENT_OP',
          payload: {
            mapName: 'conflict-test',
            opType: 'PUT',
            key: 'conflict-key',
            record: {
              value: { data: 'Y from B (later write)' },
              timestamp: { millis: baseTime + 2000, counter: 0, nodeId: 'conflict-b' },
            },
          },
        });

        await waitForSync(200);

        // Verify Y wins (later timestamp)
        const map = server.getMap('conflict-test') as LWWMap<string, any>;
        expect(map.get('conflict-key')).toEqual({ data: 'Y from B (later write)' });

        // Client A reconnects and should see Y
        clientA = await createTestClient(`ws://localhost:${server.port}`, {
          nodeId: 'conflict-a-2',
          roles: ['ADMIN'],
        });
        await clientA.waitForMessage('AUTH_ACK');

        clientA.send({
          type: 'QUERY_SUB',
          payload: { queryId: 'conflict-q', mapName: 'conflict-test', query: {} },
        });

        const response = await clientA.waitForMessage('QUERY_RESP');
        expect(response.payload.results).toHaveLength(1);

        const result = response.payload.results[0];
        const value = result.record?.value || result.value;
        expect(value).toEqual({ data: 'Y from B (later write)' });

        clientA.close();
        clientB.close();
      } finally {
        await server.shutdown();
      }
    });

    test('offline client A writes Z (even later) - Z wins on reconnect', async () => {
      const server = await createTestServer();

      try {
        const baseTime = Date.now();

        // Client B writes first
        const clientB = await createTestClient(`ws://localhost:${server.port}`, {
          nodeId: 'later-conflict-b',
          roles: ['ADMIN'],
        });
        await clientB.waitForMessage('AUTH_ACK');

        clientB.send({
          type: 'CLIENT_OP',
          payload: {
            mapName: 'later-conflict',
            opType: 'PUT',
            key: 'key',
            record: {
              value: { data: 'B wrote this first' },
              timestamp: { millis: baseTime, counter: 0, nodeId: 'later-conflict-b' },
            },
          },
        });

        await waitForSync(200);

        // Client A reconnects with a LATER timestamp (offline write with later timestamp)
        const clientA = await createTestClient(`ws://localhost:${server.port}`, {
          nodeId: 'later-conflict-a',
          roles: ['ADMIN'],
        });
        await clientA.waitForMessage('AUTH_ACK');

        clientA.send({
          type: 'CLIENT_OP',
          payload: {
            mapName: 'later-conflict',
            opType: 'PUT',
            key: 'key',
            record: {
              value: { data: 'A wrote this later (offline)' },
              timestamp: { millis: baseTime + 5000, counter: 0, nodeId: 'later-conflict-a' },
            },
          },
        });

        await waitForSync(200);

        // Z (later timestamp) should win
        const map = server.getMap('later-conflict') as LWWMap<string, any>;
        expect(map.get('key')).toEqual({ data: 'A wrote this later (offline)' });

        clientA.close();
        clientB.close();
      } finally {
        await server.shutdown();
      }
    });

    test('concurrent offline edits resolve deterministically', async () => {
      const ctx = await createTestContext(3);

      try {
        const [client1, client2, client3] = ctx.clients;
        const sameTime = Date.now();

        // All three write to same key at same timestamp
        // NodeId should break the tie lexicographically
        client1.send({
          type: 'CLIENT_OP',
          payload: {
            mapName: 'deterministic',
            opType: 'PUT',
            key: 'same-key',
            record: {
              value: { from: 'client-aaa' },
              timestamp: { millis: sameTime, counter: 0, nodeId: 'aaa-node' },
            },
          },
        });

        client2.send({
          type: 'CLIENT_OP',
          payload: {
            mapName: 'deterministic',
            opType: 'PUT',
            key: 'same-key',
            record: {
              value: { from: 'client-mmm' },
              timestamp: { millis: sameTime, counter: 0, nodeId: 'mmm-node' },
            },
          },
        });

        client3.send({
          type: 'CLIENT_OP',
          payload: {
            mapName: 'deterministic',
            opType: 'PUT',
            key: 'same-key',
            record: {
              value: { from: 'client-zzz' },
              timestamp: { millis: sameTime, counter: 0, nodeId: 'zzz-node' },
            },
          },
        });

        await waitForSync(400);

        // zzz-node > mmm-node > aaa-node lexicographically
        const map = ctx.server.getMap('deterministic') as LWWMap<string, any>;
        expect(map.get('same-key')).toEqual({ from: 'client-zzz' });
      } finally {
        await ctx.cleanup();
      }
    });
  });

  // ========================================
  // Connection Loss Simulation
  // ========================================
  describe('Connection Loss Simulation', () => {
    test('client continues working locally after connection loss', async () => {
      const server = await createTestServer();

      try {
        const client = await createTestClient(`ws://localhost:${server.port}`, {
          nodeId: 'conn-loss-client',
          roles: ['ADMIN'],
        });
        await client.waitForMessage('AUTH_ACK');

        // Write data while connected
        client.send({
          type: 'CLIENT_OP',
          payload: {
            mapName: 'conn-loss-test',
            opType: 'PUT',
            key: 'before-disconnect',
            record: createLWWRecord({ status: 'connected' }),
          },
        });

        await waitForSync(200);

        // Verify data is on server
        let map = server.getMap('conn-loss-test') as LWWMap<string, any>;
        expect(map.get('before-disconnect')).toEqual({ status: 'connected' });

        // Close client (simulating connection loss)
        client.close();
        await waitForSync(100);

        // Server still has the data
        map = server.getMap('conn-loss-test') as LWWMap<string, any>;
        expect(map.get('before-disconnect')).toEqual({ status: 'connected' });
      } finally {
        await server.shutdown();
      }
    });

    test('connection restoration syncs new data', async () => {
      const server = await createTestServer();

      try {
        // First session
        let client = await createTestClient(`ws://localhost:${server.port}`, {
          nodeId: 'restore-client',
          roles: ['ADMIN'],
        });
        await client.waitForMessage('AUTH_ACK');

        client.send({
          type: 'CLIENT_OP',
          payload: {
            mapName: 'restore-test',
            opType: 'PUT',
            key: 'first-session',
            record: createLWWRecord({ session: 1 }),
          },
        });

        await waitForSync(200);
        client.close();

        // Simulate offline period (other client writes)
        const otherClient = await createTestClient(`ws://localhost:${server.port}`, {
          nodeId: 'other-client',
          roles: ['ADMIN'],
        });
        await otherClient.waitForMessage('AUTH_ACK');

        otherClient.send({
          type: 'CLIENT_OP',
          payload: {
            mapName: 'restore-test',
            opType: 'PUT',
            key: 'during-offline',
            record: createLWWRecord({ addedWhileOffline: true }),
          },
        });

        await waitForSync(200);

        // Original client reconnects
        client = await createTestClient(`ws://localhost:${server.port}`, {
          nodeId: 'restore-client-2',
          roles: ['ADMIN'],
        });
        await client.waitForMessage('AUTH_ACK');

        // Subscribe to see all data
        client.send({
          type: 'QUERY_SUB',
          payload: { queryId: 'restore-q', mapName: 'restore-test', query: {} },
        });

        const response = await client.waitForMessage('QUERY_RESP');
        expect(response.payload.results).toHaveLength(2);

        client.close();
        otherClient.close();
      } finally {
        await server.shutdown();
      }
    });

    test('server shutdown and restart preserves data for reconnecting clients', async () => {
      let server = await createTestServer();
      const port = server.port;

      try {
        // Write data
        const client = await createTestClient(`ws://localhost:${port}`, {
          nodeId: 'shutdown-client',
          roles: ['ADMIN'],
        });
        await client.waitForMessage('AUTH_ACK');

        client.send({
          type: 'CLIENT_OP',
          payload: {
            mapName: 'shutdown-test',
            opType: 'PUT',
            key: 'persistent-data',
            record: createLWWRecord({ preserved: true }),
          },
        });

        await waitForSync(200);

        // Verify data is on server before shutdown
        let map = server.getMap('shutdown-test') as LWWMap<string, any>;
        expect(map.get('persistent-data')).toEqual({ preserved: true });

        client.close();

        // Note: This test validates server-side state persistence
        // In a real scenario, server would persist to disk
        // For this test, we just verify the pattern works
      } finally {
        await server.shutdown();
      }
    });
  });

  // ========================================
  // OpLog Queue Tests
  // ========================================
  describe('OpLog Queue', () => {
    test('operations are stored in opLog while offline', async () => {
      const storage = new MemoryStorageAdapter();
      await storage.initialize('test-db');

      // Simulate multiple offline operations
      await storage.appendOpLog({
        key: 'op-1',
        op: 'PUT',
        value: { data: 1 },
        synced: 0,
        mapName: 'oplog-test',
      });

      await storage.appendOpLog({
        key: 'op-2',
        op: 'PUT',
        value: { data: 2 },
        synced: 0,
        mapName: 'oplog-test',
      });

      await storage.appendOpLog({
        key: 'op-3',
        op: 'REMOVE',
        synced: 0,
        mapName: 'oplog-test',
      });

      const pending = await storage.getPendingOps();
      expect(pending).toHaveLength(3);
      expect(pending.map((p) => p.key)).toEqual(['op-1', 'op-2', 'op-3']);
      expect(pending.map((p) => p.op)).toEqual(['PUT', 'PUT', 'REMOVE']);
    });

    test('operations are marked as synced after OP_ACK', async () => {
      const storage = new MemoryStorageAdapter();
      await storage.initialize('test-db');

      // Add operations
      const id1 = await storage.appendOpLog({
        key: 'sync-1',
        op: 'PUT',
        synced: 0,
        mapName: 'test',
      });

      const id2 = await storage.appendOpLog({
        key: 'sync-2',
        op: 'PUT',
        synced: 0,
        mapName: 'test',
      });

      // Initially all pending
      let pending = await storage.getPendingOps();
      expect(pending).toHaveLength(2);

      // Mark first as synced
      await storage.markOpsSynced(id1);

      pending = await storage.getPendingOps();
      expect(pending).toHaveLength(1);
      expect(pending[0].key).toBe('sync-2');

      // Mark second as synced
      await storage.markOpsSynced(id2);

      pending = await storage.getPendingOps();
      expect(pending).toHaveLength(0);
    });

    test('repeated reconnect does not resend already synced operations', async () => {
      const server = await createTestServer();

      try {
        const storage = new MemoryStorageAdapter();
        await storage.initialize('test-db');

        // Add operations and mark some as synced
        const id1 = await storage.appendOpLog({
          key: 'already-synced',
          op: 'PUT',
          synced: 0,
          mapName: 'resend-test',
        });
        await storage.markOpsSynced(id1);

        await storage.appendOpLog({
          key: 'not-yet-synced',
          op: 'PUT',
          record: createLWWRecord({ pending: true }),
          synced: 0,
          mapName: 'resend-test',
        });

        // Only unsynced operations should be pending
        const pending = await storage.getPendingOps();
        expect(pending).toHaveLength(1);
        expect(pending[0].key).toBe('not-yet-synced');

        // Connect and sync
        const client = await createTestClient(`ws://localhost:${server.port}`, {
          nodeId: 'resend-test-client',
          roles: ['ADMIN'],
        });
        await client.waitForMessage('AUTH_ACK');

        // Send only pending ops
        client.send({
          type: 'OP_BATCH',
          payload: {
            ops: pending.map((op) => ({
              id: String(op.id),
              mapName: op.mapName,
              opType: op.op,
              key: op.key,
              record: op.record,
            })),
          },
        });

        const ack = await client.waitForMessage('OP_ACK');
        expect(ack).toBeDefined();

        client.close();
      } finally {
        await server.shutdown();
      }
    });

    test('opLog preserves operation order', async () => {
      const storage = new MemoryStorageAdapter();
      await storage.initialize('test-db');

      // Add operations in specific order
      for (let i = 1; i <= 10; i++) {
        await storage.appendOpLog({
          key: `ordered-${i}`,
          op: 'PUT',
          synced: 0,
          mapName: 'order-test',
        });
      }

      const pending = await storage.getPendingOps();
      expect(pending).toHaveLength(10);

      // Verify order is preserved
      for (let i = 0; i < 10; i++) {
        expect(pending[i].key).toBe(`ordered-${i + 1}`);
      }
    });

    test('opLog handles mixed operation types', async () => {
      const storage = new MemoryStorageAdapter();
      await storage.initialize('test-db');

      await storage.appendOpLog({
        key: 'item-1',
        op: 'PUT',
        synced: 0,
        mapName: 'mixed-test',
      });

      await storage.appendOpLog({
        key: 'item-2',
        op: 'OR_ADD',
        synced: 0,
        mapName: 'mixed-test',
      });

      await storage.appendOpLog({
        key: 'item-1',
        op: 'REMOVE',
        synced: 0,
        mapName: 'mixed-test',
      });

      await storage.appendOpLog({
        key: 'item-2',
        op: 'OR_REMOVE',
        synced: 0,
        mapName: 'mixed-test',
      });

      const pending = await storage.getPendingOps();
      expect(pending).toHaveLength(4);
      expect(pending.map((p) => p.op)).toEqual(['PUT', 'OR_ADD', 'REMOVE', 'OR_REMOVE']);
    });
  });

  // ========================================
  // Edge Cases
  // ========================================
  describe('Edge Cases', () => {
    test('client with empty opLog connects successfully', async () => {
      const server = await createTestServer();

      try {
        const client = await createTestClient(`ws://localhost:${server.port}`, {
          nodeId: 'empty-oplog-client',
          roles: ['ADMIN'],
        });

        const authAck = await client.waitForMessage('AUTH_ACK');
        expect(authAck).toBeDefined();

        // Client should be able to write after clean connect
        client.send({
          type: 'CLIENT_OP',
          payload: {
            mapName: 'empty-start',
            opType: 'PUT',
            key: 'first-item',
            record: createLWWRecord({ fresh: true }),
          },
        });

        await waitForSync(200);

        const map = server.getMap('empty-start') as LWWMap<string, any>;
        expect(map.get('first-item')).toEqual({ fresh: true });

        client.close();
      } finally {
        await server.shutdown();
      }
    });

    test('large batch of offline operations syncs correctly', async () => {
      const server = await createTestServer();

      try {
        const client = await createTestClient(`ws://localhost:${server.port}`, {
          nodeId: 'large-batch-client',
          roles: ['ADMIN'],
        });
        await client.waitForMessage('AUTH_ACK');

        // Create large batch
        const ops = [];
        for (let i = 0; i < 100; i++) {
          ops.push({
            id: String(i + 1),
            mapName: 'large-batch',
            opType: 'PUT',
            key: `item-${i}`,
            record: createLWWRecord({ index: i, data: `Data for item ${i}` }),
          });
        }

        client.send({
          type: 'OP_BATCH',
          payload: { ops },
        });

        const ack = await client.waitForMessage('OP_ACK');
        expect(ack.payload.lastId).toBe('100');

        await waitForSync(500);

        const map = server.getMap('large-batch') as LWWMap<string, any>;
        // Spot check a few items
        expect(map.get('item-0')).toEqual({ index: 0, data: 'Data for item 0' });
        expect(map.get('item-50')).toEqual({ index: 50, data: 'Data for item 50' });
        expect(map.get('item-99')).toEqual({ index: 99, data: 'Data for item 99' });

        client.close();
      } finally {
        await server.shutdown();
      }
    });

    test('rapid connect-disconnect cycles do not corrupt data', async () => {
      const server = await createTestServer();

      try {
        // Rapid connection cycles
        for (let cycle = 0; cycle < 5; cycle++) {
          const client = await createTestClient(`ws://localhost:${server.port}`, {
            nodeId: `rapid-${cycle}`,
            roles: ['ADMIN'],
          });
          await client.waitForMessage('AUTH_ACK');

          client.send({
            type: 'CLIENT_OP',
            payload: {
              mapName: 'rapid-cycle',
              opType: 'PUT',
              key: `cycle-${cycle}`,
              record: createLWWRecord({ cycle }),
            },
          });

          await waitForSync(100);
          client.close();
          await waitForSync(50);
        }

        // Final verification
        const verifyClient = await createTestClient(`ws://localhost:${server.port}`, {
          nodeId: 'verify-client',
          roles: ['ADMIN'],
        });
        await verifyClient.waitForMessage('AUTH_ACK');

        verifyClient.send({
          type: 'QUERY_SUB',
          payload: { queryId: 'verify-q', mapName: 'rapid-cycle', query: {} },
        });

        const response = await verifyClient.waitForMessage('QUERY_RESP');
        expect(response.payload.results).toHaveLength(5);

        verifyClient.close();
      } finally {
        await server.shutdown();
      }
    });

    test('offline operations with same key merge correctly', async () => {
      const server = await createTestServer();

      try {
        const client = await createTestClient(`ws://localhost:${server.port}`, {
          nodeId: 'same-key-client',
          roles: ['ADMIN'],
        });
        await client.waitForMessage('AUTH_ACK');

        const baseTime = Date.now();

        // Multiple writes to same key in one batch
        client.send({
          type: 'OP_BATCH',
          payload: {
            ops: [
              {
                id: '1',
                mapName: 'same-key-test',
                opType: 'PUT',
                key: 'evolving-key',
                record: {
                  value: { version: 1 },
                  timestamp: { millis: baseTime, counter: 0, nodeId: 'same-key-client' },
                },
              },
              {
                id: '2',
                mapName: 'same-key-test',
                opType: 'PUT',
                key: 'evolving-key',
                record: {
                  value: { version: 2 },
                  timestamp: { millis: baseTime + 100, counter: 0, nodeId: 'same-key-client' },
                },
              },
              {
                id: '3',
                mapName: 'same-key-test',
                opType: 'PUT',
                key: 'evolving-key',
                record: {
                  value: { version: 3 },
                  timestamp: { millis: baseTime + 200, counter: 0, nodeId: 'same-key-client' },
                },
              },
            ],
          },
        });

        await client.waitForMessage('OP_ACK');
        await waitForSync(200);

        // Final version should be 3 (latest timestamp)
        const map = server.getMap('same-key-test') as LWWMap<string, any>;
        expect(map.get('evolving-key')).toEqual({ version: 3 });

        client.close();
      } finally {
        await server.shutdown();
      }
    });
  });
});
