import { ServerCoordinator } from '@topgunbuild/server';
import {
  createTestServer,
  createTestClient,
  createTestContext,
  waitForSync,
  waitUntil,
  TestClient,
} from './helpers';

describe('E2E: Pub/Sub', () => {
  // ========================================
  // Basic Pub/Sub Tests
  // ========================================
  describe('Basic Pub/Sub', () => {
    let server: ServerCoordinator;
    let publisher: TestClient;
    let subscriber: TestClient;

    beforeEach(async () => {
      server = await createTestServer();
      publisher = await createTestClient(`ws://localhost:${server.port}`, {
        roles: ['ADMIN'],
        nodeId: 'publisher',
      });
      subscriber = await createTestClient(`ws://localhost:${server.port}`, {
        roles: ['ADMIN'],
        nodeId: 'subscriber',
      });
      await publisher.waitForMessage('AUTH_ACK');
      await subscriber.waitForMessage('AUTH_ACK');
    });

    afterEach(async () => {
      publisher.close();
      subscriber.close();
      await server.shutdown();
    });

    test('client subscribes to topic and receives message from another client', async () => {
      // Subscriber subscribes to topic
      subscriber.send({
        type: 'TOPIC_SUB',
        payload: { topic: 'notifications' },
      });

      await waitForSync(100);
      subscriber.messages.length = 0;

      // Publisher publishes message
      publisher.send({
        type: 'TOPIC_PUB',
        payload: {
          topic: 'notifications',
          data: { message: 'Hello from publisher!' },
        },
      });

      // Subscriber receives message
      const msg = await subscriber.waitForMessage('TOPIC_MESSAGE');
      expect(msg).toBeDefined();
      expect(msg.type).toBe('TOPIC_MESSAGE');
      expect(msg.payload.topic).toBe('notifications');
      expect(msg.payload.data).toEqual({ message: 'Hello from publisher!' });
      // publisherId is the server's internal clientId (UUID), not the nodeId
      expect(typeof msg.payload.publisherId).toBe('string');
      expect(msg.payload.publisherId).toBeTruthy();
      expect(msg.payload.timestamp).toBeDefined();
    });

    test('publisher does NOT receive its own message', async () => {
      // Publisher also subscribes to the topic
      publisher.send({
        type: 'TOPIC_SUB',
        payload: { topic: 'echo-test' },
      });

      await waitForSync(100);
      publisher.messages.length = 0;

      // Publisher publishes
      publisher.send({
        type: 'TOPIC_PUB',
        payload: {
          topic: 'echo-test',
          data: { text: 'Should not echo back' },
        },
      });

      // Wait a bit and check no message received
      await waitForSync(300);

      const ownMessage = publisher.messages.find(
        (m) => m.type === 'TOPIC_MESSAGE' && m.payload.data?.text === 'Should not echo back'
      );
      expect(ownMessage).toBeUndefined();
    });
  });

  // ========================================
  // Multiple Subscribers Tests
  // ========================================
  describe('Multiple Subscribers', () => {
    test('3 clients subscribed to one topic - one publishes, all others receive', async () => {
      const ctx = await createTestContext(3);

      try {
        const [client1, client2, client3] = ctx.clients;

        // All three subscribe
        client1.send({ type: 'TOPIC_SUB', payload: { topic: 'broadcast' } });
        client2.send({ type: 'TOPIC_SUB', payload: { topic: 'broadcast' } });
        client3.send({ type: 'TOPIC_SUB', payload: { topic: 'broadcast' } });

        await waitForSync(150);
        client1.messages.length = 0;
        client2.messages.length = 0;
        client3.messages.length = 0;

        // Client1 publishes
        client1.send({
          type: 'TOPIC_PUB',
          payload: {
            topic: 'broadcast',
            data: { from: 'client-0', text: 'Broadcast message' },
          },
        });

        // Wait for messages
        await waitForSync(300);

        // Client1 (publisher) should NOT receive the message
        const client1Msg = client1.messages.find((m) => m.type === 'TOPIC_MESSAGE');
        expect(client1Msg).toBeUndefined();

        // Client2 and Client3 should receive the message
        const client2Msg = client2.messages.find((m) => m.type === 'TOPIC_MESSAGE');
        const client3Msg = client3.messages.find((m) => m.type === 'TOPIC_MESSAGE');

        expect(client2Msg).toBeDefined();
        expect(client2Msg.payload.data).toEqual({ from: 'client-0', text: 'Broadcast message' });

        expect(client3Msg).toBeDefined();
        expect(client3Msg.payload.data).toEqual({ from: 'client-0', text: 'Broadcast message' });
      } finally {
        await ctx.cleanup();
      }
    });

    test('message is delivered to all subscribers simultaneously', async () => {
      const server = await createTestServer();

      try {
        // Create 5 subscribers
        const subscribers: TestClient[] = [];
        for (let i = 0; i < 5; i++) {
          const sub = await createTestClient(`ws://localhost:${server.port}`, {
            roles: ['ADMIN'],
            nodeId: `subscriber-${i}`,
          });
          await sub.waitForMessage('AUTH_ACK');
          sub.send({ type: 'TOPIC_SUB', payload: { topic: 'multi-sub' } });
          subscribers.push(sub);
        }

        // Create publisher
        const publisher = await createTestClient(`ws://localhost:${server.port}`, {
          roles: ['ADMIN'],
          nodeId: 'multi-publisher',
        });
        await publisher.waitForMessage('AUTH_ACK');

        await waitForSync(150);
        subscribers.forEach((s) => (s.messages.length = 0));

        // Publish
        publisher.send({
          type: 'TOPIC_PUB',
          payload: {
            topic: 'multi-sub',
            data: { message: 'Hello all!' },
          },
        });

        await waitForSync(400);

        // All subscribers should receive
        for (let i = 0; i < 5; i++) {
          const msg = subscribers[i].messages.find((m) => m.type === 'TOPIC_MESSAGE');
          expect(msg).toBeDefined();
          expect(msg.payload.data).toEqual({ message: 'Hello all!' });
        }

        // Cleanup
        publisher.close();
        subscribers.forEach((s) => s.close());
      } finally {
        await server.shutdown();
      }
    });
  });

  // ========================================
  // Multiple Topics Tests
  // ========================================
  describe('Multiple Topics', () => {
    let server: ServerCoordinator;
    let clientA: TestClient;
    let clientB: TestClient;

    beforeEach(async () => {
      server = await createTestServer();
      clientA = await createTestClient(`ws://localhost:${server.port}`, {
        roles: ['ADMIN'],
        nodeId: 'client-a',
      });
      clientB = await createTestClient(`ws://localhost:${server.port}`, {
        roles: ['ADMIN'],
        nodeId: 'client-b',
      });
      await clientA.waitForMessage('AUTH_ACK');
      await clientB.waitForMessage('AUTH_ACK');
    });

    afterEach(async () => {
      clientA.close();
      clientB.close();
      await server.shutdown();
    });

    test('client subscribed to topic A and B - receives messages only from subscribed topics', async () => {
      // ClientA subscribes to both topics
      clientA.send({ type: 'TOPIC_SUB', payload: { topic: 'topic-A' } });
      clientA.send({ type: 'TOPIC_SUB', payload: { topic: 'topic-B' } });

      await waitForSync(100);
      clientA.messages.length = 0;

      // ClientB publishes to topic-A
      clientB.send({
        type: 'TOPIC_PUB',
        payload: { topic: 'topic-A', data: { source: 'A' } },
      });

      await waitForSync(150);

      // ClientA should receive from topic-A
      const msgA = clientA.messages.find(
        (m) => m.type === 'TOPIC_MESSAGE' && m.payload.topic === 'topic-A'
      );
      expect(msgA).toBeDefined();
      expect(msgA.payload.data).toEqual({ source: 'A' });

      clientA.messages.length = 0;

      // ClientB publishes to topic-B
      clientB.send({
        type: 'TOPIC_PUB',
        payload: { topic: 'topic-B', data: { source: 'B' } },
      });

      await waitForSync(150);

      // ClientA should receive from topic-B
      const msgB = clientA.messages.find(
        (m) => m.type === 'TOPIC_MESSAGE' && m.payload.topic === 'topic-B'
      );
      expect(msgB).toBeDefined();
      expect(msgB.payload.data).toEqual({ source: 'B' });
    });

    test('message to topic A is not delivered to subscriber of topic B only', async () => {
      // ClientA subscribes only to topic-A
      clientA.send({ type: 'TOPIC_SUB', payload: { topic: 'topic-only-A' } });

      await waitForSync(100);
      clientA.messages.length = 0;

      // ClientB publishes to topic-B (which clientA is NOT subscribed to)
      clientB.send({
        type: 'TOPIC_PUB',
        payload: { topic: 'topic-only-B', data: { value: 'B-message' } },
      });

      await waitForSync(300);

      // ClientA should NOT receive any message
      const msg = clientA.messages.find((m) => m.type === 'TOPIC_MESSAGE');
      expect(msg).toBeUndefined();
    });

    test('messages from different topics have correct topic field', async () => {
      // ClientA subscribes to multiple topics
      clientA.send({ type: 'TOPIC_SUB', payload: { topic: 'news' } });
      clientA.send({ type: 'TOPIC_SUB', payload: { topic: 'alerts' } });
      clientA.send({ type: 'TOPIC_SUB', payload: { topic: 'updates' } });

      await waitForSync(100);
      clientA.messages.length = 0;

      // ClientB publishes to all topics
      clientB.send({ type: 'TOPIC_PUB', payload: { topic: 'news', data: { type: 'news' } } });
      clientB.send({ type: 'TOPIC_PUB', payload: { topic: 'alerts', data: { type: 'alert' } } });
      clientB.send({ type: 'TOPIC_PUB', payload: { topic: 'updates', data: { type: 'update' } } });

      await waitForSync(300);

      const messages = clientA.messages.filter((m) => m.type === 'TOPIC_MESSAGE');
      expect(messages).toHaveLength(3);

      const topics = messages.map((m) => m.payload.topic).sort();
      expect(topics).toEqual(['alerts', 'news', 'updates']);

      // Verify data matches topic
      for (const msg of messages) {
        if (msg.payload.topic === 'news') expect(msg.payload.data.type).toBe('news');
        if (msg.payload.topic === 'alerts') expect(msg.payload.data.type).toBe('alert');
        if (msg.payload.topic === 'updates') expect(msg.payload.data.type).toBe('update');
      }
    });
  });

  // ========================================
  // Unsubscribe Tests
  // ========================================
  describe('Unsubscribe', () => {
    let server: ServerCoordinator;
    let publisher: TestClient;
    let subscriber: TestClient;

    beforeEach(async () => {
      server = await createTestServer();
      publisher = await createTestClient(`ws://localhost:${server.port}`, {
        roles: ['ADMIN'],
        nodeId: 'pub',
      });
      subscriber = await createTestClient(`ws://localhost:${server.port}`, {
        roles: ['ADMIN'],
        nodeId: 'sub',
      });
      await publisher.waitForMessage('AUTH_ACK');
      await subscriber.waitForMessage('AUTH_ACK');
    });

    afterEach(async () => {
      publisher.close();
      subscriber.close();
      await server.shutdown();
    });

    test('client unsubscribes and stops receiving messages', async () => {
      // Subscribe
      subscriber.send({ type: 'TOPIC_SUB', payload: { topic: 'unsub-test' } });
      await waitForSync(100);
      subscriber.messages.length = 0;

      // First message - should receive
      publisher.send({
        type: 'TOPIC_PUB',
        payload: { topic: 'unsub-test', data: { msg: 'first' } },
      });

      await waitForSync(150);
      let msg = subscriber.messages.find((m) => m.type === 'TOPIC_MESSAGE');
      expect(msg).toBeDefined();
      expect(msg.payload.data.msg).toBe('first');

      // Unsubscribe
      subscriber.send({ type: 'TOPIC_UNSUB', payload: { topic: 'unsub-test' } });
      await waitForSync(100);
      subscriber.messages.length = 0;

      // Second message - should NOT receive
      publisher.send({
        type: 'TOPIC_PUB',
        payload: { topic: 'unsub-test', data: { msg: 'second' } },
      });

      await waitForSync(300);
      msg = subscriber.messages.find((m) => m.type === 'TOPIC_MESSAGE');
      expect(msg).toBeUndefined();
    });

    test('unsubscribe from one topic does not affect other subscriptions', async () => {
      // Subscribe to two topics
      subscriber.send({ type: 'TOPIC_SUB', payload: { topic: 'keep' } });
      subscriber.send({ type: 'TOPIC_SUB', payload: { topic: 'remove' } });
      await waitForSync(100);

      // Unsubscribe from 'remove' only
      subscriber.send({ type: 'TOPIC_UNSUB', payload: { topic: 'remove' } });
      await waitForSync(100);
      subscriber.messages.length = 0;

      // Publish to both
      publisher.send({ type: 'TOPIC_PUB', payload: { topic: 'keep', data: { topic: 'keep' } } });
      publisher.send({ type: 'TOPIC_PUB', payload: { topic: 'remove', data: { topic: 'remove' } } });

      await waitForSync(300);

      const messages = subscriber.messages.filter((m) => m.type === 'TOPIC_MESSAGE');
      expect(messages).toHaveLength(1);
      expect(messages[0].payload.topic).toBe('keep');
    });

    test('resubscribe after unsubscribe works correctly', async () => {
      // Subscribe
      subscriber.send({ type: 'TOPIC_SUB', payload: { topic: 'resub-test' } });
      await waitForSync(100);

      // Unsubscribe
      subscriber.send({ type: 'TOPIC_UNSUB', payload: { topic: 'resub-test' } });
      await waitForSync(100);

      // Resubscribe
      subscriber.send({ type: 'TOPIC_SUB', payload: { topic: 'resub-test' } });
      await waitForSync(100);
      subscriber.messages.length = 0;

      // Publish
      publisher.send({
        type: 'TOPIC_PUB',
        payload: { topic: 'resub-test', data: { after: 'resub' } },
      });

      await waitForSync(150);

      const msg = subscriber.messages.find((m) => m.type === 'TOPIC_MESSAGE');
      expect(msg).toBeDefined();
      expect(msg.payload.data).toEqual({ after: 'resub' });
    });
  });

  // ========================================
  // Message Types Tests
  // ========================================
  describe('Message Types', () => {
    let server: ServerCoordinator;
    let publisher: TestClient;
    let subscriber: TestClient;

    beforeEach(async () => {
      server = await createTestServer();
      publisher = await createTestClient(`ws://localhost:${server.port}`, {
        roles: ['ADMIN'],
        nodeId: 'type-publisher',
      });
      subscriber = await createTestClient(`ws://localhost:${server.port}`, {
        roles: ['ADMIN'],
        nodeId: 'type-subscriber',
      });
      await publisher.waitForMessage('AUTH_ACK');
      await subscriber.waitForMessage('AUTH_ACK');

      subscriber.send({ type: 'TOPIC_SUB', payload: { topic: 'types-test' } });
      await waitForSync(100);
      subscriber.messages.length = 0;
    });

    afterEach(async () => {
      publisher.close();
      subscriber.close();
      await server.shutdown();
    });

    test('string message is received correctly', async () => {
      publisher.send({
        type: 'TOPIC_PUB',
        payload: { topic: 'types-test', data: 'Simple string message' },
      });

      const msg = await subscriber.waitForMessage('TOPIC_MESSAGE');
      expect(msg.payload.data).toBe('Simple string message');
    });

    test('number message is received correctly', async () => {
      publisher.send({
        type: 'TOPIC_PUB',
        payload: { topic: 'types-test', data: 42 },
      });

      const msg = await subscriber.waitForMessage('TOPIC_MESSAGE');
      expect(msg.payload.data).toBe(42);
    });

    test('boolean message is received correctly', async () => {
      publisher.send({
        type: 'TOPIC_PUB',
        payload: { topic: 'types-test', data: true },
      });

      const msg = await subscriber.waitForMessage('TOPIC_MESSAGE');
      expect(msg.payload.data).toBe(true);
    });

    test('object message is received correctly', async () => {
      const obj = {
        user: { id: 1, name: 'John' },
        action: 'login',
        metadata: { ip: '127.0.0.1', timestamp: Date.now() },
      };

      publisher.send({
        type: 'TOPIC_PUB',
        payload: { topic: 'types-test', data: obj },
      });

      const msg = await subscriber.waitForMessage('TOPIC_MESSAGE');
      expect(msg.payload.data.user).toEqual({ id: 1, name: 'John' });
      expect(msg.payload.data.action).toBe('login');
      expect(msg.payload.data.metadata.ip).toBe('127.0.0.1');
    });

    test('array message is received correctly', async () => {
      const arr = [1, 'two', { three: 3 }, [4, 5]];

      publisher.send({
        type: 'TOPIC_PUB',
        payload: { topic: 'types-test', data: arr },
      });

      const msg = await subscriber.waitForMessage('TOPIC_MESSAGE');
      expect(msg.payload.data).toEqual([1, 'two', { three: 3 }, [4, 5]]);
    });

    test('null message is received correctly', async () => {
      publisher.send({
        type: 'TOPIC_PUB',
        payload: { topic: 'types-test', data: null },
      });

      const msg = await subscriber.waitForMessage('TOPIC_MESSAGE');
      expect(msg.payload.data).toBeNull();
    });

    test('nested complex object message is received correctly', async () => {
      const complex = {
        level1: {
          level2: {
            level3: {
              value: 'deep',
              array: [1, 2, { nested: true }],
            },
          },
        },
      };

      publisher.send({
        type: 'TOPIC_PUB',
        payload: { topic: 'types-test', data: complex },
      });

      const msg = await subscriber.waitForMessage('TOPIC_MESSAGE');
      expect(msg.payload.data.level1.level2.level3.value).toBe('deep');
      expect(msg.payload.data.level1.level2.level3.array[2].nested).toBe(true);
    });
  });

  // ========================================
  // Message Order Tests
  // ========================================
  describe('Message Order', () => {
    test('messages are received in the order they were published', async () => {
      const server = await createTestServer();

      try {
        const publisher = await createTestClient(`ws://localhost:${server.port}`, {
          roles: ['ADMIN'],
          nodeId: 'order-publisher',
        });
        const subscriber = await createTestClient(`ws://localhost:${server.port}`, {
          roles: ['ADMIN'],
          nodeId: 'order-subscriber',
        });
        await publisher.waitForMessage('AUTH_ACK');
        await subscriber.waitForMessage('AUTH_ACK');

        subscriber.send({ type: 'TOPIC_SUB', payload: { topic: 'order-test' } });
        await waitForSync(100);
        subscriber.messages.length = 0;

        // Publish multiple messages in sequence
        for (let i = 1; i <= 10; i++) {
          publisher.send({
            type: 'TOPIC_PUB',
            payload: { topic: 'order-test', data: { seq: i } },
          });
        }

        // Wait for all messages
        await waitUntil(
          () => subscriber.messages.filter((m) => m.type === 'TOPIC_MESSAGE').length >= 10,
          5000
        );

        const messages = subscriber.messages.filter((m) => m.type === 'TOPIC_MESSAGE');
        expect(messages).toHaveLength(10);

        // Verify order
        for (let i = 0; i < 10; i++) {
          expect(messages[i].payload.data.seq).toBe(i + 1);
        }

        publisher.close();
        subscriber.close();
      } finally {
        await server.shutdown();
      }
    });

    test('rapid messages maintain order', async () => {
      const server = await createTestServer();

      try {
        const publisher = await createTestClient(`ws://localhost:${server.port}`, {
          roles: ['ADMIN'],
          nodeId: 'rapid-publisher',
        });
        const subscriber = await createTestClient(`ws://localhost:${server.port}`, {
          roles: ['ADMIN'],
          nodeId: 'rapid-subscriber',
        });
        await publisher.waitForMessage('AUTH_ACK');
        await subscriber.waitForMessage('AUTH_ACK');

        subscriber.send({ type: 'TOPIC_SUB', payload: { topic: 'rapid-order' } });
        await waitForSync(100);
        subscriber.messages.length = 0;

        // Send messages as fast as possible
        const count = 50;
        for (let i = 0; i < count; i++) {
          publisher.send({
            type: 'TOPIC_PUB',
            payload: { topic: 'rapid-order', data: { index: i } },
          });
        }

        await waitUntil(
          () => subscriber.messages.filter((m) => m.type === 'TOPIC_MESSAGE').length >= count,
          10000
        );

        const messages = subscriber.messages.filter((m) => m.type === 'TOPIC_MESSAGE');
        expect(messages.length).toBe(count);

        // Verify all indices are present and in order
        for (let i = 0; i < count; i++) {
          expect(messages[i].payload.data.index).toBe(i);
        }

        publisher.close();
        subscriber.close();
      } finally {
        await server.shutdown();
      }
    });
  });

  // ========================================
  // Edge Cases Tests
  // ========================================
  describe('Edge Cases', () => {
    test('subscribe to non-existent topic works (creates topic on first subscribe)', async () => {
      const server = await createTestServer();

      try {
        const subscriber = await createTestClient(`ws://localhost:${server.port}`, {
          roles: ['ADMIN'],
          nodeId: 'nonexistent-sub',
        });
        await subscriber.waitForMessage('AUTH_ACK');

        // Subscribe to topic that doesn't exist yet
        subscriber.send({
          type: 'TOPIC_SUB',
          payload: { topic: 'brand-new-topic-' + Date.now() },
        });

        // Should not throw/error - just creates subscription
        await waitForSync(200);

        // No error message should be received
        const errorMsg = subscriber.messages.find((m) => m.type === 'ERROR');
        expect(errorMsg).toBeUndefined();

        subscriber.close();
      } finally {
        await server.shutdown();
      }
    });

    test('publish to topic without subscribers does not cause errors', async () => {
      const server = await createTestServer();

      try {
        const publisher = await createTestClient(`ws://localhost:${server.port}`, {
          roles: ['ADMIN'],
          nodeId: 'lonely-publisher',
        });
        await publisher.waitForMessage('AUTH_ACK');
        publisher.messages.length = 0;

        // Publish to topic with no subscribers
        publisher.send({
          type: 'TOPIC_PUB',
          payload: {
            topic: 'empty-topic-' + Date.now(),
            data: { message: 'Anyone there?' },
          },
        });

        await waitForSync(200);

        // Should not receive error
        const errorMsg = publisher.messages.find((m) => m.type === 'ERROR');
        expect(errorMsg).toBeUndefined();

        publisher.close();
      } finally {
        await server.shutdown();
      }
    });

    test('late subscriber does NOT receive old messages (no persistence)', async () => {
      const server = await createTestServer();

      try {
        const publisher = await createTestClient(`ws://localhost:${server.port}`, {
          roles: ['ADMIN'],
          nodeId: 'early-publisher',
        });
        await publisher.waitForMessage('AUTH_ACK');

        // Publish messages BEFORE any subscriber exists
        publisher.send({
          type: 'TOPIC_PUB',
          payload: { topic: 'history-test', data: { msg: 'old message 1' } },
        });
        publisher.send({
          type: 'TOPIC_PUB',
          payload: { topic: 'history-test', data: { msg: 'old message 2' } },
        });

        await waitForSync(200);

        // Now a subscriber connects
        const subscriber = await createTestClient(`ws://localhost:${server.port}`, {
          roles: ['ADMIN'],
          nodeId: 'late-subscriber',
        });
        await subscriber.waitForMessage('AUTH_ACK');

        subscriber.send({ type: 'TOPIC_SUB', payload: { topic: 'history-test' } });
        await waitForSync(200);

        // Subscriber should NOT have received old messages
        const oldMessages = subscriber.messages.filter(
          (m) => m.type === 'TOPIC_MESSAGE' && m.payload.data.msg?.startsWith('old')
        );
        expect(oldMessages).toHaveLength(0);

        // But should receive new messages
        subscriber.messages.length = 0;

        publisher.send({
          type: 'TOPIC_PUB',
          payload: { topic: 'history-test', data: { msg: 'new message' } },
        });

        const newMsg = await subscriber.waitForMessage('TOPIC_MESSAGE');
        expect(newMsg.payload.data.msg).toBe('new message');

        publisher.close();
        subscriber.close();
      } finally {
        await server.shutdown();
      }
    });

    test('client disconnect cleans up subscriptions', async () => {
      const server = await createTestServer();

      try {
        const subscriber1 = await createTestClient(`ws://localhost:${server.port}`, {
          roles: ['ADMIN'],
          nodeId: 'disconnect-sub-1',
        });
        const subscriber2 = await createTestClient(`ws://localhost:${server.port}`, {
          roles: ['ADMIN'],
          nodeId: 'disconnect-sub-2',
        });
        const publisher = await createTestClient(`ws://localhost:${server.port}`, {
          roles: ['ADMIN'],
          nodeId: 'disconnect-pub',
        });

        await subscriber1.waitForMessage('AUTH_ACK');
        await subscriber2.waitForMessage('AUTH_ACK');
        await publisher.waitForMessage('AUTH_ACK');

        // Both subscribe
        subscriber1.send({ type: 'TOPIC_SUB', payload: { topic: 'disconnect-test' } });
        subscriber2.send({ type: 'TOPIC_SUB', payload: { topic: 'disconnect-test' } });
        await waitForSync(100);

        // Subscriber1 disconnects
        subscriber1.close();
        await waitForSync(200);

        // Clear subscriber2 messages
        subscriber2.messages.length = 0;

        // Publisher sends message
        publisher.send({
          type: 'TOPIC_PUB',
          payload: { topic: 'disconnect-test', data: { after: 'disconnect' } },
        });

        // Only subscriber2 should receive
        const msg = await subscriber2.waitForMessage('TOPIC_MESSAGE');
        expect(msg.payload.data).toEqual({ after: 'disconnect' });

        publisher.close();
        subscriber2.close();
      } finally {
        await server.shutdown();
      }
    });

    test('empty string data is handled correctly', async () => {
      const server = await createTestServer();

      try {
        const publisher = await createTestClient(`ws://localhost:${server.port}`, {
          roles: ['ADMIN'],
          nodeId: 'empty-pub',
        });
        const subscriber = await createTestClient(`ws://localhost:${server.port}`, {
          roles: ['ADMIN'],
          nodeId: 'empty-sub',
        });
        await publisher.waitForMessage('AUTH_ACK');
        await subscriber.waitForMessage('AUTH_ACK');

        subscriber.send({ type: 'TOPIC_SUB', payload: { topic: 'empty-string' } });
        await waitForSync(100);
        subscriber.messages.length = 0;

        publisher.send({
          type: 'TOPIC_PUB',
          payload: { topic: 'empty-string', data: '' },
        });

        const msg = await subscriber.waitForMessage('TOPIC_MESSAGE');
        expect(msg.payload.data).toBe('');

        publisher.close();
        subscriber.close();
      } finally {
        await server.shutdown();
      }
    });

    test('empty object data is handled correctly', async () => {
      const server = await createTestServer();

      try {
        const publisher = await createTestClient(`ws://localhost:${server.port}`, {
          roles: ['ADMIN'],
          nodeId: 'empty-obj-pub',
        });
        const subscriber = await createTestClient(`ws://localhost:${server.port}`, {
          roles: ['ADMIN'],
          nodeId: 'empty-obj-sub',
        });
        await publisher.waitForMessage('AUTH_ACK');
        await subscriber.waitForMessage('AUTH_ACK');

        subscriber.send({ type: 'TOPIC_SUB', payload: { topic: 'empty-object' } });
        await waitForSync(100);
        subscriber.messages.length = 0;

        publisher.send({
          type: 'TOPIC_PUB',
          payload: { topic: 'empty-object', data: {} },
        });

        const msg = await subscriber.waitForMessage('TOPIC_MESSAGE');
        expect(msg.payload.data).toEqual({});

        publisher.close();
        subscriber.close();
      } finally {
        await server.shutdown();
      }
    });

    test('special characters in topic name work correctly', async () => {
      const server = await createTestServer();

      try {
        const publisher = await createTestClient(`ws://localhost:${server.port}`, {
          roles: ['ADMIN'],
          nodeId: 'special-pub',
        });
        const subscriber = await createTestClient(`ws://localhost:${server.port}`, {
          roles: ['ADMIN'],
          nodeId: 'special-sub',
        });
        await publisher.waitForMessage('AUTH_ACK');
        await subscriber.waitForMessage('AUTH_ACK');

        // Topic with allowed special characters (based on TopicManager validation)
        const specialTopic = 'users/123/notifications:alerts-v2.0';

        subscriber.send({ type: 'TOPIC_SUB', payload: { topic: specialTopic } });
        await waitForSync(100);
        subscriber.messages.length = 0;

        publisher.send({
          type: 'TOPIC_PUB',
          payload: { topic: specialTopic, data: { special: true } },
        });

        const msg = await subscriber.waitForMessage('TOPIC_MESSAGE');
        expect(msg.payload.topic).toBe(specialTopic);
        expect(msg.payload.data).toEqual({ special: true });

        publisher.close();
        subscriber.close();
      } finally {
        await server.shutdown();
      }
    });

    test('unicode data in messages is handled correctly', async () => {
      const server = await createTestServer();

      try {
        const publisher = await createTestClient(`ws://localhost:${server.port}`, {
          roles: ['ADMIN'],
          nodeId: 'unicode-pub',
        });
        const subscriber = await createTestClient(`ws://localhost:${server.port}`, {
          roles: ['ADMIN'],
          nodeId: 'unicode-sub',
        });
        await publisher.waitForMessage('AUTH_ACK');
        await subscriber.waitForMessage('AUTH_ACK');

        subscriber.send({ type: 'TOPIC_SUB', payload: { topic: 'unicode-test' } });
        await waitForSync(100);
        subscriber.messages.length = 0;

        const unicodeData = {
          japanese: 'こんにちは',
          emoji: '🚀💻🎉',
          chinese: '中文测试',
          arabic: 'مرحبا',
          russian: 'Привет мир',
        };

        publisher.send({
          type: 'TOPIC_PUB',
          payload: { topic: 'unicode-test', data: unicodeData },
        });

        const msg = await subscriber.waitForMessage('TOPIC_MESSAGE');
        expect(msg.payload.data).toEqual(unicodeData);

        publisher.close();
        subscriber.close();
      } finally {
        await server.shutdown();
      }
    });

    test('large message is handled correctly', async () => {
      const server = await createTestServer();

      try {
        const publisher = await createTestClient(`ws://localhost:${server.port}`, {
          roles: ['ADMIN'],
          nodeId: 'large-pub',
        });
        const subscriber = await createTestClient(`ws://localhost:${server.port}`, {
          roles: ['ADMIN'],
          nodeId: 'large-sub',
        });
        await publisher.waitForMessage('AUTH_ACK');
        await subscriber.waitForMessage('AUTH_ACK');

        subscriber.send({ type: 'TOPIC_SUB', payload: { topic: 'large-message' } });
        await waitForSync(100);
        subscriber.messages.length = 0;

        // Create a reasonably large message (100KB of data)
        const largeArray = new Array(1000).fill(null).map((_, i) => ({
          id: i,
          data: 'x'.repeat(100),
        }));

        publisher.send({
          type: 'TOPIC_PUB',
          payload: { topic: 'large-message', data: { items: largeArray } },
        });

        const msg = await subscriber.waitForMessage('TOPIC_MESSAGE', 10000);
        expect(msg.payload.data.items).toHaveLength(1000);
        expect(msg.payload.data.items[500].id).toBe(500);

        publisher.close();
        subscriber.close();
      } finally {
        await server.shutdown();
      }
    });
  });

  // ========================================
  // Message Context Tests
  // ========================================
  describe('Message Context', () => {
    test('message includes timestamp', async () => {
      const server = await createTestServer();

      try {
        const publisher = await createTestClient(`ws://localhost:${server.port}`, {
          roles: ['ADMIN'],
          nodeId: 'ts-publisher',
        });
        const subscriber = await createTestClient(`ws://localhost:${server.port}`, {
          roles: ['ADMIN'],
          nodeId: 'ts-subscriber',
        });
        await publisher.waitForMessage('AUTH_ACK');
        await subscriber.waitForMessage('AUTH_ACK');

        subscriber.send({ type: 'TOPIC_SUB', payload: { topic: 'timestamp-test' } });
        await waitForSync(100);
        subscriber.messages.length = 0;

        const beforePublish = Date.now();

        publisher.send({
          type: 'TOPIC_PUB',
          payload: { topic: 'timestamp-test', data: { test: true } },
        });

        const msg = await subscriber.waitForMessage('TOPIC_MESSAGE');
        const afterReceive = Date.now();

        expect(msg.payload.timestamp).toBeGreaterThanOrEqual(beforePublish);
        expect(msg.payload.timestamp).toBeLessThanOrEqual(afterReceive);

        publisher.close();
        subscriber.close();
      } finally {
        await server.shutdown();
      }
    });

    test('message includes publisherId', async () => {
      const server = await createTestServer();

      try {
        const publisher = await createTestClient(`ws://localhost:${server.port}`, {
          roles: ['ADMIN'],
          nodeId: 'identified-publisher',
        });
        const subscriber = await createTestClient(`ws://localhost:${server.port}`, {
          roles: ['ADMIN'],
          nodeId: 'identified-subscriber',
        });
        await publisher.waitForMessage('AUTH_ACK');
        await subscriber.waitForMessage('AUTH_ACK');

        subscriber.send({ type: 'TOPIC_SUB', payload: { topic: 'publisher-id-test' } });
        await waitForSync(100);
        subscriber.messages.length = 0;

        publisher.send({
          type: 'TOPIC_PUB',
          payload: { topic: 'publisher-id-test', data: {} },
        });

        const msg = await subscriber.waitForMessage('TOPIC_MESSAGE');
        // publisherId is the server's internal clientId (UUID), not the nodeId
        expect(typeof msg.payload.publisherId).toBe('string');
        expect(msg.payload.publisherId).toBeTruthy();

        publisher.close();
        subscriber.close();
      } finally {
        await server.shutdown();
      }
    });
  });
});
