import { TopicManager, TopicManagerConfig } from '../topic/TopicManager';
import { ClusterManager } from '../cluster/ClusterManager';

describe('TopicManager', () => {
  let topicManager: TopicManager;
  let mockSendToClient: jest.Mock;
  let mockCluster: jest.Mocked<ClusterManager>;

  beforeEach(() => {
    mockSendToClient = jest.fn();
    mockCluster = {
      getMembers: jest.fn().mockReturnValue([]),
      isLocal: jest.fn().mockReturnValue(true),
      send: jest.fn(),
    } as any;

    const config: TopicManagerConfig = {
      cluster: mockCluster,
      sendToClient: mockSendToClient,
    };

    topicManager = new TopicManager(config);
  });

  describe('subscribe()', () => {
    test('should subscribe a client to a topic', () => {
      expect(() => topicManager.subscribe('client1', 'topic1')).not.toThrow();
    });

    test('should allow multiple clients to subscribe to the same topic', () => {
      topicManager.subscribe('client1', 'topic1');
      topicManager.subscribe('client2', 'topic1');
      topicManager.subscribe('client3', 'topic1');

      // Verify all clients receive messages when publishing
      topicManager.publish('topic1', { msg: 'hello' }, 'external-sender');

      expect(mockSendToClient).toHaveBeenCalledTimes(3);
      expect(mockSendToClient).toHaveBeenCalledWith('client1', expect.any(Object));
      expect(mockSendToClient).toHaveBeenCalledWith('client2', expect.any(Object));
      expect(mockSendToClient).toHaveBeenCalledWith('client3', expect.any(Object));
    });

    test('should allow one client to subscribe to multiple topics', () => {
      topicManager.subscribe('client1', 'topic1');
      topicManager.subscribe('client1', 'topic2');
      topicManager.subscribe('client1', 'topic3');

      topicManager.publish('topic1', { msg: 'msg1' }, 'external');
      topicManager.publish('topic2', { msg: 'msg2' }, 'external');
      topicManager.publish('topic3', { msg: 'msg3' }, 'external');

      expect(mockSendToClient).toHaveBeenCalledTimes(3);
    });

    test('should throw error when subscription limit is reached', () => {
      // Subscribe to MAX_SUBSCRIPTIONS (100) topics
      for (let i = 0; i < 100; i++) {
        topicManager.subscribe('client1', `topic${i}`);
      }

      // 101st subscription should fail
      expect(() => topicManager.subscribe('client1', 'topic100')).toThrow('Subscription limit reached');
    });

    test('should allow same client to subscribe to the same topic twice (idempotent)', () => {
      topicManager.subscribe('client1', 'topic1');
      topicManager.subscribe('client1', 'topic1');

      topicManager.publish('topic1', { msg: 'hello' }, 'external');

      // Should only receive once, not twice
      expect(mockSendToClient).toHaveBeenCalledTimes(1);
    });
  });

  describe('unsubscribe()', () => {
    test('should unsubscribe a client from a topic', () => {
      topicManager.subscribe('client1', 'topic1');
      topicManager.unsubscribe('client1', 'topic1');

      topicManager.publish('topic1', { msg: 'hello' }, 'external');

      expect(mockSendToClient).not.toHaveBeenCalled();
    });

    test('should handle unsubscribe of non-existent client gracefully', () => {
      expect(() => topicManager.unsubscribe('non-existent', 'topic1')).not.toThrow();
    });

    test('should handle unsubscribe from non-existent topic gracefully', () => {
      topicManager.subscribe('client1', 'topic1');
      expect(() => topicManager.unsubscribe('client1', 'non-existent-topic')).not.toThrow();
    });

    test('should not affect other subscribers when one client unsubscribes', () => {
      topicManager.subscribe('client1', 'topic1');
      topicManager.subscribe('client2', 'topic1');
      topicManager.subscribe('client3', 'topic1');

      topicManager.unsubscribe('client2', 'topic1');

      topicManager.publish('topic1', { msg: 'hello' }, 'external');

      expect(mockSendToClient).toHaveBeenCalledTimes(2);
      expect(mockSendToClient).toHaveBeenCalledWith('client1', expect.any(Object));
      expect(mockSendToClient).toHaveBeenCalledWith('client3', expect.any(Object));
      expect(mockSendToClient).not.toHaveBeenCalledWith('client2', expect.any(Object));
    });
  });

  describe('unsubscribeAll()', () => {
    test('should unsubscribe client from all topics on disconnect', () => {
      topicManager.subscribe('client1', 'topic1');
      topicManager.subscribe('client1', 'topic2');
      topicManager.subscribe('client1', 'topic3');

      topicManager.unsubscribeAll('client1');

      topicManager.publish('topic1', { msg: 'msg1' }, 'external');
      topicManager.publish('topic2', { msg: 'msg2' }, 'external');
      topicManager.publish('topic3', { msg: 'msg3' }, 'external');

      expect(mockSendToClient).not.toHaveBeenCalled();
    });

    test('should handle unsubscribeAll for client with no subscriptions', () => {
      expect(() => topicManager.unsubscribeAll('non-existent-client')).not.toThrow();
    });

    test('should not affect other clients when one client disconnects', () => {
      topicManager.subscribe('client1', 'topic1');
      topicManager.subscribe('client2', 'topic1');
      topicManager.subscribe('client1', 'topic2');
      topicManager.subscribe('client2', 'topic2');

      topicManager.unsubscribeAll('client1');

      topicManager.publish('topic1', { msg: 'msg1' }, 'external');
      topicManager.publish('topic2', { msg: 'msg2' }, 'external');

      expect(mockSendToClient).toHaveBeenCalledTimes(2);
      expect(mockSendToClient).toHaveBeenCalledWith('client2', expect.any(Object));
    });
  });

  describe('publish()', () => {
    test('should send message to all subscribers of a topic', () => {
      topicManager.subscribe('client1', 'topic1');
      topicManager.subscribe('client2', 'topic1');

      topicManager.publish('topic1', { content: 'hello world' }, 'external-sender');

      expect(mockSendToClient).toHaveBeenCalledTimes(2);
      expect(mockSendToClient).toHaveBeenCalledWith('client1', {
        type: 'TOPIC_MESSAGE',
        payload: expect.objectContaining({
          topic: 'topic1',
          data: { content: 'hello world' },
          publisherId: 'external-sender',
          timestamp: expect.any(Number),
        }),
      });
    });

    test('should not send message to topic without subscribers', () => {
      topicManager.publish('empty-topic', { msg: 'hello' }, 'sender');

      expect(mockSendToClient).not.toHaveBeenCalled();
    });

    test('should not deliver message back to the sender', () => {
      topicManager.subscribe('client1', 'topic1');
      topicManager.subscribe('client2', 'topic1');
      topicManager.subscribe('client3', 'topic1');

      // client2 is the sender
      topicManager.publish('topic1', { msg: 'hello' }, 'client2');

      expect(mockSendToClient).toHaveBeenCalledTimes(2);
      expect(mockSendToClient).toHaveBeenCalledWith('client1', expect.any(Object));
      expect(mockSendToClient).toHaveBeenCalledWith('client3', expect.any(Object));
      expect(mockSendToClient).not.toHaveBeenCalledWith('client2', expect.any(Object));
    });

    test('should broadcast to cluster nodes when publishing locally', () => {
      mockCluster.getMembers.mockReturnValue(['node1', 'node2', 'node3']);
      mockCluster.isLocal.mockImplementation((nodeId) => nodeId === 'node1');

      topicManager.subscribe('client1', 'topic1');
      topicManager.publish('topic1', { msg: 'hello' }, 'client1');

      // Should send to remote cluster nodes
      expect(mockCluster.send).toHaveBeenCalledTimes(2);
      expect(mockCluster.send).toHaveBeenCalledWith('node2', 'CLUSTER_TOPIC_PUB', {
        topic: 'topic1',
        data: { msg: 'hello' },
        originalSenderId: 'client1',
      });
      expect(mockCluster.send).toHaveBeenCalledWith('node3', 'CLUSTER_TOPIC_PUB', {
        topic: 'topic1',
        data: { msg: 'hello' },
        originalSenderId: 'client1',
      });
    });

    test('should not broadcast to cluster when message is from cluster', () => {
      mockCluster.getMembers.mockReturnValue(['node1', 'node2']);
      mockCluster.isLocal.mockReturnValue(false);

      topicManager.subscribe('client1', 'topic1');
      topicManager.publish('topic1', { msg: 'hello' }, 'remote-sender', true);

      expect(mockCluster.send).not.toHaveBeenCalled();
    });

    test('should publish to multiple topics independently', () => {
      topicManager.subscribe('client1', 'topic1');
      topicManager.subscribe('client2', 'topic2');
      topicManager.subscribe('client3', 'topic1');
      topicManager.subscribe('client3', 'topic2');

      topicManager.publish('topic1', { msg: 'msg1' }, 'external');
      topicManager.publish('topic2', { msg: 'msg2' }, 'external');

      // topic1: client1, client3 (2 calls)
      // topic2: client2, client3 (2 calls)
      expect(mockSendToClient).toHaveBeenCalledTimes(4);
    });
  });

  describe('topic name validation', () => {
    test('should throw error for empty topic name', () => {
      expect(() => topicManager.subscribe('client1', '')).toThrow('Invalid topic name');
      expect(() => topicManager.publish('', { msg: 'hello' })).toThrow('Invalid topic name');
    });

    test('should throw error for topic name longer than 256 characters', () => {
      const longTopic = 'a'.repeat(257);
      expect(() => topicManager.subscribe('client1', longTopic)).toThrow('Invalid topic name');
    });

    test('should accept topic name with exactly 256 characters', () => {
      const maxLengthTopic = 'a'.repeat(256);
      expect(() => topicManager.subscribe('client1', maxLengthTopic)).not.toThrow();
    });

    test('should accept valid topic names with allowed special characters', () => {
      const validTopics = [
        'simple-topic',
        'topic_with_underscore',
        'topic.with.dots',
        'topic:with:colons',
        'topic/with/slashes',
        'complex-topic_name.with:all/chars',
        'TopicWithNumbers123',
      ];

      validTopics.forEach((topic) => {
        expect(() => topicManager.subscribe('client1', topic)).not.toThrow();
        topicManager.unsubscribe('client1', topic);
      });
    });

    test('should throw error for topic names with invalid characters', () => {
      const invalidTopics = [
        'topic with spaces',
        'topic@symbol',
        'topic#hash',
        'topic$dollar',
        'topic%percent',
        'topic&ampersand',
        'topic*asterisk',
        'topic(parens)',
        'topic[brackets]',
        'topic{braces}',
        'topic<angle>',
        'topic"quotes',
        "topic'apostrophe",
        'topic\\backslash',
        'topic|pipe',
        'topic?question',
        'topic!exclaim',
        'topic~tilde',
        'topic`backtick',
        'topic+plus',
        'topic=equals',
        'emoji😀topic',
      ];

      invalidTopics.forEach((topic) => {
        expect(() => topicManager.subscribe('client1', topic)).toThrow('Invalid topic name');
      });
    });
  });

  describe('edge cases', () => {
    test('should handle large number of subscribers', () => {
      const subscriberCount = 1000;

      for (let i = 0; i < subscriberCount; i++) {
        topicManager.subscribe(`client${i}`, 'popular-topic');
      }

      topicManager.publish('popular-topic', { msg: 'broadcast' }, 'external-sender');

      expect(mockSendToClient).toHaveBeenCalledTimes(subscriberCount);
    });

    test('should handle publish without senderId', () => {
      topicManager.subscribe('client1', 'topic1');

      topicManager.publish('topic1', { msg: 'hello' });

      expect(mockSendToClient).toHaveBeenCalledWith('client1', {
        type: 'TOPIC_MESSAGE',
        payload: expect.objectContaining({
          topic: 'topic1',
          data: { msg: 'hello' },
          publisherId: undefined,
        }),
      });
    });

    test('should include timestamp in published messages', () => {
      const beforePublish = Date.now();
      topicManager.subscribe('client1', 'topic1');
      topicManager.publish('topic1', { msg: 'hello' }, 'sender');
      const afterPublish = Date.now();

      const publishedMessage = mockSendToClient.mock.calls[0][1];
      const timestamp = publishedMessage.payload.timestamp;

      expect(timestamp).toBeGreaterThanOrEqual(beforePublish);
      expect(timestamp).toBeLessThanOrEqual(afterPublish);
    });

    test('should handle rapid subscribe/unsubscribe cycles', () => {
      for (let i = 0; i < 100; i++) {
        topicManager.subscribe('client1', 'topic1');
        topicManager.unsubscribe('client1', 'topic1');
      }

      topicManager.publish('topic1', { msg: 'hello' }, 'external');
      expect(mockSendToClient).not.toHaveBeenCalled();
    });

    test('should handle various data types in publish payload', () => {
      topicManager.subscribe('client1', 'topic1');

      const payloads = [
        null,
        'string data',
        12345,
        { nested: { object: true } },
        [1, 2, 3],
        true,
      ];

      payloads.forEach((data, index) => {
        topicManager.publish('topic1', data, 'external');
        expect(mockSendToClient).toHaveBeenLastCalledWith('client1', {
          type: 'TOPIC_MESSAGE',
          payload: expect.objectContaining({
            data,
          }),
        });
      });
    });
  });
});
