import { QueryRegistry, Subscription } from '../QueryRegistry';
import { LWWRecord, LWWMap, deserialize } from '@topgunbuild/core';
import { WebSocket } from 'ws';

// Mock WebSocket
const createMockSocket = (): WebSocket => {
  return {
    readyState: 1,
    send: jest.fn(),
  } as unknown as WebSocket;
};

const createRecord = (value: any, timestamp: number = Date.now()): LWWRecord<any> => ({
  value,
  timestamp: { millis: timestamp, counter: 0, nodeId: 'test' }
});

describe('QueryRegistry', () => {
  let registry: QueryRegistry;
  let mockMap: any;
  let mapRecords: Map<string, LWWRecord<any>>;

  beforeEach(() => {
    registry = new QueryRegistry();
    mapRecords = new Map();
    
    mockMap = {
      allKeys: () => mapRecords.keys(),
      getRecord: (key: string) => mapRecords.get(key),
    };
  });

  test('should handle sliding window (limit)', () => {
    const socket = createMockSocket();
    const sub: Subscription = {
      id: 'sub1',
      clientId: 'c1',
      mapName: 'items',
      query: {
        sort: { score: 'desc' },
        limit: 3
      },
      socket,
      previousResultKeys: new Set(['A', 'B', 'C'])
    };

    // Initial State: A=100, B=90, C=80, D=70
    mapRecords.set('A', createRecord({ score: 100 }));
    mapRecords.set('B', createRecord({ score: 90 }));
    mapRecords.set('C', createRecord({ score: 80 }));
    mapRecords.set('D', createRecord({ score: 70 }));

    registry.register(sub);

    // Update D to 95. It should enter top 3 (A, D, B). C should fall out.
    const newD = createRecord({ score: 95 });
    mapRecords.set('D', newD);

    registry.processChange('items', mockMap as LWWMap<string, any>, 'D', newD);

    expect(socket.send).toHaveBeenCalledTimes(2);

    // C should be removed
    const call1 = deserialize((socket.send as jest.Mock).mock.calls[0][0] as Uint8Array) as any;
    // D should be updated/added
    const call2 = deserialize((socket.send as jest.Mock).mock.calls[1][0] as Uint8Array) as any;
    
    // The order of calls depends on iteration, but logically we expect both.
    // My implementation iterates 'removed' then 'added'.
    
    // Check removed
    const removedMsg = [call1, call2].find(m => m.payload.type === 'REMOVE');
    expect(removedMsg).toBeDefined();
    expect(removedMsg.payload.key).toBe('C');

    // Check updated
    const updatedMsg = [call1, call2].find(m => m.payload.key === 'D');
    expect(updatedMsg).toBeDefined();
    expect(updatedMsg.payload.type).toBe('UPDATE');
    expect(updatedMsg.payload.value.score).toBe(95);

    // Verify internal state
    expect(sub.previousResultKeys).toEqual(new Set(['A', 'B', 'D']));
  });

  test('should handle moving out of window', () => {
    const socket = createMockSocket();
    const sub: Subscription = {
      id: 'sub1',
      clientId: 'c1',
      mapName: 'items',
      query: {
        sort: { score: 'desc' },
        limit: 2
      },
      socket,
      previousResultKeys: new Set(['A', 'B'])
    };

    // Initial: A=100, B=90, C=80
    mapRecords.set('A', createRecord({ score: 100 }));
    mapRecords.set('B', createRecord({ score: 90 }));
    mapRecords.set('C', createRecord({ score: 80 }));

    registry.register(sub);

    // Update A to 70. New order: B(90), C(80), A(70). Top 2: B, C.
    // A leaves, C enters.
    const newA = createRecord({ score: 70 });
    mapRecords.set('A', newA);

    registry.processChange('items', mockMap as LWWMap<string, any>, 'A', newA);

    expect(socket.send).toHaveBeenCalledTimes(2);

    // A should be removed
    const call1 = deserialize((socket.send as jest.Mock).mock.calls[0][0] as Uint8Array) as any;
    const call2 = deserialize((socket.send as jest.Mock).mock.calls[1][0] as Uint8Array) as any;

    const removedMsg = [call1, call2].find(m => m.payload.type === 'REMOVE');
    expect(removedMsg).toBeDefined();
    expect(removedMsg.payload.key).toBe('A');

    const addedMsg = [call1, call2].find(m => m.payload.key === 'C');
    expect(addedMsg).toBeDefined();
    expect(addedMsg.payload.type).toBe('UPDATE');
  });
});
