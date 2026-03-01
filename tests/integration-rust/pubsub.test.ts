import {
  createRustTestClient,
  spawnRustServer,
  waitForSync,
  waitUntil,
  TestClient,
} from './helpers';

describe('Integration: Pub/Sub (Rust Server)', () => {
  let cleanup: () => Promise<void>;
  let port: number;

  beforeAll(async () => {
    const server = await spawnRustServer();
    port = server.port;
    cleanup = server.cleanup;
  });

  afterAll(async () => {
    await cleanup();
  });

  // ========================================
  // Basic Pub/Sub Tests (AC30)
  // ========================================
  describe('TOPIC_SUB + TOPIC_PUB delivers TOPIC_MESSAGE', () => {
    test('subscriber receives published message', async () => {
      const topic = `topic-basic-${Date.now()}`;

      const subscriber = await createRustTestClient(port, {
        nodeId: 'sub-client-1',
        userId: 'sub-user-1',
        roles: ['ADMIN'],
      });
      await subscriber.waitForMessage('AUTH_ACK');

      const publisher = await createRustTestClient(port, {
        nodeId: 'pub-client-1',
        userId: 'pub-user-1',
        roles: ['ADMIN'],
      });
      await publisher.waitForMessage('AUTH_ACK');

      // Subscribe to topic
      subscriber.send({
        type: 'TOPIC_SUB',
        payload: {
          topic,
        },
      });

      await waitForSync(200);

      // Clear subscriber messages before publishing
      subscriber.messages.length = 0;

      // Publish a message
      publisher.send({
        type: 'TOPIC_PUB',
        payload: {
          topic,
          data: { greeting: 'hello world' },
        },
      });

      // Wait for subscriber to receive TOPIC_MESSAGE
      await waitUntil(
        () =>
          subscriber.messages.some(
            (m) =>
              m.type === 'TOPIC_MESSAGE' &&
              m.payload?.topic === topic
          ),
        5000
      );

      const message = subscriber.messages.find(
        (m) =>
          m.type === 'TOPIC_MESSAGE' &&
          m.payload?.topic === topic
      );
      expect(message).toBeDefined();
      expect(message.payload.data).toEqual({ greeting: 'hello world' });

      subscriber.close();
      publisher.close();
    });
  });

  // ========================================
  // Publisher Exclusion Tests (AC31)
  // ========================================
  describe('Publisher exclusion', () => {
    test('publisher does NOT receive its own published message', async () => {
      const topic = `topic-excl-${Date.now()}`;

      const pubSub = await createRustTestClient(port, {
        nodeId: 'pubsub-client-1',
        userId: 'pubsub-user-1',
        roles: ['ADMIN'],
      });
      await pubSub.waitForMessage('AUTH_ACK');

      // Subscribe and publish from the same client
      pubSub.send({
        type: 'TOPIC_SUB',
        payload: {
          topic,
        },
      });

      await waitForSync(200);

      // Create a separate subscriber to verify the message was actually published
      const verifier = await createRustTestClient(port, {
        nodeId: 'verify-client-1',
        userId: 'verify-user-1',
        roles: ['ADMIN'],
      });
      await verifier.waitForMessage('AUTH_ACK');

      verifier.send({
        type: 'TOPIC_SUB',
        payload: {
          topic,
        },
      });

      await waitForSync(200);

      // Clear messages before publishing
      pubSub.messages.length = 0;
      verifier.messages.length = 0;

      // Publish from pubSub client
      pubSub.send({
        type: 'TOPIC_PUB',
        payload: {
          topic,
          data: { selfTest: true },
        },
      });

      // Wait for verifier to receive the message (proves it was published)
      await waitUntil(
        () =>
          verifier.messages.some(
            (m) =>
              m.type === 'TOPIC_MESSAGE' &&
              m.payload?.topic === topic
          ),
        5000
      );

      // Give extra time for the publisher to potentially receive its own message
      await waitForSync(500);

      // Publisher should NOT have received a TOPIC_MESSAGE
      const selfMessages = pubSub.messages.filter(
        (m) =>
          m.type === 'TOPIC_MESSAGE' &&
          m.payload?.topic === topic
      );
      expect(selfMessages.length).toBe(0);

      pubSub.close();
      verifier.close();
    });
  });

  // ========================================
  // Multiple Subscribers Tests (AC32)
  // ========================================
  describe('Multiple subscribers', () => {
    test('all subscribers receive published message', async () => {
      const topic = `topic-multi-${Date.now()}`;

      // Create 3 subscribers
      const subscribers: TestClient[] = [];
      for (let i = 0; i < 3; i++) {
        const sub = await createRustTestClient(port, {
          nodeId: `multi-sub-${i}`,
          userId: `multi-sub-user-${i}`,
          roles: ['ADMIN'],
        });
        await sub.waitForMessage('AUTH_ACK');
        subscribers.push(sub);
      }

      // All subscribe to the same topic
      for (const sub of subscribers) {
        sub.send({
          type: 'TOPIC_SUB',
          payload: {
            topic,
          },
        });
      }

      await waitForSync(200);

      // Clear all subscriber messages
      for (const sub of subscribers) {
        sub.messages.length = 0;
      }

      // Publisher sends a message
      const publisher = await createRustTestClient(port, {
        nodeId: 'multi-pub-1',
        userId: 'multi-pub-user-1',
        roles: ['ADMIN'],
      });
      await publisher.waitForMessage('AUTH_ACK');

      publisher.send({
        type: 'TOPIC_PUB',
        payload: {
          topic,
          data: { broadcast: 'to all' },
        },
      });

      // Wait for all subscribers to receive the message
      for (const sub of subscribers) {
        await waitUntil(
          () =>
            sub.messages.some(
              (m) =>
                m.type === 'TOPIC_MESSAGE' &&
                m.payload?.topic === topic
            ),
          5000
        );

        const msg = sub.messages.find(
          (m) =>
            m.type === 'TOPIC_MESSAGE' &&
            m.payload?.topic === topic
        );
        expect(msg).toBeDefined();
        expect(msg.payload.data).toEqual({ broadcast: 'to all' });
      }

      for (const sub of subscribers) {
        sub.close();
      }
      publisher.close();
    });
  });

  // ========================================
  // Unsubscribe Tests (AC33)
  // ========================================
  describe('TOPIC_UNSUB', () => {
    test('unsubscribed client stops receiving messages', async () => {
      const topic = `topic-unsub-${Date.now()}`;

      const subscriber = await createRustTestClient(port, {
        nodeId: 'unsub-client-1',
        userId: 'unsub-user-1',
        roles: ['ADMIN'],
      });
      await subscriber.waitForMessage('AUTH_ACK');

      // A second subscriber to verify messages are still being published
      const stayer = await createRustTestClient(port, {
        nodeId: 'stayer-client-1',
        userId: 'stayer-user-1',
        roles: ['ADMIN'],
      });
      await stayer.waitForMessage('AUTH_ACK');

      const publisher = await createRustTestClient(port, {
        nodeId: 'unsub-pub-1',
        userId: 'unsub-pub-user-1',
        roles: ['ADMIN'],
      });
      await publisher.waitForMessage('AUTH_ACK');

      // Both subscribe
      subscriber.send({
        type: 'TOPIC_SUB',
        payload: { topic },
      });
      stayer.send({
        type: 'TOPIC_SUB',
        payload: { topic },
      });

      await waitForSync(200);

      // Verify subscription works with first message
      subscriber.messages.length = 0;
      stayer.messages.length = 0;

      publisher.send({
        type: 'TOPIC_PUB',
        payload: {
          topic,
          data: { msg: 'before-unsub' },
        },
      });

      // Both should receive
      await waitUntil(
        () =>
          subscriber.messages.some(
            (m) => m.type === 'TOPIC_MESSAGE'
          ),
        5000
      );
      await waitUntil(
        () =>
          stayer.messages.some(
            (m) => m.type === 'TOPIC_MESSAGE'
          ),
        5000
      );

      // Now unsubscribe the first subscriber
      subscriber.send({
        type: 'TOPIC_UNSUB',
        payload: { topic },
      });

      await waitForSync(200);

      // Clear messages
      subscriber.messages.length = 0;
      stayer.messages.length = 0;

      // Publish another message
      publisher.send({
        type: 'TOPIC_PUB',
        payload: {
          topic,
          data: { msg: 'after-unsub' },
        },
      });

      // Stayer should receive
      await waitUntil(
        () =>
          stayer.messages.some(
            (m) => m.type === 'TOPIC_MESSAGE'
          ),
        5000
      );

      // Give extra time for unsubscribed client to potentially receive
      await waitForSync(500);

      // Unsubscribed client should NOT receive the message
      const afterMessages = subscriber.messages.filter(
        (m) => m.type === 'TOPIC_MESSAGE'
      );
      expect(afterMessages.length).toBe(0);

      subscriber.close();
      stayer.close();
      publisher.close();
    });
  });

  // ========================================
  // Topic Isolation Tests (AC34)
  // ========================================
  describe('Topic isolation', () => {
    test('messages to topic A are not delivered to subscriber of topic B', async () => {
      const topicA = `topic-iso-A-${Date.now()}`;
      const topicB = `topic-iso-B-${Date.now()}`;

      const subA = await createRustTestClient(port, {
        nodeId: 'iso-subA-1',
        userId: 'iso-subA-user-1',
        roles: ['ADMIN'],
      });
      await subA.waitForMessage('AUTH_ACK');

      const subB = await createRustTestClient(port, {
        nodeId: 'iso-subB-1',
        userId: 'iso-subB-user-1',
        roles: ['ADMIN'],
      });
      await subB.waitForMessage('AUTH_ACK');

      const publisher = await createRustTestClient(port, {
        nodeId: 'iso-pub-1',
        userId: 'iso-pub-user-1',
        roles: ['ADMIN'],
      });
      await publisher.waitForMessage('AUTH_ACK');

      // Subscribe to different topics
      subA.send({
        type: 'TOPIC_SUB',
        payload: { topic: topicA },
      });
      subB.send({
        type: 'TOPIC_SUB',
        payload: { topic: topicB },
      });

      await waitForSync(200);

      // Clear messages
      subA.messages.length = 0;
      subB.messages.length = 0;

      // Publish to topic A only
      publisher.send({
        type: 'TOPIC_PUB',
        payload: {
          topic: topicA,
          data: { target: 'A only' },
        },
      });

      // subA should receive the message
      await waitUntil(
        () =>
          subA.messages.some(
            (m) =>
              m.type === 'TOPIC_MESSAGE' &&
              m.payload?.topic === topicA
          ),
        5000
      );

      const msgA = subA.messages.find(
        (m) =>
          m.type === 'TOPIC_MESSAGE' &&
          m.payload?.topic === topicA
      );
      expect(msgA).toBeDefined();
      expect(msgA.payload.data).toEqual({ target: 'A only' });

      // Give extra time for subB to potentially receive
      await waitForSync(500);

      // subB should NOT have received the message
      const bMessages = subB.messages.filter(
        (m) => m.type === 'TOPIC_MESSAGE'
      );
      expect(bMessages.length).toBe(0);

      subA.close();
      subB.close();
      publisher.close();
    });
  });

  // ========================================
  // Message Ordering Tests (AC35)
  // ========================================
  describe('Message ordering', () => {
    test('10 sequential messages maintain publishing order', async () => {
      const topic = `topic-order-${Date.now()}`;

      const subscriber = await createRustTestClient(port, {
        nodeId: 'order-sub-1',
        userId: 'order-sub-user-1',
        roles: ['ADMIN'],
      });
      await subscriber.waitForMessage('AUTH_ACK');

      const publisher = await createRustTestClient(port, {
        nodeId: 'order-pub-1',
        userId: 'order-pub-user-1',
        roles: ['ADMIN'],
      });
      await publisher.waitForMessage('AUTH_ACK');

      subscriber.send({
        type: 'TOPIC_SUB',
        payload: { topic },
      });

      await waitForSync(200);

      // Clear subscriber messages
      subscriber.messages.length = 0;

      // Send 10 messages sequentially
      for (let i = 0; i < 10; i++) {
        publisher.send({
          type: 'TOPIC_PUB',
          payload: {
            topic,
            data: { seq: i },
          },
        });
        // Small delay between sends to ensure sequential processing
        await waitForSync(50);
      }

      // Wait for all 10 messages to arrive
      await waitUntil(
        () =>
          subscriber.messages.filter(
            (m) =>
              m.type === 'TOPIC_MESSAGE' &&
              m.payload?.topic === topic
          ).length >= 10,
        10000
      );

      const receivedMessages = subscriber.messages.filter(
        (m) =>
          m.type === 'TOPIC_MESSAGE' &&
          m.payload?.topic === topic
      );
      expect(receivedMessages.length).toBe(10);

      // Verify ordering
      const sequences = receivedMessages.map(
        (m: any) => m.payload.data.seq
      );
      expect(sequences).toEqual([0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);

      subscriber.close();
      publisher.close();
    });
  });

  // ========================================
  // Various Data Types
  // ========================================
  describe('Various data types in messages', () => {
    test('string, number, boolean, object, array, null payloads', async () => {
      const topic = `topic-types-${Date.now()}`;

      const subscriber = await createRustTestClient(port, {
        nodeId: 'types-sub-1',
        userId: 'types-sub-user-1',
        roles: ['ADMIN'],
      });
      await subscriber.waitForMessage('AUTH_ACK');

      const publisher = await createRustTestClient(port, {
        nodeId: 'types-pub-1',
        userId: 'types-pub-user-1',
        roles: ['ADMIN'],
      });
      await publisher.waitForMessage('AUTH_ACK');

      subscriber.send({
        type: 'TOPIC_SUB',
        payload: { topic },
      });

      await waitForSync(200);

      const testPayloads = [
        { type: 'string', data: 'hello' },
        { type: 'number', data: 42 },
        { type: 'boolean', data: true },
        { type: 'object', data: { nested: { deep: 'value' } } },
        { type: 'array', data: [1, 'two', 3] },
        { type: 'null', data: null },
      ];

      subscriber.messages.length = 0;

      // Send each payload type sequentially
      for (const payload of testPayloads) {
        publisher.send({
          type: 'TOPIC_PUB',
          payload: {
            topic,
            data: payload,
          },
        });
        await waitForSync(50);
      }

      // Wait for all messages to arrive
      await waitUntil(
        () =>
          subscriber.messages.filter(
            (m) =>
              m.type === 'TOPIC_MESSAGE' &&
              m.payload?.topic === topic
          ).length >= testPayloads.length,
        10000
      );

      const received = subscriber.messages.filter(
        (m) =>
          m.type === 'TOPIC_MESSAGE' &&
          m.payload?.topic === topic
      );
      expect(received.length).toBe(testPayloads.length);

      // Verify each payload arrived correctly
      for (let i = 0; i < testPayloads.length; i++) {
        expect(received[i].payload.data).toEqual(testPayloads[i]);
      }

      subscriber.close();
      publisher.close();
    });
  });
});
