import { ServerCoordinator } from '../ServerCoordinator';
import { LWWRecord, deserialize, ORMap, LWWMap, serialize } from '@topgunbuild/core';

describe('Sync Protocol Integration', () => {
  let server: ServerCoordinator;

  beforeAll(async () => {
    server = new ServerCoordinator({
      port: 0,
      nodeId: 'test-server',
      host: 'localhost',
      clusterPort: 0,
      peers: []
    });
    await server.ready();
  });

  afterAll(async () => {
    await server.shutdown();
  });

  const createRecord = (value: any, timestampMillis: number = Date.now()): LWWRecord<any> => ({
    value,
    timestamp: { millis: timestampMillis, counter: 0, nodeId: 'client-1' }
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

  test('Should handle OP_BATCH and send OP_ACK', async () => {
    const clientSocket = {
      send: jest.fn(),
      readyState: 1 // OPEN
    };

    const clientMock = {
      id: 'client-1',
      socket: clientSocket as any,
      writer: createMockWriter(clientSocket) as any,
      isAuthenticated: true,
      principal: { roles: ['ADMIN'] }, // Add principal with ADMIN role
      subscriptions: new Set()
    };

    // Inject client
    (server as any).connectionManager.getClients().set('client-1', clientMock);

    const ops = [
      {
        id: '100',
        mapName: 'todos',
        opType: 'PUT',
        key: 'todo:1',
        record: createRecord({ title: 'Buy Milk' })
      },
      {
        id: '101',
        mapName: 'todos',
        opType: 'PUT',
        key: 'todo:2',
        record: createRecord({ title: 'Walk Dog' })
      }
    ];

    // Simulate sending OP_BATCH (await async handleMessage)
    await (server as any).handleMessage(clientMock, {
      type: 'OP_BATCH',
      payload: { ops }
    });

    // Wait for async batch processing to complete
    await server.waitForPendingBatches();

    // 1. Check Server State
    const map = server.getMap('todos') as LWWMap<string, any>;
    expect(map.get('todo:1')).toEqual({ title: 'Buy Milk' });
    expect(map.get('todo:2')).toEqual({ title: 'Walk Dog' });

    // 2. Check ACK
    expect(clientSocket.send).toHaveBeenCalled();
    const calls = clientSocket.send.mock.calls.map(c => deserialize(c[0] as Uint8Array) as any);
    const ackMsg = calls.find(m => m.type === 'OP_ACK');

    expect(ackMsg).toBeDefined();
    expect(ackMsg.payload.lastId).toBe('101');
  });

  test('Should be idempotent (handle duplicate batches)', async () => {
    const clientSocket = {
      send: jest.fn(),
      readyState: 1 // OPEN
    };

    const clientMock = {
      id: 'client-retry',
      socket: clientSocket as any,
      writer: createMockWriter(clientSocket) as any,
      isAuthenticated: true,
      principal: { roles: ['ADMIN'] }, // Add principal with ADMIN role
      subscriptions: new Set()
    };

    (server as any).connectionManager.getClients().set('client-retry', clientMock);

    const ts = Date.now();
    const ops = [
      {
        id: '200',
        mapName: 'notes',
        opType: 'PUT',
        key: 'note:1',
        record: createRecord({ text: 'Original' }, ts)
      }
    ];

    // First attempt
    await (server as any).handleMessage(clientMock, {
      type: 'OP_BATCH',
      payload: { ops }
    });

    // Wait for async batch processing to complete
    await server.waitForPendingBatches();

    const map = server.getMap('notes') as LWWMap<string, any>;
    expect(map.get('note:1')).toEqual({ text: 'Original' });

    // Second attempt (simulate retry)
    // Sending same batch
    clientSocket.send.mockClear();
    await (server as any).handleMessage(clientMock, {
      type: 'OP_BATCH',
      payload: { ops }
    });

    // Check ACK
    const calls = clientSocket.send.mock.calls.map(c => deserialize(c[0] as Uint8Array) as any);
    const ackMsg = calls.find(m => m.type === 'OP_ACK');
    expect(ackMsg).toBeDefined();
    expect(ackMsg.payload.lastId).toBe('200');

    // State should remain same
    expect(map.get('note:1')).toEqual({ text: 'Original' });
  });

  test('Should handle OR_ADD and OR_REMOVE for ORMap', async () => {
    const clientSocket = {
      send: jest.fn(),
      readyState: 1
    };

    const clientMock = {
      id: 'client-or',
      socket: clientSocket as any,
      isAuthenticated: true,
      principal: { roles: ['ADMIN'] }, // Add principal with ADMIN role
      subscriptions: new Set()
    };

    (server as any).connectionManager.getClients().set('client-or', clientMock);

    const ts = Date.now();
    // Use a distinct map name for ORMap
    const mapName = 'shared-list';

    // 1. OR_ADD
    const orRecord = {
        value: 'Item 1',
        timestamp: { millis: ts, counter: 0, nodeId: 'client-or' },
        tag: 'tag-1'
    };

    const addOp = {
        id: '300',
        mapName,
        opType: 'OR_ADD',
        key: 'list:1',
        orRecord
    };

    await (server as any).handleMessage(clientMock, {
        type: 'CLIENT_OP',
        payload: addOp
    });

    // Check Server State
    const map = server.getMap(mapName, 'OR') as ORMap<string, any>; // Should be ORMap

    // ORMap returns array of values
    expect(Array.isArray(map.get('list:1'))).toBe(true);
    expect(map.get('list:1')).toEqual(['Item 1']);

    // 2. OR_REMOVE
    const removeOp = {
        id: '301',
        mapName,
        opType: 'OR_REMOVE',
        key: 'list:1',
        orTag: 'tag-1'
    };

    await (server as any).handleMessage(clientMock, {
        type: 'CLIENT_OP',
        payload: removeOp
    });

    expect(map.get('list:1')).toEqual([]);
  });
});
