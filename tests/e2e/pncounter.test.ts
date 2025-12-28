import {
  createTestServer,
  createTestClient,
  createTestContext,
  waitForSync,
  waitUntil,
  MemoryStorageAdapter,
} from './helpers';

describe('E2E: PN Counter', () => {
  // ========================================
  // Basic Counter Operations
  // ========================================
  describe('Basic Operations', () => {
    test('client can increment and decrement counter via server', async () => {
      const ctx = await createTestContext(1);

      try {
        const [client] = ctx.clients;

        // Send counter sync with increment
        client.send({
          type: 'COUNTER_SYNC',
          payload: {
            name: 'test-counter',
            state: {
              p: { [client.nodeId]: 5 },
              n: {},
            },
          },
        });

        const response = await client.waitForMessage('COUNTER_UPDATE');
        expect(response.type).toBe('COUNTER_UPDATE');
        expect(response.payload.name).toBe('test-counter');
        expect(response.payload.state.p[client.nodeId]).toBe(5);
      } finally {
        await ctx.cleanup();
      }
    });

    test('server broadcasts counter updates to other clients', async () => {
      const ctx = await createTestContext(2);

      try {
        const [clientA, clientB] = ctx.clients;

        // Client B subscribes to counter by requesting it
        clientB.send({
          type: 'COUNTER_REQUEST',
          payload: { name: 'broadcast-counter' },
        });
        await clientB.waitForMessage('COUNTER_RESPONSE');

        // Clear message queue
        clientB.messages.length = 0;

        // Client A increments
        clientA.send({
          type: 'COUNTER_SYNC',
          payload: {
            name: 'broadcast-counter',
            state: {
              p: { [clientA.nodeId]: 10 },
              n: {},
            },
          },
        });

        await waitForSync(200);

        // Client B should receive the update
        const update = await clientB.waitForMessage('COUNTER_UPDATE');
        expect(update.payload.name).toBe('broadcast-counter');
        expect(update.payload.state.p[clientA.nodeId]).toBe(10);
      } finally {
        await ctx.cleanup();
      }
    });

    test('counter values merge correctly from multiple clients', async () => {
      const ctx = await createTestContext(3);

      try {
        const [clientA, clientB, clientC] = ctx.clients;

        // Each client subscribes
        clientA.send({ type: 'COUNTER_REQUEST', payload: { name: 'merge-counter' } });
        clientB.send({ type: 'COUNTER_REQUEST', payload: { name: 'merge-counter' } });
        clientC.send({ type: 'COUNTER_REQUEST', payload: { name: 'merge-counter' } });

        await Promise.all([
          clientA.waitForMessage('COUNTER_RESPONSE'),
          clientB.waitForMessage('COUNTER_RESPONSE'),
          clientC.waitForMessage('COUNTER_RESPONSE'),
        ]);

        // Each client sends their increments
        clientA.send({
          type: 'COUNTER_SYNC',
          payload: {
            name: 'merge-counter',
            state: { p: { 'node-A': 10 }, n: {} },
          },
        });

        await waitForSync(100);

        clientB.send({
          type: 'COUNTER_SYNC',
          payload: {
            name: 'merge-counter',
            state: { p: { 'node-B': 20 }, n: {} },
          },
        });

        await waitForSync(100);

        clientC.send({
          type: 'COUNTER_SYNC',
          payload: {
            name: 'merge-counter',
            state: { p: { 'node-C': 30 }, n: { 'node-C': 5 } },
          },
        });

        await waitForSync(300);

        // Request final state
        clientA.messages.length = 0;
        clientA.send({ type: 'COUNTER_REQUEST', payload: { name: 'merge-counter' } });
        const finalState = await clientA.waitForMessage('COUNTER_RESPONSE');

        // Merged state should have all nodes
        expect(finalState.payload.state.p['node-A']).toBe(10);
        expect(finalState.payload.state.p['node-B']).toBe(20);
        expect(finalState.payload.state.p['node-C']).toBe(30);
        expect(finalState.payload.state.n['node-C']).toBe(5);

        // Total value = 10 + 20 + 30 - 5 = 55
      } finally {
        await ctx.cleanup();
      }
    });
  });

  // ========================================
  // Offline Persistence
  // ========================================
  describe('Offline Persistence', () => {
    test('counter state is persisted to storage', async () => {
      const storage = new MemoryStorageAdapter();
      await storage.initialize('test-db');

      // Simulate counter state being saved
      const counterState = {
        p: { 'node-1': 10, 'node-2': 5 },
        n: { 'node-1': 2 },
      };
      await storage.setMeta('__counter__:likes', counterState);

      // Verify it can be retrieved
      const retrieved = await storage.getMeta('__counter__:likes');
      expect(retrieved).toEqual(counterState);
      expect(retrieved.p['node-1']).toBe(10);
      expect(retrieved.p['node-2']).toBe(5);
      expect(retrieved.n['node-1']).toBe(2);
    });

    test('counter state survives storage round-trip', async () => {
      const storage = new MemoryStorageAdapter();
      await storage.initialize('test-db');

      // Write state
      const originalState = {
        p: { 'client-abc': 100, 'client-xyz': 50 },
        n: { 'client-abc': 10, 'client-xyz': 5 },
      };
      await storage.setMeta('__counter__:inventory', originalState);

      // Read back
      const restoredState = await storage.getMeta('__counter__:inventory');

      // Calculate values
      const positiveSum = Object.values(restoredState.p as Record<string, number>).reduce((a, b) => a + b, 0);
      const negativeSum = Object.values(restoredState.n as Record<string, number>).reduce((a, b) => a + b, 0);
      const value = positiveSum - negativeSum;

      expect(value).toBe(135); // (100 + 50) - (10 + 5)
    });

    test('multiple counters can be stored independently', async () => {
      const storage = new MemoryStorageAdapter();
      await storage.initialize('test-db');

      await storage.setMeta('__counter__:likes', { p: { n: 10 }, n: {} });
      await storage.setMeta('__counter__:views', { p: { n: 1000 }, n: {} });
      await storage.setMeta('__counter__:shares', { p: { n: 50 }, n: { n: 5 } });

      const likes = await storage.getMeta('__counter__:likes');
      const views = await storage.getMeta('__counter__:views');
      const shares = await storage.getMeta('__counter__:shares');

      expect(likes.p.n).toBe(10);
      expect(views.p.n).toBe(1000);
      expect(shares.p.n).toBe(50);
      expect(shares.n.n).toBe(5);
    });
  });

  // ========================================
  // CRDT Convergence
  // ========================================
  describe('CRDT Convergence', () => {
    test('concurrent increments from different clients converge', async () => {
      const ctx = await createTestContext(2);

      try {
        const [clientA, clientB] = ctx.clients;

        // Both clients subscribe
        clientA.send({ type: 'COUNTER_REQUEST', payload: { name: 'converge-counter' } });
        clientB.send({ type: 'COUNTER_REQUEST', payload: { name: 'converge-counter' } });

        await Promise.all([
          clientA.waitForMessage('COUNTER_RESPONSE'),
          clientB.waitForMessage('COUNTER_RESPONSE'),
        ]);

        // Simulate concurrent increments (no network between clients)
        clientA.send({
          type: 'COUNTER_SYNC',
          payload: {
            name: 'converge-counter',
            state: { p: { 'A': 5 }, n: {} },
          },
        });

        clientB.send({
          type: 'COUNTER_SYNC',
          payload: {
            name: 'converge-counter',
            state: { p: { 'B': 3 }, n: {} },
          },
        });

        await waitForSync(300);

        // Both should converge to same state
        clientA.messages.length = 0;
        clientB.messages.length = 0;

        clientA.send({ type: 'COUNTER_REQUEST', payload: { name: 'converge-counter' } });
        clientB.send({ type: 'COUNTER_REQUEST', payload: { name: 'converge-counter' } });

        const [stateA, stateB] = await Promise.all([
          clientA.waitForMessage('COUNTER_RESPONSE'),
          clientB.waitForMessage('COUNTER_RESPONSE'),
        ]);

        // Both should see A=5, B=3
        expect(stateA.payload.state.p['A']).toBe(5);
        expect(stateA.payload.state.p['B']).toBe(3);
        expect(stateB.payload.state.p['A']).toBe(5);
        expect(stateB.payload.state.p['B']).toBe(3);
      } finally {
        await ctx.cleanup();
      }
    });

    test('idempotent syncs do not change counter value', async () => {
      const server = await createTestServer();

      try {
        const client = await createTestClient(`ws://localhost:${server.port}`, {
          nodeId: 'idempotent-client',
          roles: ['ADMIN'],
        });
        await client.waitForMessage('AUTH_ACK');

        const state = {
          p: { 'node': 10 },
          n: { 'node': 3 },
        };

        // Send same state multiple times
        for (let i = 0; i < 3; i++) {
          client.send({
            type: 'COUNTER_SYNC',
            payload: { name: 'idempotent-counter', state },
          });
          await waitForSync(50);
        }

        // Request final state
        client.messages.length = 0;
        client.send({ type: 'COUNTER_REQUEST', payload: { name: 'idempotent-counter' } });
        const response = await client.waitForMessage('COUNTER_RESPONSE');

        // Value should still be 7 (10 - 3)
        expect(response.payload.state.p['node']).toBe(10);
        expect(response.payload.state.n['node']).toBe(3);

        client.close();
      } finally {
        await server.shutdown();
      }
    });

    test('higher values always win in merge', async () => {
      const server = await createTestServer();

      try {
        const client = await createTestClient(`ws://localhost:${server.port}`, {
          nodeId: 'lww-client',
          roles: ['ADMIN'],
        });
        await client.waitForMessage('AUTH_ACK');

        // First sync with higher value
        client.send({
          type: 'COUNTER_SYNC',
          payload: {
            name: 'lww-counter',
            state: { p: { 'node': 100 }, n: {} },
          },
        });
        await waitForSync(100);

        // Try to sync with lower value
        client.send({
          type: 'COUNTER_SYNC',
          payload: {
            name: 'lww-counter',
            state: { p: { 'node': 50 }, n: {} },
          },
        });
        await waitForSync(100);

        // Request final state
        client.messages.length = 0;
        client.send({ type: 'COUNTER_REQUEST', payload: { name: 'lww-counter' } });
        const response = await client.waitForMessage('COUNTER_RESPONSE');

        // Higher value (100) should be preserved
        expect(response.payload.state.p['node']).toBe(100);

        client.close();
      } finally {
        await server.shutdown();
      }
    });
  });

  // ========================================
  // Reconnect Scenarios
  // ========================================
  describe('Reconnect Scenarios', () => {
    test('offline counter state syncs on reconnect', async () => {
      const server = await createTestServer();

      try {
        // Client A connects and increments
        let clientA = await createTestClient(`ws://localhost:${server.port}`, {
          nodeId: 'offline-client-a',
          roles: ['ADMIN'],
        });
        await clientA.waitForMessage('AUTH_ACK');

        clientA.send({
          type: 'COUNTER_SYNC',
          payload: {
            name: 'offline-counter',
            state: { p: { 'A': 10 }, n: {} },
          },
        });
        await waitForSync(200);
        clientA.close();

        // Client B connects while A is offline
        const clientB = await createTestClient(`ws://localhost:${server.port}`, {
          nodeId: 'offline-client-b',
          roles: ['ADMIN'],
        });
        await clientB.waitForMessage('AUTH_ACK');

        clientB.send({
          type: 'COUNTER_SYNC',
          payload: {
            name: 'offline-counter',
            state: { p: { 'B': 20 }, n: {} },
          },
        });
        await waitForSync(200);

        // Client A reconnects
        clientA = await createTestClient(`ws://localhost:${server.port}`, {
          nodeId: 'offline-client-a-2',
          roles: ['ADMIN'],
        });
        await clientA.waitForMessage('AUTH_ACK');

        // Client A gets current state
        clientA.send({ type: 'COUNTER_REQUEST', payload: { name: 'offline-counter' } });
        const response = await clientA.waitForMessage('COUNTER_RESPONSE');

        // Should see both A and B contributions
        expect(response.payload.state.p['A']).toBe(10);
        expect(response.payload.state.p['B']).toBe(20);

        clientA.close();
        clientB.close();
      } finally {
        await server.shutdown();
      }
    });

    test('client sends accumulated offline changes on reconnect', async () => {
      const server = await createTestServer();

      try {
        // Simulate offline accumulation
        const offlineState = {
          p: { 'offline-node': 50 },
          n: { 'offline-node': 10 },
        };

        // Client reconnects with accumulated state
        const client = await createTestClient(`ws://localhost:${server.port}`, {
          nodeId: 'reconnect-client',
          roles: ['ADMIN'],
        });
        await client.waitForMessage('AUTH_ACK');

        // Sync offline state
        client.send({
          type: 'COUNTER_SYNC',
          payload: {
            name: 'reconnect-counter',
            state: offlineState,
          },
        });

        const response = await client.waitForMessage('COUNTER_UPDATE');
        expect(response.payload.state.p['offline-node']).toBe(50);
        expect(response.payload.state.n['offline-node']).toBe(10);

        client.close();
      } finally {
        await server.shutdown();
      }
    });
  });

  // ========================================
  // Edge Cases
  // ========================================
  describe('Edge Cases', () => {
    test('counter with only decrements works correctly', async () => {
      const server = await createTestServer();

      try {
        const client = await createTestClient(`ws://localhost:${server.port}`, {
          nodeId: 'decrement-client',
          roles: ['ADMIN'],
        });
        await client.waitForMessage('AUTH_ACK');

        // Only decrements (negative counter)
        client.send({
          type: 'COUNTER_SYNC',
          payload: {
            name: 'negative-counter',
            state: { p: {}, n: { 'node': 5 } },
          },
        });

        const response = await client.waitForMessage('COUNTER_UPDATE');
        expect(response.payload.state.p).toEqual({});
        expect(response.payload.state.n['node']).toBe(5);

        client.close();
      } finally {
        await server.shutdown();
      }
    });

    test('counter with many nodes handles correctly', async () => {
      const server = await createTestServer();

      try {
        const client = await createTestClient(`ws://localhost:${server.port}`, {
          nodeId: 'many-nodes-client',
          roles: ['ADMIN'],
        });
        await client.waitForMessage('AUTH_ACK');

        // Create state with many nodes (starting from 1 to avoid zero values)
        const positive: Record<string, number> = {};
        const negative: Record<string, number> = {};
        for (let i = 1; i <= 100; i++) {
          positive[`node-${i}`] = i;
          if (i % 2 === 0) {
            negative[`node-${i}`] = Math.floor(i / 2);
          }
        }

        client.send({
          type: 'COUNTER_SYNC',
          payload: {
            name: 'many-nodes-counter',
            state: { p: positive, n: negative },
          },
        });

        const response = await client.waitForMessage('COUNTER_UPDATE');

        // Verify some nodes
        expect(response.payload.state.p['node-1']).toBe(1);
        expect(response.payload.state.p['node-50']).toBe(50);
        expect(response.payload.state.p['node-100']).toBe(100);
        expect(response.payload.state.n['node-50']).toBe(25);

        client.close();
      } finally {
        await server.shutdown();
      }
    });

    test('empty counter state is handled correctly', async () => {
      const server = await createTestServer();

      try {
        const client = await createTestClient(`ws://localhost:${server.port}`, {
          nodeId: 'empty-counter-client',
          roles: ['ADMIN'],
        });
        await client.waitForMessage('AUTH_ACK');

        // Request non-existent counter
        client.send({ type: 'COUNTER_REQUEST', payload: { name: 'new-counter' } });

        const response = await client.waitForMessage('COUNTER_RESPONSE');
        expect(response.payload.state.p).toEqual({});
        expect(response.payload.state.n).toEqual({});

        client.close();
      } finally {
        await server.shutdown();
      }
    });
  });
});
