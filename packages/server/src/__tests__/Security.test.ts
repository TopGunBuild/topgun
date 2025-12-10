import { ServerCoordinator } from '../ServerCoordinator';
import { LWWRecord, deserialize, PermissionPolicy } from '@topgunbuild/core';

// Mock WebSocket
class MockWebSocket {
    readyState = 1; // OPEN
    send = jest.fn();
    close = jest.fn();
    on = jest.fn();
}

describe('Field-Level Security (RBAC)', () => {
  let server: ServerCoordinator;
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
    server = new ServerCoordinator({
      port: 0,
      nodeId: 'test-node-sec',
      host: 'localhost',
      clusterPort: 0,
      peers: [],
      securityPolicies: policies
    });
    await server.ready();
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
          isAuthenticated: true,
          subscriptions: new Set(),
          principal: { userId: 'u1', roles: ['USER'] }
      };
      (server as any).clients.set('client-user', clientMock);

      // 3. Send QUERY_SUB
      const queryId = 'q1';
      await (server as any).handleMessage(clientMock, {
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
          isAuthenticated: true,
          subscriptions: new Set(),
          principal: { userId: 'a1', roles: ['ADMIN'] }
      };
      (server as any).clients.set('client-admin', clientMock);

      // 3. Send QUERY_SUB
      const queryId = 'q2';
      await (server as any).handleMessage(clientMock, {
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
          isAuthenticated: true,
          subscriptions: new Set(),
          principal: { userId: 'u2', roles: ['USER'] }
      };
      const adminClient = {
          id: 'client-admin-2',
          socket: adminSocket as any,
          isAuthenticated: true,
          subscriptions: new Set(),
          principal: { userId: 'a2', roles: ['ADMIN'] }
      };
      const adminListener = {
          id: 'client-admin-listener',
          socket: adminListenerSocket as any,
          isAuthenticated: true,
          subscriptions: new Set(),
          principal: { userId: 'a3', roles: ['ADMIN'] }
      };

      (server as any).clients.set('client-user-2', userClient);
      (server as any).clients.set('client-admin-2', adminClient);
      (server as any).clients.set('client-admin-listener', adminListener);

      // Trigger update via ADMIN
      // This calls processLocalOp -> broadcast
      await (server as any).handleMessage(adminClient, {
          type: 'CLIENT_OP',
          payload: {
              opType: 'set',
              mapName: 'profiles',
              key: 'user2',
              record: createRecord({ publicName: 'Bob', email: 'bob@secret.com' })
          }
      });

      // Verify Broadcast to USER
      expect(userSocket.send).toHaveBeenCalled();
      const userMsg = deserialize(userSocket.send.mock.calls[0][0]) as any;
      expect(userMsg.type).toBe('SERVER_EVENT');
      expect(userMsg.payload.key).toBe('user2');
      expect(userMsg.payload.record.value).toEqual({ publicName: 'Bob' });
      expect(userMsg.payload.record.value.email).toBeUndefined();

      // Verify Broadcast to ADMIN LISTENER
      expect(adminListenerSocket.send).toHaveBeenCalled();
      const adminMsg = deserialize(adminListenerSocket.send.mock.calls[0][0]) as any;
      expect(adminMsg.payload.record.value).toEqual({ publicName: 'Bob', email: 'bob@secret.com' });
  });
});
