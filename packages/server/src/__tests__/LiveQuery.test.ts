import { ServerCoordinator } from '../ServerCoordinator';
import { LWWRecord, deserialize, Predicates, serialize } from '@topgunbuild/core';

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

// Retry flaky tests up to 3 times
jest.retryTimes(3);

describe('Live Query Sliding Window Integration', () => {
  let server: ServerCoordinator;

  beforeAll(async () => {
    server = new ServerCoordinator({
      port: 0,
      nodeId: 'test-node',
      host: 'localhost',
      clusterPort: 0,
      peers: []
    });
    await server.ready();
  });

  afterAll(async () => {
    await server.shutdown();
  });

  const createRecord = (value: any): LWWRecord<any> => ({
    value,
    timestamp: { millis: Date.now(), counter: 0, nodeId: 'test-node' }
  });

  test('Should handle sliding window updates (REMOVE/UPDATE)', async () => {
    // 1. Seed Data: A=100, B=90, C=80
    const map = server.getMap('scores') as any;
    map.merge('A', createRecord({ score: 100 }));
    map.merge('B', createRecord({ score: 90 }));
    map.merge('C', createRecord({ score: 80 }));

    // 2. Setup Client & Subscribe (Top 2)
    const clientSocket = {
      send: jest.fn(),
      close: jest.fn(),
      readyState: 1 // OPEN
    };

    const clientMock = {
      id: 'client-1',
      socket: clientSocket as any,
      writer: createMockWriter(clientSocket) as any,
      isAuthenticated: true,
      subscriptions: new Set(),
      principal: { userId: 'test', roles: ['ADMIN'] }
    };

    // Inject client
    (server as any).connectionManager.getClients().set('client-1', clientMock);

    // Send SUBSCRIBE
    const queryId = 'q1';
    await (server as any).handleMessage(clientMock, {
      type: 'QUERY_SUB',
      payload: {
        queryId,
        mapName: 'scores',
        query: {
          sort: { score: 'desc' },
          limit: 2
        }
      }
    });

    // Expect Initial Results: A, B
    expect(clientSocket.send).toHaveBeenCalled();
    const initialMsg = deserialize(clientSocket.send.mock.calls[0][0] as Uint8Array) as any;
    expect(initialMsg.type).toBe('QUERY_RESP');
    expect(initialMsg.payload.results).toHaveLength(2);
    expect(initialMsg.payload.results.map((r: any) => r.key).sort()).toEqual(['A', 'B']);

    clientSocket.send.mockClear();

    // 3. Update D=95 (Should displace B? No, A=100, D=95, B=90. Top 2 is A, D. B is displaced)
    // Wait, previous top 2: A(100), B(90).
    // New top 2: A(100), D(95).
    // B(90) falls out.

    const op = {
      opType: 'set',
      mapName: 'scores',
      key: 'D',
      record: createRecord({ score: 95 })
    };

    // Simulate Client OP (or direct server op)
    await (server as any).handleMessage(clientMock, {
      type: 'CLIENT_OP',
      payload: op
    });

    // 4. Verify Events
    // Expect: REMOVE B, UPDATE D
    expect(clientSocket.send).toHaveBeenCalled();
    const msgs = clientSocket.send.mock.calls.map(c => deserialize(c[0] as Uint8Array) as any);

    const removeMsg = msgs.find(m => m.type === 'QUERY_UPDATE' && m.payload.type === 'REMOVE');
    const updateMsg = msgs.find(m => m.type === 'QUERY_UPDATE' && m.payload.type === 'UPDATE');

    expect(removeMsg).toBeDefined();
    expect(removeMsg.payload.key).toBe('B');

    expect(updateMsg).toBeDefined();
    expect(updateMsg.payload.key).toBe('D');
    expect(updateMsg.payload.value.score).toBe(95);
  });

  test('Should handle Predicate filtering', async () => {
    // 1. Seed Data
    const map = server.getMap('users') as any;
    map.merge('u1', createRecord({ age: 20, active: true }));
    map.merge('u2', createRecord({ age: 30, active: false }));
    map.merge('u3', createRecord({ age: 25, active: true }));

    // 2. Setup Client
    const clientSocket = {
      send: jest.fn(),
      close: jest.fn(),
      readyState: 1
    };
    const clientMock = {
      id: 'client-2',
      socket: clientSocket as any,
      writer: createMockWriter(clientSocket) as any,
      isAuthenticated: true,
      subscriptions: new Set(),
      principal: { userId: 'test2', roles: ['ADMIN'] }
    };
    (server as any).connectionManager.getClients().set('client-2', clientMock);

    // 3. Subscribe with Predicate (Active & Age > 22) -> Expect u3
    const queryId = 'q2';
    await (server as any).handleMessage(clientMock, {
      type: 'QUERY_SUB',
      payload: {
        queryId,
        mapName: 'users',
        query: {
          predicate: Predicates.and(
             Predicates.equal('active', true),
             Predicates.greaterThan('age', 22)
          )
        }
      }
    });

    // Expect Initial Result: u3
    // Note: Server sends QUERY_RESP
    expect(clientSocket.send).toHaveBeenCalled();
    // Decode last message if multiple? No, just 1 expected for SUB response
    const initialMsg = deserialize(clientSocket.send.mock.calls[0][0] as Uint8Array) as any;
    expect(initialMsg.type).toBe('QUERY_RESP');
    expect(initialMsg.payload.results).toHaveLength(1);
    expect(initialMsg.payload.results[0].key).toBe('u3');

    clientSocket.send.mockClear();

    // 4. Update u1 age to 23 -> Should enter result
    await (server as any).handleMessage(clientMock, {
      type: 'CLIENT_OP',
      payload: {
        opType: 'set',
        mapName: 'users',
        key: 'u1',
        record: createRecord({ age: 23, active: true })
      }
    });

    // Wait for async query update propagation
    await new Promise(r => setTimeout(r, 200));

    // Expect UPDATE for u1
    expect(clientSocket.send).toHaveBeenCalled();
    const updateMsg = deserialize(clientSocket.send.mock.calls[0][0] as Uint8Array) as any;
    expect(updateMsg.type).toBe('QUERY_UPDATE');
    expect(updateMsg.payload.type).toBe('UPDATE');
    expect(updateMsg.payload.key).toBe('u1');
  });
});
