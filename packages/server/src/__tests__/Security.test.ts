import { ServerCoordinator, ServerFactory } from '../';
import { LWWRecord, deserialize, PermissionPolicy, serialize } from '@topgunbuild/core';
import { createTestHarness, ServerTestHarness } from './utils/ServerTestHarness';
import { pollUntil } from './utils/test-helpers';

// Mock WebSocket
class MockWebSocket {
    readyState = 1; // OPEN
    send = jest.fn();
    close = jest.fn();
    on = jest.fn();
}

const createMockWriter = (socket: any) => ({
  write: jest.fn((message: any, _urgent?: boolean) => {
    const data = serialize(message);
    socket.send(data);
  }),
  writeRaw: jest.fn((data: Uint8Array) => {
    socket.send(data);
  }),
  flush: jest.fn(),
  close: jest.fn(),
  getMetrics: jest.fn(() => ({
    messagesSent: 0,
    batchesSent: 0,
    bytesSent: 0,
    avgMessagesPerBatch: 0,
  })),
});

describe('Field-Level Security (RBAC)', () => {
  let server: ServerCoordinator;
  let harness: ServerTestHarness;
  const policies: PermissionPolicy[] = [
      {
          role: 'USER',
          mapNamePattern: 'profiles',
          actions: ['READ'],
          allowedFields: ['publicName']
      },
      {
          role: 'ADMIN',
          mapNamePattern: '*',
          actions: ['ALL'],
          allowedFields: ['*']
      }
  ];

  beforeAll(async () => {
    server = ServerFactory.create({
      port: 0,
      nodeId: 'test-node-sec',
      host: 'localhost',
      clusterPort: 0,
      peers: [],
      securityPolicies: policies
    });
    await server.ready();
    harness = createTestHarness(server);
  });

  afterAll(async () => {
    await server.shutdown();
  });

  const createRecord = (value: any): LWWRecord<any> => ({
    value,
    timestamp: { millis: Date.now(), counter: 0, nodeId: 'test-node-sec' }
  });

  test('Should filter fields on QUERY_RESP for USER role', async () => {
      // 1. Seed Data
      const map = server.getMap('profiles') as any; // Cast to any to access merge freely or LWWMap
      map.merge('user1', createRecord({ publicName: 'Alex', email: 'secret@test.com', internalId: 123 }));

      // 2. Mock Client (USER role)
      const clientSocket = new MockWebSocket();
      const clientMock = {
          id: 'client-user',
          socket: clientSocket as any,
          writer: createMockWriter(clientSocket) as any,
          isAuthenticated: true,
          subscriptions: new Set(),
          principal: { userId: 'u1', roles: ['USER'] }
      };
      harness.connectionManager.getClients().set('client-user', clientMock);

      // 3. Send QUERY_SUB
      const queryId = 'q1';
      await harness.handleMessage(clientMock, {
          type: 'QUERY_SUB',
          payload: {
              queryId,
              mapName: 'profiles',
              query: {}
          }
      });

      // 4. Verify Response
      expect(clientSocket.send).toHaveBeenCalled();
      // First arg is Buffer/Uint8Array usually
      const args = clientSocket.send.mock.calls[0][0];
      const response = deserialize(args) as any;

      expect(response.type).toBe('QUERY_RESP');
      expect(response.payload.results).toHaveLength(1);
      const record = response.payload.results[0];
      expect(record.key).toBe('user1');
      expect(record.value).toEqual({ publicName: 'Alex' }); // Should NOT have email
      expect(record.value.email).toBeUndefined();
  });

  test('Should NOT filter fields for ADMIN role', async () => {
       // 2. Mock Client (ADMIN role)
      const clientSocket = new MockWebSocket();
      const clientMock = {
          id: 'client-admin',
          socket: clientSocket as any,
          writer: createMockWriter(clientSocket) as any,
          isAuthenticated: true,
          subscriptions: new Set(),
          principal: { userId: 'a1', roles: ['ADMIN'] }
      };
      harness.connectionManager.getClients().set('client-admin', clientMock);

      // 3. Send QUERY_SUB
      const queryId = 'q2';
      await harness.handleMessage(clientMock, {
          type: 'QUERY_SUB',
          payload: {
              queryId,
              mapName: 'profiles',
              query: {}
          }
      });

      // 4. Verify Response
      expect(clientSocket.send).toHaveBeenCalled();
      const args = clientSocket.send.mock.calls[0][0];
      const response = deserialize(args) as any;

      expect(response.type).toBe('QUERY_RESP');
      const record = response.payload.results[0];
      expect(record.value).toEqual({ publicName: 'Alex', email: 'secret@test.com', internalId: 123 });
  });

  test('Should filter broadcast (SERVER_EVENT) per client', async () => {
      // Setup: User Client and Admin Client connected
      const userSocket = new MockWebSocket();
      const adminSocket = new MockWebSocket();
      const adminListenerSocket = new MockWebSocket();

      const userClient = {
          id: 'client-user-2',
          socket: userSocket as any,
          writer: createMockWriter(userSocket) as any,
          isAuthenticated: true,
          subscriptions: new Set(),
          principal: { userId: 'u2', roles: ['USER'] },
          lastActiveHlc: { millis: Date.now(), counter: 0, nodeId: 'test-node-sec' },
          lastPingReceived: Date.now()
      };
      const adminClient = {
          id: 'client-admin-2',
          socket: adminSocket as any,
          writer: createMockWriter(adminSocket) as any,
          isAuthenticated: true,
          subscriptions: new Set(),
          principal: { userId: 'a2', roles: ['ADMIN'] },
          lastActiveHlc: { millis: Date.now(), counter: 0, nodeId: 'test-node-sec' },
          lastPingReceived: Date.now()
      };
      const adminListener = {
          id: 'client-admin-listener',
          socket: adminListenerSocket as any,
          writer: createMockWriter(adminListenerSocket) as any,
          isAuthenticated: true,
          subscriptions: new Set(),
          principal: { userId: 'a3', roles: ['ADMIN'] },
          lastActiveHlc: { millis: Date.now(), counter: 0, nodeId: 'test-node-sec' },
          lastPingReceived: Date.now()
      };

      harness.connectionManager.getClients().set('client-user-2', userClient);
      harness.connectionManager.getClients().set('client-admin-2', adminClient);
      harness.connectionManager.getClients().set('client-admin-listener', adminListener);

      // IMPORTANT: Clients must subscribe to receive SERVER_EVENT (subscription-based routing)
      await harness.handleMessage(userClient, {
          type: 'QUERY_SUB',
          payload: { queryId: 'user-q', mapName: 'profiles', query: {} }
      });
      await harness.handleMessage(adminListener, {
          type: 'QUERY_SUB',
          payload: { queryId: 'admin-q', mapName: 'profiles', query: {} }
      });

      // Clear mock calls after subscriptions
      userSocket.send.mockClear();
      adminListenerSocket.send.mockClear();

      // Trigger update via ADMIN
      // This calls processLocalOp -> broadcast
      await harness.handleMessage(adminClient, {
          type: 'CLIENT_OP',
          payload: {
              opType: 'set',
              mapName: 'profiles',
              key: 'user2',
              record: createRecord({ publicName: 'Bob', email: 'bob@secret.com' })
          }
      });

      // Wait for broadcast to reach the USER client
      await pollUntil(
        () => userSocket.send.mock.calls.length > 0,
        { timeoutMs: 5000, intervalMs: 20, description: 'FLS-filtered broadcast to USER client' }
      );
      // Find the SERVER_EVENT message (not QUERY_UPDATE)
      const userMessages = userSocket.send.mock.calls.map((c: any[]) => {
          try { return deserialize(c[0]) as any; } catch { return null; }
      }).filter(Boolean);
      const userServerEvent = userMessages.find((m: any) => m.type === 'SERVER_EVENT');

      expect(userServerEvent).toBeDefined();
      expect(userServerEvent.payload.key).toBe('user2');
      expect(userServerEvent.payload.record.value).toEqual({ publicName: 'Bob' });
      expect(userServerEvent.payload.record.value.email).toBeUndefined();

      // Verify Broadcast to ADMIN LISTENER (full data)
      expect(adminListenerSocket.send).toHaveBeenCalled();
      const adminMessages = adminListenerSocket.send.mock.calls.map((c: any[]) => {
          try { return deserialize(c[0]) as any; } catch { return null; }
      }).filter(Boolean);
      const adminServerEvent = adminMessages.find((m: any) => m.type === 'SERVER_EVENT');

      expect(adminServerEvent).toBeDefined();
      expect(adminServerEvent.payload.record.value).toEqual({ publicName: 'Bob', email: 'bob@secret.com' });
  });
});
