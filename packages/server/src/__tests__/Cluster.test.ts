import { ServerCoordinator } from '../ServerCoordinator';
import { WebSocket } from 'ws';
import { LWWMap, deserialize } from '@topgunbuild/core';

describe('Cluster Integration', () => {
  let node1: ServerCoordinator;
  let node2: ServerCoordinator;

  beforeAll(async () => {
    // Note: Low-ID Initiator Policy means the node with lower ID must initiate.
    // So we name nodes such that node-a < node-b, and node-a initiates connection.

    // Start Node B first (higher ID, will receive connection)
    node1 = new ServerCoordinator({
      port: 0,
      nodeId: 'node-b',
      host: 'localhost',
      clusterPort: 0,
      peers: []
    });

    // Wait for node-b to be ready first
    await node1.ready();

    // Start Node A (lower ID, will initiate connection to node-b)
    node2 = new ServerCoordinator({
      port: 0,
      nodeId: 'node-a',
      host: 'localhost',
      clusterPort: 0,
      peers: [`localhost:${node1.clusterPort}`]
    });

    // Wait for node-a to be ready
    await node2.ready();

    // Wait for cluster to stabilize
    // Poll until both nodes see each other
    const start = Date.now();
    while (Date.now() - start < 5000) {
        const m1 = (node1 as any).cluster.getMembers();
        const m2 = (node2 as any).cluster.getMembers();
        if (m1.includes('node-a') && m2.includes('node-b')) {
            break;
        }
        await new Promise(resolve => setTimeout(resolve, 100));
    }
  }, 15000);

  afterAll(async () => {
    await node1.shutdown();
    await node2.shutdown();
    // Give it a moment to release ports
    await new Promise(resolve => setTimeout(resolve, 200));
  });

  test('Cluster Formation', () => {
    const members1 = (node1 as any).cluster.getMembers();
    const members2 = (node2 as any).cluster.getMembers();

    expect(members1).toContain('node-b');
    expect(members1).toContain('node-a');
    expect(members2).toContain('node-b');
    expect(members2).toContain('node-a');
  });

  test('Data Replication: Node 1 Write -> Node 2 Read', async () => {
    // 1. Write to Node 1 (simulating a client op)
    const clientMock = {
      id: 'client-1',
      socket: { send: jest.fn(), readyState: WebSocket.OPEN, close: jest.fn() } as any,
      isAuthenticated: true,
      subscriptions: new Set(),
      principal: { userId: 'test', roles: ['ADMIN'] }
    };

    // We need a key that hashes such that Node 2 is either Owner or Backup.
    // Since we have 2 nodes, Node 2 will always be one or the other (Replica Count 1).
    // So replication SHOULD happen regardless of hash.
    const key = 'user:100';

    const op = {
      opType: 'set',
      mapName: 'users',
      key: key,
      record: {
        value: { name: 'Iceman' },
        timestamp: { millis: Date.now(), counter: 0, nodeId: 'client' }
      }
    };

    // Force processing
    (node1 as any).handleMessage(clientMock, {
      type: 'CLIENT_OP',
      payload: op
    });

    // Wait for propagation
    await new Promise(resolve => setTimeout(resolve, 500));

    // 2. Check Node 2's internal state
    const map2 = node2.getMap('users') as LWWMap<string, any>;
    const val = map2.getRecord(key);

    expect(val).toBeDefined();
    expect(val!.value).toEqual({ name: 'Iceman' });
  });

  test('Pub/Sub: Node 1 Write -> Node 2 Client Notification', async () => {
    const client2Mock = {
        id: 'client-2',
        socket: { send: jest.fn(), readyState: WebSocket.OPEN, close: jest.fn() } as any,
        isAuthenticated: true,
        subscriptions: new Set(),
        principal: { userId: 'test2', roles: ['ADMIN'] }
    };
    (node2 as any).clients.set(client2Mock.id, client2Mock);

    // Client 2 subscribes on Node 2
    (node2 as any).handleMessage(client2Mock, {
        type: 'QUERY_SUB',
        payload: { queryId: 'q2', mapName: 'users', query: {} }
    });

    // Wait for subscription to be registered
    await new Promise(resolve => setTimeout(resolve, 100));

    // Write on Node 1
    const op = {
        opType: 'set',
        mapName: 'users',
        key: 'user:101',
        record: {
            value: { name: 'Goose' },
            timestamp: { millis: Date.now(), counter: 0, nodeId: 'client' }
        }
    };

    const client1Mock = {
        id: 'client-1',
        socket: { send: jest.fn(), readyState: WebSocket.OPEN, close: jest.fn() } as any,
        isAuthenticated: true,
        subscriptions: new Set(),
        principal: { userId: 'test', roles: ['ADMIN'] }
    };

    (node1 as any).handleMessage(client1Mock, {
        type: 'CLIENT_OP',
        payload: op
    });

    await new Promise(resolve => setTimeout(resolve, 1000));

    // Check if Client 2 received update
    expect(client2Mock.socket.send).toHaveBeenCalled();
    const calls = (client2Mock.socket.send as jest.Mock).mock.calls;

    // Find QUERY_UPDATE or SERVER_EVENT
    // The current implementation sends SERVER_EVENT for broadcast,
    // which triggers QueryRegistry.processChange, which sends QUERY_UPDATE

    let updateMsg = null;
    for (const call of calls) {
        try {
            const msg = deserialize(call[0] as Uint8Array) as any;
            if (msg.type === 'QUERY_UPDATE') {
                updateMsg = msg;
                break;
            }
        } catch (e) {
            // Ignore deserialization errors for non-msgpack messages
        }
    }

    expect(updateMsg).not.toBeNull();
    if (updateMsg) {
        expect(updateMsg.payload.key).toBe('user:101');
        expect(updateMsg.payload.value.name).toBe('Goose');
    }
  });

  test('Partition Service Distribution', () => {
      // Access private service via cast
      const ps = (node1 as any).partitionService;
      const key = 'test-key-123';

      const dist = ps.getDistribution(key);
      expect(dist.owner).toBeDefined();
      expect(dist.backups).toBeInstanceOf(Array);
      // With 2 nodes, we expect 1 backup
      expect(dist.backups.length).toBeGreaterThan(0);

      const members = (node1 as any).cluster.getMembers();
      // Owner + Backup should cover the cluster in 2-node scenario
      const covered = new Set([dist.owner, ...dist.backups]);
      expect(covered.size).toBe(2);
  });

});
