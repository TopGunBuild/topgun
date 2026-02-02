import { SyncEngine } from './SyncEngine';
import { logger } from './utils/logger';

export type TopicCallback<T = unknown> = (
  data: T,
  context: { timestamp: number; publisherId?: string }
) => void;

export class TopicHandle {
  private engine: SyncEngine;
  private topic: string;
  private listeners: Set<TopicCallback> = new Set();

  constructor(engine: SyncEngine, topic: string) {
    this.engine = engine;
    this.topic = topic;
  }

  public get id(): string {
    return this.topic;
  }

  /**
   * Publish a message to the topic
   */
  public publish(data: unknown): void {
    this.engine.publishTopic(this.topic, data);
  }

  /**
   * Subscribe to the topic
   */
  public subscribe(callback: TopicCallback) {
    if (this.listeners.size === 0) {
      this.engine.subscribeToTopic(this.topic, this);
    }
    this.listeners.add(callback);
    return () => this.unsubscribe(callback);
  }

  private unsubscribe(callback: TopicCallback) {
    this.listeners.delete(callback);
    if (this.listeners.size === 0) {
      this.engine.unsubscribeFromTopic(this.topic);
    }
  }

  /**
   * Called by SyncEngine when a message is received
   */
  public onMessage(data: unknown, context: { timestamp: number; publisherId?: string }): void {
    this.listeners.forEach(cb => {
      try {
        cb(data, context);
      } catch (e) {
        logger.error({ err: e, topic: this.topic, context: 'listener' }, 'Error in topic listener');
      }
    });
  }
}

