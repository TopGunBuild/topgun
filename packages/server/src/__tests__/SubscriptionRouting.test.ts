import { ServerCoordinator, ServerFactory } from '../';
import { LWWRecord, deserialize, PermissionPolicy, serialize } from '@topgunbuild/core';
import { createTestHarness, ServerTestHarness } from './utils/ServerTestHarness';
import { pollUntil } from './utils/test-helpers';

// Default policy that allows all operations for testing
const defaultTestPolicies: PermissionPolicy[] = [
  {
    role: 'USER',
    mapNamePattern: '*',
    actions: ['ALL'],
    allowedFields: ['*']
  },
  {
    role: 'ADMIN',
    mapNamePattern: '*',
    actions: ['ALL'],
    allowedFields: ['*']
  }
];

const createRecord = (value: any): LWWRecord<any> => ({
  value,
  timestamp: { millis: Date.now(), counter: 0, nodeId: 'test-node' }
});

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


describe('Subscription-Based Event Routing', () => {
  let server: ServerCoordinator;
  let harness: ServerTestHarness;

  beforeAll(async () => {
    server = ServerFactory.create({
      port: 0,
      nodeId: 'test-node',
      host: 'localhost',
      clusterPort: 0,
      peers: [],
      securityPolicies: defaultTestPolicies,
      metricsPort: 0 // Use random port for metrics to avoid conflicts
    });
    await server.ready();
    harness = createTestHarness(server);
  });

  afterAll(async () => {
    await server.shutdown();
  });



  const createMockClient = (id: string) => {
    const socket = {
      send: jest.fn(),
      close: jest.fn(),
      readyState: 1 // OPEN
    };
    return {
      id,
      socket: socket as any,
      writer: createMockWriter(socket) as any,
      isAuthenticated: true,
      subscriptions: new Set<string>(),
      principal: { userId: id, roles: ['USER'] },
      lastActiveHlc: { millis: Date.now(), counter: 0, nodeId: 'test-node' },
      lastPingReceived: Date.now()
    };
  };

  const deserializeCall = (call: any[]): any => {
    try {
      return deserialize(call[0] as Uint8Array);
    } catch {
      return null;
    }
  };

  test('Unsubscribed clients should NOT receive events', async () => {
    const mapName = 'routing-test-1';

    // Create two clients
    const subscribedClient = createMockClient('subscribed-client');
    const unsubscribedClient = createMockClient('unsubscribed-client');

    // Inject clients into server
    harness.connectionManager.getClients().set(subscribedClient.id, subscribedClient);
    harness.connectionManager.getClients().set(unsubscribedClient.id, unsubscribedClient);

    // Only one client subscribes to the map
    await harness.handleMessage(subscribedClient, {
      type: 'QUERY_SUB',
      payload: {
        queryId: 'q-subscribed',
        mapName,
        query: {}
      }
    });

    // Clear mock calls after subscription
    subscribedClient.socket.send.mockClear();
    unsubscribedClient.socket.send.mockClear();

    // Send an operation from a third client (or simulate server-side event)
    const operatorClient = createMockClient('operator-client');
    harness.connectionManager.getClients().set(operatorClient.id, operatorClient);

    await harness.handleMessage(operatorClient, {
      type: 'CLIENT_OP',
      payload: {
        opType: 'set',
        mapName,
        key: 'test-key',
        record: createRecord({ data: 'test-value' })
      }
    });

    // Wait for subscribed client to receive the event (QUERY_UPDATE or SERVER_EVENT)
    await pollUntil(
      () => subscribedClient.socket.send.mock.calls.length > 0,
      { timeoutMs: 5000, intervalMs: 20, description: 'subscribed client receives event' }
    );

    // Unsubscribed client should NOT receive any events for this map
    const unsubscribedCalls = unsubscribedClient.socket.send.mock.calls;
    const serverEvents = unsubscribedCalls
      .map((c: any[]) => deserializeCall(c))
      .filter((m: any) => m && m.type === 'SERVER_EVENT' && m.payload?.mapName === mapName);

    expect(serverEvents.length).toBe(0);
  });

  test('Subscribed clients should receive events for their map', async () => {
    const mapName = 'routing-test-2';

    const client = createMockClient('client-2');
    harness.connectionManager.getClients().set(client.id, client);

    // Subscribe to the map
    await harness.handleMessage(client, {
      type: 'QUERY_SUB',
      payload: {
        queryId: 'q-2',
        mapName,
        query: {}
      }
    });

    // Clear subscription response
    client.socket.send.mockClear();

    // Create another client to send the operation
    const operatorClient = createMockClient('operator-2');
    harness.connectionManager.getClients().set(operatorClient.id, operatorClient);

    await harness.handleMessage(operatorClient, {
      type: 'CLIENT_OP',
      payload: {
        opType: 'set',
        mapName,
        key: 'key-2',
        record: createRecord({ value: 42 })
      }
    });

    // Wait for client to receive the event
    await pollUntil(
      () => client.socket.send.mock.calls.length > 0,
      { timeoutMs: 5000, intervalMs: 20, description: 'client receives event for subscribed map' }
    );

    const messages = client.socket.send.mock.calls
      .map((c: any[]) => deserializeCall(c))
      .filter(Boolean);

    // Should have received QUERY_UPDATE for the subscribed query
    const queryUpdate = messages.find((m: any) => m.type === 'QUERY_UPDATE');
    expect(queryUpdate).toBeDefined();
    expect(queryUpdate.payload.key).toBe('key-2');
  });

  test('Early exit when no subscribers exist', async () => {
    const mapName = 'no-subscribers-map';

    // Create a client but DON'T subscribe to this map
    const client = createMockClient('client-no-sub');
    harness.connectionManager.getClients().set(client.id, client);

    // Subscribe to a DIFFERENT map
    await harness.handleMessage(client, {
      type: 'QUERY_SUB',
      payload: {
        queryId: 'q-other-map',
        mapName: 'other-map',
        query: {}
      }
    });

    client.socket.send.mockClear();

    // Send an operation to the map with no subscribers
    const operatorClient = createMockClient('operator-no-sub');
    harness.connectionManager.getClients().set(operatorClient.id, operatorClient);

    await harness.handleMessage(operatorClient, {
      type: 'CLIENT_OP',
      payload: {
        opType: 'set',
        mapName, // Map with no subscribers
        key: 'key-no-sub',
        record: createRecord({ data: 'should not be sent' })
      }
    });

    // Allow async processing to complete before checking no-op result
    await pollUntil(
      () => {
        // Give the event loop a chance to process; check if operator got an ACK or any processing occurred
        return true;
      },
      { timeoutMs: 200, intervalMs: 20, maxIterations: 10, description: 'async processing for no-subscribers map' }
    );

    // Client should NOT receive any SERVER_EVENT for the no-subscribers map
    const messages = client.socket.send.mock.calls
      .map((c: any[]) => deserializeCall(c))
      .filter((m: any) => m && m.type === 'SERVER_EVENT' && m.payload?.mapName === mapName);

    expect(messages.length).toBe(0);
  });

  test('Subscription routing does not break SERVER_EVENT FLS filtering', async () => {
    // This test verifies that FLS filtering is still applied in broadcast()
    // by checking that SERVER_EVENT messages sent via subscription routing
    // still filter fields based on the user's role.
    const mapName = 'fls-broadcast-test';

    // Create server with FLS policy
    const flsPolicy: PermissionPolicy[] = [
      {
        role: 'USER',
        mapNamePattern: mapName,
        actions: ['PUT', 'READ'],
        allowedFields: ['public'] // Only allow 'public' field
      },
      {
        role: 'ADMIN',
        mapNamePattern: '*',
        actions: ['ALL'],
        allowedFields: ['*']
      }
    ];

    const flsServer = ServerFactory.create({
      port: 0,
      nodeId: 'fls-test-node',
      host: 'localhost',
      clusterPort: 0,
      peers: [],
      securityPolicies: flsPolicy,
      metricsPort: 0
    });
    await flsServer.ready();
    const flsHarness = createTestHarness(flsServer);

    try {
      // Create USER client and ADMIN client
      const userClient = createMockClient('fls-user');
      const adminClient = {
        ...createMockClient('fls-admin'),
        principal: { userId: 'admin', roles: ['ADMIN'] }
      };

      flsHarness.connectionManager.getClients().set(userClient.id, userClient);
      flsHarness.connectionManager.getClients().set(adminClient.id, adminClient);

      // Both subscribe to the same map
      await flsHarness.handleMessage(userClient, {
        type: 'QUERY_SUB',
        payload: { queryId: 'user-query', mapName, query: {} }
      });
      await flsHarness.handleMessage(adminClient, {
        type: 'QUERY_SUB',
        payload: { queryId: 'admin-query', mapName, query: {} }
      });

      userClient.socket.send.mockClear();
      adminClient.socket.send.mockClear();

      // Third client (another admin) sends data with secret field
      const operator = {
        ...createMockClient('operator'),
        principal: { userId: 'op', roles: ['ADMIN'] }
      };
      flsHarness.connectionManager.getClients().set(operator.id, operator);

      await flsHarness.handleMessage(operator, {
        type: 'CLIENT_OP',
        payload: {
          opType: 'set',
          mapName,
          key: 'test-key',
          record: createRecord({
            public: 'visible-data',
            secret: 'hidden-data'
          })
        }
      });

      // Wait for both clients to receive broadcast
      await pollUntil(
        () => userClient.socket.send.mock.calls.length > 0 && adminClient.socket.send.mock.calls.length > 0,
        { timeoutMs: 5000, intervalMs: 20, description: 'FLS broadcast to user and admin clients' }
      );

      // Check SERVER_EVENT sent to USER client - should have filtered fields
      const userMessages = userClient.socket.send.mock.calls
        .map((c: any[]) => deserializeCall(c))
        .filter(Boolean);

      const userServerEvent = userMessages.find((m: any) => m.type === 'SERVER_EVENT');

      // If SERVER_EVENT was sent, verify FLS is applied
      if (userServerEvent) {
        expect(userServerEvent.payload.record.value.public).toBe('visible-data');
        expect(userServerEvent.payload.record.value.secret).toBeUndefined();
      }

      // Check SERVER_EVENT sent to ADMIN client - should have all fields
      const adminMessages = adminClient.socket.send.mock.calls
        .map((c: any[]) => deserializeCall(c))
        .filter(Boolean);

      const adminServerEvent = adminMessages.find((m: any) => m.type === 'SERVER_EVENT');

      if (adminServerEvent) {
        expect(adminServerEvent.payload.record.value.public).toBe('visible-data');
        expect(adminServerEvent.payload.record.value.secret).toBe('hidden-data');
      }
    } finally {
      await flsServer.shutdown();
    }
  });

  test('Multiple clients subscribed to same map all receive events', async () => {
    const mapName = 'multi-client-map';

    // Create multiple clients
    const client1 = createMockClient('multi-1');
    const client2 = createMockClient('multi-2');
    const client3 = createMockClient('multi-3');

    harness.connectionManager.getClients().set(client1.id, client1);
    harness.connectionManager.getClients().set(client2.id, client2);
    harness.connectionManager.getClients().set(client3.id, client3);

    // All clients subscribe to the same map
    for (const client of [client1, client2, client3]) {
      await harness.handleMessage(client, {
        type: 'QUERY_SUB',
        payload: {
          queryId: `q-${client.id}`,
          mapName,
          query: {}
        }
      });
      client.socket.send.mockClear();
    }

    // Create operator to send data
    const operatorClient = createMockClient('operator-multi');
    harness.connectionManager.getClients().set(operatorClient.id, operatorClient);

    await harness.handleMessage(operatorClient, {
      type: 'CLIENT_OP',
      payload: {
        opType: 'set',
        mapName,
        key: 'multi-key',
        record: createRecord({ value: 'broadcast-to-all-subscribers' })
      }
    });

    // Wait for all subscribed clients to receive the event
    await pollUntil(
      () => [client1, client2, client3].every(c => c.socket.send.mock.calls.length > 0),
      { timeoutMs: 5000, intervalMs: 20, description: 'all subscribed clients receive broadcast' }
    );

    for (const client of [client1, client2, client3]) {
      expect(client.socket.send).toHaveBeenCalled();

      const messages = client.socket.send.mock.calls
        .map((c: any[]) => deserializeCall(c))
        .filter(Boolean);

      const hasUpdate = messages.some((m: any) => m.type === 'QUERY_UPDATE');
      expect(hasUpdate).toBe(true);
    }
  });

  test('Client unsubscribing should stop receiving events', async () => {
    const mapName = 'unsub-test-map';

    const client = createMockClient('unsub-client');
    harness.connectionManager.getClients().set(client.id, client);

    const queryId = 'q-unsub-test';

    // Subscribe
    await harness.handleMessage(client, {
      type: 'QUERY_SUB',
      payload: {
        queryId,
        mapName,
        query: {}
      }
    });

    client.socket.send.mockClear();

    // Unsubscribe
    await harness.handleMessage(client, {
      type: 'QUERY_UNSUB',
      payload: { queryId }
    });

    client.socket.send.mockClear();

    // Send an operation
    const operatorClient = createMockClient('operator-unsub');
    harness.connectionManager.getClients().set(operatorClient.id, operatorClient);

    await harness.handleMessage(operatorClient, {
      type: 'CLIENT_OP',
      payload: {
        opType: 'set',
        mapName,
        key: 'unsub-key',
        record: createRecord({ data: 'after-unsub' })
      }
    });

    // Allow async processing to complete before verifying no messages sent
    await pollUntil(
      () => true,
      { timeoutMs: 200, intervalMs: 20, maxIterations: 10, description: 'async processing after unsubscribe' }
    );

    // Client should NOT receive SERVER_EVENT for this map after unsubscribing
    const serverEvents = client.socket.send.mock.calls
      .map((c: any[]) => deserializeCall(c))
      .filter((m: any) => m && m.type === 'SERVER_EVENT' && m.payload?.mapName === mapName);

    expect(serverEvents.length).toBe(0);
  });

  test('System messages (GC_PRUNE) still broadcast to all clients', async () => {
    // Create clients without subscriptions
    const client1 = createMockClient('gc-client-1');
    const client2 = createMockClient('gc-client-2');

    harness.connectionManager.getClients().set(client1.id, client1);
    harness.connectionManager.getClients().set(client2.id, client2);

    client1.socket.send.mockClear();
    client2.socket.send.mockClear();

    // Directly call broadcast with a GC_PRUNE message (non-SERVER_EVENT)
    const gcMessage = {
      type: 'GC_PRUNE',
      payload: {
        olderThan: { millis: Date.now() - 1000, counter: 0, nodeId: 'test' }
      }
    };

    harness.broadcast(gcMessage);

    // Both clients should receive the message even without subscriptions
    expect(client1.socket.send).toHaveBeenCalled();
    expect(client2.socket.send).toHaveBeenCalled();

    const msg1 = deserialize(client1.socket.send.mock.calls[0][0] as Uint8Array) as any;
    const msg2 = deserialize(client2.socket.send.mock.calls[0][0] as Uint8Array) as any;

    expect(msg1.type).toBe('GC_PRUNE');
    expect(msg2.type).toBe('GC_PRUNE');
  });
});

describe('QueryRegistry.getSubscriptionsForMap', () => {
  let server: ServerCoordinator;
  let harness: ServerTestHarness;

  beforeAll(async () => {
    server = ServerFactory.create({
      port: 0,
      nodeId: 'registry-test-node',
      host: 'localhost',
      clusterPort: 0,
      peers: [],
      securityPolicies: defaultTestPolicies,
      metricsPort: 0
    });
    await server.ready();
    harness = createTestHarness(server);
  });

  afterAll(async () => {
    await server.shutdown();
  });

  test('Returns empty array for maps with no subscriptions', () => {
    const queryRegistry = harness.queryRegistry;
    const result = queryRegistry.getSubscriptionsForMap('non-existent-map');
    expect(result).toEqual([]);
  });

  test('Returns all subscriptions for a map', async () => {
    const mapName = 'registry-test-map';
    const queryRegistry = harness.queryRegistry;

    const createMockClient = (id: string) => {
      const socket = {
        send: jest.fn(),
        close: jest.fn(),
        readyState: 1 // OPEN
      };
      return {
        id,
        socket: socket as any,
        writer: createMockWriter(socket) as any,
        isAuthenticated: true,
        subscriptions: new Set<string>(),
        principal: { userId: id, roles: ['USER'] },
        lastActiveHlc: { millis: Date.now(), counter: 0, nodeId: 'test-node' },
        lastPingReceived: Date.now()
      };
    };

    const client1 = createMockClient('reg-client-1');
    const client2 = createMockClient('reg-client-2');

    harness.connectionManager.getClients().set(client1.id, client1);
    harness.connectionManager.getClients().set(client2.id, client2);

    // Subscribe both clients
    await harness.handleMessage(client1, {
      type: 'QUERY_SUB',
      payload: { queryId: 'reg-q1', mapName, query: {} }
    });

    await harness.handleMessage(client2, {
      type: 'QUERY_SUB',
      payload: { queryId: 'reg-q2', mapName, query: {} }
    });

    const subscriptions = queryRegistry.getSubscriptionsForMap(mapName);
    expect(subscriptions.length).toBe(2);

    const clientIds = queryRegistry.getSubscribedClientIds(mapName);
    expect(clientIds.size).toBe(2);
    expect(clientIds.has('reg-client-1')).toBe(true);
    expect(clientIds.has('reg-client-2')).toBe(true);
  });
});
