import WebSocket from 'ws';
import {
  createRustTestContext,
  createRustTestClient,
  spawnRustServer,
  createTestToken,
  waitForSync,
  TestClient,
} from './helpers';

describe('Integration: Connection & Auth (Rust Server)', () => {
  // ========================================
  // Connection Tests
  // ========================================
  describe('Connection', () => {
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

    test('client connects to Rust server (WebSocket OPEN state)', async () => {
      const client = await createRustTestClient(port);

      expect(client.ws.readyState).toBe(WebSocket.OPEN);

      client.close();
    });

    test('client receives AUTH_REQUIRED message on connect', async () => {
      const client = await createRustTestClient(port, {
        autoAuth: false,
      });

      const authRequired = await client.waitForMessage('AUTH_REQUIRED');
      expect(authRequired).toBeDefined();
      expect(authRequired.type).toBe('AUTH_REQUIRED');

      client.close();
    });

    test('client sends valid JWT, receives AUTH_ACK', async () => {
      const client = await createRustTestClient(port);

      const authAck = await client.waitForMessage('AUTH_ACK');
      expect(authAck).toBeDefined();
      expect(authAck.type).toBe('AUTH_ACK');
      expect(client.isAuthenticated).toBe(true);

      client.close();
    });

    test('client sends invalid JWT, receives AUTH_FAIL', async () => {
      const client = await createRustTestClient(port, {
        autoAuth: false,
      });

      await client.waitForMessage('AUTH_REQUIRED');

      // Send an invalid token that will fail JWT verification
      client.send({ type: 'AUTH', token: 'invalid-token-garbage' });

      const authFail = await client.waitForMessage('AUTH_FAIL');
      expect(authFail).toBeDefined();
      expect(authFail.type).toBe('AUTH_FAIL');

      // Wait briefly for server to process the failure
      await waitForSync(100);
    });
  });

  // ========================================
  // Reconnect Tests
  // ========================================
  describe('Reconnect', () => {
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

    test('client reconnects after disconnect', async () => {
      // First connection
      const client1 = await createRustTestClient(port);
      await client1.waitForMessage('AUTH_ACK');
      expect(client1.isAuthenticated).toBe(true);

      // Close the first connection
      client1.close();
      await waitForSync(200);

      // Reconnect with a new client
      const client2 = await createRustTestClient(port);
      const authAck = await client2.waitForMessage('AUTH_ACK');
      expect(authAck).toBeDefined();
      expect(authAck.type).toBe('AUTH_ACK');
      expect(client2.isAuthenticated).toBe(true);

      client2.close();
    });

    test('multiple clients connect simultaneously', async () => {
      const clients: TestClient[] = [];

      try {
        // Connect 3 clients in parallel
        const clientPromises = Array.from({ length: 3 }, (_, i) =>
          createRustTestClient(port, {
            nodeId: `multi-client-${i}`,
            userId: `user-${i}`,
          })
        );

        const connectedClients = await Promise.all(clientPromises);
        clients.push(...connectedClients);

        // Wait for all clients to authenticate
        await Promise.all(
          clients.map((c) => c.waitForMessage('AUTH_ACK', 10_000))
        );

        // Verify all clients are authenticated
        for (const client of clients) {
          expect(client.ws.readyState).toBe(WebSocket.OPEN);
          expect(client.isAuthenticated).toBe(true);
        }
      } finally {
        for (const client of clients) {
          client.close();
        }
      }
    });
  });
});
