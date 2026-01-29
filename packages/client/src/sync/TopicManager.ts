/**
 * TopicManager - Handles topic (pub/sub) operations for SyncEngine
 *
 * Responsibilities:
 * - Topic subscriptions and unsubscriptions
 * - Publishing messages to topics
 * - Queueing messages when offline
 * - Flushing queued messages after authentication
 * - Handling incoming topic messages from server
 */

import type { TopicQueueConfig } from '../SyncEngine';
import { TopicHandle } from '../TopicHandle';
import { logger } from '../utils/logger';
import type { ITopicManager, TopicManagerConfig } from './types';

/**
 * Queued topic message for offline publishing.
 */
interface QueuedTopicMessage {
  topic: string;
  data: any;
  timestamp: number;
}

/**
 * TopicManager implements ITopicManager.
 *
 * Manages topic subscriptions with support for:
 * - Pub/sub pattern for real-time messaging
 * - Offline message queueing with configurable strategies
 * - Automatic resubscription after authentication
 */
export class TopicManager implements ITopicManager {
  private readonly config: TopicManagerConfig;

  // Topic subscriptions (single source of truth)
  private topics: Map<string, TopicHandle> = new Map();

  // Offline message queue
  private topicQueue: QueuedTopicMessage[] = [];

  constructor(config: TopicManagerConfig) {
    this.config = config;
  }

  // ============================================
  // Public API
  // ============================================

  /**
   * Subscribe to a topic.
   * Adds to topics Map and sends subscription to server if authenticated.
   */
  public subscribeToTopic(topic: string, handle: TopicHandle): void {
    this.topics.set(topic, handle);
    if (this.config.isAuthenticated()) {
      this.sendTopicSubscription(topic);
    }
  }

  /**
   * Unsubscribe from a topic.
   * Removes from Map and sends unsubscription to server if authenticated.
   */
  public unsubscribeFromTopic(topic: string): void {
    this.topics.delete(topic);
    if (this.config.isAuthenticated()) {
      this.config.sendMessage({
        type: 'TOPIC_UNSUB',
        payload: { topic }
      });
    }
  }

  /**
   * Publish a message to a topic.
   * Sends immediately if authenticated, otherwise queues for later.
   */
  public publishTopic(topic: string, data: any): void {
    if (this.config.isAuthenticated()) {
      this.config.sendMessage({
        type: 'TOPIC_PUB',
        payload: { topic, data }
      });
    } else {
      this.queueTopicMessage(topic, data);
    }
  }

  /**
   * Flush all queued topic messages.
   * Called by SyncEngine after authentication.
   */
  public flushTopicQueue(): void {
    if (this.topicQueue.length === 0) return;

    logger.info({ count: this.topicQueue.length }, 'Flushing queued topic messages');

    for (const msg of this.topicQueue) {
      this.config.sendMessage({
        type: 'TOPIC_PUB',
        payload: { topic: msg.topic, data: msg.data },
      });
    }

    this.topicQueue = [];
  }

  /**
   * Get topic queue status.
   */
  public getTopicQueueStatus(): { size: number; maxSize: number } {
    return {
      size: this.topicQueue.length,
      maxSize: this.config.topicQueueConfig.maxSize,
    };
  }

  /**
   * Get all subscribed topics.
   * Used for resubscription after authentication.
   */
  public getTopics(): IterableIterator<string> {
    return this.topics.keys();
  }

  /**
   * Handle incoming topic message from server.
   */
  public handleTopicMessage(topic: string, data: any, publisherId: string, timestamp: number): void {
    const handle = this.topics.get(topic);
    if (handle) {
      handle.onMessage(data, { publisherId, timestamp });
    }
  }

  // ============================================
  // Private Methods
  // ============================================

  /**
   * Queue a topic message for offline publishing.
   */
  private queueTopicMessage(topic: string, data: any): void {
    const message: QueuedTopicMessage = {
      topic,
      data,
      timestamp: Date.now(),
    };

    if (this.topicQueue.length >= this.config.topicQueueConfig.maxSize) {
      if (this.config.topicQueueConfig.strategy === 'drop-oldest') {
        const dropped = this.topicQueue.shift();
        logger.warn({ topic: dropped?.topic }, 'Dropped oldest queued topic message (queue full)');
      } else {
        logger.warn({ topic }, 'Dropped newest topic message (queue full)');
        return;
      }
    }

    this.topicQueue.push(message);
    logger.debug({ topic, queueSize: this.topicQueue.length }, 'Queued topic message for offline');
  }

  /**
   * Send topic subscription message to server.
   */
  private sendTopicSubscription(topic: string): void {
    this.config.sendMessage({
      type: 'TOPIC_SUB',
      payload: { topic }
    });
  }
}
