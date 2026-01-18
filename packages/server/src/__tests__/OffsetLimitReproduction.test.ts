import { ServerCoordinator } from '../ServerCoordinator';
import { LWWRecord, deserialize, serialize, QueryCursor } from '@topgunbuild/core';

/**
 * Phase 14.1: This test was updated from offset-based to cursor-based pagination.
 * The original test demonstrated a double-apply bug with offset/limit.
 * Cursor-based pagination eliminates this class of bugs entirely.
 */
describe('Cursor-Based Pagination (formerly Offset/Limit Reproduction)', () => {
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

  test('Should correctly paginate using cursors', async () => {
    // 1. Seed Data: 10 items with scores 0-9
    const map = server.getMap('items') as any;
    for (let i = 0; i < 10; i++) {
      map.merge(`item-${i}`, createRecord({ score: i }));
    }

    // 2. Setup Client
    const clientSocket = {
      send: jest.fn(),
      readyState: 1, // OPEN
      close: jest.fn(),
    };

    const clientMock = {
      id: 'client-repro',
      socket: clientSocket as any,
      writer: createMockWriter(clientSocket) as any,
      isAuthenticated: true,
      subscriptions: new Set(),
      principal: { userId: 'test', roles: ['ADMIN'] }
    };

    // Inject client
    (server as any).connectionManager.getClients().set('client-repro', clientMock);

    // 3. First page: Get first 3 items (sorted by score asc)
    const queryId = 'q1';
    await (server as any).handleMessage(clientMock, {
      type: 'QUERY_SUB',
      payload: {
        queryId,
        mapName: 'items',
        query: {
          sort: { score: 'asc' },
          limit: 3
        }
      }
    });

    // 4. Verify first page results
    expect(clientSocket.send).toHaveBeenCalled();
    const page1Msg = deserialize(clientSocket.send.mock.calls[0][0] as Uint8Array) as any;
    expect(page1Msg.type).toBe('QUERY_RESP');

    const page1Results = page1Msg.payload.results;
    expect(page1Results).toHaveLength(3);
    expect(page1Results[0].value.score).toBe(0);
    expect(page1Results[1].value.score).toBe(1);
    expect(page1Results[2].value.score).toBe(2);

    // Verify cursor is returned for next page
    expect(page1Msg.payload.hasMore).toBe(true);
    expect(page1Msg.payload.nextCursor).toBeDefined();

    // 5. Second page: Use cursor to get next 3 items
    clientSocket.send.mockClear();
    const queryId2 = 'q2';
    await (server as any).handleMessage(clientMock, {
      type: 'QUERY_SUB',
      payload: {
        queryId: queryId2,
        mapName: 'items',
        query: {
          sort: { score: 'asc' },
          limit: 3,
          cursor: page1Msg.payload.nextCursor
        }
      }
    });

    // 6. Verify second page results (items with score 3, 4, 5)
    expect(clientSocket.send).toHaveBeenCalled();
    const page2Msg = deserialize(clientSocket.send.mock.calls[0][0] as Uint8Array) as any;
    expect(page2Msg.type).toBe('QUERY_RESP');

    const page2Results = page2Msg.payload.results;
    expect(page2Results).toHaveLength(3);
    expect(page2Results[0].value.score).toBe(3);
    expect(page2Results[1].value.score).toBe(4);
    expect(page2Results[2].value.score).toBe(5);
    expect(page2Msg.payload.hasMore).toBe(true);
  });
});
