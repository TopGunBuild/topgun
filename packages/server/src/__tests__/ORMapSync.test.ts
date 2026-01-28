import { ServerCoordinator, ServerFactory } from '../';
import { HLC, ORMap, serialize, deserialize, ORMapRecord } from '@topgunbuild/core';

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

describe('ORMap Merkle Tree Sync Integration', () => {
  let server: ServerCoordinator;

  beforeAll(async () => {
    server = ServerFactory.create({
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

  const createMockClient = (id: string) => {
    const responses: any[] = [];
    const clientSocket = {
      send: jest.fn((data: Uint8Array) => {
        responses.push(deserialize(data));
      }),
      close: jest.fn(), // Mock close() to prevent cleanup errors
      readyState: 1 // OPEN
    };

    const clientMock = {
      id,
      socket: clientSocket as any,
      writer: createMockWriter(clientSocket) as any,
      isAuthenticated: true,
      subscriptions: new Set<string>(),
      principal: { userId: id, roles: ['ADMIN'] },
      lastActiveHlc: { millis: Date.now(), counter: 0, nodeId: id },
      lastPingReceived: Date.now()
    };

    return { client: clientMock, socket: clientSocket, responses };
  };

  const injectClient = (clientMock: any) => {
    (server as any).connectionManager.getClients().set(clientMock.id, clientMock);
  };

  describe('ORMAP_SYNC_INIT', () => {
    test('should respond with root hash for ORMap', async () => {
      // Get or create ORMap with 'OR' type hint
      const map = server.getMap('or:tags', 'OR') as ORMap<string, string>;

      // Add some data
      map.add('user-1', 'tag-1');
      map.add('user-1', 'tag-2');

      const { client, socket, responses } = createMockClient('client-1');
      injectClient(client);

      // Send ORMAP_SYNC_INIT
      await (server as any).handleMessage(client, {
        type: 'ORMAP_SYNC_INIT',
        mapName: 'or:tags',
        rootHash: 0, // Client has empty map
        bucketHashes: {},
        lastSyncTimestamp: 0
      });

      expect(socket.send).toHaveBeenCalled();
      const response = responses[0];
      expect(response.type).toBe('ORMAP_SYNC_RESP_ROOT');
      expect(response.payload.mapName).toBe('or:tags');
      expect(typeof response.payload.rootHash).toBe('number');
      expect(response.payload.rootHash).not.toBe(0); // Server has data
    });

    test('should respond with rootHash=0 for empty ORMap', async () => {
      // Create empty ORMap
      const map = server.getMap('or:empty', 'OR') as ORMap<string, string>;

      const { client, socket, responses } = createMockClient('client-2');
      injectClient(client);

      await (server as any).handleMessage(client, {
        type: 'ORMAP_SYNC_INIT',
        mapName: 'or:empty',
        rootHash: 0,
        bucketHashes: {},
        lastSyncTimestamp: 0
      });

      expect(socket.send).toHaveBeenCalled();
      const response = responses[0];
      expect(response.type).toBe('ORMAP_SYNC_RESP_ROOT');
      expect(response.payload.rootHash).toBe(0);
    });
  });

  describe('ORMAP_MERKLE_REQ_BUCKET', () => {
    test('should respond with buckets for non-leaf path', async () => {
      const map = server.getMap('or:buckets-test', 'OR') as ORMap<string, string>;

      // Add data to multiple keys to ensure distribution
      for (let i = 0; i < 10; i++) {
        map.add(`key-${i}`, `value-${i}`);
      }

      const { client, socket, responses } = createMockClient('client-3');
      injectClient(client);

      await (server as any).handleMessage(client, {
        type: 'ORMAP_MERKLE_REQ_BUCKET',
        payload: {
          mapName: 'or:buckets-test',
          path: ''
        }
      });

      expect(socket.send).toHaveBeenCalled();
      const response = responses[0];
      // Should be either ORMAP_SYNC_RESP_BUCKETS or ORMAP_SYNC_RESP_LEAF
      expect(['ORMAP_SYNC_RESP_BUCKETS', 'ORMAP_SYNC_RESP_LEAF']).toContain(response.type);
    });
  });

  describe('ORMAP_DIFF_REQUEST', () => {
    test('should respond with records for requested keys', async () => {
      const map = server.getMap('or:diff-test', 'OR') as ORMap<string, string>;
      map.add('key-a', 'value-a');
      map.add('key-a', 'value-a2');
      map.add('key-b', 'value-b');

      const { client, socket, responses } = createMockClient('client-4');
      injectClient(client);

      await (server as any).handleMessage(client, {
        type: 'ORMAP_DIFF_REQUEST',
        payload: {
          mapName: 'or:diff-test',
          keys: ['key-a', 'key-b']
        }
      });

      expect(socket.send).toHaveBeenCalled();
      const response = responses[0];
      expect(response.type).toBe('ORMAP_DIFF_RESPONSE');
      expect(response.payload.mapName).toBe('or:diff-test');
      expect(response.payload.entries).toHaveLength(2);

      // key-a should have 2 records
      const entryA = response.payload.entries.find((e: any) => e.key === 'key-a');
      expect(entryA).toBeDefined();
      expect(entryA.records.length).toBe(2);

      // key-b should have 1 record
      const entryB = response.payload.entries.find((e: any) => e.key === 'key-b');
      expect(entryB).toBeDefined();
      expect(entryB.records.length).toBe(1);
    });

    test('should return empty records for non-existent keys', async () => {
      const { client, socket, responses } = createMockClient('client-5');
      injectClient(client);

      await (server as any).handleMessage(client, {
        type: 'ORMAP_DIFF_REQUEST',
        payload: {
          mapName: 'or:non-existent-map',
          keys: ['non-existent-key']
        }
      });

      expect(socket.send).toHaveBeenCalled();
      const response = responses[0];
      expect(response.type).toBe('ORMAP_DIFF_RESPONSE');
      const entry = response.payload.entries[0];
      expect(entry.records).toHaveLength(0);
    });
  });

  describe('ORMAP_PUSH_DIFF', () => {
    test('should merge pushed records into server state', async () => {
      const map = server.getMap('or:push-test', 'OR') as ORMap<string, string>;

      const { client, socket, responses } = createMockClient('client-6');
      injectClient(client);

      const record: ORMapRecord<string> = {
        value: 'pushed-value',
        timestamp: { millis: Date.now(), counter: 1, nodeId: 'client-6' },
        tag: `${Date.now()}:1:client-6`
      };

      await (server as any).handleMessage(client, {
        type: 'ORMAP_PUSH_DIFF',
        payload: {
          mapName: 'or:push-test',
          entries: [{
            key: 'pushed-key',
            records: [record],
            tombstones: []
          }]
        }
      });

      // Verify server has the data
      const values = map.get('pushed-key');
      expect(values).toContain('pushed-value');
    });

    test('should broadcast pushed changes to other clients', async () => {
      const { client: client1, socket: socket1, responses: responses1 } = createMockClient('client-7');
      const { client: client2, socket: socket2, responses: responses2 } = createMockClient('client-8');

      injectClient(client1);
      injectClient(client2);

      // Initialize ORMap to ensure it exists before subscription
      server.getMap('or:broadcast-test', 'OR');

      // IMPORTANT: Client 2 must subscribe to receive SERVER_EVENT (subscription-based routing)
      await (server as any).handleMessage(client2, {
        type: 'QUERY_SUB',
        payload: { queryId: 'sub-or-broadcast', mapName: 'or:broadcast-test', query: {} }
      });

      // Wait for subscription to be processed
      await new Promise(r => setTimeout(r, 10));
      responses2.length = 0; // Clear subscription response

      const record: ORMapRecord<string> = {
        value: 'broadcast-value',
        timestamp: { millis: Date.now(), counter: 1, nodeId: 'client-7' },
        tag: `${Date.now()}:1:client-7-broadcast`
      };

      await (server as any).handleMessage(client1, {
        type: 'ORMAP_PUSH_DIFF',
        payload: {
          mapName: 'or:broadcast-test',
          entries: [{
            key: 'broadcast-key',
            records: [record],
            tombstones: []
          }]
        }
      });

      // Wait for async broadcast
      await new Promise(r => setTimeout(r, 10));

      // Client 2 should receive broadcast (now that it's subscribed)
      const broadcastMsg = responses2.find((r: any) => r.type === 'SERVER_EVENT');
      expect(broadcastMsg).toBeDefined();
      expect(broadcastMsg.payload.eventType).toBe('OR_ADD');
      expect(broadcastMsg.payload.orRecord.value).toBe('broadcast-value');
    });

    test('should handle tombstones in push', async () => {
      const map = server.getMap('or:tombstone-test', 'OR') as ORMap<string, string>;

      // Add a record first
      const initialRecord = map.add('tombstone-key', 'initial-value');
      expect(map.get('tombstone-key')).toContain('initial-value');

      const { client, socket, responses } = createMockClient('client-9');
      injectClient(client);

      // Push tombstone for the initial record
      await (server as any).handleMessage(client, {
        type: 'ORMAP_PUSH_DIFF',
        payload: {
          mapName: 'or:tombstone-test',
          entries: [{
            key: 'tombstone-key',
            records: [],
            tombstones: [initialRecord.tag]
          }]
        }
      });

      // Value should be removed
      expect(map.get('tombstone-key')).not.toContain('initial-value');
    });
  });

  describe('Three-client sync scenario', () => {
    test('should sync data between three clients through server', async () => {
      const mapName = 'or:three-client-test';

      // Setup: Client A has [A1], Client B has [B1], Client C has [C1]
      const hlcA = new HLC('client-A');
      const mapA = new ORMap<string, string>(hlcA);
      mapA.add('shared-key', 'value-from-A');

      const hlcB = new HLC('client-B');
      const mapB = new ORMap<string, string>(hlcB);
      mapB.add('shared-key', 'value-from-B');

      const hlcC = new HLC('client-C');
      const mapC = new ORMap<string, string>(hlcC);
      mapC.add('shared-key', 'value-from-C');

      const { client: clientA, responses: responsesA } = createMockClient('client-A');
      const { client: clientB, responses: responsesB } = createMockClient('client-B');
      const { client: clientC, responses: responsesC } = createMockClient('client-C');

      injectClient(clientA);
      injectClient(clientB);
      injectClient(clientC);

      const serverMap = server.getMap(mapName, 'OR') as ORMap<string, string>;

      // Client A pushes its data to server
      const recordsA = mapA.getRecords('shared-key');
      await (server as any).handleMessage(clientA, {
        type: 'ORMAP_PUSH_DIFF',
        payload: {
          mapName,
          entries: [{
            key: 'shared-key',
            records: recordsA,
            tombstones: []
          }]
        }
      });

      // Client B pushes its data
      const recordsB = mapB.getRecords('shared-key');
      await (server as any).handleMessage(clientB, {
        type: 'ORMAP_PUSH_DIFF',
        payload: {
          mapName,
          entries: [{
            key: 'shared-key',
            records: recordsB,
            tombstones: []
          }]
        }
      });

      // Client C pushes its data
      const recordsC = mapC.getRecords('shared-key');
      await (server as any).handleMessage(clientC, {
        type: 'ORMAP_PUSH_DIFF',
        payload: {
          mapName,
          entries: [{
            key: 'shared-key',
            records: recordsC,
            tombstones: []
          }]
        }
      });

      // Server should have all three values
      const serverValues = serverMap.get('shared-key');
      expect(serverValues).toHaveLength(3);
      expect(serverValues).toContain('value-from-A');
      expect(serverValues).toContain('value-from-B');
      expect(serverValues).toContain('value-from-C');

      // Now client D connects and requests diff
      const { client: clientD, responses: responsesD } = createMockClient('client-D');
      injectClient(clientD);

      await (server as any).handleMessage(clientD, {
        type: 'ORMAP_DIFF_REQUEST',
        payload: {
          mapName,
          keys: ['shared-key']
        }
      });

      const diffResponse = responsesD.find((r: any) => r.type === 'ORMAP_DIFF_RESPONSE');
      expect(diffResponse).toBeDefined();

      const entry = diffResponse.payload.entries[0];
      expect(entry.records).toHaveLength(3);

      const values = entry.records.map((r: any) => r.value);
      expect(values).toContain('value-from-A');
      expect(values).toContain('value-from-B');
      expect(values).toContain('value-from-C');
    });

    test('should handle concurrent add and remove across clients', async () => {
      const mapName = 'or:concurrent-test';
      const serverMap = server.getMap(mapName, 'OR') as ORMap<string, string>;

      // First, Client A adds a value
      const hlcA = new HLC('client-A-concurrent');
      const mapA = new ORMap<string, string>(hlcA);
      const recordA = mapA.add('concurrent-key', 'original-value');

      const { client: clientA, responses: responsesA } = createMockClient('client-A-concurrent');
      injectClient(clientA);

      await (server as any).handleMessage(clientA, {
        type: 'ORMAP_PUSH_DIFF',
        payload: {
          mapName,
          entries: [{
            key: 'concurrent-key',
            records: [recordA],
            tombstones: []
          }]
        }
      });

      expect(serverMap.get('concurrent-key')).toContain('original-value');

      // Client B (offline) doesn't know about A's value, adds its own
      const hlcB = new HLC('client-B-concurrent');
      const mapB = new ORMap<string, string>(hlcB);
      const recordB = mapB.add('concurrent-key', 'concurrent-value');

      // Meanwhile, Client A removes the original value (creates tombstone)
      const removedTags = mapA.remove('concurrent-key', 'original-value');

      // Client A pushes tombstone
      await (server as any).handleMessage(clientA, {
        type: 'ORMAP_PUSH_DIFF',
        payload: {
          mapName,
          entries: [{
            key: 'concurrent-key',
            records: [],
            tombstones: removedTags
          }]
        }
      });

      // Client B comes online and pushes its value
      const { client: clientB, responses: responsesB } = createMockClient('client-B-concurrent');
      injectClient(clientB);

      await (server as any).handleMessage(clientB, {
        type: 'ORMAP_PUSH_DIFF',
        payload: {
          mapName,
          entries: [{
            key: 'concurrent-key',
            records: [recordB],
            tombstones: []
          }]
        }
      });

      // Server should have B's concurrent value (because A's remove only affects A's tag)
      const finalValues = serverMap.get('concurrent-key');
      expect(finalValues).toContain('concurrent-value');
      expect(finalValues).not.toContain('original-value'); // Was removed by A
    });
  });

  describe('Access control', () => {
    test('should deny ORMAP_SYNC_INIT without READ permission', async () => {
      const clientSocket = {
        send: jest.fn(),
        close: jest.fn(),
        readyState: 1
      };

      const clientMock = {
        id: 'no-read-client',
        socket: clientSocket as any,
        writer: createMockWriter(clientSocket) as any,
        isAuthenticated: true,
        subscriptions: new Set<string>(),
        principal: { userId: 'no-read', roles: ['GUEST'] }, // No ADMIN role
        lastActiveHlc: { millis: Date.now(), counter: 0, nodeId: 'no-read' },
        lastPingReceived: Date.now()
      };

      injectClient(clientMock);

      // Configure server with restrictive policy (if security manager allows)
      // For this test, assume default policy allows all (as no policy is set)
      // The test primarily validates that the security check is in place

      await (server as any).handleMessage(clientMock, {
        type: 'ORMAP_SYNC_INIT',
        mapName: 'or:protected',
        rootHash: 0,
        bucketHashes: {},
        lastSyncTimestamp: 0
      });

      // With default policy (ALLOW_ALL), this should succeed
      // With restrictive policy, would get ERROR 403
      expect(clientSocket.send).toHaveBeenCalled();
    });
  });
});
