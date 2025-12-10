import { SyncEngine } from './SyncEngine';

export type TopicCallback = (data: any, context: { timestamp: number; publisherId?: string }) => void;

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
  public publish(data: any) {
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
  public onMessage(data: any, context: { timestamp: number; publisherId?: string }) {
    this.listeners.forEach(cb => {
      try {
        cb(data, context);
      } catch (e) {
        console.error('Error in topic listener', e);
      }
    });
  }
}

