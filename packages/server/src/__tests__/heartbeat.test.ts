import { ServerCoordinator } from '../ServerCoordinator';
import { deserialize, serialize } from '@topgunbuild/core';

describe('Heartbeat', () => {
  describe('Server', () => {
    let server: ServerCoordinator;

    beforeAll(async () => {
      server = new ServerCoordinator({
        port: 0,
        nodeId: 'test-server',
        host: 'localhost',
        clusterPort: 0,
        peers: [],
      });
      await server.ready();
    });

    afterAll(async () => {
      await server.shutdown();
    });

    const createMockClient = (id: string, authenticated = true) => {
      const socket = {
        send: jest.fn(),
        readyState: 1, // OPEN
        close: jest.fn(),
      };

      const client = {
        id,
        socket: socket as any,
        isAuthenticated: authenticated,
        principal: authenticated ? { roles: ['USER'], userId: id } : undefined,
        subscriptions: new Set<string>(),
        lastActiveHlc: { millis: Date.now(), counter: 0, nodeId: 'test-server' },
        lastPingReceived: Date.now(),
      };

      return { client, socket };
    };

    it('should respond with PONG immediately on PING', async () => {
      const { client, socket } = createMockClient('client-ping-1');
      (server as any).clients.set('client-ping-1', client);

      const clientTimestamp = Date.now();

      await (server as any).handleMessage(client, {
        type: 'PING',
        timestamp: clientTimestamp,
      });

      expect(socket.send).toHaveBeenCalled();
      const calls = socket.send.mock.calls.map(
        (c) => deserialize(c[0] as Uint8Array) as any
      );
      const pongMsg = calls.find((m) => m.type === 'PONG');

      expect(pongMsg).toBeDefined();
      expect(pongMsg.timestamp).toBe(clientTimestamp);
      expect(pongMsg.serverTime).toBeGreaterThan(0);

      // Cleanup
      (server as any).clients.delete('client-ping-1');
    });

    it('should include serverTime in PONG', async () => {
      const { client, socket } = createMockClient('client-ping-2');
      (server as any).clients.set('client-ping-2', client);

      const beforeTime = Date.now();
      const clientTimestamp = beforeTime - 100;

      await (server as any).handleMessage(client, {
        type: 'PING',
        timestamp: clientTimestamp,
      });

      const afterTime = Date.now();

      const calls = socket.send.mock.calls.map(
        (c) => deserialize(c[0] as Uint8Array) as any
      );
      const pongMsg = calls.find((m) => m.type === 'PONG');

      expect(pongMsg.serverTime).toBeGreaterThanOrEqual(beforeTime);
      expect(pongMsg.serverTime).toBeLessThanOrEqual(afterTime);

      // Cleanup
      (server as any).clients.delete('client-ping-2');
    });

    it('should track lastPingReceived per client', async () => {
      const { client: client1 } = createMockClient('client-ping-3');
      const { client: client2 } = createMockClient('client-ping-4');

      // Set different initial lastPingReceived
      client1.lastPingReceived = Date.now() - 10000;
      client2.lastPingReceived = Date.now() - 5000;

      (server as any).clients.set('client-ping-3', client1);
      (server as any).clients.set('client-ping-4', client2);

      const timestamp1 = Date.now();

      // Only client1 sends PING
      await (server as any).handleMessage(client1, {
        type: 'PING',
        timestamp: timestamp1,
      });

      // client1's lastPingReceived should be updated
      expect(client1.lastPingReceived).toBeGreaterThanOrEqual(timestamp1);

      // client2's lastPingReceived should NOT be updated
      expect(client2.lastPingReceived).toBeLessThan(timestamp1);

      // Cleanup
      (server as any).clients.delete('client-ping-3');
      (server as any).clients.delete('client-ping-4');
    });

    it('should report client as not alive after clientTimeoutMs', async () => {
      const { client } = createMockClient('client-ping-5');

      // Set lastPingReceived to old time (beyond 20s timeout)
      client.lastPingReceived = Date.now() - 25000;

      (server as any).clients.set('client-ping-5', client);

      expect(server.isClientAlive('client-ping-5')).toBe(false);

      // Cleanup
      (server as any).clients.delete('client-ping-5');
    });

    it('should report client as alive when recently pinged', async () => {
      const { client } = createMockClient('client-ping-6');

      // Set lastPingReceived to recent time
      client.lastPingReceived = Date.now() - 5000;

      (server as any).clients.set('client-ping-6', client);

      expect(server.isClientAlive('client-ping-6')).toBe(true);

      // Cleanup
      (server as any).clients.delete('client-ping-6');
    });

    it('should return Infinity idle time for unknown client', () => {
      expect(server.getClientIdleTime('non-existent-client')).toBe(Infinity);
    });

    it('should return false for isClientAlive on unknown client', () => {
      expect(server.isClientAlive('non-existent-client')).toBe(false);
    });

    it('should calculate correct idle time', async () => {
      const { client } = createMockClient('client-ping-7');

      const tenSecondsAgo = Date.now() - 10000;
      client.lastPingReceived = tenSecondsAgo;

      (server as any).clients.set('client-ping-7', client);

      const idleTime = server.getClientIdleTime('client-ping-7');
      expect(idleTime).toBeGreaterThanOrEqual(10000);
      expect(idleTime).toBeLessThan(11000); // Allow 1 second tolerance

      // Cleanup
      (server as any).clients.delete('client-ping-7');
    });

    it('should update lastPingReceived after PING', async () => {
      const { client } = createMockClient('client-ping-8');

      // Set old lastPingReceived
      const oldTime = Date.now() - 15000;
      client.lastPingReceived = oldTime;

      (server as any).clients.set('client-ping-8', client);

      // Send PING
      const timestamp = Date.now();
      await (server as any).handleMessage(client, {
        type: 'PING',
        timestamp,
      });

      // lastPingReceived should be updated
      expect(client.lastPingReceived).toBeGreaterThan(oldTime);
      expect(client.lastPingReceived).toBeGreaterThanOrEqual(timestamp);

      // Cleanup
      (server as any).clients.delete('client-ping-8');
    });

    it('should handle PING even for unauthenticated clients', async () => {
      const { client, socket } = createMockClient('client-ping-9', false);
      (server as any).clients.set('client-ping-9', client);

      const clientTimestamp = Date.now();

      await (server as any).handleMessage(client, {
        type: 'PING',
        timestamp: clientTimestamp,
      });

      expect(socket.send).toHaveBeenCalled();
      const calls = socket.send.mock.calls.map(
        (c) => deserialize(c[0] as Uint8Array) as any
      );
      const pongMsg = calls.find((m) => m.type === 'PONG');

      expect(pongMsg).toBeDefined();
      expect(pongMsg.timestamp).toBe(clientTimestamp);

      // Cleanup
      (server as any).clients.delete('client-ping-9');
    });
  });

  describe('Server - Dead Client Eviction', () => {
    let server: ServerCoordinator;

    beforeEach(async () => {
      server = new ServerCoordinator({
        port: 0,
        nodeId: 'test-server-eviction',
        host: 'localhost',
        clusterPort: 0,
        peers: [],
      });
      await server.ready();
    });

    afterEach(async () => {
      await server.shutdown();
    });

    it('should evict dead clients during heartbeat check', async () => {
      const socket = {
        send: jest.fn(),
        readyState: 1, // OPEN
        close: jest.fn(),
      };

      const client = {
        id: 'dead-client-1',
        socket: socket as any,
        isAuthenticated: true,
        principal: { roles: ['USER'], userId: 'dead-client-1' },
        subscriptions: new Set<string>(),
        lastActiveHlc: { millis: Date.now(), counter: 0, nodeId: 'test-server' },
        lastPingReceived: Date.now() - 25000, // 25 seconds ago (beyond 20s timeout)
      };

      (server as any).clients.set('dead-client-1', client);

      // Manually trigger eviction check
      (server as any).evictDeadClients();

      // Socket should be closed
      expect(socket.close).toHaveBeenCalledWith(4002, 'Heartbeat timeout');
    });

    it('should NOT evict clients that are still alive', async () => {
      const socket = {
        send: jest.fn(),
        readyState: 1, // OPEN
        close: jest.fn(),
      };

      const client = {
        id: 'alive-client-1',
        socket: socket as any,
        isAuthenticated: true,
        principal: { roles: ['USER'], userId: 'alive-client-1' },
        subscriptions: new Set<string>(),
        lastActiveHlc: { millis: Date.now(), counter: 0, nodeId: 'test-server' },
        lastPingReceived: Date.now() - 5000, // 5 seconds ago (within 20s timeout)
      };

      (server as any).clients.set('alive-client-1', client);

      // Manually trigger eviction check
      (server as any).evictDeadClients();

      // Socket should NOT be closed
      expect(socket.close).not.toHaveBeenCalled();

      // Cleanup
      (server as any).clients.delete('alive-client-1');
    });

    it('should NOT evict unauthenticated clients', async () => {
      const socket = {
        send: jest.fn(),
        readyState: 1, // OPEN
        close: jest.fn(),
      };

      const client = {
        id: 'unauth-client-1',
        socket: socket as any,
        isAuthenticated: false, // Not authenticated
        subscriptions: new Set<string>(),
        lastActiveHlc: { millis: Date.now(), counter: 0, nodeId: 'test-server' },
        lastPingReceived: Date.now() - 25000, // Old but unauthenticated
      };

      (server as any).clients.set('unauth-client-1', client);

      // Manually trigger eviction check
      (server as any).evictDeadClients();

      // Socket should NOT be closed (unauthenticated clients are handled by auth timeout)
      expect(socket.close).not.toHaveBeenCalled();

      // Cleanup
      (server as any).clients.delete('unauth-client-1');
    });
  });

  describe('Integration', () => {
    let server: ServerCoordinator;

    beforeEach(async () => {
      server = new ServerCoordinator({
        port: 0,
        nodeId: 'test-server-integration',
        host: 'localhost',
        clusterPort: 0,
        peers: [],
      });
      await server.ready();
    });

    afterEach(async () => {
      await server.shutdown();
    });

    it('should maintain connection with heartbeats over 30 seconds', async () => {
      const socket = {
        send: jest.fn(),
        readyState: 1,
        close: jest.fn(),
      };

      const client = {
        id: 'long-lived-client',
        socket: socket as any,
        isAuthenticated: true,
        principal: { roles: ['USER'], userId: 'long-lived-client' },
        subscriptions: new Set<string>(),
        lastActiveHlc: { millis: Date.now(), counter: 0, nodeId: 'test-server' },
        lastPingReceived: Date.now(),
      };

      (server as any).clients.set('long-lived-client', client);

      // Simulate 30 seconds of heartbeats (every 5 seconds)
      for (let i = 0; i < 6; i++) {
        // Check that client is still alive
        expect(server.isClientAlive('long-lived-client')).toBe(true);

        // Simulate PING from client
        const timestamp = Date.now();
        await (server as any).handleMessage(client, {
          type: 'PING',
          timestamp,
        });

        // Verify PONG was sent
        const calls = socket.send.mock.calls.map(
          (c) => deserialize(c[0] as Uint8Array) as any
        );
        const pongMsgs = calls.filter((m) => m.type === 'PONG');
        expect(pongMsgs.length).toBeGreaterThanOrEqual(i + 1);

        // Simulate 5 seconds passing
        client.lastPingReceived = Date.now();
      }

      // Client should still be connected (not evicted)
      expect(socket.close).not.toHaveBeenCalled();
      expect(server.isClientAlive('long-lived-client')).toBe(true);

      // Cleanup
      (server as any).clients.delete('long-lived-client');
    });

    it('should detect and evict after simulated freeze', async () => {
      const socket = {
        send: jest.fn(),
        readyState: 1,
        close: jest.fn(),
      };

      const client = {
        id: 'frozen-client',
        socket: socket as any,
        isAuthenticated: true,
        principal: { roles: ['USER'], userId: 'frozen-client' },
        subscriptions: new Set<string>(),
        lastActiveHlc: { millis: Date.now(), counter: 0, nodeId: 'test-server' },
        lastPingReceived: Date.now(),
      };

      (server as any).clients.set('frozen-client', client);

      // Initially alive
      expect(server.isClientAlive('frozen-client')).toBe(true);

      // Simulate client freeze by setting old lastPingReceived
      // (21 seconds ago - beyond 20s timeout)
      client.lastPingReceived = Date.now() - 21000;

      // Now client should be reported as not alive
      expect(server.isClientAlive('frozen-client')).toBe(false);

      // Trigger eviction
      (server as any).evictDeadClients();

      // Socket should be closed
      expect(socket.close).toHaveBeenCalledWith(4002, 'Heartbeat timeout');
    });

    it('should handle rapid PING messages', async () => {
      const socket = {
        send: jest.fn(),
        readyState: 1,
        close: jest.fn(),
      };

      const client = {
        id: 'rapid-client',
        socket: socket as any,
        isAuthenticated: true,
        principal: { roles: ['USER'], userId: 'rapid-client' },
        subscriptions: new Set<string>(),
        lastActiveHlc: { millis: Date.now(), counter: 0, nodeId: 'test-server' },
        lastPingReceived: Date.now(),
      };

      (server as any).clients.set('rapid-client', client);

      // Send 100 rapid PINGs
      for (let i = 0; i < 100; i++) {
        await (server as any).handleMessage(client, {
          type: 'PING',
          timestamp: Date.now() + i,
        });
      }

      // All should result in PONGs
      const calls = socket.send.mock.calls.map(
        (c) => deserialize(c[0] as Uint8Array) as any
      );
      const pongMsgs = calls.filter((m) => m.type === 'PONG');
      expect(pongMsgs).toHaveLength(100);

      // Client should still be alive
      expect(server.isClientAlive('rapid-client')).toBe(true);

      // Cleanup
      (server as any).clients.delete('rapid-client');
    });
  });
});
