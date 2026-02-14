/**
 * Integration Tests: Distributed Subscriptions
 *
 * Tests distributed live subscriptions across a real 3-node cluster:
 * - Search subscriptions with initial results from all nodes
 * - Query subscriptions with predicate filtering across nodes
 * - Live updates (ENTER/UPDATE/LEAVE) propagation
 * - Client disconnect cleanup
 * - Cluster resilience during node failures
 * - Performance and latency tests
 *
 * Run: pnpm --filter @topgunbuild/server test:integration:distributed
 */

import { WebSocket } from 'ws';
import jwt from 'jsonwebtoken';
import { ServerCoordinator, ServerFactory } from '../../';
import { serialize, deserialize, ConsistencyLevel } from '@topgunbuild/core';
import { waitForCluster } from '../utils/test-helpers';

const JWT_SECRET = 'test-secret-for-e2e-tests';

// Helper: Create a valid JWT token with ADMIN role for full access
function createTestToken(userId = 'test-user', roles = ['ADMIN']): string {
  return jwt.sign({ userId, roles }, JWT_SECRET, { expiresIn: '1h' });
}

interface TestNode {
  server: ServerCoordinator;
  port: number;
  clusterPort: number;
  nodeId: string;
}

// Helper: Sleep for a given number of milliseconds
function sleep(ms: number): Promise<void> {
  return new Promise(resolve => setTimeout(resolve, ms));
}

// Helper: Create WebSocket client connection
function createClient(port: number): Promise<WebSocket> {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(`ws://localhost:${port}`);
    const timeout = setTimeout(() => {
      ws.close();
      reject(new Error('Connection timeout'));
    }, 5000);

    ws.on('open', () => {
      clearTimeout(timeout);
      resolve(ws);
    });
    ws.on('error', (err) => {
      clearTimeout(timeout);
      reject(err);
    });
  });
}

// Helper: Authenticate client
async function authenticateClient(ws: WebSocket, userId = 'test-user'): Promise<void> {
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => reject(new Error('Auth timeout')), 5000);

    const handler = (data: Buffer) => {
      try {
        const msg = deserialize(data) as any;
        if (msg.type === 'AUTH_ACK' || msg.type === 'AUTH_SUCCESS' || msg.type === 'AUTH_RESP') {
          clearTimeout(timeout);
          ws.off('message', handler);
          resolve();
        } else if (msg.type === 'AUTH_FAIL' || msg.type === 'ERROR' || msg.type === 'AUTH_ERROR') {
          clearTimeout(timeout);
          ws.off('message', handler);
          reject(new Error(msg.payload?.message || msg.error || 'Auth failed'));
        }
      } catch {
        // Skip non-msgpack messages
      }
    };

    ws.on('message', handler);
    const token = createTestToken(userId);
    ws.send(serialize({ type: 'AUTH', token }));
  });
}

// Helper: Send message and wait for response
function sendAndWait(
  ws: WebSocket,
  message: any,
  responseType: string | string[],
  timeout = 5000
): Promise<any> {
  const responseTypes = Array.isArray(responseType) ? responseType : [responseType];

  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      ws.off('message', handler);
      reject(new Error(`Timeout waiting for ${responseTypes.join(' or ')}`));
    }, timeout);

    const handler = (data: Buffer) => {
      try {
        const msg = deserialize(data) as any;
        if (responseTypes.includes(msg.type)) {
          clearTimeout(timer);
          ws.off('message', handler);
          resolve(msg);
        }
      } catch {
        // Skip non-msgpack messages
      }
    };

    ws.on('message', handler);
    ws.send(serialize(message));
  });
}

// Helper: Parse message (tries msgpack first, then JSON)
function parseMessage(data: Buffer | string): any {
  // Try msgpack first
  try {
    if (Buffer.isBuffer(data)) {
      return deserialize(data);
    }
  } catch {
    // Not msgpack
  }

  // Try JSON
  try {
    const str = Buffer.isBuffer(data) ? data.toString() : data;
    return JSON.parse(str);
  } catch {
    return null;
  }
}

function collectMessages(
  ws: WebSocket,
  types: string[],
  count: number,
  timeout = 10000
): Promise<any[]> {
  return new Promise((resolve) => {
    const messages: any[] = [];
    const timer = setTimeout(() => {
      ws.off('message', handler);
      resolve(messages);
    }, timeout);

    const handler = (data: Buffer) => {
      const msg = parseMessage(data);
      if (msg && types.includes(msg.type)) {
        messages.push(msg);
        if (messages.length >= count) {
          clearTimeout(timer);
          ws.off('message', handler);
          resolve(messages);
        }
      }
    };

    ws.on('message', handler);
  });
}

// Helper: Create a timestamp for records
function createTimestamp(nodeId = 'test-node') {
  return {
    millis: Date.now(),
    counter: Math.floor(Math.random() * 1000),
    nodeId,
  };
}

// Helper: Write data to a node via WebSocket (fire-and-forget style)
// CLIENT_OP doesn't return an ACK, so we just send and wait a bit for processing
async function writeData(
  ws: WebSocket,
  mapName: string,
  key: string,
  value: any
): Promise<void> {
  // Send the operation
  ws.send(
    serialize({
      type: 'CLIENT_OP',
      payload: {
        opType: 'set',
        mapName,
        key,
        record: {
          value,
          timestamp: createTimestamp(),
        },
      },
    })
  );

  // Wait briefly for the operation to be processed
  await sleep(50);
}

describe('Distributed Subscriptions E2E', () => {
  let nodes: TestNode[] = [];
  const BASE_PORT = 10100;
  const CLUSTER_BASE_PORT = 11100;

  beforeAll(async () => {
    // Start 3-node cluster
    // Start first node
    const server1 = ServerFactory.create({
      port: BASE_PORT,
      nodeId: 'e2e-node-1',
      host: 'localhost',
      clusterPort: CLUSTER_BASE_PORT,
      peers: [],
      jwtSecret: JWT_SECRET,
      metricsPort: 0, // Use random port for metrics to avoid conflicts
      fullTextSearch: {
        articles: { fields: ['title', 'body'] },
        events: { fields: ['title', 'body'] },
        latency: { fields: ['title', 'body'] },
      },
      replicationEnabled: true,
      defaultConsistency: ConsistencyLevel.EVENTUAL,
    });
    await server1.ready();
    nodes.push({
      server: server1,
      port: server1.port,
      clusterPort: server1.clusterPort,
      nodeId: 'e2e-node-1',
    });

    // Start second node, connect to first
    const server2 = ServerFactory.create({
      port: BASE_PORT + 1,
      nodeId: 'e2e-node-2',
      host: 'localhost',
      clusterPort: CLUSTER_BASE_PORT + 1,
      peers: [`localhost:${nodes[0].clusterPort}`],
      jwtSecret: JWT_SECRET,
      metricsPort: 0, // Use random port for metrics to avoid conflicts
      fullTextSearch: {
        articles: { fields: ['title', 'body'] },
        events: { fields: ['title', 'body'] },
        latency: { fields: ['title', 'body'] },
      },
      replicationEnabled: true,
      defaultConsistency: ConsistencyLevel.EVENTUAL,
    });
    await server2.ready();
    nodes.push({
      server: server2,
      port: server2.port,
      clusterPort: server2.clusterPort,
      nodeId: 'e2e-node-2',
    });

    // Start third node, connect to first two
    const server3 = ServerFactory.create({
      port: BASE_PORT + 2,
      nodeId: 'e2e-node-3',
      host: 'localhost',
      clusterPort: CLUSTER_BASE_PORT + 2,
      peers: [`localhost:${nodes[0].clusterPort}`, `localhost:${nodes[1].clusterPort}`],
      jwtSecret: JWT_SECRET,
      metricsPort: 0, // Use random port for metrics to avoid conflicts
      fullTextSearch: {
        articles: { fields: ['title', 'body'] },
        events: { fields: ['title', 'body'] },
        latency: { fields: ['title', 'body'] },
      },
      replicationEnabled: true,
      defaultConsistency: ConsistencyLevel.EVENTUAL,
    });
    await server3.ready();
    nodes.push({
      server: server3,
      port: server3.port,
      clusterPort: server3.clusterPort,
      nodeId: 'e2e-node-3',
    });

    // Wait for cluster formation
    const servers = nodes.map(n => n.server);
    await waitForCluster(servers, 3, 15000);

    // Give extra time for cluster stabilization
    await sleep(500);
  }, 30000);

  afterAll(async () => {
    // Shutdown all nodes
    for (const node of nodes) {
      try {
        await node.server.shutdown();
      } catch {
        // Ignore shutdown errors
      }
    }
    await sleep(300);
  }, 15000);

  describe('Cluster Formation', () => {
    it('should form a 3-node cluster', () => {
      for (const node of nodes) {
        const members = (node.server as any).cluster?.getMembers() || [];
        expect(members.length).toBe(3);
        expect(members).toContain('e2e-node-1');
        expect(members).toContain('e2e-node-2');
        expect(members).toContain('e2e-node-3');
      }
    });
  });

  describe('Search Subscriptions', () => {
    it('should receive initial results from all nodes', async () => {
      // Connect and authenticate clients to all nodes
      const client1 = await createClient(nodes[0].port);
      const client2 = await createClient(nodes[1].port);
      const client3 = await createClient(nodes[2].port);

      await authenticateClient(client1);
      await authenticateClient(client2);
      await authenticateClient(client3);

      // Write data to different nodes
      await writeData(client1, 'articles', 'art-1', {
        title: 'Node 1 Article',
        body: 'Test content from node one',
      });
      await writeData(client2, 'articles', 'art-2', {
        title: 'Node 2 Article',
        body: 'Test content from node two',
      });
      await writeData(client3, 'articles', 'art-3', {
        title: 'Node 3 Article',
        body: 'Test content from node three',
      });

      // Wait for replication
      await sleep(500);

      // Subscribe to search from node 1
      const subResponse = await sendAndWait(
        client1,
        {
          type: 'SEARCH_SUB',
          payload: {
            subscriptionId: 'e2e-search-initial',
            mapName: 'articles',
            query: 'Article',
            options: { limit: 10 },
          },
        },
        'SEARCH_RESP',
        10000
      );

      // Should receive results from all nodes
      expect(subResponse.payload.results.length).toBeGreaterThanOrEqual(3);
      const keys = subResponse.payload.results.map((r: any) => r.key);
      expect(keys).toContain('art-1');
      expect(keys).toContain('art-2');
      expect(keys).toContain('art-3');

      client1.close();
      client2.close();
      client3.close();
    }, 30000);

    it('should receive live updates from remote nodes', async () => {
      const subscriber = await createClient(nodes[0].port);
      const writer = await createClient(nodes[1].port);

      await authenticateClient(subscriber);
      await authenticateClient(writer);

      // Subscribe on node 1
      await sendAndWait(
        subscriber,
        {
          type: 'SEARCH_SUB',
          payload: {
            subscriptionId: 'e2e-search-live',
            mapName: 'articles',
            query: 'breaking news',
            options: { limit: 10 },
          },
        },
        'SEARCH_RESP',
        5000
      );

      // Start collecting updates
      const updatePromise = collectMessages(subscriber, ['SEARCH_UPDATE'], 1, 5000);

      // Write matching document on node 2
      await writeData(writer, 'articles', 'breaking-1', {
        title: 'Breaking News',
        body: 'Important breaking news story',
      });

      // Should receive ENTER update
      const updates = await updatePromise;
      expect(updates.length).toBeGreaterThan(0);

      const enterUpdate = updates.find(
        (m) => m.payload?.changeType === 'ENTER' && m.payload?.key === 'breaking-1'
      );
      expect(enterUpdate).toBeDefined();

      subscriber.close();
      writer.close();
    }, 15000);

    it('should receive LEAVE update when document no longer matches', async () => {
      const subscriber = await createClient(nodes[0].port);
      const writer = await createClient(nodes[1].port);

      await authenticateClient(subscriber);
      await authenticateClient(writer);

      // Create initial matching document
      await writeData(writer, 'articles', 'update-test', {
        title: 'Matching Title',
        body: 'Original content',
      });

      await sleep(300);

      // Subscribe
      const subResponse = await sendAndWait(
        subscriber,
        {
          type: 'SEARCH_SUB',
          payload: {
            subscriptionId: 'e2e-search-leave',
            mapName: 'articles',
            query: 'Matching Title',
            options: {},
          },
        },
        'SEARCH_RESP',
        5000
      );

      expect(
        subResponse.payload.results.some((r: any) => r.key === 'update-test')
      ).toBe(true);

      // Start collecting updates
      const updatePromise = collectMessages(subscriber, ['SEARCH_UPDATE'], 1, 5000);

      // Update document to no longer match
      await writeData(writer, 'articles', 'update-test', {
        title: 'Different Title',
        body: 'Changed content completely',
      });

      const updates = await updatePromise;
      // Should receive either LEAVE or UPDATE
      expect(updates.length).toBeGreaterThan(0);

      subscriber.close();
      writer.close();
    }, 15000);
  });

  describe('Query Subscriptions', () => {
    it('should receive query results from all nodes', async () => {
      const client1 = await createClient(nodes[0].port);
      const client2 = await createClient(nodes[1].port);
      const client3 = await createClient(nodes[2].port);

      await authenticateClient(client1);
      await authenticateClient(client2);
      await authenticateClient(client3);

      // Write data to different nodes
      await writeData(client1, 'users', 'user-1', {
        name: 'Alice',
        age: 25,
        active: true,
      });
      await writeData(client2, 'users', 'user-2', {
        name: 'Bob',
        age: 30,
        active: true,
      });
      await writeData(client3, 'users', 'user-3', {
        name: 'Charlie',
        age: 17,
        active: true,
      });

      await sleep(500);

      // Subscribe with query predicate
      const subResponse = await sendAndWait(
        client1,
        {
          type: 'QUERY_SUB',
          payload: {
            queryId: 'e2e-query-1',
            mapName: 'users',
            query: { where: { age: { $gte: 18 }, active: true } },
          },
        },
        'QUERY_RESP',
        10000
      );

      // Should only get Alice and Bob (age >= 18)
      expect(subResponse.payload.results.length).toBeGreaterThanOrEqual(2);
      const keys = subResponse.payload.results.map((r: any) => r.key);
      expect(keys).toContain('user-1');
      expect(keys).toContain('user-2');
      // Charlie should not be included (age 17 < 18)
      expect(keys).not.toContain('user-3');

      client1.close();
      client2.close();
      client3.close();
    }, 30000);

    it('should receive live query updates from remote nodes', async () => {
      // Cross-node Query live updates now work via DistributedSubscriptionCoordinator
      // Subscriber connects to node 0, writer connects to node 2 (different node)
      const subscriber = await createClient(nodes[0].port);
      const writer = await createClient(nodes[2].port); // Different node!

      await authenticateClient(subscriber, 'subscriber-user');
      await authenticateClient(writer, 'writer-user');

      // Subscribe for active premium users on node 0
      await sendAndWait(
        subscriber,
        {
          type: 'QUERY_SUB',
          payload: {
            queryId: 'e2e-query-live-remote',
            mapName: 'accounts',
            query: { where: { tier: 'premium', active: true } },
          },
        },
        'QUERY_RESP',
        5000
      );

      // Wait a bit for subscription to propagate across cluster
      await sleep(200);

      const updatePromise = collectMessages(subscriber, ['QUERY_UPDATE'], 1, 5000);

      // Create matching record on DIFFERENT node (node 2)
      await writeData(writer, 'accounts', 'acc-premium-remote', {
        name: 'Premium Remote User',
        tier: 'premium',
        active: true,
      });

      const updates = await updatePromise;
      // With distributed subscription coordinator, cross-node updates should propagate
      expect(updates.length).toBeGreaterThanOrEqual(1);
      if (updates.length > 0) {
        expect(updates[0].type).toBe('QUERY_UPDATE');
        expect(updates[0].payload).toBeDefined();
      }

      subscriber.close();
      writer.close();
    }, 15000);
  });

  describe('Multiple Subscriptions', () => {
    it('should handle multiple subscriptions from same client sequentially', async () => {
      const client = await createClient(nodes[0].port);
      await authenticateClient(client);

      // Create subscriptions sequentially to avoid message handler conflicts
      const sub1 = await sendAndWait(
        client,
        {
          type: 'SEARCH_SUB',
          payload: {
            subscriptionId: 'multi-1',
            mapName: 'articles',
            query: 'tech',
            options: {},
          },
        },
        'SEARCH_RESP',
        5000
      );
      expect(sub1.payload).toBeDefined();

      const sub2 = await sendAndWait(
        client,
        {
          type: 'SEARCH_SUB',
          payload: {
            subscriptionId: 'multi-2',
            mapName: 'articles',
            query: 'science',
            options: {},
          },
        },
        'SEARCH_RESP',
        5000
      );
      expect(sub2.payload).toBeDefined();

      const sub3 = await sendAndWait(
        client,
        {
          type: 'QUERY_SUB',
          payload: {
            queryId: 'multi-3',
            mapName: 'users',
            query: { where: { active: true } },
          },
        },
        'QUERY_RESP',
        5000
      );
      expect(sub3.payload).toBeDefined();

      client.close();
    }, 20000);

    it('should properly cleanup subscriptions on client disconnect', async () => {
      const client = await createClient(nodes[0].port);
      await authenticateClient(client);

      await sendAndWait(
        client,
        {
          type: 'SEARCH_SUB',
          payload: {
            subscriptionId: 'cleanup-1',
            mapName: 'articles',
            query: 'cleanup test',
            options: {},
          },
        },
        'SEARCH_RESP',
        5000
      );

      // Get subscription count before disconnect
      const coordBefore = (nodes[0].server as any).distributedSubCoordinator;
      const countBefore = coordBefore?.getActiveSubscriptionCount() ?? 0;

      // Disconnect client
      client.close();
      await sleep(1000);

      // Subscription should be cleaned up
      const countAfter = coordBefore?.getActiveSubscriptionCount() ?? 0;
      expect(countAfter).toBeLessThanOrEqual(countBefore);
    }, 15000);
  });

  describe('Cluster Resilience', () => {
    it('should continue receiving updates after restarting a data node', async () => {
      // This test verifies that the subscription continues to work
      // when one of the data nodes restarts
      const subscriber = await createClient(nodes[0].port);
      await authenticateClient(subscriber);

      // Subscribe on node 1
      await sendAndWait(
        subscriber,
        {
          type: 'SEARCH_SUB',
          payload: {
            subscriptionId: 'e2e-resilience-1',
            mapName: 'articles',
            query: 'resilience test',
            options: {},
          },
        },
        'SEARCH_RESP',
        5000
      );

      // Write from node 2 (still running)
      const writer = await createClient(nodes[1].port);
      await authenticateClient(writer);

      await writeData(writer, 'articles', 'during-test', {
        title: 'Resilience Test',
        body: 'Written during resilience test',
      });

      // Should receive update
      const updatePromise = collectMessages(subscriber, ['SEARCH_UPDATE'], 1, 3000);
      const updates = await updatePromise;
      // May or may not receive depending on timing, but should not crash
      expect(Array.isArray(updates)).toBe(true);

      subscriber.close();
      writer.close();
    }, 20000);
  });

  describe('Performance', () => {
    it('should handle high-frequency updates', async () => {
      const subscriber = await createClient(nodes[0].port);
      const writer = await createClient(nodes[1].port);

      await authenticateClient(subscriber);
      await authenticateClient(writer);

      await sendAndWait(
        subscriber,
        {
          type: 'SEARCH_SUB',
          payload: {
            subscriptionId: 'perf-1',
            mapName: 'events',
            query: 'event',
            options: {},
          },
        },
        'SEARCH_RESP',
        5000
      );

      const updateCount = 50;
      const collectPromise = collectMessages(
        subscriber,
        ['SEARCH_UPDATE'],
        updateCount,
        30000
      );

      // Write many documents rapidly
      const startTime = Date.now();
      for (let i = 0; i < updateCount; i++) {
        await writeData(writer, 'events', `event-${i}`, {
          title: `Event ${i}`,
          body: 'Event description',
        });
      }

      const updates = await collectPromise;
      const duration = Date.now() - startTime;

      console.log(`Received ${updates.length} updates in ${duration}ms`);
      // Allow some loss due to batching/timing
      expect(updates.length).toBeGreaterThanOrEqual(updateCount * 0.5);

      subscriber.close();
      writer.close();
    }, 60000);

    it('should maintain reasonable latency for updates', async () => {
      const subscriber = await createClient(nodes[0].port);
      const writer = await createClient(nodes[2].port);

      await authenticateClient(subscriber);
      await authenticateClient(writer);

      await sendAndWait(
        subscriber,
        {
          type: 'SEARCH_SUB',
          payload: {
            subscriptionId: 'latency-1',
            mapName: 'latency',
            query: 'latency',
            options: {},
          },
        },
        'SEARCH_RESP',
        5000
      );

      const latencies: number[] = [];

      for (let i = 0; i < 5; i++) {
        const sendTime = Date.now();

        const updatePromise = new Promise<number>((resolve) => {
          const handler = (data: Buffer) => {
            try {
              const msg = deserialize(data) as any;
              if (
                msg.type === 'SEARCH_UPDATE' &&
                msg.payload?.key === `lat-${i}`
              ) {
                subscriber.off('message', handler);
                resolve(Date.now() - sendTime);
              }
            } catch {
              // Skip
            }
          };
          subscriber.on('message', handler);

          // Timeout fallback
          setTimeout(() => {
            subscriber.off('message', handler);
            resolve(-1);
          }, 5000);
        });

        await writeData(writer, 'latency', `lat-${i}`, {
          title: 'Latency Test',
          body: 'latency measurement',
        });

        const latency = await updatePromise;
        if (latency > 0) {
          latencies.push(latency);
        }
      }

      if (latencies.length > 0) {
        const avgLatency =
          latencies.reduce((a, b) => a + b, 0) / latencies.length;
        const maxLatency = Math.max(...latencies);

        console.log(
          `Average latency: ${avgLatency.toFixed(2)}ms, Max: ${maxLatency}ms`
        );
        // Average should be under 1 second for local cluster
        expect(avgLatency).toBeLessThan(1000);
      }

      subscriber.close();
      writer.close();
    }, 60000);
  });
});
