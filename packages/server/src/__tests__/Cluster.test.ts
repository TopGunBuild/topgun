import { ServerCoordinator, ServerFactory } from '../';
import { WebSocket } from 'ws';
import { LWWMap, deserialize } from '@topgunbuild/core';
import { pollUntil } from './utils/test-helpers';

describe('Cluster Integration', () => {
  let node1: ServerCoordinator;
  let node2: ServerCoordinator;

  beforeAll(async () => {
    // Note: Low-ID Initiator Policy means the node with lower ID must initiate.
    // So we name nodes such that node-a < node-b, and node-a initiates connection.

    // Start Node B first (higher ID, will receive connection)
    node1 = ServerFactory.create({
      port: 0,
      nodeId: 'node-b',
      host: 'localhost',
      clusterPort: 0,
      peers: []
    });

    // Wait for node-b to be ready first
    await node1.ready();

    // Start Node A (lower ID, will initiate connection to node-b)
    node2 = ServerFactory.create({
      port: 0,
      nodeId: 'node-a',
      host: 'localhost',
      clusterPort: 0,
      peers: [`localhost:${node1.clusterPort}`]
    });

    // Wait for node-a to be ready
    await node2.ready();

    // Wait for cluster to stabilize with bounded polling
    await pollUntil(
      () => {
        const m1 = (node1 as any).cluster.getMembers();
        const m2 = (node2 as any).cluster.getMembers();
        return m1.includes('node-a') && m2.includes('node-b');
      },
      {
        timeoutMs: 5000,
        intervalMs: 100,
        description: 'cluster formation (both nodes see each other)',
      }
    );
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

  test('Local Pub/Sub: Write on same node triggers client notification', async () => {
    // Phase 14.2: Cross-node live updates now require distributed subscriptions (CLUSTER_SUB_*).
    // Local QUERY_SUB only receives updates from local data changes.
    // This test verifies local pub/sub still works correctly.

    const sendMock = jest.fn();
    const clientMock = {
        id: 'client-local',
        socket: { send: sendMock, readyState: WebSocket.OPEN, close: jest.fn() } as any,
        writer: { write: sendMock, close: jest.fn() },
        isAuthenticated: true,
        subscriptions: new Set(),
        principal: { userId: 'test', roles: ['ADMIN'] },
        lastActiveHlc: { millis: Date.now(), counter: 0, nodeId: 'node-b' },
        lastPingReceived: Date.now(),
    };
    (node1 as any).connectionManager.getClients().set(clientMock.id, clientMock);

    // Client subscribes on Node 1
    (node1 as any).handleMessage(clientMock, {
        type: 'QUERY_SUB',
        payload: { queryId: 'q-local', mapName: 'users', query: {} }
    });

    // Wait for subscription to be registered (distributed subscription needs more time)
    await new Promise(resolve => setTimeout(resolve, 500));

    // Write on same Node 1
    const op = {
        opType: 'set',
        mapName: 'users',
        key: 'user:local-101',
        record: {
            value: { name: 'LocalUser' },
            timestamp: { millis: Date.now(), counter: 0, nodeId: 'client' }
        }
    };

    (node1 as any).handleMessage(clientMock, {
        type: 'CLIENT_OP',
        payload: op
    });

    await new Promise(resolve => setTimeout(resolve, 500));

    // Check if client received update
    const calls = sendMock.mock.calls;

    let updateMsg = null;
    for (const call of calls) {
        try {
            const arg = call[0];

            // 1. Direct object (writer.write sends objects directly)
            if (arg && typeof arg === 'object' && !Buffer.isBuffer(arg) && !(arg instanceof Uint8Array)) {
                const msg = arg as any;
                if (msg.type === 'QUERY_UPDATE' && msg.payload?.key === 'user:local-101') {
                    updateMsg = msg;
                    break;
                }
            }

            // 2. JSON string (distributed subscriptions use JSON.stringify)
            if (typeof arg === 'string') {
                const parsed = JSON.parse(arg);
                if (parsed.type === 'QUERY_UPDATE' && parsed.payload?.key === 'user:local-101') {
                    updateMsg = parsed;
                    break;
                }
            }

            // 3. Msgpack binary (local QueryRegistry uses serialize())
            if (Buffer.isBuffer(arg) || arg instanceof Uint8Array) {
                const deserialized = deserialize(arg as Uint8Array) as any;
                if (deserialized.type === 'QUERY_UPDATE' && deserialized.payload?.key === 'user:local-101') {
                    updateMsg = deserialized;
                    break;
                }
            }
        } catch (e) {
            // Ignore parsing errors
        }
    }

    expect(updateMsg).not.toBeNull();
    if (updateMsg) {
        expect(updateMsg.payload.key).toBe('user:local-101');
        expect(updateMsg.payload.value.name).toBe('LocalUser');
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
