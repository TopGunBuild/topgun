import { ServerCoordinator } from '../ServerCoordinator';
import { LWWRecord, deserialize } from '@topgunbuild/core';

describe('Offset/Limit Reproduction', () => {
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

  test('Should correctly apply offset and limit only once', async () => {
    // 1. Seed Data: 10 items with scores 0-9
    const map = server.getMap('items') as any;
    for (let i = 0; i < 10; i++) {
      map.merge(`item-${i}`, createRecord({ score: i }));
    }

    // 2. Setup Client
    const clientSocket = {
      send: jest.fn(),
      readyState: 1 // OPEN
    };

    const clientMock = {
      id: 'client-repro',
      socket: clientSocket as any,
      isAuthenticated: true,
      subscriptions: new Set(),
      principal: { userId: 'test', roles: ['ADMIN'] }
    };

    // Inject client
    (server as any).clients.set('client-repro', clientMock);

    // 3. Send SUBSCRIBE with offset: 3, limit: 3
    // Expected: items with score 3, 4, 5 (assuming sorted by score asc? or just insertion order if no sort?)
    // Let's sort by score to be deterministic.
    const queryId = 'q1';
    await (server as any).handleMessage(clientMock, {
      type: 'QUERY_SUB',
      payload: {
        queryId,
        mapName: 'items',
        query: {
          sort: { score: 'asc' },
          offset: 3,
          limit: 3
        }
      }
    });

    // 4. Verify Results
    expect(clientSocket.send).toHaveBeenCalled();
    const initialMsg = deserialize(clientSocket.send.mock.calls[0][0] as Uint8Array) as any;
    expect(initialMsg.type).toBe('QUERY_RESP');
    
    const results = initialMsg.payload.results;
    // With the bug:
    // Local: slice(3, 6) -> returns items 3, 4, 5
    // Finalize: slice(3, 6) on [3, 4, 5] -> returns [] (empty)
    
    // We expect 3 items
    expect(results).toHaveLength(3);
    expect(results[0].value.score).toBe(3);
    expect(results[1].value.score).toBe(4);
    expect(results[2].value.score).toBe(5);
  });
});
